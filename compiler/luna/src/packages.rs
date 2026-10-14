//! Explicit source-package inputs for the driver. Dependency discovery and build
//! identities belong to the build tool; this API does not infer them from paths.
use crate::frontend::{Artifact, discover_sources_excluding};
use crate::cli::Emit;
use interface::{CompilerConfig, CompilerInstance, Session};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

pub struct SourcePackage {
    pub resolution_id: u32,
    pub identity: u64,
    pub name: String,
    pub entry: PathBuf,
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
    compile_stage_with_protocols(packages, dependencies, primary, builtin, emit, output, &Default::default(), None)
}

/// Explicit package-set entry with driver-bound language protocols.
pub fn compile_with_protocols(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
    builtin: Option<u32>,
    protocols: &elaboration::program::Protocols,
) -> Result<Artifact, String> {
    compile_stage_with_protocols(packages, dependencies, primary, builtin, Emit::Nir, None, protocols, None)
}

/// Resolve an explicitly designated source protocol in the same compilation.
/// Qualify with the supplied source package name when trait names collide.
pub fn compile_with_splice(
    packages: &[SourcePackage], dependencies: &[resolve::DependencyEdge],
    primary: u32, builtin: Option<u32>, selector: &str,
) -> Result<Artifact, String> {
    compile_stage_with_protocols(packages, dependencies, primary, builtin, Emit::Nir, None,
        &Default::default(), Some(selector))
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
    compile_stage_with_selectors(packages, dependencies, primary, builtin, emit, output, protocols,
        crate::protocols::type_constructors::Selectors { splice: splice_selector, ..Default::default() })
}

/// Bind driver-selected source protocols after resolution in this compilation.
/// Selectors are qualified by the supplied package names, not dependency aliases.
pub fn compile_with_source_protocols(
    packages: &[SourcePackage], dependencies: &[resolve::DependencyEdge],
    primary: u32, builtin: Option<u32>, protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<Artifact, String> {
    compile_stage_with_selectors(packages, dependencies, primary, builtin, Emit::Nir, None, protocols, selectors)
}

pub(crate) fn compile_stage_with_selectors(
    packages: &[SourcePackage], dependencies: &[resolve::DependencyEdge],
    primary: u32, builtin: Option<u32>, emit: Emit, output: Option<&std::path::Path>,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<Artifact, String> {
    if packages.is_empty() {
        return Err("at least one source package is required".into());
    }
    let mut resolution_ids = HashSet::new();
    let mut identities = HashSet::new();
    for package in packages {
        protect_source_output(&package.entry, output)?;
        if package.name.is_empty()
            || !resolution_ids.insert(package.resolution_id)
            || !identities.insert(package.identity)
        {
            return Err(
                "source packages require nonempty names and unique resolution/semantic identities"
                    .into(),
            );
        }
    }
    if !resolution_ids.contains(&primary) {
        return Err("primary source package is absent".into());
    }
    // Ownership comes from the explicit package inputs, not incidental root
    // filenames found while walking another package's directory.
    let boundaries = packages.iter().map(|package| {
        let entry = std::fs::canonicalize(&package.entry)
            .map_err(|e| format!("cannot read package {}: {e}", package.name))?;
        let path = if matches!(package.entry.file_name().and_then(|n| n.to_str()), Some("lib.fl" | "main.fl")) {
            entry.parent().ok_or("package entry has no parent")?.to_path_buf()
        } else {
            entry
        };
        Ok((package.resolution_id, path))
    }).collect::<Result<Vec<_>, String>>()?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let session = Session::new(CompilerConfig::new("package-set", cwd));
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
        let root = vfs.add_file(filename.into(), source);
        vfs.selected_root = Some(root);
        if matches!(filename.to_str(), Some("lib.fl" | "main.fl")) {
            let exclusions: Vec<_> = boundaries.iter()
                .filter(|(id, _)| *id != package.resolution_id)
                .map(|(_, path)| path.clone()).collect();
            let (files, directories) = discover_sources_excluding(directory, &exclusions)?;
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
    .map_err(|e| format!("package resolution failed: {e:?}"))?;
    if instance.diag_ctx.has_errors() || !resolved.tree.errors.is_empty() {
        return Err(format!(
            "package resolution failed: {:?}",
            resolved.tree.errors
        ));
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
        return Err("multi-package HIR lowering failed".into());
    }
    if emit == Emit::Hir {
        return Ok(Artifact::Hir(hir.package().dump_to_lisp()));
    }
    let identities: BTreeMap<_, _> = packages
        .iter()
        .map(|p| (p.resolution_id, p.identity))
        .collect();
    let protocols = crate::protocols::type_constructors::resolve_source(hir.package(),
        &packages.iter().map(|p| (p.resolution_id, p.identity, p.name.as_str())).collect::<Vec<_>>(),
        selectors, protocols)?;
    let nir = elaboration::program::elaborate_packages_with_protocols(hir.package(), &identities, &protocols)
        .map_err(|e| format!("multi-package elaboration failed: {e}"))?;
    crate::protocols::type_constructors::validate(&nir, &protocols)?;
    Ok(Artifact::Nir(nir))
}

pub(crate) fn protect_source_output(source: &std::path::Path, output: Option<&std::path::Path>) -> Result<(), String> {
    let Some(output) = output else { return Ok(()) };
    let (Ok(source_metadata), Ok(output_metadata)) = (std::fs::metadata(source), std::fs::metadata(output)) else { return Ok(()) };
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        source_metadata.dev() == output_metadata.dev() && source_metadata.ino() == output_metadata.ino()
    };
    #[cfg(not(unix))]
    let same = source.canonicalize().ok() == output.canonicalize().ok();
    if same { Err(format!("output must not refer to source file '{}'", source.display())) } else { Ok(()) }
}
