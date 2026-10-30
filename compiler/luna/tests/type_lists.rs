//! Contextual meta.List<Type> construction retains expression evaluation.
#[path = "support/execution.rs"]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::{Primitive as P, Term};

const PREFIX: &str = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}";

fn compile(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!("{PREFIX}{body}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            ..Default::default()
        },
    )
}

#[test]
fn callable_clause_lists_preserve_domains_and_dependent_type_results() {
    let package = compile(
        "comptime fn First()->Type where Es:List<Type> {Es[0]}
         comptime fn Second()->Type where Es:List<Type> {Es[1]}
         comptime fn Defaulted()->Type where .Es:List<Type> = [bool] {Es[0]}
         fn run()->bool {
             let a:First<[bool,.u8'builtin]>()=true;
             let b:Second<[.u8'builtin,bool]>()=true;
             let c:Defaulted<>()=true;
             let d:Defaulted<.Es=[bool,.u8'builtin]>()=true;
             a and b and c and d
         }",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for input in ["[]", "[true]", "bool"] {
        assert!(
            compile(&format!(
                "comptime fn First()->Type where Es:List<Type> {{Es[0]}}
             fn run()->bool {{let value:First<{input}>()=true;value}}"
            ))
            .is_err(),
            "{input}"
        );
    }
}

#[test]
fn type_list_clauses_specialize_deferred_runtime_signatures() {
    let source = "comptime fn First()->Type where Es:List<Type> {Es[0]}
        comptime fn Forward()->Type where Es:List<Type> {First<Es>()}
        fn identity(value:defer First<Es>())->defer First<Es>() where Es:List<Type> {value}
        fn run()->bool {
            let value:Forward<[bool]>()=true;
            let flag=identity<[bool]>(value);
            let byte=identity<[.u8'builtin]>(42);
            flag and byte == 42
        }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for replacement in ["identity<[bool]>(42)", "identity<[]>(value)"] {
        assert!(
            compile(&source.replace("identity<[bool]>(value)", replacement)).is_err(),
            "{replacement}"
        );
    }
    // Rejection must not poison later signature instances.
    assert!(compile(source).is_ok());
}

#[test]
fn concrete_list_clause_instances_can_be_passed_as_function_values() {
    let source = "comptime fn First()->Type where Es:List<Type> {Es[0]}
        fn identity(value:defer First<Es>())->defer First<Es>() where Es:List<Type> {value}
        fn invoke(callback:fn(bool)->bool)->bool {callback(true)}
        fn run()->bool {
            let flag:fn(bool)->bool=identity<[bool]>;
            let byte:fn(.u8'builtin)->.u8'builtin=identity<[.u8'builtin]>;
            let captured:fn(bool)->bool=identity<[bool]>;
            let closure=||captured(true);
            invoke(flag) and byte(42)==42 and closure()
        }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace(
            "let flag:fn(bool)->bool=identity<[bool]>",
            "let flag:fn(bool)->bool=identity<[.u8'builtin]>",
        ),
        source.replace("fn identity(", "unsafe fn identity("),
        source.replace(
            "value:defer First<Es>()",
            "value:defer First<Es>(),.spare:bool=true",
        ),
        source.replace("identity<[bool]>", "identity<[]>"),
    ] {
        assert!(compile(&invalid).is_err(), "{invalid}");
    }
    assert!(compile(source).is_ok());
}

#[test]
fn open_function_values_preserve_program_contract_boundaries() {
    let prefix = "comptime fn First()->Type where Es:List<Type> {Es[0]}
        fn guarded(gate:bool,value:defer First<Es>())->defer First<Es>()
        where Es:List<Type>, requires gate {value}";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{guarded<[bool]>(true,true)}}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for body in [
        "let callback:fn(bool,bool)->bool=guarded<[bool]>;callback(true,true)",
        "let callback=guarded<[bool]>;true",
    ] {
        let failure = compile(&format!("{prefix} fn run()->bool {{{body}}}")).unwrap_err();
        assert!(failure.contains("erase program preconditions"), "{failure}");
    }
    assert!(
        compile(&format!(
            "{prefix} fn run()->bool {{guarded<[bool]>(false,true)}}"
        ))
        .is_err()
    );
    assert!(
        compile(&format!(
            "{prefix} fn run()->bool {{guarded<[bool]>(true,true)}}"
        ))
        .is_ok()
    );
}

#[test]
fn open_function_value_factories_preserve_type_captures_and_evaluation_order() {
    let package = compile(
        "comptime fn First()->Type where Es:List<Type> {Es[0]}
         fn identity(value:defer First<Es>())->defer First<Es>() where Es:List<Type> {value}
         fn factory()->fn(T)->T where T {identity<[T]>}
         fn run()->bool {
             var callee=false;
             var argument=false;
             let result=(do {
                 assumes not callee and not argument;
                 callee=true;
                 factory<bool>()
             })(do {
                 assumes callee and not argument;
                 argument=true;
                 true
             });
             let byte=factory<.u8'builtin>();
             result and callee and argument and byte(42)==42
         }",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn defaulted_list_clauses_form_function_values_before_context_checking() {
    let source = "comptime fn First()->Type where Es:List<Type> {Es[0]}
        fn defaulted(value:defer First<Es>())->defer First<Es>() where .Es:List<Type> = [bool] {value}
        fn invoke(callback:fn(bool)->bool)->bool {callback(true)}
        fn run()->bool {
            let annotated:fn(bool)->bool=defaulted;
            let inferred=defaulted;
            let explicit:fn(.u8'builtin)->.u8'builtin=defaulted<.Es=[.u8'builtin]>;
            annotated(true) and inferred(true) and explicit(42)==42 and invoke(defaulted)
        }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(&source.replace(
        "let annotated:fn(bool)->bool=defaulted",
        "let annotated:fn(.u8'builtin)->.u8'builtin=defaulted",
    ))
    .unwrap_err();
    assert!(!failure.contains("not a supported callable"), "{failure}");
    assert!(
        compile(&source.replace(
            "value:defer First<Es>()",
            "value:defer First<Es>(),.spare:bool=true"
        ))
        .is_err()
    );
}

#[test]
fn explicit_list_clause_function_values_skip_failing_default_suppliers() {
    let prefix = "comptime fn First()->Type where Es:List<Type> {Es[0]}
        comptime fn unavailable()->List<Type> {assumes false;[bool]}
        fn defaulted(value:defer First<Es>())->defer First<Es>() where .Es:List<Type> = unavailable() {value}";
    let explicit =
        format!("{prefix} fn run()->bool {{let callback=defaulted<.Es=[bool]>;callback(true)}}");
    assert_eq!(
        luna::execution::interpret(&compile(&explicit).unwrap(), "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let failure = compile(&format!(
        "{prefix} fn run()->bool {{let callback=defaulted;callback(true)}}"
    ))
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    assert!(compile(&explicit).is_ok());
}

#[test]
fn callable_context_infers_known_domains_before_dependent_list_defaults() {
    let prefix = "comptime fn First()->Type where Es:List<Type> {Es[0]}
        fn forward(value:T)->defer First<Es>() where T,.Es:List<Type> = [T] {value}
        fn reverse(value:defer First<Es>())->T where T,.Es:List<Type> = [T] {value}
        fn invoke(callback:fn(bool)->bool)->bool {callback(true)}";
    let package = compile(&format!(
        "{prefix}
        fn run()->bool {{
            let flag:fn(bool)->bool=forward;
            let byte:fn(.u8'builtin)->.u8'builtin=forward;
            let from_result:fn(bool)->bool=reverse;
            flag(true) and byte(42)==42 and from_result(true) and invoke(forward)
        }}"
    ))
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for declaration in [
        "let callback:fn(bool)->.u8'builtin=forward;",
        "let callback:fn(.u8'builtin)->bool=reverse;",
        "let callback:fn(bool,bool)->bool=forward;",
        "let callback=forward;",
    ] {
        assert!(
            compile(&format!("{prefix} fn run()->bool {{{declaration} true}}")).is_err(),
            "{declaration}"
        );
    }
    // Unknown annotations do not authorize solving a computation backwards.
    assert!(
        compile(
            "comptime fn First()->Type where Es:List<Type> {Es[0]}
        fn unknown(value:defer First<Es>())->defer First<Es>() where Es:List<Type> {value}
        fn run()->bool {let callback:fn(bool)->bool=unknown;callback(true)}"
        )
        .is_err()
    );
}

#[test]
fn open_function_value_factories_capture_declared_values_with_their_domains() {
    let package = compile(
        "comptime fn First()->Type where Es:List<Type> {Es[0]}
         fn choose(value:defer First<Es>(),state:*bool)->defer First<Es>()
         where Es:List<Type>,flag:bool {state.* = flag;value}
         fn factory()->fn(bool,*bool)->bool where flag:bool {choose<[bool],flag>}
         fn run()->bool {
             var state=false;
             let yes=factory<true>();
             let no=factory<false>();
             let a=yes(true,state.ref);
             assumes state;
             let b=no(true,state.ref);
             a and b and not state
         }",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let metadata = package
        .function_parameters
        .iter()
        .find(|entry| {
            entry.declaration.package == u64::MAX - 1
                && entry.domains.contains(&Term::Primitive(P::Bool))
        })
        .expect("generated value capture must retain a declaration domain");
    assert!(
        metadata
            .binders()
            .unwrap()
            .iter()
            .any(|binder| binder.sort == nir::Sort::ComptimeValue)
    );
    let mut invalid = package.clone();
    let metadata = invalid
        .function_parameters
        .iter_mut()
        .find(|entry| entry.declaration == metadata.declaration)
        .unwrap();
    let index = metadata
        .domains
        .iter()
        .position(|domain| *domain == Term::Primitive(P::Bool))
        .unwrap();
    metadata.domains[index] = Term::Primitive(P::U8);
    assert!(Interpreter::new(&invalid, Budget::default()).is_err());
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn open_function_value_factories_keep_integer_capture_instances_distinct() {
    let package = compile(
        "comptime fn First()->Type where Es:List<Type> {Es[0]}
         fn choose(value:defer First<Es>(),state:*.u8'builtin)->defer First<Es>()
         where Es:List<Type>,tag:.u8'builtin {state.* = tag;value}
         fn factory()->fn(bool,*.u8'builtin)->bool where tag:.u8'builtin {choose<[bool],tag>}
         fn run()->bool {
             var state:.u8'builtin=0;
             let first=factory<7>();
             let second=factory<255>();
             let a=first(true,state.ref);
             assumes state == 7;
             let b=second(true,state.ref);
             a and b and state == 255
         }",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn unused_value_clauses_do_not_suspend_completed_annotations() {
    let package = compile(
        "comptime fn Concrete()->Type {bool}
         pure fn keep(value:defer Concrete())->defer Concrete() where ignored:bool {value}
         fn run()->bool {keep<true>(true) and keep<false>(true)}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_list_literals_construct_empty_and_ordered_values() {
    for (body, types) in [
        ("[]", vec![]),
        ("[bool,.u8'builtin,bool]", vec![P::Bool, P::U8, P::Bool]),
    ] {
        let package = compile(&format!("comptime fn make()->List<Type> {{{body}}}")).unwrap();
        assert!(package.to_string().contains("(type-list (elements"));
        assert_eq!(
            luna::invocation::invoke(&package, "make", 100_000).unwrap(),
            support::type_list_value(&package, types.into_iter().map(Term::Primitive).collect())
        );
    }
}

#[test]
fn type_list_elements_execute_once_and_stop_at_return() {
    let package = compile(
        "comptime fn First()->Type {bool}
        comptime fn Second()->Type {.u8'builtin}
        comptime fn make()->List<Type> {[First(),Second(),First()]}
        comptime fn early()->List<Type> {[First(),do {return [];},Second()]}",
    )
    .unwrap();
    for (entry, expected, calls) in [
        (
            "make",
            vec![P::Bool, P::U8, P::Bool],
            vec!["make", "First", "Second", "First"],
        ),
        ("early", vec![], vec!["early", "First"]),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            support::type_list_value(
                &package,
                expected.into_iter().map(Term::Primitive).collect()
            )
        );
        assert_eq!(support::calls(&package, &vm), calls);
    }
}

#[test]
fn type_list_literal_requires_type_elements_and_bound_domain() {
    for source in [
        "comptime fn bad()->List<Type> {[true]}",
        "comptime fn bad()->List<Type> {[7]}",
        "comptime fn bad()->List<bool> {[bool]}",
        "fn bad()->List<Type> {[bool]}",
        "fn runtime()->Type {bool} comptime fn bad()->List<Type> {[runtime()]}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    assert!(
        support::compile_with_protocols(
            &format!("{PREFIX}comptime fn bad()->List<Type> {{[bool]}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn value_list_literals_preserve_order_and_early_exit() {
    let package = compile(
        "comptime fn first()->bool {true}
         comptime fn second()->bool {false}
         comptime fn make()->List<bool> {[first(),second(),first()]}
         comptime fn early()->List<bool> {[first(),do {return [];},second()]}",
    )
    .unwrap();
    for (entry, expected, calls) in [
        (
            "make",
            vec![Value::Bool(true), Value::Bool(false), Value::Bool(true)],
            vec!["make", "first", "second", "first"],
        ),
        ("early", vec![], vec!["early", "first"]),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Value::List { ty, values } = vm.invoke(support::id(&package, entry), vec![]).unwrap()
        else {
            panic!("expected immutable list")
        };
        assert!(matches!(ty, Term::NormalForm { .. }));
        assert_eq!(values, expected);
        assert_eq!(support::calls(&package, &vm), calls);
    }
}

#[test]
fn type_lists_and_tuple_packets_keep_distinct_domains() {
    let compile_packet = |body: &str| {
        support::compile_with_source_bindings(
            &format!("{PREFIX} pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}} {body}"),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                tuple: Some("input.Tuple"),
                ..Default::default()
            },
        )
    };
    for (domain, value, elements) in [
        ("List<Type>", "[bool,.u8'builtin]", vec![P::Bool, P::U8]),
        (
            "Tuple<Type,Type>",
            "(bool,.u8'builtin)",
            vec![P::Bool, P::U8],
        ),
        ("List<Type>", "[]", vec![]),
        ("Tuple<>", "()", vec![]),
    ] {
        let package = compile_packet(&format!("comptime fn make()->{domain} {{{value}}}")).unwrap();
        let types = elements
            .into_iter()
            .map(Term::Primitive)
            .collect::<Vec<_>>();
        let expected = if domain == "List<Type>" {
            support::type_list_value(&package, types)
        } else {
            Value::Tuple(types.into_iter().map(Value::Type).collect())
        };
        assert_eq!(
            luna::invocation::invoke(&package, "make", 100_000).unwrap(),
            expected
        );
    }
    for (domain, value) in [
        ("Tuple<Type,Type>", "[bool,bool]"),
        ("List<Type>", "(bool,bool)"),
        ("Tuple<>", "[]"),
        ("List<Type>", "()"),
    ] {
        let error =
            compile_packet(&format!("comptime fn bad()->{domain} {{{value}}}")).unwrap_err();
        assert!(!error.contains("value, not a type"), "{error}");
        assert!(!error.contains("unresolved identifier"), "{error}");
    }
}

#[test]
fn type_list_literals_keep_generic_elements_constants_and_indexing() {
    let package = compile(
        "comptime fn collect(T:Type)->List<Type> {[T,bool]}
        comptime fn generic()->List<Type> where T {[T,bool]}
        const fixed:List<Type> = [bool,.u8'builtin];
        comptime fn Selected()->Type {let values=collect(.u8'builtin);values[0]}
        comptime fn GenericSelected()->Type {let values=generic<.u8'builtin>();values[0]}
        comptime fn ConstantSelected()->Type {fixed[1]}
        comptime fn nested(T:Type)->List<Type> {const values:List<Type> = [T,bool];values}",
    )
    .unwrap();
    for name in ["Selected", "GenericSelected", "ConstantSelected"] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            Value::Type(Term::Primitive(P::U8))
        );
    }
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(
            support::id(&package, "nested"),
            vec![Value::Type(Term::Primitive(P::U8))]
        )
        .unwrap(),
        support::type_list_value(
            &package,
            vec![Term::Primitive(P::U8), Term::Primitive(P::Bool)]
        )
    );
}

#[test]
fn constructed_type_lists_feed_normal_heads_without_unfolding() {
    let package = compile(
        "pure comptime fn Product<...Es:List<Type>>->Type {bool}
        comptime fn First()->Type {.u8'builtin}
        comptime fn Second()->Type {bool}
        comptime fn Build()->Type {Product<...[First(),Second()]>}
        comptime fn Named()->Type {Product<.Es=[First(),Second()]>}",
    )
    .unwrap();
    let expected = Value::Type(Term::NormalForm {
        head: support::id(&package, "Product"),
        sort: nir::Sort::Type,
        arguments: vec![Term::TypeListValue(vec![
            Term::Primitive(P::U8),
            Term::Primitive(P::Bool),
        ])],
    });
    for name in ["Build", "Named"] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            expected
        );
        assert_eq!(support::calls(&package, &vm), vec![name, "First", "Second"]);
    }
}

#[test]
fn type_list_elements_preserve_state_changes_and_reject_bad_later_elements() {
    let source = "comptime fn Selected()->Type {
        var seen=false;
        let values:List<Type> = [do {seen=true;bool},do {assumes seen;.u8'builtin}];
        assumes seen;values[1]
    }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Selected", 100_000).unwrap(),
        Value::Type(Term::Primitive(P::U8))
    );
    let invalid = compile(&source.replace("seen=true;bool", "seen=false;bool")).unwrap();
    assert!(luna::invocation::invoke(&invalid, "Selected", 100_000).is_err());
    assert!(compile("comptime fn bad()->List<Type> {[do {return [];},true]}").is_err());
}

#[test]
fn generic_aliases_preserve_type_list_inputs_and_opaque_heads() {
    let package = compile(
        "pure comptime fn Product<...Es:List<Type>>->Type {bool}
        typealias Mixed<T> = Product<.Es=[T,bool,T]>;
        comptime fn Chosen()->Type {Mixed<.u8'builtin>}
        comptime fn Forward()->Type where T {Mixed<T>}
        comptime fn Forwarded()->Type {Forward<.u8'builtin>()}",
    )
    .unwrap();
    let expected = Value::Type(Term::NormalForm {
        head: support::id(&package, "Product"),
        sort: nir::Sort::Type,
        arguments: vec![Term::TypeListValue(vec![
            Term::Primitive(P::U8),
            Term::Primitive(P::Bool),
            Term::Primitive(P::U8),
        ])],
    });
    for name in ["Chosen", "Forwarded"] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            expected
        );
    }
}

#[test]
fn quoted_code_generators_use_type_list_data_at_construction_time() {
    let package = compile(
        "pure comptime fn build(T:Type)->< |- bool> {
            let types:List<Type> = [T,bool];
            if types[0] == types[1] {'(true)} else {'(false)}
        }
        fn run()->bool {~<build(bool)> and not ~<build(.u8'builtin)>}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_list_defaults_and_generator_updates_keep_invocation_state() {
    let package = compile(
        "comptime fn choose(.types:List<Type> = [bool,.u8'builtin])->Type {types[0]}
        comptime fn Defaulted()->Type {choose()}
        comptime fn Explicit()->Type {choose(.types=[.u8'builtin])}
        pure comptime fn build(T:Type)->< |- bool> {
            var types:List<Type> = [bool];
            types=[T,types[0]];
            if types[0] == types[1] {'(true)} else {'(false)}
        }
        fn run()->bool {~<build(bool)> and not ~<build(.u8'builtin)>}",
    )
    .unwrap();
    for (name, expected) in [("Defaulted", P::Bool), ("Explicit", P::U8)] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            Value::Type(Term::Primitive(expected))
        );
    }
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_list_index_failures_leave_later_invocations_usable() {
    let package = compile(
        "comptime fn pick(index:.Integer'builtin)->Type {
            let values:List<Type> = [bool,.u8'builtin];values[index]
        }",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let pick = support::id(&package, "pick");
    for position in [-1, 2, 17] {
        let error = vm
            .invoke(
                pick,
                vec![Value::Integer(comptime_runtime::Integer::from_i128(
                    position,
                ))],
            )
            .unwrap_err();
        assert_eq!(
            error.kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert!(error.source.is_some());
        assert_eq!(
            vm.invoke(
                pick,
                vec![Value::Integer(comptime_runtime::Integer::from_u128(1))]
            )
            .unwrap(),
            Value::Type(Term::Primitive(P::U8))
        );
    }
}

#[test]
fn type_list_index_captures_the_subject_before_index_side_effects() {
    let package = compile(
        "comptime fn pick()->Type {
            var values:List<Type> = [bool,.u8'builtin];
            let selected=values[do {values=[.u16'builtin,.u32'builtin];0}];
            if values[0] == .u16'builtin {selected} else {.u64'builtin}
        }
        comptime fn early()->Type {
            let values:List<Type> = [];
            values[do {return bool;}]
        }",
    )
    .unwrap();
    for name in ["pick", "early"] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            Value::Type(Term::Primitive(P::Bool))
        );
    }
}

#[test]
fn oversized_type_list_indices_report_bounds_and_preserve_invocation_recovery() {
    let package = compile(
        "comptime fn pick(index:.Integer'builtin)->Type {
            let values:List<Type> = [bool];values[index]
        }",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let entry = support::id(&package, "pick");
    let mut arithmetic = comptime_runtime::Budget::default();
    let huge = comptime_runtime::Integer::from_u128(u128::MAX)
        .add(&comptime_runtime::Integer::from_u128(1), &mut arithmetic)
        .unwrap();
    for index in [huge.clone(), huge.negate(&mut arithmetic).unwrap()] {
        let failure = vm.invoke(entry, vec![Value::Integer(index)]).unwrap_err();
        assert_eq!(
            failure.kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert!(failure.source.is_some());
        assert_eq!(
            vm.invoke(
                entry,
                vec![Value::Integer(comptime_runtime::Integer::zero())]
            )
            .unwrap(),
            Value::Type(Term::Primitive(P::Bool))
        );
    }
}

#[test]
fn compiler_bound_lists_cannot_enter_runtime_storage() {
    for source in [
        "fn invalid(value:List<Type>) {}",
        "fn invalid(value:List<bool>) {}",
        "struct Hidden {values:List<Type>} fn invalid(value:Hidden) {}",
        "fn invalid(value:*List<Type>) {}",
        "newtype Hidden=List<Type>; fn invalid(value:Hidden) {}",
        "enum Hidden {empty, values(List<Type>)} fn invalid(value:Hidden) {}",
        "fn invalid(action:fn(List<Type>)->bool) {}",
    ] {
        let error = compile(source).unwrap_err();
        assert!(error.contains("comptime-only"), "{source}: {error}");
    }
}

#[test]
fn tuple_storage_phase_uses_only_bound_structural_views() {
    let prefix = format!("{PREFIX} pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}} ");
    for (argument, accepted) in [
        ("bool", true),
        (".Integer'builtin", false),
        ("List<Type>", false),
        ("Tuple<bool>", true),
        ("Tuple<List<bool>>", false),
        ("*List<Type>", false),
    ] {
        let source = format!("{prefix} fn consume(value:Tuple<{argument}>) {{}}");
        let result = support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                tuple: Some("input.Tuple"),
                ..Default::default()
            },
        );
        if accepted {
            result.unwrap();
        } else {
            assert!(result.unwrap_err().contains("comptime-only"), "{argument}");
        }
    }
    // A spelling or a body returning a comptime-only type is not a layout contract.
    compile("pure comptime fn Identity<T>->Type {T} fn consume(value:Identity<Type>) {}").unwrap();
}

#[test]
fn comptime_list_loop_reads_local_indices_without_quadratic_copy_cost() {
    let elements = vec!["true"; 512].join(",");
    let package = compile(&format!(
        "comptime fn count()->.Integer'builtin {{
            let values:List<bool> = [{elements}];
            var index:.Integer'builtin = 0;
            var count:.Integer'builtin = 0;
            while index < 64 {{
                if values[index] {{count = count + 1;}}
                index = index + 1;
            }}
            count
        }}"
    ))
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "count", 20_000).unwrap(),
        Value::Integer(comptime_runtime::Integer::from_u128(64))
    );
}
