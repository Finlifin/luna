//! Detached invocation bodies use ordinary frames without synthetic declarations.
use super::*;

impl Interpreter<'_> {
    /// Invoke a closed, checked comptime expression in this exact source world.
    /// Phase and unsafe-call inspection are repeated for this body; normal
    /// invocation admission, result checking and caller restoration still apply.
    /// No local bindings or graph execution authority are imported by this API.
    pub fn invoke_comptime_expression(&mut self, body: &Expr) -> Result<Value, Error> {
        self.check_comptime_expression(body)?;
        self.charge_copy_work(Some(body.source), |fuel| {
            let mut remaining = *fuel as u64;
            let result = nir::copy_budget::charge_expression(body, &mut remaining);
            *fuel = remaining as usize;
            result
        })?;
        self.invoke_checked_expression(Arc::new(body.clone()))
    }

    /// Retain immutable expression code for repeated host calls without copying
    /// it or inventing a declaration. Every call repeats phase/unsafe inspection
    /// and creates fresh closure state; ownership is not execution permission.
    pub fn invoke_comptime_expression_owned(&mut self, body: Arc<Expr>) -> Result<Value, Error> {
        self.check_comptime_expression(&body)?;
        self.tick(Some(body.source))?;
        self.invoke_checked_expression(body)
    }

    fn check_comptime_expression(&mut self, body: &Expr) -> Result<(), Error> {
        if !self.comptime_only || self.graph.is_some() {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(body.source)));
        }
        let inspection = self
            .phase_inspection
            .as_ref()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(body.source)))?;
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = inspection.check(body, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(Error::phase)?;
        Ok(())
    }

    fn invoke_checked_expression(&mut self, body: Arc<Expr>) -> Result<Value, Error> {
        self.charge_copy_work(Some(body.source), |fuel| {
            nir::copy_budget::charge_type_substitution(&body.ty, &[], &[], fuel)
        })?;
        let result_type = body.ty.clone();
        let function = Value::Closure(Arc::new(Closure {
            definition_source: Some(body.source),
            origin: None,
            instance_arguments: None,
            parameters: vec![],
            environment: Environment::new(),
            capture_order: vec![],
            body,
            ty: Term::ComptimeFunction(Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(result_type),
            })),
        }));
        self.invoke_value(function, vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{Function, FunctionPhase};

    fn flag(value: bool) -> Expr {
        Expr {
            source: SourceRange { start: 10, end: 20 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(value)),
        }
    }

    #[test]
    fn retained_expression_code_rechecks_phase_and_never_reuses_invocation_values() {
        let package = Arc::new(Package::default());
        let body = Arc::new(flag(true));
        let mut session = CodeSession::new(package.clone());
        for round in 0..2 {
            let mut vm = Interpreter::for_comptime_in_session(
                &package,
                &mut session,
                &mut Budget::default(),
            )
            .unwrap();
            assert_eq!(
                vm.with_code_session(&mut session, |vm| {
                    vm.invoke_comptime_expression_owned(body.clone())
                })
                .unwrap(),
                Value::Bool(true)
            );
            if round > 0 {
                assert_eq!(vm.bytecode_compilations(), 0);
            }
            let invalid = Arc::new(Expr {
                kind: K::Local(LocalId {
                    owner: 44,
                    index: 1,
                }),
                ..flag(false)
            });
            assert!(vm.invoke_comptime_expression_owned(invalid).is_err());
            assert_eq!(
                vm.invoke_comptime_expression_owned(Arc::new(flag(false)))
                    .unwrap(),
                Value::Bool(false)
            );
        }
        let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            runtime
                .invoke_comptime_expression_owned(body)
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        );
        assert!(runtime.events().is_empty());
    }

    fn function(local: u32, phase: FunctionPhase, is_unsafe: bool) -> Function {
        Function {
            interface: nir::program::FunctionDeclaration {
                declaration: DeclarationId { package: 1, local },
                name: format!("function{local}"),
                phase,
                inline: false,
                pure: false,
                is_unsafe,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(flag(true).ty),
                },
                parameters: vec![],
                type_parameters: vec![],
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
            body: flag(true),
        }
    }

    fn call(function: &Function) -> Expr {
        let mut ty = function.signature.clone();
        if function.phase == FunctionPhase::Comptime {
            ty = Term::ComptimeFunction(Box::new(ty));
        }
        if function.is_unsafe {
            ty = Term::UnsafeFunction(Box::new(ty));
        }
        Expr {
            kind: K::Call {
                callee: Box::new(Expr {
                    kind: K::Function(function.declaration),
                    ty,
                    ..flag(true)
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
            ..flag(true)
        }
    }

    #[test]
    fn detached_invocations_refresh_code_and_restore_frames_on_every_exit() {
        let package = Package::default();
        let binding = LocalId { owner: 1, index: 0 };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            vm.bytecode_enabled = compiled;
            vm.effect_state.invocation = 7;
            vm.effect_state.resume = Some(9);
            let mut body = flag(true);
            for value in [true, false, true] {
                body.kind = K::Block {
                    statements: vec![Expr {
                        ty: Term::Primitive(Primitive::Void),
                        kind: K::Let {
                            binding,
                            value: Box::new(flag(value)),
                        },
                        ..flag(true)
                    }],
                    result: Some(Box::new(Expr {
                        kind: K::Local(binding),
                        ..flag(true)
                    })),
                };
                assert_eq!(
                    vm.invoke_comptime_expression(&body).unwrap(),
                    Value::Bool(value)
                );
                let unbound = Expr {
                    kind: K::Local(binding),
                    ..flag(true)
                };
                assert_eq!(
                    vm.invoke_comptime_expression(&unbound).unwrap_err().kind,
                    ErrorKind::MissingBinding
                );
                let forged = Expr {
                    ty: Term::Primitive(Primitive::Char),
                    ..flag(true)
                };
                assert_eq!(
                    vm.invoke_comptime_expression(&forged).unwrap_err().kind,
                    ErrorKind::Type
                );
                assert_eq!(vm.depth, 0);
                assert!(vm.prepared_frames.is_empty());
                assert!(vm.cleanup_frame.is_none());
                assert_eq!(vm.effect_state.invocation, 7);
                assert_eq!(vm.effect_state.resume, Some(9));
            }
            body.ty = Term::Primitive(Primitive::Type);
            body.kind = K::TypeValue(Term::Primitive(Primitive::Char));
            assert_eq!(
                vm.invoke_comptime_expression(&body).unwrap(),
                Value::Type(Term::Primitive(Primitive::Char))
            );
            assert_eq!(vm.bytecode_invocations() > 0, compiled);
        }
        assert!(package.functions.is_empty());
    }

    #[test]
    fn detached_code_preserves_phase_and_lexical_unsafe_permissions() {
        let package = Package {
            functions: vec![
                function(0, FunctionPhase::Runtime, false),
                function(1, FunctionPhase::Comptime, true),
            ],
            ..Default::default()
        };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            vm.bytecode_enabled = compiled;
            for function in &package.functions {
                assert!(matches!(
                    vm.invoke_comptime_expression(&call(function))
                        .unwrap_err()
                        .kind,
                    ErrorKind::InvalidPhase(nir::phases::Error {
                        kind: nir::phases::ErrorKind::Invalid,
                        ..
                    })
                ));
            }
            let authorized = Expr {
                kind: K::UnsafeScope(Box::new(call(&package.functions[1]))),
                ..flag(true)
            };
            assert_eq!(
                vm.invoke_comptime_expression(&authorized).unwrap(),
                Value::Bool(true)
            );
            // Lexical permission in a previous body cannot escape to its successor.
            assert!(matches!(
                vm.invoke_comptime_expression(&call(&package.functions[1]))
                    .unwrap_err()
                    .kind,
                ErrorKind::InvalidPhase(_)
            ));
            assert_eq!(
                vm.invoke_comptime_expression(&flag(false)).unwrap(),
                Value::Bool(false)
            );
        }
        let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            runtime
                .invoke_comptime_expression(&flag(true))
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        );
    }

    #[test]
    fn phase_and_execution_exhaustion_remain_resource_failures() {
        let package = Package::default();
        let body = Expr {
            kind: K::Block {
                statements: vec![flag(true); 32],
                result: Some(Box::new(flag(false))),
            },
            ..flag(false)
        };
        for compiled in [false, true] {
            let mut complete = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            complete.bytecode_enabled = compiled;
            let before = complete.remaining_steps();
            assert_eq!(
                complete.invoke_comptime_expression(&body).unwrap(),
                Value::Bool(false)
            );
            let cost = before - complete.remaining_steps();
            for allowance in 0..cost {
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                vm.bytecode_enabled = compiled;
                vm.budget.steps = allowance;
                assert_eq!(
                    vm.invoke_comptime_expression(&body).unwrap_err().kind,
                    ErrorKind::StepsExhausted,
                    "compiled={compiled}, allowance={allowance}"
                );
                assert!(vm.remaining_steps() <= allowance);
                assert_eq!(vm.depth, 0);
                assert!(vm.prepared_frames.is_empty());
                vm.budget.steps = 10_000;
                assert_eq!(
                    vm.invoke_comptime_expression(&flag(true)).unwrap(),
                    Value::Bool(true)
                );
            }
        }
    }
}
