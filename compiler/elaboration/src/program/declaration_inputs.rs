//! Shared source-order input plans, independent of the eventual type constructor.
use super::*;
mod progress;

/// Consumers that cannot yet suspend frontend checking require ordinary typed
/// expressions. Both consumers retain the same routing and execution plan.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InputChecking {
    Contextual,
    Typed,
}

pub(super) struct PreparedDeclarationInputs {
    pub arguments: Vec<n::NormalFormArgument>,
    pub plan: n::NormalFormInputPlan,
    pub formation: FormationInputs,
}

/// Typed consumers receive ordinary NIR only, with execution statements separate
/// from the declaration-order result slots and their formation evidence.
pub(super) struct CheckedDeclarationInputs {
    pub arguments: Vec<n::Expr>,
    pub statements: Vec<n::Expr>,
    pub formation: FormationInputs,
}

impl std::fmt::Display for CheckedDeclarationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(checked-declaration-inputs (statements")?;
        for statement in &self.statements {
            write!(f, " {statement}")?;
        }
        f.write_str(") (arguments")?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str(") (identities")?;
        for term in &self.formation.terms {
            write!(f, " {term}")?;
        }
        f.write_str(") (deferred")?;
        for parameter in &self.formation.deferred {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (environment")?;
        for (parameter, value) in &self.formation.environment {
            write!(f, " (input {parameter} {value})")?;
        }
        f.write_str("))")
    }
}

impl PreparedDeclarationInputs {
    fn into_typed(
        self,
        parameters: &[(DeclarationId, Sort, Term)],
        span: Span,
    ) -> Result<CheckedDeclarationInputs, Error> {
        let n::NormalFormInputPlan {
            order,
            preparations,
            bindings,
        } = self.plan;
        let mut sources = self.arguments.into_iter().map(Some).collect::<Vec<_>>();
        let mut outputs = vec![None; sources.len()];
        let mut preparations = preparations.into_iter().peekable();
        let mut statements = Vec::new();
        for (position, slot) in order.into_iter().enumerate() {
            while preparations
                .peek()
                .is_some_and(|preparation| preparation.before <= position)
            {
                statements.push(preparations.next().unwrap().expression);
            }
            let Some(n::NormalFormArgument::Value(value)) = sources[slot].take() else {
                return Err(error(span, "typed declaration input was not checked"));
            };
            let parameter = parameters[slot].0;
            let ty = value.ty.clone();
            let output = if matches!(value.kind, K::TypeValue(_)) {
                value
            } else {
                let location = value.source;
                statements.push(n::Expr {
                    source: location,
                    ty: if is_never(&ty) {
                        p(P::Never)
                    } else {
                        p(P::Void)
                    },
                    kind: K::Let {
                        binding: bindings[slot],
                        value: Box::new(value),
                    },
                });
                n::Expr {
                    source: location,
                    ty,
                    kind: if parameters[slot].2 == p(P::Type)
                        && !self.formation.deferred.contains(&parameter)
                    {
                        K::TypeValue(self.formation.terms[slot].clone())
                    } else {
                        K::Local(bindings[slot])
                    },
                }
            };
            outputs[slot] = Some(output);
        }
        statements.extend(preparations.map(|preparation| preparation.expression));
        Ok(CheckedDeclarationInputs {
            arguments: outputs.into_iter().map(Option::unwrap).collect(),
            statements,
            formation: self.formation,
        })
    }

    pub fn diverges(&self) -> bool {
        self.arguments
            .iter()
            .any(n::NormalFormArgument::is_diverging)
            || self
                .plan
                .preparations
                .iter()
                .any(|p| is_never(&p.expression.ty))
    }
}

impl std::fmt::Display for PreparedDeclarationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(prepared-declaration-inputs (arguments")?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") {} (identities", self.plan)?;
        for term in &self.formation.terms {
            write!(f, " {term}")?;
        }
        f.write_str(") (deferred")?;
        for parameter in &self.formation.deferred {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (environment")?;
        for (parameter, value) in &self.formation.environment {
            write!(f, " (input {parameter} {value})")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone)]
enum RoutedDeclarationInput<'a, 'h> {
    Source(&'a Expr<'h>),
    Prepared(n::Expr),
    Collected(Vec<n::NormalFormArgument>),
}

/// Source references awaiting collection checking, not another expression IR.
/// `before` records the number of fixed inputs preceding this source input.
struct PendingDeclarationCollection<'a, 'h> {
    source_index: usize,
    before: usize,
    input: &'a Expr<'h>,
    spread: bool,
}

impl std::fmt::Display for PendingDeclarationCollection<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(pending-declaration-collection (source-index {}) (before {}) (spread {}) {})",
            self.source_index,
            self.before,
            self.spread,
            source(self.input.span)
        )
    }
}

/// A caller-scoped domain can remain generic while all of this call's
/// dependencies are supplied. Pending refers to callee inputs, not rigidity.
struct DeclarationInputDomain {
    ty: Term,
    pending: bool,
}
impl std::fmt::Display for DeclarationInputDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-domain (pending {}) (type {}))",
            self.pending, self.ty
        )
    }
}

impl Checker<'_, '_> {
    // Inspect the original callee domain before substitution. A supplied
    // caller parameter may have the same declaration identity as another
    // callee input in a recursive call; it is not another pending dependency.
    fn declaration_domain_pending(
        &mut self,
        domain: &Term,
        parameters: &[(DeclarationId, Sort, Term)],
        completed: &[DeclarationId],
        span: Span,
    ) -> Result<bool, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(completed.len() as u64)
            .ok_or_else(|| error(span, "input domain completion budget exhausted"))?;
        let completed = completed.iter().copied().collect::<BTreeSet<_>>();
        input_observation::pending_domain_inputs(
            domain,
            parameters
                .iter()
                .map(|(id, _, _)| (*id, completed.contains(id))),
            &mut self.required_steps,
            span,
        )
        .map(|pending| !pending.is_empty())
    }

    fn prepare_declaration_collection_input(
        &mut self,
        value: n::Expr,
        before: usize,
        preparations: &mut Vec<n::NormalFormPreparation>,
        known_bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let binding = self.spread_binding(span)?;
        let known = self.preserved_input_identity(
            &value,
            known_bindings
                .iter()
                .map(|(binding, term)| (*binding, term.as_ref())),
            span,
        )?;
        if let Some(known) = known {
            known_bindings.insert(binding, std::sync::Arc::new(known));
        }

        let ty = value.ty.clone();
        self.locals.insert(binding, ty.clone());
        let location = value.source;
        let diverges = is_never(&ty);
        let tuple_ty = if diverges {
            p(P::Never)
        } else {
            self.tuple_value_type(vec![ty.clone()], span)?
        };
        preparations.push(n::NormalFormPreparation {
            before,
            expression: n::Expr {
                source: location,
                ty: if diverges { p(P::Never) } else { p(P::Void) },
                kind: K::LetPattern {
                    pattern: n::Pattern::Tuple(vec![n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    }]),
                    value: Box::new(n::Expr {
                        source: location,
                        ty: tuple_ty,
                        kind: K::Tuple(vec![value]),
                    }),
                    guard: None,
                    otherwise: None,
                },
            },
        });
        Ok(n::Expr {
            source: location,
            ty,
            kind: K::Local(binding),
        })
    }

    // Check domain dependencies without invoking input code. A later source
    // input can become checked first, but Progress retains its execution slot.
    fn declaration_input_domain(
        &mut self,
        slot: usize,
        parameters: &[(DeclarationId, Sort, Term)],
        routed: &mut [Option<RoutedDeclarationInput<'_, '_>>],
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        progress: &mut progress::Progress,
        span: Span,
    ) -> Result<DeclarationInputDomain, Error> {
        let domain = &parameters[slot].2;
        let mut needed = domain
            .rigid_parameters()
            .map_err(|_| error(span, "invalid normal-form input domain"))?;
        for previous in (0..slot).rev() {
            if needed.contains_key(&parameters[previous].0) {
                needed.extend(
                    parameters[previous]
                        .2
                        .rigid_parameters()
                        .map_err(|_| error(span, "invalid normal-form input dependency"))?,
                );
            }
        }
        for previous in 0..slot {
            if progress.is_prepared(previous)
                || !needed.contains_key(&parameters[previous].0)
                || !matches!(
                    routed[previous],
                    Some(RoutedDeclarationInput::Source(_) | RoutedDeclarationInput::Prepared(_))
                )
            {
                continue;
            }
            if self.declaration_domain_pending(
                &parameters[previous].2,
                parameters,
                progress.observed_parameters(),
                span,
            )? {
                continue;
            }
            let expected = self.instantiate_input_types(
                &parameters[previous].2,
                progress.observed_parameters(),
                progress.observed_values(),
                span,
            )?;
            let value = match routed[previous].take().expect("routed dependency") {
                RoutedDeclarationInput::Source(input) => self.expr(input, Some(&expected))?,
                RoutedDeclarationInput::Prepared(value) => {
                    self.require_expression(&value.ty, &expected, span)?;
                    value
                }
                RoutedDeclarationInput::Collected(_) => unreachable!("fixed dependency input"),
            };
            let identity = self.preserved_input_identity(
                &value,
                bindings
                    .iter()
                    .map(|(binding, term)| (*binding, term.as_ref())),
                span,
            )?;
            let ty = value.ty.clone();
            self.record_declaration_input(
                progress,
                previous,
                n::NormalFormArgument::Value(value),
                identity,
                ty,
                bindings,
                span,
            )?;
        }
        let pending = self.declaration_domain_pending(
            domain,
            parameters,
            progress.observed_parameters(),
            span,
        )?;
        let ty = self.instantiate_input_types(
            domain,
            progress.observed_parameters(),
            progress.observed_values(),
            span,
        )?;
        Ok(DeclarationInputDomain { ty, pending })
    }

    fn record_declaration_input(
        &mut self,
        progress: &mut progress::Progress,
        slot: usize,
        value: n::NormalFormArgument,
        identity: Option<Term>,
        ty: Term,
        bindings: &mut BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<(), Error> {
        let known = identity
            .as_ref()
            .map(|known| self.copy_alias_type(known, span).map(std::sync::Arc::new))
            .transpose()?;
        progress.record(slot, value, identity, &mut self.required_steps, span)?;
        if let Some(known) = known {
            bindings.insert(progress.binding(slot), known);
        }
        self.locals.insert(progress.binding(slot), ty);
        Ok(())
    }

    /// Prepare explicit inputs in source order, then omitted inputs in their
    /// declaration telescope. All reuse goes through fresh local bindings.
    pub(super) fn planned_declaration_inputs(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: &[(DeclarationId, Sort, Term)],
        defaults: Option<&nominal::InputEnvironment<'_>>,
    ) -> Result<PreparedDeclarationInputs, Error> {
        self.plan_declaration_inputs(
            owner,
            inputs,
            span,
            parameters,
            defaults,
            InputChecking::Contextual,
        )
    }

    pub(super) fn planned_typed_declaration_inputs(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: &[(DeclarationId, Sort, Term)],
        defaults: Option<&nominal::InputEnvironment<'_>>,
    ) -> Result<CheckedDeclarationInputs, Error> {
        self.plan_declaration_inputs(
            owner,
            inputs,
            span,
            parameters,
            defaults,
            InputChecking::Typed,
        )?
        .into_typed(parameters, span)
    }

    fn plan_declaration_inputs(
        &mut self,
        owner: OwnerId,
        inputs: &[Arg<'_>],
        span: Span,
        parameters: &[(DeclarationId, Sort, Term)],
        defaults: Option<&nominal::InputEnvironment<'_>>,
        checking: InputChecking,
    ) -> Result<PreparedDeclarationInputs, Error> {
        let clauses = self.type_clause_params(owner);
        if parameters.len() != clauses.len() {
            return Err(error(span, "declaration input telescope arity mismatch"));
        }
        let saved_locals = self.locals.clone();
        let saved_bindings = self.normal_default_bindings.clone();
        let saved_types = self.alias_substitution.clone();
        let saved_type_values = self.alias_value_bindings.clone();
        let saved_context = self.context.clone();
        let saved_owner = self.signature_owner;
        let saved_mode = self.in_required_comptime;
        let caller_scope = self.call_inference_domains();
        let caller_owner = self.default_input_owner.or(self.signature_owner);
        let saved_input_owner = self.default_input_owner;
        self.default_input_owner = caller_owner;
        let saved_input_scope = self.default_input_scope.replace(caller_scope);
        let result = (|| {
            // Layout hints establish only positions. Actual packet checking
            // still validates the tuple and its element types. Do not expose
            // a head's unresolved Type binders as caller-side expectations.
            let positional_types = parameters
                .iter()
                .zip(&clauses)
                .filter(|(_, clause)| {
                    !matches!(
                        clause.kind,
                        hir::clause::ClauseParamKind::Optional(..)
                            | hir::clause::ClauseParamKind::Varadic(..)
                    )
                })
                .map(|((_, _, ty), _)| ty.clone())
                .collect::<Vec<_>>();
            let collector = self.clause_has_collector(owner).then(|| clauses.len() - 1);
            let whole_collector = collector.is_some_and(|slot| {
                inputs.iter().any(|input|
                matches!(input, Arg::Named(name, _) if name.name == clauses[slot].name.name))
            });
            let mut layout_inputs = inputs.to_vec();
            let mut list_spreads = BTreeSet::new();
            let mut remaining = Some(positional_types.len());
            let mut layout_fuel = 10_000;
            if let Some(slot) = collector.filter(|_| !whole_collector) {
                for (index, input) in inputs.iter().enumerate() {
                    match input {
                        Arg::Positional(value) if remaining == Some(0) => {
                            layout_inputs[index] = Arg::Named(clauses[slot].name, value);
                        }
                        Arg::Expand(value) if remaining == Some(0) => {
                            layout_inputs[index] = Arg::Named(clauses[slot].name, value);
                            list_spreads.insert(index);
                        }
                        Arg::Positional(_) => remaining = remaining.map(|n| n.saturating_sub(1)),
                        Arg::Expand(value) => {
                            remaining =
                                match (remaining, self.spread_width(value, &mut layout_fuel, 0)?) {
                                    (Some(left), Some(width)) if width <= left => {
                                        Some(left - width)
                                    }
                                    _ => None,
                                };
                        }
                        _ => {}
                    }
                }
            }
            // Flatten only literal list spreads already routed to the
            // collector. Their elements then use the ordinary positional
            // preparation path, preserving effects before later arguments.
            if list_spreads.iter().any(|index| matches!(&inputs[*index], Arg::Expand(value) if matches!(value.kind, H::List(_)))) {
                let mut expanded = Vec::new();
                for (index, input) in inputs.iter().enumerate() {
                    self.required_steps = self.required_steps.checked_sub(1)
                        .ok_or_else(|| error(span, "normal-form collection routing budget exhausted"))?;
                    if list_spreads.contains(&index)
                        && let Arg::Expand(value) = input
                        && let H::List(elements) = value.kind
                    {
                        self.required_steps = self.required_steps.checked_sub(elements.len() as u64)
                            .ok_or_else(|| error(span, "normal-form collection routing budget exhausted"))?;
                        expanded.extend(elements.iter().map(Arg::Positional));
                    } else {
                        expanded.push(input.clone());
                    }
                }
                return self.plan_declaration_inputs(owner, &expanded, span, parameters, defaults, checking);
            }
            let packet_contexts = if inputs.iter().any(|input| matches!(input, Arg::Expand(_))) {
                self.spread_packet_layout(&layout_inputs, &positional_types, span)?
            } else {
                vec![None; inputs.len()]
            };
            let mut routed = vec![None; parameters.len()];
            let mut preparations = vec![Vec::new(); inputs.len()];
            let mut order = Vec::new();
            let mut routing = clause_inputs::ClauseInputRouting::new(&clauses, inputs, span)?;
            let mut pending_collection = Vec::new();
            let mut prepared_bindings: BTreeMap<n::LocalId, std::sync::Arc<Term>> = BTreeMap::new();
            for (input_index, input) in inputs.iter().enumerate() {
                if let Arg::Expand(packet) = input {
                    if list_spreads.contains(&input_index) {
                        routing.slot(None, packet.span)?;
                        pending_collection.push(PendingDeclarationCollection {
                            source_index: input_index,
                            before: order.len(),
                            input: packet,
                            spread: true,
                        });
                        continue;
                    }

                    let context = if let Some(fields) = &packet_contexts[input_index] {
                        use crate::calls::InputContext as C;
                        let pending = fields
                            .iter()
                            .map(|ty| self.declaration_domain_pending(ty, parameters, &[], span))
                            .collect::<Result<Vec<_>, _>>()?;
                        if pending.iter().any(|pending| *pending) {
                            let fields = fields
                                .iter()
                                .zip(pending)
                                .map(|(ty, pending)| {
                                    if pending {
                                        C::Unknown
                                    } else {
                                        C::Known(ty.clone())
                                    }
                                })
                                .collect();
                            Some(match self.bound_tuple_head(packet.span)? {
                                Some(head) => C::NormalTuple {
                                    head,
                                    fields,
                                    list_domain: self.bound_tuple_list_domain(),
                                },
                                None => C::Tuple(fields),
                            })
                        } else {
                            Some(C::Known(
                                self.tuple_value_type(fields.clone(), packet.span)?,
                            ))
                        }
                    } else {
                        None
                    };
                    let value = match context {
                        Some(context) => self.input_context_expression(packet, &context)?,
                        None => self.expr(packet, None)?,
                    };
                    let fields = self.prepared_packet_fields(
                        &value,
                        &layout_inputs,
                        input_index,
                        &positional_types,
                        packet.span,
                    )?;
                    let observed = self.preserved_input_identity(
                        &value,
                        prepared_bindings
                            .iter()
                            .map(|(binding, term)| (*binding, term.as_ref())),
                        packet.span,
                    )?;
                    let mut observed_fields = match observed {
                        Some(Term::TupleValue(values)) if values.len() == fields.len() => {
                            Some(values.into_iter())
                        }
                        _ => None,
                    };
                    let before = order.len();
                    let mut pattern = Vec::new();
                    for ty in fields {
                        let slot = routing.packet_slot(packet.span)?;
                        let binding = self.spread_binding(packet.span)?;
                        if let Some(known) = observed_fields.as_mut().and_then(Iterator::next) {
                            prepared_bindings.insert(binding, std::sync::Arc::new(known));
                        }
                        self.locals.insert(binding, ty.clone());
                        pattern.push(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        });
                        let input = n::Expr {
                            source: value.source,
                            ty,
                            kind: K::Local(binding),
                        };
                        let cell = routed
                            .get_mut(slot)
                            .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
                        if cell
                            .replace(RoutedDeclarationInput::Prepared(input))
                            .is_some()
                        {
                            return Err(error(span, "duplicate normal-form argument"));
                        }
                        order.push(slot);
                    }
                    preparations[input_index].push(n::NormalFormPreparation {
                        before,
                        expression: n::Expr {
                            source: n::SourceRange {
                                start: value.source.end,
                                end: value.source.end,
                            },
                            ty: if is_never(&value.ty) {
                                p(P::Never)
                            } else {
                                p(P::Void)
                            },
                            kind: K::LetPattern {
                                pattern: n::Pattern::Tuple(pattern),
                                value: Box::new(value),
                                guard: None,
                                otherwise: None,
                            },
                        },
                    });
                    continue;
                }
                let (slot, value) = match input {
                    Arg::Positional(value) => {
                        let slot = routing.slot(None, span)?;
                        if collector == Some(slot) {
                            pending_collection.push(PendingDeclarationCollection {
                                source_index: input_index,
                                before: order.len(),
                                input: value,
                                spread: false,
                            });
                            continue;
                        }
                        (slot, *value)
                    }
                    Arg::Named(name, value) => (routing.slot(Some(*name), span)?, *value),
                    _ => {
                        return Err(error(
                            span,
                            "normal-form input expansion is not yet implemented",
                        ));
                    }
                };
                let cell = routed
                    .get_mut(slot)
                    .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
                if cell
                    .replace(RoutedDeclarationInput::Source(value))
                    .is_some()
                {
                    return Err(error(span, "duplicate normal-form argument"));
                }
                order.push(slot);
            }
            // Routing fixes execution order before any dependency-driven checks.
            // Nonempty collection elements occupy the final collected slot; their
            // earlier effects remain in preparations at their source positions.
            if !pending_collection.is_empty() {
                order.push(collector.expect("collected inputs have a collector"));
            }
            let bindings = (0..parameters.len())
                .map(|_| self.spread_binding(span))
                .collect::<Result<Vec<_>, _>>()?;
            let mut progress = progress::Progress::new(
                parameters,
                bindings,
                order,
                &mut self.required_steps,
                span,
            )?;
            // A contiguous source suffix can execute inside the final
            // collection. Earlier inputs need preparations at their original
            // positions, including around zero-width packet producers.
            let mut tail_start = inputs.len();
            for input in pending_collection.iter().rev() {
                if input.source_index + 1 != tail_start {
                    break;
                }
                tail_start = input.source_index;
            }
            // Routing is complete, and collection temporaries cannot be
            // referenced by declaration inputs. Observe the same receiver
            // domain once instead of rechecking its telescope per element.
            let collection_domain = if pending_collection.is_empty() {
                None
            } else {
                let slot = collector.expect("collected input has a collector");
                Some(self.declaration_input_domain(
                    slot,
                    parameters,
                    &mut routed,
                    &mut prepared_bindings,
                    &mut progress,
                    span,
                )?)
            };
            let mut collected = Vec::with_capacity(pending_collection.len());
            for pending in pending_collection {
                let domain = collection_domain
                    .as_ref()
                    .expect("observed collection domain");
                let expected = if pending.spread {
                    &domain.ty
                } else {
                    self.list_element_type(&domain.ty).ok_or_else(|| {
                        error(
                            pending.input.span,
                            "variadic input requires a bound meta.List<T>",
                        )
                    })?
                };
                let dependent = domain.pending;
                let at_tail = pending.source_index >= tail_start;
                let value = if dependent && !pending.spread && checking == InputChecking::Contextual
                {
                    if at_tail {
                        collected.push(self.dependent_normal_argument(pending.input, 0)?);
                        continue;
                    }
                    // Contextual literals have no producer effects. General
                    // contextual control flow still needs a resumable source
                    // preparation before it can cross another input's slot.
                    if matches!(pending.input.kind, H::Unit) {
                        collected.push(n::NormalFormArgument::Unit {
                            source: source(pending.input.span),
                        });
                        continue;
                    }
                    match self.checking_attempt(
                        pending.input.span,
                        false,
                        CheckingDisposition::Commit,
                        |checker| checker.expr(pending.input, None),
                    )? {
                        Some(value) => value,
                        None => match self.dependent_normal_argument(pending.input, 0)? {
                            argument @ (n::NormalFormArgument::Integer { .. }
                            | n::NormalFormArgument::Null { .. }) => {
                                collected.push(argument);
                                continue;
                            }
                            _ => {
                                return Err(context_error(
                                    pending.input.span,
                                    "structured collection input requires an element type context",
                                ));
                            }
                        },
                    }
                } else {
                    self.expr(pending.input, (!dependent).then_some(expected))?
                };
                let value = if at_tail {
                    value
                } else {
                    self.prepare_declaration_collection_input(
                        value,
                        pending.before,
                        &mut preparations[pending.source_index],
                        &mut prepared_bindings,
                        span,
                    )?
                };
                collected.push(if pending.spread {
                    n::NormalFormArgument::ListSpread(value)
                } else {
                    n::NormalFormArgument::Value(value)
                });
            }
            let preparations: Vec<_> = preparations.into_iter().flatten().collect();
            if !collected.is_empty() {
                let slot = collector.expect("collected inputs have a collector");
                routed[slot] = Some(RoutedDeclarationInput::Collected(collected));
            }
            while let Some(slot) = progress.current() {
                if routed[slot].is_none()
                    && matches!(
                        clauses[slot].kind,
                        hir::clause::ClauseParamKind::Varadic(..)
                    )
                {
                    // An unsupplied collector has zero elements, not a missing
                    // required input. Keep it in the normal-form telescope.
                    let ty = self.instantiate_input_types(
                        &parameters[slot].2,
                        progress.observed_parameters(),
                        progress.observed_values(),
                        span,
                    )?;
                    if self.list_element_type(&ty).is_none() {
                        return Err(error(
                            span,
                            "normal-form variadic collector requires meta.List<T>",
                        ));
                    }
                    let identity =
                        self.checked_collected_list_in(&[], &ty, &prepared_bindings, span)?;
                    self.locals.insert(progress.binding(slot), ty.clone());
                    progress.record(
                        slot,
                        n::NormalFormArgument::Value(n::Expr {
                            source: source(span),
                            ty,
                            kind: K::TypeList(Vec::new()),
                        }),
                        identity,
                        &mut self.required_steps,
                        span,
                    )?;
                    continue;
                }
                let mut dependent = self.declaration_domain_pending(
                    &parameters[slot].2,
                    parameters,
                    progress.observed_parameters(),
                    span,
                )?;
                let ty = if matches!(&routed[slot], Some(RoutedDeclarationInput::Source(_)))
                    && dependent
                {
                    let domain = self.declaration_input_domain(
                        slot,
                        parameters,
                        &mut routed,
                        &mut prepared_bindings,
                        &mut progress,
                        span,
                    )?;
                    dependent = domain.pending;
                    domain.ty
                } else {
                    self.instantiate_input_types(
                        &parameters[slot].2,
                        progress.observed_parameters(),
                        progress.observed_values(),
                        span,
                    )?
                };
                let explicit = routed[slot].as_ref();
                let input = match explicit {
                    Some(RoutedDeclarationInput::Source(input)) => Some(*input),
                    Some(
                        RoutedDeclarationInput::Prepared(_) | RoutedDeclarationInput::Collected(_),
                    ) => None,
                    None => {
                        // Defaults use the declaration environment; explicit inputs
                        // retain the caller environment, including member projections.
                        if let Some(defaults) = defaults {
                            self.alias_substitution = defaults.substitution.clone();
                            self.alias_value_bindings = defaults.bindings.clone();
                            self.normal_default_bindings = defaults.normal_bindings.clone();
                            self.context = defaults.context.clone();
                        }
                        // Only preceding declaration inputs enter this default.
                        for previous in 0..slot {
                            self.alias_substitution.remove(&parameters[previous].0);
                            self.alias_value_bindings.remove(&parameters[previous].0);
                            self.normal_default_bindings.remove(&parameters[previous].0);
                            if !progress.is_prepared(previous) {
                                return Err(error(
                                    span,
                                    "normal-form default awaits preceding input",
                                ));
                            }
                            if parameters[previous].2 == p(P::Type) {
                                if let Some(ty) = progress.identity(previous) {
                                    self.alias_substitution.insert(
                                        parameters[previous].0,
                                        std::sync::Arc::new(ty.clone()),
                                    );
                                } else {
                                    self.alias_value_bindings
                                        .insert(parameters[previous].0, progress.binding(previous));
                                }
                            } else {
                                // A value prefix contributes to later type identities
                                // as well as to executable default expressions. Keep
                                // the local for evaluation and the known term for
                                // dependent nominal domains; neither runs input code.
                                if let Some(value) = progress.identity(previous) {
                                    let value = self.copy_alias_type(value, span)?;
                                    self.alias_substitution
                                        .insert(parameters[previous].0, std::sync::Arc::new(value));
                                }
                                self.normal_default_bindings.insert(
                                    parameters[previous].0,
                                    (
                                        progress.binding(previous),
                                        self.instantiate_input_types(
                                            &self.locals[&progress.binding(previous)],
                                            progress.observed_parameters(),
                                            progress.observed_values(),
                                            span,
                                        )?,
                                    ),
                                );
                            }
                        }
                        self.signature_owner = Some(owner);
                        self.context = self.binder_context(owner);
                        self.in_required_comptime = true;
                        let hir::clause::ClauseParamKind::Optional(_, _, default) =
                            clauses[slot].kind
                        else {
                            return Err(error(span, "normal-form argument count mismatch"));
                        };
                        Some(default)
                    }
                };
                if explicit.is_none()
                    && (self.active_type_defaults.len() >= 64
                        || !self.active_type_defaults.insert((owner, slot)))
                {
                    return Err(error(
                        span,
                        "cyclic or excessively deep normal-form default",
                    ));
                }
                let checked = (|| {
                    let checked = if let Some(RoutedDeclarationInput::Collected(items)) = explicit {
                        if (!dependent || checking == InputChecking::Typed)
                            && items.iter().all(|item| {
                                matches!(
                                    item,
                                    n::NormalFormArgument::Value(_)
                                        | n::NormalFormArgument::ListSpread(_)
                                )
                            })
                        {
                            let mut collection = super::clause_inputs::ClauseCollection::default();
                            for item in items {
                                match item {
                                    n::NormalFormArgument::Value(value) => {
                                        collection.push_element(value.clone())
                                    }
                                    n::NormalFormArgument::ListSpread(value) => {
                                        collection.push_spread(value.clone())
                                    }
                                    _ => unreachable!("checked collection input"),
                                }
                            }
                            self.clause_collection_expression(collection, ty.clone(), span)
                                .map(n::NormalFormArgument::Value)
                        } else {
                            Ok(n::NormalFormArgument::List {
                                source: source(span),
                                items: items.clone(),
                            })
                        }
                    } else if let Some(RoutedDeclarationInput::Prepared(value)) = explicit {
                        if !dependent {
                            self.require_expression(&value.ty, &ty, span)?;
                        }
                        Ok(n::NormalFormArgument::Value(value.clone()))
                    } else if dependent && checking == InputChecking::Contextual {
                        self.dependent_normal_argument(input.unwrap(), 0)
                    } else {
                        self.with_prepared_input_observations(&prepared_bindings, span, |this| {
                            this.expr(input.unwrap(), (!dependent).then_some(&ty))
                        })
                        .map(n::NormalFormArgument::Value)
                    };
                    checked.map(|value| (value, dependent))
                })();
                if explicit.is_none() {
                    self.active_type_defaults.remove(&(owner, slot));
                }
                let (value, dependent) = checked.map_err(|mut failure| {
                    if parameters[slot].2 != p(P::Type) && failure.message.contains("type mismatch")
                    {
                        failure.message = format!(
                            "normal-form value argument type mismatch: {}",
                            failure.message
                        );
                    }
                    failure
                })?;
                let identity = match &value {
                    n::NormalFormArgument::List { items, .. } if !dependent => {
                        self.checked_collected_list_in(items, &ty, &prepared_bindings, span)?
                    }
                    _ => match value.expression() {
                        Some(value) => self.preserved_input_identity(
                            value,
                            prepared_bindings
                                .iter()
                                .map(|(binding, term)| (*binding, term.as_ref())),
                            span,
                        )?,
                        None => None,
                    },
                };
                let ty = value.expression().map(|expr| expr.ty.clone()).unwrap_or(ty);
                self.record_declaration_input(
                    &mut progress,
                    slot,
                    value,
                    identity,
                    ty,
                    &mut prepared_bindings,
                    span,
                )?;
            }
            self.signature_owner = saved_owner;
            self.context = saved_context.clone();
            self.in_required_comptime = saved_mode;
            progress.finish(preparations, span)
        })();
        self.locals = saved_locals;
        self.normal_default_bindings = saved_bindings;
        self.alias_substitution = saved_types;
        self.alias_value_bindings = saved_type_values;
        self.context = saved_context;
        self.signature_owner = saved_owner;
        self.in_required_comptime = saved_mode;
        self.default_input_scope = saved_input_scope;
        self.default_input_owner = saved_input_owner;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_dump_retains_source_slots_identities_and_escaped_values() {
        let location = n::SourceRange { start: 3, end: 9 };
        let id = DeclarationId {
            package: 1,
            local: 7,
        };
        let value = n::Expr {
            source: location,
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("quote\"\n".into())),
        };
        let inputs = PreparedDeclarationInputs {
            arguments: vec![n::NormalFormArgument::Value(value)],
            plan: n::NormalFormInputPlan {
                order: vec![0],
                bindings: vec![n::LocalId { owner: 4, index: 2 }],
                preparations: vec![],
            },
            formation: FormationInputs {
                terms: vec![p(P::U8)],
                deferred: BTreeSet::from([id]),
                environment: BTreeMap::new(),
            },
        };
        let dump = inputs.to_string();
        assert!(
            dump.starts_with("(prepared-declaration-inputs (arguments "),
            "{dump}"
        );
        assert!(dump.contains("(source 3 9)"), "{dump}");
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        assert!(dump.contains("(order 0)"), "{dump}");
        assert!(dump.contains("(bindings (local 4 2))"), "{dump}");
        assert!(dump.contains("(identities (primitive u8))"), "{dump}");
        assert!(
            dump.contains("(deferred (declaration (package 1) (local 7)))"),
            "{dump}"
        );
        assert!(!dump.contains('\n'), "unescaped newline: {dump}");
        let checked = inputs
            .into_typed(&[(id, Sort::ComptimeValue, p(P::Str))], Span::default())
            .unwrap();
        let dump = checked.to_string();
        assert!(
            dump.starts_with("(checked-declaration-inputs (statements "),
            "{dump}"
        );
        assert!(dump.contains("quote\\\"\\n"), "{dump}");
        assert!(dump.contains("(arguments (expr"), "{dump}");
        assert!(dump.contains("(local 4 2)"), "{dump}");
        assert!(dump.contains("(identities (primitive u8))"), "{dump}");
        assert!(!dump.contains('\n'), "unescaped newline: {dump}");
    }
}
