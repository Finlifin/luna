use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse(source: &str) -> (Ast, NodeIndex) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("precedence.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{source}: {errors:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_expr()
        .unwrap_or_else(|error| panic!("{source}: {error:?}"));
    assert_ne!(root, 0);
    assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
    (parser.finalize(), root)
}

#[test]
fn function_result_arrows_nest_right_and_parentheses_keep_explicit_scope() {
    for source in ["fn() -> fn() -> bool", "comptime fn() -> comptime fn() -> bool"] {
        let (ast, root) = parse(source);
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::Arrow));
        let children = ast.get_children(root);
        assert_eq!(ast.get_node_kind(children[0]), Some(NodeKind::FnType));
        assert_eq!(ast.get_node_kind(children[1]), Some(NodeKind::Arrow));
    }
    let (ast, root) = parse("(fn() -> fn()) -> bool");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Arrow));
    assert_ne!(ast.get_node_kind(ast.get_children(root)[1]), Some(NodeKind::Arrow));
    let (ast, root) = parse("a - b - c");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Sub));
    assert_eq!(ast.get_node_kind(ast.get_children(root)[0]), Some(NodeKind::Sub));
}

#[test]
fn prefixes_stop_before_infix_operators() {
    for (source, outer, prefix) in [
        ("-1.5 != value", NodeKind::BoolNotEq, NodeKind::Negative),
        ("-a + b", NodeKind::Add, NodeKind::Negative),
        ("-a * b", NodeKind::Mul, NodeKind::Negative),
        ("not a and b", NodeKind::BoolAnd, NodeKind::BoolNot),
        ("not a or b", NodeKind::BoolOr, NodeKind::BoolNot),
        ("*A -> B", NodeKind::Arrow, NodeKind::PointerType),
        ("?A == B", NodeKind::BoolEq, NodeKind::OptionalType),
        ("dyn A == B", NodeKind::BoolEq, NodeKind::TraitObjectType),
        ("lift A == B", NodeKind::BoolEq, NodeKind::LiftType),
        ("error a == b", NodeKind::BoolEq, NodeKind::ErrorNew),
    ] {
        let (ast, root) = parse(source);
        assert_eq!(ast.get_node_kind(root), Some(outer), "{source}");
        assert_eq!(
            ast.get_node_kind(ast.get_children(root)[0]),
            Some(prefix),
            "{source}"
        );
    }
}

#[test]
fn prefixes_include_postfix_chains_and_parentheses_override_scope() {
    for (source, prefix, child) in [
        ("-f(x)", NodeKind::Negative, NodeKind::Application),
        ("not value.flag", NodeKind::BoolNot, NodeKind::Projection),
        ("-items[0]", NodeKind::Negative, NodeKind::IndexApplication),
        (
            "*Pair<A, B>",
            NodeKind::PointerType,
            NodeKind::NormalFormHeadApplication,
        ),
        (
            "not Flag { value: true }.value",
            NodeKind::BoolNot,
            NodeKind::Projection,
        ),
        ("not not value", NodeKind::BoolNot, NodeKind::BoolNot),
    ] {
        let (ast, root) = parse(source);
        assert_eq!(ast.get_node_kind(root), Some(prefix), "{source}");
        assert_eq!(
            ast.get_node_kind(ast.get_children(root)[0]),
            Some(child),
            "{source}"
        );
    }
    let (ast, root) = parse("not (a or b)");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::BoolNot));
    let (ast, root) = parse("(-a) + b");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Add));
}

#[test]
fn bool_type_keyword_is_not_a_boolean_literal_or_bindable_identifier() {
    let (ast, node) = parse("bool");
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::BoolType));
    for literal in ["true", "false"] {
        let (ast, node) = parse(literal);
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::Bool));
    }
}
