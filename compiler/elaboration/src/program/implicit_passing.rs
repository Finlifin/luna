//! ImplicitPassing protocol resolution and compile-time generator invocation.
use super::*;
use crate::implementation::Selection;
use crate::program::cmtt::generators::Argument;
use crate::ptc::Budget;
use comptime_interpretor::Value;
use std::sync::Arc;

impl Checker<'_, '_> {
    fn declaration_context_fields(&self, decl: Option<DeclarationId>) -> (String, String, String) {
        let Some(id) = decl else {
            return (String::new(), String::new(), String::new());
        };
        let package_name = self
            .package_metadata
            .get(&id.package)
            .map(|meta| meta.name.clone())
            .unwrap_or_default();
        if let Some(path) = self.declaration_paths.get(&id) {
            let module_path = if path.modules.is_empty() {
                package_name.clone()
            } else if package_name.is_empty() {
                path.modules.join(".")
            } else {
                format!("{package_name}.{}", path.modules.join("."))
            };
            let callable_name = if module_path.is_empty() {
                path.name.clone()
            } else {
                format!("{module_path}.{}", path.name)
            };
            (package_name, module_path, callable_name)
        } else {
            let owner_name = self
                .source_declarations
                .owner(id)
                .and_then(|owner| self.package.item(owner))
                .map(|item| item.ident.name.as_str().to_string())
                .unwrap_or_default();
            (package_name.clone(), package_name, owner_name)
        }
    }

    fn build_implicit_context_value(
        &self,
        context_decl: DeclarationId,
        target: Option<DeclarationId>,
    ) -> Value {
        let (package_name, module_path, callable_name) = self.declaration_context_fields(target);
        Value::Record {
            arguments: Vec::new(),
            declaration: context_decl,
            fields: vec![
                Value::String(Arc::from(package_name)),
                Value::String(Arc::from(module_path)),
                Value::String(Arc::from(callable_name)),
            ],
        }
    }

    fn build_call_description_value(&self, desc_decl: DeclarationId) -> Value {
        Value::Record {
            arguments: Vec::new(),
            declaration: desc_decl,
            fields: Vec::new(),
        }
    }

    fn build_implicit_call_site(&self, call_span: Span) -> comptime_interpretor::ImplicitCallSite {
        let loc = self
            .protocols
            .source_locator
            .as_ref()
            .map(|locator| locator.locate(call_span))
            .unwrap_or_else(|| CallSiteLocation::fallback(call_span));
        comptime_interpretor::ImplicitCallSite {
            file: Arc::from(loc.file),
            line: loc.line,
            byte_start: loc.byte_start as u64,
            byte_end: loc.byte_end as u64,
            expansion_id: loc.expansion_id,
        }
    }

    fn unwrap_identity_splice_if_comptime(&self, expr: n::Expr, is_comptime: bool) -> n::Expr {
        if !is_comptime {
            return expr;
        }
        let Some(splice_id) = self.protocols.splice else {
            return expr;
        };
        let K::Call {
            callee, arguments, ..
        } = &expr.kind
        else {
            return expr;
        };
        if arguments.len() == 1
            && matches!(
                &callee.kind,
                K::TraitMethod {
                    interface,
                    member,
                    ..
                } if *interface == splice_id
                    && self
                        .checked_traits
                        .by_id(*interface)
                        .and_then(|t| t.methods.iter().find(|m| m.declaration == *member))
                        .is_some_and(|m| m.name == "splice")
            )
            && arguments[0].value.ty == expr.ty
        {
            return arguments[0].value.clone();
        }
        expr
    }

    /// Resolve and execute `ImplicitPassing<.param_name>::pass(caller, callee, calling)`
    /// for an omitted non-contextual `implicit` parameter. Never captures local variables.
    pub(super) fn resolve_implicit_passing_expr(
        &mut self,
        callee_decl: Option<DeclarationId>,
        param_name: &str,
        param_ty: &Term,
        call_span: Span,
        is_comptime_slot: bool,
    ) -> Result<n::Expr, Error> {
        let Some(protocol_decl) = self.protocols.implicit_passing else {
            return Err(error(
                call_span,
                input_routing::FailureKind::MissingImplicit.to_string(),
            ));
        };
        let context_decl = self.protocols.call_context.ok_or_else(|| {
            error(
                call_span,
                "ImplicitPassing requires a bound `Context` type (`call-context-type`)",
            )
        })?;
        let desc_decl = self.protocols.call_description.ok_or_else(|| {
            error(
                call_span,
                "ImplicitPassing requires a bound `CallDescription` type (`call-description-type`)",
            )
        })?;
        if !self.interfaces_ready {
            return Err(error(
                call_span,
                "ImplicitPassing resolution is waiting for the trait implementation world",
            ));
        }

        let subject = self.method_subject_view(param_ty, call_span)?;
        let interface = n::InterfaceInstance {
            declaration: protocol_decl,
            arguments: vec![Term::Symbol(param_name.into())],
        };

        let mut unresolved = false;
        match self.symbolic_interface_evidence(&subject, &interface) {
            Ok(true) => {
                return Err(context_error(
                    call_span,
                    format!(
                        "ImplicitPassing<.{param_name}> for `{subject}` cannot invoke an abstract trait clause without a concrete implementation"
                    ),
                ));
            }
            Ok(false) => {}
            Err(nir::NominalAdmissionError::Unresolved(_)) => {
                unresolved = true;
            }
            Err(reason) => return Err(error(call_span, reason.to_string())),
        }

        let resolution = self
            .resolve_trait_capability(&interface, &subject, Budget::default())
            .map_err(|reason| capability_query_error(call_span, reason))?;

        let selected_impl = match resolution.selection {
            Selection::Selected(id) => id,
            Selection::NoApplicableCandidate => {
                if unresolved {
                    return Err(context_error(
                        call_span,
                        format!(
                            "ImplicitPassing<.{param_name}> lookup blocked on unresolved type `{subject}`"
                        ),
                    ));
                }
                return Err(error(
                    call_span,
                    format!(
                        "missing `ImplicitPassing<.{param_name}>` implementation for `{subject}`"
                    ),
                ));
            }
            Selection::Deferred { .. } | Selection::UnresolvedInterface | Selection::StaleIndex => {
                return Err(context_error(
                    call_span,
                    format!(
                        "ImplicitPassing<.{param_name}> lookup blocked on unresolved obligations for `{subject}`"
                    ),
                ));
            }
            Selection::Conflict(candidates) => {
                return Err(error(
                    call_span,
                    format!(
                        "ambiguous `ImplicitPassing<.{param_name}>` implementations for `{subject}` ({} candidates)",
                        candidates.len()
                    ),
                ));
            }
            Selection::InvalidCandidate(_) => {
                return Err(error(
                    call_span,
                    format!(
                        "invalid `ImplicitPassing<.{param_name}>` implementation for `{subject}`"
                    ),
                ));
            }
        };

        let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
            resolution.assessments.get(&selected_impl)
        else {
            return Err(error(
                call_span,
                format!(
                    "selected `ImplicitPassing<.{param_name}>` implementation has no type evidence"
                ),
            ));
        };
        let implementation = self
            .checked_implementations
            .iter()
            .find(|item| item.declaration == selected_impl)
            .ok_or_else(|| {
                error(
                    call_span,
                    format!(
                        "selected `ImplicitPassing<.{param_name}>` implementation is unavailable"
                    ),
                )
            })?
            .clone();
        if implementation.type_parameters.len() != evidence.type_arguments.len() {
            return Err(error(
                call_span,
                "ImplicitPassing implementation type argument count mismatch",
            ));
        }
        let body_id = implementation
            .methods
            .iter()
            .find(|(member, _)| member == "pass")
            .map(|(_, id)| *id)
            .ok_or_else(|| {
                error(
                    call_span,
                    format!(
                        "`ImplicitPassing<.{param_name}>` implementation is missing `pass` method"
                    ),
                )
            })?;
        let target_owner = self.source_declarations.owner(body_id).ok_or_else(|| {
            error(
                call_span,
                "ImplicitPassing `pass` method declaration is unavailable",
            )
        })?;

        self.ensure_code_generator(target_owner, call_span)?;
        let template = self.copy_code_generator(target_owner, call_span)?;
        let definition =
            self.instantiate_code_generator(&template, &evidence.type_arguments, call_span)?;
        let saved = definition
            .captures
            .iter()
            .map(|(id, value)| {
                (
                    *id,
                    self.alias_substitution.insert(*id, Arc::new(value.clone())),
                )
            })
            .collect::<Vec<_>>();
        let checked =
            self.check_type_application(target_owner, &evidence.type_arguments, call_span);
        for (id, value) in saved {
            match value {
                Some(value) => {
                    self.alias_substitution.insert(id, value);
                }
                None => {
                    self.alias_substitution.remove(&id);
                }
            }
        }
        checked?;

        let caller_decl = self
            .signature_owner
            .map(|owner| declaration(self.package, &self.package_identities, owner))
            .or(self.active_body_declaration);
        let caller_val = self.build_implicit_context_value(context_decl, caller_decl);
        let callee_val = self.build_implicit_context_value(context_decl, callee_decl);
        let calling_val = self.build_call_description_value(desc_decl);

        if definition.inputs.len() != 3 {
            return Err(error(
                call_span,
                "`ImplicitPassing.pass` must accept `(caller: Context, callee: Context, calling: CallDescription)`",
            ));
        }

        let arguments = vec![
            Some(Argument::LiveData(caller_val)),
            Some(Argument::LiveData(callee_val)),
            Some(Argument::LiveData(calling_val)),
        ];

        let saved_call_site = self
            .active_implicit_call_site
            .replace(self.build_implicit_call_site(call_span));
        let code = self.generator_frame(target_owner, |this| {
            this.run_code_generator(&definition, arguments, call_span)
        });
        self.active_implicit_call_site = saved_call_site;
        let code = code?;

        if !code.context.is_empty() {
            return Err(error(
                call_span,
                "`ImplicitPassing.pass` returned code with unbound context dependencies",
            ));
        }
        self.require(&code.result, &subject, call_span)?;
        let Some(body) = &code.body else {
            return Err(error(
                call_span,
                "`ImplicitPassing.pass` did not produce a code body",
            ));
        };
        constants::charge_copy(body, &mut self.required_steps, call_span)?;
        let mut produced = body.as_ref().clone();
        produced.source = source(call_span);
        let produced = self.unwrap_identity_splice_if_comptime(produced, is_comptime_slot);
        Ok(produced)
    }
}
