//! Compiler session – owns the foundational resources that exist for the
//! entire duration of a compilation.

use std::path::PathBuf;

use intrinsic::sysroot::Sysroot;
use rustc_span::source_map::{FilePathMapping, SourceMap};

// ── Configuration ────────────────────────────────────────────────────────────

/// Compiler configuration, typically derived from CLI flags or a project manifest.
pub struct CompilerConfig {
    /// Project / package name.
    pub name: String,
    /// Root directory of the project being compiled.
    pub root: PathBuf,
    /// Directory names to skip when scanning for sources.
    pub ignores: Vec<String>,
    /// Optional explicit sysroot path (`library/` dir). If `None` the
    /// session will try to discover it automatically.
    pub sysroot_override: Option<PathBuf>,
}

impl CompilerConfig {
    pub fn new(name: impl Into<String>, root: PathBuf) -> Self {
        CompilerConfig {
            name: name.into(),
            root,
            ignores: vec![
                ".git".into(),
                "target".into(),
                ".cache".into(),
                ".vscode".into(),
                ".idea".into(),
            ],
            sysroot_override: None,
        }
    }
}

// ── Session ──────────────────────────────────────────────────────────────────

/// The compiler session.
///
/// Holds the foundational resources that are established **before** compilation
/// begins and shared across the entire process: configuration, the source map,
/// the sysroot, etc. The `Session` is created once and then borrowed by
/// [`CompilerInstance`](super::CompilerInstance).
pub struct Session {
    /// Compiler configuration.
    pub config: CompilerConfig,
    /// Source file manager (maps byte positions to files).
    pub source_map: SourceMap,
    /// The resolved sysroot (builtin + std packages). `None` only when
    /// the sysroot could not be discovered (e.g. in bare-metal / no-std
    /// scenarios).
    pub sysroot: Option<Sysroot>,
}

impl Session {
    pub fn new(config: CompilerConfig) -> Self {
        // Try to discover the sysroot.
        let sysroot = if let Some(ref override_path) = config.sysroot_override {
            Sysroot::from_library_dir(override_path)
        } else {
            Sysroot::discover(&config.root)
        };

        Session {
            config,
            source_map: SourceMap::new(FilePathMapping::empty()),
            sysroot,
        }
    }
}
