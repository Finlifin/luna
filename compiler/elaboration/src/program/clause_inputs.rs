//! Source-order preparation of ordinary clause inputs and collection values.
use super::*;
mod result_contracts;
pub(super) use result_contracts::ResultClauses;

/// HIR supplies only entry declarations; no input values or domains are routed here.
pub(super) fn entries(
    clauses: &[hir::clause::ClauseParam<'_>],
) -> Vec<input_routing::Entry<hir::Symbol>> {
    clauses
        .iter()
        .map(|clause| {
            let optional = matches!(
                clause.kind.unquoted(),
                hir::clause::ClauseParamKind::Optional(..)
            );
            let variadic = matches!(
                clause.kind.unquoted(),
                hir::clause::ClauseParamKind::Varadic(..)
            );
            input_routing::Entry {
                name: (optional || variadic).then_some(clause.name.name),
                optional,
                variadic,
                post_target: false,
                ..input_routing::Entry::anchored()
            }
        })
        .collect()
}

pub(super) fn supplies(inputs: &[Arg<'_>]) -> Vec<input_routing::Input<hir::Symbol>> {
    inputs
        .iter()
        .map(|input| match input {
            Arg::Positional(_) => input_routing::Input::Positional,
            Arg::Named(name, _) => input_routing::Input::Named(name.name),
            _ => input_routing::Input::Unsupported,
        })
        .collect::<Vec<_>>()
}

pub(super) fn routing_cursor(
    clauses: &[hir::clause::ClauseParam<'_>],
    inputs: &[Arg<'_>],
) -> Result<input_routing::Cursor<hir::Symbol>, input_routing::Failure> {
    input_routing::Cursor::new(entries(clauses), &supplies(inputs), None)
}

/// Source adaptation for the same supply cursor used by ordinary calls.
/// Packet widths are discovered by the caller; no HIR is retained by the cursor.
pub(super) struct ClauseInputRouting {
    cursor: input_routing::Cursor<hir::Symbol>,
}

/// The source cursor selects the destination before the checker interprets
/// the spread. A literal tuple exposes its fields directly in fixed slots.
pub(super) enum Spread<'h> {
    Fields(&'h [Expr<'h>]),
    Packet,
    Collection(usize),
}

impl ClauseInputRouting {
    pub(super) fn new(
        clauses: &[hir::clause::ClauseParam<'_>],
        inputs: &[Arg<'_>],
        span: Span,
    ) -> Result<Self, Error> {
        Self::from_entries(entries(clauses), inputs, span)
    }

    pub(super) fn from_entries(
        entries: Vec<input_routing::Entry<hir::Symbol>>,
        inputs: &[Arg<'_>],
        span: Span,
    ) -> Result<Self, Error> {
        let cursor = input_routing::Cursor::new(entries, &supplies(inputs), None)
            .map_err(|failure| Self::failure(failure.kind, span))?;
        Ok(Self { cursor })
    }

    fn failure(kind: input_routing::FailureKind, span: Span) -> Error {
        use input_routing::FailureKind as F;
        error(
            span,
            match kind {
                F::UnknownName => "unknown named normal-form argument".into(),
                F::Duplicate => "duplicate normal-form argument".into(),
                F::TooMany | F::PacketTargetsCollection => {
                    "normal-form argument count mismatch".into()
                }
                _ => kind.to_string(),
            },
        )
    }

    pub(super) fn has_open_collector(&self) -> bool {
        self.cursor.has_open_collector()
    }

    pub(super) fn spread<'h>(&mut self, input: &'h Expr<'h>) -> Result<Spread<'h>, Error> {
        if self.cursor.at_collector() {
            return self.slot(None, input.span).map(Spread::Collection);
        }
        Ok(match input.kind {
            H::Tuple(fields) => Spread::Fields(fields),
            H::Unit => Spread::Fields(&[]),
            _ => Spread::Packet,
        })
    }

    pub(super) fn fixed_packet_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.cursor.fixed_packet_slots()
    }

    pub(super) fn remaining_anchors(&self) -> usize {
        self.cursor.remaining_anchors()
    }

    pub(super) fn slot(
        &mut self,
        name: Option<hir::common::Ident>,
        span: Span,
    ) -> Result<usize, Error> {
        let input = name.map_or(input_routing::Input::Positional, |name| {
            input_routing::Input::Named(name.name)
        });
        self.cursor
            .select(&input)
            .map(|selection| selection.parameter)
            .map_err(|kind| Self::failure(kind, span))
    }

    pub(super) fn packet_slots(&mut self, width: usize, span: Span) -> Result<Vec<usize>, Error> {
        self.cursor
            .packet_slots(width)
            .map_err(|kind| Self::failure(kind, span))
    }

    pub(super) fn missing_required(&self) -> bool {
        self.cursor.missing_required()
    }
}

impl std::fmt::Display for ClauseInputRouting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(clause-input-routing {})", self.cursor)
    }
}

#[derive(Default)]
pub(super) struct ClauseCollection {
    parts: Vec<CollectionPart>,
}

enum CollectionPart {
    Elements(Vec<n::Expr>),
    Spread(n::Expr),
}

impl ClauseCollection {
    pub(super) fn push_element(&mut self, value: n::Expr) {
        if let Some(CollectionPart::Elements(fields)) = self.parts.last_mut() {
            fields.push(value);
        } else {
            self.parts.push(CollectionPart::Elements(vec![value]));
        }
    }
    pub(super) fn push_spread(&mut self, value: n::Expr) {
        self.parts.push(CollectionPart::Spread(value));
    }
}

impl std::fmt::Display for ClauseCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(clause-collection (parts")?;
        for part in &self.parts {
            match part {
                CollectionPart::Elements(elements) => {
                    f.write_str(" (elements")?;
                    for value in elements {
                        write!(f, " {value}")?;
                    }
                    f.write_str(")")?;
                }
                CollectionPart::Spread(value) => write!(f, " (spread {value})")?,
            }
        }
        f.write_str("))")
    }
}

impl Checker<'_, '_> {
    /// Publish a checked prefix slot for use by later domains, bounds and
    /// defaults. The owner index locates this lexical frame while it exists.
    pub(super) fn record_clause_input(
        &mut self,
        owner: OwnerId,
        id: DeclarationId,
        binding: n::LocalId,
        domain: Term,
    ) {
        self.normal_input_prefixes
            .entry(owner)
            .or_default()
            .push((id, binding, domain));
        self.parameter_domain_owners.entry(id).or_insert(owner);
    }

    pub(super) fn clause_collector(&self, owner: OwnerId) -> Option<(usize, hir::Symbol)> {
        let clauses = self.type_clause_params(owner);
        let (last, prefix) = clauses.split_last()?;
        match last.kind {
            hir::clause::ClauseParamKind::Varadic(name, _) => Some((prefix.len(), name.name)),
            _ => None,
        }
    }

    /// Assemble already checked inputs without moving their producers. Both
    /// ordinary clause packets and opaque head plans use this expression.
    pub(super) fn clause_collection_expression(
        &mut self,
        collection: ClauseCollection,
        domain: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let element = self
            .list_element_type(&domain)
            .cloned()
            .ok_or_else(|| error(span, "variadic clause input requires a bound meta.List<T>"))?;
        let mut parts = Vec::with_capacity(collection.parts.len());
        for part in collection.parts {
            let value = match part {
                CollectionPart::Elements(fields) => {
                    for field in &fields {
                        self.require_expression(&field.ty, &element, span)?;
                    }
                    n::Expr {
                        source: source(span),
                        ty: if fields.iter().any(|field| is_never(&field.ty)) {
                            p(P::Never)
                        } else {
                            domain.clone()
                        },
                        kind: K::TypeList(fields),
                    }
                }
                CollectionPart::Spread(value) => {
                    self.require_expression(&value.ty, &domain, span)?;
                    value
                }
            };
            parts.push(value);
        }
        // Left-to-right leaf order is retained even when a caller leaves its
        // contiguous tail producers inline. Balanced assembly avoids a linear
        // NIR depth and repeated copies of the entire accumulated prefix.
        while parts.len() > 1 {
            let mut next = Vec::with_capacity(parts.len().div_ceil(2));
            let mut values = parts.into_iter();
            while let Some(left) = values.next() {
                next.push(if let Some(right) = values.next() {
                    n::Expr {
                        source: source(span),
                        ty: if is_never(&left.ty) || is_never(&right.ty) {
                            p(P::Never)
                        } else {
                            domain.clone()
                        },
                        kind: K::ListConcat(Box::new(left), Box::new(right)),
                    }
                } else {
                    left
                });
            }
            parts = next;
        }
        let value = parts.pop().unwrap_or_else(|| n::Expr {
            source: source(span),
            ty: domain,
            kind: K::TypeList(Vec::new()),
        });
        Ok(value)
    }
}

/// The source interface can inspect arity without copying identities or
/// interpreting a producer. Public members include Self before either tuple.
#[derive(Clone, Copy)]
pub(super) enum CallableClauseView<'v> {
    Fixed(&'v [Term]),
    Prepared(&'v n::ClauseOperands),
    Contextual(&'v [n::NormalFormArgument]),
    Member {
        subject: &'v Term,
        interface: &'v [Term],
        method: &'v [Term],
    },
}

impl<'v> CallableClauseView<'v> {
    pub(super) fn direct(callee: &'v n::Expr) -> Option<(DeclarationId, Self)> {
        Some(match &callee.kind {
            K::Function(id) => (*id, Self::Fixed(&[])),
            K::PlannedFunctionInstance {
                declaration,
                arguments,
                ..
            } => (*declaration, Self::Contextual(arguments)),
            K::ComptimeFunctionInstance {
                declaration,
                inputs,
            } => (*declaration, Self::Prepared(inputs)),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, Self::Fixed(arguments)),
            _ => return None,
        })
    }

    pub(super) fn len(&self) -> usize {
        match *self {
            Self::Fixed(values) => values.len(),
            Self::Prepared(inputs) => inputs.arguments.len(),
            Self::Contextual(inputs) => inputs.len(),
            Self::Member {
                interface, method, ..
            } => 1 + interface.len() + method.len(),
        }
    }

    pub(super) fn identity(&self, slot: usize) -> Option<&'v Term> {
        match *self {
            Self::Fixed(values) => values.get(slot),
            Self::Prepared(inputs) => inputs.observed_argument(slot),
            Self::Contextual(inputs) => match &inputs.get(slot)?.expression()?.kind {
                K::TypeValue(ty) | K::ComptimeValue(ty) => Some(ty),
                _ => None,
            },
            Self::Member { subject, .. } if slot == 0 => Some(subject),
            Self::Member {
                interface, method, ..
            } => {
                let slot = slot - 1;
                if slot < interface.len() {
                    interface.get(slot)
                } else {
                    method.get(slot - interface.len())
                }
            }
        }
    }
}
