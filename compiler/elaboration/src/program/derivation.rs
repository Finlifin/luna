//! Instantiate explicit, declaration-only strategies before ordinary impl checking.
use super::{Error, error, function_templates};
use hir::common::DefId;
use hir::definitions::DefinitionBatch;
use hir::{ExprKind as E, HirArena, ItemKind, OwnerId, OwnerInfo, OwnerNode, Package};
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn unsupported(span: rustc_span::Span, detail: &str) -> Error {
    error(
        span,
        format!("derivation expansion requires {detail}, which is not implemented"),
    )
}
fn charge(fuel: &mut usize, span: rustc_span::Span) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "derivation expansion budget exhausted"))?;
    Ok(())
}
fn members(
    source: &Package<'_>,
    strategy: OwnerId,
    fuel: &mut usize,
) -> Result<Vec<OwnerId>, Error> {
    let item = source.item(strategy).unwrap();
    let ItemKind::Deriver(def) = &item.kind else {
        return Err(error(item.span, "derive selection must name a strategy"));
    };
    if !def.parameters.is_empty()
        || !def.clause_params.is_empty()
        || !def.clause_constraints.is_empty()
    {
        return Err(unsupported(
            item.span,
            "strategy input and contract instantiation",
        ));
    }
    let body = source
        .body(def.body)
        .ok_or_else(|| error(item.span, "derivation strategy body missing"))?;
    let E::Block(block) = body.value.kind else {
        return Err(unsupported(item.span, "strategy body evaluation"));
    };
    let mut result = Vec::new();
    for statement in block.stmts.iter().chain(block.expr.into_iter()) {
        charge(fuel, statement.span)?;
        match statement.kind {
            E::Item(owner)
                if matches!(
                    source.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(_))
                ) =>
            {
                result.push(owner)
            }
            E::Unit => {}
            _ => {
                return Err(unsupported(
                    statement.span,
                    "strategy statements and associated member construction",
                ));
            }
        }
    }
    Ok(result)
}

pub(super) fn expand<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    fuel: &mut usize,
) -> Result<Package<'h>, Error> {
    let mut requests = Vec::new();
    let mut selected = BTreeSet::new();
    let mut strategies = BTreeMap::new();
    let mut omitted = BTreeSet::new();
    for (owner, info) in source.owners() {
        let item = info.node.expect_item();
        charge(fuel, item.span)?;
        if let ItemKind::Derive(request) = &item.kind {
            if source.lexical_parents.get(&owner).is_some_and(|parent| {
                !matches!(
                    source.item(*parent).map(|item| &item.kind),
                    Some(ItemKind::Mod(_))
                )
            }) {
                return Err(unsupported(item.span, "block-local derive requests"));
            }
            if !request.clause_params.is_empty() || !request.clause_constraints.is_empty() {
                return Err(unsupported(item.span, "generic derive requests"));
            }
            let selection = request
                .strategy
                .ok_or_else(|| unsupported(item.span, "default strategy selection"))?;
            let E::ResolvedDef { def_id, .. } = selection.kind else {
                return Err(unsupported(selection.span, "computed strategy selection"));
            };
            let strategy = source
                .definition_owner(def_id)
                .ok_or_else(|| error(selection.span, "missing derivation strategy"))?;
            if !strategies.contains_key(&strategy) {
                strategies.insert(strategy, members(source, strategy, fuel)?);
            }
            selected.insert(strategy);
            omitted.insert(owner);
            requests.push((owner, request, strategy));
        }
    }
    for (owner, info) in source.owners() {
        charge(fuel, info.node.expect_item().span)?;
        if matches!(info.node.expect_item().kind, ItemKind::Deriver(_)) {
            if !selected.contains(&owner) {
                return Err(unsupported(
                    info.node.expect_item().span,
                    "independent checking of unused strategies",
                ));
            }
            omitted.insert(owner);
            omitted.extend(strategies[&owner].iter().copied());
        }
    }
    // All nested source definitions must be accounted for before pruning.
    for (owner, parent) in &source.lexical_parents {
        charge(fuel, rustc_span::DUMMY_SP)?;
        if omitted.contains(parent) && !omitted.contains(owner) {
            return Err(unsupported(
                source.item(*owner).unwrap().span,
                "nested generation definitions",
            ));
        }
    }
    for attribute in &source.definition_attributes {
        charge(fuel, attribute.span)?;
        let target = match attribute.target {
            hir::AttributeTarget::Definition(owner) => owner,
            hir::AttributeTarget::Member(id) => id.owner,
        };
        if matches!(
            source.item(target).map(|item| &item.kind),
            Some(ItemKind::Derive(_) | ItemKind::Deriver(_))
        ) {
            return Err(unsupported(
                attribute.span,
                "strategy and derive-request attributes",
            ));
        }
    }
    let mut candidate = hir::definitions::omit_generation_sources(source, arena, &omitted, fuel)
        .map_err(|e| error(rustc_span::DUMMY_SP, e.to_string()))?;
    let mut batch = DefinitionBatch::new(&mut candidate);
    let mut replacements = BTreeMap::new();
    let mut resolution = Vec::new();
    let mut attributes = Vec::new();
    let mut staged = Vec::new();
    for (request_owner, request, strategy) in requests {
        let ItemKind::Deriver(generator) = &source.item(strategy).unwrap().kind else {
            unreachable!()
        };
        let span = source.item(request_owner).unwrap().span;
        let mut reserve_owner = || batch.reserve().map_err(|e| error(span, e.to_string()));
        let impl_owner = reserve_owner()?;
        replacements.insert(request_owner, impl_owner);
        let pkg = source
            .owner_definition(request_owner)
            .ok_or_else(|| error(span, "derive request has no definition identity"))?
            .pkg;
        let impl_identity = batch
            .reserve_identity(pkg, fuel)
            .map_err(|e| error(span, e.to_string()))?;
        let mut mapping = HashMap::<DefId, DefId>::new();
        mapping.insert(source.owner_definition(strategy).unwrap(), impl_identity);
        let mut owners = Vec::new();
        for member in &strategies[&strategy] {
            charge(fuel, span)?;
            let new_owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            mapping.insert(
                source.owner_definition(*member).ok_or_else(|| {
                    error(span, "generated method source has no definition identity")
                })?,
                identity,
            );
            let item = source.item(*member).unwrap();
            for parameter in hir::definitions::declared_clause_parameters(item) {
                let old = parameter
                    .definition
                    .ok_or_else(|| error(span, "generated clause has no identity"))?;
                let new = batch
                    .reserve_identity(pkg, fuel)
                    .map_err(|e| error(span, e.to_string()))?;
                mapping.insert(old, new);
            }
            let signature = match &item.kind {
                ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig,
                _ => unreachable!(),
            };
            for parameter in signature.params {
                if let Some(old) = source.staged_parameter_definitions.get(&parameter.hir_id) {
                    let new = batch
                        .reserve_identity(pkg, fuel)
                        .map_err(|e| error(span, e.to_string()))?;
                    mapping.insert(*old, new);
                }
            }
            owners.push((*member, new_owner, identity));
        }
        let shell = function_templates::implementation_draft(
            source,
            arena,
            request_owner,
            impl_owner,
            request,
            generator.trait_ref,
            owners.iter().map(|(_, owner, _)| *owner).collect(),
            fuel,
        )?;
        let mut drafts = vec![(shell, impl_identity)];
        for (member, owner, identity) in owners {
            drafts.push((
                function_templates::clone_member(source, arena, member, owner, &mapping, fuel)?,
                identity,
            ));
        }
        for (draft, identity) in drafts {
            for attribute in &draft.attributes {
                if omitted.contains(&attribute.lexical_owner) {
                    return Err(unsupported(
                        attribute.span,
                        "captured attribute environments",
                    ));
                }
            }
            batch
                .define(
                    OwnerInfo {
                        node: OwnerNode::Item(arena.alloc_item(draft.item)),
                        nodes: draft.nodes,
                    },
                    Some(identity),
                    None,
                )
                .map_err(|e| error(span, e.to_string()))?;
            for (id, body) in draft.bodies {
                batch
                    .body(id, body)
                    .map_err(|e| error(span, e.to_string()))?;
            }
            resolution.extend(draft.resolution);
            attributes.extend(draft.attributes);
            staged.extend(draft.staged_parameters);
        }
    }
    batch.set_resolution(resolution);
    batch.set_attributes(attributes);
    batch.set_staged_parameters(staged);
    batch
        .publish(fuel)
        .map_err(|e| error(rustc_span::DUMMY_SP, e.to_string()))?;
    for (owner, original) in source.owners() {
        charge(fuel, original.node.expect_item().span)?;
        let ItemKind::Mod(module) = &original.node.expect_item().kind else {
            continue;
        };
        let Some(info) = candidate.owner(owner).cloned() else {
            continue;
        };
        let mut item = info.node.expect_item().clone();
        let ItemKind::Mod(current) = &mut item.kind else {
            unreachable!()
        };
        for child in &module.items {
            charge(fuel, original.node.expect_item().span)?;
            if let Some(replacement) = replacements.get(child) {
                current.items.push(*replacement);
            }
        }
        let item = arena.alloc_item(item);
        let mut info = info;
        info.node = OwnerNode::Item(item);
        if info.nodes.get(hir::ItemLocalId::new(0)).is_some() {
            info.nodes.insert(
                hir::ItemLocalId::new(0),
                hir::ItemLocalId::new(0),
                hir::Node::Item(item),
            );
        }
        candidate.insert_owner(owner, info);
    }
    candidate
        .pending_semantics
        .retain(|(_, feature)| feature != "derivation expansion");
    Ok(candidate)
}
