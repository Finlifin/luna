//! Member defaults observe completed parent identities without replacing producers.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Interpreter};

#[test]
fn member_defaults_observe_parent_value_indices_in_their_annotations() {
    let source = "
        struct Config where enabled: bool { flag: bool }
        comptime fn compute_flag(value: bool) -> bool { value }
        struct Holder where enabled: bool {
            pub newtype Wrapped<.config: Config<enabled> = defer Config<enabled>{flag: compute_flag(enabled)}> = bool;
        }
        comptime fn Enabled() -> Type { Holder<true>.Wrapped<> }
        comptime fn ExpectedEnabled() -> Type {
            Holder<true>.Wrapped<.config = Config<true> { flag: true }>
        }
        comptime fn Disabled() -> Type { Holder<false>.Wrapped<> }
        comptime fn ExpectedDisabled() -> Type {
            Holder<false>.Wrapped<.config = Config<false> { flag: false }>
        }
    ";
    let package = sysroot::compile(source).unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for tree in [false, true] {
        let make = || {
            if tree {
                Interpreter::for_comptime_tree(&package, Budget::default())
            } else {
                Interpreter::for_comptime(&package, Budget::default())
            }
            .unwrap()
        };
        let mut actual_vm = make();
        let mut expected_vm = make();
        for (iteration, (entry, expected)) in [
            ("Enabled", "ExpectedEnabled"),
            ("Disabled", "ExpectedDisabled"),
            ("Enabled", "ExpectedEnabled"),
        ]
        .into_iter()
        .enumerate()
        {
            let actual = actual_vm.invoke(id(entry), vec![]).unwrap();
            let expected = expected_vm.invoke(id(expected), vec![]).unwrap();
            assert_eq!(actual, expected, "{entry}, tree={tree}");
            assert_eq!(
                actual_vm
                    .events()
                    .iter()
                    .filter(|event| {
                        matches!(event, Event::EnterFunction(found) if *found == id("compute_flag"))
                    })
                    .count(),
                iteration + 1,
                "the default producer runs once"
            );
            assert!(
                !expected_vm.events().iter().any(|event| {
                    matches!(event, Event::EnterFunction(found) if *found == id("compute_flag"))
                }),
                "an explicit argument skips its default"
            );
        }
    }
    let wrong = source.replace(
        "Config<false> { flag: false }>",
        "Config<true> { flag: false }>",
    );
    let failure = sysroot::compile(&wrong).unwrap_err();
    assert!(failure.contains("type mismatch"), "{failure}");
}
