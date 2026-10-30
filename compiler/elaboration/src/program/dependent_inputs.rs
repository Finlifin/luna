//! Prepare the compile-time dependencies of ordinary call input types.
//! Checking order does not change the source-ordered runtime argument plan.
use super::*;

pub(super) struct PreparedInputs {
    pub values: Vec<Option<n::Expr>>,
    pub symbols: Vec<Option<Term>>,
    pub live: Vec<Option<comptime_interpretor::Value>>,
    pub defaults: Vec<Option<n::Expr>>,
    active: BTreeSet<usize>,
}

impl PreparedInputs {
    pub fn new(count: usize) -> Self {
        Self {
            values: vec![None; count],
            symbols: vec![None; count],
            live: vec![None; count],
            defaults: vec![None; count],
            active: BTreeSet::new(),
        }
    }
}

impl Checker<'_, '_> {
    /// A required computation may wait for a staged parameter, but cannot
    /// capture an ordinary parameter of a runtime function. Ordinary inputs of
    /// a comptime function are supplied by its interpreter invocation instead.
    pub(super) fn required_runtime_parameter(&self, binding: n::LocalId) -> bool {
        let Some(owner) = self.signature_owner else {
            return false;
        };
        if binding.owner != owner.def_id.raw() {
            return false;
        }
        let Some(item) = self.package.item(owner) else {
            return false;
        };
        let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) = &item.kind else {
            return false;
        };
        !signature.modifiers.is_comptime
            && signature
                .params
                .iter()
                .any(|parameter| local(parameter.hir_id) == binding && !parameter.ty.is_comptime())
    }

    pub(super) fn signature_comptime_input(&self, binding: n::LocalId) -> Option<(Term, Term)> {
        let owner = self.signature_owner?;
        if !self.active_signatures.contains(&owner) || binding.owner != owner.def_id.raw() {
            return None;
        }
        if let Some(symbol) = self.type_input_bindings.get(&binding) {
            return Some((symbol.clone(), p(P::Type)));
        }
        self.value_input_bindings.get(&binding).cloned()
    }

    /// Collect evidence from inputs whose types do not read the ordinary call
    /// telescope. Their checked expressions are reused by the final call plan.
    pub(super) fn infer_independent_call_inputs(
        &mut self,
        ordinary: &[Parameter],
        result: &Term,
        generics: &[DeclarationId],
        routes: &[arguments::RoutedArgument<'_>],
        offset: usize,
        prefix: &[n::Expr],
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<
        (
            Vec<DeclarationId>,
            Vec<Term>,
            Vec<Option<n::Expr>>,
            BTreeMap<usize, Vec<Option<n::Expr>>>,
        ),
        Error,
    > {
        let mut values = vec![None; ordinary.len()];
        let mut collected = BTreeMap::<usize, Vec<Option<n::Expr>>>::new();
        for (index, value) in prefix.iter().enumerate() {
            values[index] = Some(value.clone());
        }
        let mut indices = Vec::new();
        let mut parameters = Vec::new();
        for (index, parameter) in ordinary.iter().enumerate() {
            if parameter
                .ty
                .substitution_parameters()
                .map_err(|e| error(expr.span, e.to_string()))?
                .is_empty()
            {
                indices.push(index);
                parameters.push(parameter.clone());
            }
        }
        // Even an independent telescope can have comptime inputs demanded by
        // capability contracts. Infer their domains before input preparation.
        // Provisional Type slots let result context constrain clause inputs
        // without claiming that the corresponding default has been evaluated.
        let abstracted = Term::Function {
            parameters: ordinary.to_vec(),
            result: Box::new(result.clone()),
        }
        .abstract_parameters(&self.input_binders(generics))
        .map_err(|e| error(expr.span, e.to_string()))?;
        let Term::Function {
            parameters: abstract_parameters,
            result: abstract_result,
        } = abstracted
        else {
            unreachable!()
        };
        let mut count = generics.len();
        let symbols = ordinary
            .iter()
            .map(|parameter| {
                if parameter.sort == Sort::Type {
                    let index = count;
                    count += 1;
                    Some(Term::Bound {
                        depth: 0,
                        index,
                        sort: Sort::Type,
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let lift = |ty: &Term| {
            ty.substitute_partial(&symbols)
                .and_then(|ty| ty.abstract_parameters(&[]))
        };
        for (parameter, index) in parameters.iter_mut().zip(&indices) {
            parameter.ty = lift(&abstract_parameters[*index].ty)
                .map_err(|e| error(expr.span, e.to_string()))?;
        }
        let (result, expected) = match lift(&abstract_result) {
            Ok(result) => (result, expected),
            Err(nir::TypeError::MissingArgument(_)) => (p(P::Void), None),
            Err(e) => return Err(error(expr.span, e.to_string())),
        };
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(generics)
                .iter()
                .map(|p| p.sort)
                .chain(std::iter::repeat_n(Sort::Type, count - generics.len()))
                .collect(),
            type_parameters: count,
            signature: Term::Function {
                parameters,
                result: Box::new(result),
            },
        };
        let input_domains = generics
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let scope = self.call_inference_domains();
        let discharge_effects = self.active_effect_environment().is_some()
            && !matches!(expected, Some(Term::Computation { .. }));
        let candidate = loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(expr.span, "dependent call inference budget exhausted"))?;
            let actual = indices
                .iter()
                .map(|index| {
                    values[*index]
                        .as_ref()
                        .filter(|v| !is_never(&v.ty))
                        .map(|v| v.ty.clone())
                })
                .collect::<Vec<_>>();
            let mut candidate = scheme
                .infer_declared_sparse_types_mode(
                    generics,
                    &input_domains,
                    &actual,
                    expected,
                    None,
                    &self.context,
                    crate::ptc::Budget::default(),
                    discharge_effects,
                    &scope,
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            for (slot, fields) in &collected {
                let Some(index) = indices.iter().position(|index| index == slot) else {
                    continue;
                };
                if fields.is_empty()
                    || values[*slot]
                        .as_ref()
                        .is_some_and(|value| !is_never(&value.ty))
                {
                    continue;
                }
                let route = routes
                    .iter()
                    .find(|route| route.parameter + offset == *slot)
                    .unwrap();
                let context = route.retained_collection_context(
                    self,
                    fields,
                    &candidate
                        .parameter_input_context(index)
                        .map_err(|e| call_inference_error(expr.span, e))?,
                );
                candidate
                    .constrain_parameter_context(index, &context, crate::ptc::Budget::default())
                    .map_err(|e| call_inference_error(expr.span, e))?;
            }
            candidate
                .refine_type_views_metered(
                    |ty, budget| {
                        self.callable_input_view_metered(ty, budget, expr.span)
                            .map_err(|e| e.message)
                    },
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            let mut progress = false;
            for allow_defaults in [false, true] {
                if allow_defaults && (progress || !self.numeric_defaults) {
                    break;
                }
                for route in routes {
                    let slot = route.parameter + offset;
                    if values[slot].is_some() {
                        continue;
                    }
                    let Some(index) = indices.iter().position(|index| *index == slot) else {
                        continue;
                    };
                    let context = candidate
                        .parameter_type(index)
                        .map_err(|e| call_inference_error(expr.span, e))?;
                    let partial = if context.is_none() {
                        candidate
                            .parameter_callable_context(index)
                            .map_err(|e| call_inference_error(expr.span, e))?
                    } else {
                        None
                    };
                    if let Some(ty) = &context {
                        route.require_collection_type(self, ty, expr.span)?;
                    }
                    if context.is_none() && matches!(ordinary[slot].ty, Term::Computation { .. }) {
                        self.preserve_effect_value = true;
                    }
                    let input = route.expression(expr);
                    let input_context = candidate
                        .parameter_input_context(index)
                        .map_err(|e| call_inference_error(expr.span, e))?;
                    if route.has_collection_fields() {
                        let (value, advanced) = self.attempt_collection_fields(
                            expr,
                            route,
                            &input_context,
                            collected.entry(slot).or_default(),
                            ordinary[slot].sort != Sort::Runtime,
                            allow_defaults,
                        )?;
                        values[slot] = value;
                        progress |= advanced;
                        if advanced {
                            break;
                        }
                        continue;
                    }
                    values[slot] =
                        self.with_staged_input(ordinary[slot].sort != Sort::Runtime, |this| {
                            this.argument_attempt_with_input(
                                &input,
                                context.as_ref(),
                                partial.as_ref(),
                                allow_defaults,
                                Some(&input_context),
                            )
                        })?;
                    if values[slot].is_some() {
                        progress = true;
                        // Rebuild contexts before defaulting another literal.
                        if allow_defaults {
                            break;
                        }
                    }
                }
            }
            if !progress {
                break candidate;
            }
        };
        let mut known = Vec::new();
        let mut arguments = Vec::new();
        for (index, parameter) in generics.iter().enumerate() {
            if let Some(value) = candidate
                .type_argument(index)
                .map_err(|e| call_inference_error(expr.span, e))?
            {
                // A proposal is not evidence until its collected relations hold.
                if candidate
                    .engine()
                    .freeze(&crate::InferenceTerm::Stable(value.clone()))
                    .is_ok()
                {
                    known.push(*parameter);
                    arguments.push(value);
                }
            }
        }
        Ok((known, arguments, values, collected))
    }

    pub(super) fn call_inference_domains(&self) -> Vec<(Sort, Option<Term>)> {
        if let Some(domains) = self
            .signature_owner
            .and_then(|owner| self.signature_domains.get(&owner))
        {
            return domains.clone();
        }
        if let Some(scope) = &self.default_input_scope {
            return scope.clone();
        }
        if let Some(definition) = self
            .signature_owner
            .and_then(|owner| self.open_functions.get(&owner))
        {
            return definition.input_domains();
        }
        self.call_inference_scope()
            .into_iter()
            .map(|parameter| (parameter.sort, Some(parameter.ty)))
            .collect()
    }

    pub(super) fn call_inference_scope(&self) -> Vec<Parameter> {
        if let Some(prefix) = self
            .signature_owner
            .and_then(|owner| self.signature_prefixes.get(&owner))
        {
            return prefix.clone();
        }
        if let Some(scope) = &self.default_input_scope {
            return scope
                .iter()
                .map_while(|(sort, ty)| ty.clone().map(|ty| Parameter { sort: *sort, ty }))
                .collect();
        }
        self.signature_owner
            .and_then(|owner| {
                self.signatures
                    .get(&owner)
                    .or_else(|| self.interface_signatures.get(&owner))
            })
            .and_then(|signature| {
                if let Term::Function { parameters, .. } = signature.callable_signature() {
                    Some(parameters.clone())
                } else {
                    None
                }
            })
            .or_else(|| {
                self.signature_owner
                    .and_then(|owner| self.open_functions.get(&owner))
                    .map(|definition| definition.known_prefix())
            })
            .unwrap_or_default()
    }

    pub(super) fn prepare_call_binding(
        &self,
        parameters: &[Parameter],
        inputs: &[Option<Term>],
        span: Span,
    ) -> Result<nir::FunctionCallBinding, Error> {
        let frames = self.call_input_frames();
        let signature = Term::Function {
            parameters: parameters.to_vec(),
            result: Box::new(Term::Primitive(P::Void)),
        };
        nir::FunctionCallBinding::new(&signature, inputs, &frames, |ty| {
            self.callable_input_view(ty, span)
        })
        .map_err(|e| match e {
            nir::CallError::Contract { error, .. } => error,
            other => error(span, other.to_string()),
        })
    }

    pub(super) fn call_input_frames(&self) -> Vec<Vec<Sort>> {
        if let Some(domains) = self
            .signature_owner
            .and_then(|owner| self.signature_domains.get(&owner))
        {
            return if domains.is_empty() {
                vec![]
            } else {
                vec![domains.iter().map(|entry| entry.0).collect()]
            };
        }
        // Scope inspection only needs sorts, not copies of dependent types.
        if let Some(scope) = &self.default_input_scope {
            return if scope.is_empty() {
                vec![]
            } else {
                vec![scope.iter().map(|entry| entry.0).collect()]
            };
        }
        let scope = self
            .signature_owner
            .and_then(|owner| self.signature_prefixes.get(&owner))
            .map(Vec::as_slice)
            .or_else(|| {
                self.signature_owner
                    .and_then(|owner| {
                        self.signatures
                            .get(&owner)
                            .or_else(|| self.interface_signatures.get(&owner))
                    })
                    .and_then(|signature| {
                        if let Term::Function { parameters, .. } = signature.callable_signature() {
                            Some(parameters.as_slice())
                        } else {
                            None
                        }
                    })
            });
        match scope {
            Some(scope) if !scope.is_empty() => vec![scope.iter().map(|p| p.sort).collect()],
            _ => self
                .signature_owner
                .and_then(|owner| self.open_functions.get(&owner))
                .map(|definition| vec![definition.input_sorts()])
                .unwrap_or_default(),
        }
    }

    pub(super) fn instantiate_input_types(
        &self,
        ty: &Term,
        parameters: &[DeclarationId],
        arguments: &[Term],
        span: Span,
    ) -> Result<Term, Error> {
        self.instantiate_input_types_in_scope(
            ty,
            parameters,
            arguments,
            &self.call_input_frames(),
            span,
        )
    }

    pub(super) fn instantiate_input_types_in_scope(
        &self,
        ty: &Term,
        parameters: &[DeclarationId],
        arguments: &[Term],
        frames: &[Vec<Sort>],
        span: Span,
    ) -> Result<Term, Error> {
        self.input_substitution(parameters, arguments, frames)
            .and_then(|substitution| substitution.instantiate(ty))
            .map_err(|e| error(span, e.to_string()))
    }

    pub(super) fn input_substitution<'a>(
        &self,
        parameters: &[DeclarationId],
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
    ) -> Result<nir::DeclarationSubstitution<'a>, nir::TypeError> {
        let domains = parameters
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        nir::DeclarationSubstitution::new(parameters, &domains, arguments, frames)
    }

    pub(super) fn instantiate_declaration_type(
        &self,
        ty: &Term,
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        span: Span,
    ) -> Result<Term, Error> {
        let frames = self.call_input_frames();
        nir::DeclarationSubstitution::new(parameters, domains, arguments, &frames)
            .and_then(|substitution| substitution.instantiate(ty))
            .map_err(|e| error(span, e.to_string()))
    }

    pub(super) fn prepare_call_types(
        &mut self,
        parameters: &[Parameter],
        result: &Term,
        callee: &n::Expr,
        direct: Option<OwnerId>,
        routes: &[arguments::RoutedArgument<'_>],
        offset: usize,
        assigned: &[bool],
        prefix: &[n::Expr],
        expr: &Expr<'_>,
        seed: Vec<Option<n::Expr>>,
    ) -> Result<PreparedInputs, Error> {
        let mut prepared = PreparedInputs::new(parameters.len());
        if !seed.is_empty() {
            prepared.values = seed;
        }
        for (index, value) in prefix.iter().enumerate() {
            prepared.values[index] = Some(value.clone());
        }
        let mut demanded = BTreeSet::new();
        for ty in parameters
            .iter()
            .map(|parameter| &parameter.ty)
            .chain(std::iter::once(result))
        {
            demanded.extend(
                ty.substitution_parameters()
                    .map_err(|e| error(expr.span, e.to_string()))?
                    .keys()
                    .copied(),
            );
        }
        if let Some(requirements) = direct.and_then(|owner| self.capabilities.get(&owner)) {
            for requirement in requirements {
                for ty in
                    std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
                {
                    demanded.extend(
                        ty.substitution_parameters()
                            .map_err(|e| error(expr.span, e.to_string()))?
                            .keys()
                            .copied(),
                    );
                }
            }
        }
        if let Some(requirements) = direct.and_then(|owner| self.requirements.get(&owner)) {
            for requirement in requirements {
                for ty in required::requirement_terms(requirement, expr.span)? {
                    demanded.extend(
                        ty.substitution_parameters()
                            .map_err(|e| error(expr.span, e.to_string()))?
                            .keys()
                            .copied(),
                    );
                }
            }
        }
        // Internal default annotations can have additional Type dependencies.
        for (parameter, assigned) in assigned.iter().enumerate() {
            if *assigned {
                continue;
            }
            let parameter = parameter + offset;
            let owner = direct.ok_or_else(|| {
                error(
                    expr.span,
                    "default computation needs declaration parameter metadata",
                )
            })?;
            let mut value = self.call_default(owner, parameter, callee, expr.span)?;
            value.rewrite_types(&mut |ty| {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(expr.span, "dependent default annotation budget exhausted")
                })?;
                demanded.extend(
                    ty.substitution_parameters()
                        .map_err(|e| error(expr.span, e.to_string()))?
                        .keys()
                        .copied(),
                );
                Ok::<_, Error>(ty.clone())
            })?;
            prepared.defaults[parameter] = Some(value);
        }
        for slot in demanded {
            self.prepare_type_dependency(
                slot,
                parameters,
                callee,
                direct,
                routes,
                offset,
                expr,
                &mut prepared,
            )?;
        }
        Ok(prepared)
    }

    pub(super) fn call_default(
        &mut self,
        owner: OwnerId,
        parameter: usize,
        callee: &n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        self.ensure_parameter_default(owner, parameter, span)?;
        let value = self
            .defaults
            .get(&owner)
            .and_then(|values| values.get(parameter))
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                error(
                    span,
                    "default computation depends on unfinished definition elaboration",
                )
            })?
            .clone();
        let (parameters, arguments) = match &callee.kind {
            K::ComptimeFunctionInstance { subject, .. } => (
                self.type_parameters[&owner].clone(),
                self.comptime_instance_clause_evidence(owner, subject)
                    .ok_or_else(|| error(span, "invalid comptime clause input packet"))?,
            ),
            K::FunctionInstance { arguments, .. } => {
                (self.type_parameters[&owner].clone(), arguments.clone())
            }
            K::TraitMethod {
                interface,
                interface_arguments,
                method_arguments,
                subject,
                ..
            } => {
                let instance = n::InterfaceInstance {
                    declaration: *interface,
                    arguments: interface_arguments.clone(),
                };
                let (mut parameters, mut arguments) =
                    self.interface_substitution(&instance, subject);
                let inherited = parameters.len();
                parameters.extend(self.type_parameters[&owner].iter().skip(inherited).copied());
                arguments.extend(method_arguments.iter().cloned());
                (parameters, arguments)
            }
            _ => return Ok(value),
        };
        // Default annotations live under the callee's ordinary parameter frame.
        // Generic arguments arrive in the caller frame and must cross that
        // boundary before replacing declaration rigids. Later input preparation
        // removes the callee frame once, using its actual/defaulted Type inputs.
        let mut frames = self.call_input_frames();
        for argument in &arguments {
            argument
                .check_scope(&frames)
                .map_err(|e| error(span, e.to_string()))?;
        }
        let Term::Function {
            parameters: ordinary,
            ..
        } = callee.ty.callable_signature()
        else {
            return Err(error(
                span,
                "default computation requires a callable signature",
            ));
        };
        frames.push(ordinary.iter().map(|p| p.sort).collect());
        let arguments = arguments
            .iter()
            .map(|argument| argument.abstract_parameters(&[]))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| error(span, e.to_string()))?;
        let domains = parameters
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let substitution =
            nir::DeclarationSubstitution::new(&parameters, &domains, &arguments, &frames)
                .map_err(|e| error(span, e.to_string()))?;
        let mut value = value;
        value
            .rewrite_types(&mut |ty| substitution.instantiate(ty))
            .map_err(|e| error(span, e.to_string()))?;
        Ok(value)
    }

    pub(super) fn prepare_type_dependency(
        &mut self,
        slot: usize,
        parameters: &[Parameter],
        callee: &n::Expr,
        owner: Option<OwnerId>,
        routes: &[arguments::RoutedArgument<'_>],
        offset: usize,
        source: &Expr<'_>,
        prepared: &mut PreparedInputs,
    ) -> Result<(), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "dependent input preparation budget exhausted"))?;
        if prepared.symbols.get(slot).is_some_and(Option::is_some) {
            return Ok(());
        }
        if prepared.active.len() >= 64 || !prepared.active.insert(slot) {
            return Err(error(
                source.span,
                "cyclic or excessively deep dependent input preparation",
            ));
        }
        let checked = (|| {
            let parameter = parameters
                .get(slot)
                .ok_or_else(|| error(source.span, "invalid dependent input slot"))?;
            if parameter.sort == Sort::Runtime {
                return Err(error(
                    source.span,
                    "dependent comptime input cannot read a runtime argument",
                ));
            }
            for dependency in parameter
                .ty
                .substitution_parameters()
                .map_err(|e| error(source.span, e.to_string()))?
                .keys()
            {
                self.prepare_type_dependency(
                    *dependency,
                    parameters,
                    callee,
                    owner,
                    routes,
                    offset,
                    source,
                    prepared,
                )?;
            }
            let binding = self.prepare_call_binding(parameters, &prepared.symbols, source.span)?;
            let expected = binding
                .substitute(&parameter.ty)
                .map_err(|e| error(source.span, e.to_string()))?;
            let expected = self.callable_input_view(&expected, source.span)?;
            let route = routes.iter().find(|route| route.parameter + offset == slot);
            let mut bindings = Vec::new();
            let mut forwarded_default = None;
            let mut value = if let Some(value) = &prepared.values[slot] {
                value.clone()
            } else if let Some(route) = route {
                route.require_collection_type(self, &expected, source.span)?;
                let value = route.expression(source);
                self.with_staged_input(true, |this| this.expr(&value, Some(&expected)))?
            } else {
                let owner = owner.ok_or_else(|| {
                    error(
                        source.span,
                        "dependent default needs declaration parameter metadata",
                    )
                })?;
                let mut value = match prepared.defaults[slot].take() {
                    Some(value) => value,
                    None => self.call_default(owner, slot, callee, source.span)?,
                };
                let item = self.package.item(owner).unwrap();
                let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
                    return Err(error(
                        source.span,
                        "dependent default needs function parameter metadata",
                    ));
                };
                let ids: Vec<_> = sig
                    .params
                    .iter()
                    .map(|parameter| local(parameter.hir_id))
                    .collect();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let references = nir::preparation::references(&value, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                for binding in references.map_err(|e| error(source.span, e))? {
                    if let Some(dependency) = ids.iter().position(|id| *id == binding) {
                        self.prepare_type_dependency(
                            dependency,
                            parameters,
                            callee,
                            Some(owner),
                            routes,
                            offset,
                            source,
                            prepared,
                        )?;
                        bindings.push((
                            binding,
                            prepared.values[dependency].as_ref().unwrap().ty.clone(),
                            prepared.symbols[dependency].as_ref().unwrap().clone(),
                        ));
                    }
                }
                let binding =
                    self.prepare_call_binding(parameters, &prepared.symbols, source.span)?;
                value
                    .rewrite_types(&mut |ty| binding.substitute(ty))
                    .map_err(|e| error(source.span, e.to_string()))?;
                value.rewrite_types(&mut |ty| self.callable_input_view(ty, source.span))?;
                // A direct parameter default forwards an existing symbolic
                // value; invoking it would require closing the caller frame.
                if let K::Local(binding) = value.kind {
                    if let Some(dependency) = ids.iter().position(|id| *id == binding) {
                        forwarded_default = Some((
                            prepared.values[dependency].as_ref().unwrap().clone(),
                            prepared.symbols[dependency].as_ref().unwrap().clone(),
                        ));
                    }
                }
                value
            };
            if !matches!(value.kind, K::TypeValue(_) | K::Local(_)) {
                let unresolved = owner
                    .and_then(|owner| self.type_parameters.get(&owner))
                    .cloned()
                    .unwrap_or_default();
                let mut waiting = false;
                value.rewrite_types(&mut |ty| {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(
                            source.span,
                            "dependent computation inspection budget exhausted",
                        )
                    })?;
                    waiting |= unresolved.iter().any(|id| ty.mentions_rigid(*id));
                    Ok::<_, Error>(ty.clone())
                })?;
                if waiting {
                    // Opaque heads and other structural Type values can retain
                    // rigids without invoking a computation over unknown inputs.
                    let mut budget = comptime_runtime::Budget {
                        steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                        max_limbs: 4096,
                    };
                    let before = budget.steps;
                    let structural = self.known_normal_input(&value, 0, &mut budget).is_some();
                    self.required_steps -= (before - budget.steps) as u64;
                    if !structural {
                        return Err(context_error(
                            source.span,
                            "dependent Type computation awaits inferred type arguments",
                        ));
                    }
                }
            }
            let (value, symbol) = if let Some(forwarded) = forwarded_default {
                forwarded
            } else {
                match &value.kind {
                    K::TypeValue(ty) => (value.clone(), ty.clone()),
                    K::Local(binding)
                        if route.is_some() && self.type_input_bindings.contains_key(binding) =>
                    {
                        (value.clone(), self.type_input_bindings[binding].clone())
                    }
                    K::Local(binding)
                        if route.is_some() && self.value_input_bindings.contains_key(binding) =>
                    {
                        (value.clone(), self.value_input_bindings[binding].0.clone())
                    }
                    K::Local(binding) => {
                        // A prepared container input is supplied by the enclosing
                        // invocation. Its declaration identity is enough to check
                        // the callee telescope and container premises now; retain
                        // the Local expression so execution receives the actual
                        // value rather than freezing this symbolic identity.
                        let captured = self
                            .alias_value_bindings
                            .iter()
                            .find_map(|(parameter, id)| (id == binding).then_some(*parameter));
                        if let Some(declaration) = captured {
                            (
                                value,
                                Term::Rigid {
                                    declaration,
                                    sort: Sort::Type,
                                },
                            )
                        } else {
                            match self.spread_input_symbol(*binding, &value.ty, source)? {
                                Some(symbol) => (value, symbol),
                                None => self.freeze_dependency_input(source, value, bindings)?,
                            }
                        }
                    }
                    _ => self.freeze_dependency_input(source, value, bindings)?,
                }
            };
            prepared.values[slot] = Some(value);
            prepared.symbols[slot] = Some(symbol);
            Ok(())
        })();
        prepared.active.remove(&slot);
        checked
    }
}

impl Checker<'_, '_> {
    pub(super) fn check_ordinary_type_requirements(
        &self,
        owner: OwnerId,
        arguments: &[Term],
        inputs: &[Option<Term>],
        span: Span,
    ) -> Result<(), Error> {
        let Some(requirements) = self.requirements.get(&owner) else {
            return Ok(());
        };
        let requirements = requirements
            .iter()
            .filter_map(|r| match required::requirement_terms(r, span) {
                Ok(terms)
                    if terms.iter().any(|ty| {
                        ty.substitution_parameters()
                            .is_ok_and(|slots| !slots.is_empty())
                    }) =>
                {
                    Some(Ok(r))
                }
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if requirements.is_empty() {
            return Ok(());
        }
        let signature = self
            .signatures
            .get(&owner)
            .or_else(|| self.interface_signatures.get(&owner))
            .ok_or_else(|| error(span, "ordinary type requirement needs a function signature"))?;
        let mut frames = self.call_input_frames();
        let declarations = self
            .declaration_binders(owner)
            .iter()
            .map(|parameter| parameter.declaration)
            .collect::<Vec<_>>();
        let signature = self.instantiate_input_types_in_scope(
            signature,
            &declarations,
            arguments,
            &frames,
            span,
        )?;
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            return Err(error(span, "expected a function signature"));
        };
        let binding = self.prepare_call_binding(parameters, inputs, span)?;
        frames.push(parameters.iter().map(|p| p.sort).collect());

        let arguments = arguments
            .iter()
            .map(|ty| ty.abstract_parameters(&[]))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| error(span, e.to_string()))?;
        let substitution = self
            .input_substitution(&declarations, &arguments, &frames)
            .map_err(|e| error(span, e.to_string()))?;
        for requirement in requirements {
            let requirement = substitution
                .instantiate_predicate(requirement)
                .map_err(|e| error(span, e.to_string()))?;
            let requirement = required::map_requirement(
                &requirement,
                &mut |ty| {
                    let ty = binding
                        .substitute(ty)
                        .map_err(|e| error(span, e.to_string()))?;
                    self.callable_input_view(&ty, span)
                },
                span,
                0,
            )?;
            let mut outcome = self.context.prove(
                crate::ptc::Formula::from_predicate(&requirement),
                crate::ptc::Budget::default(),
            );
            if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
                let assumptions = self.capability_assumptions();
                outcome = crate::capabilities::World {
                    index: &self.implementation_index,
                    implementations: &self.checked_implementations,
                    traits: &self.checked_traits,
                    assumptions: &assumptions,
                    context: &self.context,
                }
                .check_predicate(&requirement, crate::ptc::Budget::default())
                .map_err(|e| error(span, e))?;
            }
            if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
                return Err(error(
                    span,
                    format!(
                        "ordinary input type requirement is not established: {requirement}: {outcome}"
                    ),
                ));
            }
        }
        Ok(())
    }
}
