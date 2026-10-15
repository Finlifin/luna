#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{ErrorKind, Interpreter, Value};
use nir::{Primitive, Sort, Term};
use support::compile;

#[test]
fn enum_matches_supply_staged_defaults_and_derived_inputs() {
    let package = compile(
        "struct Settings {enabled:bool}
        enum Mode {off,on(Settings),}
        fn select(comptime flag:bool)->bool {flag}
        fn defaults(comptime mode:Mode,comptime .flag:bool=mode match {
            .on({enabled}) if enabled => true,
            _=>false,
        })->bool {flag}
        fn derived(comptime mode:Mode)->bool {
            let flag=mode match {.on({enabled})=>enabled,_=>false,};select(flag)
        }
        fn guarded(comptime value:bool)->bool {
            let flag=value match {matched if matched=>true,_=>false,};select(flag)
        }
        fn run()->bool {defaults(Mode.on(Settings {enabled:true}))
            and not defaults(Mode.on(Settings {enabled:false})) and not defaults(Mode.off)
            and derived(Mode.on(Settings {enabled:true})) and not derived(Mode.off)
            and guarded(true) and not guarded(false)}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn staged_enum_alternatives_roll_back_bindings_and_sequence_computations() {
    let package = compile(
        "enum Pair {pair(bool,bool),empty,}
        fn select(comptime flag:bool)->bool {flag}
        fn alternative(comptime pair:Pair)->bool {
            let chosen=pair match {
                .pair(chosen,false) or .pair(true,chosen)=>chosen,
                _=>false,
            };select(chosen)
        }
        fn sequence(comptime pair:Pair)->bool {
            let chosen=pair match {
                .pair(first,second) and (first and second) is true=>true,
                _=>false,
            };select(chosen)
        }
        fn run()->bool {alternative(Pair.pair(true,true))
            and not alternative(Pair.pair(false,false))
            and not alternative(Pair.empty)
            and sequence(Pair.pair(true,true)) and not sequence(Pair.pair(true,false))}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_fields_supply_staged_defaults_aliases_and_forwarded_construction() {
    let package=compile("struct Options {enabled:bool}
        struct Config {options:Options}
        fn select(comptime flag:bool)->bool {flag}
        fn defaults(comptime config:Config,comptime .flag:bool=config.options.enabled)->bool {flag}
        fn aliases(comptime config:Config)->bool {let options=config.options;let enabled=options.enabled;select(enabled)}
        fn rebuilt(comptime config:Config)->bool {let options=Options {enabled:not config.options.enabled};defaults(Config {options:options})}
        fn patterns(comptime config:Config)->bool {let {options:{enabled}}=config;select(enabled)}
        fn run()->bool {
            defaults(Config {options:Options {enabled:true}})
            and aliases(Config {options:Options {enabled:true}})
            and not aliases(Config {options:Options {enabled:false}})
            and rebuilt(Config {options:Options {enabled:false}})
            and patterns(Config {options:Options {enabled:true}})}");
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_staged_field_forwarding_rejects_runtime_writes_and_address_escape() {
    for body in [
        "config.enabled=false;select(config.enabled)",
        "let pointer=config.ref;select(config.enabled)",
    ] {
        let source = format!(
            "struct Config {{enabled:bool}} fn select(comptime flag:bool)->bool {{flag}}
            fn changed(comptime config:Config)->bool {{{body}}}
            fn run()->bool {{changed(Config {{enabled:true}})}}"
        );
        match support::compile_with_protocols(&source, &Default::default()) {
            Ok(package) => {
                let error = luna::instantiation::instantiate(&package, "run", 100_000)
                    .err()
                    .expect("stale staged value must be rejected");
                assert!(error.to_string().contains("staged"), "{body}: {error}");
            }
            Err(error) => assert!(
                error.contains("staged") || error.contains("comptime"),
                "{body}: {error}"
            ),
        }
    }
}

#[test]
fn nominal_layout_budget_only_expands_the_selected_variant_and_recovers() {
    let fields = std::iter::repeat_n("T", 128).collect::<Vec<_>>().join(",");
    let package = compile(&format!(
        "enum Layout where T {{small(T),large({fields}),}}"
    ));
    let declaration = package
        .enums
        .iter()
        .find(|definition| definition.name == "Layout")
        .unwrap()
        .declaration;
    let ty = Term::Nominal {
        declaration,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let mut fuel = 32;
    assert_eq!(
        nir::normal_forms::nominal_value_fields_budgeted(&package, &ty, Some(0), &mut fuel),
        Some(vec![Term::Primitive(Primitive::Bool)])
    );
    assert!(fuel < 32 && fuel > 0);
    assert!(
        nir::normal_forms::nominal_value_fields_budgeted(&package, &ty, Some(1), &mut 32).is_none()
    );
    assert!(
        nir::normal_forms::nominal_value_fields_budgeted(&package, &ty, Some(999), &mut 32)
            .is_none()
    );
    assert!(
        nir::normal_forms::nominal_value_fields_budgeted(&package, &ty, Some(0), &mut 32).is_some()
    );
    let wrong = Term::Nominal {
        declaration,
        arguments: vec![],
    };
    assert!(
        nir::normal_forms::nominal_value_fields_budgeted(&package, &wrong, Some(0), &mut 32)
            .is_none()
    );
}

#[test]
fn nominal_values_form_opaque_heads_and_reconstruct_through_patterns() {
    let package = compile(
        "typealias Type=.Type'builtin;typealias str=.str'builtin;
        struct Config {flag:bool}
        enum Mode where T {empty,configured(T),}
        pure comptime fn Tagged<T,value:Mode<T>>->Type {bool}
        typealias Enabled=Tagged<Config,Mode<Config>.configured(Config {flag:true})>;
        typealias Same=Tagged<Config,Mode<Config>.configured(Config {flag:true})>;
        typealias Disabled=Tagged<Config,Mode<Config>.configured(Config {flag:false})>;
        comptime fn equal()->bool {Enabled==Same}
        comptime fn read(ty:Type)->bool {ty match {
            Tagged<'Config,value> => value match {.configured(config)=>config.flag,_=>false,},
            _=>false,
        }}
        comptime fn run()->bool {equal() and read(Enabled) and not read(Disabled)}
        fn retained(value:Enabled)->Enabled {value}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "retained", 100_000).unwrap();
    Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert!(graph.to_string().contains("nominal-value"));
    assert!(graph.to_string().contains("normal-form"));
}

#[test]
fn nominal_normal_form_admission_rejects_forged_type_arguments_and_recovers() {
    let package = compile(
        "typealias Type=.Type'builtin;
        struct Config {flag:bool}
        enum Mode where T {empty,configured(T),}
        pure comptime fn Tagged<T,value:Mode<T>>->Type {bool}
        comptime fn Make()->Type {Tagged<Config,Mode<Config>.configured(Config {flag:true})>}
        comptime fn identity(ty:Type)->Type {ty}",
    );
    let valid = luna::invocation::invoke(&package, "Make", 100_000).unwrap();
    let Value::Type(Term::NormalForm {
        head,
        sort,
        arguments,
    }) = valid.clone()
    else {
        panic!()
    };
    let Term::NominalValue {
        ty,
        variant,
        fields,
    } = &arguments[1]
    else {
        panic!()
    };
    let inputs = [
        Term::NominalValue {
            ty: ty.clone(),
            variant: Some(100),
            fields: fields.clone(),
        },
        Term::NominalValue {
            ty: ty.clone(),
            variant: None,
            fields: fields.clone(),
        },
        Term::NominalValue {
            ty: ty.clone(),
            variant: *variant,
            fields: vec![],
        },
        Term::NominalValue {
            ty: ty.clone(),
            variant: *variant,
            fields: vec![Term::Bool(true)],
        },
    ];
    let identity = package
        .functions
        .iter()
        .find(|f| f.name == "identity")
        .unwrap()
        .declaration;
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    for input in inputs {
        let bad = Value::Type(Term::NormalForm {
            head,
            sort,
            arguments: vec![arguments[0].clone(), input],
        });
        assert_eq!(
            vm.invoke(identity, vec![bad]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert_eq!(vm.invoke(identity, vec![valid.clone()]).unwrap(), valid);
    }
    let descriptor = package
        .normal_forms
        .iter()
        .find(|metadata| metadata.declaration == head)
        .unwrap();
    assert!(descriptor.check_closed_arguments(&arguments).is_err());
    assert!(
        descriptor
            .check_closed_arguments_with_layouts(
                &arguments,
                |_| false,
                |ty, variant| nir::normal_forms::nominal_value_fields(&package, ty, variant)
            )
            .is_ok()
    );
}

#[test]
fn nominal_head_patterns_handle_records_variants_and_pointer_type_arguments() {
    let package = compile(
        "typealias Type=.Type'builtin;
        struct Config {flag:bool}
        enum Mode {empty,configured(Config),}
        enum Marker where T {token,}
        pure comptime fn Configured<value:Config>->Type {bool}
        pure comptime fn Selected<value:Mode>->Type {bool}
        pure comptime fn Marked<T,value:Marker<T>>->Type {bool}
        comptime fn inspect(ty:Type)->bool {ty match {
            Configured<{flag:true}> => true,
            Selected<.configured({flag:true})> => true,
            _=>false,
        }}
        comptime fn run()->bool {
            inspect(Configured<Config {flag:true}>)
            and inspect(Selected<Mode.configured(Config {flag:true})>)
            and not inspect(Selected<Mode.empty>)
            and not inspect(Configured<Config {flag:false}>)
            and Marked<*bool,Marker<*bool>.token> == Marked<*bool,Marker<*bool>.token>
        }",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_frozen_inputs_validate_identity_variant_and_payload_before_reconstruction() {
    let package = compile(
        "struct Config {enabled:bool}
        enum State {idle,active(Config),}
        comptime fn make()->State {State.active(Config {enabled:true})}
        comptime fn identity(state:State)->State {state}",
    );
    let value = luna::invocation::invoke(&package, "make", 100_000).unwrap();
    let frozen = value
        .clone()
        .into_normal_term(&mut comptime_runtime::Budget::default())
        .unwrap();
    assert_eq!(frozen.sort(), Sort::ComptimeValue);
    assert!(frozen.to_string().contains("(nominal-value (type (nominal"));
    let Term::NominalValue {
        ty,
        variant,
        fields,
    } = frozen.clone()
    else {
        panic!()
    };
    let identity = package
        .functions
        .iter()
        .find(|f| f.name == "identity")
        .unwrap()
        .declaration;
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let mut invalid = vec![
        Term::NominalValue {
            ty: ty.clone(),
            variant: None,
            fields: fields.clone(),
        },
        Term::NominalValue {
            ty: ty.clone(),
            variant: Some(99),
            fields: fields.clone(),
        },
        Term::NominalValue {
            ty: ty.clone(),
            variant,
            fields: vec![],
        },
        Term::NominalValue {
            ty: ty.clone(),
            variant,
            fields: vec![Term::Bool(true)],
        },
        Term::NominalValue {
            ty: Box::new(Term::Primitive(Primitive::Bool)),
            variant,
            fields: fields.clone(),
        },
    ];
    if let Term::NominalValue { ty: inner, .. } = &fields[0] {
        invalid.push(Term::NominalValue {
            ty: ty.clone(),
            variant,
            fields: vec![Term::NominalValue {
                ty: inner.clone(),
                variant: Some(0),
                fields: vec![Term::Bool(true)],
            }],
        });
    }
    for invalid in invalid {
        assert_eq!(
            vm.invoke_with_normal_arguments(identity, vec![((*ty).clone(), invalid)])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(
            vm.invoke_with_normal_arguments(identity, vec![((*ty).clone(), frozen.clone())])
                .unwrap(),
            value
        );
    }
    assert!(
        value
            .clone()
            .into_normal_term(&mut comptime_runtime::Budget {
                steps: 1,
                max_limbs: 128
            })
            .is_err()
    );
    assert_eq!(
        value
            .into_normal_term(&mut comptime_runtime::Budget::default())
            .unwrap(),
        frozen
    );
}

#[test]
fn nominal_staged_arguments_forward_defaults_and_distinguish_instances() {
    let package=compile("typealias str=.str'builtin;
        struct Config {flag:bool}
        enum Mode {first,second,}
        fn enabled(comptime original:Config,comptime .selected:Config=original)->bool {selected.flag}
        fn forward(comptime value:Config)->bool {enabled(value)}
        fn name(comptime value:Mode)->str {value'tag_name}
        fn run()->bool {forward(Config {flag:true}) and not forward(Config {flag:false})
            and name(Mode.first)==\"first\" and name(Mode.second)==\"second\"}");
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nominal_values_cross_staged_parameters_with_nested_payloads() {
    let package = compile(
        "typealias str=.str'builtin;
        struct Settings {enabled:bool}
        enum Message where T {empty,body(T),}
        fn inspect(comptime value:Message<Settings>)->bool {
            value match {.body(settings)=>settings.enabled,_=>false,}
        }
        fn label(comptime value:Message<Settings>)->str {value'tag_name}
        fn enabled(comptime settings:Settings)->bool {settings.enabled}
        fn run()->bool {inspect(Message<Settings>.body(Settings {enabled:true}))
            and not inspect(Message<Settings>.body(Settings {enabled:false}))
            and not inspect(Message<Settings>.empty)
            and enabled(Settings {enabled:true})
            and label(Message<Settings>.empty)==\"empty\"}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    assert!(graph.to_string().contains("nominal-value"));
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}
