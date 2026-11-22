//! Use the checked call's source order to collect inputs. A reference slot
//! names caller storage while subsequent explicit arguments are evaluated.
use super::flow::Returns;
use super::*;

impl Checker<'_, '_> {
    pub(super) fn observe_factory_call(
        &mut self,
        expr: &n::Expr,
        mut caller: Option<&mut ObservationScope>,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
            env_arguments,
        } = &expr.kind
        else {
            return Ok(Completion::Unavailable);
        };
        if depth >= 32 || !env_arguments.is_empty() {
            return Ok(Completion::Unavailable);
        }
        let observed_target;
        let target = match &callee.kind {
            K::Function(id) => Some((*id, &[][..], false)),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Some((*declaration, arguments.as_slice(), false)),
            _ => {
                observed_target = match self.observe_call_operand(
                    callee,
                    caller.as_deref_mut(),
                    returns,
                    span,
                    depth + 1,
                )? {
                    Completion::Next(target) => target,
                    transfer => return Ok(transfer),
                };
                match observed_target.known_term() {
                    Some(Term::FunctionValue {
                        declaration,
                        arguments,
                    }) => Some((*declaration, arguments.as_slice(), true)),
                    _ => None,
                }
            }
        };
        let mut remaining = arguments.as_slice();
        let mut inputs = BTreeMap::new();
        let mut receiver = None;
        let mut reference_slot = None;
        while let Some((argument, rest)) = remaining.split_first() {
            if argument.is_default {
                break;
            }
            remaining = rest;
            let operand = &argument.value;
            if argument.passing != n::ArgumentPassing::Value {
                if argument.parameter != 0 || reference_slot.replace(argument.parameter).is_some() {
                    return Ok(Completion::Unavailable);
                }
                if let Some(place) =
                    self.schema_fuel(span, |_, fuel| structural_place(operand, fuel))?
                {
                    if caller.is_none() {
                        return Ok(Completion::Unavailable);
                    }
                    receiver = Some((argument, place));
                    continue;
                }
                // A computed receiver owns temporary storage. Its writes can
                // affect the result but there is no place to update on return.
            }
            let value = match self.observe_call_operand(
                operand,
                caller.as_deref_mut(),
                returns,
                span,
                depth + 1,
            )? {
                Completion::Next(value) => value,
                transfer => return Ok(transfer),
            };
            inputs.insert(argument.parameter, value);
        }

        // An explicit operand can return from its caller before invoking an
        // otherwise unknown target.
        let Some((id, indices, dynamic)) = target else {
            return Ok(Completion::Unavailable);
        };
        if let Some((argument, place)) = &receiver {
            let Some(frame) = caller.as_deref() else {
                return Ok(Completion::Unavailable);
            };
            if !frame.unknown_outer_locals && !frame.values.contains_key(&place.root) {
                return Ok(Completion::Unavailable);
            }
            let value = self.schema_fuel(span, |this, fuel| {
                this.partial_staged_value_in(&argument.value, 0, fuel, frame)
            })?;
            inputs.insert(argument.parameter, value);
        }

        if !remaining.is_empty() {
            let substitution = if dynamic && !indices.is_empty() {
                let Some(substitution) = self.schema_fuel(span, |this, fuel| {
                    this.instance_default_substitution(id, indices, fuel)
                })?
                else {
                    return Ok(Completion::Unavailable);
                };
                Some(substitution)
            } else {
                None
            };
            if !self.observe_default_inputs(
                remaining,
                parameter_bindings,
                substitution.as_ref(),
                reference_slot,
                &mut inputs,
                span,
                depth,
            )? {
                return Ok(Completion::Unavailable);
            }
        }
        let slot = receiver.as_ref().map(|(argument, _)| argument.parameter);
        let Some(exit) = self.invoke_observed_factory(id, indices, &inputs, slot, span, depth)?
        else {
            return Ok(Completion::Unavailable);
        };
        if let Some((_, place)) = receiver {
            let (Some(frame), Some(updated)) = (caller, exit.receiver) else {
                return Ok(Completion::Unavailable);
            };
            if !self.schema_fuel(span, |this, fuel| {
                update_place(frame, &place, updated, this.protocols.meta_list, fuel)
            })? {
                return Ok(Completion::Unavailable);
            }
        }
        Ok(Completion::Next(exit.value))
    }

    fn observe_call_operand(
        &mut self,
        expr: &n::Expr,
        caller: Option<&mut ObservationScope>,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        if let Some(frame) = caller {
            return self.observe_factory_expr(expr, frame, returns, span, depth);
        }
        self.schema_fuel(span, |this, fuel| {
            if structural_operand(expr, None, fuel)? {
                this.partial_staged_value(expr, 0, fuel)
                    .map(Completion::Next)
            } else {
                Ok(Completion::Unavailable)
            }
        })
    }

    fn observe_default_inputs(
        &mut self,
        defaults: &[n::CallArgument],
        bindings: &[n::LocalId],
        substitution: Option<&nir::DeclarationSubstitution<'_>>,
        reference_slot: Option<usize>,
        inputs: &mut BTreeMap<usize, Arc<PartialStagedValue>>,
        span: Span,
        depth: usize,
    ) -> Result<bool, Error> {
        let mut frame = ObservationScope::default();
        self.schema_fuel(span, |_, fuel| {
            for (&slot, value) in inputs.iter() {
                if let Some(binding) = bindings.get(slot) {
                    frame.insert(*binding, value.clone(), fuel)?;
                }
            }
            Ok(())
        })?;
        for argument in defaults {
            if !argument.is_default || argument.passing != n::ArgumentPassing::Value {
                return Ok(false);
            }
            let specialized;
            let expr = if let Some(substitution) = substitution {
                specialized = self.schema_fuel(span, |_, fuel| {
                    substitution
                        .instantiate_expression_bounded(&argument.value, fuel)
                        .map_err(|error| error.to_string())
                })?;
                &specialized
            } else {
                &argument.value
            };
            let Completion::Next(value) =
                self.observe_factory_expr(expr, &mut frame, &mut None, span, depth + 1)?
            else {
                return Ok(false);
            };
            if let Some(binding) = bindings.get(argument.parameter) {
                self.schema_fuel(span, |_, fuel| frame.insert(*binding, value.clone(), fuel))?;
            }
            inputs.insert(argument.parameter, value);
        }
        // Defaults see the supplied value slots through private bindings.
        // Only a reference receiver shares storage with the eventual callee.
        if let Some(slot) = reference_slot
            && let Some(value) = bindings.get(slot).and_then(|id| frame.values.get(id))
        {
            inputs.insert(slot, value.clone());
        }
        Ok(true)
    }
}
