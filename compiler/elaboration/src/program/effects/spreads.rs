//! Operation inputs share ordinary tuple preparation; operation identity and
//! ABI still come exclusively from effect formation.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn effect_request_spreads(
        &mut self,
        site: &Expr<'_>,
        call: &Expr<'_>,
        head: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if arguments
            .iter()
            .any(|argument| !matches!(argument, Arg::Positional(_) | Arg::Expand(_)))
        {
            return Err(error(
                site.span,
                "effect request requires positional arguments",
            ));
        }
        if let Some(flat) = arguments::literal_spread_arguments(arguments) {
            let call = Expr {
                kind: H::Application(head, &flat),
                ..call.clone()
            };
            return self.effect_request(site, &call, expected);
        }

        let owner = self.inferred_effect_owner(head);
        let (signature, binders) = if let Some(owner) = owner {
            self.ensure_signature(owner, site.span)?;
            self.ensure_contract(owner, site.span)?;
            let plan_owner = self.effect_parents.get(&owner).copied().unwrap_or(owner);
            let all = self.type_parameters[&owner].clone();
            let (binders, template, _) = self.function_type_plan(plan_owner, &[], site.span)?;
            let signature = self.instantiate_input_types(
                &self.signatures[&owner].clone(),
                &all,
                &template,
                site.span,
            )?;
            (signature, binders)
        } else {
            let operations = self.effect_row(head, 0)?;
            if operations.len() != 1 {
                return Err(error(
                    site.span,
                    "effect request requires one independent operation",
                ));
            }
            let operation = &operations[0];
            let metadata = self.effect_metadata(operation, site.span)?;
            (
                self.formed_effect_signature(&metadata, operation, site.span)?,
                Vec::new(),
            )
        };
        let Term::Function { parameters, .. } = &signature else {
            return Err(error(site.span, "invalid effect signature"));
        };
        let width = parameters.len();
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&binders)
                .iter()
                .map(|binder| binder.sort)
                .collect(),
            type_parameters: binders.len(),
            signature: signature
                .abstract_parameters(&self.input_binders(&binders))
                .map_err(|cause| error(site.span, cause.to_string()))?,
        };
        let structural = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == site.hir_id)
            .map(|(_, context)| context.clone());
        let mut checked = BTreeMap::new();
        let mut prepared_packets = BTreeMap::new();
        let candidate = self.infer_packet_inputs(
            &scheme,
            width,
            arguments,
            expected,
            structural.as_ref(),
            site.span,
            &mut checked,
            &mut prepared_packets,
        )?;
        let fields = (0..width)
            .map(|slot| {
                candidate
                    .parameter_input_context(slot)
                    .map_err(|cause| call_inference_error(site.span, cause))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut widths = BTreeMap::new();
        for (index, packet) in &prepared_packets {
            if let Term::Tuple(fields) = self.structural_type(&packet.ty, site.span)? {
                widths.insert(*index, fields.len());
            }
        }
        let contexts = self
            .spread_packet_layout_with_widths(arguments, &fields, site.span, &widths)?
            .into_iter()
            .map(|fields| {
                fields.map(|fields| match self.protocols.tuple {
                    Some(head) => crate::calls::InputContext::NormalTuple { head, fields },
                    None => crate::calls::InputContext::Tuple(fields),
                })
            })
            .collect::<Vec<_>>();
        self.prepare_spreads_checked(
            site,
            arguments,
            &vec![None; arguments.len()],
            &vec![false; arguments.len()],
            Some(&contexts),
            checked,
            prepared_packets,
            0,
            |this, inputs, diverges| {
                let call = Expr {
                    kind: H::Application(head, inputs),
                    ..call.clone()
                };
                this.effect_request(site, &call, expected.filter(|_| !diverges))
            },
        )
    }
}
