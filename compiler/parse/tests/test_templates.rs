use ast::{Ast, NodeIndex, NodeKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn parse(source: &str) -> Result<(Ast, NodeIndex), String> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("test-templates.fl".into()), source.into());
    let (tokens, symbols, errors) = lex::lex(source, file.start_pos);
    assert!(errors.is_empty(), "{errors:?}");
    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let root = parser
        .try_statement_or_definition()
        .map_err(|e| format!("{e:?}"))?;
    parser.eat_token(TokenKind::Semi);
    if root == 0 || parser.peek_next_token().kind != TokenKind::Eof {
        return Err("incomplete declaration".into());
    }
    Ok((parser.finalize(), root))
}

#[test]
fn function_templates_preserve_signatures_modifiers_and_optional_bodies() {
    for modifier in ["", "quote ", "inline ", "pure ", "verified "] {
        for body in [";", " {value}", " = value;"] {
            let (ast, root) = parse(&format!(
                "template {modifier}fn unary(value:bool)->bool{body}"
            ))
            .unwrap();
            assert_eq!(ast.get_node_kind(root), Some(NodeKind::FunctionTemplateDef));
            let mut declaration = ast.get_children(root)[0];
            if !modifier.is_empty() {
                assert_eq!(
                    ast.get_node_kind(declaration),
                    Some(NodeKind::AttributeSetTrue)
                );
                declaration = ast.get_children(declaration)[1];
            }
            assert_eq!(ast.get_node_kind(declaration), Some(NodeKind::Function));
            let fields = ast.get_children(declaration);
            assert_eq!(ast.get_node_kind(fields[0]), Some(NodeKind::Id));
            assert_eq!(ast.get_multi_child_slice(fields[1]).unwrap().len(), 1);
            assert_ne!(fields[2], 0);
            assert_eq!(fields[4] == 0, body == ";");
        }
    }
    let (ast, root) = parse("template fn with_current(value:bool, quote lambda body:< current:bool |- bool >)->bool {let current=value;~<body>}").unwrap();
    let declaration = ast.get_children(root)[0];
    assert_eq!(
        ast.get_multi_child_slice(ast.get_children(declaration)[1])
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn function_template_prefix_requires_a_named_function_declaration() {
    for source in [
        "template",
        "template struct Bad {}",
        "template let value=true;",
        "template pure struct Bad {}",
        "template fn () {}",
        "template fn missing",
        "template template fn nested() {}",
    ] {
        assert!(parse(source).is_err(), "accepted {source}");
    }
}

#[test]
fn function_instances_preserve_inherited_and_explicit_parameter_lists() {
    for template in ["unary", "rules.unary", "rules.unary<bool>"] {
        for params in [None, Some(""), Some("value:bool")] {
            let input = params
                .map(|params| format!("({params})"))
                .unwrap_or_default();
            let modifier = if params.is_some() { "quote " } else { "" };
            let (ast, mut root) = parse(&format!(
                "{modifier}fn apply{input} use {template} {{true}}"
            ))
            .unwrap();
            if params.is_some() {
                assert_eq!(ast.get_node_kind(root), Some(NodeKind::AttributeSetTrue));
                root = ast.get_children(root)[1];
            }
            let fields = ast.get_children(root);
            assert_eq!(ast.get_node_kind(fields[0]), Some(NodeKind::Id));
            if let Some(params) = params {
                assert_eq!(
                    ast.get_node_kind(root),
                    Some(NodeKind::FunctionTemplateInstanceWithParams)
                );
                assert_eq!(
                    ast.get_multi_child_slice(fields[1]).unwrap().len(),
                    usize::from(!params.is_empty())
                );
                assert_ne!(fields[2], 0);
                assert_eq!(ast.get_node_kind(fields[4]), Some(NodeKind::Block));
            } else {
                assert_eq!(
                    ast.get_node_kind(root),
                    Some(NodeKind::FunctionTemplateInstance)
                );
                assert_ne!(fields[1], 0);
                assert_eq!(ast.get_node_kind(fields[2]), Some(NodeKind::Block));
            }
        }
    }
    for modifier in ["quote", "inline", "pure"] {
        let (ast, root) = parse(&format!("{modifier} fn apply use unary {{true}}")).unwrap();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::AttributeSetTrue));
        assert_eq!(
            ast.get_node_kind(ast.get_children(root)[1]),
            Some(NodeKind::FunctionTemplateInstance)
        );
    }
    for source in [
        "fn use unary {}",
        "fn named use {}",
        "fn named use unary",
        "fn named() use unary;",
        "fn named() use unary = true;",
        "fn named use unary {} trailing",
    ] {
        assert!(parse(source).is_err(), "accepted {source}");
    }
}

#[test]
fn contextual_test_template_preserves_clauses_and_body() {
    let (ast, root) = parse("template test with_counter where body: < counter: *u32 |- void > { let value: u32 = 0; let counter = value.ref; ~<body>; }").unwrap();
    assert_eq!(ast.get_node_kind(root), Some(NodeKind::TestTemplateDef));
    let children = ast.get_children(root);
    assert_eq!(ast.get_node_kind(children[0]), Some(NodeKind::Id));
    let clauses = ast.get_multi_child_slice(children[1]).unwrap();
    assert_eq!(clauses.len(), 1);
    assert_eq!(ast.get_node_kind(children[2]), Some(NodeKind::Block));
    assert_eq!(
        ast.get_multi_child_slice(ast.get_children(children[2])[0])
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn template_instances_keep_application_separate_from_runtime_body() {
    for template in [
        "with_counter",
        "fixtures.with_counter",
        "fixtures.with_counter<true>",
    ] {
        let (ast, root) = parse(&format!(
            "test increment use {template} {{ counter.* = 1; }}"
        ))
        .unwrap();
        assert_eq!(
            ast.get_node_kind(root),
            Some(NodeKind::TestTemplateInstance)
        );
        let children = ast.get_children(root);
        assert_ne!(children[1], 0);
        assert_eq!(ast.get_node_kind(children[2]), Some(NodeKind::Block));
    }
}

#[test]
fn ordinary_tests_and_empty_instance_bodies_remain_distinct() {
    for source in ["test {}", "test ordinary {}"] {
        let (ast, root) = parse(source).unwrap();
        assert_eq!(ast.get_node_kind(root), Some(NodeKind::TestDef));
    }
    let (ast, root) = parse("test empty use fixture {}").unwrap();
    assert_eq!(
        ast.get_node_kind(root),
        Some(NodeKind::TestTemplateInstance)
    );
}

#[test]
fn malformed_template_declarations_and_instances_are_rejected() {
    for source in [
        "template test {}",
        "template test fixture",
        "template test fixture where body: < |- void >",
        "test use fixture {}",
        "test named use {}",
        "test named use fixture",
    ] {
        assert!(parse(source).is_err(), "accepted {source}");
    }
}
