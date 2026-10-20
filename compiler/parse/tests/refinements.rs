use ast::NodeKind;
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};
fn parse(source: &str) -> Result<(ast::Ast, ast::NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("refinement.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let node = parser.try_expr().map_err(|e| format!("{e:?}"))?;
    assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
    Ok((parser.finalize(), node))
}
#[test]
fn refinement_preserves_requirement_base_and_parenthesized_boundaries() {
    for source in [
        "refines { Itself == bool } bool",
        "refines { Itself :- A + B } Tuple<u8,bool>",
    ] {
        let (ast, node) = parse(source).unwrap();
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::RefinedType));
        assert_eq!(ast.get_children(node).len(), 2);
    }
    let (ast, node) = parse("*(refines { Itself == bool } bool)").unwrap();
    assert_eq!(ast.get_node_kind(node), Some(NodeKind::PointerType));
}
#[test]
fn refinement_rejects_missing_delimiters_predicate_and_base() {
    for source in [
        "refines bool",
        "refines {} bool",
        "refines {true bool",
        "refines {true}",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
}
