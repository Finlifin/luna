use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("effects.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{errors:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_statement_or_definition()
        .map_err(|e| format!("{e:?}"))?;
    parser.eat_token(TokenKind::Semi);
    if root == 0 || parser.peek_next_token().kind != TokenKind::Eof {
        return Err("incomplete declaration".into());
    }
    Ok((parser.finalize(), root))
}

#[test]
fn standalone_operations_keep_parameters_results_and_no_body() {
    for source in [
        "effect read() -> bool;",
        "effect log(value: bool);",
        "effect read() -> !Failure bool;",
    ] {
        let (ast, root) = parse(source).unwrap();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::AlgebraicEffect));
        assert_eq!(ast.get_children(root)[4], 0);
    }
}

#[test]
fn effect_group_members_are_independent_operation_signatures() {
    let (ast, root) = parse("effect enum Report { count() -> bool, message(text: str), }").unwrap();
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::EffectEnumDef));
    let block = ast.get_children(root)[2];
    let members = ast
        .get_multi_child_slice(ast.get_children(block)[0])
        .unwrap();
    assert_eq!(members.len(), 2);
    for &member in members {
        assert_eq!(ast.get_node_kind(member), Some(NodeKind::AlgebraicEffect));
        assert_eq!(ast.get_children(member)[4], 0);
    }
    assert_ne!(ast.get_children(members[0])[2], 0);
    assert_eq!(ast.get_children(members[1])[2], 0);
    assert_eq!(
        ast.get_multi_child_slice(ast.get_children(members[0])[1])
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        ast.get_multi_child_slice(ast.get_children(members[1])[1])
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn group_operation_modifiers_and_attributes_wrap_the_signature() {
    let (ast, root) =
        parse("effect enum IO { ^attrs.marker async inline read() -> bool, }").unwrap();
    let block = ast.get_children(root)[2];
    let member = ast
        .get_multi_child_slice(ast.get_children(block)[0])
        .unwrap()[0];
    assert_eq!(ast.get_node_kind(member), Some(NodeKind::Attribute));
    let mut inner = ast.get_children(member)[1];
    for _ in 0..2 {
        assert_eq!(ast.get_node_kind(inner), Some(NodeKind::AttributeSetTrue));
        inner = ast.get_children(inner)[1];
    }
    assert_eq!(ast.get_node_kind(inner), Some(NodeKind::AlgebraicEffect));
}

#[test]
fn bodies_payload_variants_and_missing_delimiters_are_rejected() {
    for source in [
        "effect read() -> bool { true }",
        "effect read() -> bool = true;",
        "effect read() -> bool",
        "effect () -> bool;",
        "effect read;",
        "effect enum { read(), }",
        "effect enum Report;",
        "effect enum Report { value, }",
        "effect enum Report { value: bool, }",
        "effect enum Report { read() = true, }",
        "effect enum Report { read() {} }",
        "effect enum Report { read() next() }",
    ] {
        assert!(parse(source).is_err(), "accepted: {source}");
    }
}

#[test]
fn removed_handles_signature_is_rejected_explicitly() {
    let error = parse("fn read() -> bool handles IO { true }").unwrap_err();
    assert!(error.contains("do not have a `handles` clause"), "{error}");
    parse("fn read() -> #IO bool { true }").unwrap();
}

#[test]
fn standalone_effect_modifiers_are_definition_attributes() {
    for (source, count) in [
        ("inline effect read() -> bool;", 1),
        ("async effect read() -> bool;", 1),
        ("async inline effect read() -> bool;", 2),
    ] {
        let (ast, mut node) = parse(source).unwrap();
        for _ in 0..count {
            assert_eq!(ast.get_node_kind(node), Some(NodeKind::AttributeSetTrue));
            node = ast.get_children(node)[1];
        }
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::AlgebraicEffect));
    }
    let (ast, root) = parse("inline while true { break; }").unwrap();
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::InlineStatement));
}

#[test]
fn nested_effect_qualifiers_do_not_parse_the_inner_prefix_as_a_request() {
    let (ast, root) = parse("effect compute() -> #read #read bool;").unwrap();
    let outer = ast.get_children(root)[2];
    assert_eq!(
        ast.get_node_kind(outer),
        Some(NodeKind::EffectQualifiedType)
    );
    let inner = ast.get_children(outer)[1];
    assert_eq!(
        ast.get_node_kind(inner),
        Some(NodeKind::EffectQualifiedType)
    );
    assert_ne!(
        ast.get_node_kind(ast.get_children(outer)[0]),
        Some(NodeKind::EffectElimination)
    );
}

#[test]
fn inline_functions_are_definition_modifiers_not_inline_statements() {
    for (source, count) in [
        ("inline fn f()->bool {true}", 1),
        ("pure inline fn f()->bool {true}", 2),
        ("comptime inline fn f()->bool {true}", 2),
    ] {
        let (ast, mut node) = parse(source).unwrap();
        for _ in 0..count {
            assert_eq!(ast.get_node_kind(node), Some(NodeKind::AttributeSetTrue));
            node = ast.get_children(node)[1];
        }
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::Function));
    }
}
