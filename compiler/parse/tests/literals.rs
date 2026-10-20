use ast::{Ast, NodeKind, NodeType};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse_literal(source: &str, pattern: bool) -> (Ast, ast::NodeIndex, TokenKind, SourceMap) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("literal.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{source}: {errors:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let node = if pattern {
        parser.try_pattern()
    } else {
        parser.try_expr()
    }
    .unwrap_or_else(|error| panic!("{source}: {error:?}"));
    assert_ne!(node, 0, "{source}");
    let next = parser.peek_next_token().kind;
    (parser.finalize(), node, next, map)
}

#[test]
fn all_numeric_token_families_keep_exact_leaf_spans() {
    for (source, kind) in [
        ("123456789012345678901234567890", NodeKind::Int),
        ("0b101010", NodeKind::Int),
        ("0o177", NodeKind::Int),
        ("0xABCD", NodeKind::Int),
        ("1.234567890123456789", NodeKind::Real),
        ("1.0e-1234567890", NodeKind::Real),
    ] {
        let (ast, node, next, map) = parse_literal(source, false);
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(node), Some(kind));
        assert_eq!(ast.source_content(node, &map).as_deref(), Some(source));
    }
}

#[test]
fn concatenation_retains_ordered_leaf_spans_and_ast_traversal() {
    let source = "\"a\" \n\t\"b\"\"c\"";
    let (ast, node, next, map) = parse_literal(source, false);
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::StringConcat));
    assert_eq!(NodeKind::StringConcat.node_type(), NodeType::MultiChildren);
    let leaves = ast
        .get_multi_child_slice(ast.get_children(node)[0])
        .unwrap();
    assert_eq!(leaves.len(), 3);
    for (&leaf, text) in leaves.iter().zip(["\"a\"", "\"b\"", "\"c\""]) {
        assert_eq!(ast.get_node_kind(leaf), Some(NodeKind::Str));
        assert_eq!(ast.source_content(leaf, &map).as_deref(), Some(text));
    }
    assert_eq!(ast.source_content(node, &map).as_deref(), Some(source));
    let dump = ast.dump_to_s_expression(node, &map);
    assert!(dump.contains("StringConcat"), "{dump}");
    assert_eq!(dump.matches("(Str ").count(), 3, "{dump}");
}

#[test]
fn comments_and_suffixes_end_a_string_group() {
    for source in ["\"a\" {- comment -} \"b\"", "\"a\" -- comment\n \"b\""] {
        let (ast, node, next, _) = parse_literal(source, false);
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::Str));
        // The enclosing production must reject the remaining string, rather
        // than hide a comment in a newly synthesized literal value.
        assert_eq!(next, TokenKind::Str);
    }
    let (ast, node, next, _) = parse_literal("\"a\"tag \"b\"", false);
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::LiteralExtension));
    assert_eq!(next, TokenKind::Str);
}

#[test]
fn suffix_is_one_identifier_and_does_not_swallow_literal_source() {
    for pattern in [false, true] {
        for (source, literal, suffix) in [
            ("100u32px", "100", "u32px"),
            ("1.0e-6units", "1.0e-6", "units"),
            ("\"a\" \"b\"text", "\"a\" \"b\"", "text"),
        ] {
            let (ast, node, next, map) = parse_literal(source, pattern);
            assert_eq!(next, TokenKind::Eof);
            assert_eq!(ast.get_node_kind(node), Some(NodeKind::LiteralExtension));
            let children = ast.get_children(node);
            assert_eq!(
                ast.source_content(children[0], &map).as_deref(),
                Some(literal)
            );
            assert_eq!(
                ast.source_content(children[1], &map).as_deref(),
                Some(suffix)
            );
            assert_eq!(ast.source_content(node, &map).as_deref(), Some(source));
        }
    }
}

#[test]
fn arbitrary_expressions_and_booleans_do_not_become_suffix_receivers() {
    for source in ["name tag", "true tag", "false tag", "() tag", "null tag"] {
        let (ast, node, next, _) = parse_literal(source, false);
        assert_ne!(ast.get_node_kind(node), Some(NodeKind::LiteralExtension));
        assert_eq!(next, TokenKind::Id, "{source}");
    }
}

#[test]
fn a_second_suffix_is_a_parse_error() {
    let map = SourceMap::new(FilePathMapping::empty());
    let source = "100unit tag";
    let file = map.new_source_file(FileName::Custom("literal.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let error = parser.try_expr().unwrap_err();
    assert!(format!("{error:?}").contains("only one suffix"));
}

#[test]
fn prefixed_patterns_preserve_bitvec_identity() {
    for (source, expected) in [
        ("0b1010", NodeKind::BitVecBinPattern),
        ("0o123", NodeKind::BitVecOctPattern),
        ("0xABCD", NodeKind::BitVecHexPattern),
    ] {
        let (ast, node, next, _) = parse_literal(source, true);
        assert_eq!(next, TokenKind::Eof);
        assert_eq!(ast.get_node_kind(node), Some(expected));
    }
}

#[test]
fn comment_trivia_preserves_identifier_symbols_and_source_gaps() {
    let source = "{- preceding -} 12 {- between -} units";
    let (ast, node, next, map) = parse_literal(source, false);
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::LiteralExtension));
    let suffix = ast.get_children(node)[1];
    assert_eq!(ast.source_content(suffix, &map).as_deref(), Some("units"));
    let dump = ast.dump_to_s_expression(node, &map);
    assert!(dump.contains("units"), "{dump}");
}

#[test]
fn object_view_chains_preserve_individual_names_and_spans() {
    for source in ["{}'a'type", "{}'Σ'type", "{ field: 1 } 'a' type"] {
        let (ast, outer, next, map) = parse_literal(source, false);
        assert_eq!(next, TokenKind::Eof);
        assert_eq!(ast.get_node_kind(outer), Some(NodeKind::TakeView));
        let children = ast.get_children(outer);
        let inner = children[0];
        assert_eq!(ast.get_node_kind(inner), Some(NodeKind::TakeView));
        assert_eq!(
            ast.source_content(children[1], &map).as_deref(),
            Some("type")
        );
        let first_name = ast.get_children(inner)[1];
        let expected = if source.contains('Σ') { "Σ" } else { "a" };
        assert_eq!(
            ast.source_content(first_name, &map).as_deref(),
            Some(expected)
        );
        let dump = ast.dump_to_s_expression(outer, &map);
        assert_eq!(dump.matches("(TakeView").count(), 2, "{dump}");
    }
}

#[test]
fn pattern_parentheses_group_or_form_unit_and_tuples() {
    for (source, kind) in [
        ("(true)", NodeKind::Bool),
        ("()", NodeKind::Unit),
        ("(true,)", NodeKind::TuplePattern),
        ("(true, false)", NodeKind::TuplePattern),
        ("(true or false) as value", NodeKind::AsBindPattern),
    ] {
        let (ast, node, next, _) = parse_literal(source, true);
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(node), Some(kind), "{source}");
    }
}

#[test]
fn pattern_guards_leave_control_flow_do_for_the_enclosing_parser() {
    let (ast, node, next, _) = parse_literal("value if ready do { }", true);
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::IfGuardPattern));
    assert_eq!(next, TokenKind::Do);
}

#[test]
fn let_else_retains_all_four_children_in_ast_traversal() {
    let source = "let true as value: Flag = input else { return; }";
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("binding.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let node = parser.try_let_statement().unwrap();
    assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
    let ast = parser.finalize();
    assert_eq!(NodeKind::LetDecl.node_type(), NodeType::QuadrupleChildren);
    let children = ast.get_children(node);
    assert_eq!(children.len(), 4);
    assert_eq!(
        ast.get_node_kind(children[0]),
        Some(NodeKind::AsBindPattern)
    );
    assert_eq!(ast.get_node_kind(children[3]), Some(NodeKind::Block));
    let dump = ast.dump_to_s_expression(node, &map);
    assert!(dump.contains("Return"), "{dump}");
}

#[test]
fn expression_patterns_stop_at_their_angle_delimiter() {
    for source in [
        " < E.flag(true) > ",
        " < (left > right) > ",
        " < f(left > right) > ",
    ] {
        let (ast, node, next, _) = parse_literal(source, true);
        assert_eq!(
            ast.get_node_kind(node),
            Some(NodeKind::ExprAsPattern),
            "{source}"
        );
        assert_eq!(next, TokenKind::Eof, "{source}");
    }
    let (_, _, next, _) = parse_literal(" < true > => false", true);
    assert_eq!(next, TokenKind::FatArrow);
}

#[test]
fn bare_type_clause_retains_its_identifier_in_ast_traversal() {
    let source = "T";
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("clause.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let node = parser.try_clause().unwrap();
    let ast = parser.finalize();
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::TypeDeclClause));
    let child = ast.get_children(node)[0];
    assert_eq!(ast.get_node_kind(child), Some(NodeKind::Id));
    assert_eq!(ast.source_content(child, &map).as_deref(), Some("T"));
    assert_ne!(NodeKind::TypeDeclClause.node_type(), NodeType::NoChild);
}

#[test]
fn qualified_generic_pattern_heads_parse_type_expressions_in_diamonds() {
    for source in [
        "Choice<true'type>.value(x)",
        "Choice<Box<'x''type>>.value(x)",
        "pkg.Choice<(true'type, 'x''type)>.value(x)",
    ] {
        let (ast, node, next, _) = parse_literal(source, true);
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::ApplicationPattern));
        let projection = ast.get_children(node)[0];
        assert_eq!(ast.get_node_kind(projection), Some(NodeKind::Projection));
        let head = ast.get_children(projection)[0];
        assert_eq!(
            ast.get_node_kind(head),
            Some(NodeKind::NormalFormHeadApplication)
        );
    }
}

#[test]
fn unqualified_type_patterns_retain_pattern_argument_roles() {
    let (ast, node, next, _) = parse_literal("Array<'Expected, count>", true);
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(
        ast.get_node_kind(node),
        Some(NodeKind::NormalFormHeadApplicationPattern)
    );
    let arguments = ast
        .get_multi_child_slice(ast.get_children(node)[1])
        .unwrap();
    assert_eq!(arguments.len(), 2);
    assert_eq!(
        ast.get_node_kind(arguments[0]),
        Some(NodeKind::TypeBindPattern)
    );
    assert_eq!(ast.get_node_kind(arguments[1]), Some(NodeKind::Id));
}

#[test]
fn bitvec_prefixes_accept_real_subpatterns_and_keep_computed_segments() {
    for (source, kind, count) in [
        ("0b(value: Byte)", NodeKind::BitVecBinPattern, 1),
        ("0o(_: Byte) 007", NodeKind::BitVecOctPattern, 2),
        ("0x AF ((60 or 61): Byte)", NodeKind::BitVecHexPattern, 2),
        ("0b < value >", NodeKind::BitVecBinPattern, 1),
        ("0b<value>", NodeKind::BitVecBinPattern, 1),
    ] {
        let (ast, node, next, _) = parse_literal(source, true);
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(node), Some(kind));
        assert_eq!(
            ast.get_multi_child_slice(ast.get_children(node)[0])
                .unwrap()
                .len(),
            count
        );
    }
    for source in ["0b", "0o", "0x", "0b()", "0b(value:)"] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("pattern.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        assert!(parser.try_pattern().is_err(), "{source}");
    }
}

#[test]
fn range_endpoints_leave_guards_and_alternatives_on_the_whole_pattern() {
    for (source, outer, inner) in [
        (
            "0..=10 if false",
            NodeKind::IfGuardPattern,
            NodeKind::RangeFromToInclusivePattern,
        ),
        (
            "..=10 if false",
            NodeKind::IfGuardPattern,
            NodeKind::RangeToInclusivePattern,
        ),
        (
            "0..10 or 20..30",
            NodeKind::OrPattern,
            NodeKind::RangeFromToPattern,
        ),
        (
            "..10 as value",
            NodeKind::AsBindPattern,
            NodeKind::RangeToPattern,
        ),
    ] {
        let (ast, node, next, _) = parse_literal(source, true);
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(node), Some(outer), "{source}");
        assert_eq!(
            ast.get_node_kind(ast.get_children(node)[0]),
            Some(inner),
            "{source}"
        );
    }
    let (ast, node, next, _) = parse_literal("..", true);
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::RangeFull));
}
