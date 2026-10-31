//! Demand a complete name-specific inherent candidate set before trait setup.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn early_inherent_name_available(&self, name: &str) -> bool {
        self.package.owners().any(|(_, info)| {
            let members = match &info.node.expect_item().kind {
                ItemKind::Impl(def) if def.trait_ref.is_none() => &def.items,
                ItemKind::Struct(def) => &def.nested_items,
                ItemKind::Enum(def) => &def.nested_items,
                _ => return false,
            };
            members.iter().any(|member| {
                self.package.item(*member).is_some_and(|item| {
                    item.ident.name.as_str() == name
                        && matches!(item.kind, ItemKind::Fn(..) | ItemKind::FnDeclaration(_))
                })
            })
        })
    }
    pub(super) fn early_inherent_method(
        &mut self,
        subject: Term,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        if self.active_early_inherent.len() >= 64 || !self.active_early_inherent.insert(name.into())
        {
            return Err(error(
                span,
                "cyclic or excessively deep inherent method preparation",
            ));
        }
        let saved_owner = self.signature_owner;
        let saved_context = self.context.clone();
        let result = self.prepare_early_inherent_method(subject, name, span);
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.active_early_inherent.remove(name);
        result
    }

    fn prepare_early_inherent_method(
        &mut self,
        subject: Term,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let context = self.context.clone();
        let query_owner = self.signature_owner;
        let mut index = crate::implementation::InherentIndex::for_builtin_packages(
            self.target,
            self.protocols
                .builtin_implementation_packages
                .iter()
                .copied(),
        );
        let mut members = BTreeMap::new();
        // Enumerate all source declarations for this name. An incomplete world
        // must never silently choose the first currently available candidate.
        for (owner, info) in self.package.owners() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "inherent method preparation budget exhausted"))?;
            let item = info.node.expect_item();
            let candidates = match &item.kind {
                ItemKind::Impl(def) if def.trait_ref.is_none() => &def.items,
                ItemKind::Struct(def) => &def.nested_items,
                ItemKind::Enum(def) => &def.nested_items,
                _ => continue,
            };
            self.required_steps = self
                .required_steps
                .checked_sub(candidates.len() as u64)
                .ok_or_else(|| error(span, "inherent method preparation budget exhausted"))?;
            let named = candidates
                .iter()
                .copied()
                .filter(|member| {
                    self.package.item(*member).is_some_and(|item| {
                        item.ident.name.as_str() == name
                            && matches!(item.kind, ItemKind::Fn(..) | ItemKind::FnDeclaration(_))
                    })
                })
                .collect::<Vec<_>>();
            if named.is_empty() {
                continue;
            }
            if named.len() != 1 {
                return Err(error(item.span, "duplicate implementation member"));
            }
            self.signature_owner = Some(owner);
            self.context = self.binder_context(owner);
            self.ensure_contract(owner, item.span)?;
            // Trait applicability needs the complete implementation world. Do
            // not treat missing trait evidence as proof that a head is absent.
            if self
                .capabilities
                .get(&owner)
                .is_some_and(|items| !items.is_empty())
            {
                return Err(error(
                    span,
                    "required comptime evaluation is waiting for the trait implementation world",
                ));
            }
            let receiver = match &item.kind {
                ItemKind::Impl(def) => self.type_expr(def.self_ty)?,
                _ => self.nominal_self_type(owner),
            };
            let identity = declaration(self.package, &self.package_identities, owner);
            index
                .check_ownership(&receiver, identity.package)
                .map_err(|e| {
                    error(
                        item.span,
                        format!("inherent implementation admission failed: {e:?}"),
                    )
                })?;
            let mut parameters = self.type_parameters[&owner].clone();
            parameters.extend(self.nominal_capture_ids(owner));
            index
                .insert(
                    crate::implementation::InherentHead {
                        parameter_types: parameters
                            .iter()
                            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                            .collect(),
                        declaration: identity,
                        subject: receiver,
                        type_parameters: parameters,
                        requirements: self.requirements[&owner].clone(),
                        members: BTreeSet::from([name.into()]),
                    },
                    Budget::default(),
                )
                .map_err(|e| {
                    error(
                        item.span,
                        format!("inherent implementation admission failed: {e:?}"),
                    )
                })?;
            members.insert(identity, named[0]);
        }
        for (id, domain) in self.implementation_index.parameter_domains() {
            index
                .register_parameter_domain(*id, domain.clone())
                .map_err(|message| error(span, message))?;
        }
        index.sync_nominal_layouts(
            self.structs.values(),
            self.enums.values(),
            self.newtypes.values(),
        );
        index.set_normal_form_metadata(self.normal_form_metadata());
        // Preparing a candidate may discover List after the caller's context
        // was saved. Refresh its checked head metadata without importing the
        // candidate's binder assumptions into the caller's proof context.
        self.context = context;
        self.refresh_list_inference_metadata();
        let context = self.context.clone();
        index
            .check_coherence(&self.nominal_context(), Budget::default())
            .map_err(|message| error(span, message))?;
        let resolution = index
            .resolve_types(name, &subject, &context, Budget::default())
            .map_err(|e| error(span, format!("inherent method query failed: {e:?}")))?;
        let selected = match resolution.selection {
            crate::implementation::Selection::NoApplicableCandidate => return Ok(None),
            crate::implementation::Selection::Selected(id) => id,
            other => {
                return Err(error(
                    span,
                    format!("inherent method applicability is unresolved: {other}"),
                ));
            }
        };
        let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
            resolution.assessments.get(&selected)
        else {
            return Err(error(
                span,
                "selected inherent implementation has no type evidence",
            ));
        };
        index
            .verify_evidence(name, evidence, &subject, &context, Budget::default())
            .map_err(|e| {
                error(
                    span,
                    format!("inherent method evidence replay failed: {e:?}"),
                )
            })?;
        let method = members[&selected];
        self.signature_owner = query_owner;
        self.context = context.clone();
        self.ensure_signature(method, span)?;
        self.ensure_contract(method, span)?;
        if self
            .capabilities
            .get(&method)
            .is_some_and(|items| !items.is_empty())
        {
            return Err(error(
                span,
                "required comptime evaluation is waiting for the trait implementation world",
            ));
        }
        self.context = context;
        self.inherent_method_value(method, evidence.type_arguments.clone(), span)
    }
}
