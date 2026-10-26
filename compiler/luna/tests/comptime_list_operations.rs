//! Execute the real meta wrappers, retaining nominal types and argument effects.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    compile_bound(source, false)
}

fn compile_bound(source: &str, splice: bool) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} {source}",
            include_str!("../../../library/meta/lib.fl")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            splice: splice.then_some("input.Splice"),
            ..Default::default()
        },
    )
}

#[test]
fn list_length_and_concat_preserve_empty_nested_and_type_elements() {
    let source = "
        typealias Integer=.Integer'builtin;
        typealias Type=.Type'builtin;
        trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
        impl Splice for bool {assoc RuntimeTarget:Type=bool;fn splice(comptime data:bool)->bool {data}}
        comptime fn empty()->meta.List<bool> {[]}
        comptime fn run()->bool {
            let a:meta.List<bool> = [true,false];
            let joined=meta.concat(a, [false,true]);
            let nested=meta.concat<meta.List<bool>>([a], [[false]]);
            let types=meta.concat<Type>([bool], [Integer]);
            meta.length(empty()) == 0 and meta.length(joined) == 4
                and joined[0] and not joined[1] and not joined[2] and joined[3]
                and meta.length(meta.concat(empty(), a)) == 2
                and meta.length(meta.concat(a, empty())) == 2
                and meta.length(meta.concat(empty(), empty())) == 0
                and meta.length(nested) == 2 and meta.length(nested[1]) == 1
                and not nested[1][0] and meta.length(types) == 2
                and types[0] == bool and types[1] == Integer
        }
        pure comptime fn Packet<...flags: meta.List<bool>> -> Type {bool}
        comptime fn Joined() -> Type {Packet<...meta.concat<bool>([true], [false])>}
        comptime fn Direct() -> Type {Packet<true, false>}
        const checked=run();
        fn check()->bool {checked}
    ";
    let package = compile_bound(source, true).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Joined", 100_000).unwrap(),
        luna::invocation::invoke(&package, "Direct", 100_000).unwrap()
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "check", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_operations_evaluate_arguments_once_in_order_and_propagate_returns() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        comptime fn input(counter:*Integer, expected:Integer)->meta.List<bool> {
            let ordered=counter.* == expected;
            counter.* = counter.* + 1;
            [ordered]
        }
        comptime fn early_left()->bool {
            meta.concat<bool>(do {return true;}, [false]); false
        }
        comptime fn early_right()->bool {
            meta.concat<bool>([false], do {return true;}); false
        }
        comptime fn early_length()->bool {
            .list_length'builtin(do {return true;}); false
        }
        comptime fn run()->bool {
            var counter:Integer=0;
            let xs=meta.concat(input(counter.ref,0), input(counter.ref,1));
            let count=meta.length(input(counter.ref,2));
            counter == 3 and xs[0] and xs[1] and count == 1
                and early_left() and early_right() and early_length()
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_operations_reject_wrong_domains_arities_and_unbound_runtime_transfer() {
    for body in [
        "comptime fn bad()->bool {.list_length'builtin(true);true}",
        "comptime fn bad()->bool {.list_length'builtin();true}",
        "comptime fn bad()->bool {.list_concat'builtin([true]);true}",
        "comptime fn bad()->bool {meta.concat<bool>([true], [42]);true}",
        "fn bad()->bool {meta.length<bool>([true]) == 1}",
    ] {
        assert!(compile(body).is_err(), "accepted {body}");
    }
}

#[test]
fn ordinary_package_uses_public_list_operations_without_builtin_access() {
    let directory = std::env::temp_dir().join(format!("luna-meta-lists-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let input = directory.join("input.fl");
    std::fs::write(
        &input,
        "const size=meta.length(meta.concat<bool>([true], [false,true])); comptime fn run()->bool {size == 3}",
    )
    .unwrap();
    let library =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/meta/lib.fl");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .args(["--packages-v1", "--package", "1", "101", "app"])
        .arg(&input)
        .args(["--package", "2", "202", "meta"])
        .arg(&library)
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
