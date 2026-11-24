//! HIR + scope-tree symbol indexing, occurrences, hover signatures, and member tables.
//!
//! Designed in the spirit of Zig's `zls`: fast, deterministic, early-resolution and
//! HIR-driven navigation without running full elaboration or comptime evaluation.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use hir::common::{Arg, FnParam, TyParam, TyParamKind};
use hir::expr::{ClosureParam, ObjectEntry};
use hir::item::{FnModifiers, UseKind, UsePath};
use hir::{
    Block, Body, BodyId, ClauseConstraint, ClauseConstraintKind, ClauseParam, ClauseParamKind,
    CondictionArm, DefKind, EnumDef, Expr, ExprKind, FieldDef, FieldPat, FnSig, HirId, Ident,
    ImplDef, ImplementationPhase, Item, ItemKind, LetDecl, OwnerId, Package, Pattern, PatternArm,
    PatternKind, StructDef, TraitDef, Variant, VariantKind,
};
use lex::{Token, TokenKind};
use rustc_span::{BytePos, FileName, RealFileName, SourceFile, SourceMap, Span};
use symbol::{DefId, Symbol};

/// LSP `SymbolKind` numeric constants (LSP 3.17).
pub mod lsp_symbol_kind {
    pub const FILE: u32 = 1;

    pub const MODULE: u32 = 2;

    pub const NAMESPACE: u32 = 3;

    pub const PACKAGE: u32 = 4;

    pub const CLASS: u32 = 5;

    pub const METHOD: u32 = 6;

    pub const PROPERTY: u32 = 7;

    pub const FIELD: u32 = 8;

    pub const CONSTRUCTOR: u32 = 9;

    pub const ENUM: u32 = 10;

    pub const INTERFACE: u32 = 11;

    pub const FUNCTION: u32 = 12;

    pub const VARIABLE: u32 = 13;

    pub const CONSTANT: u32 = 14;

    pub const TYPE_PARAMETER: u32 = 26;

    pub const ENUM_MEMBER: u32 = 22;

    pub const STRUCT: u32 = 23;

    pub const EVENT: u32 = 24;

    pub const OPERATOR: u32 = 25;
}

/// LSP `CompletionItemKind` numeric constants (LSP 3.17).
pub mod lsp_completion_kind {
    pub const TEXT: u32 = 1;

    pub const METHOD: u32 = 2;

    pub const FUNCTION: u32 = 3;

    pub const CONSTRUCTOR: u32 = 4;

    pub const FIELD: u32 = 5;

    pub const VARIABLE: u32 = 6;

    pub const CLASS: u32 = 7;

    pub const INTERFACE: u32 = 8;

    pub const MODULE: u32 = 9;

    pub const PROPERTY: u32 = 10;

    pub const KEYWORD: u32 = 14;

    pub const SNIPPET: u32 = 15;

    pub const ENUM: u32 = 13;

    pub const STRUCT: u32 = 22;

    pub const ENUM_MEMBER: u32 = 20;

    pub const CONSTANT: u32 = 21;

    pub const TYPE_PARAMETER: u32 = 25;
}

/// Canonical Semantic Token type indices advertised in `initialize`.
pub mod semantic_token_type {
    pub const NAMESPACE: u32 = 0;

    pub const TYPE: u32 = 1;

    pub const STRUCT: u32 = 2;

    pub const ENUM: u32 = 3;

    pub const INTERFACE: u32 = 4;

    pub const TYPE_PARAMETER: u32 = 5;

    pub const PARAMETER: u32 = 6;

    pub const VARIABLE: u32 = 7;

    pub const PROPERTY: u32 = 8;

    pub const ENUM_MEMBER: u32 = 9;

    pub const FUNCTION: u32 = 10;

    pub const METHOD: u32 = 11;

    pub const MACRO: u32 = 12;

    pub const KEYWORD: u32 = 13;

    pub const MODIFIER: u32 = 14;

    pub const COMMENT: u32 = 15;

    pub const STRING: u32 = 16;

    pub const NUMBER: u32 = 17;

    pub const OPERATOR: u32 = 18;

    pub const LEGEND: &[&str] = &[
        "namespace",
        "type",
        "struct",
        "enum",
        "interface",
        "typeParameter",
        "parameter",
        "variable",
        "property",
        "enumMember",
        "function",
        "method",
        "macro",
        "keyword",
        "modifier",
        "comment",
        "string",
        "number",
        "operator",
    ];
}

/// Canonical Semantic Token modifier bit indices advertised in `initialize`.
pub mod semantic_token_modifier {
    pub const DECLARATION: u32 = 1 << 0;

    pub const DEFINITION: u32 = 1 << 1;

    pub const READONLY: u32 = 1 << 2;

    pub const MODIFICATION: u32 = 1 << 3;

    pub const DOCUMENTATION: u32 = 1 << 4;

    pub const LEGEND: &[&str] = &[
        "declaration",
        "definition",
        "readonly",
        "modification",
        "documentation",
    ];
}

/// Unique cross-package symbol identity across top-level definitions, members, and locals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKey {
    Def(DefId),
    Local { pkg: u32, hir_id: HirId },
    Field { owner: DefId, index: u32 },
    Variant { owner: DefId, index: u32 },
}

/// Parameter metadata for `textDocument/signatureHelp`.
#[derive(Debug, Clone)]
pub struct ParamDoc {
    pub label: String,
}

/// Rich symbol record stored in [`SymbolIndex`].
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub key: SymbolKey,
    pub name: String,
    pub kind: u32,
    pub completion_kind: u32,
    pub detail: String,
    pub doc: Option<String>,
    pub container_name: Option<String>,
    pub file: PathBuf,
    /// Full item/declaration byte span `(lo, hi)` within `file`.
    pub full_span: (usize, usize),
    /// Identifier byte span `(lo, hi)` within `file`.
    pub selection_span: (usize, usize),
    pub children: Vec<SymbolKey>,
    pub params: Vec<ParamDoc>,
    pub is_user_pkg: bool,
    pub is_mutable: bool,
}

/// A resolved identifier occurrence within a source file.
#[derive(Debug, Clone)]
pub struct Occurrence {
    pub span: (usize, usize),
    pub target: SymbolKey,
    pub is_decl: bool,
    pub is_write: bool,
}

/// Member entry attached to a nominal type/module (`struct`, `enum`, `trait`, `mod`).
#[derive(Debug, Clone)]
pub struct MemberEntry {
    pub name: String,
    pub key: SymbolKey,
    pub has_self_receiver: bool,
}

/// Indexed semantic snapshot across a user package and any loaded sysroot packages.
#[derive(Debug, Clone, Default)]
pub struct SymbolIndex {
    pub symbols: HashMap<SymbolKey, SymbolInfo>,
    /// Per-file top-level outline roots in source order.
    pub file_roots: HashMap<PathBuf, Vec<SymbolKey>>,
    /// Per-file occurrences sorted by `(span.0, span.1)`.
    pub file_occurrences: HashMap<PathBuf, Vec<Occurrence>>,
    /// Members of a nominal `DefId` (fields, variants, nested items, inherent/trait methods).
    pub nominal_members: HashMap<DefId, Vec<MemberEntry>>,
    /// Nominal `DefId` lookup by simple type/module name (user package prioritized first).
    pub defs_by_name: HashMap<String, Vec<DefId>>,
    /// Local/parameter receiver type hints: `(file, var_name, decl_offset) -> nominal DefId`.
    pub local_type_hints: HashMap<SymbolKey, DefId>,
    /// Visible module/prelude/package names per file for top-level completion.
    pub file_visible_defs: HashMap<PathBuf, Vec<(String, SymbolKey)>>,
}

impl SymbolIndex {
    pub fn occurrence_at(&self, file: &Path, offset: usize) -> Option<&Occurrence> {
        let occs = self.file_occurrences.get(file)?;
        // Prefer the tightest occurrence containing `offset` (inclusive on end for cursor-after-ident).
        let mut best: Option<&Occurrence> = None;
        for occ in occs {
            let (lo, hi) = occ.span;
            if lo <= offset && offset <= hi && hi > lo {
                match best {
                    None => best = Some(occ),
                    Some(prev) => {
                        let prev_len = prev.span.1.saturating_sub(prev.span.0);
                        let cur_len = hi.saturating_sub(lo);
                        // Prefer exact interior hit `[lo, hi)` over boundary touch `offset == hi`,
                        // and tighter spans over wider spans.
                        let prev_interior = offset < prev.span.1;
                        let cur_interior = offset < hi;
                        if (cur_interior && !prev_interior)
                            || (cur_interior == prev_interior && cur_len <= prev_len)
                        {
                            best = Some(occ);
                        }
                    }
                }
            }
        }
        best
    }

    pub fn symbol(&self, key: SymbolKey) -> Option<&SymbolInfo> {
        self.symbols.get(&key)
    }

    pub fn references_to(
        &self,
        target: SymbolKey,
        include_decl: bool,
    ) -> Vec<(PathBuf, (usize, usize))> {
        let mut results = Vec::new();
        let mut files: Vec<_> = self.file_occurrences.keys().cloned().collect();
        files.sort();
        for file in files {
            if let Some(occs) = self.file_occurrences.get(&file) {
                for occ in occs {
                    if occ.target == target && (include_decl || !occ.is_decl) {
                        results.push((file.clone(), occ.span));
                    }
                }
            }
        }
        results.dedup();
        results
    }

    pub fn highlights_in_file(&self, file: &Path, target: SymbolKey) -> Vec<&Occurrence> {
        let Some(occs) = self.file_occurrences.get(file) else {
            return Vec::new();
        };
        occs.iter().filter(|o| o.target == target).collect()
    }

    pub fn members_of(&self, def_id: DefId) -> &[MemberEntry] {
        self.nominal_members
            .get(&def_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

fn item_clauses<'hir>(item: &Item<'hir>) -> &'hir [ClauseParam<'hir>] {
    match &item.kind {
        ItemKind::Fn(sig, _)
        | ItemKind::Case(sig, _)
        | ItemKind::FnDeclaration(sig)
        | ItemKind::Effect(sig, _) => sig.clause_params,
        ItemKind::FunctionTemplateInstance {
            signature: Some(sig),
            ..
        } => sig.clause_params,
        ItemKind::Struct(d) => d.clause_params,
        ItemKind::Enum(d) => d.clause_params,
        ItemKind::Impl(d) => d.clause_params,
        ItemKind::Trait(d) => d.clause_params,
        ItemKind::Deriver(d) => d.clause_params,
        ItemKind::Derive(d) => d.clause_params,
        ItemKind::Associated(d) => d.clause_params,
        ItemKind::EffectGroup(d) => d.clause_params,
        ItemKind::TypeAlias(_, clauses, _) | ItemKind::Newtype(_, clauses, _) => clauses,
        ItemKind::FunctionTemplateInstance {
            signature: None, ..
        }
        | ItemKind::TestTemplateInstance { .. }
        | ItemKind::Mod(_)
        | ItemKind::Use(_)
        | ItemKind::Const(..)
        | ItemKind::Static(..)
        | ItemKind::Invalid => &[],
    }
}

pub fn span_to_file_offsets(
    source_map: &SourceMap,
    span: Span,
) -> Option<(PathBuf, (usize, usize))> {
    if span == rustc_span::DUMMY_SP {
        return None;
    }
    let sf = source_map.lookup_source_file(span.lo());
    let path = source_file_path(&sf)?;
    let start_pos = sf.start_pos.0;
    if span.lo().0 < start_pos || span.hi().0 < span.lo().0 {
        return None;
    }
    let src_len = sf.src.as_ref().map(|s| s.len()).unwrap_or(0);
    let lo = ((span.lo().0 - start_pos) as usize).min(src_len);
    let hi = ((span.hi().0 - start_pos) as usize).min(src_len);
    Some((path, (lo, hi)))
}

pub fn source_file_path(sf: &SourceFile) -> Option<PathBuf> {
    match &sf.name {
        FileName::Real(RealFileName::LocalPath(p)) => Some(p.clone()),
        FileName::Real(other) => other.local_path().map(Path::to_path_buf),
        _ => None,
    }
}

fn consume_balanced_group(s: &str, open: char, close: char) -> Option<&str> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    if first != open {
        return None;
    }
    let mut depth = 1usize;
    let mut in_str = false;
    let mut escaped = false;
    for (idx, ch) in chars {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch == '"' {
            in_str = true;
        } else if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(&s[idx + ch.len_utf8()..]);
            }
        }
    }
    None
}

/// Returns `true` when `line` contains only prefix attributes (`^...`) and
/// declaration modifiers (`pub`, `private`, `pure`, `comptime`, etc.).
fn is_item_prefix_only(line: &str) -> bool {
    let mut rest = line.trim();
    while !rest.is_empty() {
        if let Some(after_caret) = rest.strip_prefix('^') {
            let path_len = after_caret
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                .unwrap_or(after_caret.len());
            if path_len == 0 {
                return false;
            }
            rest = &after_caret[path_len..];
            loop {
                let next_non_ws = rest.trim_start_matches([' ', '\t']);
                let Some(first) = next_non_ws.chars().next() else {
                    rest = next_non_ws;
                    break;
                };
                let pair = match first {
                    '(' => Some(('(', ')')),
                    '[' => Some(('[', ']')),
                    '{' => Some(('{', '}')),
                    '<' => Some(('<', '>')),
                    _ => None,
                };
                if let Some((open, close)) = pair {
                    let Some(after_group) = consume_balanced_group(next_non_ws, open, close) else {
                        return false;
                    };
                    rest = after_group;
                } else {
                    break;
                }
            }
            rest = rest.trim_start();
            continue;
        }

        let word_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if word_len == 0 {
            return false;
        }
        let word = &rest[..word_len];
        if !matches!(
            word,
            "pub"
                | "private"
                | "pure"
                | "comptime"
                | "inline"
                | "unsafe"
                | "async"
                | "spec"
                | "verified"
                | "atomic"
                | "ghost"
                | "quote"
                | "effect"
        ) {
            return false;
        }
        rest = rest[word_len..].trim_start();
    }
    true
}

/// Extract leading `--` and `{- ... -}` documentation comments immediately preceding `item_lo`.
pub fn extract_leading_doc(src: &str, item_lo: usize) -> Option<String> {
    if item_lo == 0 || item_lo > src.len() {
        return None;
    }
    let prefix = &src[..item_lo];
    let lines: Vec<&str> = prefix.split('\n').collect();
    if lines.is_empty() {
        return None;
    }
    // Check if the current line has only whitespace, attributes, or declaration modifiers before `item_lo`.
    let mut idx = lines.len() - 1;
    let tail_line = lines[idx].trim();
    if !tail_line.is_empty() && !is_item_prefix_only(tail_line) {
        return None;
    }
    let mut doc_lines = Vec::new();
    while idx > 0 {
        idx -= 1;
        let trimmed = lines[idx].trim_end_matches('\r').trim();
        if !trimmed.is_empty() && is_item_prefix_only(trimmed) {
            // Skip prefix attributes or modifiers sitting between doc comment and declaration.
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("--") {
            // Skip `--%` formatter directives.
            if rest.starts_with('%') {
                continue;
            }
            let cleaned = rest.strip_prefix('-').unwrap_or(rest);
            let cleaned = cleaned.strip_prefix(' ').unwrap_or(cleaned);
            doc_lines.push(cleaned.to_owned());
        } else if trimmed.ends_with("-}") {
            // Collect preceding `{- ... -}` block comment.
            let mut block_lines = Vec::new();
            let mut cur = idx;
            let mut found_open = false;
            loop {
                let line_raw = lines[cur].trim_end_matches('\r');
                block_lines.push(line_raw);
                if line_raw.trim_start().starts_with("{-") {
                    found_open = true;
                    break;
                }
                if cur == 0 {
                    break;
                }
                cur -= 1;
            }
            if found_open {
                block_lines.reverse();
                let joined = block_lines.join("\n");
                let inner = joined
                    .trim()
                    .strip_prefix("{-")
                    .and_then(|s| s.strip_suffix("-}"))
                    .unwrap_or(&joined)
                    .trim();
                if !inner.is_empty() {
                    for l in inner.lines().rev() {
                        doc_lines.push(l.trim().to_owned());
                    }
                }
            }
            break;
        } else {
            break;
        }
    }
    if doc_lines.is_empty() {
        return None;
    }
    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Package input passed to [`IndexBuilder`].
pub struct LoweredPackageInput<'a, 'hir> {
    pub pkg_id: u32,
    pub is_user_pkg: bool,
    pub pkg_name: &'a str,
    pub hir_pkg: &'a Package<'hir>,
}

/// Builds a unified [`SymbolIndex`] from `resolve::ModuleTree` and all lowered `hir::Package`s.
pub struct IndexBuilder<'a> {
    source_map: &'a SourceMap,
    module_tree: &'a resolve::ModuleTree,
    index: SymbolIndex,
    /// Maps `(pkg_id, OwnerId)` -> `DefId` across all lowered packages.
    owner_defs: HashMap<(u32, OwnerId), DefId>,
    /// Maps normalized file path -> file-level `resolve::ScopeId`.
    path_to_scope: HashMap<PathBuf, resolve::ScopeId>,
    /// Maps `DefId` -> owned `resolve::ScopeId` (for modules, packages, ADTs, traits, etc.).
    def_to_scope: HashMap<DefId, resolve::ScopeId>,
    /// Tracks children registered under a parent symbol to avoid duplicates.
    seen_children: HashSet<(SymbolKey, SymbolKey)>,
    /// Maps `(pkg_id, ImplOwnerId)` -> self nominal `DefId` when `impl` targets a nominal type.
    impl_self_nominal: HashMap<(u32, OwnerId), DefId>,
}

impl<'a> IndexBuilder<'a> {
    pub fn new(source_map: &'a SourceMap, module_tree: &'a resolve::ModuleTree) -> Self {
        let mut def_to_scope = HashMap::new();
        for scope in module_tree.scope_tree.iter() {
            if scope.owner_def.is_valid() {
                def_to_scope.entry(scope.owner_def).or_insert(scope.id);
            }
        }
        Self {
            source_map,
            module_tree,
            index: SymbolIndex::default(),
            owner_defs: HashMap::new(),
            path_to_scope: HashMap::new(),
            def_to_scope,
            seen_children: HashSet::new(),
            impl_self_nominal: HashMap::new(),
        }
    }

    pub fn build(
        mut self,
        packages: &[LoweredPackageInput<'_, '_>],
        vfs_files: &[(u32, vfs::FileId, PathBuf)],
    ) -> SymbolIndex {
        for (_, combined_fid, path) in vfs_files {
            if let Some(&scope_id) = self.module_tree.file_scopes.get(combined_fid) {
                self.path_to_scope.insert(path.clone(), scope_id);
            }
        }

        // Pass 1: collect all `(pkg_id, OwnerId) -> DefId` and register every item & nominal member.
        for pkg in packages {
            for (owner_id, _) in pkg.hir_pkg.owners() {
                if let Some(def_id) = pkg.hir_pkg.owner_definition(owner_id) {
                    self.owner_defs.insert((pkg.pkg_id, owner_id), def_id);
                }
            }
        }

        for pkg in packages {
            self.index_package_items(pkg);
        }

        // Pass 2: link nested children & `impl` blocks onto nominal types, and walk bodies/expressions!
        for pkg in packages {
            self.index_package_hierarchy_and_impls(pkg);
        }

        for pkg in packages {
            self.index_package_bodies_and_refs(pkg);
        }

        // Pass 3: populate per-file visible scope bindings for fast scope-aware completion.
        self.index_visible_scope_defs(vfs_files);

        // Sort & deduplicate per-file occurrences and roots.
        for occs in self.index.file_occurrences.values_mut() {
            occs.sort_by_key(|o| (o.span.0, o.span.1, !o.is_decl, !o.is_write));
            occs.dedup_by(|a, b| a.span == b.span && a.target == b.target);
        }
        for roots in self.index.file_roots.values_mut() {
            roots.sort_by_key(|k| {
                self.index
                    .symbols
                    .get(k)
                    .map(|s| s.full_span.0)
                    .unwrap_or(0)
            });
            roots.dedup();
        }
        for members in self.index.nominal_members.values_mut() {
            let mut seen = HashSet::new();
            members.retain(|m| seen.insert((m.name.clone(), m.key)));
        }

        self.index
    }

    fn record_occurrence(
        &mut self,
        file: PathBuf,
        span: (usize, usize),
        target: SymbolKey,
        is_decl: bool,
        is_write: bool,
    ) {
        if span.1 <= span.0 {
            return;
        }
        self.index
            .file_occurrences
            .entry(file)
            .or_default()
            .push(Occurrence {
                span,
                target,
                is_decl,
                is_write,
            });
    }

    fn add_child(&mut self, parent: SymbolKey, child: SymbolKey) {
        if parent == child || !self.seen_children.insert((parent, child)) {
            return;
        }
        if let Some(p) = self.index.symbols.get_mut(&parent) {
            p.children.push(child);
        }
    }

    fn add_nominal_member(
        &mut self,
        nominal: DefId,
        name: String,
        key: SymbolKey,
        has_self_receiver: bool,
    ) {
        if name.is_empty() || name.starts_with('$') || name == "<invalid>" {
            return;
        }
        self.index
            .nominal_members
            .entry(nominal)
            .or_default()
            .push(MemberEntry {
                name,
                key,
                has_self_receiver,
            });
    }

    fn register_def_name(&mut self, name: &str, def_id: DefId, is_user_pkg: bool) {
        if name.is_empty() || name.starts_with('$') || name.starts_with('<') {
            return;
        }
        let entry = self.index.defs_by_name.entry(name.to_owned()).or_default();
        if !entry.contains(&def_id) {
            if is_user_pkg {
                entry.insert(0, def_id);
            } else {
                entry.push(def_id);
            }
        }
    }

    fn index_package_items(&mut self, pkg: &LoweredPackageInput<'_, '_>) {
        for (owner_id, owner_info) in pkg.hir_pkg.owners() {
            let item = owner_info.node.expect_item();
            let Some(def_id) = pkg.hir_pkg.owner_definition(owner_id) else {
                continue;
            };
            if matches!(item.kind, ItemKind::Invalid | ItemKind::Use(..)) {
                continue;
            }

            let raw_name = item.ident.name.as_str();
            let is_synthetic_file_mod =
                matches!(item.kind, ItemKind::Mod(_)) && item.ident.span == item.span;
            // Skip synthetic anonymous root modules with DUMMY_SP unless they have a real span.
            let span_info = span_to_file_offsets(self.source_map, item.span);
            let ident_span_info = span_to_file_offsets(self.source_map, item.ident.span);

            let (file, full_span, selection_span, src_doc) = match (span_info, ident_span_info) {
                (Some((f, full)), _) if is_synthetic_file_mod => (f, full, (0, 0), None),
                (Some((f, full)), Some((if_path, sel))) if if_path == f && sel.1 > sel.0 => {
                    let doc = self.doc_at(item.span);
                    (f, full, sel, doc)
                }
                (Some((f, full)), _) => {
                    let doc = self.doc_at(item.span);
                    (f, full, full, doc)
                }
                (None, Some((f, sel))) => {
                    let doc = self.doc_at(item.ident.span);
                    (f, sel, sel, doc)
                }
                (None, None) => continue,
            };

            let display_name = self.item_display_name(pkg.hir_pkg, item, raw_name);
            if display_name.is_empty() || display_name == "<root>" {
                continue;
            }

            let (sym_kind, comp_kind) = item_lsp_kinds(&item.kind);
            let (detail, params) = format_item_signature(pkg.hir_pkg, item, self.source_map);

            let key = SymbolKey::Def(def_id);
            self.index.symbols.insert(
                key,
                SymbolInfo {
                    key,
                    name: display_name.clone(),
                    kind: sym_kind,
                    completion_kind: comp_kind,
                    detail,
                    doc: src_doc,
                    container_name: Some(pkg.pkg_name.to_owned()),
                    file: file.clone(),
                    full_span,
                    selection_span,
                    children: Vec::new(),
                    params,
                    is_user_pkg: pkg.is_user_pkg,
                    is_mutable: false,
                },
            );

            self.register_def_name(&display_name, def_id, pkg.is_user_pkg);

            if !is_synthetic_file_mod
                && !matches!(item.kind, ItemKind::Impl(..) | ItemKind::Derive(..))
                && !decode_test_label(raw_name).is_some_and(str::is_empty)
                && selection_span.1 > selection_span.0
            {
                self.record_occurrence(file.clone(), selection_span, key, true, false);
            }

            // Index clause parameters (`<T, const N: usize>` / `where ...`) on this item!
            for clause in item_clauses(item) {
                self.index_clause_param(pkg, clause, &display_name);
            }

            // Index struct fields and enum variants!
            match &item.kind {
                ItemKind::Struct(sdef) => {
                    self.index_struct_fields(pkg, def_id, &display_name, sdef.fields, key);
                }
                ItemKind::Enum(edef) => {
                    self.index_enum_variants(pkg, def_id, &display_name, edef.variants, key);
                }
                _ => {}
            }
        }
    }

    fn index_clause_param(
        &mut self,
        pkg: &LoweredPackageInput<'_, '_>,
        clause: &ClauseParam<'_>,
        container: &str,
    ) {
        let Some(clause_def) = clause.definition else {
            return;
        };
        let Some((file, full_span)) = span_to_file_offsets(self.source_map, clause.span) else {
            return;
        };
        let sel_span = span_to_file_offsets(self.source_map, clause.name.span)
            .filter(|(f, s)| f == &file && s.1 > s.0)
            .map(|(_, s)| s)
            .unwrap_or(full_span);
        let name = clause.name.name.as_str().to_owned();
        if name.is_empty() || name == "<invalid>" {
            return;
        }
        let detail = format_clause_param(clause, self.source_map);
        let key = SymbolKey::Def(clause_def);
        self.index.symbols.insert(
            key,
            SymbolInfo {
                key,
                name,
                kind: lsp_symbol_kind::TYPE_PARAMETER,
                completion_kind: lsp_completion_kind::TYPE_PARAMETER,
                detail,
                doc: None,
                container_name: Some(container.to_owned()),
                file: file.clone(),
                full_span,
                selection_span: sel_span,
                children: Vec::new(),
                params: Vec::new(),
                is_user_pkg: pkg.is_user_pkg,
                is_mutable: false,
            },
        );
        self.record_occurrence(file, sel_span, key, true, false);
    }

    fn index_struct_fields(
        &mut self,
        pkg: &LoweredPackageInput<'_, '_>,
        owner_def: DefId,
        struct_name: &str,
        fields: &[FieldDef<'_>],
        parent_key: SymbolKey,
    ) {
        for (idx, field) in fields.iter().enumerate() {
            let fname = field.ident.name.as_str().to_owned();
            if fname.is_empty() || fname == "<invalid>" {
                continue;
            }
            let Some((file, full_span)) = span_to_file_offsets(self.source_map, field.span) else {
                continue;
            };
            let sel_span = span_to_file_offsets(self.source_map, field.ident.span)
                .filter(|(f, s)| f == &file && s.1 > s.0)
                .map(|(_, s)| s)
                .unwrap_or(full_span);
            let ty_str = format_expr_compact(field.ty, self.source_map);
            let detail = match field.default {
                Some(def_expr) => {
                    format!(
                        "{fname}: {ty_str} = {}",
                        format_expr_compact(def_expr, self.source_map)
                    )
                }
                None => format!("{fname}: {ty_str}"),
            };
            let doc = self.doc_at(field.span);
            let fkey = SymbolKey::Field {
                owner: owner_def,
                index: idx as u32,
            };
            self.index.symbols.insert(
                fkey,
                SymbolInfo {
                    key: fkey,
                    name: fname.clone(),
                    kind: lsp_symbol_kind::FIELD,
                    completion_kind: lsp_completion_kind::FIELD,
                    detail,
                    doc,
                    container_name: Some(struct_name.to_owned()),
                    file: file.clone(),
                    full_span,
                    selection_span: sel_span,
                    children: Vec::new(),
                    params: Vec::new(),
                    is_user_pkg: pkg.is_user_pkg,
                    is_mutable: false,
                },
            );
            self.add_child(parent_key, fkey);
            self.add_nominal_member(owner_def, fname, fkey, true);
            self.record_occurrence(file, sel_span, fkey, true, false);
        }
    }

    fn index_enum_variants(
        &mut self,
        pkg: &LoweredPackageInput<'_, '_>,
        owner_def: DefId,
        enum_name: &str,
        variants: &[Variant<'_>],
        parent_key: SymbolKey,
    ) {
        let mut counter = 0u32;
        self.index_enum_variants_rec(
            pkg,
            owner_def,
            enum_name,
            variants,
            parent_key,
            &mut counter,
        );
    }

    fn index_enum_variants_rec(
        &mut self,
        pkg: &LoweredPackageInput<'_, '_>,
        owner_def: DefId,
        enum_name: &str,
        variants: &[Variant<'_>],
        parent_key: SymbolKey,
        counter: &mut u32,
    ) {
        for variant in variants {
            let idx = *counter;
            *counter += 1;
            let vname = variant.ident.name.as_str().to_owned();
            if vname.is_empty() || vname == "<invalid>" {
                continue;
            }
            let Some((file, full_span)) = span_to_file_offsets(self.source_map, variant.span)
            else {
                continue;
            };
            let sel_span = span_to_file_offsets(self.source_map, variant.ident.span)
                .filter(|(f, s)| f == &file && s.1 > s.0)
                .map(|(_, s)| s)
                .unwrap_or(full_span);
            let detail = format_variant(enum_name, variant, self.source_map);
            let doc = self.doc_at(variant.span);
            let vkey = SymbolKey::Variant {
                owner: owner_def,
                index: idx,
            };
            self.index.symbols.insert(
                vkey,
                SymbolInfo {
                    key: vkey,
                    name: vname.clone(),
                    kind: lsp_symbol_kind::ENUM_MEMBER,
                    completion_kind: lsp_completion_kind::ENUM_MEMBER,
                    detail,
                    doc,
                    container_name: Some(enum_name.to_owned()),
                    file: file.clone(),
                    full_span,
                    selection_span: sel_span,
                    children: Vec::new(),
                    params: Vec::new(),
                    is_user_pkg: pkg.is_user_pkg,
                    is_mutable: false,
                },
            );
            self.add_child(parent_key, vkey);
            self.add_nominal_member(owner_def, vname.clone(), vkey, false);
            self.record_occurrence(file, sel_span, vkey, true, false);

            if let VariantKind::SubEnum(sub) = variant.kind {
                let sub_container = format!("{enum_name}.{vname}");
                self.index_enum_variants_rec(pkg, owner_def, &sub_container, sub, vkey, counter);
            }
        }
    }

    fn index_package_hierarchy_and_impls(&mut self, pkg: &LoweredPackageInput<'_, '_>) {
        let mut child_owners: HashSet<OwnerId> = HashSet::new();

        for (owner_id, owner_info) in pkg.hir_pkg.owners() {
            let item = owner_info.node.expect_item();
            let Some(parent_def) = pkg.hir_pkg.owner_definition(owner_id) else {
                continue;
            };
            let parent_key = SymbolKey::Def(parent_def);
            let parent_file = self.index.symbols.get(&parent_key).map(|s| s.file.clone());
            let parent_name = self.index.symbols.get(&parent_key).map(|s| s.name.clone());

            match &item.kind {
                ItemKind::Mod(mod_def_items) => {
                    let is_synthetic_file_mod = item.ident.span == item.span;
                    for &child_owner in &mod_def_items.items {
                        if let Some(&child_def) = self.owner_defs.get(&(pkg.pkg_id, child_owner)) {
                            let child_key = SymbolKey::Def(child_def);
                            if let Some(child_sym) = self.index.symbols.get_mut(&child_key) {
                                if let Some(ref pname) = parent_name
                                    && !pname.starts_with('<')
                                {
                                    child_sym.container_name = Some(pname.clone());
                                }
                                let cname = child_sym.name.clone();
                                let same_file = parent_file.as_ref() == Some(&child_sym.file);
                                if same_file
                                    && !is_synthetic_file_mod
                                    && owner_id != pkg.hir_pkg.root_mod
                                {
                                    child_owners.insert(child_owner);
                                    self.add_child(parent_key, child_key);
                                }
                                self.add_nominal_member(parent_def, cname, child_key, false);
                            }
                        }
                    }
                }
                ItemKind::Struct(StructDef { nested_items, .. })
                | ItemKind::Enum(EnumDef { nested_items, .. }) => {
                    for &child_owner in nested_items {
                        child_owners.insert(child_owner);
                        self.attach_member_owner(
                            pkg,
                            parent_def,
                            parent_key,
                            parent_name.as_deref(),
                            child_owner,
                        );
                    }
                }
                ItemKind::Trait(TraitDef { items, .. }) => {
                    for &child_owner in items {
                        child_owners.insert(child_owner);
                        self.attach_member_owner(
                            pkg,
                            parent_def,
                            parent_key,
                            parent_name.as_deref(),
                            child_owner,
                        );
                    }
                }
                ItemKind::Impl(ImplDef { self_ty, items, .. }) => {
                    let target_nominal = extract_head_def_id(self_ty);
                    if let Some(nom_def) = target_nominal {
                        self.impl_self_nominal
                            .insert((pkg.pkg_id, owner_id), nom_def);
                    }
                    for &child_owner in items {
                        child_owners.insert(child_owner);
                        if let Some(&child_def) = self.owner_defs.get(&(pkg.pkg_id, child_owner)) {
                            let child_key = SymbolKey::Def(child_def);
                            self.add_child(parent_key, child_key);
                            if let Some(child_sym) = self.index.symbols.get_mut(&child_key) {
                                if child_sym.kind == lsp_symbol_kind::FUNCTION {
                                    child_sym.kind = lsp_symbol_kind::METHOD;
                                    child_sym.completion_kind = lsp_completion_kind::METHOD;
                                }
                                if let Some(ref pname) = parent_name {
                                    child_sym.container_name = Some(pname.clone());
                                }
                            }
                            if let Some(nom_def) = target_nominal
                                && let Some(child_item) = pkg.hir_pkg.item(child_owner)
                            {
                                let mname = child_item.ident.name.as_str().to_owned();
                                let has_self = item_has_self_param(child_item);
                                self.add_nominal_member(nom_def, mname, child_key, has_self);
                            }
                        }
                    }
                }
                ItemKind::EffectGroup(eg) => {
                    for &op_owner in &eg.operations {
                        child_owners.insert(op_owner);
                        self.attach_member_owner(
                            pkg,
                            parent_def,
                            parent_key,
                            parent_name.as_deref(),
                            op_owner,
                        );
                    }
                }
                _ => {}
            }
        }

        // Link local items declared inside function/block bodies to their enclosing owner.
        for (&child_owner, &parent_owner) in &pkg.hir_pkg.lexical_parents {
            if !child_owners.contains(&child_owner)
                && let Some(&parent_def) = self.owner_defs.get(&(pkg.pkg_id, parent_owner))
                && let Some(&child_def) = self.owner_defs.get(&(pkg.pkg_id, child_owner))
            {
                if let Some(parent_item) = pkg.hir_pkg.item(parent_owner) {
                    let is_synthetic_parent = matches!(parent_item.kind, ItemKind::Mod(_))
                        && parent_item.ident.span == parent_item.span;
                    if !is_synthetic_parent && parent_owner != pkg.hir_pkg.root_mod {
                        child_owners.insert(child_owner);
                        self.add_child(SymbolKey::Def(parent_def), SymbolKey::Def(child_def));
                    }
                }
            }
        }

        // Any user/sysroot item in a file that is NOT nested inside another same-file item
        // becomes a top-level document outline root for its file!
        for (owner_id, owner_info) in pkg.hir_pkg.owners() {
            let item = owner_info.node.expect_item();
            let is_synthetic_file_mod =
                matches!(item.kind, ItemKind::Mod(_)) && item.ident.span == item.span;
            if owner_id == pkg.hir_pkg.root_mod
                || is_synthetic_file_mod
                || child_owners.contains(&owner_id)
            {
                continue;
            }
            if pkg.hir_pkg.lexical_parents.contains_key(&owner_id) {
                continue;
            }
            if let Some(&def_id) = self.owner_defs.get(&(pkg.pkg_id, owner_id)) {
                let key = SymbolKey::Def(def_id);
                if let Some(sym) = self.index.symbols.get(&key) {
                    self.index
                        .file_roots
                        .entry(sym.file.clone())
                        .or_default()
                        .push(key);
                }
            }
        }
    }

    fn attach_member_owner(
        &mut self,
        pkg: &LoweredPackageInput<'_, '_>,
        parent_def: DefId,
        parent_key: SymbolKey,
        parent_name: Option<&str>,
        child_owner: OwnerId,
    ) {
        let Some(&child_def) = self.owner_defs.get(&(pkg.pkg_id, child_owner)) else {
            return;
        };
        let child_key = SymbolKey::Def(child_def);
        self.add_child(parent_key, child_key);
        if let Some(child_sym) = self.index.symbols.get_mut(&child_key) {
            if child_sym.kind == lsp_symbol_kind::FUNCTION {
                child_sym.kind = lsp_symbol_kind::METHOD;
                child_sym.completion_kind = lsp_completion_kind::METHOD;
            }
            if let Some(pname) = parent_name {
                child_sym.container_name = Some(pname.to_owned());
            }
        }
        if let Some(child_item) = pkg.hir_pkg.item(child_owner) {
            let mname = child_item.ident.name.as_str().to_owned();
            let has_self = item_has_self_param(child_item);
            self.add_nominal_member(parent_def, mname, child_key, has_self);
        }
    }

    fn index_package_bodies_and_refs(&mut self, pkg: &LoweredPackageInput<'_, '_>) {
        // Build reverse map from child_owner -> enclosing nominal `DefId` (for `self` resolution!).
        let mut owner_enclosing_nominal: HashMap<OwnerId, DefId> = HashMap::new();
        for (owner_id, owner_info) in pkg.hir_pkg.owners() {
            let item = owner_info.node.expect_item();
            match &item.kind {
                ItemKind::Impl(ImplDef { items, .. }) => {
                    if let Some(&nom) = self.impl_self_nominal.get(&(pkg.pkg_id, owner_id)) {
                        for &c in items {
                            owner_enclosing_nominal.insert(c, nom);
                        }
                    }
                }
                ItemKind::Struct(StructDef { nested_items, .. })
                | ItemKind::Enum(EnumDef { nested_items, .. }) => {
                    if let Some(def_id) = pkg.hir_pkg.owner_definition(owner_id) {
                        for &c in nested_items {
                            owner_enclosing_nominal.insert(c, def_id);
                        }
                    }
                }
                ItemKind::Trait(TraitDef { items, .. }) => {
                    if let Some(def_id) = pkg.hir_pkg.owner_definition(owner_id) {
                        for &c in items {
                            owner_enclosing_nominal.insert(c, def_id);
                        }
                    }
                }
                _ => {}
            }
        }

        for (owner_id, owner_info) in pkg.hir_pkg.owners() {
            let item = owner_info.node.expect_item();
            let self_nominal = owner_enclosing_nominal
                .get(&owner_id)
                .copied()
                .or_else(|| self.impl_self_nominal.get(&(pkg.pkg_id, owner_id)).copied());

            let ctx = WalkCtx { pkg, self_nominal };

            // Walk item-level type/value/clause expressions and bodies!
            for clause in item_clauses(item) {
                self.walk_clause_param(&ctx, clause);
            }

            match &item.kind {
                ItemKind::Fn(sig, body_id) | ItemKind::Case(sig, body_id) => {
                    self.walk_fn_sig(&ctx, sig, Some(*body_id));
                    if let Some(body) = pkg.hir_pkg.body(*body_id) {
                        self.walk_body(&ctx, body);
                    }
                }
                ItemKind::FnDeclaration(sig) | ItemKind::Effect(sig, _) => {
                    self.walk_fn_sig(&ctx, sig, None);
                }
                ItemKind::FunctionTemplateInstance {
                    template,
                    signature,
                    body,
                    ..
                } => {
                    self.walk_expr(&ctx, template, false);
                    if let Some(sig) = signature {
                        self.walk_fn_sig(&ctx, sig, Some(*body));
                    }
                    if let Some(b) = pkg.hir_pkg.body(*body) {
                        self.walk_body(&ctx, b);
                    }
                }
                ItemKind::TestTemplateInstance {
                    template,
                    ports,
                    body,
                    ..
                } => {
                    self.walk_expr(&ctx, template, false);
                    for port in ports {
                        let pname = port.name.name.as_str();
                        if pname.is_empty() || pname == "<invalid>" || pname == "_" {
                            continue;
                        }
                        if let Some((file, sel_span)) =
                            span_to_file_offsets(self.source_map, port.name.span)
                        {
                            let key = SymbolKey::Local {
                                pkg: ctx.pkg.pkg_id,
                                hir_id: port.binding,
                            };
                            self.index.symbols.insert(
                                key,
                                SymbolInfo {
                                    key,
                                    name: pname.to_owned(),
                                    kind: lsp_symbol_kind::VARIABLE,
                                    completion_kind: lsp_completion_kind::VARIABLE,
                                    detail: format!("context {pname}"),
                                    doc: None,
                                    container_name: None,
                                    file: file.clone(),
                                    full_span: sel_span,
                                    selection_span: sel_span,
                                    children: Vec::new(),
                                    params: Vec::new(),
                                    is_user_pkg: ctx.pkg.is_user_pkg,
                                    is_mutable: false,
                                },
                            );
                            self.record_occurrence(file, sel_span, key, true, false);
                        }
                    }
                    if let Some(b) = pkg.hir_pkg.body(*body) {
                        self.walk_body(&ctx, b);
                    }
                }
                ItemKind::Struct(sdef) => {
                    for f in sdef.fields {
                        self.walk_expr(&ctx, f.ty, false);
                        if let Some(d) = f.default {
                            self.walk_expr(&ctx, d, false);
                        }
                    }
                    for c in sdef.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::Enum(edef) => {
                    self.walk_variants(&ctx, edef.variants);
                    for cfg in &edef.configurations {
                        self.walk_expr(&ctx, cfg.value, false);
                    }
                    for c in edef.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::Impl(idef) => {
                    if let Some(gt) = idef.generation_trait {
                        self.walk_expr(&ctx, gt, false);
                    }
                    self.walk_expr(&ctx, idef.self_ty, false);
                    if let Some(tr) = idef.trait_ref {
                        self.walk_expr(&ctx, tr, false);
                    }
                    for c in idef.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::Trait(tdef) => {
                    if let Some(st) = tdef.super_traits {
                        self.walk_expr(&ctx, st, false);
                    }
                    for c in tdef.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::Deriver(ddef) => {
                    for p in ddef.parameters {
                        self.walk_fn_param(&ctx, p, Some(ddef.body));
                    }
                    self.walk_expr(&ctx, ddef.trait_ref, false);
                    for c in ddef.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                    if let Some(b) = pkg.hir_pkg.body(ddef.body) {
                        self.walk_body(&ctx, b);
                    }
                }
                ItemKind::Derive(dreq) => {
                    self.walk_expr(&ctx, dreq.trait_ref, false);
                    self.walk_expr(&ctx, dreq.self_ty, false);
                    if let Some(s) = dreq.strategy {
                        self.walk_expr(&ctx, s, false);
                    }
                    for c in dreq.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::Associated(assoc) => {
                    for p in assoc.parameters {
                        self.walk_fn_param(&ctx, p, None);
                    }
                    self.walk_expr(&ctx, assoc.ty, false);
                    if let Some(d) = assoc.default {
                        self.walk_expr(&ctx, d, false);
                    }
                    for c in assoc.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::TypeAlias(ty, _, constraints) | ItemKind::Newtype(ty, _, constraints) => {
                    self.walk_expr(&ctx, ty, false);
                    for c in *constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::Const(ty, init) => {
                    self.walk_expr(&ctx, ty, false);
                    self.walk_expr(&ctx, init, false);
                }
                ItemKind::Static(ty, init) => {
                    self.walk_expr(&ctx, ty, false);
                    if let Some(init) = init {
                        self.walk_expr(&ctx, init, false);
                    }
                }
                ItemKind::Use(use_path) => {
                    let enclosing_scope = self.enclosing_scope_for_owner(pkg, owner_id, item.span);
                    self.walk_use_path(enclosing_scope, use_path, true);
                }
                ItemKind::EffectGroup(eg) => {
                    for c in eg.clause_constraints {
                        self.walk_clause_constraint(&ctx, c);
                    }
                }
                ItemKind::Mod(_) | ItemKind::Invalid => {}
            }
        }

        // Walk deferred attributes (`^attr`) as expressions so attribute paths resolve too!
        for attr in &pkg.hir_pkg.definition_attributes {
            let ctx = WalkCtx {
                pkg,
                self_nominal: None,
            };
            self.walk_expr(&ctx, attr.value, false);
        }
    }

    fn enclosing_scope_for_owner(
        &self,
        pkg: &LoweredPackageInput<'_, '_>,
        owner_id: OwnerId,
        item_span: Span,
    ) -> Option<resolve::ScopeId> {
        let mut cur = pkg.hir_pkg.lexical_parents.get(&owner_id).copied();
        while let Some(parent_owner) = cur {
            if let Some(&pdef) = self.owner_defs.get(&(pkg.pkg_id, parent_owner))
                && let Some(&scope_id) = self.def_to_scope.get(&pdef)
            {
                return Some(scope_id);
            }
            cur = pkg.hir_pkg.lexical_parents.get(&parent_owner).copied();
        }
        let (file, _) = span_to_file_offsets(self.source_map, item_span)?;
        self.path_to_scope.get(&file).copied()
    }

    fn lookup_def_in_scope(
        &self,
        scope_id: resolve::ScopeId,
        name: symbol::Symbol,
        walk_ancestors: bool,
    ) -> Option<DefId> {
        if walk_ancestors {
            for sc in self.module_tree.scope_tree.ancestors(scope_id) {
                if let Some(b) = sc
                    .items
                    .get_local(name)
                    .or_else(|| sc.items.unique_glob(name))
                    && let resolve::BindingKind::Definition(d, _)
                    | resolve::BindingKind::ClauseParam(d, _) = b.kind
                {
                    return Some(d);
                }
            }
            None
        } else {
            let sc = self.module_tree.scope_tree.get(scope_id)?;
            let b = sc
                .items
                .get_local(name)
                .or_else(|| sc.items.unique_glob(name))?;
            match b.kind {
                resolve::BindingKind::Definition(d, _)
                | resolve::BindingKind::ClauseParam(d, _) => Some(d),
                _ => None,
            }
        }
    }

    fn walk_use_path(
        &mut self,
        start_scope: Option<resolve::ScopeId>,
        use_path: &UsePath<'_>,
        is_root: bool,
    ) {
        if use_path.path.res.is_some() {
            self.walk_path_ref(&use_path.path);
        }

        let (mut cur_scope, mut first_is_lexical) = match start_scope {
            Some(s) if !is_root => (Some(s), false),
            Some(s) => match use_path.path.anchor {
                hir::common::PathAnchor::Local => (Some(s), true),
                hir::common::PathAnchor::Super(n) => {
                    let mut sid = Some(s);
                    for _ in 0..n {
                        sid = sid
                            .and_then(|id| self.module_tree.scope_tree.get(id))
                            .and_then(|sc| sc.parent);
                    }
                    (sid, false)
                }
                hir::common::PathAnchor::Package => {
                    let pkg_scope = self
                        .module_tree
                        .scope_tree
                        .ancestors(s)
                        .find(|sc| sc.kind == resolve::scope::ScopeKind::Package)
                        .map(|sc| sc.id);
                    (pkg_scope, false)
                }
            },
            None => (None, false),
        };

        let mut last_def: Option<DefId> = None;
        let mut last_key: Option<SymbolKey> = None;

        for seg in use_path.path.segments {
            let walk_anc = first_is_lexical;
            first_is_lexical = false;

            let mut seg_key = None;
            if let Some(sid) = cur_scope
                && let Some(def_id) = self.lookup_def_in_scope(sid, seg.ident.name, walk_anc)
            {
                seg_key = Some(SymbolKey::Def(def_id));
                last_def = Some(def_id);
                cur_scope = self.def_to_scope.get(&def_id).copied();
            } else if let Some(nom) = last_def
                && let Some(mkey) = self.find_member_key(nom, seg.ident.name.as_str())
            {
                seg_key = Some(mkey);
                last_def = match mkey {
                    SymbolKey::Def(d) => Some(d),
                    _ => None,
                };
                cur_scope = last_def.and_then(|d| self.def_to_scope.get(&d).copied());
            } else {
                cur_scope = None;
                last_def = None;
            }

            if let Some(k) = seg_key {
                if let Some((file, span)) = span_to_file_offsets(self.source_map, seg.ident.span) {
                    self.record_occurrence(file, span, k, false, false);
                }
                last_key = Some(k);
            } else {
                last_key = None;
            }
        }

        match use_path.kind {
            UseKind::Simple | UseKind::Glob => {}
            UseKind::Alias(alias_ident) => {
                if let Some(k) = last_key
                    && let Some((file, span)) =
                        span_to_file_offsets(self.source_map, alias_ident.span)
                {
                    self.record_occurrence(file, span, k, false, false);
                }
            }
            UseKind::Multi(idents) => {
                for ident in idents {
                    let mut mkey = None;
                    if let Some(sid) = cur_scope
                        && let Some(def_id) = self.lookup_def_in_scope(sid, ident.name, false)
                    {
                        mkey = Some(SymbolKey::Def(def_id));
                    } else if let Some(nom) = last_def {
                        mkey = self.find_member_key(nom, ident.name.as_str());
                    }
                    if let Some(k) = mkey
                        && let Some((file, span)) =
                            span_to_file_offsets(self.source_map, ident.span)
                    {
                        self.record_occurrence(file, span, k, false, false);
                    }
                }
            }
            UseKind::Group(children) => {
                for child in children {
                    self.walk_use_path(cur_scope, child, false);
                }
            }
        }
    }

    fn walk_clause_param(&mut self, ctx: &WalkCtx<'_, '_>, clause: &ClauseParam<'_>) {
        self.walk_clause_param_kind(ctx, &clause.kind);
    }

    fn walk_clause_param_kind(&mut self, ctx: &WalkCtx<'_, '_>, kind: &ClauseParamKind<'_>) {
        match kind {
            ClauseParamKind::Type(_) => {}
            ClauseParamKind::Positional(_, ty)
            | ClauseParamKind::Varadic(_, ty)
            | ClauseParamKind::TraitBound(_, ty) => {
                self.walk_expr(ctx, ty, false);
            }
            ClauseParamKind::Optional(_, ty, def) => {
                self.walk_expr(ctx, ty, false);
                self.walk_expr(ctx, def, false);
            }
            ClauseParamKind::Quote(inner) => self.walk_clause_param_kind(ctx, inner),
        }
    }

    fn walk_clause_constraint(&mut self, ctx: &WalkCtx<'_, '_>, c: &ClauseConstraint<'_>) {
        match c.kind {
            ClauseConstraintKind::Requires(e)
            | ClauseConstraintKind::Ensures(e)
            | ClauseConstraintKind::Decreases(e) => {
                self.walk_expr(ctx, e, false);
            }
            ClauseConstraintKind::Outcome => {}
        }
    }

    fn walk_variants(&mut self, ctx: &WalkCtx<'_, '_>, variants: &[Variant<'_>]) {
        for v in variants {
            match v.kind {
                VariantKind::Unit => {}
                VariantKind::Pattern(p) => self.walk_pattern(ctx, p, None),
                VariantKind::Const(e) => self.walk_expr(ctx, e, false),
                VariantKind::Tuple(tys) => {
                    for t in tys {
                        self.walk_expr(ctx, t, false);
                    }
                }
                VariantKind::Struct(fields) => {
                    for f in fields {
                        self.walk_expr(ctx, f.ty, false);
                        if let Some(d) = f.default {
                            self.walk_expr(ctx, d, false);
                        }
                    }
                }
                VariantKind::SubEnum(sub) => self.walk_variants(ctx, sub),
            }
        }
    }

    fn walk_fn_sig(&mut self, ctx: &WalkCtx<'_, '_>, sig: &FnSig<'_>, body_id: Option<BodyId>) {
        let body = body_id.and_then(|id| ctx.pkg.hir_pkg.body(id));
        for (i, param) in sig.params.iter().enumerate() {
            // Note: `ast_lowering` binds `ResolvedFnParam` to `BodyParam.hir_id` when a body exists,
            // or `FnParam.hir_id` for bodyless declarations! Register both if distinct.
            let target_hir_id = body
                .and_then(|b| b.params.get(i))
                .map(|bp| bp.hir_id)
                .unwrap_or(param.hir_id);
            self.register_param_symbol(ctx, param, target_hir_id);
            if let Some(ta) = param.template_annotation {
                self.walk_expr(ctx, ta, false);
            }
            self.walk_ty_param(ctx, &param.ty);
        }
        if let Some(ret) = sig.return_ty {
            self.walk_expr(ctx, ret, false);
        }
        for c in sig.clause_constraints {
            self.walk_clause_constraint(ctx, c);
        }
    }

    fn walk_fn_param(
        &mut self,
        ctx: &WalkCtx<'_, '_>,
        param: &FnParam<'_>,
        body_id: Option<BodyId>,
    ) {
        let target_hir_id = body_id
            .and_then(|id| ctx.pkg.hir_pkg.body(id))
            .and_then(|b| b.params.get(param.index as usize))
            .map(|bp| bp.hir_id)
            .unwrap_or(param.hir_id);
        self.register_param_symbol(ctx, param, target_hir_id);
        self.walk_ty_param(ctx, &param.ty);
    }

    fn register_param_symbol(
        &mut self,
        ctx: &WalkCtx<'_, '_>,
        param: &FnParam<'_>,
        target_hir_id: HirId,
    ) {
        let pname = param.name.name.as_str();
        if pname.is_empty() || pname == "<invalid>" || pname == "_" {
            return;
        }
        let Some((file, full_span)) = span_to_file_offsets(self.source_map, param.span) else {
            return;
        };
        let sel_span = span_to_file_offsets(self.source_map, param.name.span)
            .filter(|(f, s)| f == &file && s.1 > s.0)
            .map(|(_, s)| s)
            .unwrap_or(full_span);

        // Staged comptime parameters may also have a `DefId` in `staged_parameter_definitions`!
        let key = if let Some(&staged_def) = ctx
            .pkg
            .hir_pkg
            .staged_parameter_definitions
            .get(&param.hir_id)
        {
            SymbolKey::Def(staged_def)
        } else {
            SymbolKey::Local {
                pkg: ctx.pkg.pkg_id,
                hir_id: target_hir_id,
            }
        };

        let detail = format_fn_param(param, self.source_map);
        self.index.symbols.insert(
            key,
            SymbolInfo {
                key,
                name: pname.to_owned(),
                kind: lsp_symbol_kind::VARIABLE,
                completion_kind: lsp_completion_kind::VARIABLE,
                detail,
                doc: None,
                container_name: None,
                file: file.clone(),
                full_span,
                selection_span: sel_span,
                children: Vec::new(),
                params: Vec::new(),
                is_user_pkg: ctx.pkg.is_user_pkg,
                is_mutable: false,
            },
        );
        self.record_occurrence(file, sel_span, key, true, false);

        // Record receiver nominal type hint (`x: Point`, `x: *Point`, `self`, `*self`)!
        let hint = match &param.ty.kind {
            TyParamKind::Itself { .. } => ctx.self_nominal,
            TyParamKind::Positional(ty)
            | TyParamKind::PositionalDependencyCatched(_, ty)
            | TyParamKind::Optional(_, ty, _)
            | TyParamKind::Varadic(_, ty) => extract_head_def_id(ty),
            TyParamKind::Case(_) => None,
        };
        if let Some(nom) = hint {
            self.index.local_type_hints.insert(key, nom);
            if target_hir_id != param.hir_id {
                self.index.local_type_hints.insert(
                    SymbolKey::Local {
                        pkg: ctx.pkg.pkg_id,
                        hir_id: param.hir_id,
                    },
                    nom,
                );
            }
        }
    }

    fn walk_ty_param(&mut self, ctx: &WalkCtx<'_, '_>, tp: &TyParam<'_>) {
        match tp.kind {
            TyParamKind::Itself { .. } => {}
            TyParamKind::Positional(ty)
            | TyParamKind::PositionalDependencyCatched(_, ty)
            | TyParamKind::Varadic(_, ty) => {
                self.walk_expr(ctx, ty, false);
            }
            TyParamKind::Optional(_, ty, def) => {
                self.walk_expr(ctx, ty, false);
                self.walk_expr(ctx, def, false);
            }
            TyParamKind::Case(pat) => {
                self.walk_pattern(ctx, pat, None);
            }
        }
    }

    fn walk_body(&mut self, ctx: &WalkCtx<'_, '_>, body: &Body<'_>) {
        // Ensure closure parameters (which appear in `body.params` when not already added by `FnSig`)
        // are registered too.
        for bp in body.params {
            let key = SymbolKey::Local {
                pkg: ctx.pkg.pkg_id,
                hir_id: bp.hir_id,
            };
            if !self.index.symbols.contains_key(&key) {
                let pname = bp.name.name.as_str();
                if !pname.is_empty()
                    && pname != "<invalid>"
                    && pname != "_"
                    && let Some((file, full_span)) = span_to_file_offsets(self.source_map, bp.span)
                {
                    let sel_span = span_to_file_offsets(self.source_map, bp.name.span)
                        .filter(|(f, s)| f == &file && s.1 > s.0)
                        .map(|(_, s)| s)
                        .unwrap_or(full_span);
                    self.index.symbols.insert(
                        key,
                        SymbolInfo {
                            key,
                            name: pname.to_owned(),
                            kind: lsp_symbol_kind::VARIABLE,
                            completion_kind: lsp_completion_kind::VARIABLE,
                            detail: format!(
                                "{pname}: {}",
                                format_ty_param(&bp.ty, self.source_map)
                            ),
                            doc: None,
                            container_name: None,
                            file: file.clone(),
                            full_span,
                            selection_span: sel_span,
                            children: Vec::new(),
                            params: Vec::new(),
                            is_user_pkg: ctx.pkg.is_user_pkg,
                            is_mutable: false,
                        },
                    );
                    self.record_occurrence(file, sel_span, key, true, false);
                }
            }
        }
        self.walk_expr(ctx, body.value, false);
    }

    fn walk_block(&mut self, ctx: &WalkCtx<'_, '_>, block: &Block<'_>) {
        for stmt in block.stmts {
            self.walk_expr(ctx, stmt, false);
        }
        if let Some(tail) = block.expr {
            self.walk_expr(ctx, tail, false);
        }
    }

    fn walk_expr(&mut self, ctx: &WalkCtx<'_, '_>, expr: &Expr<'_>, is_write: bool) {
        match &expr.kind {
            ExprKind::ResolvedLocal(_, target_hir_id)
            | ExprKind::ResolvedFnParam(_, target_hir_id)
            | ExprKind::ResolvedLoopLabel(_, target_hir_id) => {
                if let Some((file, span)) = span_to_file_offsets(self.source_map, expr.span) {
                    let key = if let Some(&staged_def) = ctx
                        .pkg
                        .hir_pkg
                        .staged_parameter_definitions
                        .get(target_hir_id)
                    {
                        SymbolKey::Def(staged_def)
                    } else {
                        SymbolKey::Local {
                            pkg: ctx.pkg.pkg_id,
                            hir_id: *target_hir_id,
                        }
                    };
                    self.record_occurrence(file, span, key, false, is_write);
                }
            }
            ExprKind::ResolvedClauseParam(_, def_id) | ExprKind::ResolvedDef { def_id, .. } => {
                if def_id.is_valid()
                    && let Some((file, span)) = span_to_file_offsets(self.source_map, expr.span)
                {
                    self.record_occurrence(file, span, SymbolKey::Def(*def_id), false, is_write);
                }
            }
            ExprKind::SelfValue | ExprKind::TySelf => {
                if let Some(nom_def) = ctx.self_nominal
                    && let Some((file, span)) = span_to_file_offsets(self.source_map, expr.span)
                {
                    self.record_occurrence(file, span, SymbolKey::Def(nom_def), false, false);
                }
            }
            ExprKind::Path(path) => {
                self.walk_path_ref(path);
                for seg in path.segments {
                    for arg in seg.args {
                        self.walk_arg(ctx, arg);
                    }
                }
            }
            ExprKind::Projection(base, field_ident) => {
                self.walk_expr(ctx, base, false);
                // Resolve projection target via:
                // 1) `hir_pkg.member_projections`
                // 2) `hir_pkg.deferred_member_projections`
                // 3) receiver nominal type hint (`local_type_hints` / `self_nominal` / constructor)
                let member_key =
                    self.resolve_projection_target(ctx, expr.hir_id, base, field_ident);
                if let Some(target_key) = member_key
                    && let Some((file, span)) =
                        span_to_file_offsets(self.source_map, field_ident.span)
                {
                    self.record_occurrence(file, span, target_key, false, is_write);
                }
            }
            ExprKind::Let(let_decl) => {
                self.walk_let_decl(ctx, let_decl);
            }
            ExprKind::Assign(lhs, rhs) | ExprKind::AssignOp(_, lhs, rhs) => {
                self.walk_expr(ctx, lhs, true);
                self.walk_expr(ctx, rhs, false);
            }
            ExprKind::Application(callee, args)
            | ExprKind::ExtendedApplication(callee, args)
            | ExprKind::NFApplication(callee, args) => {
                let callee_nominal = extract_head_def_id(callee);
                self.walk_expr(ctx, callee, false);
                for arg in *args {
                    if let Arg::Named(ident, val) = arg {
                        if let Some(nom) = callee_nominal
                            && let Some(fkey) = self.find_member_key(nom, ident.name.as_str())
                            && let Some((file, span)) =
                                span_to_file_offsets(self.source_map, ident.span)
                        {
                            self.record_occurrence(file, span, fkey, false, false);
                        }
                        self.walk_expr(ctx, val, false);
                    } else {
                        self.walk_arg(ctx, arg);
                    }
                }
            }
            ExprKind::CollectedProperties(fields) => {
                for f in *fields {
                    self.walk_expr(ctx, f.expr, false);
                }
            }
            ExprKind::Object(entries) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Value(v) => self.walk_expr(ctx, v, false),
                        ObjectEntry::Property(f) => self.walk_expr(ctx, f.expr, false),
                    }
                }
            }
            ExprKind::Binary(_, a, b)
            | ExprKind::Index(a, b)
            | ExprKind::Cast(a, b)
            | ExprKind::DynCast(a, b)
            | ExprKind::TyClosure(a, b)
            | ExprKind::TyFnArrow(a, b)
            | ExprKind::ErrorQualifiedType(a, b)
            | ExprKind::EffectQualifiedType(a, b)
            | ExprKind::TraitBound(a, b) => {
                self.walk_expr(ctx, a, false);
                self.walk_expr(ctx, b, false);
            }
            ExprKind::RefinedType { requirement, base } => {
                self.walk_expr(ctx, requirement, false);
                self.walk_expr(ctx, base, false);
            }
            ExprKind::Unary(_, inner)
            | ExprKind::EffectRequest(inner)
            | ExprKind::Assumes(inner)
            | ExprKind::Asserts(inner)
            | ExprKind::Invariant(inner)
            | ExprKind::Decreases(inner)
            | ExprKind::Defer(inner)
            | ExprKind::InlineControl(inner)
            | ExprKind::OptionPropagate(inner)
            | ExprKind::Ref(inner)
            | ExprKind::Deref(inner)
            | ExprKind::ErrorNew(inner)
            | ExprKind::ErrorPropagate(inner)
            | ExprKind::Semi(inner)
            | ExprKind::TyPtr(inner)
            | ExprKind::TyOptional(inner)
            | ExprKind::TyTraitObject(inner)
            | ExprKind::LiftType(inner)
            | ExprKind::TakeOld(inner)
            | ExprKind::TakeBitVec(inner)
            | ExprKind::TakeTagName(inner)
            | ExprKind::TakeAttribute(inner)
            | ExprKind::TakeIntrinsic(inner)
            | ExprKind::TakeTupleEnumerate(inner)
            | ExprKind::TakeTupleIndexed(inner)
            | ExprKind::TypeOf(inner)
            | ExprKind::MultiDeref(inner)
            | ExprKind::TacticArgument(inner) => {
                self.walk_expr(ctx, inner, false);
            }
            ExprKind::LiteralConversion { literal, .. } => {
                self.walk_expr(ctx, literal, false);
            }
            ExprKind::CodeQuote(inner, subs) | ExprKind::CodeSplice(inner, subs) => {
                self.walk_expr(ctx, inner, false);
                for sub in subs {
                    self.walk_expr(ctx, sub.binding, false);
                }
            }
            ExprKind::CodePatternQuote(pat, subs) => {
                self.walk_pattern(ctx, pat, None);
                for sub in subs {
                    self.walk_expr(ctx, sub.binding, false);
                }
            }
            ExprKind::ContextualExprType(bindings, inner)
            | ExprKind::ContextualPatternType(inner, bindings) => {
                for b in bindings {
                    self.walk_expr(ctx, b.ty, false);
                    if let Some(d) = b.default {
                        self.walk_expr(ctx, d, false);
                    }
                }
                self.walk_expr(ctx, inner, false);
            }
            ExprKind::ConstantObjectEntry(inner, _) => {
                self.walk_expr(ctx, inner, false);
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.walk_expr(ctx, s, false);
                }
                if let Some(e) = end {
                    self.walk_expr(ctx, e, false);
                }
            }
            ExprKind::If(cond, then_blk, else_expr) => {
                self.walk_expr(ctx, cond, false);
                self.walk_block(ctx, then_blk);
                if let Some(el) = else_expr {
                    self.walk_expr(ctx, el, false);
                }
            }
            ExprKind::InlineIf {
                cond,
                then_expr,
                else_expr,
            } => {
                self.walk_expr(ctx, cond, false);
                self.walk_expr(ctx, then_expr, false);
                if let Some(el) = else_expr {
                    self.walk_expr(ctx, el, false);
                }
            }
            ExprKind::When(arms) => {
                for CondictionArm { cond, body, .. } in *arms {
                    if let Some(c) = cond {
                        self.walk_expr(ctx, c, false);
                    }
                    self.walk_expr(ctx, body, false);
                }
            }
            ExprKind::Block(blk) | ExprKind::UnsafeBlock(blk) | ExprKind::Loop(blk) => {
                self.walk_block(ctx, blk);
            }
            ExprKind::Match(scrutinee, arms) | ExprKind::EffectHandle(scrutinee, arms) => {
                let scrutinee_nom = self.infer_expr_nominal(ctx, scrutinee);
                self.walk_expr(ctx, scrutinee, false);
                for arm in *arms {
                    if let Some(import_expr) = ctx.pkg.hir_pkg.match_arm_imports.get(&arm.hir_id) {
                        self.walk_expr(ctx, import_expr, false);
                    } else {
                        self.walk_pattern_arm(ctx, arm, scrutinee_nom);
                    }
                }
            }
            ExprKind::InlineMatch(arms) => {
                for arm in *arms {
                    self.walk_pattern_arm(ctx, arm, None);
                }
            }
            ExprKind::ErrorHandle {
                value,
                before,
                arms,
                after,
            } => {
                self.walk_expr(ctx, value, false);
                if let Some(b) = before {
                    self.walk_pattern_arm(ctx, b, None);
                }
                for arm in *arms {
                    self.walk_pattern_arm(ctx, arm, None);
                }
                if let Some(a) = after {
                    self.walk_pattern_arm(ctx, a, None);
                }
            }
            ExprKind::Return(opt) | ExprKind::Resume(opt) => {
                if let Some(v) = opt {
                    self.walk_expr(ctx, v, false);
                }
            }
            ExprKind::BreakTo(target_hir) | ExprKind::ContinueTo(target_hir) => {
                if let Some((file, span)) = span_to_file_offsets(self.source_map, expr.span) {
                    let key = SymbolKey::Local {
                        pkg: ctx.pkg.pkg_id,
                        hir_id: *target_hir,
                    };
                    self.record_occurrence(file, span, key, false, false);
                }
            }
            ExprKind::Tuple(elems) | ExprKind::List(elems) => {
                for e in *elems {
                    self.walk_expr(ctx, e, false);
                }
            }
            ExprKind::Closure(params, ret_ty, body_id) => {
                for ClosureParam { pat, ty, .. } in *params {
                    let hint = ty.and_then(extract_head_def_id);
                    self.walk_pattern(ctx, pat, hint);
                    if let Some(t) = ty {
                        self.walk_expr(ctx, t, false);
                    }
                }
                if let Some(rt) = ret_ty {
                    self.walk_expr(ctx, rt, false);
                }
                if let Some(b) = ctx.pkg.hir_pkg.body(*body_id) {
                    self.walk_body(ctx, b);
                }
            }
            ExprKind::PostLambdaBody(body_id) => {
                if let Some(b) = ctx.pkg.hir_pkg.body(*body_id) {
                    self.walk_body(ctx, b);
                }
            }
            ExprKind::For {
                label,
                pat,
                guard,
                iter,
                body,
            } => {
                if let Some(lbl) = label
                    && let Some((file, span)) = span_to_file_offsets(self.source_map, lbl.span)
                {
                    let lkey = SymbolKey::Local {
                        pkg: ctx.pkg.pkg_id,
                        hir_id: expr.hir_id,
                    };
                    self.index.symbols.insert(
                        lkey,
                        SymbolInfo {
                            key: lkey,
                            name: lbl.name.as_str().to_owned(),
                            kind: lsp_symbol_kind::CONSTANT,
                            completion_kind: lsp_completion_kind::CONSTANT,
                            detail: format!("loop label {}", lbl.name),
                            doc: None,
                            container_name: None,
                            file: file.clone(),
                            full_span: span,
                            selection_span: span,
                            children: Vec::new(),
                            params: Vec::new(),
                            is_user_pkg: ctx.pkg.is_user_pkg,
                            is_mutable: false,
                        },
                    );
                    self.record_occurrence(file, span, lkey, true, false);
                }
                self.walk_pattern(ctx, pat, None);
                if let Some(g) = guard {
                    self.walk_expr(ctx, g, false);
                }
                self.walk_expr(ctx, iter, false);
                self.walk_block(ctx, body);
            }
            ExprKind::InlineFor {
                pat, iter, body, ..
            } => {
                self.walk_pattern(ctx, pat, None);
                self.walk_expr(ctx, iter, false);
                self.walk_expr(ctx, body, false);
            }
            ExprKind::TyFn(tps)
            | ExprKind::TyComptimeFn(tps)
            | ExprKind::TyModifiedFn(_, tps)
            | ExprKind::TyNFFn(tps) => {
                for tp in *tps {
                    self.walk_ty_param(ctx, tp);
                }
            }
            ExprKind::TyClosureEnvironment(bindings) => {
                for b in *bindings {
                    self.walk_expr(ctx, b.ty, false);
                    if let Some(v) = b.value {
                        self.walk_expr(ctx, v, false);
                    }
                }
            }
            ExprKind::DeliveredType(_)
            | ExprKind::Lit(_)
            | ExprKind::UnresolvedIdent(_)
            | ExprKind::TacticParameter(_)
            | ExprKind::RefinementSubject
            | ExprKind::Break(_)
            | ExprKind::Continue(_)
            | ExprKind::Item(_)
            | ExprKind::Undefined
            | ExprKind::Null
            | ExprKind::Unit
            | ExprKind::ReachabilityType
            | ExprKind::TyPlaceholder
            | ExprKind::TyNoReturn
            | ExprKind::TyVoid
            | ExprKind::TyAny
            | ExprKind::TyType
            | ExprKind::TermTypedWith
            | ExprKind::LambdaBound
            | ExprKind::TermTraitBound
            | ExprKind::Implication
            | ExprKind::Subtype
            | ExprKind::Forall
            | ExprKind::Exist
            | ExprKind::Builtin(_)
            | ExprKind::Invalid => {}
        }
    }

    fn walk_arg(&mut self, ctx: &WalkCtx<'_, '_>, arg: &Arg<'_>) {
        match arg {
            Arg::Positional(e)
            | Arg::Named(_, e)
            | Arg::Expand(e)
            | Arg::Implicit(e)
            | Arg::PostLambda(e) => self.walk_expr(ctx, e, false),
        }
    }

    fn walk_let_decl(&mut self, ctx: &WalkCtx<'_, '_>, decl: &LetDecl<'_>) {
        let mut hint = decl.ty.and_then(extract_head_def_id);
        if hint.is_none()
            && let Some(init) = decl.init
        {
            hint = self.infer_expr_nominal(ctx, init);
        }
        if let Some(ty) = decl.ty {
            self.walk_expr(ctx, ty, false);
        }
        if let Some(init) = decl.init {
            self.walk_expr(ctx, init, false);
        }
        self.walk_let_pattern(ctx, &decl.pattern, decl, hint);
        if let Some(g) = decl.guard {
            self.walk_expr(ctx, g, false);
        }
        if let Some(oth) = decl.otherwise {
            self.walk_expr(ctx, oth, false);
        }
    }

    fn walk_let_pattern(
        &mut self,
        ctx: &WalkCtx<'_, '_>,
        pat: &Pattern<'_>,
        decl: &LetDecl<'_>,
        nominal_hint: Option<DefId>,
    ) {
        let detail_prefix = if decl.comptime {
            "comptime let"
        } else if decl.mutable {
            "var"
        } else {
            "let"
        };
        let ty_suffix = decl
            .ty
            .map(|t| format!(": {}", format_expr_compact(t, self.source_map)))
            .or_else(|| {
                nominal_hint
                    .and_then(|d| self.index.symbols.get(&SymbolKey::Def(d)))
                    .map(|s| format!(": {}", s.name))
            })
            .unwrap_or_default();
        self.walk_pattern_inner(
            ctx,
            pat,
            nominal_hint,
            detail_prefix,
            &ty_suffix,
            decl.mutable,
        );
    }

    fn walk_pattern_arm(
        &mut self,
        ctx: &WalkCtx<'_, '_>,
        arm: &PatternArm<'_>,
        nominal_hint: Option<DefId>,
    ) {
        self.walk_pattern(ctx, &arm.pat, nominal_hint);
        if let Some(g) = arm.guard {
            self.walk_expr(ctx, g, false);
        }
        self.walk_expr(ctx, arm.body, false);
    }

    fn walk_pattern(
        &mut self,
        ctx: &WalkCtx<'_, '_>,
        pat: &Pattern<'_>,
        nominal_hint: Option<DefId>,
    ) {
        self.walk_pattern_inner(ctx, pat, nominal_hint, "let", "", false);
    }

    fn walk_pattern_inner(
        &mut self,
        ctx: &WalkCtx<'_, '_>,
        pat: &Pattern<'_>,
        nominal_hint: Option<DefId>,
        prefix: &str,
        ty_suffix: &str,
        is_mutable: bool,
    ) {
        match &pat.kind {
            PatternKind::Binding(mode, ident, subpat) => {
                let bname = ident.name.as_str();
                if !bname.is_empty()
                    && bname != "<invalid>"
                    && bname != "_"
                    && let Some((file, span)) = span_to_file_offsets(self.source_map, ident.span)
                {
                    let key = SymbolKey::Local {
                        pkg: ctx.pkg.pkg_id,
                        hir_id: pat.hir_id,
                    };
                    let ref_kw = match mode {
                        hir::BindingMode::ByRef => "ref ",
                        hir::BindingMode::ByValue => "",
                    };
                    self.index.symbols.insert(
                        key,
                        SymbolInfo {
                            key,
                            name: bname.to_owned(),
                            kind: lsp_symbol_kind::VARIABLE,
                            completion_kind: lsp_completion_kind::VARIABLE,
                            detail: format!("{prefix} {ref_kw}{bname}{ty_suffix}"),
                            doc: None,
                            container_name: None,
                            file: file.clone(),
                            full_span: span,
                            selection_span: span,
                            children: Vec::new(),
                            params: Vec::new(),
                            is_user_pkg: ctx.pkg.is_user_pkg,
                            is_mutable,
                        },
                    );
                    self.record_occurrence(file, span, key, true, false);
                    if let Some(nom) = nominal_hint {
                        self.index.local_type_hints.insert(key, nom);
                    }
                }
                if let Some(sub) = subpat {
                    self.walk_pattern_inner(ctx, sub, nominal_hint, prefix, ty_suffix, is_mutable);
                }
            }
            PatternKind::Const(expr) => {
                if let ExprKind::Lit(lit) = &expr.kind
                    && let hir::common::LitKind::Symbol(sym) = lit.kind
                    && let Some(nom) = nominal_hint
                    && let Some(mkey) = self.find_member_key(nom, sym.as_str())
                    && let Some((file, span)) = span_to_file_offsets(self.source_map, expr.span)
                {
                    self.record_occurrence(file, span, mkey, false, false);
                }
                self.walk_expr(ctx, expr, false);
            }
            PatternKind::Comptime(expr) => {
                self.walk_expr(ctx, expr, false);
            }
            PatternKind::Projection(base, ident) => {
                self.walk_pattern(ctx, base, nominal_hint);
                let head_nom = pattern_head_def_id(base).or(nominal_hint);
                if let Some(nom) = head_nom
                    && let Some(mkey) = self.find_member_key(nom, ident.name.as_str())
                    && let Some((file, span)) = span_to_file_offsets(self.source_map, ident.span)
                {
                    self.record_occurrence(file, span, mkey, false, false);
                }
            }
            PatternKind::Struct(head, fields, _) | PatternKind::AppStruct(head, fields) => {
                self.walk_pattern(ctx, head, nominal_hint);
                let struct_nom = pattern_head_def_id(head).or(nominal_hint);
                self.walk_field_pats(ctx, fields, struct_nom, prefix, is_mutable);
            }
            PatternKind::AppTuple(head, args) => {
                self.walk_pattern(ctx, head, nominal_hint);
                for arg in *args {
                    self.walk_pattern_inner(ctx, arg, None, prefix, "", is_mutable);
                }
            }
            PatternKind::Tuple(elems) | PatternKind::Or(elems) => {
                for e in *elems {
                    self.walk_pattern_inner(ctx, e, None, prefix, "", is_mutable);
                }
            }
            PatternKind::List(elems, rest) => {
                for e in *elems {
                    self.walk_pattern_inner(ctx, e, None, prefix, "", is_mutable);
                }
                if let Some(r) = rest {
                    self.walk_pattern_inner(ctx, r, None, prefix, "", is_mutable);
                }
            }
            PatternKind::NormalForm(head, args) => {
                self.walk_expr(ctx, head, false);
                for a in *args {
                    self.walk_pattern_inner(ctx, &a.pattern, None, prefix, "", is_mutable);
                }
            }
            PatternKind::OptionSome(inner)
            | PatternKind::ErrorOk(inner)
            | PatternKind::ErrorErr(inner)
            | PatternKind::Not(inner)
            | PatternKind::Ref(inner) => {
                self.walk_pattern_inner(ctx, inner, nominal_hint, prefix, "", is_mutable);
            }
            PatternKind::Guard(inner, cond) => {
                self.walk_pattern_inner(ctx, inner, nominal_hint, prefix, ty_suffix, is_mutable);
                self.walk_expr(ctx, cond, false);
            }
            PatternKind::Then(first, func, second) => {
                self.walk_pattern(ctx, first, nominal_hint);
                self.walk_expr(ctx, func, false);
                self.walk_pattern(ctx, second, None);
            }
            PatternKind::CodeSplice(expr, outputs) => {
                self.walk_expr(ctx, expr, false);
                for out in outputs {
                    if let Some((file, span)) = span_to_file_offsets(self.source_map, out.name.span)
                    {
                        let key = SymbolKey::Local {
                            pkg: ctx.pkg.pkg_id,
                            hir_id: out.hir_id,
                        };
                        self.index.symbols.insert(
                            key,
                            SymbolInfo {
                                key,
                                name: out.name.name.as_str().to_owned(),
                                kind: lsp_symbol_kind::VARIABLE,
                                completion_kind: lsp_completion_kind::VARIABLE,
                                detail: format!("let {}", out.name),
                                doc: None,
                                container_name: None,
                                file: file.clone(),
                                full_span: span,
                                selection_span: span,
                                children: Vec::new(),
                                params: Vec::new(),
                                is_user_pkg: ctx.pkg.is_user_pkg,
                                is_mutable: false,
                            },
                        );
                        self.record_occurrence(file, span, key, true, false);
                    }
                }
            }
            PatternKind::Range(lo, hi, _) => {
                if let Some(l) = lo {
                    self.walk_expr(ctx, l, false);
                }
                if let Some(h) = hi {
                    self.walk_expr(ctx, h, false);
                }
            }
            PatternKind::BitVec { segments, .. } => {
                for seg in segments {
                    match seg {
                        hir::pattern::BitSegment::Literal { .. } => {}
                        hir::pattern::BitSegment::Typed { pattern, ty } => {
                            self.walk_pattern_inner(ctx, pattern, None, prefix, "", is_mutable);
                            self.walk_expr(ctx, ty, false);
                        }
                        hir::pattern::BitSegment::Computed(e) => self.walk_expr(ctx, e, false),
                    }
                }
            }
            PatternKind::Wild
            | PatternKind::OptionNull
            | PatternKind::Async
            | PatternKind::Invalid => {}
        }
    }

    fn walk_field_pats(
        &mut self,
        ctx: &WalkCtx<'_, '_>,
        fields: &[FieldPat<'_>],
        struct_nom: Option<DefId>,
        prefix: &str,
        is_mutable: bool,
    ) {
        for fp in fields {
            if let Some(nom) = struct_nom
                && let Some(fkey) = self.find_member_key(nom, fp.ident.name.as_str())
                && let Some((file, span)) = span_to_file_offsets(self.source_map, fp.ident.span)
            {
                self.record_occurrence(file, span, fkey, false, false);
            }
            self.walk_pattern_inner(ctx, &fp.pat, None, prefix, "", is_mutable);
        }
    }

    /// Walk a `hir::Path` and record occurrences for both its final segment (`path.res`)
    /// and any intermediate module/type segments we can resolve in the scope tree!
    fn walk_path_ref(&mut self, path: &hir::Path<'_>) {
        if path.segments.is_empty() {
            return;
        }
        if let Some(res_def) = path.res
            && res_def.is_valid()
            && let Some(last_seg) = path.segments.last()
            && let Some((file, span)) = span_to_file_offsets(self.source_map, last_seg.ident.span)
        {
            self.record_occurrence(file, span, SymbolKey::Def(res_def), false, false);
        }

        // For multi-segment paths `a.b.c`, walk ancestor scopes of `res` or `defs_by_name`
        // so intermediate module/type segments (`a`, `b`) also get go-to-definition!
        if path.segments.len() > 1 {
            if let Some(res_def) = path.res
                && res_def.is_valid()
            {
                // Walk backwards from `res_def`'s owning scope when possible.
                let mut cur_scope = self
                    .module_tree
                    .scope_tree
                    .iter()
                    .find(|s| {
                        s.items
                            .declarations()
                            .values()
                            .any(|b| matches!(b.kind, resolve::BindingKind::Definition(d, _) if d == res_def))
                    })
                    .map(|s| s.id);

                for seg in path.segments[..path.segments.len() - 1].iter().rev() {
                    let mut matched_def = None;
                    if let Some(sid) = cur_scope
                        && let Some(sc) = self.module_tree.scope_tree.get(sid)
                    {
                        if sc.name == Some(seg.ident.name) && sc.owner_def.is_valid() {
                            matched_def = Some(sc.owner_def);
                            cur_scope = sc.parent;
                        }
                    }
                    if matched_def.is_none()
                        && let Some(defs) = self.index.defs_by_name.get(seg.ident.name.as_str())
                    {
                        matched_def = defs.first().copied();
                    }
                    if let Some(d) = matched_def
                        && let Some((file, span)) =
                            span_to_file_offsets(self.source_map, seg.ident.span)
                    {
                        self.record_occurrence(file, span, SymbolKey::Def(d), false, false);
                    }
                }
            }
        }
    }

    fn resolve_projection_target(
        &self,
        ctx: &WalkCtx<'_, '_>,
        proj_hir_id: HirId,
        base: &Expr<'_>,
        field_ident: &Ident,
    ) -> Option<SymbolKey> {
        if let Some(&def_id) = ctx.pkg.hir_pkg.member_projections.get(&proj_hir_id)
            && def_id.is_valid()
        {
            return Some(SymbolKey::Def(def_id));
        }
        if let Some(candidates) = ctx
            .pkg
            .hir_pkg
            .deferred_member_projections
            .get(&proj_hir_id)
            && let Some(&(_, member_def)) = candidates.first()
            && member_def.is_valid()
        {
            return Some(SymbolKey::Def(member_def));
        }
        let receiver_nom = self.infer_expr_nominal(ctx, base)?;
        self.find_member_key(receiver_nom, field_ident.name.as_str())
    }

    fn find_member_key(&self, nominal: DefId, member_name: &str) -> Option<SymbolKey> {
        self.index
            .nominal_members
            .get(&nominal)?
            .iter()
            .find(|m| m.name == member_name)
            .map(|m| m.key)
    }

    /// Lightweight HIR expression nominal type inference for ZLS-style dot-navigation!
    fn infer_expr_nominal(&self, ctx: &WalkCtx<'_, '_>, expr: &Expr<'_>) -> Option<DefId> {
        match &expr.kind {
            ExprKind::SelfValue | ExprKind::TySelf => ctx.self_nominal,
            ExprKind::ResolvedLocal(_, hid) | ExprKind::ResolvedFnParam(_, hid) => {
                let key = SymbolKey::Local {
                    pkg: ctx.pkg.pkg_id,
                    hir_id: *hid,
                };
                self.index.local_type_hints.get(&key).copied()
            }
            ExprKind::ResolvedDef { def_id, .. } => Some(*def_id),
            ExprKind::Path(path) => path.res.filter(|d| d.is_valid()),
            ExprKind::Ref(inner)
            | ExprKind::Deref(inner)
            | ExprKind::MultiDeref(inner)
            | ExprKind::OptionPropagate(inner)
            | ExprKind::ErrorPropagate(inner)
            | ExprKind::EffectRequest(inner) => self.infer_expr_nominal(ctx, inner),
            ExprKind::Cast(_, target_ty) => extract_head_def_id(target_ty),
            ExprKind::Application(callee, _)
            | ExprKind::ExtendedApplication(callee, _)
            | ExprKind::NFApplication(callee, _) => {
                // Either a struct/type constructor `Point(...)` or a function whose return type
                // has a nominal head (`fn make_point() -> Point`)!
                let head = match &callee.kind {
                    ExprKind::Projection(recv, method_ident) => {
                        let recv_nom = self.infer_expr_nominal(ctx, recv)?;
                        match self.find_member_key(recv_nom, method_ident.name.as_str())? {
                            SymbolKey::Def(d) => Some(d),
                            _ => None,
                        }
                    }
                    _ => extract_head_def_id(callee),
                }?;
                if self.index.nominal_members.contains_key(&head)
                    && matches!(
                        self.index
                            .symbols
                            .get(&SymbolKey::Def(head))
                            .map(|s| s.kind),
                        Some(lsp_symbol_kind::STRUCT | lsp_symbol_kind::ENUM)
                    )
                {
                    return Some(head);
                }
                // Check if `head` is a function with a known return nominal type!
                self.fn_return_nominal(ctx, head)
            }
            ExprKind::Projection(recv, field_ident) => {
                let recv_nom = self.infer_expr_nominal(ctx, recv)?;
                let mkey = self.find_member_key(recv_nom, field_ident.name.as_str())?;
                match mkey {
                    SymbolKey::Field { owner, index } => self.field_nominal(ctx, owner, index),
                    SymbolKey::Def(d) => Some(d),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn fn_return_nominal(&self, ctx: &WalkCtx<'_, '_>, fn_def: DefId) -> Option<DefId> {
        if fn_def.pkg == ctx.pkg.pkg_id
            && let Some(owner) = ctx.pkg.hir_pkg.definition_owner(fn_def)
            && let Some(item) = ctx.pkg.hir_pkg.item(owner)
        {
            match &item.kind {
                ItemKind::Fn(sig, _)
                | ItemKind::Case(sig, _)
                | ItemKind::FnDeclaration(sig)
                | ItemKind::Effect(sig, _) => {
                    return sig.return_ty.and_then(extract_head_def_id);
                }
                _ => {}
            }
        }
        // Fallback: parse return type from indexed `detail` `fn foo(...) -> Point` across packages!
        let sym = self.index.symbols.get(&SymbolKey::Def(fn_def))?;
        let arrow = sym.detail.rfind("->")?;
        let ret_part = sym.detail[arrow + 2..].trim();
        let clean = ret_part
            .trim_start_matches(['*', '?', '!'])
            .split(['<', ' ', '{', '('])
            .next()?
            .trim();
        let last_seg = clean.rsplit('.').next()?;
        self.index.defs_by_name.get(last_seg)?.first().copied()
    }

    fn field_nominal(
        &self,
        ctx: &WalkCtx<'_, '_>,
        struct_def: DefId,
        field_idx: u32,
    ) -> Option<DefId> {
        if struct_def.pkg == ctx.pkg.pkg_id
            && let Some(owner) = ctx.pkg.hir_pkg.definition_owner(struct_def)
            && let Some(item) = ctx.pkg.hir_pkg.item(owner)
            && let ItemKind::Struct(sdef) = &item.kind
            && let Some(field) = sdef.fields.get(field_idx as usize)
        {
            return extract_head_def_id(field.ty);
        }
        let sym = self.index.symbols.get(&SymbolKey::Field {
            owner: struct_def,
            index: field_idx,
        })?;
        let colon = sym.detail.find(':')?;
        let ty_part = sym.detail[colon + 1..].split('=').next()?.trim();
        let clean = ty_part
            .trim_start_matches(['*', '?'])
            .split(['<', ' ', '('])
            .next()?
            .trim();
        let last_seg = clean.rsplit('.').next()?;
        self.index.defs_by_name.get(last_seg)?.first().copied()
    }

    fn index_visible_scope_defs(&mut self, vfs_files: &[(u32, vfs::FileId, PathBuf)]) {
        for (_, combined_file_id, path) in vfs_files {
            let Some(&file_scope) = self.module_tree.file_scopes.get(combined_file_id) else {
                continue;
            };
            let mut seen = HashSet::new();
            let mut entries = Vec::new();
            for ancestor in self.module_tree.scope_tree.ancestors(file_scope) {
                for (name, binding) in ancestor.items.declarations() {
                    if let resolve::BindingKind::Definition(def_id, _) = binding.kind
                        && binding.accessible(&self.module_tree.scope_tree, file_scope)
                    {
                        let s = name.as_str();
                        if !s.is_empty()
                            && !s.starts_with('$')
                            && !s.starts_with('<')
                            && seen.insert(s.to_owned())
                        {
                            entries.push((s.to_owned(), SymbolKey::Def(def_id)));
                        }
                    }
                }
                for imp in ancestor.items.all_imports() {
                    match imp {
                        resolve::import::ResolvedImport::Single(src_scope, name) => {
                            self.collect_imported_name(
                                *src_scope,
                                *name,
                                name.as_str(),
                                file_scope,
                                &mut seen,
                                &mut entries,
                            );
                        }
                        resolve::import::ResolvedImport::Alias {
                            source_scope,
                            original,
                            alias,
                        } => {
                            self.collect_imported_name(
                                *source_scope,
                                *original,
                                alias.as_str(),
                                file_scope,
                                &mut seen,
                                &mut entries,
                            );
                        }
                        resolve::import::ResolvedImport::Multi(src_scope, names) => {
                            for name in names {
                                self.collect_imported_name(
                                    *src_scope,
                                    *name,
                                    name.as_str(),
                                    file_scope,
                                    &mut seen,
                                    &mut entries,
                                );
                            }
                        }
                        resolve::import::ResolvedImport::Glob(src_scope) => {
                            if let Some(src) = self.module_tree.scope_tree.get(*src_scope) {
                                for (name, binding) in src.items.declarations() {
                                    if let resolve::BindingKind::Definition(def_id, _) =
                                        binding.kind
                                        && binding
                                            .accessible(&self.module_tree.scope_tree, file_scope)
                                    {
                                        let s = name.as_str();
                                        if !s.is_empty()
                                            && !s.starts_with('$')
                                            && !s.starts_with('<')
                                            && seen.insert(s.to_owned())
                                        {
                                            entries.push((s.to_owned(), SymbolKey::Def(def_id)));
                                        }
                                    }
                                }
                                // Follow re-exports inside prelude modules (`pub use ...`)!
                                for reexp in src.items.reexports() {
                                    match reexp {
                                        resolve::import::ResolvedImport::Glob(sub_scope) => {
                                            if let Some(sub) =
                                                self.module_tree.scope_tree.get(*sub_scope)
                                            {
                                                for (n, b) in sub.items.declarations() {
                                                    if let resolve::BindingKind::Definition(
                                                        def_id,
                                                        _,
                                                    ) = b.kind
                                                        && b.entry_visibility()
                                                            == resolve::binding::Visibility::Public
                                                    {
                                                        let s = n.as_str();
                                                        if !s.is_empty()
                                                            && !s.starts_with('$')
                                                            && !s.starts_with('<')
                                                            && seen.insert(s.to_owned())
                                                        {
                                                            entries.push((
                                                                s.to_owned(),
                                                                SymbolKey::Def(def_id),
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        resolve::import::ResolvedImport::Single(sub_scope, n) => {
                                            self.collect_imported_name(
                                                *sub_scope,
                                                *n,
                                                n.as_str(),
                                                file_scope,
                                                &mut seen,
                                                &mut entries,
                                            );
                                        }
                                        resolve::import::ResolvedImport::Multi(
                                            sub_scope,
                                            names,
                                        ) => {
                                            for n in names {
                                                self.collect_imported_name(
                                                    *sub_scope,
                                                    *n,
                                                    n.as_str(),
                                                    file_scope,
                                                    &mut seen,
                                                    &mut entries,
                                                );
                                            }
                                        }
                                        resolve::import::ResolvedImport::Alias {
                                            source_scope,
                                            original,
                                            alias,
                                        } => {
                                            self.collect_imported_name(
                                                *source_scope,
                                                *original,
                                                alias.as_str(),
                                                file_scope,
                                                &mut seen,
                                                &mut entries,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            self.index.file_visible_defs.insert(path.clone(), entries);
        }
    }

    fn collect_imported_name(
        &self,
        src_scope: resolve::ScopeId,
        orig_name: Symbol,
        visible_name: &str,
        requester: resolve::ScopeId,
        seen: &mut HashSet<String>,
        entries: &mut Vec<(String, SymbolKey)>,
    ) {
        let Some(src) = self.module_tree.scope_tree.get(src_scope) else {
            return;
        };
        if let Some(binding) = src
            .items
            .get_direct(orig_name)
            .or_else(|| src.items.unique_glob(orig_name))
            && let resolve::BindingKind::Definition(def_id, _) = binding.kind
            && binding.accessible(&self.module_tree.scope_tree, requester)
            && !visible_name.is_empty()
            && !visible_name.starts_with('$')
            && seen.insert(visible_name.to_owned())
        {
            entries.push((visible_name.to_owned(), SymbolKey::Def(def_id)));
        }
    }

    fn doc_at(&self, span: Span) -> Option<String> {
        if span == rustc_span::DUMMY_SP {
            return None;
        }
        let sf = self.source_map.lookup_source_file(span.lo());
        let src = sf.src.as_ref()?;
        let lo = (span.lo().0.checked_sub(sf.start_pos.0)?) as usize;
        extract_leading_doc(src, lo)
    }

    fn item_display_name(&self, _hir_pkg: &Package<'_>, item: &Item<'_>, raw_name: &str) -> String {
        if let Some(label) = decode_test_label(raw_name) {
            if label.is_empty() {
                return String::from("test");
            }
            return format!("test \"{label}\"");
        }
        match &item.kind {
            ItemKind::Impl(idef) => {
                let self_str = format_expr_compact(idef.self_ty, self.source_map);
                let phase = match idef.phase {
                    ImplementationPhase::Runtime => "",
                    ImplementationPhase::Comptime => "pure comptime ",
                };
                if let Some(tr) = idef.trait_ref {
                    format!(
                        "{phase}impl {} for {self_str}",
                        format_expr_compact(tr, self.source_map)
                    )
                } else {
                    format!("{phase}impl {self_str}")
                }
            }
            ItemKind::Derive(dreq) => {
                format!(
                    "derive {} for {}",
                    format_expr_compact(dreq.trait_ref, self.source_map),
                    format_expr_compact(dreq.self_ty, self.source_map)
                )
            }
            _ => raw_name.to_owned(),
        }
    }
}

struct WalkCtx<'a, 'hir> {
    pkg: &'a LoweredPackageInput<'a, 'hir>,
    self_nominal: Option<DefId>,
}

/// Extract the head nominal `DefId` of a type/constructor expression, unwrapping `*T`, `?T`, `!E T`,
/// `#E T`, refined types, and generic `<...>` applications (`List<T>` -> `List`).
pub fn extract_head_def_id(expr: &Expr<'_>) -> Option<DefId> {
    match &expr.kind {
        ExprKind::ResolvedDef { def_id, .. } | ExprKind::ResolvedClauseParam(_, def_id) => {
            def_id.is_valid().then_some(*def_id)
        }
        ExprKind::Path(path) => path.res.filter(|d| d.is_valid()),
        ExprKind::NFApplication(head, _)
        | ExprKind::Application(head, _)
        | ExprKind::ExtendedApplication(head, _)
        | ExprKind::TyPtr(head)
        | ExprKind::TyOptional(head)
        | ExprKind::TyTraitObject(head)
        | ExprKind::Ref(head)
        | ExprKind::Deref(head) => extract_head_def_id(head),
        ExprKind::ErrorQualifiedType(_, ok_ty) | ExprKind::EffectQualifiedType(_, ok_ty) => {
            extract_head_def_id(ok_ty)
        }
        ExprKind::RefinedType { base, .. } => extract_head_def_id(base),
        _ => None,
    }
}

fn pattern_head_def_id(pat: &Pattern<'_>) -> Option<DefId> {
    match &pat.kind {
        PatternKind::Const(e) | PatternKind::NormalForm(e, _) => extract_head_def_id(e),
        PatternKind::Projection(base, _) => pattern_head_def_id(base),
        _ => None,
    }
}

fn item_has_self_param(item: &Item<'_>) -> bool {
    match &item.kind {
        ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => {
            sig.params.first().is_some_and(|p| {
                matches!(p.ty.kind, TyParamKind::Itself { .. }) || p.name.name == "self"
            })
        }
        _ => false,
    }
}

fn item_lsp_kinds(kind: &ItemKind<'_>) -> (u32, u32) {
    match DefKind::from_item_kind(kind) {
        DefKind::Fn | DefKind::FunctionTemplateInstance | DefKind::TestTemplateInstance => {
            (lsp_symbol_kind::FUNCTION, lsp_completion_kind::FUNCTION)
        }
        DefKind::Case => (
            lsp_symbol_kind::ENUM_MEMBER,
            lsp_completion_kind::ENUM_MEMBER,
        ),
        DefKind::Effect => (lsp_symbol_kind::EVENT, lsp_completion_kind::FUNCTION),
        DefKind::EffectGroup => (lsp_symbol_kind::INTERFACE, lsp_completion_kind::INTERFACE),
        DefKind::Struct | DefKind::Newtype => {
            (lsp_symbol_kind::STRUCT, lsp_completion_kind::STRUCT)
        }
        DefKind::Enum => (lsp_symbol_kind::ENUM, lsp_completion_kind::ENUM),
        DefKind::Mod => (lsp_symbol_kind::MODULE, lsp_completion_kind::MODULE),
        DefKind::Impl | DefKind::Derive => (lsp_symbol_kind::CLASS, lsp_completion_kind::CLASS),
        DefKind::Trait | DefKind::Deriver => {
            (lsp_symbol_kind::INTERFACE, lsp_completion_kind::INTERFACE)
        }
        DefKind::Associated | DefKind::TypeAlias => {
            (lsp_symbol_kind::TYPE_PARAMETER, lsp_completion_kind::STRUCT)
        }
        DefKind::Const => (lsp_symbol_kind::CONSTANT, lsp_completion_kind::CONSTANT),
        DefKind::Static | DefKind::Use | DefKind::Invalid => {
            (lsp_symbol_kind::VARIABLE, lsp_completion_kind::VARIABLE)
        }
    }
}

/// Reconstruct clean Flurry signature text and parameter docs from HIR item descriptors.
pub fn format_item_signature(
    _pkg: &Package<'_>,
    item: &Item<'_>,
    source_map: &SourceMap,
) -> (String, Vec<ParamDoc>) {
    let name = item.ident.name.as_str();
    match &item.kind {
        ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => {
            format_fn_item_sig(name, sig, source_map, "fn")
        }
        ItemKind::Case(sig, _) => format_fn_item_sig(name, sig, source_map, "case"),
        ItemKind::Effect(sig, _) => format_fn_item_sig(name, sig, source_map, "effect"),
        ItemKind::FunctionTemplateInstance {
            signature: Some(sig),
            ..
        } => format_fn_item_sig(name, sig, source_map, "fn"),
        ItemKind::FunctionTemplateInstance { template, .. } => (
            format!("fn {name} : {}", format_expr_compact(template, source_map)),
            Vec::new(),
        ),
        ItemKind::TestTemplateInstance { template, .. } => {
            let tpl = format_expr_compact(template, source_map);
            let formatted = match decode_test_label(name) {
                Some(label) if !label.is_empty() => format!("test \"{label}\" use {tpl}"),
                _ => format!("test use {tpl}"),
            };
            (formatted, Vec::new())
        }
        ItemKind::Struct(sdef) => {
            let clauses = format_clause_list(sdef.clause_params, source_map);
            (format!("struct {name}{clauses}"), Vec::new())
        }
        ItemKind::Enum(edef) => {
            let clauses = format_clause_list(edef.clause_params, source_map);
            (format!("enum {name}{clauses}"), Vec::new())
        }
        ItemKind::Trait(tdef) => {
            let prefix = if tdef.comptime_only {
                "pure comptime trait"
            } else {
                "trait"
            };
            let clauses = format_clause_list(tdef.clause_params, source_map);
            let sup = tdef
                .super_traits
                .map(|s| format!(": {}", format_expr_compact(s, source_map)))
                .unwrap_or_default();
            (format!("{prefix} {name}{clauses}{sup}"), Vec::new())
        }
        ItemKind::Impl(idef) => {
            let phase = match idef.phase {
                ImplementationPhase::Runtime => "",
                ImplementationPhase::Comptime => "pure comptime ",
            };
            let self_str = format_expr_compact(idef.self_ty, source_map);
            let sig = if let Some(tr) = idef.trait_ref {
                format!(
                    "{phase}impl {} for {self_str}",
                    format_expr_compact(tr, source_map)
                )
            } else {
                format!("{phase}impl {self_str}")
            };
            (sig, Vec::new())
        }
        ItemKind::Mod(_) => (format!("mod {name}"), Vec::new()),
        ItemKind::TypeAlias(target, clauses, _) => {
            let cl = format_clause_list(clauses, source_map);
            (
                format!(
                    "typealias {name}{cl} = {}",
                    format_expr_compact(target, source_map)
                ),
                Vec::new(),
            )
        }
        ItemKind::Newtype(target, clauses, _) => {
            let cl = format_clause_list(clauses, source_map);
            (
                format!(
                    "newtype {name}{cl} = {}",
                    format_expr_compact(target, source_map)
                ),
                Vec::new(),
            )
        }
        ItemKind::Static(ty, init) => {
            let ty_s = format_expr_compact(ty, source_map);
            if let Some(init) = init {
                let init_s = format_expr_compact(init, source_map);
                (format!("static {name}: {ty_s} = {init_s}"), Vec::new())
            } else {
                (format!("static {name}: {ty_s}"), Vec::new())
            }
        }
        ItemKind::Const(ty, init) => {
            let ty_s = format_expr_compact(ty, source_map);
            let init_s = format_expr_compact(init, source_map);
            if ty_s == "_" || ty_s.is_empty() {
                (format!("const {name} = {init_s}"), Vec::new())
            } else {
                (format!("const {name}: {ty_s} = {init_s}"), Vec::new())
            }
        }
        ItemKind::Associated(assoc) => {
            let cl = format_clause_list(assoc.clause_params, source_map);
            (
                format!(
                    "assoc {name}{cl}: {}",
                    format_expr_compact(assoc.ty, source_map)
                ),
                Vec::new(),
            )
        }
        ItemKind::EffectGroup(eg) => {
            let cl = format_clause_list(eg.clause_params, source_map);
            (format!("effect group {name}{cl}"), Vec::new())
        }
        ItemKind::Deriver(ddef) => (
            format!(
                "deriver {name} -> {}",
                format_expr_compact(ddef.trait_ref, source_map)
            ),
            Vec::new(),
        ),
        ItemKind::Derive(dreq) => (
            format!(
                "derive {} for {}",
                format_expr_compact(dreq.trait_ref, source_map),
                format_expr_compact(dreq.self_ty, source_map)
            ),
            Vec::new(),
        ),
        ItemKind::Use(_) | ItemKind::Invalid => (name.to_owned(), Vec::new()),
    }
}

fn decode_test_label(raw_name: &str) -> Option<&str> {
    let rest = raw_name.strip_prefix("$test$")?;
    Some(rest.splitn(3, '$').nth(2).unwrap_or(""))
}

fn format_fn_item_sig(
    name: &str,
    sig: &FnSig<'_>,
    source_map: &SourceMap,
    kw: &str,
) -> (String, Vec<ParamDoc>) {
    if let Some(label) = decode_test_label(name) {
        if label.is_empty() {
            return (String::from("test"), Vec::new());
        }
        return (format!("test \"{label}\""), Vec::new());
    }
    let mods = format_fn_modifiers(sig.modifiers);
    let clauses = format_clause_list(sig.clause_params, source_map);
    let mut param_docs = Vec::with_capacity(sig.params.len());
    let mut param_strs = Vec::with_capacity(sig.params.len());
    for p in sig.params {
        let s = format_fn_param(p, source_map);
        param_docs.push(ParamDoc { label: s.clone() });
        param_strs.push(s);
    }
    let ret = sig
        .return_ty
        .map(|r| format!(" -> {}", format_expr_compact(r, source_map)))
        .unwrap_or_default();
    (
        format!(
            "{mods}{kw} {name}{clauses}({}) {ret}",
            param_strs.join(", ")
        )
        .replace(" ) ", ")")
        .replace(")  ->", ") ->")
        .trim()
        .to_owned(),
        param_docs,
    )
}

fn format_fn_modifiers(m: FnModifiers) -> String {
    let mut parts = Vec::new();
    if m.is_pure {
        parts.push("pure");
    }
    if m.is_comptime {
        parts.push("comptime");
    }
    if m.is_inline {
        parts.push("inline");
    }
    if m.is_quote {
        parts.push("quote");
    }
    if m.is_unsafe {
        parts.push("unsafe");
    }
    if m.is_spec {
        parts.push("spec");
    }
    if m.is_verified {
        parts.push("verified");
    }
    if m.is_function_template {
        parts.push("template");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{} ", parts.join(" "))
    }
}

fn format_clause_list(clauses: &[ClauseParam<'_>], source_map: &SourceMap) -> String {
    if clauses.is_empty() {
        return String::new();
    }
    let rendered: Vec<String> = clauses
        .iter()
        .map(|c| format_clause_param(c, source_map))
        .collect();
    format!("<{}>", rendered.join(", "))
}

fn format_clause_param(c: &ClauseParam<'_>, source_map: &SourceMap) -> String {
    format_clause_kind(&c.kind, source_map)
}

fn format_clause_kind(kind: &ClauseParamKind<'_>, source_map: &SourceMap) -> String {
    match kind {
        ClauseParamKind::Type(id) => id.name.as_str().to_owned(),
        ClauseParamKind::Positional(id, ty) => {
            format!("{}: {}", id.name, format_expr_compact(ty, source_map))
        }
        ClauseParamKind::Optional(id, ty, def) => format!(
            ".{}: {} = {}",
            id.name,
            format_expr_compact(ty, source_map),
            format_expr_compact(def, source_map)
        ),
        ClauseParamKind::Varadic(id, ty) => {
            format!("...{}: {}", id.name, format_expr_compact(ty, source_map))
        }
        ClauseParamKind::TraitBound(id, bound) => {
            format!("{}:- {}", id.name, format_expr_compact(bound, source_map))
        }
        ClauseParamKind::Quote(inner) => format!("quote {}", format_clause_kind(inner, source_map)),
    }
}

fn format_fn_param(param: &FnParam<'_>, source_map: &SourceMap) -> String {
    match &param.ty.kind {
        TyParamKind::Itself { is_ref: true } => String::from("*self"),
        TyParamKind::Itself { is_ref: false } => String::from("self"),
        _ => {
            let mut prefix = String::new();
            if param.ty.is_implicit() {
                prefix.push_str("implicit ");
            }
            if param.ty.is_comptime() {
                prefix.push_str("comptime ");
            }
            if param.ty.is_quote() {
                prefix.push_str("quote ");
            }
            let pname = param.name.name.as_str();
            match &param.ty.kind {
                TyParamKind::Positional(ty) | TyParamKind::PositionalDependencyCatched(_, ty) => {
                    format!("{prefix}{pname}: {}", format_expr_compact(ty, source_map))
                }
                TyParamKind::Optional(_, ty, def) => format!(
                    "{prefix}.{pname}: {} = {}",
                    format_expr_compact(ty, source_map),
                    format_expr_compact(def, source_map)
                ),
                TyParamKind::Varadic(_, ty) => {
                    format!(
                        "{prefix}...{pname}: {}",
                        format_expr_compact(ty, source_map)
                    )
                }
                TyParamKind::Case(_) => format!("{prefix}case ..."),
                TyParamKind::Itself { .. } => unreachable!(),
            }
        }
    }
}

fn format_ty_param(tp: &TyParam<'_>, source_map: &SourceMap) -> String {
    match &tp.kind {
        TyParamKind::Itself { is_ref: true } => String::from("*Self"),
        TyParamKind::Itself { is_ref: false } => String::from("Self"),
        TyParamKind::Positional(ty) | TyParamKind::PositionalDependencyCatched(_, ty) => {
            format_expr_compact(ty, source_map)
        }
        TyParamKind::Optional(id, ty, def) => format!(
            ".{}: {} = {}",
            id.name,
            format_expr_compact(ty, source_map),
            format_expr_compact(def, source_map)
        ),
        TyParamKind::Varadic(id, ty) => {
            format!("...{}: {}", id.name, format_expr_compact(ty, source_map))
        }
        TyParamKind::Case(_) => String::from("case ..."),
    }
}

fn format_variant(enum_name: &str, v: &Variant<'_>, source_map: &SourceMap) -> String {
    let vname = v.ident.name.as_str();
    match &v.kind {
        VariantKind::Unit => format!("{enum_name}.{vname}"),
        VariantKind::Tuple(tys) => {
            let parts: Vec<_> = tys
                .iter()
                .map(|t| format_expr_compact(t, source_map))
                .collect();
            format!("{enum_name}.{vname}({})", parts.join(", "))
        }
        VariantKind::Struct(fields) => {
            let parts: Vec<_> = fields
                .iter()
                .map(|f| {
                    format!(
                        "{}: {}",
                        f.ident.name,
                        format_expr_compact(f.ty, source_map)
                    )
                })
                .collect();
            format!("{enum_name}.{vname} {{ {} }}", parts.join(", "))
        }
        VariantKind::Const(e) => format!(
            "{enum_name}.{vname} = {}",
            format_expr_compact(e, source_map)
        ),
        VariantKind::Pattern(_) => format!("{enum_name}.{vname}"),
        VariantKind::SubEnum(_) => format!("enum {enum_name}.{vname}"),
    }
}

/// Slice clean source text for an expression when single-line & short, or reconstruct from HIR.
pub fn format_expr_compact(expr: &Expr<'_>, source_map: &SourceMap) -> String {
    if expr.span != rustc_span::DUMMY_SP {
        let sf = source_map.lookup_source_file(expr.span.lo());
        if let Some(src) = &sf.src {
            let start = sf.start_pos.0;
            if expr.span.lo().0 >= start && expr.span.hi().0 >= expr.span.lo().0 {
                let lo = (expr.span.lo().0 - start) as usize;
                let hi = ((expr.span.hi().0 - start) as usize).min(src.len());
                if lo <= hi && src.is_char_boundary(lo) && src.is_char_boundary(hi) {
                    let slice = src[lo..hi].trim();
                    if !slice.is_empty() && !slice.contains('\n') && slice.len() <= 96 {
                        return slice.to_owned();
                    }
                }
            }
        }
    }
    match &expr.kind {
        ExprKind::Path(p) => p.to_string(),
        ExprKind::ResolvedDef { name, .. }
        | ExprKind::ResolvedClauseParam(name, _)
        | ExprKind::ResolvedLocal(name, _)
        | ExprKind::ResolvedFnParam(name, _)
        | ExprKind::UnresolvedIdent(name) => name.as_str().to_owned(),
        ExprKind::TyPtr(inner) => format!("*{}", format_expr_compact(inner, source_map)),
        ExprKind::TyOptional(inner) => format!("?{}", format_expr_compact(inner, source_map)),
        ExprKind::TyTraitObject(inner) => format!("dyn {}", format_expr_compact(inner, source_map)),
        ExprKind::LiftType(inner) => format!("lift {}", format_expr_compact(inner, source_map)),
        ExprKind::ErrorQualifiedType(err, ok) => format!(
            "!{} {}",
            format_expr_compact(err, source_map),
            format_expr_compact(ok, source_map)
        ),
        ExprKind::EffectQualifiedType(eff, ok) => format!(
            "#{} {}",
            format_expr_compact(eff, source_map),
            format_expr_compact(ok, source_map)
        ),
        ExprKind::TyType => String::from("Type"),
        ExprKind::TySelf => String::from("Self"),
        ExprKind::TyVoid => String::from("void"),
        ExprKind::TyAny => String::from("any"),
        ExprKind::TyNoReturn => String::from("noreturn"),
        ExprKind::TyPlaceholder => String::from("_"),
        ExprKind::Null => String::from("null"),
        ExprKind::Undefined => String::from("undefined"),
        ExprKind::Unit => String::from("()"),
        ExprKind::Builtin(sym) => format!(".{}'builtin", sym),
        _ => String::from("..."),
    }
}

/// Lexical fallback outline & local-type hints for broken buffers that cannot currently parse.
pub fn lexical_fallback_symbols(file: &Path, src: &str) -> Vec<SymbolInfo> {
    let (tokens, _, _) = lex::lex(src, BytePos(0));
    let mut out = Vec::new();
    let mut i = 1usize;
    let mut synth_idx = 0u32;
    while i < tokens.len() {
        let tk = tokens[i].kind;
        let item_kind = match tk {
            TokenKind::Fn => Some((
                lsp_symbol_kind::FUNCTION,
                lsp_completion_kind::FUNCTION,
                "fn",
            )),
            TokenKind::Struct => Some((
                lsp_symbol_kind::STRUCT,
                lsp_completion_kind::STRUCT,
                "struct",
            )),
            TokenKind::Enum => Some((lsp_symbol_kind::ENUM, lsp_completion_kind::ENUM, "enum")),
            TokenKind::Trait => Some((
                lsp_symbol_kind::INTERFACE,
                lsp_completion_kind::INTERFACE,
                "trait",
            )),
            TokenKind::Mod => Some((lsp_symbol_kind::MODULE, lsp_completion_kind::MODULE, "mod")),
            TokenKind::Const => Some((
                lsp_symbol_kind::CONSTANT,
                lsp_completion_kind::CONSTANT,
                "const",
            )),
            TokenKind::Typealias => Some((
                lsp_symbol_kind::TYPE_PARAMETER,
                lsp_completion_kind::STRUCT,
                "typealias",
            )),
            TokenKind::Newtype => Some((
                lsp_symbol_kind::STRUCT,
                lsp_completion_kind::STRUCT,
                "newtype",
            )),
            TokenKind::Effect => Some((
                lsp_symbol_kind::EVENT,
                lsp_completion_kind::FUNCTION,
                "effect",
            )),
            TokenKind::Case
                if tokens
                    .get(i + 2)
                    .is_some_and(|tok| tok.kind == TokenKind::LParen) =>
            {
                Some((
                    lsp_symbol_kind::ENUM_MEMBER,
                    lsp_completion_kind::ENUM_MEMBER,
                    "case",
                ))
            }
            _ => None,
        };
        if let Some((sk, ck, kw)) = item_kind
            && let Some(id_tok) = tokens.get(i + 1)
            && id_tok.kind == TokenKind::Id
        {
            let name = &src[id_tok.from..id_tok.to];
            let key = SymbolKey::Def(DefId::new(u32::MAX - 1, synth_idx));
            synth_idx += 1;
            out.push(SymbolInfo {
                key,
                name: name.to_owned(),
                kind: sk,
                completion_kind: ck,
                detail: format!("{kw} {name}"),
                doc: extract_leading_doc(src, tokens[i].from),
                container_name: None,
                file: file.to_path_buf(),
                full_span: (tokens[i].from, id_tok.to),
                selection_span: (id_tok.from, id_tok.to),
                children: Vec::new(),
                params: Vec::new(),
                is_user_pkg: true,
                is_mutable: false,
            });
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

/// Scan tokens before `cursor_offset` to infer local variable type names even in broken buffers!
/// Handles `let x: Type`, `var x: *Type`, `fn foo(x: Type)`, `let x = Type { ... }`, `let x = Type(...)`.
pub fn lexical_local_types_before(src: &str, cursor_offset: usize) -> HashMap<String, String> {
    let clamped = cursor_offset.min(src.len());
    let (tokens, _, _) = lex::lex(&src[..clamped], BytePos(0));
    let sig: Vec<Token> = tokens
        .into_iter()
        .filter(|t| !matches!(t.kind, TokenKind::Sof | TokenKind::Eof | TokenKind::Comment))
        .collect();
    let mut map = HashMap::new();
    for idx in 0..sig.len() {
        let tok = sig[idx];
        if tok.kind != TokenKind::Id && tok.kind != TokenKind::SelfLower {
            continue;
        }
        let var_name = &src[tok.from..tok.to];
        // Pattern 1: `<id> : [* | ?]* <TypeName>`
        if sig.get(idx + 1).is_some_and(|t| t.kind == TokenKind::Colon) {
            let mut t_idx = idx + 2;
            while sig
                .get(t_idx)
                .is_some_and(|t| matches!(t.kind, TokenKind::Star | TokenKind::Question))
            {
                t_idx += 1;
            }
            // Skip module prefixes `a.b.TypeName`
            while sig.get(t_idx).is_some_and(|t| t.kind == TokenKind::Id)
                && sig.get(t_idx + 1).is_some_and(|t| t.kind == TokenKind::Dot)
            {
                t_idx += 2;
            }
            if let Some(ty_tok) = sig.get(t_idx)
                && matches!(ty_tok.kind, TokenKind::Id | TokenKind::SelfCap)
            {
                map.insert(var_name.to_owned(), src[ty_tok.from..ty_tok.to].to_owned());
                continue;
            }
        }
        // Pattern 2: `(let|var) <id> = [a.b.]<CapitalizedType> ( '{' | '(' | '<' )`
        if idx > 0
            && matches!(sig[idx - 1].kind, TokenKind::Let | TokenKind::Var)
            && sig.get(idx + 1).is_some_and(|t| t.kind == TokenKind::Eq)
        {
            let mut t_idx = idx + 2;
            while sig.get(t_idx).is_some_and(|t| t.kind == TokenKind::Id)
                && sig.get(t_idx + 1).is_some_and(|t| t.kind == TokenKind::Dot)
            {
                t_idx += 2;
            }
            if let Some(ty_tok) = sig.get(t_idx)
                && ty_tok.kind == TokenKind::Id
            {
                let candidate = &src[ty_tok.from..ty_tok.to];
                if candidate.chars().next().is_some_and(char::is_uppercase) {
                    map.insert(var_name.to_owned(), candidate.to_owned());
                }
            }
        }
    }
    map
}

/// Build LSP `semanticTokens/full` delta-encoded array (`[deltaLine, deltaStart, len, tokenType, modMask]`)
/// by combining exact lexical tokens with HIR `SymbolIndex` classifications!
pub fn build_semantic_tokens(
    src: &str,
    line_index: &crate::position::LineIndex,
    file_path: &Path,
    sym_index: Option<&SymbolIndex>,
) -> Vec<u32> {
    let (tokens, _, _) = lex::lex(src, BytePos(0));
    let mut raw_entries: BTreeMap<(u32, u32), (u32, u32, u32)> = BTreeMap::new();

    for (idx, tok) in tokens.iter().enumerate() {
        if matches!(
            tok.kind,
            TokenKind::Sof | TokenKind::Eof | TokenKind::Invalid
        ) || tok.to <= tok.from
            || tok.to > src.len()
        {
            continue;
        }
        let slice = &src[tok.from..tok.to];
        let prev_kind = idx
            .checked_sub(1)
            .and_then(|p| tokens.get(p))
            .map(|t| t.kind);
        let Some((tok_ty, mut tok_mod)) =
            classify_token(tok, slice, prev_kind, file_path, sym_index)
        else {
            continue;
        };
        if tok.kind == TokenKind::Comment && slice.starts_with("---") {
            tok_mod |= semantic_token_modifier::DOCUMENTATION;
        }

        // Split multi-line tokens (such as `{- ... -}` block comments or multi-line strings)
        // per line as required by standard single-line LSP semantic token clients.
        let mut seg_start = tok.from;
        for part in slice.split('\n') {
            let clean_len = part.strip_suffix('\r').unwrap_or(part).len();
            if clean_len > 0 {
                let start_pos = line_index.offset_to_position(src, seg_start);
                let end_pos = line_index.offset_to_position(src, seg_start + clean_len);
                let utf16_len = end_pos.character.saturating_sub(start_pos.character);
                if utf16_len > 0 {
                    raw_entries.insert(
                        (start_pos.line, start_pos.character),
                        (utf16_len, tok_ty, tok_mod),
                    );
                }
            }
            seg_start += part.len() + 1;
        }
    }

    let mut data = Vec::with_capacity(raw_entries.len() * 5);
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for ((line, start), (len, ty, mods)) in raw_entries {
        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 {
            start - prev_start
        } else {
            start
        };
        data.extend_from_slice(&[delta_line, delta_start, len, ty, mods]);
        prev_line = line;
        prev_start = start;
    }
    data
}

fn classify_token(
    tok: &Token,
    slice: &str,
    prev_kind: Option<TokenKind>,
    file_path: &Path,
    sym_index: Option<&SymbolIndex>,
) -> Option<(u32, u32)> {
    match tok.kind {
        TokenKind::Comment => Some((semantic_token_type::COMMENT, 0)),
        TokenKind::Str | TokenKind::Char | TokenKind::MacroContent => {
            Some((semantic_token_type::STRING, 0))
        }
        TokenKind::Int
        | TokenKind::IntBin
        | TokenKind::IntOct
        | TokenKind::IntHex
        | TokenKind::Real
        | TokenKind::RealSci
        | TokenKind::BitBinPrefix
        | TokenKind::BitOctPrefix
        | TokenKind::BitHexPrefix => Some((semantic_token_type::NUMBER, 0)),
        TokenKind::Pub
        | TokenKind::Private
        | TokenKind::Pure
        | TokenKind::Comptime
        | TokenKind::Inline
        | TokenKind::KwQuote
        | TokenKind::Unsafe
        | TokenKind::Async
        | TokenKind::Ghost
        | TokenKind::Spec
        | TokenKind::Verified
        | TokenKind::Static
        | TokenKind::Implicit
        | TokenKind::Opaque => Some((semantic_token_type::MODIFIER, 0)),
        TokenKind::SelfCap | TokenKind::FnCap => Some((semantic_token_type::TYPE, 0)),
        TokenKind::SelfLower => Some((
            semantic_token_type::PARAMETER,
            semantic_token_modifier::READONLY,
        )),
        k if is_keyword_token(k) => Some((semantic_token_type::KEYWORD, 0)),
        TokenKind::Plus
        | TokenKind::PlusEq
        | TokenKind::PlusPlus
        | TokenKind::SeparatedPlus
        | TokenKind::LtEq
        | TokenKind::SeparatedLt
        | TokenKind::GtEq
        | TokenKind::SeparatedGt
        | TokenKind::Bang
        | TokenKind::BangEq
        | TokenKind::Minus
        | TokenKind::Arrow
        | TokenKind::MinusEq
        | TokenKind::SeparatedMinus
        | TokenKind::ColonTilde
        | TokenKind::ColonMinus
        | TokenKind::Star
        | TokenKind::StarEq
        | TokenKind::SeparatedStar
        | TokenKind::Slash
        | TokenKind::SlashEq
        | TokenKind::SeparatedSlash
        | TokenKind::Percent
        | TokenKind::PercentEq
        | TokenKind::SeparatedPercent
        | TokenKind::Eq
        | TokenKind::FatArrow
        | TokenKind::EqEq
        | TokenKind::EqEqEq
        | TokenKind::Tilde
        | TokenKind::TildeGt
        | TokenKind::PipeGt
        | TokenKind::Hash
        | TokenKind::Question => Some((semantic_token_type::OPERATOR, 0)),
        TokenKind::Id => {
            if matches!(prev_kind, Some(TokenKind::At | TokenKind::Dollar)) {
                return Some((semantic_token_type::MACRO, 0));
            }
            if let Some(idx) = sym_index
                && let Some(occ) = idx.occurrence_at(file_path, tok.from)
                && occ.span == (tok.from, tok.to)
                && let Some(sym) = idx.symbol(occ.target)
            {
                let mut mods = 0u32;
                if occ.is_decl {
                    mods |=
                        semantic_token_modifier::DECLARATION | semantic_token_modifier::DEFINITION;
                }
                if occ.is_write {
                    mods |= semantic_token_modifier::MODIFICATION;
                } else if !sym.is_mutable {
                    mods |= semantic_token_modifier::READONLY;
                }
                let ty = match sym.kind {
                    lsp_symbol_kind::MODULE
                    | lsp_symbol_kind::PACKAGE
                    | lsp_symbol_kind::NAMESPACE => semantic_token_type::NAMESPACE,
                    lsp_symbol_kind::STRUCT | lsp_symbol_kind::CLASS => semantic_token_type::STRUCT,
                    lsp_symbol_kind::ENUM => semantic_token_type::ENUM,
                    lsp_symbol_kind::ENUM_MEMBER => semantic_token_type::ENUM_MEMBER,
                    lsp_symbol_kind::INTERFACE => semantic_token_type::INTERFACE,
                    lsp_symbol_kind::TYPE_PARAMETER => semantic_token_type::TYPE_PARAMETER,
                    lsp_symbol_kind::FUNCTION | lsp_symbol_kind::EVENT => {
                        semantic_token_type::FUNCTION
                    }
                    lsp_symbol_kind::METHOD => semantic_token_type::METHOD,
                    lsp_symbol_kind::FIELD | lsp_symbol_kind::PROPERTY => {
                        semantic_token_type::PROPERTY
                    }
                    _ => semantic_token_type::VARIABLE,
                };
                return Some((ty, mods));
            }
            // Lexical heuristic fallback when unresolved: PascalCase -> type, `Type` -> type.
            if slice.chars().next().is_some_and(char::is_uppercase) {
                Some((semantic_token_type::TYPE, 0))
            } else {
                Some((semantic_token_type::VARIABLE, 0))
            }
        }
        _ => None,
    }
}

pub fn is_keyword_token(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::And
            | TokenKind::As
            | TokenKind::Assoc
            | TokenKind::Asserts
            | TokenKind::Assumes
            | TokenKind::Async
            | TokenKind::Atomic
            | TokenKind::Axiom
            | TokenKind::Await
            | TokenKind::Break
            | TokenKind::Case
            | TokenKind::Catch
            | TokenKind::Comptime
            | TokenKind::Defer
            | TokenKind::Const
            | TokenKind::Continue
            | TokenKind::Decreases
            | TokenKind::Default
            | TokenKind::Define
            | TokenKind::Derive
            | TokenKind::Do
            | TokenKind::Dyn
            | TokenKind::Effect
            | TokenKind::Else
            | TokenKind::Ensures
            | TokenKind::Enum
            | TokenKind::Error
            | TokenKind::Exists
            | TokenKind::Extend
            | TokenKind::False
            | TokenKind::Fn
            | TokenKind::FnCap
            | TokenKind::For
            | TokenKind::Forall
            | TokenKind::Ghost
            | TokenKind::Handles
            | TokenKind::If
            | TokenKind::Implicit
            | TokenKind::Impl
            | TokenKind::In
            | TokenKind::Inline
            | TokenKind::Invariant
            | TokenKind::Is
            | TokenKind::Itself
            | TokenKind::Lambda
            | TokenKind::Lemma
            | TokenKind::Var
            | TokenKind::Let
            | TokenKind::Lift
            | TokenKind::Match
            | TokenKind::Matches
            | TokenKind::Move
            | TokenKind::Mod
            | TokenKind::Newtype
            | TokenKind::Not
            | TokenKind::Null
            | TokenKind::Opaque
            | TokenKind::Opens
            | TokenKind::Or
            | TokenKind::Outcomes
            | TokenKind::Predicate
            | TokenKind::Private
            | TokenKind::Pub
            | TokenKind::Pure
            | TokenKind::KwQuote
            | TokenKind::Ref
            | TokenKind::Refines
            | TokenKind::Requires
            | TokenKind::Resume
            | TokenKind::Return
            | TokenKind::SelfLower
            | TokenKind::SelfCap
            | TokenKind::Spec
            | TokenKind::Static
            | TokenKind::Struct
            | TokenKind::Template
            | TokenKind::Test
            | TokenKind::Trait
            | TokenKind::True
            | TokenKind::Typealias
            | TokenKind::Union
            | TokenKind::Unsafe
            | TokenKind::Undefined
            | TokenKind::Use
            | TokenKind::Verified
            | TokenKind::When
            | TokenKind::While
            | TokenKind::Where
    )
}
