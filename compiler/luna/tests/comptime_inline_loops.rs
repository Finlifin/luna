//! Inline range controls executed wholly within comptime invocation.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn invoke(source: &str) -> Value {
    let package = support::compile(source);
    luna::invocation::invoke(&package, "run", 100_000).unwrap()
}

#[test]
fn ordinary_inputs_supply_exclusive_inclusive_and_descending_ranges() {
    assert_eq!(
        invoke(
            r#"
        comptime fn sum(start:.Integer'builtin,end:.Integer'builtin)->.Integer'builtin {
            var sum:.Integer'builtin=0;
            inline for i in start..end {sum = sum + i;}
            sum
        }
        comptime fn inclusive(start:.Integer'builtin,end:.Integer'builtin)->.Integer'builtin {
            var sum:.Integer'builtin=0;
            inline for i in start..=end {sum = sum + i;}
            sum
        }
        comptime fn run()->bool {
            sum(0,4)==6 and sum(4,0)==0 and sum(2,2)==0
                and inclusive(-3,1)==-5 and inclusive(2,2)==2
        }
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn boundaries_run_once_before_per_iteration_guards() {
    assert_eq!(
        invoke(
            r#"
        comptime fn bound(trace:*.Integer'builtin,digit:.Integer'builtin,value:.Integer'builtin)->.Integer'builtin {
            trace.* = trace.* * 10 + digit; value
        }
        comptime fn keep(trace:*.Integer'builtin,i:.Integer'builtin)->bool {
            trace.* = trace.* * 10 + i + 3; i!=1
        }
        comptime fn scan(trace:*.Integer'builtin,count:.Integer'builtin)->.Integer'builtin {
            var sum:.Integer'builtin=0;
            inline for i if keep(trace,i) in bound(trace,1,0)..bound(trace,2,count) {
                sum = sum + i;
            }
            sum
        }
        comptime fn run()->bool {
            var trace:.Integer'builtin=0;
            let sum=scan(trace.ref,3);
            sum==2 and trace==12345
        }
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn transformed_patterns_feed_guards_and_nested_inline_branches() {
    assert_eq!(
        invoke(
            r#"
        comptime fn scan(count:.Integer'builtin)->.Integer'builtin {
            var sum:.Integer'builtin=0;
            inline for (i and i + 1 is shifted) if shifted!=2 in 0..count {
                inline if shifted==3 {sum = sum + 10;} else {sum = sum + shifted;}
            }
            sum
        }
        comptime fn run()->bool {scan(3)==11 and scan(0)==0}
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn continue_break_and_return_preserve_loop_targets() {
    assert_eq!(
        invoke(
            r#"
        comptime fn scan(count:.Integer'builtin)->.Integer'builtin {
            var sum:.Integer'builtin=0;
            inline for i in 0..count {
                if i==1 {continue;}
                if i==4 {break;}
                sum = sum + i;
            }
            sum
        }
        comptime fn first(count:.Integer'builtin)->.Integer'builtin {
            inline for i in 0..count {if i==2 {return i;}}
            -1
        }
        comptime fn run()->bool {scan(9)==5 and first(4)==2 and first(0)==-1}
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn invalid_empty_loop_bodies_and_runtime_boundaries_are_rejected() {
    for source in [
        "comptime fn unused(){inline for i in 0..0 {let invalid:bool=i;}}",
        "fn runtime_value()->bool {true} comptime fn unused(){inline for i in 0..0 {runtime_value();}}",
        "comptime fn unused(){inline for i in 0..1 {i=2;}}",
        "fn runtime_bound()->.Integer'builtin {0} comptime fn unused(){inline for i in 0..runtime_bound() {()}}",
        "comptime fn unused(){inline for i in false..true {()}}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "accepted: {source}"
        );
    }
}

#[test]
fn large_invocation_ranges_are_limited_by_vm_execution_budget() {
    let package = support::compile(
        "comptime fn count(end:.Integer'builtin){inline for i in 0..end {()}} comptime fn run(){count(1000000);}",
    );
    let error = luna::invocation::invoke(&package, "run", 100)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("budget") || error.contains("step limit"),
        "{error}"
    );
}

#[test]
fn nested_labelled_continues_advance_only_the_target_loop() {
    assert_eq!(
        invoke(
            r#"
        comptime fn scan(count:.Integer'builtin)->.Integer'builtin {
            var sum:.Integer'builtin=0;
            inline for:outer i in 0..count {
                inline for j in 0..3 {
                    if j==1 {continue outer;}
                    sum = sum + i;
                }
            }
            sum
        }
        comptime fn run()->bool {scan(4)==6}
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn empty_ranges_skip_guards_and_pattern_computations() {
    assert_eq!(
        invoke(
            r#"
        comptime fn fail()->bool {while true {}}
        comptime fn scan(start:.Integer'builtin,end:.Integer'builtin)->bool {
            inline for (i and fail() is selected) if selected in start..end {return false;}
            true
        }
        comptime fn run()->bool {scan(2,1) and scan(0,0)}
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn exact_integer_ranges_do_not_require_machine_sized_boundaries() {
    assert_eq!(
        invoke(
            r#"
        comptime fn count(start:.Integer'builtin,end:.Integer'builtin)->.Integer'builtin {
            var count:.Integer'builtin=0;
            inline for _ in start..=end {count = count + 1;}
            count
        }
        comptime fn run()->bool {
            count(340282366920938463463374607431768211456,340282366920938463463374607431768211458)==3
        }
    "#
        ),
        Value::Bool(true)
    );
}

#[test]
fn range_boundary_returns_skip_remaining_producers_and_body() {
    for range in ["do {return true;0}..fail()", "0..do {return true;0}"] {
        assert_eq!(
            invoke(&format!(
                "comptime fn fail()->.Integer'builtin {{while true {{}}}}
             comptime fn run()->bool {{inline for i in {range} {{fail();}} false}}"
            )),
            Value::Bool(true),
            "{range}"
        );
    }
}

#[test]
fn range_guard_return_keeps_the_function_return_target() {
    assert_eq!(
        invoke(
            r#"
        comptime fn scan(count:.Integer'builtin)->bool {
            inline for i if do {if i==2 {return true;} true} in 0..count {()}
            false
        }
        comptime fn run()->bool {scan(4) and not scan(2)}
    "#
        ),
        Value::Bool(true)
    );
}
