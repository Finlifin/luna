use elaboration::{definitions::publish, program::Protocols};
use hir::{
    definitions::{
        DefinitionBatch, DefinitionNodes, Error, TypeDeclaration, assemble_function,
        assemble_newtype, assemble_type_alias,
    },
    *,
};
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn expr<'a>(
    arena: &'a HirArena,
    nodes: &mut DefinitionNodes<'a>,
    parent: HirId,
    kind: ExprKind<'a>,
    fuel: &mut usize,
) -> Result<&'a Expr<'a>, Error> {
    let expr = arena.alloc_expr(Expr {
        hir_id: nodes.reserve(parent, fuel)?,
        kind,
        span: DUMMY_SP,
    });
    nodes.register(Node::Expr(expr), fuel)?;
    Ok(expr)
}
fn declaration<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    nominal: bool,
    valid: bool,
) -> OwnerInfo<'a> {
    let make =
        |nodes: &mut DefinitionNodes<'a>, fuel: &mut usize| -> Result<TypeDeclaration<'a>, Error> {
            let root = nodes.root();
            let identity = hir::common::DefId::new(0, 1000 + owner.def_id.raw());
            let parameters = arena.alloc_clause_param_slice([ClauseParam {
                hir_id: nodes.reserve(root, fuel)?,
                definition: Some(identity),
                name: name("T"),
                kind: ClauseParamKind::Type(name("T")),
                span: DUMMY_SP,
            }]);
            nodes.register(Node::ClauseParam(&parameters[0]), fuel)?;
            let body = expr(
                arena,
                nodes,
                root,
                if valid {
                    ExprKind::ResolvedClauseParam(Symbol::intern("T"), identity)
                } else {
                    ExprKind::Lit(Lit {
                        kind: LitKind::Bool(true),
                        span: DUMMY_SP,
                    })
                },
                fuel,
            )?;
            Ok((body, parameters, &[]))
        };
    if nominal {
        assemble_newtype(arena, owner, name("Wrapped"), DUMMY_SP, &mut 100, make)
    } else {
        assemble_type_alias(arena, owner, name("Alias"), DUMMY_SP, &mut 100, make)
    }
    .unwrap()
}

#[test]
fn generated_aliases_and_newtypes_keep_distinct_type_semantics() {
    use comptime_interpretor::{Interpreter, Value};
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let alias = batch.reserve().unwrap();
    let nominal = batch.reserve().unwrap();
    let alias_entry = batch.reserve().unwrap();
    let nominal_entry = batch.reserve().unwrap();
    let bind = |owner: OwnerId| Some(hir::common::DefId::new(0, owner.def_id.raw()));
    batch
        .define(declaration(&arena, alias, false, true), bind(alias), None)
        .unwrap();
    batch
        .define(
            declaration(&arena, nominal, true, false),
            bind(nominal),
            None,
        )
        .unwrap();
    for (owner, target) in [(alias_entry, alias), (nominal_entry, nominal)] {
        let function = assemble_function(
            &arena,
            owner,
            name("get_type"),
            DUMMY_SP,
            &mut 100,
            |nodes, fuel| {
                let root = nodes.root();
                let result = expr(
                    &arena,
                    nodes,
                    root,
                    ExprKind::Builtin(Symbol::intern("Type")),
                    fuel,
                )?;
                let body_id = nodes.reserve(root, fuel)?;
                let head = expr(
                    &arena,
                    nodes,
                    body_id,
                    ExprKind::ResolvedDef {
                        name: Symbol::intern("Target"),
                        has_clause_param: true,
                        def_id: bind(target).unwrap(),
                    },
                    fuel,
                )?;
                let input = expr(
                    &arena,
                    nodes,
                    body_id,
                    ExprKind::Builtin(Symbol::intern("bool")),
                    fuel,
                )?;
                let body = arena.alloc_expr(Expr {
                    hir_id: body_id,
                    span: DUMMY_SP,
                    kind: ExprKind::NFApplication(
                        head,
                        arena.alloc_arg_slice([hir::common::Arg::Positional(input)]),
                    ),
                });
                Ok((
                    FnSig {
                        params: &[],
                        return_ty: Some(result),
                        return_bind: None,
                        return_binding: None,
                        modifiers: hir::item::FnModifiers {
                            is_pure: true,
                            is_comptime: true,
                            ..Default::default()
                        },
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
            .define(function.definition, bind(owner), None)
            .unwrap();
        batch.body(function.body_id, function.body).unwrap();
    }
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
    batch
        .define(
            declaration(&arena, nominal, true, true),
            bind(nominal),
            None,
        )
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(checked.newtypes.len(), 1);
    let mut vm = Interpreter::for_comptime(&checked, Default::default()).unwrap();
    let id = |owner: OwnerId| nir::DeclarationId {
        package: 73,
        local: owner.def_id.raw(),
    };
    let primitive = nir::Term::Primitive(nir::Primitive::Bool);
    assert_eq!(
        vm.invoke(id(alias_entry), vec![]).unwrap(),
        Value::Type(primitive.clone())
    );
    assert_eq!(
        vm.invoke(id(nominal_entry), vec![]).unwrap(),
        Value::Type(nir::Term::Nominal {
            declaration: id(nominal),
            arguments: vec![primitive],
        })
    );
}
