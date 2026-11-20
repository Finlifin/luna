//! Runtime place identity follows the root storage or pointer binding and its
//! checked field indices. Reading a pointer binding also fixes its write epoch.
use super::*;

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
        span: Span,
    ) -> Result<Term, Error> {
        let origin = match self.runtime_place(&expr, span)? {
            Some(Place::Global(origin)) => {
                return self
                    .intern_global_runtime_operand(origin, expr, span)
                    .map(Term::Runtime);
            }
            Some(Place::Local(origin)) => origin,
            None => LocalRuntimeOrigin::Ephemeral(self.next_runtime_id),
        };
        if let Some(id) = self.local_runtime_origin_ids.get(&origin) {
            return Ok(Term::Runtime(*id));
        }
        let checked_epoch = match &origin {
            LocalRuntimeOrigin::LocalValue { binding, epoch, .. } => Some((*binding, *epoch)),
            _ => None,
        };
        let id = self.intern_runtime_id(Term::Lift(Box::new(expr.ty.clone())), span)?;
        self.local_runtime_origin_ids.insert(origin, id);
        self.local_runtime_deliveries.insert(
            id,
            LocalRuntimeDelivery {
                expr,
                checked_epoch,
            },
        );
        Ok(Term::Runtime(id))
    }
}
