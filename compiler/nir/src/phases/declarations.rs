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
        let package_fallback = super::package_source(package);
        *declaration_fuel = declaration_fuel
            .checked_sub(package.enums.len())
            .ok_or_else(|| {
                Error::budget(
                    package_fallback,
                    "phase declaration directory budget exhausted",
                )
            })?;
        let parameter_domains = parameters::domains(package, declaration_fuel)
            .map_err(parameters::Failure::into_diagnostic)?;
        let aliases = crate::type_aliases::admit_in(&package.alias_contracts, declaration_fuel)
            .map_err(|failure| match failure {
                crate::type_aliases::ValidationError::Budget => {
                    Error::budget(package_fallback, failure.message())
                }
                crate::type_aliases::ValidationError::Invalid(message) => {
                    Error::invalid(package_fallback, message)
                }
            })?;
        for definition in &package.enums {
            definition
                .validate_encoding_in(package.target)
                .map_err(|message| Error {
                    kind: ErrorKind::Invalid,
                    source: package_fallback,
                    message,
                })?;
        }
        let trait_index = trait_index::Index::new(package, declaration_fuel)
            .map_err(|error| Error::member(error.source().unwrap_or(package_fallback), error))?;
        crate::normal_forms::validate_in(package, declaration_fuel).map_err(|error| Error {
            kind: match error {
                crate::normal_forms::ValidationError::Budget
                | crate::normal_forms::ValidationError::Newtype(
                    crate::newtypes::ValidationError::Budget,
                ) => ErrorKind::Budget,
                _ => ErrorKind::Invalid,
            },
            source: super::normal_form_source(package),
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

/// A phase check may inspect an active declaration. The executable source
/// directory and the package evidence it belongs to remain unchanged.
pub(super) struct Interfaces<'a> {
    pub(super) source: crate::callable_source::Callables<'a>,
    imports: Option<&'a crate::callable_source::RegionDeclarations>,
}

impl<'a> Interfaces<'a> {
    pub(super) fn new(source: crate::callable_source::Callables<'a>) -> Self {
        Self {
            source,
            imports: None,
        }
    }

    pub(super) fn import_declarations(
        &mut self,
        declarations: &'a crate::callable_source::RegionDeclarations,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        for (id, declaration) in declarations {
            *fuel = fuel.checked_sub(1).ok_or_else(|| {
                Error::budget(declaration.source, "region declaration budget exhausted")
            })?;
            if *id != declaration.declaration || self.phase(id).is_some() {
                return Err(Error::invalid(
                    declaration.source,
                    "invalid or duplicate region declaration identity",
                ));
            }
            crate::callable_source::validate_frame(
                crate::contracts::CallableProgram::declaration(declaration),
                false,
                fuel,
            )?;
        }
        self.imports = Some(declarations);
        Ok(())
    }

    pub(super) fn method_definition(
        &self,
        id: &crate::DeclarationId,
    ) -> Option<&'a crate::program::FunctionDeclaration> {
        self.source
            .method_definition(id)
            .or_else(|| self.imports?.get(id))
    }

    pub(super) fn phase(&self, id: &crate::DeclarationId) -> Option<(Phase, bool, bool)> {
        if let Some(f) = self.method_definition(id) {
            Some((f.phase, !f.preconditions.is_empty(), f.is_unsafe))
        } else {
            self.source
                .runtime_identity(id)
                .map(|f| (Phase::Runtime, false, f.signature.is_unsafe_callable()))
        }
    }
}
