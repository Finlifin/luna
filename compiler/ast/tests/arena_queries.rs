use ast::{Ast, NodeBuilder, NodeKind};
use rustc_span::{BytePos, Span};

#[test]
fn invalid_node_handles_do_not_panic_or_alias_valid_nodes() {
    let mut ast = Ast::new(BytePos(0));
    let span = Span::new(BytePos(2), BytePos(6));
    let leaf = NodeBuilder::new(NodeKind::Bool, span).build(&mut ast);
    let parent = NodeBuilder::new(NodeKind::BoolNot, span)
        .add_single_child(leaf)
        .build(&mut ast);
    for invalid in [0, ast.nodes.len() as u32, u32::MAX] {
        assert!(ast.get_node_kind(invalid).is_none());
        assert!(ast.get_node(invalid).is_none());
        assert!(ast.get_span(invalid).is_none());
        assert!(ast.get_children(invalid).is_empty());
    }
    assert_eq!(ast.get_node_kind(leaf), Some(NodeKind::Bool));
    assert_eq!(ast.get_span(parent), Some(span));
    assert_eq!(ast.get_node(parent).unwrap().2[0], leaf);
}

#[test]
fn child_lists_check_both_handle_and_declared_extent() {
    let mut ast = Ast::new(BytePos(0));
    let empty = NodeBuilder::new(NodeKind::Block, Span::default())
        .add_multiple_children(vec![])
        .build(&mut ast);
    let empty_list = ast.get_children(empty)[0];
    assert_eq!(ast.get_multi_child_slice(empty_list), Some(&[][..]));
    let block = NodeBuilder::new(NodeKind::Block, Span::default())
        .add_multiple_children(vec![empty, empty])
        .build(&mut ast);
    let list = ast.get_children(block)[0];
    assert_eq!(ast.get_multi_child_slice(list), Some(&[empty, empty][..]));
    for invalid in [0, ast.children.len() as u32, u32::MAX] {
        assert!(ast.get_multi_child_slice(invalid).is_none());
    }
    ast.children[list as usize] = u32::MAX;
    assert!(ast.get_multi_child_slice(list).is_none());
    ast.children[list as usize] = 2;
    ast.children.truncate(list as usize + 2);
    assert!(ast.get_multi_child_slice(list).is_none());
}

#[test]
fn incomplete_public_arena_metadata_is_rejected() {
    let mut ast = Ast::new(BytePos(0));
    let node = NodeBuilder::new(NodeKind::Bool, Span::default()).build(&mut ast);
    ast.children_start[node as usize] = u32::MAX;
    assert!(ast.get_children(node).is_empty());
    assert!(ast.get_node(node).is_none());
    ast.children_start.pop();
    assert!(ast.get_children(node).is_empty());
    assert!(ast.get_node(node).is_none());
    ast.spans.pop();
    assert!(ast.get_span(node).is_none());
}

#[test]
fn node_children_remain_local_after_later_allocations_and_clone() {
    let mut ast = Ast::new(BytePos(0));
    let leaf = NodeBuilder::new(NodeKind::Bool, Span::default()).build(&mut ast);
    let unary = NodeBuilder::new(NodeKind::BoolNot, Span::default())
        .add_single_child(leaf)
        .build(&mut ast);
    let block = NodeBuilder::new(NodeKind::Block, Span::default())
        .add_multiple_children(vec![unary])
        .build(&mut ast);
    let list = ast.get_children(block)[0];
    for _ in 0..20 {
        NodeBuilder::new(NodeKind::Add, Span::default())
            .add_single_child(leaf)
            .add_single_child(unary)
            .build(&mut ast);
        NodeBuilder::new(NodeKind::Block, Span::default())
            .add_multiple_children(vec![block, unary, leaf])
            .build(&mut ast);
        assert!(ast.get_children(leaf).is_empty());
        assert_eq!(ast.get_children(unary), &[leaf]);
        assert_eq!(ast.get_children(block), &[list]);
        assert_eq!(ast.get_multi_child_slice(list), Some(&[unary][..]));
    }
    let mut copied = ast.clone();
    copied.children_len[unary as usize] = u32::MAX;
    assert!(copied.get_node(unary).is_none());
    assert!(copied.get_children(unary).is_empty());
    assert_eq!(ast.get_children(unary), &[leaf]);
}

#[test]
fn layout_errors_identify_incomplete_predicates_before_traversal() {
    let mut ast = Ast::new(BytePos(0));
    let predicate = NodeBuilder::new(NodeKind::Bool, Span::default()).build(&mut ast);
    let malformed = NodeBuilder::new(NodeKind::Assumes, Span::default())
        .add_single_child(predicate)
        .build(&mut ast);
    // Later allocations must not make the missing label/predicate slot valid.
    NodeBuilder::new(NodeKind::BoolNot, Span::default())
        .add_single_child(predicate)
        .build(&mut ast);
    let error = ast.validate_node_layouts().unwrap_err();
    assert_eq!(error.node, malformed);
    assert_eq!(error.expected, 2);
    assert_eq!(error.actual, Some(1));
    assert_eq!(
        error.to_string(),
        format!("(ast-layout-error (node {malformed}) (expected-slots 2) (actual-slots 1))")
    );
}

#[test]
fn layout_check_accepts_raw_function_flags_and_empty_parameter_lists() {
    let mut ast = Ast::new(BytePos(0));
    NodeBuilder::new(NodeKind::FnType, Span::default())
        .add_single_child(u32::MAX)
        .add_multiple_children(vec![])
        .build(&mut ast);
    ast.validate_node_layouts().unwrap();
}

#[test]
fn leaf_token_payload_is_distinct_from_traversal_children() {
    let mut ast = Ast::new(BytePos(0));
    let id = NodeBuilder::new(NodeKind::Id, Span::default())
        .add_single_child(u32::MAX)
        .add_single_child(u32::MAX)
        .build(&mut ast);
    NodeBuilder::new(NodeKind::Symbol, Span::default())
        .add_single_child(id)
        .build(&mut ast);
    ast.validate_node_layouts().unwrap();
    assert_eq!(NodeKind::Id.node_type(), ast::NodeType::NoChild);
    assert_eq!(NodeKind::Symbol.node_type(), ast::NodeType::NoChild);
}

#[test]
fn structure_errors_locate_direct_and_list_references() {
    for target in [2, u32::MAX] {
        let mut ast = Ast::new(BytePos(0));
        let node = NodeBuilder::new(NodeKind::BoolNot, Span::default())
            .add_single_child(target)
            .build(&mut ast);
        let error = ast.validate_structure().unwrap_err();
        assert_eq!(error.node(), node);
        assert_eq!(
            error.to_string(),
            format!(
                "(ast-reference-error (node {node}) (slot 0) (element none) (target {target}) (kind node))"
            )
        );
    }
    for target in [0, u32::MAX] {
        let mut ast = Ast::new(BytePos(0));
        let node = NodeBuilder::new(NodeKind::Block, Span::default())
            .add_multiple_children(vec![target])
            .build(&mut ast);
        let error = ast.validate_structure().unwrap_err();
        assert_eq!(error.node(), node);
        assert!(error.to_string().contains("(element 0)"));
    }
}

#[test]
fn structure_checks_lists_but_not_raw_payload_or_optional_absence() {
    let mut ast = Ast::new(BytePos(0));
    NodeBuilder::new(NodeKind::Id, Span::default())
        .add_single_child(u32::MAX)
        .add_single_child(u32::MAX)
        .build(&mut ast);
    NodeBuilder::new(NodeKind::FnType, Span::default())
        .add_single_child(u32::MAX)
        .add_multiple_children(vec![])
        .build(&mut ast);
    NodeBuilder::new(NodeKind::ReturnStatement, Span::default())
        .add_single_child(0)
        .add_single_child(0)
        .build(&mut ast);
    ast.validate_structure().unwrap();
    let node = NodeBuilder::new(NodeKind::Block, Span::default())
        .add_multiple_children(vec![])
        .build(&mut ast);
    let list = ast.get_children(node)[0];
    ast.children[list as usize] = u32::MAX;
    let error = ast.validate_structure().unwrap_err();
    assert_eq!(error.node(), node);
    assert!(error.to_string().contains("(kind list)"));
}
