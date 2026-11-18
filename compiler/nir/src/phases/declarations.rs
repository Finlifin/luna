//! Immutable declaration data, paired with its source by the inspection owner.
//! All references into contracts and parameter frames are recovered by position.
use super::*;

pub(super) struct Declarations {
    pub(super) aliases: BTreeMap<crate::DeclarationId, crate::type_aliases::AdmittedContract>,
    pub(super) parameter_domains: std::sync::Arc<BTreeMap<crate::DeclarationId, Term>>,
    pub(super) trait_index: trait_index::Index,
    pub(super) callables: std::sync::Arc<crate::callable_source::Positions>,
}
impl Declarations {
    pub(super) fn new(package: &Package, declaration_fuel: &mut usize) -> Result<Self, Error> {
        // Reserve membership and position tables before allocating them. Their
        // entries hold IDs/slots; parameter and contract payloads are metered by
        // the shared domain/frame validators below.
        *declaration_fuel = declaration_fuel
            .checked_sub(package.enums.len())
            .ok_or_else(|| {
                Error::budget(
                    SourceRange { start: 0, end: 0 },
                    "phase declaration directory budget exhausted",
                )
            })?;
        let parameter_domains = parameters::domains(package, declaration_fuel)
            .map_err(parameters::Failure::into_diagnostic)?;
        let aliases = crate::type_aliases::admit_in(&package.alias_contracts, declaration_fuel)
            .map_err(|failure| {
                let source = SourceRange { start: 0, end: 0 };
                match failure {
                    crate::type_aliases::ValidationError::Budget => {
                        Error::budget(source, failure.message())
                    }
                    crate::type_aliases::ValidationError::Invalid(message) => {
                        Error::invalid(source, message)
                    }
                }
            })?;
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
        let callables = crate::callable_source::Positions::build(package, declaration_fuel)?;
        Ok(Self {
            aliases,
            parameter_domains: std::sync::Arc::new(parameter_domains),
            trait_index,
            callables,
        })
    }
}
