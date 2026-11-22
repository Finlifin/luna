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
        if matches!(
            &value.ty,
            Term::Nominal { .. } | Term::NormalForm { .. } | Term::Record(_) | Term::Tuple(_)
        ) && let Some(observed) = self.structural_call(value, None, span, 0)?
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

    fn structural_call(
        &mut self,
        expr: &n::Expr,
        caller: Option<&mut ObservationScope>,
        span: Span,
        depth: usize,
    ) -> Result<Option<Arc<PartialStagedValue>>, Error> {
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
            env_arguments,
        } = &expr.kind
        else {
            return Ok(None);
        };
        if depth >= 32 || !env_arguments.is_empty() {
            return Ok(None);
        }
        let (id, indices) = match &callee.kind {
            K::Function(id) => (*id, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            _ => return Ok(None),
        };
        let Some((inputs, writeback)) = self.schema_fuel(span, |this, fuel| {
            let mut inputs = BTreeMap::new();
            let mut defaults = ObservationScope::default();
            let mut writeback = None;
            for argument in arguments {
                let frame = if argument.is_default {
                    Some(&defaults)
                } else {
                    caller.as_deref()
                };
                if !structural_operand(&argument.value, frame, fuel)? {
                    return Ok(None);
                }
                if argument.passing != n::ArgumentPassing::Value {
                    // The receiver aliases one place in its caller. Suspend
                    // observation when another reference could overlap it.
                    if caller.is_none() || argument.is_default || writeback.is_some() {
                        return Ok(None);
                    }
                    let Some(place) = structural_place(&argument.value, fuel)? else {
                        return Ok(None);
                    };
                    writeback = Some((argument.parameter, place));
                }
                let value = match frame {
                    Some(frame) => this.partial_staged_value_in(&argument.value, 0, fuel, frame)?,
                    None => this.partial_staged_value(&argument.value, 0, fuel)?,
                };
                if let Some(formal) = parameter_bindings.get(argument.parameter) {
                    defaults.insert(*formal, value.clone(), fuel)?;
                }
                inputs.insert(argument.parameter, value);
            }
            Ok(Some((inputs, writeback)))
        })?
        else {
            return Ok(None);
        };

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
                    let mut domains = Vec::new();
                    for parameter in &function.type_parameters {
                        let domain = this
                            .observe_input_domain(*parameter, || {
                                *fuel = fuel
                                    .checked_sub(1)
                                    .ok_or(nir::type_computations::Error::Budget)?;
                                Ok(())
                            })
                            .map_err(|e| e.to_string())?;
                        domains.push(
                            nir::copy_budget::clone_term(
                                domain.unwrap_or(&Term::Primitive(P::Type)),
                                fuel,
                            )
                            .map_err(|_| {
                                "structural declaration domain budget exhausted".to_string()
                            })?,
                        );
                    }
                    function
                        .instantiate_declaration_parameters(&domains, indices, fuel)
                        .map_err(|e| e.to_string())
                })?;
            }
            let Some(mut frame) =
                self.schema_fuel(span, |_, fuel| bind_inputs(&mut function, &inputs, fuel))?
            else {
                return Ok(None);
            };
            let Some(result) =
                self.structural_body_result(&function.body, &mut frame, span, depth)?
            else {
                return Ok(None);
            };
            if let Some((index, place)) = writeback {
                let Some(updated) = function
                    .parameters
                    .get(index)
                    .and_then(|id| frame.values.get(id))
                else {
                    return Ok(None);
                };
                if !self.schema_fuel(span, |this, fuel| {
                    update_place(
                        caller.unwrap(),
                        &place,
                        updated.clone(),
                        this.protocols.meta_list,
                        fuel,
                    )
                })? {
                    return Ok(None);
                }
            }
            Ok(Some(result))
        })();
        if !generated {
            self.observing_factory_bodies.remove(&owner.unwrap());
        }
        result
    }

    pub(super) fn structural_value(
        &mut self,
        expr: &n::Expr,
        scope: &mut ObservationScope,
        span: Span,
        depth: usize,
    ) -> Result<Option<Arc<PartialStagedValue>>, Error> {
        if let K::BuildStruct(packet) = &expr.kind {
            return self.observe_built_struct(packet, scope, span);
        }
        if matches!(expr.kind, K::Call { .. }) {
            return self.structural_call(expr, Some(scope), span, depth + 1);
        }
        self.schema_fuel(span, |this, fuel| {
            if !structural_operand(expr, Some(scope), fuel)? {
                return Ok(None);
            }
            this.partial_staged_value_in(expr, 0, fuel, scope).map(Some)
        })
    }

    fn structural_body_result(
        &mut self,
        mut expr: &n::Expr,
        scope: &mut ObservationScope,
        span: Span,
        depth: usize,
    ) -> Result<Option<Arc<PartialStagedValue>>, Error> {
        loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "structural call observation budget exhausted"))?;
            match &expr.kind {
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    for statement in statements {
                        let statement = match &statement.kind {
                            K::Discard(value) => value.as_ref(),
                            _ => statement,
                        };
                        if !self.structural_statement(statement, scope, span, depth)? {
                            return Ok(None);
                        }
                    }
                    let Some(tail) = result else {
                        return Ok(Some(PartialStagedValue::known(Term::Unit)));
                    };
                    expr = tail;
                }
                K::Return(Some(value)) => expr = value,
                K::Assign { .. } => {
                    return Ok(self
                        .structural_statement(expr, scope, span, depth)?
                        .then(|| PartialStagedValue::known(Term::Unit)));
                }
                _ => return self.structural_value(expr, scope, span, depth),
            }
        }
    }

    pub(super) fn structural_statement(
        &mut self,
        expr: &n::Expr,
        scope: &mut ObservationScope,
        span: Span,
        depth: usize,
    ) -> Result<bool, Error> {
        match &expr.kind {
            K::Let { binding, value } => {
                let Some(value) = self.structural_value(value, scope, span, depth)? else {
                    return Ok(false);
                };
                self.schema_fuel(span, |_, fuel| scope.insert(*binding, value, fuel))?;
                Ok(true)
            }
            K::Assign { place, value } => {
                let Some(value) = self.structural_value(value, scope, span, depth)? else {
                    return Ok(false);
                };
                self.schema_fuel(span, |this, fuel| {
                    update_place(scope, place, value, this.protocols.meta_list, fuel)
                })
            }
            K::Call { .. } => Ok(self.structural_value(expr, scope, span, depth)?.is_some()),
            _ => Ok(false),
        }
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
