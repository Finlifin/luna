//! Opaque head inputs: a stable value or a declaration-side symbolic input.
use super::*;
pub(super) mod known_patterns;
mod known_scalars;

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

#[derive(Clone)]
enum RoutedNormalInput<'a, 'h> {
    Source(&'a Expr<'h>),
    Prepared(n::Expr),
    Collected(Vec<n::NormalFormArgument>),
}

impl Checker<'_, '_> {
    pub(super) fn bound_tuple_head(&mut self, span: Span) -> Result<Option<DeclarationId>, Error> {
        let Some(head) = self.protocols.tuple else {
            return Ok(None);
        };
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                (matches_declaration(self.package, &self.package_identities, owner, head))
                    .then_some(owner)
            })
            .ok_or_else(|| error(span, "bound Tuple head is not in this source package"))?;
        self.ensure_signature(owner, span)?;
        let parameters = self
            .normal_parameters
            .get(&owner)
            .ok_or_else(|| error(span, "bound Tuple declaration is not a normal-form head"))?;
        if parameters.len() != 1
            || !self.normal_type_list_tail(owner)
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
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                (matches_declaration(self.package, &self.package_identities, owner, head))
                    .then_some(owner)
            })
            .unwrap();
        let arguments = vec![Term::TypeListValue(elements)];
        self.check_type_application(owner, &arguments, span)?;
        Ok(Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        })
    }

    fn known_collected_type_list(&mut self, items: &[n::NormalFormArgument]) -> Option<Term> {
        self.known_collected_type_list_in(items, &BTreeMap::new())
    }

    fn known_collected_type_list_in(
        &mut self,
        items: &[n::NormalFormArgument],
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Option<Term> {
        let mut budget = comptime_runtime::Budget {
            steps: self.required_steps as usize,
            max_limbs: 4096,
        };
        let before = budget.steps;
        let mut elements = Vec::new();
        let mut known = true;
        for item in items {
            match item {
                n::NormalFormArgument::Value(value) => {
                    match self.known_normal_input_in(value, 0, &mut budget, bindings) {
                        Some(ty) => elements.push(ty),
                        None => known = false,
                    }
                }
                n::NormalFormArgument::TypeListSpread(value) => {
                    match self.known_normal_input_in(value, 0, &mut budget, bindings) {
                        Some(Term::TypeListValue(types)) => elements.extend(types),
                        _ => known = false,
                    }
                }
                _ => known = false,
            }
        }
        self.required_steps -= (before - budget.steps) as u64;
        known.then_some(Term::TypeListValue(elements))
    }

    pub(super) fn is_type_list_type(&self, ty: &Term) -> bool {
        nir::normal_forms::type_list_input_type(ty)
            && match ty {
                Term::NormalForm { head, .. } => Some(*head) == self.protocols.meta_list,
                Term::Nominal { declaration, .. } => Some(*declaration) == self.protocols.meta_list,
                _ => false,
            }
    }
    pub(super) fn normal_type_list_tail(&self, owner: OwnerId) -> bool {
        self.type_clause_params(owner)
            .last()
            .is_some_and(|parameter| {
                matches!(parameter.kind, hir::clause::ClauseParamKind::Varadic(..))
            })
    }
    pub(super) fn normal_type_inputs_need_plan(&self, owner: OwnerId, inputs: &[Arg<'_>]) -> bool {
        if !self.normal_type_list_tail(owner) {
            return false;
        }
        if inputs.iter().any(|input| matches!(input, Arg::Named(..)))
            && inputs.iter().any(|input| {
                matches!(input, Arg::Expand(value)
                if !matches!(value.kind, H::Tuple(_) | H::Unit))
            })
        {
            return true;
        }
        let mut remaining = self.normal_fixed_prefix_count(owner);
        for input in inputs {
            match input {
                Arg::Positional(_) => remaining = remaining.saturating_sub(1),
                Arg::Expand(_) if remaining > 0 => return true,
                _ => {}
            }
        }
        false
    }

    fn normal_fixed_prefix_count(&self, owner: OwnerId) -> usize {
        self.type_clause_params(owner)
            .iter()
            .filter(|clause| {
                !matches!(
                    clause.kind,
                    hir::clause::ClauseParamKind::Optional(..)
                        | hir::clause::ClauseParamKind::Varadic(..)
                )
            })
            .count()
    }

    pub(super) fn normal_prefix_literal_spreads<'h>(
        &self,
        owner: OwnerId,
        inputs: &[Arg<'h>],
    ) -> Option<Vec<Arg<'h>>> {
        let whole_collector = self.type_clause_params(owner).last().is_some_and(|clause| {
            inputs
                .iter()
                .any(|input| matches!(input, Arg::Named(name, _) if name.name == clause.name.name))
        });
        let mut remaining = self.normal_fixed_prefix_count(owner);
        let mut expanded = Vec::new();
        let mut changed = false;
        for input in inputs {
            match input {
                Arg::Expand(value) if remaining > 0 => match value.kind {
                    H::Tuple(fields) if fields.len() <= remaining || whole_collector => {
                        remaining = remaining.saturating_sub(fields.len());
                        expanded.extend(fields.iter().map(Arg::Positional));
                        changed = true;
                    }
                    H::Unit => {
                        changed = true;
                    }
                    _ => {
                        // A computed packet can determine the remaining prefix
                        // later. Do not flatten subsequent tail list inputs.
                        remaining = 0;
                        expanded.push(input.clone());
                    }
                },
                Arg::Positional(_) => {
                    remaining = remaining.saturating_sub(1);
                    expanded.push(input.clone());
                }
                _ => expanded.push(input.clone()),
            }
        }
        changed.then_some(expanded)
    }

    pub(super) fn normal_type_list_route<'hir>(
        &self,
        owner: OwnerId,
        inputs: &[Arg<'hir>],
        span: Span,
    ) -> Result<(Vec<Arg<'hir>>, Vec<usize>), Error> {
        let clauses = self.type_clause_params(owner);
        let count = clauses.len();
        // A named whole collector leaves positional inputs exclusively for
        // the fixed prefix. Expand literal packets before assigning dependent
        // contexts, without confusing a trailing List<Type> spread with a tuple.
        if self.normal_type_list_tail(owner)
            && inputs.iter().any(|input| matches!(input, Arg::Named(..)))
        {
            if let Some(expanded) = literal_spreads(inputs) {
                return self.normal_type_list_route(owner, &expanded, span);
            }
        }
        if self.normal_type_list_tail(owner)
            && inputs.len() == count
            && inputs[..count - 1]
                .iter()
                .all(|input| matches!(input, Arg::Positional(_)))
        {
            if let Arg::Expand(value) = &inputs[count - 1] {
                let mut routed = inputs.to_vec();
                routed[count - 1] = Arg::Named(clauses.last().unwrap().name, value);
                return self.normal_type_list_route(owner, &routed, span);
            }
        }
        if !self.normal_type_list_tail(owner) || !inputs.iter().any(|a| matches!(a, Arg::Named(..)))
        {
            return Ok((inputs.to_vec(), (0..count).collect()));
        }
        let name = clauses.last().unwrap().name.name;
        let mut positional = Vec::new();
        let mut whole = None;
        let mut order = Vec::new();
        for input in inputs {
            match input {
                Arg::Named(label, _) => {
                    if label.name != name {
                        return Err(error(span, "unknown named normal-form argument"));
                    }
                    if whole.replace(input.clone()).is_some() {
                        return Err(error(span, "duplicate type-list argument"));
                    }
                    order.push(count - 1);
                }
                Arg::Positional(_) => {
                    order.push(positional.len());
                    positional.push(input.clone());
                }
                _ => {
                    return Err(error(
                        span,
                        "normal-form input expansion is not yet implemented",
                    ));
                }
            }
        }
        if positional.len() > count - 1 {
            return Err(error(
                span,
                "cannot combine a complete type-list argument with collected inputs",
            ));
        }
        if positional.len() < count - 1 {
            return Err(error(span, "normal-form argument count mismatch"));
        }
        positional.push(whole.unwrap());
        Ok((positional, order))
    }

    pub(super) fn normal_type_list_supply<'hir>(
        &self,
        owner: OwnerId,
        inputs: &[Arg<'hir>],
        span: Span,
    ) -> Result<Option<&'hir Expr<'hir>>, Error> {
        let clauses = self.type_clause_params(owner);
        let name = clauses
            .last()
            .ok_or_else(|| error(span, "missing type-list collector"))?
            .name
            .name;
        let mut supplied = None;
        for input in inputs {
            if let Arg::Named(label, value) = input {
                if label.name != name {
                    return Err(error(span, "unknown named normal-form argument"));
                }
                if supplied.replace(*value).is_some() {
                    return Err(error(span, "duplicate type-list argument"));
                }
            }
        }
        if supplied.is_some() && inputs.len() != 1 {
            return Err(error(
                span,
                "cannot combine a complete type-list argument with collected inputs",
            ));
        }
        Ok(supplied)
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

    // Normalization can retain scoped Type descriptions. This is distinct from
    // completed immutable export, which must continue rejecting open values.
    pub(super) fn required_normal_term(
        &mut self,
        value: comptime_interpretor::Value,
        span: Span,
    ) -> Result<Term, Error> {
        self.optional_normal_term(value, span)?
            .ok_or_else(|| error(span, "normal-form input freezing failed: Unsupported"))
    }

    // Delivery-only values need no symbolic identity. Resource, scope and
    // representation failures still propagate; only absent representation is optional.
    pub(super) fn optional_normal_term(
        &mut self,
        value: comptime_interpretor::Value,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        use comptime_interpretor::Value;
        let term = match value {
            Value::Type(ty) if ty.sort() == Sort::Type => ty,
            Value::Type(_) => {
                return Err(error(span, "normal-form Type input contains a non-Type"));
            }
            Value::TypeList(types) => Term::TypeListValue(types),
            value => {
                let parameters = self.visible_comptime_parameters(span)?;
                let mut value_parameters = BTreeMap::new();
                for id in &parameters {
                    if let Some(domain) = self
                        .trait_input_domain(*id)
                        .filter(|domain| *domain != p(P::Type))
                    {
                        value_parameters.insert(*id, self.substituted_trait_domain(&domain, span)?);
                    }
                }
                let parameters = parameters
                    .into_iter()
                    .filter(|id| !value_parameters.contains_key(id))
                    .collect();
                // Domain substitution consumes preparation fuel before the
                // exporter receives its remaining allowance.
                let steps = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut budget = comptime_runtime::Budget {
                    steps,
                    max_limbs: 4096,
                };
                let term = value.into_abstract_invocation_term(
                    &mut budget,
                    &parameters,
                    &value_parameters,
                );
                self.required_steps -= (steps - budget.steps) as u64;
                return term.map(Some).or_else(|failure| {
                    if matches!(failure, comptime_interpretor::ErrorKind::Unsupported) {
                        return Ok(None);
                    }
                    Err(error(
                        span,
                        format!("normal-form input freezing failed: {failure:?}"),
                    ))
                });
            }
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let bounded = nir::type_computations::charge_term(&term, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        bounded.map_err(|e| error(span, e.to_string()))?;
        if let Term::TypeListValue(types) = &term {
            if types.iter().any(|ty| ty.sort() != Sort::Type) {
                return Err(error(
                    span,
                    "normal-form Type-list input contains a non-Type",
                ));
            }
        }
        term.check_scope(&self.call_input_frames())
            .map_err(|e| error(span, e.to_string()))?;
        Ok(Some(term))
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
        if let H::ResolvedLocal(_, id) = value.kind {
            let binding = local(id);
            if self.callable_type_inputs.contains_key(&binding) {
                let Some((symbol, actual)) = self.callable_value_inputs.get(&binding) else {
                    return Err(error(
                        value.span,
                        "normal-form argument requires an explicit comptime value input in this function type",
                    ));
                };
                self.require(actual, ty, value.span)?;
                return Ok(symbol.clone());
            }
        }
        if let H::ResolvedFnParam(_, id) = value.kind {
            let binding = local(id);
            let in_signature = self.signature_owner.is_some_and(|owner| {
                owner == id.owner
                    && (self.active_signatures.contains(&owner) || self.checking_contract)
            });
            if let Some((symbol, actual)) = self
                .value_input_bindings
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
        if let H::ResolvedClauseParam(_, id) = value.kind {
            let declaration = DeclarationId {
                package: semantic_package(&self.package_identities, id.pkg),
                local: id.index,
            };
            if self.forbidden_default_inputs.contains(&declaration) {
                return Err(error(
                    value.span,
                    "trait default references itself or a later parameter",
                ));
            }
            if let Some(actual) = self
                .trait_input_domain(declaration)
                .filter(|ty| *ty != p(P::Type))
            {
                let actual = self.substituted_trait_domain(&actual, value.span)?;
                self.require(&actual, ty, value.span)?;
                return Ok(self
                    .alias_substitution
                    .get(&declaration)
                    .map(|term| (**term).clone())
                    .unwrap_or(Term::Rigid {
                        declaration,
                        sort: Sort::ComptimeValue,
                    }));
            }
            let actual = self
                .normal_parameters
                .values()
                .chain(self.normal_input_prefixes.values())
                .flatten()
                .find(|(d, _, _)| *d == declaration)
                .map(|(_, _, t)| t)
                .ok_or_else(|| Error {
                    needs_capability: self
                        .signature_owner
                        .is_some_and(|owner| self.active_signatures.contains(&owner)),
                    ..error(
                        value.span,
                        "normal-form value argument is not an available declaration input",
                    )
                })?;
            self.require(actual, ty, value.span)
                .map_err(|mut failure| {
                    failure.message = format!(
                        "normal-form value argument type mismatch: {}",
                        failure.message
                    );
                    failure
                })?;
            return Ok(Term::Rigid {
                declaration,
                sort: Sort::ComptimeValue,
            });
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

    pub(super) fn normal_form_metadata(&self) -> Vec<nir::normal_forms::Head> {
        self.normal_parameters
            .iter()
            .map(|(owner, parameters)| nir::normal_forms::Head {
                role: if Some(declaration(self.package, &self.package_identities, *owner))
                    == self.protocols.tuple
                {
                    nir::normal_forms::Role::Tuple
                } else if Some(declaration(self.package, &self.package_identities, *owner))
                    == self.protocols.meta_list
                {
                    nir::normal_forms::Role::TypeList
                } else {
                    nir::normal_forms::Role::Opaque
                },
                declaration: declaration(self.package, &self.package_identities, *owner),
                requirements: self
                    .requirements
                    .get(owner)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .chain(
                        self.capabilities
                            .get(owner)
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
                        kind: if self.is_type_list_type(ty) {
                            nir::normal_forms::Kind::TypeList
                        } else if *ty == p(P::Type) {
                            nir::normal_forms::Kind::Type
                        } else {
                            nir::normal_forms::Kind::Value
                        },
                    })
                    .collect(),
            })
            .collect()
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
        if self.normal_type_inputs_need_plan(owner, inputs) {
            if let Some(expanded) = self.normal_prefix_literal_spreads(owner, inputs) {
                return self.normal_type_expression(expr, owner, &expanded, expected);
            }
            return self.defaulted_normal_expression(owner, inputs, expr.span, expected);
        }
        if !self.normal_type_list_tail(owner) {
            if let Some(inputs) = literal_spreads(inputs) {
                return self.normal_type_expression(expr, owner, &inputs, expected);
            }
        }
        if self.has_optional_type_clauses(owner)
            || (!self.normal_type_list_tail(owner)
                && inputs.iter().any(|arg| matches!(arg, Arg::Expand(_))))
        {
            if self.normal_parameters[&owner]
                .iter()
                .any(|(_, _, ty)| *ty != p(P::Type))
            {
                return self.defaulted_normal_expression(owner, inputs, expr.span, expected);
            }
            return self.defaulted_type_expression(owner, inputs, expr.span, expected);
        }
        let parameters = self.normal_parameters[&owner].clone();
        let variadic = self.normal_type_list_tail(owner);
        let (routed, order) = self.normal_type_list_route(owner, inputs, expr.span)?;
        let inputs = routed.as_slice();
        if if variadic {
            inputs.len() < parameters.len() - 1
        } else {
            parameters.len() != inputs.len()
        } {
            return Err(error(expr.span, "normal-form argument count mismatch"));
        }
        let mut arguments = vec![None; parameters.len()];
        let mut known_parameters = Vec::new();
        let mut known_types = Vec::new();
        let mut pending_parameters = BTreeSet::new();
        let mut formation_arguments = vec![None; parameters.len()];
        let mut pending_inputs = BTreeSet::new();
        let count = parameters.len();
        for &slot in &order {
            let (parameter, _, declared) = parameters[slot].clone();
            if variadic && slot + 1 == count {
                if let Some(input) =
                    self.normal_type_list_supply(owner, &inputs[slot..], expr.span)?
                {
                    let value = self.expr(input, Some(&declared))?;
                    let mut budget = comptime_runtime::Budget {
                        steps: self.required_steps as usize,
                        max_limbs: 4096,
                    };
                    let before = budget.steps;
                    let known = self.known_normal_input(&value, 0, &mut budget);
                    self.required_steps -= (before - budget.steps) as u64;
                    formation_arguments[slot] = Some(known.unwrap_or_else(|| {
                        pending_inputs.insert(parameter);
                        Term::Rigid {
                            declaration: parameter,
                            sort: Sort::ComptimeValue,
                        }
                    }));
                    arguments[slot] = Some(n::NormalFormArgument::Value(value));
                    continue;
                }
                let mut items = Vec::new();
                for input in &inputs[slot..] {
                    let (input, spread) = match input {
                        Arg::Positional(input) => (*input, false),
                        Arg::Expand(input) => (*input, true),
                        _ => {
                            return Err(error(expr.span, "unsupported type-list collection input"));
                        }
                    };
                    let element_type = if spread { declared.clone() } else { p(P::Type) };
                    let value = self.expr(input, Some(&element_type))?;
                    items.push(if spread {
                        n::NormalFormArgument::TypeListSpread(value)
                    } else {
                        n::NormalFormArgument::Value(value)
                    });
                }
                formation_arguments[slot] =
                    Some(self.known_collected_type_list(&items).unwrap_or_else(|| {
                        pending_inputs.insert(parameter);
                        Term::Rigid {
                            declaration: parameter,
                            sort: Sort::ComptimeValue,
                        }
                    }));
                arguments[slot] = Some(n::NormalFormArgument::TypeList {
                    source: source(expr.span),
                    items,
                });
                continue;
            }
            let input = &inputs[slot];
            let Arg::Positional(input) = input else {
                return Err(error(expr.span, "normal-form arguments must be positional"));
            };
            let dependent = pending_parameters
                .iter()
                .any(|parameter| declared.mentions_rigid(*parameter));
            if dependent {
                let argument = self.dependent_normal_argument(input, 0)?;
                arguments[slot] = Some(argument);
                pending_inputs.insert(parameter);
                formation_arguments[slot] = Some(Term::Rigid {
                    declaration: parameter,
                    sort: Sort::ComptimeValue,
                });
                continue;
            }
            let ty = self.instantiate_input_types(
                &declared,
                &known_parameters,
                &known_types,
                input.span,
            )?;
            let value = self.expr(input, Some(&ty)).map_err(|mut failure| {
                if declared != p(P::Type) && failure.message.contains("type mismatch") {
                    failure.message = format!(
                        "normal-form value argument type mismatch: {}",
                        failure.message
                    );
                }
                failure
            })?;
            let mut budget = comptime_runtime::Budget::default();
            let known = self.known_normal_input(&value, 0, &mut budget);
            let known = known.filter(|value| declared != p(P::Type) || value.sort() == Sort::Type);
            if let Some(value) = &known {
                known_parameters.push(parameter);
                known_types.push(value.clone());
            } else {
                pending_parameters.insert(parameter);
            }
            formation_arguments[slot] = Some(known.unwrap_or_else(|| {
                pending_inputs.insert(parameter);
                Term::Rigid {
                    declaration: parameter,
                    sort: if declared == p(P::Type) {
                        Sort::Type
                    } else {
                        Sort::ComptimeValue
                    },
                }
            }));
            arguments[slot] = Some(n::NormalFormArgument::Value(value));
        }
        let arguments = arguments
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| error(expr.span, "incomplete normal-form input routing"))?;
        let formation_arguments = formation_arguments
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| error(expr.span, "incomplete normal-form formation inputs"))?;
        let plan = if order.iter().copied().eq(0..count) {
            None
        } else {
            Some(n::NormalFormInputPlan {
                preparations: vec![],
                order,
                bindings: (0..count)
                    .map(|_| self.spread_binding(expr.span))
                    .collect::<Result<_, _>>()?,
            })
        };
        self.check_formation_application(
            owner,
            FormationInputs {
                terms: formation_arguments,
                deferred: pending_inputs,
                environment: BTreeMap::new(),
            },
            expr.span,
        )?;
        let ty = if arguments.iter().any(n::NormalFormArgument::is_diverging) {
            p(P::Never)
        } else {
            p(P::Type)
        };
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::NormalFormType {
                plan,
                head: declaration(self.package, &self.package_identities, owner),
                arguments,
            },
        })
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
        if depth >= 128 || budget.consume(1).is_err() {
            return None;
        }
        match &expr.kind {
            K::TypeValue(ty) | K::ComptimeValue(ty) => clone_known_term(ty, budget),
            K::TypeListValue(types) if self.is_type_list_type(&expr.ty) => {
                let mut values = Vec::new();
                for ty in types {
                    if ty.sort() != Sort::Type {
                        return None;
                    }
                    values.push(clone_known_term(ty, budget)?);
                }
                Some(Term::TypeListValue(values))
            }
            K::TypeListIndex(values, index) => {
                if !self.is_type_list_type(&values.ty) {
                    return None;
                }
                let Term::TypeListValue(mut values) =
                    self.known_normal_input_in(values, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                let Term::Integer(index) =
                    self.known_normal_input_in(index, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                budget
                    .consume(index.bit_len().div_ceil(32).saturating_add(1))
                    .ok()?;
                let index = usize::try_from(index.to_u128().ok()?).ok()?;
                if index >= values.len() || values[index].sort() != Sort::Type {
                    return None;
                }
                Some(values.swap_remove(index))
            }
            K::NormalFormType {
                head,
                arguments,
                plan,
            } => {
                let arguments = if let Some(plan) = plan {
                    // This inspects immutable construction only. Calls,
                    // assignments and contextual conversions still require
                    // invocation through the original plan. Known branches
                    // below supply context without removing that plan.
                    budget
                        .consume(arguments.len().saturating_add(bindings.len()))
                        .ok()?;
                    if !plan.preparations.is_empty()
                        || !plan.valid(arguments.len())
                        || plan.bindings.iter().any(|id| bindings.contains_key(id))
                    {
                        return None;
                    }
                    let mut scoped = bindings.clone();
                    let mut values = vec![None; arguments.len()];
                    for &slot in &plan.order {
                        let value = collect_known_normal_argument(
                            &arguments[slot],
                            budget,
                            |value, budget| {
                                self.known_normal_input_in(value, depth + 1, budget, &scoped)
                            },
                        )?;
                        let bound = clone_known_term(&value, budget)?;
                        scoped.insert(plan.bindings[slot], std::sync::Arc::new(bound));
                        values[slot] = Some(value);
                    }
                    values.into_iter().collect::<Option<Vec<_>>>()?
                } else {
                    arguments
                        .iter()
                        .map(|argument| {
                            collect_known_normal_argument(argument, budget, |value, budget| {
                                self.known_normal_input_in(value, depth + 1, budget, bindings)
                            })
                        })
                        .collect::<Option<Vec<_>>>()?
                };
                Some(Term::NormalForm {
                    head: *head,
                    sort: Sort::Type,
                    arguments,
                })
            }
            K::PointerType(value) => {
                let inner = self.known_normal_input_in(value, depth + 1, budget, bindings)?;
                (inner.sort() == Sort::Type).then(|| Term::Pointer(Box::new(inner)))
            }
            K::BitVecType(value) => {
                let Term::Integer(width) =
                    self.known_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                budget
                    .consume(width.bit_len().div_ceil(32).saturating_add(1))
                    .ok()?;
                let width = usize::try_from(width.to_u128().ok()?).ok()?;
                Some(p(P::BitVec(width)))
            }
            K::FunctionType {
                comptime,
                comptime_parameters,
                parameters,
            } => {
                budget.consume(parameters.len().saturating_add(1)).ok()?;
                if comptime_parameters.len() != parameters.len() {
                    return None;
                }
                let mut types = Vec::with_capacity(parameters.len());
                for (parameter, staged) in parameters.iter().zip(comptime_parameters) {
                    let ty = self.known_normal_input_in(parameter, depth + 1, budget, bindings)?;
                    if ty.sort() != Sort::Type {
                        return None;
                    }
                    types.push(Parameter {
                        sort: if *staged {
                            if ty == p(P::Type) {
                                Sort::Type
                            } else {
                                Sort::ComptimeValue
                            }
                        } else {
                            Sort::Runtime
                        },
                        ty,
                    });
                }
                Some(
                    Term::Function {
                        parameters: types,
                        result: Box::new(p(P::Void)),
                    }
                    .with_comptime_phase(*comptime),
                )
            }
            K::FunctionResultType { signature, result } => {
                let signature =
                    self.known_normal_input_in(signature, depth + 1, budget, bindings)?;
                if !matches!(signature.callable_signature(), Term::Function { .. }) {
                    return None;
                }
                let result = self.known_normal_input_in(result, depth + 1, budget, bindings)?;
                if result.sort() != Sort::Type {
                    return None;
                }
                signature.with_callable_result(result)
            }
            K::ClosureType {
                environment,
                signature,
            } => {
                budget.consume(environment.len().saturating_add(1)).ok()?;
                let mut types = Vec::with_capacity(environment.len());
                for field in environment {
                    let ty = self.known_normal_input_in(field, depth + 1, budget, bindings)?;
                    if ty.sort() != Sort::Type {
                        return None;
                    }
                    types.push(ty);
                }
                let signature =
                    self.known_normal_input_in(signature, depth + 1, budget, bindings)?;
                if signature.sort() != Sort::Type {
                    return None;
                }
                if types.is_empty() {
                    Some(signature)
                } else {
                    if !matches!(
                        signature,
                        Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
                    ) {
                        return None;
                    }
                    Some(Term::Closure {
                        environment: types,
                        signature: Box::new(signature),
                    })
                }
            }
            K::TypeList(values) if self.is_type_list_type(&expr.ty) => {
                let values = values
                    .iter()
                    .map(|value| self.known_normal_input_in(value, depth + 1, budget, bindings))
                    .collect::<Option<Vec<_>>>()?;
                values
                    .iter()
                    .all(|value| value.sort() == Sort::Type)
                    .then_some(Term::TypeListValue(values))
            }
            K::TupleType(values) => Some(Term::Tuple(
                values
                    .iter()
                    .map(|value| self.known_normal_input_in(value, depth + 1, budget, bindings))
                    .collect::<Option<Vec<_>>>()?,
            )),
            K::AnonymousRecord { fields } => {
                let Term::Record(types) = &expr.ty else {
                    return None;
                };
                if fields.len() != types.len() {
                    return None;
                }
                Some(Term::RecordValue(
                    fields
                        .iter()
                        .zip(types)
                        .map(|(field, (name, ty))| {
                            budget.consume(name.len()).ok()?;
                            if field.ty != *ty {
                                return None;
                            }
                            Some((
                                name.clone(),
                                self.known_normal_input_in(field, depth + 1, budget, bindings)?,
                            ))
                        })
                        .collect::<Option<Vec<_>>>()?,
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
                    return None;
                };
                if declaration != actual {
                    return None;
                }
                if fields.len() > budget.steps {
                    return None;
                }
                let variant = match &expr.kind {
                    K::Variant { index, .. } => Some(u32::try_from(*index).ok()?),
                    _ => None,
                };
                Some(Term::NominalValue {
                    ty: Box::new(clone_known_term(&expr.ty, budget)?),
                    variant,
                    fields: fields
                        .iter()
                        .map(|field| self.known_normal_input_in(field, depth + 1, budget, bindings))
                        .collect::<Option<Vec<_>>>()?,
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
                } = self.known_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                let Term::Nominal {
                    declaration: actual,
                    ..
                } = ty.as_ref()
                else {
                    return None;
                };
                if actual != declaration || *index >= fields.len() {
                    return None;
                }
                Some(fields.swap_remove(*index))
            }
            K::AnonymousField { value, name, index } => {
                let Term::RecordValue(mut fields) =
                    self.known_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                let (actual, _) = fields.get(*index)?;
                budget
                    .consume(actual.len().saturating_add(name.len()))
                    .ok()?;
                if actual != name {
                    return None;
                }
                Some(fields.swap_remove(*index).1)
            }
            K::Tuple(values) => Some(Term::TupleValue(
                values
                    .iter()
                    .map(|value| self.known_normal_input_in(value, depth + 1, budget, bindings))
                    .collect::<Option<Vec<_>>>()?,
            )),
            K::Unit => Some(Term::Unit),
            K::Literal(n::Literal::Bool(value)) => Some(Term::Bool(*value)),
            K::Literal(n::Literal::Char(value)) => Some(Term::Char(*value)),
            K::Literal(n::Literal::Float(spelling)) => Some(Term::Float(
                comptime_runtime::decimal::DecimalValue::parse_literal(spelling, budget).ok()?,
            )),
            K::Literal(n::Literal::Float32(bits)) => Some(Term::Float32(*bits)),
            K::Literal(n::Literal::Float64(bits)) => Some(Term::Float64(*bits)),
            K::Literal(n::Literal::BitVec { width, words }) => Some(Term::BitVec(
                comptime_runtime::bitvec::BitVec::from_words(*width, words, budget).ok()?,
            )),
            K::SymbolName(value) => {
                if expr.ty != p(P::Str) || value.ty != p(P::Symbol) {
                    return None;
                }
                let Term::Symbol(name) =
                    self.known_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                Some(Term::String(name))
            }
            K::Literal(n::Literal::Symbol(value)) => {
                budget.consume(value.len()).ok()?;
                Some(Term::Symbol(value.as_str().into()))
            }
            K::Literal(n::Literal::String(value)) => {
                budget.consume(value.len()).ok()?;
                Some(Term::String(value.as_str().into()))
            }
            K::Literal(n::Literal::SignedInteger(value)) => {
                Some(Term::Integer(comptime_runtime::Integer::from_i128(*value)))
            }
            K::Literal(n::Literal::UnsignedInteger(value)) => {
                Some(Term::Integer(comptime_runtime::Integer::from_u128(*value)))
            }
            K::Literal(n::Literal::Integer(value)) => {
                let (digits, negative) = value
                    .strip_prefix('-')
                    .map_or((value.as_str(), false), |digits| (digits, true));
                let value = comptime_runtime::Integer::parse_literal(digits, budget).ok()?;
                Some(Term::Integer(if negative {
                    value.negate(budget).ok()?
                } else {
                    value
                }))
            }
            K::Not(value) => {
                match self.known_normal_input_in(value, depth + 1, budget, bindings)? {
                    Term::Bool(value) => Some(Term::Bool(!value)),
                    Term::BitVec(value) => Some(Term::BitVec(value.not(budget).ok()?)),
                    _ => None,
                }
            }
            K::And(left, right) | K::Or(left, right) => {
                let left = self.known_normal_input_in(left, depth + 1, budget, bindings)?;
                let conjunction = matches!(expr.kind, K::And(..));
                // Boolean operators short-circuit; bit-vector operators do not.
                // In particular an unreachable call must not erase a known
                // boolean context or consume its invocation budget here.
                if let Term::Bool(value) = &left {
                    if *value != conjunction {
                        return Some(Term::Bool(*value));
                    }
                }
                let right = self.known_normal_input_in(right, depth + 1, budget, bindings)?;
                match (left, right) {
                    (Term::Bool(a), Term::Bool(b)) => {
                        Some(Term::Bool(if conjunction { a && b } else { a || b }))
                    }
                    (Term::BitVec(a), Term::BitVec(b)) => Some(Term::BitVec(
                        if conjunction {
                            a.and(&b, budget)
                        } else {
                            a.or(&b, budget)
                        }
                        .ok()?,
                    )),
                    _ => None,
                }
            }
            K::BitConcat(left, right) | K::BitEqual(left, right) => {
                let Term::BitVec(left) =
                    self.known_normal_input_in(left, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                let Term::BitVec(right) =
                    self.known_normal_input_in(right, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                if matches!(expr.kind, K::BitConcat(..)) {
                    Some(Term::BitVec(left.concat(&right, budget).ok()?))
                } else {
                    if left.width() != right.width() {
                        return None;
                    }
                    budget.consume(left.words().len().saturating_add(1)).ok()?;
                    Some(Term::Bool(left == right))
                }
            }
            K::BitSlice { value, start, end } => {
                let Term::BitVec(value) =
                    self.known_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                Some(Term::BitVec(value.slice(*start, *end, budget).ok()?))
            }
            K::TakeBits(value) => {
                let Term::Primitive(primitive) = value.ty else {
                    return None;
                };
                let known = self.known_normal_input_in(value, depth + 1, budget, bindings)?;
                if let Term::BitVec(bits) = known {
                    return Some(Term::BitVec(bits));
                }
                let Term::Integer(integer) = known else {
                    return None;
                };
                let (signed, width) = primitive.fixed_integer_layout()?;
                let raw = if signed {
                    integer.to_signed(width).ok()? as u128
                } else {
                    integer.to_unsigned(width).ok()?
                };
                let raw = if width == 128 {
                    raw
                } else {
                    raw & ((1u128 << width) - 1)
                };
                Some(Term::BitVec(
                    comptime_runtime::Integer::from_u128(raw)
                        .to_bitvec(width as usize, budget)
                        .ok()?,
                ))
            }
            K::BitConvert(value) => {
                let Term::Primitive(P::BitVec(width)) = expr.ty else {
                    return None;
                };
                let Term::BitVec(value) =
                    self.known_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                Some(Term::BitVec(value.widen(width, budget).ok()?))
            }
            K::TypeEqual(left, right) => {
                let left = self.known_normal_input_in(left, depth + 1, budget, bindings)?;
                let right = self.known_normal_input_in(right, depth + 1, budget, bindings)?;
                known_type_equality(&self.context, &left, &right, budget).map(Term::Bool)
            }
            K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right) => {
                let left = self.known_normal_input_in(left, depth + 1, budget, bindings)?;
                let right = self.known_normal_input_in(right, depth + 1, budget, bindings)?;
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
            K::FloatNegate(value) => Some(
                match self.known_normal_input_in(value, depth + 1, budget, bindings)? {
                    Term::Float(value) => Term::Float(value.negate(budget).ok()?),
                    Term::Float32(bits) => Term::Float32(bits ^ (1 << 31)),
                    Term::Float64(bits) => Term::Float64(bits ^ (1 << 63)),
                    _ => return None,
                },
            ),
            K::FloatBinary { op, left, right } => {
                let left = self.known_normal_input_in(left, depth + 1, budget, bindings)?;
                let right = self.known_normal_input_in(right, depth + 1, budget, bindings)?;
                Some(match (left, right) {
                    (Term::Float(left), Term::Float(right)) => Term::Float(match op {
                        n::FloatOp::Add => left.add(&right, budget).ok()?,
                        n::FloatOp::Subtract => left.subtract(&right, budget).ok()?,
                        n::FloatOp::Multiply => left.multiply(&right, budget).ok()?,
                        n::FloatOp::Divide => return None,
                    }),
                    (Term::Float32(left), Term::Float32(right)) => {
                        Term::Float32(op.evaluate_f32(left, right))
                    }
                    (Term::Float64(left), Term::Float64(right)) => {
                        Term::Float64(op.evaluate_f64(left, right))
                    }
                    _ => return None,
                })
            }
            K::IntegerNegate(value) => {
                let Term::Integer(value) =
                    self.known_normal_input_in(value, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                Some(Term::Integer(value.negate(budget).ok()?))
            }
            K::IntegerBinary { op, left, right } => {
                let Term::Integer(left) =
                    self.known_normal_input_in(left, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                let Term::Integer(right) =
                    self.known_normal_input_in(right, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                op.evaluate(&left, &right, budget).ok()
            }
            K::Match { value, arms } => {
                let value = self.known_normal_input_in(value, depth + 1, budget, bindings)?;
                for arm in arms {
                    budget.consume(bindings.len().saturating_add(1)).ok()?;
                    let mut scoped = bindings.clone();
                    if !self.known_normal_pattern(
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
                            self.known_normal_input_in(guard, depth + 1, budget, &scoped)?
                        else {
                            return None;
                        };
                        if !accepted {
                            continue;
                        }
                    }
                    return self.known_normal_input_in(&arm.body, depth + 1, budget, &scoped);
                }
                None
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
                    self.known_normal_input_in(condition, depth + 1, budget, bindings)?
                else {
                    return None;
                };
                if condition {
                    self.known_normal_input_in(then_branch, depth + 1, budget, bindings)
                } else {
                    match else_branch {
                        Some(branch) => {
                            self.known_normal_input_in(branch, depth + 1, budget, bindings)
                        }
                        None => Some(Term::Unit),
                    }
                }
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                // Only immutable local construction is inspected. Calls,
                // assignments and control transfer retain invocation-time work.
                if statements.len() > budget.steps {
                    return None;
                }
                budget.consume(bindings.len()).ok()?;
                let mut scoped = bindings.clone();
                for statement in statements {
                    budget.consume(1).ok()?;
                    match &statement.kind {
                        K::Let { binding, value } => {
                            let value =
                                self.known_normal_input_in(value, depth + 1, budget, &scoped)?;
                            scoped.insert(*binding, std::sync::Arc::new(value));
                        }
                        K::LetPattern {
                            pattern,
                            value,
                            guard,
                            ..
                        } => {
                            let value =
                                self.known_normal_input_in(value, depth + 1, budget, &scoped)?;
                            // Any failure leaves the whole block unknown. In
                            // particular a let-else transfer is not fallthrough.
                            if !self.known_normal_pattern(
                                pattern,
                                &value,
                                depth + 1,
                                budget,
                                &mut scoped,
                            )? {
                                return None;
                            }
                            if let Some(guard) = guard {
                                if self.known_normal_input_in(guard, depth + 1, budget, &scoped)?
                                    != Term::Bool(true)
                                {
                                    return None;
                                }
                            }
                        }
                        _ => return None,
                    }
                }
                match result {
                    Some(result) => self.known_normal_input_in(result, depth + 1, budget, &scoped),
                    None => Some(Term::Unit),
                }
            }
            K::Local(binding) => {
                if let Some(value) = bindings.get(binding) {
                    return clone_known_term(value, budget);
                }
                if let Some((symbol, _)) = self.signature_comptime_input(*binding) {
                    return clone_known_term(&symbol, budget);
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
                        return clone_known_term(value, budget);
                    }
                    if let Some((symbol, _)) = self.value_input_bindings.get(binding) {
                        return clone_known_term(symbol, budget);
                    }
                    if let Some(symbol) = self.type_input_bindings.get(binding) {
                        return clone_known_term(symbol, budget);
                    }
                }
                let parameters = self.normal_parameters.get(&self.signature_owner?)?;
                let (declaration, _, _) =
                    parameters.iter().find(|(_, local, _)| local == binding)?;
                Some(Term::Rigid {
                    declaration: *declaration,
                    sort: Sort::ComptimeValue,
                })
            }
            _ => None,
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
        if matches!(input.kind, H::Unit) {
            return Ok(n::NormalFormArgument::Unit {
                source: source(input.span),
            });
        }
        if let H::Tuple(items) = &input.kind {
            return Ok(n::NormalFormArgument::Tuple {
                source: source(input.span),
                type_head: self.bound_tuple_head(input.span)?,
                items: items
                    .iter()
                    .map(|value| self.dependent_normal_argument(value, depth + 1))
                    .collect::<Result<_, _>>()?,
            });
        }
        if let H::Match(value, arms) = &input.kind {
            let value = self.expr(value, None)?;
            let saved = self.locals.clone();
            let result = (|| {
                let mut checked = Vec::new();
                for arm in *arms {
                    self.locals = saved.clone();
                    let pattern = self.pattern(&arm.pat, &value.ty)?;
                    let guard = arm
                        .guard
                        .map(|guard| self.expr(guard, Some(&p(P::Bool))))
                        .transpose()?;
                    let body = self.dependent_normal_argument(arm.body, depth + 1)?;
                    checked.push(n::NormalMatchArm {
                        pattern,
                        guard,
                        body,
                    });
                }
                let patterns: Vec<_> = checked
                    .iter()
                    .filter(|arm| arm.guard.is_none())
                    .map(|arm| &arm.pattern)
                    .collect();
                let exhaustive = self.patterns_exhaustive(&patterns, &value.ty);
                if exhaustive != crate::coverage::Coverage::Exhaustive {
                    return Err(error(
                        input.span,
                        if exhaustive == crate::coverage::Coverage::Unknown {
                            "match coverage budget exhausted; add an unguarded fallback pattern"
                        } else {
                            "non-exhaustive match; add an unguarded fallback pattern"
                        },
                    ));
                }
                Ok(n::NormalFormArgument::Match {
                    source: source(input.span),
                    value,
                    arms: checked,
                })
            })();
            self.locals = saved;
            return result;
        }
        if let H::When(arms) = &input.kind {
            let fallback = arms.iter().position(|arm| arm.cond.is_none());
            if fallback.is_some_and(|index| index + 1 != arms.len()) {
                return Err(error(input.span, "when fallback must be the final arm"));
            }
            // A when without fallback has a discarded result, not a value
            // awaiting this input's expected type.
            if fallback.is_none() {
                return self.expr(input, None).map(n::NormalFormArgument::Value);
            }
            let saved = self.locals.clone();
            let result = (|| {
                let mut branches = Vec::new();
                for arm in *arms {
                    self.locals = saved.clone();
                    let condition = arm
                        .cond
                        .map(|value| self.expr(value, Some(&p(P::Bool))))
                        .transpose()?;
                    let body = self.dependent_normal_argument(arm.body, depth + 1)?;
                    // Every arm owns its local declarations even when its
                    // source body was not written as an explicit block.
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
            })();
            self.locals = saved;
            return result;
        }
        if let H::If(condition, yes, Some(no)) = &input.kind {
            return Ok(n::NormalFormArgument::If {
                source: source(input.span),
                condition: self.expr(condition, Some(&p(P::Bool)))?,
                then_input: Box::new(self.dependent_normal_block(yes, depth + 1)?),
                else_input: Box::new(self.dependent_normal_argument(no, depth + 1)?),
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
        let saved = self.locals.clone();
        let result = (|| {
            let statements = block
                .stmts
                .iter()
                .map(|statement| self.expr(statement, None))
                .collect::<Result<Vec<_>, _>>()?;
            let result = block
                .expr
                .map(|value| self.dependent_normal_argument(value, depth + 1))
                .transpose()?
                .map(Box::new);
            Ok(n::NormalFormArgument::Block {
                source: source(block.span),
                statements,
                result,
            })
        })();
        self.locals = saved;
        result
    }
}

// Bound copied semantic structure as well as expression visits. Local values
// are shared while inspecting a block, but returning an owned term must pay for
// expansion (e.g. repeated tuple duplication).
fn collect_known_normal_argument(
    argument: &n::NormalFormArgument,
    budget: &mut comptime_runtime::Budget,
    mut known: impl FnMut(&n::Expr, &mut comptime_runtime::Budget) -> Option<Term>,
) -> Option<Term> {
    budget.consume(1).ok()?;
    let n::NormalFormArgument::TypeList { items, .. } = argument else {
        return known(argument.expression()?, budget);
    };
    let mut collected = Vec::new();
    for item in items {
        budget.consume(1).ok()?;
        match item {
            n::NormalFormArgument::Value(expr) => {
                let value = known(expr, budget)?;
                if value.sort() != Sort::Type {
                    return None;
                }
                collected.push(value);
            }
            n::NormalFormArgument::TypeListSpread(expr) => {
                let Term::TypeListValue(types) = known(expr, budget)? else {
                    return None;
                };
                budget.consume(types.len()).ok()?;
                if types.iter().any(|ty| ty.sort() != Sort::Type) {
                    return None;
                }
                collected.extend(types);
            }
            _ => return None,
        }
    }
    Some(Term::TypeListValue(collected))
}

/// A type relation needs a proof in either direction. A failed equality query
/// is not a disequality fact, and an opaque head is never reduced to its body.
fn known_type_equality(
    context: &crate::ptc::Context,
    left: &Term,
    right: &Term,
    budget: &mut comptime_runtime::Budget,
) -> Option<bool> {
    use crate::ptc::{Formula, Outcome};
    if left.sort() != Sort::Type || right.sort() != Sort::Type {
        return None;
    }
    let equality = Formula::equal(
        clone_known_term(left, budget)?,
        clone_known_term(right, budget)?,
    );
    let inequality = Formula::equal(
        clone_known_term(left, budget)?,
        clone_known_term(right, budget)?,
    )
    .negate();
    let mut proof_budget = crate::ptc::Budget {
        steps: budget.steps,
        max_depth: 128,
    };
    let result = if matches!(
        context.prove_metered(equality, &mut proof_budget),
        Outcome::Entailed(_)
    ) {
        Some(true)
    } else if matches!(
        context.prove_metered(inequality, &mut proof_budget),
        Outcome::Entailed(_)
    ) {
        Some(false)
    } else {
        None
    };
    budget.steps = proof_budget.steps;
    result
}

pub(super) fn clone_known_term(term: &Term, budget: &mut comptime_runtime::Budget) -> Option<Term> {
    let mut pending = vec![(term, 0usize)];
    while let Some((term, depth)) = pending.pop() {
        if depth >= 128 {
            return None;
        }
        budget.consume(1).ok()?;
        match term {
            Term::Computation { effects, result } => {
                budget.consume(effects.len()).ok()?;
                let width = effects
                    .iter()
                    .try_fold(1usize, |sum, op| sum.checked_add(op.arguments.len()))?;
                if width > budget.steps {
                    return None;
                }
                pending.push((result, depth + 1));
                for operation in effects {
                    pending.extend(operation.arguments.iter().map(|item| (item, depth + 1)));
                }
            }
            Term::AssociatedType {
                arguments, subject, ..
            } => {
                if arguments.len().saturating_add(1) > budget.steps {
                    return None;
                }
                pending.push((subject, depth + 1));
                pending.extend(arguments.iter().map(|item| (item, depth + 1)));
            }
            Term::ErrorValue { ty, label, value } => {
                pending.push((ty, depth + 1));
                if let Some(label) = label {
                    pending.push((label, depth + 1));
                }
                pending.push((value, depth + 1));
            }
            Term::NominalValue { ty, fields, .. } => {
                if fields.len().saturating_add(1) > budget.steps {
                    return None;
                }
                pending.push((ty, depth + 1));
                pending.extend(fields.iter().map(|field| (field, depth + 1)));
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                if fields.len() > budget.steps {
                    return None;
                }
                for (name, ty) in fields {
                    budget.consume(name.len()).ok()?;
                    pending.push((ty, depth + 1));
                }
            }
            Term::Tuple(items)
            | Term::TypeListValue(items)
            | Term::TypeListConcat(items)
            | Term::TupleValue(items)
            | Term::Nominal {
                arguments: items, ..
            }
            | Term::NormalForm {
                arguments: items, ..
            } => {
                if items.len() > budget.steps {
                    return None;
                }
                pending.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Term::Pointer(inner) | Term::ComptimeFunction(inner) | Term::UnsafeFunction(inner) => {
                pending.push((inner, depth + 1))
            }
            Term::Function { parameters, result } => {
                if parameters.len() > budget.steps {
                    return None;
                }
                pending.push((result, depth + 1));
                pending.extend(parameters.iter().map(|p| (&p.ty, depth + 1)));
            }
            Term::Closure {
                environment,
                signature,
            } => {
                if environment.len() > budget.steps {
                    return None;
                }
                pending.push((signature, depth + 1));
                pending.extend(environment.iter().map(|item| (item, depth + 1)));
            }
            Term::Integer(value) => {
                budget.consume(value.bit_len().div_ceil(32)).ok()?;
            }
            Term::Float(value) => {
                budget
                    .consume(
                        value
                            .digits()
                            .len()
                            .saturating_add(value.exponent().bit_len().div_ceil(32)),
                    )
                    .ok()?;
            }
            Term::BitVec(value) => {
                budget.consume(value.words().len()).ok()?;
            }
            Term::String(value) | Term::Symbol(value) => {
                budget.consume(value.len()).ok()?;
            }
            _ => {}
        }
    }
    Some(term.clone())
}

#[cfg(test)]
mod tests {
    use super::clone_known_term;
    use super::known_type_equality;
    use comptime_runtime::Budget;
    use nir::Term;
    use nir::{Primitive, Sort};

    #[test]
    fn known_type_list_collection_preserves_order_and_stops_at_unknowns() {
        use super::collect_known_normal_argument;
        use nir::program::{Expr, ExprKind as K, NormalFormArgument as A, SourceRange};
        use nir::{Primitive, Sort};
        let source = SourceRange { start: 0, end: 0 };
        let expr = |kind| Expr {
            source,
            ty: Term::Primitive(Primitive::Type),
            kind,
        };
        let input = |items| A::TypeList { source, items };
        let flag = Term::Primitive(Primitive::Bool);
        let byte = Term::Primitive(Primitive::U8);
        let resolve = |e: &Expr, budget: &mut Budget| match &e.kind {
            K::TypeValue(ty) | K::ComptimeValue(ty) => super::clone_known_term(ty, budget),
            K::TypeListValue(types) => Some(Term::TypeListValue(types.clone())),
            _ => None,
        };
        let valid = input(vec![
            A::Value(expr(K::TypeValue(flag.clone()))),
            A::TypeListSpread(expr(K::TypeListValue(vec![byte.clone(), flag.clone()]))),
            A::TypeListSpread(expr(K::TypeListValue(vec![]))),
        ]);
        assert_eq!(
            collect_known_normal_argument(&valid, &mut Budget::default(), resolve),
            Some(Term::TypeListValue(vec![flag.clone(), byte, flag.clone()]))
        );
        assert!(
            collect_known_normal_argument(
                &valid,
                &mut Budget {
                    steps: 1,
                    max_limbs: 4096
                },
                resolve
            )
            .is_none()
        );
        assert!(collect_known_normal_argument(&valid, &mut Budget::default(), resolve).is_some());
        for invalid in [
            A::Value(expr(K::TypeListValue(vec![]))),
            A::TypeListSpread(expr(K::TypeValue(flag))),
            A::TypeListSpread(expr(K::TypeListValue(vec![Term::Bool(true)]))),
            A::TypeListSpread(expr(K::TypeValue(Term::Rigid {
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
                    &mut Budget::default(),
                    resolve
                )
                .is_none()
            );
        }
        let unknown = input(vec![
            A::Value(expr(K::Unit)),
            A::Value(expr(K::TypeValue(Term::Primitive(Primitive::Bool)))),
        ]);
        let mut visits = 0;
        assert!(
            collect_known_normal_argument(&unknown, &mut Budget::default(), |e, b| {
                visits += 1;
                resolve(e, b)
            })
            .is_none()
        );
        assert_eq!(visits, 1);
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
            Some(true)
        );
        assert_eq!(
            known_type_equality(&context, &boolean, &character, &mut Budget::default()),
            Some(false)
        );
        assert_eq!(
            known_type_equality(&context, &opaque, &boolean, &mut Budget::default()),
            None
        );
        assert_eq!(
            known_type_equality(&context, &opaque, &opaque, &mut Budget::default()),
            Some(true)
        );
        let mut empty = Budget {
            steps: 0,
            max_limbs: 4096,
        };
        assert_eq!(
            known_type_equality(&context, &boolean, &boolean, &mut empty),
            None
        );
        assert_eq!(
            known_type_equality(&context, &boolean, &boolean, &mut Budget::default()),
            Some(true)
        );
        assert_eq!(
            known_type_equality(
                &context,
                &Term::Bool(true),
                &Term::Bool(true),
                &mut Budget::default()
            ),
            None
        );
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

    fn prepare_normal_collection_input(
        &mut self,
        value: n::Expr,
        before: usize,
        preparations: &mut Vec<n::NormalFormPreparation>,
        known_bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let binding = self.spread_binding(span)?;
        let mut budget = comptime_runtime::Budget {
            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
            max_limbs: 4096,
        };
        let budget_before = budget.steps;
        let known = self.known_normal_input_in(&value, 0, &mut budget, known_bindings);
        self.required_steps -= (budget_before - budget.steps) as u64;
        if let Some(known) = known {
            known_bindings.insert(binding, std::sync::Arc::new(known));
        }

        let ty = value.ty.clone();
        self.locals.insert(binding, ty.clone());
        let location = value.source;
        let diverges = is_never(&ty);
        let tuple_ty = if diverges {
            p(P::Never)
        } else {
            self.tuple_value_type(vec![ty.clone()], span)?
        };
        preparations.push(n::NormalFormPreparation {
            before,
            expression: n::Expr {
                source: location,
                ty: if diverges { p(P::Never) } else { p(P::Void) },
                kind: K::LetPattern {
                    pattern: n::Pattern::Tuple(vec![n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    }]),
                    value: Box::new(n::Expr {
                        source: location,
                        ty: tuple_ty,
                        kind: K::Tuple(vec![value]),
                    }),
                    guard: None,
                    otherwise: None,
                },
            },
        });
        Ok(n::Expr {
            source: location,
            ty,
            kind: K::Local(binding),
        })
    }

    /// Prepare explicit inputs in source order, then omitted inputs in their
    /// declaration telescope. All reuse goes through fresh local bindings.
    fn defaulted_normal_expression(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let clauses = self.type_clause_params(owner);
        let parameters = self.normal_parameters[&owner].clone();
        let saved_locals = self.locals.clone();
        let saved_bindings = self.normal_default_bindings.clone();
        let saved_types = self.alias_substitution.clone();
        let saved_type_values = self.alias_value_bindings.clone();
        let saved_context = self.context.clone();
        let saved_owner = self.signature_owner;
        let saved_mode = self.in_required_comptime;
        let caller_scope = self.call_inference_domains();
        let caller_owner = self.default_input_owner.or(self.signature_owner);
        let saved_input_owner = self.default_input_owner;
        self.default_input_owner = caller_owner;
        let saved_input_scope = self.default_input_scope.replace(caller_scope);
        let result = (|| {
            // Layout hints establish only positions. Actual packet checking
            // still validates the tuple and its element types. Do not expose
            // a head's unresolved Type binders as caller-side expectations.
            let positional_types = parameters
                .iter()
                .zip(&clauses)
                .filter(|(_, clause)| {
                    !matches!(
                        clause.kind,
                        hir::clause::ClauseParamKind::Optional(..)
                            | hir::clause::ClauseParamKind::Varadic(..)
                    )
                })
                .map(|((_, _, ty), _)| ty.clone())
                .collect::<Vec<_>>();
            let collector = self.normal_type_list_tail(owner).then(|| clauses.len() - 1);
            let whole_collector = collector.is_some_and(|slot| {
                inputs.iter().any(|input|
                matches!(input, Arg::Named(name, _) if name.name == clauses[slot].name.name))
            });
            let mut layout_inputs = inputs.to_vec();
            let mut list_spreads = BTreeSet::new();
            let mut remaining = Some(positional_types.len());
            let mut layout_fuel = 10_000;
            if let Some(slot) = collector.filter(|_| !whole_collector) {
                for (index, input) in inputs.iter().enumerate() {
                    match input {
                        Arg::Positional(value) if remaining == Some(0) => {
                            layout_inputs[index] = Arg::Named(clauses[slot].name, value);
                        }
                        Arg::Expand(value) if remaining == Some(0) => {
                            layout_inputs[index] = Arg::Named(clauses[slot].name, value);
                            list_spreads.insert(index);
                        }
                        Arg::Positional(_) => remaining = remaining.map(|n| n.saturating_sub(1)),
                        Arg::Expand(value) => {
                            remaining =
                                match (remaining, self.spread_width(value, &mut layout_fuel, 0)?) {
                                    (Some(left), Some(width)) if width <= left => {
                                        Some(left - width)
                                    }
                                    _ => None,
                                };
                        }
                        _ => {}
                    }
                }
            }
            let packet_contexts = if inputs.iter().any(|input| matches!(input, Arg::Expand(_))) {
                self.spread_packet_layout(&layout_inputs, &positional_types, span)?
            } else {
                vec![None; inputs.len()]
            };
            let mut routed = vec![None; parameters.len()];
            let mut preparations = Vec::new();
            let mut order = Vec::new();
            let mut positional = 0;
            let mut collected = Vec::new();
            let mut prepared_bindings = BTreeMap::new();
            for (input_index, input) in inputs.iter().enumerate() {
                if let Arg::Expand(packet) = input {
                    if list_spreads.contains(&input_index) {
                        let slot = collector.expect("list spread has a collector");
                        let value = self.expr(packet, Some(&parameters[slot].2))?;
                        let value = self.prepare_normal_collection_input(
                            value,
                            order.len(),
                            &mut preparations,
                            &mut prepared_bindings,
                            span,
                        )?;
                        collected.push(n::NormalFormArgument::TypeListSpread(value));
                        continue;
                    }

                    let context = if let Some(fields) = &packet_contexts[input_index] {
                        use crate::calls::InputContext as C;
                        let dependent =
                            |ty: &Term| parameters.iter().any(|(id, _, _)| ty.mentions_rigid(*id));
                        if fields.iter().any(dependent) {
                            let fields = fields
                                .iter()
                                .map(|ty| {
                                    if dependent(ty) {
                                        C::Unknown
                                    } else {
                                        C::Known(ty.clone())
                                    }
                                })
                                .collect();
                            Some(match self.bound_tuple_head(packet.span)? {
                                Some(head) => C::NormalTuple { head, fields },
                                None => C::Tuple(fields),
                            })
                        } else {
                            Some(C::Known(
                                self.tuple_value_type(fields.clone(), packet.span)?,
                            ))
                        }
                    } else {
                        None
                    };
                    let value = match context {
                        Some(context) => self.input_context_expression(packet, &context)?,
                        None => self.expr(packet, None)?,
                    };
                    let fields = self.prepared_packet_fields(
                        &value,
                        &layout_inputs,
                        input_index,
                        &positional_types,
                        packet.span,
                    )?;
                    let mut budget = comptime_runtime::Budget {
                        steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                        max_limbs: 4096,
                    };
                    let budget_before = budget.steps;
                    let observed =
                        self.known_normal_input_in(&value, 0, &mut budget, &prepared_bindings);
                    self.required_steps -= (budget_before - budget.steps) as u64;
                    let mut observed_fields = match observed {
                        Some(Term::TupleValue(values)) if values.len() == fields.len() => {
                            Some(values.into_iter())
                        }
                        _ => None,
                    };
                    let before = order.len();
                    let mut pattern = Vec::new();
                    for ty in fields {
                        while positional < clauses.len()
                            && matches!(
                                clauses[positional].kind,
                                hir::clause::ClauseParamKind::Optional(..)
                                    | hir::clause::ClauseParamKind::Varadic(..)
                            )
                        {
                            positional += 1;
                        }
                        let slot = positional;
                        positional += 1;
                        let binding = self.spread_binding(packet.span)?;
                        if let Some(known) = observed_fields.as_mut().and_then(Iterator::next) {
                            prepared_bindings.insert(binding, std::sync::Arc::new(known));
                        }
                        self.locals.insert(binding, ty.clone());
                        pattern.push(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        });
                        let input = n::Expr {
                            source: value.source,
                            ty,
                            kind: K::Local(binding),
                        };
                        let cell = routed
                            .get_mut(slot)
                            .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
                        if cell.replace(RoutedNormalInput::Prepared(input)).is_some() {
                            return Err(error(span, "duplicate normal-form argument"));
                        }
                        order.push(slot);
                    }
                    preparations.push(n::NormalFormPreparation {
                        before,
                        expression: n::Expr {
                            source: n::SourceRange {
                                start: value.source.end,
                                end: value.source.end,
                            },
                            ty: if is_never(&value.ty) {
                                p(P::Never)
                            } else {
                                p(P::Void)
                            },
                            kind: K::LetPattern {
                                pattern: n::Pattern::Tuple(pattern),
                                value: Box::new(value),
                                guard: None,
                                otherwise: None,
                            },
                        },
                    });
                    continue;
                }
                let (slot, value) = match input {
                    Arg::Positional(value) => {
                        while positional < clauses.len()
                            && matches!(
                                clauses[positional].kind,
                                hir::clause::ClauseParamKind::Optional(..)
                            )
                        {
                            positional += 1;
                        }
                        if collector == Some(positional) {
                            if whole_collector {
                                return Err(error(
                                    span,
                                    "cannot combine a complete type-list argument with collected inputs",
                                ));
                            }
                            let value = self.expr(value, Some(&p(P::Type)))?;
                            let value = self.prepare_normal_collection_input(
                                value,
                                order.len(),
                                &mut preparations,
                                &mut prepared_bindings,
                                span,
                            )?;
                            collected.push(n::NormalFormArgument::Value(value));
                            continue;
                        }
                        let slot = positional;
                        positional += 1;
                        (slot, *value)
                    }
                    Arg::Named(name, value) => (
                        clauses
                            .iter()
                            .position(|clause| {
                                clause.name.name == name.name
                                    && matches!(
                                        clause.kind,
                                        hir::clause::ClauseParamKind::Optional(..)
                                            | hir::clause::ClauseParamKind::Varadic(..)
                                    )
                            })
                            .ok_or_else(|| error(span, "unknown named normal-form argument"))?,
                        *value,
                    ),
                    _ => {
                        return Err(error(
                            span,
                            "normal-form input expansion is not yet implemented",
                        ));
                    }
                };
                let cell = routed
                    .get_mut(slot)
                    .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
                if cell.replace(RoutedNormalInput::Source(value)).is_some() {
                    return Err(error(span, "duplicate normal-form argument"));
                }
                order.push(slot);
            }
            if !collected.is_empty() {
                let slot = collector.expect("collected inputs have a collector");
                routed[slot] = Some(RoutedNormalInput::Collected(collected));
                order.push(slot);
            }
            let mut values = vec![None; parameters.len()];
            let mut known: Vec<Option<Term>> = vec![None; parameters.len()];
            let bindings = (0..parameters.len())
                .map(|_| self.spread_binding(span))
                .collect::<Result<Vec<_>, _>>()?;
            let mut evaluation_order = Vec::new();
            // Type information is accumulated without invoking input code.
            let mut known_parameters = Vec::new();
            let mut known_types = Vec::new();
            for slot in order.into_iter().chain(0..parameters.len()) {
                if values[slot].is_some() {
                    continue;
                }
                if routed[slot].is_none()
                    && matches!(
                        clauses[slot].kind,
                        hir::clause::ClauseParamKind::Varadic(..)
                    )
                {
                    // An unsupplied collector has zero elements, not a missing
                    // required input. Keep it in the normal-form telescope.
                    let ty = self.instantiate_input_types(
                        &parameters[slot].2,
                        &known_parameters,
                        &known_types,
                        span,
                    )?;
                    if !self.is_type_list_type(&ty) {
                        return Err(error(
                            span,
                            "normal-form variadic collector requires meta.List<Type>",
                        ));
                    }
                    known[slot] = Some(Term::TypeListValue(Vec::new()));
                    self.locals.insert(bindings[slot], ty.clone());
                    values[slot] = Some(n::NormalFormArgument::Value(n::Expr {
                        source: source(span),
                        ty,
                        kind: K::TypeListValue(Vec::new()),
                    }));
                    evaluation_order.push(slot);
                    continue;
                }
                let explicit = routed[slot].as_ref();
                let input = match explicit {
                    Some(RoutedNormalInput::Source(input)) => Some(*input),
                    Some(RoutedNormalInput::Prepared(_) | RoutedNormalInput::Collected(_)) => None,
                    None => {
                        // Defaults can see only preceding declarations, regardless
                        // of which later named inputs have already been evaluated.
                        for previous in 0..slot {
                            if values[previous].is_none() {
                                return Err(error(
                                    span,
                                    "normal-form default awaits preceding input",
                                ));
                            }
                            if parameters[previous].2 == p(P::Type) {
                                if let Some(ty) = &known[previous] {
                                    self.alias_substitution.insert(
                                        parameters[previous].0,
                                        std::sync::Arc::new(ty.clone()),
                                    );
                                } else {
                                    self.alias_value_bindings
                                        .insert(parameters[previous].0, bindings[previous]);
                                }
                            } else {
                                // A value prefix contributes to later type identities
                                // as well as to executable default expressions. Keep
                                // the local for evaluation and the known term for
                                // dependent nominal domains; neither runs input code.
                                if let Some(value) = &known[previous] {
                                    let value = self.copy_alias_type(value, span)?;
                                    self.alias_substitution
                                        .insert(parameters[previous].0, std::sync::Arc::new(value));
                                }
                                self.normal_default_bindings.insert(
                                    parameters[previous].0,
                                    (
                                        bindings[previous],
                                        self.instantiate_input_types(
                                            &self.locals[&bindings[previous]],
                                            &known_parameters,
                                            &known_types,
                                            span,
                                        )?,
                                    ),
                                );
                            }
                        }
                        self.signature_owner = Some(owner);
                        self.context = self.binder_context(owner);
                        self.in_required_comptime = true;
                        let hir::clause::ClauseParamKind::Optional(_, _, default) =
                            clauses[slot].kind
                        else {
                            return Err(error(span, "normal-form argument count mismatch"));
                        };
                        Some(default)
                    }
                };
                let ty = self.instantiate_input_types(
                    &parameters[slot].2,
                    &known_parameters,
                    &known_types,
                    span,
                )?;
                if explicit.is_none()
                    && (self.active_type_defaults.len() >= 64
                        || !self.active_type_defaults.insert((owner, slot)))
                {
                    return Err(error(
                        span,
                        "cyclic or excessively deep normal-form default",
                    ));
                }
                let dependent = parameters.iter().any(|(id, _, _)| ty.mentions_rigid(*id));
                let checked = if let Some(RoutedNormalInput::Collected(items)) = explicit {
                    Ok(n::NormalFormArgument::TypeList {
                        source: source(span),
                        items: items.clone(),
                    })
                } else if let Some(RoutedNormalInput::Prepared(value)) = explicit {
                    if !dependent {
                        self.require_expression(&value.ty, &ty, span)?;
                    }
                    Ok(n::NormalFormArgument::Value(value.clone()))
                } else if dependent {
                    self.dependent_normal_argument(input.unwrap(), 0)
                } else {
                    self.expr(input.unwrap(), Some(&ty))
                        .map(n::NormalFormArgument::Value)
                };
                if explicit.is_none() {
                    self.active_type_defaults.remove(&(owner, slot));
                }
                let value = checked?;
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let budget_before = budget.steps;
                known[slot] = match &value {
                    n::NormalFormArgument::TypeList { items, .. } => {
                        self.known_collected_type_list_in(items, &prepared_bindings)
                    }
                    _ => value.expression().and_then(|value| {
                        self.known_normal_input_in(value, 0, &mut budget, &prepared_bindings)
                    }),
                };
                self.required_steps -= (budget_before - budget.steps) as u64;
                if let Some(known) = &known[slot] {
                    prepared_bindings.insert(
                        bindings[slot],
                        std::sync::Arc::new(self.copy_alias_type(known, span)?),
                    );
                }
                if let Some(value) = &known[slot] {
                    known_parameters.push(parameters[slot].0);
                    known_types.push(value.clone());
                }
                self.locals.insert(
                    bindings[slot],
                    value.expression().map(|expr| expr.ty.clone()).unwrap_or(ty),
                );
                values[slot] = Some(value);
                evaluation_order.push(slot);
            }
            self.signature_owner = saved_owner;
            self.context = saved_context.clone();
            self.in_required_comptime = saved_mode;
            let mut deferred = BTreeSet::new();
            let terms = known
                .into_iter()
                .zip(&parameters)
                .map(|(known, (id, _, ty))| {
                    known.unwrap_or_else(|| {
                        deferred.insert(*id);
                        Term::Rigid {
                            declaration: *id,
                            sort: if *ty == p(P::Type) {
                                Sort::Type
                            } else {
                                Sort::ComptimeValue
                            },
                        }
                    })
                })
                .collect();
            self.check_formation_application(
                owner,
                FormationInputs {
                    terms,
                    deferred,
                    environment: BTreeMap::new(),
                },
                span,
            )?;
            let arguments: Vec<_> = values.into_iter().map(Option::unwrap).collect();
            let ty = if arguments.iter().any(n::NormalFormArgument::is_diverging)
                || preparations.iter().any(|p| is_never(&p.expression.ty))
            {
                p(P::Never)
            } else {
                p(P::Type)
            };
            if let Some(expected) = expected {
                self.require_expression(&ty, expected, span)?;
            }
            Ok(n::Expr {
                source: source(span),
                ty,
                kind: K::NormalFormType {
                    head: declaration(self.package, &self.package_identities, owner),
                    arguments,
                    plan: Some(n::NormalFormInputPlan {
                        order: evaluation_order,
                        preparations,
                        bindings,
                    }),
                },
            })
        })();
        self.locals = saved_locals;
        self.normal_default_bindings = saved_bindings;
        self.alias_substitution = saved_types;
        self.alias_value_bindings = saved_type_values;
        self.context = saved_context;
        self.signature_owner = saved_owner;
        self.in_required_comptime = saved_mode;
        self.default_input_scope = saved_input_scope;
        self.default_input_owner = saved_input_owner;
        result
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
            let mut budget = comptime_runtime::Budget {
                steps: self.required_steps as usize,
                max_limbs: 4096,
            };
            let before = budget.steps;
            let known = self.known_normal_input_in(input, 0, &mut budget, &scoped);
            self.required_steps -= (before - budget.steps) as u64;
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
