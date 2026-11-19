//! Owned host preparation, driven between bytecode slices by the execution session.
use super::*;

pub(super) use crate::request_identity::{Authority, Ticket};

#[derive(Debug)]
pub(crate) struct Preparation {
    ticket: Ticket,
    operation: Operation,
}

impl Frame {
    pub(super) fn request_preparation(&mut self, operation: Operation) -> Result<Progress, Error> {
        if self.terminal || self.waiting.is_some() {
            return Err(Error {
                kind: ErrorKind::InvalidControlFlow,
                source: None,
            });
        }
        let ticket = self.preparations.issue()?;
        self.waiting = Some(Waiting::Preparation(ticket.clone()));
        Ok(Progress::Prepare(Preparation { ticket, operation }))
    }
}

impl Preparation {
    pub(crate) fn is_definition(&self) -> bool {
        matches!(self.operation, Operation::Definition { .. })
    }
    pub(crate) fn is_method(&self) -> bool {
        match &self.operation {
            Operation::Method { .. } => true,
            Operation::Call {
                code,
                instruction,
                dynamic_method,
            } => {
                dynamic_method.is_some()
                    || matches!(
                        &code.instructions()[*instruction].op,
                        Op::BeginCall {
                            method: Some(_),
                            ..
                        }
                    )
            }
            _ => false,
        }
    }
}

#[derive(Debug)]
pub(super) enum Operation {
    Method {
        destination: usize,
        method: Arc<comptime_compiler::methods::Method>,
    },
    Call {
        code: Arc<Program>,
        instruction: usize,
        dynamic_method: Option<comptime_compiler::methods::Method>,
    },
    Definition {
        code: Arc<Program>,
        instruction: usize,
    },
}

pub(crate) struct MethodWait {
    pub(crate) request: trait_methods::Request,
    pub(crate) continuation: MethodContinuation,
}
pub(crate) enum MethodContinuation {
    Value {
        destination: usize,
    },
    Call {
        code: Arc<Program>,
        instruction: usize,
        setup: CallSetup,
    },
}
pub(crate) struct CallSetup {
    defaults: Option<method_catalog::Inputs<Term>>,
    program: nir::contracts::ProgramContract,
    parameters: Option<Vec<LocalId>>,
}
impl fmt::Display for MethodContinuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value { destination } => write!(f, "(method-value (destination {destination}))"),
            Self::Call { instruction, .. } => {
                write!(f, "(method-call (instruction {instruction}))")
            }
        }
    }
}
impl fmt::Display for MethodWait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(method-wait {} {})", self.continuation, self.request)
    }
}

impl fmt::Display for Preparation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(preparation-request {} ", self.ticket)?;
        self.operation.fmt(f)?;
        f.write_str(")")
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition { instruction, .. } => {
                write!(f, "(prepare-definition-build (instruction {instruction}))")
            }
            Self::Method {
                destination,
                method,
            } => {
                write!(f, "(prepare-method (destination {destination}) {method})")
            }
            Self::Call {
                instruction,
                dynamic_method,
                ..
            } => {
                write!(
                    f,
                    "(prepare-call (instruction {instruction}) (dynamic-method"
                )?;
                if let Some(method) = dynamic_method {
                    write!(f, " {method}")?;
                }
                f.write_str("))")
            }
        }
    }
}

impl<'a> Interpreter<'a> {
    pub(crate) fn prepare_bytecode_request(
        &mut self,
        frame: &mut Frame,
        request: Preparation,
    ) -> Result<(), Error> {
        if frame.terminal
            || !matches!(&frame.waiting, Some(Waiting::Preparation(ticket))
                if ticket.matches(&request.ticket))
        {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        if self.graph.is_none() && request.is_method() {
            if let Some(wait) = self.begin_bytecode_method(frame, request, false)? {
                let selected = self.resolve_method_request(&wait.request);
                self.finish_bytecode_method(frame, wait, selected)?;
            }
            return Ok(());
        }
        if request.is_definition() {
            let (destination, request) = self.begin_bytecode_definition(frame, request)?;
            let resolver = self.definition_resolver(&request);
            let prepared = host_budget::clamped(&mut self.budget.steps, |fuel| {
                resolver.prepare(&request, fuel)
            })?;
            return self.finish_bytecode_definition(frame, destination, &request, prepared);
        }
        // Claim before touching operands or calling the host. Errors and unwinds
        // leave this activation terminal; retry requires a fresh execution frame.
        frame.waiting = None;
        frame.terminal = true;
        match request.operation {
            Operation::Definition { .. } => unreachable!("handled above"),
            Operation::Method {
                destination,
                method,
            } => {
                let value = self.bytecode_trait_method(method.view(), false)?;
                frame.slots[destination] = Some(value);
            }
            Operation::Call {
                code,
                instruction,
                dynamic_method,
            } => {
                if !Arc::ptr_eq(&frame.code, &code) || frame.pc.checked_sub(1) != Some(instruction)
                {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, None));
                }
                self.prepare_bytecode_call(
                    frame,
                    &code.instructions()[instruction],
                    dynamic_method,
                )?;
            }
        }
        frame.terminal = false;
        Ok(())
    }

    /// Claim this instruction once, retaining a terminal frame until its
    /// checked publication response is admitted by the owning session.
    pub(crate) fn begin_bytecode_definition(
        &mut self,
        frame: &mut Frame,
        request: Preparation,
    ) -> Result<(usize, DefinitionBuildRequest), Error> {
        if frame.terminal
            || !matches!(&frame.waiting, Some(Waiting::Preparation(ticket))
            if ticket.matches(&request.ticket))
        {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        frame.waiting = None;
        frame.terminal = true;
        let Operation::Definition { code, instruction } = request.operation else {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        };
        if !Arc::ptr_eq(&frame.code, &code) || frame.pc.checked_sub(1) != Some(instruction) {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        let instruction = &code.instructions()[instruction];
        let Op::BuildDefinition {
            destination,
            value,
            build,
        } = &instruction.op
        else {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(instruction.source)));
        };
        if build.output != Term::Primitive(Primitive::Type) {
            return Err(self.failure(ErrorKind::Type, Some(instruction.source)));
        }
        let value = frame.slots[*value]
            .take()
            .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(instruction.source)))?;
        self.decode_definition_build(build.kind, value, instruction.source)
            .map(|request| (*destination, request))
    }

    pub(crate) fn finish_bytecode_definition(
        &mut self,
        frame: &mut Frame,
        destination: usize,
        request: &DefinitionBuildRequest,
        prepared: PreparedDefinitionBuild<'a>,
    ) -> Result<(), Error> {
        if !frame.terminal || frame.waiting.is_some() || destination >= frame.slots.len() {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(request.source())));
        }
        let value = self.admit_definition_build(request, prepared)?;
        frame.slots[destination] = Some(value);
        frame.terminal = false;
        Ok(())
    }

    pub(crate) fn begin_bytecode_method(
        &mut self,
        frame: &mut Frame,
        request: Preparation,
        external: bool,
    ) -> Result<Option<MethodWait>, Error> {
        if frame.terminal
            || !matches!(&frame.waiting,Some(Waiting::Preparation(ticket)) if ticket.matches(&request.ticket))
        {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        frame.waiting = None;
        frame.terminal = true;
        let (method, continuation, direct) = match request.operation {
            Operation::Method {
                destination,
                method,
            } => (method, MethodContinuation::Value { destination }, false),
            Operation::Call {
                code,
                instruction,
                dynamic_method,
            } => {
                if !Arc::ptr_eq(&frame.code, &code) || frame.pc.checked_sub(1) != Some(instruction)
                {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, None));
                }
                let setup = self.bytecode_call_setup(
                    &code.instructions()[instruction],
                    dynamic_method.as_ref(),
                )?;
                let Op::BeginCall { method, .. } = &code.instructions()[instruction].op else {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, None));
                };
                let method = method
                    .clone()
                    .or_else(|| dynamic_method.map(Arc::new))
                    .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, None))?;
                self.tick(Some(method.source))?;
                (
                    method,
                    MethodContinuation::Call {
                        code,
                        instruction,
                        setup,
                    },
                    true,
                )
            }
            _ => return Err(self.failure(ErrorKind::InvalidControlFlow, None)),
        };
        let prepared = if direct {
            self.with_native_depth(|vm| vm.prepare_trait_method(method.view(), direct, external))?
        } else {
            self.prepare_trait_method(method.view(), direct, external)?
        };
        match prepared {
            trait_methods::Preparation::Ready(selection) => {
                let value = if direct {
                    self.with_native_depth(|vm| vm.trait_selection_value(selection, method.source))?
                } else {
                    self.trait_selection_value(selection, method.source)?
                };
                self.complete_bytecode_method(frame, continuation, value)?;
                Ok(None)
            }
            trait_methods::Preparation::Request(request) => Ok(Some(MethodWait {
                request,
                continuation,
            })),
        }
    }

    pub(crate) fn finish_bytecode_method(
        &mut self,
        frame: &mut Frame,
        wait: MethodWait,
        selected: Result<nir::program::MethodInstance, Error>,
    ) -> Result<(), Error> {
        if !frame.terminal || frame.waiting.is_some() {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        let source = wait.request.access().source;
        let complete = |vm: &mut Self| {
            let selection = vm.accept_method_selection(wait.request, selected)?;
            vm.trait_selection_value(selection, source)
        };
        let value = if matches!(wait.continuation, MethodContinuation::Call { .. }) {
            self.with_native_depth(complete)?
        } else {
            complete(self)?
        };
        self.complete_bytecode_method(frame, wait.continuation, value)
    }

    fn complete_bytecode_method(
        &mut self,
        frame: &mut Frame,
        continuation: MethodContinuation,
        value: Value,
    ) -> Result<(), Error> {
        match continuation {
            MethodContinuation::Value { destination } => {
                frame.slots[destination] = Some(value);
            }
            MethodContinuation::Call {
                code,
                instruction,
                setup,
            } => {
                self.finish_bytecode_call(frame, &code.instructions()[instruction], value, setup)?
            }
        }
        frame.terminal = false;
        Ok(())
    }

    fn prepare_bytecode_call(
        &mut self,
        frame: &mut Frame,
        instruction: &comptime_compiler::Instruction,
        dynamic_method: Option<comptime_compiler::methods::Method>,
    ) -> Result<(), Error> {
        let source = instruction.source;
        let missing = || Error {
            kind: ErrorKind::MissingBinding,
            source: Some(source),
        };
        let Op::BeginCall { callee, method, .. } = &instruction.op else {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
        };
        let setup = self.bytecode_call_setup(instruction, dynamic_method.as_ref())?;
        let method = method.as_deref().or(dynamic_method.as_ref());
        let callee = if let Some(method) = method {
            self.tick(Some(method.source))?;
            self.with_native_depth(|vm| vm.bytecode_trait_method(method.view(), true))?
        } else {
            frame.slots[*callee].take().ok_or_else(missing)?
        };
        self.finish_bytecode_call(frame, instruction, callee, setup)
    }

    fn bytecode_call_setup(
        &mut self,
        instruction: &comptime_compiler::Instruction,
        dynamic_method: Option<&comptime_compiler::methods::Method>,
    ) -> Result<CallSetup, Error> {
        let Op::BeginCall {
            prepare_defaults,
            method,
            ..
        } = &instruction.op
        else {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(instruction.source)));
        };
        let defaults = if *prepare_defaults {
            dynamic_method
                .map(|method| self.trait_method_default_inputs(method.view()))
                .transpose()?
        } else {
            None
        };
        let (program, parameters) = if let Some(method) = method.as_deref().or(dynamic_method) {
            self.interface_method_contract(method.view(), instruction.source)?
        } else {
            (Default::default(), None)
        };
        Ok(CallSetup {
            defaults,
            program,
            parameters,
        })
    }

    fn finish_bytecode_call(
        &mut self,
        frame: &mut Frame,
        instruction: &comptime_compiler::Instruction,
        callee: Value,
        setup: CallSetup,
    ) -> Result<(), Error> {
        let source = instruction.source;
        let Op::BeginCall {
            call,
            parameter_bindings,
            prepare_defaults,
            ..
        } = &instruction.op
        else {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
        };
        let CallSetup {
            defaults: trait_defaults,
            program: interface_program,
            parameters: interface_parameters,
        } = setup;
        self.check_transported_instance(&callee)?;
        let parameters = self.call_parameters(&callee, source)?;
        let count = parameters.len();
        let preparation_parameters = interface_parameters.as_ref().unwrap_or(&parameters);
        if !parameter_bindings.is_empty()
            && (*parameter_bindings != *preparation_parameters || parameter_bindings.len() != count)
        {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        self.effect_charge(count, Some(source))?;
        frame.calls.insert(
            *call,
            PendingCall {
                callee,
                trait_defaults,
                interface: interface_parameters.map(|parameters| (interface_program, parameters)),
                arguments: vec![None; count],
                parameter_bindings: if *prepare_defaults {
                    parameter_bindings.clone()
                } else {
                    vec![]
                },
                defaults: Environment::new(),
                last_default: None,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
