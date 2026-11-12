//! Pending input domains retain ordinary block and branch checking rules.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Interpreter, Value};
use elaboration::materialization::SourceInvocationQueries;

fn check_both(source: &str, calls: &[&str]) {
    check_both_in_budget(source, calls, Budget::default());
}

fn check_both_in_budget(source: &str, calls: &[&str], budget: Budget) {
    let package = sysroot::compile(source).unwrap_or_else(|failure| panic!("{failure}\n{source}"));
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    let context = elaboration::ptc::Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    let solver = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, budget)
        } else {
            Interpreter::for_comptime(&package, budget)
        }
        .unwrap()
        .with_budgeted_type_solver(&solver)
        .with_method_resolver(&methods)
        .with_associated_resolver(&associated);
        assert_eq!(
            vm.invoke(entry, vec![]).unwrap(),
            Value::Bool(true),
            "tree={tree}"
        );
        let observed = vm
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) if id.package == entry.package => package
                    .functions
                    .iter()
                    .find(|f| f.declaration == *id)
                    .map(|f| f.name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(observed, calls, "tree={tree}");
        if budget.steps != Budget::default().steps {
            eprintln!(
                "tree={tree}, invocation_work={}",
                budget.steps - vm.remaining_steps()
            );
        }
    }
}

#[test]
fn pending_blocks_keep_continuation_type_facts_and_source_order() {
    check_both(
        "
        pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn mark(flag: bool) -> bool { flag }
        comptime fn Build(value: T) -> Type
        where T
        {
            Packet<.values = do {
                if T != bool { return Packet<u8, .values = []>; }
                let flag: bool = value;
                mark(flag);
                [1, 255]
            }, Byte()>
        }
        comptime fn check() -> bool {
            Build<bool>(true) == Packet<u8, .values = [1, 255]>
                and Build<u8>(9) == Packet<u8, .values = []>
        }
        ",
        &["check", "Build", "mark", "Byte", "Build"],
    );
}

#[test]
fn pending_branches_use_ordinary_type_facts() {
    for branch in [
        "if T == bool { let flag: bool = value; mark(flag); [1] } else { [] }",
        "when { T != bool => [], else => do { let flag: bool = value; mark(flag); [1] } }",
        "T match { 'bool => do { let flag: bool = value; mark(flag); [1] }, _ => [] }",
        "T match { 'bool if mark(value) => [1], _ => [] }",
        "when { T == bool => do { mark(value); [1] },
            do { let witness: Type = refines { Itself != bool } T; false } => [], else => [] }",
    ] {
        check_both(
            &format!(
                "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark(flag: bool) -> bool {{ flag }}
            comptime fn Build(value: T) -> Type
            where T
            {{ Packet<.values = {branch}, Byte()> }}
            comptime fn check() -> bool {{
                Build<bool>(true) == Packet<u8, .values = [1]>
                    and Build<u8>(9) == Packet<u8, .values = []>
            }}"
            ),
            &["check", "Build", "mark", "Byte", "Build", "Byte"],
        );
    }
}

#[test]
fn pending_region_facts_do_not_escape_or_cross_branches() {
    for body in [
        "let unused = Packet<.values = do {
            if T != bool { return Packet<u8>; }
            let flag: bool = value; [1]
        }, Byte()>; let escaped: bool = value; Packet<u8>",
        "Packet<.values = if T == bool { [] } else { let invalid: bool = value; [1] }, Byte()>",
        "Packet<.values = when {
            T == bool => [],
            else => do { let invalid: bool = value; [1] }
        }, Byte()>",
        "Packet<.values = T match {
            'bool => [], _ => do { let invalid: bool = value; [1] }
        }, Byte()>",
    ] {
        let failure = sysroot::compile(&format!(
            "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn Bad(value: T) -> Type
            where T
            {{ {body} }}
        "
        ))
        .err()
        .expect("invalid branch evidence was accepted");
        assert!(failure.contains("type mismatch"), "{body}: {failure}");
    }
}

#[test]
fn pending_blocks_retain_mixed_pattern_producers_and_guard_reads() {
    for declaration in [
        "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type { bool }",
        "struct Packet where T, .values: meta.List<T> = [] {}",
    ] {
        check_both(
            &format!(
                "
            {declaration}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark() -> bool {{ true }}
            comptime fn check() -> bool {{
                let selected = Packet<.values = do {{
                    let (Kind, flag) = (u8, mark());
                    let Alias = Kind;
                    true match {{
                        _ if flag and Alias == u8 => [1],
                        _ => [],
                    }}
                }}, Byte()>;
                selected == Packet<u8, .values = [1]>
            }}"
            ),
            &["check", "mark", "Byte"],
        );
    }
}

#[test]
fn nested_lexical_exit_keeps_outer_writes_in_ordinary_and_pending_blocks() {
    for (input, expected_calls) in [
        (
            "do { do { changed = mark(); }; let inner = changed; inner }",
            vec!["check", "mark"],
        ),
        (
            "Packet<.values = do { do { changed = mark(); }; [1] }, Byte()> == Packet<u8, .values = [1]>",
            vec!["check", "mark", "Byte"],
        ),
    ] {
        check_both(
            &format!(
                "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark() -> bool {{ true }}
            comptime fn check() -> bool {{
                var changed = false;
                let result = {input};
                result and changed
            }}
        "
            ),
            &expected_calls,
        );
    }
}

#[test]
fn block_exit_does_not_skip_checks_or_export_a_private_binding() {
    for source in [
        "comptime fn bad() -> bool { return true; let invalid: bool = 7; false }",
        "comptime fn bad() -> bool { let unused = do { let hidden_value = true; hidden_value }; hidden_value }",
    ] {
        assert!(sysroot::compile(source).is_err(), "accepted {source}");
    }
    check_both(
        "
        comptime fn mark() -> bool { true }
        comptime fn leave() -> bool { return true; mark(); false }
        comptime fn check() -> bool { leave() }
    ",
        &["check", "leave"],
    );
}

#[test]
fn ordinary_block_frames_keep_callee_delivery_shadowing_and_receiver_storage() {
    check_both(
        "
        struct Counter { count: Integer }
        impl Counter {
            comptime fn next(self) -> u8 { self.count += 1; 7 }
        }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn check() -> bool {
            var counter = Counter { count: 0 };
            let value = (do { do { identity<u8> } })(do { do { counter.next() } });
            var outer = false;
            let shadow = do { let outer = true; do { outer } };
            do { do { outer = true; } };
            shadow and outer and value == 7 and counter.count == 1
        }
        ",
        &["check", "next", "identity"],
    );
}

#[test]
fn nested_block_exits_preserve_definition_checks_and_skip_unexecuted_calls() {
    check_both(
        "comptime fn check() -> bool { do { do { let hidden_value = true; hidden_value } } }",
        &["check"],
    );
    check_both(
        "
        comptime fn mark() -> bool { false }
        comptime fn leave() -> bool { do { do { return true; mark() } }; false }
        comptime fn check() -> bool { leave() }
        ",
        &["check", "leave"],
    );
    for source in [
        "comptime fn bad() -> bool { do { do { return true; let invalid: bool = 7; false } } }",
        "comptime fn bad() -> bool { do { do { let hidden_value = true; } }; hidden_value }",
    ] {
        assert!(sysroot::compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn contextual_block_frames_preserve_mixed_children_and_producer_order() {
    let mut body = "[1, 255]".to_owned();
    for _ in 0..24 {
        body = format!("do {{ do {{ trace += 1; }}; {body} }}");
    }
    check_both_in_budget(
        &format!(
            "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark(flag: bool) -> bool {{ flag }}
            comptime fn Build(value: T) -> Type
            where T
            {{
                var trace: Integer = 0;
                let selected = Packet<.values = do {{
                    do {{
                        if T != bool {{ return Packet<u8>; }}
                        let flag: bool = value;
                        mark(flag);
                        {body}
                    }}
                }}, Byte()>;
                if trace != 24 {{ return Packet<u8>; }}
                selected
            }}
            comptime fn check() -> bool {{
                Build<bool>(true) == Packet<u8, .values = [1, 255]>
                    and Build<u8>(9) == Packet<u8>
            }}"
        ),
        &["check", "Build", "mark", "Byte", "Build"],
        // This tests deep source scopes and both execution engines, including
        // their charged cold admission work. Production limits stay unchanged.
        Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    );
}

#[test]
fn contextual_nested_exit_keeps_facts_local_and_checks_unreachable_children() {
    for (body, diagnostic) in [
        (
            "let unused = Packet<.values = do { do {
            if T != bool { return Packet<u8>; }
            let flag: bool = value; [1]
        } }, Byte()>; let escaped: bool = value; Packet<u8>",
            "type mismatch",
        ),
        (
            "Packet<.values = do { do {
            return Packet<u8>; let invalid: bool = 7; [1]
        } }, Byte()>",
            "integer runtime proxy requires a supported fixed-width integer target",
        ),
    ] {
        let failure = sysroot::compile(&format!(
            "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn Bad(value: T) -> Type
            where T
            {{ {body} }}"
        ))
        .unwrap_err();
        assert!(failure.contains(diagnostic), "{body}: {failure}");
    }
}
