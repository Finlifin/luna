//! Immutable build inputs owned by source package identity.
//!
//! This is the driver-side query contract. Source `build` declarations and the
//! compilation cache have not yet been connected to it. No query consults the
//! process environment or substitutes the compiler's execution host as a target.
use crate::packages::SourcePackage;
use nir::{DeclarationId, QuotedDisplay, target::TargetData};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub const INPUT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub arch: String,
    pub os: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub arch: String,
    pub os: String,
    pub layout: TargetData,
    /// Absent and explicitly disabled features are distinct observations.
    pub features: BTreeMap<String, bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageBuild {
    pub identity: u64,
    pub mode: String,
    pub execution_host: Host,
    /// The default target belongs to the host pipeline, not a device projection.
    pub host_pipeline_target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Query {
    Mode,
    TargetArch,
    TargetOs,
    TargetPointerBits,
    TargetFeature(String),
    ExecutionHostArch,
    ExecutionHostOs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Symbol(String),
    PointerBits(u32),
    Feature(Option<bool>),
}

/// A precise dependency: unrelated build inputs do not invalidate this read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    owner: u64,
    query: Query,
    value: Value,
}
impl Observation {
    pub fn owner(&self) -> u64 {
        self.owner
    }
    pub fn query(&self) -> &Query {
        &self.query
    }
    pub fn value(&self) -> &Value {
        &self.value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Version(u32),
    DuplicatePackage(u64),
    DuplicateResolution(u32),
    UnknownPackage(u64),
    MissingPackage(u64),
    DuplicateInput(u64),
    EmptyName { owner: u64, field: &'static str },
}
impl std::error::Error for Error {}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(build-input-error ")?;
        match self {
            Self::Version(v) => write!(f, "(version {v} expected {INPUT_VERSION})")?,
            Self::DuplicatePackage(id) => write!(f, "(duplicate-package {id})")?,
            Self::DuplicateResolution(id) => write!(f, "(duplicate-resolution {id})")?,
            Self::UnknownPackage(id) => write!(f, "(unknown-package {id})")?,
            Self::MissingPackage(id) => write!(f, "(missing-package {id})")?,
            Self::DuplicateInput(id) => write!(f, "(duplicate-input {id})")?,
            Self::EmptyName { owner, field } => write!(
                f,
                "(empty-name (owner {owner}) (field {}))",
                QuotedDisplay(field)
            )?,
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    packages: BTreeMap<u64, PackageBuild>,
}
impl Snapshot {
    /// Reject incomplete or ambiguous inputs before publishing the snapshot.
    /// Source resolution IDs are checked separately from semantic identities.
    pub fn new(
        version: u32,
        sources: &[SourcePackage],
        inputs: Vec<PackageBuild>,
    ) -> Result<Self, Error> {
        if version != INPUT_VERSION {
            return Err(Error::Version(version));
        }
        let mut identities = BTreeSet::new();
        let mut resolutions = BTreeSet::new();
        for source in sources {
            if !identities.insert(source.identity) {
                return Err(Error::DuplicatePackage(source.identity));
            }
            if !resolutions.insert(source.resolution_id) {
                return Err(Error::DuplicateResolution(source.resolution_id));
            }
        }
        let mut packages = BTreeMap::new();
        for input in inputs {
            let owner = input.identity;
            if !identities.contains(&owner) {
                return Err(Error::UnknownPackage(owner));
            }
            for (field, value) in [
                ("mode", input.mode.as_str()),
                ("execution-host.arch", input.execution_host.arch.as_str()),
                ("execution-host.os", input.execution_host.os.as_str()),
                ("target.arch", input.host_pipeline_target.arch.as_str()),
                ("target.os", input.host_pipeline_target.os.as_str()),
            ] {
                if value.is_empty() {
                    return Err(Error::EmptyName { owner, field });
                }
            }
            if input.host_pipeline_target.features.contains_key("") {
                return Err(Error::EmptyName {
                    owner,
                    field: "target.feature",
                });
            }
            if packages.insert(owner, input).is_some() {
                return Err(Error::DuplicateInput(owner));
            }
        }
        for owner in identities {
            if !packages.contains_key(&owner) {
                return Err(Error::MissingPackage(owner));
            }
        }
        Ok(Self { packages })
    }

    /// `definition` must be the originating declaration, even after inlining or
    /// template reuse. Caller identity and resolution aliases do not select it.
    pub fn read(&self, definition: DeclarationId, query: Query) -> Result<Observation, Error> {
        let owner = definition.package;
        let input = self
            .packages
            .get(&owner)
            .ok_or(Error::MissingPackage(owner))?;
        let target = &input.host_pipeline_target;
        let value = match &query {
            Query::Mode => Value::Symbol(input.mode.clone()),
            Query::TargetArch => Value::Symbol(target.arch.clone()),
            Query::TargetOs => Value::Symbol(target.os.clone()),
            Query::TargetPointerBits => Value::PointerBits(target.layout.pointer_bits()),
            Query::TargetFeature(name) => Value::Feature(target.features.get(name).copied()),
            Query::ExecutionHostArch => Value::Symbol(input.execution_host.arch.clone()),
            Query::ExecutionHostOs => Value::Symbol(input.execution_host.os.clone()),
        };
        Ok(Observation {
            owner,
            query,
            value,
        })
    }

    /// Missing owners invalidate evidence; they never inherit another package.
    pub fn unchanged(&self, observation: &Observation) -> bool {
        self.read(
            DeclarationId {
                package: observation.owner,
                local: 0,
            },
            observation.query.clone(),
        )
        .is_ok_and(|current| current.value == observation.value)
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mode => f.write_str("mode"),
            Self::TargetArch => f.write_str("target-arch"),
            Self::TargetOs => f.write_str("target-os"),
            Self::TargetPointerBits => f.write_str("target-pointer-bits"),
            Self::TargetFeature(name) => write!(f, "(target-feature {})", QuotedDisplay(name)),
            Self::ExecutionHostArch => f.write_str("execution-host-arch"),
            Self::ExecutionHostOs => f.write_str("execution-host-os"),
        }
    }
}
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symbol(value) => write!(f, "(symbol {})", QuotedDisplay(value)),
            Self::PointerBits(bits) => write!(f, "(pointer-bits {bits})"),
            Self::Feature(Some(value)) => write!(f, "(feature (present {value}))"),
            Self::Feature(None) => f.write_str("(feature absent)"),
        }
    }
}
impl fmt::Display for Observation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(build-read (owner {}) (query {}) (value {}))",
            self.owner, self.query, self.value
        )
    }
}
impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(build-snapshot (version {INPUT_VERSION})")?;
        for (owner, input) in &self.packages {
            let host = &input.execution_host;
            let target = &input.host_pipeline_target;
            write!(
                f,
                " (package {owner} (mode {}) (execution-host (arch {}) (os {})) (host-pipeline-target (arch {}) (os {}) {} (features",
                QuotedDisplay(&input.mode),
                QuotedDisplay(&host.arch),
                QuotedDisplay(&host.os),
                QuotedDisplay(&target.arch),
                QuotedDisplay(&target.os),
                target.layout
            )?;
            for (name, enabled) in &target.features {
                write!(f, " ({} {enabled})", QuotedDisplay(name))?;
            }
            f.write_str(")))")?;
        }
        f.write_str(")")
    }
}
