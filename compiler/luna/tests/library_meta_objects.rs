//! Exercise the public meta Object interface with the repository library source.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Interpreter, Value};
use nir::{Primitive, Term};

const META: &str = include_str!("../../../library/meta/lib.fl");

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

fn invoke(source: &str) -> Result<Value, String> {
    let package = compile(&format!("mod meta {{ {META} }} {source}"))?;
    let mut vm =
        Interpreter::for_comptime(&package, Default::default()).map_err(|e| e.to_string())?;
    vm.invoke(support::id(&package, "run"), vec![])
        .map_err(|e| e.to_string())
}

#[test]
fn indexed_object_queries_preserve_order_domains_and_optional_layers() {
    let value = invoke(
        "
        comptime fn run()->bool {
            let absent:?bool=null;
            let nested={enabled:true};
            let object={flag:true, 'x', flag:false, absent, nested};
            let first=meta.object_value<bool>(object,0);
            let last=meta.object_value<bool>(object,2);
            let missing=meta.object_value<bool>(object,1);
            let layered=meta.object_value<?bool>(object,3);
            let inner=meta.object_value<meta.Object>(object,4);
            meta.object_length(object)==5
                and (meta.object_key(object,0) match {key?=>key==.flag,null=>false})
                and (meta.object_key(object,1) match {_?=>false,null=>true})
                and (meta.object_key(object,2) match {key?=>key==.flag,null=>false})
                and meta.ObjectEntryType(object,1)==.char'builtin
                and (first match {value?=>value,null=>false})
                and (last match {value?=>not value,null=>false})
                and (missing match {_?=>false,null=>true})
                and (layered match {value?=>value match {_?=>false,null=>true},null=>false})
                and (inner match {value?=>meta.object_length(value)==1,null=>false})
        }
    ",
    )
    .unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn object_queries_preserve_type_identity_and_evaluate_inputs_once_in_order() {
    let value=invoke("
        struct A {} struct B {}
        comptime fn make(state:*bool)->meta.Object {state.* = not state.*; let item=A {}; let result={stored:item}; result}
        comptime fn index(state:*bool)->.Integer'builtin {
            if state.* {state.* = false; return 0;} 9
        }
        comptime fn run()->bool {
            var state=false;
            let got=meta.object_value<A>(make(state.ref),index(state.ref));
            let item=A {};
            let object={stored:item, kind:.u8'builtin};
            not state and (got match {_?=>true,null=>false})
                and (meta.object_value<B>(object,0) match {_?=>false,null=>true})
                and (meta.object_value<.Type'builtin>(object,1) match {T?=>T==.u8'builtin,null=>false})
        }
    ").unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn object_queries_reject_bounds_and_runtime_transport() {
    for index in ["-1", "1", "99999999999999999999999999999999999"] {
        for query in ["object_key", "ObjectEntryType", "object_value<bool>"] {
            let error = invoke(&format!(
                "comptime fn run()->bool {{let value={{true}}; meta.{query}(value,{index}); true}}"
            ))
            .unwrap_err();
            assert!(error.contains("IndexOutOfBounds"), "{error}");
        }
    }
    let error = compile(&format!(
        "mod meta {{ {META} }} fn bad()->.Integer'builtin {{meta.object_length({{true}})}}"
    ))
    .unwrap_err();
    assert!(
        error.contains("comptime") || error.contains("phase"),
        "{error}"
    );
    assert_eq!(
        invoke("comptime fn run()->bool {meta.object_length({})==0}").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn object_query_argument_return_skips_later_inputs() {
    assert_eq!(
        invoke(
            "comptime fn run()->bool {
        meta.object_key(do {return true;}, do {while true {} 0}); false
    }"
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn code_generators_can_reconstruct_nested_object_data_without_normal_term_erasure() {
    let package = compile(&format!(
        "mod meta {{ {META} }}
        comptime fn generate()-><|-bool> {{
            let nested={{flag:true}};
            let object={{nested:nested, tag:.sample}};
            let selected=meta.object_value<meta.Object>(object,0);
            let answer=selected match {{
                inner?=>meta.object_value<bool>(inner,0) match {{flag?=>flag,null=>false}},
                null=>false,
            }};
            '(answer)
        }}
        fn run()->bool {{~<generate()>}}
    "
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );

    let failure = compile(&format!(
        "mod meta {{ {META} }}
        comptime fn generate()-><|-bool> {{
            var flag=true; let object={{pointer:flag.ref}}; '(true)
        }} fn run()->bool {{~<generate()>}}
    "
    ))
    .unwrap_err();
    assert!(
        failure.contains("frozen") || failure.contains("pointer"),
        "{failure}"
    );
}

#[test]
fn repository_package_exports_object_for_user_attribute_constructors() {
    let directory = std::env::temp_dir().join(format!("luna-meta-object-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.fl");
    std::fs::write(
        &input,
        "use meta.prelude.*;
        pure comptime fn marker() -> Object { let value = { enabled: true }; value }
        ^marker() fn example() {}",
    )
    .unwrap();
    let library =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/meta/lib.fl");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .args(["--packages-v1", "--package", "1", "101", "app"])
        .arg(&input)
        .args(["--package", "2", "202", "meta"])
        .arg(&library)
        .args([
            "--dependency",
            "1",
            "meta",
            "2",
            "--primary",
            "1",
            "--meta-list-type",
            "meta.List",
            "--emit",
            "attributes",
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = String::from_utf8_lossy(&output.stdout);
    assert_eq!(dump.matches("(evaluated-attribute ").count(), 1, "{dump}");
    assert!(dump.contains("(property \"enabled\")"), "{dump}");
    assert!(dump.contains("(bool true)"), "{dump}");
}

#[test]
fn public_object_preserves_order_duplicate_keys_and_nested_constants() {
    let package = compile(&format!(
        "mod meta {{ {META} }}
         use meta.prelude.*;
         const saved = {{ flag: true, 'x', flag: false }};
         comptime fn identity(value: meta.Object) -> Object {{ value }}
         comptime fn metadata() -> Object {{
             let result = {{ nested: saved, mode: .c }};
             identity(result)
         }}"
    ))
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm
        .invoke(support::id(&package, "metadata"), vec![])
        .unwrap();
    let Value::Object(object) = value else {
        panic!("expected Object")
    };
    assert_eq!(object.entries().len(), 2);
    let nested = &object.entries()[0];
    assert_eq!(nested.key(), Some("nested"));
    assert_eq!(nested.value().ty, Term::Primitive(Primitive::Object));
    let Value::Object(nested) = &nested.value().value else {
        panic!("expected nested Object")
    };
    assert_eq!(
        nested
            .entries()
            .iter()
            .map(|entry| entry.key())
            .collect::<Vec<_>>(),
        vec![Some("flag"), None, Some("flag")]
    );
    assert_eq!(nested.entries()[0].value().value, Value::Bool(true));
    assert_eq!(nested.entries()[1].value().value, Value::Char('x'));
    assert_eq!(nested.entries()[2].value().value, Value::Bool(false));
    assert_eq!(object.entries()[1].key(), Some("mode"));
    assert_eq!(
        object.entries()[1].value().ty,
        Term::Primitive(Primitive::Symbol)
    );
    assert_eq!(object.entries()[1].value().value, Value::Symbol("c".into()));
}

#[test]
fn public_alias_does_not_allow_runtime_transport_or_wrong_values() {
    for body in [
        "fn invalid(value: Object) {}",
        "fn invalid() -> Object { let value = { flag: true }; value }",
        "comptime fn invalid() -> Object { true }",
    ] {
        let source = format!("mod meta {{ {META} }} use meta.prelude.*; {body}");
        assert!(compile(&source).is_err(), "{body}");
    }
}
