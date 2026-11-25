use ast::NodeKind;
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

#[test]
fn index_delimiters_are_not_synthesized_list_literals() {
    for (source, kind) in [
        ("value[7]", NodeKind::Int),
        ("value[1..8]", NodeKind::RangeFromTo),
        ("value[0..=7]", NodeKind::RangeFromToInclusive),
        ("value[..]", NodeKind::RangeFull),
        ("value[[1,2]]", NodeKind::ListOf),
        ("value[index[0]]", NodeKind::IndexApplication),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("index.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let node = parser.try_expr().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
        let ast = parser.finalize();
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::IndexApplication));
        let args = ast
            .get_multi_child_slice(ast.get_children(node)[1])
            .unwrap();
        assert_eq!(args.len(), 1, "{source}");
        assert_eq!(ast.get_node_kind(args[0]), Some(kind), "{source}");
    }
}

#[test]
fn indexing_supports_multi_named_spread_and_empty_arguments() {
    for (source, expected_len) in [
        ("value[]", 0),
        ("value[1, 2]", 2),
        ("value[1, ]", 1),
        ("value[0..2, .axis = 1, ...rest]", 3),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("index.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty(), "{source}");
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let node = parser.try_expr().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
        let ast = parser.finalize();
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::IndexApplication));
        let args = ast
            .get_multi_child_slice(ast.get_children(node)[1])
            .unwrap();
        assert_eq!(args.len(), expected_len, "{source}");
    }
}

#[test]
fn indexing_requires_a_closing_bracket() {
    for source in ["value[1", "value[1)", "value[, 1]"] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("index.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        assert!(parser.try_expr().is_err(), "{source}");
    }
}
