use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

fn parse(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(
        FileName::Custom("enum-configuration.fl".into()),
        source.into(),
    );
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_statement_or_definition()
        .map_err(|error| format!("{error:?}"))?;
    if root == 0 || parser.peek_next_token().kind != TokenKind::Eof {
        return Err("incomplete declaration".into());
    }
    Ok((parser.finalize(), root))
}

#[test]
fn enum_configurations_are_distinct_from_variants_and_member_attributes() {
    let (ast, root) = parse(
        "enum Instruction {
        .base_type u32, .width 16 + 16, .discrete_tag true,
        add: 0b0000000 (rs2:u5) (rs1:u5) 000 (rd:u5) 0110011,
        sub: 0b0100000 (rs2:u5) (rs1:u5) 000 (rd:u5) 0110011,
    }",
    )
    .unwrap();
    let body = ast.get_children(root)[2];
    let elements = ast
        .get_multi_child_slice(ast.get_children(body)[0])
        .unwrap();
    assert_eq!(
        elements
            .iter()
            .map(|node| ast.get_node_kind(*node).unwrap())
            .collect::<Vec<_>>(),
        vec![
            NodeKind::Configuration,
            NodeKind::Configuration,
            NodeKind::Configuration,
            NodeKind::PatternEnumVariant,
            NodeKind::PatternEnumVariant
        ]
    );
    let width = ast.get_children(elements[1]);
    assert_eq!(ast.get_node_kind(width[1]), Some(NodeKind::Add));
    assert_ne!(ast.get_children(elements[3])[1], 0);
}

#[test]
fn configuration_expression_boundaries_and_errors_are_not_variant_syntax() {
    for source in [
        "enum E {.base_type Select<u32>, a,}",
        "enum E {.width do {16 + 16}, a,}",
        "enum E {.tag .encoded, a,}",
        "enum E {.width 8, .width 16, a,}",
    ] {
        parse(source).unwrap();
    }
    for source in [
        "enum E {.base_type, a,}",
        "enum E {.width}",
        "enum E {.width = 32, a,}",
        "enum E {.width:32, a,}",
    ] {
        assert!(parse(source).is_err(), "accepted {source}");
    }
}
