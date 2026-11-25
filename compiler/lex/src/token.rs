use std::collections::HashMap;

pub type Index = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // operators
    Plus,             // +
    PlusEq,           // +=
    PlusPlus,         // ++
    SeparatedPlus,    // " + "
    Lt,               // <
    LtEq,             // <=
    SeparatedLt,      // " < "
    Gt,               // >
    GtEq,             // >=
    SeparatedGt,      // " > "
    Bang,             // !
    BangEq,           // !=
    Minus,            // -
    Arrow,            // ->
    MinusEq,          // -=
    SeparatedMinus,   // " - "
    Dot,              // .
    Colon,            // :
    ColonColon,       // ::
    ColonTilde,       // :~
    ColonMinus,       // :-
    Star,             // *
    StarEq,           // *=
    SeparatedStar,    // " * "
    Slash,            // /
    SlashEq,          // /=
    SeparatedSlash,   // " / "
    Percent,          // %
    PercentEq,        // %=
    SeparatedPercent, // " % "
    Eq,               // =
    FatArrow,         // =>
    EqEq,             // ==
    EqEqEq,           // ==>
    Tilde,            // ~
    TildeGt,          // ~>
    Pipe,             // |
    PipeGt,           // |>
    Hash,             // #
    Question,         // ?
    Backslash,        // \
    Ampersand,        // &
    LBracket,         // [
    RBracket,         // ]
    LParen,           // (
    RParen,           // )
    LBrace,           // {
    RBrace,           // }
    Comma,            // ,
    Quote,            // '
    Semi,             // ;
    Caret,            // ^
    Dollar,           // $
    At,               // @
    Underscore,       // _

    // primitive literals
    Str, // "..."
    Int, // 123
    BitBinPrefix,
    BitOctPrefix,
    BitHexPrefix,
    IntBin,  // 0b1010
    IntOct,  // 0o777
    IntHex,  // 0xFF
    Real,    // 123.45
    RealSci, // 1.23e-4 (scientific notation)
    Char,    // 'a' or '\n' or '\x{1F600}'

    // keywords
    And,       // and
    As,        // as
    Assoc,     // assoc
    Asserts,   // asserts
    Assumes,   // assumes
    Async,     // async
    Atomic,    // atomic
    Axiom,     // axiom
    Await,     // await
    Break,     // break
    Case,      // case
    Catch,     // catch
    Comptime,  // comptime
    Defer,     // defer
    Const,     // const
    Continue,  // continue
    Decreases, // decreases
    Define,    // define
    Derive,    // derive
    Do,        // do
    Dyn,       // dyn
    Effect,    // effect
    Else,      // else
    Ensures,   // ensures
    Enum,      // enum
    Error,     // error
    Exists,    // exists
    Extend,    // extend
    False,     // false
    Fn,        // fn
    FnCap,     // Fn
    For,       // for
    Forall,    // forall
    Ghost,     // ghost
    Handles,   // handles
    If,        // if
    Implicit,  // implicit
    Impl,      // impl
    In,        // in
    Inline,    // inline
    Invariant, // invariant
    Is,        // is
    Itself,    // itself
    Lambda,    // lambda
    Lemma,     // lemma
    Var,
    Let,       // let
    Lift,      // lift
    Match,     // match
    Matches,   // matches
    Move,      // move
    Mod,       // mod
    Newtype,   // newtype
    Not,       // not
    Null,      // null
    Opaque,    // opaque
    Opens,     // opens
    Or,        // or
    Outcomes,  // outcome
    Predicate, // predicate
    Private,   // private
    Pub,       // pub
    Pure,      // pure
    KwQuote,   // quote
    Ref,       // ref
    Refines,   // refines
    Requires,  // requires
    Resume,    // resume
    Return,    // return
    SelfLower, // self
    SelfCap,   // Self
    Spec,      // spec
    Static,    // static
    Struct,    // struct
    Template,  // template
    Test,      // test
    Trait,     // trait
    True,      // true
    Typealias, // typealias
    Union,     // union
    Unsafe,    // unsafe
    Undefined, // undefined
    Use,       // use
    Verified,  // verified
    When,      // when
    While,     // while
    Where,     // where

    // others
    Id,           // identifier
    MacroContent, // macro content, 当lexer碰到`'`字符后, 向后看到第一个非空字符为 `{` 的时候,
    // 认为是宏内容, 开始识别macro content, 并按栈式处理遇到的 `{` 和 `}`, 当最后一个 `}` 闭合栈时,
    // 结束macro content, 这时的 `from` 是 `'` 的位置, `to` 是最后一个 `}` 的位置
    Comment, // -- comment or {- comment -}
    Invalid, // invalid token
    Sof,     // start of file
    Eof,     // end of file
}

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: TokenKind,
    pub from: Index,
    pub to: Index,
}

impl AsRef<[TokenKind]> for TokenKind {
    fn as_ref(&self) -> &[TokenKind] {
        std::slice::from_ref(self)
    }
}

impl TokenKind {
    pub fn lexme(self) -> &'static str {
        match self {
            TokenKind::Plus => "+",
            TokenKind::PlusEq => "+=",
            TokenKind::PlusPlus => "++",
            TokenKind::SeparatedPlus => " + ",
            TokenKind::Lt => "<",
            TokenKind::LtEq => "<=",
            TokenKind::SeparatedLt => " < ",
            TokenKind::Gt => ">",
            TokenKind::GtEq => ">=",
            TokenKind::SeparatedGt => " > ",
            TokenKind::Bang => "!",
            TokenKind::BangEq => "!=",
            TokenKind::Minus => "-",
            TokenKind::Arrow => "->",
            TokenKind::MinusEq => "-=",
            TokenKind::SeparatedMinus => " - ",
            TokenKind::Dot => ".",
            TokenKind::Colon => ":",
            TokenKind::ColonColon => "::",
            TokenKind::ColonTilde => ":~",
            TokenKind::ColonMinus => ":-",
            TokenKind::Star => "*",
            TokenKind::StarEq => "*=",
            TokenKind::SeparatedStar => " * ",
            TokenKind::Slash => "/",
            TokenKind::SlashEq => "/=",
            TokenKind::SeparatedSlash => " / ",
            TokenKind::Percent => "%",
            TokenKind::PercentEq => "%=",
            TokenKind::SeparatedPercent => " % ",
            TokenKind::Eq => "=",
            TokenKind::FatArrow => "=>",
            TokenKind::EqEq => "==",
            TokenKind::EqEqEq => "==>",
            TokenKind::Tilde => "~",
            TokenKind::TildeGt => "~>",
            TokenKind::Pipe => "|",
            TokenKind::PipeGt => "|>",
            TokenKind::Hash => "#",
            TokenKind::Question => "?",
            TokenKind::Backslash => "\\",
            TokenKind::Ampersand => "&",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::Comma => ",",
            TokenKind::Quote => "'",
            TokenKind::Semi => ";",
            TokenKind::Caret => "^",
            TokenKind::Dollar => "$",
            TokenKind::At => "@",
            TokenKind::Underscore => "_",
            TokenKind::And => "and",
            TokenKind::As => "as",
            TokenKind::Assoc => "assoc",
            TokenKind::Asserts => "asserts",
            TokenKind::Assumes => "assumes",
            TokenKind::Async => "async",
            TokenKind::Atomic => "atomic",
            TokenKind::Axiom => "axiom",
            TokenKind::Await => "await",
            TokenKind::Break => "break",
            TokenKind::Case => "case",
            TokenKind::Catch => "catch",
            TokenKind::Comptime => "comptime",
            TokenKind::Defer => "defer",
            TokenKind::Const => "const",
            TokenKind::Continue => "continue",
            TokenKind::Decreases => "decreases",
            TokenKind::Define => "define",
            TokenKind::Derive => "derive",
            TokenKind::Do => "do",
            TokenKind::Dyn => "dyn",
            TokenKind::Effect => "effect",
            TokenKind::Else => "else",
            TokenKind::Ensures => "ensures",
            TokenKind::Enum => "enum",
            TokenKind::Error => "error",
            TokenKind::Exists => "exists",
            TokenKind::Extend => "extend",
            TokenKind::False => "false",
            TokenKind::Fn => "fn",
            TokenKind::FnCap => "Fn",
            TokenKind::For => "for",
            TokenKind::Forall => "forall",
            TokenKind::Ghost => "ghost",
            TokenKind::Handles => "handles",
            TokenKind::If => "if",
            TokenKind::Implicit => "implicit",
            TokenKind::Impl => "impl",
            TokenKind::In => "in",
            TokenKind::Inline => "inline",
            TokenKind::Invariant => "invariant",
            TokenKind::Is => "is",
            TokenKind::Itself => "itself",
            TokenKind::Lambda => "lambda",
            TokenKind::Lemma => "lemma",
            TokenKind::Let => "let",
            TokenKind::Var => "var",
            TokenKind::Lift => "lift",
            TokenKind::Match => "match",
            TokenKind::Matches => "matches",
            TokenKind::Move => "move",
            TokenKind::Mod => "mod",
            TokenKind::Newtype => "newtype",
            TokenKind::Not => "not",
            TokenKind::Null => "null",
            TokenKind::Opaque => "opaque",
            TokenKind::Opens => "opens",
            TokenKind::Or => "or",
            TokenKind::Outcomes => "outcomes",
            TokenKind::Predicate => "predicate",
            TokenKind::Private => "private",
            TokenKind::Pure => "pure",
            TokenKind::Pub => "pub",
            TokenKind::KwQuote => "quote",
            TokenKind::Ref => "ref",
            TokenKind::Refines => "refines",
            TokenKind::Requires => "requires",
            TokenKind::Resume => "resume",
            TokenKind::Return => "return",
            TokenKind::SelfLower => "self",
            TokenKind::SelfCap => "Self",
            TokenKind::Spec => "spec",
            TokenKind::Static => "static",
            TokenKind::Struct => "struct",
            TokenKind::Template => "template",
            TokenKind::Test => "test",
            TokenKind::Trait => "trait",
            TokenKind::True => "true",
            TokenKind::Typealias => "typealias",
            TokenKind::Union => "union",
            TokenKind::Unsafe => "unsafe",
            TokenKind::Undefined => "undefined",
            TokenKind::Use => "use",
            TokenKind::Verified => "verified",
            TokenKind::When => "when",
            TokenKind::While => "while",
            TokenKind::Where => "where",
            TokenKind::Id => "<identifier>",
            TokenKind::MacroContent => "<macro content>",
            TokenKind::Comment => "<comment>",
            TokenKind::Invalid => "<invalid>",
            TokenKind::Sof => "<start of file>",
            TokenKind::Eof => "<end of file>",
            TokenKind::Str => "<string literal>",
            TokenKind::Int => "<integer literal>",
            TokenKind::BitBinPrefix => "0b",
            TokenKind::BitOctPrefix => "0o",
            TokenKind::BitHexPrefix => "0x",
            TokenKind::IntBin => "<binary integer literal>",
            TokenKind::IntOct => "<octal integer literal>",
            TokenKind::IntHex => "<hexadecimal integer literal>",
            TokenKind::Real => "<real literal>",
            TokenKind::RealSci => "<scientific notation literal>",
            TokenKind::Char => "<character literal>",
        }
    }

    /// Canonical lowercase identifier spelling used as a Flurry `.symbol` in `meta.ast.Token`.
    ///
    /// Every keyword variant is prefixed with `kw_` because `.id` symbol literals require an
    /// identifier token rather than a keyword token.
    pub fn symbol_name(self) -> &'static str {
        match self {
            TokenKind::Plus => "plus",
            TokenKind::PlusEq => "plus_eq",
            TokenKind::PlusPlus => "plus_plus",
            TokenKind::SeparatedPlus => "separated_plus",
            TokenKind::Lt => "lt",
            TokenKind::LtEq => "lt_eq",
            TokenKind::SeparatedLt => "separated_lt",
            TokenKind::Gt => "gt",
            TokenKind::GtEq => "gt_eq",
            TokenKind::SeparatedGt => "separated_gt",
            TokenKind::Bang => "bang",
            TokenKind::BangEq => "bang_eq",
            TokenKind::Minus => "minus",
            TokenKind::Arrow => "arrow",
            TokenKind::MinusEq => "minus_eq",
            TokenKind::SeparatedMinus => "separated_minus",
            TokenKind::Dot => "dot",
            TokenKind::Colon => "colon",
            TokenKind::ColonColon => "colon_colon",
            TokenKind::ColonTilde => "colon_tilde",
            TokenKind::ColonMinus => "colon_minus",
            TokenKind::Star => "star",
            TokenKind::StarEq => "star_eq",
            TokenKind::SeparatedStar => "separated_star",
            TokenKind::Slash => "slash",
            TokenKind::SlashEq => "slash_eq",
            TokenKind::SeparatedSlash => "separated_slash",
            TokenKind::Percent => "percent",
            TokenKind::PercentEq => "percent_eq",
            TokenKind::SeparatedPercent => "separated_percent",
            TokenKind::Eq => "eq",
            TokenKind::FatArrow => "fat_arrow",
            TokenKind::EqEq => "eq_eq",
            TokenKind::EqEqEq => "eq_eq_eq",
            TokenKind::Tilde => "tilde",
            TokenKind::TildeGt => "tilde_gt",
            TokenKind::Pipe => "pipe",
            TokenKind::PipeGt => "pipe_gt",
            TokenKind::Hash => "hash",
            TokenKind::Question => "question",
            TokenKind::Backslash => "backslash",
            TokenKind::Ampersand => "ampersand",
            TokenKind::LBracket => "lbracket",
            TokenKind::RBracket => "rbracket",
            TokenKind::LParen => "lparen",
            TokenKind::RParen => "rparen",
            TokenKind::LBrace => "lbrace",
            TokenKind::RBrace => "rbrace",
            TokenKind::Comma => "comma",
            TokenKind::Quote => "single_quote",
            TokenKind::Semi => "semi",
            TokenKind::Caret => "caret",
            TokenKind::Dollar => "dollar",
            TokenKind::At => "at",
            TokenKind::Underscore => "underscore",

            TokenKind::Str => "str",
            TokenKind::Int => "int",
            TokenKind::BitBinPrefix => "bit_bin_prefix",
            TokenKind::BitOctPrefix => "bit_oct_prefix",
            TokenKind::BitHexPrefix => "bit_hex_prefix",
            TokenKind::IntBin => "int_bin",
            TokenKind::IntOct => "int_oct",
            TokenKind::IntHex => "int_hex",
            TokenKind::Real => "real",
            TokenKind::RealSci => "real_sci",
            TokenKind::Char => "char",

            TokenKind::And => "kw_and",
            TokenKind::As => "kw_as",
            TokenKind::Assoc => "kw_assoc",
            TokenKind::Asserts => "kw_asserts",
            TokenKind::Assumes => "kw_assumes",
            TokenKind::Async => "kw_async",
            TokenKind::Atomic => "kw_atomic",
            TokenKind::Axiom => "kw_axiom",
            TokenKind::Await => "kw_await",
            TokenKind::Break => "kw_break",
            TokenKind::Case => "kw_case",
            TokenKind::Catch => "kw_catch",
            TokenKind::Comptime => "kw_comptime",
            TokenKind::Defer => "kw_defer",
            TokenKind::Const => "kw_const",
            TokenKind::Continue => "kw_continue",
            TokenKind::Decreases => "kw_decreases",
            TokenKind::Define => "kw_define",
            TokenKind::Derive => "kw_derive",
            TokenKind::Do => "kw_do",
            TokenKind::Dyn => "kw_dyn",
            TokenKind::Effect => "kw_effect",
            TokenKind::Else => "kw_else",
            TokenKind::Ensures => "kw_ensures",
            TokenKind::Enum => "kw_enum",
            TokenKind::Error => "kw_error",
            TokenKind::Exists => "kw_exists",
            TokenKind::Extend => "kw_extend",
            TokenKind::False => "kw_false",
            TokenKind::Fn => "kw_fn",
            TokenKind::FnCap => "kw_fn_cap",
            TokenKind::For => "kw_for",
            TokenKind::Forall => "kw_forall",
            TokenKind::Ghost => "kw_ghost",
            TokenKind::Handles => "kw_handles",
            TokenKind::If => "kw_if",
            TokenKind::Implicit => "kw_implicit",
            TokenKind::Impl => "kw_impl",
            TokenKind::In => "kw_in",
            TokenKind::Inline => "kw_inline",
            TokenKind::Invariant => "kw_invariant",
            TokenKind::Is => "kw_is",
            TokenKind::Itself => "kw_itself",
            TokenKind::Lambda => "kw_lambda",
            TokenKind::Lemma => "kw_lemma",
            TokenKind::Var => "kw_var",
            TokenKind::Let => "kw_let",
            TokenKind::Lift => "kw_lift",
            TokenKind::Match => "kw_match",
            TokenKind::Matches => "kw_matches",
            TokenKind::Move => "kw_move",
            TokenKind::Mod => "kw_mod",
            TokenKind::Newtype => "kw_newtype",
            TokenKind::Not => "kw_not",
            TokenKind::Null => "kw_null",
            TokenKind::Opaque => "kw_opaque",
            TokenKind::Opens => "kw_opens",
            TokenKind::Or => "kw_or",
            TokenKind::Outcomes => "kw_outcomes",
            TokenKind::Predicate => "kw_predicate",
            TokenKind::Private => "kw_private",
            TokenKind::Pub => "kw_pub",
            TokenKind::Pure => "kw_pure",
            TokenKind::KwQuote => "kw_quote",
            TokenKind::Ref => "kw_ref",
            TokenKind::Refines => "kw_refines",
            TokenKind::Requires => "kw_requires",
            TokenKind::Resume => "kw_resume",
            TokenKind::Return => "kw_return",
            TokenKind::SelfLower => "kw_self",
            TokenKind::SelfCap => "kw_self_cap",
            TokenKind::Spec => "kw_spec",
            TokenKind::Static => "kw_static",
            TokenKind::Struct => "kw_struct",
            TokenKind::Template => "kw_template",
            TokenKind::Test => "kw_test",
            TokenKind::Trait => "kw_trait",
            TokenKind::True => "kw_true",
            TokenKind::Typealias => "kw_typealias",
            TokenKind::Union => "kw_union",
            TokenKind::Unsafe => "kw_unsafe",
            TokenKind::Undefined => "kw_undefined",
            TokenKind::Use => "kw_use",
            TokenKind::Verified => "kw_verified",
            TokenKind::When => "kw_when",
            TokenKind::While => "kw_while",
            TokenKind::Where => "kw_where",

            TokenKind::Id => "id",
            TokenKind::MacroContent => "macro_content",
            TokenKind::Comment => "comment",
            TokenKind::Invalid => "invalid",
            TokenKind::Sof => "sof",
            TokenKind::Eof => "eof",
        }
    }

    /// Reconstruct a [`TokenKind`] from its [`Self::symbol_name`].
    pub fn from_symbol_name(name: &str) -> Option<Self> {
        Some(match name {
            "plus" => TokenKind::Plus,
            "plus_eq" => TokenKind::PlusEq,
            "plus_plus" => TokenKind::PlusPlus,
            "separated_plus" => TokenKind::SeparatedPlus,
            "lt" => TokenKind::Lt,
            "lt_eq" => TokenKind::LtEq,
            "separated_lt" => TokenKind::SeparatedLt,
            "gt" => TokenKind::Gt,
            "gt_eq" => TokenKind::GtEq,
            "separated_gt" => TokenKind::SeparatedGt,
            "bang" => TokenKind::Bang,
            "bang_eq" => TokenKind::BangEq,
            "minus" => TokenKind::Minus,
            "arrow" => TokenKind::Arrow,
            "minus_eq" => TokenKind::MinusEq,
            "separated_minus" => TokenKind::SeparatedMinus,
            "dot" => TokenKind::Dot,
            "colon" => TokenKind::Colon,
            "colon_colon" => TokenKind::ColonColon,
            "colon_tilde" => TokenKind::ColonTilde,
            "colon_minus" => TokenKind::ColonMinus,
            "star" => TokenKind::Star,
            "star_eq" => TokenKind::StarEq,
            "separated_star" => TokenKind::SeparatedStar,
            "slash" => TokenKind::Slash,
            "slash_eq" => TokenKind::SlashEq,
            "separated_slash" => TokenKind::SeparatedSlash,
            "percent" => TokenKind::Percent,
            "percent_eq" => TokenKind::PercentEq,
            "separated_percent" => TokenKind::SeparatedPercent,
            "eq" => TokenKind::Eq,
            "fat_arrow" => TokenKind::FatArrow,
            "eq_eq" => TokenKind::EqEq,
            "eq_eq_eq" => TokenKind::EqEqEq,
            "tilde" => TokenKind::Tilde,
            "tilde_gt" => TokenKind::TildeGt,
            "pipe" => TokenKind::Pipe,
            "pipe_gt" => TokenKind::PipeGt,
            "hash" => TokenKind::Hash,
            "question" => TokenKind::Question,
            "backslash" => TokenKind::Backslash,
            "ampersand" => TokenKind::Ampersand,
            "lbracket" => TokenKind::LBracket,
            "rbracket" => TokenKind::RBracket,
            "lparen" => TokenKind::LParen,
            "rparen" => TokenKind::RParen,
            "lbrace" => TokenKind::LBrace,
            "rbrace" => TokenKind::RBrace,
            "comma" => TokenKind::Comma,
            "single_quote" => TokenKind::Quote,
            "semi" => TokenKind::Semi,
            "caret" => TokenKind::Caret,
            "dollar" => TokenKind::Dollar,
            "at" => TokenKind::At,
            "underscore" => TokenKind::Underscore,

            "str" => TokenKind::Str,
            "int" => TokenKind::Int,
            "bit_bin_prefix" => TokenKind::BitBinPrefix,
            "bit_oct_prefix" => TokenKind::BitOctPrefix,
            "bit_hex_prefix" => TokenKind::BitHexPrefix,
            "int_bin" => TokenKind::IntBin,
            "int_oct" => TokenKind::IntOct,
            "int_hex" => TokenKind::IntHex,
            "real" => TokenKind::Real,
            "real_sci" => TokenKind::RealSci,
            "char" => TokenKind::Char,

            "kw_and" => TokenKind::And,
            "kw_as" => TokenKind::As,
            "kw_assoc" => TokenKind::Assoc,
            "kw_asserts" => TokenKind::Asserts,
            "kw_assumes" => TokenKind::Assumes,
            "kw_async" => TokenKind::Async,
            "kw_atomic" => TokenKind::Atomic,
            "kw_axiom" => TokenKind::Axiom,
            "kw_await" => TokenKind::Await,
            "kw_break" => TokenKind::Break,
            "kw_case" => TokenKind::Case,
            "kw_catch" => TokenKind::Catch,
            "kw_comptime" => TokenKind::Comptime,
            "kw_defer" => TokenKind::Defer,
            "kw_const" => TokenKind::Const,
            "kw_continue" => TokenKind::Continue,
            "kw_decreases" => TokenKind::Decreases,
            "kw_define" => TokenKind::Define,
            "kw_derive" => TokenKind::Derive,
            "kw_do" => TokenKind::Do,
            "kw_dyn" => TokenKind::Dyn,
            "kw_effect" => TokenKind::Effect,
            "kw_else" => TokenKind::Else,
            "kw_ensures" => TokenKind::Ensures,
            "kw_enum" => TokenKind::Enum,
            "kw_error" => TokenKind::Error,
            "kw_exists" => TokenKind::Exists,
            "kw_extend" => TokenKind::Extend,
            "kw_false" => TokenKind::False,
            "kw_fn" => TokenKind::Fn,
            "kw_fn_cap" => TokenKind::FnCap,
            "kw_for" => TokenKind::For,
            "kw_forall" => TokenKind::Forall,
            "kw_ghost" => TokenKind::Ghost,
            "kw_handles" => TokenKind::Handles,
            "kw_if" => TokenKind::If,
            "kw_implicit" => TokenKind::Implicit,
            "kw_impl" => TokenKind::Impl,
            "kw_in" => TokenKind::In,
            "kw_inline" => TokenKind::Inline,
            "kw_invariant" => TokenKind::Invariant,
            "kw_is" => TokenKind::Is,
            "kw_itself" => TokenKind::Itself,
            "kw_lambda" => TokenKind::Lambda,
            "kw_lemma" => TokenKind::Lemma,
            "kw_var" => TokenKind::Var,
            "kw_let" => TokenKind::Let,
            "kw_lift" => TokenKind::Lift,
            "kw_match" => TokenKind::Match,
            "kw_matches" => TokenKind::Matches,
            "kw_move" => TokenKind::Move,
            "kw_mod" => TokenKind::Mod,
            "kw_newtype" => TokenKind::Newtype,
            "kw_not" => TokenKind::Not,
            "kw_null" => TokenKind::Null,
            "kw_opaque" => TokenKind::Opaque,
            "kw_opens" => TokenKind::Opens,
            "kw_or" => TokenKind::Or,
            "kw_outcomes" => TokenKind::Outcomes,
            "kw_predicate" => TokenKind::Predicate,
            "kw_private" => TokenKind::Private,
            "kw_pub" => TokenKind::Pub,
            "kw_pure" => TokenKind::Pure,
            "kw_quote" => TokenKind::KwQuote,
            "kw_ref" => TokenKind::Ref,
            "kw_refines" => TokenKind::Refines,
            "kw_requires" => TokenKind::Requires,
            "kw_resume" => TokenKind::Resume,
            "kw_return" => TokenKind::Return,
            "kw_self" => TokenKind::SelfLower,
            "kw_self_cap" => TokenKind::SelfCap,
            "kw_spec" => TokenKind::Spec,
            "kw_static" => TokenKind::Static,
            "kw_struct" => TokenKind::Struct,
            "kw_template" => TokenKind::Template,
            "kw_test" => TokenKind::Test,
            "kw_trait" => TokenKind::Trait,
            "kw_true" => TokenKind::True,
            "kw_typealias" => TokenKind::Typealias,
            "kw_union" => TokenKind::Union,
            "kw_unsafe" => TokenKind::Unsafe,
            "kw_undefined" => TokenKind::Undefined,
            "kw_use" => TokenKind::Use,
            "kw_verified" => TokenKind::Verified,
            "kw_when" => TokenKind::When,
            "kw_while" => TokenKind::While,
            "kw_where" => TokenKind::Where,

            "id" => TokenKind::Id,
            "macro_content" => TokenKind::MacroContent,
            "comment" => TokenKind::Comment,
            "invalid" => TokenKind::Invalid,
            "sof" => TokenKind::Sof,
            "eof" => TokenKind::Eof,
            _ => return None,
        })
    }

    pub const ALL: &'static [Self] = &[
        TokenKind::Plus,
        TokenKind::PlusEq,
        TokenKind::PlusPlus,
        TokenKind::SeparatedPlus,
        TokenKind::Lt,
        TokenKind::LtEq,
        TokenKind::SeparatedLt,
        TokenKind::Gt,
        TokenKind::GtEq,
        TokenKind::SeparatedGt,
        TokenKind::Bang,
        TokenKind::BangEq,
        TokenKind::Minus,
        TokenKind::Arrow,
        TokenKind::MinusEq,
        TokenKind::SeparatedMinus,
        TokenKind::Dot,
        TokenKind::Colon,
        TokenKind::ColonColon,
        TokenKind::ColonTilde,
        TokenKind::ColonMinus,
        TokenKind::Star,
        TokenKind::StarEq,
        TokenKind::SeparatedStar,
        TokenKind::Slash,
        TokenKind::SlashEq,
        TokenKind::SeparatedSlash,
        TokenKind::Percent,
        TokenKind::PercentEq,
        TokenKind::SeparatedPercent,
        TokenKind::Eq,
        TokenKind::FatArrow,
        TokenKind::EqEq,
        TokenKind::EqEqEq,
        TokenKind::Tilde,
        TokenKind::TildeGt,
        TokenKind::Pipe,
        TokenKind::PipeGt,
        TokenKind::Hash,
        TokenKind::Question,
        TokenKind::Backslash,
        TokenKind::Ampersand,
        TokenKind::LBracket,
        TokenKind::RBracket,
        TokenKind::LParen,
        TokenKind::RParen,
        TokenKind::LBrace,
        TokenKind::RBrace,
        TokenKind::Comma,
        TokenKind::Quote,
        TokenKind::Semi,
        TokenKind::Caret,
        TokenKind::Dollar,
        TokenKind::At,
        TokenKind::Underscore,
        TokenKind::Str,
        TokenKind::Int,
        TokenKind::BitBinPrefix,
        TokenKind::BitOctPrefix,
        TokenKind::BitHexPrefix,
        TokenKind::IntBin,
        TokenKind::IntOct,
        TokenKind::IntHex,
        TokenKind::Real,
        TokenKind::RealSci,
        TokenKind::Char,
        TokenKind::And,
        TokenKind::As,
        TokenKind::Assoc,
        TokenKind::Asserts,
        TokenKind::Assumes,
        TokenKind::Async,
        TokenKind::Atomic,
        TokenKind::Axiom,
        TokenKind::Await,
        TokenKind::Break,
        TokenKind::Case,
        TokenKind::Catch,
        TokenKind::Comptime,
        TokenKind::Defer,
        TokenKind::Const,
        TokenKind::Continue,
        TokenKind::Decreases,
        TokenKind::Define,
        TokenKind::Derive,
        TokenKind::Do,
        TokenKind::Dyn,
        TokenKind::Effect,
        TokenKind::Else,
        TokenKind::Ensures,
        TokenKind::Enum,
        TokenKind::Error,
        TokenKind::Exists,
        TokenKind::Extend,
        TokenKind::False,
        TokenKind::Fn,
        TokenKind::FnCap,
        TokenKind::For,
        TokenKind::Forall,
        TokenKind::Ghost,
        TokenKind::Handles,
        TokenKind::If,
        TokenKind::Implicit,
        TokenKind::Impl,
        TokenKind::In,
        TokenKind::Inline,
        TokenKind::Invariant,
        TokenKind::Is,
        TokenKind::Itself,
        TokenKind::Lambda,
        TokenKind::Lemma,
        TokenKind::Var,
        TokenKind::Let,
        TokenKind::Lift,
        TokenKind::Match,
        TokenKind::Matches,
        TokenKind::Move,
        TokenKind::Mod,
        TokenKind::Newtype,
        TokenKind::Not,
        TokenKind::Null,
        TokenKind::Opaque,
        TokenKind::Opens,
        TokenKind::Or,
        TokenKind::Outcomes,
        TokenKind::Predicate,
        TokenKind::Private,
        TokenKind::Pub,
        TokenKind::Pure,
        TokenKind::KwQuote,
        TokenKind::Ref,
        TokenKind::Refines,
        TokenKind::Requires,
        TokenKind::Resume,
        TokenKind::Return,
        TokenKind::SelfLower,
        TokenKind::SelfCap,
        TokenKind::Spec,
        TokenKind::Static,
        TokenKind::Struct,
        TokenKind::Template,
        TokenKind::Test,
        TokenKind::Trait,
        TokenKind::True,
        TokenKind::Typealias,
        TokenKind::Union,
        TokenKind::Unsafe,
        TokenKind::Undefined,
        TokenKind::Use,
        TokenKind::Verified,
        TokenKind::When,
        TokenKind::While,
        TokenKind::Where,
        TokenKind::Id,
        TokenKind::MacroContent,
        TokenKind::Comment,
        TokenKind::Invalid,
        TokenKind::Sof,
        TokenKind::Eof,
    ];
}

impl Token {
    pub fn new(kind: TokenKind, from: Index, to: Index) -> Self {
        Self { kind, from, to }
    }

    pub fn keywords() -> HashMap<&'static str, TokenKind> {
        let mut map = HashMap::new();
        map.insert("and", TokenKind::And);
        map.insert("as", TokenKind::As);
        map.insert("assoc", TokenKind::Assoc);
        map.insert("asserts", TokenKind::Asserts);
        map.insert("assumes", TokenKind::Assumes);
        map.insert("async", TokenKind::Async);
        map.insert("atomic", TokenKind::Atomic);
        map.insert("axiom", TokenKind::Axiom);
        map.insert("await", TokenKind::Await);
        map.insert("break", TokenKind::Break);
        map.insert("case", TokenKind::Case);
        map.insert("catch", TokenKind::Catch);
        map.insert("comptime", TokenKind::Comptime);
        map.insert("defer", TokenKind::Defer);
        map.insert("const", TokenKind::Const);
        map.insert("continue", TokenKind::Continue);
        map.insert("decreases", TokenKind::Decreases);
        map.insert("define", TokenKind::Define);
        map.insert("derive", TokenKind::Derive);
        map.insert("do", TokenKind::Do);
        map.insert("dyn", TokenKind::Dyn);
        map.insert("effect", TokenKind::Effect);
        map.insert("else", TokenKind::Else);
        map.insert("ensures", TokenKind::Ensures);
        map.insert("enum", TokenKind::Enum);
        map.insert("error", TokenKind::Error);
        map.insert("exists", TokenKind::Exists);
        map.insert("extend", TokenKind::Extend);
        map.insert("false", TokenKind::False);
        map.insert("fn", TokenKind::Fn);
        map.insert("Fn", TokenKind::FnCap);
        map.insert("for", TokenKind::For);
        map.insert("forall", TokenKind::Forall);
        map.insert("ghost", TokenKind::Ghost);
        map.insert("handles", TokenKind::Handles);
        map.insert("if", TokenKind::If);
        map.insert("implicit", TokenKind::Implicit);
        map.insert("impl", TokenKind::Impl);
        map.insert("in", TokenKind::In);
        map.insert("inline", TokenKind::Inline);
        map.insert("invariant", TokenKind::Invariant);
        map.insert("is", TokenKind::Is);
        map.insert("itself", TokenKind::Itself);
        map.insert("lambda", TokenKind::Lambda);
        map.insert("lemma", TokenKind::Lemma);
        map.insert("let", TokenKind::Let);
        map.insert("var", TokenKind::Var);
        map.insert("lift", TokenKind::Lift);
        map.insert("match", TokenKind::Match);
        map.insert("matches", TokenKind::Matches);
        map.insert("move", TokenKind::Move);
        map.insert("mod", TokenKind::Mod);
        map.insert("newtype", TokenKind::Newtype);
        map.insert("not", TokenKind::Not);
        map.insert("null", TokenKind::Null);
        map.insert("opaque", TokenKind::Opaque);
        map.insert("opens", TokenKind::Opens);
        map.insert("or", TokenKind::Or);
        map.insert("outcomes", TokenKind::Outcomes);
        map.insert("predicate", TokenKind::Predicate);
        map.insert("private", TokenKind::Private);
        map.insert("pub", TokenKind::Pub);
        map.insert("pure", TokenKind::Pure);
        map.insert("quote", TokenKind::KwQuote);
        map.insert("ref", TokenKind::Ref);
        map.insert("refines", TokenKind::Refines);
        map.insert("requires", TokenKind::Requires);
        map.insert("resume", TokenKind::Resume);
        map.insert("return", TokenKind::Return);
        map.insert("self", TokenKind::SelfLower);
        map.insert("Self", TokenKind::SelfCap);
        map.insert("spec", TokenKind::Spec);
        map.insert("static", TokenKind::Static);
        map.insert("struct", TokenKind::Struct);
        map.insert("template", TokenKind::Template);
        map.insert("test", TokenKind::Test);
        map.insert("trait", TokenKind::Trait);
        map.insert("true", TokenKind::True);
        map.insert("typealias", TokenKind::Typealias);
        map.insert("union", TokenKind::Union);
        map.insert("unsafe", TokenKind::Unsafe);
        map.insert("undefined", TokenKind::Undefined);
        map.insert("use", TokenKind::Use);
        map.insert("verified", TokenKind::Verified);
        map.insert("when", TokenKind::When);
        map.insert("while", TokenKind::While);
        map.insert("where", TokenKind::Where);
        map.insert("_", TokenKind::Underscore);
        map
    }
}
