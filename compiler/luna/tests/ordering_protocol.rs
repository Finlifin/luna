#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

fn compile(source: &str, ordering: Option<&str>) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            ordering,
            ..Default::default()
        },
    )
}

#[test]
fn real_ord_defaults_drive_nominal_and_generic_operators() {
    let source = format!("mod comparison {{\n{}\n}}
        struct Rank {{value:.i32'builtin}}
        impl comparison.Eq for Rank {{fn eq(self,other:Self)->bool {{self.value==other.value}}}}
        impl comparison.Ord for Rank {{
            fn cmp(self,other:Self)->comparison.Ordering {{
                if self.value < other.value {{.Less}} else if self.value == other.value {{.Equal}} else {{.Greater}}
            }}
        }}
        fn before(a:T,b:T)->bool where T :- comparison.Ord {{a < b}}
        fn run()->bool {{
            before(Rank {{value:1}},Rank {{value:2}})
                and Rank {{value:2}} <= Rank {{value:2}}
                and Rank {{value:3}} > Rank {{value:2}}
                and Rank {{value:2}} >= Rank {{value:2}}
                and not (Rank {{value:3}} < Rank {{value:2}})
                and numeric(1.0,2.0)
        }} fn numeric(a:.f64'builtin,b:.f64'builtin)->bool {{a < b}} fn verify() {{let valid=run();assumes valid;}}",include_str!("../../../library/builtin/comparison.fl"));
    let source = format!(
        "mod resources {{\n{}\n}}\n{source}",
        include_str!("../../../library/builtin/resources.fl")
    );
    assert!(compile(&source, None).is_err());
    let package = compile(&source, Some("input.comparison.Ord")).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .unwrap()
        .declaration;
    let ir = luna::host::llvm_ir_with_protocols(&package, "verify", 1000000, copy, drop).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn ordering_calls_preserve_left_right_order_and_divergence() {
    let source="trait Compare {fn lt(self,other:Self)->bool;}
        struct Rank {value:.i32'builtin}
        impl Compare for Rank {fn lt(self,other:Self)->bool {self.value < other.value}}
        fn run()->bool {
            var phase:.u8'builtin=0;
            let ordered=do {assumes phase==0;phase=1;Rank {value:1}} < do {assumes phase==1;phase=2;Rank {value:2}};
            ordered and phase==2
        }
        fn escape()->bool {Rank {value:1} < do {return true}}
        fn left_escape()->bool {do {return true} < Rank {value:2}}";
    let package = compile(source, Some("input.Compare")).unwrap();
    for entry in ["run", "escape", "left_escape"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn ordering_does_not_use_unbound_or_wrong_result_protocols() {
    for source in [
        "trait Compare {fn lt(self,other:Self)->.u8'builtin;} struct Rank {} impl Compare for Rank {fn lt(self,other:Self)->.u8'builtin {1}} fn run()->bool {Rank {} < Rank {}}",
        "trait Compare {fn lt(self,other:Self)->bool;} struct Rank {} fn run()->bool {Rank {} < Rank {}}",
    ] {
        assert!(compile(source, Some("input.Compare")).is_err());
    }
}

#[test]
fn cli_accepts_qualified_ord_binding_and_rejects_duplicates() {
    fn parse<const N: usize>(args: [&str; N]) -> Result<luna::cli::Command, String> {
        luna::cli::parse(args.into_iter().map(Into::into))
    }
    let luna::cli::Command::Compile(options) = parse([
        "input.fl",
        "--emit",
        "nir",
        "--ord-protocol",
        "input.Compare",
    ])
    .unwrap() else {
        panic!("compile expected")
    };
    assert_eq!(options.ord_protocol.as_deref(), Some("input.Compare"));
    assert!(parse(["input.fl", "--ord-protocol"]).is_err());
    assert!(
        parse([
            "input.fl",
            "--emit",
            "nir",
            "--ord-protocol",
            "input.Compare",
            "--ord-protocol",
            "input.Other"
        ])
        .is_err()
    );
}
