use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn declaration(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("derivation.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{source}: {errors:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_statement_or_definition()
        .map_err(|error| format!("{error:?}"))?;
    if root == 0 || parser.peek_next_token().kind != TokenKind::Eof {
        return Err("incomplete declaration".into());
    }
    let ast = parser.finalize();
    ast.validate_node_layouts().unwrap();
    if ast.get_node_kind(root) == Some(NodeKind::DeriverDef) {
        let dump = ast.dump_to_s_expression(root, &map);
        assert!(dump.starts_with("(DeriverDef "), "{dump}");
        assert_eq!(dump, ast.dump_to_s_expression(root, &map));
    }
    Ok((ast, root))
}

#[test]
fn strategies_keep_inputs_target_clauses_and_members_separate() {
    let (ast,root)=declaration("derive fn fields(config:Config, .enabled:bool=true) for Hash where T :- Eq {assoc Element: Type = T; fn hash(self)->u64 {0}} ").unwrap();
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::DeriverDef));
    let children = ast.get_children(root);
    assert_eq!(children.len(), 5);
    assert_eq!(ast.get_node_kind(children[0]), Some(NodeKind::Id));
    assert_eq!(ast.get_multi_child_slice(children[1]).unwrap().len(), 2);
    assert_eq!(ast.get_node_kind(children[2]), Some(NodeKind::Id));
    assert_eq!(ast.get_multi_child_slice(children[3]).unwrap().len(), 1);
    assert_eq!(ast.get_node_kind(children[4]), Some(NodeKind::Block));
    let body = ast.get_children(children[4]);
    let members = ast.get_multi_child_slice(body[0]).unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(ast.get_node_kind(members[0]), Some(NodeKind::AssocDecl));
    assert_eq!(ast.get_node_kind(members[1]), Some(NodeKind::Function));
}

#[test]
fn strategies_accept_optional_inputs_and_full_trait_expressions() {
    for text in [
        "derive fn empty for Hash {}",
        "derive fn empty() for Hash {}",
        "derive fn label(text:str) for Format<.json> {fn label(self)->str {text}}",
        "derive fn fields(value:T) for Hash where T, requires T :- Eq {fn hash(self)->u64 {0}}",
        "mod strategies { ^attrs.derivation.default derive fn standard for Hash {} }",
        "pub derive fn exposed for Hash {}",
        "derive fn fields(...types:meta.List<Type>) for Hash {}",
    ] {
        declaration(text).unwrap_or_else(|e| panic!("{text}: {e}"));
    }
}

#[test]
fn applications_keep_explicit_strategy_selection_and_phase_wrapper() {
    for text in [
        "derive Hash for Record",
        "derive Hash.use(fields) for Record",
        "comptime derive Hash.use(fields) for Record",
        "derive Format<.json>.use(label) for Record where T :- Eq",
    ] {
        let (ast, mut root) = declaration(text).unwrap();
        if text.starts_with("comptime") {
            assert_eq!(ast.get_node_kind(root), Some(NodeKind::AttributeSetTrue));
            root = ast.get_children(root)[1];
        }
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::DeriveDef));
    }
}

#[test]
fn malformed_strategy_headers_and_bodies_are_rejected() {
    for text in [
        "derive fn for Hash {}",
        "derive fn missing {}",
        "derive fn missing for {}",
        "derive fn empty for Hash",
        "derive fn empty for Hash;",
        "derive fn empty for Hash = 1",
        "derive fn empty()->Type for Hash {}",
        "derive fn bad(value:) for Hash {}",
        "derive fn bad(.value:bool=) for Hash {}",
        "derive fn bad(value:bool for Hash {}",
        "derive fn bad for Hash {fn unfinished(}",
    ] {
        assert!(declaration(text).is_err(), "accepted: {text}");
    }
}

#[test]
fn default_is_contextual_only_in_member_projection() {
    let (ast, root) = declaration("attrs.derivation.default").unwrap();
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::ExprStatement));
    assert_eq!(
        ast.get_node_kind(ast.get_children(root)[0]),
        Some(NodeKind::Projection)
    );
    assert!(declaration("fn default() {}").is_err());
    assert!(declaration("derive fn default for Hash {}").is_err());
    assert!(declaration("default").is_err());
}

#[test]
fn reserved_default_member_is_available_in_qualified_imports() {
    for text in [
        "use attrs.derivation.default",
        "use attrs.derivation.default as automatic",
        "pub use attrs.derivation.default",
        "use attrs.derivation.{default as automatic, other}",
        "use attrs.{derivation.{default as automatic}, other}",
        "use .default as automatic",
        "use @attrs.derivation.default as automatic",
    ] {
        declaration(text).unwrap_or_else(|error| panic!("{text}: {error}"));
    }
}

#[test]
fn member_import_support_does_not_unreserve_default_bindings() {
    for text in [
        "use default",
        "use attrs.derivation.default as default",
        "use attrs.derivation.{default as}",
        "use attrs.derivation.{default as default}",
        "use attrs.derivation.default extra",
        "const default = true;",
        "fn default() {}",
        "let default = true;",
        "default",
    ] {
        assert!(declaration(text).is_err(), "accepted: {text}");
    }
}
