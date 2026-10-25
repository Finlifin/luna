//! Evaluate metadata for inspection without authorizing attribute consumers.
use super::*;

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
    elaborate_with_attributes(package, package_identities, protocols, target, true)
        .map(|(_, attributes)| attributes)
}

impl Checker<'_, '_> {
    pub(super) fn evaluate_attributes(&mut self) -> Result<Vec<EvaluatedAttribute>, Error> {
        let mut output = Vec::new();
        for attribute in &self.package.definition_attributes {
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
                .ok_or_else(|| error(attribute.span, "attribute inspection budget exhausted"))?;
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
            self.signature_owner = Some(attribute.lexical_owner);
            self.context = self.definition_context(attribute.lexical_owner);
            self.locals.clear();
            self.result = None;
            let expected = attribute.configuration_key.is_none().then(|| p(P::Object));
            let (value, ty) = self.required_typed_expression(attribute.value, expected.as_ref())?;
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
    }
}
