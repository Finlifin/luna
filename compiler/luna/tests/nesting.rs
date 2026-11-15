use luna::{
    cli::{self, Command, Emit},
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
        "blocks" => format!(
            "fn run()->bool {{{}true{}}}",
            "do {".repeat(32),
            "}".repeat(32)
        ),
        "branches" => {
            let body = (0..32).fold("true".to_owned(), |body, _| {
                format!("if true {{ {body} }} else {{ false }}")
            });
            format!("fn run()->bool {{{body}}}")
        }
        "when" => {
            let body = (0..32).fold("do { return true; false }".to_owned(), |body, _| {
                format!("when {{ false => false, else => {body}, }}")
            });
            format!("fn run()->bool {{ let value: bool = {body}; not value }}")
        }
        "match" => {
            let body = (0..32).fold("do { return true; false }".to_owned(), |body, _| {
                format!("true match {{ false => false, true => {body}, }}")
            });
            format!("fn run()->bool {{ let value: bool = {body}; not value }}")
        }
        "partial-arms" => {
            let body = (0..32).fold("byte()".to_owned(), |body, index| match index % 3 {
                0 => format!("when {{ false => null, else => {body} }}"),
                1 => format!("false match {{ true => null, false => {body} }}"),
                _ => format!("if false {{ null }} else {{ {body} }}"),
            });
            format!(
                "fn byte() -> u8 {{ 9 }}
                fn keep(value: ?T) -> ?T where T {{ value }}
                fn is_byte(value: ?T) -> bool where T {{ T == u8 }}
                fn run() -> bool {{ let value = keep({body}); is_byte(value) }}"
            )
        }
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
        "normal-regions" => {
            let body = (0..32).fold("u8".to_owned(), |body, index| match index % 3 {
                0 => format!("Layer<do {{ {body} }}>"),
                1 => format!("Layer<if true {{ {body} }} else {{ bool }}>"),
                _ => format!("Layer<when {{ false => bool, else => {body} }}>"),
            });
            format!(
                "pure comptime fn Layer<T> -> Type {{ T }}
                comptime fn Build() -> Type {{ {body} }}
                fn run() -> bool {{ const first = Build(); const second = Build(); first == second }}"
            )
        }
        "logical-regions" => {
            let body = (0..48).fold("true".to_owned(), |body, index| match index % 4 {
                0 => format!("true and ({body})"),
                1 => format!("false or ({body})"),
                2 => format!("when {{ false => false, else => {body} }}"),
                _ => format!("({body}) == true"),
            });
            format!("fn run() -> bool {{ {body} }}")
        }
        "equal" => {
            let body = (0..32).fold("true".to_owned(), |body, _| format!("({body}) == true"));
            format!("fn run()->bool {{{body}}}")
        }
        "machine-logic" => format!(
            "typealias Byte=u8;fn run()->bool {{let value: Byte = 1; let result = value + 1; {}true{} and result == 2}}",
            "not (".repeat(32),
            ")".repeat(32)
        ),
        "negative" => format!(
            "typealias Tiny=i8;fn nested()->Tiny {{ {}1{} }}fn run()->bool {{nested()==1}}",
            "-(".repeat(32),
            ")".repeat(32)
        ),
        "bits" => format!(
            "typealias Bits=BitVec<8>;fn nested()->Bits {{{}0{}}}fn run()->bool {{nested()==0}}",
            "not (".repeat(32),
            ")".repeat(32)
        ),
        "concat" => format!(
            "typealias Bit=BitVec<1>;typealias Word=BitVec<32>;fn nested()->Word {{let zero:Bit=0;{}zero{}}}fn run()->bool {{nested()==0}}",
            "zero ++ (".repeat(31),
            ")".repeat(31)
        ),
        "open-not" => format!(
            "fn nested(comptime flag:bool)->bool {{{}(do {{inline if flag {{true}} else {{false}}}}){}}}fn run()->bool {{nested(true)}}",
            "not (".repeat(32),
            ")".repeat(32)
        ),
        "open-arithmetic" => format!(
            "typealias Byte=u8;fn nested(comptime flag:bool)->Byte {{{}(do {{inline if flag {{0}} else {{1}}}}){}}}fn run()->bool {{nested(true)==0}}",
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
            "typealias Bit=BitVec<1>;typealias Word=BitVec<32>;fn nested(comptime flag:bool)->Word {{let zero:Bit=0;{}(do {{inline if flag {{zero}} else {{zero}}}}){}}}fn run()->bool {{nested(true)==0}}",
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
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(mut options) = cli::parse([
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
    options.emit = emit;
    if matches!(emit, Emit::Ast | Emit::Hir) {
        options.build_targets = Default::default();
    }
    let result = frontend::compile(&options);
    eprintln!("frontend returned");
    std::fs::remove_file(path).unwrap();
    let result = result.unwrap_or_else(|error| panic!("{case}: {error}"));
    if let Artifact::Nir(package) = result {
        eprintln!("execution begin");
        let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn nested_scalar_operators_survive_frontend_and_execution() {
    check_cases(&[
        "blocks",
        "branches",
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
    ]);
}

#[test]
fn nested_when_preserves_return_through_the_pipeline() {
    check_cases(&["when"]);
}

#[test]
fn nested_match_preserves_return_through_the_pipeline() {
    check_cases(&["match"]);
}

#[test]
fn nested_partial_arm_probes_survive_frontend_and_execution() {
    check_cases(&["partial-arms"]);
}

#[test]
fn nested_logical_regions_preserve_mixed_control_children() {
    check_cases(&["logical-regions"]);
}

#[test]
fn nested_normal_inputs_share_the_control_region_stack() {
    check_cases(&["normal-regions"]);
}

fn check_cases(cases: &[&str]) {
    for case in cases {
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
