//! Newtype conversions transfer a whole payload, never expose a record field.
use super::*;
impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn eval_newtype_convert(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::NewtypeConvert(operand) = &expr.kind else {
            unreachable!()
        };
        let plan = self.prepare_newtype_conversion(&operand.ty, &expr.ty, expr.source)?;
        let value = value!(self.eval(operand, environment));
        self.complete_newtype_conversion(&plan, value, expr.source)
            .map(Flow::Value)
    }

    pub(super) fn prepare_newtype_conversion(
        &mut self,
        input: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<nir::newtypes::Conversion, Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let plan = if let Some(graph) = self.graph {
            // IIR descriptors are already instantiated, so their nominal keys
            // must select the concrete representation rather than reapplying
            // declaration arguments to an empty telescope.
            graph.newtype_conversion(input, output, &mut fuel)
        } else {
            nir::newtypes::conversion_from(self.newtypes.values(), input, output, &[], &mut fuel)
        };
        self.budget.steps -= (before - fuel) as u64;
        plan.map_err(|e| {
            self.failure(
                match e {
                    nir::newtypes::ConversionError::Budget => ErrorKind::StepsExhausted,
                    _ => ErrorKind::Type,
                },
                Some(source),
            )
        })?
        .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))
    }

    pub(super) fn complete_newtype_conversion(
        &self,
        plan: &nir::newtypes::Conversion,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if !self.has_type(&value, plan.source()) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let value = match plan.direction() {
            nir::newtypes::Direction::Wrap => Value::Newtype {
                ty: plan.target().clone(),
                value: Box::new(value),
            },
            nir::newtypes::Direction::Unwrap => {
                let Value::Newtype { ty, value } = value else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if &ty != plan.source() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                *value
            }
        };
        if !self.has_type(&value, plan.target()) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::Place;
    const SOURCE: SourceRange = SourceRange { start: 3, end: 9 };
    const INPUT: LocalId = LocalId { owner: 1, index: 0 };
    const TRACE: LocalId = LocalId { owner: 1, index: 1 };
    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: SOURCE,
            ty,
            kind,
        }
    }
    fn nominal(local: u32) -> Term {
        Term::Nominal {
            declaration: DeclarationId { package: 0, local },
            arguments: vec![],
        }
    }
    fn query(input: Term, output: Term) -> Expr {
        let boolean = Term::Primitive(Primitive::Bool);
        expr(
            output,
            K::NewtypeConvert(Box::new(expr(
                input.clone(),
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(Primitive::Void),
                        K::Assign {
                            place: Place {
                                root: TRACE,
                                fields: vec![],
                            },
                            value: Box::new(expr(
                                boolean.clone(),
                                K::Not(Box::new(expr(boolean, K::Local(TRACE)))),
                            )),
                        },
                    )],
                    result: Some(Box::new(expr(input, K::Local(INPUT)))),
                },
            ))),
        )
    }
    fn execute(
        vm: &mut Interpreter<'_>,
        body: &Expr,
        code: &Arc<comptime_compiler::Program>,
        env: &mut Environment,
        compiled: bool,
    ) -> Result<Flow, Error> {
        if compiled {
            vm.execute_code(code, env)
        } else {
            vm.eval(body, env)
        }
    }
    #[test]
    fn newtype_vm_rechecks_live_edges_and_preserves_failure_effects() {
        let boolean = Term::Primitive(Primitive::Bool);
        let mut package = Package::default();
        for local in [1, 2] {
            package.newtypes.push(nir::newtypes::Newtype {
                declaration: DeclarationId { package: 0, local },
                name: format!("Wrapper{local}"),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: boolean.clone(),
            });
        }
        let wrap = query(boolean.clone(), nominal(1));
        let wrap_code = Arc::new(comptime_compiler::compile(&wrap, &mut 10000).unwrap());
        let dump = wrap_code.to_string();
        assert!(dump.contains("(prepare-newtype-conversion "), "{dump}");
        assert!(dump.contains("(finish-newtype-conversion "), "{dump}");
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env =
                Environment::from([(INPUT, Value::Bool(true)), (TRACE, Value::Bool(false))]);
            assert!(matches!(
                execute(&mut vm, &wrap, &wrap_code, &mut env, compiled).unwrap(),
                Flow::Value(Value::Newtype { .. })
            ));
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(true)
            );
            let position = Arc::make_mut(&mut vm.newtypes.positions)
                .remove(&package.newtypes[0].declaration)
                .unwrap();
            env.insert(TRACE, Value::Bool(false));
            assert_eq!(
                execute(&mut vm, &wrap, &wrap_code, &mut env, compiled)
                    .unwrap_err()
                    .kind,
                ErrorKind::Type
            );
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(false)
            );
            Arc::make_mut(&mut vm.newtypes.positions)
                .insert(package.newtypes[0].declaration, position);
            for (input, output, value, observed) in [
                (
                    nominal(1),
                    nominal(2),
                    Value::Newtype {
                        ty: nominal(1),
                        value: Box::new(Value::Bool(true)),
                    },
                    false,
                ),
                (
                    nominal(1),
                    boolean.clone(),
                    Value::Newtype {
                        ty: nominal(2),
                        value: Box::new(Value::Bool(true)),
                    },
                    true,
                ),
                (
                    nominal(1),
                    boolean.clone(),
                    Value::Newtype {
                        ty: nominal(1),
                        value: Box::new(Value::UnsignedInteger(1)),
                    },
                    true,
                ),
            ] {
                let body = query(input, output);
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                env.insert(INPUT, value);
                env.insert(TRACE, Value::Bool(false));
                assert_eq!(
                    execute(&mut vm, &body, &code, &mut env, compiled)
                        .unwrap_err()
                        .kind,
                    ErrorKind::Type
                );
                assert_eq!(
                    vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                    Value::Bool(observed)
                );
            }
            env.insert(INPUT, Value::Bool(false));
            env.insert(TRACE, Value::Bool(false));
            assert!(matches!(
                execute(&mut vm, &wrap, &wrap_code, &mut env, compiled).unwrap(),
                Flow::Value(Value::Newtype { .. })
            ));
        }
    }
}
