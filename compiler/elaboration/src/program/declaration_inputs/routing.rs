//! Source routing retains spread checks and their original execution positions.
//! Layout hints do not execute producers or replace packet/domain validation.
use super::*;
use request::Request;

pub(super) enum Poll<'h> {
    Continue,
    Child(Request<'h, 'h>),
    Complete,
}

/// A computed fixed packet can receive its destination types before its
/// expression is checked. An open collector has no prescribed field count.
struct WidthHint {
    width: Option<usize>,
    following: Option<usize>,
}

enum PacketWidths {
    // Once a packet needs an inferred width, retain the rest in reverse
    // source order. Earlier packets and ordinary calls allocate no table.
    Fixed(Vec<WidthHint>),
    OpenCollector { last_positional: Option<usize> },
}

impl PacketWidths {
    fn new(inputs: &[Arg<'_>], open: bool) -> Self {
        if open {
            Self::OpenCollector {
                last_positional: inputs
                    .iter()
                    .rposition(|arg| matches!(arg, Arg::Expand(_) | Arg::Positional(_))),
            }
        } else {
            Self::Fixed(Vec::new())
        }
    }

    fn at(
        &mut self,
        checker: &mut Checker<'_, '_>,
        inputs: &[Arg<'_>],
        packet: &Expr<'_>,
        index: usize,
        remaining: usize,
        fuel: &mut usize,
    ) -> Result<Option<usize>, Error> {
        let from_end = inputs.len() - index - 1;
        if let Self::Fixed(hints) = self
            && let Some(hint) = hints.get(from_end)
        {
            return Ok(hint
                .width
                .or_else(|| remaining.checked_sub(hint.following?)));
        }
        if let Some(width) = checker.spread_width(packet, fuel, 0)? {
            return Ok(Some(width));
        }
        let hints = match self {
            Self::Fixed(hints) => hints,
            Self::OpenCollector { last_positional } => {
                // With a collection tail, only the last positional source must
                // fill all remaining fixed slots. Other partitions stay open.
                return Ok((*last_positional == Some(index)).then_some(remaining));
            }
        };
        let mut following = Some(0usize);
        for input in inputs[index + 1..].iter().rev() {
            let width = match input {
                Arg::Expand(packet) => checker.spread_width(packet, fuel, 0)?,
                Arg::Positional(_) => Some(1),
                _ => Some(0),
            };
            hints.push(WidthHint { width, following });
            following = match (width, following) {
                (Some(a), Some(b)) => Some(
                    a.checked_add(b)
                        .ok_or_else(|| error(packet.span, "spread layout size exhausted"))?,
                ),
                _ => None,
            };
        }
        Ok(following.and_then(|count| remaining.checked_sub(count)))
    }
}

struct PendingPacket {
    index: usize,
    slots: Option<Vec<usize>>,
}

pub(super) struct Frame<'h> {
    span: Span,
    telescope: domain::Telescope,
    caller_frames: Vec<Vec<Sort>>,
    prefix: KnownInputs,
    inputs: Vec<Arg<'h>>,
    widths: PacketWidths,
    layout_fuel: usize,
    collector: Option<usize>,
    routing: clause_inputs::ClauseInputRouting,
    next: usize,
    pending: Option<PendingPacket>,
    complete: bool,
    routed: Vec<Option<RoutedDeclarationInput<'h, 'h>>>,
    order: Vec<usize>,
    tail_start: usize,
    packets: Vec<PacketPreparation>,
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
        environment::charge(&mut checker.required_steps, inputs.len(), span)?;
        let collector = entries.iter().position(|entry| entry.variadic);
        let routing = clause_inputs::ClauseInputRouting::from_entries(entries, inputs, span)?;
        let telescope = domain::Telescope::new(parameters, &mut checker.required_steps, span)?;
        let routed = (0..telescope.parameters.len()).map(|_| None).collect();
        let widths = PacketWidths::new(inputs, routing.has_open_collector());
        Ok(Self {
            span,
            telescope,
            caller_frames: checker.call_input_frames(),
            prefix,
            inputs: inputs.to_vec(),
            widths,
            layout_fuel: 10_000,
            collector,
            routing,
            next: 0,
            pending: None,
            complete: false,
            routed,
            order: Vec::new(),
            tail_start: 0,
            packets: Vec::new(),
            pending_collection: Vec::new(),
            prepared_bindings: BTreeMap::new(),
        })
    }

    fn supply(&mut self, slot: usize, input: RoutedDeclarationInput<'h, 'h>) {
        // Cursor selection has already checked reservations and duplicates.
        self.tail_start = self.next + 1;
        self.routed[slot] = Some(input);
        self.order.push(slot);
    }

    fn collect(&mut self, input: &'h Expr<'h>, spread: bool) {
        self.pending_collection.push(PendingDeclarationCollection {
            source_index: self.next,
            before: self.order.len(),
            input,
            spread,
        });
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
            match self.routing.spread(packet)? {
                clause_inputs::Spread::Collection(_) => {
                    if let H::List(items) = packet.kind {
                        environment::charge(&mut checker.required_steps, items.len(), packet.span)?;
                        for item in items {
                            self.collect(item, false);
                        }
                    } else {
                        self.collect(packet, true);
                    }
                    self.next += 1;
                    return Ok(Poll::Continue);
                }
                clause_inputs::Spread::Fields(items) => {
                    environment::charge(&mut checker.required_steps, items.len(), packet.span)?;
                    let slots = self.routing.packet_slots(items.len(), packet.span)?;
                    for (item, slot) in items.iter().zip(slots) {
                        self.supply(slot, RoutedDeclarationInput::Source(item));
                    }
                    self.next += 1;
                    return Ok(Poll::Continue);
                }
                clause_inputs::Spread::Packet => {}
            }
            let remaining = self.routing.remaining_anchors();
            let width = self.widths.at(
                checker,
                &self.inputs,
                packet,
                input_index,
                remaining,
                &mut self.layout_fuel,
            )?;
            let slots = if let Some(width) = width.filter(|&width| width <= remaining) {
                environment::charge(&mut checker.required_steps, width, packet.span)?;
                Some(
                    self.routing
                        .fixed_packet_slots()
                        .take(width)
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            };
            let context = slots
                .as_deref()
                .map(|slots| self.packet_context(checker, slots, packet.span))
                .transpose()?;
            self.pending = Some(PendingPacket {
                index: input_index,
                slots,
            });
            return Ok(Poll::Child(match context {
                Some(context) => Request::input(packet, context),
                None => Request::ordinary(packet, None),
            }));
        }
        let (slot, value) = match input {
            Arg::Positional(value) => {
                let slot = self.routing.slot(None, span)?;
                if self.collector == Some(slot) {
                    self.collect(value, false);
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
        self.supply(slot, RoutedDeclarationInput::Source(value));
        self.next += 1;
        Ok(Poll::Continue)
    }

    fn packet_context(
        &self,
        checker: &mut Checker<'_, '_>,
        slots: &[usize],
        span: Span,
    ) -> Result<crate::calls::InputContext, Error> {
        use crate::calls::InputContext as C;
        let fields = self.packet_domains(checker, slots, span)?;
        if slots
            .iter()
            .all(|&slot| self.telescope.direct(slot).is_empty())
        {
            return Ok(C::Known(checker.tuple_value_type(fields, span)?));
        }
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
        Ok(match checker.bound_tuple_head(span)? {
            Some(head) => C::NormalTuple {
                head,
                fields,
                list_domain: checker.bound_tuple_list_domain(),
            },
            None => C::Tuple(fields),
        })
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
            nominal_fields::metered(&mut checker.required_steps, |fuel| {
                domains
                    .map(|ty| {
                        nir::instantiate_observed_domain(
                            ty,
                            |id| self.prefix.observed(id),
                            &self.caller_frames,
                            fuel,
                        )
                    })
                    .collect()
            })
            .map_err(|reason| error(span, reason.to_string()))
        }
    }

    fn packet_fields(
        &self,
        checker: &mut Checker<'_, '_>,
        packet: &n::Expr,
        slots: Option<&[usize]>,
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
            let slots = slots.ok_or_else(|| {
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
        let pending = self
            .pending
            .take()
            .ok_or_else(|| error(self.span, "input routing has no pending packet"))?;
        let Arg::Expand(packet) = self.inputs[pending.index] else {
            unreachable!("pending packet")
        };
        let n::NormalFormArgument::Value(value) = result? else {
            return Err(error(
                packet.span,
                "packet checking requires an ordinary expression",
            ));
        };
        let span = self.span;
        let fields = self.packet_fields(checker, &value, pending.slots.as_deref(), packet.span)?;
        environment::charge(&mut checker.required_steps, fields.len(), packet.span)?;
        let slots = self.routing.packet_slots(fields.len(), packet.span)?;
        let observed = checker.preserved_input_identity(
            &value,
            |id, _| Ok(self.prepared_bindings.get(&id).cloned()),
            packet.span,
        )?;
        let mut observed_fields = match observed {
            Some(Term::TupleValue(values)) if values.len() == fields.len() => {
                Some(values.into_iter())
            }
            _ => None,
        };
        let before = self.order.len();
        let mut pattern = Vec::with_capacity(fields.len());
        for (ty, slot) in fields.into_iter().zip(slots) {
            let binding = checker.spread_binding(packet.span)?;
            if let Some(known) = observed_fields.as_mut().and_then(Iterator::next) {
                self.prepared_bindings
                    .insert(binding, std::sync::Arc::new(known));
            }
            checker.locals.insert(
                binding,
                copy_input_domain(&ty, &mut checker.required_steps, packet.span)?,
            );
            pattern.push(n::Pattern::Binding {
                binding,
                inner: Box::new(n::Pattern::Wild),
            });
            let input = n::Expr {
                source: value.source,
                ty,
                kind: K::Local(binding),
            };
            self.supply(slot, RoutedDeclarationInput::Prepared(input));
        }
        self.tail_start = self.next + 1;
        self.packets.push(PacketPreparation {
            source_index: pending.index,
            action: n::NormalFormPreparation {
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
            self.caller_frames,
            &mut checker.required_steps,
            self.span,
        )?;
        let collection = collections::Frame::new(
            self.span,
            checking,
            self.collector,
            self.tail_start,
            self.pending_collection,
            self.packets,
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
        if let Some(PendingPacket { index, slots }) = &self.pending {
            write!(f, " (source-index {index})")?;
            if let Arg::Expand(input) = self.inputs[*index] {
                write!(f, " (child {} {})", input.hir_id, source(input.span))?;
            }
            if let Some(slots) = slots {
                f.write_str(" (declaration-slots")?;
                for slot in slots {
                    write!(f, " {slot}")?;
                }
                f.write_str(")")?;
            }
        }
        f.write_str(") ")?;
        write!(f, "{}", self.prefix)?;
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
        for packet in &self.packets {
            write!(f, " {packet}")?;
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
            caller_frames: vec![],
            prefix: KnownInputs::default(),
            span,
            telescope: domain::Telescope::new(Vec::new(), &mut 100, span).unwrap(),
            inputs: vec![Arg::Expand(&input), Arg::Expand(&input)],
            widths: PacketWidths::Fixed(vec![]),
            layout_fuel: 100,
            collector: None,
            routing: clause_inputs::ClauseInputRouting::new(&[], &[], span).unwrap(),
            next: 1,
            pending: Some(PendingPacket {
                index: 1,
                slots: Some(vec![]),
            }),
            complete: false,
            routed: vec![],
            order: vec![],
            tail_start: 1,
            packets: vec![PacketPreparation {
                source_index: 0,
                action: n::NormalFormPreparation {
                    before: 0,
                    expression: n::Expr {
                        source: source(span),
                        ty: p(P::Str),
                        kind: K::Literal(n::Literal::String("packet\"\n".into())),
                    },
                },
            }],
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
