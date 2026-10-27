//! The actual inline iterator protocol and its joint expansion with consumers.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn ordinary_for_retires_rejected_current_and_unproduced_items_once() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool,keep:bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var first=false;var second=false;var third=false;
            var calls:.u32'builtin=0;
            let counter=calls.ref;
            let a:?Watch=Watch {{seen:first.ref,keep:false}};
            let b:?Watch=Watch {{seen:second.ref,keep:true}};
            let c:?Watch=Watch {{seen:third.ref,keep:true}};
            for item if item.keep in a.as(Iterator).chain(b).as(Iterator).chain(c).as(Iterator)
                .inspect(|value| {{assumes counter.* < 3; counter.* = counter.* + 1;}}) {{
                assumes first and not second and not third;
                break;
            }}
            first and second and third and calls == 2
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    check_iteration_with_budget(
        &source,
        Some("handler lexical capture address exposure requires a memory contract"),
        4_000_000,
    );
}

#[test]
fn ordinary_for_evaluates_input_once_before_installing_delivery_handler() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var inputs:.u32'builtin=0;
            var outputs:.u32'builtin=0;
            do {{
                for item in (do {{
                    emit_item<bool>(false)#;
                    let value:?bool=true;
                    value
                }}) {{
                    assumes item and inputs == 1;
                    assumes outputs < 2; outputs = outputs + 1;
                }}
            }}# {{emit_item<bool>(item) => {{
                assumes not item;
                assumes inputs < 2; inputs = inputs + 1;
            }}}};
            inputs == 1 and outputs == 1
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    check_iteration(&source);
}

#[test]
fn ordinary_for_uses_symbolic_iterator_evidence_in_generic_bodies() {
    let source = format!(
        "{}\n{}\n
        inline fn visits(items:I)->.u32'builtin where I:-Iterator {{
            var count:.u32'builtin=0;
            for item in items {{assumes count < 3; count = count + 1;}}
            count
        }}
        fn run()->bool {{
            let first:?bool=false;let second:?bool=true;
            visits(first.as(Iterator).chain(second)) == 2
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    check_iteration(&source);
}

#[test]
fn ordinary_for_consumes_optional_chain_and_skips_failed_patterns() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var count:.u32'builtin=0;
            let first:?bool=false;
            let second:?bool=true;
            for true in first.as(Iterator).chain(second) {{assumes count < 3; count = count + 1;}}
            let empty:?bool=null;
            for value in empty {{assumes false;}}
            count == 1
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    check_iteration(&source);
}

#[test]
fn ordinary_for_guards_continue_break_and_return_keep_lexical_targets() {
    let source = format!(
        "{}\n{}\n
        fn stop()->bool {{
            let value:?bool=true;
            for item in value {{return item;}}
            false
        }}
        fn run()->bool {{
            var seen:.u32'builtin=0;
            var delivered:.u32'builtin=0;
            let first:?bool=false;
            let second:?bool=true;
            for item if true in first.as(Iterator).chain(second) {{
                assumes seen < 3; seen = seen + 1;
                if not item {{continue;}}
                assumes delivered < 2; delivered = delivered + 1;
                break;
            }}
            seen == 2 and delivered == 1 and stop()
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    check_iteration(&source);
}

#[test]
fn ordinary_for_nested_labelled_transfers_leave_only_the_target_iteration() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var count:.u32'builtin=0;
            let a:?bool=false;
            let b:?bool=true;
            for:outer item in a.as(Iterator).chain(b) {{
                let inner:?bool=true;
                for value in inner {{assumes count < 3; count = count + 1; continue outer;}}
                assumes false;
            }}
            let final_value:?bool=true;
            for:exit item in final_value {{
                let inner:?bool=true;
                for value in inner {{break exit;}}
                assumes false;
            }}
            count == 2
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    check_iteration(&source);
}

#[test]
fn repository_iterator_declarations_form_without_resource_implementations() {
    compile_iteration_unbound(include_str!("../../../library/builtin/iterator.fl")).unwrap();
}

#[test]
fn associated_item_inherits_implementation_parameter_constraints() {
    let source = "trait Source {assoc Item:(.Type'builtin);}
        trait Forward {assoc Item:(.Type'builtin);}
        struct Wrap where I {value:I}
        impl Source for bool {assoc Item:(.Type'builtin)=bool;}
        impl Forward for Wrap<I> where I:-Source {
            assoc Item:(.Type'builtin)=I.as(Source).Item;
        }
        fn run()->Wrap<bool>.as(Forward).Item {true}";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let error = support::compile_with_protocols(
        &source.replace("where I:-Source", "where I"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("given interface constraint"), "{error}");
}

#[test]
fn repository_inline_iterator_maps_filters_and_consumes_in_order() {
    let source = format!(
        "{}\n{}\n
        typealias Unit=.void'builtin;
        struct Flags {{}}
        impl Iterator for Flags {{
            assoc Item:(.Type'builtin)=bool;
            inline fn iterate(self)-> #emit_item<bool> Unit {{
                emit_item<bool>(false)#;emit_item<bool>(true)#;
            }}
        }}
        fn run()->bool {{
            var observed=false;
            var count:.u32'builtin=0;
            let target=observed.ref;
            let counter=count.ref;
            Flags {{}}.as(Iterator).map(|value| {{
                assumes counter.* == if value {{1}} else {{0}};
                counter.* = counter.* + 1;
                not value
            }}).as(Iterator)
                .filter(|value| value.*).as(Iterator).foreach(|value| {{
                    assumes not target.*;
                    target.* = value;
                }});
            observed and count == 2
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn repository_inline_iterator_drops_filtered_and_unproduced_items() {
    let source = format!(
        "{}\n{}\n
        typealias Unit=.void'builtin;
        struct Watch {{seen:*bool,keep:bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        struct Pair {{first:Watch,second:Watch}}
        impl Iterator for Pair {{
            assoc Item:(.Type'builtin)=Watch;
            inline fn iterate(self)->#emit_item<Watch> Unit {{
                emit_item<Watch>(self.first)#;
                emit_item<Watch>(self.second)#;
            }}
        }}
        fn filtered()->bool {{
            var first=false;var second=false;var called=false;
            let first_seen=first.ref;let consumed=called.ref;
            Pair {{first:Watch {{seen:first.ref,keep:false}},second:Watch {{seen:second.ref,keep:true}}}}
                .as(Iterator).filter(|value| value.keep.*).as(Iterator)
                .foreach(|value| {{
                    assumes first_seen.* and not value.seen.* and not consumed.*;
                    consumed.* = true;
                }});
            first and second and called
        }}
        fn short_circuit(use_any:bool)->bool {{
            var first=false;var second=false;var called=false;
            let observed=called.ref;
            let pair=Pair {{first:Watch {{seen:first.ref,keep:true}},second:Watch {{seen:second.ref,keep:false}}}};
            let result=if use_any {{
                pair.as(Iterator).any(|value| {{
                    assumes not observed.*;
                    observed.* = true;
                    value.keep.*
                }})
            }} else {{
                pair.as(Iterator).all(|value| {{
                    assumes not observed.*;
                    observed.* = true;
                    not value.keep.*
                }})
            }};
            result == use_any and first and second and called
        }}
        fn run()->bool {{filtered() and short_circuit(true) and short_circuit(false)}}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn repository_iterator_rejects_repeated_consumption_of_callback_capture() {
    for adapter in [
        "map(|flag| ticket).as(Iterator).foreach(|item| {})",
        "fold(false, |state, flag| {let consumed=ticket;state})",
        "filter_map<Ticket>(|flag| ticket).as(Iterator).foreach(|item| {})",
        "flat_map<?Ticket>(|flag| ticket).as(Iterator).foreach(|item| {})",
        "take_while(|flag| {let consumed=ticket;true}).as(Iterator).foreach(|item| {})",
        "skip_while(|flag| {let consumed=ticket;true}).as(Iterator).foreach(|item| {})",
    ] {
        let source = format!(
            "{}\n{}\n
        typealias Unit=.void'builtin;
        struct Ticket {{}}
        struct Flags {{}}
        impl Iterator for Flags {{
            assoc Item:(.Type'builtin)=bool;
            inline fn iterate(self)->#emit_item<bool> Unit {{
                emit_item<bool>(false)#;emit_item<bool>(true)#;
            }}
        }}
        fn run() {{
            let ticket=Ticket {{}};
            Flags {{}}.as(Iterator).{adapter};
        }}",
            include_str!("../../../library/builtin/iterator.fl"),
            include_str!("../../../library/builtin/resources.fl"),
        );
        let package = compile_iteration(&source);
        let mut expanded = package.clone();
        elaboration::program::inlining::expand(&mut expanded, &mut 1_000_000).unwrap();
        for package in [&expanded, &package] {
            let error = luna::execution::interpret_with_protocols(
                package, "run", "Copy", "Drop", 1_000_000,
            )
            .unwrap_err();
            assert!(error.contains("move-error"), "{error}");
            assert!(
                error.contains("unavailable value")
                    || error.contains("callable environment must remain complete"),
                "{error}"
            );
        }
    }
}

#[test]
fn optional_iteration_preserves_layers_and_chain_order() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var count:.u32'builtin=0;
            let observed=count.ref;
            let empty:?bool=null;
            let first:?bool=false;
            let second:?bool=true;
            empty.as(Iterator).chain(first).as(Iterator).chain(second).as(Iterator)
                .foreach(|value| {{
                    assumes observed.* == if value {{1}} else {{0}};
                    observed.* = observed.* + 1;
                }});
            let inner:?bool=null;
            let outer:??bool=inner;
            outer.as(Iterator).foreach(|value| {{
                let was_absent=value match {{_?=>false,null=>true}};
                assumes was_absent;
                assumes observed.* < 4294967295;
                observed.* = observed.* + 1;
            }});
            let absent:?bool=null;
            let none=absent.as(Iterator).any(|value| {{assumes false;value.*}});
            let absent:?bool=null;
            let every=absent.as(Iterator).all(|value| {{assumes false;value.*}});
            count == 3 and not none and every
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn optional_chain_cleans_owned_items_on_short_circuit() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var first=false;var second=false;var called=false;
            let observed=called.ref;
            let left:?Watch=Watch {{seen:first.ref}};
            let right:?Watch=Watch {{seen:second.ref}};
            let found=left.as(Iterator).chain(right).as(Iterator).any(|value| {{
                assumes not observed.* and not value.seen.*.*;
                observed.* = true;
                true
            }});
            found and called and first and second
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn collect_selects_the_requested_from_iterator_implementation() {
    let source = format!(
        "{}\n{}\n
        struct Count {{value:.u32'builtin}}
        impl FromIterator<bool> for Count {{
            inline fn from_iter(iter:I)->Self
                where I:-Iterator, requires I.as(Iterator).Item == bool {{
                var count:.u32'builtin=0;
                let observed=count.ref;
                iter.as(Iterator).foreach(|value| {{
                    if value {{
                        assumes observed.* < 4294967295;
                        observed.* = observed.* + 1;
                    }}
                }});
                Count {{value:count}}
            }}
        }}
        fn run()->bool {{
            let first:?bool=true;let second:?bool=false;
            let result:Count=first.as(Iterator).chain(second).as(Iterator).collect();
            let empty:?bool=null;
            let zero:Count=empty.as(Iterator).collect();
            result.value == 1 and zero.value == 0
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn chain_rejects_different_item_types() {
    let source = format!(
        "{}\nfn run() {{let left:?bool=true;let right:?.u8'builtin=1;
        left.as(Iterator).chain(right);}}",
        include_str!("../../../library/builtin/iterator.fl"),
    );
    assert!(compile_iteration_unbound(&source).is_err());
}

fn compile_iteration(source: &str) -> nir::program::Package {
    support::compile_with_source_bindings_for_target(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            iterator: Some("input.Iterator"),
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
        Some(luna::frontend::native_test_target().expect("host target available")),
    )
    .unwrap()
}

fn compile_iteration_unbound(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings_for_target(
        source,
        &Default::default(),
        Default::default(),
        Some(luna::frontend::native_test_target().expect("host target available")),
    )
}

#[test]
fn inspection_observes_items_before_filtering_without_changing_them() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var count:.u32'builtin=0;
            var delivered=false;
            let counter=count.ref;
            let output=delivered.ref;
            let first:?bool=false;
            let second:?bool=true;
            first.as(Iterator).chain(second).as(Iterator)
                .inspect(|value| {{
                    assumes counter.* == if value.* {{1}} else {{0}};
                    counter.* = counter.* + 1;
                }}).as(Iterator).filter(|value| value.*).as(Iterator)
                .foreach(|value| {{
                    assumes counter.* == 2 and not output.*;
                    output.* = value;
                }});
            let empty:?bool=null;
            empty.as(Iterator).inspect(|value| {{assumes false;}})
                .as(Iterator).foreach(|value| {{assumes false;}});
            count == 2 and delivered
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn inspection_borrows_affine_items_and_stops_with_the_consumer() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var first=false;var second=false;var inspected=false;
            let observed=inspected.ref;
            let left:?Watch=Watch {{seen:first.ref}};
            let right:?Watch=Watch {{seen:second.ref}};
            let found=left.as(Iterator).chain(right).as(Iterator)
                .inspect(|value| {{
                    assumes not observed.* and not value.seen.*.*;
                    observed.* = true;
                }}).as(Iterator).any(|value| {{
                    assumes observed.* and not value.seen.*.*;
                    true
                }});
            found and inspected and first and second
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn fold_moves_and_reinitializes_affine_accumulator_in_source_order() {
    let source = format!(
        "{}\n{}\n
        struct State {{count:.u32'builtin,seen:*bool}}
        impl Drop for State {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var dropped=false;
            var empty_dropped=false;
            let first:?bool=false;
            let second:?bool=true;
            let state=first.as(Iterator).chain(second).as(Iterator).fold(
                State {{count:0,seen:dropped.ref}}, |state, value| {{
                    assumes state.count == if value {{1}} else {{0}};
                    var next=state;
                    next.count = next.count + 1;
                    next
                }});
            let valid=state.count == 2 and not dropped;
            {{let retired=state;}}
            let empty:?bool=null;
            let unchanged=empty.as(Iterator).fold(
                State {{count:7,seen:empty_dropped.ref}}, |state, value| {{
                    assumes false;
                    state
                }});
            let empty_valid=unchanged.count == 7 and not empty_dropped;
            {{let retired=unchanged;}}
            valid and empty_valid and dropped and empty_dropped
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_contract_limit(
        &source,
        Some("borrowed handler data capture must remain complete on every exit"),
    );
}

#[test]
fn handler_borrows_pointer_bearing_affine_capture_without_copy() {
    let source = format!(
        "{}\n{}\n
        struct Observer {{seen:*bool,count:.u32'builtin}}
        impl Drop for Observer {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var dropped=false;
            var valid=false;
            {{
                var observer=Observer {{seen:dropped.ref,count:0}};
                let first:?bool=false;
                let second:?bool=true;
                first.as(Iterator).chain(second).as(Iterator).iterate()# {{
                    emit_item<bool>(value)=>{{
                        assumes not observer.seen.*;
                        assumes observer.count == if value {{1}} else {{0}};
                        observer.count = observer.count + 1;
                    }},
                }};
                valid = observer.count == 2 and not dropped;
            }}
            valid and dropped
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn borrowed_pointer_capture_retains_owner_and_storage_restrictions() {
    for body in [
        "let consumed=observer;",
        "observer.seen = replacement;",
        "let alias=observer.ref;",
    ] {
        let source = format!(
            "{}\n{}\n
            struct Observer {{seen:*bool}}
            fn run() {{
                var first=false;var second=false;
                var observer=Observer {{seen:first.ref}};
                let replacement=second.ref;
                let item:?bool=true;
                item.as(Iterator).iterate()# {{emit_item<bool>(value)=>{{{body}}},}};
            }}",
            include_str!("../../../library/builtin/iterator.fl"),
            include_str!("../../../library/builtin/resources.fl"),
        );
        let package = compile_iteration(&source);
        let error =
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
                .unwrap_err();
        assert!(
            error.contains("must remain complete")
                || error.contains("rebinding")
                || error.contains("address")
                || error.contains("unavailable value"),
            "{body}: {error}"
        );
    }
}

#[test]
fn borrowed_affine_capture_cleanup_follows_lexical_return_and_loop_exit() {
    let source = format!(
        "{}\n{}\n
        struct Observer {{seen:*bool,count:.u32'builtin}}
        impl Drop for Observer {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn visit(seen:*bool,early:bool)->.u32'builtin {{
            var observer=Observer {{seen:seen,count:0}};
            let first:?bool=false;
            let second:?bool=true;
            first.as(Iterator).chain(second).as(Iterator).iterate()# {{
                emit_item<bool>(value)=>{{
                    assumes not observer.seen.*;
                    assumes observer.count == if value {{1}} else {{0}};
                    observer.count = observer.count + 1;
                    if early {{return observer.count;}}
                }},
            }};
            observer.count
        }}
        fn broken(seen:*bool)->.u32'builtin {{
            var observer=Observer {{seen:seen,count:0}};
            while true {{
                let item:?bool=true;
                item.as(Iterator).iterate()# {{emit_item<bool>(value)=>{{
                    assumes observer.count == 0 and not observer.seen.*;
                    observer.count = 1;
                    break;
                }},}};
                assumes false;
            }}
            observer.count
        }}
        fn run()->bool {{
            var early=false;var full=false;var exited=false;
            let one=visit(early.ref,true);
            let two=visit(full.ref,false);
            let three=broken(exited.ref);
            early and full and exited and one == 1 and two == 2 and three == 1
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn find_transfers_the_match_and_cleans_rejected_and_unproduced_items() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool,keep:bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var first=false;var second=false;var third=false;
            var count:.u32'builtin=0;
            let observed=count.ref;
            let a:?Watch=Watch {{seen:first.ref,keep:false}};
            let b:?Watch=Watch {{seen:second.ref,keep:true}};
            let c:?Watch=Watch {{seen:third.ref,keep:true}};
            let found=a.as(Iterator).chain(b).as(Iterator).chain(c).as(Iterator)
                .find(|value| {{
                    assumes observed.* < 2 and not value.seen.*.*;
                    observed.* = observed.* + 1;
                    value.keep.*
                }});
            let valid=found match {{
                value?=>first and not second and third and value.keep and count == 2,
                null=>false,
            }};
            let empty:?bool=null;
            let absent=empty.as(Iterator).find(|value| {{assumes false;true}});
            let none=absent match {{_?=>false,null=>true}};
            valid and second and none
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    // Three owned optional sources and their cleanup guards share this budget.
    // Unexpanded lexical delivery of pointer-bearing results still lacks a
    // caller-relative provenance summary; expansion makes that transfer local.
    check_iteration_with_budget(
        &source,
        Some("local pointer slot origin is unknown"),
        4_000_000,
    );
}

#[test]
fn find_map_preserves_present_null_and_stops_mapping() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var calls:.u32'builtin=0;
            let counter=calls.ref;
            let a:?bool=false;let b:?bool=true;let c:?bool=false;
            let found:??bool=a.as(Iterator).chain(b).as(Iterator).chain(c)
                .as(Iterator).find_map<?bool>(|value| {{
                    assumes counter.* == if value {{1}} else {{0}};
                    counter.* = counter.* + 1;
                    if value {{let inner:?bool=null;return inner;}}
                    null
                }});
            let present_null=found match {{
                inner?=>inner match {{_?=>false,null=>true}},
                null=>false,
            }};
            let empty:?bool=null;
            let absent:?bool=empty.as(Iterator).find_map(|value| {{assumes false;null}});
            let none=absent match {{_?=>false,null=>true}};
            present_null and calls == 2 and none
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn find_scalar_handles_match_and_exhaustion_without_expansion() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            let first:?bool=false;let second:?bool=true;
            let found=first.as(Iterator).chain(second).as(Iterator).find(|value| value.*);
            let matched=found match {{value?=>value,null=>false}};
            let rejected:?bool=false;
            let missing=rejected.as(Iterator).find(|value| value.*);
            let exhausted=missing match {{_?=>false,null=>true}};
            matched and exhausted
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn filter_map_preserves_nested_optionals_and_maps_in_order() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var calls:.u32'builtin=0;var outputs:.u32'builtin=0;
            let counter=calls.ref;let output=outputs.ref;
            let first:?bool=false;let second:?bool=true;
            first.as(Iterator).chain(second).as(Iterator)
                .filter_map<?bool>(|value| {{
                    assumes counter.* == if value {{1}} else {{0}};
                    counter.* = counter.* + 1;
                    if value {{let inner:?bool=null;return inner;}}
                    null
                }}).as(Iterator).foreach(|value| {{
                    let absent=value match {{_?=>false,null=>true}};
                    assumes absent and counter.* == 2 and output.* == 0;
                    output.* = 1;
                }});
            let empty:?bool=null;
            empty.as(Iterator).filter_map<bool>(|value| {{assumes false;null}})
                .as(Iterator).foreach(|value| {{assumes false;}});
            calls == 2 and outputs == 1
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration(&source);
}

#[test]
fn filter_map_cleans_rejected_items_and_unused_callback_environment() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool,keep:bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var first=false;var second=false;var environment=false;
            var accepted=false;
            let output=accepted.ref;
            let a:?Watch=Watch {{seen:first.ref,keep:false}};
            let b:?Watch=Watch {{seen:second.ref,keep:true}};
            a.as(Iterator).chain(b).as(Iterator).filter_map<Watch>(|value| {{
                if value.keep {{return value;}}
                null
            }}).as(Iterator).foreach(|value| {{
                assumes not value.seen.* and not output.*;
                output.* = value.keep;
            }});
            {{
                let guard=Watch {{seen:environment.ref,keep:true}};
                let empty:?bool=null;
                empty.as(Iterator).filter_map<bool>(|value| {{
                    assumes false;
                    guard.keep
                }}).as(Iterator).foreach(|value| {{assumes false;}});
            }}
            accepted and first and second and environment
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn filter_map_short_circuit_cleans_remaining_inputs_without_mapping_them() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool,keep:bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var first=false;var second=false;var third=false;
            var calls:.u32'builtin=0;
            let counter=calls.ref;
            let a:?Watch=Watch {{seen:first.ref,keep:false}};
            let b:?Watch=Watch {{seen:second.ref,keep:true}};
            let c:?Watch=Watch {{seen:third.ref,keep:true}};
            let found=a.as(Iterator).chain(b).as(Iterator).chain(c).as(Iterator)
                .filter_map<Watch>(|value| {{
                    assumes counter.* < 2 and not value.seen.*;
                    counter.* = counter.* + 1;
                    if value.keep {{return value;}}
                    null
                }}).as(Iterator).any(|value| value.keep.*);
            found and first and second and third and calls == 2
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn flat_map_finishes_inner_sources_before_advancing_outer_source() {
    let source = format!(
        "{}\n{}\n
        typealias PairUnit=.void'builtin;
        struct Pair {{first:bool,second:bool}}
        impl Iterator for Pair {{
            assoc Item:(.Type'builtin)=bool;
            inline fn iterate(self)->#emit_item<bool> PairUnit {{
                emit_item<bool>(self.first)#;emit_item<bool>(self.second)#;
            }}
        }}
        fn run()->bool {{
            var count:.u32'builtin=0;
            let observed=count.ref;
            let a:?bool=false;let b:?bool=true;
            a.as(Iterator).chain(b).as(Iterator).flat_map(|value| {{
                assumes observed.* == if value {{2}} else {{0}};
                Pair {{first:value,second:not value}}
            }}).as(Iterator).foreach(|value| {{
                assumes observed.* < 4;
                assumes value == (observed.* == 1 or observed.* == 2);
                observed.* = observed.* + 1;
            }});
            let empty:?bool=null;
            empty.as(Iterator).flat_map(|value| {{assumes false;Pair {{first:value,second:value}}}})
                .as(Iterator).foreach(|value| {{assumes false;}});
            count == 4
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn flat_map_short_circuit_cleans_inner_and_outer_remainders() {
    let source = format!(
        "{}\n{}\n
        typealias GroupUnit=.void'builtin;
        struct Watch {{seen:*bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        struct Group {{first:Watch,second:Watch}}
        impl Iterator for Group {{
            assoc Item:(.Type'builtin)=Watch;
            inline fn iterate(self)->#emit_item<Watch> GroupUnit {{
                emit_item<Watch>(self.first)#;emit_item<Watch>(self.second)#;
            }}
        }}
        fn run()->bool {{
            var a=false;var b=false;var c=false;var d=false;
            var mapped=false;
            let observed=mapped.ref;
            let first:?Group=Group {{first:Watch {{seen:a.ref}},second:Watch {{seen:b.ref}}}};
            let second:?Group=Group {{first:Watch {{seen:c.ref}},second:Watch {{seen:d.ref}}}};
            let found=first.as(Iterator).chain(second).as(Iterator).flat_map(|group| {{
                assumes not observed.*;
                observed.* = true;
                group
            }}).as(Iterator).any(|value| {{
                assumes not value.seen.*.*;
                true
            }});
            found and mapped and a and b and c and d
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn flat_map_rejects_mapper_results_without_iterator_protocol() {
    let source = format!(
        "{}\nfn run() {{let value:?bool=true;value.as(Iterator).flat_map(|item| item);}}",
        include_str!("../../../library/builtin/iterator.fl"),
    );
    assert!(compile_iteration_unbound(&source).is_err());
}

#[test]
fn take_while_stops_its_source_but_not_the_enclosing_chain() {
    let source = format!(
        "{}\n{}\n
        fn run()->bool {{
            var calls:.u32'builtin=0;var outputs:.u32'builtin=0;
            let counter=calls.ref;let output=outputs.ref;
            let a:?bool=true;let b:?bool=false;let c:?bool=true;
            let tail:?bool=false;
            a.as(Iterator).chain(b).as(Iterator).chain(c).as(Iterator)
                .take_while(|value| {{
                    assumes counter.* == if value.* {{0}} else {{1}};
                    counter.* = counter.* + 1;
                    value.*
                }}).as(Iterator).chain(tail).as(Iterator).foreach(|value| {{
                    assumes output.* == if value {{0}} else {{1}};
                    output.* = output.* + 1;
                }});
            calls == 2 and outputs == 2
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn take_while_rejection_cleans_current_remaining_and_predicate_resources() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool,keep:bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var first=false;var second=false;var third=false;var environment=false;
            var calls:.u32'builtin=0;var delivered=false;
            let counter=calls.ref;let output=delivered.ref;
            let guard=Watch {{seen:environment.ref,keep:true}};
            let a:?Watch=Watch {{seen:first.ref,keep:true}};
            let b:?Watch=Watch {{seen:second.ref,keep:false}};
            let c:?Watch=Watch {{seen:third.ref,keep:true}};
            a.as(Iterator).chain(b).as(Iterator).chain(c).as(Iterator)
                .take_while(|value| {{
                    assumes counter.* < 2 and not guard.seen.*;
                    counter.* = counter.* + 1;
                    value.keep.*
                }}).as(Iterator).foreach(|value| {{
                    assumes not output.* and not value.seen.*;
                    output.* = value.keep;
                }});
            first and second and third and environment and delivered and calls == 2
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn take_while_handles_empty_sources_and_downstream_early_exit() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool,keep:bool}}
        impl Drop for Watch {{fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }}}}
        fn run()->bool {{
            var environment=false;var first=false;var second=false;var called=false;
            let observed=called.ref;
            let guard=Watch {{seen:environment.ref,keep:true}};
            let empty:?bool=null;
            empty.as(Iterator).take_while(|value| {{assumes false;guard.keep and value.*}})
                .as(Iterator).foreach(|value| {{assumes false;}});
            let a:?Watch=Watch {{seen:first.ref,keep:true}};
            let b:?Watch=Watch {{seen:second.ref,keep:true}};
            let found=a.as(Iterator).chain(b).as(Iterator).take_while(|value| {{
                assumes not observed.*;
                observed.* = true;
                value.keep.*
            }}).as(Iterator).any(|value| value.keep.*);
            found and called and first and second and environment
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn skip_while_stops_testing_after_the_first_forwarded_item() {
    let source = format!(
        "{}\n{}\n
        fn run() -> bool {{
            var calls: .u32'builtin = 0;
            var outputs: .u32'builtin = 0;
            let counter = calls.ref;
            let output = outputs.ref;
            let first: ?bool = true;
            let second: ?bool = false;
            let third: ?bool = true;
            first.as(Iterator).chain(second).as(Iterator).chain(third).as(Iterator)
                .skip_while(|value| {{
                    assumes counter.* == if value.* {{ 0 }} else {{ 1 }};
                    counter.* = counter.* + 1;
                    value.*
                }}).as(Iterator).foreach(|value| {{
                    assumes output.* == if value {{ 1 }} else {{ 0 }};
                    output.* = output.* + 1;
                }});
            calls == 2 and outputs == 2
        }}
        fn verify() {{ let passed = run(); assumes passed; }}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn skip_while_cleans_prefix_tail_and_environment_on_consumer_exit() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{ seen: *bool, skip: bool }}
        impl Drop for Watch {{ fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }} }}
        fn run() -> bool {{
            var first = false; var second = false; var third = false;
            var environment = false; var calls: .u32'builtin = 0;
            let counter = calls.ref;
            let retired_prefix = first.ref;
            let guard = Watch {{ seen: environment.ref, skip: true }};
            let a: ?Watch = Watch {{ seen: first.ref, skip: true }};
            let b: ?Watch = Watch {{ seen: second.ref, skip: false }};
            let c: ?Watch = Watch {{ seen: third.ref, skip: true }};
            let found = a.as(Iterator).chain(b).as(Iterator).chain(c).as(Iterator)
                .skip_while(|value| {{
                    assumes counter.* < 2 and not guard.seen.*;
                    counter.* = counter.* + 1;
                    value.skip.*
                }}).as(Iterator).any(|value| {{
                    assumes retired_prefix.* and not value.seen.*.*;
                    not value.skip.*
                }});
            found and first and second and third and environment and calls == 2
        }}
        fn verify() {{ let passed = run(); assumes passed; }}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn skip_while_handles_empty_and_fully_skipped_sources() {
    let source = format!(
        "{}\n{}\n
        struct Guard {{ seen: *bool }}
        impl Drop for Guard {{ fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }} }}
        fn run() -> bool {{
            var retired = false;
            let guard = Guard {{ seen: retired.ref }};
            let empty: ?bool = null;
            empty.as(Iterator).skip_while(|value| {{
                assumes false;
                value.* and guard.seen.*
            }}).as(Iterator).foreach(|value| {{ assumes false; }});
            var calls: .u32'builtin = 0;
            let counter = calls.ref;
            let a: ?bool = true; let b: ?bool = true;
            let tail: ?bool = false;
            let found = a.as(Iterator).chain(b).as(Iterator).skip_while(|value| {{
                assumes counter.* < 2;
                counter.* = counter.* + 1;
                value.*
            }}).as(Iterator).chain(tail).as(Iterator).any(|value| not value.*);
            retired and calls == 2 and found
        }}
        fn verify() {{ let passed = run(); assumes passed; }}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn counted_adapters_preserve_order_and_do_not_overproduce() {
    let source = format!(
        "{}\n{}\n
        fn run() -> bool {{
            var calls: .u32'builtin = 0;
            let counter = calls.ref;
            let first: ?bool = false;
            let second: ?bool = true;
            let third: ?bool = false;
            let result = first.as(Iterator).chain(second).as(Iterator).chain(third)
                .as(Iterator).inspect(|value| {{
                    assumes counter.* < 2;
                    counter.* = counter.* + 1;
                }}).as(Iterator).skip(1).as(Iterator).take(1)
                .as(Iterator).all(|value| value.*);
            result and calls == 2
        }}
        fn verify() {{ let passed = run(); assumes passed; }}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn counted_adapters_handle_zero_and_exhausted_limits() {
    let source = format!(
        "{}\n{}\n
        typealias Unit = .void'builtin;
        struct NeverStart {{ retired: *bool }}
        impl Drop for NeverStart {{ fn drop(*self) {{
            assumes not self.retired.*.*;
            self.retired.*.* = true;
        }} }}
        impl Iterator for NeverStart {{
            assoc Item: (.Type'builtin) = bool;
            inline fn iterate(self) -> #emit_item<bool> Unit {{ assumes false; }}
        }}
        fn run() -> bool {{
            var retired = false;
            NeverStart {{ retired: retired.ref }}.as(Iterator).take(0)
                .as(Iterator).foreach(|value| {{ assumes false; }});
            let absent: ?bool = null;
            absent.as(Iterator).take(10).as(Iterator).foreach(|value| {{ assumes false; }});
            let first: ?bool = true;
            let skipped = first.as(Iterator).skip(10).as(Iterator).any(|value| {{
                assumes false; value.*
            }});
            let second: ?bool = true;
            let forwarded = second.as(Iterator).skip(0).as(Iterator).take(10)
                .as(Iterator).any(|value| value.*);
            retired and not skipped and forwarded
        }}
        fn verify() {{ let passed = run(); assumes passed; }}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

#[test]
fn counted_adapters_retire_skipped_delivered_and_remaining_items_once() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{ seen: *bool }}
        impl Drop for Watch {{ fn drop(*self) {{
            assumes not self.seen.*.*;
            self.seen.*.* = true;
        }} }}
        fn run() -> bool {{
            var first = false; var second = false; var third = false;
            let prefix = first.ref;
            let a: ?Watch = Watch {{ seen: first.ref }};
            let b: ?Watch = Watch {{ seen: second.ref }};
            let c: ?Watch = Watch {{ seen: third.ref }};
            let found = a.as(Iterator).chain(b).as(Iterator).chain(c).as(Iterator)
                .skip(1).as(Iterator).take(1).as(Iterator).any(|value| {{
                    assumes prefix.* and not value.seen.*.*;
                    true
                }});
            found and first and second and third
        }}
        fn verify() {{ let passed = run(); assumes passed; }}",
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    check_iteration_with_budget(&source, None, 4_000_000);
}

fn check_iteration(source: &str) {
    check_iteration_with_contract_limit(source, None);
}

fn check_iteration_with_contract_limit(source: &str, original_limit: Option<&str>) {
    check_iteration_with_budget(source, original_limit, 1_000_000);
}

fn check_iteration_with_budget(source: &str, original_limit: Option<&str>, budget: usize) {
    let package = compile_iteration(source);
    let mut expanded = package.clone();
    let report = elaboration::program::inlining::expand(&mut expanded, &mut { budget }).unwrap();
    assert!(report.expanded_handlers > 0, "{report}");
    for entry in expanded
        .functions
        .iter()
        .filter(|function| matches!(function.name.as_str(), "run" | "filtered" | "short_circuit"))
    {
        nir::visit::expressions(entry, &mut 100_000, |expression, _| {
            assert!(
                !matches!(
                    expression.kind,
                    nir::program::ExprKind::EffectHandle { .. }
                        | nir::program::ExprKind::EffectRequest { .. }
                        | nir::program::ExprKind::TraitMethod { .. }
                ),
                "{report}; remaining expression: {expression}"
            );
            Ok(())
        })
        .unwrap();
    }
    for (label, package) in [("expanded", &expanded), ("original", &package)] {
        let result = luna::execution::interpret_with_protocols(
            package,
            "run",
            "Copy",
            "Drop",
            budget as u64,
        );
        if label == "original" {
            if let Some(expected) = original_limit {
                let error = result
                    .expect_err("unexpanded execution still needs the documented memory contract");
                assert!(error.contains(expected), "{error}");
                continue;
            }
        }
        assert_eq!(
            result.unwrap_or_else(|error| panic!("{label}: {error}")),
            Value::Bool(true)
        );
    }
    let copy = expanded
        .traits
        .iter()
        .find(|item| item.name == "Copy")
        .unwrap()
        .declaration;
    let drop = expanded
        .traits
        .iter()
        .find(|item| item.name == "Drop")
        .unwrap()
        .declaration;
    let prepared =
        luna::host::prepare_with_protocols(&expanded, "verify", budget as u64, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, budget as u64).unwrap();
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
