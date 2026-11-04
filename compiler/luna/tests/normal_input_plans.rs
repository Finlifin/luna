//! NF inputs share contextual checking while retaining source evaluation slots.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Interpreter, Value};
use nir::program::{ExprKind as K, NormalFormArgument, Package};

fn id(package: &Package, name: &str) -> nir::DeclarationId {
    package
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .declaration
}

fn invoke_both(package: &Package, name: &str) -> Value {
    let mut vm = Interpreter::for_comptime(package, Budget::default()).unwrap();
    let actual = vm.invoke(id(package, name), vec![]).unwrap();
    let mut tree = Interpreter::for_comptime_tree(package, Budget::default()).unwrap();
    assert_eq!(tree.invoke(id(package, name), vec![]).unwrap(), actual);
    actual
}

fn assert_ordinary_reordered_inputs(package: &Package, name: &str) {
    let function = package.functions.iter().find(|f| f.name == name).unwrap();
    let mut count = 0;
    nir::visit::expressions(function, &mut 10_000, |expression, _| {
        if let K::NormalFormType {
            arguments,
            plan: Some(plan),
            ..
        } = &expression.kind
            && plan.order == [1, 0]
        {
            // A ready domain must use ordinary checked expressions, not an
            // NF-specific If/Block/Match tree interpreted again at invocation.
            assert!(
                arguments
                    .iter()
                    .all(|argument| matches!(argument, NormalFormArgument::Value(_))),
                "{expression}"
            );
            count += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 1, "{name}: {}", function.body);
}

#[test]
fn late_static_domains_contextualize_whole_collections_as_ordinary_expressions() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, ...values: meta.List<T>> -> Type { bool }
        comptime fn Direct() -> Type { Packet<u8, 1, 255> }
        comptime fn Branch() -> Type {
            Packet<.values = if true { [1, 255] } else { [] }, u8>
        }
        comptime fn Block() -> Type {
            Packet<.values = do { let selected = true; if selected { [1, 255] } else { [] } }, u8>
        }
        comptime fn Matched() -> Type {
            Packet<.values = true match { true => [1, 255], false => [] }, u8>
        }
        comptime fn Nested() -> Type {
            Packet<.values = [[1], []], meta.List<u8>>
        }
        comptime fn NestedDirect() -> Type {
            Packet<meta.List<u8>, [1], []>
        }
        comptime fn Product() -> Type {
            Packet<.values = [(1, true)], Tuple<u8, bool>>
        }
        comptime fn ProductDirect() -> Type {
            Packet<Tuple<u8, bool>, (1, true)>
        }
    ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    for name in ["Branch", "Block", "Matched"] {
        assert_ordinary_reordered_inputs(&package, name);
        assert_eq!(invoke_both(&package, name), direct);
    }
    for (name, reference) in [("Nested", "NestedDirect"), ("Product", "ProductDirect")] {
        assert_ordinary_reordered_inputs(&package, name);
        assert_eq!(
            invoke_both(&package, name),
            invoke_both(&package, reference)
        );
    }
}

#[test]
fn fixed_inputs_and_defaults_share_the_contextual_plan() {
    let package = sysroot::compile(
        "
        pure comptime fn Key<T, .value: ?T = null> -> Type { bool }
        comptime fn Explicit() -> Type { Key<.value = null, u8> }
        comptime fn Default() -> Type { Key<u8> }
        pure comptime fn Point<Es: meta.List<Type>, value: Tuple<.Es = Es>> -> Type { bool }
        comptime fn Product() -> Type { Point<[u8, bool], (1, true)> }
        typealias ProductAlias = Point<[u8, bool], (1, true)>;
        comptime fn Alias() -> Type { ProductAlias }
    ",
    )
    .unwrap();
    assert_ordinary_reordered_inputs(&package, "Explicit");
    assert_eq!(
        invoke_both(&package, "Explicit"),
        invoke_both(&package, "Default")
    );
    assert_eq!(
        invoke_both(&package, "Product"),
        invoke_both(&package, "Alias")
    );
}

#[test]
fn dynamic_domains_do_not_move_or_replay_input_effects() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, ...values: meta.List<T>> -> Type { bool }
        comptime fn choose() -> bool { true }
        comptime fn Byte() -> Type { u8 }
        comptime fn Direct() -> Type { Packet<u8, 1, 255> }
        comptime fn Build() -> Type {
            Packet<.values = if choose() { [1, 255] } else { [] }, Byte()>
        }
        comptime fn Wrong() -> Type {
            Packet<.values = if choose() { [256] } else { [] }, Byte()>
        }
    ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
        assert!(vm.invoke(id(&package, "Wrong"), vec![]).is_err());
        let start = vm.events().len();
        assert_eq!(vm.invoke(id(&package, "Build"), vec![]).unwrap(), direct);
        let calls: Vec<_> = vm.events()[start..]
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(declaration) => package
                    .functions
                    .iter()
                    .find(|f| f.declaration == *declaration)
                    .map(|f| f.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, ["Build", "choose", "Byte"]);
    }
}

#[test]
fn ready_domains_reject_invalid_values_in_every_branch() {
    for value in [
        "[256]",
        "[true]",
        "if true { [1] } else { [256] }",
        "true match { true => [1], false => [false] }",
    ] {
        let source = format!(
            "
            pure comptime fn Packet<T, ...values: meta.List<T>> -> Type {{ bool }}
            comptime fn Bad() -> Type {{ Packet<.values = {value}, u8> }}
        "
        );
        assert!(sysroot::compile(&source).is_err(), "accepted {source}");
    }
}
