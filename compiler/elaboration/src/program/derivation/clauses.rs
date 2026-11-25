//! Strategy `where`-clause instantiation, HIR substitution, and block-local rewiring.
use super::*;
use hir::clause::{ClauseConstraint, ClauseConstraintKind as C, ClauseParam, ClauseParamKind as K};
use hir::common::{Arg, DefId, Ident, TyParam, TyParamKind as T};
use hir::pattern::{BitSegment, Pattern, PatternArm, PatternKind as P};
use hir::{Block, Body, BodyId, Expr, HirId, ItemLocalId};

const MAX_HIR_DEPTH: usize = 128;

pub(super) struct ResolvedStrategyClauses<'h> {
    pub bound: BTreeMap<DefId, &'h Expr<'h>>,
    pub inferred: Vec<&'h ClauseParam<'h>>,
}

fn check_depth(depth: usize, span: rustc_span::Span) -> Result<(), Error> {
    if depth > MAX_HIR_DEPTH {
        return Err(error(
            span,
            "derivation clause substitution depth exhausted",
        ));
    }
    Ok(())
}

/// Resolve strategy `where` clause parameters from explicit strategy clause
/// arguments, structural matching of `strategy_trait_ref` against
/// `request_trait_ref`, and optional clause defaults.
pub(super) fn resolve_strategy_clauses<'h>(
    generator: &hir::item::DeriverDef<'h>,
    request: &hir::item::DeriveRequest<'h>,
    clause_arguments: &[Arg<'h>],
    selection_span: rustc_span::Span,
    fuel: &mut usize,
) -> Result<ResolvedStrategyClauses<'h>, Error> {
    if generator.clause_params.is_empty() {
        if !clause_arguments.is_empty() {
            return Err(error(
                selection_span,
                "derivation strategy does not accept clause arguments",
            ));
        }
        return Ok(ResolvedStrategyClauses {
            bound: BTreeMap::new(),
            inferred: Vec::new(),
        });
    }

    let mut strategy_defs = BTreeSet::new();
    let mut param_order = Vec::new();
    let mut named_defs = BTreeMap::new();
    for parameter in generator.clause_params {
        charge(fuel, parameter.span)?;
        let def_id = parameter.definition.ok_or_else(|| {
            error(
                parameter.span,
                "derivation strategy clause parameter has no identity",
            )
        })?;
        strategy_defs.insert(def_id);
        named_defs.insert(parameter.name.name.as_str().to_owned(), def_id);
        let is_optional = matches!(parameter.kind.unquoted(), K::Optional(..));
        param_order.push((def_id, parameter.name, is_optional));
    }

    let mut bound: BTreeMap<DefId, &'h Expr<'h>> = BTreeMap::new();

    // 1. Explicit strategy clause arguments: `strategy<...>`
    let mut positional_cursor = 0usize;
    for arg in clause_arguments {
        charge(fuel, selection_span)?;
        match arg {
            Arg::Positional(expr) => {
                while positional_cursor < param_order.len() && param_order[positional_cursor].2 {
                    positional_cursor += 1;
                }
                let Some((def_id, name, _)) = param_order.get(positional_cursor).copied() else {
                    return Err(error(
                        expr.span,
                        "too many positional clause arguments for derivation strategy",
                    ));
                };
                positional_cursor += 1;
                if bound.insert(def_id, expr).is_some() {
                    return Err(error(
                        expr.span,
                        format!(
                            "strategy clause parameter `{}` supplied more than once",
                            name.name
                        ),
                    ));
                }
            }
            Arg::Named(ident, expr) => {
                let Some(&def_id) = named_defs.get(ident.name.as_str()) else {
                    return Err(error(
                        ident.span,
                        format!("unknown strategy clause parameter `{}`", ident.name),
                    ));
                };
                if bound.insert(def_id, expr).is_some() {
                    return Err(error(
                        ident.span,
                        format!(
                            "strategy clause parameter `{}` supplied more than once",
                            ident.name
                        ),
                    ));
                }
            }
            Arg::Expand(expr) | Arg::Implicit(expr) | Arg::PostLambda(expr) => {
                return Err(unsupported(expr.span, "expanded strategy clause arguments"));
            }
        }
    }

    // 2. Match `generator.trait_ref` against `request.trait_ref` (including
    // unordered error label chains and named trait clause arguments).
    match_hir_expr(
        generator.trait_ref,
        request.trait_ref,
        &strategy_defs,
        &mut bound,
        fuel,
        0,
    )?;

    // 3. Apply defaults for omitted optional strategy clause parameters, and
    // classify any remaining unbound parameters as parameter-inferred or missing.
    let mut inferred = Vec::new();
    for parameter in generator.clause_params {
        charge(fuel, parameter.span)?;
        let def_id = parameter.definition.unwrap();
        if bound.contains_key(&def_id) {
            continue;
        }
        if let K::Optional(_, _, default_expr) = *parameter.kind.unquoted() {
            bound.insert(def_id, default_expr);
            continue;
        }
        if !parameter.kind.is_quote()
            && request.clause_params.is_empty()
            && parameters_mention_clause(generator.parameters, def_id, fuel)?
        {
            inferred.push(parameter);
            continue;
        }
        return Err(error(
            selection_span,
            format!(
                "could not determine derivation strategy clause parameter `{}`",
                parameter.name.name
            ),
        ));
    }

    Ok(ResolvedStrategyClauses { bound, inferred })
}

fn parameters_mention_clause<'h>(
    parameters: &[hir::common::FnParam<'h>],
    target: DefId,
    fuel: &mut usize,
) -> Result<bool, Error> {
    for parameter in parameters {
        charge(fuel, parameter.span)?;
        match parameter.ty.kind {
            T::Positional(ty) | T::PositionalDependencyCatched(_, ty) | T::Varadic(_, ty) => {
                if expr_mentions_clause(ty, target, fuel, 0)? {
                    return Ok(true);
                }
            }
            T::Optional(_, ty, default) => {
                if expr_mentions_clause(ty, target, fuel, 0)?
                    || expr_mentions_clause(default, target, fuel, 0)?
                {
                    return Ok(true);
                }
            }
            T::Itself { .. } | T::Case(_) => {}
        }
    }
    Ok(false)
}

fn expr_mentions_clause<'h>(
    expr: &'h Expr<'h>,
    target: DefId,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, Error> {
    check_depth(depth, expr.span)?;
    charge(fuel, expr.span)?;
    let d = depth + 1;
    match &expr.kind {
        E::ResolvedClauseParam(_, id) => Ok(*id == target),
        E::Application(head, args)
        | E::ExtendedApplication(head, args)
        | E::NFApplication(head, args) => {
            if expr_mentions_clause(head, target, fuel, d)? {
                return Ok(true);
            }
            for arg in *args {
                let inner = match arg {
                    Arg::Positional(e)
                    | Arg::Named(_, e)
                    | Arg::Expand(e)
                    | Arg::Implicit(e)
                    | Arg::PostLambda(e) => e,
                };
                if expr_mentions_clause(inner, target, fuel, d)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        E::TyPtr(inner)
        | E::TyOptional(inner)
        | E::TyTraitObject(inner)
        | E::LiftType(inner)
        | E::Ref(inner)
        | E::Deref(inner)
        | E::Unary(_, inner)
        | E::Projection(inner, _)
        | E::TypeOf(inner)
        | E::Defer(inner) => expr_mentions_clause(inner, target, fuel, d),
        E::Binary(_, a, b)
        | E::TyFnArrow(a, b)
        | E::ErrorQualifiedType(a, b)
        | E::EffectQualifiedType(a, b)
        | E::TraitBound(a, b)
        | E::Cast(a, b)
        | E::DynCast(a, b)
        | E::Index(a, b)
        | E::TyClosure(a, b) => {
            Ok(expr_mentions_clause(a, target, fuel, d)?
                || expr_mentions_clause(b, target, fuel, d)?)
        }
        E::Tuple(items) | E::List(items) => {
            for item in *items {
                if expr_mentions_clause(item, target, fuel, d)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        E::TyFn(params)
        | E::TyComptimeFn(params)
        | E::TyModifiedFn(_, params)
        | E::TyNFFn(params) => {
            for param in *params {
                match param.kind {
                    T::Positional(e) | T::PositionalDependencyCatched(_, e) | T::Varadic(_, e) => {
                        if expr_mentions_clause(e, target, fuel, d)? {
                            return Ok(true);
                        }
                    }
                    T::Optional(_, ty, def) => {
                        if expr_mentions_clause(ty, target, fuel, d)?
                            || expr_mentions_clause(def, target, fuel, d)?
                        {
                            return Ok(true);
                        }
                    }
                    T::Itself { .. } | T::Case(_) => {}
                }
            }
            Ok(false)
        }
        E::RefinedType { requirement, base } => {
            Ok(expr_mentions_clause(requirement, target, fuel, d)?
                || expr_mentions_clause(base, target, fuel, d)?)
        }
        _ => Ok(false),
    }
}

fn flatten_error_chain<'h>(
    mut expr: &'h Expr<'h>,
    fuel: &mut usize,
) -> Result<(Vec<&'h Expr<'h>>, &'h Expr<'h>), Error> {
    let mut labels = Vec::new();
    let mut depth = 0usize;
    while let E::ErrorQualifiedType(label, rest) = expr.kind {
        check_depth(depth, expr.span)?;
        charge(fuel, expr.span)?;
        labels.push(label);
        expr = rest;
        depth += 1;
    }
    Ok((labels, expr))
}

fn flatten_effect_chain<'h>(
    mut expr: &'h Expr<'h>,
    fuel: &mut usize,
) -> Result<(Vec<&'h Expr<'h>>, &'h Expr<'h>), Error> {
    let mut effects = Vec::new();
    let mut depth = 0usize;
    while let E::EffectQualifiedType(effect, rest) = expr.kind {
        check_depth(depth, expr.span)?;
        charge(fuel, expr.span)?;
        effects.push(effect);
        expr = rest;
        depth += 1;
    }
    Ok((effects, expr))
}

fn hir_expr_head_def(expr: &Expr<'_>) -> Option<DefId> {
    match expr.kind {
        E::ResolvedDef { def_id, .. } => Some(def_id),
        E::NFApplication(head, _) | E::Application(head, _) => hir_expr_head_def(head),
        _ => None,
    }
}

fn hir_expr_has_strategy_param<'h>(
    expr: &'h Expr<'h>,
    strategy_defs: &BTreeSet<DefId>,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, Error> {
    for &def in strategy_defs {
        if expr_mentions_clause(expr, def, fuel, depth)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn hir_expr_syntactically_equal(
    left: &Expr<'_>,
    right: &Expr<'_>,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, Error> {
    check_depth(depth, left.span)?;
    charge(fuel, left.span)?;
    let d = depth + 1;
    match (&left.kind, &right.kind) {
        (E::ResolvedDef { def_id: a, .. }, E::ResolvedDef { def_id: b, .. }) => Ok(a == b),
        (E::ResolvedClauseParam(_, a), E::ResolvedClauseParam(_, b)) => Ok(a == b),
        (E::Builtin(a), E::Builtin(b)) => Ok(a == b),
        (E::Lit(a), E::Lit(b)) => Ok(a == b),
        (E::TyType, E::TyType)
        | (E::TyVoid, E::TyVoid)
        | (E::TyNoReturn, E::TyNoReturn)
        | (E::TySelf, E::TySelf)
        | (E::Unit, E::Unit) => Ok(true),
        (E::TyPtr(a), E::TyPtr(b))
        | (E::TyOptional(a), E::TyOptional(b))
        | (E::TyTraitObject(a), E::TyTraitObject(b))
        | (E::LiftType(a), E::LiftType(b)) => hir_expr_syntactically_equal(a, b, fuel, d),
        (E::NFApplication(ah, aa), E::NFApplication(bh, ba)) => {
            if aa.len() != ba.len() || !hir_expr_syntactically_equal(ah, bh, fuel, d)? {
                return Ok(false);
            }
            for (la, ra) in aa.iter().zip(*ba) {
                match (la, ra) {
                    (Arg::Positional(a), Arg::Positional(b)) => {
                        if !hir_expr_syntactically_equal(a, b, fuel, d)? {
                            return Ok(false);
                        }
                    }
                    (Arg::Named(na, a), Arg::Named(nb, b)) if na.name == nb.name => {
                        if !hir_expr_syntactically_equal(a, b, fuel, d)? {
                            return Ok(false);
                        }
                    }
                    _ => return Ok(false),
                }
            }
            Ok(true)
        }
        (E::Tuple(a), E::Tuple(b)) | (E::List(a), E::List(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (la, ra) in a.iter().zip(*b) {
                if !hir_expr_syntactically_equal(la, ra, fuel, d)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn match_hir_expr<'h>(
    pattern: &'h Expr<'h>,
    actual: &'h Expr<'h>,
    strategy_defs: &BTreeSet<DefId>,
    bound: &mut BTreeMap<DefId, &'h Expr<'h>>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), Error> {
    check_depth(depth, pattern.span)?;
    charge(fuel, pattern.span)?;
    if let E::ResolvedClauseParam(_, def_id) = pattern.kind
        && strategy_defs.contains(&def_id)
    {
        bound.entry(def_id).or_insert(actual);
        return Ok(());
    }
    let d = depth + 1;
    match (&pattern.kind, &actual.kind) {
        (E::NFApplication(p_head, p_args), E::NFApplication(a_head, a_args))
        | (E::Application(p_head, p_args), E::Application(a_head, a_args)) => {
            match_hir_expr(p_head, a_head, strategy_defs, bound, fuel, d)?;
            let mut a_pos = a_args.iter().filter_map(|arg| match arg {
                Arg::Positional(expr) => Some(*expr),
                _ => None,
            });
            for p_arg in *p_args {
                charge(fuel, pattern.span)?;
                match p_arg {
                    Arg::Positional(p_expr) => {
                        if let Some(a_expr) = a_pos.next() {
                            match_hir_expr(p_expr, a_expr, strategy_defs, bound, fuel, d)?;
                        }
                    }
                    Arg::Named(p_name, p_expr) => {
                        if let Some(a_expr) = a_args.iter().find_map(|arg| match arg {
                            Arg::Named(a_name, expr) if a_name.name == p_name.name => Some(*expr),
                            _ => None,
                        }) {
                            match_hir_expr(p_expr, a_expr, strategy_defs, bound, fuel, d)?;
                        }
                    }
                    Arg::Expand(p_expr) | Arg::Implicit(p_expr) | Arg::PostLambda(p_expr) => {
                        if let Some(a_expr) = a_pos.next() {
                            match_hir_expr(p_expr, a_expr, strategy_defs, bound, fuel, d)?;
                        }
                    }
                }
            }
        }
        (E::ErrorQualifiedType(..), E::ErrorQualifiedType(..)) => {
            let (p_labels, p_tail) = flatten_error_chain(pattern, fuel)?;
            let (a_labels, a_tail) = flatten_error_chain(actual, fuel)?;
            match_hir_expr(p_tail, a_tail, strategy_defs, bound, fuel, d)?;
            let mut used = vec![false; a_labels.len()];
            let mut unmatched_pats = Vec::new();
            // Match exact concrete labels first so unordered sets like
            // `ErrA ! E ! bool` vs `ErrB ! ErrA ! bool` bind `E -> ErrB`.
            for &p_label in &p_labels {
                if !hir_expr_has_strategy_param(p_label, strategy_defs, fuel, d)? {
                    if let Some((idx, _)) = a_labels.iter().enumerate().find(|(idx, a_label)| {
                        !used[*idx]
                            && hir_expr_syntactically_equal(p_label, a_label, fuel, d)
                                .unwrap_or(false)
                    }) {
                        used[idx] = true;
                    }
                } else {
                    unmatched_pats.push(p_label);
                }
            }
            // Match structured parameter-bearing labels by head before bare params.
            let mut bare_pats = Vec::new();
            for p_label in unmatched_pats {
                if matches!(p_label.kind, E::ResolvedClauseParam(..)) {
                    bare_pats.push(p_label);
                    continue;
                }
                let target_head = hir_expr_head_def(p_label);
                if let Some((idx, a_label)) = a_labels
                    .iter()
                    .enumerate()
                    .find(|(idx, a_label)| !used[*idx] && hir_expr_head_def(a_label) == target_head)
                {
                    used[idx] = true;
                    match_hir_expr(p_label, a_label, strategy_defs, bound, fuel, d)?;
                }
            }
            for p_label in bare_pats {
                if let Some((idx, a_label)) =
                    a_labels.iter().enumerate().find(|(idx, _)| !used[*idx])
                {
                    used[idx] = true;
                    match_hir_expr(p_label, a_label, strategy_defs, bound, fuel, d)?;
                } else if let Some(a_label) = a_labels.first() {
                    // Idempotent error label collapse (e.g. `E1 ! E2 ! bool` vs `ErrA ! bool`).
                    match_hir_expr(p_label, a_label, strategy_defs, bound, fuel, d)?;
                }
            }
        }
        (E::EffectQualifiedType(..), E::EffectQualifiedType(..)) => {
            let (p_effects, p_tail) = flatten_effect_chain(pattern, fuel)?;
            let (a_effects, a_tail) = flatten_effect_chain(actual, fuel)?;
            match_hir_expr(p_tail, a_tail, strategy_defs, bound, fuel, d)?;
            let mut used = vec![false; a_effects.len()];
            for p_eff in p_effects {
                let target_head = hir_expr_head_def(p_eff);
                if let Some((idx, a_eff)) = a_effects
                    .iter()
                    .enumerate()
                    .find(|(idx, a_eff)| !used[*idx] && hir_expr_head_def(a_eff) == target_head)
                {
                    used[idx] = true;
                    match_hir_expr(p_eff, a_eff, strategy_defs, bound, fuel, d)?;
                }
            }
        }
        (E::TyPtr(a), E::TyPtr(b))
        | (E::TyOptional(a), E::TyOptional(b))
        | (E::TyTraitObject(a), E::TyTraitObject(b))
        | (E::LiftType(a), E::LiftType(b))
        | (E::Ref(a), E::Ref(b))
        | (E::Deref(a), E::Deref(b))
        | (E::Unary(_, a), E::Unary(_, b))
        | (E::Projection(a, _), E::Projection(b, _)) => {
            match_hir_expr(a, b, strategy_defs, bound, fuel, d)?;
        }
        (E::Binary(_, a1, a2), E::Binary(_, b1, b2))
        | (E::TyFnArrow(a1, a2), E::TyFnArrow(b1, b2))
        | (E::TyClosure(a1, a2), E::TyClosure(b1, b2))
        | (E::Cast(a1, a2), E::Cast(b1, b2))
        | (E::DynCast(a1, a2), E::DynCast(b1, b2))
        | (E::Index(a1, a2), E::Index(b1, b2)) => {
            match_hir_expr(a1, b1, strategy_defs, bound, fuel, d)?;
            match_hir_expr(a2, b2, strategy_defs, bound, fuel, d)?;
        }
        (E::Tuple(a), E::Tuple(b)) | (E::List(a), E::List(b)) => {
            for (pa, aa) in a.iter().zip(*b) {
                match_hir_expr(pa, aa, strategy_defs, bound, fuel, d)?;
            }
        }
        (E::TyFn(a), E::TyFn(b))
        | (E::TyComptimeFn(a), E::TyComptimeFn(b))
        | (E::TyNFFn(a), E::TyNFFn(b))
        | (E::TyModifiedFn(_, a), E::TyModifiedFn(_, b)) => {
            for (pa, aa) in a.iter().zip(*b) {
                if let (
                    T::Positional(pe) | T::Optional(_, pe, _) | T::Varadic(_, pe),
                    T::Positional(ae) | T::Optional(_, ae, _) | T::Varadic(_, ae),
                ) = (&pa.kind, &aa.kind)
                {
                    match_hir_expr(pe, ae, strategy_defs, bound, fuel, d)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Clone and substitute strategy clause placeholders (`temp_to_expr`) inside an
/// unpublished `ClonedMember` draft, preserving fresh `HirId`s and resolver evidence.
pub(super) struct DraftSubst<'s, 'h> {
    source: &'s Package<'h>,
    arena: &'h HirArena,
    owner: OwnerId,
    next_local: u32,
    temp_to_expr: &'s BTreeMap<DefId, &'h Expr<'h>>,
    strategy_to_temp: &'s BTreeMap<DefId, DefId>,
    request_mapping: &'s BTreeMap<DefId, DefId>,
    self_replacement: Option<&'h Expr<'h>>,
    owner_replacements: &'s BTreeMap<OwnerId, OwnerId>,
    resolution: Vec<(HirId, hir::definitions::ResolutionEvidence)>,
    bodies: Vec<(BodyId, Body<'h>)>,
    body_map: BTreeMap<HirId, Body<'h>>,
    fuel: usize,
}

impl<'s, 'h> DraftSubst<'s, 'h> {
    fn alloc_id(&mut self, span: rustc_span::Span) -> Result<HirId, Error> {
        let local = self.next_local;
        self.next_local = self
            .next_local
            .checked_add(1)
            .ok_or_else(|| error(span, "derivation HirId space exhausted"))?;
        Ok(HirId::new(self.owner, ItemLocalId::new(local)))
    }

    fn record_source_resolution(&mut self, old: HirId, new: HirId) {
        let map_def = |id: &DefId| self.request_mapping.get(id).copied().unwrap_or(*id);
        let projection = self.source.member_projections.get(&old).map(map_def);
        let map_cands = |src: Option<&std::sync::Arc<[(DefId, DefId)]>>| {
            src.map(|items| {
                items
                    .iter()
                    .map(|(a, b)| (map_def(a), map_def(b)))
                    .collect::<std::sync::Arc<[(DefId, DefId)]>>()
            })
        };
        let deferred = map_cands(self.source.deferred_member_projections.get(&old));
        let access = map_cands(self.source.member_access.get(&old));
        let trait_sources = self.source.trait_sources.get(&old).map(|plan| {
            let mut cloned = (**plan).clone();
            for name in &mut cloned.names {
                for id in name.ambiguous_traits.iter_mut().flatten() {
                    *id = map_def(id);
                }
                if let Ok(Some(id)) = &mut name.fallback {
                    *id = map_def(id);
                }
            }
            std::sync::Arc::new(cloned)
        });
        let source_origin = self.source.source_origin(old);
        if source_origin.is_some()
            || projection.is_some()
            || deferred.is_some()
            || access.is_some()
            || trait_sources.is_some()
        {
            self.resolution.push((
                new,
                hir::definitions::ResolutionEvidence {
                    source_origin,
                    trait_sources,
                    projection,
                    deferred,
                    access,
                },
            ));
        }
    }

    fn subst_source_expr(
        &mut self,
        expr: &'h Expr<'h>,
        depth: usize,
    ) -> Result<&'h Expr<'h>, Error> {
        check_depth(depth, expr.span)?;
        charge(&mut self.fuel, expr.span)?;
        let d = depth + 1;
        if let E::ResolvedClauseParam(_, id) = expr.kind {
            if let Some(&temp_id) = self.strategy_to_temp.get(&id)
                && let Some(&replacement) = self.temp_to_expr.get(&temp_id)
            {
                return self.subst_source_expr(replacement, d);
            }
            if let Some(&replacement) = self.temp_to_expr.get(&id) {
                return self.subst_source_expr(replacement, d);
            }
        }
        if matches!(expr.kind, E::TySelf)
            && let Some(self_expr) = self.self_replacement
        {
            return self.subst_source_expr(self_expr, d);
        }
        let kind = match &expr.kind {
            E::ResolvedClauseParam(name, id) => {
                let mapped = self.request_mapping.get(id).copied().unwrap_or(*id);
                E::ResolvedClauseParam(*name, mapped)
            }
            E::ResolvedDef {
                name,
                def_id,
                has_clause_param,
            } => E::ResolvedDef {
                name: *name,
                def_id: self.request_mapping.get(def_id).copied().unwrap_or(*def_id),
                has_clause_param: *has_clause_param,
            },
            E::Item(owner) => E::Item(
                self.owner_replacements
                    .get(owner)
                    .copied()
                    .unwrap_or(*owner),
            ),
            E::Application(head, args)
            | E::ExtendedApplication(head, args)
            | E::NFApplication(head, args) => {
                let head = self.subst_source_expr(head, d)?;
                let mut new_args = Vec::with_capacity(args.len());
                for arg in *args {
                    new_args.push(match arg {
                        Arg::Positional(e) => Arg::Positional(self.subst_source_expr(e, d)?),
                        Arg::Named(n, e) => Arg::Named(*n, self.subst_source_expr(e, d)?),
                        Arg::Expand(e) => Arg::Expand(self.subst_source_expr(e, d)?),
                        Arg::Implicit(e) => Arg::Implicit(self.subst_source_expr(e, d)?),
                        Arg::PostLambda(e) => Arg::PostLambda(self.subst_source_expr(e, d)?),
                    });
                }
                let new_args = self.arena.alloc_arg_slice(new_args);
                match expr.kind {
                    E::Application(..) => E::Application(head, new_args),
                    E::ExtendedApplication(..) => E::ExtendedApplication(head, new_args),
                    _ => E::NFApplication(head, new_args),
                }
            }
            E::Binary(op, a, b) => E::Binary(
                *op,
                self.subst_source_expr(a, d)?,
                self.subst_source_expr(b, d)?,
            ),
            E::Unary(op, a) => E::Unary(*op, self.subst_source_expr(a, d)?),
            E::Projection(a, f) => E::Projection(self.subst_source_expr(a, d)?, *f),
            E::TyPtr(a) => E::TyPtr(self.subst_source_expr(a, d)?),
            E::TyOptional(a) => E::TyOptional(self.subst_source_expr(a, d)?),
            E::TyTraitObject(a) => E::TyTraitObject(self.subst_source_expr(a, d)?),
            E::LiftType(a) => E::LiftType(self.subst_source_expr(a, d)?),
            E::Ref(a) => E::Ref(self.subst_source_expr(a, d)?),
            E::Deref(a) => E::Deref(self.subst_source_expr(a, d)?),
            E::MultiDeref(a) => E::MultiDeref(self.subst_source_expr(a, d)?),
            E::TypeOf(a) => E::TypeOf(self.subst_source_expr(a, d)?),
            E::TyFnArrow(a, b) => {
                E::TyFnArrow(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?)
            }
            E::ErrorQualifiedType(a, b) => {
                E::ErrorQualifiedType(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?)
            }
            E::EffectQualifiedType(a, b) => {
                E::EffectQualifiedType(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?)
            }
            E::TraitBound(a, b) => {
                E::TraitBound(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?)
            }
            E::Cast(a, b) => E::Cast(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?),
            E::DynCast(a, b) => {
                E::DynCast(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?)
            }
            E::Index(a, b) => {
                E::Index(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?)
            }
            E::TyClosure(a, b) => {
                E::TyClosure(self.subst_source_expr(a, d)?, self.subst_source_expr(b, d)?)
            }
            E::Tuple(items) | E::List(items) => {
                let mut cloned = Vec::with_capacity(items.len());
                for item in *items {
                    cloned.push(self.subst_source_expr(item, d)?.clone());
                }
                let slice = self.arena.alloc_expr_slice(cloned);
                if matches!(expr.kind, E::Tuple(_)) {
                    E::Tuple(slice)
                } else {
                    E::List(slice)
                }
            }
            E::TyFn(params)
            | E::TyComptimeFn(params)
            | E::TyModifiedFn(_, params)
            | E::TyNFFn(params) => {
                let mut cloned = Vec::with_capacity(params.len());
                for param in *params {
                    cloned.push(self.subst_ty_param(param, true, d)?);
                }
                let slice = self.arena.alloc_ty_param_slice(cloned);
                match expr.kind {
                    E::TyFn(_) => E::TyFn(slice),
                    E::TyComptimeFn(_) => E::TyComptimeFn(slice),
                    E::TyModifiedFn(m, _) => E::TyModifiedFn(m, slice),
                    _ => E::TyNFFn(slice),
                }
            }
            E::RefinedType { requirement, base } => E::RefinedType {
                requirement: self.subst_source_expr(requirement, d)?,
                base: self.subst_source_expr(base, d)?,
            },
            _ => expr.kind.clone(),
        };
        let hir_id = self.alloc_id(expr.span)?;
        self.record_source_resolution(expr.hir_id, hir_id);
        Ok(self.arena.alloc_expr(Expr {
            hir_id,
            kind,
            span: expr.span,
        }))
    }

    fn rewrite_expr(&mut self, expr: &'h Expr<'h>, depth: usize) -> Result<&'h Expr<'h>, Error> {
        check_depth(depth, expr.span)?;
        charge(&mut self.fuel, expr.span)?;
        let d = depth + 1;
        if let E::ResolvedClauseParam(_, id) = expr.kind
            && let Some(&replacement) = self.temp_to_expr.get(&id)
        {
            return self.subst_source_expr(replacement, d);
        }
        let kind = match &expr.kind {
            E::Item(owner) => E::Item(
                self.owner_replacements
                    .get(owner)
                    .copied()
                    .unwrap_or(*owner),
            ),
            E::Application(head, args)
            | E::ExtendedApplication(head, args)
            | E::NFApplication(head, args) => {
                let head = self.rewrite_expr(head, d)?;
                let mut new_args = Vec::with_capacity(args.len());
                for arg in *args {
                    new_args.push(match arg {
                        Arg::Positional(e) => Arg::Positional(self.rewrite_expr(e, d)?),
                        Arg::Named(n, e) => Arg::Named(*n, self.rewrite_expr(e, d)?),
                        Arg::Expand(e) => Arg::Expand(self.rewrite_expr(e, d)?),
                        Arg::Implicit(e) => Arg::Implicit(self.rewrite_expr(e, d)?),
                        Arg::PostLambda(e) => Arg::PostLambda(self.rewrite_expr(e, d)?),
                    });
                }
                let new_args = self.arena.alloc_arg_slice(new_args);
                match expr.kind {
                    E::Application(..) => E::Application(head, new_args),
                    E::ExtendedApplication(..) => E::ExtendedApplication(head, new_args),
                    _ => E::NFApplication(head, new_args),
                }
            }
            E::Binary(op, a, b) => {
                E::Binary(*op, self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?)
            }
            E::Unary(op, a) => E::Unary(*op, self.rewrite_expr(a, d)?),
            E::If(cond, block, other) => E::If(
                self.rewrite_expr(cond, d)?,
                self.rewrite_block(block, d)?,
                other.map(|e| self.rewrite_expr(e, d)).transpose()?,
            ),
            E::Block(block) => E::Block(self.rewrite_block(block, d)?),
            E::UnsafeBlock(block) => E::UnsafeBlock(self.rewrite_block(block, d)?),
            E::Loop(block) => E::Loop(self.rewrite_block(block, d)?),
            E::Match(val, arms) => {
                E::Match(self.rewrite_expr(val, d)?, self.rewrite_arms(arms, d)?)
            }
            E::EffectHandle(val, arms) => {
                E::EffectHandle(self.rewrite_expr(val, d)?, self.rewrite_arms(arms, d)?)
            }
            E::InlineMatch(arms) => E::InlineMatch(self.rewrite_arms(arms, d)?),
            E::Let(decl) => {
                let mut decl = (*decl).clone();
                decl.pattern = self.rewrite_pattern(&decl.pattern, d)?;
                decl.ty = decl.ty.map(|e| self.rewrite_expr(e, d)).transpose()?;
                decl.init = decl.init.map(|e| self.rewrite_expr(e, d)).transpose()?;
                decl.guard = decl.guard.map(|e| self.rewrite_expr(e, d)).transpose()?;
                decl.otherwise = decl
                    .otherwise
                    .map(|e| self.rewrite_expr(e, d))
                    .transpose()?;
                E::Let(self.arena.alloc_let_decl(decl))
            }
            E::Assign(a, b) => E::Assign(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?),
            E::AssignOp(op, a, b) => {
                E::AssignOp(*op, self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?)
            }
            E::When(arms) => {
                let mut new_arms = Vec::with_capacity(arms.len());
                for arm in *arms {
                    new_arms.push(hir::expr::CondictionArm {
                        hir_id: arm.hir_id,
                        cond: arm.cond.map(|e| self.rewrite_expr(e, d)).transpose()?,
                        body: self.rewrite_expr(arm.body, d)?,
                        span: arm.span,
                    });
                }
                E::When(self.arena.alloc_cond_arm_slice(new_arms))
            }
            E::Closure(params, ret, body_id) => {
                let mut new_params = Vec::with_capacity(params.len());
                for param in *params {
                    new_params.push(hir::expr::ClosureParam {
                        hir_id: param.hir_id,
                        pat: self.rewrite_pattern(&param.pat, d)?,
                        ty: param.ty.map(|e| self.rewrite_expr(e, d)).transpose()?,
                        span: param.span,
                    });
                }
                let ret = ret.map(|e| self.rewrite_expr(e, d)).transpose()?;
                self.rewrite_body_in_map(*body_id, d)?;
                E::Closure(
                    self.arena.alloc_closure_param_slice(new_params),
                    ret,
                    *body_id,
                )
            }
            E::PostLambdaBody(body_id) => {
                self.rewrite_body_in_map(*body_id, d)?;
                E::PostLambdaBody(*body_id)
            }
            E::For {
                label,
                pat,
                guard,
                iter,
                body,
            } => E::For {
                label: *label,
                pat: self.rewrite_pattern_ref(pat, d)?,
                guard: guard.map(|e| self.rewrite_expr(e, d)).transpose()?,
                iter: self.rewrite_expr(iter, d)?,
                body: self.rewrite_block(body, d)?,
            },
            E::InlineFor {
                label,
                pat,
                iter,
                body,
            } => E::InlineFor {
                label: *label,
                pat: self.rewrite_pattern_ref(pat, d)?,
                iter: self.rewrite_expr(iter, d)?,
                body: self.rewrite_expr(body, d)?,
            },
            E::InlineIf {
                cond,
                then_expr,
                else_expr,
            } => E::InlineIf {
                cond: self.rewrite_expr(cond, d)?,
                then_expr: self.rewrite_expr(then_expr, d)?,
                else_expr: else_expr.map(|e| self.rewrite_expr(e, d)).transpose()?,
            },
            E::Semi(e) => E::Semi(self.rewrite_expr(e, d)?),
            E::Assumes(e) => E::Assumes(self.rewrite_expr(e, d)?),
            E::Asserts(e) => E::Asserts(self.rewrite_expr(e, d)?),
            E::Invariant(e) => E::Invariant(self.rewrite_expr(e, d)?),
            E::Decreases(e) => E::Decreases(self.rewrite_expr(e, d)?),
            E::InlineControl(e) => E::InlineControl(self.rewrite_expr(e, d)?),
            E::Defer(e) => E::Defer(self.rewrite_expr(e, d)?),
            E::EffectRequest(e) => E::EffectRequest(self.rewrite_expr(e, d)?),
            E::ErrorNew(e) => E::ErrorNew(self.rewrite_expr(e, d)?),
            E::ErrorPropagate(e) => E::ErrorPropagate(self.rewrite_expr(e, d)?),
            E::OptionPropagate(e) => E::OptionPropagate(self.rewrite_expr(e, d)?),
            E::Resume(e) => E::Resume(e.map(|e| self.rewrite_expr(e, d)).transpose()?),
            E::Return(e) => E::Return(e.map(|e| self.rewrite_expr(e, d)).transpose()?),
            E::ErrorHandle {
                value,
                before,
                arms,
                after,
            } => E::ErrorHandle {
                value: self.rewrite_expr(value, d)?,
                before: before
                    .map(|arm| Ok::<_, Error>(self.arena.alloc_arm(self.rewrite_arm(arm, d)?)))
                    .transpose()?,
                arms: self.rewrite_arms(arms, d)?,
                after: after
                    .map(|arm| Ok::<_, Error>(self.arena.alloc_arm(self.rewrite_arm(arm, d)?)))
                    .transpose()?,
            },
            E::ConstantObjectEntry(v, idx) => {
                E::ConstantObjectEntry(self.rewrite_expr(v, d)?, *idx)
            }
            E::Projection(a, f) => E::Projection(self.rewrite_expr(a, d)?, *f),
            E::TyPtr(a) => E::TyPtr(self.rewrite_expr(a, d)?),
            E::TyOptional(a) => E::TyOptional(self.rewrite_expr(a, d)?),
            E::TyTraitObject(a) => E::TyTraitObject(self.rewrite_expr(a, d)?),
            E::LiftType(a) => E::LiftType(self.rewrite_expr(a, d)?),
            E::TyClosure(env, sig) => {
                E::TyClosure(self.rewrite_expr(env, d)?, self.rewrite_expr(sig, d)?)
            }
            E::TyClosureEnvironment(bindings) => {
                let mut cloned = Vec::with_capacity(bindings.len());
                for b in *bindings {
                    cloned.push(hir::expr::ClosureEnvironmentBinding {
                        name: b.name,
                        ty: self.rewrite_expr(b.ty, d)?,
                        value: b.value.map(|e| self.rewrite_expr(e, d)).transpose()?,
                    });
                }
                E::TyClosureEnvironment(self.arena.alloc_closure_environment_binding_slice(cloned))
            }
            E::TypeOf(a) => E::TypeOf(self.rewrite_expr(a, d)?),
            E::TakeOld(a) => E::TakeOld(self.rewrite_expr(a, d)?),
            E::TakeBitVec(a) => E::TakeBitVec(self.rewrite_expr(a, d)?),
            E::TakeIntrinsic(a) => E::TakeIntrinsic(self.rewrite_expr(a, d)?),
            E::TakeAttribute(a) => E::TakeAttribute(self.rewrite_expr(a, d)?),
            E::TakeTagName(a) => E::TakeTagName(self.rewrite_expr(a, d)?),
            E::TakeTupleEnumerate(a) => E::TakeTupleEnumerate(self.rewrite_expr(a, d)?),
            E::TakeTupleIndexed(a) => E::TakeTupleIndexed(self.rewrite_expr(a, d)?),
            E::MultiDeref(a) => E::MultiDeref(self.rewrite_expr(a, d)?),
            E::Ref(a) => E::Ref(self.rewrite_expr(a, d)?),
            E::Deref(a) => E::Deref(self.rewrite_expr(a, d)?),
            E::TyFnArrow(a, b) => E::TyFnArrow(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?),
            E::ErrorQualifiedType(a, b) => {
                E::ErrorQualifiedType(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?)
            }
            E::EffectQualifiedType(a, b) => {
                E::EffectQualifiedType(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?)
            }
            E::TraitBound(a, b) => {
                E::TraitBound(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?)
            }
            E::Cast(a, b) => E::Cast(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?),
            E::HandlerApplication(a, b) => {
                E::HandlerApplication(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?)
            }
            E::Index(a, b) => E::Index(self.rewrite_expr(a, d)?, self.rewrite_expr(b, d)?),
            E::Range {
                start,
                end,
                inclusive,
            } => E::Range {
                start: start.map(|e| self.rewrite_expr(e, d)).transpose()?,
                end: end.map(|e| self.rewrite_expr(e, d)).transpose()?,
                inclusive: *inclusive,
            },
            E::LiteralConversion { literal, suffix } => E::LiteralConversion {
                literal: self.rewrite_expr(literal, d)?,
                suffix: *suffix,
            },
            E::Object(entries) => {
                let mut new_entries = Vec::with_capacity(entries.len());
                for entry in entries {
                    new_entries.push(match entry {
                        hir::expr::ObjectEntry::Value(e) => {
                            hir::expr::ObjectEntry::Value(self.rewrite_expr(e, d)?)
                        }
                        hir::expr::ObjectEntry::Property(f) => {
                            hir::expr::ObjectEntry::Property(hir::expr::FieldExpr {
                                ident: f.ident,
                                expr: self.rewrite_expr(f.expr, d)?,
                                span: f.span,
                            })
                        }
                    });
                }
                E::Object(new_entries)
            }
            E::CollectedProperties(fields) => {
                let mut new_fields = Vec::with_capacity(fields.len());
                for f in *fields {
                    new_fields.push(hir::expr::FieldExpr {
                        ident: f.ident,
                        expr: self.rewrite_expr(f.expr, d)?,
                        span: f.span,
                    });
                }
                E::CollectedProperties(self.arena.alloc_field_expr_slice(new_fields))
            }
            E::RefinedType { requirement, base } => E::RefinedType {
                requirement: self.rewrite_expr(requirement, d)?,
                base: self.rewrite_expr(base, d)?,
            },
            E::TacticArgument(e) => E::TacticArgument(self.rewrite_expr(e, d)?),
            E::Tuple(items) | E::List(items) => {
                let mut cloned = Vec::with_capacity(items.len());
                for item in *items {
                    cloned.push(self.rewrite_expr(item, d)?.clone());
                }
                let slice = self.arena.alloc_expr_slice(cloned);
                if matches!(expr.kind, E::Tuple(_)) {
                    E::Tuple(slice)
                } else {
                    E::List(slice)
                }
            }
            E::TyFn(params)
            | E::TyComptimeFn(params)
            | E::TyModifiedFn(_, params)
            | E::TyNFFn(params) => {
                let mut cloned = Vec::with_capacity(params.len());
                for param in *params {
                    cloned.push(self.subst_ty_param(param, false, d)?);
                }
                let slice = self.arena.alloc_ty_param_slice(cloned);
                match expr.kind {
                    E::TyFn(_) => E::TyFn(slice),
                    E::TyComptimeFn(_) => E::TyComptimeFn(slice),
                    E::TyModifiedFn(m, _) => E::TyModifiedFn(m, slice),
                    _ => E::TyNFFn(slice),
                }
            }
            _ => return Ok(expr),
        };
        Ok(self.arena.alloc_expr(Expr {
            hir_id: expr.hir_id,
            kind,
            span: expr.span,
        }))
    }

    fn subst_ty_param(
        &mut self,
        ty: &TyParam<'h>,
        from_source: bool,
        depth: usize,
    ) -> Result<TyParam<'h>, Error> {
        let mut ty = ty.clone();
        if from_source {
            ty.hir_id = self.alloc_id(ty.span)?;
        }
        let mut go = |e: &'h Expr<'h>| {
            if from_source {
                self.subst_source_expr(e, depth)
            } else {
                self.rewrite_expr(e, depth)
            }
        };
        ty.kind = match ty.kind {
            T::Positional(e) => T::Positional(go(e)?),
            T::PositionalDependencyCatched(n, e) => T::PositionalDependencyCatched(n, go(e)?),
            T::Optional(n, e, def) => T::Optional(n, go(e)?, go(def)?),
            T::Varadic(n, e) => T::Varadic(n, go(e)?),
            T::Itself { is_ref } => T::Itself { is_ref },
            T::Case(pat) => T::Case(pat),
        };
        Ok(ty)
    }

    fn rewrite_block(
        &mut self,
        block: &'h Block<'h>,
        depth: usize,
    ) -> Result<&'h Block<'h>, Error> {
        check_depth(depth, block.span)?;
        charge(&mut self.fuel, block.span)?;
        let d = depth + 1;
        let mut stmts = Vec::with_capacity(block.stmts.len());
        for stmt in block.stmts {
            stmts.push(self.rewrite_expr(stmt, d)?.clone());
        }
        let expr = block.expr.map(|e| self.rewrite_expr(e, d)).transpose()?;
        Ok(self.arena.alloc_block(Block {
            hir_id: block.hir_id,
            stmts: self.arena.alloc_expr_slice(stmts),
            expr,
            span: block.span,
        }))
    }

    fn rewrite_arms(
        &mut self,
        arms: &'h [PatternArm<'h>],
        depth: usize,
    ) -> Result<&'h [PatternArm<'h>], Error> {
        let mut out = Vec::with_capacity(arms.len());
        for arm in arms {
            out.push(self.rewrite_arm(arm, depth)?);
        }
        Ok(self.arena.alloc_arm_slice(out))
    }

    fn rewrite_arm(&mut self, arm: &PatternArm<'h>, depth: usize) -> Result<PatternArm<'h>, Error> {
        Ok(PatternArm {
            hir_id: arm.hir_id,
            pat: self.rewrite_pattern(&arm.pat, depth)?,
            guard: arm.guard.map(|e| self.rewrite_expr(e, depth)).transpose()?,
            body: self.rewrite_expr(arm.body, depth)?,
            span: arm.span,
        })
    }

    fn rewrite_pattern_ref(
        &mut self,
        pat: &Pattern<'h>,
        depth: usize,
    ) -> Result<&'h Pattern<'h>, Error> {
        let rewritten = self.rewrite_pattern(pat, depth)?;
        Ok(self.arena.alloc_pattern(rewritten))
    }

    fn rewrite_pattern(&mut self, pat: &Pattern<'h>, depth: usize) -> Result<Pattern<'h>, Error> {
        check_depth(depth, pat.span)?;
        charge(&mut self.fuel, pat.span)?;
        let d = depth + 1;
        let kind = match &pat.kind {
            P::Binding(mode, name, inner) => P::Binding(
                *mode,
                *name,
                inner.map(|p| self.rewrite_pattern_ref(p, d)).transpose()?,
            ),
            P::Tuple(items) | P::Or(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in *items {
                    out.push(self.rewrite_pattern(item, d)?);
                }
                let slice = self.arena.alloc_pattern_slice(out);
                if matches!(pat.kind, P::Tuple(_)) {
                    P::Tuple(slice)
                } else {
                    P::Or(slice)
                }
            }
            P::List(items, rest) => {
                let mut out = Vec::with_capacity(items.len());
                for item in *items {
                    out.push(self.rewrite_pattern(item, d)?);
                }
                P::List(
                    self.arena.alloc_pattern_slice(out),
                    rest.map(|p| self.rewrite_pattern_ref(p, d)).transpose()?,
                )
            }
            P::Struct(head, fields, has_rest) => {
                let head = self.rewrite_pattern_ref(head, d)?;
                let mut out = Vec::with_capacity(fields.len());
                for field in *fields {
                    out.push(hir::pattern::FieldPat {
                        ident: field.ident,
                        pat: self.rewrite_pattern(&field.pat, d)?,
                        span: field.span,
                    });
                }
                P::Struct(head, self.arena.alloc_field_pat_slice(out), *has_rest)
            }
            P::AppStruct(head, fields) => {
                let head = self.rewrite_pattern_ref(head, d)?;
                let mut out = Vec::with_capacity(fields.len());
                for field in *fields {
                    out.push(hir::pattern::FieldPat {
                        ident: field.ident,
                        pat: self.rewrite_pattern(&field.pat, d)?,
                        span: field.span,
                    });
                }
                P::AppStruct(head, self.arena.alloc_field_pat_slice(out))
            }
            P::AppTuple(head, items) => {
                let head = self.rewrite_pattern_ref(head, d)?;
                let mut out = Vec::with_capacity(items.len());
                for item in *items {
                    out.push(self.rewrite_pattern(item, d)?);
                }
                P::AppTuple(head, self.arena.alloc_pattern_slice(out))
            }
            P::Then(left, expr, right) => P::Then(
                self.rewrite_pattern_ref(left, d)?,
                self.rewrite_expr(expr, d)?,
                self.rewrite_pattern_ref(right, d)?,
            ),
            P::Guard(inner, cond) => P::Guard(
                self.rewrite_pattern_ref(inner, d)?,
                self.rewrite_expr(cond, d)?,
            ),
            P::Projection(inner, field) => {
                P::Projection(self.rewrite_pattern_ref(inner, d)?, *field)
            }
            P::OptionSome(inner) => P::OptionSome(self.rewrite_pattern_ref(inner, d)?),
            P::ErrorOk(inner) => P::ErrorOk(self.rewrite_pattern_ref(inner, d)?),
            P::ErrorErr(inner) => P::ErrorErr(self.rewrite_pattern_ref(inner, d)?),
            P::Not(inner) => P::Not(self.rewrite_pattern_ref(inner, d)?),
            P::Ref(inner) => P::Ref(self.rewrite_pattern_ref(inner, d)?),
            P::Const(e) => P::Const(self.rewrite_expr(e, d)?),
            P::Comptime(e) => P::Comptime(self.rewrite_expr(e, d)?),
            P::Range(a, b, inc) => P::Range(
                a.map(|e| self.rewrite_expr(e, d)).transpose()?,
                b.map(|e| self.rewrite_expr(e, d)).transpose()?,
                inc.clone(),
            ),
            P::BitVec { radix, segments } => {
                let mut out = Vec::with_capacity(segments.len());
                for seg in segments {
                    out.push(match seg {
                        BitSegment::Literal { spelling, span } => BitSegment::Literal {
                            spelling: spelling.clone(),
                            span: *span,
                        },
                        BitSegment::Typed { pattern, ty } => BitSegment::Typed {
                            pattern: self.rewrite_pattern_ref(pattern, d)?,
                            ty: self.rewrite_expr(ty, d)?,
                        },
                        BitSegment::Computed(expr) => {
                            BitSegment::Computed(self.rewrite_expr(expr, d)?)
                        }
                    });
                }
                P::BitVec {
                    radix: *radix,
                    segments: out,
                }
            }
            _ => return Ok(pat.clone()),
        };
        Ok(Pattern {
            hir_id: pat.hir_id,
            kind,
            span: pat.span,
        })
    }

    fn rewrite_clause_kind(&mut self, kind: &K<'h>, depth: usize) -> Result<K<'h>, Error> {
        Ok(match kind {
            K::Type(name) => K::Type(*name),
            K::Positional(name, ty) => K::Positional(*name, self.rewrite_expr(ty, depth)?),
            K::Optional(name, ty, def) => K::Optional(
                *name,
                self.rewrite_expr(ty, depth)?,
                self.rewrite_expr(def, depth)?,
            ),
            K::Varadic(name, ty) => K::Varadic(*name, self.rewrite_expr(ty, depth)?),
            K::TraitBound(name, bound) => K::TraitBound(*name, self.rewrite_expr(bound, depth)?),
            K::Quote(inner) => K::Quote(Box::new(self.rewrite_clause_kind(inner, depth)?)),
        })
    }

    fn rewrite_clauses(
        &mut self,
        clauses: &'h [ClauseParam<'h>],
        depth: usize,
    ) -> Result<&'h [ClauseParam<'h>], Error> {
        let mut out = Vec::with_capacity(clauses.len());
        for cp in clauses {
            let mut cp = cp.clone();
            cp.kind = self.rewrite_clause_kind(&cp.kind, depth)?;
            out.push(cp);
        }
        Ok(self.arena.alloc_clause_param_slice(out))
    }

    fn rewrite_constraints(
        &mut self,
        constraints: &'h [ClauseConstraint<'h>],
        depth: usize,
    ) -> Result<&'h [ClauseConstraint<'h>], Error> {
        let mut out = Vec::with_capacity(constraints.len());
        for c in constraints {
            let mut c = c.clone();
            c.kind = match c.kind {
                C::Requires(e) => C::Requires(self.rewrite_expr(e, depth)?),
                C::Ensures(e) => C::Ensures(self.rewrite_expr(e, depth)?),
                C::Decreases(e) => C::Decreases(self.rewrite_expr(e, depth)?),
                C::Outcome => C::Outcome,
            };
            out.push(c);
        }
        Ok(self.arena.alloc_clause_slice(out))
    }

    fn rewrite_fn_params(
        &mut self,
        params: &'h [hir::common::FnParam<'h>],
        depth: usize,
    ) -> Result<&'h [hir::common::FnParam<'h>], Error> {
        let mut out = Vec::with_capacity(params.len());
        for p in params {
            let mut p = p.clone();
            p.ty = self.subst_ty_param(&p.ty, false, depth)?;
            p.template_annotation = p
                .template_annotation
                .map(|e| self.rewrite_expr(e, depth))
                .transpose()?;
            out.push(p);
        }
        Ok(self.arena.alloc_fn_param_slice(out))
    }

    fn rewrite_body_in_map(&mut self, id: BodyId, depth: usize) -> Result<(), Error> {
        let Some(body) = self.body_map.remove(&id.hir_id) else {
            return Ok(());
        };
        let mut new_params = Vec::with_capacity(body.params.len());
        for param in body.params {
            let mut param = param.clone();
            param.ty = self.subst_ty_param(&param.ty, false, depth)?;
            new_params.push(param);
        }
        let value = self.rewrite_expr(body.value, depth)?;
        let rewritten = Body {
            params: self.arena.alloc_param_slice(new_params),
            value,
        };
        self.bodies.push((id, rewritten));
        Ok(())
    }
}

/// Apply bound strategy clause substitutions across a cloned member draft.
#[allow(clippy::too_many_arguments)]
pub(super) fn substitute_draft<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    draft: &mut function_templates::ClonedMember<'h>,
    temp_to_expr: &BTreeMap<DefId, &'h Expr<'h>>,
    strategy_to_temp: &BTreeMap<DefId, DefId>,
    request_mapping: &BTreeMap<DefId, DefId>,
    self_replacement: Option<&'h Expr<'h>>,
    fuel: &mut usize,
) -> Result<(), Error> {
    if temp_to_expr.is_empty() {
        return Ok(());
    }
    let max_local = draft
        .bindings
        .values()
        .map(|id| id.local_id.raw())
        .chain(draft.resolution.iter().map(|(id, _)| id.local_id.raw()))
        .max()
        .unwrap_or(0);
    let next_local = max_local
        .checked_add(16)
        .ok_or_else(|| error(draft.item.span, "derivation HirId space exhausted"))?;
    let empty_owners = BTreeMap::new();
    let original_bodies: BTreeMap<HirId, Body<'h>> = draft
        .bodies
        .drain(..)
        .map(|(id, body)| (id.hir_id, body))
        .collect();
    let mut subst = DraftSubst {
        source,
        arena,
        owner: draft.item.owner_id,
        next_local,
        temp_to_expr,
        strategy_to_temp,
        request_mapping,
        self_replacement,
        owner_replacements: &empty_owners,
        resolution: Vec::new(),
        bodies: Vec::new(),
        body_map: original_bodies,
        fuel: *fuel,
    };
    let result = (|| -> Result<(), Error> {
        match &mut draft.item.kind {
            ItemKind::Impl(def) => {
                def.self_ty = subst.rewrite_expr(def.self_ty, 0)?;
                def.trait_ref = def
                    .trait_ref
                    .map(|e| subst.rewrite_expr(e, 0))
                    .transpose()?;
                def.generation_trait = def
                    .generation_trait
                    .map(|e| subst.rewrite_expr(e, 0))
                    .transpose()?;
                def.clause_params = subst.rewrite_clauses(def.clause_params, 0)?;
                def.clause_constraints = subst.rewrite_constraints(def.clause_constraints, 0)?;
            }
            ItemKind::Fn(sig, body_id) => {
                sig.params = subst.rewrite_fn_params(sig.params, 0)?;
                sig.return_ty = sig
                    .return_ty
                    .map(|e| subst.rewrite_expr(e, 0))
                    .transpose()?;
                sig.clause_params = subst.rewrite_clauses(sig.clause_params, 0)?;
                sig.clause_constraints = subst.rewrite_constraints(sig.clause_constraints, 0)?;
                subst.rewrite_body_in_map(*body_id, 0)?;
            }
            ItemKind::FnDeclaration(sig) => {
                sig.params = subst.rewrite_fn_params(sig.params, 0)?;
                sig.return_ty = sig
                    .return_ty
                    .map(|e| subst.rewrite_expr(e, 0))
                    .transpose()?;
                sig.clause_params = subst.rewrite_clauses(sig.clause_params, 0)?;
                sig.clause_constraints = subst.rewrite_constraints(sig.clause_constraints, 0)?;
            }
            ItemKind::Associated(def) => {
                def.parameters = subst.rewrite_fn_params(def.parameters, 0)?;
                def.clause_params = subst.rewrite_clauses(def.clause_params, 0)?;
                def.clause_constraints = subst.rewrite_constraints(def.clause_constraints, 0)?;
                def.ty = subst.rewrite_expr(def.ty, 0)?;
                def.default = def.default.map(|e| subst.rewrite_expr(e, 0)).transpose()?;
            }
            ItemKind::Const(ty, value) => {
                *ty = subst.rewrite_expr(ty, 0)?;
                *value = subst.rewrite_expr(value, 0)?;
            }
            _ => {}
        }
        let remaining_ids: Vec<BodyId> = subst.body_map.keys().copied().map(BodyId::new).collect();
        for body_id in remaining_ids {
            subst.rewrite_body_in_map(body_id, 0)?;
        }
        for attribute in &mut draft.attributes {
            attribute.value = subst.rewrite_expr(attribute.value, 0)?;
        }
        Ok(())
    })();
    *fuel = subst.fuel;
    result?;
    draft.bodies = subst.bodies;
    draft.resolution.extend(subst.resolution);
    Ok(())
}

/// Rewrite `E::Item(old_owner)` references inside block bodies after replacing
/// block-local `derive` requests with their generated `impl` owners.
pub(super) fn rewrite_body_item_owners<'h>(
    source: &Package<'h>,
    candidate: &mut Package<'h>,
    arena: &'h HirArena,
    parent_owners: &BTreeSet<OwnerId>,
    replacements: &BTreeMap<OwnerId, OwnerId>,
    fuel: &mut usize,
) -> Result<(), Error> {
    if parent_owners.is_empty() || replacements.is_empty() {
        return Ok(());
    }
    let body_ids: Vec<BodyId> = candidate
        .bodies()
        .map(|(id, _)| *id)
        .filter(|id| parent_owners.contains(&id.hir_id.owner))
        .collect();
    let empty_exprs = BTreeMap::new();
    let empty_defs = BTreeMap::new();
    for body_id in body_ids {
        let Some(body) = candidate.body(body_id).cloned() else {
            continue;
        };
        let mut body_map = BTreeMap::new();
        body_map.insert(body_id.hir_id, body);
        let mut subst = DraftSubst {
            source,
            arena,
            owner: body_id.hir_id.owner,
            next_local: 1,
            temp_to_expr: &empty_exprs,
            strategy_to_temp: &empty_defs,
            request_mapping: &empty_defs,
            self_replacement: None,
            owner_replacements: replacements,
            resolution: Vec::new(),
            bodies: Vec::new(),
            body_map,
            fuel: *fuel,
        };
        let result = subst.rewrite_body_in_map(body_id, 0);
        *fuel = subst.fuel;
        result?;
        for (id, updated) in subst.bodies {
            candidate.insert_body(id, updated);
        }
    }
    Ok(())
}

/// Build an uncalled private checking function for an unused `derive fn`
/// strategy so its header, clauses, `trait_ref`, parameters, constraints, and
/// strategy-level computations are validated without executing its body.
#[allow(clippy::too_many_arguments)]
pub(super) fn unused_strategy_check_draft<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    strategy: OwnerId,
    check_owner: OwnerId,
    self_def: DefId,
    clause_defs: &BTreeMap<DefId, DefId>,
    input_stages: &BTreeMap<HirId, DefId>,
    body_info: &StrategyBody<'h>,
    fuel: &mut usize,
) -> Result<function_templates::ClonedMember<'h>, Error> {
    let item = source.item(strategy).unwrap();
    let ItemKind::Deriver(generator) = &item.kind else {
        unreachable!()
    };
    let span = item.span;
    let has_quote_clause = generator.clause_params.iter().any(|cp| cp.kind.is_quote());

    let self_ident = Ident::new(hir::Symbol::intern("$derive_self"), span);
    let self_ref = arena.alloc_expr(Expr {
        hir_id: HirId::new(check_owner, ItemLocalId::new(1)),
        span,
        kind: E::ResolvedClauseParam(self_ident.name, self_def),
    });

    // Include `requires $Self :- <trait_ref>` so the strategy's trait_ref is
    // validated in full clause + parameter scope without a fake concrete subject.
    let trait_bound_expr = arena.alloc_expr(Expr {
        hir_id: HirId::new(check_owner, ItemLocalId::new(2)),
        span: generator.trait_ref.span,
        kind: E::TraitBound(self_ref, generator.trait_ref),
    });
    let mut combined_constraints = Vec::with_capacity(generator.clause_constraints.len() + 1);
    combined_constraints.push(ClauseConstraint {
        label: None,
        hir_id: HirId::new(check_owner, ItemLocalId::new(3)),
        kind: C::Requires(trait_bound_expr),
        span: generator.trait_ref.span,
    });
    combined_constraints.extend(generator.clause_constraints.iter().cloned());

    let mut definitions = clause_defs.clone();
    definitions.insert(self_def, self_def);

    let exported = BTreeSet::new();
    let empty_locals: &[function_templates::GenerationBinding<'h>] = &[];
    let (check_params, check_locals, check_computations, check_constraints): (
        &[hir::common::FnParam<'h>],
        &[function_templates::GenerationBinding<'h>],
        &[&'h Expr<'h>],
        &[ClauseConstraint<'h>],
    ) = if has_quote_clause {
        // Runtime functions accept `quote` clause parameters; omit comptime-only
        // strategy statements from the runtime header check.
        (&[], empty_locals, &[], &combined_constraints[..1])
    } else {
        (
            generator.parameters,
            &body_info.locals,
            &body_info.computations,
            &combined_constraints,
        )
    };

    let mut draft = function_templates::execution_draft(
        source,
        arena,
        check_owner,
        strategy,
        self_ref,
        check_params,
        check_constraints,
        input_stages,
        check_locals,
        check_computations,
        &exported,
        &definitions,
        span,
        fuel,
    )?;

    // Attach cloned strategy clause parameters plus the abstract `$Self` binder.
    let max_local = draft
        .bindings
        .values()
        .map(|id| id.local_id.raw())
        .max()
        .unwrap_or(0);
    let next_local = max_local
        .checked_add(16)
        .ok_or_else(|| error(span, "unused strategy check HirId space exhausted"))?;
    let empty_exprs = BTreeMap::new();
    let empty_owners = BTreeMap::new();
    let mut subst = DraftSubst {
        source,
        arena,
        owner: check_owner,
        next_local,
        temp_to_expr: &empty_exprs,
        strategy_to_temp: &empty_exprs_defs(),
        request_mapping: &definitions,
        self_replacement: Some(self_ref),
        owner_replacements: &empty_owners,
        resolution: Vec::new(),
        bodies: Vec::new(),
        body_map: BTreeMap::new(),
        fuel: *fuel,
    };
    let mut cloned_clauses = Vec::with_capacity(generator.clause_params.len() + 1);
    for cp in generator.clause_params {
        let orig_def = cp
            .definition
            .ok_or_else(|| error(cp.span, "unused derivation strategy clause has no identity"))?;
        let fresh_def = definitions[&orig_def];
        let mut cloned = cp.clone();
        cloned.hir_id = subst.alloc_id(cp.span)?;
        cloned.definition = Some(fresh_def);
        cloned.kind = clone_source_clause_kind(&mut subst, &cp.kind, 0)?;
        cloned_clauses.push(cloned);
    }
    cloned_clauses.push(ClauseParam {
        hir_id: subst.alloc_id(span)?,
        definition: Some(self_def),
        name: self_ident,
        kind: K::Type(self_ident),
        span,
    });
    *fuel = subst.fuel;
    draft.resolution.extend(subst.resolution);
    let clause_slice = arena.alloc_clause_param_slice(cloned_clauses);
    for parameter in clause_slice {
        draft.nodes.insert(
            parameter.hir_id.local_id,
            ItemLocalId::new(0),
            hir::Node::ClauseParam(parameter),
        );
    }
    if let ItemKind::Fn(sig, _) = &mut draft.item.kind {
        sig.clause_params = clause_slice;
        if has_quote_clause {
            sig.modifiers.is_comptime = false;
            let void_ty = arena.alloc_expr(Expr {
                hir_id: HirId::new(check_owner, ItemLocalId::new(next_local + 1000)),
                span,
                kind: E::TyVoid,
            });
            let unit_val = arena.alloc_expr(Expr {
                hir_id: HirId::new(check_owner, ItemLocalId::new(next_local + 1001)),
                span,
                kind: E::Unit,
            });
            sig.return_ty = Some(void_ty);
            for (_, body) in &mut draft.bodies {
                body.value = unit_val;
            }
        }
    }
    draft.item.ident = Ident::new(
        hir::Symbol::intern(&format!("$derive_unused_{}", item.ident.name)),
        span,
    );
    Ok(draft)
}

fn empty_exprs_defs() -> BTreeMap<DefId, DefId> {
    BTreeMap::new()
}

fn clone_source_clause_kind<'h>(
    subst: &mut DraftSubst<'_, 'h>,
    kind: &K<'h>,
    depth: usize,
) -> Result<K<'h>, Error> {
    Ok(match kind {
        K::Type(name) => K::Type(*name),
        K::Positional(name, ty) => K::Positional(*name, subst.subst_source_expr(ty, depth)?),
        K::Optional(name, ty, def) => K::Optional(
            *name,
            subst.subst_source_expr(ty, depth)?,
            subst.subst_source_expr(def, depth)?,
        ),
        K::Varadic(name, ty) => K::Varadic(*name, subst.subst_source_expr(ty, depth)?),
        K::TraitBound(name, bound) => K::TraitBound(*name, subst.subst_source_expr(bound, depth)?),
        K::Quote(inner) => K::Quote(Box::new(clone_source_clause_kind(subst, inner, depth)?)),
    })
}

/// For generic `derive` requests whose selected strategy imposes `where` trait
/// bounds or `requires` constraints, emit a pair of uncalled private check
/// functions verifying that the request's own `where` clauses discharge the
/// strategy's requirements without silently adding new obligations to the impl.
#[allow(clippy::too_many_arguments)]
pub(super) fn generic_request_requirement_check_drafts<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    request: &hir::item::DeriveRequest<'h>,
    strategy_constraints: &[ClauseConstraint<'h>],
    temp_to_expr: &BTreeMap<DefId, &'h Expr<'h>>,
    strategy_to_temp: &BTreeMap<DefId, DefId>,
    target_owner: OwnerId,
    target_identity: DefId,
    caller_owner: OwnerId,
    target_clause_defs: &BTreeMap<DefId, DefId>,
    caller_clause_defs: &BTreeMap<DefId, DefId>,
    span: rustc_span::Span,
    fuel: &mut usize,
) -> Result<
    (
        function_templates::ClonedMember<'h>,
        function_templates::ClonedMember<'h>,
    ),
    Error,
> {
    let has_quote = request.clause_params.iter().any(|cp| cp.kind.is_quote());
    let empty_owners = BTreeMap::new();
    let empty_exprs = BTreeMap::new();
    let empty_defs = BTreeMap::new();

    // Helper 1: `fn $derive_req_target() where <req_clauses>, <strategy_reqs> {}`
    let mut target_subst = DraftSubst {
        source,
        arena,
        owner: target_owner,
        next_local: 1,
        temp_to_expr,
        strategy_to_temp,
        request_mapping: target_clause_defs,
        self_replacement: Some(request.self_ty),
        owner_replacements: &empty_owners,
        resolution: Vec::new(),
        bodies: Vec::new(),
        body_map: BTreeMap::new(),
        fuel: *fuel,
    };
    let mut target_clauses = Vec::with_capacity(request.clause_params.len());
    for cp in request.clause_params {
        let orig = cp.definition.unwrap();
        let mut cloned = cp.clone();
        cloned.hir_id = target_subst.alloc_id(cp.span)?;
        cloned.definition = Some(target_clause_defs[&orig]);
        cloned.kind = clone_source_clause_kind(&mut target_subst, &cp.kind, 0)?;
        target_clauses.push(cloned);
    }
    let mut target_reqs = Vec::with_capacity(strategy_constraints.len());
    for c in strategy_constraints {
        let mut c = c.clone();
        c.hir_id = target_subst.alloc_id(c.span)?;
        c.kind = match c.kind {
            C::Requires(e) => C::Requires(target_subst.subst_source_expr(e, 0)?),
            C::Ensures(e) => C::Ensures(target_subst.subst_source_expr(e, 0)?),
            C::Decreases(e) => C::Decreases(target_subst.subst_source_expr(e, 0)?),
            C::Outcome => C::Outcome,
        };
        target_reqs.push(c);
    }
    let target_void_ty = arena.alloc_expr(Expr {
        hir_id: target_subst.alloc_id(span)?,
        span,
        kind: E::TyVoid,
    });
    let target_unit = arena.alloc_expr(Expr {
        hir_id: target_subst.alloc_id(span)?,
        span,
        kind: E::Unit,
    });
    let target_body_id = BodyId::new(target_subst.alloc_id(span)?);
    *fuel = target_subst.fuel;
    let target_clause_slice = arena.alloc_clause_param_slice(target_clauses);
    let mut target_nodes = hir::OwnerNodes::new();
    for cp in target_clause_slice {
        target_nodes.insert(
            cp.hir_id.local_id,
            ItemLocalId::new(0),
            hir::Node::ClauseParam(cp),
        );
    }
    let target_draft = function_templates::ClonedMember {
        capture_uses: BTreeSet::new(),
        item: hir::Item {
            owner_id: target_owner,
            ident: Ident::new(hir::Symbol::intern("$derive_req_target"), span),
            span,
            kind: ItemKind::Fn(
                hir::item::FnSig {
                    params: &[],
                    return_ty: Some(target_void_ty),
                    return_bind: None,
                    return_binding: None,
                    modifiers: hir::item::FnModifiers {
                        is_comptime: !has_quote,
                        ..Default::default()
                    },
                    clause_params: target_clause_slice,
                    clause_constraints: arena.alloc_clause_slice(target_reqs),
                    span,
                },
                target_body_id,
            ),
        },
        bodies: vec![(
            target_body_id,
            Body {
                params: &[],
                value: target_unit,
            },
        )],
        bindings: BTreeMap::new(),
        nodes: target_nodes,
        attributes: Vec::new(),
        staged_parameters: Vec::new(),
        resolution: target_subst.resolution,
    };

    // Helper 2: `fn $derive_req_check() where <req_clauses>, <req_constraints> { $derive_req_target<...>() }`
    let mut caller_subst = DraftSubst {
        source,
        arena,
        owner: caller_owner,
        next_local: 1,
        temp_to_expr: &empty_exprs,
        strategy_to_temp: &empty_defs,
        request_mapping: caller_clause_defs,
        self_replacement: None,
        owner_replacements: &empty_owners,
        resolution: Vec::new(),
        bodies: Vec::new(),
        body_map: BTreeMap::new(),
        fuel: *fuel,
    };
    let mut caller_clauses = Vec::with_capacity(request.clause_params.len());
    let mut call_clause_args = Vec::with_capacity(request.clause_params.len());
    for cp in request.clause_params {
        let orig = cp.definition.unwrap();
        let fresh = caller_clause_defs[&orig];
        let mut cloned = cp.clone();
        cloned.hir_id = caller_subst.alloc_id(cp.span)?;
        cloned.definition = Some(fresh);
        cloned.kind = clone_source_clause_kind(&mut caller_subst, &cp.kind, 0)?;
        let arg_expr = arena.alloc_expr(Expr {
            hir_id: caller_subst.alloc_id(cp.span)?,
            span: cp.span,
            kind: E::ResolvedClauseParam(cp.name.name, fresh),
        });
        if matches!(cp.kind.unquoted(), K::Optional(..)) {
            call_clause_args.push(Arg::Named(cp.name, arg_expr));
        } else {
            call_clause_args.push(Arg::Positional(arg_expr));
        }
        caller_clauses.push(cloned);
    }
    let mut caller_reqs = Vec::with_capacity(request.clause_constraints.len());
    for c in request.clause_constraints {
        let mut c = c.clone();
        c.hir_id = caller_subst.alloc_id(c.span)?;
        c.kind = match c.kind {
            C::Requires(e) => C::Requires(caller_subst.subst_source_expr(e, 0)?),
            C::Ensures(e) => C::Ensures(caller_subst.subst_source_expr(e, 0)?),
            C::Decreases(e) => C::Decreases(caller_subst.subst_source_expr(e, 0)?),
            C::Outcome => C::Outcome,
        };
        caller_reqs.push(c);
    }
    let callee_def = arena.alloc_expr(Expr {
        hir_id: caller_subst.alloc_id(span)?,
        span,
        kind: E::ResolvedDef {
            name: hir::Symbol::intern("$derive_req_target"),
            def_id: target_identity,
            has_clause_param: !call_clause_args.is_empty(),
        },
    });
    let callee = if call_clause_args.is_empty() {
        callee_def
    } else {
        arena.alloc_expr(Expr {
            hir_id: caller_subst.alloc_id(span)?,
            span,
            kind: E::NFApplication(callee_def, arena.alloc_arg_slice(call_clause_args)),
        })
    };
    let call_expr = arena.alloc_expr(Expr {
        hir_id: caller_subst.alloc_id(span)?,
        span,
        kind: E::Application(callee, &[]),
    });
    let caller_void_ty = arena.alloc_expr(Expr {
        hir_id: caller_subst.alloc_id(span)?,
        span,
        kind: E::TyVoid,
    });
    let caller_body_id = BodyId::new(caller_subst.alloc_id(span)?);
    *fuel = caller_subst.fuel;
    let caller_clause_slice = arena.alloc_clause_param_slice(caller_clauses);
    let mut caller_nodes = hir::OwnerNodes::new();
    for cp in caller_clause_slice {
        caller_nodes.insert(
            cp.hir_id.local_id,
            ItemLocalId::new(0),
            hir::Node::ClauseParam(cp),
        );
    }
    let caller_draft = function_templates::ClonedMember {
        capture_uses: BTreeSet::new(),
        item: hir::Item {
            owner_id: caller_owner,
            ident: Ident::new(hir::Symbol::intern("$derive_req_check"), span),
            span,
            kind: ItemKind::Fn(
                hir::item::FnSig {
                    params: &[],
                    return_ty: Some(caller_void_ty),
                    return_bind: None,
                    return_binding: None,
                    modifiers: hir::item::FnModifiers {
                        is_comptime: !has_quote,
                        ..Default::default()
                    },
                    clause_params: caller_clause_slice,
                    clause_constraints: arena.alloc_clause_slice(caller_reqs),
                    span,
                },
                caller_body_id,
            ),
        },
        bodies: vec![(
            caller_body_id,
            Body {
                params: &[],
                value: call_expr,
            },
        )],
        bindings: BTreeMap::new(),
        nodes: caller_nodes,
        attributes: Vec::new(),
        staged_parameters: Vec::new(),
        resolution: caller_subst.resolution,
    };
    Ok((target_draft, caller_draft))
}
