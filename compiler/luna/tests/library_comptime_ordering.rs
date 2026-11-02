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

#[test]
fn lists_compare_common_prefixes_before_lengths_and_compose_recursively() {
    let package = compile(
        r#"
        comptime fn check()->bool {
            let empty:List<u8> = [];
            let short:List<u8> = [1];
            let long:List<u8> = [1,2];
            let larger:List<u8> = [2];
            let nested:List<List<u8>> = [[],[1],[1,2]];
            (empty.cmp(empty) matches .equal) and empty.lt(short)
                and short.lt(long) and long.gt(short) and long.lt(larger)
                and larger.gt(long) and long.eq([1,2]) and long.le([1,2])
                and nested.lt([[],[1],[2]]) and nested.ge([[],[1],[1,2]])
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
fn list_ordering_uses_element_protocols_short_circuits_and_keeps_snapshots() {
    let package = compile(
        r#"
        struct Key {rank:u8, ignored:bool}
        comptime impl ComptimeEq for Key {
            comptime fn eq(self,other:Self)->bool {self.rank == other.rank}
        }
        comptime impl ComptimeOrd for Key {
            comptime fn cmp(self,other:Self)->Order {
                assumes self.rank != 99;
                let rank=self.rank;
                let other_rank=other.rank;
                self.rank=0;
                other.rank=0;
                rank.cmp(other_rank)
            }
        }
        comptime fn check()->bool {
            let left:List<Key> = [Key {rank:2,ignored:false}, Key {rank:99,ignored:false}];
            let right:List<Key> = [Key {rank:1,ignored:true}, Key {rank:99,ignored:true}];
            let equal_left:List<Key> = [Key {rank:1,ignored:false}];
            let equal_right:List<Key> = [Key {rank:1,ignored:true}];
            (left.cmp(right) matches .greater)
                and left[0].rank == 2 and right[0].rank == 1
                and (equal_left.cmp(equal_right) matches .equal)
        }
        comptime fn fail()->Order {
            let left:List<Key> = [Key {rank:99,ignored:false}];
            left.cmp(left)
        }
        comptime fn empty()->bool {
            let left:List<Key> = [];
            let right:List<Key> = [Key {rank:99,ignored:false}];
            left.lt(right)
        }
    "#,
    )
    .unwrap();
    for mut vm in [
        Interpreter::for_comptime(&package, Budget::default()).unwrap(),
        Interpreter::for_comptime_tree(&package, Budget::default()).unwrap(),
    ] {
        assert!(vm.invoke(support::id(&package, "fail"), vec![]).is_err());
        for name in ["check", "empty"] {
            assert_eq!(
                vm.invoke(support::id(&package, name), vec![]).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn list_ordering_requires_element_ordering_even_for_empty_lists() {
    let failure = compile(
        r#"
        struct Unordered {}
        comptime impl ComptimeEq for Unordered {
            comptime fn eq(self,other:Self)->bool {true}
        }
        comptime fn invalid()->Order {
            let values:List<Unordered> = [];
            values.cmp(values)
        }
    "#,
    )
    .unwrap_err();
    assert!(!failure.contains("parsing failed"), "{failure}");
}

#[test]
fn symbols_order_by_content_and_compose_with_list_sorting() {
    let package = compile(
        "comptime fn less(left:Symbol, right:Symbol)->bool { left.lt(right) }
        comptime fn equal(left:Symbol, right:Symbol)->bool {
            (left.cmp(right) matches .equal) and left.eq(right)
        }
        comptime fn sorted()->bool {
            let tags:List<Symbol> = [.beta, .alpha, .alphabet, .alpha];
            tags.sort_by(|left,right| left.lt(right)).eq([.alpha, .alpha, .alphabet, .beta])
                and tags[0].eq(.beta)
        }",
    )
    .unwrap();
    for mut vm in [
        Interpreter::for_comptime(&package, Budget::default()).unwrap(),
        Interpreter::for_comptime_tree(&package, Budget::default()).unwrap(),
    ] {
        for left in ["", "a", "ab", "z", "e\u{301}", "é", "🦀"] {
            for right in ["", "a", "ab", "z", "e\u{301}", "é", "🦀"] {
                let values = || vec![Value::Symbol(left.into()), Value::Symbol(right.into())];
                assert_eq!(
                    vm.invoke(support::id(&package, "less"), values()).unwrap(),
                    Value::Bool(left.as_bytes() < right.as_bytes()),
                    "{left:?}, {right:?}"
                );
                assert_eq!(
                    vm.invoke(support::id(&package, "equal"), values()).unwrap(),
                    Value::Bool(left == right),
                    "{left:?}, {right:?}"
                );
            }
        }
        assert_eq!(
            vm.invoke(support::id(&package, "sorted"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}
