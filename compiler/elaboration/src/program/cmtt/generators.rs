//! Typed, first-order comptime code calls. Definition checking uses interface
//! evidence without inventing a runnable program; invocation supplies actual
//! code witnesses before anything can enter the runtime package.
use super::*;
pub(in crate::program::cmtt) mod clauses;
mod consumers;
mod defaults;
mod inference;
mod inputs;
mod matches;
mod methods;
mod places;
mod properties;
pub(in crate::program::cmtt) mod spreads;
pub(in crate::program::cmtt) mod statements;
mod type_inputs;
use statements::Control;

use nir::code_interfaces::{CodeFunctionSignature, CodeInput as Input};
#[derive(Clone)]
pub(in crate::program) struct Generator<O = Interface> {
    pub(super) captures: Vec<(DeclarationId, Term)>,
    pub(super) types: Vec<(DeclarationId, Term)>,
    pub(super) signature: CodeFunctionSignature<O>,
    pub(super) input_values: BTreeMap<usize, Term>,
    pub(super) prepared_defaults: BTreeSet<usize>,
    pub(super) defaults: Vec<(usize, hir::HirId)>,
    pub(super) variadics: Vec<usize>,
    pub(super) body: Option<hir::BodyId>,
    pub(super) checked: bool,
}
impl<O> Generator<O> {
    pub(super) fn input_value(&self, slot: usize) -> Option<Term> {
        self.input_values.get(&slot).cloned().or_else(|| {
            self.input_binders.get(&slot).map(|binder| Term::Rigid {
                declaration: binder.declaration,
                sort: binder.sort,
            })
        })
    }
}

impl<O> std::ops::Deref for Generator<O> {
    type Target = CodeFunctionSignature<O>;
    fn deref(&self) -> &Self::Target {
        &self.signature
    }
}
impl<O> std::ops::DerefMut for Generator<O> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.signature
    }
}

pub(in crate::program::cmtt) enum Argument {
    Data(Option<n::Expr>),
    LiveData(comptime_interpretor::Value),
    Code(CodeValue),
}
// Entry snapshots copy parameter values, not the objects reached through pointers.
type GeneratorEntryData = BTreeMap<n::LocalId, (n::Expr, Option<comptime_interpretor::Storage>)>;
pub(in crate::program::cmtt) type GeneratorTypePlan = (
    Vec<DeclarationId>,
    Vec<Term>,
    Vec<(usize, Vec<DeclarationId>)>,
    Vec<usize>,
);

impl<O: fmt::Display> fmt::Display for Generator<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(code-generator (captures")?;
        for (id, value) in &self.captures {
            write!(f, " (binding {id} {value})")?;
        }
        f.write_str(") (clauses")?;
        for ((id, ty), domain) in self.types.iter().zip(&self.clause_domains) {
            write!(f, " (input {id} (domain {domain}) (value {ty}))")?;
        }
        f.write_str(") (input-binders")?;
        for (slot, binder) in &self.input_binders {
            write!(
                f,
                " (binder (parameter {slot}) (declaration {}) (sort {:?}))",
                binder.declaration, binder.sort
            )?;
        }
        f.write_str(") (input-values")?;
        for (slot, value) in &self.input_values {
            write!(f, " (input {slot} {value})")?;
        }
        f.write_str(") (prepared-defaults")?;
        for slot in &self.prepared_defaults {
            write!(f, " {slot}")?;
        }
        f.write_str(") (inputs")?;
        for (binding, input) in &self.inputs {
            match input {
                Input::Data(ty) => write!(f, " (data {binding} {ty})")?,
                Input::Code(interface) => write!(f, " (code {binding} {interface})")?,
                Input::Pattern(interface) => write!(f, " (pattern {binding} {interface})")?,
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
        write!(f, ") (output {}) (body ", self.output)?;
        match self.body {
            Some(body) => write!(f, "{}", local(body.hir_id))?,
            None => f.write_str("declaration")?,
        }
        write!(f, ") (checked {}))", self.checked)
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
        let definition = self.form_contextual_generator(owner, span, |this, output| {
            this.code_interface(
                output.ok_or_else(|| error(span, "code function requires a result annotation"))?,
            )
        })?;
        self.code_generators.insert(owner, definition);
        Ok(())
    }

    fn form_contextual_generator<O>(
        &mut self,
        owner: OwnerId,
        span: Span,
        form_output: impl FnOnce(&mut Self, Option<&Expr<'_>>) -> Result<O, Error>,
    ) -> Result<Generator<O>, Error> {
        if !self.active_signatures.insert(owner) {
            return Err(error(span, "cyclic code generator signature"));
        }
        let saved = std::mem::take(&mut self.locals);
        let saved_type_inputs = std::mem::take(&mut self.type_input_bindings);
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        self.context = self.nominal_context();
        let result = (|| {
            let item = self
                .package
                .item(owner)
                .ok_or_else(|| error(span, "missing code generator declaration"))?;
            let (sig, body) = match &item.kind {
                ItemKind::Fn(sig, body) => (sig, Some(*body)),
                ItemKind::FnDeclaration(sig) if self.self_types.contains_key(&owner) => (sig, None),
                _ => {
                    return Err(error(
                        span,
                        "a code generator declaration requires a trait contract",
                    ));
                }
            };
            if !sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head {
                return Err(error(
                    span,
                    "an expression-code result requires an explicit comptime fn",
                ));
            }
            // Keep the cached declaration symbolic. Selected invocations carry
            // their enclosing binders separately from method-local clauses.
            let mut visible_parameters = self.type_parameters[&owner].clone();
            if let Some(parent) = self.self_containers.get(&owner) {
                visible_parameters.extend(self.type_parameters[parent].iter().copied());
            }
            if let Some(Term::Rigid {
                declaration: self_parameter,
                sort: Sort::Type,
            }) = self.self_types.get(&owner)
            {
                visible_parameters.push(*self_parameter);
                if let Some((parent, _)) = self.package.owners().find(|(parent, _)| {
                    matches_declaration(
                        self.package,
                        &self.package_identities,
                        *parent,
                        *self_parameter,
                    )
                }) {
                    visible_parameters.extend(self.type_parameters[&parent].iter().copied());
                }
            }
            let mut domains = Vec::new();
            for parameter in sig.clause_params {
                // Later domains can refer to already formed value inputs.
                // The prefix is incomplete until every clause has been checked.
                self.trait_parameter_types.insert(owner, domains.clone());
                let domain = match parameter.kind {
                    hir::clause::ClauseParamKind::Type(_)
                    | hir::clause::ClauseParamKind::TraitBound(..) => p(P::Type),
                    hir::clause::ClauseParamKind::Positional(_, ty)
                    | hir::clause::ClauseParamKind::Optional(_, ty, _) => self.type_expr(ty)?,
                    _ => {
                        return Err(error(
                            parameter.span,
                            "unsupported code generator clause protocol",
                        ));
                    }
                };
                if !nir::normal_forms::supported_input_type_with_collections_in(
                    &domain,
                    &self.input_binders(&visible_parameters),
                    |head| Some(head) == self.protocols.tuple,
                    |head| Some(head) == self.protocols.meta_list,
                    |id| {
                        self.structs.contains_key(&id)
                            || self.enums.contains_key(&id)
                            || self.newtypes.contains_key(&id)
                            || self.context.is_nominal_constructor(id)
                    },
                    self.target,
                ) {
                    return Err(error(
                        parameter.span,
                        "code generator clause requires an immutable value domain",
                    ));
                }
                domains.push(domain);
            }
            for (id, domain) in self.type_parameters[&owner].iter().zip(&domains) {
                self.implementation_index
                    .register_parameter_domain(*id, domain.clone())
                    .map_err(|message| error(span, message))?;
                self.inherent_index
                    .register_parameter_domain(*id, domain.clone())
                    .map_err(|message| error(span, message))?;
            }
            self.trait_parameter_types.insert(owner, domains.clone());
            self.capabilities.entry(owner).or_default();
            self.prepare_parameter_capabilities(owner, sig.clause_params)?;
            self.ensure_contract(owner, span)?;
            // Defaults are checked in the complete generator frame below,
            // including enclosing value binders, as part of definition checking.
            self.context = self.definition_context(owner);
            let mut inputs = Vec::new();
            let mut input_binders = BTreeMap::new();
            for (slot, parameter) in sig.params.iter().enumerate() {
                let annotation = match parameter.ty.kind {
                    TyParamKind::Positional(annotation)
                    | TyParamKind::Optional(_, annotation, _)
                    | TyParamKind::Varadic(_, annotation) => Some(annotation),
                    TyParamKind::Itself { .. } => None,
                    _ => {
                        return Err(error(
                            parameter.span,
                            "code generator dependent parameter bindings are not yet implemented",
                        ));
                    }
                };
                // All generator inputs are constructed at comptime. An
                // explicit phase marker is redundant, while implicit inputs
                // use the ordinary named-supply router (no contextual capture).
                if parameter.ty.flags
                    & !(hir::common::TPARAM_IMPLICIT
                        | hir::common::TPARAM_COMPTIME
                        | hir::common::TPARAM_LAMBDA)
                    != 0
                {
                    return Err(error(
                        parameter.span,
                        "code generator parameter protocols are not yet implemented",
                    ));
                }
                let input = if annotation.is_some_and(|annotation| {
                    matches!(annotation.kind, H::ContextualPatternType(..))
                }) {
                    if matches!(parameter.ty.kind, TyParamKind::Varadic(..)) {
                        return Err(error(
                            parameter.span,
                            "pattern code collections are not yet implemented",
                        ));
                    }
                    Input::Pattern(self.pattern_input_interface(annotation.unwrap())?)
                } else if annotation
                    .is_some_and(|annotation| matches!(annotation.kind, H::ContextualExprType(..)))
                {
                    if matches!(parameter.ty.kind, TyParamKind::Varadic(..)) {
                        return Err(error(
                            parameter.span,
                            "code collections require code-container evidence not yet implemented",
                        ));
                    }
                    Input::Code(self.code_interface(annotation.unwrap())?)
                } else {
                    let ty = match annotation {
                        Some(annotation) => self.type_expr(annotation)?,
                        None => {
                            let ty = self.self_type(owner, parameter.span)?;
                            if matches!(parameter.ty.kind, TyParamKind::Itself { is_ref: true }) {
                                Term::Pointer(Box::new(ty))
                            } else {
                                ty
                            }
                        }
                    };
                    if !self.generator_data_domain(&ty, &visible_parameters) {
                        return Err(error(
                            parameter.span,
                            "code generator data parameter requires a supported immutable value domain",
                        ));
                    }
                    Input::Data(ty)
                };
                if parameter.ty.is_lambda()
                    && !matches!(&input, Input::Data(ty) if ty.is_comptime_callable())
                {
                    return Err(error(
                        parameter.span,
                        "generator lambda parameter requires a comptime callable type",
                    ));
                }
                if parameter.ty.is_comptime()
                    && matches!(&input, Input::Data(ty) if *ty == p(P::Type))
                {
                    let binder = self.fresh_generator_type_binder(owner, parameter.span)?;
                    self.type_input_bindings.insert(
                        local(parameter.hir_id),
                        Term::Rigid {
                            declaration: binder.declaration,
                            sort: binder.sort,
                        },
                    );
                    visible_parameters.push(binder.declaration);
                    input_binders.insert(slot, binder);
                }
                inputs.push((local(parameter.hir_id), input));
            }
            let output = form_output(self, sig.return_ty)?;
            Ok(Generator {
                captures: self
                    .input_binders(
                        &visible_parameters
                            .into_iter()
                            .filter(|id| {
                                !self.type_parameters[&owner].contains(id)
                                    && !input_binders
                                        .values()
                                        .any(|binder| binder.declaration == *id)
                            })
                            .collect::<BTreeSet<_>>()
                            .into_iter()
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
                    .collect(),
                types: self.type_parameters[&owner]
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
                    .collect(),
                signature: CodeFunctionSignature {
                    input_binders,
                    clause_domains: domains,
                    inputs,
                    output,
                },
                input_values: BTreeMap::new(),
                prepared_defaults: BTreeSet::new(),
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
                body,
                checked: false,
            })
        })();
        self.locals = saved;
        self.type_input_bindings = saved_type_inputs;
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.active_signatures.remove(&owner);
        result
    }

    pub(in crate::program::cmtt) fn generator_frame<R>(
        &mut self,
        owner: OwnerId,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let span = self.package.item(owner).unwrap().span;
        let roots = self
            .generator_cells
            .values()
            .map(|(_, cell)| comptime_interpretor::Value::Storage(cell.clone()))
            .collect::<Vec<_>>();
        let checkpoint = self.checkpoint_generator_cells(&roots, span)?;
        // Generator inputs use declaration identities, not a new de Bruijn
        // telescope. Retain the caller's type scope for quoted interfaces that
        // mention its already staged Type slots, without importing its locals.
        let caller_scope = self.call_inference_domains();
        let outer = (
            self.context.clone(),
            std::mem::take(&mut self.type_input_bindings),
            std::mem::take(&mut self.partial_staged_bindings),
            std::mem::take(&mut self.value_input_bindings),
            self.default_input_scope.take(),
            self.default_input_owner.take(),
            std::mem::take(&mut self.specialization_inputs),
            self.effect_environment.take(),
            self.partial_return_context.take(),
            self.return_observations.take(),
        );
        let aliases = std::mem::take(&mut self.alias_substitution);
        let clause_bindings = std::mem::take(&mut self.normal_default_bindings);
        self.default_input_scope = (!caller_scope.is_empty()).then_some(caller_scope);
        self.context = self.definition_context(owner);
        let saved_cells = std::mem::take(&mut self.generator_cells);
        let saved_patterns = std::mem::take(&mut self.pattern_codes);
        let saved_pattern_quote = self.pattern_quote_pending.take();
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
        let mut result = check(self);
        self.generator_cells = saved_cells;
        self.pattern_codes = saved_patterns;
        self.pattern_quote_pending = saved_pattern_quote;
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
            self.partial_staged_bindings,
            self.value_input_bindings,
            self.default_input_scope,
            self.default_input_owner,
            self.specialization_inputs,
            self.effect_environment,
            self.partial_return_context,
            self.return_observations,
        ) = outer;
        self.alias_substitution = aliases;
        self.normal_default_bindings = clause_bindings;
        if result.is_ok() {
            if let Err(failure) = self.refresh_generator_cells(span) {
                result = Err(failure);
            }
        }
        if result.is_err() {
            checkpoint.restore().map_err(|failure| {
                error(
                    span,
                    format!("generator call restoration failed: {failure:?}"),
                )
            })?;
        }
        result
    }

    pub(in crate::program) fn check_code_generator(&mut self, owner: OwnerId) -> Result<(), Error> {
        let definition = self.copy_code_generator(owner, self.package.item(owner).unwrap().span)?;
        if definition.checked {
            return Ok(());
        }
        // Even an unused identity generator must not admit comptime-only
        // storage through a composite runtime context or result type.
        let metadata = self.code_storage_metadata();
        for interface in definition
            .inputs
            .iter()
            .filter_map(|(_, input)| match input {
                Input::Code(interface) => Some(interface),
                Input::Data(_) | Input::Pattern(_) => None,
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
        let arguments = definition
            .inputs
            .iter()
            .map(|(_, input)| {
                Ok(match input {
                    Input::Pattern(interface) => Some(Argument::Code(CodeValue::Pattern(
                        self.abstract_pattern_input(
                            interface,
                            self.package.item(owner).unwrap().span,
                        )?,
                    ))),
                    Input::Data(_) => Some(Argument::Data(None)),
                    Input::Code(interface) => Some(Argument::Code(CodeValue::Expression(
                        Arc::new(ExpressionCode {
                            body: None,
                            result: interface.result.clone(),
                            context: interface.context.clone(),
                            ports: interface.ports.clone(),
                            defaults: BTreeMap::new(),
                        }),
                    ))),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let checking = std::mem::replace(&mut self.checking_code_generator, true);
        let result = self.generator_frame(owner, |this| {
            this.install_generator_captures(&definition, this.package.item(owner).unwrap().span)?;
            this.check_generator_clause_defaults(owner, &definition)?;
            this.run_code_generator(&definition, arguments)
        });
        self.checking_code_generator = checking;
        result?;
        self.code_generators.get_mut(&owner).unwrap().checked = true;
        Ok(())
    }

    pub(in crate::program) fn code_method_contract(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(CodeFunctionSignature, nir::contracts::ProgramContract), Error> {
        let definition = self.copy_code_generator(owner, span)?;
        let program = self.contextual_method_contract(owner, &definition, None, span)?;
        Ok((definition.signature, program))
    }

    fn contextual_method_contract<O>(
        &mut self,
        owner: OwnerId,
        definition: &Generator<O>,
        result_type: Option<&Term>,
        span: Span,
    ) -> Result<nir::contracts::ProgramContract, Error> {
        let context = self.context.clone();
        let checking = std::mem::replace(&mut self.checking_code_generator, true);
        let result = self.generator_frame(owner, |this| {
            this.context = context;
            this.install_generator_capture_values(&definition.captures, span)?;
            let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
                &this.package.item(owner).unwrap().kind
            else {
                unreachable!()
            };
            if sig.return_binding.is_some() && result_type.is_none() {
                return Err(error(
                    span,
                    "code method result bindings require code-value contract evidence not yet implemented",
                ));
            }
            this.alias_substitution.extend(
                definition.types.iter().map(|(id, value)| (*id, Arc::new(value.clone()))),
            );
            for (((id, _), domain), clause) in definition.types.iter()
                .zip(&definition.clause_domains).zip(sig.clause_params)
            {
                if *domain != p(P::Type) {
                    let binding = local(clause.hir_id);
                    this.locals.insert(binding, domain.clone());
                    this.generator_data.insert(binding, None);
                    this.normal_default_bindings.insert(*id, (binding, domain.clone()));
                }
            }
            for (slot, (binding, input)) in definition.inputs.iter().enumerate() {
                this.install_generator_type_input(*binding, definition.input_value(slot));
                match input {
                    Input::Data(ty) => {
                        this.locals.insert(*binding, ty.clone());
                        this.generator_data.insert(*binding, None);
                    }
                    Input::Pattern(interface) => {
                        let code = this.abstract_pattern_input(interface, span)?;
                        this.pattern_codes.insert(*binding, code);
                    }
                    Input::Code(interface) => {
                        this.code_values.insert(*binding, Arc::new(ExpressionCode {
                            body: None,
                            result: interface.result.clone(),
                            context: interface.context.clone(),
                            ports: interface.ports.clone(),
                            defaults: BTreeMap::new(),
                        }));
                    }
                }
            }
            let preconditions = this.program_preconditions(owner)?;
            let result_binding = sig.return_binding.map(local);
            if let Some(binding) = result_binding {
                this.generator_data.insert(binding, None);
            }
            let result = result_binding.zip(result_type);
            let (postconditions, entry_snapshots) =
                this.program_postconditions_with_result(owner, result)?;
            for snapshot in &entry_snapshots {
                if !this.generator_data.contains_key(&snapshot.parameter) {
                    return Err(error(span, "code method entry snapshot requires a data parameter"));
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
                result_binding,
            })
        });
        self.checking_code_generator = checking;
        result
    }

    fn charge_code_generator<O: nir::code_interfaces::SignatureOutput>(
        definition: &Generator<O>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
        let result = (|| {
            remaining = remaining
                .checked_sub(definition.prepared_defaults.len())
                .ok_or(nir::type_computations::Error::Budget)?;
            for (_, ty) in &definition.captures {
                nir::type_computations::charge_term(ty, &mut remaining)?;
            }
            for ty in definition.input_values.values() {
                nir::type_computations::charge_term(ty, &mut remaining)?;
            }
            for (_, ty) in &definition.types {
                nir::type_computations::charge_term(ty, &mut remaining)?;
            }
            definition.signature.charge(&mut remaining)?;
            Ok::<_, nir::type_computations::Error>(())
        })();
        *fuel = remaining as u64;
        result.map_err(|e| error(span, e.to_string()))
    }

    fn instantiate_generator_type(
        &mut self,
        ty: &Term,
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
        span: Span,
    ) -> Result<Term, Error> {
        self.instantiate_generator_type_in_scope(ty, parameters, arguments, &[], span)
    }

    fn instantiate_generator_type_in_scope(
        &mut self,
        ty: &Term,
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<Sort>],
        span: Span,
    ) -> Result<Term, Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let ids = parameters
            .iter()
            .map(|parameter| parameter.declaration)
            .collect::<Vec<_>>();
        let domains = ids
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let result =
            nir::DeclarationSubstitution::bounded(&ids, &domains, arguments, frames, &mut fuel)
                .and_then(|substitution| substitution.instantiate_bounded(ty, &mut fuel));
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| {
            error(
                span,
                format!("code generator type substitution failed: {e}"),
            )
        })
    }

    fn copy_code_generator(&mut self, owner: OwnerId, span: Span) -> Result<Generator, Error> {
        let definition = &self.code_generators[&owner];
        Self::charge_code_generator(definition, &mut self.required_steps, span)?;
        let definition = definition.clone();
        self.apply_generator_captures(owner, definition, span)
    }

    fn apply_generator_captures<O: nir::code_interfaces::SignatureOutput + Clone>(
        &mut self,
        owner: OwnerId,
        mut definition: Generator<O>,
        span: Span,
    ) -> Result<Generator<O>, Error> {
        if let Some(captures) = self.code_method_captures.get(&owner) {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let charged = captures
                .iter()
                .try_for_each(|(_, ty)| nir::type_computations::charge_term(ty, &mut fuel));
            self.required_steps = fuel as u64;
            charged.map_err(|e| error(span, e.to_string()))?;
            let parameters =
                self.input_binders(&captures.iter().map(|(id, _)| *id).collect::<Vec<_>>());
            let values: Vec<_> = captures.iter().map(|(_, value)| value.clone()).collect();
            definition.captures = captures.clone();
            Self::charge_code_generator(&definition, &mut self.required_steps, span)?;
            let mut substitute =
                |ty: &Term| self.instantiate_generator_type(ty, &parameters, &values, span);
            definition.signature = definition.signature.try_map_types(&mut substitute)?;
        }
        self.freshen_generator_signature(owner, &mut definition.signature, span)?;
        Ok(definition)
    }

    fn instantiate_code_generator<O: nir::code_interfaces::SignatureOutput + Clone>(
        &mut self,
        definition: &Generator<O>,
        types: &[Term],
        span: Span,
    ) -> Result<Generator<O>, Error> {
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
        let binders = self.input_binders(&parameters);
        let mut substitute = |ty: &Term| -> Result<Term, Error> {
            self.instantiate_generator_type(ty, &binders, types, span)
        };
        definition.signature = definition.signature.try_map_types(&mut substitute)?;
        definition.types = parameters.into_iter().zip(types.iter().cloned()).collect();
        Ok(definition)
    }

    fn generator_owner(&self, callee: &Expr<'_>) -> Option<OwnerId> {
        let callee = if let H::NFApplication(head, _) = callee.kind {
            head
        } else {
            callee
        };
        let definition = match callee.kind {
            H::ResolvedDef { def_id, .. } => def_id,
            H::Projection(..) => match self.package.member_projections.get(&callee.hir_id) {
                Some(definition) => *definition,
                None => {
                    return self
                        .static_code_method_hint(callee)
                        .filter(|owner| self.is_code_generator(*owner));
                }
            },
            _ => return None,
        };
        self.package
            .definition_owner(definition)
            .filter(|owner| self.is_code_generator(*owner))
    }

    fn generator_inference_view<O: nir::code_interfaces::SignatureOutput + Clone>(
        &mut self,
        template: &Generator<O>,
        parameters: &[DeclarationId],
        type_plan: &[Term],
        candidate: &crate::calls::CallCandidate,
        span: Span,
    ) -> Result<Generator<O>, Error> {
        let hints = parameters
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let sort = candidate
                    .type_argument_sort(index)
                    .map_err(|e| call_inference_error(span, e))?;
                candidate
                    .type_argument(index)
                    .map(|value| {
                        value.unwrap_or(Term::Rigid {
                            declaration: *id,
                            sort,
                        })
                    })
                    .map_err(|e| call_inference_error(span, e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let types = type_plan
            .iter()
            .map(|ty| {
                let binders = self.input_binders(parameters);
                self.instantiate_generator_type(ty, &binders, &hints, span)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut definition = self.instantiate_code_generator(template, &types, span)?;
        for (&slot, binder) in &template.input_binders {
            let Some(index) = parameters.iter().position(|id| *id == binder.declaration) else {
                continue;
            };
            if let Some(value) = candidate
                .type_argument(index)
                .map_err(|e| call_inference_error(span, e))?
            {
                self.specialize_generator_input(&mut definition, slot, &value, span)?;
            }
        }
        Ok(definition)
    }

    pub(super) fn invoke_property_generator(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.with_code_method_callee(callee, |this, callee, receiver| {
            let inputs = receiver
                .into_iter()
                .map(Arg::Positional)
                .chain(arguments.iter().cloned())
                .collect::<Vec<_>>();
            this.invoke_property_generator_target(site, callee, &inputs, expected, result_context)
        })
    }

    fn invoke_property_generator_target(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        if !arguments.iter().any(|arg| matches!(arg, Arg::Named(..))) {
            return self.invoke_generator_with_plan(
                site,
                callee,
                arguments,
                expected,
                result_context,
                None,
            );
        }
        let owner = self
            .generator_owner(callee)
            .ok_or_else(|| error(callee.span, "property code call requires a code generator"))?;
        self.ensure_code_generator(owner, site.span)?;
        let template = self.copy_code_generator(owner, site.span)?;
        let (collector, type_plan) = self.contextual_property_plan(
            owner,
            &template,
            site,
            callee,
            expected.map(|interface| &interface.result),
            |interface| interface.result.clone(),
        )?;
        self.with_generator_properties(site, arguments, collector, |this, inputs| {
            this.invoke_generator_with_plan(
                site,
                callee,
                inputs,
                expected,
                result_context,
                Some(type_plan),
            )
        })
    }

    pub(in crate::program::cmtt) fn with_generator_properties<R>(
        &mut self,
        site: &Expr<'_>,
        arguments: &[Arg<'_>],
        collector: hir::common::Ident,
        action: impl FnOnce(&mut Self, &[Arg<'_>]) -> Result<R, Error>,
    ) -> Result<R, Error> {
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
                    inputs.push(Arg::Named(collector, &packet));
                    inserted = true;
                }
                Arg::Named(..) => {}
                other => inputs.push(other.clone()),
            }
        }
        action(self, &inputs)
    }

    pub(super) fn invoke_code_generator(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.with_code_method_callee(callee, |this, callee, receiver| {
            let inputs = receiver
                .into_iter()
                .map(Arg::Positional)
                .chain(arguments.iter().cloned())
                .collect::<Vec<_>>();
            this.invoke_generator_with_plan(site, callee, &inputs, expected, result_context, None)
        })
    }

    fn invoke_generator_with_plan(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
        type_plan: Option<GeneratorTypePlan>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        if arguments
            .iter()
            .any(|argument| matches!(argument, Arg::Expand(_)))
        {
            return self.invoke_spread_generator(
                site,
                callee,
                arguments,
                expected,
                result_context,
                type_plan,
            );
        }
        self.invoke_code_generator_inputs(
            site,
            callee,
            arguments,
            expected,
            result_context,
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
        result_context: Option<&crate::calls::InputContext>,
        packets: &mut spreads::SpreadPlan<'_>,
        prepared_types: Option<GeneratorTypePlan>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        let owner = self.generator_owner(head).ok_or_else(|| {
            error(
                callee.span,
                "code position requires a typed comptime code generator",
            )
        })?;
        self.ensure_code_generator(owner, site.span)?;
        let template = self.copy_code_generator(owner, site.span)?;
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
        let (definition, values) = self.prepare_contextual_inputs(
            owner,
            template,
            site,
            callee,
            arguments,
            expected_type,
            output_shape,
            result_context,
            packets,
            prepared_types,
        )?;
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
            Input::Pattern(interface) => {
                let code =
                    self.pattern_input_value(value, interface, Some(&interface.subject), None)?;
                self.ascribe_pattern_input(&code, interface, value.span)?;
                Ok(Argument::Code(CodeValue::Pattern(code)))
            }
            Input::Data(ty) => {
                if self.checking_code_generator {
                    let checked = self.expr(value, Some(ty))?;
                    self.validate_generator_data(&checked, value.span)?;
                    Ok(Argument::Data(None))
                } else {
                    let (data, ty) = self.generator_data_expression(value, Some(ty))?;
                    if self.generator_value_needs_live_storage(&data, value.span)? {
                        Ok(Argument::LiveData(data))
                    } else {
                        Ok(Argument::Data(Some(
                            self.materialize_generator_data(data, ty, value.span)?,
                        )))
                    }
                }
            }
            Input::Code(interface) => {
                let locals = self.locals.clone();
                self.locals.extend(interface.context.clone());
                let code = self.code_value(value, Some(interface), 0);
                self.locals = locals;
                code.map(|code| Argument::Code(CodeValue::Expression(code)))
            }
        }
    }

    pub(in crate::program::cmtt) fn install_generator_clause_values(
        &mut self,
        owner: OwnerId,
        types: &[(DeclarationId, Term)],
        domains: &[Term],
    ) -> Result<(), Error> {
        let clauses = self.type_clause_params(owner);
        for (((id, value), domain), clause) in types.iter().zip(domains).zip(clauses.iter()) {
            if *domain == p(P::Type) {
                self.context.assume(crate::ptc::Formula::equal(
                    Term::Rigid {
                        declaration: *id,
                        sort: Sort::Type,
                    },
                    value.clone(),
                ));
                continue;
            }
            let binding = local(clause.hir_id);
            self.locals.insert(binding, domain.clone());
            self.immutable.insert(binding);
            self.normal_default_bindings
                .insert(*id, (binding, domain.clone()));
            let data = if self.checking_code_generator {
                None
            } else {
                let site = Expr {
                    hir_id: clause.hir_id,
                    span: clause.span,
                    kind: H::Unit,
                };
                let input = n::Expr {
                    source: source(clause.span),
                    ty: domain.clone(),
                    kind: K::Local(binding),
                };
                let (data, ty) = self.required_normal_input(
                    &site,
                    input,
                    vec![(binding, domain.clone(), value.clone())],
                )?;
                Some(self.materialize_generator_data(data, ty, clause.span)?)
            };
            self.generator_data.insert(binding, data);
        }
        Ok(())
    }

    fn run_code_generator(
        &mut self,
        definition: &Generator,
        arguments: Vec<Option<Argument>>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.install_generator_captures(
            definition,
            self.package
                .item(self.signature_owner.unwrap())
                .unwrap()
                .span,
        )?;
        self.alias_substitution.extend(
            definition
                .types
                .iter()
                .map(|(id, ty)| (*id, Arc::new(ty.clone()))),
        );
        let owner = self.signature_owner.expect("generator frame");
        let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
            &self.package.item(owner).unwrap().kind
        else {
            unreachable!()
        };
        self.install_generator_clause_values(owner, &definition.types, &definition.clause_domains)?;
        self.locals.extend(definition.output.context.clone());
        let mut inherited: BTreeMap<n::LocalId, Option<n::LocalId>> = BTreeMap::new();
        for (index, ((id, input), argument)) in definition.inputs.iter().zip(arguments).enumerate()
        {
            let default = match signature.params[index].ty.kind {
                TyParamKind::Optional(_, _, value) => Some(value),
                _ => None,
            };
            // Every definition checks its defaults even when all calls override
            // them. Invocation evaluates only omitted inputs, in prefix order.
            let from_default = argument.is_none() || definition.prepared_defaults.contains(&index);
            let supplied = if self.checking_code_generator || argument.is_none() {
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
            self.install_generator_type_input(*id, definition.input_value(index));
            match (input, argument) {
                (Input::Pattern(interface), Argument::Code(CodeValue::Pattern(code))) => {
                    self.ascribe_pattern_input(&code, interface, signature.span)?;
                    self.pattern_codes.insert(*id, code);
                }
                (Input::Data(ty), Argument::Data(value)) => {
                    self.locals.insert(*id, ty.clone());
                    self.generator_data.insert(*id, value);
                }
                (Input::Data(ty), Argument::LiveData(value)) => {
                    self.install_live_generator_input(*id, ty, value, signature.params[index].span);
                }
                (Input::Code(interface), Argument::Code(CodeValue::Expression(code))) => {
                    let mut defaults = code.defaults.clone();
                    for (name, from) in &interface.ports {
                        if let Some(to) = definition.output.ports.get(name) {
                            self.require(
                                &interface.context[from],
                                &definition.output.context[to],
                                signature.span,
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
        // Contracts see the completed input frame, including defaults, but
        // never gain access to the quoted runtime context. Definition checking
        // validates them without invoking predicates on abstract data inputs.
        self.generator_contracts(owner, false, signature.span, None)?;
        let entry = self.capture_generator_entry_data(owner, signature.span)?;
        if self.checking_code_generator {
            self.generator_contracts(owner, true, signature.span, Some(&entry))?;
        }
        let dispatched = if !self.checking_code_generator {
            self.run_selected_code_method(owner, definition, signature.span)?
        } else {
            None
        };
        let mut output = definition.output.clone();
        output.defaults = inherited
            .iter()
            .filter_map(|(port, actual)| actual.map(|actual| (*port, actual)))
            .collect();
        let code = if let Some(code) = dispatched {
            self.ascribe_code(code, Some(&output), signature.span)?
        } else {
            let Some(body_id) = definition.body else {
                if !self.checking_code_generator {
                    return Err(error(
                        signature.span,
                        "code generator declaration requires a selected implementation",
                    ));
                }
                return Ok(Arc::new(ExpressionCode {
                    body: None,
                    result: definition.output.result.clone(),
                    context: definition.output.context.clone(),
                    ports: definition.output.ports.clone(),
                    defaults: BTreeMap::new(),
                }));
            };
            let body = self
                .package
                .body(body_id)
                .ok_or_else(|| error(signature.span, "missing generator body"))?;
            self.code_value(body.value, Some(&output), 0)?
        };
        if !self.checking_code_generator {
            self.generator_contracts(owner, true, signature.span, Some(&entry))?;
        }
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

    pub(in crate::program::cmtt) fn install_live_generator_input(
        &mut self,
        id: n::LocalId,
        ty: &Term,
        value: comptime_interpretor::Value,
        span: Span,
    ) {
        self.locals.insert(id, ty.clone());
        let marker = n::Expr {
            source: source(span),
            ty: ty.clone(),
            kind: K::Local(id),
        };
        self.generator_data.insert(id, Some(marker.clone()));
        self.generator_cells
            .insert(id, (marker, comptime_interpretor::Storage::new(value)));
    }

    pub(in crate::program::cmtt) fn capture_generator_entry_data(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<GeneratorEntryData, Error> {
        let mut entry = GeneratorEntryData::new();
        if !self.checking_code_generator && !self.program_postconditions[&owner].is_empty() {
            for (id, value) in &self.generator_data {
                let Some(value) = value else { continue };
                constants::charge_copy(value, &mut self.required_steps, span)?;
                let cell = if matches!(value.kind, K::Local(binding) if binding == *id) {
                    let cell = &self
                        .generator_cells
                        .get(id)
                        .ok_or_else(|| error(span, "missing live generator entry storage"))?
                        .1;
                    let package = n::Package::default();
                    let mut vm = comptime_interpretor::Interpreter::for_comptime(
                        &package,
                        comptime_interpretor::Budget {
                            steps: self.required_steps,
                            depth: 128,
                        },
                    )
                    .map_err(|failure| {
                        error(
                            span,
                            format!("generator entry preparation failed: {failure}"),
                        )
                    })?;
                    let data = vm.copy_owned_value(cell);
                    self.required_steps = vm.remaining_steps();
                    let data = data.map_err(|failure| {
                        error(span, format!("generator entry snapshot failed: {failure}"))
                    })?;
                    Some(comptime_interpretor::Storage::new(data))
                } else {
                    None
                };
                entry.insert(*id, (value.clone(), cell));
            }
        }
        Ok(entry)
    }

    pub(in crate::program::cmtt) fn generator_contracts(
        &mut self,
        owner: OwnerId,
        post: bool,
        span: Span,
        entry: Option<&GeneratorEntryData>,
    ) -> Result<(), Error> {
        self.generator_contracts_with_result(owner, post, span, entry, None)
    }

    fn generator_contracts_with_result(
        &mut self,
        owner: OwnerId,
        post: bool,
        span: Span,
        entry: Option<&GeneratorEntryData>,
        result: Option<(n::LocalId, &Term)>,
    ) -> Result<(), Error> {
        if if post {
            self.program_postconditions[&owner].is_empty()
        } else {
            self.program_requirements[&owner].is_empty()
        } {
            return Ok(());
        }
        self.required_steps = self
            .required_steps
            .checked_sub(self.generator_data.len() as u64)
            .ok_or_else(|| error(span, "generator contract input budget exhausted"))?;
        let concrete = std::mem::take(&mut self.generator_data);
        self.generator_data
            .extend(concrete.keys().map(|binding| (*binding, None)));
        let checked = (|| {
            if let Some((binding, _)) = result {
                self.generator_data.insert(binding, None);
            }
            let (predicates, snapshots) = if post {
                self.program_postconditions_with_result(owner, result)?
            } else {
                (self.program_preconditions(owner)?, Vec::new())
            };
            for snapshot in &snapshots {
                if !self.generator_data.contains_key(&snapshot.parameter) {
                    return Err(error(
                        span,
                        "generator entry snapshot requires a data parameter",
                    ));
                }
                self.generator_data.insert(snapshot.binding, None);
            }
            for predicate in &predicates {
                self.validate_generator_data(predicate, span)?;
            }
            Ok((predicates, snapshots))
        })();
        self.generator_data = concrete;
        let (predicates, snapshots) = checked?;
        if self.checking_code_generator {
            return Ok(());
        }
        let snapshots: BTreeMap<_, _> = snapshots
            .into_iter()
            .map(|snapshot| (snapshot.binding, snapshot.parameter))
            .collect();
        let sources = if post {
            self.program_postconditions[&owner].clone()
        } else {
            self.program_requirements[&owner].clone()
        };
        for (source, mut predicate) in sources.into_iter().zip(predicates) {
            // Parameters are immutable completed data. Substitute only after
            // checking the abstract predicate, so literal record/enum storage
            // does not change which operations qualify as a program predicate.
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let substituted =
                nir::visit::expression_mut_pruned(&mut predicate, &mut fuel, |node, fuel| {
                    let K::Local(binding) = node.kind else {
                        return Ok(true);
                    };
                    let old = snapshots.get(&binding).copied();
                    let value = if let Some(parameter) = old {
                        &entry
                            .and_then(|entry| entry.get(&parameter))
                            .ok_or_else(|| "missing generator entry snapshot".to_string())?
                            .0
                    } else {
                        let Some(Some(value)) = self.generator_data.get(&binding) else {
                            return Ok(true);
                        };
                        value
                    };
                    let mut remaining = *fuel as u64;
                    constants::charge_copy(value, &mut remaining, source.span)
                        .map_err(|error| error.message)?;
                    *fuel = usize::try_from(remaining).unwrap_or(usize::MAX);
                    *node = value.clone();
                    if old.is_some() && matches!(node.kind, K::Local(_)) {
                        node.kind = K::Local(binding);
                    }
                    if matches!(node.kind, K::Block { .. }) {
                        self.freshen_constant_graph(node, source.span)
                            .map_err(|error| error.message)?;
                    }
                    Ok(false)
                });
            self.required_steps = self
                .required_steps
                .checked_sub((before - fuel) as u64)
                .ok_or_else(|| {
                    error(
                        source.span,
                        "generator contract substitution budget exhausted",
                    )
                })?;
            substituted.map_err(|message| error(source.span, message))?;
            let mut live: Vec<_> = self
                .generator_cells
                .iter()
                .filter(|(id, (snapshot, _))| matches!(snapshot.kind, K::Local(binding) if binding == **id))
                .map(|(id, (snapshot, cell))| {
                    (
                        *id,
                        snapshot.ty.clone(),
                        comptime_interpretor::Value::Storage(cell.clone()),
                    )
                })
                .collect();
            for (binding, parameter) in &snapshots {
                if let Some((value, Some(cell))) = entry.and_then(|entry| entry.get(parameter)) {
                    live.push((
                        *binding,
                        value.ty.clone(),
                        comptime_interpretor::Value::Storage(cell.clone()),
                    ));
                }
            }
            let (value, _) = self.required_live_input(source, predicate, Vec::new(), live)?;
            if !matches!(value, comptime_interpretor::Value::Bool(true)) {
                return Err(error(
                    source.span,
                    if post {
                        "code generator postcondition rejected"
                    } else {
                        "code generator precondition rejected"
                    },
                ));
            }
        }
        Ok(())
    }

    pub(super) fn require_code_branch(
        &mut self,
        reference: &ExpressionCode,
        code: Arc<ExpressionCode>,
        span: Span,
    ) -> Result<(), Error> {
        self.require_expression(&code.result, &reference.result, span)?;
        if code.context == reference.context && code.ports == reference.ports {
            return Ok(());
        }
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let charged = reference
            .context
            .values()
            .chain(std::iter::once(&reference.result))
            .try_for_each(|ty| nir::type_computations::charge_term(ty, &mut remaining));
        self.required_steps -= (before - remaining) as u64;
        charged.map_err(|e| error(span, e.to_string()))?;
        let work = reference
            .context
            .len()
            .saturating_add(reference.defaults.len())
            .saturating_add(
                reference
                    .ports
                    .keys()
                    .map(|name| name.len().saturating_add(1))
                    .sum::<usize>(),
            );
        self.required_steps = self
            .required_steps
            .checked_sub(work as u64)
            .ok_or_else(|| error(span, "code branch interface budget exhausted"))?;
        let expected = Interface {
            infer_context: false,
            context: reference.context.clone(),
            ports: reference.ports.clone(),
            defaults: reference.defaults.clone(),
            result: reference.result.clone(),
        };
        self.ascribe_code(code, Some(&expected), span)?;
        Ok(())
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

    pub(in crate::program) fn code_producer(&mut self, value: &Expr<'_>) -> bool {
        self.code_producer_in(value, &mut BTreeSet::new(), 0)
    }

    fn code_producer_in(
        &mut self,
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
                    || self
                        .generic_code_method_hint(callee)
                        .is_some_and(|owner| self.is_code_generator(owner))
            }
            H::Let(decl) => {
                if decl
                    .init
                    .is_some_and(|value| self.code_producer_in(value, bindings, depth + 1))
                    || decl
                        .ty
                        .is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..)))
                {
                    bindings.extend(pattern_binding_ids(&decl.pattern));
                }
                false
            }
            H::Return(Some(value)) | H::Semi(value) | H::InlineControl(value) => {
                self.code_producer_in(value, bindings, depth + 1)
            }
            H::If(_, yes, no) => {
                let yes = self.code_block_producer(yes, bindings, depth + 1);
                let no = no.is_some_and(|value| self.code_producer_in(value, bindings, depth + 1));
                yes || no
            }
            H::Block(body) | H::Loop(body) | H::For { body, .. } => {
                self.code_block_producer(body, bindings, depth + 1)
            }
            H::When(arms) => arms
                .iter()
                .any(|arm| self.code_producer_in(arm.body, bindings, depth + 1)),
            H::Match(_, arms) => arms
                .iter()
                .any(|arm| self.code_producer_in(arm.body, bindings, depth + 1)),
            _ => false,
        }
    }

    fn code_block_producer(
        &mut self,
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
        result_context: Option<&crate::calls::InputContext>,
        depth: usize,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.generator_scope(block.span, |this| {
            let mut loops = Vec::new();
            let mut control = Control::Next;
            for statement in block.stmts {
                let next = this.generator_statement(statement, interface, result_context, depth + 1, &mut loops)?;
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
                let control_expr = match value.kind {
                    H::InlineControl(inner) => inner,
                    _ => value,
                };
                let tail = if matches!(
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
                ) || matches!(control_expr.kind, H::For {..})
                    || matches!(control_expr.kind, H::When(arms) if !arms.iter().any(|arm| arm.cond.is_none()))
                    || matches!(control_expr.kind, H::Match(..) | H::When(..)) && !this.code_producer(value)
                {
                    this.generator_statement(value, interface, result_context, depth + 1, &mut loops)?
                } else {
                    Control::Return(this.code_value_in_context(value, interface, result_context, depth + 1)?)
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
                    this.require_code_branch(&first, code, block.span)?;
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

    pub(super) fn generator_scope<R>(
        &mut self,
        span: Span,
        action: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let work = self
            .locals
            .len()
            .saturating_add(self.code_values.len())
            .saturating_add(self.pattern_codes.len())
            .saturating_add(self.generator_data.len())
            .saturating_add(self.immutable.len());
        self.required_steps = self
            .required_steps
            .checked_sub(work as u64)
            .ok_or_else(|| error(span, "generator scope snapshot budget exhausted"))?;
        for value in self.generator_data.values().flatten() {
            constants::charge_copy(value, &mut self.required_steps, span)?;
        }
        for (snapshot, _) in self.generator_cells.values() {
            constants::charge_copy(snapshot, &mut self.required_steps, span)?;
        }
        let saved_cells = self.generator_cells.clone();
        let roots = saved_cells
            .values()
            .map(|(_, cell)| comptime_interpretor::Value::Storage(cell.clone()))
            .collect::<Vec<_>>();
        let checkpoint = self.checkpoint_generator_cells(&roots, span)?;
        let mut saved = (
            self.locals.clone(),
            self.code_values.clone(),
            self.generator_data.clone(),
            self.immutable.clone(),
            self.pattern_codes.clone(),
        );
        let result = action(self);
        if result.is_ok() {
            for (id, value) in &mut saved.1 {
                if let Some(current) = self.code_values.get(id) {
                    *value = current.clone();
                }
            }
            for (id, value) in &mut saved.4 {
                if let Some(current) = self.pattern_codes.get(id) {
                    *value = current.clone();
                }
            }
            for (id, value) in &mut saved.2 {
                if let Some(current) = self.generator_data.get(id) {
                    *value = current.clone();
                }
            }
        }
        let restoration = if result.is_err() {
            self.generator_cells = saved_cells;
            checkpoint.restore().map_err(|failure| {
                error(
                    span,
                    format!("generator scope restoration failed: {failure:?}"),
                )
            })
        } else {
            self.generator_cells
                .retain(|id, _| saved.2.contains_key(id));
            Ok(())
        };
        (
            self.locals,
            self.code_values,
            self.generator_data,
            self.immutable,
            self.pattern_codes,
        ) = saved;
        restoration?;
        result
    }

    pub(super) fn generator_binding(&mut self, decl: &hir::decl::LetDecl<'_>) -> Result<(), Error> {
        self.data_binding(decl, false)
    }

    /// Execute matching and its guard once at comptime, exporting only the
    /// successful binding packet. The source else remains runtime control flow.
    pub(super) fn conditional_local_data_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
    ) -> Result<n::Expr, Error> {
        let init = decl
            .init
            .ok_or_else(|| error(decl.span, "local const requires an initializer"))?;
        let otherwise = decl
            .otherwise
            .map(|body| self.check_failure_branch(body))
            .transpose()?;
        if otherwise.as_ref().is_some_and(|body| !is_never(&body.ty)) {
            return Err(error(decl.span, "const else branch must diverge"));
        }
        let annotation = decl.ty.map(|ty| self.type_expr(ty)).transpose()?;
        let (data, ty) = self.required_typed_expression(init, annotation.as_ref())?;
        let value = self.materialize_local_constant(data, ty.clone(), init.span)?;
        self.immutable.extend(pattern_binding_ids(&decl.pattern));
        let observed = self.known_pattern_type_value(&value);
        let pattern = self.with_staged_input(true, |this| {
            this.pattern_observed(&decl.pattern, &ty, observed.as_ref())
        })?;
        let bindings = pattern_bindings(&decl.pattern)?
            .into_values()
            .map(|id| {
                self.locals
                    .get(&id)
                    .cloned()
                    .map(|ty| (id, ty))
                    .ok_or_else(|| error(decl.span, "constant pattern output has no type"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let staged = bindings.iter().map(|(id, _)| *id).collect::<BTreeSet<_>>();
        self.check_inline_pattern_availability(&pattern, decl.pattern.span)?;
        let guard = decl
            .guard
            .map(|guard| {
                let checked =
                    self.with_staged_input(true, |this| this.expr(guard, Some(&p(P::Bool))))?;
                self.check_inline_input_availability(&checked, &staged, guard.span)?;
                Ok(checked)
            })
            .transpose()?;
        if otherwise.is_none() && (guard.is_some() || !self.pattern_exhaustive(&pattern, &ty)) {
            return Err(error(
                decl.span,
                "refutable const pattern requires a diverging else branch",
            ));
        }
        let packet_ty = self.tuple_value_type(
            bindings.iter().map(|(_, ty)| ty.clone()).collect(),
            decl.span,
        )?;
        let result_ty = Term::ErrorResult {
            errors: vec![p(P::Void)],
            result: Box::new(packet_ty.clone()),
        };
        let packet = n::Expr {
            source: source(decl.span),
            ty: packet_ty,
            kind: K::Tuple(
                bindings
                    .iter()
                    .map(|(id, ty)| n::Expr {
                        source: source(decl.span),
                        ty: ty.clone(),
                        kind: K::Local(*id),
                    })
                    .collect(),
            ),
        };
        let success = n::Expr {
            source: source(decl.span),
            ty: result_ty.clone(),
            kind: K::ErrorConvert {
                conversion: nir::error_results::Conversion::Success,
                value: Box::new(packet),
            },
        };
        let failure = n::Expr {
            source: source(decl.span),
            ty: result_ty.clone(),
            kind: K::ErrorConvert {
                conversion: nir::error_results::Conversion::Failure,
                value: Box::new(n::Expr {
                    source: source(decl.span),
                    ty: p(P::Void),
                    kind: K::Unit,
                }),
            },
        };
        let root = n::Expr {
            source: source(decl.span),
            ty: result_ty,
            kind: K::Match {
                value: Box::new(value),
                arms: vec![
                    n::MatchArm {
                        pattern,
                        guard,
                        body: success,
                    },
                    n::MatchArm {
                        pattern: n::Pattern::Wild,
                        guard: None,
                        body: failure,
                    },
                ],
            },
        };
        let (selected, _) = self.required_normal_input(init, root, vec![])?;
        let comptime_interpretor::Value::ErrorResult {
            error: selected_error,
            value: selected,
            ..
        } = selected
        else {
            return Err(error(
                decl.span,
                "invalid constant pattern selection packet",
            ));
        };
        if selected_error.is_some() {
            return otherwise
                .ok_or_else(|| error(decl.span, "constant pattern failed without an else branch"));
        }
        let comptime_interpretor::Value::Tuple(values) = *selected else {
            return Err(error(decl.span, "invalid constant pattern binding packet"));
        };
        if bindings.len() != values.len() {
            return Err(error(decl.span, "constant pattern output arity mismatch"));
        }
        for ((binding, ty), value) in bindings.into_iter().zip(values) {
            let value = self.materialize_local_constant(value, ty, decl.span)?;
            self.generator_data.insert(binding, Some(value));
        }
        Ok(n::Expr {
            source: source(decl.span),
            ty: p(P::Void),
            kind: K::Unit,
        })
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
        if self.pattern_code_producer(value, decl.ty) {
            self.pattern_code_binding_mode(decl, None, !local_const)?;
        } else if self.code_producer(value)
            || decl
                .ty
                .is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..)))
        {
            self.code_binding(decl, None, true)?;
        } else {
            let expected = decl.ty.map(|ty| self.type_expr(ty)).transpose()?;
            let (ty, value) = if self.checking_code_generator {
                let checked = self.expr(value, expected.as_ref())?;
                self.validate_generator_data(&checked, value.span)?;
                (checked.ty.clone(), checked)
            } else {
                let (data, ty) = if local_const {
                    self.required_typed_expression(value, expected.as_ref())?
                } else {
                    self.generator_data_expression(value, expected.as_ref())?
                };
                let simple_binding = matches!(
                    decl.pattern.kind,
                    hir::pattern::PatternKind::Binding(_, _, None)
                        | hir::pattern::PatternKind::Wild
                );
                if !local_const
                    && (!simple_binding
                        || self.generator_value_needs_live_storage(&data, value.span)?)
                {
                    return self.bind_live_generator_pattern(decl, data, ty);
                }
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
            return self.assign_generator_place(target, value, None);
        }
        let id = binding(target)?;
        if self.immutable.contains(&id) {
            return Err(error(
                target.span,
                "cannot assign an immutable generator binding",
            ));
        }
        if let Some(previous) = self.pattern_codes.get(&id).cloned() {
            let code = self.pattern_argument(value, &previous)?;
            // Abstract control may visit several possible writes. Preserve the
            // interface until invocation selects and executes the actual path.
            let code = if self.checking_code_generator {
                self.pattern_interface_evidence(code, value.span)?
            } else {
                code
            };
            self.pattern_codes.insert(id, code);
            return Ok(());
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
            self.assign_generator_place(target, value, None)?;
        } else {
            return Err(error(
                target.span,
                "generator assignment cannot modify a runtime context binding",
            ));
        }
        Ok(())
    }

    pub(in crate::program::cmtt) fn generator_data_domain(
        &self,
        ty: &Term,
        parameters: &[DeclarationId],
    ) -> bool {
        let mut pending = vec![(ty, 0usize)];
        let mut remaining = 100_000usize;
        while let Some((ty, depth)) = pending.pop() {
            if depth >= 128 || remaining == 0 {
                return false;
            }
            remaining -= 1;
            match ty {
                Term::ErrorResult { errors, result } => {
                    pending.extend(errors.iter().map(|ty| (ty, depth + 1)));
                    if !is_never(result) {
                        pending.push((result, depth + 1));
                    }
                }
                Term::Optional(inner) | Term::Pointer(inner) => pending.push((inner, depth + 1)),
                Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_) => {}
                Term::Closure {
                    environment,
                    signature,
                } if signature.is_comptime_callable() => {
                    pending.extend(environment.iter().map(|ty| (ty, depth + 1)));
                }

                Term::Tuple(fields) => pending.extend(fields.iter().map(|ty| (ty, depth + 1))),
                Term::Record(fields) => {
                    pending.extend(fields.iter().map(|(_, ty)| (ty, depth + 1)))
                }
                Term::NormalForm {
                    head,
                    sort: Sort::Type,
                    ..
                } if Some(*head) == self.protocols.tuple => {
                    let Some(fields) = self.bound_tuple_elements(ty) else {
                        return false;
                    };
                    pending.extend(fields.iter().map(|ty| (ty, depth + 1)));
                }
                _ if nir::normal_forms::supported_input_type_with_collections_in(
                    ty,
                    &self.input_binders(parameters),
                    |head| Some(head) == self.protocols.tuple,
                    |head| Some(head) == self.protocols.meta_list,
                    |declaration| {
                        self.structs.contains_key(&declaration)
                            || self.enums.contains_key(&declaration)
                            || self.newtypes.contains_key(&declaration)
                            || self.context.is_nominal_constructor(declaration)
                    },
                    self.target,
                ) => {}
                _ => return false,
            }
            if pending.len() > remaining {
                return false;
            }
        }
        true
    }

    pub(super) fn validate_generator_data(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let mut may_change_schema = false;
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
                    may_change_schema |= matches!(
                        expr.kind,
                        K::AddressOf(_) | K::Assign { .. } | K::Call { .. }
                    );
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
        if self.checking_code_generator && may_change_schema {
            self.forget_generator_schema_evidence(span)?;
        }
        result.map_err(|message| error(span, message))
    }

    pub(in crate::program) fn forget_generator_schema_evidence(
        &mut self,
        span: Span,
    ) -> Result<(), Error> {
        // A let binding cannot be rebound, but its fields may still change.
        // Until the abstract interpreter tracks places and aliases precisely,
        // discard structural snapshots at calls, writes and address escape.
        self.required_steps = self
            .required_steps
            .checked_sub(self.generator_data.len() as u64)
            .ok_or_else(|| error(span, "generator schema invalidation budget exhausted"))?;
        // Structural observations carry no alias proof. Invalidate them with
        // the materialized schema snapshots at writes, calls and escapes.
        self.partial_staged_bindings.clear();
        for value in self.generator_data.values_mut() {
            if value.as_ref().is_some_and(|value| {
                matches!(
                    &value.kind,
                    K::ComptimeValue(
                        Term::RecordValue(_) | Term::TupleValue(_) | Term::NominalValue { .. }
                    )
                )
            }) {
                *value = None;
            }
        }
        Ok(())
    }

    fn bind_live_generator_pattern(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        value: comptime_interpretor::Value,
        ty: Term,
    ) -> Result<(), Error> {
        let input = self.spread_binding(decl.span)?;
        let marker = n::Expr {
            source: source(decl.span),
            ty: ty.clone(),
            kind: K::Local(input),
        };
        self.locals.insert(input, ty.clone());
        self.generator_data.insert(input, Some(marker.clone()));
        self.generator_cells.insert(
            input,
            (marker.clone(), comptime_interpretor::Storage::new(value)),
        );
        let mut bindings = Vec::new();
        // Check and execute embedded pattern computations in the same frame
        // as surrounding construction data. Even a scalar initializer may
        // produce a live pointer or mutate an outer binding during matching.
        let result = self.generator_data_frame_with(decl.init.unwrap(), |this| {
            if !decl.mutable {
                this.immutable.extend(pattern_binding_ids(&decl.pattern));
            }
            let pattern = this.pattern(&decl.pattern, &ty)?;
            if !this.pattern_exhaustive(&pattern, &ty) {
                return Err(error(
                    decl.span,
                    "refutable generator let pattern requires a diverging else branch",
                ));
            }
            bindings = pattern_bindings(&decl.pattern)?
                .into_values()
                .map(|id| (id, this.locals[&id].clone()))
                .collect();
            let packet_type = this.tuple_value_type(
                bindings.iter().map(|(_, ty)| ty.clone()).collect(),
                decl.span,
            )?;
            Ok(n::Expr {
                source: source(decl.span),
                ty: packet_type.clone(),
                kind: K::Block {
                    statements: vec![n::Expr {
                        source: source(decl.span),
                        ty: p(P::Void),
                        kind: K::LetPattern {
                            pattern,
                            value: Box::new(marker),
                            guard: None,
                            otherwise: None,
                        },
                    }],
                    result: Some(Box::new(n::Expr {
                        source: source(decl.span),
                        ty: packet_type,
                        kind: K::Tuple(
                            bindings
                                .iter()
                                .map(|(id, ty)| n::Expr {
                                    source: source(decl.span),
                                    ty: ty.clone(),
                                    kind: K::Local(*id),
                                })
                                .collect(),
                        ),
                    })),
                },
            })
        });
        self.locals.remove(&input);
        self.generator_data.remove(&input);
        self.generator_cells.remove(&input);
        let (values, _) = result?.ok_or_else(|| {
            error(
                decl.span,
                "live generator pattern requires a completed invocation",
            )
        })?;
        let comptime_interpretor::Value::Tuple(values) = values else {
            return Err(error(
                decl.span,
                "live generator pattern produced an invalid packet",
            ));
        };
        if values.len() != bindings.len() {
            return Err(error(decl.span, "live generator pattern arity mismatch"));
        }
        let mut completed = Vec::new();
        for ((id, ty), value) in bindings.into_iter().zip(values) {
            let (snapshot, cell) = if self.generator_value_needs_live_storage(&value, decl.span)? {
                (
                    n::Expr {
                        source: source(decl.span),
                        ty,
                        kind: K::Local(id),
                    },
                    Some(comptime_interpretor::Storage::new(value)),
                )
            } else {
                (self.materialize_generator_data(value, ty, decl.span)?, None)
            };
            constants::charge_copy(&snapshot, &mut self.required_steps, decl.span)?;
            completed.push((id, snapshot, cell));
        }
        for (id, snapshot, cell) in completed {
            self.generator_data.insert(id, Some(snapshot.clone()));
            if let Some(cell) = cell {
                self.generator_cells.insert(id, (snapshot, cell));
            }
        }
        Ok(())
    }

    /// Project only the explicit structure of a known type description. A
    /// normal-form head is never invoked to discover additional structure.
    fn known_type_pattern_outputs(
        &mut self,
        pattern: &n::Pattern,
        ty: &Term,
        span: Span,
    ) -> Result<BTreeMap<n::LocalId, n::Expr>, Error> {
        let mut result = BTreeMap::new();
        let mut pending = vec![(pattern, ty)];
        while let Some((pattern, value)) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "type pattern evidence budget exhausted"))?;
            match (pattern, value) {
                (n::Pattern::Binding { binding, inner }, value) => {
                    if value.sort() == Sort::Type && self.locals.get(binding) == Some(&p(P::Type)) {
                        let expression = n::Expr {
                            source: source(span),
                            ty: p(P::Type),
                            kind: K::TypeValue(value.clone()),
                        };
                        constants::charge_copy(&expression, &mut self.required_steps, span)?;
                        result.insert(*binding, expression);
                    }
                    pending.push((inner, value));
                }
                (
                    n::Pattern::NormalForm { head, arguments },
                    Term::NormalForm {
                        head: actual,
                        arguments: values,
                        ..
                    },
                ) if head == actual && arguments.len() == values.len() => {
                    pending.extend(arguments.iter().zip(values));
                }
                (n::Pattern::TypeTuple(patterns), value) => {
                    let fields = match value {
                        Term::Tuple(fields) => Some(fields.as_slice()),
                        _ => self.bound_tuple_elements(value),
                    };
                    if let Some(fields) = fields.filter(|fields| fields.len() == patterns.len()) {
                        pending.extend(patterns.iter().zip(fields));
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }

    pub(in crate::program) fn known_pattern_types(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<BTreeMap<n::LocalId, n::Expr>, Error> {
        let mut known_types = BTreeMap::new();
        // Retain Type witnesses already present in checked syntax. This
        // is structural projection, not invocation or value sampling.
        let mut pending = vec![(pattern, value)];
        while let Some((pattern, value)) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "generator type evidence budget exhausted"))?;
            let mut value = value;
            loop {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "generator type projection budget exhausted"))?;
                let projected = match &value.kind {
                    K::Field {
                        value: base,
                        declaration,
                        index,
                    } => match &base.kind {
                        K::Record {
                            declaration: actual,
                            fields,
                        } if declaration == actual => fields.get(*index),
                        _ => None,
                    },
                    K::AnonymousField {
                        value: base,
                        name,
                        index,
                    } => match (&base.kind, &base.ty) {
                        (K::AnonymousRecord { fields }, Term::Record(types))
                            if types.get(*index).is_some_and(|(actual, _)| name == actual) =>
                        {
                            fields.get(*index)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let Some(projected) = projected else { break };
                value = projected;
            }
            if let Some(ty) = self.known_pattern_type_value(value) {
                known_types.extend(self.known_type_pattern_outputs(pattern, &ty, span)?);
                continue;
            }
            match (pattern, &value.kind) {
                (n::Pattern::Binding { binding, inner }, _) => {
                    if let Some(known) = self.generator_known_type_data(value, span)? {
                        known_types.insert(*binding, known);
                    }
                    pending.push((inner, value));
                }
                (n::Pattern::Tuple(patterns), K::Tuple(values))
                    if patterns.len() == values.len() =>
                {
                    pending.extend(patterns.iter().zip(values));
                }
                (
                    n::Pattern::Record {
                        declaration,
                        fields: patterns,
                    },
                    K::Record {
                        declaration: actual,
                        fields: values,
                    },
                ) if declaration == actual && patterns.len() == values.len() => {
                    pending.extend(patterns.iter().zip(values));
                }
                (
                    n::Pattern::AnonymousRecord {
                        names,
                        fields: patterns,
                    },
                    K::AnonymousRecord { fields: values },
                ) if patterns.len() == values.len() => {
                    if let Term::Record(types) = &value.ty
                        && names.len() == types.len()
                        && names
                            .iter()
                            .zip(types)
                            .all(|(name, (actual, _))| name == actual)
                    {
                        pending.extend(patterns.iter().zip(values));
                    }
                }
                _ => {}
            }
        }
        Ok(known_types)
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
        let mut known_types = if self.checking_code_generator && !decl.mutable {
            self.record_observed_initializer(&pattern, &value, decl.span)?;
            self.known_pattern_types(&pattern, &value, decl.span)?
        } else {
            BTreeMap::new()
        };
        if let n::Pattern::Binding { binding, inner } = &pattern {
            if matches!(**inner, n::Pattern::Wild) {
                self.generator_data.insert(
                    *binding,
                    if self.checking_code_generator {
                        known_types.remove(binding)
                    } else {
                        Some(value)
                    },
                );
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
                self.generator_data.insert(id, known_types.remove(&id));
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

impl Checker<'_, '_> {
    fn generator_known_type_data(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps: before,
            max_limbs: 4096,
        };
        let known = self.known_normal_input(value, 0, &mut budget);
        self.required_steps -= (before - budget.steps) as u64;
        let kind = match known {
            Some(ty) if value.ty == p(P::Type) && ty.sort() == Sort::Type => K::TypeValue(ty),
            // An immutable schema may retain its checked structural value so
            // later field projections and aliases can recover contained Types.
            // known_normal_input never calls a factory or unfolds a head;
            // the original initializer still executes during invocation.
            Some(
                known @ (Term::RecordValue(_) | Term::TupleValue(_) | Term::NominalValue { .. }),
            ) => K::ComptimeValue(known),
            _ => return Ok(None),
        };
        Ok(Some(n::Expr {
            ty: self.copy_alias_type(&value.ty, span)?,
            source: value.source,
            kind,
        }))
    }
}
