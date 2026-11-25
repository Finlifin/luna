//! Required, hygienic expansion of concrete quote functions. Templates are
//! checked as runtime code, then removed after all direct applications expand.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn is_quote_function(&self, owner: OwnerId) -> bool {
        self.package.item(owner).is_some_and(|item| {
            matches!(&item.kind,
            ItemKind::Fn(sig,_) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_quote)
        })
    }

    pub(in crate::program) fn validate_quote_function(
        &self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        if !self.is_quote_function(owner) {
            return Ok(());
        }
        let item = self
            .package
            .item(owner)
            .ok_or_else(|| error(span, "missing quote function item"))?;
        let sig = match &item.kind {
            ItemKind::Fn(sig, _) => sig,
            ItemKind::FnDeclaration(sig)
                if sig.modifiers.is_function_template
                    || self.package.quote_fn_arms.contains_key(&owner) =>
            {
                sig
            }
            ItemKind::FnDeclaration(_)
                if self.self_types.contains_key(&owner)
                    && !self.self_containers.contains_key(&owner) =>
            {
                // An abstract interface records the required code category.
                // There is no body to expand or runtime entrypoint to emit.
                return Ok(());
            }
            _ => {
                return Err(error(
                    span,
                    "quote fn declaration requires arm or template assembly, which is not yet implemented",
                ));
            }
        };
        if sig.clause_params.iter().any(|parameter| {
            !matches!(
                parameter.kind,
                hir::clause::ClauseParamKind::Type(_)
                    | hir::clause::ClauseParamKind::TraitBound(..)
                    | hir::clause::ClauseParamKind::Positional(..)
                    | hir::clause::ClauseParamKind::Optional(..)
            )
        }) {
            return Err(error(span, "unsupported quote fn clause protocol"));
        }
        if sig.params.iter().any(|param| {
            let is_contextual_quote = param.ty.is_implicit()
                && param.ty.is_quote()
                && !param.ty.is_comptime()
                && !param.ty.is_lambda()
                && matches!(param.ty.kind, TyParamKind::Positional(_));
            let allowed_flags = if is_contextual_quote {
                hir::common::TPARAM_IMPLICIT
                    | hir::common::TPARAM_LAMBDA
                    | hir::common::TPARAM_COMPTIME
                    | hir::common::TPARAM_QUOTE
            } else {
                hir::common::TPARAM_IMPLICIT
                    | hir::common::TPARAM_LAMBDA
                    | hir::common::TPARAM_COMPTIME
            };
            param.ty.flags & !allowed_flags != 0
                || !matches!(
                    param.ty.kind,
                    TyParamKind::Positional(_)
                        | TyParamKind::Optional(..)
                        | TyParamKind::Varadic(..)
                        | TyParamKind::Itself { .. }
                )
        }) {
            return Err(error(
                span,
                "unsupported parameter protocol in quote fn signature",
            ));
        }
        if self
            .quote_fn_template_flags
            .get(&owner)
            .is_some_and(|flags| flags.for_match_arms)
        {
            if sig.params.is_empty()
                || !matches!(sig.params[0].ty.kind, TyParamKind::Positional(_))
                || sig.params[0].ty.flags != 0
            {
                return Err(error(
                    span,
                    "match-arm quote template requires an ordinary first scrutinee parameter",
                ));
            }
            if sig
                .params
                .iter()
                .skip(1)
                .any(|param| !(param.ty.is_implicit() && param.ty.is_quote()))
            {
                return Err(error(
                    span,
                    "match-arm quote template parameters after the scrutinee must be implicit quote inputs",
                ));
            }
        }
        Ok(())
    }

    #[inline(never)]
    pub(in crate::program) fn check_quote_callable(
        &self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        if self
            .quote_fn_template_flags
            .get(&owner)
            .is_some_and(|flags| flags.for_match_arms)
            || self.package.quote_fn_arms.contains_key(&owner)
        {
            return Err(error(
                span,
                "quote match-arm templates and arm instances cannot be called or referenced as values; use them in a match expression",
            ));
        }
        Ok(())
    }

    #[inline(never)]
    pub(in crate::program) fn validate_contextual_quote_binding(
        &self,
        binding: n::LocalId,
        name: &str,
        span: Span,
        context_label: &str,
    ) -> Result<Term, Error> {
        if self.code_values.contains_key(&binding) {
            return Err(error(
                span,
                format!("contextual quote input `{name}` cannot bind a quoted code value"),
            ));
        }
        if self.pattern_codes.contains_key(&binding) {
            return Err(error(
                span,
                format!("contextual quote input `{name}` cannot bind a quoted pattern value"),
            ));
        }
        if self.in_required_comptime || self.in_staged_input || !self.runtime_value_context() {
            return Err(error(
                span,
                format!(
                    "contextual quote input `{name}` cannot read a runtime value during comptime"
                ),
            ));
        }
        if self.generator_data.contains_key(&binding) {
            return Err(error(
                span,
                format!(
                    "contextual quote input `{name}` requires a runtime lexical binding, not a comptime local"
                ),
            ));
        }
        let ty = self.locals.get(&binding).cloned().ok_or_else(|| {
            error(
                span,
                format!(
                    "contextual quote input `{name}` is not active in {context_label} lexical scope"
                ),
            )
        })?;
        if ty == p(P::Type)
            || ty.is_comptime_callable()
            || ty.is_normal_form_callable()
            || matches!(ty, Term::Primitive(P::Integer | P::Float))
        {
            return Err(error(
                span,
                format!("contextual quote input `{name}` cannot bind a comptime-only value"),
            ));
        }
        Ok(ty)
    }

    #[inline(never)]
    pub(in crate::program) fn resolve_contextual_quote_input(
        &self,
        call_expr: &Expr<'_>,
        parameter: &hir::common::FnParam<'_>,
    ) -> Result<n::Expr, Error> {
        let Some(target_hir_id) = self
            .package
            .lexical_call_locals
            .get(&call_expr.hir_id)
            .and_then(|locals| locals.get(parameter.name.name.as_str()).copied())
        else {
            return Err(error(
                call_expr.span,
                format!(
                    "missing contextual quote input `{}` in caller lexical scope",
                    parameter.name.name
                ),
            ));
        };
        let binding = local(target_hir_id);
        let ty = self.validate_contextual_quote_binding(
            binding,
            parameter.name.name.as_str(),
            call_expr.span,
            "caller",
        )?;
        Ok(n::Expr {
            source: source(call_expr.span),
            ty,
            kind: K::Local(binding),
        })
    }

    #[inline(never)]
    pub(in crate::program) fn resolve_contextual_quote_inputs(
        &self,
        call_expr: &Expr<'_>,
        declarations: Option<&[hir::common::FnParam<'_>]>,
        offset: usize,
        assigned: &[bool],
    ) -> Result<Vec<(usize, n::Expr)>, Error> {
        let Some(declarations) = declarations else {
            return Ok(Vec::new());
        };
        let mut resolved = Vec::new();
        for (index, is_assigned) in assigned.iter().enumerate() {
            if *is_assigned {
                continue;
            }
            let slot = offset + index;
            let parameter = &declarations[slot];
            if parameter.ty.is_implicit() && parameter.ty.is_quote() {
                resolved.push((
                    slot,
                    self.resolve_contextual_quote_input(call_expr, parameter)?,
                ));
            }
        }
        Ok(resolved)
    }

    pub(in crate::program) fn record_quote_application(
        &mut self,
        id: hir::HirId,
        span: Span,
    ) -> Result<(), Error> {
        let comptime = self.in_required_comptime
            || self.in_staged_input
            || self
                .captures
                .last()
                .map(|frame| frame.comptime)
                .unwrap_or_else(|| {
                    self.signature_owner
                        .and_then(|owner| self.package.item(owner))
                        .is_some_and(|item| {
                            matches!(&item.kind,ItemKind::Fn(sig,_) | ItemKind::FnDeclaration(sig)
                        if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)
                        })
                });
        if comptime && !self.in_code_body {
            return Err(error(span, "quote fn application requires a runtime phase"));
        }
        self.record_quote_hole(id, span)
    }

    pub(in crate::program) fn record_quote_hole(
        &mut self,
        id: hir::HirId,
        span: Span,
    ) -> Result<(), Error> {
        if self.in_code_body {
            insert_quote_hole(&mut self.quote_holes, id, span)?;
        }
        Ok(())
    }
}

fn insert_quote_hole(
    holes: &mut BTreeSet<n::LocalId>,
    id: hir::HirId,
    span: Span,
) -> Result<(), Error> {
    holes.insert(local(id));
    if holes.len() > 1 {
        return Err(error(
            span,
            "a quote permits at most one splice or quote fn application",
        ));
    }
    Ok(())
}

fn at(source: n::SourceRange) -> Span {
    Span::new(
        rustc_span::BytePos(source.start),
        rustc_span::BytePos(source.end),
    )
}

struct Expansion<'a> {
    selection: Option<&'a n::Package>,
    parameter_domains: Option<BTreeMap<DeclarationId, Term>>,
    templates: &'a BTreeMap<DeclarationId, &'a n::Function>,
    contextual_slots: Option<&'a BTreeMap<DeclarationId, BTreeSet<usize>>>,
    caller_env_parameters: &'a [n::RuntimeEnvParameter],
    caller_runtime_deliveries: &'a BTreeMap<nir::RuntimeId, n::Expr>,
    in_closure: bool,
    next: &'a mut u32,
    fuel: &'a mut u64,
    sources: &'a mut Vec<n::ExpansionSource>,
    enums: &'a mut Vec<n::Enum>,
    internal_packets: &'a mut BTreeSet<DeclarationId>,
    empty_replacements: &'a mut BTreeMap<(u32, u32), DeclarationId>,
    depth: usize,
    active: Vec<(DeclarationId, Vec<Term>, n::SourceRange)>,
    runtime: bool,
}

// Scratch state retained while child templates expand. Keeping it on the heap
// lets the preparation and completion stack frames return before recursion.
struct PreparedTemplate {
    body: n::Expr,
    postconditions: Vec<n::Expr>,
    result_binding: Option<n::LocalId>,
    connections: BTreeMap<n::LocalId, n::LocalId>,
    effects: Option<Vec<nir::effects::Operation>>,
    delivered_type: Term,
    prepared: Vec<n::Expr>,
    statements: Vec<n::Expr>,
}

impl Expansion<'_> {
    fn domains(&mut self, ids: &[DeclarationId], span: Span) -> Result<Vec<Term>, Error> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
        let result = (|| {
            fuel = fuel
                .checked_sub(ids.len())
                .ok_or_else(|| "quote parameter budget exhausted".to_string())?;
            if self.parameter_domains.is_none() {
                self.parameter_domains = Some(match self.selection {
                    Some(package) => package.declaration_parameter_domains(&mut fuel)?,
                    None => BTreeMap::new(),
                });
            }
            let domains = self
                .parameter_domains
                .as_ref()
                .ok_or_else(|| "missing quote parameter domains".to_string())?;
            ids.iter()
                .map(|id| {
                    let domain = domains.get(id).unwrap_or(&Term::Primitive(P::Type));
                    nir::type_computations::charge_term(domain, &mut fuel)
                        .map_err(|e| e.to_string())?;
                    Ok(domain.clone())
                })
                .collect::<Result<Vec<_>, String>>()
        })();
        *self.fuel = fuel as u64;
        result.map_err(|reason| error(span, reason))
    }

    fn fresh(&mut self, span: Span) -> Result<n::LocalId, Error> {
        let id = n::LocalId {
            owner: u32::MAX,
            index: *self.next,
        };
        *self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error(span, "quote fn identity space exhausted"))?;
        Ok(id)
    }

    fn node(&mut self, ty: Term, kind: K, source: n::SourceRange) -> Result<n::Expr, Error> {
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(at(source), "quote contract budget exhausted"))?;
        let index = u32::try_from(self.sources.len())
            .map_err(|_| error(at(source), "quote source space exhausted"))?;
        self.sources.push(n::ExpansionSource {
            original: source,
            site: source,
        });
        Ok(n::Expr {
            ty,
            kind,
            source: n::SourceRange::generated(index),
        })
    }

    fn charge(&mut self, body: &n::Expr, span: Span) -> Result<(), Error> {
        let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
        let result = nir::visit::expression(body, &mut fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
        });
        *self.fuel = fuel as u64;
        result.map_err(|message| error(span, message))
    }

    #[inline(never)]
    fn expand(
        &mut self,
        expression: &mut n::Expr,
        id: DeclarationId,
        types: Vec<Term>,
    ) -> Result<(), Error> {
        if !self.runtime {
            return Err(error(
                at(expression.source),
                "quote fn application requires a runtime phase",
            ));
        }
        if self.depth >= 32 {
            return Err(error(
                at(expression.source),
                "quote fn expansion depth exhausted",
            ));
        }
        let mut origin = expression.source;
        while let Some(index) = origin.expansion_index() {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or_else(|| error(at(expression.source), "quote origin budget exhausted"))?;
            origin = self
                .sources
                .get(index as usize)
                .ok_or_else(|| error(at(expression.source), "missing quote expansion origin"))?
                .original;
        }
        if self.active.iter().any(|(active, arguments, site)| {
            *active == id && arguments == &types && *site == origin
        }) {
            return Err(error(
                at(expression.source),
                "quote fn expansion depth exhausted: recursive concrete instance",
            ));
        }
        let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
        for ty in &types {
            nir::type_computations::charge_term(ty, &mut fuel)
                .map_err(|failure| error(at(expression.source), failure.to_string()))?;
        }
        *self.fuel = fuel as u64;
        self.active.push((id, types.clone(), origin));
        self.depth += 1;
        let result = self.expand_inner(expression, id, types);
        self.depth -= 1;
        self.active.pop();
        result
    }

    #[inline(never)]
    fn expand_inner(
        &mut self,
        expression: &mut n::Expr,
        id: DeclarationId,
        types: Vec<Term>,
    ) -> Result<(), Error> {
        let template = self.templates[&id];
        self.expand_template(expression, template, types)
    }

    fn enter_computation_body(
        &mut self,
        expression: &mut n::Expr,
        effects: &[nir::effects::Operation],
        result: &Term,
    ) -> Result<(), Error> {
        let mut current = expression;
        loop {
            *self.fuel = self.fuel.checked_sub(1).ok_or_else(|| {
                error(
                    at(current.source),
                    "quote computation boundary budget exhausted",
                )
            })?;
            if matches!(&current.kind,K::EffectComputation {effects:actual,body}
                if actual==effects && (body.ty==*result || is_never(&body.ty)))
            {
                let K::EffectComputation { body, .. } =
                    std::mem::replace(&mut current.kind, K::Unit)
                else {
                    unreachable!()
                };
                *current = *body;
                return Ok(());
            }
            match &mut current.kind {
                // Staged binding can wrap the original computation in closed
                // input bindings. Keep these within its lexical body scope.
                K::Block {
                    result: Some(body), ..
                }
                | K::CallPreparation {
                    result: Some(body), ..
                } => {
                    current.ty = result.clone();
                    current = body;
                }
                _ => {
                    return Err(error(
                        at(current.source),
                        "quote effect body is missing its checked computation boundary",
                    ));
                }
            }
        }
    }

    fn checked_result(
        &mut self,
        value: n::Expr,
        predicates: &[n::Expr],
        result: Option<n::LocalId>,
        connections: &BTreeMap<n::LocalId, n::LocalId>,
    ) -> Result<n::Expr, Error> {
        let source = value.source;
        let ty = value.ty.clone();
        let binding = self.fresh(at(source))?;
        let mut aliases = connections.clone();
        if let Some(result) = result {
            aliases.insert(result, binding);
        }
        let mut statements = vec![self.node(
            p(P::Void),
            K::Let {
                binding,
                value: Box::new(value),
            },
            source,
        )?];
        for predicate in predicates {
            self.charge(predicate, at(source))?;
            let mut predicate = predicate.clone();
            freshen(
                &mut predicate,
                aliases.clone(),
                self.next,
                self.sources,
                at(source),
            )?;
            predicate.rewrite_with(self)?;
            let source = predicate.source;
            statements.push(self.node(
                p(P::Void),
                K::EnsurePredicate(Box::new(predicate)),
                source,
            )?);
        }
        let result = self.node(ty.clone(), K::Local(binding), source)?;
        self.node(
            ty,
            K::Block {
                statements,
                result: Some(Box::new(result)),
            },
            source,
        )
    }

    fn contract_returns(
        &mut self,
        body: &mut n::Expr,
        predicates: &[n::Expr],
        result: Option<n::LocalId>,
        connections: &BTreeMap<n::LocalId, n::LocalId>,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= 64 {
            return Err(error(
                at(body.source),
                "quote contract nesting budget exhausted",
            ));
        }
        let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
        let checked = nir::visit::expression_mut_pruned(body, &mut fuel, |expr, fuel| {
            *self.fuel = *fuel as u64;
            let checked = (|| {
                if let K::Closure { captures, .. } = &mut expr.kind {
                    for capture in captures {
                        if let Some(value) = &mut capture.value {
                            self.contract_returns(
                                value,
                                predicates,
                                result,
                                connections,
                                depth + 1,
                            )?;
                        }
                    }
                    return Ok(false);
                }
                if let K::Return(value) = &mut expr.kind {
                    let value = if let Some(value) = value.take() {
                        *value
                    } else {
                        self.node(p(P::Void), K::Unit, expr.source)?
                    };
                    expr.kind = K::Return(Some(Box::new(self.checked_result(
                        value,
                        predicates,
                        result,
                        connections,
                    )?)));
                }
                Ok(true)
            })();
            *fuel = *self.fuel as usize;
            checked.map_err(|failure: Error| failure.to_string())
        });
        *self.fuel = fuel as u64;
        checked.map_err(|failure| error(at(body.source), failure))
    }

    fn merge_entry_contract(
        &mut self,
        expression: &n::Expr,
        template: &mut n::Function,
    ) -> Result<(), Error> {
        let K::Call { callee, .. } = &expression.kind else {
            return Ok(());
        };
        let K::TraitMethod {
            interface,
            member,
            subject,
            interface_arguments,
            method_arguments,
        } = &callee.kind
        else {
            return Ok(());
        };
        if template.declaration == *member {
            return Ok(());
        }
        let Some(package) = self.selection else {
            return Ok(());
        };
        let interface = package
            .traits
            .iter()
            .find(|entry| entry.declaration == *interface)
            .ok_or_else(|| error(at(expression.source), "missing quote interface contract"))?;
        let method = interface
            .methods
            .iter()
            .find(|entry| entry.declaration == *member)
            .ok_or_else(|| error(at(expression.source), "missing quote method contract"))?;
        for predicate in method
            .program
            .preconditions
            .iter()
            .chain(&method.program.postconditions)
        {
            self.charge(predicate, at(expression.source))?;
        }
        let declarations = interface
            .parameters_with_self()
            .into_iter()
            .chain(method.type_parameters.iter().copied())
            .collect::<Vec<_>>();
        let arguments = std::iter::once(subject.clone())
            .chain(interface_arguments.iter().cloned())
            .chain(method_arguments.iter().cloned())
            .collect::<Vec<_>>();
        let domains = self.domains(&declarations, at(expression.source))?;
        let frames = match template.signature.callable_signature() {
            Term::Function { parameters, .. } if !parameters.is_empty() => {
                vec![parameters.iter().map(|parameter| parameter.sort).collect()]
            }
            _ => Vec::new(),
        };
        let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
        let program = method.program.instantiate_declaration_parameters(
            &declarations,
            &domains,
            &arguments,
            &frames,
            &mut fuel,
        );
        *self.fuel = fuel as u64;
        let mut program =
            program.map_err(|failure| error(at(expression.source), failure.to_string()))?;
        if method.parameter_bindings.len() != template.parameters.len() {
            return Err(error(
                at(expression.source),
                "quote contract parameter frame mismatch",
            ));
        }
        let mut aliases: BTreeMap<_, _> = method
            .parameter_bindings
            .iter()
            .copied()
            .zip(template.parameters.iter().copied())
            .collect();
        if let Some(result) = program.result_binding {
            let target = if let Some(binding) = template.interface.result_binding {
                binding
            } else {
                let binding = self.fresh(at(expression.source))?;
                template.interface.result_binding = Some(binding);
                binding
            };
            aliases.insert(result, target);
        }
        for snapshot in program.entry_snapshots {
            let parameter = *aliases.get(&snapshot.parameter).ok_or_else(|| {
                error(
                    at(expression.source),
                    "quote contract snapshot parameter is missing",
                )
            })?;
            let binding = if let Some(old) = template
                .interface
                .entry_snapshots
                .iter()
                .find(|old| old.parameter == parameter)
            {
                old.binding
            } else {
                let binding = self.fresh(at(expression.source))?;
                template
                    .interface
                    .entry_snapshots
                    .push(n::EntrySnapshot { parameter, binding });
                binding
            };
            aliases.insert(snapshot.binding, binding);
        }
        // Interface checks are invocation boundaries, independent of the
        // implementation's effective contract (including inherited guards).
        // Preserve both when replacing a method call with its body, just as
        // ordinary dispatch checks the interface before/after the invocation.
        for predicate in program
            .preconditions
            .iter_mut()
            .chain(&mut program.postconditions)
        {
            freshen(
                predicate,
                aliases.clone(),
                self.next,
                self.sources,
                at(expression.source),
            )?;
        }
        template
            .interface
            .preconditions
            .splice(0..0, program.preconditions);
        template
            .interface
            .postconditions
            .extend(program.postconditions);
        Ok(())
    }

    #[inline(never)]
    fn expand_template(
        &mut self,
        expression: &mut n::Expr,
        source_template: &n::Function,
        types: Vec<Term>,
    ) -> Result<(), Error> {
        let mut prepared = self.prepare_template(expression, source_template, types)?;
        for value in &mut prepared.prepared {
            value.rewrite_with(self)?;
        }
        for value in &mut prepared.statements {
            value.rewrite_with(self)?;
        }
        prepared.body.rewrite_with(self)?;
        self.finish_template(expression, prepared)
    }

    #[inline(never)]
    fn prepare_template(
        &mut self,
        expression: &mut n::Expr,
        source_template: &n::Function,
        types: Vec<Term>,
    ) -> Result<Box<PreparedTemplate>, Error> {
        let span = at(expression.source);
        // Declaration specialization owns the complete expanded-copy preflight.
        let domains = self.domains(&source_template.type_parameters, span)?;
        let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
        let template =
            source_template.instantiate_declaration_parameters(&domains, &types, &mut fuel);
        *self.fuel = fuel as u64;
        let mut template = template
            .map_err(|failure| error(span, format!("quote fn instantiation: {failure}")))?;
        if !template.runtime_deliveries.is_empty() {
            return Err(error(
                span,
                "inline expansion does not support callee-local runtime deliveries",
            ));
        }
        self.merge_entry_contract(expression, &mut template)?;
        if let Some(package) = self.selection {
            if template.phase == n::FunctionPhase::Runtime {
                let implicit = nir::program::implicit_runtime_env_parameters(
                    &package.runtime_domains,
                    &package.global_runtime_deliveries,
                    &types,
                    &template.interface.env_parameters,
                )
                .map_err(|failure| error(span, failure))?;
                template.interface.env_parameters.extend(implicit);
            }
            let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
            let normalized = crate::materialization::normalize_inline_annotations(
                package,
                &mut template,
                &mut fuel,
            );
            *self.fuel = fuel as u64;
            normalized.map_err(|reason| error(span, reason))?;
        }
        let (effects, delivered_type) = match &expression.ty {
            Term::Computation { effects, result } => {
                (Some(effects.clone()), result.as_ref().clone())
            }
            result => (None, result.clone()),
        };
        let K::Call {
            arguments,
            parameter_bindings,
            env_arguments,
            ..
        } = &mut expression.kind
        else {
            unreachable!()
        };
        let resolved_env_exprs = if env_arguments.len() <= template.interface.env_parameters.len() {
            let mut delivered_envs = std::mem::take(env_arguments);
            for env in &template.interface.env_parameters[delivered_envs.len()..] {
                let delivered = if !self.in_closure
                    && let Some(ep) = self.caller_env_parameters.iter().find(|ep| {
                        match (env.runtime_id, ep.runtime_id) {
                            (Some(req), Some(avail)) => req == avail,
                            (None, _) => ep.declaration == env.declaration,
                            (Some(_), None) => false,
                        }
                    }) {
                    self.node(ep.ty.clone(), K::Local(ep.binding), expression.source)?
                } else if !self.in_closure
                    && let Some(rt_id) = env.runtime_id
                    && let Some(expr) = self.caller_runtime_deliveries.get(&rt_id)
                {
                    self.charge(expr, span)?;
                    expr.clone()
                } else if let Some(package) = self.selection
                    && let Some(rt_id) = env.runtime_id
                    && let Some(expr) = package.global_runtime_deliveries.get(&rt_id)
                {
                    self.charge(expr, span)?;
                    expr.clone()
                } else {
                    return Err(error(
                        span,
                        "inline expansion cannot deliver runtime environment parameter",
                    ));
                };
                if delivered.ty != env.ty {
                    return Err(error(
                        span,
                        "inline expansion runtime environment type mismatch",
                    ));
                }
                delivered_envs.push(delivered);
            }
            delivered_envs
        } else {
            return Err(error(
                span,
                "inline expansion runtime environment argument count mismatch",
            ));
        };
        if arguments.len() != template.parameters.len() {
            return Err(error(
                span,
                "quote fn expansion requires a complete ordinary argument frame",
            ));
        }
        let mut connections = BTreeMap::new();
        let mut prepared_connections = BTreeMap::new();
        for parameter in &template.parameters {
            connections.insert(*parameter, self.fresh(span)?);
            prepared_connections.insert(*parameter, self.fresh(span)?);
        }
        let contextual = self
            .contextual_slots
            .and_then(|map| map.get(&source_template.declaration));
        let mut aliased = BTreeSet::new();
        let mut prepared_types = BTreeMap::new();
        let mut statements = Vec::new();
        for argument in std::mem::take(arguments) {
            let original = template
                .parameters
                .get(argument.parameter)
                .ok_or_else(|| error(span, "invalid quote fn argument slot"))?;
            if !argument.is_default
                && contextual.is_some_and(|slots| slots.contains(&argument.parameter))
                && let K::Local(caller_binding) = argument.value.kind
            {
                connections.insert(*original, caller_binding);
                prepared_connections.insert(*original, caller_binding);
                aliased.insert(*original);
                continue;
            }
            let mut value = argument.value;
            if argument.is_default {
                // Defaults run in the callee's parameter frame after explicit
                // inputs; redirect earlier formal identities simultaneously.
                let mut default_connections = prepared_connections.clone();
                if parameter_bindings.len() != template.parameters.len() {
                    return Err(error(
                        span,
                        "quote fn default argument frame has incompatible bindings",
                    ));
                }
                for (entry, body) in parameter_bindings.iter().zip(&template.parameters) {
                    default_connections.insert(*entry, prepared_connections[body]);
                }
                freshen(
                    &mut value,
                    default_connections,
                    self.next,
                    self.sources,
                    span,
                )?;
            }
            // Give preparation its own identity without ending temporaries.
            // Reusing the value's source aliases its projection/temporary hook
            // with this binding's hook after quote or inline expansion.
            let source = value.source;
            prepared_types.insert(*original, (value.ty.clone(), source));
            statements.push(self.node(
                p(P::Void),
                K::Let {
                    binding: prepared_connections[original],
                    value: Box::new(value),
                },
                source,
            )?);
        }
        let mut prepared_envs = Vec::with_capacity(resolved_env_exprs.len());
        for (env, env_expr) in template
            .interface
            .env_parameters
            .iter()
            .zip(resolved_env_exprs)
        {
            let prepared_env = self.fresh(span)?;
            let body_env = self.fresh(span)?;
            let env_source = env_expr.source;
            connections.insert(env.binding, body_env);
            statements.push(self.node(
                p(P::Void),
                K::Let {
                    binding: prepared_env,
                    value: Box::new(env_expr),
                },
                env_source,
            )?);
            prepared_envs.push((prepared_env, body_env, env.ty.clone(), env_source));
        }
        // Ordinary argument evaluation shares the caller's temporary lifetime.
        // Entry snapshots/contracts and the callee body have their own lexical
        // scopes, but cannot end argument projection temporaries early.
        let prepared = std::mem::take(&mut statements);
        // Parameters enter the callee only after all inputs are prepared.
        // This is a move of responsibility, not a second evaluation: failure
        // in the body must retire callee parameters before caller temporaries.
        for parameter in &template.parameters {
            if aliased.contains(parameter) {
                continue;
            }
            let (ty, source) = prepared_types
                .remove(parameter)
                .ok_or_else(|| error(span, "missing prepared quote input"))?;
            let value = self.node(ty, K::Local(prepared_connections[parameter]), source)?;
            statements.push(self.node(
                p(P::Void),
                K::Let {
                    binding: connections[parameter],
                    value: Box::new(value),
                },
                source,
            )?);
        }
        for (prepared_env, body_env, env_ty, env_source) in prepared_envs {
            let value = self.node(env_ty, K::Local(prepared_env), env_source)?;
            statements.push(self.node(
                p(P::Void),
                K::Let {
                    binding: body_env,
                    value: Box::new(value),
                },
                env_source,
            )?);
        }
        let Term::Function { parameters, .. } = template.signature.callable_signature() else {
            return Err(error(span, "quote template signature is not callable"));
        };
        for snapshot in &template.interface.entry_snapshots {
            let index = template
                .parameters
                .iter()
                .position(|parameter| *parameter == snapshot.parameter)
                .ok_or_else(|| error(span, "quote entry snapshot source is not a parameter"))?;
            let ty = parameters[index].ty.clone();
            let value = self.node(
                ty.clone(),
                K::Local(connections[&snapshot.parameter]),
                expression.source,
            )?;
            let value = self.node(ty, K::Snapshot(Box::new(value)), expression.source)?;
            let binding = self.fresh(span)?;
            connections.insert(snapshot.binding, binding);
            statements.push(self.node(
                p(P::Void),
                K::Let {
                    binding,
                    value: Box::new(value),
                },
                expression.source,
            )?);
        }
        for mut predicate in template.interface.preconditions {
            freshen(
                &mut predicate,
                connections.clone(),
                self.next,
                self.sources,
                span,
            )?;
            let source = predicate.source;
            statements.push(self.node(
                p(P::Void),
                K::RequirePredicate(Box::new(predicate)),
                source,
            )?);
        }
        let mut body = template.body;
        if let Some(effects) = &effects {
            self.enter_computation_body(&mut body, effects, &delivered_type)?;
        }
        freshen(
            &mut body,
            connections.clone(),
            self.next,
            self.sources,
            span,
        )?;
        Ok(Box::new(PreparedTemplate {
            body,
            postconditions: template.interface.postconditions,
            result_binding: template.interface.result_binding,
            connections,
            effects,
            delivered_type,
            prepared,
            statements,
        }))
    }

    #[inline(never)]
    fn finish_template(
        &mut self,
        expression: &mut n::Expr,
        frame: Box<PreparedTemplate>,
    ) -> Result<(), Error> {
        let span = at(expression.source);
        let PreparedTemplate {
            mut body,
            postconditions,
            result_binding,
            connections,
            effects,
            delivered_type,
            prepared,
            statements,
        } = *frame;
        // Explicit returns check postconditions before retiring lexical locals.
        // Turning them into block completion would move that check past cleanup.
        if postconditions.is_empty() {
            let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
            let simplified =
                nir::inline_return::simplify_tail_returns(&mut body, &delivered_type, &mut fuel);
            *self.fuel = fuel as u64;
            simplified.map_err(|message| error(span, message))?;
        }
        if !postconditions.is_empty() {
            self.contract_returns(&mut body, &postconditions, result_binding, &connections, 0)?;
            body = self.checked_result(body, &postconditions, result_binding, &connections)?;
        }
        // A tail result already has the block's ordinary ownership boundary.
        // Only lexical early returns need a private packet and exit loop.
        let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
        let needs_boundary = nir::inline_return::has_lexical_return(&body, &mut fuel);
        *self.fuel = fuel as u64;
        let body = if needs_boundary.map_err(|message| error(span, message))? {
            let slot = self.fresh(span)?;
            let exit = self.fresh(span)?;
            let result = self.fresh(span)?;
            let unreachable = self.fresh(span)?;
            let packet = DeclarationId {
                package: u64::MAX - 4,
                local: slot.index,
            };
            if self.enums.iter().any(|item| item.declaration == packet) {
                return Err(error(span, "quote fn result packet identity collision"));
            }
            let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
            let lowered = nir::inline_return::lower(
                body,
                delivered_type,
                nir::inline_return::Boundary {
                    packet,
                    slot,
                    exit,
                    result,
                    unreachable,
                },
                self.sources,
                expression.source,
                &mut fuel,
            );
            *self.fuel = fuel as u64;
            let (body, descriptor, empty_replacements) =
                lowered.map_err(|message| error(span, message))?;
            for source in empty_replacements {
                self.empty_replacements
                    .insert((source.start, source.end), packet);
            }
            self.internal_packets.insert(packet);
            self.enums.push(descriptor);
            body
        } else {
            body
        };
        let body = if let Some(effects) = effects {
            self.node(
                expression.ty.clone(),
                K::EffectComputation {
                    effects,
                    body: Box::new(body),
                },
                expression.source,
            )?
        } else {
            body
        };
        let body = self.node(
            body.ty.clone(),
            K::Block {
                statements,
                result: Some(Box::new(body)),
            },
            expression.source,
        )?;
        let prepared = self.node(
            body.ty.clone(),
            K::CallPreparation {
                statements: prepared,
                result: Some(Box::new(body)),
            },
            expression.source,
        )?;
        // Preserve the application boundary for cleanup keyed to the original
        // full expression; the preparation node itself is transparent.
        expression.kind = K::Block {
            statements: vec![],
            result: Some(Box::new(prepared)),
        };
        Ok(())
    }
}

impl n::Rewriter for Expansion<'_> {
    type Error = Error;

    #[inline(never)]
    fn expression(&mut self, expr: &mut n::Expr) -> Result<bool, Error> {
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(at(expr.source), "quote fn expansion budget exhausted"))?;
        if let K::Closure { captures, body, .. } = &mut expr.kind {
            for capture in captures {
                if let Some(value) = &mut capture.value {
                    value.rewrite_with(self)?;
                }
            }
            let previous = self.runtime;
            let previous_closure = self.in_closure;
            self.runtime = !expr.ty.is_comptime_callable();
            self.in_closure = true;
            let checked = Arc::make_mut(body).rewrite_with(self);
            self.in_closure = previous_closure;
            self.runtime = previous;
            checked?;
            return Ok(false);
        }
        if let K::Call { callee, .. } = &expr.kind {
            let target = match &callee.kind {
                K::Function(id) if self.templates.contains_key(id) => Some((*id, vec![])),
                K::FunctionInstance {
                    declaration,
                    arguments,
                } if self.templates.contains_key(declaration) => {
                    let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
                    let charged = arguments
                        .iter()
                        .try_for_each(|ty| nir::type_computations::charge_term(ty, &mut fuel));
                    *self.fuel = fuel as u64;
                    charged.map_err(|failure| error(at(expr.source), failure.to_string()))?;
                    Some((*declaration, arguments.clone()))
                }
                K::TraitMethod {
                    interface,
                    member,
                    subject,
                    interface_arguments,
                    method_arguments,
                } => {
                    if let Some(package) = self.selection {
                        let quoted = package
                            .traits
                            .iter()
                            .find(|item| item.declaration == *interface)
                            .and_then(|item| {
                                item.methods
                                    .iter()
                                    .find(|method| method.declaration == *member)
                            })
                            .is_some_and(|method| method.is_quote);
                        let concrete = std::iter::once(subject)
                            .chain(interface_arguments)
                            .chain(method_arguments)
                            .try_fold(true, |closed, ty| {
                                ty.rigid_parameters()
                                    .map(|parameters| closed && parameters.is_empty())
                            })
                            .map_err(|failure| error(at(expr.source), failure.to_string()))?;
                        // Staged method requirements can mention ordinary Type
                        // inputs. Select and prove them together with input
                        // binding, rather than proving an open telescope here.
                        let staged = matches!(callee.ty.callable_signature(),Term::Function {parameters,..}
                            if parameters.iter().any(|parameter|parameter.sort!=Sort::Runtime));
                        if quoted && concrete && !staged {
                            let mut fuel = usize::try_from(*self.fuel).unwrap_or(usize::MAX);
                            let selected = crate::materialization::resolve_quote_method_instance(
                                package, callee, &mut fuel,
                            );
                            *self.fuel = fuel as u64;
                            let selected =
                                selected.map_err(|failure| error(at(expr.source), failure))?;
                            if !self.templates.contains_key(&selected.body) {
                                return Err(error(
                                    at(expr.source),
                                    "selected quote method has no retained code template",
                                ));
                            }
                            Some((selected.body, selected.arguments))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some((id, types)) = target {
                if let Some(template) = self.templates.get(&id) {
                    if matches!(template.signature.callable_signature(),Term::Function {parameters,..}
                        if parameters.iter().any(|parameter|parameter.sort!=Sort::Runtime))
                    {
                        // Preserve the staged call until its caller has a closed
                        // input telescope. Ordinary argument subexpressions can
                        // still contain independent quote applications.
                        let K::Call { arguments, .. } = &mut expr.kind else {
                            unreachable!()
                        };
                        for argument in arguments {
                            argument.value.rewrite_with(self)?;
                        }
                        return Ok(false);
                    }
                    self.expand(expr, id, types)?;
                    return Ok(false);
                }
            }
        }
        if matches!(&expr.kind,K::Function(id) | K::FunctionInstance {declaration:id,..} if self.templates.contains_key(id))
        {
            return Err(error(
                at(expr.source),
                "quote fn values require code evidence; cannot escape as runtime function pointers",
            ));
        }
        Ok(true)
    }
}

pub(in crate::program) fn expand_quote_functions(
    output: &mut n::Package,
    hir: &hir::Package<'_>,
    identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    next: &mut u32,
    fuel: &mut u64,
) -> Result<(), Error> {
    let quotes = hir
        .owners()
        .filter_map(|(owner, info)| match &info.node.expect_item().kind {
            ItemKind::Fn(sig, _) if sig.modifiers.is_quote => {
                Some(declaration(hir, identities, owner))
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if quotes.is_empty() {
        return Ok(());
    }
    // Erasing an unused template must not erase its declared purity promise.
    purity::validate_runtime_promises(output, protocols)?;
    let mut functions = Vec::new();
    for function in std::mem::take(&mut output.functions) {
        if quotes.contains(&function.declaration) {
            output.quote_templates.push(function);
        } else {
            functions.push(function);
        }
    }
    let mut contextual_slots = BTreeMap::new();
    for (owner, info) in hir.owners() {
        if let ItemKind::Fn(sig, _) = &info.node.expect_item().kind
            && sig.modifiers.is_quote
        {
            let slots: BTreeSet<usize> = sig
                .params
                .iter()
                .enumerate()
                .filter_map(|(index, param)| {
                    (param.ty.is_implicit() && param.ty.is_quote()).then_some(index)
                })
                .collect();
            if !slots.is_empty() {
                contextual_slots.insert(declaration(hir, identities, owner), slots);
            }
        }
    }
    output.functions = functions;
    adapt_ordinary_quote_methods(output, next, fuel)?;
    expand_retained_quote_functions(output, &contextual_slots, next, fuel)
}

/// An ordinary trait contract promises a callable entry, even when its chosen
/// implementation was written as a quote function. Give that entry a distinct
/// identity; the original remains code evidence and cannot escape as a pointer.
/// Copying the checked body preserves its argument frame, contracts and return
/// boundary. Nested quote calls are expanded by the normal retained-code pass.
fn adapt_ordinary_quote_methods(
    output: &mut n::Package,
    next: &mut u32,
    fuel: &mut u64,
) -> Result<(), Error> {
    let mut adapters = BTreeMap::new();
    for implementation in &mut output.implementations {
        let Some(interface) = implementation.interface else {
            continue;
        };
        let Some(contract) = output
            .traits
            .iter()
            .find(|item| item.declaration == interface)
        else {
            continue;
        };
        for (name, body) in &mut implementation.methods {
            if !contract
                .methods
                .iter()
                .any(|method| method.name == *name && !method.is_quote)
            {
                continue;
            }
            let Some(template) = output
                .quote_templates
                .iter()
                .find(|template| template.declaration == *body)
            else {
                continue;
            };
            if let Some(adapter) = adapters.get(body) {
                *body = *adapter;
                continue;
            }
            let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
            let charged = nir::visit::expressions(template, &mut remaining, |expr, remaining| {
                nir::type_computations::charge_term(&expr.ty, remaining).map_err(|e| e.to_string())
            });
            *fuel = remaining as u64;
            charged.map_err(|message| error(at(template.body.source), message))?;
            let id = DeclarationId {
                package: u64::MAX - 7,
                local: *next,
            };
            *next = next.checked_add(1).ok_or_else(|| {
                error(
                    at(template.body.source),
                    "quote adapter identity space exhausted",
                )
            })?;
            if output
                .functions
                .iter()
                .any(|function| function.declaration == id)
            {
                return Err(error(
                    at(template.body.source),
                    "quote adapter identity collision",
                ));
            }
            let mut adapter = template.clone();
            adapter.declaration = id;
            adapter.name = format!("{}.callable", template.name);
            output.functions.push(adapter);
            if let Some(parameters) = output
                .function_parameters
                .iter()
                .find(|parameters| parameters.declaration == *body)
            {
                let mut parameters = parameters.clone();
                parameters.declaration = id;
                output.function_parameters.push(parameters);
            }
            adapters.insert(*body, id);
            *body = id;
        }
    }
    Ok(())
}

/// Expand checked direct applications using retained NIR templates. The
/// caller owns the surrounding transaction and the fresh identity allocator;
/// this pass does not need HIR or re-run declaration/type argument evaluation.
/// A later implementation-selection pass can reuse the same expansion path.
pub(in crate::program) fn expand_retained_quote_functions(
    output: &mut n::Package,
    contextual_slots: &BTreeMap<DeclarationId, BTreeSet<usize>>,
    next: &mut u32,
    fuel: &mut u64,
) -> Result<(), Error> {
    let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
    // Expansion removes the call node, so its responsibility boundary must be
    // checked while the original caller and callee are still visible.
    for function in output.functions.iter().chain(&output.quote_templates) {
        let result = nir::call_safety::validate(function, &mut remaining);
        *fuel = remaining as u64;
        result.map_err(|reason| error(at(function.body.source), reason.to_string()))?;
    }
    let Some(package_span) = output
        .functions
        .iter()
        .chain(&output.quote_templates)
        .next()
        .map(|function| at(function.body.source))
    else {
        output.quote_identity_cursor = *next;
        return Ok(());
    };
    let domains = output.declaration_parameter_domains(&mut remaining);
    *fuel = remaining as u64;
    let domains = domains.map_err(|reason| error(package_span, reason))?;
    let selection = if domains.values().any(|domain| *domain != p(P::Type))
        || output
            .traits
            .iter()
            .any(|item| item.methods.iter().any(|method| method.is_quote))
    {
        let mut remaining = usize::try_from(*fuel).unwrap_or(usize::MAX);
        for function in output.functions.iter().chain(&output.quote_templates) {
            nir::visit::expressions(function, &mut remaining, |expr, remaining| {
                nir::type_computations::charge_term(&expr.ty, remaining).map_err(|e| e.to_string())
            })
            .map_err(|message| error(at(function.body.source), message))?;
        }
        *fuel = remaining as u64;
        Some(output.clone())
    } else {
        None
    };
    let mut parameter_domains = Some(domains);
    let templates = output
        .quote_templates
        .iter()
        .map(|template| (template.declaration, template))
        .collect();
    let n::Package {
        ref mut functions,
        ref mut expansion_sources,
        ref mut enums,
        ref mut internal_return_packets,
        ref mut internal_return_empty_replacements,
        ref mut quote_identity_cursor,
        ..
    } = *output;
    for function in functions {
        let mut expansion = Expansion {
            parameter_domains: parameter_domains.take(),
            selection: selection.as_ref(),
            templates: &templates,
            contextual_slots: Some(contextual_slots),
            caller_env_parameters: &function.interface.env_parameters,
            caller_runtime_deliveries: &function.runtime_deliveries,
            in_closure: false,
            next,
            fuel,
            sources: expansion_sources,
            enums,
            internal_packets: internal_return_packets,
            empty_replacements: internal_return_empty_replacements,
            depth: 0,
            active: Vec::new(),
            runtime: function.phase == n::FunctionPhase::Runtime,
        };
        expansion.charge(&function.body, at(function.body.source))?;
        function.body.rewrite_with(&mut expansion)?;
        for value in function
            .interface
            .defaults
            .iter_mut()
            .map(|(_, value)| value)
            .chain(function.interface.preconditions.iter_mut())
            .chain(function.interface.postconditions.iter_mut())
        {
            value.rewrite_with(&mut expansion)?;
        }
        parameter_domains = expansion.parameter_domains;
    }
    *quote_identity_cursor = *next;
    Ok(())
}

/// Materialization uses the same hygiene and return lowering after substituting
/// an ordinary caller's Type arguments. The caller owns graph publication.
pub(crate) fn expand_materialized_quotes(
    package: &n::Package,
    expression: &mut n::Expr,
    caller_env_parameters: &[n::RuntimeEnvParameter],
    caller_runtime_deliveries: &BTreeMap<nir::RuntimeId, n::Expr>,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    enums: &mut Vec<n::Enum>,
    fuel: &mut usize,
) -> Result<(), String> {
    if package.quote_templates.is_empty() {
        return Ok(());
    }
    *fuel = fuel
        .checked_sub(package.quote_templates.len())
        .ok_or("quote template index budget exhausted")?;
    let templates = package
        .quote_templates
        .iter()
        .map(|template| (template.declaration, template))
        .collect();
    let mut remaining = *fuel as u64;
    let mut packets = BTreeSet::new();
    let mut empty = BTreeMap::new();
    let mut expansion = Expansion {
        parameter_domains: None,
        selection: Some(package),
        templates: &templates,
        contextual_slots: None,
        caller_env_parameters,
        caller_runtime_deliveries,
        in_closure: false,
        next,
        fuel: &mut remaining,
        sources,
        enums,
        internal_packets: &mut packets,
        empty_replacements: &mut empty,
        depth: 0,
        active: Vec::new(),
        runtime: true,
    };
    let result = expression
        .rewrite_with(&mut expansion)
        .map_err(|failure| failure.to_string());
    *fuel = remaining as usize;
    result
}

/// Expand one template after the shared staged-call binder has erased its
/// completed inputs. Its call key contains value inputs, not Type arguments;
/// the supplied body has already undergone both substitutions.
pub(crate) fn expand_bound_quote(
    package: &n::Package,
    template: &n::Function,
    expression: &mut n::Expr,
    caller_env_parameters: &[n::RuntimeEnvParameter],
    caller_runtime_deliveries: &BTreeMap<nir::RuntimeId, n::Expr>,
    in_closure: bool,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    enums: &mut Vec<n::Enum>,
    fuel: &mut usize,
) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(package.quote_templates.len())
        .ok_or("quote template index budget exhausted")?;
    let templates: BTreeMap<_, _> = package
        .quote_templates
        .iter()
        .map(|template| (template.declaration, template))
        .collect();
    let mut remaining = *fuel as u64;
    let mut packets = BTreeSet::new();
    let mut empty = BTreeMap::new();
    let mut expansion = Expansion {
        parameter_domains: None,
        selection: Some(package),
        templates: &templates,
        contextual_slots: None,
        caller_env_parameters,
        caller_runtime_deliveries,
        in_closure,
        next,
        fuel: &mut remaining,
        sources,
        enums,
        internal_packets: &mut packets,
        empty_replacements: &mut empty,
        depth: 0,
        active: Vec::new(),
        runtime: true,
    };
    let result = expansion
        .expand_template(expression, template, vec![])
        .map_err(|failure| failure.to_string());
    *fuel = remaining as usize;
    result
}

/// Optional ordinary-function expansion reuses quote's hygiene and return
/// boundary, without treating ordinary function values as quoted code.
pub(crate) fn inline_template(
    selection: Option<&n::Package>,
    template: &n::Function,
    expression: &mut n::Expr,
    types: Vec<Term>,
    caller_env_parameters: &[n::RuntimeEnvParameter],
    caller_runtime_deliveries: &BTreeMap<nir::RuntimeId, n::Expr>,
    in_closure: bool,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    enums: &mut Vec<n::Enum>,
    packets: &mut BTreeSet<DeclarationId>,
    empty: &mut BTreeMap<(u32, u32), DeclarationId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut remaining = *fuel as u64;
    let templates = BTreeMap::new();
    let mut expansion = Expansion {
        parameter_domains: None,
        selection,
        templates: &templates,
        contextual_slots: None,
        caller_env_parameters,
        caller_runtime_deliveries,
        in_closure,
        next,
        fuel: &mut remaining,
        sources,
        enums,
        internal_packets: packets,
        empty_replacements: empty,
        depth: 0,
        active: Vec::new(),
        runtime: true,
    };
    let result = expansion
        .expand_template(expression, template, types)
        .map_err(|failure| failure.to_string());
    *fuel = remaining as usize;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_holes_reject_distinct_hir_nodes_even_with_identical_synthetic_spans() {
        let mut holes = BTreeSet::new();
        let owner = hir::OwnerId::new(hir::LocalDefId::new(1));
        let first = hir::HirId::new(owner, hir::ItemLocalId::new(10));
        let second = hir::HirId::new(owner, hir::ItemLocalId::new(11));
        let synthetic = Span::default();

        insert_quote_hole(&mut holes, first, synthetic).expect("first hole succeeds");
        insert_quote_hole(&mut holes, first, synthetic).expect("same HirId does not double-count");
        let failure = insert_quote_hole(&mut holes, second, synthetic)
            .expect_err("second distinct HirId with identical synthetic span must fail");
        assert!(
            failure.message.contains("at most one"),
            "{}",
            failure.message
        );
    }
}
