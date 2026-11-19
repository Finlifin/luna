//! Free rigid inputs of the callable declarations retained for source execution.
use super::{CopyBudgetExhausted, Node, charge_node_observed};
use crate::{DeclarationId, Term, program as n};
use std::collections::{BTreeMap, BTreeSet};

/// Observe code, formation contracts and clause domains. A rigid owned by a
/// callable is supplied on entry to that declaration. Remove its telescope
/// locally so the same identity can still be a free input in another carrier.
pub fn retained_program_inputs(
    package: &n::Package,
    programs: &crate::associated_initializers::MemberPrograms,
    domains: &BTreeMap<DeclarationId, Term>,
    steps: &mut u64,
) -> Result<BTreeSet<DeclarationId>, CopyBudgetExhausted> {
    let mut free = BTreeSet::new();
    // A member template may use an outer declaration input even when its
    // helpers do not. Only its own implementation telescope is supplied locally.
    *steps = steps
        .checked_sub(package.implementations.len() as u64)
        .ok_or(CopyBudgetExhausted::Steps)?;
    let initializers = package.implementations.iter().flat_map(|row| {
        crate::associated_initializers::programs_for(row, Some(programs)).map(|(_, template)| {
            (
                Node::Expr(template.expression()),
                row.type_parameters.as_slice(),
            )
        })
    });
    for (node, binders) in package
        .functions
        .iter()
        .map(|f| (Node::Function(f), f.type_parameters.as_slice()))
        .chain(
            package
                .runtime_function_identities
                .iter()
                .map(|f| (Node::Formation(f.into()), f.type_parameters.as_slice())),
        )
        .chain(initializers)
    {
        *steps = steps
            .checked_sub(binders.len() as u64)
            .ok_or(CopyBudgetExhausted::Steps)?;
        // Inherited binders and direct clause inputs use the same admitted
        // domain directory. An omitted all-Type domain has no free inputs.
        let nodes = std::iter::once(node).chain(
            binders
                .iter()
                .filter_map(|binder| domains.get(binder))
                .map(Node::Type),
        );
        collect(nodes, binders, steps, &mut free)?;
    }
    Ok(free)
}

fn collect<'a>(
    nodes: impl Iterator<Item = Node<'a>>,
    binders: &[DeclarationId],
    steps: &mut u64,
    free: &mut BTreeSet<DeclarationId>,
) -> Result<(), CopyBudgetExhausted> {
    let mut used = BTreeSet::new();
    for node in nodes {
        charge_node_observed(node, steps, true, None, &mut |term, _| {
            if let Term::Rigid { declaration, .. } = term {
                used.insert(*declaration);
            }
        })?;
    }
    *steps = steps
        .checked_sub(binders.len() as u64)
        .ok_or(CopyBudgetExhausted::Steps)?;
    for binder in binders {
        used.remove(binder);
    }
    free.extend(used);
    Ok(())
}
