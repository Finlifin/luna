//! Source definitions whose explicit deferred annotations need invocation inputs.
//! Ordinary descriptions never enter Term or typed NIR. Only checked concrete
//! instances are published to the runtime package.
use super::dependent_inputs::PreparedInputs;
use super::required::prepared::PreparedRequired;
use super::*;
use observations::TypeObservation as D;
use std::sync::Arc;

pub(super) use annotations::Annotation;
use annotations::annotation_source;

mod annotations;
mod fragments;
mod inline_evidence;
mod inputs;
mod observations;
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
                values.last_mut().unwrap().insert(binding);
            }
            Work::Combine(count, intersect) => {
                let start = values.len() - count;
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
                let current = values.last_mut().unwrap();
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
enum OpenClauseInference {
    Ready(Vec<Term>, PreparedInputs, BTreeMap<usize, CompletedType>),
    Pending(D),
}

#[derive(Clone)]
pub(super) struct Definition {
    parameters: Vec<(Sort, Annotation)>,
    result: Annotation,
    default_types: BTreeMap<usize, Term>,
    default_dependencies: BTreeMap<usize, BTreeSet<usize>>,
    checked_tuple_loops: BTreeSet<n::LocalId>,
    checked: bool,
}
impl Definition {
    pub(super) fn checked_empty_iteration(&self, site: hir::HirId) -> bool {
        self.checked && self.checked_tuple_loops.contains(&local(site))
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
            Some(ItemKind::Fn(sig, _)) if !sig.modifiers.is_normal_form_head)
        {
            return Ok(None);
        }
        self.ensure_signature(owner, expr.span)?;
        if !self.open_functions.contains_key(&owner) || self.declared_signature(owner).is_some() {
            return Ok(None);
        }
        self.check_open_interface_dependency(owner)?;
        self.ensure_contract(owner, expr.span)?;
        let ItemKind::Fn(sig, _) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
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
                        _ => unreachable!(),
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
                        && !matches!(parameter.kind, hir::clause::ClauseParamKind::Optional(..))
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
        let arguments = self.function_clause_arguments(expr, owner, clauses)?;
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
            let ItemKind::Fn(sig, _) = &self.package.item(owner).unwrap().kind else {
                unreachable!()
            };
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
                    expr.span,
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
                result: result.unwrap(),
            };
            self.open_instance_value(owner, &symbols, signature, expr.span)
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
        let Some(ItemKind::Fn(sig, body)) = self.package.item(owner).map(|item| &item.kind) else {
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
                .filter(|parameter| parameter.ty.is_comptime())
                .map(|parameter| local(parameter.hir_id))
                .collect();
            if is_default {
                // Follow local aliases only for dependency classification.
                // Availability and mutation are still checked by elaboration.
                let mut aliases = Vec::new();
                self.scan_body_requirement(root, |this, expression| {
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
            if self.scan_body_requirement(root, |this, expression| {
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
                if self.input_facts.values.contains_key(&local(id))
                    || self.input_facts.types.contains_key(&local(id))
                    || self.open_staged_locals.contains(&local(id)));
        let actual = self.with_staged_input(self.in_staged_input || binding.comptime, |this| {
            this.describe_open_expression(init, locals, returns, annotation.as_ref(), depth + 1)
        })?;
        let plain = matches!(
            binding.pattern.kind,
            hir::pattern::PatternKind::Binding(hir::common::BindingMode::ByValue, _, None)
                | hir::pattern::PatternKind::Wild
        ) && binding.guard.is_none()
            && binding.otherwise.is_none();
        if !plain && matches!(actual, D::Known(_)) && !matches!(annotation, Some(D::Unknown)) {
            // Reuse pattern protocols, refutability, guard scope and
            // immutable-binding checks in the ordinary checker.
            let D::Known(ty) = &actual else {
                unreachable!()
            };
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
                            | Arg::Implicit(value) => pending.push(value),
                            Arg::PostLambda(_) => {}
                        }
                    }
                }
                H::Binary(_, left, right)
                | H::AssignOp(_, left, right)
                | H::Index(left, right)
                | H::Assign(left, right)
                | H::Cast(left, right) => {
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
                | H::InlineControl(value)
                | H::Defer(value)
                | H::TyPtr(value)
                | H::TyOptional(value)
                | H::Unary(_, value)
                | H::Projection(value, _)
                | H::Ref(value)
                | H::Deref(value)
                | H::ErrorNew(value)
                | H::ErrorPropagate(value)
                | H::OptionPropagate(value)
                | H::EffectRequest(value)
                | H::Assumes(value)
                | H::TakeOld(value)
                | H::TakeBitVec(value)
                | H::TakeTagName(value)
                | H::TakeIntrinsic(value)
                | H::TakeTupleEnumerate(value)
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
                P::Guard(inner, value) => {
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

    // A fixed outer signature can still contain a call whose domain needs a
    // forwarded staged input. Propagate that requirement through direct calls
    // before checking bodies, independently of declaration order.
    pub(super) fn prepare_staged_forwarders(
        &mut self,
        owners: impl IntoIterator<Item = OwnerId>,
    ) -> Result<(), Error> {
        let mut callers: BTreeMap<OwnerId, BTreeSet<OwnerId>> = BTreeMap::new();
        for owner in owners {
            if self.open_functions.contains_key(&owner)
                || self.self_containers.contains_key(&owner)
                || self.self_types.contains_key(&owner)
            {
                continue;
            }
            let Some(ItemKind::Fn(sig, body)) = self.package.item(owner).map(|item| &item.kind)
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
            if staged.is_empty() {
                continue;
            }
            let body = self
                .package
                .body(*body)
                .ok_or_else(|| error(sig.span, "missing staged function body"))?;
            let mut calls = Vec::new();
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
            let mut routed_calls = Vec::new();
            for (target, callee, arguments, span, extended) in calls {
                let mut inputs = self.expression_free_local_dependencies(owner, &callee)?;
                // Only forwarding into a staged slot postpones type formation.
                // Supplying a known staged value as an ordinary runtime input
                // must not turn an otherwise closed function into a template.
                if let Some(ItemKind::Fn(target_sig, _) | ItemKind::FnDeclaration(target_sig)) =
                    self.package.item(target).map(|item| &item.kind)
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
                        for route in routes
                            .iter()
                            .filter(|route| target_sig.params[route.parameter].ty.is_comptime())
                        {
                            for (_, value) in route.source_inputs() {
                                inputs
                                    .extend(self.expression_free_local_dependencies(owner, value)?);
                            }
                        }
                    }
                }
                routed_calls.push((target, inputs));
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
            for (target, inputs) in routed_calls {
                let forwarded = inputs.iter().any(|binding| staged.contains(binding));
                if forwarded {
                    callers.entry(target).or_default().insert(owner);
                }
            }
        }
        let mut pending: Vec<_> = self.open_functions.keys().copied().collect();
        while let Some(target) = pending.pop() {
            for owner in callers.remove(&target).unwrap_or_default() {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(
                        self.package.item(owner).unwrap().span,
                        "staged body propagation budget exhausted",
                    )
                })?;
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
                    self.package.item(owner).unwrap().span,
                )?;
                pending.push(owner);
            }
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
            Some(ItemKind::Fn(sig, _)) => sig,
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
            || self.self_containers.contains_key(&owner)
            || self.self_types.contains_key(&owner)
        {
            return Err(error(
                span,
                "open signatures with methods or normal-form bodies are not yet implemented",
            ));
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
                TyParamKind::Positional(_)
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
        self.open_functions.insert(
            owner,
            Definition {
                parameters,
                result,
                default_types: BTreeMap::new(),
                default_dependencies: BTreeMap::new(),
                checked_tuple_loops: BTreeSet::new(),
                checked: false,
            },
        );
        Ok(())
    }

    pub(super) fn check_open_definition(&mut self, owner: OwnerId) -> Result<(), Error> {
        let definition = self.open_functions[&owner].clone();
        let needs_default_behavior = self.default_behavior_functions.contains(&owner)
            && !self.ordinary_behavior_functions.iter().any(|function| {
                function.declaration == declaration(self.package, &self.package_identities, owner)
            });
        if definition.checked && !needs_default_behavior {
            return Ok(());
        }
        let span = self.package.item(owner).unwrap().span;
        self.ensure_contract(owner, span)?;
        if self.active_open_definitions.len() >= 64 || !self.active_open_definitions.insert(owner) {
            return Err(error(
                self.package.item(owner).unwrap().span,
                "cyclic or excessively deep ordinary open-definition dependency",
            ));
        }
        let checked = self.in_function_scope(owner, None, |this| {
            let item = this.package.item(owner).unwrap();
            let (sig, body) = match &item.kind {
                ItemKind::Fn(sig, body) => (sig, Some(*body)),
                ItemKind::FnDeclaration(sig) if sig.modifiers.is_function_template => (sig, None),
                _ => unreachable!(),
            };
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
                    if let Annotation::Known(ty) = annotation {
                        nir::phases::validate_runtime_storage(ty, &metadata, &frames)
                            .map_err(|message| error(item.span, message))?;
                    }
                }
            }
            let mut locals = BTreeMap::new();
            let mut default_types = BTreeMap::new();
            let mut default_dependencies = BTreeMap::new();
            for (slot, (parameter, (sort, annotation))) in
                sig.params.iter().zip(&definition.parameters).enumerate()
            {
                this.check_suspended_annotation(annotation, sig, &mut locals)?;
                let description = annotation.description();
                if let TyParamKind::Optional(_, _, default) = parameter.ty.kind {
                    let mut default_locals = locals.clone();
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
                    )?;
                    if let D::Known(ty) = actual {
                        default_types.insert(slot, ty);
                    }
                }
                if let D::Known(ty) = &description {
                    this.locals.insert(local(parameter.hir_id), ty.clone());
                }
                if *sort == Sort::Type {
                    this.input_facts.types.insert(
                        local(parameter.hir_id),
                        Term::Bound {
                            depth: 0,
                            index: slot,
                            sort: Sort::Type,
                        },
                    );
                }
                locals.insert(local(parameter.hir_id), description);
            }
            // Defaults are suppliers. Their checks must not borrow a premise
            // that only holds after binding the parameter at function entry.
            for (parameter, (_, annotation)) in sig.params.iter().zip(&definition.parameters) {
                if let Annotation::Description(description @ n::DescriptionLiteral::Refined(_)) = annotation {
                    let subject = this.assume_parameter_refinement(parameter.hir_id, description, parameter.span)?;
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
                return Ok((default_types, default_dependencies, BTreeSet::new()));
            };
            let body = this
                .package
                .body(body)
                .ok_or_else(|| error(item.span, "missing open function body"))?;
            let declared_result = definition.result.description();
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
                        unreachable!()
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
                                unreachable!()
                            };
                            Parameter {
                                sort: *sort,
                                ty: ty.clone(),
                            }
                        })
                        .collect();
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
                    this.ordinary_behavior_functions.push(n::Function {
                        interface: nir::program::FunctionDeclaration {
                            source: source(item.span),
                            declaration: declaration(this.package, &this.package_identities, owner),
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
                            defaults: vec![],
                            type_parameters: this.type_parameters[&owner].clone(),
                            preconditions,
                            postconditions,
                            entry_snapshots,
                            result_binding: sig.return_binding.map(local),
                            requirements: this.definition_requirements(owner),
                            capabilities: this
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                        },
                        body: behavior,
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
                std::mem::take(&mut this.definition_refinements.checked_tuple_loops),
            ))
        });
        self.active_open_definitions.remove(&owner);
        let (default_types, default_dependencies, checked_tuple_loops) = checked?;
        let definition = self.open_functions.get_mut(&owner).unwrap();
        definition.default_types = default_types;
        definition.default_dependencies = default_dependencies;
        definition.checked_tuple_loops = checked_tuple_loops;
        definition.checked = true;
        Ok(())
    }

    fn require_description(
        &self,
        actual: &D,
        expected: Option<&D>,
        span: Span,
    ) -> Result<(), Error> {
        if let (D::Known(actual), Some(D::Known(expected))) = (actual, expected) {
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
        let result = (|| {
            let mut completing = true;
            for statement in block.stmts {
                let ty = self.with_open_reachability(completing, statement.span, |this| {
                    this.describe_open_expression(statement, locals, returns, None, depth + 1)
                })?;
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
        *locals = saved;
        self.locals = saved_known;
        self.open_staged_locals = saved_staged_locals;
        self.open_member_locals = saved_staged_locals_members;
        self.context = saved_context;
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
            && matches!(condition.kind, H::TraitBound(..))
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
                // A callable's signature is evidence even when its value is
                // supplied by a parameter or local. Only declaration heads
                // use the declaration telescope below; source syntax does not
                // establish that a callable has a known declaration identity.
                let H::ResolvedDef { def_id, .. } = head.kind else {
                    return self.describe_known_region(expr, expected);
                };
                let inputs = self.source_expression_inputs(expr.hir_id.owner, expr)?;
                // Knowing an input's domain is sufficient to check the ordinary
                // input plan; its value need not be available or executed.
                // This includes locals captured by referenced declarations.
                // Reserve description-only routing for genuinely missing
                // domains, so known defaults and formation requirements are
                // checked by the same rules as concrete applications.
                if inputs
                    .locals
                    .iter()
                    .all(|binding| self.locals.contains_key(binding))
                {
                    return self.describe_known_region(expr, expected);
                }
                let owner = self
                    .package
                    .definition_owner(def_id)
                    .ok_or_else(|| error(expr.span, "missing type application declaration"))?;
                if !matches!(self.package.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Fn(sig, _)) if sig.modifiers.is_normal_form_head)
                {
                    return self.describe_known_region(expr, expected);
                }
                self.ensure_signature(owner, expr.span)?;
                let parameters = self.normal_parameters[&owner].clone();
                let clauses = self.type_clause_params(owner);
                let collector = self.clause_has_collector(owner).then(|| clauses.len() - 1);
                let mut routing =
                    clause_inputs::ClauseInputRouting::new(&clauses, arguments, expr.span)?;
                // Retain source order. The final marker distinguishes a single
                // collected element from a whole input or collection spread.
                let mut routed = Vec::new();
                for argument in *arguments {
                    match argument {
                        Arg::Positional(value) | Arg::Named(_, value) => {
                            let name = match argument {
                                Arg::Named(name, _) => Some(*name),
                                _ => None,
                            };
                            let slot = routing.slot(name, value.span)?;
                            let element = name.is_none() && collector == Some(slot);
                            routed.push((slot, Some(*value), None, value.span, element));
                        }
                        Arg::Expand(value) => {
                            match routing.spread(value)? {
                                clause_inputs::Spread::Collection(slot) => {
                                    routed.push((slot, Some(*value), None, value.span, false));
                                    continue;
                                }
                                clause_inputs::Spread::Fields(fields) => {
                                    let slots = routing.packet_slots(fields.len(), value.span)?;
                                    for (field, slot) in fields.iter().zip(slots) {
                                        routed.push((slot, Some(field), None, field.span, false));
                                    }
                                    continue;
                                }
                                clause_inputs::Spread::Packet => {}
                            }
                            let packet = self.describe_open_expression(
                                value,
                                locals,
                                returns,
                                None,
                                depth + 1,
                            )?;
                            let D::Known(ty) = packet else {
                                return Err(error(
                                    value.span,
                                    "type input packet requires a known tuple domain",
                                ));
                            };
                            let Term::Tuple(fields) = self.structural_type(&ty, value.span)? else {
                                return Err(error(
                                    value.span,
                                    "type argument spread requires a tuple",
                                ));
                            };
                            let slots = routing.packet_slots(fields.len(), value.span)?;
                            for (domain, slot) in fields.into_iter().zip(slots) {
                                routed.push((slot, None, Some(domain), value.span, false));
                            }
                        }
                        _ => {
                            return Err(error(expr.span, "unsupported normal-form input marker"));
                        }
                    }
                }
                // A Type-valued head input can substitute a later domain only
                // when its value is available without invocation inputs. Keep
                // the remaining domain as a description, never a fabricated
                // rigid Type value for an ordinary comptime parameter.
                let mut known_parameters = Vec::new();
                let mut known_arguments = Vec::new();
                for (slot, value, _, _, _) in &routed {
                    let Some(value) = value else { continue };
                    if parameters[*slot].2 == p(P::Type)
                        && self
                            .source_expression_inputs(value.hir_id.owner, value)?
                            .locals
                            .is_empty()
                    {
                        known_parameters.push(parameters[*slot].0);
                        known_arguments.push(self.type_expr(value)?);
                    }
                }
                for (slot, value, packet_domain, span, element) in routed {
                    let declared = if element {
                        self.list_element_type(&parameters[slot].2).ok_or_else(|| {
                            error(span, "variadic input requires a bound meta.List<T>")
                        })?
                    } else {
                        &parameters[slot].2
                    };
                    let domain = declared
                        .instantiate_types(&known_parameters, &known_arguments)
                        .map_err(|failure| error(span, failure.to_string()))?;
                    let inputs = self.term_declaration_inputs(&domain, span)?;
                    let description = if parameters.iter().any(|(id, _, _)| inputs.contains(id)) {
                        D::Unknown
                    } else {
                        D::Known(domain)
                    };
                    if let Some(value) = value {
                        self.describe_open_expression(
                            value,
                            locals,
                            returns,
                            Some(&description),
                            depth + 1,
                        )?;
                    } else {
                        self.require_description(
                            &D::Known(packet_domain.unwrap()),
                            Some(&description),
                            span,
                        )?;
                    }
                }
                if routing.missing_required() {
                    return Err(error(expr.span, "missing required type argument"));
                }
                D::Known(p(P::Type))
            }
            H::List(elements) if matches!(expected, Some(D::Known(ty)) if self.list_element_type(ty).is_some()) =>
            {
                let Some(D::Known(ty)) = expected else {
                    unreachable!()
                };
                let element_type = self.list_element_type(ty).unwrap().clone();
                for element in *elements {
                    self.describe_open_expression(
                        element,
                        locals,
                        returns,
                        Some(&D::Known(element_type.clone())),
                        depth + 1,
                    )?;
                }
                expected.unwrap().clone()
            }
            H::TyPtr(inner) | H::TyOptional(inner) => {
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
                    unreachable!()
                };
                if let H::TakeTupleEnumerate(input) = iter.kind {
                    return self.describe_open_tuple_iteration(
                        value,
                        input,
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
                    self.with_staged_input(true, |this| {
                        this.describe_open_expression(
                            condition,
                            &mut locals.clone(),
                            returns,
                            Some(&D::Known(p(P::Bool))),
                            depth + 1,
                        )
                    })?;
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
                    && !self.input_facts.types.contains_key(&local(*id))
                    && !self.input_facts.values.contains_key(&local(*id))
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
                self.describe_open_call(expr, callee, args, locals, returns, expected, depth + 1)?
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
            H::Block(block) | H::UnsafeBlock(block) => {
                return self.describe_open_block(block, locals, returns, expected, depth + 1);
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
                let transfers = self.open_loop_transfers.remove(&identity).unwrap();
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
                    _ => unreachable!(),
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
                        unreachable!()
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
                        unreachable!()
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
                let staged = self.in_staged_input
                    || self
                        .inline_selection
                        .is_some_and(|(id, _)| id == expr.hir_id);
                let condition_ty = self.with_staged_input(staged, |this| {
                    this.describe_open_expression(condition, locals, returns, None, depth + 1)
                })?;
                let D::Known(condition_ty) = condition_ty else {
                    return Err(error(
                        condition.span,
                        "an unknown domain supplies no evidence for a boolean condition",
                    ));
                };
                self.require_expression(&condition_ty, &p(P::Bool), condition.span)?;
                let choice = control::source_boolean(condition, &mut self.required_steps, 0);
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
                if !is_never(&condition_ty) {
                    for (id, found) in &mut self.open_loop_transfers {
                        if choice != Some(false) {
                            found.include(yes_transfers[id]);
                        }
                        if choice != Some(true) {
                            found.include(no_transfers[id]);
                        }
                    }
                }
                let diverges = is_never(&condition_ty)
                    || match choice {
                        Some(true) => matches!(&yes, D::Known(ty) if is_never(ty)),
                        Some(false) => matches!(&no, D::Known(ty) if is_never(ty)),
                        None => false,
                    };
                let result = match (&yes, &no) {
                    (D::Known(ty), _) if is_never(ty) => no,
                    (_, D::Known(ty)) if is_never(ty) => yes,
                    (D::Known(a), D::Known(b)) => {
                        self.require_expression(a, b, expr.span)?;
                        yes
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
            H::TakeBitVec(value) | H::TakeTagName(value) | H::Deref(value) => {
                let description =
                    self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                let D::Known(ty) = description else {
                    return Err(error(
                        value.span,
                        match expr.kind {
                            H::TakeBitVec(_) => "bit view requires an established source type",
                            H::TakeTagName(_) => "tag_name requires an established source type",
                            H::Deref(_) => "dereference requires an established pointer type",
                            _ => unreachable!(),
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
                self.with_open_operand(ty, value.span, |this, placeholder| {
                    let probe = Expr {
                        hir_id: expr.hir_id,
                        span: expr.span,
                        kind: match expr.kind {
                            H::TakeBitVec(_) => H::TakeBitVec(placeholder),
                            H::TakeTagName(_) => H::TakeTagName(placeholder),
                            H::Deref(_) => H::Deref(placeholder),
                            _ => unreachable!(),
                        },
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
                    if self.input_facts.values.contains_key(&local(id))
                        || self.input_facts.types.contains_key(&local(id))
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
            matches!(
                self.control.return_observations,
                Some(ReturnObservations::Deferred)
            )
            .then_some(D::Unknown)
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
            this.check_open_fragment(&value, init.span, false)?;
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
            self.ordinary_behavior_inputs.insert(binding, ty);
        }
        // Preserve candidate-local aliases introduced by equality and view
        // patterns; flattening observations loses their staged binding scope.
        self.check_inline_pattern_availability(&pattern, decl.span)?;
        if let Some(guard) = decl.guard {
            self.with_staged_input(true, |this| {
                let guard_value = this.expr(guard, Some(&p(P::Bool)))?;
                this.check_open_fragment(&guard_value, guard.span, false)
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
        let selector = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(decl.span, "constant selection identity space exhausted"))?;
        self.ordinary_behavior_inputs.insert(selector, p(P::Bool));
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
            this.check_open_fragment(&value, init.span, false)?;
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
                this.check_open_fragment(&expression, decl.span, false)
            })?;
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
            _ => unreachable!(),
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
                    self.expr(&probe, expected)
                }
                H::When(arms) => {
                    let mut conditions = conditions.iter();
                    let arms: Vec<_> = arms
                        .iter()
                        .map(|arm| hir::expr::CondictionArm {
                            cond: arm.cond.map(|_| conditions.next().unwrap()),
                            ..arm.clone()
                        })
                        .collect();
                    let probe = Expr {
                        kind: H::When(&arms),
                        ..value.clone()
                    };
                    self.expr(&probe, expected)
                }
                _ => unreachable!(),
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
        let outcome = (|| {
            let mut branch_type = if fallback.is_some() {
                expected.cloned()
            } else {
                None
            };
            let mut branches = Vec::new();
            let mut reachable = true;
            for arm in arms {
                self.locals = saved_locals.clone();
                self.required_steps = self
                    .required_steps
                    .checked_sub((self.open_loop_transfers.len() as u64).saturating_mul(2))
                    .ok_or_else(|| {
                        error(arm.span, "loop completion description budget exhausted")
                    })?;
                let transfers_before = self.open_loop_transfers.clone();
                let condition = arm
                    .cond
                    .map(|condition| {
                        // Only the selector inherits the inline stage. Its
                        // runtime body must still accept runtime inputs.
                        let staged = self.in_staged_input
                            || self
                                .inline_selection
                                .is_some_and(|(id, _)| id == expr.hir_id);
                        self.with_staged_input(staged, |this| {
                            let checked = this.expr(condition, Some(&p(P::Bool)))?;
                            this.check_open_fragment(&checked, condition.span, false)?;
                            Ok::<_, Error>(checked)
                        })
                    })
                    .transpose()?;
                let condition_transfers = self.open_loop_transfers.clone();
                let mut branch_locals = locals.clone();
                let mut check = |this: &mut Self| {
                    this.describe_open_expression(
                        arm.body,
                        &mut branch_locals,
                        returns,
                        branch_type.as_ref(),
                        depth + 1,
                    )
                };
                let body = match &condition {
                    Some(condition) => {
                        self.under_type_condition(condition, true, arm.body.span, check)?
                    }
                    None => check(self)?,
                };
                if !reachable {
                    self.open_loop_transfers = transfers_before;
                } else {
                    match condition.as_ref() {
                        Some(condition) if is_never(&condition.ty) => {
                            self.open_loop_transfers = condition_transfers;
                            reachable = false;
                        }
                        Some(condition) if control::boolean(condition) == Some(false) => {
                            self.open_loop_transfers = condition_transfers;
                        }
                        None => reachable = false,
                        Some(condition) if control::boolean(condition) == Some(true) => {
                            reachable = false
                        }
                        _ => {}
                    }
                }
                if branch_type.is_none() && !matches!(&body, D::Known(ty) if is_never(ty)) {
                    branch_type = Some(body.clone());
                }
                if let Some(condition) = &condition {
                    self.assume_type_condition(condition, false, arm.body.span)?;
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
                    Some(condition) if is_never(&condition.ty) => D::Known(p(P::Never)),
                    Some(condition) => {
                        let diverges = match (&body, &result) {
                            (D::Known(yes), D::Known(no)) => {
                                control::conditional_never(&condition, yes, no)
                            }
                            _ => false,
                        };
                        let joined = match (&body, &result) {
                            (D::Known(ty), _) if is_never(ty) => result,
                            (_, D::Known(ty)) if is_never(ty) => body,
                            (D::Known(a), D::Known(b)) => {
                                self.require_expression(a, b, expr.span)?;
                                body
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
        if self.contains_inline_control(value)? {
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
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let decisions: Option<Vec<_>> = patterns
                .iter()
                .map(|pattern| control::pattern_matches(pattern, &value, &mut fuel, 0))
                .collect();
            self.required_steps -= (before - fuel) as u64;
            let selected = decisions
                .as_ref()
                .and_then(|decisions| decisions.iter().position(|matches| *matches));
            if !is_never(&value.ty) {
                for (index, transfers) in branch_transfers.into_iter().enumerate() {
                    if decisions.is_some() && selected != Some(index) {
                        continue;
                    }
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
                if selected.is_some_and(|index| branch_never[index]) {
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
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        let H::For {
            pat, guard, body, ..
        } = expr.kind
        else {
            unreachable!()
        };
        let (index_source, element_source) = self.tuple_iteration_patterns(pat)?;
        let computed = self.computed_tuple_input(input)?;
        let elements = if computed {
            let binder = self.fresh_type_binder(expr.hir_id.owner, expr.span)?;
            self.definition_refinements
                .tuple_elements
                .push(binder.declaration);
            vec![Term::Rigid {
                declaration: binder.declaration,
                sort: Sort::Type,
            }]
        } else {
            let (checked, elements) = self.tuple_enumeration_input(input)?;
            self.check_open_fragment(&checked, expr.span, false)?;
            if elements.is_empty() {
                return Err(error(
                    expr.span,
                    "empty tuple_enumerate body checking is not yet implemented",
                ));
            }
            elements
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
                self.immutable.extend(pattern_binding_ids(pat));
                let index = self
                    .with_staged_input(true, |this| this.pattern(index_source, &p(P::Integer)))?;
                patterns_irrefutable &= self.pattern_exhaustive(&index, &p(P::Integer));
                for id in self.locals.keys().filter(|id| !saved.contains_key(id)) {
                    self.open_staged_locals.insert(*id);
                }
                let element = self.pattern(element_source, &ty)?;
                patterns_irrefutable &= self.pattern_exhaustive(&element, &ty);
                for (pattern, is_staged) in [(&index, true), (&element, false)] {
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let observations = nir::visit::pattern_observations(pattern, &mut fuel);
                    self.required_steps -= (before - fuel) as u64;
                    for observation in observations.map_err(|e| error(pat.span, e))? {
                        self.with_staged_input(is_staged, |this| {
                            this.check_open_fragment(observation, pat.span, false)
                        })?;
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
        if computed {
            self.definition_refinements.tuple_elements.pop();
            if checked.is_ok() {
                self.definition_refinements
                    .checked_tuple_loops
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
            unreachable!()
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
            unreachable!()
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
            checked.unwrap()
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
        if !matches!(self.package.item(owner).map(|item| &item.kind), Some(ItemKind::Fn(signature, _)) if !signature.modifiers.is_normal_form_head)
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
        let Some(ItemKind::Fn(signature, _)) = self.package.item(owner).map(|item| &item.kind)
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
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
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
                let arguments = self.function_clause_arguments(expr, owner, clauses)?;
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
                self.function_clause_arguments(expr, owner, explicit)?
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
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            unreachable!()
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
                unreachable!()
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
                D::Known(ty) => D::Known(self.callable_input_view(&ty, expr.span)?),
                D::Unknown => D::Unknown,
            };
            if route.is_collected() {
                let fields = match &expected {
                    D::Known(ty) => {
                        route.require_collection_type(self, ty, expr.span)?;
                        let Term::Tuple(fields) = self.structural_type(ty, expr.span)? else {
                            unreachable!()
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
                    route.single_position().unwrap(),
                    route.expression(expr),
                    *sort,
                    expected,
                ));
            }
        }
        inputs.sort_by_key(|(position, ..)| *position);
        for (position, input, sort, expected) in inputs {
            while prepared.peek().is_some_and(|(at, _)| *at <= position) {
                let (_, statement) = prepared.next().unwrap();
                let value = match &statement.kind {
                    K::LetPattern { value, .. } | K::Let { value, .. } => value,
                    _ => unreachable!(),
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
                _ => unreachable!(),
            };
            self.with_open_reachability(!diverges, expr.span, |this| {
                this.check_open_fragment(value, expr.span, false)
            })?;
            diverges |= is_never(&value.ty);
        }
        self.admit_open_type_requirements(owner, &symbols, false, expr.span)?;
        let result = match substitute_description(&result, &symbols, expr.span)? {
            D::Known(ty) => D::Known(self.callable_input_view(&ty, expr.span)?),
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
        let result_context = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id)
            .map(|(_, context)| context.clone());
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
        let all_generic = self.type_parameters[&owner].clone();
        let fixed = match explicit {
            Some(types) => self.function_clause_arguments(expr, owner, types)?,
            None => self.lexical_function_arguments(owner, expr.span)?,
        };
        let (generic, template, pending_defaults) =
            self.function_type_plan(owner, &fixed, expr.span)?;
        let signature =
            self.instantiate_input_types(&signature, &all_generic, &template, expr.span)?;
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
                unreachable!()
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
                for (slot, dependencies) in &pending_defaults {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(1 + dependencies.len() as u64)
                        .ok_or_else(|| {
                            error(expr.span, "generic default dependency budget exhausted")
                        })?;
                    if dependencies.iter().all(|id| supported.contains(id)) {
                        progress |= supported.insert(all_generic[*slot]);
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
            symbols[slot] = Some(self.normal_value_argument(&route.expression(call), &domain)?);
            return Ok(());
        }
        let term = self.in_function_scope(owner, None, |this| {
            let ItemKind::Fn(sig, _) = &this.package.item(owner).unwrap().kind else {
                unreachable!()
            };
            let TyParamKind::Optional(_, _, default) = sig.params[slot].ty.kind else {
                return Err(error(call.span, "missing open contract argument"));
            };
            this.in_staged_input = true;
            for (parameter, (_, description)) in sig.params.iter().zip(parameters).take(slot) {
                if let D::Known(ty) = description {
                    this.locals.insert(local(parameter.hir_id), ty.clone());
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

    fn specialized_open_requirements(
        &self,
        owner: OwnerId,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<Vec<n::TypePredicate>, Error> {
        self.definition_requirements(owner)
            .iter()
            .map(|requirement| {
                required::map_requirement(
                    requirement,
                    &mut |ty| {
                        self.specialize_open_clause_type(owner, ty, span)?.substitute_partial(symbols).map_err(|failure| error(span,
                    format!("open type contract requires available comptime inputs: {failure}")))
                    },
                    span,
                    0,
                )
            })
            .collect()
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
        let mut binders = Vec::new();
        let mut arguments = Vec::new();
        for binder in self.declaration_binders(owner) {
            if let Some(value) = self.alias_substitution.get(&binder.declaration) {
                binders.push(binder);
                arguments.push((**value).clone());
            }
        }
        if binders.is_empty() {
            return Ok(ty.clone());
        }
        let frames = vec![self.open_functions[&owner].input_sorts()];
        ty.instantiate_parameters_in_scope(&binders, &arguments, &frames)
            .map_err(|failure| error(span, failure.to_string()))
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

    fn infer_open_clause_arguments(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        mut description: Option<(&mut BTreeMap<n::LocalId, D>, Option<&D>, usize)>,
    ) -> Result<OpenClauseInference, Error> {
        self.check_open_interface_dependency(owner)?;
        let mut definition = self.open_functions[&owner].clone();
        let ItemKind::Fn(sig, _) = &self.package.item(owner).unwrap().kind else {
            unreachable!()
        };
        let (routes, assigned) =
            route_arguments(args, Some(sig.params), sig.params.len(), expr.span)?;
        let all = self.type_parameters[&owner].clone();
        let lexical = self.lexical_function_arguments(owner, expr.span)?;
        let (generic, template, pending) = self.function_type_plan(owner, &lexical, expr.span)?;
        let mut checked = PreparedInputs::new(sig.params.len());
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
                if let Some(last) = needed.iter().copied().filter(|slot| !assigned[*slot]).max() {
                    needed.extend((0..=last).filter(|slot| !assigned[*slot]));
                }
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
                                && computation.inputs.iter().all(|(binding, _, value)| {
                                    value.is_some()
                                        || sig
                                            .params
                                            .iter()
                                            .position(|p| local(p.hir_id) == *binding)
                                            .is_some_and(|index| {
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
                            expr.span,
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
                if sig.modifiers.is_comptime {
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
                    if sig.modifiers.is_comptime
                        && (0..slot).any(|earlier| {
                            !assigned[earlier]
                                && checked.symbols[earlier].is_none()
                                && checked.live[earlier].is_none()
                        })
                    {
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
                        true,
                        expr.span,
                    )?;
                    (value, symbol.expect("staged default produces a symbol"))
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
                    .unwrap();
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
                        for (slot, dependencies) in &pending {
                            self.required_steps = self
                                .required_steps
                                .checked_sub(1 + dependencies.len() as u64)
                                .ok_or_else(|| {
                                    error(expr.span, "generic default dependency budget exhausted")
                                })?;
                            if dependencies.iter().all(|id| supported.contains(id)) {
                                added |= supported.insert(all[*slot]);
                            }
                        }
                        if !added {
                            break;
                        }
                    }
                    if generic.iter().all(|id| supported.contains(id)) {
                        let (locals, returns, depth) = description.as_mut().unwrap();
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
        if self.describing_open_call || self.checking_open_behavior {
            let fixed = self
                .declaration_binders(owner)
                .iter()
                .map(|binder| {
                    self.alias_substitution
                        .get(&binder.declaration)
                        .map(|ty| ty.as_ref().clone())
                })
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
        self.open_function_call(
            expr,
            owner,
            args,
            expected,
            PreparedInputs::new(self.open_functions[&owner].parameters.len()),
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
        if !self.describing_open_call
            && !self.checking_default_behavior
            && !matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_pure)
        {
            return Ok(None);
        }
        if self.describing_open_call
            && !self.checking_default_behavior
            && matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime)
        {
            return Ok(None);
        }
        let Some(signature) = self.declared_signature(owner).map(|sig| sig.into_owned()) else {
            return Ok(None);
        };
        if self.checking_default_behavior
            && !matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime)
        {
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
            let known = match fixed {
                Some(arguments) => arguments,
                None => match clauses {
                    Some(clauses) => self.function_clause_arguments(expr, owner, clauses)?,
                    None => self.lexical_function_arguments(owner, expr.span)?,
                },
            };
            self.generic_call_with_receiver(expr, owner, args, expected, None, vec![], known)
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
                PreparedInputs::new(self.open_functions[&owner].parameters.len()),
                BTreeMap::new(),
            );
        }
        let (arguments, checked, formed) = match self
            .infer_open_clause_arguments(expr, owner, args, expected, None)?
        {
            OpenClauseInference::Ready(arguments, checked, formed) => (arguments, checked, formed),
            OpenClauseInference::Pending(_) => {
                unreachable!("concrete calls cannot suspend inference")
            }
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

    pub(super) fn open_function_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        checked: PreparedInputs,
        formed: BTreeMap<usize, CompletedType>,
    ) -> Result<n::Expr, Error> {
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
        let item = self.package.item(owner).unwrap();
        let ItemKind::Fn(sig, _) = &item.kind else {
            unreachable!()
        };
        let (routes, assigned) =
            route_arguments(args, Some(sig.params), sig.params.len(), expr.span)?;
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
        let mut symbols = checked.symbols;
        let mut values = checked.values;
        let mut live = checked.live;
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
                expr.span,
            )?;
            let ty = if let Some(route) = routes.iter().find(|route| route.parameter == slot) {
                self.admit_supplied_annotation(
                    ty,
                    route,
                    expr,
                    &mut values[slot],
                    *sort != Sort::Runtime || sig.modifiers.is_comptime,
                )?
            } else {
                self.admit_type_data(ty, expr.span, |this| {
                    let default = definition.default_types.get(&slot).ok_or_else(|| {
                        error(
                            expr.span,
                            "gradual parameter needs a concrete domain from its default",
                        )
                    })?;
                    this.specialize_open_clause_type(owner, default, expr.span)?
                        .substitute_partial(&symbols)
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
            if *sort != Sort::Runtime || ordinary_dependencies.contains(&slot) {
                if sig.modifiers.is_comptime {
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
                    )?;
                    for (slot, ty) in early_domains {
                        definition.parameters[slot].1 = Annotation::Known(ty);
                    }
                }
                if live[slot].is_some() {
                    self.require_expression(&values[slot].as_ref().unwrap().ty, &ty, expr.span)?;
                    continue;
                }
                if !assigned[slot] && sig.modifiers.is_comptime {
                    let (completed, domain) = self
                        .prepare_open_live_default(
                            owner,
                            slot,
                            &parameters,
                            &symbols,
                            &live,
                            expr.span,
                            &[],
                        )?
                        .expect("final invocation has completed declaration arguments");
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
                } else {
                    let (value, symbol) = self.prepare_open_default(
                        owner,
                        slot,
                        &parameters,
                        &symbols,
                        true,
                        expr.span,
                    )?;
                    (
                        value,
                        symbol.expect("staged default produces an immutable symbol"),
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
            expr.span,
        )?;
        let result = self.admit_type_data(result, expr.span, |_| expected.cloned().ok_or_else(|| context_error(
                expr.span, "gradual result requires a concrete delivery domain; no result context was supplied",
            )))?;
        let signature = Term::Function {
            parameters: parameters.clone(),
            result: Box::new(result.clone()),
        };
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
        let (live_bindings, completed) = self.materialize_call_values(
            live_slots
                .iter()
                .map(|slot| (live[*slot].take().unwrap(), parameters[*slot].ty.clone()))
                .chain(
                    live_packets
                        .iter()
                        .map(|key| self.completed_packet_graph(*key)),
                )
                .collect(),
            expr.span,
        )?;
        let mut completed = completed.into_iter();
        for slot in live_slots {
            values[slot] = Some(completed.next().unwrap());
        }
        for key in live_packets {
            self.set_completed_packet_expression(key, completed.next().unwrap());
        }
        let mut arguments = Vec::new();
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
        for (slot, assigned) in assigned.iter().enumerate() {
            if *assigned {
                continue;
            }
            let (value, is_default) = if let Some(value) = values[slot].take() {
                // This default has already run. Its delivered graph belongs to
                // the caller's materialization scope, not a fresh default frame.
                (value, false)
            } else {
                (
                    self.prepare_open_default(
                        owner,
                        slot,
                        &parameters,
                        &symbols,
                        false,
                        expr.span,
                    )?
                    .0,
                    true,
                )
            };
            arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter: slot,
                value,
                is_default,
            });
        }
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
        let call = n::Expr {
            source: source(expr.span),
            ty: result,
            kind: K::Call {
                callee: Box::new(callee),
                arguments,
                parameter_bindings: sig
                    .params
                    .iter()
                    .map(|parameter| local(parameter.hir_id))
                    .collect(),
            },
        };
        let call = self.stage_collected_call(call, &routes, 0, expr)?;
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
            let clauses = self
                .declaration_binders(owner)
                .into_iter()
                .filter_map(|binder| {
                    self.alias_substitution
                        .get(&binder.declaration)
                        .map(Arc::as_ref)
                });
            for value in clauses
                .chain(symbols.iter().flatten())
                .chain(std::iter::once(signature))
            {
                pending
                    .extend(input_observation::term_inputs(value, &mut steps, span)?.declarations);
            }
            let mut captures = BTreeMap::new();
            while let Some((declaration, sort)) = pending.pop() {
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
                    _ => {
                        return Err(error(
                            span,
                            "open instance cannot capture a runtime declaration input",
                        ));
                    }
                };
                let inputs = input_observation::term_inputs(&domain, &mut steps, span)?;
                domain
                    .check_scope(&[])
                    .map_err(|failure| error(span, failure.to_string()))?;
                if domain.sort() != Sort::Type
                    || (sort == Sort::ComptimeValue && domain == p(P::Type))
                {
                    return Err(error(
                        span,
                        "open value capture has an incompatible declaration domain",
                    ));
                }
                pending.extend(inputs.declarations);
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
                    .as_ref()
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
        self.open_instantiation_depth += 1;
        let body = self.in_function_scope(owner, Some(Arc::clone(&instance)), |this| {
            let signature = &instance.signature;
            let item = this.package.item(owner).unwrap();
            let ItemKind::Fn(sig, body) = &item.kind else {
                unreachable!()
            };
            let Term::Function { parameters, result } = signature else {
                unreachable!()
            };
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
                preconditions,
                postconditions,
                entry_snapshots,
                result_binding,
                type_parameters: captures
                    .iter()
                    .map(|(parameter, _)| parameter.declaration)
                    .collect(),
                requirements: this.specialized_open_requirements(owner, symbols, span)?,
                capabilities: this.specialized_open_capabilities(owner, symbols, span)?,
                defaults: vec![],
            };
            this.generated_functions.begin_body(interface);
            let body = this
                .package
                .body(*body)
                .ok_or_else(|| error(span, "missing open function body"))?;
            this.function_body(owner, body.value, result)
        });
        self.open_instantiation_depth -= 1;
        match body {
            Ok(body) => {
                self.generated_functions.finish_body(id, body);
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
