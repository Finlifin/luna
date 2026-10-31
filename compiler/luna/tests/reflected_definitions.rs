//! Source invocation and reflected fields feed ordinary generated declarations.
use comptime_interpretor::{Budget, Interpreter, Value};
use hir::definitions::{TypedField, assemble_struct};
use luna::{
    frontend::Artifact,
    packages::{SourcePackage, compile_with_definitions},
};
use rustc_span::DUMMY_SP;
use std::{collections::BTreeMap, sync::Arc};

#[test]
fn source_comptime_selection_drives_generated_field_layouts_without_reparsing() {
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
        "luna-reflected-definitions-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let _cleanup = Cleanup(directory.clone());
    let input = directory.join("lib.fl");
    std::fs::write(
        &input,
        "
        typealias bool = .bool'builtin;
        typealias char = .char'builtin;
        typealias Type = .Type'builtin;
        struct Flags { first:bool, second:bool }
        struct Letter { code:char }
        pure comptime fn Selected(flag:bool)->Type {
            if flag {Flags} else {Letter}
        }
    ",
    )
    .unwrap();
    let packages = [SourcePackage {
        resolution_id: 17,
        identity: 900,
        name: "input".into(),
        entry: input,
    }];
    let identities = BTreeMap::from([(17, 900)]);
    let mut budget = Budget {
        steps: 100_000,
        depth: 128,
    };
    for selected in [Some(true), None, Some(false), Some(true)] {
        let before = budget.steps;
        let mut calls = 0;
        let result = compile_with_definitions(
            &packages,
            &[],
            17,
            None,
            &Default::default(),
            Default::default(),
            None,
            &mut 100_000,
            &mut |arena, batch, fuel| {
                calls += 1;
                let checked = elaboration::program::elaborate_packages_with_invocation_budget(
                    batch.published(),
                    &identities,
                    &Default::default(),
                    None,
                    &mut budget.steps,
                )
                .map_err(|error| error.to_string())?;
                let entry = checked
                    .functions
                    .iter()
                    .find(|function| function.name == "Selected")
                    .unwrap()
                    .declaration;
                let reflected =
                    Interpreter::with_comptime_in_budget(&checked, &[], &mut budget, |vm| {
                        let Value::Type(ty) = vm.invoke(
                            entry,
                            vec![selected.map(Value::Bool).unwrap_or(Value::Unit)],
                        )?
                        else {
                            panic!("Type result expected")
                        };
                        vm.reflect_struct(&ty)
                    })
                    .map_err(|error| error.to_string())?;
                let fields: Vec<_> = reflected
                    .fields
                    .into_iter()
                    .map(|field| TypedField {
                        name: hir::Ident::new(hir::Symbol::intern(&field.name), DUMMY_SP),
                        ty: Arc::new(field.ty),
                        span: DUMMY_SP,
                    })
                    .collect();
                let owner = batch.reserve().map_err(|error| error.to_string())?;
                let identity = batch
                    .reserve_identity(17, fuel)
                    .map_err(|error| error.to_string())?;
                let generated = assemble_struct(
                    arena,
                    owner,
                    hir::Ident::new(hir::Symbol::intern("Generated"), DUMMY_SP),
                    &fields,
                    DUMMY_SP,
                    fuel,
                )
                .map_err(|error| error.to_string())?;
                batch
                    .define(generated, Some(identity), None)
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        );
        assert_eq!(calls, 1, "{:?}", result.as_ref().err());
        assert!(budget.steps < before);
        let Some(selected) = selected else {
            assert!(result.err().unwrap().contains("Type"));
            continue;
        };
        let (artifact, _) = result.unwrap();
        let Artifact::Nir(package) = artifact else {
            panic!()
        };
        let generated = package
            .structs
            .iter()
            .find(|definition| definition.name == "Generated")
            .unwrap();
        let source_name = if selected { "Flags" } else { "Letter" };
        let source = package
            .structs
            .iter()
            .find(|definition| definition.name == source_name)
            .unwrap();
        assert_eq!(generated.fields, source.fields);
        assert_ne!(generated.declaration, source.declaration);
        assert_eq!(generated.declaration.package, 900);
        assert_eq!(generated.fields.len(), if selected { 2 } else { 1 });
        let mut destination_budget = Budget::default();
        let reflected =
            Interpreter::with_comptime_in_budget(&package, &[], &mut destination_budget, |vm| {
                vm.reflect_struct(&nir::Term::Nominal {
                    declaration: generated.declaration,
                    arguments: vec![],
                })
            })
            .unwrap();
        assert_eq!(reflected.fields.len(), generated.fields.len());
        for (field, (name, ty)) in reflected.fields.iter().zip(&generated.fields) {
            assert_eq!(&field.name, name);
            assert_eq!(&field.ty, ty);
        }
    }
}
