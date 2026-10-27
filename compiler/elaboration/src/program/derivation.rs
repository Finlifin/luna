//! Instantiate explicit strategies with checked generation computations.
use super::{Error, error, function_templates};
use hir::common::DefId;
use hir::definitions::DefinitionBatch;
use hir::{ExprKind as E, HirArena, ItemKind, OwnerId, OwnerInfo, OwnerNode, Package};
use std::collections::{BTreeMap, BTreeSet, HashMap};
mod bindings;
mod captures;
mod diagnostics;
pub(super) use diagnostics::with_context;

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
struct StrategyBody<'h> {
    members: Vec<OwnerId>,
    locals: Vec<function_templates::GenerationBinding<'h>>,
    computations: Vec<&'h hir::Expr<'h>>,
}

fn members<'h>(
    source: &Package<'h>,
    strategy: OwnerId,
    fuel: &mut usize,
) -> Result<StrategyBody<'h>, Error> {
    let item = source.item(strategy).unwrap();
    let ItemKind::Deriver(def) = &item.kind else {
        return Err(error(item.span, "derive selection must name a strategy"));
    };
    if !def.clause_params.is_empty() {
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
    let mut locals = Vec::new();
    let mut computations = Vec::new();
    for statement in block.stmts.iter().chain(block.expr.into_iter()) {
        charge(fuel, statement.span)?;
        let statement = if let E::Semi(value) = statement.kind {
            value
        } else {
            statement
        };
        match statement.kind {
            E::Let(binding) => {
                bindings::collect(&binding.pattern, binding.ty, &mut locals, fuel)?;
                computations.push(statement);
            }
            E::Item(owner)
                if matches!(
                    source.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(_) | ItemKind::Associated(_))
                ) =>
            {
                result.push(owner)
            }
            E::Unit => {}
            E::Application(..)
            | E::ExtendedApplication(..)
            | E::Assign(..)
            | E::AssignOp(..)
            | E::If(..)
            | E::Block(..)
            | E::Loop(..)
            | E::Match(..)
            | E::When(..)
            | E::For { .. }
            | E::InlineFor { .. }
            | E::InlineIf { .. } => computations.push(statement),
            _ => {
                return Err(unsupported(statement.span, "strategy statement execution"));
            }
        }
    }
    Ok(StrategyBody {
        members: result,
        locals,
        computations,
    })
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
            let selection = request
                .strategy
                .ok_or_else(|| unsupported(item.span, "default strategy selection"))?;
            let (head, arguments, extended) = match selection.kind {
                E::Application(head, arguments) => (head, arguments, false),
                E::ExtendedApplication(head, arguments) => (head, arguments, true),
                _ => (selection, &[] as &[_], false),
            };
            let E::ResolvedDef { def_id, .. } = head.kind else {
                return Err(unsupported(selection.span, "computed strategy selection"));
            };
            let strategy = source
                .definition_owner(def_id)
                .ok_or_else(|| error(selection.span, "missing derivation strategy"))?;
            if !strategies.contains_key(&strategy) {
                strategies.insert(strategy, members(source, strategy, fuel)?);
            }
            let ItemKind::Deriver(generator) = &source.item(strategy).unwrap().kind else {
                unreachable!()
            };
            if (!generator.parameters.is_empty()
                || !generator.clause_constraints.is_empty()
                || !strategies[&strategy].computations.is_empty())
                && !request.clause_params.is_empty()
            {
                return Err(unsupported(
                    selection.span,
                    "strategy inputs in generic request environments",
                ));
            }
            for parameter in generator.parameters {
                charge(fuel, parameter.span)?;
                let supported_flags = hir::common::TPARAM_COMPTIME
                    | hir::common::TPARAM_IMPLICIT
                    | hir::common::TPARAM_LAMBDA;
                if parameter.ty.flags & !supported_flags != 0 {
                    return Err(unsupported(
                        parameter.span,
                        "specialized strategy parameter matching",
                    ));
                }
                if !matches!(
                    parameter.ty.kind,
                    hir::common::TyParamKind::Positional(_)
                        | hir::common::TyParamKind::Optional(..)
                        | hir::common::TyParamKind::Varadic(..)
                ) {
                    return Err(unsupported(parameter.span, "strategy parameter shape"));
                }
            }
            *fuel = fuel
                .checked_sub(arguments.len().saturating_add(generator.parameters.len()))
                .ok_or_else(|| error(selection.span, "strategy input routing budget exhausted"))?;
            // Nonempty signatures use ordinary invocation preparation, including
            // expansion and extended collections. No execution group is built
            // for a declaration-only zero-input strategy, so check that case here.
            if generator.parameters.is_empty() {
                super::arguments::route_arguments(
                    arguments,
                    Some(generator.parameters),
                    0,
                    selection.span,
                )?;
            }
            selected.insert(strategy);
            omitted.insert(owner);
            requests.push((owner, request, strategy, arguments, extended));
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
            omitted.extend(strategies[&owner].members.iter().copied());
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
    for (request_owner, request, strategy, arguments, extended) in requests {
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
        for parameter in request.clause_params {
            charge(fuel, parameter.span)?;
            let old = parameter
                .definition
                .ok_or_else(|| error(parameter.span, "derive request clause has no identity"))?;
            let new = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(parameter.span, e.to_string()))?;
            mapping.insert(old, new);
        }
        let mut captures = HashMap::new();
        let mut capture_slots = HashMap::new();
        let mut inputs = Vec::new();
        let mut input_stages = HashMap::new();
        let local_bindings = &strategies[&strategy].locals;
        let computations = &strategies[&strategy].computations;
        let group = if generator.parameters.is_empty()
            && generator.clause_constraints.is_empty()
            && computations.is_empty()
        {
            None
        } else {
            let owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            Some((owner, identity))
        };
        let execution = if group.is_some() {
            Some((
                batch.reserve().map_err(|e| error(span, e.to_string()))?,
                batch
                    .reserve_identity(pkg, fuel)
                    .map_err(|e| error(span, e.to_string()))?,
            ))
        } else {
            None
        };
        let subject_capture = if group.is_some() {
            Some((
                batch.reserve().map_err(|e| error(span, e.to_string()))?,
                batch
                    .reserve_identity(pkg, fuel)
                    .map_err(|e| error(span, e.to_string()))?,
            ))
        } else {
            None
        };
        for parameter in generator.parameters {
            charge(fuel, parameter.span)?;
            let owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            let stage = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            input_stages.insert(parameter.hir_id, stage);
            if let Some(old) = source.staged_parameter_definitions.get(&parameter.hir_id) {
                mapping.insert(*old, stage);
            }
            captures.insert(
                parameter.hir_id,
                function_templates::InputCapture::Definition(identity),
            );
            capture_slots.insert(identity, inputs.len());
            inputs.push((parameter, owner, identity));
        }
        let mut locals = Vec::new();
        for binding in local_bindings {
            charge(fuel, binding.span)?;
            let owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            captures.insert(
                binding.id,
                function_templates::InputCapture::Definition(identity),
            );
            capture_slots.insert(identity, generator.parameters.len() + locals.len());
            locals.push((binding, owner, identity));
        }
        let mut owners = Vec::new();
        for member in &strategies[&strategy].members {
            charge(fuel, span)?;
            let new_owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            mapping.insert(
                source.owner_definition(*member).ok_or_else(|| {
                    error(span, "generated member source has no definition identity")
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
            let parameters = match &item.kind {
                ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig.params,
                ItemKind::Associated(definition) => definition.parameters,
                _ => unreachable!(),
            };
            for parameter in parameters {
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
            group
                .iter()
                .map(|(owner, _)| *owner)
                .chain(subject_capture.iter().map(|(owner, _)| *owner))
                .chain(inputs.iter().map(|(_, owner, _)| *owner))
                .chain(locals.iter().map(|(_, owner, _)| *owner))
                .chain(owners.iter().map(|(_, owner, _)| *owner))
                .collect(),
            &mapping,
            &captures,
            fuel,
        )?;
        let mut drafts = vec![(shell, impl_identity)];
        if let Some((_, identity)) = group {
            for (index, (parameter, owner, input_identity)) in inputs.into_iter().enumerate() {
                drafts.push((
                    function_templates::input_draft(
                        source, arena, owner, parameter, identity, index, &mapping, &captures, fuel,
                    )?,
                    input_identity,
                ));
            }
        }
        if let Some((_, group_identity)) = group {
            for (index, (binding, owner, identity)) in locals.into_iter().enumerate() {
                drafts.push((
                    function_templates::value_draft(
                        source,
                        arena,
                        owner,
                        binding.annotation,
                        binding.span,
                        group_identity,
                        generator.parameters.len() + index,
                        &mapping,
                        &captures,
                        fuel,
                    )?,
                    identity,
                ));
            }
        }
        for (member, owner, identity) in owners {
            drafts.push((
                function_templates::clone_member_with_inputs(
                    source, arena, member, owner, &mapping, &captures, fuel,
                )?,
                identity,
            ));
        }
        let ((shell, _), members) = drafts.split_first_mut().unwrap();
        function_templates::relocate_generation_attributes(
            source, arena, strategy, shell, members, &mapping, &captures, fuel,
        )?;
        let exported = captures::retain_used(&mut drafts, &capture_slots, arena, fuel)?;
        if let Some((owner, identity)) = group {
            let (execution_owner, execution_identity) = execution.unwrap();
            let (subject_owner, subject_identity) = subject_capture.unwrap();
            drafts.push((
                function_templates::subject_draft(arena, subject_owner, span, fuel)?,
                subject_identity,
            ));
            let subject = arena.alloc_expr(hir::Expr {
                hir_id: hir::HirId::new(subject_owner, hir::ItemLocalId::new(3)),
                span,
                kind: E::ResolvedDef {
                    name: hir::Symbol::intern("$derive_subject"),
                    def_id: subject_identity,
                    has_clause_param: false,
                },
            });
            drafts.push((
                function_templates::execution_draft(
                    source,
                    arena,
                    execution_owner,
                    strategy,
                    subject,
                    generator.parameters,
                    generator.clause_constraints,
                    &input_stages,
                    local_bindings,
                    computations,
                    &exported,
                    &mapping,
                    span,
                    fuel,
                )?,
                execution_identity,
            ));
            drafts.push((
                function_templates::input_group(
                    source,
                    arena,
                    owner,
                    execution_identity,
                    arguments,
                    extended,
                    span,
                    fuel,
                )?,
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
