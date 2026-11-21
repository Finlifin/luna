use ast::{Ast, NodeIndex, NodeKind};
use diagnostic::DiagnosticContext;
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse_expr_ast(source: &str) -> (SourceMap, Ast, NodeIndex) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("token_buffer.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{source}: {errors:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_expr()
        .unwrap_or_else(|error| panic!("{source}: {error:?}"));
    assert_ne!(root, 0);
    assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
    let ast = parser.finalize();
    ast.validate_structure().unwrap();
    (map, ast, root)
}

fn parse_file_ast(source: &str) -> (SourceMap, Ast) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(
        FileName::Custom("token_buffer_file.fl".into()),
        source.into(),
    );
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{source}: {errors:?}");
    let diag = DiagnosticContext::new(&map);
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    assert!(parser.parse(&diag), "{source}: errors encountered");
    let ast = parser.finalize();
    ast.validate_structure().unwrap();
    (map, ast)
}

fn parse_expr_err(source: &str) -> String {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(
        FileName::Custom("token_buffer_err.fl".into()),
        source.into(),
    );
    let (tokens, symbols, _) = lex::lex(source, file.start_pos);
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let err = parser.try_expr().unwrap_err();
    err.to_string()
}

#[test]
fn empty_and_simple_token_buffers_parse_and_dump_cleanly() {
    let (_map, ast, root) = parse_expr_ast("{{}}");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::TokenBuffer));
    let parts = ast
        .get_multi_child_slice(ast.get_children(root)[0])
        .unwrap();
    assert!(parts.is_empty());

    let (map, ast, root) = parse_expr_ast("{{ let x = (1 + 2); }}");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::TokenBuffer));
    let parts = ast
        .get_multi_child_slice(ast.get_children(root)[0])
        .unwrap();
    assert_eq!(parts.len(), 9);
    for &part in parts {
        assert_eq!(ast.get_node_kind(part), Some(NodeKind::TokenBufferToken));
    }
    let sexp = ast.dump_to_s_expression(root, &map);
    assert!(sexp.contains("(TokenBufferToken \"(\")"), "{sexp}");
    assert!(sexp.contains("(TokenBufferToken \")\")"), "{sexp}");
}

#[test]
fn interpolations_and_escaped_dollars_parse_with_exact_children() {
    let (_map, ast, root) = parse_expr_ast("{{ $id $self ${a + b} $(c.foo()) $$ }}");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::TokenBuffer));
    let parts = ast
        .get_multi_child_slice(ast.get_children(root)[0])
        .unwrap();
    assert_eq!(parts.len(), 5);

    assert_eq!(
        ast.get_node_kind(parts[0]),
        Some(NodeKind::TokenBufferInterp)
    );
    assert_eq!(
        ast.get_node_kind(ast.get_children(parts[0])[0]),
        Some(NodeKind::Id)
    );

    assert_eq!(
        ast.get_node_kind(parts[1]),
        Some(NodeKind::TokenBufferInterp)
    );
    assert_eq!(
        ast.get_node_kind(ast.get_children(parts[1])[0]),
        Some(NodeKind::SelfLower)
    );

    assert_eq!(
        ast.get_node_kind(parts[2]),
        Some(NodeKind::TokenBufferInterp)
    );
    assert_eq!(
        ast.get_node_kind(ast.get_children(parts[2])[0]),
        Some(NodeKind::Add)
    );

    assert_eq!(
        ast.get_node_kind(parts[3]),
        Some(NodeKind::TokenBufferInterp)
    );
    assert_eq!(
        ast.get_node_kind(ast.get_children(parts[3])[0]),
        Some(NodeKind::Application)
    );

    assert_eq!(
        ast.get_node_kind(parts[4]),
        Some(NodeKind::TokenBufferDollar)
    );
}

#[test]
fn balanced_inner_braces_and_nested_token_buffers_parse() {
    let (map, ast, root) = parse_expr_ast("{{ if a { 1 } else { 2 }}}");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::TokenBuffer));
    let parts = ast
        .get_multi_child_slice(ast.get_children(root)[0])
        .unwrap();
    assert_eq!(parts.len(), 9);
    assert_eq!(ast.source_content(parts[8], &map).as_deref(), Some("}"));

    let (_map, ast, root) = parse_expr_ast("{{ let inner = {{ 1 + 2 }}; inner }}");
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::TokenBuffer));
    let parts = ast
        .get_multi_child_slice(ast.get_children(root)[0])
        .unwrap();
    assert_eq!(parts.len(), 12);
}

#[test]
fn token_buffer_works_as_final_block_expression_and_does_not_break_match_struct_pattern() {
    let (_map, ast) = parse_file_ast(
        r#"
        fn emit(acc: i32, p: i32) {
            {{ $acc or $p }}
        }

        fn m() -> bool {
            let s = { flag: true };
            s match {{ flag: true } => true, }
        }
        "#,
    );
    let top_items = ast
        .get_multi_child_slice(ast.get_children(ast.root)[0])
        .unwrap();
    assert_eq!(top_items.len(), 2);
}

#[test]
fn unterminated_and_invalid_dollar_in_token_buffer_report_clear_errors() {
    let err_unclosed = parse_expr_err("{{ let x = 1;");
    assert!(
        err_unclosed.contains("unterminated token buffer"),
        "{err_unclosed}"
    );

    let err_dollar = parse_expr_err("{{ $ 1 }}");
    assert!(
        err_dollar.contains("invalid `$` in token buffer"),
        "{err_dollar}"
    );

    let err_empty_interp = parse_expr_err("{{ ${} }}");
    assert!(
        err_empty_interp.contains("expected an expression inside `${...}`"),
        "{err_empty_interp}"
    );
}
