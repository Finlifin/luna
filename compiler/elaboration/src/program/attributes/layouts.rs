//! Generated declarations visible to attribute suppliers in this checking pass.
//! Frozen snapshots and the final package retain no callback into this index.
use super::*;
use crate::definitions::PublishedDefinitions;
use comptime_interpretor::{NominalBatch, SourcedValue};
use std::sync::Arc;

#[derive(Default)]
pub(in crate::program) struct Generated {
    batches: BTreeMap<DeclarationId, NominalBatch>,
}

fn batches(definitions: &PublishedDefinitions) -> impl Iterator<Item = NominalBatch> + '_ {
    definitions
        .structs
        .iter()
        .cloned()
        .map(NominalBatch::Structs)
        .chain(definitions.enums.iter().cloned().map(NominalBatch::Enums))
        .chain(
            definitions
                .newtypes
                .iter()
                .cloned()
                .map(NominalBatch::Newtypes),
        )
}

impl Generated {
    pub(in crate::program) fn charge_registration(
        &self,
        definitions: &PublishedDefinitions,
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let mut length = self.batches.len();
        for batch in batches(definitions) {
            length = length.saturating_add(batch.len());
            let cost = (batch.len() as u64)
                .saturating_mul(u64::from(usize::BITS - length.leading_zeros()) + 1);
            *steps = steps.checked_sub(cost.saturating_add(1)).ok_or_else(|| {
                error(span, "generated attribute layout indexing budget exhausted")
            })?;
        }
        Ok(())
    }

    pub(in crate::program) fn register(&mut self, definitions: &PublishedDefinitions) {
        for batch in batches(definitions) {
            for i in 0..batch.len() {
                self.batches.insert(batch.declaration(i), batch.clone());
            }
        }
    }

    pub(in crate::program) fn deliver(
        &self,
        value: Option<Arc<Term>>,
        known: &dyn Fn(DeclarationId) -> bool,
        steps: &mut u64,
        span: Span,
    ) -> Result<Option<SourcedValue>, Error> {
        let Some(value) = value else {
            return Ok(None);
        };
        let mut definitions = Vec::new();
        if !self.batches.is_empty() {
            let before = usize::try_from(*steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let result = self.dependencies(&value, known, &mut fuel, &mut definitions);
            *steps -= (before - fuel) as u64;
            result.map_err(|_| {
                error(
                    span,
                    "generated attribute layout traversal budget exhausted",
                )
            })?;
        }
        Ok(Some(SourcedValue { value, definitions }))
    }

    fn dependencies(
        &self,
        value: &Term,
        known: &dyn Fn(DeclarationId) -> bool,
        fuel: &mut usize,
        output: &mut Vec<NominalBatch>,
    ) -> Result<(), nir::type_computations::Error> {
        use nir::type_computations::{Error::Budget, nominal_declarations_bounded};
        enum Visit {
            Needed(DeclarationId),
            Ready(NominalBatch),
        }
        let mut dependencies = BTreeSet::new();
        nominal_declarations_bounded(value, fuel, &mut dependencies)?;
        let mut pending: Vec<_> = dependencies
            .iter()
            .rev()
            .copied()
            .map(Visit::Needed)
            .collect();
        let mut selected = BTreeSet::new();
        while let Some(visit) = pending.pop() {
            let id = match visit {
                Visit::Needed(id) => id,
                Visit::Ready(batch) => {
                    output.push(batch);
                    continue;
                }
            };
            let lookup = (usize::BITS - self.batches.len().leading_zeros()) as usize + 2;
            *fuel = fuel.checked_sub(lookup).ok_or(Budget)?;
            let Some(batch) = self.batches.get(&id) else {
                continue;
            };
            if known(id) || !selected.insert(batch.declaration(0)) {
                continue;
            }
            pending.push(Visit::Ready(batch.clone()));
            dependencies.clear();
            *fuel = fuel.checked_sub(batch.len()).ok_or(Budget)?;
            match batch {
                NominalBatch::Structs(defs) => {
                    for def in defs.iter() {
                        for (_, ty) in &def.fields {
                            nominal_declarations_bounded(ty, fuel, &mut dependencies)?;
                        }
                    }
                }
                NominalBatch::Enums(defs) => {
                    for def in defs.iter() {
                        *fuel = fuel.checked_sub(def.variants.len()).ok_or(Budget)?;
                        for ty in def.variants.iter().flat_map(|v| &v.fields) {
                            nominal_declarations_bounded(ty, fuel, &mut dependencies)?;
                        }
                    }
                }
                NominalBatch::Newtypes(defs) => {
                    for def in defs.iter() {
                        nominal_declarations_bounded(&def.representation, fuel, &mut dependencies)?;
                    }
                }
            }
            pending.extend(dependencies.iter().rev().copied().map(Visit::Needed));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_only_missing_reachable_batches_in_dependency_order() {
        let id = |local| DeclarationId { package: 81, local };
        let nominal = |local| Term::Nominal {
            declaration: id(local),
            arguments: vec![],
        };
        let record = |local, ty| {
            Arc::from([n::Struct {
                declaration: id(local),
                name: format!("D{local}"),
                fields: vec![("value".into(), ty)],
                packed: nir::program::PackedMode::None,
                requirements: vec![],
                capabilities: vec![],
                type_parameters: vec![],
                parameter_types: vec![],
            }])
        };
        let leaf = Term::Primitive(P::Bool);
        let definitions = PublishedDefinitions {
            structs: vec![
                record(2, nominal(9)),
                record(7, leaf.clone()),
                record(9, leaf),
            ],
            ..Default::default()
        };
        let mut layouts = Generated::default();
        layouts
            .charge_registration(&definitions, &mut 1000, Span::default())
            .unwrap();
        layouts.register(&definitions);
        let payload = Arc::new(nir::objects::value(vec![nir::objects::entry(
            Some("field"),
            Term::Primitive(P::Type),
            nominal(2),
        )]));
        let plan = |known: &[u32]| {
            let delivered = layouts
                .deliver(
                    Some(payload.clone()),
                    &|id| known.contains(&id.local),
                    &mut 1000,
                    Span::default(),
                )
                .unwrap();
            delivered
                .unwrap()
                .definitions
                .iter()
                .map(|b| b.declaration(0).local)
                .collect::<Vec<_>>()
        };
        assert_eq!(plan(&[]), [9, 2]);
        assert_eq!(plan(&[9]), [2]);
        assert!(plan(&[2]).is_empty());
    }
}
