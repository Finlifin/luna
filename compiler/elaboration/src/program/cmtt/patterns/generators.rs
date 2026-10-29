//! Pattern-producing calls share generator frames and comptime data evaluation.
//! Pattern interfaces remain separate from expression-code input contexts.
use super::*;

type PatternTypePlan = super::super::generators::GeneratorTypePlan;

#[derive(Clone)]
enum PatternInput {
    Data(Term),
    Expression(Interface),
    Code(Arc<PatternCode>),
}

use super::super::generators::Argument as PatternArgument;

impl fmt::Display for PatternInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expression(code) => write!(f, "(expression {code})"),
            Self::Data(ty) => write!(f, "(data {ty})"),
            Self::Code(code) => write!(f, "(code {code})"),
        }
    }
}

#[derive(Clone)]
pub(in crate::program) struct PatternGenerator {
    inputs: Vec<(n::LocalId, PatternInput)>,
    captures: Vec<(DeclarationId, Term)>,
    types: Vec<(DeclarationId, Term)>,
    clause_domains: Vec<Term>,
    interface: PatternInterface,
    body: Option<hir::BodyId>,
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
        f.write_str("(pattern-generator (captures")?;
        for (id, value) in &self.captures {
            write!(f, " (binding {id} {value})")?;
        }
        f.write_str(") (types")?;
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
                .chain(definition.captures.iter().map(|(_, value)| value))
            {
                nir::type_computations::charge_term(ty, &mut fuel)?;
            }
            for (_, input) in &definition.inputs {
                match input {
                    PatternInput::Data(ty) => nir::type_computations::charge_term(ty, &mut fuel)?,
                    PatternInput::Expression(code) => code.charge(&mut fuel)?,
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
        let mut definition = definition.clone();
        if let Some(captures) = self.code_method_captures.get(&owner) {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let charged = captures
                .iter()
                .try_for_each(|(_, value)| nir::type_computations::charge_term(value, &mut fuel));
            self.required_steps = fuel as u64;
            charged.map_err(|e| error(span, e.to_string()))?;
            let captures = captures.clone();
            let binders =
                self.input_binders(&captures.iter().map(|(id, _)| *id).collect::<Vec<_>>());
            let values = captures
                .iter()
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>();
            self.rewrite_pattern_generator(&mut definition, &binders, &values, span)?;
            definition.captures = captures;
        }
        Ok(definition)
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
            let (sig, body) = match &this.package.item(owner).unwrap().kind {
                ItemKind::Fn(sig, body) => (sig, Some(*body)),
                ItemKind::FnDeclaration(sig) => (sig, None),
                _ => return Err(error(span, "expected a pattern generator")),
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
            if sig.return_binding.is_some() {
                return Err(error(
                    span,
                    "named pattern code results are not yet implemented",
                ));
            }
            let parameters = this.type_parameters[&owner].clone();
            let mut visible_parameters = parameters.clone();
            if let Some(parent) = this.self_containers.get(&owner) {
                visible_parameters.extend(this.type_parameters[parent].iter().copied());
            }
            if let Some(Term::Rigid {
                declaration: self_parameter,
                sort: Sort::Type,
            }) = this.self_types.get(&owner)
            {
                visible_parameters.push(*self_parameter);
                if let Some((parent, _)) = this.package.owners().find(|(parent, _)| {
                    matches_declaration(
                        this.package,
                        &this.package_identities,
                        *parent,
                        *self_parameter,
                    )
                }) {
                    visible_parameters.extend(this.type_parameters[&parent].iter().copied());
                }
            }
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
                if !nir::normal_forms::supported_input_type_with_collections_in(
                    &domain,
                    &this.input_binders(&visible_parameters),
                    |head| Some(head) == this.protocols.tuple,
                    |head| Some(head) == this.protocols.meta_list,
                    |id| {
                        this.structs.contains_key(&id)
                            || this.enums.contains_key(&id)
                            || this.newtypes.contains_key(&id)
                            || this.context.is_nominal_constructor(id)
                    },
                    this.target,
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
                if let TyParamKind::Itself { is_ref } = parameter.ty.kind {
                    let subject = this.self_type(owner, parameter.span)?;
                    let ty = if is_ref {
                        Term::Pointer(Box::new(subject))
                    } else {
                        subject
                    };
                    this.locals.insert(local(parameter.hir_id), ty.clone());
                    inputs.push((local(parameter.hir_id), PatternInput::Data(ty)));
                    continue;
                }
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
                    & !(hir::common::TPARAM_COMPTIME
                        | hir::common::TPARAM_IMPLICIT
                        | hir::common::TPARAM_LAMBDA)
                    != 0
                {
                    return Err(error(
                        parameter.span,
                        "pattern generator parameter protocol is not yet implemented",
                    ));
                }
                if parameter.ty.is_lambda()
                    && matches!(
                        annotation.kind,
                        H::ContextualPatternType(..) | H::ContextualExprType(..)
                    )
                {
                    return Err(error(
                        parameter.span,
                        "generator lambda parameter requires a comptime callable type",
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
                if matches!(annotation.kind, H::ContextualExprType(..)) {
                    if matches!(parameter.ty.kind, TyParamKind::Varadic(..)) {
                        return Err(error(
                            parameter.span,
                            "code collections require code-container evidence not yet implemented",
                        ));
                    }
                    inputs.push((
                        local(parameter.hir_id),
                        PatternInput::Expression(this.code_interface(annotation)?),
                    ));
                    continue;
                }
                let ty = this.type_expr(annotation)?;
                if parameter.ty.is_lambda() && !ty.is_comptime_callable() {
                    return Err(error(
                        parameter.span,
                        "generator lambda parameter requires a comptime callable type",
                    ));
                }
                if !this.generator_data_domain(&ty, &visible_parameters) {
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
            let captures = this
                .input_binders(
                    &visible_parameters
                        .into_iter()
                        .filter(|id| !parameters.contains(id))
                        .collect::<Vec<_>>(),
                )
                .into_iter()
                .map(|binder| {
                    (
                        binder.declaration,
                        Term::Rigid {
                            declaration: binder.declaration,
                            sort: binder.sort,
                        },
                    )
                })
                .collect();
            Ok(PatternGenerator {
                inputs,
                captures,
                types,
                clause_domains: domains,
                interface: PatternInterface { subject, outputs },
                body,
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
            this.install_generator_capture_values(&definition.captures, span)?;
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
            let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                &this.package.item(owner).unwrap().kind
            else {
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
                    PatternInput::Expression(code) => {
                        this.code_values.insert(
                            *id,
                            Arc::new(ExpressionCode {
                                body: None,
                                result: code.result.clone(),
                                context: code.context.clone(),
                                ports: code.ports.clone(),
                                defaults: code.defaults.clone(),
                            }),
                        );
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

    pub(in crate::program) fn pattern_method_contract(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<
        (
            nir::code_interfaces::PatternFunctionSignature,
            nir::contracts::ProgramContract,
        ),
        Error,
    > {
        let definition = self.copy_pattern_generator(owner, span)?;
        let signature = nir::code_interfaces::PatternFunctionSignature {
            clause_domains: definition.clause_domains.clone(),
            inputs: definition
                .inputs
                .iter()
                .map(|(binding, input)| {
                    (
                        *binding,
                        match input {
                            PatternInput::Data(ty) => {
                                nir::code_interfaces::CodeInput::Data(ty.clone())
                            }
                            PatternInput::Expression(code) => {
                                nir::code_interfaces::CodeInput::Code(code.clone())
                            }
                            PatternInput::Code(code) => {
                                nir::code_interfaces::CodeInput::Pattern(PatternInterface {
                                    subject: code.subject.clone(),
                                    outputs: code
                                        .outputs
                                        .iter()
                                        .map(|(name, (_, ty))| (name.clone(), ty.clone()))
                                        .collect(),
                                })
                            }
                        },
                    )
                })
                .collect(),
            output: definition.interface.clone(),
        };
        let context = self.context.clone();
        let checking = std::mem::replace(&mut self.checking_code_generator, true);
        let result = self.generator_frame(owner, |this| {
            this.context = context;
            this.install_generator_capture_values(&definition.captures, span)?;
            this.alias_substitution.extend(
                definition
                    .types
                    .iter()
                    .map(|(id, value)| (*id, Arc::new(value.clone()))),
            );
            this.install_generator_clause_values(
                owner,
                &definition.types,
                &definition.clause_domains,
            )?;
            for (binding, input) in &definition.inputs {
                match input {
                    PatternInput::Data(ty) => {
                        this.locals.insert(*binding, ty.clone());
                        this.generator_data.insert(*binding, None);
                    }
                    PatternInput::Expression(code) => {
                        this.code_values.insert(
                            *binding,
                            Arc::new(ExpressionCode {
                                body: None,
                                result: code.result.clone(),
                                context: code.context.clone(),
                                ports: code.ports.clone(),
                                defaults: code.defaults.clone(),
                            }),
                        );
                    }
                    PatternInput::Code(code) => {
                        this.pattern_codes.insert(*binding, code.clone());
                    }
                }
            }
            let preconditions = this.program_preconditions(owner)?;
            let (postconditions, entry_snapshots) =
                this.program_postconditions_with_result(owner, None)?;
            for snapshot in &entry_snapshots {
                if !this.generator_data.contains_key(&snapshot.parameter) {
                    return Err(error(
                        span,
                        "pattern method entry snapshot requires a data parameter",
                    ));
                }
                this.generator_data.insert(snapshot.binding, None);
            }
            for predicate in preconditions.iter().chain(&postconditions) {
                this.validate_generator_data(predicate, span)?;
            }
            Ok(nir::contracts::ProgramContract {
                preconditions,
                postconditions,
                entry_snapshots,
                result_binding: None,
            })
        });
        self.checking_code_generator = checking;
        Ok((signature, result?))
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
        self.invoke_pattern_properties_in_context(site, callee, arguments, expected_subject, None)
    }

    pub(in crate::program::cmtt) fn invoke_pattern_generator_expected(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        span: Span,
    ) -> Result<Arc<PatternCode>, Error> {
        self.invoke_pattern_generator_in_context(callee, arguments, expected_subject, None, span)
    }

    pub(in crate::program::cmtt) fn invoke_pattern_properties_in_context(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        expected_interface_context: Option<&crate::calls::InputContext>,
    ) -> Result<Arc<PatternCode>, Error> {
        self.with_code_method_callee(callee, |this, callee, receiver| {
            let inputs = receiver
                .into_iter()
                .map(Arg::Positional)
                .chain(arguments.iter().cloned())
                .collect::<Vec<_>>();
            this.invoke_pattern_property_target(
                site,
                callee,
                &inputs,
                expected_subject,
                expected_interface_context,
            )
        })
    }

    fn invoke_pattern_property_target(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        expected_interface_context: Option<&crate::calls::InputContext>,
    ) -> Result<Arc<PatternCode>, Error> {
        if !arguments.iter().any(|arg| matches!(arg, Arg::Named(..))) {
            return self.invoke_pattern_generator_planned(
                callee,
                arguments,
                expected_subject,
                expected_interface_context,
                site.span,
                None,
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
        let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
            &self.package.item(owner).unwrap().kind
        else {
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
                expected_interface_context,
                site.span,
                Some(plan),
            )
        })
    }

    pub(in crate::program::cmtt) fn invoke_pattern_generator_in_context(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        expected_interface_context: Option<&crate::calls::InputContext>,
        span: Span,
    ) -> Result<Arc<PatternCode>, Error> {
        self.with_code_method_callee(callee, |this, callee, receiver| {
            let inputs = receiver
                .into_iter()
                .map(Arg::Positional)
                .chain(arguments.iter().cloned())
                .collect::<Vec<_>>();
            this.invoke_pattern_generator_planned(
                callee,
                &inputs,
                expected_subject,
                expected_interface_context,
                span,
                None,
            )
        })
    }

    fn invoke_pattern_generator_planned(
        &mut self,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected_subject: Option<&Term>,
        expected_interface_context: Option<&crate::calls::InputContext>,
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
                    expected_interface_context,
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
            expected_interface_context,
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
        expected_interface_context: Option<&crate::calls::InputContext>,
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
            // The inference result includes the success interface, not only
            // the scrutinee. These tuples are solver shapes, never runtime data.
            let expected_result = expected_interface_context.cloned().or_else(|| {
                expected_subject.map(|subject| {
                    crate::calls::InputContext::Tuple(
                        std::iter::once(crate::calls::InputContext::Known(subject.clone()))
                            .chain(
                                definition
                                    .outputs
                                    .values()
                                    .map(|_| crate::calls::InputContext::Unknown),
                            )
                            .collect(),
                    )
                })
            });
            let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                &self.package.item(owner).unwrap().kind
            else {
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
            let mut input_contexts =
                vec![crate::calls::InputContext::Unknown; definition.inputs.len()];
            let mut partial_contexts =
                vec![crate::calls::InputContext::Unknown; definition.inputs.len()];
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
                        } else if matches!(
                            definition.inputs[route.parameter].1,
                            PatternInput::Expression(_)
                        ) {
                            match route.source_inputs().next().map(|(_, value)| &value.kind) {
                                Some(H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id)) => self
                                    .code_values
                                    .get(&local(*id))
                                    .map_or(C::Unknown, |code| {
                                        C::Known(Term::Tuple(
                                            std::iter::once(code.result.clone())
                                                .chain(code.context.values().cloned())
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
                            let adapted = signature.params[route.parameter]
                                .ty
                                .is_lambda()
                                .then(|| route.expression(callee));
                            self.pattern_data_input_hint(
                                adapted
                                    .as_ref()
                                    .unwrap_or(route.source_inputs().next().unwrap().1),
                                packets,
                            )?
                        };
                    contexts[route.parameter] = context.clone();
                }
                partial_contexts = contexts.clone();
                let candidate = self.generator_call_candidate_with_result_context(
                    owner,
                    &parameters,
                    &mut plan,
                    &pending_defaults,
                    &deferred_clauses,
                    &mut completed_defaults,
                    &scheme,
                    &vec![None; definition.inputs.len()],
                    &contexts,
                    None,
                    expected_result.as_ref(),
                    span,
                )?;
                input_contexts = (0..definition.inputs.len())
                    .map(|index| {
                        candidate
                            .parameter_input_context(index)
                            .map_err(|e| call_inference_error(span, e))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
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
            let mut input_definition = hinted.unwrap_or_else(|| definition.clone());
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
                let adapted = if signature.params[index].ty.is_lambda() {
                    routes
                        .iter()
                        .find(|route| route.parameter == index)
                        .map(|route| route.expression(callee))
                } else {
                    None
                };
                let value = adapted.as_ref().unwrap_or(value);
                let (argument, shape) = match &input_definition.inputs[index].1 {
                    PatternInput::Expression(interface) => {
                        if packets.fields.contains_key(&local(value.hir_id)) {
                            return Err(error(
                                value.span,
                                "data packet cannot supply code evidence",
                            ));
                        }
                        let known = !parameters
                            .iter()
                            .any(|id| interface.result.mentions_rigid(*id));
                        let mut expected = interface.clone();
                        expected.infer_context = true;
                        let partial_result = match &input_contexts[index] {
                            crate::calls::InputContext::Tuple(fields) => fields.first(),
                            _ => None,
                        };
                        let code = self.code_value_in_context(
                            value,
                            known.then_some(&expected),
                            partial_result,
                            0,
                        )?;
                        let shape = Term::Tuple(
                            std::iter::once(code.result.clone())
                                .chain(code.context.values().cloned())
                                .collect(),
                        );
                        (PatternArgument::Code(CodeValue::Expression(code)), shape)
                    }
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
                        let subject = (!parameters
                            .iter()
                            .any(|id| expected.subject.mentions_rigid(*id)))
                        .then_some(&expected.subject);
                        let partial = match &input_contexts[index] {
                            context @ (crate::calls::InputContext::Tuple(_)
                            | crate::calls::InputContext::Known(_)) => Some(context),
                            _ => None,
                        };
                        let code = self.pattern_input_value(value, &expected, subject, partial)?;
                        let shape = Term::Tuple(
                            std::iter::once(code.subject.clone())
                                .chain(code.outputs.values().map(|(_, ty)| ty.clone()))
                                .collect(),
                        );
                        (PatternArgument::Code(CodeValue::Pattern(code)), shape)
                    }
                    PatternInput::Data(ty) => {
                        if collected.contains_key(&index)
                            && !parameters.iter().any(|id| ty.mentions_rigid(*id))
                        {
                            if let Some(route) =
                                routes.iter().find(|route| route.parameter == index)
                            {
                                route.require_collection_type(self, ty, span)?;
                            }
                            match self.structural_type(ty, span)? {
                                Term::Tuple(fields) => {
                                    collection_types.insert(index, fields);
                                }
                                Term::Record(fields) => {
                                    if let Some(properties) = properties.get(&index) {
                                        let types = properties
                                            .iter()
                                            .map(|field| {
                                                fields
                                                    .iter()
                                                    .find(|(name, _)| {
                                                        name == field.ident.name.as_str()
                                                    })
                                                    .map(|(_, ty)| ty.clone())
                                                    .ok_or_else(|| {
                                                        error(
                                                            field.span,
                                                            "unknown pattern property field",
                                                        )
                                                    })
                                            })
                                            .collect::<Result<Vec<_>, _>>()?;
                                        collection_types.insert(index, types);
                                    }
                                }
                                _ => {}
                            }
                        }
                        let expected = if let Some(elements) = collected.get(&index) {
                            collection_types
                                .get(&index)
                                .and_then(|types| types.get(elements.len()))
                                .filter(|ty| !parameters.iter().any(|id| ty.mentions_rigid(*id)))
                        } else {
                            (!parameters.iter().any(|id| ty.mentions_rigid(*id))).then_some(ty)
                        };
                        let context = if expected.is_some() {
                            None
                        } else {
                            use crate::calls::InputContext as C;
                            match (&input_contexts[index], collected.get(&index)) {
                                (
                                    C::Tuple(fields) | C::NormalTuple { fields, .. },
                                    Some(elements),
                                ) => fields.get(elements.len()),
                                (C::Record(fields), Some(elements)) => {
                                    fields.get(elements.len()).map(|(_, context)| context)
                                }
                                (context, None) => Some(context),
                                _ => None,
                            }
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
                            let checked =
                                self.check_generator_data_input(value, expected, context)?;
                            (PatternArgument::Data(None), checked.ty)
                        } else {
                            let (data, ty) = self
                                .generator_data_expression_in_context(value, expected, context)?;
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
                if !parameters.is_empty() {
                    use crate::calls::InputContext as C;
                    if let Some(elements) = collected.get(&index) {
                        let count = properties
                            .get(&index)
                            .map(|fields| fields.len())
                            .unwrap_or_else(|| {
                                routes
                                    .iter()
                                    .find(|route| route.parameter == index)
                                    .map(|route| route.source_inputs().count())
                                    .unwrap_or(elements.len())
                            });
                        let fields = (0..count)
                            .map(|slot| {
                                if let Some((_, ty)) = elements.get(slot) {
                                    C::Known(ty.clone())
                                } else {
                                    // Unvisited fields retain statically established evidence.
                                    // Completing an earlier field does not invalidate their types.
                                    match &partial_contexts[index] {
                                        C::Tuple(fields) | C::NormalTuple { fields, .. } => {
                                            fields.get(slot).cloned().unwrap_or(C::Unknown)
                                        }
                                        C::Record(fields) => fields
                                            .get(slot)
                                            .map(|(_, context)| context.clone())
                                            .unwrap_or(C::Unknown),
                                        _ => C::Unknown,
                                    }
                                }
                            })
                            .collect::<Vec<_>>();
                        partial_contexts[index] = if let Some(properties) = properties.get(&index) {
                            C::Record(
                                properties
                                    .iter()
                                    .map(|field| field.ident.name.to_string())
                                    .zip(fields)
                                    .collect(),
                            )
                        } else if let Some(head) = self.protocols.tuple {
                            C::NormalTuple { head, fields }
                        } else {
                            C::Tuple(fields)
                        };
                    }
                    let scheme = self.pattern_call_scheme(&definition, &parameters, span)?;
                    let candidate = self.generator_call_candidate_with_result_context(
                        owner,
                        &parameters,
                        &mut plan,
                        &pending_defaults,
                        &deferred_clauses,
                        &mut completed_defaults,
                        &scheme,
                        &actual,
                        &partial_contexts,
                        None,
                        expected_result.as_ref(),
                        span,
                    )?;
                    input_contexts = (0..definition.inputs.len())
                        .map(|index| {
                            candidate
                                .parameter_input_context(index)
                                .map_err(|e| call_inference_error(span, e))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
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
                    input_definition =
                        self.instantiate_pattern_generator(template.clone(), &types, span)?;
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
                let candidate = self.generator_call_candidate_with_result_context(
                    owner,
                    &parameters,
                    &mut plan,
                    &pending_defaults,
                    &deferred_clauses,
                    &mut completed_defaults,
                    &scheme,
                    &actual,
                    &[],
                    None,
                    expected_result.as_ref(),
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
            let saved_captures = template
                .captures
                .iter()
                .map(|(id, value)| {
                    (
                        *id,
                        self.alias_substitution.insert(*id, Arc::new(value.clone())),
                    )
                })
                .collect::<Vec<_>>();
            let formation = self.check_type_application(owner, &types, span);
            for (id, saved) in saved_captures {
                match saved {
                    Some(value) => {
                        self.alias_substitution.insert(id, value);
                    }
                    None => {
                        self.alias_substitution.remove(&id);
                    }
                }
            }
            formation?;
            let definition = self.instantiate_pattern_generator(template, &types, span)?;
            if let Some(expected) = expected_subject {
                self.require(&definition.subject, expected, span)?;
            }
            for (index, (_, input)) in definition.inputs.iter().enumerate() {
                match (&mut supplied[index], input) {
                    (
                        Some(PatternArgument::Code(CodeValue::Expression(code))),
                        PatternInput::Expression(expected),
                    ) => {
                        *code = self.ascribe_code(code.clone(), Some(expected), span)?;
                    }
                    (
                        Some(PatternArgument::Code(CodeValue::Pattern(code))),
                        PatternInput::Code(expected),
                    ) => {
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
                this.install_generator_capture_values(&definition.captures, span)?;
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
                        (
                            PatternInput::Expression(_),
                            PatternArgument::Code(CodeValue::Expression(code)),
                        ) => {
                            this.code_values.insert(*id, code);
                        }
                        (
                            PatternInput::Code(_),
                            PatternArgument::Code(CodeValue::Pattern(code)),
                        ) => {
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
        self.generator_partial_input_hint(value, None, &[], packets, 0)
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
                        PatternInput::Expression(code) => Term::Tuple(
                            std::iter::once(code.result.clone())
                                .chain(code.context.values().cloned())
                                .collect(),
                        ),
                        PatternInput::Code(code) => Term::Tuple(
                            std::iter::once(code.subject.clone())
                                .chain(code.outputs.values().map(|(_, ty)| ty.clone()))
                                .collect(),
                        ),
                    },
                })
                .collect(),
            result: Box::new(Term::Tuple(
                std::iter::once(definition.subject.clone())
                    .chain(definition.outputs.values().cloned())
                    .collect(),
            )),
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
        let dispatched = if self.checking_code_generator {
            None
        } else {
            self.run_selected_pattern_method(owner, definition, span)?
        };
        let code = if let Some(code) = dispatched {
            self.ascribe_pattern_input(&code, &definition.interface, span)?;
            code
        } else if let Some(body) = definition.body {
            let body = self.package.body(body).unwrap().value;
            self.pattern_generator_value(body, definition, 0)?
        } else if self.checking_code_generator {
            let ItemKind::FnDeclaration(signature) = &self.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let H::ContextualPatternType(_, entries) = &signature.return_ty.unwrap().kind else {
                unreachable!()
            };
            Arc::new(PatternCode {
                subject: definition.subject.clone(),
                outputs: entries
                    .iter()
                    .map(|entry| {
                        let name = entry.name.name.to_string();
                        let ty = definition.outputs[&name].clone();
                        (name, (local(entry.hir_id), ty))
                    })
                    .collect(),
                body: None,
            })
        } else {
            return Err(error(
                span,
                "pattern generator declaration requires a selected implementation",
            ));
        };
        if !self.checking_code_generator {
            self.generator_contracts(owner, true, span, Some(&entry))?;
        }
        Ok(code)
    }

    fn run_selected_pattern_method(
        &mut self,
        owner: OwnerId,
        public: &PatternGenerator,
        span: Span,
    ) -> Result<Option<Arc<PatternCode>>, Error> {
        let Some(target) = self.code_method_targets.get(&owner).copied() else {
            return Ok(None);
        };
        self.ensure_pattern_generator(target, span)?;
        let template = self.copy_pattern_generator(target, span)?;
        let types = public
            .types
            .iter()
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        if template.types.len() != types.len() || template.inputs.len() != public.inputs.len() {
            return Err(error(span, "selected code method signature arity mismatch"));
        }
        let definition = self.instantiate_pattern_generator(template, &types, span)?;
        let saved = definition
            .captures
            .iter()
            .map(|(id, value)| {
                (
                    *id,
                    self.alias_substitution.insert(*id, Arc::new(value.clone())),
                )
            })
            .collect::<Vec<_>>();
        let checked = self.check_type_application(target, &types, span);
        for (id, value) in saved {
            match value {
                Some(value) => {
                    self.alias_substitution.insert(id, value);
                }
                None => {
                    self.alias_substitution.remove(&id);
                }
            }
        }
        checked?;
        let mut arguments = Vec::new();
        for ((binding, input), (_, expected)) in public.inputs.iter().zip(&definition.inputs) {
            let argument =
                match (input, expected) {
                    (PatternInput::Data(actual), PatternInput::Data(expected)) => {
                        self.require(actual, expected, span)?;
                        let value = self
                            .generator_data
                            .get(binding)
                            .ok_or_else(|| error(span, "missing trait code data input"))?;
                        if let Some(value) = value {
                            constants::charge_copy(value, &mut self.required_steps, span)?;
                        }
                        if value.as_ref().is_some_and(
                            |value| matches!(value.kind, K::Local(id) if id == *binding),
                        ) {
                            let cell = self
                                .generator_cells
                                .get(binding)
                                .ok_or_else(|| error(span, "missing live trait code input"))?
                                .1
                                .clone();
                            PatternArgument::LiveData(self.read_generator_cell(&cell, span)?)
                        } else {
                            PatternArgument::Data(value.clone())
                        }
                    }
                    (PatternInput::Expression(_), PatternInput::Expression(expected)) => {
                        let code =
                            self.code_values.get(binding).cloned().ok_or_else(|| {
                                error(span, "missing trait expression code input")
                            })?;
                        PatternArgument::Code(CodeValue::Expression(self.ascribe_code(
                            code,
                            Some(expected),
                            span,
                        )?))
                    }
                    (PatternInput::Code(_), PatternInput::Code(expected)) => {
                        let code = self
                            .pattern_codes
                            .get(binding)
                            .cloned()
                            .ok_or_else(|| error(span, "missing trait pattern code input"))?;
                        self.ascribe_pattern_input(
                            &code,
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
                        PatternArgument::Code(CodeValue::Pattern(code))
                    }
                    _ => return Err(error(span, "selected code method input category mismatch")),
                };
            arguments.push(argument);
        }
        self.generator_frame(target, |this| {
            this.install_generator_capture_values(&definition.captures, span)?;
            this.alias_substitution.extend(
                definition
                    .types
                    .iter()
                    .map(|(id, value)| (*id, Arc::new(value.clone()))),
            );
            this.install_generator_clause_values(
                target,
                &definition.types,
                &definition.clause_domains,
            )?;
            for ((binding, input), argument) in definition.inputs.iter().zip(arguments) {
                this.immutable.insert(*binding);
                match (input, argument) {
                    (PatternInput::Data(ty), PatternArgument::Data(value)) => {
                        this.locals.insert(*binding, ty.clone());
                        this.generator_data.insert(*binding, value);
                    }
                    (PatternInput::Data(ty), PatternArgument::LiveData(value)) => {
                        this.install_live_generator_input(*binding, ty, value, span);
                    }
                    (
                        PatternInput::Expression(_),
                        PatternArgument::Code(CodeValue::Expression(code)),
                    ) => {
                        this.code_values.insert(*binding, code);
                    }
                    (PatternInput::Code(_), PatternArgument::Code(CodeValue::Pattern(code))) => {
                        this.pattern_codes.insert(*binding, code);
                    }
                    _ => {
                        return Err(error(
                            span,
                            "selected pattern method input category mismatch",
                        ));
                    }
                }
            }
            this.run_pattern_generator_body(target, &definition, span)
        })
        .map(Some)
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
        self.rewrite_pattern_generator(
            &mut definition,
            &self.input_binders(&parameters),
            types,
            span,
        )?;
        definition.types = parameters.into_iter().zip(types.iter().cloned()).collect();
        Ok(definition)
    }

    fn rewrite_pattern_generator(
        &mut self,
        definition: &mut PatternGenerator,
        binders: &[nir::DeclarationParameter],
        types: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        let mut substitute = |ty: &Term| {
            self.copy_alias_type(ty, span)?
                .instantiate_parameters(binders, types)
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
                PatternInput::Expression(code) => {
                    PatternInput::Expression(code.try_map_types(&mut substitute)?)
                }
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
        Ok(())
    }

    fn evaluate_pattern_argument(
        &mut self,
        value: &Expr<'_>,
        input: &PatternInput,
    ) -> Result<PatternArgument, Error> {
        match input {
            PatternInput::Expression(interface) => {
                let locals = self.locals.clone();
                self.locals.extend(interface.context.clone());
                let code = self.code_value(value, Some(interface), 0);
                self.locals = locals;
                code.map(|code| PatternArgument::Code(CodeValue::Expression(code)))
            }
            PatternInput::Code(interface) => Ok(PatternArgument::Code(CodeValue::Pattern(
                self.pattern_argument(value, interface)?,
            ))),
            PatternInput::Data(ty) => {
                if self.checking_code_generator {
                    self.check_generator_data_input(value, Some(ty), None)?;
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
        definition: Option<&PatternInterface>,
        subject: Option<&Term>,
        input_context: Option<&crate::calls::InputContext>,
        depth: usize,
    ) -> Result<Arc<PatternCode>, Error> {
        use crate::program::cmtt::generators::statements::{Control, ResultInterface};
        self.generator_scope(block.span, |this| {
            let mut loops = Vec::new();
            let mut control = Control::Next;
            for statement in block.stmts {
                let next = this.generator_statement_with_result(
                    statement,
                    ResultInterface::Pattern(definition, subject, input_context),
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
                        ResultInterface::Pattern(definition, subject, input_context),
                        depth + 1,
                        &mut loops,
                    )?
                } else {
                    Control::PatternReturn(this.pattern_generator_value_inferred(
                        tail,
                        definition,
                        subject,
                        input_context,
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
                Some(first) => {
                    let mut multiple = false;
                    for other in returns {
                        this.require_pattern_branch(&first, &other, block.span)?;
                        multiple = true;
                    }
                    if multiple {
                        this.pattern_interface_evidence(first, block.span)
                    } else {
                        Ok(first)
                    }
                }
                None => {
                    let definition = definition.ok_or_else(|| {
                        error(
                            block.span,
                            "nonterminating pattern construction needs an expected interface",
                        )
                    })?;
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

    fn require_pattern_branch(
        &mut self,
        first: &PatternCode,
        other: &PatternCode,
        span: Span,
    ) -> Result<(), Error> {
        self.ascribe_pattern_input(
            other,
            &PatternInterface {
                subject: first.subject.clone(),
                outputs: first
                    .outputs
                    .iter()
                    .map(|(name, (_, ty))| (name.clone(), ty.clone()))
                    .collect(),
            },
            span,
        )
    }

    pub(in crate::program::cmtt) fn pattern_generator_value(
        &mut self,
        value: &Expr<'_>,
        definition: &PatternInterface,
        depth: usize,
    ) -> Result<Arc<PatternCode>, Error> {
        self.pattern_generator_value_inferred(
            value,
            Some(definition),
            Some(&definition.subject),
            None,
            depth,
        )
    }

    pub(in crate::program::cmtt) fn pattern_generator_value_inferred(
        &mut self,
        value: &Expr<'_>,
        definition: Option<&PatternInterface>,
        subject: Option<&Term>,
        input_context: Option<&crate::calls::InputContext>,
        depth: usize,
    ) -> Result<Arc<PatternCode>, Error> {
        if depth >= 64 || self.required_steps == 0 {
            return Err(error(
                value.span,
                "pattern generator construction budget exhausted",
            ));
        }
        self.required_steps -= 1;
        let declared_context = definition.map(Self::pattern_interface_context);
        let input_context = input_context.or(declared_context.as_ref());
        let code = match &value.kind {
            H::Block(block) => {
                return self.pattern_generator_block(
                    block,
                    definition,
                    subject,
                    input_context,
                    depth + 1,
                );
            }
            H::Return(Some(inner)) => {
                return self.pattern_generator_value_inferred(
                    inner,
                    definition,
                    subject,
                    input_context,
                    depth + 1,
                );
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
                    let result = self.pattern_generator_value_inferred(
                        &yes_expr,
                        definition,
                        subject,
                        input_context,
                        depth + 1,
                    )?;
                    let other = self.pattern_generator_value_inferred(
                        no,
                        definition,
                        subject,
                        input_context,
                        depth + 1,
                    )?;
                    self.require_pattern_branch(&result, &other, value.span)?;
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
                        comptime_interpretor::Value::Bool(true) => self
                            .pattern_generator_value_inferred(
                                &yes_expr,
                                definition,
                                subject,
                                input_context,
                                depth + 1,
                            )?,
                        comptime_interpretor::Value::Bool(false) => self
                            .pattern_generator_value_inferred(
                                no,
                                definition,
                                subject,
                                input_context,
                                depth + 1,
                            )?,
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
                return self.pattern_generator_value_inferred(
                    inner,
                    definition,
                    subject,
                    input_context,
                    depth + 1,
                );
            }
            H::When(arms) => {
                if !arms.iter().any(|arm| arm.cond.is_none()) {
                    return Err(error(
                        value.span,
                        "pattern-valued when requires an else arm",
                    ));
                }
                let mut first: Option<Arc<PatternCode>> = None;
                for arm in *arms {
                    let selected = self.generator_when_condition(arm.cond)?;
                    if self.checking_code_generator {
                        let code = self.generator_scope(arm.span, |this| {
                            this.pattern_generator_value_inferred(
                                arm.body,
                                definition,
                                subject,
                                input_context,
                                depth + 1,
                            )
                        })?;
                        if let Some(first) = &first {
                            self.require_pattern_branch(first, &code, arm.span)?;
                        } else {
                            first = Some(code);
                        }
                    } else if selected == Some(true) {
                        return self.generator_scope(arm.span, |this| {
                            this.pattern_generator_value_inferred(
                                arm.body,
                                definition,
                                subject,
                                input_context,
                                depth + 1,
                            )
                        });
                    }
                }
                let first = first
                    .ok_or_else(|| error(value.span, "pattern-valued when selected no arm"))?;
                self.pattern_interface_evidence(first, value.span)?
            }
            H::Match(scrutinee, arms) => {
                let results =
                    self.generator_match(value, scrutinee, arms, None, |this, _, body| {
                        this.pattern_generator_value_inferred(
                            body,
                            definition,
                            subject,
                            input_context,
                            depth + 1,
                        )
                    })?;
                let mut results = results.into_iter();
                let first = results.next().ok_or_else(|| {
                    error(
                        value.span,
                        "pattern generator match needs a code-producing arm",
                    )
                })?;
                for other in results {
                    self.require_pattern_branch(&first, &other, value.span)?;
                }
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
                let subject = subject.ok_or_else(|| {
                    error(
                        value.span,
                        "pattern quote requires an explicit subject interface",
                    )
                })?;
                self.build_pattern_quote(
                    value,
                    subject,
                    definition.map(|interface| &interface.outputs),
                )?
            }
            H::ExtendedApplication(callee, arguments) => self
                .invoke_pattern_properties_in_context(
                    value,
                    callee,
                    arguments,
                    subject,
                    input_context,
                )?,
            H::Application(callee, arguments) => self.invoke_pattern_generator_in_context(
                callee,
                arguments,
                subject,
                input_context,
                value.span,
            )?,
            _ => {
                return Err(error(
                    value.span,
                    "pattern generator expression is not yet implemented",
                ));
            }
        };
        if let Some(definition) = definition {
            self.ascribe_pattern_input(&code, definition, value.span)?;
        }
        if let Some(subject) = subject {
            self.require(&code.subject, subject, value.span)?;
        }
        self.validate_pattern_code(&code, value.span)?;
        Ok(code)
    }
}
