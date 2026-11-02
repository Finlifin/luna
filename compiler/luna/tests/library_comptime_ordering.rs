//! Phase-specific total ordering through shipped protocols and ordinary calls.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

fn compile(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} use meta.{{List, ComptimeEq, ComptimeOrd}}; use meta.builtin.*; {body}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn exact_integer_ordering_and_default_methods_work_in_both_engines() {
    let mut source = String::new();
    for ty in [
        "u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32", "i64", "i128", "Integer",
    ] {
        source.push_str(&format!(
            "comptime fn check_{ty}()->bool {{ let a:{ty}=1;let b:{ty}=2;
             (a.cmp(b) matches .less) and (b.cmp(a) matches .greater)
             and (a.cmp(a) matches .equal) and a.lt(b) and a.le(a)
             and b.gt(a) and b.ge(b) and not b.le(a) and not a.ge(b) }}"
        ));
    }
    let package = compile(&source).unwrap();
    for mut vm in [
        Interpreter::for_comptime(&package, Budget::default()).unwrap(),
        Interpreter::for_comptime_tree(&package, Budget::default()).unwrap(),
    ] {
        for ty in [
            "u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32", "i64", "i128", "Integer",
        ] {
            assert_eq!(
                vm.invoke(support::id(&package, &format!("check_{ty}")), vec![])
                    .unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn text_order_is_lexicographic_without_unicode_normalization() {
    let package = compile(
        r#"
        comptime fn check()->bool {
            "".lt("a") and "a".lt("ab") and "ab".gt("a")
            and "é".gt("z") and "é".lt("é") and "🦀".gt("é")
            and "same".le("same") and "same".ge("same")
            and false.lt(true) and true.gt(false)
        }
    "#,
    )
    .unwrap();
    for mut vm in [
        Interpreter::for_comptime(&package, Budget::default()).unwrap(),
        Interpreter::for_comptime_tree(&package, Budget::default()).unwrap(),
    ] {
        assert_eq!(
            vm.invoke(support::id(&package, "check"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn floating_point_has_no_implicit_total_order() {
    let failure = compile("comptime fn invalid()->Order { let a:f32=1.0; a.cmp(a) }").unwrap_err();
    assert!(!failure.contains("parsing failed"), "{failure}");
}

#[test]
fn custom_ordering_composes_through_generic_parent_bounds() {
    let package = compile(
        r#"
        struct Key { rank:u8 }
        comptime impl ComptimeEq for Key {
            comptime fn eq(self, other:Self)->bool {self.rank == other.rank}
        }
        comptime impl ComptimeOrd for Key {
            comptime fn cmp(self, other:Self)->Order {self.rank.cmp(other.rank)}
        }
        comptime fn increasing(left:T, right:T)->bool
        where T :- ComptimeOrd
        {
            left.lt(right) and left.ne(right)
        }
        comptime fn check()->bool {increasing(Key {rank:1}, Key {rank:2})}
    "#,
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "check"), vec![]).unwrap(),
        Value::Bool(true)
    );
}
