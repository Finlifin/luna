//! Expression, control-flow statement, block, closure, type-expression, and postfix-chain formatting.

use ast::{
    FN_MOD_ATOMIC, FN_MOD_COMPTIME, FN_MOD_INLINE, FN_MOD_NORMAL_FORM, FN_MOD_PURE, FN_MOD_SPEC,
    FN_MOD_UNSAFE, FN_MOD_VERIFIED, NodeIndex, NodeKind,
};
use lex::TokenKind;

use crate::doc::Doc;
use crate::formatter::{ContainerKind, ExprPatternMode, Formatter, TokenSlot};
use crate::patterns::{DelimStyle, find_matching_delim, strip_last_hardline};

impl<'a> Formatter<'a> {
    /// Format an expression or statement node, returning `(doc, ends_with_rbrace)`.
    pub fn format_expr_with_rbrace(&mut self, node: NodeIndex, slot: TokenSlot) -> (Doc, bool) {
        if node == 0 {
            return (Doc::Nil, false);
        }
        if self.parsed.ast.get_node_kind(node) == Some(NodeKind::ExprStatement) {
            let inner = self.parsed.ast.get_children(node)[0];
            let inner_slot = slot.child_slot_from(self.parsed, slot.first, inner);
            return self.format_expr_with_rbrace(inner, inner_slot);
        }
        let (paren_count, inner_slot) = slot.unwrap_parens(self.parsed, node);
        if paren_count > 0 {
            let (inner_doc, _) = self.format_expr_unwrapped(node, inner_slot);
            let open = "(".repeat(paren_count);
            let close = ")".repeat(paren_count);
            return (
                Doc::concat([Doc::text(open), inner_doc, Doc::text(close)]),
                false,
            );
        }
        self.format_expr_unwrapped(node, inner_slot)
    }

    /// Format an expression node.
    pub fn format_expr(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        self.format_expr_with_rbrace(node, slot).0
    }

    /// Format `(args...)` of an `Application` node (used by `@path(args...)` macro formatting).
    pub fn format_call_args_only(&mut self, call_node: NodeIndex, call_slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(call_node);
        let callee = ch[0];
        let args = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
        let callee_slot = call_slot.child_slot_from(self.parsed, call_slot.first, callee);
        let open_paren = callee_slot.last + 1;
        let budget = Some(self.cfg().width_heuristics().fn_call_width);
        self.format_delimited_seq(
            args,
            open_paren,
            call_slot.last,
            DelimStyle::Parens,
            budget,
            |fmt, arg, arg_slot| fmt.format_call_arg(arg, arg_slot),
        )
    }

    pub fn format_call_arg(&mut self, arg: NodeIndex, slot: TokenSlot) -> Doc {
        match self.parsed.ast.get_node_kind(arg) {
            Some(NodeKind::OptionalArg) => {
                let ch = self.parsed.ast.get_children(arg);
                // `.id = expr` (`slot.first` is `.`)
                let id_str = self.decode_id(ch[0], slot.first + 1);
                let eq_tok = self
                    .parsed
                    .find_depth_zero_token(slot.first + 2, slot.last, |k| k == TokenKind::Eq, false)
                    .unwrap_or(slot.first + 2);
                let val_slot = slot.child_slot_from(self.parsed, eq_tok + 1, ch[1]);
                let val_doc = self.format_expr(ch[1], val_slot);
                Doc::concat([Doc::str("."), Doc::text(id_str), Doc::str(" = "), val_doc])
            }
            Some(NodeKind::ExpandArg) => {
                let ch = self.parsed.ast.get_children(arg);
                let val_slot = slot.child_slot_from(self.parsed, slot.first + 3, ch[0]);
                let val_doc = self.format_expr(ch[0], val_slot);
                Doc::concat([Doc::str("..."), val_doc])
            }
            Some(NodeKind::Property) => self.format_property(arg, slot),
            _ => self.format_expr(arg, slot),
        }
    }

    pub fn format_property(&mut self, prop: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(prop);
        let id_str = self.decode_id(ch[0], slot.first);
        let colon_tok = self
            .parsed
            .find_depth_zero_token(slot.first + 1, slot.last, |k| k == TokenKind::Colon, false)
            .unwrap_or(slot.first + 1);
        let val_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
        let val_doc = self.format_expr(ch[1], val_slot);
        Doc::concat([Doc::text(id_str), Doc::str(": "), val_doc])
    }

    /// Format a `{ ... }` `Block` node.
    /// When `single_line_budget` is `Some(budget)` and the block has at most 1 simple element
    /// without comments/newlines, tries `{ elem }` within `budget`; otherwise expands multi-line.
    /// Empty blocks without comments always render `{}`.
    pub fn format_block(
        &mut self,
        block_node: NodeIndex,
        block_slot: TokenSlot,
        single_line_budget: Option<usize>,
        container_kind: ContainerKind,
    ) -> Doc {
        self.format_block_with_policy(
            block_node,
            block_slot,
            single_line_budget,
            false,
            container_kind,
        )
    }

    pub fn format_block_with_policy(
        &mut self,
        block_node: NodeIndex,
        block_slot: TokenSlot,
        single_line_budget: Option<usize>,
        allow_stmt_in_single_line: bool,
        container_kind: ContainerKind,
    ) -> Doc {
        let ch = self.parsed.ast.get_children(block_node);
        let elems = ch
            .first()
            .and_then(|&h| self.parsed.ast.get_multi_child_slice(h))
            .unwrap_or(&[]);

        let open_tok = block_slot.first;
        let close_tok = block_slot.last;
        let open_end_byte = self.parsed.sig_tokens.get(open_tok).map_or(0, |t| t.to);
        let close_start_byte = self
            .parsed
            .sig_tokens
            .get(close_tok)
            .map_or(self.parsed.original_src.len(), |t| t.from);

        let has_comments_in_block = self
            .comments
            .peek()
            .is_some_and(|(_, c)| c.byte_start < close_start_byte);

        if elems.is_empty() && !has_comments_in_block {
            return Doc::str("{}");
        }

        let tab = self.tab_spaces();

        // Check single-line eligibility for short sub-expression blocks (`if a { x } else { y }`,
        // `|x| { x + 1 }`, `let p = x else { return 0; }`, `do { x }`).
        if let Some(budget) = single_line_budget {
            if budget > 0
                && elems.len() == 1
                && !has_comments_in_block
                && can_single_line_block_elem(&self.parsed.ast, elems[0], allow_stmt_in_single_line)
            {
                let saved_indent = self.indent_spaces;
                self.indent_spaces += tab;
                self.brace_depth += 1;

                let inner_bound =
                    TokenSlot::new(open_tok + 1, close_tok.saturating_sub(1).max(open_tok + 1));
                let elem_slot = inner_bound.child_slot_from(self.parsed, open_tok + 1, elems[0]);
                let (elem_doc, ends_rbrace) =
                    self.format_container_member(elems[0], elem_slot, true, container_kind);
                let direct_kind = self.parsed.ast.get_node_kind(elems[0]);
                let semi = if direct_kind == Some(NodeKind::ExprStatement) {
                    Doc::str(";")
                } else if !ends_rbrace && requires_semi_in_single_line_block(direct_kind) {
                    Doc::str(";")
                } else {
                    Doc::Nil
                };

                self.brace_depth -= 1;
                self.cfg_stack.pop_above(self.brace_depth);
                self.indent_spaces = saved_indent;

                return Doc::group_with_budget(
                    budget,
                    Doc::concat([
                        Doc::str("{"),
                        Doc::indent(tab, Doc::concat([Doc::Line, elem_doc, semi])),
                        Doc::Line,
                        Doc::str("}"),
                    ]),
                );
            }
        }

        let saved_indent = self.indent_spaces;
        self.indent_spaces += tab;
        self.brace_depth += 1;

        let inner_doc = self.format_container_elements(
            elems,
            open_tok + 1,
            close_tok.saturating_sub(1).max(open_tok + 1),
            open_end_byte,
            close_start_byte,
            container_kind,
        );

        self.brace_depth -= 1;
        self.cfg_stack.pop_above(self.brace_depth);
        self.indent_spaces = saved_indent;

        if matches!(inner_doc, Doc::Nil) {
            Doc::str("{}")
        } else {
            Doc::concat([
                Doc::str("{"),
                Doc::indent(tab, Doc::concat([Doc::HardLine, inner_doc])),
                Doc::HardLine,
                Doc::str("}"),
            ])
        }
    }

    fn format_expr_unwrapped(&mut self, node: NodeIndex, slot: TokenSlot) -> (Doc, bool) {
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

        let Some(kind) = self.parsed.ast.get_node_kind(node) else {
            return (Doc::Nil, false);
        };
        let ch = self.parsed.ast.get_children(node);

        match kind {
            // Leaves & literals
            NodeKind::Id => (Doc::text(self.decode_id(node, slot.first)), false),
            NodeKind::Null => (Doc::str("null"), false),
            NodeKind::Undefined => (Doc::str("undefined"), false),
            NodeKind::SelfLower => (Doc::str("self"), false),
            NodeKind::SelfCap => (Doc::str("Self"), false),
            NodeKind::Wildcard => (Doc::str("_"), false),
            NodeKind::Int | NodeKind::Real | NodeKind::Str | NodeKind::Char | NodeKind::Bool => {
                (Doc::text(self.literal_slice(node, slot.first)), false)
            }
            NodeKind::StringConcat => (self.format_string_concat(node, slot), false),
            NodeKind::LiteralExtension => {
                let left_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let left_doc = self.format_expr(ch[0], left_slot);
                let suffix = self.decode_id(ch[1], left_slot.last + 1);
                (Doc::concat([left_doc, Doc::text(suffix)]), false)
            }
            NodeKind::Unit => (Doc::str("()"), false),
            NodeKind::RangeFull => (Doc::str(".."), false),
            NodeKind::Symbol => {
                let id_str = self.decode_id(ch[0], slot.first);
                (Doc::concat([Doc::str("."), Doc::text(id_str)]), false)
            }

            // Tuple, List, Object
            NodeKind::Tuple => {
                let elems = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
                let style = if elems.len() == 1 {
                    DelimStyle::SingleTupleParens
                } else {
                    DelimStyle::Parens
                };
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                let doc = self.format_delimited_seq(
                    elems,
                    slot.first,
                    slot.last,
                    style,
                    budget,
                    |fmt, el, el_slot| fmt.format_expr(el, el_slot),
                );
                (doc, false)
            }
            NodeKind::ListOf => {
                let elems = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
                let budget = Some(self.cfg().width_heuristics().array_width);
                let doc = self.format_delimited_seq(
                    elems,
                    slot.first,
                    slot.last,
                    DelimStyle::Brackets,
                    budget,
                    |fmt, el, el_slot| fmt.format_call_arg(el, el_slot),
                );
                (doc, false)
            }
            NodeKind::Object => {
                let props = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
                let budget = Some(self.cfg().width_heuristics().struct_lit_width);
                let doc = self.format_delimited_seq(
                    props,
                    slot.first,
                    slot.last,
                    DelimStyle::SpacedBraces,
                    budget,
                    |fmt, prop, prop_slot| fmt.format_call_arg(prop, prop_slot),
                );
                (doc, true)
            }

            // Postfix expressions & method/field chains
            NodeKind::Projection
            | NodeKind::Deref
            | NodeKind::Refer
            | NodeKind::Await
            | NodeKind::TypeCast
            | NodeKind::DynCast
            | NodeKind::HandlerApplication
            | NodeKind::Application
            | NodeKind::NormalFormHeadApplication
            | NodeKind::IndexApplication
            | NodeKind::ExtendedApplication
            | NodeKind::TakeView
            | NodeKind::OptionPropagation
            | NodeKind::ErrorPropagation
            | NodeKind::EffectPropagation
            | NodeKind::OptionElimination
            | NodeKind::ErrorElimination
            | NodeKind::EffectElimination
            | NodeKind::PostMatch
            | NodeKind::PostLambda => self.format_postfix_or_chain(node, slot),

            // Ranges
            NodeKind::RangeFrom => {
                let lo_slot =
                    TokenSlot::new(slot.first, slot.last.saturating_sub(2).max(slot.first));
                let lo_doc = self.format_expr(ch[0], lo_slot);
                (Doc::concat([lo_doc, Doc::str("..")]), false)
            }
            NodeKind::RangeTo => {
                let hi_slot = slot.child_slot_from(self.parsed, slot.first + 2, ch[0]);
                let sep = if self.slot_starts_with_minus(hi_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let hi_doc = self.format_expr(ch[0], hi_slot);
                (Doc::concat([Doc::str(".."), sep, hi_doc]), false)
            }
            NodeKind::RangeToInclusive => {
                let hi_slot = slot.child_slot_from(self.parsed, slot.first + 3, ch[0]);
                let sep = if self.slot_starts_with_minus(hi_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let hi_doc = self.format_expr(ch[0], hi_slot);
                (Doc::concat([Doc::str("..="), sep, hi_doc]), false)
            }
            NodeKind::RangeFromTo | NodeKind::RangeFromToInclusive => {
                let (op_str, op_toks) = if kind == NodeKind::RangeFromToInclusive {
                    ("..=", 3usize)
                } else {
                    ("..", 2usize)
                };
                let lo_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let hi_slot = slot.child_slot_from(self.parsed, lo_slot.last + 1 + op_toks, ch[1]);
                let sep = if self.slot_starts_with_minus(hi_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let lo_doc = self.format_expr(ch[0], lo_slot);
                let hi_doc = self.format_expr(ch[1], hi_slot);
                (Doc::concat([lo_doc, Doc::str(op_str), sep, hi_doc]), false)
            }

            // Binary expressions
            NodeKind::Add
            | NodeKind::Sub
            | NodeKind::Mul
            | NodeKind::Div
            | NodeKind::Mod
            | NodeKind::AddAdd
            | NodeKind::BoolEq
            | NodeKind::BoolNotEq
            | NodeKind::BoolAnd
            | NodeKind::BoolOr
            | NodeKind::BoolGt
            | NodeKind::BoolGtEq
            | NodeKind::BoolLt
            | NodeKind::BoolLtEq
            | NodeKind::BoolImplies
            | NodeKind::BoolMatches
            | NodeKind::Arrow
            | NodeKind::TypedWith
            | NodeKind::Subtype
            | NodeKind::TraitBound
            | NodeKind::Pipe
            | NodeKind::PipePrepend => self.format_binary_expr(node, kind, slot),

            // Prefix unary & type forms
            NodeKind::Negative => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let sep = if self.slot_starts_with_minus(sub_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("-"), sep, sub_doc]), ends_b)
            }
            NodeKind::BoolNot => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("not "), sub_doc]), ends_b)
            }
            NodeKind::ErrorNew => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("error "), sub_doc]), ends_b)
            }
            NodeKind::Defer => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("defer "), sub_doc]), ends_b)
            }
            NodeKind::OptionalType => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("?"), sub_doc]), ends_b)
            }
            NodeKind::PointerType => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("*"), sub_doc]), ends_b)
            }
            NodeKind::TraitObjectType => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("dyn "), sub_doc]), ends_b)
            }
            NodeKind::LiftType => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (sub_doc, ends_b) = self.format_expr_with_rbrace(ch[0], sub_slot);
                (Doc::concat([Doc::str("lift "), sub_doc]), ends_b)
            }
            NodeKind::RefinedType => {
                // `refines { req } base`
                let open_brace = slot.first + 1;
                let close_brace = find_matching_delim(
                    &self.parsed.sig_tokens,
                    open_brace,
                    TokenKind::LBrace,
                    TokenKind::RBrace,
                )
                .unwrap_or(open_brace + 1);
                let req_slot = TokenSlot::new(
                    open_brace + 1,
                    close_brace.saturating_sub(1).max(open_brace + 1),
                );
                let req_doc = self.format_expr(ch[0], req_slot);
                let base_slot = slot.child_slot_from(self.parsed, close_brace + 1, ch[1]);
                let (base_doc, ends_b) = self.format_expr_with_rbrace(ch[1], base_slot);
                let tab = self.tab_spaces();
                let doc = Doc::concat([
                    Doc::group(Doc::concat([
                        Doc::str("refines {"),
                        Doc::indent(tab, Doc::concat([Doc::Line, req_doc])),
                        Doc::Line,
                        Doc::str("}"),
                    ])),
                    Doc::space(),
                    base_doc,
                ]);
                (doc, ends_b)
            }
            NodeKind::EffectQualifiedType
            | NodeKind::ErrorQualifiedType
            | NodeKind::ReachabilityQualifiedType => {
                let sigil = match kind {
                    NodeKind::EffectQualifiedType => "#",
                    NodeKind::ErrorQualifiedType => "!",
                    _ => "&",
                };
                let qual_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let body_slot = slot.child_slot_from(self.parsed, qual_slot.last + 1, ch[1]);
                let qual_doc = self.format_expr(ch[0], qual_slot);
                let (body_doc, ends_b) = self.format_expr_with_rbrace(ch[1], body_slot);
                (
                    Doc::concat([Doc::str(sigil), qual_doc, Doc::space(), body_doc]),
                    ends_b,
                )
            }
            NodeKind::ClosureQualifiedType => {
                let ctx = ch[0];
                let body = ch[1];
                let (ctx_doc, after_ctx_tok) =
                    if self.parsed.ast.get_node_kind(ctx) == Some(NodeKind::ClosureEnvironment) {
                        let open_brace = slot.first + 1;
                        let close_brace = find_matching_delim(
                            &self.parsed.sig_tokens,
                            open_brace,
                            TokenKind::LBrace,
                            TokenKind::RBrace,
                        )
                        .unwrap_or(open_brace + 1);
                        let env_ch = self.parsed.ast.get_children(ctx);
                        let bindings = self
                            .parsed
                            .ast
                            .get_multi_child_slice(env_ch[0])
                            .unwrap_or(&[]);
                        let env_doc = self.format_delimited_seq(
                            bindings,
                            open_brace,
                            close_brace,
                            DelimStyle::SpacedBraces,
                            Some(self.cfg().width_heuristics().struct_lit_width),
                            |fmt, b_node, b_slot| fmt.format_closure_env_binding(b_node, b_slot),
                        );
                        (env_doc, close_brace + 1)
                    } else {
                        let ctx_slot = slot.child_slot_from(self.parsed, slot.first + 1, ctx);
                        (self.format_expr(ctx, ctx_slot), ctx_slot.last + 1)
                    };
                let body_slot = slot.child_slot_from(self.parsed, after_ctx_tok, body);
                let (body_doc, ends_b) = self.format_expr_with_rbrace(body, body_slot);
                (
                    Doc::concat([Doc::str("%"), ctx_doc, Doc::space(), body_doc]),
                    ends_b,
                )
            }
            NodeKind::FnType => (self.format_fn_type(node, slot), false),
            NodeKind::ForallType | NodeKind::ForType => {
                let kw = if kind == NodeKind::ForallType {
                    "forall"
                } else {
                    "for"
                };
                let body = ch[0];
                let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let open_lt = slot.first + 1;
                let close_gt = find_matching_delim(
                    &self.parsed.sig_tokens,
                    open_lt,
                    TokenKind::Lt,
                    TokenKind::Gt,
                )
                .unwrap_or(open_lt + 1);
                let params_doc = self.format_delimited_seq(
                    params,
                    open_lt,
                    close_gt,
                    DelimStyle::Angles,
                    None,
                    |fmt, p, p_slot| fmt.format_clause_or_param(p, p_slot),
                );
                let body_slot = slot.child_slot_from(self.parsed, close_gt + 1, body);
                let (body_doc, ends_b) = self.format_expr_with_rbrace(body, body_slot);
                let tab = self.tab_spaces();
                let doc = Doc::group(Doc::concat([
                    Doc::str(kw),
                    params_doc,
                    Doc::indent(tab, Doc::concat([Doc::Line, body_doc])),
                ]));
                (doc, ends_b)
            }
            NodeKind::BoolForall | NodeKind::BoolExists => {
                let kw = if kind == NodeKind::BoolForall {
                    "forall "
                } else {
                    "exists "
                };
                let body = ch[0];
                let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                // Locate `=>` (`FatArrow`) separating `params` from `body`.
                let fat_arrow = self
                    .parsed
                    .find_depth_zero_token(
                        slot.first + 1,
                        slot.last,
                        |k| k == TokenKind::FatArrow,
                        false,
                    )
                    .unwrap_or(slot.first + 1);
                let mut param_docs = Vec::with_capacity(params.len());
                let mut cursor = slot.first + 1;
                let params_bound = TokenSlot::new(cursor, fat_arrow.saturating_sub(1).max(cursor));
                for &p in params {
                    let p_slot = params_bound.child_slot_from(self.parsed, cursor, p);
                    param_docs.push(self.format_clause_or_param(p, p_slot));
                    cursor = p_slot.last + 1;
                    if cursor < fat_arrow && self.parsed.sig_tokens[cursor].kind == TokenKind::Comma
                    {
                        cursor += 1;
                    }
                }
                let body_slot = slot.child_slot_from(self.parsed, fat_arrow + 1, body);
                let (body_doc, ends_b) = self.format_expr_with_rbrace(body, body_slot);
                let tab = self.tab_spaces();
                let doc = Doc::group(Doc::concat([
                    Doc::str(kw),
                    Doc::join(param_docs, Doc::str(", ")),
                    Doc::str(" =>"),
                    Doc::indent(tab, Doc::concat([Doc::Line, body_doc])),
                ]));
                (doc, ends_b)
            }

            // CMTT Contextual types & Quote/Splice expressions
            NodeKind::ContextualExprType => (self.format_contextual_expr_type(node, slot), false),
            NodeKind::ContextualPatternType => {
                (self.format_contextual_pattern_type(node, slot), false)
            }
            NodeKind::QuoteExpr => {
                let body = ch[0];
                let substs = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let (subst_doc, after_subst) =
                    self.format_code_substitutions(substs, slot.first + 1, slot.last);
                // `(` is at `after_subst`, `)` is at `slot.last`.
                let inner_slot = TokenSlot::new(
                    (after_subst + 1).min(slot.last),
                    slot.last.saturating_sub(1).max(after_subst),
                );
                let tab = self.tab_spaces();
                let body_doc = self.format_expr(body, inner_slot);
                let doc = Doc::group(Doc::concat([
                    Doc::str("'"),
                    subst_doc,
                    Doc::str("("),
                    Doc::indent(tab, Doc::concat([Doc::SoftLine, body_doc])),
                    Doc::SoftLine,
                    Doc::str(")"),
                ]));
                (doc, false)
            }
            NodeKind::SpliceExpr => {
                let operand = ch[0];
                let substs = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let (subst_doc, after_subst) =
                    self.format_code_substitutions(substs, slot.first + 1, slot.last);
                // `<` is at `after_subst`, `>` is at `slot.last`.
                let inner_slot = TokenSlot::new(
                    (after_subst + 1).min(slot.last),
                    slot.last.saturating_sub(1).max(after_subst),
                );
                let expr_doc = self.format_expr(operand, inner_slot);
                (
                    Doc::concat([
                        Doc::str("~"),
                        subst_doc,
                        Doc::str("<"),
                        expr_doc,
                        Doc::str(">"),
                    ]),
                    false,
                )
            }
            NodeKind::QuotePattern | NodeKind::SplicePattern => {
                (self.format_pattern(node, slot), false)
            }

            // Lambda (`|params| -> ret body` or `|params| body`)
            NodeKind::Lambda => self.format_lambda(node, slot),

            // Blocks & keyword blocks
            NodeKind::Block => {
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                (
                    self.format_block(node, slot, budget, ContainerKind::ValueBlock),
                    true,
                )
            }
            NodeKind::DoBlock
            | NodeKind::AsyncBlock
            | NodeKind::UnsafeBlock
            | NodeKind::ComptimeBlock => {
                let kw = match kind {
                    NodeKind::DoBlock => "do ",
                    NodeKind::AsyncBlock => "async ",
                    NodeKind::UnsafeBlock => "unsafe ",
                    _ => "comptime ",
                };
                let block_slot = TokenSlot::new(slot.first + 1, slot.last);
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                let b_doc = self.format_block(ch[0], block_slot, budget, ContainerKind::ValueBlock);
                (Doc::concat([Doc::str(kw), b_doc]), true)
            }
            NodeKind::AtomicBlock => {
                let block = ch[0];
                let ids = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let open_paren = slot.first + 1;
                let close_paren = find_matching_delim(
                    &self.parsed.sig_tokens,
                    open_paren,
                    TokenKind::LParen,
                    TokenKind::RParen,
                )
                .unwrap_or(open_paren + 1);
                let ids_doc = self.format_delimited_seq(
                    ids,
                    open_paren,
                    close_paren,
                    DelimStyle::Parens,
                    None,
                    |fmt, id_n, id_s| Doc::text(fmt.decode_id(id_n, id_s.first)),
                );
                let block_slot = TokenSlot::new(close_paren + 1, slot.last);
                let b_doc = self.format_block(block, block_slot, None, ContainerKind::ValueBlock);
                (
                    Doc::concat([Doc::str("atomic"), ids_doc, Doc::space(), b_doc]),
                    true,
                )
            }

            // Control-flow & Statement expressions
            NodeKind::IfStatement | NodeKind::IfIsMatch | NodeKind::IfMatch => {
                (self.format_if_family(node, kind, slot, true), true)
            }
            NodeKind::WhenStatement => (self.format_when_statement(node, slot), true),
            NodeKind::WhileStatement | NodeKind::WhileIsMatch | NodeKind::WhileMatch => {
                (self.format_while_family(node, kind, slot), true)
            }
            NodeKind::ForStatement => (self.format_for_statement(node, slot), true),
            NodeKind::InlineStatement => {
                let inner_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let (inner_doc, ends_b) = self.format_container_member(
                    ch[0],
                    inner_slot,
                    false,
                    ContainerKind::ValueBlock,
                );
                (Doc::concat([Doc::str("inline "), inner_doc]), ends_b)
            }
            NodeKind::LetDecl | NodeKind::VarDecl | NodeKind::ConstDecl => {
                self.format_local_decl(node, kind, slot)
            }
            NodeKind::Assign
            | NodeKind::AddAssign
            | NodeKind::SubAssign
            | NodeKind::MulAssign
            | NodeKind::DivAssign
            | NodeKind::RemAssign => {
                let op_str = match kind {
                    NodeKind::Assign => "=",
                    NodeKind::AddAssign => "+=",
                    NodeKind::SubAssign => "-=",
                    NodeKind::MulAssign => "*=",
                    NodeKind::DivAssign => "/=",
                    _ => "%=",
                };
                let lhs_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let op_tok = lhs_slot.last + 1;
                let rhs_slot = slot.child_slot_from(self.parsed, op_tok + 1, ch[1]);
                let lhs_doc = self.format_expr(ch[0], lhs_slot);
                let (rhs_doc, ends_b) = self.format_expr_with_rbrace(ch[1], rhs_slot);
                let tab = self.tab_spaces();
                let doc = if ends_b {
                    Doc::concat([lhs_doc, Doc::text(format!(" {op_str} ")), rhs_doc])
                } else {
                    Doc::group(Doc::concat([
                        lhs_doc,
                        Doc::text(format!(" {op_str}")),
                        Doc::indent(tab, Doc::concat([Doc::Line, rhs_doc])),
                    ]))
                };
                (doc, ends_b)
            }
            NodeKind::ReturnStatement | NodeKind::ResumeStatement => {
                let kw = if kind == NodeKind::ReturnStatement {
                    "return"
                } else {
                    "resume"
                };
                self.format_jump_with_expr_and_guard(kw, ch[0], ch[1], slot)
            }
            NodeKind::BreakStatement | NodeKind::ContinueStatement => {
                let kw = if kind == NodeKind::BreakStatement {
                    "break"
                } else {
                    "continue"
                };
                self.format_jump_with_label_and_guard(kw, ch[0], ch[1], slot)
            }
            NodeKind::Asserts
            | NodeKind::Assumes
            | NodeKind::Axiom
            | NodeKind::Invariant
            | NodeKind::Decreases
            | NodeKind::Requires
            | NodeKind::Ensures => self.format_verification_stmt_or_clause(node, kind, slot),

            // Fallback to item formatter for nested local definitions
            _ => self.format_container_member(node, slot, false, ContainerKind::ValueBlock),
        }
    }

    fn format_postfix_or_chain(&mut self, node: NodeIndex, slot: TokenSlot) -> (Doc, bool) {
        let Some(kind) = self.parsed.ast.get_node_kind(node) else {
            return (Doc::Nil, false);
        };
        let ch = self.parsed.ast.get_children(node);

        // Trailing postfix blocks (`match { ... }`, `do ...`, `! { ... }`, `# { ... }`, `? { ... }`)
        // format their receiver at current indent so the block body indents `+tab_spaces` from
        // the statement base indent rather than double-indenting!
        match kind {
            NodeKind::PostMatch => {
                let recv = ch[0];
                let arms = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let recv_slot = slot.child_slot_from(self.parsed, slot.first, recv);
                let match_tok = self
                    .parsed
                    .find_depth_zero_token(
                        recv_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Match,
                        false,
                    )
                    .unwrap_or(recv_slot.last + 1);
                let open_brace = match_tok + 1;
                let recv_doc = self.format_expr(recv, recv_slot);
                let arms_doc = self.format_arms_block(
                    arms,
                    open_brace,
                    slot.last,
                    ExprPatternMode::Standalone,
                );
                return (Doc::concat([recv_doc, Doc::str(" match "), arms_doc]), true);
            }
            NodeKind::ErrorElimination => {
                let recv = ch[0];
                let arms = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let recv_slot = slot.child_slot_from(self.parsed, slot.first, recv);
                let bang_tok = self
                    .parsed
                    .find_depth_zero_token(
                        recv_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Bang,
                        false,
                    )
                    .unwrap_or(recv_slot.last + 1);
                let open_brace = bang_tok + 1;
                let recv_doc = self.format_expr(recv, recv_slot);
                let arms_doc = self.format_arms_block(
                    arms,
                    open_brace,
                    slot.last,
                    ExprPatternMode::Standalone,
                );
                return (Doc::concat([recv_doc, Doc::str("! "), arms_doc]), true);
            }
            NodeKind::EffectElimination => {
                let recv = ch[0];
                let arms = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let recv_slot = slot.child_slot_from(self.parsed, slot.first, recv);
                let hash_tok = self
                    .parsed
                    .find_depth_zero_token(
                        recv_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Hash,
                        false,
                    )
                    .unwrap_or(recv_slot.last + 1);
                let open_brace = hash_tok + 1;
                let recv_doc = self.format_expr(recv, recv_slot);
                let arms_doc = self.format_arms_block(
                    arms,
                    open_brace,
                    slot.last,
                    ExprPatternMode::BareEffectOp,
                );
                return (Doc::concat([recv_doc, Doc::str("# "), arms_doc]), true);
            }
            NodeKind::OptionElimination => {
                let recv = ch[0];
                let block = ch[1];
                let recv_slot = slot.child_slot_from(self.parsed, slot.first, recv);
                let q_tok = self
                    .parsed
                    .find_depth_zero_token(
                        recv_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Question,
                        false,
                    )
                    .unwrap_or(recv_slot.last + 1);
                let block_slot = TokenSlot::new(q_tok + 1, slot.last);
                let recv_doc = self.format_expr(recv, recv_slot);
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                let b_doc = self.format_block(block, block_slot, budget, ContainerKind::ValueBlock);
                return (Doc::concat([recv_doc, Doc::str("? "), b_doc]), true);
            }
            NodeKind::PostLambda => {
                let recv = ch[0];
                let body = ch[1];
                let recv_slot = slot.child_slot_from(self.parsed, slot.first, recv);
                let do_tok = self
                    .parsed
                    .find_depth_zero_token(
                        recv_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Do,
                        false,
                    )
                    .unwrap_or(recv_slot.last + 1);
                let body_slot = slot.child_slot_from(self.parsed, do_tok + 1, body);
                let recv_doc = self.format_expr(recv, recv_slot);
                let (body_doc, ends_b) = self.format_expr_with_rbrace(body, body_slot);
                return (Doc::concat([recv_doc, Doc::str(" do "), body_doc]), ends_b);
            }
            NodeKind::ExtendedApplication => {
                let callee = ch[0];
                let args = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let callee_slot = slot.child_slot_from(self.parsed, slot.first, callee);
                let open_brace = callee_slot.last + 1;
                let callee_doc = self.format_expr(callee, callee_slot);
                let budget = Some(self.cfg().width_heuristics().struct_lit_width);
                let args_doc = self.format_delimited_seq(
                    args,
                    open_brace,
                    slot.last,
                    DelimStyle::SpacedBraces,
                    budget,
                    |fmt, arg, arg_slot| fmt.format_call_arg(arg, arg_slot),
                );
                return (Doc::concat([callee_doc, Doc::space(), args_doc]), true);
            }
            _ => {}
        }

        // Unroll `.` / call / index / `'view` / `?` / `!` / `#` postfix chain.
        let mut steps: Vec<ChainStep> = Vec::new();
        let mut cur_node = node;
        let mut cur_slot = slot;

        loop {
            let (parens, inner) = cur_slot.unwrap_parens(self.parsed, cur_node);
            if parens > 0 && !steps.is_empty() {
                break;
            }
            let Some(k) = self.parsed.ast.get_node_kind(cur_node) else {
                break;
            };
            let c = self.parsed.ast.get_children(cur_node);
            match k {
                NodeKind::TakeView => {
                    // Unroll consecutive `'a'b` views together to avoid split `Char` token hazards!
                    let mut views = Vec::new();
                    let mut v_cur = cur_node;
                    while self.parsed.ast.get_node_kind(v_cur) == Some(NodeKind::TakeView) {
                        let vc = self.parsed.ast.get_children(v_cur);
                        views.push(self.decode_id(vc[1], inner.first));
                        v_cur = vc[0];
                    }
                    views.reverse();
                    let recv_slot = inner.child_slot_from(self.parsed, inner.first, v_cur);
                    steps.push(ChainStep::TightViews(views));
                    cur_node = v_cur;
                    cur_slot = recv_slot;
                }
                NodeKind::OptionPropagation => {
                    let recv_slot =
                        TokenSlot::new(inner.first, inner.last.saturating_sub(1).max(inner.first));
                    steps.push(ChainStep::TightSigil("?"));
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::ErrorPropagation => {
                    let recv_slot =
                        TokenSlot::new(inner.first, inner.last.saturating_sub(1).max(inner.first));
                    steps.push(ChainStep::TightSigil("!"));
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::EffectPropagation => {
                    let recv_slot =
                        TokenSlot::new(inner.first, inner.last.saturating_sub(1).max(inner.first));
                    steps.push(ChainStep::TightSigil("#"));
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::Application => {
                    let recv_slot = inner.child_slot_from(self.parsed, inner.first, c[0]);
                    steps.push(ChainStep::Call {
                        node: cur_node,
                        open_tok: recv_slot.last + 1,
                        close_tok: inner.last,
                    });
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::NormalFormHeadApplication => {
                    let recv_slot = inner.child_slot_from(self.parsed, inner.first, c[0]);
                    steps.push(ChainStep::DiamondCall {
                        node: cur_node,
                        open_tok: recv_slot.last + 1,
                        close_tok: inner.last,
                    });
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::IndexApplication => {
                    let recv_slot = inner.child_slot_from(self.parsed, inner.first, c[0]);
                    steps.push(ChainStep::Index {
                        idx_expr: c[1],
                        open_tok: recv_slot.last + 1,
                        close_tok: inner.last,
                    });
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::Projection => {
                    let recv_slot = inner.child_slot_from(self.parsed, inner.first, c[0]);
                    let field = self.decode_id(c[1], recv_slot.last + 1);
                    steps.push(ChainStep::DotField(field));
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::Deref => {
                    let recv_slot =
                        TokenSlot::new(inner.first, inner.last.saturating_sub(2).max(inner.first));
                    steps.push(ChainStep::DotStatic(".*"));
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::Refer => {
                    let recv_slot =
                        TokenSlot::new(inner.first, inner.last.saturating_sub(2).max(inner.first));
                    steps.push(ChainStep::DotStatic(".ref"));
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::Await => {
                    let recv_slot =
                        TokenSlot::new(inner.first, inner.last.saturating_sub(2).max(inner.first));
                    steps.push(ChainStep::DotStatic(".await"));
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                NodeKind::TypeCast | NodeKind::DynCast | NodeKind::HandlerApplication => {
                    let prefix = match k {
                        NodeKind::TypeCast => ".as(",
                        NodeKind::DynCast => ".dyn(",
                        _ => ".use(",
                    };
                    let recv_slot = inner.child_slot_from(self.parsed, inner.first, c[0]);
                    let open_paren = recv_slot.last + 3; // `.` + `as`/`dyn`/`use` + `(`
                    let arg_slot = TokenSlot::new(
                        (open_paren + 1).min(inner.last),
                        inner.last.saturating_sub(1).max(open_paren),
                    );
                    steps.push(ChainStep::DotParen {
                        prefix,
                        arg_node: c[1],
                        arg_slot,
                    });
                    cur_node = c[0];
                    cur_slot = recv_slot;
                }
                _ => break,
            }
        }

        steps.reverse();
        let head_doc = self.format_expr(cur_node, cur_slot);

        // Count how many `.`-prefixed links exist in `steps`.
        // Note: pure identifier path `a.b.c` (all `Id` + `DotField` with no calls/casts/derefs)
        // stays inline unless exceeding `max_width`; method/property chains with 2+ dot links obey `chain_width`.
        let dot_count = steps.iter().filter(|s| s.is_dot_link()).count();
        let has_complex_step = steps.iter().any(|s| !matches!(s, ChainStep::DotField(_)))
            || self.parsed.ast.get_node_kind(cur_node) != Some(NodeKind::Id);

        // Group tight suffixes (`(...)`, `<...>`, `[...]`, `'view`, `?`, `!`, `#`) onto their
        // preceding head or `.` link.
        let mut head_parts = vec![head_doc];
        let mut dot_links: Vec<Doc> = Vec::new();

        for step in steps {
            let is_dot = step.is_dot_link();
            let rendered = self.render_chain_step(step);
            if is_dot {
                dot_links.push(rendered);
            } else if let Some(last_link) = dot_links.last_mut() {
                let prev = std::mem::replace(last_link, Doc::Nil);
                *last_link = Doc::concat([prev, rendered]);
            } else {
                head_parts.push(rendered);
            }
        }

        let full_head = Doc::concat(head_parts);
        if dot_links.is_empty() {
            return (full_head, false);
        }

        if dot_count == 1 && !has_complex_step {
            return (Doc::concat([full_head, dot_links.remove(0)]), false);
        }

        let tab = self.tab_spaces();
        let mut chain_tail = Vec::with_capacity(dot_links.len() * 2);
        for link in dot_links {
            chain_tail.push(Doc::SoftLine);
            chain_tail.push(link);
        }

        let combined = Doc::concat([full_head, Doc::indent(tab, Doc::concat(chain_tail))]);
        let doc = if dot_count >= 2 && has_complex_step {
            Doc::group_with_budget(self.cfg().width_heuristics().chain_width, combined)
        } else {
            Doc::group(combined)
        };
        (doc, false)
    }

    fn render_chain_step(&mut self, step: ChainStep) -> Doc {
        match step {
            ChainStep::TightViews(views) => {
                let mut s = String::new();
                for v in views {
                    s.push('\'');
                    s.push_str(&v);
                }
                Doc::text(s)
            }
            ChainStep::TightSigil(sig) => Doc::str(sig),
            ChainStep::Call {
                node,
                open_tok,
                close_tok,
            } => {
                let ch = self.parsed.ast.get_children(node);
                let args = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                self.format_delimited_seq(
                    args,
                    open_tok,
                    close_tok,
                    DelimStyle::Parens,
                    budget,
                    |fmt, arg, arg_slot| fmt.format_call_arg(arg, arg_slot),
                )
            }
            ChainStep::DiamondCall {
                node,
                open_tok,
                close_tok,
            } => {
                let ch = self.parsed.ast.get_children(node);
                let args = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                self.format_delimited_seq(
                    args,
                    open_tok,
                    close_tok,
                    DelimStyle::Angles,
                    budget,
                    |fmt, arg, arg_slot| fmt.format_call_arg(arg, arg_slot),
                )
            }
            ChainStep::Index {
                idx_expr,
                open_tok,
                close_tok,
            } => {
                let idx_slot = TokenSlot::new(
                    (open_tok + 1).min(close_tok),
                    close_tok.saturating_sub(1).max(open_tok),
                );
                let idx_doc = self.format_expr(idx_expr, idx_slot);
                let tab = self.tab_spaces();
                Doc::group(Doc::concat([
                    Doc::str("["),
                    Doc::indent(tab, Doc::concat([Doc::SoftLine, idx_doc])),
                    Doc::SoftLine,
                    Doc::str("]"),
                ]))
            }
            ChainStep::DotField(name) => Doc::text(format!(".{name}")),
            ChainStep::DotStatic(s) => Doc::str(s),
            ChainStep::DotParen {
                prefix,
                arg_node,
                arg_slot,
            } => {
                let arg_doc = self.format_expr(arg_node, arg_slot);
                let tab = self.tab_spaces();
                Doc::group(Doc::concat([
                    Doc::str(prefix),
                    Doc::indent(tab, Doc::concat([Doc::SoftLine, arg_doc])),
                    Doc::SoftLine,
                    Doc::str(")"),
                ]))
            }
        }
    }

    fn format_binary_expr(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let ch = self.parsed.ast.get_children(node);

        // `TypedWith` (`expr: Type`) has no space before `:`!
        if kind == NodeKind::TypedWith {
            let lhs_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
            let colon_tok = lhs_slot.last + 1;
            let rhs_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
            let lhs_doc = self.format_expr(ch[0], lhs_slot);
            let (rhs_doc, ends_b) = self.format_expr_with_rbrace(ch[1], rhs_slot);
            return (Doc::concat([lhs_doc, Doc::str(": "), rhs_doc]), ends_b);
        }

        // `BoolMatches` (`expr matches pattern`) has a pattern RHS!
        if kind == NodeKind::BoolMatches {
            let lhs_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
            let matches_tok = lhs_slot.last + 1;
            let pat_slot = slot.child_slot_from(self.parsed, matches_tok + 1, ch[1]);
            let lhs_doc = self.format_expr(ch[0], lhs_slot);
            let pat_doc = self.format_pattern(ch[1], pat_slot);
            let tab = self.tab_spaces();
            let doc = Doc::group(Doc::concat([
                lhs_doc,
                Doc::indent(tab, Doc::concat([Doc::Line, Doc::str("matches "), pat_doc])),
            ]));
            return (doc, false);
        }

        // Flatten same-precedence chains for left-associative (and right-associative `->` / `==>`)
        // operators so Front-style continuation lines align at `+tab_spaces` rather than stair-stepping!
        let chain_prec = binary_prec(kind);
        let mut operands: Vec<(Option<String>, NodeIndex, TokenSlot, usize, usize)> = Vec::new();
        self.collect_binary_chain(node, slot, chain_prec, None, 0, 0, &mut operands);

        let tab = self.tab_spaces();
        let (_, first_node, first_slot, _, _) = operands[0];
        let first_doc = self.format_expr(first_node, first_slot);

        let mut rest_parts = Vec::with_capacity((operands.len() - 1) * 3);
        let mut last_ends_b = false;
        let last_idx = operands.len() - 1;

        for (i, &(ref op_opt, rhs_node, rhs_slot, prev_hi, rhs_lo)) in
            operands.iter().enumerate().skip(1)
        {
            let op_str = op_opt.as_deref().unwrap_or("+");
            let inline_c = self.comments.take_inline_between(
                prev_hi,
                rhs_lo,
                self.indent_spaces + tab,
                &self.cfg(),
            );
            if !matches!(inline_c, Doc::Nil) {
                rest_parts.push(inline_c);
            } else {
                rest_parts.push(Doc::Line);
            }
            let (rhs_doc, ends_b) = self.format_expr_with_rbrace(rhs_node, rhs_slot);
            if i == last_idx {
                last_ends_b = ends_b;
            }
            rest_parts.push(Doc::text(format!("{op_str} ")));
            rest_parts.push(rhs_doc);
        }

        let doc = Doc::group(Doc::concat([
            first_doc,
            Doc::indent(tab, Doc::concat(rest_parts)),
        ]));
        (doc, last_ends_b)
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_binary_chain(
        &self,
        node: NodeIndex,
        slot: TokenSlot,
        target_prec: i32,
        incoming_op: Option<String>,
        incoming_prev_hi: usize,
        incoming_lo: usize,
        out: &mut Vec<(Option<String>, NodeIndex, TokenSlot, usize, usize)>,
    ) {
        let (parens, inner) = slot.unwrap_parens(self.parsed, node);
        if parens > 0 && incoming_op.is_some() {
            out.push((incoming_op, node, slot, incoming_prev_hi, incoming_lo));
            return;
        }
        let Some(kind) = self.parsed.ast.get_node_kind(node) else {
            out.push((incoming_op, node, slot, incoming_prev_hi, incoming_lo));
            return;
        };
        if kind == NodeKind::TypedWith
            || kind == NodeKind::BoolMatches
            || binary_prec(kind) != target_prec
        {
            out.push((incoming_op, node, slot, incoming_prev_hi, incoming_lo));
            return;
        }

        let ch = self.parsed.ast.get_children(node);
        let lhs = ch[0];
        let rhs = ch[1];
        let lhs_slot = inner.child_slot_from(self.parsed, inner.first, lhs);
        let op_tok_idx = lhs_slot.last + 1;
        let op_byte = self.parsed.sig_tokens.get(op_tok_idx).map_or(0, |t| t.from);
        let op_str = self.binary_op_str(kind, op_byte);
        let rhs_slot = inner.child_slot_from(self.parsed, op_tok_idx + 1, rhs);
        let lhs_hi = self
            .parsed
            .sig_tokens
            .get(lhs_slot.last)
            .map_or(0, |t| t.to);
        let rhs_lo = self
            .parsed
            .sig_tokens
            .get(rhs_slot.first)
            .map_or(0, |t| t.from);

        if is_right_assoc_binary(kind) {
            out.push((incoming_op, lhs, lhs_slot, incoming_prev_hi, incoming_lo));
            self.collect_binary_chain(
                rhs,
                rhs_slot,
                target_prec,
                Some(op_str.to_string()),
                lhs_hi,
                rhs_lo,
                out,
            );
        } else {
            self.collect_binary_chain(
                lhs,
                lhs_slot,
                target_prec,
                incoming_op,
                incoming_prev_hi,
                incoming_lo,
                out,
            );
            out.push((Some(op_str.to_string()), rhs, rhs_slot, lhs_hi, rhs_lo));
        }
    }

    fn binary_op_str(&self, kind: NodeKind, op_byte: usize) -> &'static str {
        match kind {
            NodeKind::Add => "+",
            NodeKind::Sub => "-",
            NodeKind::Mul => "*",
            NodeKind::Div => "/",
            NodeKind::Mod => "%",
            NodeKind::AddAdd => "++",
            NodeKind::BoolEq => "==",
            NodeKind::BoolNotEq => "!=",
            NodeKind::BoolAnd => "and",
            NodeKind::BoolOr => "or",
            NodeKind::BoolGt => ">",
            NodeKind::BoolGtEq => ">=",
            NodeKind::BoolLt => "<",
            NodeKind::BoolLtEq => "<=",
            NodeKind::BoolImplies => {
                if self.parsed.bridge.implies_offsets.contains(&op_byte) {
                    "==>"
                } else {
                    "==="
                }
            }
            NodeKind::Arrow => "->",
            NodeKind::Subtype => "<:",
            NodeKind::TraitBound => {
                if self.parsed.bridge.subtype_offsets.contains(&op_byte) {
                    "<:"
                } else {
                    ":-"
                }
            }
            NodeKind::Pipe => "|",
            NodeKind::PipePrepend => "|>",
            _ => "+",
        }
    }

    fn format_closure_env_binding(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let id_str = self.decode_id(ch[0], slot.first);
        let colon_tok = self
            .parsed
            .find_depth_zero_token(slot.first + 1, slot.last, |k| k == TokenKind::Colon, false)
            .unwrap_or(slot.first + 1);
        let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
        let ty_doc = self.format_expr(ch[1], ty_slot);
        let val = ch.get(2).copied().unwrap_or(0);
        if val != 0 {
            let eq_tok = self
                .parsed
                .find_depth_zero_token(ty_slot.last + 1, slot.last, |k| k == TokenKind::Eq, false)
                .unwrap_or(ty_slot.last + 1);
            let val_slot = slot.child_slot_from(self.parsed, eq_tok + 1, val);
            let val_doc = self.format_expr(val, val_slot);
            Doc::concat([
                Doc::text(id_str),
                Doc::str(": "),
                ty_doc,
                Doc::str(" = "),
                val_doc,
            ])
        } else {
            Doc::concat([Doc::text(id_str), Doc::str(": "), ty_doc])
        }
    }

    fn format_fn_type(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let flags = ch[0];
        let params = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);

        let mut mods = Vec::new();
        if flags & FN_MOD_PURE != 0 {
            mods.push("pure");
        }
        if flags & FN_MOD_COMPTIME != 0 {
            mods.push("comptime");
        }
        if flags & FN_MOD_INLINE != 0 {
            mods.push("inline");
        }
        if flags & FN_MOD_UNSAFE != 0 {
            mods.push("unsafe");
        }
        if flags & FN_MOD_SPEC != 0 {
            mods.push("spec");
        }
        if flags & FN_MOD_VERIFIED != 0 {
            mods.push("verified");
        }
        if flags & FN_MOD_ATOMIC != 0 {
            mods.push("atomic");
        }
        mods.push("fn");
        let prefix = mods.join(" ");

        let is_nf = flags & FN_MOD_NORMAL_FORM != 0;
        let open_kind = if is_nf {
            TokenKind::Lt
        } else {
            TokenKind::LParen
        };
        let style = if is_nf {
            DelimStyle::Angles
        } else {
            DelimStyle::Parens
        };
        let open_tok = self
            .parsed
            .find_depth_zero_token(slot.first, slot.last, |k| k == open_kind, false)
            .unwrap_or(slot.first + 1);
        let params_doc = self.format_delimited_seq(
            params,
            open_tok,
            slot.last,
            style,
            None,
            |fmt, p, p_slot| fmt.format_clause_or_param(p, p_slot),
        );
        Doc::concat([Doc::text(prefix), params_doc])
    }

    fn format_contextual_expr_type(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        // `< x: T, y: U |- R >` or `< |- R >` (`|-` is `Pipe` + `Minus`/`SeparatedMinus`)
        let ch = self.parsed.ast.get_children(node);
        let res_ty = ch[0];
        let bindings = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);

        let mut pipe_tok = slot.first + 1;
        for idx in (slot.first + 1)..slot.last {
            if self.parsed.sig_tokens[idx].kind == TokenKind::Pipe
                && matches!(
                    self.parsed.sig_tokens[idx + 1].kind,
                    TokenKind::Minus | TokenKind::SeparatedMinus
                )
            {
                pipe_tok = idx;
                break;
            }
        }
        let res_slot = TokenSlot::new(
            (pipe_tok + 2).min(slot.last),
            slot.last.saturating_sub(1).max(pipe_tok + 2),
        );
        let res_doc = self.format_expr(res_ty, res_slot);

        if bindings.is_empty() {
            return Doc::concat([Doc::str("< |- "), res_doc, Doc::str(" >")]);
        }

        let mut b_docs = Vec::with_capacity(bindings.len());
        let mut cursor = slot.first + 1;
        let b_bound = TokenSlot::new(cursor, pipe_tok.saturating_sub(1).max(cursor));
        for &b in bindings {
            let bs = b_bound.child_slot_from(self.parsed, cursor, b);
            b_docs.push(self.format_code_context_binding(b, bs));
            cursor = bs.last + 1;
            if cursor < pipe_tok && self.parsed.sig_tokens[cursor].kind == TokenKind::Comma {
                cursor += 1;
            }
        }
        let tab = self.tab_spaces();
        Doc::group(Doc::concat([
            Doc::str("<"),
            Doc::indent(
                tab,
                Doc::concat([
                    Doc::Line,
                    Doc::join(b_docs, Doc::concat([Doc::str(","), Doc::Line])),
                    Doc::Line,
                    Doc::str("|- "),
                    res_doc,
                ]),
            ),
            Doc::Line,
            Doc::str(">"),
        ]))
    }

    fn format_contextual_pattern_type(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        // `< S -| x: T >` or `< S -|>` (`-|` is `Minus`/`SeparatedMinus` + `Pipe` / `PipeGt`)
        let ch = self.parsed.ast.get_children(node);
        let subj_ty = ch[0];
        let bindings = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);

        let subj_slot = slot.child_slot_from(self.parsed, slot.first + 1, subj_ty);
        let subj_doc = self.format_expr(subj_ty, subj_slot);

        if bindings.is_empty() {
            return Doc::concat([Doc::str("< "), subj_doc, Doc::str(" -|>")]);
        }

        let mut b_docs = Vec::with_capacity(bindings.len());
        let mut cursor = (subj_slot.last + 3).min(slot.last);
        let b_bound = TokenSlot::new(cursor, slot.last.saturating_sub(1).max(cursor));
        for &b in bindings {
            let bs = b_bound.child_slot_from(self.parsed, cursor, b);
            b_docs.push(self.format_code_context_binding(b, bs));
            cursor = bs.last + 1;
            if cursor < slot.last && self.parsed.sig_tokens[cursor].kind == TokenKind::Comma {
                cursor += 1;
            }
        }
        let tab = self.tab_spaces();
        Doc::group(Doc::concat([
            Doc::str("<"),
            Doc::indent(
                tab,
                Doc::concat([
                    Doc::Line,
                    subj_doc,
                    Doc::str(" -|"),
                    Doc::Line,
                    Doc::join(b_docs, Doc::concat([Doc::str(","), Doc::Line])),
                ]),
            ),
            Doc::Line,
            Doc::str(">"),
        ]))
    }

    fn format_code_context_binding(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let name = self.decode_id(ch[0], slot.first);
        let colon_tok = self
            .parsed
            .find_depth_zero_token(slot.first + 1, slot.last, |k| k == TokenKind::Colon, false)
            .unwrap_or(slot.first + 1);
        let ty_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
        let ty_doc = self.format_expr(ch[1], ty_slot);
        Doc::concat([Doc::text(name), Doc::str(": "), ty_doc])
    }

    fn format_lambda(&mut self, node: NodeIndex, slot: TokenSlot) -> (Doc, bool) {
        // `Lambda`: `[return_type, body, params_handle]`
        let ch = self.parsed.ast.get_children(node);
        let ret_ty = ch[0];
        let body = ch[1];
        let params = self.parsed.ast.get_multi_child_slice(ch[2]).unwrap_or(&[]);

        // Closing `|` of `|params|`: note empty `||` may lex as two `Pipe` tokens (Flurry has `or`, not `||`,
        // so `||` is two `TokenKind::Pipe` tokens!).
        let open_pipe = slot.first;
        let close_pipe = self
            .parsed
            .find_depth_zero_token(open_pipe + 1, slot.last, |k| k == TokenKind::Pipe, false)
            .unwrap_or(open_pipe + 1);

        let params_doc = if params.is_empty() {
            Doc::str("||")
        } else {
            let mut p_docs = Vec::with_capacity(params.len());
            let mut cursor = open_pipe + 1;
            let p_bound = TokenSlot::new(cursor, close_pipe.saturating_sub(1).max(cursor));
            for &p in params {
                let ps = p_bound.child_slot_from(self.parsed, cursor, p);
                p_docs.push(self.format_lambda_param(p, ps));
                cursor = ps.last + 1;
                if cursor < close_pipe && self.parsed.sig_tokens[cursor].kind == TokenKind::Comma {
                    cursor += 1;
                }
            }
            let tab = self.tab_spaces();
            Doc::group(Doc::concat([
                Doc::str("|"),
                Doc::indent(
                    tab,
                    Doc::concat([
                        Doc::SoftLine,
                        Doc::join(p_docs, Doc::concat([Doc::str(","), Doc::Line])),
                        Doc::trailing_comma(),
                    ]),
                ),
                Doc::SoftLine,
                Doc::str("|"),
            ]))
        };

        let mut after_header = close_pipe + 1;
        let ret_doc = if ret_ty != 0 {
            let arrow_tok = self
                .parsed
                .find_depth_zero_token(after_header, slot.last, |k| k == TokenKind::Arrow, false)
                .unwrap_or(after_header);
            let ret_slot = slot.child_slot_from(self.parsed, arrow_tok + 1, ret_ty);
            after_header = ret_slot.last + 1;
            let r = self.format_return_type_node(ret_ty, ret_slot);
            Doc::concat([Doc::str(" -> "), r])
        } else {
            Doc::Nil
        };

        let body_slot = slot.child_slot_from(self.parsed, after_header, body);
        if self.parsed.ast.get_node_kind(body) == Some(NodeKind::Block) {
            let budget = Some(self.cfg().width_heuristics().fn_call_width);
            let b_doc = self.format_block(body, body_slot, budget, ContainerKind::ValueBlock);
            (
                Doc::concat([params_doc, ret_doc, Doc::space(), b_doc]),
                true,
            )
        } else {
            let (b_doc, ends_b) = self.format_expr_with_rbrace(body, body_slot);
            let tab = self.tab_spaces();
            let doc = Doc::group(Doc::concat([
                params_doc,
                ret_doc,
                Doc::indent(tab, Doc::concat([Doc::Line, b_doc])),
            ]));
            (doc, ends_b)
        }
    }

    fn format_lambda_param(&mut self, param: NodeIndex, slot: TokenSlot) -> Doc {
        // Lambda parameter may be a pattern OR a `TypeBoundParam` / `OptionalParam` / `AttrParam` etc.
        match self.parsed.ast.get_node_kind(param) {
            Some(
                NodeKind::TypeBoundParam
                | NodeKind::TraitBoundParam
                | NodeKind::OptionalParam
                | NodeKind::OptionalTypeParam
                | NodeKind::VarargParam
                | NodeKind::ComptimeParam
                | NodeKind::ErrorParam
                | NodeKind::CatchParam
                | NodeKind::LambdaParam
                | NodeKind::ImplicitParam
                | NodeKind::QuoteParam
                | NodeKind::AssocParam
                | NodeKind::AttrParam
                | NodeKind::SelfParam
                | NodeKind::SelfRefParam
                | NodeKind::ItselfParam
                | NodeKind::ItselfRefParam,
            ) => self.format_clause_or_param(param, slot),
            _ => self.format_pattern(param, slot),
        }
    }

    /// Format `if cond { ... } else ...`, `if x is p do { ... } else ...`, and `if x is do { ... }`.
    pub fn format_if_family(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
        allow_single_line_if_else: bool,
    ) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let tab = self.tab_spaces();

        if kind == NodeKind::IfMatch {
            // `if expr is do { arms }`
            let expr = ch[0];
            let arms = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
            let expr_slot = slot.child_slot_from(self.parsed, slot.first + 1, expr);
            let is_tok = self
                .parsed
                .find_depth_zero_token(expr_slot.last + 1, slot.last, |k| k == TokenKind::Is, false)
                .unwrap_or(expr_slot.last + 1);
            let do_tok = is_tok + 1;
            let open_brace = do_tok + 1;
            let expr_doc = self.format_expr(expr, expr_slot);
            let arms_doc =
                self.format_arms_block(arms, open_brace, slot.last, ExprPatternMode::Standalone);
            return Doc::concat([
                Doc::group(Doc::concat([
                    Doc::str("if"),
                    Doc::indent(tab, Doc::concat([Doc::Line, expr_doc, Doc::str(" is do")])),
                ])),
                Doc::space(),
                arms_doc,
            ]);
        }

        let (header_group, then_block, then_slot, else_node, after_then_tok) = if kind
            == NodeKind::IfStatement
        {
            let cond = ch[0];
            let then_b = ch[1];
            let else_n = ch[2];
            let cond_slot = slot.child_slot_from(self.parsed, slot.first + 1, cond);
            let open_then = cond_slot.last + 1;
            let close_then = find_matching_delim(
                &self.parsed.sig_tokens,
                open_then,
                TokenKind::LBrace,
                TokenKind::RBrace,
            )
            .unwrap_or(slot.last);
            let cond_doc = self.format_expr(cond, cond_slot);
            let hdr = Doc::group(Doc::concat([
                Doc::str("if"),
                Doc::indent(tab, Doc::concat([Doc::Line, cond_doc])),
                Doc::Line,
            ]));
            (
                hdr,
                then_b,
                TokenSlot::new(open_then, close_then),
                else_n,
                close_then + 1,
            )
        } else {
            // `IfIsMatch`: `if expr is pat do { ... } else ...`
            let expr = ch[0];
            let pat = ch[1];
            let then_b = ch[2];
            let else_n = ch[3];
            let expr_slot = slot.child_slot_from(self.parsed, slot.first + 1, expr);
            let is_tok = self
                .parsed
                .find_depth_zero_token(expr_slot.last + 1, slot.last, |k| k == TokenKind::Is, false)
                .unwrap_or(expr_slot.last + 1);
            let pat_slot = slot.child_slot_from(self.parsed, is_tok + 1, pat);
            let do_tok = self
                .parsed
                .find_depth_zero_token(pat_slot.last + 1, slot.last, |k| k == TokenKind::Do, false)
                .unwrap_or(pat_slot.last + 1);
            let open_then = do_tok + 1;
            let close_then = find_matching_delim(
                &self.parsed.sig_tokens,
                open_then,
                TokenKind::LBrace,
                TokenKind::RBrace,
            )
            .unwrap_or(slot.last);
            let expr_doc = self.format_expr(expr, expr_slot);
            let pat_doc = self.format_pattern(pat, pat_slot);
            let hdr = Doc::group(Doc::concat([
                Doc::str("if"),
                Doc::indent(
                    tab,
                    Doc::concat([
                        Doc::Line,
                        expr_doc,
                        Doc::str(" is "),
                        pat_doc,
                        Doc::str(" do"),
                    ]),
                ),
                Doc::Line,
            ]));
            (
                hdr,
                then_b,
                TokenSlot::new(open_then, close_then),
                else_n,
                close_then + 1,
            )
        };

        let if_else_budget = if allow_single_line_if_else
            && else_node != 0
            && self.parsed.ast.get_node_kind(else_node) == Some(NodeKind::Block)
        {
            Some(self.cfg().width_heuristics().single_line_if_else_max_width)
        } else {
            None
        };

        let then_doc = self.format_block(
            then_block,
            then_slot,
            if_else_budget,
            ContainerKind::ValueBlock,
        );
        if else_node == 0 {
            return Doc::concat([header_group, then_doc]);
        }

        let else_tok = self
            .parsed
            .find_depth_zero_token(after_then_tok, slot.last, |k| k == TokenKind::Else, false)
            .unwrap_or(after_then_tok);
        let else_slot = TokenSlot::new(else_tok + 1, slot.last);
        let else_kind = self.parsed.ast.get_node_kind(else_node);

        match else_kind {
            Some(NodeKind::IfStatement | NodeKind::IfIsMatch | NodeKind::IfMatch) => {
                let else_if_doc =
                    self.format_if_family(else_node, else_kind.unwrap(), else_slot, false);
                Doc::concat([header_group, then_doc, Doc::str(" else "), else_if_doc])
            }
            Some(NodeKind::Block) => {
                let else_doc = self.format_block(
                    else_node,
                    else_slot,
                    if_else_budget,
                    ContainerKind::ValueBlock,
                );
                if let Some(budget) = if_else_budget {
                    Doc::group_with_budget(
                        budget,
                        Doc::concat([header_group, then_doc, Doc::str(" else "), else_doc]),
                    )
                } else {
                    Doc::concat([header_group, then_doc, Doc::str(" else "), else_doc])
                }
            }
            _ => {
                let else_doc = self.format_expr(else_node, else_slot);
                Doc::concat([header_group, then_doc, Doc::str(" else "), else_doc])
            }
        }
    }

    fn format_when_statement(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let arms = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
        let open_brace = slot.first + 1;
        let arms_doc =
            self.format_arms_block(arms, open_brace, slot.last, ExprPatternMode::Standalone);
        Doc::concat([Doc::str("when "), arms_doc])
    }

    fn format_while_family(&mut self, node: NodeIndex, kind: NodeKind, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let label = ch[0];
        let mut cursor = slot.first + 1; // after `while`
        let prefix = if label != 0 {
            // `:label` (`:` is at `cursor`, `label` is at `cursor + 1`)
            let lbl = self.decode_id(label, cursor + 1);
            cursor += 2;
            format!("while:{lbl}")
        } else {
            "while".to_string()
        };
        let tab = self.tab_spaces();

        match kind {
            NodeKind::WhileStatement => {
                let cond = ch[1];
                let block = ch[2];
                let cond_slot = slot.child_slot_from(self.parsed, cursor, cond);
                let block_slot = TokenSlot::new(cond_slot.last + 1, slot.last);
                let cond_doc = self.format_expr(cond, cond_slot);
                let b_doc = self.format_block(block, block_slot, None, ContainerKind::ValueBlock);
                Doc::concat([
                    Doc::group(Doc::concat([
                        Doc::text(prefix),
                        Doc::indent(tab, Doc::concat([Doc::Line, cond_doc])),
                        Doc::Line,
                    ])),
                    b_doc,
                ])
            }
            NodeKind::WhileIsMatch => {
                let expr = ch[1];
                let pat = ch[2];
                let block = ch[3];
                let expr_slot = slot.child_slot_from(self.parsed, cursor, expr);
                let is_tok = self
                    .parsed
                    .find_depth_zero_token(
                        expr_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Is,
                        false,
                    )
                    .unwrap_or(expr_slot.last + 1);
                let pat_slot = slot.child_slot_from(self.parsed, is_tok + 1, pat);
                let do_tok = self
                    .parsed
                    .find_depth_zero_token(
                        pat_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Do,
                        false,
                    )
                    .unwrap_or(pat_slot.last + 1);
                let block_slot = TokenSlot::new(do_tok + 1, slot.last);
                let expr_doc = self.format_expr(expr, expr_slot);
                let pat_doc = self.format_pattern(pat, pat_slot);
                let b_doc = self.format_block(block, block_slot, None, ContainerKind::ValueBlock);
                Doc::concat([
                    Doc::group(Doc::concat([
                        Doc::text(prefix),
                        Doc::indent(
                            tab,
                            Doc::concat([
                                Doc::Line,
                                expr_doc,
                                Doc::str(" is "),
                                pat_doc,
                                Doc::str(" do"),
                            ]),
                        ),
                        Doc::Line,
                    ])),
                    b_doc,
                ])
            }
            _ => {
                // `WhileMatch`: `while(:label)? expr is do { arms }`
                let expr = ch[1];
                let arms = self.parsed.ast.get_multi_child_slice(ch[2]).unwrap_or(&[]);
                let expr_slot = slot.child_slot_from(self.parsed, cursor, expr);
                let is_tok = self
                    .parsed
                    .find_depth_zero_token(
                        expr_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Is,
                        false,
                    )
                    .unwrap_or(expr_slot.last + 1);
                let open_brace = is_tok + 2; // after `is` + `do`
                let expr_doc = self.format_expr(expr, expr_slot);
                let arms_doc = self.format_arms_block(
                    arms,
                    open_brace,
                    slot.last,
                    ExprPatternMode::Standalone,
                );
                Doc::concat([
                    Doc::group(Doc::concat([
                        Doc::text(prefix),
                        Doc::indent(tab, Doc::concat([Doc::Line, expr_doc, Doc::str(" is do")])),
                    ])),
                    Doc::space(),
                    arms_doc,
                ])
            }
        }
    }

    fn format_for_statement(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let label = ch[0];
        let pat = ch[1];
        let iter_expr = ch[2];
        let block = ch[3];

        let mut cursor = slot.first + 1;
        let prefix = if label != 0 {
            let lbl = self.decode_id(label, cursor + 1);
            cursor += 2;
            format!("for:{lbl}")
        } else {
            "for".to_string()
        };

        let pat_slot = slot.child_slot_from(self.parsed, cursor, pat);
        let in_tok = self
            .parsed
            .find_depth_zero_token(pat_slot.last + 1, slot.last, |k| k == TokenKind::In, false)
            .unwrap_or(pat_slot.last + 1);
        let iter_slot = slot.child_slot_from(self.parsed, in_tok + 1, iter_expr);
        let block_slot = TokenSlot::new(iter_slot.last + 1, slot.last);

        let pat_doc = self.format_pattern(pat, pat_slot);
        let iter_doc = self.format_expr(iter_expr, iter_slot);
        let b_doc = self.format_block(block, block_slot, None, ContainerKind::ValueBlock);
        let tab = self.tab_spaces();

        Doc::concat([
            Doc::group(Doc::concat([
                Doc::text(prefix),
                Doc::indent(
                    tab,
                    Doc::concat([Doc::Line, pat_doc, Doc::str(" in "), iter_doc]),
                ),
                Doc::Line,
            ])),
            b_doc,
        ])
    }

    fn format_local_decl(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let kw = match kind {
            NodeKind::LetDecl => "let ",
            NodeKind::VarDecl => "var ",
            _ => "const ",
        };
        let ch = self.parsed.ast.get_children(node);
        let pat = ch[0];
        let ty = ch[1];
        let init = ch[2];
        let else_block = ch[3];

        let pat_slot = slot.child_slot_from(self.parsed, slot.first + 1, pat);
        let pat_doc = self.format_pattern(pat, pat_slot);
        let mut cursor = pat_slot.last + 1;

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

        if init == 0 {
            return (Doc::concat([Doc::str(kw), pat_doc, ty_doc]), false);
        }

        let eq_tok = self
            .parsed
            .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::Eq, false)
            .unwrap_or(cursor);
        let init_slot = slot.child_slot_from(self.parsed, eq_tok + 1, init);
        let (init_doc, init_ends_b) = self.format_expr_with_rbrace(init, init_slot);
        let tab = self.tab_spaces();

        if else_block != 0 {
            let else_tok = self
                .parsed
                .find_depth_zero_token(
                    init_slot.last + 1,
                    slot.last,
                    |k| k == TokenKind::Else,
                    false,
                )
                .unwrap_or(init_slot.last + 1);
            let else_slot = TokenSlot::new(else_tok + 1, slot.last);
            let let_else_budget =
                Some(self.cfg().width_heuristics().single_line_let_else_max_width);
            let else_doc = self.format_block_with_policy(
                else_block,
                else_slot,
                let_else_budget,
                true,
                ContainerKind::ValueBlock,
            );
            let doc = Doc::concat([
                Doc::str(kw),
                pat_doc,
                ty_doc,
                Doc::str(" = "),
                init_doc,
                Doc::str(" else "),
                else_doc,
            ]);
            return (doc, true);
        }

        if init_ends_b {
            (
                Doc::concat([Doc::str(kw), pat_doc, ty_doc, Doc::str(" = "), init_doc]),
                true,
            )
        } else {
            let doc = Doc::group(Doc::concat([
                Doc::str(kw),
                pat_doc,
                ty_doc,
                Doc::str(" ="),
                Doc::indent(tab, Doc::concat([Doc::Line, init_doc])),
            ]));
            (doc, false)
        }
    }

    fn format_jump_with_expr_and_guard(
        &mut self,
        kw: &'static str,
        val: NodeIndex,
        guard: NodeIndex,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let mut cursor = slot.first + 1;
        let mut parts = vec![Doc::str(kw)];
        let mut ends_b = false;
        if val != 0 {
            let val_slot = slot.child_slot_from(self.parsed, cursor, val);
            cursor = val_slot.last + 1;
            let (v_doc, eb) = self.format_expr_with_rbrace(val, val_slot);
            ends_b = eb && guard == 0;
            parts.push(Doc::space());
            parts.push(v_doc);
        }
        if guard != 0 {
            let while_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::While, false)
                .unwrap_or(cursor);
            let g_slot = slot.child_slot_from(self.parsed, while_tok + 1, guard);
            let (g_doc, eb) = self.format_expr_with_rbrace(guard, g_slot);
            ends_b = eb;
            parts.push(Doc::str(" while "));
            parts.push(g_doc);
        }
        (Doc::concat(parts), ends_b)
    }

    fn format_jump_with_label_and_guard(
        &mut self,
        kw: &'static str,
        label: NodeIndex,
        guard: NodeIndex,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let mut cursor = slot.first + 1;
        let mut parts = vec![Doc::str(kw)];
        let mut ends_b = false;
        if label != 0 {
            let lbl = self.decode_id(label, cursor);
            cursor += 1;
            parts.push(Doc::space());
            parts.push(Doc::text(lbl));
        }
        if guard != 0 {
            let while_tok = self
                .parsed
                .find_depth_zero_token(cursor, slot.last, |k| k == TokenKind::While, false)
                .unwrap_or(cursor);
            let g_slot = slot.child_slot_from(self.parsed, while_tok + 1, guard);
            let (g_doc, eb) = self.format_expr_with_rbrace(guard, g_slot);
            ends_b = eb;
            parts.push(Doc::str(" while "));
            parts.push(g_doc);
        }
        (Doc::concat(parts), ends_b)
    }

    pub fn format_verification_stmt_or_clause(
        &mut self,
        node: NodeIndex,
        kind: NodeKind,
        slot: TokenSlot,
    ) -> (Doc, bool) {
        let kw_byte = self.parsed.sig_tokens.get(slot.first).map_or(0, |t| t.from);
        let kw = if kind == NodeKind::Requires
            && self.parsed.bridge.outcomes_kw_offsets.contains(&kw_byte)
        {
            "outcomes"
        } else {
            match kind {
                NodeKind::Asserts => "asserts",
                NodeKind::Assumes => "assumes",
                NodeKind::Axiom => "axiom",
                NodeKind::Invariant => "invariant",
                NodeKind::Decreases => "decreases",
                NodeKind::Requires => "requires",
                NodeKind::Ensures => "ensures",
                _ => "requires",
            }
        };
        let ch = self.parsed.ast.get_children(node);
        let label = ch[0];
        let pred = ch[1];
        let mut cursor = slot.first + 1;
        let header = if label != 0 {
            let lbl = self.decode_id(label, cursor + 1);
            cursor += 2;
            format!("{kw}: {lbl}")
        } else {
            kw.to_string()
        };
        let pred_slot = slot.child_slot_from(self.parsed, cursor, pred);
        let (pred_doc, ends_b) = self.format_expr_with_rbrace(pred, pred_slot);
        let tab = self.tab_spaces();
        let doc = Doc::group(Doc::concat([
            Doc::text(header),
            Doc::indent(tab, Doc::concat([Doc::Line, pred_doc])),
        ]));
        (doc, ends_b)
    }

    /// Format `{ arm1, arm2, ... }` for `match`, `when`, `!`, `#`, `if ... is do`, and `while ... is do`.
    /// Expands multi-line whenever non-empty (except 1 simple arm inside a compact sub-expression if desired?
    /// In Flurry & Rust, non-empty match/when blocks expand multi-line cleanly, `{}` when empty).
    pub fn format_arms_block(
        &mut self,
        arms: &[NodeIndex],
        open_brace_tok: usize,
        close_brace_tok: usize,
        pat_mode: ExprPatternMode,
    ) -> Doc {
        let open_end = self
            .parsed
            .sig_tokens
            .get(open_brace_tok)
            .map_or(0, |t| t.to);
        let close_start = self
            .parsed
            .sig_tokens
            .get(close_brace_tok)
            .map_or(self.parsed.original_src.len(), |t| t.from);

        if arms.is_empty() {
            let (inner_c, _, _) = self.comments.take_leading_before(
                open_end,
                close_start,
                self.indent_spaces + self.tab_spaces(),
                self.brace_depth + 1,
                false,
                &mut self.cfg_stack,
            );
            if matches!(inner_c, Doc::Nil) {
                return Doc::str("{}");
            }
            let tab = self.tab_spaces();
            return Doc::concat([
                Doc::str("{"),
                Doc::indent(
                    tab,
                    Doc::concat([Doc::HardLine, strip_last_hardline(inner_c)]),
                ),
                Doc::HardLine,
                Doc::str("}"),
            ]);
        }

        // Precompute arm slots monotonically.
        let mut arm_slots = Vec::with_capacity(arms.len());
        let mut cursor = open_brace_tok + 1;
        let bound = TokenSlot::new(cursor, close_brace_tok.saturating_sub(1).max(cursor));
        for &arm in arms {
            let s = bound.child_slot_from(self.parsed, cursor, arm);
            let lo = self.parsed.sig_tokens[s.first].from;
            let hi = self.parsed.sig_tokens[s.last].to;
            let mut sep_end = hi;
            let mut next_tok = s.last + 1;
            if next_tok < close_brace_tok
                && self.parsed.sig_tokens[next_tok].kind == TokenKind::Comma
            {
                sep_end = self.parsed.sig_tokens[next_tok].to;
                next_tok += 1;
            }
            let end_line = self.parsed.line_of_byte(sep_end.saturating_sub(1));
            arm_slots.push((arm, s, lo, hi, sep_end, end_line));
            cursor = next_tok;
        }

        let tab = self.tab_spaces();
        let saved_indent = self.indent_spaces;
        self.indent_spaces += tab;
        self.brace_depth += 1;

        let mut docs: Vec<Doc> = Vec::with_capacity(arm_slots.len() * 3);
        let mut prev_end = open_end;

        for (i, &(arm_node, arm_slot, lo, hi, sep_end, end_line)) in arm_slots.iter().enumerate() {
            let (lead, _, dir_flags) = self.comments.take_leading_before(
                prev_end,
                lo,
                self.indent_spaces,
                self.brace_depth,
                i > 0,
                &mut self.cfg_stack,
            );
            if i > 0 {
                let user_blank = self.parsed.blank_lines_in_gap(prev_end, lo) > 0;
                if user_blank {
                    docs.push(Doc::BlankLine);
                } else {
                    docs.push(Doc::HardLine);
                }
            }
            if !matches!(lead, Doc::Nil) {
                docs.push(lead);
            }

            let arm_doc = if dir_flags.skip {
                self.format_skipped_bytes(lo, hi)
            } else if dir_flags.preserve {
                self.format_preserved_bytes(lo, hi)
            } else {
                self.format_single_arm(arm_node, arm_slot, pat_mode)
            };

            let next_limit = arm_slots.get(i + 1).map_or(close_start, |n| n.2);
            let cfg_now = self.cfg();
            let trailing = self.comments.take_same_line_trailing(
                end_line,
                next_limit,
                self.indent_spaces,
                &cfg_now,
            );

            docs.push(arm_doc);
            docs.push(Doc::str(","));
            if !matches!(trailing, Doc::Nil) {
                docs.push(trailing);
            }
            prev_end = sep_end;
        }

        let (tail_c, _, _) = self.comments.take_leading_before(
            prev_end,
            close_start,
            self.indent_spaces,
            self.brace_depth,
            true,
            &mut self.cfg_stack,
        );
        if !matches!(tail_c, Doc::Nil) {
            docs.push(Doc::HardLine);
            docs.push(strip_last_hardline(tail_c));
        }

        self.brace_depth -= 1;
        self.cfg_stack.pop_above(self.brace_depth);
        self.indent_spaces = saved_indent;

        Doc::concat([
            Doc::str("{"),
            Doc::indent(tab, Doc::concat([Doc::HardLine, Doc::concat(docs)])),
            Doc::HardLine,
            Doc::str("}"),
        ])
    }

    fn format_single_arm(
        &mut self,
        arm_node: NodeIndex,
        arm_slot: TokenSlot,
        pat_mode: ExprPatternMode,
    ) -> Doc {
        // Unwrap any `^attr` prefix chain on the arm!
        let (attrs_doc, inner_arm, inner_slot) = self.unwrap_arm_attributes(arm_node, arm_slot);
        let Some(kind) = self.parsed.ast.get_node_kind(inner_arm) else {
            return attrs_doc;
        };
        let ch = self.parsed.ast.get_children(inner_arm);
        let tab = self.tab_spaces();

        let (head_doc, body, body_slot) = match kind {
            NodeKind::ConditionArm => {
                // `[cond, body]`, where `cond == 0` is `else => body`!
                let cond = ch[0];
                let body = ch[1];
                if cond == 0 {
                    let fat = self
                        .parsed
                        .find_depth_zero_token(
                            inner_slot.first,
                            inner_slot.last,
                            |k| k == TokenKind::FatArrow,
                            false,
                        )
                        .unwrap_or(inner_slot.first + 1);
                    let b_slot = inner_slot.child_slot_from(self.parsed, fat + 1, body);
                    (Doc::str("else"), body, b_slot)
                } else {
                    let c_slot = inner_slot.child_slot_from(self.parsed, inner_slot.first, cond);
                    let fat = self
                        .parsed
                        .find_depth_zero_token(
                            c_slot.last + 1,
                            inner_slot.last,
                            |k| k == TokenKind::FatArrow,
                            false,
                        )
                        .unwrap_or(c_slot.last + 1);
                    let b_slot = inner_slot.child_slot_from(self.parsed, fat + 1, body);
                    (self.format_expr(cond, c_slot), body, b_slot)
                }
            }
            NodeKind::CatchArm => {
                let pat = ch[0];
                let body = ch[1];
                let p_slot = inner_slot.child_slot_from(self.parsed, inner_slot.first + 1, pat);
                let fat = self
                    .parsed
                    .find_depth_zero_token(
                        p_slot.last + 1,
                        inner_slot.last,
                        |k| k == TokenKind::FatArrow,
                        false,
                    )
                    .unwrap_or(p_slot.last + 1);
                let b_slot = inner_slot.child_slot_from(self.parsed, fat + 1, body);
                let p_doc = self.format_pattern(pat, p_slot);
                (Doc::concat([Doc::str("catch "), p_doc]), body, b_slot)
            }
            _ => {
                // `CaseArm`: `[pat, body]`
                let pat = ch[0];
                let body = ch[1];
                let p_slot = inner_slot.child_slot_from(self.parsed, inner_slot.first, pat);
                let fat = self
                    .parsed
                    .find_depth_zero_token(
                        p_slot.last + 1,
                        inner_slot.last,
                        |k| k == TokenKind::FatArrow,
                        false,
                    )
                    .unwrap_or(p_slot.last + 1);
                let b_slot = inner_slot.child_slot_from(self.parsed, fat + 1, body);
                let p_doc = self.format_pattern_with_mode(pat, p_slot, pat_mode);
                (p_doc, body, b_slot)
            }
        };

        // Never add or remove `{}` around arm bodies!
        let arm_body_doc = if self.parsed.ast.get_node_kind(body) == Some(NodeKind::Block) {
            let budget = Some(self.cfg().width_heuristics().fn_call_width);
            let b_doc = self.format_block(body, body_slot, budget, ContainerKind::ValueBlock);
            Doc::concat([head_doc, Doc::str(" => "), b_doc])
        } else {
            let b_doc = self.format_expr(body, body_slot);
            Doc::group(Doc::concat([
                head_doc,
                Doc::str(" =>"),
                Doc::indent(tab, Doc::concat([Doc::Line, b_doc])),
            ]))
        };

        Doc::concat([attrs_doc, arm_body_doc])
    }

    fn unwrap_arm_attributes(
        &mut self,
        mut node: NodeIndex,
        slot: TokenSlot,
    ) -> (Doc, NodeIndex, TokenSlot) {
        let mut attr_docs = Vec::new();
        let mut cursor = slot.first;
        while self.parsed.ast.get_node_kind(node) == Some(NodeKind::Attribute) {
            let ch = self.parsed.ast.get_children(node);
            let attr_expr = ch[0];
            let target = ch[1];
            let attr_slot = slot.child_slot_from(self.parsed, cursor + 1, attr_expr);
            let rendered = self.format_attribute_line(attr_expr, attr_slot);
            attr_docs.push(rendered);
            attr_docs.push(Doc::HardLine);
            cursor = attr_slot.last + 1;
            node = target;
        }
        (
            Doc::concat(attr_docs),
            node,
            TokenSlot::new(cursor, slot.last),
        )
    }
}

enum ChainStep {
    TightViews(Vec<String>),
    TightSigil(&'static str),
    Call {
        node: NodeIndex,
        open_tok: usize,
        close_tok: usize,
    },
    DiamondCall {
        node: NodeIndex,
        open_tok: usize,
        close_tok: usize,
    },
    Index {
        idx_expr: NodeIndex,
        open_tok: usize,
        close_tok: usize,
    },
    DotField(String),
    DotStatic(&'static str),
    DotParen {
        prefix: &'static str,
        arg_node: NodeIndex,
        arg_slot: TokenSlot,
    },
}

impl ChainStep {
    fn is_dot_link(&self) -> bool {
        matches!(
            self,
            Self::DotField(_) | Self::DotStatic(_) | Self::DotParen { .. }
        )
    }
}

fn binary_prec(kind: NodeKind) -> i32 {
    match kind {
        NodeKind::BoolImplies => 10,
        NodeKind::BoolOr => 20,
        NodeKind::BoolAnd => 30,
        NodeKind::BoolEq
        | NodeKind::BoolNotEq
        | NodeKind::BoolGt
        | NodeKind::BoolGtEq
        | NodeKind::BoolLt
        | NodeKind::BoolLtEq
        | NodeKind::TypedWith
        | NodeKind::Subtype
        | NodeKind::TraitBound
        | NodeKind::BoolMatches => 40,
        NodeKind::Arrow => 50,
        NodeKind::Add | NodeKind::Sub => 60,
        NodeKind::Mul | NodeKind::Div | NodeKind::Mod | NodeKind::AddAdd => 70,
        NodeKind::Pipe | NodeKind::PipePrepend => 80,
        _ => -1,
    }
}

fn is_right_assoc_binary(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Arrow
            | NodeKind::BoolImplies
            | NodeKind::TraitBound
            | NodeKind::Subtype
            | NodeKind::TypedWith
    )
}

fn can_single_line_block_elem(
    ast: &ast::Ast,
    node: NodeIndex,
    allow_stmt_in_single_line: bool,
) -> bool {
    let direct_kind = ast.get_node_kind(node);
    if !allow_stmt_in_single_line {
        if direct_kind == Some(NodeKind::ExprStatement)
            || requires_semi_in_single_line_block(direct_kind)
        {
            return false;
        }
    }
    let kind = if direct_kind == Some(NodeKind::ExprStatement) {
        ast.get_node_kind(ast.get_children(node)[0])
    } else {
        direct_kind
    };
    !matches!(
        kind,
        Some(
            NodeKind::IfStatement
                | NodeKind::IfIsMatch
                | NodeKind::IfMatch
                | NodeKind::WhenStatement
                | NodeKind::WhileStatement
                | NodeKind::WhileIsMatch
                | NodeKind::WhileMatch
                | NodeKind::ForStatement
                | NodeKind::Block
                | NodeKind::DoBlock
                | NodeKind::AsyncBlock
                | NodeKind::UnsafeBlock
                | NodeKind::ComptimeBlock
                | NodeKind::AtomicBlock
                | NodeKind::Function
                | NodeKind::NormalFormHeadDef
                | NodeKind::StructDef
                | NodeKind::EnumDef
                | NodeKind::UnionDef
                | NodeKind::TraitDef
                | NodeKind::ImplDef
                | NodeKind::ImplTraitDef
                | NodeKind::ExtendDef
                | NodeKind::ExtendTraitDef
                | NodeKind::ModuleDef
                | NodeKind::Attribute
                | NodeKind::Pub
                | NodeKind::Private
                | NodeKind::LetDecl
                | NodeKind::VarDecl
                | NodeKind::ConstDecl
                | NodeKind::ConstDef
                | NodeKind::TypealiasDef
                | NodeKind::NewtypeDef
                | NodeKind::UseStatement
                | NodeKind::ModStatement
                | NodeKind::DeriveDef
        )
    )
}

fn requires_semi_in_single_line_block(kind: Option<NodeKind>) -> bool {
    matches!(
        kind,
        Some(
            NodeKind::ReturnStatement
                | NodeKind::ResumeStatement
                | NodeKind::BreakStatement
                | NodeKind::ContinueStatement
                | NodeKind::LetDecl
                | NodeKind::VarDecl
                | NodeKind::ConstDecl
                | NodeKind::Assign
                | NodeKind::AddAssign
                | NodeKind::SubAssign
                | NodeKind::MulAssign
                | NodeKind::DivAssign
                | NodeKind::RemAssign
                | NodeKind::Asserts
                | NodeKind::Assumes
                | NodeKind::Axiom
                | NodeKind::Invariant
                | NodeKind::Decreases
                | NodeKind::Requires
                | NodeKind::Ensures
        )
    )
}
