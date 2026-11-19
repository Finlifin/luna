//! Post-format self-verification: re-parse formatted output, verify comment retention,
//! and check canonical AST structural equivalence (modulo modifier normalization & `use` sorting).

use ast::{NodeIndex, NodeKind, NodeType};

use crate::bridge::{BridgedConstruct, ParsedSource, parse_and_bridge};
use crate::items::wraps_callable_def;

/// Re-parse `formatted` and verify that zero parse errors occur, no comments were dropped,
/// and the canonical AST matches `orig`.
pub fn verify_formatted_output(
    orig: &ParsedSource,
    formatted: &str,
    display_name: &str,
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

    let before_fp = canonical_node_fingerprint(orig, orig.ast.root);
    let after_fp = canonical_node_fingerprint(&reparsed, reparsed.ast.root);
    if before_fp != after_fp {
        return Err(format!(
            "AST structure changed during formatting:\n  before: {before_fp}\n   after: {after_fp}"
        ));
    }

    Ok(())
}

fn canonical_node_fingerprint(parsed: &ParsedSource, node: NodeIndex) -> String {
    if node == 0 {
        return "0".to_string();
    }

    let (exact_lo, _) = parsed.exact_byte_range(node, 1);
    if let Some(b) = parsed.bridge.constructs_by_start.get(&exact_lo) {
        return bridged_fingerprint(b);
    }

    // Normalize outer `Attribute` / `Pub` / `Private` / `FunctionTemplateDef` / `AttributeSetTrue`
    // chains into `(attrs_in_order, sorted_kw_modifiers, core_node)`!
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
                // Check if this attribute was a bridged `@define_macro1/2`.
                let (attr_lo, _) = parsed.exact_byte_range(cur, 1);
                if let Some(dm) = parsed.bridge.define_macro_offsets.get(&attr_lo) {
                    attrs.push(dm.as_str().to_string());
                } else {
                    attrs.push(canonical_node_fingerprint(parsed, ch[0]));
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
                } else {
                    kw_mods.push(name);
                }
                cur = ch[1];
            }
            _ => break,
        }
    }

    kw_mods.sort();
    kw_mods.dedup();

    let (core_lo, _) = parsed.exact_byte_range(cur, 1);
    let core_fp = if let Some(b) = parsed.bridge.constructs_by_start.get(&core_lo) {
        bridged_fingerprint(b)
    } else {
        canonical_core_fingerprint(parsed, cur)
    };

    if attrs.is_empty() && kw_mods.is_empty() {
        core_fp
    } else {
        format!(
            "(mod [{}] [{}] {core_fp})",
            attrs.join(","),
            kw_mods.join(",")
        )
    }
}

fn canonical_core_fingerprint(parsed: &ParsedSource, node: NodeIndex) -> String {
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
        NodeKind::FnType => {
            let flags = slots[0];
            let params = parsed
                .ast
                .get_multi_child_slice(slots[1])
                .unwrap_or(&[])
                .iter()
                .map(|&p| canonical_node_fingerprint(parsed, p))
                .collect::<Vec<_>>();
            return format!("(FnType {flags} [{}])", params.join(","));
        }
        NodeKind::ProjectionMultiPath => {
            let base = canonical_node_fingerprint(parsed, slots[0]);
            let mut subs = parsed
                .ast
                .get_multi_child_slice(slots[1])
                .unwrap_or(&[])
                .iter()
                .map(|&p| canonical_node_fingerprint(parsed, p))
                .collect::<Vec<_>>();
            subs.sort();
            return format!("(ProjectionMultiPath {base} [{}])", subs.join(","));
        }
        NodeKind::CaseArm | NodeKind::ConditionArm | NodeKind::CatchArm => {
            let head = canonical_node_fingerprint(parsed, slots[0]);
            let mut body = slots[1];
            if parsed.ast.get_node_kind(body) == Some(NodeKind::ExprStatement)
                && let Some(&inner) = parsed.ast.get_children(body).first()
            {
                body = inner;
            }
            let body_fp = canonical_node_fingerprint(parsed, body);
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
                canonical_container_list(parsed, children)
            } else {
                children
                    .iter()
                    .map(|&c| canonical_node_fingerprint(parsed, c))
                    .collect()
            };
            parts.push(format!("[{}]", rendered_list.join(",")));
        } else {
            parts.push(canonical_node_fingerprint(parsed, raw_val));
        }
    }

    format!("({})", parts.join(" "))
}

/// Normalize runs of `UseStatement` items so `.reorder_imports` passes structural equivalence!
fn canonical_container_list(parsed: &ParsedSource, items: &[NodeIndex]) -> Vec<String> {
    let mut out = Vec::with_capacity(items.len());
    let mut i = 0usize;
    while i < items.len() {
        if is_use_item(&parsed.ast, items[i]) {
            let mut run = Vec::new();
            while i < items.len() && is_use_item(&parsed.ast, items[i]) {
                run.push(canonical_node_fingerprint(parsed, items[i]));
                i += 1;
            }
            run.sort();
            out.extend(run);
        } else {
            out.push(canonical_node_fingerprint(parsed, items[i]));
            i += 1;
        }
    }
    out
}

fn is_use_item(ast: &ast::Ast, mut node: NodeIndex) -> bool {
    loop {
        match ast.get_node_kind(node) {
            Some(NodeKind::Pub | NodeKind::Private) => {
                node = ast.get_children(node)[0];
            }
            Some(NodeKind::UseStatement) => return true,
            _ => return false,
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

fn bridged_fingerprint(b: &BridgedConstruct) -> String {
    match b {
        BridgedConstruct::TemplateMacroDef { .. } => "(BridgedTemplateMacroDef)".to_string(),
        BridgedConstruct::BraceMacroCall { path, .. } => format!("(BridgedBraceMacro {path:?})"),
        BridgedConstruct::ParenMacroCall { path, .. } => format!("(BridgedParenMacro {path:?})"),
        BridgedConstruct::DollarMacroCall { .. } => "(BridgedDollarMacro)".to_string(),
        BridgedConstruct::TokenBuffer { .. } => "(BridgedTokenBuffer)".to_string(),
        BridgedConstruct::SkippedUnparsed { .. } => "(BridgedSkipped)".to_string(),
    }
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
