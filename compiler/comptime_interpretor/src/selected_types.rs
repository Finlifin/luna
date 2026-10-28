//! Type views use only compiler-selected associated values. No implementation
//! search, default substitution or normal-form head evaluation occurs here.
use super::*;
type Values<'a> = HashMap<(InterfaceInstance, Term, DeclarationId), &'a Expr>;

impl Interpreter<'_> {
    pub(super) fn selected_type_view(
        &mut self,
        ty: &Term,
        source: Option<SourceRange>,
    ) -> Result<Term, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let selected = view_budgeted(ty, &self.associated_values, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        selected.map_err(|kind| self.failure(kind, source))
    }
}

pub(super) fn same(a: &Term, b: &Term, values: &Values<'_>) -> bool {
    if a == b {
        return true;
    }
    match (view(a, values), view(b, values)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
pub(super) fn view(ty: &Term, values: &Values<'_>) -> Result<Term, ErrorKind> {
    normalize(ty, values, &mut 100_000, 0)
}
/// Invocation-owned budget for explicit type observation. Admission remains
/// the caller's responsibility; this uses only already selected evidence.
pub(super) fn view_budgeted(
    ty: &Term,
    values: &Values<'_>,
    fuel: &mut usize,
) -> Result<Term, ErrorKind> {
    charge_copy(ty, fuel)?;
    normalize(ty, values, fuel, 0)
}
fn charge_copy(ty: &Term, fuel: &mut usize) -> Result<(), ErrorKind> {
    nir::copy_budget::charge_type_substitution(ty, &[], &[], fuel).map_err(|error| match error {
        nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
        nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
    })
}
pub(super) fn predicate(
    input: &nir::program::TypePredicate,
    values: &Values<'_>,
    fuel: &mut usize,
    depth: usize,
) -> Result<nir::program::TypePredicate, ErrorKind> {
    use nir::program::TypePredicate as P;
    if depth >= 64 {
        return Err(ErrorKind::DepthExhausted);
    }
    *fuel = fuel.checked_sub(1).ok_or(ErrorKind::StepsExhausted)?;
    Ok(match input {
        P::Bool(value) => P::Bool(*value),
        P::Trait(requirement) => {
            if requirement.interface_arguments.len() > *fuel {
                return Err(ErrorKind::StepsExhausted);
            }
            P::Trait(nir::program::TraitRequirement {
                interface: requirement.interface,
                subject: normalize(&requirement.subject, values, fuel, depth + 1)?,
                interface_arguments: requirement
                    .interface_arguments
                    .iter()
                    .map(|term| normalize(term, values, fuel, depth + 1))
                    .collect::<Result<_, _>>()?,
            })
        }
        P::Equal(a, b) => P::Equal(
            normalize(a, values, fuel, depth + 1)?,
            normalize(b, values, fuel, depth + 1)?,
        ),
        P::Not(inner) => P::Not(Box::new(predicate(inner, values, fuel, depth + 1)?)),
        P::And(items) | P::Or(items) => {
            if items.len() > *fuel {
                return Err(ErrorKind::StepsExhausted);
            }
            let output = items
                .iter()
                .map(|item| predicate(item, values, fuel, depth + 1))
                .collect::<Result<_, _>>()?;
            if matches!(input, P::And(_)) {
                P::And(output)
            } else {
                P::Or(output)
            }
        }
    })
}

fn normalize(
    ty: &Term,
    values: &Values<'_>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Term, ErrorKind> {
    if depth >= 64 {
        return Err(ErrorKind::DepthExhausted);
    }
    *fuel = fuel.checked_sub(1).ok_or(ErrorKind::StepsExhausted)?;
    let many = |items: &[Term], fuel: &mut usize| -> Result<Vec<Term>, ErrorKind> {
        if items.len() > *fuel {
            return Err(ErrorKind::StepsExhausted);
        }
        items
            .iter()
            .map(|ty| normalize(ty, values, fuel, depth + 1))
            .collect()
    };
    Ok(match ty {
        Term::ErrorResult { errors, result } => Term::error_result(
            many(errors, fuel)?,
            normalize(result, values, fuel, depth + 1)?,
        ),
        Term::Computation { effects, result } => {
            *fuel = fuel
                .checked_sub(effects.len())
                .ok_or(ErrorKind::StepsExhausted)?;
            let operations = effects
                .iter()
                .map(|operation| {
                    Ok(nir::effects::Operation {
                        declaration: operation.declaration,
                        arguments: many(&operation.arguments, fuel)?,
                    })
                })
                .collect::<Result<Vec<_>, ErrorKind>>()?;
            Term::computation(operations, normalize(result, values, fuel, depth + 1)?)
        }
        Term::AssociatedType {
            origin,
            arguments,
            subject,
        } => {
            let interface = InterfaceInstance {
                declaration: origin.interface,
                arguments: many(arguments, fuel)?,
            };
            let subject = normalize(subject, values, fuel, depth + 1)?;
            let value = values
                .get(&(interface, subject, origin.member))
                .ok_or(ErrorKind::UnmaterializedDispatch)?;
            if value.ty != Term::Primitive(Primitive::Type) {
                return Err(ErrorKind::Type);
            }
            let K::TypeValue(ty) = &value.kind else {
                return Err(ErrorKind::Type);
            };
            charge_copy(ty, fuel)?;
            normalize(ty, values, fuel, depth + 1)?
        }
        Term::Nominal {
            declaration,
            arguments,
        } => Term::Nominal {
            declaration: *declaration,
            arguments: many(arguments, fuel)?,
        },
        Term::NormalForm {
            head,
            sort,
            arguments,
        } => Term::NormalForm {
            head: *head,
            sort: *sort,
            arguments: many(arguments, fuel)?,
        },
        Term::FunctionValue {
            declaration,
            arguments,
        } => Term::FunctionValue {
            declaration: *declaration,
            arguments: many(arguments, fuel)?,
        },
        Term::ClosureSnapshot { factory, arguments } => Term::ClosureSnapshot {
            factory: *factory,
            arguments: many(arguments, fuel)?,
        },
        Term::Tuple(items) => Term::Tuple(many(items, fuel)?),
        Term::TupleValue(items) => Term::TupleValue(many(items, fuel)?),
        Term::TypeListValue(items) => Term::TypeListValue(many(items, fuel)?),
        Term::ClosureContextValue(items) => Term::ClosureContextValue(many(items, fuel)?),
        Term::ContextualClosure { context, signature } => Term::contextual_closure(
            normalize(context, values, fuel, depth + 1)?,
            normalize(signature, values, fuel, depth + 1)?,
        )
        .map_err(|_| ErrorKind::Type)?,
        Term::TypeListConcat(items) => Term::type_list_concat(many(items, fuel)?),
        Term::Record(fields) => {
            if fields.len() > *fuel {
                return Err(ErrorKind::StepsExhausted);
            }
            Term::Record(
                fields
                    .iter()
                    .map(|(name, ty)| {
                        *fuel = fuel
                            .checked_sub(name.len())
                            .ok_or(ErrorKind::StepsExhausted)?;
                        Ok((name.clone(), normalize(ty, values, fuel, depth + 1)?))
                    })
                    .collect::<Result<_, ErrorKind>>()?,
            )
        }
        Term::OptionalValue { ty, value } => Term::OptionalValue {
            ty: Box::new(normalize(ty, values, fuel, depth + 1)?),
            value: value
                .as_ref()
                .map(|value| normalize(value, values, fuel, depth + 1).map(Box::new))
                .transpose()?,
        },
        Term::ErrorValue { ty, label, value } => {
            let Term::ErrorResult { errors, result } = ty.as_ref() else {
                return Err(ErrorKind::Type);
            };
            Term::error_value(
                many(errors, fuel)?,
                normalize(result, values, fuel, depth + 1)?,
                match label {
                    Some(label) => Some(Box::new(normalize(label, values, fuel, depth + 1)?)),
                    None => None,
                },
                normalize(value, values, fuel, depth + 1)?,
            )
            .map_err(|_| ErrorKind::Type)?
        }
        Term::NominalValue {
            ty,
            variant,
            fields,
        } => Term::NominalValue {
            ty: Box::new(normalize(ty, values, fuel, depth + 1)?),
            variant: *variant,
            fields: many(fields, fuel)?,
        },
        Term::RecordValue(fields) => {
            if fields.len() > *fuel {
                return Err(ErrorKind::StepsExhausted);
            }
            Term::RecordValue(
                fields
                    .iter()
                    .map(|(name, ty)| {
                        *fuel = fuel
                            .checked_sub(name.len())
                            .ok_or(ErrorKind::StepsExhausted)?;
                        Ok((name.clone(), normalize(ty, values, fuel, depth + 1)?))
                    })
                    .collect::<Result<_, ErrorKind>>()?,
            )
        }
        Term::Optional(inner) => {
            Term::Optional(Box::new(normalize(inner, values, fuel, depth + 1)?))
        }
        Term::Pointer(inner) => Term::Pointer(Box::new(normalize(inner, values, fuel, depth + 1)?)),
        Term::ComptimeFunction(inner) => {
            Term::ComptimeFunction(Box::new(normalize(inner, values, fuel, depth + 1)?))
        }
        Term::UnsafeFunction(inner) => {
            Term::UnsafeFunction(Box::new(normalize(inner, values, fuel, depth + 1)?))
        }
        Term::Closure {
            environment,
            signature,
        } => Term::Closure {
            environment: many(environment, fuel)?,
            signature: Box::new(normalize(signature, values, fuel, depth + 1)?),
        },
        Term::Function { parameters, result } => {
            if parameters.len() > *fuel {
                return Err(ErrorKind::StepsExhausted);
            }
            Term::Function {
                parameters: parameters
                    .iter()
                    .map(|p| {
                        Ok(nir::Parameter {
                            sort: p.sort,
                            ty: normalize(&p.ty, values, fuel, depth + 1)?,
                        })
                    })
                    .collect::<Result<_, ErrorKind>>()?,
                result: Box::new(normalize(result, values, fuel, depth + 1)?),
            }
        }
        Term::Primitive(_)
        | Term::ClosureContextType
        | Term::Hole { .. }
        | Term::Bound { .. }
        | Term::Rigid { .. }
        | Term::Runtime(_)
        | Term::Unit
        | Term::Bool(_)
        | Term::Char(_)
        | Term::BitVec(_)
        | Term::Float16(_)
        | Term::BFloat16(_)
        | Term::Float32(_)
        | Term::Float64(_)
        | Term::String(_)
        | Term::Symbol(_)
        | Term::Integer(_)
        | Term::Float(_)
        | Term::Value { .. } => {
            let work = match ty {
                Term::String(s) | Term::Symbol(s) => s.len(),
                Term::Integer(i) => i.bit_len().div_ceil(32),
                Term::Float(value) => value
                    .digits()
                    .len()
                    .saturating_add(value.exponent().bit_len().div_ceil(32)),
                Term::BitVec(v) => v.words().len(),
                _ => 0,
            };
            *fuel = fuel.checked_sub(work).ok_or(ErrorKind::StepsExhausted)?;
            ty.clone()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evaluating_selected_type_chains_consumes_invocation_budget() {
        let package = Package::default();
        let selections: Vec<_> = (2..22)
            .map(|member| {
                value(if member == 21 {
                    Term::Primitive(Primitive::Bool)
                } else {
                    projection(member + 1)
                })
            })
            .collect();
        let expression = value(projection(2));
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (offset, selection) in selections.iter().enumerate() {
            vm.associated_values
                .insert(key(2 + offset as u32), selection);
        }
        vm.budget.steps = 30;
        let result = vm.eval(&expression, &mut Environment::new());
        let Err(error) = result else {
            panic!("selection work escaped the invocation budget")
        };
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(expression.source));
        assert!(vm.remaining_steps() < 30);
        vm.budget.steps = 10_000;
        assert!(matches!(
            vm.eval(&expression, &mut Environment::new()).unwrap(),
            Flow::Value(Value::Type(Term::Primitive(Primitive::Bool)))
        ));
        assert!(vm.events().is_empty());
    }

    #[test]
    fn evaluating_selected_type_reports_missing_evidence_at_source() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let expression = value(projection(2));
        let Err(error) = vm.eval(&expression, &mut Environment::new()) else {
            panic!("missing selection accepted")
        };
        assert_eq!(error.kind, ErrorKind::UnmaterializedDispatch);
        assert_eq!(error.source, Some(expression.source));
    }
    #[test]
    fn callable_value_inputs_resolve_selected_types_without_changing_identity() {
        let selected = value(Term::Primitive(Primitive::Bool));
        let values = HashMap::from([(key(2), &selected)]);
        for input in [
            Term::FunctionValue {
                declaration: id(10),
                arguments: vec![projection(2)],
            },
            Term::ClosureSnapshot {
                factory: id(11),
                arguments: vec![projection(2)],
            },
        ] {
            let expected = match &input {
                Term::FunctionValue { declaration, .. } => Term::FunctionValue {
                    declaration: *declaration,
                    arguments: vec![Term::Primitive(Primitive::Bool)],
                },
                Term::ClosureSnapshot { factory, .. } => Term::ClosureSnapshot {
                    factory: *factory,
                    arguments: vec![Term::Primitive(Primitive::Bool)],
                },
                _ => unreachable!(),
            };
            assert_eq!(view(&input, &values).unwrap(), expected);
            assert_eq!(
                view(&input, &HashMap::new()).unwrap_err(),
                ErrorKind::UnmaterializedDispatch
            );
        }
    }

    #[test]
    fn callable_value_inputs_cannot_hide_work_from_view_budget() {
        for input in [
            Term::FunctionValue {
                declaration: id(10),
                arguments: vec![Term::Bool(true); 100],
            },
            Term::ClosureSnapshot {
                factory: id(11),
                arguments: vec![Term::Bool(true); 100],
            },
        ] {
            assert_eq!(
                normalize(&input, &HashMap::new(), &mut 3, 0).unwrap_err(),
                ErrorKind::StepsExhausted
            );
            assert_eq!(
                normalize(&input, &HashMap::new(), &mut 200, 0).unwrap(),
                input
            );
        }
    }
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }
    fn projection(member: u32) -> Term {
        Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: id(1),
                member: id(member),
            }),
            arguments: vec![],
            subject: Box::new(Term::Primitive(Primitive::Bool)),
        }
    }
    fn value(ty: Term) -> Expr {
        Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::Primitive(Primitive::Type),
            kind: K::TypeValue(ty),
        }
    }
    fn key(member: u32) -> (InterfaceInstance, Term, DeclarationId) {
        (
            InterfaceInstance {
                declaration: id(1),
                arguments: vec![],
            },
            Term::Primitive(Primitive::Bool),
            id(member),
        )
    }
    #[test]
    fn nested_selected_views_keep_opaque_heads_and_function_sorts() {
        let bool_type = Term::Primitive(Primitive::Bool);
        let selected = value(bool_type.clone());
        let values = HashMap::from([(key(2), &selected)]);
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: Term::Tuple(vec![projection(2)]),
            }],
            result: Box::new(projection(2)),
        };
        let expected = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: Term::Tuple(vec![bool_type.clone()]),
            }],
            result: Box::new(bool_type.clone()),
        };
        assert_eq!(view(&signature, &values).unwrap(), expected);
        let opaque = Term::NormalForm {
            head: id(10),
            sort: nir::Sort::Type,
            arguments: vec![projection(2)],
        };
        assert!(!same(&opaque, &bool_type, &values));
        assert!(
            matches!(view(&opaque,&values).unwrap(),Term::NormalForm {head,..} if head==id(10))
        );
        assert!(!same(
            &signature,
            &Term::ComptimeFunction(Box::new(expected)),
            &values
        ));
    }
    #[test]
    fn selected_views_reject_missing_wrong_sort_and_cyclic_entries() {
        assert_eq!(
            view(&projection(2), &HashMap::new()).unwrap_err(),
            ErrorKind::UnmaterializedDispatch
        );
        let mut invalid = value(Term::Primitive(Primitive::Bool));
        invalid.ty = Term::Primitive(Primitive::Bool);
        assert_eq!(
            view(&projection(2), &HashMap::from([(key(2), &invalid)])).unwrap_err(),
            ErrorKind::Type
        );
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let a = value(projection(3));
                let b = value(projection(2));
                assert_eq!(
                    view(&projection(2), &HashMap::from([(key(2), &a), (key(3), &b)])).unwrap_err(),
                    ErrorKind::DepthExhausted
                );
                let good = value(Term::Primitive(Primitive::Bool));
                assert_eq!(
                    view(&projection(2), &HashMap::from([(key(2), &good)])).unwrap(),
                    Term::Primitive(Primitive::Bool)
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn predicate_views_preserve_connectives_and_recover_after_budget_failure() {
        use nir::program::TypePredicate as P;
        let boolean = Term::Primitive(Primitive::Bool);
        let character = Term::Primitive(Primitive::Char);
        let selected = value(boolean.clone());
        let values = HashMap::from([(key(2), &selected)]);
        let input = P::And(vec![
            P::Equal(projection(2), boolean.clone()),
            P::Not(Box::new(P::Equal(projection(2), character.clone()))),
        ]);
        let before = input.clone();
        assert_eq!(
            predicate(&input, &values, &mut 1, 0).unwrap_err(),
            ErrorKind::StepsExhausted
        );
        let expected = P::And(vec![
            P::Equal(boolean.clone(), boolean.clone()),
            P::Not(Box::new(P::Equal(boolean, character))),
        ]);
        let mut fuel = 100;
        assert_eq!(predicate(&input, &values, &mut fuel, 0).unwrap(), expected);
        assert!(fuel < 100);
        assert_eq!(input, before);
    }
    #[test]
    fn nominal_views_preserve_declaration_and_argument_identity() {
        let boolean = Term::Primitive(Primitive::Bool);
        let selected = value(boolean.clone());
        let values = HashMap::from([(key(2), &selected)]);
        let projected = Term::Nominal {
            declaration: id(20),
            arguments: vec![projection(2)],
        };
        let completed = Term::Nominal {
            declaration: id(20),
            arguments: vec![boolean.clone()],
        };
        assert!(same(&projected, &completed, &values));
        assert!(!same(
            &projected,
            &Term::Nominal {
                declaration: id(21),
                arguments: vec![boolean.clone()]
            },
            &values
        ));
        assert!(!same(
            &projected,
            &Term::Nominal {
                declaration: id(20),
                arguments: vec![Term::Primitive(Primitive::Char)]
            },
            &values
        ));
        let opaque = Term::Nominal {
            declaration: id(20),
            arguments: vec![Term::NormalForm {
                head: id(22),
                sort: nir::Sort::Type,
                arguments: vec![boolean],
            }],
        };
        assert!(!same(&projected, &opaque, &values));
    }

    #[test]
    fn selected_error_views_resolve_labels_success_and_missing_evidence() {
        let boolean = Term::Primitive(Primitive::Bool);
        let character = Term::Primitive(Primitive::Char);
        let label = value(boolean.clone());
        let success = value(Term::error_result(vec![character.clone()], boolean.clone()));
        let values = HashMap::from([(key(2), &label), (key(3), &success)]);
        let input = Term::error_result(vec![projection(2), boolean.clone()], projection(3));
        let expected = Term::error_result(vec![boolean.clone(), character], boolean);
        assert_eq!(view(&input, &values).unwrap(), expected);
        assert!(same(&input, &expected, &values));
        assert_eq!(
            view(&input, &HashMap::new()).unwrap_err(),
            ErrorKind::UnmaterializedDispatch
        );
        assert_eq!(
            normalize(&input, &values, &mut 0, 0).unwrap_err(),
            ErrorKind::StepsExhausted
        );
    }

    #[test]
    fn selected_frozen_success_flattens_only_with_matching_domain() {
        let boolean = Term::Primitive(Primitive::Bool);
        let character = Term::Primitive(Primitive::Char);
        let inner_type = Term::error_result(vec![character.clone()], boolean.clone());
        let selected = value(inner_type.clone());
        let values = HashMap::from([(key(2), &selected)]);
        let inner = Term::ErrorValue {
            ty: Box::new(inner_type.clone()),
            label: Some(Box::new(character.clone())),
            value: Box::new(Term::Char('x')),
        };
        let input = Term::ErrorValue {
            ty: Box::new(Term::error_result(vec![boolean.clone()], projection(2))),
            label: None,
            value: Box::new(inner.clone()),
        };
        let expected = Term::ErrorValue {
            ty: Box::new(Term::error_result(
                vec![boolean.clone(), character.clone()],
                boolean.clone(),
            )),
            label: Some(Box::new(character)),
            value: Box::new(Term::Char('x')),
        };
        assert_eq!(view(&input, &values).unwrap(), expected);
        let wrong = value(boolean);
        assert_eq!(
            view(&input, &HashMap::from([(key(2), &wrong)])).unwrap_err(),
            ErrorKind::Type
        );
    }
}
