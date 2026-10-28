//! Finite scalar type cases for behavior checking. PTC proves only that the
//! declared type premises cover these cases; it proves no program property.
use super::*;

fn prove_case(
    context: &crate::ptc::Context,
    goal: crate::ptc::Formula,
    fuel: &mut usize,
) -> crate::ptc::Outcome {
    // An optional impossibility query must not spend the entire behavior
    // analysis budget trying to construct a countermodel for a viable case.
    let allowance = (*fuel).min(4096);
    let mut budget = crate::ptc::Budget {
        steps: allowance,
        ..Default::default()
    };
    let result = context.prove_metered(goal, &mut budget);
    *fuel -= allowance - budget.steps;
    result
}

// Reduce only ground primitive type propositions for this prefix. Unknown
// predicates (including opaque heads) still go through PTC. This avoids
// searching for a countermodel of every already satisfied scalar alternative.
fn primitive_truth(
    predicate: &n::TypePredicate,
    parameters: &[Term],
    inputs: &[Term],
    fuel: &mut usize,
    depth: usize,
) -> Option<Option<bool>> {
    *fuel = fuel.checked_sub(1)?;
    if depth >= 128 {
        return Some(None);
    }
    let primitive = |term: &Term, fuel: &mut usize| -> Option<Option<P>> {
        match term {
            Term::Primitive(p) => Some(Some(*p)),
            Term::Rigid {
                declaration,
                sort: Sort::Type,
            } => {
                *fuel = fuel.checked_sub(parameters.len())?;
                Some(parameters.iter().position(|parameter| {
                    matches!(parameter, Term::Rigid { declaration: other, sort: Sort::Type } if other == declaration)
                }).and_then(|index| inputs.get(index)).and_then(|ty| {
                    if let Term::Primitive(p) = ty { Some(*p) } else { None }
                }))
            }
            _ => Some(None),
        }
    };
    Some(match predicate {
        n::TypePredicate::Bool(value) => Some(*value),
        n::TypePredicate::Equal(a, b) => match (primitive(a, fuel)?, primitive(b, fuel)?) {
            (Some(a), Some(b)) => Some(a == b),
            _ => None,
        },
        n::TypePredicate::Not(inner) => {
            primitive_truth(inner, parameters, inputs, fuel, depth + 1)?.map(|value| !value)
        }
        n::TypePredicate::And(parts) | n::TypePredicate::Or(parts) => {
            let conjunction = matches!(predicate, n::TypePredicate::And(_));
            let mut unknown = false;
            for part in parts {
                match primitive_truth(part, parameters, inputs, fuel, depth + 1)? {
                    Some(value) if value != conjunction => return Some(Some(value)),
                    Some(_) => {}
                    None => unknown = true,
                }
            }
            (!unknown).then_some(conjunction)
        }
        _ => None,
    })
}

// A positive equality premise already fixes an input to an earlier input's
// case. Do not rediscover that identity by searching every Cartesian prefix.
// Equalities beneath alternatives or negation are not unconditional premises.
fn given_alias(
    function: &n::Function,
    parameter: &Term,
    earlier: &[Term],
    fuel: &mut usize,
) -> Option<usize> {
    let mut pending: Vec<_> = function.requirements.iter().collect();
    while let Some(requirement) = pending.pop() {
        *fuel = fuel.checked_sub(1)?;
        match requirement {
            n::TypePredicate::And(parts) => pending.extend(parts),
            n::TypePredicate::Equal(a, b) => {
                let other = if a == parameter {
                    b
                } else if b == parameter {
                    a
                } else {
                    continue;
                };
                *fuel = fuel.checked_sub(earlier.len())?;
                if let Some(index) = earlier.iter().position(|ty| ty == other) {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn cases(
    function: &n::Function,
    parameters: &[Term],
    fuel: &mut usize,
) -> Option<Vec<Vec<Term>>> {
    if parameters.is_empty() || parameters.iter().any(|t| t.sort() != Sort::Type) {
        return None;
    }
    let Term::Function {
        parameters: runtime,
        ..
    } = function.signature.callable_signature()
    else {
        return None;
    };
    let mut context =
        crate::ptc::Context::with_scope(vec![runtime.iter().map(|p| p.sort).collect()]);
    let mut candidates = Vec::new();
    for requirement in &function.requirements {
        nir::copy_budget::charge_predicate_substitution(requirement, &[], &[], fuel).ok()?;
        context.assume(crate::ptc::Formula::from_predicate(requirement));
        let mut pending = vec![requirement];
        while let Some(predicate) = pending.pop() {
            *fuel = fuel.checked_sub(1)?;
            match predicate {
                n::TypePredicate::Equal(a, b) => {
                    for ty in [a, b] {
                        if let Term::Primitive(p) = ty {
                            if (p.fixed_integer_layout().is_some()
                                // Classifying a platform scalar's behavior does not
                                // select its layout. Execution still needs target data.
                                || matches!(p, P::Bool | P::Char | P::F16 | P::Bf16 | P::F32 | P::F64 | P::Usize | P::Isize))
                                && !candidates.contains(ty)
                            {
                                candidates.push(ty.clone());
                            }
                        }
                    }
                }
                n::TypePredicate::And(items) | n::TypePredicate::Or(items) => pending.extend(items),
                n::TypePredicate::Not(inner) => pending.push(inner),
                _ => {}
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    let mut result: Vec<Vec<Term>> = vec![vec![]];
    for (index, parameter) in parameters.iter().enumerate() {
        if let Some(alias) = given_alias(function, parameter, &parameters[..index], fuel) {
            *fuel = fuel.checked_sub(result.len())?;
            for inputs in &mut result {
                inputs.push(inputs[alias].clone());
            }
            continue;
        }
        let goal = crate::ptc::Formula::Or(
            candidates
                .iter()
                .map(|ty| crate::ptc::Formula::equal(parameter.clone(), ty.clone()))
                .collect(),
        );
        let outcome = prove_case(&context, goal, fuel);
        if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
            return None;
        }
        let mut next = Vec::new();
        for prefix in result {
            for ty in &candidates {
                *fuel = fuel.checked_sub(index + 1)?;
                let mut inputs = prefix.clone();
                inputs.push(ty.clone());
                let mut known = true;
                let mut excluded = false;
                for requirement in &function.requirements {
                    match primitive_truth(requirement, parameters, &inputs, fuel, 0)? {
                        Some(false) => {
                            excluded = true;
                            break;
                        }
                        Some(true) => {}
                        None => known = false,
                    }
                }
                if excluded {
                    continue;
                }
                if !known {
                    let goal = crate::ptc::Formula::Not(Box::new(crate::ptc::Formula::And(
                        parameters
                            .iter()
                            .zip(&inputs)
                            .map(|(parameter, ty)| {
                                crate::ptc::Formula::equal(parameter.clone(), ty.clone())
                            })
                            .collect(),
                    )));
                    let outcome = prove_case(&context, goal, fuel);
                    // Only a proof excludes a prefix. In particular, Unknown
                    // cannot silently reduce the behavior-checking obligation.
                    if matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
                        continue;
                    }
                }
                if next.len() == 64 {
                    return None;
                }
                next.push(inputs);
            }
        }
        result = next;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function() -> (n::Function, Term) {
        let id = DeclarationId {
            package: 7,
            local: 1,
        };
        let ty = Term::Rigid {
            declaration: id,
            sort: Sort::Type,
        };
        (
            n::Function {
                declaration: DeclarationId {
                    package: 7,
                    local: 2,
                },
                name: "finite".into(),
                phase: n::FunctionPhase::Runtime,
                inline: false,
                pure: true,
                is_unsafe: false,
                type_parameters: vec![id],
                parameters: vec![],
                defaults: vec![],
                requirements: vec![n::TypePredicate::Or(vec![
                    n::TypePredicate::Equal(ty.clone(), p(P::F32)),
                    n::TypePredicate::Equal(ty.clone(), p(P::F64)),
                ])],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                entry_snapshots: vec![],
                result_binding: None,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(p(P::Bool)),
                },
                body: n::Expr {
                    source: n::SourceRange { start: 0, end: 1 },
                    ty: p(P::Bool),
                    kind: K::Literal(n::Literal::Bool(true)),
                },
            },
            ty,
        )
    }

    #[test]
    fn type_case_coverage_is_budgeted_and_recovers_without_assuming_exhaustiveness() {
        let (mut function, ty) = function();
        assert!(cases(&function, &[ty.clone()], &mut 0).is_none());
        let covered = cases(&function, &[ty.clone()], &mut 10000).unwrap();
        assert_eq!(covered.len(), 2);
        assert!(covered.contains(&vec![p(P::F32)]) && covered.contains(&vec![p(P::F64)]));
        function.requirements.clear();
        assert!(cases(&function, &[ty.clone()], &mut 10000).is_none());
        // A type parameter equal to an opaque head is not its body's result.
        function.requirements.push(n::TypePredicate::Or(vec![
            n::TypePredicate::Equal(ty.clone(), p(P::F32)),
            n::TypePredicate::Equal(
                ty.clone(),
                Term::NormalForm {
                    head: DeclarationId {
                        package: 7,
                        local: 3,
                    },
                    sort: Sort::Type,
                    arguments: vec![p(P::F64)],
                },
            ),
        ]));
        assert!(cases(&function, &[ty], &mut 10000).is_none());
    }

    #[test]
    fn complete_integer_domain_reduction_keeps_exclusions_with_bounded_work() {
        let (mut function, ty) = function();
        let domain = [
            P::U8,
            P::U16,
            P::U32,
            P::U64,
            P::U128,
            P::I8,
            P::I16,
            P::I32,
            P::I64,
            P::I128,
            P::Usize,
            P::Isize,
        ];
        function.requirements = vec![n::TypePredicate::Or(
            domain
                .iter()
                .map(|primitive| n::TypePredicate::Equal(ty.clone(), p(*primitive)))
                .collect(),
        )];
        let result = cases(&function, &[ty.clone()], &mut 5000).unwrap();
        assert_eq!(result.len(), domain.len());
        for primitive in domain {
            assert!(result.contains(&vec![p(primitive)]));
        }
        function
            .requirements
            .push(n::TypePredicate::Not(Box::new(n::TypePredicate::Equal(
                ty.clone(),
                p(P::Usize),
            ))));
        let result = cases(&function, &[ty.clone()], &mut 5000).unwrap();
        assert_eq!(result.len(), 11);
        assert!(!result.contains(&vec![p(P::Usize)]));
        assert!(result.contains(&vec![p(P::U64)]));
        assert!(cases(&function, &[ty.clone()], &mut 0).is_none());
        assert_eq!(cases(&function, &[ty], &mut 5000).unwrap(), result);
    }

    #[test]
    fn primitive_reduction_preserves_unknown_heads_and_partial_inputs() {
        let (_, ty) = function();
        let opaque = Term::NormalForm {
            head: DeclarationId {
                package: 7,
                local: 99,
            },
            sort: Sort::Type,
            arguments: vec![p(P::Usize)],
        };
        let unknown = n::TypePredicate::Equal(ty.clone(), opaque);
        let equal = n::TypePredicate::Equal(ty.clone(), p(P::Usize));
        assert_eq!(
            primitive_truth(&unknown, &[ty.clone()], &[p(P::Usize)], &mut 100, 0),
            Some(None)
        );
        assert_eq!(
            primitive_truth(&equal, &[ty.clone()], &[], &mut 100, 0),
            Some(None)
        );
        let either = n::TypePredicate::Or(vec![unknown.clone(), equal.clone()]);
        assert_eq!(
            primitive_truth(&either, &[ty.clone()], &[p(P::Usize)], &mut 100, 0),
            Some(Some(true))
        );
        let both = n::TypePredicate::And(vec![unknown, n::TypePredicate::Not(Box::new(equal))]);
        assert_eq!(
            primitive_truth(&both, &[ty.clone()], &[p(P::Usize)], &mut 100, 0),
            Some(Some(false))
        );
        assert_eq!(
            primitive_truth(&both, &[ty], &[p(P::Usize)], &mut 0, 0),
            None
        );
    }

    #[test]
    fn platform_purity_cases_preserve_type_identity_without_layout_selection() {
        let (mut function, ty) = function();
        let primitives = [P::Usize, P::Isize, P::U64, P::I64];
        function.requirements = vec![n::TypePredicate::Or(
            primitives
                .iter()
                .map(|primitive| n::TypePredicate::Equal(ty.clone(), p(*primitive)))
                .collect(),
        )];
        assert!(cases(&function, &[ty.clone()], &mut 0).is_none());
        let covered = cases(&function, &[ty], &mut 100000).unwrap();
        assert_eq!(covered.len(), primitives.len());
        for primitive in primitives {
            assert!(covered.contains(&vec![p(primitive)]));
        }
    }

    #[test]
    fn correlated_purity_inputs_do_not_consume_impossible_cases() {
        let (mut function, first) = function();
        let mut inputs = vec![first.clone()];
        for local in 10..17 {
            let declaration = DeclarationId { package: 7, local };
            let ty = Term::Rigid {
                declaration,
                sort: Sort::Type,
            };
            function.type_parameters.push(declaration);
            function
                .requirements
                .push(n::TypePredicate::Equal(first.clone(), ty.clone()));
            inputs.push(ty);
        }
        assert!(cases(&function, &inputs, &mut 0).is_none());
        let result = cases(&function, &inputs, &mut 100000).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&vec![p(P::F32); 8]));
        assert!(result.contains(&vec![p(P::F64); 8]));
        function.requirements.truncate(1);
        assert!(cases(&function, &inputs, &mut 100000).is_none());
    }

    #[test]
    fn conditional_equalities_do_not_alias_purity_cases() {
        let (mut function, first) = function();
        let declaration = DeclarationId {
            package: 7,
            local: 20,
        };
        let second = Term::Rigid {
            declaration,
            sort: Sort::Type,
        };
        function.type_parameters.push(declaration);
        function.requirements.push(n::TypePredicate::Or(vec![
            n::TypePredicate::Equal(second.clone(), p(P::F32)),
            n::TypePredicate::Equal(second.clone(), p(P::F64)),
        ]));
        let equality = n::TypePredicate::Equal(first.clone(), second.clone());
        function.requirements.push(n::TypePredicate::Or(vec![
            equality.clone(),
            n::TypePredicate::Bool(true),
        ]));
        let inputs = [first, second];
        assert_eq!(cases(&function, &inputs, &mut 100000).unwrap().len(), 4);
        *function.requirements.last_mut().unwrap() =
            n::TypePredicate::Not(Box::new(equality.clone()));
        let distinct = cases(&function, &inputs, &mut 100000).unwrap();
        assert_eq!(distinct.len(), 2);
        assert!(distinct.iter().all(|case| case[0] != case[1]));
        *function.requirements.last_mut().unwrap() = n::TypePredicate::And(vec![equality]);
        let same = cases(&function, &inputs, &mut 100000).unwrap();
        assert_eq!(same.len(), 2);
        assert!(same.iter().all(|case| case[0] == case[1]));
    }
}
