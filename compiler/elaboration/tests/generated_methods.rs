use elaboration::{definitions::publish, program::Protocols};
use hir::{
    common::{FnParam, TyParam, TyParamKind},
    definitions::{DefinitionBatch, TypedField, assemble_struct_with_defaults},
    *,
};
use rustc_span::DUMMY_SP;
use std::{collections::BTreeMap, sync::Arc};

fn ident(name: &str) -> Ident {
    Ident::new(Symbol::intern(name), DUMMY_SP)
}
fn id(owner: OwnerId, local: u32) -> HirId {
    HirId::new(owner, ItemLocalId::new(local))
}
fn expr<'a>(arena: &'a HirArena, owner: OwnerId, local: u32, kind: ExprKind<'a>) -> &'a Expr<'a> {
    arena.alloc_expr(Expr {
        hir_id: id(owner, local),
        kind,
        span: DUMMY_SP,
    })
}
fn item<'a>(arena: &'a HirArena, owner: OwnerId, name: &str, kind: ItemKind<'a>) -> OwnerInfo<'a> {
    OwnerInfo {
        node: OwnerNode::Item(arena.alloc_item(Item {
            owner_id: owner,
            ident: ident(name),
            kind,
            span: DUMMY_SP,
        })),
        nodes: OwnerNodes::new(),
    }
}
fn signature<'a>(params: &'a [FnParam<'a>], return_ty: &'a Expr<'a>) -> FnSig<'a> {
    FnSig {
        params,
        return_ty: Some(return_ty),
        return_bind: None,
        return_binding: None,
        modifiers: Default::default(),
        clause_params: &[],
        clause_constraints: &[],
        span: DUMMY_SP,
    }
}

#[test]
fn generated_struct_impl_and_instance_method_publish_atomically_and_execute() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let record = batch.reserve().unwrap();
    let implementation = batch.reserve().unwrap();
    let method = batch.reserve().unwrap();
    let consumer = batch.reserve().unwrap();
    let record_type = nir::Term::Nominal {
        declaration: nir::DeclarationId {
            package: 73,
            local: record.def_id.raw(),
        },
        arguments: vec![],
    };
    let fields = [TypedField {
        name: ident("flag"),
        ty: Arc::new(nir::Term::Primitive(nir::Primitive::Bool)),
        span: DUMMY_SP,
    }];
    let structure = assemble_struct_with_defaults(
        &arena,
        record,
        ident("Generated"),
        &fields,
        DUMMY_SP,
        &mut 100,
        |_, field, nodes, fuel| {
            let root = nodes.reserve(field, fuel)?;
            Ok(Some(arena.alloc_expr(Expr {
                hir_id: root,
                kind: ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                }),
                span: DUMMY_SP,
            })))
        },
    )
    .unwrap();
    batch.define(structure, None, None).unwrap();
    let implementation_info = hir::definitions::assemble_inherent_impl(
        &arena,
        implementation,
        ident("generated_impl"),
        Arc::new(record_type.clone()),
        &[method],
        DUMMY_SP,
        &mut 100,
    )
    .unwrap();
    batch.define(implementation_info, None, None).unwrap();
    let parameters = arena.alloc_fn_param_slice(vec![FnParam {
        hir_id: id(method, 1),
        name: ident("self"),
        index: 0,
        ty: TyParam::new(
            id(method, 1),
            TyParamKind::Itself { is_ref: false },
            DUMMY_SP,
        ),
        span: DUMMY_SP,
    }]);
    let return_ty = expr(&arena, method, 4, ExprKind::Builtin(Symbol::intern("bool")));
    let method_body = BodyId::new(id(method, 2));
    batch
        .define(
            item(
                &arena,
                method,
                "enabled",
                ItemKind::Fn(signature(parameters, return_ty), method_body),
            ),
            None,
            None,
        )
        .unwrap();
    let body_params = arena.alloc_param_slice(vec![BodyParam::from_fn_param(&parameters[0])]);
    let wrong = expr(
        &arena,
        method,
        2,
        ExprKind::Lit(Lit {
            kind: LitKind::Char('x'),
            span: DUMMY_SP,
        }),
    );
    batch
        .body(
            method_body,
            Body {
                params: body_params,
                value: wrong,
            },
        )
        .unwrap();

    let base = expr(
        &arena,
        consumer,
        5,
        ExprKind::DeliveredType(Arc::new(record_type)),
    );
    let constructed = expr(
        &arena,
        consumer,
        4,
        ExprKind::ExtendedApplication(base, &[]),
    );
    let selected = expr(
        &arena,
        consumer,
        3,
        ExprKind::Projection(constructed, ident("enabled")),
    );
    let call = expr(&arena, consumer, 2, ExprKind::Application(selected, &[]));
    let return_ty = expr(
        &arena,
        consumer,
        1,
        ExprKind::Builtin(Symbol::intern("bool")),
    );
    let consumer_body = BodyId::new(id(consumer, 2));
    batch
        .body(
            consumer_body,
            Body {
                params: &[],
                value: call,
            },
        )
        .unwrap();
    batch
        .define(
            item(
                &arena,
                consumer,
                "run",
                ItemKind::Fn(signature(&[], return_ty), consumer_body),
            ),
            None,
            None,
        )
        .unwrap();
    let identities = BTreeMap::from([(0, 73)]);
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
    assert_eq!(batch.published().num_bodies(), 0);

    let receiver = expr(
        &arena,
        method,
        3,
        ExprKind::ResolvedFnParam(Symbol::intern("self"), id(method, 1)),
    );
    let body = expr(
        &arena,
        method,
        2,
        ExprKind::Projection(receiver, ident("flag")),
    );
    batch
        .body(
            method_body,
            Body {
                params: body_params,
                value: body,
            },
        )
        .unwrap();
    let (owners, checked) = publish(
        &mut batch,
        &mut 500,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(owners, [record, implementation, method, consumer]);
    assert_eq!(batch.published().num_bodies(), 2);
    let mut vm =
        comptime_interpretor::Interpreter::new(&checked, comptime_interpretor::Budget::default())
            .unwrap();
    let consumer = nir::DeclarationId {
        package: 73,
        local: consumer.def_id.raw(),
    };
    assert_eq!(
        vm.invoke(consumer, vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert_eq!(
        vm.invoke(consumer, vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
