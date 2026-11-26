//! Instantiate explicit strategies with checked generation computations.
use super::{Error, error, function_templates};
use hir::common::DefId;
use hir::definitions::DefinitionBatch;
use hir::{ExprKind as E, HirArena, ItemKind, OwnerId, OwnerInfo, OwnerNode, Package};
use std::collections::{BTreeMap, BTreeSet};

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
        let mut mapping = BTreeMap::<DefId, DefId>::new();
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
        let mut strategy_to_temp = BTreeMap::<DefId, DefId>::new();
        let mut temp_to_expr = BTreeMap::<DefId, &'h hir::Expr<'h>>::new();
        let mut inferred_clause_defs = BTreeMap::<DefId, DefId>::new();
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
                        if !parameter.kind.is_quote() =>
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

        let mut captures = BTreeMap::new();
        let mut capture_slots = BTreeMap::new();
        let mut inputs = Vec::new();
        let mut input_stages = BTreeMap::new();
        let local_bindings = &strategies[&strategy].locals;
        let group = if generator.parameters.is_empty()
            && (combined_constraints.is_empty() || !request.clause_params.is_empty())
            && combined_computations.is_empty()
            && resolved_clauses.inferred.is_empty()
        {
            None
        } else {
            let owner = batch.reserve().map_err(|e| error(span, e.to_string()))?;
            let identity = batch
                .reserve_identity(pkg, fuel)
                .map_err(|e| error(span, e.to_string()))?;
            Some((owner, identity))
        };
        let mut static_req_constraints = Vec::new();
        let mut exec_constraints = Vec::new();
        for constraint in &combined_constraints {
            let mentions_fn_param = match constraint.kind {
                hir::clause::ClauseConstraintKind::Requires(e)
                | hir::clause::ClauseConstraintKind::Ensures(e)
                | hir::clause::ClauseConstraintKind::Decreases(e) => {
                    clauses::expr_mentions_fn_param(e, fuel)?
                }
                hir::clause::ClauseConstraintKind::Outcome => false,
            };
            if request.clause_params.is_empty() || mentions_fn_param {
                exec_constraints.push(constraint.clone());
            } else {
                static_req_constraints.push(constraint.clone());
            }
        }
        let has_generic_req_check =
            !request.clause_params.is_empty() && !static_req_constraints.is_empty();
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
        let subject_capture = if group.is_some() && request.clause_params.is_empty() {
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
                    source, arena, member, owner, &mapping, &captures, span, fuel,
                )?,
                identity,
            ));
        }
        let ((shell, _), members) = drafts
            .split_first_mut()
            .ok_or_else(|| error(span, "missing derivation shell draft"))?;
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
        splice_auto::maybe_expand(
            source,
            arena,
            strategy,
            validated_deriver_attrs,
            &mut drafts,
            span,
            fuel,
        )?;
        if let Some((owner, identity)) = group {
            let (execution_owner, execution_identity) = execution.unwrap();
            let mut exec_clause_defs = BTreeMap::new();
            let mut quote_request_defs = BTreeSet::new();
            let mut exec_clause_args = Vec::new();
            for parameter in request.clause_params {
                let old = parameter.definition.unwrap();
                if parameter.kind.is_quote() {
                    quote_request_defs.insert(old);
                    continue;
                }
                let fresh = batch
                    .reserve_identity(pkg, fuel)
                    .map_err(|e| error(parameter.span, e.to_string()))?;
                exec_clause_defs.insert(old, fresh);
                exec_clause_args.push((parameter.name, old));
            }
            let subject = if let Some((subject_owner, subject_identity)) = subject_capture {
                drafts.push((
                    function_templates::subject_draft(arena, subject_owner, span, fuel)?,
                    subject_identity,
                ));
                arena.alloc_expr(hir::Expr {
                    hir_id: hir::HirId::new(subject_owner, hir::ItemLocalId::new(3)),
                    span,
                    kind: E::ResolvedDef {
                        name: hir::Symbol::intern("$derive_subject"),
                        def_id: subject_identity,
                        has_clause_param: false,
                    },
                })
            } else {
                let mentions_quote =
                    clauses::expr_mentions_any_def(request.self_ty, &quote_request_defs, fuel)?;
                if mentions_quote {
                    // Self cannot be inspected at comptime if it references an open
                    // runtime quote parameter; emit a placeholder unless unused.
                    arena.alloc_expr(hir::Expr {
                        hir_id: hir::HirId::new(execution_owner, hir::ItemLocalId::new(3)),
                        span,
                        kind: E::TyVoid,
                    })
                } else {
                    clauses::rewrite_request_self_for_exec(
                        source,
                        arena,
                        execution_owner,
                        request.self_ty,
                        &exec_clause_defs,
                        fuel,
                    )?
                }
            };
            let mut exec_definitions = mapping.clone();
            if !request.clause_params.is_empty() {
                for &fresh in exec_clause_defs.values() {
                    exec_definitions.insert(fresh, fresh);
                }
            }
            let mut exec_draft = function_templates::execution_draft(
                source,
                arena,
                execution_owner,
                strategy,
                subject,
                generator.parameters,
                &exec_constraints,
                &input_stages,
                local_bindings,
                &combined_computations,
                &exported,
                &exec_definitions,
                span,
                fuel,
            )?;
            extra_module_items.push((source.root_mod, execution_owner));
            if !request.clause_params.is_empty() {
                clauses::attach_execution_clauses(
                    source,
                    arena,
                    request,
                    &mut exec_draft,
                    &exec_clause_defs,
                    &resolved_clauses.inferred,
                    &inferred_clause_defs,
                    fuel,
                )?;
            } else if !resolved_clauses.inferred.is_empty() {
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
            let exec_request_mapping = if request.clause_params.is_empty() {
                &mapping
            } else {
                &exec_clause_defs
            };
            clauses::substitute_draft(
                source,
                arena,
                &mut exec_draft,
                &temp_to_expr,
                &strategy_to_temp,
                exec_request_mapping,
                Some(subject),
                fuel,
            )?;
            if !request.clause_params.is_empty() {
                clauses::prune_unused_execution_clauses(
                    arena,
                    &mut exec_draft,
                    &exec_clause_defs,
                    &mut exec_clause_args,
                    fuel,
                )?;
            }
            drafts.push((exec_draft, execution_identity));
            drafts.push((
                function_templates::input_group(
                    source,
                    arena,
                    owner,
                    execution_identity,
                    &mapping,
                    &exec_clause_args,
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
            let mut target_clause_defs = BTreeMap::new();
            let mut caller_clause_defs = BTreeMap::new();
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
                &static_req_constraints,
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
        let mut clause_defs = BTreeMap::new();
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
        let mut input_stages = BTreeMap::new();
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

mod splice_auto {
    use super::*;
    use hir::common::{Arg, BindingMode, Ident};
    use hir::item::{Variant, VariantKind};
    use hir::pattern::{FieldPat, Pattern, PatternArm, PatternKind as P};
    use hir::{Expr, HirId, ItemLocalId, Node};

    pub(super) fn maybe_expand<'h>(
        source: &Package<'h>,
        arena: &'h HirArena,
        strategy: OwnerId,
        validated_deriver_attrs: &BTreeSet<OwnerId>,
        drafts: &mut [(function_templates::ClonedMember<'h>, DefId)],
        span: rustc_span::Span,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        if !validated_deriver_attrs.contains(&strategy) {
            return Ok(());
        }
        let Some(strategy_item) = source.item(strategy) else {
            return Ok(());
        };
        if strategy_item.ident.name != "splice_auto" {
            return Ok(());
        }
        let ItemKind::Deriver(generator) = &strategy_item.kind else {
            return Ok(());
        };
        if !generator.parameters.is_empty() {
            return Ok(());
        }
        let Some(strategy_def) = source.owner_definition(strategy) else {
            return Ok(());
        };
        let Some(trait_owner) = expr_head_owner(source, generator.trait_ref) else {
            return Ok(());
        };
        let Some(trait_def) = source.owner_definition(trait_owner) else {
            return Ok(());
        };
        if strategy_def.pkg != trait_def.pkg || !is_builtin_splice_trait(source, trait_owner) {
            return Ok(());
        }
        let Some((shell_draft, _)) = drafts.first() else {
            return Ok(());
        };
        let ItemKind::Impl(impl_def) = &shell_draft.item.kind else {
            return Ok(());
        };
        let self_ty_expr = impl_def.self_ty;
        let subject_owner = expr_head_owner(source, self_ty_expr).ok_or_else(|| {
            error(
                span,
                "derive(Splice) requires a nominal struct or enum type",
            )
        })?;
        let drop_nominals = collect_drop_nominals(source, trait_def.pkg, fuel)?;
        validate_finite_hir(
            source,
            subject_owner,
            &drop_nominals,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            span,
            fuel,
        )?;
        let Some((fn_draft, _)) = drafts
            .iter_mut()
            .skip(1)
            .find(|(draft, _)| draft.item.ident.name == "splice")
        else {
            return Ok(());
        };
        rewrite_splice_fn_body(
            source,
            arena,
            fn_draft,
            self_ty_expr,
            subject_owner,
            span,
            fuel,
        )
    }

    fn expr_head_owner<'h>(source: &Package<'h>, mut expr: &'h Expr<'h>) -> Option<OwnerId> {
        loop {
            match expr.kind {
                E::ResolvedDef { def_id, .. } => return source.definition_owner(def_id),
                E::NFApplication(head, _) | E::Application(head, _) => expr = head,
                _ => return None,
            }
        }
    }

    fn is_builtin_splice_trait<'h>(source: &Package<'h>, owner: OwnerId) -> bool {
        let Some(item) = source.item(owner) else {
            return false;
        };
        if item.ident.name != "Splice" {
            return false;
        }
        let ItemKind::Trait(def) = &item.kind else {
            return false;
        };
        if def.items.len() != 2 {
            return false;
        }
        let mut saw_runtime_target = false;
        let mut saw_splice_fn = false;
        for &member_owner in &def.items {
            let Some(member) = source.item(member_owner) else {
                return false;
            };
            match &member.kind {
                ItemKind::Associated(_) if member.ident.name == "RuntimeTarget" => {
                    saw_runtime_target = true;
                }
                ItemKind::FnDeclaration(sig)
                    if member.ident.name == "splice"
                        && sig.params.len() == 1
                        && sig.params[0].ty.is_comptime() =>
                {
                    saw_splice_fn = true;
                }
                _ => return false,
            }
        }
        saw_runtime_target && saw_splice_fn
    }

    fn is_builtin_drop_trait<'h>(source: &Package<'h>, owner: OwnerId) -> bool {
        let Some(item) = source.item(owner) else {
            return false;
        };
        if item.ident.name != "Drop" {
            return false;
        }
        let ItemKind::Trait(def) = &item.kind else {
            return false;
        };
        if def.items.len() != 1 {
            return false;
        }
        let Some(member) = source.item(def.items[0]) else {
            return false;
        };
        matches!(
            &member.kind,
            ItemKind::FnDeclaration(sig)
                if member.ident.name == "drop"
                    && sig.params.len() == 1
                    && matches!(
                        sig.params[0].ty.kind,
                        hir::common::TyParamKind::Itself { is_ref: true }
                    )
        )
    }

    fn is_drop_trait<'h>(source: &Package<'h>, trait_ref: &'h Expr<'h>, builtin_pkg: u32) -> bool {
        let Some(owner) = expr_head_owner(source, trait_ref) else {
            return false;
        };
        source
            .owner_definition(owner)
            .is_some_and(|def| def.pkg == builtin_pkg)
            && is_builtin_drop_trait(source, owner)
    }

    fn collect_drop_nominals<'h>(
        source: &Package<'h>,
        builtin_pkg: u32,
        fuel: &mut usize,
    ) -> Result<BTreeSet<OwnerId>, Error> {
        let mut dropped = BTreeSet::new();
        for (_, info) in source.owners() {
            let item = info.node.expect_item();
            charge(fuel, item.span)?;
            if let ItemKind::Impl(impl_def) = &item.kind
                && let Some(trait_ref) = impl_def.trait_ref
                && is_drop_trait(source, trait_ref, builtin_pkg)
                && let Some(subject) = expr_head_owner(source, impl_def.self_ty)
            {
                dropped.insert(subject);
            }
        }
        Ok(dropped)
    }

    fn validate_finite_hir<'h>(
        source: &Package<'h>,
        owner: OwnerId,
        drop_nominals: &BTreeSet<OwnerId>,
        visiting: &mut BTreeSet<OwnerId>,
        visited: &mut BTreeSet<OwnerId>,
        span: rustc_span::Span,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        charge(fuel, span)?;
        if !visiting.insert(owner) {
            return Err(error(span, "cannot auto-derive Splice for cyclic type"));
        }
        if drop_nominals.contains(&owner) {
            return Err(error(span, "cannot auto-derive Splice for Drop type"));
        }
        if visited.contains(&owner) {
            visiting.remove(&owner);
            return Ok(());
        }
        let item = source
            .item(owner)
            .ok_or_else(|| error(span, "derive(Splice) target item is missing"))?;
        match &item.kind {
            ItemKind::Struct(def) => {
                for field in def.fields {
                    validate_hir_ty(
                        source,
                        field.ty,
                        Some(owner),
                        drop_nominals,
                        visiting,
                        visited,
                        fuel,
                    )?;
                }
            }
            ItemKind::Enum(def) => {
                validate_enum_variants(
                    source,
                    def.variants,
                    owner,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )?;
            }
            ItemKind::TypeAlias(target, ..) | ItemKind::Newtype(target, ..) => {
                validate_hir_ty(
                    source,
                    target,
                    Some(owner),
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )?;
            }
            _ => {
                visiting.remove(&owner);
                return Err(error(
                    item.span,
                    "derive(Splice) requires a struct or enum target type",
                ));
            }
        }
        visiting.remove(&owner);
        visited.insert(owner);
        Ok(())
    }

    fn validate_enum_variants<'h>(
        source: &Package<'h>,
        variants: &'h [Variant<'h>],
        enum_owner: OwnerId,
        drop_nominals: &BTreeSet<OwnerId>,
        visiting: &mut BTreeSet<OwnerId>,
        visited: &mut BTreeSet<OwnerId>,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        for variant in variants {
            charge(fuel, variant.span)?;
            match &variant.kind {
                VariantKind::Unit | VariantKind::Const(_) => {}
                VariantKind::Tuple(fields) => {
                    for field_ty in *fields {
                        validate_hir_ty(
                            source,
                            field_ty,
                            Some(enum_owner),
                            drop_nominals,
                            visiting,
                            visited,
                            fuel,
                        )?;
                    }
                }
                VariantKind::Struct(fields) => {
                    for field in *fields {
                        validate_hir_ty(
                            source,
                            field.ty,
                            Some(enum_owner),
                            drop_nominals,
                            visiting,
                            visited,
                            fuel,
                        )?;
                    }
                }
                VariantKind::SubEnum(nested) => {
                    validate_enum_variants(
                        source,
                        nested,
                        enum_owner,
                        drop_nominals,
                        visiting,
                        visited,
                        fuel,
                    )?;
                }
                VariantKind::Pattern(_) => {
                    return Err(error(
                        variant.span,
                        "cannot auto-derive Splice for pattern enum variant",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_ty_params<'h>(
        source: &Package<'h>,
        params: &'h [hir::common::TyParam<'h>],
        current_nominal: Option<OwnerId>,
        drop_nominals: &BTreeSet<OwnerId>,
        visiting: &mut BTreeSet<OwnerId>,
        visited: &mut BTreeSet<OwnerId>,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        for param in params {
            if param.is_comptime() || param.is_quote() {
                return Err(error(
                    param.span,
                    "cannot auto-derive Splice across comptime or quote callable parameter",
                ));
            }
            match param.kind {
                hir::common::TyParamKind::Positional(ty)
                | hir::common::TyParamKind::PositionalDependencyCatched(_, ty)
                | hir::common::TyParamKind::Varadic(_, ty)
                | hir::common::TyParamKind::Optional(_, ty, _) => {
                    validate_hir_ty(
                        source,
                        ty,
                        current_nominal,
                        drop_nominals,
                        visiting,
                        visited,
                        fuel,
                    )?;
                }
                hir::common::TyParamKind::Itself { is_ref } => {
                    if is_ref {
                        return Err(error(
                            param.span,
                            "cannot auto-derive Splice across `*self` callable parameter",
                        ));
                    }
                }
                hir::common::TyParamKind::Case(_) => {}
            }
        }
        Ok(())
    }

    fn validate_hir_ty<'h>(
        source: &Package<'h>,
        expr: &'h Expr<'h>,
        current_nominal: Option<OwnerId>,
        drop_nominals: &BTreeSet<OwnerId>,
        visiting: &mut BTreeSet<OwnerId>,
        visited: &mut BTreeSet<OwnerId>,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        charge(fuel, expr.span)?;
        match expr.kind {
            E::TyPtr(_) => Err(error(
                expr.span,
                "cannot auto-derive Splice across shared reference `*T`",
            )),
            E::LiftType(_) => Err(error(
                expr.span,
                "cannot auto-derive Splice across `lift` type",
            )),
            E::TyTraitObject(_) => Err(error(
                expr.span,
                "cannot auto-derive Splice across `dyn` trait object",
            )),
            E::TyComptimeFn(_) => Err(error(
                expr.span,
                "cannot auto-derive Splice across comptime function type",
            )),
            E::TyNFFn(_) => Err(error(
                expr.span,
                "cannot auto-derive Splice across normal-form function type",
            )),
            E::ContextualExprType(..) | E::ContextualPatternType(..) => Err(error(
                expr.span,
                "cannot auto-derive Splice across contextual code type",
            )),
            E::TyModifiedFn(mods, params) => {
                if mods.is_comptime {
                    return Err(error(
                        expr.span,
                        "cannot auto-derive Splice across comptime function type",
                    ));
                }
                validate_ty_params(
                    source,
                    params,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )
            }
            E::TyFn(params) => validate_ty_params(
                source,
                params,
                current_nominal,
                drop_nominals,
                visiting,
                visited,
                fuel,
            ),
            E::TyFnArrow(lhs, rhs) => {
                validate_hir_ty(
                    source,
                    lhs,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )?;
                validate_hir_ty(
                    source,
                    rhs,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )
            }
            E::TyClosure(env, sig) => {
                validate_hir_ty(
                    source,
                    env,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )?;
                validate_hir_ty(
                    source,
                    sig,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )
            }
            E::TyClosureEnvironment(bindings) => {
                for binding in bindings {
                    validate_hir_ty(
                        source,
                        binding.ty,
                        current_nominal,
                        drop_nominals,
                        visiting,
                        visited,
                        fuel,
                    )?;
                }
                Ok(())
            }
            E::TySelf => {
                if current_nominal.is_some() {
                    Err(error(
                        expr.span,
                        "cannot auto-derive Splice for cyclic `Self` field",
                    ))
                } else {
                    Ok(())
                }
            }
            E::TyOptional(inner)
            | E::EffectQualifiedType(_, inner)
            | E::RefinedType { base: inner, .. } => validate_hir_ty(
                source,
                inner,
                current_nominal,
                drop_nominals,
                visiting,
                visited,
                fuel,
            ),
            E::ErrorQualifiedType(err, ok) => {
                validate_hir_ty(
                    source,
                    err,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )?;
                validate_hir_ty(
                    source,
                    ok,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )
            }
            E::Tuple(elements) => {
                for element in elements {
                    validate_hir_ty(
                        source,
                        element,
                        current_nominal,
                        drop_nominals,
                        visiting,
                        visited,
                        fuel,
                    )?;
                }
                Ok(())
            }
            E::Object(ref entries) => {
                for entry in entries {
                    let target = match entry {
                        hir::expr::ObjectEntry::Value(e) => e,
                        hir::expr::ObjectEntry::Property(f) => f.expr,
                    };
                    validate_hir_ty(
                        source,
                        target,
                        current_nominal,
                        drop_nominals,
                        visiting,
                        visited,
                        fuel,
                    )?;
                }
                Ok(())
            }
            E::ResolvedDef { def_id, .. } => {
                if let Some(owner) = source.definition_owner(def_id) {
                    match source.item(owner).map(|item| &item.kind) {
                        Some(
                            ItemKind::Struct(_)
                            | ItemKind::Enum(_)
                            | ItemKind::TypeAlias(..)
                            | ItemKind::Newtype(..),
                        ) => {
                            validate_finite_hir(
                                source,
                                owner,
                                drop_nominals,
                                visiting,
                                visited,
                                expr.span,
                                fuel,
                            )?;
                        }
                        Some(ItemKind::Const(_, init))
                            if matches!(
                                &init.kind,
                                E::Builtin(sym)
                                    if matches!(
                                        sym.as_str(),
                                        "Type"
                                            | "Object"
                                            | "Continuation"
                                            | "List"
                                            | "Integer"
                                            | "Float"
                                    )
                            ) =>
                        {
                            return Err(error(
                                expr.span,
                                "cannot auto-derive Splice across comptime-only type",
                            ));
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            E::NFApplication(head, args)
            | E::Application(head, args)
            | E::ExtendedApplication(head, args) => {
                validate_hir_ty(
                    source,
                    head,
                    current_nominal,
                    drop_nominals,
                    visiting,
                    visited,
                    fuel,
                )?;
                for arg in args {
                    let arg_expr = match arg {
                        Arg::Positional(e)
                        | Arg::Named(_, e)
                        | Arg::Expand(e)
                        | Arg::Implicit(e)
                        | Arg::PostLambda(e) => e,
                    };
                    validate_hir_ty(
                        source,
                        arg_expr,
                        current_nominal,
                        drop_nominals,
                        visiting,
                        visited,
                        fuel,
                    )?;
                }
                Ok(())
            }
            E::TyType => Err(error(
                expr.span,
                "cannot auto-derive Splice across comptime-only `Type`",
            )),
            E::Builtin(sym)
                if matches!(
                    sym.as_str(),
                    "Type" | "Object" | "Continuation" | "List" | "Integer" | "Float"
                ) =>
            {
                Err(error(
                    expr.span,
                    format!(
                        "cannot auto-derive Splice across comptime-only `{}`",
                        sym.as_str()
                    ),
                ))
            }
            E::DeliveredType(_)
            | E::ConstantObjectEntry(..)
            | E::Lit(_)
            | E::CodeQuote(..)
            | E::CodePatternQuote(..)
            | E::CodeSplice(..)
            | E::LiteralConversion { .. }
            | E::Path(_)
            | E::UnresolvedIdent(_)
            | E::TacticParameter(_)
            | E::TacticArgument(_)
            | E::PostLambdaBody(_)
            | E::ResolvedLocal(..)
            | E::ResolvedFnParam(..)
            | E::ResolvedClauseParam(..)
            | E::ResolvedLoopLabel(..)
            | E::SelfValue
            | E::Index(..)
            | E::Range { .. }
            | E::CollectedProperties(_)
            | E::Binary(..)
            | E::Unary(..)
            | E::If(..)
            | E::When(_)
            | E::Block(_)
            | E::UnsafeBlock(_)
            | E::Loop(_)
            | E::Match(..)
            | E::EffectRequest(_)
            | E::EffectHandle(..)
            | E::Assign(..)
            | E::AssignOp(..)
            | E::Assumes(_)
            | E::Asserts(_)
            | E::Invariant(_)
            | E::Decreases(_)
            | E::Defer(_)
            | E::InlineControl(_)
            | E::RefinementSubject
            | E::Return(_)
            | E::OptionPropagate(_)
            | E::Resume(_)
            | E::Break(_)
            | E::BreakTo(_)
            | E::Continue(_)
            | E::ContinueTo(_)
            | E::Projection(..)
            | E::List(_)
            | E::Ref(_)
            | E::Deref(_)
            | E::ErrorNew(_)
            | E::ErrorPropagate(_)
            | E::ErrorHandle { .. }
            | E::HandlerApplication(..)
            | E::Closure(..)
            | E::Cast(..)
            | E::DynCast(..)
            | E::Let(_)
            | E::Semi(_)
            | E::Item(_)
            | E::Undefined
            | E::Null
            | E::Unit
            | E::For { .. }
            | E::InlineIf { .. }
            | E::InlineMatch(_)
            | E::InlineFor { .. }
            | E::ReachabilityType
            | E::TyPlaceholder
            | E::TyNoReturn
            | E::TyVoid
            | E::TyAny
            | E::TermTypedWith
            | E::TraitBound(..)
            | E::LambdaBound
            | E::TermTraitBound
            | E::Implication
            | E::Subtype
            | E::Forall
            | E::Exist
            | E::Builtin(_)
            | E::TakeOld(_)
            | E::TakeBitVec(_)
            | E::TakeTagName(_)
            | E::TakeAttribute(_)
            | E::TakeIntrinsic(_)
            | E::TakeTupleEnumerate(_)
            | E::TakeTupleIndexed(_)
            | E::TypeOf(_)
            | E::MultiDeref(_)
            | E::Invalid => Ok(()),
        }
    }

    struct DraftBuilder<'a, 'h> {
        arena: &'h HirArena,
        draft: &'a mut function_templates::ClonedMember<'h>,
        owner: OwnerId,
        next_local: u32,
        span: rustc_span::Span,
        fuel: &'a mut usize,
    }

    impl<'a, 'h> DraftBuilder<'a, 'h> {
        fn new(
            arena: &'h HirArena,
            draft: &'a mut function_templates::ClonedMember<'h>,
            span: rustc_span::Span,
            fuel: &'a mut usize,
        ) -> Self {
            let owner = draft.item.owner_id;
            let max_binding = draft
                .bindings
                .values()
                .filter(|id| id.owner == owner)
                .map(|id| id.local_id.raw())
                .max()
                .unwrap_or(0);
            let mut next_local = max_binding + 16;
            while draft.nodes.get(ItemLocalId::new(next_local)).is_some() {
                next_local += 1;
            }
            Self {
                arena,
                draft,
                owner,
                next_local,
                span,
                fuel,
            }
        }

        fn alloc_id(&mut self) -> Result<HirId, Error> {
            charge(self.fuel, self.span)?;
            let id = HirId::new(self.owner, ItemLocalId::new(self.next_local));
            self.next_local += 1;
            Ok(id)
        }

        fn expr(&mut self, kind: E<'h>, span: rustc_span::Span) -> Result<&'h Expr<'h>, Error> {
            let hir_id = self.alloc_id()?;
            let allocated = self.arena.alloc_expr(Expr { hir_id, kind, span });
            self.draft
                .nodes
                .insert(hir_id.local_id, ItemLocalId::new(0), Node::Expr(allocated));
            Ok(allocated)
        }

        fn clone_expr(&mut self, expr: &'h Expr<'h>) -> Result<&'h Expr<'h>, Error> {
            charge(self.fuel, expr.span)?;
            let kind = match &expr.kind {
                E::ResolvedDef {
                    name,
                    has_clause_param,
                    def_id,
                } => E::ResolvedDef {
                    name: *name,
                    has_clause_param: *has_clause_param,
                    def_id: *def_id,
                },
                E::ResolvedClauseParam(name, def_id) => E::ResolvedClauseParam(*name, *def_id),
                E::NFApplication(head, args) => {
                    let head = self.clone_expr(head)?;
                    let args = self.clone_args(args)?;
                    E::NFApplication(head, args)
                }
                E::Application(head, args) => {
                    let head = self.clone_expr(head)?;
                    let args = self.clone_args(args)?;
                    E::Application(head, args)
                }
                E::Projection(base, field) => E::Projection(self.clone_expr(base)?, *field),
                E::TySelf => E::TySelf,
                E::DeliveredType(term) => E::DeliveredType(term.clone()),
                other => other.clone(),
            };
            self.expr(kind, expr.span)
        }

        fn clone_args(&mut self, args: &'h [Arg<'h>]) -> Result<&'h [Arg<'h>], Error> {
            let mut cloned = Vec::with_capacity(args.len());
            for arg in args {
                cloned.push(match arg {
                    Arg::Positional(e) => Arg::Positional(self.clone_expr(e)?),
                    Arg::Named(ident, e) => Arg::Named(*ident, self.clone_expr(e)?),
                    Arg::Expand(e) => Arg::Expand(self.clone_expr(e)?),
                    Arg::Implicit(e) => Arg::Implicit(self.clone_expr(e)?),
                    Arg::PostLambda(e) => Arg::PostLambda(self.clone_expr(e)?),
                });
            }
            Ok(self.arena.alloc_arg_slice(cloned))
        }

        fn pat(
            &mut self,
            kind: P<'h>,
            span: rustc_span::Span,
        ) -> Result<(Pattern<'h>, &'h Pattern<'h>), Error> {
            let hir_id = self.alloc_id()?;
            let pat = Pattern { hir_id, kind, span };
            let pat_ref = self.arena.alloc_pattern(pat.clone());
            self.draft
                .nodes
                .insert(hir_id.local_id, ItemLocalId::new(0), Node::Pattern(pat_ref));
            Ok((pat, pat_ref))
        }
    }

    fn rewrite_splice_fn_body<'h>(
        source: &Package<'h>,
        arena: &'h HirArena,
        fn_draft: &mut function_templates::ClonedMember<'h>,
        self_ty_expr: &'h Expr<'h>,
        subject_owner: OwnerId,
        span: rustc_span::Span,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        let ItemKind::Fn(sig, body_id) = &fn_draft.item.kind else {
            return Ok(());
        };
        let body_id = *body_id;
        let Some(data_param) = sig.params.first() else {
            return Ok(());
        };
        let data_param_name = data_param.name.name;
        let data_param_hir_id = data_param.hir_id;
        let data_span = data_param.span;
        let subject_item = source
            .item(subject_owner)
            .ok_or_else(|| error(span, "derive(Splice) subject item missing"))?;
        let mut builder = DraftBuilder::new(arena, fn_draft, span, fuel);
        let new_body_expr = match &subject_item.kind {
            ItemKind::Struct(def) => {
                let mut args = Vec::with_capacity(def.fields.len());
                for field in def.fields {
                    let data_ref = builder.expr(
                        E::ResolvedFnParam(data_param_name, data_param_hir_id),
                        field.span,
                    )?;
                    let field_val =
                        builder.expr(E::Projection(data_ref, field.ident), field.span)?;
                    args.push(Arg::Named(field.ident, field_val));
                }
                let callee = builder.clone_expr(self_ty_expr)?;
                let args = builder.arena.alloc_arg_slice(args);
                builder.expr(E::ExtendedApplication(callee, args), span)?
            }
            ItemKind::Enum(def) => {
                let mut arms = Vec::new();
                collect_enum_arms(
                    &mut builder,
                    self_ty_expr,
                    &[],
                    def.variants,
                    &mut arms,
                    span,
                )?;
                let data_ref = builder.expr(
                    E::ResolvedFnParam(data_param_name, data_param_hir_id),
                    data_span,
                )?;
                let arms = builder.arena.alloc_arm_slice(arms);
                for arm in arms {
                    builder.draft.nodes.insert(
                        arm.hir_id.local_id,
                        ItemLocalId::new(0),
                        Node::PatternArm(arm),
                    );
                }
                let match_expr = builder.expr(E::Match(data_ref, arms), span)?;
                match_expr
            }
            _ => {
                return Err(error(
                    span,
                    "derive(Splice) requires a struct or enum target type",
                ));
            }
        };
        if let Some((_, body)) = fn_draft.bodies.iter_mut().find(|(id, _)| *id == body_id) {
            body.value = new_body_expr;
        }
        Ok(())
    }

    fn projected_variant<'h>(
        builder: &mut DraftBuilder<'_, 'h>,
        self_ty_expr: &'h Expr<'h>,
        prefix: &[Ident],
        variant_ident: Ident,
    ) -> Result<&'h Expr<'h>, Error> {
        let mut expr = builder.clone_expr(self_ty_expr)?;
        for segment in prefix {
            expr = builder.expr(E::Projection(expr, *segment), segment.span)?;
        }
        builder.expr(E::Projection(expr, variant_ident), variant_ident.span)
    }

    fn collect_enum_arms<'h>(
        builder: &mut DraftBuilder<'_, 'h>,
        self_ty_expr: &'h Expr<'h>,
        prefix: &[Ident],
        variants: &'h [Variant<'h>],
        arms: &mut Vec<PatternArm<'h>>,
        span: rustc_span::Span,
    ) -> Result<(), Error> {
        for variant in variants {
            charge(builder.fuel, variant.span)?;
            match &variant.kind {
                VariantKind::Unit | VariantKind::Const(_) => {
                    let pat_target =
                        projected_variant(builder, self_ty_expr, prefix, variant.ident)?;
                    let (pat, _) = builder.pat(P::Const(pat_target), variant.span)?;
                    let body = projected_variant(builder, self_ty_expr, prefix, variant.ident)?;
                    let arm_id = builder.alloc_id()?;
                    arms.push(PatternArm {
                        hir_id: arm_id,
                        pat,
                        guard: None,
                        body,
                        span: variant.span,
                    });
                }
                VariantKind::Tuple(fields) => {
                    let callee_pat_expr =
                        projected_variant(builder, self_ty_expr, prefix, variant.ident)?;
                    let (_, callee_pat) = builder.pat(P::Const(callee_pat_expr), variant.span)?;
                    let mut sub_pats = Vec::with_capacity(fields.len());
                    let mut call_args = Vec::with_capacity(fields.len());
                    for (idx, field_ty) in fields.iter().enumerate() {
                        let name = hir::Symbol::intern(&format!("__splice_v{idx}"));
                        let ident = Ident::new(name, field_ty.span);
                        let (bind_pat, _) = builder
                            .pat(P::Binding(BindingMode::ByValue, ident, None), field_ty.span)?;
                        let arg_ref =
                            builder.expr(E::ResolvedLocal(name, bind_pat.hir_id), field_ty.span)?;
                        sub_pats.push(bind_pat);
                        call_args.push(Arg::Positional(arg_ref));
                    }
                    let sub_pats = builder.arena.alloc_pattern_slice(sub_pats);
                    let (pat, _) = builder.pat(P::AppTuple(callee_pat, sub_pats), variant.span)?;
                    let ctor = projected_variant(builder, self_ty_expr, prefix, variant.ident)?;
                    let call_args = builder.arena.alloc_arg_slice(call_args);
                    let body = builder.expr(E::Application(ctor, call_args), variant.span)?;
                    let arm_id = builder.alloc_id()?;
                    arms.push(PatternArm {
                        hir_id: arm_id,
                        pat,
                        guard: None,
                        body,
                        span: variant.span,
                    });
                }
                VariantKind::Struct(fields) => {
                    let callee_pat_expr =
                        projected_variant(builder, self_ty_expr, prefix, variant.ident)?;
                    let (_, callee_pat) = builder.pat(P::Const(callee_pat_expr), variant.span)?;
                    let mut field_pats = Vec::with_capacity(fields.len());
                    let mut call_args = Vec::with_capacity(fields.len());
                    for (idx, field) in fields.iter().enumerate() {
                        let local_name = hir::Symbol::intern(&format!("__splice_f{idx}"));
                        let local_ident = Ident::new(local_name, field.span);
                        let (bind_pat, _) = builder.pat(
                            P::Binding(BindingMode::ByValue, local_ident, None),
                            field.span,
                        )?;
                        let arg_ref = builder
                            .expr(E::ResolvedLocal(local_name, bind_pat.hir_id), field.span)?;
                        field_pats.push(FieldPat {
                            ident: field.ident,
                            pat: bind_pat,
                            span: field.span,
                        });
                        call_args.push(Arg::Named(field.ident, arg_ref));
                    }
                    let field_pats = builder.arena.alloc_field_pat_slice(field_pats);
                    let (pat, _) =
                        builder.pat(P::Struct(callee_pat, field_pats, false), variant.span)?;
                    let ctor = projected_variant(builder, self_ty_expr, prefix, variant.ident)?;
                    let call_args = builder.arena.alloc_arg_slice(call_args);
                    let body =
                        builder.expr(E::ExtendedApplication(ctor, call_args), variant.span)?;
                    let arm_id = builder.alloc_id()?;
                    arms.push(PatternArm {
                        hir_id: arm_id,
                        pat,
                        guard: None,
                        body,
                        span: variant.span,
                    });
                }
                VariantKind::SubEnum(nested) => {
                    let mut next_prefix = prefix.to_vec();
                    next_prefix.push(variant.ident);
                    collect_enum_arms(builder, self_ty_expr, &next_prefix, nested, arms, span)?;
                }
                VariantKind::Pattern(_) => {
                    return Err(error(
                        variant.span,
                        "cannot auto-derive Splice for pattern enum variant",
                    ));
                }
            }
        }
        Ok(())
    }
}
