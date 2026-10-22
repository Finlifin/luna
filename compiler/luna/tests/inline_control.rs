//! Inline control flow must select runtime residuals at comptime.
#[path = "support/execution.rs"]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_protocols(source, &Default::default())
}

#[test]
fn inline_ranges_unroll_runtime_bodies_and_bind_indices() {
    let package = compile(
        "fn first() {} fn rest() {}
        fn run()->bool {inline for index in 0..3 {
            inline if index==0 {first();} else {rest();}
        } true}
        fn inclusive()->bool {inline for index in (-1)..=1 {rest();} true}
        fn empty()->bool {inline for index in 2..1 {rest();} true}",
    )
    .unwrap();
    for (entry, calls) in [
        ("run", vec!["run", "first", "rest", "rest"]),
        ("inclusive", vec!["inclusive", "rest", "rest", "rest"]),
        ("empty", vec!["empty"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), calls);
    }
}

#[test]
fn inline_range_break_continue_and_runtime_state_preserve_scope() {
    let package = compile(
        "fn visit() {}
        fn run()->bool {var flag=false;
            inline for index in 0..5 {
                inline if index==1 {continue;}
                flag=not flag;
                visit();
                inline if index==3 {break;}
            }
            flag
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "visit", "visit", "visit"]
    );
}

#[test]
fn inline_patterns_execute_nested_guards_and_transformations() {
    let package = compile(
        "comptime fn accepts(value:bool)->bool {not value}
        comptime fn rejects(value:bool)->bool {value}
        comptime fn flip(value:bool)->bool {not value}
        comptime fn unreachable_value()->bool {unreachable_value()}
        fn nested()->bool {inline if (false,true) is do {
            ((first if accepts(first)),second)=>not first and second,
            _=>false,
        }}
        fn retry()->bool {inline if false is do {
            ((result if rejects(result)) or result)=>not result,
            _=>false,
        }}
        fn transform()->bool {inline if false is do {
            (first and flip(first) is result)=>result,
            _=>false,
        }}
        fn skipped()->bool {inline if false is do {
            (true and unreachable_value() is result)=>result,
            _=>true,
        }}",
    )
    .unwrap();
    for entry in ["nested", "retry", "transform", "skipped"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), vec![entry]);
    }
}

#[test]
fn inline_match_specializes_inputs_and_remaps_or_bindings() {
    let package = compile(
        "fn choose(comptime flag:bool,value:bool)->bool {
            inline if flag is do {true=>value,false=>not value,}
        }
        fn yes()->bool {choose(true,true)}
        fn no()->bool {choose(false,false)}
        fn remap()->bool {inline if (false,false) is do {
            (true,result) or (false,result)=>not result,
        }}",
    )
    .unwrap();
    for entry in ["yes", "no", "remap"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for source in [
        "fn bad(flag:bool)->bool {inline if flag is do {true=>true,false=>false,}}",
        "fn accept(value:bool)->bool {value} fn bad()->bool {inline if true is do {true=>true,false=>accept(3),}}",
        "fn bad()->bool {inline if true is do {true=>true,}}",
        "fn guard(value:bool)->bool {value} fn bad()->bool {inline if true is do {value if guard(value)=>value,_=>false,}}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inline_match_selects_and_delivers_pattern_bindings() {
    let package = compile(
        "comptime fn input()->(bool,bool) {(true,false)}
        fn run()->bool {inline if input() is do {(true,second)=>not second,_=>false,}}
        fn direct()->bool {inline if false is false do {true} else {false}}",
    )
    .unwrap();
    for entry in ["run", "direct"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), vec![entry]);
    }
}

#[test]
fn inline_match_guards_and_nested_selection_keep_staging() {
    let package = compile(
        "comptime fn accepts(value:bool)->bool {not value}
        comptime fn unreachable_condition()->bool {unreachable_condition()}
        fn run()->bool {inline if false is do {
            value if accepts(value)=>{inline if value {false} else {true}},
            _ if unreachable_condition()=>false,
            _=>false,
        }}
        fn retry()->bool {inline if false is do {
            value if not accepts(value)=>{inline if unreachable_condition() {false} else {false}},
            value=>not value,
        }}
        fn nested_unselected()->bool {inline if true {true} else {
            inline if unreachable_condition() is do {true=>false,false=>false,}
        }}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for entry in ["retry", "nested_unselected"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inline_when_selects_first_match_without_invoking_later_conditions() {
    let package = compile("comptime fn no()->bool {false}
        comptime fn yes()->bool {true}
        comptime fn unreachable_condition()->bool {unreachable_condition()}
        fn first()->bool {true} fn second()->bool {false}
        fn run()->bool {inline when {
            no()=>second(), yes()=>first(), unreachable_condition()=>second(), else=>second(),
        }}
        fn empty()->bool {inline when {false=>second(),} true}
        fn nested_when()->bool {inline when {
            no()=>{inline if unreachable_condition() {false} else {false}},
            yes()=>{inline when {true=>first(),unreachable_condition()=>second(),else=>second(),}},
            else=>second(),
        }}
        fn nested()->bool {inline if true {true} else {inline when {unreachable_condition()=>false,else=>false,}}}").unwrap();
    for (entry, calls) in [
        ("run", vec!["run", "first"]),
        ("empty", vec!["empty"]),
        ("nested_when", vec!["nested_when", "first"]),
        ("nested", vec!["nested"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), calls);
    }
}

#[test]
fn inline_when_instantiates_inputs_and_checks_every_arm() {
    let package = compile(
        "fn choose(comptime first:bool,comptime second:bool,value:bool)->bool {
        inline when {first=>value,second=>not value,else=>false,}
    }
    fn yes()->bool {choose(true,true,true)}
    fn no()->bool {choose(false,true,false)}
    fn fallback()->bool {not choose(false,false,true)}",
    )
    .unwrap();
    for entry in ["yes", "no", "fallback"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for source in [
        "fn bad(flag:bool)->bool {inline when {flag=>true,else=>false,}}",
        "fn accept(value:bool)->bool {value} fn bad()->bool {inline when {true=>true,else=>accept(3),}}",
        "fn bad()->bool {inline when {else=>true,true=>false,}}",
        "fn bad()->bool {inline when {true=>true,3=>false,else=>false,}}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inline_if_specializes_comptime_parameters_and_preserves_runtime_inputs() {
    let package = compile(
        "fn choose(comptime flag:bool,value:bool)->bool {
        inline if flag {value} else {not value}
    }
    fn yes()->bool {choose(true,true)}
    fn no()->bool {choose(false,false)}",
    )
    .unwrap();
    for entry in ["yes", "no"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inline_if_handles_nested_selection_and_absent_else() {
    let package = compile(
        "comptime fn flag()->bool {false}
        fn first()->bool {true} fn second()->bool {false}
        fn run()->bool {inline if flag() {second()} else {inline if true {first()} else {second()}}}
        fn empty()->bool {inline if false {second();} true}",
    )
    .unwrap();
    for (entry, calls) in [("run", vec!["run", "first"]), ("empty", vec!["empty"])] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), calls);
    }
}

#[test]
fn unselected_nested_inline_conditions_are_checked_without_invocation() {
    let package = compile("comptime fn unreachable_condition()->bool {unreachable_condition()}
        fn run()->bool {inline if true {true} else {inline if unreachable_condition() {false} else {false}}}").unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        compile(
            "fn accept(value:bool)->bool {value}
        fn choose(comptime flag:bool,value:bool)->bool {inline if flag {value} else {accept(3)}}
        fn run()->bool {choose(true,true)}"
        )
        .is_err()
    );
    assert!(
        compile(
            "fn choose(comptime flag:bool,value:bool)->bool {inline if value {flag} else {false}}
        fn run()->bool {choose(true,true)}"
        )
        .is_err()
    );
}

#[test]
fn inline_if_selects_runtime_code_after_ordinary_branch_checks() {
    let package = compile(
        "comptime fn selected()->bool {true}
        fn first()->bool {true} fn second()->bool {false}
        fn run()->bool {inline if selected() {first()} else {second()}}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), vec!["run", "first"]);
    for source in [
        "fn run(flag:bool)->bool {inline if flag {true} else {false}}",
        "fn accept(value:bool)->bool {value} fn run()->bool {inline if true {true} else {accept(3)}}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inline_ranges_specialize_staged_endpoints_and_skip_inactive_invocations() {
    let package = compile(
        "typealias Integer=.Integer'builtin; comptime fn forever()->Integer {forever()}
        fn visit() {}
        fn repeat(comptime end:Integer) {
            inline for i in 0..end {inline if i==1 {visit();}}
        }
        fn run()->bool {
            repeat(3);
            inline if false {inline for i in 0..forever() {visit();}}
            inline for i in 0..0 {inline for j in 0..forever() {visit();}}
            true
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "visit")
            .count(),
        1
    );
}

#[test]
fn inline_ranges_check_unused_bodies_and_reject_runtime_bounds() {
    for source in [
        "fn run(){inline for i in 0..0 {let wrong:bool=3;}}",
        "typealias i32=.i32'builtin; fn run(end:i32){inline for i in 0..end {}}",
        "fn run(){inline for i in 0..100001 {}}",
        "fn run(){inline if false {inline for i in 0..1 {let wrong:bool=3;}}}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn inline_ranges_preserve_nested_labels_runtime_transfers_and_returns() {
    let package = compile(
        "fn visit() {}
        fn run(flag:bool)->bool {
            inline for:outer i in 0..3 {
                inline for j in 0..2 {
                    visit();
                    if flag {continue outer;}
                    break outer;
                }
            }
            flag
        }
        fn early()->bool {inline for i in 0..3 {return true;} false}",
    )
    .unwrap();
    for (flag, count) in [(true, 3), (false, 1)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(flag)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "visit")
                .count(),
            count
        );
    }
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "early"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inline_range_indices_select_staged_when_without_staging_runtime_bodies() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        fn first(value:bool)->bool {value}
        fn later(value:bool)->bool {not value}
        fn run(comptime count:Integer,value:bool)->bool {
            var result=value;
            inline for i in 0..count {
                inline when {i==0=>{result=first(result);},else=>{result=later(result);},}
            }
            result
        }
        fn entry()->bool {run(3,true)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "entry"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let calls = support::calls(&package, &vm);
    assert_eq!(calls.iter().filter(|name| **name == "first").count(), 1);
    assert_eq!(calls.iter().filter(|name| **name == "later").count(), 2);
}

#[test]
fn inline_when_rejects_runtime_dependencies_in_open_selectors() {
    let source = "typealias Integer=.Integer'builtin;
        fn run(comptime count:Integer,flag:bool) {
            inline for i in 0..count {
                inline when {flag=>{},else=>{},}
            }
        }
        fn entry(){run(2,true);}";
    let failure = compile(source).unwrap_err();
    assert!(failure.contains("runtime parameter"), "{failure}");
}

#[test]
fn inline_range_budget_and_inactive_body_diagnostics_are_specific() {
    for (source, message) in [
        (
            "fn run(){inline for i in 0..100001 {}}",
            "inline range expansion budget exhausted",
        ),
        (
            "fn run(){inline for i in 0..340282366920938463463374607431768211456 {}}",
            "inline range expansion budget exhausted",
        ),
        (
            "fn run(){inline for i in (true,false) {}}",
            "bounded comptime Integer range",
        ),
    ] {
        let failure = compile(source).unwrap_err();
        assert!(failure.contains(message), "{failure}");
    }
}

#[test]
fn inline_control_in_call_arguments_triggers_open_instantiation() {
    let package = compile(
        "fn keep(value:bool)->bool {value}
        fn choose(comptime flag:bool,value:bool)->bool {
            keep(do {inline if flag {value} else {false}})
        }
        fn yes()->bool {choose(true,true)}
        fn no()->bool {choose(false,true)}",
    )
    .unwrap();
    for (entry, wanted) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(wanted)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "keep")
                .count(),
            1
        );
    }
}

#[test]
fn nested_inline_call_arguments_keep_definition_checks() {
    for (source, message) in [
        (
            "fn keep(value:bool)->bool {value}
          fn choose(comptime flag:bool,value:bool)->bool {
              keep(do {inline if value {flag} else {false}})
          } fn entry()->bool {choose(true,true)}",
            "runtime parameter",
        ),
        (
            "fn keep(value:bool)->bool {value}
          fn choose(comptime flag:bool,value:bool)->bool {
              keep(do {inline if flag {value} else {keep(3)}})
          } fn entry()->bool {choose(true,true)}",
            "fixed-width integer target",
        ),
    ] {
        let failure = compile(source).unwrap_err();
        assert!(failure.contains(message), "{failure}");
    }
}

#[test]
fn inline_tuple_elements_preserve_staging_order_and_runtime_values() {
    let package = compile(
        "fn first(value:bool)->bool {value}
        fn second(value:bool)->bool {value}
        fn choose(comptime flag:bool,value:bool)->(bool,bool) {
            (do {inline if flag {first(value)} else {first(false)}},second(value))
        }
        fn yes()->(bool,bool) {choose(true,true)}
        fn no()->(bool,bool) {choose(false,true)}",
    )
    .unwrap();
    for (entry, first) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Tuple(vec![Value::Bool(first), Value::Bool(true)])
        );
        let calls = support::calls(&package, &vm);
        let calls: Vec<_> = calls
            .into_iter()
            .filter(|name| *name == "first" || *name == "second")
            .collect();
        assert_eq!(calls, vec!["first", "second"]);
    }
}

#[test]
fn inline_logical_operands_preserve_runtime_short_circuit() {
    let package = compile(
        "fn visit(value:bool)->bool {value}
        fn both(comptime flag:bool,left:bool,right:bool)->bool {
            left and do {inline if flag {visit(right)} else {visit(false)}}
        }
        fn either(comptime flag:bool,left:bool,right:bool)->bool {
            do {inline if flag {left} else {false}} or visit(right)
        }
        fn a()->bool {both(true,false,true)}
        fn b()->bool {both(true,true,true)}
        fn c()->bool {both(false,true,true)}
        fn d()->bool {either(true,true,false)}
        fn e()->bool {either(false,true,true)}",
    )
    .unwrap();
    for (entry, value, count) in [
        ("a", false, 0),
        ("b", true, 1),
        ("c", false, 1),
        ("d", true, 0),
        ("e", true, 1),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(value)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "visit")
                .count(),
            count
        );
    }
}

#[test]
fn inline_logical_operands_do_not_skip_ordinary_type_checks() {
    for source in [
        "fn choose(comptime flag:bool)->bool {false and do {inline if flag {3} else {4}}} fn entry()->bool {choose(true)}",
        "fn choose(comptime flag:bool)->bool {true or do {inline if flag {true} else {3}}} fn entry()->bool {choose(true)}",
        "fn choose(comptime flag:bool,value:bool)->bool {true or do {inline if value {flag} else {false}}} fn entry()->bool {choose(true,true)}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn unary_inline_operands_keep_runtime_evaluation_once() {
    let package = compile(
        "typealias i32=.i32'builtin;
        fn visit(value:bool)->bool {value}
        fn number(value:i32)->i32 {value}
        fn invert(comptime flag:bool,value:bool)->bool {
            not do {inline if flag {visit(value)} else {visit(false)}}
        }
        fn negate(comptime flag:bool,value:i32)->i32 {
            -do {inline if flag {7} else {3}}
        }
        fn a()->bool {invert(true,true)}
        fn b()->bool {invert(false,true)}
        fn c()->i32 {negate(true,7)}
        fn d()->i32 {negate(false,7)}",
    )
    .unwrap();
    for (entry, value, callee) in [
        ("a", Value::Bool(false), "visit"),
        ("b", Value::Bool(true), "visit"),
        ("c", Value::SignedInteger(-7), "negate"),
        ("d", Value::SignedInteger(-3), "negate"),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            value
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == callee)
                .count(),
            1
        );
    }
}

#[test]
fn unary_inline_operands_retain_type_phase_and_overflow_checks() {
    for (source,message) in [
        ("typealias i32=.i32'builtin;
          fn negate(comptime flag:bool,value:i32)->i32 { -do {inline if flag {value} else {3}}}
          fn run(value:i32)->i32 {negate(true,value)}", "machine arithmetic range is not proven"),
        ("fn invert(comptime flag:bool)->bool {not do {inline if flag {3} else {4}}}
          fn run()->bool {invert(true)}", "integer"),
        ("fn invert(comptime flag:bool,value:bool)->bool {not do {inline if value {flag} else {false}}}
          fn run()->bool {invert(true,true)}", "runtime parameter"),
    ] {
        let failure=compile(source).unwrap_err();
        assert!(failure.contains(message),"{failure}");
    }
}

#[test]
fn inline_projection_evaluates_selected_receiver_once() {
    let package = compile(
        "struct Packet {value:bool}
        fn first(value:bool)->Packet {Packet {value:value}}
        fn second()->Packet {Packet {value:false}}
        fn choose(comptime flag:bool,value:bool)->bool {
            (do {inline if flag {first(value)} else {second()}}).value
        }
        fn yes()->bool {choose(true,true)}
        fn no()->bool {choose(false,true)}",
    )
    .unwrap();
    for (entry, value, selected) in [("yes", true, "first"), ("no", false, "second")] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(value)
        );
        let calls: Vec<_> = support::calls(&package, &vm)
            .into_iter()
            .filter(|name| name == "first" || name == "second")
            .collect();
        assert_eq!(calls, vec![selected]);
    }
}

#[test]
fn inline_projection_requires_field_and_valid_unselected_receiver() {
    for source in [
        "struct Packet {value:bool} fn choose(comptime flag:bool,packet:Packet)->bool {
            (do {inline if flag {packet} else {Packet {value:false}}}).missing
         } fn run()->bool {choose(true,Packet {value:true})}",
        "struct Packet {value:bool} fn choose(comptime flag:bool,packet:Packet)->bool {
            (do {inline if flag {packet} else {false}}).value
         } fn run()->bool {choose(true,Packet {value:true})}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inline_comparisons_preserve_operand_order_and_literal_context() {
    let package = compile(
        "typealias i32=.i32'builtin;
        fn first(value:i32)->i32 {value}
        fn second(value:i32)->i32 {value}
        fn compare(comptime flag:bool,a:i32,b:i32)->bool {
            (do {inline if flag {first(a)} else {first(b)}}) < second(b)
        }
        fn literal(comptime flag:bool,value:i32)->bool {
            -2147483648 < do {inline if flag {value} else {0}}
        }
        fn yes()->bool {compare(true,2,3)}
        fn no()->bool {compare(false,2,3)}
        fn edge()->bool {literal(true,7)}",
    )
    .unwrap();
    for (entry, value) in [("yes", true), ("no", false), ("edge", true)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(value)
        );
        if entry != "edge" {
            let calls: Vec<_> = support::calls(&package, &vm)
                .into_iter()
                .filter(|name| name == "first" || name == "second")
                .collect();
            assert_eq!(calls, vec!["first", "second"]);
        }
    }
}

#[test]
fn inline_operands_support_all_scalar_comparisons() {
    for (op, wanted) in [
        ("==", false),
        ("!=", true),
        ("<", true),
        ("<=", true),
        (">", false),
        (">=", false),
    ] {
        let source=format!("typealias i32=.i32'builtin;
            fn compare(comptime flag:bool,a:i32,b:i32)->bool {{
                (do {{inline if flag {{a}} else {{b}}}}) {op} (do {{inline if flag {{b}} else {{a}}}})
            }} fn run()->bool {{compare(true,2,3)}}");
        let package = compile(&source).unwrap();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(wanted),
            "{op}"
        );
    }
}

#[test]
fn inline_comparisons_retain_peer_type_and_literal_range_checks() {
    for source in [
        "typealias u8=.u8'builtin;
         fn compare(comptime flag:bool,value:u8)->bool {(do {inline if flag {value} else {0}})==256}
         fn run()->bool {compare(true,1)}",
        "typealias u8=.u8'builtin;
         fn compare(comptime flag:bool,value:u8)->bool { -1<(do {inline if flag {value} else {0}})}
         fn run()->bool {compare(true,1)}",
        "fn compare(comptime flag:bool,value:bool)->bool {(do {inline if flag {value} else {false}})==3}
         fn run()->bool {compare(true,true)}",
    ] { assert!(compile(source).is_err(),"accepted {source}"); }
}

#[test]
fn inline_arithmetic_operands_keep_context_and_selected_values() {
    for (op, wanted) in [("+", 9), ("-", 3), ("*", 18)] {
        let source=format!("typealias i32=.i32'builtin;
            fn calculate(comptime flag:bool)->i32 {{
                (do {{inline if flag {{6}} else {{4}}}}) {op} (do {{inline if flag {{3}} else {{2}}}})
            }} fn run()->i32 {{calculate(true)}}");
        let package = compile(&source).unwrap();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::SignedInteger(wanted),
            "{op}"
        );
    }
}

#[test]
fn inline_unsigned_division_and_remainder_keep_safety_checks() {
    for (op, wanted) in [("/", 2), ("%", 1)] {
        let source=format!("typealias u8=.u8'builtin;
            fn calculate(comptime flag:bool)->u8 {{
                (do {{inline if flag {{7}} else {{4}}}}) {op} (do {{inline if flag {{3}} else {{2}}}})
            }} fn run()->u8 {{calculate(true)}}");
        let package = compile(&source).unwrap();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::UnsignedInteger(wanted),
            "{op}"
        );
    }
    for (expr, message) in [
        (
            "(do {inline if flag {255} else {2}}) + 1",
            "range is not proven",
        ),
        (
            "7 / (do {inline if flag {0} else {1}})",
            "divisor is not proven nonzero",
        ),
    ] {
        let source = format!(
            "typealias u8=.u8'builtin; fn calculate(comptime flag:bool)->u8 {{{expr}}} fn run()->u8 {{calculate(true)}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(failure.contains(message), "{failure}");
    }
}

#[test]
fn inline_arithmetic_keeps_runtime_operand_once() {
    let package = compile(
        "typealias i32=.i32'builtin;
        fn visit(value:i32)->i32 {value}
        fn calculate(comptime flag:bool,value:i32)->i32 {
            (do {inline if flag {visit(value)} else {visit(3)}}) + 0
        } fn run()->i32 {calculate(true,7)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::SignedInteger(7)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "visit")
            .count(),
        1
    );
}

#[test]
fn inline_bit_concat_retains_distinct_widths_and_input_order() {
    let package = compile(
        "typealias High=.BitVec'builtin<2>;typealias Low=.BitVec'builtin<3>;
        typealias Word=.BitVec'builtin<5>;typealias Empty=.BitVec'builtin<0>;
        fn high(value:High)->High {value}
        fn low(value:Low)->Low {value}
        fn empty()->Empty {0}
        fn join(comptime flag:bool,a:High,b:Low)->Word {
            (do {inline if flag {high(a)} else {high(0)}}) ++ low(b)
        }
        fn identity(comptime flag:bool,value:Low)->Low {
            (do {inline if flag {empty()} else {empty()}}) ++ low(value)
        }
        fn yes()->bool {join(true,2,5)==21}
        fn no()->bool {join(false,2,5)==5}
        fn zero()->bool {identity(true,5)==5}",
    )
    .unwrap();
    for entry in ["yes", "no", "zero"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        let calls: Vec<_> = support::calls(&package, &vm)
            .into_iter()
            .filter(|name| name == "high" || name == "low" || name == "empty")
            .collect();
        assert_eq!(
            calls,
            if entry == "zero" {
                vec!["empty", "low"]
            } else {
                vec!["high", "low"]
            }
        );
    }
}

#[test]
fn inline_bit_concat_rejects_wrong_result_width() {
    let failure = compile(
        "typealias High=.BitVec'builtin<2>;typealias Low=.BitVec'builtin<3>;
        typealias Wrong=.BitVec'builtin<4>;
        fn join(comptime flag:bool,a:High,b:Low)->Wrong {
            (do {inline if flag {a} else {a}}) ++ b
        } fn run()->Wrong {join(true,2,5)}",
    )
    .unwrap_err();
    assert!(failure.starts_with("elaboration failed:"), "{failure}");
}

#[test]
fn inline_bit_receivers_support_checked_indices_and_slices() {
    let package = compile(
        "typealias Bits=.BitVec'builtin<8>;typealias Low=.BitVec'builtin<4>;
        typealias Bit=.BitVec'builtin<1>;typealias Empty=.BitVec'builtin<0>;
        fn visit(value:Bits)->Bits {value}
        fn slice(comptime flag:bool,value:Bits)->Low {
            (do {inline if flag {visit(value)} else {visit(0)}})[0..4]
        }
        fn bit(comptime flag:bool,value:Bits)->Bit {
            (do {inline if flag {visit(value)} else {visit(0)}})[7]
        }
        fn empty(comptime flag:bool,value:Bits)->Empty {
            (do {inline if flag {visit(value)} else {visit(0)}})[8..8]
        }
        fn low()->bool {slice(true,175)==15}
        fn high()->bool {bit(true,175)==1}
        fn zero()->bool {empty(true,175)==0}",
    )
    .unwrap();
    for entry in ["low", "high", "zero"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "visit")
                .count(),
            1
        );
    }
}

#[test]
fn inline_bit_receivers_reject_invalid_indices() {
    for index in ["8", "9..9", "4..3", "0..9"] {
        let source=format!("typealias Bits=.BitVec'builtin<8>;
            fn bad(comptime flag:bool,value:Bits) {{let result=(do {{inline if flag {{value}} else {{value}}}})[{index}];}}
            fn run(){{bad(true,0);}}");
        let failure = compile(&source).unwrap_err();
        assert!(failure.starts_with("elaboration failed:"), "{failure}");
    }
}

#[test]
fn inline_bit_conversion_preserves_widening_and_single_evaluation() {
    let package = compile(
        "typealias Bits=.BitVec'builtin<4>;typealias u8=.u8'builtin;
        fn visit(value:Bits)->Bits {value}
        fn convert(comptime flag:bool,value:Bits)->u8 {
            (do {inline if flag {visit(value)} else {visit(0)}}).as(u8)
        }
        fn yes()->u8 {convert(true,15)}
        fn no()->u8 {convert(false,15)}",
    )
    .unwrap();
    for (entry, wanted) in [("yes", 15), ("no", 0)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::UnsignedInteger(wanted)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "visit")
                .count(),
            1
        );
    }
}

#[test]
fn inline_bit_conversions_retain_representation_restrictions() {
    for (width, target, message) in [
        (16, ".u8'builtin", "narrowing"),
        (4, ".i8'builtin", "exactly the target width"),
        (8, "bool", "unrestricted scalar"),
    ] {
        let source = format!(
            "typealias Bits=.BitVec'builtin<{width}>;typealias Target={target};
            fn convert(comptime flag:bool,value:Bits)->Target {{
                (do {{inline if flag {{value}} else {{value}}}}).as(Target)
            }} fn run()->Target {{convert(true,0)}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(failure.contains(message), "{failure}");
    }
}

#[test]
fn inline_bit_views_keep_signed_representation_and_source_execution() {
    let package = compile(
        "typealias i8=.i8'builtin;typealias Bits=.BitVec'builtin<8>;
        fn visit(value:i8)->i8 {value}
        fn bits(comptime flag:bool,value:i8)->Bits {
            (do {inline if flag {visit(value)} else {visit(0)}})'bitvec
        }
        fn yes()->bool {bits(true,-5)==251}
        fn no()->bool {bits(false,-5)==0}",
    )
    .unwrap();
    for entry in ["yes", "no"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "visit")
                .count(),
            1
        );
    }
}

#[test]
fn inline_bit_views_reject_unsupported_or_mismatched_representations() {
    for source in [
        "typealias Bits=.BitVec'builtin<8>;
         fn bits(comptime flag:bool,value:bool)->Bits {(do {inline if flag {value} else {false}})'bitvec}
         fn run()->Bits {bits(true,true)}",
        "typealias i8=.i8'builtin;typealias Bits=.BitVec'builtin<4>;
         fn bits(comptime flag:bool,value:i8)->Bits {(do {inline if flag {value} else {0}})'bitvec}
         fn run()->Bits {bits(true,7)}",
    ] {
        let failure=compile(source).unwrap_err();
        assert!(failure.starts_with("elaboration failed:"),"{failure}");
    }
}

#[test]
fn nested_open_operator_descriptions_remain_bounded_and_executable() {
    let source = format!(
        "fn choose(comptime flag:bool,value:bool)->bool {{
        {}(do {{inline if flag {{value}} else {{false}}}})
    }} fn run()->bool {{choose(true,true)}}",
        "not ".repeat(32)
    );
    let package = compile(&source).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inline_assignment_rhs_preserves_mutation_and_single_evaluation() {
    let package = compile(
        "fn visit(value:bool)->bool {value}
        fn choose(comptime flag:bool,value:bool)->bool {
            var result=false;
            result=do {inline if flag {visit(value)} else {visit(false)}};
            result
        } fn yes()->bool {choose(true,true)} fn no()->bool {choose(false,true)}",
    )
    .unwrap();
    for (entry, wanted) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(wanted)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "visit")
                .count(),
            1
        );
    }
}

#[test]
fn inline_assignment_rhs_retains_mutability_and_type_checks() {
    for source in [
        "fn choose(comptime flag:bool)->bool {let result=false;result=do {inline if flag {true} else {false}};result} fn run()->bool {choose(true)}",
        "fn choose(comptime flag:bool)->bool {var result=false;result=do {inline if flag {true} else {3}};result} fn run()->bool {choose(true)}",
        "fn choose(comptime flag:bool,value:bool)->bool {var result=false;result=do {inline if value {flag} else {false}};result} fn run()->bool {choose(true,true)}",
    ] {
        let failure = compile(source).unwrap_err();
        assert!(failure.starts_with("elaboration failed:"), "{failure}");
    }
}

#[test]
fn runtime_assignment_requires_evidence_from_suspended_input() {
    let failure = compile(
        "typealias Type=.Type'builtin;
        comptime fn Domain(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn choose(comptime flag:bool,value:defer Domain(flag))->bool {
            var result=false;result=value;result
        } fn run()->bool {choose(true,true)}",
    )
    .unwrap_err();
    assert!(failure.contains("value-type evidence"), "{failure}");
}

#[test]
fn inline_field_assignment_keeps_the_original_place() {
    let package = compile(
        "struct Cell {flag:bool} struct Pair {left:Cell,right:Cell}
        fn visit(value:bool)->bool {value}
        fn choose(comptime flag:bool,value:bool)->bool {
            var pair=Pair {left:Cell {flag:false},right:Cell {flag:true}};
            pair.left.flag=do {inline if flag {visit(value)} else {visit(false)}};
            pair.left.flag and pair.right.flag
        } fn yes()->bool {choose(true,true)} fn no()->bool {choose(false,true)}",
    )
    .unwrap();
    for (entry, wanted) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(wanted)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| **name == "visit")
                .count(),
            1
        );
    }
}

#[test]
fn inline_field_assignment_retains_field_and_value_validation() {
    for statement in [
        "cell.missing=do {inline if flag {true} else {false}};",
        "cell.flag=do {inline if flag {true} else {3}};",
    ] {
        let source=format!("struct Cell {{flag:bool}}
            fn choose(comptime flag:bool)->bool {{var cell=Cell {{flag:false}};{statement}cell.flag}}
            fn run()->bool {{choose(true)}}");
        let failure = compile(&source).unwrap_err();
        assert!(failure.starts_with("elaboration failed:"), "{failure}");
    }
}

#[test]
fn pure_inline_definitions_keep_ordinary_behavior_evidence() {
    let package = compile(
        "pure fn choose(comptime flag:bool,value:bool)->bool {
            inline if flag {value} else {not value}
        }
        pure fn repeat(comptime count:.Integer'builtin,value:bool)->bool {
            var result=value; inline for i in 0..count {result=not result;} result
        }
        fn run()->bool {choose(true,true) and repeat(2,true)}",
    )
    .unwrap();
    assert!(
        package
            .functions
            .iter()
            .all(|function| !function.name.contains("[ordinary]"))
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_inline_definitions_reject_effects_in_unselected_branches() {
    for source in [
        "pure fn bad(comptime flag:bool,p:*bool)->bool {inline if flag {true} else {p.* = false;false}} fn run(p:*bool)->bool {bad(true,p)}",
        "pure fn bad(comptime count:.Integer'builtin,p:*bool)->bool {inline for i in 0..count {p.* = false;} true} fn run(p:*bool)->bool {bad(0,p)}",
    ] {
        let failure = compile(source).unwrap_err();
        assert!(failure.contains("P4 evidence"), "{failure}");
    }
}

#[test]
fn pure_open_behavior_checks_calls_and_unused_definitions() {
    let package = compile(
        "fn invert(value:bool)->bool {not value}
        pure fn choose(comptime flag:bool,value:bool)->bool {
            inline if flag {invert(value)} else {value}
        } fn run()->bool {choose(true,false)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    for source in [
        "fn write(p:*bool)->bool {p.* = false;false}
         pure fn bad(comptime flag:bool,p:*bool)->bool {inline if flag {true} else {write(p)}}
         fn run(p:*bool)->bool {bad(true,p)}",
        "pure fn unused(comptime flag:bool,p:*bool)->bool {inline if flag {true} else {p.* = false;false}}
         fn run()->bool {true}",
    ] {
        let failure=compile(source).unwrap_err();
        assert!(failure.contains("P4 evidence"),"{failure}");
    }
}

#[test]
fn pure_open_behavior_includes_contracts_of_unused_definitions() {
    for clause in ["ensures"] {
        let source = format!(
            "fn write(p:*bool)->bool {{p.* = false;true}}
            pure fn unused(comptime flag:bool,p:*bool)->bool where {clause} write(p)==true {{
                inline if flag {{true}} else {{false}}
            }} fn run()->bool {{true}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(
            failure.contains("effect-free program predicate"),
            "{failure}"
        );
    }
    let package = compile(
        "pure fn choose(comptime flag:bool,value:bool)->result:bool
        where ensures result == value {inline if flag {value} else {value}}
        fn run()->bool {choose(true,true)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_inline_loop_indices_remain_abstract_in_behavior_checks() {
    let package = compile(
        "pure fn indexed(comptime count:.Integer'builtin,value:bool)->bool {
        var result=value;
        inline for i in 0..count {inline if i==0 {result=not result;}}
        result
    } fn run()->bool {indexed(2,true)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(false)
    );
    let failure = compile(
        "pure fn bad(comptime count:.Integer'builtin,p:*bool)->bool {
        inline for i in 0..count {inline if i==1 {p.* = false;}}
        true
    } fn run(p:*bool)->bool {bad(1,p)}",
    )
    .unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}

#[test]
fn nested_pure_inline_indices_do_not_escape_behavior_witnesses() {
    let package = compile(
        "pure fn nested(comptime count:.Integer'builtin,value:bool)->bool {
        var result=value;
        inline for i in 0..count {inline for j in 0..count {
            inline if i + j==0 {result=not result;}
        }} result
    } fn run()->bool {nested(2,true)}",
    )
    .unwrap();
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "nested")
        .unwrap();
    assert_eq!(function.parameters.len(), 2);
    assert!(
        package
            .functions
            .iter()
            .all(|function| !function.name.contains("[ordinary]"))
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn pure_inline_comptime_selectors_are_not_runtime_calls() {
    let package = compile(
        "comptime fn invert(value:bool)->bool {not value}
        comptime fn forever()->bool {forever()}
        pure fn skipped(comptime flag:bool)->bool {
            inline if flag {true} else {inline if forever() {false} else {false}}
        }
        pure fn branch(comptime flag:bool,value:bool)->bool {
            inline if invert(flag) {not value} else {value}
        }
        pure fn arms(comptime flag:bool,value:bool)->bool {
            inline when {invert(flag)=>not value,else=>value,}
        }
        fn run()->bool {branch(true,true) and arms(false,false) and skipped(true)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !support::calls(&package, &vm)
            .iter()
            .any(|name| name == "invert" || name == "forever")
    );
    let failure = compile(
        "comptime fn invert(value:bool)->bool {not value}
        pure fn bad(comptime flag:bool,p:*bool)->bool {
            inline if invert(flag) {p.* = false;false} else {true}
        } fn run(p:*bool)->bool {bad(true,p)}",
    )
    .unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}

#[test]
fn pure_inline_loop_boundaries_remain_comptime() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        comptime fn bound(count:Integer)->Integer {count}
        comptime fn forever()->Integer {forever()}
        pure fn repeat(comptime count:Integer,value:bool)->bool {
            var result=value;
            inline for i in bound(0)..bound(count) {result=not result;}
            result
        }
        pure fn skipped(comptime flag:bool)->bool {
            inline if flag {true} else {
                inline for i in 0..forever() {} false
            }
        }
        fn run()->bool {repeat(2,true) and skipped(true)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !support::calls(&package, &vm)
            .iter()
            .any(|name| name == "bound" || name == "forever")
    );
    let failure = compile(
        "typealias Integer=.Integer'builtin;
        comptime fn bound(count:Integer)->Integer {count}
        pure fn bad(comptime count:Integer,p:*bool)->bool {
            inline for i in 0..bound(count) {p.* = false;} true
        } fn run(p:*bool)->bool {bad(0,p)}",
    )
    .unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}

#[test]
fn pure_inline_matches_abstract_comptime_subjects() {
    let package = compile(
        "comptime fn input(flag:bool)->(bool,bool) {(flag,false)}
        comptime fn forever()->bool {forever()}
        pure fn choose(comptime flag:bool,value:bool)->bool {
            inline if input(flag) is do {
                (true,second)=>value and not second,
                (false,second)=>not value or second,
            }
        }
        pure fn skipped(comptime flag:bool)->bool {
            inline if flag {true} else {
                inline if forever() is do {true=>false,false=>false,}
            }
        }
        fn run()->bool {choose(true,true) and choose(false,false) and skipped(true)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !support::calls(&package, &vm)
            .iter()
            .any(|name| name == "input" || name == "forever")
    );
    let failure = compile(
        "comptime fn input(flag:bool)->bool {flag}
        pure fn bad(comptime flag:bool,p:*bool)->bool {
            inline if input(flag) is do {true=>true,false=>{p.* = false;false},}
        } fn run(p:*bool)->bool {bad(true,p)}",
    )
    .unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}

#[test]
fn pure_inline_pattern_computations_are_staged() {
    let package = compile(
        "comptime fn accepts(value:bool)->bool {not value}
        comptime fn flip(value:bool)->bool {not value}
        comptime fn forever()->bool {forever()}
        pure fn guarded(comptime flag:bool,value:bool)->bool {
            inline if flag is do {
                candidate if accepts(candidate)=>value,
                _=>not value,
            }
        }
        pure fn transform(comptime flag:bool)->bool {
            inline if flag is do {(first and flip(first) is result)=>result,}
        }
        pure fn skipped(comptime flag:bool)->bool {
            inline if flag is do {true=>true,_ if forever()=>false,_=>false,}
        }
        pure fn nested(comptime flag:bool)->bool {
            inline if (flag,true) is do {
                ((candidate if accepts(candidate)),second)=>not candidate and second,
                _=>false,
            }
        }
        fn run()->bool {guarded(false,true) and transform(false) and skipped(true) and nested(false)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !support::calls(&package, &vm)
            .iter()
            .any(|name| matches!(name.as_str(), "accepts" | "flip" | "forever"))
    );
    let failure = compile(
        "comptime fn never(value:bool)->bool {false}
        pure fn bad(comptime flag:bool,p:*bool)->bool {
            inline if flag is do {value if never(value)=>{p.* = false;false},_=>true,}
        } fn run(p:*bool)->bool {bad(true,p)}",
    )
    .unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}

#[test]
fn pure_inline_pattern_guards_cannot_capture_runtime_parameters() {
    for source in [
        "comptime fn accepts(value:bool)->bool {value}
         pure fn bad(comptime flag:bool,value:bool)->bool {
             inline if flag is do {candidate if accepts(value)=>candidate,_=>false,}
         } fn run()->bool {bad(true,true)}",
        "pure fn bad(comptime flag:bool,value:bool)->bool {
             inline if flag is do {(candidate and value is result)=>result,}
         } fn run()->bool {bad(true,true)}",
    ] {
        let failure = compile(source).unwrap_err();
        assert!(failure.starts_with("elaboration failed:"), "{failure}");
    }
}

#[test]
fn inline_range_guards_filter_each_index_at_comptime() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        comptime fn keep(i:Integer)->bool {i==1 or i==3}
        comptime fn forever()->bool {forever()}
        pure fn filtered(comptime count:Integer,value:bool)->bool {
            var result=value;
            inline for i if keep(i) in 0..count {result=not result;}
            result
        }
        pure fn empty()->bool {
            inline for i if forever() in 0..0 {} true
        }
        fn skipped()->bool {
            inline for i if false in 0..3 {
                inline if forever() {}
            } true
        }
        fn run()->bool {filtered(4,true) and not filtered(2,true) and empty() and skipped()}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !support::calls(&package, &vm)
            .iter()
            .any(|name| name == "keep" || name == "forever")
    );
    for source in [
        "pure fn bad(comptime count:.Integer'builtin,p:*bool)->bool {
            inline for i if false in 0..count {p.* = false;} true
        } fn run(p:*bool)->bool {bad(2,p)}",
        "fn bad(comptime count:.Integer'builtin,flag:bool)->bool {
            inline for i if flag in 0..count {} true
        } fn run()->bool {bad(0,true)}",
        "fn take(value:bool) {} fn bad()->bool {
            inline for i if false in 0..2 {take(3);} true
        }",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn tuple_enumeration_checks_shape_without_unfolding_type_heads() {
    for source in [
        "fn bad(value:bool) {let view=value'tuple_enumerate;}",
        "fn bad(value:bool) {inline for (i,element) in value'tuple_enumerate {}}",
        "fn bad() {let view=bool'tuple_enumerate;}",
    ] {
        let failure = compile(source).unwrap_err();
        assert!(
            failure.contains("tuple_enumerate requires a tuple value"),
            "{failure}"
        );
    }
    let failure = compile(
        "typealias Type=.Type'builtin;pure comptime fn Identity<T>->Type {T}
         fn bad(value:Identity<(bool,bool)>) {let view=value'tuple_enumerate;}",
    )
    .unwrap_err();
    assert!(
        failure.contains("structural type view remains unresolved"),
        "{failure}"
    );
    let failure = compile(
        "fn source()->(bool,bool) {source()} fn run() {let view=source()'tuple_enumerate;}",
    )
    .unwrap_err();
    assert!(
        failure.contains("tuple_enumerate structural iteration is not yet implemented"),
        "{failure}"
    );
}

#[test]
fn tuple_enumeration_recognizes_the_bound_tuple_normal_form() {
    let failure = support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;
         pure comptime fn List<T>->Type {T}
         pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
         fn pending(value:Tuple<bool,.u8'builtin>) {let view=value'tuple_enumerate;}",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        failure.contains("tuple_enumerate structural iteration is not yet implemented"),
        "{failure}"
    );
}

#[test]
fn destructuring_inline_initializers_checks_patterns_without_invoking_selectors() {
    let package = compile(
        "fn yes(value:bool)->(bool,bool) {(value,false)}
        fn no(value:bool)->(bool,bool) {(false,value)}
        fn choose(comptime flag:bool,value:bool)->bool {
            let (a,b)=do {inline if flag {yes(value)} else {no(value)}};
            a or b
        }
        fn first()->bool {choose(true,true)}
        fn second()->bool {choose(false,true)}",
    )
    .unwrap();
    for (entry, callee) in [("first", "yes"), ("second", "no")] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), [entry, "choose", callee]);
    }
    for source in [
        "fn bad(comptime flag:bool,value:bool)->bool {
            let (a,b)=do {inline if flag {(value,false)} else {(false,value)}};
            a=false;a
        } fn run()->bool {bad(true,true)}",
        "fn bad(comptime flag:bool,value:bool)->bool {
            let (a,b,c)=do {inline if flag {(value,false)} else {(false,value)}};
            a
        } fn run()->bool {bad(true,true)}",
        "fn bad(comptime flag:bool,value:bool)->bool {
            let (true,b)=do {inline if flag {(value,false)} else {(false,value)}};
            b
        } fn run()->bool {bad(true,true)}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inline_destructuring_preserves_refutable_pattern_else_control_flow() {
    let package = compile(
        "pure fn choose(comptime flag:bool,value:bool)->bool {
            let (true,b)=do {inline if flag {(value,true)} else {(false,value)}}
                else {return false;};
            b
        }
        fn yes()->bool {choose(true,true)}
        fn no()->bool {not choose(false,true)}",
    )
    .unwrap();
    for entry in ["yes", "no"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn ordinary_matches_accept_inline_subjects_without_early_selection() {
    let package = compile(
        "fn left(value:bool)->(bool,bool) {(true,value)}
        fn right(value:bool)->(bool,bool) {(false,value)}
        pure fn choose(comptime flag:bool,value:bool)->bool {
            if do {inline if flag {left(value)} else {right(value)}} is do {
                (true,payload)=>payload,
                (false,payload)=>not payload,
            }
        }
        fn yes()->bool {choose(true,true)}
        fn no()->bool {choose(false,false)}",
    )
    .unwrap();
    for (entry, callee) in [("yes", "left"), ("no", "right")] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), [entry, "choose", callee]);
    }
    for source in [
        "fn bad(comptime flag:bool,value:bool)->bool {
            if do {inline if flag {value} else {false}} is do {true=>true,}
        } fn run()->bool {bad(true,true)}",
        "fn bad(comptime flag:bool,value:bool)->bool {
            if do {inline if flag {(value,false)} else {false}} is do {_=>true,}
        } fn run()->bool {bad(true,true)}",
        "pure fn bad(comptime flag:bool,value:bool,p:*bool)->bool {
            if do {inline if flag {value} else {false}} is do {
                true=>true,false=>{p.* = false;false},
            }
        } fn run(p:*bool)->bool {bad(true,true,p)}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inline_match_subject_composition_preserves_stage_boundaries() {
    let package = compile(
        "comptime fn invert(value:bool)->bool {not value}
        pure fn choose(comptime flag:bool,value:bool)->bool {
            inline if do {inline if flag {invert(false)} else {invert(true)}} is do {
                true=>value,false=>not value,
            }
        } fn run()->bool {choose(true,true) and choose(false,false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        compile(
            "fn bad(comptime flag:bool,value:bool)->bool {
            inline if do {inline if flag {value} else {false}} is do {true=>true,false=>false,}
        } fn run()->bool {bad(true,true)}"
        )
        .is_err()
    );
}

#[test]
fn inline_pointer_assignment_preserves_target_and_value_order() {
    let package = compile(
        "fn target(p:*bool)->*bool {p}
        fn yes()->bool {true} fn no()->bool {false}
        fn store(comptime flag:bool,p:*bool) {
            target(p).* = do {inline if flag {yes()} else {no()}};
        }
        fn first()->bool {let value=false;store(true,value.ref);value}
        fn second()->bool {let value=true;store(false,value.ref);not value}",
    )
    .unwrap();
    for (entry, callee) in [("first", "yes"), ("second", "no")] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm),
            [entry, "store", "target", callee]
        );
    }
    for source in [
        "pure fn bad(comptime flag:bool,p:*bool) {
            p.* = do {inline if flag {true} else {false}};
        } fn run(p:*bool) {bad(true,p);}",
        "fn bad(comptime flag:bool,p:*bool) {
            p.* = do {inline if flag {true} else {(false,false)}};
        } fn run(p:*bool) {bad(true,p);}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn inline_pointer_field_assignment_preserves_sibling_storage() {
    let package=compile(
        "struct Pair {first:bool,second:bool}
        fn store(comptime flag:bool,p:*Pair) {
            p.*.first = do {inline if flag {true} else {false}};
        }
        fn run()->bool {let pair=Pair {first:false,second:true};store(true,pair.ref);pair.first and pair.second}",
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inline_pointer_selection_can_be_read_without_early_invocation() {
    let package = compile(
        "fn left(p:*bool)->*bool {p} fn right(p:*bool)->*bool {p}
        fn read(comptime flag:bool,a:*bool,b:*bool)->bool {
            do {inline if flag {left(a)} else {right(b)}}.*
        }
        fn first()->bool {let a=true;let b=false;read(true,a.ref,b.ref)}
        fn second()->bool {let a=false;let b=true;read(false,a.ref,b.ref)}",
    )
    .unwrap();
    for (entry, callee) in [("first", "left"), ("second", "right")] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), [entry, "read", callee]);
    }
    let failure=compile("fn bad(comptime flag:bool)->bool {do {inline if flag {true} else {false}}.*} fn run()->bool {bad(true)}").unwrap_err();
    assert!(failure.starts_with("elaboration failed:"), "{failure}");
}

#[test]
fn inline_tag_name_views_preserve_selected_enum_identity() {
    let package = compile(
        "typealias str=.str'builtin;
        enum State {ready,closed}
        fn ready()->State {State.ready} fn closed()->State {State.closed}
        fn name(comptime flag:bool)->str {
            do {inline if flag {ready()} else {closed()}}'tag_name
        }
        fn first()->str {name(true)} fn second()->str {name(false)}",
    )
    .unwrap();
    for (entry, callee) in [("first", "ready"), ("second", "closed")] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::String(callee.into())
        );
        assert_eq!(support::calls(&package, &vm), [entry, "name", callee]);
    }
    assert!(compile("fn bad(comptime flag:bool) {let name=do {inline if flag {true} else {false}}'tag_name;} fn run() {bad(true);}").is_err());
}

#[test]
fn inline_compound_assignments_preserve_place_preparation_and_context() {
    let package = compile(
        "typealias u8=.u8'builtin;
        fn target(p:*u8)->*u8 {p}
        fn rhs()->result:u8 where ensures result==3 {3}
        fn update(comptime flag:bool,p:*u8) {
            target(p).* %= do {inline if flag {rhs()} else {2}};
        }
        fn run()->bool {
            var value:u8=1;
            value += do {inline if true {2} else {3}};
            update(true,value.ref);
            value==0
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        ["run", "update", "target", "rhs"]
    );
    for source in [
        "fn bad(comptime flag:bool)->.u8'builtin {let value:.u8'builtin=1;value += do {inline if flag {1} else {2}};value} fn run()->.u8'builtin {bad(true)}",
        "fn bad(comptime flag:bool)->.u8'builtin {var value:.u8'builtin=1;value += do {inline if flag {1} else {256}};value} fn run()->.u8'builtin {bad(true)}",
        "pure fn bad(comptime flag:bool,p:*.u8'builtin) {p.* += do {inline if flag {1} else {2}};} fn run(p:*.u8'builtin) {bad(true,p);}",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn returned_pointer_contracts_keep_direct_storage_identity() {
    let _package = compile(
        "typealias Byte=.u8'builtin;
        fn identity(p:*Byte)->result:*Byte where requires p.* < 255, ensures result.* < 255 {p}
        fn explicit(p:*Byte)->result:*Byte where requires p.* < 255, ensures result.* < 255 {return p;}",
    )
    .unwrap();
    for body in ["q", "p.* = 255;p", "var alias=p;alias=q;alias"] {
        let failure=compile(&format!(
            "typealias Byte=.u8'builtin;
             fn bad(p:*Byte,q:*Byte)->result:*Byte where requires p.* < 255, ensures result.* < 255 {{{body}}}"
        )).unwrap_err();
        assert!(failure.contains("postcondition is not proven"), "{failure}");
    }
}

#[test]
fn pointer_call_preconditions_import_local_storage_ranges() {
    for postcondition in ["", ", ensures result.* < 255"] {
        let source=format!(
            "typealias Byte=.u8'builtin;
            fn identity(p:*Byte)->result:*Byte where requires p.* < 255{postcondition} {{p}}
            fn run()->bool {{let byte:Byte=3;identity(byte.ref).* == 3 and identity(byte.ref).* == 3}}"
        );
        let package =
            compile(&source).unwrap_or_else(|failure| panic!("{postcondition}: {failure}"));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn returned_pointer_memory_contracts_preserve_safe_updates_and_real_writes() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn target(p:*Byte)->result:*Byte where requires p.* <= 252, ensures result.* <= 252 {p}
        fn run()->bool {let byte:Byte=3;
            target(byte.ref).* += do {inline if true {3} else {2}};
            byte==6
        }",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn change(p:*Byte)->result:*Byte where ensures result.* <= 255 {p.* = 255;p}
        fn consume(p:*Byte) where requires p.* < 255 {}
        fn run() {let byte:Byte=3;change(byte.ref);consume(byte.ref);}",
    )
    .unwrap_err();
    assert!(failure.contains("precondition is not proven"), "{failure}");
}

#[test]
fn external_pointer_return_identity_preserves_checked_ranges() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn identity(p:*Byte)->*Byte {p}
        fn update(comptime flag:bool,p:*Byte) where requires p.* <= 252 {
            identity(p).* += do {inline if flag {3} else {2}};
        }
        fn read(p:*Byte)->Byte where requires p.* <= 252 {identity(p).* + 3}
        fn run()->bool {let byte:Byte=3;let before=read(byte.ref);update(true,byte.ref);before==6 and byte==6}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn returned_pointer_ranges_do_not_survive_invalidating_arguments_or_writes() {
    for body in [
        "pass(p,do {p.* = 255;true}).* + 3",
        "var selected=q;pass(selected,do {selected=p;true}).* + 3",
        "other(p,q).* + 3",
        "dirty(p).* + 3",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;
            fn pass(p:*Byte,flag:bool)->*Byte {{p}}
            fn other(p:*Byte,q:*Byte)->*Byte {{q}}
            fn dirty(p:*Byte)->*Byte {{p.* = 255;p}}
            fn bad(p:*Byte,q:*Byte)->Byte where requires p.* <= 252 {{{body}}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{body}: {failure}"
        );
    }
}

#[test]
fn returned_pointer_field_paths_keep_their_own_ranges() {
    let package=compile(
        "typealias Byte=.u8'builtin;
        struct Pair {number:Byte,other:Byte}
        fn number(p:*Pair)->*Byte {p.*.number.ref}
        fn read(p:*Pair)->Byte where requires p.*.number <= 252 {number(p).* + 3}
        fn bound(p:*Pair)->Byte where requires p.*.number <= 252 {
            let field=number(p);field.* + 3
        }
        fn identity(p:*Pair)->*Pair {p}
        fn projected(p:*Pair)->Byte where requires p.*.number <= 252 {identity(p).*.number + 3}
        fn run()->bool {let pair=Pair {number:3,other:255};read(pair.ref)==6 and bound(pair.ref)==6 and projected(pair.ref)==6}",
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "other(p).* + 3",
        "number(q).* + 3",
        "identity(p).*.other + 3",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
             struct Pair {{number:Byte,other:Byte}}
             fn number(p:*Pair)->*Byte {{p.*.number.ref}}
             fn other(p:*Pair)->*Byte {{p.*.other.ref}}
             fn identity(p:*Pair)->*Pair {{p}}
             fn bad(p:*Pair,q:*Pair)->Byte where requires p.*.number <= 252 {{{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn forwarded_pointer_results_keep_definition_side_contract_evidence() {
    let package=compile(
        "typealias Byte=.u8'builtin;
        struct Pair {number:Byte,other:Byte}
        fn identity(p:*Byte)->*Byte {p}
        fn number(p:*Pair)->*Byte {p.*.number.ref}
        fn forward(p:*Byte)->result:*Byte where requires p.* <= 252, ensures result.* <= 252 {identity(p)}
        fn twice(p:*Byte)->result:*Byte where requires p.* <= 252, ensures result.* <= 252 {return forward(p);}
        fn field(p:*Pair)->result:*Byte where requires p.*.number <= 252, ensures result.* <= 252 {number(p)}
        fn choose(flag:bool,p:*Byte,q:*Byte)->result:*Byte
            where requires p.* <= 252, requires q.* <= 252, ensures result.* <= 252 {
            if flag {identity(p)} else {identity(q)}
        }
        fn run()->bool {let value:Byte=3;twice(value.ref).* == 3}",
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "identity(q)",
        "p.* = 255;identity(p)",
        "if true {identity(q)} else {identity(p)}",
    ] {
        let failure=compile(&format!(
            "typealias Byte=.u8'builtin;fn identity(p:*Byte)->*Byte {{p}}
            fn bad(p:*Byte,q:*Byte)->result:*Byte where requires p.* <= 252, ensures result.* <= 252 {{{body}}}"
        )).unwrap_err();
        assert!(failure.contains("postcondition is not proven"), "{failure}");
    }
}

#[test]
fn short_circuit_pointer_predicates_retain_observation_identities() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn identity(p:*Byte)->*Byte {p}
        fn choose(flag:bool,p:*Byte,q:*Byte)->result:*Byte
            where requires p.* <= 252 and q.* <= 252, ensures result.* <= 252 {
            if flag {identity(p)} else {identity(q)}
        }
        fn run()->bool {let a:Byte=3;let b:Byte=4;choose(false,a.ref,b.ref).* == 4}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn bad(p:*Byte,q:*Byte)->Byte where requires p.* < 255 or q.* < 255 {p.* + 1}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn pointer_observation_priming_preserves_short_circuit_effects() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn target(p:*Byte)->*Byte {p}
        fn run()->bool {let byte:Byte=3;
            not (false and target(byte.ref).* < 255)
        }
        fn bounded(p:*Byte,q:*Byte)->Byte
            where requires not (p.* > 252 or q.* > 252) {q.* + 3}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), ["run"]);
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn bad(p:*Byte)->Byte {
            if p.* <= 252 and do {p.* = 255;true} {p.* + 3} else {0}
        }",
    )
    .unwrap_err();
    assert!(failure.starts_with("elaboration failed:"), "{failure}");
}

#[test]
fn returned_pointer_alternatives_can_share_one_actual_address() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {if flag {p} else {q}}
        fn identity(p:*Byte)->*Byte {p}
        fn read(flag:bool,p:*Byte)->Byte where requires p.* <= 252 {
            choose(flag,p,identity(p)).* + 3
        }
        fn run()->bool {let value:Byte=3;read(true,value.ref)==6 and read(false,value.ref)==6}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for arguments in ["p,q", "q,p"] {
        let failure=compile(&format!(
            "typealias Byte=.u8'builtin;
             fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {{if flag {{p}} else {{q}}}}
             fn bad(flag:bool,p:*Byte,q:*Byte)->Byte where requires p.* <= 252 {{choose(flag,{arguments}).* + 3}}"
        )).unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn returned_pointer_alternatives_do_not_merge_distinct_fields() {
    let failure = compile(
        "typealias Byte=.u8'builtin;struct Pair {first:Byte,second:Byte}
        fn choose(flag:bool,p:*Pair)->*Byte {if flag {p.*.first.ref} else {p.*.second.ref}}
        fn bad(flag:bool,p:*Pair)->Byte where requires p.*.first <= 252 {choose(flag,p).* + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn returned_pointer_alternatives_join_all_observed_ranges() {
    for expression in [
        "choose(flag,p,q).*",
        "do {let selected=choose(flag,p,q);selected.*}",
        "identity(choose(flag,p,q)).*",
        "do {let selected=if flag {choose(flag,p,q)} else {choose(flag,p,q)};selected.*}",
        "choose(flag,identity(p),choose(flag,p,q)).*",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {{if flag {{p}} else {{q}}}}
            fn identity(p:*Byte)->*Byte {{p}}
            fn read(flag:bool,p:*Byte,q:*Byte)->Byte
                where requires p.* <= 250 and q.* <= 252 {{{expression} + 3}}
            fn run()->bool {{let a:Byte=10;let b:Byte=20;
                read(true,a.ref,b.ref)==13 and read(false,a.ref,b.ref)==23}}"
        ))
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn returned_pointer_alternatives_project_each_candidate() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Pair {first:Byte,second:Byte}
        fn choose(flag:bool,p:*Pair,q:*Pair)->*Pair {if flag {p} else {q}}
        fn field(flag:bool,p:*Pair)->*Byte {if flag {p.*.first.ref} else {p.*.second.ref}}
        fn read(flag:bool,p:*Pair,q:*Pair)->Byte
            where requires p.*.first <= 250 and q.*.first <= 252 {
            choose(flag,p,q).*.first + 3
        }
        fn read_field(flag:bool,p:*Pair)->Byte
            where requires p.*.first <= 250 and p.*.second <= 252 {field(flag,p).* + 3}
        fn run()->bool {let p=Pair {first:10,second:20};let q=Pair {first:30,second:40};
            read(true,p.ref,q.ref)==13 and read(false,p.ref,q.ref)==33
            and read_field(false,p.ref)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn returned_pointer_alternative_evidence_expires_after_writes() {
    for body in [
        "let selected=choose(flag,p,q);q.* = 255;selected.* + 3",
        "let selected=choose(flag,p,q);p.* = 255;selected.* + 3",
        "choose(flag,p,do {q.* = 255;q}).* + 3",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {{if flag {{p}} else {{q}}}}
            fn bad(flag:bool,p:*Byte,q:*Byte)->Byte
                where requires p.* <= 250 and q.* <= 252 {{{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn returned_pointer_alternatives_require_every_nested_source() {
    for expression in [
        "identity(choose(flag,p,q)).*",
        "do {let selected=choose(flag,p,q);selected.*}",
        "choose(flag,p,choose(flag,p,q)).*",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {{if flag {{p}} else {{q}}}}
            fn identity(p:*Byte)->*Byte {{p}}
            fn bad(flag:bool,p:*Byte,q:*Byte)->Byte where requires p.* <= 252 {{{expression} + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn aggregate_pointer_inputs_preserve_external_return_origins() {
    for expression in [
        "first(Links {p:p,q:q}).*",
        "do {let links=Links {p:p,q:q};first(links).*}",
        "choose(flag,Links {p:p,q:q}).*",
        "nested(Nested {links:Links {p:p,q:q}}).*",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            struct Links {{p:*Byte,q:*Byte}} struct Nested {{links:Links}}
            fn first(links:Links)->*Byte {{links.p}}
            fn choose(flag:bool,links:Links)->*Byte {{if flag {{links.p}} else {{links.q}}}}
            fn nested(value:Nested)->*Byte {{value.links.q}}
            fn read(flag:bool,p:*Byte,q:*Byte)->Byte
                where requires p.* <= 250 and q.* <= 252 {{{expression} + 3}}
            fn run()->bool {{let a:Byte=10;let b:Byte=10;
                read(true,a.ref,b.ref)==13 and read(false,a.ref,b.ref)==13}}"
        ))
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn aggregate_pointer_return_origins_require_complete_current_evidence() {
    for (premise, expression) in [
        ("p.* <= 252", "choose(flag,Links {p:p,q:q}).* + 3"),
        (
            "p.* <= 252 and q.* <= 252",
            "let links=Links {p:p,q:q};q.* = 255;choose(flag,links).* + 3",
        ),
        (
            "p.* <= 252 and q.* <= 252",
            "choose(do {q.* = 255;flag},Links {p:p,q:q}).* + 3",
        ),
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}
            fn choose(flag:bool,links:Links)->*Byte {{if flag {{links.p}} else {{links.q}}}}
            fn bad(flag:bool,p:*Byte,q:*Byte)->Byte where requires {premise} {{{expression}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn aggregate_pointer_inputs_preserve_local_return_places() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Links {p:*Byte,q:*Byte}
        fn choose(flag:bool,links:Links)->*Byte {if flag {links.p} else {links.q}}
        fn run()->bool {let a:Byte=10;let b:Byte=20;
            choose(true,Links {p:a.ref,q:b.ref}).* + 3 == 13
            and choose(false,Links {p:a.ref,q:b.ref}).* + 3 == 23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;struct Links {p:*Byte,q:*Byte}
        fn choose(flag:bool,links:Links)->*Byte {if flag {links.p} else {links.q}}
        fn bad(flag:bool)->Byte {let a:Byte=10;let b:Byte=255;
            choose(flag,Links {p:a.ref,q:b.ref}).* + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn aggregate_pointer_writes_preserve_unwritten_local_ranges() {
    for setup in [
        "write(Links {p:a.ref,q:b.ref});",
        "let links=Links {p:a.ref,q:b.ref};write(links);",
        "nested(Nested {links:Links {p:a.ref,q:b.ref}});",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}} struct Nested {{links:Links}}
            fn write(links:Links) {{links.p.* = 255;}}
            fn nested(value:Nested) {{value.links.p.* = 255;}}
            fn run()->bool {{let a:Byte=10;let b:Byte=20;{setup} b + 3 == 23 and a == 255}}"
        )).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn aggregate_pointer_writes_invalidate_every_possible_destination() {
    for body in [
        "write(Links {p:a.ref,q:b.ref});a + 3",
        "write(Links {p:b.ref,q:a.ref});b + 3",
        "choose_write(flag,Links {p:a.ref,q:b.ref});a + 3",
        "choose_write(flag,Links {p:a.ref,q:b.ref});b + 3",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}
            fn write(links:Links) {{links.p.* = 255;}}
            fn choose_write(flag:bool,links:Links) {{if flag {{links.p.* = 255;}} else {{links.q.* = 255;}}}}
            fn bad(flag:bool)->Byte {{let a:Byte=10;let b:Byte=20;{body}}}"
        )).unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn aggregate_pointer_write_paths_select_pointee_fields() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Pair {first:Byte,second:Byte}struct Link {target:*Pair}
        fn write(link:Link) {link.target.*.first = 255;}
        fn run()->bool {let pair=Pair {first:10,second:20};write(Link {target:pair.ref});
            pair.second + 3 == 23 and pair.first == 255}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_aggregate_pointer_writes_preserve_stable_unwritten_ranges() {
    for call in [
        "write(Links {p:a.ref,q:b.ref})",
        "write(saved)",
        "nested(outer)",
        "get(saved).* = 255",
        "write(saved);saved.q=a.ref",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}struct Outer {{links:Links}}
            fn write(links:Links) {{links.p.* = 255;}}
            fn get(links:Links)->*Byte {{links.p}}
            fn nested(value:Outer) {{value.links.p.* = 255;}}
            fn read(flag:bool)->Byte {{let a:Byte=10;let b:Byte=20;
                var saved=Links {{p:a.ref,q:b.ref}};let outer=Outer {{links:Links {{p:a.ref,q:b.ref}}}};
                var again=flag;while again {{{call};again=false;}} b + 3}}
            fn run()->bool {{read(true)==23 and read(false)==23}}"
        )).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn loop_aggregate_pointer_origins_expire_when_slots_change() {
    for body in [
        "write(links);links.p=b.ref;",
        "links.p=b.ref;write(links);",
        "write(links);links=Links {p:b.ref,q:a.ref};",
        "write(Links {p:selected,q:b.ref});selected=b.ref;",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}
            fn write(links:Links) {{links.p.* = 255;}}
            fn bad(flag:bool)->Byte {{let a:Byte=10;let b:Byte=20;
                var links=Links {{p:a.ref,q:b.ref}};var selected=a.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn loop_aggregate_branch_inputs_preserve_all_destinations() {
    for selected in [
        "if flag {left} else {right}",
        "flag match {true=>left,false=>right}",
        "if flag {Links {p:a.ref,q:spare.ref}} else {Links {p:b.ref,q:spare.ref}}",
    ] {
        let source = format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}
            fn write(links:Links) {{links.p.* = 255;}}
            fn read(flag:bool)->Byte {{let a:Byte=10;let b:Byte=20;let spare:Byte=30;
                let left=Links {{p:a.ref,q:spare.ref}};let right=Links {{p:b.ref,q:spare.ref}};
                var again=true;while again {{write({selected});again=false;}} RESULT}}
            fn run()->bool {{read(true)==33 and read(false)==33}}"
        );
        let package = compile(&source.replace("RESULT", "spare + 3")).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        for result in ["a + 3", "b + 3"] {
            let failure = compile(&source.replace("RESULT", result)).unwrap_err();
            assert!(
                failure.contains("machine arithmetic range is not proven"),
                "{failure}"
            );
        }
    }
}

#[test]
fn loop_aggregate_branch_inputs_reject_unknown_or_rebound_candidates() {
    for selected in [
        "if flag {left} else {right}",
        "flag match {true=>left,false=>right}",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}
            fn write(links:Links) {{links.p.* = 255;}}
            fn bad(flag:bool,external:*Byte)->Byte {{let a:Byte=10;let b:Byte=20;
                let left=Links {{p:a.ref,q:b.ref}};let right=Links {{p:external,q:b.ref}};
                while flag {{write({selected});}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}
            fn write(links:Links) {{links.p.* = 255;}}
            fn bad(flag:bool)->Byte {{let a:Byte=10;let b:Byte=20;
                let left=Links {{p:a.ref,q:b.ref}};var right=Links {{p:a.ref,q:b.ref}};
                while flag {{write({selected});right.p=b.ref;}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn nominal_input_pointer_returns_use_declared_field_contracts() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Links {p:*Byte,q:*Byte}struct Outer {links:Links}
        struct Generic where T {pointer:*T}
        fn choose(flag:bool,links:Links)->*Byte {if flag {links.p} else {links.q}}
        fn nested(value:Outer)->*Byte {value.links.q}
        fn generic(value:Generic<Byte>)->*Byte {value.pointer}
        fn read(flag:bool,links:Links)->Byte
            where requires links.p.* <= 250 and links.q.* <= 252 {choose(flag,links).* + 3}
        fn read_nested(value:Outer)->Byte
            where requires value.links.q.* <= 252 {nested(value).* + 3}
        fn read_generic(value:Generic<Byte>)->Byte
            where requires value.pointer.* <= 252 {generic(value).* + 3}
        fn run()->bool {let a:Byte=10;let b:Byte=20;
            read(true,Links {p:a.ref,q:b.ref})==13 and read(false,Links {p:a.ref,q:b.ref})==23
            and read_nested(Outer {links:Links {p:a.ref,q:b.ref}})==23
            and read_generic(Generic<Byte> {pointer:a.ref})==13}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_input_pointer_returns_do_not_invent_field_bounds() {
    for body in [
        "choose(flag,links).* + 3",
        "links.q.* = 255;choose(flag,links).* + 3",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Links {{p:*Byte,q:*Byte}}
            fn choose(flag:bool,links:Links)->*Byte {{if flag {{links.p}} else {{links.q}}}}
            fn bad(flag:bool,links:Links)->Byte where requires links.p.* <= 252 {{{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn aggregate_pointer_preconditions_import_current_caller_observations() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Pair {number:Byte,other:Byte}struct Link {target:*Pair}
        fn read(link:Link)->Byte where requires link.target.*.number <= 252 {link.target.*.number + 3}
        fn forward(link:Link)->Byte where requires link.target.*.number <= 250 {read(link)}
        fn from_pointer(p:*Pair)->Byte where requires p.*.number <= 250 {read(Link {target:p})}
        fn run()->bool {let pair=Pair {number:20,other:255};
            read(Link {target:pair.ref})==23 and forward(Link {target:pair.ref})==23
            and from_pointer(pair.ref)==23}"
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn aggregate_pointer_preconditions_reject_stale_or_unrelated_bounds() {
    for body in [
        "read(Link {target:b},false)",
        "read(Link {target:a},do {a.* = 255;false})",
    ] {
        let failure=compile(&format!(
            "typealias Byte=.u8'builtin;struct Link {{target:*Byte}}
            fn read(link:Link,flag:bool)->Byte where requires link.target.* <= 252 {{link.target.* + 3}}
            fn bad(a:*Byte,b:*Byte)->Byte where requires a.* <= 252 {{{body}}}"
        )).unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
    let failure=compile(
        "typealias Byte=.u8'builtin;struct Pair {number:Byte,other:Byte}struct Link {target:*Pair}
        fn read(link:Link)->Byte where requires link.target.*.number <= 252 {link.target.*.number + 3}
        fn bad(link:Link)->Byte where requires link.target.*.other <= 252 {read(link)}"
    ).unwrap_err();
    assert!(failure.contains("precondition is not proven"), "{failure}");
}

#[test]
fn mixed_pointer_origins_join_local_and_external_ranges() {
    for selected in [
        "choose(flag,local.ref,p)",
        "identity(choose(flag,local.ref,p))",
        "if flag {local.ref} else {p}",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {{if flag {{p}} else {{q}}}}
            fn identity(p:*Byte)->*Byte {{p}}
            fn read(flag:bool,p:*Byte)->Byte where requires p.* <= 252 {{
                let local:Byte=10;let selected={selected};selected.* + 3
            }}
            fn run()->bool {{let external:Byte=20;
                read(true,external.ref)==13 and read(false,external.ref)==23}}"
        ))
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn mixed_pointer_origin_contracts_require_both_current_ranges() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {target:*Byte}
        fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {if flag {p} else {q}}
        fn bounded(p:*Byte)->Byte where requires p.* <= 252 {p.* + 3}
        fn wrapped(link:Link)->Byte where requires link.target.* <= 252 {link.target.* + 3}
        fn read(flag:bool,p:*Byte)->Byte where requires p.* <= 252 {
            let local:Byte=10;wrapped(Link {target:choose(flag,local.ref,p)})
        }
        fn direct(flag:bool,p:*Byte)->Byte where requires p.* <= 252 {
            let local:Byte=10;bounded(choose(flag,local.ref,p))
        }
        fn run()->bool {let value:Byte=20;read(true,value.ref)==13 and read(false,value.ref)==23
            and direct(true,value.ref)==13 and direct(false,value.ref)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for (premise, body) in [
        ("", "let local:Byte=10;choose(flag,local.ref,p).* + 3"),
        (
            "where requires p.* <= 252",
            "let local:Byte=255;choose(flag,local.ref,p).* + 3",
        ),
        (
            "where requires p.* <= 252",
            "let local:Byte=10;let selected=choose(flag,local.ref,p);p.* = 255;selected.* + 3",
        ),
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {{if flag {{p}} else {{q}}}}
            fn bad(flag:bool,p:*Byte)->Byte {premise} {{{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn mixed_pointer_origins_preserve_each_field_projection() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Pair {number:Byte,other:Byte}
        fn choose(flag:bool,p:*Pair,q:*Pair)->*Pair {if flag {p} else {q}}
        fn read(flag:bool,p:*Pair)->Byte where requires p.*.number <= 252 {
            let local=Pair {number:10,other:255};choose(flag,local.ref,p).*.number + 3
        }
        fn run()->bool {let value=Pair {number:20,other:255};
            read(true,value.ref)==13 and read(false,value.ref)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn mixed_pointer_writes_do_not_preserve_a_known_local_subset() {
    for body in [
        "let selected=choose(flag,local.ref,p);selected.* = 255;local + 3",
        "let selected=choose(flag,local.ref,p);selected.* = 255;p.* + 3",
        "let selected=choose(flag,local.ref,p);write(selected);local + 3",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn choose(flag:bool,p:*Byte,q:*Byte)->*Byte {{if flag {{p}} else {{q}}}}
            fn write(p:*Byte) {{p.* = 255;}}
            fn bad(flag:bool,p:*Byte)->Byte where requires p.* <= 252 {{let local:Byte=10;{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn readonly_indirect_pointer_returns_load_current_local_slots() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Slot {pointer:*Byte}
        fn load(slot:**Byte)->*Byte {slot.*}
        fn twice(slot:***Byte)->*Byte {slot.*.*}
        fn field(slot:*Slot)->*Byte {slot.*.pointer}
        fn read(p:*Byte)->Byte where requires p.* <= 252 {
            let saved=p;load(saved.ref).* + 3
        }
        fn run()->bool {let a:Byte=10;let b:Byte=20;var pointer=a.ref;
            let first=load(pointer.ref).* + 3;
            pointer=b.ref;
            let second=load(pointer.ref).* + 3;
            let indirect=pointer.ref;let deep=twice(indirect.ref).* + 3;
            let slot=Slot {pointer:pointer};let projected=field(slot.ref).* + 3;
            first==13 and second==23 and deep==23 and projected==23 and read(a.ref)==13}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn readonly_indirect_returns_observe_later_argument_slot_writes() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn load(slot:**Byte,flag:bool)->*Byte {slot.*}
        fn run()->bool {let a:Byte=10;let b:Byte=20;var pointer=a.ref;
            load(pointer.ref,do {pointer=b.ref;false}).* + 3 == 23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn load(slot:**Byte,flag:bool)->*Byte {slot.*}
        fn bad()->Byte {let a:Byte=10;let b:Byte=255;var pointer=a.ref;
            load(pointer.ref,do {pointer=b.ref;false}).* + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn indirect_pointer_returns_keep_all_loaded_alternatives() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn select(flag:bool,p:**Byte,q:**Byte)->*Byte {if flag {p.*} else {q.*}}
        fn read(flag:bool,p:*Byte)->Byte where requires p.* <= 252 {
            let a:Byte=10;let first=a.ref;let second=p;select(flag,first.ref,second.ref).* + 3
        }
        fn run()->bool {let value:Byte=20;read(true,value.ref)==13 and read(false,value.ref)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn select(flag:bool,p:**Byte,q:**Byte)->*Byte {if flag {p.*} else {q.*}}
        fn bad(flag:bool,p:**Byte)->Byte {let a:Byte=10;let first=a.ref;
            select(flag,first.ref,p).* + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn indirect_pointer_returns_do_not_reuse_entry_slots_across_callee_writes() {
    for (body, first, replacement) in [
        ("slot.* = replacement;slot.*", 10, 255),
        ("let saved=slot.*;slot.* = replacement;saved", 255, 10),
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn change(slot:**Byte,replacement:*Byte)->*Byte {{{body}}}
            fn bad()->Byte {{let a:Byte={first};let b:Byte={replacement};var pointer=a.ref;
                change(pointer.ref,b.ref).* + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn memory_local_slots_preserve_external_read_summaries() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn select(flag:bool,p:**Byte,q:**Byte)->*Byte {if flag {p.*} else {q.*}}
        fn read(flag:bool,p:*Byte)->Byte {
            let a:Byte=10;let first=a.ref;let second=p;select(flag,first.ref,second.ref).*
        }",
    )
    .unwrap();
    let read = package
        .functions
        .iter()
        .find(|function| function.name == "read")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        read.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert!(
        summary.accesses.iter().all(|access| !access.write),
        "{summary}"
    );
    assert!(
        summary.accesses.iter().any(|access| access.read_value
            && access.target.parameter == 1
            && access.target.through.is_empty()),
        "{summary}"
    );
}

#[test]
fn memory_local_slot_results_capture_the_correct_evaluation_point() {
    let package=compile(
        "struct Slot {pointer:*bool}
        fn load(slot:**bool,flag:bool)->*bool {slot.*}
        fn field(slot:*Slot)->*bool {slot.*.pointer}
        fn after(p:*bool,q:*bool)->*bool {var slot=p;load(slot.ref,do {slot=q;false})}
        fn before(p:*bool,q:*bool)->*bool {var slot=p;let old=load(slot.ref,false);slot=q;old}
        fn projected(p:*bool,q:*bool)->*bool {var slot=Slot {pointer:p};slot.pointer=q;field(slot.ref)}
        fn escape()->*bool {let value=true;let slot=value.ref;load(slot.ref,false)}"
    ).unwrap();
    for (name, parameter) in [("after", 1), ("before", 0), ("projected", 1)] {
        let function = package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            &mut 100_000,
        )
        .unwrap();
        let targets = summary.result_targets.as_ref().unwrap();
        assert_eq!(targets.len(), 1, "{summary}");
        let target = targets.first().unwrap();
        assert_eq!(target.parameter, parameter, "{summary}");
        assert!(
            target.input_fields.is_empty() && target.through.is_empty() && target.fields.is_empty(),
            "{summary}"
        );
        assert!(
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                &mut 1
            )
            .is_err()
        );
    }
    let escape = package
        .functions
        .iter()
        .find(|function| function.name == "escape")
        .unwrap();
    assert!(
        elaboration::verification::memory::infer_in_package(
            &package,
            escape.declaration,
            &[],
            &mut 100_000
        )
        .is_err()
    );
}

#[test]
fn loop_indirect_local_slots_preserve_unwritten_ranges() {
    for write in [
        "slot.*.* = 255",
        "load(slot).* = 255",
        "deep(slot.ref).* = 255",
        "field(holder.ref).* = 255",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;struct Slot {{pointer:*Byte}}
            fn load(slot:**Byte)->*Byte {{slot.*}}
            fn field(slot:*Slot)->*Byte {{slot.*.pointer}}
            fn deep(slot:***Byte)->*Byte {{slot.*.*}}
            fn read(flag:bool)->Byte {{let a:Byte=10;let b:Byte=20;let spare=b.ref;
                let holder=Slot {{pointer:a.ref}};let pointer=a.ref;let slot=pointer.ref;var again=flag;
                while again {{{write};again=false;}} b + 3}}
            fn run()->bool {{read(true)==23 and read(false)==23}}"
        ))
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn loop_indirect_local_slots_reject_rebound_intermediate_storage() {
    for body in [
        "load(slot).* = 255;pointer=b.ref;",
        "pointer=b.ref;load(slot).* = 255;",
        "load(slot).* = 255;slot.* = b.ref;",
        "slot.*.* = 255;pointer=b.ref;",
        "field(holder.ref).* = 255;holder=Slot {pointer:b.ref};",
        "field(holder.ref).* = 255;holder.pointer=b.ref;",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Slot {{pointer:*Byte}}
            fn load(slot:**Byte)->*Byte {{slot.*}}
            fn field(slot:*Slot)->*Byte {{slot.*.pointer}}
            fn bad(flag:bool)->Byte {{let a:Byte=10;let b:Byte=20;
                var holder=Slot {{pointer:a.ref}};var pointer=a.ref;let slot=pointer.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn external_pointer_slots_support_checked_indirect_ranges() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn read(slot:**Byte)->Byte {assumes slot.*.* <= 252;slot.*.* + 3}
        fn deep(slot:***Byte)->Byte {assumes slot.*.*.* <= 252;slot.*.*.* + 3}
        fn both(a:**Byte,b:**Byte)->Byte {assumes a.*.* <= 250 and b.*.* <= 252;b.*.* + 3}
        fn contract(slot:**Byte)->Byte where requires slot.*.* <= 252 {slot.*.* + 3}
        fn run()->bool {let a:Byte=10;let b:Byte=20;let p=a.ref;let q=b.ref;let slot=p.ref;
            read(p.ref)==13 and deep(slot.ref)==13 and both(p.ref,q.ref)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn external_pointer_slots_do_not_reuse_bounds_after_writes() {
    for body in [
        "assumes slot.*.* <= 252;slot.*.* = 255;slot.*.* + 3",
        "assumes slot.*.* <= 252;slot.* = other;slot.*.* + 3",
        "assumes slot.*.* <= 252 or other.* <= 252;slot.*.* + 3",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;fn bad(slot:**Byte,other:*Byte)->Byte {{{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn external_pointer_slot_observations_keep_load_identity() {
    let package=compile(
        "typealias Byte=.u8'builtin;
        fn same(slot:**Byte)->result:bool where ensures result {
            let loaded=slot.*;loaded.* == slot.*.*
        }
        fn checked(slot:**Byte)->bool {assumes slot.*.* <= 252;true}
        fn run()->bool {let value:Byte=20;let pointer=value.ref;same(pointer.ref) and checked(pointer.ref)}
        fn fail()->bool {let value:Byte=255;let pointer=value.ref;checked(pointer.ref)}"
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(luna::execution::interpret(&package, "fail", 100_000).is_err());
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn bad(slot:**Byte,other:*Byte)->Byte {
            let old=slot.*;slot.* = other;assumes slot.*.* <= 252;old.* + 3
        }",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn external_slot_priming_does_not_invoke_skipped_pointer_factories() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn factory(slot:**Byte)->**Byte {slot}
        fn run()->result:bool where ensures result {let value:Byte=255;let pointer=value.ref;
            not (false and factory(pointer.ref).*.* < 255)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), ["run"]);
}

#[test]
fn readonly_external_slot_returns_preserve_observed_ranges() {
    let package=compile(
        "typealias Byte=.u8'builtin;struct Slot {pointer:*Byte}
        fn load(slot:**Byte)->*Byte {slot.*}
        fn deep(slot:***Byte)->*Byte {slot.*.*}
        fn field(slot:*Slot)->*Byte {slot.*.pointer}
        fn read(slot:**Byte)->Byte {assumes slot.*.* <= 252;load(slot).* + 3}
        fn read_deep(slot:***Byte)->Byte {assumes slot.*.*.* <= 252;deep(slot).* + 3}
        fn read_field(slot:*Slot)->Byte {assumes slot.*.pointer.* <= 252;field(slot).* + 3}
        fn contracted(slot:**Byte)->Byte where requires slot.*.* <= 252 {load(slot).* + 3}
        fn run()->bool {let value:Byte=20;let pointer=value.ref;let slot=pointer.ref;
            let record=Slot {pointer:pointer};read(slot)==23 and read_deep(slot.ref)==23 and read_field(record.ref)==23}"
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn readonly_external_slot_returns_require_all_current_observations() {
    for body in [
        "assumes first.*.* <= 252;choose(flag,first,second).* + 3",
        "assumes first.*.* <= 252;first.* = other;load(first).* + 3",
        "assumes first.*.* <= 252;let old=load(first);first.* = other;assumes first.*.* <= 252;old.* + 3",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn load(slot:**Byte)->*Byte {{slot.*}}
            fn choose(flag:bool,p:**Byte,q:**Byte)->*Byte {{if flag {{p.*}} else {{q.*}}}}
            fn bad(flag:bool,first:**Byte,second:**Byte,other:*Byte)->Byte {{{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn external_loaded_results_compose_candidates_forwarding_and_fields() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Pair {number:Byte,other:Byte}
        fn load(slot:**Byte)->*Byte {slot.*}
        fn forward(slot:**Byte)->*Byte {load(slot)}
        fn choose(flag:bool,p:**Byte,q:**Byte)->*Byte {if flag {p.*} else {q.*}}
        fn number(slot:**Pair)->*Byte {slot.*.*.number.ref}
        fn read(flag:bool,p:**Byte,q:**Byte)->Byte {
            assumes p.*.* <= 250 and q.*.* <= 252;choose(flag,p,q).* + 3
        }
        fn forwarded(slot:**Byte)->Byte {assumes slot.*.* <= 252;forward(slot).* + 3}
        fn projected(slot:**Pair)->Byte {assumes slot.*.*.number <= 252;number(slot).* + 3}
        fn run()->bool {let a:Byte=10;let b:Byte=20;let p=a.ref;let q=b.ref;
            let pair=Pair {number:30,other:255};let ptr=pair.ref;
            read(true,p.ref,q.ref)==13 and read(false,p.ref,q.ref)==23
            and forwarded(p.ref)==13 and projected(ptr.ref)==33}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_preconditions_import_local_and_external_slot_observations() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Pair {number:Byte,other:Byte}
        fn read(slot:**Byte)->Byte where requires slot.*.* <= 252 {slot.*.* + 3}
        fn deep(slot:***Byte)->Byte where requires slot.*.*.* <= 252 {slot.*.*.* + 3}
        fn field(slot:**Pair)->Byte where requires slot.*.*.number <= 252 {slot.*.*.number + 3}
        fn forward(slot:**Byte)->Byte where requires slot.*.* <= 250 {read(slot)}
        fn checked(slot:**Byte)->Byte {assumes slot.*.* <= 252;read(slot)}
        fn run()->bool {let value:Byte=20;let p=value.ref;let slot=p.ref;
            let pair=Pair {number:30,other:255};let pair_ptr=pair.ref;
            read(slot)==23 and deep(slot.ref)==23 and field(pair_ptr.ref)==33
            and forward(slot)==23 and checked(slot)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_preconditions_reject_stale_wrong_or_incomplete_targets() {
    for body in [
        "read(first,false)",
        "assumes first.*.* <= 252;read(second,false)",
        "assumes first.*.* <= 252;read(first,do {first.* = other;false})",
        "assumes first.*.* <= 252;read(first,do {first.*.* = 255;false})",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn read(slot:**Byte,flag:bool)->Byte where requires slot.*.* <= 252 {{slot.*.* + 3}}
            fn bad(first:**Byte,second:**Byte,other:*Byte)->Byte {{{body}}}"
        ))
        .unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
}

#[test]
fn indirect_preconditions_preserve_relations_between_loaded_values() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn delta(first:**Byte,second:**Byte)->Byte
            where requires first.*.* <= second.*.* {second.*.* - first.*.*}
        fn forward(first:**Byte,second:**Byte)->Byte
            where requires first.*.* <= second.*.* {delta(first,second)}
        fn checked(first:**Byte,second:**Byte)->Byte {
            assumes first.*.* <= second.*.*;delta(first,second)
        }
        fn run()->bool {let a:Byte=7;let b:Byte=20;let p=a.ref;let q=b.ref;
            forward(p.ref,q.ref)==13 and checked(p.ref,q.ref)==13}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn delta(first:**Byte,second:**Byte)->Byte
            where requires first.*.* <= second.*.* {second.*.* - first.*.*}
        fn bad(first:**Byte,second:**Byte)->Byte
            where requires first.*.* <= second.*.* {delta(second,first)}",
    )
    .unwrap_err();
    assert!(failure.contains("precondition is not proven"), "{failure}");
}

#[test]
fn indirect_preconditions_do_not_require_a_readonly_body_summary() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        struct Input {slot:**Byte}
        fn store(slot:**Byte,next:*Byte) where requires slot.*.* <= 252 {slot.* = next;}
        fn store_owned(input:Input,next:*Byte) where requires input.slot.*.* <= 252 {
            input.slot.* = next;
        }
        fn forward(slot:**Byte,next:*Byte) where requires slot.*.* <= 250 {store(slot,next);}
        fn forward_owned(input:Input,next:*Byte) where requires input.slot.*.* <= 250 {
            store_owned(input,next);
        }
        fn run()->bool {let a:Byte=10;let b:Byte=255;var p=a.ref;
            forward(p.ref,b.ref);let first=p.* == 255;let c:Byte=10;var q=c.ref;
            forward_owned(Input {slot:q.ref},b.ref);first and q.* == 255}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "store(slot,next)",
        "assumes slot.*.* <= 252;store(slot,do {slot.*.* = 255;next})",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn store(slot:**Byte,next:*Byte) where requires slot.*.* <= 252 {{slot.* = next;}}
            fn bad(slot:**Byte,next:*Byte) {{{body}}}"
        ))
        .unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
}

#[test]
fn indirect_preconditions_join_all_selected_slot_targets() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn read(slot:**Byte)->Byte where requires slot.*.* <= 252 {slot.*.* + 3}
        fn external(flag:bool,first:**Byte,second:**Byte)->Byte
            where requires first.*.* <= 250 and second.*.* <= 252 {
            let selected=if flag {first} else {second};read(selected)
        }
        fn mixed(flag:bool,slot:**Byte)->Byte where requires slot.*.* <= 252 {
            let local:Byte=20;let ptr=local.ref;
            let selected=if flag {ptr.ref} else {slot};read(selected)
        }
        fn run()->bool {let a:Byte=10;let b:Byte=252;let p=a.ref;let q=b.ref;
            external(true,p.ref,q.ref)==13 and external(false,p.ref,q.ref)==255
            and mixed(true,q.ref)==23 and mixed(false,q.ref)==255}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_preconditions_cannot_drop_an_unproven_selected_slot() {
    for body in [
        "let selected=if flag {first} else {second};read(selected)",
        "let selected=if flag {second} else {first};read(selected)",
        "let local:Byte=255;let p=local.ref;let selected=if flag {first} else {p.ref};read(selected)",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn read(slot:**Byte)->Byte where requires slot.*.* <= 252 {{slot.*.* + 3}}
            fn bad(flag:bool,first:**Byte,second:**Byte)->Byte
                where requires first.*.* <= 252 {{{body}}}"
        ))
        .unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
}

#[test]
fn indirect_precondition_assumes_fail_before_entering_the_callee() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn entered() {}
        fn read(slot:**Byte)->Byte where requires slot.*.* <= 252 {entered();slot.*.* + 3}
        fn checked(slot:**Byte)->Byte {assumes slot.*.* <= 252;read(slot)}
        fn good()->Byte {let value:Byte=252;let pointer=value.ref;checked(pointer.ref)}
        fn bad()->Byte {let value:Byte=253;let pointer=value.ref;checked(pointer.ref)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    vm.invoke(support::id(&package, "good"), vec![]).unwrap();
    assert_eq!(
        support::calls(&package, &vm),
        ["good", "checked", "read", "entered"]
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert!(vm.invoke(support::id(&package, "bad"), vec![]).is_err());
    assert_eq!(support::calls(&package, &vm), ["bad", "checked"]);
}

#[test]
fn indirect_default_preconditions_preserve_prepared_pointer_origins() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Input {slot:**Byte}
        fn read(source:**Byte,.slot:**Byte=source)->Byte
            where requires slot.*.* <= 252 {slot.*.* + 3}
        fn chain(source:**Byte,.first:**Byte=source,.slot:**Byte=first)->Byte
            where requires slot.*.* <= 252 {slot.*.* + 3}
        fn owned(source:**Byte,.input:Input=do {Input {slot:source}})->Byte
            where requires input.slot.*.* <= 252 {input.slot.*.* + 3}
        fn forward(source:**Byte)->Byte where requires source.*.* <= 252 {read(source)}
        fn run()->bool {let a:Byte=20;let p=a.ref;
            read(p.ref)==23 and chain(p.ref)==23 and forward(p.ref)==23
            and owned(p.ref)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_default_preconditions_reject_wrong_or_mutated_inputs() {
    for body in [
        "read(second,false)",
        "read(first,do {first.*.* = 255;false})",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn read(source:**Byte,flag:bool,.slot:**Byte=source)->Byte
                where requires slot.*.* <= 252 {{slot.*.* + 3}}
            fn bad(first:**Byte,second:**Byte)->Byte
                where requires first.*.* <= 252 {{{body}}}"
        ))
        .unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
}

#[test]
fn indirect_default_preconditions_join_conditional_prepared_inputs() {
    let declaration = "typealias Byte=.u8'builtin;
        fn choose(flag:bool,first:**Byte,second:**Byte,
            .slot:**Byte=if flag {first} else {second})->Byte
            where requires slot.*.* <= 252 {slot.*.* + 3}";
    let package = compile(&format!(
        "{declaration}
        fn checked(flag:bool,first:**Byte,second:**Byte)->Byte
            where requires first.*.* <= 252 and second.*.* <= 252 {{choose(flag,first,second)}}
        fn run()->bool {{let a:Byte=10;let b:Byte=252;let p=a.ref;let q=b.ref;
            checked(true,p.ref,q.ref)==13 and checked(false,p.ref,q.ref)==255}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(&format!(
        "{declaration}
        fn bad(flag:bool,first:**Byte,second:**Byte)->Byte
            where requires first.*.* <= 252 {{choose(flag,first,second)}}"
    ))
    .unwrap_err();
    assert!(failure.contains("precondition is not proven"), "{failure}");
}

#[test]
fn indirect_postconditions_transfer_readonly_loaded_relations() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Pair {number:Byte,other:Byte}
        fn read(slot:**Byte)->result:Byte where ensures result == slot.*.* {slot.*.*}
        fn deep(slot:***Byte)->result:Byte where ensures result == slot.*.*.* {slot.*.*.*}
        fn field(slot:**Pair)->result:Byte where ensures result == slot.*.*.number {slot.*.*.number}
        fn use_read(slot:**Byte)->Byte {assumes slot.*.* <= 252;read(slot) + 3}
        fn use_deep(slot:***Byte)->Byte {assumes slot.*.*.* <= 252;deep(slot) + 3}
        fn use_field(slot:**Pair)->Byte {assumes slot.*.*.number <= 252;field(slot) + 3}
        fn run()->bool {let n:Byte=20;let p=n.ref;let q=p.ref;
            let pair=Pair {number:30,other:255};let r=pair.ref;
            use_read(q)==23 and use_deep(q.ref)==23 and use_field(r.ref)==33}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_postconditions_preserve_forwarded_relational_results() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn read(slot:**Byte)->result:Byte where ensures result == slot.*.* {slot.*.*}
        fn forward(slot:**Byte)->result:Byte where ensures result == slot.*.* {read(slot)}
        fn difference(first:**Byte,second:**Byte)->Byte {
            assumes first.*.* <= second.*.*;forward(second) - forward(first)
        }
        fn run()->bool {let a:Byte=7;let b:Byte=20;let p=a.ref;let q=b.ref;
            difference(p.ref,q.ref)==13}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_postconditions_do_not_reuse_wrong_or_invalidated_observations() {
    for body in [
        "assumes first.*.* <= 252;read(second,false) + 3",
        "assumes first.*.* <= 252;read(first,do {first.*.* = 255;false}) + 3",
        "assumes first.*.* <= 252;write(first) + 3",
    ] {
        let failure=compile(&format!(
            "typealias Byte=.u8'builtin;
            fn read(slot:**Byte,flag:bool)->result:Byte where ensures result == slot.*.* {{slot.*.*}}
            fn write(slot:**Byte)->result:Byte where ensures result == slot.*.* {{slot.*.* = 255;slot.*.*}}
            fn bad(first:**Byte,second:**Byte)->Byte {{{body}}}"
        )).unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn inline_range_composite_irrefutable_patterns_materialize_all_bindings() {
    let package = compile(
        "fn first() {} fn later() {}
        fn run()->bool {
            inline for _ as index in 0..3 {
                inline if index==0 {first();} else {later();}
            }
            inline for (index or index) in 0..2 {
                inline if index==0 {first();} else {later();}
            }
            true
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        ["run", "first", "later", "later", "first", "later"]
    );
}

#[test]
fn inline_range_composite_patterns_compose_with_guards_and_continue() {
    let package = compile(
        "fn visit() {}
        fn run()->bool {
            inline for (index or index) if index!=1 in 0..4 {
                inline if index==2 {continue;}
                visit();
            }
            inline for _ as index in 0..0 {
                inline if index==0 {visit();}
            }
            true
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), ["run", "visit", "visit"]);
    for source in [
        "fn bad() {inline for (left or right) in 0..2 {}}",
        "fn bad() {inline for _ as index in 0..0 {let value:bool=index;}}",
        "pure fn bad(comptime count:.Integer'builtin,p:*bool)->bool {inline for (index or index) if false in 0..count {p.* = false;} true} fn run(p:*bool)->bool {bad(2,p)}",
    ] {
        assert!(
            compile(source).is_err(),
            "accepted invalid source: {source}"
        );
    }
}

#[test]
fn inline_range_sequential_patterns_deliver_computed_bindings_to_guards() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        pure comptime fn increment(value:Integer)->Integer {value + 1}
        fn visit() {}
        fn run()->bool {
            inline for (index and increment(index) is next) if next!=2 in 0..4 {
                inline if next==index + 1 {visit();}
            }
            true
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        ["run", "visit", "visit", "visit"]
    );
}

#[test]
fn inline_range_sequential_patterns_skip_empty_invocation_but_check_stages() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
        comptime fn forever()->Integer {forever()}
        fn run()->bool {
            inline for (index and forever() is next) in 0..0 {
                inline if next==index {}
            }
            inline if false {
                inline for (index and forever() is next) in 0..3 {}
            }
            true
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), ["run"]);
    for source in [
        "fn bad(value:bool) {inline for (index and value is selected) in 0..2 {inline if selected {}}}",
        "fn bad(comptime count:.Integer'builtin,value:bool) {inline for (index and value is selected) in 0..count {}} fn run() {bad(0,true);}",
        "fn bad() {inline for (index and true is selected) in 0..0 {let wrong:.Integer'builtin=selected;}}",
    ] {
        assert!(
            compile(source).is_err(),
            "accepted invalid source: {source}"
        );
    }
}

#[test]
fn loaded_pointer_defaults_import_current_caller_storage() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Input {slot:**Byte}
        fn read(slot:**Byte,.p:*Byte=slot.*)->Byte where requires p.* <= 252 {p.* + 3}
        fn deep(slot:***Byte,.middle:**Byte=slot.*,.p:*Byte=middle.*)->Byte
            where requires p.* <= 252 {p.* + 3}
        fn field(input:Input,.p:*Byte=input.slot.*)->Byte where requires p.* <= 252 {p.* + 3}
        fn forward(slot:**Byte)->Byte where requires slot.*.* <= 252 {read(slot)}
        fn run()->bool {let a:Byte=20;let p=a.ref;let q=p.ref;
            read(q)==23 and deep(q.ref)==23 and field(Input {slot:q})==23 and forward(q)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loaded_pointer_defaults_follow_argument_order_and_reject_stale_ranges() {
    let prefix = "typealias Byte=.u8'builtin;
        fn read(slot:**Byte,flag:bool,.p:*Byte=slot.*)->Byte
            where requires p.* <= 252 {p.* + 3}";
    let package = compile(&format!(
        "{prefix}
        fn run()->bool {{let a:Byte=255;let b:Byte=20;var p=a.ref;
            read(p.ref,do {{p=b.ref;false}})==23}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "read(slot,false)",
        "assumes slot.*.* <= 252;read(slot,do {slot.*.* = 255;false})",
        "assumes slot.*.* <= 252;read(slot,do {slot.* = other;false})",
    ] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(slot:**Byte,other:*Byte)->Byte {{{body}}}"
        ))
        .unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
}

#[test]
fn readonly_default_calls_transfer_complete_pointer_return_paths() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Input {slot:**Byte}
        fn load(slot:**Byte)->*Byte {slot.*}
        fn forward(slot:**Byte)->*Byte {load(slot)}
        fn field(input:*Input)->*Byte {input.*.slot.*}
        fn choose(flag:bool,a:**Byte,b:**Byte)->*Byte {if flag {a.*} else {b.*}}
        fn read(slot:**Byte,.p:*Byte=forward(slot))->Byte where requires p.* <= 252 {p.* + 3}
        fn scoped(slot:**Byte,.p:*Byte=do {let local=slot;load(local)})->Byte
            where requires p.* <= 252 {p.* + 3}
        fn owned(input:*Input,.p:*Byte=field(input))->Byte where requires p.* <= 252 {p.* + 3}
        fn selected(flag:bool,a:**Byte,b:**Byte,.p:*Byte=choose(flag,a,b))->Byte
            where requires p.* <= 252 {p.* + 3}
        fn checked(flag:bool,a:**Byte,b:**Byte)->Byte
            where requires a.*.* <= 252 and b.*.* <= 252 {selected(flag,a,b)}
        fn run()->bool {let a:Byte=10;let b:Byte=252;let p=a.ref;let q=b.ref;let input=Input {slot:p.ref};
            read(p.ref)==13 and scoped(p.ref)==13 and owned(input.ref)==13
            and checked(true,p.ref,q.ref)==13 and checked(false,p.ref,q.ref)==255}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn readonly_default_calls_require_all_current_candidate_bounds() {
    for body in [
        "read(flag,first,second,false)",
        "assumes second.*.* <= 252;read(flag,first,second,do {second.*.* = 255;false})",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn choose(flag:bool,a:**Byte,b:**Byte)->*Byte {{if flag {{a.*}} else {{b.*}}}}
            fn read(flag:bool,a:**Byte,b:**Byte,ignored:bool,.p:*Byte=choose(flag,a,b))->Byte
                where requires p.* <= 252 {{p.* + 3}}
            fn bad(flag:bool,first:**Byte,second:**Byte)->Byte
                where requires first.*.* <= 252 {{{body}}}"
        ))
        .unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
}

#[test]
fn aggregate_default_calls_preserve_nested_pointer_fields() {
    let package = support::compile_with_source_bindings(
        "trait Copy {} trait Drop {fn drop(*self);} typealias Byte=.u8'builtin;
        struct Link {pointer:*Byte} struct Outer {link:Link} impl Copy for Link {} impl Copy for Outer {}
        fn make(slot:**Byte)->Link {Link {pointer:slot.*}}
        fn nested(slot:**Byte)->Outer {Outer {link:make(slot)}}
        fn read(slot:**Byte,.link:Link=make(slot))->Byte
            where requires link.pointer.* <= 252 {link.pointer.* + 3}
        fn deep(slot:**Byte,.outer:Outer=nested(slot))->Byte
            where requires outer.link.pointer.* <= 252 {outer.link.pointer.* + 3}
        fn chain(slot:**Byte,.link:Link=make(slot),.p:*Byte=link.pointer)->Byte
            where requires p.* <= 252 {p.* + 3}
        fn forward(slot:**Byte)->Byte where requires slot.*.* <= 252 {deep(slot)}
        fn run()->bool {let a:Byte=20;let p=a.ref;
            read(p.ref)==23 and deep(p.ref)==23 and chain(p.ref)==23 and forward(p.ref)==23}",
        &Default::default(), luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"), drop: Some("input::Drop"), ..Default::default()
        }
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn aggregate_default_calls_keep_fields_and_candidates_distinct() {
    for default in [
        "Pair {left:a.*,right:b.*}",
        "if flag {Pair {left:a.*,right:a.*}} else {Pair {left:a.*,right:b.*}}",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Pair {{left:*Byte,right:*Byte}}
            fn make(flag:bool,a:**Byte,b:**Byte)->Pair {{{default}}}
            fn read(flag:bool,a:**Byte,b:**Byte,.pair:Pair=make(flag,a,b))->Byte
                where requires pair.right.* <= 252 {{pair.right.* + 3}}
            fn bad(flag:bool,a:**Byte,b:**Byte)->Byte
                where requires a.*.* <= 252 {{read(flag,a,b)}}"
        ))
        .unwrap_err();
        assert!(failure.contains("precondition is not proven"), "{failure}");
    }
}

#[test]
fn readonly_aggregate_results_preserve_loaded_pointer_fields() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        struct Link {pointer:*Byte} struct Outer {link:Link}
        fn make(slot:**Byte)->Link {Link {pointer:slot.*}}
        fn nested(slot:**Byte)->Outer {Outer {link:make(slot)}}
        fn forwarded(slot:**Byte)->Outer {nested(slot)}
        fn read(slot:**Byte)->Byte {
            assumes slot.*.* <= 252;let outer=forwarded(slot);outer.link.pointer.* + 3
        }
        fn run()->bool {let a:Byte=20;let p=a.ref;
            let outer=forwarded(p.ref);outer.link.pointer.* + 3 == 23 and read(p.ref)==23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn readonly_aggregate_results_require_each_current_field_candidate() {
    let prefix = "typealias Byte=.u8'builtin;struct Pair {left:*Byte,right:*Byte}
        fn make(flag:bool,a:**Byte,b:**Byte,ignored:bool)->Pair {
            if flag {Pair {left:a.*,right:a.*}} else {Pair {left:a.*,right:b.*}}
        }";
    let package = compile(&format!(
        "{prefix}
        fn checked(flag:bool,a:**Byte,b:**Byte)->Byte {{
            assumes a.*.* <= 250 and b.*.* <= 252;
            let pair=make(flag,a,b,false);pair.right.* + 3
        }}
        fn run()->bool {{let a:Byte=10;let b:Byte=252;let p=a.ref;let q=b.ref;
            checked(true,p.ref,q.ref)==13 and checked(false,p.ref,q.ref)==255}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "let pair=make(flag,a,b,false);pair.right.* + 3",
        "assumes b.*.* <= 252;let pair=make(flag,a,b,do {b.*.* = 255;false});pair.right.* + 3",
        "assumes b.*.* <= 252;let pair=make(flag,a,b,false);pair.right.* = 255;pair.right.* + 3",
    ] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(flag:bool,a:**Byte,b:**Byte)->Byte where requires a.*.* <= 252 {{{body}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn readonly_tuple_results_keep_normal_form_and_pointer_origins() {
    let package = support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        fn pair(a:**Byte,b:**Byte)->(*Byte,*Byte) {(a.*,b.*)}
        fn forward(a:**Byte,b:**Byte)->(*Byte,*Byte) {pair(a,b)}
        fn checked(a:**Byte,b:**Byte)->Byte {
            assumes a.*.* <= 250 and b.*.* <= 252;
            let (first,second)=forward(a,b);second.* + 3
        }
        fn run()->bool {let a:Byte=10;let b:Byte=20;let p=a.ref;let q=b.ref;
            let (first,second)=pair(p.ref,q.ref);
            first.* + 3 == 13 and second.* + 3 == 23 and checked(p.ref,q.ref)==23}",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let pair = package.functions.iter().find(|f| f.name == "pair").unwrap();
    assert!(
        matches!(pair.body.ty, nir::Term::NormalForm { head, .. } if head == support::id(&package, "Tuple"))
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn readonly_tuple_results_do_not_swap_field_bounds() {
    for body in [
        "let (first,second)=pair(a,b);second.* + 3",
        "let (second,first)=pair(a,b);first.* + 3",
    ] {
        let failure = support::compile_with_source_bindings(
            &format!(
                "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
                pure comptime fn List<T>->Type {{T}}
                pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
                fn pair(a:**Byte,b:**Byte)->(*Byte,*Byte) {{(a.*,b.*)}}
                fn bad(a:**Byte,b:**Byte)->Byte where requires a.*.* <= 252 {{{body}}}"
            ),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                tuple: Some("input::Tuple"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn returned_local_pointer_fields_preserve_disjoint_storage_on_writes() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn make(p:*Byte)->Link {Link {pointer:p}}
        fn write(link:Link) {link.pointer.* = 255;}
        fn direct()->bool {var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            let link=make(a.ref);link.pointer.* = 255;
            b + 3 == 23 and a==255 and exposed.* == 20}
        fn called()->bool {var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            let link=make(a.ref);write(link);
            b + 3 == 23 and a==255 and exposed.* == 20}
        fn run()->bool {direct() and called()}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn returned_pointer_field_writes_merge_all_local_candidates() {
    let prefix="typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn choose(flag:bool,a:*Byte,b:*Byte)->Link {if flag {Link {pointer:a}} else {Link {pointer:b}}}";
    let package = compile(&format!(
        "{prefix}
        fn checked(flag:bool)->bool {{var a:Byte=10;var b:Byte=20;let c:Byte=30;let exposed=c.ref;
            let link=choose(flag,a.ref,b.ref);link.pointer.* = 255;
            c + 3 == 33 and exposed.* == 30}}
        fn run()->bool {{checked(true) and checked(false)}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for read in ["a + 3", "b + 3"] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;
                let link=choose(flag,a.ref,b.ref);link.pointer.* = 255;{read}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn default_aggregate_writes_preserve_disjoint_caller_ranges() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn make(p:*Byte)->Link {Link {pointer:p}}
        fn write(p:*Byte,.link:Link=make(p)) {link.pointer.* = 255;}
        fn chained(p:*Byte,.link:Link=make(p),.target:*Byte=link.pointer) {target.* = 255;}
        fn run()->bool {var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            write(a.ref);chained(a.ref);b + 3 == 23 and a==255 and exposed.* == 20}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn default_aggregate_writes_include_every_possible_target() {
    let prefix="typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn choose(flag:bool,a:*Byte,b:*Byte)->Link {if flag {Link {pointer:a}} else {Link {pointer:b}}}
        fn write(flag:bool,a:*Byte,b:*Byte,.link:Link=choose(flag,a,b)) {link.pointer.* = 255;}";
    let package = compile(&format!(
        "{prefix}
        fn checked(flag:bool)->bool {{var a:Byte=10;var b:Byte=20;let c:Byte=30;let exposed=c.ref;
            write(flag,a.ref,b.ref);c + 3 == 33 and exposed.* == 30}}
        fn run()->bool {{checked(true) and checked(false)}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for read in ["a + 3", "b + 3"] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;write(flag,a.ref,b.ref);{read}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
    let failure = compile(&format!(
        "{prefix}
        fn bad(flag:bool,external:*Byte)->Byte {{var a:Byte=10;
            write(flag,external,a.ref);a + 3}}"
    ))
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn loop_aggregate_factories_preserve_disjoint_local_ranges() {
    let package=compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn make(p:*Byte)->Link {Link {pointer:p}}
        fn write(link:Link) {link.pointer.* = 255;}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            while active {write(make(a.ref));active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn projected(flag:bool)->bool {var active=flag;var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            while active {make(a.ref).pointer.* = 255;active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false) and projected(true)}"
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_aggregate_factories_follow_stable_loaded_and_nested_fields() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}struct Outer {link:Link}
        fn make(slot:**Byte)->Outer {Outer {link:Link {pointer:slot.*}}}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {make(p.ref).link.pointer.* = 255;active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_aggregate_factories_reject_rebound_source_slots() {
    for body in [
        "make(p.ref).pointer.* = 255;p=b.ref;",
        "p=b.ref;make(p.ref).pointer.* = 255;",
        "let slot=p.ref;slot.* = b.ref;make(p.ref).pointer.* = 255;",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Link {{pointer:*Byte}}
            fn make(slot:**Byte)->Link {{Link {{pointer:slot.*}}}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;var p=a.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn loop_saved_tuple_pointer_fields_preserve_disjoint_ranges() {
    let package = support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        fn write(packet:(*Byte,bool)) {let (p,flag)=packet;p.* = 255;}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            let packet=(a.ref,false);while active {write(packet);active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_saved_tuple_fields_track_parent_replacement_and_siblings() {
    let prefix = "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        struct Holder {packet:(*Byte,bool),flag:bool}
        fn write(packet:(*Byte,bool)) {let (p,flag)=packet;p.* = 255;}";
    let compile_bound = |body: &str| {
        support::compile_with_source_bindings(
            &format!("{prefix}{body}"),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input::List"),
                tuple: Some("input::Tuple"),
                ..Default::default()
            },
        )
    };
    let package=compile_bound(
        "fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            var holder=Holder {packet:(a.ref,false),flag:false};
            while active {write(holder.packet);holder.flag=true;active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}"
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "write(holder.packet);holder.packet=(b.ref,false);",
        "holder=Holder {packet:(b.ref,false),flag:false};write(holder.packet);",
    ] {
        let failure = compile_bound(&format!(
            "fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;
                var holder=Holder {{packet:(a.ref,false),flag:false}};
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn loop_aggregate_candidates_preserve_only_disjoint_ranges() {
    let prefix="typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn choose(flag:bool,a:*Byte,b:*Byte)->Link {if flag {Link {pointer:a}} else {Link {pointer:b}}}
        fn write(link:Link) {link.pointer.* = 255;}";
    let package = compile(&format!(
        "{prefix}
        fn checked(flag:bool)->bool {{var active=true;var a:Byte=10;var b:Byte=20;
            let c:Byte=30;let exposed=c.ref;
            while active {{write(choose(flag,a.ref,b.ref));active=false;}}
            c + 3 == 33 and exposed.* == 30}}
        fn run()->bool {{checked(true) and checked(false)}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for read in ["a + 3", "b + 3"] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(flag:bool,active:bool)->Byte {{var a:Byte=10;var b:Byte=20;
                while active {{write(choose(flag,a.ref,b.ref));}} {read}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
    for args in ["a.ref,external", "external,a.ref"] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(flag:bool,active:bool,external:*Byte)->Byte {{var a:Byte=10;
                while active {{write(choose(flag,{args}));}} a + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn writable_aggregate_returns_preserve_direct_input_addresses() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn set(p:*Byte)->Link {p.* = 20;Link {pointer:p}}
        fn run()->bool {var a:Byte=10;let link=set(a.ref);
            assumes a <= 252;link.pointer.* + 3 == 23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn set(p:*Byte)->Link {p.* = 255;Link {pointer:p}}
        fn bad()->Byte {var a:Byte=10;let link=set(a.ref);link.pointer.* + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn generic_aggregate_returns_keep_instantiated_pointer_fields_separate() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link where T {pointer:*T}
        fn make(p:*T)->Link<T> where T {Link<T> {pointer:p}}
        fn run()->bool {var number:Byte=20;var flag=false;
            let numeric=make(number.ref);let boolean=make(flag.ref);
            boolean.pointer.* = true;
            numeric.pointer.* + 3 == 23 and flag}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(
        "typealias Byte=.u8'builtin;struct Link where T {pointer:*T}
        fn make(p:*T)->Link<T> where T {Link<T> {pointer:p}}
        fn bad()->Byte {var number:Byte=20;var flag=false;
            let boolean=make(flag.ref);let numeric=make(number.ref);
            numeric.pointer.* = 255;numeric.pointer.* + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn indirect_scalar_calls_preserve_disjoint_local_ranges() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Cell {value:Byte,other:Byte}
        fn write(slot:**Byte) {slot.*.* = 255;}
        fn deep(slot:***Byte) {slot.*.*.* = 255;}
        fn field(slot:**Cell) {slot.*.*.value = 255;}
        fn run()->bool {var a:Byte=10;let b:Byte=20;let exposed=b.ref;let p=a.ref;let q=p.ref;
            write(q);deep(q.ref);
            var cell=Cell {value:10,other:20};let pointer=cell.ref;field(pointer.ref);
            b + 3 == 23 and exposed.* == 20 and cell.other + 3 == 23 and cell.value==255}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn indirect_scalar_calls_invalidate_every_possible_write_target() {
    let prefix = "typealias Byte=.u8'builtin;
        fn write(flag:bool,a:**Byte,b:**Byte) {if flag {a.*.* = 255;} else {b.*.* = 255;}}";
    let package=compile(&format!("{prefix}
        fn checked(flag:bool)->bool {{var a:Byte=10;var b:Byte=20;let p=a.ref;let q=b.ref;
            let c:Byte=30;let exposed=c.ref;write(flag,p.ref,q.ref);c + 3 == 33 and exposed.* == 30}}
        fn run()->bool {{checked(true) and checked(false)}}"
    )).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for read in ["a + 3", "b + 3"] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;let p=a.ref;let q=b.ref;
                write(flag,p.ref,q.ref);{read}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
    let failure = compile(&format!(
        "{prefix}
        fn bad(flag:bool,external:**Byte)->Byte {{var a:Byte=10;let p=a.ref;
            write(flag,p.ref,external);a + 3}}"
    ))
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn indirect_scalar_calls_load_slots_after_all_arguments() {
    let prefix = "typealias Byte=.u8'builtin;fn write(slot:**Byte,flag:bool) {slot.*.* = 255;}";
    let package = compile(&format!(
        "{prefix}
        fn run()->bool {{var a:Byte=10;var b:Byte=20;var p=a.ref;
            write(p.ref,do {{p=b.ref;false}});a + 3 == 13 and b==255}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(&format!(
        "{prefix}
        fn bad()->Byte {{var a:Byte=10;var b:Byte=20;var p=a.ref;
            write(p.ref,do {{p=b.ref;false}});b + 3}}"
    ))
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn loop_indirect_scalar_calls_preserve_disjoint_ranges() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn write(slot:**Byte) {slot.*.* = 255;}
        fn deep(slot:***Byte) {slot.*.*.* = 255;}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;let q=p.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {write(q);deep(q.ref);active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_indirect_scalar_calls_reject_changed_or_unknown_slots() {
    for body in [
        "write(p.ref);p=b.ref;",
        "p=b.ref;write(p.ref);",
        "let slot=p.ref;slot.* = b.ref;write(p.ref);",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;fn write(slot:**Byte) {{slot.*.* = 255;}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;var p=a.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn write(flag:bool,a:**Byte,b:**Byte) {if flag {a.*.* = 255;} else {b.*.* = 255;}}
        fn bad(flag:bool,external:**Byte)->Byte {var a:Byte=10;let p=a.ref;
            while flag {write(flag,p.ref,external);} a + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn scalar_writing_calls_return_stable_loaded_pointer_origins() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn after(slot:**Byte)->*Byte {slot.*.* = 20;slot.*}
        fn before(slot:**Byte)->*Byte {let p=slot.*;slot.*.* = 20;p}
        fn aggregate(slot:**Byte)->Link {slot.*.* = 20;Link {pointer:slot.*}}
        fn run()->bool {var a:Byte=10;let p=a.ref;
            let first=after(p.ref);assumes a <= 252;let x=first.* + 3 == 23;
            let second=before(p.ref);assumes a <= 252;let y=second.* + 3 == 23;
            let link=aggregate(p.ref);assumes a <= 252;x and y and link.pointer.* + 3 == 23}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scalar_writing_loaded_returns_do_not_restore_prewrite_ranges() {
    for body in [
        "slot.*.* = 255;slot.*",
        "let saved=slot.*;slot.*.* = 255;saved",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn write(slot:**Byte)->*Byte {{{body}}}
            fn bad()->Byte {{var a:Byte=10;let p=a.ref;let result=write(p.ref);result.* + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn loop_scalar_writing_returns_keep_stable_slot_origins() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn set(slot:**Byte)->*Byte {slot.*.* = 20;slot.*}
        fn link(slot:**Byte)->Link {slot.*.* = 20;Link {pointer:slot.*}}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {set(p.ref).* = 255;link(p.ref).pointer.* = 255;active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_scalar_writing_returns_reject_rebound_slots() {
    for body in [
        "set(p.ref).* = 255;p=b.ref;",
        "p=b.ref;set(p.ref).* = 255;",
        "link(p.ref).pointer.* = 255;p=b.ref;",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Link {{pointer:*Byte}}
            fn set(slot:**Byte)->*Byte {{slot.*.* = 20;slot.*}}
            fn link(slot:**Byte)->Link {{slot.*.* = 20;Link {{pointer:slot.*}}}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;var p=a.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn loop_scalar_return_origins_reject_parent_and_alias_replacement() {
    for body in [
        "set(holder.pointer.ref).* = 255;holder=Holder {pointer:b.ref,flag:false};",
        "holder.pointer=b.ref;set(holder.pointer.ref).* = 255;",
        "let alias=holder.pointer.ref;alias.* = b.ref;set(holder.pointer.ref).* = 255;",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Holder {{pointer:*Byte,flag:bool}}
            fn set(slot:**Byte)->*Byte {{slot.*.* = 20;slot.*}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;
                var holder=Holder {{pointer:a.ref,flag:false}};
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
    let package = compile(
        "typealias Byte=.u8'builtin;struct Holder {pointer:*Byte,flag:bool}
        fn set(slot:**Byte)->*Byte {slot.*.* = 20;slot.*}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let b:Byte=20;let exposed=b.ref;
            var holder=Holder {pointer:a.ref,flag:false};
            while active {set(holder.pointer.ref).* = 255;holder.flag=true;active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_default_loaded_pointers_preserve_disjoint_ranges() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn write(slot:**Byte,.target:*Byte=slot.*) {target.* = 255;}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {write(p.ref);active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_default_loaded_pointers_reject_rebinding_and_all_candidates() {
    for body in [
        "write(p.ref);p=b.ref;",
        "p=b.ref;write(p.ref);",
        "let alias=p.ref;alias.* = b.ref;write(p.ref);",
        "write(p.ref,.target=b.ref);",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;
            fn write(slot:**Byte,.target:*Byte=slot.*) {{target.* = 255;}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;var p=a.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
    let failure = compile(
        "typealias Byte=.u8'builtin;
        fn write(flag:bool,a:**Byte,b:**Byte,.p:*Byte=if flag {a.*} else {b.*}) {p.* = 255;}
        fn bad(flag:bool)->Byte {var a:Byte=10;var b:Byte=20;let p=a.ref;let q=b.ref;
            while flag {write(flag,p.ref,q.ref);} b + 3}",
    )
    .unwrap_err();
    assert!(
        failure.contains("machine arithmetic range is not proven"),
        "{failure}"
    );
}

#[test]
fn loop_default_loaded_pointers_follow_nested_slots_and_defaults() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        fn write(slot:***Byte,.middle:**Byte=slot.*,.target:*Byte=middle.*) {target.* = 255;}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;let q=p.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {write(q.ref);active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_defaults_can_read_owned_input_pointer_fields() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Holder {slot:**Byte}
        fn write(holder:Holder,.target:*Byte=holder.slot.*) {target.* = 255;}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {write(Holder {slot:p.ref});active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_owned_default_dependencies_chain_through_aggregate_factories() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Holder {slot:**Byte} struct Link {pointer:*Byte}
        fn make(slot:**Byte)->Link {Link {pointer:slot.*}}
        fn write(holder:Holder,.link:Link=make(holder.slot),.target:*Byte=link.pointer) {target.* = 255;}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {write(Holder {slot:p.ref});active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    ).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_owned_default_dependencies_do_not_hide_alias_writes() {
    for body in [
        "write(Holder {slot:p.ref});p=b.ref;",
        "p=b.ref;write(Holder {slot:p.ref});",
        "let alias=p.ref;alias.* = b.ref;write(Holder {slot:p.ref});",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Holder {{slot:**Byte}}
            fn write(holder:Holder,.target:*Byte=holder.slot.*) {{target.* = 255;}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;var p=a.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
    for read in ["a + 3", "b + 3"] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Pair {{left:**Byte,right:**Byte}}
            fn write(flag:bool,pair:Pair,.target:*Byte=if flag {{pair.left.*}} else {{pair.right.*}}) {{target.* = 255;}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;let p=a.ref;let q=b.ref;
                while flag {{write(flag,Pair {{left:p.ref,right:q.ref}});}} {read}}}"
        )).unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn loop_aggregate_return_fields_follow_prepared_defaults() {
    let package = compile(
        "typealias Byte=.u8'builtin;struct Link {pointer:*Byte}
        fn make(slot:**Byte,.target:*Byte=slot.*)->Link {Link {pointer:target}}
        fn checked(flag:bool)->bool {var active=flag;var a:Byte=10;let p=a.ref;
            let b:Byte=20;let exposed=b.ref;
            while active {make(p.ref).pointer.* = 255;active=false;}
            b + 3 == 23 and exposed.* == 20}
        fn run()->bool {checked(true) and checked(false)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn loop_nested_default_returns_preserve_all_candidate_origins() {
    let prefix = "typealias Byte=.u8'builtin;struct Link {pointer:*Byte} struct Outer {link:Link}
        fn make(flag:bool,left:**Byte,right:**Byte,
            .target:*Byte=if flag {left.*} else {right.*})->Outer {
            target.* = 255;Outer {link:Link {pointer:target}}}";
    let package = compile(&format!(
        "{prefix}
        fn checked(flag:bool)->bool {{var active=true;var a:Byte=10;var b:Byte=20;
            let p=a.ref;let q=b.ref;let c:Byte=30;let exposed=c.ref;
            while active {{make(flag,p.ref,q.ref).link.pointer.* = 255;active=false;}}
            c + 3 == 33 and exposed.* == 30}}
        fn run()->bool {{checked(true) and checked(false)}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for read in ["a + 3", "b + 3"] {
        let failure = compile(&format!(
            "{prefix}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;let p=a.ref;let q=b.ref;
                while flag {{make(flag,p.ref,q.ref).link.pointer.* = 255;}} {read}}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn loop_default_return_origins_reject_changed_slots_and_explicit_overrides() {
    for body in [
        "make(p.ref).pointer.* = 255;p=b.ref;",
        "p=b.ref;make(p.ref).pointer.* = 255;",
        "let alias=p.ref;alias.* = b.ref;make(p.ref).pointer.* = 255;",
        "make(p.ref,.target=b.ref).pointer.* = 255;",
    ] {
        let failure = compile(&format!(
            "typealias Byte=.u8'builtin;struct Link {{pointer:*Byte}}
            fn make(slot:**Byte,.target:*Byte=slot.*)->Link {{Link {{pointer:target}}}}
            fn bad(flag:bool)->Byte {{var a:Byte=10;var b:Byte=20;var p=a.ref;
                while flag {{{body}}} b + 3}}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("machine arithmetic range is not proven"),
            "{failure}"
        );
    }
}

#[test]
fn inline_type_tuple_patterns_use_protocol_views_and_definite_mismatches() {
    let source = "typealias Type=.Type'builtin;
        typealias Pair=(bool,(bool,bool));
        fn nested()->bool {inline if Pair is do {
            (A,(B,C)) if A==bool and B==bool and C==bool => true,
            _ => false
        }}
        fn mismatch()->bool {inline if bool is do {(_,_) => false,_ => true}}
        fn arity()->bool {inline if Pair is do {(_,_,_) => false,_ => true}}";
    let structural = compile(source).unwrap();
    let bound=support::compile_with_source_bindings(
        &format!("{source} pure comptime fn List<T>->Type {{T}} pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list:Some("input::List"),tuple:Some("input::Tuple"),..Default::default()
        },
    ).unwrap();
    for package in [&structural, &bound] {
        for entry in ["nested", "mismatch", "arity"] {
            let mut vm = Interpreter::new(package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(package, entry), vec![]).unwrap(),
                Value::Bool(true)
            );
        }
    }
    let failure=compile("typealias Type=.Type'builtin; pure comptime fn Opaque<T>->Type {(T,T)} fn bad()->bool {inline if Opaque<bool> is do {(_,_) => true,_ => false}}").unwrap_err();
    assert!(
        failure.contains("inline pattern selection requires"),
        "{failure}"
    );
}

#[test]
fn inline_exact_numeric_bindings_remain_comptime_constants() {
    let package = compile(
        "typealias Integer=.Integer'builtin; typealias Float=.Float'builtin;
        comptime fn large()->Integer {1000000000000000000000000000000000000000000}
        comptime fn fraction()->Float {1.25}
        fn integer()->bool {inline if large() is number do {
            inline if number==1000000000000000000000000000000000000000000 {true} else {false}
        } else {false}}
        fn decimal()->bool {inline if fraction() is value do {
            inline if value==1.25 {true} else {false}
        } else {false}}
        fn unused()->bool {inline if large() is unused do {true} else {false}}",
    )
    .unwrap();
    for entry in ["integer", "decimal", "unused"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
    let failure=compile("typealias Integer=.Integer'builtin; comptime fn large()->Integer {10} fn bad()->Integer {inline if large() is value do {value} else {0}}").unwrap_err();
    assert!(
        failure.contains("comptime-only value cannot inhabit runtime storage"),
        "{failure}"
    );
}

#[test]
fn inline_normal_form_patterns_skip_definite_mismatches_without_running_guards() {
    let package = compile(
        "typealias Type=.Type'builtin;
        pure comptime fn Box<T>->Type {T}
        pure comptime fn Other<T>->Type {T}
        comptime fn diverges()->bool {diverges()}
        typealias Pair=(bool,bool);
        typealias Pointer=*bool;
        typealias Callback=fn(bool)->bool;
        struct Record {flag:bool}
        fn primitive()->bool {inline if bool is do {Box<_> if diverges() => false,_ => true}}
        fn tuple()->bool {inline if Pair is do {Box<_> if diverges() => false,_ => true}}
        fn pointer()->bool {inline if Pointer is do {Box<_> if diverges() => false,_ => true}}
        fn callable()->bool {inline if Callback is do {Box<_> if diverges() => false,_ => true}}
        fn nominal()->bool {inline if Record is do {Box<_> if diverges() => false,_ => true}}
        fn other()->bool {inline if Other<bool> is do {Box<_> if diverges() => false,_ => true}}",
    )
    .unwrap();
    for entry in [
        "primitive",
        "tuple",
        "pointer",
        "callable",
        "nominal",
        "other",
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn tuple_enumeration_evaluates_input_once_and_keeps_runtime_elements() {
    let package = compile(
        "fn source()->(bool,bool) {(true,false)}
        fn observe(value:bool)->bool {value}
        fn first() {} fn rest() {}
        fn run()->bool {
            var result=true;
            inline for (index,element) in source()'tuple_enumerate {
                inline if index==0 {first();} else {rest();}
                result=observe(element);
            }
            result
        }",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "source", "first", "observe", "rest", "observe"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn tuple_enumeration_handles_heterogeneous_elements_and_open_guards() {
    let package = compile(
        "fn visit() {}
        fn run(comptime enabled:bool,values:(bool,.u8'builtin))->bool {
            inline for (index,element) if enabled and index==1 in values'tuple_enumerate {
                let held=element;visit();
            }
            true
        }
        fn yes()->bool {run(true,(true,7))}
        fn no()->bool {run(false,(false,9))}",
    )
    .unwrap();
    for (entry, expected) in [
        ("yes", vec!["yes", "run", "visit"]),
        ("no", vec!["no", "run"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), expected);
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn tuple_enumeration_rejects_runtime_element_in_comptime_guard() {
    for header in [
        "fn run(values:(bool,bool))",
        "fn run(comptime enabled:bool,values:(bool,bool))",
    ] {
        let source = format!(
            "{header} {{inline for (index,element) if element in values'tuple_enumerate {{}}}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(
            failure.contains("comptime") || failure.contains("staged"),
            "{failure}"
        );
    }
}

#[test]
fn tuple_enumeration_executes_with_the_bound_tuple_head() {
    let package=support::compile_with_source_bindings(
        "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        fn visit() {}
        fn walk(value:Tuple<bool,.u8'builtin>) {inline for (index,element) in value'tuple_enumerate {let held=element;visit();}}
        fn run()->bool {walk((true,7));true}",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {meta_list:Some("input::List"),tuple:Some("input::Tuple"),..Default::default()}
    ).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "walk", "visit", "visit"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn tuple_enumeration_keeps_purity_checks_for_filtered_bodies() {
    let package = compile(
        "pure fn walk(comptime enabled:bool,values:(bool,bool))->bool {
        var result=false;
        inline for (index,element) if enabled in values'tuple_enumerate {result=element;}
        result
    }
    fn yes()->bool {walk(true,(false,true))}
    fn no()->bool {walk(false,(true,true))}",
    )
    .unwrap();
    for (entry, expected) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
    }
    for source in [
        "pure fn bad(comptime enabled:bool,values:(bool,bool),pointer:*bool)->bool {inline for (index,element) if enabled in values'tuple_enumerate {pointer.* = element;}true} fn run(pointer:*bool)->bool {bad(false,(true,false),pointer)}",
        "fn take(value:bool) {} fn bad(values:(bool,bool)) {inline for (index,element) if false in values'tuple_enumerate {take(7);}}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn tuple_enumeration_checks_unused_pattern_computation_phases() {
    for source in [
        "comptime fn ct(value:bool)->bool {value} fn unused(comptime enabled:bool,values:(bool,bool)) {inline for (index,(element and ct(element) is result)) in values'tuple_enumerate {}}",
        "fn runtime(value:bool)->bool {value} fn unused(comptime enabled:bool,values:(bool,bool)) {inline for (index and runtime(index==0) is next,element) in values'tuple_enumerate {}}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn tuple_iteration_keeps_index_and_element_computations_in_their_stages() {
    let package = compile(
        "fn select(value:bool)->bool {not value}
         fn walk(comptime enabled:bool, values:(bool,bool))->bool {
             var answer=false;
             inline for ((index and index==0 is first),(element and select(element) is selected)) if enabled in values'tuple_enumerate {
                 inline if first {answer=false;} else {answer=selected;}
             }
             answer
         }
         fn run()->bool {walk(true,(false,false))}
         fn filtered()->bool {walk(false,(false,false))}",
    ).unwrap();
    for (entry, expected, calls) in [
        ("run", true, vec!["run", "walk", "select", "select"]),
        ("filtered", false, vec!["filtered", "walk"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(support::calls(&package, &vm), calls);
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn tuple_element_pattern_computation_can_use_completed_index() {
    let package = compile(
        "pure fn select(first:bool, value:bool)->bool {if first {value} else {not value}}
         pure fn walk(comptime enabled:bool, values:(bool,bool))->bool {
             var answer=false;
             inline for ((index and index==0 is first),(element and select(first,element) is selected)) if enabled in values'tuple_enumerate {
                 answer=selected;
             }
             answer
         }
         fn run()->bool {walk(true,(false,false))}
         fn filtered()->bool {walk(false,(false,false))}",
    ).unwrap();
    for (entry, expected, calls) in [
        ("run", true, vec!["run", "walk", "select", "select"]),
        ("filtered", false, vec!["filtered", "walk"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(support::calls(&package, &vm), calls);
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn tuple_iteration_abstract_index_inputs_preserve_pure_body_checks() {
    let package = compile(
        "pure fn walk(comptime enabled:bool, values:(bool,bool))->bool {
             var answer=false;
             inline for ((index and index==0 is first),element) if enabled in values'tuple_enumerate {
                 if first {answer=element;}
             }
             answer
         }
         fn yes()->bool {walk(true,(true,false))}
         fn no()->bool {walk(false,(true,false))}",
    ).unwrap();
    for (entry, expected) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
    let failure=compile(
        "pure fn bad(comptime enabled:bool,values:(bool,bool),pointer:*bool) {
             inline for ((index and index==0 is first),element) if enabled in values'tuple_enumerate {
                 if first {pointer.* = element;}
             }
         }
         fn run(pointer:*bool) {bad(false,(true,false),pointer);}",
    ).unwrap_err();
    assert!(
        failure.contains("P4") || failure.contains("pure"),
        "{failure}"
    );
}

#[test]
fn tuple_iteration_specializes_generic_element_calls_independently() {
    let package=compile(
        "fn identity(value:T)->T where T {value}
         fn walk(values:(A,B)) where A,B {
             inline for (index,element) in values'tuple_enumerate {
                 let held=identity(element);
             }
         }
         fn run()->bool {let a:.u8'builtin=7;let b:.u16'builtin=9;walk((true,a));walk((b,false));true}",
    ).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec![
            "run", "walk", "identity", "identity", "walk", "identity", "identity"
        ]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn tuple_iteration_resolves_each_generic_elements_trait_evidence() {
    let package = compile(
        "trait Ready {fn ready(self)->bool;}
         impl Ready for bool {fn ready(self)->bool {self}}
         impl Ready for .u8'builtin {fn ready(self)->bool {self==7}}
         fn walk(values:(A,B))->bool where A:-Ready,B:-Ready {
             var answer=false;
             inline for (index,element) in values'tuple_enumerate {answer=element.ready();}
             answer
         }
         fn run()->bool {let number:.u8'builtin=7;walk((false,number))}
         fn reverse()->bool {let number:.u8'builtin=7;walk((number,false))}",
    )
    .unwrap();
    for (entry, expected) in [("run", true), ("reverse", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn tuple_iteration_uses_generic_shape_evidence_without_unfolding_heads() {
    let package = compile(
        "fn identity(value:T)->T where T {value}
         fn walk(value:T) where T,A,B,requires T==(A,B) {
             inline for (index,element) in value'tuple_enumerate {let held=identity(element);}
         }
         fn run()->bool {walk<(bool,.u8'builtin),bool,.u8'builtin>((true,7));true}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "walk", "identity", "identity"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn open_inline_ranges_check_and_specialize_computed_patterns() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
         pure comptime fn step(index:Integer)->Integer {index + 1}
         fn visit() {}
         fn walk(comptime count:Integer)->bool {
             inline for (index and step(index) is next) if next!=2 in 0..count {
                 inline if next==index + 1 {visit();}
             }
             true
         }
         fn run()->bool {walk(4)}
         fn empty()->bool {walk(0)}",
    )
    .unwrap();
    for (entry, calls) in [
        ("run", vec!["run", "walk", "visit", "visit", "visit"]),
        ("empty", vec!["empty", "walk"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), calls);
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn open_inline_ranges_accept_irrefutable_aliases_and_alternatives() {
    for pattern in ["_ as index", "index or index"] {
        let source = format!(
            "pure fn walk(comptime count:.Integer'builtin)->bool {{
            var answer=false;
            inline for ({pattern}) if index==1 in 0..count {{answer=true;}}
            answer
        }} fn yes()->bool {{walk(3)}} fn no()->bool {{walk(0)}}"
        );
        let package = compile(&source).unwrap();
        for (entry, expected) in [("yes", true), ("no", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_inline_range_pattern_descriptions_do_not_invoke_computations() {
    let package = compile(
        "typealias Integer=.Integer'builtin;
         pure comptime fn forever()->Integer {forever()}
         pure fn walk(comptime count:Integer)->bool {
             inline for (index and forever() is next) in 0..count {inline if next==index {}}
             true
         }
         fn run()->bool {walk(0)}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), vec!["run", "walk"]);
    for source in [
        "fn runtime(value:bool)->bool {value} fn unused(comptime count:.Integer'builtin) {inline for (index and runtime(true) is next) in 0..count {}}",
        "fn unused(comptime count:.Integer'builtin,flag:bool) {inline for (index and flag is next) in 0..count {}}",
        "pure fn bad(comptime count:.Integer'builtin,pointer:*bool) {inline for (_ as index) if false in 0..count {pointer.* = false;}} fn run(pointer:*bool) {bad(0,pointer);}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn nonempty_tuple_iteration_can_return_without_a_trailing_value() {
    let package = compile(
        "fn first(comptime flag:bool, values:(bool,bool))->bool {
             inline for (index,element) in values'tuple_enumerate {return element;}
         }
         fn yes()->bool {first(true,(true,false))}
         fn no()->bool {first(false,(false,true))}",
    )
    .unwrap();
    for (entry, expected) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn tuple_iteration_completion_distinguishes_inner_and_outer_loop_exits() {
    let package = compile(
        "pure fn first(comptime staged:bool, values:(bool,bool),choose:bool)->bool {
             inline for (index,element) in values'tuple_enumerate {
                 while true {break;}
                 if choose {return element;} else {return not element;}
             }
         }
         fn yes()->bool {first(true,(false,true),false)}
         fn no()->bool {first(false,(false,true),true)}",
    )
    .unwrap();
    for (entry, expected) in [("yes", true), ("no", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
    for body in [
        "break;",
        "continue;",
        "let ignored:bool=do {break;true};",
        "if do {break;true} {return element;}",
        "if choose {return element;} else {break;}",
        "return do {break;true};",
        "while true {break all;}",
        "while true {continue all;}",
    ] {
        let source = format!(
            "fn bad(comptime staged:bool,values:(bool,bool),choose:bool)->bool {{
            inline for:all (index,element) in values'tuple_enumerate {{{body}}}
        }}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(failure.contains("type mismatch"), "{source}: {failure}");
    }
    assert!(compile("fn bad(comptime enabled:bool,values:(bool,bool))->bool {inline for (index,element) if enabled in values'tuple_enumerate {return element;}}").is_err());
}

#[test]
fn open_definition_preserves_nonreturning_initializers_and_conditions() {
    for body in [
        "let ignored = do {return value;};",
        "let ignored:bool = do {return value;};",
        "var ignored:bool = do {return value;};",
        "if do {return value;} {unreachable_call();};",
        "inline for (index,element) in (value,false)'tuple_enumerate {let ignored:bool=do {return element;};}",
        "inline for (index,element) in (value,false)'tuple_enumerate {if do {return element;} {unreachable_call();};}",
    ] {
        let source = format!(
            "fn unreachable_call() {{}}
            fn choose(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} {body}}}
            fn yes()->bool {{choose(true,true)}} fn no()->bool {{choose(false,false)}}"
        );
        let package = compile(&source).unwrap();
        for (entry, expected) in [("yes", true), ("no", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            assert_eq!(support::calls(&package, &vm), vec![entry, "choose"]);
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_nonreturning_regions_still_check_ordinary_operations() {
    for body in [
        "let ignored:bool=do {return value;};take(7);",
        "if do {return value;} {take(7);};",
        "let ignored:bool=do {return 7;};",
    ] {
        let source = format!(
            "fn take(value:bool) {{}} fn bad(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} {body}}}"
        );
        assert!(compile(&source).is_err(), "{source}");
    }
}

#[test]
fn open_literal_boolean_control_preserves_definite_returns() {
    for body in [
        "if true {return value;};",
        "if not false {return value;};",
        "let ignored:bool=if false {false} else {return value;};",
        "(not false) and do {return value;};",
        "(not true) or do {return value;};",
        "inline for (index,element) in (value,false)'tuple_enumerate {if true {return element;} else {break;}}",
        "inline for (index,element) in (value,false)'tuple_enumerate {false and do {break;true};return element;}",
        "inline for (index,element) in (value,false)'tuple_enumerate {when {false=>{break;},true=>{return element;},}}",
    ] {
        let source = format!(
            "fn choose(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} {body}}}
            fn yes()->bool {{choose(true,true)}} fn no()->bool {{choose(false,false)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{body}: {failure}"));
        for (entry, expected) in [("yes", true), ("no", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_boolean_completion_does_not_guess_runtime_conditions() {
    for body in [
        "if condition {return value;};",
        "if runtime_condition() {return value;};",
        "(not condition) and do {return value;};",
        "false and do {return value;};",
        "true or do {return value;};",
        "inline for (index,element) in (value,false)'tuple_enumerate {if true {break;} else {return element;}}",
        "inline for (index,element) in (value,false)'tuple_enumerate {condition and do {break;true};return element;}",
    ] {
        let source = format!(
            "fn runtime_condition()->bool {{true}} fn bad(comptime enabled:bool,condition:bool,value:bool)->bool {{inline if enabled {{}} {body}}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(failure.contains("type mismatch"), "{body}: {failure}");
    }
    for body in [
        "if true {return value;} else {take(7);}",
        "false and take(7);return value;",
    ] {
        let source = format!(
            "fn take(value:bool)->bool {{value}} fn bad(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} {body}}}"
        );
        assert!(compile(&source).is_err(), "{body}");
    }
}

#[test]
fn open_while_preserves_returning_and_noncompleting_bodies() {
    for body in [
        "while true {return value;}",
        "while not false {if value {return true;} else {return false;}}",
        "while do {return value;} {}",
        "while do {return value;} {break;}",
        "while true {while true {break;} return value;}",
    ] {
        let source = format!(
            "fn choose(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} {body}}}
            fn yes()->bool {{choose(true,true)}} fn no()->bool {{choose(false,false)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{body}: {failure}"));
        for (entry, expected) in [("yes", true), ("no", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_infinite_loops_keep_continue_distinct_from_break() {
    for body in [
        "while true {}",
        "while true {continue;}",
        "while true {continue;break;}",
        "while true {if false {break;}}",
        "while true {while true {break;}}",
        "while true {while true {}break;}",
        "while:all true {while true {continue all;}}",
    ] {
        let source = format!(
            "fn spin(comptime enabled:bool)->bool {{inline if enabled {{}} {body}}} fn run()->bool {{spin(true)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{body}: {failure}"));
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 100,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![])
                .unwrap_err()
                .kind,
            comptime_interpretor::ErrorKind::StepsExhausted,
            "{body}"
        );
    }
    for body in [
        "while condition {return true;}",
        "while true {if condition {break;}}",
        "while:all true {while true {break all;}}",
    ] {
        let source = format!(
            "fn bad(comptime enabled:bool,condition:bool)->bool {{inline if enabled {{}} {body}}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(failure.contains("type mismatch"), "{body}: {failure}");
    }
    assert!(compile("fn take(value:bool) {} fn bad(comptime enabled:bool)->bool {inline if enabled {} while true {continue;take(7);}}").is_err());
}

#[test]
fn open_loop_completion_uses_inert_match_selection() {
    for choice in [
        "true match {true=>{return value;},false=>{break;},}",
        "(true,false) match {(false,_)=>{break;},(true,_)=>{return value;},}",
        "false match {_=>{return value;},true=>{break;},}",
    ] {
        let source=format!("fn choose(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} while true {{{choice}}}}}
            fn yes()->bool {{choose(true,true)}} fn no()->bool {{choose(false,false)}}");
        let package = compile(&source).unwrap_or_else(|failure| panic!("{choice}: {failure}"));
        for (entry, expected) in [("yes", true), ("no", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_match_completion_retains_unknown_and_reachable_transfers() {
    for choice in [
        "condition match {true=>{return value;},false=>{break;},}",
        "true match {true=>{break;},false=>{return value;},}",
        "true match {true if yes()=>{return value;},_=>{break;},}",
        "true match {true=>{return value;},}",
        "true match {true=>{return value;},false=>{take(7);break;},}",
    ] {
        let source=format!("fn yes()->bool {{true}} fn take(value:bool) {{}}
            fn bad(comptime enabled:bool,condition:bool,value:bool)->bool {{inline if enabled {{}} while true {{{choice}}}}}");
        assert!(compile(&source).is_err(), "{choice}");
    }
    let package=compile("fn spin(comptime enabled:bool)->bool {inline if enabled {} while true {true match {true=>{continue;},false=>{break;},}}} fn run()->bool {spin(true)}").unwrap();
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let mut vm = Interpreter::new(
        &package,
        Budget {
            steps: 100,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::StepsExhausted
    );
}

#[test]
fn open_loop_completion_preserves_aggregate_evaluation_order() {
    for init in [
        "(do {return value;},do {break;false})",
        "(do {return value;},do {continue;false})",
        "(true,(do {return value;},do {break;false}))",
        "Pair {first:do {return value;},second:do {break;false}}",
        "Choice.pair(do {return value;},do {break;false})",
    ] {
        let source=format!("struct Pair {{first:bool,second:bool}} enum Choice {{pair(bool,bool)}} fn choose(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} while true {{let ignored={init};}}}}
            fn yes()->bool {{choose(true,true)}} fn no()->bool {{choose(false,false)}}");
        let package = compile(&source).unwrap_or_else(|failure| panic!("{init}: {failure}"));
        for (entry, expected) in [("yes", true), ("no", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_aggregate_completion_keeps_reachable_exits_and_later_type_checks() {
    for init in [
        "(do {break;false},do {return value;})",
        "(do {if condition {break;} false},do {return value;})",
        "(do {return value;},take(7))",
    ] {
        let source = format!(
            "fn take(value:bool)->bool {{value}} fn bad(comptime enabled:bool,condition:bool,value:bool)->bool {{inline if enabled {{}} while true {{let ignored={init};}}}}"
        );
        assert!(compile(&source).is_err(), "{init}");
    }
}

#[test]
fn open_calls_preserve_nonreturning_input_order() {
    for body in [
        "leaf(do {return value;},false);",
        "while true {named(.second=do {return value;},.first=do {break;false});}",
        "while true {named_generic<bool>(.second=do {return value;},.first=do {break;false});}",
        "while true {retry(|item| item,do {return value;},do {break;false},bool_identity);}",
        "while true {leaf(do {return value;},do {break;false});}",
        "while true {generic(do {return value;},do {break;false},true);}",
    ] {
        let source = format!(
            "fn leaf(first:bool,second:bool)->bool {{first}}
            fn generic(first:T,second:T,hint:T)->T where T {{first}}
            fn named(implicit first:bool,implicit second:bool)->bool {{first}}
            fn named_generic(implicit first:T,implicit second:T)->T where T {{first}}
            fn bool_identity(value:bool)->bool {{value}}
            fn retry(first:T,second:bool,third:bool,hint:T)->bool where T {{false}}
            fn choose(comptime enabled:bool,value:bool)->bool {{inline if enabled {{}} {body}}}
            fn yes()->bool {{choose(true,true)}} fn no()->bool {{choose(false,false)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{body}: {failure}"));
        for (entry, expected) in [("yes", true), ("no", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            assert_eq!(support::calls(&package, &vm), vec![entry, "choose"]);
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_eager_binary_operands_stop_transfers_after_the_left_exits() {
    let package=compile("fn choose(comptime enabled:bool,value:bool,condition:bool)->bool {inline if enabled {} while true {let ignored=do {return value;} == do {if condition {break;} false};}} fn run()->bool {choose(true,true,true)}").unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn open_call_completion_preserves_earlier_exits_and_input_errors() {
    for call in [
        "leaf(do {if condition {break;} false},do {return value;})",
        "named(.first=do {break;false},.second=do {return value;})",
        "retry(do {if condition {break;} |item| item},do {return value;},bool_identity)",
        "leaf(do {return value;},7)",
    ] {
        let source=format!("fn leaf(first:bool,second:bool)->bool {{first}}
            fn named(implicit first:bool,implicit second:bool)->bool {{first}}
            fn bool_identity(value:bool)->bool {{value}}
            fn retry(first:T,second:bool,hint:T)->bool where T {{false}}
            fn bad(comptime enabled:bool,condition:bool,value:bool)->bool {{inline if enabled {{}} while true {{{call};}}}}");
        let failure = compile(&source).unwrap_err();
        assert!(
            !failure.contains("parsing failed") && !failure.contains("HIR lowering failed"),
            "{call}: {failure}"
        );
    }
}

#[test]
fn open_definition_calls_accept_established_tuple_spreads() {
    let package=compile("fn select(first:bool,second:bool)->bool {second}
        pure fn choose(comptime enabled:bool,values:(bool,bool))->bool {inline if enabled {} select(...values)}
        fn run()->bool {choose(true,(false,true))}").unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "choose", "select"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn open_tuple_spread_calls_keep_inference_defaults_and_single_evaluation() {
    for (declarations, call, expected_calls) in [
        (
            "fn packet()->(bool,bool) {(false,true)} fn select(first:bool,second:bool)->bool {second}",
            "select(...packet())",
            vec!["run", "choose", "packet", "select"],
        ),
        (
            "fn packet()->(.u8'builtin,bool) {(7,true)} fn select(first:A,second:B)->B where A,B {second}",
            "select(...packet())",
            vec!["run", "choose", "packet", "select"],
        ),
        (
            "fn packet()->(bool,) {(false,)} pure fn fallback()->bool {true} fn select(first:bool,.second:bool=fallback())->bool {second}",
            "select(...packet())",
            vec!["run", "choose", "packet", "fallback", "select"],
        ),
        (
            "fn packet()->(bool,) {(false,)} pure fn fallback()->bool {false} fn supplied()->bool {true} fn select(first:bool,.second:bool=fallback())->bool {second}",
            "select(...packet(),.second=supplied())",
            vec!["run", "choose", "packet", "supplied", "select"],
        ),
    ] {
        let source = format!(
            "{declarations} fn choose(comptime enabled:bool)->bool {{inline if enabled {{}} {call}}} fn run()->bool {{choose(true)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{source}: {failure}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), expected_calls);
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
    for input in ["true", "(true,)", "(true,7)", "(true,false,true)"] {
        let source = format!(
            "fn select(first:bool,second:bool)->bool {{second}} fn bad(comptime enabled:bool)->bool {{inline if enabled {{}} select(...{input})}}"
        );
        assert!(compile(&source).is_err(), "{source}");
    }
}

#[test]
fn open_tuple_spread_call_requires_packet_shape_evidence() {
    let failure = compile(
        "typealias Type=.Type'builtin;
        comptime fn Packet(flag:bool)->Type {if flag {(bool,bool)} else {bool}}
        fn select(first:bool,second:bool)->bool {second}
        fn unused(comptime flag:bool,values:defer Packet(flag))->bool {select(...values)}",
    )
    .unwrap_err();
    assert!(
        !failure.contains("parsing failed") && !failure.contains("HIR lowering failed"),
        "{failure}"
    );
}

#[test]
fn open_spread_packets_can_select_inline_runtime_values() {
    let package=compile("fn yes()->(bool,bool) {(false,true)} fn no()->(bool,bool) {(true,false)}
        fn select(first:bool,second:bool)->bool {second}
        fn choose(comptime enabled:bool)->bool {select(...do {inline if enabled {yes()} else {no()}})}
        fn run()->bool {choose(true)} fn other()->bool {choose(false)}").unwrap();
    for (entry, expected, calls) in [
        ("run", true, vec!["run", "choose", "yes", "select"]),
        ("other", false, vec!["other", "choose", "no", "select"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(support::calls(&package, &vm), calls);
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn open_inline_spread_packets_check_all_selectors_and_branches() {
    for body in [
        "select(...do {inline if runtime {(false,true)} else {(true,false)}})",
        "select(...do {inline if enabled {(false,true)} else {(true,7)}})",
        "select(...do {inline if enabled {(false,)} else {(true,)}}, do {inline if runtime {true} else {false}})",
    ] {
        let source = format!(
            "fn select(first:bool,second:bool)->bool {{second}} fn unused(comptime enabled:bool,runtime:bool)->bool {{{body}}}"
        );
        let failure = compile(&source).unwrap_err();
        assert!(
            !failure.contains("parsing failed") && !failure.contains("HIR lowering failed"),
            "{failure}"
        );
    }
}

#[test]
fn open_inline_spreads_receive_callable_element_context() {
    let package = compile("typealias Callback=fn(bool)->bool;
        fn apply(f:Callback,value:bool)->bool {f(value)}
        fn choose(comptime flag:bool)->bool {apply(...do {inline if flag {(|x| x,true)} else {(|x| not x,true)}})}
        fn run()->bool {choose(true)} fn other()->bool {choose(false)}").unwrap();
    for (entry, expected) in [("run", true), ("other", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn open_inline_spreads_contextualize_named_inputs_and_multiple_packets() {
    for call in [
        "apply(...do {inline if flag {(|x| x,)} else {(|x| not x,)}}, ...do {inline if flag {(true,)} else {(true,)}})",
        "generic(...do {inline if flag {(|x| x,true)} else {(|x| not x,true)}})",
        "named(...do {inline if flag {(true,)} else {(true,)}}, .f=do {inline if flag {|x| x} else {|x| not x}})",
    ] {
        let source = format!(
            "typealias Callback=fn(bool)->bool;
            fn apply(f:Callback,value:bool)->bool {{f(value)}}
            fn named(value:bool,implicit f:Callback)->bool {{f(value)}}
            fn generic(f:fn(T)->T,value:T)->T where T {{f(value)}}
            fn choose(comptime flag:bool)->bool {{{call}}}
            fn run()->bool {{choose(true)}} fn other()->bool {{choose(false)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{source}: {failure}"));
        for (entry, expected) in [("run", true), ("other", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_to_open_literal_spreads_preserve_parameter_stages() {
    for declarations in [
        "fn select(comptime flag:bool,first:bool,second:bool)->bool {inline if flag {first} else {second}}",
        "fn select(comptime flag:bool,first:T,second:T)->T where T {inline if flag {first} else {second}}",
    ] {
        let source = format!(
            "{declarations}
            fn choose(comptime flag:bool,value:bool)->bool {{select(...(flag,value,false))}}
            fn run()->bool {{choose(true,true)}} fn other()->bool {{choose(false,true)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{source}: {failure}"));
        for (entry, expected) in [("run", true), ("other", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn literal_spread_forwarding_propagates_transitively_in_source_order() {
    for call in [
        "select(...(flag,),...(first(),second()))",
        "select(flag,...(first(),second()))",
        "select(...(flag,first()),second())",
        "select(...(),...(flag,first(),second()))",
    ] {
        let source=format!("fn run()->bool {{outer(true)}} fn other()->bool {{outer(false)}}
            fn outer(comptime flag:bool)->bool {{let forwarded=flag; middle(...(forwarded,))}}
            fn middle(comptime flag:bool)->bool {{{call}}}
            fn select(comptime flag:bool,first:T,second:T)->T where T {{inline if flag {{first}} else {{second}}}}
            fn first()->bool {{true}} fn second()->bool {{false}}");
        let package = compile(&source).unwrap_or_else(|failure| panic!("{source}: {failure}"));
        for (entry, expected) in [("run", true), ("other", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            assert_eq!(
                support::calls(&package, &vm),
                vec![entry, "outer", "middle", "first", "second", "select"]
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn open_literal_spreads_do_not_hide_invalid_staged_or_runtime_inputs() {
    for input in [
        "(runtime,true,false)",
        "(flag,true,7)",
        "(flag,true)",
        "(flag,true,false,true)",
    ] {
        let source=format!("fn select(comptime flag:bool,first:bool,second:bool)->bool {{inline if flag {{first}} else {{second}}}}
            fn unused(comptime flag:bool,runtime:bool)->bool {{inline if flag {{}} select(...{input})}}");
        let failure = compile(&source).unwrap_err();
        assert!(
            !failure.contains("parsing failed") && !failure.contains("HIR lowering failed"),
            "{failure}"
        );
    }
}

#[test]
fn open_targets_with_completed_staged_inputs_accept_computed_spreads() {
    for target in [
        "fn select(comptime selected:bool,first:bool,second:bool)->bool {inline if selected {second} else {first}}",
        "fn select(comptime selected:bool,first:T,second:T)->T where T {inline if selected {second} else {first}}",
    ] {
        let source = format!(
            "{target} fn packet()->(bool,bool) {{(false,true)}}
            fn choose(comptime flag:bool)->bool {{inline if flag {{}} select(true,...packet())}}
            fn run()->bool {{choose(true)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{source}: {failure}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm),
            vec!["run", "choose", "packet", "select"]
        );
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn computed_spreads_to_open_targets_keep_defaults_and_deferred_annotations() {
    for (declarations, call, calls) in [
        (
            "fn packet()->(bool,) {(false,)} fn select(comptime flag:bool,first:bool,.second:bool=true)->bool {inline if flag {second} else {first}}",
            "select(true,...packet())",
            vec!["run", "forward", "packet", "select"],
        ),
        (
            "fn packet()->(bool,) {(false,)} fn supplied()->bool {true} fn select(comptime flag:bool,first:bool,.second:bool=true)->bool {inline if flag {second} else {first}}",
            "select(true,...packet(),.second=supplied())",
            vec!["run", "forward", "packet", "supplied", "select"],
        ),
        (
            "typealias Type=.Type'builtin; comptime fn Select(flag:bool)->Type {if flag {bool} else {.u8'builtin}} fn packet()->(bool,) {(true,)} fn select(comptime flag:bool,value:defer Select(flag))->defer Select(flag) {value}",
            "select(true,...packet())",
            vec!["run", "forward", "packet", "select"],
        ),
    ] {
        let source = format!(
            "{declarations} fn forward(comptime flag:bool)->bool {{inline if flag {{}} {call}}} fn run()->bool {{forward(true)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{source}: {failure}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), calls);
        luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    }
}

#[test]
fn open_runtime_defaults_use_complete_p3_call_graph_evidence() {
    let package=compile("fn packet()->(bool,) {(false,)}
        fn select(comptime flag:bool,first:bool,.second:bool=fallback())->bool {inline if flag {second} else {first}}
        pure fn fallback()->bool {leaf()} pure fn leaf()->bool {true}
        fn forward(comptime flag:bool)->bool {inline if flag {} select(true,...packet())}
        fn run()->bool {forward(true)}").unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "forward", "packet", "fallback", "leaf", "select"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn pure_open_runtime_recursion_uses_ordinary_behavior_witnesses() {
    for definitions in [
        "pure fn recur(comptime enabled:bool,done:bool)->bool {inline if enabled {if done {true} else {recur(enabled,true)}} else {false}}",
        "pure fn recur(comptime enabled:bool,done:bool)->bool {inline if enabled {if done {true} else {other(enabled,true)}} else {false}} pure fn other(comptime enabled:bool,done:bool)->bool {inline if enabled {recur(enabled,done)} else {false}}",
    ] {
        let source = format!(
            "{definitions} fn run()->bool {{recur(true,false)}} fn disabled()->bool {{recur(false,false)}}"
        );
        let package = compile(&source).unwrap_or_else(|failure| panic!("{source}: {failure}"));
        for (entry, expected) in [("run", true), ("disabled", false)] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, entry), vec![]).unwrap(),
                Value::Bool(expected)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
}

#[test]
fn pure_open_generic_recursion_retains_clause_bindings() {
    let package=compile("pure fn recur(comptime enabled:bool,done:bool)->bool where T {inline if enabled {if done {true} else {recur<T>(enabled,true)}} else {false}} fn run()->bool {recur<bool>(true,false)}").unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn pure_open_recursive_calls_prepare_omitted_defaults() {
    let package=compile("pure fn next()->bool {true}
        pure fn recur(comptime enabled:bool,done:bool,.answer:bool=next())->bool {inline if enabled {if done {answer} else {recur(enabled,true)}} else {false}}
        fn run()->bool {recur(true,false)}").unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "next", "recur", "next", "recur"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn pure_open_recursive_defaults_preserve_dependency_order_and_overrides() {
    let package=compile("pure fn base()->bool {false} pure fn invert(value:bool)->bool {not value}
        fn supplied()->bool {true}
        pure fn recur(comptime enabled:bool,done:bool,.first:bool=base(),.answer:bool=invert(first))->bool {inline if enabled {if done {answer} else {recur(enabled,true)}} else {false}}
        fn run()->bool {recur(true,false,.first=supplied())}
        fn direct()->bool {recur(true,true)}").unwrap();
    for (entry, expected_calls) in [
        (
            "run",
            vec![
                "run", "supplied", "invert", "recur", "base", "invert", "recur",
            ],
        ),
        ("direct", vec!["direct", "base", "invert", "recur"]),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), expected_calls);
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn pure_open_recursive_calls_keep_staged_defaults_symbolic() {
    let package=compile("pure fn recur(comptime enabled:bool,done:bool,comptime .selected:bool=enabled)->bool {inline if selected {if done {true} else {recur(enabled,true)}} else {false}}
        fn run()->bool {recur(true,false)} fn disabled()->bool {recur(true,false,.selected=false)}").unwrap();
    for (entry, expected) in [("run", true), ("disabled", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn pure_open_generic_recursive_defaults_keep_instance_types_separate() {
    let package=compile("pure fn recur(comptime enabled:bool,done:bool,.callback:fn(T)->T=|value| value)->bool where T {inline if enabled {if done {true} else {recur<T>(enabled,true)}} else {false}}
        fn boolean()->bool {recur<bool>(true,false)}
        fn byte()->bool {recur<.u8'builtin>(true,false)}").unwrap();
    for entry in ["boolean", "byte"] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn computed_spread_forwarding_propagates_through_closed_signatures() {
    let source = "fn run()->bool {outer(true)} fn other()->bool {outer(false)}
        fn outer(comptime flag:bool)->bool {middle(flag)}
        fn middle(comptime flag:bool)->bool {select(flag,...do {(first(),second())})}
        fn select(comptime flag:bool,first:bool,second:bool)->bool {inline if flag {first} else {second}}
        fn first()->bool {true} fn second()->bool {false}";
    let package = compile(source).unwrap();
    for (entry, expected) in [("run", true), ("other", false)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(
            support::calls(&package, &vm),
            vec![entry, "outer", "middle", "first", "second", "select"]
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn expanded_tuple_index_storage_error_points_into_user_source() {
    for source in [
        "fn run(values:(bool,bool)) {inline for (index,element) in values'tuple_enumerate {let invalid=index;}}",
        "fn run() {select(true,(true,false))} fn select(comptime flag:bool,values:(bool,bool)) {inline if flag {inline for (index,element) in values'tuple_enumerate {let invalid=index;}}}",
    ] {
        let error = compile(source).unwrap_err();
        assert!(
            error.contains("comptime-only value cannot inhabit runtime storage"),
            "{error}"
        );
        assert!(!error.contains("4294967295"), "{error}");
    }
}

#[test]
fn inline_conditions_work_in_initializers_and_call_arguments() {
    let source = "fn identity(value:bool)->bool {value}
        fn run(value:bool)->bool {
            let selected=inline if true {value} else {not value};
            identity(inline when {false=>not selected,else=>selected})
        }";
    let package = compile(source).unwrap();
    for value in [false, true] {
        assert_eq!(
            Interpreter::new(&package, Budget::default())
                .unwrap()
                .invoke(support::id(&package, "run"), vec![Value::Bool(value)])
                .unwrap(),
            Value::Bool(value)
        );
    }
}

#[test]
fn inline_ranges_filter_refutable_patterns_before_guard_invocation() {
    let package = compile(
        "comptime fn never()->bool {never()}
        fn visit() {}
        fn run()->bool {inline for (1 or 3) in 0..5 {visit();} true}
        fn skipped()->bool {inline for 9 if never() in 0..3 {visit();} true}
        fn bound()->bool {inline for (1 as index) in 0..3 {
            inline if index==1 {visit();}
        } true}",
    )
    .unwrap();
    for (entry, visits) in [("run", 2), ("skipped", 0), ("bound", 1)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| *name == "visit")
                .count(),
            visits
        );
    }
}

#[test]
fn filtered_inline_ranges_keep_control_flow_and_skip_unselected_invocations() {
    let package = compile(
        "comptime fn never()->bool {never()}
        fn visit() {}
        fn run()->bool {
            inline for:scan ((1 or 3 or 4) as index) in 0..6 {
                inline if index==1 {continue scan;}
                visit();
                break scan;
            }
            true
        }
        fn transform()->bool {inline for (index and index==2 is true) in 0..4 {visit();} true}
        fn skipped()->bool {inline for 9 in 0..3 {inline if never() {visit();}} true}
        fn empty()->bool {inline for 9 if never() in 0..0 {visit();} true}",
    )
    .unwrap();
    for (entry, visits) in [("run", 1), ("transform", 1), ("skipped", 0), ("empty", 0)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| *name == "visit")
                .count(),
            visits
        );
    }
}

#[test]
fn filtered_inline_ranges_still_check_unselected_bodies_and_guard_phases() {
    for source in [
        "fn run(){inline for 9 in 0..3 {let wrong:bool=3;}}",
        "fn run(flag:bool){inline for 9 if flag in 0..3 {}}",
        "fn run(flag:bool){let alias=flag;inline for 9 if alias in 0..3 {}}",
        "fn runtime()->bool {true} fn run(){inline for 9 if runtime() in 0..3 {}}",
        "fn run(flag:bool){inline for (i and flag is true) in 0..0 {}}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn open_inline_ranges_filter_patterns_and_preserve_definition_checks() {
    let package = compile(
        "pure fn walk(comptime count:.Integer'builtin)->bool {
            var selected=false;
            inline for (1 as index) if index==1 in 0..count {selected=true;}
            selected
        }
        fn yes()->bool {walk(3)} fn no()->bool {walk(1)}",
    )
    .unwrap();
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 1_000_000).unwrap(),
            Value::Bool(expected)
        );
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
    let invalid = "pure fn bad(comptime count:.Integer'builtin,p:*bool) {
        inline for 9 in 0..count {p.* = false;}
    } fn run(p:*bool){bad(0,p);}";
    assert!(compile(invalid).is_err());
}

#[test]
fn skipped_range_guards_accept_comptime_function_inputs() {
    for guard in ["flag", "alias"] {
        let source = format!(
            "comptime fn choose(flag:bool)->bool {{
                var alias=flag;
                inline for 9 if {guard} in 0..3 {{}}
                flag
            }}"
        );
        let package = compile(&source).unwrap();
        for flag in [true, false] {
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, "choose"), vec![Value::Bool(flag)])
                    .unwrap(),
                Value::Bool(flag)
            );
        }
    }
}

#[test]
fn tuple_enumeration_filters_refutable_comptime_indices() {
    let package = compile(
        "comptime fn never()->bool {never()}
        fn visit(value:bool)->bool {value}
        fn run()->bool {
            var answer=false;
            inline for (1,value) in (false,true,false)'tuple_enumerate {answer=visit(value);}
            answer
        }
        fn skipped()->bool {
            inline for (9,value) if never() in (false,true)'tuple_enumerate {visit(value);}
            true
        }",
    )
    .unwrap();
    for (entry, visits) in [("run", 1), ("skipped", 0)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm)
                .iter()
                .filter(|name| *name == "visit")
                .count(),
            visits
        );
    }
}

#[test]
fn open_tuple_index_filters_preserve_phases_and_completion() {
    let package = compile("pure fn select(comptime enabled:bool,values:(bool,bool,bool))->bool {
            var answer=false;
            inline for ((0 or 2) as index,value) if enabled in values'tuple_enumerate {answer=answer or value;}
            answer
        }
        fn yes()->bool {select(true,(false,false,true))}
        fn no()->bool {select(true,(false,true,false))}
        fn disabled()->bool {select(false,(true,true,true))}").unwrap();
    for (entry, expected) in [("yes", true), ("no", false), ("disabled", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 1_000_000).unwrap(),
            Value::Bool(expected)
        );
    }
    for source in [
        "fn run(flag:bool){inline for (9,value) if flag in (true,false)'tuple_enumerate {}}",
        "fn run(){inline for (9,value) if value in (true,false)'tuple_enumerate {}}",
        "fn run(){inline for (9,value) in (true,false)'tuple_enumerate {let wrong:bool=3;}}",
        "fn missing(comptime enabled:bool,values:(bool,bool))->bool {inline for (9,value) in values'tuple_enumerate {return value;}}",
        "pure fn bad(comptime enabled:bool,p:*bool){inline for (9,value) in (true,false)'tuple_enumerate {p.* = value;}}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn tuple_enumeration_filters_runtime_elements() {
    let package = compile(
        "fn visit() {}
        fn select(values:(bool,bool,bool))->bool {
            inline for (_,true) in values'tuple_enumerate {visit();}
            true
        }
        fn run()->bool {select((false,true,true))}",
    )
    .unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| *name == "visit")
            .count(),
        2
    );
}

#[test]
fn tuple_runtime_pattern_observations_execute_once_per_reached_element() {
    let package = compile("fn visit() {} fn observe(value:bool)->bool {value}
        fn select(comptime enabled:bool,values:(bool,bool,bool))->bool {
            inline for (_, (value and observe(value) is true)) if enabled in values'tuple_enumerate {visit();}
            true
        }
        fn run()->bool {select(true,(false,true,true))}
        fn skipped()->bool {select(false,(false,true,true))}").unwrap();
    for (entry, observes, visits) in [("run", 3, 2), ("skipped", 0, 0)] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        let calls = support::calls(&package, &vm);
        assert_eq!(
            calls.iter().filter(|name| *name == "observe").count(),
            observes
        );
        assert_eq!(calls.iter().filter(|name| *name == "visit").count(), visits);
    }
    for source in [
        "fn missing(comptime enabled:bool,values:(bool,bool))->bool {inline for (_,true) in values'tuple_enumerate {return true;}}",
        "fn missing(values:(bool,bool))->bool {inline for (_,true) in values'tuple_enumerate {return true;}}",
        "pure fn bad(comptime enabled:bool,p:*bool){inline for (_,true) in (false,false)'tuple_enumerate {p.* = true;}}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
}

#[test]
fn iteration_bindings_cannot_be_assigned_inside_sequential_patterns() {
    for iteration in [
        "inline for (index and do {index=1;true} is true) in 0..2 {}",
        "inline for ((index and do {index=1;true} is true),element) in (true,false)'tuple_enumerate {}",
        "inline for (index,(element and do {element=false;true} is true)) in (true,false)'tuple_enumerate {}",
    ] {
        for signature in ["fn unused()", "fn unused(comptime enabled:bool)"] {
            let source = format!("{signature} {{{iteration}}}");
            assert!(compile(&source).is_err(), "{signature}: {iteration}");
        }
    }
}
