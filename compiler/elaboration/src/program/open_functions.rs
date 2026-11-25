//! Source definitions whose explicit deferred annotations need invocation inputs.
//! Ordinary descriptions never enter Term or typed NIR. Only checked concrete
//! instances are published to the runtime package.
use super::required::prepared::PreparedRequired;
use super::*;
use inputs::StagedInputs;
use observations::TypeObservation as D;
use std::sync::Arc;

pub(super) use annotations::Annotation;
use annotations::annotation_source;

mod annotations;
mod behavior_values;
mod fragments;
mod inline_evidence;
mod inputs;
mod live_defaults;
mod normal_applications;
mod observations;
mod pending_inputs;
mod scalar_descriptions;

/// Transfers observed during ordinary definition checking, keyed by the
/// source loop identity. Continue advances finite iteration but cannot make
/// an unconditional repeating loop complete normally.
#[derive(Clone, Copy, Default)]
pub(super) struct LoopTransfers {
    breaks: bool,
    continues: bool,
}

impl LoopTransfers {
    fn include(&mut self, other: Self) {
        self.breaks |= other.breaks;
        self.continues |= other.continues;
    }

    fn any(self) -> bool {
        self.breaks || self.continues
    }
}

// Only bindings delivered from the subject inherit its stage. A Then result
// belongs to the observation's stage. Alternatives must agree after their
// simultaneous binding remaps; one staged candidate is not enough.
fn subject_pattern_bindings(
    pattern: &n::Pattern,
    fuel: &mut u64,
    span: Span,
) -> Result<BTreeSet<n::LocalId>, Error> {
    enum Work<'a> {
        Visit(&'a n::Pattern),
        Bind(n::LocalId),
        Combine(usize, bool),
        Remap(&'a [(n::LocalId, n::LocalId)]),
    }
    let mut pending = vec![Work::Visit(pattern)];
    let mut values: Vec<BTreeSet<n::LocalId>> = Vec::new();
    while let Some(work) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "pattern stage provenance budget exhausted"))?;
        match work {
            Work::Bind(binding) => {
                values
                    .last_mut()
                    .ok_or_else(|| error(span, "pattern stage provenance stack underflow"))?
                    .insert(binding);
            }
            Work::Combine(count, intersect) => {
                let start = values
                    .len()
                    .checked_sub(count)
                    .ok_or_else(|| error(span, "pattern stage provenance stack underflow"))?;
                let mut sets = values.drain(start..);
                let mut result = sets.next().unwrap_or_default();
                for set in sets {
                    *fuel = fuel
                        .checked_sub((result.len() + set.len()) as u64)
                        .ok_or_else(|| error(span, "pattern stage provenance budget exhausted"))?;
                    if intersect {
                        result.retain(|id| set.contains(id));
                    } else {
                        result.extend(set);
                    }
                }
                values.push(result);
            }
            Work::Remap(mapping) => {
                *fuel = fuel
                    .checked_sub(mapping.len() as u64)
                    .ok_or_else(|| error(span, "pattern stage provenance budget exhausted"))?;
                let current = values
                    .last_mut()
                    .ok_or_else(|| error(span, "pattern stage provenance stack underflow"))?;
                let moved: Vec<_> = mapping
                    .iter()
                    .map(|(from, to)| (*to, current.contains(from)))
                    .collect();
                for (from, _) in mapping {
                    current.remove(from);
                }
                for (to, staged) in moved {
                    if staged {
                        current.insert(to);
                    } else {
                        current.remove(&to);
                    }
                }
            }
            Work::Visit(pattern) => match pattern {
                n::Pattern::CodeEvidence { .. } => {
                    return Err(error(span, "unresolved pattern-code evidence"));
                }
                n::Pattern::Binding { binding, inner } => {
                    pending.push(Work::Bind(*binding));
                    pending.push(Work::Visit(inner));
                }
                n::Pattern::OptionalPresent(pattern)
                | n::Pattern::ErrorResult { inner: pattern, .. }
                | n::Pattern::Observe { inner: pattern, .. }
                | n::Pattern::Guard { pattern, .. }
                | n::Pattern::Then { first: pattern, .. } => pending.push(Work::Visit(pattern)),
                n::Pattern::Remap { pattern, bindings } => {
                    pending.push(Work::Remap(bindings));
                    pending.push(Work::Visit(pattern));
                }
                n::Pattern::Tuple(fields)
                | n::Pattern::TypeTuple(fields)
                | n::Pattern::AnonymousRecord { fields, .. }
                | n::Pattern::NominalType { fields, .. }
                | n::Pattern::Record { fields, .. }
                | n::Pattern::Variant { fields, .. }
                | n::Pattern::NormalForm {
                    arguments: fields, ..
                }
                | n::Pattern::Or(fields) => {
                    *fuel = fuel
                        .checked_sub(fields.len() as u64)
                        .ok_or_else(|| error(span, "pattern stage provenance budget exhausted"))?;
                    pending.push(Work::Combine(
                        fields.len(),
                        matches!(pattern, n::Pattern::Or(_)),
                    ));
                    pending.extend(fields.iter().map(Work::Visit));
                }
                n::Pattern::OptionalAbsent
                | n::Pattern::Wild
                | n::Pattern::Unit
                | n::Pattern::Literal(_) => values.push(BTreeSet::new()),
            },
        }
    }
    Ok(values.pop().unwrap_or_default())
}

// A pending ordinary description is not a frozen inference candidate or IR.
enum OpenClauseInference<'src> {
    Ready(
        Vec<Term>,
        StagedInputs<'src>,
        BTreeMap<usize, CompletedType>,
    ),
    Pending(D),
}

/// Completion and truth known without invoking a definition's staged inputs.
struct OpenCondition {
    diverges: bool,
    choice: Option<bool>,
}

/// Source regions whose ordinary checks are complete before specialization.
#[derive(Clone, Default)]
pub(super) struct CheckedRegions {
    tuple_loops: BTreeSet<n::LocalId>,
    const_continuation_blocks: BTreeSet<n::LocalId>,
}

#[derive(Clone)]
pub(super) struct Definition {
    parameters: Vec<(Sort, Annotation)>,
    result: Annotation,
    default_types: BTreeMap<usize, Term>,
    default_dependencies: BTreeMap<usize, BTreeSet<usize>>,
    regions: CheckedRegions,
    checked: bool,
}

impl Definition {
    fn new(parameters: Vec<(Sort, Annotation)>, result: Annotation) -> Self {
        Self {
            parameters,
            result,
            default_types: BTreeMap::new(),
            default_dependencies: BTreeMap::new(),
            regions: CheckedRegions::default(),
            checked: false,
        }
    }

    pub(super) fn checked_empty_iteration(&self, site: hir::HirId) -> bool {
        self.checked && self.regions.tuple_loops.contains(&local(site))
    }

    pub(super) fn checked_const_continuation(&self, site: hir::HirId) -> bool {
        self.checked
            && self
                .regions
                .const_continuation_blocks
                .contains(&local(site))
    }

    pub(super) fn input_domain_refs(&self) -> lexical_domains::InputDomains<'_> {
        lexical_domains::InputDomains::Open(self.parameters.iter())
    }

    pub(super) fn input_domains(&self) -> Vec<(Sort, Option<Term>)> {
        self.input_domain_refs()
            .map(|(sort, ty)| (sort, ty.cloned()))
            .collect()
    }

    pub(super) fn input_sorts(&self) -> Vec<Sort> {
        self.parameters.iter().map(|(sort, _)| *sort).collect()
    }
}

impl std::fmt::Display for Definition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(open-function (parameters")?;
        for (sort, annotation) in &self.parameters {
            write!(f, " (parameter {sort} {annotation})")?;
        }
        write!(
            f,
            ") (result {}) (definition-checked {}) (default-types",
            self.result, self.checked
        )?;
        for (slot, ty) in &self.default_types {
            write!(f, " (default {slot} {ty})")?;
        }
        f.write_str(") (default-dependencies")?;
        for (slot, dependencies) in &self.default_dependencies {
            write!(f, " (default {slot}")?;
            for dependency in dependencies {
                write!(f, " {dependency}")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}

impl<'a, 'h> Checker<'a, 'h> {
    fn open_item(&self, owner: OwnerId, span: Span) -> Result<&'h hir::Item<'h>, Error> {
        self.package
            .item(owner)
            .ok_or_else(|| error(span, "missing open function definition"))
    }

    fn open_fn_or_case_sig(
        &self,
        owner: OwnerId,
        span: Span,
    ) -> Result<&'h hir::item::FnSig<'h>, Error> {
        match &self.open_item(owner, span)?.kind {
            ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) => Ok(sig),
            _ => Err(error(span, "expected open function or case definition")),
        }
    }
}

impl Checker<'_, '_> {
    /// The outer definition check still owns validation when a recursive edge
    /// forms an interface. This does not make pending comptime code executable.
    fn check_open_interface_dependency(&mut self, owner: OwnerId) -> Result<(), Error> {
        if self.active_open_definitions.contains(&owner) {
            return Ok(());
        }
        self.check_open_definition(owner)
    }

    /// Describe explicit callable formation without executing a signature that
    /// still inspects an abstract caller value. No callable NIR is published.
    fn describe_pending_function_value(
        &mut self,
        expr: &Expr<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<Option<D>, Error> {
        let H::NFApplication(head, clauses) = &expr.kind else {
            return Ok(None);
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        if !matches!(self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Fn(sig, _) | ItemKind::Case(sig, _)) if !sig.modifiers.is_normal_form_head)
        {
            return Ok(None);
        }
        self.ensure_signature(owner, expr.span)?;
        if !self.open_functions.contains_key(&owner) || self.declared_signature(owner).is_some() {
            return Ok(None);
        }
        self.check_open_interface_dependency(owner)?;
        self.ensure_contract(owner, expr.span)?;
        let sig = self.open_fn_or_case_sig(owner, expr.span)?;
        arguments::require_function_value_parameters(sig.params, expr.span)?;
        let definition = self.open_functions[&owner].clone();
        if definition
            .parameters
            .iter()
            .any(|(sort, _)| *sort != Sort::Runtime)
        {
            return Err(error(
                expr.span,
                "open function value still requires ordinary comptime inputs",
            ));
        }
        let lexical = self.lexical_function_arguments(owner, expr.span)?;
        let expanded = normal::literal_spreads(clauses);
        let routed = expanded.as_deref().unwrap_or(clauses);
        let mut pending_source = false;
        let mut simple = true;
        for argument in routed {
            match argument {
                Arg::Positional(value) | Arg::Named(_, value) => {
                    // Direct inputs already have symbolic transport below.
                    // Defer computations, not otherwise known callable shapes.
                    if !matches!(
                        value.kind,
                        H::ResolvedFnParam(..) | H::ResolvedClauseParam(..) | H::ResolvedLocal(..)
                    ) {
                        pending_source |= !self
                            .source_expression_inputs(value.hir_id.owner, value)?
                            .is_empty();
                    }
                }
                _ => simple = false,
            }
        }
        if pending_source && simple {
            let parameters = self.type_clause_params(owner);
            let domains = self.trait_domains(owner)?;
            // Domain-dependent telescopes need sequential value formation.
            // Observe both input kinds once; observation failure is not a
            // dependent domain that may silently take another checking path.
            let mut independent = true;
            for domain in &domains {
                let inputs = self.observed_term_inputs(domain, expr.span)?;
                independent &= inputs.declarations.is_empty() && inputs.parameters.is_empty();
            }
            if independent && domains.len() == lexical.len() + parameters.len() {
                let mut supplied = vec![false; parameters.len()];
                let mut routing = clause_inputs::routing_cursor(&parameters, routed)
                    .map_err(|failure| Self::type_input_routing_error(failure.kind, expr.span))?;
                for argument in routed {
                    let (input, value) = match argument {
                        Arg::Positional(value) => (input_routing::Input::Positional, *value),
                        Arg::Named(name, value) => (input_routing::Input::Named(name.name), *value),
                        _ => {
                            return Err(error(
                                expr.span,
                                "unsupported clause argument in pending function value",
                            ));
                        }
                    };
                    let slot = routing
                        .select(&input)
                        .map_err(|kind| Self::type_input_routing_error(kind, value.span))?
                        .parameter;
                    if std::mem::replace(&mut supplied[slot], true) {
                        return Err(error(value.span, "duplicate type argument"));
                    }
                    let domain = D::Known(domains[lexical.len() + slot].clone());
                    self.with_staged_input(true, |this| {
                        this.describe_open_expression(
                            value,
                            &mut locals.clone(),
                            returns,
                            Some(&domain),
                            depth + 1,
                        )
                    })?;
                }
                for (parameter, supplied) in parameters.iter().zip(supplied) {
                    if !supplied
                        && !matches!(
                            parameter.kind.unquoted(),
                            hir::clause::ClauseParamKind::Optional(..)
                        )
                    {
                        return Err(error(expr.span, "missing required type argument"));
                    }
                }
                self.check_type_defaults(owner)?;
                // Only a source description escapes. Concrete specialization
                // performs all applications/requirements and evaluates inputs
                // in source order, including only the selected defaults.
                return Ok(Some(D::Unknown));
            }
        }
        let saved_open_call = std::mem::replace(&mut self.describing_open_call, true);
        let arguments = self.function_clause_arguments(expr, owner, clauses);
        self.describing_open_call = saved_open_call;
        let arguments = arguments?;
        self.check_type_application(owner, &arguments, expr.span)?;
        let mut unresolved = Vec::new();
        for (binder, argument) in self.declaration_binders(owner).iter().zip(&arguments) {
            let inputs = self.observed_term_inputs(argument, expr.span)?;
            if inputs
                .declarations
                .values()
                .chain(inputs.parameters.values())
                .any(|sort| *sort == Sort::ComptimeValue)
            {
                unresolved.push(binder.declaration);
            }
        }
        if !unresolved.is_empty() {
            for annotation in definition
                .parameters
                .iter()
                .map(|(_, ty)| ty)
                .chain(std::iter::once(&definition.result))
            {
                match annotation {
                    Annotation::Pending(code)
                        if self.open_computation_uses_clauses(code, &unresolved, expr.span)? =>
                    {
                        return Ok(Some(D::Unknown));
                    }
                    Annotation::Suspended { .. } => return Ok(Some(D::Unknown)),
                    _ => {}
                }
            }
        }
        // Reuse these evaluated clause inputs; falling back through source
        // expression elaboration would run default suppliers a second time.
        let expected = match expected {
            Some(D::Known(ty)) => Some(ty),
            _ => None,
        };
        let value = self.function_instance(expr, owner, arguments, expected, false)?;
        let value = self.callable_value_view(value, expected, expr.span)?;
        self.check_open_fragment(&value, expr.span, false)?;
        Ok(Some(D::Known(value.ty)))
    }

    /// A private inference shape: pending annotations contribute no equations.
    /// Void placeholders are never exposed as the function's actual signature.
    pub(super) fn open_callable_inference_shape(
        &self,
        owner: OwnerId,
    ) -> Option<(Term, Vec<bool>, bool)> {
        let definition = self.open_functions.get(&owner)?;
        let mut known = Vec::new();
        let parameters = definition
            .parameters
            .iter()
            .map(|(sort, annotation)| {
                let ty = match annotation {
                    Annotation::Known(ty) => {
                        known.push(true);
                        ty.clone()
                    }
                    _ => {
                        known.push(false);
                        p(P::Void)
                    }
                };
                Parameter { sort: *sort, ty }
            })
            .collect();
        let (result, known_result) = match &definition.result {
            Annotation::Known(ty) => (ty.clone(), true),
            _ => (p(P::Void), false),
        };
        Some((
            Term::Function {
                parameters,
                result: Box::new(result),
            },
            known,
            known_result,
        ))
    }

    /// Runtime callable storage needs an executable body, even if the signature
    /// is already known. Staged contexts may still transport a pending identity.
    pub(super) fn open_value_needs_instance(&self, owner: OwnerId) -> bool {
        self.open_functions.get(&owner).is_some_and(|definition| {
            self.declared_signature(owner).is_none()
                || (self.runtime_value_context()
                    && definition
                        .parameters
                        .iter()
                        .all(|(sort, _)| *sort == Sort::Runtime))
        })
    }

    /// Finish clause-dependent annotations and body work before publishing a
    /// callable value. Ordinary staged inputs and parameter protocols remain.
    pub(super) fn open_function_value(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        arguments: &[Term],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.check_open_interface_dependency(owner)?;
        self.check_type_application(owner, arguments, expr.span)?;
        let binders = self.declaration_binders(owner);
        if binders.len() != arguments.len() {
            return Err(error(
                expr.span,
                "open function value clause argument count mismatch",
            ));
        }
        let saved = self.alias_substitution.clone();
        for (binder, argument) in binders.iter().zip(arguments) {
            self.alias_substitution
                .insert(binder.declaration, Arc::new(argument.clone()));
        }
        let result = (|| {
            let definition = self.open_functions[&owner].clone();
            let sig = self.open_fn_or_case_sig(owner, expr.span)?;
            arguments::require_function_value_parameters(sig.params, expr.span)?;
            if definition
                .parameters
                .iter()
                .any(|(sort, _)| *sort != Sort::Runtime)
            {
                return Err(error(
                    expr.span,
                    "open function value still requires ordinary comptime inputs",
                ));
            }
            let symbols = vec![None; sig.params.len()];
            let mut parameters = Vec::new();
            let mut result = None;
            for (slot, annotation) in definition
                .parameters
                .iter()
                .map(|(_, ty)| ty)
                .chain(std::iter::once(&definition.result))
                .enumerate()
            {
                let annotation = match annotation {
                    Annotation::Known(ty) => {
                        Annotation::Known(self.specialize_open_clause_type(owner, ty, expr.span)?)
                    }
                    Annotation::Pending(_) => Annotation::Suspended { owner, slot },
                    other => other.clone(),
                };
                let ty = self.instantiate_open_annotation(
                    owner,
                    slot,
                    &annotation,
                    sig,
                    &symbols,
                    &parameters,
                    expr,
                )?;
                let ty = self.admit_type_data(ty, expr.span, |_| {
                    let Some(Term::Function { parameters, result }) =
                        expected.map(Term::callable_signature)
                    else {
                        return Err(context_error(
                            expr.span,
                            "gradual function value needs a concrete callable target",
                        ));
                    };
                    if parameters.len() != sig.params.len() {
                        return Err(error(expr.span, "callable target parameter count mismatch"));
                    }
                    Ok(parameters
                        .get(slot)
                        .map_or_else(|| (**result).clone(), |p| p.ty.clone()))
                })?;
                if slot == sig.params.len() {
                    result = Some(Box::new(ty));
                } else {
                    parameters.push(Parameter {
                        sort: Sort::Runtime,
                        ty,
                    });
                }
            }
            let signature = Term::Function {
                parameters,
                result: result.ok_or_else(|| {
                    error(
                        expr.span,
                        "open function value is missing result annotation",
                    )
                })?,
            };
            let value = self.open_instance_value(owner, &symbols, signature, expr.span)?;
            let generated_id = match &value.kind {
                K::Function(id)
                | K::FunctionInstance {
                    declaration: id, ..
                } => Some(*id),
                _ => None,
            };
            if let Some(id) = generated_id
                && self
                    .generated_functions
                    .declaration(id)
                    .is_some_and(|decl| !decl.env_parameters.is_empty())
            {
                return Err(error(
                    expr.span,
                    "function with runtime quote dependencies cannot form a bare function value",
                ));
            }
            Ok(value)
        })();
        self.alias_substitution = saved;
        result
    }

    /// A known callable telescope can be named before its staged body exists.
    /// This exposes identity and signature, never an executable residual body.
    pub(super) fn declared_signature(&self, owner: OwnerId) -> Option<std::borrow::Cow<'_, Term>> {
        if let Some(signature) = self
            .signatures
            .get(&owner)
            .or_else(|| self.interface_signatures.get(&owner))
        {
            return Some(std::borrow::Cow::Borrowed(signature));
        }
        let definition = self.open_functions.get(&owner)?;
        let Annotation::Known(result) = &definition.result else {
            return None;
        };
        let parameters = definition
            .parameters
            .iter()
            .map(|(sort, annotation)| {
                let Annotation::Known(ty) = annotation else {
                    return None;
                };
                Some(Parameter {
                    sort: *sort,
                    ty: ty.clone(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(std::borrow::Cow::Owned(Term::Function {
            parameters,
            result: Box::new(result.clone()),
        }))
    }

    pub(super) fn has_staged_body_work(&mut self, owner: OwnerId) -> Result<bool, Error> {
        let Some(ItemKind::Fn(sig, body) | ItemKind::Case(sig, body)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(false);
        };
        let Some(body) = self.package.body(*body) else {
            return Ok(false);
        };
        for (is_default, root) in
            std::iter::once((false, body.value)).chain(sig.params.iter().filter_map(|parameter| {
                match parameter.ty.kind {
                    TyParamKind::Optional(_, _, value) => Some((true, value)),
                    _ => None,
                }
            }))
        {
            let mut pending_inputs: BTreeSet<_> = sig
                .params
                .iter()
                .filter(|parameter| sig.modifiers.is_comptime || parameter.ty.is_comptime())
                .map(|parameter| local(parameter.hir_id))
                .collect();
            if is_default {
                // Follow local aliases only for dependency classification.
                // Availability and mutation are still checked by elaboration.
                let mut aliases = Vec::new();
                self.scan_callable_requirements(root, |this, expression| {
                    if let H::Let(binding) = &expression.kind {
                        if let Some(init) = binding.init {
                            aliases.push((
                                pattern_binding_ids(&binding.pattern),
                                this.expression_free_local_dependencies(owner, init)?,
                            ));
                        }
                    }
                    Ok(false)
                })?;
                loop {
                    let mut changed = false;
                    for (bindings, inputs) in &aliases {
                        self.required_steps =
                            self.required_steps.checked_sub(1).ok_or_else(|| {
                                error(root.span, "default dependency budget exhausted")
                            })?;
                        if !inputs.is_disjoint(&pending_inputs) {
                            for binding in bindings {
                                changed |= pending_inputs.insert(*binding);
                            }
                        }
                    }
                    if !changed {
                        break;
                    }
                }
            }
            if self.scan_callable_requirements(root, |this, expression| {
                // A trait query supplied directly to a staged argument can
                // depend on the caller even when both signatures are closed.
                // Route that free function through the existing source instance.
                if matches!(expression.kind, H::TraitBound(..)) && !sig.modifiers.is_comptime {
                    let inputs = this.expression_declaration_dependencies(owner, expression)?;
                    if this.type_parameters[&owner]
                        .iter()
                        .any(|id| inputs.contains(id))
                        || !pending_inputs.is_empty()
                            && !this
                                .expression_free_local_dependencies(owner, expression)?
                                .is_disjoint(&pending_inputs)
                    {
                        return Ok(true);
                    }
                }
                let selectors: Vec<_> = match expression.kind {
                    H::InlineControl(value) => {
                        if !is_default {
                            return Ok(true);
                        }
                        match value.kind {
                            H::If(condition, _, _) => vec![condition],
                            H::When(arms) => arms.iter().filter_map(|arm| arm.cond).collect(),
                            H::Match(subject, arms) => {
                                let mut selectors = vec![subject];
                                for arm in arms {
                                    selectors.extend(arm.guard);
                                    this.pattern_requirement_expressions(&arm.pat, &mut selectors)?;
                                }
                                selectors
                            }
                            _ => vec![value],
                        }
                    }
                    H::Let(binding) if binding.comptime => {
                        if !is_default {
                            return Ok(true);
                        }
                        let mut selectors: Vec<_> = binding.init.into_iter().collect();
                        selectors.extend(binding.guard);
                        this.pattern_requirement_expressions(&binding.pattern, &mut selectors)?;
                        selectors
                    }
                    _ => Vec::new(),
                };
                for selector in selectors {
                    if !this
                        .expression_free_local_dependencies(owner, selector)?
                        .is_disjoint(&pending_inputs)
                    {
                        return Ok(true);
                    }
                }
                if !matches!(expression.kind, H::Projection(..)) {
                    return Ok(false);
                }
                let mut receiver = expression;
                while let H::Projection(inner, _) = receiver.kind {
                    receiver = inner;
                }
                let H::ResolvedFnParam(_, parameter) = receiver.kind else {
                    return Ok(false);
                };
                // Visit projections inside composite annotations as well as value
                // expressions. Their Type receiver identity arrives at invocation,
                // even when all signature domains already have symbolic identities.
                Ok(sig
                    .params
                    .iter()
                    .position(|input| input.hir_id == parameter)
                    .and_then(|slot| this.signature_domains.get(&owner)?.get(slot))
                    .is_some_and(|(sort, domain)| {
                        *sort != Sort::Runtime && domain.as_ref() == Some(&p(P::Type))
                    }))
            })? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Description-only dependency through immutable member aliases. This
    /// does not assert that the selected value is Type; instance checking must
    /// still establish its domain, member signature and access permissions.
    fn pending_member_origin(
        &mut self,
        expression: &Expr<'_>,
        locals: &BTreeMap<n::LocalId, D>,
    ) -> Result<bool, Error> {
        // This is a meet over value-producing branches, not a scan for any
        // staged occurrence. A staged condition alone cannot authorize member
        // access on an unrelated unknown result. Branches retain their original
        // source expressions for instance-side checking.
        // Local aliases inside a result branch have already left the checker's
        // live scope. Follow their HIR binding identities, never their names;
        // this preserves lexical separation between sibling blocks.
        fn remember<'h>(
            block: &hir::expr::Block<'h>,
            aliases: &mut BTreeMap<n::LocalId, &'h Expr<'h>>,
            fuel: &mut u64,
        ) -> Result<(), Error> {
            for statement in block.stmts {
                let mut statement = statement;
                loop {
                    *fuel = fuel.checked_sub(1).ok_or_else(|| {
                        error(statement.span, "pending member provenance budget exhausted")
                    })?;
                    if let H::Semi(inner) = statement.kind {
                        statement = inner;
                    } else {
                        break;
                    }
                }
                if let H::Let(binding) = statement.kind
                    && !binding.mutable
                    && let Some(init) = binding.init
                {
                    // Description checking has already validated the actual
                    // Tuple constructor and its pattern. Recover only source
                    // dependencies here. A guard only filters the successful
                    // continuation; its checked diverging else cannot supply
                    // a different value to these bindings. Do not manufacture
                    // a structural Type or assume that an arbitrary pattern
                    // view is positional.
                    let mut fields = vec![(&binding.pattern, init)];
                    while let Some((pattern, mut value)) = fields.pop() {
                        *fuel = fuel.checked_sub(1).ok_or_else(|| {
                            error(pattern.span, "pending member provenance budget exhausted")
                        })?;
                        match &pattern.kind {
                            hir::pattern::PatternKind::Binding(
                                hir::common::BindingMode::ByValue,
                                _,
                                inner,
                            ) => {
                                aliases.insert(local(pattern.hir_id), value);
                                if let Some(inner) = inner {
                                    fields.push((inner, value));
                                }
                            }
                            hir::pattern::PatternKind::Tuple(patterns) => {
                                // Immutable aliases can hold the tuple before
                                // destructuring. Identity-based links retain
                                // sibling scopes and earlier binding snapshots.
                                while let H::ResolvedLocal(_, id) = value.kind {
                                    *fuel = fuel.checked_sub(1).ok_or_else(|| {
                                        error(
                                            value.span,
                                            "pending member provenance budget exhausted",
                                        )
                                    })?;
                                    let Some(init) = aliases.get(&local(id)) else {
                                        break;
                                    };
                                    value = init;
                                }
                                if let H::Tuple(values) = value.kind
                                    && patterns.len() == values.len()
                                {
                                    fields.extend(patterns.iter().zip(values.iter()));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(())
        }
        let mut aliases = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut pending = vec![(expression, false)];
        while let Some((root, selected)) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(root.span, "pending member provenance budget exhausted"))?;
            // Alias graphs can share whole conditional subtrees. Inspect each
            // source node once per selection mode, while charging every edge.
            if !seen.insert((local(root.hir_id), selected)) {
                continue;
            }
            match root.kind {
                H::Projection(inner, _) | H::NFApplication(inner, _) => {
                    pending.push((inner, true));
                }
                H::Block(block) | H::UnsafeBlock(block) => {
                    remember(block, &mut aliases, &mut self.required_steps)?;
                    let Some(value) = block.expr else {
                        return Ok(false);
                    };
                    pending.push((value, selected));
                }
                H::If(_, yes, Some(no)) => {
                    remember(yes, &mut aliases, &mut self.required_steps)?;
                    let Some(yes) = yes.expr else {
                        return Ok(false);
                    };
                    pending.push((yes, selected));
                    pending.push((no, selected));
                }
                H::Match(_, arms) if !arms.is_empty() => {
                    pending.extend(arms.iter().map(|arm| (arm.body, selected)));
                }
                H::When(arms) if arms.iter().any(|arm| arm.cond.is_none()) => {
                    pending.extend(arms.iter().map(|arm| (arm.body, selected)));
                }
                H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                    let id = local(id);
                    if let Some(init) = aliases.get(&id) {
                        pending.push((init, selected));
                        continue;
                    }
                    let member = self.open_member_locals.contains(&id)
                        || (selected
                            && !self.generator_data.get(&id).is_some_and(Option::is_some)
                            && (matches!(locals.get(&id), Some(D::Known(domain)) if *domain == p(P::Type))
                                || self.locals.get(&id) == Some(&p(P::Type))));
                    if !member {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    // Keep binding preparation temporaries out of every recursive expression
    // frame, including deeply nested operators which never bind a local.
    #[inline(never)]
    fn describe_open_binding(
        &mut self,
        expr: &Expr<'_>,
        binding: &&hir::decl::LetDecl<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        // Contextual values have dedicated interfaces, not ordinary callable
        // results or unknown observations. Keep their witnesses in the same code
        // binding path used by a direct splice in this definition.
        if binding.comptime
            && binding
                .init
                .map(|init| {
                    Ok::<_, Error>(
                        self.code_producer(init)?
                            || self.pattern_code_producer(init, binding.ty)?
                            || binding
                                .ty
                                .is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..))),
                    )
                })
                .transpose()?
                .unwrap_or(false)
        {
            return self.describe_known_region(expr, expected);
        }
        let pending_annotation = std::cell::Cell::new(false);
        let annotation = binding
            .ty
            .map(|ty| {
                let pending = self.scan_body_requirement(ty, |this, component| {
                    let member = this.pending_member_origin(component, locals)?;
                    let alias = matches!(component.kind,
                        H::ResolvedLocal(_, id)
                        if this.open_staged_locals.contains(&local(id))
                            && !this.generator_data.get(&local(id)).is_some_and(Option::is_some)
                            && matches!(locals.get(&local(id)), Some(D::Unknown) | Some(D::Known(Term::Primitive(P::Type)))));
                    Ok(member || alias)
                })?;
                if pending {
                    self.with_staged_input(true, |this| {
                        this.describe_open_expression(
                            ty,
                            locals,
                            returns,
                            Some(&D::Known(p(P::Type))),
                            depth + 1,
                        )
                    })?;
                    // Constructor syntax can establish the domain Type
                    // without establishing the identity of its inputs.
                    // Keep the entire source annotation for the instance.
                    pending_annotation.set(true);
                    return Ok(D::Unknown);
                }
                self.signature_annotation(ty)
                    .map(|annotation| annotation.description())
            })
            .transpose()?;
        let init = binding
            .init
            .ok_or_else(|| error(expr.span, "open definition binding requires an initializer"))?;
        let mut root = init;
        while let H::Projection(inner, _) = root.kind {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(init.span, "staged alias description budget exhausted"))?;
            root = inner;
        }
        let staged_alias = !binding.mutable
            && matches!(root.kind,
                H::ResolvedFnParam(_, id) | H::ResolvedLocal(_, id)
                if self.input_facts.is_staged(&local(id))
                    || self.open_staged_locals.contains(&local(id)));
        let owner = init.hir_id.owner;
        let outer = self.pending_initializer.replace((owner, false));
        let actual = self.with_staged_input(self.in_staged_input || binding.comptime, |this| {
            this.describe_open_expression(init, locals, returns, annotation.as_ref(), depth + 1)
        });
        let (_, pending_formation) = std::mem::replace(&mut self.pending_initializer, outer)
            .ok_or_else(|| error(init.span, "missing active initializer scope"))?;
        if pending_formation
            && let Some((scope, pending)) = &mut self.pending_initializer
            && *scope == owner
        {
            *pending = true;
        }
        let actual = actual?;
        let plain = matches!(
            binding.pattern.kind,
            hir::pattern::PatternKind::Binding(hir::common::BindingMode::ByValue, _, None)
                | hir::pattern::PatternKind::Wild
        ) && binding.guard.is_none()
            && binding.otherwise.is_none();
        if !plain
            && let D::Known(ty) = &actual
            && !matches!(annotation, Some(D::Unknown))
        {
            // Reuse pattern protocols, refutability, guard scope and
            // immutable-binding checks in the ordinary checker.
            let previous_bindings = self.locals.keys().copied().collect::<BTreeSet<_>>();
            let result = if *ty == p(P::Type) || !self.contains_inline_control(init)? {
                // Preserve concrete pattern evidence when there is no
                // inline selector requiring compositional checking.
                // Type patterns need the original type identity, not
                // merely an abstract value with the domain Type.
                if staged_alias || binding.comptime {
                    self.describe_staged_binding_region(expr, expected, locals, returns, depth + 1)?
                } else {
                    self.with_staged_input(self.in_staged_input || staged_alias, |this| {
                        this.describe_known_region(expr, expected)
                    })?
                }
            } else {
                self.with_open_operand(ty.clone(), init.span, |this, placeholder| {
                    let binding = hir::decl::LetDecl {
                        init: Some(placeholder),
                        ..(*binding).clone()
                    };
                    let probe = Expr {
                        kind: H::Let(&binding),
                        ..expr.clone()
                    };
                    this.describe_known_region(&probe, expected)
                })?
            };
            // Pattern splices can introduce outputs absent from the source
            // pattern's syntactic binding list. Publish the bindings actually
            // established by the checked pattern, including these outputs.
            for (&id, ty) in &self.locals {
                if !previous_bindings.contains(&id) {
                    locals.insert(id, D::Known(ty.clone()));
                    if self.in_staged_input {
                        self.open_staged_locals.insert(id);
                    }
                }
            }
            self.require_description(&result, expected, expr.span)?;
            return Ok(result);
        }
        if binding.guard.is_some() || binding.otherwise.is_some() {
            return Err(error(
                expr.span,
                "guarded binding with an unknown domain requires established pattern evidence",
            ));
        }
        let diverges = matches!(&actual, D::Known(ty) if is_never(ty));
        let pending_input = matches!(actual, D::Unknown);
        let member_origin =
            pending_input && !binding.mutable && self.pending_member_origin(init, locals)?;
        // A pending source annotation is rechecked at invocation.
        // Preserve the initializer's established domain meanwhile, so
        // pointer/tuple operations do not lose their existing shape.
        let ty = if pending_annotation.get() {
            actual
        } else {
            annotation.unwrap_or(actual)
        };
        if binding.comptime
            && !pending_input
            && !pending_formation
            && plain
            && let D::Known(domain) = &ty
            && (*domain == p(P::Type)
                || self.is_type_list_type(domain)
                || matches!(domain.callable_signature(), Term::Function { .. }))
        {
            // An unconditional Type alias needs the same definition-
            // side evidence as a guarded/destructured const. Retain
            // known structure without executing an open invocation.
            // An annotation alone does not establish the initializer's
            // identity; pending member inputs are rechecked per instance.
            let input = self.with_staged_input(true, |this| this.expr(init, Some(domain)))?;
            self.with_staged_input(true, |this| {
                this.check_open_fragment(&input, init.span, false)
            })?;
            let pattern = self.pattern(&binding.pattern, domain)?;
            if matches!(input.kind, K::Function(_) | K::FunctionInstance { .. }) {
                if let n::Pattern::Binding { binding: id, inner } = &pattern {
                    if matches!(**inner, n::Pattern::Wild) {
                        constants::charge_copy(&input, &mut self.required_steps, expr.span)?;
                        self.generator_data.insert(*id, Some(input.clone()));
                    }
                }
            }
            for (id, value) in self.known_pattern_types(&pattern, &input, expr.span)? {
                self.generator_data.insert(id, Some(value));
            }
            let mut formed = &input;
            while let K::Block {
                result: Some(result),
                ..
            }
            | K::CheckTypeAlias {
                body: Some(result), ..
            } = &formed.kind
            {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(expr.span, "constant formation evidence budget exhausted")
                })?;
                formed = result;
            }
            if matches!(formed.kind, K::NominalType { .. })
                && let n::Pattern::Binding { binding: id, inner } = &pattern
                && matches!(**inner, n::Pattern::Wild)
                && !self.generator_data.contains_key(id)
            {
                // The container head is known even when its value
                // inputs await invocation. Retain its checked formation
                // for member routing, without claiming a completed Type.
                self.generator_data.insert(*id, Some(input));
            }
        }
        match &binding.pattern.kind {
            hir::pattern::PatternKind::Binding(hir::common::BindingMode::ByValue, _, None) => {
                let id = local(binding.pattern.hir_id);
                if let D::Known(ty) = &ty {
                    self.locals.insert(id, ty.clone());
                }
                if self.in_staged_input || staged_alias || binding.comptime {
                    self.open_staged_locals.insert(id);
                }
                if member_origin {
                    self.open_member_locals.insert(id);
                } else {
                    self.open_member_locals.remove(&id);
                }
                locals.insert(id, ty);
                if !binding.mutable {
                    self.immutable.insert(id);
                }
            }
            hir::pattern::PatternKind::Wild => {}
            _ => {
                return Err(error(
                    binding.pattern.span,
                    "destructuring an unknown definition domain requires established pattern evidence",
                ));
            }
        }
        let result = D::Known(p(if diverges { P::Never } else { P::Void }));
        self.require_description(&result, expected, expr.span)?;
        Ok(result)
    }

    #[inline(never)]
    fn describe_open_unary(
        &mut self,
        expression: &Expr<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let mut nodes = Vec::new();
        let mut current = expression;
        let mut context = expected;
        while let H::Unary(op @ (UnOp::Not | UnOp::Neg), operand) = current.kind {
            if is_numeric_literal(operand) {
                break;
            }
            if depth + nodes.len() >= 128 {
                return Err(error(
                    current.span,
                    "ordinary deferred body depth exhausted",
                ));
            }
            if !nodes.is_empty() {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(
                        current.span,
                        "ordinary deferred body checking budget exhausted",
                    )
                })?;
            }
            nodes.push((current, operand, op, context));
            let operator = if op == UnOp::Not {
                crate::arithmetic::Operator::Invert
            } else {
                crate::arithmetic::Operator::Negate
            };
            if self.protocols.arithmetic[operator as usize].is_some() {
                context = None;
            }
            current = operand;
        }
        let mut description =
            self.describe_open_expression(current, locals, returns, context, depth + nodes.len())?;
        for (node, operand, op, context) in nodes.into_iter().rev() {
            let D::Known(ty) = description else {
                return Err(error(
                    operand.span,
                    "unary operation requires an established operand type",
                ));
            };
            description = self.with_open_operand(ty, operand.span, |this, placeholder| {
                let probe = Expr {
                    hir_id: node.hir_id,
                    span: node.span,
                    kind: H::Unary(op, placeholder),
                };
                this.describe_known_region(&probe, context)
            })?;
        }
        Ok(description)
    }

    fn contains_inline_control(&mut self, value: &Expr<'_>) -> Result<bool, Error> {
        self.scan_body_requirement(value, |_, expr| {
            Ok(matches!(expr.kind, H::InlineControl(_)))
        })
    }

    // A residual function also forms its nested lambdas. Their local control
    // targets stay private, but their staging may require this invocation's
    // inputs. Keep visiting bodies iterative, including post-do suppliers.
    fn scan_callable_requirements(
        &mut self,
        root: &Expr<'_>,
        mut required: impl for<'e> FnMut(&mut Self, &Expr<'e>) -> Result<bool, Error>,
    ) -> Result<bool, Error> {
        let mut bodies = Vec::new();
        let mut current = root;
        loop {
            if self.scan_body_requirement(current, |this, expr| {
                if let H::Closure(_, _, body) | H::PostLambdaBody(body) = expr.kind {
                    bodies.push(body);
                }
                required(this, expr)
            })? {
                return Ok(true);
            }
            let Some(id) = bodies.pop() else {
                return Ok(false);
            };
            current = self
                .package
                .body(id)
                .ok_or_else(|| error(root.span, "missing closure body"))?
                .value;
        }
    }

    pub(super) fn closure_needs_staging(
        &mut self,
        body: &Expr<'_>,
        ordinary: bool,
    ) -> Result<bool, Error> {
        self.scan_body_requirement(body, |_, node| {
            Ok(matches!(node.kind, H::Let(decl) if decl.comptime && (ordinary || decl.otherwise.is_some()))
                || ordinary && matches!(node.kind, H::InlineControl(_)))
        })
    }

    fn scan_body_requirement<'h>(
        &mut self,
        value: &Expr<'h>,
        mut required: impl FnMut(&mut Self, &Expr<'h>) -> Result<bool, Error>,
    ) -> Result<bool, Error> {
        let mut pending = vec![value];
        while let Some(expr) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(expr.span, "body requirement scan budget exhausted"))?;
            if required(self, expr)? {
                return Ok(true);
            }
            match &expr.kind {
                H::Block(block) | H::UnsafeBlock(block) | H::Loop(block) => {
                    pending.extend(block.stmts);
                    pending.extend(block.expr);
                }
                H::For {
                    pat,
                    iter,
                    guard,
                    body,
                    ..
                } => {
                    self.pattern_requirement_expressions(pat, &mut pending)?;
                    pending.push(iter);
                    pending.extend(*guard);
                    pending.extend(body.stmts);
                    pending.extend(body.expr);
                }
                H::If(condition, block, otherwise) => {
                    pending.push(condition);
                    pending.extend(block.stmts);
                    pending.extend(block.expr);
                    pending.extend(*otherwise);
                }
                H::When(arms) => {
                    for arm in *arms {
                        pending.extend(arm.cond);
                        pending.push(arm.body);
                    }
                }
                H::Match(value, arms) | H::EffectHandle(value, arms) => {
                    pending.push(value);
                    pending.extend(arms.iter().map(|arm| arm.body));
                    pending.extend(arms.iter().filter_map(|arm| arm.guard));
                    for arm in *arms {
                        self.pattern_requirement_expressions(&arm.pat, &mut pending)?;
                    }
                }
                H::Application(callee, args)
                | H::ExtendedApplication(callee, args)
                | H::NFApplication(callee, args) => {
                    pending.push(callee);
                    for arg in *args {
                        match arg {
                            Arg::Positional(value)
                            | Arg::Named(_, value)
                            | Arg::Expand(value)
                            | Arg::Implicit(value)
                            | Arg::PostLambda(value) => pending.push(value),
                        }
                    }
                }
                H::Binary(_, left, right)
                | H::AssignOp(_, left, right)
                | H::Index(left, right)
                | H::Assign(left, right)
                | H::Cast(left, right)
                | H::DynCast(left, right) => {
                    pending.push(left);
                    pending.push(right);
                }
                H::TyFnArrow(parameters, result) => {
                    pending.push(parameters);
                    pending.push(result);
                }
                H::TyFn(parameters) => {
                    for parameter in *parameters {
                        if let TyParamKind::Positional(ty) = parameter.kind {
                            pending.push(ty);
                        }
                    }
                }
                H::ErrorQualifiedType(errors, result) => {
                    pending.push(errors);
                    pending.push(result);
                }
                H::ErrorHandle {
                    value,
                    before,
                    arms,
                    after,
                } => {
                    pending.push(value);
                    for arm in before
                        .iter()
                        .copied()
                        .chain(arms.iter())
                        .chain(after.iter().copied())
                    {
                        self.pattern_requirement_expressions(&arm.pat, &mut pending)?;
                        pending.extend(arm.guard);
                        pending.push(arm.body);
                    }
                }
                H::Range { start, end, .. } => {
                    pending.extend(*start);
                    pending.extend(*end);
                }
                H::Tuple(values) | H::List(values) => pending.extend(*values),
                H::Object(entries) => {
                    for entry in entries {
                        pending.push(match entry {
                            hir::expr::ObjectEntry::Value(value) => value,
                            hir::expr::ObjectEntry::Property(field) => field.expr,
                        });
                    }
                }
                H::CollectedProperties(fields) => {
                    pending.extend(fields.iter().map(|field| field.expr))
                }
                H::Semi(value)
                | H::TacticArgument(value)
                | H::InlineControl(value)
                | H::Defer(value)
                | H::TyPtr(value)
                | H::TyOptional(value)
                | H::TyTraitObject(value)
                | H::LiftType(value)
                | H::Unary(_, value)
                | H::Projection(value, _)
                | H::Ref(value)
                | H::Deref(value)
                | H::ErrorNew(value)
                | H::ErrorPropagate(value)
                | H::OptionPropagate(value)
                | H::EffectRequest(value)
                | H::Assumes(value)
                | H::Asserts(value)
                | H::Invariant(value)
                | H::Decreases(value)
                | H::TakeOld(value)
                | H::TakeBitVec(value)
                | H::TakeTagName(value)
                | H::TakeAttribute(value)
                | H::TakeIntrinsic(value)
                | H::TakeTupleEnumerate(value)
                | H::TakeTupleIndexed(value)
                | H::TypeOf(value)
                | H::MultiDeref(value)
                | H::LiteralConversion { literal: value, .. } => pending.push(value),
                H::Return(value) | H::Resume(value) => pending.extend(*value),
                H::Let(decl) => {
                    self.pattern_requirement_expressions(&decl.pattern, &mut pending)?;
                    pending.extend(decl.ty);
                    pending.extend(decl.init);
                    pending.extend(decl.guard);
                    pending.extend(decl.otherwise);
                }
                // Nested function/quote bodies own their staging requirements.
                _ => {}
            }
        }
        Ok(false)
    }

    /// Pattern computations participate in staging just like expression selectors.
    /// Walk iteratively and charge the same budget as the surrounding body scan.
    fn pattern_requirement_expressions<'a, 'h: 'a>(
        &mut self,
        pattern: &hir::pattern::Pattern<'h>,
        expressions: &mut Vec<&'a Expr<'h>>,
    ) -> Result<(), Error> {
        use hir::pattern::{BitSegment, PatternKind as P};
        let mut pending = vec![pattern];
        while let Some(pattern) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(pattern.span, "pattern requirement scan budget exhausted"))?;
            match &pattern.kind {
                P::Const(value) | P::Comptime(value) | P::CodeSplice(value, _) => {
                    expressions.push(value)
                }
                P::Then(first, value, next) => {
                    pending.extend([*first, *next]);
                    expressions.push(value);
                }
                P::Concat(first, next) => {
                    pending.extend([*first, *next]);
                }
                P::Guard(inner, value) | P::Typed(inner, value) => {
                    pending.push(inner);
                    expressions.push(value);
                }
                P::Binding(_, _, inner) => pending.extend(*inner),
                P::Projection(inner, _)
                | P::OptionSome(inner)
                | P::ErrorOk(inner)
                | P::ErrorErr(inner)
                | P::Not(inner)
                | P::Ref(inner) => pending.push(inner),
                P::Tuple(fields) | P::Or(fields) => pending.extend(fields.iter()),
                P::List(fields, rest) => {
                    pending.extend(fields.iter());
                    pending.extend(*rest);
                }
                P::AppTuple(head, fields) => {
                    pending.push(head);
                    pending.extend(fields.iter());
                }
                P::Struct(head, fields, _) | P::AppStruct(head, fields) => {
                    pending.push(head);
                    pending.extend(fields.iter().map(|field| &field.pat));
                }
                P::NormalForm(head, arguments) => {
                    expressions.push(head);
                    pending.extend(arguments.iter().map(|argument| &argument.pattern));
                }
                P::Range(start, end, _) => {
                    expressions.extend(*start);
                    expressions.extend(*end);
                }
                P::BitVec { segments, .. } => {
                    for segment in segments {
                        match segment {
                            BitSegment::Typed { pattern, ty } => {
                                pending.push(pattern);
                                expressions.push(ty);
                            }
                            BitSegment::Computed(value) => expressions.push(value),
                            BitSegment::Literal { .. } => {}
                        }
                    }
                }
                P::Wild | P::OptionNull | P::Async | P::Invalid => {}
            }
        }
        Ok(())
    }

    fn is_runtime_specialization(&self, expression: &Expr<'_>) -> bool {
        let H::NFApplication(head, _) = expression.kind else {
            return false;
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return false;
        };
        let Some(ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package
                .definition_owner(def_id)
                .and_then(|owner| self.package.item(owner))
                .map(|item| &item.kind)
        else {
            return false;
        };
        !sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head && !sig.modifiers.is_quote
    }

    fn forwarded_call_demands<'a>(
        &mut self,
        target: OwnerId,
        cache: &'a mut BTreeMap<OwnerId, BTreeSet<usize>>,
        span: Span,
    ) -> Result<&'a BTreeSet<usize>, Error> {
        use std::collections::btree_map::Entry;
        Ok(match cache.entry(target) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let demands = match self
                    .signatures
                    .get(&target)
                    .and_then(dependent_inputs::CallSignature::of)
                {
                    Some(signature) => signature.demanded_inputs(
                        self.capabilities
                            .get(&target)
                            .map(Vec::as_slice)
                            .unwrap_or_default(),
                        self.requirements
                            .get(&target)
                            .map(Vec::as_slice)
                            .unwrap_or_default(),
                        &mut self.required_steps,
                        span,
                    )?,
                    None => BTreeSet::new(),
                };
                entry.insert(demands)
            }
        })
    }

    // A fixed outer signature can still contain a call whose domain needs a
    // forwarded staged input. Propagate that requirement through direct calls
    // before checking bodies, independently of declaration order.
    pub(super) fn prepare_staged_forwarders(
        &mut self,
        owners: impl IntoIterator<Item = OwnerId>,
    ) -> Result<(), Error> {
        let mut callers: BTreeMap<OwnerId, BTreeSet<OwnerId>> = BTreeMap::new();
        let mut target_demands = BTreeMap::new();
        let mut needs_instance = BTreeSet::new();
        for owner in owners {
            if self.open_functions.contains_key(&owner) {
                continue;
            }
            let Some(ItemKind::Fn(sig, body) | ItemKind::Case(sig, body)) =
                self.package.item(owner).map(|item| &item.kind)
            else {
                continue;
            };
            if sig.modifiers.is_comptime
                || sig.modifiers.is_normal_form_head
                || sig.modifiers.is_quote
            {
                continue;
            }
            let mut staged: BTreeSet<_> = sig
                .params
                .iter()
                .filter(|parameter| parameter.ty.is_comptime())
                .map(|parameter| local(parameter.hir_id))
                .collect();
            let has_clauses = !self.type_parameters[&owner].is_empty();
            if staged.is_empty() && !has_clauses {
                continue;
            }
            let body = self
                .package
                .body(*body)
                .ok_or_else(|| error(sig.span, "missing staged function body"))?;
            let mut calls = Vec::new();
            let mut clause_dependencies = BTreeSet::new();
            let mut aliases = Vec::new();
            for root in
                std::iter::once(body.value).chain(sig.params.iter().filter_map(|parameter| {
                    match parameter.ty.kind {
                        TyParamKind::Optional(_, _, value) => Some(value),
                        _ => None,
                    }
                }))
            {
                self.scan_body_requirement(root, |this, expression| {
                    if let H::Let(binding) = &expression.kind {
                        if !binding.mutable {
                            if let Some(init) = binding.init {
                                aliases.push((pattern_binding_ids(&binding.pattern), init.clone()));
                            }
                        }
                    }
                    if this.is_runtime_specialization(expression) {
                        // Returning a specialized function or binding it locally
                        // forms the same instance as using it directly in a call.
                        clause_dependencies
                            .extend(this.expression_free_local_dependencies(owner, expression)?);
                    }
                    let (callee, arguments) = match &expression.kind {
                        H::Application(callee, arguments)
                        | H::ExtendedApplication(callee, arguments) => (*callee, *arguments),
                        _ => return Ok(false),
                    };
                    let head = match callee.kind {
                        H::NFApplication(head, _) => head,
                        _ => callee,
                    };
                    let H::ResolvedDef { def_id, .. } = head.kind else {
                        return Ok(false);
                    };
                    let Some(target) = this.package.definition_owner(def_id) else {
                        return Ok(false);
                    };
                    calls.push((
                        target,
                        callee.clone(),
                        arguments.to_vec(),
                        expression.span,
                        matches!(expression.kind, H::ExtendedApplication(..)),
                    ));
                    Ok(false)
                })?;
            }
            let mut computed_inputs = BTreeSet::new();
            let mut computed_clause = false;
            let mut routed_calls = Vec::new();
            for (target, callee, arguments, span, extended) in calls {
                let mut forwarded_clause = false;
                let mut inputs = if self.is_runtime_specialization(&callee) {
                    BTreeSet::new()
                } else {
                    self.expression_free_local_dependencies(owner, &callee)?
                };
                // Only forwarding into a staged slot postpones type formation.
                // Supplying a known staged value as an ordinary runtime input
                // must not turn an otherwise closed function into a template.
                if let Some(
                    ItemKind::Fn(target_sig, _)
                    | ItemKind::Case(target_sig, _)
                    | ItemKind::FnDeclaration(target_sig),
                ) = self.package.item(target).map(|item| &item.kind)
                {
                    // Colon properties belong to the collection entrance, not
                    // named parameter slots. Normalize their dependency route
                    // before looking for forwarded staged inputs.
                    let properties = arguments
                        .iter()
                        .filter_map(|argument| match argument {
                            Arg::Named(name, value) if extended => Some(hir::expr::FieldExpr {
                                ident: *name,
                                expr: value,
                                span: value.span,
                            }),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let packet = Expr {
                        hir_id: callee.hir_id,
                        span,
                        kind: H::CollectedProperties(&properties),
                    };
                    let collectors = target_sig
                        .params
                        .iter()
                        .filter(|parameter| matches!(parameter.ty.kind, TyParamKind::Varadic(..)))
                        .map(|parameter| (parameter.name, parameter.index as usize))
                        .collect::<Vec<_>>();
                    let collector = if properties.is_empty() {
                        None
                    } else {
                        self.property_collector(target, &collectors, None, None, None, span)
                            .ok()
                    };
                    let arguments = if let Some((name, _)) = collector {
                        let mut supplied = false;
                        arguments
                            .iter()
                            .filter_map(|argument| match argument {
                                Arg::Named(..) if !supplied => {
                                    supplied = true;
                                    Some(Arg::Named(name, &packet))
                                }
                                Arg::Named(..) => None,
                                other => Some(other.clone()),
                            })
                            .collect::<Vec<_>>()
                    } else {
                        arguments
                    };
                    let flat = if let Some(flat) = arguments::literal_spread_arguments(&arguments) {
                        Some(flat)
                    } else {
                        // Dependency scanning needs source provenance and slot
                        // widths, not executable unpacking. Repeated references
                        // here are never used to build or evaluate the call.
                        let mut flat = Vec::new();
                        let mut complete = true;
                        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                        let before = fuel;
                        for argument in &arguments {
                            match argument {
                                Arg::Expand(value) => {
                                    if let H::Tuple(fields) = value.kind {
                                        flat.extend(fields.iter().map(Arg::Positional));
                                    } else if let Some(width) =
                                        self.spread_width(value, &mut fuel, 0)?
                                    {
                                        if width > fuel {
                                            return Err(error(
                                                value.span,
                                                "staged spread routing budget exhausted",
                                            ));
                                        }
                                        fuel -= width;
                                        flat.extend((0..width).map(|_| Arg::Positional(value)));
                                    } else {
                                        complete = false;
                                        break;
                                    }
                                }
                                other => flat.push(other.clone()),
                            }
                        }
                        self.required_steps -= (before - fuel) as u64;
                        complete.then_some(flat)
                    };
                    // Fixed anchors remain unambiguous while explicit clauses
                    // are still needed to distinguish collection entrances.
                    let flat_inputs = flat.as_deref().unwrap_or(&arguments);
                    let post = arguments::post_lambda_target(
                        flat_inputs,
                        Some(target_sig.params),
                        target_sig.params.len(),
                        span,
                    )
                    .ok()
                    .flatten();
                    let positional = target_sig
                        .params
                        .iter()
                        .enumerate()
                        .filter(|(index, parameter)| {
                            Some(*index) != post
                                && !parameter.ty.is_implicit()
                                && !matches!(parameter.ty.kind, TyParamKind::Optional(..))
                        })
                        .map(|(_, parameter)| parameter)
                        .collect::<Vec<_>>();
                    for reverse in [false, true] {
                        let parameters: Box<dyn Iterator<Item = _>> = if reverse {
                            Box::new(positional.iter().rev())
                        } else {
                            Box::new(positional.iter())
                        };
                        let arguments: Box<dyn Iterator<Item = _>> = if reverse {
                            Box::new(flat_inputs.iter().rev())
                        } else {
                            Box::new(flat_inputs.iter())
                        };
                        for (parameter, argument) in parameters
                            .take_while(|parameter| {
                                !matches!(parameter.ty.kind, TyParamKind::Varadic(..))
                            })
                            .zip(
                                arguments
                                    .take_while(|argument| !matches!(argument, Arg::Expand(_)))
                                    .filter_map(|argument| match argument {
                                        Arg::Positional(value) => Some(*value),
                                        _ => None,
                                    }),
                            )
                        {
                            if parameter.ty.is_comptime() {
                                inputs.extend(
                                    self.expression_free_local_dependencies(owner, argument)?,
                                );
                            }
                        }
                    }
                    if let Ok((routes, _)) = route_arguments(
                        flat.as_deref().unwrap_or(&arguments),
                        Some(target_sig.params),
                        target_sig.params.len(),
                        span,
                    ) {
                        let demanded = if !target_sig.params.iter().any(|p| p.ty.is_comptime())
                            || target_sig.modifiers.is_comptime
                            || target_sig.modifiers.is_normal_form_head
                            || target_sig.modifiers.is_quote
                        {
                            None
                        } else {
                            Some(self.forwarded_call_demands(target, &mut target_demands, span)?)
                        };
                        for route in routes
                            .iter()
                            .filter(|route| target_sig.params[route.parameter].ty.is_comptime())
                        {
                            for (_, value) in route.source_inputs() {
                                // Direct staged binders already carry their symbolic value.
                                // A producer depending on them may have no identity until
                                // this invocation supplies the enclosing inputs.
                                let carries_identity = matches!(
                                    value.kind,
                                    H::ResolvedClauseParam(..) | H::ResolvedFnParam(..)
                                );
                                let computed = !carries_identity
                                    && demanded
                                        .is_some_and(|slots| slots.contains(&route.parameter));
                                if computed || has_clauses {
                                    let uses = self.source_expression_inputs(owner, value)?;
                                    let reads_clause = self.type_parameters[&owner]
                                        .iter()
                                        .any(|id| uses.declarations.contains(id));
                                    forwarded_clause |= reads_clause;
                                    if computed {
                                        computed_clause |= reads_clause;
                                        computed_inputs.extend(uses.locals.iter().copied());
                                    }
                                    inputs.extend(uses.locals);
                                } else {
                                    inputs.extend(
                                        self.expression_free_local_dependencies(owner, value)?,
                                    );
                                }
                            }
                        }
                    }
                }
                routed_calls.push((target, inputs, forwarded_clause));
            }
            let mut alias_dependencies = Vec::new();
            for (bindings, init) in aliases {
                alias_dependencies.push((
                    bindings,
                    self.expression_free_local_dependencies(owner, &init)?,
                ));
            }
            loop {
                let mut changed = false;
                for (bindings, inputs) in &alias_dependencies {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(sig.span, "staged alias propagation budget exhausted")
                    })?;
                    if !inputs.is_empty() && inputs.iter().all(|input| staged.contains(input)) {
                        for binding in bindings {
                            changed |= staged.insert(*binding);
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            for (target, inputs, forwarded_clause) in routed_calls {
                if forwarded_clause || !inputs.is_disjoint(&staged) {
                    callers.entry(target).or_default().insert(owner);
                }
            }
            if computed_clause
                || !computed_inputs.is_disjoint(&staged)
                || !clause_dependencies.is_disjoint(&staged)
            {
                needs_instance.insert(owner);
            }
        }
        for target in self.open_functions.keys() {
            needs_instance.extend(callers.remove(target).unwrap_or_default());
        }
        while let Some(owner) = needs_instance.pop_first() {
            let owner_span = self.open_item(owner, Span::default())?.span;
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(owner_span, "staged body propagation budget exhausted"))?;
            if self.open_functions.contains_key(&owner) {
                continue;
            }
            let signature = self.signatures[&owner].clone();
            let Term::Function { parameters, result } = signature else {
                continue;
            };
            self.register_open_function(
                owner,
                parameters
                    .into_iter()
                    .map(|parameter| (parameter.sort, Annotation::Known(parameter.ty)))
                    .collect(),
                Annotation::Known(*result),
                owner_span,
            )?;
            needs_instance.extend(callers.remove(&owner).unwrap_or_default());
        }
        Ok(())
    }

    pub(super) fn register_open_function(
        &mut self,
        owner: OwnerId,
        parameters: Vec<(Sort, Annotation)>,
        result: Annotation,
        span: Span,
    ) -> Result<(), Error> {
        let sig = match self.package.item(owner).map(|item| &item.kind) {
            Some(ItemKind::Fn(sig, _) | ItemKind::Case(sig, _)) => sig,
            Some(ItemKind::FnDeclaration(sig)) if sig.modifiers.is_function_template => sig,
            _ => {
                return Err(error(
                    span,
                    "deferred interface and effect signatures are not yet implemented",
                ));
            }
        };
        if self.package.function_template_bodies.contains_key(&owner)
            && (parameters
                .iter()
                .any(|(_, annotation)| annotation.needs_instance())
                || result.needs_instance())
        {
            return Err(error(
                span,
                "shared function templates with deferred interface descriptions are not yet implemented",
            ));
        }
        if sig.modifiers.is_normal_form_head
            || ((self.self_containers.contains_key(&owner) || self.self_types.contains_key(&owner))
                && !matches!(sig_item_kind(self.package.item(owner)), Some(true)))
        {
            return Err(error(
                span,
                "open signatures with normal-form bodies are not yet implemented",
            ));
        }
        fn sig_item_kind(item: Option<&hir::item::Item<'_>>) -> Option<bool> {
            item.map(|item| matches!(item.kind, ItemKind::Fn(..)))
        }
        if sig.modifiers.is_pure
            && (parameters
                .iter()
                .any(|(_, ty)| !matches!(ty, Annotation::Known(_)))
                || !matches!(result, Annotation::Known(_)))
        {
            return Err(error(
                span,
                "pure open definitions with pending domains require definition-side behavior evidence not yet represented",
            ));
        }
        // Lambda and explicitly supplied implicit roles use the shared router.
        // Concrete calls form the domain before checking the input. Missing
        // implicit inputs are rejected by routing until a provider is installed.
        for parameter in sig.params {
            if !matches!(
                parameter.ty.kind,
                TyParamKind::Itself { .. }
                    | TyParamKind::Positional(_)
                    | TyParamKind::PositionalDependencyCatched(..)
                    | TyParamKind::Optional(..)
                    | TyParamKind::Varadic(..)
            ) {
                return Err(error(
                    parameter.span,
                    "open signature parameter routing protocols are not yet implemented",
                ));
            }
        }
        self.open_functions
            .insert(owner, Definition::new(parameters, result));
        Ok(())
    }

    pub(super) fn check_open_definition(&mut self, owner: OwnerId) -> Result<(), Error> {
        let definition = &self.open_functions[&owner];
        let needs_default_behavior = self.default_behavior_functions.contains(&owner)
            && !self.ordinary_behavior_functions.iter().any(|function| {
                function.declaration == declaration(self.package, &self.package_identities, owner)
            });
        if definition.checked && !needs_default_behavior {
            return Ok(());
        }
        let definition = self.check_definition_source(owner, definition.clone())?;
        self.open_functions.insert(owner, definition);
        Ok(())
    }

    /// Only certified regions may omit a success continuation. A closed
    /// source function retains its ordinary ABI and declaration identity. Its
    /// first selected failure (or ordinary purity obligation) requests the
    /// same formal check as an open body.
    pub(super) fn checked_const_continuation(&mut self, block: hir::HirId) -> Result<bool, Error> {
        if self.in_code_body {
            return Ok(false);
        }
        if let Some(frame) = self.captures.last() {
            return Ok(frame.const_regions.contains(block));
        }
        // A default is specialized in its declaration's input frame before
        // the callee's body instance exists. Both paths consume the proof
        // attached to this exact source block by definition checking.
        let owner = self
            .open_instance
            .as_ref()
            .map(|instance| instance.owner)
            .or(self.default_input_owner);
        if let Some(definition) = owner.and_then(|owner| self.open_functions.get(&owner)) {
            return Ok(definition.checked_const_continuation(block));
        }
        let Some(owner) = self
            .signature_owner
            .filter(|_| self.definition_refinements.closed_source_body)
        else {
            return Ok(false);
        };
        self.check_closed_const_regions(owner)?;
        Ok(self.closed_const_regions[&owner]
            .const_continuation_blocks
            .contains(&local(block)))
    }

    pub(super) fn check_closed_const_regions(&mut self, owner: OwnerId) -> Result<(), Error> {
        if !self.closed_const_regions.contains_key(&owner) {
            let owner_span = self.open_item(owner, Span::default())?.span;
            let Term::Function { parameters, result } = self
                .declared_signature(owner)
                .ok_or_else(|| error(owner_span, "missing closed source signature"))?
                .into_owned()
            else {
                return Err(error(
                    owner_span,
                    "closed source declaration must have a function signature",
                ));
            };
            let definition = Definition::new(
                parameters
                    .into_iter()
                    .map(|p| (p.sort, Annotation::Known(p.ty)))
                    .collect(),
                Annotation::Known(*result),
            );
            // An independent declaration check must not use evidence learned
            // from the selected branch, including entry values and loop exits.
            let definition = self.in_formal_declaration(owner, |this| {
                this.check_definition_source(owner, definition)
            })?;
            self.closed_const_regions.insert(owner, definition.regions);
        }
        Ok(())
    }

    fn check_definition_source(
        &mut self,
        owner: OwnerId,
        mut definition: Definition,
    ) -> Result<Definition, Error> {
        let span = self.open_item(owner, Span::default())?.span;
        self.ensure_contract(owner, span)?;
        if self.active_open_definitions.len() >= 64 || !self.active_open_definitions.insert(owner) {
            return Err(error(
                span,
                "cyclic or excessively deep ordinary open-definition dependency",
            ));
        }
        let checked = self.in_open_function_scope(owner, None, |this| {
            let item = this.open_item(owner, span)?;
            let (sig, body) = match &item.kind {
                ItemKind::Fn(sig, body) => (sig, Some(*body)),
                ItemKind::FnDeclaration(sig) if sig.modifiers.is_function_template => (sig, None),
                _ => return Err(error(span, "expected open function definition")),
            };
            this.unsafe_depth = usize::from(sig.modifiers.is_unsafe);
            let metadata = n::Package {
                target: this.target,
                structs: this.structs.values().cloned().collect(),
                enums: this.enums.values().cloned().collect(),
                normal_forms: this.normal_form_metadata(),
                ..Default::default()
            };
            let frames = vec![
                definition
                    .parameters
                    .iter()
                    .map(|(sort, _)| *sort)
                    .collect(),
            ];
            if !sig.modifiers.is_comptime {
                for (_, annotation) in definition
                    .parameters
                    .iter()
                    .filter(|(sort, _)| *sort == Sort::Runtime)
                    .chain(std::iter::once(&(Sort::Runtime, definition.result.clone())))
                {
                    if let Annotation::Known(ty) = annotation
                        && !ty.structural_terms().any(|term| matches!(term, Term::AssociatedTypeCall(_)))
                    {
                        nir::phases::validate_runtime_storage(ty, &metadata, &frames)
                            .map_err(|message| error(item.span, message))?;
                    }
                }
            }
            let mut env_parameters = Vec::new();
            if !sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head {
                let type_params = this.type_parameters[&owner].clone();
                let domains = this.trait_domains(owner)?;
                for (decl_id, domain) in type_params.iter().zip(&domains) {
                    let Term::Lift(inner) = domain else {
                        continue;
                    };
                    let inner_ty = this.callable_input_view(inner, item.span)?;
                    let binding = this.spread_binding(item.span)?;
                    this.locals.insert(binding, inner_ty.clone());
                    this.immutable.insert(binding);
                    this.active_env_parameters
                        .insert(*decl_id, (binding, inner_ty.clone()));
                    env_parameters.push(n::RuntimeEnvParameter {
                        declaration: *decl_id,
                        runtime_id: None,
                        binding,
                        ty: inner_ty,
                    });
                }
            }
            let mut locals = BTreeMap::new();
            let mut default_types = BTreeMap::new();
            let mut default_dependencies = BTreeMap::new();
            let mut suspended_facts = BTreeMap::new();
            for (slot, (parameter, (sort, annotation))) in
                sig.params.iter().zip(&definition.parameters).enumerate()
            {
                if let Some(facts) = this.check_suspended_annotation(annotation, sig, &mut locals)? {
                    suspended_facts.insert(slot, facts);
                }
                let description = annotation.description();
                if let TyParamKind::Optional(_, _, default) = parameter.ty.kind {
                    let mut default_locals = locals.clone();
                    // A supplier owns its abstract staged operands. They are
                    // not extra parameters of the function body or its siblings.
                    let enclosing_inputs = this.ordinary_behavior_inputs.clone();
                    let staged_count = this.definition_refinements.staged_values.len();
                    let actual = this.with_staged_input(
                        sig.modifiers.is_comptime || *sort != Sort::Runtime,
                        |this| {
                            let actual = this.describe_open_expression(
                                default,
                                &mut default_locals,
                                None,
                                Some(&description),
                                0,
                            )?;
                            if !sig.modifiers.is_comptime
                                && *sort == Sort::Runtime
                                && matches!(actual, D::Unknown)
                            {
                                return Err(error(
                                    default.span,
                                    "an unknown domain supplies no P3 stability evidence for this default",
                                ));
                            }
                            let expected = match &description {
                                D::Known(ty) => Some(ty),
                                D::Unknown => None,
                            };
                            let saved_call =
                                std::mem::replace(&mut this.describing_open_call, true);
                            let saved_behavior =
                                std::mem::replace(&mut this.checking_default_behavior, true);
                            let saved_open_behavior = std::mem::replace(
                                &mut this.checking_open_behavior,
                                !sig.modifiers.is_comptime && *sort == Sort::Runtime,
                            );
                            let value = this.expr(default, expected);
                            this.checking_open_behavior = saved_open_behavior;
                            this.checking_default_behavior = saved_behavior;
                            this.describing_open_call = saved_call;
                            let value = value?;
                            let mut fuel =
                                usize::try_from(this.required_steps).unwrap_or(usize::MAX);
                            let before = fuel;
                            let references = nir::preparation::references(&value, &mut fuel);
                            this.required_steps -= (before - fuel) as u64;
                            let mut references = references.map_err(|e| error(default.span, e))?;
                            // The behavior view abstracts inline selectors; the
                            // original source still determines which earlier
                            // inputs must be available for actual selection.
                            references
                                .extend(this.expression_free_local_dependencies(owner, default)?);
                            let mut dependencies: BTreeSet<_> = sig
                                .params
                                .iter()
                                .enumerate()
                                .filter_map(|(index, parameter)| {
                                    references
                                        .contains(&local(parameter.hir_id))
                                        .then_some(index)
                                })
                                .collect();
                            dependencies.extend(
                                this.checked_substitution_inputs(&value, default.span)?
                                    .into_keys(),
                            );
                            if dependencies.iter().any(|dependency| *dependency >= slot) {
                                return Err(error(
                                    default.span,
                                    "default input dependency must precede its parameter",
                                ));
                            }
                            default_dependencies.insert(slot, dependencies);
                            if !sig.modifiers.is_comptime && *sort == Sort::Runtime {
                                this.require_stable_default(&value, default.span)?;
                            }
                            // Keep the whole default together so branches retain
                            // their proof facts. This checks operations, not values:
                            // an overridden comptime supplier still never executes.
                            this.check_open_fragment(&value, default.span, true)?;
                            Ok(actual)
                        },
                    );
                    this.ordinary_behavior_inputs = enclosing_inputs;
                    this.definition_refinements.staged_values.truncate(staged_count);
                    if let D::Known(ty) = actual? {
                        default_types.insert(slot, ty);
                    }
                }
                if let D::Known(ty) = &description {
                    let binding = local(parameter.hir_id);
                    this.locals.insert(binding, ty.clone());
                    this.input_facts.parameters.bind(binding, slot, *sort, ty);
                }
                locals.insert(local(parameter.hir_id), description);
            }
            // Defaults are suppliers. Their checks must not borrow a premise
            // that only holds after binding the parameter at function entry.
            for (parameter, (_, annotation)) in sig.params.iter().zip(&definition.parameters) {
                let facts = match annotation {
                    Annotation::Description(description @ n::DescriptionLiteral::Refined(_)) => {
                        let subject = this.assume_parameter_refinement(
                            parameter.hir_id, description, parameter.span,
                        )?;
                        this.locals.insert(local(parameter.hir_id), subject.clone());
                        locals.insert(local(parameter.hir_id), D::Known(subject));
                        continue;
                    }
                    Annotation::Pending(prepared) => prepared.direct_call_result_guarantees(
                        &mut this.required_steps, parameter.span,
                    )?,
                    Annotation::Suspended { slot, .. } => suspended_facts.remove(slot),
                    _ => None,
                };
                if let Some((binder, guarantees)) = facts {
                    let subject = this.assume_deferred_result_guarantees(
                        parameter.hir_id, binder, &guarantees, parameter.span,
                    )?;
                    this.locals.insert(local(parameter.hir_id), subject.clone());
                    locals.insert(local(parameter.hir_id), D::Known(subject));
                }
            }
            this.check_suspended_annotation(&definition.result, sig, &mut locals)?;
            let preconditions = this
                .program_preconditions(owner)
                .map_err(open_contract_error)?;
            if let Some(predicate) = preconditions.iter().cloned().reduce(|left, right| n::Expr {
                source: source(item.span),
                ty: p(P::Bool),
                kind: K::And(Box::new(left), Box::new(right)),
            }) {
                // Validate entry predicates before assuming them, preserving
                // short-circuit facts from earlier conjuncts.
                this.check_open_fragment(&predicate, item.span, true)?;
            }
            let result_evidence = match &definition.result {
                Annotation::Known(ty) => sig.return_binding.map(|binding| (local(binding), ty)),
                Annotation::Description(_) | Annotation::Pending(_)
                | Annotation::InputType { .. }
                | Annotation::Suspended { .. } => None,
            };
            let (postconditions, entry_snapshots) = this
                .program_postconditions_with_result(owner, result_evidence)
                .map_err(open_contract_error)?;
            let Some(body) = body else {
                // A signature template has no executable body. Its annotation,
                // default and contract checks above still apply; each instance
                // independently checks its supplied body in the inherited domain.
                return Ok((default_types, default_dependencies, CheckedRegions::default()));
            };
            let body = this
                .package
                .body(body)
                .ok_or_else(|| error(item.span, "missing open function body"))?;
            let declared_result = match definition.result.description() {
                D::Known(ref ty)
                    if ty.structural_terms().any(|term| {
                        matches!(
                            term,
                            Term::AssociatedTypeCall(call)
                                if call.call_arguments.iter().any(|arg| arg.check_scope(&[]).is_err())
                        )
                    }) =>
                {
                    D::Unknown
                }
                other => other,
            };
            let result = match &declared_result {
                D::Known(Term::Computation { effects, result }) => {
                    this.effect_environment = Some(effects.clone());
                    D::Known(result.as_ref().clone())
                }
                other => other.clone(),
            };
            this.control.result = match &result {
                D::Known(ty) => Some(ty.clone()),
                D::Unknown => None,
            };
            let template_result = match &declared_result {
                D::Known(ty) => Some(ty.clone()),
                D::Unknown => None,
            };
            let check_body = |this: &mut Self| -> Result<(), Error> {
                this.describe_open_expression(
                    body.value,
                    &mut locals,
                    Some(&result),
                    Some(&result),
                    0,
                )?;
                if sig.modifiers.is_pure || this.default_behavior_functions.contains(&owner) {
                    let D::Known(result_type) = declared_result else {
                        return Err(error(
                            span,
                            "pure open definition behavior check requires known result domain",
                        ));
                    };
                    this.checking_open_behavior = true;
                    this.in_unselected_inline = true;
                    this.inline_selection = Some((
                        hir::HirId::new(owner, hir::ItemLocalId::new(u32::MAX)),
                        usize::MAX,
                    ));
                    let behavior = this.computation_body(body.value, Some(&result_type));
                    this.checking_open_behavior = false;
                    let behavior = behavior?;
                    let mut parameters: Vec<_> = definition
                        .parameters
                        .iter()
                        .map(|(sort, annotation)| {
                            let Annotation::Known(ty) = annotation else {
                                return Err(error(
                                    span,
                                    "pure open definition behavior check requires known parameter domain",
                                ));
                            };
                            Ok(Parameter {
                                sort: *sort,
                                ty: ty.clone(),
                            })
                        })
                        .collect::<Result<Vec<_>, Error>>()?;
                    let inputs = std::mem::take(&mut this.ordinary_behavior_inputs);
                    parameters.extend(inputs.values().cloned().map(|ty| Parameter {
                        sort: Sort::ComptimeValue,
                        ty,
                    }));
                    let abstract_inputs = this.abstract_code_inputs(&behavior, item.span)?;
                    parameters.extend(abstract_inputs.values().cloned().map(|ty| Parameter {
                        sort: Sort::Runtime,
                        ty,
                    }));
                    let bindings = sig
                        .params
                        .iter()
                        .map(|param| local(param.hir_id))
                        .chain(inputs.keys().copied())
                        .chain(abstract_inputs.keys().copied())
                        .collect();
                    let decl_id =
                        declaration(this.package, &this.package_identities, owner);
                    let decreases = this.program_decreases(owner)?;
                    if sig.modifiers.is_verified {
                        this.safety_contracts
                            .levels
                            .insert(decl_id, n::SafetyLevel::Verified);
                    } else if sig.modifiers.is_unsafe {
                        this.safety_contracts
                            .levels
                            .insert(decl_id, n::SafetyLevel::Unsafe);
                    }
                    if !decreases.is_empty() {
                        this.safety_contracts.decreases.insert(decl_id, decreases);
                    }
                    this.ordinary_behavior_functions.push(n::Function {
                        interface: nir::program::FunctionDeclaration {
                            source: source(item.span),
                            declaration: decl_id,
                            name: format!("{} [ordinary]", item.ident.name),
                            phase: n::FunctionPhase::Runtime,
                            pure: sig.modifiers.is_pure,
                            is_unsafe: sig.modifiers.is_unsafe,
                            inline: false,
                            signature: Term::Function {
                                parameters,
                                result: Box::new(result_type),
                            },
                            parameters: bindings,
                            env_parameters,
                            defaults: vec![],
                            type_parameters: this.type_parameters[&owner].clone(),
                            preconditions,
                            postconditions,
                            entry_snapshots,
                            result_binding: sig.return_binding.map(local),
                            result_type_binder: None,
                            result_guarantees: vec![],
                            requirements: this.definition_requirements(owner),
                            capabilities: this
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                        },
                        body: behavior,
                        runtime_deliveries: Default::default(),
                    });
                }
                Ok(())
            };
            if let Some(result) = template_result {
                this.with_function_template_body(owner, &result, check_body)?;
            } else {
                check_body(this)?;
            }
            Ok((
                default_types,
                default_dependencies,
                std::mem::take(&mut this.definition_refinements.regions),
            ))
        });
        self.active_open_definitions.remove(&owner);
        let (default_types, default_dependencies, regions) = checked?;
        definition.default_types = default_types;
        definition.default_dependencies = default_dependencies;
        definition.regions = regions;
        definition.checked = true;
        Ok(definition)
    }

    fn require_description(
        &self,
        actual: &D,
        expected: Option<&D>,
        span: Span,
    ) -> Result<(), Error> {
        if let (D::Known(actual), Some(D::Known(expected))) = (actual, expected) {
            if !is_never(actual) && matches!(expected, Term::ErrorResult { .. }) {
                let mut unwrapped = actual;
                if let Some(effect_env) = self.active_effect_environment()
                    && let Term::Computation { effects, result } = actual
                {
                    crate::effects::embed_row(
                        effects,
                        effect_env,
                        &self.context,
                        crate::ptc::Budget::default(),
                    )
                    .map_err(|failure| error(span, failure.to_string()))?;
                    unwrapped = result;
                }
                if !is_never(unwrapped) {
                    let conversion = if matches!(unwrapped, Term::ErrorResult { .. }) {
                        nir::error_results::Conversion::Embed
                    } else {
                        nir::error_results::Conversion::Success
                    };
                    let frames = self.context.scope().to_vec();
                    return nir::error_results::validate_conversion_in_scope(
                        conversion,
                        unwrapped,
                        expected,
                        &frames,
                        &mut 100_000,
                        &mut |a, b| self.require(a, b, span).is_ok(),
                    )
                    .map_err(|e| {
                        error(
                            span,
                            format!(
                                "error result conversion: {e}; input {unwrapped}; output {expected}"
                            ),
                        )
                    });
                }
            }
            self.require_expression(actual, expected, span)?;
        }
        Ok(())
    }

    pub(super) fn assume_open_continuation(&mut self, expr: &Expr<'_>) -> Result<(), Error> {
        if let Some(fact) = self.open_continuation_fact(expr, 0)? {
            self.context
                .validate_formula(&fact, &mut crate::ptc::Budget::default())
                .map_err(|failure| error(expr.span, failure.to_string()))?;
            self.context.assume(fact);
        }
        Ok(())
    }

    fn open_continuation_fact(
        &mut self,
        expr: &Expr<'_>,
        depth: usize,
    ) -> Result<Option<crate::ptc::Formula>, Error> {
        use crate::ptc::Formula as F;
        if depth >= 128 {
            return Err(error(expr.span, "open continuation fact depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(expr.span, "open continuation fact budget exhausted"))?;
        Ok(match &expr.kind {
            H::Assumes(condition) if self.is_type_assumption(condition) => {
                Some(F::from_predicate(&self.type_predicate(condition)?))
            }
            H::Semi(value) => self.open_continuation_fact(value, depth + 1)?,
            H::Let(binding) => match binding.init {
                Some(value) => self.open_continuation_fact(value, depth + 1)?,
                None => None,
            },
            H::Block(block) => {
                let mut facts = Vec::new();
                for statement in block.stmts.iter().chain(block.expr) {
                    if let Some(fact) = self.open_continuation_fact(statement, depth + 1)? {
                        facts.push(fact);
                    }
                }
                if facts.is_empty() {
                    None
                } else {
                    Some(F::And(facts))
                }
            }
            H::If(_, yes, no) => {
                let yes_expr = Expr {
                    kind: H::Block(yes),
                    ..expr.clone()
                };
                let yes_fact = self.open_continuation_fact(&yes_expr, depth + 1)?;
                let no_fact = match no {
                    Some(no) => self.open_continuation_fact(no, depth + 1)?,
                    None => None,
                };
                if source_always_exits(&yes_expr, depth + 1) {
                    no_fact
                } else if no.is_some_and(|no| source_always_exits(no, depth + 1)) {
                    yes_fact
                } else {
                    match (yes_fact, no_fact) {
                        (Some(yes), Some(no)) if yes == no => Some(yes),
                        (Some(yes), Some(no)) => Some(F::Or(vec![yes, no])),
                        _ => None,
                    }
                }
            }
            // Loop bodies can be skipped or left through break; closure bodies
            // have not executed here. Neither contributes continuation facts.
            _ => None,
        })
    }

    /// Keep ordinary type/stage checking of unreachable source, while
    /// excluding its transfers from surrounding loop completion evidence.
    fn with_open_reachability<T>(
        &mut self,
        reachable: bool,
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if reachable || self.open_loop_transfers.is_empty() {
            return check(self);
        }
        self.required_steps = self
            .required_steps
            .checked_sub(self.open_loop_transfers.len() as u64)
            .ok_or_else(|| error(span, "loop completion description budget exhausted"))?;
        let saved = self.open_loop_transfers.clone();
        let result = check(self);
        self.open_loop_transfers = saved;
        result
    }

    fn describe_open_block(
        &mut self,
        block: &hir::expr::Block<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let saved = locals.clone();
        let saved_known = self.locals.clone();
        let saved_staged_locals = self.open_staged_locals.clone();
        let saved_staged_locals_members = self.open_member_locals.clone();
        let saved_context = self.context.clone();
        let saved_req_len = self.definition_refinements.requirements.len();
        let saved_generator_data = self.generator_data.clone();
        let outer_conditional =
            std::mem::take(&mut self.definition_refinements.block_contains_conditional_const);
        let result = (|| {
            let mut completing = true;
            for statement in block.stmts {
                let ty = self.with_open_reachability(completing, statement.span, |this| {
                    this.describe_open_expression(statement, locals, returns, None, depth + 1)
                })?;
                if matches!(statement.kind, H::Let(binding) if binding.comptime && binding.otherwise.is_some())
                {
                    self.definition_refinements.block_contains_conditional_const = true;
                }
                completing &= !matches!(ty, D::Known(ref ty) if is_never(ty));
                if completing {
                    self.assume_open_continuation(statement)?;
                }
            }
            let result = match block.expr {
                Some(tail) => self.with_open_reachability(completing, tail.span, |this| {
                    this.describe_open_expression(tail, locals, returns, expected, depth + 1)
                })?,
                None => D::Known(p(P::Void)),
            };
            if completing {
                self.require_description(&result, expected, block.span)?;
                Ok(result)
            } else {
                Ok(D::Known(p(P::Never)))
            }
        })();
        // An instance may omit a continuation only in a block whose source
        // was described with the success bindings. Retain just the enclosing
        // blocks of conditional declarations, including their checked tails.
        let checked =
            result.is_ok() && self.definition_refinements.block_contains_conditional_const;
        if checked {
            self.definition_refinements
                .regions
                .const_continuation_blocks
                .insert(local(block.hir_id));
        }
        self.definition_refinements.block_contains_conditional_const = outer_conditional || checked;
        *locals = saved;
        self.locals = saved_known;
        self.open_staged_locals = saved_staged_locals;
        self.open_member_locals = saved_staged_locals_members;
        self.context = saved_context;
        self.definition_refinements
            .requirements
            .truncate(saved_req_len);
        self.generator_data = saved_generator_data;
        result
    }

    // Keep recursive scalar descriptions off the general statement/call frame.
    // Both dispatchers retain the same checking budget and completion rule.
    #[inline(never)]
    fn describe_open_expression(
        &mut self,
        expr: &Expr<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        if depth >= 128 {
            return Err(error(expr.span, "ordinary deferred body depth exhausted"));
        }
        self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
            error(
                expr.span,
                "ordinary deferred body checking budget exhausted",
            )
        })?;
        if let H::InlineControl(inner) = expr.kind
            && let H::If(condition, yes, no) = inner.kind
            && self.is_type_assumption(condition)
        {
            return self.describe_inline_trait_guard(
                expr, condition, yes, no, locals, returns, expected, depth,
            );
        }
        if super::intrinsics::is_application(expr) {
            return self.describe_known_region(expr, expected);
        }
        if let Some(description) =
            self.describe_pending_function_value(expr, locals, returns, expected, depth)?
        {
            return Ok(description);
        }
        if matches!(expected, Some(D::Unknown))
            && self.protocols.arithmetic[crate::arithmetic::Operator::Invert as usize].is_none()
        {
            // An integer literal has no committed operand type yet. Preserve
            // contextual builtin inversion until the result domain is known;
            // this grants no operation on a variable with an unknown domain. The original
            // source is rechecked when forming each concrete function instance.
            let mut literal = expr;
            let mut inversions = 0usize;
            while let H::Unary(UnOp::Not, operand) = &literal.kind {
                if depth + inversions >= 128 {
                    return Err(error(expr.span, "ordinary deferred body depth exhausted"));
                }
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(
                        expr.span,
                        "ordinary deferred body checking budget exhausted",
                    )
                })?;
                inversions += 1;
                literal = operand;
            }
            if inversions != 0
                && matches!(
                    &literal.kind,
                    H::Lit(hir::Lit {
                        kind: hir::LitKind::Integer(_),
                        ..
                    })
                )
            {
                // Parse and validate the literal now, without fabricating a
                // BitVec width or publishing a placeholder operand type.
                self.describe_known_region(literal, None)?;
                return Ok(D::Unknown);
            }
        }
        match &expr.kind {
            H::TraitBound(..) if self.in_staged_input => {
                // The source instance will decide the query once its inputs
                // are available. Checking its domain here needs a well-formed
                // proposition, but supplies no evidence for either outcome.
                let predicate = self.type_predicate(expr)?;
                let formula = crate::ptc::Formula::from_predicate(&predicate);
                self.context
                    .validate_formula(&formula, &mut crate::ptc::Budget::default())
                    .map_err(|e| error(expr.span, e.to_string()))?;
                let domain = D::Known(p(P::Bool));
                self.require_description(&domain, expected, expr.span)?;
                Ok(domain)
            }
            H::Unary(UnOp::Not | UnOp::Neg, _)
            | H::Binary(
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::Concat
                | BinOp::And
                | BinOp::Or,
                _,
                _,
            ) => self.describe_open_scalar(expr, locals, returns, expected, depth),
            _ => self.describe_open_expression_general(expr, locals, returns, expected, depth),
        }
    }

    #[inline(never)]
    fn describe_open_expression_general(
        &mut self,
        expr: &Expr<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let result = match &expr.kind {
            H::NFApplication(head, arguments) if matches!(head.kind, H::Projection(..)) => {
                let pending = self.pending_member_origin(head, locals)?;
                if !pending {
                    return self.describe_known_region(expr, expected);
                }
                let description =
                    self.describe_open_expression(head, locals, returns, None, depth + 1)?;
                if !matches!(description, D::Unknown) {
                    return self.describe_known_region(expr, expected);
                }
                for argument in *arguments {
                    let value = match argument {
                        Arg::Positional(value)
                        | Arg::Named(_, value)
                        | Arg::Expand(value)
                        | Arg::Implicit(value) => *value,
                        Arg::PostLambda(_) => return self.describe_known_region(expr, expected),
                    };
                    self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                }
                // The member signature and even its result domain depend on
                // the container identity. Concrete checking retains the full
                // argument list, routing, defaults and formation requirements.
                D::Unknown
            }
            H::NFApplication(head, arguments) => {
                return self.describe_normal_application(
                    expr, head, arguments, locals, returns, expected, depth,
                );
            }
            H::List(elements) if matches!(expected, Some(D::Known(ty)) if self.list_element_type(ty).is_some()) =>
            {
                let Some(D::Known(expected_ty)) = expected else {
                    return Err(error(expr.span, "expected list domain"));
                };
                let Some(element_type) = self.list_element_type(expected_ty).cloned() else {
                    return Err(error(expr.span, "expected list element domain"));
                };
                for element in *elements {
                    self.describe_open_expression(
                        element,
                        locals,
                        returns,
                        Some(&D::Known(element_type.clone())),
                        depth + 1,
                    )?;
                }
                D::Known(expected_ty.clone())
            }
            H::TyPtr(inner)
            | H::TyOptional(inner)
            | H::TyTraitObject(inner)
            | H::LiftType(inner) => {
                self.describe_open_expression(
                    inner,
                    locals,
                    returns,
                    Some(&D::Known(p(P::Type))),
                    depth + 1,
                )?;
                D::Known(p(P::Type))
            }
            H::TyFnArrow(parameters, result) => {
                for component in [*parameters, *result] {
                    self.describe_open_expression(
                        component,
                        locals,
                        returns,
                        Some(&D::Known(p(P::Type))),
                        depth + 1,
                    )?;
                }
                D::Known(p(P::Type))
            }
            H::TyFn(parameters) => {
                for parameter in *parameters {
                    let TyParamKind::Positional(ty) = parameter.kind else {
                        // Preserve the ordinary checker for richer parameter
                        // protocols until their descriptions can carry binders.
                        return self.describe_known_region(expr, expected);
                    };
                    self.describe_open_expression(
                        ty,
                        locals,
                        returns,
                        Some(&D::Known(p(P::Type))),
                        depth + 1,
                    )?;
                }
                D::Known(p(P::Type))
            }
            H::InlineControl(value) if matches!(value.kind, H::For { .. }) => {
                let H::For {
                    pat,
                    guard,
                    iter,
                    body,
                    ..
                } = value.kind
                else {
                    return Err(error(expr.span, "expected inline loop expression"));
                };
                if let H::TakeTupleEnumerate(input) = iter.kind {
                    return self.describe_open_tuple_iteration(
                        value,
                        input,
                        false,
                        locals,
                        returns,
                        expected,
                        depth + 1,
                    );
                }
                if let H::TakeTupleIndexed(input) = iter.kind {
                    return self.describe_open_tuple_iteration(
                        value,
                        input,
                        true,
                        locals,
                        returns,
                        expected,
                        depth + 1,
                    );
                }
                let H::Range {
                    start: Some(start),
                    end: Some(end),
                    ..
                } = iter.kind
                else {
                    return Err(error(
                        iter.span,
                        "inline for currently requires a bounded comptime Integer range",
                    ));
                };
                for endpoint in [start, end] {
                    self.with_staged_input(true, |this| {
                        this.describe_open_expression(
                            endpoint,
                            &mut locals.clone(),
                            returns,
                            Some(&D::Known(p(P::Integer))),
                            depth + 1,
                        )
                    })?;
                }
                let saved = self.locals.clone();
                let staged = self.open_staged_locals.clone();
                let staged_members = self.open_member_locals.clone();
                let immutable = self.immutable.clone();
                let context = self.context.clone();
                self.control.loops.push(local(value.hir_id));
                let checked = (|| {
                    self.immutable.extend(pattern_binding_ids(pat));
                    let pattern =
                        self.with_staged_input(true, |this| this.pattern(pat, &p(P::Integer)))?;
                    let mut body_locals = locals.clone();
                    for (id, ty) in &self.locals {
                        body_locals.insert(*id, D::Known(ty.clone()));
                        if !saved.contains_key(id) {
                            self.open_staged_locals.insert(*id);
                        }
                    }
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let observations = nir::visit::pattern_observations(&pattern, &mut fuel);
                    self.required_steps -= (before - fuel) as u64;
                    for observation in observations.map_err(|e| error(pat.span, e))? {
                        self.with_staged_input(true, |this| {
                            this.check_open_fragment(observation, pat.span, false)
                        })?;
                    }
                    if let Some(guard) = guard {
                        self.with_staged_input(true, |this| {
                            this.describe_open_expression(
                                guard,
                                &mut body_locals.clone(),
                                returns,
                                Some(&D::Known(p(P::Bool))),
                                depth + 1,
                            )
                        })?;
                    }
                    self.describe_open_block(body, &mut body_locals, returns, None, depth + 1)
                })();
                self.control.loops.pop();
                self.locals = saved;
                self.open_staged_locals = staged;
                self.open_member_locals = staged_members;
                self.immutable = immutable;
                self.context = context;
                checked?;
                D::Known(p(P::Void))
            }
            H::InlineControl(value) => {
                let conditions: Vec<_> = match value.kind {
                    H::If(condition, _, _) => vec![condition],
                    H::When(arms) => arms.iter().filter_map(|arm| arm.cond).collect(),
                    H::Match(subject, _) => {
                        self.with_staged_input(true, |this| {
                            this.describe_open_expression(
                                subject,
                                &mut locals.clone(),
                                returns,
                                None,
                                depth + 1,
                            )
                        })?;
                        Vec::new()
                    }
                    _ => {
                        return Err(error(
                            expr.span,
                            "inline control-flow expansion is not yet implemented",
                        ));
                    }
                };
                for condition in conditions {
                    if self.is_type_assumption(condition) {
                        continue;
                    }
                    let saved_unselected = if self.open_condition_has_type_evidence(condition, 0) {
                        Some(std::mem::replace(&mut self.in_unselected_inline, true))
                    } else {
                        None
                    };
                    let checked = self.with_staged_input(true, |this| {
                        this.describe_open_expression(
                            condition,
                            &mut locals.clone(),
                            returns,
                            Some(&D::Known(p(P::Bool))),
                            depth + 1,
                        )
                    });
                    if let Some(prev) = saved_unselected {
                        self.in_unselected_inline = prev;
                    }
                    checked?;
                }
                let saved_selection = self.inline_selection.replace((value.hir_id, usize::MAX));
                let result =
                    self.describe_open_expression(value, locals, returns, expected, depth + 1);
                self.inline_selection = saved_selection;
                result?
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                if self.in_staged_input && self.required_runtime_parameter(local(*id)) {
                    return Err(error(
                        expr.span,
                        "required comptime computation cannot read a runtime parameter",
                    ));
                }
                if self.in_staged_input
                    && matches!(expr.kind, H::ResolvedLocal(..))
                    && !self.open_staged_locals.contains(&local(*id))
                    && !self.input_facts.is_staged(&local(*id))
                    && !self.signature_owner.and_then(|owner| self.package.item(owner)).is_some_and(|item|
                        matches!(&item.kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime))
                {
                    return Err(error(expr.span, "required comptime computation cannot read an unstaged local"));
                }
                locals
                    .get(&local(*id))
                    .cloned()
                    .ok_or_else(|| error(expr.span, "local binding is unavailable in this scope"))?
            }
            H::When(arms) => {
                self.describe_open_when(expr, arms, locals, returns, expected, depth + 1)?
            }
            H::Match(value, arms) => {
                self.describe_open_match(expr, value, arms, locals, returns, expected, depth + 1)?
            }
            H::Null => {
                if expected.is_none() {
                    return Err(context_error(
                        expr.span,
                        "null requires an optional target type",
                    ));
                }
                if matches!(expected, Some(D::Unknown)) {
                    D::Unknown
                } else {
                    self.describe_known_region(expr, expected)?
                }
            }
            H::Tuple(_) | H::Unit if matches!(expected, Some(D::Known(ty)) if *ty == p(P::Type)) => {
                return Err(error(
                    expr.span,
                    "a parenthesized tuple is a value, not a type; use Tuple<...> to construct a tuple type",
                ));
            }
            H::Tuple(items) => {
                let expected_shape = match expected {
                    Some(D::Known(ty)) => Some(self.structural_type(ty, expr.span)?),
                    _ => None,
                };
                let expected_items = match &expected_shape {
                    Some(Term::Tuple(items)) => Some(items),
                    _ => None,
                };
                let mut types = Vec::new();
                let mut diverges = false;
                for (index, item) in items.iter().enumerate() {
                    let context = expected_items
                        .and_then(|items| items.get(index))
                        .cloned()
                        .map(D::Known);
                    let description =
                        self.with_open_reachability(!diverges, item.span, |this| {
                            this.describe_open_expression(
                                item,
                                locals,
                                returns,
                                context.as_ref(),
                                depth + 1,
                            )
                        })?;
                    let D::Known(ty) = description else {
                        return Err(error(
                            item.span,
                            "tuple construction requires established element types",
                        ));
                    };
                    diverges |= is_never(&ty);
                    types.push(ty);
                }
                D::Known(if diverges {
                    p(P::Never)
                } else {
                    self.tuple_value_type(types, expr.span)?
                })
            }
            H::Application(callee, args) => {
                if let Some((_, arg)) = self.type_predicate_kind_call(expr)? {
                    self.describe_open_expression(
                        arg,
                        locals,
                        returns,
                        Some(&D::Known(p(P::Type))),
                        depth + 1,
                    )?;
                    D::Known(p(P::Bool))
                } else if let Some(arg) = self.type_predicate_not_sized_call(expr)? {
                    self.describe_open_expression(
                        arg,
                        locals,
                        returns,
                        Some(&D::Known(p(P::Type))),
                        depth + 1,
                    )?;
                    D::Known(p(P::Bool))
                } else if let Some((_, arg, field)) = self.type_layout_query_call(expr)? {
                    self.describe_open_expression(
                        arg,
                        locals,
                        returns,
                        Some(&D::Known(p(P::Type))),
                        depth + 1,
                    )?;
                    if let Some(field) = field {
                        self.describe_open_expression(
                            field,
                            locals,
                            returns,
                            Some(&D::Known(p(P::Symbol))),
                            depth + 1,
                        )?;
                    }
                    D::Known(p(P::Usize))
                } else {
                    self.describe_open_call(
                        expr,
                        callee,
                        args,
                        locals,
                        returns,
                        expected,
                        depth + 1,
                    )?
                }
            }
            H::ExtendedApplication(callee, args) => self.describe_open_extended_call(
                expr,
                callee,
                args,
                locals,
                returns,
                expected,
                depth + 1,
                false,
            )?,
            H::Block(block) => {
                return self.describe_open_block(block, locals, returns, expected, depth + 1);
            }
            H::UnsafeBlock(block) => {
                let saved = self.unsafe_depth;
                self.unsafe_depth = self.unsafe_depth.saturating_add(1);
                let result = self.describe_open_block(block, locals, returns, expected, depth + 1);
                self.unsafe_depth = saved;
                return result;
            }
            H::Semi(inner) => {
                let ty = self.describe_open_expression(inner, locals, returns, None, depth + 1)?;
                if matches!(ty, D::Known(ref ty) if is_never(ty)) {
                    ty
                } else {
                    D::Known(p(P::Void))
                }
            }
            H::Return(value) => {
                let result = returns.ok_or_else(|| {
                    error(expr.span, "return is unavailable in a parameter default")
                })?;
                if let Some(value) = value {
                    self.describe_open_expression(value, locals, returns, Some(result), depth + 1)?;
                } else {
                    self.require_description(&D::Known(p(P::Void)), Some(result), expr.span)?;
                }
                D::Known(p(P::Never))
            }
            H::Assumes(condition) if self.is_type_assumption(condition) => {
                let predicate = self.type_predicate(condition)?;
                let fact = crate::ptc::Formula::from_predicate(&predicate);
                self.context
                    .validate_formula(&fact, &mut crate::ptc::Budget::default())
                    .map_err(|failure| error(condition.span, failure.to_string()))?;
                self.context.assume(fact);
                self.definition_refinements.requirements.push(predicate);
                D::Known(p(P::Void))
            }
            H::Assumes(condition) | H::Asserts(condition) | H::Invariant(condition) => {
                self.with_program_predicate(|this| {
                    this.describe_open_boolean(condition, locals, returns, depth + 1)
                })?;
                D::Known(p(P::Void))
            }
            H::Decreases(metric) => {
                self.with_program_predicate(|this| {
                    this.describe_open_expression(metric, locals, returns, None, depth + 1)
                })?;
                D::Known(p(P::Void))
            }
            H::Loop(body) => {
                let identity = local(expr.hir_id);
                let saved_transfer = self
                    .open_loop_transfers
                    .insert(identity, LoopTransfers::default());
                self.control.loops.push(identity);
                let checked = self.describe_open_block(body, locals, returns, None, depth + 1);
                self.control.loops.pop();
                let transfers = self
                    .open_loop_transfers
                    .remove(&identity)
                    .unwrap_or_default();
                if let Some(previous) = saved_transfer {
                    self.open_loop_transfers.insert(identity, previous);
                }
                checked?;
                D::Known(p(if transfers.breaks { P::Void } else { P::Never }))
            }
            H::Assign(target, value)
                if (self.in_staged_input || !self.in_required_comptime)
                    && match target.kind {
                        H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                            (!self.in_staged_input && locals.contains_key(&local(id)))
                                || matches!(locals.get(&local(id)), Some(D::Unknown))
                                || (self.in_staged_input
                                    && self.open_staged_locals.contains(&local(id)))
                        }
                        _ => false,
                    } =>
            {
                let binding = match target.kind {
                    H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => local(id),
                    _ => return Err(error(target.span, "expected local assignment target")),
                };
                if self.in_staged_input && !self.open_staged_locals.contains(&binding) {
                    return Err(error(
                        target.span,
                        "required comptime assignment needs a locally established staged target",
                    ));
                }
                let target_type = locals
                    .get(&binding)
                    .cloned()
                    .ok_or_else(|| error(target.span, "assignment target is outside its scope"))?;
                if self.immutable.contains(&binding) {
                    return Err(error(
                        target.span,
                        "cannot reassign a let binding; use var for whole-value assignment",
                    ));
                }
                let value = self.describe_open_expression(
                    value,
                    locals,
                    returns,
                    Some(&target_type),
                    depth + 1,
                )?;
                if !self.in_staged_input
                    && matches!(target_type, D::Known(_))
                    && matches!(value, D::Unknown)
                {
                    return Err(error(
                        expr.span,
                        "assignment to an established type requires value-type evidence",
                    ));
                }
                if matches!(&value, D::Known(ty) if is_never(ty)) {
                    value
                } else {
                    D::Known(p(P::Void))
                }
            }
            H::Assign(target, value)
                if matches!(target.kind, H::Deref(_))
                    || (matches!(target.kind, H::Projection(..)) && {
                        let mut root = *target;
                        while let H::Projection(inner, _) = root.kind {
                            root = inner;
                        }
                        matches!(
                            root.kind,
                            H::ResolvedLocal(..) | H::ResolvedFnParam(..) | H::Deref(_)
                        )
                    }) =>
            {
                // Form the destination address without reading its contents;
                // writes must retain their own transfer/Copy contract checks.
                let target_type = if let Some(pointer) = self.dereferenced_place(target)? {
                    let Term::Pointer(ty) = pointer.ty else {
                        return Err(error(
                            target.span,
                            "dereferenced place must have pointer type",
                        ));
                    };
                    *ty
                } else {
                    self.expr(target, None)?.ty
                };
                let description = self.describe_open_expression(
                    value,
                    locals,
                    returns,
                    Some(&D::Known(target_type)),
                    depth + 1,
                )?;
                let D::Known(ty) = description else {
                    return Err(error(
                        value.span,
                        "place assignment requires value-type evidence",
                    ));
                };
                self.with_open_operand(ty, value.span, |this, placeholder| {
                    let probe = Expr {
                        hir_id: expr.hir_id,
                        span: expr.span,
                        kind: H::Assign(target, placeholder),
                    };
                    this.describe_known_region(&probe, expected)
                })?
            }
            H::AssignOp(op, target, value) if self.contains_inline_control(value)? => {
                let target_type = if let Some(pointer) = self.dereferenced_place(target)? {
                    let Term::Pointer(ty) = pointer.ty else {
                        return Err(error(
                            target.span,
                            "dereferenced place must have pointer type",
                        ));
                    };
                    *ty
                } else {
                    self.expr(target, None)?.ty
                };
                let arithmetic = match op {
                    BinOp::Add => Some(crate::arithmetic::Operator::Add),
                    BinOp::Sub => Some(crate::arithmetic::Operator::Subtract),
                    BinOp::Mul => Some(crate::arithmetic::Operator::Multiply),
                    BinOp::Div => Some(crate::arithmetic::Operator::Divide),
                    BinOp::Rem => Some(crate::arithmetic::Operator::Remainder),
                    _ => None,
                };
                let protocol = arithmetic.and_then(|op| self.protocols.arithmetic[op as usize]);
                let context =
                    (protocol.is_none() && *op != BinOp::Concat).then_some(D::Known(target_type));
                let description = self.describe_open_expression(
                    value,
                    locals,
                    returns,
                    context.as_ref(),
                    depth + 1,
                )?;
                let D::Known(ty) = description else {
                    return Err(error(
                        value.span,
                        "compound assignment requires operand-type evidence",
                    ));
                };
                self.with_open_operand(ty, value.span, |this, placeholder| {
                    let probe = Expr {
                        kind: H::AssignOp(*op, target, placeholder),
                        ..expr.clone()
                    };
                    this.describe_known_region(&probe, expected)
                })?
            }
            H::Let(binding) => {
                return self.describe_open_binding(expr, binding, locals, returns, expected, depth);
            }
            H::If(condition, yes, no) if self.open_condition_has_type_evidence(condition, 0) => {
                let yes = Expr {
                    kind: H::Block(yes),
                    ..expr.clone()
                };
                let mut arms = vec![hir::expr::CondictionArm {
                    hir_id: expr.hir_id,
                    cond: Some(condition),
                    body: &yes,
                    span: expr.span,
                }];
                if let Some(no) = no {
                    arms.push(hir::expr::CondictionArm {
                        hir_id: no.hir_id,
                        cond: None,
                        body: no,
                        span: no.span,
                    });
                }
                self.describe_open_when(expr, &arms, locals, returns, expected, depth + 1)?
            }
            H::If(condition, yes, no) => {
                let inline_if = self
                    .inline_selection
                    .is_some_and(|(id, _)| id == expr.hir_id);
                let staged = self.in_staged_input || inline_if;
                let OpenCondition {
                    diverges: condition_exits,
                    choice,
                } = self.with_staged_input(staged, |this| {
                    this.describe_open_boolean(condition, locals, returns, depth + 1)
                })?;
                self.required_steps = self
                    .required_steps
                    .checked_sub((self.open_loop_transfers.len() as u64).saturating_mul(4))
                    .ok_or_else(|| {
                        error(expr.span, "loop completion description budget exhausted")
                    })?;
                let condition_transfers = self.open_loop_transfers.clone();
                let yes = self.describe_open_block(
                    yes,
                    locals,
                    returns,
                    if no.is_some() { expected } else { None },
                    depth + 1,
                )?;
                let yes = if no.is_none() && !matches!(&yes, D::Known(ty) if is_never(ty)) {
                    D::Known(p(P::Void))
                } else {
                    yes
                };
                let yes_transfers =
                    std::mem::replace(&mut self.open_loop_transfers, condition_transfers.clone());
                let no = if let Some(no) = no {
                    self.describe_open_expression(
                        no,
                        &mut locals.clone(),
                        returns,
                        expected,
                        depth + 1,
                    )?
                } else {
                    D::Known(p(P::Void))
                };
                let no_transfers =
                    std::mem::replace(&mut self.open_loop_transfers, condition_transfers);
                if !condition_exits {
                    for (id, found) in &mut self.open_loop_transfers {
                        if choice != Some(false) {
                            found.include(yes_transfers[id]);
                        }
                        if choice != Some(true) {
                            found.include(no_transfers[id]);
                        }
                    }
                }
                let diverges = condition_exits
                    || match choice {
                        Some(true) => matches!(&yes, D::Known(ty) if is_never(ty)),
                        Some(false) => matches!(&no, D::Known(ty) if is_never(ty)),
                        None => false,
                    };
                let result = match (&yes, &no) {
                    (D::Known(ty), _) if is_never(ty) => no,
                    (_, D::Known(ty)) if is_never(ty) => yes,
                    (D::Known(a), D::Known(b)) => {
                        if inline_if && matches!(expected, None | Some(D::Unknown)) {
                            if self.require_expression(a, b, expr.span).is_ok() {
                                yes
                            } else {
                                D::Unknown
                            }
                        } else {
                            self.require_expression(a, b, expr.span)?;
                            yes
                        }
                    }
                    _ => D::Unknown,
                };
                if diverges {
                    D::Known(p(P::Never))
                } else {
                    result
                }
            }
            H::TakeIntrinsic(_) => {
                return Err(error(
                    expr.span,
                    "intrinsic view requires a direct positional call; target input protocols for other call forms are not implemented",
                ));
            }
            H::TakeAttribute(value) => {
                if self.runtime_value_context() || self.in_code_body {
                    return Err(error(
                        expr.span,
                        "attribute view requires comptime invocation",
                    ));
                }
                match self.describe_open_expression(value, locals, returns, None, depth + 1)? {
                    D::Known(ty) if is_never(&ty) => D::Known(ty),
                    _ => D::Known(p(P::Object)),
                }
            }
            H::TakeBitVec(value)
            | H::TakeTagName(value)
            | H::Deref(value)
            | H::MultiDeref(value) => {
                let description =
                    self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                let D::Known(ty) = description else {
                    return Err(error(
                        value.span,
                        match expr.kind {
                            H::TakeBitVec(_) => "bit view requires an established source type",
                            H::TakeTagName(_) => "tag_name requires an established source type",
                            H::Deref(_) => "dereference requires an established pointer type",
                            H::MultiDeref(_) => "multi_deref requires an established pointer type",
                            _ => "unary projection requires an established source type",
                        },
                    ));
                };
                if matches!(expr.kind, H::Deref(_))
                    && let Term::Pointer(pointee) = &ty
                    && !self
                        .term_substitution_inputs(pointee, expr.span)?
                        .is_empty()
                {
                    // Only the result domain is known here. Concrete read
                    // formation and Copy/origin checks belong to the instance;
                    // no ReadPointer NIR is published by this description.
                    let result = D::Known((**pointee).clone());
                    self.require_description(&result, expected, expr.span)?;
                    return Ok(result);
                }
                if matches!(expr.kind, H::MultiDeref(_)) {
                    let mut current = ty.clone();
                    let mut ptr_depth = 0usize;
                    while let Term::Pointer(pointee) = current {
                        ptr_depth += 1;
                        if ptr_depth > 128 {
                            return Err(error(expr.span, "multi_deref depth exhausted"));
                        }
                        current = (*pointee).clone();
                    }
                    if ptr_depth > 0
                        && !self
                            .term_substitution_inputs(&current, expr.span)?
                            .is_empty()
                    {
                        let result = D::Known(current);
                        self.require_description(&result, expected, expr.span)?;
                        return Ok(result);
                    }
                }
                self.with_open_operand(ty, value.span, |this, placeholder| {
                    let kind = match expr.kind {
                        H::TakeBitVec(_) => H::TakeBitVec(placeholder),
                        H::TakeTagName(_) => H::TakeTagName(placeholder),
                        H::Deref(_) => H::Deref(placeholder),
                        H::MultiDeref(_) => H::MultiDeref(placeholder),
                        _ => {
                            return Err(error(
                                expr.span,
                                "unexpected unary projection in open operand probe",
                            ));
                        }
                    };
                    let probe = Expr {
                        hir_id: expr.hir_id,
                        span: expr.span,
                        kind,
                    };
                    this.describe_known_region(&probe, expected)
                })?
            }
            H::ErrorPropagate(value) => {
                let description =
                    self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                match description {
                    D::Unknown => {
                        if returns.is_none() {
                            return Err(context_error(
                                expr.span,
                                "error propagation requires a function result context",
                            ));
                        }
                        D::Unknown
                    }
                    D::Known(ty) if is_never(&ty) => D::Known(ty),
                    D::Known(ty) if self.control.result.is_some() => {
                        self.with_open_operand(ty, value.span, |this, placeholder| {
                            let probe = Expr {
                                hir_id: expr.hir_id,
                                span: expr.span,
                                kind: H::ErrorPropagate(placeholder),
                            };
                            this.describe_known_region(&probe, expected)
                        })?
                    }
                    D::Known(Term::ErrorResult { result, .. })
                        if matches!(returns, Some(D::Unknown)) =>
                    {
                        D::Known((*result).clone())
                    }
                    D::Known(_) if returns.is_none() => {
                        return Err(context_error(
                            expr.span,
                            "error propagation requires a function result context",
                        ));
                    }
                    D::Known(_) => {
                        return Err(error(
                            expr.span,
                            "error propagation requires an error-qualified operand",
                        ));
                    }
                }
            }
            H::OptionPropagate(value) => {
                let description =
                    self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                match description {
                    D::Unknown => {
                        if returns.is_none() {
                            return Err(context_error(
                                expr.span,
                                "optional propagation requires a function result context",
                            ));
                        }
                        D::Unknown
                    }
                    D::Known(ty) if is_never(&ty) => D::Known(ty),
                    D::Known(ty) if self.control.result.is_some() => {
                        self.with_open_operand(ty, value.span, |this, placeholder| {
                            let probe = Expr {
                                hir_id: expr.hir_id,
                                span: expr.span,
                                kind: H::OptionPropagate(placeholder),
                            };
                            this.describe_known_region(&probe, expected)
                        })?
                    }
                    D::Known(Term::Optional(inner)) if matches!(returns, Some(D::Unknown)) => {
                        D::Known((*inner).clone())
                    }
                    D::Known(_) if returns.is_none() => {
                        return Err(context_error(
                            expr.span,
                            "optional propagation requires a function result context",
                        ));
                    }
                    D::Known(_) => {
                        return Err(error(
                            expr.span,
                            "optional propagation requires an optional operand",
                        ));
                    }
                }
            }
            H::ErrorNew(value) => {
                let hint = match expected {
                    Some(D::Known(Term::ErrorResult { errors, .. })) if errors.len() == 1 => {
                        Some(D::Known(errors[0].clone()))
                    }
                    _ => None,
                };
                let description = self.describe_open_expression(
                    value,
                    locals,
                    returns,
                    hint.as_ref(),
                    depth + 1,
                )?;
                match description {
                    D::Unknown => D::Unknown,
                    D::Known(ty) if is_never(&ty) => D::Known(ty),
                    D::Known(ty) => {
                        self.with_open_operand(ty, value.span, |this, placeholder| {
                            let probe = Expr {
                                hir_id: expr.hir_id,
                                span: expr.span,
                                kind: H::ErrorNew(placeholder),
                            };
                            this.describe_known_region(&probe, expected)
                        })?
                    }
                }
            }
            H::DynCast(value, target) => {
                let description =
                    self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                let D::Known(ty) = description else {
                    return Err(error(
                        value.span,
                        "dynamic trait cast requires an established source type",
                    ));
                };
                self.with_open_operand(ty, value.span, |this, placeholder| {
                    let probe = Expr {
                        hir_id: expr.hir_id,
                        span: expr.span,
                        kind: H::DynCast(placeholder, target),
                    };
                    this.describe_known_region(&probe, expected)
                })?
            }
            H::Cast(value, target) => {
                let description =
                    self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                let D::Known(ty) = description else {
                    return Err(error(
                        value.span,
                        "explicit conversion requires an established source type",
                    ));
                };
                self.with_open_operand(ty, value.span, |this, placeholder| {
                    let probe = Expr {
                        hir_id: expr.hir_id,
                        span: expr.span,
                        kind: H::Cast(placeholder, target),
                    };
                    this.describe_known_region(&probe, expected)
                })?
            }
            H::Index(receiver, index) => {
                let description =
                    self.describe_open_expression(receiver, locals, returns, None, depth + 1)?;
                let D::Known(ty) = description else {
                    return Err(error(
                        receiver.span,
                        "indexing requires an established receiver type",
                    ));
                };
                self.with_open_operand(ty, receiver.span, |this, placeholder| {
                    let probe = Expr {
                        hir_id: expr.hir_id,
                        span: expr.span,
                        kind: H::Index(placeholder, index),
                    };
                    this.describe_known_region(&probe, expected)
                })?
            }
            H::Projection(receiver, field)
                if !matches!(receiver.kind, H::ResolvedDef { .. } | H::Path(_)) =>
            {
                let mut root = *receiver;
                while let H::Projection(inner, _) | H::NFApplication(inner, _) = root.kind {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(expr.span, "staged projection description budget exhausted")
                    })?;
                    root = inner;
                }
                let staged = self.in_staged_input
                    || matches!(root.kind,
                    H::ResolvedFnParam(_, id) | H::ResolvedLocal(_, id)
                    if self.input_facts.is_staged(&local(id))
                        || self.open_staged_locals.contains(&local(id)));
                let description =
                    self.describe_open_expression(receiver, locals, returns, None, depth + 1)?;
                let D::Known(ty) = description else {
                    let type_root = matches!(root.kind,
                        H::ResolvedFnParam(_, id) | H::ResolvedLocal(_, id)
                        if matches!(locals.get(&local(id)), Some(D::Known(ty)) if *ty == p(P::Type))
                            || self.locals.get(&local(id)) == Some(&p(P::Type)));
                    if staged && (type_root || self.pending_member_origin(receiver, locals)?) {
                        // A previous staged member can itself select a Type
                        // or a value. Preserve the chain for instance checking.
                        return Ok(D::Unknown);
                    }
                    return Err(error(
                        receiver.span,
                        "field projection requires an established receiver type",
                    ));
                };
                if ty == p(P::Type) {
                    // A bound Type may already have an explicit capability.
                    // Its member contract supplies a description without
                    // selecting a concrete implementation or executing code.
                    if self.is_static_method_subject(receiver) {
                        let subject = self.type_expr(receiver)?;
                        if self
                            .context_member_interface(&subject, field.name.as_str(), expr.span)?
                            .is_some()
                        {
                            return self.describe_known_region(expr, expected);
                        }
                    }
                    // A staged Type factory supplies its container identity at
                    // invocation. Preserve an ordinary unknown description;
                    // the concrete instance must redo member/access checking.
                    if staged
                        && matches!(receiver.kind,
                            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id)
                            if !self.generator_data.get(&local(id)).is_some_and(Option::is_some))
                    {
                        return Ok(D::Unknown);
                    }
                    // Associated type/member lookup needs the receiver's
                    // identity, not merely its Type domain.
                    return self.describe_known_region(expr, expected);
                }
                self.with_staged_input(staged, |this| {
                    this.with_open_operand(ty, receiver.span, |this, placeholder| {
                        let probe = Expr {
                            hir_id: expr.hir_id,
                            span: expr.span,
                            kind: H::Projection(placeholder, *field),
                        };
                        // Check field shape in the input's stage. Concrete
                        // uses still undergo ordinary phase validation.
                        this.describe_known_region(&probe, expected)
                    })
                })?
            }
            // Known subtrees retain the normal checker and its full protocols.
            // Unknown locals are deliberately absent from its Term map: they
            // cannot acquire projection, arithmetic, layout or trait evidence.
            _ => self.describe_known_region(expr, expected)?,
        };
        self.require_description(&result, expected, expr.span)?;
        Ok(result)
    }

    /// Check an operation against an abstract, already checked operand.
    /// Only the suspended source is instantiated; this binding never escapes.
    fn with_open_operand(
        &mut self,
        ty: Term,
        span: Span,
        check: impl FnOnce(&mut Self, &Expr<'_>) -> Result<D, Error>,
    ) -> Result<D, Error> {
        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(span, "ordinary operand identity space exhausted"))?;
        let id = hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index));
        self.locals.insert(binding, ty);
        if self.in_staged_input {
            self.open_staged_locals.insert(binding);
        }
        let placeholder = Expr {
            hir_id: id,
            span,
            kind: H::ResolvedLocal(hir::Symbol::intern("$ordinary-operand"), id),
        };
        let result = check(self, &placeholder);
        self.locals.remove(&binding);
        self.open_staged_locals.remove(&binding);
        result
    }

    /// Validate an inline input where the ordinary call checker has supplied
    /// its context. The result is a definition-only witness; actual instances
    /// recheck the suspended source and select their own branches.
    pub(super) fn describe_contextual_inline_input(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(self.locals.len() as u64)
            .ok_or_else(|| error(expr.span, "ordinary input description budget exhausted"))?;
        let saved_spread = std::mem::replace(&mut self.describing_open_spread, false);
        let expected_description = expected.cloned().map(D::Known);
        let returns = self.control.result.clone().map(D::Known).or_else(|| {
            self.control
                .return_observations
                .as_ref()
                .map(|_| D::Unknown)
        });
        let mut locals = self
            .locals
            .iter()
            .map(|(id, ty)| (*id, D::Known(ty.clone())))
            .collect();
        // A callee's result determines whether its producer is consumed at
        // comptime. First check all ordinary branches without invoking inline
        // selectors; no branch value or environment is fabricated here.
        let callee_delivery = matches!(expr.kind, H::InlineControl(inner)
            if self.comptime_callee_result == Some(inner.hir_id));
        let witness = if callee_delivery {
            let saved_unselected = std::mem::replace(&mut self.in_unselected_inline, true);
            let saved_selection = self.inline_selection.replace((expr.hir_id, usize::MAX));
            let result = self.expr_context(expr, expected, partial);
            self.inline_selection = saved_selection;
            self.in_unselected_inline = saved_unselected;
            match result {
                Ok(value) => Some(value),
                Err(failure) => {
                    self.describing_open_spread = saved_spread;
                    return Err(failure);
                }
            }
        } else {
            None
        };
        let staged = self.in_staged_input
            || witness
                .as_ref()
                .is_some_and(|value| value.ty.is_comptime_callable() || value.ty == p(P::Type));
        let checked = self.with_staged_input(staged, |this| {
            this.with_open_reachability(false, expr.span, |this| {
                this.describe_open_expression(
                    expr,
                    &mut locals,
                    returns.as_ref(),
                    expected_description.as_ref(),
                    0,
                )
            })
        });
        let result = checked.and_then(|_| {
            if let Some(witness) = witness {
                return Ok(witness);
            }
            let saved_unselected = std::mem::replace(&mut self.in_unselected_inline, true);
            let saved_selection = self.inline_selection.replace((expr.hir_id, usize::MAX));
            let result = self.expr_context(expr, expected, partial);
            self.inline_selection = saved_selection;
            self.in_unselected_inline = saved_unselected;
            result
        });
        self.describing_open_spread = saved_spread;
        result
    }

    /// Describe both continuations without executing a staged selection. These
    /// abstract inputs are behavior witnesses only; invocation rechecks the
    /// original binding and materializes only the selected continuation.
    pub(super) fn ordinary_conditional_constant_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if let Some(expected) = expected {
            self.require(&p(P::Void), expected, decl.span)?;
        }
        // Failure has the surrounding runtime scope, not candidate bindings.
        let otherwise = decl
            .otherwise
            .map(|body| self.check_failure_branch(body))
            .transpose()?;
        if otherwise.as_ref().is_some_and(|body| !is_never(&body.ty)) {
            return Err(error(decl.span, "const else branch must diverge"));
        }
        let init = decl
            .init
            .ok_or_else(|| error(decl.span, "local const requires an initializer"))?;
        let annotation = decl.ty.map(|ty| self.type_expr(ty)).transpose()?;
        let value = self.with_staged_input(true, |this| {
            let value = this.expr(init, annotation.as_ref())?;
            if this.checking_open_behavior {
                this.check_open_fragment(&value, init.span, false)?;
            } else {
                this.check_inline_input_availability(&value, &BTreeSet::new(), init.span)?;
            }
            Ok(value)
        })?;
        self.immutable.extend(pattern_binding_ids(&decl.pattern));
        let observed = self.known_pattern_type_value(&value, init.span)?;
        let pattern = self.with_staged_input(true, |this| {
            this.pattern_observed(&decl.pattern, &value.ty, observed.as_ref())
        })?;
        if otherwise.is_none()
            && (decl.guard.is_some() || !self.pattern_exhaustive(&pattern, &value.ty))
        {
            return Err(error(
                decl.span,
                "refutable const pattern requires a diverging else branch",
            ));
        }
        let known = self.known_pattern_types(&pattern, &value, decl.span)?;
        for binding in pattern_binding_ids(&decl.pattern) {
            let ty = self
                .locals
                .get(&binding)
                .cloned()
                .ok_or_else(|| error(decl.span, "constant pattern output has no type"))?;
            self.open_staged_locals.insert(binding);
            if self.checking_open_behavior {
                self.ordinary_behavior_inputs.insert(binding, ty);
            }
        }
        // Preserve candidate-local aliases introduced by equality and view
        // patterns; flattening observations loses their staged binding scope.
        self.check_inline_pattern_availability(&pattern, decl.span)?;
        if let Some(guard) = decl.guard {
            self.with_staged_input(true, |this| {
                let guard_value = this.expr(guard, Some(&p(P::Bool)))?;
                if this.checking_open_behavior {
                    this.check_open_fragment(&guard_value, guard.span, false)
                } else {
                    this.check_inline_input_availability(&guard_value, &BTreeSet::new(), guard.span)
                }
            })?;
        }
        // Only structural Type evidence survives an arbitrary guard. Values
        // (including mutable closure environments) are not frozen before it.
        for (binding, value) in known {
            self.generator_data.insert(binding, Some(value));
        }
        let unit = n::Expr {
            source: source(decl.span),
            ty: p(P::Void),
            kind: K::Unit,
        };
        let Some(otherwise) = otherwise else {
            return Ok(unit);
        };
        if !self.checking_open_behavior {
            return Ok(unit);
        }
        let selector = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(decl.span, "constant selection identity space exhausted"))?;
        self.ordinary_behavior_inputs.insert(selector, p(P::Bool));
        if let Some(frame) = self.captures.last_mut() {
            frame.const_regions.record(decl.hir_id);
        }
        Ok(n::Expr {
            source: source(decl.span),
            ty: p(P::Void),
            kind: K::If {
                condition: Box::new(n::Expr {
                    source: source(decl.span),
                    ty: p(P::Bool),
                    kind: K::Local(selector),
                }),
                then_branch: Box::new(unit),
                else_branch: Some(Box::new(otherwise)),
            },
        })
    }

    /// Retain checked constant computations in a definition-only behavior view.
    /// Concrete invocation still executes and materializes the original const.
    pub(super) fn ordinary_constant_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if let Some(expected) = expected {
            self.require(&p(P::Void), expected, decl.span)?;
        }
        let init = decl
            .init
            .ok_or_else(|| error(decl.span, "local const requires an initializer"))?;
        let annotation = decl.ty.map(|ty| self.type_expr(ty)).transpose()?;
        let value = self.with_staged_input(true, |this| {
            let value = this.expr(init, annotation.as_ref())?;
            if this.checking_open_behavior {
                this.check_open_fragment(&value, init.span, false)?;
            } else {
                this.check_inline_input_availability(&value, &BTreeSet::new(), init.span)?;
            }
            Ok(value)
        })?;
        self.immutable.extend(pattern_binding_ids(&decl.pattern));
        let pattern =
            self.with_staged_input(true, |this| this.pattern(&decl.pattern, &value.ty))?;
        if !self.pattern_exhaustive(&pattern, &value.ty) {
            return Err(error(
                decl.span,
                "refutable const pattern requires a diverging else branch",
            ));
        }
        let mut known = self.known_pattern_types(&pattern, &value, decl.span)?;
        for binding in pattern_binding_ids(&decl.pattern) {
            let ty = self
                .locals
                .get(&binding)
                .cloned()
                .ok_or_else(|| error(decl.span, "constant pattern output has no type"))?;
            let expression = if let Some(known) = known.remove(&binding) {
                known
            } else if matches!(&pattern, n::Pattern::Binding { binding: id, inner }
                if *id == binding && matches!(**inner, n::Pattern::Wild))
            {
                constants::charge_copy(&value, &mut self.required_steps, decl.span)?;
                value.clone()
            } else {
                constants::charge_copy(&value, &mut self.required_steps, decl.span)?;
                nir::copy_budget::charge_pattern_rewrite(&pattern, &mut self.required_steps)
                    .map_err(|_| error(decl.span, "constant pattern copy budget exhausted"))?;
                n::Expr {
                    source: source(decl.span),
                    ty: ty.clone(),
                    kind: K::Block {
                        statements: vec![n::Expr {
                            source: source(decl.span),
                            ty: p(P::Void),
                            kind: K::LetPattern {
                                pattern: pattern.clone(),
                                value: Box::new(value.clone()),
                                guard: None,
                                otherwise: None,
                            },
                        }],
                        result: Some(Box::new(n::Expr {
                            source: source(decl.span),
                            ty,
                            kind: K::Local(binding),
                        })),
                    },
                }
            };
            self.with_staged_input(true, |this| {
                if this.checking_open_behavior {
                    this.check_open_fragment(&expression, decl.span, false)
                } else {
                    this.check_inline_input_availability(&expression, &BTreeSet::new(), decl.span)
                }
            })?;
            if self.checking_open_behavior {
                self.retain_behavior_value(binding, &expression, decl.span)?;
            }
            self.generator_data.insert(binding, Some(expression));
            self.open_staged_locals.insert(binding);
        }
        Ok(n::Expr {
            source: source(decl.span),
            ty: p(P::Void),
            kind: K::Unit,
        })
    }

    /// Staged selectors contribute control uncertainty, not runtime effects.
    /// Ordinary description has already checked their stage and operations.
    pub(super) fn ordinary_inline_branches(
        &mut self,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let sources: Vec<_> = match value.kind {
            H::If(condition, _, _) => vec![condition],
            H::When(arms) => arms.iter().filter_map(|arm| arm.cond).collect(),
            H::Match(subject, _) => vec![subject],
            _ => {
                return Err(error(
                    value.span,
                    "expected conditional or match expression in inline branch probe",
                ));
            }
        };
        let mut bindings = Vec::new();
        let checked = (|| {
            let mut conditions = Vec::new();
            for condition in sources {
                if !matches!(value.kind, H::Match(..))
                    && self.open_condition_has_type_evidence(condition, 0)
                {
                    conditions.push(condition.clone());
                    continue;
                }
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index: self.next_temporary_binding,
                };
                self.next_temporary_binding =
                    self.next_temporary_binding.checked_add(1).ok_or_else(|| {
                        error(
                            condition.span,
                            "ordinary condition identity space exhausted",
                        )
                    })?;
                bindings.push(binding);
                let ty = if matches!(value.kind, H::Match(..)) {
                    self.with_staged_input(true, |this| this.expr(condition, None))?
                        .ty
                } else {
                    p(P::Bool)
                };
                self.locals.insert(binding, ty.clone());
                self.ordinary_behavior_inputs.insert(binding, ty);
                let id =
                    hir::HirId::new(hir::OwnerId::INVALID, hir::ItemLocalId::new(binding.index));
                conditions.push(Expr {
                    hir_id: id,
                    span: condition.span,
                    kind: H::ResolvedLocal(hir::Symbol::intern("$ordinary-condition"), id),
                });
            }
            match value.kind {
                H::Match(_, arms) => {
                    let probe = Expr {
                        kind: H::Match(&conditions[0], arms),
                        ..value.clone()
                    };
                    let saved = self.inline_selection.replace((value.hir_id, usize::MAX));
                    let checked = self.expr(&probe, expected);
                    self.inline_selection = saved;
                    checked
                }
                H::If(_, yes, no) => {
                    let probe = Expr {
                        kind: H::If(&conditions[0], yes, no),
                        ..value.clone()
                    };
                    let saved = self.inline_selection.replace((value.hir_id, usize::MAX));
                    let checked = self.expr(&probe, expected);
                    self.inline_selection = saved;
                    checked
                }
                H::When(arms) => {
                    let mut conditions = conditions.iter();
                    let arms: Vec<_> = arms
                        .iter()
                        .map(|arm| {
                            let cond = arm
                                .cond
                                .map(|_| {
                                    conditions.next().ok_or_else(|| {
                                        error(value.span, "missing elaborated when arm condition")
                                    })
                                })
                                .transpose()?;
                            Ok(hir::expr::CondictionArm { cond, ..*arm })
                        })
                        .collect::<Result<Vec<_>, Error>>()?;
                    let probe = Expr {
                        kind: H::When(&arms),
                        ..value.clone()
                    };
                    let saved = self.inline_selection.replace((value.hir_id, usize::MAX));
                    let checked = self.expr(&probe, expected);
                    self.inline_selection = saved;
                    checked
                }
                _ => Err(error(
                    value.span,
                    "expected conditional or match expression in inline branch probe",
                )),
            }
        })();
        for binding in bindings {
            self.locals.remove(&binding);
            if checked.is_err() {
                self.ordinary_behavior_inputs.remove(&binding);
            }
        }
        checked
    }

    fn open_condition_has_type_evidence(&self, condition: &Expr<'_>, depth: usize) -> bool {
        if depth >= 128 {
            return false;
        }
        match &condition.kind {
            H::Binary(BinOp::And | BinOp::Or, left, right) => {
                self.open_condition_has_type_evidence(left, depth + 1)
                    || self.open_condition_has_type_evidence(right, depth + 1)
            }
            H::Unary(UnOp::Not, value) => self.open_condition_has_type_evidence(value, depth + 1),
            _ => self.extracted_refinement(condition),
        }
    }

    fn describe_open_boolean(
        &mut self,
        condition: &Expr<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        depth: usize,
    ) -> Result<OpenCondition, Error> {
        let expected = p(P::Bool);
        let D::Known(ty) = self.describe_open_expression(
            condition,
            locals,
            returns,
            Some(&D::Known(expected.clone())),
            depth,
        )?
        else {
            return Err(error(
                condition.span,
                "an unknown domain supplies no evidence for a boolean condition",
            ));
        };
        self.require_expression(&ty, &expected, condition.span)?;
        Ok(OpenCondition {
            diverges: is_never(&ty),
            choice: control::source_boolean(condition, &mut self.required_steps, 0),
        })
    }

    fn describe_open_when(
        &mut self,
        expr: &Expr<'_>,
        arms: &[hir::expr::CondictionArm<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let fallback = arms.iter().position(|arm| arm.cond.is_none());
        if fallback.is_some_and(|index| index + 1 != arms.len()) {
            return Err(error(expr.span, "when fallback must be the final arm"));
        }
        let saved_context = self.context.clone();
        let saved_locals = self.locals.clone();
        let saved_req_len = self.definition_refinements.requirements.len();
        let outcome = (|| {
            let mut branch_type = if fallback.is_some() {
                expected.cloned()
            } else {
                None
            };
            let mut branches = Vec::new();
            let mut reachable = true;
            let inline_when = self
                .inline_selection
                .is_some_and(|(id, _)| id == expr.hir_id);
            for arm in arms {
                self.locals = saved_locals.clone();
                self.required_steps = self
                    .required_steps
                    .checked_sub((self.open_loop_transfers.len() as u64).saturating_mul(2))
                    .ok_or_else(|| {
                        error(arm.span, "loop completion description budget exhausted")
                    })?;
                let transfers_before = self.open_loop_transfers.clone();
                let mut arm_predicate: Option<(n::TypePredicate, crate::ptc::Formula)> = None;
                let mut condition_evidence = None;
                let mut staged_evidence = (None, None);
                let condition = match arm.cond {
                    Some(hir_cond) => {
                        // Only the selector inherits the inline stage. Its
                        // runtime body must still accept runtime inputs.
                        let staged = self.in_staged_input || inline_when;
                        if staged && self.is_type_assumption(hir_cond) {
                            let predicate = self.type_predicate(hir_cond)?;
                            let fact = crate::ptc::Formula::from_predicate(&predicate);
                            self.context
                                .validate_formula(&fact, &mut crate::ptc::Budget::default())
                                .map_err(|e| error(hir_cond.span, e.to_string()))?;
                            arm_predicate = Some((predicate, fact));
                            Some(OpenCondition {
                                diverges: false,
                                choice: None,
                            })
                        } else if !self.open_condition_has_type_evidence(hir_cond, 0) {
                            Some(self.with_staged_input(staged, |this| {
                                this.describe_open_boolean(hir_cond, locals, returns, depth + 1)
                            })?)
                        } else if staged {
                            let description = self.with_staged_input(true, |this| {
                                this.describe_open_boolean(hir_cond, locals, returns, depth + 1)
                            })?;
                            staged_evidence = (
                                self.staged_condition_fact(hir_cond, true, 0)?,
                                self.staged_condition_fact(hir_cond, false, 0)?,
                            );
                            Some(description)
                        } else {
                            let checked = self.expr(hir_cond, Some(&p(P::Bool)))?;
                            self.check_open_fragment(&checked, hir_cond.span, false)?;
                            let description = OpenCondition {
                                diverges: is_never(&checked.ty),
                                choice: control::boolean(&checked),
                            };
                            condition_evidence = Some(checked);
                            Some(description)
                        }
                    }
                    None => None,
                };
                let condition_transfers = self.open_loop_transfers.clone();
                let mut branch_locals = locals.clone();
                let saved_arm_context = self.context.clone();
                let saved_arm_req_len = self.definition_refinements.requirements.len();
                let body_result = (|| -> Result<D, Error> {
                    if let Some((predicate, fact)) = &arm_predicate {
                        self.context.assume(fact.clone());
                        self.definition_refinements
                            .requirements
                            .push(predicate.clone());
                    } else if let Some(fact) = staged_evidence.0.take() {
                        self.context.assume(fact);
                    } else if let Some(checked_cond) = &condition_evidence {
                        self.assume_source_type_condition(
                            arm.cond,
                            checked_cond,
                            true,
                            arm.body.span,
                        )?;
                    }
                    self.describe_open_expression(
                        arm.body,
                        &mut branch_locals,
                        returns,
                        branch_type.as_ref(),
                        depth + 1,
                    )
                })();
                self.context = saved_arm_context;
                self.definition_refinements
                    .requirements
                    .truncate(saved_arm_req_len);
                let body = body_result?;
                if !reachable {
                    self.open_loop_transfers = transfers_before;
                } else {
                    match condition.as_ref() {
                        Some(condition) if condition.diverges => {
                            self.open_loop_transfers = condition_transfers;
                            reachable = false;
                        }
                        Some(condition) if condition.choice == Some(false) => {
                            self.open_loop_transfers = condition_transfers;
                        }
                        None => reachable = false,
                        Some(condition) if condition.choice == Some(true) => reachable = false,
                        _ => {}
                    }
                }
                if branch_type.is_none() && !matches!(&body, D::Known(ty) if is_never(ty)) {
                    branch_type = Some(body.clone());
                }
                if let Some((predicate, fact)) = arm_predicate {
                    self.context.assume(fact.negate());
                    self.definition_refinements
                        .requirements
                        .push(n::TypePredicate::Not(Box::new(predicate)));
                } else if let Some(fact) = staged_evidence.1 {
                    self.context.assume(fact);
                } else if let Some(checked_cond) = &condition_evidence {
                    self.assume_source_type_condition(
                        arm.cond,
                        checked_cond,
                        false,
                        arm.body.span,
                    )?;
                }
                let body = if fallback.is_none() && !matches!(&body, D::Known(ty) if is_never(ty)) {
                    D::Known(p(P::Void))
                } else {
                    body
                };
                branches.push((condition, body));
            }
            let mut result = D::Known(p(P::Void));
            for (condition, mut body) in branches.into_iter().rev() {
                if fallback.is_some() && matches!(&body, D::Known(ty) if !is_never(ty)) {
                    if let Some(D::Known(ty)) = &branch_type {
                        body = D::Known(ty.clone());
                    }
                }
                result = match condition {
                    None => body,
                    Some(condition) if condition.diverges => D::Known(p(P::Never)),
                    Some(condition) => {
                        let diverges = match (&body, &result) {
                            (D::Known(yes), D::Known(no)) => match condition.choice {
                                Some(true) => is_never(yes),
                                Some(false) => is_never(no),
                                None => is_never(yes) && is_never(no),
                            },
                            _ => false,
                        };
                        let joined = match (&body, &result) {
                            (D::Known(ty), _) if is_never(ty) => result,
                            (_, D::Known(ty)) if is_never(ty) => body,
                            (D::Known(a), D::Known(b)) => {
                                if inline_when && matches!(expected, None | Some(D::Unknown)) {
                                    if self.require_expression(a, b, expr.span).is_ok() {
                                        body
                                    } else {
                                        D::Unknown
                                    }
                                } else {
                                    self.require_expression(a, b, expr.span)?;
                                    body
                                }
                            }
                            _ => D::Unknown,
                        };
                        if diverges {
                            D::Known(p(P::Never))
                        } else {
                            joined
                        }
                    }
                };
            }
            Ok(result)
        })();
        self.context = saved_context;
        self.locals = saved_locals;
        self.definition_refinements
            .requirements
            .truncate(saved_req_len);
        outcome
    }

    fn describe_open_match(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        if self.scan_body_requirement(value, |_, node| {
            Ok(matches!(node.kind, H::InlineControl(_))
                || matches!(node.kind, H::Let(binding) if binding.comptime))
        })? {
            let staged = self.in_staged_input
                || self
                    .inline_selection
                    .is_some_and(|(id, _)| id == expr.hir_id);
            let description = self.with_staged_input(staged, |this| {
                this.describe_open_expression(value, locals, returns, None, depth + 1)
            })?;
            let D::Known(ty) = description else {
                return Err(error(
                    value.span,
                    "matching an unknown domain requires definition-side pattern evidence",
                ));
            };
            if ty != p(P::Type) {
                return self.with_open_operand(ty, value.span, |this, placeholder| {
                    if staged {
                        // The subject passed staged checking above. Only this
                        // abstract input inherits that stage; arm bodies do not.
                        this.open_staged_locals.insert(local(placeholder.hir_id));
                    }
                    this.describe_open_match_input(
                        expr,
                        placeholder,
                        arms,
                        locals,
                        returns,
                        expected,
                        depth,
                    )
                });
            }
        }
        self.describe_open_match_input(expr, value, arms, locals, returns, expected, depth)
    }

    fn describe_open_match_input(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let staged = self.in_staged_input
            || self
                .inline_selection
                .is_some_and(|(id, _)| id == expr.hir_id);
        let value = self
            .with_staged_input(staged, |this| this.expr(value, None))
            .map_err(|mut failure| {
                if failure.message == "local binding is unavailable in this scope" {
                    failure.message =
                        "matching an unknown domain requires definition-side pattern evidence"
                            .into();
                }
                failure
            })?;
        // A pattern binding inherits the selector's stage only after the
        // selector itself passes phase validation. Arm guards are validated
        // with the complete pattern probe below.
        if staged {
            self.with_staged_input(true, |this| {
                this.check_open_fragment(&value, expr.span, false)
            })?;
        }
        let saved = self.locals.clone();
        let saved_staged = self.open_staged_locals.clone();
        let saved_staged_members = self.open_member_locals.clone();
        let outcome = (|| {
            let mut result = D::Known(p(P::Never));
            let mut patterns = Vec::new();
            self.required_steps = self
                .required_steps
                .checked_sub(self.open_loop_transfers.len() as u64)
                .ok_or_else(|| error(expr.span, "match completion description budget exhausted"))?;
            let incoming_transfers = self.open_loop_transfers.clone();
            let mut branch_transfers = Vec::new();
            let mut branch_never = Vec::new();
            for arm in arms {
                self.required_steps = self
                    .required_steps
                    .checked_sub(incoming_transfers.len() as u64)
                    .ok_or_else(|| {
                        error(arm.span, "match completion description budget exhausted")
                    })?;
                self.locals = saved.clone();
                self.open_staged_locals = saved_staged.clone();
                self.open_member_locals = saved_staged_members.clone();
                let pattern =
                    self.with_staged_input(staged, |this| this.match_arm_pattern(arm, &value))?;
                let mut branch_locals = locals.clone();
                for (binding, ty) in &self.locals {
                    branch_locals.insert(*binding, D::Known(ty.clone()));
                    if staged && !saved.contains_key(binding) {
                        self.open_staged_locals.insert(*binding);
                    }
                }
                let context = expected.or_else(|| match &result {
                    D::Known(ty) if is_never(ty) => None,
                    _ => Some(&result),
                });
                let branch =
                    self.under_matched_pattern_type_facts(&pattern, &value, arm.span, |this| {
                        this.describe_open_expression(
                            arm.body,
                            &mut branch_locals,
                            returns,
                            context,
                            depth + 1,
                        )
                    })?;
                branch_never.push(matches!(&branch, D::Known(ty) if is_never(ty)));
                branch_transfers.push(std::mem::replace(
                    &mut self.open_loop_transfers,
                    incoming_transfers.clone(),
                ));
                result = match (&result, &branch) {
                    (D::Known(ty), _) if is_never(ty) => branch,
                    (_, D::Known(ty)) if is_never(ty) => result,
                    (D::Known(a), D::Known(b)) => {
                        self.require_expression(b, a, arm.span)?;
                        result
                    }
                    _ => D::Unknown,
                };
                patterns.push(pattern);
            }
            let coverage =
                self.patterns_exhaustive(&patterns.iter().collect::<Vec<_>>(), &value.ty);
            if coverage != crate::coverage::Coverage::Exhaustive {
                return Err(error(
                    expr.span,
                    if coverage == crate::coverage::Coverage::Unknown {
                        "match coverage budget exhausted; add an unguarded fallback pattern"
                    } else {
                        "non-exhaustive match; add an unguarded fallback pattern"
                    },
                ));
            }
            self.open_loop_transfers = incoming_transfers;
            if !is_never(&value.ty) {
                let possible =
                    control::possible_match_arms(&value, patterns.iter(), &mut self.required_steps);
                let all_diverge = possible.iter().all(|&(index, _)| branch_never[index]);
                for (index, _) in possible {
                    let transfers = std::mem::take(&mut branch_transfers[index]);
                    self.required_steps = self
                        .required_steps
                        .checked_sub(transfers.len() as u64)
                        .ok_or_else(|| {
                        error(expr.span, "match completion description budget exhausted")
                    })?;
                    for (id, found) in transfers {
                        if let Some(current) = self.open_loop_transfers.get_mut(&id) {
                            current.include(found);
                        }
                    }
                }
                if all_diverge {
                    result = D::Known(p(P::Never));
                }
            }
            // Validate the typed selector/pattern program separately from the
            // described bodies. Unit arms discard matches; no Type value is manufactured.
            let probe = n::Expr {
                source: source(expr.span),
                ty: p(P::Void),
                kind: K::Match {
                    value: Box::new(value.clone()),
                    arms: patterns
                        .into_iter()
                        .map(|pattern| n::MatchArm {
                            pattern,
                            guard: None,
                            body: n::Expr {
                                source: source(expr.span),
                                ty: p(P::Void),
                                kind: K::Unit,
                            },
                        })
                        .collect(),
                },
            };
            self.open_staged_locals = saved_staged.clone();
            self.open_member_locals = saved_staged_members.clone();
            self.with_staged_input(staged, |this| {
                this.check_open_fragment(&probe, expr.span, false)
            })?;
            Ok(if is_never(&value.ty) {
                D::Known(p(P::Never))
            } else {
                result
            })
        })();
        self.locals = saved;
        self.open_staged_locals = saved_staged;
        self.open_member_locals = saved_staged_members;
        outcome
    }

    fn describe_open_tuple_iteration(
        &mut self,
        expr: &Expr<'_>,
        input: &Expr<'_>,
        indexed: bool,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let H::For {
            pat, guard, body, ..
        } = expr.kind
        else {
            return Err(error(expr.span, "expected tuple iteration loop"));
        };
        let indexed_pats = if indexed {
            Some(inline_loops::tuple_indexed_parts(pat)?)
        } else {
            None
        };
        let computed = self.computed_tuple_input(input)?;
        let mut empty_tuple = false;
        let (elements, added_rigid) = if computed {
            let binder = self.fresh_type_binder(expr.hir_id.owner, expr.span)?;
            self.definition_refinements
                .tuple_elements
                .push(binder.declaration);
            (
                vec![Term::Rigid {
                    declaration: binder.declaration,
                    sort: Sort::Type,
                }],
                true,
            )
        } else {
            let (checked, elements) = self.tuple_enumeration_input(input)?;
            self.check_open_fragment(&checked, expr.span, false)?;
            if elements.is_empty() {
                empty_tuple = true;
                let binder = self.fresh_type_binder(expr.hir_id.owner, expr.span)?;
                self.definition_refinements
                    .tuple_elements
                    .push(binder.declaration);
                (
                    vec![Term::Rigid {
                        declaration: binder.declaration,
                        sort: Sort::Type,
                    }],
                    true,
                )
            } else {
                (elements, false)
            }
        };
        let saved = self.locals.clone();
        let staged = self.open_staged_locals.clone();
        let staged_members = self.open_member_locals.clone();
        let immutable = self.immutable.clone();
        let context = self.context.clone();
        let identity = local(expr.hir_id);
        let saved_transfer = self
            .open_loop_transfers
            .insert(identity, LoopTransfers::default());
        self.control.loops.push(identity);
        let checked = (|| {
            let mut bodies_diverge = true;
            let mut patterns_irrefutable = true;
            for ty in elements {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(expr.span, "tuple iteration description budget exhausted")
                })?;
                self.locals = saved.clone();
                self.open_staged_locals = staged.clone();
                self.open_member_locals = staged_members.clone();
                self.immutable = immutable.clone();
                self.context = context.clone();
                let elem_pat = if let Some((index_pat, elem_pat)) = indexed_pats {
                    if let Some(index_pat) = index_pat {
                        let before_index = self.locals.clone();
                        self.immutable.extend(pattern_binding_ids(index_pat));
                        let index_pattern = self.with_staged_input(true, |this| {
                            this.pattern(index_pat, &p(P::Integer))
                        })?;
                        patterns_irrefutable &=
                            self.pattern_exhaustive(&index_pattern, &p(P::Integer));
                        for id in self.locals.keys() {
                            if !before_index.contains_key(id) {
                                self.open_staged_locals.insert(*id);
                            }
                        }
                        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                        let before = fuel;
                        let observations =
                            nir::visit::pattern_observations(&index_pattern, &mut fuel);
                        self.required_steps -= (before - fuel) as u64;
                        for observation in observations.map_err(|e| error(index_pat.span, e))? {
                            self.with_staged_input(true, |this| {
                                this.check_open_fragment(observation, index_pat.span, false)
                            })?;
                        }
                    }
                    elem_pat
                } else {
                    Some(pat)
                };
                if let Some(elem_pat) = elem_pat {
                    self.immutable.extend(pattern_binding_ids(elem_pat));
                    let element = self.pattern(elem_pat, &ty)?;
                    patterns_irrefutable &= self.pattern_exhaustive(&element, &ty);
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let observations = nir::visit::pattern_observations(&element, &mut fuel);
                    self.required_steps -= (before - fuel) as u64;
                    for observation in observations.map_err(|e| error(elem_pat.span, e))? {
                        self.check_open_fragment(observation, elem_pat.span, false)?;
                    }
                }
                let mut branch_locals = locals.clone();
                branch_locals.extend(
                    self.locals
                        .iter()
                        .map(|(id, ty)| (*id, D::Known(ty.clone()))),
                );
                if let Some(guard) = guard {
                    self.with_staged_input(true, |this| {
                        this.describe_open_expression(
                            guard,
                            &mut branch_locals,
                            returns,
                            Some(&D::Known(p(P::Bool))),
                            depth + 1,
                        )
                    })?;
                }
                let body_type =
                    self.describe_open_block(body, &mut branch_locals, returns, None, depth + 1)?;
                bodies_diverge &= matches!(body_type, D::Known(ref ty) if is_never(ty));
            }
            // A nonempty, unfiltered iteration cannot fall through if every
            // element body diverges and none can break/continue this loop.
            // Transfers to enclosing loops remain visible to their own probe.
            let result = D::Known(p(
                if !computed
                    && !empty_tuple
                    && patterns_irrefutable
                    && guard.is_none()
                    && bodies_diverge
                    && !self.open_loop_transfers[&identity].any()
                {
                    P::Never
                } else {
                    P::Void
                },
            ));
            self.require_description(&result, expected, expr.span)?;
            Ok(result)
        })();
        if added_rigid {
            self.definition_refinements.tuple_elements.pop();
            if computed && checked.is_ok() {
                self.definition_refinements
                    .regions
                    .tuple_loops
                    .insert(local(expr.hir_id));
            }
        }
        self.control.loops.pop();
        self.open_loop_transfers.remove(&identity);
        if let Some(previous) = saved_transfer {
            self.open_loop_transfers.insert(identity, previous);
        }
        self.locals = saved;
        self.open_staged_locals = staged;
        self.open_member_locals = staged_members;
        self.immutable = immutable;
        self.context = context;
        checked
    }

    fn describe_staged_binding_region(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&D>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let H::Let(binding) = expr.kind else {
            return Err(error(expr.span, "expected staged let binding"));
        };
        if binding.guard.is_some() && binding.otherwise.is_none() {
            return Err(error(
                expr.span,
                "refutable let pattern requires a diverging else branch",
            ));
        }
        // A pending return domain has no ordinary Term yet. Describe the
        // continuation compositionally, before installing candidate bindings.
        if let Some(otherwise) = binding.otherwise.filter(|_| self.control.result.is_none()) {
            let description =
                self.describe_open_expression(otherwise, locals, returns, None, depth + 1)?;
            self.require_description(&description, Some(&D::Known(p(P::Never))), otherwise.span)?;
            self.with_open_operand(p(P::Never), otherwise.span, |this, placeholder| {
                this.describe_staged_binding_probe(
                    expr,
                    expected,
                    Some(placeholder),
                    locals,
                    returns,
                    depth,
                )
            })
        } else {
            self.describe_staged_binding_probe(
                expr,
                expected,
                binding.otherwise,
                locals,
                returns,
                depth,
            )
        }
    }

    fn describe_staged_binding_probe(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&D>,
        otherwise: Option<&Expr<'_>>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let H::Let(binding) = expr.kind else {
            return Err(error(expr.span, "expected staged let binding"));
        };
        let probe_binding = hir::decl::LetDecl {
            // This is a definition-side witness, not a const invocation.
            // Actual matching happens after the enclosing inputs are supplied.
            comptime: false,
            guard: None,
            otherwise,
            ..binding.clone()
        };
        let probe = Expr {
            kind: H::Let(&probe_binding),
            ..expr.clone()
        };
        let expected = match expected {
            Some(D::Known(ty)) => Some(ty),
            _ => None,
        };
        // Pattern protocols and failure divergence use the ordinary checker.
        // The guard is described after installing its candidate environment;
        // it may call a function whose domain depends on a staged binding.
        let mut constant_input = None;
        let value = if binding.comptime {
            let init = binding
                .init
                .ok_or_else(|| error(expr.span, "constant binding needs an initializer"))?;
            let annotation = binding.ty.map(|ty| self.type_expr(ty)).transpose()?;
            let input =
                self.with_staged_input(true, |this| this.expr(init, annotation.as_ref()))?;
            self.with_staged_input(true, |this| {
                this.check_open_fragment(&input, init.span, false)
            })?;
            let input_type = input.ty.clone();
            constant_input = Some(input);
            let mut checked = None;
            self.with_open_operand(input_type, init.span, |this, placeholder| {
                let binding = hir::decl::LetDecl {
                    init: Some(placeholder),
                    ..probe_binding.clone()
                };
                let probe = Expr {
                    kind: H::Let(&binding),
                    ..probe.clone()
                };
                // Reuse the already phase-checked input so pattern elaboration
                // sees its known type structure, without invoking it again.
                let temporary = local(placeholder.hir_id);
                this.generator_data.insert(temporary, constant_input.take());
                let value = this.expr(&probe, expected);
                constant_input = this.generator_data.remove(&temporary).flatten();
                let value = value?;
                let result = D::Known(value.ty.clone());
                checked = Some(value);
                Ok(result)
            })?;
            checked.ok_or_else(|| error(expr.span, "missing staged binding probe expression"))?
        } else {
            self.expr(&probe, expected)?
        };
        if let K::LetPattern {
            otherwise: Some(otherwise),
            ..
        } = &value.kind
        {
            self.check_open_fragment(otherwise, expr.span, false)?;
        }
        let K::LetPattern { pattern, .. } = &value.kind else {
            return Err(error(
                expr.span,
                "staged pattern probe lost its binding structure",
            ));
        };
        if let Some(input) = constant_input {
            // Preserve known Type identities through the abstract value probe.
            // This is structural projection only: no branch selection or call.
            for (id, value) in self.known_pattern_types(pattern, &input, expr.span)? {
                self.generator_data.insert(id, Some(value));
            }
        }
        let staged_bindings =
            subject_pattern_bindings(pattern, &mut self.required_steps, expr.span)?;
        for id in pattern_binding_ids(&binding.pattern) {
            if let Some(ty) = self.locals.get(&id) {
                locals.insert(id, D::Known(ty.clone()));
                if self.in_staged_input || binding.comptime || staged_bindings.contains(&id) {
                    self.open_staged_locals.insert(id);
                }
            }
        }
        if let Some(guard) = binding.guard {
            self.with_staged_input(self.in_staged_input || binding.comptime, |this| {
                this.describe_open_expression(
                    guard,
                    locals,
                    returns,
                    Some(&D::Known(p(P::Bool))),
                    depth + 1,
                )
            })?;
        }
        let K::LetPattern {
            pattern,
            value: input,
            ..
        } = &value.kind
        else {
            return Err(error(
                expr.span,
                "staged pattern probe lost its binding structure",
            ));
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let observations = nir::visit::pattern_observations(pattern, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        for observation in observations.map_err(|reason| error(expr.span, reason))? {
            self.check_open_fragment(observation, expr.span, false)?;
        }
        // Only the subject is a staged input. Pattern computations remain in
        // the enclosing phase; checking the entire pattern as staged would
        // incorrectly require ordinary predicates to be comptime functions.
        if !binding.comptime {
            self.with_staged_input(true, |this| {
                this.check_open_fragment(input, expr.span, false)
            })?;
        }
        Ok(D::Known(value.ty))
    }

    /// Retain the checked region until its consumer has validated behavior.
    /// Domain-only consumers may discard it after their phase check; annotation
    /// consumers also need its bindings and control flow for safe arithmetic.
    fn elaborate_known_region(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&D>,
    ) -> Result<n::Expr, Error> {
        let expected = match expected {
            Some(D::Known(ty)) => Some(ty),
            _ => None,
        };
        // Known regions still belong to definition checking. Runtime open
        // calls inside handlers or other wrappers retain symbolic staged
        // inputs just like direct calls described by the specialized paths.
        let saved_call = std::mem::replace(&mut self.describing_open_call, true);
        // Pattern transformations and other known wrappers can contain inline
        // selectors whose staged inputs are only supplied at invocation. Reuse
        // contextual description so selectors are phase-checked, not invoked.
        let saved_spread = std::mem::replace(&mut self.describing_open_spread, true);
        let context = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id && expected.is_none())
            .map(|(_, context)| context.clone());
        let value = match context {
            Some(context) => self.input_context_expression(expr, &context),
            None => self.expr(expr, expected),
        };
        self.describing_open_spread = saved_spread;
        self.describing_open_call = saved_call;
        let value = value.map_err(|mut failure| {
            if failure.message == "local binding is unavailable in this scope" {
                failure.message =
                    "operation on an unknown domain requires definition-side type or trait evidence".into();
            }
            failure
        })?;
        Ok(value)
    }

    fn describe_known_region(&mut self, expr: &Expr<'_>, expected: Option<&D>) -> Result<D, Error> {
        let value = self.elaborate_known_region(expr, expected)?;
        self.check_open_fragment(&value, expr.span, false)?;
        Ok(D::Known(value.ty))
    }

    fn describe_open_extended_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
        instantiated: bool,
    ) -> Result<D, Error> {
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            let saved = std::mem::replace(&mut self.describing_open_spread, true);
            let saved_call = std::mem::replace(&mut self.describing_open_call, true);
            let result = self.describe_known_region(expr, expected);
            self.describing_open_call = saved_call;
            self.describing_open_spread = saved;
            return result;
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return self.describe_known_region(expr, expected);
        };
        if !matches!(self.package.item(owner).map(|item| &item.kind), Some(ItemKind::Fn(signature, _) | ItemKind::Case(signature, _)) if !signature.modifiers.is_normal_form_head)
        {
            return self.describe_known_region(expr, expected);
        }
        self.ensure_signature(owner, expr.span)?;
        if !self.open_functions.contains_key(&owner) {
            return self.describe_known_region(expr, expected);
        }
        if !instantiated
            && args
                .iter()
                .any(|argument| matches!(argument, Arg::Named(..)))
        {
            if let H::NFApplication(head, clauses) = callee.kind {
                return self.with_explicit_open_clauses(owner, clauses, expr, |this| {
                    this.describe_open_extended_call(
                        expr, head, args, locals, returns, expected, depth, true,
                    )
                });
            }
        }
        let Some(ItemKind::Fn(signature, _) | ItemKind::Case(signature, _)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return self.describe_known_region(expr, expected);
        };
        if !args.iter().any(|arg| matches!(arg, Arg::Named(..))) {
            return self.describe_open_call(expr, callee, args, locals, returns, expected, depth);
        }
        let collectors = signature
            .params
            .iter()
            .filter(|parameter| matches!(parameter.ty.kind, TyParamKind::Varadic(..)))
            .map(|parameter| (parameter.name, parameter.index as usize))
            .collect::<Vec<_>>();
        let target = match expected {
            Some(D::Known(ty)) => Some(ty),
            _ => None,
        };
        let (name, _) =
            self.property_collector(owner, &collectors, None, target, None, expr.span)?;
        let mut names = BTreeSet::new();
        let mut properties = Vec::new();
        for argument in args {
            if let Arg::Named(name, value) = argument {
                if !names.insert(name.name.to_string()) {
                    return Err(error(name.span, "property supplied more than once"));
                }
                properties.push(hir::expr::FieldExpr {
                    ident: *name,
                    expr: value,
                    span: value.span,
                });
            }
        }
        // This is signature routing only. Runtime property producers keep the
        // source-order preparation built by collect_properties_resolved.
        let packet = Expr {
            hir_id: expr.hir_id,
            span: expr.span.shrink_to_lo(),
            kind: H::CollectedProperties(&properties),
        };
        let mut inputs = Vec::new();
        let mut supplied = false;
        for argument in args {
            match argument {
                Arg::Named(..) if !supplied => {
                    inputs.push(Arg::Named(name, &packet));
                    supplied = true;
                }
                Arg::Named(..) => (),
                other => inputs.push(other.clone()),
            }
        }
        if instantiated {
            let comptime_context = self.in_staged_input || self.signature_owner
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(signature, _) if signature.modifiers.is_comptime));
            if signature.modifiers.is_comptime != comptime_context {
                return Err(error(
                    callee.span,
                    "function is unavailable in this execution phase",
                ));
            }
            if inputs
                .iter()
                .any(|argument| matches!(argument, Arg::Expand(_)))
            {
                if let Some(description) = self.describe_open_runtime_spreads(
                    expr, owner, signature, &inputs, locals, returns, expected, depth, true,
                )? {
                    return Ok(description);
                }
                return Err(context_error(
                    expr.span,
                    "explicit property spread awaits input shape evidence",
                ));
            }
            self.describe_prepared_open_signature_call(
                expr,
                owner,
                signature,
                &inputs,
                locals,
                returns,
                depth,
                &[],
            )
        } else {
            self.describe_open_call(expr, callee, &inputs, locals, returns, expected, depth)
        }
    }

    fn describe_open_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        // Preserve per-element phase/type context, including open callees.
        if args.iter().any(|arg| matches!(arg, Arg::Expand(_))) {
            if let Some(flat) = arguments::literal_spread_arguments(args) {
                return self.describe_open_call(
                    expr,
                    callee,
                    &flat,
                    locals,
                    returns,
                    expected,
                    depth + 1,
                );
            }
        }
        let (head, explicit) = match callee.kind {
            H::NFApplication(head, types) => (head, Some(types)),
            _ => (callee, None),
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            let saved = std::mem::replace(&mut self.describing_open_spread, true);
            let saved_call = std::mem::replace(&mut self.describing_open_call, true);
            let result = self.describe_known_region(expr, expected);
            self.describing_open_call = saved_call;
            self.describing_open_spread = saved;
            return result;
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .ok_or_else(|| error(callee.span, "missing callable declaration"))?;
        if self
            .package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Const(..) | ItemKind::Associated(_)))
        {
            // A resolved value declaration follows the same callable-value
            // route as a local. Its source item need not be a function item.
            let saved = std::mem::replace(&mut self.describing_open_spread, true);
            let saved_call = std::mem::replace(&mut self.describing_open_call, true);
            let result = self.describe_known_region(expr, expected);
            self.describing_open_call = saved_call;
            self.describing_open_spread = saved;
            return result;
        }
        self.ensure_signature(owner, callee.span)?;
        self.ensure_contract(owner, callee.span)?;
        let Some(ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Err(error(
                callee.span,
                "ordinary deferred call requires a function declaration",
            ));
        };
        let comptime_context = self.in_staged_input || self.signature_owner
            .and_then(|owner| self.package.item(owner))
            .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(signature, _) if signature.modifiers.is_comptime));
        if sig.modifiers.is_comptime != comptime_context {
            return Err(error(
                callee.span,
                "function is unavailable in this execution phase",
            ));
        }
        if args
            .iter()
            .any(|argument| matches!(argument, Arg::Expand(_)))
        {
            let description = if let Some(clauses) =
                explicit.filter(|_| self.open_functions.contains_key(&owner))
            {
                // Establish only the explicit type telescope. Runtime-function
                // comptime inputs remain symbolic until actual invocation.
                if self.type_parameters[&owner].is_empty() {
                    return Err(error(
                        expr.span,
                        "explicit instantiation requires a generic function",
                    ));
                }
                let saved_open_call = std::mem::replace(&mut self.describing_open_call, true);
                let arguments = self.function_clause_arguments(expr, owner, clauses);
                self.describing_open_call = saved_open_call;
                let arguments = arguments?;
                self.check_type_application(owner, &arguments, expr.span)?;
                let saved = self.alias_substitution.clone();
                for (binder, argument) in self.declaration_binders(owner).into_iter().zip(arguments)
                {
                    self.alias_substitution
                        .insert(binder.declaration, Arc::new(argument));
                }
                let result = self.describe_open_runtime_spreads(
                    expr, owner, sig, args, locals, returns, expected, depth, true,
                );
                self.alias_substitution = saved;
                result?
            } else {
                self.describe_open_runtime_spreads(
                    expr, owner, sig, args, locals, returns, expected, depth, false,
                )?
            };
            if let Some(description) = description {
                return Ok(description);
            }
            // Reuse packet routing, contextual element checking, defaults and
            // source-ordered preparation, including open targets whose staged
            // inputs can be completed here. Unknown staged inputs still need
            // a description-only telescope; they must not be fabricated.
            // An unknown domain still lacks structural evidence for packet layout.
            let saved = std::mem::replace(&mut self.describing_open_spread, true);
            let saved_call = std::mem::replace(&mut self.describing_open_call, true);
            let result = self.describe_known_region(expr, expected);
            self.describing_open_call = saved_call;
            self.describing_open_spread = saved;
            return result;
        }
        if self.open_functions.contains_key(&owner) && !self.type_parameters[&owner].is_empty() {
            let arguments = if let Some(explicit) = explicit {
                let saved_open_call = std::mem::replace(&mut self.describing_open_call, true);
                let target = match expected {
                    Some(D::Known(ty)) => Some(ty),
                    _ => None,
                };
                let partial = self.with_partial_named_call_plan(
                    owner,
                    explicit,
                    expr,
                    args,
                    target,
                    None,
                    |this, plan| {
                        this.infer_open_clauses_with_plan(
                            expr,
                            owner,
                            args,
                            target,
                            Some((locals, returns, depth)),
                            plan,
                        )
                    },
                );
                self.describing_open_call = saved_open_call;
                match partial? {
                    Some(OpenClauseInference::Ready(arguments, _, _)) => arguments,
                    Some(OpenClauseInference::Pending(description)) => return Ok(description),
                    None => {
                        let saved = std::mem::replace(&mut self.describing_open_call, true);
                        let arguments = self.function_clause_arguments(expr, owner, explicit);
                        self.describing_open_call = saved;
                        arguments?
                    }
                }
            } else {
                let target = match expected {
                    Some(D::Known(ty)) => Some(ty),
                    _ => None,
                };
                match self.infer_open_clause_arguments(
                    expr,
                    owner,
                    args,
                    target,
                    Some((locals, returns, depth)),
                )? {
                    OpenClauseInference::Ready(arguments, _, _) => arguments,
                    OpenClauseInference::Pending(description) => return Ok(description),
                }
            };
            self.check_type_application(owner, &arguments, expr.span)?;
            let saved = self.alias_substitution.clone();
            for (binder, argument) in self.declaration_binders(owner).into_iter().zip(arguments) {
                self.alias_substitution
                    .insert(binder.declaration, Arc::new(argument));
            }
            let result =
                self.describe_open_signature_call(expr, owner, sig, args, locals, returns, depth);
            self.alias_substitution = saved;
            return result;
        }
        if !self.type_parameters[&owner].is_empty() {
            if sig.params.iter().all(|p| {
                matches!(
                    p.ty.kind,
                    TyParamKind::Positional(_) | TyParamKind::Optional(..)
                )
            }) {
                return self.describe_generic_call(
                    expr, owner, sig, args, explicit, locals, returns, expected, depth,
                );
            }
            return self.describe_known_region(expr, expected);
        }
        if explicit.is_some() {
            return self.describe_known_region(expr, expected);
        }
        self.describe_open_signature_call(expr, owner, sig, args, locals, returns, depth)
    }

    /// Derive packet contexts without reading invocation values. This is an
    /// inference candidate only; completed generic arguments are checked later.
    pub(super) fn open_spread_candidate(
        &mut self,
        owner: OwnerId,
        target: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        structural: Option<&crate::calls::InputContext>,
        args: &[Arg<'_>],
        checked: &mut BTreeMap<usize, (hir::HirId, n::Expr)>,
        packets: &mut BTreeMap<usize, n::Expr>,
        span: Span,
    ) -> Result<Option<crate::calls::CallCandidate>, Error> {
        let Some(definition) = self.open_functions.get(&owner).cloned() else {
            return Ok(None);
        };
        let all = self.type_parameters[&owner].clone();
        let lexical = self.lexical_function_arguments(owner, span)?;
        let (generic, template, _) = self.function_type_plan(owner, &lexical, span)?;
        let mut parameters = Vec::new();
        for (sort, annotation) in &definition.parameters {
            let Annotation::Known(ty) = annotation else {
                return Ok(None);
            };
            if !self.term_substitution_inputs(ty, span)?.is_empty() {
                // Callee value binders need invocation substitution, not the
                // caller's inference scope. Do not use their indices as types.
                return Ok(None);
            }
            parameters.push(Parameter {
                sort: *sort,
                ty: self.instantiate_input_types(ty, &all, &template, span)?,
            });
        }
        let (result, target, known_result) = match &definition.result {
            Annotation::Known(ty) => (
                self.instantiate_input_types(ty, &all, &template, span)?,
                target,
                true,
            ),
            // Private inference signature only, as in open clause inference.
            _ => (p(P::Void), None, false),
        };
        let (result, target, known_result) =
            if self.term_substitution_inputs(&result, span)?.is_empty() {
                (result, target, known_result)
            } else {
                (p(P::Void), None, false)
            };
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&generic)
                .iter()
                .map(|input| input.sort)
                .collect(),
            type_parameters: generic.len(),
            signature: Term::Function {
                parameters,
                result: Box::new(result),
            }
            .abstract_parameters(&self.input_binders(&generic))
            .map_err(|e| error(span, e.to_string()))?,
        };
        let actual = vec![None; definition.parameters.len()];
        let scope = self.call_inference_domains();
        let input_domains = generic
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let infer = |target| {
            scheme.infer_declared_sparse_types_mode(
                &generic,
                &input_domains,
                &actual,
                target,
                None,
                &self.context,
                crate::ptc::Budget::default(),
                self.active_effect_environment().is_some()
                    && !matches!(target, Some(Term::Computation { .. })),
                &scope,
            )
        };
        // This candidate supplies input contexts only. A result mismatch is
        // not final until packet completion is known. Retry this isolated type
        // problem without the result hint; no source expression is evaluated.
        let target = match infer(target) {
            Ok(_) => target,
            Err(original) if target.is_some() => {
                infer(None).map_err(|_| call_inference_error(span, original))?;
                None
            }
            Err(failure) => return Err(call_inference_error(span, failure)),
        };
        let Some(ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Err(error(span, "expected callable definition for spread check"));
        };
        self.infer_routed_packet_inputs(
            &scheme,
            &generic,
            actual,
            args,
            Some(sig.params),
            0,
            target,
            partial.filter(|_| known_result),
            structural.filter(|_| known_result),
            &definition
                .parameters
                .iter()
                .map(|(sort, _)| sig.modifiers.is_comptime || *sort != Sort::Runtime)
                .collect::<Vec<_>>(),
            span,
            checked,
            packets,
            true,
        )
        .map(Some)
    }

    /// Describe runtime packets without asking for the invocation's staged
    /// inputs. Packet producers are checked once and replayed only as phase /
    /// completion evidence in source order; no executable call is published.
    fn describe_open_runtime_spreads(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        args: &[Arg<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
        clauses_instantiated: bool,
    ) -> Result<Option<D>, Error> {
        let Some(definition) = self.open_functions.get(&owner).cloned() else {
            return Ok(None);
        };
        if args.iter().any(|arg| matches!(arg, Arg::Implicit(_)))
            || sig.params.iter().any(|parameter| {
                !matches!(
                    parameter.ty.kind,
                    TyParamKind::Positional(_)
                        | TyParamKind::Optional(..)
                        | TyParamKind::Varadic(..)
                )
            })
        {
            return Ok(None);
        }
        let stages = definition
            .parameters
            .iter()
            .map(|(sort, _)| *sort != Sort::Runtime)
            .collect::<Vec<_>>();
        let initial_types = definition
            .parameters
            .iter()
            .map(|(_, annotation)| match annotation {
                Annotation::Known(ty) => self
                    .specialize_open_clause_type(owner, ty, expr.span)
                    .map(Some),
                _ => Ok(None),
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let Some(initial_inputs) = self.spread_parameter_context(
            initial_types,
            Some(sig.params),
            &stages,
            args,
            expr.span,
        )?
        else {
            return Ok(None);
        };
        let positional_stages = initial_inputs
            .iter()
            .map(|(_, stage)| *stage)
            .collect::<Vec<_>>();
        let packet_stages = self.spread_packet_layout(args, &positional_stages, expr.span)?;
        if args.iter().zip(&packet_stages).any(|(arg, fields)| {
            // Literal tuples retain each element's own stage. Only an opaque
            // computed packet needs one phase for its entire producer.
            matches!(arg, Arg::Expand(value) if !matches!(value.kind, H::Tuple(_) | H::Unit))
                && fields
                    .as_ref()
                    .is_none_or(|fields| fields.iter().any(|stage| *stage))
        }) {
            return Ok(None);
        }
        let infer_clauses = !clauses_instantiated && !self.type_parameters[&owner].is_empty();
        let target = match expected {
            Some(D::Known(ty)) => Some(ty),
            _ => None,
        };
        let mut checked = BTreeMap::new();
        let mut packets = BTreeMap::new();
        let result_context = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id)
            .map(|(_, context)| context.clone());
        let candidate = if infer_clauses {
            let saved = std::mem::replace(&mut self.describing_open_spread, true);
            let saved_returns =
                if matches!(returns, Some(D::Unknown)) && self.control.result.is_none() {
                    Some(
                        self.control
                            .return_observations
                            .replace(ReturnObservations::Deferred),
                    )
                } else {
                    None
                };
            let candidate = self.with_open_reachability(false, expr.span, |this| {
                this.open_spread_candidate(
                    owner,
                    target,
                    None,
                    result_context.as_ref(),
                    args,
                    &mut checked,
                    &mut packets,
                    expr.span,
                )
            });
            self.describing_open_spread = saved;
            if let Some(saved) = saved_returns {
                self.control.return_observations = saved;
            }
            let Some(candidate) = candidate? else {
                return Ok(None);
            };
            Some(candidate)
        } else {
            None
        };
        let mut types = Vec::new();
        for (slot, (_, annotation)) in definition.parameters.iter().enumerate() {
            types.push(match &candidate {
                Some(candidate) => candidate
                    .parameter_input_context(slot)
                    .map_err(|e| call_inference_error(expr.span, e))?,
                None => match annotation {
                    Annotation::Known(ty) => crate::calls::InputContext::Known(
                        self.specialize_open_clause_type(owner, ty, expr.span)?,
                    ),
                    _ => crate::calls::InputContext::Unknown,
                },
            });
        }
        let Some(inputs) =
            self.spread_parameter_input_context(types, Some(sig.params), &stages, args, expr.span)?
        else {
            return Ok(None);
        };
        let types = inputs.into_iter().map(|(ty, _)| ty).collect::<Vec<_>>();
        let widths = self.checked_packet_widths(&packets, expr.span)?;
        let (contexts, partial_contexts) =
            self.spread_input_packet_contexts(args, &types, expr.span, &widths)?;
        self.required_steps = self
            .required_steps
            .checked_sub(self.open_loop_transfers.len() as u64)
            .ok_or_else(|| error(expr.span, "spread completion description budget exhausted"))?;
        let saved = std::mem::replace(&mut self.describing_open_spread, true);
        let saved_returns = if matches!(returns, Some(D::Unknown)) && self.control.result.is_none()
        {
            Some(
                self.control
                    .return_observations
                    .replace(ReturnObservations::Deferred),
            )
        } else {
            None
        };
        let transfers = self.open_loop_transfers.clone();
        let result = self.with_prepared_spreads(
            expr,
            args,
            &contexts,
            &vec![false; args.len()],
            Some(&partial_contexts),
            checked,
            packets,
            |this, flat, preparations, _| {
                // Producer probing cannot make source control flow reachable.
                // Observe the checked producers at their actual input positions.
                this.open_loop_transfers = transfers.clone();
                this.describing_open_spread = saved;
                this.required_steps = this
                    .required_steps
                    .checked_sub((locals.len() + this.locals.len()) as u64)
                    .ok_or_else(|| error(expr.span, "spread input description budget exhausted"))?;
                let mut descriptions = locals.clone();
                descriptions.extend(
                    this.locals
                        .iter()
                        .map(|(binding, ty)| (*binding, D::Known(ty.clone()))),
                );
                let saved_substitution = this.alias_substitution.clone();
                if infer_clauses {
                    let target = target
                        .filter(|_| !preparations.iter().any(|(_, value)| is_never(&value.ty)));
                    let inferred = this.with_open_reachability(false, expr.span, |this| {
                        this.infer_open_clause_arguments(
                            expr,
                            owner,
                            flat,
                            target,
                            Some((&mut descriptions, returns, depth)),
                        )
                    })?;
                    let OpenClauseInference::Ready(arguments, _, _) = inferred else {
                        return Err(context_error(
                            expr.span,
                            "spread call awaits generic type evidence",
                        ));
                    };
                    this.check_type_application(owner, &arguments, expr.span)?;
                    for (binder, argument) in
                        this.declaration_binders(owner).into_iter().zip(arguments)
                    {
                        this.alias_substitution
                            .insert(binder.declaration, Arc::new(argument));
                    }
                }
                let description = this.describe_prepared_open_signature_call(
                    expr,
                    owner,
                    sig,
                    flat,
                    &mut descriptions,
                    returns,
                    depth,
                    &preparations,
                );
                this.alias_substitution = saved_substitution;
                description
            },
        );
        if let Some(saved_returns) = saved_returns {
            self.control.return_observations = saved_returns;
        }
        self.describing_open_spread = saved;
        if result.is_err() {
            self.open_loop_transfers = transfers;
        }
        result.map(Some)
    }

    fn describe_open_signature_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        args: &[Arg<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        self.describe_prepared_open_signature_call(
            expr,
            owner,
            sig,
            args,
            locals,
            returns,
            depth,
            &[],
        )
    }

    fn describe_prepared_open_signature_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        args: &[Arg<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        depth: usize,
        preparations: &[(usize, n::Expr)],
    ) -> Result<D, Error> {
        let (parameters, result) = if let Some(definition) = self.open_functions.get(&owner) {
            (
                definition
                    .parameters
                    .iter()
                    .map(|(sort, annotation)| {
                        Ok((
                            *sort,
                            match annotation {
                                Annotation::Known(ty) => D::Known(
                                    self.specialize_open_clause_type(owner, ty, expr.span)?,
                                ),
                                _ => D::Unknown,
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, Error>>()?,
                match &definition.result {
                    Annotation::Known(ty) => {
                        D::Known(self.specialize_open_clause_type(owner, ty, expr.span)?)
                    }
                    _ => D::Unknown,
                },
            )
        } else {
            let signature = self
                .signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
                .ok_or_else(|| error(expr.span, "missing callable signature"))?;
            let Term::Function { parameters, result } = signature.callable_signature() else {
                return Err(error(expr.span, "expected callable function signature"));
            };
            (
                parameters
                    .iter()
                    .map(|parameter| (parameter.sort, D::Known(parameter.ty.clone())))
                    .collect(),
                D::Known((**result).clone()),
            )
        };
        let (routes, _) = route_arguments(args, Some(sig.params), sig.params.len(), expr.span)?;
        let mut symbols = vec![None; parameters.len()];
        let mut needed = BTreeSet::new();
        for requirement in &self.open_definition_obligations(owner) {
            for term in required::requirement_terms(requirement, expr.span)? {
                needed.extend(
                    self.term_substitution_inputs(term, expr.span)?
                        .keys()
                        .copied(),
                );
            }
        }
        for description in parameters
            .iter()
            .map(|(_, description)| description)
            .chain(std::iter::once(&result))
        {
            if let D::Known(ty) = description {
                needed.extend(
                    self.term_substitution_inputs(ty, expr.span)?
                        .keys()
                        .copied(),
                );
            }
        }
        for slot in needed {
            self.open_contract_argument(owner, slot, &parameters, &routes, expr, &mut symbols, 0)?;
        }
        // Known descriptions belong to the callee binder. Transport them before
        // comparing caller operands; coincident indices are not type evidence.
        let mut diverges = false;
        let mut prepared = preparations.iter().peekable();
        // A collection may interleave with named inputs and packet producers.
        // Describe operands in source order, not grouped by parameter route.
        let mut inputs = Vec::new();
        for route in &routes {
            let (sort, ty) = &parameters[route.parameter];
            let expected = match substitute_description(ty, &symbols, expr.span)? {
                D::Known(ty) => D::Known(self.observe_call_type(&ty, expr)?),
                D::Unknown => D::Unknown,
            };
            if route.is_collected() {
                let fields = match &expected {
                    D::Known(ty) => {
                        route.require_collection_type(self, ty, expr.span)?;
                        let Term::Tuple(fields) = self.structural_type(ty, expr.span)? else {
                            return Err(error(
                                expr.span,
                                "variadic collection domain must lower to a tuple structure",
                            ));
                        };
                        if fields.len() != route.source_inputs().count() {
                            return Err(error(
                                expr.span,
                                "variadic collection argument count mismatch",
                            ));
                        }
                        Some(fields)
                    }
                    D::Unknown => None,
                };
                for (field, (position, input)) in route.source_inputs().enumerate() {
                    let expected = fields
                        .as_ref()
                        .map_or(D::Unknown, |fields| D::Known(fields[field].clone()));
                    inputs.push((position, input.clone(), *sort, expected));
                }
            } else {
                inputs.push((
                    route.single_position().ok_or_else(|| {
                        error(
                            expr.span,
                            "non-variadic argument route has no source position",
                        )
                    })?,
                    route.expression(expr),
                    *sort,
                    expected,
                ));
            }
        }
        inputs.sort_by_key(|(position, ..)| *position);
        for (position, input, sort, expected) in inputs {
            while let Some((_, statement)) = prepared.next_if(|(at, _)| *at <= position) {
                let value = match &statement.kind {
                    K::LetPattern { value, .. } | K::Let { value, .. } => value,
                    _ => {
                        return Err(error(
                            expr.span,
                            "unexpected prepared statement fragment in open call",
                        ));
                    }
                };
                self.with_open_reachability(!diverges, expr.span, |this| {
                    this.check_open_fragment(value, expr.span, false)
                })?;
                diverges |= is_never(&value.ty);
            }
            let description = self.with_open_reachability(!diverges, input.span, |this| {
                this.with_staged_input(this.in_staged_input || sort != Sort::Runtime, |this| {
                    this.describe_open_expression(
                        &input,
                        locals,
                        returns,
                        Some(&expected),
                        depth + 1,
                    )
                })
            })?;
            diverges |= matches!(description, D::Known(ref ty) if is_never(ty));
        }
        for (_, statement) in prepared {
            let value = match &statement.kind {
                K::LetPattern { value, .. } | K::Let { value, .. } => value,
                _ => {
                    return Err(error(
                        expr.span,
                        "unexpected trailing prepared statement fragment in open call",
                    ));
                }
            };
            self.with_open_reachability(!diverges, expr.span, |this| {
                this.check_open_fragment(value, expr.span, false)
            })?;
            diverges |= is_never(&value.ty);
        }
        self.admit_open_type_requirements(owner, &symbols, false, expr.span)?;
        let result = match substitute_description(&result, &symbols, expr.span)? {
            D::Known(ty) => D::Known(self.observe_call_type(&ty, expr)?),
            D::Unknown => D::Unknown,
        };
        Ok(if diverges {
            D::Known(p(P::Never))
        } else {
            result
        })
    }

    fn describe_generic_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        args: &[Arg<'_>],
        explicit: Option<&[Arg<'_>]>,
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let signature = self
            .signatures
            .get(&owner)
            .or_else(|| self.interface_signatures.get(&owner))
            .ok_or_else(|| error(expr.span, "generic call requires a formed signature"))?;
        // Runtime capability admission needs evidence about the operand's
        // actual domain. A result-directed guess cannot turn an unknown domain into that
        // evidence; retain ordinary typed checking for constrained calls.
        let constrained = self.capabilities.get(&owner).is_some_and(|r| !r.is_empty())
            || self.requirements.get(&owner).is_some_and(|r| !r.is_empty());
        if !self.in_staged_input && constrained {
            return self.describe_known_region(expr, expected);
        }
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            return self.describe_known_region(expr, expected);
        };
        if parameters.iter().any(|p| p.sort != Sort::Runtime) {
            return self.describe_known_region(expr, expected);
        }
        let signature = signature.clone();
        if let Some(types) = explicit {
            let saved = std::mem::replace(&mut self.describing_open_call, true);
            let partial = self.with_partial_named_call_plan(
                owner,
                types,
                expr,
                args,
                match expected {
                    Some(D::Known(ty)) => Some(ty),
                    _ => None,
                },
                None,
                |this, plan| {
                    this.describing_open_call = saved;
                    this.describe_generic_call_with_plan(
                        expr,
                        owner,
                        sig,
                        args,
                        locals,
                        returns,
                        expected,
                        depth,
                        &signature,
                        constrained,
                        plan,
                    )
                },
            );
            self.describing_open_call = saved;
            if let Some(description) = partial? {
                return Ok(description);
            }
        }
        let fixed = match explicit {
            Some(types) => {
                let saved_open_call = std::mem::replace(&mut self.describing_open_call, true);
                let arguments = self.function_clause_arguments(expr, owner, types);
                self.describing_open_call = saved_open_call;
                arguments?
            }
            None => self.lexical_function_arguments(owner, expr.span)?,
        };
        let plan = self.function_type_plan(owner, &fixed, expr.span)?;
        self.describe_generic_call_with_plan(
            expr,
            owner,
            sig,
            args,
            locals,
            returns,
            expected,
            depth,
            &signature,
            constrained,
            plan,
        )
    }

    fn describe_generic_call_with_plan(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        sig: &hir::item::FnSig<'_>,
        args: &[Arg<'_>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
        signature: &Term,
        constrained: bool,
        plan: type_defaults::FunctionTypePlan<'_>,
    ) -> Result<D, Error> {
        let result_context = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id)
            .map(|(_, context)| context.clone());
        let (generic, template, pending_defaults) = plan;
        let all_generic = self.type_parameters[&owner].clone();
        let signature =
            self.instantiate_input_types(signature, &all_generic, &template, expr.span)?;
        let input_domains = generic
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&generic)
                .iter()
                .map(|input| input.sort)
                .collect(),
            type_parameters: generic.len(),
            signature: signature
                .abstract_parameters(&self.input_binders(&generic))
                .map_err(|e| error(expr.span, e.to_string()))?,
        };
        let (routes, assigned) =
            route_arguments(args, Some(sig.params), sig.params.len(), expr.span)?;
        let target = match expected {
            Some(D::Known(ty)) => Some(ty),
            _ => None,
        };
        let scope = self.call_inference_domains();
        let mut actual = vec![None; sig.params.len()];
        let mut default_diverges = false;
        for (slot, assigned) in assigned.iter().enumerate() {
            if *assigned {
                continue;
            }
            self.ensure_parameter_default(owner, slot, expr.span)?;
            let default = self
                .defaults
                .get(&owner)
                .and_then(|values| values.get(slot))
                .and_then(Option::as_ref)
                .ok_or_else(|| error(expr.span, "missing generic call default"))?;
            let ty =
                self.instantiate_input_types(&default.ty, &all_generic, &template, expr.span)?;
            default_diverges |= is_never(&ty);
            // A definition-side rigid type is not fresh caller evidence for
            // that same generic. Concrete default types may supply evidence.
            if !is_never(&ty) {
                let inputs = self.term_declaration_inputs(&ty, expr.span)?;
                if !generic.iter().any(|id| inputs.contains(id)) {
                    actual[slot] = Some(ty);
                }
            }
        }
        let mut initial = scheme
            .infer_declared_sparse_types_mode(
                &generic,
                &input_domains,
                &actual,
                target,
                None,
                &self.context,
                crate::ptc::Budget::default(),
                self.active_effect_environment().is_some()
                    && !matches!(target, Some(Term::Computation { .. })),
                &scope,
            )
            .map_err(|e| call_inference_error(expr.span, e))?;
        if let Some(context) = &result_context {
            initial
                .constrain_result_context(context, crate::ptc::Budget::default())
                .map_err(|e| call_inference_error(expr.span, e))?;
        }
        let mut diverges = default_diverges;
        let mut resolved_defaults = BTreeMap::new();
        self.settle_function_defaults(
            owner,
            &generic,
            &template,
            &pending_defaults,
            &mut resolved_defaults,
            &mut initial,
            expr.span,
        )?;
        self.required_steps = self
            .required_steps
            .checked_sub(self.open_loop_transfers.len() as u64)
            .ok_or_else(|| error(expr.span, "call completion description budget exhausted"))?;
        let incoming_transfers = self.open_loop_transfers.clone();
        let mut argument_transfers = vec![None; sig.params.len()];
        let mut argument_never = vec![false; sig.params.len()];
        let mut checked = vec![false; sig.params.len()];
        let mut unknown = vec![false; sig.params.len()];
        loop {
            let mut progress = false;
            let mut waiting = None;
            for route in &routes {
                if checked[route.parameter] {
                    continue;
                }
                let input_context = initial
                    .parameter_input_context(route.parameter)
                    .map_err(|e| call_inference_error(expr.span, e))?;
                let hint = match &input_context {
                    crate::calls::InputContext::Known(ty) => Some(D::Known(ty.clone())),
                    _ => None,
                };
                let input = route.expression(expr);
                let mut attempt_locals = locals.clone();
                // Inference may revisit inputs out of execution order. Keep
                // each successful input's transfers separate until routing is settled.
                self.required_steps = self
                    .required_steps
                    .checked_sub(incoming_transfers.len() as u64)
                    .ok_or_else(|| {
                        error(input.span, "call completion description budget exhausted")
                    })?;
                let saved_transfers =
                    std::mem::replace(&mut self.open_loop_transfers, incoming_transfers.clone());
                let saved_context = self
                    .structural_result_context
                    .replace((input.hir_id, input_context));
                let attempt = self.describe_open_expression(
                    &input,
                    &mut attempt_locals,
                    returns,
                    hint.as_ref(),
                    depth + 1,
                );
                self.structural_result_context = saved_context;
                let transfers = std::mem::replace(&mut self.open_loop_transfers, saved_transfers);
                let description = match attempt {
                    Ok(description) => description,
                    Err(failure) if failure.needs_context => {
                        waiting = Some(failure);
                        continue;
                    }
                    Err(failure) => return Err(failure),
                };
                argument_transfers[route.parameter] = Some(transfers);
                argument_never[route.parameter] =
                    matches!(&description, D::Known(ty) if is_never(ty));
                *locals = attempt_locals;
                checked[route.parameter] = true;
                unknown[route.parameter] = matches!(description, D::Unknown);
                progress = true;
                if let D::Known(ty) = description {
                    if is_never(&ty) {
                        diverges = true;
                    } else {
                        actual[route.parameter] = Some(ty);
                    }
                }
                initial = scheme
                    .infer_declared_sparse_types_mode(
                        &generic,
                        &input_domains,
                        &actual,
                        target,
                        None,
                        &self.context,
                        crate::ptc::Budget::default(),
                        self.active_effect_environment().is_some()
                            && !matches!(target, Some(Term::Computation { .. })),
                        &scope,
                    )
                    .map_err(|e| call_inference_error(expr.span, e))?;
                if let Some(context) = &result_context {
                    initial
                        .constrain_result_context(context, crate::ptc::Budget::default())
                        .map_err(|e| call_inference_error(expr.span, e))?;
                }
                self.settle_function_defaults(
                    owner,
                    &generic,
                    &template,
                    &pending_defaults,
                    &mut resolved_defaults,
                    &mut initial,
                    expr.span,
                )?;
            }
            if routes.iter().all(|route| checked[route.parameter]) {
                break;
            }
            if !progress {
                return Err(waiting.unwrap_or_else(|| {
                    context_error(expr.span, "generic inputs await type evidence")
                }));
            }
        }
        let mut reachable = true;
        for route in &routes {
            if !reachable {
                break;
            }
            if let Some(transfers) = argument_transfers[route.parameter].take() {
                self.required_steps = self
                    .required_steps
                    .checked_sub(transfers.len() as u64)
                    .ok_or_else(|| {
                        error(expr.span, "call completion description budget exhausted")
                    })?;
                for (id, transfer) in transfers {
                    if let Some(found) = self.open_loop_transfers.get_mut(&id) {
                        found.include(transfer);
                    }
                }
            }
            reachable &= !argument_never[route.parameter];
        }
        if !constrained && unknown.iter().any(|unknown| *unknown) {
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Err(error(expr.span, "expected callable function signature"));
            };
            // One observation per unknown operand domain, not one scan per
            // generic binder. This support belongs only to this description.
            let mut unknown_inputs = BTreeSet::new();
            for (parameter, unknown) in parameters.iter().zip(&unknown) {
                if *unknown {
                    unknown_inputs.extend(self.term_declaration_inputs(&parameter.ty, expr.span)?);
                }
            }
            let mut suspended = false;
            let mut supported = BTreeSet::new();
            let mut unresolved = Vec::new();
            for (index, generic) in generic.iter().enumerate() {
                if initial
                    .type_argument(index)
                    .map_err(|e| call_inference_error(expr.span, e))?
                    .is_some()
                {
                    supported.insert(*generic);
                    continue;
                }
                unresolved.push(*generic);
                if unknown_inputs.contains(generic) {
                    supported.insert(*generic);
                }
                suspended = true;
            }
            // A default has an inference source too: it waits for its input
            // types rather than inventing an independent unknown. Follow only
            // dependencies already justified by actual operands or known types.
            // Cycles and unrelated phantom binders cannot seed this closure.
            loop {
                let mut progress = false;
                for clause in &pending_defaults {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(1 + clause.dependencies.len() as u64)
                        .ok_or_else(|| {
                            error(expr.span, "generic default dependency budget exhausted")
                        })?;
                    if clause.dependencies.iter().all(|id| supported.contains(id)) {
                        progress |= supported.insert(clause.binder);
                    }
                }
                if !progress {
                    break;
                }
            }
            if unresolved.iter().any(|id| !supported.contains(id)) {
                return Err(context_error(
                    expr.span,
                    "generic input has no inference evidence",
                ));
            }
            if suspended {
                // This is only an ordinary description. No incomplete type or
                // candidate is frozen into NIR; concrete checking repeats the call.
                return if diverges {
                    Ok(D::Known(p(P::Never)))
                } else {
                    initial
                        .result_type()
                        .map(|ty| ty.map_or(D::Unknown, D::Known))
                        .map_err(|e| call_inference_error(expr.span, e))
                };
            }
        }
        let instance = initial
            .finish()
            .map_err(|e| call_inference_error(expr.span, e))?;
        let full_arguments = template
            .iter()
            .map(|ty| {
                self.instantiate_input_types(ty, &generic, &instance.type_arguments, expr.span)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.check_type_application(owner, &full_arguments, expr.span)?;
        let symbols = vec![None; sig.params.len()];
        self.check_ordinary_capabilities(owner, &full_arguments, &symbols, expr.span)?;
        self.check_ordinary_type_requirements(owner, &full_arguments, &symbols, expr.span)?;
        Ok(D::Known(if diverges {
            p(P::Never)
        } else {
            instance.result
        }))
    }

    // Resolve only inputs demanded by the type contract. An explicitly supplied
    // argument suppresses its default, including any nonterminating computation.
    fn open_contract_argument(
        &mut self,
        owner: OwnerId,
        slot: usize,
        parameters: &[(Sort, D)],
        routes: &[arguments::RoutedArgument<'_>],
        call: &Expr<'_>,
        symbols: &mut [Option<Term>],
        depth: usize,
    ) -> Result<(), Error> {
        if symbols.get(slot).is_some_and(Option::is_some) {
            return Ok(());
        }
        if depth >= 128 {
            return Err(error(
                call.span,
                "open contract input dependency depth exhausted",
            ));
        }
        let Some((sort, D::Known(domain))) = parameters.get(slot) else {
            return Err(error(call.span, "type contract input has no known domain"));
        };
        if *sort == Sort::Runtime {
            return Err(error(
                call.span,
                "type contract cannot read a runtime argument",
            ));
        }
        if let Some(route) = routes.iter().find(|route| route.parameter == slot) {
            for dependency in self.term_substitution_inputs(domain, call.span)?.keys() {
                if *dependency >= slot {
                    return Err(error(
                        call.span,
                        "staged input domain depends on a later parameter",
                    ));
                }
                self.open_contract_argument(
                    owner,
                    *dependency,
                    parameters,
                    routes,
                    call,
                    symbols,
                    depth + 1,
                )?;
            }
            let domain = domain
                .substitute_partial(symbols)
                .map_err(|failure| error(call.span, failure.to_string()))?;
            let input = route.expression(call);
            let identity = if domain == p(P::Type) && self.describing_input_of(input.hir_id.owner) {
                self.completed_type_in(&input, true)?.identity(input.span)?
            } else {
                self.normal_value_argument(&input, &domain)?
            };
            symbols[slot] = Some(identity);
            return Ok(());
        }
        let term = self.in_open_function_scope(owner, None, |this| {
            let sig = this.open_fn_or_case_sig(owner, call.span)?;
            let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                return Err(error(call.span, "missing open contract argument"));
            };
            this.in_staged_input = true;
            for (index, (parameter, (sort, description))) in
                sig.params.iter().zip(parameters).take(slot).enumerate()
            {
                if let D::Known(ty) = description {
                    let binding = local(parameter.hir_id);
                    this.locals.insert(binding, ty.clone());
                    this.input_facts.parameters.bind(binding, index, *sort, ty);
                }
            }
            this.normal_value_argument(default, domain)
        })?;
        for dependency in self.term_substitution_inputs(&term, call.span)?.keys() {
            if *dependency >= slot {
                return Err(error(
                    call.span,
                    "default type contract input depends on a later parameter",
                ));
            }
            self.open_contract_argument(
                owner,
                *dependency,
                parameters,
                routes,
                call,
                symbols,
                depth + 1,
            )?;
        }
        symbols[slot] = Some(
            term.substitute_partial(symbols)
                .map_err(|failure| error(call.span, failure.to_string()))?,
        );
        Ok(())
    }

    fn open_definition_obligations(&self, owner: OwnerId) -> Vec<n::TypePredicate> {
        let mut obligations = self.definition_requirements(owner);
        obligations.extend(
            self.capabilities
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .map(n::TypePredicate::Trait),
        );
        obligations
    }

    fn specialized_open_capabilities(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Vec<n::TraitRequirement>, Error> {
        self.capabilities
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|requirement| {
                Ok(n::TraitRequirement {
                    interface: requirement.interface,
                    subject: self
                        .specialize_open_clause_type(owner, &requirement.subject, span)?
                        .substitute_partial(symbols)
                        .map_err(|failure| error(span, failure.to_string()))?,
                    interface_arguments: requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| {
                            self.specialize_open_clause_type(owner, ty, span)?
                                .substitute_partial(symbols)
                                .map_err(|failure| error(span, failure.to_string()))
                        })
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect()
    }

    fn specialized_open_binder_assumptions(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Vec<n::TraitRequirement>, Error> {
        self.binder_assumptions(owner)
            .into_iter()
            .map(|requirement| {
                Ok(n::TraitRequirement {
                    interface: requirement.interface,
                    subject: self
                        .specialize_open_clause_type(owner, &requirement.subject, span)?
                        .substitute_partial(symbols)
                        .map_err(|failure| error(span, failure.to_string()))?,
                    interface_arguments: requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| {
                            self.specialize_open_clause_type(owner, ty, span)?
                                .substitute_partial(symbols)
                                .map_err(|failure| error(span, failure.to_string()))
                        })
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect()
    }

    fn specialized_open_predicates(
        &self,
        owner: OwnerId,
        predicates: &[n::TypePredicate],
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Vec<n::TypePredicate>, Error> {
        predicates
            .iter()
            .map(|predicate| {
                required::map_requirement(
                    predicate,
                    &mut |ty| {
                        self.specialize_open_clause_type(owner, ty, span)?
                            .substitute_partial(symbols)
                            .map_err(|failure| {
                                error(
                                    span,
                                    format!(
                                        "open type contract requires available comptime inputs: {failure}"
                                    ),
                                )
                            })
                    },
                    span,
                    0,
                )
            })
            .collect()
    }

    fn specialized_open_requirements(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Vec<n::TypePredicate>, Error> {
        let requirements = self.definition_requirements(owner);
        self.specialized_open_predicates(owner, &requirements, symbols, span)
    }

    fn specialized_open_result_guarantees(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<(Option<DeclarationId>, Vec<n::TypePredicate>), Error> {
        let Some((binder, guarantees)) = self.result_guarantees.get(&owner) else {
            return Ok((None, Vec::new()));
        };
        let guarantees = self.specialized_open_predicates(owner, guarantees, symbols, span)?;
        Ok((Some(*binder), guarantees))
    }

    fn admit_open_type_requirements(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        allow_pending: bool,
        span: Span,
    ) -> Result<(), Error> {
        for requirement in self.open_definition_obligations(owner) {
            let mut pending = false;
            let requirement = required::map_requirement(
                &requirement,
                &mut |ty| match self
                    .specialize_open_clause_type(owner, ty, span)?
                    .substitute_partial(symbols)
                {
                    Ok(ty) => Ok(ty),
                    Err(nir::TypeError::MissingArgument(slot))
                        if allow_pending && symbols.get(slot).is_some_and(Option::is_none) =>
                    {
                        pending = true;
                        Ok(ty.clone())
                    }
                    Err(failure) => Err(error(
                        span,
                        format!("open type contract requires available comptime inputs: {failure}"),
                    )),
                },
                span,
                0,
            )?;
            // Only completed premises are checked or used. The final admission
            // requires every staged input and cannot skip a pending obligation.
            if pending {
                continue;
            }
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
                .map_err(|message| error(span, message))?;
            }
            if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
                return Err(error(
                    span,
                    format!(
                        "open function type requirement is not established: {requirement}: {outcome}"
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn specialize_open_clause_type(
        &self,
        owner: OwnerId,
        ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let mut parameters = Vec::new();
        let mut arguments = Vec::new();
        for id in &self.type_parameters[&owner] {
            if let Some(value) = self.alias_substitution.get(id) {
                parameters.push(*id);
                arguments.push(value.clone());
            }
        }
        if parameters.is_empty() {
            return Ok(ty.clone());
        }
        let frames = [self.open_functions[&owner].input_sorts()];
        self.instantiate_input_types_in_scope(ty, &parameters, &arguments, &frames, span)
    }

    fn open_annotation_requirements_ready(
        &mut self,
        owner: OwnerId,
        clauses: &[DeclarationId],
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<bool, Error> {
        for requirement in self.open_definition_obligations(owner) {
            for ty in required::requirement_terms(&requirement, span)? {
                // No callee assumption becomes evidence for an unresolved
                // declaration argument. Ordinary input facts can be discharged
                // once their actual immutable symbols have arrived.
                let inputs = self.observed_term_inputs(ty, span)?;
                if clauses
                    .iter()
                    .any(|id| inputs.declarations.contains_key(id))
                {
                    return Ok(false);
                }
                if inputs
                    .parameters
                    .keys()
                    .any(|slot| symbols.get(*slot).is_none_or(Option::is_none))
                {
                    return Ok(false);
                }
            }
        }
        self.admit_open_type_requirements(owner, symbols, false, span)?;
        Ok(true)
    }

    fn open_computation_uses_clauses(
        &mut self,
        computation: &PreparedRequired,
        clauses: &[DeclarationId],
        span: Span,
    ) -> Result<bool, Error> {
        computation.uses_declaration_inputs(clauses, &mut self.required_steps, span)
    }

    fn infer_open_clause_arguments<'src>(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'src>],
        expected: Option<&Term>,
        description: Option<(&mut BTreeMap<n::LocalId, D>, Option<&D>, usize)>,
    ) -> Result<OpenClauseInference<'src>, Error> {
        let lexical = self.lexical_function_arguments(owner, expr.span)?;
        let plan = self.function_type_plan(owner, &lexical, expr.span)?;
        self.infer_open_clauses_with_plan(expr, owner, args, expected, description, plan)
    }

    fn infer_open_clauses_with_plan<'src>(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'src>],
        expected: Option<&Term>,
        mut description: Option<(&mut BTreeMap<n::LocalId, D>, Option<&D>, usize)>,
        plan: type_defaults::FunctionTypePlan<'_>,
    ) -> Result<OpenClauseInference<'src>, Error> {
        self.check_open_interface_dependency(owner)?;
        let mut definition = self.open_functions[&owner].clone();
        let sig = self.open_fn_or_case_sig(owner, expr.span)?;
        let (routes, assigned) =
            route_arguments(args, Some(sig.params), sig.params.len(), expr.span)?;
        let all = self.type_parameters[&owner].clone();
        let (generic, template, pending) = plan;
        let mut checked = StagedInputs::new(sig.params.len());
        let mut formed = BTreeMap::new();
        // Prepare staged inputs demanded by formed annotations.
        // Preserve both their value and symbol for the final invocation, so
        // inference never invokes the same supplier twice.
        if description.is_none() {
            let mut needed = BTreeSet::new();
            let mut source_dependencies = BTreeMap::new();
            for annotation in definition
                .parameters
                .iter()
                .map(|(_, ty)| ty)
                .chain(std::iter::once(&definition.result))
            {
                if let Annotation::Known(ty) = annotation {
                    needed.extend(
                        self.term_substitution_inputs(ty, expr.span)?
                            .keys()
                            .copied(),
                    );
                }
            }
            for requirement in self.open_definition_obligations(owner) {
                for ty in required::requirement_terms(&requirement, expr.span)? {
                    needed.extend(
                        self.term_substitution_inputs(ty, expr.span)?
                            .keys()
                            .copied(),
                    );
                }
            }
            {
                for slot in 0..=sig.params.len() {
                    let annotation = definition
                        .parameters
                        .get(slot)
                        .map(|(_, ty)| ty)
                        .unwrap_or(&definition.result);
                    if !annotation.awaits_inputs() {
                        continue;
                    }
                    if let Some(source) = annotation_source(sig, slot) {
                        let value = if let H::Defer(value) = source.kind {
                            value
                        } else {
                            source
                        };
                        let dependencies = self.expression_parameter_dependencies(owner, value)?;
                        needed.extend(dependencies.iter().copied());
                        source_dependencies.insert(slot, dependencies);
                    }
                }
            }
            if sig.modifiers.is_comptime {
                needed.extend(
                    definition
                        .parameters
                        .iter()
                        .enumerate()
                        .filter_map(|(slot, (sort, _))| (*sort != Sort::Runtime).then_some(slot)),
                );
            }
            if let Some(last) = needed
                .iter()
                .copied()
                .filter(|&i| {
                    !assigned[i]
                        && (sig.modifiers.is_comptime
                            || definition.parameters[i].0 != Sort::Runtime)
                })
                .max()
            {
                needed.extend((0..=last).filter(|&i| {
                    !assigned[i]
                        && (sig.modifiers.is_comptime
                            || definition.parameters[i].0 != Sort::Runtime)
                }));
            }
            // Follow only selected defaults. Overridden defaults contribute
            // neither values nor dependency edges to this invocation.
            let mut pending_dependencies: Vec<_> = needed.iter().copied().collect();
            while let Some(slot) = pending_dependencies.pop() {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(expr.span, "default dependency budget exhausted"))?;
                if !assigned[slot] {
                    if let Some(dependencies) = definition.default_dependencies.get(&slot) {
                        for dependency in dependencies {
                            if needed.insert(*dependency) {
                                pending_dependencies.push(*dependency);
                            }
                        }
                    }
                }
            }
            let independent = self.signature_independent_of_producers(
                &definition,
                sig,
                &routes,
                &needed,
                expr.span,
            )?;
            for slot in 0..=sig.params.len() {
                let annotation = definition
                    .parameters
                    .get(slot)
                    .map(|(_, ty)| ty)
                    .unwrap_or(&definition.result)
                    .clone();
                if annotation.awaits_inputs()
                    && self.open_annotation_requirements_ready(
                        owner,
                        &all,
                        &checked.symbols,
                        expr.span,
                    )?
                {
                    let ready = match &annotation {
                        Annotation::Pending(computation) => {
                            !self.open_computation_uses_clauses(computation, &all, expr.span)?
                                && computation.pending_parameters().all(|slot| {
                                    slot.is_some_and(|index| {
                                        checked.symbols[index].is_some()
                                            || checked.live[index].is_some()
                                    })
                                })
                        }
                        Annotation::InputType { slot } => checked.symbols[*slot].is_some(),
                        Annotation::Suspended { .. } => source_dependencies
                            .get(&slot)
                            .ok_or_else(|| {
                                error(expr.span, "missing annotation dependency observation")
                            })?
                            .iter()
                            .all(|index| {
                                checked.symbols[*index].is_some() || checked.live[*index].is_some()
                            }),
                        Annotation::Known(_) | Annotation::Description(_) => false,
                    };
                    if ready {
                        let mut prefix = Vec::new();
                        for (sort, annotation) in definition.parameters.iter().take(slot) {
                            let ty = if let Annotation::Known(ty) = annotation {
                                match ty.substitute_partial(&checked.symbols) {
                                    Ok(ty) => Some(ty),
                                    Err(nir::TypeError::MissingArgument(_)) => None,
                                    Err(e) => return Err(error(expr.span, e.to_string())),
                                }
                            } else {
                                None
                            };
                            let sort = if *sort != Sort::Runtime && ty.as_ref() == Some(&p(P::Type))
                            {
                                Sort::Type
                            } else {
                                *sort
                            };
                            prefix.push((sort, ty));
                        }
                        let ty = self.instantiate_open_annotation_with_live_in_scope(
                            owner,
                            slot,
                            &annotation,
                            sig,
                            &checked.symbols,
                            &prefix,
                            &checked.live,
                            expr,
                        )?;
                        formed.insert(slot, ty.clone());
                        let ty = Annotation::from(ty);
                        if slot == sig.params.len() {
                            definition.result = ty;
                        } else {
                            definition.parameters[slot].1 = ty;
                        }
                    }
                }
                if slot == sig.params.len() || !needed.contains(&slot) {
                    continue;
                }
                let Some((sort, Annotation::Known(domain))) = definition.parameters.get(slot)
                else {
                    continue;
                };
                if checked.symbols[slot].is_some()
                    || checked.live[slot].is_some()
                    || (*sort == Sort::Runtime && !sig.modifiers.is_comptime)
                {
                    continue;
                }
                let domain = match domain.substitute_partial(&checked.symbols) {
                    Ok(domain) => domain,
                    Err(nir::TypeError::MissingArgument(_)) => continue,
                    Err(e) => return Err(error(expr.span, e.to_string())),
                };
                let domain = self.instantiate_input_types(&domain, &all, &template, expr.span)?;
                let inputs = self.observed_term_inputs(&domain, expr.span)?;
                if generic
                    .iter()
                    .any(|id| inputs.declarations.contains_key(id))
                    || !inputs.parameters.is_empty()
                {
                    continue;
                }
                if sig.modifiers.is_comptime && !independent {
                    // Inference may need ordinary inputs to form a callback
                    // domain. Use the same source-order preparation as final
                    // invocation, and retain its values for that invocation.
                    let mut prefix = Vec::new();
                    for (sort, annotation) in definition.parameters.iter().take(slot + 1) {
                        let Annotation::Known(ty) = annotation else {
                            break;
                        };
                        let ty = match ty.substitute_partial(&checked.symbols) {
                            Ok(ty) => {
                                self.instantiate_input_types(&ty, &all, &template, expr.span)?
                            }
                            Err(nir::TypeError::MissingArgument(_)) => break,
                            Err(e) => return Err(error(expr.span, e.to_string())),
                        };
                        let inputs = self.term_declaration_inputs(&ty, expr.span)?;
                        if generic.iter().any(|id| inputs.contains(id)) {
                            break;
                        }
                        prefix.push(Parameter { sort: *sort, ty });
                    }
                    if prefix.len() != slot + 1 {
                        continue;
                    }
                    let mut preparation = definition.clone();
                    for (_, annotation) in &mut preparation.parameters {
                        if let Annotation::Known(ty) = annotation {
                            *ty = self.instantiate_input_types(ty, &all, &template, expr.span)?;
                        }
                    }
                    let end = Self::open_comptime_prefix_end(&routes, slot);
                    let mut ready = true;
                    for route in &routes {
                        if route
                            .source_inputs()
                            .next()
                            .is_some_and(|(position, _)| position > end)
                        {
                            continue;
                        }
                        if let Annotation::Known(ty) = &preparation.parameters[route.parameter].1 {
                            let inputs = self.term_declaration_inputs(ty, expr.span)?;
                            if generic.iter().any(|id| inputs.contains(id)) {
                                ready = false;
                                break;
                            }
                        }
                    }
                    if !ready {
                        // Preserve inference from independent formed domains.
                        // A missing clause argument is not yet an expected type
                        // against which to freeze an explicit input.
                        continue;
                    }
                    let early_domains = self.prepare_open_comptime_prefix(
                        expr,
                        owner,
                        sig,
                        &preparation,
                        &routes,
                        slot,
                        &prefix,
                        &mut checked.values,
                        &mut checked.symbols,
                        &mut checked.live,
                        &mut checked.pending,
                    )?;
                    for (slot, ty) in early_domains {
                        definition.parameters[slot].1 = Annotation::Known(ty.clone());
                        formed.insert(slot, CompletedType::Identity(ty));
                    }
                    if checked.symbols[slot].is_some() || checked.live[slot].is_some() {
                        continue;
                    }
                }
                let (value, symbol) = if let Some(route) =
                    routes.iter().find(|route| route.parameter == slot)
                {
                    route.require_collection_type(self, &domain, expr.span)?;
                    let input = route.expression(expr);
                    let value =
                        self.with_staged_input(true, |this| this.expr(&input, Some(&domain)))?;
                    self.freeze_open_call_input(&input, value)?
                } else {
                    if (0..slot).any(|earlier| {
                        !assigned[earlier]
                            && (sig.modifiers.is_comptime
                                || definition.parameters[earlier].0 != Sort::Runtime)
                            && checked.symbols[earlier].is_none()
                            && checked.live[earlier].is_none()
                    }) {
                        // A later default cannot overtake an earlier default
                        // waiting for declaration arguments, even if unrelated.
                        continue;
                    }
                    // Defaults are checked in the declaration's input scope.
                    // Do not fabricate domains for an unfinished prefix.
                    let mut prefix = Vec::new();
                    for (sort, annotation) in definition.parameters.iter().take(slot + 1) {
                        let ty = if let Annotation::Known(ty) = annotation {
                            match ty.substitute_partial(&checked.symbols) {
                                Ok(ty) => Some(
                                    self.instantiate_input_types(&ty, &all, &template, expr.span)?,
                                ),
                                Err(nir::TypeError::MissingArgument(_)) => None,
                                Err(e) => return Err(error(expr.span, e.to_string())),
                            }
                        } else {
                            None
                        };
                        let sort = if *sort != Sort::Runtime && ty.as_ref() == Some(&p(P::Type)) {
                            Sort::Type
                        } else {
                            *sort
                        };
                        prefix.push((sort, ty));
                    }
                    if sig.modifiers.is_comptime {
                        if let Some(parameters) = prefix
                            .iter()
                            .map(|(sort, ty)| ty.clone().map(|ty| Parameter { sort: *sort, ty }))
                            .collect::<Option<Vec<_>>>()
                        {
                            let Some((completed, ty)) = self.prepare_open_live_default(
                                owner,
                                slot,
                                &parameters,
                                &checked.symbols,
                                &checked.live,
                                &mut checked.live_writes,
                                expr.span,
                                &all,
                            )?
                            else {
                                continue;
                            };
                            if self
                                .optional_normal_term(completed.clone(), expr.span)?
                                .is_none()
                            {
                                checked.values[slot] = Some(self.materialize_local_constant(
                                    completed.clone(),
                                    ty,
                                    expr.span,
                                )?);
                                checked.live[slot] = Some(completed);
                            } else {
                                let (value, symbol) =
                                    self.freeze_completed_dependency(completed, ty, expr.span)?;
                                checked.values[slot] = Some(value);
                                checked.symbols[slot] = Some(symbol);
                            }
                            continue;
                        }
                    }
                    let (value, symbol) = self.prepare_open_default_in_scope(
                        owner,
                        slot,
                        &prefix,
                        &checked.symbols,
                        &mut checked.default_writes,
                        true,
                        expr.span,
                    )?;
                    (
                        value,
                        symbol.ok_or_else(|| {
                            error(
                                expr.span,
                                "staged default did not produce an immutable symbol",
                            )
                        })?,
                    )
                };
                symbol
                    .check_scope(&[])
                    .map_err(|e| error(expr.span, e.to_string()))?;
                checked.values[slot] = Some(value);
                checked.symbols[slot] = Some(symbol);
            }
        }
        // Project only already formed, ordinary-input-independent domains into
        // this inference problem. Omitted domains are not placeholder types.
        let mut slots = Vec::new();
        let mut parameters = Vec::new();
        for (slot, (sort, annotation)) in definition.parameters.iter().enumerate() {
            let Annotation::Known(ty) = annotation else {
                continue;
            };
            let ty = match ty.substitute_partial(&checked.symbols) {
                Ok(ty) => ty,
                Err(nir::TypeError::MissingArgument(_)) => continue,
                Err(e) => return Err(error(expr.span, e.to_string())),
            };
            if !self.term_substitution_inputs(&ty, expr.span)?.is_empty() {
                continue;
            }
            slots.push(slot);
            parameters.push(Parameter {
                sort: *sort,
                ty: self.instantiate_input_types(&ty, &all, &template, expr.span)?,
            });
        }
        let result = if let Annotation::Known(ty) = &definition.result {
            match ty.substitute_partial(&checked.symbols) {
                Ok(ty) => Some(self.instantiate_input_types(&ty, &all, &template, expr.span)?),
                Err(nir::TypeError::MissingArgument(_)) => None,
                Err(e) => return Err(error(expr.span, e.to_string())),
            }
        } else {
            None
        };
        let has_result = result.is_some();
        let result_context = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id && has_result)
            .map(|(_, context)| context.clone());
        let domains: Vec<_> = parameters
            .iter()
            .map(|parameter| parameter.ty.clone())
            .collect();
        let target = result.as_ref().and(expected);
        // Void is an unused inference result when the real result is pending;
        // this signature is never published as a callable or runtime IR type.
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&generic)
                .iter()
                .map(|input| input.sort)
                .collect(),
            type_parameters: generic.len(),
            signature: Term::Function {
                parameters,
                result: Box::new(result.unwrap_or_else(|| p(P::Void))),
            }
            .abstract_parameters(&self.input_binders(&generic))
            .map_err(|e| error(expr.span, e.to_string()))?,
        };
        let scope = self.call_inference_domains();
        let mut actual = vec![None; slots.len()];
        for (index, slot) in slots.iter().enumerate() {
            if assigned[*slot] {
                continue;
            }
            let Some(ty) = definition.default_types.get(slot) else {
                continue;
            };
            let ty = match ty.substitute_partial(&checked.symbols) {
                Ok(ty) => ty,
                Err(nir::TypeError::MissingArgument(_)) => continue,
                Err(e) => return Err(error(expr.span, e.to_string())),
            };
            let ty = self.instantiate_input_types(&ty, &all, &template, expr.span)?;
            // A default annotated by the same unresolved declaration binder is
            // not independent evidence for that binder. Nor are caller-frame
            // slots or divergence usable as a concrete default domain.
            if !is_never(&ty) {
                let inputs = self.observed_term_inputs(&ty, expr.span)?;
                if !generic
                    .iter()
                    .any(|id| inputs.declarations.contains_key(id))
                    && inputs.parameters.is_empty()
                {
                    actual[index] = Some(ty);
                }
            }
        }
        let mut visited = BTreeSet::new();
        let mut property_values = BTreeMap::<usize, Vec<Option<n::Expr>>>::new();
        for (index, slot) in slots.iter().enumerate() {
            if let Some(value) = &checked.values[*slot] {
                actual[index] = Some(value.ty.clone());
                visited.insert(*slot);
            }
        }
        let mut unknown = BTreeSet::new();
        let mut defaults = BTreeMap::new();
        let mut allow_numeric_defaults = false;
        loop {
            let input_domains = generic
                .iter()
                .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                .collect::<Vec<_>>();
            let mut candidate = scheme
                .infer_declared_sparse_types_mode(
                    &generic,
                    &input_domains,
                    &actual,
                    target,
                    None,
                    &self.context,
                    crate::ptc::Budget::default(),
                    self.active_effect_environment().is_some()
                        && !matches!(target, Some(Term::Computation { .. })),
                    &scope,
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            if let Some(context) = &result_context {
                candidate
                    .constrain_result_context(context, crate::ptc::Budget::default())
                    .map_err(|e| call_inference_error(expr.span, e))?;
            }
            for (slot, values) in &property_values {
                if values.is_empty() {
                    continue;
                }
                let Some(index) = slots.iter().position(|index| index == slot) else {
                    continue;
                };
                let route = routes
                    .iter()
                    .find(|route| route.parameter == *slot)
                    .ok_or_else(|| {
                        error(expr.span, "missing route for property collection parameter")
                    })?;
                let context = route.retained_collection_context(
                    self,
                    values,
                    &candidate
                        .parameter_input_context(index)
                        .map_err(|e| call_inference_error(expr.span, e))?,
                );
                candidate
                    .constrain_parameter_context(index, &context, crate::ptc::Budget::default())
                    .map_err(|e| call_inference_error(expr.span, e))?;
            }
            // Prepared packet fields provide partial evidence before sibling
            // callbacks can be checked against a completed collection domain.
            for route in &routes {
                let Some(index) = slots.iter().position(|slot| *slot == route.parameter) else {
                    continue;
                };
                if let Some(context) = route.local_collection_context(self) {
                    candidate
                        .constrain_parameter_context(index, &context, crate::ptc::Budget::default())
                        .map_err(|e| call_inference_error(expr.span, e))?;
                }
            }
            // An associated equality can await a later input's subject type.
            // Keep that obligation pending while collecting evidence, then
            // resolve established projections before finishing the candidate.
            candidate
                .refine_type_views_metered(
                    |ty, budget| self.callable_input_view_metered(ty, budget),
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            self.settle_function_defaults(
                owner,
                &generic,
                &template,
                &pending,
                &mut defaults,
                &mut candidate,
                expr.span,
            )?;
            let complete = (0..generic.len())
                .map(|index| candidate.type_argument(index))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| call_inference_error(expr.span, e))?
                .iter()
                .all(Option::is_some);
            if complete
                && property_values
                    .keys()
                    .all(|slot| checked.values[*slot].is_some())
            {
                let instance = candidate
                    .finish()
                    .map_err(|e| call_inference_error(expr.span, e))?;
                let arguments = template
                    .iter()
                    .map(|ty| {
                        self.instantiate_input_types(
                            ty,
                            &generic,
                            &instance.type_arguments,
                            expr.span,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(OpenClauseInference::Ready(arguments, checked, formed));
            }
            let mut progress = false;
            for route in &routes {
                if visited.contains(&route.parameter) {
                    continue;
                }
                let Some(index) = slots.iter().position(|slot| *slot == route.parameter) else {
                    continue;
                };
                let hint = candidate
                    .parameter_type(index)
                    .map_err(|e| call_inference_error(expr.span, e))?;
                let partial = candidate
                    .parameter_callable_context(index)
                    .map_err(|e| call_inference_error(expr.span, e))?;
                let context = candidate
                    .parameter_input_context(index)
                    .map_err(|e| call_inference_error(expr.span, e))?;
                if let Some(ty) = &hint {
                    route.require_collection_type(self, ty, expr.span)?;
                }
                if checked.pending.contains(route.parameter) {
                    let evidence = match hint {
                        Some(known) => Some(known),
                        None => checked.pending.actual_domain(
                            self,
                            route,
                            expr.span,
                            allow_numeric_defaults,
                        )?,
                    };
                    let Some(evidence) = evidence else { continue };
                    if !is_never(&evidence) {
                        actual[index] = Some(evidence);
                    }
                    visited.insert(route.parameter);
                    progress = true;
                    allow_numeric_defaults = false;
                    break;
                }
                let input = route.expression(expr);
                let staged = definition.parameters[route.parameter].0 != Sort::Runtime;
                if route.property_fields().is_some() && !staged {
                    let saved_returns = if description
                        .as_ref()
                        .is_some_and(|(_, returns, _)| matches!(returns, Some(D::Unknown)))
                        && self.control.result.is_none()
                    {
                        Some(
                            self.control
                                .return_observations
                                .replace(ReturnObservations::Deferred),
                        )
                    } else {
                        None
                    };
                    let attempt = self.attempt_collection_fields(
                        expr,
                        route,
                        &context,
                        property_values.entry(route.parameter).or_default(),
                        false,
                        allow_numeric_defaults,
                    );
                    if let Some(saved) = saved_returns {
                        self.control.return_observations = saved;
                    }
                    let (value, advanced) = attempt?;
                    if let Some(value) = value {
                        self.check_open_fragment(&value, expr.span, false)?;
                        if !is_never(&value.ty) {
                            actual[index] = Some(value.ty.clone());
                        }
                        checked.values[route.parameter] = Some(value);
                        visited.insert(route.parameter);
                    }
                    if advanced {
                        progress = true;
                        allow_numeric_defaults = false;
                        break;
                    }
                    continue;
                }
                if let Some((locals, returns, depth)) = description.as_mut() {
                    let hint = hint.map(D::Known);
                    let mut attempt_locals = (**locals).clone();
                    let saved_defaults =
                        std::mem::replace(&mut self.numeric_defaults, allow_numeric_defaults);
                    let attempt = self.with_staged_input(staged, |this| {
                        this.describe_open_expression(
                            &input,
                            &mut attempt_locals,
                            *returns,
                            hint.as_ref(),
                            *depth + 1,
                        )
                    });
                    self.numeric_defaults = saved_defaults;
                    let value = match attempt {
                        Ok(value) => value,
                        Err(failure) if failure.needs_context => continue,
                        Err(failure) => return Err(failure),
                    };
                    **locals = attempt_locals;
                    match value {
                        D::Known(ty) if !is_never(&ty) => actual[index] = Some(ty),
                        D::Unknown => {
                            unknown.insert(index);
                        }
                        _ => {}
                    }
                    visited.insert(route.parameter);
                    allow_numeric_defaults = false;
                    progress = true;
                    break;
                }
                if let Some(value) = self.with_staged_input(staged, |this| {
                    this.argument_attempt_with_input(
                        &input,
                        hint.as_ref(),
                        partial.as_ref(),
                        allow_numeric_defaults,
                        Some(&context),
                    )
                })? {
                    if !is_never(&value.ty) {
                        actual[index] = Some(value.ty.clone());
                    }
                    checked.values[route.parameter] = Some(value);
                    visited.insert(route.parameter);
                    allow_numeric_defaults = false;
                    progress = true;
                    break;
                }
            }
            if !progress {
                if self.numeric_defaults && !allow_numeric_defaults {
                    allow_numeric_defaults = true;
                    continue;
                }
                if description.is_some()
                    && !unknown.is_empty()
                    && self.open_definition_obligations(owner).is_empty()
                {
                    let mut unknown_inputs = BTreeSet::new();
                    for slot in &unknown {
                        unknown_inputs
                            .extend(self.term_declaration_inputs(&domains[*slot], expr.span)?);
                    }
                    let mut supported = BTreeSet::new();
                    for (index, id) in generic.iter().enumerate() {
                        if candidate
                            .type_argument(index)
                            .map_err(|e| call_inference_error(expr.span, e))?
                            .is_some()
                            || unknown_inputs.contains(id)
                        {
                            supported.insert(*id);
                        }
                    }
                    // Defaults can wait on supported inputs, but cannot seed
                    // unrelated phantom parameters or dependency cycles.
                    loop {
                        let mut added = false;
                        for clause in &pending {
                            self.required_steps = self
                                .required_steps
                                .checked_sub(1 + clause.dependencies.len() as u64)
                                .ok_or_else(|| {
                                    error(expr.span, "generic default dependency budget exhausted")
                                })?;
                            if clause.dependencies.iter().all(|id| supported.contains(id)) {
                                added |= supported.insert(clause.binder);
                            }
                        }
                        if !added {
                            break;
                        }
                    }
                    if generic.iter().all(|id| supported.contains(id)) {
                        let Some((locals, returns, depth)) = description.as_mut() else {
                            return Err(error(
                                expr.span,
                                "missing open call description state for suspended generic call",
                            ));
                        };
                        // Inference omits pending domains; ordinary checking
                        // must still inspect every operand before suspending.
                        for route in &routes {
                            if visited.contains(&route.parameter) {
                                continue;
                            }
                            let input = route.expression(expr);
                            let staged = definition.parameters[route.parameter].0 != Sort::Runtime;
                            self.with_staged_input(staged, |this| {
                                this.describe_open_expression(
                                    &input,
                                    locals,
                                    *returns,
                                    None,
                                    *depth + 1,
                                )
                            })?;
                        }
                        let result = if has_result {
                            candidate
                                .result_type()
                                .map_err(|e| call_inference_error(expr.span, e))?
                                .map_or(D::Unknown, D::Known)
                        } else {
                            D::Unknown
                        };
                        return Ok(OpenClauseInference::Pending(result));
                    }
                }
                return Err(context_error(
                    expr.span,
                    "open declaration parameters lack inference evidence from formed domains; supply explicit arguments",
                ));
            }
        }
    }

    fn freeze_open_call_input(
        &mut self,
        source: &Expr<'_>,
        value: n::Expr,
    ) -> Result<(n::Expr, Term), Error> {
        if let K::Local(binding) = value.kind {
            if let Some(symbol) = self.spread_input_symbol(binding, &value.ty, source)? {
                // Keep the local backed by its enclosing packet preparation;
                // only the immutable symbol participates in type formation.
                return Ok((value, symbol));
            }
        }
        self.freeze_dependency_input(source, value, vec![])
    }

    /// Evaluate explicit clauses once, then share their substitution across
    /// packet context formation, input preparation and the concrete call.
    pub(super) fn with_explicit_open_clauses<R>(
        &mut self,
        owner: OwnerId,
        clauses: &[Arg<'_>],
        expr: &Expr<'_>,
        build: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        self.check_open_definition(owner)?;
        if self.type_parameters[&owner].is_empty() {
            return Err(error(
                expr.span,
                "explicit instantiation requires a generic function",
            ));
        }
        let arguments = self.function_clause_arguments(expr, owner, clauses)?;
        self.check_type_application(owner, &arguments, expr.span)?;
        let saved = self.alias_substitution.clone();
        for (binder, argument) in self.declaration_binders(owner).into_iter().zip(arguments) {
            self.alias_substitution
                .insert(binder.declaration, Arc::new(argument));
        }
        let result = build(self);
        self.alias_substitution = saved;
        result
    }

    pub(super) fn instantiated_open_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.instantiated_open_call_with_prefix(expr, owner, args, Vec::new(), expected)
    }

    pub(super) fn instantiated_open_call_with_prefix(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if prefix.is_empty() && (self.describing_open_call || self.checking_open_behavior) {
            let fixed = self
                .declaration_binders(owner)
                .iter()
                .map(|binder| self.alias_substitution.get(&binder.declaration).cloned())
                .collect::<Option<Vec<_>>>();
            if let Some(fixed) = fixed {
                if let Some(value) = self.ordinary_open_behavior_call(
                    expr,
                    owner,
                    None,
                    Some(fixed),
                    args,
                    expected,
                )? {
                    return Ok(value);
                }
            }
        }
        self.open_function_call_with_prefix(
            expr,
            owner,
            args,
            prefix,
            expected,
            StagedInputs::new(self.open_functions[&owner].parameters.len()),
            BTreeMap::new(),
        )
    }

    fn ordinary_open_behavior_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        clauses: Option<&[Arg<'_>]>,
        fixed: Option<Vec<Term>>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<Option<n::Expr>, Error> {
        if !self.checking_open_behavior && !self.describing_open_call {
            return Ok(None);
        }
        let is_pure_item = matches!(
            &self.open_item(owner, expr.span)?.kind,
            ItemKind::Fn(sig, _) if sig.modifiers.is_pure
        );
        let is_comptime_item = matches!(
            &self.open_item(owner, expr.span)?.kind,
            ItemKind::Fn(sig, _) if sig.modifiers.is_comptime
        );
        if !self.describing_open_call && !self.checking_default_behavior && !is_pure_item {
            return Ok(None);
        }
        if self.describing_open_call && !self.checking_default_behavior && is_comptime_item {
            return Ok(None);
        }
        let Some(signature) = self.declared_signature(owner).map(|sig| sig.into_owned()) else {
            return Ok(None);
        };
        if self.checking_default_behavior && !is_comptime_item {
            self.default_behavior_functions.insert(owner);
            if !self.active_open_definitions.contains(&owner) {
                self.check_open_definition(owner)?;
            }
        }
        // This edge belongs to a definition-side behavior or call description,
        // not an executable instance. Its body is checked separately, including
        // every inline alternative.
        // Keep the declared stages; ordinary input checking can retain symbolic
        // comptime inputs without recursively demanding the callee definition.
        let result = if self.type_parameters[&owner].is_empty() && clauses.is_none() {
            let callee = n::Expr {
                source: source(expr.span),
                ty: self.callable_type(owner, signature),
                kind: K::Function(declaration(self.package, &self.package_identities, owner)),
            };
            self.finish_prepared_call(
                expr,
                callee,
                args,
                vec![],
                expected,
                None,
                CallUse::DefinitionBehavior,
            )
        } else {
            if fixed.is_none()
                && let Some(clauses) = clauses
                && let Some(call) = self.with_partial_named_call_plan(
                    owner,
                    clauses,
                    expr,
                    args,
                    expected,
                    None,
                    |this, plan| {
                        this.generic_call_with_plan(
                            expr,
                            owner,
                            args,
                            expected,
                            None,
                            vec![],
                            plan,
                            None,
                            CallUse::DefinitionBehavior,
                        )
                    },
                )?
            {
                return Ok(Some(call));
            }
            let known = match fixed {
                Some(arguments) => arguments,
                None => match clauses {
                    Some(clauses) => self.function_clause_arguments(expr, owner, clauses)?,
                    None => self.lexical_function_arguments(owner, expr.span)?,
                },
            };
            let plan = self.function_type_plan(owner, &known, expr.span)?;
            self.generic_call_with_plan(
                expr,
                owner,
                args,
                expected,
                None,
                vec![],
                plan,
                None,
                CallUse::DefinitionBehavior,
            )
        };
        result.map(Some)
    }

    pub(super) fn explicit_open_function_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        clauses: Option<&[Arg<'_>]>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if let Some(call) =
            self.ordinary_open_behavior_call(expr, owner, clauses, None, args, expected)?
        {
            return Ok(call);
        }
        if let Some(clauses) = clauses {
            if let Some(call) = self.with_partial_named_call_plan(
                owner,
                clauses,
                expr,
                args,
                expected,
                None,
                |this, plan| {
                    this.check_open_definition(owner)?;
                    let inferred =
                        this.infer_open_clauses_with_plan(expr, owner, args, expected, None, plan)?;
                    this.finish_inferred_open_call(expr, owner, args, expected, inferred)
                },
            )? {
                return Ok(call);
            }
            return self.with_explicit_open_clauses(owner, clauses, expr, |this| {
                this.instantiated_open_call(expr, owner, args, expected)
            });
        }
        self.check_open_definition(owner)?;
        if self.type_parameters[&owner].is_empty() {
            return self.open_function_call(
                expr,
                owner,
                args,
                expected,
                StagedInputs::new(self.open_functions[&owner].parameters.len()),
                BTreeMap::new(),
            );
        }
        let inferred = self.infer_open_clause_arguments(expr, owner, args, expected, None)?;
        self.finish_inferred_open_call(expr, owner, args, expected, inferred)
    }

    fn finish_inferred_open_call<'src>(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'src>],
        expected: Option<&Term>,
        inferred: OpenClauseInference<'src>,
    ) -> Result<n::Expr, Error> {
        let OpenClauseInference::Ready(arguments, checked, formed) = inferred else {
            return Err(error(
                expr.span,
                "concrete open call cannot suspend clause inference",
            ));
        };
        self.check_type_application(owner, &arguments, expr.span)?;
        let saved = self.alias_substitution.clone();
        for (binder, argument) in self.declaration_binders(owner).into_iter().zip(arguments) {
            self.alias_substitution
                .insert(binder.declaration, Arc::new(argument));
        }
        let result = self.open_function_call(expr, owner, args, expected, checked, formed);
        self.alias_substitution = saved;
        result
    }

    fn open_function_call<'src>(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'src>],
        expected: Option<&Term>,
        checked: StagedInputs<'src>,
        formed: BTreeMap<usize, CompletedType>,
    ) -> Result<n::Expr, Error> {
        self.open_function_call_with_prefix(
            expr,
            owner,
            args,
            Vec::new(),
            expected,
            checked,
            formed,
        )
    }

    fn open_function_call_with_prefix<'src>(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'src>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        checked: StagedInputs<'src>,
        formed: BTreeMap<usize, CompletedType>,
    ) -> Result<n::Expr, Error> {
        let written_types = self.acquire_written_call_types(expr)?;
        self.check_open_definition(owner)?;
        let mut definition = self.open_functions[&owner].clone();
        for (slot, data) in formed {
            let ty = Annotation::from(data);
            if slot == definition.parameters.len() {
                definition.result = ty;
            } else {
                definition.parameters[slot].1 = ty;
            }
        }
        if self
            .declaration_binders(owner)
            .iter()
            .any(|binder| self.alias_substitution.contains_key(&binder.declaration))
        {
            for (slot, annotation) in definition
                .parameters
                .iter_mut()
                .map(|(_, ty)| ty)
                .chain(std::iter::once(&mut definition.result))
                .enumerate()
            {
                *annotation = match &*annotation {
                    Annotation::Known(ty) => {
                        Annotation::Known(self.specialize_open_clause_type(owner, ty, expr.span)?)
                    }
                    Annotation::Pending(_) => Annotation::Suspended { owner, slot },
                    other => other.clone(),
                };
            }
        }
        let sig = self.open_fn_or_case_sig(owner, expr.span)?;
        let offset = prefix.len();
        if offset > sig.params.len() {
            return Err(error(expr.span, "method receiver count mismatch"));
        }
        let (mut routes, tail_assigned) = route_arguments(
            args,
            Some(&sig.params[offset..]),
            sig.params.len() - offset,
            expr.span,
        )?;
        for route in &mut routes {
            route.parameter += offset;
        }
        let mut assigned = vec![true; offset];
        assigned.extend(tail_assigned);
        let mut ordinary_dependencies = BTreeSet::new();
        if sig.modifiers.is_comptime {
            for slot in 0..=sig.params.len() {
                if let Some(source) = annotation_source(sig, slot) {
                    if let H::Defer(value) = source.kind {
                        ordinary_dependencies
                            .extend(self.expression_parameter_dependencies(owner, value)?);
                    }
                }
            }
            for annotation in definition
                .parameters
                .iter()
                .map(|(_, ty)| ty)
                .chain(std::iter::once(&definition.result))
            {
                if let Annotation::Known(ty) = annotation {
                    for term in ty.structural_terms() {
                        if let Term::AssociatedTypeCall(call) = term {
                            for arg in &call.call_arguments {
                                for node in arg.structural_terms() {
                                    if let Term::Bound {
                                        depth: 0, index, ..
                                    } = node
                                    {
                                        ordinary_dependencies.insert(*index);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(ItemKind::Fn(_, body_id) | ItemKind::Case(_, body_id)) =
                self.package.item(owner).map(|item| &item.kind)
                && let Some(body) = self.package.body(*body_id)
            {
                let mut inline_deps = Vec::new();
                self.scan_callable_requirements(body.value, |this, expression| {
                    if let H::InlineControl(value) = expression.kind {
                        let selectors: Vec<_> = match value.kind {
                            H::If(cond, _, _) => vec![cond],
                            H::When(arms) => arms.iter().filter_map(|arm| arm.cond).collect(),
                            H::Match(subject, _) => vec![subject],
                            _ => Vec::new(),
                        };
                        for selector in selectors {
                            inline_deps
                                .extend(this.expression_parameter_dependencies(owner, selector)?);
                        }
                    }
                    Ok(false)
                })?;
                ordinary_dependencies.extend(inline_deps);
            }
        }
        if sig.modifiers.is_comptime {
            // A demanded omitted default can itself read earlier ordinary
            // inputs. Close that dependency chain before declaration-ordered
            // preparation, but never inspect a default explicitly overridden
            // by the caller. Each parameter enters the worklist at most once.
            ordinary_dependencies.extend(
                definition
                    .parameters
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, (sort, _))| (*sort != Sort::Runtime).then_some(slot)),
            );
            // Reaching a demanded default executes every preceding omitted
            // default, even if its value is irrelevant to the signature. Those
            // computations can fail or diverge, so dependency slicing alone
            // must not reorder the observable invocation.
            if let Some(last_default) = ordinary_dependencies
                .iter()
                .copied()
                .filter(|slot| !assigned[*slot])
                .max()
            {
                ordinary_dependencies.extend((0..=last_default).filter(|slot| !assigned[*slot]));
            }
            let mut pending: Vec<_> = ordinary_dependencies.iter().copied().collect();
            while let Some(slot) = pending.pop() {
                if assigned[slot] {
                    continue;
                }
                let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                    continue;
                };
                for input in self.expression_parameter_dependencies(owner, default)? {
                    if ordinary_dependencies.insert(input) {
                        pending.push(input);
                    }
                }
            }
        }
        let independent = self.signature_independent_of_producers(
            &definition,
            sig,
            &routes,
            &ordinary_dependencies,
            expr.span,
        )?;
        let mut symbols = checked.symbols;
        let mut values = checked.values;
        for (slot, value) in prefix.into_iter().enumerate() {
            if values[slot].is_none() {
                values[slot] = Some(value);
            }
        }
        let mut live = checked.live;
        let mut pending_inputs = checked.pending;
        let mut default_writes = checked.default_writes;
        let mut live_writes = checked.live_writes;
        let mut parameters = Vec::new();
        // Type formation is declaration ordered. Runtime evaluation remains in
        // source order in the emitted call argument plan below.
        for slot in 0..definition.parameters.len() {
            let (sort, annotation) = &definition.parameters[slot];
            if annotation.needs_instance() {
                self.admit_open_type_requirements(owner, &symbols, true, expr.span)?;
            }
            let ty = self.instantiate_open_annotation_with_live(
                owner,
                slot,
                annotation,
                sig,
                &symbols,
                &parameters,
                &live,
                expr,
            )?;
            let ty = if let Some(route) = routes.iter().find(|route| route.parameter == slot) {
                if pending_inputs.contains(slot) {
                    self.admit_type_data(ty, expr.span, |this| {
                        pending_inputs
                            .actual_domain(this, route, expr.span, this.numeric_defaults)?
                            .ok_or_else(|| {
                                context_error(
                                    expr.span,
                                    "gradual operand requires a concrete input domain",
                                )
                            })
                    })?
                } else {
                    self.admit_supplied_annotation(
                        ty,
                        route,
                        expr,
                        &mut values[slot],
                        *sort != Sort::Runtime || sig.modifiers.is_comptime,
                    )?
                }
            } else if slot < offset {
                let actual_ty = values[slot]
                    .as_ref()
                    .ok_or_else(|| error(expr.span, "missing prefix receiver argument"))?
                    .ty
                    .clone();
                self.admit_type_data(ty, expr.span, |_| Ok(actual_ty))?
            } else {
                self.admit_type_data(ty, expr.span, |this| {
                    let default = definition.default_types.get(&slot).ok_or_else(|| {
                        error(
                            expr.span,
                            "gradual parameter needs a concrete domain from its default",
                        )
                    })?;
                    let default_ty = this.specialize_open_clause_type(owner, default, expr.span)?;
                    if sig.modifiers.is_comptime {
                        default_ty.substitute_partial_comptime(&symbols)
                    } else {
                        default_ty.substitute_partial(&symbols)
                    }
                    .map_err(|failure| error(expr.span, failure.to_string()))
                })?
            };
            parameters.push(Parameter {
                sort: if *sort != Sort::Runtime && ty == p(P::Type) {
                    Sort::Type
                } else {
                    *sort
                },
                ty: ty.clone(),
            });
            if pending_inputs.contains(slot) {
                let route = routes
                    .iter()
                    .find(|route| route.parameter == slot)
                    .ok_or_else(|| error(expr.span, "missing route for pending open input"))?;
                let (value, symbol, completed) = pending_inputs.deliver(self, route, &ty, expr)?;
                values[slot] = Some(value);
                symbols[slot] = symbol;
                live[slot] = Some(completed);
            }
            if *sort != Sort::Runtime || ordinary_dependencies.contains(&slot) {
                if sig.modifiers.is_comptime && !independent {
                    let early_domains = self.prepare_open_comptime_prefix(
                        expr,
                        owner,
                        sig,
                        &definition,
                        &routes,
                        slot,
                        &parameters,
                        &mut values,
                        &mut symbols,
                        &mut live,
                        &mut pending_inputs,
                    )?;
                    for (slot, ty) in early_domains {
                        definition.parameters[slot].1 = Annotation::Known(ty);
                    }
                }
                if live[slot].is_some() {
                    let value = values[slot].as_ref().ok_or_else(|| {
                        error(expr.span, "missing materialized live open input expression")
                    })?;
                    self.require_expression(&value.ty, &ty, expr.span)?;
                    continue;
                }
                if !assigned[slot] && sig.modifiers.is_comptime {
                    // Inference can finish a default whose result is ordinary
                    // data even when its producer mutates a live closure.
                    if let (Some(value), Some(_)) = (&values[slot], &symbols[slot]) {
                        self.require_expression(&value.ty, &ty, expr.span)?;
                        continue;
                    }
                    let (completed, domain) = self
                        .prepare_open_live_default(
                            owner,
                            slot,
                            &parameters,
                            &symbols,
                            &live,
                            &mut live_writes,
                            expr.span,
                            &[],
                        )?
                        .ok_or_else(|| {
                            error(
                                expr.span,
                                "final invocation is missing completed declaration default",
                            )
                        })?;
                    if self
                        .optional_normal_term(completed.clone(), expr.span)?
                        .is_none()
                    {
                        values[slot] = Some(self.materialize_local_constant(
                            completed.clone(),
                            domain,
                            expr.span,
                        )?);
                        live[slot] = Some(completed);
                    } else {
                        let (value, symbol) =
                            self.freeze_completed_dependency(completed, domain, expr.span)?;
                        values[slot] = Some(value);
                        symbols[slot] = Some(symbol);
                    }
                    continue;
                }
                let (value, symbol) = if let (Some(value), Some(symbol)) =
                    (&values[slot], &symbols[slot])
                {
                    self.require_expression(&value.ty, &ty, expr.span)?;
                    (value.clone(), symbol.clone())
                } else if let Some(route) = routes.iter().find(|route| route.parameter == slot) {
                    route.require_collection_type(self, &ty, expr.span)?;
                    let input = route.expression(expr);
                    let checked = if let Some(value) = values[slot].take() {
                        self.require_expression(&value.ty, &ty, expr.span)?;
                        value
                    } else {
                        self.with_staged_input(true, |this| this.expr(&input, Some(&ty)))?
                    };
                    if let Some(symbol) = symbols[slot].clone() {
                        (checked, symbol)
                    } else {
                        self.freeze_open_call_input(&input, checked)?
                    }
                } else if slot < offset {
                    let checked = values[slot]
                        .take()
                        .ok_or_else(|| error(expr.span, "missing prefix staged argument"))?;
                    self.require_expression(&checked.ty, &ty, expr.span)?;
                    self.freeze_open_call_input(expr, checked)?
                } else {
                    let (value, symbol) = self.prepare_open_default(
                        owner,
                        slot,
                        &parameters,
                        &symbols,
                        &mut default_writes,
                        true,
                        expr.span,
                    )?;
                    (
                        value,
                        symbol.ok_or_else(|| {
                            error(
                                expr.span,
                                "staged default did not produce an immutable symbol",
                            )
                        })?,
                    )
                };
                symbol.check_scope(&[]).map_err(|failure| error(expr.span,
                    format!("open signature invocation requires completed comptime inputs: {failure}")))?;
                symbols[slot] = Some(symbol);
                values[slot] = Some(value);
            }
        }
        self.admit_open_type_requirements(owner, &symbols, false, expr.span)?;
        let result = self.instantiate_open_annotation_with_live(
            owner,
            sig.params.len(),
            &definition.result,
            sig,
            &symbols,
            &parameters,
            &live,
            expr,
        )?;
        let result = self.admit_type_data(result, expr.span, |_| expected.cloned().ok_or_else(|| context_error(
                expr.span, "gradual result requires a concrete delivery domain; no result context was supplied",
            )))?;
        let result = self.observe_call_type(&result, expr)?;
        let signature = Term::Function {
            parameters: parameters.clone(),
            result: Box::new(result.clone()),
        };
        let mut residuals = self.prepare_open_live_suffix(
            owner,
            sig,
            &parameters,
            &symbols,
            &mut live_writes,
            assigned.iter().enumerate().filter_map(|(slot, &assigned)| {
                (!assigned && values[slot].is_none()).then_some(slot)
            }),
            expr.span,
        )?;
        let mut live_packets = BTreeSet::new();
        for (slot, value) in live.iter_mut().enumerate() {
            if value.is_none() {
                continue;
            }
            if let Some(expression) = &values[slot] {
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let free = nir::free_bindings::collect(expression, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                for binding in free.map_err(|message| error(expr.span, message))? {
                    if let Some(key) = self.completed_packet_key(binding) {
                        live_packets.insert(key);
                    }
                }
            }
            if let Some(n::Expr {
                kind: K::Local(binding),
                ..
            }) = &values[slot]
            {
                if let Some(key) = self.completed_packet_key(*binding) {
                    live_packets.insert(key);
                    // The packet owns this graph, including undemanded aliases.
                    // Keep its field binding and refresh the entire packet.
                    *value = None;
                }
            }
        }
        let live_slots: Vec<_> = live
            .iter()
            .enumerate()
            .filter_map(|(slot, value)| value.as_ref().map(|_| slot))
            .collect();
        let live_inputs = live_slots
            .iter()
            .map(|slot| {
                let value = live[*slot]
                    .take()
                    .ok_or_else(|| error(expr.span, "missing live open input value"))?;
                Ok((value, parameters[*slot].ty.clone()))
            })
            .chain(
                live_packets
                    .iter()
                    .map(|key| Ok(self.completed_packet_graph(*key))),
            )
            .chain(
                live_writes
                    .values()
                    .map(|(ty, value)| Ok((value.clone(), ty.clone()))),
            )
            .collect::<Result<Vec<_>, Error>>()?;
        let (live_bindings, completed) = self.materialize_call_values(live_inputs, expr.span)?;
        let mut completed = completed.into_iter();
        for slot in live_slots {
            values[slot] =
                Some(completed.next().ok_or_else(|| {
                    error(expr.span, "materialized live call input count mismatch")
                })?);
        }
        for key in live_packets {
            self.set_completed_packet_expression(
                key,
                completed.next().ok_or_else(|| {
                    error(expr.span, "materialized live packet input count mismatch")
                })?,
            );
        }
        let frame_writes: BTreeMap<_, _> = live_writes.into_keys().zip(completed).collect();
        let mut arguments = Vec::new();
        for slot in 0..offset {
            let value = values[slot]
                .take()
                .ok_or_else(|| error(expr.span, "missing prefix argument expression"))?;
            self.require_expression(&value.ty, &parameters[slot].ty, expr.span)?;
            arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter: slot,
                value,
                is_default: false,
            });
        }
        for route in &routes {
            let slot = route.parameter;
            route.require_collection_type(self, &parameters[slot].ty, expr.span)?;
            let value = if let Some(value) = values[slot].take() {
                self.require_expression(&value.ty, &parameters[slot].ty, expr.span)?;
                value
            } else {
                let input = route.expression(expr);
                self.expr(&input, Some(&parameters[slot].ty))?
            };
            arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter: slot,
                value,
                is_default: false,
            });
        }
        let mut pending_defaults = Vec::new();
        for (slot, assigned) in assigned.iter().enumerate() {
            if *assigned {
                continue;
            }
            if let Some(value) = values[slot].take() {
                // This default already executed. Deliver its caller-owned value
                // before entering the frame for the remaining runtime defaults.
                arguments.push(n::CallArgument {
                    passing: Default::default(),
                    parameter: slot,
                    value,
                    is_default: false,
                });
            } else {
                let value = match residuals.remove(&slot) {
                    Some(value) => value,
                    None => {
                        self.prepare_open_default(
                            owner,
                            slot,
                            &parameters,
                            &symbols,
                            &mut default_writes,
                            false,
                            expr.span,
                        )?
                        .0
                    }
                };
                pending_defaults.push(n::CallArgument {
                    passing: Default::default(),
                    parameter: slot,
                    value,
                    is_default: true,
                });
            }
        }
        arguments.extend(pending_defaults);
        let result = if arguments
            .iter()
            .any(|argument| is_never(&argument.value.ty))
        {
            p(P::Never)
        } else {
            result
        };
        if let Some(expected) = expected {
            self.require_expression(&result, expected, expr.span)?;
        }
        let callee = self.open_instance_value(owner, &symbols, signature, expr.span)?;
        let generated_id = match &callee.kind {
            K::Function(id)
            | K::FunctionInstance {
                declaration: id, ..
            } => Some(*id),
            _ => None,
        };
        let env_params = generated_id
            .and_then(|id| self.generated_functions.declaration(id))
            .map(|decl| decl.env_parameters.clone())
            .unwrap_or_default();
        let inputs = env_params
            .iter()
            .map(|env| {
                let term = env.runtime_id.map(Term::Runtime).unwrap_or(Term::Rigid {
                    declaration: env.declaration,
                    sort: Sort::Runtime,
                });
                (term, env.ty.clone())
            })
            .collect();
        let mut call = n::Expr {
            source: source(expr.span),
            ty: result,
            kind: K::Call {
                callee: Box::new(callee),
                arguments,
                env_arguments: Vec::new(),
                parameter_bindings: sig
                    .params
                    .iter()
                    .map(|parameter| local(parameter.hir_id))
                    .collect(),
            },
        };
        let environment =
            self.prepare_call_environment(&mut call, inputs, written_types, 0, &routes, expr)?;
        let mut call = environment.finish(self, call, &routes, 0, expr)?;
        if !frame_writes.is_empty() {
            self.resume_open_default_frame(&mut call, frame_writes, expr.span)?;
        }
        if live_bindings.is_empty() {
            return Ok(call);
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty: call.ty.clone(),
            kind: K::Block {
                statements: live_bindings,
                result: Some(Box::new(call)),
            },
        })
    }

    fn collect_term_runtime_ids(
        term: &Term,
        pending: &mut Vec<nir::RuntimeId>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let deps = term.runtime_dependencies();
        *steps = steps
            .checked_sub(deps.len() as u64)
            .ok_or_else(|| error(span, "open capture runtime dependency budget exhausted"))?;
        pending.extend(deps);
        Ok(())
    }

    fn open_instance_captures(
        &mut self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        signature: &Term,
        span: Span,
    ) -> Result<Vec<(nir::DeclarationParameter, Term)>, Error> {
        let mut steps = self.required_steps;
        let result = (|| {
            let mut pending = Vec::new();
            let mut pending_runtimes = Vec::new();
            let mut seen_runtimes = BTreeSet::new();
            let clauses = self
                .declaration_binders(owner)
                .into_iter()
                .filter_map(|binder| self.alias_substitution.get(&binder.declaration));
            for value in clauses
                .chain(symbols.iter().flatten())
                .chain(std::iter::once(signature))
            {
                pending
                    .extend(input_observation::term_inputs(value, &mut steps, span)?.declarations);
                Self::collect_term_runtime_ids(value, &mut pending_runtimes, &mut steps, span)?;
            }
            let mut captures = BTreeMap::new();
            while !pending.is_empty() || !pending_runtimes.is_empty() {
                if let Some(rt_id) = pending_runtimes.pop() {
                    steps = steps
                        .checked_sub(1)
                        .ok_or_else(|| error(span, "open capture domain budget exhausted"))?;
                    if seen_runtimes.insert(rt_id)
                        && let Some(domain) = self.runtime_domains.get(&rt_id)
                    {
                        pending.extend(
                            input_observation::term_inputs(domain, &mut steps, span)?.declarations,
                        );
                        Self::collect_term_runtime_ids(
                            domain,
                            &mut pending_runtimes,
                            &mut steps,
                            span,
                        )?;
                    }
                    continue;
                }
                let Some((declaration, sort)) = pending.pop() else {
                    return Err(error(span, "missing pending open capture entry"));
                };
                steps = steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "open capture domain budget exhausted"))?;
                if let Some((previous, _)) = captures.get(&declaration) {
                    if *previous != sort {
                        return Err(error(
                            span,
                            "open capture has inconsistent declaration sorts",
                        ));
                    }
                    continue;
                }
                let domain = match sort {
                    Sort::Type => p(P::Type),
                    Sort::ComptimeValue => {
                        self.trait_input_domain(declaration).ok_or_else(|| {
                            error(span, "open value capture lacks declaration domain evidence")
                        })?
                    }
                    Sort::Runtime => self
                        .active_env_parameters
                        .get(&declaration)
                        .map(|(_, inner)| Term::Lift(Box::new(inner.clone())))
                        .or_else(|| self.trait_input_domain(declaration))
                        .ok_or_else(|| {
                            error(
                                span,
                                "open runtime capture lacks declaration domain evidence",
                            )
                        })?,
                };
                let inputs = input_observation::term_inputs(&domain, &mut steps, span)?;
                domain
                    .check_scope(&[])
                    .map_err(|failure| error(span, failure.to_string()))?;
                if domain.sort() != Sort::Type
                    || (sort == Sort::ComptimeValue && domain == p(P::Type))
                    || (sort == Sort::Runtime && !matches!(domain, Term::Lift(_)))
                {
                    return Err(error(
                        span,
                        "open value capture has an incompatible declaration domain",
                    ));
                }
                pending.extend(inputs.declarations);
                Self::collect_term_runtime_ids(&domain, &mut pending_runtimes, &mut steps, span)?;
                captures.insert(declaration, (sort, domain));
            }
            Ok(captures
                .into_iter()
                .map(|(declaration, (sort, domain))| {
                    (nir::DeclarationParameter { declaration, sort }, domain)
                })
                .collect())
        })();
        self.required_steps = steps;
        result
    }

    fn open_instance_local_runtimes(
        &mut self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        signature: &Term,
        captures: &[(nir::DeclarationParameter, Term)],
        span: Span,
    ) -> Result<BTreeMap<nir::RuntimeId, (DeclarationId, Term)>, Error> {
        let mut steps = self.required_steps;
        let collected = (|| {
            let mut pending = Vec::new();
            let binders = self.declaration_binders(owner);
            for value in binders
                .iter()
                .filter_map(|binder| self.alias_substitution.get(&binder.declaration))
                .chain(symbols.iter().flatten())
                .chain(std::iter::once(signature))
                .chain(captures.iter().map(|(_, domain)| domain))
            {
                Self::collect_term_runtime_ids(value, &mut pending, &mut steps, span)?;
            }
            let mut seen = BTreeSet::new();
            while let Some(rt_id) = pending.pop() {
                steps = steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "open local runtime dependency budget exhausted"))?;
                if seen.insert(rt_id)
                    && let Some(domain) = self.runtime_domains.get(&rt_id)
                {
                    Self::collect_term_runtime_ids(domain, &mut pending, &mut steps, span)?;
                }
            }
            Ok::<_, Error>((seen, binders))
        })();
        self.required_steps = steps;
        let (seen, binders) = collected?;
        let mut used_decls: BTreeSet<DeclarationId> = captures
            .iter()
            .filter(|(parameter, _)| parameter.sort == Sort::Runtime)
            .map(|(parameter, _)| parameter.declaration)
            .collect();
        let mut local_runtime_ids = BTreeMap::new();
        for rt_id in seen {
            if self.global_runtime_deliveries.contains_key(&rt_id) {
                continue;
            }
            let Some(Term::Lift(inner)) = self.runtime_domains.get(&rt_id).cloned() else {
                return Err(error(
                    span,
                    "caller local runtime dependency domain is missing",
                ));
            };
            let matched_binder = binders.iter().find_map(|binder| {
                (binder.sort == Sort::Runtime
                    && matches!(
                        self.alias_substitution.get(&binder.declaration),
                        Some(Term::Runtime(id)) if *id == rt_id
                    )
                    && !used_decls.contains(&binder.declaration))
                .then_some(binder.declaration)
            });
            let env_decl = if let Some(decl) = matched_binder {
                used_decls.insert(decl);
                decl
            } else {
                let fresh = self.generated_functions.fresh_identity(span)?;
                used_decls.insert(fresh);
                fresh
            };
            local_runtime_ids.insert(rt_id, (env_decl, *inner));
        }
        Ok(local_runtime_ids)
    }

    fn open_instance_value(
        &mut self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        signature: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let captures = self.open_instance_captures(owner, symbols, &signature, span)?;
        let ty = self.callable_type(owner, signature.clone());
        let id = self.instantiate_open_function(owner, symbols, signature, &captures, span)?;
        let kind = if captures.is_empty() {
            K::Function(id)
        } else {
            K::FunctionInstance {
                declaration: id,
                arguments: captures
                    .into_iter()
                    .map(|(binder, _)| Term::Rigid {
                        declaration: binder.declaration,
                        sort: binder.sort,
                    })
                    .collect(),
            }
        };
        Ok(n::Expr {
            source: source(span),
            ty,
            kind,
        })
    }

    pub(super) fn specialize_open_trait_method_value(
        &mut self,
        owner: OwnerId,
        clause_arguments: Vec<Term>,
        span: Span,
    ) -> Result<(n::Expr, Vec<n::LocalId>), Error> {
        self.check_open_definition(owner)?;
        self.check_type_application(owner, &clause_arguments, span)?;
        let saved = self.alias_substitution.clone();
        for (binder, argument) in self
            .declaration_binders(owner)
            .into_iter()
            .zip(clause_arguments)
        {
            self.alias_substitution
                .insert(binder.declaration, Arc::new(argument));
        }
        let result = (|| {
            let sig = self.open_fn_or_case_sig(owner, span)?;
            let definition = self.open_functions[&owner].clone();
            let mut parameters = Vec::with_capacity(definition.parameters.len());
            for (sort, annotation) in &definition.parameters {
                let Annotation::Known(ty) = annotation else {
                    return Err(error(
                        span,
                        "open collector method requires known parameter domain",
                    ));
                };
                let ty = self.specialize_open_clause_type(owner, ty, span)?;
                parameters.push(Parameter { sort: *sort, ty });
            }
            let Annotation::Known(result_ty) = &definition.result else {
                return Err(error(
                    span,
                    "open collector method requires known result domain",
                ));
            };
            let result_ty = self.specialize_open_clause_type(owner, result_ty, span)?;
            let signature = Term::Function {
                parameters,
                result: Box::new(result_ty),
            };
            let symbols = vec![None; sig.params.len()];
            let callee = self.open_instance_value(owner, &symbols, signature, span)?;
            let bindings = sig
                .params
                .iter()
                .map(|parameter| local(parameter.hir_id))
                .collect();
            Ok((callee, bindings))
        })();
        self.alias_substitution = saved;
        result
    }

    fn instantiate_open_function(
        &mut self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        signature: Term,
        captures: &[(nir::DeclarationParameter, Term)],
        span: Span,
    ) -> Result<DeclarationId, Error> {
        let mut identity = Vec::new();
        for binder in self.declaration_binders(owner) {
            identity.push(
                self.alias_substitution
                    .get(&binder.declaration)
                    .ok_or_else(|| error(span, "open instance is missing a declaration argument"))?
                    .clone(),
            );
        }
        let key = generated_functions::InstanceKey {
            owner,
            declaration_inputs: identity,
            ordinary_inputs: symbols.to_vec(),
            signature,
        };
        if let Some(id) = self.generated_functions.instance(&key) {
            return Ok(id);
        }
        if self.open_instantiation_depth >= 64 {
            return Err(error(span, "open function instantiation depth exhausted"));
        }
        let local_runtime_ids =
            self.open_instance_local_runtimes(owner, symbols, &key.signature, captures, span)?;
        let checkpoint = self.generated_functions.checkpoint();
        let id = self.generated_functions.fresh_identity(span)?;
        let instance = self.generated_functions.reserve(
            key,
            nir::function_parameters::Declaration {
                declaration: id,
                parameters: captures
                    .iter()
                    .map(|(parameter, _)| parameter.declaration)
                    .collect(),
                domains: captures.iter().map(|(_, domain)| domain.clone()).collect(),
            },
        );
        let capture_type_scope: std::collections::BTreeSet<DeclarationId> = captures
            .iter()
            .filter(|(parameter, _)| parameter.sort == Sort::Type)
            .map(|(parameter, _)| parameter.declaration)
            .collect();
        let (caller_requirements, caller_capabilities) =
            self.caller_signature_contract_in_scope(&capture_type_scope, span)?;
        self.open_instantiation_depth += 1;
        let body = self.in_open_function_scope(owner, Some(Arc::clone(&instance)), |this| {
            let signature = &instance.signature;
            let item = this.open_item(owner, span)?;
            let (ItemKind::Fn(sig, body) | ItemKind::Case(sig, body)) = &item.kind else {
                return Err(error(span, "expected open function or case definition"));
            };
            this.unsafe_depth = usize::from(sig.modifiers.is_unsafe);
            if sig.modifiers.is_comptime && !local_runtime_ids.is_empty() {
                return Err(error(
                    span,
                    "comptime function cannot capture caller-local runtime quote dependency",
                ));
            }
            let Term::Function { parameters, result } = signature else {
                return Err(error(
                    span,
                    "open function instantiation requires a function signature",
                ));
            };
            let mut specialized_assumptions =
                this.specialized_open_binder_assumptions(owner, symbols, span)?;
            let mut requirements = this.specialized_open_requirements(owner, symbols, span)?;
            let mut capabilities = this.specialized_open_capabilities(owner, symbols, span)?;
            for capability in caller_capabilities {
                if !specialized_assumptions.contains(&capability) {
                    specialized_assumptions.push(capability.clone());
                }
                if !capabilities.contains(&capability) {
                    capabilities.push(capability);
                }
            }
            for requirement in caller_requirements {
                if !requirements.contains(&requirement) {
                    requirements.push(requirement);
                }
            }
            this.open_instance_capabilities = Some(specialized_assumptions.clone());
            let mut context = this
                .nominal_context()
                .theory_in_scope(this.call_input_frames());
            for requirement in crate::capabilities::subject_requirements_with_layouts(
                &this.checked_traits,
                &specialized_assumptions,
                crate::ptc::Budget::default(),
                this.implementation_index.parameter_domains(),
                Some(this.implementation_index.nominal_layouts()),
            ) {
                context.assume(crate::ptc::Formula::from_predicate(&requirement));
            }
            for capability in &specialized_assumptions {
                if std::iter::once(&capability.subject)
                    .chain(&capability.interface_arguments)
                    .any(|ty| {
                        ty.substitution_parameters()
                            .is_ok_and(|slots| !slots.is_empty())
                    })
                {
                    context.assume(crate::ptc::Formula::from_predicate(
                        &n::TypePredicate::Trait(capability.clone()),
                    ));
                }
            }
            for requirement in &requirements {
                context.assume(crate::ptc::Formula::from_predicate(requirement));
            }
            this.context = context;
            let mut env_parameters = Vec::new();
            if !sig.modifiers.is_comptime {
                for (parameter, domain) in captures {
                    let Term::Lift(inner) = domain else {
                        continue;
                    };
                    let inner_ty = this.callable_input_view(inner, span)?;
                    let binding = this.spread_binding(span)?;
                    this.locals.insert(binding, inner_ty.clone());
                    this.immutable.insert(binding);
                    this.active_env_parameters
                        .insert(parameter.declaration, (binding, inner_ty.clone()));
                    env_parameters.push(n::RuntimeEnvParameter {
                        declaration: parameter.declaration,
                        runtime_id: None,
                        binding,
                        ty: inner_ty,
                    });
                }
                for (&rt_id, (env_decl, inner_ty)) in &local_runtime_ids {
                    let inner_ty = this.callable_input_view(inner_ty, span)?;
                    let binding = this.spread_binding(span)?;
                    this.locals.insert(binding, inner_ty.clone());
                    this.immutable.insert(binding);
                    this.local_runtime_deliveries.insert(
                        rt_id,
                        LocalRuntimeDelivery {
                            address_owner: None,
                            producer_site: None,
                            expr: n::Expr {
                                source: source(span),
                                ty: inner_ty.clone(),
                                kind: K::Local(binding),
                            },
                            checked_epoch: None,
                        },
                    );
                    env_parameters.push(n::RuntimeEnvParameter {
                        declaration: *env_decl,
                        runtime_id: Some(rt_id),
                        binding,
                        ty: inner_ty,
                    });
                }
            }
            for (slot, parameter) in sig.params.iter().enumerate() {
                let binding = local(parameter.hir_id);
                let ty = &parameters[slot].ty;
                this.locals.insert(binding, ty.clone());
                if let Some(value) = &symbols[slot] {
                    this.input_facts
                        .specialized
                        .push((binding, ty.clone(), value.clone()));
                    if parameters[slot].sort == Sort::Type {
                        this.input_facts.types.insert(binding, value.clone());
                        // The instance binds this declaration's ordinary Type
                        // input. Preserve that equality for its original
                        // capability facts as well as expression substitution.
                        this.context.assume(crate::ptc::Formula::equal(
                            Term::Bound {
                                depth: 0,
                                index: slot,
                                sort: Sort::Type,
                            },
                            value.clone(),
                        ));
                    } else {
                        this.input_facts
                            .values
                            .insert(binding, (value.clone(), ty.clone()));
                    }
                }
            }
            this.control.result = Some((**result).clone());
            let preconditions = this.program_preconditions(owner)?;
            let result_binding = sig.return_binding.map(local);
            let (postconditions, entry_snapshots) =
                this.program_postconditions(owner, result_binding, result)?;
            let (result_type_binder, result_guarantees) =
                this.specialized_open_result_guarantees(owner, symbols, span)?;
            let interface = n::FunctionDeclaration {
                source: source(item.span),
                declaration: id,
                name: item.ident.name.as_str().into(),
                phase: if sig.modifiers.is_comptime {
                    n::FunctionPhase::Comptime
                } else {
                    n::FunctionPhase::Runtime
                },
                inline: sig.modifiers.is_inline,
                pure: sig.modifiers.is_pure,
                is_unsafe: sig.modifiers.is_unsafe,
                signature: signature.clone(),
                parameters: sig
                    .params
                    .iter()
                    .map(|parameter| local(parameter.hir_id))
                    .collect(),
                env_parameters,
                preconditions,
                postconditions,
                entry_snapshots,
                result_binding,
                result_type_binder,
                result_guarantees,
                type_parameters: captures
                    .iter()
                    .map(|(parameter, _)| parameter.declaration)
                    .collect(),
                requirements,
                capabilities,
                defaults: vec![],
            };
            this.generated_functions.begin_body(interface);
            let body = this
                .package
                .body(*body)
                .ok_or_else(|| error(span, "missing open function body"))?;
            let body = this.function_body(owner, body.value, result)?;
            let decreases = this.program_decreases(owner)?;
            if sig.modifiers.is_verified {
                this.safety_contracts
                    .levels
                    .insert(id, n::SafetyLevel::Verified);
            } else if sig.modifiers.is_unsafe {
                this.safety_contracts
                    .levels
                    .insert(id, n::SafetyLevel::Unsafe);
            }
            if !decreases.is_empty() {
                this.safety_contracts.decreases.insert(id, decreases);
            }
            let runtime_deliveries = this
                .local_runtime_deliveries
                .iter()
                .map(|(id, entry)| (*id, entry.expr.clone()))
                .collect();
            Ok((body, runtime_deliveries))
        });
        self.open_instantiation_depth -= 1;
        match body {
            Ok((body, runtime_deliveries)) => {
                self.generated_functions
                    .finish_body(id, body, runtime_deliveries);
                Ok(id)
            }
            Err(failure) => {
                self.generated_functions.rollback(checkpoint);
                Err(failure)
            }
        }
    }
}

fn open_contract_error(mut failure: Error) -> Error {
    if failure.message == "local binding is unavailable in this scope" {
        failure.message =
            "contract operation on an unknown domain requires definition-side type or trait evidence".into();
    }
    failure
}

fn substitute_description(
    description: &D,
    symbols: &[Option<Term>],
    span: Span,
) -> Result<D, Error> {
    match description {
        D::Unknown => Ok(D::Unknown),
        D::Known(ty) => ty
            .substitute_partial(symbols)
            .map(D::Known)
            .map_err(|failure| {
                error(
                    span,
                    format!("dependent call description requires available inputs: {failure}"),
                )
            }),
    }
}

fn source_always_exits(expr: &Expr<'_>, depth: usize) -> bool {
    if depth >= 128 {
        return false;
    }
    match &expr.kind {
        H::Return(_) | H::Break(_) | H::BreakTo(_) | H::Continue(_) | H::ContinueTo(_) => true,
        H::Semi(value) => source_always_exits(value, depth + 1),
        H::Block(block) => block
            .stmts
            .iter()
            .chain(block.expr)
            .any(|value| source_always_exits(value, depth + 1)),
        H::If(_, yes, Some(no)) => {
            yes.stmts
                .iter()
                .chain(yes.expr)
                .any(|value| source_always_exits(value, depth + 1))
                && source_always_exits(no, depth + 1)
        }
        _ => false,
    }
}
