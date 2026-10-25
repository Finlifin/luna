use ast::{
    NodeKind, NodeType,
    ast_visitor::{AstVisitor, SExpressionVisitor},
};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};

#[test]
fn atomic_body_and_resource_list_share_the_declared_layout() {
    for (source, names) in [
        ("atomic() {true}", vec![]),
        ("atomic(first) {false}", vec!["first"]),
        ("atomic(first, second) {true}", vec!["first", "second"]),
    ] {
        let map = SourceMap::new(FilePathMapping::empty());
        let file = map.new_source_file(FileName::Custom("atomic.fl".into()), source.into());
        let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
        assert!(errors.is_empty());
        let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser.try_expr().unwrap();
        assert_eq!(parser.peek_next_token().kind, TokenKind::Eof);
        let ast = parser.finalize();
        ast.validate_node_layouts().unwrap();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::AtomicBlock));
        assert_eq!(
            NodeKind::AtomicBlock.node_type(),
            NodeType::SingleWithMultiChildren
        );
        let slots = ast.get_children(root);
        assert_eq!(slots.len(), 2);
        assert_eq!(ast.get_node_kind(slots[0]), Some(NodeKind::Block));
        let resources = ast.get_multi_child_slice(slots[1]).unwrap();
        assert_eq!(resources.len(), names.len());
        for (&node, name) in resources.iter().zip(names) {
            assert_eq!(ast.get_node_kind(node), Some(NodeKind::Id));
            assert_eq!(ast.source_content(node, &map).as_deref(), Some(name));
        }
        // Both traversal implementations must accept the same layout.
        let dump = ast.dump_to_s_expression(root, &map);
        let visited = SExpressionVisitor.visit_node(&ast, root, &map);
        for rendered in [dump, visited] {
            assert!(rendered.contains("AtomicBlock"), "{rendered}");
            assert!(rendered.contains("Block"), "{rendered}");
            assert!(rendered.contains("Bool"), "{rendered}");
        }
    }
}
