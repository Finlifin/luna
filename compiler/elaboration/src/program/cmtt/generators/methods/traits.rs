//! Trait calls prepare the public interface before entering the selected body.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn with_trait_code_method_callee<R>(
        &mut self,
        callee: &Expr<'_>,
        head: &Expr<'_>,
        arguments: Option<&[Arg<'_>]>,
        base: &Expr<'_>,
        interface_expr: &Expr<'_>,
        name: hir::common::Ident,
        invoke: impl FnOnce(&mut Self, &Expr<'_>, Option<&Expr<'_>>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        if !self.interfaces_ready {
            return Err(error(
                head.span,
                "code method selection is waiting for the implementation world",
            ));
        }
        let static_subject = self.is_static_method_subject(base)
            || self.local_field_domain(base)? == Some(p(P::Type));
        let receiver = (!static_subject).then_some(base);
        let subject = if receiver.is_some() {
            self.generator_intrinsic_input_hint(base, 0)?
                .ok_or_else(|| error(base.span, "code method receiver type is not established"))?
        } else {
            self.type_expr(base)?
        };
        let subject = self.type_identity_view(&subject, head.span)?;
        let requested = self.interface_expr_for(interface_expr, &subject)?;
        let requested_resolution = self
            .resolve_trait_capability(&requested, &subject, Budget::default())
            .map_err(|e| {
                error(
                    head.span,
                    format!("code method interface query failed: {e}"),
                )
            })?;
        if !matches!(
            requested_resolution.selection,
            crate::implementation::Selection::Selected(_)
        ) && !(self.checking_code_generator
            && self.symbolic_interface_evidence(&subject, &requested))
        {
            return Err(error(
                head.span,
                "requested code method interface is not established",
            ));
        }
        let interface = self
            .inherited_member_source(&subject, &requested, name.name.as_str(), head.span)?
            .unwrap_or(requested);
        let resolution = self
            .resolve_trait_capability(&interface, &subject, Budget::default())
            .map_err(|e| {
                error(
                    head.span,
                    format!("code method implementation query failed: {e}"),
                )
            })?;
        let contract = self
            .checked_traits
            .iter()
            .find(|item| item.declaration == interface.declaration)
            .and_then(|item| {
                item.methods
                    .iter()
                    .find(|method| method.name == name.name.as_str())
            })
            .ok_or_else(|| error(head.span, "unknown trait code method"))?;
        if !matches!(
            contract.signature,
            nir::code_interfaces::MethodSignature::Generator(_)
                | nir::code_interfaces::MethodSignature::PatternGenerator(_)
        ) {
            return Err(error(
                head.span,
                "code position requires a code-producing trait method",
            ));
        }
        let public_id = contract.declaration;
        let (body_id, target_captures) = match resolution.selection {
            crate::implementation::Selection::Selected(selected) => {
                let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
                    resolution.assessments.get(&selected)
                else {
                    return Err(error(
                        head.span,
                        "selected code implementation has no type evidence",
                    ));
                };
                let implementation = self
                    .checked_implementations
                    .iter()
                    .find(|item| item.declaration == selected)
                    .ok_or_else(|| {
                        error(head.span, "selected code implementation is unavailable")
                    })?;
                if implementation.type_parameters.len() != evidence.type_arguments.len() {
                    return Err(error(
                        head.span,
                        "code method captured argument count mismatch",
                    ));
                }
                let body = implementation
                    .methods
                    .iter()
                    .find(|(member, _)| member == name.name.as_str())
                    .map(|(_, id)| *id)
                    .ok_or_else(|| error(head.span, "selected code method is unavailable"))?;
                (
                    body,
                    implementation
                        .type_parameters
                        .iter()
                        .copied()
                        .zip(evidence.type_arguments.iter().cloned())
                        .collect::<Vec<_>>(),
                )
            }
            _ if self.checking_code_generator
                && self.symbolic_interface_evidence(&subject, &interface) =>
            {
                (public_id, Vec::new())
            }
            _ => {
                return Err(error(
                    head.span,
                    "code method implementation is not uniquely established",
                ));
            }
        };
        let find_owner = |id| {
            self.package.owners().find_map(|(owner, _)| {
                matches_declaration(self.package, &self.package_identities, owner, id)
                    .then_some(owner)
            })
        };
        let public = find_owner(public_id)
            .ok_or_else(|| error(head.span, "trait code declaration is unavailable"))?;
        let target = find_owner(body_id)
            .ok_or_else(|| error(head.span, "trait code body is unavailable"))?;
        let (parameters, values) = self.interface_substitution(&interface, &subject);
        let public_captures = parameters.into_iter().zip(values).collect::<Vec<_>>();
        self.check_method_declaration_access(Some(public_id), head.hir_id, head.span)?;
        let (ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
            &self.package.item(public).unwrap().kind
        else {
            return Err(error(head.span, "trait code member is not a function"));
        };
        if receiver.is_some()
            && !signature
                .params
                .first()
                .is_some_and(|param| matches!(param.ty.kind, TyParamKind::Itself { .. }))
        {
            return Err(error(head.span, "instance code call requires a receiver"));
        }
        let selected_head = Expr {
            kind: H::ResolvedDef {
                name: name.name,
                has_clause_param: !signature.clause_params.is_empty(),
                def_id: self.package.owner_definition(public).ok_or_else(|| {
                    error(head.span, "trait code member has no definition identity")
                })?,
            },
            ..head.clone()
        };
        let selected_callee = match arguments {
            Some(arguments) => Expr {
                kind: H::NFApplication(&selected_head, arguments),
                ..callee.clone()
            },
            None => selected_head.clone(),
        };
        let mut captures = BTreeMap::new();
        if target != public {
            captures.insert(target, target_captures);
        }
        captures.insert(public, public_captures);
        let saved = captures
            .into_iter()
            .map(|(owner, captures)| (owner, self.code_method_captures.insert(owner, captures)))
            .collect::<Vec<_>>();
        let saved_target = self.code_method_targets.remove(&public);
        if target != public {
            self.code_method_targets.insert(public, target);
        }
        let by_reference = signature
            .params
            .first()
            .is_some_and(|param| matches!(param.ty.kind, TyParamKind::Itself { is_ref: true }));
        let addressed = receiver.map(|base| Expr {
            kind: H::Ref(base),
            ..base.clone()
        });
        let result = invoke(
            self,
            &selected_callee,
            if by_reference {
                addressed.as_ref()
            } else {
                receiver
            },
        );
        self.code_method_targets.remove(&public);
        if let Some(target) = saved_target {
            self.code_method_targets.insert(public, target);
        }
        for (owner, captures) in saved {
            match captures {
                Some(captures) => {
                    self.code_method_captures.insert(owner, captures);
                }
                None => {
                    self.code_method_captures.remove(&owner);
                }
            }
        }
        result
    }
}
