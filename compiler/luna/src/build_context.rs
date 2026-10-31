//! Immutable build inputs owned by source package identity.
//!
//! This is the driver-side query contract. Explicit mode bindings feed source
//! elaboration; configure also installs source target snapshots. Precise
//! compilation-cache observations are not yet connected. No query consults the process environment or substitutes the
//! compiler's execution host as a target.
use crate::packages::SourcePackage;
use nir::{DeclarationId, QuotedDisplay, target::TargetData};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub mod metadata;

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

    /// Install only this snapshot's explicit mode values for source elaboration.
    /// This replaces previous values; missing packages must not inherit stale data.
    pub fn configure_modes(&self, protocols: &mut elaboration::program::Protocols) {
        protocols.build_modes = self
            .packages
            .iter()
            .map(|(owner, input)| (*owner, input.mode.clone()))
            .collect();
    }

    /// Replace source-visible modes and host pipeline target snapshots together.
    pub fn configure(&self, protocols: &mut elaboration::program::Protocols) {
        self.configure_modes(protocols);
        protocols.build_targets = self
            .packages
            .iter()
            .map(|(owner, input)| {
                let target = &input.host_pipeline_target;
                (
                    *owner,
                    elaboration::program::build_context::Target {
                        arch: target.arch.clone(),
                        os: target.os.clone(),
                        layout: target.layout,
                        features: target.features.clone(),
                    },
                )
            })
            .collect();
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

/// Process-boundary mode inputs. Primary applies only to the source entry package.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeOptions {
    pub primary: Option<String>,
    pub packages: BTreeMap<u64, String>,
}
impl ModeOptions {
    pub fn validate_stage(&self, emit: crate::cli::Emit) -> Result<(), String> {
        if !self.is_empty() && emit != crate::cli::Emit::Nir {
            return Err("build mode inputs require NIR elaboration or execution".into());
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.primary.is_none() && self.packages.is_empty()
    }
    pub fn set_primary(&mut self, value: String) -> Result<(), String> {
        if value.is_empty() {
            return Err("--build-mode requires a nonempty mode".into());
        }
        if self.primary.is_some() {
            return Err("--build-mode supplied more than once".into());
        }
        self.primary = Some(value);
        Ok(())
    }
    pub fn add_package(&mut self, value: &str) -> Result<(), String> {
        let (id, mode) = value
            .split_once('=')
            .ok_or("--package-build-mode requires identity=mode")?;
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) || mode.is_empty() {
            return Err(
                "--package-build-mode requires an unsigned identity and nonempty mode".into(),
            );
        }
        let id = id
            .parse::<u64>()
            .map_err(|_| "build mode package identity is out of range")?;
        if self.packages.contains_key(&id) {
            return Err(format!("duplicate build mode for package {id}"));
        }
        self.packages.insert(id, mode.into());
        Ok(())
    }
    pub fn bind(
        &self,
        owners: impl IntoIterator<Item = u64>,
        primary: u64,
        base: &elaboration::program::Protocols,
    ) -> Result<elaboration::program::Protocols, String> {
        let owners: BTreeSet<_> = owners.into_iter().collect();
        let mut selected = self.packages.clone();
        if let Some(mode) = &self.primary {
            if selected.insert(primary, mode.clone()).is_some() {
                return Err(format!(
                    "primary build mode conflicts with explicit package {primary}"
                ));
            }
        }
        let mut protocols = base.clone();
        for (owner, mode) in selected {
            if !owners.contains(&owner) {
                return Err(format!(
                    "build mode names unknown semantic package identity {owner}"
                ));
            }
            if mode.is_empty() {
                return Err(format!("empty build mode for package {owner}"));
            }
            if protocols
                .build_modes
                .get(&owner)
                .is_some_and(|old| old != &mode)
            {
                return Err(format!(
                    "build mode conflicts with driver snapshot for package {owner}"
                ));
            }
            protocols.build_modes.insert(owner, mode);
        }
        Ok(protocols)
    }
}

/// A target and its explicit feature overrides form one complete input.
/// Features may precede the target flag, but cannot exist without that target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TargetSelection {
    target: Option<elaboration::program::build_context::Target>,
    features: BTreeMap<String, bool>,
}
impl TargetSelection {
    fn complete(&self) -> Result<Option<elaboration::program::build_context::Target>, String> {
        match &self.target {
            Some(target) => {
                let mut target = target.clone();
                target.features = self.features.clone();
                Ok(Some(target))
            }
            None if self.features.is_empty() => Ok(None),
            None => Err("build features require an explicit target for the same package".into()),
        }
    }
}

/// Explicit CLI target inputs, keyed by semantic package identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetOptions {
    primary: TargetSelection,
    packages: BTreeMap<u64, TargetSelection>,
}
impl TargetOptions {
    fn selection(&mut self, owner: Option<u64>) -> &mut TargetSelection {
        match owner {
            Some(owner) => self.packages.entry(owner).or_default(),
            None => &mut self.primary,
        }
    }

    pub fn package_identity(value: &str) -> Result<u64, String> {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err("build target requires an unsigned semantic package identity".into());
        }
        value
            .parse()
            .map_err(|_| "build target package identity is out of range".into())
    }

    pub fn set_target(
        &mut self,
        owner: Option<u64>,
        arch: String,
        os: String,
        bits: &str,
    ) -> Result<(), String> {
        if arch.is_empty() || os.is_empty() {
            return Err("build target architecture and OS must be nonempty".into());
        }
        if bits.is_empty() || !bits.bytes().all(|b| b.is_ascii_digit()) {
            return Err("build target pointer width must be an unsigned decimal integer".into());
        }
        let bits = bits
            .parse::<u32>()
            .map_err(|_| "build target pointer width is out of range")?;
        let layout = TargetData::new(bits)?;
        let selected = self.selection(owner);
        if selected.target.is_some() {
            return Err("duplicate build target for the same package".into());
        }
        selected.target = Some(elaboration::program::build_context::Target {
            arch,
            os,
            layout,
            features: BTreeMap::new(),
        });
        Ok(())
    }

    pub fn add_feature(&mut self, owner: Option<u64>, feature: &str) -> Result<(), String> {
        let (name, value) = feature
            .split_once('=')
            .ok_or("build feature requires name=true|false")?;
        if name.is_empty() {
            return Err("build feature name must be nonempty".into());
        }
        let enabled = match value {
            "true" => true,
            "false" => false,
            _ => return Err("build feature requires name=true|false".into()),
        };
        let selected = self.selection(owner);
        if selected.features.contains_key(name) {
            return Err(format!("duplicate build feature {name}"));
        }
        selected.features.insert(name.into(), enabled);
        Ok(())
    }

    pub fn validate_stage(&self, emit: crate::cli::Emit) -> Result<(), String> {
        if self.primary.target.is_none()
            && self.primary.features.is_empty()
            && self.packages.is_empty()
        {
            return Ok(());
        }
        if emit != crate::cli::Emit::Nir {
            return Err("build target inputs require NIR elaboration or execution".into());
        }
        self.primary.complete()?;
        for selected in self.packages.values() {
            selected.complete()?;
        }
        Ok(())
    }

    /// Target input supplies semantic pointer width, but does not retarget LLVM.
    /// Native actions provide their actual layout and must agree with this input.
    pub fn resolve_layout(
        &self,
        primary: u64,
        existing: Option<TargetData>,
    ) -> Result<Option<TargetData>, String> {
        let direct = self.primary.complete()?;
        let explicit = self
            .packages
            .get(&primary)
            .map(TargetSelection::complete)
            .transpose()?
            .flatten();
        if direct.is_some() && explicit.is_some() {
            return Err(format!(
                "primary build target conflicts with explicit package {primary}"
            ));
        }
        let configured = direct.or(explicit).map(|target| target.layout);
        if matches!((existing, configured), (Some(a), Some(b)) if a != b) {
            return Err("build target layout conflicts with the compilation target".into());
        }
        Ok(existing.or(configured))
    }

    pub fn bind(
        &self,
        owners: impl IntoIterator<Item = u64>,
        primary: u64,
        base: &elaboration::program::Protocols,
    ) -> Result<elaboration::program::Protocols, String> {
        let owners: BTreeSet<_> = owners.into_iter().collect();
        let mut selected = BTreeMap::new();
        for (owner, input) in &self.packages {
            if !owners.contains(owner) {
                return Err(format!(
                    "build target names unknown semantic package identity {owner}"
                ));
            }
            if let Some(target) = input.complete()? {
                selected.insert(*owner, target);
            }
        }
        if let Some(target) = self.primary.complete()? {
            if !owners.contains(&primary) {
                return Err(format!(
                    "build target names unknown semantic package identity {primary}"
                ));
            }
            if selected.insert(primary, target).is_some() {
                return Err(format!(
                    "primary build target conflicts with explicit package {primary}"
                ));
            }
        }
        let mut protocols = base.clone();
        for (owner, target) in selected {
            if protocols
                .build_targets
                .get(&owner)
                .is_some_and(|old| old != &target)
            {
                return Err(format!(
                    "build target conflicts with driver snapshot for package {owner}"
                ));
            }
            protocols.build_targets.insert(owner, target);
        }
        Ok(protocols)
    }
}
