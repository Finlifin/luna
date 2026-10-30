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

impl PatternGenerator {
    fn charge(&self, fuel: &mut usize) -> Result<(), nir::type_computations::Error> {
        for ty in std::iter::once(&self.subject)
            .chain(self.outputs.values())
            .chain(self.types.iter().map(|(_, ty)| ty))
            .chain(self.clause_domains.iter())
            .chain(self.captures.iter().map(|(_, value)| value))
        {
            nir::type_computations::charge_term(ty, fuel)?;
        }
        for (_, input) in &self.inputs {
            match input {
                PatternInput::Data(ty) => nir::type_computations::charge_term(ty, fuel)?,
                PatternInput::Expression(code) => code.charge(fuel)?,
                PatternInput::Code(code) => {
                    nir::type_computations::charge_term(&code.subject, fuel)?;
                    for (name, (_, ty)) in &code.outputs {
                        *fuel = fuel
                            .checked_sub(name.len().saturating_add(1))
                            .ok_or(nir::type_computations::Error::Budget)?;
                        nir::type_computations::charge_term(ty, fuel)?;
                    }
                }
            }
        }
        for name in self.outputs.keys() {
            *fuel = fuel
                .checked_sub(name.len().saturating_add(1))
                .ok_or(nir::type_computations::Error::Budget)?;
        }
        Ok(())
    }

    // Preserve named interface types separately from the pattern-local output
    // identities used by definition checking and body elaboration.
    fn contextual_signature(
        &self,
        steps: &mut u64,
        span: Span,
    ) -> Result<nir::code_interfaces::PatternFunctionSignature, Error> {
        let mut fuel = usize::try_from(*steps).unwrap_or(usize::MAX);
        let result = self.charge(&mut fuel);
        *steps = fuel as u64;
        result.map_err(|failure| error(span, failure.to_string()))?;
        let definition = self;
        Ok(nir::code_interfaces::PatternFunctionSignature {
            input_binders: Default::default(),
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
        })
    }

    fn apply_contextual_signature(
        &mut self,
        signature: nir::code_interfaces::PatternFunctionSignature,
    ) {
        use nir::code_interfaces::CodeInput;
        self.clause_domains = signature.clause_domains;
        self.interface = signature.output;
        for ((binding, input), (new_binding, completed)) in
            self.inputs.iter_mut().zip(signature.inputs)
        {
            debug_assert_eq!(*binding, new_binding);
            *input = match (completed, &*input) {
                (CodeInput::Data(ty), PatternInput::Data(_)) => PatternInput::Data(ty),
                (CodeInput::Code(code), PatternInput::Expression(_)) => {
                    PatternInput::Expression(code)
                }
                (CodeInput::Pattern(code), PatternInput::Code(original)) => {
                    PatternInput::Code(Arc::new(PatternCode {
                        subject: code.subject,
                        outputs: code
                            .outputs
                            .into_iter()
                            .map(|(name, ty)| {
                                let binding = original.outputs[&name].0;
                                (name, (binding, ty))
                            })
                            .collect(),
                        body: None,
                    }))
                }
                _ => unreachable!("contextual input preparation preserves categories"),
            };
        }
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
        let result = definition.charge(&mut fuel);
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
        let signature = definition.contextual_signature(&mut self.required_steps, span)?;
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
            let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                &self.package.item(owner).unwrap().kind
            else {
                unreachable!()
            };
            // The inference result includes both the subject and every named
            // success output. Solver tuples are never materialized runtime data.
            let expected_result = expected_interface_context.cloned().or_else(|| {
                expected_subject.map(|subject| {
                    crate::calls::InputContext::Tuple(
                        std::iter::once(crate::calls::InputContext::Known(subject.clone()))
                            .chain(
                                template
                                    .outputs
                                    .values()
                                    .map(|_| crate::calls::InputContext::Unknown),
                            )
                            .collect(),
                    )
                })
            });
            let contextual = super::super::generators::Generator {
                signature: template.contextual_signature(&mut self.required_steps, span)?,
                captures: template.captures.clone(),
                types: template.types.clone(),
                defaults: signature
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, parameter)| match parameter.ty.kind {
                        TyParamKind::Optional(_, _, value) => Some((slot, value.hir_id)),
                        _ => None,
                    })
                    .collect(),
                variadics: signature
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, parameter)| {
                        matches!(parameter.ty.kind, TyParamKind::Varadic(..)).then_some(slot)
                    })
                    .collect(),
                body: template.body,
                checked: template.checked,
            };
            let site = Expr {
                hir_id: callee.hir_id,
                span,
                kind: H::Unit,
            };
            let (prepared, supplied) = self.prepare_contextual_inputs(
                owner,
                contextual,
                &site,
                callee,
                arguments,
                None,
                |output| {
                    Term::Tuple(
                        std::iter::once(output.subject.clone())
                            .chain(output.outputs.values().cloned())
                            .collect(),
                    )
                },
                expected_result.as_ref(),
                packets,
                prepared_types,
            )?;
            let mut definition = template;
            definition.types = prepared.types;
            definition.captures = prepared.captures;
            definition.apply_contextual_signature(prepared.signature);
            if let Some(expected) = expected_subject {
                self.require(&definition.subject, expected, span)?;
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
