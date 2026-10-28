//! Source comptime reflection feeds ordinary list transformations and type inputs.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn struct_fields_are_observed_in_order_with_applied_generic_types() {
    let package = compile(
        "
        typealias Type = .Type'builtin;
        typealias Symbol = .Symbol'builtin;
        struct Sample where T { first: T, second: bool }
        struct Empty {}
        comptime fn run() -> bool {
            let types = meta.struct_field_types(Sample<.u32'builtin>);
            let names = meta.struct_field_names(Sample<.u32'builtin>);
            types.length() == 2 and types[0] == .u32'builtin and types[1] == bool
                and names.length() == 2 and names[0] == .first and names[1] == .second
                and (meta.struct_field_types(Empty)).length() == 0
                and (meta.struct_field_names(Empty)).length() == 0
        }
        pure comptime fn Packet<...fields: meta.List<Type>> -> Type {Packet<...fields>}
        pure comptime fn Generated() -> Type {
            Packet<...meta.struct_field_types(Sample<bool>)>
        }
        comptime fn Direct() -> Type {Packet<bool,bool>}
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Generated", 500_000).unwrap(),
        luna::invocation::invoke(&package, "Direct", 500_000).unwrap()
    );
    let dump = package.to_string();
    assert!(dump.contains("(struct-field-types "), "{dump}");
    assert!(dump.contains("(struct-field-names "), "{dump}");
}

#[test]
fn reflection_evaluates_the_subject_once_and_preserves_opaque_fields() {
    let package = compile(
        "
        typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        pure comptime fn Identity<T> -> Type {T}
        struct Sample { field: Identity<bool> }
        comptime fn Selected(count: *Integer) -> Type {
            count.* = count.* + 1; Sample
        }
        comptime fn run() -> bool {
            var count: Integer = 0;
            let types = meta.struct_field_types(Selected(count.ref));
            let names = meta.struct_field_names(Selected(count.ref));
            count == 2 and types[0] == Identity<bool> and names[0] == .field
        }
        comptime fn bad() -> Type {meta.struct_field_types(Identity<Sample>)[0]}
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    assert!(luna::invocation::invoke(&package, "bad", 500_000).is_err());
}

#[test]
fn reflection_rejects_non_structs_wrong_operands_and_runtime_invocation() {
    for source in [
        "comptime fn bad() {meta.struct_field_types(true);}",
        "fn bad() {meta.struct_field_types(bool);}",
        "comptime fn bad() {.struct_field_names'builtin();}",
        "comptime fn bad() {.struct_field_types'builtin(bool, bool);}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    let package = compile(
        "
        comptime fn bad() {meta.struct_field_types(bool);}
        struct Good { flag: bool }
        comptime fn good() -> bool {meta.struct_field_types(Good)[0] == bool}
    ",
    )
    .unwrap();
    assert!(luna::invocation::invoke(&package, "bad", 100_000).is_err());
    assert_eq!(
        luna::invocation::invoke(&package, "good", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn reflected_types_participate_in_required_elaboration_and_runtime_results() {
    let package = compile(
        "
        typealias Type = .Type'builtin;
        struct Flags { flag: bool }
        pure comptime fn First() -> Type {meta.struct_field_types(Flags)[0]}
        typealias Reflected = First();
        fn entry() -> Reflected {true}
        comptime fn early() -> bool {
            .struct_field_types'builtin(do {return true;}); false
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "entry", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "early", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_package_observes_fields_through_meta_without_builtin_access() {
    let directory =
        std::env::temp_dir().join(format!("luna-source-reflection-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let input = directory.join("input.fl");
    std::fs::write(
        &input,
        "
        struct Flags { enabled: bool }
        comptime fn run() -> bool {
            let names = meta.struct_field_names(Flags);
            let types = meta.struct_field_types(Flags);
            names.length() == 1 and names[0] == .enabled and types[0] == bool
        }
    ",
    )
    .unwrap();
    let library =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/meta/lib.fl");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .args(["--packages-v1", "--package", "1", "101", "app"])
        .arg(&input)
        .args(["--builtin-impl-package", "2"])
        .args(["--package", "2", "202", "meta"])
        .arg(&library)
        .args(["--package", "3", "303", "builtin"])
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../library/builtin/primitives.fl"),
        )
        .args(["--dependency", "2", "builtin", "3"])
        .args(nir::intrinsics::Operation::ALL.iter().flat_map(|op| {
            [
                "--comptime-intrinsic".to_owned(),
                format!(
                    "{}=meta.numeric.scalar_math.target.Scalar.{}",
                    op.name(),
                    op.name()
                ),
            ]
        }))
        .args([
            "--dependency",
            "1",
            "meta",
            "2",
            "--primary",
            "1",
            "--meta-list-type",
            "meta.List",
            "--invoke-comptime",
            "run",
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool true)"
    );
}

#[test]
fn existing_source_invocation_can_observe_a_newly_published_struct_in_the_same_vm() {
    let source = "
        typealias Type = .Type'builtin;
        comptime fn First(subject: Type) -> Type {meta.struct_field_types(subject)[0]}
    ";
    let baseline = compile(source).unwrap();
    let published = compile(&format!("{source} struct Generated {{flag:bool}}")).unwrap();
    let generated = published
        .structs
        .iter()
        .find(|item| item.name == "Generated")
        .unwrap();
    let entry = baseline
        .functions
        .iter()
        .find(|item| item.name == "First")
        .unwrap()
        .declaration;
    let ty = nir::Term::Nominal {
        declaration: generated.declaration,
        arguments: vec![],
    };
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&baseline, Default::default()).unwrap();
    assert!(vm.invoke(entry, vec![Value::Type(ty.clone())]).is_err());
    let before = vm.remaining_steps();
    vm.install_generated_structs(std::slice::from_ref(generated))
        .unwrap();
    assert!(vm.remaining_steps() < before);
    assert_eq!(
        vm.invoke(entry, vec![Value::Type(ty)]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn source_invocation_retains_generated_descriptors_after_publisher_storage_is_dropped() {
    let source = "
        typealias Type = .Type'builtin;
        comptime fn First(subject: Type) -> Type {meta.struct_field_types(subject)[0]}
    ";
    let baseline = compile(source).unwrap();
    let entry = baseline
        .functions
        .iter()
        .find(|item| item.name == "First")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&baseline, Default::default()).unwrap();
    let (ty, witness) = {
        let published = compile(&format!("{source} struct Generated {{flag:bool}}")).unwrap();
        let generated = published
            .structs
            .into_iter()
            .find(|item| item.name == "Generated")
            .unwrap();
        let ty = nir::Term::Nominal {
            declaration: generated.declaration,
            arguments: vec![],
        };
        let definitions: std::sync::Arc<[nir::program::Struct]> = vec![generated].into();
        let witness = std::sync::Arc::downgrade(&definitions);
        vm.install_owned_generated_structs_with(definitions, || Ok::<_, ()>(()))
            .unwrap();
        (ty, witness)
    };
    assert_eq!(
        vm.invoke(entry, vec![Value::Type(ty)]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert!(witness.upgrade().is_some());
    drop(vm);
    assert!(witness.upgrade().is_none());
}

#[test]
fn source_struct_build_uses_checked_publication_without_replaying_argument_effects() {
    use comptime_interpretor::{PreparedStructBuild, StructBuildRequest};
    use hir::{HirArena, Ident, Symbol};
    use rustc_span::DUMMY_SP;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
        sync::Arc,
    };
    let package = compile(
        "
        typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        comptime fn run() -> bool {
            var count: Integer = 0;
            let built = .build_struct'builtin(
                do { count = count * 10 + 1; .Generated },
                do { count = count * 10 + 2; [.flag] },
                do { count = count * 10 + 3; [bool] });
            count == 123 and meta.struct_field_types(built)[0] == bool
                and meta.struct_field_names(built)[0] == .flag
        }
    ",
    )
    .unwrap();
    assert!(package.to_string().contains("(build-struct "));
    let entry = package
        .functions
        .iter()
        .find(|item| item.name == "run")
        .unwrap()
        .declaration;
    let arena = HirArena::new();
    let published = RefCell::new(hir::Package::new());
    let calls = Cell::new(0);
    let commits = Cell::new(0);
    let builder = |request: &StructBuildRequest, steps: &mut u64| {
        calls.set(calls.get() + 1);
        assert_eq!(published.borrow().num_defs(), 0);
        assert!(request.to_string().contains("(name \"Generated\")"));
        let mut candidate = published.borrow().clone();
        let mut batch = hir::definitions::DefinitionBatch::new(&mut candidate);
        let owner = batch.reserve().unwrap();
        let fields: Vec<_> = request
            .fields
            .iter()
            .map(|field| hir::definitions::TypedField {
                name: Ident::new(Symbol::intern(&field.name), DUMMY_SP),
                ty: Arc::new(field.ty.clone()),
                span: DUMMY_SP,
            })
            .collect();
        let mut fuel = 100_000;
        let info = hir::definitions::assemble_struct(
            &arena,
            owner,
            Ident::new(Symbol::intern(&request.name), DUMMY_SP),
            &fields,
            DUMMY_SP,
            &mut fuel,
        )
        .unwrap();
        batch.define(info, None, None).unwrap();
        let (_, checked) = elaboration::definitions::publish_with_invocation_budget(
            &mut batch,
            &mut fuel,
            steps,
            &BTreeMap::from([(0, 77123)]),
            &Default::default(),
            None,
        )
        .unwrap();
        let result = checked.structs[0].declaration;
        let published = &published;
        let commits = &commits;
        Ok(PreparedStructBuild {
            definitions: checked.structs.into(),
            result,
            commit: Box::new(move || {
                *published.borrow_mut() = candidate;
                commits.set(commits.get() + 1);
                Ok(())
            }),
        })
    };
    let mut vm = comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
        .unwrap()
        .with_struct_builder(&builder);
    assert_eq!(vm.invoke(entry, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(calls.get(), 1);
    assert_eq!(commits.get(), 1);
    assert_eq!(published.borrow().num_defs(), 1);
}

#[test]
fn source_struct_build_rejects_runtime_wrong_inputs_and_missing_service() {
    for source in [
        "fn bad() -> .Type'builtin {.build_struct'builtin(.Generated, [.flag], [bool])}",
        "comptime fn Bad() -> .Type'builtin {.build_struct'builtin(true, [.flag], [bool])}",
        "comptime fn Bad() -> .Type'builtin {.build_struct'builtin(.Generated, [.flag])}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    let package = compile(
        "comptime fn Generate() -> .Type'builtin {.build_struct'builtin(.Generated, [.flag], [bool])}",
    )
    .unwrap();
    assert!(luna::invocation::invoke(&package, "Generate", 500_000).is_err());
}

#[test]
fn source_struct_build_rejection_does_not_commit_and_can_retry_in_the_same_vm() {
    use comptime_interpretor::{Error, ErrorKind, PreparedStructBuild, StructBuildRequest};
    use std::cell::Cell;
    let package = compile("
        comptime fn Generate() -> .Type'builtin {.build_struct'builtin(.Generated, [.flag], [bool])}
        comptime fn Mismatch() -> .Type'builtin {.build_struct'builtin(.Generated, [.flag], [])}
        comptime fn Duplicate() -> .Type'builtin {.build_struct'builtin(.Generated, [.flag, .flag], [bool, bool])}
    ").unwrap();
    let entry = |name| {
        package
            .functions
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .declaration
    };
    let mode = Cell::new(0);
    let calls = Cell::new(0);
    let commits = Cell::new(0);
    let builder = |request: &StructBuildRequest, steps: &mut u64| {
        calls.set(calls.get() + 1);
        *steps = steps.saturating_sub(7);
        let fail = Error {
            kind: ErrorKind::Type,
            source: Some(request.source),
        };
        if mode.get() == 0 {
            return Err(fail);
        }
        let result = nir::DeclarationId {
            package: 77124,
            local: 0,
        };
        let definition = nir::program::Struct {
            declaration: result,
            name: if mode.get() == 1 {
                "Wrong".into()
            } else {
                request.name.clone()
            },
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            fields: request
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect(),
        };
        let mode = &mode;
        let commits = &commits;
        Ok(PreparedStructBuild {
            definitions: vec![definition].into(),
            result,
            commit: Box::new(move || {
                if mode.get() == 2 {
                    return Err(fail);
                }
                commits.set(commits.get() + 1);
                Ok(())
            }),
        })
    };
    let mut vm = comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
        .unwrap()
        .with_struct_builder(&builder);
    for name in ["Mismatch", "Duplicate"] {
        assert!(vm.invoke(entry(name), vec![]).is_err());
    }
    assert_eq!(calls.get(), 0);
    for failure in 0..3 {
        mode.set(failure);
        let before = vm.remaining_steps();
        assert!(vm.invoke(entry("Generate"), vec![]).is_err());
        assert!(vm.remaining_steps() < before);
        assert_eq!(commits.get(), 0);
    }
    mode.set(3);
    let Value::Type(generated) = vm.invoke(entry("Generate"), vec![]).unwrap() else {
        panic!("expected Type")
    };
    assert_eq!(
        vm.reflect_struct(&generated).unwrap().fields[0].name,
        "flag"
    );
    assert_eq!(calls.get(), 4);
    assert_eq!(commits.get(), 1);
}

#[test]
fn source_struct_build_shares_budget_and_does_not_request_after_early_return() {
    use comptime_interpretor::{PreparedStructBuild, StructBuildRequest};
    use std::cell::Cell;
    let package = compile(
        "
        comptime fn Generate() -> .Type'builtin {.build_struct'builtin(.Generated, [.flag], [bool])}
        comptime fn early() -> bool {
            .build_struct'builtin(do {return true;}, [.flag], [bool]); false
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "early", 500_000).unwrap(),
        Value::Bool(true)
    );
    let entry = package
        .functions
        .iter()
        .find(|item| item.name == "Generate")
        .unwrap()
        .declaration;
    let mode = Cell::new(0);
    let commits = Cell::new(0);
    let builder = |request: &StructBuildRequest, steps: &mut u64| {
        if mode.get() == 2 {
            panic!("host preparation failed")
        }
        *steps = if mode.get() == 0 { 0 } else { u64::MAX };
        let result = nir::DeclarationId {
            package: 77125,
            local: 0,
        };
        let definition = nir::program::Struct {
            declaration: result,
            name: request.name.clone(),
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            fields: request
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect(),
        };
        let commits = &commits;
        Ok(PreparedStructBuild {
            definitions: vec![definition].into(),
            result,
            commit: Box::new(move || {
                commits.set(commits.get() + 1);
                Ok(())
            }),
        })
    };
    for current in 0..3 {
        mode.set(current);
        let mut vm = comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
            .unwrap()
            .with_struct_builder(&builder);
        let before = vm.remaining_steps();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.invoke(entry, vec![])));
        match current {
            0 => {
                assert!(result.unwrap().is_err());
                assert_eq!(commits.get(), 0);
                assert_eq!(vm.remaining_steps(), 0);
            }
            1 => {
                assert!(result.unwrap().is_ok());
                assert_eq!(commits.get(), 1);
                assert!(vm.remaining_steps() < before);
            }
            _ => {
                assert!(result.is_err());
                assert_eq!(commits.get(), 1);
                assert_eq!(vm.remaining_steps(), 0);
            }
        }
    }
}

#[test]
fn source_builds_chain_through_the_compiler_publisher_and_keep_final_nir() {
    use std::collections::BTreeMap;
    let package = compile(
        "
        comptime fn Generate() -> .Type'builtin {
            let first = .build_struct'builtin(.First, [.flag], [bool]);
            .build_struct'builtin(.Second, [.nested], [first])
        }
    ",
    )
    .unwrap();
    let entry = package
        .functions
        .iter()
        .find(|item| item.name == "Generate")
        .unwrap()
        .declaration;
    let arena = hir::HirArena::new();
    let publisher = elaboration::definitions::StructPublisher::new(
        &arena,
        hir::Package::new(),
        0,
        BTreeMap::from([(0, 79123)]),
        Default::default(),
        None,
        100_000,
    )
    .unwrap();
    let generated = {
        let builder = |request: &comptime_interpretor::StructBuildRequest, steps: &mut u64| {
            publisher.prepare(request, steps)
        };
        let mut vm = comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
            .unwrap()
            .with_struct_builder(&builder);
        let Value::Type(generated) = vm.invoke(entry, vec![]).unwrap() else {
            panic!("expected Type")
        };
        let nested = vm.reflect_struct(&generated).unwrap().fields[0].ty.clone();
        assert_eq!(
            vm.reflect_struct(&nested).unwrap().fields[0].ty,
            nir::Term::Primitive(nir::Primitive::Bool)
        );
        generated
    };
    let (hir, checked) = publisher.finish();
    assert_eq!(hir.num_defs(), 2);
    let checked = checked.unwrap();
    let mut next =
        comptime_interpretor::Interpreter::for_comptime(&checked, Default::default()).unwrap();
    let nested = next.reflect_struct(&generated).unwrap().fields[0]
        .ty
        .clone();
    assert_eq!(
        next.reflect_struct(&nested).unwrap().fields[0].ty,
        nir::Term::Primitive(nir::Primitive::Bool)
    );
}
