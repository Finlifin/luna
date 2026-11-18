//! Source routing retains spread checks and their original execution positions.
//! Layout hints do not execute producers or replace packet/domain validation.
use super::*;
use request::Request;

pub(super) enum Poll<'h> {
    Continue,
    Child(Request<'h, 'h>),
    Complete,
}

pub(super) struct Frame<'h> {
    span: Span,
    telescope: domain::Telescope,
    prefix: KnownInputs,
    inputs: Vec<Arg<'h>>,
    packet_slots: Vec<Option<Vec<usize>>>,
    list_spreads: BTreeSet<usize>,
    collector: Option<usize>,
    routing: clause_inputs::ClauseInputRouting,
    next: usize,
    pending: Option<usize>,
    complete: bool,
    routed: Vec<Option<RoutedDeclarationInput<'h, 'h>>>,
    order: Vec<usize>,
    preparations: Vec<Vec<n::NormalFormPreparation>>,
    pending_collection: Vec<PendingDeclarationCollection<'h, 'h>>,
    prepared_bindings: BTreeMap<n::LocalId, std::sync::Arc<Term>>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        inputs: &[Arg<'h>],
        span: Span,
        parameters: Parameters,
        prefix: KnownInputs,
        entries: Vec<input_routing::Entry<hir::Symbol>>,
    ) -> Result<Self, Error> {
        if parameters.len() != entries.len() {
            return Err(error(span, "input routing interface arity mismatch"));
        }
        let mut inputs = inputs.to_vec();
        // Layout hints establish only positions. Actual packet checking
        // still validates the tuple and its element types. Do not expose
        // a head's unresolved Type binders as caller-side expectations.
        let positional_slots: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.optional && !entry.variadic)
            .map(|(slot, _)| slot)
            .collect();
        let telescope = domain::Telescope::new(parameters, &mut checker.required_steps, span)?;
        let collector = entries.iter().position(|entry| entry.variadic);
        loop {
            let whole_collector = collector.is_some_and(|slot| {
                inputs.iter().any(|input|
                matches!(input, Arg::Named(name, _) if Some(name.name) == entries[slot].name))
            });
            let mut excluded = BTreeSet::new();
            let mut list_spreads = BTreeSet::new();
            let mut remaining = Some(positional_slots.len());
            let mut layout_fuel = 10_000;
            if collector.is_some() && !whole_collector {
                for (index, input) in inputs.iter().enumerate() {
                    match input {
                        Arg::Positional(_) if remaining == Some(0) => {
                            excluded.insert(index);
                        }
                        Arg::Expand(_) if remaining == Some(0) => {
                            excluded.insert(index);
                            list_spreads.insert(index);
                        }
                        Arg::Positional(_) => remaining = remaining.map(|n| n.saturating_sub(1)),
                        Arg::Expand(value) => {
                            remaining = match (
                                remaining,
                                checker.spread_width(value, &mut layout_fuel, 0)?,
                            ) {
                                (Some(left), Some(width)) if width <= left => Some(left - width),
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
                    checker.required_steps = checker.required_steps.checked_sub(1)
                        .ok_or_else(|| error(span, "normal-form collection routing budget exhausted"))?;
                    if list_spreads.contains(&index)
                        && let Arg::Expand(value) = input
                        && let H::List(elements) = value.kind
                    {
                        checker.required_steps = checker.required_steps.checked_sub(elements.len() as u64)
                            .ok_or_else(|| error(span, "normal-form collection routing budget exhausted"))?;
                        expanded.extend(elements.iter().map(Arg::Positional));
                    } else {
                        expanded.push(input.clone());
                    }
                }
                inputs = expanded;
                continue;
            }
            let packet_slots = if inputs.iter().any(|input| matches!(input, Arg::Expand(_))) {
                checker.spread_packet_layout_excluding(
                    &inputs,
                    &positional_slots,
                    span,
                    &excluded,
                )?
            } else {
                vec![None; inputs.len()]
            };

            let routing = clause_inputs::ClauseInputRouting::from_entries(entries, &inputs, span)?;
            let preparations = vec![Vec::new(); inputs.len()];
            let routed = (0..telescope.parameters.len()).map(|_| None).collect();
            return Ok(Self {
                span,
                telescope,
                prefix,
                inputs,
                packet_slots,
                list_spreads,
                collector,
                routing,
                next: 0,
                pending: None,
                complete: false,
                routed,
                order: Vec::new(),
                preparations,
                pending_collection: Vec::new(),
                prepared_bindings: BTreeMap::new(),
            });
        }
    }

    pub(super) fn poll(&mut self, checker: &mut Checker<'_, '_>) -> Result<Poll<'h>, Error> {
        if self.complete || self.pending.is_some() {
            return Err(error(self.span, "input routing is pending or complete"));
        }
        let Some(input) = self.inputs.get(self.next).cloned() else {
            self.complete = true;
            return Ok(Poll::Complete);
        };
        let input_index = self.next;
        let span = self.span;
        if let Arg::Expand(packet) = input {
            if self.list_spreads.contains(&input_index) {
                self.routing.slot(None, packet.span)?;
                self.pending_collection.push(PendingDeclarationCollection {
                    source_index: input_index,
                    before: self.order.len(),
                    input: packet,
                    spread: true,
                });
                self.next += 1;
                return Ok(Poll::Continue);
            }
            let context = if let Some(slots) = &self.packet_slots[input_index] {
                use crate::calls::InputContext as C;
                let fields = self.packet_domains(checker, slots, span)?;
                if slots
                    .iter()
                    .any(|&slot| !self.telescope.direct(slot).is_empty())
                {
                    let fields = slots
                        .iter()
                        .zip(fields)
                        .map(|(&slot, ty)| {
                            if self.telescope.direct(slot).is_empty() {
                                C::Known(ty)
                            } else {
                                C::Unknown
                            }
                        })
                        .collect();
                    Some(match checker.bound_tuple_head(packet.span)? {
                        Some(head) => C::NormalTuple {
                            head,
                            fields,
                            list_domain: checker.bound_tuple_list_domain(),
                        },
                        None => C::Tuple(fields),
                    })
                } else {
                    Some(C::Known(checker.tuple_value_type(fields, packet.span)?))
                }
            } else {
                None
            };
            self.pending = Some(input_index);
            return Ok(Poll::Child(match context {
                Some(context) => Request::input(packet, context),
                None => Request::ordinary(packet, None),
            }));
        }
        let (slot, value) = match input {
            Arg::Positional(value) => {
                let slot = self.routing.slot(None, span)?;
                if self.collector == Some(slot) {
                    self.pending_collection.push(PendingDeclarationCollection {
                        source_index: input_index,
                        before: self.order.len(),
                        input: value,
                        spread: false,
                    });
                    self.next += 1;
                    return Ok(Poll::Continue);
                }
                (slot, value)
            }
            Arg::Named(name, value) => (self.routing.slot(Some(name), span)?, value),
            _ => {
                return Err(error(
                    span,
                    "normal-form input expansion is not yet implemented",
                ));
            }
        };
        let cell = self
            .routed
            .get_mut(slot)
            .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
        if cell
            .replace(RoutedDeclarationInput::Source(value))
            .is_some()
        {
            return Err(error(span, "duplicate normal-form argument"));
        }
        self.order.push(slot);
        self.next += 1;
        Ok(Poll::Continue)
    }

    fn packet_domains(
        &self,
        checker: &mut Checker<'_, '_>,
        slots: &[usize],
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        let domains = slots.iter().map(|&slot| &self.telescope.parameters[slot].2);
        if self.prefix.values().is_empty() {
            domains
                .map(|ty| environment::copy_term(ty, &mut checker.required_steps, span))
                .collect()
        } else {
            self.prefix
                .with_substitution(checker, span, |substitution, fuel| {
                    domains
                        .map(|ty| substitution.instantiate_bounded(ty, fuel))
                        .collect()
                })
        }
    }

    fn packet_fields(
        &self,
        checker: &mut Checker<'_, '_>,
        packet: &n::Expr,
        input_index: usize,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        // Early exit does not erase the checked type of an unreachable tail.
        let mut shape = packet;
        while is_never(&shape.ty) {
            match &shape.kind {
                K::Block {
                    result: Some(result),
                    ..
                } => shape = result,
                _ => break,
            }
        }
        if let Term::Tuple(fields) = checker.structural_type(&shape.ty, span)? {
            Ok(fields)
        } else if is_never(&shape.ty) {
            let slots = self.packet_slots[input_index].as_deref().ok_or_else(|| {
                error(
                    span,
                    "diverging type input spread has ambiguous packet width",
                )
            })?;
            self.packet_domains(checker, slots, span)
        } else {
            Err(error(span, "type input spread requires a tuple"))
        }
    }

    pub(super) fn accept(
        &mut self,
        checker: &mut Checker<'_, '_>,
        result: Result<n::NormalFormArgument, Error>,
    ) -> Result<(), Error> {
        let input_index = self
            .pending
            .take()
            .ok_or_else(|| error(self.span, "input routing has no pending packet"))?;
        let Arg::Expand(packet) = self.inputs[input_index] else {
            unreachable!("pending packet")
        };
        let n::NormalFormArgument::Value(value) = result? else {
            return Err(error(
                packet.span,
                "packet checking requires an ordinary expression",
            ));
        };
        let span = self.span;
        let fields = self.packet_fields(checker, &value, input_index, packet.span)?;
        let observed = checker.preserved_input_identity(
            &value,
            self.prepared_bindings
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
        let before = self.order.len();
        let mut pattern = Vec::new();
        for ty in fields {
            let slot = self.routing.packet_slot(packet.span)?;
            let binding = checker.spread_binding(packet.span)?;
            if let Some(known) = observed_fields.as_mut().and_then(Iterator::next) {
                self.prepared_bindings
                    .insert(binding, std::sync::Arc::new(known));
            }
            checker.locals.insert(binding, ty.clone());
            pattern.push(n::Pattern::Binding {
                binding,
                inner: Box::new(n::Pattern::Wild),
            });
            let input = n::Expr {
                source: value.source,
                ty,
                kind: K::Local(binding),
            };
            let cell = self
                .routed
                .get_mut(slot)
                .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
            if cell
                .replace(RoutedDeclarationInput::Prepared(input))
                .is_some()
            {
                return Err(error(span, "duplicate normal-form argument"));
            }
            self.order.push(slot);
        }
        self.preparations[input_index].push(n::NormalFormPreparation {
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

        self.next += 1;
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        checker: &mut Checker<'_, '_>,
        interface: interface::Interface<'h>,
        defaults: Option<environment::Environment>,
        checking: InputChecking,
    ) -> Result<slots::Frame<'h, 'h>, Error> {
        if !self.complete || self.pending.is_some() {
            return Err(error(self.span, "input routing is incomplete"));
        }
        // Keep source scheduling independent of dependency-driven checks.
        if !self.pending_collection.is_empty() {
            self.order
                .push(self.collector.expect("collected inputs have a collector"));
        }
        let bindings = (0..self.telescope.parameters.len())
            .map(|_| checker.spread_binding(self.span))
            .collect::<Result<Vec<_>, _>>()?;
        let progress = progress::Progress::new(
            self.telescope,
            bindings,
            self.order,
            self.prefix,
            &mut checker.required_steps,
            self.span,
        )?;
        let collection = collections::Frame::new(
            self.span,
            checking,
            self.collector,
            self.inputs.len(),
            self.pending_collection,
            self.preparations,
        );
        Ok(slots::Frame::new(
            self.span,
            interface,
            defaults,
            checking,
            self.routed,
            self.prepared_bindings,
            progress,
            collection,
        ))
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-routing-frame (source {}) (next {}) (complete {}) {} (pending",
            source(self.span),
            self.next,
            self.complete,
            self.routing
        )?;
        if let Some(index) = self.pending {
            write!(f, " (source-index {index})")?;
            if let Arg::Expand(input) = self.inputs[index] {
                write!(f, " (child {} {})", input.hir_id, source(input.span))?;
            }
            if let Some(slots) = &self.packet_slots[index] {
                f.write_str(" (declaration-slots")?;
                for slot in slots {
                    write!(f, " {slot}")?;
                }
                f.write_str(")")?;
            }
        }
        f.write_str(") ")?;
        self.prefix.fmt_prefix(self.prefix.values.len(), f)?;
        f.write_str(" (domains")?;
        for (id, sort, domain) in &self.telescope.parameters {
            write!(f, " (input {id} {sort} {domain})")?;
        }
        f.write_str(") (order")?;
        for slot in &self.order {
            write!(f, " {slot}")?;
        }
        f.write_str(") (routed")?;
        for (slot, input) in self.routed.iter().enumerate() {
            if let Some(input) = input {
                write!(f, " (slot {slot} {input})")?;
            }
        }
        f.write_str(") (collections")?;
        for input in &self.pending_collection {
            write!(f, " {input}")?;
        }
        f.write_str(") (observations")?;
        for (binding, value) in &self.prepared_bindings {
            write!(f, " (binding {binding} {value})")?;
        }
        f.write_str(") (preparations")?;
        for (index, preparations) in self.preparations.iter().enumerate() {
            for preparation in preparations {
                write!(
                    f,
                    " (source-index {index} (before {}) {})",
                    preparation.before, preparation.expression
                )?;
            }
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_dump_distinguishes_pending_source_from_prepared_packet() {
        let span = Span::default();
        let owner = hir::OwnerId::new(hir::LocalDefId::new(1));
        let input = Expr {
            hir_id: hir::HirId::new(owner, hir::ItemLocalId::new(2)),
            span,
            kind: H::Unit,
        };
        let frame = Frame {
            prefix: KnownInputs::default(),
            span,
            telescope: domain::Telescope::new(Vec::new(), &mut 100, span).unwrap(),
            inputs: vec![Arg::Expand(&input), Arg::Expand(&input)],
            packet_slots: vec![None, Some(vec![])],
            list_spreads: BTreeSet::new(),
            collector: None,
            routing: clause_inputs::ClauseInputRouting::new(&[], &[], span).unwrap(),
            next: 1,
            pending: Some(1),
            complete: false,
            routed: vec![],
            order: vec![],
            preparations: vec![vec![n::NormalFormPreparation {
                before: 0,
                expression: n::Expr {
                    source: source(span),
                    ty: p(P::Str),
                    kind: K::Literal(n::Literal::String("packet\"\n".into())),
                },
            }]],
            pending_collection: vec![],
            prepared_bindings: BTreeMap::new(),
        };
        let dump = frame.to_string();
        for fragment in [
            "(next 1) (complete false)",
            "(pending (source-index 1) (child ",
            "(declaration-slots)",
            "(preparations (source-index 0 (before 0)",
            "packet\\\"\\n",
        ] {
            assert!(dump.contains(fragment), "missing {fragment}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
        let request = Request::input(
            &input,
            crate::calls::InputContext::Tuple(vec![
                crate::calls::InputContext::Known(p(P::Bool)),
                crate::calls::InputContext::Unknown,
            ]),
        )
        .to_string();
        assert!(request.contains("(input "), "{request}");
        assert!(request.contains("(primitive bool)"), "{request}");
        assert!(!request.contains("packet"), "{request}");
    }
}
