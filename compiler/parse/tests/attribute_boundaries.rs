use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

fn parameter(source: &str, type_clause: bool) -> (Ast, NodeIndex, u32) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("attributes.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{errors:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = if type_clause {
        parser.try_type_param()
    } else {
        parser.try_param()
    }
    .unwrap_or_else(|error| panic!("{source}: {error:?}"));
    assert_ne!(root, 0);
    assert_eq!(parser.peek_next_token().kind, TokenKind::Eof, "{source}");
    (parser.finalize(), root, file.start_pos.0)
}

fn spelling<'a>(ast: &Ast, node: NodeIndex, source: &'a str, offset: u32) -> &'a str {
    let span = ast.get_span(node).unwrap();
    &source[(span.lo().0 - offset) as usize..(span.hi().0 - offset) as usize]
}

#[test]
fn parameter_attribute_chains_retain_source_order_complete_paths_and_target() {
    for type_clause in [false, true] {
        let source = "^attrs.outer(1).enabled ^attrs.inner(2) .value: bool = true";
        let (ast, mut node, offset) = parameter(source, type_clause);
        for expected in ["attrs.outer(1).enabled", "attrs.inner(2)"] {
            assert_eq!(ast.get_node_kind(node), Some(NodeKind::AttrParam));
            let children = ast.get_children(node);
            assert_eq!(spelling(&ast, children[0], source, offset), expected);
            node = children[1];
        }
        assert_eq!(ast.get_node_kind(node), Some(NodeKind::OptionalParam));
        assert_eq!(spelling(&ast, node, source, offset), ".value: bool = true");
    }
}

#[test]
fn nested_multi_attributes_do_not_steal_an_outer_prefix_or_sibling_argument() {
    let source = "^attrs.outer(^attrs.inner ^attrs.last true, false) ^attrs.next value: bool";
    let (ast, root, offset) = parameter(source, false);
    let children = ast.get_children(root);
    let application = children[0];
    assert_eq!(ast.get_node_kind(application), Some(NodeKind::Application));
    let inputs = ast
        .get_multi_child_slice(ast.get_children(application)[1])
        .unwrap();
    assert_eq!(inputs.len(), 2);
    let mut first = inputs[0];
    for expected in ["attrs.inner", "attrs.last"] {
        assert_eq!(ast.get_node_kind(first), Some(NodeKind::Attribute));
        let children = ast.get_children(first);
        assert_eq!(spelling(&ast, children[0], source, offset), expected);
        first = children[1];
    }
    assert_eq!(spelling(&ast, first, source, offset), "true");
    assert_eq!(spelling(&ast, inputs[1], source, offset), "false");
    let next = children[1];
    assert_eq!(ast.get_node_kind(next), Some(NodeKind::AttrParam));
    let children = ast.get_children(next);
    assert_eq!(spelling(&ast, children[0], source, offset), "attrs.next");
    assert_eq!(spelling(&ast, children[1], source, offset), "value: bool");
}
