use ast::NodeKind;
use lex::TokenKind;

/// 操作符信息，包含优先级和对应的AST标签
#[derive(Debug, Clone, Copy)]
pub struct ExprOpInfo {
    pub prec: i32,
    pub node_kind: NodeKind,
}

impl ExprOpInfo {
    pub fn new(prec: i32, node_kind: NodeKind) -> Self {
        Self { prec, node_kind }
    }
}

pub fn get_expr_op_info(token_kind: TokenKind) -> ExprOpInfo {
    match token_kind {
        // 布尔逻辑操作符
        TokenKind::EqEqEq => ExprOpInfo::new(10, NodeKind::BoolImplies), // ==>
        TokenKind::Or => ExprOpInfo::new(20, NodeKind::BoolOr),          // or
        TokenKind::And => ExprOpInfo::new(30, NodeKind::BoolAnd),        // and

        // 比较操作符
        TokenKind::BangEq => ExprOpInfo::new(40, NodeKind::BoolNotEq), // !=
        TokenKind::EqEq => ExprOpInfo::new(40, NodeKind::BoolEq),      // ==
        TokenKind::GtEq => ExprOpInfo::new(40, NodeKind::BoolGtEq),    // >=
        TokenKind::SeparatedGt => ExprOpInfo::new(40, NodeKind::BoolGt), // " > "
        TokenKind::LtEq => ExprOpInfo::new(40, NodeKind::BoolLtEq),    // <=
        TokenKind::SeparatedLt => ExprOpInfo::new(40, NodeKind::BoolLt), // " < "

        // type-related operators
        TokenKind::Colon => ExprOpInfo::new(40, NodeKind::TypedWith), // :
        TokenKind::ColonMinus => ExprOpInfo::new(40, NodeKind::TraitBound), // :-
        TokenKind::Matches => ExprOpInfo::new(40, NodeKind::BoolMatches), // matches

        // 箭头和管道
        TokenKind::Arrow => ExprOpInfo::new(50, NodeKind::Arrow), // ->

        // 算术操作符
        TokenKind::SeparatedPlus => ExprOpInfo::new(60, NodeKind::Add), // " + "
        TokenKind::SeparatedMinus => ExprOpInfo::new(60, NodeKind::Sub), // " - "
        TokenKind::SeparatedSlash => ExprOpInfo::new(70, NodeKind::Div), // " / "
        TokenKind::SeparatedStar => ExprOpInfo::new(70, NodeKind::Mul), // " * "
        TokenKind::SeparatedPercent => ExprOpInfo::new(70, NodeKind::Mod), // " % "
        TokenKind::PlusPlus => ExprOpInfo::new(70, NodeKind::AddAdd),   // ++

        // 管道操作符
        TokenKind::Pipe => ExprOpInfo::new(80, NodeKind::Pipe), // |
        TokenKind::PipeGt => ExprOpInfo::new(80, NodeKind::PipePrepend), // |>

        // 90级保留用于前缀表达式优先级

        // 调用操作符 (100级)
        TokenKind::LParen => ExprOpInfo::new(100, NodeKind::Application), // (
        TokenKind::LBracket => ExprOpInfo::new(100, NodeKind::IndexApplication), // [
        TokenKind::LBrace => ExprOpInfo::new(100, NodeKind::ExtendedApplication), // {
        TokenKind::Lt => ExprOpInfo::new(100, NodeKind::NormalFormApplication), // <
        TokenKind::Hash => ExprOpInfo::new(100, NodeKind::EffectElimination), // #
        TokenKind::Bang => ExprOpInfo::new(100, NodeKind::ErrorElimination), // !
        TokenKind::Question => ExprOpInfo::new(100, NodeKind::OptionElimination), // ?
        TokenKind::Match => ExprOpInfo::new(100, NodeKind::PostMatch), // match
        TokenKind::Do => ExprOpInfo::new(100, NodeKind::PostLambda),   // do
        TokenKind::Matches => ExprOpInfo::new(100, NodeKind::BoolMatches), // matches

        // projection and take_view (110)
        TokenKind::Dot => ExprOpInfo::new(110, NodeKind::Select), // .
        TokenKind::Quote => ExprOpInfo::new(110, NodeKind::TakeView), // '

        // 标识符 (120级)
        TokenKind::Id => ExprOpInfo::new(120, NodeKind::Id), // id

        _ => ExprOpInfo::new(-1, NodeKind::Invalid), // 默认无效操作符
    }
}

/// Get pattern operator info for pratt parsing
pub fn get_pattern_op_info(token_kind: TokenKind) -> ExprOpInfo {
    match token_kind {
        // pattern postfix operators (low precedence)
        TokenKind::If => ExprOpInfo::new(10, NodeKind::IfGuardPattern), // if guard
        TokenKind::And => ExprOpInfo::new(10, NodeKind::AndIsPattern),  // and ... is ...
        TokenKind::As => ExprOpInfo::new(20, NodeKind::AsBindPattern),  // as bind
        TokenKind::Or => ExprOpInfo::new(30, NodeKind::OrPattern),      // or

        // propagation postfix (high precedence)
        TokenKind::Question => ExprOpInfo::new(40, NodeKind::OptionSomePattern), // ?
        TokenKind::Bang => ExprOpInfo::new(40, NodeKind::ErrorOkPattern), // !

        // application postfix
        TokenKind::LParen => ExprOpInfo::new(80, NodeKind::ApplicationPattern), // (
        TokenKind::LBrace => ExprOpInfo::new(80, NodeKind::ExtendedApplicationPattern), // {
        TokenKind::Lt => ExprOpInfo::new(80, NodeKind::NormalFormApplicationPattern), // <

        // projection
        TokenKind::Dot => ExprOpInfo::new(90, NodeKind::Select), // .

        _ => ExprOpInfo::new(-1, NodeKind::Invalid),
    }
}
