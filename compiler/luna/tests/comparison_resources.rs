#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

fn source(body: &str) -> String {
    format!(
        "mod comparison {{\n{}\n}}
        trait Duplicate {{}} trait Retire {{fn drop(*self);}}
        impl Duplicate for bool {{}}
        struct Watch {{value:bool,dropped:*bool,called:*bool}}
        impl Retire for Watch {{fn drop(*self) {{
            assumes not self.dropped.*.*;
            self.dropped.*.* = true;
        }}}}
        impl comparison.Eq for Watch {{fn eq(self,other:Self)->bool {{
            assumes not self.dropped.* and not other.dropped.* and not self.called.*;
            self.called.* = true;
            self.value == other.value
        }}}}
        impl comparison.Ord for Watch {{fn cmp(self,other:Self)->comparison.Order {{
            assumes not self.dropped.* and not other.dropped.* and not self.called.*;
            self.called.* = true;
            if self.value == other.value {{.equal}} else if self.value {{.greater}} else {{.less}}
        }}}}
        {body}",
        support::with_attrs(include_str!("../../../library/builtin/comparison.fl"))
    )
}

fn compile(source: &str) -> nir::program::Package {
    support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("Duplicate"),
            drop: Some("Retire"),
            equality: Some("input.comparison.Eq"),
            ordering: Some("input.comparison.Ord"),
            ..Default::default()
        },
    )
    .unwrap()
}

fn verify(package: &nir::program::Package, op: &str) {
    assert_eq!(
        luna::execution::interpret_with_protocols(package, "run", "Duplicate", "Retire", 1000000)
            .unwrap_or_else(|error| panic!("{op}: {error}")),
        comptime_interpretor::Value::Bool(true)
    );
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Duplicate")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Retire")
        .unwrap()
        .declaration;
    let ir = luna::host::llvm_ir_with_protocols(package, "verify", 1000000, copy, drop).unwrap();
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
fn comparisons_consume_both_owners_once_after_method_execution() {
    for (op, expected) in [
        ("==", false),
        ("!=", true),
        ("<", false),
        ("<=", false),
        (">", true),
        (">=", true),
    ] {
        let body = format!(
            "fn run()->bool {{
            var left=false;var right=false;var called=false;
            let result=Watch {{value:true,dropped:left.ref,called:called.ref}} {op}
                Watch {{value:false,dropped:right.ref,called:called.ref}};
            result=={expected} and called and left and right
        }} fn verify() {{let valid=run();assumes valid;}}"
        );
        verify(&compile(&source(&body)), op);
    }
}

#[test]
fn diverging_comparison_arguments_retire_only_evaluated_owners() {
    for op in ["==", "!=", "<", "<=", ">", ">="] {
        let body = format!(
            "fn right_exit(dropped:*bool,called:*bool)->bool {{
            Watch {{value:true,dropped:dropped,called:called}} {op} do {{return true}}
        }}
        fn left_exit(dropped:*bool,called:*bool)->bool {{
            do {{return true}} {op} Watch {{value:false,dropped:dropped,called:called}}
        }}
        fn run()->bool {{var left=false;var right=false;var called=false;
            let a=right_exit(left.ref,called.ref);
            let b=left_exit(right.ref,called.ref);
            a and b and left and not right and not called
        }} fn verify() {{let valid=run();assumes valid;}}"
        );
        verify(&compile(&source(&body)), op);
    }
}

#[test]
fn comparison_does_not_make_affine_operands_copyable() {
    for op in ["==", "!=", "<", "<=", ">", ">="] {
        let body = format!(
            "fn run()->bool {{var left=false;var right=false;var called=false;
            let a=Watch {{value:true,dropped:left.ref,called:called.ref}};
            let b=Watch {{value:false,dropped:right.ref,called:called.ref}};
            let first=a {op} b;
            let second=a {op} b;
            first or second
        }}"
        );
        let package = compile(&source(&body));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            1000000,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("unavailable"), "{op}: {error}");
    }
}

#[test]
fn borrowed_comparison_preserves_owners_and_returned_resource() {
    let body = "
        impl comparison.Eq for *Watch {fn eq(self,other:Self)->bool {
            assumes not self.dropped.*.* and not other.dropped.*.*;
            self.value.* == other.value.*
        }}
        impl comparison.Ord for *Watch {fn cmp(self,other:Self)->comparison.Order {
            assumes not self.dropped.*.* and not other.dropped.*.*;
            if self.value.* == other.value.* {.equal}
            else if self.value.* {.greater} else {.less}
        }}
        fn precedes(a:T,b:T)->bool where T :- comparison.Ord {a < b}
        fn choose(a:T,b:T)->T where T, requires *T :- comparison.Ord {
            if precedes(a.ref,b.ref) {b} else {a}
        }
        fn scenario(first:bool,second:bool)->bool {
            var left=false;var right=false;var called=false;
            do {
                let selected=choose(
                    Watch {value:first,dropped:left.ref,called:called.ref},
                    Watch {value:second,dropped:right.ref,called:called.ref});
                assumes selected.value == (first or second);
                assumes if not first and second {left and not right} else {right and not left};
                assumes not called;
            };
            left and right
        }
        fn run()->bool {
            scenario(false,true) and scenario(true,false)
                and scenario(false,false) and scenario(true,true)
        }
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "borrowed comparison");
}

#[test]
fn indirect_comparator_transfers_selected_owner_after_callback() {
    let body = "
        fn ascending(a:*Watch,b:*Watch)->bool {
            assumes not a.dropped.*.* and not b.dropped.*.*;
            a.called.*.* = true;
            not a.value.* and b.value.*
        }
        fn descending(a:*Watch,b:*Watch)->bool {
            assumes not a.dropped.*.* and not b.dropped.*.*;
            a.called.*.* = true;
            a.value.* and not b.value.*
        }
        fn choose_by(a:T,b:T,compare:fn(*T,*T)->bool)->T where T {
            if compare(a.ref,b.ref) {b} else {a}
        }
        fn scenario(reverse:bool)->bool {
            var left=false;var right=false;var called=false;
            let compare=if reverse {descending} else {ascending};
            do {
                let selected=choose_by(
                    Watch {value:false,dropped:left.ref,called:called.ref},
                    Watch {value:true,dropped:right.ref,called:called.ref}, compare);
                assumes selected.value == not reverse;
                assumes called;
                assumes if reverse {right and not left} else {left and not right};
            };
            left and right
        }
        fn run()->bool {scenario(false) and scenario(true)}
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "indirect comparator");
}

#[test]
fn captured_comparator_retires_its_environment_after_selecting_owner() {
    let body = "
        typealias Compare=%{marker:Watch} fn(*Watch,*Watch)->bool;
        fn choose_by(a:Watch,b:Watch,compare:Compare)->Watch {
            if compare(a.ref,b.ref) {b} else {a}
        }
        fn scenario(reverse:bool)->bool {
            var left=false;var right=false;var called=false;var retired=false;
            do {
                let marker=Watch {value:reverse,dropped:retired.ref,called:called.ref};
                let compare=|a:*Watch,b:*Watch| {
                    assumes not marker.dropped.*;
                    assumes not a.dropped.*.* and not b.dropped.*.*;
                    marker.called.* = true;
                    if marker.value {a.value.* and not b.value.*}
                    else {not a.value.* and b.value.*}
                };
                let selected=choose_by(
                    Watch {value:false,dropped:left.ref,called:called.ref},
                    Watch {value:true,dropped:right.ref,called:called.ref}, compare);
                assumes selected.value == not reverse;
                assumes called and retired;
                assumes if reverse {right and not left} else {left and not right};
            };
            left and right and retired
        }
        fn run()->bool {scenario(false) and scenario(true)}
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "captured comparator");
}

#[test]
fn returned_comparator_can_be_reused_or_dropped_without_invocation() {
    let body = "
        typealias Compare=%{marker:Watch} fn(*Watch,*Watch)->bool;
        fn make(marker:Watch)->Compare {
            |a:*Watch,b:*Watch| {
                assumes not marker.dropped.*;
                assumes not a.dropped.*.* and not b.dropped.*.*;
                marker.called.* = not marker.called.*;
                if marker.value {a.value.* and not b.value.*}
                else {not a.value.* and b.value.*}
            }
        }
        fn scenario(invoke:bool)->bool {
            var left=false;var right=false;var called=false;var retired=false;
            do {
                let a=Watch {value:false,dropped:left.ref,called:called.ref};
                let b=Watch {value:true,dropped:right.ref,called:called.ref};
                do {
                    let compare=make(Watch {value:false,dropped:retired.ref,called:called.ref});
                    if invoke {
                        let first=compare(a.ref,b.ref);
                        assumes first and called and not retired;
                        let second=compare(b.ref,a.ref);
                        assumes not second and not called and not retired;
                    };
                    assumes not left and not right;
                };
                assumes retired and not called and not left and not right;
            };
            left and right and retired
        }
        fn run()->bool {scenario(false) and scenario(true)}
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "returned comparator");
}

#[test]
fn owning_comparator_parameters_preserve_nested_pointer_contracts() {
    let body = "
        typealias Compare=%{reverse:bool} fn(Watch,Watch)->bool;
        fn make(reverse:bool)->Compare {
            |a:Watch,b:Watch| {
                assumes not a.dropped.* and not b.dropped.*;
                a.called.* = true;
                if reverse {a.value and not b.value} else {not a.value and b.value}
            }
        }
        fn scenario(reverse:bool)->bool {
            var left=false;var right=false;var called=false;
            let compare=make(reverse);
            let result=compare(
                Watch {value:false,dropped:left.ref,called:called.ref},
                Watch {value:true,dropped:right.ref,called:called.ref});
            result == not reverse and called and left and right
        }
        fn run()->bool {scenario(false) and scenario(true)}
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "owning comparator");
}

#[test]
fn conditional_closure_parameter_origins_do_not_authorize_escaped_pointees() {
    let body = "
        fn run()->bool {
            var called=false;
            let marker=true;
            let consume=|value:Watch| {
                assumes not value.dropped.*;
                value.called.* = true;
                marker
            };
            let value=do {
                var retired=false;
                Watch {value:false,dropped:retired.ref,called:called.ref}
            };
            consume(value)
        }
    ";
    let package = compile(&source(body));
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 1000000)
            .unwrap_err()
            .to_string();
    assert!(
        error.contains("pointer") || error.contains("provenance"),
        "{error}"
    );
}

#[test]
fn enum_closure_calls_use_distinct_native_cleanup_plans() {
    let body = "
        enum Choice {none,some(Watch)}
        fn run()->bool {
            var retired=false;var called=false;
            let enabled=true;
            let consume=|value:Choice| value match {
                .none=>false,
                .some(item)=>do {
                    assumes not item.dropped.*;
                    item.called.* = true;
                    enabled and item.value
                },
            };
            let empty=consume(Choice.none);
            assumes not empty and not called and not retired;
            let full=consume(Choice.some(Watch {value:true,dropped:retired.ref,called:called.ref}));
            full and called and retired
        }
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "distinct enum cleanup plans");
}

#[test]
fn owning_closure_enum_parameter_selects_only_active_pointer_fields() {
    let body = "
        enum Choice {none,some(Watch)}
        fn scenario(full:bool)->bool {
            var retired=false;var called=false;
            let enabled=true;
            let consume=|value:Choice| value match {
                .none=>false,
                .some(item)=>do {
                    assumes not item.dropped.*;
                    item.called.* = true;
                    enabled and item.value
                },
            };
            let argument=if full {
                Choice.some(Watch {value:true,dropped:retired.ref,called:called.ref})
            } else {Choice.none};
            let result=consume(argument);
            result == full and called == full and retired == full
        }
        fn run()->bool {scenario(false) and scenario(true)}
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "dynamic enum owned parameter");
}

#[test]
fn dynamic_closure_identity_selects_its_own_cleanup_specializations() {
    let body = "
        enum Choice {none,some(Watch)}
        fn scenario(select:bool)->bool {
            var retired=false;var called=false;
            let enabled=true;
            let first=|value:Choice| value match {
                .none=>false,
                .some(item)=>do {item.called.* = true; enabled and item.value},
            };
            let second=|value:Choice| value match {
                .none=>false,
                .some(item)=>do {item.called.* = true; not enabled or not item.value},
            };
            let consume=if select {first} else {second};
            let empty=consume(Choice.none);
            assumes not empty and not called and not retired;
            let full=consume(Choice.some(Watch {value:true,dropped:retired.ref,called:called.ref}));
            full == select and called and retired
        }
        fn run()->bool {scenario(false) and scenario(true)}
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "dynamic closure cleanup identity");
}

#[test]
fn named_bodies_specialize_nested_closure_dispatch() {
    let body = "
        enum Choice {none,some(Watch)}
        typealias Compare=%{enabled:bool} fn(Choice)->bool;
        fn invoke(compare:Compare,value:Choice)->bool {compare(value)}
        fn make(enabled:bool)->Compare {
            |value:Choice| value match {
                .none=>false,
                .some(item)=>do {item.called.* = true;enabled and item.value},
            }
        }
        fn run()->bool {
            var retired=false;var called=false;
            let empty=invoke(make(true),Choice.none);
            let full=invoke(make(true),Choice.some(Watch {value:true,dropped:retired.ref,called:called.ref}));
            not empty and full and called and retired
        }
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(
        &compile(&source(body)),
        "named closure dispatch specialization",
    );
}

#[test]
fn recursive_named_calls_keep_each_variant_cleanup_context() {
    let body = "
        enum Choice {none,some(Watch)}
        fn consume(value:Choice,again:bool)->bool {
            if again {consume(value,false)} else {value match {
                .none=>false,
                .some(item)=>do {
                    assumes not item.dropped.*;
                    item.called.* = true;
                    item.value
                },
            }}
        }
        fn run()->bool {
            var retired=false;var called=false;
            let empty=consume(Choice.none,true);
            assumes not empty and not called and not retired;
            let full=consume(Choice.some(Watch {value:true,dropped:retired.ref,called:called.ref}),true);
            full and called and retired
        }
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "recursive named specialization");
}

#[test]
fn dynamic_named_identity_routes_variant_specific_cleanup() {
    let body = "
        enum Choice {none,some(Watch)}
        typealias Consume=fn(Choice)->bool;
        impl Duplicate for Consume {}
        fn first(value:Choice)->bool {value match {
            .none=>false,
            .some(item)=>do {item.called.* = true;item.value},
        }}
        fn second(value:Choice)->bool {value match {
            .none=>false,
            .some(item)=>do {item.called.* = true;not item.value},
        }}
        fn scenario(select:bool)->bool {
            var retired=false;var called=false;
            let consume=if select {first} else {second};
            let empty=consume(Choice.none);
            assumes not empty and not called and not retired;
            let full=consume(Choice.some(Watch {value:true,dropped:retired.ref,called:called.ref}));
            full == select and called and retired
        }
        fn run()->bool {scenario(false) and scenario(true)}
        fn verify() {let valid=run();assumes valid;}
    ";
    verify(&compile(&source(body)), "dynamic named specialization");
}
