use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

fn parse(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("while-patterns.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_while_loop()
        .map_err(|error| format!("{error:?}"))?;
    if root == 0 || parser.peek_next_token().kind != TokenKind::Eof {
        return Err("incomplete loop".into());
    }
    Ok((parser.finalize(), root))
}

#[test]
fn multi_arm_while_keeps_label_and_comma_separated_arms() {
    for (source, labeled) in [
        (
            "while:scan value is do {true=>continue scan,false=>break scan,}",
            true,
        ),
        (
            "while value is do {true=>{continue;},false=>{break;}}",
            false,
        ),
    ] {
        let (ast, root) = parse(source).unwrap();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::WhileMatch));
        let children = ast.get_children(root);
        assert_eq!(children[0] != 0, labeled);
        if labeled {
            assert_eq!(ast.get_node_kind(children[0]), Some(NodeKind::Id));
        }
        let arms = ast.get_multi_child_slice(children[2]).unwrap();
        assert_eq!(arms.len(), 2);
        assert!(
            arms.iter()
                .all(|arm| ast.get_node_kind(*arm) == Some(NodeKind::CaseArm))
        );
    }
}

#[test]
fn multi_arm_while_rejects_wrong_separators_and_missing_labels() {
    for source in [
        "while flag is do {true=>continue;false=>break;}",
        "while flag is do {true=>continue false=>break}",
        "while value is do {true=>{continue;} false=>{break;}}",
        "while: is do {true=>break}",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
}
