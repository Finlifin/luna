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

#[test]
fn repository_list_combinators_preserve_callback_order_and_state() {
    let source = "
        typealias Integer=.Integer'builtin;
        comptime fn digits()->meta.List<Integer> {[1,2,3]}
        comptime fn encode(acc:Integer, digit:Integer)->Integer {acc * 10 + digit}
        comptime fn total()->Integer {
            let values:meta.List<Integer> = [1,2,3];
            let doubled=meta.map(values,|value| value * 2);
            meta.fold(doubled,0,|sum,value| sum + value)
        }
        comptime fn run()->bool {
            var visits:Integer=0;
            var calls:Integer=0;
            let call_count=calls.ref;
            let mapped=meta.map(digits(),|value| {
                visits = visits + 1;
                call_count.* = call_count.* + 1;
                value + visits
            });
            var order:Integer=0;
            let output=order.ref;
            meta.foreach(mapped,|value| {output.* = output.* * 10 + value;});
            let folded=meta.fold(digits(),0,encode);
            let empty:meta.List<Integer> = [];
            let untouched=meta.map(empty,|value| {call_count.* = call_count.* + 100;value});
            meta.foreach(empty,|value| {call_count.* = call_count.* + 100;});
            let initial=meta.fold(empty,42,|acc,value| {call_count.* = call_count.* + 100;acc + value});
            visits == 0 and calls == 3 and order == 246 and folded == 123 and initial == 42 and total() == 12
                and meta.length(untouched) == 0 and meta.length(mapped) == 3
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
        comptime fn Generated()->Type {Packet<...meta.map(fields(),Pointer)>}
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
        "fn transform(value:bool)->bool {not value} comptime fn bad()->meta.List<bool> {let xs:meta.List<bool> = [true];meta.map(xs,transform)}",
        "fn visit(value:bool) {} comptime fn bad() {let xs:meta.List<bool> = [true];meta.foreach(xs,visit);}",
        "comptime fn bad()->bool {let xs:meta.List<bool> = [true];meta.fold(xs,true,|acc,value| 'x')}",
    ] {
        assert!(compile(body).is_err(), "accepted {body}");
    }
    let package=compile("comptime fn invert(value:bool)->bool {not value} comptime fn run()->bool {let xs:meta.List<bool> = [true];let ys=meta.map(xs,invert);not ys[0]}").unwrap();
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
            let mapped=meta.map(input(trace.ref),producer(trace.ref));
            var original:meta.List<Integer> = [1,2,3];
            let snapshot=meta.map(original,|value| {original = []; value});
            trace == 12 and mapped[0] == 2 and mapped[2] == 4
                and meta.length(original) == 3 and meta.length(snapshot) == 3
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
            let mapped=meta.map(original,|value| {slot.* = []; value + 1});
            var types:meta.List<Type> = [bool];
            let types_slot=types.ref;
            let old_types=types_slot.*;
            types_slot.* = [Integer,Type];
            var nested:meta.List<meta.List<bool>> = [[true]];
            let nested_slot=nested.ref;
            let old_nested=nested_slot.*;
            nested_slot.* = [[false],[]];
            meta.length(original) == 0 and meta.length(before) == 3
                and before[0] == 1 and before[2] == 3
                and meta.length(mapped) == 3 and mapped[0] == 2 and mapped[2] == 4
                and meta.length(old_types) == 1 and old_types[0] == bool
                and meta.length(types) == 2 and types[0] == Integer and types[1] == Type
                and old_nested[0][0] and not nested[0][0]
                and meta.length(nested[1]) == 0
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
            let values:meta.List<Integer> = [0,2,1]; meta.map(values,select)
        }
        comptime fn recover()->meta.List<Integer> {
            let values:meta.List<Integer> = [0,1]; meta.map(values,select)
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
            let optional=meta.map(input,select);
            let nested=meta.map(input,packet);
            meta.length(optional) == 2 and meta.length(nested) == 2
                and (optional[0] match {value? => value == 7,null => false})
                and (optional[1] match {_? => false,null => true})
                and meta.length(nested[0]) == 2 and meta.length(nested[1]) == 2
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
            let found_any=meta.any(input,|value| {slot.* = slot.* * 10 + value; value == 2});
            let any_order=trace == 12;
            trace = 0;
            let every=meta.all(input,|value| {slot.* = slot.* * 10 + value; value < 2});
            let all_order=trace == 12;
            trace = 0;
            let found=meta.find(input,|value| {slot.* = slot.* * 10 + value; value == 2});
            let find_order=trace == 12;
            trace = 0;
            let no_any=meta.any(empty,|value| {slot.* = 99; true});
            let yes_all=meta.all(empty,|value| {slot.* = 99; false});
            let no_find=meta.find(empty,|value| {slot.* = 99; true});
            let no_filter=meta.filter(empty,|value| {slot.* = 99; true});
            found_any and not every and any_order and all_order and find_order
                and (found match {value? => value == 2,null => false})
                and not no_any and yes_all and trace == 0
                and (no_find match {_? => false,null => true})
                and meta.length(no_filter) == 0
                and meta.length(meta.filter(input,|value| false)) == 0
                and meta.length(meta.filter(input,|value| true)) == 3
                and meta.all(input,|value| value > 0)
                and not meta.any(input,|value| value == 4)
                and (meta.find(input,|value| value == 4) match {_? => false,null => true})
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
            Packet<...meta.filter(types,|value| value == bool)>
        }
        comptime fn Direct()->Type {Packet<bool,bool>}
        comptime fn run()->bool {
            var original:meta.List<Integer> = [1,2,3,4];
            let slot=original.ref;
            var order:Integer=0;
            let order_slot=order.ref;
            let kept=meta.filter(original,|value| {
                order_slot.* = order_slot.* * 10 + value;
                slot.* = [];
                value == 1 or value == 3
            });
            meta.length(original) == 0 and order == 1234
                and meta.length(kept) == 2 and kept[0] == 1 and kept[1] == 3
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
            let matched=meta.find(values,|value| value match {null => true,_? => false});
            let missing=meta.find(values,|value| false);
            let kept=meta.filter(values,|value| value match {null => true,_? => false});
            (matched match {inner? => inner match {null => true,_? => false},null => false})
                and (missing match {_? => false,null => true})
                and meta.length(kept) == 2
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
                meta.{query}(input,pred);
            }}
            comptime fn recover()->bool {{
                let input:meta.List<bool> = [true]; meta.all(input,|value| value)
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
    for query in ["any", "all", "find", "filter"] {
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
                    "comptime fn run() {{let input:meta.List<bool> = {values}; meta.{query}(input,pred);}}"
                );
                compile(&format!(
                    "comptime fn pred(value:bool)->bool {{value}} {call}"
                ))
                .expect("the same call accepts a compatible predicate");
                let error = compile(&format!("{pred} {call}")).unwrap_err();
                assert!(
                    error.starts_with("elaboration failed:"),
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
        comptime fn input(trace:*Integer)->meta.List<Integer> {
            trace.* = trace.* * 10 + 1; [3,5,7]
        }
        comptime fn accepts(value:Integer)->bool {true}
        comptime fn producer(trace:*Integer)->comptime fn(Integer)->bool {
            trace.* = trace.* * 10 + 2; accepts
        }
        comptime fn run()->bool {
            var trace:Integer=0;
            let first=meta.find(input(trace.ref),producer(trace.ref));
            var visits:Integer=0;
            let values:meta.List<Integer> = [3,5,7];
            let second=meta.find(values,|value| {visits = visits + 1; visits == 2});
            trace == 12 and visits == 0
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
        comptime fn replace(target:*meta.List<bool>)->meta.List<bool> {
            target.* = [false]; [true]
        }
        comptime fn run()->bool {
            var values:meta.List<bool> = [true];
            let snapshot=values;
            meta.append(values.ref,false);
            let target=values.ref;
            target.* = .list_concat'builtin(target.*,replace(target));
            let frozen=target.*;
            meta.append(target,false);
            var nested:meta.List<meta.List<bool>> = [];
            meta.append(nested.ref,snapshot);
            meta.append(nested.ref,values);
            meta.length(snapshot) == 1 and snapshot[0]
                and meta.length(frozen) == 3 and frozen[0] and not frozen[1] and frozen[2]
                and meta.length(values) == 4 and not values[3]
                and meta.length(nested[0]) == 1 and meta.length(nested[1]) == 4
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
        comptime fn list_pattern_view_get_len(*self) -> Integer;
        comptime fn list_pattern_view_get_element(*self, comptime index: Integer) -> Element;
    }
    impl ListView for meta.List<T> where T {
        assoc Element: Type = T;
        comptime fn list_pattern_view_get_len(*self) -> Integer { meta.length(self.*) }
        comptime fn list_pattern_view_get_element(*self, comptime index: Integer) -> T {
            self.*[index]
        }
    }
";

fn compile_list_view(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} {source}",
            include_str!("../../../library/meta/lib.fl")
        ),
        &Default::default(),
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
            let middle = meta.slice(input, 1, 3);
            let full = meta.slice(input, 0, 4);
            let empty = meta.slice(input, 4, 4);
            let changed = meta.replace_range(input, 1, 3, [7,8,9]);
            let inserted = meta.replace_range(input, 0, 0, [7]);
            let appended = meta.replace_range(input, 4, 4, [9]);
            let removed = meta.replace_range<Integer>(input, 0, 4, []);
            (middle match {xs? => meta.length(xs) == 2 and xs[0] == 20 and xs[1] == 30, null => false})
                and (full match {xs? => meta.length(xs) == 4 and xs[3] == 40, null => false})
                and (empty match {xs? => meta.length(xs) == 0, null => false})
                and (changed match {xs? => meta.length(xs) == 5 and xs[0] == 10
                    and xs[1] == 7 and xs[2] == 8 and xs[3] == 9 and xs[4] == 40, null => false})
                and (inserted match {xs? => meta.length(xs) == 5 and xs[0] == 7 and xs[1] == 10, null => false})
                and (appended match {xs? => meta.length(xs) == 5 and xs[4] == 9, null => false})
                and (removed match {xs? => meta.length(xs) == 0, null => false})
                and meta.length(input) == 4 and input[1] == 20
                and (meta.slice<Integer>([], 0, 0) match {xs? => meta.length(xs) == 0, null => false})
                and (meta.replace_range<Integer>([], 0, 0, [5]) match {xs? => xs[0] == 5, null => false})
        }
        comptime fn invalid(start: Integer, end: Integer) -> bool {
            let input: meta.List<Integer> = [1,2];
            (meta.slice(input, start, end) match {null => true, _? => false})
                and (meta.replace_range(input, start, end, [3]) match {null => true, _? => false})
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
            meta.replace_range(fields, 1, 2, [bool]) match {
                xs? => Packet<...xs>, null => bool,
            }
        }
        comptime fn Direct() -> Type {Packet<bool,bool,bool>}
        comptime fn run() -> bool {
            let nested: meta.List<meta.List<bool>> = [[true],[],[false]];
            let optional: meta.List<?Integer> = [7,null,9];
            (meta.slice(nested, 1, 3) match {xs? => meta.length(xs) == 2
                and meta.length(xs[0]) == 0 and not xs[1][0], null => false})
                and (meta.slice(optional, 1, 2) match {xs? => meta.length(xs) == 1
                    and (xs[0] match {null => true, _? => false}), null => false})
                and (meta.replace_range<?Integer>(optional, 0, 3, [null]) match {
                    xs? => meta.length(xs) == 1 and (xs[0] match {null => true, _? => false}), null => false})
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
            let changed = meta.replace_range(input(trace.ref, original),
                bound(trace.ref, 2, 1), bound(trace.ref, 3, 2), replacement(trace.ref, original.ref));
            let first = trace == 1234 and meta.length(original) == 0
                and (changed match {xs? => meta.length(xs) == 3 and xs[0] == 1
                    and xs[1] == 9 and xs[2] == 3, null => false});
            trace = 0;
            original = [1,2,3];
            let invalid = meta.replace_range(input(trace.ref, original),
                bound(trace.ref, 2, -1), bound(trace.ref, 3, 2), replacement(trace.ref, original.ref));
            let second = trace == 1234 and meta.length(original) == 0
                and (invalid match {null => true, _? => false});
            trace = 0;
            let selected = meta.slice(input(trace.ref, [7,8,9]),
                bound(trace.ref, 2, 1), bound(trace.ref, 3, 3));
            first and second and trace == 123
                and (selected match {xs? => meta.length(xs) == 2 and xs[0] == 8 and xs[1] == 9, null => false})
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
        "comptime fn bad() {meta.slice<bool>([true], false, 1);}",
        "comptime fn bad() {meta.replace_range<bool>([true], 0, 1, [7]);}",
        "comptime fn bad() {meta.slice<bool>(true, 0, 1);}",
        "fn bad() {meta.slice<bool>([true], 0, 1);}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    let package = compile(
        "comptime fn run() -> bool {
        meta.slice<bool>([true,false,true,false], 0, 4) match {
            xs? => meta.length(xs) == 4, null => false,
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
    let package = compile(&format!("typealias Integer=.Integer'builtin;{source}")).unwrap();
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
            let output=meta.flat_map(values,expand);
            meta.length(output) == 4 and output[0] == Identity<bool>
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
            comptime fn fail()->meta.List<bool> {{let values:meta.List<Integer> = [0,2,1];meta.{operation}(values,transform)}}
            comptime fn recover()->meta.List<bool> {{let values:meta.List<Integer> = [0,1];meta.{operation}(values,transform)}}"
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
        "comptime fn bad()->meta.List<bool> {meta.filter_map([true],|value| [value])}",
        "comptime fn bad()->meta.List<bool> {meta.flat_map([true],|value| value)}",
        "fn callback(value:bool)->?bool {value} comptime fn bad()->meta.List<bool> {meta.filter_map([true],callback)}",
        "fn bad()->meta.List<bool> {meta.flat_map([true],|value| [value])}",
        "comptime fn bad() {meta.reduce([true],|left,right| [left]);}",
        "comptime fn bad() {meta.find_map([true],|value| [value]);}",
        "fn callback(value:bool)->?bool {value} comptime fn bad() {meta.find_map([true],callback);}",
        "fn bad() {meta.reduce([true],|left,right| right);}",
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
                    "comptime fn run() {{let input:meta.List<bool> = {values}; meta.{query}(input,callback);}}"
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
            let reduced=meta.reduce(values,|left,right|left);
            let found=meta.find_map(values,select);
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
            "meta.reduce(values,transform)",
        ),
        (
            "comptime fn transform(value:Integer)->?Integer {let values:meta.List<Integer> = [10];let checked=values[value];null}",
            "meta.find_map(values,transform)",
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
