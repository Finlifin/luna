use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("assignment.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    if !errors.is_empty() {
        return Err(format!("{errors:?}"));
    }
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_statement_or_definition()
        .map_err(|error| format!("{error:?}"))?;
    parser.eat_token(TokenKind::Semi);
    if root == 0 || parser.peek_next_token().kind != TokenKind::Eof {
        return Err("incomplete assignment".into());
    }
    Ok((parser.finalize(), root))
}

#[test]
fn assignments_keep_their_operator_and_two_operands() {
    for (operator, kind) in [
        ("=", NodeKind::Assign),
        ("+=", NodeKind::AddAssign),
        ("-=", NodeKind::SubAssign),
        ("*=", NodeKind::MulAssign),
        ("/=", NodeKind::DivAssign),
        ("%=", NodeKind::RemAssign),
    ] {
        for target in ["value", "pointer.*", "record.field", "value[1]"] {
            let (ast, root) = parse(&format!("{target} {operator} amount + 1;")).unwrap();
            assert_eq!(ast.get_node_kind(root), Some(kind));
            let children = ast.get_children(root);
            assert!(children[0] != 0 && children[1] != 0);
        }
        let error = parse(&format!("value {operator};")).unwrap_err();
        assert!(error.contains("Expected an expression"), "{error}");
    }
    assert!(parse("value %= 2 %= 3;").is_err());
}
