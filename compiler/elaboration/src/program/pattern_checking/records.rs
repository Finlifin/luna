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
        if rest {
            return Err(error(span, "record rest binding is not implemented"));
        }
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
        // A guard in a later declared field can observe an earlier binding.
        let mut checked = Vec::with_capacity(declared.len());
        for (field, (_, ty)) in routed.into_iter().zip(&declared) {
            checked.push(match field {
                Some(field) => self.pattern(&field.pat, ty)?,
                None => n::Pattern::Wild,
            });
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
