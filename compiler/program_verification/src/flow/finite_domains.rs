//! Universal program checking over a proved finite type domain. PTC determines
//! only which type substitutions are possible; each value proof stays in flow.
use super::*;
use crate::ptc::{Budget, Context, Formula, Outcome};
use nir::{DeclarationParameter, Primitive as P, Sort};

pub(super) fn complete_literals(
    instances: &mut [Function],
    target: Option<nir::target::TargetData>,
    limits: &mut Limits,
) -> Result<(), Error> {
    for function in instances {
        for value in std::iter::once(&mut function.body)
            .chain(&mut function.preconditions)
            .chain(&mut function.postconditions)
            .chain(function.defaults.iter_mut().map(|(_, value)| value))
        {
            nir::integer_formation::complete(value, target, &mut limits.arithmetic.steps).map_err(
                |message| Error {
                    source: value.source,
                    reason: Reason::IntegerFormation(message),
                },
            )?;
        }
    }
    Ok(())
}

pub(super) fn integer_instances(
    function: &Function,
    limits: &mut Limits,
) -> Result<Option<Vec<Function>>, Error> {
    if function.type_parameters.is_empty() {
        return Ok(None);
    }
    let fail = || Error {
        source: function.body.source,
        reason: Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()),
    };
    let Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Ok(None);
    };
    let mut context = Context::with_scope(vec![parameters.iter().map(|p| p.sort).collect()]);
    let mut fuel = 4096;
    for requirement in &function.requirements {
        let formula = Formula::from_predicate(requirement);
        let before = fuel;
        if !crate::ptc::within_limits(&formula, 128, &mut fuel) {
            return Err(fail());
        }
        limits.step(before - fuel).map_err(|_| fail())?;
        context.assume(formula);
    }
    let prove = |formula, limits: &mut Limits| -> Result<bool, Error> {
        // Bound each query, but charge its actual work on every outcome.
        // Cheap type facts must not consume an entire search allowance.
        let allowance = limits.arithmetic.steps.min(4096);
        let mut budget = Budget {
            steps: allowance,
            ..Default::default()
        };
        let outcome = context.prove_metered(formula, &mut budget);
        limits.step(allowance - budget.steps).map_err(|_| fail())?;
        Ok(matches!(outcome, Outcome::Entailed(_)))
    };
    let mut choices = vec![Vec::new()];
    let mut binders = Vec::new();
    for declaration in &function.type_parameters {
        let subject = Term::Rigid {
            declaration: *declaration,
            sort: Sort::Type,
        };
        // Enumerate type identities, not layouts. Platform integers remain
        // distinct; literal completion and flow use the package's target.
        let full_domain = [
            P::I8,
            P::I16,
            P::I32,
            P::I64,
            P::I128,
            P::U8,
            P::U16,
            P::U32,
            P::U64,
            P::U128,
            P::Usize,
            P::Isize,
        ];
        // Start from equality operands occurring in the requirements, then
        // prove that they cover the parameter. Occurrence alone is not evidence:
        // negations and disjunctions can mention types outside the true domain.
        let mut mentioned = Vec::new();
        for requirement in &function.requirements {
            mentioned_primitives(requirement, &subject, &mut mentioned, limits).map_err(
                |reason| Error {
                    source: function.body.source,
                    reason: Reason::Unknown(reason),
                },
            )?;
        }
        let narrowed: Vec<_> = full_domain
            .iter()
            .copied()
            .filter(|primitive| mentioned.contains(primitive))
            .collect();
        let cover = Formula::Or(
            narrowed
                .iter()
                .map(|p| Formula::equal(subject.clone(), Term::Primitive(*p)))
                .collect(),
        );
        let domain = if !narrowed.is_empty() && prove(cover, limits)? {
            narrowed
        } else {
            full_domain.to_vec()
        };
        let alternatives: Vec<_> = domain
            .iter()
            .map(|p| Formula::equal(subject.clone(), Term::Primitive(*p)))
            .collect();
        if !prove(Formula::Or(alternatives.clone()), limits)? {
            return Ok(None);
        }
        let mut possible = Vec::new();
        for (primitive, equal) in domain.into_iter().zip(alternatives) {
            if !prove(Formula::Not(Box::new(equal)), limits)? {
                possible.push(Term::Primitive(primitive));
            }
        }
        binders.push(DeclarationParameter {
            declaration: *declaration,
            sort: Sort::Type,
        });
        let mut next = Vec::new();
        for prefix in choices {
            for ty in &possible {
                let mut arguments = prefix.clone();
                arguments.push(ty.clone());
                // Prune as each parameter is added. A proved-impossible
                // prefix has no valid completion; Unknown must remain live.
                // Singletons were already checked while selecting `possible`.
                if !prefix.is_empty() {
                    let conjunction = Formula::And(
                        binders
                            .iter()
                            .zip(&arguments)
                            .map(|(binder, ty)| {
                                Formula::equal(
                                    Term::Rigid {
                                        declaration: binder.declaration,
                                        sort: binder.sort,
                                    },
                                    ty.clone(),
                                )
                            })
                            .collect(),
                    );
                    if prove(Formula::Not(Box::new(conjunction)), limits)? {
                        continue;
                    }
                }
                if next.len() == 64 {
                    return Err(fail());
                }
                next.push(arguments);
            }
        }
        choices = next;
    }
    let mut output = Vec::new();
    for arguments in choices {
        nir::copy_budget::charge_function_substitution(
            function,
            &binders,
            &arguments,
            &mut limits.arithmetic.steps,
        )
        .map_err(|_| fail())?;
        let mut instance = function
            .instantiate_parameters(&binders, &arguments)
            .map_err(|_| Error {
                source: function.body.source,
                reason: Reason::InvalidSignature,
            })?;
        instance.type_parameters.clear();
        output.push(instance);
    }
    Ok(Some(output))
}

fn mentioned_primitives(
    predicate: &nir::program::TypePredicate,
    subject: &Term,
    output: &mut Vec<P>,
    limits: &mut Limits,
) -> Result<(), Unknown> {
    limits.step(1)?;
    use nir::program::TypePredicate as Q;
    match predicate {
        Q::Equal(left, right) => {
            let other = if left == subject {
                right
            } else if right == subject {
                left
            } else {
                return Ok(());
            };
            if let Term::Primitive(primitive) = other {
                if !output.contains(primitive) {
                    output.push(*primitive);
                }
            }
        }
        Q::Not(inner) => mentioned_primitives(inner, subject, output, limits)?,
        Q::And(items) | Q::Or(items) => {
            for item in items {
                mentioned_primitives(item, subject, output, limits)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{FunctionPhase, TypePredicate as Q};
    use nir::{DeclarationId, Parameter};

    fn template(count: u32) -> Function {
        let ids: Vec<_> = (1..=count)
            .map(|local| DeclarationId { package: 1, local })
            .collect();
        let types: Vec<_> = ids
            .iter()
            .map(|id| Term::Rigid {
                declaration: *id,
                sort: Sort::Type,
            })
            .collect();
        Function {
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: true,
            is_unsafe: false,
            declaration: DeclarationId {
                package: 1,
                local: 100,
            },
            name: "finite".into(),
            parameters: (0..count)
                .map(|index| LocalId { owner: 100, index })
                .collect(),
            signature: Term::Function {
                parameters: types
                    .iter()
                    .map(|ty| Parameter {
                        sort: Sort::Runtime,
                        ty: ty.clone(),
                    })
                    .collect(),
                result: Box::new(Term::Primitive(P::Bool)),
            },
            requirements: types
                .iter()
                .map(|ty| {
                    Q::Or(vec![
                        Q::Equal(ty.clone(), Term::Primitive(P::I8)),
                        Q::Equal(ty.clone(), Term::Primitive(P::I16)),
                    ])
                })
                .collect(),
            type_parameters: ids,
            body: Expr {
                ty: Term::Primitive(P::Bool),
                source: SourceRange { start: 0, end: 1 },
                kind: K::Literal(Literal::Bool(true)),
            },
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            defaults: vec![],
        }
    }

    #[test]
    fn finite_domains_preserve_correlations_and_recover_after_budget_failure() {
        let mut f = template(2);
        assert_eq!(
            integer_instances(&f, &mut Limits::default())
                .unwrap()
                .unwrap()
                .len(),
            4
        );
        let Term::Function { parameters, .. } = &f.signature else {
            unreachable!()
        };
        f.requirements
            .push(Q::Equal(parameters[0].ty.clone(), parameters[1].ty.clone()));
        let instances = integer_instances(&f, &mut Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(instances.len(), 2);
        for instance in instances {
            assert!(instance.type_parameters.is_empty());
            let Term::Function { parameters, .. } = instance.signature else {
                unreachable!()
            };
            assert_eq!(parameters[0].ty, parameters[1].ty);
            assert!(matches!(parameters[0].ty, Term::Primitive(P::I8 | P::I16)));
        }
        let mut exhausted = Limits::default();
        exhausted.arithmetic.steps = 0;
        assert!(integer_instances(&f, &mut exhausted).is_err());
        assert!(
            integer_instances(&f, &mut Limits::default())
                .unwrap()
                .is_some()
        );
        assert!(integer_instances(&template(7), &mut Limits::default()).is_err());
        f.requirements.clear();
        assert!(
            integer_instances(&f, &mut Limits::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn platform_domains_remain_distinct_from_fixed_width_domains() {
        let mut function = template(1);
        let Term::Function { parameters, .. } = &function.signature else {
            unreachable!()
        };
        let domain = [P::Usize, P::Isize, P::U64, P::I64];
        function.requirements = vec![Q::Or(
            domain
                .iter()
                .map(|primitive| Q::Equal(parameters[0].ty.clone(), Term::Primitive(*primitive)))
                .collect(),
        )];
        let instances = integer_instances(&function, &mut Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(instances.len(), domain.len());
        for primitive in domain {
            assert_eq!(
                instances
                    .iter()
                    .filter(|instance| {
                        let Term::Function { parameters, .. } = &instance.signature else {
                            unreachable!()
                        };
                        parameters[0].ty == Term::Primitive(primitive)
                    })
                    .count(),
                1
            );
        }
    }

    #[test]
    fn six_signed_alternatives_do_not_admit_unsigned_proof_cases() {
        let mut function = template(1);
        let Term::Function { parameters, .. } = &function.signature else {
            unreachable!()
        };
        let domain = [P::I8, P::I16, P::I32, P::I64, P::I128, P::Isize];
        function.requirements = vec![Q::Or(
            domain
                .iter()
                .map(|primitive| Q::Equal(parameters[0].ty.clone(), Term::Primitive(*primitive)))
                .collect(),
        )];
        let instances = integer_instances(&function, &mut Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(instances.len(), domain.len());
        for primitive in domain {
            assert_eq!(
                instances
                    .iter()
                    .filter(|instance| {
                        let Term::Function { parameters, .. } = &instance.signature else {
                            unreachable!()
                        };
                        parameters[0].ty == Term::Primitive(primitive)
                    })
                    .count(),
                1
            );
        }
    }

    #[test]
    fn mentioned_candidates_require_a_proved_cover_and_aliases_use_fallback() {
        let mut f = template(1);
        let Term::Function { parameters, .. } = &f.signature else {
            unreachable!()
        };
        f.requirements = vec![Q::Not(Box::new(Q::Equal(
            parameters[0].ty.clone(),
            Term::Primitive(P::I8),
        )))];
        assert!(
            integer_instances(&f, &mut Limits::default())
                .unwrap()
                .is_none()
        );

        let mut f = template(2);
        let Term::Function { parameters, .. } = &f.signature else {
            unreachable!()
        };
        f.requirements = vec![
            Q::Equal(parameters[0].ty.clone(), parameters[1].ty.clone()),
            Q::Equal(parameters[1].ty.clone(), Term::Primitive(P::I8)),
        ];
        let instances = integer_instances(&f, &mut Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(instances.len(), 1);
        let Term::Function { parameters, .. } = &instances[0].signature else {
            unreachable!()
        };
        assert!(parameters.iter().all(|p| p.ty == Term::Primitive(P::I8)));
    }

    #[test]
    fn correlated_domains_are_pruned_before_the_instance_limit() {
        let mut f = template(8);
        let Term::Function { parameters, .. } = &f.signature else {
            unreachable!()
        };
        for parameter in &parameters[1..] {
            f.requirements
                .push(Q::Equal(parameters[0].ty.clone(), parameter.ty.clone()));
        }
        let instances = integer_instances(&f, &mut Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(instances.len(), 2);
        for instance in instances {
            let Term::Function { parameters, .. } = instance.signature else {
                unreachable!()
            };
            assert!(parameters.iter().all(|p| p.ty == parameters[0].ty));
        }
    }
}
