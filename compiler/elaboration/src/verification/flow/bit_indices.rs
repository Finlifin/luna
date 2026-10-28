//! Endpoint snapshots and constant-width proofs belong to program verification.
use super::*;

impl Walker<'_> {
    pub(super) fn bit_window(
        &mut self,
        expr: &Expr,
        value: &Expr,
        start: &Expr,
        end: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<(), Reason> {
        self.child(value, state, depth)?;
        let mut lower = self.child(start, state, depth)?;
        let start_type = self.scalar_type_view(&start.ty)?;
        // Bind the already-evaluated endpoint before evaluating the upper one.
        // A call or write in the upper endpoint cannot retroactively change it.
        let root = if state.facts.is_reachable() {
            self.result_root(&start_type)?
        } else {
            None
        };
        if let Some(root) = root {
            lower.evidence = self.prepare_copy(&lower, state)?;
            self.bind_result(state, root, &start_type, lower)?;
            state.frozen.insert(root, start_type.clone());
            lower = self.local_value_at(state, root)?;
        }
        let upper = self.child(end, state, depth);
        if let Some(root) = root {
            state.frozen.remove(&root);
        }
        let upper = upper?;
        if !state.facts.is_reachable() {
            return Ok(());
        }
        if let Some(root) = root {
            lower = self.local_value_at(state, root)?;
        }
        let Term::Primitive(nir::Primitive::BitVec(input_width)) =
            self.scalar_type_view(&value.ty)?
        else {
            return Err(Reason::Unsupported);
        };
        let Term::Primitive(nir::Primitive::BitVec(width)) = expr.ty else {
            return Err(Reason::Unsupported);
        };
        let end_type = self.scalar_type_view(&end.ty)?;
        for ty in [&start_type, &end_type] {
            if !matches!(ty, Term::Primitive(nir::Primitive::Integer))
                && !matches!(ty, Term::Primitive(p) if self.integer_layout(*p).is_some())
            {
                return Err(Reason::Unsupported);
            }
        }
        let maximum = Integer::from_u128(input_width as u128);
        for endpoint in [&lower, &upper] {
            let range = endpoint
                .range
                .as_ref()
                .ok_or(Reason::MissingBitWindowProof)?;
            if range.lower().is_negative() || self.limits.compare(range.upper(), &maximum)?.is_gt()
            {
                return Err(Reason::MissingBitWindowProof);
            }
        }
        let lo = lower
            .polynomial(self.limits)?
            .ok_or(Reason::MissingBitWindowProof)?;
        let hi = upper
            .polynomial(self.limits)?
            .ok_or(Reason::MissingBitWindowProof)?;
        let width = Polynomial::constant(Integer::from_u128(width as u128), self.limits)?;
        let condition = Condition::PolynomialCompare {
            left: hi.subtract(&lo, self.limits)?,
            relation: C::Equal,
            right: width,
        };
        let proof = state
            .facts
            .prove_condition(&condition, true, self.limits)
            .map_err(|_| Reason::MissingBitWindowProof)?;
        proof
            .replay(&state.facts, self.limits)
            .map_err(|_| Reason::MissingBitWindowProof)?;
        self.record(expr, state, PointKind::BitWindow, lower.range.as_ref())?;
        Ok(())
    }
    pub(super) fn bit_index(
        &mut self,
        expr: &Expr,
        value: &Expr,
        index: &Expr,
        state: &mut State,
        depth: usize,
    ) -> Result<(), Reason> {
        self.child(value, state, depth)?;
        let position = self.child(index, state, depth)?;
        if state.facts.is_reachable() {
            let Term::Primitive(nir::Primitive::BitVec(width)) =
                self.scalar_type_view(&value.ty)?
            else {
                return Err(Reason::Unsupported);
            };
            let position_type = self.scalar_type_view(&index.ty)?;
            if !matches!(position_type, Term::Primitive(nir::Primitive::Integer))
                && !matches!(position_type, Term::Primitive(p) if self.integer_layout(p).is_some())
            {
                return Err(Reason::Unsupported);
            }
            if expr.ty != Term::Primitive(nir::Primitive::BitVec(1)) {
                return Err(Reason::Unsupported);
            }
            let range = position
                .range
                .as_ref()
                .ok_or(Reason::MissingBitIndexProof)?;
            if range.lower().is_negative()
                || self
                    .limits
                    .compare(range.upper(), &Integer::from_u128(width as u128))?
                    != std::cmp::Ordering::Less
            {
                return Err(Reason::MissingBitIndexProof);
            }
            self.record(expr, state, PointKind::BitIndex, Some(range))?;
        }

        Ok(())
    }
    pub(super) fn bit_selections(
        &mut self,
        expr: &Expr,
        storage: &Term,
        selections: &[nir::program::BitSelection],
        state: &mut State,
        depth: usize,
    ) -> Result<(), Reason> {
        let mut width = match self.scalar_type_view(storage)? {
            Term::Primitive(nir::Primitive::BitVec(width)) => width,
            Term::Primitive(p) => p
                .unrestricted_scalar_width_in(self.target())
                .ok_or(Reason::Unsupported)? as usize,
            _ => return Err(Reason::Unsupported),
        };
        for selection in selections {
            self.limits.step(1)?;
            let next = selection
                .output_width(width)
                .ok_or(Reason::MissingBitWindowProof)?;
            let dummy = Expr {
                source: expr.source,
                ty: Term::Primitive(nir::Primitive::BitVec(width)),
                kind: K::Unit,
            };
            let result = Expr {
                source: expr.source,
                ty: Term::Primitive(nir::Primitive::BitVec(next)),
                kind: K::Unit,
            };
            match selection {
                nir::program::BitSelection::Static { .. } => (),
                nir::program::BitSelection::Index(index) => {
                    self.bit_index(&result, &dummy, index, state, depth)?
                }
                nir::program::BitSelection::Window { start, end, .. } => {
                    self.bit_window(&result, &dummy, start, end, state, depth)?
                }
            }
            width = next;
        }
        Ok(())
    }
}
