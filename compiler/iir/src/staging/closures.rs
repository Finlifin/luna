use super::*;
use nir::program::{
    CallArgument, Capture, Expr, ExprKind as K, LocalId, MatchArm, NormalFormArgument,
    NormalFormInputPlan, NormalFormPreparation, Pattern as P, Place, Rewriter, SourceRange,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;

pub(super) fn unwrap_initializer_closure_call(expr: &Expr) -> Option<&Expr> {
    let K::Call {
        callee,
        arguments,
        env_arguments,
        parameter_bindings,
    } = &expr.kind
    else {
        return None;
    };
    if !arguments.is_empty()
        || !env_arguments.is_empty()
        || !parameter_bindings.is_empty()
        || !callee.ty.is_comptime_callable()
    {
        return None;
    }
    let K::Closure {
        parameters,
        captures,
        body,
    } = &callee.kind
    else {
        return None;
    };
    (parameters.is_empty() && captures.is_empty() && body.ty == expr.ty).then_some(body.as_ref())
}

pub(super) fn validate_closed_closure_delivery<'layout>(
    heads: Layouts<'layout>,
    expr: &Expr,
    locals: &BTreeSet<LocalId>,
    fuel: &mut usize,
    depth: usize,
    recurse: &mut impl FnMut(
        Layouts<'layout>,
        &Expr,
        &BTreeSet<LocalId>,
        &mut usize,
        usize,
    ) -> Result<(), String>,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("staged constant depth exceeded".into());
    }
    charge(fuel, 1)?;
    match &expr.kind {
        K::Local(id) if locals.contains(id) => Ok(()),
        K::Block { statements, result } => {
            crate::completed_type(&expr.ty, &BTreeSet::new(), fuel)?;
            let mut inner = locals.clone();
            for statement in statements {
                let K::Let { binding, value } = &statement.kind else {
                    return Err("staged constant block allows only closed let bindings".into());
                };
                recurse(heads, value, &inner, fuel, depth + 1)?;
                inner.insert(*binding);
            }
            if let Some(result) = result {
                recurse(heads, result, &inner, fuel, depth + 1)?;
            }
            Ok(())
        }
        K::Call { .. } => {
            let Some(body) = unwrap_initializer_closure_call(expr) else {
                return Err("staged input requires a supported canonical constant".into());
            };
            recurse(heads, body, locals, fuel, depth + 1)
        }
        K::Closure {
            parameters,
            captures,
            body,
        } => {
            validate_closure_type(&expr.ty, fuel)?;
            charge(fuel, parameters.len() + captures.len())?;
            let mut allowed: BTreeSet<LocalId> = parameters.iter().copied().collect();
            for capture in captures {
                crate::completed_type(&capture.ty, &BTreeSet::new(), fuel)?;
                if let Some(value) = &capture.value {
                    if value.ty != capture.ty {
                        return Err("staged closure capture type mismatch".into());
                    }
                    recurse(heads, value, locals, fuel, depth + 1)?;
                } else if !locals.contains(&capture.binding) {
                    return Err("staged closure capture must be closed".into());
                }
                allowed.insert(capture.binding);
            }
            check_closure_body_closed(body, &allowed, fuel)
        }
        _ => Err("staged input requires a supported canonical constant".into()),
    }
}

pub(super) fn canonicalize_closed_closure<'layout>(
    heads: Layouts<'layout>,
    expr: &Expr,
    parameters: &[LocalId],
    captures: &[Capture],
    body: &Expr,
    locals: &BTreeMap<LocalId, Term>,
    fuel: &mut usize,
    depth: usize,
    resolve_constant: &mut impl FnMut(
        Layouts<'layout>,
        &Expr,
        &BTreeMap<LocalId, Term>,
        &mut usize,
        usize,
    ) -> Result<Term, String>,
) -> Result<Term, String> {
    validate_closure_type(&expr.ty, fuel)?;
    charge(fuel, parameters.len() + captures.len())?;

    let mut allowed: BTreeSet<LocalId> = parameters.iter().copied().collect();
    let mut capture_terms = Vec::with_capacity(captures.len());
    for capture in captures {
        crate::completed_type(&capture.ty, &BTreeSet::new(), fuel)?;
        let term = if let Some(value) = &capture.value {
            if value.ty != capture.ty {
                return Err("staged closure capture type mismatch".into());
            }
            resolve_constant(heads, value, locals, fuel, depth + 1)?
        } else if let Some(bound) = locals.get(&capture.binding) {
            nir::type_computations::charge_term(bound, fuel).map_err(|e| e.to_string())?;
            bound.clone()
        } else {
            return Err("staged closure capture must be a closed constant".into());
        };
        allowed.insert(capture.binding);
        capture_terms.push(term);
    }

    check_closure_body_closed(body, &allowed, fuel)?;
    let canonical = alpha_normalize_closure(parameters, captures, body, fuel)?;
    Ok(Term::ClosureValue {
        ty: Box::new(expr.ty.clone()),
        body: canonical.into(),
        captures: capture_terms,
    })
}

fn validate_closure_type(ty: &Term, fuel: &mut usize) -> Result<(), String> {
    crate::completed_type(ty, &BTreeSet::new(), fuel)?;
    if !matches!(
        ty,
        Term::Closure { .. }
            | Term::Function { .. }
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::UnsafeFunction(_)
    ) {
        return Err("staged closure requires a completed callable or closure type".into());
    }
    Ok(())
}

fn check_closure_body_closed(
    body: &Expr,
    allowed: &BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let free = nir::free_bindings::collect(body, fuel)
        .map_err(|error| format!("staged closure binding check failed: {error:?}"))?;
    if free.into_iter().any(|id| !allowed.contains(&id)) {
        return Err("staged closure cannot reference uncaptured local bindings".into());
    }
    Ok(())
}

pub(super) fn alpha_normalize_closure(
    parameters: &[LocalId],
    captures: &[Capture],
    body: &Expr,
    fuel: &mut usize,
) -> Result<String, String> {
    let mut normalizer = AlphaNormalizer::default();
    let mut normalized_params = Vec::with_capacity(parameters.len());
    for parameter in parameters {
        normalized_params.push(normalizer.bind(*parameter, fuel)?);
    }
    let mut normalized_captures = Vec::with_capacity(captures.len());
    for capture in captures {
        let binding = normalizer.bind(capture.binding, fuel)?;
        normalized_captures.push(Capture {
            binding,
            ty: capture.ty.clone(),
            value: None,
        });
    }
    let mut normalized_body = body.clone();
    normalizer.rewrite_expr(&mut normalized_body, fuel)?;

    let mut out = String::from("(closure (parameters");
    for param in &normalized_params {
        let _ = write!(out, " {param}");
    }
    out.push_str(") (captures");
    for cap in &normalized_captures {
        let _ = write!(out, " (capture {} {})", cap.binding, cap.ty);
    }
    let _ = write!(out, ") {normalized_body})");
    Ok(out)
}

#[derive(Default)]
struct AlphaNormalizer {
    bindings: BTreeMap<LocalId, LocalId>,
    next_index: u32,
}

impl AlphaNormalizer {
    fn mark(&self) -> Vec<(LocalId, Option<LocalId>)> {
        Vec::new()
    }

    fn bind_in(
        &mut self,
        original: LocalId,
        saved: &mut Vec<(LocalId, Option<LocalId>)>,
        fuel: &mut usize,
    ) -> Result<LocalId, String> {
        charge(fuel, 1)?;
        let mapped = LocalId {
            owner: 0,
            index: self.next_index,
        };
        self.next_index = self
            .next_index
            .checked_add(1)
            .ok_or("staged closure local index overflow")?;
        let previous = self.bindings.insert(original, mapped);
        saved.push((original, previous));
        Ok(mapped)
    }

    fn bind(&mut self, original: LocalId, fuel: &mut usize) -> Result<LocalId, String> {
        let mut discarded = Vec::new();
        self.bind_in(original, &mut discarded, fuel)
    }

    fn restore(&mut self, saved: Vec<(LocalId, Option<LocalId>)>) {
        for (original, previous) in saved.into_iter().rev() {
            match previous {
                Some(prev) => {
                    self.bindings.insert(original, prev);
                }
                None => {
                    self.bindings.remove(&original);
                }
            }
        }
    }

    fn lookup(&self, original: LocalId) -> Result<LocalId, String> {
        self.bindings
            .get(&original)
            .copied()
            .ok_or_else(|| "staged closure contains unbound local reference".to_owned())
    }

    fn rewrite_expr(&mut self, expr: &mut Expr, fuel: &mut usize) -> Result<(), String> {
        struct Bridge<'a> {
            norm: &'a mut AlphaNormalizer,
            fuel: &'a mut usize,
        }

        impl Rewriter for Bridge<'_> {
            type Error = String;

            fn expression(&mut self, expr: &mut Expr) -> Result<bool, Self::Error> {
                self.norm.normalize_expr_node(expr, self.fuel)
            }

            fn pattern(&mut self, pattern: &mut P) -> Result<(), Self::Error> {
                // Handled explicitly in binder-introducing expressions; non-binding
                // nested patterns shouldn't remap bindings without a scope.
                let _ = pattern;
                Ok(())
            }
        }

        expr.rewrite_with(&mut Bridge { norm: self, fuel })
    }

    fn normalize_expr_node(&mut self, expr: &mut Expr, fuel: &mut usize) -> Result<bool, String> {
        charge(fuel, 1)?;
        expr.source = SourceRange { start: 0, end: 0 };
        match &mut expr.kind {
            K::Local(id) => {
                *id = self.lookup(*id)?;
                Ok(false)
            }
            K::Assign { place, value } => {
                self.normalize_place(place, fuel)?;
                self.rewrite_expr(value, fuel)?;
                Ok(false)
            }
            K::BitAssign { place, value, .. } | K::BitUpdate { place, value, .. } => {
                self.normalize_place(place, fuel)?;
                self.rewrite_expr(value, fuel)?;
                Ok(false)
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut saved = self.mark();
                for statement in statements {
                    self.normalize_statement(statement, &mut saved, fuel)?;
                }
                if let Some(result) = result {
                    self.rewrite_expr(result, fuel)?;
                }
                self.restore(saved);
                Ok(false)
            }
            K::Let { .. } | K::LetPattern { .. } => {
                let mut saved = self.mark();
                self.normalize_statement(expr, &mut saved, fuel)?;
                self.restore(saved);
                Ok(false)
            }
            K::Match { value, arms } => {
                self.rewrite_expr(value, fuel)?;
                for arm in arms {
                    self.normalize_match_arm(arm, fuel)?;
                }
                Ok(false)
            }
            K::Loop { identity, body } => {
                let mut saved = self.mark();
                *identity = self.bind_in(*identity, &mut saved, fuel)?;
                self.rewrite_expr(body, fuel)?;
                self.restore(saved);
                Ok(false)
            }
            K::Break(id) | K::Continue(id) => {
                *id = self.lookup(*id)?;
                Ok(false)
            }
            K::Closure {
                parameters,
                captures,
                body,
            } => {
                let mut inner = AlphaNormalizer {
                    bindings: BTreeMap::new(),
                    next_index: self.next_index,
                };
                for capture in captures.iter_mut() {
                    if let Some(value) = &mut capture.value {
                        self.rewrite_expr(value, fuel)?;
                        capture.binding = inner.bind(capture.binding, fuel)?;
                    } else {
                        let outer_id = self.lookup(capture.binding)?;
                        inner.bindings.insert(capture.binding, outer_id);
                        capture.binding = outer_id;
                    }
                }
                for param in parameters.iter_mut() {
                    *param = inner.bind(*param, fuel)?;
                }
                inner.rewrite_expr(Arc::make_mut(body), fuel)?;
                self.next_index = inner.next_index;
                Ok(false)
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
                env_arguments,
            } => {
                self.rewrite_expr(callee, fuel)?;
                for env in env_arguments {
                    self.rewrite_expr(env, fuel)?;
                }
                self.normalize_call_arguments(arguments, parameter_bindings, fuel)?;
                Ok(false)
            }
            K::DynCall(call) => {
                self.rewrite_expr(&mut call.receiver, fuel)?;
                self.normalize_call_arguments(
                    &mut call.arguments,
                    &mut call.parameter_bindings,
                    fuel,
                )?;
                Ok(false)
            }
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => {
                self.rewrite_expr(computation, fuel)?;
                for handler in handlers {
                    for arm in &mut handler.arms {
                        self.normalize_match_arm(arm, fuel)?;
                    }
                }
                Ok(false)
            }
            K::ComptimeFunctionInstance { inputs, .. } => {
                let mut saved = self.mark();
                for statement in &mut inputs.statements {
                    self.normalize_statement(statement, &mut saved, fuel)?;
                }
                for argument in &mut inputs.arguments {
                    self.rewrite_expr(argument, fuel)?;
                }
                self.restore(saved);
                Ok(false)
            }
            K::PlannedFunctionInstance {
                arguments, plan, ..
            }
            | K::NormalFormType {
                arguments, plan, ..
            } => {
                self.normalize_planned_inputs(plan.as_mut(), arguments, fuel)?;
                Ok(false)
            }
            K::ApplyNormalForm {
                callee,
                arguments,
                plan,
                ..
            } => {
                self.rewrite_expr(Arc::make_mut(callee), fuel)?;
                self.normalize_planned_inputs(plan.as_mut(), arguments, fuel)?;
                Ok(false)
            }
            K::NominalType {
                preparations,
                order,
                bindings,
                arguments,
                ..
            } => {
                self.normalize_scheduled_inputs(
                    preparations,
                    order.as_deref(),
                    bindings.as_deref_mut(),
                    arguments,
                    None,
                    fuel,
                )?;
                Ok(false)
            }
            K::CheckTypeAlias {
                preparations,
                order,
                bindings,
                arguments,
                body,
                ..
            } => {
                self.normalize_scheduled_inputs(
                    preparations,
                    order.as_deref(),
                    bindings.as_deref_mut(),
                    arguments,
                    body.as_deref_mut(),
                    fuel,
                )?;
                Ok(false)
            }
            K::WithTypeInputs {
                preparations,
                arguments,
                body,
                ..
            } => {
                self.normalize_scheduled_inputs(
                    preparations,
                    None,
                    None,
                    arguments,
                    Some(body.as_mut()),
                    fuel,
                )?;
                Ok(false)
            }
            _ => Ok(true),
        }
    }

    fn normalize_place(&mut self, place: &mut Place, fuel: &mut usize) -> Result<(), String> {
        charge(fuel, 1)?;
        place.root = self.lookup(place.root)?;
        Ok(())
    }

    fn normalize_statement(
        &mut self,
        expr: &mut Expr,
        saved: &mut Vec<(LocalId, Option<LocalId>)>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        charge(fuel, 1)?;
        expr.source = SourceRange { start: 0, end: 0 };
        match &mut expr.kind {
            K::Let { binding, value } => {
                self.rewrite_expr(value, fuel)?;
                *binding = self.bind_in(*binding, saved, fuel)?;
                Ok(())
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                self.rewrite_expr(value, fuel)?;
                let mut pat_saved = self.mark();
                self.normalize_pattern(pattern, None, &mut pat_saved, fuel)?;
                if let Some(guard) = guard {
                    self.rewrite_expr(guard, fuel)?;
                }
                if let Some(otherwise) = otherwise {
                    let mut pat_bound = Vec::new();
                    let mut seen = BTreeSet::new();
                    for (orig, _) in pat_saved.iter().rev() {
                        if seen.insert(*orig)
                            && let Some(&mapped) = self.bindings.get(orig)
                        {
                            pat_bound.push((*orig, mapped));
                        }
                    }
                    pat_bound.reverse();
                    // On failure of the pattern, `otherwise` sees only the pre-pattern scope.
                    self.restore(pat_saved);
                    self.rewrite_expr(otherwise, fuel)?;
                    // Re-bind pattern outputs into `saved` for downstream statements on match.
                    for (orig, mapped) in pat_bound {
                        let prev = self.bindings.insert(orig, mapped);
                        saved.push((orig, prev));
                    }
                } else {
                    saved.extend(pat_saved);
                }
                Ok(())
            }
            _ => {
                self.rewrite_expr(expr, fuel)?;
                Ok(())
            }
        }
    }

    fn normalize_match_arm(&mut self, arm: &mut MatchArm, fuel: &mut usize) -> Result<(), String> {
        let mut saved = self.mark();
        self.normalize_pattern(&mut arm.pattern, None, &mut saved, fuel)?;
        if let Some(guard) = &mut arm.guard {
            self.rewrite_expr(guard, fuel)?;
        }
        self.rewrite_expr(&mut arm.body, fuel)?;
        self.restore(saved);
        Ok(())
    }

    fn bind_pattern_in(
        &mut self,
        original: LocalId,
        shared: Option<&BTreeMap<LocalId, LocalId>>,
        saved: &mut Vec<(LocalId, Option<LocalId>)>,
        fuel: &mut usize,
    ) -> Result<LocalId, String> {
        if let Some(mapped) = shared.and_then(|s| s.get(&original).copied()) {
            charge(fuel, 1)?;
            let previous = self.bindings.insert(original, mapped);
            saved.push((original, previous));
            Ok(mapped)
        } else {
            self.bind_in(original, saved, fuel)
        }
    }

    fn normalize_pattern(
        &mut self,
        pattern: &mut P,
        shared: Option<&BTreeMap<LocalId, LocalId>>,
        saved: &mut Vec<(LocalId, Option<LocalId>)>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        charge(fuel, 1)?;
        match pattern {
            P::Binding { binding, inner } => {
                self.normalize_pattern(inner, shared, saved, fuel)?;
                *binding = self.bind_pattern_in(*binding, shared, saved, fuel)?;
            }
            P::Observe { binding, inner } => {
                let mut obs_saved = self.mark();
                let mapped = self.bind_in(*binding, &mut obs_saved, fuel)?;
                *binding = mapped;
                self.normalize_pattern(inner, shared, &mut obs_saved, fuel)?;
                self.restore(obs_saved);
            }
            P::Then { first, value, next } => {
                self.normalize_pattern(first, shared, saved, fuel)?;
                self.rewrite_expr(value, fuel)?;
                self.normalize_pattern(next, shared, saved, fuel)?;
            }
            P::Guard { pattern, condition } => {
                self.normalize_pattern(pattern, shared, saved, fuel)?;
                self.rewrite_expr(condition, fuel)?;
            }
            P::Remap { pattern, bindings } => {
                self.normalize_pattern(pattern, shared, saved, fuel)?;
                for (from, to) in bindings.iter_mut() {
                    *from = self.lookup(*from)?;
                    *to = self.bind_pattern_in(*to, shared, saved, fuel)?;
                }
            }
            P::OptionalPresent(inner) | P::ErrorResult { inner, .. } => {
                self.normalize_pattern(inner, shared, saved, fuel)?;
            }
            P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            } => {
                for field in fields {
                    self.normalize_pattern(field, shared, saved, fuel)?;
                }
            }
            P::Or(alts) => {
                if let Some((first, rest)) = alts.split_first_mut() {
                    let mut common_saved = self.mark();
                    self.normalize_pattern(first, shared, &mut common_saved, fuel)?;
                    let mut bound_order = Vec::new();
                    let mut shared_map: BTreeMap<LocalId, LocalId> =
                        shared.cloned().unwrap_or_default();
                    let mut seen = BTreeSet::new();
                    for (orig, _) in common_saved.iter().rev() {
                        if seen.insert(*orig)
                            && let Some(&mapped) = self.bindings.get(orig)
                        {
                            bound_order.push((*orig, mapped));
                            shared_map.insert(*orig, mapped);
                        }
                    }
                    bound_order.reverse();
                    self.restore(common_saved);
                    for alt in rest {
                        let mut alt_saved = self.mark();
                        self.normalize_pattern(alt, Some(&shared_map), &mut alt_saved, fuel)?;
                        self.restore(alt_saved);
                    }
                    // Expose consistent normalized bindings from the Or pattern to the arm body.
                    for (orig, mapped) in bound_order {
                        let prev = self.bindings.insert(orig, mapped);
                        saved.push((orig, prev));
                    }
                }
            }
            P::CodeEvidence { outputs } => {
                for (id, _) in outputs {
                    *id = self.bind_pattern_in(*id, shared, saved, fuel)?;
                }
            }
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
        }
        Ok(())
    }

    fn normalize_call_arguments(
        &mut self,
        arguments: &mut [CallArgument],
        parameter_bindings: &mut [LocalId],
        fuel: &mut usize,
    ) -> Result<(), String> {
        let mut default_norm = AlphaNormalizer {
            bindings: BTreeMap::new(),
            next_index: self.next_index,
        };
        let mut normalized_params: Vec<Option<LocalId>> = vec![None; parameter_bindings.len()];
        for argument in arguments.iter_mut() {
            if argument.is_default {
                default_norm.next_index = self.next_index;
                default_norm.rewrite_expr(&mut argument.value, fuel)?;
                self.next_index = default_norm.next_index;
            } else {
                self.rewrite_expr(&mut argument.value, fuel)?;
            }
            if let Some(original) = parameter_bindings.get(argument.parameter).copied() {
                if normalized_params[argument.parameter].is_none() {
                    default_norm.next_index = self.next_index;
                    let mapped = default_norm.bind(original, fuel)?;
                    self.next_index = default_norm.next_index;
                    normalized_params[argument.parameter] = Some(mapped);
                }
            }
        }
        for (slot, orig) in parameter_bindings.iter_mut().enumerate() {
            if let Some(mapped) = normalized_params[slot] {
                *orig = mapped;
            } else {
                default_norm.next_index = self.next_index;
                *orig = default_norm.bind(*orig, fuel)?;
                self.next_index = default_norm.next_index;
            }
        }
        Ok(())
    }

    fn normalize_planned_inputs(
        &mut self,
        plan: Option<&mut NormalFormInputPlan>,
        arguments: &mut [NormalFormArgument],
        fuel: &mut usize,
    ) -> Result<(), String> {
        if let Some(plan) = plan {
            self.normalize_scheduled_inputs(
                &mut plan.preparations,
                Some(&plan.order),
                Some(&mut plan.bindings),
                arguments,
                None,
                fuel,
            )
        } else {
            self.normalize_scheduled_inputs(&mut [], None, None, arguments, None, fuel)
        }
    }

    fn normalize_scheduled_inputs(
        &mut self,
        preparations: &mut [NormalFormPreparation],
        order: Option<&[usize]>,
        mut bindings: Option<&mut [LocalId]>,
        arguments: &mut [NormalFormArgument],
        body: Option<&mut Expr>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let count = arguments.len();
        if order.is_some_and(|order| order.len() != count)
            || bindings.as_deref().is_some_and(|ids| ids.len() != count)
        {
            return Err("invalid staged declaration input plan".into());
        }
        let mut saved = self.mark();
        let mut prep_idx = 0usize;
        let mut seen_slots = order.map(|_| vec![false; count]);
        for pos in 0..count {
            while prep_idx < preparations.len() && preparations[prep_idx].before == pos {
                self.normalize_statement(&mut preparations[prep_idx].expression, &mut saved, fuel)?;
                prep_idx += 1;
            }
            let slot = match order {
                Some(order) => {
                    let slot = order[pos];
                    let seen = seen_slots
                        .as_mut()
                        .and_then(|s| s.get_mut(slot))
                        .ok_or("invalid staged declaration input order")?;
                    if *seen {
                        return Err("duplicate staged declaration input slot".into());
                    }
                    *seen = true;
                    slot
                }
                None => pos,
            };
            self.normalize_normal_argument(&mut arguments[slot], fuel)?;
            if let Some(binding) = bindings.as_deref_mut().and_then(|ids| ids.get_mut(slot)) {
                *binding = self.bind_in(*binding, &mut saved, fuel)?;
            }
        }
        while prep_idx < preparations.len() && preparations[prep_idx].before == count {
            self.normalize_statement(&mut preparations[prep_idx].expression, &mut saved, fuel)?;
            prep_idx += 1;
        }
        if prep_idx != preparations.len() {
            return Err("invalid staged declaration input preparation offset".into());
        }
        if let Some(body) = body {
            self.rewrite_expr(body, fuel)?;
        }
        self.restore(saved);
        Ok(())
    }

    fn normalize_normal_argument(
        &mut self,
        arg: &mut NormalFormArgument,
        fuel: &mut usize,
    ) -> Result<(), String> {
        charge(fuel, 1)?;
        match arg {
            NormalFormArgument::List { source, items }
            | NormalFormArgument::Tuple { source, items } => {
                *source = SourceRange { start: 0, end: 0 };
                for item in items {
                    self.normalize_normal_argument(item, fuel)?;
                }
            }
            NormalFormArgument::Value(expr) | NormalFormArgument::ListSpread(expr) => {
                self.rewrite_expr(expr, fuel)?;
            }
            NormalFormArgument::Literal { source, .. } => {
                *source = SourceRange { start: 0, end: 0 };
            }
            NormalFormArgument::If {
                source,
                condition,
                then_input,
                else_input,
            } => {
                *source = SourceRange { start: 0, end: 0 };
                self.rewrite_expr(condition, fuel)?;
                self.normalize_normal_argument(then_input, fuel)?;
                self.normalize_normal_argument(else_input, fuel)?;
            }
            NormalFormArgument::Block {
                source,
                statements,
                result,
            } => {
                *source = SourceRange { start: 0, end: 0 };
                let mut saved = self.mark();
                for statement in statements {
                    self.normalize_statement(statement, &mut saved, fuel)?;
                }
                if let Some(result) = result {
                    self.normalize_normal_argument(result, fuel)?;
                }
                self.restore(saved);
            }
            NormalFormArgument::Match {
                source,
                value,
                arms,
            } => {
                *source = SourceRange { start: 0, end: 0 };
                self.rewrite_expr(value, fuel)?;
                for arm in arms {
                    let mut arm_saved = self.mark();
                    self.normalize_pattern(&mut arm.pattern, None, &mut arm_saved, fuel)?;
                    if let Some(guard) = &mut arm.guard {
                        self.rewrite_expr(guard, fuel)?;
                    }
                    self.normalize_normal_argument(&mut arm.body, fuel)?;
                    self.restore(arm_saved);
                }
            }
        }
        Ok(())
    }
}
