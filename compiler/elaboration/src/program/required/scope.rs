//! Select existing lexical authority from the actual invocation carriers.
//! Dependency discovery cannot authorize a previously unknown value binder.
use super::*;

pub(super) fn retain_abstract_values(
    authority: &mut Vec<(DeclarationId, Term)>,
    entry: &n::Function,
    inputs: &[(n::LocalId, Term, Option<Term>)],
    live: &BTreeSet<DeclarationId>,
    steps: &mut u64,
    span: Span,
) -> Result<(), Error> {
    if authority.is_empty() {
        return Ok(());
    }
    // Sparse local capture does not collect constants embedded in the body.
    // Include those terms, the entry contract and its captured parameter types.
    let mut used = input_observation::function(entry, steps, span)?;
    for (_, _, value) in inputs {
        *steps = steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "required value scope selection budget exhausted"))?;
        if let Some(value) = value {
            used.extend(input_observation::term(value, steps, span)?);
        }
    }
    *steps = steps
        .checked_sub(authority.len() as u64)
        .ok_or_else(|| error(span, "required value scope selection budget exhausted"))?;
    // Commit selection only after every carrier has been observed successfully.
    authority.retain(|(id, _)| used.contains(id) || live.contains(id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }

    fn value(local: u32) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort: Sort::ComptimeValue,
        }
    }

    fn entry() -> n::Function {
        n::Function {
            interface: nir::program::FunctionDeclaration {
                declaration: id(1),
                name: "scope".into(),
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
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
            body: n::Expr {
                source: source(Span::default()),
                ty: p(P::Bool),
                kind: K::ComptimeValue(value(2)),
            },
        }
    }

    #[test]
    fn residual_and_transported_uses_select_only_existing_authority() {
        let entry = entry();
        let mut authority = vec![
            (id(2), p(P::Bool)),
            (id(3), p(P::Bool)),
            (id(4), p(P::Bool)),
        ];
        let inputs = vec![(
            n::LocalId { owner: 1, index: 0 },
            Term::Tuple(vec![p(P::Bool), p(P::Bool)]),
            Some(Term::TupleValue(vec![value(3), value(99)])),
        )];
        retain_abstract_values(
            &mut authority,
            &entry,
            &inputs,
            &BTreeSet::new(),
            &mut 1000,
            Span::default(),
        )
        .unwrap();
        assert_eq!(authority, vec![(id(2), p(P::Bool)), (id(3), p(P::Bool))]);
        // The residual constant itself has no local capture, yet still needs
        // its known lexical binder to cross the VM admission boundary.
        let package = n::Package {
            functions: vec![entry],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_abstract_value_parameters(&authority)
            .unwrap();
        assert_eq!(
            vm.invoke(id(1), vec![]).unwrap(),
            Value::AbstractValue {
                declaration: id(2),
                ty: p(P::Bool),
            }
        );
    }

    #[test]
    fn live_carriers_select_existing_value_binders_without_granting_new_ones() {
        let rigid = |n| Term::Rigid {
            declaration: id(n),
            sort: Sort::Type,
        };
        let packet = Value::Record {
            declaration: id(50),
            arguments: vec![rigid(51)],
            fields: vec![
                Value::FunctionInstance {
                    declaration: id(52),
                    arguments: vec![rigid(53), value(4)],
                },
                Value::AbstractValue {
                    declaration: id(99),
                    ty: p(P::Bool),
                },
            ],
        };
        let mut observed = dependencies::ObservedInputs::default();
        observed.live(&packet, &mut 1000, Span::default()).unwrap();
        assert_eq!(observed.types, BTreeSet::from([id(51), id(53)]));
        assert_eq!(observed.values, BTreeSet::from([id(4), id(99)]));
        assert_eq!(
            observed.code,
            BTreeSet::from([Dependency::Callable(id(52))])
        );

        let mut authority = vec![(id(3), p(P::Bool)), (id(4), p(P::Bool))];
        retain_abstract_values(
            &mut authority,
            &entry(),
            &[],
            &observed.values,
            &mut 1000,
            Span::default(),
        )
        .unwrap();
        assert_eq!(authority, [(id(4), p(P::Bool))]);
    }

    #[test]
    fn failed_scope_selection_keeps_authority_and_can_retry() {
        let entry = entry();
        let original = vec![(id(2), p(P::Bool)), (id(4), p(P::Bool))];
        let mut authority = original.clone();
        let inputs = vec![(
            n::LocalId { owner: 1, index: 0 },
            p(P::Symbol),
            Some(Term::Symbol("wide".repeat(4096).into())),
        )];
        let mut steps = 1000;
        assert!(
            retain_abstract_values(
                &mut authority,
                &entry,
                &inputs,
                &BTreeSet::new(),
                &mut steps,
                Span::default()
            )
            .is_err()
        );
        assert!(steps < 1000);
        assert_eq!(authority, original);
        retain_abstract_values(
            &mut authority,
            &entry,
            &inputs,
            &BTreeSet::new(),
            &mut 100_000,
            Span::default(),
        )
        .unwrap();
        assert_eq!(authority, vec![(id(2), p(P::Bool))]);
    }
}
