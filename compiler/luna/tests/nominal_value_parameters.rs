//! Value-indexed nominal identities and concrete layouts.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn struct_value_parameters_keep_distinct_nominal_identities() {
    let source = "struct Flag where tag:bool {value:bool}
        fn read(value:Flag<true>)->bool {value.value}
        fn run()->bool {read(Flag<true>{value:true})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("read(Flag<true>", "read(Flag<false>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn enum_value_parameters_keep_concrete_payload_layouts() {
    let source = "enum Choice where tag:bool {none,some(bool)}
        fn read(value:Choice<true>)->bool {value match {.none=>false,.some(flag)=>flag}}
        fn run()->bool {read(Choice<true>.some(true))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_value_parameters_mix_type_and_dependent_value_domains() {
    let source = "struct Tagged where T, tag:T {value:bool}
        struct Pair where T, enabled:bool {value:T}
        fn run()->bool {
            let tagged=Tagged<bool,true>{value:true};
            let pair=Pair<bool,false>{value:true};tagged.value and pair.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Tagged<bool,true>", "Tagged<bool,'x'>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nominal_value_parameters_use_declaration_and_field_defaults() {
    let source = "struct Flag where .tag:bool=true {value:bool=true}
        fn read(value:Flag<.tag=true>)->bool {value.value}
        fn run()->bool {read(Flag{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_value_parameters_reject_runtime_indices_and_program_constraints() {
    for source in [
        "struct Flag where tag:bool {} fn bad(tag:bool) {let value=Flag<tag>{};}",
        "struct Flag where tag:bool, requires tag {} fn run() {let value=Flag<false>{};}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn concrete_impls_select_value_indexed_nominal_subjects() {
    let source = "struct Flag where tag:bool {}
        impl Flag<true> {comptime fn build()-><|-bool> {'(true)}}
        impl Flag<false> {comptime fn build()-><|-bool> {'(false)}}
        fn run()->bool {~<Flag<true>.build()> and not ~<Flag<false>.build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_impl_generator_captures_nominal_value_parameters() {
    let source = "struct Flag where tag:bool {}
        impl Flag<tag> where tag:bool {
            comptime fn build()-><|-bool> { if tag {'(true)} else {'(false)} }
        }
        fn run()->bool {~<Flag<true>.build()> and not ~<Flag<false>.build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_impl_generator_captures_dependent_type_and_value_domains() {
    let source = "struct Tagged where T, tag:T {}
        impl Tagged<T,tag> where T, tag:T {
            comptime fn build(value:T)-><|-T> { '(value) }
        }
        fn run()->bool {~<Tagged<bool,true>.build(true)> and not ~<Tagged<bool,false>.build(false)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_trait_impl_generator_captures_nominal_value_parameters() {
    let source = "trait Build {comptime fn build()-><|-bool>;}
        struct Flag where tag:bool {}
        impl Build for Flag<tag> where tag:bool {
            comptime fn build()-><|-bool> {if tag {'(true)} else {'(false)}}
        }
        fn run()->bool {~<Flag<true>.as(Build).build()> and not ~<Flag<false>.as(Build).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_impl_ordinary_methods_retain_value_indexed_self() {
    let source = "struct Flag where tag:bool {value:bool}
        impl Flag<tag> where tag:bool {fn read(self)->bool {self.value}}
        fn run()->bool {Flag<true>{value:true}.read() and not Flag<false>{value:false}.read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_trait_impl_ordinary_methods_dispatch_with_value_arguments() {
    let source = "trait Read {fn read(self)->bool;}
        struct Flag where tag:bool {value:bool}
        impl Read for Flag<tag> where tag:bool {fn read(self)->bool {self.value}}
        fn run()->bool {Flag<true>{value:true}.as(Read).read() and not Flag<false>{value:false}.as(Read).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_trait_impl_captures_values_inferred_from_interface_arguments() {
    let source = "trait Build where flag:bool {comptime fn build()-><|-bool>;}
        struct Host {}
        impl Build<flag> for Host where flag:bool {
            comptime fn build()-><|-bool> {if flag {'(true)} else {'(false)}}
        }
        fn run()->bool {~<Host.as(Build<true>).build()> and not ~<Host.as(Build<false>).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn interface_value_parameters_infer_dependent_domains() {
    let source = "trait Build where T,value:T {comptime fn build()-><|-T>;}
        struct Host {}
        impl Build<T,value> for Host where T,value:T {comptime fn build()-><|-T> {'(value)}}
        fn run()->bool {~<Host.as(Build<bool,true>).build()> and not ~<Host.as(Build<bool,false>).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_interface_indices_do_not_prevent_subject_coherence() {
    let source = "trait Build where flag:bool {comptime fn build()-><|-bool>;}
        struct A {} struct B {}
        impl Build<flag> for A where flag:bool {comptime fn build()-><|-bool> {'(flag)}}
        impl Build<flag> for B where flag:bool {comptime fn build()-><|-bool> {'(not flag)}}
        fn run()->bool {~<A.as(Build<true>).build()> and ~<B.as(Build<false>).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let overlapping = source.replace("for B where", "for A where");
    assert!(support::compile_with_protocols(&overlapping, &Default::default()).is_err());
}

#[test]
fn repeated_interface_value_parameters_require_consistent_inputs() {
    let source = "trait Build where a:bool,b:bool {comptime fn build()-><|-bool>;}
        struct Host {}
        impl Build<flag,flag> for Host where flag:bool {comptime fn build()-><|-bool> {'(flag)}}
        fn run()->bool {~<Host.as(Build<true,true>).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Build<true,true>", "Build<true,false>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generic_trait_impl_captures_whole_record_indices() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true})'type;
        trait Build where mode:Settings {comptime fn build()-><|-bool>;}
        struct Host {}
        impl Build<mode> for Host where mode:Settings {comptime fn build()-><|-bool> {'(mode.enabled)}}
        fn run()->bool {~<Host.as(Build<(settings {enabled:true})>).build()> and not ~<Host.as(Build<(settings {enabled:false})>).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_record_indices_use_known_tags_for_impl_coherence() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {tag:true,enabled:true})'type;
        trait Build where mode:Settings {comptime fn build()-><|-bool>;}
        struct Host {}
        impl Build<(settings {tag:true,enabled:flag})> for Host where flag:bool {
            comptime fn build()-><|-bool> {'(flag)}
        }
        impl Build<(settings {tag:false,enabled:flag})> for Host where flag:bool {
            comptime fn build()-><|-bool> {'(not flag)}
        }
        fn run()->bool {~<Host.as(Build<(settings {tag:true,enabled:true})>).build()> and ~<Host.as(Build<(settings {tag:false,enabled:false})>).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_calls_return_and_materialize_value_indexed_records() {
    let source = "struct Flag where tag:bool {value:bool}
        comptime fn make()->Flag<true> {Flag<true>{value:true}}
        const saved=make();
        const result=saved.value;
        comptime fn run()->bool {result}";
    let package = support::compile(source);
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let mut interpreter =
        comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        interpreter.invoke(entry, vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_calls_return_and_materialize_value_indexed_variants() {
    let source = "enum Choice where tag:bool {none,some(bool)}
        comptime fn make()->Choice<true> {Choice<true>.some(true)}
        const saved=make();
        const result=saved match {.none=>false,.some(value)=>value};
        comptime fn run()->bool {result}";
    let package = support::compile(source);
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let mut interpreter =
        comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        interpreter.invoke(entry, vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nir_interpreter_replays_generic_value_indexed_trait_dispatch() {
    let source = "trait Read {comptime fn read(self)->bool;}
        struct Flag where tag:bool {value:bool}
        impl Read for Flag<tag> where tag:bool {comptime fn read(self)->bool {self.value}}
        comptime fn read(value:T)->bool where T:-Read {value.read()}
        comptime fn run()->bool {read(Flag<true>{value:true}) and not read(Flag<false>{value:false})}";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn code_generators_read_comptime_value_indexed_record_inputs() {
    let source = "struct Flag where tag:bool {value:bool}
        comptime fn build(value:Flag<true>)-><|-bool> {'(value.value)}
        fn run()->bool {~<build(Flag<true>{value:true})>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn abstract_record_invocation_preserves_nested_repeated_input_identities() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {tag:true,nested:(settings {left:true,right:true})})'type;
        trait Build where mode:Settings {comptime fn build()-><|-bool>;}
        struct Host {}
        impl Build<(settings {tag:true,nested:(settings {left:flag,right:flag})})> for Host where flag:bool {
            comptime fn build()-><|-bool> {'(flag)}
        }
        fn run()->bool {~<Host.as(Build<(settings {tag:true,nested:(settings {left:true,right:true})})>).build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "left:true,right:true})})>).build()",
        "left:true,right:false})})>).build()",
    );
    assert_ne!(invalid, source);
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn abstract_value_invocation_does_not_guess_a_branch_result() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {tag:true})'type;
        comptime fn choose(flag:bool)->Settings {
            if flag {settings {tag:true}} else {settings {tag:false}}
        }
        trait Build where mode:Settings {comptime fn build()-><|-bool>;}
        struct Host {}
        impl Build<choose(flag)> for Host where flag:bool {comptime fn build()-><|-bool> {'(true)}}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("AbstractValueUnresolved"), "{error}");
}

#[test]
fn abstract_boolean_transport_keeps_impl_input_identity() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true})'type;
        comptime fn keep(flag:bool)->bool {true and (false or flag)}
        trait Build where mode:Settings {comptime fn build()-><|-bool>;}
        struct Host {}
        impl Build<(settings {enabled:keep(flag)})> for Host where flag:bool {
            comptime fn build()-><|-bool> {'(flag)}
        }
        fn run()->bool {
            ~<Host.as(Build<(settings {enabled:true})>).build()>
            and not ~<Host.as(Build<(settings {enabled:false})>).build()>
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for observation in ["not flag", "flag and false", "flag or true"] {
        let rejected = source.replace("true and (false or flag)", observation);
        let error = support::compile_with_protocols(&rejected, &Default::default()).unwrap_err();
        assert!(
            error.contains("AbstractValueUnresolved"),
            "{observation}: {error}"
        );
    }
}

#[test]
fn type_computations_accept_unobserved_abstract_value_inputs() {
    let source = "comptime fn Payload(flag:bool)->.Type'builtin {bool}
        struct Flag where tag:bool {value:Payload(tag)}
        fn run()->bool {Flag<true>{value:true}.value and not Flag<false>{value:false}.value}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("{bool}", "{if flag {bool} else {bool}}");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("AbstractValueUnresolved"), "{error}");
}

#[test]
fn generic_type_computations_preserve_type_and_value_input_sorts() {
    let source = "comptime fn Payload(value:T)->.Type'builtin where T {T}
        struct Tagged where T, tag:T {value:Payload(tag)}
        fn run()->bool {
            let flag=Tagged<bool,true>{value:true};
            let number=Tagged<.u8'builtin,9>{value:7};
            flag.value and number.value == 7
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "Tagged<bool,true>{value:true}",
        "Tagged<bool,true>{value:7}",
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn newtype_value_parameters_keep_casts_and_nominal_identity() {
    let source = "newtype Flag<tag:bool> = bool;
        fn read(value:Flag<true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Flag<true>))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("read(true.as(Flag<true>))", "read(true.as(Flag<false>))");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn newtype_value_domains_depend_on_types_and_use_defaults() {
    let source = "newtype Tagged<T,tag:T> = T;
        newtype Defaulted<.tag:bool=true> = bool;
        fn read(value:Tagged<bool,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Tagged<bool,true>)) and true.as(Defaulted).as(bool)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("Tagged<bool,true>", "Tagged<bool,'x'>"),
        source.replace("true.as(Defaulted)", "true.as(Defaulted<.tag=3>)"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn newtype_value_indices_specialize_underlying_nominal_layouts() {
    let source = "struct Payload where tag:bool {value:bool}
        newtype Wrapped<tag:bool> = Payload<tag>;
        fn run()->bool {
            let wrapped=Payload<true>{value:true}.as(Wrapped<true>);
            wrapped.as(Payload<true>).value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("wrapped.as(Payload<true>)", "wrapped.as(Payload<false>)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn newtype_value_indices_cross_comptime_constants_and_forward_declarations() {
    let source = "typealias Chosen=Flag<true>;
        newtype Flag<tag:bool> = bool;
        comptime fn build()->Chosen {true.as(Chosen)}
        const SAVED=build();
        comptime fn unwrap()->bool {SAVED.as(bool)}";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    let id = package
        .functions
        .iter()
        .find(|f| f.name == "unwrap")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(vm.invoke(id, vec![]).unwrap(), Value::Bool(true));
}

#[test]
fn newtype_value_indices_select_distinct_implementations() {
    let source = "newtype Flag<tag:bool> = bool;
        trait Read {fn read(self)->bool;}
        impl Read for Flag<true> {fn read(self)->bool {self.as(bool)}}
        impl Read for Flag<false> {fn read(self)->bool {not self.as(bool)}}
        fn run()->bool {true.as(Flag<true>).as(Read).read() and false.as(Flag<false>).as(Read).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn newtype_value_indices_reject_runtime_domains_and_out_of_range_inputs() {
    for source in [
        "newtype Flag<tag:bool> = bool;fn bad(tag:bool)->bool {true.as(Flag<tag>).as(bool)}",
        "newtype Flag<tag:bool> = bool;fn bad()->bool {true.as(Flag<3>).as(bool)}",
        "newtype Flag<tag:.u8'builtin> = bool;fn bad()->bool {true.as(Flag<256>).as(bool)}",
        "newtype Flag<tag:bool> = bool;fn bad()->Flag<false> {true.as(Flag<true>)}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn newtype_generic_impl_generators_capture_value_indices() {
    let source = "newtype Flag<tag:bool> = bool;
        impl Flag<tag> where tag:bool {
            comptime fn build()-><|-bool> {'(tag)}
        }
        fn run()->bool {~<Flag<true>.build()> and not ~<Flag<false>.build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn newtype_mixed_parameters_preserve_formation_capabilities() {
    let source = "trait Marker {}impl Marker for bool {}
        newtype Guarded<T:-Marker,tag:bool> = T;
        fn run()->bool {true.as(Guarded<bool,true>).as(bool)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "true.as(Guarded<bool,true>).as(bool)",
        "'x'.as(Guarded<.char'builtin,true>).as(.char'builtin) == 'x'",
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nested_newtypes_capture_parent_value_identity() {
    let source = "struct Holder where tag:bool {pub newtype Wrapped=bool;}
        fn read(value:Holder<true>.Wrapped)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Holder<true>.Wrapped))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "read(true.as(Holder<true>.Wrapped))",
        "read(true.as(Holder<false>.Wrapped))",
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nested_newtype_own_value_domain_can_depend_on_captured_type() {
    let source = "struct Holder where T, tag:T {
            pub newtype Wrapped<value:T> = T;
        }
        fn run()->bool {true.as(Holder<bool,false>.Wrapped<true>).as(bool)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Wrapped<true>", "Wrapped<'x'>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nested_newtype_defaults_capture_selected_parent_and_keep_identity() {
    let source = "struct Holder where T, tag:T {
            pub newtype Wrapped<.value:T=tag> = T;
        }
        fn read(value:Holder<bool,false>.Wrapped)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Holder<bool,false>.Wrapped<.value=false>))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Wrapped<.value=false>", "Wrapped<.value=true>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nested_newtype_capture_specializes_underlying_layout() {
    let source = "struct Payload where tag:bool {value:bool}
        struct Holder where tag:bool {pub newtype Wrapped = Payload<tag>;}
        fn run()->bool {
            Payload<true>{value:true}.as(Holder<true>.Wrapped).as(Payload<true>).value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(".as(Payload<true>)", ".as(Payload<false>)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nested_newtype_explicit_values_use_caller_but_defaults_use_selected_parent() {
    let source = "struct Holder where tag:bool {
            pub newtype Wrapped<value:bool,.fallback:bool=tag> = bool;
            fn make()->Holder<false>.Wrapped<tag> {true.as(Holder<false>.Wrapped<tag>)}
        }
        fn read(value:Holder<false>.Wrapped<true,.fallback=false>)->bool {value.as(bool)}
        fn run()->bool {read(Holder<true>.make())}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_type_member_projection_preserves_value_captures() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {pub newtype Wrapped=bool;}
        comptime fn Build(tag:bool)->Type {Holder<tag>.Wrapped}
        fn read(value:Holder<true>.Wrapped)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_constructors_accept_mixed_inputs_and_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Flag where tag:bool {}
        enum Choice where tag:bool {none}
        newtype Wrapped<T,.tag:bool=true> = T;
        comptime fn Record(tag:bool)->Type {Flag<tag>}
        comptime fn Variant(tag:bool)->Type {Choice<tag>}
        comptime fn Wrapper(tag:bool)->Type {Wrapped<bool,.tag=tag>}
        fn record(value:Record(true))->Flag<true> {value}
        fn variant(value:Variant(false))->Choice<false> {value}
        fn read(value:Wrapped<bool,.tag=false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Wrapper(false)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "read(true.as(Wrapper(false)))",
        "read(true.as(Wrapper(true)))",
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn comptime_nominal_member_defaults_read_selected_parent_value() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {pub newtype Wrapped<.value:bool=tag> = bool;}
        comptime fn Build(tag:bool)->Type {Holder<tag>.Wrapped}
        fn read(value:Holder<false>.Wrapped<.value=false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(false)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_member_value_inputs_execute_once_in_source_order() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {pub newtype Wrapped<value:bool> = bool;}
        comptime fn flip(pointer:*bool)->bool {pointer.* = not pointer.*;pointer.*}
        comptime fn Build()->Type {
            var flag=false;
            Holder<flip(flag.ref)>.Wrapped<flip(flag.ref)>
        }
        fn read(value:Holder<true>.Wrapped<false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_constructor_rejects_wrong_domains_and_runtime_escape() {
    for source in [
        "typealias Type=.Type'builtin;struct Flag where tag:bool {}comptime fn Build()->Type {Flag<3>}",
        "typealias Type=.Type'builtin;struct Flag where tag:bool {}fn Build(tag:bool)->Type {Flag<tag>}",
        "typealias Type=.Type'builtin;struct Holder where tag:bool {pub newtype Wrapped=bool;}comptime fn Build()->Type {Holder<'x'>.Wrapped}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn comptime_nominal_mixed_packets_execute_once() {
    let source = "typealias Type=.Type'builtin;
        newtype Flags<a:bool,b:bool> = bool;
        comptime fn packet(pointer:*bool)->(bool,bool) {
            pointer.* = not pointer.*;
            (pointer.*,not pointer.*)
        }
        comptime fn Build()->Type {
            var flag=false;
            let result=Flags<...packet(flag.ref)>;
            if flag {result} else {Flags<false,false>}
        }
        fn read(value:Flags<true,false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_mixed_packet_domains_and_width_are_checked() {
    let source = "typealias Type=.Type'builtin;
        newtype Flag<T,tag:bool> = T;
        comptime fn packet()->(Type,bool) {(bool,true)}
        comptime fn Build()->Type {Flag<...packet()>}
        fn read(value:Flag<bool,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace(
            "(Type,bool) {(bool,true)}",
            "(Type,.char'builtin) {(bool,'x')}",
        ),
        source.replace("Flag<...packet()>", "Flag<...packet(),false>"),
        source.replace("Flag<...packet()>", "Flag<...true>"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn comptime_nominal_empty_packets_keep_effects_and_named_defaults() {
    let source = "typealias Type=.Type'builtin;
        newtype Flag<tag:bool,.other:bool=tag> = bool;
        comptime fn pack(...items:T)->T where T {items}
        comptime fn Build()->Type {
            var seen=false;
            Flag<...do {seen=true;pack()},seen>
        }
        fn read(value:Flag<true,.other=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_multiple_packets_and_named_inputs_preserve_source_order() {
    let source = "typealias Type=.Type'builtin;
        newtype Flags<a:bool,b:bool,.last:bool=false> = bool;
        comptime fn packet(pointer:*bool)->(bool,) {
            pointer.* = not pointer.*;(pointer.*,)
        }
        comptime fn Build()->Type {
            var flag=false;
            Flags<...packet(flag.ref),.last=flag,...packet(flag.ref)>
        }
        fn read(value:Flags<true,false,.last=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_member_packets_reuse_parent_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {pub newtype Wrapped<value:bool,.other:bool=tag> = bool;}
        comptime fn packet(value:bool)->(bool,) {(value,)}
        comptime fn Build(tag:bool)->Type {Holder<tag>.Wrapped<...packet(not tag)>}
        fn read(value:Holder<false>.Wrapped<true,.other=false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(false)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_packets_preserve_bound_tuple_normal_form() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        newtype Flags<a:bool,b:bool> = bool;
        comptime fn packet()->(bool,bool) {(true,false)}
        comptime fn Build()->Type {Flags<...packet()>}
        fn read(value:Flags<true,false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            tuple: Some("input::Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}
