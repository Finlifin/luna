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
    let source = "const eq_key_phase={__flurry_comptime_phase:true}; trait Eq {^eq_key_phase comptime fn eq(self,other:Self)->bool;}
        const hash_key_phase={__flurry_comptime_phase:true}; trait Hash {^hash_key_phase comptime fn hash(self)->.u8'builtin;}
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

const KEY_PROTOCOLS: &str = "const eq_key_phase = {__flurry_comptime_phase: true};
trait Eq { ^eq_key_phase comptime fn eq(self, other: Self) -> bool; }
const hash_key_phase = {__flurry_comptime_phase: true};
trait Hash { ^hash_key_phase comptime fn hash(self) -> .u8'builtin; }
comptime impl Eq for bool { comptime fn eq(self, other: bool) -> bool { self == other } }
comptime impl Hash for bool { comptime fn hash(self) -> .u8'builtin { if self { 1 } else { 0 } } }";

fn key_items(package: &nir::program::Package) -> intrinsic::LangItems {
    let mut items = intrinsic::LangItems::new();
    for (item, name) in [
        (intrinsic::LangItem::Eq, "Eq"),
        (intrinsic::LangItem::Hash, "Hash"),
    ] {
        let decl = package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration;
        items.bind_source_traits(package, &[(item, decl)]).unwrap();
    }
    items
}

#[test]
fn memoization_driver_caches_non_type_pure_results() {
    let source = format!(
        "{TYPES}
        {KEY_PROTOCOLS}
        pure comptime fn sum_to(flag: bool) -> Integer {{
            var i: Integer = 0;
            var total: Integer = 0;
            let limit: Integer = if flag {{ 80 }} else {{ 60 }};
            while i < limit {{
                i = i + 1;
                total = total + i;
            }}
            total
        }}"
    );
    let package = compile(&source);
    let items = key_items(&package);
    let mut invocation = luna::memoization::Invocation::prepare(
        &package,
        &items,
        id(&package, "sum_to"),
        vec![],
        4,
        Default::default(),
    )
    .unwrap();
    assert_ne!(
        invocation.scope(),
        comptime_executor::Scope {
            definitions: 0,
            protocols: 0,
            target: 0,
        }
    );
    let mut costs = Vec::new();
    for ((flag, expected_reused), expected_cached) in [(true, false), (false, false), (true, true)]
        .into_iter()
        .zip([1, 2, 2])
    {
        let mut context = comptime_executor::Context::new(100_000);
        let result = invocation
            .invoke(vec![Value::Bool(flag)], &mut context)
            .unwrap();
        let expected_sum = if flag { 3240 } else { 1830 };
        assert_eq!(
            *result.value,
            Value::Integer(comptime_runtime::Integer::from_i128(expected_sum))
        );
        assert_eq!(result.reused, expected_reused);
        assert_eq!(invocation.cached_results(), expected_cached);
        costs.push(100_000 - context.remaining());
    }
    assert!(
        costs[2] < costs[0],
        "cached hit must save execution steps: {costs:?}"
    );
    assert!(
        costs[2] < costs[1],
        "cached hit must save execution steps: {costs:?}"
    );
}

#[test]
fn memoization_driver_retains_generic_preparation_and_caches_non_type_results() {
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
    for index in 0..3 {
        let mut context = comptime_executor::Context::new(100_000);
        let result = invocation.invoke(vec![], &mut context).unwrap();
        assert_eq!(*result.value, Value::Bool(true));
        assert_eq!(result.reused, index > 0);
        assert_eq!(invocation.cached_results(), 1);
        costs.push(100_000 - context.remaining());
    }
    assert!(costs[1] < costs[0], "generic preparation costs: {costs:?}");
    assert_eq!(costs[1], costs[2]);
}

#[test]
fn memoization_scope_changes_with_definitions_protocols_and_target_and_invalidates() {
    let base_source = format!(
        "{TYPES}
        {KEY_PROTOCOLS}
        pure comptime fn compute(flag: bool) -> Integer {{
            if flag {{ 42 }} else {{ 7 }}
        }}"
    );
    let alt_def_source = format!(
        "{TYPES}
        {KEY_PROTOCOLS}
        pure comptime fn compute(flag: bool) -> Integer {{
            if flag {{ 99 }} else {{ 7 }}
        }}"
    );
    let proto_source = format!(
        "{TYPES}
        {KEY_PROTOCOLS}
        trait Marker {{}}
        pure comptime fn compute(flag: bool) -> Integer {{
            if flag {{ 42 }} else {{ 7 }}
        }}"
    );
    let base = compile(&base_source);
    let alt_def = compile(&alt_def_source);
    let proto_pkg = compile(&proto_source);
    let mut with_target = base.clone();
    with_target.target = Some(nir::target::TargetData::new(64).unwrap());
    let base_items = key_items(&base);
    let alt_def_items = key_items(&alt_def);
    let proto_base_items = key_items(&proto_pkg);
    let mut alt_items = key_items(&proto_pkg);
    let marker = proto_pkg
        .traits
        .iter()
        .find(|t| t.name == "Marker")
        .unwrap()
        .declaration;
    alt_items
        .bind_source_traits(&proto_pkg, &[(intrinsic::LangItem::Copy, marker)])
        .unwrap();

    let s_base = luna::memoization::scope_for(&base, &base_items);
    let s_same = luna::memoization::scope_for(&base, &base_items);
    let s_def = luna::memoization::scope_for(&alt_def, &alt_def_items);
    let s_proto = luna::memoization::scope_for(&proto_pkg, &proto_base_items);
    let s_lang = luna::memoization::scope_for(&proto_pkg, &alt_items);
    let s_target = luna::memoization::scope_for(&with_target, &base_items);

    assert_eq!(s_base, s_same);
    assert_ne!(s_base.definitions, s_def.definitions);
    assert_ne!(s_base.protocols, s_proto.protocols);
    assert_ne!(s_proto.protocols, s_lang.protocols);
    assert_ne!(s_base.target, s_target.target);

    let mut invocation = luna::memoization::Invocation::prepare(
        &base,
        &base_items,
        id(&base, "compute"),
        vec![],
        4,
        Default::default(),
    )
    .unwrap();
    assert_eq!(invocation.scope(), s_base);
    let first = invocation
        .invoke(
            vec![Value::Bool(true)],
            &mut comptime_executor::Context::new(100_000),
        )
        .unwrap();
    assert!(!first.reused);
    assert_eq!(
        *first.value,
        Value::Integer(comptime_runtime::Integer::from_i128(42))
    );
    let hit = invocation
        .invoke(
            vec![Value::Bool(true)],
            &mut comptime_executor::Context::new(100_000),
        )
        .unwrap();
    assert!(hit.reused);
    assert_eq!(invocation.cached_results(), 1);

    assert!(!invocation.set_scope(s_base));
    assert_eq!(invocation.cached_results(), 1);
    assert!(invocation.set_scope(s_def));
    assert_eq!(invocation.cached_results(), 0);
    let after_invalidation = invocation
        .invoke(
            vec![Value::Bool(true)],
            &mut comptime_executor::Context::new(100_000),
        )
        .unwrap();
    assert!(!after_invalidation.reused);
    assert_eq!(invocation.cached_results(), 1);
}

#[test]
fn elaboration_required_memoizes_pure_non_type_queries_and_invalidates_across_worlds() {
    let source_a = format!(
        "{TYPES}
        pure comptime fn fib_like(n: Integer) -> Integer {{
            if n <= 1 {{ n }} else {{ n + fib_like(n - 1) }}
        }}
        const A: Integer = fib_like(12);
        const B: Integer = fib_like(12);
        const C: Integer = fib_like(10);
        comptime fn run() -> bool {{ A == 78 and B == 78 and C == 55 }}"
    );
    let source_b = format!(
        "{TYPES}
        pure comptime fn fib_like(n: Integer) -> Integer {{
            if n <= 1 {{ n }} else {{ n * 2 + fib_like(n - 1) }}
        }}
        const A: Integer = fib_like(12);
        comptime fn run() -> bool {{ A == 155 }}"
    );
    let pkg_a = compile(&source_a);
    assert_eq!(
        luna::invocation::invoke(&pkg_a, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    let pkg_b = compile(&source_b);
    assert_eq!(
        luna::invocation::invoke(&pkg_b, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
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
