//! Opaque head inputs: a stable value or a declaration-side symbolic input.
use super::*;
use observation::Failure;
mod arguments;
pub(super) mod known_patterns;
pub(super) mod known_scalars;
pub(super) mod observation;
mod values;

/// Expand syntax before assigning dependent contexts. Evaluating a literal
/// tuple first would default its numeric fields independently of preceding
/// Type arguments. Variadic List<Type> tails have a different collection rule.
pub(super) fn literal_spreads<'h>(inputs: &[Arg<'h>]) -> Option<Vec<Arg<'h>>> {
    if !inputs.iter().any(|input| {
        matches!(input, Arg::Expand(value)
        if matches!(value.kind, H::Tuple(_) | H::Unit))
    }) {
        return None;
    }
    let mut flat = Vec::new();
    for input in inputs {
        match input {
            Arg::Expand(value) => match value.kind {
                H::Tuple(fields) => flat.extend(fields.iter().map(Arg::Positional)),
                H::Unit => {}
                _ => flat.push(input.clone()),
            },
            _ => flat.push(input.clone()),
        }
    }
    Some(flat)
}

impl Checker<'_, '_> {
    pub(super) fn is_registered_normal_form(&self, owner: OwnerId) -> bool {
        self.normal_parameters.contains_key(&owner)
            && self
                .package
                .item(owner)
                .is_some_and(|item| match &item.kind {
                    ItemKind::Fn(signature, _) => {
                        signature.modifiers.is_normal_form_head && signature.modifiers.is_pure
                    }
                    ItemKind::Const(..) => self.signatures.contains_key(&owner),
                    _ => false,
                })
    }

    pub(super) fn bound_tuple_head(&mut self, span: Span) -> Result<Option<DeclarationId>, Error> {
        let Some(head) = self.protocols.tuple else {
            return Ok(None);
        };
        let owner = self
            .source_declarations
            .owner(head)
            .ok_or_else(|| error(span, "bound Tuple head is not in this source package"))?;
        self.ensure_signature(owner, span)?;
        let parameters = self
            .normal_parameters
            .get(&owner)
            .ok_or_else(|| error(span, "bound Tuple declaration is not a normal-form head"))?;
        if parameters.len() != 1
            || !self.clause_has_collector(owner)
            || !self.is_type_list_type(&parameters[0].2)
        {
            return Err(error(
                span,
                "bound Tuple head requires one variadic meta.List<Type> input",
            ));
        }
        self.ensure_contract(owner, span)?;
        Ok(Some(head))
    }
    pub(super) fn tuple_value_type(
        &mut self,
        elements: Vec<Term>,
        span: Span,
    ) -> Result<Term, Error> {
        let Some(head) = self.bound_tuple_head(span)? else {
            return Ok(Term::Tuple(elements));
        };
        let owner = self.source_declarations.owner(head).unwrap();
        let domain = self.normal_parameters[&owner][0].2.clone();
        let arguments = vec![self.freeze_normal_list(&domain, elements, span)?];
        self.check_type_application(owner, &arguments, span)?;
        Ok(Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        })
    }

    pub(super) fn freeze_normal_list(
        &mut self,
        domain: &Term,
        fields: Vec<Term>,
        span: Span,
    ) -> Result<Term, Error> {
        let available = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut remaining = available;
        let value = nir::comptime_values::freeze_admitted_list(domain, fields, &mut remaining);
        self.required_steps -= (available - remaining) as u64;
        value.map_err(|failure| error(span, failure.to_string()))
    }

    // Legacy optional hint consumer. Required NF planning uses the typed
    // boundary below; CMTT hint migration remains a separate consumer change.
    pub(super) fn known_collected_list_in(
        &mut self,
        items: &[n::NormalFormArgument],
        ty: &Term,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Option<Term> {
        self.observe_collected_list_in(items, ty, bindings).ok()
    }

    pub(super) fn checked_collected_list_in(
        &mut self,
        items: &[n::NormalFormArgument],
        ty: &Term,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        observation::optional(self.observe_collected_list_in(items, ty, bindings))
            .map_err(|failure| error(span, format!("static value observation failed: {failure}")))
    }

    fn observe_collected_list_in(
        &mut self,
        items: &[n::NormalFormArgument],
        ty: &Term,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<Term, Failure> {
        let element = self.list_element_type(ty).ok_or(Failure::Unavailable)?;
        let mut budget = comptime_runtime::Budget {
            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
            max_limbs: 4096,
        };
        let before = budget.steps;
        let result = (|| {
            budget.consume(items.len())?;
            let domain = copy_observed_term(ty, &mut budget)?;
            let mut parts = Vec::new();
            let mut fields = Vec::new();
            for item in items {
                match item {
                    n::NormalFormArgument::Value(value) if value.ty == *element => {
                        let value =
                            self.observe_normal_input_in(value, 0, &mut budget, bindings)?;
                        fields.push(value);
                    }
                    n::NormalFormArgument::ListSpread(value) if value.ty == *ty => {
                        if !fields.is_empty() {
                            parts.push(freeze_known_list(
                                ty,
                                element,
                                std::mem::take(&mut fields),
                                &mut budget,
                            )?);
                        }
                        parts.push(self.observe_normal_input_in(
                            value,
                            0,
                            &mut budget,
                            bindings,
                        )?);
                    }
                    _ => return Err(Failure::Unavailable),
                }
            }
            if !fields.is_empty() {
                parts.push(freeze_known_list(ty, element, fields, &mut budget)?);
            }
            // Pay for inspecting/merging retained operands before normalization.
            for part in &parts {
                nir::type_computations::charge_term(part, &mut budget.steps)?;
            }
            Term::list_concat(domain, parts).map_err(|reason| Failure::Invalid(reason.to_string()))
        })();
        self.required_steps -= (before - budget.steps) as u64;
        result
    }

    pub(super) fn is_type_list_type(&self, ty: &Term) -> bool {
        nir::normal_forms::type_list_input_type(ty)
            && match ty {
                Term::NormalForm { head, .. } => Some(*head) == self.protocols.meta_list,
                Term::Nominal { declaration, .. } => Some(*declaration) == self.protocols.meta_list,
                _ => false,
            }
    }

    pub(super) fn collection_domain_budgeted(
        &self,
        domain: &Term,
        fuel: &mut usize,
    ) -> Result<Option<Term>, nir::NominalAdmissionError> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or(nir::NominalAdmissionError::Budget)?;
        let Some(element) = self.list_element_type(domain) else {
            return Ok(None);
        };
        nir::type_computations::charge_term(element, fuel)?;
        Ok(Some(element.clone()))
    }

    pub(super) fn list_element_type<'a>(&self, ty: &'a Term) -> Option<&'a Term> {
        match ty {
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments,
            } if Some(*head) == self.protocols.meta_list => match arguments.as_slice() {
                [element] if element.sort() == Sort::Type => Some(element),
                _ => None,
            },
            _ => None,
        }
    }
    /// Lexically visible declaration parameters used by both invocation and
    /// reification. A member's outer value parameters remain invocation-scoped.
    pub(super) fn visible_comptime_parameters(
        &mut self,
        span: Span,
    ) -> Result<Vec<DeclarationId>, Error> {
        let mut owner = self.signature_owner;
        let mut seen = BTreeSet::new();
        let mut parameters = BTreeSet::new();
        while let Some(current) = owner {
            if seen.len() >= 64 || !seen.insert(current) {
                return Err(error(
                    span,
                    "abstract value capture environment is excessively deep",
                ));
            }
            let current_parameters = self
                .type_parameters
                .get(&current)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            self.required_steps = self
                .required_steps
                .checked_sub(current_parameters.len() as u64 + 1)
                .ok_or_else(|| error(span, "abstract value scope preparation budget exhausted"))?;
            parameters.extend(current_parameters.iter().copied());
            for inputs in self
                .normal_parameters
                .get(&current)
                .into_iter()
                .chain(self.normal_input_prefixes.get(&current))
            {
                self.required_steps = self
                    .required_steps
                    .checked_sub(inputs.len() as u64)
                    .ok_or_else(|| {
                        error(span, "abstract value scope preparation budget exhausted")
                    })?;
                parameters.extend(inputs.iter().map(|(id, _, _)| *id));
            }
            owner = self.self_containers.get(&current).copied();
            if owner.is_none() {
                let steps = &mut self.required_steps;
                owner = self.lexical_source_protocols.trait_parent(current, || {
                    *steps = steps.checked_sub(1).ok_or_else(|| {
                        error(span, "abstract value scope preparation budget exhausted")
                    })?;
                    Ok(())
                })?;
            }
        }
        Ok(parameters.into_iter().collect())
    }

    pub(super) fn normal_value_argument(
        &mut self,
        value: &Expr<'_>,
        ty: &Term,
    ) -> Result<Term, Error> {
        self.normal_value_argument_at(value, ty, 0)
    }

    fn normal_value_argument_at(
        &mut self,
        value: &Expr<'_>,
        ty: &Term,
        depth: usize,
    ) -> Result<Term, Error> {
        if depth >= 128 || self.required_steps == 0 {
            return Err(error(
                value.span,
                "normal-form input structure budget exhausted",
            ));
        }
        self.required_steps -= 1;
        // A dependent input specialized to Type is a type identity, not a
        // scalar comptime-value reference with the wrong semantic sort.
        if *ty == p(P::Type) {
            return self.type_expr(value);
        }
        if let H::ResolvedLocal(_, id) = value.kind
            && let Some(input) = self.callable_type_input_expression(value, local(id), Some(ty))?
        {
            return match input.kind {
                K::TypeValue(symbol) | K::ComptimeValue(symbol) => Ok(symbol),
                _ => unreachable!("callable telescope supplies abstract values"),
            };
        }
        if let H::ResolvedFnParam(_, id) = value.kind {
            let binding = local(id);
            let in_signature = self.signature_owner.is_some_and(|owner| {
                owner == id.owner
                    && (self.active_signatures.contains(&owner) || self.checking_contract)
            });
            if let Some((symbol, actual)) = self
                .input_facts
                .values
                .get(&binding)
                .filter(|_| in_signature || self.locals.contains_key(&binding))
            {
                self.require(actual, ty, value.span)?;
                return Ok(symbol.clone());
            }
            // This annotation is checked before invocation. Do not send an
            // unavailable ordinary input to the evaluator and expose its
            // internal MissingBinding error as a source diagnostic.
            let comptime = self
                .package
                .item(id.owner)
                .is_some_and(|item| match &item.kind {
                    ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature) => {
                        signature.modifiers.is_comptime
                    }
                    _ => false,
                });
            return Err(error(
                value.span,
                if comptime {
                    "normal-form annotation depending on an ordinary comptime-function input requires invocation-time type elaboration, which is not yet implemented"
                } else {
                    "normal-form annotation cannot read an ordinary runtime input; its value must be available at comptime"
                },
            ));
        }
        if matches!(value.kind, H::ResolvedClauseParam(..)) {
            // Symbolic inputs use the same contextual conversions as ordinary
            // expressions. Observing a checked identifier preserves its scope;
            // it does not require a concrete invocation-time value.
            let checked = self.expr(value, Some(ty))?;
            if let Some(identity) = self.preserved_input_identity(&checked, [], value.span)? {
                return Ok(identity);
            }
            let (result, _) = self.required_normal_input(value, checked, Vec::new())?;
            return self.required_normal_term(result, value.span);
        }
        // Preserve declaration inputs inside aggregates instead of attempting
        // to execute them before their enclosing definition is instantiated.
        let shape = if matches!(value.kind, H::Tuple(_)) {
            Some(self.structural_type(ty, value.span)?)
        } else {
            None
        };
        if let (H::Tuple(values), Some(Term::Tuple(types))) = (&value.kind, shape.as_ref()) {
            if values.len() != types.len() {
                return Err(error(
                    value.span,
                    "normal-form tuple argument arity mismatch",
                ));
            }
            return Ok(Term::TupleValue(
                values
                    .iter()
                    .zip(types)
                    .map(|(value, ty)| self.normal_value_argument_at(value, ty, depth + 1))
                    .collect::<Result<_, _>>()?,
            ));
        }
        if let H::List(values) = value.kind {
            if let Some(element) = self.list_element_type(ty).cloned() {
                let mut fields = Vec::with_capacity(values.len());
                for value in values {
                    fields.push(self.normal_value_argument_at(value, &element, depth + 1)?);
                }
                return self.freeze_normal_list(ty, fields, value.span);
            }
        }
        let result = self.required_expression(value, Some(ty))?;
        self.required_normal_term(result, value.span)
    }
}

impl Checker<'_, '_> {
    pub(super) fn function_parameter_metadata(&self) -> Vec<nir::function_parameters::Declaration> {
        self.signatures
            .keys()
            .chain(self.interface_signatures.keys())
            .chain(self.open_functions.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|owner| {
                let parameters = self.type_parameters.get(owner)?;
                let (parameters, domains): (Vec<DeclarationId>, Vec<Term>) = if let Some(inputs) =
                    self.normal_parameters.get(owner)
                {
                    (
                        inputs.iter().map(|(id, _, _)| *id).collect(),
                        inputs.iter().map(|(_, _, domain)| domain.clone()).collect(),
                    )
                } else {
                    (
                        parameters.clone(),
                        parameters
                            .iter()
                            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                            .collect(),
                    )
                };
                if parameters.is_empty() {
                    return None;
                }
                Some(nir::function_parameters::Declaration {
                    declaration: declaration(self.package, &self.package_identities, *owner),
                    parameters,
                    domains,
                })
            })
            .chain(self.generated_function_parameters.values().cloned())
            .collect()
    }

    pub(super) fn normal_form_head_metadata(
        &self,
        owner: OwnerId,
    ) -> Option<nir::normal_forms::Head> {
        let parameters = self.normal_parameters.get(&owner)?;
        Some(nir::normal_forms::Head {
            collector: self
                .clause_has_collector(owner)
                .then(|| parameters.len() - 1),
            role: if Some(declaration(self.package, &self.package_identities, owner))
                == self.protocols.tuple
            {
                nir::normal_forms::Role::Tuple
            } else if Some(declaration(self.package, &self.package_identities, owner))
                == self.protocols.meta_list
            {
                nir::normal_forms::Role::TypeList
            } else {
                nir::normal_forms::Role::Opaque
            },
            declaration: declaration(self.package, &self.package_identities, owner),
            requirements: self
                .requirements
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .chain(
                    self.capabilities
                        .get(&owner)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .map(n::TypePredicate::Trait),
                )
                .collect(),
            parameters: parameters
                .iter()
                .map(|(declaration, binding, ty)| nir::normal_forms::Parameter {
                    declaration: *declaration,
                    binding: *binding,
                    ty: ty.clone(),
                    kind: if *ty == p(P::Type) {
                        nir::normal_forms::Kind::Type
                    } else {
                        nir::normal_forms::Kind::Value
                    },
                })
                .collect(),
        })
    }

    pub(super) fn normal_form_metadata(&self) -> Vec<nir::normal_forms::Head> {
        self.normal_parameters
            .keys()
            .filter_map(|owner| self.normal_form_head_metadata(*owner))
            .collect()
    }

    pub(super) fn normal_form_value(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.ensure_signature(owner, expr.span)?;
        self.ensure_contract(owner, expr.span)?;
        let head = self.normal_form_head_metadata(owner).ok_or_else(|| {
            error(
                expr.span,
                "normal-form callable is missing declaration metadata",
            )
        })?;
        let ty = head
            .callable_type()
            .map_err(|reason| error(expr.span, reason.to_string()))?;
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::Function(head.declaration),
        })
    }

    pub(super) fn normal_form_constant_target(
        &mut self,
        head: &Expr<'_>,
    ) -> Result<Option<OwnerId>, Error> {
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Const(..))
        ) {
            return Ok(None);
        }
        let value = self.constant_value(owner, head.span)?;
        if !value.ty.is_normal_form_callable() {
            return Ok(None);
        }
        let K::Function(identity) = value.kind else {
            return Err(error(
                head.span,
                "normal-form constant has no completed declaration identity",
            ));
        };
        self.source_declarations
            .owner(identity)
            .map(Some)
            .ok_or_else(|| error(head.span, "normal-form callable declaration is unavailable"))
    }

    pub(super) fn dynamic_normal_form_application(
        &mut self,
        expr: &Expr<'_>,
        callee: n::Expr,
        inputs: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Term::NormalFormFunction(_, collector) = &callee.ty else {
            return Err(error(
                expr.span,
                "normal-form application requires a normal-form callable",
            ));
        };
        let Some(index) = *collector else {
            return self.finish_prepared_call(expr, callee, inputs, Vec::new(), expected, None);
        };
        if inputs.len() < index {
            return Err(error(expr.span, "normal-form argument count mismatch"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(inputs.len() as u64)
            .ok_or_else(|| {
                error(
                    expr.span,
                    "normal-form argument preparation budget exhausted",
                )
            })?;
        if let [Arg::Expand(value)] = &inputs[index..] {
            let mut arguments = inputs[..index].to_vec();
            arguments.push(Arg::Positional(value));
            return self.finish_prepared_call(expr, callee, &arguments, Vec::new(), expected, None);
        }
        let mut elements = Vec::new();
        for input in &inputs[index..] {
            match input {
                Arg::Positional(value) => elements.push((**value).clone()),
                _ => {
                    return Err(error(
                        expr.span,
                        "dynamic normal-form collection currently requires positional elements",
                    ));
                }
            }
        }
        let collected = Expr {
            kind: H::List(&elements),
            ..expr.clone()
        };
        let mut arguments = inputs[..index].to_vec();
        arguments.push(Arg::Positional(&collected));
        self.finish_prepared_call(expr, callee, &arguments, Vec::new(), expected, None)
    }

    /// Type positions consume the same checked input plan as value positions.
    /// Observation can retain abstract declaration inputs; invocation is only
    /// required for computations that cannot supply static evidence.
    pub(super) fn normal_type_annotation(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        inputs: &[Arg<'_>],
    ) -> Result<Term, Error> {
        // Input producers belong to comptime here. Runtime Splice must not
        // intercept packet formation while checking a type annotation.
        let checked = self.with_staged_input(true, |this| {
            this.normal_type_expression(expr, owner, inputs, Some(&p(P::Type)))
        })?;
        let known = self.checked_normal_input_in(&checked, &BTreeMap::new(), expr.span)?;
        let known = match known {
            Some(known) => Some(known),
            None => self.required_normal_type_plan(expr, &checked)?,
        };
        if let Some(ty @ Term::NormalForm { .. }) = known {
            let Term::NormalForm { arguments, .. } = &ty else {
                unreachable!()
            };
            self.check_type_application(owner, arguments, expr.span)?;
            return Ok(ty);
        }
        let (comptime_interpretor::Value::Type(ty), _) =
            self.required_normal_input(expr, checked, Vec::new())?
        else {
            return Err(error(
                expr.span,
                "normal-form application did not produce Type",
            ));
        };
        Ok(ty)
    }

    pub(super) fn normal_type_expression(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.ensure_signature(owner, expr.span)?;
        self.ensure_contract(owner, expr.span)?;
        let expanded = if self.clause_has_collector(owner) {
            self.clause_prefix_literal_spreads(owner, inputs)
        } else {
            literal_spreads(inputs)
        };
        if let Some(expanded) = expanded {
            return self.normal_type_expression(expr, owner, &expanded, expected);
        }
        self.planned_normal_expression(owner, inputs, expr.span, expected)
    }
}

impl Checker<'_, '_> {
    pub(super) fn matched_value_pattern_input(
        &mut self,
        pattern: &n::Pattern,
        ty: &Term,
        span: Span,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        if depth >= 128 {
            return Err(error(span, "value pattern evidence depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "value pattern evidence budget exhausted"))?;
        match pattern {
            // Float equality need not identify one retained bit representation
            // (notably signed zero). A known observed input may still supply it.
            n::Pattern::Literal(
                n::Literal::Float(_) | n::Literal::Float32(_) | n::Literal::Float64(_),
            ) => Ok(None),
            n::Pattern::Literal(literal) => {
                let expr = n::Expr {
                    source: source(span),
                    ty: ty.clone(),
                    kind: K::Literal(literal.clone()),
                };
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let value = self.known_normal_input(&expr, 0, &mut budget);
                self.required_steps -= (before - budget.steps) as u64;
                Ok(value)
            }
            n::Pattern::Binding { inner, .. } | n::Pattern::Remap { pattern: inner, .. } => {
                self.matched_value_pattern_input(inner, ty, span, depth + 1)
            }
            n::Pattern::Guard { pattern, condition } => {
                if let Some(value) =
                    self.matched_value_pattern_input(pattern, ty, span, depth + 1)?
                {
                    return Ok(Some(value));
                }
                if *ty == p(P::Bool)
                    && let n::Pattern::Binding { binding, inner } = pattern.as_ref()
                    && matches!(inner.as_ref(), n::Pattern::Wild)
                {
                    let mut admitted = 0u8;
                    for candidate in [false, true] {
                        let Some(outcomes) = boolean_guard_values(
                            condition,
                            *binding,
                            candidate,
                            &mut self.required_steps,
                            span,
                            depth + 1,
                        )?
                        else {
                            return Ok(None);
                        };
                        if outcomes & 2 != 0 {
                            admitted |= 1 << u8::from(candidate);
                        }
                    }
                    return Ok(match admitted {
                        1 => Some(Term::Bool(false)),
                        2 => Some(Term::Bool(true)),
                        _ => None,
                    });
                }
                if let n::Pattern::Binding { binding, inner } = pattern.as_ref()
                    && matches!(inner.as_ref(), n::Pattern::Wild)
                    && matches!(ty, Term::Primitive(primitive) if primitive.fixed_integer_layout().is_some()
                        || matches!(primitive, P::Integer | P::Char | P::Symbol | P::Str | P::BitVec(_)))
                {
                    return self.exact_scalar_guard_input(condition, *binding, ty, span, depth + 1);
                }
                Ok(None)
            }
            n::Pattern::Then { first, value, next } => {
                if let Some(value) = self.matched_value_pattern_input(first, ty, span, depth + 1)? {
                    return Ok(Some(value));
                }
                if let (n::Pattern::Binding { binding, inner }, K::Local(id)) =
                    (first.as_ref(), &value.kind)
                {
                    if binding == id && matches!(inner.as_ref(), n::Pattern::Wild) {
                        return self.matched_value_pattern_input(next, ty, span, depth + 1);
                    }
                }
                Ok(None)
            }
            n::Pattern::Or(arms) => {
                let mut common = None;
                for arm in arms {
                    let Some(value) = self.matched_value_pattern_input(arm, ty, span, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    if common.as_ref().is_some_and(|previous| previous != &value) {
                        return Ok(None);
                    }
                    common = Some(value);
                }
                Ok(common)
            }
            _ => Ok(None),
        }
    }

    fn exact_scalar_guard_input(
        &mut self,
        condition: &n::Expr,
        binding: n::LocalId,
        ty: &Term,
        span: Span,
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        let mut fuel = self.required_steps;
        let result = pattern_guard_evidence(
            condition,
            binding,
            true,
            &mut fuel,
            span,
            depth,
            &mut |expr, truth, fuel| {
                let (left, right) = match &expr.kind {
                    K::ScalarEqual(left, right)
                    | K::BitEqual(left, right)
                    | K::IntegerBinary {
                        op: n::IntegerOp::Equal,
                        left,
                        right,
                    } => (left, right),
                    _ => return Ok(None),
                };
                if left.ty != *ty || right.ty != *ty {
                    return Ok(None);
                }
                if !readonly_guard_operand(left, fuel, span)?
                    || !readonly_guard_operand(right, fuel, span)?
                {
                    return Ok(None);
                }
                if !truth {
                    return Ok(Some(PatternGuard::Unknown));
                }
                let value = match (&left.kind, &right.kind) {
                    (K::Local(id), _) if *id == binding => right,
                    (_, K::Local(id)) if *id == binding => left,
                    _ => return Ok(Some(PatternGuard::Unknown)),
                };
                let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
                let mut budget = comptime_runtime::Budget {
                    steps: before,
                    max_limbs: 4096,
                };
                let value = self.known_normal_input(value, 0, &mut budget);
                *fuel -= (before - budget.steps) as u64;
                if let (Some(Term::Integer(value)), Term::Primitive(primitive)) = (&value, ty)
                    && let Some((signed, width)) = primitive.fixed_integer_layout()
                    && (if signed {
                        value.to_signed(width).is_err()
                    } else {
                        value.to_unsigned(width).is_err()
                    })
                {
                    return Ok(Some(PatternGuard::Unknown));
                }
                Ok(Some(
                    value.map_or(PatternGuard::Unknown, PatternGuard::Exact),
                ))
            },
        );
        self.required_steps = fuel;
        Ok(match result? {
            Some(PatternGuard::Exact(value)) => Some(value),
            _ => None,
        })
    }

    pub(super) fn checked_normal_input_in(
        &mut self,
        expr: &n::Expr,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps: before,
            max_limbs: 4096,
        };
        let observed = self.observe_normal_input_in(expr, 0, &mut budget, bindings);
        self.required_steps -= (before - budget.steps) as u64;
        observation::optional(observed)
            .map_err(|failure| error(span, format!("static value observation failed: {failure}")))
    }

    // Recover an identity from already elaborated structure, never by running a
    // function or reducing a normal-form body. Keep the original expression so
    // its execution order and invocation-time validation remain unchanged.
    pub(super) fn known_normal_input(
        &self,
        expr: &n::Expr,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
    ) -> Option<Term> {
        self.known_normal_input_in(expr, depth, budget, &BTreeMap::new())
    }

    pub(in crate::program) fn known_normal_input_in(
        &self,
        expr: &n::Expr,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Option<Term> {
        self.observe_normal_input_in(expr, depth, budget, bindings)
            .ok()
    }

    /// Inspect an immutable binding with the ordinary static value and pattern
    /// observers. Unavailable is not fallthrough evidence for a let-else.
    pub(in crate::program) fn observe_normal_binding(
        &self,
        statement: &n::Expr,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<(), Failure> {
        match &statement.kind {
            K::Let { binding, value } => {
                let value = self.observe_normal_input_in(value, depth + 1, budget, bindings)?;
                bindings.insert(*binding, std::sync::Arc::new(value));
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                ..
            } => {
                let value = self.observe_normal_input_in(value, depth + 1, budget, bindings)?;
                if !self.observe_normal_pattern(pattern, &value, depth + 1, budget, bindings)? {
                    return Err(Failure::Unavailable);
                }
                if let Some(guard) = guard {
                    if self.observe_normal_input_in(guard, depth + 1, budget, bindings)?
                        != Term::Bool(true)
                    {
                        return Err(Failure::Unavailable);
                    }
                }
            }
            _ => return Err(Failure::Unavailable),
        }
        Ok(())
    }

    pub(in crate::program) fn observe_normal_input_in(
        &self,
        expr: &n::Expr,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<Term, Failure> {
        if depth >= 128 {
            return Err(Failure::Depth);
        }
        budget.consume(1)?;
        match &expr.kind {
            K::TypeValue(ty) | K::ComptimeValue(ty) | K::StagedValue(ty) => {
                copy_observed_term(ty, budget)
            }
            K::OptionalAbsent if matches!(expr.ty, Term::Optional(_)) => Ok(Term::OptionalValue {
                ty: Box::new(copy_observed_term(&expr.ty, budget)?),
                value: None,
            }),
            K::OptionalPresent(value) => {
                let Term::Optional(payload) = &expr.ty else {
                    return Err(Failure::Unavailable);
                };
                if value.ty != **payload {
                    return Err(Failure::Unavailable);
                }
                Ok(Term::OptionalValue {
                    ty: Box::new(copy_observed_term(&expr.ty, budget)?),
                    value: Some(Box::new(self.observe_normal_input_in(
                        value,
                        depth + 1,
                        budget,
                        bindings,
                    )?)),
                })
            }
            K::TypeListIndex(values, index) => {
                let element = self
                    .list_element_type(&values.ty)
                    .ok_or(Failure::Unavailable)?;
                if *element != expr.ty {
                    return Err(Failure::Unavailable);
                }
                let frozen = self.observe_normal_input_in(values, depth + 1, budget, bindings)?;
                let mut values = take_known_list_elements(frozen, &values.ty, element, budget)?;
                let Term::Integer(index) =
                    self.observe_normal_input_in(index, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                budget.consume(index.bit_len().div_ceil(32).saturating_add(1))?;
                let index = usize::try_from(index.to_u128()?)
                    .map_err(|_| Failure::Invalid("list index overflow".into()))?;
                if index >= values.len() {
                    return Err(Failure::Invalid("list index out of bounds".into()));
                }
                Ok(values.swap_remove(index))
            }
            K::NormalFormType {
                head,
                arguments,
                plan,
            } => {
                let arguments = self.observe_declaration_arguments(
                    *head,
                    arguments,
                    arguments::Plan::normal(plan.as_ref()),
                    depth,
                    budget,
                    bindings,
                )?;
                Ok(Term::NormalForm {
                    head: *head,
                    sort: Sort::Type,
                    arguments,
                })
            }
            K::NominalType {
                declaration,
                arguments,
                preparations,
                bindings: input_bindings,
                order,
            } => {
                let arguments = self.observe_declaration_arguments(
                    *declaration,
                    arguments,
                    arguments::Plan {
                        preparations,
                        bindings: input_bindings.as_deref(),
                        order: order.as_deref(),
                    },
                    depth,
                    budget,
                    bindings,
                )?;
                Ok(Term::Nominal {
                    declaration: *declaration,
                    arguments,
                })
            }
            K::OptionalType(value) => {
                let inner = self.observe_normal_input_in(value, depth + 1, budget, bindings)?;
                (inner.sort() == Sort::Type)
                    .then(|| Term::Optional(Box::new(inner)))
                    .ok_or(Failure::Unavailable)
            }
            K::PointerType(value) => {
                let inner = self.observe_normal_input_in(value, depth + 1, budget, bindings)?;
                (inner.sort() == Sort::Type)
                    .then(|| Term::Pointer(Box::new(inner)))
                    .ok_or(Failure::Unavailable)
            }
            K::BitVecType(value) => {
                let Term::Integer(width) =
                    self.observe_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                budget.consume(width.bit_len().div_ceil(32).saturating_add(1))?;
                let width = usize::try_from(width.to_u128()?)
                    .map_err(|_| Failure::Invalid("bit-vector width overflow".into()))?;
                Ok(p(P::BitVec(width)))
            }
            K::FunctionType {
                comptime,
                comptime_parameters,
                parameters,
            } => {
                budget.consume(parameters.len().saturating_add(1))?;
                if comptime_parameters.len() != parameters.len() {
                    return Err(Failure::Unavailable);
                }
                let mut types = Vec::with_capacity(parameters.len());
                for parameter in parameters {
                    types.push(self.observe_normal_input_in(
                        parameter,
                        depth + 1,
                        budget,
                        bindings,
                    )?);
                }
                nir::callable_types::from_outer(
                    types,
                    comptime_parameters,
                    *comptime,
                    &mut budget.steps,
                )
                .map_err(Failure::from)
            }
            K::FunctionResultType { signature, result } => {
                let signature =
                    self.observe_normal_input_in(signature, depth + 1, budget, bindings)?;
                if !matches!(signature.callable_signature(), Term::Function { .. }) {
                    return Err(Failure::Unavailable);
                }
                let result = self.observe_normal_input_in(result, depth + 1, budget, bindings)?;
                if result.sort() != Sort::Type {
                    return Err(Failure::Unavailable);
                }
                nir::callable_types::with_result_from_outer(signature, result, &mut budget.steps)
                    .map_err(Failure::from)
            }
            K::ClosureType {
                environment,
                signature,
            } => {
                budget.consume(environment.len().saturating_add(1))?;
                let mut types = Vec::with_capacity(environment.len());
                for field in environment {
                    let ty = self.observe_normal_input_in(field, depth + 1, budget, bindings)?;
                    if ty.sort() != Sort::Type {
                        return Err(Failure::Unavailable);
                    }
                    types.push(ty);
                }
                let signature =
                    self.observe_normal_input_in(signature, depth + 1, budget, bindings)?;
                if signature.sort() != Sort::Type {
                    return Err(Failure::Unavailable);
                }
                if types.is_empty() {
                    Ok(signature)
                } else {
                    if !matches!(
                        signature,
                        Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
                    ) {
                        return Err(Failure::Unavailable);
                    }
                    Ok(Term::Closure {
                        environment: types,
                        signature: Box::new(signature),
                    })
                }
            }
            K::ListConcat(left, right) => {
                let element = self
                    .list_element_type(&expr.ty)
                    .ok_or(Failure::Unavailable)?;
                observe_list_concat(&expr.ty, element, left, right, budget, |input, budget| {
                    self.observe_normal_input_in(input, depth + 1, budget, bindings)
                })
            }
            K::TypeList(values) => {
                let element = self
                    .list_element_type(&expr.ty)
                    .ok_or(Failure::Unavailable)?;
                budget.consume(values.len())?;
                let mut fields = Vec::with_capacity(values.len());
                for value in values {
                    if value.ty != *element {
                        return Err(Failure::Unavailable);
                    }
                    fields.push(self.observe_normal_input_in(
                        value,
                        depth + 1,
                        budget,
                        bindings,
                    )?);
                }
                freeze_known_list(&expr.ty, element, fields, budget)
            }
            K::Object { names, fields }
                if expr.ty == p(P::Object) && names.len() == fields.len() =>
            {
                budget.consume(fields.len().saturating_mul(4))?;
                let mut entries = Vec::with_capacity(fields.len());
                for (key, field) in names.iter().zip(fields) {
                    if let Some(key) = key {
                        budget.consume(key.len())?;
                    }
                    let domain = copy_observed_term(&field.ty, budget)?;
                    let value = self.observe_normal_input_in(field, depth + 1, budget, bindings)?;
                    entries.push(nir::objects::entry(key.as_deref(), domain, value));
                }
                Ok(nir::objects::value(entries))
            }
            K::AnonymousRecord { fields } => {
                let Term::Record(types) = &expr.ty else {
                    return Err(Failure::Unavailable);
                };
                if fields.len() != types.len() {
                    return Err(Failure::Unavailable);
                }
                Ok(Term::RecordValue(
                    fields
                        .iter()
                        .zip(types)
                        .map(|(field, (name, ty))| {
                            budget.consume(name.len())?;
                            if field.ty != *ty {
                                return Err(Failure::Unavailable);
                            }
                            Ok((
                                name.clone(),
                                self.observe_normal_input_in(field, depth + 1, budget, bindings)?,
                            ))
                        })
                        .collect::<Result<Vec<_>, Failure>>()?,
                ))
            }
            K::Record {
                declaration,
                fields,
            }
            | K::Variant {
                declaration,
                fields,
                ..
            } => {
                let Term::Nominal {
                    declaration: actual,
                    ..
                } = &expr.ty
                else {
                    return Err(Failure::Unavailable);
                };
                if declaration != actual {
                    return Err(Failure::Unavailable);
                }
                if fields.len() > budget.steps {
                    return Err(Failure::Steps);
                }
                let variant = match &expr.kind {
                    K::Variant { index, .. } => Some(
                        u32::try_from(*index)
                            .map_err(|_| Failure::Invalid("variant index overflow".into()))?,
                    ),
                    _ => None,
                };
                Ok(Term::NominalValue {
                    ty: Box::new(copy_observed_term(&expr.ty, budget)?),
                    variant,
                    fields: fields
                        .iter()
                        .map(|field| {
                            self.observe_normal_input_in(field, depth + 1, budget, bindings)
                        })
                        .collect::<Result<Vec<_>, Failure>>()?,
                })
            }
            K::Field {
                value,
                declaration,
                index,
            } => {
                let Term::NominalValue {
                    ty,
                    variant: None,
                    mut fields,
                } = self.observe_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                let Term::Nominal {
                    declaration: actual,
                    ..
                } = ty.as_ref()
                else {
                    return Err(Failure::Unavailable);
                };
                if actual != declaration || *index >= fields.len() {
                    return Err(Failure::Unavailable);
                }
                Ok(fields.swap_remove(*index))
            }
            K::AnonymousField { value, name, index } => {
                let Term::RecordValue(mut fields) =
                    self.observe_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                let (actual, _) = fields
                    .get(*index)
                    .ok_or_else(|| Failure::Invalid("record field index out of bounds".into()))?;
                budget.consume(actual.len().saturating_add(name.len()))?;
                if actual != name {
                    return Err(Failure::Unavailable);
                }
                Ok(fields.swap_remove(*index).1)
            }
            K::Tuple(values) => Ok(Term::TupleValue(
                values
                    .iter()
                    .map(|value| self.observe_normal_input_in(value, depth + 1, budget, bindings))
                    .collect::<Result<Vec<_>, Failure>>()?,
            )),
            K::Unit => Ok(Term::Unit),
            K::Literal(literal) => known_scalars::literal(literal, budget),
            K::SymbolName(value) => {
                if expr.ty != p(P::Str) || value.ty != p(P::Symbol) {
                    return Err(Failure::Unavailable);
                }
                let Term::Symbol(name) =
                    self.observe_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                Ok(Term::String(name))
            }
            K::Not(value) => {
                match self.observe_normal_input_in(value, depth + 1, budget, bindings)? {
                    Term::Bool(value) => Ok(Term::Bool(!value)),
                    Term::BitVec(value) => Ok(Term::BitVec(value.not(budget)?)),
                    _ => Err(Failure::Unavailable),
                }
            }
            K::And(left, right) | K::Or(left, right) => {
                let left = self.observe_normal_input_in(left, depth + 1, budget, bindings)?;
                let conjunction = matches!(expr.kind, K::And(..));
                // Boolean operators short-circuit; bit-vector operators do not.
                // In particular an unreachable call must not erase a known
                // boolean context or consume its invocation budget here.
                if let Term::Bool(value) = &left {
                    if *value != conjunction {
                        return Ok(Term::Bool(*value));
                    }
                }
                let right = self.observe_normal_input_in(right, depth + 1, budget, bindings)?;
                match (left, right) {
                    (Term::Bool(a), Term::Bool(b)) => {
                        Ok(Term::Bool(if conjunction { a && b } else { a || b }))
                    }
                    (Term::BitVec(a), Term::BitVec(b)) => Ok(Term::BitVec(if conjunction {
                        a.and(&b, budget)
                    } else {
                        a.or(&b, budget)
                    }?)),
                    _ => Err(Failure::Unavailable),
                }
            }
            K::BitConcat(left, right) | K::BitEqual(left, right) => {
                let Term::BitVec(left) =
                    self.observe_normal_input_in(left, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                let Term::BitVec(right) =
                    self.observe_normal_input_in(right, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                if matches!(expr.kind, K::BitConcat(..)) {
                    Ok(Term::BitVec(left.concat(&right, budget)?))
                } else {
                    if left.width() != right.width() {
                        return Err(Failure::Unavailable);
                    }
                    budget.consume(left.words().len().saturating_add(1))?;
                    Ok(Term::Bool(left == right))
                }
            }
            K::BitSlice { value, start, end } => {
                let Term::BitVec(value) =
                    self.observe_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                Ok(Term::BitVec(value.slice(*start, *end, budget)?))
            }
            K::TakeBits(value) => {
                let Term::Primitive(primitive) = value.ty else {
                    return Err(Failure::Unavailable);
                };
                let known = self.observe_normal_input_in(value, depth + 1, budget, bindings)?;
                if let Term::BitVec(bits) = known {
                    return Ok(Term::BitVec(bits));
                }
                let Term::Integer(integer) = known else {
                    return Err(Failure::Unavailable);
                };
                let (signed, width) = primitive
                    .fixed_integer_layout()
                    .ok_or(Failure::Unavailable)?;
                let raw = if signed {
                    integer.to_signed(width)? as u128
                } else {
                    integer.to_unsigned(width)?
                };
                let raw = if width == 128 {
                    raw
                } else {
                    raw & ((1u128 << width) - 1)
                };
                Ok(Term::BitVec(
                    comptime_runtime::Integer::from_u128(raw).to_bitvec(width as usize, budget)?,
                ))
            }
            K::BitConvert(value) => {
                let Term::Primitive(P::BitVec(width)) = expr.ty else {
                    return Err(Failure::Unavailable);
                };
                let Term::BitVec(value) =
                    self.observe_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                Ok(Term::BitVec(value.widen(width, budget)?))
            }
            K::TypeEqual(left, right) => {
                let left = self.observe_normal_input_in(left, depth + 1, budget, bindings)?;
                let right = self.observe_normal_input_in(right, depth + 1, budget, bindings)?;
                known_type_equality(&self.context, &left, &right, budget).map(Term::Bool)
            }
            K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right) => {
                let left = self.observe_normal_input_in(left, depth + 1, budget, bindings)?;
                let right = self.observe_normal_input_in(right, depth + 1, budget, bindings)?;
                let op = match expr.kind {
                    K::ScalarEqual(..) => n::IntegerOp::Equal,
                    K::ScalarLess(..) => n::IntegerOp::Less,
                    K::ScalarLessEqual(..) => n::IntegerOp::LessEqual,
                    K::ScalarGreater(..) => n::IntegerOp::Greater,
                    K::ScalarGreaterEqual(..) => n::IntegerOp::GreaterEqual,
                    _ => unreachable!(),
                };
                known_scalars::compare(&left, &right, op, budget).map(Term::Bool)
            }
            K::FloatNegate(value) => Ok(
                match self.observe_normal_input_in(value, depth + 1, budget, bindings)? {
                    Term::Float(value) => Term::Float(value.negate(budget)?),
                    Term::Float32(bits) => Term::Float32(bits ^ (1 << 31)),
                    Term::Float64(bits) => Term::Float64(bits ^ (1 << 63)),
                    _ => return Err(Failure::Unavailable),
                },
            ),
            K::FloatBinary { op, left, right } => {
                let left = self.observe_normal_input_in(left, depth + 1, budget, bindings)?;
                let right = self.observe_normal_input_in(right, depth + 1, budget, bindings)?;
                Ok(match (left, right) {
                    (Term::Float(left), Term::Float(right)) => Term::Float(match op {
                        n::FloatOp::Add => left.add(&right, budget)?,
                        n::FloatOp::Subtract => left.subtract(&right, budget)?,
                        n::FloatOp::Multiply => left.multiply(&right, budget)?,
                        n::FloatOp::Divide => return Err(Failure::Unavailable),
                    }),
                    (Term::Float32(left), Term::Float32(right)) => {
                        Term::Float32(op.evaluate_f32(left, right))
                    }
                    (Term::Float64(left), Term::Float64(right)) => {
                        Term::Float64(op.evaluate_f64(left, right))
                    }
                    _ => return Err(Failure::Unavailable),
                })
            }
            K::IntegerNegate(value) => {
                let Term::Integer(value) =
                    self.observe_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                Ok(Term::Integer(value.negate(budget)?))
            }
            K::IntegerBinary { op, left, right } => {
                let Term::Integer(left) =
                    self.observe_normal_input_in(left, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                let Term::Integer(right) =
                    self.observe_normal_input_in(right, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                op.evaluate(&left, &right, budget).map_err(Failure::from)
            }
            K::Match { value, arms } => {
                let value = self.observe_normal_input_in(value, depth + 1, budget, bindings)?;
                for arm in arms {
                    budget.consume(bindings.len().saturating_add(1))?;
                    let mut scoped = bindings.clone();
                    if !self.observe_normal_pattern(
                        &arm.pattern,
                        &value,
                        depth + 1,
                        budget,
                        &mut scoped,
                    )? {
                        continue;
                    }
                    if let Some(guard) = &arm.guard {
                        let Term::Bool(accepted) =
                            self.observe_normal_input_in(guard, depth + 1, budget, &scoped)?
                        else {
                            return Err(Failure::Unavailable);
                        };
                        if !accepted {
                            continue;
                        }
                    }
                    return self.observe_normal_input_in(&arm.body, depth + 1, budget, &scoped);
                }
                Err(Failure::Unavailable)
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Inspect only a known condition and its selected branch. The
                // original expression remains in the input plan for execution;
                // this never evaluates calls or guesses the result of effects.
                let Term::Bool(condition) =
                    self.observe_normal_input_in(condition, depth + 1, budget, bindings)?
                else {
                    return Err(Failure::Unavailable);
                };
                if condition {
                    self.observe_normal_input_in(then_branch, depth + 1, budget, bindings)
                } else {
                    match else_branch {
                        Some(branch) => {
                            self.observe_normal_input_in(branch, depth + 1, budget, bindings)
                        }
                        None => Ok(Term::Unit),
                    }
                }
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                // Only immutable local construction is inspected. Calls,
                // assignments and control transfer retain invocation-time work.
                if statements.len() > budget.steps {
                    return Err(Failure::Steps);
                }
                budget.consume(bindings.len())?;
                let mut scoped = bindings.clone();
                for statement in statements {
                    budget.consume(1)?;
                    self.observe_normal_binding(statement, depth, budget, &mut scoped)?;
                }
                match result {
                    Some(result) => {
                        self.observe_normal_input_in(result, depth + 1, budget, &scoped)
                    }
                    None => Ok(Term::Unit),
                }
            }
            K::Local(binding) => {
                if let Some(value) = bindings.get(binding) {
                    return copy_observed_term(value, budget);
                }
                if let Some((symbol, _)) = self.signature_comptime_input(*binding) {
                    return copy_observed_term(&symbol, budget);
                }
                if self.locals.contains_key(binding) {
                    // Generator clause inputs use local expressions, but can
                    // already have a stable symbolic value. Keep that value
                    // instead of invoking an unavailable
                    // definition-time local. Unknown defaults stay unknown.
                    if self.generator_data.contains_key(binding)
                        && let Some(value) = self.normal_default_bindings.iter().find_map(
                            |(declaration, (local, _))| {
                                (local == binding)
                                    .then(|| self.alias_substitution.get(declaration))
                                    .flatten()
                            },
                        )
                    {
                        return copy_observed_term(value, budget);
                    }
                    if let Some((symbol, _)) = self.input_facts.values.get(binding) {
                        return copy_observed_term(symbol, budget);
                    }
                    if let Some(symbol) = self.input_facts.types.get(binding) {
                        return copy_observed_term(symbol, budget);
                    }
                }
                let parameters = self
                    .normal_parameters
                    .get(&self.signature_owner.ok_or(Failure::Unavailable)?)
                    .ok_or(Failure::Unavailable)?;
                let (declaration, _, _) = parameters
                    .iter()
                    .find(|(_, local, _)| local == binding)
                    .ok_or(Failure::Unavailable)?;
                Ok(Term::Rigid {
                    declaration: *declaration,
                    sort: Sort::ComptimeValue,
                })
            }
            _ => Err(Failure::Unavailable),
        }
    }
}

impl Checker<'_, '_> {
    pub(super) fn dependent_normal_argument(
        &mut self,
        input: &Expr<'_>,
        depth: usize,
    ) -> Result<n::NormalFormArgument, Error> {
        if depth >= 128 {
            return Err(error(input.span, "contextual input nesting limit exceeded"));
        }
        if matches!(input.kind, H::Null) {
            return Ok(n::NormalFormArgument::Null {
                source: source(input.span),
            });
        }
        if matches!(input.kind, H::Unit) {
            return Ok(n::NormalFormArgument::Unit {
                source: source(input.span),
            });
        }
        if let H::List(items) = &input.kind {
            return Ok(n::NormalFormArgument::List {
                source: source(input.span),
                items: items
                    .iter()
                    .map(|value| self.dependent_normal_argument(value, depth + 1))
                    .collect::<Result<_, _>>()?,
            });
        }
        if let H::Tuple(items) = &input.kind {
            return Ok(n::NormalFormArgument::Tuple {
                source: source(input.span),
                items: items
                    .iter()
                    .map(|value| self.dependent_normal_argument(value, depth + 1))
                    .collect::<Result<_, _>>()?,
            });
        }
        if let H::Match(value, arms) = &input.kind {
            let value = self.expr(value, None)?;
            let mut checked = Vec::new();
            for arm in *arms {
                checked.push(self.in_block_scope(arm.span, |this| {
                    let pattern = this.match_arm_pattern(arm, &value)?;
                    let body = this.under_matched_pattern_type_facts(
                        &pattern,
                        &value,
                        arm.body.span,
                        |this| this.dependent_normal_argument(arm.body, depth + 1),
                    )?;
                    Ok(n::NormalMatchArm {
                        pattern,
                        guard: None,
                        body,
                    })
                })?);
            }
            let patterns: Vec<_> = checked.iter().map(|arm| &arm.pattern).collect();
            self.require_match_coverage(&patterns, &value.ty, input.span)?;
            return Ok(n::NormalFormArgument::Match {
                source: source(input.span),
                value,
                arms: checked,
            });
        }
        if let H::When(arms) = &input.kind {
            let fallback = arms.iter().position(|arm| arm.cond.is_none());
            if fallback.is_some_and(|index| index + 1 != arms.len()) {
                return Err(error(input.span, "when fallback must be the final arm"));
            }
            // A when without fallback discards its result.
            if fallback.is_none() {
                return self.expr(input, None).map(n::NormalFormArgument::Value);
            }
            return self.in_block_scope(input.span, |this| {
                let mut branches = Vec::new();
                for arm in *arms {
                    let condition = arm
                        .cond
                        .map(|value| this.expr(value, Some(&p(P::Bool))))
                        .transpose()?;
                    let check = |this: &mut Self| {
                        this.in_block_scope(arm.span, |this| {
                            this.dependent_normal_argument(arm.body, depth + 1)
                        })
                    };
                    let body = match &condition {
                        Some(condition) => {
                            this.under_type_condition(condition, true, arm.body.span, check)?
                        }
                        None => check(this)?,
                    };
                    if let Some(condition) = &condition {
                        this.assume_type_condition(condition, false, arm.body.span)?;
                    }
                    // Each source arm owns its runtime locals as well.
                    let body = n::NormalFormArgument::Block {
                        source: source(arm.span),
                        statements: Vec::new(),
                        result: Some(Box::new(body)),
                    };
                    branches.push((condition, body));
                }
                let mut branches = branches.into_iter().rev();
                let (_, mut tail) = branches.next().unwrap();
                for (condition, body) in branches {
                    tail = n::NormalFormArgument::If {
                        source: source(input.span),
                        condition: condition.unwrap(),
                        then_input: Box::new(body),
                        else_input: Box::new(tail),
                    };
                }
                Ok(tail)
            });
        }
        if let H::If(condition, yes, Some(no)) = &input.kind {
            let condition = self.expr(condition, Some(&p(P::Bool)))?;
            let then_input = self.under_type_condition(&condition, true, input.span, |this| {
                this.dependent_normal_block(yes, depth + 1)
            })?;
            let else_input = self.under_type_condition(&condition, false, input.span, |this| {
                this.dependent_normal_argument(no, depth + 1)
            })?;
            return Ok(n::NormalFormArgument::If {
                source: source(input.span),
                condition,
                then_input: Box::new(then_input),
                else_input: Box::new(else_input),
            });
        }
        if let H::Block(block) = &input.kind {
            return self.dependent_normal_block(block, depth + 1);
        }
        let (literal, negative) = match &input.kind {
            H::Unary(UnOp::Neg, inner) => (*inner, true),
            _ => (input, false),
        };
        if let H::Lit(hir::Lit {
            kind: hir::LitKind::Integer(spelling),
            ..
        }) = &literal.kind
        {
            comptime_runtime::Integer::parse_literal(
                spelling,
                &mut comptime_runtime::Budget::default(),
            )
            .map_err(|e| error(input.span, e.to_string()))?;
            return Ok(n::NormalFormArgument::Integer {
                source: source(input.span),
                spelling: spelling.clone(),
                negative,
            });
        }
        let saved_defaults = std::mem::replace(&mut self.numeric_defaults, false);
        let result = self.expr(input, None);
        self.numeric_defaults = saved_defaults;
        result.map(n::NormalFormArgument::Value)
    }
    fn dependent_normal_block(
        &mut self,
        block: &hir::Block<'_>,
        depth: usize,
    ) -> Result<n::NormalFormArgument, Error> {
        if depth >= 128 {
            return Err(error(block.span, "contextual input nesting limit exceeded"));
        }
        self.check_block_region(block, block_regions::ContextualTail { depth })
    }
}

// Bound copied semantic structure as well as expression visits. Local values
// are shared while inspecting a block, but returning an owned term must pay for
// expansion (e.g. repeated tuple duplication).
fn collect_known_normal_argument(
    argument: &n::NormalFormArgument,
    domain: Option<&Term>,
    list_head: Option<DeclarationId>,
    budget: &mut comptime_runtime::Budget,
    mut known: impl FnMut(&n::Expr, &mut comptime_runtime::Budget) -> Result<Term, Failure>,
) -> Result<Term, Failure> {
    budget.consume(1)?;
    let n::NormalFormArgument::List { items, .. } = argument else {
        return known(argument.expression().ok_or(Failure::Unavailable)?, budget);
    };
    let domain = domain.ok_or(Failure::Unavailable)?;
    let Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments,
    } = domain
    else {
        return Err(Failure::Unavailable);
    };
    if Some(*head) != list_head {
        return Err(Failure::Unavailable);
    }
    let [element] = arguments.as_slice() else {
        return Err(Failure::Unavailable);
    };
    if element.sort() != Sort::Type {
        return Err(Failure::Unavailable);
    }
    let types = *element == p(P::Type);
    let mut collected = Vec::new();
    for item in items {
        budget.consume(1)?;
        match item {
            n::NormalFormArgument::Value(expr) => {
                let value = known(expr, budget)?;
                if types && value.sort() != Sort::Type {
                    return Err(Failure::Unavailable);
                }
                collected.push(value);
            }
            n::NormalFormArgument::ListSpread(expr) => {
                let fields =
                    take_known_list_elements(known(expr, budget)?, domain, element, budget)?;
                collected.extend(fields);
            }
            _ => return Err(Failure::Unavailable),
        }
    }
    freeze_known_list(domain, element, collected, budget)
}

// Observe ordinary list assembly in leaf order. This shares the bounded
// payload reader/writer with literal lists and never invokes a producer.
fn observe_list_concat(
    domain: &Term,
    element: &Term,
    left: &n::Expr,
    right: &n::Expr,
    budget: &mut comptime_runtime::Budget,
    mut observe: impl FnMut(&n::Expr, &mut comptime_runtime::Budget) -> Result<Term, Failure>,
) -> Result<Term, Failure> {
    nir::type_computations::charge_term(domain, &mut budget.steps)?;
    let mut fields = Vec::new();
    for input in [left, right] {
        nir::type_computations::charge_term(&input.ty, &mut budget.steps)?;
        if input.ty != *domain {
            return Err(Failure::Unavailable);
        }
        let value = observe(input, budget)?;
        let values = take_known_list_elements(value, domain, element, budget)?;
        budget.consume(values.len())?;
        fields.extend(values);
    }
    freeze_known_list(domain, element, fields, budget)
}

// Callers establish the registered list domain before using this representation
// reader. Payload encoding does not grant collection authority.
fn take_known_list_elements(
    value: Term,
    domain: &Term,
    element: &Term,
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<Term>, Failure> {
    let fields = match value {
        Term::NominalValue {
            ty,
            variant: None,
            fields,
        } if ty.as_ref() == domain => fields,
        _ => return Err(Failure::Unavailable),
    };
    budget.consume(fields.len())?;
    let sort = if *element == p(P::Type) {
        Sort::Type
    } else {
        Sort::ComptimeValue
    };
    fields
        .iter()
        .all(|field| field.sort() == sort)
        .then_some(fields)
        .ok_or_else(|| Failure::Invalid("list element sort mismatch".into()))
}

// Keep one canonical writer until all frozen-list consumers migrate together.
fn freeze_known_list(
    domain: &Term,
    element: &Term,
    fields: Vec<Term>,
    budget: &mut comptime_runtime::Budget,
) -> Result<Term, Failure> {
    if !matches!(domain, Term::NormalForm { arguments, .. }
        if matches!(arguments.as_slice(), [actual] if actual == element))
    {
        return Err(Failure::Unavailable);
    }
    nir::comptime_values::freeze_admitted_list(domain, fields, &mut budget.steps)
        .map_err(Failure::from)
}

/// A type relation needs a proof in either direction. A failed equality query
/// is not a disequality fact, and an opaque head is never reduced to its body.
fn known_type_equality(
    context: &crate::ptc::Context,
    left: &Term,
    right: &Term,
    budget: &mut comptime_runtime::Budget,
) -> Result<bool, Failure> {
    use crate::ptc::{Formula, Outcome, UnknownReason};
    if left.sort() != Sort::Type || right.sort() != Sort::Type {
        return Err(Failure::Unavailable);
    }
    let equality = Formula::equal(
        copy_observed_term(left, budget)?,
        copy_observed_term(right, budget)?,
    );
    let inequality = Formula::equal(
        copy_observed_term(left, budget)?,
        copy_observed_term(right, budget)?,
    )
    .negate();
    let mut proof_budget = crate::ptc::Budget {
        steps: budget.steps,
        max_depth: 128,
    };
    let result = (|| {
        for (proposition, value) in [(equality, true), (inequality, false)] {
            match context.prove_metered(proposition, &mut proof_budget) {
                Outcome::Entailed(_) => return Ok(value),
                Outcome::Unknown {
                    reason: UnknownReason::BudgetExhausted,
                    ..
                } => return Err(Failure::Proof),
                Outcome::IllFormed(message) => return Err(Failure::Invalid(message)),
                Outcome::NotEntailed(_) | Outcome::Unknown { .. } => {}
            }
        }
        Err(Failure::Unavailable)
    })();
    budget.steps = proof_budget.steps;
    result
}

fn copy_observed_term(term: &Term, budget: &mut comptime_runtime::Budget) -> Result<Term, Failure> {
    nir::copy_budget::clone_term(term, &mut budget.steps).map_err(Failure::from)
}

pub(super) fn clone_known_term(term: &Term, budget: &mut comptime_runtime::Budget) -> Option<Term> {
    // Opportunistic observation may decline to supply evidence. Required copies
    // call the typed service directly so a resource limit remains a diagnostic.
    nir::copy_budget::clone_term(term, &mut budget.steps).ok()
}

#[cfg(test)]
mod tests {
    use super::clone_known_term;
    use super::known_type_equality;
    use super::observation::Failure;
    use comptime_runtime::Budget;
    use nir::Term;
    use nir::{Primitive, Sort};

    #[test]
    fn list_concat_observation_preserves_domains_order_and_failure_recovery() {
        use nir::program::{Expr, ExprKind, SourceRange};
        let element = Term::Primitive(Primitive::Bool);
        let domain = Term::NormalForm {
            head: nir::DeclarationId {
                package: 1,
                local: 900,
            },
            sort: Sort::Type,
            arguments: vec![element.clone()],
        };
        let input = |flag| Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: domain.clone(),
            kind: ExprKind::ComptimeValue(Term::NominalValue {
                ty: Box::new(domain.clone()),
                variant: None,
                fields: vec![Term::Bool(flag)],
            }),
        };
        let left = input(true);
        let right = input(false);
        let read = |expr: &Expr, _: &mut Budget| {
            let ExprKind::ComptimeValue(value) = &expr.kind else {
                unreachable!()
            };
            Ok(value.clone())
        };
        for failure in [
            Failure::Unavailable,
            Failure::Steps,
            Failure::Depth,
            Failure::Size,
            Failure::Structure,
            Failure::Proof,
            Failure::Invalid("bad list".into()),
        ] {
            let mut visits = 0;
            assert_eq!(
                super::observe_list_concat(
                    &domain,
                    &element,
                    &left,
                    &right,
                    &mut Budget::default(),
                    |_, _| {
                        visits += 1;
                        Err(failure.clone())
                    }
                ),
                Err(failure)
            );
            assert_eq!(visits, 1);
        }
        assert!(
            super::observe_list_concat(
                &domain,
                &element,
                &left,
                &right,
                &mut Budget {
                    steps: 0,
                    ..Default::default()
                },
                read
            )
            .is_err()
        );
        let mut wrong = right.clone();
        wrong.ty = Term::Tuple(Vec::new());
        let mut visits = 0;
        assert_eq!(
            super::observe_list_concat(
                &domain,
                &element,
                &left,
                &wrong,
                &mut Budget::default(),
                |expr, budget| {
                    visits += 1;
                    read(expr, budget)
                }
            ),
            Err(Failure::Unavailable)
        );
        assert_eq!(visits, 1);
        let value = super::observe_list_concat(
            &domain,
            &element,
            &left,
            &right,
            &mut Budget::default(),
            read,
        )
        .unwrap();
        assert_eq!(
            super::take_known_list_elements(value, &domain, &element, &mut Budget::default()),
            Ok(vec![Term::Bool(true), Term::Bool(false)])
        );
    }

    #[test]
    fn known_type_list_collection_preserves_order_and_stops_at_unknowns() {
        use super::collect_known_normal_argument;
        use nir::program::{Expr, ExprKind as K, NormalFormArgument as A, SourceRange};
        use nir::{Primitive, Sort};
        let list_id = nir::DeclarationId {
            package: 0,
            local: 90,
        };
        let list = Term::NormalForm {
            head: list_id,
            sort: Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Type)],
        };
        let source = SourceRange { start: 0, end: 0 };
        let expr = |kind| Expr {
            source,
            ty: match &kind {
                K::ComptimeValue(Term::NominalValue { ty, .. }) => ty.as_ref().clone(),
                _ => Term::Primitive(Primitive::Type),
            },
            kind,
        };
        let input = |items| A::List { source, items };
        let flag = Term::Primitive(Primitive::Bool);
        let byte = Term::Primitive(Primitive::U8);
        let resolve = |e: &Expr, budget: &mut Budget| match &e.kind {
            K::TypeValue(ty) | K::ComptimeValue(ty) => super::copy_observed_term(ty, budget),
            _ => Err(Failure::Unavailable),
        };
        let valid = input(vec![
            A::Value(expr(K::TypeValue(flag.clone()))),
            A::ListSpread(expr(K::ComptimeValue(Term::NominalValue {
                ty: Box::new(list.clone()),
                variant: None,
                fields: vec![byte.clone(), flag.clone()],
            }))),
            A::ListSpread(expr(K::ComptimeValue(Term::NominalValue {
                ty: Box::new(list.clone()),
                variant: None,
                fields: vec![],
            }))),
            A::ListSpread(expr(K::ComptimeValue(Term::NominalValue {
                ty: Box::new(list.clone()),
                variant: None,
                fields: vec![byte.clone()],
            }))),
        ]);
        assert_eq!(
            collect_known_normal_argument(
                &valid,
                Some(&list),
                Some(list_id),
                &mut Budget::default(),
                resolve
            ),
            Ok(Term::NominalValue {
                ty: Box::new(list.clone()),
                variant: None,
                fields: vec![flag.clone(), byte.clone(), flag.clone(), byte],
            })
        );
        assert!(
            collect_known_normal_argument(
                &valid,
                Some(&list),
                Some(list_id),
                &mut Budget {
                    steps: 1,
                    max_limbs: 4096
                },
                resolve
            )
            .is_err()
        );
        assert!(
            collect_known_normal_argument(
                &valid,
                Some(&list),
                Some(list_id),
                &mut Budget::default(),
                resolve
            )
            .is_ok()
        );
        for invalid in [
            A::Value(expr(K::ComptimeValue(Term::NominalValue {
                ty: Box::new(list.clone()),
                variant: None,
                fields: vec![],
            }))),
            A::ListSpread(expr(K::TypeValue(flag))),
            A::ListSpread(expr(K::ComptimeValue(Term::NominalValue {
                ty: Box::new(list.clone()),
                variant: None,
                fields: vec![Term::Bool(true)],
            }))),
            A::ListSpread(expr(K::TypeValue(Term::Rigid {
                declaration: nir::DeclarationId {
                    package: 1,
                    local: 1,
                },
                sort: Sort::ComptimeValue,
            }))),
        ] {
            assert!(
                collect_known_normal_argument(
                    &input(vec![invalid]),
                    Some(&list),
                    Some(list_id),
                    &mut Budget::default(),
                    resolve
                )
                .is_err()
            );
        }
        let unknown = input(vec![
            A::Value(expr(K::Unit)),
            A::Value(expr(K::TypeValue(Term::Primitive(Primitive::Bool)))),
        ]);
        let mut visits = 0;
        assert!(
            collect_known_normal_argument(
                &unknown,
                Some(&list),
                Some(list_id),
                &mut Budget::default(),
                |e, b| {
                    visits += 1;
                    resolve(e, b)
                }
            )
            .is_err()
        );
        assert_eq!(visits, 1);
    }

    #[test]
    fn known_collections_require_a_domain_and_preserve_non_type_list_identity() {
        use nir::program::{Expr, ExprKind as K, NormalFormArgument as A, SourceRange};
        let source = SourceRange { start: 0, end: 0 };
        let head = nir::DeclarationId {
            package: 0,
            local: 90,
        };
        let domain = |element| Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            arguments: vec![element],
        };
        let boolean = domain(Term::Primitive(Primitive::Bool));
        let character = domain(Term::Primitive(Primitive::Char));
        let nominal = |ty: &Term, fields| Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields,
        };
        let empty = A::List {
            source,
            items: vec![],
        };
        let resolve = |expr: &Expr, budget: &mut Budget| match &expr.kind {
            K::ComptimeValue(value) => super::copy_observed_term(value, budget),
            _ => Err(Failure::Unavailable),
        };
        assert_eq!(
            super::collect_known_normal_argument(
                &empty,
                Some(&boolean),
                Some(head),
                &mut Budget::default(),
                resolve
            ),
            Ok(nominal(&boolean, vec![]))
        );
        assert_eq!(
            super::collect_known_normal_argument(
                &empty,
                Some(&character),
                Some(head),
                &mut Budget::default(),
                resolve
            ),
            Ok(nominal(&character, vec![]))
        );
        assert!(
            super::collect_known_normal_argument(
                &empty,
                None,
                Some(head),
                &mut Budget::default(),
                resolve
            )
            .is_err()
        );
        assert!(
            super::collect_known_normal_argument(
                &empty,
                Some(&boolean),
                None,
                &mut Budget::default(),
                resolve
            )
            .is_err()
        );
        let spread = |ty: &Term, value| A::List {
            source,
            items: vec![A::ListSpread(Expr {
                source,
                ty: ty.clone(),
                kind: K::ComptimeValue(value),
            })],
        };
        let values = nominal(&boolean, vec![Term::Bool(true), Term::Bool(false)]);
        assert_eq!(
            super::collect_known_normal_argument(
                &spread(&boolean, values.clone()),
                Some(&boolean),
                Some(head),
                &mut Budget::default(),
                resolve
            ),
            Ok(values.clone())
        );
        assert!(
            super::collect_known_normal_argument(
                &spread(&boolean, values),
                Some(&character),
                Some(head),
                &mut Budget::default(),
                resolve
            )
            .is_err()
        );
        assert!(
            super::collect_known_normal_argument(
                &empty,
                Some(&boolean),
                Some(head),
                &mut Budget {
                    steps: 1,
                    max_limbs: 4096
                },
                resolve
            )
            .is_err()
        );
    }

    #[test]
    fn known_list_payloads_require_exact_domains_and_valid_element_sorts() {
        use super::{freeze_known_list, take_known_list_elements};
        let domain = |element| Term::NormalForm {
            head: nir::DeclarationId {
                package: 3,
                local: 1,
            },
            sort: Sort::Type,
            arguments: vec![element],
        };
        let type_domain = domain(Term::Primitive(Primitive::Type));
        let bool_domain = domain(Term::Primitive(Primitive::Bool));
        let nominal = |ty: &Term, fields| Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields,
        };
        for (ty, element, fields) in [
            (
                type_domain.clone(),
                Term::Primitive(Primitive::Type),
                vec![Term::Primitive(Primitive::Bool)],
            ),
            (
                bool_domain.clone(),
                Term::Primitive(Primitive::Bool),
                vec![Term::Bool(true)],
            ),
        ] {
            let input = nominal(&ty, fields.clone());
            assert_eq!(
                take_known_list_elements(input.clone(), &ty, &element, &mut Budget::default()),
                Ok(fields.clone())
            );
            assert!(
                take_known_list_elements(
                    input.clone(),
                    &ty,
                    &element,
                    &mut Budget {
                        steps: 0,
                        max_limbs: 4096
                    }
                )
                .is_err()
            );
            assert_eq!(
                take_known_list_elements(input, &ty, &element, &mut Budget::default()),
                Ok(fields.clone())
            );
            let frozen =
                freeze_known_list(&ty, &element, fields.clone(), &mut Budget::default()).unwrap();
            assert_eq!(
                take_known_list_elements(frozen, &ty, &element, &mut Budget::default()),
                Ok(fields)
            );
        }
        for bad in [
            Term::TupleValue(vec![]),
            Term::TupleValue(vec![Term::Primitive(Primitive::Bool)]),
            nominal(&bool_domain, vec![Term::Primitive(Primitive::Bool)]),
            nominal(&type_domain, vec![Term::Bool(true)]),
            Term::NominalValue {
                ty: Box::new(type_domain.clone()),
                variant: Some(0),
                fields: vec![],
            },
        ] {
            assert!(
                take_known_list_elements(
                    bad,
                    &type_domain,
                    &Term::Primitive(Primitive::Type),
                    &mut Budget::default()
                )
                .is_err()
            );
        }
        assert!(
            take_known_list_elements(
                Term::TupleValue(vec![]),
                &bool_domain,
                &Term::Primitive(Primitive::Bool),
                &mut Budget::default()
            )
            .is_err()
        );
    }

    #[test]
    fn known_type_equality_requires_proofs_and_keeps_heads_opaque() {
        let head = nir::DeclarationId {
            package: 0,
            local: 900,
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let character = Term::Primitive(Primitive::Char);
        let opaque = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![boolean.clone()],
        };
        let mut context = crate::ptc::Context::default();
        context.admit_congruence(head);
        assert_eq!(
            known_type_equality(&context, &boolean, &boolean, &mut Budget::default()),
            Ok(true)
        );
        assert_eq!(
            known_type_equality(&context, &boolean, &character, &mut Budget::default()),
            Ok(false)
        );
        assert_eq!(
            known_type_equality(&context, &opaque, &boolean, &mut Budget::default()),
            Err(Failure::Unavailable)
        );
        assert_eq!(
            known_type_equality(&context, &opaque, &opaque, &mut Budget::default()),
            Ok(true)
        );
        let mut empty = Budget {
            steps: 0,
            max_limbs: 4096,
        };
        assert_eq!(
            known_type_equality(&context, &boolean, &boolean, &mut empty),
            Err(Failure::Steps)
        );
        assert_eq!(
            known_type_equality(&context, &boolean, &boolean, &mut Budget::default()),
            Ok(true)
        );
        assert_eq!(
            known_type_equality(
                &context,
                &Term::Bool(true),
                &Term::Bool(true),
                &mut Budget::default()
            ),
            Err(Failure::Unavailable)
        );
    }

    #[test]
    fn type_observation_distinguishes_copy_proof_limits_and_unknown_relations() {
        let boolean = Term::Primitive(Primitive::Bool);
        let mut budget = Budget {
            steps: 4,
            ..Default::default()
        };
        assert_eq!(
            known_type_equality(&Default::default(), &boolean, &boolean, &mut budget),
            Err(Failure::Proof)
        );
        assert_eq!(
            known_type_equality(
                &Default::default(),
                &boolean,
                &boolean,
                &mut Budget {
                    steps: 3,
                    ..Default::default()
                }
            ),
            Err(Failure::Steps)
        );
        assert_eq!(
            known_type_equality(
                &Default::default(),
                &boolean,
                &boolean,
                &mut Budget::default()
            ),
            Ok(true)
        );
    }

    #[test]
    fn collection_observation_propagates_failure_without_reading_later_inputs() {
        use nir::program::{Expr, ExprKind, NormalFormArgument as A, SourceRange};
        let head = nir::DeclarationId {
            package: 1,
            local: 17,
        };
        let domain = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Bool)],
        };
        let source = SourceRange { start: 0, end: 1 };
        let value = Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: ExprKind::ComptimeValue(Term::Bool(true)),
        };
        let argument = A::List {
            source,
            items: vec![A::Value(value.clone()), A::Value(value)],
        };
        for failure in [
            Failure::Unavailable,
            Failure::Steps,
            Failure::Depth,
            Failure::Size,
            Failure::Structure,
            Failure::Proof,
            Failure::Invalid("invalid input".into()),
        ] {
            let mut visits = 0;
            let result = super::collect_known_normal_argument(
                &argument,
                Some(&domain),
                Some(head),
                &mut Budget::default(),
                |_, _| {
                    visits += 1;
                    Err(failure.clone())
                },
            );
            assert_eq!(result, Err(failure));
            assert_eq!(visits, 1);
        }
        let value = super::collect_known_normal_argument(
            &argument,
            Some(&domain),
            Some(head),
            &mut Budget::default(),
            |_, _| Ok(Term::Bool(true)),
        )
        .unwrap();
        assert_eq!(
            super::take_known_list_elements(
                value,
                &domain,
                &Term::Primitive(Primitive::Bool),
                &mut Budget::default()
            ),
            Ok(vec![Term::Bool(true), Term::Bool(true)])
        );
    }

    #[test]
    fn observed_copy_cannot_hide_payloads_or_depth_in_callable_and_error_terms() {
        fn wrappers(value: &Term) -> Vec<Term> {
            let id = nir::DeclarationId {
                package: 1,
                local: 1,
            };
            let mut terms = vec![
                Term::ErrorResult {
                    errors: vec![value.clone()],
                    result: Box::new(Term::Unit),
                },
                Term::ErrorResult {
                    errors: vec![],
                    result: Box::new(value.clone()),
                },
                Term::ClosureContextValue(vec![value.clone()]),
                Term::ContextualClosure {
                    context: Box::new(value.clone()),
                    signature: Box::new(Term::Unit),
                },
                Term::ContextualClosure {
                    context: Box::new(Term::ClosureContextType),
                    signature: Box::new(value.clone()),
                },
                Term::FunctionValue {
                    declaration: id,
                    arguments: vec![value.clone()],
                },
                Term::ClosureSnapshot {
                    factory: id,
                    arguments: vec![value.clone()],
                },
            ];
            // Each method identity owns three independent sources of terms.
            for slot in 0..3 {
                terms.push(Term::MethodValue(Box::new(nir::TraitMethodIdentity {
                    interface: nir::program::InterfaceInstance {
                        declaration: id,
                        arguments: if slot == 0 {
                            vec![value.clone()]
                        } else {
                            vec![]
                        },
                    },
                    subject: if slot == 1 { value.clone() } else { Term::Unit },
                    member: id,
                    method_arguments: if slot == 2 {
                        vec![value.clone()]
                    } else {
                        vec![]
                    },
                })));
            }
            terms
        }
        for value in wrappers(&Term::String("x".repeat(128).into())) {
            let mut short = Budget {
                steps: 64,
                ..Default::default()
            };
            assert!(clone_known_term(&value, &mut short).is_none(), "{value}");
            assert_eq!(
                clone_known_term(&value, &mut Budget::default()),
                Some(value)
            );
        }
        let deep = (0..128).fold(Term::Unit, |inner, _| Term::Optional(Box::new(inner)));
        for value in wrappers(&deep) {
            assert!(clone_known_term(&value, &mut Budget::default()).is_none());
        }
    }

    #[test]
    fn copied_exact_decimal_charges_payload_before_cloning() {
        let value = comptime_runtime::decimal::DecimalValue::parse_literal(
            "12345678901234567890.123456789e123456789012345678901234567890",
            &mut Budget::default(),
        )
        .unwrap();
        let term = Term::Float(value);
        let mut insufficient = Budget {
            steps: 2,
            max_limbs: 4096,
        };
        assert!(clone_known_term(&term, &mut insufficient).is_none());
        assert_eq!(clone_known_term(&term, &mut Budget::default()), Some(term));
    }

    #[test]
    fn copied_identity_structure_consumes_work_before_expanding() {
        let term = Term::TupleValue(vec![Term::String("λ".into()), Term::Unit]);
        let mut short = Budget {
            steps: 4,
            max_limbs: 4096,
        };
        assert!(clone_known_term(&term, &mut short).is_none());
        let mut sufficient = Budget {
            steps: 5,
            max_limbs: 4096,
        };
        assert_eq!(clone_known_term(&term, &mut sufficient), Some(term));
        assert_eq!(sufficient.steps, 0);
        let mut deep = Term::Unit;
        for _ in 0..130 {
            deep = Term::TupleValue(vec![deep]);
        }
        assert!(clone_known_term(&deep, &mut Budget::default()).is_none());
    }
}

impl Checker<'_, '_> {
    pub(super) fn check_normal_defaults(&mut self, owner: OwnerId) -> Result<(), Error> {
        let clauses = self.type_clause_params(owner);
        let parameters = self.normal_parameters[&owner].clone();
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_forbidden = self.forbidden_default_inputs.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_types = std::mem::take(&mut self.alias_substitution);
        let saved_values = std::mem::take(&mut self.alias_value_bindings);
        let saved_bindings = std::mem::take(&mut self.normal_default_bindings);
        self.context = self.binder_context(owner);
        let result = (|| {
            for (slot, clause) in clauses.iter().enumerate() {
                let hir::clause::ClauseParamKind::Optional(_, _, value) = clause.kind else {
                    continue;
                };
                self.forbidden_default_inputs
                    .extend(parameters[slot..].iter().map(|p| p.0));
                self.expr(value, Some(&parameters[slot].2))?;
                self.forbidden_default_inputs = saved_forbidden.clone();
            }
            Ok(())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.in_required_comptime = saved_mode;
        self.forbidden_default_inputs = saved_forbidden;
        self.locals = saved_locals;
        self.alias_substitution = saved_types;
        self.alias_value_bindings = saved_values;
        self.normal_default_bindings = saved_bindings;
        result
    }

    fn planned_normal_expression(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let parameters = self.normal_parameters[&owner]
            .iter()
            .map(|(id, _, domain)| {
                (
                    *id,
                    if *domain == p(P::Type) {
                        Sort::Type
                    } else {
                        Sort::ComptimeValue
                    },
                    domain.clone(),
                )
            })
            .collect::<Vec<_>>();
        let prepared = self.planned_declaration_inputs(owner, inputs, span, &parameters, None)?;
        let ty = if prepared.diverges() {
            p(P::Never)
        } else {
            p(P::Type)
        };
        self.check_formation_application(owner, prepared.formation, span)?;
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, span)?;
        }
        Ok(n::Expr {
            source: source(span),
            ty,
            kind: K::NormalFormType {
                head: declaration(self.package, &self.package_identities, owner),
                arguments: prepared.arguments,
                plan: Some(prepared.plan),
            },
        })
    }
}

impl Checker<'_, '_> {
    /// Normalize independent concrete computations while retaining unrelated
    /// symbolic Type inputs. Preflight before invoking anything: contextual
    /// inputs must keep their complete runtime conversion plan.
    pub(super) fn required_normal_type_plan(
        &mut self,
        source: &Expr<'_>,
        checked: &n::Expr,
    ) -> Result<Option<Term>, Error> {
        let K::NormalFormType {
            head,
            arguments,
            plan: Some(plan),
        } = &checked.kind
        else {
            return Ok(None);
        };
        if !plan.valid(arguments.len()) || arguments.iter().any(|a| a.expression().is_none()) {
            return Ok(None);
        }
        let mut scoped: BTreeMap<n::LocalId, std::sync::Arc<Term>> = BTreeMap::new();
        let mut binding_types: BTreeMap<n::LocalId, Term> = BTreeMap::new();
        let mut values = vec![None; arguments.len()];
        if !plan.preparations.is_empty() {
            return Ok(None);
        }
        for &slot in &plan.order {
            let input = arguments[slot].expression().unwrap();
            let known = self.checked_normal_input_in(input, &scoped, source.span)?;
            let term = if let Some(known) = known {
                known
            } else {
                let mut referenced = BTreeSet::new();
                let mut fuel = self.required_steps as usize;
                let before = fuel;
                let visited = iir::visit::expression(input, &mut fuel, |value, _| {
                    if let K::Local(binding) = value.kind {
                        if scoped.contains_key(&binding) {
                            referenced.insert(binding);
                        }
                    }
                    Ok(())
                });
                self.required_steps -= (before - fuel) as u64;
                visited.map_err(|message| error(source.span, message))?;
                let mut inputs = Vec::new();
                for binding in referenced {
                    let value = self.copy_alias_type(&scoped[&binding], source.span)?;
                    inputs.push((binding, binding_types[&binding].clone(), value));
                }
                let (value, _) = self.required_normal_input(source, input.clone(), inputs)?;
                self.required_normal_term(value, source.span)?
            };
            let bound = self.copy_alias_type(&term, source.span)?;
            scoped.insert(plan.bindings[slot], std::sync::Arc::new(bound));
            binding_types.insert(plan.bindings[slot], input.ty.clone());
            values[slot] = Some(term);
        }
        Ok(Some(Term::NormalForm {
            head: *head,
            sort: Sort::Type,
            arguments: values
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| error(source.span, "incomplete normal-form input plan"))?,
        }))
    }
}

/// Possible Boolean results (bit 0 = false, bit 1 = true) for a fixed subject.
/// Other Boolean locals
/// remain unknown; their correlations may be lost, but no result is excluded
/// without evidence. This is a non-mutating syntactic subset, not invocation.
pub(super) fn boolean_guard_values(
    expr: &n::Expr,
    subject: n::LocalId,
    candidate: bool,
    fuel: &mut u64,
    span: Span,
    depth: usize,
) -> Result<Option<u8>, Error> {
    if depth >= 128 {
        return Err(error(span, "Boolean guard evidence depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "Boolean guard evidence budget exhausted"))?;
    if expr.ty != p(P::Bool) {
        return Ok(None);
    }
    Ok(Some(match &expr.kind {
        K::Literal(n::Literal::Bool(value)) => 1 << u8::from(*value),
        K::Local(id) => {
            if *id == subject {
                1 << u8::from(candidate)
            } else {
                3
            }
        }
        K::Block {
            statements,
            result: Some(result),
        } if statements.is_empty() => {
            return boolean_guard_values(result, subject, candidate, fuel, span, depth + 1);
        }
        K::If {
            condition,
            then_branch,
            else_branch: Some(else_branch),
        } => {
            let Some(condition) =
                boolean_guard_values(condition, subject, candidate, fuel, span, depth + 1)?
            else {
                return Ok(None);
            };
            let Some(yes) =
                boolean_guard_values(then_branch, subject, candidate, fuel, span, depth + 1)?
            else {
                return Ok(None);
            };
            let Some(no) =
                boolean_guard_values(else_branch, subject, candidate, fuel, span, depth + 1)?
            else {
                return Ok(None);
            };
            (if condition & 2 != 0 { yes } else { 0 }) | (if condition & 1 != 0 { no } else { 0 })
        }
        K::Not(value) => {
            let Some(mask) =
                boolean_guard_values(value, subject, candidate, fuel, span, depth + 1)?
            else {
                return Ok(None);
            };
            ((mask & 1) << 1) | ((mask & 2) >> 1)
        }
        K::And(left, right) | K::Or(left, right) | K::ScalarEqual(left, right)
            if left.ty == p(P::Bool) && right.ty == p(P::Bool) =>
        {
            let Some(left) = boolean_guard_values(left, subject, candidate, fuel, span, depth + 1)?
            else {
                return Ok(None);
            };
            let Some(right) =
                boolean_guard_values(right, subject, candidate, fuel, span, depth + 1)?
            else {
                return Ok(None);
            };
            let mut outcomes = 0;
            for a in [false, true] {
                for b in [false, true] {
                    if left & (1 << u8::from(a)) == 0 || right & (1 << u8::from(b)) == 0 {
                        continue;
                    }
                    let result = match expr.kind {
                        K::And(..) => a && b,
                        K::Or(..) => a || b,
                        _ => a == b,
                    };
                    outcomes |= 1 << u8::from(result);
                }
            }
            outcomes
        }
        _ => {
            if readonly_guard_comparison(expr, fuel, span)? {
                3
            } else {
                return Ok(None);
            }
        }
    }))
}
