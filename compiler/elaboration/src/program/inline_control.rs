//! Required pattern selection and constant binding delivery for inline matches.
use super::*;

impl Checker<'_, '_> {
    /// Validate checked selectors without interpreting them. Only walk the
    /// when-condition spine, never ordinary runtime conditions in arm bodies.
    pub(super) fn check_inactive_inline_selectors(
        &mut self,
        source: &Expr<'_>,
        checked: &n::Expr,
    ) -> Result<(), Error> {
        let staged = BTreeSet::new();
        match (&source.kind, &checked.kind) {
            (H::If(..), K::If { condition, .. }) => {
                self.check_inline_input_availability(condition, &staged, source.span)
            }
            (H::Match(..), K::Match { value, .. }) => {
                self.check_inline_input_availability(value, &staged, source.span)
            }
            (H::When(arms), _) => {
                let mut tail = checked;
                for arm in *arms {
                    if arm.cond.is_none() {
                        break;
                    }
                    let K::If {
                        condition,
                        else_branch,
                        ..
                    } = &tail.kind
                    else {
                        return Err(error(source.span, "inline when lost its checked selector"));
                    };
                    self.check_inline_input_availability(condition, &staged, arm.body.span)?;
                    if let Some(next) = else_branch {
                        tail = next;
                    } else {
                        break;
                    }
                }
                Ok(())
            }
            _ => Err(error(
                source.span,
                "inline control lost its checked selector",
            )),
        }
    }

    /// Candidate-local observation aliases and or-branch binders are staged
    /// even when they are not successful exports of the complete pattern.
    pub(super) fn check_inline_pattern_availability(
        &mut self,
        pattern: &n::Pattern,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let scanned = (|| {
            let external = nir::free_bindings::collect_pattern(pattern, &mut fuel)?;
            let observations = nir::visit::pattern_observations(pattern, &mut fuel)?;
            let mut checks = Vec::new();
            for observation in observations {
                let free = nir::free_bindings::collect(observation, &mut fuel)?;
                let staged = free.difference(&external).copied().collect::<BTreeSet<_>>();
                checks.push((observation, staged));
            }
            Ok::<_, String>(checks)
        })();
        self.required_steps -= (before - fuel) as u64;
        for (observation, staged) in scanned.map_err(|e| error(span, e))? {
            self.check_inline_input_availability(observation, &staged, span)?;
        }
        Ok(())
    }

    /// Pattern computations have already passed staged ordinary checking.
    /// Retain their domains and all possible outcomes in the P4 witness, without
    /// invoking them or treating their comptime calls as runtime behavior.
    pub(super) fn abstract_inline_pattern(
        &mut self,
        pattern: &mut n::Pattern,
        span: Span,
    ) -> Result<(), Error> {
        let mut pending = vec![pattern];
        while let Some(pattern) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "inline pattern behavior budget exhausted"))?;
            let computation = match pattern {
                n::Pattern::CodeEvidence { .. } => {
                    return Err(error(span, "unresolved pattern-code evidence"));
                }
                n::Pattern::Then { first, value, next } => {
                    pending.push(first);
                    pending.push(next);
                    Some(value)
                }
                n::Pattern::Guard { pattern, condition } => {
                    pending.push(pattern);
                    Some(condition)
                }
                n::Pattern::OptionalPresent(inner)
                | n::Pattern::ErrorResult { inner, .. }
                | n::Pattern::Observe { inner, .. }
                | n::Pattern::Binding { inner, .. }
                | n::Pattern::Remap { pattern: inner, .. } => {
                    pending.push(inner);
                    None
                }
                n::Pattern::Tuple(fields)
                | n::Pattern::TypeTuple(fields)
                | n::Pattern::Or(fields)
                | n::Pattern::NominalType { fields, .. }
                | n::Pattern::Record { fields, .. }
                | n::Pattern::AnonymousRecord { fields, .. }
                | n::Pattern::Variant { fields, .. }
                | n::Pattern::NormalForm {
                    arguments: fields, ..
                } => {
                    pending.extend(fields);
                    None
                }
                n::Pattern::OptionalAbsent
                | n::Pattern::Wild
                | n::Pattern::Unit
                | n::Pattern::Literal(_) => None,
            };
            if let Some(value) = computation {
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index: self.next_temporary_binding,
                };
                self.next_temporary_binding =
                    self.next_temporary_binding.checked_add(1).ok_or_else(|| {
                        error(span, "inline pattern behavior identity space exhausted")
                    })?;
                self.ordinary_behavior_inputs
                    .insert(binding, value.ty.clone());
                value.kind = K::Local(binding);
            }
        }
        Ok(())
    }

    pub(super) fn select_inline_pattern(
        &mut self,
        pattern: &n::Pattern,
        subject: &Term,
        source: &Expr<'_>,
    ) -> Result<Option<BTreeMap<n::LocalId, std::sync::Arc<Term>>>, Error> {
        let mut bindings = BTreeMap::new();
        let original_steps = self.required_steps;
        let before = usize::try_from(original_steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps: before,
            max_limbs: 4096,
        };
        let mut failure = None;
        let matches = normal::known_patterns::PatternInspector {
            tuple_head: self.bound_tuple_metadata(),
            evaluate:
                |expr: &n::Expr,
                 depth,
                 budget: &mut comptime_runtime::Budget,
                 bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>| {
                    if matches!(expr.kind, K::Literal(_)) {
                        return self.known_normal_input_in(expr, depth, budget, bindings);
                    }
                    self.required_steps = budget.steps as u64;
                    let result = self.invoke_inline_pattern_expression(expr, bindings, source);
                    budget.steps = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    match result {
                        Ok(value) => Some(value),
                        Err(error) => {
                            failure = Some(error);
                            None
                        }
                    }
                },
        }
        .inspect(pattern, subject, 0, &mut budget, &mut bindings);
        self.required_steps = original_steps - (before - budget.steps) as u64;
        if let Some(error) = failure {
            return Err(error);
        }
        match matches {
            Some(false) => return Ok(None),
            None => {
                return Err(error(
                    source.span,
                    "inline pattern selection requires supported immutable pattern evidence and sufficient budget",
                ));
            }
            Some(true) => {}
        }
        // Keep alternative-local slots while executing computations; only the
        // arm's declared exports may be delivered to its runtime body.
        bindings.retain(|id, _| self.locals.contains_key(id));
        Ok(Some(bindings))
    }

    fn invoke_inline_pattern_expression(
        &mut self,
        expression: &n::Expr,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        source: &Expr<'_>,
    ) -> Result<Term, Error> {
        let mut types = BTreeMap::new();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let scanned = nir::visit::expression(expression, &mut fuel, |expr, fuel| {
            if let K::Local(id) = expr.kind {
                if bindings.contains_key(&id) {
                    nir::type_computations::charge_term(&expr.ty, fuel)
                        .map_err(|e| e.to_string())?;
                    if types.get(&id).is_some_and(|ty| *ty != expr.ty) {
                        return Err("inline pattern local has inconsistent domains".into());
                    }
                    types.insert(id, expr.ty.clone());
                }
            }
            Ok(())
        });
        self.required_steps -= (before - fuel) as u64;
        scanned.map_err(|e| error(source.span, e))?;
        let mut inputs = Vec::new();
        for (id, ty) in types {
            let value = &bindings[&id];
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let charged = nir::type_computations::charge_term(value, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            charged.map_err(|e| error(source.span, e.to_string()))?;
            inputs.push((id, ty, value.as_ref().clone()));
        }
        constants::charge_copy(expression, &mut self.required_steps, source.span)?;
        let (value, _) = self.required_normal_input(source, expression.clone(), inputs)?;
        self.required_normal_term(value, source.span)
    }

    fn inline_pattern_inputs(
        &mut self,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<Vec<(n::LocalId, Term, Term)>, Error> {
        bindings
            .iter()
            .map(|(id, value)| {
                let ty =
                    self.locals.get(id).cloned().ok_or_else(|| {
                        error(span, "inline pattern binding has no declared type")
                    })?;
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let charged = nir::type_computations::charge_term(value, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                charged.map_err(|e| error(span, e.to_string()))?;
                Ok((*id, ty, value.as_ref().clone()))
            })
            .collect()
    }

    pub(super) fn materialize_inline_bindings(
        &mut self,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        source_expr: &Expr<'_>,
    ) -> Result<Vec<n::Expr>, Error> {
        let mut statements = Vec::new();
        for (id, ty, value) in self.inline_pattern_inputs(bindings, source_expr.span)? {
            let root = n::Expr {
                source: source(source_expr.span),
                ty: ty.clone(),
                kind: K::Local(id),
            };
            let (value, _) =
                self.required_normal_input(source_expr, root, vec![(id, ty.clone(), value)])?;
            let value = self.materialize_local_constant(value, ty, source_expr.span)?;
            self.generator_data.insert(id, Some(value.clone()));
            // Immutable comptime descriptions and exact scalar literals are
            // substituted into the selected body. Emitting an additional let
            // would incorrectly require runtime storage for these domains.
            // Keep aggregate/pointer reconstruction and resource storage intact.
            if matches!(value.kind, K::TypeValue(_))
                || (matches!(value.kind, K::Literal(_))
                    && matches!(value.ty, Term::Primitive(P::Integer | P::Float)))
            {
                continue;
            }
            statements.push(n::Expr {
                source: source(source_expr.span),
                ty: p(P::Void),
                kind: K::Let {
                    binding: id,
                    value: Box::new(value),
                },
            });
        }
        Ok(statements)
    }
}
