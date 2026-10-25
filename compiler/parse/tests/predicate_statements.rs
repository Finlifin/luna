use ast::{NodeKind, NodeType};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

#[test]
fn predicate_statements_preserve_label_and_predicate_slots() {
    for (keyword, kind) in [
        ("axiom", NodeKind::Axiom),
        ("requires", NodeKind::Requires),
        ("ensures", NodeKind::Ensures),
        ("asserts", NodeKind::Asserts),
        ("assumes", NodeKind::Assumes),
        ("invariant", NodeKind::Invariant),
        ("decreases", NodeKind::Decreases),
    ] {
        let source = format!("{keyword} left == right; {keyword} true;");
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("predicates.fl".into()), source.clone());
        let (tokens, symbols, errors) = lex::lex(&source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let first = parser.try_statement_or_definition().unwrap();
        assert!(parser.eat_token(TokenKind::Semi));
        let second = parser.try_statement_or_definition().unwrap();
        assert!(parser.eat_token(TokenKind::Semi));
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
        let ast = parser.finalize();
        assert_eq!(kind.node_type(), NodeType::DoubleChildren);
        for (node, predicate_kind) in [(first, NodeKind::BoolEq), (second, NodeKind::Bool)] {
            assert_eq!(ast.get_node_kind(node), Some(kind), "{source}");
            let children = ast.get_children(node);
            assert_eq!(children[0], 0, "statement has no label: {source}");
            assert_eq!(
                ast.get_node_kind(children[1]),
                Some(predicate_kind),
                "{source}"
            );
        }
    }
}

#[test]
fn predicate_statements_reject_missing_predicates() {
    for keyword in [
        "axiom",
        "requires",
        "ensures",
        "asserts",
        "assumes",
        "invariant",
        "decreases",
    ] {
        let source = format!("{keyword};");
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("predicates.fl".into()), source.clone());
        let (tokens, symbols, _) = lex::lex(&source, file.start_pos);
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        assert!(parser.try_statement_or_definition().is_err(), "{source}");
    }
}
