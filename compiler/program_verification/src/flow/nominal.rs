//! Nominal shapes come from package declarations, never inferred from payloads.
use super::*;
impl Walker<'_> {
    pub(super) fn place_fields(
        &mut self,
        ty: &Term,
        owner: nir::program::FieldOwner,
    ) -> Result<Vec<Term>, Reason> {
        match owner {
            nir::program::FieldOwner::Nominal(id) => self.nominal_fields(ty, id, None),
            nir::program::FieldOwner::Anonymous => {
                if let Some(fields) = self.tuple_field_types(ty)? {
                    return Ok(fields);
                }
                let Term::Record(fields) = ty else {
                    return Err(Reason::InvalidSignature);
                };
                self.limits.step(fields.len().saturating_add(1))?;
                Ok(fields.iter().map(|(_, ty)| ty.clone()).collect())
            }
        }
    }
    pub(super) fn read_anonymous_field(
        &mut self,
        base: Value,
        ty: &Term,
        index: usize,
        state: &mut State,
    ) -> Result<Value, Reason> {
        let actual_type = base.nominal_type.as_deref().unwrap_or(ty);
        let kind = if self.tuple_field_types(actual_type)?.is_some() {
            AggregateKind::Tuple
        } else {
            AggregateKind::AnonymousRecord
        };
        let types = self.place_fields(actual_type, nir::program::FieldOwner::Anonymous)?;
        let field_type = types.get(index).ok_or(Reason::InvalidSignature)?;
        let projection = base.projection.clone();
        let mut result = if let Some(fields) = base.fields {
            if base.aggregate_kind != Some(kind) {
                return Err(Reason::InvalidSignature);
            }
            fields
                .get(index)
                .ok_or(Reason::InvalidSignature)?
                .snapshot(self.limits, 0)?
        } else {
            self.unknown_value(field_type)?
        };
        self.project_component(state, &mut result, projection, (kind, index))?;
        Ok(result)
    }
    pub(super) fn read_record_field(
        &mut self,
        base: Value,
        ty: &Term,
        declaration: nir::DeclarationId,
        index: usize,
        state: &mut State,
    ) -> Result<Value, Reason> {
        let projection = base.projection.clone();
        // Call contracts are evaluated in a frame populated with actual inputs.
        // Their template annotations can still name the callee's Type slots;
        // use the bound value's declared nominal instance, not that stale frame.
        let actual_type = base.nominal_type.as_deref().unwrap_or(ty);
        let types = self.nominal_fields(actual_type, declaration, None)?;
        let field_type = types.get(index).ok_or(Reason::InvalidSignature)?;
        let mut result = if let Some(fields) = base.fields {
            if base.aggregate_kind != Some(AggregateKind::Record(declaration)) {
                return Err(Reason::InvalidSignature);
            }
            fields
                .get(index)
                .ok_or(Reason::InvalidSignature)?
                .snapshot(self.limits, 0)?
        } else {
            self.unknown_value(field_type)?
        };
        self.project_component(
            state,
            &mut result,
            projection,
            (AggregateKind::Record(declaration), index),
        )?;
        Ok(result)
    }

    pub(super) fn nominal_fields(
        &mut self,
        ty: &Term,
        expected: nir::DeclarationId,
        variant: Option<usize>,
    ) -> Result<Vec<Term>, Reason> {
        self.limits.step(1)?;
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(Reason::Unsupported);
        };
        if *declaration != expected {
            return Err(Reason::InvalidSignature);
        }
        let package = self.package.ok_or(Reason::Unsupported)?;
        if let Some(index) = variant {
            self.limits.step(package.enums.len())?;
            let definition = package
                .enums
                .iter()
                .find(|d| d.declaration == expected)
                .ok_or(Reason::InvalidSignature)?;
            Term::Tuple(Vec::new())
                .instantiate_types_in_scope(
                    &definition.type_parameters,
                    arguments,
                    &self.input_frames,
                )
                .map_err(|_| Reason::InvalidSignature)?;
            let fields = &definition
                .variants
                .get(index)
                .ok_or(Reason::InvalidSignature)?
                .fields;
            self.limits.step(fields.len())?;
            fields
                .iter()
                .map(|ty| {
                    ty.instantiate_types_in_scope(
                        &definition.type_parameters,
                        arguments,
                        &self.input_frames,
                    )
                    .map_err(|_| Reason::InvalidSignature)
                })
                .collect()
        } else {
            self.limits.step(package.structs.len())?;
            let definition = package
                .structs
                .iter()
                .find(|d| d.declaration == expected)
                .ok_or(Reason::InvalidSignature)?;
            self.limits.step(definition.fields.len())?;
            definition
                .instantiate_fields_in_scope(arguments, &self.input_frames)
                .map(|fields| fields.into_iter().map(|(_, ty)| ty).collect())
                .map_err(|_| Reason::InvalidSignature)
        }
    }
}

impl Walker<'_> {
    /// NIR Place paths here contain direct owned record fields, not pointer
    /// dereferences or arbitrary aliases. The RHS has already been evaluated.
    pub(super) fn write_field(
        &mut self,
        state: &mut State,
        target: &nir::program::Place,
        value: Value,
    ) -> Result<(), Reason> {
        let transfer = self.prepare_copy(&value, state)?;
        let root = place(target.root);
        self.limits.step(target.fields.len().saturating_add(1))?;
        let Some(old) = state.aggregates.get(&root) else {
            state.invalidate_paths(Some(root), &[], self.limits)?;
            state.facts.invalidate(root)?;
            return Ok(());
        };
        let mut ty = old.nominal_type.clone().ok_or(Reason::Unsupported)?;
        let mut path = Vec::new();
        for (owner, index) in &target.fields {
            let kind = if *owner == nir::program::FieldOwner::Anonymous
                && self.tuple_field_types(&ty)?.is_some()
            {
                AggregateKind::Tuple
            } else {
                field_owner_kind(*owner)
            };
            let fields = self.place_fields(&ty, *owner)?;
            let next = fields.get(*index).ok_or(Reason::InvalidSignature)?;
            ty = std::sync::Arc::new(next.clone());
            path.push((kind, *index));
        }
        let mut updated = old.snapshot(self.limits, 0)?;
        state.invalidate_paths(Some(root), &path, self.limits)?;
        let replacement = value.snapshot(self.limits, 0)?;
        self.replace_field(&mut updated, &path, replacement, 0)?;
        state.facts.invalidate(root)?;
        updated.clear_evidence(self.limits)?;
        state.aggregates.insert(root, updated);
        if let Some(transfer) = transfer {
            let installed = self.install_copy_at(state, root, &path, transfer)?;
            self.finish_copies(state, installed)?;
        }
        Ok(())
    }
    pub(super) fn replace_field(
        &mut self,
        base: &mut Value,
        path: &[Component],
        replacement: Value,
        depth: usize,
    ) -> Result<(), Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        base.evidence = None;
        let Some(((kind, index), rest)) = path.split_first() else {
            *base = replacement;
            return Ok(());
        };
        let types = match kind {
            AggregateKind::EnumTag(_) | AggregateKind::OptionalTag | AggregateKind::ErrorTag => {
                return Err(Reason::InvalidSignature);
            }
            AggregateKind::ErrorResult(tag) => {
                let ty = base
                    .nominal_type
                    .as_deref()
                    .ok_or(Reason::InvalidSignature)?;
                Some(vec![self.error_payload_type(ty, *tag)?.clone()])
            }
            AggregateKind::Optional(present) => {
                let Term::Optional(payload) = base
                    .nominal_type
                    .as_deref()
                    .ok_or(Reason::InvalidSignature)?
                else {
                    return Err(Reason::InvalidSignature);
                };
                Some(if *present {
                    vec![(**payload).clone()]
                } else {
                    vec![]
                })
            }
            AggregateKind::AnonymousRecord | AggregateKind::Tuple => {
                let ty = base.nominal_type.as_ref().ok_or(Reason::Unsupported)?;
                Some(self.place_fields(ty, nir::program::FieldOwner::Anonymous)?)
            }
            AggregateKind::Record(declaration) => {
                let ty = base.nominal_type.as_ref().ok_or(Reason::Unsupported)?;
                Some(self.nominal_fields(ty, *declaration, None)?)
            }
            AggregateKind::Variant(declaration, variant) => {
                let ty = base.nominal_type.as_ref().ok_or(Reason::Unsupported)?;
                Some(self.nominal_fields(ty, *declaration, Some(*variant))?)
            }
        };
        if base.aggregate_kind.is_some_and(|actual| actual != *kind) {
            return Err(Reason::InvalidSignature);
        }
        if types.as_ref().is_some_and(|types| *index >= types.len()) {
            return Err(Reason::InvalidSignature);
        }
        if base.fields.is_none() {
            base.fields = Some(
                types
                    .as_ref()
                    .ok_or(Reason::Unsupported)?
                    .iter()
                    .map(|ty| self.unknown_value(ty))
                    .collect::<Result<_, _>>()?,
            );
            base.aggregate_kind = Some(*kind);
        }
        let fields = base.fields.as_mut().unwrap();
        if *index >= fields.len() || types.as_ref().is_some_and(|t| fields.len() != t.len()) {
            return Err(Reason::InvalidSignature);
        }
        let mut replacement = replacement;
        if rest.is_empty()
            && replacement.nominal_type.is_none()
            && types
                .as_ref()
                .is_some_and(|t| matches!(t[*index], Term::Nominal { .. }))
        {
            replacement.nominal_type =
                Some(std::sync::Arc::new(types.as_ref().unwrap()[*index].clone()));
        }
        self.replace_field(&mut fields[*index], rest, replacement, depth + 1)
    }
}
