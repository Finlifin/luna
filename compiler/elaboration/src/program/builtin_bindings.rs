//! Named bindings for compiler operations retain their checked call lowering.
use super::*;

mod signatures;
mod values;

fn operation(name: &str) -> bool {
    matches!(
        name,
        "list_length"
            | "list_concat"
            | "comptime_integer_convert"
            | "comptime_integer_div_trunc"
            | "comptime_integer_rem_trunc"
            | "integer_representation"
            | "try_integer_convert"
            | "str_byte_length"
            | "str_byte"
            | "str_slice"
            | "str_concat"
            | "str_from_char"
            | "symbol_byte_length"
            | "symbol_byte"
            | "object_length"
            | "object_key"
            | "object_entry_type"
            | "object_value"
            | "object_get"
            | "object_concat"
            | "object_append"
            | "object_replace"
            | "object_remove"
            | "object_slice"
            | "newtype_representation"
            | "struct_field_types"
            | "struct_field_names"
            | "record_field_types"
            | "record_field_names"
            | "struct_field_spans"
            | "record_field_spans"
            | "declaration_span"
            | "enum_variant_spans"
            | "enum_variant_field_spans"
            | "reflect_callable"
            | "type_id"
            | "enum_group_paths"
            | "enum_variant_paths"
            | "enum_variant_shapes"
            | "enum_variant_field_names"
            | "enum_variant_field_types"
            | "enum_symbol_values"
            | "enum_bit_width"
            | "enum_bit_base_type"
            | "enum_bit_masks"
            | "enum_bit_values"
            | "enum_bit_offsets"
            | "build_struct"
            | "build_newtype"
            | "build_enum"
            | "build_enum_paths"
            | "build_symbol_enum_paths"
            | "build_bits_enum_paths"
            | "implicit_caller"
            | "implicit_module"
            | "implicit_package"
            | "implicit_file"
            | "implicit_line"
            | "implicit_src"
    )
}

impl Checker<'_, '_> {
    fn binding_annotation(&mut self, owner: OwnerId, annotation: &Expr<'_>) -> Result<Term, Error> {
        self.in_formal_declaration(owner, |this| {
            this.context = this.definition_context(owner);
            this.type_expr(annotation)
        })
    }

    pub(super) fn builtin_normal_form_constant(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let Some(ItemKind::Const(annotation, initializer)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        let H::Builtin(symbol) = initializer.kind else {
            return Ok(None);
        };
        use nir::normal_forms::BuiltinCallable;
        let Some(builtin) = BuiltinCallable::from_symbol(symbol.as_str()) else {
            return Ok(None);
        };
        let identity = match builtin {
            BuiltinCallable::Tuple => self.protocols.tuple,
            BuiltinCallable::List => self.protocols.meta_list,
        }
        .ok_or_else(|| error(span, "builtin callable requires a registered binding"))?;
        let expected = builtin
            .signature(self.protocols.meta_list)
            .ok_or_else(|| error(span, "builtin callable signature dependency is unavailable"))?;
        let canonical = self
            .source_declarations
            .owner(identity)
            .ok_or_else(|| error(span, "builtin constructor binding is unavailable"))?;
        if canonical != owner {
            let value = self.normal_form_value(initializer, canonical, None)?;
            if !matches!(annotation.kind, H::TyPlaceholder) {
                let expected = self.binding_annotation(owner, annotation)?;
                self.require_expression(&value.ty, &expected, span)?;
            }
            return Ok(Some(value));
        }
        if !self.normal_parameters.contains_key(&owner) {
            if self.active_signatures.len() >= 64 || !self.active_signatures.insert(owner) {
                return Err(error(span, "cyclic builtin constructor signature"));
            }
            let prepared = (|| {
                let ty = self.binding_annotation(owner, annotation)?;
                let inputs = function_types::normal_form_inputs(annotation)
                    .ok_or_else(|| error(span, "builtin constructor binding requires an explicit normal-form callable signature"))?;
                if ty != expected {
                    return Err(error(
                        span,
                        format!(
                            "builtin callable signature mismatch: expected {expected}, found {ty}"
                        ),
                    ));
                }
                let ids: Vec<_> = self.fresh_callable_inputs(inputs.len(), span)?.collect();
                let opened = nominal_fields::metered(&mut self.required_steps, |fuel| {
                    nir::normal_forms::open_inputs(&ty, &ids, fuel)
                })
                .map_err(|reason| error(span, reason.to_string()))?;
                let descriptors = opened
                    .into_iter()
                    .zip(inputs)
                    .map(|((id, _, domain), input)| (id, local(input.hir_id), domain))
                    .collect::<Vec<_>>();
                Ok((ty, descriptors))
            })();
            self.active_signatures.remove(&owner);
            let (ty, descriptors) = prepared?;
            self.type_parameters.insert(
                owner,
                descriptors
                    .iter()
                    .filter(|(_, _, domain)| *domain == p(P::Type))
                    .map(|(id, _, _)| *id)
                    .collect(),
            );
            self.signatures
                .insert(owner, ty.callable_signature().clone());
            for &(id, _, _) in &descriptors {
                self.parameter_domain_owners.entry(id).or_insert(owner);
            }
            self.normal_parameters.insert(owner, descriptors);
            self.publish_normal_head(owner);
        }
        self.ensure_contract(owner, span)?;
        let ty = self.normal_form_callable_type(owner, span)?;
        Ok(Some(n::Expr {
            source: source(span),
            ty,
            kind: K::Function(identity),
        }))
    }

    pub(super) fn compiler_operation_binding(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Option<hir::Symbol>, Error> {
        Ok(self
            .resolve_compiler_operation(owner, span)?
            .map(|(name, _, _)| name))
    }

    pub(super) fn direct_compiler_operation(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Option<hir::Symbol>, Error> {
        let Some((name, _, signature)) = self.resolve_compiler_operation(owner, span)? else {
            return Ok(None);
        };
        // Established contracts are ordinary callable values. Keep the temporary
        // direct lowering only for operations awaiting callable contracts.
        Ok(signature.is_none().then_some(name))
    }

    fn resolve_compiler_operation(
        &mut self,
        mut owner: OwnerId,
        span: Span,
    ) -> Result<Option<(hir::Symbol, OwnerId, Option<Term>)>, Error> {
        let mut visited = BTreeSet::new();
        let mut annotations = Vec::new();
        loop {
            let Some(item) = self.package.item(owner) else {
                return Ok(None);
            };
            let ItemKind::Const(annotation, initializer) = item.kind else {
                return Ok(None);
            };
            if !matches!(annotation.kind, H::TyPlaceholder) {
                annotations.push((owner, annotation));
            }
            if !visited.insert(owner) || visited.len() > 64 {
                return Err(error(
                    span,
                    "cyclic or excessively deep constant dependency",
                ));
            }
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "compiler operation binding budget exhausted"))?;
            match &initializer.kind {
                H::Builtin(name) if operation(name.as_str()) => {
                    let name = *name;
                    // Legacy phase-neutral bindings retain direct lowering.
                    // Only the original declaration may establish a phase;
                    // an alias cannot turn an untyped token into a function.
                    if (signatures::text_query(name.as_str()).is_some()
                        || matches!(
                            name.as_str(),
                            "integer_representation"
                                | "try_integer_convert"
                                | "comptime_integer_convert"
                        ))
                        && !annotations.iter().any(|(annotated, _)| *annotated == owner)
                    {
                        if let Some((_, annotation)) = annotations.first() {
                            return Err(error(
                                annotation.span,
                                "typed compiler operation requires an explicit phase on its original binding",
                            ));
                        }
                        return Ok(Some((name, owner, None)));
                    }
                    let mut signature = signatures::callable(
                        name.as_str(),
                        self.protocols.meta_list,
                        self.protocols.call_context,
                        self.protocols.call_description,
                        self.protocols.source_span,
                    )
                    .map_err(|message| error(span, message))?;
                    if !annotations.is_empty() {
                        let mut expected = signature.clone();
                        for (annotated_owner, annotation) in annotations.into_iter().rev() {
                            if !self.active_signatures.insert(annotated_owner) {
                                return Err(error(
                                    annotation.span,
                                    "cyclic compiler operation signature",
                                ));
                            }
                            let actual = self.binding_annotation(annotated_owner, annotation);
                            self.active_signatures.remove(&annotated_owner);
                            let actual = actual?;
                            if annotated_owner == owner
                                && expected.is_none()
                                && self.validate_implicit_query_fallback(
                                    name.as_str(),
                                    &actual,
                                    annotation.span,
                                )
                            {
                                expected = Some(actual.clone());
                                signature = Some(actual.clone());
                            }
                            if annotated_owner == owner
                                && matches!(
                                    name.as_str(),
                                    "integer_representation"
                                        | "try_integer_convert"
                                        | "comptime_integer_convert"
                                )
                            {
                                let valid = match name.as_str() {
                                    "integer_representation" => {
                                        signatures::integer_representation(&actual)
                                    }
                                    "try_integer_convert" => {
                                        signatures::try_integer_convert(&actual)
                                    }
                                    _ => signatures::integer_conversion(&actual),
                                };
                                if !valid {
                                    return Err(error(
                                        annotation.span,
                                        format!(
                                            "compiler operation signature mismatch: invalid concrete domains or phase for {}",
                                            name.as_str()
                                        ),
                                    ));
                                }
                                expected = Some(actual.clone());
                                signature = Some(actual.clone());
                            }
                            // Phase-neutral primitive operations may establish
                            // either phase at their original typed binding.
                            // Aliases retain that binding's phase and identity.
                            if annotated_owner == owner
                                && signatures::text_query(name.as_str()).is_some()
                                && expected.as_ref().is_some_and(|expected| {
                                    actual == Term::ComptimeFunction(Box::new(expected.clone()))
                                })
                            {
                                expected = Some(actual.clone());
                                signature = Some(actual.clone());
                            }
                            let expected = expected.as_ref().ok_or_else(|| error(span, "typed compiler operation requires a supported callable signature contract"))?;
                            if &actual != expected {
                                return Err(error(
                                    annotation.span,
                                    format!(
                                        "compiler operation signature mismatch: expected {expected}, found {actual}"
                                    ),
                                ));
                            }
                        }
                    }
                    return Ok(Some((name, owner, signature)));
                }
                H::ResolvedDef { def_id, .. } => {
                    let Some(next) = self.package.definition_owner(*def_id) else {
                        return Ok(None);
                    };
                    owner = next;
                }
                _ => return Ok(None),
            }
        }
    }

    fn is_source_span_term(&mut self, ty: &Term, span: Span) -> bool {
        let _ = self.ensure_nominal_type(ty, span);
        let Term::Nominal {
            declaration: out_decl,
            arguments: out_args,
        } = ty
        else {
            return false;
        };
        if !out_args.is_empty() {
            return false;
        }
        let Some(out_def) = self.structs.get(out_decl) else {
            return false;
        };
        out_def.fields.as_slice()
            == [
                ("file".to_string(), p(P::Str)),
                ("start".to_string(), p(P::Usize)),
                ("end".to_string(), p(P::Usize)),
                ("expansion_id".to_string(), p(P::Isize)),
            ]
    }

    fn unwrap_meta_list_elem<'t>(&self, ty: &'t Term) -> Option<&'t Term> {
        let Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } = ty
        else {
            return None;
        };
        if Some(*head) != self.protocols.meta_list {
            return None;
        }
        let [elem] = arguments.as_slice() else {
            return None;
        };
        Some(elem)
    }

    fn validate_implicit_query_fallback(&mut self, name: &str, actual: &Term, span: Span) -> bool {
        let Term::ComptimeFunction(inner) = actual else {
            return false;
        };
        let Term::Function { parameters, result } = inner.as_ref() else {
            return false;
        };
        let [param] = parameters.as_slice() else {
            return false;
        };
        if param.sort != Sort::Runtime {
            return false;
        }
        match name {
            "declaration_span" => {
                return param.ty == p(P::Type) && self.is_source_span_term(result, span);
            }
            "struct_field_spans" | "record_field_spans" | "enum_variant_spans" => {
                return param.ty == p(P::Type)
                    && self
                        .unwrap_meta_list_elem(result)
                        .cloned()
                        .is_some_and(|elem| self.is_source_span_term(&elem, span));
            }
            "enum_variant_field_spans" => {
                return param.ty == p(P::Type)
                    && self
                        .unwrap_meta_list_elem(result)
                        .and_then(|inner| self.unwrap_meta_list_elem(inner))
                        .cloned()
                        .is_some_and(|elem| self.is_source_span_term(&elem, span));
            }
            _ => {}
        }
        let _ = self.ensure_nominal_type(&param.ty, span);
        let Term::Nominal {
            declaration: input_decl,
            arguments: input_args,
        } = &param.ty
        else {
            return false;
        };
        if !input_args.is_empty() {
            return false;
        }
        let Some(input_def) = self.structs.get(input_decl) else {
            return false;
        };
        match name {
            "implicit_caller" | "implicit_module" | "implicit_package" => {
                input_def.fields.len() == 3
                    && input_def.fields.iter().all(|(_, ty)| *ty == p(P::Str))
                    && **result == p(P::Str)
            }
            "implicit_file" => input_def.fields.is_empty() && **result == p(P::Str),
            "implicit_line" => input_def.fields.is_empty() && **result == p(P::U32),
            "implicit_src" => input_def.fields.is_empty() && self.is_source_span_term(result, span),
            _ => false,
        }
    }
}
