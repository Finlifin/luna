use hir::{definitions::assemble_enum, *};
use rustc_span::DUMMY_SP;

fn ident(name: &str) -> Ident {
    Ident::new(Symbol::intern(name), DUMMY_SP)
}
fn owner() -> OwnerId {
    OwnerId::new(LocalDefId::new(1))
}

#[test]
fn variants_and_payloads_have_canonical_indices_and_order() {
    let arena = HirArena::new();
    let names = [ident("none"), ident("some"), ident("group")];
    let mut seen = Vec::new();
    let info = assemble_enum(
        &arena,
        owner(),
        ident("Generated"),
        &names,
        DUMMY_SP,
        &mut 100,
        |index, parent, nodes, fuel| {
            seen.push(index);
            Ok(match index {
                0 => VariantKind::Unit,
                1 => {
                    let hir_id = nodes.reserve(parent, fuel)?;
                    let types = arena.alloc_expr_slice(vec![Expr {
                        hir_id,
                        kind: ExprKind::Builtin(Symbol::intern("bool")),
                        span: DUMMY_SP,
                    }]);
                    nodes.register(Node::Expr(&types[0]), fuel)?;
                    VariantKind::Tuple(types)
                }
                _ => {
                    let hir_id = nodes.reserve(parent, fuel)?;
                    let children = arena.alloc_variant_slice(vec![Variant {
                        hir_id,
                        ident: ident("leaf"),
                        kind: VariantKind::Unit,
                        span: DUMMY_SP,
                    }]);
                    nodes.register(Node::Variant(&children[0]), fuel)?;
                    VariantKind::SubEnum(children)
                }
            })
        },
    )
    .unwrap();
    assert_eq!(seen, [0, 1, 2]);
    let ItemKind::Enum(def) = &info.node.expect_item().kind else {
        panic!()
    };
    assert_eq!(
        def.variants.iter().map(|v| v.ident).collect::<Vec<_>>(),
        names
    );
    for variant in def.variants {
        let entry = info.nodes.get(variant.hir_id.local_id).unwrap();
        assert_eq!(entry.parent, ItemLocalId::new(0));
        assert!(matches!(entry.node, Node::Variant(v) if std::ptr::eq(v, variant)));
        let child = match variant.kind {
            VariantKind::Tuple(types) => Some(types[0].hir_id),
            VariantKind::SubEnum(children) => Some(children[0].hir_id),
            _ => None,
        };
        if let Some(child) = child {
            assert_eq!(
                info.nodes.get(child.local_id).unwrap().parent,
                variant.hir_id.local_id
            );
        }
    }
    let dump = info.node.expect_item().to_string();
    assert!(dump.contains("Generated"));
}

#[test]
fn budget_preflight_empty_enum_and_callback_failures() {
    let arena = HirArena::new();
    let mut fuel = 2;
    let mut called = false;
    assert!(
        assemble_enum(
            &arena,
            owner(),
            ident("E"),
            &[ident("a")],
            DUMMY_SP,
            &mut fuel,
            |_, _, _, _| {
                called = true;
                Ok(VariantKind::Unit)
            }
        )
        .is_err()
    );
    assert!(!called);
    assert_eq!(fuel, 2);
    assert!(
        assemble_enum(
            &arena,
            owner(),
            ident("E"),
            &[],
            DUMMY_SP,
            &mut 1,
            |_, _, _, _| unreachable!()
        )
        .is_ok()
    );
    for case in 0..3 {
        let result = assemble_enum(
            &arena,
            owner(),
            ident("E"),
            &[ident("a")],
            DUMMY_SP,
            &mut 20,
            |_, parent, nodes, fuel| {
                let id = nodes.reserve(if case == 0 { nodes.root() } else { parent }, fuel)?;
                let expr = arena.alloc_expr(Expr {
                    hir_id: id,
                    kind: ExprKind::TyVoid,
                    span: DUMMY_SP,
                });
                if case != 1 {
                    nodes.register(Node::Expr(expr), fuel)?;
                }
                if case == 2 {
                    *fuel = 0;
                }
                Ok(VariantKind::Const(expr))
            },
        );
        assert!(result.is_err(), "case {case}");
    }
}

#[test]
fn pattern_and_expression_variants_retain_their_distinct_payloads() {
    let arena = HirArena::new();
    let names = [ident("pattern\n\""), ident("expression")];
    let info = assemble_enum(
        &arena,
        owner(),
        ident("Encoded"),
        &names,
        DUMMY_SP,
        &mut 100,
        |index, parent, nodes, fuel| {
            let hir_id = nodes.reserve(parent, fuel)?;
            if index == 0 {
                let pattern = arena.alloc_pattern(Pattern {
                    hir_id,
                    kind: PatternKind::Wild,
                    span: DUMMY_SP,
                });
                nodes.register(Node::Pattern(pattern), fuel)?;
                Ok(VariantKind::Pattern(pattern))
            } else {
                let value = arena.alloc_expr(Expr {
                    hir_id,
                    kind: ExprKind::Lit(Lit {
                        kind: LitKind::Bool(true),
                        span: DUMMY_SP,
                    }),
                    span: DUMMY_SP,
                });
                nodes.register(Node::Expr(value), fuel)?;
                Ok(VariantKind::Const(value))
            }
        },
    )
    .unwrap();
    let ItemKind::Enum(def) = &info.node.expect_item().kind else {
        panic!()
    };
    assert!(matches!(def.variants[0].kind, VariantKind::Pattern(_)));
    assert!(matches!(def.variants[1].kind, VariantKind::Const(_)));
    let dump = info.node.expect_item().to_string();
    assert!(dump.contains("pattern\\n\\\""), "{dump}");
    // Assembly alone grants no encoding, coverage or layout validity.
}

#[test]
fn complete_enum_assembly_preserves_metadata_and_checks_variant_parent() {
    use hir::definitions::assemble_enum_definition;
    let arena = HirArena::new();
    let mut called = false;
    let error = assemble_enum_definition(&arena, owner(), ident("E"), DUMMY_SP, &mut 0, |_, _| {
        called = true;
        unreachable!()
    })
    .unwrap_err();
    assert_eq!(error.kind, "budget exhausted");
    assert!(!called);
    for valid in [false, true] {
        let result = assemble_enum_definition(
            &arena,
            owner(),
            ident("E"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let root = nodes.root();
                let metadata = arena.alloc_expr(Expr {
                    hir_id: nodes.reserve(root, fuel)?,
                    kind: ExprKind::Lit(Lit {
                        kind: LitKind::Bool(true),
                        span: DUMMY_SP,
                    }),
                    span: DUMMY_SP,
                });
                nodes.register(Node::Expr(metadata), fuel)?;
                let variants = arena.alloc_variant_slice([Variant {
                    hir_id: nodes.reserve(if valid { root } else { metadata.hir_id }, fuel)?,
                    ident: ident("op"),
                    kind: VariantKind::Unit,
                    span: DUMMY_SP,
                }]);
                Ok(EnumDef {
                    is_intrinsic: true,
                    variants,
                    configurations: vec![Configuration {
                        ident: ident("setting"),
                        value: metadata,
                        span: DUMMY_SP,
                    }],
                    clause_params: &[],
                    clause_constraints: &[],
                    nested_items: vec![OwnerId::new(LocalDefId::new(9))],
                })
            },
        );
        if valid {
            let info = result.unwrap();
            let ItemKind::Enum(def) = &info.node.expect_item().kind else {
                panic!()
            };
            assert!(def.is_intrinsic);
            assert_eq!(def.configurations[0].ident, ident("setting"));
            assert_eq!(def.nested_items, vec![OwnerId::new(LocalDefId::new(9))]);
            assert!(matches!(
                info.nodes
                    .get(def.variants[0].hir_id.local_id)
                    .unwrap()
                    .node,
                Node::Variant(_)
            ));
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                "variant must be reserved under its definition"
            );
        }
    }
}
