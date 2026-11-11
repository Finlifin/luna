//! Bounded dependency discovery for checked and retained required code. Facts
//! publish only after a complete walk; discovery does not execute producers or
//! authorize opaque pattern evidence for invocation.
use super::*;

fn collect<'a>(
    roots: impl DoubleEndedIterator<Item = &'a n::Expr>,
    members: &mut BTreeSet<DeclarationId>,
    associated: &mut BTreeSet<DeclarationId>,
    steps: &mut u64,
) -> Result<BTreeSet<DeclarationId>, nir::visit::Error> {
    let mut functions = BTreeSet::new();
    let mut found_members = BTreeSet::new();
    let mut found_associated = BTreeSet::new();
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = nir::visit::retained_expressions_checked(roots, &mut fuel, |expr, _| {
        match &expr.kind {
            K::Function(declaration)
            | K::FunctionInstance { declaration, .. }
            | K::ComptimeFunctionInstance { declaration, .. } => {
                functions.insert(*declaration);
            }
            K::TraitMethod { member, .. } => {
                found_members.insert(*member);
            }
            K::AssociatedValue { member, .. } => {
                found_associated.insert(*member);
            }
            _ => {}
        }
        Ok::<_, nir::visit::Error>(())
    });
    *steps -= (before - fuel) as u64;
    result?;
    members.extend(found_members);
    associated.extend(found_associated);
    Ok(functions)
}

pub(in crate::program) fn references(
    root: &n::Expr,
    members: &mut BTreeSet<DeclarationId>,
    associated: &mut BTreeSet<DeclarationId>,
    steps: &mut u64,
) -> Result<BTreeSet<DeclarationId>, nir::visit::Error> {
    collect(std::iter::once(root), members, associated, steps)
}

pub(in crate::program) fn function_references(
    function: &n::Function,
    members: &mut BTreeSet<DeclarationId>,
    associated: &mut BTreeSet<DeclarationId>,
    steps: &mut u64,
) -> Result<BTreeSet<DeclarationId>, nir::visit::Error> {
    // Contracts and retained defaults are dependencies too. Source defaults
    // moved into call plans are also found by the same expression traversal.
    collect(
        std::iter::once(&function.body)
            .chain(&function.preconditions)
            .chain(&function.postconditions)
            .chain(function.defaults.iter().map(|(_, value)| value)),
        members,
        associated,
        steps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 5, local }
    }
    fn expr(kind: K) -> n::Expr {
        n::Expr {
            source: n::SourceRange { start: 1, end: 2 },
            ty: p(P::Bool),
            kind,
        }
    }
    fn function() -> n::Function {
        n::Function {
            declaration: id(90),
            name: "dependencies".into(),
            phase: n::FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(p(P::Bool)),
            },
            parameters: vec![],
            type_parameters: vec![],
            body: expr(K::LetPattern {
                pattern: n::Pattern::Tuple(vec![
                    n::Pattern::CodeEvidence { outputs: vec![] },
                    n::Pattern::Guard {
                        pattern: Box::new(n::Pattern::Wild),
                        condition: Box::new(expr(K::Function(id(1)))),
                    },
                ]),
                value: Box::new(expr(K::Closure {
                    parameters: vec![],
                    captures: vec![],
                    body: std::sync::Arc::new(expr(K::FunctionInstance {
                        declaration: id(2),
                        arguments: vec![],
                    })),
                })),
                guard: None,
                otherwise: None,
            }),
            defaults: vec![(0, expr(K::Function(id(3))))],
            preconditions: vec![expr(K::TraitMethod {
                method_arguments: vec![],
                interface_arguments: vec![],
                interface: id(20),
                member: id(4),
                subject: p(P::Bool),
            })],
            postconditions: vec![expr(K::AssociatedValue {
                interface: n::InterfaceInstance {
                    declaration: id(20),
                    arguments: vec![],
                },
                member: id(5),
                subject: p(P::Bool),
            })],
            result_binding: None,
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
        }
    }

    #[test]
    fn retained_dependencies_include_patterns_closures_contracts_and_defaults() {
        let function = function();
        let mut members = BTreeSet::new();
        let mut associated = BTreeSet::new();
        let found =
            function_references(&function, &mut members, &mut associated, &mut 1000).unwrap();
        assert_eq!(found, BTreeSet::from([id(1), id(2), id(3)]));
        assert_eq!(members, BTreeSet::from([id(4)]));
        assert_eq!(associated, BTreeSet::from([id(5)]));
        // Discovery of retained code must not certify unresolved evidence as
        // executable or weaken any ordinary validation traversal.
        assert!(
            nir::visit::expression_checked::<nir::visit::Error>(
                &function.body,
                &mut 1000,
                |_, _| Ok(())
            )
            .is_err()
        );
    }

    #[test]
    fn exhausted_dependency_walk_publishes_no_partial_member_facts() {
        let function = function();
        let original = BTreeSet::from([id(99)]);
        let mut failed = false;
        let mut completed = false;
        for limit in 0..100 {
            let mut members = original.clone();
            let mut associated = original.clone();
            let mut steps = limit;
            match function_references(&function, &mut members, &mut associated, &mut steps) {
                Ok(found) => {
                    completed = true;
                    assert_eq!(found, BTreeSet::from([id(1), id(2), id(3)]));
                    assert_eq!(members, BTreeSet::from([id(4), id(99)]));
                    assert_eq!(associated, BTreeSet::from([id(5), id(99)]));
                }
                Err(nir::visit::Error::Budget(_)) => {
                    failed = true;
                    assert_eq!(members, original);
                    assert_eq!(associated, original);
                }
                Err(failure) => panic!("{failure}"),
            }
            assert!(steps < limit || limit == 0);
        }
        assert!(failed && completed);
    }

    #[test]
    fn contextual_placeholders_consume_dependency_budget_without_invocation() {
        let source = n::SourceRange { start: 1, end: 2 };
        let value = expr(K::NormalFormType {
            head: id(80),
            plan: None,
            arguments: vec![n::NormalFormArgument::List {
                source,
                items: (0..100)
                    .map(|_| n::NormalFormArgument::Null { source })
                    .collect(),
            }],
        });
        let mut members = BTreeSet::new();
        let mut associated = BTreeSet::new();
        assert!(matches!(
            references(&value, &mut members, &mut associated, &mut 20),
            Err(nir::visit::Error::Budget(_))
        ));
        assert!(
            references(&value, &mut members, &mut associated, &mut 1000)
                .unwrap()
                .is_empty()
        );
        // Opaque heads are not executable function dependencies.
        assert!(members.is_empty() && associated.is_empty());
    }
}
