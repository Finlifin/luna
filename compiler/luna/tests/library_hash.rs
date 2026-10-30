#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

fn source(body: &str) -> String {
    format!(
        "mod primitives {{ {} }} use primitives.*;\nmod comparison {{ {} }}\nmod resources {{ {} }}\nmod conversion {{ {} }}\nmod traits {{ {} }}\npub use traits.*;\n{body}",
        include_str!("../../../library/builtin/primitives.fl"),
        support::with_attrs(include_str!(
            "../../../library/builtin/numeric/comparison.fl"
        )),
        include!("support/resources_source.rs"),
        include_str!("../../../library/builtin/protocols/conversion.fl"),
        support::with_attrs(include_str!("../../../library/builtin/protocols/traits.fl")),
    )
}

#[test]
fn scalar_hashes_agree_across_execution_phases_and_integer_boundaries() {
    for (ty, literal, expected) in [
        ("bool", "false", 0),
        ("bool", "true", 1),
        ("char", "'A'", 65),
        ("char", "'\u{d7ff}'", 0xd7ff),
        ("char", "'\u{e000}'", 0xe000),
        ("char", "'🦀'", 0x1f980),
        ("char", "'\u{10ffff}'", 0x10ffff),
        ("u8", "255", 255),
        ("u16", "65535", 65535),
        ("u32", "4294967295", 4294967295),
        ("u64", "18446744073709551615", u64::MAX),
        ("i8", "-128", 128),
        ("i16", "-32768", 32768),
        ("i32", "-2147483648", 2147483648),
        ("i64", "-9223372036854775808", 1u64 << 63),
        ("u128", "18446744073709551616", 1),
        ("u128", "340282366920938463463374607431768211455", 0),
        (
            "i128",
            "-170141183460469231731687303715884105728",
            1u64 << 63,
        ),
        ("i128", "-1", 0),
    ] {
        let package = support::compile(&source(&format!(
            "fn run()->u64 {{let value:{ty}={literal};value.as(Hash).hash()}}\ncomptime fn staged()->u64 {{let value:{ty}={literal};value.as(Hash).comptime_hash()}}"
        )));
        let expected = Value::UnsignedInteger(expected.into());
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            expected,
            "{ty} {literal}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            expected,
            "{ty} {literal}"
        );
    }
}

#[test]
fn wide_hash_collisions_still_require_equality() {
    use comptime_executor::{Context, KeyProtocol};
    let package = support::compile(&source(""));
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "Eq"),
        (intrinsic::LangItem::Hash, "Hash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    let mut protocol = luna::comptime_keys::Protocol::prepare(
        &package,
        &items,
        &nir::Term::Primitive(nir::Primitive::U128),
        Default::default(),
    )
    .unwrap();
    let low = Value::UnsignedInteger(1);
    let high = Value::UnsignedInteger(1u128 << 64);
    let mut context = Context::new(1_000_000);
    assert_eq!(
        protocol.hash(&low, &mut context).unwrap(),
        protocol.hash(&high, &mut context).unwrap()
    );
    assert!(!protocol.equivalent(&low, &high, &mut context).unwrap());
    assert!(protocol.equivalent(&high, &high, &mut context).unwrap());
}

#[test]
fn scalar_hashes_lower_to_host_code_without_comptime_dispatch() {
    let package = support::compile(&source(
        "fn verify() {\nlet wide:u128=18446744073709551616;\nlet signed:i128=-170141183460469231731687303715884105728;\nlet wide_hash=wide.as(Hash).hash();assumes wide_hash == 1;\nlet signed_hash=signed.as(Hash).hash();assumes signed_hash == 9223372036854775808;\nlet bool_hash=true.as(Hash).hash();assumes bool_hash == 1;\n}",
    ));
    let protocol = |name: &str| {
        package
            .traits
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .declaration
    };
    let ir = luna::host::llvm_ir_with_protocols(
        &package,
        "verify",
        1_000_000,
        protocol("Copy"),
        protocol("Drop"),
    )
    .unwrap();
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

#[test]
fn character_bit_reads_do_not_authorize_reconstruction_or_writeback() {
    for body in [
        "let value:u32=65;value'bitvec.as(char)",
        "var value:char='A';value'bitvec[0..32]=55296;value",
    ] {
        let error = support::compile_with_protocols(
            &source(&format!("fn invalid()->char {{{body}}}")),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            error.contains("representation") || error.contains("unrestricted"),
            "{error}"
        );
    }
    let package = support::compile(&source(
        "fn read(value:char)->u64 {value'bitvec.as(u64)} fn verify() {let value=read('🦀');assumes value == 129408;}",
    ));
    let ir = luna::host::llvm_ir(&package, "verify", 1_000_000).unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn symbol_hashes_follow_utf8_content_in_both_phases_and_native_code() {
    // Published FNV-1a vectors, plus a separately computed UTF-8 vector.
    for (name, expected) in [
        ("a", 0xaf63dc4c8601ec8cu64),
        ("foobar", 0x85944171f73967e8u64),
        ("é", 0x0ac21707b7181e01u64),
    ] {
        let package = support::compile(&source(&format!(
            "fn hash(value:Symbol)->u64 {{value.as(Hash).hash()}} fn run()->u64 {{hash(.{name})}} comptime fn staged()->u64 {{let value:Symbol=.{name};value.as(Hash).comptime_hash()}} fn verify() {{let value=run();assumes value == {expected};}}"
        )));
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into()),
            "{name}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into()),
            "{name}"
        );
        let protocol = |name: &str| {
            package
                .traits
                .iter()
                .find(|item| item.name == name)
                .unwrap()
                .declaration
        };
        let ir = luna::host::llvm_ir_with_protocols(
            &package,
            "verify",
            1_000_000,
            protocol("Copy"),
            protocol("Drop"),
        )
        .unwrap();
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
}

#[test]
fn text_and_symbol_cache_keys_compare_content_and_recover_after_budget_exhaustion() {
    use comptime_executor::{Context, KeyProtocol};
    let package = support::compile(&source(""));
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "Eq"),
        (intrinsic::LangItem::Hash, "Hash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    for primitive in [nir::Primitive::Symbol, nir::Primitive::Str] {
        let value = |text: String| {
            if primitive == nir::Primitive::Symbol {
                Value::Symbol(text.into())
            } else {
                Value::String(text.into())
            }
        };
        let mut protocol = luna::comptime_keys::Protocol::prepare(
            &package,
            &items,
            &nir::Term::Primitive(primitive),
            Default::default(),
        )
        .unwrap();
        let first = value("é".repeat(128));
        let same = value("é".repeat(128));
        let different = value("e".repeat(128));
        assert!(protocol.hash(&first, &mut Context::new(1)).is_err());
        let mut context = Context::new(4_000_000);
        let hash = protocol.hash(&first, &mut context).unwrap();
        assert_eq!(hash, protocol.hash(&same, &mut context).unwrap());
        assert!(protocol.equivalent(&first, &same, &mut context).unwrap());
        assert!(
            !protocol
                .equivalent(&first, &different, &mut context)
                .unwrap()
        );
        // The empty name is admitted by the VM Symbol domain even though a source
        // symbol literal normally spells an identifier.
        assert_eq!(
            protocol.hash(&value("".to_owned()), &mut context).unwrap(),
            protocol.hash(&value(String::new()), &mut context).unwrap()
        );
    }
}

#[test]
fn target_sized_hashes_use_the_selected_target_width() {
    for width in (8u32..=128).step_by(8) {
        let mask = u128::MAX >> (128 - width);
        let sign = 1u128 << (width - 1);
        for (ty, literal, representation) in [
            ("usize", mask.to_string(), mask),
            ("isize", "-1".to_owned(), mask),
            ("isize", format!("-{sign}"), sign),
            ("isize", "0".to_owned(), 0),
        ] {
            let text = source(&format!(
                "fn run()->u64 {{let value:{ty}={literal};value.as(Hash).hash()}} comptime fn staged()->u64 {{let value:{ty}={literal};value.as(Hash).comptime_hash()}}"
            ));
            let package = support::compile_with_source_bindings_for_target(
                &text,
                &Default::default(),
                Default::default(),
                Some(nir::target::TargetData::new(width).unwrap()),
            )
            .unwrap();
            let expected = Value::UnsignedInteger(
                ((representation as u64) ^ ((representation >> 64) as u64)).into(),
            );
            assert_eq!(
                luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
                expected,
                "{ty} {width}"
            );
            assert_eq!(
                luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
                expected,
                "{ty} {width}"
            );
        }
    }
}

#[test]
fn target_sized_hashes_execute_on_the_native_target() {
    let target = luna::host::Toolchain::default().target_layout().unwrap();
    let mask = u128::MAX >> (128 - target.pointer_bits);
    let expected = (mask as u64) ^ ((mask >> 64) as u64);
    let text = source(&format!(
        "fn verify() {{let signed:isize=-1;let unsigned:usize={mask};let first=signed.as(Hash).hash();let second=unsigned.as(Hash).hash();assumes first == {expected} and second == {expected};}}"
    ));
    let package = support::compile_with_source_bindings_for_target(
        &text,
        &Default::default(),
        Default::default(),
        Some(nir::target::TargetData::new(target.pointer_bits).unwrap()),
    )
    .unwrap();
    let ir = luna::host::llvm_ir(&package, "verify", 1_000_000).unwrap();
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

#[test]
fn text_hashes_and_equality_observe_complete_utf8_contents() {
    for (literal, expected) in [
        ("\"\"", 0xcbf29ce484222325u64),
        ("\"a\"", 0xaf63dc4c8601ec8cu64),
        ("\"é\"", 0x0ac21707b7181e01u64),
        ("\"a\\0b\"", 0xe5d29919042666b2u64),
    ] {
        let package = support::compile(&source(&format!(
            "fn run()->u64 {{{literal}.as(Hash).hash()}} comptime fn staged()->u64 {{{literal}.as(Hash).comptime_hash()}} fn verify() {{let value=run();assumes value == {expected};}}"
        )));
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into())
        );
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into())
        );
        let ir = luna::host::llvm_ir(&package, "verify", 1_000_000).unwrap();
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
    let body = r#"fn run()->bool {
        "é".as(Eq).eq("é") and "é".as(Eq).ne("é")
        and "a\0b".as(Eq).ne("a\0c") and "".as(Eq).ne("a")
    }"#;
    let staged = body
        .replace("fn run", "comptime fn staged")
        .replace(".eq(", ".comptime_eq(")
        .replace(".ne(", ".comptime_ne(");
    let package = support::compile(&source(&format!(
        "{body}{staged} fn verify() {{let valid=run();assumes valid;}}"
    )));
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
        Value::Bool(true)
    );
    let ir = luna::host::llvm_ir(&package, "verify", 1_000_000).unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn string_hash_does_not_bypass_resource_provenance_checks() {
    for body in ["fn run()->u64 {let value:str=\"foobar\";value.as(Hash).hash()}"] {
        let package = support::compile(&source(body));
        let protocol = |name: &str| {
            package
                .traits
                .iter()
                .find(|item| item.name == name)
                .unwrap()
                .declaration
        };
        let error = luna::host::llvm_ir_with_protocols(
            &package,
            "run",
            1_000_000,
            protocol("Copy"),
            protocol("Drop"),
        )
        .unwrap_err();
        assert!(error.contains("string-view memory provenance"), "{error}");
    }
}

#[test]
fn root_dispatch_preserves_generic_parents_of_concrete_implementations() {
    let package = support::compile(
        "trait Parent {} impl Parent for T where T {}\n\
         trait Child :- Parent {} impl Child for bool {}\n\
         comptime fn run()->bool {true}",
    );
    let root = iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    for invocation in [false, true] {
        let materialize = if invocation {
            elaboration::materialization::dispatch_instances_for_invocation
        } else {
            elaboration::materialization::dispatch_instances_for_roots
        };
        assert!(
            materialize(
                &package,
                &[root.clone()],
                elaboration::ptc::Budget {
                    steps: 0,
                    ..Default::default()
                }
            )
            .is_err()
        );
        let mut prepared = package.clone();
        prepared.dispatch_instances =
            materialize(&package, &[root.clone()], Default::default()).unwrap();
        let parent = package
            .traits
            .iter()
            .find(|t| t.name == "Parent")
            .unwrap()
            .declaration;
        assert!(
            prepared
                .dispatch_instances
                .iter()
                .any(|i| i.interface == parent
                    && i.subject == nir::Term::Primitive(nir::Primitive::Bool))
        );
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&prepared, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instantiated(root.declaration, vec![], vec![])
                .unwrap(),
            Value::Bool(true)
        );
        let mut missing = package.clone();
        missing
            .implementations
            .retain(|i| i.interface != Some(parent));
        assert!(materialize(&missing, &[root.clone()], Default::default()).is_err());
    }
}

#[test]
#[ignore = "requires trait-method callable bytecode with interface contracts"]
fn shipped_text_hash_runs_its_loop_in_bytecode() {
    use comptime_interpretor::{Budget, Interpreter};
    let package = support::compile(&source(
        "comptime fn staged()->u64 {\"é\".as(Hash).comptime_hash()}",
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(support::id(&package, "staged"), vec![]).unwrap();
    assert_eq!(result, Value::UnsignedInteger(0x0ac21707b7181e01));
    assert_eq!(
        vm.bytecode_invocations(),
        2,
        "wrapper and shipped hash body must both compile"
    );
}

#[test]
fn shipped_text_hash_body_uses_bytecode_for_all_bytes() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{Primitive, Term};
    let package = support::compile(&source(""));
    let method=package.functions.iter().find(|function| {
        function.phase==nir::program::FunctionPhase::Comptime
        && matches!(function.signature.callable_signature(), Term::Function {parameters,result}
            if parameters.len()==1 && parameters[0].ty==Term::Primitive(Primitive::Str)
            && **result==Term::Primitive(Primitive::U64))
    }).expect("shipped comptime string hash").declaration;
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for (text, expected) in [
        ("", 0xcbf29ce484222325u128),
        ("é", 0x0ac21707b7181e01),
        ("a\0b", 0xe5d29919042666b2),
    ] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(method, vec![Value::String(text.into())]).unwrap(),
            Value::UnsignedInteger(expected)
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}
