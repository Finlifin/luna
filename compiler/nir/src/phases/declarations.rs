//! Immutable declaration data, paired with its source by the inspection owner.
//! All references into contracts and parameter frames are recovered by position.
use super::*;

pub(super) enum ParameterFrame {
    Function(usize),
    Quote(usize),
}
impl ParameterFrame {
    pub(super) fn parameters<'a>(&self, package: &'a Package) -> &'a [crate::program::LocalId] {
        match self {
            Self::Function(index) => &package.functions[*index].parameters,
            Self::Quote(index) => &package.quote_templates[*index].parameters,
        }
    }
}

pub(super) struct Declarations {
    pub(super) parameter_domains: BTreeMap<crate::DeclarationId, Term>,
    pub(super) trait_index: trait_index::Index,
    pub(super) functions: BTreeMap<crate::DeclarationId, (Phase, bool, bool)>,
    pub(super) parameter_frames: BTreeMap<crate::DeclarationId, ParameterFrame>,
    pub(super) quote_ids: BTreeSet<crate::DeclarationId>,
    pub(super) staged_quote_ids: BTreeSet<crate::DeclarationId>,
}
impl Declarations {
    pub(super) fn new(package: &Package, declaration_fuel: &mut usize) -> Result<Self, Error> {
        // Reserve membership and position tables before allocating them. Their
        // entries hold IDs/slots; parameter and contract payloads are metered by
        // the shared domain/frame validators below.
        let slots = package
            .functions
            .len()
            .saturating_mul(3)
            .saturating_add(package.quote_templates.len().saturating_mul(4))
            .saturating_add(package.runtime_function_identities.len().saturating_mul(3))
            .saturating_add(package.enums.len());
        *declaration_fuel = declaration_fuel.checked_sub(slots).ok_or_else(|| {
            Error::budget(
                SourceRange { start: 0, end: 0 },
                "phase declaration directory budget exhausted",
            )
        })?;
        let parameter_domains = parameters::domains(package, declaration_fuel)
            .map_err(parameters::Failure::into_diagnostic)?;
        for definition in &package.enums {
            definition
                .validate_encoding_in(package.target)
                .map_err(|message| Error {
                    kind: ErrorKind::Invalid,
                    source: SourceRange { start: 0, end: 0 },
                    message,
                })?;
        }
        let trait_index = trait_index::Index::new(package, declaration_fuel)
            .map_err(|error| Error::member(SourceRange { start: 0, end: 0 }, error))?;
        // Identity membership is queried for every retained callable. Index once:
        // scanning function vectors here makes snapshot admission quadratic.
        let body_ids: BTreeSet<_> = package.functions.iter().map(|f| f.declaration).collect();
        let runtime_ids: BTreeSet<_> = package
            .runtime_function_identities
            .iter()
            .map(|f| f.declaration)
            .collect();
        let mut quote_ids = BTreeSet::new();
        let mut staged_quote_ids = BTreeSet::new();
        for template in &package.quote_templates {
            if template.phase != Phase::Runtime
                || !quote_ids.insert(template.declaration)
                || body_ids.contains(&template.declaration)
                || runtime_ids.contains(&template.declaration)
            {
                return Err(Error {
                    kind: ErrorKind::Invalid,
                    source: template.body.source,
                    message: "invalid or duplicate quote template identity",
                });
            }
            let parameters = parameters::validate_frame(template, true, declaration_fuel)
                .map_err(parameters::Failure::into_diagnostic)?;
            if parameters
                .iter()
                .any(|parameter| parameter.sort != Sort::Runtime)
            {
                staged_quote_ids.insert(template.declaration);
            }
        }
        let mut identity_ids = std::collections::BTreeSet::new();
        for identity in &package.runtime_function_identities {
            if !matches!(
                identity.signature.callable_signature(),
                Term::Function { .. }
            ) || identity.signature.is_comptime_callable()
                || !identity_ids.insert(identity.declaration)
                || body_ids.contains(&identity.declaration)
            {
                return Err(Error {
                    kind: ErrorKind::Invalid,
                    source: SourceRange { start: 0, end: 0 },
                    message: "invalid or duplicate runtime function identity",
                });
            }
        }
        crate::normal_forms::validate_in(package, declaration_fuel).map_err(|error| Error {
            kind: match error {
                crate::normal_forms::ValidationError::Budget
                | crate::normal_forms::ValidationError::Newtype(
                    crate::newtypes::ValidationError::Budget,
                ) => ErrorKind::Budget,
                _ => ErrorKind::Invalid,
            },
            source: SourceRange { start: 0, end: 0 },
            message: error.message(),
        })?;
        for function in &package.functions {
            parameters::validate_frame(function, false, declaration_fuel)
                .map_err(parameters::Failure::into_diagnostic)?;
        }
        let functions: BTreeMap<_, _> = package
            .functions
            .iter()
            .chain(&package.quote_templates)
            .map(|f| {
                (
                    f.declaration,
                    (f.phase, !f.preconditions.is_empty(), f.is_unsafe),
                )
            })
            .chain(package.runtime_function_identities.iter().map(|f| {
                (
                    f.declaration,
                    (Phase::Runtime, false, f.signature.is_unsafe_callable()),
                )
            }))
            .collect();
        let parameter_frames = package
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| (function.declaration, ParameterFrame::Function(index)))
            .chain(
                package
                    .quote_templates
                    .iter()
                    .enumerate()
                    .map(|(index, function)| (function.declaration, ParameterFrame::Quote(index))),
            )
            .collect();
        Ok(Self {
            parameter_domains,
            trait_index,
            functions,
            parameter_frames,
            quote_ids,
            staged_quote_ids,
        })
    }
}
