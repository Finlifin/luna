//! Exercise the shipped pattern contribution, including its target-size bridge.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::{Primitive, Term};

fn compile(width: u32, legacy: bool) -> nir::program::Package {
    let contribution = include_str!("../../../library/builtin/meta_list_patterns.fl");
    let contribution = if legacy {
        let previous = contribution.replace(
            "let position = meta.from_usize(index);\n        self[position]",
            "self[meta.from_usize(index)]",
        );
        assert_ne!(
            previous, contribution,
            "baseline must use the generic index path"
        );
        previous
    } else {
        contribution.to_owned()
    };
    support::compile_with_source_bindings_for_target(
        &format!(
            "mod meta {{ {} }} mod patterns {{ {} }} {contribution}
             comptime fn First(values:meta.List<.Type'builtin>)
                 ->.Type'builtin {{values match {{[first, ..._] => first, _ => bool}}}}
             comptime fn Read(values:meta.List<.Type'builtin>, index:.usize'builtin)
                 ->.Type'builtin {{values.as(ListPatternView).list_pattern_view_get_element_comptime(index)}}",
            include_str!("../../../library/meta/lib.fl"),
            include_str!("../../../library/builtin/patterns.fl"),
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            list_pattern: Some("input.patterns.ListPatternView"),
            ..Default::default()
        },
        Some(nir::target::TargetData::new(width).unwrap()),
    )
    .unwrap()
}

fn arguments(length: usize) -> Vec<Value> {
    vec![Value::TypeList(vec![
        Term::Primitive(Primitive::Bool);
        length
    ])]
}

#[test]
fn staged_list_getter_avoids_an_additional_full_list_copy() {
    for width in [32, 64] {
        let optimized = compile(width, false);
        let legacy = compile(width, true);
        for length in [1, 512] {
            let mut costs = Vec::new();
            for package in [&optimized, &legacy] {
                let mut vm = Interpreter::for_comptime(
                    package,
                    Budget {
                        steps: 1_000_000,
                        ..Default::default()
                    },
                )
                .unwrap();
                let before = vm.remaining_steps();
                assert_eq!(
                    vm.invoke(support::id(package, "First"), arguments(length))
                        .unwrap(),
                    Value::Type(Term::Primitive(Primitive::Bool)),
                );
                costs.push(before - vm.remaining_steps());
            }
            eprintln!(
                "target={width} length={length} optimized={} legacy={}",
                costs[0], costs[1]
            );
            // Small lists may not amortize the extra local binding. A large list
            // must save at least one unit per unrelated element, not just ticks.
            if length == 512 {
                assert!(costs[1] >= costs[0] + 500, "{costs:?}");
            }
        }
    }
}

#[test]
fn staged_list_pattern_handles_empty_input_and_budget_exhaustion() {
    for width in [32, 64] {
        let package = compile(width, false);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for length in [0, 1, 512, 0] {
            assert_eq!(
                vm.invoke(support::id(&package, "First"), arguments(length))
                    .unwrap(),
                Value::Type(Term::Primitive(Primitive::Bool)),
            );
        }
        let mut limited = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10_000,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            limited
                .invoke(support::id(&package, "First"), arguments(512))
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted,
        );
    }
}

#[test]
fn ordinary_list_method_checks_indices_and_recovers_after_failure() {
    for width in [32, 64] {
        let package = compile(width, false);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (length, index) in [(0, 0), (3, 3), (3, u32::MAX)] {
            let mut args = arguments(length);
            args.push(Value::UnsignedInteger(index as u128));
            let error = vm.invoke(support::id(&package, "Read"), args).unwrap_err();
            assert_eq!(error.kind, ErrorKind::IndexOutOfBounds);
            assert!(error.source.is_some());
            let mut args = arguments(3);
            args.push(Value::UnsignedInteger(2));
            assert_eq!(
                vm.invoke(support::id(&package, "Read"), args).unwrap(),
                Value::Type(Term::Primitive(Primitive::Bool)),
            );
        }
    }
}
