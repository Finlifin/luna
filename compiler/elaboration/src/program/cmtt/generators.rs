//! Typed, first-order comptime code calls. Definition checking uses interface
//! evidence without inventing a runnable program; invocation supplies actual
//! code witnesses before anything can enter the runtime package.
use super::*;
mod matches;
mod places;
mod spreads;
mod statements;
use statements::Control;

#[derive(Clone)]
enum Input {
    Data(Term),
    Code(Interface),
}
#[derive(Clone)]
pub(in crate::program) struct Generator {
    types: Vec<(DeclarationId, Term)>,
    inputs: Vec<(n::LocalId, Input)>,
    output: Interface,
    defaults: Vec<(usize, hir::HirId)>,
    variadics: Vec<usize>,
    body: hir::BodyId,
    checked: bool,
}
enum Argument {
    Data(Option<n::Expr>),
    Code(Arc<ExpressionCode>),
}
type GeneratorTypePlan = (
    Vec<DeclarationId>,
    Vec<Term>,
    Vec<(usize, Vec<DeclarationId>)>,
);

impl fmt::Display for Generator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(code-generator (types")?;
        for (id, ty) in &self.types {
            write!(f, " (type {id} {ty})")?;
        }
        f.write_str(") (inputs")?;
        for (binding, input) in &self.inputs {
            match input {
                Input::Data(ty) => write!(f, " (data {binding} {ty})")?,
                Input::Code(interface) => write!(f, " (code {binding} {interface})")?,
            }
        }
        f.write_str(") (variadics")?;
        for slot in &self.variadics {
            write!(f, " (parameter {slot})")?;
        }
        f.write_str(") (defaults")?;
        for (slot, expression) in &self.defaults {
            write!(
                f,
                " (default (parameter {slot}) (expression {}))",
                local(*expression)
            )?;
        }
        write!(
            f,
            ") (output {}) (body {}) (checked {}))",
            self.output,
            local(self.body.hir_id),
            self.checked
        )
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn is_code_generator(&self, owner: OwnerId) -> bool {
        self.package.item(owner).is_some_and(|item| {
            matches!(&item.kind,
            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                if sig.return_ty.is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..))))
        })
    }

    pub(in crate::program) fn ensure_code_generator(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        if self.code_generators.contains_key(&owner) {
            return Ok(());
        }
        if !self.active_signatures.insert(owner) {
            return Err(error(span, "cyclic code generator signature"));
        }
        let saved = std::mem::take(&mut self.locals);
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        self.context = self.nominal_context();
        let result = (|| {
            let item = self
                .package
                .item(owner)
                .ok_or_else(|| error(span, "missing code generator declaration"))?;
            let ItemKind::Fn(sig, body) = &item.kind else {
                return Err(error(span, "code generator requires a body"));
            };
            if !sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head {
                return Err(error(
                    span,
                    "an expression-code result requires an explicit comptime fn",
                ));
            }
            if !sig.modifiers.is_pure || self.self_containers.contains_key(&owner) {
                return Err(error(
                    span,
                    "impure or method code generators are not yet implemented",
                ));
            }
            for parameter in sig.clause_params {
                match parameter.kind {
                    hir::clause::ClauseParamKind::Type(_)
                    | hir::clause::ClauseParamKind::TraitBound(..) => {}
                    hir::clause::ClauseParamKind::Optional(_, ty, _)
                        if self.type_expr(ty)? == p(P::Type) => {}
                    _ => {
                        return Err(error(
                            parameter.span,
                            "code generator clauses require Type parameters",
                        ));
                    }
                }
            }
            self.prepare_parameter_capabilities(owner, sig.clause_params)?;
            self.ensure_contract(owner, span)?;
            if !self.program_requirements[&owner].is_empty()
                || !self.program_postconditions[&owner].is_empty()
            {
                return Err(error(
                    span,
                    "program contracts on code generators are not yet implemented",
                ));
            }
            self.check_type_defaults(owner)?;
            self.context = self.definition_context(owner);
            let mut inputs = Vec::new();
            for parameter in sig.params {
                let annotation = match parameter.ty.kind {
                    TyParamKind::Positional(annotation)
                    | TyParamKind::Optional(_, annotation, _)
                    | TyParamKind::Varadic(_, annotation) => annotation,
                    _ => {
                        return Err(error(
                            parameter.span,
                            "code generator receiver parameters are not yet implemented",
                        ));
                    }
                };
                if parameter.ty.flags != 0 {
                    return Err(error(
                        parameter.span,
                        "code generator parameter protocols are not yet implemented",
                    ));
                }
                let input = if matches!(annotation.kind, H::ContextualExprType(..)) {
                    if matches!(parameter.ty.kind, TyParamKind::Varadic(..)) {
                        return Err(error(
                            parameter.span,
                            "code collections require code-container evidence not yet implemented",
                        ));
                    }
                    Input::Code(self.code_interface(annotation)?)
                } else {
                    let ty = self.type_expr(annotation)?;
                    if !nir::normal_forms::supported_input_type_with_layouts(
                        &ty,
                        &self.type_parameters[&owner],
                        |head| Some(head) == self.protocols.tuple,
                        |declaration| {
                            self.structs.contains_key(&declaration)
                                || self.enums.contains_key(&declaration)
                                || self.newtypes.contains_key(&declaration)
                                || self.context.is_nominal_constructor(declaration)
                        },
                    ) {
                        return Err(error(
                            parameter.span,
                            "code generator data parameter requires a supported immutable value domain",
                        ));
                    }
                    Input::Data(ty)
                };
                inputs.push((local(parameter.hir_id), input));
            }
            let output = self.code_interface(sig.return_ty.unwrap())?;
            Ok(Generator {
                types: self.type_parameters[&owner]
                    .iter()
                    .map(|id| {
                        (
                            *id,
                            Term::Rigid {
                                declaration: *id,
                                sort: Sort::Type,
                            },
                        )
                    })
                    .collect(),
                inputs,
                output,
                variadics: sig
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, param)| {
                        matches!(param.ty.kind, TyParamKind::Varadic(..)).then_some(slot)
                    })
                    .collect(),
                defaults: sig
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, param)| match param.ty.kind {
                        TyParamKind::Optional(_, _, value) => Some((slot, value.hir_id)),
                        _ => None,
                    })
                    .collect(),
                body: *body,
                checked: false,
            })
        })();
        self.locals = saved;
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.active_signatures.remove(&owner);
        self.code_generators.insert(owner, result?);
        Ok(())
    }

    fn generator_frame<R>(
        &mut self,
        owner: OwnerId,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let outer = (
            self.context.clone(),
            std::mem::take(&mut self.type_input_bindings),
            std::mem::take(&mut self.value_input_bindings),
            self.default_input_scope.take(),
            std::mem::take(&mut self.specialization_inputs),
            self.effect_environment.take(),
            self.partial_return_context.take(),
            self.return_observations.take(),
        );
        let aliases = std::mem::take(&mut self.alias_substitution);
        self.context = self.definition_context(owner);
        let saved = (
            std::mem::take(&mut self.locals),
            std::mem::take(&mut self.code_values),
            std::mem::take(&mut self.generator_data),
            std::mem::take(&mut self.abstract_code_dependencies),
            self.signature_owner.replace(owner),
            self.in_code_body,
            std::mem::take(&mut self.captures),
            self.result.take(),
            std::mem::take(&mut self.loops),
            self.effect_resume_result.take(),
            self.in_required_comptime,
            self.in_staged_input,
            std::mem::take(&mut self.immutable),
        );
        self.in_code_body = false;
        self.in_required_comptime = false;
        self.in_staged_input = false;
        let result = check(self);
        (
            self.locals,
            self.code_values,
            self.generator_data,
            self.abstract_code_dependencies,
            self.signature_owner,
            self.in_code_body,
            self.captures,
            self.result,
            self.loops,
            self.effect_resume_result,
            self.in_required_comptime,
            self.in_staged_input,
            self.immutable,
        ) = saved;
        (
            self.context,
            self.type_input_bindings,
            self.value_input_bindings,
            self.default_input_scope,
            self.specialization_inputs,
            self.effect_environment,
            self.partial_return_context,
            self.return_observations,
        ) = outer;
        self.alias_substitution = aliases;
        result
    }

    pub(in crate::program) fn check_code_generator(&mut self, owner: OwnerId) -> Result<(), Error> {
        let definition = self.copy_code_generator(owner, self.package.item(owner).unwrap().span)?;
        if definition.checked {
            return Ok(());
        }
        // Even an unused identity generator must not admit comptime-only
        // storage through a composite runtime context or result type.
        let metadata = n::Package {
            structs: self.structs.values().cloned().collect(),
            enums: self.enums.values().cloned().collect(),
            normal_forms: self.normal_form_metadata(),
            ..Default::default()
        };
        for interface in definition
            .inputs
            .iter()
            .filter_map(|(_, input)| match input {
                Input::Code(interface) => Some(interface),
                Input::Data(_) => None,
            })
            .chain(std::iter::once(&definition.output))
        {
            for ty in interface
                .context
                .values()
                .chain(std::iter::once(&interface.result))
            {
                nir::phases::validate_runtime_storage(ty, &metadata, &[]).map_err(|message| {
                    error(
                        self.package.item(owner).unwrap().span,
                        format!("quoted runtime interface: {message}"),
                    )
                })?;
            }
        }
        let checking = std::mem::replace(&mut self.checking_code_generator, true);
        let arguments = definition
            .inputs
            .iter()
            .map(|(_, input)| match input {
                Input::Data(_) => Some(Argument::Data(None)),
                Input::Code(interface) => Some(Argument::Code(Arc::new(ExpressionCode {
                    body: None,
                    result: interface.result.clone(),
                    context: interface.context.clone(),
                    ports: interface.ports.clone(),
                    defaults: BTreeMap::new(),
                }))),
            })
            .collect();
        let result = self.generator_frame(owner, |this| {
            this.run_code_generator(&definition, arguments)
        });
        self.checking_code_generator = checking;
        result?;
        self.code_generators.get_mut(&owner).unwrap().checked = true;
        Ok(())
    }

    fn charge_code_generator(
        definition: &Generator,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
        let result = (|| {
            for (_, ty) in &definition.types {
                nir::type_computations::charge_term(ty, &mut remaining)?;
            }
            for (_, input) in &definition.inputs {
                match input {
                    Input::Data(ty) => nir::type_computations::charge_term(ty, &mut remaining)?,
                    Input::Code(interface) => {
                        for ty in interface
                            .context
                            .values()
                            .chain(std::iter::once(&interface.result))
                        {
                            nir::type_computations::charge_term(ty, &mut remaining)?;
                        }
                    }
                }
            }
            for ty in definition
                .output
                .context
                .values()
                .chain(std::iter::once(&definition.output.result))
            {
                nir::type_computations::charge_term(ty, &mut remaining)?;
            }
            Ok::<_, nir::type_computations::Error>(())
        })();
        *fuel = remaining as u64;
        result.map_err(|e| error(span, e.to_string()))
    }

    fn copy_code_generator(&mut self, owner: OwnerId, span: Span) -> Result<Generator, Error> {
        let definition = &self.code_generators[&owner];
        Self::charge_code_generator(definition, &mut self.required_steps, span)?;
        Ok(definition.clone())
    }

    fn instantiate_code_generator(
        &mut self,
        definition: &Generator,
        types: &[Term],
        span: Span,
    ) -> Result<Generator, Error> {
        Self::charge_code_generator(definition, &mut self.required_steps, span)?;
        for ty in types {
            self.copy_alias_type(ty, span)?;
        }
        let mut definition = definition.clone();
        let parameters = definition
            .types
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut substitute = |ty: &Term| -> Result<Term, Error> {
            let ty = self.copy_alias_type(ty, span)?;
            ty.instantiate_types(&parameters, types)
                .map_err(|e| error(span, e.to_string()))
        };
        for (_, input) in &mut definition.inputs {
            match input {
                Input::Data(ty) => *ty = substitute(ty)?,
                Input::Code(interface) => {
                    interface.result = substitute(&interface.result)?;
                    for ty in interface.context.values_mut() {
                        *ty = substitute(ty)?;
                    }
                }
            }
        }
        definition.output.result = substitute(&definition.output.result)?;
        for ty in definition.output.context.values_mut() {
            *ty = substitute(ty)?;
        }
        definition.types = parameters.into_iter().zip(types.iter().cloned()).collect();
        Ok(definition)
    }

    fn generator_owner(&self, callee: &Expr<'_>) -> Option<OwnerId> {
        let callee = if let H::NFApplication(head, _) = callee.kind {
            head
        } else {
            callee
        };
        let H::ResolvedDef { def_id, .. } = callee.kind else {
            return None;
        };
        self.package
            .definition_owner(def_id)
            .filter(|owner| self.is_code_generator(*owner))
    }

    fn generator_inference_view(
        &mut self,
        template: &Generator,
        parameters: &[DeclarationId],
        type_plan: &[Term],
        candidate: &crate::calls::CallCandidate,
        span: Span,
    ) -> Result<Generator, Error> {
        let hints = parameters
            .iter()
            .enumerate()
            .map(|(index, id)| {
                candidate
                    .type_argument(index)
                    .map(|value| {
                        value.unwrap_or(Term::Rigid {
                            declaration: *id,
                            sort: Sort::Type,
                        })
                    })
                    .map_err(|e| call_inference_error(span, e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let types = type_plan
            .iter()
            .map(|ty| {
                ty.instantiate_types(parameters, &hints)
                    .map_err(|e| error(span, e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.instantiate_code_generator(template, &types, span)
    }

    pub(super) fn invoke_property_generator(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        if !arguments.iter().any(|arg| matches!(arg, Arg::Named(..))) {
            return self.invoke_code_generator(site, callee, arguments, expected);
        }
        let owner = self
            .generator_owner(callee)
            .ok_or_else(|| error(callee.span, "property code call requires a code generator"))?;
        self.ensure_code_generator(owner, site.span)?;
        let template = self.copy_code_generator(owner, site.span)?;
        let type_plan = if let H::NFApplication(_, types) = callee.kind {
            (
                vec![],
                self.type_clause_arguments(owner, types, None, site.span)?,
                vec![],
            )
        } else {
            self.function_type_plan(owner, &[], site.span)?
        };
        let mut definition = self.instantiate_code_generator(&template, &type_plan.1, site.span)?;
        if !type_plan.0.is_empty()
            && let Some(expected) = expected
        {
            let scheme = crate::calls::Scheme {
                type_parameters: type_plan.0.len(),
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(definition.output.result.clone()),
                }
                .abstract_types(&type_plan.0)
                .map_err(|e| error(site.span, e.to_string()))?,
            };
            let candidate = scheme
                .infer_partial_types_in_context(
                    &[],
                    Some(&expected.result),
                    &self.context,
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(site.span, e))?;
            definition = self.generator_inference_view(
                &template,
                &type_plan.0,
                &type_plan.1,
                &candidate,
                site.span,
            )?;
        }
        let ItemKind::Fn(signature, _) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
        let mut collectors = Vec::new();
        for (index, param) in signature.params.iter().enumerate() {
            if !matches!(param.ty.kind, TyParamKind::Varadic(..)) {
                continue;
            }
            let Input::Data(ty) = &definition.inputs[index].1 else {
                continue;
            };
            if !matches!(
                self.structural_type(ty, site.span)?,
                Term::Record(_)
                    | Term::Rigid { .. }
                    | Term::NormalForm { .. }
                    | Term::AssociatedType { .. }
                    | Term::Bound { .. }
                    | Term::Hole { .. }
            ) {
                continue;
            }
            collectors.push(param.name);
        }
        let [collector] = collectors.as_slice() else {
            return Err(error(
                site.span,
                "generator property collection requires one eligible variadic record entrance",
            ));
        };
        let mut fields = Vec::new();
        let mut seen = BTreeSet::new();
        for arg in arguments {
            if let Arg::Named(name, value) = arg {
                if !seen.insert(name.name.to_string()) {
                    return Err(error(name.span, "property supplied more than once"));
                }
                fields.push(hir::expr::FieldExpr {
                    ident: *name,
                    expr: value,
                    span: value.span,
                });
            }
        }
        let packet = Expr {
            kind: H::CollectedProperties(&fields),
            ..site.clone()
        };
        let mut inputs = Vec::new();
        let mut inserted = false;
        for arg in arguments {
            match arg {
                Arg::Named(..) if !inserted => {
                    inputs.push(Arg::Named(*collector, &packet));
                    inserted = true;
                }
                Arg::Named(..) => {}
                other => inputs.push(other.clone()),
            }
        }
        self.invoke_generator_with_plan(site, callee, &inputs, expected, Some(type_plan))
    }

    pub(super) fn invoke_code_generator(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.invoke_generator_with_plan(site, callee, arguments, expected, None)
    }

    fn invoke_generator_with_plan(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
        type_plan: Option<GeneratorTypePlan>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        if arguments
            .iter()
            .any(|argument| matches!(argument, Arg::Expand(_)))
        {
            return self.invoke_spread_generator(site, callee, arguments, expected, type_plan);
        }
        self.invoke_code_generator_inputs(
            site,
            callee,
            arguments,
            expected,
            &mut spreads::SpreadPlan::default(),
            type_plan,
        )
    }

    fn invoke_code_generator_inputs(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
        packets: &mut spreads::SpreadPlan<'_>,
        prepared_types: Option<GeneratorTypePlan>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        let (head, explicit) = match &callee.kind {
            H::NFApplication(head, types) => (*head, Some(*types)),
            _ => (callee, None),
        };
        let owner = self.generator_owner(head).ok_or_else(|| {
            error(
                callee.span,
                "code position requires a typed comptime code generator",
            )
        })?;
        self.ensure_code_generator(owner, site.span)?;
        let template = self.copy_code_generator(owner, site.span)?;
        let ItemKind::Fn(signature, _) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
        let (routes, _) = route_arguments(
            arguments,
            Some(signature.params),
            template.inputs.len(),
            site.span,
        )?;
        let (parameters, type_plan, pending) = if let Some(plan) = prepared_types {
            plan
        } else if let Some(explicit) = explicit {
            (
                vec![],
                self.type_clause_arguments(owner, explicit, None, site.span)?,
                vec![],
            )
        } else {
            self.function_type_plan(owner, &[], site.span)?
        };
        let planned = self.instantiate_code_generator(&template, &type_plan, site.span)?;
        let output_shape = |interface: &Interface| {
            if expected.is_some_and(|expected| !expected.infer_context) {
                Term::Tuple(
                    std::iter::once(interface.result.clone())
                        .chain(interface.context.values().cloned())
                        .collect(),
                )
            } else {
                interface.result.clone()
            }
        };
        let expected_type = expected.map(output_shape);
        // Previously completed argument interfaces are safe inference evidence:
        // reading their types neither runs a generator nor reads runtime storage.
        let mut evidence = vec![None; planned.inputs.len()];
        for route in &routes {
            if route.is_collected() {
                continue;
            }
            let expression = route.expression(site);
            let id = match expression.kind {
                H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => Some(local(id)),
                _ => None,
            };
            if let Some(id) = id {
                let terms: Vec<&Term> = match &planned.inputs[route.parameter].1 {
                    Input::Code(_) => self
                        .code_values
                        .get(&id)
                        .map(|code| {
                            std::iter::once(&code.result)
                                .chain(code.context.values())
                                .collect()
                        })
                        .unwrap_or_default(),
                    Input::Data(_) => self.locals.get(&id).into_iter().collect(),
                };
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let checked = terms
                    .iter()
                    .try_for_each(|ty| nir::type_computations::charge_term(ty, &mut fuel));
                self.required_steps = fuel as u64;
                checked.map_err(|e| error(site.span, e.to_string()))?;
                if !terms.is_empty() {
                    evidence[route.parameter] = Some(match &planned.inputs[route.parameter].1 {
                        Input::Code(_) => Term::Tuple(terms.into_iter().cloned().collect()),
                        Input::Data(_) => terms[0].clone(),
                    });
                }
            }
        }
        let inference_signature = Term::Function {
            parameters: planned
                .inputs
                .iter()
                .map(|(_, input)| Parameter {
                    sort: Sort::Runtime,
                    ty: match input {
                        Input::Data(ty) => ty.clone(),
                        Input::Code(interface) => Term::Tuple(
                            std::iter::once(interface.result.clone())
                                .chain(interface.context.values().cloned())
                                .collect(),
                        ),
                    },
                })
                .collect(),
            result: Box::new(output_shape(&planned.output)),
        };
        let scheme = crate::calls::Scheme {
            type_parameters: parameters.len(),
            signature: inference_signature
                .abstract_types(&parameters)
                .map_err(|e| error(site.span, e.to_string()))?,
        };
        let mut hinted = if parameters.is_empty() {
            None
        } else {
            let candidate = scheme
                .infer_partial_types_in_context(
                    &evidence,
                    expected_type.as_ref(),
                    &self.context,
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(site.span, e))?;
            Some(self.generator_inference_view(
                &template,
                &parameters,
                &type_plan,
                &candidate,
                site.span,
            )?)
        };
        let mut values: Vec<Option<Argument>> = std::iter::repeat_with(|| None)
            .take(template.inputs.len())
            .collect();
        let mut actual = vec![None; template.inputs.len()];
        let mut events = Vec::new();
        let mut collected: Vec<Vec<(Option<n::Expr>, Term)>> = std::iter::repeat_with(Vec::new)
            .take(routes.len())
            .collect();
        let mut collection_types = vec![None; routes.len()];
        let properties = routes
            .iter()
            .map(|route| {
                let expression = route.expression(site);
                if let H::CollectedProperties(fields) = expression.kind {
                    Some(fields)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (route_index, route) in routes.iter().enumerate() {
            if let Some(fields) = properties[route_index] {
                for (element, field) in fields.iter().enumerate() {
                    events.push((field.expr.span.lo().0, route_index, element, field.expr));
                }
                continue;
            }
            if route.is_collected() {
                let Input::Data(ty) =
                    &hinted.as_ref().unwrap_or(&planned).inputs[route.parameter].1
                else {
                    return Err(error(
                        site.span,
                        "code collection requires code-container evidence",
                    ));
                };
                if !parameters.iter().any(|id| ty.mentions_rigid(*id)) {
                    route.require_collection_type(self, ty, site.span)?;
                    let Term::Tuple(fields) = self.structural_type(ty, site.span)? else {
                        unreachable!()
                    };
                    collection_types[route_index] = Some(fields);
                }
            }
            for (element, (_, argument)) in route.source_inputs().enumerate() {
                events.push((argument.span.lo().0, route_index, element, argument));
            }
        }
        for (_, packet) in &packets.empty {
            events.push((packets.source_start(*packet), usize::MAX, *packet, site));
        }
        events.sort_by_key(|(position, _, _, _)| *position);
        for (_, route_index, element, argument) in events {
            if route_index == usize::MAX {
                self.prepare_generator_packet(packets, element, None)?;
                continue;
            }
            let route = &routes[route_index];
            let (_, input) = &hinted.as_ref().unwrap_or(&planned).inputs[route.parameter];
            if route.is_collected() && collection_types[route_index].is_none() {
                if let Input::Data(ty) = input {
                    if !parameters.iter().any(|id| ty.mentions_rigid(*id)) {
                        route.require_collection_type(self, ty, site.span)?;
                        let Term::Tuple(fields) = self.structural_type(ty, site.span)? else {
                            unreachable!()
                        };
                        collection_types[route_index] = Some(fields);
                    }
                }
            }
            let (value, ty) = match input {
                Input::Code(interface) => {
                    if packets.fields.contains_key(&local(argument.hir_id)) {
                        return Err(error(
                            argument.span,
                            "data packet cannot supply code evidence",
                        ));
                    }
                    let known = !parameters
                        .iter()
                        .any(|id| interface.result.mentions_rigid(*id));
                    let mut expected = interface.clone();
                    expected.infer_context = true;
                    let code = self.code_value(argument, known.then_some(&expected), 0)?;
                    let ty = Term::Tuple(
                        std::iter::once(code.result.clone())
                            .chain(code.context.values().cloned())
                            .collect(),
                    );
                    (Argument::Code(code), ty)
                }
                Input::Data(ty) => {
                    let property_context = if let Some(fields) = properties[route_index] {
                        match self.structural_type(ty, site.span)? {
                            Term::Record(types) => types
                                .get(element)
                                .filter(|(name, _)| name == fields[element].ident.name.as_str())
                                .map(|(_, ty)| ty.clone()),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let expected = if properties[route_index].is_some() {
                        property_context.as_ref()
                    } else if route.is_collected() {
                        collection_types[route_index]
                            .as_ref()
                            .and_then(|fields| fields.get(element))
                    } else {
                        (!parameters.iter().any(|id| ty.mentions_rigid(*id))).then_some(ty)
                    };
                    if let Some((packet, element)) =
                        packets.fields.get(&local(argument.hir_id)).copied()
                    {
                        let context = self.generator_packet_context(
                            packets,
                            packet,
                            &routes,
                            hinted.as_ref().unwrap_or(&planned),
                            &parameters,
                            site.span,
                        )?;
                        self.prepare_generator_packet(packets, packet, context.as_ref())?;
                        self.generator_packet_field(
                            packets,
                            packet,
                            element,
                            expected,
                            argument.span,
                        )?
                    } else if self.checking_code_generator {
                        let value = self.expr(argument, expected)?;
                        self.validate_generator_data(&value, argument.span)?;
                        (Argument::Data(None), value.ty)
                    } else {
                        let (value, ty) = self.required_typed_expression(argument, expected)?;
                        (
                            Argument::Data(Some(self.materialize_generator_data(
                                value,
                                ty.clone(),
                                argument.span,
                            )?)),
                            ty,
                        )
                    }
                }
            };
            if route.is_collected() || properties[route_index].is_some() {
                let Argument::Data(value) = value else {
                    unreachable!()
                };
                collected[route_index].push((value, ty));
            } else {
                values[route.parameter] = Some(value);
                actual[route.parameter] = Some(ty.clone());
                evidence[route.parameter] = Some(ty);
                if !parameters.is_empty() {
                    let candidate = scheme
                        .infer_partial_types_in_context(
                            &evidence,
                            expected_type.as_ref(),
                            &self.context,
                            crate::ptc::Budget::default(),
                        )
                        .map_err(|e| call_inference_error(site.span, e))?;
                    hinted = Some(self.generator_inference_view(
                        &template,
                        &parameters,
                        &type_plan,
                        &candidate,
                        site.span,
                    )?);
                }
            }
        }
        for (route_index, route) in routes.iter().enumerate() {
            if !route.is_collected() && properties[route_index].is_none() {
                continue;
            }
            let elements = std::mem::take(&mut collected[route_index]);
            let ty = if let Some(fields) = properties[route_index] {
                Term::Record(
                    fields
                        .iter()
                        .zip(&elements)
                        .map(|(field, (_, ty))| (field.ident.name.to_string(), ty.clone()))
                        .collect(),
                )
            } else {
                self.tuple_value_type(
                    elements.iter().map(|(_, ty)| ty.clone()).collect(),
                    site.span,
                )?
            };
            let value = if self.checking_code_generator {
                None
            } else {
                let fields = elements
                    .into_iter()
                    .map(|(value, _)| value.unwrap())
                    .collect();
                Some(n::Expr {
                    ty: ty.clone(),
                    source: source(site.span),
                    kind: if properties[route_index].is_some() {
                        K::AnonymousRecord { fields }
                    } else {
                        K::Tuple(fields)
                    },
                })
            };
            values[route.parameter] = Some(Argument::Data(value));
            actual[route.parameter] = Some(ty);
        }
        let types = if parameters.is_empty() {
            type_plan
        } else {
            // This tuple is an internal inference product, never a Flurry Tuple
            // constructor or an erased code value. Context evidence stays in values.
            let mut candidate = scheme
                .infer_partial_types_in_context(
                    &actual,
                    expected_type.as_ref(),
                    &self.context,
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(site.span, e))?;
            if !self.settle_function_defaults(
                owner,
                &parameters,
                &type_plan,
                &pending,
                &mut BTreeMap::new(),
                &mut candidate,
                site.span,
            )? {
                return Err(error(
                    site.span,
                    "code generator type defaults remain unresolved",
                ));
            }
            let instance = candidate
                .finish()
                .map_err(|e| call_inference_error(site.span, e))?;
            type_plan
                .iter()
                .map(|ty| {
                    ty.instantiate_types(&parameters, &instance.type_arguments)
                        .map_err(|e| error(site.span, e.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        self.check_type_application(owner, &types, site.span)?;
        let definition = self.instantiate_code_generator(&template, &types, site.span)?;
        for ((value, actual), (_, input)) in values.iter_mut().zip(&actual).zip(&definition.inputs)
        {
            match (value, input) {
                (Some(Argument::Code(code)), Input::Code(interface)) => {
                    *code = self.ascribe_code(code.clone(), Some(interface), site.span)?
                }
                (Some(Argument::Data(_)), Input::Data(ty)) => {
                    self.require(actual.as_ref().unwrap(), ty, site.span)?
                }
                (None, _) => {}
                _ => unreachable!("generator input category changed during type instantiation"),
            }
        }
        if self.checking_code_generator {
            return Ok(Arc::new(ExpressionCode {
                body: None,
                result: definition.output.result.clone(),
                context: definition.output.context.clone(),
                ports: definition.output.ports.clone(),
                defaults: BTreeMap::new(),
            }));
        }
        self.generator_frame(owner, |this| this.run_code_generator(&definition, values))
    }

    fn generator_default(&mut self, value: &Expr<'_>, input: &Input) -> Result<Argument, Error> {
        match input {
            Input::Data(ty) => {
                if self.checking_code_generator {
                    let checked = self.expr(value, Some(ty))?;
                    self.validate_generator_data(&checked, value.span)?;
                    Ok(Argument::Data(None))
                } else {
                    let (data, ty) = self.required_typed_expression(value, Some(ty))?;
                    Ok(Argument::Data(Some(
                        self.materialize_generator_data(data, ty, value.span)?,
                    )))
                }
            }
            Input::Code(interface) => {
                let locals = self.locals.clone();
                self.locals.extend(interface.context.clone());
                let code = self.code_value(value, Some(interface), 0);
                self.locals = locals;
                code.map(Argument::Code)
            }
        }
    }

    fn run_code_generator(
        &mut self,
        definition: &Generator,
        arguments: Vec<Option<Argument>>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.alias_substitution.extend(
            definition
                .types
                .iter()
                .map(|(id, ty)| (*id, Arc::new(ty.clone()))),
        );
        for (id, ty) in &definition.types {
            self.context.assume(crate::ptc::Formula::equal(
                Term::Rigid {
                    declaration: *id,
                    sort: Sort::Type,
                },
                ty.clone(),
            ));
        }
        self.locals.extend(definition.output.context.clone());
        let mut inherited: BTreeMap<n::LocalId, Option<n::LocalId>> = BTreeMap::new();
        let owner = self.signature_owner.expect("generator frame");
        let ItemKind::Fn(signature, _) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
        for (index, ((id, input), argument)) in definition.inputs.iter().zip(arguments).enumerate()
        {
            let default = match signature.params[index].ty.kind {
                TyParamKind::Optional(_, _, value) => Some(value),
                _ => None,
            };
            // Every definition checks its defaults even when all calls override
            // them. Invocation evaluates only omitted inputs, in prefix order.
            let from_default = argument.is_none();
            let supplied = if self.checking_code_generator || from_default {
                default
                    .map(|value| self.generator_default(value, input))
                    .transpose()?
            } else {
                None
            };
            let argument = argument.or(supplied).ok_or_else(|| {
                error(
                    signature.params[index].span,
                    "missing code generator argument",
                )
            })?;
            self.immutable.insert(*id);
            match (input, argument) {
                (Input::Data(ty), Argument::Data(value)) => {
                    self.locals.insert(*id, ty.clone());
                    self.generator_data.insert(*id, value);
                }
                (Input::Code(interface), Argument::Code(code)) => {
                    let mut defaults = code.defaults.clone();
                    for (name, from) in &interface.ports {
                        if let Some(to) = definition.output.ports.get(name) {
                            self.require(
                                &interface.context[from],
                                &definition.output.context[to],
                                self.package.body(definition.body).unwrap().value.span,
                            )?;
                            defaults.insert(*from, *to);
                            let actual = code.defaults.get(from).copied().and_then(|actual| {
                                if from_default && definition.output.context.contains_key(&actual) {
                                    inherited.get(&actual).copied().flatten()
                                } else {
                                    Some(actual)
                                }
                            });
                            inherited
                                .entry(*to)
                                .and_modify(|previous| {
                                    if *previous != actual {
                                        *previous = None;
                                    }
                                })
                                .or_insert(actual);
                        }
                    }
                    let code = ExpressionCode {
                        body: code.body.clone(),
                        result: code.result.clone(),
                        context: code.context.clone(),
                        ports: code.ports.clone(),
                        defaults,
                    };
                    self.code_values.insert(*id, Arc::new(code));
                }
                _ => unreachable!("checked generator argument kind"),
            }
        }
        let body = self
            .package
            .body(definition.body)
            .ok_or_else(|| error(rustc_span::DUMMY_SP, "missing generator body"))?;
        let mut output = definition.output.clone();
        output.defaults = inherited
            .iter()
            .filter_map(|(port, actual)| actual.map(|actual| (*port, actual)))
            .collect();
        let code = self.code_value(body.value, Some(&output), 0)?;
        let mut result = Arc::try_unwrap(code).unwrap_or_else(|code| ExpressionCode {
            body: code.body.clone(),
            result: code.result.clone(),
            context: code.context.clone(),
            ports: code.ports.clone(),
            defaults: code.defaults.clone(),
        });
        // Default associations describe the caller's objects. Intermediate
        // signature/context slots must not escape as invented runtime values.
        result.defaults = result
            .defaults
            .into_iter()
            .filter_map(|(port, actual)| {
                if definition.output.context.contains_key(&actual) {
                    inherited
                        .get(&actual)
                        .copied()
                        .flatten()
                        .map(|actual| (port, actual))
                } else {
                    Some((port, actual))
                }
            })
            .collect();
        Ok(Arc::new(result))
    }

    pub(super) fn ascribe_code(
        &mut self,
        code: Arc<ExpressionCode>,
        interface: Option<&Interface>,
        span: Span,
    ) -> Result<Arc<ExpressionCode>, Error> {
        let Some(interface) = interface else {
            return Ok(code);
        };
        self.require_expression(&code.result, &interface.result, span)?;
        if interface.infer_context {
            return Ok(code);
        }
        if code.context.len() != interface.context.len() {
            return Err(error(
                span,
                "code interface cannot discard or invent runtime context",
            ));
        }
        let mut connections = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        for ((from, actual), (to, expected)) in code.context.iter().zip(&interface.context) {
            if !code.ports.values().any(|port| port == from) {
                return Err(error(
                    span,
                    "a fixed code dependency cannot be abstracted into a replaceable context port",
                ));
            }
            self.require(actual, expected, span)?;
            connections.insert(*from, *to);
            if let Some(actual) = code.defaults.get(from) {
                defaults.insert(*to, *actual);
            }
        }
        let body = if let Some(body) = &code.body {
            constants::charge_copy(body, &mut self.required_steps, span)?;
            let mut body = (**body).clone();
            freshen(
                &mut body,
                connections,
                &mut self.next_temporary_binding,
                &mut self.expansion_sources,
                span,
            )?;
            Some(Arc::new(body))
        } else {
            None
        };
        Ok(Arc::new(ExpressionCode {
            body,
            result: code.result.clone(),
            context: interface.context.clone(),
            ports: interface.ports.clone(),
            defaults,
        }))
    }

    pub(in crate::program) fn code_producer(&self, value: &Expr<'_>) -> bool {
        self.code_producer_in(value, &mut BTreeSet::new(), 0)
    }

    fn code_producer_in(
        &self,
        value: &Expr<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        depth: usize,
    ) -> bool {
        if depth >= 64 {
            return false;
        }
        match &value.kind {
            H::CodeQuote(..) => true,
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                bindings.contains(&local(*id)) || self.code_values.contains_key(&local(*id))
            }
            H::Application(callee, _) | H::ExtendedApplication(callee, _) => {
                self.generator_owner(callee).is_some()
            }
            H::Let(decl) => {
                if decl
                    .init
                    .is_some_and(|value| self.code_producer_in(value, bindings, depth + 1))
                {
                    bindings.extend(pattern_binding_ids(&decl.pattern));
                }
                false
            }
            H::Return(Some(value)) | H::Semi(value) => {
                self.code_producer_in(value, bindings, depth + 1)
            }
            H::If(_, yes, no) => {
                let yes = self.code_block_producer(yes, bindings, depth + 1);
                let no = no.is_some_and(|value| self.code_producer_in(value, bindings, depth + 1));
                yes || no
            }
            H::Block(body) => self.code_block_producer(body, bindings, depth + 1),
            H::Match(_, arms) => arms
                .iter()
                .any(|arm| self.code_producer_in(arm.body, bindings, depth + 1)),
            _ => false,
        }
    }

    fn code_block_producer(
        &self,
        body: &hir::Block<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        depth: usize,
    ) -> bool {
        if depth >= 64 {
            return false;
        }
        let mut returned = false;
        for statement in body.stmts {
            returned |= self.code_producer_in(statement, bindings, depth + 1);
        }
        returned
            || body
                .expr
                .is_some_and(|value| self.code_producer_in(value, bindings, depth + 1))
    }

    pub(super) fn code_block(
        &mut self,
        block: &hir::Block<'_>,
        interface: Option<&Interface>,
        depth: usize,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.generator_scope(block.span, |this| {
            let mut loops = Vec::new();
            let mut control = Control::Next;
            for statement in block.stmts {
                let next = this.generator_statement(statement, interface, depth + 1, &mut loops)?;
                if this.checking_code_generator {
                    control = this.sequence_generator_control(control, next, statement.span)?;
                } else {
                    match next {
                        Control::Return(code) => return Ok(code),
                        Control::Next => {}
                        _ => {
                            return Err(error(
                                statement.span,
                                "generator loop transfer escaped its loop",
                            ));
                        }
                    }
                }
            }
            if let Some(value) = block.expr {
                let tail = if matches!(
                    value.kind,
                    H::Loop(_)
                        | H::If(_, _, None)
                        | H::Semi(_)
                        | H::Assign(..)
                        | H::AssignOp(..)
                        | H::Let(_)
                        | H::Break(_)
                        | H::BreakTo(_)
                        | H::Continue(_)
                        | H::ContinueTo(_)
                ) || matches!(value.kind, H::Match(..)) && !this.code_producer(value)
                {
                    this.generator_statement(value, interface, depth + 1, &mut loops)?
                } else {
                    Control::Return(this.code_value(value, interface, depth + 1)?)
                };
                if !this.checking_code_generator {
                    return match tail {
                        Control::Return(code) => Ok(code),
                        Control::Next => Err(error(
                            value.span,
                            "generator can finish without returning a code result",
                        )),
                        _ => Err(error(
                            value.span,
                            "generator loop transfer escaped its loop",
                        )),
                    };
                }
                control = this.sequence_generator_control(control, tail, value.span)?;
            }
            if !this.checking_code_generator {
                return Err(error(block.span, "generator block requires a code result"));
            }
            let flow = control.flow();
            if flow.next {
                return Err(error(
                    block.span,
                    "generator can finish without returning a code result",
                ));
            }
            if flow.has_transfer() {
                return Err(error(
                    block.span,
                    "generator loop transfer escaped its loop",
                ));
            }
            let mut returns = flow.returns.into_iter();
            if let Some(first) = returns.next() {
                for code in returns {
                    this.require_expression(&code.result, &first.result, block.span)?;
                    if code.context != first.context || code.ports != first.ports {
                        let expected = Interface {
                            infer_context: false,
                            context: first.context.clone(),
                            ports: first.ports.clone(),
                            defaults: first.defaults.clone(),
                            result: first.result.clone(),
                        };
                        this.ascribe_code(code, Some(&expected), block.span)?;
                    }
                }
                Ok(first)
            } else {
                // No finite exit: use only the expected interface during
                // definition checking. Actual invocation still must terminate.
                let expected = interface.ok_or_else(|| {
                    error(
                        block.span,
                        "diverging code construction needs a contextual result interface",
                    )
                })?;
                Ok(Arc::new(ExpressionCode {
                    body: None,
                    result: expected.result.clone(),
                    context: expected.context.clone(),
                    ports: expected.ports.clone(),
                    defaults: expected.defaults.clone(),
                }))
            }
        })
    }

    fn generator_scope<R>(
        &mut self,
        span: Span,
        action: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let work = self
            .locals
            .len()
            .saturating_add(self.code_values.len())
            .saturating_add(self.generator_data.len())
            .saturating_add(self.immutable.len());
        self.required_steps = self
            .required_steps
            .checked_sub(work as u64)
            .ok_or_else(|| error(span, "generator scope snapshot budget exhausted"))?;
        for value in self.generator_data.values().flatten() {
            constants::charge_copy(value, &mut self.required_steps, span)?;
        }
        let mut saved = (
            self.locals.clone(),
            self.code_values.clone(),
            self.generator_data.clone(),
            self.immutable.clone(),
        );
        let result = action(self);
        if result.is_ok() {
            for (id, value) in &mut saved.1 {
                if let Some(current) = self.code_values.get(id) {
                    *value = current.clone();
                }
            }
            for (id, value) in &mut saved.2 {
                if let Some(current) = self.generator_data.get(id) {
                    *value = current.clone();
                }
            }
        }
        (
            self.locals,
            self.code_values,
            self.generator_data,
            self.immutable,
        ) = saved;
        result
    }

    pub(super) fn generator_binding(&mut self, decl: &hir::decl::LetDecl<'_>) -> Result<(), Error> {
        self.data_binding(decl, false)
    }

    pub(super) fn local_data_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
    ) -> Result<(), Error> {
        self.data_binding(decl, true)
    }

    fn data_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        local_const: bool,
    ) -> Result<(), Error> {
        if decl.guard.is_some() || decl.otherwise.is_some() {
            return Err(error(
                decl.span,
                "conditional generator bindings are not yet implemented",
            ));
        }
        let value = decl
            .init
            .ok_or_else(|| error(decl.span, "generator binding needs an initializer"))?;
        if self.code_producer(value) {
            self.code_binding(decl, None, true)?;
        } else {
            let expected = decl.ty.map(|ty| self.type_expr(ty)).transpose()?;
            let (ty, value) = if self.checking_code_generator {
                let checked = self.expr(value, expected.as_ref())?;
                self.validate_generator_data(&checked, value.span)?;
                (checked.ty.clone(), checked)
            } else {
                let (data, ty) = self.required_typed_expression(value, expected.as_ref())?;
                let materialized = if local_const {
                    self.materialize_local_constant(data, ty.clone(), value.span)?
                } else {
                    self.materialize_generator_data(data, ty.clone(), value.span)?
                };
                (ty, materialized)
            };
            self.bind_generator_pattern(decl, value, ty, local_const)?;
        }
        Ok(())
    }

    fn assign_generator_binding(
        &mut self,
        target: &Expr<'_>,
        value: &Expr<'_>,
        depth: usize,
    ) -> Result<(), Error> {
        if !matches!(target.kind, H::ResolvedLocal(..) | H::ResolvedFnParam(..)) {
            return self.assign_generator_place(target, value);
        }
        let id = binding(target)?;
        if self.immutable.contains(&id) {
            return Err(error(
                target.span,
                "cannot assign an immutable generator binding",
            ));
        }
        if let Some(previous) = self.code_values.get(&id).cloned() {
            let expected = Interface {
                infer_context: true,
                context: BTreeMap::new(),
                ports: BTreeMap::new(),
                defaults: BTreeMap::new(),
                result: previous.result.clone(),
            };
            let code = self.code_value(value, Some(&expected), depth)?;
            let work = code
                .context
                .len()
                .saturating_add(previous.context.len())
                .saturating_add(
                    code.context.len().saturating_mul(
                        code.ports
                            .len()
                            .saturating_add(previous.ports.len())
                            .saturating_add(previous.context.len()),
                    ),
                );
            self.required_steps =
                self.required_steps
                    .checked_sub(work as u64)
                    .ok_or_else(|| {
                        error(
                            value.span,
                            "code assignment context adaptation budget exhausted",
                        )
                    })?;
            let mut connections = BTreeMap::new();
            let alpha = code.context.len() == previous.context.len()
                && code
                    .context
                    .keys()
                    .all(|id| code.ports.values().any(|port| port == id))
                && previous
                    .context
                    .keys()
                    .all(|id| previous.ports.values().any(|port| port == id));
            let positions = code
                .context
                .keys()
                .zip(previous.context.keys())
                .map(|(from, to)| (*from, *to))
                .collect::<BTreeMap<_, _>>();
            for (from, ty) in &code.context {
                let names = code
                    .ports
                    .iter()
                    .filter(|(_, binding)| *binding == from)
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>();
                let targets = if alpha {
                    BTreeSet::from([positions[from]])
                } else if names.is_empty() {
                    previous
                        .context
                        .keys()
                        .filter(|to| *to == from || previous.defaults.get(to) == Some(from))
                        .copied()
                        .collect::<BTreeSet<_>>()
                } else {
                    names
                        .into_iter()
                        .map(|name| {
                            previous.ports.get(name).copied().ok_or_else(|| {
                                error(
                                    value.span,
                                    "code assignment introduces an unknown context port",
                                )
                            })
                        })
                        .collect::<Result<BTreeSet<_>, _>>()?
                };
                if targets.len() != 1 {
                    return Err(error(
                        value.span,
                        "code assignment requires an unambiguous existing runtime context connection",
                    ));
                }
                let to = *targets.first().unwrap();
                self.require(ty, &previous.context[&to], value.span)?;
                connections.insert(*from, to);
            }
            let mut defaults = previous.defaults.clone();
            for (from, to) in &connections {
                defaults.remove(to);
                if let Some(actual) = code.defaults.get(from) {
                    defaults.insert(*to, *actual);
                }
            }
            let body = if let Some(body) = &code.body {
                constants::charge_copy(body, &mut self.required_steps, value.span)?;
                let mut body = (**body).clone();
                freshen(
                    &mut body,
                    connections,
                    &mut self.next_temporary_binding,
                    &mut self.expansion_sources,
                    value.span,
                )?;
                Some(Arc::new(body))
            } else {
                None
            };
            self.code_values.insert(
                id,
                Arc::new(ExpressionCode {
                    body,
                    result: previous.result.clone(),
                    context: previous.context.clone(),
                    ports: previous.ports.clone(),
                    defaults,
                }),
            );
        } else if self.generator_data.contains_key(&id) {
            let ty = self
                .locals
                .get(&id)
                .cloned()
                .ok_or_else(|| error(target.span, "generator data binding has no type"))?;
            let data = if self.checking_code_generator {
                let checked = self.expr(value, Some(&ty))?;
                self.validate_generator_data(&checked, value.span)?;
                None
            } else {
                let (data, ty) = self.required_typed_expression(value, Some(&ty))?;
                Some(self.materialize_generator_data(data, ty, value.span)?)
            };
            self.generator_data.insert(id, data);
        } else {
            return Err(error(
                target.span,
                "generator assignment cannot modify a runtime context binding",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_generator_data(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            let free = nir::free_bindings::collect(value, &mut fuel)?;
            if free.iter().any(|id| !self.generator_data.contains_key(id)) {
                return Err(
                    "code generator computation cannot read a runtime context binding".to_string(),
                );
            }
            let mut roots = vec![(value, true)];
            while let Some((root, comptime)) = roots.pop() {
                nir::visit::expression_pruned(root, &mut fuel, |expr, _| {
                    if let K::Call {
                        callee, arguments, ..
                    } = &expr.kind
                    {
                        if callee.ty.is_comptime_callable() != comptime {
                            return Err("code generator computation calls a function in the wrong execution phase".into());
                        }
                        let Term::Function { parameters, .. } = callee.ty.callable_signature()
                        else {
                            return Err("code generator call has no callable signature".into());
                        };
                        roots.push((callee, comptime));
                        for argument in arguments {
                            let parameter =
                                parameters.get(argument.parameter).ok_or_else(|| {
                                    "code generator call has an invalid parameter slot".to_string()
                                })?;
                            roots.push((
                                &argument.value,
                                comptime || parameter.sort != Sort::Runtime,
                            ));
                        }
                        return Ok(false);
                    }
                    if let K::Closure { captures, body, .. } = &expr.kind {
                        roots.push((body, expr.ty.is_comptime_callable()));
                        roots.extend(captures.iter().filter_map(|capture| {
                            capture.value.as_deref().map(|value| (value, comptime))
                        }));
                        return Ok(false);
                    }
                    Ok(true)
                })?;
            }
            Ok(())
        })();
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|message| error(span, message))
    }

    fn bind_generator_pattern(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        value: n::Expr,
        ty: Term,
        local_const: bool,
    ) -> Result<(), Error> {
        if !decl.mutable {
            self.immutable.extend(pattern_binding_ids(&decl.pattern));
        }
        let pattern = self.pattern(&decl.pattern, &ty)?;
        if !self.pattern_exhaustive(&pattern, &ty) {
            return Err(error(
                decl.span,
                "refutable generator let pattern requires a diverging else branch",
            ));
        }
        if let n::Pattern::Binding { binding, inner } = &pattern {
            if matches!(**inner, n::Pattern::Wild) {
                self.generator_data
                    .insert(*binding, (!self.checking_code_generator).then_some(value));
                return Ok(());
            }
        }
        let bindings = pattern_bindings(&decl.pattern)?
            .into_values()
            .map(|id| {
                self.locals
                    .get(&id)
                    .cloned()
                    .map(|ty| (id, ty))
                    .ok_or_else(|| {
                        error(
                            decl.span,
                            "generator pattern did not establish its output binding",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Execute the ordinary checked pattern once, returning a private packet
        // of its exports. This reuses the interpreter's matching/remap rules,
        // rather than inventing a second matcher for code construction.
        let packet_type = self.tuple_value_type(
            bindings.iter().map(|(_, ty)| ty.clone()).collect(),
            decl.span,
        )?;
        let packet = n::Expr {
            ty: packet_type.clone(),
            source: source(decl.span),
            kind: K::Tuple(
                bindings
                    .iter()
                    .map(|(id, ty)| n::Expr {
                        ty: ty.clone(),
                        source: source(decl.span),
                        kind: K::Local(*id),
                    })
                    .collect(),
            ),
        };
        let root = n::Expr {
            ty: packet_type,
            source: source(decl.span),
            kind: K::Block {
                statements: vec![n::Expr {
                    ty: p(P::Void),
                    source: source(decl.span),
                    kind: K::LetPattern {
                        pattern,
                        value: Box::new(value),
                        guard: None,
                        otherwise: None,
                    },
                }],
                result: Some(Box::new(packet)),
            },
        };
        self.validate_generator_data(&root, decl.span)?;
        if self.checking_code_generator {
            for (id, _) in bindings {
                self.generator_data.insert(id, None);
            }
            return Ok(());
        }
        let (values, _) = self.required_normal_input(decl.init.unwrap(), root, vec![])?;
        let comptime_interpretor::Value::Tuple(values) = values else {
            return Err(error(
                decl.span,
                "generator pattern produced an invalid binding packet",
            ));
        };
        if values.len() != bindings.len() {
            return Err(error(decl.span, "generator pattern output arity mismatch"));
        }
        for ((id, ty), value) in bindings.into_iter().zip(values) {
            let value = if local_const {
                self.materialize_local_constant(value, ty, decl.span)?
            } else {
                self.materialize_generator_data(value, ty, decl.span)?
            };
            self.generator_data.insert(id, Some(value));
        }
        Ok(())
    }
}
