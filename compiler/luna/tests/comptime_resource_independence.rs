//! Runtime resource bindings do not authorize comptime values or key protocols.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_executor::{Context, KeyProtocol};
use comptime_interpretor::Value;
use intrinsic::{LangItem, LangItems};
use luna::{comptime_keys::Protocol, memoization::Invocation};
use nir::{Primitive, Term, program::Package};

fn bind(items: &mut LangItems, package: &Package, item: LangItem, name: &str) {
    let declaration = package
        .traits
        .iter()
        .find(|t| t.name == name)
        .unwrap()
        .declaration;
    items
        .bind_source_traits(package, &[(item, declaration)])
        .unwrap();
}

#[test]
fn keys_and_invocations_are_independent_of_partial_runtime_bindings() {
    let package = sysroot::compile(
        "pure comptime fn Select(value: bool) -> Type { if value { bool } else { u8 } }
         pure comptime fn flip(value: bool) -> bool { not value }",
    )
    .unwrap();
    let entry = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for mask in 0..4 {
        let mut resources = LangItems::new();
        if mask & 1 != 0 {
            bind(&mut resources, &package, LangItem::Copy, "Copy");
        }
        if mask & 2 != 0 {
            bind(&mut resources, &package, LangItem::Drop, "Drop");
        }
        // Runtime protocols cannot replace the source Eq/Hash requirements.
        assert!(
            Protocol::prepare(
                &package,
                &resources,
                &Term::Primitive(Primitive::Bool),
                Default::default()
            )
            .is_err()
        );
        // A non-Type result needs neither key protocols nor runtime resources.
        let mut flip = Invocation::prepare(
            &package,
            &resources,
            entry("flip"),
            vec![],
            4,
            Default::default(),
        )
        .unwrap();
        for value in [false, true, false] {
            let result = flip
                .invoke(vec![Value::Bool(value)], &mut Context::new(1_000_000))
                .unwrap();
            assert_eq!(*result.value, Value::Bool(!value));
            assert!(!result.reused);
        }
        assert_eq!(flip.cached_results(), 0);

        let mut items = resources;
        bind(&mut items, &package, LangItem::Eq, "ComptimeEq");
        bind(&mut items, &package, LangItem::Hash, "ComptimeHash");
        let mut protocol = Protocol::prepare(
            &package,
            &items,
            &Term::Primitive(Primitive::Bool),
            Default::default(),
        )
        .unwrap();
        let mut context = Context::new(1_000_000);
        let yes = protocol.admit(Value::Bool(true), &mut context).unwrap();
        let no = protocol.admit(Value::Bool(false), &mut context).unwrap();
        let hash = protocol.hash(&yes, &mut context).unwrap();
        assert!(protocol.equivalent(&yes, &yes, &mut context).unwrap());
        assert!(!protocol.equivalent(&yes, &no, &mut context).unwrap());
        assert_eq!(protocol.hash(&yes, &mut context).unwrap(), hash);

        let mut select = Invocation::prepare(
            &package,
            &items,
            entry("Select"),
            vec![],
            4,
            Default::default(),
        )
        .unwrap();
        for (value, expected, reused) in [
            (true, Primitive::Bool, false),
            (false, Primitive::U8, false),
            (true, Primitive::Bool, true),
        ] {
            let result = select
                .invoke(vec![Value::Bool(value)], &mut Context::new(1_000_000))
                .unwrap();
            assert_eq!(*result.value, Value::Type(Term::Primitive(expected)));
            assert_eq!(result.reused, reused);
        }
        assert_eq!(select.cached_results(), 2);
    }
}
