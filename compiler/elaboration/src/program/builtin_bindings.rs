//! Named bindings for compiler operations retain their checked call lowering.
use super::*;

fn operation(name: &str) -> bool {
    matches!(
        name,
        "list_length"
            | "list_concat"
            | "comptime_integer_convert"
            | "comptime_integer_div_trunc"
            | "comptime_integer_rem_trunc"
            | "integer_representation"
            | "str_byte_length"
            | "str_byte"
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
            | "struct_field_types"
            | "struct_field_names"
            | "enum_group_paths"
            | "enum_variant_paths"
            | "enum_variant_shapes"
            | "enum_variant_field_names"
            | "enum_variant_field_types"
            | "build_struct"
            | "build_enum"
            | "build_enum_paths"
            | "build_symbol_enum_paths"
            | "build_bits_enum_paths"
    )
}

impl Checker<'_, '_> {
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
            .package
            .owners()
            .find_map(|(owner, _)| {
                matches_declaration(self.package, &self.package_identities, owner, identity)
                    .then_some(owner)
            })
            .ok_or_else(|| error(span, "builtin constructor binding is unavailable"))?;
        if canonical != owner {
            let value = self.normal_form_value(initializer, canonical, None)?;
            if !matches!(annotation.kind, H::TyPlaceholder) {
                let expected = self.type_expr(annotation)?;
                self.require_expression(&value.ty, &expected, span)?;
            }
            return Ok(Some(value));
        }
        if !self.normal_parameters.contains_key(&owner) {
            if self.active_signatures.len() >= 64 || !self.active_signatures.insert(owner) {
                return Err(error(span, "cyclic builtin constructor signature"));
            }
            let prepared = (|| {
                let ty = self.type_expr(annotation)?;
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
                let Term::Function { parameters, .. } = ty.callable_signature() else {
                    return Err(error(span, "builtin binding requires a callable signature"));
                };
                if inputs.len() != parameters.len() {
                    return Err(error(span, "builtin callable parameter metadata mismatch"));
                }
                let mut descriptors = Vec::new();
                let mut prior = Vec::new();
                for (input, parameter) in inputs.iter().zip(parameters) {
                    let id = DeclarationId {
                        package: function_types::INPUT_NAMESPACE,
                        local: self.next_temporary_binding,
                    };
                    self.next_temporary_binding =
                        self.next_temporary_binding.checked_add(1).ok_or_else(|| {
                            error(span, "builtin constructor input identity exhausted")
                        })?;
                    let domain = parameter
                        .ty
                        .substitute(&prior)
                        .map_err(|reason| error(span, reason.to_string()))?;
                    descriptors.push((id, local(input.hir_id), domain));
                    prior.push(Term::Rigid {
                        declaration: id,
                        sort: parameter.sort,
                    });
                }
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
            self.normal_parameters.insert(owner, descriptors);
        }
        self.ensure_contract(owner, span)?;
        let ty = self
            .normal_form_head_metadata(owner)
            .unwrap()
            .callable_type()
            .map_err(|reason| error(span, reason.to_string()))?;
        Ok(Some(n::Expr {
            source: source(span),
            ty,
            kind: K::Function(identity),
        }))
    }

    pub(super) fn compiler_operation_binding(
        &mut self,
        mut owner: OwnerId,
        span: Span,
    ) -> Result<Option<hir::Symbol>, Error> {
        let mut visited = BTreeSet::new();
        loop {
            let Some(item) = self.package.item(owner) else {
                return Ok(None);
            };
            let ItemKind::Const(annotation, initializer) = item.kind else {
                return Ok(None);
            };
            if !matches!(annotation.kind, H::TyPlaceholder) {
                return Ok(None);
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
                H::Builtin(name) if operation(name.as_str()) => return Ok(Some(*name)),
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
}
