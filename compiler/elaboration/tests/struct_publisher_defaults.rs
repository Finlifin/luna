use comptime_interpretor::{Budget, Interpreter, ReflectedField, StructBuildRequest, Value};
use elaboration::definitions::{StructPublisher, publish};
use hir::{
    definitions::{DefinitionBatch, DefinitionNodes, assemble_function},
    *,
};
use nir::{Primitive, Term, program::SourceRange};
use rustc_span::DUMMY_SP;
use std::{collections::BTreeMap, sync::Arc};

fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn request(label: &str) -> StructBuildRequest {
    StructBuildRequest {
        name: label.into(),
        fields: vec![ReflectedField {
            name: "value".into(),
            ty: boolean(),
        }],
        source: SourceRange { start: 10, end: 20 },
    }
}
fn publisher(arena: &HirArena, fuel: usize) -> StructPublisher<'_> {
    StructPublisher::new(
        arena,
        Package::new(),
        7,
        BTreeMap::from([(7, 879)]),
        Default::default(),
        None,
        fuel,
    )
    .unwrap()
}
fn quoted_default<'a>(
    arena: &'a HirArena,
    field: HirId,
    nodes: &mut DefinitionNodes<'a>,
    fuel: &mut usize,
    valid: bool,
) -> Result<Option<&'a Expr<'a>>, definitions::Error> {
    let root = nodes.reserve(field, fuel)?;
    let quote_id = nodes.reserve(root, fuel)?;
    let literal = arena.alloc_expr(Expr {
        hir_id: nodes.reserve(quote_id, fuel)?,
        span: DUMMY_SP,
        kind: ExprKind::Lit(Lit {
            span: DUMMY_SP,
            kind: if valid {
                LitKind::Bool(true)
            } else {
                LitKind::Char('x')
            },
        }),
    });
    nodes.register(Node::Expr(literal), fuel)?;
    let quote = arena.alloc_expr(Expr {
        hir_id: quote_id,
        span: DUMMY_SP,
        kind: ExprKind::CodeQuote(literal, vec![]),
    });
    nodes.register(Node::Expr(quote), fuel)?;
    Ok(Some(arena.alloc_expr(Expr {
        hir_id: root,
        span: DUMMY_SP,
        kind: ExprKind::CodeSplice(quote, vec![]),
    })))
}

// Consume the retained HIR, not a manufactured NIR record. Ordinary constructor
// elaboration must find, check and instantiate the published field default.
fn execute_default<'a>(arena: &'a HirArena, mut package: Package<'a>, subject: Term) -> Value {
    let mut fuel = 100_000;
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let identity = batch.reserve_identity(7, &mut fuel).unwrap();
    let function = assemble_function(
        arena,
        owner,
        name("run"),
        DUMMY_SP,
        &mut fuel,
        |nodes, fuel| {
            let root = nodes.root();
            let result = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(root, fuel)?,
                span: DUMMY_SP,
                kind: ExprKind::Builtin(Symbol::intern("bool")),
            });
            nodes.register(Node::Expr(result), fuel)?;
            let body_id = nodes.reserve(root, fuel)?;
            let construction_id = nodes.reserve(body_id, fuel)?;
            let ty = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(construction_id, fuel)?,
                span: DUMMY_SP,
                kind: ExprKind::DeliveredType(Arc::new(subject)),
            });
            nodes.register(Node::Expr(ty), fuel)?;
            let construction = arena.alloc_expr(Expr {
                hir_id: construction_id,
                span: DUMMY_SP,
                kind: ExprKind::ExtendedApplication(ty, &[]),
            });
            nodes.register(Node::Expr(construction), fuel)?;
            let body = arena.alloc_expr(Expr {
                hir_id: body_id,
                span: DUMMY_SP,
                kind: ExprKind::Projection(construction, name("value")),
            });
            Ok((
                FnSig {
                    params: &[],
                    return_ty: Some(result),
                    return_bind: None,
                    return_binding: None,
                    modifiers: Default::default(),
                    clause_params: &[],
                    clause_constraints: &[],
                    span: DUMMY_SP,
                },
                body,
            ))
        },
    )
    .unwrap();
    batch
        .define(function.definition, Some(identity), None)
        .unwrap();
    batch.body(function.body_id, function.body).unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut fuel,
        &BTreeMap::from([(7, 879)]),
        &Default::default(),
        None,
    )
    .unwrap();
    Interpreter::new(&checked, Budget::default())
        .unwrap()
        .invoke(
            nir::DeclarationId {
                package: 879,
                local: identity.index,
            },
            vec![],
        )
        .unwrap()
}

#[test]
fn quoted_defaults_reject_wrong_types_then_publish_and_execute_after_retry() {
    let arena = HirArena::new();
    let publisher = publisher(&arena, 100_000);
    let mut steps = 1_000_000;
    let draft = request("Generated");
    let rejected = publisher.prepare_with_defaults(&draft, &mut steps, |_, field, nodes, fuel| {
        quoted_default(&arena, field, nodes, fuel, false)
    });
    assert!(rejected.is_err());
    drop(rejected);
    assert_eq!(publisher.published_definition_count(), 0);
    assert!(publisher.last_diagnostic().is_some());
    let ty = {
        let prepared = publisher
            .prepare_with_defaults(&draft, &mut steps, |_, field, nodes, fuel| {
                quoted_default(&arena, field, nodes, fuel, true)
            })
            .unwrap();
        let ty = Term::Nominal {
            declaration: prepared.result,
            arguments: vec![],
        };
        assert_eq!(publisher.published_definition_count(), 0);
        (prepared.commit)().unwrap();
        assert_eq!(publisher.published_definition_count(), 1);
        assert!(publisher.last_diagnostic().is_none());
        ty
    };
    {
        let mut later = request("Later");
        later.fields[0].ty = ty.clone();
        let prepared = publisher.prepare(&later, &mut steps).unwrap();
        (prepared.commit)().unwrap();
    }
    assert_eq!(publisher.published_definition_count(), 2);
    let (hir, checked) = publisher.finish();
    assert_eq!(checked.unwrap().structs[0].fields[0].1, boolean());
    assert_eq!(execute_default(&arena, hir, ty), Value::Bool(true));
}

#[test]
fn default_callbacks_run_in_field_order_and_candidates_remain_transactional() {
    let arena = HirArena::new();
    let publisher = publisher(&arena, 200_000);
    let mut steps = 1_000_000;
    let mut draft = request("First");
    draft.fields.push(ReflectedField {
        name: "optional".into(),
        ty: boolean(),
    });
    let mut visited = vec![];
    {
        let first = publisher
            .prepare_with_defaults(&draft, &mut steps, |index, field, nodes, fuel| {
                visited.push(index);
                if index == 1 {
                    Ok(None)
                } else {
                    quoted_default(&arena, field, nodes, fuel, true)
                }
            })
            .unwrap();
        assert_eq!(visited, vec![0, 1]);
        let stale = publisher
            .prepare_with_defaults(&request("Stale"), &mut steps, |_, field, nodes, fuel| {
                quoted_default(&arena, field, nodes, fuel, true)
            })
            .unwrap();
        (first.commit)().unwrap();
        assert!((stale.commit)().is_err());
        let abandoned = publisher
            .prepare_with_defaults(
                &request("Abandoned"),
                &mut steps,
                |_, field, nodes, fuel| quoted_default(&arena, field, nodes, fuel, true),
            )
            .unwrap();
        drop(abandoned);
        assert_eq!(publisher.published_definition_count(), 1);
    }
    let (hir, checked) = publisher.finish();
    assert_eq!(checked.unwrap().structs.len(), 1);
    let item = hir.owners().next().unwrap().1.node.expect_item();
    let ItemKind::Struct(definition) = &item.kind else {
        panic!("expected struct")
    };
    assert!(definition.fields[0].default.is_some());
    assert!(definition.fields[1].default.is_none());
}

#[test]
fn default_assembly_failure_and_unwind_never_publish_or_refund_work() {
    let arena = HirArena::new();
    let publisher = publisher(&arena, 100_000);
    let mut steps = 1_000_000;
    let initial_steps = steps;
    let rejected = publisher.prepare_with_defaults(&request("Bad"), &mut steps, |_, _, _, fuel| {
        *fuel = 0;
        Err(definitions::Error {
            kind: "test assembly failure",
            owner: None,
        })
    });
    assert!(rejected.is_err());
    drop(rejected);
    assert_eq!(publisher.remaining_structural_fuel(), 0);
    assert_eq!(publisher.published_definition_count(), 0);
    assert_eq!(steps, initial_steps);
    assert!(
        publisher
            .last_diagnostic()
            .unwrap()
            .contains("test assembly failure")
    );
    let publisher = StructPublisher::new(
        &arena,
        Package::new(),
        7,
        BTreeMap::from([(7, 879)]),
        Default::default(),
        None,
        100_000,
    )
    .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = publisher.prepare_with_defaults(&request("Panic"), &mut steps, |_, _, _, _| {
            panic!("host assembler failed")
        });
    }));
    assert!(result.is_err());
    assert_eq!(publisher.remaining_structural_fuel(), 0);
    assert_eq!(publisher.published_definition_count(), 0);
    assert!(publisher.finish().1.is_none());
}

#[test]
fn misplaced_default_root_rejects_without_poisoning_the_next_candidate() {
    let arena = HirArena::new();
    let publisher = publisher(&arena, 100_000);
    let mut steps = 1_000_000;
    let error = publisher
        .prepare_with_defaults(&request("WrongScope"), &mut steps, |_, _, nodes, fuel| {
            Ok(Some(arena.alloc_expr(Expr {
                hir_id: nodes.reserve(nodes.root(), fuel)?,
                span: DUMMY_SP,
                kind: ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                }),
            })))
        })
        .err()
        .unwrap();
    assert_eq!(error.source, Some(SourceRange { start: 10, end: 20 }));
    assert!(
        publisher
            .last_diagnostic()
            .unwrap()
            .contains("field default root")
    );
    assert_eq!(publisher.published_definition_count(), 0);
    {
        let prepared = publisher
            .prepare_with_defaults(&request("Repaired"), &mut steps, |_, field, nodes, fuel| {
                quoted_default(&arena, field, nodes, fuel, true)
            })
            .unwrap();
        (prepared.commit)().unwrap();
    }
    assert_eq!(publisher.published_definition_count(), 1);
    assert!(publisher.last_diagnostic().is_none());
}
