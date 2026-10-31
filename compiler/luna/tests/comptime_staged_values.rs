//! Ordinary comptime data must survive staged IIR preparation without erasure.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

fn compare_paths(source: &str) {
    let mut package = support::compile(source);
    let roots = [iir::InstanceKey {
        declaration: support::id(&package, "run"),
        arguments: vec![].into(),
    }];
    package.dispatch_instances = elaboration::materialization::dispatch_instances_for_invocation(
        &package,
        &roots,
        Default::default(),
    )
    .unwrap();
    let graph = elaboration::materialization::instantiate(
        &package,
        &roots,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    for materialized in [false, true] {
        for compiled in [false, true] {
            let mut vm = match (materialized, compiled) {
                (false, false) => Interpreter::for_comptime_tree(&package, Budget::default()),
                (false, true) => Interpreter::for_comptime(&package, Budget::default()),
                (true, false) => Interpreter::from_iir_for_comptime_tree(&graph, Budget::default()),
                (true, true) => Interpreter::from_iir_for_comptime(&graph, Budget::default()),
            }
            .unwrap();
            for _ in 0..3 {
                let before = vm.bytecode_invocations();
                let result = if materialized {
                    vm.invoke_instance(&graph.roots[0], vec![])
                } else {
                    vm.invoke(roots[0].declaration, vec![])
                }
                .unwrap();
                assert_eq!(result, Value::Bool(true));
                assert_eq!(vm.bytecode_invocations() > before, compiled);
            }
        }
    }
}

#[test]
fn staged_exact_decimal_values_do_not_use_binary_float_transport() {
    compare_paths(
        "typealias Float=.Float'builtin;
        typealias Next=%{value:Float} comptime fn()->Float;
        comptime fn sum(positive:bool)->Next {
            var value:Float=if positive {0.1} else { -0.1 };
            || {value=value + 0.2;value}
        }
        pure comptime trait Amount where positive:bool {assoc next:Next=sum(positive);}
        struct Holder where positive:bool {}
        comptime impl Amount<positive> for Holder<positive> where positive:bool {}
        comptime fn run()->bool {
            let next=Holder<true>.as(Amount<true>).next;
            let negative=Holder<false>.as(Amount<false>).next;
            next() == 0.3 and next() == 0.5 and negative() == 0.1
        }",
    );
}

#[test]
fn staged_objects_keep_domains_duplicate_keys_and_shared_closure_entries() {
    let source = "typealias Object=.Object'builtin;
        typealias Integer=.Integer'builtin;
        typealias Type=.Type'builtin;
        typealias Char=.char'builtin;
        const object_value=.object_value'builtin;
        const object_key=.object_key'builtin;
        const object_length=.object_length'builtin;
        pure comptime fn Identity<T>->Type {T}
        typealias Counter=%{value:bool} comptime fn()->bool;
        comptime fn counter()->Counter {var value=false; || {value=not value;value}}
        comptime fn make()->Object {
            let next=counter();
            let absent:?bool=null;
            let result={shared:next, shared:next, 'x', absent:absent,
                nested:{yes:true}, kind:Identity<bool>};
            result
        }
        pure comptime trait Data {assoc object:Object=make();}
        comptime impl Data for bool {}
        comptime fn run()->bool {
            let data=bool.as(Data).object;
            let first=object_value(Counter,data,0) match {v?=>v,null=>return false};
            let second=object_value(Counter,data,1) match {v?=>v,null=>return false};
            let again=bool.as(Data).object;
            let fresh=object_value(Counter,again,0) match {v?=>v,null=>return false};
            first() and not second() and fresh()
                and object_length(data)==6
                and (object_key(data,0) matches .shared?)
                and (object_key(data,1) matches .shared?)
                and (object_key(data,2) matches null)
                and (object_value(Char,data,2) matches 'x'?)
                and (object_value(bool,data,2) matches null)
                and (object_value(?bool,data,3) match {v?=>v matches null,null=>false})
                and (object_value(Object,data,4) match {
                    v?=>object_value(bool,v,0) matches true?,null=>false})
                and (object_value(Type,data,5) match {T?=>T==Identity<bool>,null=>false})
        }";
    compare_paths(source);
    // A generic associated captured call executes during staging and returns
    // the Object itself through freeze_staged_value, beyond constant admission.
    let captured = source
        .replace("comptime fn make()->Object", "typealias Reader=%{result:Object} comptime fn()->Object; comptime fn make(seed:bool)->Reader")
        .replace("nested:{yes:true}", "nested:{yes:seed}")
        .replace("            result\n", "            || result\n")
        .replace("pure comptime trait Data {assoc object:Object=make();}",
                 "pure comptime trait Data where seed:bool {assoc reader:Reader=make(seed);}")
        .replace("comptime impl Data for bool {}",
                 "struct Host where seed:bool {} comptime impl Data<seed> for Host<seed> where seed:bool {}")
        .replace("bool.as(Data).object", "Host<true>.as(Data<true>).reader()");
    assert_ne!(source, captured);
    compare_paths(&captured);
}
