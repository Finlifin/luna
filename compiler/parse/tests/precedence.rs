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
    for source in [
        "fn() -> fn() -> bool",
        "comptime fn() -> comptime fn() -> bool",
    ] {
        let (ast, root) = parse(source);
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::Arrow));
        let children = ast.get_children(root);
        assert_eq!(ast.get_node_kind(children[0]), Some(NodeKind::FnType));
        assert_eq!(ast.get_node_kind(children[1]), Some(NodeKind::Arrow));
    }
    let (ast, root) = parse("(fn() -> fn()) -> bool");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Arrow));
    assert_ne!(
        ast.get_node_kind(ast.get_children(root)[1]),
        Some(NodeKind::Arrow)
    );
    let (ast, root) = parse("a - b - c");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Sub));
    assert_eq!(
        ast.get_node_kind(ast.get_children(root)[0]),
        Some(NodeKind::Sub)
    );
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
fn primitive_type_names_are_identifiers_and_boolean_literals_are_literals() {
    for name in ["bool", "char", "void", "NoReturn", "u8", "Type"] {
        let (ast, node) = parse(name);
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::Id), "{name}");
    }
    for literal in ["true", "false"] {
        let (ast, node) = parse(literal);
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::Bool));
    }
}

#[test]
fn function_type_visitors_keep_flags_separate_from_parameter_nodes() {
    let mut counts = Vec::new();
    for source in ["fn(bool,fn(bool))", "comptime fn(bool,fn(bool))"] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("function-type.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_expr().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
        let ast = parser.finalize();
        let children = ast.get_children(root);
        assert_eq!(children.len(), 2);
        assert_eq!(ast.get_multi_child_slice(children[1]).unwrap().len(), 2);
        let direct = ast.dump_to_s_expression(root, &map);
        let visited = ast::ast_visitor::dump_ast_to_s_expression(&ast, root, &map);
        assert_eq!(direct, visited);
        assert_eq!(direct.contains("comptime"), source.starts_with("comptime"));
        counts.push(ast::ast_visitor::count_ast_nodes(&ast, root, &map));
    }
    assert_eq!(counts[0], counts[1]);
    assert!(counts[0] >= 4);
}

#[test]
fn normal_form_callable_types_preserve_angle_inputs_and_arrow_scope() {
    for source in [
        "pure comptime fn<...Es: meta.List<Type>> -> Type",
        "pure comptime fn<T: Type, value: T> -> T",
        "pure comptime fn<> -> Type",
        "pure comptime fn<T: Type> -> pure comptime fn<value: T> -> T",
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("nf-callable.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty(), "{source}");
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser
            .try_expr()
            .unwrap_or_else(|e| panic!("{source}: {e:?}"));
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
        let ast = parser.finalize();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::Arrow));
        let input = ast.get_children(root)[0];
        assert_eq!(ast.get_node_kind(input), Some(NodeKind::FnType));
        assert_eq!(
            ast.get_children(input)[0],
            ast::FN_MOD_PURE | ast::FN_MOD_COMPTIME | ast::FN_MOD_NORMAL_FORM
        );
        let direct = ast.dump_to_s_expression(root, &map);
        assert_eq!(
            direct,
            ast::ast_visitor::dump_ast_to_s_expression(&ast, root, &map)
        );
        assert!(direct.contains("normal-form"), "{direct}");
    }
    for source in [
        "fn<Type>",
        "comptime fn<Type>",
        "pure fn<Type>",
        "pure comptime fn<...Es: meta.List<Type>",
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file =
            map.new_source_file(FileName::Custom("bad-nf-callable.fl".into()), source.into());
        let (tokens, symbols, _) = lex::lex(source, file.start_pos);
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        assert!(parser.try_expr().is_err(), "{source}");
    }
}

#[test]
fn postfix_ranges_include_the_receiver_and_preserve_inner_ranges() {
    for source in [
        "module.factory(value).field[index]",
        "constructor<T>(value).ref.*'type",
        "provider {key:value}.field",
        "receiver.as(Type)",
        "items match {_=>true}",
        "callback() do {true}",
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("postfix-span.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_expr().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
        let ast = parser.finalize();
        assert_eq!(
            ast.source_content(root, &map).as_deref(),
            Some(source),
            "{source}"
        );
        if source.starts_with("module.") {
            let projection = ast.get_children(root)[0];
            let call = ast.get_children(projection)[0];
            let callee = ast.get_children(call)[0];
            assert_eq!(
                ast.source_content(projection, &map).as_deref(),
                Some("module.factory(value).field")
            );
            assert_eq!(
                ast.source_content(call, &map).as_deref(),
                Some("module.factory(value)")
            );
            assert_eq!(
                ast.source_content(callee, &map).as_deref(),
                Some("module.factory")
            );
        }
    }
}

#[test]
fn function_ast_visitors_preserve_parameters_clauses_and_body() {
    for (source, has_body) in [
        ("fn select(value:T)->T where T {value}", true),
        ("fn select(value:T)->T where T = value", true),
        ("fn select(value:T)->T where T;", false),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file =
            map.new_source_file(FileName::Custom("function-layout.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty(), "{errors:?}");
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_statement_or_definition().unwrap();
        parser.eat_token(TokenKind::Semi);
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
        let ast = parser.finalize();
        let children = ast.get_children(root);
        assert_eq!(children.len(), 5);
        assert_eq!(ast.get_multi_child_slice(children[1]).unwrap().len(), 1);
        assert_eq!(ast.get_multi_child_slice(children[3]).unwrap().len(), 1);
        assert_eq!(children[4] != 0, has_body);
        let direct = ast.dump_to_s_expression(root, &map);
        assert_eq!(
            direct,
            ast::ast_visitor::dump_ast_to_s_expression(&ast, root, &map)
        );
        assert!(direct.contains("select"), "{direct}");
        assert!(direct.contains("value"), "{direct}");
        assert!(ast::ast_visitor::count_ast_nodes(&ast, root, &map) >= 6);
    }
}

#[test]
fn associated_declaration_keeps_default_separate_from_clauses() {
    let source = "trait Container {assoc Item:Type=bool where T;}";
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("assoc-layout.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser.try_statement_or_definition().unwrap();
    let ast = parser.finalize();
    let body = ast.get_children(root)[3];
    let member = ast
        .get_multi_child_slice(ast.get_children(body)[0])
        .unwrap()[0];
    let children = ast.get_children(member);
    assert_eq!(
        ast.source_content(children[3], &map).as_deref(),
        Some("bool")
    );
    assert_eq!(ast.get_multi_child_slice(children[4]).unwrap().len(), 1);
    let direct = ast.dump_to_s_expression(root, &map);
    assert_eq!(
        direct,
        ast::ast_visitor::dump_ast_to_s_expression(&ast, root, &map)
    );
    assert!(direct.contains("bool"), "{direct}");
    assert!(direct.contains("TypeDeclClause"), "{direct}");
    assert!(ast::ast_visitor::count_ast_nodes(&ast, root, &map) >= 5);
}

#[test]
fn postfix_pattern_ranges_include_the_entire_matched_pattern() {
    for source in [
        "Option.Some(value) as whole if whole == value",
        "Container<T>.item(value)",
        "0..=10 as value",
        "true or false as value",
        "(true or false) as value",
        "Packet {flag:true} as whole",
        "-3i32",
        "-3..= -1 as value",
        "(-3) or -4",
        "-3.5 as value",
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let input = format!("{source} => true");
        let file = map.new_source_file(FileName::Custom("pattern-span.fl".into()), input.clone());
        let (tokens, symbols, errors) = lex::lex(&input, file.start_pos);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser
            .try_pattern()
            .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        let ast = parser.finalize();
        assert_eq!(
            ast.source_content(root, &map).as_deref(),
            Some(source),
            "{source}"
        );
    }
}

#[test]
fn inline_control_is_an_expression_without_stealing_function_types() {
    for source in [
        "inline if flag {true} else {false}",
        "inline when {flag=>true,else=>false}",
        "inline if flag is do {true=>true,false=>false}",
        "inline for i in 0..2 {}",
    ] {
        let (ast, root) = parse(source);
        assert_eq!(
            ast.get_node_kind(root),
            Some(NodeKind::InlineStatement),
            "{source}"
        );
        parse(&format!("consume({source})"));
    }
    let (ast, root) = parse("inline if flag {true} else {false} and enabled");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::BoolAnd));
    let (ast, root) = parse("inline fn(bool) -> bool");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Arrow));
    assert_eq!(
        ast.get_node_kind(ast.get_children(root)[0]),
        Some(NodeKind::FnType)
    );
}

#[test]
fn operator_spans_exclude_preceding_tokens_and_following_delimiters() {
    for (source, expected) in [
        (
            "(a) + (not b)",
            vec![
                (NodeKind::Add, "(a) + (not b)"),
                (NodeKind::BoolNot, "not b"),
            ],
        ),
        (
            "-(a + b)",
            vec![(NodeKind::Negative, "-(a + b)"), (NodeKind::Add, "a + b")],
        ),
        (
            "a + not b",
            vec![(NodeKind::Add, "a + not b"), (NodeKind::BoolNot, "not b")],
        ),
        (
            "a + -b * c",
            vec![
                (NodeKind::Add, "a + -b * c"),
                (NodeKind::Mul, "-b * c"),
                (NodeKind::Negative, "-b"),
            ],
        ),
        (
            "a and not not b",
            vec![
                (NodeKind::BoolAnd, "a and not not b"),
                (NodeKind::BoolNot, "not not b"),
                (NodeKind::BoolNot, "not b"),
            ],
        ),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        // Exercise offsets in a source map containing an earlier file too.
        map.new_source_file(FileName::Custom("earlier.fl".into()), "padding".into());
        let file = map.new_source_file(FileName::Custom("operators.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_expr().unwrap();
        let ast = parser.finalize();
        let mut pending = vec![root];
        let mut actual = Vec::new();
        while let Some(node) = pending.pop() {
            let (kind, span, children) = ast.get_node(node).unwrap();
            if matches!(
                kind,
                NodeKind::Add
                    | NodeKind::Mul
                    | NodeKind::BoolAnd
                    | NodeKind::BoolNot
                    | NodeKind::Negative
            ) {
                let start = (span.lo() - file.start_pos).0 as usize;
                let end = (span.hi() - file.start_pos).0 as usize;
                actual.push((kind, &source[start..end]));
                let arity = if matches!(kind, NodeKind::BoolNot | NodeKind::Negative) {
                    1
                } else {
                    2
                };
                pending.extend(children[..arity].iter().rev().copied());
            }
        }
        assert_eq!(actual, expected, "{source}");
    }
}

#[test]
fn guard_and_as_patterns_visit_only_their_two_children() {
    for (source, count) in [
        ("value if flag", 3),
        ("value as whole", 3),
        ("(value if flag) as whole", 5),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(
            FileName::Custom("pattern-children.fl".into()),
            source.into(),
        );
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_pattern().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
        let ast = parser.finalize();
        assert_eq!(
            ast::ast_visitor::count_ast_nodes(&ast, root, &map),
            count,
            "{source}"
        );
        assert_eq!(
            ast.dump_to_s_expression(root, &map),
            ast::ast_visitor::dump_ast_to_s_expression(&ast, root, &map)
        );
    }
}

#[test]
fn constant_binding_visitors_include_failure_block() {
    for source in [
        "const value=true",
        "const true as value=false else {return false;}",
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("constant.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_const_statement().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
        let ast = parser.finalize();
        let (kind, _, children) = ast.get_node(root).unwrap();
        assert_eq!(kind, NodeKind::ConstDecl);
        assert_eq!(children[3] != 0, source.contains("else"));
        let dump = ast.dump_to_s_expression(root, &map);
        assert_eq!(
            dump,
            ast::ast_visitor::dump_ast_to_s_expression(&ast, root, &map)
        );
        if source.contains("else") {
            assert!(dump.to_lowercase().contains("return"), "{dump}");
        }
    }
}
