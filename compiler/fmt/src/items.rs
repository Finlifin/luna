//! Top-level and nested item formatting, canonical modifier ordering, parameters, `where` clauses,
//! ADT variants, and `use` import sorting/formatting.

use ast::{NodeIndex, NodeKind};
use lex::TokenKind;

use crate::doc::Doc;
use crate::formatter::{ContainerKind, Formatter, TokenSlot, compare_import_keys};
use crate::patterns::{DelimStyle, find_matching_delim, strip_last_hardline};

/// Collected canonical modifiers for a declaration.
#[derive(Debug, Default, Clone)]
struct CollectedModifiers {
    vis: Option<&'static str>,
    is_template: bool,
    is_pure: bool,
    is_quote: bool,
    is_comptime: bool,
    is_async: bool,
    is_inline: bool,
    is_unsafe: bool,
    is_spec: bool,
    is_verified: bool,
    is_atomic: bool,
}

impl CollectedModifiers {
    fn to_prefix_string(&self) -> String {
        let mut parts = Vec::new();
        if let Some(v) = self.vis {
            parts.push(v);
        }
        if self.is_template {
            parts.push("template");
        }
        if self.is_pure {
            parts.push("pure");
        }
        if self.is_quote {
            parts.push("quote");
        }
        if self.is_comptime {
            parts.push("comptime");
        }
        if self.is_async {
            parts.push("async");
        }
        if self.is_inline {
            parts.push("inline");
        }
        if self.is_unsafe {
            parts.push("unsafe");
        }
        if self.is_spec {
            parts.push("spec");
        }
        if self.is_verified {
            parts.push("verified");
        }
        if self.is_atomic {
            parts.push("atomic");
        }
        parts.join(" ")
    }
}

impl<'a> Formatter<'a> {
    /// Format a single attribute line (`^expr` or bridged `@define_macro1`/`@define_macro2`).
    pub fn format_attribute_line(&mut self, attr_expr: NodeIndex, attr_slot: TokenSlot) -> Doc {
        let caret_tok_idx = attr_slot.first.saturating_sub(1);
        if let Some(caret_tok) = self.parsed.sig_tokens.get(caret_tok_idx) {
            if caret_tok.kind == TokenKind::Caret {
                if let Some(dm) = self.parsed.bridge.define_macro_offsets.get(&caret_tok.from) {
                    return Doc::str(dm.as_str());
                }
            }
        }
        let expr_doc = self.format_expr(attr_expr, attr_slot);
        Doc::concat([Doc::str("^"), expr_doc])
    }

    /// Format a single member inside a container (`FileScope`, `ModuleBody`, `AdtBody`,
    /// `TraitOrImplBody`, or `ValueBlock`), returning `(doc, ends_with_rbrace)`.
    pub fn format_container_member(
        &mut self,
        node: NodeIndex,
        slot: TokenSlot,
        _is_last: bool,
        container_kind: ContainerKind,
    ) -> (Doc, bool) {
        if let Some(bridged) = self.bridged_at(node, slot.first) {
            let ends_brace = matches!(
                bridged,
                crate::bridge::BridgedConstruct::BraceMacroCall { .. }
                    | crate::bridge::BridgedConstruct::TokenBuffer { .. }
                    | crate::bridge::BridgedConstruct::TemplateMacroDef { .. }
            ) || self
                .parsed
                .original_src
                .get(bridged.span_bytes().0..bridged.span_bytes().1)
                .is_some_and(|s| s.trim_end().ends_with('}'));
            return (self.format_bridged(bridged), ends_brace);
        }

        // Unwrap outer `Attribute` / `Pub` / `Private` / `FunctionTemplateDef` / `AttributeSetTrue`.
        let (attr_prefix_doc, mut mods, inner_node, inner_slot) =
            self.unwrap_item_modifiers(node, slot);

        // Check bridged construct on the unwrapped core item (e.g. `pub define Foo() { ... }`).
        if let Some(bridged) = self.bridged_at(inner_node, inner_slot.first) {
            let mod_prefix = mods.to_prefix_string();
            let b_doc = self.format_bridged(bridged);
            let combined = if mod_prefix.is_empty() {
                b_doc
            } else {
                Doc::concat([Doc::text(format!("{mod_prefix} ")), b_doc])
            };
            let ends_brace = matches!(
                bridged,
                crate::bridge::BridgedConstruct::BraceMacroCall { .. }
                    | crate::bridge::BridgedConstruct::TokenBuffer { .. }
                    | crate::bridge::BridgedConstruct::TemplateMacroDef { .. }
            );
            return (Doc::concat([attr_prefix_doc, combined]), ends_brace);
        }

        let Some(kind) = self.parsed.ast.get_node_kind(inner_node) else {
            return (attr_prefix_doc, false);
        };

        if kind == NodeKind::NormalFormHeadDef {
            mods.is_pure = true;
            mods.is_comptime = true;
        }
        if kind == NodeKind::TestTemplateDef {
            mods.is_template = true;
        }

        let mod_str = mods.to_prefix_string();
        let mod_prefix = if mod_str.is_empty() {
            Doc::Nil
        } else {
            Doc::text(format!("{mod_str} "))
        };

        let (body_doc, ends_with_rbrace) = match kind {
            NodeKind::Configuration => {
                let ch = self.parsed.ast.get_children(inner_node);
                // `.id expr` (`inner_slot.first` is `.`)
                let id_str = self.decode_id(ch[0], inner_slot.first + 1);
                let val_slot = inner_slot.child_slot_from(self.parsed, inner_slot.first + 2, ch[1]);
                let (val_doc, eb) = self.format_expr_with_rbrace(ch[1], val_slot);
                (
                    Doc::concat([Doc::str("."), Doc::text(id_str), Doc::space(), val_doc]),
                    eb,
                )
            }
            NodeKind::Property => (self.format_property(inner_node, inner_slot), false),
            NodeKind::StructField => (self.format_struct_field(inner_node, inner_slot), false),
            NodeKind::Id if container_kind == ContainerKind::AdtBody => (
                Doc::text(self.decode_id(inner_node, inner_slot.first)),
                false,
            ),
            NodeKind::PatternEnumVariant
            | NodeKind::ExprEnumVariant
            | NodeKind::TupleEnumVariant
            | NodeKind::StructEnumVariant
            | NodeKind::SubEnumEnumVariant => {
                self.format_enum_variant(inner_node, kind, inner_slot)
            }
            NodeKind::UnionVariant => {
                let ch = self.parsed.ast.get_children(inner_node);
                let id_str = self.decode_id(ch[0], inner_slot.first);
                let colon_tok = self
                    .parsed
                    .find_depth_zero_token(
                        inner_slot.first + 1,
                        inner_slot.last,
                        |k| k == TokenKind::Colon,
                        false,
                    )
                    .unwrap_or(inner_slot.first + 1);
                let ty_slot = inner_slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
                let ty_doc = self.format_expr(ch[1], ty_slot);
                (
                    Doc::concat([Doc::text(id_str), Doc::str(": "), ty_doc]),
                    false,
                )
            }
            NodeKind::UseStatement => {
                let (u_doc, eb, _) = self.format_use_core(inner_node, inner_slot);
                (u_doc, eb)
            }
            NodeKind::ModStatement => {
                let ch = self.parsed.ast.get_children(inner_node);
                let id_str = self.decode_id(ch[0], inner_slot.first + 1);
                (Doc::concat([Doc::str("mod "), Doc::text(id_str)]), false)
            }
            NodeKind::ModuleDef => {
                let ch = self.parsed.ast.get_children(inner_node);
                let id_str = self.decode_id(ch[0], inner_slot.first + 1);
                let open_brace = self
                    .parsed
                    .find_depth_zero_token(
                        inner_slot.first + 2,
                        inner_slot.last,
                        |k| k == TokenKind::LBrace,
                        false,
                    )
                    .unwrap_or(inner_slot.first + 2);
                let b_doc = self.format_block(
                    ch[1],
                    TokenSlot::new(open_brace, inner_slot.last),
                    None,
                    ContainerKind::ModuleBody,
                );
                (
                    Doc::concat([Doc::str("mod "), Doc::text(id_str), Doc::space(), b_doc]),
                    true,
                )
            }
            NodeKind::Function | NodeKind::NormalFormHeadDef => {
                self.format_function_def(inner_node, kind, inner_slot)
            }
            NodeKind::FunctionTemplateInstance => {
                let ch = self.parsed.ast.get_children(inner_node);
                let id_str = self.decode_id(ch[0], inner_slot.first + 1);
                let use_tok = self
                    .parsed
                    .find_depth_zero_token(
                        inner_slot.first + 2,
                        inner_slot.last,
                        |k| k == TokenKind::Use,
                        false,
                    )
                    .unwrap_or(inner_slot.first + 2);
                let tmpl_slot = inner_slot.child_slot_from(self.parsed, use_tok + 1, ch[1]);
                let tmpl_doc = self.format_expr(ch[1], tmpl_slot);
                let b_doc = self.format_block(
                    ch[2],
                    TokenSlot::new(tmpl_slot.last + 1, inner_slot.last),
                    None,
                    ContainerKind::ValueBlock,
                );
                (
                    Doc::concat([
                        Doc::str("fn "),
                        Doc::text(id_str),
                        Doc::str(" use "),
                        tmpl_doc,
                        Doc::space(),
                        b_doc,
                    ]),
                    true,
                )
            }
            NodeKind::FunctionTemplateInstanceWithParams => {
                let ch = self.parsed.ast.get_children(inner_node);
                let id_str = self.decode_id(ch[0], inner_slot.first + 1);
                let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let open_paren = inner_slot.first + 2;
                let close_paren = find_matching_delim(
                    &self.parsed.sig_tokens,
                    open_paren,
                    TokenKind::LParen,
                    TokenKind::RParen,
                )
                .unwrap_or(open_paren + 1);
                let params_doc = self.format_delimited_seq(
                    params,
                    open_paren,
                    close_paren,
                    DelimStyle::Parens,
                    None,
                    |fmt, p, ps| fmt.format_clause_or_param(p, ps),
                );
                let use_tok = close_paren + 1;
                let tmpl_slot = inner_slot.child_slot_from(self.parsed, use_tok + 1, ch[2]);
                let tmpl_doc = self.format_expr(ch[2], tmpl_slot);
                let b_doc = self.format_block(
                    ch[4],
                    TokenSlot::new(tmpl_slot.last + 1, inner_slot.last),
                    None,
                    ContainerKind::ValueBlock,
                );
                (
                    Doc::concat([
                        Doc::str("fn "),
                        Doc::text(id_str),
                        params_doc,
                        Doc::str(" use "),
                        tmpl_doc,
                        Doc::space(),
                        b_doc,
                    ]),
                    true,
                )
            }
            NodeKind::AlgebraicEffect => (
                self.format_algebraic_effect(inner_node, inner_slot, false),
                false,
            ),
            NodeKind::EffectEnumDef => (self.format_effect_enum_def(inner_node, inner_slot), true),
            NodeKind::StructDef | NodeKind::EnumDef | NodeKind::UnionDef => {
                (self.format_adt_def(inner_node, kind, inner_slot), true)
            }
            NodeKind::TraitDef => (self.format_trait_def(inner_node, inner_slot), true),
            NodeKind::AssocDecl => self.format_assoc_decl(inner_node, inner_slot),
            NodeKind::ImplDef | NodeKind::ExtendDef => (
                self.format_impl_or_extend_inherent(inner_node, kind, inner_slot),
                true,
            ),
            NodeKind::ImplTraitDef | NodeKind::ExtendTraitDef => (
                self.format_impl_or_extend_trait(inner_node, kind, inner_slot),
                true,
            ),
            NodeKind::DeriveDef => (self.format_derive_def(inner_node, inner_slot), false),
            NodeKind::DeriverDef => (self.format_deriver_def(inner_node, inner_slot), true),
            NodeKind::CaseDef => (self.format_case_def(inner_node, inner_slot), true),
            NodeKind::TypealiasDef | NodeKind::NewtypeDef => {
                self.format_typealias_or_newtype(inner_node, kind, inner_slot)
            }
            NodeKind::ConstDef => self.format_const_def(inner_node, inner_slot),
            NodeKind::StaticDef => self.format_static_def(inner_node, inner_slot),
            NodeKind::TestDef => {
                let ch = self.parsed.ast.get_children(inner_node);
                let id = ch[0];
                let block = ch[1];
                if id != 0 {
                    let id_str = self.decode_id(id, inner_slot.first + 1);
                    let b_doc = self.format_block(
                        block,
                        TokenSlot::new(inner_slot.first + 2, inner_slot.last),
                        None,
                        ContainerKind::ValueBlock,
                    );
                    (
                        Doc::concat([Doc::str("test "), Doc::text(id_str), Doc::space(), b_doc]),
                        true,
                    )
                } else {
                    let b_doc = self.format_block(
                        block,
                        TokenSlot::new(inner_slot.first + 1, inner_slot.last),
                        None,
                        ContainerKind::ValueBlock,
                    );
                    (Doc::concat([Doc::str("test "), b_doc]), true)
                }
            }
            NodeKind::TestTemplateDef => {
                // Note: `mods.is_template` already emits `template ` before `test ...`!
                let ch = self.parsed.ast.get_children(inner_node);
                let id_str = self.decode_id(ch[0], inner_slot.first + 1);
                let clauses = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let block = ch[2];
                let open_brace = self
                    .parsed
                    .find_depth_zero_token(
                        inner_slot.first + 2,
                        inner_slot.last,
                        |k| k == TokenKind::LBrace,
                        false,
                    )
                    .unwrap_or(inner_slot.last);
                let where_doc = self.format_where_clauses(
                    clauses,
                    inner_slot.first + 2,
                    open_brace.saturating_sub(1),
                    true,
                );
                let b_doc = self.format_block(
                    block,
                    TokenSlot::new(open_brace, inner_slot.last),
                    None,
                    ContainerKind::ValueBlock,
                );
                (
                    Doc::concat([Doc::str("test "), Doc::text(id_str), where_doc, b_doc]),
                    true,
                )
            }
            NodeKind::TestTemplateInstance => {
                let ch = self.parsed.ast.get_children(inner_node);
                let id_str = self.decode_id(ch[0], inner_slot.first + 1);
                let use_tok = inner_slot.first + 2;
                let tmpl_slot = inner_slot.child_slot_from(self.parsed, use_tok + 1, ch[1]);
                let tmpl_doc = self.format_expr(ch[1], tmpl_slot);
                let b_doc = self.format_block(
                    ch[2],
                    TokenSlot::new(tmpl_slot.last + 1, inner_slot.last),
                    None,
                    ContainerKind::ValueBlock,
                );
                (
                    Doc::concat([
                        Doc::str("test "),
                        Doc::text(id_str),
                        Doc::str(" use "),
                        tmpl_doc,
                        Doc::space(),
                        b_doc,
                    ]),
                    true,
                )
            }
            _ => {
                let (e_doc, eb) = self.format_expr_with_rbrace(inner_node, inner_slot);
                return (Doc::concat([attr_prefix_doc, mod_prefix, e_doc]), eb);
            }
        };

        (
            Doc::concat([attr_prefix_doc, mod_prefix, body_doc]),
            ends_with_rbrace,
        )
    }

    fn unwrap_item_modifiers(
        &mut self,
        mut node: NodeIndex,
        slot: TokenSlot,
    ) -> (Doc, CollectedModifiers, NodeIndex, TokenSlot) {
        let mut attr_docs: Vec<Doc> = Vec::new();
        let mut mods = CollectedModifiers::default();
        let mut cursor = slot.first;

        loop {
            let Some(kind) = self.parsed.ast.get_node_kind(node) else {
                break;
            };
            let ch = self.parsed.ast.get_children(node);
            match kind {
                NodeKind::Attribute => {
                    let attr_expr = ch[0];
                    let target = ch[1];
                    let caret_tok = self
                        .parsed
                        .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Caret, false)
                        .unwrap_or(cursor);
                    if caret_tok > cursor {
                        // Consume any comments preceding `^`.
                        let prev_b = self
                            .parsed
                            .sig_tokens
                            .get(cursor.saturating_sub(1))
                            .map_or(0, |t| t.to);
                        let caret_b = self.parsed.sig_tokens[caret_tok].from;
                        let (lead_c, _, _) = self.comments.take_leading_before(
                            prev_b,
                            caret_b,
                            self.indent_spaces,
                            self.brace_depth,
                            false,
                            &mut self.cfg_stack,
                        );
                        if !matches!(lead_c, Doc::Nil) {
                            attr_docs.push(lead_c);
                        }
                    }
                    let attr_slot = slot.child_slot_from(self.parsed, caret_tok + 1, attr_expr);
                    let line_doc = self.format_attribute_line(attr_expr, attr_slot);
                    let attr_end_byte = self.parsed.sig_tokens[attr_slot.last].to;
                    let attr_line = self.parsed.line_of_byte(attr_end_byte.saturating_sub(1));
                    let next_tok_idx = attr_slot.last + 1;
                    let next_lo = self
                        .parsed
                        .sig_tokens
                        .get(next_tok_idx)
                        .map_or(self.parsed.original_src.len(), |t| t.from);
                    let cfg_now = self.cfg();
                    let trailing_c = self.comments.take_same_line_trailing(
                        attr_line,
                        next_lo,
                        self.indent_spaces,
                        &cfg_now,
                    );
                    attr_docs.push(line_doc);
                    if !matches!(trailing_c, Doc::Nil) {
                        attr_docs.push(trailing_c);
                    }
                    attr_docs.push(Doc::HardLine);
                    // Also emit any standalone comments between this `^attr` and the next token!
                    let (between_c, _, _) = self.comments.take_leading_before(
                        attr_end_byte,
                        next_lo,
                        self.indent_spaces,
                        self.brace_depth,
                        false,
                        &mut self.cfg_stack,
                    );
                    if !matches!(between_c, Doc::Nil) {
                        attr_docs.push(between_c);
                    }
                    cursor = next_tok_idx;
                    node = target;
                }
                NodeKind::Pub => {
                    mods.vis = Some("pub");
                    if self
                        .parsed
                        .sig_tokens
                        .get(cursor)
                        .is_some_and(|t| t.kind == TokenKind::Pub)
                    {
                        cursor += 1;
                    }
                    node = ch[0];
                }
                NodeKind::Private => {
                    mods.vis = Some("private");
                    if self
                        .parsed
                        .sig_tokens
                        .get(cursor)
                        .is_some_and(|t| t.kind == TokenKind::Private)
                    {
                        cursor += 1;
                    }
                    node = ch[0];
                }
                NodeKind::FunctionTemplateDef => {
                    mods.is_template = true;
                    node = ch[0];
                }
                NodeKind::InlineStatement if wraps_callable_def(&self.parsed.ast, ch[0]) => {
                    mods.is_inline = true;
                    node = ch[0];
                }
                NodeKind::AttributeSetTrue => {
                    let kw_name = self.decode_id(ch[0], cursor);
                    match kw_name.as_str() {
                        "__flurry_kw_comptime_only_trait" => {
                            mods.is_pure = true;
                            mods.is_comptime = true;
                        }
                        "__flurry_kw_pure" => mods.is_pure = true,
                        "__flurry_kw_quote" => mods.is_quote = true,
                        "__flurry_kw_comptime" => mods.is_comptime = true,
                        "__flurry_kw_async" => mods.is_async = true,
                        "__flurry_kw_inline" => mods.is_inline = true,
                        "__flurry_kw_unsafe" => mods.is_unsafe = true,
                        "__flurry_kw_spec" => mods.is_spec = true,
                        "__flurry_kw_verified" => mods.is_verified = true,
                        "__flurry_kw_atomic" => mods.is_atomic = true,
                        _ => {}
                    }
                    node = ch[1];
                }
                _ => break,
            }
        }

        // Advance `cursor` past all consumed keyword modifier tokens (`pub`, `private`, `template`,
        // `pure`, `quote`, `comptime`, `async`, `inline`, `unsafe`, `spec`, `verified`, `atomic`)
        // stopping before the core item's own keyword (`fn`, `struct`, `trait`, `effect`, `test`, etc.)!
        let inner_kind = self.parsed.ast.get_node_kind(node);
        while cursor < slot.last {
            let tk = self.parsed.sig_tokens[cursor].kind;
            let is_mod_tok = match tk {
                TokenKind::Pub | TokenKind::Private => true,
                TokenKind::Template if inner_kind != Some(NodeKind::TestTemplateDef) => true,
                TokenKind::Pure
                | TokenKind::KwQuote
                | TokenKind::Comptime
                | TokenKind::Async
                | TokenKind::Inline
                | TokenKind::Unsafe
                | TokenKind::Spec
                | TokenKind::Verified
                | TokenKind::Atomic => {
                    // Don't eat `async`/`unsafe`/`comptime`/`atomic` if the node itself IS a block expr!
                    !matches!(
                        inner_kind,
                        Some(
                            NodeKind::AsyncBlock
                                | NodeKind::UnsafeBlock
                                | NodeKind::ComptimeBlock
                                | NodeKind::AtomicBlock
                                | NodeKind::InlineStatement
                        )
                    )
                }
                _ => false,
            };
            if is_mod_tok {
                cursor += 1;
            } else {
                break;
            }
        }

        if inner_kind == Some(NodeKind::TestTemplateDef)
            && self
                .parsed
                .sig_tokens
                .get(cursor)
                .is_some_and(|t| t.kind == TokenKind::Template)
        {
            cursor += 1;
        }

        (
            Doc::concat(attr_docs),
            mods,
            node,
            TokenSlot::new(cursor, slot.last),
        )
    }

    /// Format a `Function` or `NormalFormHeadDef` (`fn ...` / `predicate ...` / `pure comptime fn Name<...>`).
    fn format_function_def(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let ch = self.parsed.ast.get_children(node);
        let id = ch[0];
        let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let ret_ty = ch[2];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[3]).unwrap_or(&[]);
        let body = ch[4];

        let kw_tok = self.parsed.sig_tokens[slot.first];
        let is_predicate = self
            .parsed
            .bridge
            .predicate_kw_offsets
            .contains(&kw_tok.from);
        let kw_str = if is_predicate { "predicate" } else { "fn" };

        let mut cursor = slot.first + 1;
        let head_prefix = if id != 0 {
            let name = self.decode_id(id, cursor);
            cursor += 1;
            format!("{kw_str} {name}")
        } else {
            kw_str.to_string()
        };

        let is_nf = kind == NodeKind::NormalFormHeadDef;
        let (open_kind, close_kind, style) = if is_nf {
            (TokenKind::Lt, TokenKind::Gt, DelimStyle::Angles)
        } else {
            (TokenKind::LParen, TokenKind::RParen, DelimStyle::Parens)
        };
        let open_delim = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == open_kind, false)
            .unwrap_or(cursor);
        let close_delim =
            find_matching_delim(&self.parsed.sig_tokens, open_delim, open_kind, close_kind)
                .unwrap_or(open_delim + 1);
        let params_doc = self.format_delimited_seq(
            params,
            open_delim,
            close_delim,
            style,
            None,
            |fmt, p, ps| fmt.format_clause_or_param(p, ps),
        );
        cursor = close_delim + 1;

        let ret_doc = if ret_ty != 0 {
            let arrow_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Arrow, false)
                .unwrap_or(cursor);
            let ret_slot = slot.child_slot_from(self.parsed, arrow_tok + 1, ret_ty);
            cursor = ret_slot.last + 1;
            let r = self.format_return_type_node(ret_ty, ret_slot);
            Doc::concat([Doc::str(" -> "), r])
        } else {
            Doc::Nil
        };

        if body == 0 {
            // Bodyless signature inside `trait`: `fn foo(x: i32) -> i32 where ...;`
            let where_doc = self.format_where_clauses(clauses, cursor, slot.last, false);
            return (
                Doc::concat([Doc::text(head_prefix), params_doc, ret_doc, where_doc]),
                false,
            );
        }

        if self.parsed.ast.get_node_kind(body) == Some(NodeKind::Block) {
            let open_brace = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::LBrace, false)
                .unwrap_or(cursor);
            let where_doc =
                self.format_where_clauses(clauses, cursor, open_brace.saturating_sub(1), true);
            let b_doc = self.format_block(
                body,
                TokenSlot::new(open_brace, slot.last),
                None,
                ContainerKind::ValueBlock,
            );
            (
                Doc::concat([
                    Doc::text(head_prefix),
                    params_doc,
                    ret_doc,
                    where_doc,
                    b_doc,
                ]),
                true,
            )
        } else {
            // Expression-bodied function: `fn add(a: i32, b: i32) -> i32 = a + b;` (never convert to `{ ... }`!).
            let eq_tok = if clauses.is_empty() {
                self.parsed
                    .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Eq, false)
                    .unwrap_or(cursor)
            } else {
                // Find the `=` that starts the body after the last clause!
                let last_c = *clauses.last().unwrap();
                let (_, lc_last) = self.parsed.exact_token_bounds(last_c, cursor);
                self.parsed
                    .find_depth_zero_token(lc_last + 1, slot.last, |k| k == TokenKind::Eq, false)
                    .unwrap_or(lc_last + 1)
            };
            let where_doc =
                self.format_where_clauses(clauses, cursor, eq_tok.saturating_sub(1), false);
            let body_slot = slot.child_slot_from(self.parsed, eq_tok + 1, body);
            let (b_doc, ends_b) = self.format_expr_with_rbrace(body, body_slot);
            let tab = self.tab_spaces();
            let doc = Doc::group(Doc::concat([
                Doc::text(head_prefix),
                params_doc,
                ret_doc,
                where_doc,
                Doc::str(" ="),
                Doc::indent(tab, Doc::concat([Doc::Line, b_doc])),
            ]));
            (doc, ends_b)
        }
    }

    pub fn format_return_type_node(&mut self, ret_node: NodeIndex, slot: TokenSlot) -> Doc {
        if self.parsed.ast.get_node_kind(ret_node) == Some(NodeKind::ResultWithId) {
            let ch = self.parsed.ast.get_children(ret_node);
            let id_str = self.decode_id(ch[0], slot.first);
            let colon_tok = self
                .parsed
                .find_depth_zero_token(slot.first + 1, slot.last, |k| k == TokenKind::Colon, false)
                .unwrap_or(slot.first + 1);
            let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
            let ty_doc = self.format_expr(ch[1], ty_slot);
            Doc::concat([Doc::text(id_str), Doc::str(": "), ty_doc])
        } else {
            self.format_expr(ret_node, slot)
        }
    }

    fn format_algebraic_effect(
        &mut self,
        node: NodeIndex,
        slot: TokenSlot,
        is_enum_member: bool,
    ) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let id = ch[0];
        let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let ret_ty = ch[2];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[3]).unwrap_or(&[]);

        let mut cursor = if is_enum_member {
            slot.first
        } else {
            slot.first + 1 // skip `effect`
        };
        let name = self.decode_id(id, cursor);
        cursor += 1;

        let open_paren = cursor;
        let close_paren = find_matching_delim(
            &self.parsed.sig_tokens,
            open_paren,
            TokenKind::LParen,
            TokenKind::RParen,
        )
        .unwrap_or(open_paren + 1);
        let params_doc = self.format_delimited_seq(
            params,
            open_paren,
            close_paren,
            DelimStyle::Parens,
            None,
            |fmt, p, ps| fmt.format_clause_or_param(p, ps),
        );
        cursor = close_paren + 1;

        let ret_doc = if ret_ty != 0 {
            let arrow_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Arrow, false)
                .unwrap_or(cursor);
            let ret_slot = slot.child_slot_from(self.parsed, arrow_tok + 1, ret_ty);
            cursor = ret_slot.last + 1;
            let r = self.format_return_type_node(ret_ty, ret_slot);
            Doc::concat([Doc::str(" -> "), r])
        } else {
            Doc::Nil
        };

        let where_doc = self.format_where_clauses(clauses, cursor, slot.last, false);
        let kw = if is_enum_member { "" } else { "effect " };
        Doc::concat([
            Doc::str(kw),
            Doc::text(name),
            params_doc,
            ret_doc,
            where_doc,
        ])
    }

    fn format_effect_enum_def(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        // `effect enum Name clauses? { ops... }`
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 2);
        let clauses = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let block = ch[2];
        let open_brace = self
            .parsed
            .find_depth_zero_token(slot.first + 3, slot.last, |k| k == TokenKind::LBrace, false)
            .unwrap_or(slot.last);
        let where_doc =
            self.format_where_clauses(clauses, slot.first + 3, open_brace.saturating_sub(1), true);

        let b_ch = self.parsed.ast.get_children(block);
        let ops = b_ch
            .first()
            .and_then(|&h| self.parsed.ast.get_multi_child_slice(h))
            .unwrap_or(&[]);

        // `effect enum` operations are comma-separated and always multi-line when non-empty.
        let ops_doc = if ops.is_empty() {
            let open_end = self.parsed.sig_tokens[open_brace].to;
            let close_start = self.parsed.sig_tokens[slot.last].from;
            let (inner_c, _, _) = self.comments.take_leading_before(
                open_end,
                close_start,
                self.indent_spaces + self.tab_spaces(),
                self.brace_depth + 1,
                false,
                &mut self.cfg_stack,
            );
            if matches!(inner_c, Doc::Nil) {
                Doc::str("{}")
            } else {
                let tab = self.tab_spaces();
                Doc::concat([
                    Doc::str("{"),
                    Doc::indent(
                        tab,
                        Doc::concat([Doc::HardLine, strip_last_hardline(inner_c)]),
                    ),
                    Doc::HardLine,
                    Doc::str("}"),
                ])
            }
        } else {
            self.format_delimited_seq(
                ops,
                open_brace,
                slot.last,
                DelimStyle::SpacedBraces,
                Some(0), // Always expand non-empty `effect enum` body multi-line!
                |fmt, op_node, op_slot| {
                    let (a_doc, m, core_op, core_slot) =
                        fmt.unwrap_item_modifiers(op_node, op_slot);
                    let m_str = m.to_prefix_string();
                    let m_doc = if m_str.is_empty() {
                        Doc::Nil
                    } else {
                        Doc::text(format!("{m_str} "))
                    };
                    let sig = fmt.format_algebraic_effect(core_op, core_slot, true);
                    Doc::concat([a_doc, m_doc, sig])
                },
            )
        };

        Doc::concat([
            Doc::str("effect enum "),
            Doc::text(id_str),
            where_doc,
            ops_doc,
        ])
    }

    fn format_adt_def(&mut self, node: NodeIndex, kind: NodeKind, slot: TokenSlot) -> Doc {
        let kw = match kind {
            NodeKind::StructDef => "struct",
            NodeKind::EnumDef => "enum",
            _ => "union",
        };
        let ch = self.parsed.ast.get_children(node);
        let id = ch[0];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let block = ch[2];

        let mut cursor = slot.first + 1;
        let head = if id != 0 {
            let name = self.decode_id(id, cursor);
            cursor += 1;
            format!("{kw} {name}")
        } else {
            kw.to_string()
        };

        let open_brace = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::LBrace, false)
            .unwrap_or(slot.last);
        let where_doc =
            self.format_where_clauses(clauses, cursor, open_brace.saturating_sub(1), true);
        let b_doc = self.format_block(
            block,
            TokenSlot::new(open_brace, slot.last),
            None,
            ContainerKind::AdtBody,
        );
        Doc::concat([Doc::text(head), where_doc, b_doc])
    }

    fn format_struct_field(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first);
        let colon_tok = self
            .parsed
            .find_depth_zero_token(slot.first + 1, slot.last, |k| k == TokenKind::Colon, false)
            .unwrap_or(slot.first + 1);
        let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
        let ty_doc = self.format_expr(ch[1], ty_slot);
        let default_val = ch[2];
        if default_val != 0 {
            let eq_tok = self
                .parsed
                .find_depth_zero_token(ty_slot.last + 1, slot.last, |k| k == TokenKind::Eq, false)
                .unwrap_or(ty_slot.last + 1);
            let def_slot = slot.child_slot_from(self.parsed, eq_tok + 1, default_val);
            let def_doc = self.format_expr(default_val, def_slot);
            Doc::concat([
                Doc::text(id_str),
                Doc::str(": "),
                ty_doc,
                Doc::str(" = "),
                def_doc,
            ])
        } else {
            Doc::concat([Doc::text(id_str), Doc::str(": "), ty_doc])
        }
    }

    fn format_enum_variant(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first);
        match kind {
            NodeKind::PatternEnumVariant => {
                let colon_tok = self
                    .parsed
                    .find_depth_zero_token(
                        slot.first + 1,
                        slot.last,
                        |k| k == TokenKind::Colon,
                        false,
                    )
                    .unwrap_or(slot.first + 1);
                let pat_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
                let pat_doc = self.format_pattern(ch[1], pat_slot);
                (
                    Doc::concat([Doc::text(id_str), Doc::str(": "), pat_doc]),
                    false,
                )
            }
            NodeKind::ExprEnumVariant => {
                let eq_tok = self
                    .parsed
                    .find_depth_zero_token(slot.first + 1, slot.last, |k| k == TokenKind::Eq, false)
                    .unwrap_or(slot.first + 1);
                let val_slot = slot.child_slot_from(self.parsed, eq_tok + 1, ch[1]);
                let val_doc = self.format_expr(ch[1], val_slot);
                (
                    Doc::concat([Doc::text(id_str), Doc::str(" = "), val_doc]),
                    false,
                )
            }
            NodeKind::TupleEnumVariant => {
                let fields = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let open_paren = slot.first + 1;
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                let tup_doc = self.format_delimited_seq(
                    fields,
                    open_paren,
                    slot.last,
                    DelimStyle::Parens,
                    budget,
                    |fmt, f, fs| fmt.format_expr(f, fs),
                );
                (Doc::concat([Doc::text(id_str), tup_doc]), false)
            }
            NodeKind::StructEnumVariant => {
                let fields = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let open_brace = slot.first + 1;
                let budget = Some(self.cfg().width_heuristics().struct_variant_width);
                let st_doc = self.format_delimited_seq(
                    fields,
                    open_brace,
                    slot.last,
                    DelimStyle::SpacedBraces,
                    budget,
                    |fmt, f, fs| {
                        let (a_doc, m, core_f, core_s) = fmt.unwrap_item_modifiers(f, fs);
                        let m_str = m.to_prefix_string();
                        let m_doc = if m_str.is_empty() {
                            Doc::Nil
                        } else {
                            Doc::text(format!("{m_str} "))
                        };
                        let sf = fmt.format_struct_field(core_f, core_s);
                        Doc::concat([a_doc, m_doc, sf])
                    },
                );
                (Doc::concat([Doc::text(id_str), Doc::space(), st_doc]), true)
            }
            _ => {
                // `SubEnumEnumVariant`: `id.{ variant1, variant2 }`
                let sub_variants = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let open_brace = slot.first + 2; // `id` + `.` + `{`
                let sub_doc = self.format_delimited_seq(
                    sub_variants,
                    open_brace,
                    slot.last,
                    DelimStyle::SpacedBraces,
                    Some(0), // Sub-enum variants expand multi-line when non-empty
                    |fmt, v, vs| {
                        fmt.format_container_member(v, vs, false, ContainerKind::AdtBody)
                            .0
                    },
                );
                (
                    Doc::concat([Doc::text(id_str), Doc::str("."), sub_doc]),
                    true,
                )
            }
        }
    }

    fn format_trait_def(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 1);
        let super_trait = ch[1];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[2]).unwrap_or(&[]);
        let block = ch[3];

        let mut cursor = slot.first + 2;
        let super_doc = if super_trait != 0 {
            let cm_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::ColonMinus, false)
                .unwrap_or(cursor);
            let st_slot = slot.child_slot_from(self.parsed, cm_tok + 1, super_trait);
            cursor = st_slot.last + 1;
            let st = self.format_expr(super_trait, st_slot);
            Doc::concat([Doc::str(" :- "), st])
        } else {
            Doc::Nil
        };

        let open_brace = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::LBrace, false)
            .unwrap_or(slot.last);
        let where_doc =
            self.format_where_clauses(clauses, cursor, open_brace.saturating_sub(1), true);
        let b_doc = self.format_block(
            block,
            TokenSlot::new(open_brace, slot.last),
            None,
            ContainerKind::TraitOrImplBody,
        );
        Doc::concat([
            Doc::str("trait "),
            Doc::text(id_str),
            super_doc,
            where_doc,
            b_doc,
        ])
    }

    fn format_assoc_decl(&mut self, node: NodeIndex, slot: TokenSlot) -> (Doc, bool) {
        // `assoc id(<params>)?: ty (= default)? clauses?`
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 1);
        let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let ty = ch[2];
        let default_expr = ch[3];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[4]).unwrap_or(&[]);

        let mut cursor = slot.first + 2;
        let has_explicit_angles = !params.is_empty()
            || self
                .parsed
                .sig_tokens
                .get(cursor)
                .is_some_and(|t| t.kind == TokenKind::Lt);
        let params_doc = if has_explicit_angles {
            let open_lt = cursor;
            let close_gt = find_matching_delim(
                &self.parsed.sig_tokens,
                open_lt,
                TokenKind::Lt,
                TokenKind::Gt,
            )
            .unwrap_or(open_lt + 1);
            cursor = close_gt + 1;
            self.format_delimited_seq(
                params,
                open_lt,
                close_gt,
                DelimStyle::Angles,
                None,
                |fmt, p, ps| fmt.format_clause_or_param(p, ps),
            )
        } else {
            Doc::Nil
        };

        let ty_doc = if ty != 0 {
            let colon_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Colon, false)
                .unwrap_or(cursor);
            let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ty);
            cursor = ty_slot.last + 1;
            let t = self.format_expr(ty, ty_slot);
            Doc::concat([Doc::str(": "), t])
        } else {
            Doc::Nil
        };

        let mut ends_b = false;
        let default_doc = if default_expr != 0 {
            let eq_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Eq, false)
                .unwrap_or(cursor);
            let def_slot = slot.child_slot_from(self.parsed, eq_tok + 1, default_expr);
            cursor = def_slot.last + 1;
            let (d, eb) = self.format_expr_with_rbrace(default_expr, def_slot);
            ends_b = eb && clauses.is_empty();
            Doc::concat([Doc::str(" = "), d])
        } else {
            Doc::Nil
        };

        let where_doc = self.format_where_clauses(clauses, cursor, slot.last, false);
        (
            Doc::concat([
                Doc::str("assoc "),
                Doc::text(id_str),
                params_doc,
                ty_doc,
                default_doc,
                where_doc,
            ]),
            ends_b,
        )
    }

    fn format_impl_or_extend_inherent(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> Doc {
        let kw = if kind == NodeKind::ImplDef {
            "impl "
        } else {
            "extend "
        };
        let ch = self.parsed.ast.get_children(node);
        let ty = ch[0];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let block = ch[2];

        let ty_slot = slot.child_slot_from(self.parsed, slot.first + 1, ty);
        let ty_doc = self.format_expr(ty, ty_slot);
        let open_brace = self
            .parsed
            .find_depth_zero_token(
                ty_slot.last + 1,
                slot.last,
                |k| k == TokenKind::LBrace,
                false,
            )
            .unwrap_or(slot.last);
        let where_doc = self.format_where_clauses(
            clauses,
            ty_slot.last + 1,
            open_brace.saturating_sub(1),
            true,
        );
        let b_doc = self.format_block(
            block,
            TokenSlot::new(open_brace, slot.last),
            None,
            ContainerKind::TraitOrImplBody,
        );
        Doc::concat([Doc::str(kw), ty_doc, where_doc, b_doc])
    }

    fn format_impl_or_extend_trait(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> Doc {
        let kw = if kind == NodeKind::ImplTraitDef {
            "impl "
        } else {
            "extend "
        };
        let ch = self.parsed.ast.get_children(node);
        let trait_expr = ch[0];
        let ty = ch[1];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[2]).unwrap_or(&[]);
        let block = ch[3];

        let tr_slot = slot.child_slot_from(self.parsed, slot.first + 1, trait_expr);
        let for_tok = self
            .parsed
            .find_depth_zero_token(tr_slot.last + 1, slot.last, |k| k == TokenKind::For, false)
            .unwrap_or(tr_slot.last + 1);
        let ty_slot = slot.child_slot_from(self.parsed, for_tok + 1, ty);
        let open_brace = self
            .parsed
            .find_depth_zero_token(
                ty_slot.last + 1,
                slot.last,
                |k| k == TokenKind::LBrace,
                false,
            )
            .unwrap_or(slot.last);

        let tr_doc = self.format_expr(trait_expr, tr_slot);
        let ty_doc = self.format_expr(ty, ty_slot);
        let where_doc = self.format_where_clauses(
            clauses,
            ty_slot.last + 1,
            open_brace.saturating_sub(1),
            true,
        );
        let b_doc = self.format_block(
            block,
            TokenSlot::new(open_brace, slot.last),
            None,
            ContainerKind::TraitOrImplBody,
        );
        let tab = self.tab_spaces();
        let header = Doc::group(Doc::concat([
            Doc::str(kw),
            tr_doc,
            Doc::indent(tab, Doc::concat([Doc::Line, Doc::str("for "), ty_doc])),
        ]));
        Doc::concat([header, where_doc, b_doc])
    }

    fn format_derive_def(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let trait_expr = ch[0];
        let target_ty = ch[1];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[2]).unwrap_or(&[]);

        let tr_slot = slot.child_slot_from(self.parsed, slot.first + 1, trait_expr);
        let for_tok = self
            .parsed
            .find_depth_zero_token(tr_slot.last + 1, slot.last, |k| k == TokenKind::For, false)
            .unwrap_or(tr_slot.last + 1);
        let ty_slot = slot.child_slot_from(self.parsed, for_tok + 1, target_ty);
        let tr_doc = self.format_expr(trait_expr, tr_slot);
        let ty_doc = self.format_expr(target_ty, ty_slot);
        let where_doc = self.format_where_clauses(clauses, ty_slot.last + 1, slot.last, false);
        Doc::concat([
            Doc::str("derive "),
            tr_doc,
            Doc::str(" for "),
            ty_doc,
            where_doc,
        ])
    }

    fn format_deriver_def(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        // `derive fn id(params)? for trait_expr clauses? block`
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 2);
        let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let trait_expr = ch[2];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[3]).unwrap_or(&[]);
        let block = ch[4];

        let mut cursor = slot.first + 3; // after `derive`, `fn`, `id`
        let has_parens = !params.is_empty()
            || self
                .parsed
                .sig_tokens
                .get(cursor)
                .is_some_and(|t| t.kind == TokenKind::LParen);
        let params_doc = if has_parens {
            let open_p = cursor;
            let close_p = find_matching_delim(
                &self.parsed.sig_tokens,
                open_p,
                TokenKind::LParen,
                TokenKind::RParen,
            )
            .unwrap_or(open_p + 1);
            cursor = close_p + 1;
            self.format_delimited_seq(
                params,
                open_p,
                close_p,
                DelimStyle::Parens,
                None,
                |fmt, p, ps| fmt.format_clause_or_param(p, ps),
            )
        } else {
            Doc::Nil
        };

        let for_tok = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::For, false)
            .unwrap_or(cursor);
        let tr_slot = slot.child_slot_from(self.parsed, for_tok + 1, trait_expr);
        let open_brace = self
            .parsed
            .find_depth_zero_token(
                tr_slot.last + 1,
                slot.last,
                |k| k == TokenKind::LBrace,
                false,
            )
            .unwrap_or(slot.last);
        let tr_doc = self.format_expr(trait_expr, tr_slot);
        let where_doc = self.format_where_clauses(
            clauses,
            tr_slot.last + 1,
            open_brace.saturating_sub(1),
            true,
        );
        let b_doc = self.format_block(
            block,
            TokenSlot::new(open_brace, slot.last),
            None,
            ContainerKind::ValueBlock,
        );
        Doc::concat([
            Doc::str("derive fn "),
            Doc::text(id_str),
            params_doc,
            Doc::str(" for "),
            tr_doc,
            where_doc,
            b_doc,
        ])
    }

    fn format_case_def(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        // `case id(params)? (-> ret)? clauses? block`
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 1);
        let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let ret_ty = ch[2];
        let clauses = self.parsed.ast.get_multi_child_slice(ch[3]).unwrap_or(&[]);
        let block = ch[4];

        let mut cursor = slot.first + 2;
        let has_parens = !params.is_empty()
            || self
                .parsed
                .sig_tokens
                .get(cursor)
                .is_some_and(|t| t.kind == TokenKind::LParen);
        let params_doc = if has_parens {
            let open_p = cursor;
            let close_p = find_matching_delim(
                &self.parsed.sig_tokens,
                open_p,
                TokenKind::LParen,
                TokenKind::RParen,
            )
            .unwrap_or(open_p + 1);
            cursor = close_p + 1;
            self.format_delimited_seq(
                params,
                open_p,
                close_p,
                DelimStyle::Parens,
                None,
                |fmt, p, ps| fmt.format_clause_or_param(p, ps),
            )
        } else {
            Doc::Nil
        };

        let ret_doc = if ret_ty != 0 {
            let arrow_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Arrow, false)
                .unwrap_or(cursor);
            let r_slot = slot.child_slot_from(self.parsed, arrow_tok + 1, ret_ty);
            cursor = r_slot.last + 1;
            let r = self.format_return_type_node(ret_ty, r_slot);
            Doc::concat([Doc::str(" -> "), r])
        } else {
            Doc::Nil
        };

        let open_brace = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::LBrace, false)
            .unwrap_or(slot.last);
        let where_doc =
            self.format_where_clauses(clauses, cursor, open_brace.saturating_sub(1), true);
        let b_doc = self.format_block(
            block,
            TokenSlot::new(open_brace, slot.last),
            None,
            ContainerKind::ValueBlock,
        );
        Doc::concat([
            Doc::str("case "),
            Doc::text(id_str),
            params_doc,
            ret_doc,
            where_doc,
            b_doc,
        ])
    }

    fn format_typealias_or_newtype(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let kw = if kind == NodeKind::TypealiasDef {
            "typealias "
        } else {
            "newtype "
        };
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 1);
        let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let target = ch[2];

        let mut cursor = slot.first + 2;
        let has_angles = !params.is_empty()
            || self
                .parsed
                .sig_tokens
                .get(cursor)
                .is_some_and(|t| t.kind == TokenKind::Lt);
        let params_doc = if has_angles {
            let open_lt = cursor;
            let close_gt = find_matching_delim(
                &self.parsed.sig_tokens,
                open_lt,
                TokenKind::Lt,
                TokenKind::Gt,
            )
            .unwrap_or(open_lt + 1);
            cursor = close_gt + 1;
            self.format_delimited_seq(
                params,
                open_lt,
                close_gt,
                DelimStyle::Angles,
                None,
                |fmt, p, ps| fmt.format_clause_or_param(p, ps),
            )
        } else {
            Doc::Nil
        };

        let eq_tok = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Eq, false)
            .unwrap_or(cursor);
        let t_slot = slot.child_slot_from(self.parsed, eq_tok + 1, target);
        let (t_doc, ends_b) = self.format_expr_with_rbrace(target, t_slot);
        let tab = self.tab_spaces();
        let doc = if ends_b {
            Doc::concat([
                Doc::str(kw),
                Doc::text(id_str),
                params_doc,
                Doc::str(" = "),
                t_doc,
            ])
        } else {
            Doc::group(Doc::concat([
                Doc::str(kw),
                Doc::text(id_str),
                params_doc,
                Doc::str(" ="),
                Doc::indent(tab, Doc::concat([Doc::Line, t_doc])),
            ]))
        };
        (doc, ends_b)
    }

    fn format_const_def(&mut self, node: NodeIndex, slot: TokenSlot) -> (Doc, bool) {
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 1);
        let ty = ch[1];
        let init = ch[2];

        let mut cursor = slot.first + 2;
        let ty_doc = if ty != 0 {
            let colon_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Colon, false)
                .unwrap_or(cursor);
            let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ty);
            cursor = ty_slot.last + 1;
            let t = self.format_expr(ty, ty_slot);
            Doc::concat([Doc::str(": "), t])
        } else {
            Doc::Nil
        };

        let eq_tok = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Eq, false)
            .unwrap_or(cursor);
        let init_slot = slot.child_slot_from(self.parsed, eq_tok + 1, init);
        let (init_doc, ends_b) = self.format_expr_with_rbrace(init, init_slot);
        let tab = self.tab_spaces();
        let doc = if ends_b {
            Doc::concat([
                Doc::str("const "),
                Doc::text(id_str),
                ty_doc,
                Doc::str(" = "),
                init_doc,
            ])
        } else {
            Doc::group(Doc::concat([
                Doc::str("const "),
                Doc::text(id_str),
                ty_doc,
                Doc::str(" ="),
                Doc::indent(tab, Doc::concat([Doc::Line, init_doc])),
            ]))
        };
        (doc, ends_b)
    }

    fn format_static_def(&mut self, node: NodeIndex, slot: TokenSlot) -> (Doc, bool) {
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first + 1);
        let ty = ch[1];
        let init = ch[2];

        let mut cursor = slot.first + 2;
        let colon_tok = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Colon, false)
            .unwrap_or(cursor);
        let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ty);
        cursor = ty_slot.last + 1;
        let ty_doc = self.format_expr(ty, ty_slot);

        if init == 0 {
            let doc = Doc::concat([
                Doc::str("static "),
                Doc::text(id_str),
                Doc::str(": "),
                ty_doc,
            ]);
            return (doc, false);
        }

        let eq_tok = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Eq, false)
            .unwrap_or(cursor);
        let init_slot = slot.child_slot_from(self.parsed, eq_tok + 1, init);
        let (init_doc, ends_b) = self.format_expr_with_rbrace(init, init_slot);
        let tab = self.tab_spaces();
        let doc = if ends_b {
            Doc::concat([
                Doc::str("static "),
                Doc::text(id_str),
                Doc::str(": "),
                ty_doc,
                Doc::str(" = "),
                init_doc,
            ])
        } else {
            Doc::group(Doc::concat([
                Doc::str("static "),
                Doc::text(id_str),
                Doc::str(": "),
                ty_doc,
                Doc::str(" ="),
                Doc::indent(tab, Doc::concat([Doc::Line, init_doc])),
            ]))
        };
        (doc, ends_b)
    }

    /// Format a parameter, type-parameter, or `where` clause item.
    pub fn format_clause_or_param(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let Some(kind) = self.parsed.ast.get_node_kind(node) else {
            return Doc::Nil;
        };
        let ch = self.parsed.ast.get_children(node);
        match kind {
            NodeKind::SelfParam => Doc::str("self"),
            NodeKind::SelfRefParam => Doc::str("*self"),
            NodeKind::ItselfParam => Doc::str("itself"),
            NodeKind::ItselfRefParam => Doc::str("*itself"),
            NodeKind::TypeDeclClause => Doc::text(self.decode_id(ch[0], slot.first)),
            NodeKind::TypeBoundParam | NodeKind::TypeBoundDeclClause => {
                let id_str = self.decode_id(ch[0], slot.first);
                let colon_tok = self
                    .parsed
                    .find_depth_zero_token(
                        slot.first + 1,
                        slot.last,
                        |k| k == TokenKind::Colon,
                        false,
                    )
                    .unwrap_or(slot.first + 1);
                let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
                let ty_doc = self.format_expr(ch[1], ty_slot);
                Doc::concat([Doc::text(id_str), Doc::str(": "), ty_doc])
            }
            NodeKind::TraitBoundParam | NodeKind::TraitBoundDeclClause => {
                let id_str = self.decode_id(ch[0], slot.first);
                let cm_tok = self
                    .parsed
                    .find_depth_zero_token(
                        slot.first + 1,
                        slot.last,
                        |k| k == TokenKind::ColonMinus,
                        false,
                    )
                    .unwrap_or(slot.first + 1);
                let cm_byte = self.parsed.sig_tokens.get(cm_tok).map_or(0, |t| t.from);
                let op_str = if self.parsed.bridge.subtype_offsets.contains(&cm_byte) {
                    " <: "
                } else {
                    " :- "
                };
                let tr_slot = slot.child_slot_from(self.parsed, cm_tok + 1, ch[1]);
                let tr_doc = self.format_expr(ch[1], tr_slot);
                Doc::concat([Doc::text(id_str), Doc::str(op_str), tr_doc])
            }
            NodeKind::OptionalParam | NodeKind::OptionalDeclClause => {
                // `.id: ty = default` (`slot.first` is `.`)
                let id_str = self.decode_id(ch[0], slot.first + 1);
                let colon_tok = self
                    .parsed
                    .find_depth_zero_token(
                        slot.first + 2,
                        slot.last,
                        |k| k == TokenKind::Colon,
                        false,
                    )
                    .unwrap_or(slot.first + 2);
                let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
                let eq_tok = self
                    .parsed
                    .find_depth_zero_token(
                        ty_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Eq,
                        false,
                    )
                    .unwrap_or(ty_slot.last + 1);
                let def_slot = slot.child_slot_from(self.parsed, eq_tok + 1, ch[2]);
                let ty_doc = self.format_expr(ch[1], ty_slot);
                let def_doc = self.format_expr(ch[2], def_slot);
                Doc::concat([
                    Doc::str("."),
                    Doc::text(id_str),
                    Doc::str(": "),
                    ty_doc,
                    Doc::str(" = "),
                    def_doc,
                ])
            }
            NodeKind::OptionalTypeParam => {
                // `.decl_clause` inside a `FnType`
                let inner_slot = TokenSlot::new(slot.first + 1, slot.last);
                let inner_doc = self.format_clause_or_param(ch[0], inner_slot);
                Doc::concat([Doc::str("."), inner_doc])
            }
            NodeKind::VarargParam | NodeKind::VarargDeclClause => {
                // `...id: ty`
                let id_str = self.decode_id(ch[0], slot.first + 3);
                let colon_tok = self
                    .parsed
                    .find_depth_zero_token(
                        slot.first + 4,
                        slot.last,
                        |k| k == TokenKind::Colon,
                        false,
                    )
                    .unwrap_or(slot.first + 4);
                let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
                let ty_doc = self.format_expr(ch[1], ty_slot);
                Doc::concat([Doc::str("..."), Doc::text(id_str), Doc::str(": "), ty_doc])
            }
            NodeKind::ComptimeParam => {
                let sub =
                    self.format_clause_or_param(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("comptime "), sub])
            }
            NodeKind::ErrorParam => {
                let sub =
                    self.format_clause_or_param(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("error "), sub])
            }
            NodeKind::CatchParam => {
                let sub =
                    self.format_clause_or_param(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("catch "), sub])
            }
            NodeKind::LambdaParam => {
                let sub =
                    self.format_clause_or_param(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("lambda "), sub])
            }
            NodeKind::ImplicitParam => {
                let sub =
                    self.format_clause_or_param(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("implicit "), sub])
            }
            NodeKind::QuoteParam | NodeKind::QuoteDeclClause => {
                let sub =
                    self.format_clause_or_param(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("quote "), sub])
            }
            NodeKind::CaseParam => {
                let pat_doc = self.format_pattern(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("case "), pat_doc])
            }
            NodeKind::AssocParam => {
                let sub =
                    self.format_clause_or_param(ch[0], TokenSlot::new(slot.first + 1, slot.last));
                Doc::concat([Doc::str("assoc "), sub])
            }
            NodeKind::AttrParam => {
                let attr_expr = ch[0];
                let inner_param = ch[1];
                let attr_slot = slot.child_slot_from(self.parsed, slot.first + 1, attr_expr);
                let attr_doc = self.format_attribute_line(attr_expr, attr_slot);
                let p_slot = TokenSlot::new(attr_slot.last + 1, slot.last);
                let p_doc = self.format_clause_or_param(inner_param, p_slot);
                // Inline `^attr x: T` on single-line param lists; line break when enclosing param list breaks!
                Doc::concat([attr_doc, Doc::Line, p_doc])
            }
            NodeKind::Requires
            | NodeKind::Ensures
            | NodeKind::Decreases
            | NodeKind::Invariant
            | NodeKind::Asserts
            | NodeKind::Assumes
            | NodeKind::Axiom => self.format_verification_stmt_or_clause(node, kind, slot).0,
            _ => self.format_expr(node, slot),
        }
    }

    /// Format optional `where ...` clauses on an item signature.
    /// When `before_open_brace` is true, includes the space / newline before `{`!
    pub fn format_where_clauses(
        &mut self,
        clauses: &[NodeIndex],
        search_from_tok: usize,
        search_to_tok: usize,
        before_open_brace: bool,
    ) -> Doc {
        if clauses.is_empty() {
            return if before_open_brace {
                Doc::space()
            } else {
                Doc::Nil
            };
        }

        let where_tok = self
            .parsed
            .find_depth_zero_token(
                search_from_tok,
                search_to_tok.max(search_from_tok),
                |k| k == TokenKind::Where,
                false,
            )
            .unwrap_or(search_from_tok);
        let where_end_byte = self.parsed.sig_tokens.get(where_tok).map_or(0, |t| t.to);
        let limit_byte = self
            .parsed
            .sig_tokens
            .get(search_to_tok + 1)
            .map_or(self.parsed.original_src.len(), |t| t.from);

        let has_contract = clauses.iter().any(|&c| {
            matches!(
                self.parsed.ast.get_node_kind(c),
                Some(
                    NodeKind::Requires
                        | NodeKind::Ensures
                        | NodeKind::Decreases
                        | NodeKind::Outcomes
                        | NodeKind::Invariant
                )
            )
        });
        let trait_bound_count = clauses
            .iter()
            .filter(|&&c| {
                matches!(
                    self.parsed.ast.get_node_kind(c),
                    Some(NodeKind::TraitBoundDeclClause | NodeKind::TraitBoundParam)
                )
            })
            .count();
        let has_comments = self
            .comments
            .peek()
            .is_some_and(|(_, cm)| cm.byte_start < limit_byte);
        let force_multiline = has_contract || trait_bound_count >= 2 || has_comments;

        let tab = self.tab_spaces();
        let saved_indent = self.indent_spaces;
        self.indent_spaces += tab;

        // Precompute clause slots monotonically.
        let mut c_slots = Vec::with_capacity(clauses.len());
        let mut cursor = where_tok + 1;
        let bound = TokenSlot::new(cursor, search_to_tok.max(cursor));
        for &c in clauses {
            let cs = bound.child_slot_from(self.parsed, cursor, c);
            let lo = self.parsed.sig_tokens[cs.first].from;
            let hi = self.parsed.sig_tokens[cs.last].to;
            let mut sep_end = hi;
            let mut next_t = cs.last + 1;
            if next_t <= search_to_tok && self.parsed.sig_tokens[next_t].kind == TokenKind::Comma {
                sep_end = self.parsed.sig_tokens[next_t].to;
                next_t += 1;
            }
            let end_line = self.parsed.line_of_byte(sep_end.saturating_sub(1));
            c_slots.push((c, cs, lo, hi, sep_end, end_line));
            cursor = next_t;
        }

        let mut item_docs = Vec::with_capacity(c_slots.len() * 3);
        let mut prev_end = where_end_byte;
        let sep_line = if force_multiline {
            Doc::HardLine
        } else {
            Doc::Line
        };

        for (i, &(c_node, c_slot, lo, hi, sep_end, end_line)) in c_slots.iter().enumerate() {
            let (lead, _, dir_flags) = self.comments.take_leading_before(
                prev_end,
                lo,
                self.indent_spaces,
                self.brace_depth,
                i > 0,
                &mut self.cfg_stack,
            );
            if i > 0 {
                item_docs.push(sep_line.clone());
            }
            if !matches!(lead, Doc::Nil) {
                item_docs.push(lead);
            }
            let rendered = if dir_flags.skip {
                self.format_skipped_bytes(lo, hi)
            } else if dir_flags.preserve {
                self.format_preserved_bytes(lo, hi)
            } else {
                self.format_clause_or_param(c_node, c_slot)
            };
            let is_last = i + 1 == c_slots.len();
            let comma = if !is_last {
                Doc::str(",")
            } else if before_open_brace {
                if force_multiline {
                    Doc::str(",")
                } else {
                    Doc::trailing_comma()
                }
            } else {
                Doc::Nil
            };

            let next_limit = c_slots.get(i + 1).map_or(limit_byte, |n| n.2);
            let cfg_now = self.cfg();
            let trailing = self.comments.take_same_line_trailing(
                end_line,
                next_limit,
                self.indent_spaces,
                &cfg_now,
            );
            item_docs.push(rendered);
            item_docs.push(comma);
            if !matches!(trailing, Doc::Nil) {
                item_docs.push(trailing);
            }
            prev_end = sep_end;
        }

        self.indent_spaces = saved_indent;

        if force_multiline {
            Doc::concat([
                Doc::HardLine,
                Doc::str("where"),
                Doc::indent(tab, Doc::concat([Doc::HardLine, Doc::concat(item_docs)])),
                if before_open_brace {
                    Doc::HardLine
                } else {
                    Doc::Nil
                },
            ])
        } else {
            Doc::group(Doc::concat([
                Doc::Line,
                Doc::str("where"),
                Doc::indent(tab, Doc::concat([Doc::Line, Doc::concat(item_docs)])),
                if before_open_brace {
                    Doc::Line
                } else {
                    Doc::Nil
                },
            ]))
        }
    }

    /// Format a `pub`/`private` `UseStatement` (or bridged `@use`) and return
    /// `(full_doc, ends_with_rbrace, canonical_sort_key)`.
    pub fn format_use_item_with_key(
        &mut self,
        node: NodeIndex,
        slot: TokenSlot,
    ) -> (Doc, bool, String) {
        let (_, mods, inner_node, inner_slot) = self.unwrap_item_modifiers(node, slot);
        let mod_str = mods.to_prefix_string();
        let mod_prefix = if mod_str.is_empty() {
            Doc::Nil
        } else {
            Doc::text(format!("{mod_str} "))
        };
        let (u_doc, eb, key) = self.format_use_core(inner_node, inner_slot);
        (Doc::concat([mod_prefix, u_doc]), eb, key)
    }

    fn format_use_core(&mut self, use_node: NodeIndex, slot: TokenSlot) -> (Doc, bool, String) {
        let kw_byte = self.parsed.sig_tokens.get(slot.first).map_or(0, |t| t.from);
        let is_at_use = self.parsed.bridge.at_use_offsets.contains(&kw_byte);
        let kw = if is_at_use { "@use " } else { "use " };

        let ch = self.parsed.ast.get_children(use_node);
        let path_node = ch[0];
        let path_slot = TokenSlot::new(slot.first + 1, slot.last);
        let reorder = self.cfg_stack.current().reorder_imports;
        let (path_doc, ends_b, sort_key) = self.format_use_path(path_node, path_slot, reorder);
        (Doc::concat([Doc::str(kw), path_doc]), ends_b, sort_key)
    }

    fn format_use_path(
        &mut self,
        path_node: NodeIndex,
        slot: TokenSlot,
        reorder: bool,
    ) -> (Doc, bool, String) {
        let Some(kind) = self.parsed.ast.get_node_kind(path_node) else {
            return (Doc::Nil, false, String::new());
        };
        let ch = self.parsed.ast.get_children(path_node);
        match kind {
            NodeKind::Id => {
                let s = self.decode_id(path_node, slot.first);
                (Doc::text(s.clone()), false, s)
            }
            NodeKind::SelfLower => (Doc::str("self"), false, "self".to_string()),
            NodeKind::SelfCap => (Doc::str("Self"), false, "Self".to_string()),
            NodeKind::SuperPath => {
                let sub_slot = TokenSlot::new(slot.first + 1, slot.last);
                let (sub_doc, eb, sub_key) = self.format_use_path(ch[0], sub_slot, reorder);
                (
                    Doc::concat([Doc::str("."), sub_doc]),
                    eb,
                    format!(".{sub_key}"),
                )
            }
            NodeKind::PackagePath => {
                let sub_slot = TokenSlot::new(slot.first + 1, slot.last);
                let (sub_doc, eb, sub_key) = self.format_use_path(ch[0], sub_slot, reorder);
                (
                    Doc::concat([Doc::str("@"), sub_doc]),
                    eb,
                    format!("@{sub_key}"),
                )
            }
            NodeKind::ProjectionPath => {
                let base_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let (base_doc, _, base_key) = self.format_use_path(ch[0], base_slot, reorder);
                let member = self.decode_id(ch[1], base_slot.last + 2);
                (
                    Doc::concat([base_doc, Doc::str("."), Doc::text(member.clone())]),
                    false,
                    format!("{base_key}.{member}"),
                )
            }
            NodeKind::ProjectionAllPath => {
                let base_slot =
                    TokenSlot::new(slot.first, slot.last.saturating_sub(2).max(slot.first));
                let (base_doc, _, base_key) = self.format_use_path(ch[0], base_slot, reorder);
                (
                    Doc::concat([base_doc, Doc::str(".*")]),
                    false,
                    format!("{base_key}.*"),
                )
            }
            NodeKind::PathAsBind => {
                let base_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let (base_doc, _, base_key) = self.format_use_path(ch[0], base_slot, reorder);
                let alias = self.decode_id(ch[1], base_slot.last + 2);
                (
                    Doc::concat([base_doc, Doc::str(" as "), Doc::text(alias.clone())]),
                    false,
                    format!("{base_key} as {alias}"),
                )
            }
            NodeKind::ProjectionMultiPath => {
                // `base.{a, b, c}` — NO spaces inside `{...}` when single-line!
                let base_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let (base_doc, _, base_key) = self.format_use_path(ch[0], base_slot, reorder);
                let open_brace = self
                    .parsed
                    .find_depth_zero_token(
                        base_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::LBrace,
                        false,
                    )
                    .unwrap_or(base_slot.last + 2);
                let sub_paths = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);

                let open_end = self.parsed.sig_tokens[open_brace].to;
                let close_start = self.parsed.sig_tokens[slot.last].from;
                let has_inner_comments = self
                    .comments
                    .peek()
                    .is_some_and(|(_, c)| c.byte_start >= open_end && c.byte_start < close_start);

                if !reorder || has_inner_comments || sub_paths.len() <= 1 {
                    let mut child_keys = Vec::new();
                    let list_doc = self.format_delimited_seq(
                        sub_paths,
                        open_brace,
                        slot.last,
                        DelimStyle::CompactBraces,
                        None,
                        |fmt, sp, sps| {
                            let (d, _, k) = fmt.format_use_path(sp, sps, reorder);
                            child_keys.push(k);
                            d
                        },
                    );
                    let key = format!("{base_key}.{{{}}}", child_keys.join(", "));
                    return (Doc::concat([base_doc, Doc::str("."), list_doc]), true, key);
                }

                // Sort items inside `use a.{c, b}` lexicographically when comment-free!
                let mut cursor = open_brace + 1;
                let bound = TokenSlot::new(cursor, slot.last.saturating_sub(1).max(cursor));
                let mut rendered_subs: Vec<(String, Doc)> = Vec::with_capacity(sub_paths.len());
                for &sp in sub_paths {
                    let sps = bound.child_slot_from(self.parsed, cursor, sp);
                    let (d, _, k) = self.format_use_path(sp, sps, reorder);
                    rendered_subs.push((k, d));
                    cursor = sps.last + 1;
                    if cursor < slot.last && self.parsed.sig_tokens[cursor].kind == TokenKind::Comma
                    {
                        cursor += 1;
                    }
                }
                rendered_subs.sort_by(|a, b| compare_import_keys(&a.0, &b.0));

                let sorted_keys: Vec<String> = rendered_subs.iter().map(|x| x.0.clone()).collect();
                let sorted_docs: Vec<Doc> = rendered_subs.into_iter().map(|x| x.1).collect();
                let tab = self.tab_spaces();
                let list_doc = Doc::group(Doc::concat([
                    Doc::str("{"),
                    Doc::indent(
                        tab,
                        Doc::concat([
                            Doc::SoftLine,
                            Doc::join(sorted_docs, Doc::concat([Doc::str(","), Doc::Line])),
                            Doc::trailing_comma(),
                        ]),
                    ),
                    Doc::SoftLine,
                    Doc::str("}"),
                ]));
                let key = format!("{base_key}.{{{}}}", sorted_keys.join(", "));
                (Doc::concat([base_doc, Doc::str("."), list_doc]), true, key)
            }
            _ => {
                let s = self.literal_slice(path_node, slot.first).to_string();
                (Doc::text(s.clone()), false, s)
            }
        }
    }
}

/// Returns `true` when `node` unwraps (through `Pub` / `Private` / `AttributeSetTrue` / `FunctionTemplateDef`
/// / `InlineStatement`) to a callable item definition (`Function`, `NormalFormHeadDef`, or `AlgebraicEffect`),
/// where `inline` was parsed as `NodeKind::InlineStatement` because it preceded `pub` or `pure`.
pub(crate) fn wraps_callable_def(ast: &ast::Ast, mut node: NodeIndex) -> bool {
    loop {
        let Some(k) = ast.get_node_kind(node) else {
            return false;
        };
        let ch = ast.get_children(node);
        match k {
            NodeKind::Pub
            | NodeKind::Private
            | NodeKind::FunctionTemplateDef
            | NodeKind::InlineStatement => {
                let Some(&next) = ch.first() else {
                    return false;
                };
                node = next;
            }
            NodeKind::AttributeSetTrue | NodeKind::Attribute => {
                let Some(&next) = ch.get(1) else {
                    return false;
                };
                node = next;
            }
            NodeKind::Function | NodeKind::NormalFormHeadDef | NodeKind::AlgebraicEffect => {
                return true;
            }
            _ => return false,
        }
    }
}
