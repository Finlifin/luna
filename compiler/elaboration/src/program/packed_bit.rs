//! Declaration, instantiation, and width-warning checks for `attrs.packed.bit` structs.
use super::*;
use n::PackedMode;
use nir::{BitPackedError, LayoutError};
use std::collections::HashSet;

fn term_is_concrete(root: &Term) -> bool {
    let mut pending = vec![root];
    while let Some(term) = pending.pop() {
        if matches!(
            term,
            Term::Rigid { .. }
                | Term::Bound { .. }
                | Term::Hole { .. }
                | Term::AssociatedType { .. }
                | Term::ContextualClosure { .. }
                | Term::ClosureContextType
                | Term::ClosureContextValue(_)
        ) {
            return false;
        }
        let Ok(()) = term.visit_children::<std::convert::Infallible>(|child| {
            pending.push(child);
            Ok(())
        });
    }
    true
}

fn bit_packed_instance_label(name: &str, arguments: &[Term]) -> String {
    if arguments.is_empty() {
        name.to_string()
    } else {
        let formatted: Vec<_> = arguments.iter().map(ToString::to_string).collect();
        format!("{name}<{}>", formatted.join(", "))
    }
}

impl Checker<'_, '_> {
    fn term_has_drop_impl(
        &self,
        context: &crate::ptc::Context,
        ty: &Term,
        visited: &mut HashSet<Term>,
        fuel: &mut usize,
    ) -> Result<bool, Error> {
        if !self.interfaces_ready {
            return Ok(false);
        }
        let Some(drop_protocol) = self.protocols.drop else {
            return Ok(false);
        };
        if !self
            .checked_traits
            .iter()
            .any(|interface| interface.declaration == drop_protocol)
        {
            return Ok(false);
        }
        if !visited.insert(ty.clone()) {
            return Ok(false);
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(Span::default(), "packed.bit Drop check budget exhausted"))?;

        let assumptions = [];
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context,
        };
        if let Ok(resolution) = world.resolve(drop_protocol, ty, crate::ptc::Budget::default())
            && matches!(
                resolution.types.selection,
                crate::implementation::Selection::Selected(_)
            )
        {
            return Ok(true);
        }

        if let Term::Nominal {
            declaration,
            arguments,
        } = ty
            && term_is_concrete(ty)
        {
            if let Some(newtype_def) = self.newtypes.get(declaration)
                && let Ok(repr) = newtype_def.representation_budgeted(arguments, &[], fuel)
            {
                return self.term_has_drop_impl(&self.nominal_context(), &repr, visited, fuel);
            }
            if let Some(struct_def) = self.structs.get(declaration)
                && struct_def.packed == PackedMode::Bit
                && let Ok(fields) = nir::normal_forms::instantiate_value_fields_checked(
                    &struct_def.type_parameters,
                    &struct_def.parameter_types,
                    arguments,
                    struct_def.fields.iter().map(|(_, ty)| ty),
                    fuel,
                )
            {
                for field_ty in fields {
                    if self.term_has_drop_impl(&self.nominal_context(), &field_ty, visited, fuel)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn report_bit_packed_error(
        &self,
        label: &str,
        field_names: &[(String, Term)],
        span: Span,
        err: BitPackedError,
    ) -> Error {
        match err {
            BitPackedError::UnsupportedField {
                field_index,
                ty,
                reason,
            } => {
                let field_name = field_names
                    .get(field_index)
                    .map(|(name, _)| name.as_str())
                    .unwrap_or("<field>");
                error(
                    span,
                    format!(
                        "attrs.packed.bit struct `{label}` field `{field_name}` of type `{ty}` is not allowed: {reason}"
                    ),
                )
            }
            BitPackedError::Layout(LayoutError::Cycle(_)) => error(
                span,
                format!(
                    "attrs.packed.bit struct `{label}` cannot contain a recursive by-value cycle"
                ),
            ),
            BitPackedError::Layout(LayoutError::ComptimeOnly(ty)) => error(
                span,
                format!(
                    "attrs.packed.bit struct `{label}` cannot contain comptime-only field type `{ty}`"
                ),
            ),
            BitPackedError::Layout(LayoutError::BudgetExhausted) => {
                error(span, "attrs.packed.bit layout validation budget exhausted")
            }
            BitPackedError::Layout(other) => {
                error(span, format!("attrs.packed.bit struct `{label}`: {other}"))
            }
            BitPackedError::WidthOverflow => error(
                span,
                format!(
                    "attrs.packed.bit struct `{label}` total bit width exceeds supported maximum"
                ),
            ),
        }
    }

    /// Validate all `attrs.packed.bit` struct declarations once nominal metadata
    /// and trait implementations have finished loading.
    pub(super) fn validate_bit_packed_structs(&mut self) -> Result<(), Error> {
        let entries: Vec<(OwnerId, PackedMode)> = self
            .packed_structs
            .iter()
            .filter(|(_, mode)| **mode == PackedMode::Bit)
            .map(|(&owner, &mode)| (owner, mode))
            .collect();
        for (owner, _) in entries {
            let decl = declaration(self.package, &self.package_identities, owner);
            let Some(def) = self.structs.get(&decl).cloned() else {
                continue;
            };
            let span = self
                .package
                .item(owner)
                .map(|item| item.span)
                .unwrap_or(rustc_span::DUMMY_SP);
            if def.type_parameters.is_empty() {
                self.validate_concrete_bit_packed_instance_with_def(decl, &def, &[], span)?;
            } else {
                self.validate_generic_bit_packed_declaration(owner, decl, &def, span)?;
            }
        }
        self.bit_packed_validation_ready = true;
        Ok(())
    }

    fn validate_generic_bit_packed_declaration(
        &self,
        owner: OwnerId,
        decl: DeclarationId,
        def: &n::Struct,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = 100_000usize;
        let label = def.name.as_str();
        let self_args: Vec<Term> = def
            .type_parameters
            .iter()
            .enumerate()
            .map(|(idx, &id)| {
                let sort = def
                    .parameter_types
                    .get(idx)
                    .map_or(Sort::Type, Sort::of_comptime_domain);
                Term::Rigid {
                    declaration: id,
                    sort,
                }
            })
            .collect();
        let self_ty = Term::Nominal {
            declaration: decl,
            arguments: self_args,
        };
        let binder_ctx = self.binder_context(owner);
        let mut visited = HashSet::new();
        if self.term_has_drop_impl(&binder_ctx, &self_ty, &mut visited, &mut fuel)? {
            return Err(error(
                span,
                format!("attrs.packed.bit struct `{label}` cannot implement `Drop`"),
            ));
        }
        for (field_index, (_, field_ty)) in def.fields.iter().enumerate() {
            match nir::bit_packed_field_info(self, field_ty, &mut fuel) {
                Ok(_) => {
                    if term_is_concrete(field_ty) {
                        let mut field_visited = HashSet::new();
                        if self.term_has_drop_impl(
                            &self.nominal_context(),
                            field_ty,
                            &mut field_visited,
                            &mut fuel,
                        )? {
                            return Err(error(
                                span,
                                format!(
                                    "attrs.packed.bit struct `{label}` field `{}` of type `{field_ty}` cannot have a `Drop` implementation",
                                    def.fields[field_index].0
                                ),
                            ));
                        }
                    }
                }
                Err(BitPackedError::Layout(
                    LayoutError::Unresolved(_) | LayoutError::TargetRequired,
                )) => {}
                Err(BitPackedError::UnsupportedField { ty, reason, .. }) => {
                    return Err(self.report_bit_packed_error(
                        label,
                        &def.fields,
                        span,
                        BitPackedError::UnsupportedField {
                            field_index,
                            ty,
                            reason,
                        },
                    ));
                }
                Err(err) => {
                    return Err(self.report_bit_packed_error(label, &def.fields, span, err));
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_bit_packed_type_application(
        &mut self,
        owner: OwnerId,
        arguments: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        let decl = declaration(self.package, &self.package_identities, owner);
        let Some(def) = self.structs.get(&decl).cloned() else {
            return Ok(());
        };
        if def.packed != PackedMode::Bit {
            return Ok(());
        }
        if arguments.iter().all(term_is_concrete) {
            self.validate_concrete_bit_packed_instance_with_def(decl, &def, arguments, span)?;
        }
        Ok(())
    }

    pub(super) fn validate_bit_packed_use(&mut self, ty: &Term, span: Span) -> Result<(), Error> {
        let mut pending = vec![ty];
        while let Some(current) = pending.pop() {
            if let Term::Nominal {
                declaration: decl,
                arguments,
            } = current
                && let Some(def) = self.structs.get(decl).cloned()
                && def.packed == PackedMode::Bit
                && arguments.iter().all(term_is_concrete)
            {
                self.validate_concrete_bit_packed_instance_with_def(*decl, &def, arguments, span)?;
            }
            let Ok(()) = current.visit_children::<std::convert::Infallible>(|child| {
                pending.push(child);
                Ok(())
            });
        }
        Ok(())
    }

    fn validate_concrete_bit_packed_instance_with_def(
        &mut self,
        declaration: DeclarationId,
        def: &n::Struct,
        arguments: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        if def.packed != PackedMode::Bit {
            return Ok(());
        }
        let key = (declaration, arguments.to_vec());
        if self.interfaces_ready
            && self.bit_packed_validation_ready
            && self.validated_bit_packed_instances.contains(&key)
        {
            return Ok(());
        }
        let mut fuel = 100_000usize;
        let label = bit_packed_instance_label(&def.name, arguments);
        let instantiated_fields = match nir::normal_forms::instantiate_value_fields_checked(
            &def.type_parameters,
            &def.parameter_types,
            arguments,
            def.fields.iter().map(|(_, ty)| ty),
            &mut fuel,
        ) {
            Ok(fields) => fields,
            Err(nir::NominalAdmissionError::Unresolved(_)) => return Ok(()),
            Err(err) => return Err(error(span, err.to_string())),
        };
        let self_ty = Term::Nominal {
            declaration,
            arguments: arguments.to_vec(),
        };
        if self.interfaces_ready {
            let mut visited = HashSet::new();
            if self.term_has_drop_impl(
                &self.nominal_context(),
                &self_ty,
                &mut visited,
                &mut fuel,
            )? {
                return Err(error(
                    span,
                    format!(
                        "attrs.packed.bit struct `{label}` cannot implement `Drop` or contain types with `Drop`"
                    ),
                ));
            }
        }
        match nir::bit_packed_struct_layout(self, &instantiated_fields, &mut fuel) {
            Ok(layout) => {
                if self.interfaces_ready {
                    for (field_index, field_ty) in instantiated_fields.iter().enumerate() {
                        let mut visited = HashSet::new();
                        if self.term_has_drop_impl(
                            &self.nominal_context(),
                            field_ty,
                            &mut visited,
                            &mut fuel,
                        )? {
                            return Err(error(
                                span,
                                format!(
                                    "attrs.packed.bit struct `{label}` field `{}` of type `{field_ty}` cannot implement `Drop`",
                                    def.fields[field_index].0
                                ),
                            ));
                        }
                    }
                    if self.bit_packed_validation_ready {
                        self.validated_bit_packed_instances.insert(key.clone());
                    }
                }
                if layout.total_bits > 256 && self.warned_bit_packed_instances.insert(key) {
                    let warn_source = self
                        .source_declarations
                        .owner(declaration)
                        .and_then(|owner| self.packed_struct_sources.get(&owner).copied())
                        .unwrap_or_else(|| source(span));
                    self.warnings.push(Warning {
                        code: Some("W_PACKED_BIT_WIDTH"),
                        source: warn_source,
                        message: format!(
                            "attrs.packed.bit struct `{label}` has total bit width {} exceeding 256 bits; if strict bit-level packing is unnecessary, consider using `attrs.packed.byte` or splitting into smaller bit-packed structs",
                            layout.total_bits
                        ),
                    });
                }
                Ok(())
            }
            Err(BitPackedError::Layout(LayoutError::TargetRequired)) => {
                // Without a target pointer width, still validate every target-independent
                // field so illegal fields (e.g. pointers, `str`) are rejected immediately.
                for (field_index, field_ty) in instantiated_fields.iter().enumerate() {
                    match nir::bit_packed_field_info(self, field_ty, &mut fuel) {
                        Ok(_) => {}
                        Err(BitPackedError::Layout(
                            LayoutError::TargetRequired | LayoutError::Unresolved(_),
                        )) => {}
                        Err(BitPackedError::UnsupportedField { ty, reason, .. }) => {
                            return Err(self.report_bit_packed_error(
                                &label,
                                &def.fields,
                                span,
                                BitPackedError::UnsupportedField {
                                    field_index,
                                    ty,
                                    reason,
                                },
                            ));
                        }
                        Err(err) => {
                            return Err(self.report_bit_packed_error(
                                &label,
                                &def.fields,
                                span,
                                err,
                            ));
                        }
                    }
                }
                Ok(())
            }
            Err(BitPackedError::Layout(LayoutError::Unresolved(_))) => Ok(()),
            Err(err) => Err(self.report_bit_packed_error(&label, &def.fields, span, err)),
        }
    }
}
