//! Clause expansion preserves opaque heads, routing and comptime producers.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{calls, compile, compile_with_protocols, id};

const TYPE: &str = "typealias Type=.Type'builtin;";

#[test]
fn nominal_and_alias_arguments_expand_literal_and_computed_packets() {
    let package=compile(&format!("{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        enum Choice where A,B {{first(A),second(B)}}
        typealias Alias<A,B> = Pair<A,B>;
        comptime fn packet()->(Type,Type) {{(bool,.u8'builtin)}}
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
    let package = compile(&format!(
        "{TYPE}
        comptime fn packet()->(Type,) {{(bool,)}}
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
    let package = compile(&format!(
        "{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        comptime fn packet(T:Type)->(Type,) {{(T,)}}
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
    let package = compile(&format!(
        "{TYPE}
        pure comptime fn Identity<T>->Type {{T}}
        comptime fn packet(T:Type)->(Type,) {{(T,)}}
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
    let package = compile(&format!(
        "{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        typealias Alias<A,.B:Type=A> = Pair<A,B>;
        comptime fn packet(T:Type)->(Type,) {{(T,)}}
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
    let package = compile(&format!(
        "{TYPE}
        struct Pair where A,B {{first:A,second:B}}
        comptime fn packet(T:Type)->(Type,) {{(T,)}}
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
        comptime fn packet(T:Type)->(Type,Type) {{(T,bool)}}
        comptime fn Build(T:Type)->Type {{Pair<...packet(T)>}}
        comptime fn Run()->Type {{Build(.u8'builtin)}}
        typealias Concrete=Pair<...packet(bool)>;"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
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
    let package = compile(&format!(
        "{TYPE}struct Holder where T {{value:T}}
        comptime fn packet(T:Type)->(Type,) {{(T,)}}
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
        fn packet()->(Type,) {{(bool,)}} typealias Bad=Holder<...packet()>;"
    );
    let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        error.contains("runtime") || error.contains("phase") || error.contains("comptime"),
        "{error}"
    );
}

#[test]
fn residual_packet_exit_skips_following_arguments() {
    let package=compile(&format!("{TYPE}struct Pair where A,B {{first:A,second:B}}
        comptime fn packet()->(Type,) {{(bool,)}}
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
    let package=compile(&format!("{TYPE}struct Holder where T {{value:T}}
        comptime fn packet(T:Type)->(Type,) {{(T,)}}
        fn forward(comptime T:Type,value:T)->Holder<...packet(T)> {{Holder<...packet(T)> {{value:value}}}}
        fn run()->bool {{forward(bool,true).value}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn scoped_types_nested_inside_packet_fields_restore_the_original_telescope() {
    let package=compile(&format!("{TYPE}struct Holder where T {{value:T}}
        comptime fn packet(T:Type)->(Type,) {{let Pair:Type=(T,T);(Pair,)}}
        fn forward(comptime T:Type,value:(T,T))->Holder<...packet(T)> {{Holder<...packet(T)> {{value:value}}}}
        fn run()->bool {{let holder=forward(bool,(true,false));let (first,second)=holder.value;first and not second}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn documented_type_spread_example_executes() {
    let package = compile(include_str!(
        "../../../development/examples/type-spreads.fl"
    ));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn receiver_generic_prefix_is_distinct_from_expanded_method_inputs() {
    let package = compile(&format!(
        "{TYPE}
        struct Holder where T {{value:T}}
        impl Holder<T> where T {{
            fn replace(self,value:U)->Holder<U> where U {{Holder<U> {{value:value}}}}
            fn defaulted(self,value:U)->Holder<U> where .U:Type=bool {{Holder<U> {{value:value}}}}
        }}
        comptime fn packet()->(Type,) {{(bool,)}}
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
    let package = compile(&format!(
        "{TYPE}
        trait Transform where T {{fn apply(self,value:U)->U where U;}}
        struct Holder {{}}
        impl Transform<.u8'builtin> for Holder {{fn apply(self,value:U)->U where U {{value}}}}
        comptime fn packet()->(Type,) {{(bool,)}}
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
    let package = compile(&format!(
        "{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn Run()->Type {{Tagged<...((.u8'builtin,bool),(7,true))>}}"
    ));
    let result = luna::invocation::invoke(&package, "Run", 100_000).unwrap();
    assert_eq!(
        result,
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Tagged"),
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Tuple(vec![
                    nir::Term::Primitive(nir::Primitive::U8),
                    nir::Term::Primitive(nir::Primitive::Bool)
                ]),
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
    let package = compile(include_str!(
        "../../../development/examples/mixed-type-spreads.fl"
    ));
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
    let package = compile(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        comptime fn flag()->bool {{false}}
        comptime fn packet()->(Type,Byte) {{(Byte,7)}}
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
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer,other:Integer>->Type {{bool}}
        comptime fn packet(n:Integer)->(Integer,) {{(n,)}}
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
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer,.flag:bool=default_flag()>->Type {{bool}}
        comptime fn packet()->(Integer,) {{(7,)}}
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
    let package = compile(&format!(
        "{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet()->(Type,bool) {{(.u8'builtin,true)}}
        comptime fn Run()->Type {{Tagged<...packet()>}}"
    ));
    assert!(luna::invocation::invoke(&package, "Run", 100_000).is_err());
    let error=compile_with_protocols(&format!("{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet()->(Type,bool) {{(.u8'builtin,true)}} typealias Bad=Tagged<...packet()>;"),&Default::default()).unwrap_err();
    assert!(
        error.contains("comptime") || error.contains("type"),
        "{error}"
    );
}

#[test]
fn runtime_packet_producers_remain_forbidden_in_mixed_head_construction() {
    let error = compile_with_protocols(
        &format!(
            "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        fn packet()->(Integer,) {{(7,)}}
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
    let package = compile(&format!(
        "{TYPE}pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet()->(Type,) {{(.u8'builtin,)}}
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
    let package = compile(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Tagged<.T:Type=bool,value:T>->Type {{T}}
        comptime fn packet()->(Byte,) {{(7,)}}
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
}

#[test]
fn mixed_packet_preparations_are_local_to_each_loop_iteration() {
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        comptime fn packet(n:Integer)->(Integer,) {{(n,)}}
        comptime fn Run()->Type {{var n:Integer=3;var result:Type=bool;
            while n > 0 {{result=Sized<...packet(n)>;n=n - 1;}}result}}"
    ));
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Type(nir::Term::NormalForm {
            head: id(&package, "Sized"),
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::Integer(comptime_runtime::Integer::from_u128(1))],
        })
    );
}

#[test]
fn malformed_packet_plans_reject_and_valid_plans_remain_reusable() {
    use nir::program::{ExprKind as K, Pattern};
    let package = compile(&format!(
        "{TYPE}typealias Integer=.Integer'builtin;
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        comptime fn packet()->(Integer,) {{(7,)}}
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
        "{TYPE}typealias Integer=.Integer'builtin;
        trait Splice {{assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}}
        pure comptime fn Sized<n:Integer>->Type {{bool}}
        comptime fn packet()->(Integer,) {{(7,)}}
        fn pass(value:Sized<...packet()>)->Sized<...packet()> {{value}}
        fn run()->bool {{true}}"
    );
    let package =
        execution_support::compile_with_bindings(&source, &Default::default(), Some("Splice"))
            .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_mixed_producers_specialize_their_preparation_types_per_instance() {
    let package = compile(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Tagged<T,value:T>->Type {{T}}
        comptime fn packet(value:T)->(Type,T) where T {{(T,value)}}
        comptime fn Build(value:T)->Type where T {{Tagged<...packet(value)>}}
        comptime fn Run()->(Type,Type) {{let byte:Byte=7;(Build(true),Build(byte))}}"
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
        comptime fn packet()->(Type,Byte) {{(Byte,7)}}
        comptime fn Run()->Type {{Tagged<...packet()>}}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
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
            comptime fn typed()->(.Integer'builtin,) {{(7,)}}
            comptime fn Run()->Type {{Sized<...{packet}>}}"
        );
        let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("range") || error.contains("fit") || error.contains("type mismatch"),
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
    let package = compile(&format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Tagged<T,value:T,number:Byte>->Type {{T}}
        comptime fn Run()->Type {{Tagged<...do {{(Byte,true,7)}}>}}"
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
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
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
fn multiple_unknown_packet_widths_do_not_invent_a_partition() {
    let prefix = format!(
        "{TYPE}typealias Byte=.u8'builtin;
        pure comptime fn Pair<first:Byte,second:Byte>->Type {{bool}}
        comptime fn packet(...items:T)->T where T {{items}}"
    );
    assert!(
        compile_with_protocols(
            &format!("{prefix}comptime fn Run()->Type {{Pair<...packet(7),...packet(8)>}}"),
            &Default::default()
        )
        .is_err()
    );
    let package = compile(&format!(
        "{prefix}comptime fn Run()->Type {{let a:Byte=7;let b:Byte=8;Pair<...packet(a),...packet(b)>}}"
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
        })
    );
}

#[test]
fn type_only_spreads_contextualize_result_inferred_factories() {
    let package = compile(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == (Type,Type) {{(bool,.u8'builtin)}}
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
fn type_spread_context_is_available_to_function_and_method_instances() {
    let package = compile(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == (Type,) {{(bool,)}}
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
    let package = compile(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == (Type,) {{(bool,)}}
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
    let package = compile(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == (Type,) {{(bool,)}}
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
    let package = compile(&format!(
        "{TYPE}
        comptime fn packet()->T where T, requires T == (Type,) {{(bool,)}}
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
        comptime fn packet()->T where T, requires T == (Type,) {{(bool,)}}
        struct Pair where A,B {{first:A,second:B}}
        typealias Bad=Pair<...packet()>;"
    );
    let error = compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(error.contains("requirement"), "{error}");
}
