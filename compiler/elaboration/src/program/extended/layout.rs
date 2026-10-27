//! Keep property producer order separate from the completed record layout.
use super::*;

pub(in crate::program) fn property_order(
    properties: &[hir::expr::FieldExpr<'_>],
    names: Option<&[String]>,
    span: Span,
) -> Result<Vec<usize>, Error> {
    let mut source = BTreeMap::new();
    for (index, property) in properties.iter().enumerate() {
        if source.insert(property.ident.name.as_str(), index).is_some() {
            return Err(error(property.span, "property supplied more than once"));
        }
    }
    let Some(names) = names else {
        return Ok((0..properties.len()).collect());
    };
    if names.len() != properties.len() {
        return Err(error(
            span,
            "property argument fields do not match the expected record shape",
        ));
    }
    names
        .iter()
        .map(|name| {
            source.remove(name.as_str()).ok_or_else(|| {
                error(
                    span,
                    "property argument fields do not match the expected record shape",
                )
            })
        })
        .collect()
}

impl Checker<'_, '_> {
    pub(in crate::program) fn finish_property_packet(
        &mut self,
        span: Span,
        types: Vec<(String, Term)>,
        mut fields: Vec<n::Expr>,
        order: &[usize],
    ) -> Result<n::Expr, Error> {
        let reordered = order
            .iter()
            .enumerate()
            .any(|(index, source)| index != *source);
        let mut statements = Vec::new();
        if reordered {
            for field in &mut fields {
                self.stage_property_input(field, &mut statements, span)?;
            }
        }
        let mut types: Vec<_> = types.into_iter().map(Some).collect();
        let mut fields: Vec<_> = fields.into_iter().map(Some).collect();
        let mut target_types = Vec::new();
        let mut target_fields = Vec::new();
        for &index in order {
            target_types.push(
                types[index]
                    .take()
                    .ok_or_else(|| error(span, "duplicate property layout slot"))?,
            );
            target_fields.push(
                fields[index]
                    .take()
                    .ok_or_else(|| error(span, "duplicate property value slot"))?,
            );
        }
        let packet = n::Expr {
            ty: Term::Record(target_types),
            source: source(span),
            kind: K::AnonymousRecord {
                fields: target_fields,
            },
        };
        if !reordered {
            return Ok(packet);
        }
        Ok(n::Expr {
            ty: packet.ty.clone(),
            source: source(span),
            kind: K::Block {
                statements,
                result: Some(Box::new(packet)),
            },
        })
    }
}

/// The outer call interleaves these producers with positional inputs. Undo only
/// our packet-local staging, then let that call stage fields by their names.
pub(super) fn unprepare_packet(packet: &mut n::Expr, span: Span) -> Result<(), Error> {
    if !matches!(packet.kind, K::Block { .. }) {
        return Ok(());
    }
    let K::Block {
        statements,
        result: Some(result),
    } = std::mem::replace(&mut packet.kind, K::Unit)
    else {
        return Err(error(span, "property packet has no completed record"));
    };
    let K::AnonymousRecord { fields } = result.kind else {
        return Err(error(
            span,
            "property preparation does not complete a record",
        ));
    };
    let mut producers = BTreeMap::new();
    for statement in statements {
        let K::Let { binding, value } = statement.kind else {
            return Err(error(span, "invalid property preparation step"));
        };
        if producers.insert(binding, *value).is_some() {
            return Err(error(span, "duplicate property producer"));
        }
    }
    let mut restored = Vec::new();
    for field in fields {
        let K::Local(binding) = field.kind else {
            return Err(error(span, "property preparation has an unstaged field"));
        };
        restored.push(
            producers
                .remove(&binding)
                .ok_or_else(|| error(span, "missing property producer"))?,
        );
    }
    if !producers.is_empty() {
        return Err(error(span, "unused property producer"));
    }
    packet.kind = K::AnonymousRecord { fields: restored };
    Ok(())
}

pub(super) fn field_positions(ty: &Term, span: Span) -> Result<BTreeMap<String, usize>, Error> {
    let Term::Record(fields) = ty else {
        return Err(error(span, "property packet requires a record type"));
    };
    Ok(fields
        .iter()
        .enumerate()
        .map(|(index, (name, _))| (name.clone(), index))
        .collect())
}
