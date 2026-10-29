//! Invocation-only execution of linear code. Runtime cleanup and mixed-stage
//! interpretation keep their existing paths. No opcode recursively evaluates NIR.
use super::*;
use comptime_compiler::{Op, Program};

struct Frame {
    pc: usize,
    slots: Vec<Option<Value>>,
    scopes: Vec<Vec<usize>>,
}

impl Interpreter<'_> {
    pub fn bytecode_invocations(&self) -> usize {
        self.bytecode_invocations
    }

    pub(super) fn invocation_body(
        &mut self,
        body: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if !self.comptime_only || self.cleanup_schedule.is_some() {
            return self.eval(body, environment);
        }
        // Compilation has its own bounded work budget. Unsupported bodies are
        // selected before executing any instruction, never after partial work.
        let mut fuel = 100_000;
        let program = match comptime_compiler::compile(body, &mut fuel) {
            Ok(program) => program,
            Err(comptime_compiler::Error::Unsupported(_)) => return self.eval(body, environment),
            Err(comptime_compiler::Error::Budget(source)) => {
                return Err(self.failure(ErrorKind::StepsExhausted, Some(source)));
            }
            Err(comptime_compiler::Error::InvalidControl(source)) => {
                return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
            }
        };
        self.bytecode_invocations += 1;
        self.execute_code(&program, environment)
    }

    fn execute_code(
        &mut self,
        code: &Program<'_>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.effect_charge(
            code.registers(),
            code.instructions().first().map(|i| i.source),
        )?;
        let mut frame = Frame {
            pc: 0,
            scopes: Vec::new(),
            slots: (0..code.registers()).map(|_| None).collect(),
        };
        let mut imported = Vec::new();
        for &(binding, slot) in code.locals() {
            if let Some(value) = environment.remove(&binding) {
                imported.push((binding, slot));
                frame.slots[slot] = Some(value);
            }
        }
        let result = (|| {
            loop {
                let instruction = code
                    .instructions()
                    .get(frame.pc)
                    .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, None))?;
                let source = instruction.source;
                self.tick(Some(source))?;
                frame.pc += 1;
                let missing = || Error {
                    kind: ErrorKind::MissingBinding,
                    source: Some(source),
                };
                match &instruction.op {
                    Op::Constant { destination, value } => {
                        let flow = match value.kind {
                            K::Literal(_) => self.eval_literal(value, environment)?,
                            _ => self.eval_type_value(value, environment)?,
                        };
                        let Flow::Value(value) = flow else {
                            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                        };
                        frame.slots[*destination] = Some(value);
                    }
                    Op::EnterScope => frame.scopes.push(Vec::new()),
                    Op::LeaveScope => {
                        let locals = frame.scopes.pop().ok_or_else(|| {
                            self.failure(ErrorKind::InvalidControlFlow, Some(source))
                        })?;
                        for slot in locals {
                            frame.slots[slot] = None;
                        }
                    }
                    Op::Unwind(depth) => {
                        if *depth > frame.scopes.len() {
                            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                        }
                        while frame.scopes.len() > *depth {
                            for slot in frame.scopes.pop().unwrap() {
                                frame.slots[slot] = None;
                            }
                        }
                    }
                    Op::Unit(dst) => frame.slots[*dst] = Some(Value::Unit),
                    Op::Read { destination, local } => {
                        let input = frame.slots[*local].as_ref().ok_or_else(missing)?;
                        let value = self.invocation_slot_value(input, Some(source))?;
                        frame.slots[*destination] = Some(value);
                    }
                    Op::Store {
                        local,
                        value,
                        initialize,
                    } => {
                        let value = frame.slots[*value].take().ok_or_else(missing)?;
                        if *initialize {
                            if frame.slots[*local].is_none() {
                                if let Some(scope) = frame.scopes.last_mut() {
                                    scope.push(*local);
                                }
                            }
                            frame.slots[*local] = Some(value);
                        } else {
                            let target = frame.slots[*local].as_mut().ok_or_else(missing)?;
                            match target {
                                Value::ComptimeReference(reference) => reference
                                    .replace(value, &mut self.budget.steps)
                                    .map_err(|kind| self.failure(kind, Some(source)))?,
                                Value::Storage(storage) => {
                                    value_reference::ValueReference::root(storage.clone())
                                        .replace(value, &mut self.budget.steps)
                                        .map_err(|kind| self.failure(kind, Some(source)))?
                                }
                                target => *target = value,
                            }
                        }
                    }
                    Op::Not {
                        destination,
                        value,
                        ty,
                    } => {
                        let value = Self::concrete_value(
                            frame.slots[*value].take().ok_or_else(missing)?,
                            source,
                        )?;
                        let result = match value {
                            Value::Bool(value) => Value::Bool(!value),
                            Value::BitVec(value)
                                if **ty == Term::Primitive(Primitive::BitVec(value.width())) =>
                            {
                                Value::BitVec(
                                    self.bit_operation(source, |budget| value.not(budget))?,
                                )
                            }
                            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                        };
                        frame.slots[*destination] = Some(result);
                    }
                    Op::Jump(target) => frame.pc = *target,
                    Op::Branch {
                        condition,
                        when,
                        target,
                    } => match frame.slots[*condition].as_ref().ok_or_else(missing)? {
                        Value::Bool(value) => {
                            if value == when {
                                frame.pc = *target;
                            }
                        }
                        Value::AbstractValue { .. } => {
                            return Err(
                                self.failure(ErrorKind::AbstractValueUnresolved, Some(source))
                            );
                        }
                        _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                    },
                    Op::Finish { value, explicit } => {
                        let value = frame.slots[*value].take().ok_or_else(missing)?;
                        return Ok(if *explicit {
                            Flow::Return(value)
                        } else {
                            Flow::Value(value)
                        });
                    }
                }
            }
        })();
        // Contracts run after the body and can observe updated input storage.
        // Locals introduced by the body never escape into the caller frame.
        for (binding, slot) in imported {
            if let Some(value) = frame.slots[slot].take() {
                environment.insert(binding, value);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_frame_preserves_prior_writes_and_does_not_leak_locals() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let input = LocalId {
            owner: 1,
            index: 1000,
        };
        let temporary = LocalId {
            owner: 2,
            index: u32::MAX,
        };
        let expr = |ty, kind| Expr {
            ty,
            kind,
            source: SourceRange { start: 1, end: 2 },
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let void = Term::Primitive(Primitive::Void);
        let body = expr(
            boolean.clone(),
            K::Block {
                statements: vec![
                    expr(
                        void.clone(),
                        K::Assign {
                            place: nir::program::Place {
                                root: input,
                                fields: vec![],
                            },
                            value: Box::new(expr(
                                boolean.clone(),
                                K::Literal(Literal::Bool(false)),
                            )),
                        },
                    ),
                    expr(
                        void.clone(),
                        K::Let {
                            binding: temporary,
                            value: Box::new(expr(void.clone(), K::Unit)),
                        },
                    ),
                ],
                // Deliberately malformed typed input probes the engine's error
                // boundary; source elaboration rejects this operation earlier.
                result: Some(Box::new(expr(
                    boolean.clone(),
                    K::Not(Box::new(expr(void, K::Unit))),
                ))),
            },
        );
        let code = comptime_compiler::compile(&body, &mut 100).unwrap();
        assert!(
            code.registers() < 16,
            "sparse identities must not size the frame"
        );
        let mut environment = Environment::new();
        environment.insert(input, Value::Bool(true));
        assert_eq!(
            vm.execute_code(&code, &mut environment).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(environment.get(&input), Some(&Value::Bool(false)));
        assert!(!environment.contains_key(&temporary));
        let recovered = expr(boolean, K::Local(input));
        let code = comptime_compiler::compile(&recovered, &mut 100).unwrap();
        assert!(matches!(
            vm.execute_code(&code, &mut environment).unwrap(),
            Flow::Value(Value::Bool(false))
        ));
    }
}
