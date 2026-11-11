//! Source-order preparation of ordinary clause inputs and collection values.
use super::*;

/// Routing state is independent of expression checking and execution. Open
/// descriptions and concrete NF preparation consume the same clause slots.
/// A tuple packet supplies fixed inputs; a tail spread supplies the collector.
pub(super) struct ClauseInputRouting<'a, 'h> {
    clauses: &'a [hir::clause::ClauseParam<'h>],
    supplied: Vec<bool>,
    positional: usize,
    collector: Option<usize>,
    whole_collector: bool,
}

impl<'a, 'h> ClauseInputRouting<'a, 'h> {
    pub(super) fn new(clauses: &'a [hir::clause::ClauseParam<'h>], inputs: &[Arg<'_>]) -> Self {
        let collector = clauses
            .last()
            .filter(|clause| matches!(clause.kind, hir::clause::ClauseParamKind::Varadic(..)))
            .map(|_| clauses.len() - 1);
        let whole_collector = collector.is_some_and(|slot| {
            inputs.iter().any(|input|
            matches!(input, Arg::Named(name, _) if name.name == clauses[slot].name.name))
        });
        Self {
            clauses,
            supplied: vec![false; clauses.len()],
            positional: 0,
            collector,
            whole_collector,
        }
    }

    pub(super) fn at_collector(&self) -> bool {
        let next = (self.positional..self.clauses.len()).find(|slot| {
            !matches!(
                self.clauses[*slot].kind,
                hir::clause::ClauseParamKind::Optional(..)
            )
        });
        next.is_some() && next == self.collector
    }

    pub(super) fn slot(
        &mut self,
        name: Option<hir::common::Ident>,
        span: Span,
    ) -> Result<usize, Error> {
        let slot = Checker::type_input_slot(self.clauses, name, &mut self.positional, span)
            .map_err(|_| error(span, "unknown named normal-form argument"))?;
        let supplied = self
            .supplied
            .get_mut(slot)
            .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
        if Some(slot) == self.collector && name.is_none() {
            if self.whole_collector {
                return Err(error(
                    span,
                    "cannot combine a complete type-list argument with collected inputs",
                ));
            }
        } else if std::mem::replace(supplied, true) {
            return Err(error(span, "duplicate normal-form argument"));
        }
        Ok(slot)
    }

    pub(super) fn packet_slot(&mut self, span: Span) -> Result<usize, Error> {
        // Packet arity belongs to the fixed prefix. Do not silently turn an
        // excess tuple field into a list element with a different protocol.
        if self.at_collector() {
            return Err(error(span, "normal-form argument count mismatch"));
        }
        self.slot(None, span)
    }

    pub(super) fn missing_required(&self) -> bool {
        self.clauses
            .iter()
            .zip(&self.supplied)
            .any(|(clause, supplied)| {
                !supplied
                    && !matches!(
                        clause.kind,
                        hir::clause::ClauseParamKind::Optional(..)
                            | hir::clause::ClauseParamKind::Varadic(..)
                    )
            })
    }
}

impl std::fmt::Display for ClauseInputRouting<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(clause-input-routing (positional {}) (whole-collector {}) (supplied",
            self.positional, self.whole_collector
        )?;
        for (slot, supplied) in self.supplied.iter().enumerate() {
            if *supplied {
                write!(f, " {slot}")?;
            }
        }
        f.write_str("))")
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
