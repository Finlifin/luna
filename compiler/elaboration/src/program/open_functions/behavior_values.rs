//! Runtime behavior witnesses for local const values. Their staged producer
//! remains available to later type observations in the declaration scope.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn retain_behavior_value(
        &mut self,
        binding: n::LocalId,
        producer: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        constants::charge_copy(producer, &mut self.required_steps, span)?;
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = (|| {
            let mut value = producer.clone();
            let mut next = self.next_temporary_binding;
            let mut inputs = Vec::new();
            nir::visit::expression_mut_pruned(&mut value, &mut fuel, |node, fuel| {
                match &node.kind {
                    K::Unit
                    | K::Literal(_)
                    | K::TypeValue(_)
                    | K::ComptimeValue(_)
                    | K::Function(_)
                    | K::FunctionInstance { .. }
                    | K::TraitMethod { .. }
                    | K::Local(_)
                    | K::OptionalAbsent => return Ok(false),
                    K::Block { statements, .. } if statements.is_empty() => return Ok(true),
                    K::If { .. }
                    | K::Tuple(_)
                    | K::Array(_)
                    | K::Record { .. }
                    | K::AnonymousRecord { .. }
                    | K::Variant { .. }
                    | K::Field { .. }
                    | K::AnonymousField { .. }
                    | K::OptionalPresent(_)
                    | K::ErrorConvert { .. } => return Ok(true),
                    _ => {}
                }
                // Only the result crosses the stage boundary. In particular a
                // staged call inside a field or selector incurs no runtime
                // effect. Preserve the surrounding callable and data choices.
                let ty = nir::copy_budget::clone_term(&node.ty, fuel)
                    .map_err(|_| "constant behavior input type budget exhausted")?;
                let index = next;
                next = index
                    .checked_add(1)
                    .ok_or("constant behavior input identity space exhausted")?;
                let input = n::LocalId {
                    owner: u32::MAX,
                    index,
                };
                inputs.push((input, ty));
                node.kind = K::Local(input);
                Ok(false)
            })?;
            if !inputs.is_empty() {
                self.next_temporary_binding = next;
                self.ordinary_behavior_inputs.extend(inputs);
                self.definition_refinements
                    .behavior_values
                    .insert(binding, value);
            } else {
                self.definition_refinements.behavior_values.remove(&binding);
            }
            Ok::<_, String>(())
        })();
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|message| error(span, message))
    }
}
