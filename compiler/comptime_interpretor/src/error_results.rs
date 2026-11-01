//! Error results are tagged data; construction never captures a continuation.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_error_convert(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            this.eval_error_convert_payload(expr, environment)
        })
    }

    fn apply_error_transport(
        &mut self,
        plan: &nir::error_results::RepresentationTransport,
        value: Value,
        source: SourceRange,
        depth: usize,
    ) -> Result<Value, Error> {
        use nir::error_results::RepresentationTransport as Transport;
        self.tick(Some(source))?;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        match (plan, value) {
            (Transport::Identity, value) => Ok(value),
            (
                Transport::Error {
                    source: expected,
                    target,
                    tags,
                },
                Value::ErrorResult { ty, error, value },
            ) => {
                if &ty != expected {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let tag = error.map_or(0, |index| index + 1);
                let tag = *tags
                    .get(tag)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                Ok(Value::ErrorResult {
                    ty: target.clone(),
                    error: tag.checked_sub(1),
                    value,
                })
            }
            (Transport::Tuple(plans), Value::Tuple(values)) if plans.len() == values.len() => {
                let mut result = Vec::new();
                for (plan, value) in plans.iter().zip(values) {
                    result.push(self.apply_error_transport(plan, value, source, depth + 1)?);
                }
                Ok(Value::Tuple(result))
            }
            (Transport::Record(plans), Value::AnonymousRecord(values))
                if plans.len() == values.len() =>
            {
                let mut result = Vec::new();
                for ((expected, plan), (name, value)) in plans.iter().zip(values) {
                    if expected != &name {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    result.push((
                        name,
                        self.apply_error_transport(plan, value, source, depth + 1)?,
                    ));
                }
                Ok(Value::AnonymousRecord(result))
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    fn eval_error_convert_payload(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::ErrorConvert {
            conversion,
            value: operand,
        } = &expr.kind
        else {
            unreachable!()
        };
        let plan =
            self.prepare_error_conversion(*conversion, &operand.ty, &expr.ty, expr.source)?;
        let payload = value!(self.eval(operand, environment));
        if !self.has_type(&payload, &plan.input) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.retain_prepared(0, operand.source, &payload)?;
        self.complete_error_conversion(
            plan.conversion,
            &plan.input,
            &plan.output,
            payload,
            expr.source,
        )
        .map(Flow::Value)
    }

    pub(super) fn prepare_error_conversion(
        &mut self,
        conversion: nir::error_results::Conversion,
        source_ty: &Term,
        target_ty: &Term,
        at: SourceRange,
    ) -> Result<comptime_compiler::ErrorConversion, Error> {
        let was_error = matches!(source_ty, Term::ErrorResult { .. });
        let source_ty = self.selected_type_view(source_ty, Some(at))?;
        let target_ty = self.selected_type_view(target_ty, Some(at))?;
        let conversion = if conversion == nir::error_results::Conversion::Success
            && !was_error
            && matches!(source_ty, Term::ErrorResult { .. })
        {
            nir::error_results::Conversion::Embed
        } else {
            conversion
        };
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let plan = nir::error_results::validate_conversion_with_layouts(
            conversion,
            &source_ty,
            &target_ty,
            &mut fuel,
            &mut |left, right| left == right,
            &|ty| self.tuple_input_elements(ty),
        );
        self.budget.steps -= (before - fuel) as u64;
        plan.map_err(|e| {
            self.failure(
                if e == nir::error_results::Error::Budget {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(at),
            )
        })?;
        self.check_comptime_domain(&source_ty, at)?;
        self.check_comptime_domain(&target_ty, at)?;
        self.admit_type(&target_ty, 0)?;
        Ok(comptime_compiler::ErrorConversion {
            conversion,
            input: source_ty,
            output: target_ty,
        })
    }

    pub(super) fn complete_error_conversion(
        &mut self,
        conversion: nir::error_results::Conversion,
        source_ty: &Term,
        target_ty: &Term,
        payload: Value,
        at: SourceRange,
    ) -> Result<Value, Error> {
        use nir::error_results::Conversion;
        if conversion == Conversion::Product {
            let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let before = fuel;
            let plan = nir::error_results::transport_plan_with_layouts(
                source_ty,
                target_ty,
                true,
                &mut fuel,
                &mut |a, b| a == b,
                &|ty| self.tuple_input_elements(ty),
            );
            self.budget.steps -= (before - fuel) as u64;
            let plan = plan.map_err(|e| {
                self.failure(
                    if e == nir::error_results::Error::Budget {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    },
                    Some(at),
                )
            })?;
            let result = self.apply_error_transport(&plan, payload, at, 0)?;
            if !self.has_type(&result, target_ty) {
                return Err(self.failure(ErrorKind::Type, Some(at)));
            }
            return Ok(result);
        }
        let Term::ErrorResult { errors, .. } = target_ty else {
            unreachable!()
        };
        let (error, value) = match conversion {
            Conversion::Product => unreachable!(),
            Conversion::Success => (None, Box::new(payload)),
            Conversion::Failure => (
                Some(
                    errors
                        .iter()
                        .position(|label| label == source_ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, Some(at)))?,
                ),
                Box::new(payload),
            ),
            Conversion::Embed => {
                let Value::ErrorResult { ty, error, value } = payload else {
                    return Err(self.failure(ErrorKind::Type, Some(at)));
                };
                let Term::ErrorResult { errors: source, .. } = ty else {
                    return Err(self.failure(ErrorKind::Type, Some(at)));
                };
                let error = match error {
                    None => None,
                    Some(index) => {
                        let label = source
                            .get(index)
                            .ok_or_else(|| self.failure(ErrorKind::Type, Some(at)))?;
                        Some(
                            errors
                                .iter()
                                .position(|target| target == label)
                                .ok_or_else(|| self.failure(ErrorKind::Type, Some(at)))?,
                        )
                    }
                };
                (error, value)
            }
        };
        let result = Value::ErrorResult {
            ty: target_ty.clone(),
            error,
            value,
        };
        if !self.has_type(&result, target_ty) {
            return Err(self.failure(ErrorKind::Type, Some(at)));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::error_results::Conversion as C;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: SOURCE,
            ty,
            kind,
        }
    }
    fn tagged(ty: &Term, label: Option<&Term>, payload: Value) -> Value {
        let Term::ErrorResult { errors, .. } = ty else {
            panic!()
        };
        Value::ErrorResult {
            ty: ty.clone(),
            error: label.map(|label| {
                errors
                    .iter()
                    .position(|candidate| candidate == label)
                    .unwrap()
            }),
            value: Box::new(payload),
        }
    }

    #[test]
    fn associated_error_domains_preserve_branches_in_both_interpreters() {
        let package = Package::default();
        let boolean = Term::Primitive(Primitive::Bool);
        let character = Term::Primitive(Primitive::Char);
        let byte = Term::Primitive(Primitive::U8);
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
        let resolved = Term::error_result(vec![character.clone()], boolean.clone());
        let widened = Term::error_result(vec![byte.clone()], resolved.clone());
        let success_target = Term::error_result(vec![character.clone()], projection.clone());
        let failure_target = Term::error_result(vec![projection.clone()], character.clone());
        let resolved_failure = Term::error_result(vec![boolean.clone()], character.clone());
        let embed_target =
            Term::error_result(vec![byte.clone(), projection.clone()], character.clone());
        let embed_observed =
            Term::error_result(vec![byte.clone(), boolean.clone()], character.clone());
        let mut cases = vec![
            (
                boolean.clone(),
                C::Success,
                projection.clone(),
                success_target,
                Value::Bool(true),
                tagged(&resolved, None, Value::Bool(true)),
            ),
            (
                boolean.clone(),
                C::Failure,
                projection.clone(),
                failure_target.clone(),
                Value::Bool(false),
                tagged(&resolved_failure, Some(&boolean), Value::Bool(false)),
            ),
            (
                boolean.clone(),
                C::Embed,
                failure_target,
                embed_target,
                tagged(&resolved_failure, Some(&boolean), Value::Bool(true)),
                tagged(&embed_observed, Some(&boolean), Value::Bool(true)),
            ),
        ];
        for label in [None, Some(&character)] {
            let payload = if label.is_some() {
                Value::Char('x')
            } else {
                Value::Bool(true)
            };
            cases.push((
                resolved.clone(),
                C::Success,
                projection.clone(),
                Term::error_result(vec![byte.clone()], projection.clone()),
                tagged(&resolved, label, payload.clone()),
                tagged(&widened, label, payload),
            ));
        }
        // Closed physical row order is retained until explicit conversion.
        let physical = Term::ErrorResult {
            errors: vec![character.clone(), boolean.clone()],
            result: Box::new(byte),
        };
        let canonical = Term::error_result(
            vec![character.clone(), boolean.clone()],
            Term::Primitive(Primitive::U8),
        );
        cases.push((
            boolean.clone(),
            C::Embed,
            physical.clone(),
            canonical.clone(),
            tagged(&physical, Some(&boolean), Value::Bool(true)),
            tagged(&canonical, Some(&boolean), Value::Bool(true)),
        ));
        let binding = LocalId {
            owner: 82,
            index: 0,
        };
        for (selected, conversion, input, output, value, expected) in cases {
            let resolver = |_: &Expr, fuel: &mut u64| {
                *fuel -= 1;
                Ok(AssociatedSelection {
                    initializer: nir::associated_initializers::AssociatedInitializer::new(
                        expr(
                            Term::Primitive(Primitive::Type),
                            K::TypeValue(selected.clone()),
                        ),
                        &mut 1000,
                    )
                    .unwrap(),
                    requires_invocation: true,
                })
            };
            let body = expr(
                output,
                K::ErrorConvert {
                    conversion,
                    value: Box::new(expr(input, K::Local(binding))),
                },
            );
            for compiled in [false, true] {
                let mut vm = Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .with_associated_resolver(&resolver);
                let mut env = Environment::from([(binding, Value::Unit)]);
                let run = |vm: &mut Interpreter<'_>, env: &mut Environment| {
                    if compiled {
                        let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                        vm.execute_code(&code, env)
                    } else {
                        vm.eval(&body, env)
                    }
                };
                assert_eq!(run(&mut vm, &mut env).unwrap_err().kind, ErrorKind::Type);
                env.insert(binding, value.clone());
                let Flow::Value(actual) = run(&mut vm, &mut env).unwrap() else {
                    panic!()
                };
                assert_eq!(actual, expected);
                assert_eq!(env[&binding], value);
                assert!(vm.active_associated_initializers.is_empty());
            }
        }
    }
}
