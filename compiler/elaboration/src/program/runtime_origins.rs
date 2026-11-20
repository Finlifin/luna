//! Runtime place identity follows the root storage or pointer binding and its
//! checked field indices. Reading a pointer binding also fixes its write epoch.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Projection {
    pub(super) root: nir::RuntimeId,
    pub(super) fields: Vec<usize>,
}

/// Occurrences and identities have the same lexical and speculative lifetime.
#[derive(Default, Clone)]
pub(super) struct LocalOrigins {
    pub(super) ids: BTreeMap<LocalRuntimeOrigin, nir::RuntimeId>,
    pub(super) sources: SourceBindings<Term>,
    pub(super) acquired_bindings: LocalBindings<nir::RuntimeId>,
    pub(super) call_regions: super::local_bindings::Bindings<n::SourceRange, ()>,
}

pub(super) enum Place {
    Global(GlobalRuntimeOrigin),
    Local(LocalRuntimeOrigin),
}

impl Checker<'_, '_> {
    pub(super) fn runtime_place(
        &mut self,
        mut expr: &n::Expr,
        span: Span,
    ) -> Result<Option<Place>, Error> {
        let mut fields = Vec::new();
        let mut address = false;
        loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "runtime place inspection budget exhausted"))?;
            match &expr.kind {
                K::PointerField { pointer, index, .. }
                | K::PointerAnonymousField { pointer, index, .. }
                    if !address =>
                {
                    fields.push(*index);
                    expr = pointer;
                }
                K::AddressOf(place) if !address => {
                    address = true;
                    expr = place;
                }
                K::Field { value, index, .. } | K::AnonymousField { value, index, .. }
                    if address =>
                {
                    fields.push(*index);
                    expr = value;
                }
                K::Local(binding) => {
                    fields.reverse();
                    let origin = if address {
                        LocalRuntimeOrigin::AddressOf {
                            base: *binding,
                            fields,
                        }
                    } else {
                        LocalRuntimeOrigin::LocalValue {
                            binding: *binding,
                            epoch: self.local_write_versions.get(binding).copied().unwrap_or(0),
                            fields,
                        }
                    };
                    return Ok(Some(Place::Local(origin)));
                }
                K::ConstAddress { declaration, .. } if !address => {
                    fields.reverse();
                    return Ok(Some(Place::Global(GlobalRuntimeOrigin::ConstAddress {
                        declaration: *declaration,
                        ty: self.copy_alias_type(&expr.ty, span)?,
                        fields,
                    })));
                }
                _ => return Ok(None),
            }
        }
    }

    pub(super) fn intern_global_runtime_operand(
        &mut self,
        key: GlobalRuntimeOrigin,
        expr: n::Expr,
        span: Span,
    ) -> Result<nir::RuntimeId, Error> {
        if let Some(id) = self.global_runtime_origin_ids.get(&key) {
            return Ok(*id);
        }
        let id = self.intern_runtime_id(Term::Lift(Box::new(expr.ty.clone())), span)?;
        self.global_runtime_origin_ids.insert(key, id);
        self.global_runtime_deliveries.insert(id, expr);
        Ok(id)
    }

    pub(super) fn intern_runtime_operand(
        &mut self,
        expr: n::Expr,
        supplier: &Expr<'_>,
        span: Span,
    ) -> Result<Term, Error> {
        let origin = match self.runtime_place(&expr, span)? {
            Some(Place::Global(origin)) => {
                return self
                    .intern_global_runtime_operand(origin, expr, span)
                    .map(Term::Runtime);
            }
            Some(Place::Local(origin)) => origin,
            None => LocalRuntimeOrigin::Ephemeral {
                occurrence: self.next_runtime_id,
                site: local(supplier.hir_id),
            },
        };
        self.intern_local_runtime_operand(origin, expr, span)
    }

    pub(super) fn intern_local_runtime_operand(
        &mut self,
        origin: LocalRuntimeOrigin,
        expr: n::Expr,
        span: Span,
    ) -> Result<Term, Error> {
        if let Some(id) = self.local_runtime_origins.ids.get(&origin) {
            return Ok(Term::Runtime(*id));
        }
        let checked_epoch = match &origin {
            LocalRuntimeOrigin::LocalValue { binding, epoch, .. } => Some((*binding, *epoch)),
            _ => None,
        };
        let id = self.intern_runtime_id(Term::Lift(Box::new(expr.ty.clone())), span)?;
        let projection = match &origin {
            LocalRuntimeOrigin::Projection(path) => Some(path.clone()),
            _ => None,
        };
        let producer_site = match &origin {
            LocalRuntimeOrigin::Ephemeral { site, .. } => Some(*site),
            _ => None,
        };
        self.local_runtime_origins.ids.insert(origin, id);
        self.local_runtime_deliveries.insert(
            id,
            LocalRuntimeDelivery {
                producer_site,
                expr,
                checked_epoch,
                projection,
            },
        );
        Ok(Term::Runtime(id))
    }
}
