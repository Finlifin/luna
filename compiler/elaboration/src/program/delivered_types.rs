//! Stable Type values crossing from code generation into ordinary HIR checking.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn delivered_type(
        &mut self,
        id: hir::HirId,
        ty: &std::sync::Arc<Term>,
        span: Span,
    ) -> Result<Term, Error> {
        let id = local(id);
        charge(ty, &mut self.required_steps, span)?;
        if ty.sort() != nir::Sort::Type || ty.check_scope(&[]).is_err() {
            return Err(error(
                span,
                "delivered type requires a stable closed Type description",
            ));
        }
        if let Some((previous, _)) = self.delivered_types.get(&id) {
            if previous != ty {
                return Err(error(
                    span,
                    "delivered type node has inconsistent descriptions",
                ));
            }
        } else {
            self.delivered_types.insert(id, (ty.clone(), span));
        }
        Ok(ty.as_ref().clone())
    }
}

/// Recheck stable inputs against the complete destination declaration world.
pub(super) fn admit(
    inputs: &BTreeMap<n::LocalId, (std::sync::Arc<Term>, Span)>,
    output: &n::Package,
    steps: &mut u64,
) -> Result<(), Error> {
    if inputs.is_empty() {
        return Ok(());
    }
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        output,
        comptime_interpretor::Budget {
            steps: *steps,
            depth: 128,
        },
    )
    .map_err(|failure| {
        error(
            Span::default(),
            format!("delivered type world admission failed: {failure}"),
        )
    })?;
    for (ty, span) in inputs.values() {
        let mut remaining = vm.remaining_steps();
        charge(ty, &mut remaining, *span)?;
        vm.charge_preparation((vm.remaining_steps() - remaining) as usize)
            .map_err(|failure| error(*span, failure.to_string()))?;
        vm.validate_immutable_input(
            &comptime_interpretor::Value::Type(ty.as_ref().clone()),
            &p(P::Type),
        )
        .map_err(|failure| error(*span, format!("delivered type admission failed: {failure}")))?;
    }
    *steps = vm.remaining_steps();
    Ok(())
}

fn charge(ty: &Term, steps: &mut u64, span: Span) -> Result<(), Error> {
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = nir::copy_budget::charge_type_substitution(ty, &[], &[], &mut fuel);
    *steps -= (before - fuel) as u64;
    result.map_err(|failure| {
        error(
            span,
            format!("delivered type copy budget exhausted: {failure:?}"),
        )
    })
}
