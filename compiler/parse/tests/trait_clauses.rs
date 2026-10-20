use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn declaration(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("trait-clauses.fl".into()), source.into());
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
fn declaration_sugar_and_explicit_requires_keep_distinct_ast_forms() {
    for (source, kinds) in [
        (
            "fn next() where T :- A + B {}",
            vec![NodeKind::TraitBoundDeclClause],
        ),
        (
            "fn next() where T, requires T :- A + B {}",
            vec![NodeKind::TypeDeclClause, NodeKind::Requires],
        ),
    ] {
        let (ast, root) = declaration(source).unwrap();
        let clauses = ast
            .get_multi_child_slice(ast.get_children(root)[3])
            .unwrap();
        assert_eq!(
            clauses
                .iter()
                .map(|&node| ast.get_node_kind(node).unwrap())
                .collect::<Vec<_>>(),
            kinds
        );
    }
    declaration("trait IndexMut :- Index<Idx> where Idx {}").unwrap();
    declaration("impl Iterator for MapIterator where I :- Iterator, U {}").unwrap();
    declaration("impl Iterator for MapIterator where I, U, requires I :- Iterator {}").unwrap();
    declaration(
        "fn from_iter(iter: I) where I :- Iterator, requires I.as(Iterator).Item == T, T {}",
    )
    .unwrap();
}

#[test]
fn requires_is_not_a_bound_declaration_operator() {
    for source in [
        "fn next() where T requires Iterator {}",
        "impl Iterator for MapIterator where I requires Iterator, U {}",
        "trait Copy requires Clone {}",
    ] {
        assert!(declaration(source).is_err(), "accepted: {source}");
    }
}

#[test]
fn delimited_defaults_accept_constructors_without_consuming_definition_bodies() {
    for source in [
        "fn run(.config:Config=Config {flag:true}) {}",
        "fn run(.config:Config=Config {flag:true},.next:bool=false) {}",
        "pure comptime fn Packet<.enabled:bool=true,.config:Config<enabled> = Config<enabled>{flag:true}>->Type {bool}",
        "pure comptime fn Packet<.config:Config=Config {flag:true}>->Type = bool",
        "fn run() where .flag:bool=true {}",
        "pure comptime fn Packet<.flag:bool=true>->Type where .other:bool=false {bool}",
    ] {
        declaration(source).unwrap_or_else(|error| panic!("{source}: {error}"));
    }
    for source in [
        "fn run(.config:Config=Config {flag:}) {}",
        "pure comptime fn Packet<.config:Config=Config {flag:}>->Type {bool}",
        "pure comptime fn Packet<.config:Config=>->Type {bool}",
    ] {
        assert!(declaration(source).is_err(), "accepted: {source}");
    }
}
