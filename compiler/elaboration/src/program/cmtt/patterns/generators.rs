//! Pattern-producing calls share generator frames and comptime data evaluation.
//! Pattern interfaces remain separate from expression-code input contexts.
use super::*;

type PatternTypePlan = super::super::generators::GeneratorTypePlan;

#[derive(Clone)]
enum PatternInput {
    Data(Term),
    Code(Arc<PatternCode>),
}

enum PatternArgument {
    Data(Option<n::Expr>),
    LiveData(comptime_interpretor::Value),
    Code(Arc<PatternCode>),
}

impl fmt::Display for PatternInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(ty) => write!(f, "(data {ty})"),
            Self::Code(code) => write!(f, "(code {code})"),
        }
    }
}

#[derive(Clone)]
pub(in crate::program) struct PatternGenerator {
    inputs: Vec<(n::LocalId, PatternInput)>,
    types: Vec<(DeclarationId, Term)>,
    clause_domains: Vec<Term>,
    interface: PatternInterface,
    body: hir::BodyId,
    checked: bool,
}

pub(in crate::program) use nir::code_interfaces::PatternCodeInterface as PatternInterface;

impl std::ops::Deref for PatternGenerator {
    type Target = PatternInterface;
    fn deref(&self) -> &Self::Target {
        &self.interface
    }
}

impl fmt::Display for PatternGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(pattern-generator (types")?;
        for ((id, ty), domain) in self.types.iter().zip(&self.clause_domains) {
            write!(f, " (input {id} (domain {domain}) (value {ty}))")?;
        }
        f.write_str(") (inputs")?;
        for (id, ty) in &self.inputs {
            write!(f, " (input {id} {ty})")?;
        }
        write!(f, ") (subject {}) (outputs", self.subject)?;
        for (name, ty) in &self.outputs {
            write!(f, " (output {name:?} {ty})")?;
        }
        write!(f, ") (checked {}))", self.checked)
    }
}

impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn pattern_interface_evidence(
        &mut self,
        code: Arc<PatternCode>,
        span: Span,
    ) -> Result<Arc<PatternCode>, Error> {
        // Unknown selection proves an interface, not the first candidate's
        // matching behavior or coverage. Never publish that candidate as code.
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = (|| {
            nir::type_computations::charge_term(&code.subject, &mut fuel)?;
            for (name, (_, ty)) in &code.outputs {
                fuel = fuel
                    .checked_sub(name.len().saturating_add(1))
                    .ok_or(nir::type_computations::Error::Budget)?;
                nir::type_computations::charge_term(ty, &mut fuel)?;
            }
            Ok::<_, nir::type_computations::Error>(())
        })();
        self.required_steps -= (before - fuel) as u64;
        checked.map_err(|failure| error(span, failure.to_string()))?;
        Ok(Arc::new(PatternCode {
            subject: code.subject.clone(),
            outputs: code.outputs.clone(),
            body: None,
        }))
    }

    fn copy_pattern_generator(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<PatternGenerator, Error> {
        let definition = &self.pattern_generators[&owner];
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            for ty in std::iter::once(&definition.subject)
                .chain(definition.outputs.values())
                .chain(definition.types.iter().map(|(_, ty)| ty))
                .chain(definition.clause_domains.iter())
            {
                nir::type_computations::charge_term(ty, &mut fuel)?;
            }
            for (_, input) in &definition.inputs {
                match input {
                    PatternInput::Data(ty) => nir::type_computations::charge_term(ty, &mut fuel)?,
                    PatternInput::Code(code) => {
                        nir::type_computations::charge_term(&code.subject, &mut fuel)?;
                        for (name, (_, ty)) in &code.outputs {
                            fuel = fuel
                                .checked_sub(name.len().saturating_add(1))
                                .ok_or(nir::type_computations::Error::Budget)?;
                            nir::type_computations::charge_term(ty, &mut fuel)?;
                        }
                    }
                }
            }
            for name in definition.outputs.keys() {
                fuel = fuel
                    .checked_sub(name.len().saturating_add(1))
                    .ok_or(nir::type_computations::Error::Budget)?;
            }
            Ok::<_, nir::type_computations::Error>(())
        })();
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|failure| error(span, failure.to_string()))?;
        Ok(definition.clone())
    }

    pub(in crate::program) fn is_pattern_generator(&self, owner: OwnerId) -> bool {
        self.package.item(owner).is_some_and(|item| {
            matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                if sig.return_ty.is_some_and(|ty| matches!(ty.kind, H::ContextualPatternType(..))))
        })
    }

    pub(in crate::program) fn ensure_pattern_generator(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        if self.pattern_generators.contains_key(&owner) {
            return Ok(());
        }
        if self.active_signatures.len() >= 64 || !self.active_signatures.insert(owner) {
            return Err(error(span, "cyclic pattern generator signature"));
        }
        let result = self.generator_frame(owner, |this| {
            let ItemKind::Fn(sig, body) = &this.package.item(owner).unwrap().kind else {
                return Err(error(
                    span,
                    "pattern generator declarations without a body are not yet implemented",
                ));
            };
            if !sig.modifiers.is_comptime
                || sig.modifiers.is_normal_form_head
                || sig.modifiers.is_quote
            {
                return Err(error(
                    span,
                    "pattern-code result requires an explicit comptime fn",
                ));
            }
            if this.self_containers.contains_key(&owner) || sig.return_binding.is_some() {
                return Err(error(
                    span,
                    "pattern generator methods and named code results are not yet implemented",
                ));
            }
            let parameters = this.type_parameters[&owner].clone();
            let mut domains = Vec::new();
            for parameter in sig.clause_params {
                // Later domains can refer to already formed value inputs.
                // The prefix is incomplete until every clause has been checked.
                this.trait_parameter_types.insert(owner, domains.clone());
                let domain = match parameter.kind {
                    hir::clause::ClauseParamKind::Type(_)
                    | hir::clause::ClauseParamKind::TraitBound(..) => p(P::Type),
                    hir::clause::ClauseParamKind::Positional(_, ty)
                    | hir::clause::ClauseParamKind::Optional(_, ty, _) => this.type_expr(ty)?,
                    _ => {
                        return Err(error(
                            parameter.span,
                            "unsupported code generator clause protocol",
                        ));
                    }
                };
                if !nir::normal_forms::supported_input_type_in_scope(
                    &domain,
                    &this.input_binders(&parameters),
                    |head| Some(head) == this.protocols.tuple,
                    |id| {
                        this.structs.contains_key(&id)
                            || this.enums.contains_key(&id)
                            || this.newtypes.contains_key(&id)
                            || this.context.is_nominal_constructor(id)
                    },
                ) {
                    return Err(error(
                        parameter.span,
                        "code generator clause requires an immutable value domain",
                    ));
                }
                domains.push(domain);
            }
            for (id, domain) in this.type_parameters[&owner].iter().zip(&domains) {
                this.implementation_index
                    .register_parameter_domain(*id, domain.clone())
                    .map_err(|message| error(span, message))?;
                this.inherent_index
                    .register_parameter_domain(*id, domain.clone())
                    .map_err(|message| error(span, message))?;
            }
            this.trait_parameter_types.insert(owner, domains.clone());
            this.capabilities.entry(owner).or_default();
            this.prepare_parameter_capabilities(owner, sig.clause_params)?;
            this.ensure_contract(owner, span)?;
            this.context = this.definition_context(owner);
            let types = parameters
                .iter()
                .zip(&domains)
                .map(|(id, domain)| {
                    (
                        *id,
                        Term::Rigid {
                            declaration: *id,
                            sort: if *domain == p(P::Type) {
                                Sort::Type
                            } else {
                                Sort::ComptimeValue
                            },
                        },
                    )
                })
                .collect();
            let mut inputs = Vec::new();
            for parameter in sig.params {
                let annotation = match parameter.ty.kind {
                    TyParamKind::Positional(annotation)
                    | TyParamKind::Optional(_, annotation, _)
                    | TyParamKind::Varadic(_, annotation) => annotation,
                    _ => {
                        return Err(error(
                            parameter.span,
                            "pattern generator input protocol is not yet implemented",
                        ));
                    }
                };
                if parameter.ty.flags
                    & !(hir::common::TPARAM_COMPTIME | hir::common::TPARAM_IMPLICIT)
                    != 0
                {
                    return Err(error(
                        parameter.span,
                        "pattern generator parameter protocol is not yet implemented",
                    ));
                }
                if let H::ContextualPatternType(subject, entries) = &annotation.kind {
                    if matches!(parameter.ty.kind, TyParamKind::Varadic(..)) {
                        return Err(error(
                            parameter.span,
                            "variadic pattern code collections are not yet implemented",
                        ));
                    }
                    let subject = this.type_expr(subject)?;
                    let mut outputs = BTreeMap::new();
                    for entry in entries {
                        let ty = this.type_expr(entry.ty)?;
                        if outputs
                            .insert(entry.name.name.to_string(), (local(entry.hir_id), ty))
                            .is_some()
                        {
                            return Err(error(entry.name.span, "duplicate pattern input output"));
                        }
                    }
                    let code = Arc::new(PatternCode {
                        subject,
                        outputs,
                        body: None,
                    });
                    this.validate_pattern_code(&code, parameter.span)?;
                    inputs.push((local(parameter.hir_id), PatternInput::Code(code)));
                    continue;
                }
                let ty = this.type_expr(annotation)?;
                if !this.generator_data_domain(&ty, &parameters) {
                    return Err(error(
                        parameter.span,
                        "pattern generator data domain is not supported",
                    ));
                }
                inputs.push((local(parameter.hir_id), PatternInput::Data(ty.clone())));
                this.locals.insert(local(parameter.hir_id), ty);
            }
            let H::ContextualPatternType(subject, entries) = &sig.return_ty.unwrap().kind else {
                unreachable!()
            };
            let subject = this.type_expr(subject)?;
            let mut outputs = BTreeMap::new();
            for entry in entries {
                let ty = this.type_expr(entry.ty)?;
                if outputs.insert(entry.name.name.to_string(), ty).is_some() {
                    return Err(error(entry.name.span, "duplicate pattern generator output"));
                }
            }
            Ok(PatternGenerator {
                inputs,
                types,
                clause_domains: domains,
                interface: PatternInterface { subject, outputs },
                body: *body,
                checked: false,
            })
        });
        self.active_signatures.remove(&owner);
        self.pattern_generators.insert(owner, result?);
        Ok(())
    }

    pub(in crate::program) fn check_pattern_generator(
        &mut self,
        owner: OwnerId,
    ) -> Result<(), Error> {
        let span = self.package.item(owner).unwrap().span;
        self.ensure_pattern_generator(owner, span)?;
        let definition = self.copy_pattern_generator(owner, span)?;
        if definition.checked {
            return Ok(());
        }
        let checking = std::mem::replace(&mut self.checking_code_generator, true);
        let result = self.generator_frame(owner, |this| {
            this.check_generator_clause_value_defaults(owner, &definition.types)?;
            this.alias_substitution.extend(
                definition
                    .types
                    .iter()
                    .map(|(id, ty)| (*id, Arc::new(ty.clone()))),
            );
            this.install_generator_clause_values(
                owner,
                &definition.types,
                &definition.clause_domains,
            )?;
            let ItemKind::Fn(signature, _) = &this.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            for ((id, input), parameter) in definition.inputs.iter().zip(signature.params) {
                if let TyParamKind::Optional(_, _, value) = parameter.ty.kind {
                    this.evaluate_pattern_argument(value, input)?;
                }
                this.immutable.insert(*id);
                match input {
                    PatternInput::Data(ty) => {
                        this.locals.insert(*id, ty.clone());
                        this.generator_data.insert(*id, None);
                    }
                    PatternInput::Code(code) => {
                        this.pattern_codes.insert(*id, code.clone());
                    }
                }
            }
            this.run_pattern_generator_body(owner, &definition, span)
        });
        self.checking_code_generator = checking;
        result?;
        self.pattern_generators.get_mut(&owner).unwrap().checked = true;
        Ok(())
    }

    pub(in crate::program::cmtt) fn invoke_pattern_generator(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        span: Span,
    ) -> Result<Arc<PatternCode>, Error> {
        self.invoke_pattern_generator_expected(callee, arguments, None, span)
    }

    pub(in crate::program::cmtt) fn invoke_pattern_properties(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
    ) -> Result<Arc<PatternCode>, Error> {
        if !arguments.iter().any(|arg| matches!(arg, Arg::Named(..))) {
            return self.invoke_pattern_generator_expected(
                callee,
                arguments,
                expected_subject,
                site.span,
            );
        }
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Err(error(
                site.span,
                "pattern property call requires a named generator",
            ));
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .filter(|owner| self.is_pattern_generator(*owner))
            .ok_or_else(|| error(site.span, "expected a pattern generator"))?;
        self.ensure_pattern_generator(owner, site.span)?;
        let template = self.copy_pattern_generator(owner, site.span)?;
        let plan = if let H::NFApplication(_, types) = callee.kind {
            (
                vec![],
                self.type_clause_arguments(owner, types, None, site.span)?,
                vec![],
                vec![],
            )
        } else {
            self.generator_clause_plan(owner, site.span)?
        };
        let definition = self.instantiate_pattern_generator(template, &plan.1, site.span)?;
        let ItemKind::Fn(signature, _) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
        let mut collectors = Vec::new();
        for (index, parameter) in signature.params.iter().enumerate() {
            if !matches!(parameter.ty.kind, TyParamKind::Varadic(..)) {
                continue;
            }
            let PatternInput::Data(ty) = &definition.inputs[index].1 else {
                continue;
            };
            if matches!(
                self.structural_type(ty, site.span)?,
                Term::Record(_)
                    | Term::Rigid { .. }
                    | Term::NormalForm { .. }
                    | Term::AssociatedType { .. }
                    | Term::Bound { .. }
                    | Term::Hole { .. }
            ) {
                collectors.push(parameter.name);
            }
        }
        let [collector] = collectors.as_slice() else {
            return Err(error(
                site.span,
                "pattern property collection requires one eligible variadic record entrance",
            ));
        };
        self.with_generator_properties(site, arguments, *collector, |this, inputs| {
            this.invoke_pattern_generator_planned(
                callee,
                inputs,
                expected_subject,
                site.span,
                Some(plan),
            )
        })
    }

    pub(in crate::program::cmtt) fn invoke_pattern_generator_expected(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        span: Span,
    ) -> Result<Arc<PatternCode>, Error> {
        self.invoke_pattern_generator_planned(callee, arguments, expected_subject, span, None)
    }

    fn invoke_pattern_generator_planned(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        span: Span,
        plan: Option<PatternTypePlan>,
    ) -> Result<Arc<PatternCode>, Error> {
        if arguments.iter().any(|arg| matches!(arg, Arg::Expand(_))) {
            let head = match callee.kind {
                H::NFApplication(head, _) => head,
                _ => callee,
            };
            let H::ResolvedDef { def_id, .. } = head.kind else {
                return Err(error(
                    span,
                    "pattern generator callee must be a named declaration",
                ));
            };
            let owner = self
                .package
                .definition_owner(def_id)
                .filter(|owner| self.is_pattern_generator(*owner))
                .ok_or_else(|| error(span, "expected a pattern generator"))?;
            self.ensure_pattern_generator(owner, span)?;
            return self.with_generator_spreads(owner, span, arguments, |this, inputs, packets| {
                this.invoke_pattern_generator_inputs(
                    callee,
                    inputs,
                    expected_subject,
                    span,
                    packets,
                    plan,
                )
            });
        }
        self.invoke_pattern_generator_inputs(
            callee,
            arguments,
            expected_subject,
            span,
            &mut super::super::generators::spreads::SpreadPlan::default(),
            plan,
        )
    }

    fn invoke_pattern_generator_inputs(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        span: Span,
        packets: &mut super::super::generators::spreads::SpreadPlan<'_>,
        prepared_types: Option<PatternTypePlan>,
    ) -> Result<Arc<PatternCode>, Error> {
        let (head, explicit) = match callee.kind {
            H::NFApplication(head, args) => (head, Some(args)),
            _ => (callee, None),
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Err(error(
                span,
                "pattern generator callee must be a named declaration",
            ));
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .filter(|owner| self.is_pattern_generator(*owner))
            .ok_or_else(|| error(span, "expected a pattern generator"))?;
        if self.code_construction_depth >= 32 {
            return Err(error(
                span,
                "pattern generator construction depth exhausted",
            ));
        }
        self.code_construction_depth += 1;
        let result = (|| {
            self.ensure_pattern_generator(owner, span)?;
            let template = self.copy_pattern_generator(owner, span)?;
            let (parameters, mut plan, pending_defaults, deferred_clauses) =
                if let Some(plan) = prepared_types {
                    plan
                } else if let Some(args) = explicit {
                    (
                        Vec::new(),
                        self.type_clause_arguments(owner, args, None, span)?,
                        Vec::new(),
                        Vec::new(),
                    )
                } else {
                    self.generator_clause_plan(owner, span)?
                };
            let definition = self.instantiate_pattern_generator(template.clone(), &plan, span)?;
            let ItemKind::Fn(signature, _) = &self.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let (routes, _) = route_arguments(
                arguments,
                Some(signature.params),
                definition.inputs.len(),
                span,
            )?;
            let mut completed_defaults =
                super::super::generators::clauses::CompletedDefaults::default();
            let hinted = if !parameters.is_empty() {
                use crate::calls::InputContext as C;
                let scheme = self.pattern_call_scheme(&definition, &parameters, span)?;
                let mut contexts = vec![C::Unknown; definition.inputs.len()];
                for route in &routes {
                    let context =
                        if matches!(definition.inputs[route.parameter].1, PatternInput::Code(_)) {
                            let mut inputs = route.source_inputs();
                            match inputs.next().map(|(_, value)| &value.kind) {
                                Some(H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id)) => self
                                    .pattern_codes
                                    .get(&local(*id))
                                    .map_or(C::Unknown, |code| {
                                        C::Known(Term::Tuple(
                                            std::iter::once(code.subject.clone())
                                                .chain(
                                                    code.outputs.values().map(|(_, ty)| ty.clone()),
                                                )
                                                .collect(),
                                        ))
                                    }),
                                _ => C::Unknown,
                            }
                        } else if let Some(fields) = route.property_fields() {
                            let mut contexts = Vec::new();
                            for field in fields {
                                contexts.push((
                                    field.ident.name.to_string(),
                                    self.pattern_data_input_hint(field.expr, packets)?,
                                ));
                            }
                            C::Record(contexts)
                        } else if route.is_collected() {
                            let mut fields = Vec::new();
                            for (_, value) in route.source_inputs() {
                                fields.push(self.pattern_data_input_hint(value, packets)?);
                            }
                            match self.protocols.tuple {
                                Some(head) => C::NormalTuple { head, fields },
                                None => C::Tuple(fields),
                            }
                        } else {
                            self.pattern_data_input_hint(
                                route.source_inputs().next().unwrap().1,
                                packets,
                            )?
                        };
                    contexts[route.parameter] = context.clone();
                }
                let candidate = self.generator_call_candidate(
                    owner,
                    &parameters,
                    &mut plan,
                    &pending_defaults,
                    &deferred_clauses,
                    &mut completed_defaults,
                    &scheme,
                    &vec![None; definition.inputs.len()],
                    &contexts,
                    expected_subject,
                    span,
                )?;
                let hints = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, id)| {
                        candidate
                            .type_argument(index)
                            .map(|ty| {
                                ty.unwrap_or(Term::Rigid {
                                    declaration: *id,
                                    sort: self.input_binders(&[*id])[0].sort,
                                })
                            })
                            .map_err(|e| call_inference_error(span, e))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let types = plan
                    .iter()
                    .map(|ty| self.instantiate_input_types(ty, &parameters, &hints, span))
                    .collect::<Result<Vec<_>, _>>()?;
                Some(self.instantiate_pattern_generator(template.clone(), &types, span)?)
            } else {
                None
            };
            let input_definition = hinted.as_ref().unwrap_or(&definition);
            let mut collection_types = BTreeMap::new();
            let mut collected: BTreeMap<usize, Vec<(PatternArgument, Term)>> = BTreeMap::new();
            let mut properties = BTreeMap::new();
            for route in &routes {
                if let Some(fields) = route.property_fields() {
                    properties.insert(route.parameter, fields);
                    collected.insert(route.parameter, Vec::new());
                    let PatternInput::Data(ty) = &input_definition.inputs[route.parameter].1 else {
                        return Err(error(span, "property collection requires data"));
                    };
                    match self.structural_type(ty, span)? {
                        Term::Record(types) => {
                            if types.len() != fields.len() {
                                return Err(error(span, "pattern property field count mismatch"));
                            }
                            let expected = fields
                                .iter()
                                .map(|field| {
                                    types
                                        .iter()
                                        .find(|(name, _)| name == field.ident.name.as_str())
                                        .map(|(_, ty)| ty.clone())
                                        .ok_or_else(|| {
                                            error(field.span, "unknown pattern property field")
                                        })
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            collection_types.insert(route.parameter, expected);
                        }
                        _ if parameters.iter().any(|id| ty.mentions_rigid(*id)) => {}
                        _ => {
                            return Err(error(
                                span,
                                "pattern property collection requires a record type",
                            ));
                        }
                    }
                } else if route.is_collected() {
                    collected.insert(route.parameter, Vec::new());
                    let PatternInput::Data(ty) = &input_definition.inputs[route.parameter].1 else {
                        return Err(error(
                            span,
                            "variadic pattern code collections are not yet implemented",
                        ));
                    };
                    match self.structural_type(ty, span)? {
                        Term::Tuple(fields) => {
                            if fields.len() != route.source_inputs().count() {
                                return Err(error(
                                    span,
                                    "pattern generator variadic tuple arity mismatch",
                                ));
                            }
                            collection_types.insert(route.parameter, fields);
                        }
                        _ if parameters.iter().any(|id| ty.mentions_rigid(*id)) => {}
                        _ => {
                            return Err(error(
                                span,
                                "pattern generator variadic input requires a tuple type",
                            ));
                        }
                    }
                }
            }
            let mut supplied: Vec<Option<PatternArgument>> =
                input_definition.inputs.iter().map(|_| None).collect();
            let mut events = Vec::new();
            for route in &routes {
                if let Some(fields) = route.property_fields() {
                    for (position, field) in fields.iter().enumerate() {
                        events.push((
                            field.expr.span.lo().0,
                            position,
                            route.parameter,
                            field.expr,
                        ));
                    }
                    continue;
                }
                for (position, value) in route.source_inputs() {
                    events.push((value.span.lo().0, position, route.parameter, value));
                }
            }
            for (_, packet) in &packets.empty {
                events.push((packets.source_start(*packet), *packet, usize::MAX, callee));
            }
            events.sort_by_key(|(source, position, _, _)| (*source, *position));
            let mut actual = vec![None; input_definition.inputs.len()];
            for (_, position, index, value) in events {
                if index == usize::MAX {
                    self.prepare_generator_packet(packets, position, None)?;
                    continue;
                }
                let (argument, shape) = match &input_definition.inputs[index].1 {
                    PatternInput::Code(interface) => {
                        if packets.fields.contains_key(&local(value.hir_id)) {
                            return Err(error(
                                value.span,
                                "data packet cannot supply pattern code evidence",
                            ));
                        }
                        let expected = PatternInterface {
                            subject: interface.subject.clone(),
                            outputs: interface
                                .outputs
                                .iter()
                                .map(|(name, (_, ty))| (name.clone(), ty.clone()))
                                .collect(),
                        };
                        let code = self.pattern_input_value(value, &expected)?;
                        let shape = Term::Tuple(
                            std::iter::once(code.subject.clone())
                                .chain(code.outputs.values().map(|(_, ty)| ty.clone()))
                                .collect(),
                        );
                        (PatternArgument::Code(code), shape)
                    }
                    PatternInput::Data(ty) => {
                        let expected = if let Some(elements) = collected.get(&index) {
                            collection_types
                                .get(&index)
                                .and_then(|types| types.get(elements.len()))
                                .filter(|ty| !parameters.iter().any(|id| ty.mentions_rigid(*id)))
                        } else {
                            (!parameters.iter().any(|id| ty.mentions_rigid(*id))).then_some(ty)
                        };
                        if let Some((packet, element)) =
                            packets.fields.get(&local(value.hir_id)).copied()
                        {
                            let mut fields = vec![None; packets.width(packet)];
                            for route in &routes {
                                let PatternInput::Data(ty) =
                                    &input_definition.inputs[route.parameter].1
                                else {
                                    continue;
                                };
                                if parameters.iter().any(|id| ty.mentions_rigid(*id)) {
                                    continue;
                                }
                                for (element, (_, input)) in route.source_inputs().enumerate() {
                                    if let Some((owner, field)) =
                                        packets.fields.get(&local(input.hir_id))
                                    {
                                        if *owner == packet {
                                            fields[*field] = if route.is_collected() {
                                                collection_types
                                                    .get(&route.parameter)
                                                    .and_then(|types| types.get(element))
                                                    .cloned()
                                            } else {
                                                Some(ty.clone())
                                            };
                                        }
                                    }
                                }
                            }
                            let context = fields
                                .into_iter()
                                .collect::<Option<Vec<_>>>()
                                .map(|fields| self.tuple_value_type(fields, span))
                                .transpose()?;
                            self.prepare_generator_packet(packets, packet, context.as_ref())?;
                            let (argument, ty) = self.generator_packet_field(
                                packets, packet, element, expected, value.span,
                            )?;
                            let argument = match argument {
                                super::super::generators::Argument::Data(value) => {
                                    PatternArgument::Data(value)
                                }
                                super::super::generators::Argument::LiveData(value) => {
                                    PatternArgument::LiveData(value)
                                }
                                _ => unreachable!("data packet produced code evidence"),
                            };
                            (argument, ty)
                        } else if self.checking_code_generator {
                            let checked = self.expr(value, expected)?;
                            self.validate_generator_data(&checked, value.span)?;
                            (PatternArgument::Data(None), checked.ty)
                        } else {
                            let (data, ty) = self.generator_data_expression(value, expected)?;
                            (
                                self.complete_pattern_data_argument(data, &ty, value.span)?,
                                ty,
                            )
                        }
                    }
                };
                if let Some(elements) = collected.get_mut(&index) {
                    elements.push((argument, shape));
                } else {
                    actual[index] = Some(shape);
                    supplied[index] = Some(argument);
                }
            }
            for (index, elements) in collected {
                let ty = if let Some(fields) = properties.get(&index) {
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
                        span,
                    )?
                };
                let argument = if self.checking_code_generator {
                    PatternArgument::Data(None)
                } else if elements
                    .iter()
                    .any(|(value, _)| matches!(value, PatternArgument::LiveData(_)))
                {
                    let mut values = Vec::new();
                    for (value, _) in elements {
                        values.push(match value {
                            PatternArgument::LiveData(value) => value,
                            PatternArgument::Data(Some(value)) => {
                                self.required_normal_input(callee, value, vec![])?.0
                            }
                            _ => {
                                return Err(error(
                                    span,
                                    "pattern collection requires completed data",
                                ));
                            }
                        });
                    }
                    self.complete_pattern_data_argument(
                        if let Some(fields) = properties.get(&index) {
                            comptime_interpretor::Value::AnonymousRecord(
                                fields
                                    .iter()
                                    .zip(values)
                                    .map(|(field, value)| (field.ident.name.to_string(), value))
                                    .collect(),
                            )
                        } else {
                            comptime_interpretor::Value::Tuple(values)
                        },
                        &ty,
                        span,
                    )?
                } else {
                    let values = elements
                        .into_iter()
                        .map(|(value, _)| match value {
                            PatternArgument::Data(Some(value)) => Ok(value),
                            _ => Err(error(span, "pattern collection requires completed data")),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    PatternArgument::Data(Some(n::Expr {
                        source: source(span),
                        ty: ty.clone(),
                        kind: if properties.contains_key(&index) {
                            K::AnonymousRecord { fields: values }
                        } else {
                            K::Tuple(values)
                        },
                    }))
                };
                actual[index] = Some(ty);
                supplied[index] = Some(argument);
            }
            let types = {
                let scheme = self.pattern_call_scheme(&definition, &parameters, span)?;
                let candidate = self.generator_call_candidate(
                    owner,
                    &parameters,
                    &mut plan,
                    &pending_defaults,
                    &deferred_clauses,
                    &mut completed_defaults,
                    &scheme,
                    &actual,
                    &[],
                    expected_subject,
                    span,
                )?;
                let inferred = candidate
                    .finish()
                    .map_err(|e| call_inference_error(span, e))?
                    .type_arguments;
                let mut types = plan
                    .iter()
                    .map(|ty| self.instantiate_input_types(ty, &parameters, &inferred, span))
                    .collect::<Result<Vec<_>, _>>()?;
                for (slot, value) in completed_defaults.clauses {
                    types[slot] = value;
                }
                types
            };
            self.check_type_application(owner, &types, span)?;
            let definition = self.instantiate_pattern_generator(template, &types, span)?;
            if let Some(expected) = expected_subject {
                self.require(&definition.subject, expected, span)?;
            }
            for (index, (_, input)) in definition.inputs.iter().enumerate() {
                match (&supplied[index], input) {
                    (Some(PatternArgument::Code(code)), PatternInput::Code(expected)) => {
                        self.ascribe_pattern_input(
                            code,
                            &PatternInterface {
                                subject: expected.subject.clone(),
                                outputs: expected
                                    .outputs
                                    .iter()
                                    .map(|(name, (_, ty))| (name.clone(), ty.clone()))
                                    .collect(),
                            },
                            span,
                        )?;
                    }
                    (
                        Some(PatternArgument::Data(_) | PatternArgument::LiveData(_)),
                        PatternInput::Data(ty),
                    ) => self.require(actual[index].as_ref().unwrap(), ty, span)?,
                    _ => {}
                }
            }
            self.generator_frame(owner, |this| {
                this.alias_substitution.extend(
                    definition
                        .types
                        .iter()
                        .map(|(id, ty)| (*id, Arc::new(ty.clone()))),
                );
                this.install_generator_clause_values(
                    owner,
                    &definition.types,
                    &definition.clause_domains,
                )?;
                for (((id, input), parameter), provided) in
                    definition.inputs.iter().zip(signature.params).zip(supplied)
                {
                    let value = match provided {
                        Some(value) => value,
                        None => match parameter.ty.kind {
                            TyParamKind::Optional(_, _, value) => {
                                this.evaluate_pattern_argument(value, input)?
                            }
                            _ => return Err(error(span, "missing pattern generator argument")),
                        },
                    };
                    this.immutable.insert(*id);
                    match (input, value) {
                        (PatternInput::Data(ty), PatternArgument::Data(value)) => {
                            this.locals.insert(*id, ty.clone());
                            this.generator_data.insert(*id, value);
                        }
                        (PatternInput::Data(ty), PatternArgument::LiveData(value)) => {
                            this.install_live_generator_input(*id, ty, value, parameter.span);
                        }
                        (PatternInput::Code(_), PatternArgument::Code(code)) => {
                            this.pattern_codes.insert(*id, code);
                        }
                        _ => unreachable!("pattern argument category changed"),
                    }
                }
                this.run_pattern_generator_body(owner, &definition, span)
            })
        })();
        self.code_construction_depth -= 1;
        result
    }

    fn pattern_data_input_hint(
        &mut self,
        value: &Expr<'_>,
        packets: &super::super::generators::spreads::SpreadPlan<'_>,
    ) -> Result<crate::calls::InputContext, Error> {
        let ty = match packets.field_type(local(value.hir_id)) {
            Some(ty) => Some(self.copy_alias_type(ty, value.span)?),
            None => self.generator_expression_type_hint(value, 0)?,
        };
        Ok(ty.map_or(
            crate::calls::InputContext::Unknown,
            crate::calls::InputContext::Known,
        ))
    }

    fn pattern_call_scheme(
        &self,
        definition: &PatternGenerator,
        parameters: &[DeclarationId],
        span: Span,
    ) -> Result<crate::calls::Scheme, Error> {
        let shape = Term::Function {
            parameters: definition
                .inputs
                .iter()
                .map(|(_, input)| Parameter {
                    sort: Sort::Runtime,
                    ty: match input {
                        PatternInput::Data(ty) => ty.clone(),
                        PatternInput::Code(code) => Term::Tuple(
                            std::iter::once(code.subject.clone())
                                .chain(code.outputs.values().map(|(_, ty)| ty.clone()))
                                .collect(),
                        ),
                    },
                })
                .collect(),
            result: Box::new(definition.subject.clone()),
        };
        let scheme = crate::calls::Scheme {
            type_parameters: parameters.len(),
            parameter_sorts: self
                .input_binders(parameters)
                .iter()
                .map(|binder| binder.sort)
                .collect(),
            signature: shape
                .abstract_parameters(&self.input_binders(parameters))
                .map_err(|e| error(span, e.to_string()))?,
        };
        Ok(scheme)
    }

    // Both symbolic checking and concrete invocation use the same contract
    // boundary. Every finite return is collected by pattern_generator_value;
    // a diverging body must never execute its postconditions.
    fn run_pattern_generator_body(
        &mut self,
        owner: OwnerId,
        definition: &PatternGenerator,
        span: Span,
    ) -> Result<Arc<PatternCode>, Error> {
        self.generator_contracts(owner, false, span, None)?;
        let entry = self.capture_generator_entry_data(owner, span)?;
        if self.checking_code_generator {
            self.generator_contracts(owner, true, span, Some(&entry))?;
        }
        let body = self.package.body(definition.body).unwrap().value;
        let code = self.pattern_generator_value(body, definition, 0)?;
        if !self.checking_code_generator {
            self.generator_contracts(owner, true, span, Some(&entry))?;
        }
        Ok(code)
    }

    fn instantiate_pattern_generator(
        &mut self,
        mut definition: PatternGenerator,
        types: &[Term],
        span: Span,
    ) -> Result<PatternGenerator, Error> {
        let parameters = definition
            .types
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut substitute = |ty: &Term| {
            self.copy_alias_type(ty, span)?
                .instantiate_parameters(&self.input_binders(&parameters), types)
                .map_err(|e| error(span, e.to_string()))
        };
        definition.clause_domains = definition
            .clause_domains
            .iter()
            .map(&mut substitute)
            .collect::<Result<_, _>>()?;
        definition.interface = definition.interface.try_map_types(&mut substitute)?;
        for (_, input) in &mut definition.inputs {
            *input = match input {
                PatternInput::Data(ty) => PatternInput::Data(substitute(ty)?),
                PatternInput::Code(code) => PatternInput::Code(Arc::new(PatternCode {
                    subject: substitute(&code.subject)?,
                    outputs: code
                        .outputs
                        .iter()
                        .map(|(name, (id, ty))| Ok((name.clone(), (*id, substitute(ty)?))))
                        .collect::<Result<_, Error>>()?,
                    body: None,
                })),
            };
        }
        definition.types = parameters.into_iter().zip(types.iter().cloned()).collect();
        Ok(definition)
    }

    fn evaluate_pattern_argument(
        &mut self,
        value: &Expr<'_>,
        input: &PatternInput,
    ) -> Result<PatternArgument, Error> {
        match input {
            PatternInput::Code(interface) => Ok(PatternArgument::Code(
                self.pattern_argument(value, interface)?,
            )),
            PatternInput::Data(ty) => {
                if self.checking_code_generator {
                    let checked = self.expr(value, Some(ty))?;
                    self.validate_generator_data(&checked, value.span)?;
                    Ok(PatternArgument::Data(None))
                } else {
                    let (data, actual) = self.generator_data_expression(value, Some(ty))?;
                    self.complete_pattern_data_argument(data, &actual, value.span)
                }
            }
        }
    }

    fn complete_pattern_data_argument(
        &mut self,
        value: comptime_interpretor::Value,
        ty: &Term,
        span: Span,
    ) -> Result<PatternArgument, Error> {
        if self.generator_value_needs_live_storage(&value, span)? {
            Ok(PatternArgument::LiveData(value))
        } else {
            Ok(PatternArgument::Data(Some(
                self.materialize_generator_data(value, ty.clone(), span)?,
            )))
        }
    }

    pub(in crate::program::cmtt) fn pattern_argument(
        &mut self,
        value: &Expr<'_>,
        interface: &PatternCode,
    ) -> Result<Arc<PatternCode>, Error> {
        let expected = PatternInterface {
            subject: interface.subject.clone(),
            outputs: interface
                .outputs
                .iter()
                .map(|(name, (_, ty))| (name.clone(), ty.clone()))
                .collect(),
        };
        let code = self.pattern_generator_value(value, &expected, 0)?;
        self.require(&code.subject, &interface.subject, value.span)?;
        if !code.outputs.keys().eq(interface.outputs.keys()) {
            return Err(error(
                value.span,
                "pattern argument output interface mismatch",
            ));
        }
        for (name, (_, ty)) in &interface.outputs {
            self.require(&code.outputs[name].1, ty, value.span)?;
        }
        self.validate_pattern_code(&code, value.span)?;
        Ok(code)
    }

    fn pattern_generator_block(
        &mut self,
        block: &hir::Block<'_>,
        definition: &PatternInterface,
        depth: usize,
    ) -> Result<Arc<PatternCode>, Error> {
        use crate::program::cmtt::generators::statements::{Control, ResultInterface};
        self.generator_scope(block.span, |this| {
            let mut loops = Vec::new();
            let mut control = Control::Next;
            for statement in block.stmts {
                let next = this.generator_statement_with_result(
                    statement,
                    ResultInterface::Pattern(definition),
                    depth + 1,
                    &mut loops,
                )?;
                if this.checking_code_generator {
                    control = this.sequence_generator_control(control, next, statement.span)?;
                } else {
                    match next {
                        Control::PatternReturn(code) => return Ok(code),
                        Control::Next => {}
                        _ => {
                            return Err(error(
                                statement.span,
                                "pattern generator transfer escaped its target",
                            ));
                        }
                    }
                }
            }
            if let Some(tail) = block.expr {
                let control_expr = match tail.kind {
                    H::InlineControl(inner) => inner,
                    _ => tail,
                };
                let statement = matches!(
                    control_expr.kind,
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
                        | H::For { .. }
                );
                let next = if statement {
                    this.generator_statement_with_result(
                        tail,
                        ResultInterface::Pattern(definition),
                        depth + 1,
                        &mut loops,
                    )?
                } else {
                    Control::PatternReturn(this.pattern_generator_value(
                        tail,
                        definition,
                        depth + 1,
                    )?)
                };
                if !this.checking_code_generator {
                    return match next {
                        Control::PatternReturn(code) => Ok(code),
                        _ => Err(error(
                            tail.span,
                            "pattern generator can finish without a code result",
                        )),
                    };
                }
                control = this.sequence_generator_control(control, next, tail.span)?;
            }
            if !this.checking_code_generator {
                return Err(error(
                    block.span,
                    "pattern generator can finish without a code result",
                ));
            }
            let flow = control.flow();
            if flow.next || flow.has_transfer() || !flow.returns.is_empty() {
                return Err(error(
                    block.span,
                    "pattern generator can finish without a code result",
                ));
            }
            let mut returns = flow.pattern_returns.into_iter();
            match returns.next() {
                Some(first) if returns.next().is_none() => Ok(first),
                Some(first) => this.pattern_interface_evidence(first, block.span),
                None => {
                    let mut outputs = BTreeMap::new();
                    for (name, ty) in &definition.outputs {
                        outputs
                            .insert(name.clone(), (this.spread_binding(block.span)?, ty.clone()));
                    }
                    Ok(Arc::new(PatternCode {
                        subject: definition.subject.clone(),
                        outputs,
                        body: None,
                    }))
                }
            }
        })
    }

    pub(in crate::program::cmtt) fn pattern_generator_value(
        &mut self,
        value: &Expr<'_>,
        definition: &PatternInterface,
        depth: usize,
    ) -> Result<Arc<PatternCode>, Error> {
        if depth >= 64 || self.required_steps == 0 {
            return Err(error(
                value.span,
                "pattern generator construction budget exhausted",
            ));
        }
        self.required_steps -= 1;
        let code = match &value.kind {
            H::Block(block) => {
                return self.pattern_generator_block(block, definition, depth + 1);
            }
            H::Return(Some(inner)) => {
                return self.pattern_generator_value(inner, definition, depth + 1);
            }
            H::If(condition, yes, Some(no)) => {
                let yes_expr = Expr {
                    hir_id: yes.hir_id,
                    span: yes.span,
                    kind: H::Block(yes),
                };
                if self.checking_code_generator {
                    let checked = self.expr(condition, Some(&p(P::Bool)))?;
                    self.validate_generator_data(&checked, condition.span)?;
                    let result = self.pattern_generator_value(&yes_expr, definition, depth + 1)?;
                    let other = self.pattern_generator_value(no, definition, depth + 1)?;
                    // Literal control is available without invoking a producer.
                    match checked.kind {
                        K::Literal(n::Literal::Bool(true)) => result,
                        K::Literal(n::Literal::Bool(false)) => other,
                        _ => self.pattern_interface_evidence(result, value.span)?,
                    }
                } else {
                    let (condition, _) =
                        self.generator_data_expression(condition, Some(&p(P::Bool)))?;
                    match condition {
                        comptime_interpretor::Value::Bool(true) => {
                            self.pattern_generator_value(&yes_expr, definition, depth + 1)?
                        }
                        comptime_interpretor::Value::Bool(false) => {
                            self.pattern_generator_value(no, definition, depth + 1)?
                        }
                        _ => {
                            return Err(error(
                                value.span,
                                "pattern selection requires a comptime boolean",
                            ));
                        }
                    }
                }
            }
            H::InlineControl(inner)
                if matches!(inner.kind, H::If(..) | H::When(..) | H::Match(..)) =>
            {
                return self.pattern_generator_value(inner, definition, depth + 1);
            }
            H::When(arms) => {
                if !arms.iter().any(|arm| arm.cond.is_none()) {
                    return Err(error(
                        value.span,
                        "pattern-valued when requires an else arm",
                    ));
                }
                let mut first = None;
                for arm in *arms {
                    let selected = self.generator_when_condition(arm.cond)?;
                    if self.checking_code_generator {
                        let code = self.generator_scope(arm.span, |this| {
                            this.pattern_generator_value(arm.body, definition, depth + 1)
                        })?;
                        if first.is_none() {
                            first = Some(code);
                        }
                    } else if selected == Some(true) {
                        return self.generator_scope(arm.span, |this| {
                            this.pattern_generator_value(arm.body, definition, depth + 1)
                        });
                    }
                }
                let first = first
                    .ok_or_else(|| error(value.span, "pattern-valued when selected no arm"))?;
                self.pattern_interface_evidence(first, value.span)?
            }
            H::Match(subject, arms) => {
                let results =
                    self.generator_match(value, subject, arms, None, |this, _, body| {
                        this.pattern_generator_value(body, definition, depth + 1)
                    })?;
                let first = results.into_iter().next().ok_or_else(|| {
                    error(
                        value.span,
                        "pattern generator match needs a code-producing arm",
                    )
                })?;
                if self.checking_code_generator {
                    self.pattern_interface_evidence(first, value.span)?
                } else {
                    first
                }
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => self
                .pattern_codes
                .get(&local(*id))
                .cloned()
                .ok_or_else(|| error(value.span, "expected a comptime pattern code value"))?,
            H::CodePatternQuote(..) => {
                self.build_pattern_quote(value, &definition.subject, Some(&definition.outputs))?
            }
            H::ExtendedApplication(callee, arguments) => {
                self.invoke_pattern_properties(value, callee, arguments, Some(&definition.subject))?
            }
            H::Application(callee, arguments) => self.invoke_pattern_generator_expected(
                callee,
                arguments,
                Some(&definition.subject),
                value.span,
            )?,
            _ => {
                return Err(error(
                    value.span,
                    "pattern generator expression is not yet implemented",
                ));
            }
        };
        self.require(&code.subject, &definition.subject, value.span)?;
        if !code.outputs.keys().eq(definition.outputs.keys()) {
            return Err(error(
                value.span,
                "pattern generator output interface mismatch",
            ));
        }
        for (name, ty) in &definition.outputs {
            self.require(&code.outputs[name].1, ty, value.span)?;
        }
        self.validate_pattern_code(&code, value.span)?;
        Ok(code)
    }
}
