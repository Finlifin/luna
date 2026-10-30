//! Error elimination is ordinary result matching, never effect installation.
use super::*;
mod catches;
mod layouts;

impl Checker<'_, '_> {
    fn error_node(&mut self, ty: Term, kind: K, span: Span) -> Result<n::Expr, Error> {
        Ok(n::Expr {
            source: self.error_generated_source(span)?,
            ty,
            kind,
        })
    }

    /// Match the restored abstract payload before evaluating its inner pattern.
    /// Rejection transfers the same value into the remaining ordered alternatives;
    /// it never re-evaluates the input or bypasses candidate observation rules.
    pub(super) fn stabilize_error_success_patterns(
        &mut self,
        arms: &mut [n::MatchArm],
        input: &Term,
        output: &Term,
        span: Span,
    ) -> Result<(), Error> {
        let Term::ErrorResult { result, .. } = input else {
            return Ok(());
        };
        if !matches!(
            result.as_ref(),
            Term::Rigid { .. }
                | Term::Bound { .. }
                | Term::Hole { .. }
                | Term::AssociatedType { .. }
                | Term::NormalForm { .. }
        ) {
            return Ok(());
        }
        for index in (0..arms.len()).rev() {
            // Simple unguarded bindings already have a single initializer.
            if arms[index].guard.is_none()
                && matches!(&arms[index].pattern,
                n::Pattern::ErrorResult { label: None, inner }
                    if matches!(inner.as_ref(), n::Pattern::Wild)
                        || matches!(inner.as_ref(), n::Pattern::Binding { inner, .. }
                            if matches!(inner.as_ref(), n::Pattern::Wild)))
            {
                continue;
            }
            let Some(pattern) =
                self.restored_error_success_pattern(&arms[index].pattern, span, 0)?
            else {
                continue;
            };
            let guarded = n::MatchArm {
                pattern,
                guard: arms[index].guard.clone(),
                body: arms[index].body.clone(),
            };
            let raw = self.spread_binding(span)?;
            let restored = self.spread_binding(span)?;
            let fallback = self.spread_binding(span)?;
            let bind = |binding| n::Pattern::Binding {
                binding,
                inner: Box::new(n::Pattern::Wild),
            };
            let rejected = self.error_node((**result).clone(), K::Local(fallback), span)?;
            let rejected = self.error_delivery(rejected, input, span)?;
            let mut remaining = self.error_node(
                output.clone(),
                K::Match {
                    value: Box::new(rejected),
                    arms: arms[index + 1..].to_vec(),
                },
                span,
            )?;
            let allowance = usize::try_from(self.required_steps)
                .unwrap_or(usize::MAX)
                .min(100_000);
            let mut fuel = allowance;
            let anchored = nir::error_results::specialize::anchor_expression_copy(
                &mut remaining,
                source(span),
                &mut self.expansion_sources,
                &mut fuel,
            );
            self.required_steps -= (allowance - fuel) as u64;
            anchored.map_err(|message| error(span, message))?;
            let subject = self.error_node((**result).clone(), K::Local(restored), span)?;
            let selected = self.error_node(
                output.clone(),
                K::Match {
                    value: Box::new(subject),
                    arms: vec![
                        guarded,
                        n::MatchArm {
                            pattern: bind(fallback),
                            guard: None,
                            body: remaining,
                        },
                    ],
                },
                span,
            )?;
            let initializer = self.error_node((**result).clone(), K::Local(raw), span)?;
            let statement = self.error_node(
                p(P::Void),
                K::LetPattern {
                    pattern: bind(restored),
                    value: Box::new(initializer),
                    guard: None,
                    otherwise: None,
                },
                span,
            )?;
            let body = self.error_node(
                output.clone(),
                K::Block {
                    statements: vec![statement],
                    result: Some(Box::new(selected)),
                },
                span,
            )?;
            arms[index] = n::MatchArm {
                pattern: n::Pattern::ErrorResult {
                    label: None,
                    inner: Box::new(bind(raw)),
                },
                guard: None,
                body,
            };
        }
        Ok(())
    }

    /// Lift only patterns whose subject is entirely the success domain. Outer
    /// whole-value aliases and mixed success/error alternatives retain their
    /// original subject; stripping their qualifier would change binding types.
    fn restored_error_success_pattern(
        &mut self,
        pattern: &n::Pattern,
        span: Span,
        depth: usize,
    ) -> Result<Option<n::Pattern>, Error> {
        if depth >= 128 {
            return Err(error(span, "error success pattern depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "error success pattern budget exhausted"))?;
        Ok(match pattern {
            n::Pattern::ErrorResult { label: None, inner } => Some((**inner).clone()),
            n::Pattern::Guard { pattern, condition } => self
                .restored_error_success_pattern(pattern, span, depth + 1)?
                .map(|pattern| n::Pattern::Guard {
                    pattern: Box::new(pattern),
                    condition: condition.clone(),
                }),
            n::Pattern::Remap { pattern, bindings } => self
                .restored_error_success_pattern(pattern, span, depth + 1)?
                .map(|pattern| n::Pattern::Remap {
                    pattern: Box::new(pattern),
                    bindings: bindings.clone(),
                }),
            n::Pattern::Then { first, value, next } => self
                .restored_error_success_pattern(first, span, depth + 1)?
                .map(|first| n::Pattern::Then {
                    first: Box::new(first),
                    value: value.clone(),
                    next: next.clone(),
                }),
            n::Pattern::Or(alternatives) => {
                let mut restored = Vec::new();
                for alternative in alternatives {
                    let Some(pattern) =
                        self.restored_error_success_pattern(alternative, span, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    restored.push(pattern);
                }
                Some(n::Pattern::Or(restored))
            }
            _ => None,
        })
    }

    fn can_delay_success_bindings(
        &mut self,
        pattern: &n::Pattern,
        span: Span,
    ) -> Result<bool, Error> {
        let mut pending = vec![pattern];
        while let Some(pattern) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "error binding observation budget exhausted"))?;
            match pattern {
                n::Pattern::Guard { .. }
                | n::Pattern::Then { .. }
                | n::Pattern::CodeEvidence { .. } => return Ok(false),
                n::Pattern::Binding { inner, .. }
                | n::Pattern::Observe { inner, .. }
                | n::Pattern::OptionalPresent(inner)
                | n::Pattern::ErrorResult { inner, .. }
                | n::Pattern::Remap { pattern: inner, .. } => pending.push(inner),
                n::Pattern::Tuple(fields)
                | n::Pattern::TypeTuple(fields)
                | n::Pattern::Or(fields)
                | n::Pattern::NominalType { fields, .. }
                | n::Pattern::Record { fields, .. }
                | n::Pattern::AnonymousRecord { fields, .. }
                | n::Pattern::Variant { fields, .. }
                | n::Pattern::NormalForm {
                    arguments: fields, ..
                } => pending.extend(fields),
                n::Pattern::OptionalAbsent
                | n::Pattern::Wild
                | n::Pattern::Unit
                | n::Pattern::Literal(_) => {}
            }
        }
        Ok(true)
    }

    pub(super) fn checked_pattern_exports(
        &mut self,
        pattern: &n::Pattern,
        span: Span,
        depth: usize,
    ) -> Result<Option<BTreeSet<n::LocalId>>, Error> {
        if depth >= 128 {
            return Err(error(span, "pattern export depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "pattern export budget exhausted"))?;
        let mut exports = BTreeSet::new();
        match pattern {
            n::Pattern::Binding { binding, inner } => {
                let Some(nested) = self.checked_pattern_exports(inner, span, depth + 1)? else {
                    return Ok(None);
                };
                exports.extend(nested);
                exports.insert(*binding);
            }
            n::Pattern::Guard { pattern, .. } => {
                return self.checked_pattern_exports(pattern, span, depth + 1);
            }
            n::Pattern::Then { first, next, .. } => {
                let Some(first) = self.checked_pattern_exports(first, span, depth + 1)? else {
                    return Ok(None);
                };
                let Some(next) = self.checked_pattern_exports(next, span, depth + 1)? else {
                    return Ok(None);
                };
                exports.extend(first);
                exports.extend(next);
            }
            n::Pattern::Remap { pattern, bindings } => {
                let Some(inner) = self.checked_pattern_exports(pattern, span, depth + 1)? else {
                    return Ok(None);
                };
                if bindings.iter().any(|(source, _)| !inner.contains(source)) {
                    return Ok(None);
                }
                exports.extend(bindings.iter().map(|(_, target)| *target));
            }
            n::Pattern::Or(alternatives) => {
                let mut canonical = None;
                for alternative in alternatives {
                    let Some(branch) =
                        self.checked_pattern_exports(alternative, span, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    if canonical
                        .as_ref()
                        .is_some_and(|expected| expected != &branch)
                    {
                        return Ok(None);
                    }
                    canonical = Some(branch);
                }
                exports = canonical.unwrap_or_default();
            }
            n::Pattern::OptionalPresent(inner) | n::Pattern::ErrorResult { inner, .. } => {
                return self.checked_pattern_exports(inner, span, depth + 1);
            }
            n::Pattern::Tuple(fields)
            | n::Pattern::TypeTuple(fields)
            | n::Pattern::AnonymousRecord { fields, .. }
            | n::Pattern::Record { fields, .. }
            | n::Pattern::Variant { fields, .. }
            | n::Pattern::NominalType { fields, .. }
            | n::Pattern::NormalForm {
                arguments: fields, ..
            } => {
                for field in fields {
                    let Some(nested) = self.checked_pattern_exports(field, span, depth + 1)? else {
                        return Ok(None);
                    };
                    exports.extend(nested);
                }
            }
            n::Pattern::OptionalAbsent
            | n::Pattern::Wild
            | n::Pattern::Unit
            | n::Pattern::Literal(_) => {}
            _ => return Ok(None),
        }
        Ok(Some(exports))
    }

    fn stabilize_guarded_error_let(
        &mut self,
        statement: &mut n::Expr,
        span: Span,
    ) -> Result<bool, Error> {
        let K::LetPattern {
            pattern,
            value,
            otherwise: Some(_),
            ..
        } = &statement.kind
        else {
            return Ok(false);
        };
        let Term::ErrorResult { result, .. } = &value.ty else {
            return Ok(false);
        };
        if !matches!(
            result.as_ref(),
            Term::Rigid { .. }
                | Term::Bound { .. }
                | Term::Hole { .. }
                | Term::AssociatedType { .. }
                | Term::NormalForm { .. }
        ) {
            return Ok(false);
        }
        let Some(restored) = self.restored_error_success_pattern(pattern, span, 0)? else {
            return Ok(false);
        };
        let Some(originals) = self.checked_pattern_exports(&restored, span, 0)? else {
            return Ok(false);
        };
        if originals.is_empty() {
            return Ok(false);
        }
        let mut exports = Vec::new();
        for original in originals {
            let Some(ty) = self.locals.get(&original).cloned() else {
                return Ok(false);
            };
            exports.push((original, self.spread_binding(span)?, ty));
        }
        // This is an internal ownership-transfer packet, not the language's
        // Tuple normal-form identity. Each exported value is moved exactly once.
        let target = if exports.len() == 1 {
            exports[0].2.clone()
        } else {
            Term::Tuple(exports.iter().map(|(_, _, ty)| ty.clone()).collect())
        };
        let mut values = Vec::new();
        for (_, binding, ty) in &exports {
            values.push(self.error_node(ty.clone(), K::Local(*binding), span)?);
        }
        let body = if values.len() == 1 {
            values.pop().unwrap()
        } else {
            self.error_node(target.clone(), K::Tuple(values), span)?
        };
        let K::LetPattern {
            mut pattern,
            value,
            guard,
            otherwise: Some(otherwise),
        } = std::mem::replace(&mut statement.kind, K::Unit)
        else {
            unreachable!()
        };
        if let Some(condition) = guard {
            pattern = n::Pattern::Guard {
                pattern: Box::new(pattern),
                condition,
            };
        }
        let input = value.ty.clone();
        let mut arms = vec![
            n::MatchArm {
                pattern: n::Pattern::Remap {
                    pattern: Box::new(pattern),
                    bindings: exports
                        .iter()
                        .map(|(original, exported, _)| (*original, *exported))
                        .collect(),
                },
                guard: None,
                body,
            },
            n::MatchArm {
                pattern: n::Pattern::Wild,
                guard: None,
                body: *otherwise,
            },
        ];
        self.stabilize_error_success_patterns(&mut arms, &input, &target, span)?;
        let initializer = self.error_node(target, K::Match { value, arms }, span)?;
        statement.kind = if exports.len() == 1 {
            K::Let {
                binding: exports[0].0,
                value: Box::new(initializer),
            }
        } else {
            K::LetPattern {
                pattern: n::Pattern::Tuple(
                    exports
                        .iter()
                        .map(|(binding, _, _)| n::Pattern::Binding {
                            binding: *binding,
                            inner: Box::new(n::Pattern::Wild),
                        })
                        .collect(),
                ),
                value: Box::new(initializer),
                guard: None,
                otherwise: None,
            }
        };
        Ok(true)
    }

    /// Let bindings must remain in their containing block, including after a
    /// diverging else. Append restoration statements in that same scope rather
    /// than hiding the exported bindings in a nested block.
    pub(super) fn stabilize_error_let_bindings(
        &mut self,
        statements: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<(), Error> {
        let mut lowered = Vec::with_capacity(statements.len());
        for mut statement in std::mem::take(statements) {
            if let K::LetPattern { pattern, guard, .. } = &statement.kind {
                if (guard.is_some() || !self.can_delay_success_bindings(pattern, span)?)
                    && self.stabilize_guarded_error_let(&mut statement, span)?
                {
                    lowered.push(statement);
                    continue;
                }
            }
            let mut initializers = Vec::new();
            if let K::LetPattern {
                pattern,
                value,
                guard: None,
                ..
            } = &mut statement.kind
            {
                if !is_never(&value.ty) {
                    if self.can_delay_success_bindings(pattern, span)? {
                        self.stabilize_structural_success_bindings(
                            pattern,
                            &value.ty,
                            &mut initializers,
                            span,
                            0,
                        )?;
                    }
                }
            }
            lowered.push(statement);
            lowered.extend(initializers);
        }
        *statements = lowered;
        Ok(())
    }

    /// Extract nested success bindings only after the whole candidate accepts.
    /// Each initializer matches its own original field domain, so specialization
    /// can restore the abstract payload without changing sibling patterns.
    fn stabilize_structural_success_bindings(
        &mut self,
        pattern: &mut n::Pattern,
        domain: &Term,
        statements: &mut Vec<n::Expr>,
        span: Span,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= 128 {
            return Err(error(span, "nested error pattern depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "nested error pattern budget exhausted"))?;
        if let n::Pattern::Tuple(fields) = pattern {
            if let Some(types) = self.bound_tuple_elements(domain).map(<[Term]>::to_vec) {
                if fields.len() != types.len() {
                    return Err(error(span, "nested tuple pattern layout mismatch"));
                }
                for (field, ty) in fields.iter_mut().zip(&types) {
                    self.stabilize_structural_success_bindings(
                        field,
                        ty,
                        statements,
                        span,
                        depth + 1,
                    )?;
                    if matches!(ty, Term::ErrorResult { result, .. } if matches!(result.as_ref(),
                        Term::Rigid { .. } | Term::Bound { .. } | Term::Hole { .. }
                        | Term::AssociatedType { .. } | Term::NormalForm { .. }))
                    {
                        // A computed-pattern identity retains the checked field
                        // domain across generic substitution. Rewriting cannot
                        // derive this view by unfolding an opaque Tuple head.
                        let binding = self.spread_binding(span)?;
                        let next = std::mem::replace(field, n::Pattern::Wild);
                        *field = n::Pattern::Then {
                            first: Box::new(n::Pattern::Observe {
                                binding,
                                inner: Box::new(n::Pattern::Wild),
                            }),
                            value: Box::new(self.error_node(
                                ty.clone(),
                                K::Local(binding),
                                span,
                            )?),
                            next: Box::new(next),
                        };
                    }
                }
                return Ok(());
            }
        }
        match (pattern, domain) {
            (n::Pattern::Tuple(fields), Term::Tuple(types)) if fields.len() == types.len() => {
                for (field, ty) in fields.iter_mut().zip(types) {
                    self.stabilize_structural_success_bindings(
                        field,
                        ty,
                        statements,
                        span,
                        depth + 1,
                    )?;
                }
            }
            (n::Pattern::AnonymousRecord { names, fields }, Term::Record(types)) => {
                for (name, field) in names.iter().zip(fields) {
                    if let Some((_, ty)) = types.iter().find(|(key, _)| key == name) {
                        self.stabilize_structural_success_bindings(
                            field,
                            ty,
                            statements,
                            span,
                            depth + 1,
                        )?;
                    }
                }
            }
            (pattern @ n::Pattern::ErrorResult { .. }, Term::ErrorResult { result, .. })
                if matches!(
                    result.as_ref(),
                    Term::Rigid { .. }
                        | Term::Bound { .. }
                        | Term::Hole { .. }
                        | Term::AssociatedType { .. }
                        | Term::NormalForm { .. }
                ) =>
            {
                let n::Pattern::ErrorResult { label: None, inner } = pattern else {
                    return Ok(());
                };
                let n::Pattern::Binding { binding, inner } = inner.as_ref() else {
                    return Ok(());
                };
                if !matches!(inner.as_ref(), n::Pattern::Wild) {
                    return Ok(());
                }
                let original = *binding;
                let whole = self.spread_binding(span)?;
                let raw = self.spread_binding(span)?;
                let bind = |binding| n::Pattern::Binding {
                    binding,
                    inner: Box::new(n::Pattern::Wild),
                };
                let input = self.error_node(domain.clone(), K::Local(whole), span)?;
                let payload = self.error_node((**result).clone(), K::Local(raw), span)?;
                let initializer = self.error_node(
                    (**result).clone(),
                    K::Match {
                        value: Box::new(input),
                        arms: vec![n::MatchArm {
                            pattern: n::Pattern::ErrorResult {
                                label: None,
                                inner: Box::new(bind(raw)),
                            },
                            guard: None,
                            body: payload,
                        }],
                    },
                    span,
                )?;
                statements.push(self.error_node(
                    p(P::Void),
                    K::LetPattern {
                        pattern: bind(original),
                        value: Box::new(initializer),
                        guard: None,
                        otherwise: None,
                    },
                    span,
                )?);
                *pattern = n::Pattern::Binding {
                    binding: whole,
                    inner: Box::new(n::Pattern::ErrorResult {
                        label: None,
                        inner: Box::new(n::Pattern::Wild),
                    }),
                };
            }
            _ => {}
        }
        Ok(())
    }

    /// Keep an abstract success payload's binding independent of its eventual
    /// flattened representation. A single initializer can then reconstruct the
    /// abstract value before an arbitrary user body consumes or captures it.
    pub(super) fn stabilize_error_success_binding(
        &mut self,
        arm: &mut n::MatchArm,
        input: &Term,
        span: Span,
    ) -> Result<(), Error> {
        if arm.guard.is_none()
            && matches!(
                arm.pattern,
                n::Pattern::Tuple(_) | n::Pattern::AnonymousRecord { .. }
            )
        {
            // A guard/computed pattern may use a binding before the body starts.
            // Do not delay those bindings into body initializers.
            if self.can_delay_success_bindings(&arm.pattern, span)? {
                let mut statements = Vec::new();
                self.stabilize_structural_success_bindings(
                    &mut arm.pattern,
                    input,
                    &mut statements,
                    span,
                    0,
                )?;
                if !statements.is_empty() {
                    let replacement = self.error_node(p(P::Void), K::Unit, span)?;
                    let body = std::mem::replace(&mut arm.body, replacement);
                    arm.body.ty = body.ty.clone();
                    arm.body.kind = K::Block {
                        statements,
                        result: Some(Box::new(body)),
                    };
                }
            }
            return Ok(());
        }
        let Term::ErrorResult { result, .. } = input else {
            return Ok(());
        };
        if !matches!(
            result.as_ref(),
            Term::Rigid { .. }
                | Term::Bound { .. }
                | Term::Hole { .. }
                | Term::AssociatedType { .. }
                | Term::NormalForm { .. }
        ) || arm.guard.is_some()
        {
            return Ok(());
        }
        let n::Pattern::ErrorResult { label: None, inner } = &mut arm.pattern else {
            return Ok(());
        };
        let original = match inner.as_ref() {
            n::Pattern::Binding {
                binding,
                inner: nested,
            } if matches!(nested.as_ref(), n::Pattern::Wild) => *binding,
            n::Pattern::Wild => self.spread_binding(span)?,
            _ => return Ok(()),
        };
        let temporary = self.spread_binding(span)?;
        let initializer = n::Expr {
            source: self.error_generated_source(span)?,
            ty: (**result).clone(),
            kind: K::Local(temporary),
        };
        let statement = n::Expr {
            source: self.error_generated_source(span)?,
            ty: p(P::Void),
            kind: K::LetPattern {
                pattern: n::Pattern::Binding {
                    binding: original,
                    inner: Box::new(n::Pattern::Wild),
                },
                value: Box::new(initializer),
                guard: None,
                otherwise: None,
            },
        };
        **inner = n::Pattern::Binding {
            binding: temporary,
            inner: Box::new(n::Pattern::Wild),
        };
        let old_body = std::mem::replace(
            &mut arm.body,
            n::Expr {
                source: self.error_generated_source(span)?,
                ty: p(P::Void),
                kind: K::Unit,
            },
        );
        arm.body.ty = old_body.ty.clone();
        arm.body.kind = K::Block {
            statements: vec![statement],
            result: Some(Box::new(old_body)),
        };
        Ok(())
    }

    /// Complete private propagation exits before publishing a closure. Source
    /// identities keep nested closures and speculative attempts isolated.
    pub(super) fn complete_error_returns(
        &mut self,
        body: &mut n::Expr,
        result: Term,
        allowed: Option<&[Term]>,
        span: Span,
    ) -> Result<Term, Error> {
        let (effects, result) = match result {
            Term::Computation { effects, result } => (Some(effects), *result),
            result => (None, result),
        };
        let mut labels = self
            .pending_error_returns
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(observations) = &self.return_observations {
            for ty in observations.iter() {
                if let Term::ErrorResult { errors, .. } = ty {
                    labels.extend(errors.iter().cloned());
                }
            }
        }
        let target = if let Some(allowed) = allowed {
            let success = match result {
                Term::ErrorResult { result, .. } => *result,
                other => other,
            };
            Term::error_result(allowed.to_vec(), success)
        } else {
            Term::error_result(labels, result)
        };
        let mut remaining = self
            .pending_error_returns
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut explicit = self.inferred_return_sites.clone();
        nir::visit::expression_mut(body, &mut 100_000, |node, _| {
            let key = (node.source.start, node.source.end);
            if remaining.contains(&key) {
                let K::Return(Some(value)) = &mut node.kind else {
                    return Err("invalid pending error return".into());
                };
                let K::ErrorConvert {
                    conversion,
                    value: payload,
                } = &value.kind
                else {
                    return Err("invalid pending error conversion".into());
                };
                nir::error_results::validate_conversion(
                    *conversion,
                    &payload.ty,
                    &target,
                    &mut 100_000,
                    &mut |a, b| self.require(a, b, span).is_ok(),
                )
                .map_err(|e| format!("error result conversion: {e}"))?;
                value.ty = target.clone();
                remaining.remove(&key);
            } else if explicit.remove(&key) {
                let K::Return(value) = &mut node.kind else {
                    return Err("invalid inferred return site".into());
                };
                let payload = value.as_deref().cloned().unwrap_or(n::Expr {
                    source: node.source,
                    ty: p(P::Void),
                    kind: K::Unit,
                });
                *value = Some(Box::new(
                    self.error_delivery(payload, &target, span)
                        .map_err(|e| e.message)?,
                ));
            }
            Ok(())
        })
        .map_err(|message| error(span, message))?;
        if !explicit.is_empty() {
            return Err(error(span, "unresolved inferred return site"));
        }
        if !remaining.is_empty() {
            return Err(error(span, "unresolved error return site"));
        }
        if let Some(effects) = effects {
            let K::EffectComputation {
                body: inner,
                effects: actual,
            } = &mut body.kind
            else {
                return Err(context_error(
                    span,
                    "error propagation needs a computation body result context",
                ));
            };
            if *actual != effects {
                return Err(error(
                    span,
                    "inconsistent computation row during error result inference",
                ));
            }
            **inner = self.error_delivery((**inner).clone(), &target, span)?;
            let result = Term::computation(effects, target);
            body.ty = result.clone();
            Ok(result)
        } else {
            *body = self.error_delivery(body.clone(), &target, span)?;
            Ok(target)
        }
    }

    pub(super) fn deliver_error_context(
        &mut self,
        value: n::Expr,
        errors: &[Term],
        span: Span,
    ) -> Result<n::Expr, Error> {
        if is_never(&value.ty) {
            return Ok(value);
        }
        let success = match &value.ty {
            Term::ErrorResult { result, .. } => (**result).clone(),
            other => other.clone(),
        };
        self.error_delivery(value, &Term::error_result(errors.to_vec(), success), span)
    }

    pub(super) fn error_generated_source(&mut self, span: Span) -> Result<n::SourceRange, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "error lowering source budget exhausted"))?;
        let index = u32::try_from(self.expansion_sources.len())
            .map_err(|_| error(span, "error lowering source space exhausted"))?;
        self.expansion_sources.push(n::ExpansionSource {
            original: source(span),
            site: source(span),
        });
        Ok(n::SourceRange::generated(index))
    }

    fn error_delivery(
        &mut self,
        value: n::Expr,
        target: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if is_never(&value.ty) {
            return Ok(value);
        }
        if matches!(target, Term::ErrorResult { .. }) {
            let conversion = if matches!(value.ty, Term::ErrorResult { .. }) {
                nir::error_results::Conversion::Embed
            } else {
                nir::error_results::Conversion::Success
            };
            self.convert_error_value(value, target, conversion, span)
        } else {
            self.require(&value.ty, target, span)?;
            Ok(value)
        }
    }

    pub(super) fn error_handle(
        &mut self,
        expression: &Expr<'_>,
        input: &Expr<'_>,
        before: Option<&hir::pattern::PatternArm<'_>>,
        arms: &[hir::pattern::PatternArm<'_>],
        after: Option<&hir::pattern::PatternArm<'_>>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let input = self.expr(input, None)?;
        if is_never(&input.ty) {
            return Ok(input);
        }
        let Term::ErrorResult { errors, result } = &input.ty else {
            return Err(error(
                expression.span,
                "error elimination requires an error-qualified operand",
            ));
        };
        let success = (**result).clone();
        let errors = errors.clone();
        self.required_steps = self
            .required_steps
            .checked_sub(arms.len().saturating_add(errors.len()) as u64)
            .ok_or_else(|| error(expression.span, "error elimination budget exhausted"))?;
        if let Some(after) = after {
            return self
                .error_catch_handle(expression, input, before, arms, after, expected, partial);
        }
        let saved = self.locals.clone();
        let outcome = (|| {
            let mut checked = Vec::new();
            for arm in arms {
                self.locals = saved.clone();
                let pattern = hir::pattern::Pattern {
                    hir_id: arm.pat.hir_id,
                    span: arm.pat.span,
                    kind: hir::pattern::PatternKind::ErrorErr(&arm.pat),
                };
                let mut pattern = self.pattern(&pattern, &input.ty)?;
                if let Some(guard) = arm.guard {
                    let condition = self.under_matched_pattern_type_facts(
                        &pattern,
                        &input,
                        guard.span,
                        |this| this.expr(guard, Some(&p(P::Bool))),
                    )?;
                    pattern = n::Pattern::Guard {
                        pattern: Box::new(pattern),
                        condition: Box::new(condition),
                    };
                }
                let body = self.under_matched_pattern_type_facts(
                    &pattern,
                    &input,
                    arm.body.span,
                    |this| {
                        if let Some(target) = expected {
                            this.expr_context(arm.body, Some(target), partial)
                        } else if let Some(body) = this.argument_attempt(arm.body, None, partial)? {
                            Ok(body)
                        } else {
                            this.expr_context(
                                arm.body,
                                (!is_never(&success)).then_some(&success),
                                partial,
                            )
                        }
                    },
                )?;
                checked.push(n::MatchArm {
                    pattern,
                    guard: None,
                    body,
                });
            }
            let patterns: Vec<_> = checked.iter().map(|arm| &arm.pattern).collect();
            let mut residual = Vec::new();
            for label in errors {
                let domain = Term::error_result(vec![label.clone()], p(P::Never));
                let allowance = usize::try_from(self.required_steps)
                    .unwrap_or(usize::MAX)
                    .min(100_000);
                let (coverage, used) = crate::coverage::check_metered_with_collections_in(
                    &self.structs,
                    &self.enums,
                    &domain,
                    &patterns,
                    allowance,
                    &self.context,
                    self.protocols.tuple,
                    &|domain, fuel| self.collection_domain_budgeted(domain, fuel),
                    self.target,
                );
                self.required_steps -= used as u64;
                if coverage != crate::coverage::Coverage::Exhaustive {
                    residual.push(label);
                }
            }
            let target = if let Some(target) = expected {
                target.clone()
            } else {
                let mut result = success.clone();
                let mut output_errors = residual.clone();
                for arm in &checked {
                    if is_never(&arm.body.ty) {
                        continue;
                    }
                    let body_success = if let Term::ErrorResult { errors, result } = &arm.body.ty {
                        output_errors.extend(errors.iter().cloned());
                        result.as_ref()
                    } else {
                        &arm.body.ty
                    };
                    if is_never(body_success) {
                        continue;
                    }
                    if is_never(&result) {
                        result = body_success.clone();
                    } else {
                        self.require(body_success, &result, expression.span)?;
                    }
                }
                Term::error_result(output_errors, result)
            };
            self.locals = saved.clone();
            let binding = self.spread_binding(expression.span)?;
            let value = n::Expr {
                source: self.error_generated_source(expression.span)?,
                ty: success,
                kind: K::Local(binding),
            };
            let mut lowered = vec![n::MatchArm {
                pattern: n::Pattern::ErrorResult {
                    label: None,
                    inner: Box::new(n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    }),
                },
                guard: None,
                body: self.error_delivery(value, &target, expression.span)?,
            }];
            for mut arm in checked {
                arm.body = self.error_delivery(arm.body, &target, expression.span)?;
                lowered.push(arm);
            }
            // Coverage is computed on input payloads, independently of output
            // errors emitted by an ordinary branch. Guarded/partial payloads
            // retain their original label on the unmatched path.
            for label in residual {
                let binding = self.spread_binding(expression.span)?;
                let value = n::Expr {
                    source: self.error_generated_source(expression.span)?,
                    ty: label.clone(),
                    kind: K::Local(binding),
                };
                let body = self.convert_error_value(
                    value,
                    &target,
                    nir::error_results::Conversion::Failure,
                    expression.span,
                )?;
                lowered.push(n::MatchArm {
                    pattern: n::Pattern::ErrorResult {
                        label: Some(label),
                        inner: Box::new(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        }),
                    },
                    guard: None,
                    body,
                });
            }
            Ok(n::Expr {
                source: source(expression.span),
                ty: target,
                kind: K::Match {
                    value: Box::new(input),
                    arms: lowered,
                },
            })
        })();
        self.locals = saved;
        outcome
    }
}
