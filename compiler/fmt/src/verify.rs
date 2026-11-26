//! Post-format self-verification: re-parse formatted output, verify comment retention,
//! and check canonical AST structural equivalence (including attributes and bridged macro regions).

use ast::{NodeIndex, NodeKind, NodeType};
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

use crate::bridge::{BridgedConstruct, ParsedSource, parse_and_bridge};
use crate::config::FormatConfig;
use crate::items::wraps_callable_def;

/// Re-parse `formatted` and verify that zero parse errors occur, no comments were dropped,
/// and the canonical AST matches `orig`.
pub fn verify_formatted_output(
    orig: &ParsedSource,
    formatted: &str,
    display_name: &str,
    config: &FormatConfig,
) -> Result<(), String> {
    let reparsed = parse_and_bridge(formatted, display_name, false).map_err(|err| {
        format!(
            "formatted output failed to re-parse: {}\n--- formatted ---\n{formatted}",
            err.rendered_diagnostics
        )
    })?;

    if reparsed.comments.len() != orig.comments.len() {
        return Err(format!(
            "comment count changed during formatting (before: {}, after: {})",
            orig.comments.len(),
            reparsed.comments.len()
        ));
    }

    let before_fp = canonical_node_fingerprint(orig, orig.ast.root, config.reorder_imports);
    let after_fp = canonical_node_fingerprint(&reparsed, reparsed.ast.root, config.reorder_imports);
    if before_fp != after_fp {
        return Err(format!(
            "AST structure changed during formatting:\n  before: {before_fp}\n   after: {after_fp}"
        ));
    }

    Ok(())
}

fn canonical_node_fingerprint(
    parsed: &ParsedSource,
    node: NodeIndex,
    allow_import_reorder: bool,
) -> String {
    if node == 0 {
        return "0".to_string();
    }

    let (exact_lo, _) = parsed.exact_byte_range(node, 1);
    if let Some(b) = parsed.bridge.constructs_by_start.get(&exact_lo) {
        return bridged_fingerprint(parsed, b, allow_import_reorder);
    }

    // Separate ordered outer `^attr` trees from canonicalized keyword modifiers (`pub`, `pure`, ...).
    let mut attrs = Vec::new();
    let mut kw_mods = Vec::new();
    let mut cur = node;

    loop {
        let Some(k) = parsed.ast.get_node_kind(cur) else {
            break;
        };
        let ch = parsed.ast.get_children(cur);
        match k {
            NodeKind::Attribute => {
                let (attr_lo, _) = parsed.exact_byte_range(cur, 1);
                if let Some(dm) = parsed.bridge.define_macro_offsets.get(&attr_lo) {
                    attrs.push(dm.as_str().to_string());
                } else {
                    attrs.push(canonical_node_fingerprint(
                        parsed,
                        ch[0],
                        allow_import_reorder,
                    ));
                }
                cur = ch[1];
            }
            NodeKind::Pub => {
                kw_mods.push("pub".to_string());
                cur = ch[0];
            }
            NodeKind::Private => {
                kw_mods.push("private".to_string());
                cur = ch[0];
            }
            NodeKind::FunctionTemplateDef => {
                kw_mods.push("template".to_string());
                cur = ch[0];
            }
            NodeKind::InlineStatement if wraps_callable_def(&parsed.ast, ch[0]) => {
                kw_mods.push("__flurry_kw_inline".to_string());
                cur = ch[0];
            }
            NodeKind::AttributeSetTrue => {
                let name = decode_id_raw(&parsed.ast, ch[0]);
                if name == "__flurry_kw_comptime_only_trait" {
                    kw_mods.push("__flurry_kw_pure".to_string());
                    kw_mods.push("__flurry_kw_comptime".to_string());
                } else if name.starts_with("__flurry_kw_") {
                    kw_mods.push(name);
                } else {
                    attrs.push(format!("(AttrSetTrue {name:?})"));
                }
                cur = ch[1];
            }
            _ => break,
        }
    }

    kw_mods.sort();

    let (core_lo, _) = parsed.exact_byte_range(cur, 1);
    let core_fp = if let Some(b) = parsed.bridge.constructs_by_start.get(&core_lo) {
        bridged_fingerprint(parsed, b, allow_import_reorder)
    } else {
        canonical_core_fingerprint(parsed, cur, allow_import_reorder)
    };

    if attrs.is_empty() && kw_mods.is_empty() {
        core_fp
    } else {
        format!(
            "(mod [{}] [{}] {core_fp})",
            attrs.join(" "),
            kw_mods.join(",")
        )
    }
}

fn canonical_core_fingerprint(
    parsed: &ParsedSource,
    node: NodeIndex,
    allow_import_reorder: bool,
) -> String {
    let Some(kind) = parsed.ast.get_node_kind(node) else {
        return "invalid".to_string();
    };
    let slots = parsed.ast.get_children(node);

    match kind {
        NodeKind::Id => {
            let s = decode_id_raw(&parsed.ast, node);
            return format!("(Id {s:?})");
        }
        NodeKind::Int | NodeKind::Real | NodeKind::Str | NodeKind::Char | NodeKind::Bool => {
            let sp = parsed.ast.get_span(node).unwrap_or_default();
            let lo = sp.lo().0.saturating_sub(parsed.base_pos.0) as usize;
            let hi = sp.hi().0.saturating_sub(parsed.base_pos.0) as usize;
            let text = parsed.original_src.get(lo..hi).unwrap_or("");
            return format!("({kind:?} {text:?})");
        }
        NodeKind::TokenBufferToken | NodeKind::TokenBufferDollar => {
            let sp = parsed.ast.get_span(node).unwrap_or_default();
            let lo = sp.lo().0.saturating_sub(parsed.base_pos.0) as usize;
            let hi = sp.hi().0.saturating_sub(parsed.base_pos.0) as usize;
            let text = parsed.original_src.get(lo..hi).unwrap_or("");
            return format!("({kind:?} {text:?})");
        }
        NodeKind::FnType => {
            let flags = slots[0];
            let params = parsed
                .ast
                .get_multi_child_slice(slots[1])
                .unwrap_or(&[])
                .iter()
                .map(|&p| canonical_node_fingerprint(parsed, p, allow_import_reorder))
                .collect::<Vec<_>>();
            return format!("(FnType {flags} [{}])", params.join(","));
        }
        NodeKind::ProjectionMultiPath => {
            let base = canonical_node_fingerprint(parsed, slots[0], allow_import_reorder);
            let mut subs = parsed
                .ast
                .get_multi_child_slice(slots[1])
                .unwrap_or(&[])
                .iter()
                .map(|&p| canonical_node_fingerprint(parsed, p, allow_import_reorder))
                .collect::<Vec<_>>();
            let (node_lo, node_hi) = parsed.exact_byte_range(node, 1);
            let has_inner_comment = parsed
                .comments
                .iter()
                .any(|c| c.byte_start >= node_lo && c.byte_start < node_hi);
            if allow_import_reorder && !has_inner_comment {
                subs.sort();
            }
            return format!("(ProjectionMultiPath {base} [{}])", subs.join(","));
        }
        NodeKind::CaseArm | NodeKind::ConditionArm | NodeKind::CatchArm => {
            let head = canonical_node_fingerprint(parsed, slots[0], allow_import_reorder);
            let mut body = slots[1];
            if parsed.ast.get_node_kind(body) == Some(NodeKind::ExprStatement)
                && let Some(&inner) = parsed.ast.get_children(body).first()
            {
                body = inner;
            }
            let body_fp = canonical_node_fingerprint(parsed, body, allow_import_reorder);
            return format!("({kind:?} {head} {body_fp})");
        }
        _ => {}
    }

    let list_slots = list_slots_for(kind.node_type());
    let is_container = matches!(kind, NodeKind::FileScope | NodeKind::Block);
    let mut parts = Vec::with_capacity(slots.len() + 1);
    let (exact_lo, _) = parsed.exact_byte_range(node, 1);
    let head_tag = match kind {
        NodeKind::UseStatement if parsed.bridge.at_use_offsets.contains(&exact_lo) => {
            "AtUseStatement".to_string()
        }
        NodeKind::Function if parsed.bridge.predicate_kw_offsets.contains(&exact_lo) => {
            "PredicateDef".to_string()
        }
        NodeKind::Requires if parsed.bridge.outcomes_kw_offsets.contains(&exact_lo) => {
            "Outcomes".to_string()
        }
        _ => format!("{kind:?}"),
    };
    parts.push(head_tag);

    for (slot_idx, &raw_val) in slots.iter().enumerate() {
        if list_slots.contains(&slot_idx) {
            let children = parsed.ast.get_multi_child_slice(raw_val).unwrap_or(&[]);
            let rendered_list = if is_container {
                canonical_container_list(parsed, children, allow_import_reorder)
            } else {
                children
                    .iter()
                    .map(|&c| canonical_node_fingerprint(parsed, c, allow_import_reorder))
                    .collect()
            };
            parts.push(format!("[{}]", rendered_list.join(",")));
        } else {
            parts.push(canonical_node_fingerprint(
                parsed,
                raw_val,
                allow_import_reorder,
            ));
        }
    }

    format!("({})", parts.join(" "))
}

/// Normalize contiguous sortable `UseStatement` runs (matching `formatter.rs` partition keys)
/// only when `allow_import_reorder` is enabled.
fn canonical_container_list(
    parsed: &ParsedSource,
    items: &[NodeIndex],
    allow_import_reorder: bool,
) -> Vec<String> {
    let mut out = Vec::with_capacity(items.len());
    let mut i = 0usize;
    while i < items.len() {
        if allow_import_reorder && let Some(part_key) = sortable_use_partition(parsed, items[i]) {
            let mut run = Vec::new();
            while i < items.len() && sortable_use_partition(parsed, items[i]) == Some(part_key) {
                run.push(canonical_node_fingerprint(
                    parsed,
                    items[i],
                    allow_import_reorder,
                ));
                i += 1;
            }
            run.sort();
            out.extend(run);
        } else {
            out.push(canonical_node_fingerprint(
                parsed,
                items[i],
                allow_import_reorder,
            ));
            i += 1;
        }
    }
    out
}

fn sortable_use_partition(parsed: &ParsedSource, mut node: NodeIndex) -> Option<(u8, bool)> {
    let mut vis = 0u8;
    loop {
        match parsed.ast.get_node_kind(node)? {
            NodeKind::Pub => {
                vis = 1;
                node = *parsed.ast.get_children(node).first()?;
            }
            NodeKind::Private => {
                vis = 2;
                node = *parsed.ast.get_children(node).first()?;
            }
            NodeKind::Attribute | NodeKind::AttributeSetTrue => return None,
            NodeKind::UseStatement => {
                let (exact_lo, _) = parsed.exact_byte_range(node, 1);
                let is_at_use = parsed.bridge.at_use_offsets.contains(&exact_lo);
                return Some((vis, is_at_use));
            }
            _ => return None,
        }
    }
}

fn decode_id_raw(ast: &ast::Ast, node: NodeIndex) -> String {
    let slots = ast.get_children(node);
    if slots.len() == 2 {
        let sym = unsafe { symbol::Symbol::from_raw_parts(slots[0], slots[1]) };
        sym.as_str().to_string()
    } else {
        String::new()
    }
}

fn bridged_fingerprint(
    parsed: &ParsedSource,
    b: &BridgedConstruct,
    allow_import_reorder: bool,
) -> String {
    match b {
        BridgedConstruct::TemplateMacroDef {
            start,
            end,
            kw_start,
            brace_start,
        } => {
            let prefix = parsed
                .original_src
                .get(*start..*kw_start)
                .unwrap_or("")
                .trim();
            let header = parsed
                .original_src
                .get(*kw_start..*brace_start)
                .unwrap_or("")
                .trim();
            let body =
                normalize_verbatim_text(parsed.original_src.get(*brace_start..*end).unwrap_or(""));
            format!("(BridgedTemplateMacroDef {prefix:?} {header:?} {body:?})")
        }
        BridgedConstruct::BraceMacroCall {
            start,
            end,
            path,
            brace_start,
        } => {
            let raw = parsed.original_src.get(*start..*end).unwrap_or("");
            let is_raw = path == "raw" || path.ends_with(".raw");
            if !is_raw && let Some(body) = parsed.original_src.get(*brace_start..*end) {
                let synthetic = format!("fn __fmt_verify_brace() {body}\n");
                if let Ok(sub) = parse_and_bridge(&synthetic, "<verify-brace-macro>", false) {
                    let ast_fp =
                        canonical_node_fingerprint(&sub, sub.ast.root, allow_import_reorder);
                    return format!(
                        "(BridgedBraceMacro {path:?} comments={} ast={ast_fp})",
                        sub.comments.len()
                    );
                }
            }
            format!(
                "(BridgedBraceMacro {path:?} raw={:?})",
                normalize_verbatim_text(raw)
            )
        }
        BridgedConstruct::ParenMacroCall {
            start,
            end,
            path,
            args_inner,
        } => {
            if let Some((arg_lo, arg_hi)) = *args_inner {
                let inner_raw = parsed.original_src.get(arg_lo..arg_hi).unwrap_or("");
                if inner_raw.trim().is_empty() {
                    return format!("(BridgedParenMacro {path:?} empty)");
                }
                let synthetic = format!("__fmt_call({inner_raw});\n");
                if let Ok(sub) = parse_and_bridge(&synthetic, "<verify-macro-args>", false) {
                    let ast_fp =
                        canonical_node_fingerprint(&sub, sub.ast.root, allow_import_reorder);
                    return format!(
                        "(BridgedParenMacro {path:?} comments={} ast={ast_fp})",
                        sub.comments.len()
                    );
                }
                let raw = parsed.original_src.get(*start..*end).unwrap_or("");
                format!(
                    "(BridgedParenMacro {path:?} raw={:?})",
                    normalize_verbatim_text(raw)
                )
            } else {
                format!("(BridgedParenMacro {path:?} bare)")
            }
        }
        BridgedConstruct::DollarMacroCall { start, end } => {
            let raw = parsed.original_src.get(*start..*end).unwrap_or("");
            format!("(BridgedDollarMacro {:?})", normalize_verbatim_text(raw))
        }
        BridgedConstruct::TokenBuffer { start, end } => {
            let raw = parsed.original_src.get(*start..*end).unwrap_or("");
            if let Some(ast_fp) = reparse_token_buffer_fingerprint(raw, allow_import_reorder) {
                format!(
                    "(BridgedTokenBuffer ast={ast_fp} raw={:?})",
                    normalize_verbatim_text(raw)
                )
            } else {
                format!(
                    "(BridgedTokenBuffer raw={:?})",
                    normalize_verbatim_text(raw)
                )
            }
        }
        BridgedConstruct::SkippedUnparsed { start, end } => {
            let raw = parsed.original_src.get(*start..*end).unwrap_or("");
            format!("(BridgedSkipped {:?})", normalize_skipped_text(raw))
        }
    }
}

fn reparse_token_buffer_fingerprint(raw: &str, allow_import_reorder: bool) -> Option<String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(
        FileName::Custom("<verify-token-buffer>".to_string()),
        raw.to_string(),
    );
    let (tokens, symbols, lex_errors) = lex::lex(raw, file.start_pos);
    if !lex_errors.is_empty() {
        return None;
    }
    let sig_tokens: Vec<_> = tokens
        .iter()
        .copied()
        .filter(|t| t.kind != lex::TokenKind::Comment)
        .collect();
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser.try_expr().ok().filter(|&n| n != 0)?;
    if parser.peek_next_token().kind != lex::TokenKind::Eof {
        return None;
    }
    let mut ast = parser.finalize();
    ast.root = root;
    let line_starts = vec![0];
    let parsed = ParsedSource {
        original_src: raw.to_string(),
        shadow_src: raw.to_string(),
        line_starts,
        comments: Vec::new(),
        directives: Vec::new(),
        warnings: Vec::new(),
        off_regions: Vec::new(),
        bridge: Default::default(),
        sig_tokens,
        ast,
        base_pos: file.start_pos,
    };
    Some(canonical_node_fingerprint(
        &parsed,
        parsed.ast.root,
        allow_import_reorder,
    ))
}

fn normalize_verbatim_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
}

fn normalize_skipped_text(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    if lines.len() <= 1 {
        return normalized.trim_end_matches([' ', '\t']).to_string();
    }
    let min_tail_indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.bytes().take_while(|&b| b == b' ' || b == b'\t').count())
        .min()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(lines.len());
    out.push(lines[0].trim_end_matches([' ', '\t']).to_string());
    for line in lines.iter().skip(1) {
        let trimmed = line.trim_end_matches([' ', '\t']);
        if trimmed.is_empty() {
            out.push(String::new());
        } else {
            out.push(trimmed[min_tail_indent.min(trimmed.len())..].to_string());
        }
    }
    out.join("\n")
}

fn list_slots_for(nt: NodeType) -> &'static [usize] {
    use NodeType::*;
    match nt {
        MultiChildren => &[0],
        SingleWithMultiChildren | TypeDefChildren | TypeAliasChildren | FnTypeChildren => &[1],
        DoubleWithMultiChildren
        | TraitDefChildren
        | ImplTraitDefChildren
        | ExtendTraitDefChildren => &[2],
        TripleWithMultiChildren => &[3],
        FunctionDefChildren | NormalFormHeadDefChildren | AlgebraicEffectChildren => &[1, 3],
        AssocDeclChildren => &[1, 4],
        _ => &[],
    }
}
