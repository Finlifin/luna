//! Clause expansion preserves opaque heads, routing and comptime producers.
#[path = "support/frozen_lists.rs"]
mod frozen_lists;
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

const TYPE: &str = "typealias Type=.Type'builtin;";

#[test]
fn bound_meta_list_values_do_not_depend_on_tuple_declarations() {
    let prefix = format!("{TYPE}pure comptime fn List<T>->Type {{T}}");
    let baseline = compile(&prefix);
    let protocols = elaboration::program::Protocols {
        meta_list: Some(baseline.normal_forms[0].declaration),
        ..Default::default()
    };
    let package = compile_with_protocols(
        &format!("{prefix}comptime fn pass(values:List<Type>)->List<Type> {{values}}"),
        &protocols,
    )
    .unwrap();
    assert_eq!(package.normal_forms.len(), 1);
    assert_eq!(
        package.normal_forms[0].role,
        nir::normal_forms::Role::TypeList
    );
    assert!(package.to_string().contains("(role type-list)"));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for types in [vec![], vec![nir::Term::Primitive(nir::Primitive::Bool)]] {
        let value = execution_support::type_list_value(&package, types);
        assert_eq!(
            vm.invoke(id(&package, "pass"), vec![value.clone()])
                .unwrap(),
            value
        );
    }
}

#[test]
fn nominal_and_alias_arguments_expand_literal_and_computed_packets() {
    let package=execution_support::compile_with_tuple(&format!("{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        enum Choice where A,B {{first(A),second(B)}}
        typealias Alias<A,B> = Pair<A,B>;
        comptime fn packet()->Tuple<Type,Type> {{(bool,.u8'builtin)}}
        typealias P=Alias<...packet()>;
        fn run()->bool {{let p:P=Pair<...(bool,.u8'builtin)> {{first:true,second:7}};
            let c:Choice<...packet()> = .first(p.first);c match {{.first(flag)=>flag,.second(_)=>false}}}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_function_and_method_instances_expand_without_erasing_identity() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        comptime fn packet()->Tuple<Type> {{(bool,)}}
        fn keep(value:T)->T where T {{value}}
        struct Helpers {{}}
        impl Helpers {{fn keep(value:T)->T where T {{value}}}}
        fn forward(value:T)->T where T {{keep<...(T,)>(value)}}
        fn run()->bool {{let callback=keep<...packet()>;
            callback(true) and forward<...(bool,)>(true) and Helpers.keep<...packet()>(true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn residual_type_factories_prepare_packets_once_in_source_order() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        comptime fn packet(T:Type)->Tuple<Type> {{(T,)}}
        comptime fn Last()->Type {{bool}}
        comptime fn Build(T:Type)->Type {{Pair<...packet(T),Last()>}}
        comptime fn Run()->Type {{Build(.u8'builtin)}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "Build", "packet", "Last"]);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        result
    );
    assert!(
        matches!(result,Value::Type(nir::Term::Nominal {arguments,..}) if arguments==vec![nir::Term::Primitive(nir::Primitive::U8),nir::Term::Primitive(nir::Primitive::Bool)])
    );
}

#[test]
fn opaque_head_spreads_do_not_invoke_the_head_body() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        pure comptime fn Identity<T>->Type {{T}}
        comptime fn packet(T:Type)->Tuple<Type> {{(T,)}}
        typealias Static=Identity<...(bool,)>;
        comptime fn Build(T:Type)->Type {{Identity<...packet(T)>}}
        comptime fn Run()->Type {{Build(bool)}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "Build", "packet"]);
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Identity"),
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Bool)],
        })
    );
}

#[test]
fn malformed_packets_and_wrong_arity_are_rejected() {
    for arguments in [
        "...true",
        "...(true,)",
        "...(bool,bool)",
        "...(.u8'builtin,)",
    ] {
        let source = format!(
            "{TYPE}struct Only where T, requires T == bool {{value:T}} typealias Bad=Only<{arguments}>;"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{arguments}"
        );
    }
}

#[test]
fn trait_selection_accepts_expanded_arguments_and_checks_requirements() {
    let package=compile(&format!("{TYPE}trait Select where T {{fn select(self,value:T)->T;}}
        struct Holder {{}}
        impl Select<...(bool,)> for Holder {{fn select(self,value:bool)->bool {{value}}}}
        fn apply(value:H)->bool where H :- Select<...(bool,)> {{value.as(Select<...(bool,)>).select(true)}}
        fn run()->bool {{apply(Holder {{}})}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn mixed_named_inputs_skip_optional_slots_and_defaults_see_prior_types() {
    let package=compile(&format!("{TYPE}
        struct Fields where A,.B:Type=A,C {{first:A,second:B,last:C}}
        typealias Chosen=Fields<...(bool,),.B=.u8'builtin,...(bool,)>;
        fn run()->bool {{let fields:Chosen=Fields<.B=.u8'builtin,...(bool,bool)> {{first:true,second:7,last:false}};
            let defaulted=Fields<...(bool,bool)> {{first:true,second:true,last:true}};
            fields.first and not fields.last and defaulted.second}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn empty_packets_execute_before_definition_scoped_defaults() {
    let package = compile(&format!(
        "{TYPE}
        comptime fn empty(...items:T)->T where T {{items}}
        comptime fn Default()->Type {{bool}}
        struct Holder where .T:Type=Default() {{value:T}}
        comptime fn Run()->Type {{Holder<...empty()>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "empty", "Default"]);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        result
    );
}

#[test]
fn residual_alias_spreads_keep_parameter_values_and_default_order() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        typealias Alias<A,.B:Type=A> = Pair<A,B>;
        comptime fn packet(T:Type)->Tuple<Type> {{(T,)}}
        comptime fn Build(T:Type)->Type {{Alias<...packet(T)>}}
        comptime fn Run()->Type {{Build(bool)}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert!(
        matches!(result,Value::Type(nir::Term::Nominal {arguments,..}) if arguments==vec![nir::Term::Primitive(nir::Primitive::Bool);2])
    );
}

#[test]
fn later_spread_operands_do_not_change_prepared_earlier_types() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        comptime fn packet(T:Type)->Tuple<Type> {{(T,)}}
        comptime fn Run()->Type {{var T=bool;Pair<...packet(T),do {{T=.u8'builtin;T}}>}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert!(
        matches!(result,Value::Type(nir::Term::Nominal {arguments,..}) if arguments==vec![nir::Term::Primitive(nir::Primitive::Bool),nir::Term::Primitive(nir::Primitive::U8)])
    );
}

#[test]
fn duplicate_named_slots_and_missing_spread_fields_reject() {
    for arguments in [
        "...(bool,),.B=bool,.B=bool",
        "...(bool,),.unknown=bool",
        "...()",
    ] {
        let source = format!(
            "{TYPE}struct Pair where A,.B:Type=A {{first:A,second:B}} typealias Bad=Pair<{arguments}>;"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{arguments}"
        );
    }
    let package = compile(&format!(
        "{TYPE}struct Pair where A,.B:Type=A {{first:A,second:B}}
        fn run()->bool {{let p=Pair<...(bool,)> {{first:true,second:false}};p.first}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn early_exit_during_literal_packet_prevents_remaining_producers() {
    let package = compile(&format!(
        "{TYPE}struct Pair where A,B {{first:A,second:B}}
        comptime fn Later()->Type {{.u8'builtin}}
        comptime fn Run()->Type {{Pair<...(do {{return bool;}},),Later()>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Run"]);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn packet_unpacking_uses_bound_tuple_view_without_unfolding() {
    let source = format!(
        "{TYPE}
        pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        struct Pair where A,B {{first:A,second:B}}
        comptime fn packet(T:Type)->Tuple<Type,Type> {{(T,bool)}}
        comptime fn Build(T:Type)->Type {{Pair<...packet(T)>}}
        comptime fn Run()->Type {{Build(.u8'builtin)}}
        typealias Concrete=Pair<...packet(bool)>;"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.List"),
        tuple: Some("input.Tuple"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(&source, &Default::default(), selectors)
            .unwrap();
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert!(
        matches!(result,Value::Type(nir::Term::Nominal {arguments,..}) if arguments==vec![nir::Term::Primitive(nir::Primitive::U8),nir::Term::Primitive(nir::Primitive::Bool)])
    );
}

#[test]
fn empty_literal_expansion_can_use_all_defaults() {
    let package = compile(&format!(
        "{TYPE}struct Holder where .T:Type=bool {{value:T}}
        typealias Chosen=Holder<...()>;
        fn run()->bool {{let h:Chosen=Holder<...()> {{value:true}};h.value}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn computed_packets_preserve_symbolic_generic_type_inputs() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}struct Holder where T {{value:T}}
        comptime fn packet(T:Type)->Tuple<Type> {{(T,)}}
        fn forward(value:T)->Holder<...packet(T)> where T {{Holder<...packet(T)> {{value:value}}}}
        fn run()->bool {{forward(true).value}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn runtime_type_factories_cannot_be_run_as_required_packet_producers() {
    let source = format!(
        "{TYPE}struct Holder where T {{value:T}}
        fn packet()->Tuple<Type> {{(bool,)}} typealias Bad=Holder<...packet()>;"
    );
    let error =
        execution_support::compile_with_tuple_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        error.contains("runtime") || error.contains("phase") || error.contains("comptime"),
        "{error}"
    );
}

#[test]
fn residual_packet_exit_skips_following_arguments() {
    let package=execution_support::compile_with_tuple(&format!("{TYPE}struct Pair where A,B {{first:A,second:B}}
        comptime fn packet()->Tuple<Type> {{(bool,)}}
        comptime fn Later()->Type {{.u8'builtin}}
        comptime fn Build(flag:bool)->Type {{Pair<...do {{if flag {{return bool;}}packet()}},Later()>}}
        comptime fn Run()->Type {{Build(true)}}"));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Run", "Build"]);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn staged_type_inputs_survive_packet_expansion_in_signatures() {
    let package=execution_support::compile_with_tuple(&format!("{TYPE}struct Holder where T {{value:T}}
        comptime fn packet(T:Type)->Tuple<Type> {{(T,)}}
        fn forward(comptime T:Type,value:T)->Holder<...packet(T)> {{Holder<...packet(T)> {{value:value}}}}
        fn run()->bool {{forward(bool,true).value}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scoped_types_nested_inside_packet_fields_restore_the_original_telescope() {
    let package=execution_support::compile_with_tuple(&format!("{TYPE}struct Holder where T {{value:T}}
        comptime fn packet(T:Type)->Tuple<Type> {{let Pair:Type=Tuple<T,T>;(Pair,)}}
        fn forward(comptime T:Type,value:Tuple<T,T>)->Holder<...packet(T)> {{Holder<...packet(T)> {{value:value}}}}
        fn run()->bool {{let holder=forward(bool,(true,false));let (first,second)=holder.value;first and not second}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn documented_type_spread_example_executes() {
    let package = execution_support::compile_splice_fixture(
        include_str!("../../../development/examples/type-spreads.fl"),
        &Default::default(),
        None,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn receiver_generic_prefix_is_distinct_from_expanded_method_inputs() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        struct Holder where T {{value:T}}
        impl Holder<T> where T {{
            fn replace(self,value:U)->Holder<U> where U {{Holder<U> {{value:value}}}}
            fn defaulted(self,value:U)->Holder<U> where .U:Type=bool {{Holder<U> {{value:value}}}}
        }}
        comptime fn packet()->Tuple<Type> {{(bool,)}}
        fn run()->bool {{let a=Holder<.u8'builtin> {{value:7}}.replace<...packet()>(true);
            let b=Holder<.u8'builtin> {{value:9}}.defaulted<...()>(false);
            a.value and not b.value}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn trait_method_spreads_preserve_interface_arguments_and_method_constraints() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        trait Transform where T {{fn apply(self,value:U)->U where U;}}
        struct Holder {{}}
        impl Transform<.u8'builtin> for Holder {{fn apply(self,value:U)->U where U {{value}}}}
        comptime fn packet()->Tuple<Type> {{(bool,)}}
        fn run()->bool {{Holder {{}}.as(Transform<.u8'builtin>).apply<...packet()>(true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn mixed_head_packets_keep_dependent_numeric_context() {
    let package = compile(&format!(
        "{TYPE}
        pure comptime fn Tagged<T,value:T>->Type {{T}}
        typealias Chosen=Tagged<...(.u8'builtin,7)>;
        comptime fn Run()->Type {{Tagged<...(.u8'builtin,7)>}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(7))
            ],
        })
    );
}

#[test]
fn mixed_head_packets_preserve_symbolic_types() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Size<T,n:Integer>->Type {{bool}}
        typealias Alias<T> = Size<...(T,7)>;
        comptime fn Run()->Type {{Alias<bool>}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert!(
        matches!(result,Value::Type(nir::Term::NormalForm {arguments,..}) if arguments.len()==2)
    );
}

#[test]
fn mixed_head_expansion_preserves_default_and_producer_order() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Tagged<T,value:T,.flag:bool=true>->Type {{T}}
        comptime fn Pick()->Type {{.u8'builtin}}
        comptime fn flag()->bool {{false}}
        comptime fn Run()->Type {{Tagged<.flag=flag(),...(Pick(),7)>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "flag", "Pick"]);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        result
    );
}

#[test]
fn mixed_head_literal_expansion_keeps_constraints_and_value_type_checks() {
    for application in [
        "Tagged<...(.u8'builtin,256)>",
        "Tagged<...(bool,7)>",
        "Tagged<...(bool,true,false)>",
    ] {
        let source = format!(
            "{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}} typealias Bad={application};"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{application}"
        );
    }
}

#[test]
fn mixed_head_spreads_retain_composite_dependent_literal_context() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn Run()->Type {{Tagged<...(Tuple<.u8'builtin,bool>,(7,true))>}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::NormalForm {
                    head: id(&package, "Tuple"),
                    sort: nir::Sort::Type,
                    arguments: vec![frozen_lists::types(
                        &package,
                        vec![
                            nir::Term::Primitive(nir::Primitive::U8),
                            nir::Term::Primitive(nir::Primitive::Bool)
                        ]
                    )]
                },
                nir::Term::TupleValue(vec![
                    nir::Term::Integer(comptime_runtime::Integer::from_u128(7)),
                    nir::Term::Bool(true)
                ])
            ],
        })
    );
}

#[test]
fn mixed_head_spread_defaults_reuse_the_acquired_value_once() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer,.other:Integer=n>->Type {{bool}}
        comptime fn count()->Integer {{7}}
        comptime fn Run()->Type {{Sized<...(),...(count(),)>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "count"]);
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Sized"),
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Integer(comptime_runtime::Integer::from_u128(7)); 2],
        })
    );
}

#[test]
fn mixed_head_spread_exit_does_not_execute_later_operands() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer,flag:bool>->Type {{bool}}
        comptime fn flag()->bool {{true}}
        comptime fn Run()->Type {{Sized<...(do {{return bool;}},flag())>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Run"]);
}

#[test]
fn computed_mixed_packets_supply_dependent_head_inputs_once() {
    let package = execution_support::compile_splice_fixture(
        include_str!("../../../development/examples/mixed-type-spreads.fl"),
        &Default::default(),
        None,
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "packet"]);
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(7))
            ],
        })
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        result
    );
    assert!(package.to_string().contains("(preparations (before 0 "));
}

#[test]
fn mixed_packet_preparation_interleaves_with_named_inputs_and_defaults() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        comptime fn flag()->bool {{false}}
        comptime fn packet()->Tuple<Type,Byte> {{(Byte,7)}}
        comptime fn default_flag()->bool {{true}}
        pure comptime fn Tagged<T,value:T,.flag:bool=default_flag()>->Type {{T}}
        comptime fn Before()->Type {{Tagged<.flag=flag(),...packet()>}}
        comptime fn After()->Type {{Tagged<...packet(),.flag=flag()>}}
        comptime fn Default()->Type {{Tagged<...packet()>}}"
    ));
    for (entry, order) in [
        ("Before", vec!["Before", "flag", "packet"]),
        ("After", vec!["After", "packet", "flag"]),
        ("Default", vec!["Default", "packet", "default_flag"]),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let result = vm.invoke(id(&package, entry), vec![]).unwrap();
        assert_eq!(calls(&package, &vm), order);
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            result
        );
    }
}

#[test]
fn empty_mixed_packets_execute_before_defaults_and_after_last_explicit_input() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        comptime fn empty(...items:T)->T where T {{items}}
        comptime fn number()->Integer {{7}}
        comptime fn default_flag()->bool {{true}}
        pure comptime fn Sized<n:Integer,.flag:bool=default_flag()>->Type {{bool}}
        comptime fn Default()->Type {{Sized<number(),...empty()>}}
        comptime fn Last()->Type {{Sized<number(),.flag=default_flag(),...empty()>}}"
    ));
    for (entry, order) in [
        (
            "Default",
            vec!["Default", "number", "empty", "default_flag"],
        ),
        ("Last", vec!["Last", "number", "default_flag", "empty"]),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.invoke(id(&package, entry), vec![]).unwrap();
        assert_eq!(calls(&package, &vm), order);
    }
}

#[test]
fn multiple_mixed_packets_keep_prepared_values_when_later_inputs_mutate() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer,other:Integer>->Type {{bool}}
        comptime fn packet(n:Integer)->Tuple<Integer> {{(n,)}}
        comptime fn Run()->Type {{var n:Integer=3;Sized<...packet(n),...do {{n=9;packet(n)}}>}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Sized"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Integer(comptime_runtime::Integer::from_u128(3)),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(9))
            ],
        })
    );
}

#[test]
fn mixed_packet_exit_skips_later_arguments_and_defaults() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer,.flag:bool=default_flag()>->Type {{bool}}
        comptime fn packet()->Tuple<Integer> {{(7,)}}
        comptime fn default_flag()->bool {{true}}
        comptime fn Build(exit:bool)->Type {{Sized<...do {{if exit {{return bool;}}packet()}}>}}
        comptime fn Run()->Type {{Build(true)}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Run", "Build"]);
}

#[test]
fn mixed_packets_do_not_bypass_dependent_value_type_validation() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet()->Tuple<Type,bool> {{(.u8'builtin,true)}}
        comptime fn Run()->Type {{Tagged<...packet()>}}"
    ));
    assert!(luna::invocation::invoke(&package, "Run", 100_000).is_err());
    let error=execution_support::compile_with_tuple_protocols(&format!("{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet()->Tuple<Type,bool> {{(.u8'builtin,true)}} typealias Bad=Tagged<...packet()>;"),&Default::default()).unwrap_err();
    assert!(
        error.contains("comptime") || error.contains("type"),
        "{error}"
    );
}

#[test]
fn runtime_packet_producers_remain_forbidden_in_mixed_head_construction() {
    let error = execution_support::compile_with_tuple_protocols(
        &format!(
            "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        fn packet()->Tuple<Integer> {{(7,)}}
        comptime fn Run()->Type {{Sized<...packet()>}}"
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("runtime") || error.contains("phase") || error.contains("comptime"),
        "{error}"
    );
}

#[test]
fn computed_type_packet_contextualizes_following_literals() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet()->Tuple<Type> {{(.u8'builtin,)}}
        comptime fn Run()->Type {{Tagged<...packet(),7>}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(7))
            ],
        })
    );
}

#[test]
fn mixed_packet_values_can_precede_their_late_named_type() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Tagged<.T:Type=bool,value:T>->Type {{T}}
        comptime fn packet()->Tuple<Byte> {{(7,)}}
        comptime fn Pick()->Type {{Byte}}
        comptime fn Run()->Type {{Tagged<...packet(),.T=Pick()>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "packet", "Pick"]);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        result
    );
    assert_eq!(vm.bytecode_input_preparations(), 1);
}

#[test]
fn mixed_packet_preparations_are_local_to_each_loop_iteration() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        comptime fn packet(n:Integer)->Tuple<Integer> {{(n,)}}
        comptime fn Run()->Type {{var n:Integer=3;var result:Type=bool;
            while n > 0 {{result=Sized<...packet(n)>;n=n - 1;}}result}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Sized"),
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Integer(comptime_runtime::Integer::from_u128(1))],
        })
    );
    assert_eq!(vm.bytecode_input_preparations(), 3);
}

#[test]
fn malformed_packet_plans_reject_and_valid_plans_remain_reusable() {
    use nir::program::{ExprKind as K, Pattern};
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        comptime fn packet()->Tuple<Integer> {{(7,)}}
        comptime fn Run()->Type {{Sized<...packet()>}}"
    ));
    for defect in 0..3 {
        let mut bad = package.clone();
        let function = bad.functions.iter_mut().find(|f| f.name == "Run").unwrap();
        let mut changed = false;
        nir::visit::expression_mut(&mut function.body, &mut 10_000, |expr, _| {
            if let K::NormalFormType {
                plan: Some(plan), ..
            } = &mut expr.kind
            {
                changed = true;
                match defect {
                    0 => plan.preparations[0].before = plan.order.len() + 1,
                    1 => {
                        let K::LetPattern {
                            pattern: Pattern::Tuple(fields),
                            ..
                        } = &mut plan.preparations[0].expression.kind
                        else {
                            unreachable!()
                        };
                        let Pattern::Binding { binding, .. } = &mut fields[0] else {
                            unreachable!()
                        };
                        *binding = plan.bindings[0];
                    }
                    _ => plan.preparations.clear(),
                }
            }
            Ok(())
        })
        .unwrap();
        assert!(changed);
        assert!(
            luna::invocation::invoke(&bad, "Run", 100_000).is_err(),
            "defect {defect}"
        );
    }
    let expected = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(vm.invoke(id(&package, "Run"), vec![]).unwrap(), expected);
    }
}

#[test]
fn mixed_packets_in_type_annotations_do_not_request_runtime_splice() {
    let source = format!(
        "{TYPE}pure comptime fn TupleElements<T>->Type {{T}} pure comptime fn Tuple<...Es:TupleElements<Type>>->Type {{bool}} typealias Integer=.Integer'builtin;
        trait Splice {{assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}}
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        comptime fn packet()->Tuple<Integer> {{(7,)}}
        fn pass(value:Sized<...packet()>)->Sized<...packet()> {{value}}
        fn run()->bool {{true}}"
    );
    let package =
        execution_support::compile_splice_fixture(&source, &Default::default(), Some("Splice"))
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_mixed_producers_specialize_their_preparation_types_per_instance() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet(value:T)->Tuple<Type,T> where T {{(T,value)}}
        comptime fn Build(value:T)->Type where T {{Tagged<...packet(value)>}}
        comptime fn Run()->Tuple<Type,Type> {{let byte:Byte=7;(Build(true),Build(byte))}}"
    ));
    let value = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    let make = |arguments| {
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments,
        })
    };
    assert_eq!(
        value,
        Value::Tuple(vec![
            make(vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Bool(true)
            ]),
            make(vec![
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(7))
            ]),
        ])
    );
}

#[test]
fn computed_mixed_packets_use_the_bound_tuple_view() {
    let source = format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet()->Tuple<Type,Byte> {{(Byte,7)}}
        comptime fn Run()->Type {{Tagged<...packet()>}}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.List"),
        tuple: Some("input.Tuple"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(&source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(7))
            ],
        })
    );
}

#[test]
fn computed_packet_branches_receive_fixed_numeric_parameter_contexts() {
    for packet in [
        "do {(7,8)}",
        "if flag {(7,8)} else {(9,10)}",
        "flag match {true=>(7,8),false=>(9,10)}",
    ] {
        let package = compile(&format!(
            "{TYPE}typealias Byte=.u8'builtin;
            pure comptime fn Pair<first:Byte,second:Byte>->Type {{bool}}
            comptime fn Build(flag:bool)->Type {{Pair<...{packet}>}}
            comptime fn Run()->Type {{Build(true)}}"
        ));
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            Value::Type(nir::Term::NormalForm {
                head: id(&package, "Pair"),
                sort: nir::Sort::Type,
                arguments: vec![
                    nir::Term::Integer(comptime_runtime::Integer::from_u128(7)),
                    nir::Term::Integer(comptime_runtime::Integer::from_u128(8))
                ],
            }),
            "{packet}"
        );
    }
}

#[test]
fn packet_context_skips_optional_slots_and_combines_multiple_spreads() {
    let package = compile(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Pair<.flag:bool=true,first:Byte,second:Byte>->Type {{bool}}
        comptime fn Run()->Type {{Pair<...do {{(7,)}},.flag=false,...do {{(8,)}}>}}"
    ));
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Pair"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Bool(false),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(7)),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(8))
            ],
        })
    );
}

#[test]
fn packet_result_context_can_infer_a_generic_producers_collection() {
    let package = compile(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Pair<first:Byte,second:Byte>->Type {{bool}}
        comptime fn packet(...items:T)->T where T {{items}}
        comptime fn Run()->Type {{Pair<...packet(7,8)>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "packet"]);
}

#[test]
fn packet_context_rejects_overflow_without_reinterpreting_typed_values() {
    for packet in ["do {(256,)}", "typed()"] {
        let source = format!(
            "{TYPE}typealias Byte=.u8'builtin;
            pure comptime fn Sized<n:Byte>->Type {{bool}}
            comptime fn typed()->Tuple<.Integer'builtin> {{(7,)}}
            comptime fn Run()->Type {{Sized<...{packet}>}}"
        );
        let error = execution_support::compile_with_tuple_protocols(&source, &Default::default())
            .unwrap_err();
        assert!(
            error.contains("range")
                || error.contains("fit")
                || error.contains("type mismatch")
                || error.contains("unresolved type equality"),
            "{packet}: {error}"
        );
    }
}

#[test]
fn partial_packet_context_keeps_fixed_fields_beside_dependent_values() {
    for packet in [
        "do {(bool,true,7)}",
        "if flag {(bool,true,7)} else {(bool,false,8)}",
        "flag match {true=>(bool,true,7),false=>(bool,false,8)}",
    ] {
        let package = compile(&format!(
            "{TYPE}typealias Byte=.u8'builtin;
            pure comptime fn Tagged<T,value:T,number:Byte>->Type {{T}}
            comptime fn Build(flag:bool)->Type {{Tagged<...{packet}>}}
            comptime fn Run()->Type {{Build(true)}}"
        ));
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            Value::Type(nir::Term::NormalForm {
                head: id(&package, "Tagged"),
                sort: nir::Sort::Type,
                arguments: vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Bool(true),
                    nir::Term::Integer(comptime_runtime::Integer::from_u128(7))
                ],
            }),
            "{packet}"
        );
    }
}

#[test]
fn partial_packet_context_does_not_prove_the_unknown_dependency() {
    let prefix = format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Tagged<T,value:T,number:Byte>->Type {{T}}"
    );
    let failure = compile_with_protocols(
        &format!("{prefix} comptime fn Bad()->Type {{Tagged<...do {{(Byte,true,7)}}>}}"),
        &Default::default(),
    )
    .err()
    .expect("known conflicting dependency must be rejected");
    assert!(failure.contains("type mismatch"), "{failure}");
    let package = compile(&format!(
        "{prefix} comptime fn Selected()->Type {{Byte}}
        comptime fn Run()->Type {{Tagged<...do {{(Selected(),true,7)}}>}}"
    ));
    assert!(luna::invocation::invoke(&package, "Run", 100_000).is_err());
}

#[test]
fn partial_context_preserves_the_bound_tuple_constructor() {
    let source = format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        pure comptime fn Tagged<T,value:T,number:Byte>->Type {{T}}
        comptime fn Run()->Type {{Tagged<...do {{(bool,true,7)}}>}}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.List"),
        tuple: Some("input.Tuple"),
        ..Default::default()
    };
    let package =
        execution_support::compile_with_source_bindings(&source, &Default::default(), selectors)
            .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Bool(true),
                nir::Term::Integer(comptime_runtime::Integer::from_u128(7))
            ],
        })
    );
}

#[test]
fn packet_context_inference_does_not_replay_branch_conditions() {
    let package = compile(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Pair<first:Byte,second:Byte>->Type {{bool}}
        comptime fn condition()->bool {{true}}
        comptime fn Run()->Type {{Pair<...if condition() {{(7,8)}} else {{(9,10)}}>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "condition"]);
}

#[test]
fn variadic_return_signatures_determine_packet_partition() {
    let prefix = format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Pair<first:Byte,second:Byte>->Type {{bool}}
        comptime fn packet(...items:T)->T where T {{items}}"
    );
    for body in [
        "Pair<...packet(7),...packet(8)>",
        "let a:Byte=7;let b:Byte=8;Pair<...packet(a),...packet(b)>",
    ] {
        let package = compile(&format!("{prefix}comptime fn Run()->Type {{{body}}}"));
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            Value::Type(nir::Term::NormalForm {
                head: id(&package, "Pair"),
                sort: nir::Sort::Type,
                arguments: vec![
                    nir::Term::Integer(comptime_runtime::Integer::from_u128(7)),
                    nir::Term::Integer(comptime_runtime::Integer::from_u128(8))
                ],
            })
        );
    }
}

#[test]
fn required_packet_layout_preserves_context_execution_and_obligations() {
    for requirement in [
        "T == Tuple<Byte,bool>",
        "Tuple<Byte,bool> == T",
        "T == Tuple<Byte,bool> and Tuple<Byte,bool> == T",
    ] {
        let source = format!(
            "{TYPE}typealias Byte=.u8'builtin;
            comptime fn packet()->T where T, requires {requirement} {{(7,true)}}
            comptime fn take(a:Byte,b:bool,c:Byte,d:bool)->bool {{a == 7 and b and c == 7 and d}}
            comptime fn Run()->bool {{take(...packet(),...packet())}}"
        );
        let package = execution_support::compile_with_tuple(&source);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "Run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["Run", "packet", "packet", "take"]
        );

        let invalid = source.replace(
            &format!("requires {requirement}"),
            &format!("requires ({requirement}) and false"),
        );
        assert!(
            execution_support::compile_with_tuple_protocols(&invalid, &Default::default()).is_err()
        );
        let invalid = source.replace("c:Byte", "c:bool");
        assert!(
            execution_support::compile_with_tuple_protocols(&invalid, &Default::default()).is_err()
        );
    }
}

#[test]
fn type_only_spreads_contextualize_result_inferred_factories() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == Tuple<Type,Type> {{(bool,.u8'builtin)}}
        struct Pair where A,B {{first:A,second:B}}
        typealias Alias<A,B> = Pair<A,B>;
        typealias Selected=Alias<...packet()>;
        comptime fn Run()->Type {{Pair<...packet()>}}
        fn run()->bool {{let pair:Selected=Pair<...packet()> {{first:true,second:7}};pair.first}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "packet"]);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        result
    );
}

#[test]
fn explicit_packet_type_hints_do_not_repeat_producers() {
    let source = format!(
        "{TYPE}typealias Byte=.u8'builtin;
        comptime fn first()->Tuple<Byte,bool> {{(7,true)}}
        comptime fn second()->Tuple<Byte,bool> {{(9,false)}}
        comptime fn packet(value:T)->T where T {{value}}
        comptime fn take(a:Byte,b:bool,c:Byte,d:bool)->bool {{a == 7 and b and c == 9 and not d}}
        comptime fn Run()->bool {{take(
            ...packet<Tuple<Byte,bool>>(do {{let pair=first();pair}}),
            ...packet<Tuple<Byte,bool>>(do {{let pair=second();pair}}))}}"
    );
    let package = execution_support::compile_with_tuple(&source);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["Run", "first", "packet", "second", "packet", "take"]
    );
}

#[test]
fn type_spread_context_is_available_to_function_and_method_instances() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == Tuple<Type> {{(bool,)}}
        fn keep(value:T)->T where T {{value}}
        struct Holder where T {{value:T}}
        impl Holder<T> where T {{fn choose(self,value:U)->U where U {{value}}}}
        fn run()->bool {{let callback=keep<...packet()>;
            callback(true) and Holder<.u8'builtin> {{value:7}}.choose<...packet()>(true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_spread_context_skips_defaults_and_known_empty_inputs() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == Tuple<Type> {{(bool,)}}
        struct Holder where .Ignored:Type=.u8'builtin,T {{value:T}}
        typealias Selected=Holder<...(),...packet(),.Ignored=bool>;
        comptime fn Run()->Type {{Holder<...(),...packet(),.Ignored=bool>}}
        fn run()->bool {{let h:Selected=Holder<...packet(),.Ignored=bool> {{value:true}};h.value}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(luna::invocation::invoke(&package, "Run", 100_000).is_ok());
}

#[test]
fn trait_type_spreads_can_infer_packet_types_without_executing_runtime_code() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == Tuple<Type> {{(bool,)}}
        trait Pick where T {{fn pick(self,value:T)->T;}}
        struct Holder {{}}
        impl Pick<...packet()> for Holder {{fn pick(self,value:bool)->bool {{value}}}}
        fn run()->bool {{Holder {{}}.as(Pick<...packet()>).pick(true)}}"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_packet_result_context_preserves_opaque_normal_heads() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == Tuple<Type> {{(bool,)}}
        pure comptime fn Identity<T>->Type {{T}}
        typealias Chosen=Identity<...packet()>;
        comptime fn Run()->Type {{Identity<...packet()>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Identity"),
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Bool)],
        })
    );
    assert_eq!(calls(&package, &vm), vec!["Run", "packet"]);
}

#[test]
fn type_packet_result_context_does_not_override_producer_requirements() {
    let source = format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == Tuple<Type> {{(bool,)}}
        struct Pair where A,B {{first:A,second:B}}
        typealias Bad=Pair<...packet()>;"
    );
    let error =
        execution_support::compile_with_tuple_protocols(&source, &Default::default()).unwrap_err();
    assert!(error.contains("requirement"), "{error}");
}

#[test]
fn projected_type_factories_expand_packets_once_and_reuse_dynamic_defaults() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        struct Holder where T {{pub newtype Wrapped<U,.V:Type=U> = U;}}
        comptime fn packet(T:Type)->Tuple<Type> {{(T,)}}
        comptime fn Build(T:Type)->Type {{Holder<bool>.Wrapped<...packet(T)>}}
        comptime fn Run()->Type {{Build(.u8'builtin)}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(calls(&package, &vm), vec!["Run", "Build", "packet"]);
    assert_eq!(
        result,
        Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Primitive(nir::Primitive::Bool)
            ],
        })
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        result
    );
}

#[test]
fn dynamic_projection_base_and_member_packets_keep_separate_input_order() {
    let package = execution_support::compile_with_tuple(&format!(
        "{TYPE}
        struct Holder where T {{pub newtype Wrapped<U> = U;}}
        comptime fn outer(T:Type)->Tuple<Type> {{(T,)}}
        comptime fn member()->Tuple<Type> {{(bool,)}}
        comptime fn Build(T:Type)->Type {{Holder<...outer(T)>.Wrapped<...member()>}}
        comptime fn Run()->Type {{Build(.u8'builtin)}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(id(&package, "Run"), vec![]).unwrap();
    assert_eq!(
        calls(&package, &vm),
        vec!["Run", "Build", "outer", "member"]
    );
    assert_eq!(
        result,
        Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Primitive(nir::Primitive::U8)
            ],
        })
    );
}

#[test]
fn type_preserving_packet_layout_does_not_execute_or_repeat_calls() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        comptime fn first()->Byte {1}
        comptime fn second()->Byte {2}
        comptime fn identity(value:T)->T where T {value}
        comptime fn combine(left:Byte,right:Byte)->bool {left == 1 and right == 2}
        comptime fn Run()->bool {
            combine(...identity((first(),)),...identity((second(),)))
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["Run", "first", "identity", "second", "identity", "combine"]
    );
}

#[test]
fn collected_packet_width_counts_source_arguments_not_tuple_fields() {
    for (arguments, expected_calls) in [
        (
            "...pack(.items=((255,true),)),...pack(.items=(true,))",
            vec!["Run", "pack", "pack", "combine"],
        ),
        (
            "...pack((255,true)),...pack(true)",
            vec!["Run", "pack", "pack", "combine"],
        ),
        (
            "...pack(),...pack((255,true)),...pack(...(true,)),...pack(...())",
            vec!["Run", "pack", "pack", "pack", "pack", "combine"],
        ),
    ] {
        let package = execution_support::compile_with_tuple(&format!("typealias Byte=.u8'builtin;
            comptime fn pack(...items:T)->T where T {{items}}
            comptime fn combine(pair:Tuple<Byte,bool>,flag:bool)->bool {{let (number,enabled)=pair;number == 255 and enabled and flag}}
            comptime fn Run()->bool {{combine({arguments})}}"));
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "Run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), expected_calls);
    }
}

#[test]
fn complete_packet_layout_hints_do_not_accept_invalid_named_routing() {
    let prefix = "typealias Byte=.u8'builtin;
        comptime fn pack(...items:T)->T where T {items}
        comptime fn combine(left:Byte,right:Byte)->bool {left == 7 and right == 8}";
    for first in [
        "pack(.unknown=(7,))",
        "pack(.items=(7,),.items=(7,))",
        "pack(.items=(7,),7)",
    ] {
        let result = compile_with_protocols(
            &format!(
                "{prefix}comptime fn Run()->bool {{combine(...{first},...pack(.items=(8,)))}}"
            ),
            &Default::default(),
        );
        assert!(result.is_err(), "{first}");
    }
}

#[test]
fn explicit_named_packets_preserve_optional_and_implicit_input_shapes() {
    for parameter in ["implicit value:T", ".value:T=fallback<T>()"] {
        let source = format!("typealias Byte=.u8'builtin;
            comptime fn fallback()->T where T {{fallback<T>()}}
            comptime fn supply({parameter})->T where T {{value}}
            comptime fn combine(number:Byte,flag:bool)->bool {{number == 255 and flag}}
            comptime fn Run()->bool {{combine(...supply(.value=(255,)),...supply(.value=(true,)))}}");
        let package = compile(&source);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "Run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["Run", "supply", "supply", "combine"]
        );
        for invalid in [
            source.replace(".value=(255,)", ".missing=(255,)"),
            source.replace(parameter, "value:T"),
        ] {
            assert!(compile_with_protocols(&invalid, &Default::default()).is_err());
        }
    }
}

#[test]
fn preserved_packet_shape_routes_among_other_function_inputs() {
    for (parameters, first, second) in [
        (
            "flag:bool,value:T",
            "forward(first(),(255,))",
            "forward(second(),(true,))",
        ),
        (
            "flag:bool,implicit value:T",
            "forward(.value=(255,),first())",
            "forward(second(),.value=(true,))",
        ),
        (
            "flag:bool,.value:T=fallback<T>()",
            "forward(first(),.value=(255,))",
            "forward(.value=(true,),second())",
        ),
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            comptime fn first()->bool {{true}}
            comptime fn second()->bool {{false}}
            comptime fn fallback()->T where T {{fallback<T>()}}
            comptime fn forward({parameters})->T where T {{value}}
            comptime fn combine(number:Byte,flag:bool)->bool {{number == 255 and flag}}
            comptime fn Run()->bool {{combine(...{first},...{second})}}"
        ));
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "Run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            calls(&package, &vm),
            vec!["Run", "first", "forward", "second", "forward", "combine"]
        );
    }
}

#[test]
fn preserved_collection_width_excludes_fixed_prefix_and_suffix() {
    for (left, right) in [
        ("packet(first(),255,last())", "packet(first(),true,last())"),
        (
            "packet(.items=(255,),first(),last())",
            "packet(first(),last(),.items=(true,))",
        ),
        (
            "packet(...(first(),255),last())",
            "packet(first(),...(true,last()))",
        ),
    ] {
        for empty_prefix in [false, true] {
            let empty = if empty_prefix {
                "...packet(first(),last()),"
            } else {
                ""
            };
            let package = compile(&format!(
                "typealias Byte=.u8'builtin;
                comptime fn first()->bool {{true}}
                comptime fn last()->bool {{false}}
                comptime fn packet(prefix:bool,...items:T,suffix:bool)->T where T {{items}}
                comptime fn combine(number:Byte,flag:bool)->bool {{number == 255 and flag}}
                comptime fn Run()->bool {{combine({empty}...{left},...{right})}}"
            ));
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(&package, "Run"), vec![]).unwrap(),
                Value::Bool(true)
            );
            let mut expected = vec!["Run"];
            if empty_prefix {
                expected.extend(["first", "last", "packet"]);
            }
            expected.extend([
                "first", "last", "packet", "first", "last", "packet", "combine",
            ]);
            assert_eq!(calls(&package, &vm), expected);
        }
    }
}

#[test]
fn named_type_list_supply_accepts_literal_prefix_packets_in_source_order() {
    for (arguments, calls) in [
        (
            ".Es=tail(), ...(First(), value())",
            vec!["Build", "tail", "First", "value"],
        ),
        (
            "...(First(), value()), .Es=tail()",
            vec!["Build", "First", "value", "tail"],
        ),
        (
            "...(), First(), .Es=tail(), ...(value(),)",
            vec!["Build", "First", "tail", "value"],
        ),
    ] {
        let source = format!(
            "{TYPE}
            pure comptime fn List<T>->Type {{T}}
            pure comptime fn Product<T,value:T,...Es:List<Type>>->Type {{T}}
            comptime fn First()->Type {{bool}}
            comptime fn value()->bool {{true}}
            comptime fn tail()->List<Type> {{[bool]}}
            comptime fn Build()->Type {{Product<{arguments}>}}
            typealias Fixed=Product<...(.u8'builtin,9),.Es=[bool]>;"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
        )
        .unwrap();
        let mut vm = comptime_interpretor::Interpreter::for_comptime(
            &package,
            comptime_interpretor::Budget::default(),
        )
        .unwrap();
        let result = vm
            .invoke(execution_support::id(&package, "Build"), vec![])
            .unwrap();
        let Value::Type(nir::Term::NormalForm { arguments, .. }) = result else {
            panic!("expected retained head")
        };
        assert_eq!(
            arguments,
            vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Bool(true),
                frozen_lists::types(&package, vec![nir::Term::Primitive(nir::Primitive::Bool)])
            ]
        );
        assert_eq!(execution_support::calls(&package, &vm), calls);
    }
}

#[test]
fn named_type_list_prefix_packets_preserve_exits_and_reject_conflicting_supply() {
    let compile = |arguments: &str| {
        execution_support::compile_with_source_bindings(
            &format!(
                "{TYPE} pure comptime fn List<T>->Type {{T}}
                pure comptime fn Product<T,value:T,...Es:List<Type>>->Type {{T}}
                comptime fn First()->Type {{bool}}
                comptime fn value()->bool {{true}}
                comptime fn tail()->List<Type> {{[bool]}}
                comptime fn Build()->Type {{Product<{arguments}>}}"
            ),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
        )
    };
    let package = compile(".Es=do {tail();return bool;}, ...(First(),value())").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Build"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Build", "tail"]);
    for (args, message) in [
        (
            "...(bool,true,bool),.Es=tail()",
            "combine a complete type-list",
        ),
        ("...(bool,),.Es=tail()", "argument count mismatch"),
        (
            "...(bool,true),.Es=tail(),.Es=tail()",
            "duplicate type-list",
        ),
        ("...(bool,true),.other=tail()", "unknown named normal-form"),
    ] {
        let failure = compile(args).unwrap_err();
        assert!(failure.contains(message), "{args}: {failure}");
    }
}

#[test]
fn computed_prefix_packets_share_the_named_type_list_input_plan() {
    for bound_tuple in [false, true] {
        for (arguments, expected_calls) in [
            (".Es=tail(), ...packet()", vec!["Build", "tail", "packet"]),
            ("...packet(), .Es=tail()", vec!["Build", "packet", "tail"]),
            (
                "...empty(), .Es=tail(), ...packet()",
                vec!["Build", "empty", "tail", "packet"],
            ),
        ] {
            let source = format!(
                "{TYPE}
                pure comptime fn List<T>->Type {{T}}
                pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
                pure comptime fn Product<T,value:T,...Es:List<Type>>->Type {{T}}
                comptime fn packet()->Tuple<Type,bool> {{(bool,true)}}
                comptime fn empty(...items:T)->T where T {{items}}
                comptime fn tail()->List<Type> {{[bool]}}
                comptime fn Build()->Type {{Product<{arguments}>}}
                typealias Fixed=Product<...packet(),.Es=[bool]>;"
            );
            let package = execution_support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    meta_list: Some("input.List"),
                    tuple: bound_tuple.then_some("input.Tuple"),
                    ..Default::default()
                },
            );
            if !bound_tuple {
                let error =
                    package.expect_err("an unbound opaque Tuple head cannot supply tuple layout");
                assert!(
                    error.contains("structural type view remains unresolved"),
                    "{error}"
                );
                continue;
            }
            let package = package.unwrap();
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let Value::Type(nir::Term::NormalForm { arguments, .. }) =
                vm.invoke(id(&package, "Build"), vec![]).unwrap()
            else {
                panic!("expected retained head")
            };
            assert_eq!(
                arguments,
                vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Bool(true),
                    frozen_lists::types(&package, vec![nir::Term::Primitive(nir::Primitive::Bool)])
                ]
            );
            assert_eq!(calls(&package, &vm), expected_calls);
        }
    }
}

#[test]
fn computed_prefix_packets_keep_exit_order_and_dependent_checks() {
    let compile = |arguments: &str, packet: &str| {
        execution_support::compile_with_source_bindings(
            &format!(
                "{TYPE} pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
            pure comptime fn Product<T,value:T,...Es:List<Type>>->Type {{T}}
            comptime fn packet()->Tuple<Type,bool> {{{packet}}}
            comptime fn tail()->List<Type> {{[bool]}}
            comptime fn Build()->Type {{Product<{arguments}>}}"
            ),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                tuple: Some("input.Tuple"),
                ..Default::default()
            },
        )
    };
    let package = compile(".Es=do {tail();return bool;},...packet()", "(bool,true)").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Build"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Build", "tail"]);
    let package = compile("...packet(),.Es=tail()", "(.u8'builtin,true)").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(vm.invoke(id(&package, "Build"), vec![]).is_err());
    assert_eq!(calls(&package, &vm), vec!["Build", "packet"]);
    for (arguments, diagnostic) in [
        ("...packet(),bool,.Es=tail()", "argument count mismatch"),
        (
            "...packet(),.Es=tail(),.Es=tail()",
            "duplicate normal-form argument",
        ),
        (
            "...packet(),.unknown=tail()",
            "unknown named normal-form argument",
        ),
    ] {
        let failure = compile(arguments, "(bool,true)").unwrap_err();
        assert!(failure.contains(diagnostic), "{failure}");
    }
}

#[test]
fn named_type_list_packets_compose_with_dependent_defaults() {
    let source = format!(
        "{TYPE}
        pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        pure comptime fn Product<T,value:T,.copy:T=value,...Es:List<Type>>->Type {{T}}
        comptime fn packet()->Tuple<Type,bool> {{(bool,true)}}
        comptime fn tail()->List<Type> {{[bool]}}
        comptime fn replacement()->bool {{false}}
        comptime fn Default()->Type {{Product<...packet(),.Es=tail()>}}
        comptime fn Empty()->Type {{Product<...packet()>}}
        comptime fn Explicit()->Type {{Product<.copy=replacement(),.Es=tail(),...packet()>}}"
    );
    let package = execution_support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    for (entry, copy, elements, expected_calls) in [
        (
            "Default",
            true,
            vec![nir::Term::Primitive(nir::Primitive::Bool)],
            vec!["Default", "packet", "tail"],
        ),
        (
            "Explicit",
            false,
            vec![nir::Term::Primitive(nir::Primitive::Bool)],
            vec!["Explicit", "replacement", "tail", "packet"],
        ),
        ("Empty", true, vec![], vec!["Empty", "packet"]),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Value::Type(nir::Term::NormalForm { arguments, .. }) =
            vm.invoke(id(&package, entry), vec![]).unwrap()
        else {
            panic!("expected retained head")
        };
        assert_eq!(
            arguments,
            vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Bool(true),
                nir::Term::Bool(copy),
                frozen_lists::types(&package, elements)
            ]
        );
        assert_eq!(calls(&package, &vm), expected_calls);
    }
}

#[test]
fn named_type_list_packets_retain_generic_type_alias_parameters() {
    let source = format!(
        "{TYPE}
        pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        pure comptime fn Product<A,B,.C:Type=B,...Es:List<Type>>->Type {{A}}
        comptime fn packet()->Tuple<Type,Type> where T {{(T,bool)}}
        typealias Alias<T> = Product<...packet<T>(),.Es=[T]>;
        fn keep(value:Alias<T>)->Product<T,bool,.C=bool,.Es=[T]> where T {{value}}
        comptime fn Result()->Type {{Alias<.u8'builtin>}}"
    );
    let package = execution_support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let Value::Type(nir::Term::NormalForm { arguments, .. }) =
        luna::invocation::invoke(&package, "Result", 100_000).unwrap()
    else {
        panic!("expected retained head")
    };
    assert_eq!(
        arguments,
        vec![
            nir::Term::Primitive(nir::Primitive::U8),
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::Primitive(nir::Primitive::Bool),
            frozen_lists::types(&package, vec![nir::Term::Primitive(nir::Primitive::U8)])
        ]
    );
}

#[test]
fn defaulted_normal_heads_collect_type_items_in_source_order() {
    for (arguments, expected_calls) in [
        (
            "bool,true,First(),.copy=replacement(),Second()",
            vec!["Build", "First", "replacement", "Second"],
        ),
        (
            "bool,true,.copy=replacement(),First(),Second()",
            vec!["Build", "replacement", "First", "Second"],
        ),
        (
            "bool,true,First(),Second()",
            vec!["Build", "First", "Second"],
        ),
    ] {
        let source = format!(
            "{TYPE} pure comptime fn List<T>->Type {{T}}
            pure comptime fn Product<T,value:T,.copy:T=value,...Es:List<Type>>->Type {{T}}
            comptime fn First()->Type {{bool}}
            comptime fn Second()->Type {{.u8'builtin}}
            comptime fn replacement()->bool {{true}}
            comptime fn Build()->Type {{Product<{arguments}>}}
            typealias Alias=Product<bool,true,bool,.u8'builtin>;"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
        )
        .unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Value::Type(nir::Term::NormalForm { arguments, .. }) =
            vm.invoke(id(&package, "Build"), vec![]).unwrap()
        else {
            panic!("expected retained head")
        };
        assert_eq!(
            arguments.last(),
            Some(&frozen_lists::types(
                &package,
                vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Primitive(nir::Primitive::U8)
                ]
            ))
        );
        assert_eq!(calls(&package, &vm), expected_calls);
    }
}

#[test]
fn defaulted_type_collection_preserves_early_exit_and_rejects_mixed_supply() {
    let compile = |arguments: &str| {
        execution_support::compile_with_source_bindings(
            &format!(
                "{TYPE} pure comptime fn List<T>->Type {{T}}
            pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
            pure comptime fn Product<T,value:T,.copy:T=value,...Es:List<Type>>->Type {{T}}
            comptime fn First()->Type {{bool}}
            comptime fn Second()->Type {{.u8'builtin}}
            comptime fn replacement()->bool {{true}}
            comptime fn Build()->Type {{Product<{arguments}>}}"
            ),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                tuple: Some("input.Tuple"),
                ..Default::default()
            },
        )
    };
    let package =
        compile("bool,true,First(),do {return bool;},.copy=replacement(),Second()").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Build"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Build", "First"]);
    let package = compile("bool,true,First(),.copy=replacement(),Second()").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(matches!(
        vm.invoke(id(&package, "Build"), vec![]).unwrap(),
        Value::Type(nir::Term::NormalForm { .. })
    ));
    assert_eq!(
        calls(&package, &vm),
        vec!["Build", "First", "replacement", "Second"]
    );
    for args in ["bool,true,bool,.Es=[bool]", "bool,true,.Es=[bool],bool"] {
        let failure = compile(args).unwrap_err();
        assert!(
            failure.contains("combine a complete type-list"),
            "{failure}"
        );
    }
    assert!(compile("bool,true,true").is_err());
}

#[test]
fn defaulted_type_collectors_spread_lists_once_in_source_order() {
    for (arguments, expected_calls) in [
        (
            "bool,true,...items(),.copy=replacement(),bool,...empty()",
            vec!["Build", "items", "replacement", "empty"],
        ),
        (
            "...packet(),...items(),.copy=replacement(),bool,...empty()",
            vec!["Build", "packet", "items", "replacement", "empty"],
        ),
    ] {
        let source = format!(
            "{TYPE} pure comptime fn List<T>->Type {{T}}
            pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
            pure comptime fn Product<T,value:T,.copy:T=value,...Es:List<Type>>->Type {{T}}
            comptime fn packet()->Tuple<Type,bool> {{(bool,true)}}
            comptime fn items()->List<Type> {{[bool,.u8'builtin]}}
            comptime fn empty()->List<Type> {{[]}}
            comptime fn replacement()->bool {{false}}
            comptime fn Build()->Type {{Product<{arguments}>}}
            typealias Alias<T> = Product<bool,true,...[T],bool>;
            fn keep(value:Alias<T>)->Product<bool,true,.Es=[T,bool]> where T {{value}}"
        );
        let package = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                tuple: Some("input.Tuple"),
                ..Default::default()
            },
        )
        .unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Value::Type(nir::Term::NormalForm { arguments, .. }) =
            vm.invoke(id(&package, "Build"), vec![]).unwrap()
        else {
            panic!("expected retained head")
        };
        assert_eq!(arguments[2], nir::Term::Bool(false));
        assert_eq!(
            arguments[3],
            frozen_lists::types(
                &package,
                vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Primitive(nir::Primitive::U8),
                    nir::Term::Primitive(nir::Primitive::Bool)
                ]
            )
        );
        assert_eq!(calls(&package, &vm), expected_calls);
    }
}

#[test]
fn defaulted_list_spreads_preserve_exit_and_validate_element_domains() {
    let compile = |arguments: &str| {
        execution_support::compile_with_source_bindings(
            &format!(
                "{TYPE} pure comptime fn List<T>->Type {{T}}
            pure comptime fn Product<T,value:T,.copy:T=value,...Es:List<Type>>->Type {{T}}
            comptime fn items()->List<Type> {{[bool]}}
            comptime fn replacement()->bool {{false}}
            comptime fn Build()->Type {{Product<{arguments}>}}"
            ),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
        )
    };
    let package =
        compile("bool,true,...do {items();return bool;},.copy=replacement(),bool").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Build"), vec![]).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert_eq!(calls(&package, &vm), vec!["Build", "items"]);
    for args in [
        "bool,true,...[true]",
        "bool,true,...(bool,bool)",
        "bool,true,...items(),.Es=[]",
        "bool,true,.Es=[],...items()",
    ] {
        assert!(compile(args).is_err(), "{args}");
    }
}

#[test]
fn required_prefix_heads_collect_after_computed_packets() {
    let source = format!(
        "{TYPE} pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        pure comptime fn Product<T,value:T,...Es:List<Type>>->Type {{T}}
        comptime fn packet()->Tuple<Type,bool> {{(bool,true)}}
        comptime fn items()->List<Type> {{[.u8'builtin]}}
        comptime fn Build()->Type {{Product<...packet(),...items(),bool>}}
        comptime fn Empty()->Type {{Product<...packet()>}}
        typealias Alias=Product<...packet(),...items(),bool>;
        fn keep(value:Alias)->Product<bool,true,.u8'builtin,bool> {{value}}"
    );
    let package = execution_support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    for (entry, elements, expected_calls) in [
        (
            "Build",
            vec![
                nir::Term::Primitive(nir::Primitive::U8),
                nir::Term::Primitive(nir::Primitive::Bool),
            ],
            vec!["Build", "packet", "items"],
        ),
        ("Empty", vec![], vec!["Empty", "packet"]),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Value::Type(nir::Term::NormalForm { arguments, .. }) =
            vm.invoke(id(&package, entry), vec![]).unwrap()
        else {
            panic!("expected retained head")
        };
        assert_eq!(
            arguments,
            vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Bool(true),
                frozen_lists::types(&package, elements)
            ]
        );
        assert_eq!(calls(&package, &vm), expected_calls);
    }
}

#[test]
fn prefix_literal_expansion_does_not_reinterpret_tail_tuples_as_type_lists() {
    let compile = |args: &str| {
        execution_support::compile_with_source_bindings(
            &format!(
                "{TYPE} pure comptime fn List<T>->Type {{T}}
            pure comptime fn Product<T,value:T,...Es:List<Type>>->Type {{T}}
            comptime fn items()->List<Type> {{[bool]}}
            comptime fn Build()->Type {{Product<{args}>}}"
            ),
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
        )
    };
    let package = compile("...(.u8'builtin,9),...items()").unwrap();
    let Value::Type(nir::Term::NormalForm { arguments, .. }) =
        luna::invocation::invoke(&package, "Build", 100_000).unwrap()
    else {
        panic!("expected retained head")
    };
    assert_eq!(arguments[0], nir::Term::Primitive(nir::Primitive::U8));
    assert_eq!(
        arguments[1],
        nir::Term::Integer(comptime_runtime::Integer::from_u128(9))
    );
    for args in ["...(bool,true),...(bool,bool)", "...(bool,true),...()"] {
        assert!(compile(args).is_err(), "{args}");
    }
}

#[test]
fn collected_type_list_contracts_reject_known_violations_during_elaboration() {
    for tail in ["bool,.u8'builtin", "...[bool,.u8'builtin]"] {
        let source = format!(
            "{TYPE} pure comptime fn List<T>->Type {{T}}
            pure comptime fn Probe<...Es:List<Type>>->Type {{bool}}
            pure comptime fn Checked<T,.U:Type=T,...Es:List<Type>>->Type
                where requires Probe<...Es> == Probe<bool,bool> {{T}}
            comptime fn Build()->Type {{Checked<bool,{tail}>}}"
        );
        let failure = execution_support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(failure.contains("requirement"), "{failure}");
    }
}

#[test]
fn collected_type_list_contracts_check_deferred_inputs_and_recover() {
    let source = format!(
        "{TYPE} pure comptime fn List<T>->Type {{T}}
        pure comptime fn Probe<...Es:List<Type>>->Type {{bool}}
        pure comptime fn Checked<T,.U:Type=T,...Es:List<Type>>->Type
            where requires Probe<...Es> == Probe<bool,bool> {{T}}
        comptime fn items(flag:bool)->List<Type> {{[bool,if flag {{bool}} else {{.u8'builtin}}]}}
        comptime fn Build(flag:bool)->Type {{Checked<bool,...items(flag)>}}"
    );
    let package = execution_support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            ..Default::default()
        },
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(
        vm.invoke(id(&package, "Build"), vec![Value::Bool(false)])
            .is_err()
    );
    let Value::Type(nir::Term::NormalForm {
        head, arguments, ..
    }) = vm
        .invoke(id(&package, "Build"), vec![Value::Bool(true)])
        .unwrap()
    else {
        panic!("expected retained head")
    };
    assert_eq!(head, id(&package, "Checked"));
    assert_eq!(
        arguments,
        vec![
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::Primitive(nir::Primitive::Bool),
            frozen_lists::types(
                &package,
                vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Primitive(nir::Primitive::Bool)
                ]
            )
        ]
    );
    assert_eq!(
        calls(&package, &vm),
        vec!["Build", "items", "Build", "items"]
    );
}

#[test]
fn fixed_packet_type_evidence_reaches_defaults_and_requirements() {
    let prefix = format!(
        "{TYPE} typealias Char='x''type; pure comptime fn Checked<T,value:T,.U:Type=T>->Type where requires U == bool {{T}}"
    );
    for packet in ["do {(Char,'x')}", "if true {(Char,'x')} else {(Char,'y')}"] {
        let result = compile_with_protocols(
            &format!("{prefix} comptime fn Bad()->Type {{Checked<...{packet}>}}"),
            &Default::default(),
        );
        let failure = result
            .err()
            .expect("known default Type violates requirement");
        assert!(failure.contains("requirement"), "{failure}");
    }
    let package = compile(&format!(
        "{prefix}
        comptime fn Good()->Type {{Checked<...do {{(bool,true)}}>}}
        comptime fn Override()->Type {{Checked<...do {{(Char,'x')}},.U=bool>}}"
    ));
    for (entry, first, value) in [
        ("Good", nir::Primitive::Bool, nir::Term::Bool(true)),
        ("Override", nir::Primitive::Char, nir::Term::Char('x')),
    ] {
        let Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, entry, 100_000).unwrap()
        else {
            panic!("expected retained head")
        };
        assert_eq!(
            arguments,
            vec![
                nir::Term::Primitive(first),
                value,
                nir::Term::Primitive(nir::Primitive::Bool)
            ]
        );
    }
}

#[test]
fn trailing_empty_normal_input_packet_preserves_bottom_type() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        comptime fn empty(...items:T)->T where T {{items}}
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        comptime fn Run()->bool {{Sized<7,...do {{return true;empty()}}>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), vec!["Run"]);
    assert_eq!(vm.bytecode_input_preparations(), 1);
}

#[test]
fn returning_normal_input_packets_skip_defaults_and_accept_unambiguous_empty_shape() {
    for packet in ["do {return true;empty()}", "do {return true;}"] {
        let package = compile(&format!(
            "{TYPE}typealias Integer=.Integer'builtin;
            comptime fn empty(...items:T)->T where T {{items}}
            comptime fn later()->bool {{false}}
            pure comptime fn Sized<n:Integer,.flag:bool=later()>->Type {{bool}}
            comptime fn Run()->bool {{Sized<7,...{packet}>}}"
        ));
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "Run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), vec!["Run"]);
    }
}

#[test]
fn returning_normal_input_packets_check_unreachable_tail_shapes() {
    for tail in ["false", "(7,8)"] {
        let source = format!(
            "{TYPE}typealias Integer=.Integer'builtin;
            pure comptime fn Sized<n:Integer>->Type {{bool}}
            comptime fn Run()->bool {{Sized<...do {{return true;{tail}}}>}}"
        );
        assert!(
            compile_with_protocols(&source, &Default::default()).is_err(),
            "{tail}"
        );
    }
}

#[test]
fn returning_nonempty_normal_input_packet_preserves_result_and_order() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        comptime fn first()->bool {{true}}
        comptime fn later()->bool {{false}}
        pure comptime fn Sized<n:Integer,.flag:bool=false>->Type {{bool}}
        comptime fn Run()->bool {{Sized<...do {{let value=first();return value;(7,)}},.flag=later()>}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "Run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), vec!["Run", "first"]);
}

#[test]
fn returning_type_list_inputs_keep_preparation_nodes_diverging() {
    for bound_tuple in [false, true] {
        for input in ["do {return true;bool}", "...do {return true;[bool]}"] {
            let source = format!(
                "{TYPE}pure comptime fn List<T>->Type {{T}}
                pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
                comptime fn Default()->bool {{true}}
                pure comptime fn Product<.flag:bool=Default(),...Es:List<Type>>->Type {{bool}}
                comptime fn Later()->Type {{bool}}
                comptime fn Run()->bool {{Product<{input},Later()>}}"
            );
            let package = execution_support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    meta_list: Some("input.List"),
                    tuple: bound_tuple.then_some("input.Tuple"),
                    ..Default::default()
                },
            )
            .unwrap();
            let run = id(&package, "Run");
            let body = &package
                .functions
                .iter()
                .find(|f| f.declaration == run)
                .unwrap()
                .body;
            let mut found = false;
            nir::visit::expression(body, &mut 10_000, |expression, _| {
                if let nir::program::ExprKind::Tuple(fields) = &expression.kind {
                    if fields
                        .iter()
                        .any(|field| field.ty == nir::Term::Primitive(nir::Primitive::Never))
                    {
                        found = true;
                        assert_eq!(expression.ty, nir::Term::Primitive(nir::Primitive::Never));
                    }
                }
                if let nir::program::ExprKind::LetPattern { value, .. } = &expression.kind {
                    if value.ty == nir::Term::Primitive(nir::Primitive::Never) {
                        assert_eq!(expression.ty, nir::Term::Primitive(nir::Primitive::Never));
                    }
                }
                Ok(())
            })
            .unwrap();
            assert!(found);
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert_eq!(vm.invoke(run, vec![]).unwrap(), Value::Bool(true));
            assert_eq!(calls(&package, &vm), vec!["Run"]);
        }
    }
}

#[test]
fn normal_form_type_list_requirements_keep_the_collected_input_kind() {
    let prefix = format!("{TYPE} pure comptime fn List<T>->Type {{T}}");
    let baseline = compile(&prefix);
    let protocols = elaboration::program::Protocols {
        meta_list: Some(baseline.normal_forms[0].declaration),
        ..Default::default()
    };
    for suffix in ["", ",bool,.u8'builtin"] {
        let source = format!(
            "{prefix}
            pure comptime fn Indexed<T,...Es:List<Type>>->Type where requires T == bool {{T}}
            fn accept(value:*Indexed<bool{suffix}>)->bool {{true}}
            fn run()->fn(*Indexed<bool{suffix}>)->bool {{accept}}"
        );
        let package = compile_with_protocols(&source, &protocols).unwrap();
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let accept = graph
            .functions
            .iter()
            .find(|item| item.function.name == "accept")
            .unwrap();
        let nir::Term::Function { parameters, .. } = &accept.function.signature else {
            panic!("expected callable")
        };
        let nir::Term::Pointer(inner) = &parameters[0].ty else {
            panic!("expected pointer")
        };
        let nir::Term::NormalForm { arguments, .. } = inner.as_ref() else {
            panic!("head was erased")
        };
        let nir::Term::NominalValue { fields, .. } = &arguments[1] else {
            panic!("collected input lost its ordinary value carrier")
        };
        assert_eq!(fields.len(), if suffix.is_empty() { 0 } else { 2 });
        assert_eq!(arguments[1], frozen_lists::types(&package, fields.clone()));
        let invalid = source.replace("Indexed<bool", "Indexed<'x''type");
        assert!(compile_with_protocols(&invalid, &protocols).is_err());
    }
}

#[test]
fn generic_invocation_transports_residual_nominal_type_lists() {
    let source = format!(
        "{TYPE}
        pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        newtype Indexed<value:bool> = bool;
        newtype Tagged<value:.char'builtin> = bool;
        comptime fn TaggedType()->Type {{Tagged<'x'>}}
        comptime fn pack(value:bool)->List<Type> {{[Indexed<value>,Indexed<value>]}}
        comptime fn pass(values:List<Type>)->List<Type> {{values}}
        comptime fn pass_packet(values:Tuple<List<Type>>)->Tuple<List<Type>> {{values}}"
    );
    let package = execution_support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let parameter = nir::DeclarationId {
        package: 99,
        local: 1,
    };
    let domain = nir::Term::Primitive(nir::Primitive::Bool);
    let authority = [(parameter, domain.clone())];
    let opaque = Value::AbstractValue {
        declaration: parameter,
        ty: domain,
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_abstract_value_parameters(&authority)
        .unwrap();
    let value = vm.invoke(id(&package, "pack"), vec![opaque]).unwrap();
    let Value::List {
        values: elements, ..
    } = &value
    else {
        panic!("type list")
    };
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[0], elements[1]);
    assert!(
        matches!(&elements[0],Value::Type(nir::Term::Nominal {arguments,..}) if arguments==&vec![nir::Term::Rigid {declaration:parameter,sort:nir::Sort::ComptimeValue}])
    );
    assert_eq!(
        vm.invoke(id(&package, "pass"), vec![value.clone()])
            .unwrap(),
        value
    );
    assert!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(id(&package, "pass"), vec![value.clone()])
            .is_err()
    );
    let tagged = package
        .newtypes
        .iter()
        .find(|item| item.name == "Tagged")
        .unwrap()
        .declaration;
    let forged = execution_support::type_list_value(
        &package,
        vec![nir::Term::Nominal {
            declaration: tagged,
            arguments: vec![nir::Term::Rigid {
                declaration: parameter,
                sort: nir::Sort::ComptimeValue,
            }],
        }],
    );
    assert!(
        vm.invoke(
            id(&package, "pass_packet"),
            vec![Value::Tuple(vec![forged.clone()])]
        )
        .is_err()
    );
    assert!(vm.invoke(id(&package, "pass"), vec![forged]).is_err());
    assert_eq!(
        vm.invoke(
            id(&package, "pass_packet"),
            vec![Value::Tuple(vec![value.clone()])]
        )
        .unwrap(),
        Value::Tuple(vec![value.clone()])
    );
    assert_eq!(
        vm.invoke(id(&package, "pass"), vec![value.clone()])
            .unwrap(),
        value
    );
    assert!(
        value
            .clone()
            .into_normal_term(&mut Default::default())
            .is_err()
    );
    assert!(
        value
            .into_abstract_invocation_term(
                &mut Default::default(),
                &Default::default(),
                &authority.into_iter().collect()
            )
            .is_ok()
    );
    let concrete = vm
        .invoke(id(&package, "pack"), vec![Value::Bool(true)])
        .unwrap();
    assert!(concrete.into_normal_term(&mut Default::default()).is_ok());
}

#[test]
fn residual_normal_form_value_arguments_keep_heads_and_domain_evidence() {
    let package = compile(&format!(
        "{TYPE}
        pure comptime fn Indexed<T,value:T>->Type {{T}}
        comptime fn Build(value:bool)->Type {{Indexed<bool,value>}}
        comptime fn pass(value:Type)->Type {{value}}"
    ));
    let parameter = nir::DeclarationId {
        package: 99,
        local: 1,
    };
    let domain = nir::Term::Primitive(nir::Primitive::Bool);
    let authority = [(parameter, domain.clone())];
    let opaque = Value::AbstractValue {
        declaration: parameter,
        ty: domain.clone(),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_abstract_value_parameters(&authority)
        .unwrap();
    let value = vm.invoke(id(&package, "Build"), vec![opaque]).unwrap();
    let head = id(&package, "Indexed");
    let symbolic = nir::Term::Rigid {
        declaration: parameter,
        sort: nir::Sort::ComptimeValue,
    };
    assert_eq!(
        value,
        Value::Type(nir::Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            arguments: vec![domain, symbolic.clone()]
        })
    );
    assert_eq!(
        vm.invoke(id(&package, "pass"), vec![value.clone()])
            .unwrap(),
        value
    );
    assert!(
        value
            .clone()
            .into_normal_term(&mut Default::default())
            .is_err()
    );
    let forged = Value::Type(nir::Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![nir::Term::Primitive(nir::Primitive::Char), symbolic],
    });
    assert!(vm.invoke(id(&package, "pass"), vec![forged]).is_err());
    assert_eq!(
        vm.invoke(id(&package, "pass"), vec![value.clone()])
            .unwrap(),
        value
    );
    assert!(!calls(&package, &vm).iter().any(|name| name == "Indexed"));
    let concrete = vm
        .invoke(id(&package, "Build"), vec![Value::Bool(true)])
        .unwrap();
    assert!(concrete.into_normal_term(&mut Default::default()).is_ok());
}
