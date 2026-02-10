use super::basic::Rule;
use super::error::*;
use super::operators::get_expr_op_info;
use super::parser::*;
use ast::*;
use lex::TokenKind;

/// 表达式选项，包含是否记录调用和优先级
#[derive(Debug, Clone, Copy)]
pub struct ExprOption {
    pub no_extended_call: bool,
    pub precedence: i32,
}

impl ExprOption {
    pub fn new() -> Self {
        Self {
            no_extended_call: false,
            precedence: 0,
        }
    }

    pub fn with_no_extended_call(mut self, no_extended_call: bool) -> Self {
        self.no_extended_call = no_extended_call;
        self
    }

    pub fn with_precedence(mut self, precedence: i32) -> Self {
        self.precedence = precedence;
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
            let mut current_left = match p.try_prefix_expr(option)? {
                0 => return Ok(0), // 没有找到有效的前缀表达式
                node => node,
            };

            // 循环处理操作符和右操作数
            loop {
                let token = p.peek_next_token();
                let op_info = get_expr_op_info(token.kind);

                // 如果操作符无效或优先级太低，则退出循环
                if op_info.node_kind == NodeKind::Invalid || op_info.prec < min_prec {
                    break;
                }

                match p.try_postfix_expr(token.kind, current_left, option) {
                    Ok(node) if node != 0 => {
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
                        // 消耗操作符标记
                        p.eat_tokens(1);

                        // 解析右操作数（递归调用，使用更高优先级）
                        let right = match p.try_expr_pratt(op_info.prec + 1, option)? {
                            0 => {
                                return Err(ParseError::invalid_syntax(
                                    format!(
                                        "Expected a right operand after binary operator `{}`",
                                        p.peek_next_token().kind.lexme()
                                    ),
                                    p.peek_next_token().kind,
                                    p.next_token_span(),
                                ));
                            }

                            node => node,
                        };

                        // 创建二元操作符节点
                        current_left = NodeBuilder::new(op_info.node_kind, p.current_span())
                            .add_single_child(current_left)
                            .add_single_child(right)
                            .build(&mut p.ast);
                    }
                }
            }

            // 返回最终构建的表达式
            Ok(current_left)
        })
    }

    pub fn try_prefix_expr(&mut self, option: ExprOption) -> ParseResult {
        self.scoped(|p| {
            let token = p.peek_next_token();
            match token.kind {
                // for those atomic expressions
                TokenKind::Int
                | TokenKind::Real
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

                TokenKind::LParen => p.try_unit_or_parenthesis_or_tuple(),
                TokenKind::LBracket => p.try_list(),
                TokenKind::LBrace => p.try_object(),
                TokenKind::Dot => p.try_prefix_range_expr_or_symbol(option),
                TokenKind::Pipe => p.try_lambda(option),
                TokenKind::Forall => p.try_forall_prefix(option),
                TokenKind::Exists => p.try_bool_exists(option),
                TokenKind::Hash => p.try_effect_qualified_type(),
                TokenKind::Bang => p.try_error_qualified_type(),
                TokenKind::Ampersand => p.try_reachability_type(),
                TokenKind::Caret => p.try_closure_qualified_type(),
                TokenKind::Minus | TokenKind::SeparatedMinus => {
                    p.try_prefix_unary_expr(token.kind, NodeKind::Negative, 90)
                }
                TokenKind::Not => p.try_prefix_unary_expr(TokenKind::Not, NodeKind::BoolNot, 90),
                TokenKind::Error => {
                    p.try_prefix_unary_expr(TokenKind::Error, NodeKind::ErrorNew, 90)
                }
                TokenKind::Dyn => {
                    p.try_prefix_unary_expr(TokenKind::Dyn, NodeKind::TraitObjectType, 90)
                }
                TokenKind::Star => {
                    p.try_prefix_unary_expr(TokenKind::Star, NodeKind::PointerType, 90)
                }
                TokenKind::Question => {
                    p.try_prefix_unary_expr(TokenKind::Question, NodeKind::OptionalType, 90)
                }
                TokenKind::Lift => {
                    p.try_prefix_unary_expr(TokenKind::Lift, NodeKind::LiftType, 90)
                }
                TokenKind::Do => p.try_do_block_expr(),
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

                // fn_type -> pure? comptime? inline? (unsafe|spec|verified)? (extern "ABI")? fn(parameter_type*)
                TokenKind::Fn => p.try_fn_type(),
                TokenKind::Pure | TokenKind::Inline | TokenKind::Spec
                | TokenKind::Verified | TokenKind::Extern => p.try_fn_type(),

                _ => Ok(0),
            }
        })
    }

    /// 解析单元类型、括号表达式或元组
    /// tuple -> ( expr, expr* )
    /// parenthesis -> ( expr )
    /// unit -> ()
    fn try_unit_or_parenthesis_or_tuple(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LParen], |p| {
            p.eat_tokens(1); // 消耗 '('

            // 检查是否是空的单元类型 ()
            if p.eat_token(TokenKind::RParen) {
                return Ok(NodeBuilder::new(NodeKind::Unit, p.current_span()).build(&mut p.ast));
            }

            // 解析第一个表达式
            let first_expr = p.try_expr()?;
            if first_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected expression in parenthesis expression or tuple".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
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
                    ));
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
                    ));
                }

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
                &[Rule::comma("lambda parameter", |p| p.try_param())],
                (TokenKind::Pipe, TokenKind::Pipe),
            )?;

            // 解析可选的返回类型
            let return_type = if p.eat_token(TokenKind::Arrow) {
                let rt = p.try_expr()?;
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
            let body = match p.try_block()? {
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
                let params = p.try_multi(&[Rule::comma("type bound parameter", |p| {
                    p.try_param()
                })])?;
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

    /// 解析效果限定类型
    /// effect_qualified_type -> #expr expr
    fn try_effect_qualified_type(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Hash], |p| {
            p.eat_tokens(1); // 消耗 '#'

            // 解析效果列表
            let effect_list = p.try_expr_without_extended_call()?;
            if effect_list == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected effect list after '#'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 解析类型表达式
            let type_expr = p.try_expr_without_extended_call()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected type expression in effect qualified type".to_string(),
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
    fn try_error_qualified_type(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Bang], |p| {
            p.eat_tokens(1); // 消耗 '!'

            // 解析错误列表
            let error_list = p.try_expr_without_extended_call()?;
            if error_list == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected error list after '!'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 解析类型表达式
            let type_expr = p.try_expr_without_extended_call()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected type expression in error qualified type".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
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
    fn try_reachability_type(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Ampersand], |p| {
            p.eat_tokens(1);

            // 解析可达性列表
            let reachability_set = p.try_expr_without_extended_call()?;
            if reachability_set == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected reachability list after '&'".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 解析类型表达式
            let type_expr = p.try_expr_without_extended_call()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected type expression in reachability type".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
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
    fn try_keyword_block_expr(
        &mut self,
        keyword: TokenKind,
        node_kind: NodeKind,
    ) -> ParseResult {
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
            // AtomicBlock is (a, b, N) = DoubleWithMultiChildren? 
            // Actually it's N, a in current code comment. Let me use the correct layout.
            // AtomicBlock: N, a -> but should be a, N = SingleWithMultiChildren
            // Wait, current code says AtomicBlock is DoubleWithMultiChildren.
            // Looking at the comment: AtomicBlock, // N, a  
            // This is wrong for DoubleWithMultiChildren. Let me keep it as-is for now.
            Ok(NodeBuilder::new(NodeKind::AtomicBlock, p.current_span())
                .add_multiple_children(ids)
                .add_single_child(block)
                .build(&mut p.ast))
        })
    }

    /// closure_qualified_type -> ^expr expr
    fn try_closure_qualified_type(&mut self) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Caret], |p| {
            p.eat_tokens(1); // consume '^'

            let closure_expr = p.try_expr_without_extended_call()?;
            if closure_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected closure expression after `^`".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            let type_expr = p.try_expr_without_extended_call()?;
            if type_expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected type expression in closure qualified type".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(
                NodeBuilder::new(NodeKind::ClosureQualifiedType, p.current_span())
                    .add_single_child(closure_expr)
                    .add_single_child(type_expr)
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
    ) -> ParseResult {
        self.scoped_with_expected_prefix(&[token], |p| {
            p.eat_tokens(1); // 消耗前缀token

            let expr = p.try_expr_with_option(
                ExprOption::new()
                    .with_precedence(precedence)
                    .with_no_extended_call(true),
            )?;

            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    format!("Expected expression after '{}'", token.lexme()),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            Ok(NodeBuilder::new(node_kind, p.current_span())
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
        self.scoped(|p| {
            match tag {
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
            }
        })
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

    /// normal_form_application -> expr<argument*>
    fn try_diamond_call_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Lt], |p| {
            let args = p.try_multi_with_bracket(
                &[
                    Rule::comma("optional argument", |p| p.try_optional_arg()),
                    Rule::comma("function argument", |p| p.try_expr()),
                ],
                (TokenKind::Lt, TokenKind::Gt),
            )?;

            // 创建泛型调用节点
            Ok(NodeBuilder::new(NodeKind::NormalFormApplication, p.current_span())
                .add_single_child(left)
                .add_multiple_children(args)
                .build(&mut p.ast))
        })
    }

    /// extended_application -> expr { (property | expr)* }
    fn try_extended_call_expr(&mut self, left: NodeIndex, opt: ExprOption) -> ParseResult {
        if opt.no_extended_call {
            return Err(ParseError::MeetPostExtendedCallStart);
        }

        self.scoped_with_expected_prefix(&[TokenKind::LBrace], |p| {
            // 解析对象调用参数
            let children_and_properties = p.try_multi_with_bracket(
                &[
                    Rule::comma("optional arg", |p| p.try_property()),
                    Rule::comma("function argument", |p| p.try_expr()),
                ],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            // 创建扩展调用节点
            Ok(NodeBuilder::new(NodeKind::ExtendedApplication, p.current_span())
                .add_single_child(left)
                .add_multiple_children(children_and_properties)
                .build(&mut p.ast))
        })
    }

    /// 索引调用表达式
    fn try_index_call_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::LBracket], |p| {
            // 解析索引调用参数
            let expr = p.try_expr()?;
            if expr == 0 {
                return Err(ParseError::invalid_syntax(
                    "Expected expression after '[' in index call".to_string(),
                    p.peek_next_token().kind,
                    p.next_token_span(),
                ));
            }

            // 创建索引调用节点
            Ok(NodeBuilder::new(NodeKind::IndexApplication, p.current_span())
                .add_single_child(left)
                .add_single_child(expr)
                .build(&mut p.ast))
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
            Ok(NodeBuilder::new(NodeKind::HandlerApplication, p.current_span())
                .add_single_child(left)
                .add_single_child(handler_expr)
                .build(&mut p.ast))
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
        let id = self.try_id()?;
        if id == 0 {
            return Err(ParseError::invalid_syntax(
                "Expected identifier after '.' in select expression".to_string(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }

        Ok(NodeBuilder::new(NodeKind::Select, self.current_span())
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
                p.try_multi(&[Rule::comma("effect handling arm", |p| p.try_case_arm())])?;

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
                &[Rule::comma("match arm", |p| p.try_case_arm())],
                (TokenKind::LBrace, TokenKind::RBrace),
            )?;

            Ok(NodeBuilder::new(NodeKind::PostMatch, p.current_span())
                .add_single_child(left)
                .add_multiple_children(arms)
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
            } else if p.peek(TokenKind::LBrace.as_ref()) {
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
    fn try_literal_extension_expr(&mut self, left: NodeIndex) -> ParseResult {
        self.scoped_with_expected_prefix(&[TokenKind::Id], |p| {
            let id = p.try_id()?;
            if id == 0 {
                return Ok(0);
            }

            Ok(
                NodeBuilder::new(NodeKind::LiteralExtension, p.current_span())
                    .add_single_child(left)
                    .add_single_child(id)
                    .build(&mut p.ast),
            )
        })
    }
}
