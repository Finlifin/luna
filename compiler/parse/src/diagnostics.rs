use crate::error::*;
use crate::expressions::ExprOption;
use crate::parser::{ParseResult, Parser};
use crate::patterns::PatternOption;
use ast::{NodeIndex, NodeKind};
use lex::{Token, TokenKind};
use rustc_span::{BytePos, Span};

impl Parser<'_> {
    /// Snippet for an AST node, trimmed and shortened if multi-line so hints stay readable.
    pub(crate) fn snippet_or(&self, node: NodeIndex, fallback: &str) -> String {
        let Some(span) = self.ast.get_span(node) else {
            return fallback.to_string();
        };
        self.span_snippet_compact(span)
            .unwrap_or_else(|| fallback.to_string())
    }

    pub(crate) fn span_snippet_compact(&self, span: Span) -> Option<String> {
        let raw = self.source_map.span_to_snippet(span).ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.contains('\n') || trimmed.len() > 48 {
            let first = trimmed.lines().next().unwrap_or(trimmed).trim();
            let head: String = first.chars().take(32).collect();
            Some(format!("{head}..."))
        } else {
            Some(trimmed.to_string())
        }
    }

    /// True if two tokens are immediately adjacent in source bytes (`t1.to == t2.from`).
    pub(crate) fn tokens_adjacent(t1: Token, t2: Token) -> bool {
        t1.to == t2.from
    }

    /// True if two tokens sit on the same source line.
    pub(crate) fn tokens_on_same_line(&self, t1: Token, t2: Token) -> bool {
        if t2.from <= t1.to {
            return true;
        }
        let between = Span::new(
            self.start_pos + BytePos(t1.to as u32),
            self.start_pos + BytePos(t2.from as u32),
        );
        self.source_map
            .span_to_snippet(between)
            .map(|s| !s.contains('\n'))
            .unwrap_or(true)
    }

    /// Inspect whitespace immediately before and after `token` in the file.
    pub(crate) fn spacing_around(&self, token: Token) -> (bool, bool) {
        let left_space = if token.from == 0 {
            false
        } else {
            let sp = Span::new(
                self.start_pos + BytePos((token.from - 1) as u32),
                self.start_pos + BytePos(token.from as u32),
            );
            self.source_map
                .span_to_snippet(sp)
                .map(|s| s.chars().all(char::is_whitespace))
                .unwrap_or(false)
        };
        let right_space = {
            let sp = Span::new(
                self.start_pos + BytePos(token.to as u32),
                self.start_pos + BytePos((token.to + 1) as u32),
            );
            self.source_map
                .span_to_snippet(sp)
                .map(|s| !s.is_empty() && s.chars().all(char::is_whitespace))
                .unwrap_or(false)
        };
        (left_space, right_space)
    }

    // =========================================================================
    // 1. Flurry-Sensitive Syntax & Foreign-Language Habit Heuristics (Infix/Post)
    // =========================================================================

    /// Check if tokens right after `left` in expression Pratt parsing form a
    /// whitespace-sensitive operator mistake or a habit from another language.
    pub(crate) fn check_post_expr_heuristics(&self, left: NodeIndex) -> Option<ParseError> {
        let t1 = self.peek_next_token();
        let t2 = self.peek_nth_token(2);
        let span1 = self.token_span(t1);
        let lhs_snip = self.snippet_or(left, "lhs");

        // C/Rust/JS `//` or `/*` comments right after an expression
        if let Some(err) = self.check_c_style_comment_tokens() {
            return Some(err);
        }

        match t1.kind {
            // `&&` -> `and`
            TokenKind::Ampersand if t2.kind == TokenKind::Ampersand && Self::tokens_adjacent(t1, t2) => {
                let both_span = Span::new(span1.lo(), self.token_span(t2).hi());
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_logical_and",
                        "boolean conjunction in Flurry is written `and` rather than `&&`",
                        both_span,
                    )
                    .with_primary_label("replace `&&` with `and`")
                    .with_help(format!("Write `{lhs_snip} and ...` instead of `{lhs_snip} && ...`.")),
                )
            }

            // Single `&` in infix position -> bitwise AND habit
            TokenKind::Ampersand if self.current_token().kind != TokenKind::RBrace => Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_bitwise_and",
                    "`&` is not a binary bitwise operator in Flurry",
                    span1,
                )
                .with_primary_label("not a binary operator in Flurry")
                .with_note("In Flurry, prefix `&s T` writes a reachability-qualified type, and postfix `.ref` takes a reference.")
                .with_help(format!(
                    "For boolean logic, write `{lhs_snip} and rhs`. For bitwise AND, use `BitVec` (`{lhs_snip}'bitvec and rhs'bitvec`) or `math.bit_and({lhs_snip}, rhs)`."
                )),
            ),

            // `||` in infix position -> `or`
            TokenKind::Pipe if t2.kind == TokenKind::Pipe && Self::tokens_adjacent(t1, t2) => {
                let both_span = Span::new(span1.lo(), self.token_span(t2).hi());
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_logical_or",
                        "boolean disjunction in Flurry is written `or` rather than `||`",
                        both_span,
                    )
                    .with_primary_label("replace `||` with `or`")
                    .with_note("A single `|` between types writes a union type `A | B`.")
                    .with_help(format!("Write `{lhs_snip} or ...` for boolean OR.")),
                )
            }

            // `!==` (JS/TS strict inequality habit; note `===` is Flurry's `BoolImplies` operator!)
            TokenKind::BangEq if t2.kind == TokenKind::Eq && Self::tokens_adjacent(t1, t2) => {
                let full_span = Span::new(span1.lo(), self.token_span(t2).hi());
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_strict_ne",
                        "inequality in Flurry is written `!=` rather than `!==`",
                        full_span,
                    )
                    .with_primary_label("replace `!==` with `!=`")
                    .with_help(format!("Write `{lhs_snip} != ...`.")),
                )
            }

            // `::` path separator -> `.`
            TokenKind::ColonColon => Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_colon_colon",
                    "Flurry uses `.` for module, type, and member paths instead of `::`",
                    span1,
                )
                .with_primary_label("replace `::` with `.`")
                .with_help(format!("Write `{lhs_snip}.member` with a dot.")),
            ),

            // `x as T` in expression -> `x.as(T)`
            TokenKind::As => Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_infix_as",
                    "`as` in Flurry binds pattern names or import aliases, not infix type casts",
                    span1,
                )
                .with_primary_label("infix `as` is not an expression operator")
                .with_help(format!(
                    "Use postfix `.as(...)` for conversions: `{lhs_snip}.as(TargetType)`."
                )),
            ),

            // Infix `^` on the same line -> bitwise XOR habit (`^` is prefix attribute)
            TokenKind::Caret
                if self.current_token().kind != TokenKind::RBrace
                    && self.tokens_on_same_line(self.current_token(), t1) =>
            {
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_bitwise_xor",
                        "`^` introduces a prefix attribute (`^attr item`), not binary XOR or exponentiation",
                        span1,
                    )
                    .with_primary_label("`^` is a prefix attribute marker, not an infix operator")
                    .with_help(format!(
                        "For bitwise XOR, use `math.bit_xor({lhs_snip}, rhs)`; for powers, use `math.pow({lhs_snip}, exp)`."
                    )),
                )
            }

            // `<<` bit-shift habit (`a << b` lexes as `SeparatedLt`+`Lt` or `Lt`+`Lt` when adjacent)
            TokenKind::Lt | TokenKind::SeparatedLt
                if matches!(t2.kind, TokenKind::Lt | TokenKind::SeparatedLt | TokenKind::LtEq)
                    && Self::tokens_adjacent(t1, t2) =>
            {
                let both_span = Span::new(span1.lo(), self.token_span(t2).hi());
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_shift_left",
                        "Flurry does not have a `<<` bit-shift operator",
                        both_span,
                    )
                    .with_primary_label("`<<` is not an operator in Flurry")
                    .with_help(
                        "Use shift functions (such as `math.fshl` or integer shift methods) or `BitVec` operations instead.",
                    ),
                )
            }

            // `>>` bit-shift habit (only outside `< ... >` angle brackets so `List<List<T>>` works!)
            TokenKind::Gt | TokenKind::SeparatedGt
                if self.angle_bracket_depth == 0
                    && matches!(t2.kind, TokenKind::Gt | TokenKind::SeparatedGt | TokenKind::GtEq)
                    && Self::tokens_adjacent(t1, t2) =>
            {
                let both_span = Span::new(span1.lo(), self.token_span(t2).hi());
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_shift_right",
                        "Flurry does not have a `>>` bit-shift operator",
                        both_span,
                    )
                    .with_primary_label("`>>` is not an operator in Flurry")
                    .with_help(
                        "Use shift functions (such as `math.fshr` or integer shift methods) or `BitVec` operations instead.",
                    ),
                )
            }

            // Unseparated `+`, `-`, `*`, `/`, `%`, and `>` (outside `< ... >`)
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent => self.diagnose_unseparated_infix_op(left, t1),

            TokenKind::Gt if self.angle_bracket_depth == 0 => {
                if self.token_can_start_operand(t2.kind) {
                    self.diagnose_unseparated_infix_op(left, t1)
                } else {
                    Some(
                        ParseError::custom(
                            PARSE_UNEXPECTED_TOKEN,
                            "unmatched_gt",
                            "unmatched `>` with no opening `<` before it",
                            span1,
                        )
                        .with_primary_label("no matching `<` for this `>`")
                        .with_help(format!(
                            "For a greater-than comparison, put whitespace on both sides: `{lhs_snip} > rhs`."
                        )),
                    )
                }
            }

            _ => None,
        }
    }

    fn token_can_start_operand(&self, kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Id
                | TokenKind::Int
                | TokenKind::IntBin
                | TokenKind::IntOct
                | TokenKind::IntHex
                | TokenKind::Real
                | TokenKind::RealSci
                | TokenKind::Str
                | TokenKind::Char
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::SelfLower
                | TokenKind::Itself
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::Minus
                | TokenKind::Not
        )
    }

    fn diagnose_unseparated_infix_op(&self, left: NodeIndex, op_tok: Token) -> Option<ParseError> {
        let prev = self.current_token();
        let on_same_line = self.tokens_on_same_line(prev, op_tok);

        // Preserve statement boundaries like `if ready {} -value;` and `if ready {} *T;`
        // where `prev` is `}` and `-` / `*` starts the next statement!
        if matches!(
            op_tok.kind,
            TokenKind::Minus | TokenKind::Star | TokenKind::Percent
        ) && (prev.kind == TokenKind::RBrace || !on_same_line)
        {
            //Wait: check if `x\n*y` inside `{ x *y }` vs `{ if ready {} -value; }`!
            // If `prev` is `}`, `if ready {}` ended with `}` so `-value` / `*T` is a new statement.
            if prev.kind == TokenKind::RBrace {
                return None;
            }
        }

        let glyph = match op_tok.kind {
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::Gt => ">",
            TokenKind::Lt => "<",
            _ => return None,
        };
        let (left_space, right_space) = self.spacing_around(op_tok);
        let side_detail = match (left_space, right_space) {
            (false, false) => format!("no spaces on either side of `{glyph}`"),
            (false, true) => format!("missing whitespace before `{glyph}`"),
            (true, false) => format!("missing whitespace after `{glyph}`"),
            (true, true) => format!("`{glyph}` needs whitespace on both sides"),
        };

        let lhs_snip = self.snippet_or(left, "lhs");
        let next_snip = self
            .token_content(&self.peek_nth_token(2))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "rhs".to_string());

        let why_note = match op_tok.kind {
            TokenKind::Minus => Some(
                "Flurry distinguishes binary subtraction `a - b` from tight unary negation `-x` by surrounding whitespace.",
            ),
            TokenKind::Star => Some(
                "Flurry distinguishes binary multiplication `a * b` from pointer types `*T` and dereference `p.*` by surrounding whitespace.",
            ),
            TokenKind::Percent => Some(
                "Flurry distinguishes binary remainder `a % b` from closure-qualified types `%ctx T` by surrounding whitespace.",
            ),
            TokenKind::Lt | TokenKind::Gt => Some(
                "Flurry distinguishes `<` / `>` comparisons (`a < b`, `a > b`) from normal-form angle brackets `Head<...>` by surrounding whitespace.",
            ),
            _ => None,
        };

        let span = self.token_span(op_tok);
        let mut err = ParseError::custom(
            PARSE_SEPARATED_OPERATOR,
            "unseparated_infix_operator",
            format!("infix `{glyph}` requires whitespace on both sides ({side_detail})"),
            span,
        )
        .with_found(op_tok.kind)
        .with_primary_label(format!("put spaces on both sides of `{glyph}`"))
        .with_help(format!("Write `{lhs_snip} {glyph} {next_snip}`."));

        if let Some(note) = why_note {
            err = err.with_note(note);
        }
        Some(err)
    }

    /// Enhance an error when `<` after `left` failed to find a closing `>`:
    /// very often the user wrote `a<b`, `a< b`, or `a <b` intending comparison `a < b`!
    pub(crate) fn refine_failed_normal_form_lt(
        &mut self,
        left: NodeIndex,
        lt_cursor: usize,
        original_err: ParseError,
    ) -> ParseError {
        let lt_tok = self.get_token(lt_cursor);
        let lt_span = self.token_span(lt_tok);
        let lhs_snip = self.snippet_or(left, "lhs");

        // Scan forward from right after `<` at depth 0 to see if we hit a comparison
        // terminator (`{`, `;`, `)`, `and`, `or`, etc.) before any matching `>`!
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut lt_depth = 0usize;
        let mut term_idx: Option<usize> = None;

        for idx in (lt_cursor + 1)..self.tokens.len() {
            let tk = self.tokens[idx].kind;
            if paren_depth == 0 && bracket_depth == 0 && lt_depth == 0 {
                if tk == TokenKind::Gt {
                    break;
                }
                if matches!(
                    tk,
                    TokenKind::LBrace
                        | TokenKind::Semi
                        | TokenKind::RParen
                        | TokenKind::RBracket
                        | TokenKind::RBrace
                        | TokenKind::And
                        | TokenKind::Or
                        | TokenKind::EqEq
                        | TokenKind::BangEq
                        | TokenKind::FatArrow
                        | TokenKind::Do
                        | TokenKind::Eof
                ) {
                    term_idx = Some(idx);
                    break;
                }
            }
            match tk {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                TokenKind::Lt => lt_depth += 1,
                TokenKind::Gt => lt_depth = lt_depth.saturating_sub(1),
                TokenKind::Eof => break,
                _ => {}
            }
        }

        if let Some(t_idx) = term_idx
            && t_idx > lt_cursor + 1
        {
            // Rewind cursor to right before the terminator (e.g. `{` in `if a<b { ... }` or `;` in `let x = a<b;`)
            // so outer error recovery sees balanced `{ ... }` / `;`!
            self.cursor = t_idx - 1;
            let found_tok = self.tokens[t_idx];
            let found = found_tok.kind;
            let prev_tok = self.tokens[t_idx - 1];
            let rhs_span = Span::new(
                lt_span.hi(),
                self.token_span(prev_tok).hi().max(lt_span.hi()),
            );
            let rhs_snip = self
                .span_snippet_compact(rhs_span)
                .unwrap_or_else(|| "rhs".to_string());
            let (left_space, right_space) = self.spacing_around(lt_tok);
            let side_detail = match (left_space, right_space) {
                (true, false) => "missing whitespace after `<`",
                (false, true) => "missing whitespace before `<`",
                _ => "no spaces on either side of `<`",
            };

            return ParseError::custom(
                PARSE_SEPARATED_OPERATOR,
                "unclosed_lt_or_unspaced_comparison",
                format!(
                    "unclosed `<` after `{lhs_snip}` — infix `<` requires whitespace on both sides ({side_detail})"
                ),
                lt_span,
            )
            .with_found(found)
            .with_primary_label("`<` without spaces on both sides starts `Head<...>`")
            .with_secondary_label(
                self.next_token_span(),
                format!("hit {} before finding a matching `>`", describe_token(found)),
            )
            .with_help(format!(
                "For a less-than comparison, put spaces on both sides: `{lhs_snip} < {rhs_snip}`."
            ))
            .with_help(format!(
                "For normal-form arguments, add the closing `>`: `{lhs_snip}<{rhs_snip}>`."
            ));
        }

        original_err.with_secondary_label(lt_span, "normal-form `<` was opened here")
    }

    /// Detect C/Rust/JS `//` or `/*` comments at the current lookahead position.
    pub(crate) fn check_c_style_comment_tokens(&self) -> Option<ParseError> {
        let t1 = self.peek_next_token();
        let t2 = self.peek_nth_token(2);
        if t1.kind == TokenKind::Slash && Self::tokens_adjacent(t1, t2) {
            let span = Span::new(self.token_span(t1).lo(), self.token_span(t2).hi());
            if t2.kind == TokenKind::Slash {
                return Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_line_comment",
                        "`//` does not start a comment in Flurry",
                        span,
                    )
                    .with_primary_label("not a line comment in Flurry")
                    .with_help("Use `--` to start a line comment: `-- your note here`."),
                );
            }
            if t2.kind == TokenKind::Star {
                return Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_block_comment",
                        "`/* ... */` does not start a block comment in Flurry",
                        span,
                    )
                    .with_primary_label("not a block comment in Flurry")
                    .with_help("Use `{- ... -}` for block comments (nesting is supported)."),
                );
            }
        }
        None
    }

    /// Check prefix-position cross-language habits when starting an expression.
    pub(crate) fn check_prefix_expr_heuristics(&self) -> Option<ParseError> {
        if let Some(err) = self.check_c_style_comment_tokens() {
            return Some(err);
        }
        let t1 = self.peek_next_token();
        let span1 = self.token_span(t1);

        match t1.kind {
            // Prefix `match x { ... }` -> postfix `x match { ... }`
            TokenKind::Match => {
                let scrutinee = self
                    .token_content(&self.peek_nth_token(2))
                    .filter(|s| !s.trim().is_empty() && s != "{")
                    .unwrap_or_else(|| "value".to_string());
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_prefix_match",
                        "`match` is a postfix operator in Flurry rather than a prefix keyword",
                        span1,
                    )
                    .with_primary_label("`match` goes after the value being matched")
                    .with_help(format!(
                        "Put the scrutinee before `match`, for example: `{scrutinee} match {{ pat => body }}`."
                    )),
                )
            }
            _ => None,
        }
    }

    /// Check prefix-position cross-language habits when starting a pattern.
    pub(crate) fn check_prefix_pattern_heuristics(&self) -> Option<ParseError> {
        let t1 = self.peek_next_token();
        let t2 = self.peek_nth_token(2);
        let span1 = self.token_span(t1);

        match t1.kind {
            // `!pat` in pattern -> `not pat` (or postfix `pat!`)
            TokenKind::Bang => Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_pattern_bang",
                    "prefix `!` is not a pattern operator in Flurry",
                    span1,
                )
                .with_primary_label("prefix `!` is not valid in patterns")
                .with_help(
                    "Use `not pattern` to negate a pattern, or postfix `pattern!` to match a non-error payload.",
                ),
            ),

            // `&pat` / `&mut pat` in pattern -> `ref pat` or dereference note
            TokenKind::Ampersand => {
                let is_mut = t2.kind == TokenKind::Id
                    && self.token_content(&t2).as_deref() == Some("mut");
                let span = if is_mut {
                    Span::new(span1.lo(), self.token_span(t2).hi())
                } else {
                    span1
                };
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_pattern_ref",
                        "patterns in Flurry do not use prefix `&` or `&mut`",
                        span,
                    )
                    .with_primary_label("not a reference pattern in Flurry")
                    .with_help("Use `ref name` to bind by reference in a pattern."),
                )
            }

            // `*pat` in pattern -> postfix `.*`
            TokenKind::Star => Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_pattern_deref",
                    "prefix `*` is for pointer types `*T`, not dereference patterns",
                    span1,
                )
                .with_primary_label("prefix `*` is not a pattern operator")
                .with_help("Dereferencing in Flurry is written as postfix `.*`."),
            ),

            _ => None,
        }
    }

    /// Check post-pattern cross-language habits (`&&`, `||`, `::`).
    pub(crate) fn check_post_pattern_heuristics(&self, left: NodeIndex) -> Option<ParseError> {
        let t1 = self.peek_next_token();
        let t2 = self.peek_nth_token(2);
        let span1 = self.token_span(t1);
        let lhs_snip = self.snippet_or(left, "pat1");

        match t1.kind {
            // `pat1 | pat2` or `pat1 || pat2` -> `pat1 or pat2`
            TokenKind::Pipe => {
                if t2.kind == TokenKind::Pipe && Self::tokens_adjacent(t1, t2) {
                    let both_span = Span::new(span1.lo(), self.token_span(t2).hi());
                    return Some(
                        ParseError::custom(
                            PARSE_FOREIGN_HABIT,
                            "foreign_pattern_or_double_pipe",
                            "alternative patterns in Flurry are separated by `or`, not `||`",
                            both_span,
                        )
                        .with_primary_label("replace `||` with `or`")
                        .with_help(format!("Write `{lhs_snip} or pat2`.")),
                    );
                }
                // Note: single `|` could close a lambda parameter list ONLY in expressions,
                // never in `try_pattern_pratt`. In patterns, `a | b` is a Rust/OCaml habit for `a or b`!
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_pattern_or_pipe",
                        "alternative patterns in Flurry use `or` rather than `|`",
                        span1,
                    )
                    .with_primary_label("replace `|` with `or`")
                    .with_help(format!("Write `{lhs_snip} or pat2`.")),
                )
            }

            TokenKind::Ampersand
                if t2.kind == TokenKind::Ampersand && Self::tokens_adjacent(t1, t2) =>
            {
                let both_span = Span::new(span1.lo(), self.token_span(t2).hi());
                Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_pattern_and",
                        "`&&` is not a pattern operator in Flurry",
                        both_span,
                    )
                    .with_primary_label("not valid in patterns")
                    .with_help("Use `if cond` for pattern guards or `pat and expr is subpat` for chained sub-matches."),
                )
            }

            TokenKind::ColonColon => Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_pattern_colon_colon",
                    "Flurry uses `.` for enum variant and module paths instead of `::`",
                    span1,
                )
                .with_primary_label("replace `::` with `.`")
                .with_help(format!("Write `{lhs_snip}.Variant` with a dot.")),
            ),

            _ => None,
        }
    }

    /// Statement-start heuristics: `'label: while/for`, `extern fn`, stray `else`/`catch`, `//`, `/*`.
    pub(crate) fn check_statement_start_heuristics(&self) -> Option<ParseError> {
        if let Some(err) = self.check_c_style_comment_tokens() {
            return Some(err);
        }

        let t1 = self.peek_next_token();
        let t2 = self.peek_nth_token(2);
        let t3 = self.peek_nth_token(3);
        let t4 = self.peek_nth_token(4);
        let span1 = self.token_span(t1);

        // Rust-style `'label: while ...` or `'label: for ...`
        if t1.kind == TokenKind::Quote
            && t2.kind == TokenKind::Id
            && t3.kind == TokenKind::Colon
            && matches!(t4.kind, TokenKind::While | TokenKind::For)
        {
            let lbl = self
                .token_content(&t2)
                .unwrap_or_else(|| "label".to_string());
            let kw = t4.kind.lexme();
            let full_span = Span::new(span1.lo(), self.token_span(t3).hi());
            return Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_loop_label",
                    format!("loop labels in Flurry go right after `{kw}` as `:{lbl}` instead of `'{lbl}:`"),
                    full_span,
                )
                .with_primary_label(format!("move label after `{kw}` as `:{lbl}`"))
                .with_help(format!("Write `{kw} :{lbl} ...` instead of `'{lbl}: {kw} ...`.")),
            );
        }

        // `extern fn ...` or `extern "C" fn ...`
        if t1.kind == TokenKind::Id && self.token_content(&t1).as_deref() == Some("extern") {
            if t2.kind == TokenKind::Fn || (t2.kind == TokenKind::Str && t3.kind == TokenKind::Fn) {
                let hi = if t2.kind == TokenKind::Str {
                    self.token_span(t2).hi()
                } else {
                    span1.hi()
                };
                return Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_extern_fn",
                        "Flurry does not use an `extern` keyword on function declarations",
                        Span::new(span1.lo(), hi),
                    )
                    .with_primary_label("`extern` is not a declaration modifier in Flurry")
                    .with_help(
                        "Specify foreign calling conventions with an attribute, such as `^attrs.abi(.c) fn name(...);`.",
                    ),
                );
            }
        }

        // Stray `;` with no statement before it
        if t1.kind == TokenKind::Semi {
            return Some(
                ParseError::custom(
                    PARSE_SEPARATOR,
                    "empty_semicolon_statement",
                    "empty `;` statement — there is no expression or declaration before this `;`",
                    span1,
                )
                .with_primary_label("unneeded `;` here")
                .with_help("Remove this extra `;`."),
            );
        }

        // Stray `else` without a preceding `if` / `let` / `const`
        if t1.kind == TokenKind::Else {
            return Some(
                ParseError::custom(
                    PARSE_INVALID_SYNTAX,
                    "dangling_else",
                    "ran into `else` with no matching `if`, `let`, `var`, or `const` right before it",
                    span1,
                )
                .with_primary_label("unexpected standalone `else`")
                .with_help(
                    "Check whether an earlier `;` or extra `}` prematurely closed the preceding `if` or `let` statement.",
                ),
            );
        }

        // Stray `catch` at statement start (`catch` is part of postfix `expr! { catch ... }`)
        if t1.kind == TokenKind::Catch {
            return Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "standalone_catch",
                    "`catch` is not a standalone statement in Flurry",
                    span1,
                )
                .with_primary_label("`catch` belongs inside `expr! { catch pat => ... }`")
                .with_help("Handle errors with postfix `!`, for example: `fallible_call()! { catch err => ... }`."),
            );
        }

        None
    }

    /// Check `try_param` cross-language habits (`&self`, `&mut self`, `mut x: T`, `'a`).
    pub(crate) fn check_param_heuristics(&self) -> Option<ParseError> {
        let t1 = self.peek_next_token();
        let t2 = self.peek_nth_token(2);
        let t3 = self.peek_nth_token(3);
        let span1 = self.token_span(t1);

        // `&self`, `&mut self`, `&x: T`
        if t1.kind == TokenKind::Ampersand {
            let is_mut =
                t2.kind == TokenKind::Id && self.token_content(&t2).as_deref() == Some("mut");
            let after_amp = if is_mut { t3 } else { t2 };
            if matches!(after_amp.kind, TokenKind::SelfLower | TokenKind::Itself) {
                let full_span = Span::new(span1.lo(), self.token_span(after_amp).hi());
                let recv = after_amp.kind.lexme();
                return Some(
                    ParseError::custom(
                        PARSE_FOREIGN_HABIT,
                        "foreign_ref_self_param",
                        format!("method receivers in Flurry do not use `&{recv}` or `&mut {recv}`"),
                        full_span,
                    )
                    .with_primary_label("not a valid receiver parameter in Flurry")
                    .with_note("Comptime methods use `self` with value-reference semantics (comptime has no pointers).")
                    .with_help(format!(
                        "Write `{recv}` for a value / comptime receiver, or `*{recv}` for a runtime pointer receiver."
                    )),
                );
            }
            return Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_ref_param",
                    "parameter declarations cannot start with `&`",
                    span1,
                )
                .with_primary_label("unexpected `&` at start of parameter")
                .with_help(
                    "Put parameter types after `:`, using `*T` for a runtime pointer (`x: *T`) or `&s *T` when qualifying reachability.",
                ),
            );
        }

        // `mut x: T` in parameter list
        if t1.kind == TokenKind::Id
            && self.token_content(&t1).as_deref() == Some("mut")
            && t2.kind == TokenKind::Id
        {
            let name = self.token_content(&t2).unwrap_or_else(|| "param".into());
            return Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_mut_param",
                    "Flurry does not have a `mut` parameter modifier",
                    span1,
                )
                .with_primary_label("remove `mut` here")
                .with_help(format!(
                    "Write `{name}: Type` and rebind with `var {name} = {name};` in the body if needed (or pass `{name}: *Type` to mutate caller storage at runtime)."
                )),
            );
        }

        // `'a` Rust lifetime parameter in `<'a, T>` or `('a)`
        if t1.kind == TokenKind::Quote && t2.kind == TokenKind::Id {
            let lt_name = self.token_content(&t2).unwrap_or_else(|| "a".into());
            let full_span = Span::new(span1.lo(), self.token_span(t2).hi());
            return Some(
                ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_lifetime_param",
                    format!("`'{lt_name}` looks like a Rust lifetime parameter, which Flurry does not use"),
                    full_span,
                )
                .with_primary_label("lifetimes are not declared as `'a` in Flurry")
                .with_help(
                    "Flurry tracks aliasing through reachability sets (`s: Type.vars.Reachability` and `&s T`).",
                ),
            );
        }

        None
    }

    // =========================================================================
    // 2. Extended Application & Extended Application Pattern Brace Conflicts
    // =========================================================================

    /// True if `node` can syntactically act as the head of an `ExtendedApplication`
    /// (`head { ... }`) or ends with a `BoolMatches` whose pattern can take `{ ... }`.
    pub(crate) fn expr_can_have_brace_conflict(&self, node: NodeIndex) -> bool {
        let Some(kind) = self.ast.get_node_kind(node) else {
            return false;
        };
        if self.is_parenthesized(node) {
            return false;
        }
        match kind {
            NodeKind::Id
            | NodeKind::Projection
            | NodeKind::NormalFormHeadApplication
            | NodeKind::Application
            | NodeKind::IndexApplication => true,
            NodeKind::BoolMatches => {
                let children = self.ast.get_children(node);
                children
                    .get(1)
                    .copied()
                    .is_some_and(|pat| self.pattern_can_have_brace_conflict(pat))
            }
            // Binary expression whose RHS could have wanted `{ ... }`, e.g. `ok and Foo` + `{ x: 1 } { ... }`
            NodeKind::BoolAnd
            | NodeKind::BoolOr
            | NodeKind::BoolEq
            | NodeKind::BoolNotEq
            | NodeKind::Pipe
            | NodeKind::PipePrepend
            | NodeKind::Add
            | NodeKind::Sub => {
                let children = self.ast.get_children(node);
                children
                    .get(1)
                    .copied()
                    .is_some_and(|rhs| self.expr_can_have_brace_conflict(rhs))
            }
            NodeKind::BoolNot | NodeKind::Negative | NodeKind::ErrorNew => {
                let children = self.ast.get_children(node);
                children
                    .first()
                    .copied()
                    .is_some_and(|inner| self.expr_can_have_brace_conflict(inner))
            }
            _ => false,
        }
    }

    pub(crate) fn pattern_can_have_brace_conflict(&self, pat: NodeIndex) -> bool {
        let Some(kind) = self.ast.get_node_kind(pat) else {
            return false;
        };
        if self.is_parenthesized(pat) {
            return false;
        }
        matches!(
            kind,
            NodeKind::Id
                | NodeKind::Projection
                | NodeKind::Symbol
                | NodeKind::NormalFormHeadApplicationPattern
                | NodeKind::ExprAsPattern
        )
    }

    /// Check if `{ ... }` at the current cursor parses cleanly as an extended
    /// application argument block `{ config / property / expand / stmt }` or
    /// extended application pattern `{ field_pat, ... }`.
    pub(crate) fn probe_brace_looks_like_extended_args(
        &mut self,
        is_pattern: bool,
    ) -> Option<Span> {
        if !self.peek(TokenKind::LBrace.as_ref()) {
            return None;
        }
        self.speculative(|p| {
            let start_sp = p.next_token_span();
            if is_pattern {
                let dummy_head = ast::NodeBuilder::new(NodeKind::Id, start_sp).build(&mut p.ast);
                let ok = p
                    .try_postfix_pattern(TokenKind::LBrace, dummy_head, 0, PatternOption::new())
                    .is_ok();
                if ok {
                    Some(Span::new(start_sp.lo(), p.prev_token_span().hi()))
                } else {
                    None
                }
            } else {
                let dummy_head = ast::NodeBuilder::new(NodeKind::Id, start_sp).build(&mut p.ast);
                let ok = p
                    .try_extended_call_expr(dummy_head, ExprOption::new())
                    .is_ok();
                if ok {
                    Some(Span::new(start_sp.lo(), p.prev_token_span().hi()))
                } else {
                    None
                }
            }
        })
    }

    /// Called when `if <expr>`, `while <expr>`, or `for ... in <expr>` reaches `{ ... }`
    /// BEFORE `try_block()` runs, detecting BOTH:
    /// 1. `{ ... }` contains extended-call / extended-pattern arguments (`Foo { x: 1 } { ... }`,
    ///    `v matches .some { val: x } { ... }`), AND
    /// 2. `{ ... }` is a 1-item destructuring/call block immediately followed on the same line by
    ///    the control-flow body `{ ... }` (`if v matches Point { x } { use(x); }`)!
    pub(crate) fn diagnose_control_header_brace_conflict(
        &mut self,
        construct_kw: &'static str,
        kw_span: Span,
        header_expr: NodeIndex,
    ) -> Option<ParseError> {
        if !self.peek(TokenKind::LBrace.as_ref()) || !self.expr_can_have_brace_conflict(header_expr)
        {
            return None;
        }

        // Fast token scan across the first balanced `{ ... }`:
        // only run speculative AST probes if `,` / `:` / `...` appears at brace depth 1
        // OR a second `{` immediately follows the closing `}`!
        let open_idx = self.cursor + 1;
        let mut depth = 0usize;
        let mut has_prop_or_comma_at_top = false;
        let mut close_idx: Option<usize> = None;

        for idx in open_idx..self.tokens.len() {
            let tk = self.tokens[idx].kind;
            match tk {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        close_idx = Some(idx);
                        break;
                    }
                }
                TokenKind::Comma | TokenKind::Colon if depth == 1 => {
                    has_prop_or_comma_at_top = true;
                }
                TokenKind::Eof => break,
                _ => {}
            }
        }

        let c_idx = close_idx?;
        let after_tok = self
            .tokens
            .get(c_idx + 1)
            .copied()
            .unwrap_or_else(|| Token::new(TokenKind::Eof, 0, 0));
        let has_second_lbrace = after_tok.kind == TokenKind::LBrace;
        let second_lbrace_same_line =
            has_second_lbrace && self.tokens_on_same_line(self.tokens[c_idx], after_tok);

        if !has_prop_or_comma_at_top && !second_lbrace_same_line {
            return None;
        }

        let is_matches_pat = self.ast.get_node_kind(header_expr) == Some(NodeKind::BoolMatches);
        let ext_span = self.probe_brace_looks_like_extended_args(is_matches_pat)?;
        let valid_as_block = self.speculative(|p| p.try_block().is_ok());

        if valid_as_block && !second_lbrace_same_line {
            return None;
        }

        let head_snip = self.snippet_or(header_expr, "expr");
        let brace_snip = self
            .span_snippet_compact(ext_span)
            .unwrap_or_else(|| "{ ... }".to_string());
        let kind_desc = if is_matches_pat {
            "extended application pattern"
        } else {
            "extended application"
        };

        // Advance cursor past the first `{ ... }` (and if `recovery_enabled` is true and a second `{`
        // body block follows, parse that body block too so recovery stays aligned and checks its body!).
        self.cursor = c_idx;
        if self.recovery_enabled && self.peek(TokenKind::LBrace.as_ref()) {
            let _ = self.try_block();
        }

        if has_second_lbrace {
            Some(
                ParseError::custom(
                    PARSE_BRACE_CONFLICT,
                    "extended_application_double_brace_conflict",
                    format!(
                        "two `{{ ... }}` blocks follow `{construct_kw} {head_snip}` — `{brace_snip}` conflicts with the `{construct_kw}` body block"
                    ),
                    ext_span,
                )
                .with_primary_label(format!(
                    "looks like an {kind_desc} on `{head_snip}`, but `{construct_kw}` reads `{{` as its body"
                ))
                .with_secondary_label(
                    self.token_span(after_tok),
                    format!("`{construct_kw}` body block starts here"),
                )
                .with_secondary_label(kw_span, format!("`{construct_kw}` starts here"))
                .with_note(format!(
                    "Inside `{construct_kw} ... {{ ... }}`, unparenthesized trailing `{{ ... }}` calls and patterns are disabled so the `{construct_kw}` body block stays unambiguous."
                ))
                .with_help(format!(
                    "Wrap the {kind_desc} in parentheses: `{construct_kw} ({head_snip} {brace_snip}) {{ ... }}`."
                )),
            )
        } else {
            Some(
                ParseError::custom(
                    PARSE_BRACE_CONFLICT,
                    "extended_application_brace_conflict",
                    format!(
                        "`{{ ... }}` after `{head_snip}` starts the `{construct_kw}` body block rather than an {kind_desc}"
                    ),
                    ext_span,
                )
                .with_primary_label(format!(
                    "parsed as the `{construct_kw}` body block, not arguments to `{head_snip}`"
                ))
                .with_secondary_label(
                    kw_span,
                    format!("this `{construct_kw}` header stops before `{{`"),
                )
                .with_help(format!(
                    "Wrap the {kind_desc} in parentheses and add the `{construct_kw}` body block: `{construct_kw} ({head_snip} {brace_snip}) {{ ... }}`."
                )),
            )
        }
    }

    /// Reverse brace-conflict check for `if expr is pat do { ... }` and `while expr is pat do { ... }`:
    /// detects when `do` was omitted before `{ ... }` so `{ ... }` collided with `ExtendedApplicationPattern`!
    pub(crate) fn parse_is_match_pattern_checking_do(
        &mut self,
        construct_kw: &'static str,
        expr: NodeIndex,
    ) -> ParseResult {
        let saved_cursor = self.cursor;
        let saved_recovery = self.recovery_enabled;
        self.recovery_enabled = false;
        let pat_res = self.try_pattern();
        self.recovery_enabled = saved_recovery;

        let expr_snip = self.snippet_or(expr, "value");

        match pat_res {
            Err(err) => {
                // Did `try_pattern()` fail inside `{ ... }` because `{ ... }` is actually the `if`/`while` body
                // with `do` forgotten (e.g. `if x is .some { return 1; }`)?
                let head_before_brace = self.speculative(|p| {
                    p.cursor = saved_cursor;
                    let head = p.try_pattern_without_extended_call().ok()?;
                    if head != 0 && p.peek(TokenKind::LBrace.as_ref()) {
                        let head_snip = p.snippet_or(head, "pat");
                        Some((p.cursor, p.next_token_span(), head_snip))
                    } else {
                        None
                    }
                });
                if let Some((before_brace_cursor, brace_span, head_snip)) = head_before_brace {
                    self.cursor = before_brace_cursor;
                    if self.recovery_enabled && self.peek(TokenKind::LBrace.as_ref()) {
                        let _ = self.try_block();
                    }
                    return Err(
                        ParseError::custom(
                            PARSE_BRACE_CONFLICT,
                            "missing_do_in_is_match",
                            format!(
                                "missing `do` before `{{ ... }}` block in `{construct_kw} ... is`"
                            ),
                            brace_span,
                        )
                        .with_found(TokenKind::LBrace)
                        .with_primary_label(format!(
                            "expected `do` before this `{{` (otherwise `{{ ... }}` is read as pattern fields on `{head_snip}`)"
                        ))
                        .with_help(format!(
                            "Insert `do` before the body block: `{construct_kw} {expr_snip} is {head_snip} do {{ ... }}`."
                        )),
                    );
                }
                Err(err)
            }
            Ok(0) => Ok(0),
            Ok(pattern) => {
                if !self.eat_token(TokenKind::Do) {
                    if self.ast.get_node_kind(pattern) == Some(NodeKind::ExtendedApplicationPattern)
                    {
                        let pat_span = self.ast.get_span(pattern).unwrap_or(self.prev_token_span());
                        let pat_snip = self.snippet_or(pattern, "pat { ... }");
                        let inner_head =
                            self.ast.get_children(pattern).first().copied().unwrap_or(0);
                        let head_snip = self.snippet_or(inner_head, "pat");
                        if self.recovery_enabled && self.peek(TokenKind::LBrace.as_ref()) {
                            let _ = self.try_block();
                        }
                        return Err(
                            ParseError::custom(
                                PARSE_BRACE_CONFLICT,
                                "swallowed_brace_missing_do_in_is_match",
                                format!(
                                    "expected `do` in `{construct_kw} ... is` — `{{ ... }}` was parsed as destructuring pattern fields on `{head_snip}`"
                                ),
                                pat_span,
                            )
                            .with_found(self.peek_next_token().kind)
                            .with_primary_label(format!(
                                "parsed as pattern fields on `{head_snip}` because `do` was omitted"
                            ))
                            .with_help(format!(
                                "If `{{ ... }}` is the `{construct_kw}` body, insert `do`: `{construct_kw} {expr_snip} is {head_snip} do {{ ... }}`."
                            ))
                            .with_help(format!(
                                "If `{pat_snip}` destructures fields, add `do` before the body block: `{construct_kw} {expr_snip} is {pat_snip} do {{ ... }}`."
                            )),
                        );
                    }
                    return Err(
                        ParseError::unexpected_token(
                            TokenKind::Do,
                            self.peek_next_token().kind,
                            self.next_token_span(),
                        )
                        .with_help(format!(
                            "`{construct_kw} ... is` requires `do` before its body block: `{construct_kw} {expr_snip} is {} do {{ ... }}`.",
                            self.snippet_or(pattern, "pattern")
                        )),
                    );
                }
                Ok(pattern)
            }
        }
    }

    /// Reverse brace-conflict check for `match` arms (`pat => body`), `catch` arms (`catch pat => body`),
    /// and `when` arms (`cond => body`) when `=>` was omitted before `{ ... }`!
    pub(crate) fn parse_arm_head_checking_fat_arrow(
        &mut self,
        is_pattern: bool,
        arm_desc: &'static str,
    ) -> ParseResult {
        let saved_cursor = self.cursor;
        let saved_recovery = self.recovery_enabled;
        self.recovery_enabled = false;
        let head_res = if is_pattern {
            self.try_pattern()
        } else {
            self.try_expr()
        };
        self.recovery_enabled = saved_recovery;

        match head_res {
            Err(err) => {
                // Did `try_pattern()` / `try_expr()` fail inside `{ ... }` because `=>` was omitted
                // before the arm body `{ ... }` (e.g. `.ok { return 1; }` or `ready { return 1; }`)?
                let head_before_brace = self.speculative(|p| {
                    p.cursor = saved_cursor;
                    let head = if is_pattern {
                        p.try_pattern_without_extended_call().ok()?
                    } else {
                        p.try_expr_without_extended_call().ok()?
                    };
                    if head != 0 && p.peek(TokenKind::LBrace.as_ref()) {
                        let head_snip = p.snippet_or(head, "arm");
                        Some((p.cursor, p.next_token_span(), head_snip))
                    } else {
                        None
                    }
                });
                if let Some((before_brace_cursor, brace_span, head_snip)) = head_before_brace {
                    self.cursor = before_brace_cursor;
                    if self.recovery_enabled && self.peek(TokenKind::LBrace.as_ref()) {
                        let _ = self.try_block();
                    }
                    return Err(
                        ParseError::custom(
                            PARSE_BRACE_CONFLICT,
                            "missing_fat_arrow_before_arm_block",
                            format!("missing `=>` before `{{ ... }}` block in {arm_desc}"),
                            brace_span,
                        )
                        .with_found(TokenKind::LBrace)
                        .with_primary_label(format!(
                            "expected `=>` before this `{{` (otherwise `{{ ... }}` is read as arguments on `{head_snip}`)"
                        ))
                        .with_help(format!(
                            "Insert `=>` between `{head_snip}` and its body block: `{head_snip} => {{ ... }}`."
                        )),
                    );
                }
                Err(err)
            }
            Ok(0) => Ok(0),
            Ok(head) => {
                if !self.eat_token(TokenKind::FatArrow) {
                    let kind = self.ast.get_node_kind(head);
                    if matches!(
                        kind,
                        Some(NodeKind::ExtendedApplicationPattern | NodeKind::ExtendedApplication)
                    ) {
                        let full_span = self.ast.get_span(head).unwrap_or(self.prev_token_span());
                        let full_snip = self.snippet_or(head, "head { ... }");
                        let inner_head = self.ast.get_children(head).first().copied().unwrap_or(0);
                        let inner_snip = self.snippet_or(inner_head, "head");
                        if self.recovery_enabled && self.peek(TokenKind::LBrace.as_ref()) {
                            let _ = self.try_block();
                        }
                        return Err(
                            ParseError::custom(
                                PARSE_BRACE_CONFLICT,
                                "swallowed_brace_missing_fat_arrow",
                                format!(
                                    "expected `=>` in {arm_desc} — `{{ ... }}` was parsed as fields/arguments on `{inner_snip}` rather than the arm body"
                                ),
                                full_span,
                            )
                            .with_found(self.peek_next_token().kind)
                            .with_primary_label(format!(
                                "parsed as `{inner_snip} {{ ... }}` because `=>` was omitted"
                            ))
                            .with_help(format!(
                                "If `{{ ... }}` is the arm body, insert `=>`: `{inner_snip} => {{ ... }}`."
                            ))
                            .with_help(format!(
                                "If `{full_snip}` is part of the {arm_desc} head, add `=>` after it: `{full_snip} => {{ ... }}`."
                            )),
                        );
                    }
                    return Err(ParseError::unexpected_token(
                        TokenKind::FatArrow,
                        self.peek_next_token().kind,
                        self.next_token_span(),
                    )
                    .with_help(format!(
                        "Separate the {arm_desc} head and body with `=>`: `{} => ...`.",
                        self.snippet_or(head, "head")
                    )));
                }
                Ok(head)
            }
        }
    }

    /// Check when a `no_extended_call` expression in a non-block slot (such as
    /// `struct` field default `= Point { x: 0 }`, `fn() -> Ret { a: i32 } { ... }`,
    /// `impl Builder { cfg: 1 } for T {}`, `^attr { k: v } fn f()`) is followed by `{`!
    pub(crate) fn check_non_block_extended_call_conflict(
        &mut self,
        expr: NodeIndex,
        context_desc: &str,
        expected_next: Option<TokenKind>,
    ) -> Option<ParseError> {
        if !self.peek(TokenKind::LBrace.as_ref()) || !self.expr_can_have_brace_conflict(expr) {
            return None;
        }
        // Only trigger when `expected_next` is NOT `{` (e.g. `,`, `;`, `for`), OR when TWO `{` blocks follow!
        let brace_span = self.next_token_span();
        let head_snip = self.snippet_or(expr, "Head");
        let is_matches_pat = self.ast.get_node_kind(expr) == Some(NodeKind::BoolMatches);
        let kind_desc = if is_matches_pat {
            "extended application pattern"
        } else {
            "extended application"
        };

        if expected_next != Some(TokenKind::LBrace) {
            if let Some((ext_span, end_cursor)) = self.speculative(|p| {
                let dummy = ast::NodeBuilder::new(NodeKind::Id, brace_span).build(&mut p.ast);
                let ok = if is_matches_pat {
                    p.try_postfix_pattern(TokenKind::LBrace, dummy, 0, PatternOption::new())
                        .is_ok()
                } else {
                    p.try_extended_call_expr(dummy, ExprOption::new()).is_ok()
                };
                ok.then(|| {
                    (
                        Span::new(brace_span.lo(), p.prev_token_span().hi()),
                        p.cursor,
                    )
                })
            }) {
                let ext_snip = self
                    .span_snippet_compact(ext_span)
                    .unwrap_or_else(|| "{ ... }".to_string());
                if self.recovery_enabled {
                    self.cursor = end_cursor;
                }
                return Some(
                    ParseError::custom(
                        PARSE_BRACE_CONFLICT,
                        "unparenthesized_extended_call_in_header",
                        format!(
                            "unparenthesized {kind_desc} `{head_snip} {ext_snip}` is not allowed directly in {context_desc}"
                        ),
                        ext_span,
                    )
                    .with_primary_label("`{ ... }` conflicts with surrounding declaration delimiters")
                    .with_help(format!(
                        "Wrap it in parentheses: `({head_snip} {ext_snip})`."
                    )),
                );
            }
            return None;
        }

        // `expected_next == Some(TokenKind::LBrace)` (e.g., `fn -> Ret { ... } { body }`,
        // `impl Type { ... } { body }`, `where Pred { ... } { body }`):
        // Probe if there is a `{ ... }` that looks like extended args AND ANOTHER `{` right after it!
        let second_brace_after = self.speculative(|p| {
            let dummy = ast::NodeBuilder::new(NodeKind::Id, brace_span).build(&mut p.ast);
            let ok = if is_matches_pat {
                p.try_postfix_pattern(TokenKind::LBrace, dummy, 0, PatternOption::new())
                    .is_ok()
            } else {
                p.try_extended_call_expr(dummy, ExprOption::new()).is_ok()
            };
            if ok
                && matches!(
                    p.peek_next_token().kind,
                    TokenKind::LBrace | TokenKind::For | TokenKind::Where
                )
            {
                Some((
                    Span::new(brace_span.lo(), p.prev_token_span().hi()),
                    p.next_token_span(),
                    p.cursor,
                ))
            } else {
                None
            }
        });

        if let Some((ext_span, body_brace_span, end_cursor)) = second_brace_after {
            let ext_snip = self
                .span_snippet_compact(ext_span)
                .unwrap_or_else(|| "{ ... }".to_string());
            if self.recovery_enabled {
                self.cursor = end_cursor;
            }
            return Some(
                ParseError::custom(
                    PARSE_BRACE_CONFLICT,
                    "header_extended_call_brace_conflict",
                    format!(
                        "`{ext_snip}` after `{head_snip}` in {context_desc} collides with the declaration's own `{{ ... }}` body"
                    ),
                    ext_span,
                )
                .with_primary_label(format!("looks like arguments to `{head_snip}`"))
                .with_secondary_label(body_brace_span, "declaration body `{` starts here")
                .with_help(format!(
                    "Wrap the {kind_desc} in parentheses: `({head_snip} {ext_snip})`."
                )),
            );
        }

        None
    }

    // =========================================================================
    // 3. Complex Expr / Pattern Parenthesization Recommendations (Warnings)
    // =========================================================================

    /// Emit non-fatal parenthesization recommendations when `expr` mixes prefix
    /// and infix/postfix operators across multiple layers where precedence is
    /// genuinely easy to misread (and was not explicitly parenthesized).
    pub(crate) fn check_expr_parenthesization(&mut self, node: NodeIndex) {
        if node == 0 || self.warned_nodes.contains(&node) {
            return;
        }
        let Some(outer_kind) = self.ast.get_node_kind(node) else {
            return;
        };
        let Some(outer_span) = self.ast.get_span(node) else {
            return;
        };
        let children = self.ast.get_children(node).to_vec();

        // Case A: Binary infix expression wrapping an unparenthesized prefix operand
        if let Some((op_str, outer_cat)) = Self::infix_expr_info(outer_kind)
            && children.len() >= 2
        {
            let lhs = children[0];
            let rhs = children[1];

            // A1: `not a == b`, `not a != b`, `not a < b`, `not a matches P`, `not a |> f`, `not a ==> b`
            if !self.is_parenthesized(lhs)
                && self.ast.get_node_kind(lhs) == Some(NodeKind::BoolNot)
                && matches!(
                    outer_cat,
                    InfixCat::Comparison | InfixCat::Pipe | InfixCat::Implies | InfixCat::Typed
                )
            {
                let inner = self.ast.get_children(lhs).first().copied().unwrap_or(0);
                let a_s = self.snippet_or(inner, "a");
                let b_s = self.snippet_or(rhs, "b");
                self.warned_nodes.insert(node);
                self.record_warning(
                    ParseWarning::new(
                        PARSE_WARN_PARENTHESES,
                        format!(
                            "mixing prefix `not` with `{op_str}` without parentheses can be easy to misread"
                        ),
                        outer_span,
                    )
                    .with_primary_label(format!("grouped as `(not {a_s}) {op_str} {b_s}`"))
                    .with_note(format!(
                        "Prefix `not` binds tighter than `{op_str}`, so Flurry reads this as `(not {a_s}) {op_str} {b_s}`."
                    ))
                    .with_help(format!(
                        "Add parentheses to make the grouping explicit: `not ({a_s} {op_str} {b_s})` or `(not {a_s}) {op_str} {b_s}`."
                    )),
                );
                return;
            }

            // A2: Multi-layer boolean chain mixing unparenthesized `not` + `and` + `or`
            // (e.g. `not a and b or c` -> 3 operator layers without parentheses!)
            if outer_kind == NodeKind::BoolOr {
                for &arm in &[lhs, rhs] {
                    if !self.is_parenthesized(arm)
                        && self.ast.get_node_kind(arm) == Some(NodeKind::BoolAnd)
                    {
                        let and_ch = self.ast.get_children(arm).to_vec();
                        let has_unparen_not = and_ch.iter().any(|&c| {
                            !self.is_parenthesized(c)
                                && self.ast.get_node_kind(c) == Some(NodeKind::BoolNot)
                        });
                        if has_unparen_not {
                            let arm_s = self.snippet_or(arm, "not a and b");
                            self.warned_nodes.insert(node);
                            self.record_warning(
                                ParseWarning::new(
                                    PARSE_WARN_PARENTHESES,
                                    "this expression mixes `not`, `and`, and `or` across three unparenthesized precedence levels",
                                    outer_span,
                                )
                                .with_primary_label("consider parenthesizing the `and` / `not` sub-expressions")
                                .with_help(format!(
                                    "Wrap the `and` clause in parentheses for readability, e.g. `({arm_s})`."
                                )),
                            );
                            return;
                        }
                    }
                }
            }

            // A3: `error x + y` / `error x |> f` — prefix `error` inside binary infix!
            if !self.is_parenthesized(lhs)
                && self.ast.get_node_kind(lhs) == Some(NodeKind::ErrorNew)
            {
                let kw = "error";
                let inner = self.ast.get_children(lhs).first().copied().unwrap_or(0);
                let a_s = self.snippet_or(inner, "x");
                let b_s = self.snippet_or(rhs, "y");
                self.warned_nodes.insert(node);
                self.record_warning(
                    ParseWarning::new(
                        PARSE_WARN_PARENTHESES,
                        format!(
                            "prefix `{kw}` binds tighter than `{op_str}`, so `{op_str}` applies outside `{kw}`"
                        ),
                        outer_span,
                    )
                    .with_primary_label(format!("grouped as `({kw} {a_s}) {op_str} {b_s}`"))
                    .with_help(format!(
                        "Add parentheses to clarify intent: `{kw} ({a_s} {op_str} {b_s})` or `({kw} {a_s}) {op_str} {b_s}`."
                    )),
                );
                return;
            }

            // A4: Prefix `*`, `?`, `dyn`, `lift` on LHS of `->` when wrapping `fn(...)` (`*fn() -> T` is `(*fn()) -> T`!),
            // or on LHS of union `|` (`?A | B` is `(?A) | B`, unlike `!E A | B` which is `!E (A | B)`!).
            if !self.is_parenthesized(lhs)
                && let Some(pref_sym) = match self.ast.get_node_kind(lhs) {
                    Some(NodeKind::PointerType) => Some("*"),
                    Some(NodeKind::OptionalType) => Some("?"),
                    Some(NodeKind::TraitObjectType) => Some("dyn "),
                    Some(NodeKind::LiftType) => Some("lift "),
                    _ => None,
                }
            {
                let inner = self.ast.get_children(lhs).first().copied().unwrap_or(0);
                let inner_kind = self.ast.get_node_kind(inner);
                if (outer_kind == NodeKind::Arrow && inner_kind == Some(NodeKind::FnType))
                    || outer_kind == NodeKind::Pipe
                {
                    let a_s = self.snippet_or(inner, "A");
                    let b_s = self.snippet_or(rhs, "B");
                    self.warned_nodes.insert(node);
                    self.record_warning(
                        ParseWarning::new(
                            PARSE_WARN_PARENTHESES,
                            format!(
                                "prefix `{}` binds tighter than `{op_str}` here",
                                pref_sym.trim()
                            ),
                            outer_span,
                        )
                        .with_primary_label(format!(
                            "grouped as `({pref_sym}{a_s}) {op_str} {b_s}`"
                        ))
                        .with_help(format!(
                            "Add parentheses to make the type structure clear: `{pref_sym}({a_s} {op_str} {b_s})` or `({pref_sym}{a_s}) {op_str} {b_s}`."
                        )),
                    );
                    return;
                }
            }
        }

        // Case B: Prefix expression wrapping a deceptive postfix / infix structure
        match outer_kind {
            // `-2.pow(3)` / `-128.as(i8)` (unary `-` on numeric literal with method call!)
            // or `-x match { ... }` / `not x match { ... }` / `error x match { ... }`
            NodeKind::Negative | NodeKind::BoolNot | NodeKind::ErrorNew => {
                let Some(&inner) = children.first() else {
                    return;
                };
                if self.is_parenthesized(inner) {
                    return;
                }
                let pref = match outer_kind {
                    NodeKind::Negative => "-",
                    NodeKind::BoolNot => "not ",
                    _ => "error ",
                };
                let inner_kind = self.ast.get_node_kind(inner);

                if matches!(inner_kind, Some(NodeKind::PostMatch | NodeKind::PostLambda)) {
                    let inner_s = self.snippet_or(inner, "expr match { ... }");
                    self.warned_nodes.insert(node);
                    self.record_warning(
                        ParseWarning::new(
                            PARSE_WARN_PARENTHESES,
                            format!(
                                "prefix `{}` wraps the entire postfix `{}` expression",
                                pref.trim(),
                                if inner_kind == Some(NodeKind::PostMatch) {
                                    "match"
                                } else {
                                    "do"
                                }
                            ),
                            outer_span,
                        )
                        .with_primary_label(format!("applies `{}` to the result of `{inner_s}`", pref.trim()))
                        .with_help(format!(
                            "Wrap the postfix expression in parentheses for clarity: `{pref}({inner_s})`."
                        )),
                    );
                    return;
                }

                // Numeric literal receiver under unary `-` with `.as(T)` / `.dyn(T)` / `.method(...)`,
                // e.g. `-128.as(i8)` or `-2.pow(3)`!
                if outer_kind == NodeKind::Negative {
                    let numeric_recv = match inner_kind {
                        Some(
                            NodeKind::TypeCast | NodeKind::DynCast | NodeKind::HandlerApplication,
                        ) => self.ast.get_children(inner).first().copied(),
                        Some(NodeKind::Application) => self
                            .ast
                            .get_children(inner)
                            .first()
                            .copied()
                            .filter(|&c| self.ast.get_node_kind(c) == Some(NodeKind::Projection))
                            .and_then(|c| self.ast.get_children(c).first().copied()),
                        _ => None,
                    };
                    if let Some(recv) = numeric_recv
                        && !self.is_parenthesized(recv)
                        && matches!(
                            self.ast.get_node_kind(recv),
                            Some(NodeKind::Int | NodeKind::Real)
                        )
                    {
                        let lit_s = self.snippet_or(recv, "n");
                        let inner_s = self.snippet_or(inner, "n.method()");
                        let rest_span = Span::new(
                            self.ast.get_span(recv).unwrap_or(outer_span).hi(),
                            outer_span.hi(),
                        );
                        let rest_s = self
                            .span_snippet_compact(rest_span)
                            .unwrap_or_else(|| ".method()".to_string());
                        self.warned_nodes.insert(node);
                        self.record_warning(
                            ParseWarning::new(
                                PARSE_WARN_PARENTHESES,
                                format!(
                                    "`-{inner_s}` calls `{rest_s}` on positive `{lit_s}` before negating the result"
                                ),
                                outer_span,
                            )
                            .with_primary_label(format!("grouped as `-({inner_s})`"))
                            .with_help(format!(
                                "Write `(-{lit_s}){rest_s}` to apply `{rest_s}` to `-{lit_s}`, or `-({inner_s})` if negating afterwards was intended."
                            )),
                        );
                    }
                }
            }

            // `!E T | U` / `#E T | U` / `&s T | U` / `%c T | U`:
            // Because type qualifiers parse their payload at prec 50 and `|` has prec 80,
            // `!E A | B` groups as `!E (A | B)` (opposite of `?A | B` which is `(?A) | B`!).
            NodeKind::ErrorQualifiedType
            | NodeKind::EffectQualifiedType
            | NodeKind::ReachabilityQualifiedType
            | NodeKind::ClosureQualifiedType => {
                if children.len() >= 2 {
                    let payload = children[1];
                    if !self.is_parenthesized(payload)
                        && self.ast.get_node_kind(payload) == Some(NodeKind::Pipe)
                    {
                        let q_sym = match outer_kind {
                            NodeKind::ErrorQualifiedType => "!",
                            NodeKind::EffectQualifiedType => "#",
                            NodeKind::ReachabilityQualifiedType => "&",
                            _ => "%",
                        };
                        let head_s = self.snippet_or(children[0], "q");
                        let pay_s = self.snippet_or(payload, "A | B");
                        self.warned_nodes.insert(node);
                        self.record_warning(
                            ParseWarning::new(
                                PARSE_WARN_PARENTHESES,
                                format!(
                                    "`{q_sym}{head_s} {pay_s}` groups the union `|` inside the `{q_sym}` type qualifier"
                                ),
                                outer_span,
                            )
                            .with_primary_label(format!("grouped as `{q_sym}{head_s} ({pay_s})`"))
                            .with_help(format!(
                                "Add parentheses to make the type boundary clear: `{q_sym}{head_s} ({pay_s})` or `({q_sym}{head_s} ...) | ...`."
                            )),
                        );
                    }
                }
            }

            // Prefix range `..expr` / `..=expr` or `defer expr` wrapping low-precedence infix (`|>`, `and`, `or`, `==`, etc.)
            NodeKind::RangeTo | NodeKind::RangeToInclusive | NodeKind::Defer => {
                if let Some(&inner) = children.first()
                    && !self.is_parenthesized(inner)
                    && let Some((_, cat)) = self
                        .ast
                        .get_node_kind(inner)
                        .and_then(Self::infix_expr_info)
                    && matches!(
                        cat,
                        InfixCat::Implies
                            | InfixCat::Logical
                            | InfixCat::Comparison
                            | InfixCat::Pipe
                            | InfixCat::Arrow
                    )
                {
                    let dots = match outer_kind {
                        NodeKind::RangeToInclusive => "..=",
                        NodeKind::RangeTo => "..",
                        _ => "defer ",
                    };
                    let inner_s = self.snippet_or(inner, "expr");
                    self.warned_nodes.insert(node);
                    self.record_warning(
                        ParseWarning::new(
                            PARSE_WARN_PARENTHESES,
                            format!(
                                "prefix `{}` captures the entire `{inner_s}` expression",
                                dots.trim()
                            ),
                            outer_span,
                        )
                        .with_primary_label(format!("grouped as `{dots}({inner_s})`"))
                        .with_help(format!(
                            "Add parentheses around the range or its endpoint, e.g. `{dots}({inner_s})`."
                        )),
                    );
                }
            }

            _ => {}
        }
    }

    /// Emit non-fatal parenthesization recommendations on complex mixed prefix +
    /// infix/postfix patterns!
    /// Remember: in Flurry patterns, prefix `not`, `ref`, `error`, `async` parse
    /// their operand at precedence 0, so `not a or b` groups as `not (a or b)`
    /// (the opposite of expressions!), and `ref x as whole` groups as `ref (x as whole)`!
    pub(crate) fn check_pattern_parenthesization(&mut self, pat: NodeIndex) {
        if pat == 0 || self.warned_nodes.contains(&pat) {
            return;
        }
        let Some(outer_kind) = self.ast.get_node_kind(pat) else {
            return;
        };
        let Some(outer_span) = self.ast.get_span(pat) else {
            return;
        };
        let children = self.ast.get_children(pat).to_vec();

        // Prefix pattern (`not`, `ref`, `error`, `async`, `..`, `..=`) wrapping unparenthesized
        // `or`, `as`, `if` guard, `and ... is`, range, or `?`/`!` propagation!
        let prefix_kw = match outer_kind {
            NodeKind::NotPattern => Some("not "),
            NodeKind::RefPattern => Some("ref "),
            NodeKind::ErrorPattern => Some("error "),
            NodeKind::AsyncPattern => Some("async "),
            NodeKind::RangeToPattern => Some(".."),
            NodeKind::RangeToInclusivePattern => Some("..="),
            _ => None,
        };

        if let Some(pref) = prefix_kw
            && let Some(&inner) = children.first()
            && !self.is_parenthesized(inner)
        {
            let inner_kind = self.ast.get_node_kind(inner);
            let inner_desc = match inner_kind {
                Some(NodeKind::OrPattern) => Some("or"),
                Some(NodeKind::AsBindPattern) => Some("as"),
                Some(NodeKind::IfGuardPattern) => Some("if"),
                Some(NodeKind::AndIsPattern) => Some("and ... is"),
                Some(NodeKind::OptionSomePattern)
                    if !matches!(
                        outer_kind,
                        NodeKind::RangeToPattern | NodeKind::RangeToInclusivePattern
                    ) =>
                {
                    Some("?")
                }
                Some(NodeKind::ErrorOkPattern)
                    if !matches!(
                        outer_kind,
                        NodeKind::RangeToPattern | NodeKind::RangeToInclusivePattern
                    ) =>
                {
                    Some("!")
                }
                _ => None,
            };

            if let Some(op_desc) = inner_desc {
                let inner_s = self.snippet_or(inner, "pat");
                let trimmed_pref = pref.trim();
                self.warned_nodes.insert(pat);
                let extra_alt = match inner_kind {
                    Some(NodeKind::OrPattern) => {
                        let ich = self.ast.get_children(inner);
                        let p1 = ich
                            .first()
                            .copied()
                            .map(|n| self.snippet_or(n, "a"))
                            .unwrap_or_default();
                        let p2 = ich
                            .get(1)
                            .copied()
                            .map(|n| self.snippet_or(n, "b"))
                            .unwrap_or_default();
                        format!(" or `({pref}{p1}) or {p2}`")
                    }
                    Some(NodeKind::AsBindPattern) => {
                        let ich = self.ast.get_children(inner);
                        let p1 = ich
                            .first()
                            .copied()
                            .map(|n| self.snippet_or(n, "pat"))
                            .unwrap_or_default();
                        let p2 = ich
                            .get(1)
                            .copied()
                            .map(|n| self.snippet_or(n, "name"))
                            .unwrap_or_default();
                        format!(" or `({pref}{p1}) as {p2}`")
                    }
                    _ => String::new(),
                };
                self.record_warning(
                    ParseWarning::new(
                        PARSE_WARN_PARENTHESES,
                        format!(
                            "prefix `{trimmed_pref}` in a pattern wraps the following `{op_desc}` pattern"
                        ),
                        outer_span,
                    )
                    .with_primary_label(format!("grouped as `{pref}({inner_s})`"))
                    .with_note(format!(
                        "In Flurry patterns, prefix `{trimmed_pref}` extends across `{op_desc}` unless parentheses say otherwise."
                    ))
                    .with_help(format!(
                        "Add parentheses to make the pattern boundary clear: `{pref}({inner_s})`{extra_alt}."
                    )),
                );
            }
        }
    }

    /// Warn when assigning to `*ptr = value` (`Assign` whose LHS is `PointerType`)!
    pub(crate) fn check_deref_assignment_habit(&mut self, lhs: NodeIndex, assign_span: Span) {
        if !self.is_parenthesized(lhs)
            && self.ast.get_node_kind(lhs) == Some(NodeKind::PointerType)
            && let Some(&inner) = self.ast.get_children(lhs).first()
        {
            let ptr_s = self.snippet_or(inner, "ptr");
            let lhs_span = self.ast.get_span(lhs).unwrap_or(assign_span);
            self.record_warning(
                ParseWarning::new(
                    PARSE_WARN_DEREF_ASSIGN,
                    format!(
                        "`*{ptr_s}` on the left of `=` is a pointer type expression, not a pointer dereference"
                    ),
                    lhs_span,
                )
                .with_primary_label("parsed as pointer type `*T`, not dereferencing")
                .with_help(format!(
                    "Use postfix `.*` to write through a pointer in Flurry: `{ptr_s}.* = ...`."
                )),
            );
        }
    }

    fn infix_expr_info(kind: NodeKind) -> Option<(&'static str, InfixCat)> {
        Some(match kind {
            NodeKind::BoolImplies => ("==>", InfixCat::Implies),
            NodeKind::BoolOr => ("or", InfixCat::Logical),
            NodeKind::BoolAnd => ("and", InfixCat::Logical),
            NodeKind::BoolEq => ("==", InfixCat::Comparison),
            NodeKind::BoolNotEq => ("!=", InfixCat::Comparison),
            NodeKind::BoolLt => ("<", InfixCat::Comparison),
            NodeKind::BoolLtEq => ("<=", InfixCat::Comparison),
            NodeKind::BoolGt => (">", InfixCat::Comparison),
            NodeKind::BoolGtEq => (">=", InfixCat::Comparison),
            NodeKind::BoolMatches => ("matches", InfixCat::Comparison),
            NodeKind::TypedWith => (":", InfixCat::Typed),
            NodeKind::TraitBound => (":-", InfixCat::Typed),
            NodeKind::Arrow => ("->", InfixCat::Arrow),
            NodeKind::Add => ("+", InfixCat::Arithmetic),
            NodeKind::Sub => ("-", InfixCat::Arithmetic),
            NodeKind::Mul => ("*", InfixCat::Arithmetic),
            NodeKind::Div => ("/", InfixCat::Arithmetic),
            NodeKind::Mod => ("%", InfixCat::Arithmetic),
            NodeKind::AddAdd => ("++", InfixCat::Arithmetic),
            NodeKind::Pipe => ("|", InfixCat::Pipe),
            NodeKind::PipePrepend => ("|>", InfixCat::Pipe),
            _ => return None,
        })
    }

    // =========================================================================
    // 4. Synchronized Multi-Error Recovery
    // =========================================================================

    pub(crate) fn is_item_or_stmt_sync_keyword(kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Fn
                | TokenKind::Pub
                | TokenKind::Private
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Union
                | TokenKind::Trait
                | TokenKind::Impl
                | TokenKind::Extend
                | TokenKind::Derive
                | TokenKind::Mod
                | TokenKind::Effect
                | TokenKind::Typealias
                | TokenKind::Newtype
                | TokenKind::Case
                | TokenKind::Test
                | TokenKind::Template
                | TokenKind::Pure
                | TokenKind::Comptime
                | TokenKind::Let
                | TokenKind::Var
                | TokenKind::Const
                | TokenKind::Use
                | TokenKind::Return
                | TokenKind::Resume
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::If
                | TokenKind::While
                | TokenKind::For
                | TokenKind::When
        )
    }

    /// Advance past a broken statement or top-level item up to the next safe
    /// synchronization boundary (`;`, closing `}`, or a fresh item/statement
    /// keyword at bracket depth 0). Always makes forward progress if not at
    /// `closing` / `Eof`.
    pub(crate) fn synchronize_in_block_or_file(&mut self, closing: Option<TokenKind>) {
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut steps = 0usize;

        while !self.is_eof() {
            let next = self.peek_next_token().kind;

            if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
                if Some(next) == closing {
                    return;
                }
                if next == TokenKind::Semi {
                    self.eat_tokens(1);
                    return;
                }
                if steps > 0 && Self::is_item_or_stmt_sync_keyword(next) {
                    return;
                }
            }

            match next {
                TokenKind::LParen => {
                    paren_depth += 1;
                    self.eat_tokens(1);
                }
                TokenKind::RParen => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                        self.eat_tokens(1);
                    } else if Some(TokenKind::RParen) == closing {
                        return;
                    } else {
                        self.eat_tokens(1);
                    }
                }
                TokenKind::LBracket => {
                    bracket_depth += 1;
                    self.eat_tokens(1);
                }
                TokenKind::RBracket => {
                    if bracket_depth > 0 {
                        bracket_depth -= 1;
                        self.eat_tokens(1);
                    } else if Some(TokenKind::RBracket) == closing {
                        return;
                    } else {
                        self.eat_tokens(1);
                    }
                }
                TokenKind::LBrace => {
                    brace_depth += 1;
                    self.eat_tokens(1);
                }
                TokenKind::RBrace => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                        self.eat_tokens(1);
                        // Finishing a balanced `{ ... }` at depth 0 is also a clean statement boundary!
                        if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 {
                            let _ = self.eat_token(TokenKind::Semi);
                            return;
                        }
                    } else if Some(TokenKind::RBrace) == closing {
                        return;
                    } else {
                        // Stray top-level `}`: consume it so file-scope recovery never stalls!
                        self.eat_tokens(1);
                        return;
                    }
                }
                _ => {
                    self.eat_tokens(1);
                }
            }
            steps += 1;
        }
    }

    /// Advance past a broken element inside a comma-delimited bracketed list
    /// (`(...)`, `[...]`, `{...}`, `<...>`) to the next `,` (consuming it) or
    /// the matching `closing` token (leaving it for `try_multi_with_bracket`).
    pub(crate) fn synchronize_in_comma_list(&mut self, closing: TokenKind) {
        let mut paren_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut brace_depth = 0usize;

        while !self.is_eof() {
            let next = self.peek_next_token().kind;

            if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
                if next == closing {
                    return;
                }
                if matches!(next, TokenKind::Comma | TokenKind::Semi) {
                    self.eat_tokens(1);
                    return;
                }
                // Don't swallow an enclosing `}` or `;` if an inner `)` or `>` was forgotten!
                if matches!(next, TokenKind::RBrace) && closing != TokenKind::RBrace {
                    return;
                }
            }

            match next {
                TokenKind::LParen => {
                    paren_depth += 1;
                    self.eat_tokens(1);
                }
                TokenKind::RParen => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                        self.eat_tokens(1);
                    } else {
                        self.eat_tokens(1);
                        return;
                    }
                }
                TokenKind::LBracket => {
                    bracket_depth += 1;
                    self.eat_tokens(1);
                }
                TokenKind::RBracket => {
                    if bracket_depth > 0 {
                        bracket_depth -= 1;
                        self.eat_tokens(1);
                    } else {
                        self.eat_tokens(1);
                        return;
                    }
                }
                TokenKind::LBrace => {
                    brace_depth += 1;
                    self.eat_tokens(1);
                }
                TokenKind::RBrace => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                        self.eat_tokens(1);
                    } else {
                        return;
                    }
                }
                _ => {
                    self.eat_tokens(1);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfixCat {
    Implies,
    Logical,
    Comparison,
    Typed,
    Arrow,
    Arithmetic,
    Pipe,
}
