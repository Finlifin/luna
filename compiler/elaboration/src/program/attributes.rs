//! Evaluate metadata once and apply supported phase properties before interfaces.
use super::*;
mod derivation;
mod effects;
mod layout;
mod merge;
mod metadata;
mod phases;
pub use derivation::{DefaultStrategyMarker, default_strategy_markers};
pub use effects::{EffectDirectPassing, direct_passing_effects};
pub use layout::{PackedStructLayout, packed_structs};
pub use merge::{MergeFailure, MergedTarget, SelectedValue, merge};
pub use metadata::{MetadataEntry, metadata_entries};
pub use phases::{MemberPhase, member_phases};

/// An independently materialized attribute in HIR application order. Configuration
/// entries retain their key; prefixes produce an Object. No merge has happened.
#[derive(Clone, Debug)]
pub struct EvaluatedAttribute {
    pub target: hir::AttributeTarget,
    pub lexical_owner: OwnerId,
    pub source: n::SourceRange,
    pub configuration_key: Option<String>,
    pub value: n::Expr,
}
impl std::fmt::Display for EvaluatedAttribute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(evaluated-attribute (target {}) (lexical-owner {}) (source {} {}) (configuration",
            self.target, self.lexical_owner, self.source.start, self.source.end
        )?;
        if let Some(key) = &self.configuration_key {
            write!(f, " {key:?}")?;
        }
        write!(f, ") (value {}))", self.value)
    }
}

/// Inspect completed metadata using ordinary elaboration and comptime evaluation.
/// This returns no executable package: layout, ABI and other attribute consumers
/// must still discharge their contracts before normal compilation can proceed.
/// Generic/runtime-dependent metadata and reflection cycles remain unsupported;
/// they are diagnosed rather than replaced by empty objects. Failure returns no
/// partial attribute list and does not modify the input HIR.
pub fn inspect(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
) -> Result<Vec<EvaluatedAttribute>, Error> {
    let mut invocation_steps =
        DEFAULT_PACKAGE_ELABORATION_BUDGET.saturating_mul((package_identities.len() as u64).max(1));
    elaborate_with_attributes(
        package,
        package_identities,
        protocols,
        target,
        true,
        &mut invocation_steps,
    )
    .map(|(_, attributes)| attributes)
}

impl Checker<'_, '_> {
    pub(super) fn prepare_phase_attributes(
        &mut self,
        inspect_only: bool,
    ) -> Result<Vec<EvaluatedAttribute>, Error> {
        if self.package.definition_attributes.is_empty()
            && !self.package.owners().any(|(_, info)| {
                matches!(&info.node.expect_item().kind, ItemKind::Trait(def) if def.comptime_only)
            })
        {
            self.attributes_prepared = true;
            return Ok(Vec::new());
        }
        let attributes = self.evaluate_attributes()?;
        let mut budget = comptime_runtime::Budget {
            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
            ..Default::default()
        };
        let before = budget.steps;
        let result = (|| {
            let selected = merge(&attributes, 1024, &mut budget)?;
            let phases = member_phases(self.package, &selected, &mut budget)?;
            let packed = packed_structs(self.package, &selected, &mut budget)?;
            let direct_passing = direct_passing_effects(self.package, &selected, &mut budget)?;
            let _metadata = metadata_entries(self.package, &selected, &mut budget)?;
            let _defaults = default_strategy_markers(self.package, &selected, &mut budget)?;
            for target in &selected {
                for (key, value) in target.iter() {
                    if !inspect_only
                        && !matches!(
                            key,
                            "comptime_phase" | "packed" | "direct_passing" | "metadata"
                        )
                    {
                        return Err(MergeFailure {
                            source: value.origin.source,
                            message: format!(
                                "elaboration is not implemented for definition attribute or modifier `{key}`"
                            ),
                        });
                    }
                }
            }
            let frozen = if inspect_only {
                BTreeMap::new()
            } else {
                self.freeze_merged_attributes(&selected, &mut budget)?
            };
            Ok((phases, packed, direct_passing, frozen))
        })();
        self.required_steps = self
            .required_steps
            .saturating_sub((before - budget.steps) as u64);
        /*
         * Raw inspection may expose Objects which are not legal merge inputs.
         * It returns no executable package; only a complete valid phase plan
         * influences its interface checks. Executable compilation never takes
         * this diagnostic-only path.
         */
        if inspect_only && result.is_err() {
            return Ok(attributes);
        }
        let (phases, packed, direct_passing, frozen) = result.map_err(|failure| {
            let mut diagnostic = error(Span::default(), failure.message);
            diagnostic.source = failure.source;
            diagnostic
        })?;
        self.member_phases = phases
            .into_iter()
            .map(|entry| {
                (
                    entry.owner,
                    match entry.phase {
                        hir::ImplementationPhase::Runtime => n::TraitPhase::Runtime,
                        hir::ImplementationPhase::Comptime => n::TraitPhase::Comptime,
                    },
                )
            })
            .collect();
        self.packed_structs = packed
            .into_iter()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.owner)
            .collect();
        self.direct_passing_effects = direct_passing
            .into_iter()
            .map(|entry| (entry.owner, entry.enabled))
            .collect();
        self.definition_attributes = frozen;
        self.attributes_prepared = true;
        Ok(attributes)
    }

    fn freeze_merged_attributes(
        &self,
        selected: &[MergedTarget<'_>],
        budget: &mut comptime_runtime::Budget,
    ) -> Result<BTreeMap<DeclarationId, Term>, MergeFailure> {
        let mut frozen_map = BTreeMap::new();
        for target in selected {
            let hir::AttributeTarget::Definition(owner) = target.target else {
                continue;
            };
            let mut entries = Vec::new();
            for (key, item) in target.iter() {
                budget
                    .consume(key.len().saturating_add(4))
                    .map_err(|e| MergeFailure {
                        source: item.origin.source,
                        message: e.to_string(),
                    })?;
                let mut term_fuel = budget.steps;
                nir::type_computations::charge_term(&item.value.ty, &mut term_fuel).map_err(
                    |e| MergeFailure {
                        source: item.origin.source,
                        message: e.to_string(),
                    },
                )?;
                budget.steps = term_fuel;
                let domain = item.value.ty.clone();
                let frozen = self
                    .observe_normal_input_in(item.value, 0, budget, &BTreeMap::new())
                    .map_err(|failure| MergeFailure {
                        source: item.origin.source,
                        message: format!(
                            "attribute `{key}` requires a freezeable comptime value: {failure}"
                        ),
                    })?;
                entries.push(nir::objects::entry(Some(key), domain, frozen));
            }
            if !entries.is_empty() {
                let id = declaration(self.package, &self.package_identities, owner);
                frozen_map.insert(id, nir::objects::value(entries));
            }
        }
        Ok(frozen_map)
    }

    pub(super) fn evaluate_attributes(&mut self) -> Result<Vec<EvaluatedAttribute>, Error> {
        self.evaluate_attributes_filtered(|_| true)
    }

    pub(super) fn evaluate_attributes_filtered(
        &mut self,
        mut filter: impl FnMut(&hir::DeferredDefinitionAttribute<'_>) -> bool,
    ) -> Result<Vec<EvaluatedAttribute>, Error> {
        let was_evaluating = self.evaluating_attributes;
        self.evaluating_attributes = true;
        let result = (|| {
            let mut output = Vec::new();
            for attribute in &self.package.definition_attributes {
                if !filter(attribute) {
                    continue;
                }
                self.required_steps = self
                    .required_steps
                    .checked_sub(
                        1u64.saturating_add(
                            attribute
                                .configuration_key
                                .as_ref()
                                .map_or(0, |key| key.len() as u64),
                        ),
                    )
                    .ok_or_else(|| {
                        error(attribute.span, "attribute inspection budget exhausted")
                    })?;
                let owner = match attribute.target {
                    hir::AttributeTarget::Definition(owner) => owner,
                    hir::AttributeTarget::Member(id) => id.owner,
                };
                if self.package.item(owner).is_none()
                    || self.package.item(attribute.lexical_owner).is_none()
                    || attribute.value.hir_id.owner != attribute.lexical_owner
                {
                    return Err(error(
                        attribute.span,
                        "invalid attribute target or lexical owner",
                    ));
                }
                if let hir::AttributeTarget::Member(id) = attribute.target {
                    if id.local_id.raw() == 0 || self.package.node(id).is_none() {
                        return Err(error(attribute.span, "missing attribute member target"));
                    }
                }
                // Module scope supplies lexical name resolution, not a callable or
                // type telescope. Required-call preparation must not ask a module
                // for a definition formation contract.
                self.signature_owner = (!matches!(
                    self.package
                        .item(attribute.lexical_owner)
                        .map(|item| &item.kind),
                    Some(ItemKind::Mod(_))
                ))
                .then_some(attribute.lexical_owner);
                self.context = self.definition_context(attribute.lexical_owner);
                self.locals.clear();
                self.control.result = None;
                let expected = attribute.configuration_key.is_none().then(|| p(P::Object));
                let (value, ty) =
                    self.required_typed_expression(attribute.value, expected.as_ref())?;
                let value = self.materialize_local_constant(value, ty, attribute.span)?;
                output.push(EvaluatedAttribute {
                    target: attribute.target,
                    lexical_owner: attribute.lexical_owner,
                    source: source(attribute.span),
                    configuration_key: attribute.configuration_key.clone(),
                    value,
                });
            }
            self.signature_owner = None;
            self.context = self.nominal_context();
            Ok(output)
        })();
        self.evaluating_attributes = was_evaluating;
        result
    }
}
