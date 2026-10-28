//! Transient Object slots retain snapshots and check payload resource evidence.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> nir::program::Package {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }}
            trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            copy: Some("input.Duplicate"),
            drop: Some("input.Retire"),
            ..Default::default()
        },
    )
    .unwrap()
}

fn invoke(package: &nir::program::Package) -> Result<Value, String> {
    luna::invocation::invoke_with_protocols(package, "run", "Duplicate", "Retire", 1_000_000)
}

#[test]
fn object_pointer_reads_and_writes_preserve_nested_snapshots() {
    let package = compile("
        pure comptime fn Identity<T>->.Type'builtin {T}
        comptime fn run()->bool {
            let absent:?bool=null;
            let types:meta.List<.Type'builtin> = [Identity<bool>,bool];
            var object={flag:true, flag:false, absent:absent, nested:{flag:true}, types:types};
            let pointer=object.ref;
            let saved=pointer.*;
            let earlier=meta.Object.get<bool>(object,do {pointer.* = {flag:true}; .flag});
            let read_back=pointer.*;
            pointer.* = {};
            (earlier matches false?) and meta.Object.length(object)==0
                and (meta.Object.get<bool>(read_back,.flag) matches true?)
                and meta.Object.length(saved)==5
                and (meta.Object.get<?bool>(saved,.absent) match {v?=>v matches null,null=>false})
                and (meta.Object.get<meta.Object>(saved,.nested) match {v?=>meta.Object.get<bool>(v,.flag) matches true?,null=>false})
                and (meta.Object.get<meta.List<.Type'builtin>>(saved,.types) match {v?=>v[0]==Identity<bool>,null=>false})
        }
    ");
    assert_eq!(invoke(&package).unwrap(), Value::Bool(true));
}

#[test]
fn nominal_object_payloads_require_copy_without_drop() {
    let source = "struct Item {flag:bool} IMPL
        comptime fn run()->bool {
            let item=Item {flag:true};
            var object={item:item};
            let pointer=object.ref;
            let saved=pointer.*;
            pointer.* = {flag:false};
            meta.Object.get<Item>(saved,.item) match {value?=>value.flag,null=>false}
        }";
    let package = compile(&source.replace("IMPL", "impl Duplicate for Item {}"));
    assert_eq!(invoke(&package).unwrap(), Value::Bool(true));
    for implementation in [
        "",
        "impl Duplicate for Item {} impl Retire for Item {fn drop(*self) {}}",
    ] {
        let package = compile(&source.replace("IMPL", implementation));
        let error = invoke(&package).unwrap_err();
        assert!(error.contains("Unsupported"), "{error}");
    }
}

#[test]
fn writes_check_old_new_and_inactive_payload_domains() {
    for body in [
        "var object={item:Item {flag:true}}; object.ref.* = {flag:false};",
        "var object={flag:false}; object.ref.* = {item:Item {flag:true}};",
        "let absent:?Item=null; var object={item:absent}; object.ref.* = {};",
        "let absent:?Item=null; var object={}; object.ref.* = {nested:{item:absent}};",
        "let empty:meta.List<Item> = []; var object={items:empty}; object.ref.* = {};",
    ] {
        let package = compile(&format!(
            "struct Item {{flag:bool}} comptime fn run()->bool {{{body} true}}"
        ));
        let error = invoke(&package).unwrap_err();
        assert!(error.contains("Unsupported"), "{body}: {error}");
    }
}

#[test]
fn list_storage_cannot_bypass_object_payload_permissions() {
    for body in [
        "var values:meta.List<Item> = [Item {flag:true}]; let copied=values.ref.*;",
        "var values:meta.List<Item> = [Item {flag:true}]; values.ref.* = [];",
        "var values:meta.List<Item> = []; values.ref.* = [Item {flag:true}];",
        "var values:meta.List<Item> = []; values.append(Item {flag:true});",
        "var values:meta.List<meta.Object> = [{item:Item {flag:true}}]; let copied=values.ref.*;",
        "var values:meta.List<meta.Object> = []; let item={item:Item {flag:true}}; values.append(item);",
        "let absent:?Item=null; var values:meta.List<?Item> = [absent]; let copied=values.ref.*;",
    ] {
        let package = compile(&format!(
            "struct Item {{flag:bool}} comptime fn run()->bool {{{body} true}}"
        ));
        let error = invoke(&package).unwrap_err();
        assert!(error.contains("Unsupported"), "{body}: {error}");
    }
}

#[test]
fn list_storage_copy_payloads_preserve_snapshots_and_reject_drop() {
    let source = "struct Item {flag:bool} impl Duplicate for Item {} DROP
        comptime fn run()->bool {
            var items:meta.List<Item> = [Item {flag:true}];
            let pointer=items.ref;
            let saved=pointer.*;
            pointer.*.append(Item {flag:false});
            let extended=pointer.*;
            pointer.* = [];
            items.length()==0 and saved.length()==1 and saved[0].flag
                and extended.length()==2 and not extended[1].flag
        }";
    assert_eq!(
        invoke(&compile(&source.replace("DROP", ""))).unwrap(),
        Value::Bool(true)
    );
    let error = invoke(&compile(
        &source.replace("DROP", "impl Retire for Item {fn drop(*self) {}}"),
    ))
    .unwrap_err();
    assert!(error.contains("Unsupported"), "{error}");
}
