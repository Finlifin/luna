use ast::NodeKind;
use lex::TokenKind;
use parse::fragment::FragmentKind;
use parse::parser::Parser;
use rustc_span::FileName;
use rustc_span::SourceMap;
use rustc_span::source_map::FilePathMapping;

fn parse_single_ok(src: &str, kind: FragmentKind) -> (ast::Ast, ast::NodeIndex) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("fragments.fl".into()), src.into());
    let (tokens, symbols, errs) = lex::lex(src, file.start_pos);
    assert!(errs.is_empty(), "lex errors for `{src}`: {errs:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let matched = parser
        .parse_single_fragment(kind)
        .unwrap_or_else(|e| panic!("unexpected parse error for `{src}` as {kind}: {e:?}"));
    let ast = parser.finalize();
    ast.validate_structure().expect("valid AST structure");
    (ast, matched.root)
}

fn parse_single_fails(src: &str, kind: FragmentKind) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("fragments.fl".into()), src.into());
    let (tokens, symbols, errs) = lex::lex(src, file.start_pos);
    if !errs.is_empty() {
        return;
    }
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let res = parser.parse_single_fragment(kind);
    assert!(
        res.is_err(),
        "expected `{src}` to fail when parsed as single {kind}, got {res:?}"
    );
}

#[test]
fn parses_all_ten_flurry_fragment_kinds() {
    let (ast, root) = parse_single_ok("\"yes\" or \"Y\" if ok", FragmentKind::Pattern);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::IfGuardPattern));

    let (ast, root) = parse_single_ok("1 + 2 * 3", FragmentKind::Expr);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Add));

    let (ast, root) = parse_single_ok("?meta.List<Integer>", FragmentKind::Type);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::OptionalType));

    let (ast, root) = parse_single_ok("my_ident", FragmentKind::Ident);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Id));

    let (ast, root) = parse_single_ok(".ready", FragmentKind::Literal);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Symbol));
    let (ast, root) = parse_single_ok("123px", FragmentKind::Literal);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::LiteralExtension));
    let (ast, root) = parse_single_ok("null", FragmentKind::Literal);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Null));

    let (ast, root) = parse_single_ok("{ let x = 1; x + 1 }", FragmentKind::Block);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Block));

    let (ast, root) = parse_single_ok("std.io.Reader", FragmentKind::Path);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::ProjectionPath));

    let (ast, root) = parse_single_ok("let x: i32 = 10;", FragmentKind::Statement);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::LetDecl));

    let (ast, root) = parse_single_ok("fn f(x: i32) -> i32 { x }", FragmentKind::ExtendedStatement);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Function));

    let (ast, root) = parse_single_ok("pub const ANSWER: i32 = 42;", FragmentKind::Item);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Pub));
    let (ast, root) = parse_single_ok("struct Point { x: i32, y: i32 }", FragmentKind::Item);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::StructDef));
}

#[test]
fn rejects_wrong_kinds_and_adjacent_gap_on_literal_extension() {
    parse_single_fails("fn f() {}", FragmentKind::Statement);
    parse_single_fails("struct S { x: i32 }", FragmentKind::Statement);
    parse_single_fails("1 + 2", FragmentKind::Item);
    parse_single_fails("let x = 1;", FragmentKind::Item);
    parse_single_fails("123 foo", FragmentKind::Literal);
    parse_single_fails("1 + 2", FragmentKind::Ident);
    parse_single_fails("a.b.c", FragmentKind::Ident);
}

#[test]
fn parse_fragment_rolls_back_on_failure_and_sequences_statements() {
    let src = "let a = 1; fn helper() -> i32 { 2 } let b = 3;";
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("fragments.fl".into()), src.into());
    let (tokens, symbols, errs) = lex::lex(src, file.start_pos);
    assert!(errs.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);

    let m1 = parser
        .parse_fragment(FragmentKind::ExtendedStatement)
        .unwrap()
        .unwrap();
    let failed_stmt = parser.parse_fragment(FragmentKind::Statement);
    assert!(failed_stmt.is_err());

    // Parser cursor must still sit right at `fn helper() -> i32 { 2 }` after rollback!
    assert_eq!(parser.peek_next_token().kind, TokenKind::Fn);

    let m2 = parser
        .parse_fragment(FragmentKind::ExtendedStatement)
        .unwrap()
        .unwrap();
    let m3 = parser
        .parse_fragment(FragmentKind::Statement)
        .unwrap()
        .unwrap();
    assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);

    let ast = parser.finalize();
    assert_eq!(ast.get_node_kind(m1.root), Some(NodeKind::LetDecl));
    assert_eq!(ast.get_node_kind(m2.root), Some(NodeKind::Function));
    assert_eq!(ast.get_node_kind(m3.root), Some(NodeKind::LetDecl));
    ast.validate_structure().unwrap();
}
