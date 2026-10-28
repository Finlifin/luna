//! Immutable package metadata input and observations for the build driver.
//!
//! This boundary checks closed descriptor shape, not semantic value admission.
//! Consuming a descriptor as a Flurry value still requires the provider's type
//! world. No process environment, filesystem lookup or global package-name search
//! participates in resolution. `Snapshot::configure` installs the immutable
//! input for origin-bound source elaboration.
use crate::packages::SourcePackage;
use nir::{DeclarationId, QuotedDisplay as Q, Sort, Term};
use std::{collections::BTreeMap, fmt, sync::Arc};

pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    ty: Term,
    value: Term,
}
impl Entry {
    /// Retain closed immutable terms without unfolding normal-form heads.
    /// Shape validity does not prove that `value` inhabits `ty`.
    pub fn new(ty: Term, value: Term, fuel: &mut usize) -> Result<Self, Error> {
        for term in [&ty, &value] {
            // Prepay shape, dependency and rigid-parameter traversals (including
            // the existing rigid collector's temporary term copies).
            for _ in 0..4 {
                nir::type_computations::charge_term(term, fuel).map_err(|_| Error::Budget)?;
            }
            if term.check_scope(&[]).is_err()
                || !term.runtime_dependencies().is_empty()
                || !term
                    .rigid_parameters()
                    .map_err(|_| Error::OpenEntry)?
                    .is_empty()
            {
                return Err(Error::OpenEntry);
            }
        }
        if ty.sort() != Sort::Type || value.sort() == Sort::Runtime {
            return Err(Error::OpenEntry);
        }
        Ok(Self { ty, value })
    }
    /// Decode and admit on each use in the supplied provider world. The result
    /// is not a persistent certificate and grants no resource copying permission.
    pub fn admit(
        &self,
        interpreter: &mut comptime_interpretor::Interpreter<'_>,
    ) -> Result<comptime_interpretor::Value, comptime_interpretor::Error> {
        interpreter.decode_immutable_input(&self.value, &self.ty)
    }
    pub fn ty(&self) -> &Term {
        &self.ty
    }
    pub fn value(&self) -> &Term {
        &self.value
    }
}

#[derive(Clone, Debug)]
pub struct Input {
    pub identity: u64,
    /// Vec preserves duplicate names for validation before canonicalization.
    pub fields: Vec<(String, Entry)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provider {
    Local,
    Dependency(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    owner: u64,
    provider: Provider,
    resolved: u64,
    field: String,
    value: Option<Arc<Entry>>,
}
impl Observation {
    pub fn owner(&self) -> u64 {
        self.owner
    }
    pub fn provider(&self) -> u64 {
        self.resolved
    }
    pub fn value(&self) -> Option<&Entry> {
        self.value.as_deref()
    }

    /// Preserve absent fields without manufacturing a typed null. Admission of
    /// a present descriptor uses the caller-supplied provider compilation world.
    pub fn admit(
        &self,
        interpreter: &mut comptime_interpretor::Interpreter<'_>,
    ) -> Result<Option<comptime_interpretor::Value>, AdmissionError> {
        self.value
            .as_ref()
            .map(|entry| entry.admit(interpreter))
            .transpose()
            .map_err(|cause| AdmissionError {
                owner: self.owner,
                provider: self.resolved,
                field: self.field.clone(),
                cause,
            })
    }
}

#[derive(Debug)]
pub struct AdmissionError {
    pub owner: u64,
    pub provider: u64,
    pub field: String,
    pub cause: comptime_interpretor::Error,
}
impl std::error::Error for AdmissionError {}
impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(metadata-admission-error (owner {}) (provider {}) (field {}) (cause {}))",
            self.owner,
            self.provider,
            Q(&self.field),
            Q(&self.cause.to_string())
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Version(u32),
    DuplicateIdentity(u64),
    Graph(String),
    MissingOwner(u64),
    MissingDependency { owner: u64, alias: String },
    MissingInput(u64),
    UnknownInput(u64),
    DuplicateInput(u64),
    DuplicateField { owner: u64, field: String },
    EmptyField(u64),
    OpenEntry,
    Budget,
}
impl std::error::Error for Error {}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(metadata-error ")?;
        match self {
            Self::Version(value) => write!(f, "(version {value} expected {VERSION})")?,
            Self::DuplicateIdentity(id) => write!(f, "(duplicate-identity {id})")?,
            Self::Graph(message) => write!(f, "(graph {})", Q(message))?,
            Self::MissingOwner(id) => write!(f, "(missing-owner {id})")?,
            Self::MissingDependency { owner, alias } => write!(
                f,
                "(missing-dependency (owner {owner}) (alias {}))",
                Q(alias)
            )?,
            Self::MissingInput(id) => write!(f, "(missing-input {id})")?,
            Self::UnknownInput(id) => write!(f, "(unknown-input {id})")?,
            Self::DuplicateInput(id) => write!(f, "(duplicate-input {id})")?,
            Self::DuplicateField { owner, field } => {
                write!(f, "(duplicate-field (owner {owner}) (name {}))", Q(field))?
            }
            Self::EmptyField(id) => write!(f, "(empty-field {id})")?,
            Self::OpenEntry => f.write_str("open-entry")?,
            Self::Budget => f.write_str("budget-exhausted")?,
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    aliases: BTreeMap<u64, BTreeMap<String, u64>>,
    inputs: BTreeMap<u64, BTreeMap<String, Arc<Entry>>>,
}
impl Snapshot {
    pub fn new(
        version: u32,
        sources: &[SourcePackage],
        dependencies: &[resolve::DependencyEdge],
        inputs: Vec<Input>,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        if version != VERSION {
            return Err(Error::Version(version));
        }
        charge(
            fuel,
            sources
                .len()
                .saturating_add(dependencies.len())
                .saturating_add(inputs.len()),
        )?;
        for edge in dependencies {
            charge(fuel, edge.alias.len())?;
        }
        resolve::validate_package_dependencies(
            sources.iter().map(|source| source.resolution_id),
            dependencies,
            None,
        )
        .map_err(|error| Error::Graph(format!("{error:?}")))?;
        let mut resolutions = BTreeMap::new();
        let mut aliases = BTreeMap::new();
        for source in sources {
            resolutions.insert(source.resolution_id, source.identity);
            if aliases.insert(source.identity, BTreeMap::new()).is_some() {
                return Err(Error::DuplicateIdentity(source.identity));
            }
        }
        for edge in dependencies {
            aliases
                .get_mut(&resolutions[&edge.requester])
                .unwrap()
                .insert(edge.alias.clone(), resolutions[&edge.provider]);
        }
        let mut installed = BTreeMap::new();
        for input in inputs {
            let owner = input.identity;
            if !aliases.contains_key(&owner) {
                return Err(Error::UnknownInput(owner));
            }
            if installed.contains_key(&owner) {
                return Err(Error::DuplicateInput(owner));
            }
            charge(fuel, input.fields.len())?;
            let mut fields = BTreeMap::new();
            for (name, value) in input.fields {
                charge(fuel, name.len())?;
                if name.is_empty() {
                    return Err(Error::EmptyField(owner));
                }
                if fields.insert(name.clone(), Arc::new(value)).is_some() {
                    return Err(Error::DuplicateField { owner, field: name });
                }
            }
            installed.insert(owner, fields);
        }
        Ok(Self {
            aliases,
            inputs: installed,
        })
    }

    /// Install this snapshot for origin-bound source metadata calls.
    pub fn configure(&self, protocols: &mut elaboration::program::Protocols) {
        protocols.build_metadata = Some(Arc::new(elaboration::program::build_context::Metadata {
            aliases: self.aliases.clone(),
            inputs: self
                .inputs
                .iter()
                .map(|(owner, fields)| {
                    (
                        *owner,
                        fields
                            .iter()
                            .map(|(name, entry)| {
                                (name.clone(), (entry.ty.clone(), entry.value.clone()))
                            })
                            .collect(),
                    )
                })
                .collect(),
        }));
    }

    /// The origin is the definition's semantic package, not its resolution ID.
    /// Missing input differs from an explicitly supplied empty metadata object.
    pub fn read(
        &self,
        origin: DeclarationId,
        provider: Provider,
        field: &str,
    ) -> Result<Observation, Error> {
        let owner = origin.package;
        let aliases = self.aliases.get(&owner).ok_or(Error::MissingOwner(owner))?;
        let resolved = match &provider {
            Provider::Local => owner,
            Provider::Dependency(alias) => {
                *aliases.get(alias).ok_or_else(|| Error::MissingDependency {
                    owner,
                    alias: alias.clone(),
                })?
            }
        };
        let input = self
            .inputs
            .get(&resolved)
            .ok_or(Error::MissingInput(resolved))?;
        Ok(Observation {
            owner,
            provider,
            resolved,
            field: field.into(),
            value: input.get(field).cloned(),
        })
    }

    /// Conservative structural equality is sufficient for cache reuse. It does
    /// not assert that different representations denote different Flurry types.
    pub fn unchanged(&self, observation: &Observation) -> bool {
        self.read(
            DeclarationId {
                package: observation.owner,
                local: 0,
            },
            observation.provider.clone(),
            &observation.field,
        )
        .is_ok_and(|current| {
            current.resolved == observation.resolved && current.value == observation.value
        })
    }
}
fn charge(fuel: &mut usize, amount: usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(amount).ok_or(Error::Budget)?;
    Ok(())
}
impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(metadata-entry (type {}) (value {}))",
            self.ty, self.value
        )
    }
}
impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => f.write_str("local"),
            Self::Dependency(alias) => write!(f, "(dependency {})", Q(alias)),
        }
    }
}
impl fmt::Display for Observation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(metadata-read (owner {}) (provider {} {}) (field {}) (value",
            self.owner,
            self.provider,
            self.resolved,
            Q(&self.field)
        )?;
        match &self.value {
            Some(value) => write!(f, " {value}")?,
            None => f.write_str(" absent")?,
        };
        f.write_str("))")
    }
}
impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(metadata-snapshot (version {VERSION}) (aliases")?;
        for (owner, aliases) in &self.aliases {
            write!(f, " (package {owner}")?;
            for (name, provider) in aliases {
                write!(f, " ({} {provider})", Q(name))?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (inputs")?;
        for (owner, fields) in &self.inputs {
            write!(f, " (package {owner}")?;
            for (name, value) in fields {
                write!(f, " ({} {value})", Q(name))?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}
