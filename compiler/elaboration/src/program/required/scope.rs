//! Select existing lexical authority from the actual invocation carriers.
//! Dependency discovery cannot authorize a previously unknown value binder.
use super::*;

pub(super) fn rebind_input(
    ty: &Term,
    mapping: &[Term],
    steps: &mut u64,
    span: Span,
) -> Result<Term, Error> {
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let checked = nir::type_computations::charge_term(ty, &mut fuel);
    *steps -= (before - fuel) as u64;
    checked.map_err(|reason| error(span, reason.to_string()))?;
    ty.rebind_scope(mapping)
        .map_err(|reason| error(span, reason.to_string()))
}

/// Select the transitive lexical prefix of the code and transported values.
/// Emit a value only after the binders used by its checked domain.
pub(super) fn select_abstract_inputs(
    declarations: &[(DeclarationId, Term)],
    callable: &BTreeMap<n::LocalId, Option<function_types::StagedParameter>>,
    mapping: Option<&[Term]>,
    world: &prepared::RequiredWorld,
    live: &dependencies::ObservedInputs,
    steps: &mut u64,
    span: Span,
) -> Result<(Vec<DeclarationId>, Vec<(DeclarationId, Term)>), Error> {
    if declarations.is_empty() && callable.is_empty() {
        return Ok(Default::default());
    }
    let charge = |steps: &mut u64, count: usize| {
        *steps = steps
            .checked_sub(count as u64)
            .ok_or_else(|| error(span, "required value scope selection budget exhausted"))?;
        Ok::<_, Error>(())
    };
    charge(steps, declarations.len() + callable.len())?;
    let available: BTreeMap<_, _> = declarations
        .iter()
        .map(|(id, domain)| (*id, domain))
        .chain(
            callable
                .values()
                .flatten()
                .map(|slot| (slot.declaration, &slot.domain)),
        )
        .collect();
    let source = world.free_inputs(steps, span)?;
    charge(steps, source.len() + live.values.len() + live.types.len())?;
    let needed: BTreeSet<_> = source
        .iter()
        .chain(&live.values)
        .chain(&live.types)
        .copied()
        .collect();
    // Pending terms are already in the VM frame. Keep the transformed
    // domain until its dependencies have been selected.
    let mut pending: Vec<_> = needed.into_iter().rev().map(|id| (id, None)).collect();
    let mut active = BTreeSet::new();
    let mut selected = BTreeSet::new();
    let mut types = Vec::new();
    let mut values = Vec::new();
    while let Some((id, prepared_domain)) = pending.pop() {
        charge(steps, 1)?;
        let Some(&domain) = available.get(&id) else {
            continue;
        };
        if selected.contains(&id) {
            continue;
        }
        if let Some(domain) = prepared_domain {
            active.remove(&id);
            selected.insert(id);
            if domain == p(P::Type) {
                types.push(id);
            } else {
                values.push((id, domain));
            }
        } else {
            if !active.insert(id) {
                return Err(error(span, "cyclic abstract input domain"));
            }
            let domain = match mapping {
                Some(slots) => rebind_input(domain, slots, steps, span)?,
                None => inputs::copy(domain, steps, span)?,
            };
            let dependencies = input_observation::term(&domain, steps, span)?;
            pending.push((id, Some(domain)));
            pending.extend(dependencies.into_iter().rev().map(|id| (id, None)));
        }
    }
    Ok((types, values))
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
                source: nir::program::SourceRange { start: 0, end: 0 },
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

    fn world(entry: n::Function) -> std::sync::Arc<prepared::RequiredWorld> {
        prepared::RequiredWorld::new(n::Package {
            functions: vec![entry],
            ..Default::default()
        })
    }

    #[test]
    fn residual_and_transported_uses_select_only_existing_authority() {
        let entry = entry();
        let available = vec![
            (id(2), p(P::Bool)),
            (id(3), p(P::Bool)),
            (id(4), p(P::Bool)),
        ];
        let transported = dependencies::ObservedInputs {
            values: [id(3), id(99)].into_iter().collect(),
            ..Default::default()
        };
        let (_, authority) = select_abstract_inputs(
            &available,
            &BTreeMap::new(),
            None,
            &world(entry.clone()),
            &transported,
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
    fn nested_source_can_use_a_lexical_value_without_an_entry_capture() {
        let mut root = entry();
        let mut helper = entry();
        helper.interface.declaration = id(8);
        root.body.kind = K::Call {
            callee: Box::new(n::Expr {
                source: root.body.source,
                ty: helper.signature.clone().with_comptime_phase(true),
                kind: K::Function(id(8)),
            }),
            arguments: vec![],
            parameter_bindings: vec![],
        };
        let package = n::Package {
            functions: vec![root, helper],
            ..Default::default()
        };
        let world = prepared::RequiredWorld::new(package.clone());
        let available = vec![(id(2), p(P::Bool)), (id(7), p(P::Bool))];
        let (_, authority) = select_abstract_inputs(
            &available,
            &BTreeMap::new(),
            None,
            &world,
            &Default::default(),
            &mut 1000,
            Span::default(),
        )
        .unwrap();
        assert_eq!(authority, [(id(2), p(P::Bool))]);
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
    fn a_captured_binder_selects_the_predecessors_in_its_domain() {
        let ty = Term::NormalForm {
            head: id(50),
            sort: Sort::Type,
            arguments: vec![value(8)],
        };
        let available = [(id(2), ty.clone()), (id(8), p(P::Bool)), (id(9), p(P::U8))];
        let (_, selected) = select_abstract_inputs(
            &available,
            &BTreeMap::new(),
            None,
            &world(entry()),
            &Default::default(),
            &mut 1000,
            Span::default(),
        )
        .unwrap();
        assert_eq!(selected, [(id(8), p(P::Bool)), (id(2), ty)]);
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

        let available = vec![(id(3), p(P::Bool)), (id(4), p(P::Bool))];
        let (_, authority) = select_abstract_inputs(
            &available,
            &BTreeMap::new(),
            None,
            &world(entry()),
            &observed,
            &mut 1000,
            Span::default(),
        )
        .unwrap();
        assert_eq!(authority, [(id(4), p(P::Bool))]);
    }

    #[test]
    fn failed_scope_selection_keeps_authority_and_can_retry() {
        let entry = entry();
        let wide = Term::Tuple(vec![p(P::Bool); 4096]);
        let available = vec![(id(2), wide.clone()), (id(4), p(P::Bool))];
        let mut steps = 1000;
        assert!(
            select_abstract_inputs(
                &available,
                &BTreeMap::new(),
                None,
                &world(entry.clone()),
                &Default::default(),
                &mut steps,
                Span::default()
            )
            .is_err()
        );
        assert!(steps < 1000);
        let (_, authority) = select_abstract_inputs(
            &available,
            &BTreeMap::new(),
            None,
            &world(entry.clone()),
            &Default::default(),
            &mut 100_000,
            Span::default(),
        )
        .unwrap();
        assert_eq!(authority, vec![(id(2), wide)]);
    }
}
