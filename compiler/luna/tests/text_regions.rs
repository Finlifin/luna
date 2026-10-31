//! Backing-region return/read dependencies, separate from runtime permission.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use elaboration::verification::memory::{self, TextRegionOrigin as Origin};
use std::collections::BTreeSet;

fn infer(
    package: &nir::program::Package,
    name: &str,
    fuel: usize,
) -> Result<memory::TextRegionSummary, String> {
    let id = package
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap()
        .declaration;
    memory::infer_text_regions(package, id, &[], &mut { fuel })
}
fn parameter(parameter: usize, fields: &[usize]) -> Origin {
    Origin::Parameter {
        parameter,
        fields: fields.to_vec(),
    }
}

#[test]
fn named_calls_substitute_view_dependencies_instead_of_local_slot_identity() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn pick(flag:bool,a:str,b:str)->str {if flag {return a;} b}
        fn reordered(first:str,second:str,flag:bool)->str {pick(flag,second,echo(first))}
        fn literal()->str {echo(\"literal\")}
        fn symbol(tag:.Symbol'builtin)->str {tag'tag_name}
        fn observed(flag:bool,value:str)->str {let local=echo(value); if flag {local} else {\"static\"}}"
    );
    let before = package.clone();
    let summary = infer(&package, "reordered", 100_000).unwrap();
    assert_eq!(
        summary.returned.as_ref().unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[]), parameter(1, &[])])
    );
    for name in ["literal", "symbol"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([Origin::Static])
        );
    }
    assert_eq!(
        infer(&package, "observed", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    assert!(
        summary
            .to_string()
            .contains("(parameter-text-region 0 (fields))")
    );
    assert_eq!(package, before);
    assert!(infer(&package, "reordered", 1).is_err());
    assert_eq!(infer(&package, "reordered", 100_000).unwrap(), summary);
}

#[test]
fn aggregate_and_optional_results_keep_region_paths_and_absence() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Packet {first:str,second:str}
        fn field(packet:Packet)->str {packet.second}
        fn pair(a:str,b:str)->Packet {Packet {first:b,second:a}}
        fn selected(a:str,b:str)->str {field(pair(a,b))}
        fn maybe(flag:bool,value:str)->?str {if flag {value} else {null}}
        fn absent()->?str {null}
        fn error_view(value:str)->!str bool {error value}",
    );
    assert_eq!(
        infer(&package, "field", 100_000).unwrap().returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[1])])
    );
    assert_eq!(
        infer(&package, "selected", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    let maybe = infer(&package, "maybe", 100_000).unwrap();
    assert!(
        maybe
            .to_string()
            .contains("(view (fields (variant 1) 0) (origins (parameter-text-region 1 (fields))))"),
        "{maybe}"
    );
    assert!(
        infer(&package, "absent", 100_000)
            .unwrap()
            .returned
            .unwrap()
            .is_empty()
    );
    let failure = infer(&package, "error_view", 100_000).unwrap();
    assert!(
        failure.to_string().contains("(view (fields (variant 1) 0)"),
        "{failure}"
    );
}

#[test]
fn byte_reads_export_region_obligations_through_discarded_results() {
    let package = support::compile(&format!(
        "mod primitives {{ {} }} use primitives.*; mod bindings {{ {} }} mod text {{ {} }} use text.TextBytes;
        fn reads(a:str,b:str)->?u8 {{let ignored=a.byte(0);b.byte(0)}}
        fn overwritten(a:str,b:str)->str {{var slot=a;let old=slot.byte(0);slot=b;slot}}
        fn static_read()->?u8 {{\"x\".byte(0)}}
        fn equal(a:str,b:str)->bool {{a == b}}
        fn length(value:str)->u64 {{value.byte_length()}}",
        include_str!("../../../library/builtin/primitives.fl"),
        include_str!("../../../library/builtin/bindings.fl"),
        include_str!("../../../library/builtin/text_impl/bytes.fl")
    ));
    assert_eq!(
        infer(&package, "reads", 100_000).unwrap().reads,
        BTreeSet::from([parameter(0, &[]), parameter(1, &[])])
    );
    assert_eq!(
        infer(&package, "static_read", 100_000).unwrap().reads,
        BTreeSet::from([Origin::Static])
    );
    assert_eq!(
        infer(&package, "equal", 100_000).unwrap().reads,
        BTreeSet::from([parameter(0, &[]), parameter(1, &[])])
    );
    assert!(infer(&package, "length", 100_000).unwrap().reads.is_empty());
    let overwritten = infer(&package, "overwritten", 100_000).unwrap();
    assert_eq!(overwritten.reads, BTreeSet::from([parameter(0, &[])]));
    assert_eq!(
        overwritten.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
}

#[test]
fn unresolved_calls_fail_and_unproductive_recursion_has_no_return() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn looped(value:str)->str {looped(value)}
        fn indirect(f:fn(str)->str,value:str)->str {f(value)}
        fn good(value:str)->str {value}",
    );
    assert!(
        infer(&package, "looped", 100_000)
            .unwrap()
            .returned
            .is_none()
    );
    for (name, expected) in [("indirect", "named concrete callee")] {
        let error = infer(&package, name, 100_000).unwrap_err();
        assert!(error.contains(expected), "{name}: {error}");
    }
    assert_eq!(
        infer(&package, "good", 100_000).unwrap().returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
}

#[test]
fn generic_and_newtype_transport_preserve_the_original_region() {
    let package = support::compile(
        "typealias str=.str'builtin;
        newtype Inner = str; newtype Outer = Inner;
        fn echo(value:T)->T where T {value}
        fn wrapped(value:str)->Outer {value.as(Inner).as(Outer)}
        fn unwrapped(value:Outer)->str {value.as(Inner).as(str)}
        fn roundtrip(value:str)->str {echo<str>(unwrapped(wrapped(value)))}
        fn generic(value:str)->str {echo<str>(value)}",
    );
    for name in ["roundtrip", "generic"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])])
        );
    }
    let wrapped = infer(&package, "wrapped", 100_000).unwrap();
    assert_eq!(
        wrapped.returned.unwrap()[&vec![0, 0]],
        BTreeSet::from([parameter(0, &[])])
    );
}

#[test]
fn returned_closure_carries_captured_text_dependency_without_invoking_it() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn capture(value:str)->%{value:str} fn()->str {|| value}",
    );
    let summary = infer(&package, "capture", 100_000).unwrap();
    assert_eq!(
        summary.returned.unwrap()[&vec![0]],
        BTreeSet::from([parameter(0, &[])])
    );
    assert!(summary.reads.is_empty());
}

#[test]
fn local_writes_replace_subtrees_and_join_only_live_successors() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Packet {first:str,second:str}
        fn replaced(a:str,b:str)->str {var slot=a;slot=b;slot}
        fn scoped(a:str,b:str)->str {var slot=a;do {slot=b;};slot}
        fn joined(flag:bool,a:str,b:str)->str {var slot=a;if flag {slot=b;};slot}
        fn exited(flag:bool,a:str,b:str)->str {
            var slot=a;if flag {slot=b;return \"static\";};slot
        }
        fn field(a:str,b:str,c:str)->Packet {
            var packet=Packet {first:a,second:b};packet.first=c;packet
        }
        fn cleared(value:str)->?str {var slot:?str=value;slot=null;slot}",
    );
    for name in ["replaced", "scoped"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[])])
        );
    }
    assert_eq!(
        infer(&package, "joined", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    assert_eq!(
        infer(&package, "exited", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    let fields = infer(&package, "field", 100_000).unwrap().returned.unwrap();
    assert_eq!(fields[&vec![0]], BTreeSet::from([parameter(2, &[])]));
    assert_eq!(fields[&vec![1]], BTreeSet::from([parameter(1, &[])]));
    assert!(
        infer(&package, "cleared", 100_000)
            .unwrap()
            .returned
            .unwrap()
            .is_empty()
    );
}

#[test]
fn loop_fixed_points_preserve_backedges_and_labeled_exits() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn zero(flag:bool,a:str,b:str)->str {var slot=a;while flag {slot=b;} slot}
        fn once(a:str,b:str)->str {var slot=a;while true {slot=b;break;} slot}
        fn carried(flag:bool,a:str,b:str,c:str)->str {
            var x=a;var y=b;
            while flag {x=echo(y);y=c;continue;}
            x
        }
        fn labeled(flag:bool,a:str,b:str)->str {
            var slot=a;
            while:outer true {
                while true {slot=b;break outer;}
            }
            slot
        }
        fn continued(flag:bool,a:str,b:str,c:str)->str {
            var x=a;var y=b;
            while:outer flag {
                while true {x=y;y=c;continue outer;}
            }
            x
        }
        fn skipped(value:str)->str {var slot=value;while false {slot=\"dead\";} slot}
        fn diverged(value:str)->str {while true {continue;}}
        fn returned(value:str)->str {while true {return value;}}",
    );
    assert_eq!(
        infer(&package, "zero", 100_000).unwrap().returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    for name in ["once", "labeled"] {
        let expected = if name == "once" { 1 } else { 2 };
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(expected, &[])])
        );
    }
    let carried = infer(&package, "carried", 100_000).unwrap();
    assert_eq!(
        carried.returned.as_ref().unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[]), parameter(3, &[])])
    );
    for name in ["skipped", "returned"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])])
        );
    }
    assert!(
        infer(&package, "diverged", 100_000)
            .unwrap()
            .returned
            .is_none()
    );
    assert_eq!(
        infer(&package, "continued", 100_000).unwrap().returned,
        carried.returned
    );
    assert!(infer(&package, "carried", 20).is_err());
    assert_eq!(infer(&package, "carried", 100_000).unwrap(), carried);
}

#[test]
fn match_payloads_keep_origins_and_select_arms_in_order() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Packet {first:str,second:str}
        enum Choice {Empty, Text(str)}
        fn optional(value:?str,fallback:str)->str {value match {text?=>text,null=>fallback,}}
        fn record(value:Packet)->str {value match {{first:_,second:text}=>text,}}
        fn variant(value:Choice,fallback:str)->str {value match {.Text(text)=>text,.Empty=>fallback,}}
        fn ordered(value:str)->str {true match {true=>value,false=>\"dead\",}}
        fn compared(value:str)->bool {value match {\"x\"=>true,_=>false,}}
        fn fallback(a:str,b:str)->str {a match {_ if false=>a,_=>b,}}
        fn guarded(flag:bool,a:str,b:str)->str {a match {text if flag=>text,_=>b,}}
        fn selected(flag:bool,a:str,b:str)->str {
            var slot=a;flag match {true=>do {slot=b;},false=>do {},};slot
        }",
    );
    let optional = infer(&package, "optional", 100_000).unwrap();
    let dump = optional.to_string();
    assert!(
        dump.contains("(parameter-text-region 0 (fields (variant 1) 0))"),
        "{dump}"
    );
    assert!(
        dump.contains("(parameter-text-region 1 (fields))"),
        "{dump}"
    );
    assert_eq!(
        infer(&package, "record", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[1])])
    );
    let variant = infer(&package, "variant", 100_000).unwrap().to_string();
    assert!(
        variant.contains("(parameter-text-region 0 (fields (variant 1) 0))"),
        "{variant}"
    );
    assert_eq!(
        infer(&package, "ordered", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    assert_eq!(
        infer(&package, "compared", 100_000).unwrap().reads,
        BTreeSet::from([parameter(0, &[])])
    );
    assert_eq!(
        infer(&package, "fallback", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
    for name in ["guarded", "selected"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
        );
    }
}

#[test]
fn default_arguments_use_prepared_parameter_origins() {
    use nir::program::ExprKind as K;
    // Input-derived text defaults still fail the P3 gate. Start with typed explicit
    // calls and construct the normalized default frame to test this analysis
    // independently, without claiming source-level default certification.
    let mut package = support::compile(
        "typealias str=.str'builtin;
        fn choose(first:str,second:str,third:str)->str {third}
        fn forwarded(value:str)->str {choose(value,value,value)}
        fn literal()->str {choose(\"static\",\"unused\",\"unused\")}
        fn shadowed(value:str,second:str)->str {choose(value,second,second)}
        fn boolean_default(value:str,.flag:bool=true)->str {value}
        fn source_default(value:str)->str {boolean_default(value)}",
    );
    let choose = package
        .functions
        .iter()
        .find(|f| f.name == "choose")
        .unwrap()
        .clone();
    for function in &mut package.functions {
        nir::visit::expression_mut(&mut function.body, &mut 100_000, |expr, _| {
            if let K::Call {
                callee,
                arguments,
                parameter_bindings,
            } = &mut expr.kind
            {
                if matches!(callee.kind, K::Function(id) if id == choose.declaration) {
                    *parameter_bindings = choose.parameters.clone();
                    for index in 1..3 {
                        arguments[index].is_default = true;
                        arguments[index].value.kind = K::Local(choose.parameters[index - 1]);
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    }
    for name in ["forwarded", "shadowed", "source_default"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])])
        );
    }
    assert_eq!(
        infer(&package, "literal", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static])
    );
    for wrong_frame in [false, true] {
        let mut invalid = package.clone();
        let function = invalid
            .functions
            .iter_mut()
            .find(|f| f.name == "forwarded")
            .unwrap();
        nir::visit::expression_mut(&mut function.body, &mut 100_000, |expr, _| {
            if let K::Call {
                arguments,
                parameter_bindings,
                ..
            } = &mut expr.kind
            {
                if wrong_frame {
                    parameter_bindings.swap(0, 1);
                } else {
                    arguments.swap(1, 2);
                }
            }
            Ok(())
        })
        .unwrap();
        let error = infer(&invalid, "forwarded", 100_000).unwrap_err();
        assert!(
            error.contains(if wrong_frame {
                "does not match callee"
            } else {
                "declaration order"
            }),
            "{error}"
        );
    }
}

#[test]
fn short_circuit_reads_and_writes_follow_only_evaluated_paths() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn skipped(value:str)->bool {false and value == \"x\"}
        fn skipped_or(value:str)->bool {true or value == \"x\"}
        fn read(flag:bool,value:str)->bool {flag and value == \"x\"}
        fn written(flag:bool,a:str,b:str)->str {
            var slot=a;flag and do {slot=b;true};slot
        }
        fn forced(a:str,b:str)->str {var slot=a;false or do {slot=b;true};slot}
        fn returned(flag:bool,a:str,b:str)->str {flag and do {return b;};a}",
    );
    for name in ["skipped", "skipped_or"] {
        assert!(infer(&package, name, 100_000).unwrap().reads.is_empty());
    }
    assert_eq!(
        infer(&package, "read", 100_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    for name in ["written", "returned"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
        );
    }
    assert_eq!(
        infer(&package, "forced", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
}

#[test]
fn static_text_defaults_have_construction_evidence_without_copying_inputs() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Message {text:str}
        fn literal(.text:str=\"static\")->str {text}
        fn message(.value:Message=Message {text:\"nested\"})->str {value.text}
        fn conditional(flag:bool,.text:str=if flag {\"yes\"} else {\"no\"})->str {text}
        fn plain()->str {literal()}
        fn nested()->str {message()}
        fn branched(flag:bool)->str {conditional(flag)}
        fn overridden(value:str)->str {literal(.text=value)}",
    );
    for name in ["plain", "nested", "branched"] {
        let summary = infer(&package, name, 100_000).unwrap();
        assert_eq!(
            summary.returned.unwrap()[&vec![]],
            BTreeSet::from([Origin::Static])
        );
        assert!(summary.reads.is_empty());
    }
    assert_eq!(
        infer(&package, "overridden", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    for source in [
        "typealias str=.str'builtin;fn copy(value:str,.other:str=value)->str {other}",
        "typealias str=.str'builtin;fn temporary(.other:str=do {let text=\"x\";text})->str {other}",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(
            error.contains("P3 stability/termination evidence"),
            "{error}"
        );
    }
}

#[test]
fn failed_structural_patterns_keep_only_reads_reached_before_mismatch() {
    let package = support::compile_with_tuple(
        "typealias str=.str'builtin;
        enum Choice {Empty,Pair(str,bool)}
        fn enum_earlier(value:str)->bool {Choice.Pair(value,false) match {.Pair(\"x\",true)=>true,_=>false,}}
        fn guard_skipped(value:str,other:str)->bool {(value,false) match {(\"x\",true) if other == \"y\"=>true,_=>false,}}
        fn earlier(value:str)->bool {(value,false) match {(\"x\",true)=>true,_=>false,}}
        fn later(value:str)->bool {(false,value) match {(true,\"x\")=>true,_=>false,}}
        fn matched(value:str)->bool {(true,value) match {(true,\"x\")=>true,_=>false,}}",
    );
    assert_eq!(
        infer(&package, "earlier", 100_000).unwrap().reads,
        BTreeSet::from([parameter(0, &[])])
    );
    assert!(infer(&package, "later", 100_000).unwrap().reads.is_empty());
    for name in ["enum_earlier", "guard_skipped"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().reads,
            BTreeSet::from([parameter(0, &[])])
        );
    }

    assert_eq!(
        infer(&package, "matched", 100_000).unwrap().reads,
        BTreeSet::from([parameter(0, &[])])
    );
}

#[test]
fn alternative_patterns_isolate_bindings_and_stop_after_success() {
    let package = support::compile_with_tuple(
        "typealias str=.str'builtin;
        fn swapped(flag:bool,a:str,b:str)->str {
            (a,b) match {((first,second) if flag) or (second,first)=>first,}
        }
        fn first(a:str,b:str)->str {(a,b) match {(picked,_) or (picked,\"x\")=>picked,}}
        fn outer_guard(a:str,b:str,fallback:str)->str {
            (a,b) match {((picked,_) or (picked,\"x\")) if false=>picked,_=>fallback,}
        }",
    );
    assert_eq!(
        infer(&package, "swapped", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    let first = infer(&package, "first", 100_000).unwrap();
    assert_eq!(
        first.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    assert!(first.reads.is_empty());
    let guarded = infer(&package, "outer_guard", 100_000).unwrap();
    assert_eq!(
        guarded.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(2, &[])])
    );
    assert!(guarded.reads.is_empty());
}

#[test]
fn nested_pattern_observations_preserve_text_reads_and_computed_bindings() {
    let package = support::compile_with_tuple(
        "typealias str=.str'builtin;
        fn guarded(a:str,b:str,fallback:str)->str {
            (a,b) match {(first,(second if first == \"x\"))=>second,_=>fallback,}
        }
        fn computed(value:str,a:str,b:str)->str {
            value match {item and (item == \"x\") is true=>a,_=>b,}
        }",
    );
    for name in ["guarded", "computed"] {
        let summary = infer(&package, name, 100_000).unwrap();
        assert_eq!(
            summary.reads,
            BTreeSet::from([Origin::Static, parameter(0, &[])])
        );
        assert_eq!(
            summary.returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
        );
    }
}

#[test]
fn let_patterns_export_success_bindings_and_isolate_failure_scope() {
    let package = support::compile_with_tuple(
        "typealias str=.str'builtin;
        fn tuple(a:str,b:str)->str {let (first,second)=(a,b);second}
        fn optional(value:?str,fallback:str)->str {let text?=value else {return fallback;};text}
        fn shadow(value:?str,text:str)->str {let text?=value else {return text;};text}
        fn known(a:str,b:str)->str {let (text,true)=(a,false) else {return b;};text}
        fn observed(a:str,b:str)->str {let (\"x\",true)=(a,false) else {return b;};a}
        fn guarded(flag:bool,a:str,b:str)->str {let text if flag=a else {return b;};text}
        fn alternatives(flag:bool,a:str,b:str)->str {
            let ((first,second) if flag) or (second,first)=(a,b);first
        }
        fn computed(flag:bool,a:str,b:str)->str {
            let flag and not flag is opposite=flag;if opposite {a}else{b}
        }
        fn looped(value:?str,fallback:str)->str {
            while true {let text?=value else {break;};return text;}
            fallback
        }",
    );
    assert_eq!(
        infer(&package, "tuple", 100_000).unwrap().returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
    for name in ["optional", "shadow", "looped"] {
        let dump = infer(&package, name, 100_000).unwrap().to_string();
        assert!(
            dump.contains("(parameter-text-region 0 (fields (variant 1) 0))"),
            "{dump}"
        );
        assert!(
            dump.contains("(parameter-text-region 1 (fields))"),
            "{dump}"
        );
    }
    for name in ["known", "observed"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[])])
        );
    }
    assert_eq!(
        infer(&package, "observed", 100_000).unwrap().reads,
        BTreeSet::from([parameter(0, &[])])
    );
    assert_eq!(
        infer(&package, "guarded", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    for name in ["alternatives", "computed"] {
        assert_eq!(
            infer(&package, name, 100_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
        );
    }
}

#[test]
fn error_row_transport_moves_view_slots_without_rewriting_origins() {
    let package = support::compile_with_tuple(
        "typealias str=.str'builtin;
        fn widen(value:!str bool)->![bool,str] bool {value}
        fn reordered(value:![bool,str] bool)->![str,bool] bool {value}
        fn failed(text:str)->![bool,str] bool {widen(error text)}
        enum Failure where T {Bad(T)} enum Other {Bad}
        fn forward(value:Tuple<![Failure<T>,Other] bool,str>)->Tuple<![Failure<T>,Other] bool,str> where T {value}
        fn product_slots(value:Tuple<![Other,Failure<str>] bool,str>)->Tuple<![Failure<str>,Other] bool,str> {forward(value)}
        fn sample(text:str)->![Other,Failure<str>] bool {error Failure<str>.Bad(text)}
        fn packet(text:str)->Tuple<Tuple<![Other,Failure<str>] bool,str>,str> {((sample(text),text),text)}
        fn take(value:Tuple<Tuple<![Failure<T>,Other] bool,str>,str>)->str where T {let ((result,text),last)=value;text}
        fn product(text:str)->str {take(packet(text))}",
    );
    let widening = package
        .functions
        .iter()
        .find(|f| f.name == "widen")
        .unwrap();
    let nir::Term::Function { result, .. } = widening.signature.callable_signature() else {
        panic!("signature")
    };
    let nir::Term::ErrorResult { errors, .. } = result.as_ref() else {
        panic!("error result")
    };
    let tag = errors
        .iter()
        .position(|ty| *ty == nir::Term::Primitive(nir::Primitive::Str))
        .unwrap()
        + 1;
    assert_ne!(tag, 1, "widening must relocate the text payload tag");
    let summary = infer(&package, "widen", 100_000).unwrap();
    assert_eq!(
        summary.returned.unwrap()[&vec![usize::MAX, tag, 0]],
        BTreeSet::from([parameter(0, &[usize::MAX, 1, 0])])
    );
    let failed = infer(&package, "failed", 100_000).unwrap();
    assert_eq!(
        failed.returned.unwrap()[&vec![usize::MAX, tag, 0]],
        BTreeSet::from([parameter(0, &[])])
    );
    for name in ["reordered", "product_slots"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let nir::Term::Function { parameters, result } = function.signature.callable_signature()
        else {
            panic!("signature")
        };
        let input =
            memory::borrowed_input_layout(Some(&package), &parameters[0].ty, &mut 100_000).unwrap();
        let output = memory::borrowed_input_layout(Some(&package), result, &mut 100_000).unwrap();
        let summary = infer(&package, name, 100_000).unwrap();
        let returned = summary.returned.unwrap();
        if name == "product_slots" {
            assert_eq!(returned[&vec![1]], BTreeSet::from([parameter(0, &[1])]));
        }
        let origins: BTreeSet<_> = returned.values().flatten().cloned().collect();
        let expected: BTreeSet<_> = input
            .into_iter()
            .map(|leaf| {
                let memory::BorrowedInputLeaf::Text(leaf) = leaf else {
                    panic!("text")
                };
                parameter(0, &leaf.input_fields)
            })
            .collect();
        assert_eq!(origins, expected);
        for leaf in output {
            let memory::BorrowedInputLeaf::Text(leaf) = leaf else {
                panic!("text")
            };
            assert!(returned.contains_key(&leaf.input_fields));
        }
        assert!(summary.reads.is_empty());
    }
    assert_eq!(
        infer(&package, "product", 100_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    // Source and concrete-body instantiation both canonicalize these rows.
    // Retain an explicit Product node to exercise its identity plan without
    // asserting that noncanonical concrete tag order survives instantiation.
    let original = infer(&package, "product_slots", 100_000)
        .unwrap()
        .returned
        .unwrap();
    let mut transported = package.clone();
    let function = transported
        .functions
        .iter_mut()
        .find(|f| f.name == "product_slots")
        .unwrap();
    let nir::Term::Function { result, .. } = &mut function.signature else {
        panic!("function")
    };
    let nir::Term::NormalForm { arguments, .. } = result.as_mut() else {
        panic!("Tuple")
    };
    let nir::Term::NominalValue { fields, .. } = &mut arguments[0] else {
        panic!("fields")
    };
    let nir::Term::ErrorResult { errors, .. } = &mut fields[0] else {
        panic!("row")
    };
    assert_eq!(errors.len(), 2);
    errors.reverse();
    let body = function.body.clone();
    function.body = nir::program::Expr {
        source: body.source,
        ty: result.as_ref().clone(),
        kind: nir::program::ExprKind::ErrorConvert {
            conversion: nir::error_results::Conversion::Product,
            value: Box::new(body),
        },
    };
    let remapped = infer(&transported, "product_slots", 100_000)
        .unwrap()
        .returned
        .unwrap();
    assert_eq!(remapped, original);
    assert!(infer(&package, "product", 1).is_err());
    assert!(infer(&package, "product", 100_000).is_ok());
}

#[test]
fn optional_and_error_propagation_keep_separate_exit_dependencies() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn optional(value:?str)->?str {let text=value?;text}
        fn errors(value:!str str)->!str str {let text=value!;text}
        fn widened(value:!str str)->![bool,str] str {let text=value!;text}
        fn read_after(value:!str str,other:str)->!str bool {let text=value!;text == other}",
    );
    let optional = infer(&package, "optional", 100_000).unwrap();
    assert_eq!(
        optional.returned.unwrap()[&vec![usize::MAX, 1, 0]],
        BTreeSet::from([parameter(0, &[usize::MAX, 1, 0])])
    );
    let errors = infer(&package, "errors", 100_000).unwrap();
    let returned = errors.returned.unwrap();
    for tag in [0, 1] {
        assert_eq!(
            returned[&vec![usize::MAX, tag, 0]],
            BTreeSet::from([parameter(0, &[usize::MAX, tag, 0])])
        );
    }
    let widening = package
        .functions
        .iter()
        .find(|f| f.name == "widened")
        .unwrap();
    let nir::Term::Function { result, .. } = widening.signature.callable_signature() else {
        panic!("signature")
    };
    let nir::Term::ErrorResult { errors, .. } = result.as_ref() else {
        panic!("error result")
    };
    let tag = errors
        .iter()
        .position(|ty| *ty == nir::Term::Primitive(nir::Primitive::Str))
        .unwrap()
        + 1;
    let widened = infer(&package, "widened", 100_000)
        .unwrap()
        .returned
        .unwrap();
    assert_eq!(
        widened[&vec![usize::MAX, tag, 0]],
        BTreeSet::from([parameter(0, &[usize::MAX, 1, 0])])
    );
    assert_eq!(
        widened[&vec![usize::MAX, 0, 0]],
        BTreeSet::from([parameter(0, &[usize::MAX, 0, 0])])
    );
    let after = infer(&package, "read_after", 100_000).unwrap();
    assert_eq!(
        after.reads,
        BTreeSet::from([parameter(0, &[usize::MAX, 0, 0]), parameter(1, &[])])
    );
    assert_eq!(
        after.returned.unwrap()[&vec![usize::MAX, 1, 0]],
        BTreeSet::from([parameter(0, &[usize::MAX, 1, 0])])
    );
}

#[test]
fn scalar_index_operations_preserve_text_scan_dependencies() {
    let package = support::compile(&format!(
        "mod primitives {{ {} }} use primitives.*; mod bindings {{ {} }} mod text {{ {} }} use text.TextBytes;
        fn contains(value:str,needle:u8)->bool {{
            let length=value.byte_length();var index:u64=0;
            while index < length {{
                let byte?=value.byte(index) else {{return false;}};
                if byte'bitvec.as(u64) == needle'bitvec.as(u64) {{return true;}}
                index = index + 1;
            }}
            false
        }}
        fn run()->bool {{contains(\"Aé\",169) and not contains(\"Aé\",0) and not contains(\"\",65)}}",
        include_str!("../../../library/builtin/primitives.fl"),
        include_str!("../../../library/builtin/bindings.fl"),
        include_str!("../../../library/builtin/text_impl/bytes.fl")
    ));
    let scan = infer(&package, "contains", 100_000).unwrap();
    assert_eq!(scan.reads, BTreeSet::from([parameter(0, &[])]));
    assert!(scan.returned.unwrap().is_empty());
    assert_eq!(
        infer(&package, "run", 100_000).unwrap().reads,
        BTreeSet::from([Origin::Static])
    );
    // Algorithm execution is deliberately separate from resource permission.
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn admitted_scalar_intrinsics_keep_input_reads_and_reject_missing_bindings() {
    let package = support::compile_with_source_bindings(
        "typealias str=.str'builtin;typealias F=.f64'builtin;
        ^intrinsic_enum enum Ops {fma}
        fn observed(value:str)->F {
            let weight:F=if value == \"x\" {2.0}else{3.0};
            unsafe {Ops.fma'intrinsic(-weight + 1.0,weight,weight).as(F)}
        }",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            host_fma: Some("input.Ops.fma"),
            ..Default::default()
        },
    )
    .unwrap();
    let summary = infer(&package, "observed", 100_000).unwrap();
    assert_eq!(
        summary.reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert!(summary.returned.unwrap().is_empty());
    let mut invalid = package.clone();
    let function = invalid
        .functions
        .iter_mut()
        .find(|f| f.name == "observed")
        .unwrap();
    let mut removed = 0;
    nir::visit::expression_mut(&mut function.body, &mut 100_000, |expr, _| {
        if let nir::program::ExprKind::Intrinsic { binding, .. } = &mut expr.kind {
            assert!(binding.take().is_some());
            removed += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(removed, 1);
    assert!(
        infer(&invalid, "observed", 100_000)
            .unwrap_err()
            .contains("admission")
    );
    assert!(infer(&package, "observed", 100_000).is_ok());
}

#[test]
fn contracts_observe_entry_snapshots_and_all_normal_return_origins() {
    use nir::program::{EntrySnapshot, Expr, ExprKind as K, Literal, LocalId};
    use nir::{Primitive as P, Term};
    // The source flow checker does not yet certify text predicates. Attach
    // well-typed NIR contracts to test dependency analysis independently.
    let mut package = support::compile(
        "typealias str=.str'builtin;
        fn required(value:str)->str {value}
        fn called(value:str)->str {required(value)}
        fn ensured(flag:bool,a:str,b:str)->str {if flag {return a;} b}
        fn old(flag:bool,value:str)->str {value}
        fn changed(flag:bool,a:str,b:str)->str {if flag {return a;} b}
        fn saved(flag:bool,a:str,b:str)->str {if flag {return a;} b}
        fn diverged(value:str)->str {while true {}}
        fn scalar(flag:bool)->out:bool where requires flag, ensures out {flag}
        fn snapshot(flag:bool)->out:bool where ensures out == flag'old {flag}",
    );
    for function in &mut package.functions {
        let source = function.body.source;
        let local = |id, ty| Expr {
            source,
            ty: Term::Primitive(ty),
            kind: K::Local(id),
        };
        let result = LocalId {
            owner: function.parameters[0].owner,
            index: u32::MAX - 1,
        };
        let saved = LocalId {
            owner: result.owner,
            index: u32::MAX - 2,
        };
        let equals_x = |id| Expr {
            source,
            ty: Term::Primitive(P::Bool),
            kind: K::ScalarEqual(
                Box::new(local(id, P::Str)),
                Box::new(Expr {
                    source,
                    ty: Term::Primitive(P::Str),
                    kind: K::Literal(Literal::String("x".into())),
                }),
            ),
        };
        match function.name.as_str() {
            "required" => function
                .preconditions
                .push(equals_x(function.parameters[0])),
            "ensured" => {
                function.result_binding = Some(result);
                function.postconditions.push(equals_x(result));
            }
            "old" => {
                function.result_binding = Some(result);
                function.entry_snapshots.push(EntrySnapshot {
                    parameter: function.parameters[0],
                    binding: saved,
                });
                function.postconditions.push(Expr {
                    source,
                    ty: Term::Primitive(P::Bool),
                    kind: K::Or(Box::new(local(saved, P::Bool)), Box::new(equals_x(result))),
                });
            }
            "changed" | "saved" => {
                let a = function.parameters[1];
                let b = function.parameters[2];
                let K::Block { statements, .. } = &mut function.body.kind else {
                    panic!("expected function block");
                };
                statements.push(Expr {
                    source,
                    ty: Term::Primitive(P::Unit),
                    kind: K::Assign {
                        place: nir::program::Place {
                            root: a,
                            fields: vec![],
                        },
                        value: Box::new(local(b, P::Str)),
                    },
                });
                if function.name == "saved" {
                    function.entry_snapshots.push(EntrySnapshot {
                        parameter: a,
                        binding: saved,
                    });
                    function.postconditions.push(equals_x(saved));
                } else {
                    function.postconditions.push(equals_x(a));
                }
            }
            "diverged" => {
                function.result_binding = Some(result);
                function.postconditions.push(Expr {
                    source,
                    ty: Term::Primitive(P::Bool),
                    kind: K::ScalarEqual(
                        Box::new(local(result, P::Str)),
                        Box::new(local(function.parameters[0], P::Str)),
                    ),
                });
            }
            _ => (),
        }
    }
    for name in ["scalar", "snapshot"] {
        let summary = infer(&package, name, 100_000).unwrap();
        assert!(summary.reads.is_empty());
        assert!(summary.returned.unwrap().is_empty());
    }
    assert!(
        !package
            .functions
            .iter()
            .find(|f| f.name == "snapshot")
            .unwrap()
            .entry_snapshots
            .is_empty()
    );
    for name in ["required", "called"] {
        let summary = infer(&package, name, 100_000).unwrap();
        assert_eq!(
            summary.reads,
            BTreeSet::from([Origin::Static, parameter(0, &[])])
        );
        assert_eq!(
            summary.returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])])
        );
    }
    assert_eq!(
        infer(&package, "ensured", 100_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(1, &[]), parameter(2, &[])])
    );
    assert_eq!(
        infer(&package, "changed", 100_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(1, &[]), parameter(2, &[])])
    );
    assert_eq!(
        infer(&package, "saved", 100_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    let old = package.functions.iter().find(|f| f.name == "old").unwrap();
    assert!(!old.entry_snapshots.is_empty());
    assert_eq!(
        infer(&package, "old", 100_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    let diverged = infer(&package, "diverged", 100_000).unwrap();
    assert!(diverged.returned.is_none());
    assert!(diverged.reads.is_empty());
    assert!(infer(&package, "ensured", 1).is_err());
    assert!(infer(&package, "ensured", 100_000).is_ok());
}

#[test]
fn recursive_text_summaries_reach_a_fixed_point_across_instances() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn rotate(flag:bool,a:str,b:str)->str {
            if flag {return a;} rotate(flag,b,a)
        }
        fn left(flag:bool,a:str,b:str)->str {
            if flag {return a;} right(flag,b,a)
        }
        fn right(flag:bool,a:str,b:str)->str {left(flag,a,b)}
        fn generic(flag:bool,a:T,b:T)->T where T {
            if flag {return a;} generic<T>(flag,b,a)
        }
        fn concrete(flag:bool,a:str,b:str)->str {generic<str>(flag,a,b)}
        fn reading(flag:bool,a:str,b:str)->str {
            let observed=a == \"x\";
            if flag {return b;} reading(flag,b,a)
        }
        fn endless(a:str,b:str)->str {
            let observed=a == \"x\";
            endless(b,a)
        }
        fn wrapper(flag:bool,a:str,b:str)->str {left(flag,b,a)}",
    );
    let origins = BTreeSet::from([parameter(1, &[]), parameter(2, &[])]);
    for name in ["rotate", "left", "right", "concrete", "reading", "wrapper"] {
        let summary = infer(&package, name, 200_000).unwrap();
        assert_eq!(summary.returned.unwrap()[&vec![]], origins, "{name}");
    }
    assert_eq!(
        infer(&package, "reading", 200_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(1, &[]), parameter(2, &[])])
    );
    let endless = infer(&package, "endless", 200_000).unwrap();
    assert!(endless.returned.is_none());
    assert_eq!(
        endless.reads,
        BTreeSet::from([Origin::Static, parameter(0, &[]), parameter(1, &[])])
    );
    assert!(infer(&package, "left", 10).is_err());
    assert_eq!(
        infer(&package, "left", 200_000).unwrap().returned.unwrap()[&vec![]],
        origins
    );
}

#[test]
fn recursive_observations_retry_pending_summaries_and_reject_failed_callees() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn matches_text(flag:bool,value:str)->bool {
            if flag {return value == \"x\";} matches_text(flag,value)
        }
        fn observed(flag:bool,value:str)->bool {
            value match {item and matches_text(flag,item) is true=>true,_=>false,}
        }
        fn bad(flag:bool,f:fn(str)->str,value:str)->str {
            if flag {return f(value);} bad(flag,f,value)
        }
        fn caller(flag:bool,f:fn(str)->str,value:str)->str {bad(flag,f,value)}",
    );
    let summary = infer(&package, "observed", 200_000).unwrap();
    assert_eq!(
        summary.reads,
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    assert_eq!(summary.returned, Some(Default::default()));
    for name in ["bad", "caller"] {
        assert!(
            infer(&package, name, 200_000)
                .unwrap_err()
                .contains("named concrete callee")
        );
    }
    assert_eq!(infer(&package, "observed", 200_000).unwrap(), summary);
}

#[test]
fn immediate_closures_preserve_capture_order_and_local_return_boundaries() {
    let mut package = support::compile(
        "typealias str=.str'builtin;
        fn identity(value:str)->str {(|arg:str|arg)(value)}
        fn captured(value:str)->str {(||value)()}
        fn snapshot(a:str,b:str)->str {
            var slot=a; (|ignored:bool|slot)(do {slot=b;true})
        }
        fn nested(a:str,b:str)->str {(|value:str| (|other:str|value)(b))(a)}
        fn returning(a:str,b:str)->str {let ignored=(|| {return a;})();b}
        fn reading(a:str,b:str)->str {let ignored=(|| {return a == \"x\";})();b}
        fn recursive(flag:bool,a:str,b:str)->str {
            (|| if flag {a} else {recursive(flag,b,a)})()
        }
        fn endless(value:str)->str {(|| {while true {}})();value}",
    );
    let returning = package
        .functions
        .iter_mut()
        .find(|f| f.name == "returning")
        .unwrap();
    let result = nir::program::LocalId {
        owner: returning.parameters[0].owner,
        index: u32::MAX - 1,
    };
    returning.result_binding = Some(result);
    returning.postconditions.push(nir::program::Expr {
        source: returning.body.source,
        ty: nir::Term::Primitive(nir::Primitive::Bool),
        kind: nir::program::ExprKind::ScalarEqual(
            Box::new(nir::program::Expr {
                source: returning.body.source,
                ty: nir::Term::Primitive(nir::Primitive::Str),
                kind: nir::program::ExprKind::Local(result),
            }),
            Box::new(nir::program::Expr {
                source: returning.body.source,
                ty: nir::Term::Primitive(nir::Primitive::Str),
                kind: nir::program::ExprKind::Literal(nir::program::Literal::String("x".into())),
            }),
        ),
    });
    assert_eq!(
        infer(&package, "returning", 200_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    for name in ["identity", "captured", "snapshot", "nested"] {
        assert_eq!(
            infer(&package, name, 200_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    for name in ["returning", "reading"] {
        assert_eq!(
            infer(&package, name, 200_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "reading", 200_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert_eq!(
        infer(&package, "recursive", 200_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    assert!(
        infer(&package, "endless", 200_000)
            .unwrap()
            .returned
            .is_none()
    );
}

#[test]
fn stable_local_callables_keep_stored_capture_origins() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn other(value:str)->str {\"static\"}
        fn named(value:str)->str {let action=echo;let alias=action;alias(value)}
        fn captured(a:str,b:str)->str {
            var slot=a;let action=||slot;let alias=action;slot=b;alias()
        }
        fn nested(value:str)->str {
            let action=||value;let wrapper=||action();wrapper()
        }
        fn read(value:str)->bool {let action=||value == \"x\";action()}
        fn branch(flag:bool,a:str,b:str)->str {
            let action=|value:str|value;if flag {action(a)}else{action(b)}
        }
        fn recursive(flag:bool,a:str,b:str)->str {
            let action=||if flag {a}else{recursive(flag,b,a)};action()
        }
        fn changed(flag:bool,value:str)->str {
            var action=echo;if flag {action=other;}action(value)
        }",
    );
    for name in ["named", "captured", "nested"] {
        assert_eq!(
            infer(&package, name, 200_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "read", 200_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    for name in ["branch", "recursive"] {
        assert_eq!(
            infer(&package, name, 200_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "changed", 200_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    assert!(infer(&package, "nested", 1).is_err());
    assert!(infer(&package, "nested", 200_000).is_ok());
}

#[test]
fn immediate_callback_parameters_have_invocation_local_code_and_captures() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn other(value:str)->str {\"static\"}
        fn apply(value:str)->str {(|action:fn(str)->str|action(value))(echo)}
        fn nested(value:str)->str {
            (|action:fn(str)->str| (||action(value))())(echo)
        }
        fn alias(value:str)->str {
            (|action:fn(str)->str| {let alias=action;alias(value)})(echo)
        }
        fn reused(value:str)->str {
            let invoke=|action:fn(str)->str|action(value);
            let ignored=invoke(other);invoke(echo)
        }
        fn captured(value:str)->str {
            let action=||value;(|callback:%{value:str} fn()->str|callback())(action)
        }
        fn observed(value:str)->bool {
            let action=||value == \"x\";
            (|callback:%{value:str} fn()->bool|callback())(action)
        }
        fn unknown(action:fn(str)->str,value:str)->str {
            (|callback:fn(str)->str|callback(value))(action)
        }",
    );
    for name in ["apply", "nested", "alias", "reused", "captured"] {
        assert_eq!(
            infer(&package, name, 300_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "observed", 300_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert!(
        infer(&package, "unknown", 300_000)
            .unwrap_err()
            .contains("stable local callable")
    );
    assert!(infer(&package, "reused", 1).is_err());
    assert!(infer(&package, "reused", 300_000).is_ok());
}

#[test]
fn named_higher_order_summaries_separate_callback_instances() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn literal(value:str)->str {\"static\"}
        fn generic(value:T)->T where T {value}
        fn equal(value:str)->bool {value == \"x\"}
        fn apply(action:fn(str)->str,value:str)->str {action(value)}
        fn observe(action:fn(str)->bool,value:str)->bool {action(value)}
        fn relay(action:fn(str)->str,value:str)->str {apply(action,value)}
        fn walk(action:fn(str)->str,flag:bool,a:str,b:str)->str {
            if flag {return action(a);}walk(action,flag,b,a)
        }
        fn input(a:str,b:str)->str {let ignored=apply(literal,b);apply(echo,a)}
        fn static_result(a:str,b:str)->str {let ignored=apply(echo,a);apply(literal,b)}
        fn forwarded(value:str)->str {let action=echo;relay(action,value)}
        fn specialized(value:str)->str {apply(generic<str>,value)}
        fn reads(value:str)->bool {observe(equal,value)}
        fn recursive(flag:bool,a:str,b:str)->str {walk(echo,flag,a,b)}
        fn recursive_static(flag:bool,a:str,b:str)->str {walk(literal,flag,a,b)}
        fn captured(callback:%{value:str} fn()->str)->str {callback()}
        fn closure_callback(value:str)->str {captured(||value)}",
    );
    for name in ["input", "forwarded", "specialized"] {
        assert_eq!(
            infer(&package, name, 300_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    for name in ["static_result", "recursive_static"] {
        assert_eq!(
            infer(&package, name, 300_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([Origin::Static]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "reads", 300_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert_eq!(
        infer(&package, "recursive", 300_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    for name in ["apply", "captured"] {
        assert!(
            infer(&package, name, 300_000)
                .unwrap_err()
                .contains("stable local callable")
        );
    }
    assert_eq!(
        infer(&package, "closure_callback", 300_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    assert!(infer(&package, "recursive", 10).is_err());
    assert!(infer(&package, "recursive", 300_000).is_ok());
}

#[test]
fn named_closure_callbacks_close_code_environments_and_transport_text() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn literal(value:str)->str {\"static\"}
        fn invoke(callback:%{value:str} fn()->str)->str {callback()}
        fn relay(callback:%{value:str} fn()->str)->str {invoke(callback)}
        fn observe(callback:%{value:str} fn()->bool)->bool {callback()}
        fn walk(callback:%{value:str} fn()->str,flag:bool)->str {
            if flag {return callback();}walk(callback,flag)
        }
        fn run(callback:%{action:fn(str)->str,value:str} fn()->str)->str {callback()}
        fn adapt(action:fn(str)->str,value:str)->str {
            run(|| {let alias=action;alias(value)})
        }
        fn input(a:str,b:str)->str {let ignored=adapt(literal,b);adapt(echo,a)}
        fn static_result(a:str,b:str)->str {let ignored=adapt(echo,a);adapt(literal,b)}
        fn forwarded(value:str)->str {let callback=||value;relay(callback)}
        fn reading(value:str)->bool {observe(||value == \"x\")}
        fn recursive(value:str,flag:bool)->str {walk(||value,flag)}
        fn invoke_nested(callback:%{inner:%{value:str} fn()->str} fn()->str)->str {callback()}
        fn nested(value:str)->str {let inner=||value;invoke_nested(||inner())}",
    );
    for name in ["input", "forwarded", "recursive", "nested"] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "static_result", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static])
    );
    assert_eq!(
        infer(&package, "reading", 500_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    for name in ["invoke", "run", "invoke_nested"] {
        assert!(
            infer(&package, name, 500_000)
                .unwrap_err()
                .contains("stable local callable")
        );
    }
    assert!(infer(&package, "nested", 10).is_err());
    assert!(infer(&package, "nested", 500_000).is_ok());
}

#[test]
fn default_callbacks_use_prepared_code_and_capture_frames() {
    use nir::program::{CallArgument, Expr, ExprKind as K};
    use nir::{Primitive as P, Term};
    // Text-derived defaults still need source P3 evidence. These typed explicit
    // calls isolate default-frame transport at NIR without relaxing that gate.
    let mut package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn literal(value:str)->str {\"static\"}
        fn equal(value:str)->bool {value == \"x\"}
        fn choose(value:str,action:fn(str)->str,fallback:fn(str)->str)->str {fallback(value)}
        fn forwarded(value:str)->str {choose(value,echo,literal)}
        fn take(value:str,callback:%{value:str} fn()->str)->str {callback()}
        fn closed(value:str)->str {take(value,||value)}
        fn repeat(value:str,first:%{value:str} fn()->str,second:%{value:str} fn()->str)->str {second()}
        fn copied(value:str)->str {repeat(value,||value,||value)}
        fn evaluate(value:str,callback:fn(str)->bool,answer:bool)->bool {answer}
        fn observing(value:str)->bool {evaluate(value,equal,false)}
        fn nested(value:str,inner:%{value:str} fn()->str,outer:%{inner:%{value:str} fn()->str} fn()->str)->str {outer()}
        fn nested_default(value:str)->str {let inner=||value;nested(value,inner,||inner())}",
    );
    let targets: Vec<_> = package
        .functions
        .iter()
        .filter(|f| ["choose", "take", "repeat", "evaluate", "nested"].contains(&f.name.as_str()))
        .cloned()
        .collect();
    for function in &mut package.functions {
        nir::visit::expression_mut(&mut function.body, &mut 100_000, |expr, _| {
            let K::Call {
                callee,
                arguments,
                parameter_bindings,
            } = &mut expr.kind
            else {
                return Ok(());
            };
            let K::Function(id) = callee.kind else {
                return Ok(());
            };
            let Some(target) = targets.iter().find(|f| f.declaration == id) else {
                return Ok(());
            };
            *parameter_bindings = target.parameters.clone();
            let index = arguments.len() - 1;
            arguments[index].is_default = true;
            match target.name.as_str() {
                "choose" | "repeat" => {
                    arguments[index].value.kind = K::Local(target.parameters[1]);
                }
                "take" | "nested" => {
                    let capture_parameter = if target.name == "take" { 0 } else { 1 };
                    let K::Closure { captures, body, .. } = &mut arguments[index].value.kind else {
                        panic!("closure input");
                    };
                    let original = captures[0].binding;
                    captures[0].binding = target.parameters[capture_parameter];
                    nir::visit::expression_mut(
                        std::sync::Arc::make_mut(body),
                        &mut 100_000,
                        |expr, _| {
                            if matches!(expr.kind, K::Local(id) if id == original) {
                                expr.kind = K::Local(target.parameters[capture_parameter]);
                            }
                            Ok(())
                        },
                    )?;
                }
                "evaluate" => {
                    let source = arguments[index].value.source;
                    let Term::Function { parameters, .. } = &target.signature else {
                        panic!("function signature");
                    };
                    arguments[index].value.kind = K::Call {
                        callee: Box::new(Expr {
                            source,
                            ty: parameters[1].ty.clone(),
                            kind: K::Local(target.parameters[1]),
                        }),
                        arguments: vec![CallArgument {
                            passing: Default::default(),
                            parameter: 0,
                            is_default: false,
                            value: Expr {
                                source,
                                ty: Term::Primitive(P::Str),
                                kind: K::Local(target.parameters[0]),
                            },
                        }],
                        parameter_bindings: vec![],
                    };
                }
                _ => unreachable!(),
            }
            Ok(())
        })
        .unwrap();
    }
    for name in ["forwarded", "closed", "copied", "nested_default"] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "observing", 500_000).unwrap().reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert!(infer(&package, "closed", 10).is_err());
    assert!(infer(&package, "closed", 500_000).is_ok());
}

#[test]
fn returned_callables_transport_identity_with_captured_text() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn named()->fn(str)->str {echo}
        fn factory(value:str)->%{value:str} fn()->str {||value}
        fn relay(value:str)->%{value:str} fn()->str {factory(value)}
        fn pass(callback:%{value:str} fn()->str)->%{value:str} fn()->str {callback}
        struct Holder {callback:%{value:str} fn()->str}
        fn wrapped(value:str)->Holder {Holder {callback:||value}}
        fn stored(value:str)->str {let callback=factory(value);callback()}
        fn immediate(value:str)->str {factory(value)()}
        fn forwarded(value:str)->str {let callback=relay(value);callback()}
        fn passed(value:str)->str {let callback=pass(factory(value));callback()}
        fn field(value:str)->str {let holder=wrapped(value);holder.callback()}
        fn function(value:str)->str {let callback=named();callback(value)}
        fn maybe(flag:bool,value:str)->?%{value:str} fn()->str {if flag {||value}else{null}}
        fn optional(flag:bool,value:str,fallback:str)->str {
            let callback?=maybe(flag,value) else {return fallback;};callback()
        }",
    );
    for name in [
        "stored",
        "immediate",
        "forwarded",
        "passed",
        "field",
        "function",
    ] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "optional", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    assert!(infer(&package, "passed", 10).is_err());
    assert!(infer(&package, "passed", 500_000).is_ok());
}

#[test]
fn callable_value_flow_replaces_assignments_and_rejects_unknown_joins() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn literal(value:str)->str {\"static\"}
        struct Holder {callback:fn(str)->str}
        fn assigned(value:str)->str {var action=echo;action=literal;action(value)}
        fn field(value:str)->str {var holder=Holder {callback:echo};holder.callback=literal;holder.callback(value)}
        fn copied(value:str)->str {var action=echo;let saved=action;action=literal;saved(value)}
        fn captured(value:str)->str {
            var action=echo;let saved=||action(value);action=literal;saved()
        }
        fn choose(flag:bool)->fn(str)->str {if flag {echo}else{literal}}
        fn mixed(flag:bool,value:str)->str {let action=choose(flag);action(value)}
        fn unknown(flag:bool,holder:Holder,value:str)->str {
            let action=if flag {echo}else{holder.callback};action(value)
        }
        fn same(flag:bool,value:str)->str {let action=if flag {echo}else{echo};action(value)}",
    );
    for name in ["assigned", "field"] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([Origin::Static]),
            "{name}"
        );
    }
    for name in ["copied", "captured"] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "same", 500_000).unwrap().returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
    assert_eq!(
        infer(&package, "mixed", 500_000).unwrap().returned.unwrap()[&vec![]],
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    for name in ["unknown"] {
        assert!(
            infer(&package, name, 500_000)
                .unwrap_err()
                .contains("stable local callable")
        );
    }
}

#[test]
fn closure_alternatives_keep_reads_returns_and_higher_order_calls() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn choose(flag:bool,value:str)->%{value:str} fn()->str {
            if flag {||value} else {||{let read=value == \"static\";\"static\"}}
        }
        fn invoke(action:%{value:str} fn()->str)->str {action()}
        fn selected(flag:bool,value:str)->str {let action=choose(flag,value);action()}
        fn forwarded(flag:bool,value:str)->str {invoke(choose(flag,value))}
        fn unknown(flag:bool,value:str,other:%{value:str} fn()->str)->str {
            let action=if flag {choose(flag,value)} else {other};action()
        }",
    );
    for name in ["selected", "forwarded"] {
        let summary = infer(&package, name, 500_000).unwrap();
        let expected = BTreeSet::from([Origin::Static, parameter(1, &[])]);
        assert_eq!(summary.reads, expected);
        assert_eq!(summary.returned.unwrap()[&vec![]], expected);
    }
    assert!(
        infer(&package, "unknown", 500_000)
            .unwrap_err()
            .contains("stable local callable")
    );
    assert!(infer(&package, "selected", 1).is_err());
    assert!(infer(&package, "selected", 500_000).is_ok());
}

#[test]
fn closure_alternatives_evaluate_arguments_once_before_independent_storage_updates() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn run(flag:bool,a:str,b:str,c:str)->str {
            var slot=a;let pointer=slot.ref;
            let action:%{pointer:*str} fn(str)->str = if flag {
                |value| {pointer.* = value;pointer.*}
            } else {
                |value| {let observed=value == \"read\";pointer.*}
            };
            let result=action(do {slot=b;c});
            slot
        }",
    );
    let summary = infer(&package, "run", 500_000).unwrap();
    assert_eq!(
        summary.reads,
        BTreeSet::from([Origin::Static, parameter(3, &[])])
    );
    assert_eq!(
        summary.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(2, &[]), parameter(3, &[])])
    );
}

#[test]
fn closure_alternatives_conservatively_join_distinct_captures_and_diverging_reads() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn choose(flag:bool,a:str,b:str)->%{value:str} fn()->str {
            if flag {let value=a;||value} else {let value=b;||value}
        }
        fn captured(flag:bool,a:str,b:str)->str {choose(flag,a,b)()}
        fn forever()->str {forever()}
        fn diverging(flag:bool,value:str)->str {
            let action:%{value:str} fn()->str = if flag {||value}
                else {||{let read=value == \"read\";forever()}};
            action()
        }",
    );
    assert_eq!(
        infer(&package, "captured", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    let summary = infer(&package, "diverging", 500_000).unwrap();
    assert_eq!(
        summary.reads,
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    assert_eq!(
        summary.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
}

#[test]
fn a_valid_closure_alternative_cannot_hide_unknown_descriptor_access() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn run(flag:bool,pointer:*str)->str {
            let action:%{pointer:*str} fn()->str = if flag {
                ||{let saved=pointer;\"static\"}
            } else {||pointer.*};
            action()
        }",
    );
    let error = infer(&package, "run", 500_000).unwrap_err();
    assert!(error.contains("descriptor"), "{error}");
}

#[test]
fn aggregate_callbacks_specialize_named_calls_by_parameter_and_field_path() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn literal(value:str)->str {\"static\"}
        struct Pair {first:fn(str)->str,second:fn(str)->str}
        struct Wrapper {pair:Pair}
        fn first(pair:Pair,value:str)->str {pair.first(value)}
        fn second(pair:Pair,value:str)->str {pair.second(value)}
        fn nested(wrapper:Wrapper,value:str)->str {first(wrapper.pair,value)}
        fn relay(pair:Pair,value:str)->str {second(pair,value)}
        fn input(value:str)->str {nested(Wrapper {pair:Pair {first:echo,second:literal}},value)}
        fn static_result(value:str)->str {relay(Pair {first:echo,second:literal},value)}
        fn separated(flag:bool,value:str)->str {
            if flag {first(Pair {first:echo,second:literal},value)}
            else {first(Pair {first:literal,second:echo},value)}
        }
        fn optional(callback:?(fn(str)->str),value:str)->str {
            let action?=callback else {return \"absent\";};action(value)
        }
        fn unpack(value:str)->str {optional(echo,value)}
        struct Captured {callback:%{value:str} fn()->str}
        fn invoke(holder:Captured)->str {holder.callback()}
        fn captured(value:str)->str {invoke(Captured {callback:||value})}
        fn recur(pair:Pair,value:str,flag:bool)->str {
            if flag {first(pair,value)}else{recur(pair,value,true)}
        }
        fn recursive(value:str,flag:bool)->str {recur(Pair {first:echo,second:literal},value,flag)}
        fn unknown(pair:Pair,value:str)->str {first(pair,value)}",
    );
    for name in ["input", "captured", "recursive"] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "static_result", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static])
    );
    assert_eq!(
        infer(&package, "separated", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    assert_eq!(
        infer(&package, "unpack", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert!(
        infer(&package, "unknown", 500_000)
            .unwrap_err()
            .contains("stable local callable")
    );
    assert!(infer(&package, "recursive", 10).is_err());
    assert!(infer(&package, "recursive", 500_000).is_ok());
}

#[test]
fn local_descriptor_pointers_preserve_backing_origins_and_replace_slots() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Pair {left:str,right:str}
        fn read(value:str)->str {let pointer=value.ref;pointer.*}
        fn replace(first:str,second:str)->str {var value=first;let pointer=value.ref;pointer.* = second;value}
        fn saved(first:str,second:str)->str {var value=first;let pointer=value.ref;let before=pointer.*;pointer.* = second;before}
        fn field(first:str,second:str)->str {var pair=Pair {left:first,right:first};let pointer=pair.ref;pointer.*.left=second;pair.left}
        fn sibling(first:str,second:str)->str {var pair=Pair {left:first,right:first};let pointer=pair.ref;pointer.*.left=second;pointer.*.right}
        fn alias(first:str,second:str)->str {var value=first;let pointer=value.ref;let other=pointer;other.* = second;pointer.*}
        fn conditional(flag:bool,first:str,second:str)->str {var value=first;let pointer=value.ref;if flag {pointer.* = second;};pointer.*}
        fn rebound(first:str,second:str)->str {var left=first;var right=second;var pointer=left.ref;pointer.* = do {pointer=right.ref;second};left}
        fn looped(flag:bool,first:str,second:str)->str {var value=first;let pointer=value.ref;while flag {pointer.* = second;};pointer.*}
        fn nested(value:str)->str {let pair=Pair {left:value,right:\"static\"};let pointer=pair.right.ref;pointer.*}
        fn observed(first:str,second:str)->str {var value=first;let pointer=value.ref;let equal=pointer.* == \"x\";pointer.* = second;value}
        fn unknown(pointer:*str)->str {pointer.*}
        fn ambiguous(flag:bool,first:str,second:str)->str {let pointer=if flag {first.ref}else{second.ref};pointer.*}
        fn dangling(value:str)->str {let pointer=do {let local=value;local.ref};pointer.*}
        fn external(pointer:*str)->str {pointer.*}
        fn crossing(value:str)->str {external(value.ref)}
        fn captured(value:str)->str {let pointer=value.ref;let action=||pointer.*;action()}",
    );
    for (name, index) in [
        ("read", 0),
        ("crossing", 0),
        ("captured", 0),
        ("replace", 1),
        ("saved", 0),
        ("field", 1),
        ("sibling", 0),
        ("alias", 1),
        ("rebound", 1),
    ] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(index, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "conditional", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    assert_eq!(
        infer(&package, "looped", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    assert_eq!(
        infer(&package, "nested", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([Origin::Static])
    );
    let observed = infer(&package, "observed", 500_000).unwrap();
    assert_eq!(
        observed.reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert_eq!(
        observed.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
    assert_eq!(
        infer(&package, "ambiguous", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    for (name, message) in [
        ("unknown", "target is unknown"),
        ("dangling", "escapes its local scope"),
    ] {
        let error = infer(&package, name, 500_000).unwrap_err();
        assert!(error.contains(message), "{name}: {error}");
    }
    assert!(infer(&package, "alias", 10).is_err());
    assert!(infer(&package, "alias", 500_000).is_ok());
}

#[test]
fn named_descriptor_calls_update_shared_storage_and_join_normal_exits() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Pair {left:str,right:str}
        fn replace(pointer:*str,value:str) {pointer.* = value;}
        fn relay(pointer:*str,value:str) {replace(pointer,value)}
        fn swap(left:*str,right:*str) {let before=left.*;left.* = right.*;right.* = before;}
        fn identity(pointer:*str)->*str {pointer}
        fn generic(pointer:*T)->*T where T {pointer}
        fn apply(action:fn(*str,str),pointer:*str,value:str) {action(pointer,value)}
        fn old(pointer:*str,value:str)->str {let before=pointer.*;pointer.* = value;return before;}
        fn branch(flag:bool,pointer:*str,value:str) {if flag {pointer.* = value;return;};}
        fn observed(pointer:*str,value:str)->bool {let equal=pointer.* == \"x\";pointer.* = value;equal}
        fn written(first:str,second:str)->str {var value=first;relay(value.ref,second);value}
        fn saved(first:str,second:str)->str {var value=first;old(value.ref,second)}
        fn swapped(first:str,second:str)->str {var left=first;var right=second;swap(left.ref,right.ref);left}
        fn aliased(value:str)->str {var text=value;swap(text.ref,text.ref);text}
        fn forwarded(value:str)->str {identity(value.ref).*}
        fn specialized(value:str)->str {generic(value.ref).*}
        fn ordered(first:str,second:str)->str {var value=first;replace(value.ref,do {value=second;first});value}
        fn field(first:str,second:str)->str {var pair=Pair {left:first,right:first};replace(pair.left.ref,second);pair.left}
        fn sibling(first:str,second:str)->str {var pair=Pair {left:first,right:first};replace(pair.left.ref,second);pair.right}
        fn callback(first:str,second:str)->str {var value=first;apply(replace,value.ref,second);value}
        fn joined(flag:bool,first:str,second:str)->str {var value=first;branch(flag,value.ref,second);value}
        fn reading(first:str,second:str)->str {var value=first;let equal=observed(value.ref,second);value}
        fn recursive(pointer:*str,value:str) {replace(pointer,value);recursive(pointer,value)}
        fn recursion(value:str)->str {var text=value;recursive(text.ref,\"static\");text}
        fn leak(pointer:**str,value:str) {let local=value;pointer.* = local.ref;}
        fn escaping(value:str)->str {var pointer=value.ref;leak(pointer.ref,value);pointer.*}",
    );
    for (name, index) in [
        ("written", 1),
        ("saved", 0),
        ("swapped", 1),
        ("aliased", 0),
        ("forwarded", 0),
        ("specialized", 0),
        ("ordered", 0),
        ("field", 1),
        ("sibling", 0),
        ("callback", 1),
    ] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(index, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "joined", 500_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    let reading = infer(&package, "reading", 500_000).unwrap();
    assert_eq!(
        reading.reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert_eq!(
        reading.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
    assert!(
        infer(&package, "recursion", 500_000)
            .unwrap()
            .returned
            .is_none()
    );
    assert!(
        infer(&package, "escaping", 500_000)
            .unwrap_err()
            .contains("escapes")
    );
    assert!(infer(&package, "callback", 10).is_err());
    assert!(infer(&package, "callback", 500_000).is_ok());
}

#[test]
fn text_defaults_read_prepared_pointer_storage_without_authorizing_writes() {
    use nir::program::ExprKind as K;
    // Source admission still separately needs P3 and Copy evidence for text.
    // Typed NIR fixtures isolate prepared-frame dependency transfer.
    let mut package = support::compile(
        "typealias str=.str'builtin;
        fn read(pointer:*str)->str {pointer.*}
        fn mutate(pointer:*str)->str {pointer.* = \"changed\";pointer.*}
        fn take(pointer:*str,ignored:bool,value:str)->str {value}
        fn direct(value:str)->str {let pointer=value.ref;take(pointer,false,pointer.*)}
        fn helper(value:str)->str {let pointer=value.ref;take(pointer,false,read(pointer))}
        fn late(first:str,second:str)->str {var value=first;let pointer=value.ref;take(pointer,do {value=second;false},read(pointer))}
        fn mutation(value:str)->str {var text=value;let pointer=text.ref;take(pointer,false,mutate(pointer))}
        fn direct_mutation(value:str)->str {var text=value;let pointer=text.ref;take(pointer,false,do {pointer.* = \"changed\";pointer.*})}
        fn unknown(pointer:*str)->str {take(pointer,false,pointer.*)}
        fn resumed(first:str,second:str)->str {var text=first;let pointer=text.ref;let ignored=take(pointer,false,pointer.*);pointer.* = second;text}",
    );
    let target = package
        .functions
        .iter()
        .find(|f| f.name == "take")
        .unwrap()
        .clone();
    for function in &mut package.functions {
        nir::visit::expression_mut(&mut function.body, &mut 100_000, |expr, _| {
            let K::Call {
                callee,
                arguments,
                parameter_bindings,
            } = &mut expr.kind
            else {
                return Ok(());
            };
            if !matches!(callee.kind, K::Function(id) if id == target.declaration) {
                return Ok(());
            }
            let K::Local(pointer) = arguments[0].value.kind else {
                panic!("local pointer")
            };
            *parameter_bindings = target.parameters.clone();
            arguments[2].is_default = true;
            nir::visit::expression_mut(&mut arguments[2].value, &mut 100_000, |value, _| {
                if matches!(value.kind, K::Local(id) if id == pointer) {
                    value.kind = K::Local(target.parameters[0]);
                }
                Ok::<_, String>(())
            })?;
            Ok(())
        })
        .unwrap();
    }
    for (name, index) in [("direct", 0), ("helper", 0), ("late", 1), ("resumed", 1)] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(index, &[])]),
            "{name}"
        );
    }
    for name in ["mutation", "direct_mutation"] {
        let error = infer(&package, name, 500_000).unwrap_err();
        assert!(
            error.contains("default argument cannot write through pointers"),
            "{name}: {error}"
        );
    }
    assert!(
        infer(&package, "unknown", 500_000)
            .unwrap_err()
            .contains("target is unknown")
    );
    assert!(infer(&package, "helper", 10).is_err());
    assert!(infer(&package, "helper", 500_000).is_ok());
}

#[test]
fn admitted_scalar_pointer_default_preserves_text_observation_dependencies() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn take(pointer:*bool,.observed:bool=pointer.*)->bool {observed}
        fn entry(value:str)->bool {let equal=value == \"x\";take(equal.ref)}",
    );
    let summary = infer(&package, "entry", 500_000).unwrap();
    assert_eq!(
        summary.reads,
        BTreeSet::from([Origin::Static, parameter(0, &[])])
    );
    assert!(summary.returned.unwrap().is_empty());
}

#[test]
fn closure_descriptor_frames_separate_captures_from_external_storage() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn replace(pointer:*str,value:str) {pointer.* = value;}
        fn apply(callback:%{pointer:*str,value:str} fn()->str)->str {callback()}
        fn read(value:str)->str {let pointer=value.ref;let action=||pointer.*;action()}
        fn returning(value:str)->str {let pointer=value.ref;let action=||pointer;let alias=action();alias.*}
        fn order(first:str,second:str)->str {var value=first;let pointer=value.ref;(|ignored:bool| pointer.*)(do {value=second;false})}
        fn written(first:str,second:str)->str {var value=first;let pointer=value.ref;let action=|| {pointer.* = second;};action();value}
        fn snapshot(first:str,second:str)->str {var value=first;let pointer=value.ref;let action=|| {pointer.* = second;value};action()}
        fn external(first:str,second:str)->str {var value=first;let pointer=value.ref;let action=|| {pointer.* = second;value};let saved=action();value}
        fn input(first:str,second:str)->str {var value=first;(|pointer:*str| {pointer.* = second;})(value.ref);value}
        fn forwarded(first:str,second:str)->str {var value=first;let pointer=value.ref;let action=|| {replace(pointer,second);};action();value}
        fn repeated(first:str,second:str)->str {var value=first;let pointer=value.ref;let action=||pointer.*;let before=action();pointer.* = second;action()}
        fn cycle(flag:bool,first:str,second:str)->str {var value=first;let pointer=value.ref;let action=|| {pointer.* = second;};while flag {action();};value}
        fn nested(first:str,second:str)->str {var value=first;let pointer=value.ref;let outer=|| {let inner=|| {pointer.* = second;};inner();};outer();value}
        fn joined(flag:bool,first:str,second:str)->str {var value=first;let pointer=value.ref;let action=|| {if flag {pointer.* = second;return;};};action();value}
        fn looped(flag:bool,first:str,second:str)->str {var value=first;let pointer=value.ref;let action=|| {while flag {pointer.* = second;};};action();value}
        fn passed(first:str,value:str)->str {var text=first;let pointer=text.ref;let saved=apply(|| {pointer.* = value;pointer.*});text}
        fn escape(value:str)->*str {(|| {let local=value;local.ref})()}
        fn stored(value:str)->str {var pointer=value.ref;let address=pointer.ref;let action=|| {let local=value;address.* = local.ref;};action();pointer.*}",
    );
    for (name, index) in [
        ("read", 0),
        ("returning", 0),
        ("order", 1),
        ("written", 1),
        ("snapshot", 0),
        ("external", 1),
        ("input", 1),
        ("forwarded", 1),
        ("nested", 1),
        ("repeated", 1),
        ("passed", 1),
    ] {
        assert_eq!(
            infer(&package, name, 1_000_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(index, &[])]),
            "{name}"
        );
    }
    for name in ["joined", "looped", "cycle"] {
        assert_eq!(
            infer(&package, name, 1_000_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])]),
            "{name}"
        );
    }
    for name in ["escape", "stored"] {
        let error = infer(&package, name, 1_000_000).unwrap_err();
        assert!(error.contains("escapes"), "{name}: {error}");
    }
    assert!(infer(&package, "nested", 10).is_err());
    assert!(infer(&package, "nested", 1_000_000).is_ok());
}

#[test]
fn pattern_observations_reject_indirect_descriptor_writes() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn change(pointer:*str)->bool {pointer.* = \"changed\";true}
        fn observed(value:str)->str {
            var text=value;let pointer=text.ref;
            true match {_ and change(pointer) is true=>text,_=>text,}
        }
        fn reused()->str {var text=\"changed\";let pointer=text.ref;let ignored=change(pointer);true match {_ and change(pointer) is true=>text,_=>text,}}",
    );
    for name in ["observed", "reused"] {
        let error = infer(&package, name, 500_000).unwrap_err();
        assert!(
            error.contains("text observation cannot write through pointers"),
            "{name}: {error}"
        );
    }
}

#[test]
fn closure_storage_work_scales_with_reachable_slots() {
    fn work(unrelated: usize, calls: usize) -> usize {
        let mut source = String::from("typealias str=.str'builtin;fn entry(value:str)->str {");
        for index in 0..unrelated {
            source.push_str(&format!("let spare{index}=\"unused\";"));
        }
        source.push_str("let pointer=value.ref;let action=||pointer.*;");
        for index in 0..calls {
            source.push_str(&format!("let result{index}=action();"));
        }
        source.push_str("value}");
        let package = support::compile(&source);
        let declaration = package
            .functions
            .iter()
            .find(|f| f.name == "entry")
            .unwrap()
            .declaration;
        let mut fuel = 2_000_000;
        let summary = memory::infer_text_regions(&package, declaration, &[], &mut fuel).unwrap();
        assert_eq!(
            summary.returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(0, &[])])
        );
        2_000_000 - fuel
    }
    let small_once = work(0, 1);
    let small_many = work(0, 32);
    let large_once = work(128, 1);
    let large_many = work(128, 32);
    println!(
        "closure storage work: small_once={small_once}, small_many={small_many}, large_once={large_once}, large_many={large_many}"
    );
    // Unused locals may add initial scan cost, but must not multiply the cost
    // of each additional invocation. Leave room for accounting refinements.
    assert!(large_many - large_once <= small_many - small_once + 2048);
}

#[test]
fn closure_storage_reachability_preserves_transitive_aliases_and_cycles() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Node {text:str,next:?*Node}
        fn chain(value:str)->str {let pointer=value.ref;let indirect=pointer.ref;let action=||indirect.*.*;action()}
        fn replace(first:str,second:str)->str {var value=first;let pointer=value.ref;let indirect=pointer.ref;let action=|| {indirect.*.* = second;};action();value}
        fn unrelated(first:str,second:str)->str {let untouched=first;var value=first;let pointer=value.ref;let action=|| {pointer.* = second;};action();untouched}
        fn cycle(value:str)->str {
            var node=Node {text:value,next:null};node.next=node.ref;let pointer=node.ref;
            let action=||pointer.*.text;action()
        }",
    );
    for (name, index) in [("chain", 0), ("replace", 1), ("unrelated", 0)] {
        assert_eq!(
            infer(&package, name, 500_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(index, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "cycle", 500_000).unwrap().returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    assert!(infer(&package, "cycle", 10).is_err());
    assert!(infer(&package, "cycle", 500_000).is_ok());
}

#[test]
fn recursive_descriptor_effects_converge_over_aliases_and_storage_states() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn fill(flag:bool,pointer:*str,value:str) {if flag {pointer.* = value;return;};fill(flag,pointer,value)}
        fn left(flag:bool,pointer:*str,value:str) {if flag {pointer.* = value;return;};right(flag,pointer,value)}
        fn right(flag:bool,pointer:*str,value:str) {left(flag,pointer,value)}
        fn alternate(flag:bool,first:*str,second:*str) {if flag {first.* = second.*;return;};alternate(flag,second,first)}
        fn choose(flag:bool,pointer:*str,value:str)->str {if flag {return pointer.*;};pointer.* = value;choose(flag,pointer,value)}
        fn local(flag:bool,pointer:*str)->str {if flag {return pointer.*;};let copy=pointer.*;local(flag,copy.ref)}
        fn identity(flag:bool,pointer:*T)->*T where T {if flag {return pointer;};identity<T>(flag,pointer)}
        fn forever(pointer:*str,value:str) {let observed=pointer.* == \"x\";pointer.* = value;forever(pointer,value)}
        fn broken(flag:bool,pointer:*str,action:fn(*str)->str)->str {if flag {return action(pointer);};broken(flag,pointer,action)}
        fn rejected(flag:bool,value:str,action:fn(*str)->str)->str {broken(flag,value.ref,action)}
        fn direct(flag:bool,first:str,second:str)->str {var value=first;fill(flag,value.ref,second);value}
        fn mutual(flag:bool,first:str,second:str)->str {var value=first;left(flag,value.ref,second);value}
        fn reordered(flag:bool,first:str,second:str)->str {var a=first;var b=second;alternate(flag,a.ref,b.ref);a}
        fn shared(flag:bool,value:str)->str {var text=value;alternate(flag,text.ref,text.ref);text}
        fn result(flag:bool,first:str,second:str)->str {var text=first;choose(flag,text.ref,second)}
        fn local_storage(flag:bool,value:str)->str {local(flag,value.ref)}
        fn generic(flag:bool,value:str)->str {identity(flag,value.ref).*}
        fn diverged(first:str,second:str)->str {var text=first;forever(text.ref,second);text}",
    );
    for (name, index) in [
        ("direct", 2),
        ("mutual", 2),
        ("shared", 1),
        ("local_storage", 1),
        ("generic", 1),
    ] {
        assert_eq!(
            infer(&package, name, 2_000_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(index, &[])]),
            "{name}"
        );
    }
    for name in ["reordered", "result"] {
        assert_eq!(
            infer(&package, name, 2_000_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])]),
            "{name}"
        );
    }
    let diverged = infer(&package, "diverged", 2_000_000).unwrap();
    assert!(diverged.returned.is_none());
    assert_eq!(
        diverged.reads,
        BTreeSet::from([Origin::Static, parameter(0, &[]), parameter(1, &[])])
    );
    assert!(
        infer(&package, "rejected", 2_000_000)
            .unwrap_err()
            .contains("stable local callable")
    );
    assert!(infer(&package, "mutual", 10).is_err());
    assert!(infer(&package, "mutual", 2_000_000).is_ok());
}

#[test]
fn finite_descriptor_aliases_union_reads_and_weakly_update_possible_slots() {
    let package = support::compile(
        "typealias str=.str'builtin;
        struct Pair {left:str,right:str}
        fn pick(flag:bool,first:*str,second:*str)->*str {if flag {first}else{second}}
        fn recurse(flag:bool,first:*str,second:*str)->*str {if flag {return first;};recurse(flag,second,first)}
        fn mutate(flag:bool,pointer:*str,value:str) {if flag {pointer.* = value;return;};mutate(flag,pointer,value)}
        fn recursive_write(flag:bool,first:str,second:str,replacement:str)->str {var a=first;var b=second;let pointer=pick(flag,a.ref,b.ref);mutate(flag,pointer,replacement);a}
        fn closure_write(flag:bool,first:str,second:str,replacement:str)->str {var a=first;var b=second;let pointer=pick(flag,a.ref,b.ref);let action=|| {pointer.* = replacement;};action();a}
        fn called(flag:bool,first:str,second:str)->str {pick(flag,first.ref,second.ref).*}
        fn recursive(flag:bool,first:str,second:str)->str {recurse(flag,first.ref,second.ref).*}
        fn closure(flag:bool,first:str,second:str)->str {let pointer=pick(flag,first.ref,second.ref);let action=||pointer.*;action()}
        fn indirect(flag:bool,first:str,second:str)->str {let a=first.ref;let b=second.ref;let pointer=if flag {a.ref}else{b.ref};pointer.*.*}
        fn field_read(flag:bool,first:str,second:str)->str {let a=Pair {left:first,right:first};let b=Pair {left:second,right:second};let pointer=if flag {a.ref}else{b.ref};pointer.*.left}
        fn left(flag:bool,first:str,second:str,replacement:str)->str {var a=first;var b=second;let pointer=pick(flag,a.ref,b.ref);pointer.* = replacement;a}
        fn right(flag:bool,first:str,second:str,replacement:str)->str {var a=first;var b=second;let pointer=pick(flag,a.ref,b.ref);pointer.* = replacement;b}
        fn same(flag:bool,first:str,replacement:str)->str {var value=first;let pointer=pick(flag,value.ref,value.ref);pointer.* = replacement;value}
        fn field_write(flag:bool,first:str,second:str,replacement:str)->str {var pair=Pair {left:first,right:second};let pointer=pick(flag,pair.left.ref,pair.right.ref);pointer.* = replacement;pair.left}
        fn unknown(flag:bool,pointer:*str,value:str)->str {let selected=if flag {value.ref}else{pointer};selected.*}
        fn leaking(flag:bool,pointer:*str)->*str {if flag {return pointer;};let local=\"local\";local.ref}
        fn escaped(flag:bool,value:str)->str {leaking(flag,value.ref).*}",
    );
    for name in ["called", "recursive", "closure", "indirect", "field_read"] {
        assert_eq!(
            infer(&package, name, 2_000_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(2, &[])]),
            "{name}"
        );
    }
    for name in ["left", "field_write", "recursive_write", "closure_write"] {
        assert_eq!(
            infer(&package, name, 2_000_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[]), parameter(3, &[])]),
            "{name}"
        );
    }
    assert_eq!(
        infer(&package, "right", 2_000_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(2, &[]), parameter(3, &[])])
    );
    assert_eq!(
        infer(&package, "same", 2_000_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(2, &[])])
    );
    assert!(
        infer(&package, "unknown", 2_000_000)
            .unwrap_err()
            .contains("target is unknown")
    );
    assert!(
        infer(&package, "escaped", 2_000_000)
            .unwrap_err()
            .contains("escapes")
    );
    assert!(infer(&package, "recursive", 10).is_err());
    assert!(infer(&package, "recursive", 2_000_000).is_ok());
}

#[test]
fn descriptor_pointer_slot_updates_preserve_alias_and_evaluation_order() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn pick(flag:bool,first:**str,second:**str)->**str {if flag {first}else{second}}
        fn replace(first:str,second:str)->str {
            var pointer=first.ref;let slot=pointer.ref;slot.* = second.ref;pointer.*
        }
        fn weak(flag:bool,first:str,second:str,replacement:str)->str {
            var a=first.ref;var b=second.ref;
            let slot=pick(flag,a.ref,b.ref);slot.* = replacement.ref;a.*
        }
        fn targets_before_value(first:str,second:str,replacement:str)->str {
            var a=first;var b=second;var pointer=a.ref;
            pointer.* = do {pointer=b.ref;replacement};a
        }
        fn changed_pointer(first:str,second:str,replacement:str)->str {
            var a=first;var b=second;var pointer=a.ref;
            pointer.* = do {pointer=b.ref;replacement};pointer.*
        }
        fn unknown_survives(flag:bool,external:*str,value:str)->str {
            var a=external;var b=value.ref;
            let slot=pick(flag,a.ref,b.ref);slot.* = value.ref;a.*
        }
        fn strong_recovers(external:*str,value:str)->str {
            var pointer=external;let slot=pointer.ref;slot.* = value.ref;pointer.*
        }",
    );
    for (name, expected) in [
        ("replace", BTreeSet::from([parameter(1, &[])])),
        (
            "weak",
            BTreeSet::from([parameter(1, &[]), parameter(3, &[])]),
        ),
        ("targets_before_value", BTreeSet::from([parameter(2, &[])])),
        ("changed_pointer", BTreeSet::from([parameter(1, &[])])),
        ("strong_recovers", BTreeSet::from([parameter(1, &[])])),
    ] {
        assert_eq!(
            infer(&package, name, 2_000_000).unwrap().returned.unwrap()[&vec![]],
            expected,
            "{name}"
        );
    }
    assert!(
        infer(&package, "unknown_survives", 2_000_000)
            .unwrap_err()
            .contains("target is unknown")
    );
}

#[test]
fn finite_named_callees_join_results_reads_and_independent_storage_effects() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn echo(value:str)->str {value}
        fn literal(value:str)->str {\"static\"}
        fn invoke(action:fn(str)->str,value:str)->str {action(value)}
        fn recur(flag:bool,action:fn(str)->str,value:str)->str {
            if flag {return action(value);};recur(flag,action,value)
        }
        fn higher(flag:bool,value:str)->str {invoke(if flag {echo}else{literal},value)}
        fn recursive(flag:bool,value:str)->str {recur(flag,if flag {echo}else{literal},value)}
        fn captured(flag:bool,value:str)->str {
            let selected=if flag {echo}else{literal};let closure=||selected(value);closure()
        }
        fn fill(pointer:*str,value:str) {pointer.* = value;}
        fn observe(pointer:*str,value:str) {let observed=pointer.* == value;}
        fn storage(flag:bool,first:str,second:str)->str {
            var value=first;let action=if flag {fill}else{observe};action(value.ref,second);value
        }
        fn read(pointer:*str)->str {pointer.*}
        fn absent(pointer:*str)->str {\"static\"}
        fn read_dispatch(flag:bool,value:str)->str {
            let action=if flag {read}else{absent};action(value.ref)
        }
        fn rejected(flag:bool,external:*str)->str {
            let action=if flag {read}else{absent};action(external)
        }
        fn evaluated_once(flag:bool,first:str,second:str)->str {
            var previous=first;var current=second;let action=if flag {echo}else{literal};
            let ignored=action(do {let saved=previous;previous=current;current=saved;saved});previous
        }
        fn selected_before_input(flag:bool,value:str)->str {
            var action=if flag {echo}else{literal};action(do {action=literal;value})
        }
        fn spin(value:str)->str {let seen=value == \"spin\";spin(value)}
        fn partial(flag:bool,value:str)->str {let action=if flag {echo}else{spin};action(value)}",
    );
    for name in [
        "higher",
        "recursive",
        "captured",
        "read_dispatch",
        "selected_before_input",
    ] {
        assert_eq!(
            infer(&package, name, 2_000_000).unwrap().returned.unwrap()[&vec![]],
            BTreeSet::from([Origin::Static, parameter(1, &[])]),
            "{name}"
        );
    }
    let storage = infer(&package, "storage", 2_000_000).unwrap();
    assert_eq!(
        storage.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    assert_eq!(
        storage.reads,
        BTreeSet::from([parameter(1, &[]), parameter(2, &[])])
    );
    assert_eq!(
        infer(&package, "evaluated_once", 2_000_000)
            .unwrap()
            .returned
            .unwrap()[&vec![]],
        BTreeSet::from([parameter(2, &[])])
    );
    let partial = infer(&package, "partial", 2_000_000).unwrap();
    assert_eq!(
        partial.returned.unwrap()[&vec![]],
        BTreeSet::from([parameter(1, &[])])
    );
    assert_eq!(
        partial.reads,
        BTreeSet::from([Origin::Static, parameter(1, &[])])
    );
    assert!(
        infer(&package, "rejected", 2_000_000)
            .unwrap_err()
            .contains("target is unknown")
    );
    assert!(infer(&package, "recursive", 10).is_err());
    assert!(infer(&package, "recursive", 2_000_000).is_ok());
}

#[test]
fn type_valued_dependent_inputs_preserve_text_backing_regions() {
    let package = support::compile(
        "typealias Type=.Type'builtin; typealias str=.str'builtin;
        fn echo(value:str)->str where T,index:T {value}
        fn run(value:str)->str {echo<Type,bool>(value)}",
    );
    let before = package.clone();
    let summary = infer(&package, "run", 100_000).unwrap();
    assert_eq!(
        summary.returned.as_ref().unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    assert_eq!(package, before);
    assert!(infer(&package, "run", 0).is_err());
    assert_eq!(infer(&package, "run", 100_000).unwrap(), summary);
}

#[test]
fn trait_method_text_summaries_keep_selected_subject_and_argument_origins() {
    let package = support::compile(
        "typealias str=.str'builtin;typealias u8=.u8'builtin;
        trait View {fn view(self, text:str)->str;}
        impl View for bool {fn view(self,text:str)->str {text}}
        impl View for u8 {fn view(self,text:str)->str {\"static\"}}
        fn boolean(flag:bool,text:str)->str {flag.view(text)}
        fn byte(flag:u8,text:str)->str {flag.view(text)}
        fn alias(flag:bool,text:str)->str {let selected=bool.view;selected(flag,text)}",
    );
    for name in ["boolean", "alias"] {
        let summary = infer(&package, name, 100_000).unwrap();
        assert_eq!(
            summary.returned.unwrap()[&vec![]],
            BTreeSet::from([parameter(1, &[])])
        );
        assert!(summary.reads.is_empty());
    }
    let summary = infer(&package, "byte", 100_000).unwrap();
    assert_eq!(
        summary.returned.unwrap()[&vec![]],
        BTreeSet::from([Origin::Static])
    );
    assert!(infer(&package, "alias", 1).is_err());
    assert!(infer(&package, "alias", 100_000).is_ok());
}

#[test]
fn deeply_nested_text_scopes_preserve_origins_and_fail_with_a_diagnostic() {
    use nir::program::{Expr, ExprKind};

    let baseline = support::compile("typealias str=.str'builtin; fn echo(value:str)->str {value}");
    let nested = |depth| {
        let mut package = baseline.clone();
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.name == "echo")
            .unwrap();
        for _ in 0..depth {
            let inner = function.body.clone();
            function.body = Expr {
                source: inner.source,
                ty: inner.ty.clone(),
                kind: ExprKind::Block {
                    statements: vec![],
                    result: Some(Box::new(inner)),
                },
            };
        }
        package
    };
    let package = nested(80);
    let summary = infer(&package, "echo", 1_000_000).unwrap();
    assert_eq!(
        summary.returned.as_ref().unwrap()[&vec![]],
        BTreeSet::from([parameter(0, &[])])
    );
    assert!(infer(&package, "echo", 1).is_err());
    assert_eq!(infer(&package, "echo", 1_000_000).unwrap(), summary);
    let error = infer(&nested(140), "echo", 1_000_000).unwrap_err();
    assert!(
        error.contains("depth") && error.contains("exhausted"),
        "{error}"
    );
    assert_eq!(infer(&package, "echo", 1_000_000).unwrap(), summary);
}
