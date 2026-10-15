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
