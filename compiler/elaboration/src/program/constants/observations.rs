//! Intersection of structural evidence across ordinary control-flow paths.
use super::*;
use std::sync::Arc;

impl PartialStagedValue {
    pub(super) fn common(
        left: &Arc<Self>,
        right: &Arc<Self>,
        meta_list: Option<DeclarationId>,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<Arc<Self>, String> {
        if depth >= 128 {
            return Ok(Self::unknown());
        }
        left.charge(fuel)?;
        right.charge(fuel)?;
        // Equality of partial evidence preserves unknown leaves as unknown;
        // it never asserts that two unknown runtime values are equal.
        if left == right {
            return Ok(left.clone());
        }
        let Some((left_shape, left_fields)) = left.aggregate(meta_list) else {
            return Ok(Self::unknown());
        };
        let Some((right_shape, right_fields)) = right.aggregate(meta_list) else {
            return Ok(Self::unknown());
        };
        if left_shape != right_shape {
            return Ok(Self::unknown());
        }
        // List fields describe a guaranteed prefix of evidence, not an exact
        // runtime length. Other aggregate layouts still require equal arity.
        if left_fields.len() != right_fields.len()
            && !matches!(left_shape, PartialStagedShape::List(_))
        {
            return Ok(Self::unknown());
        }
        let mut fields = Vec::with_capacity(left_fields.len().min(right_fields.len()));
        let mut height = 1;
        for (a, b) in left_fields.iter().zip(&right_fields) {
            let field = Self::common(a, b, meta_list, depth + 1, fuel)?;
            height = height.max(field.depth + 1);
            fields.push(field);
        }
        if height >= 128 {
            return Ok(Self::unknown());
        }
        Ok(Arc::new(Self {
            depth: height,
            data: PartialStagedData::Aggregate(left_shape, fields),
        }))
    }

    // The caller charges the complete evidence before cloning. Decompose only
    // for comparison: fully known constants retain their normal representation.
    fn aggregate(
        &self,
        meta_list: Option<DeclarationId>,
    ) -> Option<(PartialStagedShape, Vec<Arc<Self>>)> {
        let known_fields = |fields: &[Term]| fields.iter().cloned().map(Self::known).collect();
        Some(match &self.data {
            PartialStagedData::Aggregate(shape, fields) => (shape.clone(), fields.clone()),
            PartialStagedData::Known(Term::TypeListValue(fields)) => (
                PartialStagedShape::List(Term::NormalForm {
                    head: meta_list?,
                    sort: Sort::Type,
                    arguments: vec![Term::Primitive(nir::Primitive::Type)],
                }),
                known_fields(fields),
            ),
            PartialStagedData::Known(Term::NominalValue {
                ty,
                variant: None,
                fields,
            }) if matches!(ty.as_ref(), Term::NormalForm { head, sort: Sort::Type, arguments }
                if Some(*head) == meta_list && matches!(arguments.as_slice(), [element] if element.sort() == Sort::Type)) =>
            {
                (
                    PartialStagedShape::List(ty.as_ref().clone()),
                    known_fields(fields),
                )
            }
            PartialStagedData::Known(Term::TupleValue(fields)) => {
                (PartialStagedShape::Tuple, known_fields(fields))
            }
            PartialStagedData::Known(Term::RecordValue(fields)) => (
                PartialStagedShape::AnonymousRecord(
                    fields.iter().map(|(name, _)| name.clone()).collect(),
                ),
                fields
                    .iter()
                    .map(|(_, value)| Self::known(value.clone()))
                    .collect(),
            ),
            PartialStagedData::Known(Term::NominalValue {
                ty,
                variant,
                fields,
            }) => {
                let Term::Nominal { declaration, .. } = ty.as_ref() else {
                    return None;
                };
                let shape = match variant {
                    Some(index) => {
                        PartialStagedShape::Variant(*declaration, usize::try_from(*index).ok()?)
                    }
                    None => PartialStagedShape::Record(*declaration),
                };
                (shape, known_fields(fields))
            }
            _ => return None,
        })
    }
}

/// An observation-only lexical environment. No VM storage or inference state is
/// copied; known terms are supplied only to the existing structural inspector.
#[derive(Clone, Default)]
pub(super) struct ObservationScope {
    pub(super) values: BTreeMap<n::LocalId, Arc<PartialStagedValue>>,
    pub(super) known: BTreeMap<n::LocalId, Arc<Term>>,
}
impl ObservationScope {
    pub(super) fn fork(&self, fuel: &mut usize) -> Result<Self, String> {
        *fuel = fuel
            .checked_sub(self.values.len().saturating_add(self.known.len()))
            .ok_or("partial block observation budget exhausted")?;
        Ok(self.clone())
    }

    pub(super) fn insert(
        &mut self,
        binding: n::LocalId,
        value: Arc<PartialStagedValue>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("partial block observation budget exhausted")?;
        if let PartialStagedData::Known(term) = &value.data {
            nir::type_computations::charge_term(term, fuel).map_err(|e| e.to_string())?;
            self.known.insert(binding, Arc::new(term.clone()));
        } else {
            self.known.remove(&binding);
        }
        self.values.insert(binding, value);
        Ok(())
    }
}
