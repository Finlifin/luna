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
        "{} mod text {{ {} }}
        fn reads(a:str,b:str)->?u8 {{let ignored=text.byte(a,0);text.byte(b,0)}}
        fn overwritten(a:str,b:str)->str {{var slot=a;let old=text.byte(slot,0);slot=b;slot}}
        fn static_read()->?u8 {{text.byte(\"x\",0)}}
        fn equal(a:str,b:str)->bool {{a == b}}
        fn length(value:str)->u64 {{text.byte_length(value)}}",
        include_str!("../../../library/builtin/primitives.fl"),
        include_str!("../../../library/builtin/text_bytes.fl")
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
    let nir::Term::TypeListValue(fields) = &mut arguments[0] else {
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
        "{} mod text {{ {} }}
        fn contains(value:str,needle:u8)->bool {{
            let length=text.byte_length(value);var index:u64=0;
            while index < length {{
                let byte?=text.byte(value,index) else {{return false;}};
                if byte'bitvec.as(u64) == needle'bitvec.as(u64) {{return true;}}
                index = index + 1;
            }}
            false
        }}
        fn run()->bool {{contains(\"Aé\",169) and not contains(\"Aé\",0) and not contains(\"\",65)}}",
        include_str!("../../../library/builtin/primitives.fl"),
        include_str!("../../../library/builtin/text_bytes.fl")
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
    assert!(
        infer(&package, "changed", 200_000)
            .unwrap_err()
            .contains("stable local callable")
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
