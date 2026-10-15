//! Shared frontend entry: CLI and semantic execution tests use the same pipeline.
use crate::cli::{Emit, Options};
use diagnostic::FlurryError;
use interface::{CompilerConfig, CompilerInstance, Session};
use parse::parser::Parser;
use std::sync::Arc;

pub enum Artifact {
    Ast(String),
    Hir(String),
    Nir(nir::program::Package),
}
impl std::fmt::Display for Artifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ast(text) | Self::Hir(text) => f.write_str(text),
            Self::Nir(package) => package.fmt(f),
        }
    }
}
pub fn compile(options: &Options) -> Result<Artifact, String> {
    compile_in_package(options, 0)
}

/// Resource protocol selectors must reach elaboration before pure bodies are checked.
pub fn compile_with_resource_protocols(
    options: &Options,
    copy: Option<&str>,
    drop: Option<&str>,
) -> Result<Artifact, String> {
    let mut selectors = source_protocols(options);
    selectors.copy = copy;
    selectors.drop = drop;
    compile_in_package_with_source_protocols(options, 0, &Default::default(), selectors)
}

/// Compile one selected source package under a driver-assigned semantic identity.
/// Other packages are loaded only when a sysroot is explicitly supplied.
pub fn compile_in_package(options: &Options, package_identity: u64) -> Result<Artifact, String> {
    compile_in_package_with_protocols(options, package_identity, &Default::default())
}

/// Compile using language-item identities selected by the trusted build driver.
pub fn compile_in_package_with_protocols(
    options: &Options,
    package_identity: u64,
    protocols: &elaboration::program::Protocols,
) -> Result<Artifact, String> {
    compile_in_package_with_source_protocols(
        options,
        package_identity,
        protocols,
        source_protocols(options),
    )
}

/// Resolve explicit protocol selectors before behavior validation.
pub fn compile_in_package_with_source_protocols(
    options: &Options,
    package_identity: u64,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<Artifact, String> {
    if options.literal_into_protocol.is_some() && options.emit != Emit::Nir {
        return Err("LiteralInto protocol binding requires NIR elaboration".into());
    }
    if options.index_protocol.is_some() && options.emit != Emit::Nir {
        return Err("Index protocol binding requires NIR elaboration".into());
    }
    if options.splice_protocol.is_some() && options.emit != Emit::Nir {
        return Err("Splice protocol binding requires NIR elaboration".into());
    }
    if (options.meta_list_type.is_some()
        || options.tuple_type.is_some()
        || options.optional_type.is_some())
        && options.emit != Emit::Nir
    {
        return Err("type constructor binding requires NIR elaboration".into());
    }
    let cwd = std::env::current_dir()
        .map_err(|error| format!("cannot determine the current directory: {error}"))?;
    let mut config = CompilerConfig::new("input", cwd);
    config.sysroot_override = options.sysroot.clone();
    let sess = Session::new(config);
    if let Some(path) = &options.sysroot {
        if sess.sysroot.is_none() {
            return Err(format!(
                "invalid sysroot '{}': expected a directory containing builtin/ and std/",
                path.display()
            ));
        }
        if options.emit != Emit::Ast {
            return compile_sysroot(options, package_identity, path, protocols, selectors);
        }
    }

    let source_file = sess
        .source_map
        .load_file(&options.input)
        .map_err(|error| format!("cannot read input '{}': {error}", options.input.display()))?;
    let src = source_file
        .src
        .as_ref()
        .ok_or_else(|| format!("source text unavailable for '{}'", options.input.display()))?;

    let mut instance = CompilerInstance::new(&sess);
    let file_id = instance.vfs_mut().add_file(
        std::path::PathBuf::from(options.input.file_name().ok_or("input has no file name")?),
        source_file.clone(),
    );
    instance.vfs_mut().selected_root = Some(file_id);

    let (tokens, symbols, lex_errors) = lex::lex(src, source_file.start_pos);
    for error in &lex_errors {
        error.emit(&instance.diag_ctx, source_file.start_pos);
    }
    if instance.diag_ctx.has_errors() {
        return Err(instance.diag_ctx.failure_summary("lexing"));
    }

    let mut parser = Parser::new(&sess.source_map, tokens, symbols, source_file.start_pos);
    parser.parse(&instance.diag_ctx);
    if instance.diag_ctx.has_errors() {
        return Err(instance.diag_ctx.failure_summary("parsing"));
    }
    let ast = parser.finalize();
    if options.emit == Emit::Ast {
        return Ok(Artifact::Ast(
            ast.dump_to_s_expression(ast.root, &sess.source_map),
        ));
    }
    instance.vfs_mut().set_ast(file_id, ast);

    // A selected main/lib entry gives the driver a bounded source-root input.
    // Standalone source files continue to compile without scanning neighbours.
    if matches!(
        options.input.file_name().and_then(|name| name.to_str()),
        Some("main.fl" | "lib.fl")
    ) {
        let directory = options
            .input
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let (files, directories) = discover_sources(directory)?;
        instance.vfs_mut().source_directories = directories;
        for relative in files {
            if relative == std::path::Path::new(options.input.file_name().unwrap()) {
                continue;
            }
            let path = directory.join(&relative);
            if let Some(output) = &options.output {
                if let (Ok(source), Ok(destination)) =
                    (std::fs::metadata(&path), std::fs::metadata(output))
                {
                    #[cfg(unix)]
                    let same = {
                        use std::os::unix::fs::MetadataExt;
                        source.dev() == destination.dev() && source.ino() == destination.ino()
                    };
                    #[cfg(not(unix))]
                    let same = path.canonicalize().ok() == output.canonicalize().ok();
                    if same {
                        return Err("output must not refer to a discovered source file".into());
                    }
                }
            }

            let file = sess
                .source_map
                .load_file(&path)
                .map_err(|e| format!("cannot read source '{}': {e}", path.display()))?;
            instance.vfs_mut().add_file(relative, file);
        }
    }

    let module_tree =
        resolve::build_module_tree(&sess.source_map, &instance.diag_ctx, &mut instance.vfs);
    for error in &module_tree.errors {
        error.emit(&instance.diag_ctx, source_file.start_pos);
    }
    if instance.diag_ctx.has_errors() {
        return Err(instance.diag_ctx.failure_summary("name resolution"));
    }
    let file_scope = module_tree
        .file_scopes
        .get(&file_id)
        .copied()
        .ok_or_else(|| "name resolution did not produce an input file scope".to_string())?;

    ast_lowering::set_providers(&mut instance.db.providers);
    let ast = instance
        .vfs
        .get_ast(file_id)
        .ok_or_else(|| "parsed input is unavailable for lowering".to_string())?
        .clone();
    // These pointers remain valid until the synchronous HIR query completes.
    let mut hir_input = middle::HirQueryInput::new(
        Arc::new(ast),
        file_id,
        Arc::new(module_tree),
        file_scope,
        &sess.source_map as *const rustc_span::SourceMap,
        &instance.diag_ctx as *const diagnostic::DiagnosticContext<'_>
            as *const diagnostic::DiagnosticContext<'static>,
    );
    for (id, _) in instance.vfs.files() {
        if id == file_id {
            continue;
        }
        let ast = instance
            .vfs
            .get_ast(id)
            .ok_or("discovered source was not parsed")?;
        let scope = *hir_input
            .module_tree
            .file_scopes
            .get(&id)
            .ok_or("source has no module scope")?;
        hir_input
            .additional_files
            .push((id, Arc::new(ast.clone()), scope));
    }
    instance.set_hir_input(Arc::new(hir_input));
    let package = instance.enter(|compiler| compiler.hir_package());
    if instance.diag_ctx.has_errors() {
        return Err(instance.diag_ctx.failure_summary("HIR lowering"));
    }
    if options.emit == Emit::Nir {
        let protocols = crate::protocols::type_constructors::resolve_source(
            package.package(),
            &[(0, package_identity, "input")],
            selectors,
            protocols,
        )?;
        let program = elaboration::program::elaborate_packages_with_protocols(
            package.package(),
            &std::collections::BTreeMap::from([(0, package_identity)]),
            &protocols,
        )
        .map_err(|error| format!("elaboration failed: {error}"))?;
        crate::protocols::type_constructors::validate(&program, &protocols)?;
        return Ok(Artifact::Nir(program));
    }
    Ok(Artifact::Hir(package.package().dump_to_lisp()))
}

fn source_protocols(options: &Options) -> crate::protocols::type_constructors::Selectors<'_> {
    crate::protocols::type_constructors::Selectors {
        copy: None,
        drop: None,
        index: options.index_protocol.as_deref(),
        literal_into: options.literal_into_protocol.as_deref(),
        arithmetic: std::array::from_fn(|index| options.arithmetic_protocols[index].as_deref()),
        splice: options.splice_protocol.as_deref(),
        meta_list: options.meta_list_type.as_deref(),
        tuple: options.tuple_type.as_deref(),
        optional: options.optional_type.as_deref(),
    }
}

fn compile_sysroot(
    options: &Options,
    identity: u64,
    root: &std::path::Path,
    protocols: &elaboration::program::Protocols,
    selectors: crate::protocols::type_constructors::Selectors<'_>,
) -> Result<Artifact, String> {
    use crate::packages::SourcePackage;
    let mut packages = vec![SourcePackage {
        resolution_id: 0,
        identity,
        name: "input".into(),
        entry: options.input.clone(),
    }];
    // Standalone-driver identities are local to this package set. Build tools
    // needing persistent identities use packages::compile_with_builtin instead.
    let mut next_identity = 1u64;
    for name in ["builtin", "std", "meta", "build", "attrs", "verification"] {
        let directory = root.join(name);
        if !directory.exists() && name != "builtin" && name != "std" {
            continue;
        }
        let entry = directory.join("lib.fl");
        if !entry.is_file() {
            return Err(format!(
                "sysroot package '{name}' requires '{}'",
                entry.display()
            ));
        }
        while next_identity == identity {
            next_identity += 1;
        }
        packages.push(SourcePackage {
            resolution_id: packages.len() as u32,
            identity: next_identity,
            name: name.into(),
            entry,
        });
        next_identity += 1;
    }
    let mut dependencies = Vec::new();
    for requester in &packages {
        for provider in packages.iter().skip(1) {
            if requester.resolution_id != provider.resolution_id {
                dependencies.push(resolve::DependencyEdge {
                    requester: requester.resolution_id,
                    alias: provider.name.clone(),
                    provider: provider.resolution_id,
                });
            }
        }
    }
    let defaults = if options.emit == Emit::Nir {
        crate::packages::protect_source_output(
            &root.join("language-items.conf"),
            options.output.as_deref(),
        )?;
        crate::protocols::type_constructors::SysrootBindings::load(root)?
    } else {
        Default::default()
    };
    let selectors = defaults.apply(selectors, protocols);
    crate::packages::compile_stage_with_selectors(
        &packages,
        &dependencies,
        0,
        Some(1),
        options.emit,
        options.output.as_deref(),
        protocols,
        selectors,
    )
}

pub(crate) fn discover_sources(
    root: &std::path::Path,
) -> Result<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>), String> {
    discover_sources_excluding(root, &[])
}

/// Exclusions are canonical paths explicitly owned by other supplied packages.
/// A filename such as lib.fl alone never creates a package boundary.
pub(crate) fn discover_sources_excluding(
    root: &std::path::Path,
    exclusions: &[std::path::PathBuf],
) -> Result<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>), String> {
    fn visit(
        root: &std::path::Path,
        relative: &std::path::Path,
        exclusions: &[std::path::PathBuf],
        files: &mut Vec<std::path::PathBuf>,
        directories: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), String> {
        let directory = root.join(relative);
        let mut entries = std::fs::read_dir(&directory)
            .map_err(|e| format!("cannot scan '{}': {e}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            let path = relative.join(entry.file_name());
            if !exclusions.is_empty() {
                let absolute = std::fs::canonicalize(entry.path()).map_err(|e| {
                    format!("cannot resolve source '{}': {e}", entry.path().display())
                })?;
                if exclusions.contains(&absolute) {
                    continue;
                }
            }
            if kind.is_symlink() {
                return Err(format!(
                    "source discovery does not yet support symlink entry '{}'",
                    root.join(&path).display()
                ));
            }
            if kind.is_dir() {
                directories.push(path.clone());
                visit(root, &path, exclusions, files, directories)?;
            } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "fl") {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut directories = Vec::new();
    visit(
        root,
        std::path::Path::new(""),
        exclusions,
        &mut files,
        &mut directories,
    )?;
    Ok((files, directories))
}
