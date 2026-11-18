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
