//! Inspect the successful result of a checked comptime factory. The lexical
//! store contains field observations and never owns invocation storage.
use super::*;
use std::sync::Arc;

mod arguments;
mod bits;
mod conditions;
mod flow;
mod instances;
mod matching;
use flow::{Completion, FactoryExit};

impl Checker<'_, '_> {
    pub(in crate::program) fn record_generator_initializer(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        self.record_observed_initializer(pattern, value, span)?;
        if matches!(
            &value.ty,
            Term::Nominal { .. } | Term::NormalForm { .. } | Term::Record(_) | Term::Tuple(_)
        ) && let Completion::Next(observed) =
            self.observe_factory_call(value, None, &mut None, span, 0)?
        {
            self.record_observed_pattern(pattern, &observed, span)?;
        }
        Ok(())
    }

    pub(super) fn schema_fuel<T>(
        &mut self,
        span: Span,
        inspect: impl FnOnce(&Self, &mut usize) -> Result<T, String>,
    ) -> Result<T, Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = inspect(self, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| error(span, e))
    }

    fn invoke_observed_factory(
        &mut self,
        id: DeclarationId,
        indices: &[Term],
        inputs: &BTreeMap<usize, Arc<PartialStagedValue>>,
        receiver: Option<usize>,
        span: Span,
        depth: usize,
    ) -> Result<Option<FactoryExit>, Error> {
        // Compiler-provided operations also have checked bodies. Dispatch
        // through that body so wrappers and user methods share the same rules.
        let owner = self.source_declarations.owner(id);
        let generated = self.generated_functions.body(id).is_some();
        if !generated {
            let Some(owner) = owner else { return Ok(None) };
            let Some(ItemKind::Fn(sig, _)) = self.package.item(owner).map(|item| &item.kind) else {
                return Ok(None);
            };
            if !sig.modifiers.is_comptime
                || sig.modifiers.is_function_template
                || sig.modifiers.is_normal_form_head
                || sig.modifiers.is_quote
                || self.is_code_generator(owner)
                || self.data_consumers.contains_key(&owner)
                || self.observing_factory_bodies.len() >= 64
                || !self.observing_factory_bodies.insert(owner)
            {
                return Ok(None);
            }
        }
        let result = (|| {
            let mut function = if generated {
                nir::copy_budget::clone_function(
                    self.generated_functions.body(id).unwrap(),
                    &mut self.required_steps,
                )
                .map_err(|_| error(span, "structural callable body budget exhausted"))?
            } else {
                self.required_function(owner.unwrap(), span)?
            };
            if function.phase != n::FunctionPhase::Comptime
                || inputs.len() != function.parameters.len()
                || indices.len() != function.type_parameters.len()
                || !function.preconditions.is_empty()
                || !function.postconditions.is_empty()
            {
                return Ok(None);
            }
            if !indices.is_empty() {
                function = self.schema_fuel(span, |this, fuel| {
                    let domains = this.factory_input_domains(&function.type_parameters, fuel)?;
                    function
                        .instantiate_declaration_parameters(&domains, indices, fuel)
                        .map_err(|e| e.to_string())
                })?;
            }
            let Some(mut frame) =
                self.schema_fuel(span, |_, fuel| bind_inputs(&mut function, inputs, fuel))?
            else {
                return Ok(None);
            };
            let receiver = receiver.and_then(|slot| function.parameters.get(slot).copied());
            self.observe_factory_body(&function.body, &mut frame, receiver, span, depth)
        })();
        if !generated {
            self.observing_factory_bodies.remove(&owner.unwrap());
        }
        result
    }
}

fn update_place(
    scope: &mut ObservationScope,
    place: &n::Place,
    value: Arc<PartialStagedValue>,
    meta_list: Option<DeclarationId>,
    fuel: &mut usize,
) -> Result<bool, String> {
    let Some(root) = scope.values.get(&place.root) else {
        return Ok(false);
    };
    let Some(updated) = root.with_field_at(&place.fields, value, meta_list, fuel)? else {
        return Ok(false);
    };
    scope.insert(place.root, updated, fuel)?;
    Ok(true)
}

fn structural_place(mut expr: &n::Expr, fuel: &mut usize) -> Result<Option<n::Place>, String> {
    let mut fields = Vec::new();
    loop {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("structural receiver observation budget exhausted")?;
        match &expr.kind {
            K::Local(root) => {
                fields.reverse();
                return Ok(Some(n::Place {
                    root: *root,
                    fields,
                }));
            }
            K::Field {
                value,
                declaration,
                index,
            } => {
                fields.push((n::FieldOwner::Nominal(*declaration), *index));
                expr = value;
            }
            K::AnonymousField { value, index, .. } => {
                fields.push((n::FieldOwner::Anonymous, *index));
                expr = value;
            }
            _ => return Ok(None),
        }
    }
}

fn bind_inputs(
    function: &mut n::Function,
    inputs: &BTreeMap<usize, Arc<PartialStagedValue>>,
    fuel: &mut usize,
) -> Result<Option<ObservationScope>, String> {
    let Term::Function { parameters, .. } = function.signature.callable_signature() else {
        return Ok(None);
    };
    if parameters.iter().any(|p| p.sort != Sort::Runtime) {
        nir::type_computations::charge_term(&function.signature, fuel)
            .map_err(|e| e.to_string())?;
        let mut frame =
            nir::FunctionInputBuilder::new(&function.signature).map_err(|e| e.to_string())?;
        for (index, parameter) in parameters.iter().enumerate() {
            frame.next_type_bounded(fuel).map_err(|e| e.to_string())?;
            let input = if parameter.sort == Sort::Runtime {
                None
            } else if let Some(term) = inputs.get(&index).and_then(|v| v.known_term()) {
                nir::type_computations::charge_term(term, fuel).map_err(|e| e.to_string())?;
                Some(term.clone())
            } else {
                return Ok(None);
            };
            if frame.push(input).is_err() {
                // Open caller inputs can be valid without supplying a
                // closed identity for this observation.
                return Ok(None);
            }
        }
        let binding = frame.finish_bounded(fuel).map_err(|e| e.to_string())?;
        function.body.rewrite_types(&mut |term| {
            binding
                .rebind_bounded(term, fuel)
                .map_err(|e| e.to_string())
        })?;
    }
    let mut scope = ObservationScope::default();
    for (index, binding) in function.parameters.iter().enumerate() {
        let Some(input) = inputs.get(&index) else {
            return Ok(None);
        };
        scope.insert(*binding, input.clone(), fuel)?;
    }
    Ok(Some(scope))
}

// These NIR operations only read operands. Unsupported operations in a
// discarded statement can still write a receiver or leave the function.
pub(super) fn structural_operand(
    expr: &n::Expr,
    frame: Option<&ObservationScope>,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut transparent = true;
    nir::visit::expression_pruned(expr, fuel, |node, _| {
        if let K::Local(id) = node.kind
            && frame
                .is_some_and(|scope| !scope.unknown_outer_locals && !scope.values.contains_key(&id))
        {
            transparent = false;
        }
        transparent &= matches!(
            node.kind,
            K::Local(_)
                | K::Function(_)
                | K::FunctionInstance { .. }
                | K::Literal(_)
                | K::Unit
                | K::TypeValue(_)
                | K::ComptimeValue(_)
                | K::StagedValue(_)
                | K::Tuple(_)
                | K::Record { .. }
                | K::AnonymousRecord { .. }
                | K::Field { .. }
                | K::AnonymousField { .. }
                | K::TypeList(_)
                | K::TypeListIndex(..)
                | K::ListConcat(..)
                | K::ListLength(_)
                | K::Not(_)
                | K::And(..)
                | K::Or(..)
                | K::ScalarEqual(..)
                | K::ScalarLess(..)
                | K::ScalarLessEqual(..)
                | K::ScalarGreater(..)
                | K::ScalarGreaterEqual(..)
        );
        Ok(transparent)
    })?;
    Ok(transparent)
}
