#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

fn compile(source: &str, equality: Option<&str>) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            equality,
            ..Default::default()
        },
    )
}

#[test]
fn actual_eq_defaults_support_nominal_and_generic_operators() {
    let source=format!("mod comparison {{\n{}\n}} mod resources {{\n{}\n}}
        struct Key {{value:.u32'builtin}}
        impl comparison.Eq for Key {{fn eq(self,other:Self)->bool {{self.value==other.value}}}}
        fn same(a:T,b:T)->bool where T :- comparison.Eq {{a==b}}
        fn distinct(a:T,b:T)->bool where T :- comparison.Eq {{a!=b}}
        fn run()->bool {{same(Key {{value:7}},Key {{value:7}}) and distinct(Key {{value:7}},Key {{value:9}})
            and not (Key {{value:1}} != Key {{value:1}}) and not (Key {{value:1}} == Key {{value:2}})}}
        fn verify() {{let valid=run();assumes valid;}}",include_str!("../../../library/builtin/comparison.fl"),include_str!("../../../library/builtin/resources.fl"));
    assert!(compile(&source, None).is_err());
    let package = compile(&source, Some("input::comparison::Eq")).unwrap();
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
fn equality_preserves_operand_order_and_calls_overridden_ne() {
    let source="trait Compare {fn eq(self,other:Self)->bool;fn ne(self,other:Self)->bool;}
        struct Key {value:bool,called:*bool}
        impl Compare for Key {
            fn eq(self,other:Self)->bool {self.value==other.value}
            fn ne(self,other:Self)->bool {self.called.* = true;self.value!=other.value}
        }
        fn run()->bool {
            var phase:.u8'builtin=0;var called=false;
            let different=do {assumes phase==0;phase=1;Key {value:false,called:called.ref}}
                != do {assumes phase==1;phase=2;Key {value:true,called:called.ref}};
            different and called and phase==2
        }
        fn escape()->bool {var called=false;Key {value:false,called:called.ref} != do {return true}}
        fn left_escape()->bool {var called=false;do {return true} == Key {value:false,called:called.ref}}";
    let package = compile(source, Some("input::Compare")).unwrap();
    for name in ["run", "escape", "left_escape"] {
        assert_eq!(
            luna::execution::interpret(&package, name, 100000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn equality_binding_does_not_override_type_identity_or_scalar_equality() {
    let source = "trait Compare {fn eq(self,other:Self)->bool;fn ne(self,other:Self)->bool;}
        pure comptime fn Identity<T>->.Type'builtin {T}
        comptime fn identity_is_opaque()->bool {Identity<.u32'builtin> != .u32'builtin}
        comptime fn reflexive()->bool {Identity<.u32'builtin> == Identity<.u32'builtin>}
        fn run()->bool {let n:.u32'builtin=1;true==true and n==1 and \"a\"!=\"b\"}";
    let package = compile(source, Some("input::Compare")).unwrap();
    let error = luna::invocation::invoke(&package, "identity_is_opaque", 100000).unwrap_err();
    assert!(error.contains("TypeContractUnresolved"), "{error}");
    assert_eq!(
        luna::invocation::invoke(&package, "reflexive", 100000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn equality_rejects_missing_implementation_and_wrong_result() {
    for source in [
        "trait Compare {fn eq(self,other:Self)->bool;} struct Key {} fn run()->bool {Key {}==Key {}}",
        "trait Compare {fn eq(self,other:Self)->.u8'builtin;} struct Key {} impl Compare for Key {fn eq(self,other:Self)->.u8'builtin {1}} fn run()->bool {Key {}==Key {}}",
    ] {
        assert!(compile(source, Some("input::Compare")).is_err());
    }
}

#[test]
fn equality_cli_requires_one_binding() {
    fn parse<const N: usize>(args: [&str; N]) -> Result<luna::cli::Command, String> {
        luna::cli::parse(args.into_iter().map(Into::into))
    }
    let luna::cli::Command::Compile(options) = parse([
        "input.fl",
        "--emit",
        "nir",
        "--eq-protocol",
        "input::Compare",
    ])
    .unwrap() else {
        panic!("expected compile")
    };
    assert_eq!(options.eq_protocol.as_deref(), Some("input::Compare"));
    assert!(parse(["input.fl", "--eq-protocol"]).is_err());
    assert!(
        parse([
            "input.fl",
            "--emit",
            "nir",
            "--eq-protocol",
            "input::Compare",
            "--eq-protocol",
            "input::Other"
        ])
        .is_err()
    );
}
