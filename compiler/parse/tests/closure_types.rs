use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse(source: &str) -> Result<(Ast, NodeIndex, TokenKind), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("closures.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty());
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let node = parser.try_expr().map_err(|e| format!("{e:?}"))?;
    let next = parser.peek_next_token().kind;
    Ok((parser.finalize(), node, next))
}

#[test]
fn environment_bindings_preserve_types_sources_order_and_body() {
    let (ast, root, next) =
        parse("%{first: A = value, second: fn(A) -> B,} comptime fn(B) -> A").unwrap();
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(
        ast.get_node_kind(root),
        Some(NodeKind::ClosureQualifiedType)
    );
    let children = ast.get_children(root);
    assert_eq!(
        ast.get_node_kind(children[0]),
        Some(NodeKind::ClosureEnvironment)
    );
    assert_eq!(ast.get_node_kind(children[1]), Some(NodeKind::Arrow));
    let bindings = ast
        .get_multi_child_slice(ast.get_children(children[0])[0])
        .unwrap();
    assert_eq!(bindings.len(), 2);
    for &binding in bindings {
        assert_eq!(
            ast.get_node_kind(binding),
            Some(NodeKind::ClosureEnvironmentBinding)
        );
    }
    assert_ne!(ast.get_children(bindings[0])[2], 0);
    assert_eq!(ast.get_children(bindings[1])[2], 0);
}

#[test]
fn environment_prefix_keeps_percent_infix_and_accepts_whitespace() {
    for source in [
        "%{} T",
        "%C fn() -> T",
        " % C fn() -> T",
        "%{inner: %{x: A} fn() -> A} fn() -> A",
    ] {
        let (ast, root, next) = parse(source).unwrap();
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(
            ast.get_node_kind(root),
            Some(NodeKind::ClosureQualifiedType)
        );
    }
    let (ast, root, next) = parse("left % right").unwrap();
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::Mod));
}

#[test]
fn malformed_environment_bindings_and_missing_bodies_are_rejected() {
    for source in [
        "%",
        "%{}",
        "%{x} T",
        "%{x:} T",
        "%{x:A =} T",
        "%{x:A",
        "%{x:A,,} T",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    let old = parse("^C fn() -> T");
    assert!(old.is_err() || old.unwrap().1 == 0);
}

#[test]
fn closure_qualification_contains_arrows_but_not_outer_type_propositions() {
    let (ast, root, next) = parse("%{x:A} fn(A) -> A == %{renamed:A} fn(A) -> A").unwrap();
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::BoolEq));
    for &side in ast.get_children(root) {
        assert_eq!(
            ast.get_node_kind(side),
            Some(NodeKind::ClosureQualifiedType)
        );
        assert_eq!(
            ast.get_node_kind(ast.get_children(side)[1]),
            Some(NodeKind::Arrow)
        );
    }
}

#[test]
fn closure_parameter_types_preserve_the_lambda_pipe_boundary() {
    for source in [
        "|callback:%{flag:bool} fn(*bool)->bool|callback(pointer)",
        "|callback:%{flag:bool} fn(*bool)->(A | B)|callback(pointer)",
        "|callback:%{inner:%{flag:bool} fn()->bool} fn()->bool|callback()",
    ] {
        let (ast, root, next) = parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::Lambda), "{source}");
    }
}

#[test]
fn qualified_lambda_parameters_leave_the_body_outside_the_type() {
    for (ty, kind) in [
        ("#IO bool", NodeKind::EffectQualifiedType),
        ("!Failure bool", NodeKind::ErrorQualifiedType),
        ("!Failure ?bool", NodeKind::ErrorQualifiedType),
        ("#IO ?bool", NodeKind::EffectQualifiedType),
        ("&Reach ?bool", NodeKind::ReachabilityQualifiedType),
        ("&Reach bool", NodeKind::ReachabilityQualifiedType),
        ("#IO !Failure bool", NodeKind::EffectQualifiedType),
        (
            "!Failure %{flag:bool} fn()->bool",
            NodeKind::ErrorQualifiedType,
        ),
        (
            "&Reach %{flag:bool} fn()->bool",
            NodeKind::ReachabilityQualifiedType,
        ),
        ("!Failure #IO bool", NodeKind::ErrorQualifiedType),
        (
            "#IO &Reach %{flag:bool} fn()->bool",
            NodeKind::EffectQualifiedType,
        ),
        (
            "%Context !Failure fn()->bool",
            NodeKind::ClosureQualifiedType,
        ),
    ] {
        let source = format!("|value:{ty}|value");
        let (ast, root, next) = parse(&source).unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::Lambda), "{source}");
        let children = ast.get_children(root);
        let parameters = ast.get_multi_child_slice(children[2]).unwrap();
        assert_eq!(parameters.len(), 1, "{source}");
        assert_eq!(
            ast.get_node_kind(ast.get_children(parameters[0])[1]),
            Some(kind),
            "{source}"
        );
        assert_eq!(
            ast.get_node_kind(children[1]),
            Some(NodeKind::Id),
            "{source}"
        );
    }
}

#[test]
fn qualifier_context_parentheses_keep_inner_operators_and_cmtt_boundaries() {
    for (source, outer, context) in [
        (
            "&(Left | Right) bool",
            NodeKind::ReachabilityQualifiedType,
            NodeKind::Pipe,
        ),
        (
            "#(Left % Right) bool",
            NodeKind::EffectQualifiedType,
            NodeKind::Mod,
        ),
    ] {
        let (ast, root, next) = parse(source).unwrap();
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(ast.get_node_kind(root), Some(outer), "{source}");
        assert_eq!(
            ast.get_node_kind(ast.get_children(root)[0]),
            Some(context),
            "{source}"
        );
    }
    for source in [
        "<value: !Failure bool |- #IO bool >",
        "<value: &Reach bool |- %{flag:bool} fn()->bool >",
    ] {
        let (ast, root, next) = parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(next, TokenKind::Eof, "{source}");
        assert_eq!(
            ast.get_node_kind(root),
            Some(NodeKind::ContextualExprType),
            "{source}"
        );
    }
}

#[test]
fn qualifier_boundaries_do_not_supply_missing_operands() {
    for source in [
        "|value:#IO|value",
        "|value:!Failure|value",
        "|value:&Reach|value",
        "|value:#IO !Failure|value",
        "<value:bool |- #IO >",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
}

#[test]
fn qualified_types_leave_comparisons_and_bounds_outside_their_body() {
    for (qualified, kind) in [
        ("#IO fn()->T", NodeKind::EffectQualifiedType),
        ("!Failure fn()->T", NodeKind::ErrorQualifiedType),
        ("&Reach fn()->T", NodeKind::ReachabilityQualifiedType),
        ("%Context fn()->T", NodeKind::ClosureQualifiedType),
        ("#IO !Failure fn()->T", NodeKind::EffectQualifiedType),
    ] {
        for (operator, outer) in [
            ("==", NodeKind::BoolEq),
            ("!=", NodeKind::BoolNotEq),
            (":-", NodeKind::TraitBound),
        ] {
            let source = format!("{qualified} {operator} Other");
            let (ast, root, next) = parse(&source).unwrap();
            assert_eq!(next, TokenKind::Eof, "{source}");
            assert_eq!(ast.get_node_kind(root), Some(outer), "{source}");
            assert_eq!(
                ast.get_node_kind(ast.get_children(root)[0]),
                Some(kind),
                "{source}"
            );
        }
    }
    let (ast, root, next) = parse("#IO A == #IO B and !Failure C != !Failure D").unwrap();
    assert_eq!(next, TokenKind::Eof);
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::BoolAnd));
    let sides = ast.get_children(root);
    assert_eq!(ast.get_node_kind(sides[0]), Some(NodeKind::BoolEq));
    assert_eq!(ast.get_node_kind(sides[1]), Some(NodeKind::BoolNotEq));
}

#[test]
fn optional_results_are_not_consumed_as_row_propagation() {
    for source in ["!Failure ?T", "#IO ?T", "&Reach ?T"] {
        let (ast, root, next) = parse(source).unwrap();
        assert_eq!(next, TokenKind::Eof, "{source}");
        let children = ast.get_children(root);
        assert_eq!(
            ast.get_node_kind(children[0]),
            Some(NodeKind::Id),
            "{source}"
        );
        assert_eq!(
            ast.get_node_kind(children[1]),
            Some(NodeKind::OptionalType),
            "{source}"
        );
    }
    let (ast, root, next) = parse("!Failure ??T").unwrap();
    assert_eq!(next, TokenKind::Eof);
    let optional = ast.get_children(root)[1];
    assert_eq!(ast.get_node_kind(optional), Some(NodeKind::OptionalType));
    assert_eq!(
        ast.get_node_kind(ast.get_children(optional)[0]),
        Some(NodeKind::OptionalType)
    );
}
