//! List observation retains checked element identity and invocation work.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Event, Interpreter, Value};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{{}}} typealias Type=.Type'builtin; typealias Integer=.Integer'builtin; typealias Byte=.u8'builtin; {source}",
            include!("support/meta_source.rs")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn readonly_list_elements_supply_local_type_annotations() {
    for body in [
        "let types:meta.List<Type> = [bool,Byte];let T=types[0];let value:T=true;value",
        "let types:meta.List<Type> = [Byte,bool];let index:Integer=1;let T=types[index];let value:T=true;value",
        "let types:meta.List<Type> = [bool];let alias=types;let T=alias[0];let value:T=true;value",
        "let outer:meta.List<meta.List<Type>> = [[bool]];let types=outer[0];let T=types[0];let value:T=true;value",
        "let types=do {let types:meta.List<Type> = [bool,Byte];types};let T=types[0];let value:T=true;value",
    ] {
        let source = format!("comptime fn run()->bool {{{body}}}");
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn partial_lists_keep_known_types_without_skipping_other_initializers() {
    for body in [
        "let types:meta.List<Type> = [bool,Other()];let T=types[0];let value:T=true;value",
        "let records:meta.List<Descriptor> = [Descriptor {ty:bool,flag:observe(true)}];let d=records[0];let value:d.ty=d.flag;value",
    ] {
        let source = format!(
            "struct Descriptor {{ty:Type,flag:bool}} comptime fn Other()->Type {{Byte}} comptime fn observe(value:bool)->bool {{value}} comptime fn run()->bool {{{body}}}"
        );
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let callee = support::id(
            &package,
            if body.contains("Other()") {
                "Other"
            } else {
                "observe"
            },
        );
        assert_eq!(
            vm.events()
                .iter()
                .filter(|e| matches!(e,Event::EnterFunction(id) if *id==callee))
                .count(),
            1
        );
    }
    let source = "comptime fn Missing()->Type {let types:meta.List<Type> = [];types[0]} comptime fn run()->bool {let types:meta.List<Type> = [bool,Missing()];let T=types[0];let value:T=true;value}";
    let package = compile(source).unwrap();
    assert!(luna::invocation::invoke(&package, "run", 100_000).is_err());
}

#[test]
fn unknown_mutable_and_out_of_bounds_indices_do_not_invent_type_evidence() {
    for source in [
        "comptime fn run()->bool {var types:meta.List<Type> = [bool];let T=types[0];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [bool];var index:Integer=0;let T=types[index];let value:T=true;value}",
        "comptime fn choose(index:Integer)->bool {let types:meta.List<Type> = [bool,Byte];let T=types[index];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [bool];let T=types[1];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [bool];let T=types[-1];let value:T=true;value}",
        "pure comptime fn Identity<T>->Type {T} comptime fn run()->bool {let types:meta.List<Type> = [Identity<bool>];let T=types[0];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [Byte];let T=types[0];let value:T=true;value}",
    ] {
        assert!(compile(source).is_err(), "unexpectedly accepted {source}");
    }
}

#[test]
fn list_and_index_initializers_run_once_in_source_order() {
    let source="comptime fn Other()->Type {Byte} comptime fn observe(value:bool)->bool {value}
        comptime fn run()->bool {let types:meta.List<Type> = [bool,Other()];let T=types[do {let ignored=observe(true);0}];let value:T=true;value}";
    let package = compile(source).unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let ids = [
        support::id(&package, "Other"),
        support::id(&package, "observe"),
    ];
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => ids.iter().position(|expected| id == expected),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, [0, 1]);
}
