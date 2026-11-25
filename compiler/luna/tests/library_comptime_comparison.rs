#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_executor::{Context, KeyProtocol};
use comptime_interpretor::Value;

fn source() -> String {
    format!(
        r#"
        {}
        use builtin.u8;
        trait Hash {{
            fn hash(self)->u8;
        }}
        pure comptime trait ComptimeHash {{
            comptime fn hash(self)->u8;
        }}
        impl Hash for bool {{fn hash(self)->u8 {{0}}}}
        comptime impl ComptimeHash for bool {{
            comptime fn hash(self)->u8 {{if self {{1}} else {{0}}}}
        }}
        comptime fn compare()->bool {{
            true.as(ComptimeEq).eq(true) and true.as(ComptimeEq).ne(false)
        }}
        fn compare_runtime()->bool {{let a = true; let b = false; a.as(Eq).eq(a.ref) and a.as(Eq).ne(b.ref)}}
        "#,
        support::with_attrs(include!("support/comparison_source.rs"))
    )
}

fn prepare(package: &nir::program::Package) -> Result<luna::comptime_keys::Protocol, String> {
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
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
    items.bind_source_traits(package, &bindings).unwrap();
    luna::comptime_keys::Protocol::prepare(
        package,
        &items,
        &nir::Term::Primitive(nir::Primitive::Bool),
        Default::default(),
    )
}

#[test]
fn library_equality_supports_runtime_and_comptime_members() {
    let package = support::compile(&source());
    assert_eq!(
        luna::invocation::invoke(&package, "compare", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "compare_runtime", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mut protocol = prepare(&package).unwrap();
    let mut context = Context::new(100_000);
    assert!(
        protocol
            .equivalent(&Value::Bool(true), &Value::Bool(true), &mut context)
            .unwrap()
    );
    assert!(
        !protocol
            .equivalent(&Value::Bool(true), &Value::Bool(false), &mut context)
            .unwrap()
    );
    assert_ne!(
        protocol.hash(&Value::Bool(true), &mut context).unwrap(),
        protocol.hash(&Value::Bool(false), &mut context).unwrap()
    );
}

#[test]
fn library_comptime_equality_covers_fixed_and_target_integer_widths() {
    for ty in [
        "u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32", "i64", "i128", "usize", "isize",
    ] {
        let text = format!(
            "{} comptime fn check()->bool {{let left:builtin.{ty}=1;let right:builtin.{ty}=2;left.as(ComptimeEq).eq(left) and left.as(ComptimeEq).ne(right)}}",
            source()
        );
        for width in [32, 64] {
            let package = support::compile_with_source_bindings_for_target(
                &text,
                &Default::default(),
                Default::default(),
                Some(nir::target::TargetData::new(width).unwrap()),
            )
            .unwrap();
            assert_eq!(
                luna::invocation::invoke(&package, "check", 100_000).unwrap(),
                Value::Bool(true),
                "{ty}, {width} bits"
            );
        }
    }
}

#[test]
fn canonical_comptime_key_member_must_pass_phase_and_uniqueness_checks() {
    let mut package = support::compile(&source());
    for name in ["ComptimeEq", "ComptimeHash"] {
        let mut invalid = package.clone();
        let interface = invalid.traits.iter_mut().find(|t| t.name == name).unwrap();
        interface
            .methods
            .iter_mut()
            .find(|m| m.name == "eq" || m.name == "hash")
            .unwrap()
            .phase = nir::program::TraitPhase::Runtime;
        assert!(
            prepare(&invalid)
                .err()
                .unwrap()
                .contains("comptime trait membership")
        );
    }
    let interface = package
        .traits
        .iter_mut()
        .find(|t| t.name == "ComptimeEq")
        .unwrap();
    let member = interface
        .methods
        .iter()
        .find(|m| m.name == "eq")
        .unwrap()
        .clone();
    interface.methods.push(member);
    assert!(prepare(&package).is_err());
}
