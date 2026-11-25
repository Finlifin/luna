//! Host compiler bridge for executing Flurry comptime construction macros (`^macro1`–`^macro4`).
use crate::invocation::MacroPackageSession;
use crate::packages::{CompilationInputs, SourcePackage};
use comptime_interpretor::{ListValues, Value};
use comptime_runtime::integer::Integer;
use diagnostic::{DiagnosticContext, FlurryError};
use lex::TokenKind;
use macro_expander::peg::parse_and_validate_peg_grammar_str;
use macro_expander::provenance::{MacroToken, TokenSpacing, is_separated_kind};
use macro_expander::{
    AstArgValue, AstNodeKind, AstParamShape, ExpansionContext, MacroBudget, MacroDescriptor,
    MacroError, MacroErrorKind, MacroExecutor, MacroId, MacroInputMode, MacroRegistry,
    MacroVisibility, ParsedPegNode, ParsedPegTree, ParsedSyntaxNode, ParsedSyntaxTree, PegArgValue,
    PegParamShape, scan_referenced_macro_roots,
};
use nir::objects::entry_parts;
use nir::program::{FunctionPhase, Package};
use nir::{DeclarationId, Primitive, Term};
use parse::parser::Parser;
use resolve::{BindingKind, DefId, binding::Visibility, scope::ScopeKind};
use rustc_span::{BytePos, SourceMap, Span};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DEFAULT_MACRO_CALL_FUEL: u64 = 20_000_000;

/// Locate a package's companion `scripts/` directory if the entry file is `main.fl` or `lib.fl`.
pub(crate) fn find_package_scripts_dir(package_entry: &Path) -> Option<PathBuf> {
    let file_name = package_entry.file_name().and_then(|n| n.to_str())?;
    if !matches!(file_name, "main.fl" | "lib.fl") {
        return None;
    }
    let dir = package_entry
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let direct = dir.join("scripts");
    if direct.is_dir() {
        return Some(direct);
    }
    if dir.file_name().and_then(|n| n.to_str()) == Some("src") {
        if let Some(parent) = dir.parent() {
            let sibling = parent.join("scripts");
            if sibling.is_dir() {
                return Some(sibling);
            }
        }
    }
    None
}

/// Canonical path to a package's `scripts/` directory, suitable for `discover_sources_excluding`.
pub(crate) fn scripts_exclusion(package_entry: &Path) -> Option<PathBuf> {
    find_package_scripts_dir(package_entry).and_then(|dir| std::fs::canonicalize(dir).ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ProviderKey {
    Package(u32),
    Scripts(u32),
}

#[derive(Clone, Debug)]
struct StructLayout {
    declaration: DeclarationId,
    field_names: Vec<String>,
    field_types: Vec<Term>,
}

impl StructLayout {
    fn from_nir(
        nir: &Package,
        declaration: DeclarationId,
        expected_fields: &[&str],
    ) -> Result<Self, String> {
        let s = nir
            .structs
            .iter()
            .find(|s| s.declaration == declaration)
            .ok_or_else(|| format!("missing struct definition for {declaration}"))?;
        if s.fields.len() != expected_fields.len() {
            return Err(format!(
                "struct '{}' has {} fields, expected {}",
                s.name,
                s.fields.len(),
                expected_fields.len()
            ));
        }
        let mut field_names = Vec::with_capacity(s.fields.len());
        let mut field_types = Vec::with_capacity(s.fields.len());
        for (idx, (name, ty)) in s.fields.iter().enumerate() {
            if name != expected_fields[idx] {
                return Err(format!(
                    "struct '{}' field {idx} is `{name}`, expected `{}`",
                    s.name, expected_fields[idx]
                ));
            }
            field_names.push(name.clone());
            field_types.push(ty.clone());
        }
        Ok(Self {
            declaration,
            field_names,
            field_types,
        })
    }

    fn field_index(&self, name: &str) -> Option<usize> {
        self.field_names.iter().position(|n| n == name)
    }
}

#[derive(Clone, Debug)]
struct MacroAstTypes {
    source_span: StructLayout,
    token: StructLayout,
    token_buffer: StructLayout,
    token_list_ty: Term,
    syntax_node: StructLayout,
    syntax_tree: StructLayout,
    peg_tree_node: StructLayout,
    peg_node: StructLayout,
    int_list_ty: Term,
    syntax_node_list_ty: Term,
    peg_tree_node_list_ty: Term,
    meta_list_head: DeclarationId,
    wrapper_by_kind: HashMap<AstNodeKind, StructLayout>,
    kind_by_decl: HashMap<DeclarationId, AstNodeKind>,
}

impl MacroAstTypes {
    fn discover(
        nir: &Package,
        tree: &resolve::ModuleTree,
        package_scopes: &HashMap<u32, resolve::ScopeId>,
        meta_resolution_id: u32,
        identities: &BTreeMap<u32, u64>,
    ) -> Result<Self, String> {
        let module_scopes = collect_module_scopes(tree);
        let meta_scope = *package_scopes
            .get(&meta_resolution_id)
            .ok_or("missing package scope for `meta`")?;
        let ast_binding = resolve_exported_binding(&tree.scope_tree, meta_scope, "ast")
            .ok_or("missing `meta.ast` module while preparing construction macros")?;
        let BindingKind::Definition(ast_def, _) = ast_binding.kind else {
            return Err("`meta.ast` is not a module definition".into());
        };
        let ast_scope = *module_scopes
            .get(&ast_def)
            .ok_or("`meta.ast` has no module scope")?;

        let lookup_decl = |name: &str| -> Result<DeclarationId, String> {
            let binding = resolve_exported_binding(&tree.scope_tree, ast_scope, name)
                .ok_or_else(|| format!("missing `meta.ast.{name}`"))?;
            let BindingKind::Definition(def_id, _) = binding.kind else {
                return Err(format!("`meta.ast.{name}` is not a struct definition"));
            };
            def_to_declaration_id(def_id, identities)
        };

        let span_decl = lookup_decl("SourceSpan")?;
        let token_decl = lookup_decl("Token")?;
        let tb_decl = lookup_decl("TokenBuffer")?;
        let syntax_node_decl = lookup_decl("SyntaxNode")?;
        let syntax_tree_decl = lookup_decl("SyntaxTree")?;
        let peg_tree_node_decl = lookup_decl("PegTreeNode")?;
        let peg_node_decl = lookup_decl("PegNode")?;

        let source_span =
            StructLayout::from_nir(nir, span_decl, &["file", "start", "end", "expansion_id"])?;
        let token = StructLayout::from_nir(nir, token_decl, &["kind", "text", "spacing", "span"])?;
        let token_buffer = StructLayout::from_nir(nir, tb_decl, &["tokens"])?;
        let tokens_idx = token_buffer
            .field_index("tokens")
            .ok_or("missing `tokens` field on `TokenBuffer`")?;
        let token_list_ty = token_buffer.field_types[tokens_idx].clone();
        let Term::NormalForm {
            head: meta_list_head,
            ..
        } = &token_list_ty
        else {
            return Err(format!(
                "expected `TokenBuffer.tokens` to use `meta.List`, found {token_list_ty}"
            ));
        };
        let meta_list_head = *meta_list_head;

        let syntax_node =
            StructLayout::from_nir(nir, syntax_node_decl, &["kind", "span", "children", "text"])?;
        let int_list_ty = syntax_node.field_types[2].clone();
        let syntax_tree =
            StructLayout::from_nir(nir, syntax_tree_decl, &["nodes", "root", "tokens"])?;
        let syntax_node_list_ty = syntax_tree.field_types[0].clone();
        let peg_tree_node = StructLayout::from_nir(
            nir,
            peg_tree_node_decl,
            &["rule", "span", "text", "tokens", "child_indices"],
        )?;
        let peg_node = StructLayout::from_nir(nir, peg_node_decl, &["tree", "root"])?;
        let peg_tree_node_list_ty = peg_node.field_types[0].clone();

        const ALL_AST_KINDS: [AstNodeKind; 10] = [
            AstNodeKind::FlurryPattern,
            AstNodeKind::FlurryExpr,
            AstNodeKind::FlurryStatement,
            AstNodeKind::FlurryExtendedStatement,
            AstNodeKind::FlurryItem,
            AstNodeKind::FlurryType,
            AstNodeKind::FlurryIdent,
            AstNodeKind::FlurryLiteral,
            AstNodeKind::FlurryBlock,
            AstNodeKind::FlurryPath,
        ];
        let mut wrapper_by_kind = HashMap::with_capacity(ALL_AST_KINDS.len());
        let mut kind_by_decl = HashMap::with_capacity(ALL_AST_KINDS.len());
        for kind in ALL_AST_KINDS {
            let decl = lookup_decl(kind.as_str())?;
            let layout = StructLayout::from_nir(nir, decl, &["tree", "node"])?;
            wrapper_by_kind.insert(kind, layout);
            kind_by_decl.insert(decl, kind);
        }

        Ok(Self {
            source_span,
            token,
            token_buffer,
            token_list_ty,
            syntax_node,
            syntax_tree,
            peg_tree_node,
            peg_node,
            int_list_ty,
            syntax_node_list_ty,
            peg_tree_node_list_ty,
            meta_list_head,
            wrapper_by_kind,
            kind_by_decl,
        })
    }
}

#[derive(Clone, Debug)]
struct ExtractedMacro {
    relative_path: Vec<String>,
    declaration: DeclarationId,
    vis: Visibility,
    def_site: Option<Span>,
    mode: MacroInputMode,
    param_tys: Vec<Term>,
}

#[derive(Clone, Debug)]
struct CompiledMacroEntry {
    provider: ProviderKey,
    declaration: DeclarationId,
    param_tys: Vec<Term>,
}

struct PreparedMacroProvider {
    macro_package_id: u32,
    is_scripts: bool,
    owner_package: u32,
    session: MacroPackageSession,
    ast_types: MacroAstTypes,
    macros: Vec<ExtractedMacro>,
}

#[derive(Clone, Debug, Default)]
struct OwnedSelectors {
    dispatch_fuel: Option<usize>,
    host_fma: Option<String>,
    host_sqrt: Option<String>,
    host_copysign: Option<String>,
    comptime_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
    host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
    copy: Option<String>,
    drop: Option<String>,
    index: Option<String>,
    index_mut: Option<String>,
    space_apply: Option<String>,
    iterator: Option<String>,
    into_iterator: Option<String>,
    variadic_collector: Option<String>,
    extended_variadic_collector: Option<String>,
    ordering: Option<String>,
    equality: Option<String>,
    comptime_ordering: Option<String>,
    comptime_equality: Option<String>,
    list_pattern: Option<String>,
    comptime_list_pattern: Option<String>,
    stable_list_pattern: Option<String>,
    list_pattern_endless: Option<String>,
    stable_list_pattern_endless: Option<String>,
    range_end: Option<String>,
    range_pattern: Option<String>,
    stable_range_pattern: Option<String>,
    concat_pattern: Option<String>,
    stable_concat_pattern: Option<String>,
    map_pattern: Option<String>,
    stable_map_pattern: Option<String>,
    struct_pattern: Option<String>,
    stable_struct_pattern: Option<String>,
    type_pattern: Option<String>,
    literal_into: Option<String>,
    implies: Option<String>,
    comptime_implies: Option<String>,
    handle_error: Option<String>,
    deref: Option<String>,
    deref_mut: Option<String>,
    reference: Option<String>,
    implicit_passing: Option<String>,
    call_context: Option<String>,
    call_description: Option<String>,
    arithmetic: [Option<String>; 7],
    compound_arithmetic: [Option<String>; 5],
    comptime_compound_arithmetic: [Option<String>; 5],
    splice: Option<String>,
    meta_list: Option<String>,
    tuple: Option<String>,
}

impl OwnedSelectors {
    fn from_borrowed(s: crate::protocols::type_constructors::Selectors<'_>) -> Self {
        Self {
            dispatch_fuel: s.dispatch_fuel,
            host_fma: s.host_fma.map(str::to_owned),
            host_sqrt: s.host_sqrt.map(str::to_owned),
            host_copysign: s.host_copysign.map(str::to_owned),
            comptime_intrinsics: s.comptime_intrinsics.to_vec(),
            host_intrinsics: s.host_intrinsics.to_vec(),
            copy: s.copy.map(str::to_owned),
            drop: s.drop.map(str::to_owned),
            index: s.index.map(str::to_owned),
            index_mut: s.index_mut.map(str::to_owned),
            space_apply: s.space_apply.map(str::to_owned),
            iterator: s.iterator.map(str::to_owned),
            into_iterator: s.into_iterator.map(str::to_owned),
            variadic_collector: s.variadic_collector.map(str::to_owned),
            extended_variadic_collector: s.extended_variadic_collector.map(str::to_owned),
            ordering: s.ordering.map(str::to_owned),
            equality: s.equality.map(str::to_owned),
            comptime_ordering: s.comptime_ordering.map(str::to_owned),
            comptime_equality: s.comptime_equality.map(str::to_owned),
            list_pattern: s.list_pattern.map(str::to_owned),
            comptime_list_pattern: s.comptime_list_pattern.map(str::to_owned),
            stable_list_pattern: s.stable_list_pattern.map(str::to_owned),
            list_pattern_endless: s.list_pattern_endless.map(str::to_owned),
            stable_list_pattern_endless: s.stable_list_pattern_endless.map(str::to_owned),
            range_end: s.range_end.map(str::to_owned),
            range_pattern: s.range_pattern.map(str::to_owned),
            stable_range_pattern: s.stable_range_pattern.map(str::to_owned),
            concat_pattern: s.concat_pattern.map(str::to_owned),
            stable_concat_pattern: s.stable_concat_pattern.map(str::to_owned),
            map_pattern: s.map_pattern.map(str::to_owned),
            stable_map_pattern: s.stable_map_pattern.map(str::to_owned),
            struct_pattern: s.struct_pattern.map(str::to_owned),
            stable_struct_pattern: s.stable_struct_pattern.map(str::to_owned),
            type_pattern: s.type_pattern.map(str::to_owned),
            literal_into: s.literal_into.map(str::to_owned),
            implies: s.implies.map(str::to_owned),
            comptime_implies: s.comptime_implies.map(str::to_owned),
            handle_error: s.handle_error.map(str::to_owned),
            deref: s.deref.map(str::to_owned),
            deref_mut: s.deref_mut.map(str::to_owned),
            reference: s.reference.map(str::to_owned),
            implicit_passing: s.implicit_passing.map(str::to_owned),
            call_context: s.call_context.map(str::to_owned),
            call_description: s.call_description.map(str::to_owned),
            arithmetic: std::array::from_fn(|i| s.arithmetic[i].map(str::to_owned)),
            compound_arithmetic: std::array::from_fn(|i| {
                s.compound_arithmetic[i].map(str::to_owned)
            }),
            comptime_compound_arithmetic: std::array::from_fn(|i| {
                s.comptime_compound_arithmetic[i].map(str::to_owned)
            }),
            splice: s.splice.map(str::to_owned),
            meta_list: s.meta_list.map(str::to_owned),
            tuple: s.tuple.map(str::to_owned),
        }
    }

    fn as_selectors(&self) -> crate::protocols::type_constructors::Selectors<'_> {
        crate::protocols::type_constructors::Selectors {
            dispatch_fuel: self.dispatch_fuel,
            host_fma: self.host_fma.as_deref(),
            host_sqrt: self.host_sqrt.as_deref(),
            host_copysign: self.host_copysign.as_deref(),
            comptime_intrinsics: &self.comptime_intrinsics,
            host_intrinsics: &self.host_intrinsics,
            copy: self.copy.as_deref(),
            drop: self.drop.as_deref(),
            index: self.index.as_deref(),
            index_mut: self.index_mut.as_deref(),
            space_apply: self.space_apply.as_deref(),
            iterator: self.iterator.as_deref(),
            into_iterator: self.into_iterator.as_deref(),
            variadic_collector: self.variadic_collector.as_deref(),
            extended_variadic_collector: self.extended_variadic_collector.as_deref(),
            ordering: self.ordering.as_deref(),
            equality: self.equality.as_deref(),
            comptime_ordering: self.comptime_ordering.as_deref(),
            comptime_equality: self.comptime_equality.as_deref(),
            list_pattern: self.list_pattern.as_deref(),
            comptime_list_pattern: self.comptime_list_pattern.as_deref(),
            stable_list_pattern: self.stable_list_pattern.as_deref(),
            list_pattern_endless: self.list_pattern_endless.as_deref(),
            stable_list_pattern_endless: self.stable_list_pattern_endless.as_deref(),
            range_end: self.range_end.as_deref(),
            range_pattern: self.range_pattern.as_deref(),
            stable_range_pattern: self.stable_range_pattern.as_deref(),
            concat_pattern: self.concat_pattern.as_deref(),
            stable_concat_pattern: self.stable_concat_pattern.as_deref(),
            map_pattern: self.map_pattern.as_deref(),
            stable_map_pattern: self.stable_map_pattern.as_deref(),
            struct_pattern: self.struct_pattern.as_deref(),
            stable_struct_pattern: self.stable_struct_pattern.as_deref(),
            type_pattern: self.type_pattern.as_deref(),
            literal_into: self.literal_into.as_deref(),
            implies: self.implies.as_deref(),
            comptime_implies: self.comptime_implies.as_deref(),
            handle_error: self.handle_error.as_deref(),
            deref: self.deref.as_deref(),
            deref_mut: self.deref_mut.as_deref(),
            reference: self.reference.as_deref(),
            implicit_passing: self.implicit_passing.as_deref(),
            call_context: self.call_context.as_deref(),
            call_description: self.call_description.as_deref(),
            arithmetic: std::array::from_fn(|i| self.arithmetic[i].as_deref()),
            compound_arithmetic: std::array::from_fn(|i| self.compound_arithmetic[i].as_deref()),
            comptime_compound_arithmetic: std::array::from_fn(|i| {
                self.comptime_compound_arithmetic[i].as_deref()
            }),
            splice: self.splice.as_deref(),
            meta_list: self.meta_list.as_deref(),
            tuple: self.tuple.as_deref(),
        }
    }
}

/// Stateful host coordinating macro package/script compilation and `^macro3`/`^macro4` execution.
pub(crate) struct MacroHost {
    packages: Vec<SourcePackage>,
    dependencies: Vec<resolve::DependencyEdge>,
    builtin: Option<u32>,
    protocols: elaboration::program::Protocols,
    selectors: OwnedSelectors,
    target: Option<nir::target::TargetData>,
    preludes: Vec<u32>,
    providers: HashMap<ProviderKey, PreparedMacroProvider>,
    active_stack: Vec<ProviderKey>,
    compiled_macros: HashMap<MacroId, CompiledMacroEntry>,
    expanded_packages: HashSet<u32>,
    incremental: crate::package_cache::IncrementalConfig,
    next_res_id: u32,
    next_identity: u64,
    macro_fuel_per_call: u64,
    current_source_ranges: Vec<(u32, u32)>,
}

impl MacroHost {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        packages: &[SourcePackage],
        dependencies: &[resolve::DependencyEdge],
        builtin: Option<u32>,
        protocols: &elaboration::program::Protocols,
        selectors: crate::protocols::type_constructors::Selectors<'_>,
        target: Option<nir::target::TargetData>,
        preludes: &[u32],
    ) -> Self {
        let next_res_id = packages
            .iter()
            .map(|p| p.resolution_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let next_identity = packages
            .iter()
            .map(|p| p.identity)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Self {
            packages: packages.to_vec(),
            dependencies: dependencies.to_vec(),
            builtin,
            protocols: protocols.clone(),
            selectors: OwnedSelectors::from_borrowed(selectors),
            target: target.or_else(|| nir::target::TargetData::new(usize::BITS).ok()),
            preludes: preludes.to_vec(),
            providers: HashMap::new(),
            active_stack: Vec::new(),
            compiled_macros: HashMap::new(),
            expanded_packages: HashSet::new(),
            incremental: crate::package_cache::IncrementalConfig::default(),
            next_res_id,
            next_identity,
            macro_fuel_per_call: DEFAULT_MACRO_CALL_FUEL,
            current_source_ranges: Vec::new(),
        }
    }

    pub(crate) fn with_incremental(
        mut self,
        incremental: crate::package_cache::IncrementalConfig,
    ) -> Self {
        self.incremental = incremental;
        self
    }

    pub(crate) fn expanded_package(&self, res_id: u32) -> bool {
        self.expanded_packages.contains(&res_id)
    }

    /// Inspect all VFS packages; if any references non-builtin macros, prepare required providers,
    /// expand macros across all files of that package, parse them, and populate `vfs.set_ast`.
    pub(crate) fn expand_and_parse_packages(
        &mut self,
        source_map: &SourceMap,
        diag_ctx: &DiagnosticContext<'_>,
        active_packages: &[SourcePackage],
        active_dependencies: &[resolve::DependencyEdge],
        inputs: &mut [(u32, vfs::Vfs)],
        observed: Option<&mut CompilationInputs>,
    ) -> Result<(), String> {
        let mut new_ranges = Vec::new();
        for (_, vfs) in inputs.iter() {
            for (_, entry) in vfs.files() {
                let lo = entry.source_file.start_pos.0;
                let byte_len = entry
                    .source_file
                    .src
                    .as_ref()
                    .map_or(0, |s| u32::try_from(s.len()).unwrap_or(u32::MAX));
                let hi = lo.saturating_add(byte_len);
                new_ranges.push((lo, hi));
            }
        }
        let prev_ranges = std::mem::replace(&mut self.current_source_ranges, new_ranges);
        let result = self.expand_and_parse_packages_inner(
            source_map,
            diag_ctx,
            active_packages,
            active_dependencies,
            inputs,
            observed,
        );
        self.current_source_ranges = prev_ranges;
        result
    }

    fn expand_and_parse_packages_inner(
        &mut self,
        source_map: &SourceMap,
        diag_ctx: &DiagnosticContext<'_>,
        active_packages: &[SourcePackage],
        active_dependencies: &[resolve::DependencyEdge],
        inputs: &mut [(u32, vfs::Vfs)],
        mut observed: Option<&mut CompilationInputs>,
    ) -> Result<(), String> {
        for idx in 0..inputs.len() {
            let pkg_res_id = inputs[idx].0;
            let Some(pkg_meta) = active_packages
                .iter()
                .find(|p| p.resolution_id == pkg_res_id)
                .cloned()
            else {
                continue;
            };
            let pkg_name = pkg_meta.name;
            let pkg_entry = pkg_meta.entry;
            let mut combined_refs = macro_expander::ReferencedMacros::default();
            let mut first_ref_start = None;

            for (_, entry) in inputs[idx].1.files() {
                let Some(src) = entry.source_file.src.as_deref() else {
                    continue;
                };
                let refs = scan_referenced_macro_roots(src);
                if refs.has_non_builtin_macros {
                    if first_ref_start.is_none() {
                        first_ref_start = Some(entry.source_file.start_pos);
                    }
                    combined_refs.has_non_builtin_macros = true;
                    combined_refs.roots.extend(refs.roots);
                    combined_refs.import_roots.extend(refs.import_roots);
                }
            }

            if !combined_refs.has_non_builtin_macros {
                continue;
            }
            self.expanded_packages.insert(pkg_res_id);

            let err_start_pos = first_ref_start.unwrap_or(BytePos(0));

            // Detect direct preparation self-cycles for currently preparing `Scripts(owner)`.
            let in_scripts = matches!(self.active_stack.last(), Some(ProviderKey::Scripts(_)));
            if in_scripts && combined_refs.roots.contains("scripts") {
                let err = MacroError::new(
                    MacroErrorKind::PreparationCycle,
                    "macro preparation cycle: `scripts` cannot invoke macros from `scripts`",
                    Span::new(err_start_pos, err_start_pos),
                );
                err.emit(diag_ctx, err_start_pos);
                return Err(diag_ctx.failure_summary("macro expansion"));
            }

            let mut provider_aliases: Vec<(String, ProviderKey)> = Vec::new();
            let scripts_dir = find_package_scripts_dir(&pkg_entry);

            for root in &combined_refs.roots {
                let dep_edge = active_dependencies
                    .iter()
                    .find(|e| e.requester == pkg_res_id && e.alias == *root)
                    .map(|e| (e.alias.clone(), e.provider));
                if let Some((alias, provider_id)) = dep_edge {
                    let key = ProviderKey::Package(provider_id);
                    self.ensure_provider_prepared(
                        key,
                        err_start_pos,
                        diag_ctx,
                        observed.as_deref_mut(),
                    )?;
                    if !provider_aliases.iter().any(|(a, _)| a == &alias) {
                        provider_aliases.push((alias, key));
                    }
                } else if root == "scripts" && scripts_dir.is_some() {
                    let key = ProviderKey::Scripts(pkg_res_id);
                    self.ensure_provider_prepared(
                        key,
                        err_start_pos,
                        diag_ctx,
                        observed.as_deref_mut(),
                    )?;
                    if !provider_aliases.iter().any(|(a, _)| a == "scripts") {
                        provider_aliases.push(("scripts".into(), key));
                    }
                }
            }

            // Include any already-prepared direct dependency macro providers so generated
            // tokens can invoke them even when unreferenced before expansion.
            for edge in active_dependencies {
                if edge.requester == pkg_res_id
                    && !provider_aliases.iter().any(|(a, _)| a == &edge.alias)
                {
                    let key = ProviderKey::Package(edge.provider);
                    if self.providers.contains_key(&key) {
                        provider_aliases.push((edge.alias.clone(), key));
                    }
                }
            }

            // Register prepared providers and let `macro_expander` diagnose unresolved/ambiguous roots.
            let mut registry = self.build_registry_for_consumer(&provider_aliases);

            let file_count = inputs[idx].1.file_count();
            for raw_id in 0..file_count {
                let file_id = vfs::FileId::from_raw(raw_id as u32);
                if inputs[idx].1.get_ast(file_id).is_some() {
                    continue;
                }
                let entry = inputs[idx].1.file(file_id);
                let rel_path = entry.rel_path.clone();
                let source_file = entry.source_file.clone();
                let current_module_path = module_path_for_file(&pkg_name, &rel_path);

                let (expanded, lex_errors) = loop {
                    let expand_result = {
                        let mut ctx = ExpansionContext {
                            current_package: pkg_res_id,
                            current_module_path: current_module_path.clone(),
                            registry: &registry,
                            executor: Some(self),
                        };
                        let mut budget = MacroBudget::default();
                        macro_expander::expand_source_file_with_context(
                            source_map,
                            &source_file,
                            &mut ctx,
                            &mut budget,
                        )
                    };
                    match expand_result {
                        Ok(res) => break res,
                        Err(err) => {
                            if err.kind == MacroErrorKind::UndefinedMacro
                                && let Some(missing_root) = err.unresolved_root.as_deref()
                                && !provider_aliases.iter().any(|(a, _)| a == missing_root)
                            {
                                let dep_edge = active_dependencies
                                    .iter()
                                    .find(|e| e.requester == pkg_res_id && e.alias == missing_root)
                                    .map(|e| (e.alias.clone(), e.provider));
                                if let Some((alias, provider_id)) = dep_edge {
                                    let key = ProviderKey::Package(provider_id);
                                    self.ensure_provider_prepared(
                                        key,
                                        err_start_pos,
                                        diag_ctx,
                                        observed.as_deref_mut(),
                                    )?;
                                    provider_aliases.push((alias, key));
                                    registry = self.build_registry_for_consumer(&provider_aliases);
                                    continue;
                                } else if missing_root == "scripts" && scripts_dir.is_some() {
                                    if in_scripts {
                                        let cyc = MacroError::new(
                                            MacroErrorKind::PreparationCycle,
                                            "macro preparation cycle: `scripts` cannot invoke macros from `scripts`",
                                            err.span,
                                        );
                                        cyc.emit(diag_ctx, source_file.start_pos);
                                        return Err(diag_ctx.failure_summary("macro expansion"));
                                    }
                                    let key = ProviderKey::Scripts(pkg_res_id);
                                    self.ensure_provider_prepared(
                                        key,
                                        err_start_pos,
                                        diag_ctx,
                                        observed.as_deref_mut(),
                                    )?;
                                    provider_aliases.push(("scripts".into(), key));
                                    registry = self.build_registry_for_consumer(&provider_aliases);
                                    continue;
                                }
                            }
                            err.emit(diag_ctx, source_file.start_pos);
                            return Err(diag_ctx.failure_summary("macro expansion"));
                        }
                    }
                };
                let lex_failed = !lex_errors.is_empty();
                for err in lex_errors {
                    err.emit(diag_ctx, source_file.start_pos);
                }
                let mut parser = Parser::new(
                    source_map,
                    expanded.tokens,
                    expanded.symbols,
                    expanded.start_pos,
                );
                let parsed = parser.parse(diag_ctx);
                if lex_failed || !parsed {
                    return Err(format!(
                        "{}: source file contains lexical or syntax errors",
                        diag_ctx.failure_summary("package resolution")
                    ));
                }
                let ast = parser.finalize();
                if let Err(error) = ast.validate_structure() {
                    return Err(error.to_string());
                }
                inputs[idx].1.set_ast(file_id, ast);
            }
        }
        Ok(())
    }

    fn ensure_provider_prepared(
        &mut self,
        key: ProviderKey,
        call_site_pos: BytePos,
        diag_ctx: &DiagnosticContext<'_>,
        observed: Option<&mut CompilationInputs>,
    ) -> Result<(), String> {
        if self.providers.contains_key(&key) {
            return Ok(());
        }
        if self.active_stack.contains(&key) {
            let desc = match key {
                ProviderKey::Package(id) => self
                    .packages
                    .iter()
                    .find(|p| p.resolution_id == id)
                    .map(|p| format!("package `{}`", p.name))
                    .unwrap_or_else(|| format!("package #{id}")),
                ProviderKey::Scripts(_) => "`scripts`".into(),
            };
            let err = MacroError::new(
                MacroErrorKind::PreparationCycle,
                format!("macro preparation cycle while preparing {desc}"),
                Span::new(call_site_pos, call_site_pos),
            );
            err.emit(diag_ctx, call_site_pos);
            return Err(diag_ctx.failure_summary("macro expansion"));
        }

        self.active_stack.push(key);
        let result = self.compile_provider(key, observed);
        self.active_stack.pop();
        result
    }

    fn compile_provider(
        &mut self,
        key: ProviderKey,
        observed: Option<&mut CompilationInputs>,
    ) -> Result<(), String> {
        let mut virtual_sources = BTreeMap::new();
        let mut sub_protocols = self.protocols.clone();
        let mut force_discovery = false;

        let (
            sub_packages,
            sub_dependencies,
            target_pkg_id,
            is_scripts,
            owner_pkg_id,
            macro_pkg_u32,
        ) = match key {
            ProviderKey::Package(pkg_id) => {
                let mut needed = BTreeSet::new();
                needed.insert(pkg_id);
                if let Some(b) = self.builtin {
                    needed.insert(b);
                }
                for &p in &self.preludes {
                    needed.insert(p);
                }
                let mut changed = true;
                while changed {
                    changed = false;
                    for edge in &self.dependencies {
                        if needed.contains(&edge.requester) && needed.insert(edge.provider) {
                            changed = true;
                        }
                    }
                }
                let sub_packages: Vec<SourcePackage> = self
                    .packages
                    .iter()
                    .filter(|p| needed.contains(&p.resolution_id))
                    .cloned()
                    .collect();
                let sub_dependencies: Vec<resolve::DependencyEdge> = self
                    .dependencies
                    .iter()
                    .filter(|e| needed.contains(&e.requester) && needed.contains(&e.provider))
                    .cloned()
                    .collect();
                (
                    sub_packages,
                    sub_dependencies,
                    pkg_id,
                    false,
                    pkg_id,
                    pkg_id,
                )
            }
            ProviderKey::Scripts(owner_res_id) => {
                let owner_pkg = self
                    .packages
                    .iter()
                    .find(|p| p.resolution_id == owner_res_id)
                    .ok_or_else(|| format!("missing owner package {owner_res_id} for scripts"))?;
                let scripts_dir = find_package_scripts_dir(&owner_pkg.entry)
                    .ok_or("missing `scripts/` directory")?;
                let scripts_res_id = self.next_res_id;
                self.next_res_id = self.next_res_id.saturating_add(1);
                let scripts_identity = self.next_identity;
                self.next_identity = self.next_identity.saturating_add(1);

                if let Some(mode) = self.protocols.build_modes.get(&owner_pkg.identity).cloned() {
                    sub_protocols.build_modes.insert(scripts_identity, mode);
                }
                if let Some(target_cfg) = self
                    .protocols
                    .build_targets
                    .get(&owner_pkg.identity)
                    .cloned()
                {
                    sub_protocols
                        .build_targets
                        .insert(scripts_identity, target_cfg);
                }

                let lib_fl = scripts_dir.join("lib.fl");
                let mod_fl = scripts_dir.join("mod.fl");
                let entry = if lib_fl.is_file() {
                    lib_fl
                } else if mod_fl.is_file() {
                    force_discovery = true;
                    mod_fl
                } else {
                    force_discovery = true;
                    virtual_sources.insert(lib_fl.clone(), Arc::from(""));
                    lib_fl
                };

                let mut needed = BTreeSet::new();
                for edge in &self.dependencies {
                    if edge.requester == owner_res_id {
                        needed.insert(edge.provider);
                    }
                }
                if let Some(b) = self.builtin {
                    needed.insert(b);
                }
                for &p in &self.preludes {
                    needed.insert(p);
                }
                let mut changed = true;
                while changed {
                    changed = false;
                    for edge in &self.dependencies {
                        if needed.contains(&edge.requester) && needed.insert(edge.provider) {
                            changed = true;
                        }
                    }
                }

                let mut sub_packages: Vec<SourcePackage> = self
                    .packages
                    .iter()
                    .filter(|p| needed.contains(&p.resolution_id))
                    .cloned()
                    .collect();
                sub_packages.push(SourcePackage {
                    resolution_id: scripts_res_id,
                    identity: scripts_identity,
                    name: "scripts".into(),
                    entry,
                });

                let mut sub_dependencies: Vec<resolve::DependencyEdge> = self
                    .dependencies
                    .iter()
                    .filter(|e| needed.contains(&e.requester) && needed.contains(&e.provider))
                    .cloned()
                    .collect();
                for edge in &self.dependencies {
                    if edge.requester == owner_res_id {
                        sub_dependencies.push(resolve::DependencyEdge {
                            requester: scripts_res_id,
                            alias: edge.alias.clone(),
                            provider: edge.provider,
                        });
                    }
                }

                (
                    sub_packages,
                    sub_dependencies,
                    scripts_res_id,
                    true,
                    owner_res_id,
                    scripts_res_id,
                )
            }
        };

        let mut provider_observed = if is_scripts && observed.is_some() {
            Some(CompilationInputs::default())
        } else {
            None
        };

        let selectors_owned = self.selectors.clone();
        let preludes_owned = self.preludes.clone();
        let target_owned = self.target.clone();
        let incremental_owned = self.incremental.clone();
        let output = crate::packages::compile_for_macro_provider(
            &sub_packages,
            &sub_dependencies,
            target_pkg_id,
            self.builtin,
            &sub_protocols,
            selectors_owned.as_selectors(),
            provider_observed.as_mut(),
            target_owned,
            &preludes_owned,
            self,
            &virtual_sources,
            force_discovery,
            &incremental_owned,
        )?;

        if let (Some(target_obs), Some(script_obs)) = (observed, provider_observed) {
            for s in script_obs.sources {
                if s.resolution_id == target_pkg_id {
                    target_obs.sources.push(s);
                }
            }
            for (rid, dir) in script_obs.directories {
                if rid == target_pkg_id {
                    target_obs.directories.push((rid, dir));
                }
            }
            for disc in script_obs.discovery {
                if disc.resolution_id == target_pkg_id {
                    target_obs.discovery.push(disc);
                }
            }
        }

        let meta_res_id = sub_dependencies
            .iter()
            .find(|e| e.requester == target_pkg_id && e.alias == "meta")
            .map(|e| e.provider)
            .or_else(|| {
                sub_packages
                    .iter()
                    .find(|p| p.name == "meta")
                    .map(|p| p.resolution_id)
            })
            .ok_or("construction macro package requires access to `meta` sysroot package")?;

        let ast_types = MacroAstTypes::discover(
            &output.nir,
            &output.module_tree,
            &output.package_scopes,
            meta_res_id,
            &output.package_identities,
        )?;
        let macros = extract_provider_macros(
            &output.nir,
            &output.module_tree,
            &output.package_scopes,
            target_pkg_id,
            &output.package_identities,
            &ast_types,
            is_scripts,
        )?;

        let session = MacroPackageSession::new(Arc::new(output.nir));
        for m in &macros {
            let id = MacroId {
                package: macro_pkg_u32,
                local: m.declaration.local,
            };
            self.compiled_macros.insert(
                id,
                CompiledMacroEntry {
                    provider: key,
                    declaration: m.declaration,
                    param_tys: m.param_tys.clone(),
                },
            );
        }

        self.providers.insert(
            key,
            PreparedMacroProvider {
                macro_package_id: macro_pkg_u32,
                is_scripts,
                owner_package: owner_pkg_id,
                session,
                ast_types,
                macros,
            },
        );
        Ok(())
    }

    fn build_registry_for_consumer(&self, aliases: &[(String, ProviderKey)]) -> MacroRegistry {
        let mut reg = MacroRegistry::with_builtins();
        let mut registered_ids = HashSet::new();

        for (alias, key) in aliases {
            reg.register_module_root(alias.clone());
            let Some(provider) = self.providers.get(key) else {
                continue;
            };
            for m in &provider.macros {
                let id = MacroId {
                    package: provider.macro_package_id,
                    local: m.declaration.local,
                };
                let mut canonical_path = Vec::with_capacity(1 + m.relative_path.len());
                canonical_path.push(alias.clone());
                canonical_path.extend(m.relative_path.iter().cloned());

                if registered_ids.insert(id) {
                    let vis_pkg = provider.owner_package;
                    let mut module_path = if provider.is_scripts {
                        vec!["<scripts>".into()]
                    } else {
                        vec![alias.clone()]
                    };
                    if m.relative_path.len() > 1 {
                        module_path
                            .extend(m.relative_path[..m.relative_path.len() - 1].iter().cloned());
                    }
                    let vis = match m.vis {
                        Visibility::Public => MacroVisibility::Public,
                        Visibility::Package => MacroVisibility::Package(vis_pkg),
                        Visibility::Private => MacroVisibility::Private,
                    };
                    reg.register(MacroDescriptor {
                        id,
                        canonical_path,
                        package: vis_pkg,
                        module_path,
                        vis,
                        def_site: m.def_site,
                        mode: m.mode.clone(),
                    });
                } else {
                    reg.register_path_alias(canonical_path, id);
                }
            }
        }

        reg
    }

    fn execute_provider_macro(
        &mut self,
        descriptor: &MacroDescriptor,
        arg_values: Vec<Value>,
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        let _ = budget.consume_step();
        let Some(entry) = self.compiled_macros.get(&descriptor.id).cloned() else {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!(
                    "missing prepared handler for construction macro `@{}`",
                    descriptor.full_path()
                ),
                call_site,
            ));
        };
        let fuel = self.macro_fuel_per_call;
        let current_ranges = self.current_source_ranges.clone();
        let provider = self.providers.get_mut(&entry.provider).ok_or_else(|| {
            MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!(
                    "missing prepared provider for construction macro `@{}`",
                    descriptor.full_path()
                ),
                call_site,
            )
        })?;
        let ast_types = provider.ast_types.clone();
        let output = provider
            .session
            .invoke_declaration(entry.declaration, arg_values, fuel)
            .map_err(|msg| {
                let kind = if msg.contains("steps") || msg.contains("budget") {
                    MacroErrorKind::BudgetExhausted
                } else {
                    MacroErrorKind::HandlerExecutionError
                };
                MacroError::new(
                    kind,
                    format!("macro `@{}` failed: {msg}", descriptor.full_path()),
                    call_site,
                )
            })?;

        decode_token_buffer_value(&output, &ast_types, &current_ranges, call_site).map_err(
            |reason| {
                MacroError::new(
                    MacroErrorKind::InvalidMacroOutput,
                    format!(
                        "macro `@{}` returned invalid TokenBuffer: {reason}",
                        descriptor.full_path()
                    ),
                    call_site,
                )
            },
        )
    }
}

impl MacroExecutor for MacroHost {
    fn execute_raw_source(
        &mut self,
        descriptor: &MacroDescriptor,
        raw_source: &str,
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        let arg = Value::String(Arc::from(raw_source));
        self.execute_provider_macro(descriptor, vec![arg], call_site, budget)
    }

    fn execute_tokens(
        &mut self,
        descriptor: &MacroDescriptor,
        input_tokens: &[MacroToken],
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        let Some(entry) = self.compiled_macros.get(&descriptor.id) else {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!("missing prepared handler for `@{}`", descriptor.full_path()),
                call_site,
            ));
        };
        let provider = self.providers.get(&entry.provider).ok_or_else(|| {
            MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!(
                    "missing prepared provider for `@{}`",
                    descriptor.full_path()
                ),
                call_site,
            )
        })?;
        // Use empty SourceMap reference fallback when Span is out-of-range; encode BytePos directly.
        let arg = encode_token_buffer_value(input_tokens, &provider.ast_types);
        self.execute_provider_macro(descriptor, vec![arg], call_site, budget)
    }

    fn execute_ast(
        &mut self,
        descriptor: &MacroDescriptor,
        args: &[AstArgValue],
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        let Some(entry) = self.compiled_macros.get(&descriptor.id).cloned() else {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!("missing prepared handler for `@{}`", descriptor.full_path()),
                call_site,
            ));
        };
        let ast_types = self
            .providers
            .get(&entry.provider)
            .ok_or_else(|| {
                MacroError::new(
                    MacroErrorKind::HandlerExecutionError,
                    format!(
                        "missing prepared provider for `@{}`",
                        descriptor.full_path()
                    ),
                    call_site,
                )
            })?
            .ast_types
            .clone();
        let MacroInputMode::Ast { params } = &descriptor.mode else {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!(
                    "internal error: `execute_ast` called for non-AST macro `@{}`",
                    descriptor.full_path()
                ),
                call_site,
            ));
        };
        if args.len() != params.len() || params.len() != entry.param_tys.len() {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!(
                    "AST macro `@{}` parameter count mismatch: expected {}, got {}",
                    descriptor.full_path(),
                    params.len(),
                    args.len()
                ),
                call_site,
            ));
        }
        let mut arg_values = Vec::with_capacity(args.len());
        for ((arg, &shape), param_ty) in args.iter().zip(params.iter()).zip(entry.param_tys.iter())
        {
            let encoded =
                encode_ast_arg_value(arg, shape, param_ty, &ast_types).map_err(|reason| {
                    MacroError::new(
                        MacroErrorKind::HandlerExecutionError,
                        format!(
                            "failed to marshal AST argument for `@{}`: {reason}",
                            descriptor.full_path()
                        ),
                        call_site,
                    )
                })?;
            arg_values.push(encoded);
        }
        self.execute_provider_macro(descriptor, arg_values, call_site, budget)
    }

    fn execute_peg(
        &mut self,
        descriptor: &MacroDescriptor,
        arg: &PegArgValue,
        call_site: Span,
        budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        let Some(entry) = self.compiled_macros.get(&descriptor.id).cloned() else {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!("missing prepared handler for `@{}`", descriptor.full_path()),
                call_site,
            ));
        };
        let ast_types = self
            .providers
            .get(&entry.provider)
            .ok_or_else(|| {
                MacroError::new(
                    MacroErrorKind::HandlerExecutionError,
                    format!(
                        "missing prepared provider for `@{}`",
                        descriptor.full_path()
                    ),
                    call_site,
                )
            })?
            .ast_types
            .clone();
        let MacroInputMode::Peg { param_shape, .. } = &descriptor.mode else {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!(
                    "internal error: `execute_peg` called for non-PEG macro `@{}`",
                    descriptor.full_path()
                ),
                call_site,
            ));
        };
        if entry.param_tys.len() != 1 {
            return Err(MacroError::new(
                MacroErrorKind::HandlerExecutionError,
                format!(
                    "PEG macro `@{}` handler must have 1 parameter, found {}",
                    descriptor.full_path(),
                    entry.param_tys.len()
                ),
                call_site,
            ));
        }
        let encoded = encode_peg_arg_value(arg, *param_shape, &entry.param_tys[0], &ast_types)
            .map_err(|reason| {
                MacroError::new(
                    MacroErrorKind::HandlerExecutionError,
                    format!(
                        "failed to marshal PEG input for `@{}`: {reason}",
                        descriptor.full_path()
                    ),
                    call_site,
                )
            })?;
        self.execute_provider_macro(descriptor, vec![encoded], call_site, budget)
    }
}

fn module_path_for_file(pkg_name: &str, rel_path: &Path) -> Vec<String> {
    let mut path = vec![pkg_name.to_string()];
    let parent = rel_path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = parent {
        for comp in dir.components() {
            if let std::path::Component::Normal(s) = comp {
                path.push(s.to_string_lossy().into_owned());
            }
        }
    }
    if let Some(stem) = rel_path.file_stem().and_then(|s| s.to_str()) {
        if !matches!(stem, "main" | "lib" | "mod") {
            path.push(stem.to_string());
        }
    }
    path
}

fn def_to_declaration_id(
    def_id: DefId,
    identities: &BTreeMap<u32, u64>,
) -> Result<DeclarationId, String> {
    let pkg = *identities
        .get(&def_id.pkg)
        .ok_or_else(|| format!("unknown semantic identity for package {}", def_id.pkg))?;
    Ok(DeclarationId {
        package: pkg,
        local: def_id.index,
    })
}

fn collect_module_scopes(tree: &resolve::ModuleTree) -> HashMap<DefId, resolve::ScopeId> {
    let mut map = HashMap::new();
    for scope in tree.scope_tree.iter() {
        if scope.kind == ScopeKind::Module && scope.owner_def.is_valid() {
            map.insert(scope.owner_def, scope.id);
        }
    }
    map
}

fn min_visibility(a: Visibility, b: Visibility) -> Visibility {
    match (a, b) {
        (Visibility::Private, _) | (_, Visibility::Private) => Visibility::Private,
        (Visibility::Package, _) | (_, Visibility::Package) => Visibility::Package,
        (Visibility::Public, Visibility::Public) => Visibility::Public,
    }
}

fn resolve_exported_binding(
    scope_tree: &resolve::scope::ScopeTree,
    scope_id: resolve::ScopeId,
    name: &str,
) -> Option<resolve::Binding> {
    let exports = collect_scope_exports(scope_tree, scope_id);
    exports
        .into_iter()
        .find(|(item_name, _, _)| item_name == name)
        .map(|(_, binding, _)| binding)
}

fn collect_scope_exports(
    scope_tree: &resolve::scope::ScopeTree,
    scope_id: resolve::ScopeId,
) -> Vec<(String, resolve::Binding, Visibility)> {
    let mut visited = HashSet::new();
    collect_scope_exports_inner(scope_tree, scope_id, &mut visited)
}

fn collect_scope_exports_inner(
    scope_tree: &resolve::scope::ScopeTree,
    scope_id: resolve::ScopeId,
    visited: &mut HashSet<resolve::ScopeId>,
) -> Vec<(String, resolve::Binding, Visibility)> {
    if !visited.insert(scope_id) {
        return Vec::new();
    }
    let Some(scope) = scope_tree.get(scope_id) else {
        visited.remove(&scope_id);
        return Vec::new();
    };

    let mut result = Vec::new();
    let mut seen_names = HashSet::new();

    for (sym, binding) in scope.items.declarations() {
        let name = sym.as_str().to_string();
        let eff_vis = min_visibility(binding.entry_visibility(), binding.vis);
        seen_names.insert(name.clone());
        result.push((name, binding.clone(), eff_vis));
    }

    for reexport in scope.items.reexports() {
        match reexport {
            resolve::import::ResolvedImport::Glob(src_scope) => {
                for (name, binding, vis) in
                    collect_scope_exports_inner(scope_tree, *src_scope, visited)
                {
                    if vis == Visibility::Public && seen_names.insert(name.clone()) {
                        result.push((name, binding, Visibility::Public));
                    }
                }
            }
            resolve::import::ResolvedImport::Single(src_scope, target_sym) => {
                let target_name = target_sym.as_str();
                if let Some((_, binding, vis)) =
                    collect_scope_exports_inner(scope_tree, *src_scope, visited)
                        .into_iter()
                        .find(|(n, _, _)| n == target_name)
                {
                    if vis == Visibility::Public && seen_names.insert(target_name.to_string()) {
                        result.push((target_name.to_string(), binding, Visibility::Public));
                    }
                }
            }
            resolve::import::ResolvedImport::Multi(src_scope, syms) => {
                let sub = collect_scope_exports_inner(scope_tree, *src_scope, visited);
                for sym in syms {
                    let target_name = sym.as_str();
                    if let Some((_, binding, vis)) = sub.iter().find(|(n, _, _)| n == target_name) {
                        if *vis == Visibility::Public && seen_names.insert(target_name.to_string())
                        {
                            result.push((
                                target_name.to_string(),
                                binding.clone(),
                                Visibility::Public,
                            ));
                        }
                    }
                }
            }
            resolve::import::ResolvedImport::Alias {
                source_scope,
                original,
                alias,
            } => {
                let orig_name = original.as_str();
                let alias_name = alias.as_str().to_string();
                if let Some((_, binding, vis)) =
                    collect_scope_exports_inner(scope_tree, *source_scope, visited)
                        .into_iter()
                        .find(|(n, _, _)| n == orig_name)
                {
                    if vis == Visibility::Public && seen_names.insert(alias_name.clone()) {
                        result.push((alias_name, binding, Visibility::Public));
                    }
                }
            }
        }
    }

    visited.remove(&scope_id);
    result
}

#[derive(Clone, Debug)]
enum DecodedMacroKind {
    Tokens,
    RawSource,
    Ast { variadic: bool },
    Peg { grammar_source: String },
}

fn decode_macro_kind_attribute(
    nir: &Package,
    decl_id: DeclarationId,
) -> Result<Option<DecodedMacroKind>, String> {
    let Some(attr_term) = nir
        .definition_attributes
        .lookup(decl_id)
        .map_err(|id| format!("macro attributes for {id} are still pending"))?
    else {
        return Ok(None);
    };
    let Term::NominalValue {
        ty,
        variant: None,
        fields,
    } = attr_term
    else {
        return Ok(None);
    };
    if **ty != Term::Primitive(Primitive::Object) {
        return Ok(None);
    }

    let mut kind_term: Option<&Term> = None;
    let mut variadic: Option<bool> = None;
    let mut grammar_source: Option<String> = None;
    for meta_entry in fields {
        let Some((mkey, _, mval)) = entry_parts(meta_entry) else {
            continue;
        };
        match mkey {
            Some("__flurry_macro_kind") => {
                kind_term = Some(mval);
            }
            Some("__flurry_macro_variadic") => match mval {
                Term::Bool(b) => {
                    variadic = Some(*b);
                }
                other => {
                    return Err(format!(
                        "invalid `__flurry_macro_variadic` metadata value {other}; expected bool"
                    ));
                }
            },
            Some("__flurry_macro_grammar") => match mval {
                Term::String(s) => {
                    grammar_source = Some(s.as_ref().to_string());
                }
                other => {
                    return Err(format!(
                        "invalid `__flurry_macro_grammar` metadata value {other}; expected str"
                    ));
                }
            },
            _ => {}
        }
    }
    if let Some(mval) = kind_term {
        match mval {
            Term::Symbol(s) if s.as_ref() == "tokens" => {
                return Ok(Some(DecodedMacroKind::Tokens));
            }
            Term::Symbol(s) if s.as_ref() == "source" => {
                return Ok(Some(DecodedMacroKind::RawSource));
            }
            Term::Symbol(s) if s.as_ref() == "ast" => {
                return Ok(Some(DecodedMacroKind::Ast {
                    variadic: variadic.unwrap_or(false),
                }));
            }
            Term::Symbol(s) if s.as_ref() == "peg" => {
                let grammar_source = grammar_source.ok_or_else(|| {
                    "missing `__flurry_macro_grammar` string metadata for `.peg` macro".to_string()
                })?;
                return Ok(Some(DecodedMacroKind::Peg { grammar_source }));
            }
            other => {
                return Err(format!(
                    "unsupported `__flurry_macro_kind` metadata value {other}"
                ));
            }
        }
    }
    Ok(None)
}

fn decode_flurry_ast_kind(ty: &Term, ast_types: &MacroAstTypes) -> Option<AstNodeKind> {
    let Term::Nominal {
        declaration,
        arguments,
    } = ty
    else {
        return None;
    };
    if !arguments.is_empty() {
        return None;
    }
    ast_types.kind_by_decl.get(declaration).copied()
}

fn decode_ast_param_shape(ty: &Term, ast_types: &MacroAstTypes) -> Option<AstParamShape> {
    if let Some(kind) = decode_flurry_ast_kind(ty, ast_types) {
        return Some(AstParamShape::Single(kind));
    }
    match ty {
        Term::Optional(inner) => {
            decode_flurry_ast_kind(inner, ast_types).map(AstParamShape::Optional)
        }
        Term::NormalForm {
            head, arguments, ..
        } if *head == ast_types.meta_list_head && arguments.len() == 1 => {
            decode_flurry_ast_kind(&arguments[0], ast_types).map(AstParamShape::Repetition)
        }
        _ => None,
    }
}

fn validate_macro_handler_signature(
    nir: &Package,
    decl_id: DeclarationId,
    decoded: DecodedMacroKind,
    ast_types: &MacroAstTypes,
    macro_name: &str,
) -> Result<(MacroInputMode, Vec<Term>, Option<Span>), String> {
    let func = nir
        .functions
        .iter()
        .find(|f| f.interface.declaration == decl_id)
        .ok_or_else(|| {
            format!("macro attribute on `{macro_name}` requires a `comptime fn` declaration")
        })?;
    let iface = &func.interface;
    if iface.phase != FunctionPhase::Comptime {
        return Err(format!(
            "macro `{macro_name}` must be declared `comptime fn`"
        ));
    }
    if !iface.type_parameters.is_empty() || !iface.env_parameters.is_empty() {
        return Err(format!(
            "macro handler `{macro_name}` cannot have generic or runtime environment parameters"
        ));
    }
    let Term::Function { parameters, result } = &iface.signature else {
        return Err(format!(
            "macro handler `{macro_name}` has invalid signature"
        ));
    };
    let tb_ty = Term::Nominal {
        declaration: ast_types.token_buffer.declaration,
        arguments: vec![],
    };
    if **result != tb_ty {
        return Err(format!(
            "macro handler `{macro_name}` return type must be `meta.ast.TokenBuffer`, found {}",
            result
        ));
    }

    let param_tys: Vec<Term> = parameters.iter().map(|p| p.ty.clone()).collect();
    let def_site = (iface.source.start <= iface.source.end && iface.source.start != u32::MAX)
        .then(|| Span::new(BytePos(iface.source.start), BytePos(iface.source.end)));

    match decoded {
        DecodedMacroKind::Tokens => {
            if parameters.len() != 1 {
                return Err(format!(
                    "macro handler `{macro_name}` must take exactly 1 parameter, found {}",
                    parameters.len()
                ));
            }
            if parameters[0].ty != tb_ty {
                return Err(format!(
                    "macro handler `{macro_name}` parameter must have type `meta.ast.TokenBuffer`, found {}",
                    parameters[0].ty
                ));
            }
            Ok((MacroInputMode::Tokens, param_tys, def_site))
        }
        DecodedMacroKind::RawSource => {
            if parameters.len() != 1 {
                return Err(format!(
                    "macro handler `{macro_name}` must take exactly 1 parameter, found {}",
                    parameters.len()
                ));
            }
            if parameters[0].ty != Term::Primitive(Primitive::Str) {
                return Err(format!(
                    "macro handler `{macro_name}` parameter must have type `str`, found {}",
                    parameters[0].ty
                ));
            }
            Ok((MacroInputMode::RawSource, param_tys, def_site))
        }
        DecodedMacroKind::Ast { variadic } => {
            let mut params = Vec::with_capacity(parameters.len());
            let mut saw_optional = false;
            let mut saw_repetition = false;

            for (idx, param) in parameters.iter().enumerate() {
                let shape = decode_ast_param_shape(&param.ty, ast_types).ok_or_else(|| {
                    format!(
                        "macro handler `{macro_name}` parameter #{} must have type `meta.ast.Flurry*`, `Optional<Flurry*>`, or `Repetition<Flurry*>`, found {}",
                        idx + 1,
                        param.ty
                    )
                })?;

                match shape {
                    AstParamShape::Single(_) => {
                        if saw_optional || saw_repetition {
                            return Err(format!(
                                "macro handler `{macro_name}` parameter #{} cannot follow an `Optional` or `Repetition` parameter",
                                idx + 1
                            ));
                        }
                    }
                    AstParamShape::Optional(_) => {
                        if saw_repetition {
                            return Err(format!(
                                "macro handler `{macro_name}` parameter #{} cannot follow a `Repetition` parameter",
                                idx + 1
                            ));
                        }
                        saw_optional = true;
                    }
                    AstParamShape::Repetition(_) => {
                        if idx + 1 != parameters.len() {
                            return Err(format!(
                                "macro handler `{macro_name}` `Repetition` parameter #{} must be the last parameter",
                                idx + 1
                            ));
                        }
                        saw_repetition = true;
                    }
                }
                params.push(shape);
            }

            if variadic {
                if !matches!(params.last(), Some(AstParamShape::Repetition(_))) {
                    return Err(format!(
                        "variadic `...` parameter in macro handler `{macro_name}` must have type `meta.ast.Repetition<meta.ast.Flurry*>`"
                    ));
                }
            } else if params
                .iter()
                .any(|p| matches!(p, AstParamShape::Repetition(_)))
            {
                return Err(format!(
                    "`Repetition<...>` parameter in `@define_macro1` macro `{macro_name}` must be marked variadic with `...`"
                ));
            }

            Ok((MacroInputMode::Ast { params }, param_tys, def_site))
        }
        DecodedMacroKind::Peg { grammar_source } => {
            let grammar = parse_and_validate_peg_grammar_str(&grammar_source)
                .map_err(|e| format!("invalid PEG grammar on macro `{macro_name}`: {e}"))?;
            if parameters.len() != 1 {
                return Err(format!(
                    "PEG macro handler `{macro_name}` must take exactly 1 parameter, found {}",
                    parameters.len()
                ));
            }
            let param_ty = &parameters[0].ty;
            let param_shape = if let Term::Nominal {
                declaration,
                arguments,
            } = param_ty
                && arguments.is_empty()
                && *declaration == ast_types.peg_node.declaration
            {
                PegParamShape::PegNode
            } else {
                match decode_ast_param_shape(param_ty, ast_types) {
                    Some(AstParamShape::Single(kind)) => PegParamShape::Single(kind),
                    Some(AstParamShape::Optional(kind)) => PegParamShape::Optional(kind),
                    Some(AstParamShape::Repetition(kind)) => PegParamShape::Repetition(kind),
                    None => {
                        return Err(format!(
                            "PEG macro handler `{macro_name}` parameter must have type `meta.ast.PegNode`, `Flurry*`, `Optional<Flurry*>`, or `Repetition<Flurry*>`, found {param_ty}"
                        ));
                    }
                }
            };
            if !grammar.supports_param_shape(param_shape) {
                return Err(format!(
                    "PEG macro `{macro_name}` `Entry` grammar does not match handler parameter shape {param_shape}"
                ));
            }
            Ok((
                MacroInputMode::Peg {
                    grammar,
                    param_shape,
                },
                param_tys,
                def_site,
            ))
        }
    }
}

fn extract_provider_macros(
    nir: &Package,
    tree: &resolve::ModuleTree,
    package_scopes: &HashMap<u32, resolve::ScopeId>,
    provider_res_id: u32,
    identities: &BTreeMap<u32, u64>,
    ast_types: &MacroAstTypes,
    is_scripts: bool,
) -> Result<Vec<ExtractedMacro>, String> {
    let module_scopes = collect_module_scopes(tree);
    let root_scope = *package_scopes
        .get(&provider_res_id)
        .ok_or_else(|| format!("missing package scope for macro provider {provider_res_id}"))?;

    let mut out = Vec::new();
    let mut seen_decls = HashSet::new();
    let mut visited_scopes = HashSet::new();

    #[allow(clippy::too_many_arguments)]
    fn walk_scope(
        nir: &Package,
        tree: &resolve::ModuleTree,
        scope_id: resolve::ScopeId,
        module_scopes: &HashMap<DefId, resolve::ScopeId>,
        identities: &BTreeMap<u32, u64>,
        ast_types: &MacroAstTypes,
        is_scripts: bool,
        prefix: Vec<String>,
        path_vis: Visibility,
        visited_scopes: &mut HashSet<(resolve::ScopeId, Vec<String>)>,
        seen_decls: &mut HashSet<(Vec<String>, DeclarationId)>,
        out: &mut Vec<ExtractedMacro>,
    ) -> Result<(), String> {
        if !visited_scopes.insert((scope_id, prefix.clone())) {
            return Ok(());
        }
        let mut exports = collect_scope_exports(&tree.scope_tree, scope_id);
        exports.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, binding, member_vis) in exports {
            let eff_vis = min_visibility(path_vis, member_vis);
            if !is_scripts && eff_vis != Visibility::Public {
                // Keep direct Package/Private macros in `out` when reachable via public path,
                // so macro calls get an explicit `[4010] MacroVisibilityError`.
            }
            let BindingKind::Definition(def_id, _) = binding.kind else {
                continue;
            };
            let mut next_path = prefix.clone();
            next_path.push(name.clone());

            if let Some(&sub_scope) = module_scopes.get(&def_id) {
                walk_scope(
                    nir,
                    tree,
                    sub_scope,
                    module_scopes,
                    identities,
                    ast_types,
                    is_scripts,
                    next_path,
                    eff_vis,
                    visited_scopes,
                    seen_decls,
                    out,
                )?;
                continue;
            }

            let Ok(decl_id) = def_to_declaration_id(def_id, identities) else {
                continue;
            };
            let Some(decoded_kind) = decode_macro_kind_attribute(nir, decl_id)? else {
                continue;
            };
            if !seen_decls.insert((next_path.clone(), decl_id)) {
                continue;
            }
            let joined = next_path.join(".");
            let (mode, param_tys, def_site) =
                validate_macro_handler_signature(nir, decl_id, decoded_kind, ast_types, &joined)?;
            out.push(ExtractedMacro {
                relative_path: next_path,
                declaration: decl_id,
                vis: eff_vis,
                def_site,
                mode,
                param_tys,
            });
        }
        Ok(())
    }

    walk_scope(
        nir,
        tree,
        root_scope,
        &module_scopes,
        identities,
        ast_types,
        is_scripts,
        Vec::new(),
        Visibility::Public,
        &mut visited_scopes,
        &mut seen_decls,
        &mut out,
    )?;

    Ok(out)
}

fn encode_source_span_value(span: Span, layout: &StructLayout) -> Value {
    Value::Record {
        arguments: vec![],
        declaration: layout.declaration,
        fields: vec![
            Value::String(Arc::from("<source>")),
            Value::UnsignedInteger(span.lo().0 as u128),
            Value::UnsignedInteger(span.hi().0 as u128),
            Value::SignedInteger(-1),
        ],
    }
}

fn encode_token_buffer_value(tokens: &[MacroToken], types: &MacroAstTypes) -> Value {
    let encoded_tokens: Vec<Value> = tokens
        .iter()
        .map(|tok| {
            let mut span_val = encode_source_span_value(tok.span, &types.source_span);
            if let Some(exp_id) = tok.expansion_id {
                if let Value::Record { fields, .. } = &mut span_val {
                    fields[3] = Value::SignedInteger(exp_id.0 as i128);
                }
            }
            Value::Record {
                arguments: vec![],
                declaration: types.token.declaration,
                fields: vec![
                    Value::Symbol(Arc::from(tok.kind.symbol_name())),
                    Value::String(Arc::from(tok.text.as_str())),
                    Value::Symbol(Arc::from(tok.spacing.as_str())),
                    span_val,
                ],
            }
        })
        .collect();

    let list_val = Value::List {
        ty: types.token_list_ty.clone(),
        values: ListValues::from(encoded_tokens),
    };

    Value::Record {
        arguments: vec![],
        declaration: types.token_buffer.declaration,
        fields: vec![list_val],
    }
}

fn encode_syntax_node_value(node: &ParsedSyntaxNode, types: &MacroAstTypes) -> Value {
    let children: Vec<Value> = node
        .children
        .iter()
        .map(|&idx| Value::Integer(Integer::from_u128(idx as u128)))
        .collect();
    Value::Record {
        arguments: vec![],
        declaration: types.syntax_node.declaration,
        fields: vec![
            Value::Symbol(Arc::from(node.kind.as_str())),
            encode_source_span_value(node.span, &types.source_span),
            Value::List {
                ty: types.int_list_ty.clone(),
                values: ListValues::from(children),
            },
            Value::String(Arc::from(node.text.as_str())),
        ],
    }
}

fn encode_syntax_tree_value(tree: &ParsedSyntaxTree, types: &MacroAstTypes) -> Value {
    let nodes: Vec<Value> = tree
        .nodes
        .iter()
        .map(|node| encode_syntax_node_value(node, types))
        .collect();
    Value::Record {
        arguments: vec![],
        declaration: types.syntax_tree.declaration,
        fields: vec![
            Value::List {
                ty: types.syntax_node_list_ty.clone(),
                values: ListValues::from(nodes),
            },
            Value::Integer(Integer::from_u128(tree.root as u128)),
            encode_token_buffer_value(&tree.tokens, types),
        ],
    }
}

fn encode_flurry_wrapper_value(
    kind: AstNodeKind,
    tree: &ParsedSyntaxTree,
    types: &MacroAstTypes,
) -> Result<Value, String> {
    let layout = types
        .wrapper_by_kind
        .get(&kind)
        .ok_or_else(|| format!("missing AST wrapper layout for `{kind}`"))?;
    Ok(Value::Record {
        arguments: vec![],
        declaration: layout.declaration,
        fields: vec![
            encode_syntax_tree_value(tree, types),
            Value::Integer(Integer::from_u128(tree.root as u128)),
        ],
    })
}

fn encode_ast_arg_value(
    arg: &AstArgValue,
    shape: AstParamShape,
    param_ty: &Term,
    types: &MacroAstTypes,
) -> Result<Value, String> {
    match (arg, shape) {
        (AstArgValue::Single(tree), AstParamShape::Single(kind)) => {
            encode_flurry_wrapper_value(kind, tree, types)
        }
        (AstArgValue::Optional(opt), AstParamShape::Optional(kind)) => {
            let encoded = opt
                .as_ref()
                .map(|tree| encode_flurry_wrapper_value(kind, tree, types).map(Box::new))
                .transpose()?;
            Ok(Value::Optional {
                ty: param_ty.clone(),
                value: encoded,
            })
        }
        (AstArgValue::Repetition(trees), AstParamShape::Repetition(kind)) => {
            let mut items = Vec::with_capacity(trees.len());
            for tree in trees {
                items.push(encode_flurry_wrapper_value(kind, tree, types)?);
            }
            Ok(Value::List {
                ty: param_ty.clone(),
                values: ListValues::from(items),
            })
        }
        _ => Err(format!(
            "mismatched AST argument representation ({arg}) for parameter shape {shape}"
        )),
    }
}

fn encode_peg_tree_node_value(node: &ParsedPegNode, types: &MacroAstTypes) -> Value {
    let children: Vec<Value> = node
        .children
        .iter()
        .map(|&idx| Value::Integer(Integer::from_u128(idx as u128)))
        .collect();
    Value::Record {
        arguments: vec![],
        declaration: types.peg_tree_node.declaration,
        fields: vec![
            Value::Symbol(Arc::from(node.rule.as_str())),
            encode_source_span_value(node.span, &types.source_span),
            Value::String(Arc::from(node.text.as_str())),
            encode_token_buffer_value(&node.tokens, types),
            Value::List {
                ty: types.int_list_ty.clone(),
                values: ListValues::from(children),
            },
        ],
    }
}

fn encode_peg_node_value(tree: &ParsedPegTree, types: &MacroAstTypes) -> Value {
    let encoded_nodes: Vec<Value> = tree
        .nodes
        .iter()
        .map(|node| encode_peg_tree_node_value(node, types))
        .collect();
    Value::Record {
        arguments: vec![],
        declaration: types.peg_node.declaration,
        fields: vec![
            Value::List {
                ty: types.peg_tree_node_list_ty.clone(),
                values: ListValues::from(encoded_nodes),
            },
            Value::Integer(Integer::from_u128(tree.root as u128)),
        ],
    }
}

fn encode_peg_arg_value(
    arg: &PegArgValue,
    shape: PegParamShape,
    param_ty: &Term,
    types: &MacroAstTypes,
) -> Result<Value, String> {
    match (arg, shape) {
        (PegArgValue::PegNode(tree), PegParamShape::PegNode) => {
            Ok(encode_peg_node_value(tree, types))
        }
        (PegArgValue::Single(tree), PegParamShape::Single(kind)) => {
            encode_flurry_wrapper_value(kind, tree, types)
        }
        (PegArgValue::Optional(opt), PegParamShape::Optional(kind)) => {
            let encoded = opt
                .as_ref()
                .map(|tree| encode_flurry_wrapper_value(kind, tree, types).map(Box::new))
                .transpose()?;
            Ok(Value::Optional {
                ty: param_ty.clone(),
                value: encoded,
            })
        }
        (PegArgValue::Repetition(trees), PegParamShape::Repetition(kind)) => {
            let mut items = Vec::with_capacity(trees.len());
            for tree in trees {
                items.push(encode_flurry_wrapper_value(kind, tree, types)?);
            }
            Ok(Value::List {
                ty: param_ty.clone(),
                values: ListValues::from(items),
            })
        }
        _ => Err(format!(
            "mismatched PEG argument representation ({arg}) for parameter shape {shape}"
        )),
    }
}

fn decode_source_span_value(
    val: &Value,
    layout: &StructLayout,
    valid_ranges: &[(u32, u32)],
    fallback: Span,
) -> Result<Span, String> {
    let Value::Record {
        declaration,
        fields,
        ..
    } = val
    else {
        return Err("expected `SourceSpan` record".into());
    };
    if *declaration != layout.declaration || fields.len() != 4 {
        return Err("mismatched `SourceSpan` record layout".into());
    }
    let Value::String(file) = &fields[0] else {
        return Err("expected `str` in `SourceSpan.file`".into());
    };
    let start: i128 = match &fields[1] {
        Value::UnsignedInteger(v) => {
            i128::try_from(*v).map_err(|e| format!("invalid `SourceSpan.start`: {e:?}"))?
        }
        Value::Integer(v) => v
            .to_i128()
            .map_err(|e| format!("invalid `SourceSpan.start`: {e:?}"))?,
        _ => return Err("expected `usize` in `SourceSpan.start`".into()),
    };
    let end: i128 = match &fields[2] {
        Value::UnsignedInteger(v) => {
            i128::try_from(*v).map_err(|e| format!("invalid `SourceSpan.end`: {e:?}"))?
        }
        Value::Integer(v) => v
            .to_i128()
            .map_err(|e| format!("invalid `SourceSpan.end`: {e:?}"))?,
        _ => return Err("expected `usize` in `SourceSpan.end`".into()),
    };

    if file.as_ref() != "<source>" || (start == 0 && end == 0) || start < 0 || end < start {
        return Ok(fallback);
    }
    let (Ok(lo), Ok(hi)) = (u32::try_from(start), u32::try_from(end)) else {
        return Ok(fallback);
    };
    if valid_ranges
        .iter()
        .any(|(f_lo, f_hi)| lo >= *f_lo && hi <= *f_hi)
    {
        Ok(Span::new(BytePos(lo), BytePos(hi)))
    } else {
        Ok(fallback)
    }
}

fn validate_decoded_token_text(kind: TokenKind, text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err("token text cannot be empty".into());
    }
    if matches!(
        kind,
        TokenKind::Sof
            | TokenKind::Eof
            | TokenKind::Invalid
            | TokenKind::Comment
            | TokenKind::MacroContent
    ) {
        return Err(format!(
            "disallowed token kind `{}` in macro output",
            kind.symbol_name()
        ));
    }
    if kind == TokenKind::Quote {
        return if text == "'" {
            Ok(())
        } else {
            Err("single_quote token text must be `'`".into())
        };
    }

    if is_separated_kind(kind) {
        let probe = format!(" {text} ");
        let (toks, _, errs) = lex::lex(&probe, BytePos(0));
        let sig: Vec<_> = toks
            .into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::Sof | TokenKind::Eof | TokenKind::Comment))
            .collect();
        if !errs.is_empty()
            || sig.len() != 1
            || sig[0].from != 1
            || sig[0].to != 1 + text.len()
            || sig[0].kind != kind
        {
            return Err(format!(
                "token text {:?} does not lex as separated operator `{}`",
                text,
                kind.symbol_name()
            ));
        }
        return Ok(());
    }

    let (toks, _, errs) = lex::lex(text, BytePos(0));
    let sig: Vec<_> = toks
        .into_iter()
        .filter(|t| !matches!(t.kind, TokenKind::Sof | TokenKind::Eof | TokenKind::Comment))
        .collect();
    if !errs.is_empty()
        || sig.len() != 1
        || sig[0].from != 0
        || sig[0].to != text.len()
        || sig[0].kind != kind
    {
        return Err(format!(
            "token text {:?} does not lex cleanly as `{}`",
            text,
            kind.symbol_name()
        ));
    }
    Ok(())
}

fn decode_token_spacing(sym: &str) -> Result<TokenSpacing, String> {
    match sym {
        "joint" => Ok(TokenSpacing::Joint),
        "alone" => Ok(TokenSpacing::Alone),
        "newline" => Ok(TokenSpacing::Newline),
        "separated" => Ok(TokenSpacing::Separated),
        other => Err(format!("unknown token spacing symbol `.{other}`")),
    }
}

fn decode_token_buffer_value(
    val: &Value,
    types: &MacroAstTypes,
    valid_ranges: &[(u32, u32)],
    call_site: Span,
) -> Result<Vec<MacroToken>, String> {
    let Value::Record {
        declaration,
        fields,
        ..
    } = val
    else {
        return Err(format!("expected `TokenBuffer` record, found {val:?}"));
    };
    if *declaration != types.token_buffer.declaration || fields.len() != 1 {
        return Err("value is not a `meta.ast.TokenBuffer`".into());
    }
    let Value::List { values, .. } = &fields[0] else {
        return Err("`TokenBuffer.tokens` is not a list".into());
    };

    let mut out = Vec::with_capacity(values.len());
    for elem in values.iter() {
        let Value::Record {
            declaration: tok_decl,
            fields: tok_fields,
            ..
        } = elem
        else {
            return Err("expected `Token` record element in `TokenBuffer.tokens`".into());
        };
        if *tok_decl != types.token.declaration || tok_fields.len() != 4 {
            return Err("element is not a `meta.ast.Token`".into());
        }
        let Value::Symbol(kind_sym) = &tok_fields[0] else {
            return Err("expected `Symbol` in `Token.kind`".into());
        };
        let kind = TokenKind::from_symbol_name(kind_sym)
            .ok_or_else(|| format!("unknown token kind symbol `.{kind_sym}`"))?;
        let Value::String(text) = &tok_fields[1] else {
            return Err("expected `str` in `Token.text`".into());
        };
        validate_decoded_token_text(kind, text)?;
        let Value::Symbol(spacing_sym) = &tok_fields[2] else {
            return Err("expected `Symbol` in `Token.spacing`".into());
        };
        let spacing = decode_token_spacing(spacing_sym)?;
        let span =
            decode_source_span_value(&tok_fields[3], &types.source_span, valid_ranges, call_site)?;
        out.push(MacroToken::new(kind, text.as_ref(), span, spacing, None));
    }
    Ok(out)
}
