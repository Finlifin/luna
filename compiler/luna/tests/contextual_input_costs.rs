//! Measure actual contextual compilation and invocation work across input sizes.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Value};

#[test]
fn spread_producer_preparation_and_execution_scale_with_input_size() {
    let sizes = [24, 48, 96, 192];
    let mut source = String::from(
        "pure comptime fn Packet<T, ...values: meta.List<T>> -> Type { bool }
         comptime fn part(value: u8) -> meta.List<u8> { [value] }",
    );
    for count in sizes {
        let inputs = (0..count)
            .map(|n| format!("...part({n})"))
            .collect::<Vec<_>>()
            .join(", ");
        source.push_str(&format!(
            "\ncomptime fn Build{count}() -> Type {{ Packet<u8, {inputs}> }}"
        ));
    }
    let package = sysroot::compile(&source).unwrap();
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let producer = id("part");
    for tree in [false, true] {
        let mut previous = None;
        for count in sizes {
            let budget = Budget {
                steps: 1_000_000,
                ..Default::default()
            };
            sysroot::with_comptime(&package, budget, tree, |vm| {
                let before = vm.remaining_steps();
                let cold = budget.steps - before;
                assert!(matches!(
                    vm.invoke(id(&format!("Build{count}")), vec![]).unwrap(),
                    Value::Type(_)
                ));
                let work = before - vm.remaining_steps();
                let compilation = vm.bytecode_compilation_steps();
                assert_eq!(
                    vm.events()
                        .iter()
                        .filter(
                            |event| matches!(event, Event::EnterFunction(id) if *id == producer)
                        )
                        .count(),
                    count
                );
                assert!(work > compilation);
                if tree {
                    assert_eq!(compilation, 0);
                } else {
                    assert!(compilation > 0);
                }
                if let Some((prior_work, prior_compilation)) = previous {
                    // Doubling real producers permits linear overhead variation,
                    // and guards against quadratic growth in charged work.
                    assert!(
                        work > prior_work && work * 10 <= prior_work * 23,
                        "tree={tree}, size={count}, work={prior_work}->{work}"
                    );
                    if !tree {
                        assert!(
                            compilation > prior_compilation
                                && compilation * 10 <= prior_compilation * 23,
                            "size={count}, compilation={prior_compilation}->{compilation}"
                        );
                    }
                }
                eprintln!(
                    "tree={tree}, size={count}, cold_work={cold}, invocation_work={work}, compilation_work={compilation}"
                );
                previous = Some((work, compilation));
            });
        }
    }
}

#[test]
fn repeated_fixed_spreads_compile_once_but_read_the_current_iteration() {
    let package = sysroot::compile(
        r#"
        pure comptime fn Mark<n: Integer> -> Type { bool }
        comptime fn packet(n: Integer) -> Tuple<Integer> { (n,) }
        comptime fn Build(count: Integer) -> Type {
            var i: Integer = 0;
            var result: Type = bool;
            while i < count {
                i = i + 1;
                result = Mark<...packet(i)>;
            }
            result
        }
    "#,
    )
    .unwrap();
    let function = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let packet = function("packet");
    let build = function("Build");
    let mut costs = Vec::new();
    for n in [1, 8] {
        let n_value = comptime_runtime::Integer::from_u128(n);
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, false, |vm| {
            let result = vm
                .invoke(build, vec![Value::Integer(n_value.clone())])
                .unwrap();
            let Value::Type(nir::Term::NormalForm { arguments, .. }) = result else {
                panic!("expected Mark applied to the last input")
            };
            assert_eq!(arguments, [nir::Term::Integer(n_value)]);
            assert_eq!(
                vm.events()
                    .iter()
                    .filter(|event| matches!(event, Event::EnterFunction(id) if *id == packet))
                    .count(),
                n as usize
            );
            assert_eq!(vm.bytecode_input_preparations(), n as usize);
            costs.push(vm.bytecode_compilation_steps());
        });
    }
    assert!(costs[0] > 0);
    assert_eq!(
        costs[0], costs[1],
        "extra activations recompiled lexical code"
    );
}

fn bool_clauses(width: usize) -> String {
    (0..width)
        .map(|index| format!("v{index}: bool"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn a_wide_callable_checks_independent_clause_domains_in_the_real_sysroot() {
    const WIDTH: usize = 720;
    let parameters = bool_clauses(WIDTH);
    let arguments = (0..WIDTH)
        .map(|index| if index + 1 == WIDTH { "false" } else { "true" })
        .collect::<Vec<_>>()
        .join(", ");
    let last = WIDTH - 1;
    let source = format!(
        "comptime fn pick() -> bool where {parameters} {{ v0 and not v{last} }}
         comptime fn check() -> bool {{ pick<{arguments}>() }}"
    );
    assert_eq!(
        sysroot::invoke(&source, "check").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn wide_call_operands_keep_the_caller_and_default_provider_scopes() {
    const WIDTH: usize = 1000;
    let parameters = bool_clauses(WIDTH);
    let arguments = std::iter::once("flag")
        .chain(std::iter::repeat_n("true", WIDTH - 2))
        .chain(std::iter::once("false"))
        .collect::<Vec<_>>()
        .join(", ");
    let last = WIDTH - 1;
    let source = format!(
        "comptime fn pick() -> bool where {parameters}, .mirror: bool = v0
         {{ mirror and not v{last} }}
         comptime fn forward() -> bool where flag: bool {{ pick<{arguments}>() }}
         comptime fn check() -> bool {{ forward<true>() and not forward<false>() }}"
    );
    assert_eq!(
        sysroot::invoke(&source, "check").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn wide_ordinary_defaults_keep_the_domain_of_each_call() {
    const WIDTH: usize = 384;
    let parameters = (0..WIDTH)
        .map(|slot| format!("comptime .T{slot}: Type = root, v{slot}: T{slot}"))
        .collect::<Vec<_>>()
        .join(", ");
    let arguments = |prefix, last| {
        std::iter::repeat_n(prefix, WIDTH - 1)
            .chain(std::iter::once(last))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let last = WIDTH - 1;
    let bytes = arguments("0", "255");
    let flags = arguments("false", "true");
    let source = format!(
        "comptime fn pick(comptime root: Type, {parameters}) -> T{last} {{ v{last} }}
         comptime fn check() -> bool {{ pick(u8, {bytes}) == 255 and pick(bool, {flags}) }}"
    );
    assert_eq!(
        sysroot::invoke(&source, "check").unwrap(),
        Value::Bool(true)
    );
}
