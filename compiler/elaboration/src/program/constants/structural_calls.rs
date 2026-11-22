//! Inspect the successful result of a checked comptime factory. The lexical
//! store contains field observations and never owns invocation storage.
use super::*;
use std::sync::Arc;

impl Checker<'_, '_> {
    pub(in crate::program) fn record_generator_initializer(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        self.record_observed_initializer(pattern, value, span)?;
        let K::Call {
            callee,
            arguments,
            env_arguments,
            parameter_bindings,
        } = &value.kind
        else {
            return Ok(());
        };
        let K::Function(id) = callee.kind else {
            return Ok(());
        };
        if !env_arguments.is_empty()
            || !matches!(
                &value.ty,
                Term::Nominal { .. } | Term::NormalForm { .. } | Term::Record(_) | Term::Tuple(_)
            )
        {
            return Ok(());
        }
        let Some(owner) = self.source_declarations.owner(id) else {
            return Ok(());
        };
        let Some(item) = self.package.item(owner) else {
            return Ok(());
        };
        let ItemKind::Fn(sig, _) = &item.kind else {
            return Ok(());
        };
        if !sig.modifiers.is_comptime
            || sig.modifiers.is_function_template
            || sig.modifiers.is_normal_form_head
            || sig.modifiers.is_quote
            || self
                .type_parameters
                .get(&owner)
                .is_some_and(|parameters| !parameters.is_empty())
            || self.is_code_generator(owner)
            || self.data_consumers.contains_key(&owner)
        {
            return Ok(());
        }
        // Explicit operands read the caller. Defaults read formals that have
        // already been supplied, in the order recorded by the checked call.
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let inputs = (|| {
            let mut inputs = BTreeMap::new();
            let mut defaults = ObservationScope::default();
            for argument in arguments {
                let frame = argument.is_default.then_some(&defaults);
                if argument.passing != n::ArgumentPassing::Value
                    || !structural_operand(&argument.value, frame, &mut fuel)?
                {
                    return Ok(None);
                }
                let observed = match frame {
                    Some(frame) => {
                        self.partial_staged_value_in(&argument.value, 0, &mut fuel, frame)?
                    }
                    None => self.partial_staged_value(&argument.value, 0, &mut fuel)?,
                };
                if let Some(formal) = parameter_bindings.get(argument.parameter) {
                    defaults.insert(*formal, observed.clone(), &mut fuel)?;
                }
                inputs.insert(argument.parameter, observed);
            }
            Ok(Some(inputs))
        })();
        self.required_steps -= (before - fuel) as u64;
        let Some(inputs) = inputs.map_err(|e: String| error(span, e))? else {
            return Ok(());
        };

        // Declaration checking enters the callee's lexical scope. It produces
        // NIR even when this generator precedes the factory in source order.
        if self.observing_factory_bodies.len() >= 64 || !self.observing_factory_bodies.insert(owner)
        {
            return Ok(());
        }
        let function = self.required_function(owner, span);
        self.observing_factory_bodies.remove(&owner);
        let mut function = function?;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            if inputs.len() != function.parameters.len() {
                return Ok(None);
            }
            let Term::Function { parameters, .. } = function.signature.callable_signature() else {
                return Ok(None);
            };
            if parameters.iter().any(|p| p.sort != Sort::Runtime) {
                nir::type_computations::charge_term(&function.signature, &mut fuel)
                    .map_err(|e| e.to_string())?;
                let mut frame = nir::FunctionInputBuilder::new(&function.signature)
                    .map_err(|e| e.to_string())?;
                for (index, parameter) in parameters.iter().enumerate() {
                    frame
                        .next_type_bounded(&mut fuel)
                        .map_err(|e| e.to_string())?;
                    let input = if parameter.sort == Sort::Runtime {
                        None
                    } else if let Some(term) = inputs.get(&index).and_then(|v| v.known_term()) {
                        nir::type_computations::charge_term(term, &mut fuel)
                            .map_err(|e| e.to_string())?;
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
                let binding = frame.finish_bounded(&mut fuel).map_err(|e| e.to_string())?;
                function.body.rewrite_types(&mut |term| {
                    binding
                        .rebind_bounded(term, &mut fuel)
                        .map_err(|e| e.to_string())
                })?;
            }
            let mut scope = ObservationScope::default();
            for (index, binding) in function.parameters.iter().enumerate() {
                let Some(input) = inputs.get(&index) else {
                    return Ok(None);
                };
                scope.insert(*binding, input.clone(), &mut fuel)?;
            }
            self.structural_body_result(&function.body, &mut scope, &mut fuel)
        })();
        self.required_steps -= (before - fuel) as u64;
        if let Some(result) = result.map_err(|e: String| error(span, e))? {
            self.record_observed_pattern(pattern, &result, span)?;
        }
        Ok(())
    }

    fn structural_body_result(
        &self,
        mut expr: &n::Expr,
        scope: &mut ObservationScope,
        fuel: &mut usize,
    ) -> Result<Option<Arc<PartialStagedValue>>, String> {
        loop {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("structural call observation budget exhausted")?;
            match &expr.kind {
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    for statement in statements {
                        let statement = match &statement.kind {
                            K::Discard(value) => value.as_ref(),
                            _ => statement,
                        };
                        match &statement.kind {
                            K::Let { binding, value }
                                if structural_operand(value, Some(scope), fuel)? =>
                            {
                                let observed =
                                    self.partial_staged_value_in(value, 0, fuel, scope)?;
                                scope.insert(*binding, observed, fuel)?;
                            }
                            K::Assign { place, value }
                                if structural_operand(value, Some(scope), fuel)? =>
                            {
                                let Some(root) = scope.values.get(&place.root) else {
                                    return Ok(None);
                                };
                                let value = self.partial_staged_value_in(value, 0, fuel, scope)?;
                                let Some(updated) = root.with_field_at(
                                    &place.fields,
                                    value,
                                    self.protocols.meta_list,
                                    fuel,
                                )?
                                else {
                                    return Ok(None);
                                };
                                scope.insert(place.root, updated, fuel)?;
                            }
                            _ => return Ok(None),
                        }
                    }
                    let Some(tail) = result else { return Ok(None) };
                    expr = tail;
                }
                K::Return(Some(value)) => expr = value,
                _ if structural_operand(expr, Some(scope), fuel)? => {
                    return self.partial_staged_value_in(expr, 0, fuel, scope).map(Some);
                }
                _ => return Ok(None),
            }
        }
    }
}

// These NIR operations only read operands. Unsupported operations in a
// discarded statement can still write a receiver or leave the function.
fn structural_operand(
    expr: &n::Expr,
    frame: Option<&ObservationScope>,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut transparent = true;
    nir::visit::expression_pruned(expr, fuel, |node, _| {
        if let K::Local(id) = node.kind
            && frame.is_some_and(|scope| !scope.values.contains_key(&id))
        {
            transparent = false;
        }
        transparent &= matches!(
            node.kind,
            K::Local(_)
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
        );
        Ok(transparent)
    })?;
    Ok(transparent)
}
