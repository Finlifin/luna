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

#[test]
fn local_bindings_require_initializers() {
    for keyword in ["let", "var"] {
        for declaration in ["value", "value: bool", "(left, right)"] {
            let error = parse(&format!("{keyword} {declaration};")).unwrap_err();
            assert!(error.contains("requires an initializer"), "{error}");
        }
        let error = parse(&format!("{keyword} value = ;")).unwrap_err();
        assert!(
            error.contains("initializer expression after '='"),
            "{error}"
        );
        let (ast, root) = parse(&format!("{keyword} value: bool = true;")).unwrap();
        assert_ne!(ast.get_children(root)[2], 0);
        parse(&format!("{keyword} true = value else {{return;}};")).unwrap();
    }
    // Associated const declarations have a separate grammar and can omit a value.
    parse("trait Limit { const maximum: u32; }").unwrap();
}

#[test]
fn missing_initializer_preserves_the_next_statement() {
    let source = "var missing: bool; let present = true;";
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(
        FileName::Custom("binding-recovery.fl".into()),
        source.into(),
    );
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let error = parser.try_statement_or_definition().unwrap_err();
    assert!(format!("{error:?}").contains("requires an initializer"));
    assert!(parser.eat_token(TokenKind::Semi));
    let next = parser.try_statement_or_definition().unwrap();
    assert!(parser.eat_token(TokenKind::Semi));
    assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
    let ast = parser.finalize();
    assert_eq!(ast.get_node_kind(next), Some(NodeKind::LetDecl));
    assert_ne!(ast.get_children(next)[2], 0);
}

#[test]
fn extern_is_an_identifier_and_not_an_abi_modifier() {
    for source in [
        "let extern = true;",
        "fn extern(value: bool)->bool {value}",
        "typealias extern = bool;",
        "trait Foreign { ^attrs.abi(.c) fn imported(value: bool)->bool; }",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
    for source in [
        "extern fn imported();",
        "extern \"C\" fn imported();",
        "typealias Callback = extern fn(bool)->bool;",
        "typealias Callback = extern \"C\" fn(bool)->bool;",
    ] {
        assert!(
            parse(source).is_err(),
            "accepted obsolete modifier: {source}"
        );
    }
}
