use ast::NodeKind;
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

#[test]
fn extended_calls_keep_spread_operands_and_properties_distinct() {
    for (source, expected) in [
        (
            "f {first:true,...packet(),last:false}",
            vec![NodeKind::Property, NodeKind::ExpandArg, NodeKind::Property],
        ),
        (
            "f {...(1,true),flag:false}",
            vec![NodeKind::ExpandArg, NodeKind::Property],
        ),
        (
            "f {flag:true,...do {(1,true)}}",
            vec![NodeKind::Property, NodeKind::ExpandArg],
        ),
        ("f {...packet()}", vec![NodeKind::ExpandArg]),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("extended.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_expr().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
        let ast = parser.finalize();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::ExtendedApplication));
        let inputs = ast
            .get_multi_child_slice(ast.get_children(root)[1])
            .unwrap();
        assert_eq!(
            inputs
                .iter()
                .map(|node| ast.get_node_kind(*node).unwrap())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn extended_spread_requires_its_operand() {
    for source in ["f {...}", "f {flag:true,...}", "f {...,flag:true}"] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("extended.fl".into()), source.into());
        let (tokens, symbols, _) = lex::lex(source, file.start_pos);
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        assert!(parser.try_expr().is_err(), "{source}");
    }
}

#[test]
fn pattern_guards_parse_constructors_as_expressions() {
    for source in ["_ if Box {flag:true}.flag", "_ if check {flag:true}"] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("guard.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_pattern().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
        let ast = parser.finalize();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::IfGuardPattern));
        let guard = ast.get_children(root)[1];
        let constructor = if source.ends_with(".flag") {
            assert_eq!(ast.get_node_kind(guard), Some(NodeKind::Projection));
            ast.get_children(guard)[0]
        } else {
            guard
        };
        assert_eq!(
            ast.get_node_kind(constructor),
            Some(NodeKind::ExtendedApplication)
        );
    }
}

#[test]
fn pattern_guard_brace_and_do_boundaries_remain_owned_by_the_caller() {
    for (source, no_extended, next) in [
        ("_ if ready {}", true, TokenKind::LBrace),
        ("_ if Box {flag:true}.flag do {}", false, TokenKind::Do),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("guard-boundary.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = if no_extended {
            parser.try_pattern_without_extended_call()
        } else {
            parser.try_pattern()
        }
        .unwrap();
        assert_eq!(parser.peek_next_token().kind, next, "{source}");
        let ast = parser.finalize();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::IfGuardPattern));
    }
}

#[test]
fn pattern_guard_constructor_errors_are_not_reinterpreted_as_patterns() {
    for source in ["_ if", "_ if Box {flag:}.flag", "_ if Box {flag:true"] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("bad-guard.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        assert!(parser.try_pattern().is_err(), "{source}");
    }
}
