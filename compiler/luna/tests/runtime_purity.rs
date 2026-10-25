//! Runtime pure promises are P4; termination is a separate obligation.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

#[test]
fn finite_scalar_type_domains_check_every_resource_behavior_case() {
    let prefix="trait Copy {} trait Drop {fn drop(*self);} impl Copy for bool {} impl Copy for .f32'builtin {} impl Copy for .f64'builtin {}
        trait Read {fn read(self,p:*bool)->bool;}
        impl Read for .f32'builtin {fn read(self,p:*bool)->bool {true}}
        impl Read for .f64'builtin {fn read(self,p:*bool)->bool {true}}
        pure fn choose(value:T,p:*bool)->bool where T :- Read, requires T == .f32'builtin or T == .f64'builtin {value.as(Read).read(p)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    execution_support::compile_with_source_bindings(prefix, &Default::default(), selectors)
        .unwrap();
    for source in [
        prefix.replace(
            "impl Read for .f32'builtin {fn read(self,p:*bool)->bool {true}}",
            "impl Read for .f32'builtin {fn read(self,p:*bool)->bool {p.* = false;true}}",
        ),
        prefix.replace(
            "impl Read for .f64'builtin {fn read(self,p:*bool)->bool {true}}",
            "impl Read for .f64'builtin {fn read(self,p:*bool)->bool {p.* = false;true}}",
        ),
        prefix.replace(", requires T == .f32'builtin or T == .f64'builtin", ""),
    ] {
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
    // A failed alternative must not persist as an assumption for a fresh check.
    execution_support::compile_with_source_bindings(prefix, &Default::default(), selectors)
        .unwrap();
}

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
        "pure fn write(p:*bool)->bool {unsafe {p.* = true;true}}",
        "pure fn read(p:*bool)->bool {unsafe {p.*}}",
        "pure unsafe fn read(p:*bool)->bool {p.*}",
        "pure unsafe fn write(p:*bool)->bool {p.* = true;true}",
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
    let package = execution_support::compile_with_tuple(
        "struct Ticket {flag:bool} pure fn reorder(first:T,second:T,flag:bool)->Tuple<T,T> where T {if flag {return (first,second);}else{(second,first)}} fn run()->bool {let (a,b)=reorder(Ticket {flag:true},Ticket {flag:false},false);not a.flag and b.flag}",
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
        "pure fn duplicate(value:T)->Tuple<T,T> where T {(value,value)}",
        "pure fn forget(value:T) where T {}",
        "pure fn choose(first:T,second:T,flag:bool)->T where T {if flag {first} else {second}}",
        "fn forget(value:T)->bool where T {true} pure fn caller(value:T)->bool where T {forget(value)}",
        "struct Ticket {} pure fn leak()->bool {let value=Ticket {};true}",
    ] {
        let error = execution_support::compile_with_tuple_protocols(source, &Default::default())
            .unwrap_err();
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
    let source = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T} pure comptime fn Tuple<...Es:List<Type>>->Type {bool} pure fn swap(a:A,b:B)->Tuple<B,A> where A,B {(b,a)} struct Ticket {flag:bool} fn run()->bool {let (a,b)=swap(Ticket {flag:true},Ticket {flag:false});not a.flag and b.flag}";
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
    let source = "trait Copy {} trait Drop {fn drop(*self);} struct Flag {value:bool} impl Copy for Flag {} pure fn duplicate(value:Flag)->Tuple<Flag,Flag> {(value,value)} fn run()->bool {let (first,second)=duplicate(Flag {value:true});first.value and second.value}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_tuple_selectors(source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let unbound =
        execution_support::compile_with_tuple_protocols(source, &Default::default()).unwrap_err();
    assert!(unbound.contains("P4 evidence"), "{unbound}");
    let with_drop = format!("{source} impl Drop for Flag {{fn drop(*self) {{}}}}");
    let error =
        execution_support::compile_with_tuple_selectors(&with_drop, &Default::default(), selectors)
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
            "trait Copy {{}} trait Drop {{fn drop(*self);}} struct Owned {{}} {definition} impl Copy for Flag {{}} pure fn duplicate(value:Flag)->Tuple<Flag,Flag> {{(value,value)}}"
        );
        let error = execution_support::compile_with_tuple_selectors(
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
        let error = execution_support::compile_with_tuple_selectors(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_err();
        assert!(error.contains("P4 evidence"), "{error}");
    }
    let duplicate = "trait Drop {fn drop(*self);} struct Owned {} pure fn duplicate(value:Owned)->Tuple<Owned,Owned> {(value,value)}";
    let error =
        execution_support::compile_with_tuple_selectors(duplicate, &Default::default(), selectors)
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

#[test]
fn pure_let_else_retirement_still_requires_selected_drop_evidence() {
    for flag in [true, false] {
        let source = format!(
            "trait Copy {{}}trait Drop {{fn drop(*self);}}struct Owned {{}}impl Drop for Owned {{fn drop(*self) {{}}}}pure fn retire(value:Owned,flag:bool)->bool {{let true=flag else {{return true;}};true}}fn run()->bool {{retire(Owned {{}},{flag})}}"
        );
        let unbound = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(unbound.contains("P4 evidence"), "{unbound}");
        let selectors = luna::protocols::type_constructors::Selectors {
            drop: Some("input::Drop"),
            ..Default::default()
        };
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols_in_package(
                &package, 0, "run", "Copy", "Drop", 100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn finite_defaults_allow_checked_let_else_returns_but_not_divergence() {
    for flag in [true, false] {
        let package = compile(&format!(
            "fn choose(flag:bool)->bool {{let true=flag else {{return false;}};true}}fn take(.flag:bool=choose({flag}))->bool {{flag}}fn run()->bool {{take()}}"
        ));
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(flag)
        );
    }
    for failure in ["while true {}", "return spin();"] {
        let source = format!(
            "fn spin()->bool {{spin()}}fn choose(flag:bool)->bool {{let true=flag else {{{failure}}};true}}fn take(.flag:bool=choose(false))->bool {{flag}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("P3 stability/termination evidence"),
            "{failure}: {error}"
        );
    }
    let source = "fn choose(flag:bool,p:*bool)->bool {let true=flag else {p.* = true;return true;};true}fn take(p:*bool,.flag:bool=choose(false,p))->bool {flag}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("P3 stability/termination evidence"),
        "{error}"
    );
}

#[test]
fn finite_let_else_defaults_observe_each_call_storage_version() {
    let package = compile(
        "fn choose(p:*bool,flag:bool)->bool {let true=flag else {return p.*;};false}fn take(p:*bool,.value:bool=choose(p,false))->bool {value}fn run()->bool {var value=false;let first=take(value.ref);value=true;let second=take(value.ref);not first and second}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let names = calls(&package, &vm);
    assert_eq!(
        names
            .iter()
            .filter(|name| name.as_str() == "choose")
            .count(),
        2,
        "{names:?}"
    );
}

#[test]
fn finite_let_else_default_suppliers_check_cleanup_completion() {
    for flag in [true, false] {
        let source = format!(
            "trait Copy {{}}trait Drop {{fn drop(*self);}}struct Owned {{}}impl Drop for Owned {{fn drop(*self) {{}}}}pure fn choose(flag:bool)->bool {{let value=Owned {{}};let true=flag else {{return true;}};true}}fn take(.flag:bool=choose({flag}))->bool {{flag}}fn run()->bool {{take()}}"
        );
        let selectors = luna::protocols::type_constructors::Selectors {
            drop: Some("input::Drop"),
            ..Default::default()
        };
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols_in_package(
                &package, 0, "run", "Copy", "Drop", 100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
    let source = "trait Copy {}trait Drop {fn drop(*self);}struct Owned {}impl Drop for Owned {fn drop(*self) {while true {}}}pure fn choose(flag:bool)->bool {let value=Owned {};let true=flag else {return true;};true}fn take(.flag:bool=choose(false))->bool {flag}";
    let selectors = luna::protocols::type_constructors::Selectors {
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let error =
        execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
            .unwrap_err();
    assert!(
        error.contains("P3 stability/termination evidence"),
        "{error}"
    );
}

#[test]
fn owned_default_arguments_have_checked_preparation_cleanup() {
    for flag in [true, false] {
        let source = format!(
            "trait Copy {{}}trait Drop {{fn drop(*self);}}struct Owned {{}}impl Drop for Owned {{fn drop(*self) {{}}}}fn choose(value:Owned,flag:bool)->bool {{let true=flag else {{return true;}};true}}fn take(.flag:bool=choose(Owned {{}},{flag}))->bool {{flag}}fn run()->bool {{take()}}"
        );
        let selectors = luna::protocols::type_constructors::Selectors {
            drop: Some("input::Drop"),
            ..Default::default()
        };
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols_in_package(
                &package, 0, "run", "Copy", "Drop", 100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
    for source in [
        "trait Copy {}trait Drop {fn drop(*self);}struct Owned {}impl Drop for Owned {fn drop(*self) {while true {}}}fn choose(value:Owned)->bool {true}fn take(.flag:bool=choose(Owned {}))->bool {flag}",
        "trait Copy {}trait Drop {fn drop(*self);}struct Owned {}impl Drop for Owned {fn drop(*self) {}}fn consume(value:Owned)->bool {true}fn take(value:Owned,.flag:bool=consume(value))->bool {flag}",
        "trait Copy {}trait Drop {fn drop(*self);}fn take(flag:bool,.value:bool=do {flag=false;true})->bool {value}",
    ] {
        let selectors = luna::protocols::type_constructors::Selectors {
            drop: Some("input::Drop"),
            ..Default::default()
        };
        let error =
            execution_support::compile_with_source_bindings(source, &Default::default(), selectors)
                .unwrap_err();
        assert!(
            error.contains("P3 stability/termination evidence"),
            "{source}: {error}"
        );
    }
}

#[test]
fn owned_defaults_copy_external_nominal_inputs_with_protocol_evidence() {
    for (definition, construct, read) in [
        ("newtype Flag=bool;", "true.as(Flag)", "flag.as(bool)"),
        (
            "struct Flag {value:bool}",
            "Flag {value:true}",
            "flag.value",
        ),
    ] {
        for (implementation, selected, accepted) in [
            ("impl Copy for Flag {}", true, true),
            ("", true, false),
            ("impl Copy for Flag {}", false, false),
            (
                "impl Copy for Flag {}impl Drop for Flag {fn drop(*self) {}}",
                true,
                false,
            ),
        ] {
            let source = format!(
                "trait Copy {{}}trait Drop {{fn drop(*self);}}{definition}{implementation}struct Owned {{}}impl Drop for Owned {{fn drop(*self) {{}}}}fn consume(value:Owned,flag:Flag)->bool {{{read}}}fn take(flag:Flag,.result:bool=consume(Owned {{}},flag))->bool {{result and {read}}}fn run()->bool {{take({construct})}}"
            );
            let result = execution_support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    copy: selected.then_some("input::Copy"),
                    drop: Some("input::Drop"),
                    ..Default::default()
                },
            );
            if accepted {
                let package = result.unwrap_or_else(|error| panic!("{source}: {error}"));
                assert_eq!(
                    luna::execution::interpret_with_protocols_in_package(
                        &package, 0, "run", "Copy", "Drop", 100_000
                    )
                    .unwrap(),
                    Value::Bool(true)
                );
            } else {
                let error = result.unwrap_err();
                assert!(
                    error.contains("P3 stability/termination evidence"),
                    "{source}: {error}"
                );
            }
        }
    }
}

#[test]
fn default_copy_evidence_checks_nested_representation() {
    for (inner_impl, accepted) in [("impl Copy for Inner {}", true), ("", false)] {
        let source = format!(
            "trait Copy {{}}trait Drop {{fn drop(*self);}}struct Inner {{flag:bool}}{inner_impl}struct Outer {{inner:Inner}}impl Copy for Outer {{}}struct Owned {{}}impl Drop for Owned {{fn drop(*self) {{}}}}fn consume(value:Owned,outer:Outer)->bool {{outer.inner.flag}}fn take(outer:Outer,.result:bool=consume(Owned {{}},outer))->bool {{result and outer.inner.flag}}fn run()->bool {{take(Outer {{inner:Inner {{flag:true}}}})}}"
        );
        let result = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::Copy"),
                drop: Some("input::Drop"),
                ..Default::default()
            },
        );
        if accepted {
            let package = result.unwrap();
            assert_eq!(
                luna::execution::interpret_with_protocols_in_package(
                    &package, 0, "run", "Copy", "Drop", 100_000
                )
                .unwrap(),
                Value::Bool(true)
            );
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("P3 stability/termination evidence"),
                "{error}"
            );
        }
    }
}

#[test]
fn default_nominal_pointer_copies_preserve_external_storage() {
    let prefix = "trait Copy {}trait Drop {fn drop(*self);}newtype Ref=*bool;impl Copy for Ref {}struct Owned {}impl Drop for Owned {fn drop(*self) {}}";
    let source = format!(
        "{prefix}fn read(value:Owned,p:Ref)->bool {{p.as(*bool).*}}fn take(p:Ref,.result:bool=read(Owned {{}},p))->bool {{result}}fn run()->bool {{var flag=true;let p=flag.ref.as(Ref);let first=take(p);flag=false;let second=take(p);first and not second}}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(&source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols_in_package(
            &package, 0, "run", "Copy", "Drop", 100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let source = format!(
        "{prefix}fn write(value:Owned,p:Ref)->bool {{p.as(*bool).* = false;true}}fn take(p:Ref,.result:bool=write(Owned {{}},p))->bool {{result}}"
    );
    let error =
        execution_support::compile_with_source_bindings(&source, &Default::default(), selectors)
            .unwrap_err();
    assert!(
        error.contains("P3 stability/termination evidence"),
        "{error}"
    );
}

#[test]
fn default_pointer_purity_does_not_authorize_moved_targets() {
    let source = "trait Copy {}trait Drop {fn drop(*self);}newtype Ref=*bool;impl Copy for Ref {}struct Owned {}impl Drop for Owned {fn drop(*self) {}}struct Cell {flag:bool}fn retire(value:Cell) {}fn read(value:Owned,p:Ref)->bool {p.as(*bool).*}fn take(p:Ref,.result:bool=read(Owned {},p))->bool {result}fn run()->bool {let cell=Cell {flag:true};let p=cell.flag.ref.as(Ref);retire(cell);take(p)}";
    let package = execution_support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    let error = luna::execution::interpret_with_protocols_in_package(
        &package, 0, "run", "Copy", "Drop", 100_000,
    )
    .unwrap_err();
    assert!(
        error.contains("memory call target unavailable: NotPresent"),
        "{error}"
    );
}

#[test]
fn default_pointer_reads_require_complete_copy_evidence() {
    for (definition, construct, read) in [
        (
            "struct Flag {value:bool}",
            "Flag {value:true}",
            "let copied=p.*;copied.value",
        ),
        (
            "newtype Flag=bool;",
            "true.as(Flag)",
            "let copied=p.*;copied.as(bool)",
        ),
    ] {
        for (implementation, accepted) in [
            ("impl Copy for Flag {}", true),
            ("", false),
            (
                "impl Copy for Flag {}impl Drop for Flag {fn drop(*self) {}}",
                false,
            ),
        ] {
            let source = format!(
                "trait Copy {{}}trait Drop {{fn drop(*self);}}{definition}{implementation}struct Owned {{}}impl Drop for Owned {{fn drop(*self) {{}}}}fn read(value:Owned,p:*Flag)->bool {{{read}}}fn take(p:*Flag,.result:bool=read(Owned {{}},p))->bool {{result}}fn run()->bool {{let flag={construct};take(flag.ref)}}"
            );
            let result = execution_support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    copy: Some("input::Copy"),
                    drop: Some("input::Drop"),
                    ..Default::default()
                },
            );
            if accepted {
                let package = result.unwrap_or_else(|error| panic!("{source}: {error}"));
                assert_eq!(
                    luna::execution::interpret_with_protocols_in_package(
                        &package, 0, "run", "Copy", "Drop", 100_000
                    )
                    .unwrap(),
                    Value::Bool(true)
                );
            } else {
                let error = result.unwrap_err();
                assert!(
                    error.contains("P3 stability/termination evidence"),
                    "{source}: {error}"
                );
            }
        }
    }
}

#[test]
fn nominal_default_loads_deliver_each_calls_current_value() {
    for (definition, construct, consume, change) in [
        (
            "struct Flag {value:bool}",
            "Flag {value:true}",
            "value.value",
            "flag.value=false",
        ),
        (
            "newtype Flag=bool;",
            "true.as(Flag)",
            "value.as(bool)",
            "flag=false.as(Flag)",
        ),
    ] {
        for default in ["p.*", "load(p)"] {
            let source = format!(
                "trait Copy {{}}trait Drop {{fn drop(*self);}}{definition}impl Copy for Flag {{}}fn load(p:*Flag)->Flag {{p.*}}fn take(p:*Flag,.value:Flag={default})->bool {{{consume}}}fn run()->bool {{var flag={construct};let first=take(flag.ref);{change};let second=take(flag.ref);first and not second}}"
            );
            let package = execution_support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    copy: Some("input::Copy"),
                    drop: Some("input::Drop"),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(
                luna::execution::interpret_with_protocols_in_package(
                    &package, 0, "run", "Copy", "Drop", 100_000
                )
                .unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn pure_private_pointer_newtypes_keep_their_targets() {
    let prefix = "trait Copy {}trait Drop {fn drop(*self);}newtype Ref=*bool;impl Copy for Ref {}newtype Outer=Ref;impl Copy for Outer {}";
    for body in [
        "var flag=false;let p=flag.ref.as(Ref);p.as(*bool).* = true;flag",
        "var flag=false;let p=flag.ref.as(Ref).as(Outer);let q=p;q.as(Ref).as(*bool).* = true;flag",
        "var a=false;var b=false;let p=if true {a.ref.as(Ref)} else {b.ref.as(Ref)};p.as(*bool).* = true;a and not b",
        "var a=false;var b=false;var p=a.ref.as(Ref);var again=true;while again {p.as(*bool).* = true;p=b.ref.as(Ref);again=false;}a",
        "var flag=false;let p=flag.ref.as(Ref);p match {q=>q.as(*bool).* = true};flag",
    ] {
        let source = format!("{prefix}pure fn run()->bool {{{body}}}");
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::Copy"),
                drop: Some("input::Drop"),
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret_with_protocols_in_package(
                &package, 0, "run", "Copy", "Drop", 100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
    for body in [
        "let p=external.as(Ref);p.as(*bool).*",
        "let p=external.as(Ref).as(Outer);p.as(Ref).as(*bool).* = true;true",
        "var local=false;let p=if true {local.ref.as(Ref)} else {external.as(Ref)};p.as(*bool).*",
        "var local=false;var p=local.ref.as(Ref);var again=true;while again {p.as(*bool).* = true;p=external.as(Ref);again=false;}true",
    ] {
        let source = format!("{prefix}pure fn bad(external:*bool)->bool {{{body}}}");
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::Copy"),
                drop: Some("input::Drop"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("pure"), "{error}");
    }
}

#[test]
fn pure_private_pointer_slot_loads_preserve_all_target_origins() {
    let prefix = "trait Copy {}trait Drop {fn drop(*self);}newtype Ref=*bool;impl Copy for Ref {}";
    for body in [
        "var flag=false;let p=flag.ref;let slot=p.ref;slot.*.* = true;flag",
        "var flag=false;let p=flag.ref.as(Ref);let slot=p.ref;slot.*.as(*bool).* = true;flag",
        "var a=false;var b=false;let p=a.ref;let q=b.ref;let slot=if true {p.ref} else {q.ref};slot.*.* = true;a and not b",
        "var a=false;var b=false;var p=a.ref;let slot=p.ref;p=b.ref;slot.*.* = true;b and not a",
    ] {
        let source = format!("{prefix}pure fn run()->bool {{{body}}}");
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::Copy"),
                drop: Some("input::Drop"),
                ..Default::default()
            },
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            luna::execution::interpret_with_protocols_in_package(
                &package, 0, "run", "Copy", "Drop", 100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
    for body in [
        "let p=external;let slot=p.ref;slot.*.*",
        "var local=false;let p=local.ref;let q=external;let slot=if true {p.ref} else {q.ref};slot.*.*",
        "var local=false;let p=external;let q=local.ref;let slot=if true {p.ref} else {q.ref};slot.*.*",
        "var local=false;var p=local.ref;let slot=p.ref;p=external;slot.*.* = true;true",
    ] {
        let source = format!("{prefix}pure fn bad(external:*bool)->bool {{{body}}}");
        let error = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::Copy"),
                drop: Some("input::Drop"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("lacks P4 evidence"), "{error}");
    }
}

#[test]
fn pure_private_data_writes_check_copy_and_cleanup() {
    for (definition, initial, replacement, result) in [
        (
            "newtype Flag=bool;",
            "false.as(Flag)",
            "true.as(Flag)",
            "value.as(bool)",
        ),
        (
            "struct Flag {flag:bool}",
            "Flag {flag:false}",
            "Flag {flag:true}",
            "value.flag",
        ),
    ] {
        for (implementation, accepted) in [
            ("impl Copy for Flag {}", true),
            ("", false),
            (
                "impl Copy for Flag {}impl Drop for Flag {fn drop(*self) {}}",
                false,
            ),
        ] {
            let source = format!(
                "trait Copy {{}}trait Drop {{fn drop(*self);}}{definition}{implementation}pure fn run()->bool {{var value={initial};let p=value.ref;p.* = {replacement};{result}}}"
            );
            let compiled = execution_support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    copy: Some("input::Copy"),
                    drop: Some("input::Drop"),
                    ..Default::default()
                },
            );
            if accepted {
                let package = compiled.unwrap_or_else(|error| panic!("{source}: {error}"));
                assert_eq!(
                    luna::execution::interpret_with_protocols_in_package(
                        &package, 0, "run", "Copy", "Drop", 100_000
                    )
                    .unwrap(),
                    Value::Bool(true)
                );
            } else {
                let error = compiled.unwrap_err();
                assert!(error.contains("lacks P4 evidence"), "{error}");
            }
        }
    }
}

#[test]
fn pure_data_write_rule_does_not_overwrite_pointer_or_code_evidence() {
    for source in [
        "trait Copy {}trait Drop {fn drop(*self);}newtype Ref=*bool;impl Copy for Ref {}pure fn bad(external:*bool)->bool {var local=false;var value=local.ref.as(Ref);let slot=value.ref;slot.* = external.as(Ref);value.as(*bool).*}",
        "trait Copy {}trait Drop {fn drop(*self);}typealias Action=fn()->bool;impl Copy for Action {}struct Cell {action:Action}impl Copy for Cell {}fn yes()->bool {true}pure fn bad(external:Action)->bool {var cell=Cell {action:yes};let slot=cell.ref;slot.* = Cell {action:external};cell.action()}",
    ] {
        let error = execution_support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::Copy"),
                drop: Some("input::Drop"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("lacks P4 evidence"), "{error}");
    }
}

#[test]
fn pure_nested_data_replacement_preserves_copied_enum_values() {
    let source = "trait Copy {}trait Drop {fn drop(*self);}enum Choice {yes(bool),no}impl Copy for Choice {}newtype Wrapped=Choice;impl Copy for Wrapped {}struct Cell {value:Wrapped}impl Copy for Cell {}pure fn run()->bool {var cell=Cell {value:Choice.no.as(Wrapped)};let old=cell.ref.*;cell.ref.* = Cell {value:Choice.yes(true).as(Wrapped)};let before=old.value.as(Choice) match {.no=>true,.yes(_)=>false};let after=cell.value.as(Choice) match {.yes(flag)=>flag,.no=>false};before and after}";
    let package = execution_support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols_in_package(
            &package, 0, "run", "Copy", "Drop", 100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let missing_copy = source.replace("impl Copy for Choice {}", "");
    let error = execution_support::compile_with_source_bindings(
        &missing_copy,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("lacks P4 evidence"), "{error}");
}

#[test]
fn open_defaults_require_p3_even_when_unused_or_explicitly_overridden() {
    for supplier in [
        "pure fn spin()->bool {spin()}",
        "pure fn spin()->bool {while true {}true}",
        "pure fn spin()->bool {other()} pure fn other()->bool {spin()}",
    ] {
        for use_site in ["", "fn run()->bool {take(true,.value=true)}"] {
            let source = format!(
                "{supplier} fn take(comptime flag:bool,.value:bool=spin())->bool {{inline if flag {{value}} else {{false}}}} {use_site}"
            );
            let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
            assert!(
                error.contains("P3 stability/termination"),
                "{source}: {error}"
            );
        }
    }
}

#[test]
fn open_default_call_evidence_retains_input_dependencies_and_override_order() {
    let package=compile("pure fn invert(value:bool)->bool {not value}
        fn input()->bool {false} fn supplied()->bool {true}
        fn take(comptime flag:bool,first:bool,.second:bool=invert(first))->bool {inline if flag {second} else {first}}
        fn run()->bool {take(true,input())}
        fn override_value()->bool {take(true,input(),.second=supplied())}");
    for (entry, expected_calls) in [
        ("run", vec!["run", "input", "invert", "take"]),
        (
            "override_value",
            vec!["override_value", "input", "supplied", "take"],
        ),
    ] {
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, entry), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), expected_calls);
        luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
    }
}

#[test]
fn open_defaults_can_use_completed_pure_open_suppliers() {
    let package=compile("pure fn supplier(comptime flag:bool)->bool {inline if flag {true} else {false}}
        fn take(comptime flag:bool,.value:bool=supplier(true))->bool {inline if flag {value} else {false}}
        fn run()->bool {take(true)}");
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), vec!["run", "supplier", "take"]);
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
}

#[test]
fn open_default_call_graph_rejects_staged_recursion_and_external_writes() {
    for source in [
        "pure fn spin()->bool {spin()} pure fn supplier(comptime flag:bool)->bool {inline if flag {spin()} else {true}} fn take(comptime flag:bool,.value:bool=supplier(true))->bool {inline if flag {value} else {false}}",
        "fn mutate(p:*bool)->bool {p.* = true;true} fn take(comptime flag:bool,p:*bool,.value:bool=mutate(p))->bool {inline if flag {value} else {false}}",
    ] {
        let error = compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(error.contains("P3 stability/termination"), "{error}");
    }
}

#[test]
fn pure_open_recursion_is_p4_without_becoming_p3() {
    let definitions =
        "pure fn spin(comptime flag:bool)->bool {inline if flag {spin(flag)} else {true}}";
    let package = compile(&format!(
        "{definitions} fn run()->bool {{spin(true)}} fn finish()->bool {{spin(false)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "finish", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mut vm = Interpreter::new(
        &package,
        Budget {
            steps: 100,
            ..Budget::default()
        },
    )
    .unwrap();
    let error = vm.invoke(id(&package, "run"), vec![]).unwrap_err();
    assert_eq!(error.kind, comptime_interpretor::ErrorKind::StepsExhausted);
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let error = compile_with_protocols(
        &format!("{definitions} fn take(.value:bool=spin(true))->bool {{value}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn recursive_open_behavior_keeps_unselected_effects_and_stage_checks() {
    let effect = "pure fn recur(comptime enabled:bool,done:bool,p:*bool)->bool {inline if enabled {if done {true} else {recur(enabled,true,p)}} else {p.* = true;false}}";
    let error = compile_with_protocols(effect, &Default::default()).unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
    let stage = "pure fn recur(comptime enabled:bool,done:bool)->bool {inline if enabled {if done {true} else {recur(done,true)}} else {false}}";
    let error = compile_with_protocols(stage, &Default::default()).unwrap_err();
    assert!(error.contains("runtime parameter"), "{error}");
}

#[test]
fn recursive_p4_checks_each_changed_callback_environment() {
    let common="fn clean(p:*bool)->bool {true}
        fn visit(callback:fn(*bool)->bool,p:*bool,done:bool)->bool {if done {callback(p)} else {visit(next,p,true)}}
        pure fn caller(p:*bool)->bool {visit(clean,p,false)}";
    let package = compile(&format!(
        "{common} fn next(p:*bool)->bool {{true}} fn run()->bool {{var flag=false;caller(flag.ref)}}"
    ));
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "caller", "visit", "visit", "next"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let error = compile_with_protocols(
        &format!("{common} fn next(p:*bool)->bool {{p.* = true;true}}"),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn changed_recursive_callbacks_do_not_establish_p3_termination() {
    let source = "fn clean()->bool {true} fn next()->bool {false}
        fn visit(callback:fn()->bool,done:bool)->bool {if done {callback()} else {visit(next,true)}}
        fn take(.value:bool=visit(clean,false))->bool {value}";
    let error = compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("P3 stability/termination"), "{error}");
}

#[test]
fn recursive_callbacks_keep_factory_capture_identities_distinct() {
    let common="trait Copy {} trait Drop {fn drop(*self);} typealias Wrapped=%{callback:fn(*bool)->bool} fn(*bool)->bool;
        fn wrap(callback:fn(*bool)->bool)->Wrapped {|p| callback(p)}
        fn clean(p:*bool)->bool {true}
        fn visit(callback:Wrapped,p:*bool,done:bool)->bool {if done {callback(p)} else {visit(wrap(next),p,true)}}
        pure fn caller(p:*bool)->bool {visit(wrap(clean),p,false)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let package=execution_support::compile_with_source_bindings(&format!("{common} fn next(p:*bool)->bool {{true}} fn run()->bool {{var flag=false;caller(flag.ref)}}"),&Default::default(),selectors).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["run", "caller", "wrap", "visit", "wrap", "visit", "next"]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let error = execution_support::compile_with_source_bindings(
        &format!("{common} fn next(p:*bool)->bool {{p.* = true;true}}"),
        &Default::default(),
        selectors,
    )
    .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
    let unknown = common.replace(
        "pure fn caller(p:*bool)->bool {visit(wrap(clean),p,false)}",
        "pure fn caller(callback:Wrapped,p:*bool)->bool {visit(callback,p,false)}",
    );
    let error = execution_support::compile_with_source_bindings(
        &format!("{unknown} fn next(p:*bool)->bool {{true}}"),
        &Default::default(),
        selectors,
    )
    .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn recursive_nested_closure_factories_preserve_each_capture_layer() {
    let common="trait Copy {} trait Drop {fn drop(*self);}
        typealias Wrapped=%{callback:fn(*bool)->bool} fn(*bool)->bool;
        typealias Nested=%{inner:Wrapped} fn(*bool)->bool;
        fn wrap(callback:fn(*bool)->bool)->Wrapped {|p| callback(p)}
        fn nest(inner:Wrapped)->Nested {|p| inner(p)}
        fn clean(p:*bool)->bool {true}
        fn visit(callback:Nested,p:*bool,done:bool)->bool {if done {callback(p)} else {visit(nest(wrap(next)),p,true)}}
        pure fn caller(p:*bool)->bool {visit(nest(wrap(clean)),p,false)}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    let package=execution_support::compile_with_source_bindings(&format!("{common} fn next(p:*bool)->bool {{true}} fn run()->bool {{var flag=false;caller(flag.ref)}}"),&Default::default(),selectors).unwrap();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec![
            "run", "caller", "wrap", "nest", "visit", "wrap", "nest", "visit", "next"
        ]
    );
    luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
    let error = execution_support::compile_with_source_bindings(
        &format!("{common} fn next(p:*bool)->bool {{p.* = true;true}}"),
        &Default::default(),
        selectors,
    )
    .unwrap_err();
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn closure_capture_code_evidence_preserves_absent_enum_payloads() {
    let common="trait Copy {} trait Drop {fn drop(*self);}
        enum Choice {none,some(fn(*bool)->bool)}
        typealias Wrapped=%{choice:Choice} fn(*bool)->bool;
        fn wrap(choice:Choice)->Wrapped {|p| choice match {.none=>true,.some(callback)=>callback(p)}}
        fn clean(p:*bool)->bool {true} fn dirty(p:*bool)->bool {p.* = true;true}";
    let selectors = luna::protocols::type_constructors::Selectors {
        copy: Some("input::Copy"),
        drop: Some("input::Drop"),
        ..Default::default()
    };
    for choice in [
        "Choice.none",
        "Choice.some(clean)",
        "if flag {Choice.none} else {Choice.some(clean)}",
    ] {
        let source = format!(
            "{common} pure fn caller(flag:bool,p:*bool)->bool {{wrap(do {{{choice}}})(p)}} fn run()->bool {{var value=false;caller(true,value.ref)}} fn other()->bool {{var value=false;caller(false,value.ref)}}"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            selectors,
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        for entry in ["run", "other"] {
            assert_eq!(
                luna::execution::interpret(&package, entry, 100_000).unwrap(),
                Value::Bool(true)
            );
            luna::instantiation::instantiate(&package, entry, 200_000).unwrap();
        }
    }
    for choice in [
        "Choice.some(dirty)",
        "if flag {Choice.none} else {Choice.some(dirty)}",
        "Choice.some(callback)",
    ] {
        let source = format!(
            "{common} pure fn caller(flag:bool,p:*bool,callback:fn(*bool)->bool)->bool {{wrap(do {{{choice}}})(p)}}"
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
fn pure_error_construction_propagation_and_catches_keep_runtime_behavior() {
    let package=compile_error_purity("enum Failure {Bad,}
        pure fn input(failed:bool)->!Failure bool {if failed {error Failure.Bad}else{true}}
        pure fn propagate(failed:bool)->!Failure bool {input(failed)!}
        pure fn recover(failed:bool)->bool {propagate(failed)! {catch _=>{},.Bad=>{},catch _=>false,}}
        fn run()->bool {recover(false) and not recover(true)}").unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_error_payload_computation_cannot_hide_external_writes() {
    let failure = compile_with_protocols(
        "fn mutate(target:*bool)->bool {target.* = true;false}
        pure fn bad(target:*bool)->!bool bool {error mutate(target)}",
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        failure.contains("pure") || failure.contains("purity"),
        "{failure}"
    );
}

fn compile_error_purity(source: &str) -> Result<nir::program::Package, String> {
    execution_support::compile_with_tuple_selectors(
        &format!("trait Copy {{}} trait Drop {{fn drop(*self);}} {source}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
            ..Default::default()
        },
    )
}

#[test]
fn pure_error_callable_payloads_survive_row_reordering_and_pattern_projection() {
    let package=compile_error_purity("typealias Action=fn()->bool;
        enum Code {Run(Action)} enum Other {Missing,}
        fn good()->bool {true}
        pure fn make()->![Code,Other] bool {error Code.Run(good)}
        pure fn reordered()->![Other,Code] bool {make()}
        pure fn invoke()->bool {reordered() match {value! => value,error Code.Run(action)=>action(),error Other.Missing=>false,}}
        fn run()->bool {invoke()}").unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn pure_error_transport_does_not_make_impure_callbacks_pure() {
    let source="typealias Action=fn(*bool)->bool;
        fn dirty(target:*bool)->bool {target.* = true;true}
        pure fn make()->!Action Action {error dirty}
        pure fn invoke(target:*bool)->bool {make() match {action! => action(target),error action=>action(target),}}";
    let failure = compile_error_purity(source).unwrap_err();
    assert!(
        failure.contains("invoke") && failure.contains("P4 evidence"),
        "{failure}"
    );
    compile_error_purity(
        "typealias Action=fn(*bool)->bool;
        fn dirty(target:*bool)->bool {target.* = true;true}
        pure fn make()->!Action Action {error dirty}",
    )
    .unwrap();
}

#[test]
fn pure_error_loop_tracks_callback_updates_across_iterations() {
    let package=compile_error_purity("typealias Action=fn()->bool;fn good()->bool {true}
        pure fn run()->bool {var packet:!Action Action=error good;var again=true;var answer=false;
            while again {packet match {action! => {answer=action();},error action=>{answer=action();}};
                packet=good;again=false;}answer}").unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
    let failure=compile_error_purity("typealias Action=fn(*bool)->bool;
        fn good(target:*bool)->bool {true} fn dirty(target:*bool)->bool {target.* = true;true}
        pure fn bad(target:*bool)->bool {var packet:!Action Action=error good;var first=true;var again=true;
            while again {packet match {action! => {action(target);},error action=>{action(target);}};
                if first {packet=error dirty;first=false;}else{packet=good;again=false;}}true}").unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}

#[test]
fn pure_error_transfer_does_not_authorize_impure_payload_cleanup() {
    let common = "struct Owned {target:*bool} impl Drop for Owned {fn drop(*self) {self.target.*.* = true;}}";
    compile_error_purity(&format!(
        "{common}
        pure fn wrap(value:Owned)->!Owned bool {{error value}}
        pure fn forward(value:!Owned bool)->!Owned bool {{value!}}"
    ))
    .unwrap();
    let failure = compile_error_purity(&format!(
        "{common}
        pure fn discard(value:!Owned bool)->bool {{value! {{_=>true,}}}}"
    ))
    .unwrap_err();
    assert!(
        failure.contains("discard") && failure.contains("P4 evidence"),
        "{failure}"
    );
}

#[test]
fn pure_error_constructor_does_not_invent_identity_for_unknown_callback() {
    let failure = compile_error_purity(
        "typealias Action=fn(*bool)->bool;
        pure fn invoke(action:Action,target:*bool)->bool {
            let packet:!Action Action=error action;
            packet match {callback! => callback(target),error callback=>callback(target),}}",
    )
    .unwrap_err();
    assert!(failure.contains("P4 evidence"), "{failure}");
}

#[test]
fn pure_error_runtime_defaults_check_payload_effects() {
    let package = compile_error_purity(
        "fn accept(.value:!bool bool=error true)->bool {
            value match {result! => result,error result=>result,}}
        fn run()->bool {accept()}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile_error_purity(
        "fn dirty(target:*bool)->bool {target.* = true;true}
        fn accept(target:*bool,.value:!bool bool=error dirty(target))->bool {true}",
    )
    .unwrap_err();
    assert!(
        failure.contains("default") || failure.contains("finite"),
        "{failure}"
    );
}

#[test]
fn pure_error_cleanup_precision_keeps_unmoved_sibling_obligations() {
    let common = "struct Owned {target:*bool} impl Drop for Owned {fn drop(*self) {self.target.*.* = true;}}";
    compile_error_purity(&format!(
        "{common}
        pure fn transfer(value:Tuple<Owned,bool>)->!Owned bool {{let (owner,flag)=value;error owner}}"
    ))
    .unwrap();
    let failure = compile_error_purity(&format!(
        "{common}
        pure fn lose(value:Tuple<Owned,Owned>)->!Owned bool {{let (first,second)=value;error first}}"
    ))
    .unwrap_err();
    assert!(
        failure.contains("lose") && failure.contains("P4 evidence"),
        "{failure}"
    );
}

#[test]
fn pure_product_error_transport_preserves_callable_field_identity() {
    use comptime_interpretor::Value;
    let source="enum Failure where T {Bad(T)} enum Other {Bad}
        pure fn sample()->![Other,Failure<bool>] bool {error Failure<bool>.Bad(true)}
        pure fn action()->bool {true}
        pure fn forward(value:Tuple<![Failure<T>,Other] bool,fn()->bool>)->Tuple<![Failure<T>,Other] bool,fn()->bool> where T {value}
        pure fn run()->bool {let (result,callback)=forward((sample(),action));
            result! {Failure<bool>.Bad(_)=>callback(),Other.Bad=>false}}";
    let package = compile_error_purity(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn owned_defaults_preserve_symbolic_copy_parameter_domains() {
    let source = "trait Copy {} trait Drop {fn drop(*self);}
        struct Flag where tag:bool {value:bool}
        impl Copy for Flag<tag> where tag:bool {}
        struct Owned {}
        impl Drop for Owned {fn drop(*self) {}}
        fn consume(value:Owned, flag:Flag<tag>)->bool where tag:bool {flag.value}
        fn take(flag:Flag<which>, .result:bool=consume(Owned{},flag))->bool
            where which:bool {result and flag.value}
        fn run()->bool {take(Flag<true>{value:true})}";
    let package = execution_support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000,)
            .unwrap(),
        Value::Bool(true)
    );
    for replacement in ["", "impl Copy for Flag<false> {}"] {
        let rejected = source.replace("impl Copy for Flag<tag> where tag:bool {}", replacement);
        let error = execution_support::compile_with_source_bindings(
            &rejected,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::Copy"),
                drop: Some("input::Drop"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("P3 stability/termination evidence"),
            "{error}"
        );
    }
}
