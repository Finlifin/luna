use ast::NodeKind;
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

#[test]
fn defer_is_a_keyword_and_retains_the_complete_expression() {
    for (source,inner) in [
        ("defer Select(flag)",NodeKind::Application),
        ("defer es[0]",NodeKind::IndexApplication),
        ("defer left + right",NodeKind::Add),
        ("defer defer bool",NodeKind::Defer),
    ] {
        let map=SourceMap::new(FilePathMapping::empty());
        let file=map.new_source_file(FileName::Custom("defer.fl".into()),source.into());
        let (tokens,symbols,errors)=lex::lex(source,file.start_pos);
        assert!(errors.is_empty());
        assert_eq!(tokens[1].kind,TokenKind::Defer);
        let mut parser=Parser::new(&map,tokens,symbols,file.start_pos);
        let node=parser.try_expr().unwrap();
        assert_eq!(parser.peek_next_token().kind,TokenKind::Eof,"{source}");
        let ast=parser.finalize();
        assert_eq!(ast.get_node_kind(node),Some(NodeKind::Defer));
        assert_eq!(ast.get_node_kind(ast.get_children(node)[0]),Some(inner),"{source}");
    }
}

#[test]
fn defer_preserves_parenthesized_scope_and_requires_an_operand() {
    for source in ["defer","defer )","defer ,"] {
        let map=SourceMap::new(FilePathMapping::empty());
        let file=map.new_source_file(FileName::Custom("defer.fl".into()),source.into());
        let (tokens,symbols,_)=lex::lex(source,file.start_pos);
        let mut parser=Parser::new(&map,tokens,symbols,file.start_pos);
        assert!(parser.try_expr().is_err(),"{source}");
    }
    let source="(defer left) + right";
    let map=SourceMap::new(FilePathMapping::empty());
    let file=map.new_source_file(FileName::Custom("defer.fl".into()),source.into());
    let (tokens,symbols,_)=lex::lex(source,file.start_pos);
    let mut parser=Parser::new(&map,tokens,symbols,file.start_pos);
    let node=parser.try_expr().unwrap();
    assert_eq!(parser.peek_next_token().kind,TokenKind::Eof);
    let ast=parser.finalize();
    assert_eq!(ast.get_node_kind(node),Some(NodeKind::Add));
    // Parentheses may retain their own AST wrapper; the root must remain Add.
    let (tokens,_,errors)=lex::lex("deferred",rustc_span::BytePos(0));
    assert!(errors.is_empty());
    assert_eq!(tokens[1].kind,TokenKind::Id);
}
