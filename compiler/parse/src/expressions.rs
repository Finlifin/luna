use super::basic::Rule;
use super::error::*;
use super::operators::get_expr_op_info;
use super::parser::*;
use ast::*;
use lex::TokenKind;

/// Expression boundary options shared by nested Pratt parsing.
#[derive(Debug, Clone, Copy)]
pub struct ExprOption {
    pub no_extended_call: bool,
    pub stop_at_pipe: bool,
    pub stop_at_do: bool,
    pub stop_at_pattern_gt: bool,
    pub stop_at_pattern_context: bool,
    pub stop_at_type_qualifier: bool,
    pub stop_at_call: bool,
    pub stop_at_named_parameter: bool,
    /// A following caret starts the next prefix on the same attributed element.
    pub stop_at_attribute: bool,
}

impl ExprOption {
    pub fn new() -> Self {
        Self {
            no_extended_call: false,
            stop_at_pipe: false,
            stop_at_do: false,
            stop_at_pattern_gt: false,
            stop_at_pattern_context: false,
            stop_at_type_qualifier: false,
            stop_at_call: false,
            stop_at_named_parameter: false,
            stop_at_attribute: false,
        }
    }

    pub fn with_no_extended_call(mut self, no_extended_call: bool) -> Self {
        self.no_extended_call = no_extended_call;
        self
    }
}

impl Default for ExprOption {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser<'_> {
    #[inline]
    pub fn try_expr(&mut self) -> ParseResult {
        self.try_expr_with_option(ExprOption::default())
    }

    pub fn try_expr_with_option(&mut self, option: ExprOption) -> ParseResult {
        self.scoped(|p| p.try_expr_pratt(0, option))
    }

    #[inline]
    /// Parse an expression while disabling `extended_application`.
    pub fn try_expr_without_extended_call(&mut self) -> ParseResult {
        self.try_expr_with_option(ExprOption::new().with_no_extended_call(true))
    }

    /// 使用 Pratt 解析法解析表达式
    ///
    /// # 参数
    /// * `min_prec` - 最小优先级
    /// * `option` - 表达式选项
    ///
    /// # 返回值
    /// 解析结果
    pub fn try_expr_pratt(&mut self, min_prec: i32, option: ExprOption) -> ParseResult {
        self.scoped(|p| {
            // 尝试解析前缀表达式作为左操作数
            let expression_start = p.next_token_span().lo();
            let mut current_left = match p.try_prefix_expr(option)? {
                0 => return Ok(0), // 没有找到有效的前缀表达式
                node => node,
            };

            // 循环处理操作符和右操作数
            loop {
                let token = p.peek_next_token();
                if option.stop_at_attribute && token.kind == TokenKind::Caret {
                    break;
                }
                if option.stop_at_named_parameter
                    && p.peek(&[TokenKind::Dot, TokenKind::Id, TokenKind::Colon])
                {
                    break;
                }
                if option.stop_at_call && token.kind == TokenKind::LParen {
                    break;
                }
                if option.stop_at_type_qualifier
                    && matches!(
                        token.kind,
                        TokenKind::Hash
                            | TokenKind::Bang
                            | TokenKind::Question
                            | TokenKind::Ampersand
                            | TokenKind::Percent
                            | TokenKind::SeparatedPercent
                    )
                {
                    break;
                }
                if option.stop_at_pattern_gt
                    && matches!(token.kind, TokenKind::Gt | TokenKind::SeparatedGt)
                {
                    break;
                }
                if option.stop_at_pattern_context
                    && (p.peek(&[TokenKind::Minus, TokenKind::Pipe])
                        || p.peek(&[TokenKind::SeparatedMinus, TokenKind::Pipe])
                        || p.peek(&[TokenKind::Minus, TokenKind::PipeGt])
                        || p.peek(&[TokenKind::SeparatedMinus, TokenKind::PipeGt]))
                {
                    break;
                }
                if option.stop_at_do && token.kind == TokenKind::Do {
                    break;
                }
                if option.stop_at_pipe && token.kind == TokenKind::Pipe {
                    break;
                }
                // A closed one-scalar character after `}` is lexically
                // ambiguous with two apostrophes in a take-view chain. Only
                // expression continuation can choose the latter; statement
                // parsing must still accept `if condition {} 'a';`.
                if token.kind == TokenKind::Char && min_prec <= 110 {
                    let views = p.try_compact_take_views(current_left)?;
                    if views != 0 {
                        current_left = views;
                        continue;
                    }
                }
                let op_info = get_expr_op_info(token.kind);

                // Check Flurry-sensitive spacing rules and cross-language operator habits
                // at the appropriate precedence layer so `lhs` snippets match natural grouping.
                if !option.stop_at_type_qualifier {
                    let check_here = min_prec == 0
                        || (token.kind == TokenKind::Pipe && min_prec <= 20)
                        || (token.kind == TokenKind::BangEq && min_prec <= 30)
                        || (matches!(token.kind, TokenKind::Plus | TokenKind::Minus)
                            && min_prec <= 60)
                        || (matches!(
                            token.kind,
                            TokenKind::Star | TokenKind::Slash | TokenKind::Percent
                        ) && min_prec <= 70);
                    if check_here
                        && let Some(err) = p.check_post_expr_heuristics(current_left)
                    {
                        return Err(err);
                    }
                }

                // 如果操作符无效或优先级太低，则退出循环
                if op_info.node_kind == NodeKind::Invalid || op_info.prec < min_prec {
                    break;
                }

                match p.try_postfix_expr(token.kind, current_left, option) {
                    Ok(node) if node != 0 => {
                        let full = p.current_span();
                        if let Some(sp) = p.ast.get_span(node) {
                            p.ast.spans[node as usize] =
                                rustc_span::Span::new(sp.lo().min(full.lo()), sp.hi().max(full.hi()));
                        }
                        current_left = node;
                    }
                    Err(ParseError::MeetPostExtendedCallStart) => {
                        // 如果遇到 MeetPostExtendedCallStart，跳过后缀表达式处理
                        break;
                    }
                    Err(ParseError::MeetPostId) => {
                        // 如果遇到 MeetPostId，跳过后缀表达式处理
                        break;
                    }
                    Err(e) => return Err(e),

                    _ => {
                        let op_span = p.next_token_span();
                        let op_str = token.kind.lexme().trim();
                        // 消耗操作符标记
                        p.eat_tokens(1);

                        // A function's result is itself a complete type (`->`), and logical
                        // implication (`==>`) associates to the right.
                        let right_prec = if matches!(token.kind, TokenKind::Arrow | TokenKind::EqEqEq) {
                            op_info.prec
                        } else {
                            op_info.prec + 1
                        };
                        let right = match p.try_expr_pratt(right_prec, option)? {
                            0 => {
                                let found = p.peek_next_token().kind;
                                return Err(
                                    ParseError::invalid_syntax(
                                        format!(
                                            "expected a right-hand operand after binary operator `{op_str}`, but ran into {}",
                                            describe_token(found)
                                        ),
                                        found,
                                        p.next_token_span(),
                                    )
                                    .with_primary_label(format!(
                                        "expected an expression here after `{op_str}`"
                                    ))
                                    .with_secondary_label(
                                        op_span,
                                        format!("`{op_str}` needs a right-hand operand"),
                                    ),
                                );
                            }

                            node => node,
                        };

                        // 创建二元操作符节点
                        current_left = NodeBuilder::new(
                            op_info.node_kind,
                            rustc_span::Span::new(expression_start, p.current_span().hi()),
                        )
                        .add_single_child(current_left)
                        .add_single_child(right)
                        .build(&mut p.ast);

                        p.check_expr_parenthesization(current_left);
                    }
                }
            }

            // 返回最终构建的表达式
            Ok(current_left)
        })
    }

    fn try_compact_take_views(&mut self, left: NodeIndex) -> ParseResult {
        let token = self.peek_next_token();
        if token.kind != TokenKind::Char || self.get_token(self.cursor + 2).kind != TokenKind::Id {
            return Ok(0);
        }
        let spelling = self.token_text(&token);
        let Some(name) = spelling
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
        else {
            return Ok(0);
        };
        // A decoded escape, whitespace or punctuation cannot spell a view
        // identifier. Ask the lexer about the raw spelling, not its value.
        let (tokens, _, errors) = lex::lex(name, rustc_span::BytePos(0));
        if !errors.is_empty() || tokens.len() != 3 || tokens[1].kind != TokenKind::Id {
            return Ok(0);
        }
        let first_span = rustc_span::Span::new(
            self.start_pos + rustc_span::BytePos((token.from + 1) as u32),
            self.start_pos + rustc_span::BytePos((token.to - 1) as u32),
        );
        let (hi, lo) = lex::Symbol::intern(name).to_raw_parts();
        let first_name = NodeBuilder::new(NodeKind::Id, first_span)
            .add_single_child(hi)
            .add_single_child(lo)
            .build(&mut self.ast);
        let base_start = self.ast.get_span(left).unwrap().lo();
        let first = NodeBuilder::new(
            NodeKind::TakeView,
            rustc_span::Span::new(base_start, first_span.hi()),
        )
        .add_single_child(left)
        .add_single_child(first_name)
        .build(&mut self.ast);
        self.eat_tokens(1);
        let second_name = self.try_id()?;
        let second_span = self.ast.get_span(second_name).unwrap();
        Ok(NodeBuilder::new(
            NodeKind::TakeView,
            rustc_span::Span::new(base_start, second_span.hi()),
        )
        .add_single_child(first)
        .add_single_child(second_name)
        .build(&mut self.ast))
    }

    pub fn try_prefix_expr(&mut self, option: ExprOption) -> ParseResult {
        self.scoped(|p| {
            let token = p.peek_next_token();
            let node = match token.kind {
                // for those atomic expressions
                TokenKind::Int
                | TokenKind::IntBin
                | TokenKind::IntOct
                | TokenKind::IntHex
                | TokenKind::Real
                | TokenKind::RealSci
                | TokenKind::Str
                | TokenKind::Char
                | TokenKind::Id
                | TokenKind::False
                | TokenKind::True
                | TokenKind::SelfCap
                | TokenKind::SelfLower
                | TokenKind::Underscore
                | TokenKind::Null
                | TokenKind::Undefined => p.try_atomic(),

                TokenKind::If => p.try_if_statement(),
                TokenKind::Quote => p.try_code_quote(),
                TokenKind::Tilde => p.try_code_splice(),
                TokenKind::Lt | TokenKind::SeparatedLt => p.try_contextual_expr_type(),
                TokenKind::When => p.try_when_statement(),
                TokenKind::LParen => p.try_unit_or_parenthesis_or_tuple(),
                TokenKind::LBracket => p.try_list(),
                TokenKind::LBrace => {
                    if p.is_token_buffer_start() {
                        p.try_token_buffer()
                    } else {
                        p.try_object()
                    }
                }
                TokenKind::Dot => p.try_prefix_range_expr_or_symbol(option),
                TokenKind::Pipe => p.try_lambda(option),
                TokenKind::Forall => p.try_forall_prefix(option),
                TokenKind::Exists => p.try_bool_exists(option),
                TokenKind::Hash => p.try_effect_qualified_type(option),
                TokenKind::Bang => p.try_error_qualified_type(option),
                TokenKind::Ampersand => p.try_reachability_type(option),
                TokenKind::Percent | TokenKind::SeparatedPercent => {
                    p.try_closure_qualified_type(option)
                }
                TokenKind::Minus | TokenKind::SeparatedMinus => {
                    p.try_prefix_unary_expr(token.kind, NodeKind::Negative, 90, option)
                }
                TokenKind::Not => {
                    p.try_prefix_unary_expr(TokenKind::Not, NodeKind::BoolNot, 90, option)
                }
                TokenKind::Error => {
                    p.try_prefix_unary_expr(TokenKind::Error, NodeKind::ErrorNew, 90, option)
                }
                TokenKind::Dyn => {
                    p.try_prefix_unary_expr(TokenKind::Dyn, NodeKind::TraitObjectType, 90, option)
                }
                TokenKind::Star => {
                    p.try_prefix_unary_expr(TokenKind::Star, NodeKind::PointerType, 90, option)
                }
                TokenKind::Question => {
                    p.try_prefix_unary_expr(TokenKind::Question, NodeKind::OptionalType, 90, option)
                }
                TokenKind::Lift => {
                    p.try_prefix_unary_expr(TokenKind::Lift, NodeKind::LiftType, 90, option)
                }
                TokenKind::Do => p.try_do_block_expr(),
                TokenKind::Refines => {
                    p.eat_tokens(1);
                    if !p.eat_token(TokenKind::LBrace) {
                        return Err(ParseError::invalid_syntax(
                            "Expected `{` after refines".into(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    let requirement = p.try_expr()?;
                    if requirement == 0 || !p.eat_token(TokenKind::RBrace) {
                        return Err(ParseError::invalid_syntax(
                            "Expected refinement expression and `}`".into(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    let base = p.try_expr_pratt(0, option)?;
                    if base == 0 {
                        return Err(ParseError::invalid_syntax(
                            "Expected base type after refinement".into(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    Ok(NodeBuilder::new(NodeKind::RefinedType, p.current_span())
                        .add_single_child(requirement)
                        .add_single_child(base)
                        .build(&mut p.ast))
                }
                TokenKind::Defer => {
                    p.try_prefix_unary_expr(TokenKind::Defer, NodeKind::Defer, 0, option)
                }
                TokenKind::Async => {
                    // async { ... } = AsyncBlock; async fn(...) = fn_type modifier
                    if p.peek(&[TokenKind::Async, TokenKind::LBrace]) {
                        p.try_keyword_block_expr(TokenKind::Async, NodeKind::AsyncBlock)
                    } else {
                        // async is not a fn_type modifier in syntax, treat as block
                        p.try_keyword_block_expr(TokenKind::Async, NodeKind::AsyncBlock)
                    }
                }
                TokenKind::Unsafe => {
                    // unsafe { ... } = UnsafeBlock; unsafe fn(...) = fn_type modifier
                    if p.peek(&[TokenKind::Unsafe, TokenKind::LBrace]) {
                        p.try_keyword_block_expr(TokenKind::Unsafe, NodeKind::UnsafeBlock)
                    } else {
                        p.try_fn_type()
                    }
                }
                TokenKind::Comptime => {
                    // comptime { ... } = ComptimeBlock; comptime fn(...) = fn_type modifier
                    if p.peek(&[TokenKind::Comptime, TokenKind::LBrace]) {
                        p.try_keyword_block_expr(TokenKind::Comptime, NodeKind::ComptimeBlock)
                    } else {
                        p.try_fn_type()
                    }
                }
                TokenKind::Atomic => p.try_atomic_block_expr(),

                // fn_type -> pure? comptime? inline? (unsafe|spec|verified)? fn(parameter_type*)
                TokenKind::Fn => p.try_fn_type(),
                TokenKind::Inline
                    if [TokenKind::If, TokenKind::When, TokenKind::For]
                        .iter()
                        .any(|next| p.peek(&[TokenKind::Inline, *next])) =>
                {
                    p.try_inline_statement()
                }
                TokenKind::Pure | TokenKind::Inline | TokenKind::Spec | TokenKind::Verified => {
                    p.try_fn_type()
                }

                _ => {
                    if let Some(err) = p.check_prefix_expr_heuristics() {
                        return Err(err);
                    }
                    Ok(0)
                }
            }?;
            if node != 0 {
                p.check_expr_parenthesization(node);
            }
            Ok(node)
        })
    }

    /// 解析单元类型、括号表达式或元组
    /// tuple -> ( expr, expr* )
    /// parenthesis -> ( expr )
    /// unit -> ()
    fn try_unit_or_parenthesis_or_tuple(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LParen], |p| {
            let open_span = p.next_token_span();
            p.eat_tokens(1); // 消耗 '('

            // 检查是否是空的单元类型 ()
            if p.eat_token(TokenKind::RParen) {
                return Ok(NodeBuilder::new(NodeKind::Unit, p.current_span()).build(&mut p.ast));
            }

            // 解析第一个表达式
            let first_expr = p.try_expr()?;
            if first_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an expression or `)` inside `(...)`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(open_span, "this `(` was opened here"));
            }

            // 检查是否有逗号，决定是元组还是括号表达式
            if p.eat_token(TokenKind::Comma) {
                // 解析剩余的元组元素
                let mut elements =
                    p.try_multi(&[Rule::comma("tuple element", |p| p.try_expr())])?;
                elements.insert(0, first_expr); // 将第一个表达式添加到元素列表

                if !p.eat_token(TokenKind::RParen) {
                    return Err(ParseError::unexpected_token(
                        TokenKind::RParen,
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    )
                    .with_secondary_label(open_span, "this `(` was opened here"));
                }

                Ok(NodeBuilder::new(NodeKind::Tuple, p.current_span())
                    .add_multiple_children(elements)
                    .build(&mut p.ast))
            } else {
                // 这是一个括号表达式
                if !p.eat_token(TokenKind::RParen) {
                    return Err(ParseError::unexpected_token(
                        TokenKind::RParen,
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    )
                    .with_secondary_label(open_span, "this `(` was opened here"));
                }

                p.mark_parenthesized(first_expr);
                Ok(first_expr)
            }
        })
    }

    pub fn try_prefix_range_expr_or_symbol(&mut self, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dot], |p| {
            if p.peek(&[TokenKind::Dot, TokenKind::Dot]) {
                p.eat_tokens(2);

                if p.eat_token(TokenKind::Eq) {
                    let to = p.try_expr_with_option(option)?;
                    if to == 0 {
                        return Err(ParseError::invalid_syntax(
                            "Expected expression after '..='".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }

                    return Ok(
                        NodeBuilder::new(NodeKind::RangeToInclusive, p.current_span())
                            .add_single_child(to)
                            .build(&mut p.ast),
                    );
                } else {
                    let to = p.try_expr_with_option(option)?;
                    if to == 0 {
                        return Ok(NodeBuilder::new(NodeKind::RangeFull, p.current_span())
                            .build(&mut p.ast));
                    }

                    return Ok(NodeBuilder::new(NodeKind::RangeTo, p.current_span())
                        .add_single_child(to)
                        .build(&mut p.ast));
                }
            }

            p.try_symbol()
        })
    }

    /// 解析列表表达式
    /// list -> [ expr* ]
    fn try_list(&mut self) -> ParseResult {
        let nodes = self.try_multi_with_bracket(
            &[Rule::comma("list element", |p| p.try_expr())],
            (TokenKind::LBracket, TokenKind::RBracket),
        )?;

        Ok(NodeBuilder::new(NodeKind::ListOf, self.current_span())
            .add_multiple_children(nodes)
            .build(&mut self.ast))
    }

    /// True when the next two tokens are an adjacent `{{` opening a token buffer literal.
    pub(crate) fn is_token_buffer_start(&self) -> bool {
        let first = self.peek_nth_token(1);
        let second = self.peek_nth_token(2);
        first.kind == TokenKind::LBrace
            && second.kind == TokenKind::LBrace
            && Self::tokens_adjacent(first, second)
    }

    /// Parse a token buffer literal: `{{ token_part* }}` where `token_part` is a raw token,
    /// an escaped `$$`, or an interpolation `$id` / `$self` / `${expr}` / `$(expr)`.
    pub fn try_token_buffer(&mut self) -> ParseResult {
        self.scoped(|p| {
            let first = p.peek_nth_token(1);
            let second = p.peek_nth_token(2);
            if first.kind != TokenKind::LBrace
                || second.kind != TokenKind::LBrace
                || !Self::tokens_adjacent(first, second)
            {
                return Ok(0);
            }

            let open_span =
                rustc_span::Span::new(p.token_span(first).lo(), p.token_span(second).hi());
            p.eat_tokens(2);

            let mut parts: Vec<NodeIndex> = Vec::new();
            let mut brace_depth: usize = 0;

            while !p.is_eof() {
                let tok = p.peek_nth_token(1);
                let next = p.peek_nth_token(2);

                if brace_depth == 0
                    && tok.kind == TokenKind::RBrace
                    && next.kind == TokenKind::RBrace
                    && Self::tokens_adjacent(tok, next)
                {
                    let close_hi = p.token_span(next).hi();
                    p.eat_tokens(2);
                    let full_span = rustc_span::Span::new(open_span.lo(), close_hi);
                    return Ok(NodeBuilder::new(NodeKind::TokenBuffer, full_span)
                        .add_multiple_children(parts)
                        .build(&mut p.ast));
                }

                if tok.kind == TokenKind::Dollar {
                    let tok_span = p.token_span(tok);
                    let adjacent = Self::tokens_adjacent(tok, next);
                    if adjacent && next.kind == TokenKind::Dollar {
                        let dollar_span =
                            rustc_span::Span::new(tok_span.lo(), p.token_span(next).hi());
                        p.eat_tokens(2);
                        let node = NodeBuilder::new(NodeKind::TokenBufferDollar, dollar_span)
                            .build(&mut p.ast);
                        parts.push(node);
                        continue;
                    }
                    if adjacent && next.kind == TokenKind::Id {
                        p.eat_tokens(1);
                        let id = p.try_id()?;
                        let hi = p.ast.get_span(id).map_or(tok_span.hi(), |s| s.hi());
                        let interp_span = rustc_span::Span::new(tok_span.lo(), hi);
                        let node = NodeBuilder::new(NodeKind::TokenBufferInterp, interp_span)
                            .add_single_child(id)
                            .build(&mut p.ast);
                        parts.push(node);
                        continue;
                    }
                    if adjacent && next.kind == TokenKind::SelfLower {
                        p.eat_tokens(1);
                        let self_node = p.try_atomic()?;
                        let hi = p
                            .ast
                            .get_span(self_node)
                            .map_or(tok_span.hi(), |s| s.hi());
                        let interp_span = rustc_span::Span::new(tok_span.lo(), hi);
                        let node = NodeBuilder::new(NodeKind::TokenBufferInterp, interp_span)
                            .add_single_child(self_node)
                            .build(&mut p.ast);
                        parts.push(node);
                        continue;
                    }
                    if adjacent && next.kind == TokenKind::LBrace {
                        p.eat_tokens(2);
                        let expr = p.try_expr()?;
                        if expr == 0 {
                            return Err(ParseError::invalid_syntax(
                                "expected an expression inside `${...}` in token buffer"
                                    .to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            )
                            .with_secondary_label(tok_span, "interpolation starts here"));
                        }
                        if !p.eat_token(TokenKind::RBrace) {
                            return Err(ParseError::unexpected_token(
                                TokenKind::RBrace,
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            )
                            .with_secondary_label(
                                p.token_span(next),
                                "interpolation `${` opened here",
                            ));
                        }
                        let close_hi = p.prev_token_span().hi();
                        let interp_span = rustc_span::Span::new(tok_span.lo(), close_hi);
                        let node = NodeBuilder::new(NodeKind::TokenBufferInterp, interp_span)
                            .add_single_child(expr)
                            .build(&mut p.ast);
                        parts.push(node);
                        continue;
                    }
                    if adjacent && next.kind == TokenKind::LParen {
                        p.eat_tokens(2);
                        let expr = p.try_expr()?;
                        if expr == 0 {
                            return Err(ParseError::invalid_syntax(
                                "expected an expression inside `$(...)` in token buffer"
                                    .to_string(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            )
                            .with_secondary_label(tok_span, "interpolation starts here"));
                        }
                        if !p.eat_token(TokenKind::RParen) {
                            return Err(ParseError::unexpected_token(
                                TokenKind::RParen,
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            )
                            .with_secondary_label(
                                p.token_span(next),
                                "interpolation `$(` opened here",
                            ));
                        }
                        let close_hi = p.prev_token_span().hi();
                        let interp_span = rustc_span::Span::new(tok_span.lo(), close_hi);
                        let node = NodeBuilder::new(NodeKind::TokenBufferInterp, interp_span)
                            .add_single_child(expr)
                            .build(&mut p.ast);
                        parts.push(node);
                        continue;
                    }
                    return Err(ParseError::invalid_syntax(
                        "invalid `$` in token buffer `{{ ... }}`; use `$ident`, `$self`, `${expr}`, `$(expr)`, or `$$` for a literal `$`".to_string(),
                        tok.kind,
                        tok_span,
                    )
                    .with_secondary_label(open_span, "token buffer opened here"));
                }

                if tok.kind == TokenKind::LBrace {
                    brace_depth += 1;
                } else if tok.kind == TokenKind::RBrace {
                    brace_depth = brace_depth.saturating_sub(1);
                }

                let tok_span = p.token_span(tok);
                p.eat_tokens(1);
                let node = NodeBuilder::new(NodeKind::TokenBufferToken, tok_span)
                    .build(&mut p.ast);
                parts.push(node);
            }

            Err(ParseError::invalid_syntax(
                "unterminated token buffer literal `{{ ... }}`; expected closing `}}`".to_string(),
                p.peek_next_token().kind,
                p.next_token_span(),
            )
            .with_secondary_label(open_span, "unclosed `{{` opened here"))
        })
    }

    /// 解析对象表达式
    /// object -> { property* }
    fn try_object(&mut self) -> ParseResult {
        let nodes = self.try_multi_with_bracket(
            &[
                Rule::comma("property", |p| p.try_property()),
                Rule::comma("child expr", |p| p.try_expr()),
            ],
            (TokenKind::LBrace, TokenKind::RBrace),
        )?;

        Ok(NodeBuilder::new(NodeKind::Object, self.current_span())
            .add_multiple_children(nodes)
            .build(&mut self.ast))
    }

    /// 解析lambda表达式
    /// lambda -> |(id | param)*| return_type? block|expr
    fn try_lambda(&mut self, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Pipe], |p| {
            // 解析参数列表
            let params = p.try_multi_with_bracket(
                &[Rule::comma("lambda parameter", |p| {
                    p.scoped(|p| {
                        let id = p.try_id()?;
                        if id == 0 {
                            return Ok(0);
                        }
                        if !p.eat_token(TokenKind::Colon) {
                            return Ok(id);
                        }
                        let mut option = ExprOption::new().with_no_extended_call(true);
                        option.stop_at_pipe = true;
                        let ty = p.try_expr_with_option(option)?;
                        if ty == 0 {
                            return Err(ParseError::invalid_syntax(
                                "expected lambda parameter type".into(),
                                p.peek_next_token().kind,
                                p.next_token_span(),
                            ));
                        }
                        Ok(NodeBuilder::new(NodeKind::TypeBoundParam, p.current_span())
                            .add_single_child(id)
                            .add_single_child(ty)
                            .build(&mut p.ast))
                    })
                })],
                (TokenKind::Pipe, TokenKind::Pipe),
            )?;

            // 解析可选的返回类型
            let return_type = if p.eat_token(TokenKind::Arrow) {
                let rt = p.try_expr_without_extended_call()?;
                if rt == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected return type after '->' in lambda".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                rt
            } else {
                0
            };

            // 解析函数体（块或表达式）
            let body = match if p.is_token_buffer_start() {
                0
            } else {
                p.try_block()?
            } {
                0 => {
                    let expr = p.try_expr_with_option(option)?;
                    if expr == 0 {
                        return Err(ParseError::invalid_syntax(
                            "Expected block or expression as lambda body".to_string(),
                            p.peek_next_token().kind,
                            p.next_token_span(),
                        ));
                    }
                    expr
                }
                block => block,
            };

            // Build as (a, b, N): return_type, body, then params
            Ok(NodeBuilder::new(NodeKind::Lambda, p.current_span())
                .add_single_child(return_type)
                .add_single_child(body)
                .add_multiple_children(params)
                .build(&mut p.ast))
        })
    }

    /// Parse `forall` prefix — dispatches to:
    /// - `forall<parameter*> expr`   → ForallType (a, N)
    /// - `forall type_bound_param* => expr` → BoolForall (a, N)
    fn try_forall_prefix(&mut self, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Forall], |p| {
            p.eat_tokens(1); // consume 'forall'

            // If next token is `<`, this is ForallType
            if p.peek(TokenKind::Lt.as_ref()) {
                let params = p.try_multi_with_bracket(
                    &[Rule::comma("forall type parameter", |p| p.try_param())],
                    (TokenKind::Lt, TokenKind::Gt),
                )?;

                let expr = p.try_expr_with_option(option)?;
                if expr == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected expression after forall type parameters".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                // Build as (a, N): body first, then params
                Ok(NodeBuilder::new(NodeKind::ForallType, p.current_span())
                    .add_single_child(expr)
                    .add_multiple_children(params)
                    .build(&mut p.ast))
            } else {
                // bool_forall: forall type_bound_param* => expr
                let params =
                    p.try_multi(&[Rule::comma("type bound parameter", |p| p.try_param())])?;
                if params.is_empty() {
                    return Err(ParseError::invalid_syntax(
                        "Expected at least one parameter after `forall`".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                if !p.eat_token(TokenKind::FatArrow) {
                    return Err(ParseError::unexpected_token(
                        TokenKind::FatArrow,
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                let body = p.try_expr_with_option(option)?;
                if body == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected expression after `=>` in forall".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }

                Ok(NodeBuilder::new(NodeKind::BoolForall, p.current_span())
                    .add_single_child(body)
                    .add_multiple_children(params)
                    .build(&mut p.ast))
            }
        })
    }

    /// bool_exists: `exists type_bound_param* => expr`
    fn try_bool_exists(&mut self, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Exists], |p| {
            p.eat_tokens(1); // consume 'exists'

            let params = p.try_multi(&[Rule::comma("type bound parameter", |p| p.try_param())])?;
            if params.is_empty() {
                return Err(ParseError::invalid_syntax(
                    "Expected at least one parameter after `exists`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if !p.eat_token(TokenKind::FatArrow) {
                return Err(ParseError::unexpected_token(
                    TokenKind::FatArrow,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let body = p.try_expr_with_option(option)?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected expression after `=>` in exists".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::BoolExists, p.current_span())
                .add_single_child(body)
                .add_multiple_children(params)
                .build(&mut p.ast))
        })
    }

    /// Operation labels are expressions; only their runtime arguments are
    /// patterns. In particular `op<T>(x)` observes T rather than binding it.
    fn try_effect_arm(&mut self) -> ParseResult {
        self.scoped(|p| {
            let mut option = ExprOption::new().with_no_extended_call(true);
            option.stop_at_call = true;
            let head = p.try_expr_pratt(100, option)?;
            if head == 0 {
                return Ok(0);
            }
            let head = NodeBuilder::new(NodeKind::ExprAsPattern, p.current_span())
                .add_single_child(head)
                .build(&mut p.ast);
            let fields = p.try_multi_with_bracket(
                &[Rule::comma("effect parameter pattern", |p| p.try_pattern())],
                (TokenKind::LParen, TokenKind::RParen),
            )?;
            let mut pattern = NodeBuilder::new(NodeKind::ApplicationPattern, p.current_span())
                .add_single_child(head)
                .add_multiple_children(fields)
                .build(&mut p.ast);
            if p.eat_token(TokenKind::If) {
                let condition = p.try_expr_without_extended_call()?;
                if condition == 0 {
                    return Err(ParseError::invalid_syntax(
                        "Expected effect arm guard".into(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                pattern = NodeBuilder::new(NodeKind::IfGuardPattern, p.current_span())
                    .add_single_child(pattern)
                    .add_single_child(condition)
                    .build(&mut p.ast);
            }
            if !p.eat_token(TokenKind::FatArrow) {
                return Err(ParseError::unexpected_token(
                    TokenKind::FatArrow,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            let body = p.try_block_or_statement()?;
            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected effect arm body".into(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(NodeKind::CaseArm, p.current_span())
                .add_single_child(pattern)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    /// 解析效果限定类型
    /// effect_qualified_type -> #expr expr
    fn try_effect_qualified_type(&mut self, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Hash], |p| {
            p.eat_tokens(1); // 消耗 '#'

            // 解析效果列表
            // Another type qualifier begins the result, not a postfix
            // operation on this row. Parentheses provide an inner scope.
            let mut row_option = option.with_no_extended_call(true);
            row_option.stop_at_type_qualifier = true;
            let effect_list = p.try_expr_with_option(row_option)?;
            if effect_list == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an effect list after `#`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 解析类型表达式
            let type_expr = p.try_expr_pratt(50, option.with_no_extended_call(true))?;
            if type_expr == 0 {
                if let Some(err) = p.check_non_block_extended_call_conflict(
                    effect_list,
                    "an effect-qualified type",
                    None,
                ) {
                    return Err(err);
                }
                return Err(ParseError::invalid_syntax(
                    "expected a result type after the `#...` effect qualifier".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(
                NodeBuilder::new(NodeKind::EffectQualifiedType, p.current_span())
                    .add_single_child(effect_list)
                    .add_single_child(type_expr)
                    .build(&mut p.ast),
            )
        })
    }

    /// 解析错误限定类型
    /// error_qualified_type -> !expr expr
    fn try_error_qualified_type(&mut self, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Bang], |p| {
            let bang_span = p.next_token_span();
            p.eat_tokens(1); // 消耗 '!'

            // 解析错误列表
            let mut context_option = option.with_no_extended_call(true);
            context_option.stop_at_type_qualifier = true;
            let error_list = p.try_expr_with_option(context_option)?;
            if error_list == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an error type after `!` (or use `not` for boolean negation)"
                        .to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_help("For boolean negation, write `not expr` instead of `!expr`."));
            }

            // 解析类型表达式
            let type_expr = p.try_expr_pratt(50, option.with_no_extended_call(true))?;
            if type_expr == 0 {
                if let Some(err) = p.check_non_block_extended_call_conflict(
                    error_list,
                    "an error-qualified type",
                    None,
                ) {
                    return Err(err);
                }
                let err_snip = p.snippet_or(error_list, "expr");
                let full_span = rustc_span::Span::new(bang_span.lo(), p.prev_token_span().hi());
                return Err(ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_bang_negation",
                    format!(
                        "prefix `!{err_snip}` looks like boolean negation, or an error-qualified type `!Err T` missing its payload type"
                    ),
                    full_span,
                )
                .with_found(p.peek_next_token().kind)
                .with_primary_label("`!Err T` expects a payload type here — or use `not` to negate")
                .with_note(
                    "In Flurry, prefix `!Err T` writes an error-qualified type, while boolean negation is written `not expr`.",
                )
                .with_help(format!(
                    "For boolean negation, write `not {err_snip}`; for an error-qualified type, add the result type: `!{err_snip} ResultType`."
                )));
            }

            Ok(
                NodeBuilder::new(NodeKind::ErrorQualifiedType, p.current_span())
                    .add_single_child(error_list)
                    .add_single_child(type_expr)
                    .build(&mut p.ast),
            )
        })
    }

    // &expr expr
    fn try_reachability_type(&mut self, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Ampersand], |p| {
            let amp_span = p.next_token_span();
            p.eat_tokens(1);

            // Detect `&mut expr` right away!
            if p.peek_next_token().kind == TokenKind::Id
                && p.token_content(&p.peek_next_token()).as_deref() == Some("mut")
            {
                let mut_span = p.next_token_span();
                p.eat_tokens(1);
                let target = p.try_expr_with_option(option.with_no_extended_call(true))?;
                let target_snip = if target != 0 {
                    p.snippet_or(target, "value")
                } else {
                    "value".to_string()
                };
                let hi = if target != 0 {
                    p.prev_token_span().hi()
                } else {
                    mut_span.hi()
                };
                return Err(ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_amp_mut_ref",
                    "Flurry takes references with postfix `.ref` rather than `&mut`",
                    rustc_span::Span::new(amp_span.lo(), hi),
                )
                .with_primary_label("use postfix `.ref` instead of `&mut`")
                .with_help(format!(
                    "Write `{target_snip}.ref` to take a reference (mutability comes from `var` bindings)."
                )));
            }

            // 解析可达性列表
            let mut context_option = option.with_no_extended_call(true);
            context_option.stop_at_type_qualifier = true;
            let reachability_set = p.try_expr_with_option(context_option)?;
            if reachability_set == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected a reachability qualifier after `&` (or use postfix `.ref` to take a reference)"
                        .to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_help("To take a reference to a value, write `value.ref`; for a pointer type, write `*T`."));
            }

            // 解析类型表达式
            let type_expr = p.try_expr_pratt(50, option.with_no_extended_call(true))?;
            if type_expr == 0 {
                if let Some(err) = p.check_non_block_extended_call_conflict(
                    reachability_set,
                    "a reachability-qualified type",
                    None,
                ) {
                    return Err(err);
                }
                let reach_snip = p.snippet_or(reachability_set, "value");
                let full_span = rustc_span::Span::new(amp_span.lo(), p.prev_token_span().hi());
                return Err(ParseError::custom(
                    PARSE_FOREIGN_HABIT,
                    "foreign_amp_ref",
                    format!(
                        "prefix `&{reach_snip}` looks like taking a reference, or a reachability-qualified type `&s T` missing its target type"
                    ),
                    full_span,
                )
                .with_found(p.peek_next_token().kind)
                .with_primary_label("use `.ref` to take a reference, or `*T` for a pointer type")
                .with_note(
                    "In Flurry, prefix `&s T` qualifies a type with reachability `s`, whereas taking the address of a place uses postfix `.ref`.",
                )
                .with_help(format!(
                    "To take a reference, write `{reach_snip}.ref`; for a pointer type, write `*{reach_snip}`; for reachability, write `&{reach_snip} TargetType`."
                )));
            }

            Ok(
                NodeBuilder::new(NodeKind::ReachabilityQualifiedType, p.current_span())
                    .add_single_child(reachability_set)
                    .add_single_child(type_expr)
                    .build(&mut p.ast),
            )
        })
    }

    /// do_block -> do { statement* }
    fn try_do_block_expr(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Do.as_ref(), |p| {
            p.eat_tokens(1); // consume 'do'
            let block = p.try_block()?;
            if block == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected block after `do`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(NodeKind::DoBlock, p.current_span())
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    /// Generic keyword block: async/unsafe/comptime { statement* }
    fn try_keyword_block_expr(&mut self, keyword: TokenKind, node_kind: NodeKind) -> ParseResult {
        self.scoped_with_expected_prefix(keyword.as_ref(), |p| {
            p.eat_tokens(1); // consume keyword
            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Ok(0); // Not a block expression — backtrack
            }
            let block = p.try_block()?;
            if block == 0 {
                return Err(ParseError::invalid_syntax(
                    format!("Expected block after `{}`", keyword.lexme()),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(node_kind, p.current_span())
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    /// atomic_block -> atomic(id*) { statement* }
    fn try_atomic_block_expr(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Atomic, TokenKind::LParen], |p| {
            p.eat_tokens(1); // consume 'atomic'
            let ids = p.try_multi_with_bracket(
                &[Rule::comma("atomic id", |p| p.try_id())],
                (TokenKind::LParen, TokenKind::RParen),
            )?;
            let block = p.try_block()?;
            if block == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected block after `atomic(...)`.".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(NodeKind::AtomicBlock, p.current_span())
                .add_single_child(block)
                .add_multiple_children(ids)
                .build(&mut p.ast))
        })
    }

    /// closure_qualified_type -> % (expr | { binding* }) expr
    fn try_closure_qualified_type(&mut self, option: ExprOption) -> ParseResult {
        let prefix = self.peek_next_token().kind;
        if !matches!(prefix, TokenKind::Percent | TokenKind::SeparatedPercent) {
            return Ok(0);
        }
        self.scoped_with_expected_prefix(&[prefix], |p| {
            p.eat_tokens(1);
            let context = if p.peek(&[TokenKind::LBrace]) {
                let bindings = p.try_multi_with_bracket(
                    &[Rule::comma("closure environment binding", |p| {
                        p.scoped(|p| {
                            let name = p.try_id()?;
                            if name == 0 {
                                return Ok(0);
                            }
                            if !p.eat_token(TokenKind::Colon) {
                                return Err(ParseError::invalid_syntax(
                                    "expected `:` in closure environment binding".into(),
                                    p.peek_next_token().kind,
                                    p.next_token_span(),
                                ));
                            }
                            let ty = p.try_expr_without_extended_call()?;
                            if ty == 0 {
                                return Err(ParseError::invalid_syntax(
                                    "expected closure environment binding type".into(),
                                    p.peek_next_token().kind,
                                    p.next_token_span(),
                                ));
                            }
                            let value = if p.eat_token(TokenKind::Eq) {
                                let value = p.try_expr_without_extended_call()?;
                                if value == 0 {
                                    return Err(ParseError::invalid_syntax(
                                        "expected closure environment binding source".into(),
                                        p.peek_next_token().kind,
                                        p.next_token_span(),
                                    ));
                                }
                                value
                            } else {
                                0
                            };
                            Ok(NodeBuilder::new(
                                NodeKind::ClosureEnvironmentBinding,
                                p.current_span(),
                            )
                            .add_single_child(name)
                            .add_single_child(ty)
                            .add_single_child(value)
                            .build(&mut p.ast))
                        })
                    })],
                    (TokenKind::LBrace, TokenKind::RBrace),
                )?;
                NodeBuilder::new(NodeKind::ClosureEnvironment, p.current_span())
                    .add_multiple_children(bindings)
                    .build(&mut p.ast)
            } else {
                let mut context_option = option.with_no_extended_call(true);
                context_option.stop_at_type_qualifier = true;
                p.try_expr_with_option(context_option)?
            };
            if context == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected closure context after `%`".into(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            // Include a function result arrow, but leave comparisons and
            // boolean propositions outside the qualified type.
            // The qualified type still belongs to the surrounding expression;
            // in a lambda parameter its closing pipe is not a union operator.
            let body = p.try_expr_pratt(50, option.with_no_extended_call(true))?;
            if body == 0 {
                if let Some(err) = p.check_non_block_extended_call_conflict(
                    context,
                    "a closure-qualified type",
                    None,
                ) {
                    return Err(err);
                }
                return Err(ParseError::invalid_syntax(
                    "expected closure body type".into(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(
                NodeBuilder::new(NodeKind::ClosureQualifiedType, p.current_span())
                    .add_single_child(context)
                    .add_single_child(body)
                    .build(&mut p.ast),
            )
        })
    }

    /// Parse prefix unary expression (generic method)
    fn try_prefix_unary_expr(
        &mut self,
        token: TokenKind,
        node_kind: NodeKind,
        precedence: i32,
        option: ExprOption,
    ) -> ParseResult {
        self.scoped_with_expected_prefix(&[token], |p| {
            let start = p.next_token_span().lo();
            p.eat_tokens(1); // 消耗前缀token

            // A prefix operand includes tighter postfix operations, but not
            // lower-precedence infix operators. Keep enclosing delimiters (a
            // condition's body, lambda pipe or computed-pattern closing >).
            let expr = p.try_expr_pratt(precedence, option)?;

            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    format!("Expected expression after '{}'", token.lexme()),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(
                node_kind,
                rustc_span::Span::new(start, p.current_span().hi()),
            )
            .add_single_child(expr)
            .build(&mut p.ast))
        })
    }

    /// Try to parse a postfix expression
    fn try_postfix_expr(
        &mut self,
        tag: TokenKind,
        left: NodeIndex,
        opt: ExprOption,
    ) -> ParseResult {
        let node = self.scoped(|p| match tag {
            TokenKind::LParen => p.try_call_expr(left),
            TokenKind::Lt => p.try_diamond_call_expr(left),
            TokenKind::LBrace => p.try_extended_call_expr(left, opt),
            TokenKind::LBracket => p.try_index_call_expr(left),
            TokenKind::Dot => p.try_dot_expr(left),
            TokenKind::Quote => p.try_take_view_expr(left),
            TokenKind::Hash => p.try_effect_handling_expr(left),
            TokenKind::Bang => p.try_error_handling_expr(left),
            TokenKind::Question => p.try_option_expr(left),
            TokenKind::Match => p.try_post_match_expr(left),
            TokenKind::Do => p.try_post_lambda_expr(left, opt),
            TokenKind::Matches => p.try_matches_expr(left, opt),
            TokenKind::Id => p.try_literal_extension_expr(left),
            _ => Ok(0),
        })?;
        if node != 0 {
            self.include_left_span(left, node);
        }
        Ok(node)
    }

    pub(crate) fn include_left_span(&mut self, left: NodeIndex, node: NodeIndex) {
        let left_span = self.ast.get_span(left).expect("postfix receiver");
        let span = self.ast.get_span(node).expect("postfix result");
        self.ast.spans[node as usize] =
            rustc_span::Span::new(left_span.lo().min(span.lo()), left_span.hi().max(span.hi()));
    }

    /// application -> expr(argument*)
    fn try_call_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LParen], |p| {
            let args = p.try_multi_with_bracket(
                &[
                    Rule::comma("optional argument", |p| p.try_optional_arg()),
                    Rule::comma("extend argument", |p| p.try_extend_arg()),
                    Rule::comma("function argument", |p| p.try_expr()),
                ],
                (TokenKind::LParen, TokenKind::RParen),
            )?;

            // 创建函数调用节点
            Ok(NodeBuilder::new(NodeKind::Application, p.current_span())
                .add_single_child(left)
                .add_multiple_children(args)
                .build(&mut p.ast))
        })
    }

    /// normal_form_head_application -> expr<argument*>
    pub(crate) fn try_diamond_call_expr(&mut self, left: NodeIndex) -> ParseResult {
        if !self.peek(TokenKind::Lt.as_ref()) {
            return Ok(0);
        }
        let lt_cursor = self.cursor + 1;
        let res = self.scoped_with_expected_prefix(&[TokenKind::Lt], |p| {
            let args = p.try_multi_with_bracket(
                &[
                    Rule::comma("optional argument", |p| p.try_optional_arg()),
                    Rule::comma("extend argument", |p| p.try_extend_arg()),
                    Rule::comma("function argument", |p| p.try_expr()),
                ],
                (TokenKind::Lt, TokenKind::Gt),
            )?;

            // 创建泛型调用节点
            Ok(
                NodeBuilder::new(NodeKind::NormalFormHeadApplication, p.current_span())
                    .add_single_child(left)
                    .add_multiple_children(args)
                    .build(&mut p.ast),
            )
        });
        let node = match res {
            Ok(node) => node,
            Err(err) => return Err(self.refine_failed_normal_form_lt(left, lt_cursor, err)),
        };
        if node != 0 {
            // Also called directly by normal-form pattern parsing.
            self.include_left_span(left, node);
        }
        Ok(node)
    }

    /// extended_application -> expr { (property | extend_arg | expr)* }
    pub(crate) fn try_extended_call_expr(
        &mut self,
        left: NodeIndex,
        opt: ExprOption,
    ) -> ParseResult {
        if opt.no_extended_call {
            return Err(ParseError::MeetPostExtendedCallStart);
        }

        self.scoped_with_expected_prefix(&[TokenKind::LBrace], |p| {
            // 解析对象调用参数
            let children_and_properties = p.try_multi_with_bracket(
                &[
                    Rule::comma("optional arg", |p| p.try_property()),
                    Rule::comma("expanded argument", |p| p.try_extend_arg()),
                    Rule::comma("function argument", |p| p.try_expr()),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            // 创建扩展调用节点
            Ok(
                NodeBuilder::new(NodeKind::ExtendedApplication, p.current_span())
                    .add_single_child(left)
                    .add_multiple_children(children_and_properties)
                    .build(&mut p.ast),
            )
        })
    }

    /// 索引调用表达式
    fn try_index_call_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LBracket], |p| {
            let open_span = p.next_token_span();
            p.eat_tokens(1); // consume the opening bracket, not a list literal
            // 解析索引调用参数
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected an index expression after `[`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(open_span, "this `[` was opened here"));
            }

            if !p.eat_token(TokenKind::RBracket) {
                return Err(ParseError::unexpected_token(
                    TokenKind::RBracket,
                    p.peek_next_token().kind,
                    p.next_token_span(),
                )
                .with_secondary_label(open_span, "this `[` was opened here"));
            }
            // 创建索引调用节点
            Ok(
                NodeBuilder::new(NodeKind::IndexApplication, p.current_span())
                    .add_single_child(left)
                    .add_single_child(expr)
                    .build(&mut p.ast),
            )
        })
    }

    /// 点操作符表达式
    fn try_dot_expr(&mut self, left: NodeIndex) -> ParseResult {
        if !self.eat_token(TokenKind::Dot) {
            return Ok(0);
        }
        let next = self.peek_next_token();
        return match next.kind {
            TokenKind::Star => self.try_deref_expr(left),
            TokenKind::Use => self.try_handler_apply_expr(left),
            TokenKind::Ref => self.try_refer_expr(left),
            TokenKind::Await => self.try_await_expr(left),
            TokenKind::Dyn => self.try_as_dyn_expr(left),
            TokenKind::As => self.try_type_cast_expr(left),
            TokenKind::Dot => self.parse_range_expr(left),
            _ => self.try_select_expr(left),
        };
    }

    /// 解引用表达式
    fn try_deref_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.eat_tokens(1); // 消耗解引用操作符

        // 创建解引用节点
        Ok(NodeBuilder::new(NodeKind::Deref, self.current_span())
            .add_single_child(left)
            .build(&mut self.ast))
    }

    fn try_handler_apply_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Use], |p| {
            p.eat_tokens(1); // 消耗处理器应用操作符

            if !p.eat_token(TokenKind::LParen) {
                return Err(ParseError::invalid_syntax(
                    "Expected '(' after 'use' in handler apply expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let handler_expr = p.try_expr()?;
            if handler_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected expression after '(' in handler apply expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if !p.eat_token(TokenKind::RParen) {
                return Err(ParseError::invalid_syntax(
                    "Expected ')' after expression in handler apply expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 创建处理器应用节点
            Ok(
                NodeBuilder::new(NodeKind::HandlerApplication, p.current_span())
                    .add_single_child(left)
                    .add_single_child(handler_expr)
                    .build(&mut p.ast),
            )
        })
    }

    /// 引用表达式
    fn try_refer_expr(&mut self, left: NodeIndex) -> ParseResult {
        if !self.eat_token(TokenKind::Ref) {
            return Err(ParseError::invalid_syntax(
                "Expected 'ref' after '.' in refer expression".to_string(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }

        Ok(NodeBuilder::new(NodeKind::Refer, self.current_span())
            .add_single_child(left)
            .build(&mut self.ast))
    }

    /// 等待表达式
    fn try_await_expr(&mut self, left: NodeIndex) -> ParseResult {
        if !self.eat_token(TokenKind::Await) {
            return Err(ParseError::invalid_syntax(
                "Expected 'await' after '.' in await expression".to_string(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }

        Ok(NodeBuilder::new(NodeKind::Await, self.current_span())
            .add_single_child(left)
            .build(&mut self.ast))
    }

    /// 字段选择表达式
    fn try_select_expr(&mut self, left: NodeIndex) -> ParseResult {
        let id = self.try_member_id()?;
        if id == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected identifier after '.' in select expression".to_string(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }

        Ok(NodeBuilder::new(NodeKind::Projection, self.current_span())
            .add_single_child(left)
            .add_single_child(id)
            .build(&mut self.ast))
    }

    /// take_view -> expr ' id
    fn try_take_view_expr(&mut self, left: NodeIndex) -> ParseResult {
        if !self.eat_token(TokenKind::Quote) {
            return Ok(0);
        }

        let id = self.try_id()?;
        if id == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected identifier after `'` in take-view expression".to_string(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }
        Ok(NodeBuilder::new(NodeKind::TakeView, self.current_span())
            .add_single_child(left)
            .add_single_child(id)
            .build(&mut self.ast))
    }

    /// 动态转换表达式
    fn try_as_dyn_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Dyn], |p| {
            p.eat_tokens(1);

            if !p.eat_token(TokenKind::LParen) {
                return Err(ParseError::invalid_syntax(
                    "Expected '(' after 'dyn' in as dyn expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let dyn_expr = p.try_expr()?;
            if dyn_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected expression after '(' in as dyn expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if !p.eat_token(TokenKind::RParen) {
                return Err(ParseError::invalid_syntax(
                    "Expected ')' after expression in as dyn expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 创建动态转换节点
            Ok(NodeBuilder::new(NodeKind::DynCast, p.current_span())
                .add_single_child(left)
                .add_single_child(dyn_expr)
                .build(&mut p.ast))
        })
    }

    /// 类型转换表达式
    fn try_type_cast_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::As], |p| {
            p.eat_tokens(1); // 消耗 'as'

            if !p.eat_token(TokenKind::LParen) {
                return Err(ParseError::invalid_syntax(
                    "Expected '(' after 'as' in type cast expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let type_expr = p.try_expr()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected expression after '(' in type cast expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            if !p.eat_token(TokenKind::RParen) {
                return Err(ParseError::invalid_syntax(
                    "Expected ')' after expression in type cast expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 创建类型转换节点
            Ok(NodeBuilder::new(NodeKind::TypeCast, p.current_span())
                .add_single_child(left)
                .add_single_child(type_expr)
                .build(&mut p.ast))
        })
    }

    /// 区间表达式
    fn parse_range_expr(&mut self, left: NodeIndex) -> ParseResult {
        if self.peek(&[TokenKind::Dot, TokenKind::Eq]) {
            self.eat_tokens(2);
            let end = self.try_expr_without_extended_call()?;
            if end == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected expression after '..=' in range expression".to_string(),
                    self.peek_next_token().kind,
                    self.next_token_span(),
                ));
            }
            Ok(
                NodeBuilder::new(NodeKind::RangeFromToInclusive, self.current_span())
                    .add_single_child(left)
                    .add_single_child(end)
                    .build(&mut self.ast),
            )
        } else {
            self.eat_tokens(1);
            let end = self.try_expr_without_extended_call()?;
            if end != 0 {
                Ok(NodeBuilder::new(NodeKind::RangeFromTo, self.current_span())
                    .add_single_child(left)
                    .add_single_child(end)
                    .build(&mut self.ast))
            } else {
                Ok(NodeBuilder::new(NodeKind::RangeFrom, self.current_span())
                    .add_single_child(left)
                    .build(&mut self.ast))
            }
        }
    }

    /// 效果处理表达式
    fn try_effect_handling_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Hash], |p| {
            p.eat_tokens(1); // 消耗 '#'

            if !p.eat_token(TokenKind::LBrace) {
                return Ok(
                    NodeBuilder::new(NodeKind::EffectPropagation, p.current_span())
                        .add_single_child(left)
                        .build(&mut p.ast),
                );
            }

            let arms =
                p.try_multi(&[Rule::comma("effect handling arm", |p| p.try_effect_arm())])?;

            if !p.eat_token(TokenKind::RBrace) {
                return Err(ParseError::invalid_syntax(
                    "Expected '}' after effect handling arms".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(
                NodeBuilder::new(NodeKind::EffectElimination, p.current_span())
                    .add_single_child(left)
                    .add_multiple_children(arms)
                    .build(&mut p.ast),
            )
        })
    }

    /// 错误表达式
    fn try_error_handling_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Bang], |p| {
            p.eat_tokens(1); // 消耗 '!'

            if !p.eat_token(TokenKind::LBrace) {
                return Ok(
                    NodeBuilder::new(NodeKind::ErrorPropagation, p.current_span())
                        .add_single_child(left)
                        .build(&mut p.ast),
                );
            }

            let arms = p.try_multi(&[
                Rule::comma("catching arm", |p| p.try_catch_arm()),
                Rule::comma("error handling arm", |p| p.try_case_arm()),
            ])?;

            if !p.eat_token(TokenKind::RBrace) {
                return Err(ParseError::invalid_syntax(
                    "Expected '}' after error handling arms".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(
                NodeBuilder::new(NodeKind::ErrorElimination, p.current_span())
                    .add_single_child(left)
                    .add_multiple_children(arms)
                    .build(&mut p.ast),
            )
        })
    }

    /// 选项表达式
    fn try_option_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Question], |p| {
            p.eat_tokens(1); // 消耗 '?'

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Ok(
                    NodeBuilder::new(NodeKind::OptionPropagation, p.current_span())
                        .add_single_child(left)
                        .build(&mut p.ast),
                );
            }

            let handling_block = p.try_block()?;
            if handling_block == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected block after '?' in option expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(
                NodeBuilder::new(NodeKind::OptionElimination, p.current_span())
                    .add_single_child(left)
                    .add_single_child(handling_block)
                    .build(&mut p.ast),
            )
        })
    }

    /// 后置匹配表达式
    fn try_post_match_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Match.as_ref(), |p| {
            p.eat_tokens(1);

            if !p.peek(TokenKind::LBrace.as_ref()) {
                return Err(ParseError::invalid_syntax(
                    "Expected '{' after 'match' in post match expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let arms = p.try_multi_with_bracket(
                &[Rule::comma("match arm or `use` import", |p| {
                    if p.peek(TokenKind::Use.as_ref()) {
                        p.try_arm_import()
                    } else {
                        p.try_case_arm()
                    }
                })],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            Ok(NodeBuilder::new(NodeKind::PostMatch, p.current_span())
                .add_single_child(left)
                .add_multiple_children(arms)
                .build(&mut p.ast))
        })
    }

    fn try_arm_import(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Use.as_ref(), |p| {
            p.eat_tokens(1);
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "expected template expression after `use` in `match`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }
            Ok(NodeBuilder::new(NodeKind::ArmImport, p.current_span())
                .add_single_child(expr)
                .build(&mut p.ast))
        })
    }

    /// 匹配表达式
    fn try_matches_expr(&mut self, left: NodeIndex, option: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Matches.as_ref(), |p| {
            p.eat_tokens(1);

            let pattern = if option.no_extended_call {
                p.try_pattern_without_extended_call()?
            } else {
                p.try_pattern()?
            };
            if pattern == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected pattern after 'matches' in matches expression".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::BoolMatches, p.current_span())
                .add_single_child(left)
                .add_single_child(pattern)
                .build(&mut p.ast))
        })
    }

    /// post_lambda -> expr do (lambda | block | expr)
    fn try_post_lambda_expr(&mut self, left: NodeIndex, opt: ExprOption) -> ParseResult {
        self.scoped_with_expected_prefix(TokenKind::Do.as_ref(), |p| {
            p.eat_tokens(1); // consume 'do'

            // Try lambda first (|params| body)
            let body = if p.peek(TokenKind::Pipe.as_ref()) {
                p.try_lambda(opt)?
            } else if p.peek(TokenKind::LBrace.as_ref()) && !p.is_token_buffer_start() {
                // Try block
                p.try_block()?
            } else {
                // Otherwise parse as expression
                p.try_expr_with_option(opt)?
            };

            if body == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected lambda, block, or expression after `do`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(NodeKind::PostLambda, p.current_span())
                .add_single_child(left)
                .add_single_child(body)
                .build(&mut p.ast))
        })
    }

    /// Literal extension expression (e.g., `123px`, `45deg`)
    pub(crate) fn try_literal_extension_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Id], |p| {
            match p.ast.get_node_kind(left) {
                Some(
                    NodeKind::Int
                    | NodeKind::Real
                    | NodeKind::Str
                    | NodeKind::StringConcat
                    | NodeKind::Char,
                ) => {}
                Some(NodeKind::LiteralExtension) => {
                    return Err(ParseError::invalid_syntax(
                        "A literal accepts only one suffix identifier".to_string(),
                        p.peek_next_token().kind,
                        p.next_token_span(),
                    ));
                }
                _ => return Err(ParseError::MeetPostId),
            }
            let id = p.try_id()?;
            if id == 0 {
                return Ok(0);
            }

            let span = rustc_span::Span::new(
                p.ast.get_span(left).unwrap().lo(),
                p.ast.get_span(id).unwrap().hi(),
            );
            Ok(NodeBuilder::new(NodeKind::LiteralExtension, span)
                .add_single_child(left)
                .add_single_child(id)
                .build(&mut p.ast))
        })
    }
}
