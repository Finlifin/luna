//! Instantiate explicit strategies with checked generation computations.
use super::{Error, error, function_templates};
use hir::common::DefId;
use hir::definitions::DefinitionBatch;
use hir::{ExprKind as E, HirArena, ItemKind, OwnerId, OwnerInfo, OwnerNode, Package};
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod bindings;
mod captures;
mod clauses;
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

fn validate_strategy_parameters<'h>(
    parameters: &[hir::common::FnParam<'h>],
    fuel: &mut usize,
) -> Result<(), Error> {
    for parameter in parameters {
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
    Ok(())
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
            | E::InlineIf { .. }
            | E::InlineControl(..) => computations.push(statement),
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
    resolved_defaults: &BTreeMap<OwnerId, OwnerId>,
    validated_deriver_attrs: &BTreeSet<OwnerId>,
    fuel: &mut usize,
) -> Result<Package<'h>, Error> {
    let mut requests = Vec::new();
    let mut selected = BTreeSet::new();
    let mut strategies = BTreeMap::new();
    let mut unused_strategies = Vec::new();
    let mut omitted = BTreeSet::new();
    let mut fallback_span = source
        .item(source.root_mod)
        .map_or(rustc_span::DUMMY_SP, |item| item.span);
    for (owner, info) in source.owners() {
        let item = info.node.expect_item();
        charge(fuel, item.span)?;
        if let ItemKind::Derive(request) = &item.kind {
            fallback_span = item.span;
            let selection_span = request.strategy.map_or(item.span, |expr| expr.span);
            let (strategy, clause_arguments, arguments, extended) =
                if let Some(selection) = request.strategy {
                    let (callee, arguments, extended) = match selection.kind {
                        E::Application(head, arguments) => (head, arguments, false),
                        E::ExtendedApplication(head, arguments) => (head, arguments, true),
                        _ => (selection, &[] as &[_], false),
                    };
                    let (head, clause_arguments) = match callee.kind {
                        E::NFApplication(head, clause_args) => (head, clause_args),
                        _ => (callee, &[] as &[_]),
                    };
                    let E::ResolvedDef { def_id, .. } = head.kind else {
                        return Err(unsupported(selection.span, "computed strategy selection"));
                    };
                    let strategy = source
                        .definition_owner(def_id)
                        .ok_or_else(|| error(selection.span, "missing derivation strategy"))?;
                    (strategy, clause_arguments, arguments, extended)
                } else {
                    let strategy = resolved_defaults.get(&owner).copied().ok_or_else(|| {
                        error(item.span, "no applicable default derivation strategy found")
                    })?;
                    (strategy, &[] as &[_], &[] as &[_], false)
                };
            if !strategies.contains_key(&strategy) {
                strategies.insert(strategy, members(source, strategy, fuel)?);
            }
            let ItemKind::Deriver(generator) = &source.item(strategy).unwrap().kind else {
                unreachable!()
            };
            if (!generator.parameters.is_empty() || !strategies[&strategy].computations.is_empty())
                && !request.clause_params.is_empty()
            {
                return Err(unsupported(
                    selection_span,
                    "strategy inputs in generic request environments",
                ));
            }
            validate_strategy_parameters(generator.parameters, fuel)?;
            *fuel = fuel
                .checked_sub(arguments.len().saturating_add(generator.parameters.len()))
                .ok_or_else(|| error(selection_span, "strategy input routing budget exhausted"))?;
            // Nonempty signatures use ordinary invocation preparation, including
            // expansion and extended collections. No execution group is built
            // for a declaration-only zero-input strategy, so check that case here.
            if generator.parameters.is_empty() {
                super::arguments::route_arguments(
                    arguments,
                    Some(generator.parameters),
                    0,
                    selection_span,
                )?;
            }
            let resolved_clauses = clauses::resolve_strategy_clauses(
                generator,
                request,
                clause_arguments,
                selection_span,
                fuel,
            )?;
            selected.insert(strategy);
            omitted.insert(owner);
            requests.push((
                owner,
                request,
                strategy,
                arguments,
                extended,
                resolved_clauses,
            ));
        }
    }
    for (owner, info) in source.owners() {
        let item = info.node.expect_item();
        charge(fuel, item.span)?;
        if let ItemKind::Deriver(generator) = &item.kind {
            fallback_span = item.span;
            if !selected.contains(&owner) {
                validate_strategy_parameters(generator.parameters, fuel)?;
                let body = members(source, owner, fuel)?;
                omitted.extend(body.members.iter().copied());
                strategies.insert(owner, body);
                unused_strategies.push(owner);
            } else {
                omitted.extend(strategies[&owner].members.iter().copied());
            }
            omitted.insert(owner);
        }
    }
    // All nested source definitions must be accounted for before pruning.
    for (owner, parent) in &source.lexical_parents {
        let span = source.item(*owner).map_or(fallback_span, |item| item.span);
        charge(fuel, span)?;
        if omitted.contains(parent) && !omitted.contains(owner) {
            return Err(unsupported(span, "nested generation definitions"));
        }
    }
    for attribute in &source.definition_attributes {
        charge(fuel, attribute.span)?;
        let target = match attribute.target {
            hir::AttributeTarget::Definition(owner) => owner,
            hir::AttributeTarget::Member(id) => id.owner,
        };
        match source.item(target).map(|item| &item.kind) {
            Some(ItemKind::Deriver(_))
                if matches!(
                    attribute.target,
                    hir::AttributeTarget::Definition(owner)
                        if validated_deriver_attrs.contains(&owner)
                ) => {}
            Some(ItemKind::Derive(_) | ItemKind::Deriver(_)) => {
                return Err(unsupported(
                    attribute.span,
                    "strategy and derive-request attributes",
                ));
            }
            _ => {}
        }
    }
    let mut candidate = hir::definitions::omit_generation_sources(source, arena, &omitted, fuel)
        .map_err(|e| error(fallback_span, e.to_string()))?;
    let mut batch = DefinitionBatch::new(&mut candidate);
    let mut replacements = BTreeMap::new();
    let mut block_replacements = BTreeMap::new();
    let mut block_parent_owners = BTreeSet::new();
    let mut impl_lexical_parents = BTreeMap::new();
    let mut resolution = Vec::new();
    let mut attributes = Vec::new();
    let mut staged = Vec::new();
    let mut extra_module_items: Vec<(OwnerId, OwnerId)> = Vec::new();
    for (request_owner, request, strategy, arguments, extended, resolved_clauses) in requests {
        let ItemKind::Deriver(generator) = &source.item(strategy).unwrap().kind else {
            unreachable!()
        };
        let span = source.item(request_owner).unwrap().span;
        let mut reserve_owner = || batch.reserve().map_err(|e| error(span, e.to_string()));
        let impl_owner = reserve_owner()?;
        replacements.insert(request_owner, impl_owner);
        if let Some(&parent) = source.lexical_parents.get(&request_owner)
            && !matches!(
                source.item(parent).map(|item| &item.kind),
                Some(ItemKind::Mod(_))
            )
        {
            block_replacements.insert(request_owner, impl_owner);
            block_parent_owners.insert(parent);
            impl_lexical_parents.insert(impl_owner, parent);
        }
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
        let mut strategy_to_temp = HashMap::<DefId, DefId>::new();
        let mut temp_to_expr = HashMap::<DefId, &'h hir::Expr<'h>>::new();
        let mut inferred_clause_defs = HashMap::<DefId, DefId>::new();
        let mut extra_strategy_constraints = Vec::new();
        let mut extra_strategy_computations = Vec::new();
        let mut synthetic_local = 1u32;
        for parameter in generator.clause_params {
            charge(fuel, parameter.span)?;
            let old = parameter
                .definition
                .ok_or_else(|| error(parameter.span, "derive strategy clause has no identity"))?;
            if let Some(&bound_expr) = resolved_clauses.bound.get(&old) {
                let temp = batch
                    .reserve_identity(pkg, fuel)
                    .map_err(|e| error(parameter.span, e.to_string()))?;
                mapping.insert(old, temp);
                strategy_to_temp.insert(old, temp);
                temp_to_expr.insert(temp, bound_expr);
                match parameter.kind.unquoted() {
                    hir::clause::ClauseParamKind::TraitBound(_, bound) => {
                        let lhs = arena.alloc_expr(hir::Expr {
                            hir_id: hir::HirId::new(
                                strategy,
                                hir::ItemLocalId::new(synthetic_local),
                            ),
                            span: parameter.span,
                            kind: E::ResolvedClauseParam(parameter.name.name, old),
                        });
                        synthetic_local += 1;
                        let bound_check = arena.alloc_expr(hir::Expr {
                            hir_id: hir::HirId::new(
                                strategy,
                                hir::ItemLocalId::new(synthetic_local),
                            ),
                            span: parameter.span,
                            kind: E::TraitBound(lhs, bound),
                        });
                        synthetic_local += 1;
                        extra_strategy_constraints.push(hir::clause::ClauseConstraint {
                            label: None,
                            hir_id: hir::HirId::new(
                                strategy,
                                hir::ItemLocalId::new(synthetic_local),
                            ),
                            kind: hir::clause::ClauseConstraintKind::Requires(bound_check),
                            span: parameter.span,
                        });
                        synthetic_local += 1;
                    }
                    hir::clause::ClauseParamKind::Positional(_, ty_expr)
                    | hir::clause::ClauseParamKind::Optional(_, ty_expr, _)
                        if !parameter.kind.is_quote() && request.clause_params.is_empty() =>
                    {
                        let rhs = arena.alloc_expr(hir::Expr {
                            hir_id: hir::HirId::new(
                                strategy,
                                hir::ItemLocalId::new(synthetic_local),
                            ),
                            span: parameter.span,
                            kind: E::ResolvedClauseParam(parameter.name.name, old),
                        });
                        synthetic_local += 1;
                        let let_decl = arena.alloc_let_decl(hir::decl::LetDecl {
                            hir_id: hir::HirId::new(
                                strategy,
                                hir::ItemLocalId::new(synthetic_local),
                            ),
                            pattern: hir::pattern::Pattern {
                                hir_id: hir::HirId::new(
                                    strategy,
                                    hir::ItemLocalId::new(synthetic_local + 1),
                                ),
                                kind: hir::pattern::PatternKind::Wild,
                                span: parameter.span,
                            },
                            ty: Some(ty_expr),
                            init: Some(rhs),
                            guard: None,
                            otherwise: None,
                            mutable: false,
                            comptime: false,
                            span: parameter.span,
                        });
                        synthetic_local += 2;
                        extra_strategy_computations.push(arena.alloc_expr(hir::Expr {
                            hir_id: hir::HirId::new(
                                strategy,
                                hir::ItemLocalId::new(synthetic_local),
                            ),
                            span: parameter.span,
                            kind: E::Let(let_decl),
                        }) as &_);
                        synthetic_local += 1;
                    }
                    _ => {}
                }
            } else {
                let fresh = batch
                    .reserve_identity(pkg, fuel)
                    .map_err(|e| error(parameter.span, e.to_string()))?;
                mapping.insert(old, fresh);
                inferred_clause_defs.insert(old, fresh);
            }
        }
        let mut combined_constraints = Vec::with_capacity(
            generator.clause_constraints.len() + extra_strategy_constraints.len(),
        );
        combined_constraints.extend(extra_strategy_constraints.iter().cloned());
        combined_constraints.extend(generator.clause_constraints.iter().cloned());
        let mut combined_computations = Vec::with_capacity(
            extra_strategy_computations.len() + strategies[&strategy].computations.len(),
        );
        combined_computations.extend(extra_strategy_computations.iter().copied());
        combined_computations.extend(strategies[&strategy].computations.iter().copied());

        let mut captures = HashMap::new();
        let mut capture_slots = HashMap::new();
        let mut inputs = Vec::new();
        let mut input_stages = HashMap::new();
        let local_bindings = &strategies[&strategy].locals;
        let has_generic_req_check =
            !request.clause_params.is_empty() && !combined_constraints.is_empty();
        let group = if !request.clause_params.is_empty()
            || (generator.parameters.is_empty()
                && combined_constraints.is_empty()
                && combined_computations.is_empty()
                && resolved_clauses.inferred.is_empty())
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
        let exported = captures::retain_used(&mut drafts, &capture_slots, arena, span, fuel)?;
        for (draft, _) in &mut drafts {
            clauses::substitute_draft(
                source,
                arena,
                draft,
                &temp_to_expr,
                &strategy_to_temp,
                &mapping,
                None,
                fuel,
            )?;
        }
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
            let mut exec_draft = function_templates::execution_draft(
                source,
                arena,
                execution_owner,
                strategy,
                subject,
                generator.parameters,
                &combined_constraints,
                &input_stages,
                local_bindings,
                &combined_computations,
                &exported,
                &mapping,
                span,
                fuel,
            )?;
            if !resolved_clauses.inferred.is_empty() {
                let mut exec_clauses = Vec::with_capacity(resolved_clauses.inferred.len());
                let mut next_local = exec_draft
                    .bindings
                    .values()
                    .map(|id| id.local_id.raw())
                    .max()
                    .unwrap_or(0)
                    + 16;
                for cp in &resolved_clauses.inferred {
                    let orig_def = cp.definition.unwrap();
                    let fresh_def = inferred_clause_defs[&orig_def];
                    let mut cloned = (*cp).clone();
                    cloned.hir_id =
                        hir::HirId::new(execution_owner, hir::ItemLocalId::new(next_local));
                    next_local += 1;
                    cloned.definition = Some(fresh_def);
                    exec_clauses.push(cloned);
                }
                let exec_clause_slice = arena.alloc_clause_param_slice(exec_clauses);
                for cp in exec_clause_slice {
                    exec_draft.nodes.insert(
                        cp.hir_id.local_id,
                        hir::ItemLocalId::new(0),
                        hir::Node::ClauseParam(cp),
                    );
                }
                if let ItemKind::Fn(sig, _) = &mut exec_draft.item.kind {
                    sig.clause_params = exec_clause_slice;
                }
            }
            clauses::substitute_draft(
                source,
                arena,
                &mut exec_draft,
                &temp_to_expr,
                &strategy_to_temp,
                &mapping,
                Some(subject),
                fuel,
            )?;
            drafts.push((exec_draft, execution_identity));
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
        if has_generic_req_check {
            let target_owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let target_identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            let caller_owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let caller_identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            let mut target_clause_defs = HashMap::new();
            let mut caller_clause_defs = HashMap::new();
            for parameter in request.clause_params {
                let old = parameter.definition.unwrap();
                target_clause_defs.insert(
                    old,
                    batch
                        .reserve_identity(pkg, fuel)
                        .map_err(|e| error(parameter.span, e.to_string()))?,
                );
                caller_clause_defs.insert(
                    old,
                    batch
                        .reserve_identity(pkg, fuel)
                        .map_err(|e| error(parameter.span, e.to_string()))?,
                );
            }
            let (target_draft, caller_draft) = clauses::generic_request_requirement_check_drafts(
                source,
                arena,
                request,
                &combined_constraints,
                &temp_to_expr,
                &strategy_to_temp,
                target_owner,
                target_identity,
                caller_owner,
                &target_clause_defs,
                &caller_clause_defs,
                span,
                fuel,
            )?;
            drafts.push((target_draft, target_identity));
            drafts.push((caller_draft, caller_identity));
            extra_module_items.push((source.root_mod, target_owner));
            extra_module_items.push((source.root_mod, caller_owner));
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
    for strategy in unused_strategies {
        let item = source.item(strategy).unwrap();
        let ItemKind::Deriver(generator) = &item.kind else {
            unreachable!()
        };
        let span = item.span;
        let pkg = source
            .owner_definition(strategy)
            .ok_or_else(|| {
                error(
                    span,
                    "unused derivation strategy has no definition identity",
                )
            })?
            .pkg;
        let check_owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
        let check_identity = batch
            .reserve_identity(pkg, fuel)
            .map_err(|e| error(span, e.to_string()))?;
        let self_def = batch
            .reserve_identity(pkg, fuel)
            .map_err(|e| error(span, e.to_string()))?;
        let mut clause_defs = HashMap::new();
        for parameter in generator.clause_params {
            charge(fuel, parameter.span)?;
            let old = parameter.definition.ok_or_else(|| {
                error(
                    parameter.span,
                    "unused derivation strategy clause has no identity",
                )
            })?;
            let fresh = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(parameter.span, e.to_string()))?;
            clause_defs.insert(old, fresh);
        }
        let mut input_stages = HashMap::new();
        for parameter in generator.parameters {
            charge(fuel, parameter.span)?;
            let stage = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(parameter.span, e.to_string()))?;
            input_stages.insert(parameter.hir_id, stage);
            if let Some(old) = source.staged_parameter_definitions.get(&parameter.hir_id) {
                clause_defs.insert(*old, stage);
            }
        }
        let draft = clauses::unused_strategy_check_draft(
            source,
            arena,
            strategy,
            check_owner,
            self_def,
            &clause_defs,
            &input_stages,
            &strategies[&strategy],
            fuel,
        )?;
        batch
            .define(
                OwnerInfo {
                    node: OwnerNode::Item(arena.alloc_item(draft.item)),
                    nodes: draft.nodes,
                },
                Some(check_identity),
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
        extra_module_items.push((source.root_mod, check_owner));
    }
    batch.set_resolution(resolution);
    batch.set_attributes(attributes);
    batch.set_staged_parameters(staged);
    batch
        .publish(fuel)
        .map_err(|e| error(fallback_span, e.to_string()))?;
    for (impl_owner, parent) in impl_lexical_parents {
        candidate.lexical_parents.insert(impl_owner, parent);
    }
    clauses::rewrite_body_item_owners(
        source,
        &mut candidate,
        arena,
        &block_parent_owners,
        &block_replacements,
        fuel,
    )?;
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
        for &(target_mod, extra_owner) in &extra_module_items {
            if target_mod == owner {
                current.items.push(extra_owner);
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
    let cleared_all_attributes =
        !source.definition_attributes.is_empty() && candidate.definition_attributes.is_empty();
    candidate.pending_semantics.retain(|(_, feature)| {
        feature != "derivation expansion"
            && (!cleared_all_attributes
                || !matches!(
                    feature.as_str(),
                    "definition attribute" | "definition configuration"
                ))
    });
    Ok(candidate)
}
