//! Supplier execution belongs to the checking pass. Only the final merged table
//! can escape into executable NIR; waiting VMs borrow this source temporarily.
use super::*;
use std::sync::Arc;

#[derive(Default)]
pub(in crate::program) struct Demands {
    suppliers: BTreeMap<DeclarationId, Arc<[usize]>>,
    pub(super) values: Vec<Option<EvaluatedAttribute>>,
    merged: BTreeMap<DeclarationId, Option<Arc<Term>>>,
    active: Vec<usize>,
    pub(super) evaluating: bool,
}

impl Demands {
    pub(in crate::program) fn owns(
        &self,
        current: &n::DefinitionAttributes,
        captured: &n::DefinitionAttributes,
    ) -> bool {
        match (current, captured) {
            (
                n::DefinitionAttributes::Pending(owner),
                n::DefinitionAttributes::Pending(snapshot),
            ) => self.evaluating && Arc::ptr_eq(owner, snapshot),
            _ => false,
        }
    }
}

pub(in crate::program) fn index_suppliers(
    package: &hir::Package<'_>,
    identities: &BTreeMap<u32, u64>,
    steps: &mut u64,
) -> Result<(n::DefinitionAttributes, Demands), Error> {
    let mut suppliers = BTreeMap::<DeclarationId, Vec<usize>>::new();
    for (index, attribute) in package.definition_attributes.iter().enumerate() {
        let work = 3 * u64::from(usize::BITS - suppliers.len().leading_zeros()) + 4;
        *steps = steps
            .checked_sub(work)
            .ok_or_else(|| error(attribute.span, "attribute target indexing budget exhausted"))?;
        if let hir::AttributeTarget::Definition(owner) = attribute.target {
            suppliers
                .entry(declaration(package, identities, owner))
                .or_default()
                .push(index);
        }
    }
    let pending = if suppliers.is_empty() {
        Default::default()
    } else {
        n::DefinitionAttributes::Pending(Arc::new(suppliers.keys().copied().collect()))
    };
    let demands = Demands {
        suppliers: suppliers
            .into_iter()
            .map(|(id, indices)| (id, indices.into()))
            .collect(),
        values: (0..package.definition_attributes.len())
            .map(|_| None)
            .collect(),
        ..Default::default()
    };
    Ok((pending, demands))
}

impl Checker<'_, '_> {
    pub(super) fn evaluate_supplier(&mut self, index: usize) -> Result<(), Error> {
        if self.attribute_demands.values[index].is_some() {
            return Ok(());
        }
        let attribute = &self.package.definition_attributes[index];
        let active = &mut self.attribute_demands.active;
        if let Some(start) = active.iter().position(|&i| i == index) {
            let path = active[start..]
                .iter()
                .chain(std::iter::once(&index))
                .map(|&i| match self.package.definition_attributes[i].target {
                    hir::AttributeTarget::Definition(owner) => self
                        .package
                        .item(owner)
                        .expect("active supplier has a declaration")
                        .ident
                        .name
                        .to_string(),
                    hir::AttributeTarget::Member(id) => {
                        hir::AttributeTarget::Member(id).to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(error(
                attribute.span,
                format!("cyclic definition attribute dependency: {path}"),
            ));
        }
        // Each synchronous source request retains the caller's VM and native
        // elaboration frames. Enforce a separate depth bound before entering.
        if active.len() >= 8 {
            return Err(error(
                attribute.span,
                format!("attribute supplier depth exhausted at {}", attribute.target),
            ));
        }
        active.push(index);
        // A caller may enter on a small host thread. Keep the VM on the same
        // thread (including its symbol session) while reserving stack for one
        // more source elaboration. The active limit bounds retained VMs.
        let result = stacker::maybe_grow(1024 * 1024, 4 * 1024 * 1024, || {
            self.evaluate_supplier_body(attribute)
        });
        self.attribute_demands.active.pop();
        self.attribute_demands.values[index] = Some(result?);
        Ok(())
    }

    pub(in crate::program) fn demand_definition_attributes(
        &mut self,
        target: DeclarationId,
    ) -> Result<Option<Arc<Term>>, Error> {
        let indices = self
            .attribute_demands
            .suppliers
            .get(&target)
            .expect("source-owned pending target")
            .clone();
        let span = self.package.definition_attributes[indices[0]].span;
        let lookup_cost =
            2 * u64::from(usize::BITS - self.attribute_demands.suppliers.len().leading_zeros()) + 1;
        self.required_steps = self
            .required_steps
            .checked_sub(lookup_cost)
            .ok_or_else(|| error(span, "attribute supplier lookup budget exhausted"))?;
        if let Some(value) = self.attribute_demands.merged.get(&target) {
            return Ok(value.clone());
        }
        self.required_steps = self
            .required_steps
            .checked_sub(indices.len() as u64)
            .ok_or_else(|| error(span, "attribute supplier traversal budget exhausted"))?;
        for &index in indices.iter() {
            self.evaluate_supplier(index)?;
        }

        let mut budget = comptime_runtime::Budget {
            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
            ..Default::default()
        };
        let before = budget.steps;
        let result = (|| {
            let values = indices.iter().map(|&i| {
                self.attribute_demands.values[i]
                    .as_ref()
                    .expect("completed supplier")
            });
            let selected = merge::merge_entries(values, 1024, &mut budget)?;
            Ok(self
                .freeze_merged_attributes(&selected, &mut budget)?
                .remove(&target)
                .map(Arc::new))
        })();
        self.required_steps -= (before - budget.steps) as u64;
        let value = result.map_err(|failure: MergeFailure| {
            let mut diagnostic = error(span, failure.message);
            diagnostic.source = failure.source;
            diagnostic
        })?;
        self.attribute_demands.merged.insert(target, value.clone());
        Ok(value)
    }
}
