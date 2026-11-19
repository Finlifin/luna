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
    MethodContract(DeclarationId),
    MethodBody {
        implementation: DeclarationId,
        member: DeclarationId,
    },
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

/// A declaration scan records member identities. A required world also knows
/// the implementation index and can restrict code retention to compatible heads.
#[derive(Clone, Copy)]
pub(in crate::program) enum Discovery<'w> {
    Declarations,
    Required(&'w crate::implementation::Index),
}

impl Discovery<'_> {
    fn method(
        self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        member: DeclarationId,
        found: &mut BTreeSet<Dependency>,
        fuel: &mut usize,
    ) -> Result<(), nir::visit::Error> {
        let Self::Required(index) = self else {
            found.insert(Dependency::Method(member));
            return Ok(());
        };
        let mut budget = crate::ptc::Budget {
            steps: *fuel,
            ..Default::default()
        };
        let candidates = index.candidates_metered(Some(interface), arguments, subject, &mut budget);
        *fuel = budget.steps;
        let candidates = candidates.map_err(|_| {
            nir::visit::Error::Budget("required method dependency search exhausted")
        })?;
        found.insert(Dependency::MethodContract(member));
        found.extend(candidates.declarations().iter().map(|&implementation| {
            Dependency::MethodBody {
                implementation,
                member,
            }
        }));
        Ok(())
    }

    fn collect<'a>(
        self,
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
                    self.collect_values(arguments, &mut found, fuel)?;
                }
                K::TraitMethod {
                    member,
                    interface_arguments,
                    method_arguments,
                    interface,
                    subject,
                } => {
                    self.method(
                        *interface,
                        interface_arguments,
                        subject,
                        *member,
                        &mut found,
                        fuel,
                    )?;
                    self.collect_values(
                        interface_arguments.iter().chain(method_arguments),
                        &mut found,
                        fuel,
                    )?;
                }
                K::AssociatedValue {
                    member, interface, ..
                } => {
                    found.insert(Dependency::Associated(*member));
                    self.collect_values(&interface.arguments, &mut found, fuel)?;
                }
                K::ComptimeValue(value) | K::StagedValue(value) => {
                    self.collect_values([value], &mut found, fuel)?;
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
        self,
        root: &n::Expr,
        steps: &mut u64,
    ) -> Result<BTreeSet<Dependency>, nir::visit::Error> {
        self.collect(std::iter::once(root), steps)
    }

    pub(in crate::program) fn function_references(
        self,
        function: &n::Function,
        steps: &mut u64,
    ) -> Result<BTreeSet<Dependency>, nir::visit::Error> {
        self.collect(
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
        self,
        values: impl IntoIterator<Item = &'a Term>,
        steps: &mut u64,
        span: Span,
    ) -> Result<BTreeSet<Dependency>, Error> {
        let mut found = BTreeSet::new();
        self.collect_frozen(values, &mut found, steps)
            .map_err(|failure| error(span, failure.to_string()))?;
        Ok(found)
    }

    fn collect_frozen<'a>(
        self,
        values: impl IntoIterator<Item = &'a Term>,
        found: &mut BTreeSet<Dependency>,
        steps: &mut u64,
    ) -> Result<(), nir::visit::Error> {
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = self.collect_values(values, found, &mut fuel);
        *steps -= (before - fuel) as u64;
        result
    }

    // Walk data operands only. A Type identity, including an opaque application,
    // has no executable edge merely because it describes the enclosing value.
    fn collect_values<'a>(
        self,
        values: impl IntoIterator<Item = &'a Term>,
        found: &mut BTreeSet<Dependency>,
        fuel: &mut usize,
    ) -> Result<(), nir::visit::Error> {
        let mut pending = Vec::new();
        let enqueue = |pending: &mut Vec<&'a Term>, term: &'a Term, fuel: &mut usize| {
            *fuel = fuel.checked_sub(1).ok_or(nir::visit::Error::Budget(
                "required comptime value dependency budget exhausted",
            ))?;
            pending.push(term);
            Ok(())
        };
        for root in values {
            enqueue(&mut pending, root, fuel)?;
            while let Some(value) = pending.pop() {
                match value {
                    Term::MethodValue(method) => {
                        self.method(
                            method.interface.declaration,
                            &method.interface.arguments,
                            &method.subject,
                            method.member,
                            found,
                            fuel,
                        )?;
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
                value.visit_children(|child| enqueue(&mut pending, child, fuel))?;
            }
        }
        Ok(())
    }
}

/// Observations from the selected packet. A carried value binder can select
/// existing lexical authority; its ID alone never supplies a domain proof.
#[derive(Default)]
pub(super) struct ObservedInputs {
    pub(super) code: BTreeSet<Dependency>,
    pub(super) types: BTreeSet<DeclarationId>,
    pub(super) values: BTreeSet<DeclarationId>,
}

impl ObservedInputs {
    fn term(&mut self, ty: &Term, fuel: &mut u64) -> Result<(), comptime_interpretor::ErrorKind> {
        use comptime_interpretor::ErrorKind;
        use nir::copy_budget::{CopyBudgetExhausted as Limit, InputObservationError};
        let inputs = nir::copy_budget::term_inputs(ty, fuel).map_err(|failure| match failure {
            InputObservationError::Limit(Limit::Steps) => ErrorKind::StepsExhausted,
            InputObservationError::Limit(Limit::Depth) => ErrorKind::DepthExhausted,
            InputObservationError::Scope(_) => ErrorKind::Type,
        })?;
        for (id, sort) in inputs.declarations {
            match sort {
                Sort::Type => {
                    self.types.insert(id);
                }
                Sort::ComptimeValue => {
                    self.values.insert(id);
                }
                Sort::Runtime => {}
            }
        }
        Ok(())
    }

    pub(super) fn live(
        &mut self,
        discovery: Discovery<'_>,
        root: &Value,
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        root.visit_owned_graph(steps, &mut |value, fuel| {
            match value {
                Value::TypeParameter(id) => {
                    self.types.insert(*id);
                }
                Value::AbstractValue { declaration, ty } => {
                    self.values.insert(*declaration);
                    self.term(ty, fuel)?;
                }
                Value::Function(id) => {
                    self.code.insert(Dependency::Callable(*id));
                }
                Value::IirCallable(_) | Value::FunctionInstance { .. } => {
                    let (declaration, arguments) = value.declaration_instance().unwrap();
                    if let Value::IirCallable(handle) = value {
                        self.term(handle.ty(), fuel)?;
                    }
                    self.code.insert(Dependency::Callable(declaration));
                    discovery
                        .collect_frozen(arguments, &mut self.code, fuel)
                        .map_err(visit_failure)?;
                    for term in arguments {
                        self.term(term, fuel)?;
                    }
                }
                Value::RuntimeTraitMethod(method) => {
                    let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
                    let before = remaining;
                    let result = discovery.method(
                        method.interface.declaration,
                        &method.interface.arguments,
                        &method.subject,
                        method.member,
                        &mut self.code,
                        &mut remaining,
                    );
                    *fuel -= (before - remaining) as u64;
                    result.map_err(visit_failure)?;
                    discovery
                        .collect_frozen(method.terms(), &mut self.code, fuel)
                        .map_err(visit_failure)?;
                    for term in method.terms() {
                        self.term(term, fuel)?;
                    }
                }
                Value::Closure(closure) => {
                    self.term(closure.ty(), fuel)?;
                    self.code.extend(
                        discovery
                            .references(closure.body(), fuel)
                            .map_err(visit_failure)?,
                    );
                }
                Value::Record { arguments, .. } | Value::Variant { arguments, .. } => {
                    for term in arguments {
                        self.term(term, fuel)?;
                    }
                }
                Value::Object(object) => {
                    for entry in object.entries() {
                        self.term(&entry.value().ty, fuel)?;
                    }
                }
                Value::Type(ty)
                | Value::List { ty, .. }
                | Value::Newtype { ty, .. }
                | Value::Optional { ty, .. }
                | Value::ErrorResult { ty, .. } => self.term(ty, fuel)?,
                _ => {}
            }
            Ok(())
        })
        .map_err(|failure| {
            error(
                span,
                format!("required comptime graph inspection failed: {failure:?}"),
            )
        })
    }
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
                source: nir::program::SourceRange { start: 0, end: 0 },
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
        let found = Discovery::Declarations
            .function_references(&function, &mut 1000)
            .unwrap();
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
        assert_eq!(
            Discovery::Declarations.references(&root, &mut 100).unwrap(),
            expected
        );
        assert_eq!(
            Discovery::Declarations
                .frozen_references([&frozen], &mut 100, Span::default())
                .unwrap(),
            expected
        );
        let mut observation = ObservedInputs::default();
        observation
            .live(Discovery::Declarations, &live, &mut 100, Span::default())
            .unwrap();
        assert_eq!(observation.code, expected);
    }

    #[test]
    fn exhausted_dependency_walk_publishes_no_partial_member_facts() {
        let function = function();
        let expected = Discovery::Declarations
            .function_references(&function, &mut 1000)
            .unwrap();
        let mut failed = false;
        let mut completed = false;
        for limit in 0..100 {
            let mut steps = limit;
            match Discovery::Declarations.function_references(&function, &mut steps) {
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
            Discovery::Declarations.references(&value, &mut 20),
            Err(nir::visit::Error::Budget(_))
        ));
        assert!(
            Discovery::Declarations
                .references(&value, &mut 1000)
                .unwrap()
                .is_empty()
        );
    }
}
