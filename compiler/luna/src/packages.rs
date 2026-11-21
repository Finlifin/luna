//! Explicit source-package inputs for the driver. Dependency discovery and build
//! identities belong to the build tool; this API does not infer them from paths.
use crate::cli::Emit;
use crate::frontend::{Artifact, discover_sources_excluding};
use crate::package_cache::{
    ArtifactKind, CachedParsedFile, CachedParsedPackage, DiskPackageCache, IncrementalConfig,
    PackageSnapshotKey, Sha256, UpstreamKeyInputs, VerifiedUpstreamCertificate,
    compute_parsed_package_key, compute_upstream_key_with_jobs, decode_parsed_package,
    decode_verified_upstream, encode_parsed_package, encode_selectors_tag,
    encode_verified_upstream, global_parsed_cache, global_verified_cache, parallel_map_ordered,
    partition_closed_upstream,
};
use diagnostic::FlurryError;
use interface::{CompilerConfig, CompilerInstance, Session};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePackage {
    pub resolution_id: u32,
    pub identity: u64,
    pub name: String,
    pub entry: PathBuf,
}

/// Inputs observed by the frontend, not a cache key or a filesystem transaction.
#[derive(Debug, Default)]
pub struct CompilationInputs {
    pub sources: Vec<SourceInput>,
    /// Absolute directories whose entries participate in source discovery.
    /// Revalidate membership as well as file contents before reusing a result.
    pub directories: Vec<(u32, PathBuf)>,
    pub discovery: Vec<SourceDiscovery>,
}

#[derive(Debug)]
pub struct SourceDiscovery {
    pub resolution_id: u32,
    pub root: PathBuf,
    pub canonical_root: PathBuf,
    pub exclusions: Vec<PathBuf>,
    /// Relative paths returned by the same scan used to compile this package.
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DiscoveryChange {
    pub resolution_id: u32,
    pub added_files: Vec<PathBuf>,
    pub removed_files: Vec<PathBuf>,
    pub added_directories: Vec<PathBuf>,
    pub removed_directories: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct SourceInput {
    pub resolution_id: u32,
    pub identity: u64,
    pub path: PathBuf,
    /// The normalized text actually held by the compiler's source map.
    pub text: Arc<str>,
    /// Exact UTF-8 text returned by the file loader before normalization.
    pub raw_text: Arc<str>,
}

/// Compile an explicit package set and retain the source-map inputs used by
/// that same invocation. Protocol selection and package identities remain
/// explicit request inputs and must also participate in a build cache key.
pub fn compile_with_inputs(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<(Artifact, CompilationInputs), String> {
    compile_with_target_and_inputs(
        packages,
        dependencies,
        primary,
        builtin,
        protocols,
        selectors,
        None,
    )
}

/// Compile an explicit package set with explicit target data and retain
/// observed source-map inputs.
pub fn compile_with_target_and_inputs(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    target: Option<nir::target::TargetData>,
) -> Result<(Artifact, CompilationInputs), String> {
    let mut inputs = CompilationInputs::default();
    let (artifact, _) = compile_observed_for_target(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        selectors,
        Some(&mut inputs),
        target,
    )?;
    Ok((artifact, inputs))
}

#[derive(Clone)]
struct ObservedLoader {
    root: PathBuf,
    preloaded: Arc<HashMap<PathBuf, Arc<str>>>,
    text: Arc<std::sync::Mutex<BTreeMap<PathBuf, Arc<str>>>>,
}

impl rustc_span::source_map::FileLoader for ObservedLoader {
    fn file_exists(&self, path: &std::path::Path) -> bool {
        self.preloaded.contains_key(&self.root.join(path)) || path.exists()
    }

    fn read_file(&self, path: &std::path::Path) -> std::io::Result<String> {
        let key = self.root.join(path);
        let mut text = self
            .text
            .lock()
            .map_err(|_| std::io::Error::other("source observation lock poisoned"))?;
        // SourceMap retains the first source for a filename. Match that rule
        // for raw observations even if an input is requested more than once.
        if let Some(value) = text.get(&key) {
            return Ok(value.to_string());
        }
        if let Some(preloaded) = self.preloaded.get(&key) {
            text.insert(key, Arc::clone(preloaded));
            return Ok(preloaded.to_string());
        }
        let value = std::fs::read_to_string(path)?;
        text.insert(key, Arc::from(value.as_str()));
        Ok(value)
    }
}

impl ObservedLoader {
    fn get(&self, path: &std::path::Path) -> Result<Arc<str>, String> {
        self.text
            .lock()
            .map_err(|_| "source observation lock poisoned")?
            .get(&self.root.join(path))
            .cloned()
            .ok_or_else(|| "missing loaded source observation".into())
    }
}

pub fn compile(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
) -> Result<Artifact, String> {
    compile_with_builtin(packages, dependencies, primary, None)
}

/// The caller designates a supplied source package as builtin. No package is
/// chosen by an incidental source name or automatically read from the host.
pub fn compile_with_builtin(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
) -> Result<Artifact, String> {
    compile_stage(packages, dependencies, primary, builtin, Emit::Nir, None)
}

pub(crate) fn compile_stage(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
) -> Result<Artifact, String> {
    compile_stage_with_protocols(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        &Default::default(),
        None,
    )
}

/// Explicit package-set entry with driver-bound language protocols.
pub fn compile_with_protocols(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
) -> Result<Artifact, String> {
    compile_stage_with_protocols(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        None,
    )
}

/// Resolve an explicitly designated source protocol in the same compilation.
/// Qualify with the supplied source package name when trait names collide.
pub fn compile_with_splice(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    selector: &str,
) -> Result<Artifact, String> {
    compile_stage_with_protocols(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        &Default::default(),
        Some(selector),
    )
}

pub(crate) fn compile_stage_with_protocols(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    splice_selector: Option<&str>,
) -> Result<Artifact, String> {
    compile_stage_with_selectors(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        crate::protocols::type_constructors::Selectors {
            splice: splice_selector,
            ..Default::default()
        },
    )
}

/// Bind driver-selected source protocols after resolution in this compilation.
/// Selectors are qualified by the supplied package names, not dependency aliases.
pub fn compile_with_source_protocols(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<Artifact, String> {
    compile_with_source_protocols_for_target(
        packages,
        dependencies,
        primary,
        builtin,
        protocols,
        selectors,
        None,
    )
}

/// Bind driver-selected source protocols after resolution with explicit target data.
pub fn compile_with_source_protocols_for_target(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    target: Option<nir::target::TargetData>,
) -> Result<Artifact, String> {
    compile_observed_for_target(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        selectors,
        None,
        target,
    )
    .map(|(artifact, _)| artifact)
}

pub(crate) fn compile_stage_with_selectors(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<Artifact, String> {
    compile_bound(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
    )
    .map(|(artifact, _)| artifact)
}

pub(crate) fn compile_bound(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    compile_observed(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        None,
    )
}

pub(crate) fn compile_observed(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    observed: Option<&mut CompilationInputs>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    compile_observed_for_target(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        observed,
        None,
    )
}

pub(crate) fn compile_observed_for_target(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    compile_observed_for_target_with_invocation(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        observed,
        target,
        None,
    )
}

pub(crate) fn compile_observed_for_target_with_invocation(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    invocation: Option<&mut BuilderInvocation<'_>>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    compile_observed_for_target_with_incremental(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        observed,
        target,
        invocation,
        &IncrementalConfig::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_observed_for_target_with_incremental(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    invocation: Option<&mut BuilderInvocation<'_>>,
    incremental: &IncrementalConfig,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    if emit == Emit::Ast {
        return Err("package sets support HIR or NIR, not AST output".into());
    }
    let preludes: Vec<_> = builtin.into_iter().collect();
    compile_observed_with_preludes_and_macro_selectors_and_incremental(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        None,
        observed,
        target,
        &preludes,
        invocation,
        incremental,
        &[],
    )
}

pub(crate) fn compile_observed_with_preludes(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    invocation: Option<&mut BuilderInvocation<'_>>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    if emit == Emit::Ast {
        return Err("package sets support HIR or NIR, not AST output".into());
    }
    compile_observed_with_preludes_and_macro_selectors(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        None,
        observed,
        target,
        preludes,
        invocation,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_observed_with_preludes_and_macro_selectors(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    macro_selectors: Option<crate::protocols::type_constructors::Selectors<'_>>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    invocation: Option<&mut BuilderInvocation<'_>>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    compile_observed_with_preludes_and_macro_selectors_and_incremental(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        macro_selectors,
        observed,
        target,
        preludes,
        invocation,
        &IncrementalConfig::default(),
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_observed_with_preludes_and_macro_selectors_and_incremental(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    macro_selectors: Option<crate::protocols::type_constructors::Selectors<'_>>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    invocation: Option<&mut BuilderInvocation<'_>>,
    incremental: &IncrementalConfig,
    extra_config_files: &[PathBuf],
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    compile_with_expansion(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        macro_selectors,
        observed,
        target,
        preludes,
        None,
        invocation,
        incremental,
        extra_config_files,
    )
}

/// Host-side definition assembly after source lowering and before ordinary
/// protocol binding/elaboration. This is not a source-language builder or VM
/// callback. The callback runs once and must account for its own work using the
/// shared fuel; it cannot mutate cached HIR. No partially checked NIR is returned.
/// The caller's callback inputs must participate in any external build cache key.
/// Every generated owner must have an explicit DefId in a supplied source package;
/// newly declared clause identities must belong to that same package.
pub fn compile_with_definitions(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    target: Option<nir::target::TargetData>,
    fuel: &mut usize,
    expand: &mut DefinitionExpander<'_>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    let preludes: Vec<_> = builtin.into_iter().collect();
    compile_with_definitions_and_preludes(
        packages,
        dependencies,
        primary,
        builtin,
        protocols,
        selectors,
        target,
        &preludes,
        fuel,
        expand,
    )
}

/// Assemble generated definitions with the same explicit prelude providers as
/// ordinary package compilation. Provider IDs refer to the supplied packages;
/// package names do not confer builtin privileges or automatic-import status.
/// Resolution and provider validation finish before the callback runs. Each
/// provider excludes its own prelude, and std is imported only when requested
/// by source. Callers should pass the configured builtin/meta/build/attrs/
/// verification providers, not std, in `preludes`.
pub fn compile_with_definitions_and_preludes(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    fuel: &mut usize,
    expand: &mut DefinitionExpander<'_>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    compile_with_expansion(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        selectors,
        None,
        None,
        target,
        preludes,
        Some((fuel, expand)),
        None,
        &IncrementalConfig::default(),
        &[],
    )
}

/// Compile source packages, run one comptime entry with checked definition
/// builders, and return the latest checked package together with its value.
/// Earlier required invocations have already advanced the source identity frontier.
pub fn compile_and_invoke_with_struct_builders(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    entry: &str,
    structural_fuel: &mut usize,
    invocation_fuel: u64,
) -> Result<
    (
        Artifact,
        comptime_interpretor::Value,
        elaboration::program::Protocols,
    ),
    String,
> {
    let mut invocation = BuilderInvocation {
        entry,
        structural_fuel,
        invocation_fuel,
        value: None,
    };
    let (artifact, protocols) = compile_with_expansion(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        selectors,
        None,
        None,
        target,
        preludes,
        None,
        Some(&mut invocation),
        &IncrementalConfig::default(),
        &[],
    )?;
    Ok((
        artifact,
        invocation
            .value
            .ok_or("builder invocation produced no value")?,
        protocols,
    ))
}

pub(crate) struct BuilderInvocation<'a> {
    pub(crate) entry: &'a str,
    pub(crate) structural_fuel: &'a mut usize,
    pub(crate) invocation_fuel: u64,
    pub(crate) value: Option<comptime_interpretor::Value>,
}

impl BuilderInvocation<'_> {
    pub(crate) fn execute(
        &mut self,
        mut nir: nir::program::Package,
        identity: u64,
        source_frontier: u32,
        protocols: &elaboration::program::Protocols,
    ) -> Result<nir::program::Package, String> {
        elaboration::verification::optimization::elide_proven_checks(
            &mut nir,
            &mut elaboration::verification::Limits::default(),
        );
        let (value, definitions, next_identity) = crate::invocation::invoke_publishing(
            &nir,
            self.entry,
            identity,
            nir.definition_identity_cursor.max(source_frontier),
            self.structural_fuel,
            self.invocation_fuel,
        )?;
        self.value = Some(value);
        nir.definition_identity_cursor = next_identity;
        definitions.append_to(&mut nir);
        let completed = nir;
        crate::protocols::type_constructors::validate(&completed, protocols)?;
        Ok(completed)
    }
}

/// Generated names are not re-resolved into source AST. Callbacks connect
/// definitions using the ordinary resolved HIR identities and owner references.
pub type DefinitionExpander<'host> = dyn for<'hir> FnMut(
        &'hir hir::HirArena,
        &mut hir::definitions::DefinitionBatch<'_, 'hir>,
        &mut usize,
    ) -> Result<(), String>
    + 'host;

pub(crate) struct ProviderCompileOutput {
    pub(crate) nir: nir::program::Package,
    pub(crate) module_tree: Arc<resolve::ModuleTree>,
    pub(crate) package_scopes: std::collections::HashMap<u32, resolve::ScopeId>,
    pub(crate) package_identities: BTreeMap<u32, u64>,
}

enum CoreArtifact {
    Driver(Artifact),
    Provider(ProviderCompileOutput),
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_for_macro_provider(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    macro_host: &mut crate::macro_host::MacroHost,
    virtual_sources: &BTreeMap<PathBuf, Arc<str>>,
    force_directory_discovery_for_primary: bool,
    incremental: &IncrementalConfig,
) -> Result<ProviderCompileOutput, String> {
    let (artifact, _) = compile_packages_core(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        selectors,
        observed,
        target,
        preludes,
        None,
        None,
        Some(macro_host),
        None,
        virtual_sources,
        force_directory_discovery_for_primary,
        true,
        incremental,
        &[],
    )?;
    match artifact {
        CoreArtifact::Provider(out) => Ok(out),
        CoreArtifact::Driver(_) => unreachable!(),
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_with_expansion(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    macro_selectors: Option<crate::protocols::type_constructors::Selectors<'_>>,
    observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    expansion: Option<(&mut usize, &mut DefinitionExpander<'_>)>,
    builder_invocation: Option<&mut BuilderInvocation<'_>>,
    incremental: &IncrementalConfig,
    extra_config_files: &[PathBuf],
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    let empty_virtual = BTreeMap::new();
    let (artifact, protocols) = compile_packages_core(
        packages,
        dependencies,
        primary,
        builtin,
        emit,
        output,
        protocols,
        selectors,
        observed,
        target,
        preludes,
        expansion,
        builder_invocation,
        None,
        macro_selectors,
        &empty_virtual,
        false,
        false,
        incremental,
        extra_config_files,
    )?;
    match artifact {
        CoreArtifact::Driver(out) => Ok((out, protocols)),
        CoreArtifact::Provider(_) => unreachable!(),
    }
}

struct ParallelDiscoveredPackage {
    entry_read: Option<Result<Arc<str>, String>>,
    discovery: Option<ParallelPackageDiscovery>,
}

struct ParallelPackageDiscovery {
    canonical_root: PathBuf,
    exclusions: Vec<PathBuf>,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    discovered_reads: Vec<(PathBuf, Result<Arc<str>, String>)>,
}

fn all_resolved_protocols_in(
    protocols: &elaboration::program::Protocols,
    upstream_identities: &BTreeSet<u64>,
) -> bool {
    let decl_ok = |decl: Option<nir::DeclarationId>| {
        decl.is_none_or(|d| upstream_identities.contains(&d.package))
    };
    let host_pair_ok = |item: &Option<(nir::DeclarationId, String)>| {
        item.as_ref()
            .is_none_or(|(d, _)| upstream_identities.contains(&d.package))
    };
    [
        protocols.copy,
        protocols.drop,
        protocols.index,
        protocols.iterator,
        protocols.into_iterator,
        protocols.ordering,
        protocols.equality,
        protocols.list_pattern,
        protocols.comptime_list_pattern,
        protocols.literal_into,
        protocols.splice,
        protocols.meta_list,
        protocols.tuple,
    ]
    .into_iter()
    .all(decl_ok)
        && protocols.arithmetic.iter().copied().all(decl_ok)
        && host_pair_ok(&protocols.host_fma)
        && host_pair_ok(&protocols.host_sqrt)
        && host_pair_ok(&protocols.host_copysign)
        && protocols
            .comptime_intrinsics
            .iter()
            .all(|((d, _), _)| upstream_identities.contains(&d.package))
        && protocols
            .host_intrinsics
            .iter()
            .all(|((d, _), _)| upstream_identities.contains(&d.package))
}

#[allow(clippy::too_many_arguments)]
fn compile_packages_core(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    emit: Emit,
    output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
    mut observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
    preludes: &[u32],
    expansion: Option<(&mut usize, &mut DefinitionExpander<'_>)>,
    builder_invocation: Option<&mut BuilderInvocation<'_>>,
    macro_host: Option<&mut crate::macro_host::MacroHost>,
    macro_selectors: Option<crate::protocols::type_constructors::Selectors<'_>>,
    virtual_sources: &BTreeMap<PathBuf, Arc<str>>,
    force_directory_discovery_for_primary: bool,
    return_provider_output: bool,
    incremental: &IncrementalConfig,
    extra_config_files: &[PathBuf],
) -> Result<(CoreArtifact, elaboration::program::Protocols), String> {
    selectors.validate_stage(emit)?;
    if packages.is_empty() {
        return Err("at least one source package is required".into());
    }
    let jobs = incremental.effective_jobs();
    let cache_enabled = incremental.cache_enabled();
    let disk_cache = if cache_enabled {
        incremental.disk_cache()
    } else {
        DiskPackageCache::disabled()
    };

    let mut resolution_ids = HashSet::new();
    let mut identities = HashSet::new();
    for package in packages {
        if !virtual_sources.contains_key(&package.entry) {
            protect_source_output(&package.entry, output)?;
        }
        if package.name.is_empty() {
            return Err(format!(
                "source package {} has an empty name",
                package.resolution_id
            ));
        }
        if !resolution_ids.insert(package.resolution_id) {
            return Err(format!(
                "duplicate source package resolution id {} for package {:?}",
                package.resolution_id, package.name
            ));
        }
        if !identities.insert(package.identity) {
            return Err(format!(
                "duplicate source package semantic identity {} for package {:?} (resolution id {})",
                package.identity, package.name, package.resolution_id
            ));
        }
    }
    if !resolution_ids.contains(&primary) {
        return Err(format!("primary source package {primary} is absent"));
    }
    resolve::validate_package_dependencies(
        packages.iter().map(|package| package.resolution_id),
        dependencies,
        builtin,
    )
    .map_err(|error| format!("invalid package graph: {}", error.message()))?;
    // Ownership comes from the explicit package inputs, not incidental root
    // filenames found while walking another package's directory.
    let boundaries = parallel_map_ordered(packages, jobs, |package| -> Result<_, String> {
        let entry = if virtual_sources.contains_key(&package.entry) {
            let parent = package
                .entry
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let canon_dir = std::fs::canonicalize(parent)
                .map_err(|e| format!("cannot read package {}: {e}", package.name))?;
            return Ok((package.resolution_id, canon_dir));
        } else {
            std::fs::canonicalize(&package.entry)
                .map_err(|e| format!("cannot read package {}: {e}", package.name))?
        };
        let path = if matches!(
            package.entry.file_name().and_then(|n| n.to_str()),
            Some("lib.fl" | "main.fl")
        ) {
            entry
                .parent()
                .ok_or_else(|| "package entry has no parent".to_string())?
                .to_path_buf()
        } else {
            entry
        };
        Ok((package.resolution_id, path))
    })?;

    // Discover sources and read all package files in parallel before registering
    // them deterministically in SourceMap.
    let parallel_reads = parallel_map_ordered(packages, jobs, |package| -> Result<_, String> {
        let directory = package
            .entry
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let filename = package
            .entry
            .file_name()
            .ok_or("package entry has no filename")?;
        let is_virtual = virtual_sources.contains_key(&package.entry);
        let entry_read = if is_virtual {
            None
        } else {
            Some(
                std::fs::read_to_string(&package.entry)
                    .map(Arc::<str>::from)
                    .map_err(|e| format!("cannot read package {}: {e}", package.name)),
            )
        };
        let scan_dir = matches!(filename.to_str(), Some("lib.fl" | "main.fl"))
            || (force_directory_discovery_for_primary && package.resolution_id == primary);
        let discovery = if scan_dir {
            let mut exclusions: Vec<_> = boundaries
                .iter()
                .filter(|(id, _)| *id != package.resolution_id)
                .map(|(_, path)| path.clone())
                .collect();
            if !is_virtual
                && !(force_directory_discovery_for_primary && package.resolution_id == primary)
            {
                if let Some(scripts_dir) = crate::macro_host::scripts_exclusion(&package.entry) {
                    exclusions.push(scripts_dir);
                }
            }
            let (files, directories) = discover_sources_excluding(directory, &exclusions)?;
            let canonical_root =
                std::fs::canonicalize(directory).map_err(|error| error.to_string())?;
            let discovered_reads = files
                .iter()
                .filter(|relative| *relative != std::path::Path::new(filename))
                .map(|relative| {
                    let full = directory.join(relative);
                    let read = std::fs::read_to_string(&full)
                        .map(Arc::<str>::from)
                        .map_err(|e| e.to_string());
                    (relative.clone(), read)
                })
                .collect();
            Some(ParallelPackageDiscovery {
                canonical_root,
                exclusions,
                files,
                directories,
                discovered_reads,
            })
        } else {
            None
        };
        Ok(ParallelDiscoveredPackage {
            entry_read,
            discovery,
        })
    })?;

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut preloaded_texts: HashMap<PathBuf, Arc<str>> = HashMap::new();
    for (package, read_pkg) in packages.iter().zip(&parallel_reads) {
        if let Some(Ok(text)) = &read_pkg.entry_read {
            preloaded_texts
                .entry(cwd.join(&package.entry))
                .or_insert_with(|| Arc::clone(text));
        }
        if let Some(disc) = &read_pkg.discovery {
            let directory = package
                .entry
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            for (relative, read) in &disc.discovered_reads {
                if let Ok(text) = read {
                    preloaded_texts
                        .entry(cwd.join(directory).join(relative))
                        .or_insert_with(|| Arc::clone(text));
                }
            }
        }
    }

    let mut session = Session::new(CompilerConfig::new("package-set", cwd.clone()));
    let loader = ObservedLoader {
        root: cwd.clone(),
        preloaded: Arc::new(preloaded_texts),
        text: Default::default(),
    };
    session.source_map = rustc_span::source_map::SourceMap::with_file_loader_and_hash_kind(
        Box::new(loader.clone()),
        rustc_span::source_map::FilePathMapping::empty(),
        rustc_span::SourceFileHashAlgorithm::Md5,
    );
    let mut instance = CompilerInstance::new(&session);
    let mut inputs = Vec::with_capacity(packages.len());
    for (package, read_pkg) in packages.iter().zip(parallel_reads) {
        let directory = package
            .entry
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let filename = package
            .entry
            .file_name()
            .ok_or("package entry has no filename")?;
        let mut vfs = vfs::Vfs::new(&package.name, directory.to_path_buf());
        let is_virtual = virtual_sources.get(&package.entry);
        let source = if let Some(text) = is_virtual {
            session.source_map.new_source_file(
                rustc_span::FileName::Real(rustc_span::RealFileName::LocalPath(
                    package.entry.clone(),
                )),
                text.to_string(),
            )
        } else {
            if let Some(Err(err)) = read_pkg.entry_read {
                return Err(err);
            }
            session
                .source_map
                .load_file(&package.entry)
                .map_err(|e| format!("cannot read package {}: {e}", package.name))?
        };
        if let Some(inputs) = observed.as_deref_mut() {
            if is_virtual.is_none() {
                inputs.sources.push(SourceInput {
                    resolution_id: package.resolution_id,
                    identity: package.identity,
                    path: cwd.join(&package.entry),
                    raw_text: loader.get(&package.entry)?,
                    text: Arc::from(
                        source
                            .src
                            .as_ref()
                            .ok_or("source text is unavailable")?
                            .as_str(),
                    ),
                });
            }
        }
        let root = vfs.add_file(filename.into(), source);
        vfs.selected_root = Some(root);
        if let Some(disc) = read_pkg.discovery {
            if let Some(inputs) = observed.as_deref_mut() {
                inputs.discovery.push(SourceDiscovery {
                    resolution_id: package.resolution_id,
                    root: cwd.join(directory),
                    canonical_root: disc.canonical_root,
                    exclusions: disc.exclusions,
                    files: disc.files,
                    directories: disc.directories.clone(),
                });
                inputs
                    .directories
                    .push((package.resolution_id, cwd.join(directory)));
                inputs.directories.extend(
                    disc.directories.iter().map(|relative| {
                        (package.resolution_id, cwd.join(directory).join(relative))
                    }),
                );
            }
            vfs.source_directories = disc.directories;
            for (relative, read) in disc.discovered_reads {
                protect_source_output(&directory.join(&relative), output)?;
                if let Err(err) = read {
                    return Err(err);
                }
                let source = session
                    .source_map
                    .load_file(&directory.join(&relative))
                    .map_err(|e| e.to_string())?;
                if let Some(inputs) = observed.as_deref_mut() {
                    inputs.sources.push(SourceInput {
                        resolution_id: package.resolution_id,
                        identity: package.identity,
                        path: cwd.join(directory).join(&relative),
                        raw_text: loader.get(&directory.join(&relative))?,
                        text: Arc::from(
                            source
                                .src
                                .as_ref()
                                .ok_or("source text is unavailable")?
                                .as_str(),
                        ),
                    });
                }
                vfs.add_file(relative, source);
            }
        }
        inputs.push((package.resolution_id, vfs));
    }

    // Closed-upstream partition + ParsedPackage AST cache lookup (memory LRU + `.flpkg`).
    let partition = partition_closed_upstream(packages, dependencies, primary);
    let upstream_set: BTreeSet<u32> = partition.upstream.iter().copied().collect();
    struct PendingParsedCache {
        pkg_idx: usize,
        key: PackageSnapshotKey,
        file_meta: Vec<(String, [u8; 32])>,
    }
    let mut pending_parsed_stores = Vec::new();
    let mut pre_scanned_upstream_macros = false;

    if cache_enabled && !upstream_set.is_empty() {
        struct PkgScanInput {
            pkg_idx: usize,
            pkg_name: String,
            files: Vec<(vfs::FileId, PathBuf, rustc_span::BytePos, Option<Arc<str>>)>,
        }
        struct PkgScanOutput {
            pkg_idx: usize,
            has_macro_refs: bool,
            pkg_key: Option<PackageSnapshotKey>,
            file_meta: Vec<(String, [u8; 32])>,
            hit_asts: Option<Vec<(vfs::FileId, parse::ast::Ast)>>,
        }
        let mut scan_inputs = Vec::new();
        for (pkg_idx, pkg) in packages.iter().enumerate() {
            if !upstream_set.contains(&pkg.resolution_id) {
                continue;
            }
            if virtual_sources.contains_key(&pkg.entry) {
                continue;
            }
            if crate::macro_host::find_package_scripts_dir(&pkg.entry).is_some() {
                pre_scanned_upstream_macros = true;
                continue;
            }
            let (_, vfs) = &inputs[pkg_idx];
            let files = vfs
                .files()
                .map(|(fid, entry)| {
                    (
                        fid,
                        entry.rel_path.clone(),
                        entry.source_file.start_pos,
                        entry
                            .source_file
                            .src
                            .as_deref()
                            .map(|src| Arc::from(src.as_str())),
                    )
                })
                .collect();
            scan_inputs.push(PkgScanInput {
                pkg_idx,
                pkg_name: pkg.name.clone(),
                files,
            });
        }

        let scan_outputs = parallel_map_ordered(&scan_inputs, jobs, |item| -> Result<_, ()> {
            let mut has_macro_refs = false;
            let mut key_files: Vec<(PathBuf, &[u8])> = Vec::with_capacity(item.files.len());
            let mut file_meta: Vec<(String, [u8; 32])> = Vec::with_capacity(item.files.len());
            for (_, rel_path, _, src_opt) in &item.files {
                let Some(src) = src_opt.as_deref() else {
                    has_macro_refs = true;
                    break;
                };
                if macro_expander::scan_referenced_macro_roots(src).has_non_builtin_macros {
                    has_macro_refs = true;
                    break;
                }
                let rel_str = rel_path.to_string_lossy().into_owned();
                let hash = Sha256::hash(src.as_bytes());
                key_files.push((rel_path.clone(), src.as_bytes()));
                file_meta.push((rel_str, hash));
            }
            if has_macro_refs {
                return Ok(PkgScanOutput {
                    pkg_idx: item.pkg_idx,
                    has_macro_refs: true,
                    pkg_key: None,
                    file_meta: Vec::new(),
                    hit_asts: None,
                });
            }

            let pkg_key = compute_parsed_package_key(&item.pkg_name, &key_files);
            let cached = global_parsed_cache()
                .get(pkg_key, ArtifactKind::ParsedPackage)
                .or_else(|| {
                    let bytes = disk_cache.load(&pkg_key, ArtifactKind::ParsedPackage)?;
                    let decoded = Arc::new(decode_parsed_package(&bytes).ok()?);
                    global_parsed_cache().insert(
                        pkg_key,
                        ArtifactKind::ParsedPackage,
                        Arc::clone(&decoded),
                    );
                    Some(decoded)
                });
            let mut hit_asts = None;
            if let Some(cached_pkg) = cached {
                if cached_pkg.files.len() == item.files.len()
                    && cached_pkg
                        .files
                        .iter()
                        .zip(&file_meta)
                        .all(|(cf, (rel, hash))| {
                            cf.rel_path == *rel && cf.content_hash == *hash && cf.ast.is_some()
                        })
                {
                    let targets = item
                        .files
                        .iter()
                        .zip(&cached_pkg.files)
                        .map(|((fid, _, start_pos, _), cf)| {
                            let mut ast = cf.ast.as_ref().expect("verified cached AST").clone();
                            ast.rebase(*start_pos);
                            (*fid, ast)
                        })
                        .collect();
                    hit_asts = Some(targets);
                }
            }
            Ok(PkgScanOutput {
                pkg_idx: item.pkg_idx,
                has_macro_refs: false,
                pkg_key: Some(pkg_key),
                file_meta,
                hit_asts,
            })
        })
        .expect("parsed cache lookups never return Err");

        for out in scan_outputs {
            if out.has_macro_refs {
                pre_scanned_upstream_macros = true;
                continue;
            }
            if let Some(targets) = out.hit_asts {
                let (_, vfs) = &mut inputs[out.pkg_idx];
                for (fid, ast) in targets {
                    vfs.set_ast(fid, ast);
                }
            } else if let Some(pkg_key) = out.pkg_key {
                pending_parsed_stores.push(PendingParsedCache {
                    pkg_idx: out.pkg_idx,
                    key: pkg_key,
                    file_meta: out.file_meta,
                });
            }
        }
    } else if !upstream_set.is_empty() {
        for pkg in packages {
            if upstream_set.contains(&pkg.resolution_id)
                && crate::macro_host::find_package_scripts_dir(&pkg.entry).is_some()
            {
                pre_scanned_upstream_macros = true;
                break;
            }
        }
    }

    let mut owned_host;
    let active_host = match macro_host {
        Some(existing) => existing,
        None => {
            owned_host = crate::macro_host::MacroHost::new(
                packages,
                dependencies,
                builtin,
                protocols,
                macro_selectors.unwrap_or(selectors),
                target.clone(),
                preludes,
            )
            .with_incremental(incremental.clone());
            &mut owned_host
        }
    };
    active_host.expand_and_parse_packages(
        &session.source_map,
        &instance.diag_ctx,
        packages,
        dependencies,
        &mut inputs,
        observed,
    )?;

    let any_upstream_used_macros = pre_scanned_upstream_macros
        || packages.iter().any(|pkg| {
            upstream_set.contains(&pkg.resolution_id)
                && (crate::macro_host::find_package_scripts_dir(&pkg.entry).is_some()
                    || active_host.expanded_package(pkg.resolution_id))
        });

    // Parallel per-file builtin-macro expansion & parsing for any remaining unparsed
    // files (skips when emitting single-file AST or already populated by cache/macro_host).
    if emit != Emit::Ast {
        struct ParseWorkItem {
            pkg_idx: usize,
            file_id: vfs::FileId,
            file_name: rustc_span::FileName,
            src: Arc<str>,
            target_start_pos: rustc_span::BytePos,
        }
        let mut parse_work = Vec::new();
        for (pkg_idx, (_, vfs)) in inputs.iter().enumerate() {
            for (file_id, entry) in vfs.files() {
                if vfs.get_ast(file_id).is_some() {
                    continue;
                }
                let Some(src) = entry.source_file.src.as_deref() else {
                    continue;
                };
                parse_work.push(ParseWorkItem {
                    pkg_idx,
                    file_id,
                    file_name: entry.source_file.name.clone(),
                    src: Arc::from(src.as_str()),
                    target_start_pos: entry.source_file.start_pos,
                });
            }
        }
        if !parse_work.is_empty() {
            let parsed_asts = parallel_map_ordered(&parse_work, jobs, |item| -> Result<_, ()> {
                let local_sm = rustc_span::source_map::SourceMap::new(
                    rustc_span::source_map::FilePathMapping::empty(),
                );
                // Reserve BytePos(0)..BytePos(1) so zero-width spans at byte 0 of the
                // real file sit at BytePos(1) and never collide with rustc_span::DUMMY_SP.
                let _ = local_sm
                    .new_source_file(rustc_span::FileName::Custom("dummy".into()), " ".into());
                let local_sf =
                    local_sm.new_source_file(item.file_name.clone(), item.src.to_string());
                let local_diag = diagnostic::DiagnosticContext::new(&local_sm);
                let mut budget = macro_expander::MacroBudget::default();
                let Ok((expanded, lex_errors)) =
                    macro_expander::expand_source_file(&local_sm, &local_sf, &mut budget)
                else {
                    return Ok(None);
                };
                if expanded.expanded || !lex_errors.is_empty() {
                    return Ok(None);
                }
                let mut parser = parse::parser::Parser::new(
                    &local_sm,
                    expanded.tokens,
                    expanded.symbols,
                    expanded.start_pos,
                );
                let parsed = parser.parse(&local_diag);
                if !parsed || !parser.warnings().is_empty() || local_diag.has_errors() {
                    return Ok(None);
                }
                let ast = parser.finalize();
                if ast.validate_structure().is_err() {
                    return Ok(None);
                }
                Ok(Some(ast))
            })
            .expect("parallel parser workers never return Err");

            let store_indices: HashMap<usize, usize> = pending_parsed_stores
                .iter()
                .enumerate()
                .map(|(i, p)| (p.pkg_idx, i))
                .collect();
            let mut relative_asts_by_pkg: HashMap<
                usize,
                Vec<(vfs::FileId, Option<parse::ast::Ast>)>,
            > = HashMap::new();

            for (item, maybe_ast) in parse_work.into_iter().zip(parsed_asts) {
                let Some(mut ast) = maybe_ast else {
                    continue;
                };
                if store_indices.contains_key(&item.pkg_idx) {
                    relative_asts_by_pkg
                        .entry(item.pkg_idx)
                        .or_default()
                        .push((item.file_id, Some(ast.clone())));
                }
                ast.rebase(item.target_start_pos);
                inputs[item.pkg_idx].1.set_ast(item.file_id, ast);
            }

            for pending in pending_parsed_stores {
                let Some(rel_entries) = relative_asts_by_pkg.remove(&pending.pkg_idx) else {
                    continue;
                };
                if rel_entries.len() != pending.file_meta.len() {
                    continue;
                }
                let mut files = Vec::with_capacity(pending.file_meta.len());
                let mut by_file: HashMap<vfs::FileId, Option<parse::ast::Ast>> =
                    rel_entries.into_iter().collect();
                let mut complete = true;
                for (raw_idx, (rel_path, content_hash)) in pending.file_meta.into_iter().enumerate()
                {
                    let fid = vfs::FileId::from_raw(raw_idx as u32);
                    match by_file.remove(&fid).flatten() {
                        Some(ast) => files.push(CachedParsedFile {
                            rel_path,
                            content_hash,
                            ast: Some(ast),
                        }),
                        None => {
                            complete = false;
                            break;
                        }
                    }
                }
                if !complete {
                    continue;
                }
                let cached_pkg = CachedParsedPackage { files };
                let Ok(encoded) = encode_parsed_package(&cached_pkg) else {
                    continue;
                };
                global_parsed_cache().insert(
                    pending.key,
                    ArtifactKind::ParsedPackage,
                    Arc::new(cached_pkg),
                );
                if let Err(err) =
                    disk_cache.store(&pending.key, ArtifactKind::ParsedPackage, &encoded)
                {
                    if incremental.cache_dir.is_some() {
                        return Err(err);
                    }
                }
            }
        }
    }

    if emit == Emit::Ast {
        let (_, selected) = inputs.iter().find(|(id, _)| *id == primary).unwrap();
        let selected_file = selected.selected_root.unwrap();
        let ast = match selected.get_ast(selected_file) {
            Some(ast) => ast.clone(),
            None => {
                let source_file = selected.file(selected_file).source_file.clone();
                let mut macro_budget = macro_expander::MacroBudget::default();
                let (expanded, lex_errors) = match macro_expander::expand_source_file(
                    &session.source_map,
                    &source_file,
                    &mut macro_budget,
                ) {
                    Ok(res) => res,
                    Err(err) => {
                        err.emit(&instance.diag_ctx, source_file.start_pos);
                        return Err(instance.diag_ctx.failure_summary("macro expansion"));
                    }
                };
                for err in lex_errors {
                    err.emit(&instance.diag_ctx, source_file.start_pos);
                }
                if instance.diag_ctx.has_errors() {
                    return Err(instance.diag_ctx.failure_summary("lexing"));
                }
                let mut parser = parse::parser::Parser::new(
                    &session.source_map,
                    expanded.tokens,
                    expanded.symbols,
                    expanded.start_pos,
                );
                if !parser.parse(&instance.diag_ctx) {
                    return Err(instance.diag_ctx.failure_summary("parsing"));
                }
                let ast = parser.finalize();
                ast.validate_structure().map_err(|e| e.to_string())?;
                ast
            }
        };
        return Ok((
            CoreArtifact::Driver(Artifact::Ast(
                ast.dump_to_s_expression(ast.root, &session.source_map),
            )),
            protocols.clone(),
        ));
    }

    let mut references: Vec<_> = inputs.iter_mut().map(|(id, vfs)| (*id, vfs)).collect();
    let resolved = resolve::build_package_set_with_preludes(
        &session.source_map,
        &instance.diag_ctx,
        &mut references,
        dependencies,
        builtin,
        preludes,
    )
    .map_err(|e| format!("package resolution failed: {}", e.message()))?;
    if instance.diag_ctx.has_errors() || !resolved.tree.errors.is_empty() {
        let mut message = instance.diag_ctx.failure_summary("package resolution");
        if !resolved.tree.errors.is_empty() {
            message.push_str(&format!(": {:?}", resolved.tree.errors));
        }
        return Err(message);
    }
    let (_, selected) = inputs.iter().find(|(id, _)| *id == primary).unwrap();
    let selected_file = selected.selected_root.unwrap();
    let file_id = vfs::FileId::from_raw(selected_file.raw() + resolved.file_offsets[&primary]);
    let scope = resolved.tree.file_scopes[&file_id];
    let ast = Arc::new(
        selected
            .get_ast(selected_file)
            .ok_or("primary AST was not parsed")?
            .clone(),
    );
    let mut input = middle::HirQueryInput::new(
        ast,
        file_id,
        Arc::new(resolved.tree),
        scope,
        &session.source_map,
        Arc::clone(&instance.diag_ctx),
    );
    for (id, vfs) in &inputs {
        for (local_file, _) in vfs.files() {
            let combined = vfs::FileId::from_raw(local_file.raw() + resolved.file_offsets[id]);
            if combined == file_id {
                continue;
            }
            let ast = vfs
                .get_ast(local_file)
                .ok_or("dependency AST was not parsed")?;
            input.additional_files.push((
                combined,
                Arc::new(ast.clone()),
                input.module_tree.file_scopes[&combined],
            ));
        }
    }
    ast_lowering::set_providers(&mut instance.db.providers);
    let module_tree_arc = Arc::clone(&input.module_tree);
    instance.set_hir_input(Arc::new(input));
    let hir = instance.enter(|compiler| compiler.hir_package());
    if instance.diag_ctx.has_errors() {
        return Err(instance
            .diag_ctx
            .failure_summary("multi-package HIR lowering"));
    }
    let has_expansion = expansion.is_some();
    let expanded;
    let package = if let Some((fuel, expand)) = expansion {
        expanded = {
            let mut candidate = hir::definitions::fork_package(hir.package(), fuel)
                .map_err(|error| format!("definition expansion preparation failed: {error}"))?;
            let mut batch = hir::definitions::DefinitionBatch::new(&mut candidate);
            expand(hir.arena(), &mut batch, fuel)?;
            batch
                .publish(fuel)
                .map_err(|error| format!("definition expansion publication failed: {error}"))?;
            for index in hir.package().num_defs()..candidate.num_defs() {
                let charge = |fuel: &mut usize| -> Result<(), String> {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("generated package identity budget exhausted")?;
                    Ok(())
                };
                charge(fuel)?;
                let owner = hir::OwnerId::new(hir::LocalDefId::new(index as u32));
                let identity = candidate
                    .owner_definition(owner)
                    .ok_or("generated definition requires an explicit package identity")?;
                if !resolution_ids.contains(&identity.pkg) {
                    return Err(format!(
                        "generated definition uses unknown source package {}",
                        identity.pkg
                    ));
                }
                let item = candidate
                    .item(owner)
                    .ok_or("generated definition is missing")?;
                for parameter in hir::definitions::declared_clause_parameters(item) {
                    charge(fuel)?;
                    if parameter
                        .definition
                        .is_some_and(|id| id.pkg != identity.pkg)
                    {
                        return Err(
                            "generated clause parameter belongs to a different package".into()
                        );
                    }
                }
            }
            candidate
        };
        &expanded
    } else {
        hir.package()
    };
    if emit == Emit::Hir {
        return Ok((
            CoreArtifact::Driver(Artifact::Hir(package.dump_to_lisp())),
            protocols.clone(),
        ));
    }
    let identities: BTreeMap<_, _> = packages
        .iter()
        .map(|p| (p.resolution_id, p.identity))
        .collect();
    let initial_protocols = protocols;
    let mut protocols = crate::protocols::type_constructors::resolve_source(
        package,
        &packages
            .iter()
            .map(|p| (p.resolution_id, p.identity, p.name.as_str()))
            .collect::<Vec<_>>(),
        selectors,
        initial_protocols,
    )?;
    if let Some(selected) = builtin {
        if let Some(package) = packages
            .iter()
            .find(|package| package.resolution_id == selected)
        {
            protocols
                .builtin_implementation_packages
                .insert(package.identity);
        }
    }
    if matches!(emit, Emit::Attributes | Emit::MergedAttributes) {
        let attributes =
            elaboration::program::attributes::inspect(package, &identities, &protocols, target)
                .map_err(|error| format!("multi-package attribute inspection failed: {error}"))?;
        return Ok((
            CoreArtifact::Driver(crate::frontend::attribute_artifact(
                package, attributes, emit,
            )?),
            protocols,
        ));
    }

    let upstream_identities: BTreeSet<u64> = packages
        .iter()
        .filter(|pkg| upstream_set.contains(&pkg.resolution_id))
        .map(|pkg| pkg.identity)
        .collect();
    let downstream_identities: BTreeSet<u64> = packages
        .iter()
        .filter(|pkg| !upstream_set.contains(&pkg.resolution_id))
        .map(|pkg| pkg.identity)
        .collect();
    let upstream_verification_eligible = emit == Emit::Nir
        && !has_expansion
        && virtual_sources.is_empty()
        && !partition.upstream.is_empty()
        && !any_upstream_used_macros
        && downstream_identities.is_disjoint(&protocols.builtin_implementation_packages)
        && all_resolved_protocols_in(&protocols, &upstream_identities);

    let selector_tag = encode_selectors_tag(&selectors);
    let mut verified_publish_target: Option<(PackageSnapshotKey, Vec<u64>)> = None;
    let verified_upstream_packages = if cache_enabled && upstream_verification_eligible {
        let (upstream_key, _) = compute_upstream_key_with_jobs(
            &UpstreamKeyInputs {
                all_packages: packages,
                upstream_ids: &partition.upstream,
                upstream_edges: &partition.upstream_edges,
                builtin,
                preludes,
                extra_config_files,
                target: target.clone(),
                protocols: initial_protocols,
                selector_tag: &selector_tag,
                compiler_override: None,
            },
            jobs,
        )?;
        let expected_sorted: Vec<u64> = upstream_identities.iter().copied().collect();
        let cached_cert = global_verified_cache()
            .get(upstream_key, ArtifactKind::VerifiedUpstream)
            .or_else(|| {
                let bytes = disk_cache.load(&upstream_key, ArtifactKind::VerifiedUpstream)?;
                let decoded = Arc::new(decode_verified_upstream(&bytes).ok()?);
                global_verified_cache().insert(
                    upstream_key,
                    ArtifactKind::VerifiedUpstream,
                    Arc::clone(&decoded),
                );
                Some(decoded)
            });
        if cached_cert
            .as_ref()
            .is_some_and(|cert| cert.upstream_identities == expected_sorted)
        {
            upstream_identities
        } else {
            verified_publish_target = Some((upstream_key, expected_sorted));
            BTreeSet::new()
        }
    } else {
        BTreeSet::new()
    };

    let nir = elaboration::program::elaborate_packages_incremental(
        package,
        &identities,
        &protocols,
        target,
        &elaboration::program::IncrementalOptions {
            verified_upstream_packages,
            jobs: Some(jobs),
        },
    )
    .map_err(|e| format!("multi-package elaboration failed: {e}"))?;
    crate::protocols::type_constructors::validate(&nir, &protocols)?;
    if let Some((upstream_key, upstream_sorted)) = verified_publish_target {
        let cert = VerifiedUpstreamCertificate {
            upstream_identities: upstream_sorted,
        };
        let encoded = encode_verified_upstream(&cert);
        global_verified_cache().insert(
            upstream_key,
            ArtifactKind::VerifiedUpstream,
            Arc::new(cert),
        );
        if let Err(err) = disk_cache.store(&upstream_key, ArtifactKind::VerifiedUpstream, &encoded)
        {
            if incremental.cache_dir.is_some() {
                return Err(err);
            }
        }
    }
    let nir = if let Some(invocation) = builder_invocation {
        invocation.execute(
            nir,
            identities[&primary],
            package.next_definition_index(),
            &protocols,
        )?
    } else {
        nir
    };
    if return_provider_output {
        return Ok((
            CoreArtifact::Provider(ProviderCompileOutput {
                nir,
                module_tree: module_tree_arc,
                package_scopes: resolved.package_scopes,
                package_identities: identities,
            }),
            protocols,
        ));
    }
    Ok((CoreArtifact::Driver(Artifact::Nir(nir)), protocols))
}

pub(crate) fn protect_source_output(
    source: &std::path::Path,
    output: Option<&std::path::Path>,
) -> Result<(), String> {
    let Some(output) = output else { return Ok(()) };
    let (Ok(source_metadata), Ok(output_metadata)) =
        (std::fs::metadata(source), std::fs::metadata(output))
    else {
        return Ok(());
    };
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        source_metadata.dev() == output_metadata.dev()
            && source_metadata.ino() == output_metadata.ino()
    };
    #[cfg(not(unix))]
    let same = source.canonicalize().ok() == output.canonicalize().ok();
    if same {
        Err(format!(
            "output must not refer to source file '{}'",
            source.display()
        ))
    } else {
        Ok(())
    }
}

impl CompilationInputs {
    /// Rescan using the recorded package boundaries, without parsing or
    /// executing source. I/O failures and unsupported symlinks are errors.
    /// A missing scan root reports its recorded members as removed.
    pub fn changed_discovery(&self) -> Result<Vec<DiscoveryChange>, String> {
        use std::collections::BTreeSet;
        let mut changes = Vec::new();
        for scan in &self.discovery {
            let (files, directories) = match std::fs::canonicalize(&scan.root) {
                Ok(root) if root == scan.canonical_root => {
                    discover_sources_excluding(&scan.root, &scan.exclusions)?
                }
                Ok(_) => {
                    return Err(format!(
                        "source discovery root changed identity: '{}'",
                        scan.root.display()
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    (Vec::new(), Vec::new())
                }
                Err(error) => {
                    return Err(format!(
                        "cannot revalidate discovery root '{}': {error}",
                        scan.root.display()
                    ));
                }
            };
            let differences = |before: &[PathBuf], after: &[PathBuf]| {
                let before: BTreeSet<_> = before.iter().collect();
                let after: BTreeSet<_> = after.iter().collect();
                (
                    after
                        .difference(&before)
                        .map(|path| scan.root.join(path))
                        .collect::<Vec<_>>(),
                    before
                        .difference(&after)
                        .map(|path| scan.root.join(path))
                        .collect::<Vec<_>>(),
                )
            };
            let (added_files, removed_files) = differences(&scan.files, &files);
            let (added_directories, removed_directories) =
                differences(&scan.directories, &directories);
            if !added_files.is_empty()
                || !removed_files.is_empty()
                || !added_directories.is_empty()
                || !removed_directories.is_empty()
            {
                changes.push(DiscoveryChange {
                    resolution_id: scan.resolution_id,
                    added_files,
                    removed_files,
                    added_directories,
                    removed_directories,
                });
            }
        }
        Ok(changes)
    }

    /// Compare current file bytes with the exact inputs originally loaded.
    /// Missing files are changes; other I/O failures remain errors. This does
    /// not validate directory membership, request options or atomicity.
    pub fn changed_sources(&self) -> Result<Vec<&SourceInput>, String> {
        let mut changed = Vec::new();
        for input in &self.sources {
            match std::fs::read(&input.path) {
                Ok(bytes) if bytes == input.raw_text.as_bytes() => {}
                Ok(_) => changed.push(input),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => changed.push(input),
                Err(error) => {
                    return Err(format!(
                        "cannot revalidate source '{}': {error}",
                        input.path.display()
                    ));
                }
            }
        }
        Ok(changed)
    }
}

fn write_input_path(f: &mut std::fmt::Formatter<'_>, path: &std::path::Path) -> std::fmt::Result {
    f.write_str("(path-bytes \"")?;
    for byte in path.as_os_str().as_encoded_bytes() {
        write!(f, "{byte:02x}")?;
    }
    f.write_str("\")")
}

impl std::fmt::Display for CompilationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Native path bytes are lossless on the producing platform. They are
        // opaque to other platforms, just like the OS-native CLI path inputs.
        f.write_str("(compilation-inputs (version 1) (sources")?;
        for input in &self.sources {
            write!(
                f,
                " (source (resolution-id {}) (identity {}) ",
                input.resolution_id, input.identity
            )?;
            write_input_path(f, &input.path)?;
            write!(
                f,
                " (text (string {}))",
                nir::QuotedDisplay(input.text.as_ref())
            )?;
            if input.raw_text != input.text {
                write!(
                    f,
                    " (raw-text (string {}))",
                    nir::QuotedDisplay(input.raw_text.as_ref())
                )?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (directories")?;
        for (owner, directory) in &self.directories {
            write!(f, " (directory (resolution-id {owner}) ")?;
            write_input_path(f, directory)?;
            f.write_str(")")?;
        }
        f.write_str(") (discovery")?;
        for scan in &self.discovery {
            write!(f, " (scan (resolution-id {}) (root ", scan.resolution_id)?;
            write_input_path(f, &scan.root)?;
            f.write_str(") (canonical-root ")?;
            write_input_path(f, &scan.canonical_root)?;
            f.write_str(")")?;
            for (label, paths) in [
                ("exclusions", &scan.exclusions),
                ("files", &scan.files),
                ("directories", &scan.directories),
            ] {
                write!(f, " ({label}")?;
                for value in paths {
                    f.write_str(" ")?;
                    write_input_path(f, value)?;
                }
                f.write_str(")")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}

impl std::fmt::Display for DiscoveryChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(discovery-change (resolution-id {})",
            self.resolution_id
        )?;
        for (label, paths) in [
            ("added-files", &self.added_files),
            ("removed-files", &self.removed_files),
            ("added-directories", &self.added_directories),
            ("removed-directories", &self.removed_directories),
        ] {
            write!(f, " ({label}")?;
            for path in paths {
                f.write_str(" ")?;
                write_input_path(f, path)?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}
