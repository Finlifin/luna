//! Bounded dependency discovery for checked and retained required code. Facts
//! publish only after a complete walk; discovery does not execute producers or
//! authorize opaque pattern evidence for invocation.
use super::*;

/// Dependencies identify which part of a declaration the source uses.
/// A method can also be the declaration of its inherited default body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::program) enum Dependency {
    Associated(DeclarationId),
    Method(DeclarationId),
    Callable(DeclarationId),
    Definition(DeclarationId),
}

impl Dependency {
    pub(in crate::program) fn callable(self) -> Option<DeclarationId> {
        match self {
            Self::Callable(id) => Some(id),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct Worklist {
    discovered: BTreeSet<Dependency>,
    pending: VecDeque<Dependency>,
}

impl Worklist {
    pub(super) fn extend(
        &mut self,
        dependencies: impl IntoIterator<Item = Dependency>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        for need in dependencies {
            *steps = steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "required comptime dependency budget exhausted"))?;
            if self.discovered.insert(need) {
                self.pending.push_back(need);
            }
        }
        Ok(())
    }

    pub(super) fn next(&mut self) -> Option<Dependency> {
        self.pending.pop_front()
    }

    pub(super) fn into_discovered(self) -> BTreeSet<Dependency> {
        self.discovered
    }
}

fn collect<'a>(
    roots: impl DoubleEndedIterator<Item = &'a n::Expr>,
    steps: &mut u64,
) -> Result<BTreeSet<Dependency>, nir::visit::Error> {
    let mut found = BTreeSet::new();
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = nir::visit::retained_expressions_checked(roots, &mut fuel, |expr, fuel| {
        match &expr.kind {
            K::Function(id)
            | K::ComptimeFunctionInstance {
                declaration: id, ..
            } => {
                found.insert(Dependency::Callable(*id));
            }
            K::FunctionInstance {
                declaration,
                arguments,
            } => {
                found.insert(Dependency::Callable(*declaration));
                collect_values(arguments, &mut found, fuel)?;
            }
            K::TraitMethod {
                member,
                interface_arguments,
                method_arguments,
                ..
            } => {
                found.insert(Dependency::Method(*member));
                collect_values(
                    interface_arguments.iter().chain(method_arguments),
                    &mut found,
                    fuel,
                )?;
            }
            K::AssociatedValue {
                member, interface, ..
            } => {
                found.insert(Dependency::Associated(*member));
                collect_values(&interface.arguments, &mut found, fuel)?;
            }
            K::ComptimeValue(value) | K::StagedValue(value) => {
                collect_values([value], &mut found, fuel)?;
            }
            _ => {}
        }
        Ok::<_, nir::visit::Error>(())
    });
    *steps -= (before - fuel) as u64;
    result?;
    Ok(found)
}

pub(in crate::program) fn references(
    root: &n::Expr,
    steps: &mut u64,
) -> Result<BTreeSet<Dependency>, nir::visit::Error> {
    collect(std::iter::once(root), steps)
}

pub(in crate::program) fn function_references(
    function: &n::Function,
    steps: &mut u64,
) -> Result<BTreeSet<Dependency>, nir::visit::Error> {
    collect(
        std::iter::once(&function.body)
            .chain(&function.preconditions)
            .chain(&function.postconditions)
            .chain(function.defaults.iter().map(|(_, value)| value)),
        steps,
    )
}

// Callable identities are execution dependencies wherever they occur in a
// completed aggregate. Data types are not unfolded and no factory is invoked.
pub(in crate::program) fn frozen_references<'a>(
    values: impl IntoIterator<Item = &'a Term>,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<Dependency>, Error> {
    let mut found = BTreeSet::new();
    collect_frozen(values, &mut found, steps)
        .map_err(|failure| error(span, failure.to_string()))?;
    Ok(found)
}

fn collect_frozen<'a>(
    values: impl IntoIterator<Item = &'a Term>,
    found: &mut BTreeSet<Dependency>,
    steps: &mut u64,
) -> Result<(), nir::visit::Error> {
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = collect_values(values, found, &mut fuel);
    *steps -= (before - fuel) as u64;
    result
}

// Walk data operands only. A Type identity, including an opaque application,
// has no executable edge merely because it describes the enclosing value.
fn collect_values<'a>(
    values: impl IntoIterator<Item = &'a Term>,
    found: &mut BTreeSet<Dependency>,
    fuel: &mut usize,
) -> Result<(), nir::visit::Error> {
    let mut pending = Vec::new();
    let mut enqueue = |pending: &mut Vec<&'a Term>, term: &'a Term| {
        *fuel = fuel.checked_sub(1).ok_or(nir::visit::Error::Budget(
            "required comptime value dependency budget exhausted",
        ))?;
        pending.push(term);
        Ok(())
    };
    for root in values {
        enqueue(&mut pending, root)?;
        while let Some(value) = pending.pop() {
            match value {
                Term::MethodValue(method) => {
                    found.insert(Dependency::Method(method.member));
                }
                Term::FunctionValue { declaration, .. } => {
                    found.insert(Dependency::Callable(*declaration));
                }
                Term::ClosureSnapshot { factory, .. } => {
                    found.insert(Dependency::Callable(*factory));
                }
                Term::TupleValue(_)
                | Term::RecordValue(_)
                | Term::OptionalValue { .. }
                | Term::ErrorValue { .. }
                | Term::NominalValue { .. }
                | Term::ListConcat { .. } => {}
                _ => continue,
            }
            value.visit_children(|child| enqueue(&mut pending, child))?;
        }
    }
    Ok(())
}

// The traversal includes guards and pattern computations; a function hidden in
// one of those positions must not escape phase/dependency checks.
pub(super) fn live_references(
    value: &Value,
    steps: &mut u64,
    span: Span,
) -> Result<BTreeSet<Dependency>, Error> {
    let mut found = BTreeSet::new();
    value
        .visit_owned_graph(steps, &mut |value, fuel| {
            match value {
                Value::Function(id) => {
                    found.insert(Dependency::Callable(*id));
                }
                Value::FunctionInstance {
                    declaration,
                    arguments,
                } => {
                    found.insert(Dependency::Callable(*declaration));
                    collect_frozen(arguments, &mut found, fuel).map_err(visit_failure)?;
                }
                Value::RuntimeTraitMethod(method) => {
                    found.insert(Dependency::Method(method.member));
                    collect_frozen(method.terms(), &mut found, fuel).map_err(visit_failure)?;
                }
                Value::Closure(closure) => {
                    if let Some((declaration, arguments)) = closure.declaration_instance() {
                        found.insert(Dependency::Callable(declaration));
                        collect_frozen(arguments, &mut found, fuel).map_err(visit_failure)?;
                    }
                    found.extend(references(closure.body(), fuel).map_err(visit_failure)?);
                }
                _ => {}
            }
            Ok(())
        })
        .map_err(|failure| {
            error(
                span,
                format!("required comptime graph dependency inspection failed: {failure:?}"),
            )
        })?;
    Ok(found)
}

fn visit_failure(error: nir::visit::Error) -> comptime_interpretor::ErrorKind {
    match error {
        nir::visit::Error::Budget(_) => comptime_interpretor::ErrorKind::StepsExhausted,
        nir::visit::Error::Invalid(_) => comptime_interpretor::ErrorKind::Type,
    }
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
            interface: nir::program::FunctionDeclaration {
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
            },
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
        }
    }

    #[test]
    fn retained_dependencies_include_patterns_closures_contracts_and_defaults() {
        let function = function();
        let found = function_references(&function, &mut 1000).unwrap();
        assert_eq!(
            found,
            BTreeSet::from([
                Dependency::Callable(id(1)),
                Dependency::Callable(id(2)),
                Dependency::Callable(id(3)),
                Dependency::Method(id(4)),
                Dependency::Associated(id(5)),
            ])
        );
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
    fn instance_inputs_keep_the_same_callable_edges_across_value_forms() {
        let callable = |declaration| Term::FunctionValue {
            declaration,
            arguments: vec![],
        };
        let arguments = vec![Term::TupleValue(vec![
            callable(id(2)),
            Term::NormalForm {
                head: id(3),
                sort: Sort::Type,
                arguments: vec![callable(id(4))],
            },
        ])];
        let root = expr(K::FunctionInstance {
            declaration: id(1),
            arguments: arguments.clone(),
        });
        let frozen = Term::FunctionValue {
            declaration: id(1),
            arguments: arguments.clone(),
        };
        let live = Value::FunctionInstance {
            declaration: id(1),
            arguments,
        };
        let expected = BTreeSet::from([Dependency::Callable(id(1)), Dependency::Callable(id(2))]);
        assert_eq!(references(&root, &mut 100).unwrap(), expected);
        assert_eq!(
            frozen_references([&frozen], &mut 100, Span::default()).unwrap(),
            expected
        );
        assert_eq!(
            live_references(&live, &mut 100, Span::default()).unwrap(),
            expected
        );
    }

    #[test]
    fn exhausted_dependency_walk_publishes_no_partial_member_facts() {
        let function = function();
        let expected = function_references(&function, &mut 1000).unwrap();
        let mut failed = false;
        let mut completed = false;
        for limit in 0..100 {
            let mut steps = limit;
            match function_references(&function, &mut steps) {
                Ok(found) => {
                    completed = true;
                    assert_eq!(found, expected);
                }
                Err(nir::visit::Error::Budget(_)) => failed = true,
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
        assert!(matches!(
            references(&value, &mut 20),
            Err(nir::visit::Error::Budget(_))
        ));
        assert!(references(&value, &mut 1000).unwrap().is_empty());
    }
}
