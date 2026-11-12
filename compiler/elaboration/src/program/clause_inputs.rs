//! Source-order preparation of ordinary clause inputs and collection values.
use super::*;

/// Source adaptation for the same supply cursor used by ordinary calls.
/// Packet widths are discovered by the caller; no HIR is retained by the cursor.
pub(super) struct ClauseInputRouting {
    cursor: input_routing::Cursor<hir::Symbol>,
}

impl ClauseInputRouting {
    pub(super) fn new(
        clauses: &[hir::clause::ClauseParam<'_>],
        inputs: &[Arg<'_>],
        span: Span,
    ) -> Result<Self, Error> {
        let entries = clauses
            .iter()
            .map(|clause| {
                let optional = matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..));
                let variadic = matches!(clause.kind, hir::clause::ClauseParamKind::Varadic(..));
                input_routing::Entry {
                    name: (optional || variadic).then_some(clause.name.name),
                    optional,
                    variadic,
                    post_target: false,
                    ..input_routing::Entry::anchored()
                }
            })
            .collect();
        let inputs = inputs
            .iter()
            .map(|input| match input {
                Arg::Positional(_) => input_routing::Input::Positional,
                Arg::Named(name, _) => input_routing::Input::Named(name.name),
                _ => input_routing::Input::Unsupported,
            })
            .collect::<Vec<_>>();
        let cursor = input_routing::Cursor::new(entries, &inputs, None)
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
                F::AppendToComplete => {
                    "cannot combine a complete type-list argument with collected inputs".into()
                }
                F::TooMany | F::PacketTargetsCollection => {
                    "normal-form argument count mismatch".into()
                }
                _ => kind.to_string(),
            },
        )
    }

    pub(super) fn at_collector(&self) -> bool {
        self.cursor.at_collector()
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

    pub(super) fn packet_slot(&mut self, span: Span) -> Result<usize, Error> {
        self.cursor
            .packet_slot()
            .map(|selection| selection.parameter)
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
    pub(super) fn clause_has_collector(&self, owner: OwnerId) -> bool {
        self.type_clause_params(owner)
            .last()
            .is_some_and(|parameter| {
                matches!(parameter.kind, hir::clause::ClauseParamKind::Varadic(..))
            })
    }
    pub(super) fn clause_fixed_prefix_count(&self, owner: OwnerId) -> usize {
        self.type_clause_params(owner)
            .iter()
            .filter(|clause| {
                !matches!(
                    clause.kind,
                    hir::clause::ClauseParamKind::Optional(..)
                        | hir::clause::ClauseParamKind::Varadic(..)
                )
            })
            .count()
    }

    pub(super) fn clause_prefix_literal_spreads<'h>(
        &self,
        owner: OwnerId,
        inputs: &[Arg<'h>],
    ) -> Option<Vec<Arg<'h>>> {
        let whole_collector = self.type_clause_params(owner).last().is_some_and(|clause| {
            inputs
                .iter()
                .any(|input| matches!(input, Arg::Named(name, _) if name.name == clause.name.name))
        });
        let mut remaining = self.clause_fixed_prefix_count(owner);
        let mut expanded = Vec::new();
        let mut changed = false;
        for input in inputs {
            match input {
                Arg::Expand(value) if remaining > 0 => match value.kind {
                    H::Tuple(fields) if fields.len() <= remaining || whole_collector => {
                        remaining = remaining.saturating_sub(fields.len());
                        expanded.extend(fields.iter().map(Arg::Positional));
                        changed = true;
                    }
                    H::Unit => {
                        changed = true;
                    }
                    _ => {
                        // A computed packet can determine the remaining prefix
                        // later. Do not flatten subsequent tail list inputs.
                        remaining = 0;
                        expanded.push(input.clone());
                    }
                },
                Arg::Positional(_) => {
                    remaining = remaining.saturating_sub(1);
                    expanded.push(input.clone());
                }
                _ => expanded.push(input.clone()),
            }
        }
        changed.then_some(expanded)
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
