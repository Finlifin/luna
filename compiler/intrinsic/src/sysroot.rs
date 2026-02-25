//! # Sysroot – compiler's pre-built standard library packages.
//!
//! The sysroot is the collection of foundational Flurry packages that are
//! automatically available to every compilation without an explicit `use`
//! declaration. It mirrors Rust's sysroot concept (`core`, `alloc`, `std`).
//!
//! ## Package Hierarchy
//!
//! ```text
//!   intrinsic (compiler-internal, not a Flurry package)
//!       ↑  provides primitive types + built-in fns
//!   builtin (Flurry source package)
//!       ↑  re-exports primitives, defines Option, Result, basic traits
//!   std (Flurry source package)
//!       ↑  re-exports builtin, adds I/O, collections, concurrency, …
//!       ↑
//!   user code
//! ```
//!
//! ## Lifecycle
//!
//! 1. [`Sysroot::discover`] locates the `library/` directory relative to
//!    the compiler binary (or via `FLURRY_SYSROOT` env var).
//! 2. [`Sysroot::load`] reads the source files into the SourceMap and
//!    creates a [`Vfs`] for each sysroot package.
//! 3. The driver (luna) parses & resolves sysroot packages **before** user
//!    code, so all standard definitions are available during name
//!    resolution.

use std::fmt;
use std::path::{Path, PathBuf};

// ── PackageId ────────────────────────────────────────────────────────────────

/// Identifies a package in the compilation graph.
///
/// The sysroot packages use well-known fixed IDs; user packages get
/// sequentially assigned IDs starting after the sysroot range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(u32);

impl PackageId {
    /// The `builtin` sysroot package (always ID 0).
    pub const BUILTIN: PackageId = PackageId(0);
    /// The `std` sysroot package (always ID 1).
    pub const STD: PackageId = PackageId(1);
    /// The first ID available for user packages.
    pub const USER_START: PackageId = PackageId(2);

    #[inline]
    pub fn new(raw: u32) -> Self {
        PackageId(raw)
    }

    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// Is this a sysroot (builtin / std) package?
    pub fn is_sysroot(self) -> bool {
        self == Self::BUILTIN || self == Self::STD
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BUILTIN => write!(f, "builtin"),
            Self::STD => write!(f, "std"),
            _ => write!(f, "pkg#{}", self.0),
        }
    }
}

// ── SysrootPackage ───────────────────────────────────────────────────────────

/// Metadata for a single sysroot package.
#[derive(Debug, Clone)]
pub struct SysrootPackage {
    /// Package identity.
    pub package_id: PackageId,
    /// Human-readable name (e.g. `"builtin"`, `"std"`).
    pub name: String,
    /// Absolute path to the package's source root directory.
    pub source_root: PathBuf,
    /// Package IDs this package depends on (in link order).
    pub deps: Vec<PackageId>,
}

// ── Sysroot ──────────────────────────────────────────────────────────────────

/// The resolved sysroot: paths to the `builtin` and `std` packages.
///
/// Created once per [`Session`](super::super::Session) and shared with
/// all compilation instances.
#[derive(Debug, Clone)]
pub struct Sysroot {
    /// Absolute path to the `library/` directory.
    pub root: PathBuf,
    /// The `builtin` package descriptor.
    pub builtin: SysrootPackage,
    /// The `std` package descriptor.
    pub std: SysrootPackage,
}

impl Sysroot {
    /// Discover the sysroot by probing well-known locations.
    ///
    /// Resolution order:
    /// 1. `$FLURRY_SYSROOT` environment variable.
    /// 2. `<compiler_binary_dir>/../library/` (relative to the luna binary).
    /// 3. `<workspace_root>/library/` (for development).
    ///
    /// Returns `None` if no valid sysroot is found.
    pub fn discover(workspace_root: &Path) -> Option<Self> {
        // 1. Environment override.
        if let Ok(env_root) = std::env::var("FLURRY_SYSROOT") {
            let p = PathBuf::from(env_root);
            if p.is_dir() {
                if let Some(sr) = Self::from_library_dir(&p) {
                    return Some(sr);
                }
            }
        }

        // 2. Relative to the compiler binary.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(bin_dir) = exe.parent() {
                let candidate = bin_dir.join("../library");
                if candidate.is_dir() {
                    if let Some(sr) = Self::from_library_dir(&candidate) {
                        return Some(sr);
                    }
                }
            }
        }

        // 3. Workspace root (development mode).
        let candidate = workspace_root.join("library");
        if candidate.is_dir() {
            return Self::from_library_dir(&candidate);
        }

        None
    }

    /// Build a [`Sysroot`] from a `library/` directory that is expected to
    /// contain `builtin/` and `std/` sub-directories.
    pub fn from_library_dir(library_dir: &Path) -> Option<Self> {
        let library_dir = library_dir
            .canonicalize()
            .unwrap_or_else(|_| library_dir.to_path_buf());

        let builtin_root = library_dir.join("builtin");
        let std_root = library_dir.join("std");

        if !builtin_root.is_dir() || !std_root.is_dir() {
            return None;
        }

        Some(Sysroot {
            root: library_dir,
            builtin: SysrootPackage {
                package_id: PackageId::BUILTIN,
                name: "builtin".into(),
                source_root: builtin_root,
                deps: vec![], // builtin has no deps (only intrinsics)
            },
            std: SysrootPackage {
                package_id: PackageId::STD,
                name: "std".into(),
                source_root: std_root,
                deps: vec![PackageId::BUILTIN], // std depends on builtin
            },
        })
    }

    /// Iterate over sysroot packages in dependency order (builtin first).
    pub fn packages(&self) -> impl Iterator<Item = &SysrootPackage> {
        [&self.builtin, &self.std].into_iter()
    }

    /// Look up a sysroot package by its [`PackageId`].
    pub fn package(&self, id: PackageId) -> Option<&SysrootPackage> {
        match id {
            PackageId::BUILTIN => Some(&self.builtin),
            PackageId::STD => Some(&self.std),
            _ => None,
        }
    }

    /// Look up a sysroot package by name.
    pub fn package_by_name(&self, name: &str) -> Option<&SysrootPackage> {
        match name {
            "builtin" => Some(&self.builtin),
            "std" => Some(&self.std),
            _ => None,
        }
    }
}
