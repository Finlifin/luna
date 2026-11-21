//! Check opaque Type constructor arguments before all member identities are known.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn describe_normal_application<'h>(
        &mut self,
        expr: &Expr<'h>,
        head: &Expr<'h>,
        arguments: &[Arg<'h>],
        locals: &mut BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        if matches!(head.kind, H::Builtin(_)) {
            return self.describe_known_region(expr, expected);
        }
        let inputs = self.source_expression_inputs(expr.hir_id.owner, expr)?;
        // A local's domain can be Type while its namespace still depends on
        // the invocation. A projection through that local needs the receiver's
        // identity even when every source binding already has a known domain.
        if inputs.locals.iter().all(|id| self.locals.contains_key(id)) {
            let staged_receiver = inputs.locals.iter().any(|id| {
                (self.open_staged_locals.contains(id) || self.input_facts.is_staged(id))
                    && self.locals.get(id) == Some(&p(P::Type))
                    && !self.generator_data.get(id).is_some_and(Option::is_some)
            });
            if !staged_receiver
                || !self.scan_body_requirement(expr, |this, component| {
                    Ok(matches!(component.kind, H::Projection(..))
                        && this.pending_member_origin(component, locals)?)
                })?
            {
                return self.describe_known_region(expr, expected);
            }
        }
        let (parameters, supply) = if let H::ResolvedDef { def_id, .. } = head.kind {
            let owner = self
                .package
                .definition_owner(def_id)
                .ok_or_else(|| error(expr.span, "missing type application declaration"))?;
            let Some(application) = self.static_normal_application(owner, expr.span)? else {
                return self.describe_known_region(expr, expected);
            };
            self.ensure_signature(application.head, expr.span)?;
            let parameters = self.normal_input_parameters(application.head, expr.span)?;
            (parameters, application.interface)
        } else {
            // The public signature suffices here. The actual opaque head is
            // supplied by the callee value when this application executes.
            let D::Known(ty) =
                self.describe_open_expression(head, locals, returns, None, depth + 1)?
            else {
                return self.describe_known_region(expr, expected);
            };
            if !ty.is_normal_form_callable() {
                return self.describe_known_region(expr, expected);
            }
            let callable = self.open_normal_callable(head, &ty, expr.span)?;
            (
                callable.parameters,
                normal::Supply::Callable {
                    collector: callable.collector,
                },
            )
        };
        let (mut routing, collector) =
            self.describe_normal_routing(supply, parameters.len(), arguments, expr.span)?;
        // Retain source order. The final marker distinguishes a single
        // collected element from a whole input or collection spread.
        let mut routed = Vec::new();
        for argument in arguments {
            match argument {
                Arg::Positional(value) | Arg::Named(_, value) => {
                    let name = match argument {
                        Arg::Named(name, _) => Some(*name),
                        _ => None,
                    };
                    let slot = routing.slot(name, value.span)?;
                    let element = name.is_none() && collector == Some(slot);
                    routed.push((slot, Some(*value), None, value.span, element));
                }
                Arg::Expand(value) => {
                    match routing.spread(value)? {
                        clause_inputs::Spread::Collection(slot) => {
                            routed.push((slot, Some(*value), None, value.span, false));
                            continue;
                        }
                        clause_inputs::Spread::Fields(fields) => {
                            let slots = routing.packet_slots(fields.len(), value.span)?;
                            for (field, slot) in fields.iter().zip(slots) {
                                routed.push((slot, Some(field), None, field.span, false));
                            }
                            continue;
                        }
                        clause_inputs::Spread::Packet => {}
                    }
                    let packet =
                        self.describe_open_expression(value, locals, returns, None, depth + 1)?;
                    let D::Known(ty) = packet else {
                        return Err(error(
                            value.span,
                            "type input packet requires a known tuple domain",
                        ));
                    };
                    let Term::Tuple(fields) = self.structural_type(&ty, value.span)? else {
                        return Err(error(value.span, "type argument spread requires a tuple"));
                    };
                    let slots = routing.packet_slots(fields.len(), value.span)?;
                    for (domain, slot) in fields.into_iter().zip(slots) {
                        routed.push((slot, None, Some(domain), value.span, false));
                    }
                }
                _ => {
                    return Err(error(expr.span, "unsupported normal-form input marker"));
                }
            }
        }
        // A Type-valued head input can substitute a later domain only
        // when its value is available without invocation inputs. Keep
        // the remaining domain as a description, never a fabricated
        // rigid Type value for an ordinary comptime parameter.
        let mut known_parameters = Vec::new();
        let mut known_arguments = Vec::new();
        for (slot, value, _, _, _) in &routed {
            let Some(value) = value else { continue };
            if parameters[*slot].2 == p(P::Type)
                && self
                    .source_expression_inputs(value.hir_id.owner, value)?
                    .locals
                    .is_empty()
            {
                known_parameters.push(parameters[*slot].0);
                known_arguments.push(self.type_expr(value)?);
            }
        }
        for (slot, value, packet_domain, span, element) in routed {
            let declared = if element {
                self.list_element_type(&parameters[slot].2)
                    .ok_or_else(|| error(span, "variadic input requires a bound meta.List<T>"))?
            } else {
                &parameters[slot].2
            };
            let domain = declared
                .instantiate_types(&known_parameters, &known_arguments)
                .map_err(|failure| error(span, failure.to_string()))?;
            let inputs = self.term_declaration_inputs(&domain, span)?;
            let description = if parameters.iter().any(|(id, _, _)| inputs.contains(id)) {
                D::Unknown
            } else {
                D::Known(domain)
            };
            if let Some(value) = value {
                self.describe_open_expression(
                    value,
                    locals,
                    returns,
                    Some(&description),
                    depth + 1,
                )?;
            } else {
                self.require_description(
                    &D::Known(packet_domain.unwrap()),
                    Some(&description),
                    span,
                )?;
            }
        }
        if routing.missing_required() {
            return Err(error(expr.span, "missing required type argument"));
        }
        let result = D::Known(p(P::Type));
        self.require_description(&result, expected, expr.span)?;
        // The telescope established Type, but the application cannot yet
        // supply a checked value. Keep the initializer for the instance.
        if let Some((owner, pending)) = &mut self.pending_initializer
            && *owner == expr.hir_id.owner
        {
            *pending = true;
        }
        Ok(result)
    }
}
