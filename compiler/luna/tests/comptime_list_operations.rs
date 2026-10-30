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
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            splice: splice.then_some("input.Splice"),
            ..Default::default()
        },
    )
}

#[test]
fn binary_bounds_preserve_duplicate_ranges_and_custom_ordering() {
    let package = compile(
        "typealias Integer=.Integer'builtin;typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn less(a:Integer,b:Integer)->bool {a < b}
        comptime fn run()->bool {
            let values:meta.List<Integer> = [1,2,2,2,4];
            let reverse:meta.List<Integer> = [4,2,2,1];
            let empty:meta.List<Integer> = [];
            let types:meta.List<Type> = [Identity<bool>,bool];
            values.lower_bound(2,less) == 1 and values.upper_bound(2,less) == 4
                and values.lower_bound(0,less) == 0 and values.upper_bound(5,less) == 5
                and values.lower_bound(3,less) == 4 and values.upper_bound(3,less) == 4
                and reverse.lower_bound(2,|a,b|a > b) == 1
                and reverse.upper_bound(2,|a,b|a > b) == 3
                and empty.lower_bound(2,|a,b|{let bad:meta.List<bool> = [];bad[0]}) == 0
                and empty.upper_bound(2,|a,b|{let bad:meta.List<bool> = [];bad[0]}) == 0
                and types.lower_bound(bool,|a,b|false) == 0
                and types.upper_bound(bool,|a,b|false) == 2
                and types[0] == Identity<bool>
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn partition_point_checks_all_boundaries_with_logarithmic_callback_counts() {
    for count in [0usize, 1, 2, 3, 7, 8, 9, 31, 32, 33] {
        let values = (0..count)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let bound = if count == 0 {
            0
        } else {
            usize::BITS - count.leading_zeros()
        };
        let package = compile(&format!(
            "typealias Integer=.Integer'builtin;
            comptime fn run()->bool {{
                let values:meta.List<Integer> = [{values}];
                inline for cut in 0..{end} {{
                    var calls:Integer=0;let counter=calls.ref;
                    let index=values.partition_point(|value|{{counter.* += 1;value < cut}});
                    if index != cut or calls > {bound} {{return false;}}
                }}
                true
            }}",
            end = count + 1
        ))
        .unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 5_000_000).unwrap(),
            Value::Bool(true),
            "length {count}"
        );
    }
}

#[test]
fn binary_search_uses_fixed_input_snapshots_and_midpoint_order() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            var input:meta.List<Integer> = [0,1,2,3,4,5,6,7];
            let saved=input;let source=input.ref;
            var visits:Integer=0;let trace=visits.ref;
            let result=input.partition_point(|value| {
                trace.* = trace.* * 10 + value;source.* = [];value < 3
            });
            result == 3 and visits == 423 and input.length() == 0 and saved.length() == 8
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn partition_callback_failure_stops_search_and_vm_recovers() {
    let package=compile(
        "typealias Integer=.Integer'builtin;
        comptime fn in_prefix(value:Integer)->bool {let flags:meta.List<bool> = [true];flags[value]}
        comptime fn fail()->Integer {meta.List<Integer>.partition_point([0,0,0,1,3],in_prefix)}
        comptime fn recover()->Integer {let values:meta.List<Integer> = [1,2,2,4];values.upper_bound(2,|a,b|a < b)}"
    ).unwrap();
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &package,
        comptime_interpretor::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "fail"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::IndexOutOfBounds
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "in_prefix")
            .count(),
        2
    );
    let result = vm.invoke(support::id(&package, "recover"), vec![]).unwrap();
    assert_eq!(
        result,
        Value::Integer(comptime_runtime::Integer::from_u128(3))
    );
}

#[test]
fn stable_sort_retains_type_identity_snapshots_and_equal_item_order() {
    let package = compile(
        "typealias Type=.Type'builtin;
        typealias Integer=.Integer'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn run()->bool {
            let types:meta.List<Type> = [Identity<bool>,bool,Identity<bool>];
            let sorted_types=types.sort_by(|a,b| false);
            var input:meta.List<Integer> = [12,1,11,3,2];
            let saved=input;
            let target=input.ref;
            let sorted=input.sort_by(|a,b| {target.* = [];let left=if a > 10 {a - 10} else {a};let right=if b > 10 {b - 10} else {b};left < right});
            let empty:meta.List<Integer> = [];
            let no_calls=empty.sort_by(|a,b| {let bad:meta.List<bool> = [];bad[0]});
            let singles:meta.List<Integer> = [7];let single=singles.sort_by(|a,b| {let bad:meta.List<bool> = [];bad[0]});
            input.length() == 0 and saved[0] == 12 and saved[4] == 2
                and sorted[0] == 1 and sorted[1] == 11 and sorted[2] == 12
                and sorted[3] == 2 and sorted[4] == 3
                and sorted_types[0] == Identity<bool> and sorted_types[1] == bool
                and sorted_types[2] == Identity<bool> and types[0] == Identity<bool>
                and no_calls.length() == 0 and single[0] == 7
        }",
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 2_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn sort_callback_failure_stops_comparisons_and_vm_recovers() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        comptime fn compare(a:Integer,b:Integer)->bool {
            let flags:meta.List<bool> = [true];flags[a]
        }
        comptime fn fail()->meta.List<Integer> {meta.List<Integer>.sort_by([0,0,0,2],compare)}
        comptime fn recover()->bool {let input:meta.List<Integer> = [3,1,2];let values=input.sort_by(|a,b|a < b);values[0] == 1 and values[1] == 2 and values[2] == 3}",
    ).unwrap();
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &package,
        comptime_interpretor::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "fail"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::IndexOutOfBounds
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "compare")
            .count(),
        2
    );
    assert_eq!(
        vm.invoke(support::id(&package, "recover"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordered_list_queries_check_callback_contracts_even_without_comparisons() {
    for callback in [
        "fn compare(a:bool,b:bool)->bool {a and not b}",
        "comptime fn compare(a:bool,b:bool)->.char'builtin {'x'}",
        "comptime fn compare(a:bool,b:.char'builtin)->bool {a}",
    ] {
        compile(callback).unwrap();
        for call in [
            "sort_by(compare)",
            "lower_bound(true,compare)",
            "upper_bound(true,compare)",
        ] {
            for values in ["[]", "[true]"] {
                let call = format!(
                    "comptime fn run() {{let values:meta.List<bool> = {values};values.{call};}}"
                );
                compile(&format!(
                    "comptime fn compare(a:bool,b:bool)->bool {{a and not b}} {call}"
                ))
                .unwrap();
                assert!(
                    compile(&format!("{callback} {call}"))
                        .unwrap_err()
                        .starts_with("elaboration failed:")
                );
            }
        }
    }
}

#[test]
fn sort_handles_incomplete_runs_with_bounded_comparison_counts() {
    for count in [0usize, 1, 2, 3, 7, 8, 9, 31, 32, 33] {
        let values = (0..count)
            .rev()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let checks = (0..count)
            .map(|n| format!("ordered[{n}] == {n}"))
            .collect::<Vec<_>>()
            .join(" and ");
        let levels = if count < 2 {
            0
        } else {
            usize::BITS - (count - 1).leading_zeros()
        };
        let bound = count * levels as usize;
        let package = compile(&format!(
            "typealias Integer=.Integer'builtin;
            comptime fn run()->bool {{
                var comparisons:Integer=0;let counter=comparisons.ref;
                let values:meta.List<Integer> = [{values}];
                let ordered=values.sort_by(|a,b| {{counter.* += 1;a < b}});
                ordered.length() == {count} and comparisons <= {bound} and {checks}
            }}",
            checks = if checks.is_empty() { "true" } else { &checks }
        ))
        .unwrap();
        if count == 33 {
            assert!(luna::invocation::invoke(&package, "run", 100).is_err());
        }
        assert_eq!(
            luna::invocation::invoke(&package, "run", 5_000_000).unwrap(),
            Value::Bool(true),
            "length {count}"
        );
    }
}

#[test]
fn indexed_scalars_establish_domain_bounds_without_inventing_contents() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        comptime fn increment(values:meta.List<Byte>)->Byte {
            let value=values[0];
            if value < 255 {value + 1} else {0}
        }
        comptime fn counted(flags:meta.List<bool>)->Byte {
            var count:Byte=0;
            if flags[0] {count=count + 1;}
            count
        }
        comptime fn run()->bool {
            increment([254]) == 255 and increment([255]) == 0
                and counted([true]) == 1 and counted([false]) == 0
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );

    for body in [
        "let value=values[0];value + 1",
        "let value=values[0];if flags[0] {value + 1} else {0}",
        "var value=values[0];if value < 255 {value=values[1];value + 1} else {0}",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            comptime fn invalid(values:meta.List<Byte>,flags:meta.List<bool>)->Byte {{{body}}}"
        ))
        .unwrap_err();
        assert!(failure.contains("range"), "{failure}");
    }
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
            let joined=a.concat([false,true]);
            let nested=meta.List<meta.List<bool>>.concat([a], [[false]]);
            let types=meta.List<Type>.concat([bool], [Integer]);
            (empty()).length() == 0 and joined.length() == 4
                and joined[0] and not joined[1] and not joined[2] and joined[3]
                and ((empty()).concat(a)).length() == 2
                and (a.concat(empty())).length() == 2
                and ((empty()).concat(empty())).length() == 0
                and nested.length() == 2 and (nested[1]).length() == 1
                and not nested[1][0] and types.length() == 2
                and types[0] == bool and types[1] == Integer
        }
        pure comptime fn Packet<...flags: meta.List<bool>> -> Type {bool}
        comptime fn Joined() -> Type {Packet<...meta.List<bool>.concat([true], [false])>}
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
            meta.List<bool>.concat(do {return true;}, [false]); false
        }
        comptime fn early_right()->bool {
            meta.List<bool>.concat([false], do {return true;}); false
        }
        comptime fn early_length()->bool {
            .list_length'builtin(do {return true;}); false
        }
        comptime fn run()->bool {
            var counter:Integer=0;
            let xs=(input(counter.ref,0)).concat(input(counter.ref,1));
            let count=(input(counter.ref,2)).length();
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
        "comptime fn bad()->bool {meta.List<bool>.concat([true], [42]);true}",
        "fn bad()->bool {meta.List<bool>.length([true]) == 1}",
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
        "use builtin.bool; const size=(meta.List<bool>.concat([true], [false,true])).length(); comptime fn run()->bool {size == 3}",
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
        .args(["--dependency", "1", "builtin", "3"])
        .args(["--package", "4", "404", "attrs"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/attrs/lib.fl"))
        .args(["--dependency", "2", "attrs", "4"])
        .args(["--dependency", "4", "builtin", "3"])
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
fn repository_list_combinators_preserve_callback_order_and_state() {
    let source = "
        typealias Integer=.Integer'builtin;
        comptime fn digits()->meta.List<Integer> {[1,2,3]}
        comptime fn encode(acc:Integer, digit:Integer)->Integer {acc * 10 + digit}
        comptime fn total()->Integer {
            let values:meta.List<Integer> = [1,2,3];
            let doubled=values.map(|value| value * 2);
            doubled.fold(0,|sum,value| sum + value)
        }
        comptime fn run()->bool {
            var visits:Integer=0;
            var calls:Integer=0;
            let call_count=calls.ref;
            let mapped=(digits()).map(|value| {
                visits = visits + 1;
                call_count.* = call_count.* + 1;
                value + visits
            });
            var order:Integer=0;
            let output=order.ref;
            mapped.foreach(|value| {output.* = output.* * 10 + value;});
            let folded=(digits()).fold(0,encode);
            let empty:meta.List<Integer> = [];
            let untouched=empty.map(|value| {call_count.* = call_count.* + 100;value});
            empty.foreach(|value| {call_count.* = call_count.* + 100;});
            let initial=empty.fold(42,|acc,value| {call_count.* = call_count.* + 100;acc + value});
            visits == 0 and calls == 3 and order == 246 and folded == 123 and initial == 42 and total() == 12
                and untouched.length() == 0 and mapped.length() == 3
                and mapped[0] == 2 and mapped[1] == 4 and mapped[2] == 6
        }
    ";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn mapped_types_feed_variadic_type_factories_without_unfolding_the_head() {
    let package = compile(
        "
        typealias Type=.Type'builtin;
        typealias Char=.char'builtin;
        comptime fn fields()->meta.List<Type> {[bool,Char]}
        comptime fn Pointer(value:Type)->Type {*value}
        pure comptime fn Packet<...fields:meta.List<Type>>->Type {bool}
        comptime fn Generated()->Type {Packet<...(fields()).map(Pointer)>}
        comptime fn Direct()->Type {Packet<*bool,*Char>}
    ",
    )
    .unwrap();
    let generated = luna::invocation::invoke(&package, "Generated", 1_000_000).unwrap();
    assert!(matches!(
        generated,
        Value::Type(nir::Term::NormalForm { .. })
    ));
    assert_eq!(
        generated,
        luna::invocation::invoke(&package, "Direct", 1_000_000).unwrap()
    );
}

#[test]
fn list_combinators_reject_runtime_callbacks_and_incompatible_accumulators() {
    for body in [
        "fn transform(value:bool)->bool {not value} comptime fn bad()->meta.List<bool> {let xs:meta.List<bool> = [true];xs.map(transform)}",
        "fn visit(value:bool) {} comptime fn bad() {let xs:meta.List<bool> = [true];xs.foreach(visit);}",
        "comptime fn bad()->bool {let xs:meta.List<bool> = [true];xs.fold(true,|acc,value| 'x')}",
    ] {
        assert!(compile(body).is_err(), "accepted {body}");
    }
    let package=compile("comptime fn invert(value:bool)->bool {not value} comptime fn run()->bool {let xs:meta.List<bool> = [true];let ys=xs.map(invert);not ys[0]}").unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_map_evaluates_producers_once_and_keeps_owned_inputs_separate() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        comptime fn input(trace:*Integer)->meta.List<Integer> {
            trace.* = trace.* * 10 + 1; [1,2,3]
        }
        comptime fn increment(value:Integer)->Integer {value + 1}
        comptime fn producer(trace:*Integer)->comptime fn(Integer)->Integer {
            trace.* = trace.* * 10 + 2; increment
        }
        comptime fn run()->bool {
            var trace:Integer=0;
            let mapped=(input(trace.ref)).map(producer(trace.ref));
            var original:meta.List<Integer> = [1,2,3];
            let snapshot=original.map(|value| {original = []; value});
            trace == 12 and mapped[0] == 2 and mapped[2] == 4
                and original.length() == 3 and snapshot.length() == 3
                and snapshot[0] == 1 and snapshot[1] == 2 and snapshot[2] == 3
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_pointer_replacement_preserves_active_iteration_and_read_snapshots() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        typealias Type=.Type'builtin;
        comptime fn run()->bool {
            var original:meta.List<Integer> = [1,2,3];
            let slot=original.ref;
            let before=slot.*;
            let mapped=original.map(|value| {slot.* = []; value + 1});
            var types:meta.List<Type> = [bool];
            let types_slot=types.ref;
            let old_types=types_slot.*;
            types_slot.* = [Integer,Type];
            var nested:meta.List<meta.List<bool>> = [[true]];
            let nested_slot=nested.ref;
            let old_nested=nested_slot.*;
            nested_slot.* = [[false],[]];
            original.length() == 0 and before.length() == 3
                and before[0] == 1 and before[2] == 3
                and mapped.length() == 3 and mapped[0] == 2 and mapped[2] == 4
                and old_types.length() == 1 and old_types[0] == bool
                and types.length() == 2 and types[0] == Integer and types[1] == Type
                and old_nested[0][0] and not nested[0][0]
                and (nested[1]).length() == 0
        }
        ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_map_stops_at_callback_failure_and_the_vm_can_be_used_again() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        comptime fn select(value:Integer)->Integer {let table:meta.List<Integer> = [10,5];table[value]}
        comptime fn fail()->meta.List<Integer> {
            let values:meta.List<Integer> = [0,2,1]; values.map(select)
        }
        comptime fn recover()->meta.List<Integer> {
            let values:meta.List<Integer> = [0,1]; values.map(select)
        }
    ",
    )
    .unwrap();
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &package,
        comptime_interpretor::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "fail"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::IndexOutOfBounds
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| *name == "select")
            .count(),
        2
    );
    let result = vm.invoke(support::id(&package, "recover"), vec![]).unwrap();
    let Value::List { values, .. } = result else {
        panic!("expected typed list")
    };
    assert_eq!(
        values,
        vec![
            Value::Integer(comptime_runtime::Integer::from_u128(10)),
            Value::Integer(comptime_runtime::Integer::from_u128(5))
        ]
    );
}

#[test]
fn list_map_can_change_domains_without_flattening_optional_or_nested_values() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        comptime fn select(flag:bool)->?Integer {if flag {7} else {null}}
        comptime fn packet(flag:bool)->meta.List<bool> {[flag,not flag]}
        comptime fn run()->bool {
            let input:meta.List<bool> = [true,false];
            let optional=input.map(select);
            let nested=input.map(packet);
            optional.length() == 2 and nested.length() == 2
                and (optional[0] match {value? => value == 7,null => false})
                and (optional[1] match {_? => false,null => true})
                and (nested[0]).length() == 2 and (nested[1]).length() == 2
                and nested[0][0] and not nested[0][1]
                and not nested[1][0] and nested[1][1]
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_queries_short_circuit_and_empty_inputs_never_call_predicates() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            let input:meta.List<Integer> = [1,2,3];
            let empty:meta.List<Integer> = [];
            var trace:Integer=0;
            let slot=trace.ref;
            let found_any=input.any(|value| {slot.* = slot.* * 10 + value; value == 2});
            let any_order=trace == 12;
            trace = 0;
            let every=input.all(|value| {slot.* = slot.* * 10 + value; value < 2});
            let all_order=trace == 12;
            trace = 0;
            let found=input.find(|value| {slot.* = slot.* * 10 + value; value == 2});
            let find_order=trace == 12;
            trace = 0;
            let no_any=empty.any(|value| {slot.* = 99; true});
            let yes_all=empty.all(|value| {slot.* = 99; false});
            let no_find=empty.find(|value| {slot.* = 99; true});
            let no_filter=empty.filter(|value| {slot.* = 99; true});
            found_any and not every and any_order and all_order and find_order
                and (found match {value? => value == 2,null => false})
                and not no_any and yes_all and trace == 0
                and (no_find match {_? => false,null => true})
                and no_filter.length() == 0
                and (input.filter(|value| false)).length() == 0
                and (input.filter(|value| true)).length() == 3
                and input.all(|value| value > 0)
                and not input.any(|value| value == 4)
                and (input.find(|value| value == 4) match {_? => false,null => true})
        }
        ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_filter_preserves_order_identity_and_snapshot_during_callback_writes() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        typealias Type=.Type'builtin;
        pure comptime fn Packet<...fields:meta.List<Type>>->Type {Packet<...fields>}
        comptime fn Generated()->Type {
            let types:meta.List<Type> = [bool,Integer,bool];
            Packet<...(types).filter(|value| value == bool)>
        }
        comptime fn Direct()->Type {Packet<bool,bool>}
        comptime fn run()->bool {
            var original:meta.List<Integer> = [1,2,3,4];
            let slot=original.ref;
            var order:Integer=0;
            let order_slot=order.ref;
            let kept=original.filter(|value| {
                order_slot.* = order_slot.* * 10 + value;
                slot.* = [];
                value == 1 or value == 3
            });
            original.length() == 0 and order == 1234
                and kept.length() == 2 and kept[0] == 1 and kept[1] == 3
        }
        ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
    let generated = luna::invocation::invoke(&package, "Generated", 1_000_000).unwrap();
    assert!(matches!(
        generated,
        Value::Type(nir::Term::NormalForm { .. })
    ));
    assert_eq!(
        generated,
        luna::invocation::invoke(&package, "Direct", 1_000_000).unwrap()
    );
}

#[test]
fn list_find_distinguishes_a_matched_null_element_from_no_match() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            let values:meta.List<?Integer> = [null,7,null];
            let matched=values.find(|value| value match {null => true,_? => false});
            let missing=values.find(|value| false);
            let kept=values.filter(|value| value match {null => true,_? => false});
            (matched match {inner? => inner match {null => true,_? => false},null => false})
                and (missing match {_? => false,null => true})
                and kept.length() == 2
                and (kept[0] match {null => true,_? => false})
                and (kept[1] match {null => true,_? => false})
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_query_callback_failure_stops_and_vm_recovers() {
    for query in ["any", "all", "find", "filter"] {
        let package = compile(&format!(
            "
            typealias Integer=.Integer'builtin;
            comptime fn pred(index:Integer)->bool {{
                let table:meta.List<bool> = [{}]; table[index]
            }}
            comptime fn fail() {{
                let input:meta.List<Integer> = [0,2,0];
                input.{query}(pred);
            }}
            comptime fn recover()->bool {{
                let input:meta.List<bool> = [true]; input.all(|value| value)
            }}
        ",
            if query == "all" { "true" } else { "false" }
        ))
        .unwrap();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "fail"), vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| *name == "pred")
                .count(),
            2,
            "{query}"
        );
        assert_eq!(
            vm.invoke(support::id(&package, "recover"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn list_queries_reject_incompatible_predicates_even_for_empty_inputs() {
    for query in ["any", "all", "find", "filter", "partition_point"] {
        for pred in [
            "fn pred(value:bool)->bool {value}",
            "comptime fn pred(value:bool)->.char'builtin {'x'}",
            "comptime fn pred(value:.char'builtin)->bool {true}",
        ] {
            // A valid declaration must fail at the call contract, not earlier
            // because the fixture contains an unresolved name such as Char.
            compile(pred).expect("predicate declaration is independently valid");
            for values in ["[]", "[true]"] {
                let call = format!(
                    "comptime fn run() {{let input:meta.List<bool> = {values}; input.{query}(pred);}}"
                );
                compile(&format!(
                    "comptime fn pred(value:bool)->bool {{value}} {call}"
                ))
                .expect("the same call accepts a compatible predicate");
                let error = compile(&format!("{pred} {call}")).unwrap_err();
                assert!(
                    error.contains("required type constraint") && !error.contains("ObligationId"),
                    "{query} with {values} and {pred}: {error}"
                );
            }
        }
    }
}

#[test]
fn list_queries_evaluate_producers_once_and_retain_callback_capture_state() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        struct Trace {value: Integer}
        impl Trace {
            comptime fn input(self)->meta.List<Integer> {
                self.value = self.value * 10 + 1; [3,5,7]
            }
            comptime fn producer(self)->comptime fn(Integer)->bool {
                self.value = self.value * 10 + 2; accepts
            }
        }
        comptime fn accepts(value:Integer)->bool {true}
        comptime fn run()->bool {
            let trace = Trace {value: 0};
            let first=trace.input().find(trace.producer());
            var visits:Integer=0;
            let values:meta.List<Integer> = [3,5,7];
            let second=values.find(|value| {visits = visits + 1; visits == 2});
            trace.value == 12 and visits == 0
                and (first match {value? => value == 3,null => false})
                and (second match {value? => value == 5,null => false})
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_append_preserves_prior_snapshots_and_effectful_concat_order() {
    let package = compile(
        "
        typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            var values:meta.List<bool> = [true];
            let snapshot=values;
            values.append(false);
            let before_concat = values;
            values = before_concat.concat(do { values = [false]; [true] });
            let frozen=values;
            values.append(false);
            var nested:meta.List<meta.List<bool>> = [];
            nested.append(snapshot);
            nested.append(values);
            snapshot.length() == 1 and snapshot[0]
                and frozen.length() == 3 and frozen[0] and not frozen[1] and frozen[2]
                and values.length() == 4 and not values[3]
                and (nested[0]).length() == 1 and (nested[1]).length() == 4
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

const LIST_VIEW: &str = "
    typealias Type = .Type'builtin;
    typealias Integer = .Integer'builtin;
    trait ListView {
        assoc Element: Type;
        comptime fn list_pattern_view_get_len(self) -> Integer;
        comptime fn list_pattern_view_get_element(self, comptime index: Integer) -> Element;
    }
    impl ListView for meta.List<T> where T {
        assoc Element: Type = T;
        comptime fn list_pattern_view_get_len(self) -> Integer { let values = self; values.length() }
        comptime fn list_pattern_view_get_element(self, comptime index: Integer) -> T {
            self[index]
        }
    }
";

fn compile_list_view(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            list_pattern: Some("input.ListView"),
            ..Default::default()
        },
    )
}

#[test]
fn real_meta_lists_match_through_comptime_observation_protocol() {
    let package = compile_list_view(&format!(
        "{LIST_VIEW}
        comptime fn pair(values: meta.List<bool>) -> bool {{
            values match {{ [a, (b if a == b)] => a, _ => false }}
        }}
        comptime fn prefix(values: meta.List<bool>) -> bool {{
            values match {{ [true, false, ..._] => true, _ => false }}
        }}
        comptime fn First(values: meta.List<Type>) -> Type {{
            values match {{ [first, ..._] => first, _ => bool }}
        }}
        comptime fn nested(values: meta.List<meta.List<bool>>) -> bool {{
            values match {{ [[true], []] => true, _ => false }}
        }}
        comptime fn run() -> bool {{
            pair([true, true]) and not pair([true, false])
                and not pair([]) and not pair([true]) and not pair([true, true, true])
                and prefix([true, false]) and prefix([true, false, true])
                and not prefix([true]) and not prefix([false, false])
                and First([Integer, bool]) == Integer and First([]) == bool
                and nested([[true], []]) and not nested([[false], []])
        }}
    "
    ))
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_list_view_rejects_pointer_receivers_before_use() {
    let source = LIST_VIEW.split("    impl ListView").next().unwrap();
    let error = compile_list_view(&source.replace("(self", "(*self")).unwrap_err();
    assert!(error.contains("invalid ListPatternView"), "{error}");
}

#[test]
fn list_view_integer_sizes_remain_comptime_only() {
    let error = compile_list_view(&LIST_VIEW.replace("comptime fn", "fn")).unwrap_err();
    assert!(error.contains("ListPatternView"), "{error}");
    let error = compile_list_view(&format!(
        "{LIST_VIEW}
        fn bad(values: meta.List<bool>) -> bool {{ values match {{ [] => true, _ => false }} }}
    "
    ))
    .unwrap_err();
    assert!(error.contains("runtime ListPatternView phase"), "{error}");
}

#[test]
fn list_view_binding_does_not_authorize_an_opaque_list_factory() {
    let source = format!(
        "{LIST_VIEW}
        pure comptime fn Hidden<T> -> Type {{ meta.List<T> }}
        trait Marker {{}}
        impl Marker for Hidden<bool> {{}}
    "
    );
    let error = compile_list_view(&source).unwrap_err();
    assert!(
        error.contains("implementation subject eligibility"),
        "{error}"
    );
}

#[test]
fn list_ranges_preserve_bounds_empty_results_and_input_snapshots() {
    let package = compile(
        "
        typealias Integer = .Integer'builtin;
        comptime fn run() -> bool {
            let input: meta.List<Integer> = [10,20,30,40];
            let middle = input.slice(1, 3);
            let full = input.slice(0, 4);
            let empty = input.slice(4, 4);
            let changed = input.replace_range(1, 3, [7,8,9]);
            let inserted = input.replace_range(0, 0, [7]);
            let appended = input.replace_range(4, 4, [9]);
            let removed = meta.List<Integer>.replace_range(input, 0, 4, []);
            (middle match {xs? => xs.length() == 2 and xs[0] == 20 and xs[1] == 30, null => false})
                and (full match {xs? => xs.length() == 4 and xs[3] == 40, null => false})
                and (empty match {xs? => xs.length() == 0, null => false})
                and (changed match {xs? => xs.length() == 5 and xs[0] == 10
                    and xs[1] == 7 and xs[2] == 8 and xs[3] == 9 and xs[4] == 40, null => false})
                and (inserted match {xs? => xs.length() == 5 and xs[0] == 7 and xs[1] == 10, null => false})
                and (appended match {xs? => xs.length() == 5 and xs[4] == 9, null => false})
                and (removed match {xs? => xs.length() == 0, null => false})
                and input.length() == 4 and input[1] == 20
                and (meta.List<Integer>.slice([], 0, 0) match {xs? => xs.length() == 0, null => false})
                and (meta.List<Integer>.replace_range([], 0, 0, [5]) match {xs? => xs[0] == 5, null => false})
        }
        comptime fn invalid(start: Integer, end: Integer) -> bool {
            let input: meta.List<Integer> = [1,2];
            (input.slice(start, end) match {null => true, _? => false})
                and (input.replace_range(start, end, [3]) match {null => true, _? => false})
        }
        comptime fn rejected() -> bool {
            invalid(-1, 1) and invalid(2, 1) and invalid(0, 3)
                and invalid(3, 3) and invalid(-2, -1)
                and invalid(0, 999999999999999999999999999999999999)
        }
        ",
    ).unwrap();
    for name in ["run", "rejected"] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 1_000_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn list_ranges_keep_nested_optional_and_normal_form_type_domains() {
    let package = compile(
        "
        typealias Integer = .Integer'builtin;
        typealias Type = .Type'builtin;
        pure comptime fn Packet<...fields: meta.List<Type>> -> Type {Packet<...fields>}
        comptime fn Generated() -> Type {
            let fields: meta.List<Type> = [bool,Integer,bool];
            fields.replace_range(1, 2, [bool]) match {
                xs? => Packet<...xs>, null => bool,
            }
        }
        comptime fn Direct() -> Type {Packet<bool,bool,bool>}
        comptime fn run() -> bool {
            let nested: meta.List<meta.List<bool>> = [[true],[],[false]];
            let optional: meta.List<?Integer> = [7,null,9];
            (nested.slice(1, 3) match {xs? => xs.length() == 2
                and (xs[0]).length() == 0 and not xs[1][0], null => false})
                and (optional.slice(1, 2) match {xs? => xs.length() == 1
                    and (xs[0] match {null => true, _? => false}), null => false})
                and (meta.List<?Integer>.replace_range(optional, 0, 3, [null]) match {
                    xs? => xs.length() == 1 and (xs[0] match {null => true, _? => false}), null => false})
        }
        ",
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Generated", 1_000_000).unwrap(),
        luna::invocation::invoke(&package, "Direct", 1_000_000).unwrap()
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_ranges_evaluate_arguments_once_before_bounds_checks() {
    let package = compile(
        "
        typealias Integer = .Integer'builtin;
        comptime fn input(trace: *Integer, value: meta.List<Integer>) -> meta.List<Integer> {
            trace.* = trace.* * 10 + 1; value
        }
        comptime fn bound(trace: *Integer, tag: Integer, value: Integer) -> Integer {
            trace.* = trace.* * 10 + tag; value
        }
        comptime fn replacement(trace: *Integer, original: *meta.List<Integer>) -> meta.List<Integer> {
            trace.* = trace.* * 10 + 4;
            original.* = [];
            [9]
        }
        comptime fn run() -> bool {
            var trace: Integer = 0;
            var original: meta.List<Integer> = [1,2,3];
            let changed = (input(trace.ref, original)).replace_range(bound(trace.ref, 2, 1), bound(trace.ref, 3, 2), replacement(trace.ref, original.ref));
            let first = trace == 1234 and original.length() == 0
                and (changed match {xs? => xs.length() == 3 and xs[0] == 1
                    and xs[1] == 9 and xs[2] == 3, null => false});
            trace = 0;
            original = [1,2,3];
            let invalid = (input(trace.ref, original)).replace_range(bound(trace.ref, 2, -1), bound(trace.ref, 3, 2), replacement(trace.ref, original.ref));
            let second = trace == 1234 and original.length() == 0
                and (invalid match {null => true, _? => false});
            trace = 0;
            let selected = (input(trace.ref, [7,8,9])).slice(bound(trace.ref, 2, 1), bound(trace.ref, 3, 3));
            first and second and trace == 123
                and (selected match {xs? => xs.length() == 2 and xs[0] == 8 and xs[1] == 9, null => false})
        }
        ",
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_ranges_reject_incompatible_inputs_and_propagate_invocation_limits() {
    for source in [
        "comptime fn bad() {meta.List<bool>.slice([true], false, 1);}",
        "comptime fn bad() {meta.List<bool>.replace_range([true], 0, 1, [7]);}",
        "comptime fn bad() {meta.List<bool>.slice(true, 0, 1);}",
        "fn bad() {meta.List<bool>.slice([true], 0, 1);}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    let package = compile(
        "comptime fn run() -> bool {
        meta.List<bool>.slice([true,false,true,false], 0, 4) match {
            xs? => xs.length() == 4, null => false,
        }
    }",
    )
    .unwrap();
    assert!(luna::invocation::invoke(&package, "run", 1).is_err());
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_transform_collectors_preserve_order_snapshots_and_optional_layers() {
    let source = include_str!("../../../development/examples/sysroot-meta-lists.fl");
    let package = compile(&format!(
        "typealias Integer = .Integer'builtin;
        typealias Type = .Type'builtin;
        typealias u32 = .u32'builtin;
        typealias BitVec<n: Integer> = .BitVec'builtin<n>;
        {source}"
    ))
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_flat_map_preserves_nominal_type_elements_and_inner_order() {
    let package = compile(
        "typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn expand(value:Type)->meta.List<Type> {[value,bool]}
        comptime fn run()->bool {
            let values:meta.List<Type> = [Identity<bool>,bool];
            let output=values.flat_map(expand);
            output.length() == 4 and output[0] == Identity<bool>
                and output[1] == bool and output[2] == bool and output[3] == bool
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_transform_collectors_stop_on_failure_and_recover() {
    for (operation, result, body) in [
        ("filter_map", "?bool", "table[value]"),
        ("flat_map", "meta.List<bool>", "[table[value]]"),
    ] {
        let package=compile(&format!(
            "typealias Integer=.Integer'builtin;
            comptime fn transform(value:Integer)->{result} {{let table:meta.List<bool> = [true,false];{body}}}
            comptime fn fail()->meta.List<bool> {{let values:meta.List<Integer> = [0,2,1];values.{operation}(transform)}}
            comptime fn recover()->meta.List<bool> {{let values:meta.List<Integer> = [0,1];values.{operation}(transform)}}"
        )).unwrap();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "fail"), vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "transform")
                .count(),
            2
        );
        let Value::List { values, .. } =
            vm.invoke(support::id(&package, "recover"), vec![]).unwrap()
        else {
            panic!("expected list")
        };
        assert_eq!(values, vec![Value::Bool(true), Value::Bool(false)]);
    }
}

#[test]
fn list_transform_collectors_reject_wrong_callback_domains_and_runtime_use() {
    for source in [
        "comptime fn bad()->meta.List<bool> {meta.List<bool>.filter_map([true],|value| [value])}",
        "comptime fn bad()->meta.List<bool> {meta.List<bool>.flat_map([true],|value| value)}",
        "fn callback(value:bool)->?bool {value} comptime fn bad()->meta.List<bool> {meta.List<bool>.filter_map([true],callback)}",
        "fn bad()->meta.List<bool> {meta.List<bool>.flat_map([true],|value| [value])}",
        "comptime fn bad() {meta.List<bool>.reduce([true],|left,right| [left]);}",
        "comptime fn bad() {meta.List<bool>.find_map([true],|value| [value]);}",
        "fn callback(value:bool)->?bool {value} comptime fn bad() {meta.List<bool>.find_map([true],callback);}",
        "fn bad() {meta.List<bool>.reduce([true],|left,right| right);}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn list_collectors_check_named_callback_contracts_before_iteration() {
    for (query, valid, invalid) in [
        (
            "filter_map",
            "comptime fn callback(value:bool)->?bool {value}",
            [
                "fn callback(value:bool)->?bool {value}",
                "comptime fn callback(value:.char'builtin)->?bool {true}",
                "comptime fn callback(value:bool)->meta.List<bool> {[value]}",
            ],
        ),
        (
            "flat_map",
            "comptime fn callback(value:bool)->meta.List<bool> {[value]}",
            [
                "comptime fn callback()->meta.List<bool> {[true]}",
                "comptime fn callback(value:.char'builtin)->meta.List<bool> {[true]}",
                "comptime fn callback(value:bool)->bool {value}",
            ],
        ),
        (
            "find_map",
            "comptime fn callback(value:bool)->?bool {value}",
            [
                "fn callback(value:bool)->?bool {value}",
                "comptime fn callback(value:.char'builtin)->?bool {true}",
                "comptime fn callback(value:bool)->meta.List<bool> {[value]}",
            ],
        ),
        (
            "reduce",
            "comptime fn callback(left:bool,right:bool)->bool {right}",
            [
                "fn callback(left:bool,right:bool)->bool {right}",
                "comptime fn callback(left:bool,right:.char'builtin)->bool {left}",
                "comptime fn callback(left:bool,right:bool)->.char'builtin {'x'}",
            ],
        ),
    ] {
        for callback in invalid {
            compile(callback).expect("callback declaration is independently valid");
            // Empty and singleton reductions need no callback invocation, but
            // must still enforce the declared callback contract.
            for values in ["[]", "[true]", "[true,false]"] {
                let call = format!(
                    "comptime fn run() {{let input:meta.List<bool> = {values}; input.{query}(callback);}}"
                );
                compile(&format!("{valid} {call}"))
                    .expect("the same call accepts a compatible callback");
                let error = compile(&format!("{callback} {call}")).unwrap_err();
                assert!(
                    error.starts_with("elaboration failed:"),
                    "{query} with {values} and {callback}: {error}"
                );
            }
        }
    }
}

#[test]
fn list_reduction_and_search_preserve_type_identity() {
    let package = compile(
        "typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn select(value:Type)->?Type {value}
        comptime fn run()->bool {
            let values:meta.List<Type> = [Identity<bool>,bool];
            let reduced=values.reduce(|left,right|left);
            let found=values.find_map(select);
            (reduced match {value?=>value == Identity<bool>,null=>false})
                and (found match {value?=>value == Identity<bool>,null=>false})
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_reduction_and_search_abort_failed_callbacks_and_recover() {
    for (callback, invocation) in [
        (
            "comptime fn transform(left:Integer,right:Integer)->Integer {let values:meta.List<Integer> = [10];values[right]}",
            "values.reduce(transform)",
        ),
        (
            "comptime fn transform(value:Integer)->?Integer {let values:meta.List<Integer> = [10];let checked=values[value];null}",
            "values.find_map(transform)",
        ),
    ] {
        let package = compile(&format!(
            "typealias Integer=.Integer'builtin; {callback}
            comptime fn fail()->?Integer {{let values:meta.List<Integer> = [0,0,2,0];{invocation}}}
            comptime fn recover()->?Integer {{let values:meta.List<Integer> = [0,0];{invocation}}}"
        ))
        .unwrap();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "fail"), vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        let expected_calls = if invocation.contains("reduce") { 2 } else { 3 };
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "transform")
                .count(),
            expected_calls
        );
        let recovered = vm.invoke(support::id(&package, "recover"), vec![]).unwrap();
        let expected = luna::invocation::invoke(&package, "recover", 1_000_000).unwrap();
        assert_eq!(recovered, expected);
    }
}

#[test]
fn indexed_and_paired_maps_preserve_order_domains_and_closure_state() {
    let package = compile(
        r#"
        typealias Integer=.Integer'builtin;
        typealias Type=.Type'builtin;
        pure comptime fn Identity<T> -> Type {T}
        comptime fn run()->bool {
            let values:meta.List<Integer> = [10,20,30];
            var calls:Integer=0;
            let mapped=values.map_indexed(|index,value| {
                calls = calls + 1; index * 100 + value + calls
            });
            let flags:meta.List<bool> = [true,false];
            let paired=values.zip_with(flags,|value,flag| if flag {value} else { -value });
            let shorter=flags.zip_with(values,|flag,value| if flag {value} else { -value });
            let types:meta.List<Type> = [Identity<bool>,bool];
            let retained=types.map_indexed(|index,value| value);
            let selected=types.zip_with(flags,|value,flag| value);
            mapped.length()==3 and mapped[0]==11 and mapped[1]==122 and mapped[2]==233
                and paired.length()==2 and paired[0]==10 and paired[1]==-20
                and shorter.length()==2 and shorter[1]==-20
                and retained[0]==Identity<bool>
                and selected[0]==Identity<bool> and selected[1]==bool
                and values.length()==3 and values[0]==10
        }
    "#,
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn position_short_circuits_and_empty_mapping_never_calls_callbacks() {
    let package = compile(
        r#"
        typealias Integer=.Integer'builtin;
        comptime fn crash(value:Integer)->bool {let empty:meta.List<bool> = [];empty[0]}
        comptime fn run()->bool {
            let values:meta.List<Integer> = [0,1,2];
            let empty:meta.List<Integer> = [];
            let a=empty.map_indexed(|index,value| crash(value));
            let b=empty.zip_with(values,|left,right| crash(left));
            let c=values.zip_with(empty,|left,right| crash(right));
            let zero=values.position(|value| true);
            let second=values.position(|value| if value==2 {crash(value)} else {value==1});
            let missing=values.position(|value| false);
            let absent=empty.position(crash);
            a.length()==0 and b.length()==0 and c.length()==0
                and (zero match {index? => index==0,null=>false})
                and (second match {index? => index==1,null=>false})
                and (missing match {null=>true,_?=>false})
                and (absent match {null=>true,_?=>false})
        }
    "#,
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indexed_and_paired_callbacks_reject_wrong_domains_and_runtime_phase() {
    for source in [
        "comptime fn bad() {meta.List<bool>.map_indexed([true],|index| index);}",
        "comptime fn bad() {meta.List<bool>.zip_with([true],[false],|a,b| a + b);}",
        "comptime fn bad() {meta.List<bool>.position([true],|value| 'x');}",
        "fn bad() {meta.List<bool>.map_indexed([true],|index,value| value);}",
        "fn callback(a:bool,b:bool)->bool {a} comptime fn bad() {meta.List<bool>.zip_with([true],[false],callback);}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn indexed_pairing_and_position_stop_on_failure_and_allow_retry() {
    for (parameters, call, result) in [
        (
            "index:Integer,value:Integer",
            "values.map_indexed(callback)",
            "result.length() == 2 and not result[0] and result[1]",
        ),
        (
            "value:Integer,flag:bool",
            "values.zip_with([true,true,true],callback)",
            "result.length() == 2 and not result[0] and result[1]",
        ),
        (
            "value:Integer",
            "values.position(callback)",
            "result match {index? => index == 1,null => false}",
        ),
    ] {
        let package = compile(&format!(
            r#"
            typealias Integer=.Integer'builtin;
            comptime fn callback({parameters})->bool {{
                let table:meta.List<bool> = [false,true]; table[value]
            }}
            comptime fn fail()->bool {{
                let values:meta.List<Integer> = [0,2,1];let result={call}; {result}
            }}
            comptime fn recover()->bool {{
                let values:meta.List<Integer> = [0,1];let result={call}; {result}
            }}
        "#
        ))
        .unwrap();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "fail"), vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "callback")
                .count(),
            2
        );
        assert_eq!(
            vm.invoke(support::id(&package, "recover"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn list_append_all_preserves_sources_aliases_and_projected_receivers() {
    let package = compile(
        r#"
        typealias Type=.Type'builtin;
        pure comptime fn Identity<T> -> Type {T}
        struct Holder {values:meta.List<bool>, marker:bool}
        impl Holder {
            comptime fn reset(self)->meta.List<bool> {self.values=[];[true]}
        }
        comptime fn run()->bool {
            var values:meta.List<bool> = [true,false];
            let saved=values;
            values.append_all(values);
            let twice=values;
            let empty:meta.List<bool> = [];
            values.append_all(empty);
            let holder=Holder {values:values, marker:true};
            holder.values.append_all([false]);
            var types:meta.List<Type> = [Identity<bool>];
            types.append_all([bool]);
            types.append_all(types);
            var nested:meta.List<meta.List<bool>> = [];
            nested.append_all([saved,empty]);
            let modified=Holder {values:values, marker:false};
            modified.values.append_all(modified.reset());
            values.length()==4 and values[0] and not values[1]
                and values[2] and not values[3] and saved.length()==2
                and twice.length()==4 and holder.values.length()==5
                and not holder.values[4] and holder.marker
                and types.length()==4 and types[0]==Identity<bool>
                and types[2]==Identity<bool> and types[3]==bool
                and nested.length()==2 and nested[0].length()==2
                and nested[1].length()==0
                and modified.values.length()==1 and modified.values[0]
        }
    "#,
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_append_all_rejects_incompatible_domains_and_readonly_rebinding() {
    for source in [
        "comptime fn bad() {var xs:meta.List<bool> = [];xs.append_all(['x']);}",
        "fn bad() {var xs:meta.List<bool> = [];xs.append_all([true]);}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
    let package =
        compile("comptime fn run() {let xs:meta.List<bool> = [true];xs.append_all([false]);}")
            .unwrap();
    assert!(luna::invocation::invoke(&package, "run", 100_000).is_err());
}

#[test]
fn list_scans_generate_prefix_values_and_field_offsets() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        comptime fn run()->bool {
            let sizes:meta.List<Integer> = [4,8,1];
            let ends=sizes.scan(0,|offset,size|offset + size);
            let starts=sizes.scan_exclusive(0,|offset,size|offset + size);
            let digits:meta.List<Integer> = [1,2,3];
            let prefixes=digits.scan(9,|acc,value|acc * 10 + value);
            ends.length() == 3 and ends[0] == 4 and ends[1] == 12 and ends[2] == 13
                and starts.length() == 3 and starts[0] == 0 and starts[1] == 4 and starts[2] == 12
                and prefixes[0] == 91 and prefixes[1] == 912 and prefixes[2] == 9123
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_scans_preserve_input_and_accumulator_snapshots() {
    for (method, lengths) in [("scan", [1, 2, 3]), ("scan_exclusive", [0, 1, 2])] {
        let package = compile(&format!(
            "typealias Integer=.Integer'builtin;typealias Type=.Type'builtin;
            pure comptime fn Identity<T>->Type {{T}}
            comptime fn run()->bool {{
                var input:meta.List<Type> = [Identity<bool>,bool,Integer];
                let slot=input.ref;let original=input;
                let seed:meta.List<Type> = [];
                let history=input.{method}(seed,|acc,value| {{slot.* = [];acc.append(value);acc}});
                history.length() == 3 and input.length() == 0 and seed.length() == 0
                    and (history[0]).length() == {a} and (history[1]).length() == {b}
                    and (history[2]).length() == {c} and history[2][0] == Identity<bool>
                    and original[0] == Identity<bool> and original[1] == bool
            }}",
            a = lengths[0],
            b = lengths[1],
            c = lengths[2]
        ))
        .unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
            Value::Bool(true),
            "{method}"
        );
    }
}

#[test]
fn list_scans_visit_each_element_once_including_the_last_exclusive_step() {
    for method in ["scan", "scan_exclusive"] {
        let package = compile(&format!(
            "typealias Integer=.Integer'builtin;
            comptime fn run()->bool {{
                let values:meta.List<Integer> = [1,2,3];
                let empty:meta.List<Integer> = [];
                var trace:Integer=0;let record=trace.ref;
                let result=values.{method}(0,|acc,value|{{record.* = record.* * 10 + value;acc + value}});
                let absent=empty.{method}(0,|acc,value|{{record.* = 999;acc}});
                trace == 123 and result.length() == 3 and absent.length() == 0
            }}"
        )).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn list_scans_propagate_last_callback_failure_and_allow_vm_recovery() {
    for method in ["scan", "scan_exclusive"] {
        let package=compile(&format!(
            "typealias Integer=.Integer'builtin;
            comptime fn combine(acc:Integer,index:Integer)->Integer {{let values:meta.List<Integer> = [3];acc + values[index]}}
            comptime fn fail()->meta.List<Integer> {{let xs:meta.List<Integer> = [0,0,1];xs.{method}(0,combine)}}
            comptime fn recover()->bool {{let xs:meta.List<Integer> = [0];let result=xs.{method}(0,combine);result.length() == 1}}"
        )).unwrap();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "fail"), vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "combine")
                .count(),
            3
        );
        assert_eq!(
            vm.invoke(support::id(&package, "recover"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn list_scans_reject_wrong_callback_phases_and_domains_even_when_empty() {
    for method in ["scan", "scan_exclusive"] {
        for callback in [
            "fn combine(acc:bool,value:bool)->bool {acc and value}",
            "comptime fn combine(acc:bool,value:bool)->.char'builtin {'x'}",
            "comptime fn combine(acc:bool,value:.char'builtin)->bool {acc}",
        ] {
            compile(callback).unwrap();
            assert!(compile(&format!("{callback} comptime fn run() {{let xs:meta.List<bool> = [];xs.{method}(false,combine);}} ")).is_err());
        }
    }
}

#[test]
fn list_scans_evaluate_receiver_seed_and_callback_producer_once_in_order() {
    for method in ["scan", "scan_exclusive"] {
        let package = compile(&format!(
            "typealias Integer=.Integer'builtin;
            comptime fn input(trace:*Integer)->meta.List<Integer> {{trace.* = trace.* * 10 + 1;[2,3]}}
            comptime fn seed(trace:*Integer)->Integer {{trace.* = trace.* * 10 + 2;10}}
            comptime fn combine(acc:Integer,value:Integer)->Integer {{acc + value}}
            comptime fn callback(trace:*Integer)->comptime fn(Integer,Integer)->Integer {{trace.* = trace.* * 10 + 3;combine}}
            comptime fn run()->bool {{
                var trace:Integer=0;
                let values=(input(trace.ref)).{method}(seed(trace.ref),callback(trace.ref));
                trace == 123 and values.length() == 2 and values[1] == {last}
            }}", last=if method == "scan" {15} else {12}
        )).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 1_000_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn adjacent_list_operations_preserve_representatives_and_strict_ordering() {
    let source = "typealias Integer=.Integer'builtin;typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn run()->bool {
            let input:meta.List<Integer> = [1,2,3,8,9,1];
            let groups=input.dedup_by(|left,right|right - left < 2);
            let empty:meta.List<Integer> = [];
            let single:meta.List<Integer> = [7];
            let ordered:meta.List<Integer> = [1,1,2,3];
            let reverse:meta.List<Integer> = [3,2,2,1];
            let types:meta.List<Type> = [Identity<bool>,Identity<bool>,Identity<bool>];
            let distinct=types.dedup_by(|a,b|a==b);
            groups.length()==3 and groups[0]==1 and groups[1]==3 and groups[2]==8
                and input.length()==6
                and empty.dedup_by(|a,b|{let bad:meta.List<bool> = [];bad[0]}).length()==0
                and single.dedup_by(|a,b|{let bad:meta.List<bool> = [];bad[0]}).length()==1
                and empty.is_sorted_by(|a,b|{let bad:meta.List<bool> = [];bad[0]})
                and single.is_sorted_by(|a,b|{let bad:meta.List<bool> = [];bad[0]})
                and ordered.is_sorted_by(|a,b|a < b)
                and reverse.is_sorted_by(|a,b|a > b)
                and not input.is_sorted_by(|a,b|a < b)
                and distinct.length()==1 and distinct[0]==Identity<bool>
        }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 2_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn adjacent_list_operations_short_circuit_and_recover_after_callback_failure() {
    let package=compile(
        "typealias Integer=.Integer'builtin;
        comptime fn less(a:Integer,b:Integer)->bool {
            if a==9 {let bad:meta.List<bool> = [];bad[0]} else {a < b}
        }
        comptime fn equivalent(a:Integer,b:Integer)->bool {
            if b==9 {let bad:meta.List<bool> = [];bad[0]} else {a==b}
        }
        comptime fn short()->bool {let xs:meta.List<Integer> = [2,1,9];not xs.is_sorted_by(less)}
        comptime fn bad_order()->bool {let xs:meta.List<Integer> = [1,2,9];xs.is_sorted_by(less)}
        comptime fn bad_group()->meta.List<Integer> {let xs:meta.List<Integer> = [1,1,9];xs.dedup_by(equivalent)}
        comptime fn recover()->bool {let xs:meta.List<Integer> = [1,1,2];xs.dedup_by(equivalent).length()==2 and xs.is_sorted_by(less)}"
    ).unwrap();
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &package,
        comptime_interpretor::Budget {
            steps: 2_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "short"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "less")
            .count(),
        1
    );
    for entry in ["bad_order", "bad_group"] {
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert_eq!(
            vm.invoke(support::id(&package, "recover"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn adjacent_list_operations_reject_runtime_and_wrong_domain_callbacks() {
    for method in ["dedup_by", "is_sorted_by"] {
        for declaration in [
            "fn compare(a:bool,b:bool)->bool {a}",
            "comptime fn compare(a:bool,b:bool)->.Integer'builtin {0}",
            "typealias char=.char'builtin;comptime fn compare(a:char,b:char)->bool {true}",
        ] {
            assert!(compile(&format!("{declaration} comptime fn bad() {{let xs:meta.List<bool> = [];xs.{method}(compare);}}" )).is_err());
        }
    }
}

#[test]
fn adjacent_list_dedup_propagates_unknown_type_equality() {
    let package = compile(
        "typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn run()->bool {
            let types:meta.List<Type> = [bool,Identity<bool>];
            types.dedup_by(|a,b|a==b).length()==1
        }",
    )
    .unwrap();
    let error = luna::invocation::invoke(&package, "run", 1_000_000).unwrap_err();
    assert!(error.contains("TypeContractUnresolved"), "{error}");
}

#[test]
fn checked_list_query_evaluates_index_once_before_reading_receiver() {
    let package = compile(
        "typealias Integer = .Integer'builtin;
        comptime fn run()->bool {
            var values: meta.List<bool> = [true, true];
            let saved = values;
            var calls: Integer = 0;
            let found = values.get(do {
                calls = calls + 1;
                values = [false];
                0
            });
            (found matches false?) and calls == 1
                and values.length() == 1 and saved.length() == 2 and saved[0]
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn checked_list_query_does_not_catch_index_expression_failures() {
    let package = compile(
        "typealias Integer = .Integer'builtin;
        comptime fn fail()->?bool {
            let values: meta.List<bool> = [true];
            let indices: meta.List<Integer> = [];
            values.get(indices[0])
        }
        comptime fn recover()->bool {
            let values: meta.List<bool> = [true];
            values.get(0) matches true?
        }",
    )
    .unwrap();
    let mut vm = comptime_interpretor::Interpreter::for_comptime(
        &package,
        comptime_interpretor::Budget {
            steps: 100_000,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "fail"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::IndexOutOfBounds
    );
    assert_eq!(
        vm.invoke(support::id(&package, "recover"), vec![]).unwrap(),
        Value::Bool(true)
    );
}
