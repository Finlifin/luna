use elaboration::{definitions::publish, program::Protocols};
use hir::{
    definitions::{DefinitionBatch, assemble_struct_definition},
    *,
};
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}

fn description<'a>(arena: &'a HirArena, owner: OwnerId, valid: bool) -> OwnerInfo<'a> {
    assemble_struct_definition(
        arena,
        owner,
        name("Generated"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let parameter = hir::common::DefId::new(0, 1000);
            let clauses = arena.alloc_clause_param_slice([ClauseParam {
                hir_id: nodes.reserve(nodes.root(), fuel)?,
                definition: Some(parameter),
                name: name("T"),
                kind: ClauseParamKind::Type(name("T")),
                span: DUMMY_SP,
            }]);
            nodes.register(Node::ClauseParam(&clauses[0]), fuel)?;
            let field = nodes.reserve(nodes.root(), fuel)?;
            let ty = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(field, fuel)?,
                span: DUMMY_SP,
                kind: if valid {
                    ExprKind::ResolvedClauseParam(Symbol::intern("T"), parameter)
                } else {
                    ExprKind::Lit(Lit {
                        kind: LitKind::Bool(true),
                        span: DUMMY_SP,
                    })
                },
            });
            nodes.register(Node::Expr(ty), fuel)?;
            let flag = nodes.reserve(nodes.root(), fuel)?;
            let flag_type = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(flag, fuel)?,
                kind: ExprKind::Builtin(Symbol::intern("bool")),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(flag_type), fuel)?;
            let flag_default = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(flag, fuel)?,
                kind: ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                }),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(flag_default), fuel)?;
            let fields = arena.alloc_field_def_slice([
                FieldDef {
                    hir_id: field,
                    ident: name("value"),
                    ty,
                    default: None,
                    span: DUMMY_SP,
                },
                FieldDef {
                    hir_id: flag,
                    ident: name("enabled"),
                    ty: flag_type,
                    default: Some(flag_default),
                    span: DUMMY_SP,
                },
            ]);
            Ok(StructDef {
                fields,
                clause_params: clauses,
                clause_constraints: &[],
                nested_items: vec![],
            })
        },
    )
    .unwrap()
}

#[test]
fn generated_generic_struct_checks_then_reflects_each_substitution() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(description(&arena, owner, false), None, None)
        .unwrap();
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
        .define(description(&arena, owner, true), None, None)
        .unwrap();
    let (_, checked) = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    let declaration = checked.structs[0].declaration;
    assert_eq!(checked.structs[0].type_parameters.len(), 1);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&checked, Default::default()).unwrap();
    for primitive in [
        nir::Primitive::Bool,
        nir::Primitive::Char,
        nir::Primitive::Bool,
    ] {
        let ty = nir::Term::Primitive(primitive);
        let reflected = vm
            .reflect_struct(&nir::Term::Nominal {
                declaration,
                arguments: vec![ty.clone()],
            })
            .unwrap();
        assert_eq!(reflected.fields.len(), 2);
        assert_eq!(reflected.fields[0].name, "value");
        assert_eq!(reflected.fields[0].ty, ty);
    }
    assert!(
        vm.reflect_struct(&nir::Term::Nominal {
            declaration,
            arguments: vec![]
        })
        .is_err()
    );
}

fn consumer<'a>(
    arena: &'a HirArena,
    owner: OwnerId,
    record: OwnerId,
    primitive: nir::Primitive,
    value: LitKind,
    override_flag: bool,
    comptime: bool,
) -> hir::definitions::AssembledFunction<'a> {
    use hir::definitions::assemble_function;
    assemble_function(
        arena,
        owner,
        name("construct"),
        DUMMY_SP,
        &mut 100,
        |nodes, fuel| {
            let root = nodes.root();
            let result_type = std::sync::Arc::new(nir::Term::Nominal {
                declaration: nir::DeclarationId {
                    package: 73,
                    local: record.def_id.raw(),
                },
                arguments: vec![nir::Term::Primitive(primitive)],
            });
            let result = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(root, fuel)?,
                kind: ExprKind::DeliveredType(result_type.clone()),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(result), fuel)?;
            let body_id = nodes.reserve(root, fuel)?;
            let base = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(body_id, fuel)?,
                kind: ExprKind::DeliveredType(result_type),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(base), fuel)?;
            let value = arena.alloc_expr(Expr {
                hir_id: nodes.reserve(body_id, fuel)?,
                kind: ExprKind::Lit(Lit {
                    kind: value,
                    span: DUMMY_SP,
                }),
                span: DUMMY_SP,
            });
            nodes.register(Node::Expr(value), fuel)?;
            let mut inputs = vec![hir::common::Arg::Named(name("value"), value)];
            if override_flag {
                let flag = arena.alloc_expr(Expr {
                    hir_id: nodes.reserve(body_id, fuel)?,
                    kind: ExprKind::Lit(Lit {
                        kind: LitKind::Bool(false),
                        span: DUMMY_SP,
                    }),
                    span: DUMMY_SP,
                });
                nodes.register(Node::Expr(flag), fuel)?;
                inputs.push(hir::common::Arg::Named(name("enabled"), flag));
            }
            let body = arena.alloc_expr(Expr {
                hir_id: body_id,
                kind: ExprKind::ExtendedApplication(base, arena.alloc_arg_slice(inputs)),
                span: DUMMY_SP,
            });
            let signature = FnSig {
                params: &[],
                return_ty: Some(result),
                return_bind: None,
                return_binding: None,
                modifiers: hir::item::FnModifiers {
                    is_pure: true,
                    is_comptime: comptime,
                    ..Default::default()
                },
                clause_params: &[],
                clause_constraints: &[],
                span: DUMMY_SP,
            };
            Ok((signature, body))
        },
    )
    .unwrap()
}

fn check_construction(comptime: bool) {
    use comptime_interpretor::{Interpreter, Value};
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let record = batch.reserve().unwrap();
    let bool_entry = batch.reserve().unwrap();
    let char_entry = batch.reserve().unwrap();
    batch
        .define(description(&arena, record, true), None, None)
        .unwrap();
    fn install<'a>(
        batch: &mut DefinitionBatch<'_, 'a>,
        function: hir::definitions::AssembledFunction<'a>,
    ) {
        batch.define(function.definition, None, None).unwrap();
        batch.body(function.body_id, function.body).unwrap();
    }
    install(
        &mut batch,
        consumer(
            &arena,
            bool_entry,
            record,
            nir::Primitive::Bool,
            LitKind::Char('x'),
            false,
            comptime,
        ),
    );
    install(
        &mut batch,
        consumer(
            &arena,
            char_entry,
            record,
            nir::Primitive::Char,
            LitKind::Char('x'),
            true,
            comptime,
        ),
    );
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
    install(
        &mut batch,
        consumer(
            &arena,
            bool_entry,
            record,
            nir::Primitive::Bool,
            LitKind::Bool(false),
            false,
            comptime,
        ),
    );
    let (_, checked) = publish(
        &mut batch,
        &mut 5000,
        &identities,
        &Protocols::default(),
        None,
    )
    .unwrap();
    let mut vm = if comptime {
        Interpreter::for_comptime(&checked, Default::default())
    } else {
        Interpreter::new(&checked, Default::default())
    }
    .unwrap();
    for (entry, primitive, value, enabled) in [
        (bool_entry, nir::Primitive::Bool, Value::Bool(false), true),
        (char_entry, nir::Primitive::Char, Value::Char('x'), false),
        (bool_entry, nir::Primitive::Bool, Value::Bool(false), true),
    ] {
        assert_eq!(
            vm.invoke(
                nir::DeclarationId {
                    package: 73,
                    local: entry.def_id.raw()
                },
                vec![]
            )
            .unwrap(),
            Value::Record {
                declaration: nir::DeclarationId {
                    package: 73,
                    local: record.def_id.raw()
                },
                arguments: vec![nir::Term::Primitive(primitive)],
                fields: vec![value, Value::Bool(enabled)]
            }
        );
    }
}

#[test]
fn generated_generic_struct_construction_checks_inputs_and_applies_defaults() {
    check_construction(true);
}

#[test]
fn generated_generic_struct_construction_executes_at_runtime() {
    check_construction(false);
}
