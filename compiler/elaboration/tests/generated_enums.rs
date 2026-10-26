use elaboration::{definitions::publish, program::Protocols};
use hir::{
    definitions::{DefinitionBatch, assemble_enum},
    *,
};
use nir::{Primitive, Term, program::VariantShape};
use rustc_span::DUMMY_SP;
use std::{collections::BTreeMap, sync::Arc};

fn ident(name: &str) -> Ident {
    Ident::new(Symbol::intern(name), DUMMY_SP)
}
fn description<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    payload: Term,
    duplicate: bool,
) -> OwnerInfo<'a> {
    let names = [
        ident("none"),
        ident(if duplicate { "none" } else { "some" }),
        ident("named"),
        ident("group"),
    ];
    assemble_enum(
        arena,
        owner,
        ident("Generated"),
        &names,
        DUMMY_SP,
        &mut 100,
        |index, variant, nodes, fuel| {
            Ok(match index {
                0 => VariantKind::Unit,
                1 => {
                    let hir_id = nodes.reserve(variant, fuel)?;
                    let types = arena.alloc_expr_slice(vec![Expr {
                        hir_id,
                        kind: ExprKind::DeliveredType(Arc::new(payload.clone())),
                        span: DUMMY_SP,
                    }]);
                    nodes.register(Node::Expr(&types[0]), fuel)?;
                    VariantKind::Tuple(types)
                }
                2 => {
                    let field_id = nodes.reserve(variant, fuel)?;
                    let ty_id = nodes.reserve(field_id, fuel)?;
                    let ty = arena.alloc_expr(Expr {
                        hir_id: ty_id,
                        kind: ExprKind::DeliveredType(Arc::new(Term::Primitive(Primitive::Char))),
                        span: DUMMY_SP,
                    });
                    nodes.register(Node::Expr(ty), fuel)?;
                    let fields = arena.alloc_field_def_slice(vec![FieldDef {
                        hir_id: field_id,
                        ident: ident("letter"),
                        ty,
                        default: None,
                        span: DUMMY_SP,
                    }]);
                    nodes.register(Node::FieldDef(&fields[0]), fuel)?;
                    VariantKind::Struct(fields)
                }
                _ => {
                    let hir_id = nodes.reserve(variant, fuel)?;
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
    .unwrap()
}

#[test]
fn generated_enum_payloads_check_atomically_then_reflect_in_declaration_order() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let identities = BTreeMap::from([(0, 73)]);
    for (payload, duplicate) in [
        (Term::Bool(true), false),
        (Term::Primitive(Primitive::Bool), true),
    ] {
        batch
            .define(description(&arena, owner, payload, duplicate), None, None)
            .unwrap();
        let before = batch.to_string();
        assert!(
            publish(
                &mut batch,
                &mut 500,
                &identities,
                &Protocols::default(),
                None
            )
            .is_err()
        );
        assert_eq!(batch.to_string(), before);
        assert_eq!(batch.published().num_defs(), 0);
    }
    batch
        .define(
            description(&arena, owner, Term::Primitive(Primitive::Bool), false),
            None,
            None,
        )
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 500,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(batch.published().num_defs(), 1);
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &checked,
        comptime_interpretor::Budget::default(),
    )
    .unwrap();
    let reflected = vm
        .reflect_enum(&Term::Nominal {
            declaration: nir::DeclarationId {
                package: 73,
                local: owner.def_id.raw(),
            },
            arguments: vec![],
        })
        .unwrap();
    assert_eq!(
        reflected
            .variants
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["none", "some", "named", "group.leaf"]
    );
    assert_eq!(reflected.groups, ["group"]);
    assert_eq!(reflected.variants[0].shape, VariantShape::Unit);
    assert_eq!(reflected.variants[1].shape, VariantShape::Tuple);
    assert_eq!(
        reflected.variants[1].fields,
        [Term::Primitive(Primitive::Bool)]
    );
    assert_eq!(
        reflected.variants[2].shape,
        VariantShape::Named(vec!["letter".into()])
    );
    assert_eq!(
        reflected.variants[2].fields,
        [Term::Primitive(Primitive::Char)]
    );
}

#[test]
fn generated_generic_enum_preserves_payload_parameters_and_rejects_bad_types() {
    use hir::definitions::assemble_enum_definition;
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let make = |valid| {
        assemble_enum_definition(
            &arena,
            owner,
            ident("Generated"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let parameter = hir::common::DefId::new(0, 2000);
                let clauses = arena.alloc_clause_param_slice([ClauseParam {
                    hir_id: nodes.reserve(nodes.root(), fuel)?,
                    definition: Some(parameter),
                    name: ident("T"),
                    kind: ClauseParamKind::Type(ident("T")),
                    span: DUMMY_SP,
                }]);
                nodes.register(Node::ClauseParam(&clauses[0]), fuel)?;
                let empty = nodes.reserve(nodes.root(), fuel)?;
                let some = nodes.reserve(nodes.root(), fuel)?;
                let payload = arena.alloc_expr_slice([Expr {
                    hir_id: nodes.reserve(some, fuel)?,
                    span: DUMMY_SP,
                    kind: if valid {
                        ExprKind::ResolvedClauseParam(Symbol::intern("T"), parameter)
                    } else {
                        ExprKind::Lit(Lit {
                            kind: LitKind::Bool(true),
                            span: DUMMY_SP,
                        })
                    },
                }]);
                nodes.register(Node::Expr(&payload[0]), fuel)?;
                let variants = arena.alloc_variant_slice([
                    Variant {
                        hir_id: empty,
                        ident: ident("empty"),
                        kind: VariantKind::Unit,
                        span: DUMMY_SP,
                    },
                    Variant {
                        hir_id: some,
                        ident: ident("some"),
                        kind: VariantKind::Tuple(payload),
                        span: DUMMY_SP,
                    },
                ]);
                Ok(EnumDef {
                    is_intrinsic: false,
                    variants,
                    configurations: vec![],
                    clause_params: clauses,
                    clause_constraints: &[],
                    nested_items: vec![],
                })
            },
        )
        .unwrap()
    };
    batch.define(make(false), None, None).unwrap();
    let identities = BTreeMap::from([(0, 73)]);
    let before = batch.to_string();
    assert!(matches!(
        publish(
            &mut batch,
            &mut 5000,
            &identities,
            &Protocols::default(),
            None
        ),
        Err(hir::definitions::PublicationError::Check(_))
    ));
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    batch.define(make(true), None, None).unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    let declaration = checked.enums[0].declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&checked, Default::default()).unwrap();
    for primitive in [Primitive::Bool, Primitive::Char, Primitive::Bool] {
        let ty = Term::Primitive(primitive);
        let reflected = vm
            .reflect_enum(&Term::Nominal {
                declaration,
                arguments: vec![ty.clone()],
            })
            .unwrap();
        assert_eq!(reflected.variants.len(), 2);
        assert_eq!(reflected.variants[0].name, "empty");
        assert_eq!(reflected.variants[0].shape, VariantShape::Unit);
        assert_eq!(reflected.variants[1].name, "some");
        assert_eq!(reflected.variants[1].shape, VariantShape::Tuple);
        assert_eq!(reflected.variants[1].fields, vec![ty]);
    }
    assert!(
        vm.reflect_enum(&Term::Nominal {
            declaration,
            arguments: vec![]
        })
        .is_err()
    );
}
