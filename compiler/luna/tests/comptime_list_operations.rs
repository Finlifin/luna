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
fn list_queries_reject_runtime_or_nonboolean_predicates() {
    for query in ["any", "all", "find", "filter"] {
        for pred in [
            "fn pred(value:bool)->bool {value}",
            "comptime fn pred(value:bool)->Char {'x'}",
        ] {
            let source = format!(
                "{pred} comptime fn bad() {{let input:meta.List<bool> = [true]; meta.{query}(input,pred);}}"
            );
            assert!(compile(&source).is_err(), "accepted {source}");
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
