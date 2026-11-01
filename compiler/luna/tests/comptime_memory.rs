//! Comptime mutation uses value-reference receivers, independently of runtime resources.
#[path = "support/execution.rs"]
mod execution_support;
use comptime_interpretor::{Budget, Interpreter, Value};
use execution_support::{compile, compile_with_protocols, id};

// Use the same centralized bindings as the library, without installing runtime
// Copy/Drop protocols into this isolated execution fixture.
const TYPES: &str = include_str!("../../../library/builtin/primitives.fl");

fn check(source: &str) {
    let package = compile(&format!("{TYPES} {source}"));
    assert_eq!(
        luna::invocation::invoke(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn enum_and_nominal_snapshots_do_not_require_runtime_copy() {
    check(
        "struct Payload {flag:bool}
        enum Choice {empty,item(Payload)}
        newtype Wrapped=Choice;
        struct Cell {value:Wrapped}
        impl Cell {
            comptime fn set(self) {self.value = Choice.item(Payload {flag:true}).as(Wrapped);}
        }
        comptime fn run()->bool {
            let cell=Cell {value:Choice.empty.as(Wrapped)};
            let saved=cell.value;
            cell.set();
            let original=saved.as(Choice) match {.empty=>true,.item(_)=>false};
            let updated=cell.value.as(Choice) match {.empty=>false,.item(p)=>p.flag};
            original and updated
        }",
    );
}

#[test]
fn nested_value_snapshots_are_independent_of_receiver_updates() {
    check(
        "struct Leaf {flag:bool} struct Pair {left:Leaf,right:Leaf}
        impl Leaf {comptime fn set(self,value:bool) {self.flag=value;}}
        comptime fn run()->bool {
            let pair=Pair {left:Leaf {flag:false},right:Leaf {flag:false}};
            let saved=pair;
            pair.left.set(true);
            pair.right.set(pair.left.flag);
            pair.left.flag and pair.right.flag and not saved.left.flag and not saved.right.flag
        }",
    );
}

#[test]
fn generic_receivers_mutate_without_resource_bounds() {
    check(
        "struct Leaf {flag:bool} struct Cell where T {value:T}
        impl Cell<T> where T {
            comptime fn put(self,value:T) {self.value=value;}
            comptime fn get(self)->T {self.value}
        }
        comptime fn run()->bool {
            let cell=Cell<Leaf> {value:Leaf {flag:true}};
            let saved=cell.get();
            cell.put(Leaf {flag:false});
            saved.flag and not cell.get().flag
        }",
    );
}

#[test]
fn local_value_updates_work_during_constant_and_type_preparation() {
    let package = compile(&format!(
        "{TYPES}
        struct Cell {{flag:bool}}
        impl Cell {{comptime fn clear(self) {{self.flag=false;}}}}
        comptime fn compute()->bool {{
            let cell=Cell {{flag:true}};let saved=cell;cell.clear();
            saved.flag and not cell.flag
        }}
        const VALUE=compute();comptime fn run()->bool {{VALUE}}
        comptime fn Chosen()->Type {{if compute() {{bool}} else {{u8}}}}
        typealias Selected=Chosen();fn native()->Selected {{true}}"
    ));
    assert_eq!(
        luna::invocation::invoke(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "native", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn exact_numeric_receiver_updates_preserve_values() {
    check(
        "struct Numbers {integer:Integer,decimal:Float}
        impl Numbers {
            comptime fn add(self) {self.integer += 12;self.decimal += 1.25;}
        }
        comptime fn run()->bool {
            let numbers=Numbers {integer:1000000000000000000000000000000000000000000,decimal:1.5};
            let before=numbers;numbers.add();
            numbers.integer == 1000000000000000000000000000000000000000012
                and numbers.decimal == 2.75 and before.decimal == 1.5
                and before.integer == 1000000000000000000000000000000000000000000
        }",
    );
    for ty in ["Integer", "Float"] {
        assert!(
            compile_with_protocols(
                &format!("{TYPES} fn bad(p:*{ty})->{ty} {{p.*}}"),
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn callable_value_replacement_preserves_phase_and_snapshots() {
    let source="typealias Action=comptime fn(bool)->bool;
        struct View {action:Action,value:bool}
        impl View {comptime fn replace(self,action:Action,value:bool) {self.action=action;self.value=value;}}
        comptime fn read(value:bool)->bool {value}
        comptime fn invert(value:bool)->bool {not value}
        comptime fn run()->bool {
            let view=View {action:read,value:false};let saved=view;
            view.replace(invert,true);
            view.value and view.action(false) and not view.action(view.value)
                and not saved.action(saved.value)
        }";
    check(source);
    assert!(
        compile_with_protocols(
            &source.replace("comptime fn invert", "fn invert"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn type_value_snapshots_keep_normal_form_identity_without_copy() {
    let package = compile(&format!(
        "{TYPES}
        pure comptime fn Identity<T>->Type {{T}}
        struct Selection {{value:Type}}
        impl Selection {{comptime fn replace(self,value:Type) {{self.value=value;}}}}
        comptime fn Chosen()->Type {{
            let cell=Selection {{value:Identity<bool>}};
            let saved=cell.value;cell.replace(bool);saved
        }}
        typealias Selected=Chosen();comptime fn Observed()->Type {{Selected}}"
    ));
    let selected = luna::invocation::invoke(&package, "Observed", 100_000).unwrap();
    assert!(
        matches!(&selected, Value::Type(nir::Term::NormalForm { .. })),
        "{selected:?}"
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Chosen", 100_000).unwrap(),
        selected
    );
}

#[test]
fn failed_receiver_invocation_does_not_poison_later_storage() {
    let package=compile("struct Cell {value:bool}
        impl Cell {
            comptime fn fail(self) {self.value=true;assumes false;}
            comptime fn set(self) {self.value=true;}
        }
        comptime fn bad()->bool {let cell=Cell {value:false};cell.fail();cell.value}
        comptime fn good()->bool {let cell=Cell {value:false};let before=cell;cell.set();cell.value and not before.value}");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(vm.invoke(id(&package, "bad"), vec![]).is_err());
    assert_eq!(
        vm.invoke(id(&package, "good"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn bit_field_writeback_keeps_rhs_updates_and_early_return() {
    check("struct Cell {word:u16}
        impl Cell {
            comptime fn edit(self) {self.word'bitvec[4..8]=do {self.word=0xABCD;0b1010};}
            comptime fn early(self)->u16 {self.word'bitvec[do {return 7;}]=1;self.word}
            comptime fn empty(self) {let i:u8=16;self.word'bitvec[i..i]=do {self.word=0x1234;0};}
        }
        comptime fn run()->bool {
            let cell=Cell {word:0};cell.edit();let edited=cell.word;
            let early=cell.early();let unchanged=cell.word;
            cell.empty();edited == 0xABAD and early == 7 and unchanged == edited and cell.word == 0x1234
        }");
}

#[test]
fn memoized_type_storage_requires_no_runtime_resource_authority() {
    let package = compile(&format!(
        "{TYPES}
        pure comptime fn Select()->Type {{var value=bool;let saved=value;value=u8;saved}}"
    ));
    let mut invocation = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        id(&package, "Select"),
        vec![],
        4,
        Default::default(),
    )
    .unwrap();
    assert!(
        invocation
            .invoke(vec![], &mut comptime_executor::Context::new(1))
            .is_err()
    );
    assert_eq!(invocation.cached_results(), 0);
    for reused in [false, true] {
        let outcome = invocation
            .invoke(vec![], &mut comptime_executor::Context::new(100_000))
            .unwrap();
        assert_eq!(
            *outcome.value,
            Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
        );
        assert_eq!(outcome.reused, reused);
    }
    assert_eq!(invocation.cached_results(), 1);
}

#[test]
fn memoized_keys_use_value_snapshots_without_runtime_resources() {
    let source = "const eq_key_phase={comptime_phase:true}; trait Eq {^eq_key_phase comptime fn eq(self,other:Self)->bool;}
        const hash_key_phase={comptime_phase:true}; trait Hash {^hash_key_phase comptime fn hash(self)->.u8'builtin;}
        trait Duplicate {}trait Retire {fn drop(*self);}
        struct Packet {flag:bool}impl Duplicate for Packet {}
        comptime impl Eq for bool {comptime fn eq(self,other:bool)->bool {
            let packet=Packet {flag:self};let saved=packet;saved.flag == other
        }}
        comptime impl Hash for bool {comptime fn hash(self)->.u8'builtin {
            let packet=Packet {flag:self};let saved=packet;
            if saved.flag {0} else {0}
        }}
        typealias Type=.Type'builtin;pure comptime fn Inspect(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    let package = compile(source);
    let prepare = |resources: bool| {
        let mut items = intrinsic::LangItems::new();
        for (item, name) in [
            (intrinsic::LangItem::Eq, "Eq"),
            (intrinsic::LangItem::Hash, "Hash"),
            (intrinsic::LangItem::Copy, "Duplicate"),
            (intrinsic::LangItem::Drop, "Retire"),
        ] {
            if !resources && matches!(item, intrinsic::LangItem::Copy | intrinsic::LangItem::Drop) {
                continue;
            }
            let declaration = package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration;
            items
                .bind_source_traits(&package, &[(item, declaration)])
                .unwrap();
        }
        luna::memoization::Invocation::prepare(
            &package,
            &items,
            id(&package, "Inspect"),
            vec![],
            4,
            Default::default(),
        )
        .unwrap()
    };
    let mut allowed = prepare(true);
    for (value, reused) in [(true, false), (true, true), (false, false), (false, true)] {
        let outcome = allowed
            .invoke(
                vec![Value::Bool(value)],
                &mut comptime_executor::Context::new(100_000),
            )
            .unwrap();
        assert_eq!(
            *outcome.value,
            Value::Type(nir::Term::Primitive(if value {
                nir::Primitive::Bool
            } else {
                nir::Primitive::U8
            }))
        );
        assert_eq!(outcome.reused, reused);
    }
    let mut unbound = prepare(false);
    for reused in [false, true] {
        let outcome = unbound
            .invoke(
                vec![Value::Bool(true)],
                &mut comptime_executor::Context::new(100_000),
            )
            .unwrap();
        assert_eq!(
            *outcome.value,
            Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
        );
        assert_eq!(outcome.reused, reused);
    }
    assert_eq!(unbound.cached_results(), 1);
}

#[test]
fn memoization_driver_reuses_code_without_caching_non_type_results() {
    let package = compile("pure comptime fn invert(flag:bool)->bool {not flag}");
    let mut invocation = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        id(&package, "invert"),
        vec![],
        4,
        Default::default(),
    )
    .unwrap();
    let mut costs = Vec::new();
    for flag in [true, false, true] {
        let mut context = comptime_executor::Context::new(100_000);
        let result = invocation
            .invoke(vec![Value::Bool(flag)], &mut context)
            .unwrap();
        assert_eq!(*result.value, Value::Bool(!flag));
        assert!(!result.reused);
        assert_eq!(invocation.cached_results(), 0);
        costs.push(100_000 - context.remaining());
    }
    assert!(costs[1] < costs[0], "cold and warm costs: {costs:?}");
    assert_eq!(costs[1], costs[2]);
}

#[test]
fn memoization_driver_retains_generic_preparation_without_result_memoization() {
    let package = compile("pure comptime fn inspect() -> bool where T {true}");
    let mut invocation = luna::memoization::Invocation::prepare(
        &package,
        &intrinsic::LangItems::new(),
        id(&package, "inspect"),
        vec![nir::Term::Primitive(nir::Primitive::Bool)],
        4,
        Default::default(),
    )
    .unwrap();
    let mut costs = Vec::new();
    for _ in 0..3 {
        let mut context = comptime_executor::Context::new(100_000);
        let result = invocation.invoke(vec![], &mut context).unwrap();
        assert_eq!(*result.value, Value::Bool(true));
        assert!(!result.reused);
        assert_eq!(invocation.cached_results(), 0);
        costs.push(100_000 - context.remaining());
    }
    assert!(costs[1] < costs[0], "generic preparation costs: {costs:?}");
    assert_eq!(costs[1], costs[2]);
}

#[test]
fn generic_code_session_with_ordinary_inputs_reuses_body_not_arguments() {
    use comptime_interpretor::CodeSession;
    use std::sync::Arc;
    let source = Arc::new(compile("pure comptime fn keep(value:T)->T where T {value}"));
    let mut session = CodeSession::new(source.clone());
    for (index, flag) in [true, false, true].into_iter().enumerate() {
        let mut vm = Interpreter::for_comptime(&source, Budget::default()).unwrap();
        let value = vm
            .with_code_session(&mut session, |vm| {
                vm.invoke_instantiated(
                    id(&source, "keep"),
                    vec![nir::Term::Primitive(nir::Primitive::Bool)],
                    vec![Value::Bool(flag)],
                )
            })
            .unwrap();
        assert_eq!(value, Value::Bool(flag));
        assert!(vm.bytecode_invocations() > 0);
        assert_eq!(vm.bytecode_compilations() > 0, index == 0);
    }
}
