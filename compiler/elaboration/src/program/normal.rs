//! Opaque head inputs: a stable value or a declaration-side symbolic input.
use super::*;
use observation::Failure;
mod arguments;
mod control_observations;
mod dynamic_inputs;
pub(super) mod known_patterns;
pub(super) mod known_scalars;
pub(super) mod observation;
mod supply;
mod values;

pub(super) use supply::{Application, Supply};

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

    pub(super) fn normal_head_contract(
        &self,
        owner: OwnerId,
    ) -> Option<(DeclarationId, nir::normal_forms::Role)> {
        use nir::normal_forms::Role;
        if !self.is_registered_normal_form(owner) {
            return None;
        }
        let head = declaration(self.package, &self.package_identities, owner);
        let parameters = &self.normal_parameters[&owner];
        let role = match parameters.as_slice() {
            [(_, _, ty)] if Some(head) == self.protocols.meta_list && *ty == p(P::Type) => {
                Role::TypeList
            }
            [(_, _, ty)]
                if Some(head) == self.protocols.tuple
                    && self.clause_collector(owner).is_some()
                    && self.is_type_list_type(ty) =>
            {
                Role::Tuple
            }
            _ => Role::Opaque,
        };
        Some((head, role))
    }

    pub(super) fn publish_normal_head(&mut self, owner: OwnerId) {
        if let Some((head, role)) = self.normal_head_contract(owner) {
            self.declared_theory.admit_normal_form_head(head, role);
        }
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
            || self.clause_collector(owner).is_none()
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

    pub(super) fn checked_collected_list_in(
        &mut self,
        items: &[n::NormalFormArgument],
        ty: &Term,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        self.with_observation_budget(span, |this, budget| {
            let Some(element) = this.list_element_type(ty) else {
                return Ok(None);
            };
            observation::optional(collect_list_inputs(
                ty,
                element,
                items.iter(),
                budget,
                |item, budget| {
                    let (expr, spread) = match item {
                        n::NormalFormArgument::Value(expr) if expr.ty == *element => (expr, false),
                        n::NormalFormArgument::ListSpread(expr) if expr.ty == *ty => (expr, true),
                        _ => return Err(Failure::Unavailable),
                    };
                    Ok((
                        this.observe_normal_input_in(expr, 0, budget, bindings)?,
                        spread,
                    ))
                },
            ))
        })
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
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(value.span, "normal-form input structure budget exhausted"))?;
        // A dependent input specialized to Type is a type identity, not a
        // scalar comptime-value reference with the wrong semantic sort.
        if *ty == p(P::Type) {
            return self.type_expr(value);
        }
        if let Term::Lift(expected_inner) = ty {
            return self.runtime_lift_argument(value, expected_inner);
        }
        if let H::ResolvedLocal(_, id) = value.kind
            && let Some(input) = self.callable_type_input_expression(value, local(id), Some(ty))?
        {
            return match input.kind {
                K::TypeValue(symbol) | K::ComptimeValue(symbol) => Ok(symbol),
                _ => unreachable!("callable telescope supplies abstract values"),
            };
        }
        if let H::ResolvedLocal(_, id) = value.kind
            && let Some((symbol, actual)) = self
                .input_facts
                .value_identity(&local(id))
                .filter(|_| self.locals.contains_key(&local(id)))
        {
            let symbol = symbol.clone();
            let actual = actual.clone();
            self.require(&actual, ty, value.span)?;
            return Ok(symbol);
        }
        if let H::ResolvedFnParam(_, id) = value.kind {
            let binding = local(id);
            let in_signature = self.signature_owner.is_some_and(|owner| {
                owner == id.owner
                    && (self.active_signatures.contains(&owner) || self.checking_contract)
            });
            if let Some((symbol, actual)) = self
                .input_facts
                .value_identity(&binding)
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
            // Symbolic inputs use the same contextual conversions as ordinary
            // expressions. Observing a checked identifier preserves its scope;
            // it does not require a concrete invocation-time value.
            let checked = self.expr(value, Some(ty))?;
            if let Some(identity) =
                self.preserved_input_identity(&checked, |_, _| Ok(None), value.span)?
            {
                return Ok(identity);
            }
            let declaration = self.visible_clause_input(id, value.span)?;
            if let K::Local(binding) = &checked.kind
                && self.default_template_inputs.get(&declaration) == Some(binding)
            {
                return Ok(Term::Rigid {
                    declaration,
                    sort: Sort::of_comptime_domain(&checked.ty),
                });
            }
            let (result, _) = self.required_normal_input(value, checked, Vec::new())?;
            return self.required_normal_term(result, value.span);
        }
        if !self.in_required_comptime
            && (self.describing_open_call
                || self.checking_open_behavior
                || self.in_unselected_inline)
        {
            let checked = self.with_staged_input(true, |this| this.expr(value, Some(ty)))?;
            if self
                .signature_owner
                .is_some_and(|owner| self.open_functions.contains_key(&owner))
            {
                self.with_staged_input(true, |this| {
                    this.check_open_fragment(&checked, value.span, false)
                })?;
            }
            if let Some(identity) =
                self.preserved_input_identity(&checked, |_, _| Ok(None), value.span)?
            {
                return Ok(identity);
            }
            let mut staged_bindings: std::collections::BTreeSet<_> = self
                .signature_owner
                .and_then(|owner| self.package.item(owner))
                .map(|item| match &item.kind {
                    ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig
                        .params
                        .iter()
                        .filter(|parameter| parameter.ty.is_comptime())
                        .map(|parameter| local(parameter.hir_id))
                        .collect(),
                    _ => std::collections::BTreeSet::new(),
                })
                .unwrap_or_default();
            staged_bindings.extend(self.open_staged_locals.iter().copied());
            staged_bindings.extend(self.ordinary_behavior_inputs.keys().copied());
            let declaration_inputs = self.visible_comptime_parameters(value.span)?;
            if self.staged_input_can_defer(
                &checked,
                false,
                &std::collections::BTreeSet::new(),
                &staged_bindings,
                &declaration_inputs,
                value.span,
            )? {
                let binder = self.fresh_value_binder(value.hir_id.owner, value.span)?;
                let symbol = Term::Rigid {
                    declaration: binder.declaration,
                    sort: Sort::ComptimeValue,
                };
                self.definition_refinements
                    .staged_values
                    .push((binder.declaration, checked.ty.clone()));
                if let H::ResolvedLocal(_, id) = value.kind {
                    let binding = local(id);
                    if self.locals.contains_key(&binding) {
                        self.input_facts
                            .values
                            .insert(binding, (symbol.clone(), checked.ty.clone()));
                    }
                }
                return Ok(symbol);
            }
            let (result, _) = self.required_normal_input(value, checked, Vec::new())?;
            return self.required_normal_term(result, value.span);
        }
        // Aggregate construction uses ordinary contextual checking and one
        // required root. Its lexical declaration inputs are captured by that
        // root; individual elements do not become separate invocations.
        let result = self.required_expression(value, Some(ty))?;
        self.required_normal_term(result, value.span)
    }

    /// Forward the identity in a quote binder without reading its runtime payload.
    /// Both written arguments and omitted defaults use the current telescope.
    pub(super) fn forward_runtime_dependency(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<Term>, Error> {
        let H::ResolvedClauseParam(_, id) = value.kind else {
            return Ok(None);
        };
        let declaration = self.visible_clause_input(id, value.span)?;
        let substitution = self.alias_substitution.get(&declaration);
        let binder = match substitution {
            Some(projected @ Term::RuntimeField { .. }) => {
                let identity = declaration_inputs::copy_input_domain(
                    projected,
                    &mut self.required_steps,
                    value.span,
                )?;
                if let Some(expected) = expected {
                    // The binder's lifted payload was checked when this identity
                    // was supplied. Substitute its declaration domain in the
                    // caller telescope, without acquiring the projected value.
                    let Some(Term::Lift(inner)) = self.trait_input_domain(declaration) else {
                        return Err(error(
                            value.span,
                            "forwarded runtime dependency domain is missing",
                        ));
                    };
                    let actual = self.substituted_trait_domain(&inner, value.span)?;
                    self.require(&actual, expected, value.span)?;
                }
                return Ok(Some(identity));
            }
            Some(Term::Runtime(id)) => {
                let Some(Term::Lift(inner)) = self.runtime_domains.get(id) else {
                    return Err(error(
                        value.span,
                        "substituted runtime dependency domain is missing",
                    ));
                };
                if let Some(expected) = expected {
                    self.require(inner, expected, value.span)?;
                }
                return Ok(Some(Term::Runtime(*id)));
            }
            Some(Term::Rigid {
                declaration,
                sort: Sort::Runtime,
            }) => *declaration,
            Some(_) => return Ok(None),
            None => declaration,
        };
        let domain = self
            .active_env_parameters
            .get(&binder)
            .map(|(_, ty)| Term::Lift(Box::new(ty.clone())))
            .or_else(|| self.trait_input_domain(binder))
            .or_else(|| self.nominal_input_prefixes.get(&binder).cloned())
            .or_else(|| {
                self.normal_input_prefixes
                    .values()
                    .flatten()
                    .find(|(id, _, _)| *id == binder)
                    .map(|(_, _, domain)| domain.clone())
            });
        let Some(Term::Lift(inner)) = domain else {
            if substitution.is_some() {
                return Err(error(
                    value.span,
                    "forwarded runtime dependency domain is missing",
                ));
            }
            return Ok(None);
        };
        if let Some(expected) = expected {
            let actual = self.substituted_trait_domain(&inner, value.span)?;
            self.require(&actual, expected, value.span)?;
        }
        Ok(Some(Term::Rigid {
            declaration: binder,
            sort: Sort::Runtime,
        }))
    }

    pub(super) fn runtime_dependency_body(&self, owner: Option<OwnerId>) -> bool {
        !self.observing_type
            && !self.checking_contract
            && self.captures.last().is_none_or(|frame| !frame.comptime)
            && (self.in_code_body
                || owner.is_some_and(|owner| {
                    !self.active_signatures.contains(&owner)
                        && self.package.item(owner).is_some_and(|item| {
                            matches!(
                                &item.kind,
                                ItemKind::Fn(sig, _)
                                    if !sig.modifiers.is_comptime
                                        && !sig.modifiers.is_normal_form_head
                            )
                        })
                }))
    }

    pub(super) fn runtime_lift_argument(
        &mut self,
        value: &Expr<'_>,
        expected_inner: &Term,
    ) -> Result<Term, Error> {
        if let Some(identity) = self.forward_runtime_dependency(value, Some(expected_inner))? {
            let (root, _) = self.runtime_field_path(&identity, value.span)?;
            self.local_runtime_origins
                .sources
                .insert(value.hir_id, root.clone());
            return Ok(identity);
        }
        if matches!(value.kind, H::ResolvedClauseParam(..)) {
            return Err(error(
                value.span,
                "quote runtime parameter expected a lift dependency, not a comptime input",
            ));
        }
        if !self.runtime_dependency_body(self.signature_owner) {
            if let Some(identity) = self.signature_runtime_projection(value, expected_inner)? {
                return Ok(identity);
            }
            return Err(error(
                value.span,
                "runtime quote argument requires a runtime function body or enclosing quote parameter",
            ));
        }
        let expected_inner = self.callable_input_view(expected_inner, value.span)?;
        let saved_required = std::mem::replace(&mut self.in_required_comptime, false);
        let saved_staged = std::mem::replace(&mut self.in_staged_input, false);
        let checked = self.expr(value, Some(&expected_inner));
        self.in_staged_input = saved_staged;
        self.in_required_comptime = saved_required;
        let checked = checked?;
        if is_never(&checked.ty) {
            return Err(error(
                value.span,
                "diverging expression cannot satisfy a runtime quote dependency",
            ));
        }
        let identity = self.intern_runtime_operand(checked, value, value.span)?;
        let (root, _) = self.runtime_field_path(&identity, value.span)?;
        self.local_runtime_origins
            .sources
            .insert(value.hir_id, root.clone());
        Ok(identity)
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
            .chain(self.generated_functions.parameter_declarations().cloned())
            .collect()
    }

    pub(super) fn normal_form_head_metadata(
        &self,
        owner: OwnerId,
    ) -> Option<nir::normal_forms::Head> {
        if !self.is_registered_normal_form(owner) {
            return None;
        }
        let parameters = self.normal_parameters.get(&owner)?;
        Some(nir::normal_forms::Head {
            collector: self
                .clause_collector(owner)
                .map(|(slot, name)| nir::normal_forms::Collector::named(slot, name.as_str())),
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
        let ty = self.normal_form_callable_type(owner, expr.span)?;
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::Function(declaration(self.package, &self.package_identities, owner)),
        })
    }

    pub(super) fn normal_input_parameters(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<declaration_inputs::Parameters, Error> {
        self.normal_parameters[&owner]
            .iter()
            .map(|(id, _, domain)| {
                Ok((
                    *id,
                    Sort::of_comptime_domain(domain),
                    declaration_inputs::copy_input_domain(domain, &mut self.required_steps, span)?,
                ))
            })
            .collect()
    }

    pub(super) fn normal_form_callable_type(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Term, Error> {
        let inputs = self
            .normal_parameters
            .get(&owner)
            .ok_or_else(|| error(span, "normal-form callable is missing declaration metadata"))?;
        let collector = self.clause_collector(owner).map(|(slot, _)| slot);
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let signature = nir::normal_forms::callable_type_from_declarations(
            inputs.iter().map(|(declaration, _, domain)| {
                (*declaration, Sort::of_comptime_domain(domain), domain)
            }),
            collector,
            &mut fuel,
        );
        self.required_steps -= (before - fuel) as u64;
        signature.map_err(|reason| error(span, reason.to_string()))
    }

    pub(super) fn static_normal_application(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Option<Application>, Error> {
        match self.package.item(owner).map(|item| &item.kind) {
            Some(ItemKind::Fn(signature, _)) if signature.modifiers.is_normal_form_head => {
                return Ok(Some(Application {
                    head: owner,
                    interface: Supply::Declaration(owner),
                }));
            }
            Some(ItemKind::Const(..)) => {}
            _ => return Ok(None),
        }
        let value = self.constant_value(owner, span)?;
        let Term::NormalFormFunction(_, slot) = &value.ty else {
            return Ok(None);
        };
        let K::Function(identity) = value.kind else {
            return Err(error(
                span,
                "normal-form constant has no completed declaration identity",
            ));
        };
        let implementation = self
            .source_declarations
            .owner(identity)
            .ok_or_else(|| error(span, "normal-form callable declaration is unavailable"))?;
        let interface = self.constant_normal_supply(owner, *slot, span)?;
        Ok(Some(Application {
            head: implementation,
            interface,
        }))
    }

    /// Type positions consume the same checked input plan as value positions.
    /// Observation can retain abstract declaration inputs; invocation is only
    /// required for computations that cannot supply static evidence.
    pub(super) fn normal_type_annotation(
        &mut self,
        expr: &Expr<'_>,
        application: Application,
        inputs: &[Arg<'_>],
    ) -> Result<CompletedType, Error> {
        // Input producers belong to comptime here. Runtime Splice must not
        // intercept packet formation while checking a type annotation.
        let head = application.head;
        let checked = self.with_staged_input(true, |this| {
            this.normal_type_expression(expr, application, inputs, Some(&p(P::Type)))
        })?;
        if let Some(ty) = self.checked_normal_input_in(&checked, &BTreeMap::new(), expr.span)?
            && let Term::NormalForm { arguments, .. } = &ty
        {
            self.check_type_application(head, arguments, expr.span)?;
            return Ok(CompletedType::Identity(ty));
        }
        // The input plan owns its lexical prefix. Its fields may be gradual
        // descriptions even when a sibling contains a bound function input.
        self.complete_checked_type(expr, checked)
    }

    pub(super) fn normal_type_expression(
        &mut self,
        expr: &Expr<'_>,
        application: Application,
        inputs: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.check_normal_application_region(application, inputs, expr.span, expected)
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
                self.with_observation_budget(span, |this, budget| {
                    this.normal_input_hint(&expr, budget)
                })
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
                let value = self.normal_input_hint(value, &mut budget);
                *fuel -= (before - budget.steps) as u64;
                let value = value.map_err(|failure| error(span, failure.to_string()))?;
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
        self.with_observation_budget(span, |this, budget| {
            observation::optional(this.observe_normal_input_in(expr, 0, budget, bindings))
        })
    }

    pub(super) fn with_observation_budget<T>(
        &mut self,
        span: Span,
        inspect: impl FnOnce(&Self, &mut comptime_runtime::Budget) -> Result<T, Failure>,
    ) -> Result<T, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps: before,
            max_limbs: 4096,
        };
        let result = inspect(self, &mut budget);
        self.required_steps -= (before - budget.steps) as u64;
        result.map_err(|failure| error(span, format!("static value observation failed: {failure}")))
    }

    pub(super) fn normal_input_hint(
        &self,
        expr: &n::Expr,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<Option<Term>, Failure> {
        observation::hint(self.observe_normal_input_in(expr, 0, budget, &BTreeMap::new()))
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

    pub(super) fn observe_error_conversion(
        &self,
        conversion: nir::error_results::Conversion,
        input: &Term,
        output: &Term,
        payload: &Term,
        fuel: &mut usize,
    ) -> Result<Term, observation::Failure> {
        nir::error_results::convert_frozen_in_scope(
            conversion,
            input,
            output,
            payload,
            &self.call_input_frames(),
            fuel,
            &|ty| self.bound_tuple_elements(ty),
        )
        .map_err(|reason| match reason {
            nir::error_results::Error::Budget => observation::Failure::Steps,
            _ => observation::Failure::Unavailable,
        })
    }

    // Control observation owns its traversal in control_observations. Keep
    // value constructors in a separate frame, entered only for non-control nodes.
    #[inline(never)]
    fn observe_normal_value_in(
        &self,
        expr: &n::Expr,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<Term, Failure> {
        match &expr.kind {
            // A checked named operand denotes its declaration if formation
            // succeeds. Observing the key does not perform formation or call it.
            K::Function(declaration) => Ok(Term::FunctionValue {
                declaration: *declaration,
                arguments: vec![],
            }),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Ok(Term::FunctionValue {
                declaration: *declaration,
                arguments: copy_observed_terms(arguments, budget)?,
            }),
            K::TraitMethod {
                interface,
                interface_arguments,
                member,
                subject,
                method_arguments,
            } if !expr.ty.is_comptime_callable() => {
                Ok(Term::MethodValue(Box::new(nir::TraitMethodIdentity {
                    interface: n::InterfaceInstance {
                        declaration: *interface,
                        arguments: copy_observed_terms(interface_arguments, budget)?,
                    },
                    subject: copy_observed_term(subject, budget)?,
                    member: *member,
                    method_arguments: copy_observed_terms(method_arguments, budget)?,
                })))
            }
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
            K::ErrorConvert { conversion, value } => {
                let payload = self.observe_normal_input_in(value, depth + 1, budget, bindings)?;
                self.observe_error_conversion(
                    *conversion,
                    &value.ty,
                    &expr.ty,
                    &payload,
                    &mut budget.steps,
                )
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
                    .map_err(|_| Failure::Evaluation("list index overflow".into()))?;
                if index >= values.len() {
                    return Err(Failure::Evaluation("list index out of bounds".into()));
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
                    n::InputSchedule::normal(plan.as_ref()),
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
                    n::InputSchedule {
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
                    .map_err(|_| Failure::Evaluation("bit-vector width overflow".into()))?;
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
            K::TypeKind { kind, operand } => {
                let operand = self.observe_normal_input_in(operand, depth + 1, budget, bindings)?;
                known_type_kind(&self.context, *kind, &operand, budget).map(Term::Bool)
            }
            K::TypeLayout { query, operand } => {
                let operand = self.observe_normal_input_in(operand, depth + 1, budget, bindings)?;
                let layout = nir::compute_layout(self, &operand, &mut budget.steps).map_err(
                    |err| match err {
                        nir::LayoutError::TargetRequired | nir::LayoutError::Unresolved(_) => {
                            Failure::Unavailable
                        }
                        nir::LayoutError::BudgetExhausted => Failure::Steps,
                        nir::LayoutError::ComptimeOnly(_)
                        | nir::LayoutError::Cycle(_)
                        | nir::LayoutError::Unsupported(_)
                        | nir::LayoutError::Overflow => Failure::Evaluation(err.to_string()),
                    },
                )?;
                let value = u128::from(query.select(layout));
                if let Some(target) = self.target
                    && target.pointer_bits() < 128
                    && let Some(limit) = 1u128.checked_shl(target.pointer_bits())
                    && value >= limit
                {
                    return Err(Failure::Evaluation(
                        "layout size overflows target usize".into(),
                    ));
                }
                Ok(Term::Integer(comptime_runtime::Integer::from_u128(value)))
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
                    if let Some((symbol, _)) = self.input_facts.value_identity(binding) {
                        return copy_observed_term(symbol, budget);
                    }
                    if let Some(symbol) = self.input_facts.type_identity(binding) {
                        return copy_observed_term(symbol, budget);
                    }
                }
                let parameters = self
                    .normal_parameters
                    .get(&self.signature_owner.ok_or(Failure::Unavailable)?)
                    .ok_or(Failure::Unavailable)?;
                let (declaration, _, domain) = parameters
                    .iter()
                    .find(|(_, local, _)| local == binding)
                    .ok_or(Failure::Unavailable)?;
                Ok(Term::Rigid {
                    declaration: *declaration,
                    sort: Sort::of_comptime_domain(domain),
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
        if let H::Lit(hir::Lit {
            kind: hir::LitKind::Symbol(name),
            ..
        }) = &input.kind
        {
            let name = name.as_str();
            self.required_steps = self
                .required_steps
                .checked_sub(name.len() as u64)
                .ok_or_else(|| error(input.span, "contextual symbol input budget exhausted"))?;
            return Ok(n::NormalFormArgument::Symbol {
                source: source(input.span),
                name: name.to_owned(),
            });
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
        if matches!(input.kind, H::List(_) | H::Tuple(_)) {
            return self.check_contextual_aggregate(input, depth);
        }
        if matches!(input.kind, H::Match(..)) {
            return self.check_contextual_match_region(input, depth);
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
            return self.check_contextual_when_region(input, depth);
        }
        if matches!(input.kind, H::If(_, _, Some(_))) {
            return self.check_contextual_if_region(input, depth);
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
        self.check_contextual_block_region(block, block_regions::ContextualTail { depth })
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
    let sort = Sort::of_comptime_domain(element);
    fields
        .iter()
        .all(|field| field.sort() == sort)
        .then_some(fields)
        .ok_or_else(|| Failure::Invalid("list element sort mismatch".into()))
}

/// Assemble already admitted elements and complete-list spreads in source
/// order. The reader owns operand inspection. A frozen caller moves its terms
/// here directly, without making expressions that would need checking again.
pub(super) fn collect_list_inputs<I>(
    ty: &Term,
    element: &Term,
    inputs: impl ExactSizeIterator<Item = I>,
    budget: &mut comptime_runtime::Budget,
    mut read: impl FnMut(I, &mut comptime_runtime::Budget) -> Result<(Term, bool), Failure>,
) -> Result<Term, Failure> {
    budget.consume(inputs.len())?;
    let domain = copy_observed_term(ty, budget)?;
    let mut parts = Vec::new();
    let mut fields = Vec::new();
    for item in inputs {
        let (value, spread) = read(item, budget)?;
        if spread {
            if !fields.is_empty() {
                parts.push(freeze_known_list(
                    ty,
                    element,
                    std::mem::take(&mut fields),
                    budget,
                )?);
            }
            parts.push(value);
        } else {
            fields.push(value);
        }
    }
    if !fields.is_empty() {
        parts.push(freeze_known_list(ty, element, fields, budget)?);
    }
    for part in &parts {
        nir::type_computations::charge_term(part, &mut budget.steps)?;
    }
    Term::list_concat(domain, parts).map_err(|reason| Failure::Invalid(reason.to_string()))
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

fn known_type_formula(
    context: &crate::ptc::Context,
    positive: crate::ptc::Formula,
    negative: crate::ptc::Formula,
    budget: &mut comptime_runtime::Budget,
) -> Result<bool, Failure> {
    use crate::ptc::{Outcome, UnknownReason};
    let mut proof_budget = crate::ptc::Budget {
        steps: budget.steps,
        max_depth: 128,
    };
    let result = (|| {
        for (proposition, value) in [(positive, true), (negative, false)] {
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

/// A type relation needs a proof in either direction. A failed equality query
/// is not a disequality fact, and an opaque head is never reduced to its body.
fn known_type_equality(
    context: &crate::ptc::Context,
    left: &Term,
    right: &Term,
    budget: &mut comptime_runtime::Budget,
) -> Result<bool, Failure> {
    use crate::ptc::Formula;
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
    known_type_formula(context, equality, inequality, budget)
}

fn known_type_kind(
    context: &crate::ptc::Context,
    kind: nir::TypeKind,
    operand: &Term,
    budget: &mut comptime_runtime::Budget,
) -> Result<bool, Failure> {
    use crate::ptc::Formula;
    if operand.sort() != Sort::Type {
        return Err(Failure::Unavailable);
    }
    let positive = Formula::kind(copy_observed_term(operand, budget)?, kind);
    let negative = Formula::kind(copy_observed_term(operand, budget)?, kind).negate();
    known_type_formula(context, positive, negative, budget)
}

impl nir::LayoutWorld for Checker<'_, '_> {
    fn target(&self) -> Option<nir::target::TargetData> {
        self.target
    }

    fn is_tuple_head(&self, head: DeclarationId) -> bool {
        self.protocols.tuple == Some(head)
    }

    fn is_list_head(&self, head: DeclarationId) -> bool {
        self.protocols.meta_list == Some(head)
    }

    fn nominal_kind(&self, ty: &Term) -> Option<nir::NominalKind> {
        let Term::Nominal { declaration, .. } = ty else {
            return None;
        };
        if self.structs.contains_key(declaration) {
            Some(nir::NominalKind::Struct)
        } else if self.enums.contains_key(declaration) {
            Some(nir::NominalKind::Enum)
        } else if self.newtypes.contains_key(declaration) {
            Some(nir::NominalKind::Newtype)
        } else {
            None
        }
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::NominalLayout>, nir::LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        let admission_err = |err: nir::NominalAdmissionError| match err {
            nir::NominalAdmissionError::Budget => nir::LayoutError::BudgetExhausted,
            nir::NominalAdmissionError::Unresolved(_) => nir::LayoutError::Unresolved(ty.clone()),
            nir::NominalAdmissionError::Invalid(_) => nir::LayoutError::Unsupported(ty.clone()),
        };
        if let Some(def) = self.structs.get(declaration) {
            let fields = nir::normal_forms::instantiate_value_fields_checked(
                &def.type_parameters,
                &def.parameter_types,
                arguments,
                def.fields.iter().map(|(_, t)| t),
                fuel,
            )
            .map_err(admission_err)?;
            return Ok(Some(if def.packed {
                nir::NominalLayout::PackedStruct(fields)
            } else {
                nir::NominalLayout::Struct(fields)
            }));
        }
        if let Some(def) = self.newtypes.get(declaration) {
            let repr =
                def.representation_budgeted(arguments, &[], fuel)
                    .map_err(|err| match err {
                        nir::newtypes::ConversionError::Budget => nir::LayoutError::BudgetExhausted,
                        _ => nir::LayoutError::Unsupported(ty.clone()),
                    })?;
            return Ok(Some(nir::NominalLayout::Newtype(repr)));
        }
        if let Some(def) = self.enums.get(declaration) {
            if def.symbol_encoding.is_some() {
                return Ok(Some(nir::NominalLayout::EnumSymbol));
            }
            if let Some(encoding) = &def.encoding {
                let bits = u32::try_from(encoding.width())
                    .map_err(|_| nir::LayoutError::Unsupported(ty.clone()))?;
                return Ok(Some(nir::NominalLayout::EnumFixedBits(bits)));
            }
            if def.variants.iter().all(|v| v.fields.is_empty()) {
                return Ok(Some(nir::NominalLayout::EnumUnit));
            }
            let mut variants = Vec::with_capacity(def.variants.len());
            for variant in &def.variants {
                let fields = nir::normal_forms::instantiate_value_fields_checked(
                    &def.type_parameters,
                    &def.parameter_types,
                    arguments,
                    variant.fields.iter(),
                    fuel,
                )
                .map_err(admission_err)?;
                variants.push(fields);
            }
            return Ok(Some(nir::NominalLayout::EnumAggregate(variants)));
        }
        Ok(None)
    }
}

fn copy_observed_term(term: &Term, budget: &mut comptime_runtime::Budget) -> Result<Term, Failure> {
    nir::copy_budget::clone_term(term, &mut budget.steps).map_err(Failure::from)
}

fn copy_observed_terms(
    terms: &[Term],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<Term>, Failure> {
    budget.consume(terms.len())?;
    terms
        .iter()
        .map(|term| copy_observed_term(term, budget))
        .collect()
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
        for (slot, clause) in self.type_clause_params(owner).iter().enumerate() {
            if matches!(
                clause.kind.unquoted(),
                hir::clause::ClauseParamKind::Optional(..)
            ) {
                self.checked_normal_default(owner, slot)?;
            }
        }
        Ok(())
    }

    /// Check a default in its declaration telescope, independently of whether
    /// any particular caller has executed its prefix producers. The returned
    /// code still refers to formal inputs; connecting it is a separate step.
    pub(super) fn checked_normal_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
    ) -> Result<n::Expr, Error> {
        let clauses = self.type_clause_params(owner);
        let hir::clause::ClauseParamKind::Optional(_, _, value) = clauses[slot].kind.unquoted()
        else {
            return Err(error(
                clauses[slot].span,
                "normal-form input has no default",
            ));
        };
        self.in_formal_declaration(owner, |this| {
            this.in_required_comptime = true;
            this.context = this.binder_context(owner);
            let parameters = &this.normal_parameters[&owner];
            let domain = declaration_inputs::copy_input_domain(
                &parameters[slot].2,
                &mut this.required_steps,
                value.span,
            )?;
            this.forbidden_default_inputs
                .extend(parameters[slot..].iter().map(|p| p.0));
            this.expr(value, Some(&domain))
        })
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
