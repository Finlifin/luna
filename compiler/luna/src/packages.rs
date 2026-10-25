//! Explicit source-package inputs for the driver. Dependency discovery and build
//! identities belong to the build tool; this API does not infer them from paths.
use crate::cli::Emit;
use crate::frontend::{Artifact, discover_sources_excluding};
use interface::{CompilerConfig, CompilerInstance, Session};
use std::{
    collections::{BTreeMap, HashSet},
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
    let mut inputs = CompilationInputs::default();
    let (artifact, _) = compile_observed(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        selectors,
        Some(&mut inputs),
    )?;
    Ok((artifact, inputs))
}

#[derive(Clone)]
struct ObservedLoader {
    root: PathBuf,
    text: Arc<std::sync::Mutex<BTreeMap<PathBuf, Arc<str>>>>,
}
impl rustc_span::source_map::FileLoader for ObservedLoader {
    fn file_exists(&self, path: &std::path::Path) -> bool {
        path.exists()
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
    compile_stage_with_selectors(
        packages,
        dependencies,
        primary,
        builtin,
        Emit::Nir,
        None,
        protocols,
        selectors,
    )
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
    mut observed: Option<&mut CompilationInputs>,
    target: Option<nir::target::TargetData>,
) -> Result<(Artifact, elaboration::program::Protocols), String> {
    if emit == Emit::Ast {
        return Err("package sets support HIR or NIR, not AST output".into());
    }
    selectors.validate_stage(emit)?;
    if packages.is_empty() {
        return Err("at least one source package is required".into());
    }
    let mut resolution_ids = HashSet::new();
    let mut identities = HashSet::new();
    for package in packages {
        protect_source_output(&package.entry, output)?;
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
    let boundaries = packages
        .iter()
        .map(|package| {
            let entry = std::fs::canonicalize(&package.entry)
                .map_err(|e| format!("cannot read package {}: {e}", package.name))?;
            let path = if matches!(
                package.entry.file_name().and_then(|n| n.to_str()),
                Some("lib.fl" | "main.fl")
            ) {
                entry
                    .parent()
                    .ok_or("package entry has no parent")?
                    .to_path_buf()
            } else {
                entry
            };
            Ok((package.resolution_id, path))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut session = Session::new(CompilerConfig::new("package-set", cwd.clone()));
    let loader = observed.is_some().then(|| ObservedLoader {
        root: cwd.clone(),
        text: Default::default(),
    });
    if let Some(loader) = &loader {
        session.source_map = rustc_span::source_map::SourceMap::with_file_loader_and_hash_kind(
            Box::new(loader.clone()),
            rustc_span::source_map::FilePathMapping::empty(),
            rustc_span::SourceFileHashAlgorithm::Md5,
        );
    }
    let mut instance = CompilerInstance::new(&session);
    let mut inputs = Vec::new();
    for package in packages {
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
        let source = session
            .source_map
            .load_file(&package.entry)
            .map_err(|e| format!("cannot read package {}: {e}", package.name))?;
        if let Some(inputs) = observed.as_deref_mut() {
            inputs.sources.push(SourceInput {
                resolution_id: package.resolution_id,
                identity: package.identity,
                path: cwd.join(&package.entry),
                raw_text: loader
                    .as_ref()
                    .expect("observation loader is installed")
                    .get(&package.entry)?,
                text: Arc::from(
                    source
                        .src
                        .as_ref()
                        .ok_or("source text is unavailable")?
                        .as_str(),
                ),
            });
        }
        let root = vfs.add_file(filename.into(), source);
        vfs.selected_root = Some(root);
        if matches!(filename.to_str(), Some("lib.fl" | "main.fl")) {
            let exclusions: Vec<_> = boundaries
                .iter()
                .filter(|(id, _)| *id != package.resolution_id)
                .map(|(_, path)| path.clone())
                .collect();
            let (files, directories) = discover_sources_excluding(directory, &exclusions)?;
            if let Some(inputs) = observed.as_deref_mut() {
                inputs.discovery.push(SourceDiscovery {
                    resolution_id: package.resolution_id,
                    root: cwd.join(directory),
                    canonical_root: std::fs::canonicalize(directory)
                        .map_err(|error| error.to_string())?,
                    exclusions: exclusions.clone(),
                    files: files.clone(),
                    directories: directories.clone(),
                });
                inputs
                    .directories
                    .push((package.resolution_id, cwd.join(directory)));
                inputs.directories.extend(
                    directories.iter().map(|relative| {
                        (package.resolution_id, cwd.join(directory).join(relative))
                    }),
                );
            }
            vfs.source_directories = directories;
            for relative in files {
                if relative == std::path::Path::new(filename) {
                    continue;
                }
                protect_source_output(&directory.join(&relative), output)?;
                let source = session
                    .source_map
                    .load_file(&directory.join(&relative))
                    .map_err(|e| e.to_string())?;
                if let Some(inputs) = observed.as_deref_mut() {
                    inputs.sources.push(SourceInput {
                        resolution_id: package.resolution_id,
                        identity: package.identity,
                        path: cwd.join(directory).join(&relative),
                        raw_text: loader
                            .as_ref()
                            .expect("observation loader is installed")
                            .get(&directory.join(&relative))?,
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
    let mut references: Vec<_> = inputs.iter_mut().map(|(id, vfs)| (*id, vfs)).collect();
    let resolved = resolve::build_package_set_with_builtin(
        &session.source_map,
        &instance.diag_ctx,
        &mut references,
        dependencies,
        builtin,
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
        &session.source_map as *const rustc_span::SourceMap,
        &instance.diag_ctx as *const diagnostic::DiagnosticContext<'_>
            as *const diagnostic::DiagnosticContext<'static>,
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
    instance.set_hir_input(Arc::new(input));
    let hir = instance.enter(|compiler| compiler.hir_package());
    if instance.diag_ctx.has_errors() {
        return Err(instance
            .diag_ctx
            .failure_summary("multi-package HIR lowering"));
    }
    if emit == Emit::Hir {
        return Ok((
            Artifact::Hir(hir.package().dump_to_lisp()),
            protocols.clone(),
        ));
    }
    let identities: BTreeMap<_, _> = packages
        .iter()
        .map(|p| (p.resolution_id, p.identity))
        .collect();
    let protocols = crate::protocols::type_constructors::resolve_source(
        hir.package(),
        &packages
            .iter()
            .map(|p| (p.resolution_id, p.identity, p.name.as_str()))
            .collect::<Vec<_>>(),
        selectors,
        protocols,
    )?;
    let nir = elaboration::program::elaborate_packages_for_target(
        hir.package(),
        &identities,
        &protocols,
        target,
    )
    .map_err(|e| format!("multi-package elaboration failed: {e}"))?;
    crate::protocols::type_constructors::validate(&nir, &protocols)?;
    Ok((Artifact::Nir(nir), protocols))
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
