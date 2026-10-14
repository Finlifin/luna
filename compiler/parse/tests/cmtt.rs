use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{source_map::FilePathMapping, FileName, SourceMap};

fn parse(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map=SourceMap::new(FilePathMapping::empty());
    let file=map.new_source_file(FileName::Custom("cmtt.fl".into()),source.into());
    let (tokens,symbols,errors)=lex::lex(source,file.start_pos);
    assert!(errors.is_empty(),"{errors:?}");
    let mut parser=Parser::new(&map,tokens,symbols,file.start_pos);
    let root=parser.try_expr().map_err(|e|format!("{e:?}"))?;
    if root==0 || parser.peek_next_token().kind!=TokenKind::Eof {return Err("incomplete expression".into());}
    Ok((parser.finalize(),root))
}

#[test]
fn quote_splice_and_contextual_type_keep_delimiters_and_order() {
    for (source,kind,count) in [
        ("'[first=a,second=b](a and b)",NodeKind::QuoteExpr,2),
        ("~[first=b,second=a]<code>",NodeKind::SpliceExpr,2),
        ("<first:bool,second:bool |- bool>",NodeKind::ContextualExprType,2),
        ("< |- bool >",NodeKind::ContextualExprType,0),
    ] {
        let (ast,root)=parse(source).unwrap();
        assert_eq!(ast.get_node_kind(root),Some(kind));
        assert_eq!(ast.get_multi_child_slice(ast.get_children(root)[1]).unwrap().len(),count);
        assert!(ast.get_node(ast.get_children(root)[0]).is_some());
    }
}

#[test]
fn a_splice_operand_starts_a_separate_comptime_construction() {
    parse("'(not ~<'(not ~<'(true)>)>)").unwrap();
    parse("do {~<'(true)>;~<'(false)>}").unwrap();
    for source in [
        "'(~<left> and ~<right>)",
        "'(if true {~<left>} else {~<right>})",
        "'(do {let callback=|| ~<left>;~<right>})",
    ] {
        assert!(parse(source).unwrap_err().contains("at most one splice"),"{source}");
    }
}

#[test]
fn malformed_interfaces_and_substitution_expressions_are_rejected() {
    for source in ["'()","'(true","~<>","~<code","~[x=f()]<code>",
        "'[x=](true)","<x: |- bool>","<x:bool bool>","< |- >"] {
        assert!(parse(source).is_err(),"{source}");
    }
    // Character literals remain characters, rather than code delimiters.
    let (ast,root)=parse("'('").unwrap();
    assert_eq!(ast.get_node_kind(root),Some(NodeKind::Char));
}
