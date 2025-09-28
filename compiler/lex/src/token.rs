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
    Str,     // "..."
    Int,     // 123
    IntBin,  // 0b1010
    IntOct,  // 0o777
    IntHex,  // 0xFF
    Real,    // 123.45
    RealSci, // 1.23e-4 (scientific notation)
    Char,    // 'a' or '\n' or '\x{1F600}'

    // keywords
    And,       // and
    As,        // as
    Asserts,   // asserts
    Assumes,   // assumes
    Async,     // async
    Atomic,    // atomic
    Axiom,     // axiom
    Await,     // await
    Bool,      // bool
    Break,     // break
    Case,      // case
    Catch,     // catch
    Comptime,  // comptime
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
    Extern,    // extern
    False,     // false
    Fn,        // fn
    FnCap,     // Fn
    For,       // for
    Forall,    // forall
    Ghost,     // ghost
    Handles,   // handles
    If,        // if
    Impl,      // impl
    In,        // in
    Inline,    // inline
    Invariant, // invariant
    Is,        // is
    Itself,    // itself
    Lambda,    // lambda
    Lemma,     // lemma
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
    Test,      // test
    Trait,     // trait
    True,      // true
    Typealias, // typealias
    Union,     // union
    Unsafe,    // unsafe
    Use,       // use
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
            TokenKind::Asserts => "asserts",
            TokenKind::Assumes => "assumes",
            TokenKind::Async => "async",
            TokenKind::Atomic => "atomic",
            TokenKind::Axiom => "axiom",
            TokenKind::Await => "await",
            TokenKind::Bool => "bool",
            TokenKind::Break => "break",
            TokenKind::Case => "case",
            TokenKind::Catch => "catch",
            TokenKind::Comptime => "comptime",
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
            TokenKind::Extern => "extern",
            TokenKind::False => "false",
            TokenKind::Fn => "fn",
            TokenKind::FnCap => "Fn",
            TokenKind::For => "for",
            TokenKind::Forall => "forall",
            TokenKind::Ghost => "ghost",
            TokenKind::Handles => "handles",
            TokenKind::If => "if",
            TokenKind::Impl => "impl",
            TokenKind::In => "in",
            TokenKind::Inline => "inline",
            TokenKind::Invariant => "invariant",
            TokenKind::Is => "is",
            TokenKind::Itself => "itself",
            TokenKind::Lambda => "lambda",
            TokenKind::Lemma => "lemma",
            TokenKind::Let => "let",
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
            TokenKind::Test => "test",
            TokenKind::Trait => "trait",
            TokenKind::True => "true",
            TokenKind::Typealias => "typealias",
            TokenKind::Union => "union",
            TokenKind::Unsafe => "unsafe",
            TokenKind::Use => "use",
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
            TokenKind::IntBin => "<binary integer literal>",
            TokenKind::IntOct => "<octal integer literal>",
            TokenKind::IntHex => "<hexadecimal integer literal>",
            TokenKind::Real => "<real literal>",
            TokenKind::RealSci => "<scientific notation literal>",
            TokenKind::Char => "<character literal>",
        }
    }
}

impl Token {
    pub fn new(kind: TokenKind, from: Index, to: Index) -> Self {
        Self { kind, from, to }
    }

    pub fn keywords() -> HashMap<&'static str, TokenKind> {
        let mut map = HashMap::new();
        map.insert("and", TokenKind::And);
        map.insert("as", TokenKind::As);
        map.insert("asserts", TokenKind::Asserts);
        map.insert("assumes", TokenKind::Assumes);
        map.insert("async", TokenKind::Async);
        map.insert("atomic", TokenKind::Atomic);
        map.insert("axiom", TokenKind::Axiom);
        map.insert("await", TokenKind::Await);
        map.insert("bool", TokenKind::Bool);
        map.insert("break", TokenKind::Break);
        map.insert("case", TokenKind::Case);
        map.insert("catch", TokenKind::Catch);
        map.insert("comptime", TokenKind::Comptime);
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
        map.insert("extern", TokenKind::Extern);
        map.insert("false", TokenKind::False);
        map.insert("fn", TokenKind::Fn);
        map.insert("Fn", TokenKind::FnCap);
        map.insert("for", TokenKind::For);
        map.insert("forall", TokenKind::Forall);
        map.insert("ghost", TokenKind::Ghost);
        map.insert("handles", TokenKind::Handles);
        map.insert("if", TokenKind::If);
        map.insert("impl", TokenKind::Impl);
        map.insert("in", TokenKind::In);
        map.insert("inline", TokenKind::Inline);
        map.insert("invariant", TokenKind::Invariant);
        map.insert("is", TokenKind::Is);
        map.insert("itself", TokenKind::Itself);
        map.insert("lambda", TokenKind::Lambda);
        map.insert("lemma", TokenKind::Lemma);
        map.insert("let", TokenKind::Let);
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
        map.insert("test", TokenKind::Test);
        map.insert("trait", TokenKind::Trait);
        map.insert("true", TokenKind::True);
        map.insert("typealias", TokenKind::Typealias);
        map.insert("union", TokenKind::Union);
        map.insert("unsafe", TokenKind::Unsafe);
        map.insert("use", TokenKind::Use);
        map.insert("when", TokenKind::When);
        map.insert("while", TokenKind::While);
        map.insert("where", TokenKind::Where);
        map.insert("_", TokenKind::Underscore);
        map
    }
}
