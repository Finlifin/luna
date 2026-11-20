//! Prepare the compile-time dependencies of ordinary call input types.
//! Checking order does not change the source-ordered runtime argument plan.
use super::*;

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
    pub(super) routes: Vec<arguments::RoutedArgument<'h>>,
    pub(super) source: Expr<'h>,
}

impl std::fmt::Display for CallSite<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(call-site (source {} {}) (callee {}) (routes",
            self.source.hir_id,
            source(self.source.span),
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
    ) -> Result<nir::FunctionCallBinding, Error> {
        nir::FunctionCallBinding::new(signature, inputs, &self.call_input_frames(), |ty| {
            self.callable_input_view(ty, span)
        })
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
        callee: &n::Expr,
        direct: Option<OwnerId>,
        assigned: &[bool],
        offset: usize,
        expr: &Expr<'_>,
        defaults: &mut [Option<n::Expr>],
    ) -> Result<BTreeSet<usize>, Error> {
        let mut demanded = BTreeSet::new();
        for ty in signature
            .parameters
            .iter()
            .map(|parameter| &parameter.ty)
            .chain(std::iter::once(signature.result))
        {
            demanded.extend(self.term_substitution_inputs(ty, expr.span)?.into_keys());
        }
        if let Some(requirements) = direct.and_then(|owner| self.capabilities.get(&owner)) {
            for requirement in requirements {
                for ty in
                    std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
                {
                    demanded.extend(
                        input_observation::substitution_term(
                            ty,
                            &mut self.required_steps,
                            expr.span,
                        )?
                        .into_keys(),
                    );
                }
            }
        }
        if let Some(requirements) = direct.and_then(|owner| self.requirements.get(&owner)) {
            for requirement in requirements {
                for ty in required::requirement_terms(requirement, expr.span)? {
                    demanded.extend(
                        input_observation::substitution_term(
                            ty,
                            &mut self.required_steps,
                            expr.span,
                        )?
                        .into_keys(),
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
            let value = self.call_default(owner, parameter, callee, expr.span)?;
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
        for requirement in requirements {
            for ty in required::requirement_terms(requirement, span)? {
                if !input_observation::substitution_term(ty, &mut self.required_steps, span)?
                    .is_empty()
                {
                    dependent.push(requirement);
                    break;
                }
            }
        }
        let requirements = dependent;
        if requirements.is_empty() {
            return Ok(());
        }
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
        let binding = self.prepare_call_binding(&signature, inputs, span)?;
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
