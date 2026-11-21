//! A source region may know the selected function and its first checked
//! operand before it can form the rest of the call. Keep lexical bindings and
//! the callee's preparation alive while the region checks the next operand.
use super::*;
use crate::call_operands::CallOperands;
use std::ops::ControlFlow;

/// Owns caller bindings across successive call preparations in one source VM.
pub struct SourceOperandScope<'vm, 'world> {
    vm: &'vm mut Interpreter<'world>,
    caller: Environment,
}

/// An ordinary, directly selected source function. No complete call expression
/// or speculative suffix is needed to run the next checked argument.
#[must_use]
pub struct SourceCallPrefix<'scope, 'world> {
    vm: &'scope mut Interpreter<'world>,
    caller: &'scope mut Environment,
    target: Target<'world>,
    source: SourceRange,
    operands: Option<CallOperands>,
}

#[derive(Debug)]
pub enum SourceOperandStep {
    Supplied,
    Return(Value),
    Break(LocalId),
    Continue(LocalId),
}

impl<'world> Interpreter<'world> {
    pub fn source_operand_scope(
        &mut self,
        bindings: Vec<(LocalId, Term, InvocationValue)>,
    ) -> Result<SourceOperandScope<'_, 'world>, Error> {
        if self.graph.is_some() || !self.bytecode_invocation_available() {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        self.charge_preparation(bindings.len())?;
        let mut unique = BTreeSet::new();
        let mut ids = Vec::with_capacity(bindings.len());
        let mut inputs = Vec::with_capacity(bindings.len());
        for (id, domain, input) in bindings {
            if !unique.insert(id) {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
            ids.push(id);
            inputs.push((domain, input));
        }
        let values = self.prepare_invocation_arguments(inputs)?;
        let mut caller = Environment::new();
        for (id, value) in ids.into_iter().zip(values) {
            caller.insert(id, value);
        }
        Ok(SourceOperandScope { vm: self, caller })
    }
}

impl<'world> SourceOperandScope<'_, 'world> {
    /// Start at the point where source checking selected a named function.
    /// Check its phase and declaration inputs before any ordinary operand runs.
    pub fn begin_call(
        &mut self,
        declaration: DeclarationId,
        declaration_inputs: Vec<Term>,
        source: SourceRange,
    ) -> Result<SourceCallPrefix<'_, 'world>, Error> {
        let target = self
            .vm
            .named_entry_target(declaration, declaration_inputs)?;
        let definition = target.declaration().expect("named source target");
        if definition.is_unsafe {
            return Err(Error::phase(nir::phases::Error {
                kind: nir::phases::ErrorKind::Invalid,
                source,
                message: "unsafe call requires an enclosing unsafe scope",
            }));
        }
        let parameters = &definition.parameters;
        self.vm.charge_preparation(parameters.len())?;
        let operands = CallOperands::for_slots(
            parameters.len(),
            parameters,
            !definition.defaults.is_empty(),
        );
        Ok(SourceCallPrefix {
            vm: self.vm,
            caller: &mut self.caller,
            target,
            operands: Some(operands),
            source,
        })
    }
}

impl<'scope, 'world> SourceCallPrefix<'scope, 'world> {
    /// Submit the next routed, checked source argument. Only normal completion
    /// publishes its slot. Failure or escaping control consumes the preparation.
    pub fn supply(
        &mut self,
        argument: &nir::program::CallArgument,
    ) -> Result<SourceOperandStep, Error> {
        let mut operands = self.operands.take().ok_or_else(|| {
            self.vm
                .failure(ErrorKind::InvalidCallPlan, Some(self.source))
        })?;
        self.vm.check_comptime_expression(&argument.value)?;
        if argument.is_default {
            let defaults = &self
                .target
                .declaration()
                .expect("named source target")
                .defaults;
            self.vm.charge_preparation(defaults.len())?;
            if !defaults.iter().any(|(slot, _)| *slot == argument.parameter) {
                return Err(self
                    .vm
                    .failure(ErrorKind::InvalidCallPlan, Some(self.source)));
            }
        }
        let flow = operands.evaluate(
            self.vm,
            self.source,
            argument,
            self.caller,
            None,
            None,
            |vm, expr, caller| {
                let code = vm
                    .lexical_expression_code(expr)?
                    .ok_or_else(|| vm.failure(ErrorKind::PhaseMismatch, Some(expr.source)))?;
                let mut frame = vm.start_frame(&code, caller)?;
                let flow = vm.run_fragment(&mut frame);
                *caller = frame.restore_environment();
                flow
            },
        )?;
        match flow {
            ControlFlow::Continue(()) => {
                self.operands = Some(operands);
                Ok(SourceOperandStep::Supplied)
            }
            ControlFlow::Break(Flow::Return(value)) => Ok(SourceOperandStep::Return(value)),
            ControlFlow::Break(Flow::Break(id)) => Ok(SourceOperandStep::Break(id)),
            ControlFlow::Break(Flow::Continue(id)) => Ok(SourceOperandStep::Continue(id)),
            ControlFlow::Break(_) => Err(self
                .vm
                .failure(ErrorKind::InvalidControlFlow, Some(self.source))),
        }
    }

    /// Read an already evaluated slot in the VM's scope and budget. A missing
    /// slot is distinct from supplied Type data with description fields.
    pub fn data(
        &mut self,
        parameter: usize,
    ) -> Result<Option<nir::described_applications::DataArgument>, Error> {
        let operands = self.operands.as_ref().ok_or_else(|| {
            self.vm
                .failure(ErrorKind::InvalidCallPlan, Some(self.source))
        })?;
        if parameter
            >= self
                .target
                .declaration()
                .expect("named source target")
                .parameters
                .len()
        {
            return Err(self
                .vm
                .failure(ErrorKind::InvalidCallPlan, Some(self.source)));
        }
        operands
            .supplied(parameter)
            .map(|value| self.vm.freeze_normal_argument(value, self.source))
            .transpose()
    }

    /// Enter the ordinary invocation only after all slots are available.
    pub fn finish(self) -> Result<InvocationSession<'scope, 'world>, Error> {
        let operands = self.operands.ok_or_else(|| {
            self.vm
                .failure(ErrorKind::InvalidCallPlan, Some(self.source))
        })?;
        let (values, _) = operands.finish(self.vm, self.source)?;
        let active = self
            .vm
            .begin_target_invocation(self.target, values, None, None)?;
        Ok(InvocationSession {
            session: Session::invocation(self.vm, active),
        })
    }
}
