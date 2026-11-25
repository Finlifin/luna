//! Pattern formatting for all Flurry pattern constructs, bit-vector patterns, and CMTT quotes.

use ast::{NodeIndex, NodeKind};
use lex::TokenKind;

use crate::doc::Doc;
use crate::formatter::{ExprPatternMode, Formatter, TokenSlot};

/// Delimiter style for comma-separated sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelimStyle {
    /// `(a, b)` — no inner spaces when flat.
    Parens,
    /// `(a,)` — 1-element tuple requiring trailing comma even when flat.
    SingleTupleParens,
    /// `[a, b]` — no inner spaces when flat.
    Brackets,
    /// `<a, b>` — no inner spaces when flat.
    Angles,
    /// `{ a, b }` — 1 space inside `{` and `}` when non-empty and flat; `{}` when empty.
    SpacedBraces,
    /// `{a, b}` — no spaces inside `{` and `}` when flat (used by `use a.{b, c}`).
    CompactBraces,
}

impl DelimStyle {
    fn open_str(self) -> &'static str {
        match self {
            Self::Parens | Self::SingleTupleParens => "(",
            Self::Brackets => "[",
            Self::Angles => "<",
            Self::SpacedBraces | Self::CompactBraces => "{",
        }
    }

    fn close_str(self) -> &'static str {
        match self {
            Self::Parens | Self::SingleTupleParens => ")",
            Self::Brackets => "]",
            Self::Angles => ">",
            Self::SpacedBraces | Self::CompactBraces => "}",
        }
    }

    fn close_tok_kind(self) -> TokenKind {
        match self {
            Self::Parens | Self::SingleTupleParens => TokenKind::RParen,
            Self::Brackets => TokenKind::RBracket,
            Self::Angles => TokenKind::Gt,
            Self::SpacedBraces | Self::CompactBraces => TokenKind::RBrace,
        }
    }
}

impl<'a> Formatter<'a> {
    /// Format a comma-separated delimited list `( ... )`, `[ ... ]`, `< ... >`, or `{ ... }`
    /// with trivia preservation, optional width budget, and vertical trailing-comma rule.
    pub fn format_delimited_seq<F>(
        &mut self,
        elems: &[NodeIndex],
        open_tok_idx: usize,
        close_tok_idx: usize,
        style: DelimStyle,
        flat_budget: Option<usize>,
        mut render_elem: F,
    ) -> Doc
    where
        F: FnMut(&mut Self, NodeIndex, TokenSlot) -> Doc,
    {
        let open_end_byte = self.parsed.sig_tokens.get(open_tok_idx).map_or(0, |t| t.to);
        let close_start_byte = self
            .parsed
            .sig_tokens
            .get(close_tok_idx)
            .map_or(self.parsed.original_src.len(), |t| t.from);

        if elems.is_empty() {
            let (inner_comments, _, _) = self.comments.take_leading_before(
                open_end_byte,
                close_start_byte,
                self.indent_spaces + self.tab_spaces(),
                self.brace_depth,
                false,
                &mut self.cfg_stack,
            );
            if matches!(inner_comments, Doc::Nil) {
                return Doc::concat([Doc::str(style.open_str()), Doc::str(style.close_str())]);
            }
            let tab = self.tab_spaces();
            return Doc::concat([
                Doc::str(style.open_str()),
                Doc::indent(tab, Doc::concat([Doc::HardLine, inner_comments])),
                Doc::str(style.close_str()),
            ]);
        }

        // Precompute element slots monotonically inside `(open_tok_idx + 1 .. close_tok_idx)`.
        let mut slots = Vec::with_capacity(elems.len());
        let mut cursor_tok = open_tok_idx + 1;
        let inner_bound = close_tok_idx.saturating_sub(1).max(cursor_tok);
        let outer = TokenSlot::new(cursor_tok, inner_bound);

        for &elem in elems {
            let s = outer.child_slot_from(self.parsed, cursor_tok, elem);
            let lo = self.parsed.sig_tokens[s.first].from;
            let mut hi = self.parsed.sig_tokens[s.last].to;
            if let Some(b) = self.parsed.bridge.constructs_by_start.get(&lo) {
                hi = hi.max(b.span_bytes().1);
            }
            let mut sep_end_byte = hi;
            let mut next_tok = s.last + 1;
            if next_tok < close_tok_idx && self.parsed.sig_tokens[next_tok].kind == TokenKind::Comma
            {
                sep_end_byte = self.parsed.sig_tokens[next_tok].to;
                next_tok += 1;
            }
            let end_line = self.parsed.line_of_byte(sep_end_byte.saturating_sub(1));
            slots.push((elem, s, lo, hi, sep_end_byte, end_line));
            cursor_tok = next_tok;
        }

        let tab = self.tab_spaces();
        let saved_indent = self.indent_spaces;
        self.indent_spaces += tab;
        if matches!(style, DelimStyle::SpacedBraces | DelimStyle::CompactBraces) {
            self.brace_depth += 1;
        }

        let mut item_docs: Vec<Doc> = Vec::with_capacity(elems.len() * 2);
        let mut prev_end = open_end_byte;
        let mut forced_multiline = false;
        let mut last_has_trailing = false;

        for (i, &(elem, elem_slot, lo, hi, sep_end, end_line)) in slots.iter().enumerate() {
            let (lead, _, dir_flags) = self.comments.take_leading_before(
                prev_end,
                lo,
                self.indent_spaces,
                self.brace_depth,
                i > 0,
                &mut self.cfg_stack,
            );
            if !matches!(lead, Doc::Nil) {
                forced_multiline = true;
            }
            if i > 0 {
                let user_blank = self.parsed.blank_lines_in_gap(prev_end, lo) > 0;
                if user_blank {
                    forced_multiline = true;
                    item_docs.push(Doc::BlankLine);
                } else {
                    item_docs.push(Doc::Line);
                }
            }
            if !matches!(lead, Doc::Nil) {
                item_docs.push(lead);
            }

            let rendered = if dir_flags.skip {
                forced_multiline = true;
                self.format_skipped_bytes(lo, hi)
            } else if dir_flags.preserve {
                forced_multiline = true;
                self.format_preserved_bytes(lo, hi)
            } else {
                render_elem(self, elem, elem_slot)
            };

            let is_last = i + 1 == slots.len();
            let comma_doc = if !is_last || style == DelimStyle::SingleTupleParens {
                Doc::str(",")
            } else if style == DelimStyle::Angles {
                Doc::Nil
            } else {
                Doc::trailing_comma()
            };

            let next_limit = slots.get(i + 1).map_or(close_start_byte, |n| n.2);
            let cfg_now = self.cfg();
            let trailing = self.comments.take_same_line_trailing(
                end_line,
                next_limit,
                self.indent_spaces,
                &cfg_now,
            );
            if !matches!(trailing, Doc::Nil) {
                forced_multiline = true;
                if is_last {
                    last_has_trailing = true;
                }
            }

            item_docs.push(rendered);
            item_docs.push(comma_doc);
            if !matches!(trailing, Doc::Nil) {
                item_docs.push(trailing);
            }
            prev_end = sep_end;
        }

        let (tail_comments, _, _) = self.comments.take_leading_before(
            prev_end,
            close_start_byte,
            self.indent_spaces,
            self.brace_depth,
            true,
            &mut self.cfg_stack,
        );
        let has_tail_comments = !matches!(tail_comments, Doc::Nil);
        if has_tail_comments {
            forced_multiline = true;
            item_docs.push(Doc::HardLine);
            // Strip final `HardLine` from `tail_comments` because outer break adds `SoftLine`/`Line`.
            item_docs.push(strip_last_hardline(tail_comments));
        }

        if matches!(style, DelimStyle::SpacedBraces | DelimStyle::CompactBraces) {
            self.brace_depth -= 1;
            self.cfg_stack.pop_above(self.brace_depth);
        }
        self.indent_spaces = saved_indent;

        let angles_no_close_break =
            style == DelimStyle::Angles && !has_tail_comments && !last_has_trailing;
        let open_break = if style == DelimStyle::SpacedBraces {
            Doc::Line
        } else {
            Doc::SoftLine
        };
        let close_break = if angles_no_close_break {
            Doc::Nil
        } else if style == DelimStyle::SpacedBraces {
            Doc::Line
        } else {
            Doc::SoftLine
        };

        let body = if forced_multiline {
            Doc::concat([
                Doc::str(style.open_str()),
                Doc::indent(tab, Doc::concat([Doc::HardLine, Doc::concat(item_docs)])),
                if angles_no_close_break {
                    Doc::Nil
                } else {
                    Doc::HardLine
                },
                Doc::str(style.close_str()),
            ])
        } else {
            let inner = Doc::concat([
                Doc::str(style.open_str()),
                Doc::indent(tab, Doc::concat([open_break, Doc::concat(item_docs)])),
                close_break,
                Doc::str(style.close_str()),
            ]);
            if let Some(budget) = flat_budget {
                Doc::group_with_budget(budget, inner)
            } else {
                Doc::group(inner)
            }
        };
        let _ = style.close_tok_kind();
        body
    }

    /// Format adjacent string literal concatenation (`"a" "b"` or multi-line aligned strings).
    pub fn format_string_concat(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        let slots = self.parsed.ast.get_children(node);
        let leaves = slots
            .first()
            .and_then(|&h| self.parsed.ast.get_multi_child_slice(h))
            .unwrap_or(&[]);
        if leaves.is_empty() {
            return Doc::text(self.literal_slice(node, slot.first));
        }
        let mut parts = Vec::with_capacity(leaves.len() * 2);
        let mut prev_hi = 0usize;
        let mut prev_line = 0usize;
        let mut cursor = slot.first;
        for (idx, &leaf) in leaves.iter().enumerate() {
            let (f, l) = self.parsed.exact_token_bounds(leaf, cursor);
            let lo = self.parsed.sig_tokens[f].from;
            let hi = self.parsed.sig_tokens[l].to;
            let line = self.parsed.line_of_byte(lo);
            if idx > 0 {
                let inline_c =
                    self.comments
                        .take_inline_between(prev_hi, lo, self.indent_spaces, &self.cfg());
                if !matches!(inline_c, Doc::Nil) {
                    parts.push(inline_c);
                } else if line > prev_line {
                    parts.push(Doc::HardLine);
                } else {
                    parts.push(Doc::space());
                }
            }
            parts.push(Doc::text(&self.parsed.original_src[lo..hi]));
            prev_hi = hi;
            prev_line = line;
            cursor = l + 1;
        }
        Doc::concat(parts)
    }

    /// Format optional CMTT `[a = b, c = d]` substitution bracket prefix on quote/splice forms.
    pub fn format_code_substitutions(
        &mut self,
        substs: &[NodeIndex],
        start_tok: usize,
        end_tok: usize,
    ) -> (Doc, usize) {
        if substs.is_empty() {
            return (Doc::Nil, start_tok);
        }
        // Locate `[` and matching `]` starting at `start_tok`.
        let open_idx = self
            .parsed
            .find_depth_zero_token(start_tok, end_tok, |k| k == TokenKind::LBracket, false)
            .unwrap_or(start_tok);
        let close_idx = find_matching_delim(
            &self.parsed.sig_tokens,
            open_idx,
            TokenKind::LBracket,
            TokenKind::RBracket,
        )
        .unwrap_or(end_tok);
        let doc = self.format_delimited_seq(
            substs,
            open_idx,
            close_idx,
            DelimStyle::Brackets,
            None,
            |fmt, sub_node, sub_slot| {
                let ch = fmt.parsed.ast.get_children(sub_node);
                let lhs = ch[0];
                let rhs = ch[1];
                let lhs_slot = sub_slot.child_slot_from(fmt.parsed, sub_slot.first, lhs);
                let eq_tok = fmt
                    .parsed
                    .find_depth_zero_token(
                        lhs_slot.last + 1,
                        sub_slot.last,
                        |k| k == TokenKind::Eq,
                        false,
                    )
                    .unwrap_or(lhs_slot.last + 1);
                let rhs_slot = sub_slot.child_slot_from(fmt.parsed, eq_tok + 1, rhs);
                let lhs_doc = fmt.format_expr(lhs, lhs_slot);
                let rhs_doc = fmt.format_expr(rhs, rhs_slot);
                Doc::concat([lhs_doc, Doc::str(" = "), rhs_doc])
            },
        );
        (doc, close_idx + 1)
    }

    /// Check whether the first token of `slot` starts with `-` (requiring a space after `..` / `..=` / `-`).
    pub fn slot_starts_with_minus(&self, slot: TokenSlot) -> bool {
        self.parsed
            .sig_tokens
            .get(slot.first)
            .is_some_and(|t| matches!(t.kind, TokenKind::Minus | TokenKind::SeparatedMinus))
    }

    /// Format any pattern node in default (`Standalone`) mode.
    pub fn format_pattern(&mut self, node: NodeIndex, slot: TokenSlot) -> Doc {
        self.format_pattern_with_mode(node, slot, ExprPatternMode::Standalone)
    }

    /// Format a pattern node with explicit `ExprPatternMode`.
    pub fn format_pattern_with_mode(
        &mut self,
        node: NodeIndex,
        slot: TokenSlot,
        mode: ExprPatternMode,
    ) -> Doc {
        if node == 0 {
            return Doc::Nil;
        }
        let (paren_count, inner_slot) = slot.unwrap_parens(self.parsed, node);
        if paren_count > 0 {
            // Inside explicit `( ... )`, `ExprAsPattern` is standalone (`( < expr > )`).
            let inner =
                self.format_pattern_unwrapped(node, inner_slot, ExprPatternMode::Standalone);
            let open = "(".repeat(paren_count);
            let close = ")".repeat(paren_count);
            return Doc::concat([Doc::text(open), inner, Doc::text(close)]);
        }
        self.format_pattern_unwrapped(node, inner_slot, mode)
    }

    fn format_pattern_unwrapped(
        &mut self,
        node: NodeIndex,
        slot: TokenSlot,
        mode: ExprPatternMode,
    ) -> Doc {
        if let Some(bridged) = self.bridged_at(node, slot.first) {
            return self.format_bridged(bridged);
        }

        let Some(kind) = self.parsed.ast.get_node_kind(node) else {
            return Doc::Nil;
        };
        let ch = self.parsed.ast.get_children(node);

        match kind {
            NodeKind::Id => Doc::text(self.decode_id(node, slot.first)),
            NodeKind::Int
            | NodeKind::Real
            | NodeKind::Str
            | NodeKind::Char
            | NodeKind::Bool
            | NodeKind::Null
            | NodeKind::Undefined
            | NodeKind::SelfLower
            | NodeKind::SelfCap
            | NodeKind::Wildcard => Doc::text(self.literal_slice(node, slot.first)),
            NodeKind::StringConcat => self.format_string_concat(node, slot),
            NodeKind::Unit => Doc::str("()"),
            NodeKind::RangeFull => Doc::str(".."),
            NodeKind::Symbol => {
                let id_str = self.decode_id(ch[0], slot.first);
                Doc::concat([Doc::str("."), Doc::text(id_str)])
            }
            NodeKind::Negative => {
                let inner = ch[0];
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, inner);
                let sep = if self.slot_starts_with_minus(sub_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let sub_doc = self.format_pattern(inner, sub_slot);
                Doc::concat([Doc::str("-"), sep, sub_doc])
            }
            NodeKind::RefPattern => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let sub = self.format_pattern(ch[0], sub_slot);
                Doc::concat([Doc::str("ref "), sub])
            }
            NodeKind::AsyncPattern => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let sub = self.format_pattern(ch[0], sub_slot);
                Doc::concat([Doc::str("async "), sub])
            }
            NodeKind::NotPattern => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let sub = self.format_pattern(ch[0], sub_slot);
                Doc::concat([Doc::str("not "), sub])
            }
            NodeKind::ErrorPattern => {
                let sub_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let sub = self.format_pattern(ch[0], sub_slot);
                Doc::concat([Doc::str("error "), sub])
            }
            NodeKind::TypeBindPattern => {
                let id_str = self.decode_id(ch[0], slot.first);
                Doc::concat([Doc::str("'"), Doc::text(id_str)])
            }
            NodeKind::OptionSomePattern => {
                let sub_slot =
                    TokenSlot::new(slot.first, slot.last.saturating_sub(1).max(slot.first));
                let sub = self.format_pattern(ch[0], sub_slot);
                Doc::concat([sub, Doc::str("?")])
            }
            NodeKind::ErrorOkPattern => {
                let sub_slot =
                    TokenSlot::new(slot.first, slot.last.saturating_sub(1).max(slot.first));
                let sub = self.format_pattern(ch[0], sub_slot);
                Doc::concat([sub, Doc::str("!")])
            }
            NodeKind::OrPattern => self.format_or_pattern(node, slot, mode),
            NodeKind::AsBindPattern => {
                let lhs_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let lhs_doc = self.format_pattern_with_mode(ch[0], lhs_slot, mode);
                let id_str = self.decode_id(ch[1], lhs_slot.last + 1);
                Doc::concat([lhs_doc, Doc::str(" as "), Doc::text(id_str)])
            }
            NodeKind::IfGuardPattern => {
                let pat_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let if_tok = self
                    .parsed
                    .find_depth_zero_token(
                        pat_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::If,
                        false,
                    )
                    .unwrap_or(pat_slot.last + 1);
                let guard_slot = slot.child_slot_from(self.parsed, if_tok + 1, ch[1]);
                let pat_doc = self.format_pattern_with_mode(ch[0], pat_slot, mode);
                let guard_doc = self.format_expr(ch[1], guard_slot);
                let tab = self.tab_spaces();
                Doc::group(Doc::concat([
                    pat_doc,
                    Doc::indent(tab, Doc::concat([Doc::Line, Doc::str("if "), guard_doc])),
                ]))
            }
            NodeKind::AndIsPattern => {
                let left_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let and_tok = self
                    .parsed
                    .find_depth_zero_token(
                        left_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::And,
                        false,
                    )
                    .unwrap_or(left_slot.last + 1);
                let expr_slot = slot.child_slot_from(self.parsed, and_tok + 1, ch[1]);
                let is_tok = self
                    .parsed
                    .find_depth_zero_token(
                        expr_slot.last + 1,
                        slot.last,
                        |k| k == TokenKind::Is,
                        false,
                    )
                    .unwrap_or(expr_slot.last + 1);
                let right_slot = slot.child_slot_from(self.parsed, is_tok + 1, ch[2]);
                let left_doc = self.format_pattern_with_mode(ch[0], left_slot, mode);
                let expr_doc = self.format_expr(ch[1], expr_slot);
                let right_doc = self.format_pattern(ch[2], right_slot);
                let tab = self.tab_spaces();
                Doc::group(Doc::concat([
                    left_doc,
                    Doc::indent(
                        tab,
                        Doc::concat([
                            Doc::Line,
                            Doc::str("and "),
                            expr_doc,
                            Doc::str(" is "),
                            right_doc,
                        ]),
                    ),
                ]))
            }
            NodeKind::RangeFromPattern => {
                let lo_slot =
                    TokenSlot::new(slot.first, slot.last.saturating_sub(1).max(slot.first));
                let lo_doc = self.format_pattern(ch[0], lo_slot);
                Doc::concat([lo_doc, Doc::str("..")])
            }
            NodeKind::RangeToPattern => {
                let hi_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let sep = if self.slot_starts_with_minus(hi_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let hi_doc = self.format_pattern(ch[0], hi_slot);
                Doc::concat([Doc::str(".."), sep, hi_doc])
            }
            NodeKind::RangeToInclusivePattern => {
                let hi_slot = slot.child_slot_from(self.parsed, slot.first + 1, ch[0]);
                let sep = if self.slot_starts_with_minus(hi_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let hi_doc = self.format_pattern(ch[0], hi_slot);
                Doc::concat([Doc::str("..="), sep, hi_doc])
            }
            NodeKind::RangeFromToPattern | NodeKind::RangeFromToInclusivePattern => {
                let op_str = if kind == NodeKind::RangeFromToInclusivePattern {
                    "..="
                } else {
                    ".."
                };
                let lo_slot = slot.child_slot_from(self.parsed, slot.first, ch[0]);
                let op_tok = lo_slot.last + 1;
                let hi_slot = slot.child_slot_from(self.parsed, op_tok + 1, ch[1]);
                let sep = if self.slot_starts_with_minus(hi_slot) {
                    Doc::space()
                } else {
                    Doc::Nil
                };
                let lo_doc = self.format_pattern(ch[0], lo_slot);
                let hi_doc = self.format_pattern(ch[1], hi_slot);
                Doc::concat([lo_doc, Doc::str(op_str), sep, hi_doc])
            }
            NodeKind::PropertyPattern => {
                let id_str = self.decode_id(ch[0], slot.first);
                let colon_tok = self
                    .parsed
                    .find_depth_zero_token(slot.first, slot.last, |k| k == TokenKind::Colon, false)
                    .unwrap_or(slot.first + 1);
                let pat_slot = slot.child_slot_from(self.parsed, colon_tok + 1, ch[1]);
                let pat_doc = self.format_pattern(ch[1], pat_slot);
                Doc::concat([Doc::text(id_str), Doc::str(": "), pat_doc])
            }
            NodeKind::ListRestPattern => {
                let target = ch.first().copied().unwrap_or(0);
                if target == 0 {
                    Doc::str("...")
                } else {
                    let sub_slot = slot.child_slot_from(self.parsed, slot.first + 3, target);
                    let sub_doc = self.format_pattern(target, sub_slot);
                    Doc::concat([Doc::str("..."), sub_doc])
                }
            }
            NodeKind::StructRestPattern => Doc::str("..."),
            NodeKind::TuplePattern => {
                let elems = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
                let style = if elems.len() == 1 {
                    DelimStyle::SingleTupleParens
                } else {
                    DelimStyle::Parens
                };
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                self.format_delimited_seq(
                    elems,
                    slot.first,
                    slot.last,
                    style,
                    budget,
                    |fmt, el, el_slot| fmt.format_pattern(el, el_slot),
                )
            }
            NodeKind::ListPattern => {
                let elems = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
                let budget = Some(self.cfg().width_heuristics().array_width);
                self.format_delimited_seq(
                    elems,
                    slot.first,
                    slot.last,
                    DelimStyle::Brackets,
                    budget,
                    |fmt, el, el_slot| fmt.format_pattern(el, el_slot),
                )
            }
            NodeKind::StructPattern => {
                let elems = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
                let budget = Some(self.cfg().width_heuristics().struct_lit_width);
                self.format_delimited_seq(
                    elems,
                    slot.first,
                    slot.last,
                    DelimStyle::SpacedBraces,
                    budget,
                    |fmt, el, el_slot| fmt.format_pattern(el, el_slot),
                )
            }
            NodeKind::ApplicationPattern => {
                let callee = ch[0];
                let args = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let callee_slot = slot.child_slot_from(self.parsed, slot.first, callee);
                let open_paren = callee_slot.last + 1;
                let callee_doc = self.format_pattern_with_mode(callee, callee_slot, mode);
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                let args_doc = self.format_delimited_seq(
                    args,
                    open_paren,
                    slot.last,
                    DelimStyle::Parens,
                    budget,
                    |fmt, el, el_slot| fmt.format_pattern(el, el_slot),
                );
                Doc::concat([callee_doc, args_doc])
            }
            NodeKind::ExtendedApplicationPattern => {
                let callee = ch[0];
                let fields = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let callee_slot = slot.child_slot_from(self.parsed, slot.first, callee);
                let open_brace = callee_slot.last + 1;
                let callee_doc = self.format_pattern(callee, callee_slot);
                let budget = Some(self.cfg().width_heuristics().struct_lit_width);
                let fields_doc = self.format_delimited_seq(
                    fields,
                    open_brace,
                    slot.last,
                    DelimStyle::SpacedBraces,
                    budget,
                    |fmt, el, el_slot| fmt.format_pattern(el, el_slot),
                );
                Doc::concat([callee_doc, Doc::space(), fields_doc])
            }
            NodeKind::NormalFormHeadApplicationPattern => {
                let callee = ch[0];
                let args = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let callee_slot = slot.child_slot_from(self.parsed, slot.first, callee);
                let open_lt = callee_slot.last + 1;
                let callee_doc = self.format_expr(callee, callee_slot);
                let budget = Some(self.cfg().width_heuristics().fn_call_width);
                let args_doc = self.format_delimited_seq(
                    args,
                    open_lt,
                    slot.last,
                    DelimStyle::Angles,
                    budget,
                    |fmt, el, el_slot| fmt.format_pattern(el, el_slot),
                );
                Doc::concat([callee_doc, args_doc])
            }
            NodeKind::ExprAsPattern => {
                let inner_expr = ch[0];
                match mode {
                    ExprPatternMode::BareEffectOp => self.format_expr(inner_expr, slot),
                    ExprPatternMode::BitVecSegment => {
                        let sub_slot = TokenSlot::new(
                            (slot.first + 1).min(slot.last),
                            slot.last.saturating_sub(1).max(slot.first),
                        );
                        let expr_doc = self.format_expr(inner_expr, sub_slot);
                        Doc::concat([Doc::str("<"), expr_doc, Doc::str(">")])
                    }
                    ExprPatternMode::Standalone => {
                        let sub_slot = TokenSlot::new(
                            (slot.first + 1).min(slot.last),
                            slot.last.saturating_sub(1).max(slot.first),
                        );
                        let expr_doc = self.format_expr(inner_expr, sub_slot);
                        Doc::concat([
                            Doc::EnsureLeadingWs,
                            Doc::str("< "),
                            expr_doc,
                            Doc::str(" >"),
                            Doc::EnsureTrailingWs,
                        ])
                    }
                }
            }
            NodeKind::BitVecBinPattern
            | NodeKind::BitVecOctPattern
            | NodeKind::BitVecHexPattern => self.format_bitvec_pattern(node, kind, slot),
            NodeKind::QuotePattern => {
                let body = ch[0];
                let substs = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                // Leading `'` is `slot.first`.
                let (subst_doc, after_subst) =
                    self.format_code_substitutions(substs, slot.first + 1, slot.last);
                // `<` is at `after_subst`, `>` is at `slot.last`.
                let pat_slot = TokenSlot::new(
                    (after_subst + 1).min(slot.last),
                    slot.last.saturating_sub(1).max(after_subst),
                );
                let pat_doc = self.format_pattern(body, pat_slot);
                Doc::concat([
                    Doc::str("'"),
                    subst_doc,
                    Doc::str("<"),
                    pat_doc,
                    Doc::str(">"),
                ])
            }
            NodeKind::SplicePattern => {
                let operand = ch[0];
                let substs = self.parsed.ast.get_multi_child_slice(ch[1]).unwrap_or(&[]);
                let (subst_doc, after_subst) =
                    self.format_code_substitutions(substs, slot.first + 1, slot.last);
                let expr_slot = TokenSlot::new(
                    (after_subst + 1).min(slot.last),
                    slot.last.saturating_sub(1).max(after_subst),
                );
                let expr_doc = self.format_expr(operand, expr_slot);
                Doc::concat([
                    Doc::str("~"),
                    subst_doc,
                    Doc::str("<"),
                    expr_doc,
                    Doc::str(">"),
                ])
            }
            // Path patterns like `Foo.Bar` are parsed as `NodeKind::Projection`!
            _ => self.format_expr(node, slot),
        }
    }

    fn collect_or_pattern_arms(
        &self,
        node: NodeIndex,
        slot: TokenSlot,
        is_root: bool,
        out: &mut Vec<(NodeIndex, TokenSlot)>,
    ) {
        let (parens, inner) = slot.unwrap_parens(self.parsed, node);
        if parens > 0 && !is_root {
            out.push((node, slot));
            return;
        }
        if self.parsed.ast.get_node_kind(node) != Some(NodeKind::OrPattern) {
            out.push((node, slot));
            return;
        }
        let ch = self.parsed.ast.get_children(node);
        let lhs = ch[0];
        let rhs = ch[1];
        let lhs_slot = inner.child_slot_from(self.parsed, inner.first, lhs);
        let or_tok = self
            .parsed
            .find_depth_zero_token(lhs_slot.last + 1, inner.last, |k| k == TokenKind::Or, false)
            .unwrap_or(lhs_slot.last + 1);
        let rhs_slot = TokenSlot::new(or_tok + 1, inner.last);
        self.collect_or_pattern_arms(lhs, lhs_slot, false, out);
        self.collect_or_pattern_arms(rhs, rhs_slot, false, out);
    }

    fn format_or_pattern(
        &mut self,
        node: NodeIndex,
        slot: TokenSlot,
        mode: ExprPatternMode,
    ) -> Doc {
        // Flatten `OrPattern` tree across both left and right spines (unless explicitly parenthesized).
        let mut arms: Vec<(NodeIndex, TokenSlot)> = Vec::new();
        self.collect_or_pattern_arms(node, slot, true, &mut arms);

        let mut docs = Vec::with_capacity(arms.len() * 2);
        for (i, (arm_node, arm_slot)) in arms.into_iter().enumerate() {
            if i > 0 {
                docs.push(Doc::Line);
                docs.push(Doc::str("or "));
            }
            docs.push(self.format_pattern_with_mode(arm_node, arm_slot, mode));
        }
        Doc::group(Doc::concat(docs))
    }

    fn format_bitvec_pattern(&mut self, node: NodeIndex, kind: NodeKind, slot: TokenSlot) -> Doc {
        let ch = self.parsed.ast.get_children(node);
        let segs = self.parsed.ast.get_multi_child_slice(ch[0]).unwrap_or(&[]);
        let prefix = match kind {
            NodeKind::BitVecBinPattern => "0b",
            NodeKind::BitVecOctPattern => "0o",
            _ => "0x",
        };
        if segs.is_empty() {
            return Doc::text(self.literal_slice(node, slot.first));
        }

        let mut docs = Vec::new();
        let mut cursor = slot.first;
        let first_tok = self.parsed.sig_tokens[slot.first];
        let standalone_prefix = matches!(
            first_tok.kind,
            TokenKind::BitBinPrefix | TokenKind::BitOctPrefix | TokenKind::BitHexPrefix
        );
        let mut prev_hi = if standalone_prefix {
            docs.push(Doc::str(prefix));
            cursor += 1;
            first_tok.to
        } else {
            0
        };

        for (i, &seg) in segs.iter().enumerate() {
            let seg_slot = slot.child_slot_from(self.parsed, cursor, seg);
            let seg_lo = self.parsed.sig_tokens[seg_slot.first].from;
            let seg_hi = self.parsed.sig_tokens[seg_slot.last].to;
            if (i > 0 || standalone_prefix) && seg_lo > prev_hi {
                // Preserve 1 space when original source separated adjacent bitvec segments.
                docs.push(Doc::space());
            }
            let seg_kind = self.parsed.ast.get_node_kind(seg);
            let seg_doc = match seg_kind {
                Some(NodeKind::Int) => Doc::text(self.literal_slice(seg, seg_slot.first)),
                Some(NodeKind::Id) => Doc::text(self.decode_id(seg, seg_slot.first)),
                Some(NodeKind::TypeBoundDeclClause) => {
                    // `(pattern: type)` segment inside bitvec pattern.
                    let seg_ch = self.parsed.ast.get_children(seg);
                    let inner_open = seg_slot.first + 1;
                    let inner_close = seg_slot.last.saturating_sub(1).max(inner_open);
                    let inner_bound = TokenSlot::new(inner_open, inner_close);
                    let pat_slot = inner_bound.child_slot_from(self.parsed, inner_open, seg_ch[0]);
                    let colon_tok = self
                        .parsed
                        .find_depth_zero_token(
                            pat_slot.last + 1,
                            inner_close,
                            |k| k == TokenKind::Colon,
                            false,
                        )
                        .unwrap_or(pat_slot.last + 1);
                    let ty_slot =
                        inner_bound.child_slot_from(self.parsed, colon_tok + 1, seg_ch[1]);
                    let pat_doc = self.format_pattern(seg_ch[0], pat_slot);
                    let ty_doc = self.format_expr(seg_ch[1], ty_slot);
                    Doc::concat([
                        Doc::str("("),
                        pat_doc,
                        Doc::str(": "),
                        ty_doc,
                        Doc::str(")"),
                    ])
                }
                Some(NodeKind::ExprAsPattern) => {
                    self.format_pattern_with_mode(seg, seg_slot, ExprPatternMode::BitVecSegment)
                }
                _ => self.format_pattern(seg, seg_slot),
            };
            docs.push(seg_doc);
            prev_hi = seg_hi;
            cursor = seg_slot.last + 1;
        }

        Doc::concat(docs)
    }
}

pub(crate) fn find_matching_delim(
    tokens: &[lex::Token],
    open_idx: usize,
    open_kind: TokenKind,
    close_kind: TokenKind,
) -> Option<usize> {
    if tokens.get(open_idx)?.kind != open_kind {
        return None;
    }
    let mut depth = 0usize;
    for (idx, tok) in tokens.iter().enumerate().skip(open_idx) {
        if tok.kind == open_kind {
            depth += 1;
        } else if tok.kind == close_kind {
            depth -= 1;
            if depth == 0 {
                return Some(idx);
            }
        } else if tok.kind == TokenKind::Eof {
            break;
        }
    }
    None
}

pub(crate) fn strip_last_hardline(doc: Doc) -> Doc {
    match doc {
        Doc::Concat(mut items) => {
            if matches!(items.last(), Some(Doc::HardLine)) {
                items.pop();
            }
            Doc::concat(items)
        }
        other => other,
    }
}
