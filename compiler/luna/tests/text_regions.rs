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
fn unresolved_and_recursive_transfers_do_not_become_static_evidence() {
    let package = support::compile(
        "typealias str=.str'builtin;
        fn looped(value:str)->str {looped(value)}
        fn indirect(f:fn(str)->str,value:str)->str {f(value)}
        fn good(value:str)->str {value}",
    );
    for (name, expected) in [
        ("looped", "fixed-point contract"),
        ("indirect", "named concrete callee"),
    ] {
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
