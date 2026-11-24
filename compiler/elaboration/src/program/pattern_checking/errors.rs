//! Identify an error label before binding its payload. The success branch and
//! each distinct applied error type keep their own NIR branch identity.
use super::*;
use crate::program::checking_attempts::Disposition;
use hir::pattern::{Pattern, PatternKind as HP};
use hir::{LitKind, UnOp};

impl Checker<'_, '_> {
    pub(super) fn error_payload_pattern(
        &mut self,
        inner: &Pattern<'_>,
        ty: &Term,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        let Term::ErrorResult { errors, .. } = ty else {
            return Err(error(
                span,
                "error pattern requires an error-qualified input",
            ));
        };
        self.error_payload_pattern_in(inner, ty, errors, span)
    }

    fn error_payload_pattern_in(
        &mut self,
        inner: &Pattern<'_>,
        ty: &Term,
        errors: &[Term],
        span: Span,
    ) -> Result<n::Pattern, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "error pattern budget exhausted"))?;
        match &inner.kind {
            HP::Wild => {
                self.required_steps = self
                    .required_steps
                    .checked_sub(errors.len() as u64)
                    .ok_or_else(|| error(span, "error pattern budget exhausted"))?;
                Ok(n::Pattern::Or(
                    errors
                        .iter()
                        .map(|label| n::Pattern::ErrorResult {
                            label: Some(label.clone()),
                            inner: Box::new(n::Pattern::Wild),
                        })
                        .collect(),
                ))
            }
            HP::Or(patterns) => {
                self.required_steps = self
                    .required_steps
                    .checked_sub(patterns.len() as u64)
                    .ok_or_else(|| error(span, "error pattern budget exhausted"))?;
                let saved = self.locals.clone();
                let mut alternatives = Vec::new();
                let mut canonical = None;
                let mut exported = saved.clone();
                for alt in *patterns {
                    self.locals = saved.clone();
                    let mut checked = self.error_payload_pattern_in(alt, ty, errors, alt.span)?;
                    let bindings = pattern_bindings(alt)?;
                    if let Some(first) = &canonical {
                        let first: &BTreeMap<String, n::LocalId> = first;
                        if !first.keys().eq(bindings.keys()) {
                            self.locals = saved;
                            return Err(error(
                                alt.span,
                                "or-pattern branches must export the same binding names",
                            ));
                        }
                        let mut remap = BTreeMap::new();
                        for (name, binding) in &bindings {
                            let target = first[name];
                            self.require(&self.locals[binding], &exported[&target], alt.span)?;
                            remap.insert(*binding, target);
                        }
                        checked = n::Pattern::Remap {
                            pattern: Box::new(checked),
                            bindings: remap.into_iter().collect(),
                        };
                    } else {
                        canonical = Some(bindings);
                        exported = self.locals.clone();
                    }
                    alternatives.push(checked);
                }
                self.locals = exported;
                Ok(n::Pattern::Or(alternatives))
            }
            HP::Guard(first, condition) => {
                let first = self.error_payload_pattern_in(first, ty, errors, first.span)?;
                let condition = self.under_observed_pattern_type_facts(
                    &first,
                    None,
                    condition.span,
                    |checker| checker.expr(condition, Some(&p(P::Bool))),
                )?;
                match first {
                    n::Pattern::ErrorResult { label, inner } => Ok(n::Pattern::ErrorResult {
                        label,
                        inner: Box::new(n::Pattern::Guard {
                            pattern: inner,
                            condition: Box::new(condition),
                        }),
                    }),
                    other => Ok(n::Pattern::Guard {
                        pattern: Box::new(other),
                        condition: Box::new(condition),
                    }),
                }
            }
            HP::Then(first, value, next) => {
                let first = self.error_payload_pattern_in(first, ty, errors, first.span)?;
                let (value, next) =
                    self.under_observed_pattern_type_facts(&first, None, value.span, |checker| {
                        let value = checker.expr(value, None)?;
                        let observed = checker.known_pattern_type_value(&value, next.span)?;
                        let next = checker.pattern_observed(next, &value.ty, observed.as_ref())?;
                        Ok((value, next))
                    })?;
                match first {
                    n::Pattern::ErrorResult { label, inner } => {
                        let lowered = self.lower_computed_pattern(*inner, value, next, span)?;
                        Ok(n::Pattern::ErrorResult {
                            label,
                            inner: Box::new(lowered),
                        })
                    }
                    other => self.lower_computed_pattern(other, value, next, span),
                }
            }
            HP::Binding(hir::common::BindingMode::ByValue, _, Some(first)) if errors.len() > 1 => {
                let saved = self.locals.clone();
                let probed = self.error_payload_pattern_in(first, ty, errors, first.span)?;
                let Some(label) = self.single_error_label(&probed, first.span) else {
                    self.locals = saved;
                    return Err(error(
                        span,
                        "multiple error types require a type-identifying payload pattern",
                    ));
                };
                let label = label.clone();
                self.locals = saved;
                Ok(n::Pattern::ErrorResult {
                    label: Some(label.clone()),
                    inner: Box::new(self.pattern_with_record_head(inner, &label, None, None)?),
                })
            }
            _ => {
                let (label, record) = if errors.len() == 1 {
                    (errors[0].clone(), None)
                } else {
                    self.select_multi_error_label(inner, errors, span)?
                };
                Ok(n::Pattern::ErrorResult {
                    label: Some(label.clone()),
                    inner: Box::new(self.pattern_with_record_head(inner, &label, None, record)?),
                })
            }
        }
    }

    fn single_error_label<'a>(&self, pat: &'a n::Pattern, span: Span) -> Option<&'a Term> {
        match pat {
            n::Pattern::ErrorResult { label, .. } => label.as_ref(),
            n::Pattern::Binding { inner, .. }
            | n::Pattern::Guard { pattern: inner, .. }
            | n::Pattern::Then { first: inner, .. }
            | n::Pattern::Remap { pattern: inner, .. } => self.single_error_label(inner, span),
            n::Pattern::Or(alts) => {
                let mut chosen: Option<&'a Term> = None;
                for alt in alts {
                    let label = self.single_error_label(alt, span)?;
                    if let Some(prev) = chosen {
                        if self.require_type_identity(label, prev, span).is_err() {
                            return None;
                        }
                    } else {
                        chosen = Some(label);
                    }
                }
                chosen
            }
            _ => None,
        }
    }

    fn find_error_row_member(
        &self,
        label: &Term,
        errors: &[Term],
        span: Span,
    ) -> Result<Term, Error> {
        self.required_steps
            .checked_sub(errors.len() as u64)
            .ok_or_else(|| error(span, "error pattern budget exhausted"))?;
        errors
            .iter()
            .find(|allowed| self.require_type_identity(label, allowed, span).is_ok())
            .cloned()
            .ok_or_else(|| {
                error(
                    span,
                    "error pattern type is absent from the input error row",
                )
            })
    }

    fn pick_unique_error_match(
        &self,
        matches: Vec<usize>,
        errors: &[Term],
        span: Span,
    ) -> Result<Term, Error> {
        match matches.as_slice() {
            [] => Err(error(
                span,
                "error pattern type is absent from the input error row",
            )),
            [index] => Ok(errors[*index].clone()),
            _ => Err(error(
                span,
                "ambiguous error pattern matches multiple error types in the input error row",
            )),
        }
    }

    fn select_multi_error_label(
        &mut self,
        selected: &Pattern<'_>,
        errors: &[Term],
        span: Span,
    ) -> Result<(Term, Option<records::RecordHead>), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(errors.len() as u64)
            .ok_or_else(|| error(selected.span, "error pattern budget exhausted"))?;
        match &selected.kind {
            HP::Binding(hir::common::BindingMode::ByValue, _, None) => Err(error(
                span,
                "multiple error types require a type-identifying payload pattern",
            )),
            HP::Struct(head, _, _) => {
                if matches!(head.kind, HP::Wild) {
                    let mut matches = Vec::new();
                    for (idx, allowed) in errors.iter().enumerate() {
                        if self.pattern_compatible_with_type(selected, allowed, selected.span)? {
                            matches.push(idx);
                        }
                    }
                    let label = self.pick_unique_error_match(matches, errors, selected.span)?;
                    return Ok((label, None));
                }
                let HP::Const(callee) = &head.kind else {
                    return Err(error(
                        span,
                        "error payload requires a type-identifying pattern",
                    ));
                };
                let record = self.named_record_head(callee, None)?;
                let label = self.find_error_row_member(&record.ty, errors, selected.span)?;
                Ok((label, Some(record)))
            }
            HP::AppTuple(head, _) => {
                let HP::Const(callee) = &head.kind else {
                    return Err(error(
                        span,
                        "error payload requires a type-identifying pattern",
                    ));
                };
                if !self.is_type_head(callee)
                    && let Some((target, _)) = self.variant_target(callee, None)?
                {
                    let label = self.find_error_row_member(&target, errors, selected.span)?;
                    return Ok((label, None));
                }
                if let Some((target, _)) = self.newtype_target(callee, None)? {
                    let label = self.find_error_row_member(&target, errors, selected.span)?;
                    return Ok((label, None));
                }
                if let Some(decl) = self.newtype_declaration(callee) {
                    let mut matches = Vec::new();
                    for (idx, allowed) in errors.iter().enumerate() {
                        let view = self.type_identity_view(allowed, selected.span)?;
                        if matches!(view, Term::Nominal { declaration, .. } if declaration == decl)
                        {
                            matches.push(idx);
                        }
                    }
                    let label = self.pick_unique_error_match(matches, errors, selected.span)?;
                    return Ok((label, None));
                }
                Err(error(
                    selected.span,
                    "multiple error types require a qualified variant pattern",
                ))
            }
            HP::Tuple(fields) => {
                let mut compatible = Vec::new();
                let mut same_arity = Vec::new();
                for (idx, allowed) in errors.iter().enumerate() {
                    let shape = self.structural_type(allowed, selected.span)?;
                    if matches!(&shape, Term::Tuple(elems) if elems.len() == fields.len()) {
                        same_arity.push(idx);
                        if self.pattern_compatible_with_type(selected, allowed, selected.span)? {
                            compatible.push(idx);
                        }
                    }
                }
                let label = if compatible.is_empty() && same_arity.len() == 1 {
                    errors[same_arity[0]].clone()
                } else {
                    self.pick_unique_error_match(compatible, errors, selected.span)?
                };
                Ok((label, None))
            }
            HP::Const(expr) => {
                if !self.is_type_head(expr)
                    && let Some((target, _)) = self.variant_target(expr, None)?
                {
                    let label = self.find_error_row_member(&target, errors, selected.span)?;
                    return Ok((label, None));
                }
                let matches = self.const_expr_error_matches(expr, errors, selected.span)?;
                let label = self.pick_unique_error_match(matches, errors, selected.span)?;
                Ok((label, None))
            }
            HP::Range(lower, upper, _) if lower.is_some() || upper.is_some() => {
                let mut matches = Vec::new();
                for (idx, allowed) in errors.iter().enumerate() {
                    if self.pattern_compatible_with_type(selected, allowed, selected.span)? {
                        matches.push(idx);
                    }
                }
                let label = self.pick_unique_error_match(matches, errors, selected.span)?;
                Ok((label, None))
            }
            HP::OptionNull | HP::OptionSome(_) | HP::List(..) => {
                let mut matches = Vec::new();
                for (idx, allowed) in errors.iter().enumerate() {
                    if self.pattern_compatible_with_type(selected, allowed, selected.span)? {
                        matches.push(idx);
                    }
                }
                let label = self.pick_unique_error_match(matches, errors, selected.span)?;
                Ok((label, None))
            }
            _ => Err(error(
                span,
                "multiple error types require a type-identifying payload pattern",
            )),
        }
    }

    fn const_expr_error_matches(
        &mut self,
        expr: &Expr<'_>,
        errors: &[Term],
        span: Span,
    ) -> Result<Vec<usize>, Error> {
        if let Some(first_match) = self.syntactic_literal_compatible(expr, &errors[0], span)? {
            let mut matches = Vec::new();
            if first_match {
                matches.push(0);
            }
            for (idx, allowed) in errors.iter().enumerate().skip(1) {
                if self.syntactic_literal_compatible(expr, allowed, span)? == Some(true) {
                    matches.push(idx);
                }
            }
            return Ok(matches);
        }
        let unguided = self.checking_attempt(span, false, Disposition::Probe, |this| {
            this.expr(expr, None)
        })?;
        if let Some(probed) = unguided {
            let mut matches = Vec::new();
            for (idx, allowed) in errors.iter().enumerate() {
                if self
                    .require_type_identity(&probed.ty, allowed, span)
                    .is_ok()
                {
                    matches.push(idx);
                }
            }
            return Ok(matches);
        }
        let mut matches = Vec::new();
        for (idx, allowed) in errors.iter().enumerate() {
            let view = self.type_identity_view(allowed, span)?;
            if matches!(view, Term::Rigid { .. }) {
                continue;
            }
            match self.checking_attempt(span, false, Disposition::Probe, |this| {
                this.expr(expr, Some(allowed))
            }) {
                Ok(Some(_)) => matches.push(idx),
                Ok(None) => {}
                Err(err)
                    if !err.message.contains("budget") && !err.message.contains("exhausted") => {}
                Err(err) => return Err(err),
            }
        }
        Ok(matches)
    }

    fn syntactic_literal_compatible(
        &self,
        expr: &Expr<'_>,
        ty: &Term,
        span: Span,
    ) -> Result<Option<bool>, Error> {
        let shape = self.structural_type(ty, span)?;
        if matches!(expr.kind, H::Unit) {
            return Ok(Some(
                shape == p(P::Void) || matches!(&shape, Term::Tuple(elems) if elems.is_empty()),
            ));
        }
        let (lit, negative) = match &expr.kind {
            H::Lit(lit) => (lit, false),
            H::Unary(UnOp::Neg, inner) => match &inner.kind {
                H::Lit(lit) if matches!(lit.kind, LitKind::Integer(_) | LitKind::Float(_)) => {
                    (lit, true)
                }
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let compatible = match &lit.kind {
            LitKind::Bool(_) => shape == p(P::Bool),
            LitKind::Char(_) => shape == p(P::Char),
            LitKind::String(_) => shape == p(P::Str),
            LitKind::Symbol(_) => shape == p(P::Symbol),
            LitKind::Float(_) => matches!(
                shape,
                Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64)
            ),
            LitKind::Integer(spelling) => match shape {
                Term::Primitive(primitive)
                    if matches!(
                        primitive,
                        P::Integer
                            | P::U8
                            | P::U16
                            | P::U32
                            | P::U64
                            | P::U128
                            | P::I8
                            | P::I16
                            | P::I32
                            | P::I64
                            | P::I128
                            | P::Usize
                            | P::Isize
                    ) =>
                {
                    nir::integer_formation::form(
                        spelling,
                        negative,
                        primitive,
                        self.target,
                        &mut comptime_runtime::Budget::default(),
                    )
                    .is_ok()
                }
                Term::Primitive(P::BitVec(width)) => {
                    let mut budget = comptime_runtime::Budget::default();
                    comptime_runtime::Integer::parse_literal(spelling, &mut budget)
                        .ok()
                        .and_then(|val| {
                            let val = if negative { val.negated() } else { val };
                            val.to_bitvec(width, &mut budget).ok()
                        })
                        .is_some()
                }
                _ => false,
            },
        };
        Ok(Some(compatible))
    }

    fn pattern_compatible_with_type(
        &mut self,
        pat: &Pattern<'_>,
        ty: &Term,
        span: Span,
    ) -> Result<bool, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "error pattern budget exhausted"))?;
        let shape = self.structural_type(ty, span)?;
        match &pat.kind {
            HP::Wild | HP::Binding(_, _, None) => Ok(true),
            HP::Binding(_, _, Some(inner)) | HP::Guard(inner, _) | HP::Then(inner, _, _) => {
                self.pattern_compatible_with_type(inner, ty, span)
            }
            HP::Or(alts) => {
                for alt in *alts {
                    if !self.pattern_compatible_with_type(alt, ty, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            HP::Tuple(fields) => {
                let Term::Tuple(elems) = &shape else {
                    return Ok(false);
                };
                if fields.len() != elems.len() {
                    return Ok(false);
                }
                for (field, elem_ty) in fields.iter().zip(elems) {
                    if !self.pattern_compatible_with_type(field, elem_ty, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            HP::OptionNull => Ok(matches!(shape, Term::Optional(_))),
            HP::OptionSome(inner) => {
                let Term::Optional(payload) = &shape else {
                    return Ok(false);
                };
                self.pattern_compatible_with_type(inner, payload, span)
            }
            HP::List(elements, _) => {
                let Some(elem_ty) = self.list_element_type(&shape).cloned() else {
                    return Ok(false);
                };
                for elem in *elements {
                    if !self.pattern_compatible_with_type(elem, &elem_ty, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            HP::Range(lower, upper, _) => {
                if lower.is_none() && upper.is_none() {
                    return Ok(true);
                }
                let ordered = matches!(
                    &shape,
                    Term::Primitive(primitive)
                        if matches!(
                            primitive,
                            P::Integer
                                | P::Char
                                | P::Float
                                | P::F16
                                | P::Bf16
                                | P::F32
                                | P::F64
                                | P::Usize
                                | P::Isize
                        ) || self.integer_layout(*primitive).is_some()
                );
                if !ordered {
                    return Ok(false);
                }
                for endpoint in [lower, upper].into_iter().flatten() {
                    if !self.const_expr_compatible_with_type(endpoint, ty, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            HP::Struct(head, fields, _) => {
                if matches!(head.kind, HP::Wild) {
                    let declared = match &shape {
                        Term::Record(rec_fields) => rec_fields.clone(),
                        Term::Nominal { declaration, .. }
                            if self.structs.contains_key(declaration) =>
                        {
                            self.instantiated_fields(&shape, span)?
                        }
                        _ => return Ok(false),
                    };
                    for field in *fields {
                        let Some((_, field_ty)) = declared
                            .iter()
                            .find(|(name, _)| name == field.ident.name.as_str())
                        else {
                            return Ok(false);
                        };
                        if !self.pattern_compatible_with_type(&field.pat, field_ty, span)? {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                let HP::Const(callee) = &head.kind else {
                    return Ok(false);
                };
                match self.checking_attempt(span, false, Disposition::Probe, |this| {
                    let head = this.named_record_head(callee, Some(ty))?;
                    this.require_type_identity(&head.ty, ty, span)?;
                    Ok(n::Expr {
                        source: source(span),
                        ty: head.ty,
                        kind: K::Unit,
                    })
                }) {
                    Ok(Some(_)) => Ok(true),
                    Ok(None) => Ok(false),
                    Err(err)
                        if !err.message.contains("budget")
                            && !err.message.contains("exhausted") =>
                    {
                        Ok(false)
                    }
                    Err(err) => Err(err),
                }
            }
            HP::AppTuple(head, fields) => {
                let HP::Const(callee) = &head.kind else {
                    return Ok(false);
                };
                if !self.is_type_head(callee)
                    && matches!(&shape, Term::Nominal { declaration, .. } if self.enums.contains_key(declaration))
                    && let Ok(Some((variant_ty, _))) = self.variant_target(callee, Some(ty))
                {
                    return Ok(self.require_type_identity(&variant_ty, ty, span).is_ok());
                }
                if let Some((newtype_ty, repr)) = self.newtype_target(callee, Some(ty))? {
                    return Ok(fields.len() == 1
                        && self.require_type_identity(&newtype_ty, ty, span).is_ok()
                        && self.pattern_compatible_with_type(&fields[0], &repr, span)?);
                }
                Ok(false)
            }
            HP::Const(expr) => self.const_expr_compatible_with_type(expr, ty, span),
            _ => Ok(false),
        }
    }

    fn const_expr_compatible_with_type(
        &mut self,
        expr: &Expr<'_>,
        ty: &Term,
        span: Span,
    ) -> Result<bool, Error> {
        if !self.is_type_head(expr)
            && matches!(self.structural_type(ty, span)?, Term::Nominal { declaration, .. } if self.enums.contains_key(&declaration))
            && let Ok(Some((variant_ty, _))) = self.variant_target(expr, Some(ty))
        {
            return Ok(self.require_type_identity(&variant_ty, ty, span).is_ok());
        }
        if let Some(matched) = self.syntactic_literal_compatible(expr, ty, span)? {
            return Ok(matched);
        }
        let unguided = self.checking_attempt(span, false, Disposition::Probe, |this| {
            this.expr(expr, None)
        })?;
        if let Some(probed) = unguided {
            return Ok(self.require_type_identity(&probed.ty, ty, span).is_ok());
        }
        let view = self.type_identity_view(ty, span)?;
        if matches!(view, Term::Rigid { .. }) {
            return Ok(false);
        }
        match self.checking_attempt(span, false, Disposition::Probe, |this| {
            this.expr(expr, Some(ty))
        }) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(err) if !err.message.contains("budget") && !err.message.contains("exhausted") => {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }
}
