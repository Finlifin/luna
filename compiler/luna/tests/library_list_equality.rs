//! Generic list equality must dispatch the element protocol and short-circuit.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};

fn compile(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} use meta.{{List, ComptimeEq}}; use meta.builtin.Integer; {body}",
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
fn list_equality_uses_element_methods_and_stops_before_unneeded_comparisons() {
    let package = compile(
        "struct Key { value: Integer }
        comptime impl ComptimeEq for Key {
            comptime fn eq(self, other: Self) -> bool {
                let observed = 1.div_trunc(if self.value == 99 {0} else {1});
                self.value.rem_euclid(10) == other.value.rem_euclid(10) and observed == 1
            }
        }
        comptime fn equal()->bool {
            let a:List<Key> = [Key {value:12}];
            let b:List<Key> = [Key {value:22}];
            a.eq(b) and not a.ne(b)
        }
        comptime fn short_circuit()->bool {
            let a:List<Key> = [Key {value:1},Key {value:99}];
            let b:List<Key> = [Key {value:2},Key {value:99}];
            a.ne(b)
        }
        comptime fn lengths()->bool {
            let a:List<Key> = [Key {value:99}];
            let b:List<Key> = [Key {value:99},Key {value:99}];
            a.ne(b)
        }
        comptime fn empty()->bool {
            let a:List<Key> = []; a.eq(a)
        }
        comptime fn fail()->bool {
            let a:List<Key> = [Key {value:99}]; a.eq(a)
        }
        comptime fn nested()->bool {
            let a:List<List<Integer>> = [[],[1,2]];
            let b:List<List<Integer>> = [[],[1,2]];
            let c:List<List<Integer>> = [[1],[]];
            a.eq(b) and a.ne(c)
        }
        struct MutableKey { value: Integer }
        comptime impl ComptimeEq for MutableKey {
            comptime fn eq(self, other: Self)->bool {
                let previous = self.value;
                self.value = 99;
                previous == other.value
            }
        }
        comptime fn snapshots()->bool {
            let a:List<MutableKey> = [MutableKey {value:1}];
            a.eq(a) and a[0].value == 1
        }",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for name in [
        "equal",
        "short_circuit",
        "lengths",
        "empty",
        "nested",
        "snapshots",
    ] {
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::Bool(true),
            "{name}"
        );
    }
    assert_eq!(
        vm.invoke(support::id(&package, "fail"), vec![])
            .unwrap_err()
            .kind,
        ErrorKind::DivisionByZero
    );
    assert_eq!(
        vm.invoke(support::id(&package, "equal"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_equality_requires_the_comptime_element_protocol() {
    for declaration in [
        "struct Key { value: bool }",
        "struct Key { value: bool }
         trait RuntimeEq {fn eq(self, other:Self)->bool;}
         impl RuntimeEq for Key {fn eq(self, other:Self)->bool {true}}",
    ] {
        let result = compile(&format!(
            "{declaration} comptime fn bad(a:List<Key>,b:List<Key>)->bool {{a.as(ComptimeEq).eq(b)}}"
        ));
        let failure = result
            .err()
            .expect("accepted missing comptime element evidence");
        assert!(
            failure.contains("trait") || failure.contains("method"),
            "{failure}"
        );
    }
}
