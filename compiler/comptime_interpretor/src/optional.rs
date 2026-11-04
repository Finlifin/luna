//! Optional construction preserves the whole input, including inner absence.
use super::*;

impl Interpreter<'_> {
    pub(super) fn prepare_optional(
        &mut self,
        ty: &Term,
        input: Option<(&Term, SourceRange)>,
        source: SourceRange,
    ) -> Result<Term, Error> {
        let ty = self.selected_type_view(ty, Some(source))?;
        let Term::Optional(payload_ty) = &ty else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        self.check_comptime_domain(&ty, source)?;
        self.admit_type(&ty, 0)?;
        if let Some((input, at)) = input {
            let input = self.selected_type_view(input, Some(at))?;
            if &input != payload_ty.as_ref() {
                return Err(self.failure(ErrorKind::Type, Some(at)));
            }
        }
        Ok(ty)
    }

    pub(super) fn complete_optional(
        &mut self,
        ty: &Term,
        value: Option<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Term::Optional(payload_ty) = ty else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if let Some(value) = &value {
            self.check_value_type(value, payload_ty, Some(source))?;
        }
        Ok(Value::Optional {
            ty: ty.clone(),
            value: value.map(Box::new),
        })
    }

    pub(super) fn eval_optional(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let operand = match &expr.kind {
                K::OptionalAbsent => None,
                K::OptionalPresent(operand) => Some(operand.as_ref()),
                _ => unreachable!(),
            };
            let ty =
                this.prepare_optional(&expr.ty, operand.map(|v| (&v.ty, v.source)), expr.source)?;
            let value = match operand {
                Some(operand) => Some(value!(this.eval(operand, environment))),
                None => None,
            };
            let at = operand.map_or(expr.source, |v| v.source);
            let result = this.complete_optional(&ty, value, at)?;
            if let Value::Optional {
                value: Some(value), ..
            } = &result
            {
                this.retain_prepared(0, at, value)?;
            }
            Ok(Flow::Value(result))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: SOURCE,
            ty,
            kind,
        }
    }

    #[test]
    fn associated_optional_domains_preserve_layers_and_recover_after_rejection() {
        let package = Package::default();
        let boolean = Term::Primitive(Primitive::Bool);
        let projection = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: DeclarationId {
                    package: 1,
                    local: 80,
                },
                member: DeclarationId {
                    package: 1,
                    local: 81,
                },
            }),
            arguments: vec![],
            subject: Box::new(boolean.clone()),
        };
        let declared = Term::Optional(Box::new(projection.clone()));
        let observed = Term::Optional(Box::new(boolean.clone()));
        let resolver = |_: &Expr, fuel: &mut u64| {
            *fuel -= 1;
            Ok(AssociatedSelection {
                initializer: nir::associated_initializers::AssociatedInitializer::new(
                    expr(
                        Term::Primitive(Primitive::Type),
                        K::TypeValue(boolean.clone()),
                    ),
                    &mut 1000,
                )
                .unwrap(),
                requires_invocation: true,
            })
        };
        let binding = LocalId {
            owner: 82,
            index: 0,
        };
        let present = expr(
            declared.clone(),
            K::OptionalPresent(Box::new(expr(projection, K::Local(binding)))),
        );
        let absent = expr(declared.clone(), K::OptionalAbsent);
        let nested = expr(
            Term::Optional(Box::new(declared.clone())),
            K::OptionalPresent(Box::new(absent.clone())),
        );
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .with_associated_resolver(&resolver);
            let mut env = Environment::from([(binding, Value::Char('x'))]);
            let run = |vm: &mut Interpreter<'_>, body: &Expr, env: &mut Environment| {
                if compiled {
                    let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
                    vm.execute_code(&code, env)
                } else {
                    vm.eval(body, env)
                }
            };
            assert_eq!(
                run(&mut vm, &present, &mut env).unwrap_err().kind,
                ErrorKind::Type
            );
            assert_eq!(env[&binding], Value::Char('x'));
            env.insert(binding, Value::Bool(true));
            for (body, expected) in [
                (
                    present.clone(),
                    Value::Optional {
                        ty: observed.clone(),
                        value: Some(Box::new(Value::Bool(true))),
                    },
                ),
                (
                    absent.clone(),
                    Value::Optional {
                        ty: observed.clone(),
                        value: None,
                    },
                ),
                (
                    nested.clone(),
                    Value::Optional {
                        ty: Term::Optional(Box::new(observed.clone())),
                        value: Some(Box::new(Value::Optional {
                            ty: observed.clone(),
                            value: None,
                        })),
                    },
                ),
            ] {
                let Flow::Value(value) = run(&mut vm, &body, &mut env).unwrap() else {
                    panic!()
                };
                assert_eq!(value, expected);
            }
            assert!(vm.active_associated_initializers.is_empty());
        }
    }
}
