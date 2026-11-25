//! Prepare the compile-time dependencies of ordinary call input types.
//! Checking order does not change the source-ordered runtime argument plan.
use super::*;

mod default_outputs;
mod default_writes;

pub(super) use default_outputs::DefaultOutputs;

pub(super) mod tasks;

/// Borrow the selected function telescope, including its result under the
/// same binders. Callable wrappers remain the responsibility of call delivery.
#[derive(Clone, Copy)]
pub(super) struct CallSignature<'a> {
    pub(super) parameters: &'a [Parameter],
    pub(super) result: &'a Term,
}

impl<'a> CallSignature<'a> {
    pub(super) fn of(callable: &'a Term) -> Option<Self> {
        let term = callable.callable_signature();
        let Term::Function { parameters, result } = term else {
            return None;
        };
        Some(Self { parameters, result })
    }

    /// Inputs that must supply a value for a declared domain or obligation.
    pub(super) fn demanded_inputs(
        &self,
        capabilities: &[n::TraitRequirement],
        requirements: &[n::TypePredicate],
        steps: &mut u64,
        span: Span,
    ) -> Result<BTreeSet<usize>, Error> {
        let mut demanded = BTreeSet::new();
        let mut observe = |ty: &Term| -> Result<(), Error> {
            demanded.extend(input_observation::substitution_term(ty, steps, span)?.into_keys());
            Ok(())
        };
        for parameter in self.parameters {
            observe(&parameter.ty)?;
        }
        observe(self.result)?;
        for requirement in capabilities {
            observe(&requirement.subject)?;
            for ty in &requirement.interface_arguments {
                observe(ty)?;
            }
        }
        for requirement in requirements {
            for ty in required::requirement_terms(requirement, span)? {
                observe(ty)?;
            }
        }
        Ok(demanded)
    }
}

#[derive(Default)]
pub(super) struct InputProducers {
    pub values: Vec<Option<n::Expr>>,
    pub defaults: Vec<Option<n::Expr>>,
}

impl InputProducers {
    pub fn new(count: usize) -> Self {
        Self {
            values: vec![None; count],
            defaults: vec![None; count],
        }
    }
}

/// The source and selected callee of one call. Input continuations refer to
/// slot numbers in these routes, independent of the Rust frame that chose them.
pub(super) struct CallSite<'h> {
    pub(super) callee: n::Expr,
    pub(super) owner: Option<OwnerId>,
    pub(super) call_use: CallUse,
    pub(super) routes: Vec<arguments::RoutedArgument<'h>>,
    pub(super) source: Expr<'h>,
}

impl std::fmt::Display for CallSite<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(call-site (source {} {}) (use {}) (callee {}) (routes",
            self.source.hir_id,
            source(self.source.span),
            match self.call_use {
                CallUse::Invocation => "invocation",
                CallUse::DefinitionBehavior => "definition-behavior",
            },
            self.callee
        )?;
        for route in &self.routes {
            write!(f, " (slot {}", route.parameter)?;
            for (position, input) in route.source_inputs() {
                write!(f, " (input {position} {})", input.hir_id)?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}

pub(super) struct PreparedInputs {
    pub signature: Term,
    pub producers: InputProducers,
    pub binding: nir::FunctionCallBinding,
}

impl Checker<'_, '_> {
    pub(super) fn with_call_type_fuel<T>(
        &mut self,
        inspect: impl FnOnce(&Self, &mut usize) -> T,
    ) -> T {
        let granted = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = granted;
        let result = inspect(self, &mut fuel);
        self.required_steps -= (granted - fuel) as u64;
        result
    }

    /// Move annotations out of the callee's ordinary parameter scope once.
    /// Captured values supplied separately are already in the caller's scope.
    pub(super) fn specialize_default_annotations(
        &mut self,
        expression: &mut n::Expr,
        binding: &nir::FunctionCallBinding,
        span: Span,
    ) -> Result<(), Error> {
        nir::copy_budget::charge_type_rewrite(expression, &mut self.required_steps)
            .map_err(|limit| input_observation::structural_limit(limit, span))?;
        self.with_call_type_fuel(|_, fuel| {
            expression.rewrite_types(&mut |ty| binding.substitute_bounded(ty, fuel))
        })
        .map_err(|failure| error(span, failure.to_string()))?;
        expression.rewrite_types(&mut |term| self.callable_argument_term_view(term, span))
    }

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
        let (ItemKind::Fn(signature, _)
        | ItemKind::Case(signature, _)
        | ItemKind::FnDeclaration(signature)) = &item.kind
        else {
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
        // Contract elaboration follows signature publication, so its staged
        // inputs remain available after the active-signature guard is removed.
        if (!self.active_signatures.contains(&owner) && !self.checking_contract)
            || binding.owner != owner.def_id.raw()
        {
            return None;
        }
        if let Some(symbol) = self.input_facts.type_identity(&binding) {
            return Some((symbol.clone(), p(P::Type)));
        }
        self.input_facts.value_identity(&binding).cloned()
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
            if self
                .term_substitution_inputs(&parameter.ty, expr.span)?
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
                    |ty, budget| self.callable_input_view_metered(ty, budget),
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

    pub(super) fn prepare_call_binding(
        &self,
        signature: &Term,
        inputs: &[Option<Term>],
        span: Span,
        fuel: &mut usize,
    ) -> Result<nir::FunctionCallBinding, Error> {
        nir::FunctionCallBinding::new_metered(
            signature,
            inputs,
            self.call_input_frames(),
            fuel,
            |ty, fuel| self.callable_input_view_with_fuel(ty, fuel, span),
        )
        .map_err(|e| match e {
            nir::CallError::Contract { error, .. } => error,
            other => error(span, other.to_string()),
        })
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
        signature: Term,
        site: &CallSite<'_>,
        offset: usize,
        assigned: &[bool],
        seed: Vec<Option<n::Expr>>,
    ) -> Result<PreparedInputs, Error> {
        let prepared = InputProducers {
            defaults: vec![None; seed.len()],
            values: seed,
        };
        let frame = tasks::Frame::new(self, signature, site, assigned, offset, prepared)?;
        self.check_call_dependencies_region(frame, site)
    }

    pub(super) fn call_input_demands(
        &mut self,
        signature: CallSignature<'_>,
        site: &CallSite<'_>,
        assigned: &[bool],
        offset: usize,
        expr: &Expr<'_>,
        defaults: &mut [Option<n::Expr>],
    ) -> Result<BTreeSet<usize>, Error> {
        let direct = site.owner;
        let mut demanded = signature.demanded_inputs(
            direct
                .and_then(|owner| self.capabilities.get(&owner))
                .map(Vec::as_slice)
                .unwrap_or_default(),
            direct
                .and_then(|owner| self.requirements.get(&owner))
                .map(Vec::as_slice)
                .unwrap_or_default(),
            &mut self.required_steps,
            expr.span,
        )?;
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
            let is_non_optional_implicit = self
                .package
                .item(owner)
                .and_then(|item| match &item.kind {
                    ItemKind::Fn(sig, _)
                    | ItemKind::Case(sig, _)
                    | ItemKind::FnDeclaration(sig) => sig.params.get(parameter),
                    _ => None,
                })
                .is_some_and(|param| {
                    param.ty.is_implicit()
                        && !param.ty.is_quote()
                        && !matches!(param.ty.kind, TyParamKind::Optional(..))
                });
            if is_non_optional_implicit {
                continue;
            }
            let value = self.call_default(site, owner, parameter, expr.span)?;
            demanded.extend(
                self.checked_substitution_inputs(&value, expr.span)?
                    .into_keys(),
            );
            defaults[parameter] = Some(value);
        }
        Ok(demanded)
    }

    pub(super) fn call_default(
        &mut self,
        site: &CallSite<'_>,
        owner: OwnerId,
        parameter: usize,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let callee = &site.callee;
        if site.call_use == CallUse::DefinitionBehavior
            && self.open_functions.contains_key(&owner)
            && !callee.ty.is_comptime_callable()
            && CallSignature::of(&callee.ty)
                .is_some_and(|signature| signature.parameters[parameter].sort == Sort::Runtime)
        {
            let behavior = self.parameter_default_behavior(owner, parameter, span)?;
            return self.instantiate_call_default(behavior, owner, callee, span);
        }
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
        self.instantiate_call_default(value, owner, callee, span)
    }

    fn instantiate_call_default(
        &mut self,
        value: n::Expr,
        owner: OwnerId,
        callee: &n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let (parameters, arguments) = match &callee.kind {
            K::PlannedFunctionInstance { .. }
            | K::ComptimeFunctionInstance { .. }
            | K::FunctionInstance { .. } => (
                self.type_parameters[&owner].clone(),
                self.method_instantiation_inputs(callee, span)?
                    .ok_or_else(|| error(span, "invalid callable clause input packet"))?
                    .1,
            ),
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
}

impl Checker<'_, '_> {
    pub(super) fn check_ordinary_type_requirements(
        &mut self,
        owner: OwnerId,
        arguments: &[Term],
        inputs: &[Option<Term>],
        span: Span,
    ) -> Result<(), Error> {
        let Some(requirements) = self.requirements.get(&owner) else {
            return Ok(());
        };
        let mut dependent = Vec::new();
        for (index, requirement) in requirements.iter().enumerate() {
            for ty in required::requirement_terms(requirement, span)? {
                if !input_observation::substitution_term(ty, &mut self.required_steps, span)?
                    .is_empty()
                {
                    dependent.push(index);
                    break;
                }
            }
        }
        if dependent.is_empty() {
            return Ok(());
        }
        self.with_call_type_fuel(|checker, fuel| {
            checker
                .prove_ordinary_type_requirements(owner, arguments, inputs, &dependent, span, fuel)
        })
    }

    fn prove_ordinary_type_requirements(
        &self,
        owner: OwnerId,
        arguments: &[Term],
        inputs: &[Option<Term>],
        dependent: &[usize],
        span: Span,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        let signature = self
            .declared_signature(owner)
            .ok_or_else(|| error(span, "ordinary type requirement needs a function signature"))?;
        let mut frames = self.call_input_frames();
        let declarations = self
            .declaration_binders(owner)
            .iter()
            .map(|parameter| parameter.declaration)
            .collect::<Vec<_>>();
        let signature = self.instantiate_input_types_in_scope(
            &signature,
            &declarations,
            arguments,
            &frames,
            span,
        )?;
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            return Err(error(span, "expected a function signature"));
        };
        let binding = self.prepare_call_binding(&signature, inputs, span, fuel)?;
        frames.push(parameters.iter().map(|p| p.sort).collect());

        let arguments = arguments
            .iter()
            .map(|ty| ty.abstract_parameters(&[]))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| error(span, e.to_string()))?;
        let substitution = self
            .input_substitution(&declarations, &arguments, &frames)
            .map_err(|e| error(span, e.to_string()))?;
        for &index in dependent {
            let requirement = substitution
                .instantiate_predicate_bounded(&self.requirements[&owner][index], fuel)
                .map_err(|e| error(span, e.to_string()))?;
            let requirement = required::map_requirement(
                &requirement,
                &mut |ty| {
                    let ty = binding
                        .substitute_bounded(ty, fuel)
                        .map_err(|e| error(span, e.to_string()))?;
                    self.callable_input_view_with_fuel(&ty, fuel, span)
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
