use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse(source: &str) -> Result<(Ast, NodeIndex, TokenKind), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("closures.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let node = parser.try_expr().map_err(|e| format!("{e:?}"))?;
    let next = parser.peek_next_token().kind;
    Ok((parser.finalize(), node, next))
}

#[test]
fn environment_bindings_preserve_types_sources_order_and_body() {
    let (ast, root, next) =
        parse("%{first: A = value, second: fn(A) -> B,} comptime fn(B) -> A").unwrap();
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(
        ast.get_node_kind(root),
        Some(NodeKind::ClosureQualifiedType)
    );
    let children = ast.get_children(root);
    assert_eq!(
        ast.get_node_kind(children[0]),
        Some(NodeKind::ClosureEnvironment)
    );
    assert_eq!(ast.get_node_kind(children[1]), Some(NodeKind::Arrow));
    let bindings = ast
        .get_multi_child_slice(ast.get_children(children[0])[0])
        .unwrap();
    assert_eq!(bindings.len(), 2);
    for &binding in bindings {
        assert_eq!(
            ast.get_node_kind(binding),
            Some(NodeKind::ClosureEnvironmentBinding)
        );
    }
    assert_ne!(ast.get_children(bindings[0])[2], 0);
    assert_eq!(ast.get_children(bindings[1])[2], 0);
}

#[test]
fn environment_prefix_keeps_percent_infix_and_accepts_whitespace() {
    for source in [
        "%{} T",
        "%C fn() -> T",
        " % C fn() -> T",
        "%{inner: %{x: A} fn() -> A} fn() -> A",
    ] {
        let (ast, root, next) = parse(source).unwrap();
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(
            ast.get_node_kind(root),
            Some(NodeKind::ClosureQualifiedType)
        );
    }
    let (ast, root, next) = parse("left % right").unwrap();
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Mod));
}

#[test]
fn malformed_environment_bindings_and_missing_bodies_are_rejected() {
    for source in [
        "%",
        "%{}",
        "%{x} T",
        "%{x:} T",
        "%{x:A =} T",
        "%{x:A",
        "%{x:A,,} T",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    let old = parse("^C fn() -> T");
    assert!(old.is_err() || old.unwrap().1 == 0);
}

#[test]
fn closure_qualification_contains_arrows_but_not_outer_type_propositions() {
    let (ast, root, next) = parse("%{x:A} fn(A) -> A == %{renamed:A} fn(A) -> A").unwrap();
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::BoolEq));
    for &side in ast.get_children(root) {
        assert_eq!(
            ast.get_node_kind(side),
            Some(NodeKind::ClosureQualifiedType)
        );
        assert_eq!(
            ast.get_node_kind(ast.get_children(side)[1]),
            Some(NodeKind::Arrow)
        );
    }
}
