//! Real sysroot lists are ordinary frozen NF inputs for Type model search.
use elaboration::ptc::{Budget, Context, Formula, Outcome, TypeInputOutcome};
use luna::{
    cli::{self, Command},
    frontend::{self, Artifact},
};
use nir::{DeclarationId, Primitive, Sort, Term};

#[test]
fn shipped_lists_supply_scalar_type_and_nested_payload_models() {
    let path = std::env::temp_dir().join(format!(
        "luna-opaque-payload-models-{}.fl",
        std::process::id()
    ));
    std::fs::write(
        &path,
        r#"
pure comptime fn Carrier<T, values:meta.List<T>>->Type { bool }
fn scalars(value:Carrier<bool,[true,false]>)->bool { true }
fn types(value:Carrier<Type,[bool,u8]>)->bool { true }
fn nested(value:Carrier<meta.List<Type>,[[bool],[u8]]>)->bool { true }
comptime fn Describe()->Type {
    var values:meta.List<Type> = [bool];
    values.append(u8);
    Carrier<Type,values>
}
"#,
    )
    .unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("check command")
    };
    let result = frontend::compile(&options);
    std::fs::remove_file(&path).unwrap();
    let Artifact::Nir(package) = result.unwrap() else {
        panic!("NIR")
    };
    let context = Context::from_package(&package).unwrap();
    let input = DeclarationId {
        package: u64::MAX,
        local: 10,
    };
    for name in ["scalars", "types", "nested"] {
        let function = package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let Term::Function { parameters, .. } = &function.signature else {
            panic!("function")
        };
        let expected = &parameters[0].ty;
        let Term::NormalForm { arguments, .. } = expected else {
            panic!("opaque Carrier")
        };
        assert!(
            matches!(&arguments[1], Term::NominalValue { ty, .. } if matches!(ty.as_ref(), Term::NormalForm { .. }))
        );
        let constraint = Formula::equal(
            Term::Rigid {
                declaration: input,
                sort: Sort::Type,
            },
            expected.clone(),
        );
        let outcome =
            context.search_type_inputs(&[input], constraint.clone(), &mut Budget::default());
        let TypeInputOutcome::Unique(solution) = outcome else {
            panic!("{name}: {outcome}")
        };
        assert_eq!(&solution.bindings[&input].0, expected);
        let problem = context
            .type_input_context(&[input], constraint, &mut Budget::default())
            .unwrap();
        problem
            .verify_witness(&solution.model, Budget::default())
            .unwrap();
        problem
            .verify_proof(&solution.bindings[&input].1, Budget::default())
            .unwrap();
        let outcome = context.prove(
            Formula::equal(expected.clone(), Term::Primitive(Primitive::Bool)),
            Budget::default(),
        );
        let Outcome::NotEntailed(witness) = outcome else {
            panic!("body must not unfold: {outcome}")
        };
        context.verify_witness(&witness, Budget::default()).unwrap();
        if name == "types" {
            assert_eq!(
                luna::invocation::invoke(&package, "Describe", 1_000_000).unwrap(),
                comptime_interpretor::Value::Type(expected.clone())
            );
        }
    }
}
