//! Source checking, completion and specialization of open function annotations.
use super::*;

pub(super) fn annotation_source<'h>(
    sig: &hir::item::FnSig<'h>,
    slot: usize,
) -> Option<&'h Expr<'h>> {
    if slot == sig.params.len() {
        return sig.return_ty;
    }
    match sig.params.get(slot)?.ty.kind {
        TyParamKind::Optional(_, ty, _)
        | TyParamKind::Positional(ty)
        | TyParamKind::PositionalDependencyCatched(_, ty)
        | TyParamKind::Varadic(_, ty) => Some(ty),
        TyParamKind::Itself { .. } => None,
    }
}

#[derive(Clone)]
pub(in crate::program) enum Annotation {
    Known(Term),
    /// Complete Type data awaiting the domain of a particular delivery.
    Description(n::DescriptionLiteral),
    Pending(Arc<PreparedRequired>),
    /// The earlier input must deliver a Type. Its domain is pending, or it is
    /// an ordinary comptime parameter without a Type-sort ABI binder.
    InputType {
        slot: usize,
    },
    /// Ordinary-checked source awaiting completed input domains, not frozen NIR.
    Suspended {
        owner: OwnerId,
        slot: usize,
    },
}

impl From<CompletedType> for Annotation {
    fn from(value: CompletedType) -> Self {
        match value {
            CompletedType::Identity(ty) => Self::Known(ty),
            CompletedType::Description(data) => Self::Description(data),
        }
    }
}

impl Annotation {
    pub(super) fn awaits_inputs(&self) -> bool {
        matches!(
            self,
            Self::Pending(_) | Self::InputType { .. } | Self::Suspended { .. }
        )
    }

    pub(in crate::program) fn needs_instance(&self) -> bool {
        !matches!(self, Self::Known(_))
    }

    pub(super) fn description(&self) -> D {
        match self {
            Self::Known(ty) => D::Known(ty.clone()),
            Self::Description(_)
            | Self::Pending(_)
            | Self::InputType { .. }
            | Self::Suspended { .. } => D::Unknown,
        }
    }
}

impl std::fmt::Display for Annotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Known(ty) => write!(f, "(known {ty})"),
            Self::Description(data) => write!(f, "(completed {data})"),
            Self::Pending(code) => write!(f, "(deferred {code})"),
            Self::InputType { slot } => write!(f, "(deferred-input-type (slot {slot}))"),
            Self::Suspended { owner, slot } => write!(
                f,
                "(suspended-annotation (owner {}) (slot {slot}))",
                owner.def_id.raw()
            ),
        }
    }
}

impl Checker<'_, '_> {
    pub(super) fn check_suspended_annotation(
        &mut self,
        annotation: &Annotation,
        sig: &hir::item::FnSig<'_>,
        locals: &mut BTreeMap<n::LocalId, D>,
    ) -> Result<Option<(DeclarationId, Vec<n::TypePredicate>)>, Error> {
        let Annotation::Suspended { slot, owner } = annotation else {
            return Ok(None);
        };
        let source = annotation_source(sig, *slot)
            .ok_or_else(|| error(sig.span, "missing suspended annotation"))?;
        let H::Defer(value) = source.kind else {
            return Err(error(source.span, "suspended annotation must use defer"));
        };
        self.required_steps = self
            .required_steps
            .checked_sub(locals.len() as u64)
            .ok_or_else(|| error(source.span, "annotation input observation budget exhausted"))?;
        let pending_inputs = locals
            .iter()
            .filter_map(|(binding, domain)| matches!(domain, D::Unknown).then_some(*binding))
            .collect();
        let pending_domain = self.annotation_may_use_pending_inputs(value, &pending_inputs)?;
        let checked = self.with_staged_input(true, |this| {
            if pending_domain {
                return this
                    .describe_open_expression(
                        value,
                        &mut locals.clone(),
                        None,
                        Some(&D::Known(p(P::Type))),
                        0,
                    )
                    .map(|_| None);
            }
            let value = this.elaborate_known_region(value, Some(&D::Known(p(P::Type))))?;
            this.check_open_fragment(&value, source.span, true)?;
            Ok(Some(value))
        })?;
        if *slot == sig.params.len() {
            return Ok(None);
        }
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let facts = (|| {
            let target = if let Some(checked) = &checked {
                CallableClauseView::direct_call(checked, &mut fuel)?.and_then(|(id, operands)| {
                    self.source_declarations
                        .owner(id)
                        .map(|owner| (owner, Some(operands)))
                })
            } else if let H::Application(callee, _) = &value.kind {
                // Description checking has resolved and checked this call. It
                // did not deliver an operand packet. Never repeat a supplier
                // to obtain evidence after that check.
                let head = match &callee.kind {
                    H::NFApplication(head, _) => *head,
                    _ => callee,
                };
                match head.kind {
                    H::ResolvedDef { def_id, .. } => self
                        .package
                        .definition_owner(def_id)
                        .map(|owner| (owner, None)),
                    _ => None,
                }
            } else {
                None
            };
            let Some((target, operands)) = target else {
                return Ok(None);
            };
            if target == *owner {
                return Ok(None);
            }
            self.source_result_facts(target, operands, &[], &mut fuel)
        })();
        self.required_steps -= (before - fuel) as u64;
        facts.map_err(|e| error(source.span, e))
    }

    pub(in crate::program) fn signature_annotation(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<Annotation, Error> {
        let H::Defer(value) = &expr.kind else {
            return self.completed_type(expr).map(Annotation::from);
        };
        if let H::ResolvedFnParam(_, binding) = value.kind {
            if let Some(owner) = self.signature_owner {
                if let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
                    self.package.item(owner).map(|item| &item.kind)
                {
                    if let Some(slot) = sig.params.iter().position(|p| p.hir_id == binding) {
                        let domain = self
                            .signature_domains
                            .get(&owner)
                            .and_then(|domains| domains.get(slot));
                        if domain == Some(&(Sort::ComptimeValue, None))
                            || (sig.modifiers.is_comptime
                                && domain.is_some_and(|(sort, ty)| {
                                    *sort == Sort::Runtime
                                        && (ty.is_none() || ty.as_ref() == Some(&p(P::Type)))
                                }))
                        {
                            // An ordinary comptime Type input has no Type-sort
                            // ABI binder. Keep its value dependency explicit
                            // until the invocation supplies a concrete type.
                            return Ok(Annotation::InputType { slot });
                        }
                    }
                }
            }
        }
        if let Some(owner) = self.signature_owner {
            if let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
                self.package.item(owner).map(|item| &item.kind)
            {
                // Use the same complete dependency observation as definition
                // checking. An unsupported node in a partial readiness scan
                // must not silently erase inputs captured inside its body.
                self.required_steps = self
                    .required_steps
                    .checked_sub(sig.params.len() as u64)
                    .ok_or_else(|| {
                        error(expr.span, "signature input observation budget exhausted")
                    })?;
                let pending_inputs = sig
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, parameter)| {
                        let (sort, domain) = self.signature_domains.get(&owner)?.get(slot)?;
                        // Ordinary inputs of a comptime function remain value
                        // dependencies even when their domains are established.
                        // Source shape does not decide whether they are supplied.
                        let missing = if *sort == Sort::Runtime {
                            sig.modifiers.is_comptime
                        } else {
                            domain.is_none()
                        };
                        missing.then_some(local(parameter.hir_id))
                    })
                    .collect();
                if self.annotation_may_use_pending_inputs(value, &pending_inputs)? {
                    let slot = (0..=sig.params.len())
                        .find(|slot| {
                            annotation_source(sig, *slot).is_some_and(|ty| ty.hir_id == expr.hir_id)
                        })
                        .ok_or_else(|| error(expr.span, "missing suspended annotation source"))?;
                    return Ok(Annotation::Suspended { owner, slot });
                }
            }
        }
        // A direct Type input already has a stable symbolic identity. Explicit
        // structural constructors can contain computations (including BitVec
        // widths), so retain their prepared invocation until inputs are ready.
        if !matches!(
            value.kind,
            H::Application(..)
                | H::ExtendedApplication(..)
                | H::Index(..)
                | H::If(..)
                | H::When(_)
                | H::Match(..)
                | H::Block(_)
                | H::TyPtr(_)
                | H::TyOptional(_)
                | H::LiftType(_)
                | H::TyFn(_)
                | H::TyComptimeFn(_)
                | H::TyFnArrow(..)
        ) && !matches!(&value.kind, H::NFApplication(head, _)
            if matches!(&head.kind, H::Builtin(name) if name.as_str() == "BitVec"))
        {
            return self.completed_type(value).map(Annotation::from);
        }
        let prepared =
            self.prepare_required_expression(value, Some(&p(P::Type)), None, vec![], true)?;
        let abstract_values = self
            .visible_comptime_parameters(expr.span)?
            .into_iter()
            .filter(|declaration| {
                self.trait_input_domain(*declaration)
                    .is_some_and(|ty| ty != p(P::Type))
            })
            .collect::<Vec<_>>();
        if prepared.inputs.iter().any(|(_, _, value)| value.is_none())
            || (!abstract_values.is_empty()
                && self.open_computation_uses_clauses(&prepared, &abstract_values, expr.span)?)
        {
            // A value-clause identity authorizes symbolic transport, not
            // inspection of its contents. Keep explicit defer annotations
            // prepared until clause specialization supplies those values.
            Ok(Annotation::Pending(Arc::new(prepared)))
        } else {
            let (value, _) = self.invoke_required_publishing(
                prepared,
                Vec::new(),
                expr.hir_id.owner,
                expr.span,
            )?;
            CompletedType::from_value(value, expr.span).map(Annotation::from)
        }
    }

    /// Keep the checked supplier for call emission: admitting its type must
    /// not duplicate evaluation when the argument is later staged or emitted.
    pub(super) fn admit_supplied_annotation(
        &mut self,
        data: CompletedType,
        route: &arguments::RoutedArgument<'_>,
        call: &Expr<'_>,
        prepared: &mut Option<n::Expr>,
        staged: bool,
    ) -> Result<Term, Error> {
        self.admit_type_data(data, call.span, |this| {
            if let Some(value) = prepared.as_ref() {
                return Ok(value.ty.clone());
            }
            let input = route.expression(call);
            let checked = this.with_staged_input(staged, |this| this.expr(&input, None))?;
            let actual = checked.ty.clone();
            *prepared = Some(checked);
            Ok(actual)
        })
    }

    pub(super) fn instantiate_open_annotation(
        &mut self,
        owner: OwnerId,
        slot: usize,
        annotation: &Annotation,
        sig: &hir::item::FnSig<'_>,
        symbols: &[Option<Term>],
        parameters: &[Parameter],
        span: Span,
    ) -> Result<CompletedType, Error> {
        let domains = parameters
            .iter()
            .map(|p| (p.sort, Some(p.ty.clone())))
            .collect::<Vec<_>>();
        if !annotation.awaits_inputs() {
            return self
                .instantiate_open_annotation_in_scope(annotation, sig, symbols, &domains, span);
        }
        self.instantiate_open_annotation_with_live_in_scope(
            owner,
            slot,
            annotation,
            sig,
            symbols,
            &domains,
            &vec![None; sig.params.len()],
            span,
        )
    }

    pub(super) fn instantiate_open_annotation_with_live(
        &mut self,
        owner: OwnerId,
        slot: usize,
        annotation: &Annotation,
        sig: &hir::item::FnSig<'_>,
        symbols: &[Option<Term>],
        parameters: &[Parameter],
        live: &[Option<comptime_interpretor::Value>],
        span: Span,
    ) -> Result<CompletedType, Error> {
        let domains = parameters
            .iter()
            .map(|p| (p.sort, Some(p.ty.clone())))
            .collect::<Vec<_>>();
        self.instantiate_open_annotation_with_live_in_scope(
            owner, slot, annotation, sig, symbols, &domains, live, span,
        )
    }

    pub(super) fn instantiate_open_annotation_with_live_in_scope(
        &mut self,
        owner: OwnerId,
        slot: usize,
        annotation: &Annotation,
        sig: &hir::item::FnSig<'_>,
        symbols: &[Option<Term>],
        parameters: &[(Sort, Option<Term>)],
        live: &[Option<comptime_interpretor::Value>],
        span: Span,
    ) -> Result<CompletedType, Error> {
        // A checked computation already selected its captures, including inputs
        // referenced only in another capture's domain. Source-only annotations
        // still require the caller environment until their source is checked.
        let needs_call_world = match annotation {
            Annotation::Known(_) | Annotation::Description(_) => false,
            Annotation::Pending(prepared) => {
                prepared.definition_identity_cursor() != self.definition_identities.next()
                    || prepared.needs_call_world(symbols, live, &mut self.required_steps, span)?
            }
            Annotation::InputType { .. } | Annotation::Suspended { .. } => {
                live.iter().any(Option::is_some)
                    || !required::Discovery::Declarations
                        .frozen_references(
                            symbols.iter().flatten(),
                            &mut self.required_steps,
                            span,
                        )?
                        .is_empty()
            }
        };
        if !needs_call_world {
            return self
                .instantiate_open_annotation_in_scope(annotation, sig, symbols, parameters, span);
        }
        let source =
            annotation_source(sig, slot).ok_or_else(|| error(span, "missing live annotation"))?;
        let H::Defer(value) = source.kind else {
            return Err(error(span, "live annotation must use defer"));
        };
        let assumptions = self.context.assumptions().to_vec();
        let ty = self.in_open_function_scope(owner, None, |this| {
            this.import_closed_caller_assumptions(assumptions);
            this.default_input_scope = Some(parameters.into());
            this.default_input_owner = Some(owner);
            let mut graph = Vec::new();
            for (index, (parameter, (_, domain))) in sig.params.iter().zip(parameters).enumerate() {
                let Some(domain) = domain else {
                    continue;
                };
                let binding = local(parameter.hir_id);
                this.locals.insert(binding, domain.clone());
                if let Some(value) = &live[index] {
                    graph.push((binding, domain.clone(), value.clone()));
                } else if let Some(symbol) = &symbols[index] {
                    this.input_facts
                        .specialized
                        .push((binding, domain.clone(), symbol.clone()));
                    if *domain == p(P::Type) {
                        this.input_facts.types.insert(binding, symbol.clone());
                    } else {
                        this.input_facts
                            .values
                            .insert(binding, (symbol.clone(), domain.clone()));
                    }
                }
            }
            let (value, _) =
                this.required_expression_with_live_inputs(value, Some(&p(P::Type)), graph)?;
            CompletedType::from_value(value, span)
        })?;
        self.specialize_completed_annotation(ty, symbols, span)
    }

    fn specialize_completed_annotation(
        &mut self,
        data: CompletedType,
        symbols: &[Option<Term>],
        span: Span,
    ) -> Result<CompletedType, Error> {
        let CompletedType::Identity(ty) = data else {
            return Ok(data);
        };
        let ty = nominal_fields::metered(&mut self.required_steps, |fuel| {
            ty.substitute_partial_bounded(symbols, fuel)
        })
        .map_err(|failure| error(span, failure.to_string()))?;
        self.callable_input_view(&ty, span)
            .map(CompletedType::Identity)
    }

    fn instantiate_open_annotation_in_scope(
        &mut self,
        annotation: &Annotation,
        sig: &hir::item::FnSig<'_>,
        symbols: &[Option<Term>],
        parameters: &[(Sort, Option<Term>)],
        span: Span,
    ) -> Result<CompletedType, Error> {
        let ty = match annotation {
            Annotation::Known(ty) => CompletedType::Identity(ty.clone()),
            Annotation::Description(data) => CompletedType::Description(data.clone()),
            Annotation::Suspended { owner, slot } => {
                let source = annotation_source(sig, *slot)
                    .ok_or_else(|| error(span, "missing suspended annotation"))?;
                let H::Defer(value) = source.kind else {
                    return Err(error(span, "suspended annotation must use defer"));
                };
                // Invocation inputs can retain the caller's opaque Type
                // identities. Their established premises remain available
                // while evaluating this resumed annotation, without changing
                // the definition-side check of the open function body.
                let caller_assumptions = self.context.assumptions().to_vec();
                self.in_open_function_scope(*owner, None, |this| {
                    this.import_closed_caller_assumptions(caller_assumptions);
                    this.default_input_scope = Some(parameters.into());
                    this.default_input_owner = Some(*owner);
                    for ((parameter, (sort, domain)), symbol) in
                        sig.params.iter().zip(parameters).zip(symbols)
                    {
                        let Some(domain) = domain else {
                            continue;
                        };
                        let binding = local(parameter.hir_id);
                        this.locals.insert(binding, domain.clone());
                        if let Some(symbol) = symbol {
                            this.input_facts.specialized.push((
                                binding,
                                domain.clone(),
                                symbol.clone(),
                            ));
                            if *sort == Sort::Type
                                || (sig.modifiers.is_comptime && *domain == p(P::Type))
                            {
                                this.input_facts.types.insert(binding, symbol.clone());
                            } else if *sort != Sort::Runtime || sig.modifiers.is_comptime {
                                this.input_facts
                                    .values
                                    .insert(binding, (symbol.clone(), domain.clone()));
                            }
                        }
                    }
                    this.completed_type(value)
                })?
            }
            Annotation::InputType { slot } => {
                let ty = symbols.get(*slot).and_then(Option::as_ref).ok_or_else(|| {
                    error(span, "deferred type awaits its earlier comptime input")
                })?;
                if ty.sort() != Sort::Type {
                    return Err(error(span, "deferred type input did not produce a Type"));
                }
                CompletedType::Identity(ty.clone())
            }
            Annotation::Pending(computation) => {
                let invocation =
                    computation.for_annotation_call(symbols, &mut self.required_steps, span)?;
                let owner = sig
                    .params
                    .first()
                    .map(|parameter| parameter.hir_id.owner)
                    .or_else(|| sig.return_ty.map(|value| value.hir_id.owner))
                    .ok_or_else(|| error(span, "deferred annotation has no source owner"))?;
                let (value, _) =
                    self.invoke_required_publishing(invocation, Vec::new(), owner, span)?;
                CompletedType::from_value(value, span)?
            }
        };
        self.specialize_completed_annotation(ty, symbols, span)
    }
}

impl Checker<'_, '_> {
    pub(super) fn import_closed_caller_assumptions(
        &mut self,
        assumptions: Vec<crate::ptc::Formula>,
    ) {
        for assumption in assumptions {
            if !assumption.has_bound_or_hole() {
                self.context.assume(assumption);
            }
        }
    }
}
