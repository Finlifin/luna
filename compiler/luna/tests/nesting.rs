use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};

// Isolate stack failures from the parent runner while retaining the normal
// test-thread stack size. Exercise the public API, not a larger-stack wrapper.
#[test]
#[ignore = "invoked in subprocesses by the nesting regression"]
fn nested_expression_child() {
    let stage = std::env::var("LUNA_NESTING_PROBE_STAGE").unwrap();
    let case = std::env::var("LUNA_NESTING_PROBE_CASE").unwrap();
    let emit = match stage.as_str() {
        "ast" => Emit::Ast,
        "hir" => Emit::Hir,
        _ => Emit::Nir,
    };
    let source = match case.as_str() {
        "not" => format!(
            "fn run()->bool {{{}true{}}}",
            "not (".repeat(32),
            ")".repeat(32)
        ),
        "and" => format!(
            "fn run()->bool {{{}true{}}}",
            "true and (".repeat(32),
            ")".repeat(32)
        ),
        "or" => format!(
            "fn run()->bool {{not ({}false{})}}",
            "false or (".repeat(32),
            ")".repeat(32)
        ),
        "mixed-logic" => {
            let body = (0..32).fold("true".to_owned(), |body, _| {
                format!("not (false or ({body}))")
            });
            format!("fn run()->bool {{{body}}}")
        }
        "equal" => {
            let body = (0..32).fold("true".to_owned(), |body, _| format!("({body}) == true"));
            format!("fn run()->bool {{{body}}}")
        }
        "machine-logic" => format!(
            "typealias Byte=.u8'builtin;fn run()->bool {{let value: Byte = 1; let result = value + 1; {}true{} and result == 2}}",
            "not (".repeat(32),
            ")".repeat(32)
        ),
        "negative" => format!(
            "typealias Tiny=.i8'builtin;fn nested()->Tiny {{ {}1{} }}fn run()->bool {{nested()==1}}",
            "-(".repeat(32),
            ")".repeat(32)
        ),
        "bits" => format!(
            "typealias Bits=.BitVec'builtin<8>;fn nested()->Bits {{{}0{}}}fn run()->bool {{nested()==0}}",
            "not (".repeat(32),
            ")".repeat(32)
        ),
        "concat" => format!(
            "typealias Bit=.BitVec'builtin<1>;typealias Word=.BitVec'builtin<32>;fn nested()->Word {{let zero:Bit=0;{}zero{}}}fn run()->bool {{nested()==0}}",
            "zero ++ (".repeat(31),
            ")".repeat(31)
        ),
        "open-not" => format!(
            "fn nested(comptime flag:bool)->bool {{{}(do {{inline if flag {{true}} else {{false}}}}){}}}fn run()->bool {{nested(true)}}",
            "not (".repeat(32),
            ")".repeat(32)
        ),
        "open-arithmetic" => format!(
            "typealias Byte=.u8'builtin;fn nested(comptime flag:bool)->Byte {{{}(do {{inline if flag {{0}} else {{1}}}}){}}}fn run()->bool {{nested(true)==0}}",
            "0 + (".repeat(32),
            ")".repeat(32)
        ),
        "open-compare" => {
            let body = (0..32).fold(
                "(do {inline if flag {true} else {false}})".to_owned(),
                |body, _| format!("({body}) == true"),
            );
            format!("fn nested(comptime flag:bool)->bool {{{body}}}fn run()->bool {{nested(true)}}")
        }
        "open-concat" => format!(
            "typealias Bit=.BitVec'builtin<1>;typealias Word=.BitVec'builtin<32>;fn nested(comptime flag:bool)->Word {{let zero:Bit=0;{}(do {{inline if flag {{zero}} else {{zero}}}}){}}}fn run()->bool {{nested(true)==0}}",
            "zero ++ (".repeat(31),
            ")".repeat(31)
        ),
        _ => panic!("unknown case"),
    };
    let path = std::env::temp_dir().join(format!(
        "luna-nesting-{}-{stage}-{case}.fl",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    eprintln!("frontend begin");
    let result = frontend::compile(&Options {
        input: path.clone(),
        emit,
        output: None,
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
        tuple_type: None,
        optional_type: None,
    });
    eprintln!("frontend returned");
    std::fs::remove_file(path).unwrap();
    let result = result.unwrap_or_else(|error| panic!("{case}: {error}"));
    if let Artifact::Nir(package) = result {
        eprintln!("execution begin");
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn nested_scalar_operators_survive_frontend_and_execution() {
    for case in [
        "not",
        "and",
        "or",
        "mixed-logic",
        "equal",
        "machine-logic",
        "negative",
        "bits",
        "concat",
        "open-not",
        "open-arithmetic",
        "open-compare",
        "open-concat",
    ] {
        for stage in ["ast", "hir", "nir"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "nested_expression_child",
                    "--nocapture",
                ])
                .env("LUNA_NESTING_PROBE_STAGE", stage)
                .env("LUNA_NESTING_PROBE_CASE", case)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{case}/{stage}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
