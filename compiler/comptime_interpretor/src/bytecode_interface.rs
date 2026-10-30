//! Public interface checks suspended around an implementation invocation.
use super::*;

enum Phase {
    Entry,
    Body,
    Exit,
    Done,
}
pub(super) struct InterfaceCall {
    pub(super) destination: usize,
    source: SourceRange,
    phase: Phase,
    program: nir::contracts::ProgramContract,
    environment: Environment,
    callee: Option<Value>,
    arguments: Vec<Value>,
    result: Option<Value>,
    result_slot: usize,
}
pub(super) enum Step {
    Suspend(Progress),
    Complete(Value),
}
impl std::fmt::Display for InterfaceCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phase = match self.phase {
            Phase::Entry => "entry",
            Phase::Body => "body",
            Phase::Exit => "exit",
            Phase::Done => "done",
        };
        write!(
            f,
            "(interface-call (destination %{}) (phase {phase}) (source {} {}))",
            self.destination, self.source.start, self.source.end
        )
    }
}
impl InterfaceCall {
    pub(super) fn new(
        vm: &mut Interpreter<'_>,
        destination: usize,
        source: SourceRange,
        program: nir::contracts::ProgramContract,
        parameters: &[LocalId],
        mut environment: Environment,
        callee: Value,
        arguments: Vec<Value>,
    ) -> Result<Self, Error> {
        for (binding, value) in parameters.iter().zip(&arguments) {
            let observed = match value {
                Value::Storage(storage) => vm.read_owned_value(storage)?,
                Value::ComptimeReference(reference) => {
                    vm.read_value_reference(reference, source)?
                }
                value => vm.checkpoint_representation(value, source)?,
            };
            environment.insert(*binding, observed);
        }
        for snapshot in &program.entry_snapshots {
            if !parameters.contains(&snapshot.parameter) {
                return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            let value = vm.local_value(&environment, snapshot.parameter, Some(source))?;
            environment.insert(snapshot.binding, value);
        }
        if !program.postconditions.is_empty() {
            vm.require_interface_exit_site(source)?;
        }
        let result_slot = arguments.len();
        Ok(Self {
            destination,
            source,
            phase: Phase::Entry,
            program,
            environment,
            callee: Some(callee),
            arguments,
            result: None,
            result_slot,
        })
    }
    fn checks(
        vm: &mut Interpreter<'_>,
        source: SourceRange,
        conditions: Vec<Expr>,
    ) -> Result<Arc<Expr>, Error> {
        vm.effect_charge(conditions.len().saturating_add(1), Some(source))?;
        Ok(Arc::new(Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Block {
                statements: conditions
                    .into_iter()
                    .map(|condition| Expr {
                        source: condition.source,
                        ty: Term::Primitive(Primitive::Void),
                        kind: K::CheckPredicate(Box::new(condition)),
                    })
                    .collect(),
                result: None,
            },
        }))
    }
    pub(super) fn advance(&mut self, vm: &mut Interpreter<'_>) -> Result<Step, Error> {
        loop {
            match self.phase {
                Phase::Entry => {
                    if self.program.preconditions.is_empty() {
                        self.phase = Phase::Body;
                        continue;
                    }
                    let expression = Self::checks(
                        vm,
                        self.source,
                        std::mem::take(&mut self.program.preconditions),
                    )?;
                    return Ok(Step::Suspend(Progress::Contract {
                        expression,
                        environment: std::mem::take(&mut self.environment),
                    }));
                }
                Phase::Body => {
                    return Ok(Step::Suspend(Progress::Call {
                        callee: self.callee.take().ok_or_else(|| {
                            vm.failure(ErrorKind::InvalidControlFlow, Some(self.source))
                        })?,
                        arguments: std::mem::take(&mut self.arguments),
                    }));
                }
                Phase::Exit => {
                    if self.program.postconditions.is_empty() {
                        self.phase = Phase::Done;
                        continue;
                    }
                    let result = self.result.as_ref().unwrap();
                    vm.retain_prepared(self.result_slot, self.source, result)?;
                    if let Some(binding) = self.program.result_binding {
                        self.environment.insert(binding, result.clone());
                    }
                    let expression = Self::checks(
                        vm,
                        self.source,
                        std::mem::take(&mut self.program.postconditions),
                    )?;
                    return Ok(Step::Suspend(Progress::Contract {
                        expression,
                        environment: std::mem::take(&mut self.environment),
                    }));
                }
                Phase::Done => {
                    vm.transfer_prepared();
                    return Ok(Step::Complete(self.result.take().ok_or_else(|| {
                        vm.failure(ErrorKind::InvalidControlFlow, Some(self.source))
                    })?));
                }
            }
        }
    }
    pub(super) fn receive(
        &mut self,
        result: Result<Flow, Error>,
        environment: Option<Environment>,
    ) -> Result<(), Error> {
        let invalid = || Error {
            kind: ErrorKind::InvalidControlFlow,
            source: Some(self.source),
        };
        let Flow::Value(value) = result? else {
            return Err(invalid());
        };
        match self.phase {
            Phase::Entry | Phase::Exit => {
                if !matches!(value, Value::Unit) {
                    return Err(invalid());
                }
                self.environment = environment.ok_or_else(invalid)?;
                self.phase = if matches!(self.phase, Phase::Entry) {
                    Phase::Body
                } else {
                    Phase::Done
                };
            }
            Phase::Body => {
                self.result = Some(value);
                self.phase = Phase::Exit;
            }
            Phase::Done => return Err(invalid()),
        }
        Ok(())
    }
}
