//! Live workspace analyzer, in-memory buffer overlays, multi-package HIR lowering,
//! and LSP query providers for `anar`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use ast::Ast;
use diagnostic::{DiagnosticContext, FlurryError, Level};
use hir::{HirArena, Package};
use lex::{TokenKind, lex};
use parse::parser::Parser;
use rustc_span::source_map::{FilePathMapping, SourceMap};
use rustc_span::{BytePos, FileName, RealFileName};
use symbol::DefId;
use vfs::Vfs;

use crate::index::{
    IndexBuilder, LoweredPackageInput, SymbolIndex, SymbolInfo, SymbolKey, build_semantic_tokens,
    lexical_fallback_symbols, lexical_local_types_before, lsp_completion_kind,
    span_to_file_offsets,
};
use crate::json::JsonValue;
use crate::position::{
    LineIndex, LspPosition, LspRange, apply_content_changes, path_to_uri, uri_to_path,
};

const SYSROOT_PACKAGES: &[(&str, bool)] = &[
    ("builtin", true),
    ("meta", true),
    ("build", true),
    ("attrs", true),
    ("verification", true),
    ("std", false),
];

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    ".cache",
    ".vscode",
    ".idea",
    "node_modules",
    "eco",
];

/// LSP Diagnostic severity codes.
pub mod lsp_severity {
    pub const ERROR: u32 = 1;
    pub const WARNING: u32 = 2;
    pub const INFORMATION: u32 = 3;
    pub const HINT: u32 = 4;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedDiagInfo {
    pub file: PathBuf,
    pub range: LspRange,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: u32,
    pub code: Option<String>,
    pub message: String,
    pub related: Vec<RelatedDiagInfo>,
}

impl LspDiagnostic {
    pub fn to_json(&self) -> JsonValue {
        let mut fields = vec![
            ("range", self.range.to_json()),
            ("severity", JsonValue::from(self.severity)),
            ("source", JsonValue::from("anar")),
            ("message", JsonValue::from(self.message.as_str())),
        ];
        if let Some(ref code) = self.code {
            fields.push(("code", JsonValue::from(code.as_str())));
        }
        if !self.related.is_empty() {
            let rel: Vec<JsonValue> = self
                .related
                .iter()
                .map(|r| {
                    JsonValue::object([
                        (
                            "location",
                            JsonValue::object([
                                ("uri", JsonValue::from(path_to_uri(&r.file))),
                                ("range", r.range.to_json()),
                            ]),
                        ),
                        ("message", JsonValue::from(r.message.as_str())),
                    ])
                })
                .collect();
            fields.push(("relatedInformation", JsonValue::Array(rel)));
        }
        JsonValue::object(fields)
    }
}

/// Live document buffer tracked via `didOpen` / `didChange` / `didClose`.
#[derive(Debug, Clone)]
pub struct DocumentState {
    pub uri: String,
    pub path: PathBuf,
    pub version: i64,
    pub text: String,
    pub line_index: LineIndex,
}

impl DocumentState {
    pub fn new(uri: String, path: PathBuf, version: i64, text: String) -> Self {
        let line_index = LineIndex::new(&text);
        Self {
            uri,
            path,
            version,
            text,
            line_index,
        }
    }

    pub fn apply_changes(&mut self, version: i64, changes: &[JsonValue]) {
        apply_content_changes(&mut self.text, changes);
        self.version = version;
        self.line_index = LineIndex::new(&self.text);
    }
}

/// Cached analysis result for one package root.
#[derive(Clone)]
pub struct PackageSnapshot {
    pub pkg_root: PathBuf,
    pub index: SymbolIndex,
    pub diagnostics: HashMap<PathBuf, Vec<LspDiagnostic>>,
    pub parsed_cleanly: HashSet<PathBuf>,
}

/// Central stateful analyzer managing open buffers, package roots, sysroot, and snapshots.
pub struct WorkspaceAnalyzer {
    pub workspace_roots: Vec<PathBuf>,
    pub sysroot_override: Option<PathBuf>,
    pub open_docs: HashMap<PathBuf, DocumentState>,
    snapshots: HashMap<PathBuf, PackageSnapshot>,
    /// Last-known-good per-file symbols & occurrences when a buffer temporarily fails to parse.
    last_good_index: HashMap<PathBuf, SymbolIndex>,
    /// Emit compiler ariadne reports to stderr? Disabled during LSP stdio operation to keep logs quiet.
    pub quiet_diagnostics: bool,
}

impl Default for WorkspaceAnalyzer {
    fn default() -> Self {
        Self::new(None)
    }
}

impl WorkspaceAnalyzer {
    pub fn new(sysroot_override: Option<PathBuf>) -> Self {
        Self {
            workspace_roots: Vec::new(),
            sysroot_override,
            open_docs: HashMap::new(),
            snapshots: HashMap::new(),
            last_good_index: HashMap::new(),
            quiet_diagnostics: true,
        }
    }

    pub fn set_workspace_roots(&mut self, roots: Vec<PathBuf>) {
        self.workspace_roots = roots.into_iter().map(|p| normalize_path(&p)).collect();
    }

    pub fn set_sysroot_override(&mut self, sysroot: Option<PathBuf>) {
        self.sysroot_override = sysroot.map(|p| normalize_path(&p));
        self.snapshots.clear();
    }

    pub fn open_document(&mut self, uri: &str, version: i64, text: String) -> Option<PathBuf> {
        let raw_path = uri_to_path(uri)?;
        let path = normalize_path(&raw_path);
        let canonical_uri = path_to_uri(&path);
        self.open_docs.insert(
            path.clone(),
            DocumentState::new(canonical_uri, path.clone(), version, text),
        );
        self.reanalyze_for_file(&path);
        Some(path)
    }

    pub fn change_document(
        &mut self,
        uri: &str,
        version: i64,
        changes: &[JsonValue],
    ) -> Option<PathBuf> {
        let raw_path = uri_to_path(uri)?;
        let path = normalize_path(&raw_path);
        if let Some(doc) = self.open_docs.get_mut(&path) {
            doc.apply_changes(version, changes);
        } else if let Some(first) = changes.last().and_then(|c| c.get_str("text")) {
            let canonical_uri = path_to_uri(&path);
            self.open_docs.insert(
                path.clone(),
                DocumentState::new(canonical_uri, path.clone(), version, first.to_owned()),
            );
        }
        self.reanalyze_for_file(&path);
        Some(path)
    }

    pub fn save_document(&mut self, uri: &str, text: Option<String>) -> Option<PathBuf> {
        let raw_path = uri_to_path(uri)?;
        let path = normalize_path(&raw_path);
        if let Some(new_text) = text
            && let Some(doc) = self.open_docs.get_mut(&path)
        {
            doc.text = new_text;
            doc.line_index = LineIndex::new(&doc.text);
        }
        self.reanalyze_for_file(&path);
        Some(path)
    }

    pub fn close_document(&mut self, uri: &str) -> Option<PathBuf> {
        let raw_path = uri_to_path(uri)?;
        let path = normalize_path(&raw_path);
        self.open_docs.remove(&path);
        if path.exists() {
            self.reanalyze_for_file(&path);
        }
        Some(path)
    }

    pub fn invalidate_all(&mut self) {
        self.snapshots.clear();
        let open_paths: Vec<PathBuf> = self.open_docs.keys().cloned().collect();
        for p in open_paths {
            self.reanalyze_for_file(&p);
        }
    }

    /// Read current text and `LineIndex` for `path` (preferring live open buffer over disk).
    pub fn file_text(&self, path: &Path) -> Option<(String, LineIndex)> {
        let norm = normalize_path(path);
        if let Some(doc) = self.open_docs.get(&norm) {
            return Some((doc.text.clone(), doc.line_index.clone()));
        }
        let disk = fs::read_to_string(&norm).ok()?;
        let idx = LineIndex::new(&disk);
        Some((disk, idx))
    }

    /// Ensure the package owning `file` is analyzed and return its `PackageSnapshot`.
    pub fn ensure_snapshot(&mut self, file: &Path) -> &PackageSnapshot {
        let norm = normalize_path(file);
        let pkg_root = self.find_package_root(&norm);
        if !self.snapshots.contains_key(&pkg_root) {
            let snap = self.analyze_package(&pkg_root, Some(&norm));
            self.snapshots.insert(pkg_root.clone(), snap);
        }
        self.snapshots.get(&pkg_root).unwrap()
    }

    pub fn reanalyze_for_file(&mut self, file: &Path) -> Vec<(PathBuf, Vec<LspDiagnostic>)> {
        let norm = normalize_path(file);
        let pkg_root = self.find_package_root(&norm);
        let snap = self.analyze_package(&pkg_root, Some(&norm));
        let mut published: Vec<(PathBuf, Vec<LspDiagnostic>)> = Vec::new();

        // Collect diagnostics for every file in the package + open docs in that package.
        let mut all_files: HashSet<PathBuf> = snap.diagnostics.keys().cloned().collect();
        for f in snap.index.file_roots.keys() {
            if f.starts_with(&pkg_root) {
                all_files.insert(f.clone());
            }
        }
        all_files.insert(norm.clone());
        for f in all_files {
            if f.starts_with(&pkg_root) || f == norm {
                let diags = snap.diagnostics.get(&f).cloned().unwrap_or_default();
                published.push((f, diags));
            }
        }
        published.sort_by(|a, b| a.0.cmp(&b.0));
        self.snapshots.insert(pkg_root, snap);
        published
    }

    pub fn diagnostics_for(&mut self, file: &Path) -> Vec<LspDiagnostic> {
        let norm = normalize_path(file);
        let snap = self.ensure_snapshot(&norm);
        snap.diagnostics.get(&norm).cloned().unwrap_or_default()
    }

    /// Discover the package root directory containing `file`.
    /// Walks upwards looking for `lib.fl` or `main.fl`, stopping at workspace roots or `library/<pkg>`.
    pub fn find_package_root(&self, file: &Path) -> PathBuf {
        let start_dir = if file.is_dir() {
            file.to_path_buf()
        } else {
            file.parent().unwrap_or(Path::new(".")).to_path_buf()
        };

        let mut cur = start_dir.as_path();
        loop {
            if cur.join("lib.fl").is_file()
                || cur.join("main.fl").is_file()
                || self.open_docs.contains_key(&cur.join("lib.fl"))
                || self.open_docs.contains_key(&cur.join("main.fl"))
            {
                return cur.to_path_buf();
            }
            if self.workspace_roots.iter().any(|r| r == cur) {
                return cur.to_path_buf();
            }
            let Some(parent) = cur.parent() else {
                break;
            };
            cur = parent;
        }
        start_dir
    }

    /// Discover the `library/` sysroot directory if available.
    pub fn discover_sysroot(&self, from_pkg_root: &Path) -> Option<PathBuf> {
        if let Some(ref ov) = self.sysroot_override
            && ov.join("builtin/lib.fl").is_file()
        {
            return Some(ov.clone());
        }
        if let Ok(env_sr) = std::env::var("FLURRY_SYSROOT") {
            let p = normalize_path(Path::new(&env_sr));
            if p.join("builtin/lib.fl").is_file() {
                return Some(p);
            }
        }
        for ws in &self.workspace_roots {
            if ws.join("builtin/lib.fl").is_file() {
                return Some(ws.clone());
            }
            let cand = ws.join("library");
            if cand.join("builtin/lib.fl").is_file() {
                return Some(cand);
            }
        }
        let mut cur = Some(from_pkg_root);
        while let Some(dir) = cur {
            if dir.join("builtin/lib.fl").is_file() {
                return Some(dir.to_path_buf());
            }
            let cand = dir.join("library");
            if cand.join("builtin/lib.fl").is_file() {
                return Some(cand);
            }
            cur = dir.parent();
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut cur = exe.parent();
            while let Some(dir) = cur {
                let cand = dir.join("library");
                if cand.join("builtin/lib.fl").is_file() {
                    return Some(normalize_path(&cand));
                }
                cur = dir.parent();
            }
        }
        None
    }

    /// Run lexing, parsing, multi-package early resolution, HIR lowering, and symbol indexing!
    fn analyze_package(&mut self, pkg_root: &Path, active_file: Option<&Path>) -> PackageSnapshot {
        let source_map = SourceMap::new(FilePathMapping::empty());
        let diag_ctx = DiagnosticContext::new(&source_map);

        let sysroot_dir = self.discover_sysroot(pkg_root);

        // Check whether `pkg_root` is itself one of the sysroot packages (`library/<name>`).
        let mut user_sysroot_role: Option<(&'static str, bool)> = None;
        if let Some(ref sr) = sysroot_dir {
            for &(name, is_prelude) in SYSROOT_PACKAGES {
                if normalize_path(&sr.join(name)) == pkg_root {
                    user_sysroot_role = Some((name, is_prelude));
                    break;
                }
            }
        }

        let pkg_name = user_sysroot_role
            .map(|(n, _)| n.to_owned())
            .or_else(|| {
                pkg_root
                    .file_name()
                    .map(|n| sanitize_ident(&n.to_string_lossy()))
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("app"));

        // Collect exclusions (do not pull nested `library/` or nested subpackages with their own `lib.fl` into root!).
        let mut exclusions: Vec<PathBuf> = Vec::new();
        if let Some(ref sr) = sysroot_dir
            && sr.starts_with(pkg_root)
            && sr != pkg_root
        {
            exclusions.push(sr.clone());
        }

        let (mut user_vfs, user_clean_files, mut raw_parse_diags) =
            self.load_package_vfs(&pkg_name, pkg_root, active_file, &exclusions, &source_map);

        // Build package list: `0` = user package, `1..=N` = sysroot packages.
        let mut packages: Vec<(u32, bool, String, Vfs)> = Vec::new();
        packages.push((0, true, pkg_name.clone(), user_vfs));

        let mut builtin_id: Option<u32> = if user_sysroot_role.map(|(n, _)| n) == Some("builtin") {
            Some(0)
        } else {
            None
        };
        let mut preludes: Vec<u32> = Vec::new();
        if let Some((_, true)) = user_sysroot_role {
            preludes.push(0);
        }

        if let Some(ref sr) = sysroot_dir {
            let mut next_id = 1u32;
            for &(sr_name, is_prelude) in SYSROOT_PACKAGES {
                let sr_root = normalize_path(&sr.join(sr_name));
                if sr_root == pkg_root || !sr_root.join("lib.fl").is_file() {
                    continue;
                }
                let (sr_vfs, _, _) =
                    self.load_package_vfs(sr_name, &sr_root, None, &[], &source_map);
                let id = next_id;
                next_id += 1;
                if sr_name == "builtin" {
                    builtin_id = Some(id);
                }
                if is_prelude {
                    preludes.push(id);
                }
                packages.push((id, false, sr_name.to_owned(), sr_vfs));
            }
        }

        // Build cross-package dependency edges connecting packages to sysroot providers.
        let sysroot_providers: Vec<(u32, String)> = packages
            .iter()
            .filter_map(|(id, is_user, name, _)| {
                if !*is_user || user_sysroot_role.is_some() {
                    Some((*id, name.clone()))
                } else {
                    None
                }
            })
            .collect();

        let mut dependencies: Vec<resolve::DependencyEdge> = Vec::new();
        for (req_id, _, _, _) in &packages {
            for (prov_id, prov_name) in &sysroot_providers {
                if req_id != prov_id {
                    dependencies.push(resolve::DependencyEdge {
                        requester: *req_id,
                        alias: prov_name.clone(),
                        provider: *prov_id,
                    });
                }
            }
        }

        let resolved_res = {
            let mut refs: Vec<(u32, &mut Vfs)> = packages
                .iter_mut()
                .map(|(id, _, _, vfs)| (*id, vfs))
                .collect();
            resolve::build_package_set_with_preludes(
                &source_map,
                &diag_ctx,
                &mut refs,
                &dependencies,
                builtin_id,
                &preludes,
            )
        };

        let (module_tree, file_offsets) = match resolved_res {
            Ok(pkg_set) => (pkg_set.tree, pkg_set.file_offsets),
            Err(err) => {
                // Fallback: resolve user package standalone so one broken graph edge never kills IDE features.
                user_vfs = packages.remove(0).3;
                let tree = resolve::build_module_tree(&source_map, &diag_ctx, &mut user_vfs);
                let _ = err;
                packages.clear();
                packages.push((0, true, pkg_name.clone(), user_vfs));
                let mut fo = HashMap::new();
                fo.insert(0u32, 0u32);
                (tree, fo)
            }
        };

        // Lower all parsed ASTs across user + sysroot packages into HIR!
        let arena = HirArena::new();
        let mut lowered_hir: Vec<(u32, bool, String, Package<'_>)> =
            Vec::with_capacity(packages.len());
        let mut vfs_combined_files: Vec<(u32, vfs::FileId, PathBuf)> = Vec::new();

        for (pkg_id, is_user, name, vfs) in &packages {
            let offset = file_offsets.get(pkg_id).copied().unwrap_or(0);
            let mut hir_pkg = Package::new();

            // Lower primary entry file first (if present) so `hir_pkg.root_mod` is anchored consistently.
            let mut ordered_files: Vec<vfs::FileId> = vfs.files().map(|(fid, _)| fid).collect();
            if let Some(sel) = vfs.selected_root
                && let Some(pos) = ordered_files.iter().position(|&f| f == sel)
            {
                ordered_files.remove(pos);
                ordered_files.insert(0, sel);
            }

            let mut first_root = None;
            for local_fid in ordered_files {
                let entry = vfs.file(local_fid);
                let full_path = normalize_path(&vfs.root.join(&entry.rel_path));
                let combined_fid = vfs::FileId::from_raw(local_fid.raw() + offset);
                vfs_combined_files.push((*pkg_id, combined_fid, full_path));

                let Some(ast) = vfs.get_ast(local_fid) else {
                    continue;
                };
                let Some(&file_scope) = module_tree.file_scopes.get(&combined_fid) else {
                    continue;
                };

                // For sysroot packages, use a throwaway DiagnosticContext if we only care about user diagnostics,
                // or share `diag_ctx` for user package!
                let sysroot_silent_diag = DiagnosticContext::new(&source_map);
                let target_diag = if *is_user {
                    &diag_ctx
                } else {
                    &sysroot_silent_diag
                };
                let mut resolver = resolve::Resolver::new(&module_tree);
                ast_lowering::lower_to_hir(
                    ast,
                    combined_fid,
                    &arena,
                    &source_map,
                    target_diag,
                    &mut hir_pkg,
                    &mut resolver,
                    file_scope,
                );
                if first_root.is_none() && hir_pkg.root_mod != hir::OwnerId::INVALID {
                    first_root = Some(hir_pkg.root_mod);
                }
            }
            if let Some(root) = first_root {
                hir_pkg.root_mod = root;
            }
            lowered_hir.push((*pkg_id, *is_user, name.clone(), hir_pkg));
        }

        // Build unified `SymbolIndex`!
        let inputs: Vec<LoweredPackageInput<'_, '_>> = lowered_hir
            .iter()
            .map(|(pkg_id, is_user, name, hir_pkg)| LoweredPackageInput {
                pkg_id: *pkg_id,
                is_user_pkg: *is_user,
                pkg_name: name.as_str(),
                hir_pkg,
            })
            .collect();
        let mut index =
            IndexBuilder::new(&source_map, &module_tree).build(&inputs, &vfs_combined_files);

        // Update or merge `last_good_index` per file so broken buffers retain navigation!
        if let Some((_, _, _, user_vfs_ref)) = packages.first() {
            for (_, entry) in user_vfs_ref.files() {
                let fpath = normalize_path(&user_vfs_ref.root.join(&entry.rel_path));
                if user_clean_files.contains(&fpath) {
                    self.save_good_file_snapshot(&fpath, &index);
                } else {
                    self.merge_last_good_into(&fpath, &mut index);
                }
            }
        }

        // Collect & deduplicate diagnostics from:
        // 1) `raw_parse_diags` (captured silently without ANSI stderr spam!)
        // 2) `module_tree.errors`
        // 3) `diag_ctx.diagnostics()` (emitted by `ast_lowering` / `Resolver`)
        for r_err in &module_tree.errors {
            if let Some((fpath, (lo, hi))) = span_to_file_offsets(&source_map, r_err.span()) {
                // Only report diagnostics on files that parsed cleanly (don't cascade bogus
                // "unresolved identifier" errors caused by substituting an empty AST on syntax-broken files!).
                if user_clean_files.contains(&fpath) {
                    if let Some((text, line_idx)) = self.file_text(&fpath) {
                        let range = line_idx.span_to_range(&text, lo, hi);
                        push_unique_diag(
                            &mut raw_parse_diags,
                            fpath,
                            LspDiagnostic {
                                range,
                                severity: lsp_severity::ERROR,
                                code: Some(format!("E{:04}", r_err.error_code())),
                                message: r_err.message(),
                                related: Vec::new(),
                            },
                        );
                    }
                }
            }
        }

        for d in diag_ctx.diagnostics().iter() {
            let primary = d
                .primary_span
                .or_else(|| d.labels.first().map(|l| l.span))
                .unwrap_or(rustc_span::DUMMY_SP);
            let Some((fpath, (lo, hi))) = span_to_file_offsets(&source_map, primary) else {
                continue;
            };
            if !fpath.starts_with(pkg_root) && Some(fpath.as_path()) != active_file {
                continue;
            }
            if !user_clean_files.contains(&fpath) {
                continue;
            }
            let Some((text, line_idx)) = self.file_text(&fpath) else {
                continue;
            };
            let severity = match d.level {
                Level::Error => lsp_severity::ERROR,
                Level::Warning => lsp_severity::WARNING,
                Level::Note => lsp_severity::INFORMATION,
                Level::Help => lsp_severity::HINT,
            };
            let mut message = d.message.clone();
            for note in &d.notes {
                message.push_str(&format!("\nnote: {note}"));
            }
            for help in &d.helps {
                message.push_str(&format!("\nhelp: {help}"));
            }
            let mut related = Vec::new();
            for lbl in &d.labels {
                if Some(lbl.span) != Some(primary)
                    && let Some((lf, (llo, lhi))) = span_to_file_offsets(&source_map, lbl.span)
                    && let Some((lt, lidx)) = self.file_text(&lf)
                {
                    related.push(RelatedDiagInfo {
                        file: lf,
                        range: lidx.span_to_range(&lt, llo, lhi),
                        message: lbl.message.clone(),
                    });
                }
            }
            push_unique_diag(
                &mut raw_parse_diags,
                fpath,
                LspDiagnostic {
                    range: line_idx.span_to_range(&text, lo, hi),
                    severity,
                    code: d.code.map(|c| format!("E{c:04}")),
                    message,
                    related,
                },
            );
        }

        PackageSnapshot {
            pkg_root: pkg_root.to_path_buf(),
            index,
            diagnostics: raw_parse_diags,
            parsed_cleanly: user_clean_files,
        }
    }

    /// Load all `.fl` files for a package (preferring open document overlays over disk),
    /// pre-parsing each file so syntax errors in one file never abort scanning of sibling files!
    fn load_package_vfs(
        &self,
        pkg_name: &str,
        pkg_root: &Path,
        active_file: Option<&Path>,
        exclusions: &[PathBuf],
        source_map: &SourceMap,
    ) -> (Vfs, HashSet<PathBuf>, HashMap<PathBuf, Vec<LspDiagnostic>>) {
        let mut vfs = Vfs::new(pkg_name, pkg_root.to_path_buf());
        let (mut rel_files, dirs) = discover_package_files(pkg_root, exclusions);
        vfs.source_directories = dirs;

        // Also include any open `.fl` buffer inside `pkg_root` that hasn't been saved to disk yet!
        for open_path in self.open_docs.keys() {
            if let Ok(rel) = open_path.strip_prefix(pkg_root)
                && !rel.as_os_str().is_empty()
                && !exclusions.iter().any(|ex| open_path.starts_with(ex))
                && !rel_files.contains(&rel.to_path_buf())
            {
                rel_files.push(rel.to_path_buf());
            }
        }
        if let Some(act) = active_file
            && let Ok(rel) = act.strip_prefix(pkg_root)
            && !rel.as_os_str().is_empty()
            && !rel_files.contains(&rel.to_path_buf())
        {
            rel_files.push(rel.to_path_buf());
        }
        rel_files.sort();
        rel_files.dedup();

        // Avoid `"multiple source bodies for one module"` if a directory has BOTH `lib.fl` and `main.fl`!
        let has_lib = rel_files.iter().any(|p| p == Path::new("lib.fl"));
        let has_main = rel_files.iter().any(|p| p == Path::new("main.fl"));
        let active_rel = active_file.and_then(|a| a.strip_prefix(pkg_root).ok());
        if has_lib && has_main {
            if active_rel == Some(Path::new("main.fl")) {
                rel_files.retain(|p| p != Path::new("lib.fl"));
            } else {
                rel_files.retain(|p| p != Path::new("main.fl"));
            }
        }

        let mut clean_files = HashSet::new();
        let mut parse_diags: HashMap<PathBuf, Vec<LspDiagnostic>> = HashMap::new();

        for rel in rel_files {
            let abs = normalize_path(&pkg_root.join(&rel));
            let content = if let Some(doc) = self.open_docs.get(&abs) {
                doc.text.clone()
            } else if let Ok(disk) = fs::read_to_string(&abs) {
                disk
            } else {
                continue;
            };

            let sf = source_map.new_source_file(
                FileName::Real(RealFileName::LocalPath(abs.clone())),
                content.clone(),
            );
            let fid = vfs.add_file(rel.clone(), sf.clone());

            if rel == Path::new("lib.fl") || rel == Path::new("main.fl") {
                vfs.selected_root = Some(fid);
            }

            // Lex & parse without letting a single broken file abort `VfsScanner::scan_package`!
            let line_idx = LineIndex::new(&content);
            let (tokens, symbols, lex_errors) = lex(&content, sf.start_pos);
            let mut file_ok = lex_errors.is_empty();

            for lerr in &lex_errors {
                let (lo, hi) = lex_error_offsets(lerr, &content);
                push_unique_diag(
                    &mut parse_diags,
                    abs.clone(),
                    LspDiagnostic {
                        range: line_idx.span_to_range(&content, lo, hi),
                        severity: lsp_severity::ERROR,
                        code: Some(format!("E{:04}", lerr.error_code())),
                        message: format!("{lerr:?}"),
                        related: Vec::new(),
                    },
                );
            }

            let parse_diag_ctx = DiagnosticContext::new(source_map);
            let mut parser = Parser::new(source_map, tokens, symbols, sf.start_pos);
            let parsed = parser.parse(&parse_diag_ctx);
            if !parsed {
                file_ok = false;
            }

            for d in parse_diag_ctx.diagnostics().iter() {
                let primary = d
                    .primary_span
                    .or_else(|| d.labels.first().map(|l| l.span))
                    .unwrap_or(rustc_span::DUMMY_SP);
                let (lo, hi) = span_to_file_offsets(source_map, primary)
                    .map(|(_, s)| s)
                    .unwrap_or((0, 0));
                push_unique_diag(
                    &mut parse_diags,
                    abs.clone(),
                    LspDiagnostic {
                        range: line_idx.span_to_range(&content, lo, hi),
                        severity: lsp_severity::ERROR,
                        code: d.code.map(|c| format!("E{c:04}")),
                        message: d.message.clone(),
                        related: Vec::new(),
                    },
                );
            }

            if file_ok {
                let ast = parser.finalize();
                if ast.validate_structure().is_ok() {
                    clean_files.insert(abs);
                    vfs.set_ast(fid, ast);
                    continue;
                }
            }

            // Substitute a valid empty AST for this broken file so sibling files & sysroot still resolve!
            let empty_ast = build_empty_ast(source_map, sf.start_pos);
            vfs.set_ast(fid, empty_ast);
        }

        // If a standalone single-file directory has no `lib.fl`/`main.fl`, mark that single file as root.
        if vfs.selected_root.is_none() && vfs.file_count() == 1 {
            vfs.selected_root = Some(vfs::FileId::from_raw(0));
        }

        (vfs, clean_files, parse_diags)
    }

    fn save_good_file_snapshot(&mut self, file: &Path, index: &SymbolIndex) {
        let mut sub = SymbolIndex::default();
        if let Some(roots) = index.file_roots.get(file) {
            sub.file_roots.insert(file.to_path_buf(), roots.clone());
        }
        if let Some(occs) = index.file_occurrences.get(file) {
            sub.file_occurrences
                .insert(file.to_path_buf(), occs.clone());
            for occ in occs {
                if let Some(sym) = index.symbols.get(&occ.target) {
                    sub.symbols.insert(occ.target, sym.clone());
                }
            }
        }
        for (k, sym) in &index.symbols {
            if sym.file == file {
                sub.symbols.insert(*k, sym.clone());
            }
        }
        for (nom, members) in &index.nominal_members {
            let kept: Vec<_> = members
                .iter()
                .filter(|m| index.symbols.get(&m.key).is_some_and(|s| s.file == file))
                .cloned()
                .collect();
            if !kept.is_empty() {
                sub.nominal_members.insert(*nom, kept);
            }
        }
        if let Some(vis) = index.file_visible_defs.get(file) {
            sub.file_visible_defs
                .insert(file.to_path_buf(), vis.clone());
        }
        self.last_good_index.insert(file.to_path_buf(), sub);
    }

    fn merge_last_good_into(&self, file: &Path, index: &mut SymbolIndex) {
        let Some(good) = self.last_good_index.get(file) else {
            return;
        };
        for (k, sym) in &good.symbols {
            index.symbols.entry(*k).or_insert_with(|| sym.clone());
            if let SymbolKey::Def(def_id) = k {
                let entry = index.defs_by_name.entry(sym.name.clone()).or_default();
                if !entry.contains(def_id) {
                    entry.insert(0, *def_id);
                }
            }
        }
        if !index.file_roots.contains_key(file)
            && let Some(roots) = good.file_roots.get(file)
        {
            index.file_roots.insert(file.to_path_buf(), roots.clone());
        }
        if !index.file_occurrences.contains_key(file)
            && let Some(occs) = good.file_occurrences.get(file)
        {
            index
                .file_occurrences
                .insert(file.to_path_buf(), occs.clone());
        }
        for (nom, members) in &good.nominal_members {
            let dst = index.nominal_members.entry(*nom).or_default();
            for m in members {
                if !dst.iter().any(|x| x.name == m.name) {
                    dst.push(m.clone());
                }
            }
        }
    }

    // ── LSP Feature Queries ─────────────────────────────────────────────────

    pub fn goto_definition(&mut self, file: &Path, pos: LspPosition) -> Vec<JsonValue> {
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Vec::new();
        };
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();

        if let Some(occ) = snap.index.occurrence_at(&norm, offset)
            && let Some(sym) = snap.index.symbol(occ.target)
            && let Some(loc) = self.symbol_location_json(sym)
        {
            return vec![loc];
        }

        // Lexical fallback when cursor is on `receiver.member` or `Ident` in an unparsed edit!
        if let Some((recv, member)) = dot_access_at(&text, offset) {
            for sym in self.lookup_dot_members(&snap, &text, offset, &recv) {
                if sym.name == member
                    && let Some(loc) = self.symbol_location_json(&sym)
                {
                    return vec![loc];
                }
            }
        }
        if let Some((word, _)) = word_at(&text, offset)
            && let Some(defs) = snap.index.defs_by_name.get(&word)
        {
            let mut locs = Vec::new();
            for &d in defs {
                if let Some(sym) = snap.index.symbol(SymbolKey::Def(d))
                    && let Some(loc) = self.symbol_location_json(sym)
                {
                    locs.push(loc);
                    break;
                }
            }
            return locs;
        }
        Vec::new()
    }

    pub fn find_references(
        &mut self,
        file: &Path,
        pos: LspPosition,
        include_decl: bool,
    ) -> Vec<JsonValue> {
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Vec::new();
        };
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();
        let Some(occ) = snap.index.occurrence_at(&norm, offset) else {
            return Vec::new();
        };
        let refs = snap.index.references_to(occ.target, include_decl);
        let mut out = Vec::with_capacity(refs.len());
        for (f, (lo, hi)) in refs {
            if let Some((ftext, flines)) = self.file_text(&f) {
                out.push(JsonValue::object([
                    ("uri", JsonValue::from(path_to_uri(&f))),
                    ("range", flines.span_to_range(&ftext, lo, hi).to_json()),
                ]));
            }
        }
        out
    }

    pub fn document_highlights(&mut self, file: &Path, pos: LspPosition) -> Vec<JsonValue> {
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Vec::new();
        };
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();
        let Some(occ) = snap.index.occurrence_at(&norm, offset) else {
            return Vec::new();
        };
        // LSP DocumentHighlightKind: Text = 1, Read = 2, Write = 3
        snap.index
            .highlights_in_file(&norm, occ.target)
            .into_iter()
            .map(|h| {
                let kind = if h.is_write { 3u32 } else { 2u32 };
                JsonValue::object([
                    (
                        "range",
                        line_idx.span_to_range(&text, h.span.0, h.span.1).to_json(),
                    ),
                    ("kind", JsonValue::from(kind)),
                ])
            })
            .collect()
    }

    pub fn hover(&mut self, file: &Path, pos: LspPosition) -> Option<JsonValue> {
        let norm = normalize_path(file);
        let (text, line_idx) = self.file_text(&norm)?;
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();

        if let Some(occ) = snap.index.occurrence_at(&norm, offset)
            && let Some(sym) = snap.index.symbol(occ.target)
        {
            let range = line_idx.span_to_range(&text, occ.span.0, occ.span.1);
            return Some(build_hover_json(sym, Some(range)));
        }

        if let Some((recv, member)) = dot_access_at(&text, offset) {
            for sym in self.lookup_dot_members(&snap, &text, offset, &recv) {
                if sym.name == member {
                    let range = word_at(&text, offset)
                        .map(|(_, (lo, hi))| line_idx.span_to_range(&text, lo, hi));
                    return Some(build_hover_json(&sym, range));
                }
            }
        }

        // Hovering built-in primitives or keywords (`Type`, `noreturn`, `void`, `any`, `Self`)!
        let (word, (lo, hi)) = word_at(&text, offset)?;
        if let Some(defs) = snap.index.defs_by_name.get(&word)
            && let Some(&d) = defs.first()
            && let Some(sym) = snap.index.symbol(SymbolKey::Def(d))
        {
            return Some(build_hover_json(
                sym,
                Some(line_idx.span_to_range(&text, lo, hi)),
            ));
        }

        let builtin_doc = match word.as_str() {
            "Type" => Some(("Type", "Universe of runtime and comptime types in Flurry.")),
            "void" => Some(("void", "Unit / empty result type.")),
            "noreturn" => Some(("noreturn", "Bottom type of diverging computations.")),
            "any" => Some(("any", "Top existential / erased type.")),
            "Self" => Some(("Self", "Enclosing nominal or trait implementing type.")),
            _ => None,
        };
        if let Some((sig, doc)) = builtin_doc {
            let md = format!("```flurry\n{sig}\n```\n---\n{doc}");
            return Some(JsonValue::object([
                (
                    "contents",
                    JsonValue::object([
                        ("kind", JsonValue::from("markdown")),
                        ("value", JsonValue::from(md)),
                    ]),
                ),
                ("range", line_idx.span_to_range(&text, lo, hi).to_json()),
            ]));
        }
        None
    }

    pub fn complete(&mut self, file: &Path, pos: LspPosition) -> Vec<JsonValue> {
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Vec::new();
        };
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();

        let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_prefix = &text[line_start..offset];

        // 1. `--%` formatter directive completion inside comments!
        if let Some(after_dir) = line_prefix.trim_start().strip_prefix("--%") {
            return complete_fmt_directives(after_dir.trim());
        }

        // 2. Postfix / member / module access after `.`: e.g. `p.`, `p.no`, `std.io.`!
        if let Some((receiver_chain, partial)) = dot_completion_context(line_prefix) {
            let mut items = Vec::new();
            let mut seen = HashSet::new();

            if !receiver_chain.is_empty() {
                for sym in self.lookup_dot_members(&snap, &text, offset, &receiver_chain) {
                    if sym.name.starts_with(&partial) && seen.insert(sym.name.clone()) {
                        items.push(symbol_to_completion_item(&sym, "0"));
                    }
                }
                // Flurry postfix operators (`docs/grammar/syntax`): `.ref`, `.*`, `.as`, `.dyn`, `.use`, `.await`!
                for (op, detail, insert) in [
                    ("ref", "Take address / reference (.ref)", "ref"),
                    ("*", "Dereference pointer (.*)", "*"),
                    ("as", "Explicit conversion (.as<T>)", "as"),
                    ("dyn", "Dynamic trait coercion (.dyn<Trait>)", "dyn"),
                    ("use", "Explicit capability/value use (.use)", "use"),
                    ("await", "Await async computation (.await)", "await"),
                ] {
                    if op.starts_with(&partial) && seen.insert(op.to_owned()) {
                        items.push(JsonValue::object([
                            ("label", JsonValue::from(op)),
                            ("kind", JsonValue::from(lsp_completion_kind::KEYWORD)),
                            ("detail", JsonValue::from(detail)),
                            ("insertText", JsonValue::from(insert)),
                            ("sortText", JsonValue::from(format!("1_{op}"))),
                        ]));
                    }
                }
            }
            return items;
        }

        // 3. Take-view completion after `'`: e.g. `expr'bit_vec`!
        if let Some(view_partial) = quote_view_context(line_prefix) {
            return [
                ("bit_vec", "Bit-vector view ('bit_vec)"),
                ("tag_name", "Enum tag name view ('tag_name)"),
                ("old", "Pre-state value in ensures ('old)"),
                ("intrinsic", "Compiler intrinsic view ('intrinsic)"),
                (
                    "tuple_enumerate",
                    "Structural tuple enumeration ('tuple_enumerate)",
                ),
            ]
            .into_iter()
            .filter(|(label, _)| label.starts_with(view_partial))
            .map(|(label, detail)| {
                JsonValue::object([
                    ("label", JsonValue::from(label)),
                    ("kind", JsonValue::from(lsp_completion_kind::PROPERTY)),
                    ("detail", JsonValue::from(detail)),
                ])
            })
            .collect();
        }

        // 4. General lexical + module + prelude + keyword completion!
        let partial = identifier_prefix(line_prefix);
        let mut items = Vec::new();
        let mut seen = HashSet::new();

        // 4a. Local symbols declared before `offset` in the same file.
        for sym in snap.index.symbols.values() {
            if matches!(sym.key, SymbolKey::Local { .. })
                && sym.file == norm
                && sym.selection_span.0 <= offset
                && sym.name.starts_with(partial)
                && seen.insert(sym.name.clone())
            {
                items.push(symbol_to_completion_item(sym, "0"));
            }
        }

        // Also include lexically scanned locals from the current unparsed edit!
        for (lname, lty) in lexical_local_types_before(&text, offset) {
            if lname.starts_with(partial) && seen.insert(lname.clone()) {
                items.push(JsonValue::object([
                    ("label", JsonValue::from(lname.as_str())),
                    ("kind", JsonValue::from(lsp_completion_kind::VARIABLE)),
                    ("detail", JsonValue::from(format!("let {lname}: {lty}"))),
                    ("sortText", JsonValue::from(format!("0_{lname}"))),
                ]));
            }
        }

        // 4b. File-visible definitions (current module, ancestor modules, `use` imports, preludes).
        if let Some(vis_list) = snap.index.file_visible_defs.get(&norm) {
            for (vname, key) in vis_list {
                if vname.starts_with(partial)
                    && seen.insert(vname.clone())
                    && let Some(sym) = snap.index.symbol(*key)
                {
                    let prio = if sym.is_user_pkg { "1" } else { "2" };
                    items.push(symbol_to_completion_item(sym, prio));
                }
            }
        }

        // 4c. Same-package top-level definitions (helpful when `file_visible_defs` fell back on broken file).
        for sym in snap.index.symbols.values() {
            if sym.is_user_pkg
                && matches!(sym.key, SymbolKey::Def(_))
                && !sym.name.starts_with("impl ")
                && !sym.name.starts_with("derive ")
                && !sym.name.starts_with("test")
                && sym.name.starts_with(partial)
                && seen.insert(sym.name.clone())
            {
                items.push(symbol_to_completion_item(sym, "1"));
            }
        }

        // 4d. Well-known package roots (`builtin`, `meta`, `build`, `attrs`, `verification`, `std`).
        for &(pkg_kw, _) in SYSROOT_PACKAGES {
            if pkg_kw.starts_with(partial) && seen.insert(pkg_kw.to_owned()) {
                items.push(JsonValue::object([
                    ("label", JsonValue::from(pkg_kw)),
                    ("kind", JsonValue::from(lsp_completion_kind::MODULE)),
                    (
                        "detail",
                        JsonValue::from(format!("sysroot package {pkg_kw}")),
                    ),
                    ("sortText", JsonValue::from(format!("2_{pkg_kw}"))),
                ]));
            }
        }

        // 4e. Flurry keywords & built-in universe types.
        for kw in FLURRY_KEYWORDS {
            if kw.starts_with(partial) && seen.insert((*kw).to_owned()) {
                items.push(JsonValue::object([
                    ("label", JsonValue::from(*kw)),
                    ("kind", JsonValue::from(lsp_completion_kind::KEYWORD)),
                    ("sortText", JsonValue::from(format!("3_{kw}"))),
                ]));
            }
        }

        items
    }

    fn lookup_dot_members(
        &self,
        snap: &PackageSnapshot,
        src: &str,
        cursor_offset: usize,
        receiver_chain: &str,
    ) -> Vec<SymbolInfo> {
        let segments: Vec<&str> = receiver_chain
            .split('.')
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "*" && *s != "ref")
            .collect();
        if segments.is_empty() {
            return Vec::new();
        }

        // Resolve first segment:
        // 1) Local variable / parameter at cursor (`local_type_hints` or `lexical_local_types_before`)
        // 2) Nominal type / module in `defs_by_name` (including sysroot packages `std`, `builtin`, `meta`)
        let first = segments[0];
        let mut candidate_defs: Vec<DefId> = Vec::new();

        // Check HIR local occurrences in this file declared before `cursor_offset`.
        for sym in snap.index.symbols.values() {
            if sym.name == first
                && sym.selection_span.0 <= cursor_offset
                && let Some(&nom_def) = snap.index.local_type_hints.get(&sym.key)
            {
                candidate_defs.push(nom_def);
            }
        }

        // Check token-level local type declarations (works even when the current edit has a syntax error!).
        let lex_locals = lexical_local_types_before(src, cursor_offset);
        if let Some(ty_name) = lex_locals.get(first)
            && let Some(defs) = snap.index.defs_by_name.get(ty_name)
        {
            for &d in defs {
                if !candidate_defs.contains(&d) {
                    candidate_defs.push(d);
                }
            }
        }

        // Check direct module / nominal type name (`Point`, `std`, `builtin`, `meta`, etc.).
        if let Some(defs) = snap.index.defs_by_name.get(first) {
            for &d in defs {
                if !candidate_defs.contains(&d) {
                    candidate_defs.push(d);
                }
            }
        }

        // Walk remaining chain segments (`a.b.c.`).
        for &seg in &segments[1..] {
            let mut next_defs = Vec::new();
            for cur_def in candidate_defs {
                for m in snap.index.members_of(cur_def) {
                    if m.name == seg {
                        match m.key {
                            SymbolKey::Def(d) => next_defs.push(d),
                            SymbolKey::Field { .. } => {
                                if let Some(fsym) = snap.index.symbol(m.key)
                                    && let Some(colon) = fsym.detail.find(':')
                                {
                                    let tname = fsym.detail[colon + 1..]
                                        .split('=')
                                        .next()
                                        .unwrap_or("")
                                        .trim()
                                        .trim_start_matches(['*', '?'])
                                        .split(['<', ' ', '('])
                                        .next()
                                        .unwrap_or("")
                                        .rsplit('.')
                                        .next()
                                        .unwrap_or("");
                                    if let Some(defs) = snap.index.defs_by_name.get(tname) {
                                        next_defs.extend(defs.iter().copied());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            candidate_defs = next_defs;
        }

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for d in candidate_defs {
            for m in snap.index.members_of(d) {
                if seen.insert((m.name.clone(), m.key))
                    && let Some(sym) = snap.index.symbol(m.key)
                {
                    out.push(sym.clone());
                }
            }
        }
        out
    }

    pub fn signature_help(&mut self, file: &Path, pos: LspPosition) -> Option<JsonValue> {
        let norm = normalize_path(file);
        let (text, line_idx) = self.file_text(&norm)?;
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();

        let (callee_span, active_param) = enclosing_call_context(&text, offset)?;
        let callee_text = text[callee_span.0..callee_span.1].trim();

        // Try exact HIR occurrence on the callee end first!
        let mut target_sym: Option<SymbolInfo> = snap
            .index
            .occurrence_at(&norm, callee_span.1.saturating_sub(1))
            .and_then(|occ| snap.index.symbol(occ.target))
            .cloned();

        // Fallback to member or name lookup!
        if target_sym.is_none() {
            if let Some((recv, method)) = callee_text.rsplit_once('.') {
                target_sym = self
                    .lookup_dot_members(&snap, &text, callee_span.0, recv.trim())
                    .into_iter()
                    .find(|s| s.name == method.trim());
            } else if let Some(defs) = snap.index.defs_by_name.get(callee_text) {
                target_sym = defs
                    .iter()
                    .filter_map(|&d| snap.index.symbol(SymbolKey::Def(d)))
                    .find(|s| !s.params.is_empty() || s.detail.contains("fn "))
                    .cloned();
            }
        }

        let sym = target_sym?;
        // For method calls `recv.method(arg0, ...)`, strip leading `self` / `*self` parameter!
        let is_method_call = callee_text.contains('.');
        let skip_self = usize::from(
            is_method_call
                && sym
                    .params
                    .first()
                    .is_some_and(|p| p.label == "self" || p.label == "*self"),
        );
        let visible_params: Vec<JsonValue> = sym
            .params
            .iter()
            .skip(skip_self)
            .map(|p| JsonValue::object([("label", JsonValue::from(p.label.as_str()))]))
            .collect();

        let mut sig_obj = vec![
            ("label", JsonValue::from(sym.detail.as_str())),
            ("parameters", JsonValue::Array(visible_params)),
        ];
        if let Some(ref doc) = sym.doc {
            sig_obj.push((
                "documentation",
                JsonValue::object([
                    ("kind", JsonValue::from("markdown")),
                    ("value", JsonValue::from(doc.as_str())),
                ]),
            ));
        }

        Some(JsonValue::object([
            (
                "signatures",
                JsonValue::Array(vec![JsonValue::object(sig_obj)]),
            ),
            ("activeSignature", JsonValue::from(0u32)),
            ("activeParameter", JsonValue::from(active_param as u32)),
        ]))
    }

    pub fn document_symbols(&mut self, file: &Path) -> Vec<JsonValue> {
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Vec::new();
        };
        let snap = self.ensure_snapshot(&norm).clone();

        if let Some(roots) = snap.index.file_roots.get(&norm)
            && !roots.is_empty()
        {
            return roots
                .iter()
                .filter_map(|&k| build_document_symbol_json(k, &snap.index, &text, &line_idx))
                .collect();
        }

        // Lexical fallback outline if the file has never parsed cleanly yet!
        lexical_fallback_symbols(&norm, &text)
            .into_iter()
            .map(|sym| {
                let range = line_idx.span_to_range(&text, sym.full_span.0, sym.full_span.1);
                let sel = line_idx.span_to_range(&text, sym.selection_span.0, sym.selection_span.1);
                JsonValue::object([
                    ("name", JsonValue::from(sym.name)),
                    ("detail", JsonValue::from(sym.detail)),
                    ("kind", JsonValue::from(sym.kind)),
                    ("range", range.to_json()),
                    ("selectionRange", sel.to_json()),
                ])
            })
            .collect()
    }

    pub fn workspace_symbols(&mut self, query: &str) -> Vec<JsonValue> {
        // Ensure all workspace roots and open documents are indexed.
        let roots = self.workspace_roots.clone();
        for r in roots {
            if r.join("lib.fl").is_file() || r.join("main.fl").is_file() {
                let _ = self.ensure_snapshot(&r);
            }
        }
        let open_files: Vec<PathBuf> = self.open_docs.keys().cloned().collect();
        for f in open_files {
            let _ = self.ensure_snapshot(&f);
        }

        let q_lower = query.to_ascii_lowercase();
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        let snaps: Vec<PackageSnapshot> = self.snapshots.values().cloned().collect();
        for snap in snaps {
            for sym in snap.index.symbols.values() {
                if matches!(sym.key, SymbolKey::Local { .. }) {
                    continue;
                }
                if !q_lower.is_empty() && !sym.name.to_ascii_lowercase().contains(&q_lower) {
                    continue;
                }
                // Prioritize user packages; only include sysroot symbols when a non-empty query matches!
                if !sym.is_user_pkg && q_lower.is_empty() {
                    continue;
                }
                if !seen.insert((sym.file.clone(), sym.selection_span, sym.name.clone())) {
                    continue;
                }
                if let Some(loc) = self.symbol_location_json(sym) {
                    let mut fields = vec![
                        ("name", JsonValue::from(sym.name.as_str())),
                        ("kind", JsonValue::from(sym.kind)),
                        ("location", loc),
                    ];
                    if let Some(ref c) = sym.container_name {
                        fields.push(("containerName", JsonValue::from(c.as_str())));
                    }
                    results.push((
                        !sym.is_user_pkg,
                        sym.name.clone(),
                        JsonValue::object(fields),
                    ));
                }
                if results.len() >= 250 {
                    break;
                }
            }
        }
        results.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        results.into_iter().map(|(_, _, j)| j).collect()
    }

    pub fn prepare_rename(&mut self, file: &Path, pos: LspPosition) -> Option<JsonValue> {
        let norm = normalize_path(file);
        let (text, line_idx) = self.file_text(&norm)?;
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();
        let occ = snap.index.occurrence_at(&norm, offset)?;
        let sym = snap.index.symbol(occ.target)?;
        if !sym.is_user_pkg || sym.name == "self" || sym.name == "Self" {
            return None;
        }
        let range = line_idx.span_to_range(&text, occ.span.0, occ.span.1);
        Some(JsonValue::object([
            ("range", range.to_json()),
            ("placeholder", JsonValue::from(sym.name.as_str())),
        ]))
    }

    pub fn rename(
        &mut self,
        file: &Path,
        pos: LspPosition,
        new_name: &str,
    ) -> Result<JsonValue, String> {
        if !is_valid_flurry_ident(new_name) {
            return Err(format!("`{new_name}` is not a valid Flurry identifier"));
        }
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Err(String::from("file content unavailable"));
        };
        let offset = line_idx.position_to_offset(&text, pos);
        let snap = self.ensure_snapshot(&norm).clone();
        let Some(occ) = snap.index.occurrence_at(&norm, offset) else {
            return Err(String::from("no symbol at cursor position"));
        };
        let Some(sym) = snap.index.symbol(occ.target) else {
            return Err(String::from("symbol definition unavailable"));
        };
        if !sym.is_user_pkg {
            return Err(String::from("cannot rename sysroot library definitions"));
        }

        let refs = snap.index.references_to(occ.target, true);
        let mut by_uri: BTreeMap<String, Vec<JsonValue>> = BTreeMap::new();
        for (fpath, (lo, hi)) in refs {
            if let Some((ftext, flines)) = self.file_text(&fpath) {
                let uri = path_to_uri(&fpath);
                by_uri.entry(uri).or_default().push(JsonValue::object([
                    ("range", flines.span_to_range(&ftext, lo, hi).to_json()),
                    ("newText", JsonValue::from(new_name)),
                ]));
            }
        }
        let changes = JsonValue::Object(
            by_uri
                .into_iter()
                .map(|(k, v)| (k, JsonValue::Array(v)))
                .collect(),
        );
        Ok(JsonValue::object([("changes", changes)]))
    }

    pub fn folding_ranges(&self, file: &Path) -> Vec<JsonValue> {
        let Some((text, line_idx)) = self.file_text(file) else {
            return Vec::new();
        };
        let (tokens, _, _) = lex(&text, BytePos(0));
        let mut ranges = Vec::new();
        let mut brace_stack: Vec<(TokenKind, u32)> = Vec::new();

        for tok in &tokens {
            if tok.to <= tok.from || tok.to > text.len() {
                continue;
            }
            let start_line = line_idx.offset_to_position(&text, tok.from).line;
            let end_line = line_idx
                .offset_to_position(&text, tok.to.saturating_sub(1))
                .line;
            match tok.kind {
                TokenKind::Comment if end_line > start_line => {
                    ranges.push(JsonValue::object([
                        ("startLine", JsonValue::from(start_line)),
                        ("endLine", JsonValue::from(end_line)),
                        ("kind", JsonValue::from("comment")),
                    ]));
                }
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => {
                    brace_stack.push((tok.kind, start_line));
                }
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                    let expected = match tok.kind {
                        TokenKind::RBrace => TokenKind::LBrace,
                        TokenKind::RParen => TokenKind::LParen,
                        _ => TokenKind::LBracket,
                    };
                    if let Some(pos) = brace_stack.iter().rposition(|(k, _)| *k == expected) {
                        let (_, open_line) = brace_stack.remove(pos);
                        if end_line > open_line {
                            ranges.push(JsonValue::object([
                                ("startLine", JsonValue::from(open_line)),
                                (
                                    "endLine",
                                    JsonValue::from(end_line.saturating_sub(1).max(open_line)),
                                ),
                                ("kind", JsonValue::from("region")),
                            ]));
                        }
                    }
                }
                _ => {}
            }
        }
        ranges
    }

    pub fn semantic_tokens(&mut self, file: &Path) -> Vec<u32> {
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Vec::new();
        };
        let snap = self.ensure_snapshot(&norm).clone();
        build_semantic_tokens(&text, &line_idx, &norm, Some(&snap.index))
    }

    pub fn format_document(&self, file: &Path) -> Result<Vec<JsonValue>, String> {
        let norm = normalize_path(file);
        let Some((text, line_idx)) = self.file_text(&norm) else {
            return Ok(Vec::new());
        };
        let config = fmt::FormatConfig::discover(norm.parent().unwrap_or(Path::new(".")), None)
            .unwrap_or_default();
        let res = fmt::format_source(&text, &norm.to_string_lossy(), &config, false)
            .map_err(|e| e.to_string())?;
        if res.formatted == text {
            return Ok(Vec::new());
        }
        Ok(vec![JsonValue::object([
            ("range", line_idx.full_range(&text).to_json()),
            ("newText", JsonValue::from(res.formatted)),
        ])])
    }

    fn symbol_location_json(&self, sym: &SymbolInfo) -> Option<JsonValue> {
        let (ftext, flines) = self.file_text(&sym.file)?;
        let range = flines.span_to_range(&ftext, sym.selection_span.0, sym.selection_span.1);
        Some(JsonValue::object([
            ("uri", JsonValue::from(path_to_uri(&sym.file))),
            ("range", range.to_json()),
        ]))
    }
}

// ── Helper Functions ─────────────────────────────────────────────────────────

pub fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canon) = fs::canonicalize(path) {
        return canon;
    }
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if let Some(parent) = out.parent()
        && let Some(fname) = out.file_name()
        && let Ok(canon_parent) = fs::canonicalize(parent)
    {
        return canon_parent.join(fname);
    }
    out
}

fn sanitize_ident(raw: &str) -> String {
    let mut s = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            s.push(ch);
        } else if ch == '-' {
            s.push('_');
        }
    }
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

fn discover_package_files(root: &Path, exclusions: &[PathBuf]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    walk_pkg_dir(root, root, exclusions, &mut files, &mut dirs);
    files.sort();
    dirs.sort();
    (files, dirs)
}

fn walk_pkg_dir(
    root: &Path,
    dir: &Path,
    exclusions: &[PathBuf],
    files: &mut Vec<PathBuf>,
    dirs: &mut Vec<PathBuf>,
) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || IGNORED_DIRS.contains(&name.as_str()) {
            continue;
        }
        let norm = normalize_path(&path);
        if exclusions.iter().any(|ex| norm.starts_with(ex)) {
            continue;
        }
        if path.is_dir() {
            // Skip nested independent packages that define their own `lib.fl` / `main.fl`.
            if path != root && (path.join("lib.fl").is_file() || path.join("main.fl").is_file()) {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                dirs.push(rel.to_path_buf());
            }
            walk_pkg_dir(root, &path, exclusions, files, dirs);
        } else if path.extension().is_some_and(|e| e == "fl")
            && name != "flurryfmt.fl"
            && let Ok(rel) = path.strip_prefix(root)
        {
            files.push(rel.to_path_buf());
        }
    }
}

fn build_empty_ast(source_map: &SourceMap, start_pos: BytePos) -> Ast {
    let (tokens, symbols, _) = lex("", start_pos);
    let silent = DiagnosticContext::new(source_map);
    let mut parser = Parser::new(source_map, tokens, symbols, start_pos);
    let _ = parser.parse(&silent);
    parser.finalize()
}

fn lex_error_offsets(err: &lex::LexError, content: &str) -> (usize, usize) {
    // Extract byte index if available via debug/span or clamp to end of content.
    let _ = err;
    let end = content.len();
    (end.saturating_sub(1), end)
}

fn push_unique_diag(
    map: &mut HashMap<PathBuf, Vec<LspDiagnostic>>,
    file: PathBuf,
    diag: LspDiagnostic,
) {
    let list = map.entry(file).or_default();
    if !list.iter().any(|existing| {
        existing.range == diag.range
            && existing.severity == diag.severity
            && existing.message == diag.message
    }) {
        list.push(diag);
    }
}

fn build_hover_json(sym: &SymbolInfo, range: Option<LspRange>) -> JsonValue {
    let mut md = format!("```flurry\n{}\n```", sym.detail);
    if let Some(ref container) = sym.container_name
        && !container.is_empty()
    {
        md.push_str(&format!("\n*in `{container}`*"));
    }
    if let Some(ref doc) = sym.doc
        && !doc.is_empty()
    {
        md.push_str("\n\n---\n\n");
        md.push_str(doc);
    }
    let mut fields = vec![(
        "contents",
        JsonValue::object([
            ("kind", JsonValue::from("markdown")),
            ("value", JsonValue::from(md)),
        ]),
    )];
    if let Some(r) = range {
        fields.push(("range", r.to_json()));
    }
    JsonValue::object(fields)
}

fn symbol_to_completion_item(sym: &SymbolInfo, priority_group: &str) -> JsonValue {
    let mut fields = vec![
        ("label", JsonValue::from(sym.name.as_str())),
        ("kind", JsonValue::from(sym.completion_kind)),
        ("detail", JsonValue::from(sym.detail.as_str())),
        (
            "sortText",
            JsonValue::from(format!("{priority_group}_{}", sym.name)),
        ),
    ];
    if let Some(ref doc) = sym.doc {
        fields.push((
            "documentation",
            JsonValue::object([
                ("kind", JsonValue::from("markdown")),
                ("value", JsonValue::from(doc.as_str())),
            ]),
        ));
    }
    JsonValue::object(fields)
}

fn build_document_symbol_json(
    key: SymbolKey,
    index: &SymbolIndex,
    text: &str,
    line_idx: &LineIndex,
) -> Option<JsonValue> {
    let sym = index.symbol(key)?;
    let full_range = line_idx.span_to_range(text, sym.full_span.0, sym.full_span.1);
    let sel_lo = sym
        .selection_span
        .0
        .max(sym.full_span.0)
        .min(sym.full_span.1);
    let sel_hi = sym.selection_span.1.max(sel_lo).min(sym.full_span.1);
    let sel_range = line_idx.span_to_range(text, sel_lo, sel_hi);

    let children: Vec<JsonValue> = sym
        .children
        .iter()
        .filter_map(|&c| {
            let csym = index.symbol(c)?;
            if csym.file != sym.file {
                return None;
            }
            build_document_symbol_json(c, index, text, line_idx)
        })
        .collect();

    let mut fields = vec![
        ("name", JsonValue::from(sym.name.as_str())),
        ("detail", JsonValue::from(sym.detail.as_str())),
        ("kind", JsonValue::from(sym.kind)),
        ("range", full_range.to_json()),
        ("selectionRange", sel_range.to_json()),
    ];
    if !children.is_empty() {
        fields.push(("children", JsonValue::Array(children)));
    }
    Some(JsonValue::object(fields))
}

fn word_at(src: &str, offset: usize) -> Option<(String, (usize, usize))> {
    let bytes = src.as_bytes();
    let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = offset.min(bytes.len());
    if start == bytes.len() || !is_id(bytes[start]) {
        if start > 0 && is_id(bytes[start - 1]) {
            start -= 1;
        } else {
            return None;
        }
    }
    while start > 0 && is_id(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = start;
    while end < bytes.len() && is_id(bytes[end]) {
        end += 1;
    }
    (end > start).then(|| (src[start..end].to_owned(), (start, end)))
}

fn dot_access_at(src: &str, offset: usize) -> Option<(String, String)> {
    let (member, (m_lo, _)) = word_at(src, offset)?;
    let before = src[..m_lo].trim_end();
    let recv_part = before.strip_suffix('.')?;
    let recv = extract_trailing_chain(recv_part);
    (!recv.is_empty()).then_some((recv, member))
}

fn dot_completion_context(line_prefix: &str) -> Option<(String, String)> {
    let ident_len = line_prefix
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    let partial = &line_prefix[line_prefix.len() - ident_len..];
    let before_partial = &line_prefix[..line_prefix.len() - ident_len];
    let before_dot = before_partial.strip_suffix('.')?;
    // Ignore `..` range operator!
    if before_dot.ends_with('.') {
        return None;
    }
    let chain = extract_trailing_chain(before_dot);
    Some((chain, partial.to_owned()))
}

fn extract_trailing_chain(s: &str) -> String {
    let trimmed = s.trim_end();
    let len = trimmed
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'.' || *b == b'*')
        .count();
    trimmed[trimmed.len() - len..].trim_matches('.').to_owned()
}

fn quote_view_context(line_prefix: &str) -> Option<&str> {
    let ident_len = line_prefix
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    let partial = &line_prefix[line_prefix.len() - ident_len..];
    let before = &line_prefix[..line_prefix.len() - ident_len];
    let before_quote = before.strip_suffix('\'')?;
    let prev = before_quote.bytes().last()?;
    if prev.is_ascii_alphanumeric() || matches!(prev, b'_' | b')' | b']' | b'}') {
        Some(partial)
    } else {
        None
    }
}

fn identifier_prefix(line_prefix: &str) -> &str {
    let len = line_prefix
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    &line_prefix[line_prefix.len() - len..]
}

fn enclosing_call_context(src: &str, offset: usize) -> Option<((usize, usize), usize)> {
    let prefix = &src[..offset.min(src.len())];
    let (tokens, _, _) = lex(prefix, BytePos(0));
    let mut paren_stack: Vec<(usize, usize)> = Vec::new(); // (lparen_token_idx, comma_count)
    for (idx, tok) in tokens.iter().enumerate() {
        match tok.kind {
            TokenKind::LParen => paren_stack.push((idx, 0)),
            TokenKind::RParen => {
                paren_stack.pop();
            }
            TokenKind::Comma => {
                if let Some(top) = paren_stack.last_mut() {
                    top.1 += 1;
                }
            }
            _ => {}
        }
    }
    let (lparen_idx, active_param) = *paren_stack.last()?;
    // Walk backwards across optional `<...>` clause args and `a.b.callee` before `(`.
    let mut end_tok = lparen_idx.checked_sub(1)?;
    while end_tok > 0 && tokens[end_tok].kind == TokenKind::Comment {
        end_tok -= 1;
    }
    if tokens[end_tok].kind == TokenKind::Gt {
        let mut depth = 1i32;
        while end_tok > 0 && depth > 0 {
            end_tok -= 1;
            match tokens[end_tok].kind {
                TokenKind::Gt => depth += 1,
                TokenKind::Lt => depth -= 1,
                _ => {}
            }
        }
        end_tok = end_tok.checked_sub(1)?;
    }
    let hi = tokens[end_tok].to;
    let mut start_tok = end_tok;
    while start_tok > 1 {
        let prev = tokens[start_tok - 1].kind;
        if prev == TokenKind::Dot
            && matches!(
                tokens.get(start_tok - 2).map(|t| t.kind),
                Some(TokenKind::Id | TokenKind::SelfLower)
            )
        {
            start_tok -= 2;
        } else {
            break;
        }
    }
    let lo = tokens[start_tok].from;
    (hi > lo).then_some(((lo, hi), active_param))
}

fn complete_fmt_directives(partial: &str) -> Vec<JsonValue> {
    [
        (
            ".skip",
            "Skip formatting the next syntactic item/statement/expression",
        ),
        (
            ".off",
            "Disable formatting until `--% .on` or end of enclosing block",
        ),
        (".on", "Re-enable formatting after `--% .off`"),
        (
            ".max_width = 100",
            "Override maximum line width for current scope",
        ),
        (
            ".tab_spaces = 4",
            "Override indentation spaces per level for current scope",
        ),
        (
            ".fn_call_width = 60",
            "Override single-line function call width limit",
        ),
        (
            ".struct_lit_width = 35",
            "Override single-line struct initializer width limit",
        ),
        (
            ".chain_width = 60",
            "Override single-line method/postfix chain width limit",
        ),
    ]
    .into_iter()
    .filter(|(label, _)| label.starts_with(partial))
    .map(|(label, detail)| {
        JsonValue::object([
            ("label", JsonValue::from(label)),
            ("kind", JsonValue::from(lsp_completion_kind::PROPERTY)),
            ("detail", JsonValue::from(detail)),
        ])
    })
    .collect()
}

fn is_valid_flurry_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    !FLURRY_KEYWORDS.contains(&name)
}

pub const FLURRY_KEYWORDS: &[&str] = &[
    "and",
    "any",
    "as",
    "asserts",
    "assoc",
    "assumes",
    "async",
    "atomic",
    "await",
    "axiom",
    "break",
    "case",
    "catch",
    "comptime",
    "const",
    "continue",
    "decreases",
    "default",
    "defer",
    "define",
    "derive",
    "do",
    "dyn",
    "effect",
    "else",
    "ensures",
    "enum",
    "error",
    "exists",
    "extend",
    "false",
    "fn",
    "Fn",
    "for",
    "forall",
    "ghost",
    "handles",
    "if",
    "impl",
    "implicit",
    "in",
    "inline",
    "invariant",
    "is",
    "itself",
    "lambda",
    "lemma",
    "let",
    "lift",
    "match",
    "matches",
    "mod",
    "move",
    "newtype",
    "noreturn",
    "not",
    "null",
    "opaque",
    "opens",
    "or",
    "outcome",
    "predicate",
    "private",
    "pub",
    "pure",
    "quote",
    "ref",
    "refines",
    "requires",
    "resume",
    "return",
    "self",
    "Self",
    "spec",
    "static",
    "struct",
    "template",
    "test",
    "trait",
    "true",
    "Type",
    "typealias",
    "undefined",
    "union",
    "unsafe",
    "use",
    "var",
    "verified",
    "void",
    "when",
    "where",
    "while",
];
