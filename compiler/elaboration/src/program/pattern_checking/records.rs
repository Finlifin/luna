//! Constructor identity and declaration-order binding for named fields.
use super::*;
use hir::pattern::FieldPat;

pub(super) struct RecordHead {
    pub(super) ty: Term,
    pub(super) variant: Option<usize>,
}

impl Checker<'_, '_> {
    pub(super) fn named_record_head(
        &mut self,
        callee: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<RecordHead, Error> {
        if !self.is_type_head(callee)
            && let Some((ty, index)) = self.variant_target(callee, expected)?
        {
            return Ok(RecordHead {
                ty,
                variant: Some(index),
            });
        }
        // Check the head in type position, including ordinary comptime Type
        // values. Field checking requests the layout after admitting the label.
        Ok(RecordHead {
            ty: self.type_expr(callee)?,
            variant: None,
        })
    }

    pub(super) fn is_copy_type_for_pattern(&self, ty: &Term) -> bool {
        if self.intrinsic_pointer_input(ty) {
            return true;
        }
        let Some(copy_protocol) = self.protocols.copy else {
            return false;
        };
        if !self
            .checked_traits
            .iter()
            .any(|interface| interface.declaration == copy_protocol)
        {
            return false;
        }
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        matches!(
            crate::verification::copy::classify(
                &world,
                copy_protocol,
                ty,
                crate::ptc::Budget::default(),
            ),
            Ok(crate::verification::copy::Classification::Intrinsic
                | crate::verification::copy::Classification::Declared { .. }
                | crate::verification::copy::Classification::Assumed(_))
        )
    }

    pub(super) fn has_custom_drop(&self, ty: &Term) -> bool {
        if !self.interfaces_ready {
            return false;
        }
        let Some(drop_protocol) = self.protocols.drop else {
            return false;
        };
        if !self
            .checked_traits
            .iter()
            .any(|interface| interface.declaration == drop_protocol)
        {
            return false;
        }
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        world
            .resolve(drop_protocol, ty, crate::ptc::Budget::default())
            .is_ok_and(|resolution| {
                matches!(
                    resolution.types.selection,
                    crate::implementation::Selection::Selected(_)
                )
            })
    }

    pub(super) fn pattern_moves_non_copy(
        &mut self,
        pattern: &n::Pattern,
        ty: &Term,
        span: Span,
    ) -> Result<bool, Error> {
        self.pattern_moves_non_copy_at(pattern, ty, span, 0)
    }

    fn pattern_moves_non_copy_at(
        &mut self,
        pattern: &n::Pattern,
        ty: &Term,
        span: Span,
        depth: usize,
    ) -> Result<bool, Error> {
        if depth >= 128 {
            return Err(error(span, "pattern ownership check depth exhausted"));
        }
        match pattern {
            n::Pattern::Wild
            | n::Pattern::Unit
            | n::Pattern::Literal(_)
            | n::Pattern::OptionalAbsent => Ok(false),
            n::Pattern::Binding { inner, .. } => {
                if !self.is_copy_type_for_pattern(ty) {
                    return Ok(true);
                }
                self.pattern_moves_non_copy_at(inner, ty, span, depth + 1)
            }
            n::Pattern::Guard { pattern, .. } | n::Pattern::Remap { pattern, .. } => {
                self.pattern_moves_non_copy_at(pattern, ty, span, depth + 1)
            }
            n::Pattern::Observe { inner, .. } => {
                // Observe creates a candidate-local non-owning alias, except
                // Then transfers below may consume if non-Copy.
                if let n::Pattern::Then { first, value, .. } = inner.as_ref()
                    && matches!(value.kind, K::ReadPointer(_))
                {
                    return self.pattern_moves_non_copy_at(first, ty, span, depth + 1);
                }
                self.pattern_moves_non_copy_at(inner, ty, span, depth + 1)
            }
            n::Pattern::Then { first, value, next } => {
                Ok(self.pattern_moves_non_copy_at(first, ty, span, depth + 1)?
                    || self.pattern_moves_non_copy_at(next, &value.ty, span, depth + 1)?)
            }
            n::Pattern::Or(alts) => {
                for alt in alts {
                    if self.pattern_moves_non_copy_at(alt, ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::Tuple(fields) => {
                let shape = self.structural_type(ty, span)?;
                let Some(types) = self.bound_tuple_elements(&shape).or_else(|| match &shape {
                    Term::Tuple(types) => Some(types.as_slice()),
                    _ => None,
                }) else {
                    return Ok(false);
                };
                for (field, field_ty) in fields.iter().zip(types) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::AnonymousRecord { fields, .. } => {
                let Term::Record(types) = self.structural_type(ty, span)? else {
                    return Ok(false);
                };
                for (field, (_, field_ty)) in fields.iter().zip(&types) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::Record { fields, .. } => {
                let shape = self.structural_type(ty, span)?;
                let types = self.instantiated_fields(&shape, span)?;
                for (field, (_, field_ty)) in fields.iter().zip(&types) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::OptionalPresent(inner) => {
                let Term::Optional(inner_ty) = self.type_identity_view(ty, span)? else {
                    return Ok(false);
                };
                self.pattern_moves_non_copy_at(inner, &inner_ty, span, depth + 1)
            }
            n::Pattern::Variant { index, fields, .. } => {
                let shape = self.structural_type(ty, span)?;
                let variants = self.instantiated_variants(&shape, span)?;
                let Some(variant) = variants.get(*index) else {
                    return Ok(false);
                };
                for (field, field_ty) in fields.iter().zip(&variant.fields) {
                    if self.pattern_moves_non_copy_at(field, field_ty, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            n::Pattern::ErrorResult {
                label: Some(label),
                inner,
            } => self.pattern_moves_non_copy_at(inner, label, span, depth + 1),
            n::Pattern::ErrorResult { label: None, inner } => {
                let Term::ErrorResult { result, .. } = ty else {
                    return Ok(false);
                };
                self.pattern_moves_non_copy_at(inner, result, span, depth + 1)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn check_rest_drop_partial_move(
        &mut self,
        ty: &Term,
        checked_fields: &[n::Pattern],
        field_types: &[Term],
        has_omitted: bool,
        span: Span,
    ) -> Result<(), Error> {
        if !has_omitted || !self.has_custom_drop(ty) {
            return Ok(());
        }
        for (field, field_ty) in checked_fields.iter().zip(field_types) {
            if self.pattern_moves_non_copy(field, field_ty, span)? {
                return Err(error(
                    span,
                    "cannot partially move non-Copy field from a type that implements Drop under `...`",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn record_pattern(
        &mut self,
        ty: &Term,
        fields: &[FieldPat<'_>],
        rest: bool,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        let shape = self.structural_type(ty, span)?;
        let (declaration, declared) = match shape {
            Term::Record(fields) => (None, fields),
            Term::Nominal { declaration, .. } if self.structs.contains_key(&declaration) => {
                (Some(declaration), self.instantiated_fields(&shape, span)?)
            }
            _ => {
                return Err(error(
                    span,
                    "record pattern requires a struct type; custom pattern view elaboration is not implemented",
                ));
            }
        };
        let mut routed = vec![None; declared.len()];
        for field in fields {
            let slot = declared
                .iter()
                .position(|(name, _)| name == field.ident.name.as_str())
                .ok_or_else(|| error(field.span, "unknown struct pattern field"))?;
            if routed[slot].replace(field).is_some() {
                return Err(error(field.span, "duplicate struct pattern field"));
            }
        }
        let has_omitted = routed.iter().any(Option::is_none);
        // A guard in a later declared field can observe an earlier binding.
        // Unmentioned fields act as wildcards whether omitted silently or via `...`.
        let mut checked = Vec::with_capacity(declared.len());
        for (field, (_, ty)) in routed.into_iter().zip(&declared) {
            checked.push(match field {
                Some(field) => self.pattern(&field.pat, ty)?,
                None => n::Pattern::Wild,
            });
        }
        if rest && declaration.is_some() {
            let field_types: Vec<Term> = declared.iter().map(|(_, ty)| ty.clone()).collect();
            self.check_rest_drop_partial_move(ty, &checked, &field_types, has_omitted, span)?;
        }
        Ok(match declaration {
            Some(declaration) => n::Pattern::Record {
                declaration,
                fields: checked,
            },
            None => n::Pattern::AnonymousRecord {
                names: declared.into_iter().map(|(name, _)| name).collect(),
                fields: checked,
            },
        })
    }
}
