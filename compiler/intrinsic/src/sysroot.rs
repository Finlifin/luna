//! # Sysroot – compiler's pre-built standard library packages.
//!
//! The sysroot is the collection of foundational Flurry packages shipped with
//! the compiler (`builtin`, `meta`, `build`, `attrs`, `verification`, and
//! `std`). All except `std` and the package itself are default preludes.
//!
//! ## Package Hierarchy
//!
//! ```text
//!   intrinsic (compiler-internal, not a Flurry package)
//!       ↑  provides primitive types + built-in ops
//!   builtin (Flurry source package)
//!       ↑  re-exports primitives, defines str, Tuple, resource/iteration/formatting traits
//!   meta / build / attrs / verification (Flurry source packages)
//!       ↑  comptime data/reflection, build metadata, attributes, verification baseline
//!   std (Flurry source package; explicit std.prelude)
//!       ↑  allocators, collections, String, I/O, fs, time, process, sync
//!       ↑
//!   user code
//! ```
//!
//! ## Lifecycle
//!
//! 1. [`Sysroot::discover`] locates the `library/` directory relative to
//!    the compiler binary (or via `FLURRY_SYSROOT` env var).
//! 2. The driver scans each [`SysrootPackage`] into the `SourceMap` / `Vfs`
//!    and binds `language-items.conf`.
//! 3. The driver (luna) parses & resolves sysroot packages **before** user
//!    code, so all standard definitions are available during name
//!    resolution.

use std::fmt;
use std::path::{Path, PathBuf};

/// Identifies a package in the compilation graph.
///
/// The sysroot packages use well-known fixed IDs; user packages get
/// sequentially assigned IDs starting after the sysroot range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(u32);

impl PackageId {
    /// The `builtin` sysroot package (always ID 0).
    pub const BUILTIN: PackageId = PackageId(0);

    /// The `meta` sysroot package (always ID 1).
    pub const META: PackageId = PackageId(1);

    /// The `build` sysroot package (always ID 2).
    pub const BUILD: PackageId = PackageId(2);

    /// The `attrs` sysroot package (always ID 3).
    pub const ATTRS: PackageId = PackageId(3);

    /// The `verification` sysroot package (always ID 4).
    pub const VERIFICATION: PackageId = PackageId(4);

    /// The `std` sysroot package (always ID 5).
    pub const STD: PackageId = PackageId(5);

    /// The first ID available for user packages.
    pub const USER_START: PackageId = PackageId(6);

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

    /// Is this one of the six sysroot packages?
    pub fn is_sysroot(self) -> bool {
        self.0 < Self::USER_START.0
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BUILTIN => write!(f, "builtin"),
            Self::META => write!(f, "meta"),
            Self::BUILD => write!(f, "build"),
            Self::ATTRS => write!(f, "attrs"),
            Self::VERIFICATION => write!(f, "verification"),
            Self::STD => write!(f, "std"),
            _ => write!(f, "pkg#{}", self.0),
        }
    }
}

/// Metadata for a single sysroot package.
#[derive(Debug, Clone)]
pub struct SysrootPackage {
    /// Package identity.
    pub package_id: PackageId,
    /// Human-readable name (e.g. `"builtin"`, `"meta"`, `"std"`).
    pub name: String,
    /// Absolute path to the package's source root directory.
    pub source_root: PathBuf,
    /// Package IDs this package depends on (in link order).
    pub deps: Vec<PackageId>,
}

/// The resolved sysroot: descriptors for all six builtin packages.
///
/// Created once per [`Session`](super::super::Session) and shared with
/// all compilation instances.
#[derive(Debug, Clone)]
pub struct Sysroot {
    /// Absolute path to the `library/` directory.
    pub root: PathBuf,
    /// The `builtin` package descriptor.
    pub builtin: SysrootPackage,
    /// The `meta` package descriptor.
    pub meta: SysrootPackage,
    /// The `build` package descriptor.
    pub build: SysrootPackage,
    /// The `attrs` package descriptor.
    pub attrs: SysrootPackage,
    /// The `verification` package descriptor.
    pub verification: SysrootPackage,
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

    /// Build a [`Sysroot`] from a `library/` directory that contains all six
    /// builtin packages: `builtin/`, `meta/`, `build/`, `attrs/`,
    /// `verification/`, and `std/`.
    pub fn from_library_dir(library_dir: &Path) -> Option<Self> {
        let library_dir = library_dir
            .canonicalize()
            .unwrap_or_else(|_| library_dir.to_path_buf());

        let builtin_root = library_dir.join("builtin");
        let meta_root = library_dir.join("meta");
        let build_root = library_dir.join("build");
        let attrs_root = library_dir.join("attrs");
        let verification_root = library_dir.join("verification");
        let std_root = library_dir.join("std");

        if !builtin_root.is_dir()
            || !meta_root.is_dir()
            || !build_root.is_dir()
            || !attrs_root.is_dir()
            || !verification_root.is_dir()
            || !std_root.is_dir()
        {
            return None;
        }

        Some(Sysroot {
            root: library_dir,
            builtin: SysrootPackage {
                package_id: PackageId::BUILTIN,
                name: "builtin".into(),
                source_root: builtin_root,
                deps: vec![],
            },
            meta: SysrootPackage {
                package_id: PackageId::META,
                name: "meta".into(),
                source_root: meta_root,
                deps: vec![PackageId::BUILTIN],
            },
            build: SysrootPackage {
                package_id: PackageId::BUILD,
                name: "build".into(),
                source_root: build_root,
                deps: vec![PackageId::BUILTIN, PackageId::META],
            },
            attrs: SysrootPackage {
                package_id: PackageId::ATTRS,
                name: "attrs".into(),
                source_root: attrs_root,
                deps: vec![PackageId::BUILTIN, PackageId::META],
            },
            verification: SysrootPackage {
                package_id: PackageId::VERIFICATION,
                name: "verification".into(),
                source_root: verification_root,
                deps: vec![PackageId::BUILTIN, PackageId::META],
            },
            std: SysrootPackage {
                package_id: PackageId::STD,
                name: "std".into(),
                source_root: std_root,
                deps: vec![
                    PackageId::BUILTIN,
                    PackageId::META,
                    PackageId::BUILD,
                    PackageId::ATTRS,
                    PackageId::VERIFICATION,
                ],
            },
        })
    }

    /// Iterate over all six sysroot packages in [`PackageId`] order.
    pub fn packages(&self) -> impl Iterator<Item = &SysrootPackage> {
        [
            &self.builtin,
            &self.meta,
            &self.build,
            &self.attrs,
            &self.verification,
            &self.std,
        ]
        .into_iter()
    }

    /// Look up a sysroot package by its [`PackageId`].
    pub fn package(&self, id: PackageId) -> Option<&SysrootPackage> {
        match id {
            PackageId::BUILTIN => Some(&self.builtin),
            PackageId::META => Some(&self.meta),
            PackageId::BUILD => Some(&self.build),
            PackageId::ATTRS => Some(&self.attrs),
            PackageId::VERIFICATION => Some(&self.verification),
            PackageId::STD => Some(&self.std),
            _ => None,
        }
    }

    /// Look up a sysroot package by name.
    pub fn package_by_name(&self, name: &str) -> Option<&SysrootPackage> {
        match name {
            "builtin" => Some(&self.builtin),
            "meta" => Some(&self.meta),
            "build" => Some(&self.build),
            "attrs" => Some(&self.attrs),
            "verification" => Some(&self.verification),
            "std" => Some(&self.std),
            _ => None,
        }
    }
}
