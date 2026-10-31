//! Select property collectors without replaying explicit clause arguments.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn contextual_property_plan<O: nir::code_interfaces::SignatureOutput + Clone>(
        &mut self,
        owner: OwnerId,
        template: &Generator<O>,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        expected: Option<&Term>,
        output_type: impl Fn(&O) -> Term,
    ) -> Result<(hir::common::Ident, GeneratorTypePlan), Error> {
        let type_plan = if let H::NFApplication(_, types) = callee.kind {
            (
                vec![],
                self.type_clause_arguments(owner, types, None, site.span)?,
                vec![],
                vec![],
            )
        } else {
            self.generator_clause_plan(owner, site.span)?
        };
        let mut definition = self.instantiate_code_generator(&template, &type_plan.1, site.span)?;
        if !type_plan.0.is_empty()
            && let Some(expected) = expected
        {
            let scheme = crate::calls::Scheme {
                parameter_sorts: self
                    .input_binders(&type_plan.0)
                    .iter()
                    .map(|p| p.sort)
                    .collect(),
                type_parameters: type_plan.0.len(),
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(output_type(&definition.output)),
                }
                .abstract_parameters(&self.input_binders(&type_plan.0))
                .map_err(|e| error(site.span, e.to_string()))?,
            };
            let input_domains = type_plan
                .0
                .iter()
                .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                .collect::<Vec<_>>();
            let candidate = scheme
                .infer_declared_sparse_types_mode(
                    &type_plan.0,
                    &input_domains,
                    &[],
                    Some(expected),
                    None,
                    &self.context,
                    crate::ptc::Budget::default(),
                    false,
                    &[],
                )
                .map_err(|e| call_inference_error(site.span, e))?;
            definition = self.generator_inference_view(
                &template,
                &type_plan.0,
                &type_plan.1,
                &candidate,
                site.span,
            )?;
        }
        let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
            &self.package.item(owner).unwrap().kind
        else {
            unreachable!()
        };
        let mut collectors = Vec::new();
        for (index, param) in signature.params.iter().enumerate() {
            if !matches!(param.ty.kind, TyParamKind::Varadic(..)) {
                continue;
            }
            let Input::Data(ty) = &definition.inputs[index].1 else {
                continue;
            };
            if !matches!(
                self.structural_type(ty, site.span)?,
                Term::Record(_)
                    | Term::Rigid { .. }
                    | Term::NormalForm { .. }
                    | Term::AssociatedType { .. }
                    | Term::Bound { .. }
                    | Term::Hole { .. }
            ) {
                continue;
            }
            collectors.push(param.name);
        }
        let [collector] = collectors.as_slice() else {
            return Err(error(
                site.span,
                "generator property collection requires one eligible variadic record entrance",
            ));
        };
        Ok((*collector, type_plan))
    }
}
