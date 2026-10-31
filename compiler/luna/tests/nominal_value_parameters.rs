//! Value-indexed nominal identities and concrete layouts.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn scheduled_effect_execution_accepts_value_indexed_labels() {
    for (domain, key) in [("bool", "true"), ("Key", "Key{flag:true}")] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
            impl Duplicate for bool {{}}
            struct Key {{flag:bool}}
            effect choose()->bool where key:{domain};
            fn run()->bool {{choose<{key}>()# {{choose<{key}>()=>true,}}}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn unused_effect_defaults_are_checked_at_declaration() {
    for declaration in [
        "effect read()->bool where .tag:bool=9;",
        "effect enum Channel where .tag:bool=9 {read()->bool,}",
        "effect enum Channel where .T:.Type'builtin=true {read()->bool,}",
        "effect read()->bool where .T:.Type'builtin=true;",
        "effect enum Channel where .tag:bool=tag {read()->bool,}",
        "effect read()->bool where .tag:bool=tag;",
        "effect enum Channel where .tag:bool=later,later:bool {read()->bool,}",
        "effect read()->bool where .tag:bool=later,later:bool;",
    ] {
        assert!(
            support::compile_with_protocols(declaration, &Default::default()).is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn effect_dependent_value_defaults_preserve_prior_inputs() {
    let source = "effect enum Channel where T,key:T,.copy:T=key {read()->T,}
        fn request()->#Channel<bool,tag> bool where tag:bool {Channel<bool,tag>.read()#}
        fn run()->bool {request<true>()# {Channel<bool,true,.copy=true>.read()=>true,}
            and not request<false>()# {Channel<bool,false,.copy=false>.read()=>false,}}";
    let standalone = source
        .replace(
            "effect enum Channel where T,key:T,.copy:T=key {read()->T,}",
            "effect read()->T where T,key:T,.copy:T=key;",
        )
        .replace("Channel<", "read<")
        .replace(".read", "");
    for source in [source, standalone.as_str()] {
        let package = support::compile(source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let invalid = source.replace(".copy=true", ".copy=false");
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn effect_groups_inherit_value_domains_and_expand_exact_instances() {
    let source="effect enum Channel where tag:bool {read()->bool,write(value:bool),}
        fn request()->#Channel<tag> bool where tag:bool {Channel<tag>.write(true)#;Channel<tag>.read()#}
        fn install()->bool where tag:bool {request<tag>()# {
            Channel<tag>.read()=>tag,Channel<tag>.write(value)=>{},
        }}
        fn run()->bool {install<true>() and not install<false>()}";
    let package = support::compile(source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for wrong in [
        source.replace("Channel<tag>", "Channel<9>"),
        source.replace("Channel<tag>.read()=>tag", "Channel<false>.read()=>tag"),
        source.replace("Channel<tag>.write(value)=>{},", ""),
    ] {
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn effect_groups_admit_dependent_nominal_and_default_inputs() {
    for source in [
        "effect enum Channel where T,key:T {read()->T,}
         fn run()->bool {Channel<bool,true>.read()# {Channel<bool,true>.read()=>true,}}",
        "struct Key {flag:bool} effect enum Channel where key:Key {read()->bool,}
         fn run()->bool {Channel<Key{flag:true}>.read()# {Channel<Key{flag:true}>.read()=>true,}}",
        "effect enum Channel where .key:bool=true {read()->bool,}
         fn run()->bool {Channel.read()# {Channel<.key=true>.read()=>true,}}",
        "effect enum Channel where T,.U:.Type'builtin=T {read(value:T)->U,}
         fn run()->bool {Channel.read(true)# {Channel<bool>.read(value)=>value,}}",
        "struct Flag where tag:bool {value:bool}
         effect enum Channel where tag:bool {read(value:Flag<tag>)->bool,}
         fn run()->bool {Channel.read(Flag<true>{value:true})# {Channel<true>.read(value)=>value.value,}}",
        "effect read()->bool where .tag:bool=true;
         fn run()->bool {read()# {read<.tag=true>()=>true,}}",
    ] {
        let package=support::compile(source);
        assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true),"{source}");
    }
}

#[test]
fn effect_group_value_inference_accepts_computed_packets() {
    let source = "struct Flag where tag:bool {value:bool}
        effect enum Channel where tag:bool {read(value:Flag<tag>)->bool,}
        fn packet()->Tuple<Flag<true>> {(Flag<true>{value:true},)}
        fn run()->bool {Channel.read(...packet())# {Channel<true>.read(value)=>value.value,}}";
    let package = support::compile_with_tuple(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        support::compile_with_tuple_protocols(
            &source.replace("Channel<true>.read(value)", "Channel<false>.read(value)"),
            &Default::default(),
        )
        .is_err()
    );
}

#[test]
fn effect_nominal_value_arguments_use_checked_payload_layouts() {
    let source = "struct Key {flag:bool}
        effect choose()->bool where key:Key;
        fn run()->bool {
            choose<Key{flag:true}>()# {choose<Key{flag:true}>()=>true,}
                and not choose<Key{flag:false}>()# {choose<Key{flag:false}>()=>false,}
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn effect_nominal_values_forward_nested_enum_and_newtype_keys() {
    for (declarations, yes, no) in [
        (
            "struct Inner {flag:bool} struct Key {inner:Inner}",
            "Key{inner:Inner{flag:true}}",
            "Key{inner:Inner{flag:false}}",
        ),
        ("enum Key {none,some(bool)}", "Key.some(true)", "Key.none"),
        ("newtype Key=bool;", "true.as(Key)", "false.as(Key)"),
    ] {
        let source = format!(
            "{declarations}
            effect choose()->bool where key:Key;
            fn request()->#choose<key> bool where key:Key {{choose<key>()#}}
            fn run()->bool {{request<{yes}>()# {{choose<{yes}>()=>true,}}
                and not request<{no}>()# {{choose<{no}>()=>false,}}}}"
        );
        let package = support::compile(&source);
        elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, "run"),
            &[],
            &mut 100_000,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{declarations}"
        );
        let wrong = source.replace(
            &format!("choose<{yes}>()=>true"),
            &format!("choose<{no}>()=>true"),
        );
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn effect_value_parameters_select_distinct_handlers() {
    let source = "effect choose()->bool where tag:bool;
        fn subject()->#[choose<true>,choose<false>] bool {
            choose<true>()# and not choose<false>()#
        }
        fn run()->bool {subject()# {choose<true>()=>true,choose<false>()=>false,}}";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 100_000,
    )
    .unwrap();
    for wrong in ["bool", "9"] {
        assert!(
            support::compile_with_protocols(
                &source.replace("choose<true>", &format!("choose<{wrong}>")),
                &Default::default()
            )
            .is_err()
        );
    }
    assert!(
        support::compile_with_protocols(
            &source.replace("choose<false>()=>false,", ""),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn comptime_effect_type_values_validate_unused_value_domains() {
    let source = "typealias Type=.Type'builtin;
        effect choose()->bool where tag:bool;
        comptime fn Result()->Type {#choose<true> bool}";
    let mut package = support::compile(source);
    let entry = support::id(&package, "Result");
    let expected = nir::Term::computation(
        vec![nir::effects::Operation {
            declaration: package.effects[0].declaration,
            arguments: vec![nir::Term::Bool(true)],
        }],
        nir::Term::Primitive(nir::Primitive::Bool),
    );
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(entry, vec![]).unwrap(),
        Value::Type(expected.clone())
    );
    vm.validate_immutable_input(
        &Value::Type(expected.clone()),
        &nir::Term::Primitive(nir::Primitive::Type),
    )
    .unwrap();
    drop(vm);
    package.effects[0].parameter_types = vec![nir::Term::Primitive(nir::Primitive::U8)];
    match comptime_interpretor::Interpreter::for_comptime(&package, Default::default()) {
        Err(_) => {}
        Ok(mut vm) => assert!(
            vm.validate_immutable_input(
                &Value::Type(expected),
                &nir::Term::Primitive(nir::Primitive::Type)
            )
            .is_err()
        ),
    }
}

#[test]
fn effect_value_parameters_forward_through_generic_installers() {
    let source = "effect choose()->bool where tag:bool;
        fn request()->#choose<tag> bool where tag:bool {choose<tag>()#}
        fn install()->bool where tag:bool {request<tag>()# {choose<tag>()=>tag,}}
        fn run()->bool {install<true>() and not install<false>()}";
    let package = support::compile(source);
    elaboration::verification::memory::infer_in_package(
        &package,
        support::id(&package, "run"),
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn memoized_value_indexed_keys_keep_declaration_and_value_arguments() {
    let package=support::compile("const eq_key_phase={comptime_phase:true}; trait Eq {^eq_key_phase comptime fn eq(self,other:Self)->bool;}
        const hash_key_phase={comptime_phase:true}; trait Hash {^hash_key_phase comptime fn hash(self)->.u8'builtin;}
        comptime impl Eq for bool {comptime fn eq(self,other:bool)->bool {self == other}}
        comptime impl Hash for bool {comptime fn hash(self)->.u8'builtin {0}}
        struct Key where tag:bool {value:bool}
        comptime impl Eq for Key<tag> where tag:bool {comptime fn eq(self,other:Key<tag>)->bool {self.value == other.value}}
        comptime impl Hash for Key<tag> where tag:bool {comptime fn hash(self)->.u8'builtin {0}}
        pure comptime fn inspect(value:Key<tag>)->bool where tag:bool {value.value}
        pure comptime fn staged(comptime enabled:bool,value:Key<tag>)->bool where tag:bool {if enabled {value.value} else {false}}");
    let mut items = intrinsic::LangItems::new();
    for (item, name) in [
        (intrinsic::LangItem::Eq, "Eq"),
        (intrinsic::LangItem::Hash, "Hash"),
    ] {
        items
            .bind_source_traits(
                &package,
                &[(
                    item,
                    package
                        .traits
                        .iter()
                        .find(|t| t.name == name)
                        .unwrap()
                        .declaration,
                )],
            )
            .unwrap();
    }
    for (name, staged) in [("inspect", false), ("staged", true)] {
        for tag in [false, true] {
            let mut invocation = luna::memoization::Invocation::prepare(
                &package,
                &items,
                support::id(&package, name),
                vec![nir::Term::Bool(tag)],
                8,
                Default::default(),
            )
            .unwrap();
            let mut context = comptime_executor::Context::new(100_000);
            for (value, reused) in [(false, false), (true, false), (false, false), (true, false)] {
                let input = Value::Record {
                    declaration: package.structs[0].declaration,
                    arguments: vec![nir::Term::Bool(tag)],
                    fields: vec![Value::Bool(value)],
                };
                let inputs = if staged {
                    vec![Value::Bool(true), input]
                } else {
                    vec![input]
                };
                let outcome = invocation.invoke(inputs, &mut context).unwrap();
                assert_eq!(*outcome.value, Value::Bool(value));
                assert_eq!(outcome.reused, reused);
            }
            let foreign = Value::Record {
                declaration: package.structs[0].declaration,
                arguments: vec![nir::Term::Bool(!tag)],
                fields: vec![Value::Bool(true)],
            };
            let inputs = if staged {
                vec![Value::Bool(true), foreign]
            } else {
                vec![foreign]
            };
            assert!(invocation.invoke(inputs, &mut context).is_err());
            assert_eq!(invocation.cached_results(), 0);
        }
    }
}

#[test]
fn value_indexed_drop_methods_preserve_selected_arguments() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Flag where tag:bool {result:*bool}
        impl Retire for Flag<tag> where tag:bool {fn drop(*self) {self.result.*.* = tag;}}
        fn run()->bool {
            var yes=false;var no=true;
            do {let first=Flag<true>{result:yes.ref};let second=Flag<false>{result:no.ref};};
            yes and not no
        }";
    for contract in ["", " where ensures self.result.*.* == tag"] {
        let source = source.replace(
            "fn drop(*self) {self.result",
            &format!("fn drop(*self){contract} {{self.result"),
        );
        // Explicit premises allow frontend checking; cleanup planning must
        // still preserve and reject its currently unsupported postcondition.
        let source = if contract.is_empty() {
            source
        } else {
            source.replace(
                "self.result.*.* = tag;",
                "self.result.*.* = tag; assumes self.result.*.* == tag;",
            )
        };
        let package = support::compile(&source);
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if contract.is_empty() {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert!(
                result
                    .unwrap_err()
                    .contains("cleanup program specification obligations are not discharged")
            );
        }
    }
}

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
        comptime fn packet(pointer:*bool)->Tuple<bool,bool> {
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
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn comptime_nominal_mixed_packet_domains_and_width_are_checked() {
    let source = "typealias Type=.Type'builtin;
        newtype Flag<T,tag:bool> = T;
        comptime fn packet()->Tuple<Type,bool> {(bool,true)}
        comptime fn Build()->Type {Flag<...packet()>}
        fn read(value:Flag<bool,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
    for invalid in [
        source.replace(
            "Tuple<Type,bool> {(bool,true)}",
            "Tuple<Type,.char'builtin> {(bool,'x')}",
        ),
        source.replace("Flag<...packet()>", "Flag<...packet(),false>"),
        source.replace("Flag<...packet()>", "Flag<...true>"),
    ] {
        assert!(support::compile_with_tuple_protocols(&invalid, &Default::default()).is_err());
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
        comptime fn packet(pointer:*bool)->Tuple<bool> {
            pointer.* = not pointer.*;(pointer.*,)
        }
        comptime fn Build()->Type {
            var flag=false;
            Flags<...packet(flag.ref),.last=flag,...packet(flag.ref)>
        }
        fn read(value:Flags<true,false,.last=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn comptime_nominal_member_packets_reuse_parent_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {pub newtype Wrapped<value:bool,.other:bool=tag> = bool;}
        comptime fn packet(value:bool)->Tuple<bool> {(value,)}
        comptime fn Build(tag:bool)->Type {Holder<tag>.Wrapped<...packet(not tag)>}
        fn read(value:Holder<false>.Wrapped<true,.other=false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(false)))}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn comptime_nominal_packets_preserve_bound_tuple_normal_form() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        newtype Flags<a:bool,b:bool> = bool;
        comptime fn packet()->Tuple<bool,bool> {(true,false)}
        comptime fn Build()->Type {Flags<...packet()>}
        fn read(value:Flags<true,false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
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

#[test]
fn comptime_nominal_dependent_value_domains_wait_for_dynamic_types() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = T;
        comptime fn Build(ty:Type,value:bool)->Type {Tagged<ty,value>}
        fn read(value:Tagged<bool,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(bool,true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Build(bool,true)", "Build(.char'builtin,true)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn comptime_nominal_dependent_packet_domains_are_checked_at_invocation() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = T;
        comptime fn packet(ty:Type)->Tuple<Type,bool> {(ty,true)}
        comptime fn Build(ty:Type)->Type {Tagged<...packet(ty)>}
        fn read(value:Tagged<bool,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(bool)))}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
    let invalid = source.replace("Build(bool)", "Build(.char'builtin)");
    assert!(support::compile_with_tuple_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn comptime_nominal_dependent_defaults_reuse_checked_value_inputs() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,.copy:T=value> = T;
        comptime fn Build(ty:Type,value:bool)->Type {Tagged<ty,value>}
        fn read(value:Tagged<bool,true,.copy=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(bool,true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_nominal_dependent_domains_preserve_integer_source_types() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = bool;
        comptime fn Build(ty:Type,value:.u16'builtin)->Type {Tagged<ty,value>}
        fn run()->bool {true.as(Build(.u16'builtin,1)).as(bool)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Build(.u16'builtin,1)", "Build(.u8'builtin,1)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn comptime_nominal_dependent_domain_failure_allows_later_invocation() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = T;
        comptime fn Build(ty:Type,value:bool)->Type {Tagged<ty,value>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let build = support::id(&package, "Build");
    assert!(
        vm.invoke(
            build,
            vec![
                Value::Type(nir::Term::Primitive(nir::Primitive::Char)),
                Value::Bool(true)
            ]
        )
        .is_err()
    );
    let result = vm
        .invoke(
            build,
            vec![
                Value::Type(nir::Term::Primitive(nir::Primitive::Bool)),
                Value::Bool(true),
            ],
        )
        .unwrap();
    let Value::Type(nir::Term::Nominal { arguments, .. }) = result else {
        panic!("nominal Type expected")
    };
    assert_eq!(
        arguments,
        vec![
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::Bool(true)
        ]
    );
}

#[test]
fn comptime_nominal_dependent_inputs_allow_later_named_type_supply() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<.T:Type=bool,value:T> = T;
        comptime fn Build(ty:Type,value:bool)->Type {Tagged<value,.T=ty>}
        comptime fn Default(value:bool)->Type {Tagged<value>}
        fn read(value:Tagged<true,.T=bool>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(bool,true))) and read(true.as(Default(true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Build(bool,true)", "Build(.char'builtin,true)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn completed_comptime_parent_types_keep_value_indices_in_member_projection() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {pub newtype Wrapped=bool;}
        comptime fn Parent()->Type {Holder<true>}
        comptime fn Build()->Type {Parent().Wrapped}
        fn read(value:Holder<true>.Wrapped)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn completed_comptime_parent_projection_accepts_dynamic_member_type_inputs() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {pub newtype Wrapped<T> = T;}
        comptime fn Parent()->Type {Holder<true>}
        comptime fn Build(value:Type)->Type {Parent().Wrapped<value>}
        fn read(value:Holder<true>.Wrapped<bool>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(bool)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_member_value_domains_use_selected_parent_types() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T,tag:bool {pub newtype Wrapped<value:T> = T;}
        comptime fn Build(tag:bool,value:bool)->Type {Holder<bool,tag>.Wrapped<value>}
        fn read(value:Holder<bool,false>.Wrapped<true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(false,true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_member_value_domains_wait_for_dynamic_parent_types() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T,tag:bool {pub newtype Wrapped<value:T,.copy:T=value> = T;}
        comptime fn Build(ty:Type,tag:bool,value:bool)->Type {Holder<ty,tag>.Wrapped<value>}
        fn read(value:Holder<bool,false>.Wrapped<true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(bool,false,true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Build(bool,false,true)", "Build(.char'builtin,false,true)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn comptime_member_domains_do_not_inherit_the_callers_container_instance() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T,tag:bool {
            pub newtype Wrapped<value:T> = T;
            comptime fn Build(value:bool)->Type {Holder<bool,false>.Wrapped<value>}
        }
        fn read(value:Holder<bool,false>.Wrapped<true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Holder<.char'builtin,true>.Build(true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_reapplication_domains_do_not_capture_callers_own_parameters() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T,value:T {
            comptime fn Rebind(other:bool)->Type {Holder<bool,other>}
        }
        fn keep(value:Holder<.char'builtin,'x'>.Rebind(true))->Holder<bool,true> {value}";
    support::compile(source);
}

#[test]
fn generator_nominal_reapplication_uses_new_input_domains() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T,value:T {
            comptime fn build(other:bool)-><|-bool> {
                let selected=Holder<bool,other>;
                '(true)
            }
        }
        fn run()->bool {~<Holder<.char'builtin,'x'>.build(true)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_nominal_reapplication_restores_caller_bindings() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T,value:T {
            comptime fn build(other:bool)-><|-bool> {
                let selected=Holder<bool,other>;
                let original:T=value;
                let restored=Holder<T,original>;
                '(true)
            }
        }
        fn run()->bool {~<Holder<.char'builtin,'x'>.build(true)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Holder<bool,other>", "Holder<bool,value>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generator_nominal_reapplication_defaults_follow_new_type_inputs() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T,value:T,.copy:T=value {
            comptime fn build(other:bool)-><|-bool> {
                let selected=Holder<bool,other>;
                '(true)
            }
        }
        fn run()->bool {~<Holder<.char'builtin,'x'>.build(true)>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_spread_early_return_preserves_control_flow() {
    let source = "typealias Type=.Type'builtin;
        newtype Flag<tag:bool> = bool;
        comptime fn checked(value:bool)->bool where requires value {value}
        comptime fn Run()->bool {
            let ignored=Flag<...do {return true;(false,)}>;
            checked(false)
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_spread_early_return_checks_tails_and_skips_later_inputs() {
    for (declaration, arguments) in [
        ("newtype Flag<tag:bool> = bool;", "...do {return true;}"),
        ("newtype Flag<T> = bool;", "...do {return true;(bool,)}"),
        ("newtype Flag<T> = bool;", "...do {return true;}"),
        (
            "newtype Flag<a:bool,b:bool> = bool;",
            "...do {return true;},...do {checked(false);(false,)}",
        ),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;{declaration}
            comptime fn checked(value:bool)->bool where requires value {{value}}
            comptime fn Run()->bool {{let ignored=Flag<{arguments}>;checked(false)}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for tail in ["('x',)", "true", "(true,false)"] {
        let source = format!(
            "newtype Flag<tag:bool> = bool;
            comptime fn Run()->bool {{let ignored=Flag<...do {{return true;{tail}}}>;false}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{tail}"
        );
    }
}

#[test]
fn nominal_spread_conditional_return_keeps_both_paths() {
    let source = "newtype Flag<tag:bool> = bool;
        comptime fn Run(exit:bool)->bool {
            let ignored=Flag<...if exit {return true;} else {(false,)}>;
            false
        }";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let run = support::id(&package, "Run");
    for value in [true, false, true] {
        assert_eq!(
            vm.invoke(run, vec![Value::Bool(value)]).unwrap(),
            Value::Bool(value)
        );
    }
}

#[test]
fn generator_member_computed_defaults_use_selected_parent_instance() {
    let source = "struct Holder where tag:bool {
            pub newtype Wrapped<.value:bool=not tag> = bool;
            comptime fn build()-><|-bool> {
                let result=read(true.as(Holder<false>.Wrapped));
                '(result)
            }
        }
        comptime fn read(value:Holder<false>.Wrapped<.value=true>)->bool {value.as(bool)}
        fn run()->bool {~<Holder<true>.build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generator_member_defaults_restore_caller_across_repeated_uses() {
    let source = "struct Holder where tag:bool {
            pub newtype Wrapped<.value:bool=tag> = bool;
            comptime fn build()-><|-bool> {
                let first=read_default(true.as(Holder<false>.Wrapped));
                let second=true.as(Holder<false>.Wrapped<.value=tag>).as(bool);
                let third=read_default(true.as(Holder<false>.Wrapped));
                let result=first and second and third and tag;
                '(result)
            }
        }
        comptime fn read_default(value:Holder<false>.Wrapped<.value=false>)->bool {value.as(bool)}
        fn run()->bool {~<Holder<true>.build()>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let other = source.replace("Holder<true>.build()", "Holder<false>.build()");
    assert_eq!(support::run_iir(&other, "run", vec![]), Value::Bool(false));
}

#[test]
fn computed_member_defaults_work_in_ordinary_comptime_type_functions() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where tag:bool {
            pub newtype Wrapped<.value:bool=not tag> = bool;
            comptime fn Build(value:Type)->Type {Holder<false>.Wrapped}
        }
        fn read(value:Holder<false>.Wrapped<.value=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Holder<true>.Build(bool)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_integer_waits_for_dynamic_type() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = bool;
        comptime fn Build(ty:Type)->Type {Tagged<ty,255>}
        fn run()->bool {true.as(Build(.u8'builtin)).as(bool)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("255>", "256>"),
        source.replace("Build(.u8'builtin)", "Build(bool)"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn nominal_contextual_branch_and_block_preserve_source_order() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,last:bool> = bool;
        comptime fn Build(ty:Type,choose:bool)->Type {
            var seen=false;
            Tagged<ty,do {let selected=choose;seen=true;if selected {7} else {8}},seen>
        }
        fn read(value:Tagged<.u8'builtin,7,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin,true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_matches_check_unselected_literal_branches() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = bool;
        comptime fn Build(ty:Type,choose:bool)->Type {
            Tagged<ty,choose match {true=>7,false=>8}>
        }
        fn read(value:Tagged<.u8'builtin,7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin,true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("false=>8", "false=>256");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nominal_contextual_negative_integer_keeps_signedness() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = bool;
        comptime fn Build(ty:Type)->Type {Tagged<ty,-128>}
        fn run()->bool {true.as(Build(.i8'builtin)).as(bool)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("-128>", "-129>"),
        source.replace("Build(.i8'builtin)", "Build(.u8'builtin)"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn nominal_contextual_named_value_precedes_its_type_once() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<.T:Type=bool,value:T,last:bool> = bool;
        comptime fn Build(ty:Type)->Type {
            var seen=false;
            Tagged<do {seen=not seen;7},.T=do {if not seen {return bool;};ty},seen>
        }
        fn read(value:Tagged<.T=.u8'builtin,7,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_named_unselected_branch_is_checked_after_type_arrives() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<.T:Type=bool,value:T> = bool;
        comptime fn Build(ty:Type,choose:bool)->Type {
            Tagged<if choose {7} else {8},.T=ty>
        }
        fn read(value:Tagged<.T=.u8'builtin,7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin,true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("else {8}", "else {256}"),
        source.replace("Build(.u8'builtin,true)", "Build(bool,true)"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn nominal_contextual_named_return_skips_later_type_input() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<.T:Type=bool,value:T> = bool;
        comptime fn Fail(value:bool)->Type where requires value {bool}
        comptime fn Build()->Type {Tagged<do {return bool;7},.T=Fail(false)>}";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "Build", 100_000).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn nominal_contextual_defaults_share_preceding_values() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,.copy:T=value> = bool;
        comptime fn Build(ty:Type)->Type {Tagged<ty,7>}
        fn read(value:Tagged<.u8'builtin,7,.copy=7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_default_type_finishes_earlier_value() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<.T:Type=.u8'builtin,value:T,.copy:T=value> = bool;
        comptime fn Build()->Type {Tagged<7>}
        fn read(value:Tagged<7,.T=.u8'builtin,.copy=7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build()))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Tagged<7>}", "Tagged<256>}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nominal_contextual_defaults_do_not_replay_explicit_effects() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,last:bool,.copy:T=value> = bool;
        comptime fn Build(ty:Type)->Type {
            var seen=false;
            Tagged<ty,do {seen=not seen;7},seen>
        }
        fn read(value:Tagged<.u8'builtin,7,true,.copy=7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_literal_defaults_check_all_branches() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,choose:bool,.value:T=if choose {7} else {8}> = bool;
        comptime fn Build(ty:Type)->Type {Tagged<ty,true>}
        fn read(value:Tagged<.u8'builtin,true,.value=7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("else {8}", "else {256}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nominal_contextual_default_chains_preserve_caller_scope() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,.copy:T=value,.again:T=copy> = bool;
        comptime fn Build(ty:Type)->Type {
            let copy=false;
            let first=Tagged<ty,7>;
            if copy {return bool;};
            Tagged<ty,8>
        }
        fn read(value:Tagged<.u8'builtin,8,.copy=8,.again=8>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_packet_type_supplies_literal_domain_once() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,last:bool> = bool;
        comptime fn Build(ty:Type)->Type {
            var seen=false;
            Tagged<...do {seen=not seen;(ty,)},if seen {7} else {8},seen>
        }
        fn read(value:Tagged<.u8'builtin,7,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("else {8}", "else {256}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nominal_contextual_packet_resolves_prior_input_before_default() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,.value:T=7,.copy:T=value,last:bool> = bool;
        comptime fn Build(ty:Type)->Type {
            var seen=false;
            Tagged<.value=do {seen=not seen;7},...do {if not seen {return bool;};(ty,)},seen>
        }
        fn read(value:Tagged<.u8'builtin,.value=7,.copy=7,true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_empty_packets_preserve_effects_between_inputs() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,.last:bool=false> = bool;
        comptime fn pack(...items:T)->T where T {items}
        comptime fn Build(ty:Type)->Type {
            var seen=false;
            Tagged<...do {(ty,)},7,...do {seen=not seen;pack()},.last=seen>
        }
        fn read(value:Tagged<.u8'builtin,7,.last=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nominal_contextual_packet_return_skips_remaining_inputs() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T,.last:bool=false> = bool;
        comptime fn checked(value:bool)->bool where requires value {value}
        comptime fn Build()->Type {
            Tagged<...do {return bool;(.u8'builtin,)},7,.last=checked(false)>
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "Build", 100_000).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn nominal_contextual_trailing_empty_packet_can_return() {
    let source = "typealias Type=.Type'builtin;
        newtype Tagged<T,value:T> = bool;
        comptime fn pack(...items:T)->T where T {items}
        comptime fn Build(ty:Type)->Type {
            Tagged<ty,7,...do {return bool;pack()}>
        }";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(
            support::id(&package, "Build"),
            vec![Value::Type(nir::Term::Primitive(nir::Primitive::U8))]
        )
        .unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn contextual_parent_defaults_remain_available_to_member_types() {
    let source = "typealias Type=.Type'builtin;
        struct Parent where T,value:T,.copy:T=value { pub newtype Wrapped=bool; }
        comptime fn Build(ty:Type)->Type {Parent<ty,7>.Wrapped}
        fn read(value:Parent<.u8'builtin,7,.copy=7>.Wrapped)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn contextual_parent_packet_executes_once_before_member_inputs() {
    let source = "typealias Type=.Type'builtin;
        struct Parent where T,value:T,.copy:T=value { pub newtype Wrapped<.tag:bool=false> = bool; }
        comptime fn Build(ty:Type)->Type {
            var seen=false;
            Parent<...do {seen=not seen;(ty,)},7>.Wrapped<.tag=seen>
        }
        fn read(value:Parent<.u8'builtin,7,.copy=7>.Wrapped<.tag=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn contextual_parent_checks_formation_before_member_arguments() {
    let source = "typealias Type=.Type'builtin;
        trait Allowed {} impl Allowed for .u8'builtin {}
        struct Parent where T:-Allowed,value:T {
            pub newtype Wrapped<.tag:bool=false> = bool;
        }
        comptime fn member()->bool {true}
        comptime fn Build(ty:Type)->Type {Parent<ty,7>.Wrapped<.tag=member()>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let build = support::id(&package, "Build");
    let member = support::id(&package, "member");
    let input = || Value::Type(nir::Term::Primitive(nir::Primitive::U8));
    assert!(
        vm.invoke(
            build,
            vec![Value::Type(nir::Term::Primitive(nir::Primitive::U16))]
        )
        .is_err()
    );
    assert!(!vm.events().iter().any(
        |event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == member)
    ));
    assert!(matches!(
        vm.invoke(build, vec![input()]).unwrap(),
        Value::Type(_)
    ));
    assert_eq!(vm.events().iter().filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == member)).count(), 1);
}

#[test]
fn contextual_alias_defaults_keep_external_type_parameters_open() {
    let source = "typealias Type=.Type'builtin;
        struct Parent where T,.value:T=7,.copy:T=value { pub newtype Wrapped=bool; }
        typealias Alias<T> = Parent<T>;
        comptime fn Build(ty:Type)->Type {Alias<ty>.Wrapped}
        fn read(value:Parent<.u8'builtin,.value=7,.copy=7>.Wrapped)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn contextual_alias_defaults_validate_unselected_branches_at_invocation() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,.value:T=if true {7} else {255}> = bool;
        typealias Inner<T> = Indexed<T>;
        typealias Outer<U> = Inner<U>;
        comptime fn Build(ty:Type)->Type {Outer<ty>}
        fn run()->bool {true.as(Build(.u8'builtin)).as(bool)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("else {255}", "else {256}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn contextual_alias_default_invocations_do_not_share_concrete_type_state() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,.value:T=7> = bool;
        typealias Alias<T> = Indexed<T>;
        comptime fn Build(ty:Type)->Type {Alias<ty>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let build = support::id(&package, "Build");
    assert!(
        vm.invoke(
            build,
            vec![Value::Type(nir::Term::Primitive(nir::Primitive::Bool))]
        )
        .is_err()
    );
    let byte = vm
        .invoke(
            build,
            vec![Value::Type(nir::Term::Primitive(nir::Primitive::U8))],
        )
        .unwrap();
    let word = vm
        .invoke(
            build,
            vec![Value::Type(nir::Term::Primitive(nir::Primitive::U16))],
        )
        .unwrap();
    assert_ne!(byte, word);
    assert_eq!(
        vm.invoke(
            build,
            vec![Value::Type(nir::Term::Primitive(nir::Primitive::U8))]
        )
        .unwrap(),
        byte
    );
}

#[test]
fn contextual_alias_explicit_literals_keep_external_type_parameters_open() {
    let source = "typealias Type=.Type'builtin;
        struct Parent where T,value:T,.copy:T=value { pub newtype Wrapped=bool; }
        typealias Alias<T> = Parent<T,7>;
        comptime fn Build(ty:Type)->Type {Alias<ty>.Wrapped}
        fn read(value:Parent<.u8'builtin,7,.copy=7>.Wrapped)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn transparent_aliases_accept_value_parameters_without_adding_identity() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<flag:bool> = bool;
        typealias Alias<flag:bool> = Indexed<flag>;
        comptime fn Build(flag:bool)->Type {Alias<flag>}
        fn read(value:Indexed<true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(true)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn transparent_value_aliases_preserve_static_identity_and_defaults() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<flag:bool,.copy:bool=flag> = bool;
        typealias Inner<flag:bool,.copy:bool=flag> = Indexed<flag,.copy=copy>;
        typealias Outer<flag:bool> = Inner<flag>;
        fn read(value:Indexed<true,.copy=true>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Outer<true>))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let dynamic = source.replace("fn run()->bool {read(true.as(Outer<true>))}",
        "comptime fn Build(flag:bool)->Type {Outer<flag>} fn run()->bool {read(true.as(Build(true)))}");
    assert_eq!(support::run_iir(&dynamic, "run", vec![]), Value::Bool(true));
}

#[test]
fn transparent_value_aliases_reject_wrong_domains_even_when_unused() {
    for source in [
        "typealias Alias<flag:bool> = bool; fn run(value:Alias<7>)->bool {value}",
        "typealias Alias<.flag:bool=7> = bool;",
        "typealias Alias<.flag:bool=flag> = bool;",
        "typealias Alias<.flag:bool=next,.next:bool=true> = bool;",
        "typealias Type=.Type'builtin;typealias Alias<flag:bool> = bool; comptime fn Build()->Type {Alias<7>}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn transparent_value_aliases_use_declared_integer_domains() {
    let source = "typealias Type=.Type'builtin;
        typealias Byte=.u8'builtin;
        newtype Indexed<value:Byte> = bool;
        typealias Alias<value:Byte> = Indexed<value>;
        comptime fn Build(value:Byte)->Type {Alias<value>}
        fn read(value:Indexed<255>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(255)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace("Build(255)", "Build(256)"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn transparent_value_aliases_accept_dependent_domains_with_concrete_type_inputs() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        typealias Alias<T,value:T> = Indexed<T,value>;
        fn read(value:Indexed<.u8'builtin,7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Alias<.u8'builtin,7>))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn transparent_alias_value_inputs_execute_once_in_source_order() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<left:bool,right:bool> = bool;
        typealias Alias<left:bool,.right:bool=left> = Indexed<right,left>;
        comptime fn First()->bool {true}
        comptime fn Second()->bool {false}
        comptime fn Build()->Type {Alias<First(),.right=Second()>}
        comptime fn Expected()->Type {Indexed<false,true>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let actual = vm.invoke(support::id(&package, "Build"), vec![]).unwrap();
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| {
            let comptime_interpretor::Event::EnterFunction(id) = event else {
                return None;
            };
            package
                .functions
                .iter()
                .find(|f| f.declaration == *id)
                .map(|f| f.name.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, vec!["Build", "First", "Second"]);
    assert_eq!(
        actual,
        vm.invoke(support::id(&package, "Expected"), vec![])
            .unwrap()
    );
}

#[test]
fn transparent_alias_dependent_defaults_keep_integer_literals_contextual() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        typealias Alias<T,.value:T=7> = Indexed<T,value>;
        fn read(value:Indexed<.u8'builtin,7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Alias<.u8'builtin>))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace(".value:T=7", ".value:T=256"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn transparent_value_aliases_keep_trait_guards_before_body_evaluation() {
    let source = "typealias Type=.Type'builtin;
        trait Allowed {} impl Allowed for bool {}
        newtype Indexed<T,flag:bool> = bool;
        comptime fn Body(ty:Type,flag:bool)->Type {Indexed<ty,flag>}
        typealias Alias<T:-Allowed,flag:bool> = Body(T,flag);
        comptime fn Build(ty:Type,flag:bool)->Type {Alias<ty,flag>}
        comptime fn Expected()->Type {Indexed<bool,true>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let build = support::id(&package, "Build");
    assert!(
        vm.invoke(
            build,
            vec![
                Value::Type(nir::Term::Primitive(nir::Primitive::U8)),
                Value::Bool(true)
            ]
        )
        .is_err()
    );
    let body = support::id(&package, "Body");
    assert!(!vm.events().iter().any(
        |event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == body)
    ));
    let actual = vm
        .invoke(
            build,
            vec![
                Value::Type(nir::Term::Primitive(nir::Primitive::Bool)),
                Value::Bool(true),
            ],
        )
        .unwrap();
    assert_eq!(
        actual,
        vm.invoke(support::id(&package, "Expected"), vec![])
            .unwrap()
    );
}

#[test]
fn member_value_aliases_use_parent_domains_and_default_values() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        struct Parent where T,value:T {
            pub typealias Alias<.copy:T=value> = Indexed<T,copy>;
        }
        comptime fn Build(value:.u8'builtin)->Type {Parent<.u8'builtin,value>.Alias}
        fn read(value:Indexed<.u8'builtin,7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(7)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let explicit = source.replace(".Alias}", ".Alias<.copy=8>}").replace(
        "value:Indexed<.u8'builtin,7>",
        "value:Indexed<.u8'builtin,8>",
    );
    assert_eq!(
        support::run_iir(&explicit, "run", vec![]),
        Value::Bool(true)
    );
    let through_alias = source
        .replace(
            "comptime fn Build",
            "typealias ParentAlias<T,value:T> = Parent<T,value>; comptime fn Build",
        )
        .replace(
            "{Parent<.u8'builtin,value>",
            "{ParentAlias<.u8'builtin,value>",
        );
    assert_eq!(
        support::run_iir(&through_alias, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn member_alias_guards_capture_value_indexed_trait_requirements() {
    let source = "typealias Type=.Type'builtin;
        trait Allowed where flag:bool {}
        impl Allowed<true> for bool {}
        struct Holder where flag:bool,T,requires T:-Allowed<flag> {
            pub typealias Checked<U:-Allowed<flag>> = T;
        }
        comptime fn Build(flag:bool,inner:Type)->Type {Holder<flag,bool>.Checked<inner>}";
    let shorthand = source.replace(
        "flag:bool,T,requires T:-Allowed<flag>",
        "flag:bool,T:-Allowed<flag>",
    );
    for source in [source, shorthand.as_str()] {
        let package = support::compile(source);
        assert!(package.alias_contracts.iter().any(|contract| {
            contract
                .parameter_types
                .contains(&nir::Term::Primitive(nir::Primitive::Bool))
        }));
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let build = support::id(&package, "Build");
        let ty = |primitive| Value::Type(nir::Term::Primitive(primitive));
        assert!(
            vm.invoke(build, vec![Value::Bool(true), ty(nir::Primitive::Char)])
                .is_err()
        );
        assert_eq!(
            vm.invoke(build, vec![Value::Bool(true), ty(nir::Primitive::Bool)])
                .unwrap(),
            ty(nir::Primitive::Bool)
        );
        assert!(
            vm.invoke(build, vec![Value::Bool(false), ty(nir::Primitive::Bool)])
                .is_err()
        );
    }
}

#[test]
fn value_indexed_bound_sugar_is_available_across_type_declarations() {
    for definition in [
        "struct Chosen where flag:bool,T:-Allowed<flag> {}",
        "enum Chosen where flag:bool,T:-Allowed<flag> {none}",
        "newtype Chosen<flag:bool,T:-Allowed<flag>> = T;",
        "typealias Chosen<flag:bool,T:-Allowed<flag>> = T;",
    ] {
        let source = format!(
            "trait Allowed where flag:bool {{}} impl Allowed<true> for bool {{}} {definition} fn keep(value:Chosen<true,bool>)->Chosen<true,bool> {{value}}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_ok(),
            "{source}"
        );
        let invalid = source.replace("Chosen<true,bool>", "Chosen<false,bool>");
        assert!(
            support::compile_with_protocols(&invalid, &Default::default()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn member_alias_checks_dynamic_parent_domain_even_for_unused_values() {
    let source = "typealias Type=.Type'builtin;
        struct Parent where T,value:T {
            pub typealias Alias<.copy:T=value> = bool;
        }
        comptime fn Build(ty:Type)->Type {Parent<ty,7>.Alias}
        fn run()->bool {let value:Build(.u8'builtin)=true;value}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn dynamic_alias_domain_failures_skip_body_and_do_not_poison_later_invocations() {
    let source = "typealias Type=.Type'builtin;
        comptime fn Body()->Type {bool}
        struct Parent where T,value:T {
            pub typealias Alias<.copy:T=value> = Body();
        }
        comptime fn Build(ty:Type)->Type {Parent<ty,7>.Alias}
        comptime fn Invalid(ty:Type)->Type {Parent<ty,7>.Alias<.copy=true>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let body = support::id(&package, "Body");
    let bad = support::id(&package, "Invalid");
    let good = support::id(&package, "Build");
    let ty = |primitive| Value::Type(nir::Term::Primitive(primitive));
    for primitive in [nir::Primitive::U8, nir::Primitive::U16, nir::Primitive::U8] {
        let before = vm.events().iter().filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == body)).count();
        assert!(vm.invoke(bad, vec![ty(primitive)]).is_err());
        assert_eq!(before, vm.events().iter().filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == body)).count());
        assert_eq!(
            vm.invoke(good, vec![ty(primitive)]).unwrap(),
            ty(nir::Primitive::Bool)
        );
        assert_eq!(before + 1, vm.events().iter().filter(|event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == body)).count());
    }
}

#[test]
fn member_alias_body_uses_the_checked_dynamic_parent_domain() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        struct Parent where T,value:T {
            pub typealias Alias<.copy:T=value> = Indexed<T,copy>;
        }
        comptime fn Build(ty:Type)->Type {Parent<ty,7>.Alias}
        fn read(value:Indexed<.u8'builtin,7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let nested = source
        .replace(
            "struct Parent",
            "typealias Forward<T,value:T> = Indexed<T,value>; struct Parent",
        )
        .replace("= Indexed<T,copy>", "= Forward<T,copy>");
    assert_eq!(support::run_iir(&nested, "run", vec![]), Value::Bool(true));
    let wider = source.replace(".u8'builtin", ".u16'builtin");
    assert_eq!(support::run_iir(&wider, "run", vec![]), Value::Bool(true));
    let literal_default = source.replace(".copy:T=value", ".copy:T=7");
    assert_eq!(
        support::run_iir(&literal_default, "run", vec![]),
        Value::Bool(true)
    );
    let explicit_literal = source.replace(".Alias}", ".Alias<.copy=8>}").replace(
        "value:Indexed<.u8'builtin,7>",
        "value:Indexed<.u8'builtin,8>",
    );
    assert_eq!(
        support::run_iir(&explicit_literal, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn transparent_alias_literals_wait_for_dynamic_type_inputs() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        typealias Alias<T,value:T> = Indexed<T,value>;
        comptime fn Build(ty:Type)->Type {Alias<ty,7>}
        fn read(value:Indexed<.u8'builtin,7>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Build(.u8'builtin)))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let defaulted = source
        .replace("Alias<T,value:T>", "Alias<T,.value:T=7>")
        .replace("Alias<ty,7>", "Alias<ty>");
    assert_eq!(
        support::run_iir(&defaulted, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn dynamic_alias_literals_check_unselected_branches_and_recover() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        typealias Alias<T,.value:T=if true {7} else {255}> = Indexed<T,value>;
        comptime fn Build(ty:Type)->Type {Alias<ty>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let build = support::id(&package, "Build");
    let ty = |primitive| Value::Type(nir::Term::Primitive(primitive));
    assert!(vm.invoke(build, vec![ty(nir::Primitive::Bool)]).is_err());
    let byte = vm.invoke(build, vec![ty(nir::Primitive::U8)]).unwrap();
    let word = vm.invoke(build, vec![ty(nir::Primitive::U16)]).unwrap();
    assert_ne!(byte, word);
    assert_eq!(
        vm.invoke(build, vec![ty(nir::Primitive::U8)]).unwrap(),
        byte
    );
    let invalid = support::compile(&source.replace("else {255}", "else {256}"));
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&invalid, Default::default()).unwrap();
    assert!(
        vm.invoke(support::id(&invalid, "Build"), vec![ty(nir::Primitive::U8)])
            .is_err()
    );
    assert!(
        vm.invoke(
            support::id(&invalid, "Build"),
            vec![ty(nir::Primitive::U16)]
        )
        .is_ok()
    );
}

#[test]
fn dynamic_alias_pending_literals_preserve_packet_and_effect_order() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        typealias Alias<T,.value:T=7> = Indexed<T,value>;
        comptime fn Mark()->bool {true}
        comptime fn InputType()->Type {.u8'builtin}
        comptime fn Build()->Type {Alias<.value=do {Mark();7},...do {let ty=InputType();(ty,)}>}
        comptime fn Expected()->Type {Indexed<.u8'builtin,7>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let actual = vm.invoke(support::id(&package, "Build"), vec![]).unwrap();
    assert_eq!(vm.bytecode_input_preparations(), 1);
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| {
            let comptime_interpretor::Event::EnterFunction(id) = event else {
                return None;
            };
            package
                .functions
                .iter()
                .find(|function| function.declaration == *id)
                .map(|function| function.name.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, vec!["Build", "Mark", "InputType"]);
    assert_eq!(
        actual,
        vm.invoke(support::id(&package, "Expected"), vec![])
            .unwrap()
    );
}

#[test]
fn dynamic_alias_early_return_skips_later_inputs_and_defaults() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<T,value:T> = bool;
        comptime fn Default()->bool {true}
        typealias Alias<T,value:T,.flag:bool=Default()> = Indexed<T,value>;
        comptime fn Later()->.u8'builtin {7}
        comptime fn Run()->bool {Alias<do {return true;.u8'builtin},Later()>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let run = support::id(&package, "Run");
    assert_eq!(vm.invoke(run, vec![]).unwrap(), Value::Bool(true));
    assert_eq!(
        vm.events()
            .iter()
            .filter_map(|event| match event {
                comptime_interpretor::Event::EnterFunction(id) => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![run]
    );
}

#[test]
fn contextual_member_alias_arguments_use_caller_and_defaults_use_selected_parent() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<flag:bool,copy:bool> = bool;
        struct Holder where tag:bool {
            pub typealias Alias<flag:bool,.copy:bool=tag> = Indexed<flag,copy>;
            pub typealias Other<.unused:bool=true> = Holder<false>.Alias<tag>;
        }
        fn read(value:Indexed<true,false>)->bool {value.as(bool)}
        fn run()->bool {read(true.as(Holder<true>.Other))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn member_comptime_functions_read_captured_value_parameters() {
    let source = "typealias Type=.Type'builtin;
        newtype Indexed<flag:bool,copy:bool> = bool;
        struct Holder where tag:bool {
            pub typealias Alias<flag:bool,.copy:bool=tag> = Indexed<flag,copy>;
            pub comptime fn Build()->Type {Holder<false>.Alias<tag>}
            pub comptime fn read()->bool {tag}
        }
        comptime fn Build()->Type {Holder<true>.Build()}
        comptime fn Expected()->Type {Indexed<true,false>}
        comptime fn run()->bool {Holder<true>.read() and not Holder<false>.read()}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let build = package
        .functions
        .iter()
        .find(|f| f.name == "Build" && f.type_parameters.is_empty())
        .unwrap()
        .declaration;
    assert_eq!(
        vm.invoke(build, vec![]).unwrap(),
        vm.invoke(support::id(&package, "Expected"), vec![])
            .unwrap()
    );
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn member_functions_materialize_captured_values_in_runtime_bodies() {
    let source = "struct Holder where T, tag:T {
            pub fn read()->T {tag}
            pub fn callback()->fn()->T {|| tag}
        }
        fn run()->bool {
            Holder<bool,true>.read() and not Holder<bool,false>.callback()()
        }
        fn byte()->.u8'builtin {Holder<.u8'builtin,255>.read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert_eq!(
        support::run_iir(source, "byte", vec![]),
        Value::UnsignedInteger(255)
    );
}

#[test]
fn member_comptime_value_capture_has_dependent_domain() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where T, tag:T {pub comptime fn read()->T {tag}}
        comptime fn byte()->.u8'builtin {Holder<.u8'builtin,255>.read()}
        comptime fn flag()->bool {Holder<bool,true>.read()}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            vm.invoke(support::id(&package, "byte"), vec![]).unwrap(),
            Value::UnsignedInteger(255)
        );
        assert_eq!(
            vm.invoke(support::id(&package, "flag"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
    assert!(
        support::compile_with_protocols(
            &source.replace("Holder<.u8'builtin,255>", "Holder<.u8'builtin,256>"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn member_value_capture_defaults_and_pure_calls_keep_selected_instance() {
    let source = "struct Holder where tag:bool {
            pub pure fn read(.value:bool=tag)->bool {value}
        }
        fn run()->bool {
            Holder<true>.read() and not Holder<false>.read()
                and Holder<false>.read(.value=true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn impl_functions_read_value_parameters_without_replaying_other_instances() {
    let source = "struct Holder where tag:bool {}
        impl Holder<tag> where tag:bool {
            pure fn read()->bool {tag}
            comptime fn inspect()->bool {tag}
        }
        fn run()->bool {Holder<true>.read() and not Holder<false>.read()}
        comptime fn inspect()->bool {Holder<false>.inspect() or Holder<true>.inspect()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let package = support::compile(source);
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "inspect" && f.type_parameters.is_empty())
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(vm.invoke(entry, vec![]).unwrap(), Value::Bool(true));
}

#[test]
fn captured_value_does_not_excuse_impure_member_body() {
    let source = "struct Holder where tag:bool {
        pub pure fn bad(pointer:*bool)->bool {pointer.* = tag; tag}
    }";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn member_functions_read_nested_record_value_parameters() {
    let source="comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings=(settings {enabled:true,nested:(settings {ready:false})})'type;
        struct Host where mode:Settings {
            pub comptime fn inspect()->bool {mode.enabled and not mode.nested.ready}
            pub pure fn read()->bool {mode.enabled and not mode.nested.ready}
            pub fn pick(a:*bool,b:*bool)->*bool {if mode.enabled and not mode.nested.ready {a} else {b}}
            pub fn dynamic(flag:bool,a:*bool,b:*bool)->*bool {if flag and mode.enabled {a} else {b}}
        }
        comptime fn inspect()->bool {Host<(settings {enabled:true,nested:(settings {ready:false})})>.inspect()}
        fn run()->bool {Host<(settings {enabled:true,nested:(settings {ready:false})})>.read() and not Host<(settings {enabled:true,nested:(settings {ready:true})})>.read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let package = support::compile(source);
    let id = package
        .functions
        .iter()
        .find(|f| f.name == "inspect" && f.type_parameters.is_empty())
        .unwrap()
        .declaration;
    let pick = package
        .functions
        .iter()
        .find(|f| f.name == "pick")
        .unwrap()
        .declaration;
    for (enabled, ready, expected) in [
        (true, false, 0),
        (true, true, 1),
        (false, false, 1),
        (true, false, 0),
    ] {
        let mode = nir::Term::RecordValue(vec![
            ("enabled".into(), nir::Term::Bool(enabled)),
            (
                "nested".into(),
                nir::Term::RecordValue(vec![("ready".into(), nir::Term::Bool(ready))]),
            ),
        ]);
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            pick,
            &[mode],
            &mut 100_000,
        )
        .unwrap();
        let targets = summary.result_targets.unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets.iter().next().unwrap().parameter, expected);
    }
    let mode = nir::Term::RecordValue(vec![
        ("enabled".into(), nir::Term::Bool(true)),
        (
            "nested".into(),
            nir::Term::RecordValue(vec![("ready".into(), nir::Term::Bool(false))]),
        ),
    ]);
    let dynamic = package
        .functions
        .iter()
        .find(|f| f.name == "dynamic")
        .unwrap()
        .declaration;
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        dynamic,
        std::slice::from_ref(&mode),
        &mut 100_000,
    )
    .unwrap();
    assert_eq!(
        summary
            .result_targets
            .unwrap()
            .iter()
            .map(|p| p.parameter)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    for invalid in [
        nir::Term::RecordValue(vec![("enabled".into(), nir::Term::Bool(true))]),
        nir::Term::RecordValue(vec![
            ("enabled".into(), nir::Term::Bool(true)),
            (
                "nested".into(),
                nir::Term::RecordValue(vec![("ready".into(), nir::Term::Char('x'))]),
            ),
        ]),
        nir::Term::RecordValue(vec![
            ("enabled".into(), nir::Term::Bool(true)),
            (
                "nested".into(),
                nir::Term::RecordValue(vec![("wrong".into(), nir::Term::Bool(false))]),
            ),
        ]),
    ] {
        assert!(
            elaboration::verification::memory::infer_in_package(
                &package,
                pick,
                &[invalid],
                &mut 100_000,
            )
            .is_err()
        );
    }
    assert!(
        elaboration::verification::memory::infer_in_package(
            &package,
            pick,
            std::slice::from_ref(&mode),
            &mut 2,
        )
        .is_err()
    );
    let recovered =
        elaboration::verification::memory::infer_in_package(&package, pick, &[mode], &mut 100_000)
            .unwrap();
    assert_eq!(
        recovered
            .result_targets
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .parameter,
        0
    );
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(vm.invoke(id, vec![]).unwrap(), Value::Bool(true));
}

#[test]
fn ordinary_value_indexed_methods_preserve_call_contracts() {
    let source = "struct Limit where maximum:.u8'builtin {
        pub fn keep(value:.u8'builtin)->result:.u8'builtin
            where requires value <= maximum, ensures result <= maximum {value}
    }
    fn run()->bool {Limit<10>.keep(9) == 9 and Limit<20>.keep(19) == 19}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Limit<10>.keep(9)", "Limit<10>.keep(11)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn symbolic_value_contracts_forward_through_generic_calls_and_branches() {
    let source = "struct Limit where maximum:.u8'builtin {
        pub fn keep(value:.u8'builtin)->result:.u8'builtin
            where requires value <= maximum, ensures result <= maximum {value}
        pub fn forward(value:.u8'builtin,flag:bool)->result:.u8'builtin
            where requires value <= maximum, ensures result <= maximum {
            if flag {Self.keep(value)} else {value}
        }
    }
    fn run()->bool {Limit<10>.forward(9,true) == 9 and Limit<20>.forward(19,false) == 19}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("Self.keep(value)", "Limit<0>.keep(value)"),
        source.replace("else {value}", "else {255}"),
        source.replace("forward(19,false)", "forward(21,false)"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn symbolic_value_contracts_distinguish_same_domain_captures() {
    let source = "struct Limits where left:.u8'builtin,right:.u8'builtin {
        pub fn keep(value:.u8'builtin)->result:.u8'builtin
            where requires value <= left and value <= right, ensures result <= right {value}
    }
    fn run()->bool {Limits<10,20>.keep(9) == 9 and Limits<20,10>.keep(9) == 9}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("value <= left and value <= right", "value <= left");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn symbolic_boolean_capture_contracts_keep_program_facts() {
    let source = "struct Flag where enabled:bool {
        pub fn keep(value:bool)->result:bool
            where requires value == enabled, ensures result == enabled {value}
    }
    fn run()->bool {Flag<true>.keep(true) and not Flag<false>.keep(false)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("Flag<false>.keep(false)", "Flag<false>.keep(true)"),
        source.replace("{value}", "{not value}"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn value_indexed_enum_methods_construct_and_return_self() {
    let source = "enum Choice where enabled:bool {
        none,some(bool),
        pub fn make()->Self {Self.some(enabled)}
        pub fn forward()->Self {Self.make()}
    }
    fn run()->bool {
        let .some(first)=Choice<true>.forward() else {return false;};
        let .some(second)=Choice<false>.forward() else {return false;};
        first and not second
    }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn value_indexed_enum_payloads_use_generic_call_result_context() {
    let source = "enum Choice where enabled:bool {none,some(bool)}
        fn keep(value:T)->T where T {value}
        fn make()->Choice<true> {keep(.some(true))}
        fn run()->bool {let .some(value)=make() else {return false;};value}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn mixed_enum_indices_infer_payload_types_in_generic_arguments() {
    let source = "enum Choice where T,enabled:bool {none,some(T)}
        fn read(value:Choice<T,true>,fallback:T)->T where T {
            value match {.none=>fallback,.some(payload)=>payload}
        }
        fn run()->bool {read(.some(true),false)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn mixed_enum_indices_infer_without_a_completed_sibling_argument() {
    let source = "enum Choice where T,enabled:bool {none,some(T)}
        fn keep(value:Choice<T,true>)->Choice<T,true> where T {value}
        fn run()->bool {let stored=keep(.some(true));let .some(value)=stored else {return false;};value}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn mixed_enum_contexts_preserve_record_defaults_and_spread_payloads() {
    for payload in [".some(...(true,))", ".some(true)"] {
        let source = format!("enum Choice where T,enabled:bool {{none,some(T)}}
            fn keep(value:Choice<T,true>)->Choice<T,true> where T {{value}}
            fn run()->bool {{let stored=keep({payload});let .some(value)=stored else {{return false;}};value}}");
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
    let source = "enum Choice where T,enabled:bool {none,some {value:T,flag:bool=enabled}}
        fn keep(value:Choice<T,false>)->Choice<T,false> where T {value}
        fn run()->bool {let stored=keep(.some {value:true});let .some {value,flag}=stored else {return false;};value and not flag}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "keep(.some {value:true})",
        "keep(Choice<bool,true>.some {value:true})",
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn completed_enum_record_defaults_preserve_value_parameters() {
    let source = "enum Choice where T,enabled:bool {none,some {value:T,flag:bool=enabled}}
        fn run()->bool {
            let first=Choice<bool,true>.some {value:true};
            let second:Choice<bool,false> =.some {value:true};
            let .some {value:a,flag:b}=first else {return false;};
            let .some {value:c,flag:d}=second else {return false;};
            a and b and c and not d
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn dependent_enum_record_defaults_specialize_and_allow_overrides() {
    let source = "enum Entry where T,seed:T {data {value:T=seed}}
        fn run()->bool {
            let a=Entry<bool,true>.data {};
            let b:Entry<.u8'builtin,7> = .data {};
            let c=Entry<bool,true>.data {value:false};
            let .data {value:x}=a;
            let .data {value:y}=b;
            let .data {value:z}=c;
            x and y == 7 and not z
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("Entry<bool,true>", "Entry<bool,'x'>"),
        source.replace("value:false", "value:'x'"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn nested_alias_self_preserves_parent_value_indices() {
    let source = "struct Holder where enabled:bool {
        value:bool,
        pub typealias This = Self;
        pub typealias Selected<.unused:bool=false> = Self;
    }
    fn read(value:Holder<true>)->bool {value.value}
    fn run()->bool {
        let a:Holder<true>.This=Holder<true>{value:true};
        let b:Holder<true>.Selected<> =Holder<true>{value:true};
        read(a) and read(b)
    }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("let a:Holder<true>.This", "let a:Holder<false>.This"),
        source.replace(
            "let b:Holder<true>.Selected",
            "let b:Holder<false>.Selected",
        ),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn method_type_defaults_preserve_preceding_value_captures() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where enabled:bool {
            pub fn ready()->bool where .U:Type=Self {enabled}
            pub fn keep(value:U)->U where .U:Type=Self {value}
        }
        fn run()->bool {
            let kept=Holder<true>.keep(Holder<true>{});
            Holder<true>.ready() and not Holder<false>.ready()
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("keep(Holder<true>{})", "keep(Holder<false>{})");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn deferred_method_type_defaults_combine_inferred_types_and_value_captures() {
    let source = "typealias Type=.Type'builtin;
        struct Packet where T,enabled:bool {value:T}
        struct Holder where enabled:bool {
            pub fn ready(value:T)->bool where T,.U:Type=Packet<T,enabled> {enabled}
        }
        fn run()->bool {Holder<true>.ready(true) and not Holder<false>.ready('x')}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_type_defaults_follow_value_parameters() {
    let source = "typealias Type=.Type'builtin;
        trait Keep where enabled:bool,.T:Type=bool {fn keep(value:T)->T {value}}
        struct Holder {}
        impl Keep<true> for Holder {}
        impl Keep<false> for Holder {}
        fn run()->bool {
            Holder.as(Keep<true>).keep(true) and not Holder.as(Keep<false>).keep(false)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn value_indexed_methods_as_function_values_keep_captures_and_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where enabled:bool {
            pub fn read()->bool {enabled}
            pub fn keep(value:U)->U where .U:Type=Self {value}
        }
        fn invoke(f:fn()->bool)->bool {f()}
        fn run()->bool {
            let yes=Holder<true>.read;
            let no=Holder<false>.read;
            let keep:fn(Holder<true>)->Holder<true> = Holder<true>.keep;
            let kept=keep(Holder<true>{});
            invoke(yes) and not invoke(no)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn value_indexed_trait_associated_type_defaults_preserve_indices() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where enabled:bool {value:bool}
        trait Shape where enabled:bool {assoc Output:Type=Tagged<enabled>;}
        struct Holder {}
        impl Shape<true> for Holder {}
        impl Shape<false> for Holder {}
        fn run()->bool {
            let a:Holder.as(Shape<true>).Output=Tagged<true>{value:true};
            let b:Holder.as(Shape<false>).Output=Tagged<false>{value:false};
            a.value and not b.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Output=Tagged<false>", "Output=Tagged<true>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    let inherited=source.replace("struct Holder {}", "trait Child :- Shape<enabled> where enabled:bool {} struct Holder {} impl Child<true> for Holder {} impl Child<false> for Holder {}")
        .replace(".as(Shape<", ".as(Child<");
    assert_eq!(
        support::run_iir(&inherited, "run", vec![]),
        Value::Bool(true)
    );
    let signatures=source.replace("assoc Output:Type=Tagged<enabled>;", "assoc Output:Type=Tagged<enabled>; fn keep(value:Self.as(Shape<enabled>).Output)->Self.as(Shape<enabled>).Output {value}")
        .replace("a.value and not b.value", "Holder.as(Shape<true>).keep(a).value and not Holder.as(Shape<false>).keep(b).value");
    assert_eq!(
        support::run_iir(&signatures, "run", vec![]),
        Value::Bool(true)
    );
    let chained = source.replace(
        "assoc Output:Type=Tagged<enabled>;",
        "assoc Base:Type=Tagged<enabled>; assoc Output:Type=Self.as(Shape<enabled>).Base;",
    );
    assert_eq!(support::run_iir(&chained, "run", vec![]), Value::Bool(true));
}

#[test]
fn value_indexed_trait_associated_constants_keep_selected_instance() {
    let source = "trait Flags where enabled:bool {
            assoc flag:bool=enabled;
            assoc copy:bool=Self.as(Flags<enabled>).flag;
        }
        struct Holder {}
        impl Flags<true> for Holder {}
        impl Flags<false> for Holder {}
        fn run()->bool {
            Holder.as(Flags<true>).copy and not Holder.as(Flags<false>).copy
                and Holder.as(Flags<true>).flag
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let inherited=source.replace("struct Holder {}", "trait Child :- Flags<enabled> where enabled:bool {} struct Holder {} impl Child<true> for Holder {} impl Child<false> for Holder {}")
        .replace("Holder.as(Flags<", "Holder.as(Child<");
    assert_eq!(
        support::run_iir(&inherited, "run", vec![]),
        Value::Bool(true)
    );
    let overridden = source
        .replace(
            "impl Flags<false> for Holder {}",
            "impl Flags<false> for Holder {assoc flag:bool=true;}",
        )
        .replace(
            "not Holder.as(Flags<false>).copy",
            "Holder.as(Flags<false>).copy",
        );
    assert_eq!(
        support::run_iir(&overridden, "run", vec![]),
        Value::Bool(true)
    );
    let invalid = source.replace("assoc flag:bool=enabled", "assoc flag:bool='x'");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn dependent_trait_associated_constants_preserve_value_domains() {
    let source="trait Value where T,value:T {assoc data:T=value;}
        struct Holder {}
        impl Value<bool,true> for Holder {}
        impl Value<.u8'builtin,7> for Holder {}
        fn run()->bool {Holder.as(Value<bool,true>).data and Holder.as(Value<.u8'builtin,7>).data == 7}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_value_indexed_implementations_specialize_associated_constants() {
    let source = "trait Flags where enabled:bool {assoc flag:bool=enabled;}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {Holder<true>.as(Flags<true>).flag and not Holder<false>.as(Flags<false>).flag}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_associated_data_values_keep_dependent_domains_and_overrides() {
    let source = "trait Value where T,value:T {assoc data:T=value;}
        struct Holder where T,value:T {}
        impl Value<T,value> for Holder<T,value> where T,value:T {}
        fn run()->bool {
            Holder<bool,true>.as(Value<bool,true>).data
                and Holder<.u8'builtin,7>.as(Value<.u8'builtin,7>).data == 7
                and not Holder<bool,false>.as(Value<bool,false>).data
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let overridden = source.replace(
        "where T,value:T {}\n        fn run",
        "where T,value:T {assoc data:T=value;}\n        fn run",
    );
    assert_ne!(source, overridden);
    assert_eq!(
        support::run_iir(&overridden, "run", vec![]),
        Value::Bool(true)
    );
    let invalid = source.replace("Holder<.u8'builtin,7>", "Holder<.u8'builtin,true>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generic_associated_comptime_closures_capture_value_indices() {
    let source="comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc callback:comptime fn()->bool=|| invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn run()->bool {not Holder<true>.as(Flags<true>).callback() and Holder<false>.as(Flags<false>).callback()}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_associated_initializers_invoke_after_instance_selection() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc flag:bool=invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {not Holder<true>.as(Flags<true>).flag and Holder<false>.as(Flags<false>).flag}";
    let package = support::compile(source);
    let mut runtime = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        runtime
            .invoke(support::id(&package, "run"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::UnmaterializedDispatch
    );
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_associated_initializers_freeze_dependencies_and_check_contracts() {
    let source = "comptime fn checked(value:bool)->bool where requires value {value}
        trait Flags where enabled:bool {
            assoc flag:bool=checked(enabled);
            assoc copy:bool=if enabled {Self.as(Flags<enabled>).flag} else {false};
        }
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {Holder<true>.as(Flags<true>).copy and not Holder<false>.as(Flags<false>).copy}";
    let package = support::compile(source);
    assert!(
        package
            .implementations
            .iter()
            .any(|item| !item.associated_initializers.is_empty())
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(
        graph
            .dispatch
            .iter()
            .all(|instance| instance.associated_initializers.is_empty())
    );
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace(
        "not Holder<false>.as(Flags<false>).copy",
        "not Holder<false>.as(Flags<false>).flag",
    );
    let invalid = support::compile(&invalid);
    assert!(luna::instantiation::instantiate(&invalid, "run", 100_000).is_err());
}

#[test]
fn generic_associated_bounds_preserve_method_contract_instances() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn keep(value:Byte)->Byte {value}
        trait Bounds where limit:Byte {assoc bound:Byte=keep(limit);}
        struct Holder where limit:Byte {
            pub comptime fn checked(value:Byte)->result:Byte
                where requires Self :- Bounds<limit>, requires value < Self.as(Bounds<limit>).bound,
                ensures result < Self.as(Bounds<limit>).bound {value}
        }
        impl Bounds<limit> for Holder<limit> where limit:Byte {}
        fn decide(comptime value:Byte)->Byte {value}
        fn run()->Byte {decide(do {
            let value:Byte=8;
            assumes value < Holder<9>.as(Bounds<9>).bound;
            let first=Holder<9>.checked(value);
            Holder<9>.checked(first)
        })}";
    assert_eq!(
        support::run_iir(source, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    let inferred = source.replace("requires Self :- Bounds<limit>, ", "");
    assert_eq!(
        support::run_iir(&inferred, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    let mixed = source
        .replace(
            "struct Holder where limit:Byte",
            "struct Holder where T,limit:Byte",
        )
        .replace(
            "for Holder<limit> where limit:Byte",
            "for Holder<T,limit> where T,limit:Byte",
        )
        .replace("Holder<9>", "Holder<bool,9>");
    assert_eq!(
        support::run_iir(&mixed, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    let inferred_mixed = mixed.replace("requires Self :- Bounds<limit>, ", "");
    assert_eq!(
        support::run_iir(&inferred_mixed, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    let implementation = "impl Bounds<limit> for Holder<limit> where limit:Byte {}";
    let reordered = inferred.replace(implementation, "").replace(
        "struct Holder where limit:Byte",
        &format!("{implementation} struct Holder where limit:Byte"),
    );
    assert_eq!(
        support::run_iir(&reordered, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    // The unused method contract itself must be rejected, independently of calls.
    let declaration_only = inferred
        .split("fn decide")
        .next()
        .unwrap()
        .replace(implementation, "");
    let error =
        support::compile_with_protocols(&declaration_only, &Default::default()).unwrap_err();
    assert!(error.contains("program contract associated projection requires an established interface implementation"), "{error}");
    let wrong_instance = source.replace("Holder<9>.checked(first)", "Holder<10>.checked(first)");
    assert!(support::compile_with_protocols(&wrong_instance, &Default::default()).is_err());
}

#[test]
fn associated_bounds_preserve_polynomial_contracts() {
    for (bound, factor, power) in [(10, 15, "value * value"), (27, 6, "value * value * value")] {
        let result_power = power.replace("value", "result");
        let source = format!("typealias Byte=.u8'builtin;
            comptime fn keep(value:Byte)->Byte {{value}}
            trait Bounds where limit:Byte {{assoc bound:Byte=keep(limit);}}
            struct Holder where limit:Byte {{}}
            impl Bounds<limit> for Holder<limit> where limit:Byte {{}}
            comptime fn checked(value:Byte)->result:Byte
                where requires value <= {factor} and {power} < Holder<{bound}>.as(Bounds<{bound}>).bound,
                ensures result <= {factor} and {result_power} < Holder<{bound}>.as(Bounds<{bound}>).bound {{value}}
            fn decide(comptime value:Byte)->Byte {{value}}
            fn run()->Byte {{decide(do {{
                let value:Byte=2;
                assumes value <= {factor} and {power} < Holder<{bound}>.as(Bounds<{bound}>).bound;
                let first=checked(value);checked(first)
            }})}}");
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::UnsignedInteger(2)
        );
        let invalid = source.replace("let value:Byte=2;", "let value:Byte=4;");
        let error = match support::compile_with_protocols(&invalid, &Default::default()) {
            Ok(package) => luna::instantiation::instantiate(&package, "run", 100_000).unwrap_err(),
            Err(error) => error,
        };
        assert!(error.contains("ProgramContractRejected"), "{error}");
    }
}

#[test]
fn associated_integer_bounds_transfer_into_comptime_call_contracts() {
    let source = "typealias Byte=.u8'builtin;
        comptime fn keep(value:Byte)->Byte {value}
        trait Limits where maximum:Byte {assoc bound:Byte=keep(maximum);}
        struct Holder where maximum:Byte {}
        impl Limits<maximum> for Holder<maximum> where maximum:Byte {}
        comptime fn checked(value:Byte)->Byte
            where requires value < Holder<9>.as(Limits<9>).bound {value}
        fn decide(comptime value:Byte)->Byte {value}
        fn run()->Byte {decide(do {
            let value:Byte=8;
            assumes value < Holder<9>.as(Limits<9>).bound;
            checked(value)
        })}";
    assert_eq!(
        support::run_iir(source, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    let forwarded = source
        .replace(
            "checked(value:Byte)->Byte",
            "checked(value:Byte)->result:Byte",
        )
        .replace(
            ".bound {value}",
            ".bound, ensures result < Holder<9>.as(Limits<9>).bound {value}",
        )
        .replace(
            "checked(value)\n",
            "let result=checked(value);checked(result)\n",
        );
    assert_eq!(
        support::run_iir(&forwarded, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    let produced = source.replace("fn decide(comptime value:Byte)->Byte {value}",
        "comptime fn produce()->result:Byte where ensures result < Holder<9>.as(Limits<9>).bound {let value:Byte=8;assumes value < Holder<9>.as(Limits<9>).bound;value}
         fn decide(comptime value:Byte)->Byte {value}")
        .replace("let value:Byte=8;\n            assumes value < Holder<9>.as(Limits<9>).bound;\n            checked(value)", "checked(produce())");
    assert_eq!(
        support::run_iir(&produced, "run", vec![]),
        Value::UnsignedInteger(8)
    );
    let rejected_producer = produced.replace("let value:Byte=8;", "let value:Byte=9;");
    let error =
        support::compile_with_protocols(&rejected_producer, &Default::default()).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
    for invalid in [
        source
            .replace("let value:Byte=8;", "var value:Byte=8;")
            .replace("checked(value)\n", "value=9;checked(value)\n"),
        source.replace(
            "assumes value < Holder<9>.as(Limits<9>).bound",
            "assumes value < Holder<10>.as(Limits<10>).bound",
        ),
    ] {
        let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(
            error.contains("program precondition is not proven"),
            "{error}"
        );
    }
}

#[test]
fn required_invocations_check_independent_associated_preconditions() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc flag:bool=invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn checked()->bool where requires Holder<false>.as(Flags<false>).flag {true}
        fn decide(comptime value:bool)->bool {value}
        fn run()->bool {decide(do {assumes Holder<false>.as(Flags<false>).flag;checked()})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for condition in [
        "true and Holder<false>.as(Flags<false>).flag",
        "false or Holder<false>.as(Flags<false>).flag",
    ] {
        let source = source.replace(
            "assumes Holder<false>.as(Flags<false>).flag",
            &format!("assumes {condition}"),
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
    let missing = source.replace("assumes Holder<false>.as(Flags<false>).flag;", "");
    let wrong_argument = source.replace(
        "assumes Holder<false>.as(Flags<false>).flag",
        "assumes Holder<true>.as(Flags<true>).flag",
    );
    let wrong_member = source
        .replace(
            "assoc flag:bool=invert(enabled);",
            "assoc flag:bool=invert(enabled);assoc other:bool=invert(enabled);",
        )
        .replace(
            "assumes Holder<false>.as(Flags<false>).flag",
            "assumes Holder<false>.as(Flags<false>).other",
        );
    let wrong_subject = source.replace("struct Holder where enabled:bool {}", "struct Holder where enabled:bool {} struct Second where enabled:bool {} impl Flags<enabled> for Second<enabled> where enabled:bool {}")
        .replace("assumes Holder<false>.as(Flags<false>).flag", "assumes Second<false>.as(Flags<false>).flag");
    for invalid in [missing, wrong_argument, wrong_member, wrong_subject] {
        let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(
            error.contains("program precondition is not proven"),
            "{error}\n{invalid}"
        );
    }
    let rejected = source
        .replace("Holder<false>", "Holder<true>")
        .replace("Flags<false>", "Flags<true>");
    let error = support::compile_with_protocols(&rejected, &Default::default()).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn associated_constants_can_form_entire_program_preconditions() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc flag:bool=invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn checked()->bool where requires Holder<false>.as(Flags<false>).flag {true}";
    let condition = "Holder<false>.as(Flags<false>).flag";
    for predicate in [
        condition.to_owned(),
        format!("not not {condition}"),
        format!("{condition} == true"),
        format!("{condition} != false"),
        format!("do {{let selected={condition};selected}}"),
        format!("if true {{{condition}}} else {{false}}"),
    ] {
        for accepted in [true, false] {
            let mut source = source.replace(condition, &predicate);
            if !accepted {
                source = source
                    .replace("Holder<false>", "Holder<true>")
                    .replace("Flags<false>", "Flags<true>");
            }
            let package = support::compile_with_protocols(&source, &Default::default())
                .unwrap_or_else(|error| panic!("{error}\n{source}"));
            let checked = support::id(&package, "checked");
            let function = package
                .functions
                .iter()
                .find(|function| function.declaration == checked)
                .unwrap();
            assert!(
                function.requirements.is_empty(),
                "program condition must not enter PTC"
            );
            assert_eq!(function.preconditions.len(), 1);
            let mut vm =
                comptime_interpretor::Interpreter::for_comptime(&package, Default::default())
                    .unwrap();
            let result = vm.invoke(checked, vec![]);
            if accepted {
                assert_eq!(result.unwrap(), Value::Bool(true));
            } else {
                let error = result.unwrap_err();
                assert!(
                    format!("{error:?}").contains("ProgramContractRejected"),
                    "{error:?}"
                );
            }
        }
    }
}

#[test]
fn required_invocations_collect_associated_contract_dependencies() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc flag:bool=invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn checked(value:bool)->bool where requires value or Holder<false>.as(Flags<false>).flag {true}
        fn decide(comptime value:bool)->bool {value}
        fn run()->bool {decide(checked(true))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let postcondition = source.replace(
        "checked(value:bool)->bool where requires value or",
        "checked(value:bool)->result:bool where ensures result or",
    );
    assert_eq!(
        support::run_iir(&postcondition, "run", vec![]),
        Value::Bool(true)
    );
    for (source, accepted) in [
        (source.to_owned(), true),
        (
            source
                .replace("Holder<false>", "Holder<true>")
                .replace("Flags<false>", "Flags<true>"),
            false,
        ),
    ] {
        let package = support::compile(&source);
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let checked = support::id(&package, "checked");
        for _ in 0..2 {
            let result = vm.invoke(checked, vec![Value::Bool(false)]);
            if accepted {
                assert_eq!(result.unwrap(), Value::Bool(true));
            } else {
                let error = result.unwrap_err();
                assert!(
                    format!("{error:?}").contains("ProgramContractRejected"),
                    "{error:?}"
                );
            }
            assert_eq!(
                vm.invoke(checked, vec![Value::Bool(true)]).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn required_invocations_collect_default_function_dependencies() {
    let source = "comptime fn default_value()->bool {true}
        comptime fn checked(.value:bool=default_value())->bool {value}
        comptime fn read()->bool {checked(.value=true)}
        fn decide(comptime value:bool)->bool {value}
        fn run()->bool {decide(read())}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let used = source.replace("checked(.value=true)", "checked()");
    assert_eq!(support::run_iir(&used, "run", vec![]), Value::Bool(true));
    let failing_default = source.replace(
        "default_value()->bool {true}",
        "default_value()->bool {assumes false;true}",
    );
    assert_eq!(
        support::run_iir(&failing_default, "run", vec![]),
        Value::Bool(true)
    );
    let used_failure = failing_default.replace("checked(.value=true)", "checked()");
    let error = support::compile_with_protocols(&used_failure, &Default::default()).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn required_associated_initializers_collect_transitive_function_dependencies() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc flag:bool=invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn read()->bool {Holder<false>.as(Flags<false>).flag}
        fn decide(comptime value:bool)->bool {value}
        fn run()->bool {decide(read())}";
    let nested = source
        .replace(
            "comptime fn invert(value:bool)->bool {not value}",
            "comptime fn invert(value:bool)->bool {not value}
         trait Inner where flag:bool {assoc result:bool=invert(flag);}
         struct Other where flag:bool {}
         impl Inner<flag> for Other<flag> where flag:bool {}",
        )
        .replace(
            "assoc flag:bool=invert(enabled)",
            "assoc flag:bool=Other<enabled>.as(Inner<enabled>).result",
        );
    let closure = source
        .replace(
            "comptime fn invert(value:bool)->bool {not value}",
            "typealias Callback=%{flag:bool} comptime fn()->bool;
         comptime fn invert(value:bool)->bool {not value}
         comptime fn make(flag:bool)->Callback {|| invert(flag)}",
        )
        .replace(
            "assoc flag:bool=invert(enabled)",
            "assoc flag:Callback=make(enabled)",
        )
        .replace("Flags<false>).flag}", "Flags<false>).flag()}");
    let checked = source.replace("{not value}", "{assumes not value;not value}");
    let rejected = checked
        .replace("Holder<false>", "Holder<true>")
        .replace("Flags<false>", "Flags<true>");
    let error = support::compile_with_protocols(&rejected, &Default::default()).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
    for source in [source.to_owned(), nested, closure, checked] {
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        assert!(
            !support::calls(&package, &vm)
                .iter()
                .any(|name| matches!(name.as_str(), "read" | "invert" | "nested" | "make"))
        );
    }
}

#[test]
fn generic_associated_initializers_execute_only_in_comptime_interpreter() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc flag:bool=invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn run()->bool {Holder<false>.as(Flags<false>).flag}";
    let package = support::compile(source);
    let mut forged = package.clone();
    let (_, initializer) = forged
        .dispatch_instances
        .iter_mut()
        .flat_map(|instance| &mut instance.associated_initializers)
        .next()
        .unwrap();
    let mut value = initializer.expression().clone();
    value.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    *initializer =
        nir::associated_initializers::AssociatedInitializer::new(value, &mut 1000).unwrap();
    assert!(comptime_interpretor::Interpreter::for_comptime(&forged, Default::default()).is_err());
    let mut duplicate = package.clone();
    let instance = duplicate
        .dispatch_instances
        .iter_mut()
        .find(|instance| !instance.associated_initializers.is_empty())
        .unwrap();
    instance
        .associated_initializers
        .push(instance.associated_initializers[0].clone());
    assert!(
        comptime_interpretor::Interpreter::for_comptime(&duplicate, Default::default()).is_err()
    );
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_associated_initializers_recover_after_failed_comptime_contract() {
    let source = "comptime fn checked(value:bool)->bool where requires value {value}
        trait Flags where enabled:bool {assoc flag:bool=checked(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn fail()->bool {Holder<false>.as(Flags<false>).flag}
        comptime fn run()->bool {Holder<true>.as(Flags<true>).flag}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert!(vm.invoke(support::id(&package, "fail"), vec![]).is_err());
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_associated_initializers_freeze_aggregate_results_and_overrides() {
    let source="struct Packet {flag:bool}
        comptime fn make(flag:bool)->Packet {Packet {flag:not flag}}
        trait Flags where enabled:bool {assoc packet:Packet=make(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {not Holder<true>.as(Flags<true>).packet.flag and Holder<false>.as(Flags<false>).packet.flag}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let overridden = source.replace(
        "where enabled:bool {}\n        fn run",
        "where enabled:bool {assoc packet:Packet=make(enabled);}\n        fn run",
    );
    assert_ne!(source, overridden);
    assert_eq!(
        support::run_iir(&overridden, "run", vec![]),
        Value::Bool(true)
    );
    let package = support::compile(source);
    assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
    assert!(luna::instantiation::instantiate(&package, "run", 100_000).is_ok());
}

#[test]
fn cyclic_associated_initializer_dispatch_is_rejected_without_recursion() {
    let source = "comptime fn invert(value:bool)->bool {not value}
        trait Flags where enabled:bool {assoc flag:bool=invert(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        comptime fn run()->bool {Holder<false>.as(Flags<false>).flag}";
    let mut package = support::compile(source);
    let implementation = package
        .implementations
        .iter_mut()
        .find(|item| !item.associated_initializers.is_empty())
        .unwrap();
    let member = implementation.associated_initializers[0].0;
    let original = implementation.associated_initializers[0].1.expression();
    let recursive = nir::program::Expr {
        ty: original.ty.clone(),
        source: original.source,
        kind: nir::program::ExprKind::AssociatedValue {
            interface: implementation.interface_instance().unwrap(),
            subject: implementation.subject.clone(),
            member,
        },
    };
    implementation.associated_initializers[0].1 =
        nir::associated_initializers::AssociatedInitializer::new(recursive, &mut 1000).unwrap();
    let binders = implementation.parameter_binders().unwrap();
    for instance in &mut package.dispatch_instances {
        if instance.implementation == implementation.declaration {
            instance.associated_initializers = vec![(
                member,
                implementation.associated_initializers[0]
                    .1
                    .instantiate(&binders, &instance.arguments, &mut 1000)
                    .unwrap(),
            )];
        }
    }
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    for _ in 0..2 {
        assert!(vm.invoke(support::id(&package, "run"), vec![]).is_err());
    }
}

#[test]
fn generic_associated_initializers_freeze_selected_runtime_functions() {
    let source="fn invert(value:bool)->bool {not value}
        fn keep(value:bool)->bool {value}
        comptime fn select(enabled:bool)->fn(bool)->bool {if enabled {invert} else {keep}}
        trait Flags where enabled:bool {assoc callback:fn(bool)->bool=select(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {not Holder<true>.as(Flags<true>).callback(true) and Holder<false>.as(Flags<false>).callback(true)}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), ["run", "invert", "keep"]);
}

#[test]
fn generic_associated_initializers_preserve_generic_function_instances() {
    let source="fn keep(value:T)->T where T {value}
        fn invert(value:bool)->bool {not value}
        comptime fn select(enabled:bool)->fn(bool)->bool {if enabled {keep} else {invert}}
        trait Flags where enabled:bool {assoc callback:fn(bool)->bool=select(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {Holder<true>.as(Flags<true>).callback(true) and not Holder<false>.as(Flags<false>).callback(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_associated_initializers_preserve_runtime_trait_method_identity() {
    let source="trait Strategy where inverted:bool {fn apply(value:bool)->bool;}
        struct Worker {}
        impl Strategy<true> for Worker {fn apply(value:bool)->bool {not value}}
        impl Strategy<false> for Worker {fn apply(value:bool)->bool {value}}
        comptime fn select(enabled:bool)->fn(bool)->bool {
            if enabled {Worker.as(Strategy<true>).apply} else {Worker.as(Strategy<false>).apply}
        }
        trait Flags where enabled:bool {assoc callback:fn(bool)->bool=select(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {not Holder<true>.as(Flags<true>).callback(true) and Holder<false>.as(Flags<false>).callback(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn frozen_runtime_trait_methods_do_not_erase_interface_preconditions() {
    let source = "trait Strategy {fn apply(value:bool)->bool where requires value;}
        struct Worker {}
        impl Strategy for Worker {fn apply(value:bool)->bool {value}}
        comptime fn select(enabled:bool)->fn(bool)->bool {Worker.as(Strategy).apply}
        trait Flags where enabled:bool {assoc callback:fn(bool)->bool=select(enabled);}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {Holder<true>.as(Flags<true>).callback(false)}";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(error.contains("erase program contracts"), "{error}");
}

#[test]
fn value_indexed_methods_preserve_ordinary_type_input_requirements() {
    let source = "typealias Type=.Type'builtin;
        struct Holder where enabled:bool {
            pub fn keep(comptime T:Type,value:T)->T where requires T == bool {value}
        }
        fn run()->bool {Holder<true>.keep(bool,true) and not Holder<false>.keep(bool,false)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("keep(bool,false)", "keep('x''type,'x')");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn staged_value_indexed_methods_forward_type_requirements_and_index_identity() {
    let source="typealias Type=.Type'builtin;
        struct Tagged where enabled:bool {flag:bool}
        struct Holder where enabled:bool {
            pub fn keep(comptime T:Type,value:T)->T where requires T == Tagged<enabled> {value}
            pub fn forward(comptime T:Type,value:T)->T where requires T == Tagged<enabled> {Self.keep(T,value)}
        }
        fn run()->bool {
            let a=Holder<true>.forward(Tagged<true>,Tagged<true>{flag:true});
            let b=Holder<false>.forward(Tagged<false>,Tagged<false>{flag:false});
            a.flag and not b.flag
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Holder<false>.forward", "Holder<true>.forward");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn staged_trait_methods_preserve_captured_value_domains_and_requirements() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where enabled:bool {flag:bool}
        trait Keep where enabled:bool {
            fn keep(comptime T:Type,value:T)->T where requires T == Tagged<enabled> {value}
        }
        struct Holder where enabled:bool {}
        impl Keep<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {
            let a=Holder<true>.as(Keep<true>).keep(Tagged<true>,Tagged<true>{flag:true});
            let b=Holder<false>.as(Keep<false>).keep(Tagged<false>,Tagged<false>{flag:false});
            a.flag and not b.flag
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(
        "keep(Tagged<false>,Tagged<false>{flag:false})",
        "keep(Tagged<true>,Tagged<true>{flag:false})",
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn staged_calls_inline_value_indexed_helpers_without_changing_captures() {
    let source="typealias Type=.Type'builtin;
        struct Holder where enabled:bool {
            pub inline fn choose(value:bool)->bool {if enabled {value} else {not value}}
            pub fn forward(comptime T:Type,value:T)->bool where requires T == bool {Self.choose(value)}
        }
        fn provide()->bool {true}
        fn run()->bool {Holder<true>.forward(bool,provide()) and not Holder<false>.forward(bool,provide())}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| name.as_str() == "provide")
            .count(),
        2
    );
}

#[test]
fn staged_trait_calls_inline_helpers_with_selected_value_parameters() {
    let source="typealias Type=.Type'builtin;
        trait Select where enabled:bool {
            inline fn choose(value:bool)->bool {if enabled {value} else {not value}}
            fn forward(comptime T:Type,value:T)->bool where requires T == bool {Self.as(Select<enabled>).choose(value)}
        }
        struct Holder where enabled:bool {}
        impl Select<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {Holder<true>.as(Select<true>).forward(bool,true) and Holder<false>.as(Select<false>).forward(bool,false)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn mixed_normal_form_requirements_survive_iir_materialization() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn Tagged<T,enabled:bool>->Type where requires T == bool {T}
        fn accept(value:*Tagged<bool,true>)->bool {true}
        fn run()->fn(*Tagged<bool,true>)->bool {accept}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let accept = graph
        .functions
        .iter()
        .find(|instance| instance.function.name == "accept")
        .unwrap();
    let nir::Term::Function { parameters, .. } = &accept.function.signature else {
        panic!("expected callable")
    };
    let nir::Term::Pointer(inner) = &parameters[0].ty else {
        panic!("expected pointer")
    };
    assert_eq!(
        inner.as_ref(),
        &nir::Term::NormalForm {
            head: package.normal_forms[0].declaration,
            sort: nir::Sort::Type,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Bool(true)
            ],
        }
    );
    assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
    assert!(luna::instantiation::instantiate(&package, "run", 100_000).is_ok());
    let invalid = source.replace("Tagged<bool,true>", "Tagged<'x''type,true>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn normal_form_requirements_compare_types_containing_value_indices() {
    let source = "typealias Type=.Type'builtin;
        struct Index where enabled:bool {}
        pure comptime fn Checked<enabled:bool>->Type
            where requires Index<enabled> == Index<true> {bool}
        fn accept(value:*Checked<true>)->bool {true}
        fn run()->fn(*Checked<true>)->bool {accept}";
    let package = support::compile(source);
    assert!(luna::instantiation::instantiate(&package, "run", 100_000).is_ok());
    fn change_index(ty: &mut nir::Term, head: nir::DeclarationId) {
        match ty {
            nir::Term::Function { parameters, result } => {
                for parameter in parameters {
                    change_index(&mut parameter.ty, head);
                }
                change_index(result, head);
            }
            nir::Term::Pointer(inner) => change_index(inner, head),
            nir::Term::NormalForm {
                head: actual,
                arguments,
                ..
            } if *actual == head => arguments[0] = nir::Term::Bool(false),
            _ => {}
        }
    }
    let mut forged = package.clone();
    let head = forged.normal_forms[0].declaration;
    for function in &mut forged.functions {
        change_index(&mut function.signature, head);
        function
            .body
            .rewrite_types(&mut |ty| {
                let mut ty = ty.clone();
                change_index(&mut ty, head);
                Ok::<_, String>(ty)
            })
            .unwrap();
    }
    let error = luna::instantiation::instantiate(&forged, "run", 100_000).unwrap_err();
    assert!(
        error.contains("formation requirement is not established"),
        "{error}"
    );
    let invalid = source.replace("Checked<true>", "Checked<false>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generic_associated_initializers_freeze_anonymous_comptime_functions() {
    let source = "comptime fn select(enabled:bool)->comptime fn(bool)->bool {
            if enabled {|value| not value} else {|value| value}
        }
        trait Flags where enabled:bool {assoc callback:comptime fn(bool)->bool=select(enabled); fn get()->bool {decide(Self.as(Flags<enabled>).callback(true))}}
        fn decide(comptime flag:bool)->bool {flag}
        struct Holder where enabled:bool {}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {not Holder<true>.as(Flags<true>).get() and Holder<false>.as(Flags<false>).get()}";
    for source in [
        source.to_owned(),
        format!("comptime fn invert(value:bool)->bool {{not value}} {source}")
            .replace("|value| not value", "|value| invert(value)"),
    ] {
        let package = support::compile(&source);
        assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        assert!(
            !support::calls(&package, &vm)
                .iter()
                .any(|name| name == "select" || name == "invert")
        );
    }
}

#[test]
fn nominal_value_domains_survive_staged_capability_queries() {
    let source = "comptime fn select(enabled:bool)->comptime fn(bool)->bool {
        if enabled {|value| not value} else {|value| value}
    }
    trait Flags where enabled:bool {assoc callback:comptime fn(bool)->bool=select(enabled);}
    fn decide(comptime flag:bool)->bool {flag}
    struct Holder where enabled:bool {
        pub fn get()->bool {decide(Holder<enabled>.as(Flags<enabled>).callback(true))}
    }
    impl Flags<enabled> for Holder<enabled> where enabled:bool {}
    fn run()->bool {not Holder<true>.get() and Holder<false>.get()}";
    let package = support::compile(source);
    assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace("Flags<enabled>).callback", "Flags<'x'>).callback");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nominal_value_domains_preserve_trait_subject_guarantees() {
    let source = "trait Same where T, enabled:bool, requires Self == T {}
        impl Same<T,enabled> for T where T, enabled:bool {}
        struct Holder where enabled:bool, T, S:-Same<T,enabled> {
            pub fn convert(value:S)->T {value}
        }
        fn run()->bool {Holder<true,bool,bool>.convert(true)}";
    let inherited = source.replace("struct Holder", "trait Child :- Same<T,enabled> where T, enabled:bool {} impl Child<bool,enabled> for bool where enabled:bool {} struct Holder").replace("S:-Same<T,enabled>", "S:-Child<T,enabled>");
    for source in [source.to_owned(), inherited] {
        let package = support::compile(&source);
        assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        let unbounded = source
            .replace("S:-Same<T,enabled>", "S")
            .replace("S:-Child<T,enabled>", "S");
        assert!(support::compile_with_protocols(&unbounded, &Default::default()).is_err());
        let invalid = source.replace(
            "Holder<true,bool,bool>.convert(true)",
            "Holder<true,bool,.char'builtin>.convert('x')",
        );
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn inherent_methods_accept_external_rigid_value_domains() {
    let source = "struct Flag where tag:bool {}
        impl Flag<tag> where tag:bool {fn read()->bool {tag}}
        struct Caller where enabled:bool {
            pub fn forward()->bool {Flag<enabled>.read()}
        }
        fn run()->bool {Caller<true>.forward() and not Caller<false>.forward()}";
    let receiver = source
        .replace("fn read()->bool", "fn read(self)->bool")
        .replace("Flag<enabled>.read()", "Flag<enabled> {}.read()");
    let function_value = source.replace(
        "Flag<enabled>.read()",
        "do {let callback:fn()->bool=Flag<enabled>.read;callback()}",
    );
    for source in [source.to_owned(), receiver, function_value] {
        let package = support::compile(&source);
        assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        let invalid = source.replace("Flag<enabled>", "Flag<'x'>");
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn inherent_methods_accept_external_dependent_value_domains() {
    let source = "struct Tagged where T, tag:T {}
        impl Tagged<T,tag> where T, tag:T {fn read()->T {tag}}
        struct Caller where U, value:U {
            pub fn forward()->U {Tagged<U,value>.read()}
        }
        fn run()->bool {Caller<bool,true>.forward() and not Caller<bool,false>.forward()}
        fn character()->.char'builtin {Caller<.char'builtin,'x'>.forward()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert_eq!(
        support::run_iir(source, "character", vec![]),
        Value::Char('x')
    );
}

#[test]
fn staged_inherent_function_values_preserve_dependent_instance_inputs() {
    let source = "struct Tagged where T, tag:T {}
        impl Tagged<T,tag> where T, tag:T {fn read()->T {tag}}
        struct Factory where U, value:U {
            pub comptime fn obtain()->fn()->U {Tagged<U,value>.read}
        }
        trait Source where T, tag:T {assoc callback:fn()->T=Factory<T,tag>.obtain();}
        struct Holder where T, tag:T {}
        impl Source<T,tag> for Holder<T,tag> where T, tag:T {}
        fn run()->bool {
            Holder<bool,true>.as(Source<bool,true>).callback()
                and not Holder<bool,false>.as(Source<bool,false>).callback()
        }
        fn character()->.char'builtin {Holder<.char'builtin,'x'>.as(Source<.char'builtin,'x'>).callback()}";
    let package = support::compile(source);
    for (entry, expected) in [("run", Value::Bool(true)), ("character", Value::Char('x'))] {
        assert!(luna::instantiation::instantiate(&package, entry, 1).is_err());
        let graph = luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
        let read_keys: Vec<_> = graph
            .functions
            .iter()
            .filter(|f| f.function.name == "read")
            .map(|f| f.key.arguments.to_vec())
            .collect();
        if entry == "run" {
            assert_eq!(read_keys.len(), 2);
            for flag in [true, false] {
                assert!(read_keys.contains(&vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Bool(flag)
                ]));
            }
        } else {
            assert_eq!(
                read_keys,
                [vec![
                    nir::Term::Primitive(nir::Primitive::Char),
                    nir::Term::Char('x')
                ]]
            );
        }
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            expected
        );
        assert!(
            !support::calls(&package, &vm)
                .iter()
                .any(|name| name == "obtain")
        );
    }
}

#[test]
fn trait_value_domains_follow_proved_type_equalities() {
    let source = "trait Read where T, tag:T {fn read()->T;}
        struct Tagged where T, tag:T {}
        impl Read<T,tag> for Tagged<T,tag> where T, tag:T {fn read()->T {tag}}
        struct Caller where T, U, value:U, requires T == U {
            pub fn forward()->T {Tagged<U,value>.as(Read<T,value>).read()}
        }
        fn run()->bool {Caller<bool,bool,true>.forward()}";
    for source in [
        source.to_owned(),
        source.replace("Tagged<U,value>.as", "Tagged<T,value>.as"),
    ] {
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        let missing = source.replace(", requires T == U", "");
        assert!(support::compile_with_protocols(&missing, &Default::default()).is_err());
        let wrong = source.replace("Caller<bool,bool,true>", "Caller<.char'builtin,bool,true>");
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn generic_trait_queries_accept_record_indices_with_symbolic_fields() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true})'type;
        trait Read where mode:Settings {fn read()->bool;}
        struct Host {}
        impl Read<(settings {enabled:flag})> for Host where flag:bool {fn read()->bool {flag}}
        struct Caller where enabled:bool {
            pub fn forward()->bool {Host.as(Read<(settings {enabled:enabled})>).read()}
        }
        fn run()->bool {Caller<true>.forward() and not Caller<false>.forward()}";
    let whole = source.replace(
        "impl Read<(settings {enabled:flag})> for Host where flag:bool {fn read()->bool {flag}}",
        "impl Read<mode> for Host where mode:Settings {fn read()->bool {mode.enabled}}",
    );
    for source in [source.to_owned(), whole] {
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        let wrong = source.replace("settings {enabled:enabled}", "settings {enabled:'x'}");
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn known_record_tag_selects_an_impl_despite_an_earlier_symbolic_field() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true,tag:true})'type;
        trait Read where mode:Settings {fn read()->bool;}
        struct Host {}
        impl Read<(settings {enabled:true,tag:false})> for Host {fn read()->bool {false}}
        impl Read<(settings {enabled:flag,tag:true})> for Host where flag:bool {fn read()->bool {flag}}
        struct Caller where enabled:bool {
            pub fn forward()->bool {Host.as(Read<(settings {enabled:enabled,tag:true})>).read()}
        }
        fn run()->bool {Caller<true>.forward() and not Caller<false>.forward()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn nested_symbolic_records_flow_through_associated_initializers() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {nested:(settings {enabled:true}),tag:true})'type;
        trait Source where mode:Settings {assoc selected:bool=mode.nested.enabled;}
        struct Host {}
        impl Source<mode> for Host where mode:Settings {}
        struct Caller where enabled:bool {
            pub fn forward()->bool {Host.as(Source<(settings {nested:(settings {enabled:enabled}),tag:true})>).selected}
        }
        fn run()->bool {Caller<true>.forward() and not Caller<false>.forward()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn staged_associated_closure_calls_keep_capture_environment_until_invocation() {
    let source = "typealias Callback=%{flag:bool} comptime fn()->bool;
        comptime fn make(flag:bool)->Callback {|| flag}
        trait Flags where enabled:bool {assoc callback:Callback=make(enabled);}
        struct Holder where enabled:bool {
            pub fn get()->bool {decide(Holder<enabled>.as(Flags<enabled>).callback())}
        }
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn decide(comptime value:bool)->bool {value}
        fn run()->bool {Holder<true>.get() and not Holder<false>.get()}";
    let stateful = source.replace("typealias Callback=%{flag:bool} comptime fn()->bool;", "typealias Counter=%{value:bool} comptime fn()->bool; typealias Callback=%{next:Counter} comptime fn()->bool; comptime fn counter(flag:bool)->Counter {var value:bool=flag;|| {value=not value;value}}")
        .replace("comptime fn make(flag:bool)->Callback {|| flag}", "comptime fn make(flag:bool)->Callback {let next=counter(flag);|| {next();next()}}");
    for source in [source.to_owned(), stateful] {
        let package = support::compile(&source);
        assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        assert!(
            !support::calls(&package, &vm)
                .iter()
                .any(|name| name == "make" || name == "counter")
        );
    }
}

#[test]
fn staged_associated_callbacks_accept_locally_prepared_arguments() {
    let source = "typealias Callback=%{flag:bool} comptime fn(bool)->bool;
        comptime fn make(flag:bool)->Callback {|value| flag and value}
        comptime fn identity(value:bool)->bool {value}
        trait Flags where enabled:bool {assoc callback:Callback=make(enabled);}
        struct Holder where enabled:bool {
            pub fn get()->bool {decide(do {
                let value=identity(true);
                Holder<enabled>.as(Flags<enabled>).callback(value)
            })}
        }
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn decide(comptime value:bool)->bool {value}
        fn run()->bool {Holder<true>.get() and not Holder<false>.get()}";
    let local_callback = source.replace(
        "Holder<enabled>.as(Flags<enabled>).callback(value)",
        "let callback=Holder<enabled>.as(Flags<enabled>).callback;callback(value)",
    );
    let stateful = source.replace(
        "let value=identity(true);",
        "var state:bool=false;let next=|| {state=not state;state};
         let first=next();let second=next();let value=first and not second;",
    );
    let conditional_contract = source
        .replace(
            "let value=identity(true);",
            "let value=if true {identity(true)} else {false};",
        )
        .replace(
            "comptime fn identity(value:bool)->bool {value}",
            "comptime fn identity(value:bool)->bool where requires value {value}",
        );
    let symbolic_condition = conditional_contract.replace("if true", "if enabled");
    let invalid_contract = conditional_contract.replace("identity(true)", "identity(false)");
    assert!(support::compile_with_protocols(&invalid_contract, &Default::default()).is_err());
    let runtime_input = source
        .replace("pub fn get()->bool", "pub fn get(input:bool)->bool")
        .replace("identity(true)", "identity(input)")
        .replace(".get()", ".get(true)");
    let error = support::compile_with_protocols(&runtime_input, &Default::default()).unwrap_err();
    assert!(error.contains("runtime parameter"), "{error}");
    for source in [
        source.to_owned(),
        local_callback,
        stateful,
        conditional_contract,
        symbolic_condition,
    ] {
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        assert!(
            !support::calls(&package, &vm)
                .iter()
                .any(|name| name == "identity" || name == "make")
        );
    }
}

#[test]
fn staged_associated_closure_invocations_keep_contracts_and_failure_isolation() {
    let source = "typealias Callback=%{flag:bool} comptime fn()->bool;
        comptime fn checked(flag:bool)->bool where requires flag {flag}
        comptime fn make(flag:bool)->Callback {|| {assumes flag;checked(flag)}}
        trait Flags where enabled:bool {assoc callback:Callback=make(enabled);}
        struct Holder where enabled:bool {pub fn get()->bool {decide(Holder<enabled>.as(Flags<enabled>).callback())}}
        impl Flags<enabled> for Holder<enabled> where enabled:bool {}
        fn decide(comptime value:bool)->bool {value}
        fn good()->bool {Holder<true>.get()}
        fn bad()->bool {Holder<false>.get()}";
    let package = support::compile(source);
    for _ in 0..2 {
        let error = luna::instantiation::instantiate(&package, "bad", 100_000).unwrap_err();
        assert!(error.contains("ProgramContractRejected"), "{error}");
        let graph = luna::instantiation::instantiate(&package, "good", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn deferred_contract_projections_require_the_requested_inherited_interface() {
    let source = "trait Bounds {assoc flag:bool=true;}
        trait Child :- Bounds {}
        struct Holder {
            pub fn check()->bool where requires Self.as(Child).flag {true}
        }
        impl Bounds for Holder {}
        impl Child for Holder {}";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    let missing_child = source.replace("impl Child for Holder {}", "");
    let error = support::compile_with_protocols(&missing_child, &Default::default()).unwrap_err();
    assert!(error.contains("program contract associated projection requires an established interface implementation"), "{error}");
    let missing_parent = source.replace("impl Bounds for Holder {}", "");
    let error = support::compile_with_protocols(&missing_parent, &Default::default()).unwrap_err();
    assert!(
        error.contains("required parent interface implementation is not established"),
        "{error}"
    );
}

#[test]
fn trait_default_program_contracts_can_depend_on_later_implementations() {
    let source = "trait Bounds {assoc flag:bool=true;}
        struct Holder {}
        trait Service {
            fn check()->bool where requires Holder.as(Bounds).flag {true}
        }
        impl Service for Holder {}
        impl Bounds for Holder {}
        fn run()->bool {
            assumes Holder.as(Bounds).flag;
            Holder.as(Service).check()
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let explicit = source.replace(
        "impl Service for Holder {}",
        "impl Service for Holder {fn check()->bool where requires Holder.as(Bounds).flag {true}}",
    );
    assert_eq!(
        support::run_iir(&explicit, "run", vec![]),
        Value::Bool(true)
    );
    let dependent = source
        .replace(
            "trait Bounds {assoc flag:bool=true;}",
            "trait Bounds where T {assoc flag:T;}",
        )
        .replace("as(Bounds)", "as(Bounds<bool>)")
        .replace(
            "impl Bounds for Holder {}",
            "impl Bounds<bool> for Holder {assoc flag:bool=true;}",
        );
    assert_eq!(
        support::run_iir(&dependent, "run", vec![]),
        Value::Bool(true)
    );
    let false_source = source.replace("flag:bool=true", "flag:bool=false");
    let package = support::compile_with_protocols(&false_source, &Default::default()).unwrap();
    let failure = match luna::instantiation::instantiate(&package, "run", 100_000) {
        Err(error) => error,
        Ok(graph) => {
            let mut vm =
                comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
            format!(
                "{:?}",
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap_err()
            )
        }
    };
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    let missing = source
        .split("fn run()")
        .next()
        .unwrap()
        .replace("impl Bounds for Holder {}", "");
    let error = support::compile_with_protocols(&missing, &Default::default()).unwrap_err();
    assert!(error.contains("program contract associated projection requires an established interface implementation"), "{error}");
}

#[test]
fn generic_trait_default_contracts_preserve_associated_value_instances() {
    let source = "trait Bounds where enabled:bool {assoc flag:bool=enabled;}
        struct Holder where enabled:bool {}
        trait Service where enabled:bool {
            fn check()->bool where requires Holder<enabled>.as(Bounds<enabled>).flag {true}
        }
        impl Service<enabled> for Holder<enabled> where enabled:bool {}
        impl Bounds<enabled> for Holder<enabled> where enabled:bool {}
        fn run()->bool {
            assumes Holder<true>.as(Bounds<true>).flag;
            Holder<true>.as(Service<true>).check()
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let explicit = source.replace(
        "impl Service<enabled> for Holder<enabled> where enabled:bool {}",
        "impl Service<enabled> for Holder<enabled> where enabled:bool {fn check()->bool where requires Holder<enabled>.as(Bounds<enabled>).flag {true}}",
    );
    assert_eq!(
        support::run_iir(&explicit, "run", vec![]),
        Value::Bool(true)
    );
    let wrong = source.replace(
        "Holder<true>.as(Service<true>)",
        "Holder<false>.as(Service<false>)",
    );
    let error = support::compile_with_protocols(&wrong, &Default::default()).unwrap_err();
    assert!(
        error.contains("program precondition is not proven"),
        "{error}"
    );
}

#[test]
fn associated_scalar_equality_is_a_program_contract() {
    let source = "trait Flags {assoc left:bool=true;assoc right:bool=true;}
        struct Holder {}
        impl Flags for Holder {}
        comptime fn checked()->bool where requires Holder.as(Flags).left == Holder.as(Flags).right {true}
        fn decide(comptime value:bool)->bool {value}
        fn run()->bool {decide(do {
            assumes Holder.as(Flags).left == Holder.as(Flags).right;
            checked()
        })}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    let checked = package
        .functions
        .iter()
        .find(|f| f.name == "checked")
        .unwrap();
    assert!(checked.requirements.is_empty());
    assert_eq!(checked.preconditions.len(), 1);
    let type_equality = source
        .replace("trait Flags {assoc left:bool=true;assoc right:bool=true;}", "typealias Type=.Type'builtin; trait Flags {assoc left:Type=bool;assoc right:Type=bool;}")
        .replace("assumes Holder.as(Flags).left == Holder.as(Flags).right;", "");
    let type_package =
        support::compile_with_protocols(&type_equality, &Default::default()).unwrap();
    let type_checked = type_package
        .functions
        .iter()
        .find(|f| f.name == "checked")
        .unwrap();
    assert_eq!(type_checked.requirements.len(), 1);
    assert!(type_checked.preconditions.is_empty());
    assert_eq!(
        support::run_iir(&type_equality, "run", vec![]),
        Value::Bool(true)
    );
    let unequal = source.replace("right:bool=true", "right:bool=false");
    let error = match support::compile_with_protocols(&unequal, &Default::default()) {
        Ok(package) => luna::instantiation::instantiate(&package, "run", 100_000).unwrap_err(),
        Err(error) => error,
    };
    assert!(error.contains("ProgramContractRejected"), "{error}");
    let not_equal = unequal.replace(" == ", " != ");
    assert_eq!(
        support::run_iir(&not_equal, "run", vec![]),
        Value::Bool(true)
    );
    let dependent = source
        .replace(
            "trait Flags {assoc left:bool=true;assoc right:bool=true;}",
            "trait Flags where T {assoc left:T;assoc right:T;}",
        )
        .replace(
            "impl Flags for Holder {}",
            "impl Flags<bool> for Holder {assoc left:bool=true;assoc right:bool=true;}",
        )
        .replace("as(Flags)", "as(Flags<bool>)");
    assert_eq!(
        support::run_iir(&dependent, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn nominal_type_defaults_substitute_preceding_value_identities() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        struct Packet where .enabled:bool=true, .T:Type = defer Config<enabled> {value:T}
        comptime fn Run()->Type {Packet<>}
        comptime fn Expected()->Type {Packet<.enabled=true,.T=Config<true>>}
        comptime fn Disabled()->Type {Packet<.enabled=false>}
        comptime fn ExpectedDisabled()->Type {Packet<.enabled=false,.T=Config<false>>}";
    let package = support::compile(source);
    for (entry, expected) in [("Run", "Expected"), ("Disabled", "ExpectedDisabled")] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            luna::invocation::invoke(&package, expected, 100_000).unwrap()
        );
    }
}

#[test]
fn nominal_record_defaults_preserve_complete_value_indices() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        struct Packet where .enabled:bool=true, .config:Config<enabled> = (defer Config<enabled>{flag:enabled}) {value:bool}
        comptime fn Run()->Type {Packet<>}
        comptime fn Expected()->Type {Packet<.enabled=true,.config=Config<true>{flag:true}>}
        comptime fn Disabled()->Type {Packet<.enabled=false>}
        comptime fn ExpectedDisabled()->Type {Packet<.enabled=false,.config=Config<false>{flag:false}>}";
    let package = support::compile(source);
    for (entry, expected) in [("Run", "Expected"), ("Disabled", "ExpectedDisabled")] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            luna::invocation::invoke(&package, expected, 100_000).unwrap()
        );
    }
}

#[test]
fn newtype_record_indices_keep_capture_first_domain_admission() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        struct Holder where T {
            pub newtype Wrapped<config:T> = bool;
        }
        comptime fn Build()->Type {Holder<Config<true>>.Wrapped<Config<true>{flag:true}>}
        comptime fn Other()->Type {Holder<Config<false>>.Wrapped<Config<false>{flag:false}>}";
    let package = support::compile(source);
    let left = luna::invocation::invoke(&package, "Build", 100_000).unwrap();
    let right = luna::invocation::invoke(&package, "Other", 100_000).unwrap();
    assert_ne!(left, right);
    let bad = source.replace(
        "Holder<Config<true>>.Wrapped<Config<true>",
        "Holder<Config<true>>.Wrapped<Config<false>",
    );
    match support::compile_with_protocols(&bad, &Default::default()) {
        Err(_) => {}
        Ok(package) => assert!(luna::invocation::invoke(&package, "Build", 100_000).is_err()),
    }
}

#[test]
fn runtime_nominal_layouts_preserve_record_value_indices() {
    for source in [
        "struct Config {flag:bool}
        struct Packet where config:Config {value:bool}
        fn run()->bool {let packet=Packet<Config{flag:true}>{value:true}; packet.value}",
        "struct Config {flag:bool}
        enum Packet where config:Config {none,some(bool)}
        fn run()->bool {Packet<Config{flag:true}>.some(true) match {.some(value)=>value,_=>false}}",
        "struct Config {flag:bool}
        newtype Packet<config:Config> = bool;
        fn run()->bool {let packet=true.as(Packet<Config{flag:true}>); packet.as(bool)}",
        "struct Config {flag:bool}
        struct Holder where T {pub newtype Packet<config:T> = bool;}
        fn run()->bool {let packet=true.as(Holder<Config>.Packet<Config{flag:true}>); packet.as(bool)}",
    ] {
        assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true), "{source}");
    }
}

#[test]
fn member_domains_substitute_selected_parent_value_indices() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        struct Holder where enabled:bool {pub newtype Wrapped<config:Config<enabled>> = bool;}
        comptime fn Build()->Type {Holder<true>.Wrapped<Config<true>{flag:true}>}
        comptime fn Other()->Type {Holder<false>.Wrapped<Config<false>{flag:false}>}";
    let package = support::compile(source);
    assert_ne!(
        luna::invocation::invoke(&package, "Build", 100_000).unwrap(),
        luna::invocation::invoke(&package, "Other", 100_000).unwrap()
    );
    let invalid = source.replace(
        "Holder<true>.Wrapped<Config<true>",
        "Holder<true>.Wrapped<Config<false>",
    );
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn member_domains_wait_for_dynamic_parent_value_indices_once() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        struct Holder where enabled:bool {pub newtype Wrapped<config:Config<enabled>> = bool;}
        comptime fn enabled()->bool {true}
        comptime fn Build()->Type {Holder<enabled()>.Wrapped<Config<true>{flag:true}>}";
    let package = support::compile(source);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert!(matches!(
        vm.invoke(support::id(&package, "Build"), vec![]).unwrap(),
        Value::Type(_)
    ));
    assert_eq!(support::calls(&package, &vm), vec!["Build", "enabled"]);
    let invalid = source.replace("fn enabled()->bool {true}", "fn enabled()->bool {false}");
    let package = support::compile(&invalid);
    assert!(luna::invocation::invoke(&package, "Build", 100_000).is_err());
}

#[test]
fn selected_value_capture_does_not_inherit_callers_instance() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        struct Holder where enabled:bool {
            pub newtype Wrapped<config:Config<enabled>> = bool;
            pub comptime fn Pick()->Type {Holder<false>.Wrapped<Config<false>{flag:false}>}
        }
        comptime fn Run()->Type {Holder<true>.Pick()}
        comptime fn Expected()->Type {Holder<false>.Wrapped<Config<false>{flag:false}>}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        luna::invocation::invoke(&package, "Expected", 100_000).unwrap()
    );
}

#[test]
fn member_record_defaults_use_selected_value_capture() {
    let source = "typealias Type=.Type'builtin;
        struct Config where enabled:bool {flag:bool}
        comptime fn compute_flag(value:bool)->bool {value}
        struct Holder where enabled:bool {
            pub newtype Wrapped<.config:Config<enabled> = defer Config<enabled>{flag:compute_flag(enabled)}> = bool;
        }
        comptime fn Build()->Type {Holder<true>.Wrapped<>}
        comptime fn Expected()->Type {Holder<true>.Wrapped<.config=Config<true>{flag:true}>}
        comptime fn Disabled()->Type {Holder<false>.Wrapped<>}
        comptime fn ExpectedDisabled()->Type {Holder<false>.Wrapped<.config=Config<false>{flag:false}>}";
    let dynamic = source
        .replace(
            "comptime fn Build()",
            "comptime fn enabled()->bool {true} comptime fn Build()",
        )
        .replace("Holder<true>.Wrapped<>", "Holder<enabled()>.Wrapped<>");
    let dynamic_package = support::compile(&dynamic);
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&dynamic_package, Default::default())
            .unwrap();
    vm.invoke(support::id(&dynamic_package, "Build"), vec![])
        .unwrap();
    assert_eq!(
        support::calls(&dynamic_package, &vm),
        vec!["Build", "enabled", "compute_flag"]
    );
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&dynamic_package, Default::default())
            .unwrap();
    vm.invoke(support::id(&dynamic_package, "Expected"), vec![])
        .unwrap();
    assert_eq!(support::calls(&dynamic_package, &vm), vec!["Expected"]);

    assert_eq!(
        luna::invocation::invoke(&dynamic_package, "Build", 100_000).unwrap(),
        luna::invocation::invoke(&dynamic_package, "Expected", 100_000).unwrap()
    );
    let package = support::compile(source);
    for (entry, expected) in [("Build", "Expected"), ("Disabled", "ExpectedDisabled")] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            luna::invocation::invoke(&package, expected, 100_000).unwrap()
        );
    }
}

#[test]
fn comptime_calls_accept_values_with_record_indexed_nominal_types() {
    for source in [
        "struct Config {flag:bool}
        struct Packet where config:Config {value:bool}
        comptime fn read(value:Packet<Config{flag:true}>)->bool {value.value}
        comptime fn Run()->bool {read(Packet<Config{flag:true}>{value:true})}",
        "struct Config {flag:bool}
        enum Packet where config:Config {none,some(bool)}
        comptime fn read(value:Packet<Config{flag:true}>)->bool {value match {.some(flag)=>flag,_=>false}}
        comptime fn Run()->bool {read(Packet<Config{flag:true}>.some(true))}",
        "struct Config {flag:bool}
        newtype Packet<config:Config> = bool;
        comptime fn read(value:Packet<Config{flag:true}>)->bool {value.as(bool)}
        comptime fn Run()->bool {read(true.as(Packet<Config{flag:true}>))}",
    ] {
        let roundtrip = format!("{} comptime fn identity(value:Packet<Config{{flag:true}}>)->Packet<Config{{flag:true}}> {{value}}", source.replace("{value.value}", "{identity(value).value}").replace("{value match", "{identity(value) match").replace("{value.as(bool)}", "{identity(value).as(bool)}"));
        let package = support::compile(&roundtrip);
        assert_eq!(luna::invocation::invoke(&package, "Run", 100_000).unwrap(), Value::Bool(true), "{source}");
        let (prefix, run) = source.split_once("comptime fn Run").unwrap();
        let wrong = format!("{prefix}comptime fn Run{}", run.replace("flag:true", "flag:false"));
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err(), "{wrong}");
    }
}

#[test]
fn nested_record_value_indices_materialize_their_full_layouts() {
    for (definition, value) in [
        (
            "struct Inner where config:Config {value:bool}",
            "Inner<Config{flag:true}>{value:true}",
        ),
        (
            "enum Inner where config:Config {none,some(bool)}",
            "Inner<Config{flag:true}>.some(true)",
        ),
        (
            "newtype Inner<config:Config> = bool;",
            "true.as(Inner<Config{flag:true}>)",
        ),
    ] {
        let source = format!(
            "struct Config {{flag:bool}} {definition}
            struct Outer where inner:Inner<Config{{flag:true}}> {{value:bool}}
            fn run()->bool {{let value=Outer<{value}>{{value:true}}; value.value}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "{source}"
        );
        let wrong = source.replace(
            &format!("Outer<{value}>"),
            &format!("Outer<{}>", value.replace("flag:true", "flag:false")),
        );
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn record_indexed_nominal_types_select_inherent_methods() {
    for source in [
        "struct Config {flag:bool}
        struct Packet where config:Config {value:bool}
        impl Packet<config> where config:Config {fn read(self)->bool {self.value and config.flag}}
        fn run()->bool {Packet<Config{flag:true}>{value:true}.read()}",
        "struct Config {flag:bool}
        enum Packet where config:Config {none,some(bool)}
        impl Packet<config> where config:Config {fn read(self)->bool {config.flag and (self match {.some(value)=>value,_=>false})}}
        fn run()->bool {Packet<Config{flag:true}>.some(true).read()}",
        "struct Config {flag:bool}
        newtype Packet<config:Config> = bool;
        impl Packet<config> where config:Config {fn read(self)->bool {self.as(bool) and config.flag}}
        fn run()->bool {true.as(Packet<Config{flag:true}>).read()}",
    ] {
        assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true), "{source}");
        assert_eq!(support::run_iir(&source.replace("Config{flag:true}", "Config{flag:false}"), "run", vec![]), Value::Bool(false));
    }
}

#[test]
fn record_indexed_nominal_types_select_trait_methods() {
    for (declaration, body, receiver) in [
        (
            "struct Packet where config:Config {value:bool}",
            "self.value and config.flag",
            "Packet<Config{flag:true}>{value:true}",
        ),
        (
            "enum Packet where config:Config {none,some(bool)}",
            "config.flag and (self match {.some(value)=>value,_=>false})",
            "Packet<Config{flag:true}>.some(true)",
        ),
        (
            "newtype Packet<config:Config> = bool;",
            "self.as(bool) and config.flag",
            "true.as(Packet<Config{flag:true}>)",
        ),
    ] {
        let source = format!(
            "trait Read {{fn read(self)->bool;}}
            struct Config {{flag:bool}} {declaration}
            impl Read for Packet<config> where config:Config {{fn read(self)->bool {{{body}}}}}
            fn run()->bool {{{receiver}.as(Read).read()}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "{source}"
        );
        assert_eq!(
            support::run_iir(
                &source.replace("Config{flag:true}", "Config{flag:false}"),
                "run",
                vec![]
            ),
            Value::Bool(false)
        );
    }
}

#[test]
fn nominal_trait_arguments_preserve_selected_values() {
    let source = "struct Config {enabled:bool}
        trait Read where config:Config {fn read(self)->bool;}
        struct Host {}
        impl Read<config> for Host where config:Config {fn read(self)->bool {config.enabled}}
        fn run()->bool {Host{}.as(Read<Config{enabled:true}>).read() and not Host{}.as(Read<Config{enabled:false}>).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let defaults = source
        .replace(
            "where config:Config {fn read(self)->bool;}",
            "where .config:Config=(Config{enabled:true}) {fn read(self)->bool;}",
        )
        .replace("Read<Config{enabled:true}>", "Read<>")
        .replace("Read<config>", "Read<.config=config>")
        .replace(
            "Read<Config{enabled:false}>",
            "Read<.config=Config{enabled:false}>",
        );
    assert_eq!(
        support::run_iir(&defaults, "run", vec![]),
        Value::Bool(true)
    );
    let invalid = source.replace("Config{enabled:true}", "Config{enabled:'x'}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn nominal_trait_indices_distinguish_fixed_implementations() {
    for (definition, yes, no) in [
        (
            "struct Config {enabled:bool}",
            "Config{enabled:true}",
            "Config{enabled:false}",
        ),
        (
            "enum Config {a(bool),b(bool)}",
            "Config.a(true)",
            "Config.b(true)",
        ),
        (
            "newtype Config = bool;",
            "true.as(Config)",
            "false.as(Config)",
        ),
        (
            "struct Flag {enabled:bool} struct Config {flag:Flag}",
            "Config{flag:Flag{enabled:true}}",
            "Config{flag:Flag{enabled:false}}",
        ),
    ] {
        let source = format!("{definition}
            trait Read where config:Config {{fn read(self)->bool;}}
            struct Host {{}}
            impl Read<{yes}> for Host {{fn read(self)->bool {{true}}}}
            impl Read<{no}> for Host {{fn read(self)->bool {{false}}}}
            fn run()->bool {{Host{{}}.as(Read<{yes}>).read() and not Host{{}}.as(Read<{no}>).read()}}");
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "{source}"
        );
        assert!(
            support::compile_with_protocols(&source.replace(no, yes), &Default::default()).is_err()
        );
    }
}

#[test]
fn nominal_trait_indices_flow_through_parent_methods_and_bounds() {
    let source = "struct Config {enabled:bool}
        trait Read where config:Config {fn read(self)->bool {config.enabled}}
        trait Child :- Read<config> where config:Config {}
        struct Host {}
        impl Read<config> for Host where config:Config {}
        impl Child<config> for Host where config:Config {}
        fn read(value:T)->bool where T:-Child<Config{enabled:true}> {value.read()}
        fn run()->bool {read(Host{}) and not Host{}.as(Child<Config{enabled:false}>).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn symbolic_nominal_trait_indices_infer_fields_after_tag_selection() {
    for (definition, left, right) in [
        (
            "struct Config {tag:bool,enabled:bool}",
            "Config{tag:true,enabled:flag}",
            "Config{tag:false,enabled:flag}",
        ),
        (
            "enum Config {a(bool),b(bool)}",
            "Config.a(flag)",
            "Config.b(flag)",
        ),
        (
            "struct Flag {enabled:bool} struct Config {tag:bool,inner:Flag}",
            "Config{tag:true,inner:Flag{enabled:flag}}",
            "Config{tag:false,inner:Flag{enabled:flag}}",
        ),
    ] {
        let yes = left.replace("flag", "true");
        let no = right.replace("flag", "false");
        let source = format!(
            "{definition}
            trait Read where config:Config {{fn read(self)->bool;}}
            struct Host {{}}
            impl Read<{left}> for Host where flag:bool {{fn read(self)->bool {{flag}}}}
            impl Read<{right}> for Host where flag:bool {{fn read(self)->bool {{not flag}}}}
            fn run()->bool {{Host{{}}.as(Read<{yes}>).read() and Host{{}}.as(Read<{no}>).read()}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn repeated_nominal_index_binders_require_identical_values() {
    let source = "struct Config {left:bool,right:bool}
        trait Read where config:Config {fn read(self)->bool;}
        struct Host {}
        impl Read<Config{left:flag,right:flag}> for Host where flag:bool {fn read(self)->bool {flag}}
        fn run()->bool {Host{}.as(Read<Config{left:true,right:true}>).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace("left:true,right:true", "left:true,right:false"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn symbolic_nominal_trait_arguments_forward_through_generic_bounds() {
    let source = "struct Config {enabled:bool}
        trait Read where config:Config {fn read(self)->bool;}
        struct Host where enabled:bool {}
        impl Read<Config{enabled:flag}> for Host<flag> where flag:bool {fn read(self)->bool {flag}}
        struct Caller where flag:bool {
            pub fn read(value:T)->bool where T:-Read<Config{enabled:flag}> {value.read()}
        }
        fn run()->bool {Caller<true>.read(Host<true>{}) and not Caller<false>.read(Host<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let inherited = source.replace("struct Host where", "trait Child :- Read<config> where config:Config {} struct Host where")
        .replace("struct Caller where", "impl Child<Config{enabled:flag}> for Host<flag> where flag:bool {} struct Caller where")
        .replace("T:-Read<Config{enabled:flag}>", "T:-Child<Config{enabled:flag}>");
    assert_eq!(
        support::run_iir(&inherited, "run", vec![]),
        Value::Bool(true)
    );
    let mismatch = source.replace(
        "Caller<true>.read(Host<true>{})",
        "Caller<true>.read(Host<false>{})",
    );
    assert!(support::compile_with_protocols(&mismatch, &Default::default()).is_err());
}

#[test]
fn normal_head_bounds_wait_for_preceding_value_parameter_domains() {
    let source = "typealias Type=.Type'builtin; struct Config {enabled:bool}
        trait Allowed where config:Config {}
        struct Host where enabled:bool {}
        impl Allowed<Config{enabled:flag}> for Host<flag> where flag:bool {}
        pure comptime fn Select<flag:bool,T:-Allowed<Config{enabled:flag}>> -> Type {T}
        comptime fn Build()->Type {Select<true,Host<true>>}";
    let package = support::compile(source);
    luna::invocation::invoke(&package, "Build", 100_000).unwrap();
    let wrong = source.replace("Select<true,Host<true>>", "Select<true,Host<false>>");
    let rejected = support::compile(&wrong);
    assert!(luna::invocation::invoke(&rejected, "Build", 100_000).is_err());
    let required = source.replace(
        "comptime fn Build()->Type {Select<true,Host<true>>}",
        "const selected=Select<true,Host<true>>; comptime fn Build()->Type {selected}",
    );
    let package = support::compile(&required);
    luna::invocation::invoke(&package, "Build", 100_000).unwrap();
    let wrong_required = required.replace("Select<true,Host<true>>", "Select<true,Host<false>>");
    assert!(support::compile_with_protocols(&wrong_required, &Default::default()).is_err());
}

#[test]
fn function_metadata_retains_runtime_member_value_capture_domains() {
    let source = "struct Host where enabled:bool {pub fn read()->bool {enabled}}
        fn run()->bool {Host<true>.read()}";
    let package = support::compile(source);
    let function = package.functions.iter().find(|f| f.name == "read").unwrap();
    let metadata = package
        .function_parameters
        .iter()
        .find(|d| d.declaration == function.declaration)
        .unwrap();
    assert_eq!(metadata.parameters, function.type_parameters);
    assert_eq!(
        metadata.domains,
        vec![nir::Term::Primitive(nir::Primitive::Bool)]
    );
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_function_value_clauses_specialize_explicit_instances() {
    let source = "fn enabled()->bool where flag:bool {flag}
        fn run()->bool {enabled<true>() and not enabled<false>()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_function_value_clauses_infer_from_nominal_arguments() {
    let source = "struct Host where enabled:bool {}
        fn enabled(value:Host<flag>)->bool where flag:bool {flag}
        fn run()->bool {enabled(Host<true>{}) and not enabled(Host<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_function_value_clauses_check_dependent_trait_bounds() {
    let source = "struct Config {enabled:bool}
        trait Read where config:Config {fn read(self)->bool;}
        struct Host where enabled:bool {}
        impl Read<Config{enabled:flag}> for Host<flag> where flag:bool {fn read(self)->bool {flag}}
        fn read(value:T, witness:Host<flag>)->bool where flag:bool,T:-Read<Config{enabled:flag}> {value.read()}
        fn run()->bool {read(Host<true>{},Host<true>{}) and not read(Host<false>{},Host<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace(
                "read(Host<true>{},Host<true>{})",
                "read(Host<true>{},Host<false>{})"
            ),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn ordinary_function_value_clauses_preserve_defaults_and_callable_instances() {
    let source = "fn enabled()->bool where .flag:bool=true {flag}
        fn run()->bool {let off:fn()->bool=enabled<.flag=false>; enabled() and enabled<>() and not off()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace(".flag=false", ".flag='x'"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn ordinary_function_value_clauses_preserve_dependent_defaults() {
    let source = "struct Host where enabled:bool {}
        fn enabled(value:Host<flag>)->bool where flag:bool,.fallback:bool=flag {fallback}
        fn run()->bool {enabled(Host<true>{}) and not enabled(Host<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_function_value_clauses_admit_nominal_values_and_mixed_types() {
    let source = "typealias Type=.Type'builtin;
        struct Config {enabled:bool}
        fn enabled()->bool where config:Config {config.enabled}
        fn keep(value:T)->T where T,tag:T {value}
        fn run()->bool {enabled<Config{enabled:true}>() and not enabled<Config{enabled:false}>() and keep<bool,true>(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace("keep<bool,true>", "keep<bool,'x'>"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn method_value_clauses_preserve_captured_prefix_and_dependent_defaults() {
    let source = "struct Host where enabled:bool {
        pub fn choose()->bool where flag:bool,.fallback:bool=enabled {flag and fallback}
    }
    fn run()->bool {Host<true>.choose<true>() and not Host<false>.choose<true>()
        and not Host<true>.choose<false>() and Host<false>.choose<true,.fallback=true>()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace("choose<false>", "choose<'x'>"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn method_value_clauses_instantiate_domains_from_captured_types() {
    let source = "struct Host where T {
        pub fn pick()->T where value:T {value}
    }
    fn run()->bool {let off:fn()->bool=Host<bool>.pick<false>;
        Host<bool>.pick<true>() and not off()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace("pick<true>", "pick<'x'>"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn value_clause_inference_survives_runtime_argument_spreads() {
    let source = "struct Flag where enabled:bool {}
        fn packet(value:Flag<flag>)->Tuple<Flag<flag>,bool> where flag:bool {(value,true)}
        fn read(value:Flag<flag>, tail:bool)->bool where flag:bool {flag and tail}
        fn run()->bool {read(...packet(Flag<true>{})) and not read(...packet(Flag<false>{}))}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn method_value_clause_inference_survives_runtime_argument_spreads() {
    let source = "struct Flag where enabled:bool {}
        fn packet(value:Flag<flag>)->Tuple<Flag<flag>,bool> where flag:bool {(value,true)}
        struct Host where outer:bool {
            pub fn read(value:Flag<flag>,tail:bool)->bool where flag:bool {outer and flag and tail}
        }
        fn run()->bool {Host<true>.read(...packet(Flag<true>{}))
            and not Host<true>.read(...packet(Flag<false>{}))
            and not Host<false>.read(...packet(Flag<true>{}))}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn value_clause_inference_survives_property_collection() {
    let source = "struct Flag where enabled:bool {}
        fn packet(...fields:T)->T where T {fields}
        fn exemplar()->Flag<flag> where flag:bool {Flag<flag>{}}
        fn read(...items:Tuple<bool>,...fields:(packet {value:exemplar<flag>()})'type)->bool where flag:bool {flag}
        fn run()->bool {(read {true,value:Flag<true>{}}) and not (read {true,value:Flag<false>{}})}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn value_clause_argument_spreads_reject_conflicting_instance_inputs() {
    let source = "struct Flag where enabled:bool {}
        fn packet()->Tuple<Flag<true>,Flag<false>> {(Flag<true>{},Flag<false>{})}
        fn read(a:Flag<flag>,b:Flag<flag>)->bool where flag:bool {flag}
        fn run()->bool {read(...packet())}";
    assert!(support::compile_with_tuple_protocols(source, &Default::default()).is_err());
}

#[test]
fn value_clauses_infer_alongside_comptime_parameters() {
    let source = "struct Flag where enabled:bool {}
        fn read(comptime invert:bool,value:Flag<flag>)->bool where flag:bool {if invert {not flag} else {flag}}
        fn run()->bool {read(false,Flag<true>{}) and read(true,Flag<false>{})
            and not read(false,Flag<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn value_clauses_flow_through_nested_comptime_calls() {
    let source = "struct Flag where enabled:bool {}
        comptime fn read(value:Flag<flag>)->bool where flag:bool {flag}
        comptime fn evaluate(comptime invert:bool,value:Flag<flag>)->bool where flag:bool {if invert {not read(value)} else {read(value)}}
        comptime fn run()->bool {evaluate(false,Flag<true>{}) and evaluate(true,Flag<false>{})}";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn value_clauses_infer_for_dependent_result_signatures() {
    let source = "typealias Type=.Type'builtin;
        struct Flag where enabled:bool {}
        fn read(comptime T:Type,value:Flag<flag>,result:T)->Tuple<T,bool> where flag:bool {(result,flag)}
        fn run()->bool {let (a,yes)=read(bool,Flag<true>{},true);
            let (b,no)=read(bool,Flag<false>{},false);a and yes and not b and not no}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
    let spread = source.replace("fn run()", "fn packet(value:Flag<flag>,result:bool)->Tuple<Flag<flag>,bool> where flag:bool {(value,result)} fn run()")
        .replace("read(bool,Flag<true>{},true)", "read(bool,...packet(Flag<true>{},true))")
        .replace("read(bool,Flag<false>{},false)", "read(bool,...packet(Flag<false>{},false))");
    assert_eq!(
        support::run_iir_with_tuple(&spread, "run", vec![]),
        Value::Bool(true)
    );
    let wrong = source.replace(
        "read(bool,Flag<true>{},true)",
        "read(bool,Flag<true>{},'x')",
    );
    assert!(support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn open_value_clause_inference_settles_dependent_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Flag where enabled:bool {}
        fn read(comptime T:Type,value:Flag<flag>,result:T)->Tuple<T,bool>
        where flag:bool,.fallback:bool=flag {(result,fallback)}
        fn run()->bool {let (a,yes)=read(bool,Flag<true>{},true);
            let (b,no)=read(bool,Flag<false>{},false);a and yes and not b and not no}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn open_value_clauses_infer_while_result_computation_waits() {
    let source = "typealias Type=.Type'builtin;
        struct Flag where enabled:bool {}
        comptime fn Result(T:Type)->Type {T}
        fn read(comptime T:Type,value:Flag<flag>,yes:T,no:T)->Result(T)
        where flag:bool,.fallback:bool=flag {if fallback {yes} else {no}}
        fn run()->bool {read(bool,Flag<true>{},true,false)
            and not read(bool,Flag<false>{},true,false)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn dependent_methods_route_computed_packets_with_value_captures() {
    let source = "typealias Type=.Type'builtin;
        struct Flag where enabled:bool {}
        struct Host where outer:bool {
            pub fn read(comptime T:Type,value:Flag<flag>,result:T)->Tuple<T,bool>
            where flag:bool {(result,outer and flag)}
        }
        fn packet(value:Flag<flag>,result:bool)->Tuple<Flag<flag>,bool> where flag:bool {(value,result)}
        fn run()->bool {let (a,yes)=Host<true>.read(bool,...packet(Flag<true>{},true));
            let (b,no)=Host<false>.read(bool,...packet(Flag<true>{},false));
            a and yes and not b and not no}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
    let bound = source
        .replace("pub fn read(comptime", "pub fn read(self,comptime")
        .replace("Host<true>.read", "Host<true>{}.read")
        .replace("Host<false>.read", "Host<false>{}.read");
    assert_eq!(
        support::run_iir_with_tuple(&bound, "run", vec![]),
        Value::Bool(true)
    );
    let wrong = source.replace("read(bool,...packet", "read(.u8'builtin,...packet");
    assert!(support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn dependent_method_packets_execute_once_after_the_receiver() {
    let source = "typealias Type=.Type'builtin;
        struct Flag where enabled:bool {}
        struct Host where outer:bool {
            pub fn read(self,comptime T:Type,value:Flag<flag>,result:T)->Tuple<T,bool>
            where flag:bool {(result,outer and flag)}
        }
        fn receiver(state:*bool)->Host<true> {state.* = true;Host<true>{}}
        fn packet(state:*bool)->Tuple<Flag<true>,bool> {let previous=state.*;state.* = not previous;(Flag<true>{},previous)}
        fn run()->bool {var state=false;
            let (observed,enabled)=receiver(state.ref).read(bool,...packet(state.ref));
            observed and enabled and not state}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn trait_methods_infer_and_explicitly_select_value_clauses() {
    let source = "struct Flag where enabled:bool {}
        trait Read {fn read(self,value:Flag<flag>)->bool where flag:bool;}
        struct Host {}
        impl Read for Host {fn read(self,value:Flag<flag>)->bool where flag:bool {flag}}
        fn run()->bool {Host{}.as(Read).read(Flag<true>{}) and not Host{}.as(Read).read<false>(Flag<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn trait_method_value_clauses_preserve_generic_dispatch_and_metadata() {
    let source = "struct Flag where enabled:bool {}
        trait Read {fn read(self,value:Flag<flag>)->bool where flag:bool;}
        struct Host {}
        impl Read for Host {fn read(self,value:Flag<flag>)->bool where flag:bool {flag}}
        fn forward(value:T,witness:Flag<flag>)->bool where T:-Read,flag:bool {value.read(witness)}
        fn run()->bool {forward(Host{},Flag<true>{}) and not forward(Host{},Flag<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let package = support::compile(source);
    let method = &package
        .traits
        .iter()
        .find(|t| t.name == "Read")
        .unwrap()
        .methods[0];
    let metadata = package
        .function_parameters
        .iter()
        .find(|d| d.declaration == method.declaration)
        .unwrap();
    assert!(metadata.parameters.contains(&method.type_parameters[0]));
    let domains = package.declaration_parameter_domains(&mut 100_000).unwrap();
    assert_eq!(
        domains[&method.type_parameters[0]],
        nir::Term::Primitive(nir::Primitive::Bool)
    );
}

#[test]
fn trait_method_value_domains_are_checked_even_without_signature_occurrences() {
    let correct = "trait Select {fn select(self)->bool where flag:bool;}
        struct Host {}
        impl Select for Host {fn select(self)->bool where value:bool {value}}
        fn run()->bool {Host{}.as(Select).select<true>()}";
    assert_eq!(support::run_iir(correct, "run", vec![]), Value::Bool(true));
    let wrong = correct.replace("where value:bool {value}", "where value:'x''type {true}");
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn trait_method_mixed_clause_domains_support_alpha_renaming() {
    let source = "trait Pick {fn pick(self,value:T)->T where T,tag:T;}
        struct Host {}
        impl Pick for Host {fn pick(self,value:U)->U where U,marker:U {value}}
        fn run()->bool {Host{}.as(Pick).pick<bool,true>(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn default_trait_methods_preserve_value_clauses_and_interface_captures() {
    let source = "struct Flag where enabled:bool {}
        trait Read where outer:bool {
            fn read(self,value:Flag<flag>)->bool where flag:bool,.fallback:bool=flag {outer and fallback}
        }
        struct Host {}
        impl Read<true> for Host {}
        fn run()->bool {Host{}.as(Read<true>).read(Flag<true>{})
            and not Host{}.as(Read<true>).read(Flag<false>{})
            and Host{}.as(Read<true>).read<false,.fallback=true>(Flag<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let comptime = source.replace("fn ", "comptime fn ");
    let package = support::compile(&comptime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let required = format!("{comptime} const checked=run(); comptime fn check()->bool {{checked}}");
    assert_eq!(
        luna::invocation::invoke(&support::compile(&required), "check", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn trait_method_value_defaults_use_interface_call_contract() {
    let source = "trait Read {fn read(self)->bool where .flag:bool=true;}
        struct Host {}
        impl Read for Host {fn read(self)->bool where .flag:bool=false {flag}}
        fn run()->bool {Host{}.as(Read).read() and not Host{}.as(Read).read<.flag=false>()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn comptime_trait_methods_preserve_mixed_value_domains() {
    let source = "trait Pick {comptime fn pick(self,value:T)->T where T,tag:T;}
        struct Host {}
        impl Pick for Host {comptime fn pick(self,value:U)->U where U,marker:U {value}}
        comptime fn run()->bool {Host{}.as(Pick).pick<bool,true>(true)
            and not Host{}.as(Pick).pick<bool,false>(false)}";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let required = format!("{source} const checked=run(); comptime fn check()->bool {{checked}}");
    assert_eq!(
        luna::invocation::invoke(&support::compile(&required), "check", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert!(
        support::compile_with_protocols(
            &source.replace("pick<bool,true>", "pick<bool,'x'>"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn higher_order_value_clauses_infer_from_callback_signatures() {
    let source = "struct Flag where enabled:bool {}
        fn read(value:Flag<flag>)->bool where flag:bool {flag}
        fn apply(callback:fn(Flag<flag>)->bool,value:Flag<flag>)->bool where flag:bool {callback(value)}
        fn run()->bool {let yes:fn(Flag<true>)->bool=read;
            let no:fn(Flag<false>)->bool=read;
            yes(Flag<true>{}) and not no(Flag<false>{})
            and apply(read,Flag<true>{}) and not apply(read,Flag<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let instances = graph
        .functions
        .iter()
        .filter(|instance| instance.function.name == "read")
        .collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    for flag in [true, false] {
        assert!(
            instances
                .iter()
                .any(|instance| instance.key.arguments.to_vec() == vec![nir::Term::Bool(flag)])
        );
    }
    let wrong = source.replace(
        "fn(Flag<true>)->bool=read",
        "fn(Flag<true>)->bool=read<false>",
    );
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn higher_order_methods_infer_value_clauses_after_captures() {
    let source = "struct Flag where enabled:bool {}
        struct Host where outer:bool {
            pub fn read(value:Flag<flag>)->bool where flag:bool {outer and flag}
        }
        fn run()->bool {let yes:fn(Flag<true>)->bool=Host<true>.read;
            let no:fn(Flag<true>)->bool=Host<false>.read;
            yes(Flag<true>{}) and not no(Flag<true>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn quote_functions_infer_value_clauses_and_preserve_defaults() {
    let source = "struct Flag where enabled:bool {}
        quote fn read(value:Flag<flag>)->bool where flag:bool,.fallback:bool=flag {fallback}
        fn run()->bool {read(Flag<true>{}) and not read(Flag<false>{})
            and read<false,.fallback=true>(Flag<false>{})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn ordinary_function_clause_domains_depend_on_prior_value_indices() {
    let source = "struct Flag where enabled:bool {}
        fn enabled()->bool where flag:bool,witness:Flag<flag> {flag}
        fn run()->bool {enabled<true,Flag<true>{}>() and not enabled<false,Flag<false>{}>()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    assert!(
        support::compile_with_protocols(
            &source.replace("enabled<true,Flag<true>{}>", "enabled<true,Flag<false>{}>"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn generic_effect_memory_preserves_value_indexed_operation_labels() {
    let source = "struct Flag where enabled:bool {}
        effect echo(value:T)->T where T;
        fn read(value:Flag<flag>)->bool where flag:bool {flag}
        fn subject()->#[echo<Flag<flag>>] bool where flag:bool {
            read(echo(Flag<flag>{})#)
        }
        fn yes()->bool {subject<true>()# {echo<Flag<true>>(value)=>value,}}
        fn no()->bool {subject<false>()# {echo<Flag<false>>(value)=>value,}}
        fn handled()->bool where flag:bool {subject<flag>()# {echo<Flag<flag>>(value)=>value,}}
        fn nested_yes()->bool {handled<true>()}
        fn nested_no()->bool {handled<false>()}";
    let package = support::compile(source);
    for (entry, expected) in [
        ("yes", true),
        ("no", false),
        ("nested_yes", true),
        ("nested_no", false),
    ] {
        elaboration::verification::memory::infer_in_package(
            &package,
            support::id(&package, entry),
            &[],
            &mut 100_000,
        )
        .unwrap();
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(expected)
        );
    }
    assert!(
        support::compile_with_protocols(
            &source.replace(
                "echo<Flag<true>>(value)=>value",
                "echo<Flag<false>>(value)=>value"
            ),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn effect_requests_preserve_value_indexed_type_instance_identity() {
    let source = "struct Flag where enabled:bool {}
        effect echo(value:T)->T where T;
        fn read(value:Flag<flag>)->bool where flag:bool {flag}
        fn subject()->#[echo<Flag<true>>,echo<Flag<false>>] bool {
            read(echo(Flag<true>{})#) and not read(echo(Flag<false>{})#)
        }
        fn run()->bool {subject()# {
            echo<Flag<true>>(value)=>value,
            echo<Flag<false>>(value)=>value,
        }}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let missing = source.replace("echo<Flag<false>>(value)=>value,", "");
    assert!(support::compile_with_protocols(&missing, &Default::default()).is_err());
}

#[test]
fn nested_generic_calls_share_partial_value_index_contexts() {
    let source = "struct Flag where enabled:bool {}
        fn identity(value:T)->T where T {value}
        fn read(value:Flag<flag>)->bool where flag:bool {flag}
        fn run()->bool {read(identity(Flag<true>{})) and not read(identity(Flag<false>{}))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn enum_value_indices_infer_from_variant_payload_types() {
    let source = "struct Flag where enabled:bool {}
        enum Wrapped where flag:bool {Item(Flag<flag>)}
        fn read(value:Wrapped<flag>)->bool where flag:bool {flag}
        fn run()->bool {read(.Item(Flag<true>{})) and not read(.Item(Flag<false>{}))}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
    let spread = source
        .replace(
            "fn run()",
            "fn packet(value:Flag<flag>)->Tuple<Flag<flag>> where flag:bool {(value,)} fn run()",
        )
        .replace(".Item(Flag<true>{})", ".Item(...packet(Flag<true>{}))")
        .replace(".Item(Flag<false>{})", ".Item(...packet(Flag<false>{}))");
    assert_eq!(
        support::run_iir_with_tuple(&spread, "run", vec![]),
        Value::Bool(true)
    );
    let missing = source
        .replace("{Item(Flag<flag>)}", "{Empty,Item(Flag<flag>)}")
        .replace(".Item(Flag<true>{})", ".Empty");
    assert!(support::compile_with_tuple_protocols(&missing, &Default::default()).is_err());
}

#[test]
fn enum_value_index_inference_rejects_conflicting_payloads() {
    let source = "struct Flag where enabled:bool {}
        enum Pair where T,flag:bool {Items(T,Flag<flag>,Flag<flag>)}
        fn read(value:Pair<bool,flag>)->bool where flag:bool {flag}
        fn run()->bool {read(.Items(true,Flag<true>{},Flag<true>{}))}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let wrong = source.replace("Flag<true>{},Flag<true>{}", "Flag<true>{},Flag<false>{}");
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn enum_payload_inference_preserves_dependent_domains_and_named_defaults() {
    let source = "struct Key where T,tag:T {}
        enum Entry where T,tag:T {Data {key:Key<T,tag>,value:T=tag}}
        fn read(value:Entry<T,tag>)->T where T,tag:T {
            let .Data {key:_,value}=value;value
        }
        fn wrap(key:Key<T,tag>)->T where T,tag:T {read(.Data {key:key})}
        fn run()->bool {
            wrap(Key<bool,true>{}) and not wrap(Key<bool,false>{})
            and wrap(Key<.u8'builtin,7>{}) == 7
            and not read(.Data {value:false,key:Key<bool,true>{}})
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for wrong in [
        source.replace("value:false", "value:'x'"),
        source.replace("Key<.u8'builtin,7>", "Key<.u8'builtin,256>"),
        source.replace("read(.Data {key:key})", "read(.Data {})"),
    ] {
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn forwarded_value_clauses_wait_for_default_type_domains() {
    let source = "struct Key where .T:.Type'builtin=bool,tag:T {}
        fn read()->T where .T:.Type'builtin=bool,tag:T {tag}
        fn unpack(value:Key<tag>)->bool where tag:bool {tag}
        fn forward()->bool where bit:bool {
            read<bit>() and unpack(Key<bit>{})
        }
        fn run()->bool {forward<true>() and not forward<false>()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for wrong in [
        source.replace("read<bit>()", "read<bit,.T=.char'builtin>()"),
        source.replace("Key<bit>{}", "Key<bit,.T=.char'builtin>{}"),
    ] {
        assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    }
}

#[test]
fn abstract_comptime_parameters_bind_dependent_arguments_and_results() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{DeclarationId, Primitive, Sort, Term};
    for (kind, definition) in [
        (0, "struct Token where enabled:bool {}"),
        (1, "enum Token where enabled:bool {unit}"),
        (2, "newtype Token<enabled:bool> = bool;"),
    ] {
        let source=format!("{definition}
            comptime fn keep(comptime enabled:bool,value:Token<enabled>)->Token<enabled> {{value}}
            comptime fn forward(comptime enabled:bool,value:Token<enabled>)->Token<enabled> {{keep(enabled,value)}}");
        let package = support::compile(&source);
        let parameter = DeclarationId {
            package: 99,
            local: 1,
        };
        let domain = Term::Primitive(Primitive::Bool);
        let symbolic = Term::Rigid {
            declaration: parameter,
            sort: Sort::ComptimeValue,
        };
        let token = match kind {
            0 => {
                package
                    .structs
                    .iter()
                    .find(|s| s.name == "Token")
                    .unwrap()
                    .declaration
            }
            1 => {
                package
                    .enums
                    .iter()
                    .find(|s| s.name == "Token")
                    .unwrap()
                    .declaration
            }
            _ => {
                package
                    .newtypes
                    .iter()
                    .find(|s| s.name == "Token")
                    .unwrap()
                    .declaration
            }
        };
        let make = |argument| match kind {
            0 => Value::Record {
                declaration: token,
                arguments: vec![argument],
                fields: vec![],
            },
            1 => Value::Variant {
                declaration: token,
                arguments: vec![argument],
                index: 0,
                fields: vec![],
            },
            _ => Value::Newtype {
                ty: Term::Nominal {
                    declaration: token,
                    arguments: vec![argument],
                },
                value: Box::new(Value::Bool(true)),
            },
        };
        let value = make(symbolic);
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_abstract_value_parameters(&[(parameter, domain.clone())])
            .unwrap();
        let opaque = Value::AbstractValue {
            declaration: parameter,
            ty: domain,
        };
        for entry in ["keep", "forward"] {
            assert_eq!(
                vm.invoke(
                    support::id(&package, entry),
                    vec![opaque.clone(), value.clone()]
                )
                .unwrap(),
                value
            );
            let concrete = make(Term::Bool(true));
            assert!(
                vm.invoke(
                    support::id(&package, entry),
                    vec![opaque.clone(), concrete.clone()]
                )
                .is_err()
            );
            assert_eq!(
                vm.invoke(
                    support::id(&package, entry),
                    vec![Value::Bool(true), concrete.clone()]
                )
                .unwrap(),
                concrete
            );
        }
    }
}

#[test]
fn aggregate_comptime_inputs_preserve_symbolic_leaves_in_dependent_identity() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{DeclarationId, Primitive, Sort, Term};
    let package=support::compile_with_tuple("struct Packet where state:Tuple<bool,bool> {}
        comptime fn keep(comptime state:Tuple<bool,bool>,value:Packet<state>)->Packet<state> {value}
        comptime fn forward(comptime state:Tuple<bool,bool>,value:Packet<state>)->Packet<state> {keep(state,value)}");
    let parameter = DeclarationId {
        package: 99,
        local: 1,
    };
    let domain = Term::Primitive(Primitive::Bool);
    let symbolic = Term::Rigid {
        declaration: parameter,
        sort: Sort::ComptimeValue,
    };
    let packet = package
        .structs
        .iter()
        .find(|s| s.name == "Packet")
        .unwrap()
        .declaration;
    let value = Value::Record {
        declaration: packet,
        arguments: vec![Term::TupleValue(vec![symbolic.clone(), Term::Bool(false)])],
        fields: vec![],
    };
    let input = Value::Tuple(vec![
        Value::AbstractValue {
            declaration: parameter,
            ty: domain.clone(),
        },
        Value::Bool(false),
    ]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_abstract_value_parameters(&[(parameter, domain)])
        .unwrap();
    for entry in ["keep", "forward"] {
        assert_eq!(
            vm.invoke(
                support::id(&package, entry),
                vec![input.clone(), value.clone()]
            )
            .unwrap(),
            value
        );
        let wrong = Value::Record {
            declaration: packet,
            arguments: vec![Term::TupleValue(vec![symbolic.clone(), Term::Bool(true)])],
            fields: vec![],
        };
        assert!(
            vm.invoke(support::id(&package, entry), vec![input.clone(), wrong])
                .is_err()
        );
    }
}

#[test]
fn nominal_comptime_inputs_bind_dependent_signatures_without_erasing_payloads() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{DeclarationId, Primitive, Sort, Term};
    for (kind, definition) in [
        (0, "struct Key {flag:bool}"),
        (1, "enum Key {flag(bool)}"),
        (2, "newtype Key = bool;"),
    ] {
        let package=support::compile(&format!("{definition}
            struct Packet where key:Key {{}}
            comptime fn keep(comptime key:Key,value:Packet<key>)->Packet<key> {{value}}
            comptime fn forward(comptime key:Key,value:Packet<key>)->Packet<key> {{keep(key,value)}}"));
        let key = match kind {
            0 => {
                package
                    .structs
                    .iter()
                    .find(|s| s.name == "Key")
                    .unwrap()
                    .declaration
            }
            1 => {
                package
                    .enums
                    .iter()
                    .find(|s| s.name == "Key")
                    .unwrap()
                    .declaration
            }
            _ => {
                package
                    .newtypes
                    .iter()
                    .find(|s| s.name == "Key")
                    .unwrap()
                    .declaration
            }
        };
        let packet = package
            .structs
            .iter()
            .find(|s| s.name == "Packet")
            .unwrap()
            .declaration;
        let parameter = DeclarationId {
            package: 99,
            local: 1,
        };
        let domain = Term::Primitive(Primitive::Bool);
        let key_type = Term::Nominal {
            declaration: key,
            arguments: vec![],
        };
        for symbolic in [false, true] {
            let leaf = if symbolic {
                Value::AbstractValue {
                    declaration: parameter,
                    ty: domain.clone(),
                }
            } else {
                Value::Bool(true)
            };
            let input = match kind {
                0 => Value::Record {
                    declaration: key,
                    arguments: vec![],
                    fields: vec![leaf],
                },
                1 => Value::Variant {
                    declaration: key,
                    arguments: vec![],
                    index: 0,
                    fields: vec![leaf],
                },
                _ => Value::Newtype {
                    ty: key_type.clone(),
                    value: Box::new(leaf),
                },
            };
            let leaf = if symbolic {
                Term::Rigid {
                    declaration: parameter,
                    sort: Sort::ComptimeValue,
                }
            } else {
                Term::Bool(true)
            };
            let make = |leaf| Value::Record {
                declaration: packet,
                arguments: vec![Term::NominalValue {
                    ty: Box::new(key_type.clone()),
                    variant: (kind == 1).then_some(0),
                    fields: vec![leaf],
                }],
                fields: vec![],
            };
            let value = make(leaf);
            let mut vm = Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .with_abstract_value_parameters(&[(parameter, domain.clone())])
                .unwrap();
            for entry in ["keep", "forward"] {
                assert_eq!(
                    vm.invoke(
                        support::id(&package, entry),
                        vec![input.clone(), value.clone()]
                    )
                    .unwrap(),
                    value
                );
                assert!(
                    vm.invoke(
                        support::id(&package, entry),
                        vec![input.clone(), make(Term::Bool(false))]
                    )
                    .is_err()
                );
            }
        }
    }
}

#[test]
fn indexed_nominal_comptime_inputs_keep_their_own_symbolic_environment() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{DeclarationId, Primitive, Sort, Term};
    let package=support::compile("struct Key where enabled:bool {flag:bool}
        struct Packet where enabled:bool,key:Key<enabled> {}
        comptime fn keep(comptime enabled:bool,comptime key:Key<enabled>,value:Packet<enabled,key>)->Packet<enabled,key> {value}
        comptime fn forward(comptime enabled:bool,comptime key:Key<enabled>,value:Packet<enabled,key>)->Packet<enabled,key> {keep(enabled,key,value)}");
    let key = package
        .structs
        .iter()
        .find(|s| s.name == "Key")
        .unwrap()
        .declaration;
    let packet = package
        .structs
        .iter()
        .find(|s| s.name == "Packet")
        .unwrap()
        .declaration;
    let parameter = DeclarationId {
        package: 99,
        local: 1,
    };
    let domain = Term::Primitive(Primitive::Bool);
    let symbol = Term::Rigid {
        declaration: parameter,
        sort: Sort::ComptimeValue,
    };
    let key_type = Term::Nominal {
        declaration: key,
        arguments: vec![symbol.clone()],
    };
    let key_value = Value::Record {
        declaration: key,
        arguments: vec![symbol.clone()],
        fields: vec![Value::Bool(true)],
    };
    let key_term = Term::NominalValue {
        ty: Box::new(key_type),
        variant: None,
        fields: vec![Term::Bool(true)],
    };
    let value = Value::Record {
        declaration: packet,
        arguments: vec![symbol, key_term],
        fields: vec![],
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_abstract_value_parameters(&[(parameter, domain.clone())])
        .unwrap();
    for entry in ["keep", "forward"] {
        assert_eq!(
            vm.invoke(
                support::id(&package, entry),
                vec![
                    Value::AbstractValue {
                        declaration: parameter,
                        ty: domain.clone()
                    },
                    key_value.clone(),
                    value.clone()
                ]
            )
            .unwrap(),
            value
        );
    }
}

#[test]
fn explicit_function_instances_accept_only_authorized_symbolic_nominal_inputs() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{DeclarationId, Primitive, Sort, Term};
    let package = support::compile(
        "struct Key {flag:bool}
        struct Packet where key:Key {}
        comptime fn keep(value:Packet<key>)->Packet<key> where key:Key {value}
        comptime fn forward(value:Packet<key>)->Packet<key> where key:Key {keep<key>(value)}
        comptime fn choose()->bool where key:Key {if key.flag {true} else {false}}",
    );
    let key = package
        .structs
        .iter()
        .find(|s| s.name == "Key")
        .unwrap()
        .declaration;
    let packet = package
        .structs
        .iter()
        .find(|s| s.name == "Packet")
        .unwrap()
        .declaration;
    let parameter = DeclarationId {
        package: 99,
        local: 1,
    };
    let symbol = Term::Rigid {
        declaration: parameter,
        sort: Sort::ComptimeValue,
    };
    let input = |field| Term::NominalValue {
        ty: Box::new(Term::Nominal {
            declaration: key,
            arguments: vec![],
        }),
        variant: None,
        fields: vec![field],
    };
    let payload = |index| Value::Record {
        declaration: packet,
        arguments: vec![index],
        fields: vec![],
    };
    let index = input(symbol);
    let value = payload(index.clone());
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_abstract_value_parameters(&[(parameter, Term::Primitive(Primitive::Bool))])
        .unwrap();
    assert_eq!(
        vm.invoke_instantiated(support::id(&package, "choose"), vec![index.clone()], vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::AbstractValueUnresolved
    );
    for flag in [false, true] {
        assert_eq!(
            vm.invoke_instantiated(
                support::id(&package, "choose"),
                vec![input(Term::Bool(flag))],
                vec![]
            )
            .unwrap(),
            Value::Bool(flag)
        );
    }
    for entry in ["keep", "forward"] {
        assert_eq!(
            vm.invoke_instantiated(
                support::id(&package, entry),
                vec![index.clone()],
                vec![value.clone()]
            )
            .unwrap(),
            value
        );
        assert!(
            Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .invoke_instantiated(
                    support::id(&package, entry),
                    vec![index.clone()],
                    vec![value.clone()]
                )
                .is_err()
        );
        let wrong = input(Term::Char('x'));
        assert!(
            vm.invoke_instantiated(
                support::id(&package, entry),
                vec![wrong.clone()],
                vec![payload(wrong)]
            )
            .is_err()
        );
        let concrete = input(Term::Bool(true));
        assert_eq!(
            vm.invoke_instantiated(
                support::id(&package, entry),
                vec![concrete.clone()],
                vec![payload(concrete.clone())]
            )
            .unwrap(),
            payload(concrete)
        );
    }
}

#[test]
fn explicit_function_instances_preserve_authorized_type_parameters() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{DeclarationId, Primitive, Sort, Term};
    let package = support::compile("comptime fn Select()->.Type'builtin where T {T}");
    let parameter = DeclarationId {
        package: 99,
        local: 1,
    };
    let ty = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let entry = support::id(&package, "Select");
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_abstract_type_parameters(&[parameter])
        .unwrap();
    assert_eq!(
        vm.invoke_instantiated(entry, vec![ty.clone()], vec![])
            .unwrap(),
        Value::TypeParameter(parameter)
    );
    assert!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke_instantiated(entry, vec![ty], vec![])
            .is_err()
    );
    assert_eq!(
        vm.invoke_instantiated(entry, vec![Term::Primitive(Primitive::Bool)], vec![])
            .unwrap(),
        Value::Type(Term::Primitive(Primitive::Bool))
    );
}

#[test]
fn dependent_comptime_closure_environments_preserve_symbolic_instance_identity() {
    use comptime_interpretor::{Budget, Interpreter};
    use nir::{DeclarationId, Primitive, Sort, Term};
    let package=support::compile("struct Token where enabled:bool {}
        comptime fn capture(comptime enabled:bool,value:Token<enabled>)->%{value:Token<enabled>} comptime fn()->Token<enabled> {|| value}
        comptime fn call(comptime enabled:bool,callback:%{value:Token<enabled>} comptime fn()->Token<enabled>)->Token<enabled> {callback()}");
    let parameter = DeclarationId {
        package: 99,
        local: 1,
    };
    let domain = Term::Primitive(Primitive::Bool);
    let opaque = Value::AbstractValue {
        declaration: parameter,
        ty: domain.clone(),
    };
    let token = package
        .structs
        .iter()
        .find(|s| s.name == "Token")
        .unwrap()
        .declaration;
    let value = Value::Record {
        declaration: token,
        arguments: vec![Term::Rigid {
            declaration: parameter,
            sort: Sort::ComptimeValue,
        }],
        fields: vec![],
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_abstract_value_parameters(&[(parameter, domain)])
        .unwrap();
    let callback = vm
        .invoke(
            support::id(&package, "capture"),
            vec![opaque.clone(), value.clone()],
        )
        .unwrap();
    assert!(
        vm.invoke(
            support::id(&package, "call"),
            vec![Value::Bool(true), callback.clone()]
        )
        .is_err()
    );
    assert_eq!(
        vm.invoke(support::id(&package, "call"), vec![opaque, callback])
            .unwrap(),
        value
    );
}

#[test]
fn generic_required_type_calls_materialize_distinct_runtime_instances() {
    let source = "struct Token where enabled:bool {pub fn active(self)->bool {enabled}}
        comptime fn Build(enabled:bool)->.Type'builtin {Token<enabled>}
        fn keep(value:Build(enabled))->Build(enabled) where enabled:bool {value}
        fn run()->bool {
            let on=keep<true>(Token<true> {});
            let off=keep<false>(Token<false> {});
            on.active() and not off.active()
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let keep = support::id(&package, "keep");
    for flag in [false, true] {
        assert!(
            graph
                .functions
                .iter()
                .any(|instance| instance.key.declaration == keep
                    && instance.key.arguments.to_vec() == vec![nir::Term::Bool(flag)])
        );
    }
    let invalid = source.replace("keep<false>(Token<false>", "keep<false>(Token<true>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn checked_execution_selects_copy_for_nominal_value_indices() {
    for (definition, value) in [
        ("struct Config {flag:bool}", "Config{flag:true}"),
        ("enum Config {off,on(bool)}", "Config.on(true)"),
        ("newtype Config = bool;", "true.as(Config)"),
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             {definition}
             struct Packet where config:Config {{value:bool}}
             impl Duplicate for Packet<config> where config:Config {{}}
             fn read(value:Packet<config>)->bool where config:Config {{value.value}}
             fn run()->bool {{
                 let packet=Packet<{value}>{{value:true}};
                 read(packet) and read(packet)
             }}"
        );
        let package = support::compile(&source);
        for inline in [false, true] {
            let result = if inline {
                luna::execution::interpret_inline_with_protocols_in_package(
                    &package,
                    0,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000,
                )
            } else {
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000,
                )
            };
            assert_eq!(result.unwrap(), Value::Bool(true), "{definition}");
        }
        let without_copy = support::compile(&source.replace(
            "impl Duplicate for Packet<config> where config:Config {}",
            "",
        ));
        assert!(
            luna::execution::interpret_with_protocols(
                &without_copy,
                "run",
                "Duplicate",
                "Retire",
                100_000,
            )
            .is_err(),
            "layout evidence must not invent Copy: {definition}"
        );
    }
}

#[test]
fn checked_execution_runs_drop_for_nominal_value_indices() {
    for (definition, value) in [
        ("struct Config {flag:bool}", "Config{flag:true}"),
        ("enum Config {off,on(bool)}", "Config.on(true)"),
        ("newtype Config = bool;", "true.as(Config)"),
    ] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             {definition}
             struct Ticket where config:Config {{closed:*bool}}
             impl Retire for Ticket<config> where config:Config {{
                 fn drop(*self) {{self.closed.*.* = true;}}
             }}
             inline fn own(closed:*bool) {{
                 let ticket=Ticket<{value}>{{closed:closed}};
             }}
             fn run()->bool {{let closed=false; own(closed.ref); closed}}"
        );
        let package = support::compile(&source);
        for inline in [false, true] {
            let result = if inline {
                luna::execution::interpret_inline_with_protocols_in_package(
                    &package,
                    0,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000,
                )
            } else {
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000,
                )
            };
            assert_eq!(result.unwrap(), Value::Bool(true), "{definition}");
        }
    }
}

#[test]
fn type_valued_dependent_indices_support_construction_and_field_access() {
    for body in [
        "let key=Key<Type,bool>{payload:true}; key.payload",
        "let key=Key<Type,bool>{}; key.payload",
        "let outer=Outer<Type,bool>{inner:Key<Type,bool>{}}; outer.inner.payload",
        "var key=Key<Type,bool>{payload:false}; key.payload=true; key.payload",
        "var key=Key<Type,bool>{payload:false}; let pointer=key.ref; pointer.*.payload=true; pointer.*.payload",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Key where T,value:T {{ payload:bool=true }}
            struct Outer where T,value:T {{ inner:Key<T,value> }}
            fn run()->bool {{ {body} }}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "{body}"
        );
    }
}

#[test]
fn dependent_layouts_do_not_admit_type_values_in_non_type_domains() {
    for inputs in ["bool,bool", "Type,true"] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Key where T,value:T {{ payload:bool }}
            fn run()->bool {{ let key=Key<{inputs}>{{payload:true}}; key.payload }}"
        );
        assert!(
            support::compile_with_protocols(&source, &Default::default()).is_err(),
            "{inputs}"
        );
    }
}

#[test]
fn type_valued_dependent_indices_support_enum_payloads_and_newtype_conversion() {
    let source = "typealias Type=.Type'builtin;
        enum Choice where T,value:T {none,some(bool)}
        newtype Wrapped<T,value:T> = bool;
        fn run()->bool {
            let item:Choice<Type,bool> = .some(true);
            let result = item match {.none=>false,.some(value)=>value};
            result.as(Wrapped<Type,bool>).as(bool)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_dependent_indices_support_named_enum_defaults() {
    let source = "typealias Type=.Type'builtin;
        enum Packet where T,value:T {data {flag:bool=true}}
        fn run()->bool {let packet:Packet<Type,bool> = .data {}; let .data {flag}=packet; flag}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_specialize_runtime_function_instances() {
    for (prefix, parameter, input) in [
        ("", "", ""),
        ("inline ", "", ""),
        ("", "comptime enabled:bool,", "true,"),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Key where T,value:T {{payload:bool}}
            {prefix}fn inspect({parameter}key:Key<T,value>)->bool
                where T,value:T, requires Key<T,value> == Key<T,value> {{key.payload}}
            fn run()->bool {{inspect<Type,bool>({input}Key<Type,bool>{{payload:true}})}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn type_valued_indices_select_inherent_implementations() {
    let source = "typealias Type=.Type'builtin;
        struct Key where T,value:T {payload:bool}
        impl Key<T,value> where T,value:T {
            fn read(self)->bool {self.payload}
        }
        fn run()->bool {Key<Type,bool>{payload:true}.read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_select_trait_implementations() {
    let source = "typealias Type=.Type'builtin;
        struct Key where T,value:T {payload:bool}
        trait Read {fn read(self)->bool;}
        impl Read for Key<T,value> where T,value:T {
            fn read(self)->bool {self.payload}
        }
        fn run()->bool {Key<Type,bool>{payload:true}.as(Read).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_select_parameterized_trait_implementations() {
    let source = "typealias Type=.Type'builtin;
        struct Key where T,value:T {payload:bool}
        trait Read where T,value:T {fn read(self)->bool;}
        impl Read<T,value> for Key<T,value> where T,value:T {
            fn read(self)->bool {self.payload}
        }
        fn run()->bool {Key<Type,bool>{payload:true}.as(Read<Type,bool>).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("as(Read<Type,bool>)", "as(Read<Type,char>)");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn type_valued_indices_preserve_parent_trait_method_sources() {
    let source = "typealias Type=.Type'builtin;
        struct Key where T,value:T {payload:bool}
        trait Read where T,value:T {fn read(self)->bool;}
        trait Child :- Read<T,value> where T,value:T {}
        impl Read<T,value> for Key<T,value> where T,value:T {
            fn read(self)->bool {self.payload}
        }
        impl Child<T,value> for Key<T,value> where T,value:T {}
        fn through_bound(key:K)->bool where K:-Child<Type,bool> {key.read()}
        fn run()->bool {
            Key<Type,bool>{payload:true}.as(Child<Type,bool>).read()
                and through_bound(Key<Type,bool>{payload:true})
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_trait_indices_preserve_runtime_preconditions() {
    let source = "typealias Type=.Type'builtin;
        struct Key where T,value:T {payload:bool}
        trait Read where T,value:T {fn read(self,allow:bool)->bool where requires allow;}
        impl Read<T,value> for Key<T,value> where T,value:T {
            fn read(self,allow:bool)->bool {self.payload}
        }
        fn run()->bool {Key<Type,bool>{payload:true}.as(Read<Type,bool>).read(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(".read(true)", ".read(false)");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn type_valued_trait_indices_preserve_staged_preconditions() {
    let source = "typealias Type=.Type'builtin;
        struct Key where T,value:T {payload:bool}
        trait Read where T,value:T {fn read(self,comptime allow:bool)->bool where requires allow;}
        impl Read<T,value> for Key<T,value> where T,value:T {
            fn read(self,comptime allow:bool)->bool {self.payload}
        }
        fn run()->bool {Key<Type,bool>{payload:true}.as(Read<Type,bool>).read(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(".read(true)", ".read(false)");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
}

#[test]
fn type_valued_trait_indices_preserve_pointer_method_contracts() {
    let source = "typealias Type=.Type'builtin;
        struct Key where T,value:T {payload:bool}
        trait Read where T,value:T {fn read(self,allow:bool)->bool where requires allow;}
        impl Read<T,value> for *Key<T,value> where T,value:T {
            fn read(self,allow:bool)->bool {self.*.payload}
        }
        fn run()->bool {
            let key=Key<Type,bool>{payload:true};
            key.ref.as(Read<Type,bool>).read(true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_preserve_associated_type_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        trait Shape where T,index:T {assoc Output:Type=Tagged<T,index>;}
        struct Holder {}
        impl Shape<Type,bool> for Holder {}
        fn run()->bool {
            let value:Holder.as(Shape<Type,bool>).Output=Tagged<Type,bool>{value:true};
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("Output=Tagged<Type,bool>", "Output=Tagged<Type,Type>");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn type_valued_indices_preserve_generic_associated_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        trait Shape where T,index:T {assoc Output:Type=Tagged<T,index>;}
        struct Holder where T,index:T {}
        impl Shape<T,index> for Holder<T,index> where T,index:T {}
        fn run()->bool {
            let value:Holder<Type,bool>.as(Shape<Type,bool>).Output=Tagged<Type,bool>{value:true};
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let inherited = source
        .replace(
            "struct Holder",
            "trait Child :- Shape<T,index> where T,index:T {} struct Holder",
        )
        .replace(
            "fn run",
            "impl Child<T,index> for Holder<T,index> where T,index:T {} fn run",
        )
        .replace("Holder<Type,bool>.as(Shape", "Holder<Type,bool>.as(Child");
    assert_eq!(
        support::run_iir(&inherited, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn type_valued_indices_preserve_associated_constants() {
    let source = "typealias Type=.Type'builtin;
        trait Value where T,index:T {assoc data:T=index;}
        struct Holder where T,index:T {}
        impl Value<T,index> for Holder<T,index> where T,index:T {}
        fn run()->bool {
            let value:Holder<Type,bool>.as(Value<Type,bool>).data=true;
            value and Holder<bool,true>.as(Value<bool,true>).data
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_preserve_chained_associated_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        trait Shape where T,index:T {
            assoc Base:Type=Tagged<T,index>;
            assoc Output:Type=Self.as(Shape<T,index>).Base;
        }
        struct Holder {}
        impl Shape<Type,bool> for Holder {}
        fn run()->bool {
            let value:Holder.as(Shape<Type,bool>).Output=Tagged<Type,bool>{value:true};
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let overridden = source
        .replace(
            "impl Shape<Type,bool> for Holder {}",
            "impl Shape<Type,bool> for Holder {assoc Base:Type=Tagged<Type,Type>;}",
        )
        .replace("Output=Tagged<Type,bool>", "Output=Tagged<Type,Type>");
    assert_eq!(
        support::run_iir(&overridden, "run", vec![]),
        Value::Bool(true)
    );
    let wrong = overridden.replace("Output=Tagged<Type,Type>", "Output=Tagged<Type,bool>");
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn type_valued_indices_preserve_associated_default_method_signatures() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        trait Shape where T,index:T {
            assoc Output:Type=Tagged<T,index>;
            fn keep(value:Self.as(Shape<T,index>).Output)->Self.as(Shape<T,index>).Output {value}
        }
        struct Holder {}
        impl Shape<Type,bool> for Holder {}
        fn run()->bool {
            Holder.as(Shape<Type,bool>).keep(Tagged<Type,bool>{value:true}).value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let overridden = source.replace("impl Shape<Type,bool> for Holder {}", "impl Shape<Type,bool> for Holder {fn keep(value:Tagged<Type,bool>)->Tagged<Type,bool>{value}}");
    assert_eq!(
        support::run_iir(&overridden, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn type_valued_indices_preserve_overridden_method_preconditions() {
    let source = "typealias Type=.Type'builtin;
        trait Read where T,index:T {fn read(allow:bool)->bool where requires allow;}
        struct Holder {}
        impl Read<Type,bool> for Holder {fn read(allow:bool)->bool {true}}
        fn run()->bool {Holder.as(Read<Type,bool>).read(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace(".read(true)", ".read(false)");
    let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
    assert!(error.contains("precondition"), "{error}");
    let stronger = source.replace(
        "fn read(allow:bool)->bool {true}",
        "fn read(allow:bool)->bool where requires not allow {true}",
    );
    assert!(support::compile_with_protocols(&stronger, &Default::default()).is_err());
}

#[test]
fn type_valued_indices_preserve_associated_public_signatures() {
    let source = "typealias Type=.Type'builtin;
        trait Shape where T,index:T {assoc Output:Type=bool;}
        struct Holder {}
        impl Shape<Type,bool> for Holder {}
        pub fn keep(value:Holder.as(Shape<Type,bool>).Output)->Holder.as(Shape<Type,bool>).Output {value}
        fn run()->bool {keep(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let inherited = source
        .replace(
            "struct Holder",
            "trait Child :- Shape<T,index> where T,index:T {} struct Holder",
        )
        .replace(
            "pub fn keep",
            "impl Child<Type,bool> for Holder {} pub fn keep",
        )
        .replace("Holder.as(Shape<Type,bool>)", "Holder.as(Child<Type,bool>)");
    assert_eq!(
        support::run_iir(&inherited, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn type_valued_indices_preserve_declared_parent_associated_domains() {
    let source = "typealias Type=.Type'builtin;
        trait Shape where T,index:T {assoc Output:Type=bool;}
        trait Child :- Shape<Type,bool> {
            fn keep(value:Self.as(Shape<Type,bool>).Output)->Self.as(Shape<Type,bool>).Output {value}
        }
        struct Holder {}
        impl Shape<Type,bool> for Holder {}
        impl Child for Holder {}
        fn run()->bool {Holder.as(Child).keep(true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for invalid in [
        source.replace("impl Shape<Type,bool> for Holder {}", ""),
        source.replace(
            "impl Shape<Type,bool> for Holder {}",
            "impl Shape<Type,Type> for Holder {}",
        ),
        source.replace(".keep(true)", ".keep('x')"),
    ] {
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn dependent_type_indices_preserve_ordinary_type_requirements() {
    let source = "typealias Type=.Type'builtin;
        fn keep(comptime U:Type,value:U)->U where T,index:T, requires U == bool {value}
        fn run()->bool {keep<Type,bool>(bool,true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("(bool,true)", "('x''type,'x')");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn dependent_type_indices_preserve_ordinary_capabilities() {
    let source = "typealias Type=.Type'builtin;
        trait Allowed {} impl Allowed for bool {}
        fn keep(comptime U:Type,value:U)->U where T,index:T, requires U :- Allowed {value}
        fn run()->bool {keep<Type,bool>(bool,true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let invalid = source.replace("impl Allowed for bool {}", "");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn dependent_type_indices_forward_ordinary_constraints_without_capture() {
    for requirement in ["U == bool", "U :- Allowed"] {
        let source = format!(
            "typealias Type=.Type'builtin;
            trait Allowed {{}} impl Allowed for bool {{}}
            fn keep(comptime U:Type,value:U)->U
            where T,index:T, requires {requirement}
            {{value}}
            fn forward(comptime U:Type,value:U)->U
            where T,index:T, requires {requirement}
            {{keep<T,index>(U,value)}}
            fn run()->bool {{forward<Type,bool>(bool,true)}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        let scoped = source.replace("{keep<T,index>(U,value)}", "{keep<Type,U>(U,value)}");
        assert_ne!(scoped, source);
        assert_eq!(support::run_iir(&scoped, "run", vec![]), Value::Bool(true));
        let invalid = source.replace(
            "forward<Type,bool>(bool,true)",
            "forward<Type,bool>('x''type,'x')",
        );
        assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
    }
}

#[test]
fn type_valued_indices_preserve_enum_constructor_contexts() {
    let source = "typealias Type=.Type'builtin;
        enum Choice where T,index:T {none,some(bool)}
        fn make()->Choice<Type,bool> {.some(true)}
        fn run()->bool {make() match {.some(value)=>value,.none=>false}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_infer_enum_payloads_from_partial_contexts() {
    let source = "typealias Type=.Type'builtin;
        enum Choice where T,index:T,U {none,some(U)}
        fn keep(value:Choice<Type,bool,U>)->Choice<Type,bool,U> where U {value}
        fn run()->bool {let result=keep(.some(true));result match {.some(value)=>value,.none=>false}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let spread = source.replace(".some(true)", ".some(...(true,))");
    assert_eq!(
        support::run_iir_with_tuple(&spread, "run", vec![]),
        Value::Bool(true)
    );
    let record = source
        .replace("some(U)", "some {value:U,flag:bool=true}")
        .replace(".some(true)", ".some {value:true}")
        .replace(".some(value)=>value", ".some {value,flag}=>value and flag");
    assert_eq!(support::run_iir(&record, "run", vec![]), Value::Bool(true));
    let wrong_index = source.replace(
        "keep(.some(true))",
        "keep(Choice<Type,Type,bool>.some(true))",
    );
    assert!(support::compile_with_protocols(&wrong_index, &Default::default()).is_err());
}

#[test]
fn type_valued_indices_complete_dependent_nominal_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T,.copy:T=index {value:bool}
        fn run()->bool {let value:Tagged<Type,bool,.copy=bool> = Tagged<Type,bool>{value:true};value.value}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let wrong = source.replace(".copy=bool", ".copy=Type");
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn type_valued_indices_complete_function_type_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        fn read(value:Tagged<T,index>)->bool where T,index:T,.U:Type=bool {value.value}
        fn run()->bool {read<Type,bool>(Tagged<Type,bool>{value:true})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_complete_inherited_function_type_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {
            value:bool,
            fn read(self)->bool where .U:Type=bool {self.value}
        }
        fn run()->bool {Tagged<Type,bool>{value:true}.read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inherited_type_defaults_preserve_indices_and_allow_overrides() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {
            value:bool,
            fn select(self,other:U)->U where .U:Type=Tagged<T,index> {other}
        }
        fn run()->bool {
            let selected=Tagged<Type,bool>{value:false}.select(Tagged<Type,bool>{value:true});
            selected.value and Tagged<Type,bool>{value:false}.select<.U=bool>(true)
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let wrong = source.replace("select(Tagged<Type,bool>", "select(Tagged<Type,Type>");
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    let scalar = source.replace("Tagged<Type,bool>", "Tagged<bool,true>");
    assert_eq!(support::run_iir(&scalar, "run", vec![]), Value::Bool(true));
}

#[test]
fn type_valued_indices_infer_function_inputs_before_completing_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        fn read(value:Tagged<T,index>)->bool where T,index:T,.U:Type=bool {value.value}
        fn run()->bool {read(Tagged<Type,bool>{value:true})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let deferred = source.replace(
        "fn run()->bool {read(",
        "fn forward(value:defer Tagged<Type,bool>)->bool {read(value)} fn run()->bool {forward(",
    );
    assert_eq!(
        support::run_iir(&deferred, "run", vec![]),
        Value::Bool(true)
    );
    let scalar = source.replace("Tagged<Type,bool>", "Tagged<bool,true>");
    assert_eq!(support::run_iir(&scalar, "run", vec![]), Value::Bool(true));
}

#[test]
fn dependent_call_staging_keeps_shared_index_constraints() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        fn same(a:Tagged<T,index>,b:Tagged<T,index>)->bool where T,index:T {a.value and b.value}
        fn run()->bool {same(Tagged<Type,bool>{value:true},Tagged<Type,bool>{value:true})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let wrong = source.replace(
        ",Tagged<Type,bool>{value:true}",
        ",Tagged<Type,Type>{value:true}",
    );
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    let malformed = source.replace("Tagged<Type,bool>", "Tagged<bool,Type>");
    assert!(support::compile_with_protocols(&malformed, &Default::default()).is_err());
    let constrained = source.replace(
        "where T,index:T {a.value",
        "where T,index:T, requires Tagged<T,index> == Tagged<Type,bool> {a.value",
    );
    assert_eq!(
        support::run_iir(&constrained, "run", vec![]),
        Value::Bool(true)
    );
    let wrong_requirement = constrained.replace(
        "same(Tagged<Type,bool>{value:true},Tagged<Type,bool>{value:true})",
        "same(Tagged<Type,Type>{value:true},Tagged<Type,Type>{value:true})",
    );
    assert!(support::compile_with_protocols(&wrong_requirement, &Default::default()).is_err());
}

#[test]
fn dependent_call_staging_combines_ordinary_type_arguments() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        fn select(comptime R:Type,tag:Tagged<T,index>,value:R)->R where T,index:T {value}
        fn run()->bool {select(bool,Tagged<Type,bool>{value:true},true)}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn dependent_effect_inputs_infer_type_valued_indices() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        effect read(value:Tagged<T,index>)->bool where T,index:T;
        fn run()->bool {read(Tagged<Type,bool>{value:true})# {read<Type,bool>(value)=>value.value,}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let wrong = source.replace("read<Type,bool>(value)", "read<Type,Type>(value)");
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    let explicit = source.replace("{read(Tagged", "{read<Type,bool>(Tagged");
    assert_eq!(
        support::run_iir(&explicit, "run", vec![]),
        Value::Bool(true)
    );
    let group = source
        .replace(
            "effect read(value:Tagged<T,index>)->bool where T,index:T;",
            "effect enum Channel where T,index:T {read(value:Tagged<T,index>)->bool,}",
        )
        .replace("{read(Tagged", "{Channel.read(Tagged")
        .replace("read<Type,bool>(value)", "Channel<Type,bool>.read(value)");
    assert_eq!(support::run_iir(&group, "run", vec![]), Value::Bool(true));
    let scalar = source
        .replace("Tagged<Type,bool>", "Tagged<bool,true>")
        .replace("read<Type,bool>", "read<bool,true>");
    assert_eq!(support::run_iir(&scalar, "run", vec![]), Value::Bool(true));
    let constrained = source.replace(
        "where T,index:T;",
        "where T,index:T, requires Tagged<T,index> == Tagged<Type,bool>;",
    );
    assert_eq!(
        support::run_iir(&constrained, "run", vec![]),
        Value::Bool(true)
    );
    let wrong_requirement = constrained
        .replace("read(Tagged<Type,bool>", "read(Tagged<Type,Type>")
        .replace("read<Type,bool>(value)", "read<Type,Type>(value)");
    assert!(support::compile_with_protocols(&wrong_requirement, &Default::default()).is_err());
}

#[test]
fn dependent_effect_inputs_use_result_context_and_group_defaults() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        effect enum Channel where T,index:T,.copy:T=index {read()->Tagged<T,index>,}
        fn request()->#[Channel<Type,bool>.read] Tagged<Type,bool> {Channel.read()#}
        fn run()->bool {
            let value=request()# {Channel<Type,bool>.read()=>Tagged<Type,bool>{value:true},};
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn dependent_generator_inputs_infer_type_valued_indices() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        comptime fn build(value:Tagged<T,index>)-><|-bool> where T,index:T {'(true)}
        fn run()->bool {~<build(Tagged<Type,bool>{value:true})>}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let defaults = source.replace(
        "where T,index:T {'(true)}",
        "where T,index:T,.copy:T=index {'(true)}",
    );
    assert_eq!(
        support::run_iir(&defaults, "run", vec![]),
        Value::Bool(true)
    );
    let code = source
        .replace("value:Tagged<T,index>", "code:<|-Tagged<T,index>>")
        .replace(
            "build(Tagged<Type,bool>{value:true})",
            "build('(Tagged<Type,bool>{value:true}))",
        );
    assert_eq!(support::run_iir(&code, "run", vec![]), Value::Bool(true));
}

#[test]
fn dependent_generator_defaults_preserve_generated_type_identity() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        comptime fn build(value:Tagged<T,index>)-><|-Tagged<T,copy>>
            where T,index:T,.copy:T=index {'(Tagged<T,copy>{value:true})}
        fn run()->bool {
            let value:Tagged<Type,bool> = ~<build(Tagged<Type,bool>{value:false})>;
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let wrong = source.replace("let value:Tagged<Type,bool>", "let value:Tagged<Type,Type>");
    assert!(support::compile_with_protocols(&wrong, &Default::default()).is_err());
    let scalar = source.replace("Tagged<Type,bool>", "Tagged<bool,true>");
    assert_eq!(support::run_iir(&scalar, "run", vec![]), Value::Bool(true));
}

#[test]
fn dependent_packet_inputs_supply_context_for_later_callbacks() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        fn packet()->Tuple<Tagged<Type,bool>> {(Tagged<Type,bool>{value:true},)}
        fn apply(value:Tagged<T,index>,action:fn(Tagged<T,index>)->bool)->bool
            where T,index:T {action(value)}
        fn run()->bool {apply(...packet(),|value| value.value)}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
    let method = source
        .replace(
            "fn apply(value",
            "struct Receiver {} impl Receiver {fn apply(self,value",
        )
        .replace("{action(value)}", "{action(value)}}")
        .replace("{apply(...", "{Receiver{}.apply(...");
    assert_eq!(
        support::run_iir_with_tuple(&method, "run", vec![]),
        Value::Bool(true)
    );
    let deferred = source.replace(
        "->bool\n            where",
        "->defer bool\n            where",
    );
    assert_eq!(
        support::run_iir_with_tuple(&deferred, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn dependent_packet_inference_preserves_producer_order_and_once_evaluation() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        fn before()->bool {true}
        fn after()->bool {true}
        fn packet()->Tuple<Tagged<Type,bool>> {(Tagged<Type,bool>{value:true},)}
        fn apply(first:bool,value:Tagged<T,index>,action:fn(Tagged<T,index>)->bool,last:bool)->bool
            where T,index:T {first and action(value) and last}
        fn run()->bool {apply(before(),...packet(),|value| value.value,after())}";
    let package = support::compile_with_tuple(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| {
            let comptime_interpretor::Event::EnterFunction(id) = event else {
                return None;
            };
            package
                .functions
                .iter()
                .find(|function| function.declaration == *id)
                .map(|function| function.name.as_str())
                .filter(|name| ["before", "packet", "after"].contains(name))
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, ["before", "packet", "after"]);
}

#[test]
fn dependent_effect_packets_preserve_exact_operation_identity() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        fn packet()->Tuple<Tagged<Type,bool>> {(Tagged<Type,bool>{value:true},)}
        effect read(value:Tagged<T,index>)->bool where T,index:T;
        fn run()->bool {read(...packet())# {read<Type,bool>(value)=>value.value,}}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
    let wrong = source.replace("read<Type,bool>(value)", "read<Type,Type>(value)");
    assert!(support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err());
}

#[test]
fn dependent_cmtt_data_hints_reach_enclosing_generators() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        comptime fn inspect(value:Tagged<T,index>)->Tagged<T,index> where T,index:T {value}
        comptime fn build(value:Tagged<T,index>)-><|-Tagged<T,index>> where T,index:T {
            '(Tagged<T,index>{value:true})
        }
        fn run()->bool {
            let value:Tagged<Type,bool> = ~<build(inspect(Tagged<Type,bool>{value:false}))>;
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn contextual_data_consumers_register_dependent_value_clauses() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        comptime fn inspect(code:<|-Tagged<T,index>>)->Tagged<T,index> where T,index:T {
            Tagged<T,index>{value:true}
        }
        fn run()->bool {
            const value=inspect('(Tagged<Type,bool>{value:false}));
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn contextual_data_calls_supply_generator_arguments_with_dependent_domains() {
    let source = "typealias Type=.Type'builtin;
        struct Tagged where T,index:T {value:bool}
        comptime fn inspect(code:<|-Tagged<T,index>>)->Tagged<T,index> where T,index:T {
            Tagged<T,index>{value:true}
        }
        comptime fn build(value:Tagged<T,index>)-><|-Tagged<T,index>> where T,index:T {
            '(Tagged<T,index>{value:true})
        }
        fn run()->bool {
            let value:Tagged<Type,bool> = ~<build(inspect('(Tagged<Type,bool>{value:false})))>;
            value.value
        }";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let wrong = source.replace("let value:Tagged<Type,bool>", "let value:Tagged<Type,Type>");
    assert!(support::compile_with_tuple_protocols(&wrong, &Default::default()).is_err());
}
