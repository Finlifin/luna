use hir::{
    definitions::{
        DefinitionNodes, Error, TypedField, assemble_function, assemble_struct_with_defaults,
    },
    *,
};
use luna::{
    frontend::Artifact,
    packages::{SourcePackage, compile_with_definitions},
};
use rustc_span::DUMMY_SP;
use std::sync::Arc;

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
    let value = arena.alloc_expr(Expr {
        hir_id: nodes.reserve(parent, fuel)?,
        kind,
        span: DUMMY_SP,
    });
    nodes.register(Node::Expr(value), fuel)?;
    Ok(value)
}

fn expand<'a>(
    arena: &'a HirArena,
    batch: &mut hir::definitions::DefinitionBatch<'_, 'a>,
    fuel: &mut usize,
    valid: bool,
    pure: bool,
) -> Result<(), String> {
    let lookup = |text: &str| {
        batch
            .published()
            .owners()
            .find(|(_, info)| info.node.expect_item().ident.name.as_str() == text)
            .and_then(|(owner, _)| batch.published().owner_definition(owner))
            .unwrap()
    };
    let source = lookup("Source");
    let make = lookup("source");
    let source_identity = match source.pkg {
        7 => 73,
        11 => 91,
        other => panic!("unexpected source package {other}"),
    };
    let source_frontier = batch.published().next_definition_index();
    let record = batch.reserve().map_err(|e| e.to_string())?;
    let record_id = batch.reserve_identity(7, fuel).map_err(|e| e.to_string())?;
    assert!(record_id.index >= source_frontier);
    let record_type = nir::Term::Nominal {
        declaration: nir::DeclarationId {
            package: 73,
            local: record_id.index,
        },
        arguments: vec![],
    };
    let fields = [TypedField {
        name: name("item"),
        span: DUMMY_SP,
        ty: Arc::new(nir::Term::Nominal {
            declaration: nir::DeclarationId {
                package: source_identity,
                local: source.index,
            },
            arguments: vec![],
        }),
    }];
    let info = assemble_struct_with_defaults(
        arena,
        record,
        name("Generated"),
        &fields,
        DUMMY_SP,
        fuel,
        |_, field, nodes, fuel| {
            let root = nodes.reserve(field, fuel)?;
            let kind = if valid {
                let callee = expr(
                    arena,
                    nodes,
                    root,
                    ExprKind::ResolvedDef {
                        name: Symbol::intern("source"),
                        has_clause_param: false,
                        def_id: make,
                    },
                    fuel,
                )?;
                ExprKind::Application(callee, &[])
            } else {
                ExprKind::Lit(Lit {
                    kind: LitKind::Bool(true),
                    span: DUMMY_SP,
                })
            };
            Ok(Some(arena.alloc_expr(Expr {
                hir_id: root,
                kind,
                span: DUMMY_SP,
            })))
        },
    )
    .map_err(|e| e.to_string())?;
    batch
        .define(info, Some(record_id), None)
        .map_err(|e| e.to_string())?;
    let caller = batch.reserve().map_err(|e| e.to_string())?;
    let caller_id = batch.reserve_identity(7, fuel).map_err(|e| e.to_string())?;
    let function = assemble_function(arena, caller, name("run"), DUMMY_SP, fuel, |nodes, fuel| {
        let root = nodes.root();
        let result = expr(
            arena,
            nodes,
            root,
            ExprKind::Builtin(Symbol::intern("bool")),
            fuel,
        )?;
        let body = nodes.reserve(root, fuel)?;
        let item = nodes.reserve(body, fuel)?;
        let construction = nodes.reserve(item, fuel)?;
        let ty = expr(
            arena,
            nodes,
            construction,
            ExprKind::DeliveredType(Arc::new(record_type)),
            fuel,
        )?;
        let construction = arena.alloc_expr(Expr {
            hir_id: construction,
            kind: ExprKind::ExtendedApplication(ty, &[]),
            span: DUMMY_SP,
        });
        nodes.register(Node::Expr(construction), fuel)?;
        let item = arena.alloc_expr(Expr {
            hir_id: item,
            kind: ExprKind::Projection(construction, name("item")),
            span: DUMMY_SP,
        });
        nodes.register(Node::Expr(item), fuel)?;
        let body = arena.alloc_expr(Expr {
            hir_id: body,
            kind: ExprKind::Projection(item, name("flag")),
            span: DUMMY_SP,
        });
        Ok((
            FnSig {
                params: &[],
                return_ty: Some(result),
                return_bind: None,
                return_binding: None,
                modifiers: hir::item::FnModifiers {
                    is_pure: pure,
                    ..Default::default()
                },
                clause_params: &[],
                clause_constraints: &[],
                span: DUMMY_SP,
            },
            body,
        ))
    })
    .map_err(|e| e.to_string())?;
    batch
        .define(function.definition, Some(caller_id), None)
        .map_err(|e| e.to_string())?;
    batch
        .body(function.body_id, function.body)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn source_types_and_functions_feed_generated_definitions_and_runtime_execution() {
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "luna-generated-definitions-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let _cleanup = Cleanup(directory.clone());
    let input = directory.join("lib.fl");
    std::fs::write(
        &input,
        "trait Copy {} trait Drop { fn drop(*self); } struct Source { flag:bool } pure fn source()->Source { Source { flag:true } }",
    )
    .unwrap();
    let packages = [SourcePackage {
        resolution_id: 7,
        identity: 73,
        name: "input".into(),
        entry: input,
    }];
    let mut called = false;
    let exhausted = compile_with_definitions(
        &packages,
        &[],
        7,
        None,
        &Default::default(),
        Default::default(),
        None,
        &mut 0,
        &mut |_, _, _| {
            called = true;
            Ok(())
        },
    );
    assert!(exhausted.err().unwrap().contains("budget exhausted"));
    assert!(!called);
    for mode in 0..3 {
        let rejected = compile_with_definitions(
            &packages,
            &[],
            7,
            None,
            &Default::default(),
            Default::default(),
            None,
            &mut 100_000,
            &mut |arena, batch, fuel| {
                let owner = batch.reserve().map_err(|e| e.to_string())?;
                let identity = batch
                    .reserve_identity(if mode == 1 { 404 } else { 7 }, fuel)
                    .map_err(|e| e.to_string())?;
                let foreign = batch
                    .reserve_identity(404, fuel)
                    .map_err(|e| e.to_string())?;
                let info = hir::definitions::assemble_type_alias(
                    arena,
                    owner,
                    name("Invalid"),
                    DUMMY_SP,
                    fuel,
                    |nodes, fuel| {
                        let root = nodes.root();
                        let representation = expr(
                            arena,
                            nodes,
                            root,
                            ExprKind::Builtin(Symbol::intern("bool")),
                            fuel,
                        )?;
                        let clauses = if mode == 2 {
                            let clauses = arena.alloc_clause_param_slice([ClauseParam {
                                hir_id: nodes.reserve(root, fuel)?,
                                definition: Some(foreign),
                                name: name("T"),
                                kind: ClauseParamKind::Type(name("T")),
                                span: DUMMY_SP,
                            }]);
                            nodes.register(Node::ClauseParam(&clauses[0]), fuel)?;
                            clauses
                        } else {
                            &[]
                        };
                        Ok((representation, clauses, &[]))
                    },
                )
                .map_err(|e| e.to_string())?;
                batch
                    .define(info, (mode != 0).then_some(identity), None)
                    .map_err(|e| e.to_string())?;
                Ok(())
            },
        );
        let error = rejected.err().unwrap();
        let expected = [
            "requires an explicit package identity",
            "unknown source package 404",
            "clause parameter belongs to a different package",
        ][mode];
        assert!(error.contains(expected), "{error}");
    }
    let rejected = compile_with_definitions(
        &packages,
        &[],
        7,
        None,
        &Default::default(),
        Default::default(),
        None,
        &mut 100_000,
        &mut |arena, batch, fuel| expand(arena, batch, fuel, false, false),
    );
    assert!(rejected.err().unwrap().contains("elaboration failed"));
    let (artifact, _) = compile_with_definitions(
        &packages,
        &[],
        7,
        None,
        &Default::default(),
        Default::default(),
        None,
        &mut 100_000,
        &mut |arena, batch, fuel| expand(arena, batch, fuel, true, false),
    )
    .unwrap();
    let Artifact::Nir(package) = artifact else {
        panic!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let rejected = compile_with_definitions(
        &packages,
        &[],
        7,
        None,
        &Default::default(),
        Default::default(),
        None,
        &mut 100_000,
        &mut |arena, batch, fuel| expand(arena, batch, fuel, true, true),
    );
    assert!(rejected.err().unwrap().contains("P4 evidence"));
    let (artifact, protocols) = compile_with_definitions(
        &packages,
        &[],
        7,
        None,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("Copy"),
            drop: Some("Drop"),
            ..Default::default()
        },
        None,
        &mut 100_000,
        &mut |arena, batch, fuel| expand(arena, batch, fuel, true, true),
    )
    .unwrap();
    assert!(protocols.drop.is_some());
    let Artifact::Nir(package) = artifact else {
        panic!()
    };
    assert!(
        package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap()
            .pure
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    use std::os::unix::fs::PermissionsExt;
    let ir = luna::host::llvm_ir_with_protocols(
        &package,
        "run",
        100_000,
        protocols.copy.unwrap(),
        protocols.drop.unwrap(),
    )
    .unwrap();
    let executable = directory.join("run");
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let bytes = luna::host::executable(
            &ir,
            &luna::host::Toolchain {
                optimization,
                ..Default::default()
            },
        )
        .unwrap();
        std::fs::write(&executable, bytes).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new(&executable).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"(bool true)\n");
    }
    let provider_dir = directory.join("provider");
    std::fs::create_dir(&provider_dir).unwrap();
    let provider_entry = provider_dir.join("lib.fl");
    std::fs::write(
        &provider_entry,
        "pub struct Source { flag:bool } pub pure fn source()->Source { Source { flag:true } }",
    )
    .unwrap();
    std::fs::write(
        &packages[0].entry,
        "trait Copy {} trait Drop {fn drop(*self);} use provider.Source; use provider.source;",
    )
    .unwrap();
    let packages = [
        packages[0].clone(),
        SourcePackage {
            resolution_id: 11,
            identity: 91,
            name: "provider".into(),
            entry: provider_entry,
        },
    ];
    let dependencies = [resolve::DependencyEdge {
        requester: 7,
        alias: "provider".into(),
        provider: 11,
    }];
    let (artifact, _) = compile_with_definitions(
        &packages,
        &dependencies,
        7,
        None,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("Copy"),
            drop: Some("Drop"),
            ..Default::default()
        },
        None,
        &mut 100_000,
        &mut |arena, batch, fuel| expand(arena, batch, fuel, true, true),
    )
    .unwrap();
    let Artifact::Nir(package) = artifact else {
        panic!()
    };
    let source = package
        .structs
        .iter()
        .find(|definition| definition.name == "Source")
        .unwrap();
    let generated = package
        .structs
        .iter()
        .find(|definition| definition.name == "Generated")
        .unwrap();
    assert_eq!(source.declaration.package, 91);
    assert_eq!(generated.declaration.package, 73);
    assert_eq!(
        generated.fields[0].1,
        nir::Term::Nominal {
            declaration: source.declaration,
            arguments: vec![]
        }
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    std::fs::remove_dir_all(directory).unwrap();
}
