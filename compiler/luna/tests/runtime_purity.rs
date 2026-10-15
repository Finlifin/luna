//! Runtime pure promises are P4; termination is a separate obligation.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

#[test]
fn pure_scalar_functions_keep_their_runtime_phase_and_execute() {
    let package = compile(
        "pure fn smaller(left:.u8'builtin,right:.u8'builtin)->.u8'builtin {var result=left;if right < left {result=right;}result} fn run()->.u8'builtin {smaller(7,3)}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "smaller")
        .unwrap();
    assert!(function.pure);
    assert!(
        function
            .to_string()
            .contains("(phase runtime) (inline false) (pure true)")
    );
    assert_eq!(function.phase, nir::program::FunctionPhase::Runtime);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(3)
    );
    assert!(luna::invocation::invoke(&package, "smaller", 100_000).is_err());
}

#[test]
fn pure_recursive_components_check_all_bodies_without_claiming_termination() {
    let package = compile(
        "pure fn even(value:.u8'builtin)->bool {if value == 0 {true} else {odd(value - 1)}} fn odd(value:.u8'builtin)->bool {if value == 0 {false} else {even(value - 1)}} fn run()->bool {even(8) and not even(7)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let package = compile("pure fn spin()->bool {spin()}");
    assert!(
        package
            .functions
            .iter()
            .find(|f| f.name == "spin")
            .unwrap()
            .pure
    );
    // The interpreter still spends fuel; purity does not erase divergence.
    let mut vm = Interpreter::new(
        &package,
        Budget {
            steps: 100,
            ..Budget::default()
        },
    )
    .unwrap();
    assert!(vm.invoke(id(&package, "spin"), vec![]).is_err());
    assert!(calls(&package, &vm).iter().all(|name| name == "spin"));
}

#[test]
fn pure_loops_may_mutate_only_private_scalar_state() {
    let package = compile(
        "pure fn drain(value:.u8'builtin)->.u8'builtin {var remaining=value;while remaining > 0 {remaining = remaining - 1;}remaining} pure fn spin() {while true {}} fn run()->.u8'builtin {drain(8)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(0)
    );
}

#[test]
fn pure_calls_can_use_unmarked_helpers_when_their_behavior_is_known() {
    let package = compile(
        "fn identity(value:T)->T where T {value} pure fn invert(value:bool)->bool {identity(not value)} fn run()->bool {invert(false)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_is_not_a_license_for_external_effects_or_unknown_cleanup() {
    for source in [
        "pure fn write(p:*bool)->bool {p.* = true;true}",
        "pure fn read(p:*bool)->bool {p.*}",
        "pure fn call(action:fn()->bool)->bool {action()}",
        "struct Owned {} pure fn discard()->bool {let value=Owned {};true}",
        "fn write(p:*bool)->bool {p.* = true;true} pure fn caller(p:*bool)->bool {write(p)}",
        "pure fn first(p:*bool)->bool {second(p)} fn second(p:*bool)->bool {if true {first(p)} else {p.* = true;true}}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P4 evidence"), "{source}: {error}");
    }
}

#[test]
fn default_suppliers_need_finite_completion_even_when_declared_pure() {
    for source in [
        "pure fn spin()->bool {spin()} fn take(.flag:bool=spin())->bool {flag}",
        "pure fn spin()->bool {while true {}true} fn take(.flag:bool=spin())->bool {flag}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{error}");
    }
    let package = compile(
        "pure fn invert(value:bool)->bool {not value} fn take(.flag:bool=invert(false))->bool {flag} fn run()->bool {take()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_comptime_retains_its_existing_phase_policy() {
    let package = compile(
        "pure comptime fn invert(value:bool)->bool {not value} comptime fn run()->bool {invert(false)}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let package = compile("trait Source {pure fn read(self)->bool;}");
    assert!(package.traits[0].methods[0].pure);
}

#[test]
fn open_pure_definitions_cannot_escape_the_definition_check() {
    let source = "typealias Type=.Type'builtin;comptime fn Select(flag:bool)->Type {if flag {bool} else {.u8'builtin}} pure fn keep(comptime flag:bool,value:defer Select(flag))->defer Select(flag) {value}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("definition-side behavior evidence"),
        "{error}"
    );
}

#[test]
fn pure_callees_do_not_erase_effectful_argument_evaluation() {
    let package = compile(
        "fn mark(p:*bool)->bool {p.* = true;true} pure fn keep(value:bool)->bool {value} fn run()->bool {let observed=false;let result=keep(mark(observed.ref));observed and result}",
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), vec!["run", "mark", "keep"]);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_promises_do_not_override_machine_arithmetic_requirements() {
    let error = compile_with_protocols(
        "pure fn next(value:.u8'builtin)->.u8'builtin {value + 1}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("range is not proven"), "{error}");
    let package = compile(
        "pure fn next(value:.u8'builtin)->.u8'builtin where requires value < 255 {value + 1} fn run()->.u8'builtin {next(41)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(42)
    );
}

#[test]
fn pure_generic_identity_transfers_owned_values_without_inventing_copy() {
    let package = compile(
        "struct Ticket {flag:bool} pure fn keep(value:T)->T where T {let forwarded=value;forwarded} fn run()->bool {keep(Ticket {flag:true}).flag}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_branches_transfer_all_owned_inputs_on_every_return() {
    let package = compile(
        "struct Ticket {flag:bool} pure fn reorder(first:T,second:T,flag:bool)->(T,T) where T {if flag {return (first,second);}else{(second,first)}} fn run()->bool {let (a,b)=reorder(Ticket {flag:true},Ticket {flag:false},false);not a.flag and b.flag}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_helpers_and_constructors_preserve_owned_responsibility() {
    let package = compile(
        "struct Ticket {flag:bool} struct Boxed where T {value:T} fn keep(value:T)->T where T {value} pure fn wrap(value:T)->Boxed<T> where T {Boxed<T> {value:keep(value)}} fn run()->bool {wrap(Ticket {flag:true}).value.flag}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_transfer_rejects_duplication_and_implicit_retirement() {
    for source in [
        "pure fn duplicate(value:T)->(T,T) where T {(value,value)}",
        "pure fn forget(value:T) where T {}",
        "pure fn choose(first:T,second:T,flag:bool)->T where T {if flag {first} else {second}}",
        "fn forget(value:T)->bool where T {true} pure fn caller(value:T)->bool where T {forget(value)}",
        "struct Ticket {} pure fn leak()->bool {let value=Ticket {};true}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P4 evidence"), "{source}: {error}");
    }
}

#[test]
fn pure_enum_forwarding_and_normal_tuple_construction_keep_transfer_paths() {
    let package = compile(
        "enum Choice where T {left(T),right(T)} pure fn keep(value:Choice<T>)->Choice<T> where T {value} struct Ticket {flag:bool} fn run()->bool {keep(Choice<Ticket>.right(Ticket {flag:true})) match {.left(inner)=>inner.flag,.right(inner)=>inner.flag}}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let source = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T} pure comptime fn Tuple<...Es:List<Type>>->Type {bool} pure fn swap(a:A,b:B)->(B,A) where A,B {(b,a)} struct Ticket {flag:bool} fn run()->bool {let (a,b)=swap(Ticket {flag:true},Ticket {flag:false});not a.flag and b.flag}";
    let package = execution_support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_owned_call_preparation_checks_early_exit_cleanup() {
    let source = "fn take(value:T,flag:bool)->T where T {value} pure fn prepare(first:T,second:T)->T where T {take(first,do {return second;false})}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn extracting_payloads_still_requires_evidence_for_wrapper_cleanup() {
    let source = "enum Choice where T {left(T),right(T)} pure fn unwrap(value:Choice<T>)->T where T {value match {.left(inner)=>inner,.right(inner)=>inner}}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    // The payload is transferred, but the enum wrapper still has a cleanup
    // candidate. Its Drop behavior cannot be guessed from empty payload fields.
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_nominal_copy_requires_bound_protocol_and_representation_evidence() {
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Flag {value:bool} impl Copy for Flag {} pure fn duplicate(value:Flag)->(Flag,Flag) {(value,value)} fn run()->bool {let (first,second)=duplicate(Flag {value:true});first.value and second.value}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let unbound = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(unbound.contains("P4 evidence"), "{unbound}");
    let with_drop = format!("{source} impl Drop for Flag {{fn drop(*self) {{}}}}");
    let error =
        execution_support::compile_with_source_bindings(&with_drop, &Default::default(), selectors)
            .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_copy_checks_all_nested_fields_and_enum_variants() {
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    for definition in [
        "struct Flag {owned:Owned}",
        "enum Flag {empty,owned(Owned)}",
    ] {
        let source = format!(
            "trait Copy {{}} trait Drop {{fn drop(*self);}} struct Owned {{}} {definition} impl Copy for Flag {{}} pure fn duplicate(value:Flag)->(Flag,Flag) {{(value,value)}}"
        );
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
}

#[test]
fn affine_aggregate_retirement_needs_drop_evidence_but_not_copy() {
    let selectors = luna::protocols::type_constructors::Selectors {
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Owned {value:bool} struct Pair {first:Owned,second:Owned} pure fn read(value:Pair)->bool {value.first.value} pure fn unwrap(value:Choice)->Owned {value match {.some(inner)=>inner}} enum Choice {some(Owned)} fn run()->bool {read(Pair {first:Owned {value:true},second:Owned {value:false}}) and unwrap(Choice.some(Owned {value:true})).value}";
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret_with_protocols_in_package(
            &package, 0, "run", "Copy", "Drop", 100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn trivial_retirement_checks_nested_destructors_and_all_variants() {
    let selectors = luna::protocols::type_constructors::Selectors {
        drop: Some("input::Drop"),
        ..Default::default()
    };
    for wrapper in [
        "struct Wrapper {value:Owned}",
        "enum Wrapper {empty,full(Owned)}",
    ] {
        let source = format!(
            "trait Drop {{fn drop(*self);}} struct Owned {{flag:bool}} impl Drop for Owned {{fn drop(*self) {{let observed=self.flag.*;}}}} {wrapper} pure fn retire(value:Wrapper) {{}}"
        );
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
    let duplicate = "trait Drop {fn drop(*self);} struct Owned {} pure fn duplicate(value:Owned)->(Owned,Owned) {(value,value)}";
    let error =
        execution_support::compile_with_source_bindings(duplicate, &Default::default(), selectors)
            .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_retirement_checks_selected_drop_bodies_and_helper_calls() {
    let selectors = luna::protocols::type_constructors::Selectors {
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Owned {} impl Drop for Owned {fn drop(*self) {finish()}} fn finish() {var flag=false;flag=true;} struct Wrapper {owned:Owned} pure fn retire(value:Wrapper)->bool {true} fn run()->bool {retire(Wrapper {owned:Owned {}})}";
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols_in_package(
            &package, 0, "run", "Copy", "Drop", 100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_drop_instances_are_specialized_before_purity_analysis() {
    let selectors = luna::protocols::type_constructors::Selectors {
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Token where T {} impl Drop for Token<T> where T {fn drop(*self) {finish<T>()}} fn finish() where T {} pure fn retire(value:Token<bool>)->bool {true} fn run()->bool {retire(Token<bool> {})}";
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols_in_package(
            &package, 0, "run", "Copy", "Drop", 100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn destructor_purity_follows_only_selected_bodies_and_rejects_indirect_writes() {
    let selectors = luna::protocols::type_constructors::Selectors {
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let prefix = "trait Drop {fn drop(*self);} struct Owned {} impl Drop for Owned {fn drop(*self) {}} struct Other {flag:bool} impl Drop for Other {fn drop(*self) {write(self.flag)}} fn write(p:*bool) {p.* = true;}";
    execution_support::compile_with_source_bindings(
        &format!("{prefix} pure fn retire(value:Owned) {{}}"),
        &Default::default(),
        selectors,
    )
    .unwrap();
    let error = execution_support::compile_with_source_bindings(
        &format!("{prefix} pure fn retire(value:Other) {{}}"),
        &Default::default(),
        selectors,
    )
    .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_conditional_variant_patterns_check_every_residual_cleanup() {
    let selectors = luna::protocols::type_constructors::Selectors {
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Held {flag:bool}
        impl Drop for Held {fn drop(*self) {}}
        enum E {left(Held,Held),right(bool,Held,Held)}
        pure fn select(value:E)->bool {let (.left(item,_) or .right(_,_,item))=value;true}
        fn run()->bool {select(E.left(Held {flag:true},Held {flag:false})) and
            select(E.right(false,Held {flag:true},Held {flag:false}))}";
    let package =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols_in_package(
            &package, 0, "run", "Copy", "Drop", 100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let impure = source.replace(
        "fn drop(*self) {}",
        "fn drop(*self) {let observed=self.flag.*;}",
    );
    let failure =
        execution_support::compile_with_source_bindings(&impure, &Default::default(), selectors)
            .unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}
