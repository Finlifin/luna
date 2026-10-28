use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn scalar_record_trait_indices_keep_values_out_of_type_propositions() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true})'type;
        trait Selected where mode:Settings {fn selected(self)->bool;}
        impl Selected<(settings {enabled:true})> for bool {fn selected(self)->bool {self}}
        impl Selected<(settings {enabled:false})> for bool {fn selected(self)->bool {not self}}
        fn yes(value:T)->bool where T:-Selected<(settings {enabled:true})> {value.selected()}
        fn no(value:T)->bool where T:-Selected<(settings {enabled:false})> {value.selected()}
        fn run()->bool {yes(true) and no(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let duplicate = source.replace(
        "Selected<(settings {enabled:false})> for bool",
        "Selected<(settings {enabled:true})> for bool",
    );
    assert!(compile(&duplicate, Emit::Nir).is_err());
    let wrong = source.replace(
        "Selected<(settings {enabled:false})> for bool",
        "Selected<(settings {enabled:'x'})> for bool",
    );
    assert!(compile(&wrong, Emit::Nir).is_err());
}

#[test]
fn nested_scalar_record_trait_indices_preserve_field_domains() {
    let source="comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true,nested:(settings {tag:.fast})})'type;
        trait Selected where mode:Settings {fn selected(self)->bool;}
        impl Selected<(settings {enabled:true,nested:(settings {tag:.fast})})> for bool {fn selected(self)->bool {self}}
        impl Selected<(settings {enabled:true,nested:(settings {tag:.slow})})> for bool {fn selected(self)->bool {not self}}
        fn fast(value:T)->bool where T:-Selected<(settings {enabled:true,nested:(settings {tag:.fast})})> {value.selected()}
        fn slow(value:T)->bool where T:-Selected<(settings {enabled:true,nested:(settings {tag:.slow})})> {value.selected()}
        fn run()->bool {fast(true) and slow(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn record_type_index_fields_lift_nested_type_equalities() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        fn fields(...properties:T)->T where T {properties}
        typealias Schema = (settings {element:bool})'type;
        typealias Payload = (settings {field:true})'type;
        trait Selected where schema:Schema {fn selected(self)->bool;}
        impl Selected<(settings {element:Payload})> for bool {fn selected(self)->bool {self}}
        fn choose(value:T)->bool where T,requires T==bool {
            let packet = fields {field:value};
            true.as(Selected<(settings {element:packet'type})>).selected()
        }
        fn run()->bool {choose<bool>(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let unproven = source.replace(",requires T==bool", "");
    assert!(compile(&unproven, Emit::Nir).is_err());
    let wrong_name = source.replace("fields {field:value}", "fields {other:value}");
    assert!(compile(&wrong_name, Emit::Nir).is_err());
}

#[test]
fn record_type_indices_use_type_equality_and_infer_implementation_parameters() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Schema = (settings {element:bool})'type;
        trait Selected where schema:Schema {fn selected(self)->bool;}
        impl Selected<(settings {element:T})> for bool where T {fn selected(self)->bool {self}}
        fn run()->bool {true.as(Selected<(settings {element:(.char'builtin)})>).selected()}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );

    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Schema = (settings {element:bool})'type;
        trait Selected where schema:Schema {fn selected(self)->bool;}
        impl Selected<(settings {element:bool})> for bool {fn selected(self)->bool {self}}
        fn choose(value:bool)->bool where T,requires T==bool {value.as(Selected<(settings {element:T})>).selected()}
        fn run()->bool {choose<bool>(true)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let duplicate = format!(
        "{source} impl Selected<(settings {{element:bool}})> for bool {{fn selected(self)->bool {{false}}}}"
    );
    assert!(compile(&duplicate, Emit::Nir).is_err());
    let missing = source.replace("element:T", "element:(.char'builtin)");
    assert!(compile(&missing, Emit::Nir).is_err());
    let opaque = format!(
        "typealias Type=.Type'builtin; pure comptime fn Identity<T>->Type {{T}} {}",
        source.replace("element:T", "element:Identity<bool>")
    );
    assert!(
        compile(&opaque, Emit::Nir).is_err(),
        "normal-form field unfolded"
    );
}

#[test]
fn record_trait_indices_guard_normal_forms_at_invocation() {
    let source = "typealias Type=.Type'builtin;
        comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true})'type;
        trait Allowed where mode:Settings {}
        impl Allowed<(settings {enabled:true})> for bool {}
        pure comptime fn Guarded<T:-Allowed<mode>,mode:Settings>->Type {T}
        comptime fn Build(mode:Settings)->Type {Guarded<bool,mode>}
        comptime fn Good()->Type {Build(settings {enabled:true})}
        comptime fn Bad()->Type {Build(settings {enabled:false})}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let value = luna::invocation::invoke(&package, "Good", 100_000).unwrap();
    let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) = &value else {
        panic!("opaque normal form: {value:?}")
    };
    assert_eq!(
        arguments,
        &vec![
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::RecordValue(vec![("enabled".into(), nir::Term::Bool(true))]),
        ]
    );
    let failure = luna::invocation::invoke(&package, "Bad", 100_000).unwrap_err();
    assert!(failure.contains("TypeContract"), "{failure}");
    assert_eq!(
        luna::invocation::invoke(&package, "Good", 100_000).unwrap(),
        value
    );
}

#[test]
fn inherited_record_indices_substitute_domains_and_defaults() {
    let source = "comptime fn settings(...properties:T)->T where T {properties}
        typealias Settings = (settings {enabled:true})'type;
        trait Root where T,mode:T,.copy:T=mode {comptime fn selected(self)->bool;}
        trait Leaf :- Root<T,mode> where T,mode:T {
            comptime fn inherited(self)->bool {self.as(Root<T,mode>).selected()}
        }
        impl Root<Settings,(settings {enabled:true})> for bool {comptime fn selected(self)->bool {true}}
        impl Root<Settings,(settings {enabled:false})> for bool {comptime fn selected(self)->bool {false}}
        impl Leaf<Settings,(settings {enabled:true})> for bool {}
        impl Leaf<Settings,(settings {enabled:false})> for bool {}
        comptime fn good()->bool {true.as(Leaf<Settings,(settings {enabled:true})>).inherited()}
        comptime fn bad()->bool {true.as(Leaf<Settings,(settings {enabled:false})>).inherited()}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (entry, expected) in [("good", true), ("bad", false)] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            comptime_interpretor::Value::Bool(expected)
        );
    }
    for index in [
        "Root<Settings,(settings {enabled:true}),.copy=(settings {enabled:false})>",
        "Leaf<Settings,(settings {enabled:'x'})>",
        "Leaf<Settings,(settings {other:true})>",
    ] {
        let invalid =
            format!("{source} comptime fn missing()->bool {{true.as({index}).selected()}}");
        assert!(compile(&invalid, Emit::Nir).is_err(), "accepted {index}");
    }
}

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    compile_selected(source, emit, false)
}

fn compile_selected(source: &str, emit: Emit, tuple: bool) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "newtypes-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile(&Options {
        comptime_intrinsics: Vec::new(),
        input: path.clone(),
        emit,
        output: None,
        build_modes: Default::default(),
        build_targets: Default::default(),
        sysroot: None,
        splice_protocol: None,
        meta_list_type: tuple.then(|| "input.List".into()),
        tuple_type: tuple.then(|| "input.Tuple".into()),
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        iterator_protocol: None,
        into_iterator_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn newtype_declarations_preserve_identity_and_representation_in_hir() {
    let source = "typealias Word=.u32'builtin;newtype UserId=Word;newtype OrderId=Word;typealias Alias=UserId;fn accept(id:UserId)->UserId {id}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(dump.matches("(newtype ").count(), 2, "{dump}");
    assert_eq!(dump.matches("(type-alias ").count(), 2, "{dump}");
    assert_eq!(dump.matches("(representation ").count(), 2, "{dump}");
    assert!(!dump.contains("(invalid)"), "{dump}");
}

#[test]
fn newtype_parameters_and_module_exports_resolve() {
    for source in [
        "newtype Wrapper<T> = T;fn use_it(value:Wrapper<bool>)->Wrapper<bool> {value}",
        "mod ids {pub newtype Id=bool;} use ids.Id;fn identity(value:Id)->Id {value}",
        "newtype First<T> = T;newtype Second<T> = T;",
    ] {
        let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
            panic!("wrong stage")
        };
        assert!(dump.contains("(newtype "), "{dump}");
        assert!(!dump.contains("(invalid)"), "{dump}");
    }
}

#[test]
fn newtype_scope_rejects_missing_representation_and_parameter_leakage() {
    for source in [
        "newtype Broken=Missing;",
        "newtype Wrapper<T> = T;typealias Leaked=T;",
        "newtype Same=bool;newtype Same=bool;",
        "mod ids {pub newtype Id=bool;}use ids.Missing;fn accept(value:Missing) {}",
    ] {
        assert!(compile(source, Emit::Hir).is_err(), "accepted {source}");
    }
}

#[test]
fn newtype_nir_retains_nominal_identity_and_generic_representation() {
    let Artifact::Nir(package) = compile("typealias Word=.u32'builtin;newtype UserId=Word;newtype OrderId=Word;newtype Wrapper<T> = T;newtype Nested=Wrapper<UserId>;", Emit::Nir).unwrap() else {panic!("wrong stage")};
    assert_eq!(package.newtypes.len(), 4);
    assert!(package.structs.is_empty());
    let get = |name: &str| package.newtypes.iter().find(|d| d.name == name).unwrap();
    let user = get("UserId");
    let order = get("OrderId");
    assert_ne!(user.declaration, order.declaration);
    assert_eq!(user.representation, order.representation);
    let user_type = nir::Term::Nominal {
        declaration: user.declaration,
        arguments: vec![],
    };
    let wrapper = get("Wrapper");
    assert_eq!(
        wrapper
            .instantiate_representation(&[user_type.clone()])
            .unwrap(),
        user_type
    );
    assert_eq!(
        get("Nested").representation,
        nir::Term::Nominal {
            declaration: wrapper.declaration,
            arguments: vec![user_type]
        }
    );
    assert_eq!(package.to_string().matches("(newtype ").count(), 4);
}

#[test]
fn newtype_rejects_invalid_applications_and_representation_cycles() {
    for source in [
        "newtype A=A;",
        "newtype A=B;newtype B=A;",
        "struct Storage {item:Id,}newtype Id=Storage;",
        "newtype Wrapper<T> = T;newtype Broken=Wrapper;",
        "newtype Wrapper<T> = T;newtype Broken=Wrapper<bool,bool>;",
        "newtype Broken=true;",
        "newtype Id=bool;fn invalid(value:Id)->bool {value}",
        "newtype Id=bool;fn invalid(value:bool)->Id {value}",
        "newtype A=bool;newtype B=bool;fn invalid(value:A)->B {value}",
    ] {
        let failure = compile(source, Emit::Nir)
            .err()
            .expect("invalid newtype accepted");
        assert!(
            failure.contains("elaboration failed"),
            "{source}: {failure}"
        );
        if source.contains("fn invalid") {
            assert!(failure.contains("type mismatch"), "{source}: {failure}");
        }
    }
}

#[test]
fn newtype_identity_signatures_and_aliases_keep_the_declared_type() {
    let Artifact::Nir(package) = compile(
        "newtype Id=bool;typealias Alias=Id;fn identity(value:Alias)->Id {value}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    let ty = nir::Term::Nominal {
        declaration: package.newtypes[0].declaration,
        arguments: vec![],
    };
    assert_eq!(package.functions[0].body.ty, ty);
}

#[test]
fn newtype_does_not_inherit_fields_constructors_or_trait_implementations() {
    for source in [
        "struct Point {x:bool,}newtype Screen=Point;fn invalid(value:Screen)->bool {value.x}",
        "struct Point {x:bool,}newtype Screen=Point;fn invalid()->Screen {Screen {x:true}}",
        "trait Ready {fn ready(self)->bool;}impl Ready for bool {fn ready(self)->bool {self}}newtype Flag=bool;fn invalid(value:Flag)->bool {value.as(Ready).ready()}",
    ] {
        let error = compile(source, Emit::Nir)
            .err()
            .expect("newtype inherited underlying operations");
        assert!(!error.contains("parsing failed"), "{source}: {error}");
        assert!(!error.contains("lowering failed"), "{source}: {error}");
    }
}

#[test]
fn newtype_can_define_its_own_trait_implementation() {
    compile("newtype Flag=bool;trait Ready {fn ready(self)->bool;}impl Ready for Flag {fn ready(self)->bool {true}}fn check(value:Flag)->bool {value.as(Ready).ready()}", Emit::Nir).unwrap();
}

#[test]
fn newtype_pointer_representation_breaks_by_value_recursion() {
    let Artifact::Nir(package) = compile("newtype Link=*Link;", Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert!(matches!(
        package.newtypes[0].representation,
        nir::Term::Pointer(_)
    ));
}

#[test]
fn newtype_instances_retain_representation_dependencies_and_arguments() {
    let Artifact::Nir(package) = compile("struct Payload {flag:bool,}newtype Wrapper<T> = T;newtype Outer=Wrapper<Payload>;fn identity(value:Outer)->Outer {value}", Emit::Nir).unwrap() else {panic!("wrong stage")};
    let graph = luna::instantiation::instantiate(&package, "identity", 100_000).unwrap();
    graph.validate(&mut 100_000).unwrap();
    assert_eq!(graph.nominals.len(), 3);
    let wrappers = graph
        .nominals
        .iter()
        .filter_map(|instance| {
            if let iir::NominalDefinition::Newtype(definition) = &instance.definition {
                assert!(definition.type_parameters.is_empty());
                Some(definition)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(wrappers.len(), 2);
    let inner = wrappers.iter().find(|n| n.name == "Wrapper").unwrap();
    let payload = package.structs[0].declaration;
    assert_eq!(
        inner.representation,
        nir::Term::Nominal {
            declaration: payload,
            arguments: vec![]
        }
    );
    assert_eq!(graph.to_string().matches("(newtype ").count(), 2);
    let mut dangling = graph.clone();
    dangling
        .nominals
        .retain(|n| !matches!(n.definition, iir::NominalDefinition::Struct(_)));
    assert!(dangling.validate(&mut 100_000).is_err());
    let mut malformed = graph;
    let iir::NominalDefinition::Newtype(definition) = &mut malformed
        .nominals
        .iter_mut()
        .find(|n| matches!(n.definition, iir::NominalDefinition::Newtype(_)))
        .unwrap()
        .definition
    else {
        unreachable!()
    };
    definition.type_parameters.push(nir::DeclarationId {
        package: 999,
        local: 999,
    });
    assert!(malformed.validate(&mut 100_000).is_err());
}

#[test]
fn newtype_representation_cannot_hide_comptime_only_runtime_storage() {
    for primitive in ["Float", "Integer", "Type"] {
        for declarations in [
            format!("newtype Hidden=.{primitive}'builtin;"),
            format!("newtype Wrapper<T> = T;newtype Hidden=Wrapper<.{primitive}'builtin>;"),
            format!("struct Storage {{value:.{primitive}'builtin,}}newtype Hidden=Storage;"),
        ] {
            let source = format!("{declarations}fn leak(value:Hidden)->Hidden {{value}}");
            let error = compile(&source, Emit::Nir)
                .err()
                .expect("comptime storage leaked through newtype");
            assert!(error.contains("comptime-only"), "{source}: {error}");
        }
    }
    // Only stored representation matters: this Type parameter is phantom.
    compile("newtype Phantom<T> = bool;fn keep(value:Phantom<.Float'builtin>)->Phantom<.Float'builtin> {value}", Emit::Nir).unwrap();
    compile(
        "newtype Hidden=.Float'builtin;comptime fn keep(value:Hidden)->Hidden {value}",
        Emit::Nir,
    )
    .unwrap();
}

#[test]
fn newtype_instance_formation_keeps_its_own_declared_capabilities() {
    let source = "trait Allowed {}impl Allowed for bool {}newtype Checked<T:-Allowed> = T;fn identity(value:Checked<bool>)->Checked<bool> {value}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let graph = luna::instantiation::instantiate(&package, "identity", 100_000).unwrap();
    graph.validate(&mut 100_000).unwrap();
    let iir::NominalDefinition::Newtype(definition) = &graph.nominals[0].definition else {
        panic!("wrong nominal kind")
    };
    assert_eq!(definition.capabilities.len(), 1);
    assert_eq!(
        definition.capabilities[0].subject,
        nir::Term::Primitive(nir::Primitive::Bool)
    );
    let invalid = source.replace("Checked<bool>", "Checked<.char'builtin>");
    let error = compile(&invalid, Emit::Nir)
        .err()
        .expect("missing formation capability accepted");
    assert!(error.contains("elaboration failed"), "{error}");
}

#[test]
fn explicit_newtype_casts_execute_as_single_edges() {
    for body in [
        "let value=true.as(Flag);value.as(bool)",
        "true.as(Flag).as(Outer).as(Flag).as(bool)",
        "true.as(Wrapper<bool>).as(bool)",
    ] {
        let source = format!(
            "newtype Flag=bool;newtype Outer=Flag;newtype Wrapper<T> = T;fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(package.to_string().contains("(newtype-convert "));
    }
}

#[test]
fn source_newtype_casts_reject_siblings_width_matches_and_skipped_edges() {
    for body in [
        "true.as(A).as(B).as(bool)",
        "true.as(Outer).as(A).as(bool)",
        "true.as(A).as(.u8'builtin)",
    ] {
        let source = format!("newtype A=bool;newtype B=bool;newtype Outer=A;fn run() {{{body};}}");
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("illegal cast accepted");
        assert!(error.contains("conversion"), "{error}");
    }
}

#[test]
fn newtype_casts_specialize_generic_helpers_and_keep_struct_payloads_opaque() {
    let source = "newtype Wrapper<T> = T;struct Point {x:bool,}fn wrap(value:T)->Wrapper<T> where T {value.as(Wrapper<T>)}fn unwrap(value:Wrapper<T>)->T where T {value.as(T)}fn run()->bool {let wrapped=wrap(Point {x:true});unwrap(wrapped).x}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn comptime_newtype_casts_evaluate_the_producer_once() {
    let source = "newtype Flag=bool;comptime fn produce()->bool {true}comptime fn run()->bool {produce().as(Flag).as(bool)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let produce = package
        .functions
        .iter()
        .find(|f| f.name == "produce")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert_eq!(
        vm.events()
            .iter()
            .filter(
                |e| matches!(e, comptime_interpretor::Event::EnterFunction(id) if *id == produce)
            )
            .count(),
        1
    );
}

#[test]
fn iir_rechecks_newtype_cast_edges_instead_of_trusting_expression_annotations() {
    let Artifact::Nir(package) = compile(
        "newtype Flag=bool;fn run()->bool {true.as(Flag).as(bool)}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    let mut graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let iir::NominalDefinition::Newtype(definition) = &mut graph.nominals[0].definition else {
        panic!("wrong kind")
    };
    definition.representation = nir::Term::Primitive(nir::Primitive::U8);
    let error = graph.validate(&mut 100_000).unwrap_err();
    assert!(error.contains("representation edge"), "{error}");
}

#[test]
fn newtype_constants_rebuild_explicit_conversion_nodes() {
    let Artifact::Nir(package) = compile(
        "newtype Flag=bool;const SAVED=true.as(Flag);comptime fn run()->bool {SAVED.as(bool)}",
        Emit::Nir,
    )
    .unwrap() else {
        panic!("wrong stage")
    };
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn interpreter_rejects_struct_impostors_and_wrong_newtype_payloads() {
    use comptime_interpretor::{Interpreter, Value};
    let Artifact::Nir(package) = compile("newtype Flag=bool;comptime fn make()->Flag {true.as(Flag)}comptime fn unwrap(value:Flag)->bool {value.as(bool)}", Emit::Nir).unwrap() else {panic!("wrong stage")};
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let flag = package.newtypes[0].declaration;
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm.invoke(id("make"), vec![]).unwrap();
    assert!(matches!(value, Value::Newtype { .. }));
    assert!(value.to_string().starts_with("(newtype-value "));
    assert_eq!(
        vm.invoke(id("unwrap"), vec![value]).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        Value::Bool(true),
        Value::Record {
            declaration: flag,
            arguments: vec![],
            fields: vec![Value::Bool(true)],
        },
        Value::Newtype {
            ty: nir::Term::Nominal {
                declaration: flag,
                arguments: vec![],
            },
            value: Box::new(Value::Char('x')),
        },
    ] {
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert!(vm.invoke(id("unwrap"), vec![invalid]).is_err());
    }
}

#[test]
fn newtype_values_freeze_and_thaw_without_becoming_structs() {
    use comptime_interpretor::{Interpreter, Value};
    let Artifact::Nir(package) = compile("newtype Flag=bool;newtype Outer=Flag;comptime fn make()->Outer {true.as(Flag).as(Outer)}comptime fn unwrap(value:Outer)->bool {value.as(Flag).as(bool)}", Emit::Nir).unwrap() else {panic!("wrong stage")};
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm.invoke(id("make"), vec![]).unwrap();
    let Value::Newtype { ty, .. } = &value else {
        panic!("lost newtype kind")
    };
    let ty = ty.clone();
    vm.validate_immutable_input(&value, &ty).unwrap();
    let frozen = value.into_normal_term(&mut Default::default()).unwrap();
    assert!(
        matches!(&frozen, nir::Term::NominalValue {variant:None,fields,..} if fields.len()==1 && matches!(fields[0],nir::Term::NominalValue {..}))
    );
    assert_eq!(
        vm.invoke_with_normal_arguments(id("unwrap"), vec![(ty.clone(), frozen.clone())])
            .unwrap(),
        Value::Bool(true)
    );
    for malformed in [
        nir::Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: Some(0),
            fields: vec![],
        },
        nir::Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields: vec![],
        },
        nir::Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields: vec![nir::Term::Bool(true)],
        },
    ] {
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert!(
            vm.invoke_with_normal_arguments(id("unwrap"), vec![(ty.clone(), malformed)])
                .is_err()
        );
    }
}

#[test]
fn newtype_normal_form_inputs_preserve_wrapper_identity_and_value() {
    let source = "typealias Type=.Type'builtin;newtype Flag=bool;pure comptime fn Tagged<value:Flag>->Type {bool}typealias A=Tagged<true.as(Flag)>;typealias B=Tagged<true.as(Flag)>;typealias C=Tagged<false.as(Flag)>;comptime fn equal()->bool {A==B}comptime fn First()->Type {A}comptime fn Third()->Type {C}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    assert_eq!(
        vm.invoke(id("equal"), vec![]).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    // Compare the retained input data, not the semantic equality of opaque
    // results: distinct arguments do not license an inverse head equation.
    for (name, expected) in [("First", true), ("Third", false)] {
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            vm.invoke(id(name), vec![]).unwrap()
        else {
            panic!("opaque head lost")
        };
        assert!(
            matches!(&arguments[0], nir::Term::NominalValue {fields,..} if fields == &[nir::Term::Bool(expected)])
        );
    }
}

#[test]
fn comptime_type_construction_preserves_newtype_parameters() {
    let source = "typealias Type=.Type'builtin;newtype Wrapper<T> = T;comptime fn Wrapped(value:Type)->Type {Wrapper<value>}comptime fn run()->Type {Wrapped(bool)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Bool)]
        })
    );
}

#[test]
fn cmtt_generators_accept_newtype_data_without_exposing_fields() {
    let source = "newtype Flag=bool;pure comptime fn build(flag:Flag)->< |- bool> {let bit=flag.as(bool);'(bit)}fn run()->bool {~<build(true.as(Flag))> and not ~<build(false.as(Flag))>}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn newtype_immutable_input_metadata_does_not_depend_on_declaration_order() {
    for source in [
        "typealias Type=.Type'builtin;pure comptime fn Tagged<value:Flag>->Type {bool}newtype Flag=bool;typealias Used=Tagged<true.as(Flag)>;",
        "pure comptime fn build(flag:Flag)->< |- bool> {let bit=flag.as(bool);'(bit)}newtype Flag=bool;fn run()->bool {~<build(true.as(Flag))>}",
    ] {
        compile(source, Emit::Nir).unwrap();
    }
}

#[test]
fn newtype_cleanup_plans_keep_wrapper_and_representation_responsibilities() {
    use elaboration::verification::{drop::CleanupStep, moves::CleanupAction};
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Payload {}impl Retire for Payload {fn drop(*self) {}}newtype Wrapper=Payload;impl Retire for Wrapper {fn drop(*self) {}}fn consume(value:Wrapper) {}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let analysis = luna::resources::analyze_with_protocols(
        &package,
        "consume",
        "Duplicate",
        Some("Retire"),
        100_000,
    )
    .unwrap();
    let calls = analysis
        .report
        .cleanup_sites
        .iter()
        .flat_map(|site| &site.actions)
        .filter_map(|action| {
            if let CleanupAction::Planned { plan, .. } = action {
                Some(plan)
            } else {
                None
            }
        })
        .flat_map(|plan| &plan.steps)
        .filter_map(|step| {
            if let CleanupStep::Invoke { selected, .. } = step {
                Some(selected.subject.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls,
        vec![
            nir::Term::Nominal {
                declaration: package.newtypes[0].declaration,
                arguments: vec![]
            },
            nir::Term::Nominal {
                declaration: package.structs[0].declaration,
                arguments: vec![]
            },
        ]
    );
}

#[test]
fn newtype_ownership_does_not_inherit_copy_from_its_representation() {
    let source = "trait Duplicate {}impl Duplicate for bool {}newtype Flag=bool;fn take(value:Flag) {}fn once(value:Flag) {take(value);}fn twice(value:Flag) {take(value);take(value);}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    luna::resources::analyze_with_protocols(&package, "once", "Duplicate", None, 100_000).unwrap();
    let error =
        luna::resources::analyze_with_protocols(&package, "twice", "Duplicate", None, 100_000)
            .err()
            .expect("wrapper must move");
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn newtype_conversions_transfer_owned_values_through_checked_execution() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;fn run()->bool {let flag=true.as(Flag);flag.as(bool)}fn bad(flag:Flag) {let first=flag.as(bool);let second=flag.as(bool);}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(
        luna::resources::analyze_with_protocols(
            &package,
            "bad",
            "Duplicate",
            Some("Retire"),
            100_000
        )
        .is_err()
    );
}

#[test]
fn newtype_conversions_preserve_pointer_and_callable_origins() {
    for source in [
        "trait Duplicate {}impl Duplicate for bool {}trait Retire {fn drop(*self);}newtype Ref=*bool;fn run()->bool {let value=true;let wrapped=value.ref.as(Ref);wrapped.as(*bool).*}",
        "trait Duplicate {}impl Duplicate for bool {}trait Retire {fn drop(*self);}newtype Ref=*bool;fn run()->bool {let value=true;let wrapped=value.ref.as(Ref);let ptr=wrapped.as(*bool);ptr.*}",
        "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;newtype Wrapped=Action;fn yes()->bool {true}fn run()->bool {let wrapped=yes.as(Wrapped);let action=wrapped.as(Action);action()}",
    ] {
        let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    let Artifact::Nir(package) = compile("trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {value:bool,}newtype Ref=*bool;fn bad()->bool {let cell=Cell {value:true};let wrapped=cell.value.ref.as(Ref);let taken=cell;let ptr=wrapped.as(*bool);ptr.*}", Emit::Nir).unwrap() else {panic!("wrong stage")};
    assert!(
        luna::resources::analyze_with_protocols(
            &package,
            "bad",
            "Duplicate",
            Some("Retire"),
            100_000
        )
        .is_err()
    );
}

#[test]
fn pure_newtype_conversions_transfer_affine_payloads_without_copy() {
    for source in [
        "newtype Flag=bool;pure fn wrap(value:bool)->Flag {value.as(Flag)}pure fn unwrap(value:Flag)->bool {value.as(bool)}fn run()->bool {unwrap(wrap(true))}",
        "struct Payload {flag:bool,}newtype Wrapper=Payload;pure fn wrap(value:Payload)->Wrapper {value.as(Wrapper)}pure fn unwrap(value:Wrapper)->Payload {value.as(Payload)}fn run()->bool {unwrap(wrap(Payload {flag:true})).flag}",
    ] {
        let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for source in [
        "newtype Flag=bool;pure fn forget(value:Flag) {}",
        "newtype Flag=bool;pure fn twice(value:Flag)->bool {let first=value.as(bool);value.as(bool)}",
        "newtype Flag=bool;fn observe(ptr:*bool)->bool {ptr.*}pure fn wrap(ptr:*bool)->Flag {observe(ptr).as(Flag)}",
    ] {
        let error = compile(source, Emit::Nir)
            .err()
            .expect("impure conversion use must fail");
        assert!(error.contains("P4 evidence"), "{error}");
    }
}

#[test]
fn pure_wrapped_callables_preserve_invocation_identity() {
    for source in [
        "typealias Action=fn()->bool;newtype Wrapped=Action;fn yes()->bool {true}fn no()->bool {false}pure fn choose(flag:bool)->bool {var action=no;var again=flag;while again {action=yes.as(Wrapped).as(Action);again=false;}action()}fn run()->bool {choose(true)}",
        "typealias Action=fn()->bool;newtype Wrapped=Action;fn yes()->bool {true}pure fn run()->bool {let wrapped=yes.as(Wrapped);wrapped.as(Action)()}",
        "typealias Action=fn()->bool;newtype Wrapped=Action;fn yes()->bool {true}fn make()->Wrapped {yes.as(Wrapped)}pure fn run()->bool {make().as(Action)()}",
        "typealias Action=fn()->bool;newtype Wrapped=Action;fn yes()->bool {true}fn apply(wrapped:Wrapped)->bool {wrapped.as(Action)()}pure fn run()->bool {apply(yes.as(Wrapped))}",
    ] {
        let Artifact::Nir(package) =
            compile(source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn wrapping_code_does_not_hide_external_reads_from_purity() {
    let source = "typealias Action=fn(*bool)->bool;newtype Wrapped=Action;fn observe(ptr:*bool)->bool {ptr.*}fn make()->Wrapped {observe.as(Wrapped)}pure fn bad(ptr:*bool)->bool {make().as(Action)(ptr)}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("wrapped external reader must not be pure");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn memory_callable_layout_does_not_expose_a_newtype_source_field() {
    use nir::program::ExprKind as K;
    let Artifact::Nir(mut package)=compile("typealias Action=fn()->bool;newtype Wrapped=Action;fn unwrap(value:Wrapped)->Action {value.as(Action)}", Emit::Nir).unwrap() else {panic!("wrong stage")};
    let declaration = package.newtypes[0].declaration;
    let function = package
        .functions
        .iter_mut()
        .find(|function| function.name == "unwrap")
        .unwrap();
    let id = function.declaration;
    let K::Block {
        result: Some(result),
        ..
    } = &mut function.body.kind
    else {
        panic!("expected block")
    };
    let K::NewtypeConvert(value) = &result.kind else {
        panic!("expected conversion")
    };
    result.kind = K::Field {
        value: value.clone(),
        declaration,
        index: 0,
    };
    let error =
        elaboration::verification::memory::infer_in_package(&package, id, &[], &mut 100_000)
            .unwrap_err();
    assert!(error.contains("not a source field"), "{error}");
}

#[test]
fn wrapped_closures_preserve_captures_through_return_and_delivery() {
    for source in [
        "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=%{flag:bool} fn()->bool;newtype Wrapped=Action;fn make(flag:bool)->Wrapped {let action=||flag;action.as(Wrapped)}fn run()->bool {let wrapped=make(true);wrapped.as(Action)()}",
        "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;fn make(value:Flag)->%{value:Flag} fn()->bool {||value.as(bool)}fn run()->bool {let action=make(true.as(Flag));action()}",
    ] {
        let Artifact::Nir(package) =
            compile(source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{source}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn returned_newtype_capture_respects_explicit_copy_permission() {
    for (implementation, accepted) in [("impl Duplicate for Flag {}", true), ("", false)] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Flag=bool;{implementation}fn make(value:Flag)->%{{value:Flag}} fn()->bool {{||value.as(bool)}}fn run()->bool {{let action=make(true.as(Flag));let first=action();let second=action();first and second}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if accepted {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn wrapped_enums_preserve_tags_across_return_and_cleanup() {
    for source in [
        "trait Duplicate {}trait Retire {fn drop(*self);}enum Choice {yes(bool),no(bool)}newtype Wrapped=Choice;fn make(flag:bool)->Wrapped {(if flag {Choice.yes(true)}else{Choice.no(false)}).as(Wrapped)}fn read(value:Wrapped)->bool {value.as(Choice) match {.yes(flag)=>flag,.no(flag)=>not flag}}fn run()->bool {read(make(true)) and read(make(false))}",
        "trait Duplicate {}trait Retire {fn drop(*self);}struct Good {}struct Bad {}impl Retire for Good {fn drop(*self) {assumes true;}}impl Retire for Bad {fn drop(*self) {assumes false;}}enum Choice {yes(Good),no(Bad)}newtype Wrapped=Choice;fn make()->Wrapped {Choice.yes(Good {}).as(Wrapped)}fn run()->bool {let value=make();true}",
    ] {
        let Artifact::Nir(package) =
            compile(source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{source}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn wrapped_enum_return_keeps_tag_evidence_without_callable_fields() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Token {}enum Choice {yes,no}newtype Wrapped=Choice;fn make()->Wrapped {Choice.yes.as(Wrapped)}fn take(value:Token) {}fn run()->bool {let token=Token {};make().as(Choice) match {.yes=>true,.no=>do {take(token);take(token);false}}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let rejected = source.replace("Choice.yes.as(Wrapped)", "Choice.no.as(Wrapped)");
    let Artifact::Nir(package) = compile(&rejected, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn guard_newtype_conversion_preserves_copy_and_retry_rules() {
    for (implementation, accepted) in [("impl Duplicate for Flag {}", true), ("", false)] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Flag=bool;{implementation}fn run()->bool {{let flag=false.as(Flag);true match {{_ if flag.as(bool)=>false,_=>not flag.as(bool)}}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if accepted {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
        } else {
            assert!(result.unwrap_err().contains("failure-path"));
        }
    }
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;fn run()->bool {true match {_ if do {let flag=true.as(Flag);flag.as(bool)}=>true,_=>false}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn guard_unwrap_keeps_callable_identity_and_rejects_hidden_writes() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}newtype Wrapped=Action;impl Duplicate for Wrapped {}fn yes()->bool {true}fn run()->bool {let action=yes.as(Wrapped);true match {_ if action.as(Action)()=>true,_=>false}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;impl Duplicate for Flag {}fn mutate(ptr:*bool)->Flag {ptr.* = false;true.as(Flag)}fn run()->bool {var flag=true;true match {_ if mutate(flag.ref).as(bool)=>true,_=>false}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .is_err()
    );
}

#[test]
fn newtype_payload_drop_follows_live_pointer_targets() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}}newtype Wrapped=Watch;newtype Outer=Wrapped;";
    for body in [
        "let flag=false;do {let wrapped=Watch {target:flag.ref}.as(Wrapped);}flag",
        "let flag=false;do {let wrapped=Watch {target:flag.ref}.as(Wrapped).as(Outer);}flag",
        "let first=false;let second=false;do {var wrapped=Watch {target:first.ref}.as(Wrapped);wrapped=Watch {target:second.ref}.as(Wrapped);}first and second",
    ] {
        let source = format!("{prefix}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn newtype_payload_drop_rejects_dead_and_early_retired_targets() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}struct Item {flag:bool}fn consume(value:Item) {}struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}}newtype Wrapped=Watch;";
    for body in [
        "let item=Item {flag:false};let wrapped=Watch {target:item.flag.ref}.as(Wrapped);consume(item);true",
        "let early=false;var wrapped=Watch {target:early.ref}.as(Wrapped);let late=false;wrapped=Watch {target:late.ref}.as(Wrapped);true",
    ] {
        let source = format!("{prefix}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("unavailable") || error.contains("retired"),
            "{body}: {error}"
        );
    }
}

#[test]
fn function_memory_contract_retains_locally_wrapped_pointer_origins() {
    let prefix =
        "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;newtype Outer=Ref;";
    for helper in [
        "fn relay(ptr:*bool)->*bool {ptr.as(Ref).as(*bool)}",
        "fn relay(ptr:*bool)->*bool {let wrapped=ptr.as(Ref).as(Outer);wrapped.as(Ref).as(*bool)}",
        "fn relay(ptr:*bool)->*bool {let wrapped=if true {ptr.as(Ref)}else{ptr.as(Ref)};wrapped.as(*bool)}",
    ] {
        let source = format!("{prefix}{helper}fn run()->bool {{let flag=true;relay(flag.ref).*}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{helper}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn wrapped_local_pointer_flow_tracks_branches_replacement_and_writes() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;";
    for definitions in [
        "fn select(left:*bool,right:*bool,flag:bool)->*bool {let wrapped=if flag {left.as(Ref)}else{right.as(Ref)};wrapped.as(*bool)}fn run()->bool {let left=true;let right=false;select(left.ref,right.ref,true).* and not select(left.ref,right.ref,false).*}",
        "struct Item {flag:bool}fn consume(value:Item) {}fn replace(left:*bool,right:*bool)->*bool {var wrapped=left.as(Ref);wrapped=right.as(Ref);wrapped.as(*bool)}fn run()->bool {let left=Item {flag:false};let right=true;let result=replace(left.flag.ref,right.ref);consume(left);result.*}",
        "fn write(ptr:*bool) {let wrapped=ptr.as(Ref);wrapped.as(*bool).* = true;}fn run()->bool {let flag=false;write(flag.ref);flag}",
    ] {
        let Artifact::Nir(package) = compile(&format!("{prefix}{definitions}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{definitions}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn wrapped_local_pointer_cannot_escape_its_storage() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;fn escape()->*bool {let local=true;let wrapped=local.ref.as(Ref);wrapped.as(*bool)}fn run()->bool {escape().*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("local") || error.contains("scope"),
        "{error}"
    );
}

#[test]
fn wrapped_pointer_parameters_publish_checked_pointee_contracts() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;newtype Outer=Ref;fn read(value:Ref)->bool {value.as(*bool).*}fn relay(value:Outer)->*bool {value.as(Ref).as(*bool)}fn run()->bool {let flag=true;let a=read(flag.ref.as(Ref));let b=relay(flag.ref.as(Ref).as(Outer)).*;a and b}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn wrapped_pointer_results_and_forwarding_keep_lifetime_evidence() {
    let prefix =
        "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;newtype Outer=Ref;";
    for definitions in [
        "newtype GenericRef<T> = *T;fn wrap(ptr:*T)->GenericRef<T> where T {ptr.as(GenericRef<T>)}fn forward(value:GenericRef<T>)->GenericRef<T> where T {value}fn run()->bool {let flag=true;forward(wrap(flag.ref)).as(*bool).*}",
        "fn wrap(ptr:*bool)->Ref {ptr.as(Ref)}fn run()->bool {let flag=true;wrap(flag.ref).as(*bool).*}",
        "fn wrap(ptr:*bool)->Ref {return ptr.as(Ref);}fn forward(value:Ref)->Outer {value.as(Outer)}fn run()->bool {let flag=true;forward(wrap(flag.ref)).as(Ref).as(*bool).*}",
        "fn read(ptr:*bool,.wrapped:Ref=ptr.as(Ref))->bool {wrapped.as(*bool).*}fn run()->bool {let flag=true;read(flag.ref)}",
    ] {
        let Artifact::Nir(package) = compile(&format!("{prefix}{definitions}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{definitions}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn wrapped_pointer_contracts_reject_dead_targets_and_local_escape() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;";
    for definitions in [
        "struct Item {flag:bool}fn consume(value:Item) {}fn read(value:Ref)->bool {value.as(*bool).*}fn run()->bool {let item=Item {flag:true};let wrapped=item.flag.ref.as(Ref);consume(item);read(wrapped)}",
        "fn escape()->Ref {let flag=true;flag.ref.as(Ref)}fn run()->bool {escape().as(*bool).*}",
    ] {
        let Artifact::Nir(package) = compile(&format!("{prefix}{definitions}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("unavailable") || error.contains("local") || error.contains("scope"),
            "{definitions}: {error}"
        );
    }
}

#[test]
fn closure_pointer_wrappers_keep_parameter_and_result_origins() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;";
    for body in [
        "let read=|value:Ref|value.as(*bool).*;let flag=true;read(flag.ref.as(Ref))",
        "let wrap=|ptr:*bool|ptr.as(Ref);let flag=true;wrap(flag.ref).as(*bool).*",
        "let forward=|value:Ref|value;let flag=true;forward(flag.ref.as(Ref)).as(*bool).*",
        "let flag=true;(|value:Ref|value)(flag.ref.as(Ref)).as(*bool).*",
    ] {
        let Artifact::Nir(package) =
            compile(&format!("{prefix}fn run()->bool {{{body}}}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn closure_wrapped_pointer_contracts_reject_dead_targets() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;struct Item {flag:bool}fn consume(value:Item) {}";
    for body in [
        "let forward=|value:Ref|value;let item=Item {flag:true};let wrapped=forward(item.flag.ref.as(Ref));consume(item);wrapped.as(*bool).*",
        "let read=|value:Ref|value.as(*bool).*;let item=Item {flag:true};let wrapped=item.flag.ref.as(Ref);consume(item);read(wrapped)",
        "let escape=|| {let local=true;local.ref.as(Ref)};escape().as(*bool).*",
    ] {
        let Artifact::Nir(package) =
            compile(&format!("{prefix}fn run()->bool {{{body}}}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("unavailable") || error.contains("local") || error.contains("scope"),
            "{body}: {error}"
        );
    }
}

#[test]
fn higher_order_wrapped_pointer_calls_keep_contracts() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;fn apply(action:fn(Ref)->Ref,value:Ref)->Ref {action(value)}fn run()->bool {let flag=true;apply(|value:Ref|value,flag.ref.as(Ref)).as(*bool).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn captured_wrapped_pointers_keep_construction_origins() {
    let prefix =
        "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;newtype Outer=Ref;";
    for body in [
        "let flag=true;let wrapped=flag.ref.as(Ref);let read=||wrapped.as(*bool).*;read()",
        "let flag=true;let wrapped=flag.ref.as(Ref).as(Outer);let read=||wrapped.as(Ref).as(*bool).*;read()",
        "let flag=true;let wrapped=flag.ref.as(Ref);let forward=||wrapped;forward().as(*bool).*",
    ] {
        let Artifact::Nir(package) =
            compile(&format!("{prefix}fn run()->bool {{{body}}}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn wrapped_pointer_captures_snapshot_sources_and_require_copy_for_reuse() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;";
    for body in [
        "let first=true;let second=false;var wrapped=first.ref.as(Ref);let read=||wrapped.as(*bool).*;wrapped=second.ref.as(Ref);read()",
        "let flag=true;let wrapped=flag.ref.as(Ref);let read=||wrapped.as(*bool).*;read() and read()",
    ] {
        let copy = if body.contains("read() and read()") {
            "impl Duplicate for Ref {}"
        } else {
            ""
        };
        let Artifact::Nir(package) = compile(
            &format!("{prefix}{copy}fn run()->bool {{{body}}}"),
            Emit::Nir,
        )
        .unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for body in [
        "let flag=true;let wrapped=flag.ref.as(Ref);let read=||wrapped.as(*bool).*;read() and read()",
        "let item=Item {flag:true};let wrapped=item.flag.ref.as(Ref);let read=||wrapped.as(*bool).*;consume(item);read()",
    ] {
        let source = format!(
            "{prefix}struct Item {{flag:bool}}fn consume(value:Item) {{}}fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(error.contains("unavailable"), "{body}: {error}");
    }
}

#[test]
fn immediate_wrapped_pointer_capture_composes_a_named_memory_summary() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;fn read(value:Ref)->bool {(||value.as(*bool).*)()}fn run()->bool {let flag=true;read(flag.ref.as(Ref))}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn wrapped_pointer_closures_under_installed_effects() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;effect poll(value:bool)->bool;";
    for body in [
        "let flag=true;let wrapped=flag.ref.as(Ref);let read:%{wrapped:Ref} fn()->#poll bool=||poll(wrapped.as(*bool).*)#;read()# {poll(value)=>value,}",
        "let flag=true;let read:fn(Ref)->#poll bool=|wrapped:Ref|poll(wrapped.as(*bool).*)#;read(flag.ref.as(Ref))# {poll(value)=>value,}",
    ] {
        let Artifact::Nir(package) =
            compile(&format!("{prefix}fn run()->bool {{{body}}}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn mixed_captures_preserve_distinct_wrapped_pointer_origins() {
    let prefix =
        "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;newtype Outer=Ref;";
    for body in [
        "let first=true;let second=false;let a=first.ref.as(Ref);let marker=true;let b=second.ref.as(Ref).as(Outer);let read=||a.as(*bool).* and marker and not b.as(Ref).as(*bool).*;read()",
        "let first=true;let second=false;let a=first.ref.as(Ref);let marker=true;let b=second.ref.as(Ref).as(Outer);let read=|arg:bool|arg and a.as(*bool).* and marker and not b.as(Ref).as(*bool).*;read(true)",
    ] {
        let Artifact::Nir(package) =
            compile(&format!("{prefix}fn run()->bool {{{body}}}"), Emit::Nir).unwrap()
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn installed_effects_do_not_revive_wrapped_pointer_targets() {
    for invocation in [
        "let wrapped=item.flag.ref.as(Ref);let read:%{wrapped:Ref} fn()->#poll bool=||poll(wrapped.as(*bool).*)#;consume(item);read()# {poll(value)=>value,}",
        "let wrapped=item.flag.ref.as(Ref);let read:fn(Ref)->#poll bool=|arg:Ref|poll(arg.as(*bool).*)#;consume(item);read(wrapped)# {poll(value)=>value,}",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Ref=*bool;struct Item {{flag:bool}}fn consume(value:Item) {{}}effect poll(value:bool)->bool;fn run()->bool {{let item=Item {{flag:true}};{invocation}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("unavailable")
                || error.contains("handler pointer requires a live local Copy scalar target"),
            "{invocation}: {error}"
        );
    }
}

#[test]
fn wrapped_pointer_match_results_keep_parameter_origins() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;";
    for (definition, expected) in [
        (
            "fn select(flag:bool,a:Ref,b:Ref)->Ref {flag match {true=>a,false=>b}}",
            false,
        ),
        (
            "fn select(flag:bool,a:Ref,b:Ref)->Ref {a match {value=>value}}",
            true,
        ),
    ] {
        let source = format!(
            "{prefix}{definition}fn run()->bool {{let a=true;let b=false;select(false,a.ref.as(Ref),b.ref.as(Ref)).as(*bool).*}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|error| panic!("{definition}: {error}")),
            comptime_interpretor::Value::Bool(expected)
        );
    }
}

#[test]
fn wrapped_pointer_match_cannot_hide_local_escape() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;fn select(flag:bool,a:Ref)->Ref {let local=true;flag match {true=>a,false=>local.ref.as(Ref)}}fn run()->bool {let a=true;select(false,a.ref.as(Ref)).as(*bool).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("local memory target cannot escape in a result"),
        "{error}"
    );
}

#[test]
fn indirect_wrapped_pointer_load_preserves_targets() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;impl Duplicate for Ref {}fn read(slot:*Ref)->bool {slot.*.as(*bool).*}fn run()->bool {let flag=true;let wrapped=flag.ref.as(Ref);read(wrapped.ref)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(luna::execution::interpret(&package, "run", 100_000).is_err());
}

#[test]
fn indirect_nested_wrapper_load_and_return_keep_origins() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;newtype Outer=Ref;impl Duplicate for Ref {}impl Duplicate for Outer {}";
    for definition in [
        "fn read(slot:*Outer)->bool {slot.*.as(Ref).as(*bool).*}",
        "fn load(slot:*Outer)->Outer {slot.*}fn read(slot:*Outer)->bool {load(slot).as(Ref).as(*bool).*}",
    ] {
        let source = format!(
            "{prefix}{definition}fn run()->bool {{let flag=true;let wrapped=flag.ref.as(Ref).as(Outer);read(wrapped.ref) and read(wrapped.ref)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{definition}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn indirect_wrapped_pointer_reads_require_copy_and_live_targets() {
    for (implementation, body, diagnostic) in [
        (
            "",
            "let flag=true;let wrapped=flag.ref.as(Ref);read(wrapped.ref)",
            "checked Copy permission",
        ),
        (
            "impl Duplicate for Ref {}",
            "let item=Item {flag:true};let wrapped=item.flag.ref.as(Ref);consume(item);read(wrapped.ref)",
            "unavailable",
        ),
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Ref=*bool;{implementation}struct Item {{flag:bool}}fn consume(value:Item) {{}}fn read(slot:*Ref)->bool {{slot.*.as(*bool).*}}fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn indirect_wrapped_read_evaluates_slot_once() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;impl Duplicate for Ref {}fn slot(p:*Ref,tick:*bool)->*Ref {tick.* = not tick.*;p}fn run()->bool {let flag=true;let wrapped=flag.ref.as(Ref);var tick=false;let value=slot(wrapped.ref,tick.ref).*.as(*bool).*;value and tick}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn indirect_scalar_wrapper_reads_keep_nominal_copy_rules() {
    for (declarations, read, body) in [
        (
            "newtype Flag=bool;impl Duplicate for Flag {}",
            "fn read(p:*Flag)->Flag {p.*}",
            "let flag=true.as(Flag);read(flag.ref).as(bool) and read(flag.ref).as(bool)",
        ),
        (
            "newtype Flag=bool;newtype Outer=Flag;impl Duplicate for Flag {}impl Duplicate for Outer {}",
            "fn read(p:*Outer)->Outer {p.*}",
            "let flag=true.as(Flag).as(Outer);read(flag.ref).as(Flag).as(bool)",
        ),
        (
            "newtype Scalar<T> = T;impl Duplicate for Scalar<bool> {}",
            "fn read(p:*Scalar<bool>)->Scalar<bool> {p.*}",
            "let flag=true.as(Scalar<bool>);read(flag.ref).as(bool)",
        ),
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}{declarations}{read}fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{source}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;fn read(p:*Flag)->Flag {p.*}fn run()->bool {let flag=true.as(Flag);read(flag.ref).as(bool)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(error.contains("checked Copy permission"), "{error}");
}

#[test]
fn scalar_wrapper_field_reads_and_guard_retry_preserve_storage() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;impl Duplicate for Flag {}struct Cell {flag:Flag}";
    for definition in [
        "fn read(p:*Cell)->bool {p.*.flag.as(bool)}fn run()->bool {let cell=Cell {flag:true.as(Flag)};read(cell.ref) and read(cell.ref)}",
        "fn read(p:*Flag)->bool {true match {_ if p.*.as(bool)=>false,_=>not p.*.as(bool)}}fn run()->bool {let flag=false.as(Flag);read(flag.ref)}",
    ] {
        let source = format!("{prefix}{definition}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{definition}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn scalar_wrapper_read_cannot_inherit_inner_copy() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;newtype Outer=Flag;impl Duplicate for Flag {}fn read(p:*Outer)->Outer {p.*}fn run()->bool {let flag=true.as(Flag).as(Outer);read(flag.ref).as(Flag).as(bool)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(error.contains("checked Copy permission"), "{error}");
}

#[test]
fn scalar_wrapper_pointer_write_preserves_nominal_storage() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;impl Duplicate for Flag {}newtype Outer=Flag;impl Duplicate for Outer {}";
    for body in [
        "let value=false.as(Flag);value.ref.* = true.as(Flag);value.as(bool)",
        "let value=false.as(Flag);set(value.ref);value.as(bool)",
        "let value=false.as(Flag).as(Outer);value.ref.* = true.as(Flag).as(Outer);value.as(Flag).as(bool)",
    ] {
        let source =
            format!("{prefix}fn set(p:*Flag) {{p.* = true.as(Flag);}}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn scalar_wrapper_write_evaluates_target_before_value_once() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;impl Duplicate for Flag {}fn target(p:*Flag,tick:*bool)->*Flag {tick.* = not tick.*;p}fn replacement(tick:*bool)->Flag {tick.*.as(Flag)}fn run()->bool {let value=false.as(Flag);var tick=false;target(value.ref,tick.ref).* = replacement(tick.ref);value.as(bool) and tick}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(luna::execution::interpret(&package, "run", 100_000).is_err());
}

#[test]
fn scalar_wrapper_writes_require_copy_and_live_storage() {
    for (implementation, body, diagnostic) in [
        (
            "",
            "let value=false.as(Flag);set(value.ref);true",
            "checked Copy permission",
        ),
        (
            "impl Duplicate for Flag {}",
            "let cell=Cell {flag:false.as(Flag)};let p=cell.flag.ref;consume(cell);set(p);true",
            "unavailable",
        ),
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Flag=bool;{implementation}struct Cell {{flag:Flag}}fn consume(cell:Cell) {{}}fn set(p:*Flag) {{p.* = true.as(Flag);}}fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(error.contains(diagnostic), "{body}: {error}");
    }
}

#[test]
fn scalar_write_support_does_not_authorize_wrapped_pointer_replacement() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;impl Duplicate for Ref {}fn replace(p:*Ref,value:Ref) {p.* = value;}fn run()->bool {let a=true;let b=false;let value=a.ref.as(Ref);replace(value.ref,b.ref.as(Ref));value.as(*bool).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("requires a richer contract")
            || error.contains("requires a memory transfer contract"),
        "{error}"
    );
}

#[test]
fn wrapped_scalar_pointer_delivery_cannot_bypass_drop_legality() {
    for operation in [
        "fn access(p:*Flag)->bool {p.*.as(bool)}",
        "fn access(p:*Flag)->bool {p.* = true.as(Flag);true}",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Flag=bool;impl Duplicate for Flag {{}}impl Retire for Flag {{fn drop(*self) {{}}}}{operation}fn run()->bool {{let flag=false.as(Flag);access(flag.ref)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("Copy") && error.contains("Drop"),
            "{operation}: {error}"
        );
    }
}

#[test]
fn wrapped_scalar_pointer_writes_are_not_guard_observations() {
    for guard in ["do {p.* = true.as(Flag);true}", "set(p)"] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}newtype Flag=bool;impl Duplicate for Flag {{}}fn set(p:*Flag)->bool {{p.* = true.as(Flag);true}}fn read(p:*Flag)->bool {{true match {{_ if {guard}=>true,_=>false}}}}fn run()->bool {{let flag=false.as(Flag);read(flag.ref)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("guard") || error.contains("observation"),
            "{guard}: {error}"
        );
    }
}

#[test]
fn wrapped_scalar_guard_read_requires_nominal_copy() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Flag=bool;fn read(p:*Flag)->bool {true match {_ if p.*.as(bool)=>true,_=>false}}fn run()->bool {let flag=true.as(Flag);read(flag.ref)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(error.contains("Copy"), "{error}");
}

#[test]
fn local_wrapped_pointer_replacement_updates_leaf_origins() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;impl Duplicate for Ref {}newtype Outer=Ref;impl Duplicate for Outer {}";
    for body in [
        "let first=false;let second=true;let slot=first.ref.as(Ref);slot.ref.* = second.ref.as(Ref);slot.as(*bool).*",
        "let first=false;let second=true;let slot=first.ref.as(Ref).as(Outer);let replacement=second.ref.as(Ref).as(Outer);slot.ref.* = replacement;slot.as(Ref).as(*bool).*",
        "let first=false;let second=true;let slot=first.ref.as(Ref);let saved=slot;slot.ref.* = second.ref.as(Ref);not saved.as(*bool).* and slot.as(*bool).*",
    ] {
        let source = format!("{prefix}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn local_wrapped_pointer_replacement_preserves_old_alias_lifetimes() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;impl Duplicate for Ref {}struct Cell {flag:bool}fn consume(value:Cell) {}";
    for (body, success) in [
        (
            "let old=Cell {flag:false};let fresh=true;let slot=old.flag.ref.as(Ref);consume(old);slot.ref.* = fresh.ref.as(Ref);slot.as(*bool).*",
            true,
        ),
        (
            "let old=Cell {flag:false};let fresh=true;let slot=old.flag.ref.as(Ref);let saved=slot;slot.ref.* = fresh.ref.as(Ref);consume(old);saved.as(*bool).*",
            false,
        ),
        (
            "let old=false;let fresh=Cell {flag:true};let slot=old.ref.as(Ref);slot.ref.* = fresh.flag.ref.as(Ref);consume(fresh);slot.as(*bool).*",
            false,
        ),
    ] {
        let source = format!("{prefix}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if success {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("unavailable"), "{body}: {error}");
        }
    }
}

#[test]
fn wrapped_pointer_replacement_accepts_branch_and_match_values() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;impl Duplicate for Ref {}fn choose()->bool {true}";
    for replacement in [
        "if choose() {a}else{b}",
        "choose() match {true=>a,false=>b}",
        "do {let value=a;value}",
        "(|| {a})()",
    ] {
        let source = format!(
            "{prefix}fn run()->bool {{let first=false;let second=true;let a=second.ref.as(Ref);let b=first.ref.as(Ref);let slot=b;slot.ref.* = {replacement};slot.as(*bool).*}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{replacement}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn ambiguous_wrapped_pointer_write_keeps_unwritten_alternatives() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}newtype Ref=*bool;impl Duplicate for Ref {}fn choose()->bool {true}struct Cell {flag:bool}fn consume(value:Cell) {}";
    for (tail, success) in [
        ("one.as(*bool).* and not two.as(*bool).*", true),
        ("consume(old);two.as(*bool).*", false),
    ] {
        let source = format!(
            "{prefix}fn run()->bool {{let old=Cell {{flag:false}};let fresh=true;let one=old.flag.ref.as(Ref);let two=old.flag.ref.as(Ref);let target=if choose() {{one.ref}}else{{two.ref}};target.* = fresh.ref.as(Ref);{tail}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if success {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn copy_data_struct_pointer_reads_and_writes_preserve_fields() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Pair {first:bool,second:bool}impl Duplicate for Pair {}newtype Wrapped=Pair;impl Duplicate for Wrapped {}fn read(p:*Pair)->Pair {p.*}fn set(p:*Pair) {p.* = Pair {first:true,second:false};}fn run()->bool {let pair=Pair {first:false,second:true};set(pair.ref);let copied=read(pair.ref);let wrapped=copied.as(Wrapped);let again=wrapped.ref.*.as(Pair);again.first and not again.second and pair.first}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn data_struct_pointer_access_requires_copy() {
    for access in [
        "fn access(p:*Cell)->Cell {p.*}",
        "fn access(p:*Cell)->Cell {p.* = Cell {flag:true};Cell {flag:false}}",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}struct Cell {{flag:bool}}{access}fn run()->bool {{let cell=Cell {{flag:true}};access(cell.ref).flag}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("checked Copy permission"),
            "{access}: {error}"
        );
    }
}

#[test]
fn copy_data_struct_pointer_access_rejects_expired_enclosing_storage() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}impl Duplicate for Cell {}struct Outer {cell:Cell}fn consume(value:Outer) {}fn read(p:*Cell)->Cell {p.*}fn run()->bool {let outer=Outer {cell:Cell {flag:true}};let pointer=outer.cell.ref;consume(outer);read(pointer).flag}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn nested_generic_copy_data_structs_are_read_without_consuming_storage() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell where T {value:T}impl Duplicate for Cell<bool> {}struct Outer {cell:Cell<bool>}impl Duplicate for Outer {}fn read(p:*Outer)->Outer {p.*}fn run()->bool {let value=Outer {cell:Cell<bool> {value:true}};read(value.ref).cell.value and read(value.ref).cell.value}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn copied_pointee_structs_support_patterns_and_guard_retry() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}struct Pair {first:bool,second:bool}impl Duplicate for Pair {}";
    for body in [
        "p.* match {{first,second}=>first and not second}",
        "p.* match {{first:flag} if false=>false,{first:flag}=>flag}",
        "true match {_ if do {let pair=p.*;pair.second}=>false,_=>p.*.first}",
    ] {
        let source = format!(
            "{prefix}fn read(p:*Pair)->bool {{{body}}}fn run()->bool {{let pair=Pair {{first:true,second:false}};read(pair.ref) and pair.first}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn canonical_tuple_data_pointer_delivery_keeps_normal_form_head() {
    let source = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}pure comptime fn Tuple<...Es:List<Type>>->Type {bool}trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for Tuple<bool,bool> {}fn read(p:*Tuple<bool,bool>)->Tuple<bool,bool> {p.*}fn set(p:*Tuple<bool,bool>) {p.* = (true,false);}fn run()->bool {let pair=(false,true);set(pair.ref);let (first,second)=read(pair.ref);first and not second}";
    let Artifact::Nir(package) = compile_selected(source, Emit::Nir, true).unwrap() else {
        panic!("wrong stage")
    };
    let read = package.functions.iter().find(|f| f.name == "read").unwrap();
    let nir::Term::Function { result, .. } = read.signature.callable_signature() else {
        panic!("function signature")
    };
    assert!(
        matches!(result.as_ref(), nir::Term::NormalForm { .. }),
        "{}",
        read.signature
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn tuple_pointer_delivery_does_not_unfold_other_heads_or_hide_aliases() {
    let prefix = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}pure comptime fn Tuple<...Es:List<Type>>->Type {bool}pure comptime fn Identity<T>->Type {T}";
    for ty in ["Identity<Tuple<bool,bool>>"] {
        let source = format!("{prefix}fn read(p:*{ty})->{ty} {{p.*}}");
        let error = compile_selected(&source, Emit::Nir, true)
            .err()
            .expect("unsupported pointee layout");
        assert!(
            error.contains("pointer reads of non-intrinsic values"),
            "{ty}: {error}"
        );
    }
    // Pointer fields are supported through declared Tuple layout and tracked
    // result provenance; copying the container must not extend their lifetime.
    let source = format!("{prefix}trait Duplicate {{}}trait Retire {{fn drop(*self);}}
        impl Duplicate for Tuple<*bool,bool> {{}}
        fn read(p:*Tuple<*bool,bool>)->Tuple<*bool,bool> {{p.*}}
        fn run()->bool {{let flag=true;let pair=(flag.ref,false);let (pointer,tag)=read(pair.ref);pointer.* and not tag}}
        fn escape()->Tuple<*bool,bool> {{let flag=true;let pair=(flag.ref,false);read(pair.ref)}}");
    let Artifact::Nir(package) = compile_selected(&source, Emit::Nir, true).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let error = luna::execution::interpret_with_protocols(
        &package,
        "escape",
        "Duplicate",
        "Retire",
        100_000,
    )
    .unwrap_err();
    assert!(
        error.contains("pointer target unavailable outside its local scope (escaping provenance)"),
        "{error}"
    );
}

#[test]
fn aggregate_pointer_replacement_invalidates_old_field_facts() {
    let prefix = "typealias Byte=.u8'builtin;struct Cell {value:Byte}fn set(p:*Cell) {p.* = Cell {value:255};}";
    for write in ["cell.ref.* = Cell {value:255};", "set(cell.ref);"] {
        let source =
            format!("{prefix}fn run()->Byte {{let cell=Cell {{value:0}};{write}cell.value + 1}}");
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("overflow must not use stale field facts");
        assert!(
            error.contains("machine arithmetic range is not proven"),
            "{write}: {error}"
        );
    }
}

#[test]
fn aggregate_pointer_read_copies_field_facts_before_replacement() {
    let source = "typealias Byte=.u8'builtin;trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {value:Byte}impl Duplicate for Cell {}fn run()->Byte {let cell=Cell {value:0};let saved=cell.ref.*;cell.ref.* = Cell {value:255};saved.value + 1}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::UnsignedInteger(1)
    );
}

#[test]
fn ambiguous_aggregate_write_does_not_replace_all_candidate_facts() {
    let source = "typealias Byte=.u8'builtin;struct Cell {value:Byte}fn run(flag:bool)->Byte {let first=Cell {value:255};let second=Cell {value:255};let target=if flag {first.ref}else{second.ref};target.* = Cell {value:0};first.value + 1}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("one candidate may still contain 255");
    assert!(
        error.contains("machine arithmetic range is not proven"),
        "{error}"
    );
}

#[test]
fn copy_enum_pointer_delivery_preserves_tags_and_payloads() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}enum Choice {yes(bool),no}impl Duplicate for Choice {}newtype Wrapped=Choice;impl Duplicate for Wrapped {}fn read(p:*Choice)->Choice {p.*}fn set(p:*Choice) {p.* = Choice.yes(true);}fn run()->bool {let value=Choice.no;set(value.ref);let saved=read(value.ref);value.ref.* = Choice.no;saved match {.yes(flag)=>flag,.no=>false}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn enum_pointer_tag_evidence_tracks_copy_and_replacement() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}enum Choice {yes,no}impl Duplicate for Choice {}newtype Wrapped=Choice;impl Duplicate for Wrapped {}struct Token {}fn take(value:Token) {}";
    for (body, success) in [
        (
            "let value=Choice.yes;let saved=value.ref.*;value.ref.* = Choice.no;saved match {.yes=>true,.no=>do {take(token);take(token);false}}",
            true,
        ),
        (
            "let value=Choice.yes;value.ref.* = Choice.no;value match {.no=>true,.yes=>do {take(token);take(token);false}}",
            true,
        ),
        (
            "let value=Choice.yes.as(Wrapped);let saved=value.ref.*;value.ref.* = Choice.no.as(Wrapped);saved.as(Choice) match {.yes=>true,.no=>do {take(token);take(token);false}}",
            true,
        ),
        (
            "let value=Choice.yes;value.ref.* = Choice.no;value match {.yes=>true,.no=>do {take(token);take(token);false}}",
            false,
        ),
    ] {
        let source = format!("{prefix}fn run()->bool {{let token=Token {{}};{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if success {
            assert_eq!(
                result.unwrap_or_else(|e| panic!("{body}: {e}")),
                comptime_interpretor::Value::Bool(true)
            );
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("unavailable"), "{error}");
        }
    }
}

#[test]
fn enum_pointer_delivery_requires_copy_and_preserves_payload_provenance() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}enum Choice {yes,no}fn read(p:*Choice)->Choice {p.*}fn run()->bool {let value=Choice.yes;read(value.ref) match {.yes=>true,.no=>false}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(error.contains("checked Copy permission"), "{error}");
    let source="trait Duplicate {}trait Retire {fn drop(*self);}
        enum Choice {none,some(*bool)}impl Duplicate for Choice {}
        fn read(p:*Choice)->Choice {p.*}
        fn run()->bool {let flag=true;let value=Choice.some(flag.ref);read(value.ref) match {.none=>false,.some(pointer)=>pointer.*}}
        fn escape()->Choice {let flag=true;let value=Choice.some(flag.ref);read(value.ref)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let error = luna::execution::interpret_with_protocols(
        &package,
        "escape",
        "Duplicate",
        "Retire",
        100_000,
    )
    .unwrap_err();
    assert!(
        error.contains("pointer target unavailable outside its local scope (escaping provenance)"),
        "{error}"
    );
}

#[test]
fn ambiguous_enum_pointer_write_keeps_old_tag_alternatives() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}enum Choice {yes,no}impl Duplicate for Choice {}struct Token {}fn take(value:Token) {}fn choose()->bool {true}";
    for (tail, success) in [
        ("second match {.yes=>true,.no=>false}", true),
        (
            "second match {.yes=>do {take(token);take(token);false},.no=>true}",
            false,
        ),
    ] {
        let source = format!(
            "{prefix}fn run()->bool {{let token=Token {{}};let first=Choice.no;let second=Choice.yes;let p=if choose() {{first.ref}}else{{second.ref}};p.* = Choice.no;{tail}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if success {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("cannot prepare resource-checked execution")
                    && error.contains("unavailable"),
                "{error}"
            );
        }
    }
}

#[test]
fn enum_field_pointer_write_preserves_sibling_and_snapshot_tags() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}enum Choice {yes,no}impl Duplicate for Choice {}struct Pair {left:Choice,right:Choice}impl Duplicate for Pair {}struct Token {}fn take(value:Token) {}fn run()->bool {let token=Token {};let pair=Pair {left:Choice.no,right:Choice.yes};let saved=pair.ref.*;pair.left.ref.* = Choice.yes;let left=pair.left match {.yes=>true,.no=>do {take(token);take(token);false}};let right=pair.right match {.yes=>true,.no=>do {take(token);take(token);false}};let old=saved.left match {.no=>true,.yes=>do {take(token);take(token);false}};left and right and old}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn local_function_pointer_reads_preserve_callable_identity() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}newtype Wrapped=Action;impl Duplicate for Wrapped {}fn yes()->bool {true}";
    for body in [
        "let action=yes;let loaded=action.ref.*;loaded()",
        "let action=yes;(action.ref.*)() and (action.ref.*)()",
        "let action=yes.as(Wrapped);let loaded=action.ref.*;loaded.as(Action)()",
    ] {
        let source = format!("{prefix}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn function_pointer_reads_merge_candidates_and_require_copy() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;fn yes()->bool {true}fn no()->bool {false}fn choose()->bool {true}fn replace(p:*Action) {p.* = no;}";
    let source = format!(
        "{prefix}impl Duplicate for Action {{}}fn run()->bool {{let left=yes;let right=no;let p=if choose() {{left.ref}}else{{right.ref}};let action=p.*;action()}}"
    );
    let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for (implementation, body, diagnostic) in [
        (
            "",
            "let action=yes;let loaded=action.ref.*;loaded()",
            "checked Copy permission",
        ),
        (
            "impl Duplicate for Action {}",
            "let action=yes;replace(action.ref);action()",
            "callable transition contract",
        ),
    ] {
        let source = format!("{prefix}{implementation}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn local_function_slot_replacement_updates_identity_and_keeps_old_copies() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}newtype Wrapped=Action;impl Duplicate for Wrapped {}fn yes()->bool {true}fn no()->bool {false}fn choose()->bool {true}";
    for body in [
        "let action=no;action.ref.* = yes;action()",
        "let action=yes;let saved=action;action.ref.* = no;saved() and not action()",
        "let action=no.as(Wrapped);action.ref.* = yes.as(Wrapped);action.as(Action)()",
        "let first=no;let second=no;let p=if choose() {first.ref}else{second.ref};p.* = yes;first() and not second()",
    ] {
        let source = format!("{prefix}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn function_slot_replacement_cannot_validate_using_the_old_target() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}struct Token {}fn take(value:Token) {}fn good()->bool {true}fn bad()->bool {let token=Token {};take(token);take(token);false}fn run()->bool {let action=good;action.ref.* = bad;action()}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution")
            && error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn copy_callable_aggregates_preserve_field_identities_through_pointer_delivery() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}struct Table {first:Action,second:Action}impl Duplicate for Table {}enum Choice {one(Action),two(Action)}impl Duplicate for Choice {}fn yes()->bool {true}fn no()->bool {false}";
    for body in [
        "let table=Table {first:yes,second:no};let saved=table.ref.*;table.ref.* = Table {first:no,second:yes};saved.first() and not table.first() and table.second()",
        "let value=Choice.one(yes);let saved=value.ref.*;value.ref.* = Choice.two(no);let old=saved match {.one(f)=>f(),.two(f)=>not f()};let new=value match {.one(f)=>not f(),.two(f)=>not f()};old and new",
    ] {
        let source = format!("{prefix}fn run()->bool {{{body}}}");
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap_or_else(|e| panic!("{body}: {e}")),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn ambiguous_callable_enum_write_keeps_variant_specific_code() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}enum Choice {one(Action),two(Action)}impl Duplicate for Choice {}fn yes()->bool {true}fn no()->bool {false}fn choose()->bool {true}fn run()->bool {let first=Choice.one(no);let second=Choice.one(no);let target=if choose() {first.ref}else{second.ref};target.* = Choice.two(yes);let saved=second.ref.*;saved match {.one(f)=>not f(),.two(f)=>f()}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn callable_aggregate_pointer_write_does_not_retain_old_field_identity() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}struct Table {action:Action}impl Duplicate for Table {}struct Token {}fn take(value:Token) {}fn good()->bool {true}fn bad()->bool {let token=Token {};take(token);take(token);false}fn run()->bool {let table=Table {action:good};table.ref.* = Table {action:bad};let saved=table.ref.*;saved.action()}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution")
            && error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn pure_direct_private_pointer_access_has_no_external_storage_effect() {
    for body in [
        "let flag=true;flag.ref.*",
        "let flag=false;flag.ref.* = true;flag",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for body in ["p.*", "p.* = true;true", "let local=p;local.ref.*.*"] {
        let source = format!("pure fn read(p:*bool)->bool {{{body}}}");
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("external storage is not P4");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
    let error = compile(
        "struct Item {flag:bool}pure fn read(value:Item)->Item {value.ref.*}",
        Emit::Nir,
    )
    .err()
    .expect("private storage is not implicit Copy");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_private_field_pointer_access_preserves_owned_place_boundary() {
    for body in [
        "if cell.ref.*.flag {cell}else{cell}",
        "cell.ref.*.flag = true;cell",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}struct Cell {{flag:bool}}pure fn access(cell:Cell)->Cell {{{body}}}fn run()->bool {{access(Cell {{flag:true}}).flag}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    let source = "struct Cell {pointer:*bool}pure fn read(cell:Cell)->Cell {if cell.ref.*.pointer.* {cell}else{cell}}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("field projection does not privatize its pointee");
    assert!(error.contains("P4 evidence"), "{error}");
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Inner {flag:bool}struct Cell {inner:Inner}pure fn set(cell:Cell)->Cell {cell.ref.*.inner.flag = true;cell}fn run()->bool {set(Cell {inner:Inner {flag:false}}).inner.flag}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn pure_conditional_private_addresses_check_targets_and_condition() {
    for (body, when_false) in [
        (
            "let first=true;let second=false;(if flag {first.ref}else{second.ref}).*",
            false,
        ),
        (
            "let first=false;let second=false;(if flag {first.ref}else{second.ref}).* = true;first or second",
            true,
        ),
        (
            "var first=false;let second=true;(if flag {first=true;first.ref}else{second.ref}).*",
            true,
        ),
    ] {
        for (flag, expected) in [(true, true), (false, when_false)] {
            let source = format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool)->bool {{{body}}}fn run()->bool {{access({flag})}}"
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
                panic!("wrong stage")
            };
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(expected)
            );
        }
    }
    for body in [
        "let local=true;(if flag {local.ref}else{external}).*",
        "let first=false;let second=false;(if change(external) {first.ref}else{second.ref}).* = true;true",
        "let local=false;local.ref.* = change(external);local",
        "let first=false;let second=false;(if flag {change(external);first.ref}else{second.ref}).*",
    ] {
        let source = format!(
            "fn change(p:*bool)->bool {{p.* = true;true}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("external access must not be hidden in private address selection");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_match_private_addresses_check_guards_and_all_targets() {
    for flag in [true, false] {
        for body in [
            "let first=true;let second=true;(flag match {true=>first.ref,false=>second.ref}).*",
            "let first=false;let second=false;(flag match {_ if flag=>first.ref,_=>second.ref}).* = true;first or second",
        ] {
            let source = format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool)->bool {{{body}}}fn run()->bool {{access({flag})}}"
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
                panic!("wrong stage")
            };
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        }
    }
    for body in [
        "let local=true;(flag match {true=>local.ref,false=>external}).*",
        "let first=false;let second=false;(flag match {_ if change(external)=>first.ref,_=>second.ref}).* = true;true",
        "let first=true;let second=true;(change(external) match {true=>first.ref,false=>second.ref}).*",
        "let first=true;let second=true;(flag match {true=>do {change(external);first.ref},false=>second.ref}).*",
    ] {
        let source = format!(
            "fn change(p:*bool)->bool {{p.* = true;true}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("match must retain external effects");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_private_pointer_bindings_keep_storage_identity() {
    for body in [
        "let value=true;let p=value.ref;p.*",
        "let value=false;let p=value.ref;let q=p;q.* = true;value",
        "let value=false;let p=value.ref;if true {p.* = true;}value",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for body in [
        "let p=external;p.*",
        "let value=true;var p=value.ref;p=external;p.*",
        "let value=true;var p=value.ref;while flag {let observed=p.*;p=external;}true",
        "let value=external;let p=value.ref;p.*.*",
        "let value=true;let p=value.ref;read(external)",
    ] {
        let source = format!(
            "fn read(p:*bool)->bool {{p.*}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("external storage cannot inherit private alias evidence");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_private_aliases_preserve_owner_and_escape_checks() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}pure fn set(cell:Cell)->Cell {let p=cell.flag.ref;p.* = true;cell}fn run()->bool {set(Cell {flag:false}).flag}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}pure fn escape()->*bool {let value=true;let p=value.ref;p}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("private aliases do not permit escaping local addresses");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_conditional_aliases_keep_all_targets_without_replaying_initializers() {
    for flag in [true, false] {
        for body in [
            "let a=true;let b=true;let p=if flag {a.ref}else{b.ref};let q=p;q.*",
            "let a=false;let b=false;let p=flag match {true=>a.ref,false=>b.ref};p.* = true;a or b",
            "let a=false;let b=false;var count: .u8'builtin = 0;let p=if flag {count = count + 1;a.ref}else{count = count + 1;b.ref};p.* = true;let observed=p.*;observed and count == 1",
        ] {
            let source = format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool)->bool {{{body}}}fn run()->bool {{access({flag})}}"
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
                panic!("wrong stage")
            };
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        }
    }
    for body in [
        "let a=true;let p=if flag {a.ref}else{external};p.*",
        "let a=true;let b=true;let p=if change(external) {a.ref}else{b.ref};p.*",
        "let a=true;let b=true;let p=flag match {_ if change(external)=>a.ref,_=>b.ref};p.*",
        "let a=true;let b=true;let p=if flag {change(external);a.ref}else{b.ref};p.*",
    ] {
        let source = format!(
            "fn change(p:*bool)->bool {{p.* = true;true}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("alias cannot hide external targets or initializer effects");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_conditional_code_aliases_join_every_callable_candidate() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}fn yes()->bool {true}fn also_yes()->bool {true}pure fn access(flag:bool)->bool {let a=yes;let b=also_yes;let p=if flag {a.ref}else{b.ref};p.*()}fn run()->bool {access(true) and access(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}fn yes()->bool {true}pure fn access(flag:bool,unknown:Action)->bool {let a=yes;let b=unknown;let p=if flag {a.ref}else{b.ref};p.*()}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("unknown callable alternative must prevent P4");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_block_alias_results_preserve_sources_across_scope_exit() {
    for flag in [true, false] {
        for body in [
            "let value=true;let p=do {let q=value.ref;q};p.*",
            "let value=false;let p=do {let q=value.ref;do {let r=q;r}};p.* = true;value",
            "let a=false;let b=false;let p=if flag {let q=a.ref;q}else{let q=b.ref;q};p.* = true;a or b",
            "let a=true;let b=true;let p=flag match {true=>do {let q=a.ref;q},false=>do {let q=b.ref;q}};p.*",
        ] {
            let source = format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool)->bool {{{body}}}fn run()->bool {{access({flag})}}"
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
                panic!("wrong stage")
            };
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        }
    }
    for body in [
        "let p=do {let value=true;value.ref};p.*",
        "let p=do {let q=external;q};p.*",
        "let local=true;let p=if flag {let q=local.ref;q}else{let q=external;q};p.*",
        "let local=external;let p=do {let q=local.ref;q};p.*.*",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("scope exit cannot extend target lifetime or privatize external storage");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_block_address_projection_reads_current_callable_evidence() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}pure fn set(cell:Cell)->Cell {(do {let p=cell.ref;p}).*.flag = true;cell}fn run()->bool {set(Cell {flag:false}).flag}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}fn yes()->bool {true}pure fn access(unknown:Action)->bool {var callback=yes;let p=do {let q=callback.ref;q};callback=unknown;p.*()}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("address evidence is not a snapshot of the original callable");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_private_pointer_reassignment_widens_storage_targets() {
    for flag in [true, false] {
        for body in [
            "let a=false;let b=true;var p=a.ref;p=b.ref;p.*",
            "let a=false;let b=false;var p=a.ref;let q=p;p=b.ref;q.* = true;a and not b",
            "let a=false;let b=false;var p=a.ref;if flag {p=b.ref;}p.* = true;a or b",
            "let a=false;let b=false;var p=a.ref;flag match {true=>do {p=b.ref;},false=>do {p=a.ref;}};p.* = true;a or b",
        ] {
            let source = format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool)->bool {{{body}}}fn run()->bool {{access({flag})}}"
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
                panic!("wrong stage")
            };
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        }
    }
    for body in [
        "let a=true;var p=a.ref;if flag {p=external;}p.*",
        "let a=true;var p=a.ref;if flag {p=external;}else{p=a.ref;}p.*",
        "let a=true;var p=a.ref;if flag {p=a.ref;}else{p=external;}p.*",
        "let a=true;var p=a.ref;flag match {true=>do {p=external;},false=>do {p=a.ref;}};p.*",
        "let a=true;var p=a.ref;while flag {let observed=p.*;p=external;}true",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("every assignment path must retain private storage evidence");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_retargeted_code_pointers_check_new_candidates() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}fn no()->bool {false}fn yes()->bool {true}pure fn run()->bool {let a=no;let b=yes;var p=a.ref;p=b.ref;p.*()}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}fn yes()->bool {true}pure fn access(unknown:Action)->bool {let a=yes;var p=a.ref;p=unknown.ref;p.*()}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("new pointer target carries its own callable evidence");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn pure_loop_private_aliases_close_backedge_targets() {
    for body in [
        "let a=false;let b=true;var p=a.ref;var n: .u8'builtin = 2;while n > 0 {p=b.ref;n = n - 1;}p.*",
        "let a=false;let b=true;var p=a.ref;var q=a.ref;var n: .u8'builtin = 2;while n > 0 {p=q;q=b.ref;n = n - 1;}p.*",
        "let a=false;let b=false;var p=a.ref;var n: .u8'builtin = 2;while n > 0 {p.* = true;p=b.ref;n = n - 1;}a and b",
        "let a=true;let b=false;var p=a.ref;while false {p=b.ref;}p.*",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for body in [
        "let a=true;var p=a.ref;while flag {let observed=p.*;p=external;}true",
        "let a=true;var p=a.ref;var q=a.ref;while flag {let observed=p.*;p=q;q=external;}true",
        "let a=true;var p=a.ref;while flag {while flag {p=external;}let observed=p.*;}true",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("later loop iterations must not inherit first-iteration private evidence");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_pointer_patterns_preserve_private_sources_and_guard_checks() {
    for body in [
        "let value=true;value.ref match {p=>p.*}",
        "let value=true;value.ref match {p if p.* => p.*,_=>false}",
        "let value=false;value.ref match {p=>do {p.* = true;}};value",
        "let value=true;value.ref match {(p or p)=>p.*}",
        "let value=true;let (p)=value.ref;p.*",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{body}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for body in [
        "external match {p=>p.*}",
        "let value=true;value.ref match {p if change(external)=>p.*,_=>false}",
        "let value=external;value.ref match {p=>p.*.*}",
    ] {
        let source = format!(
            "fn change(p:*bool)->bool {{p.* = true;true}}pure fn access(external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("pattern bindings cannot hide external storage or guard effects");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn computed_pointer_patterns_retain_resource_origins() {
    for body in [
        "let value=true;false match {_ and value.ref is p=>p.*}",
        "let value=true;let _ and value.ref is p=false;p.*",
        "let a=false;let b=true;false match {(_ and a.ref is p if false) or (_ and b.ref is p)=>p.*}",
        "let value=false;false match {_ and value.ref is p=>do {p.* = true;}};value",
        "let a=true;let b=true;false match {_ and (if true {a.ref}else{b.ref}) is p=>p.*}",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn run()->bool {{{body}}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{body}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn computed_pointer_patterns_do_not_revive_moved_storage() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}fn run()->bool {let value=Cell {flag:true};let _ and value.flag.ref is p=false;let moved=value;p.*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution")
            && error.contains("unavailable"),
        "{error}"
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}pure fn access(external:*bool)->bool {false match {_ and external is p=>p.*}}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("computed pointer binding does not privatize an external target");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn computed_pointer_pattern_returns_preserve_call_contract_origins() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn forward(p:*bool)->*bool {false match {(_ and p is q if false) or (_ and p is q)=>q}}fn run()->bool {let value=true;forward(value.ref).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn escape()->*bool {let value=true;false match {_ and value.ref is p=>p}}fn run()->bool {escape().*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn computed_pointer_alternatives_union_return_origins() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}fn choose(flag:bool,a:*bool,b:*bool)->*bool {false match {(_ and a is p if flag) or (_ and b is p)=>p}}fn run()->bool {let a=Cell {flag:true};let b=Cell {flag:false};let p=choose(true,a.flag.ref,b.flag.ref);let moved=a;p.*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution")
            && error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn computed_pointer_conditions_are_checked_before_origin_union() {
    for condition in ["not false", "decide()", "do {let flag=true;flag}"] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}fn decide()->bool {{true}}pure fn run()->bool {{let a=true;let b=true;false match {{_ and (if {condition} {{a.ref}}else{{b.ref}}) is p=>p.*}}}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{condition}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for source in [
        "trait Duplicate {}trait Retire {fn drop(*self);}fn change(p:*bool)->bool {p.* = true;true}fn run()->bool {let a=true;let b=true;let external=false;false match {_ and (if change(external.ref) {a.ref}else{b.ref}) is p=>p.*}}",
        "trait Duplicate {}trait Retire {fn drop(*self);}fn run()->bool {let a=true;let b=false;var slot=a.ref;false match {_ and (if do {slot=b.ref;true} {slot}else{a.ref}) is p=>p.*}}",
    ] {
        let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{error}"
        );
    }
}

#[test]
fn computed_conditional_pointer_projections_keep_all_field_targets() {
    for flag in [true, false] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}struct Cell {{flag:bool}}fn decide(flag:bool)->bool {{flag}}fn run()->bool {{let a=Cell {{flag:true}};let b=Cell {{flag:false}};false match {{_ and (if decide({flag}) {{a.ref}}else{{b.ref}}).*.flag.ref is p=>p.*}}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
    }
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}fn decide()->bool {true}fn run()->bool {let a=Cell {flag:true};let b=Cell {flag:false};let _ and (if decide() {a.ref}else{b.ref}).*.flag.ref is p=false;let moved=a;p.*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution")
            && error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn computed_conditional_pointer_slot_loads_keep_pointee_origins() {
    for flag in [true, false] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}fn decide(flag:bool)->bool {{flag}}fn run()->bool {{let a=true;let b=false;let pa=a.ref;let pb=b.ref;false match {{_ and (if decide({flag}) {{pa.ref}}else{{pb.ref}}).* is p=>p.*}}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
    }
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}fn decide()->bool {true}fn run()->bool {let a=Cell {flag:true};let b=Cell {flag:false};let pa=a.flag.ref;let pb=b.flag.ref;let _ and (if decide() {pa.ref}else{pb.ref}).* is p=false;let moved=a;p.*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution")
            && error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn computed_pointer_result_patterns_keep_guards_and_nested_computations() {
    for flag in [true, false] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}fn select(flag:bool,a:*bool,b:*bool)->*bool {{false match {{_ and a is (p if flag)=>p,_=>b}}}}fn run()->bool {{let a=true;let b=false;select({flag},a.ref,b.ref).*}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(flag)
        );
    }
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn select(a:*bool,b:*bool)->*bool {false match {_ and a is (p and b is q)=>q}}fn run()->bool {let a=false;let b=true;select(a.ref,b.ref).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn change(p:*bool)->bool {p.* = true;true}fn select(a:*bool,b:*bool)->*bool {false match {_ and a is (p if change(b))=>p,_=>b}}fn run()->bool {let a=true;let b=false;select(a.ref,b.ref).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let select = package
        .functions
        .iter()
        .find(|function| function.name == "select")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        select.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert!(
        summary.accesses.iter().any(|access| access.write),
        "memory summary must retain the computed-result guard's write footprint"
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn pointer_subject_patterns_preserve_computations_and_return_origins() {
    for flag in [true, false] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}fn select(a:*bool,b:*bool)->*bool {{a match {{p and p.* is true=>p,_=>b}}}}fn run()->bool {{let a={flag};let b=true;select(a.ref,b.ref).*}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn change(p:*bool)->bool {p.* = true;true}fn select(a:*bool,b:*bool)->*bool {a match {p and change(p) is true=>p,_=>b}}fn run()->bool {let a=false;let b=true;select(a.ref,b.ref).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let select = package
        .functions
        .iter()
        .find(|function| function.name == "select")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        select.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert!(
        summary.accesses.iter().any(|access| access.write),
        "computed observation write must remain in the memory summary: {summary}"
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn pointer_subject_computed_bindings_use_their_own_sources() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn select(a:*bool,b:*bool)->*bool {a match {p and b is q=>q}}fn run()->bool {let a=false;let b=true;select(a.ref,b.ref).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}struct Cell {flag:bool}fn select(a:*bool,b:*bool)->*bool {a match {p and b is q=>q}}fn run()->bool {let a=Cell {flag:false};let b=Cell {flag:true};let p=select(a.flag.ref,b.flag.ref);let moved=b;p.*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution")
            && error.contains("unavailable"),
        "{error}"
    );
}

#[test]
fn pointer_let_patterns_preserve_success_and_failure_return_origins() {
    for flag in [true, false] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}fn select(a:*bool,b:*bool)->*bool {{let p and p.* is true=a else {{return b;}};p}}fn run()->bool {{let a={flag};let b=true;select(a.ref,b.ref).*}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                100_000
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn select(a:*bool,b:*bool)->*bool {let p and b is q=a;q}fn run()->bool {let a=false;let b=true;select(a.ref,b.ref).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn pointer_let_else_keeps_failure_effects_and_all_result_sources() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}fn select(a:*bool,b:*bool)->*bool {let p and p.* is true=a else {b.* = true;return b;};p}fn run()->bool {let a=false;let b=false;select(a.ref,b.ref).*}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    let select = package
        .functions
        .iter()
        .find(|function| function.name == "select")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        select.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert!(
        summary.accesses.iter().any(|access| access.write),
        "failure-path write must remain visible: {summary}"
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for moved in ["a", "b"] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}struct Cell {{flag:bool}}fn select(a:*bool,b:*bool)->*bool {{let p and p.* is true=a else {{return b;}};p}}fn run()->bool {{let a=Cell {{flag:false}};let b=Cell {{flag:true}};let p=select(a.flag.ref,b.flag.ref);let moved={moved};p.*}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution")
                && error.contains("unavailable"),
            "{moved}: {error}"
        );
    }
}

#[test]
fn pure_let_else_checks_both_paths_with_separate_binding_evidence() {
    for flag in [true, false] {
        for body in [
            "let true=flag else {return true;};true",
            "let value=true;let (p if flag)=value.ref else {return true;};p.*",
            "let value=true;var p=value.ref;let true=flag else {let other=false;p=other.ref;return true;};p.*",
        ] {
            let source = format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool)->bool {{{body}}}fn run()->bool {{access({flag})}}"
            );
            let Artifact::Nir(package) =
                compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{body}: {error}"))
            else {
                panic!("wrong stage")
            };
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    100_000
                )
                .unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        }
    }
    for body in [
        "let true=flag else {return external.*;};true",
        "let true=flag else {external.* = true;return true;};true",
        "let true=flag else {return true;};external.*",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}pure fn access(flag:bool,external:*bool)->bool {{{body}}}"
        );
        let error = compile(&source, Emit::Nir)
            .err()
            .expect("failure and success paths both require P4");
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}

#[test]
fn pure_let_else_returned_callbacks_include_failure_candidates() {
    let source = "trait Duplicate {}trait Retire {fn drop(*self);}typealias Action=fn()->bool;impl Duplicate for Action {}fn yes()->bool {true}fn no()->bool {false}pure fn choose(flag:bool)->Action {let true=flag else {return no;};yes}pure fn run()->bool {choose(true)() and not choose(false)()}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage")
    };
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 100_000)
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let source = "typealias Action=fn()->bool;fn yes()->bool {true}pure fn choose(flag:bool,fallback:Action)->Action {let true=flag else {return fallback;};yes}pure fn access(flag:bool,unknown:Action)->bool {choose(flag,unknown)()}";
    let error = compile(source, Emit::Nir)
        .err()
        .expect("a failure-returned unknown callback must remain unknown");
    assert!(error.contains("P4 evidence"), "{error}");
}

#[test]
fn nested_newtypes_capture_self_and_outer_type_parameters() {
    for source in [
        "trait Identity {newtype Wrapped=Self;fn identity(self)->Self {self.as(Wrapped).as(Self)}}impl Identity for bool {}fn run()->bool {true.as(Identity).identity()}",
        "trait Identity where T {newtype Wrapped=T;fn identity(self,value:T)->T {value.as(Wrapped).as(T)}}impl Identity<bool> for bool {}fn run()->bool {false.as(Identity<bool>).identity(true)}",
        "struct Holder where T {value:T,}impl Holder<T> where T {newtype Wrapped=T;fn identity(self)->T {self.value.as(Wrapped).as(T)}}fn run()->bool {Holder<bool>{value:true}.identity()}",
        "trait Identity {newtype Wrapped<T> = T;fn identity(self)->Self {self.as(Wrapped<Self>).as(Self)}}impl Identity for bool {}fn run()->bool {true.as(Identity).identity()}",
    ] {
        let Artifact::Nir(package) =
            compile(source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn nested_newtype_dynamic_construction_retains_lexical_identity() {
    for (parameters, application, expected) in [
        ("T", "Wrapper<value>", nir::Primitive::Char),
        (".T:Type=Self", "Wrapper<.T=value>", nir::Primitive::Char),
        (".T:Type=Self", "Wrapper", nir::Primitive::Bool),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;trait Factory {{newtype Wrapper<{parameters}> = T;comptime fn Wrapped(self,value:Type)->Type {{{application}}}}}impl Factory for bool {{}}impl Factory for .char'builtin {{}}comptime fn run()->Type {{true.as(Factory).Wrapped(.char'builtin)}}comptime fn other()->Type {{'x'.as(Factory).Wrapped(.char'builtin)}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("wrong stage")
        };
        let run = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .declaration;
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let first = vm.invoke(run, vec![]).unwrap();
        assert_eq!(
            first,
            comptime_interpretor::Value::Type(nir::Term::Nominal {
                declaration: package.newtypes[0].declaration,
                arguments: vec![
                    nir::Term::Primitive(expected),
                    nir::Term::Primitive(nir::Primitive::Bool)
                ],
            })
        );
        let other = package
            .functions
            .iter()
            .find(|f| f.name == "other")
            .unwrap()
            .declaration;
        let second = vm.invoke(other, vec![]).unwrap();
        assert_eq!(
            second,
            comptime_interpretor::Value::Type(nir::Term::Nominal {
                declaration: package.newtypes[0].declaration,
                arguments: vec![
                    nir::Term::Primitive(nir::Primitive::Char),
                    nir::Term::Primitive(nir::Primitive::Char)
                ],
            })
        );
        assert_ne!(
            first, second,
            "distinct captured Self types must retain distinct nominal identities"
        );
    }
}

#[test]
fn nominal_body_newtypes_capture_their_enclosing_type_environment() {
    for source in [
        "struct Holder where T {value:T,newtype Wrapped=T;fn get(self)->T {self.value.as(Wrapped).as(T)}}fn run()->bool {Holder<bool>{value:true}.get()}",
        "struct Holder where T {value:T,newtype Wrapped=Self;fn get(self)->T {self.as(Wrapped).as(Self).value}}fn run()->bool {Holder<bool>{value:true}.get()}",
        "struct Holder where T {value:T,typealias Payload=T;newtype Inner=Payload;newtype Outer=Inner;fn get(self)->T {self.value.as(Inner).as(Outer).as(Inner).as(T)}}fn run()->bool {Holder<bool>{value:true}.get()}",
        "enum Holder where T {Value(T),newtype Wrapped=T;fn get(self)->T {let .Value(value)=self;value.as(Wrapped).as(T)}}fn run()->bool {Holder<bool>.Value(true).get()}",
        "enum Holder where T {Value(T),newtype Wrapped=Self;fn get(self)->T {let .Value(value)=self.as(Wrapped).as(Self);value}}fn run()->bool {Holder<bool>.Value(true).get()}",
    ] {
        let Artifact::Nir(package) =
            compile(source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn nominal_body_type_definitions_inherit_formation_premises() {
    for (constraint, prerequisite) in [
        ("T, requires T == bool", "T, requires T == bool"),
        ("T :- Allowed", "T :- Allowed"),
    ] {
        for container in ["struct", "enum"] {
            let prefix = format!(
                "trait Allowed {{}}impl Allowed for bool {{}}struct Checked where {constraint} {{value:T,}}"
            );
            let (payload, construction) = if container == "struct" {
                ("", "Holder<bool>{}")
            } else {
                ("Ready,", "Holder<bool>.Ready")
            };
            let body = format!(
                "{container} Holder where {prerequisite} {{{payload}typealias Payload=Checked<T>;newtype Wrapped=Payload;fn get(self,value:Payload)->T {{value.as(Wrapped).as(Payload).value}}}}fn run()->bool {{{construction}.get(Checked<bool>{{value:true}})}}"
            );
            let source = format!("{prefix}{body}");
            let Artifact::Nir(package) =
                compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
            else {
                panic!("wrong stage")
            };
            assert_eq!(
                luna::execution::interpret(&package, "run", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            let invalid = format!(
                "{prefix}{}",
                body.replace(&format!("where {prerequisite}"), "where T")
            );
            let error = compile(&invalid, Emit::Nir)
                .err()
                .expect("a concrete bool caller must not establish a missing generic premise");
            assert!(error.contains("elaboration failed"), "{invalid}: {error}");
        }
    }
}

#[test]
fn parameterized_type_member_projection_preserves_environment_and_identity() {
    for source in [
        "struct Holder where T {pub typealias Payload=T;pub newtype Wrapped=T;}fn wrap(value:Holder<T>.Payload)->Holder<T>.Wrapped where T {value.as(Holder<T>.Wrapped)}fn run()->bool {wrap<bool>(true).as(bool)}",
        "struct Holder where T {pub newtype Wrapped=T;}fn run()->bool {true.as(Holder<bool>.Wrapped).as(bool)}",
        "struct Holder where T {pub typealias Payload=T;}fn flag(value:Holder<bool>.Payload)->bool {value}fn letter(value:Holder<.char'builtin>.Payload)->.char'builtin {value}fn run()->bool {flag(true) and letter('x') == 'x'}",
        "struct Holder where T {value:T,pub typealias This=Self;pub newtype Wrapped=This;}fn run()->bool {Holder<bool>{value:true}.as(Holder<bool>.Wrapped).as(Holder<bool>).value}",
        "struct Holder where T {pub newtype Wrapped<U> = U;}fn run()->bool {true.as(Holder<.char'builtin>.Wrapped<bool>).as(bool)}",
        "struct Holder where T {pub newtype Wrapped<.U:.Type'builtin=T> = U;}fn run()->bool {true.as(Holder<bool>.Wrapped).as(bool) and true.as(Holder<.char'builtin>.Wrapped<.U=bool>).as(bool)}",
        "enum Holder where T {Value(T),pub newtype Wrapped=T;}fn run()->bool {true.as(Holder<bool>.Wrapped).as(bool)}",
        "trait Allowed {}impl Allowed for bool {}struct Holder where T :- Allowed {pub typealias Payload=T;pub newtype Wrapped=Payload;}fn run()->bool {true.as(Holder<bool>.Wrapped).as(Holder<bool>.Payload)}",
        "struct Holder where T, requires T == bool {pub newtype Wrapped=T;}fn identity(value:Holder<bool>.Wrapped)->Holder<bool>.Wrapped {value}fn run()->bool {identity(true.as(Holder<bool>.Wrapped)).as(bool)}",
    ] {
        let Artifact::Nir(package) =
            compile(source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("wrong stage")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{source}"
        );
    }
    let source = "struct Holder where T {pub newtype Wrapped<U> = U;}fn invalid(value:Holder<bool>.Wrapped<bool>)->Holder<.char'builtin>.Wrapped<bool> {value}";
    let failure = compile(source, Emit::Nir)
        .err()
        .expect("different environments merged");
    assert!(failure.contains("type mismatch"), "{failure}");
}

#[test]
fn projected_type_factories_retain_dynamic_arguments_and_captured_environment() {
    for application in [
        "Holder<bool>.Wrapped<value>",
        "Holder<bool>.Alias<value>",
        "Holder<bool>.Defaulted<.U=value>",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;struct Holder where T {{pub newtype Wrapped<U> = U;pub typealias Alias<U> = Wrapped<U>;pub newtype Defaulted<.U:Type=T> = U;}}comptime fn Build(value:Type)->Type {{{application}}}comptime fn Run()->Type {{Build(.char'builtin)}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("NIR")
        };
        let run = package
            .functions
            .iter()
            .find(|f| f.name == "Run")
            .unwrap()
            .declaration;
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let declaration = package
            .newtypes
            .iter()
            .find(|ty| {
                ty.name
                    == if application.contains("Defaulted") {
                        "Defaulted"
                    } else {
                        "Wrapped"
                    }
            })
            .unwrap()
            .declaration;
        assert_eq!(
            vm.invoke(run, vec![]).unwrap(),
            comptime_interpretor::Value::Type(nir::Term::Nominal {
                declaration,
                arguments: vec![
                    nir::Term::Primitive(nir::Primitive::Char),
                    nir::Term::Primitive(nir::Primitive::Bool)
                ],
            })
        );
    }
}

#[test]
fn projected_type_factories_evaluate_supplied_inputs_before_defaults_once() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub newtype Wrapped<U,.V:Type=bool,.W:Type=Third()> = U;}comptime fn First()->Type {.char'builtin}comptime fn Second()->Type {bool}comptime fn Third()->Type {.char'builtin}comptime fn Run()->Type {Holder<bool>.Wrapped<.V=Second(),First()>}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Bool)
            ],
        })
    );
    let calls: Vec<_> = vm
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
                .filter(|name| matches!(*name, "First" | "Second" | "Third"))
        })
        .collect();
    assert_eq!(calls, ["Second", "First", "Third"]);
}

#[test]
fn projected_type_defaults_reuse_dynamic_inputs_and_keep_layouts_generic() {
    let source = "typealias Type=.Type'builtin;typealias Preview=Build(.char'builtin);comptime fn Build(value:Type)->Type {Holder<bool>.Wrapped<value>}comptime fn Run()->Type {Build(.char'builtin)}fn other(value:Holder<.char'builtin>.Wrapped<bool>)->.char'builtin {value.as(.char'builtin)}fn run()->bool {other('x'.as(Holder<.char'builtin>.Wrapped<bool>)) == 'x'}struct Holder where T {pub newtype Wrapped<U,.V:Type=U> = T;}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Bool)
            ],
        })
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(matches!(
        package.newtypes[0].representation,
        nir::Term::Rigid { .. }
    ));
}

#[test]
fn projected_dynamic_newtype_inputs_check_trait_bounds_at_invocation() {
    let prefix = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Holder where T {pub newtype Wrapped<U:-Allowed> = U;}comptime fn Build(value:Type)->Type {Holder<bool>.Wrapped<value>}";
    let source = format!(
        "{prefix}comptime fn Run()->Type {{Build(bool)}}comptime fn Invalid()->Type {{Build(.char'builtin)}}"
    );
    let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Bool); 2],
        })
    );
    let invalid = package
        .functions
        .iter()
        .find(|f| f.name == "Invalid")
        .unwrap()
        .declaration;
    let failure = vm.invoke(invalid, vec![]).unwrap_err();
    assert_eq!(
        failure.kind,
        comptime_interpretor::ErrorKind::TypeContractUnresolved
    );
    assert!(luna::invocation::invoke(&package, "Run", 100_000).is_ok());
    assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
    let invalid_static = format!("{prefix}typealias Bad=Holder<bool>.Wrapped<.char'builtin>;");
    let failure = compile(&invalid_static, Emit::Nir)
        .err()
        .expect("known invalid capability must fail before invocation");
    assert!(failure.contains("trait bound"), "{failure}");
}

#[test]
fn projected_dynamic_containers_deliver_inputs_to_members_and_self() {
    for application in [
        "Holder<value>.Wrapped<bool>",
        "Holder<value>.Alias<bool>",
        "Holder<value>.Defaulted",
        "Holder<value>.Plain",
        "Holder<value>.This",
        "Holder<value>.SelfDefault",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;struct Holder where T {{pub newtype Wrapped<U> = U;pub typealias Alias<U> = Wrapped<U>;pub newtype Defaulted<.U:Type=T> = U;pub newtype Plain=T;pub typealias This=Self;pub newtype SelfDefault<.U:Type=Self> = U;}}comptime fn Build(value:Type)->Type {{{application}}}comptime fn Run()->Type {{Build(.char'builtin)}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("NIR")
        };
        let run = package
            .functions
            .iter()
            .find(|f| f.name == "Run")
            .unwrap()
            .declaration;
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let char_type = nir::Term::Primitive(nir::Primitive::Char);
        let expected = if application.ends_with("This") {
            nir::Term::Nominal {
                declaration: package.structs[0].declaration,
                arguments: vec![char_type],
            }
        } else {
            let name = if application.ends_with("SelfDefault") {
                "SelfDefault"
            } else if application.contains("Defaulted") {
                "Defaulted"
            } else if application.ends_with("Plain") {
                "Plain"
            } else {
                "Wrapped"
            };
            let arguments = match name {
                "SelfDefault" => vec![
                    nir::Term::Nominal {
                        declaration: package.structs[0].declaration,
                        arguments: vec![char_type.clone()],
                    },
                    char_type,
                ],
                "Plain" => vec![char_type],
                "Defaulted" => vec![char_type.clone(), char_type],
                _ => vec![nir::Term::Primitive(nir::Primitive::Bool), char_type],
            };
            nir::Term::Nominal {
                declaration: package
                    .newtypes
                    .iter()
                    .find(|ty| ty.name == name)
                    .unwrap()
                    .declaration,
                arguments,
            }
        };
        let result = vm.invoke(run, vec![]).unwrap();
        assert_eq!(
            result,
            comptime_interpretor::Value::Type(expected),
            "{application}"
        );
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            result
        );
    }
}

#[test]
fn dynamic_container_contracts_run_before_member_inputs() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Holder where T:-Allowed {pub newtype Wrapped<U> = U;}comptime fn Parent(flag:bool)->Type {if flag {bool} else {.char'builtin}}comptime fn Member()->Type {bool}comptime fn Build(flag:bool)->Type {Holder<Parent(flag)>.Wrapped<Member()>}comptime fn Run()->Type {Build(true)}comptime fn Invalid()->Type {Build(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (entry, valid) in [("Run", true), ("Invalid", false)] {
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let entry_id = package
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .declaration;
        let result = vm.invoke(entry_id, vec![]);
        if valid {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                comptime_interpretor::ErrorKind::TypeContractUnresolved
            );
        }
        let events: Vec<_> = vm
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
                    .filter(|name| matches!(*name, "Parent" | "Member"))
            })
            .collect();
        assert_eq!(
            events,
            if valid {
                vec!["Parent", "Member"]
            } else {
                vec!["Parent"]
            }
        );
    }
}

#[test]
fn dynamic_type_alias_members_use_only_checked_container_premises() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Checked where T:-Allowed {}struct Holder where T:-Allowed {pub typealias Payload=Checked<T>;}comptime fn Build(value:Type)->Type {Holder<value>.Payload}comptime fn Run()->Type {Build(bool)}comptime fn Invalid()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert!(luna::invocation::invoke(&package, "Run", 100_000).is_ok());
    assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
    let missing = source.replace("struct Holder where T:-Allowed", "struct Holder where T");
    assert!(
        compile(&missing, Emit::Nir).is_err(),
        "member invented a missing container premise"
    );
}

#[test]
fn diverging_container_inputs_do_not_execute_member_arguments() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub newtype Wrapped<U> = U;}comptime fn Member()->Type {.char'builtin}comptime fn Run()->Type {Holder<if true {return bool;} else {bool}>.Wrapped<Member()>}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    let member = package
        .functions
        .iter()
        .find(|f| f.name == "Member")
        .unwrap()
        .declaration;
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert!(!vm.events().iter().any(
        |event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == member)
    ));
}

#[test]
fn member_arguments_use_caller_types_before_selected_container_defaults() {
    let source = "typealias Type=.Type'builtin;trait Character {}impl Character for .char'builtin {}struct Holder where T {pub newtype Wrapped<U:-Character,.V:Type=T> = U;comptime fn Make()->Type where requires T:-Character {Holder<bool>.Wrapped<T>}fn keep(value:T)->Holder<bool>.Wrapped<T> where requires T:-Character {value.as(Holder<bool>.Wrapped<T>)}}comptime fn Run()->Type {Holder<.char'builtin>.Make()}fn run()->bool {Holder<.char'builtin>.keep('x').as(.char'builtin) == 'x'}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let expected = comptime_interpretor::Value::Type(nir::Term::Nominal {
        declaration: package.newtypes[0].declaration,
        arguments: vec![
            nir::Term::Primitive(nir::Primitive::Char),
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::Primitive(nir::Primitive::Bool),
        ],
    });
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        expected
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn dynamic_alias_guards_run_before_bodies_without_creating_type_identity() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}comptime fn Input(flag:bool)->Type {if flag {bool} else {.char'builtin}}comptime fn Body(value:Type)->Type {value}typealias Checked<T:-Allowed,.U:Type=T> = Body(U);comptime fn Build(flag:bool)->Type {Checked<Input(flag)>}comptime fn Run()->Type {Build(true)}comptime fn Invalid()->Type {Build(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert!(package.to_string().contains("(check-type-alias "));
    assert!(package.newtypes.is_empty());
    for (entry, valid) in [("Run", true), ("Invalid", false)] {
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let entry_id = package
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .declaration;
        let result = vm.invoke(entry_id, vec![]);
        if valid {
            assert_eq!(
                result.unwrap(),
                comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
            );
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                comptime_interpretor::ErrorKind::TypeContractUnresolved
            );
        }
        let events: Vec<_> = vm
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
                    .filter(|name| matches!(*name, "Input" | "Body"))
            })
            .collect();
        assert_eq!(
            events,
            if valid {
                vec!["Input", "Body"]
            } else {
                vec!["Input"]
            }
        );
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).is_ok(),
            valid
        );
    }
}

#[test]
fn dynamic_alias_guards_retain_captured_container_parameters() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Holder where T:-Allowed {pub typealias Checked<U:-Allowed> = T;}comptime fn Build(outer:Type,inner:Type)->Type {Holder<outer>.Checked<inner>}comptime fn Run()->Type {Build(bool,bool)}comptime fn Invalid()->Type {Build(bool,.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
    assert_eq!(package.alias_contracts[0].parameters.len(), 2);
}

#[test]
fn dynamic_alias_guards_support_nested_constrained_bodies() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Checked where T:-Allowed {}typealias Inner<T:-Allowed> = Checked<T>;typealias Outer<T:-Allowed> = Inner<T>;comptime fn Build(value:Type)->Type {Outer<value>}comptime fn Run()->Type {Build(bool)}comptime fn Invalid()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let checked = package
        .structs
        .iter()
        .find(|s| s.name == "Checked")
        .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: checked.declaration,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Bool)],
        })
    );
    assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
}

#[test]
fn transparent_container_aliases_preserve_member_arguments_and_defaults() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub newtype Wrapped<U,.V:Type=T> = U;pub typealias Payload=T;}typealias Alias<T> = Holder<T>;typealias Chain<T> = Alias<T>;typealias Flag=Chain<bool>;fn keep(value:Flag.Wrapped<.char'builtin>)->Holder<bool>.Wrapped<.char'builtin> {value}fn payload(value:Flag.Payload)->bool {value}comptime fn Build(value:Type)->Type {Chain<bool>.Wrapped<value>}comptime fn Run()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let expected = nir::Term::Nominal {
        declaration: package.newtypes[0].declaration,
        arguments: vec![
            nir::Term::Primitive(nir::Primitive::Char),
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::Primitive(nir::Primitive::Bool),
        ],
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(expected.clone())
    );
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, expected);
    assert_eq!(**result, expected);
}

#[test]
fn chained_type_members_keep_each_container_environment() {
    let source = "typealias Type=.Type'builtin;struct Inner where T {pub newtype Wrapped<U,.V:Type=T> = U;pub typealias Payload=T;}struct Outer where T {pub typealias Nested<U> = Inner<U>;pub typealias Captured=Inner<T>;}typealias Alias<T> = Outer<T>;fn keep(value:Outer<bool>.Nested<.char'builtin>.Wrapped<bool>)->Inner<.char'builtin>.Wrapped<bool> {value}fn captured(value:Alias<bool>.Captured.Payload)->bool {value}comptime fn Build(value:Type)->Type {Alias<bool>.Nested<.char'builtin>.Wrapped<value>}comptime fn Run()->Type {Build(bool)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let expected = nir::Term::Nominal {
        declaration: package.newtypes[0].declaration,
        arguments: vec![
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::Primitive(nir::Primitive::Char),
            nir::Term::Primitive(nir::Primitive::Char),
        ],
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(expected.clone())
    );
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, expected);
    assert_eq!(**result, expected);
    let captured = package
        .functions
        .iter()
        .find(|f| f.name == "captured")
        .unwrap();
    let nir::Term::Function { parameters, result } = &captured.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Bool));
    assert_eq!(**result, parameters[0].ty);
}

#[test]
fn dynamic_alias_containers_preserve_checks_order_and_single_evaluation() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Holder where T:-Allowed {pub newtype Wrapped<U,.V:Type=T> = U;}typealias Alias<T:-Allowed> = Holder<T>;typealias Chain<T:-Allowed> = Alias<T>;comptime fn Parent(flag:bool)->Type {if flag {bool} else {.char'builtin}}comptime fn Member()->Type {.char'builtin}comptime fn Build(flag:bool)->Type {Chain<Parent(flag)>.Wrapped<Member()>}comptime fn Run()->Type {Build(true)}comptime fn Invalid()->Type {Build(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (entry, valid) in [("Run", true), ("Invalid", false)] {
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let id = package
            .functions
            .iter()
            .find(|f| f.name == entry)
            .unwrap()
            .declaration;
        let result = vm.invoke(id, vec![]);
        if valid {
            assert_eq!(
                result.unwrap(),
                comptime_interpretor::Value::Type(nir::Term::Nominal {
                    declaration: package.newtypes[0].declaration,
                    arguments: vec![
                        nir::Term::Primitive(nir::Primitive::Char),
                        nir::Term::Primitive(nir::Primitive::Bool),
                        nir::Term::Primitive(nir::Primitive::Bool)
                    ]
                })
            );
        } else {
            assert_eq!(
                result.unwrap_err().kind,
                comptime_interpretor::ErrorKind::TypeContractUnresolved
            );
        }
        let calls: Vec<_> = vm
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
                    .filter(|name| matches!(*name, "Parent" | "Member"))
            })
            .collect();
        assert_eq!(
            calls,
            if valid {
                vec!["Parent", "Member"]
            } else {
                vec!["Parent"]
            }
        );
    }
}

#[test]
fn dynamic_chained_members_reuse_outer_inputs_and_self() {
    let source = "typealias Type=.Type'builtin;struct Inner where T {pub typealias Payload=T;pub typealias This=Self;pub newtype Wrapped<U,.V:Type=T> = U;}struct Outer where T {pub typealias Nested<U> = Inner<U>;pub typealias Captured=Inner<T>;}typealias Alias<T> = Outer<T>;comptime fn Build(outer:Type,inner:Type,value:Type)->Type {Alias<outer>.Nested<inner>.Wrapped<value>}comptime fn This(value:Type)->Type {Alias<value>.Captured.This}comptime fn Run()->Type {Build(bool,.char'builtin,bool)}comptime fn SelfType()->Type {This(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Char)
            ]
        })
    );
    let inner = package
        .structs
        .iter()
        .find(|s| s.name == "Inner")
        .unwrap()
        .declaration;
    assert_eq!(
        luna::invocation::invoke(&package, "SelfType", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: inner,
            arguments: vec![nir::Term::Primitive(nir::Primitive::Char)]
        })
    );
}

#[test]
fn alias_container_early_return_skips_member_computation() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub newtype Wrapped<U> = U;}typealias Alias<T> = Holder<T>;comptime fn Member()->Type {.char'builtin}comptime fn Run()->Type {Alias<if true {return bool;} else {bool}>.Wrapped<Member()>}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "Run")
        .unwrap()
        .declaration;
    let member = package
        .functions
        .iter()
        .find(|f| f.name == "Member")
        .unwrap()
        .declaration;
    assert_eq!(
        vm.invoke(run, vec![]).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert!(!vm.events().iter().any(
        |event| matches!(event, comptime_interpretor::Event::EnterFunction(id) if *id == member)
    ));
}

#[test]
fn completed_type_calls_select_members_without_affecting_runtime_fields() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub typealias Payload=T;pub newtype Wrapped<U,.V:Type=T> = U;}comptime fn Select(flag:bool)->Type {if flag {Holder<bool>} else {Holder<.char'builtin>}}fn keep(value:Select(false).Payload)->.char'builtin {value}comptime fn Build(value:Type)->Type {Select(true).Wrapped<value>}comptime fn Run()->Type {Build(.char'builtin)}struct Record {flag:bool}fn record()->Record {Record {flag:true}}fn runtime()->bool {record().flag}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Char));
    assert_eq!(**result, parameters[0].ty);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Primitive(nir::Primitive::Bool)
            ]
        })
    );
    assert_eq!(
        luna::execution::interpret(&package, "runtime", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn computed_type_members_keep_phase_contracts_and_normal_form_opacity() {
    let prefix = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Holder where T {pub typealias Payload=T;}comptime fn Select(value:Type)->Type {Holder<value>}pure comptime fn Head<T>->Type {Holder<T>}";
    for suffix in [
        "fn bad(value:bool)->Select(value).Payload {value}",
        "fn bad(value:Head<bool>.Payload)->bool {value}",
        "comptime fn Checked(value:Type)->Type {Bounded<value>}struct Bounded where T:-Allowed {pub typealias Payload=T;}fn bad(value:Checked(.char'builtin).Payload)->bool {value}",
    ] {
        assert!(
            compile(&format!("{prefix}{suffix}"), Emit::Nir).is_err(),
            "{suffix}"
        );
    }
}

#[test]
fn private_type_members_are_available_in_their_module_and_descendants() {
    for container in [
        "struct Holder where T {private typealias Payload=T;private newtype Wrapped= T;}",
        "enum Holder where T {unit,private typealias Payload=T;private newtype Wrapped=T;}",
    ] {
        let source = format!(
            "mod inside {{pub {container}pub typealias Alias<T> = Holder<T>;pub fn keep(value:Alias<bool>.Payload)->bool {{value}}pub fn wrap(value:bool)->Alias<bool>.Wrapped {{value.as(Alias<bool>.Wrapped)}}pub mod child {{pub fn keep(value:Alias<bool>.Payload)->bool {{value}}}}}}fn run()->bool {{inside.keep(true)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        for member in ["Payload", "Wrapped"] {
            let denied =
                format!("{source}fn denied(value:inside.Alias<bool>.{member})->bool {{true}}");
            assert!(
                compile(&denied, Emit::Nir).is_err(),
                "private {member} escaped its module"
            );
        }
    }
}

#[test]
fn constant_type_factories_keep_callable_phase_and_selected_identity() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub typealias Payload=T;pub newtype Wrapped<U> = U;}comptime fn Select(flag:bool)->Type {if flag {Holder<bool>} else {Holder<.char'builtin>}}const Factory=Select;const Again=Factory;fn keep(value:Again(false).Payload)->.char'builtin {value}comptime fn Build(value:Type)->Type {Factory(true).Wrapped<value>}comptime fn Run()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Char));
    assert_eq!(**result, parameters[0].ty);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Bool)
            ]
        })
    );
    let runtime = format!(
        "{source}struct Record {{flag:bool}}fn record()->Record {{Record {{flag:true}}}}const RuntimeFactory=record;fn runtime()->bool {{RuntimeFactory().flag}}"
    );
    let failure = compile(&runtime, Emit::Nir)
        .err()
        .expect("runtime constant requires Splice");
    assert!(failure.contains("Splice"), "{failure}");
}

#[test]
fn local_type_factory_constants_support_type_annotations_and_members() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub typealias Payload=T;pub newtype Wrapped<U> = U;}comptime fn Select(flag:bool)->Type {if flag {Holder<bool>} else {Holder<.char'builtin>}}fn run()->bool {const Factory=Select;const Again=Factory;let value:Again(true).Payload=true;let wrapped:Again(false).Wrapped<bool> = value.as(Holder<.char'builtin>.Wrapped<bool>);wrapped.as(bool)}struct Record {flag:bool}fn record()->Record {Record {flag:true}}fn runtime()->bool {let factory=record;factory().flag}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for entry in ["run", "runtime"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn static_type_factories_retain_generic_container_inputs() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub typealias Payload=T;pub newtype Wrapped<U> = U;}struct Factory where T {comptime fn Make()->Type {Holder<T>}}fn keep(value:Factory<.char'builtin>.Make().Payload)->.char'builtin {value}comptime fn Run()->Type {Factory<bool>.Make().Wrapped<.char'builtin>}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Char));
    assert_eq!(**result, parameters[0].ty);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Bool)
            ]
        })
    );
}

#[test]
fn static_type_factory_projections_check_container_and_method_bounds() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Holder where T {pub typealias Payload=T;}struct Factory where T:-Allowed {comptime fn Make()->Type where U:-Allowed {Holder<U>}}fn keep(value:Factory<bool>.Make<bool>().Payload)->bool {value}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Bool));
    assert_eq!(**result, parameters[0].ty);
    for rejected in [
        source.replace("Factory<bool>", "Factory<.char'builtin>"),
        source.replace("Make<bool>", "Make<.char'builtin>"),
    ] {
        assert!(compile(&rejected, Emit::Nir).is_err(), "{rejected}");
    }
}

#[test]
fn aliased_static_type_factories_keep_method_and_container_types() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub typealias Payload=T;pub newtype Wrapped<U> = U;}struct Factory where T {comptime fn Make()->Type {Holder<T>}comptime fn Choose()->Type where U {Holder<U>}}typealias Alias<T> = Factory<T>;typealias Chain<T> = Alias<T>;fn keep(value:Chain<.char'builtin>.Make().Payload)->.char'builtin {value}comptime fn Run()->Type {Chain<.char'builtin>.Choose<bool>().Wrapped<.char'builtin>}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Char));
    assert_eq!(**result, parameters[0].ty);
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Nominal {
            declaration: package.newtypes[0].declaration,
            arguments: vec![
                nir::Term::Primitive(nir::Primitive::Char),
                nir::Term::Primitive(nir::Primitive::Bool)
            ]
        })
    );
}

#[test]
fn private_adt_methods_follow_module_privacy_for_static_and_instance_calls() {
    let source = "mod inside {pub struct Value {private fn flag(self)->bool {true}private fn static_flag()->bool {true}}pub typealias Alias=Value;pub fn local()->bool {let value=Value {};value.flag() and Alias.static_flag()}pub mod child {pub fn nested()->bool {let value=Value {};value.flag() and Alias.static_flag()}}}fn run()->bool {inside.local() and inside.child.nested()}";
    for (source, constructor) in [
        (source.to_owned(), "inside.Value {}"),
        (
            source
                .replace("struct Value {", "enum Value {unit,")
                .replace("Value {}", "Value.unit"),
            "inside.Value.unit",
        ),
    ] {
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        for body in [
            "inside.Alias.static_flag()".to_owned(),
            "inside.Value.static_flag()".to_owned(),
            format!("let value={constructor};value.flag()"),
        ] {
            let rejected = format!("{source}fn denied()->bool {{{body}}}");
            let failure = compile(&rejected, Emit::Nir)
                .err()
                .expect("private method escaped");
            assert!(failure.contains("not accessible"), "{failure}");
        }
    }
}

#[test]
fn private_adt_constants_keep_lexical_module_access() {
    for declaration in [
        "struct Holder {private const FLAG=true;}",
        "enum Holder {unit,private const FLAG=true;}",
    ] {
        let source = format!(
            "mod inside {{pub {declaration}pub comptime fn Local()->bool {{Holder.FLAG}}pub mod child {{pub comptime fn Nested()->bool {{Holder.FLAG}}}}}}comptime fn Run()->bool {{inside.Local() and inside.child.Nested()}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let rejected = format!("{source}comptime fn Denied()->bool {{inside.Holder.FLAG}}");
        let failure = compile(&rejected, Emit::Nir)
            .err()
            .expect("private constant escaped");
        assert!(failure.contains("not accessible"), "{failure}");
    }
}

#[test]
fn projected_constants_keep_type_values_and_runtime_phase_boundaries() {
    let source = "typealias Type=.Type'builtin;struct Holder where T {pub const FLAG=true;pub const Payload=bool;}typealias Alias<T> = Holder<T>;comptime fn Run()->bool {Alias<bool>.FLAG and Holder<.char'builtin>.FLAG}fn keep(value:Alias<bool>.Payload)->bool {value}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Bool));
    assert_eq!(**result, parameters[0].ty);
    let runtime = format!("{source}fn bad()->bool {{Alias<bool>.FLAG}}");
    let failure = compile(&runtime, Emit::Nir)
        .err()
        .expect("runtime constant requires Splice");
    assert!(failure.contains("Splice"), "{failure}");
}

#[test]
fn dynamic_constant_projection_checks_even_unused_container_arguments() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Holder where T:-Allowed {pub const FLAG=true;}typealias Alias<T:-Allowed> = Holder<T>;comptime fn Build(value:Type)->bool {Alias<value>.FLAG}comptime fn Run()->bool {Build(bool)}comptime fn Invalid()->bool {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
    assert!(
        compile(
            &source.replace("Alias<value>.FLAG", "Alias<value>.FLAG<bool>"),
            Emit::Nir
        )
        .is_err()
    );
}

#[test]
fn generic_member_type_constants_keep_distinct_container_instances() {
    let source = "struct Holder where T {pub const Payload=T;}typealias Alias<T> = Holder<T>;fn flag(value:Alias<bool>.Payload)->bool {value}fn character(value:Alias<.char'builtin>.Payload)->.char'builtin {value}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (name, primitive) in [
        ("flag", nir::Primitive::Bool),
        ("character", nir::Primitive::Char),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let nir::Term::Function { parameters, result } = &function.signature else {
            panic!("signature")
        };
        assert_eq!(parameters[0].ty, nir::Term::Primitive(primitive));
        assert_eq!(**result, parameters[0].ty);
    }
}

#[test]
fn generic_member_constants_capture_dynamic_inputs_and_check_declarations() {
    let source = "typealias Type=.Type'builtin;comptime fn Select(value:Type)->Type {value}struct Holder where T {pub const Payload=Select(T);}typealias Alias<T> = Holder<T>;comptime fn Build(value:Type)->Type {Alias<value>.Payload}comptime fn Flag()->Type {Build(bool)}comptime fn Character()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (name, primitive) in [
        ("Flag", nir::Primitive::Bool),
        ("Character", nir::Primitive::Char),
    ] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            comptime_interpretor::Value::Type(nir::Term::Primitive(primitive))
        );
    }
    // Declaration checking must still reject an invalid generic initializer,
    // even when there are no uses that would demand its completed value.
    assert!(
        compile(
            "struct Holder where T {pub const INVALID:bool=T;}",
            Emit::Nir
        )
        .is_err()
    );
    assert!(compile("struct Holder where T {pub const LOOP=LOOP;}", Emit::Nir).is_err());
}

#[test]
fn generic_member_constants_preserve_formation_and_definition_contracts() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}struct Selected where U:-Allowed {}struct Holder where T:-Allowed {pub const Payload=Selected<T>;}comptime fn Build(value:Type)->Type {Holder<value>.Payload}comptime fn Valid()->Type {Build(bool)}comptime fn Invalid()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert!(
        matches!(luna::invocation::invoke(&package, "Valid", 100_000).unwrap(), comptime_interpretor::Value::Type(nir::Term::Nominal { arguments, .. }) if arguments == vec![nir::Term::Primitive(nir::Primitive::Bool)])
    );
    assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
    let invalid = source.replace("struct Holder where T:-Allowed", "struct Holder where T");
    assert!(
        compile(&invalid, Emit::Nir).is_err(),
        "caller specialization must not validate an invalid generic declaration"
    );
}

#[test]
fn generic_member_constant_initializers_still_obey_comptime_phase() {
    let failure = compile(
        "fn runtime()->bool {true}struct Holder where T {pub const FLAG=runtime();}",
        Emit::Nir,
    )
    .err()
    .expect("a generic constant cannot call a runtime function");
    assert!(failure.contains("comptime"), "{failure}");
}

#[test]
fn generic_member_constants_forward_dynamic_staged_type_inputs() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}comptime fn Select(comptime value:Type)->Type where requires value:-Allowed {value}struct Holder where T:-Allowed {pub const Payload=Select(T);}comptime fn Build(value:Type)->Type {Holder<value>.Payload}typealias Result=Build(bool);fn keep(value:Result)->bool {value}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let keep = package.functions.iter().find(|f| f.name == "keep").unwrap();
    let nir::Term::Function { parameters, result } = &keep.signature else {
        panic!("signature")
    };
    assert_eq!(parameters[0].ty, nir::Term::Primitive(nir::Primitive::Bool));
    assert_eq!(**result, parameters[0].ty);
}

#[test]
fn captured_member_staged_arguments_keep_instance_values_and_requirements() {
    let source = "typealias Type=.Type'builtin;comptime fn Select(comptime value:Type)->Type {value}struct Holder where T {pub const Payload=Select(T);}typealias Alias<T> = Holder<T>;comptime fn Build(value:Type)->Type {Alias<value>.Payload}typealias Flag=Build(bool);typealias Character=Build(.char'builtin);fn flag(value:Flag)->bool {value}fn character(value:Character)->.char'builtin {value}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (name, primitive) in [
        ("flag", nir::Primitive::Bool),
        ("character", nir::Primitive::Char),
    ] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let nir::Term::Function { parameters, result } = &function.signature else {
            panic!("signature")
        };
        assert_eq!(parameters[0].ty, nir::Term::Primitive(primitive));
        assert_eq!(**result, parameters[0].ty);
    }
    let constrained = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}comptime fn Select(comptime value:Type)->Type where requires value:-Allowed {value}struct Holder where T:-Allowed {pub const Payload=Select(T);}comptime fn Build(value:Type)->Type {Holder<value>.Payload}typealias Bad=Build(.char'builtin);";
    assert!(compile(constrained, Emit::Nir).is_err());
    assert!(
        compile(
            &constrained
                .replace("struct Holder where T:-Allowed", "struct Holder where T")
                .replace("Build(.char'builtin)", "Build(bool)"),
            Emit::Nir
        )
        .is_err(),
        "a concrete caller cannot supply a missing declaration premise"
    );
}

#[test]
fn generic_member_staged_inputs_execute_in_explicit_invocations() {
    let source = "typealias Type=.Type'builtin;trait Allowed {}impl Allowed for bool {}comptime fn Select(comptime value:Type)->Type where requires value:-Allowed {value}struct Holder where T:-Allowed {pub const Payload=Select(T);}comptime fn Build(value:Type)->Type {Holder<value>.Payload}comptime fn Valid()->Type {Build(bool)}comptime fn Invalid()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Valid", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
    assert_eq!(
        luna::invocation::invoke(&package, "Valid", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn dynamic_member_invocations_preserve_stored_callable_contracts() {
    for initializer in ["Select(T)", "do {let action=Select;action(T)}"] {
        let source = format!(
            "typealias Type=.Type'builtin;trait Allowed {{}}impl Allowed for bool {{}}impl Allowed for .char'builtin {{}}comptime fn Select(comptime value:Type)->Type where requires value:-Allowed {{value}}struct Holder where T:-Allowed {{pub const Payload={initializer};}}comptime fn Build(value:Type)->Type {{Holder<value>.Payload}}comptime fn Flag()->Type {{Build(bool)}}comptime fn Character()->Type {{Build(.char'builtin)}}comptime fn Invalid()->Type {{Build(.u8'builtin)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        for (name, primitive) in [
            ("Flag", nir::Primitive::Bool),
            ("Character", nir::Primitive::Char),
            ("Flag", nir::Primitive::Bool),
        ] {
            assert_eq!(
                luna::invocation::invoke(&package, name, 100_000).unwrap(),
                comptime_interpretor::Value::Type(nir::Term::Primitive(primitive))
            );
        }
        assert!(luna::invocation::invoke(&package, "Flag", 1).is_err());
        assert!(luna::invocation::invoke(&package, "Invalid", 100_000).is_err());
        assert_eq!(
            luna::invocation::invoke(&package, "Flag", 100_000).unwrap(),
            comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
        );
    }
}

#[test]
fn generic_member_constants_with_program_preconditions_keep_flow_checks() {
    let source = "typealias Type=.Type'builtin;comptime fn Select(comptime value:Type,allowed:bool)->Type where requires allowed {value}struct Holder where T {pub const Payload=Select(T,true);}comptime fn Build(value:Type)->Type {Holder<value>.Payload}comptime fn Flag()->Type {Build(bool)}comptime fn Character()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (name, primitive) in [
        ("Flag", nir::Primitive::Bool),
        ("Character", nir::Primitive::Char),
    ] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            comptime_interpretor::Value::Type(nir::Term::Primitive(primitive))
        );
    }
    let failure = compile(
        &source.replace("Select(T,true)", "Select(T,false)"),
        Emit::Nir,
    )
    .err()
    .expect("false program precondition must be rejected");
    assert!(failure.contains("precondition"), "{failure}");
}

#[test]
fn type_formation_operands_keep_program_effect_order() {
    for head in ["Holder", "Alias"] {
        let source = format!(
            "typealias Type=.Type'builtin;trait Allowed {{}}impl Allowed for bool {{}}struct Holder where T:-Allowed {{}}typealias Alias<T:-Allowed> = Holder<T>;comptime fn checked(flag:bool)->bool where requires flag {{true}}comptime fn Run()->bool {{var seen=false;let container={head}<do {{seen=true;bool}}>;checked(seen)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(
            compile(
                &source.replace("seen=true;bool", "seen=false;bool"),
                Emit::Nir
            )
            .is_err(),
            "operand effects must not be skipped or assumed"
        );
        let early = source.replace("seen=true;bool", "return true;bool");
        let Artifact::Nir(package) = compile(&early, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn computed_type_operations_preserve_program_operand_effects() {
    for expression in [
        "*(do {seen=true;value})",
        "fn(do {seen=true;value})->bool",
        "fn(bool)->(do {seen=true;value})",
        "value == (do {seen=true;value})",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;comptime fn checked(flag:bool)->bool where requires flag {{true}}comptime fn Build(value:Type)->bool {{var seen=false;let output={expression};checked(seen)}}comptime fn Run()->bool {{Build(bool)}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|failure| panic!("{expression}: {failure}"))
        else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(
            compile(&source.replace("seen=true", "seen=false"), Emit::Nir).is_err(),
            "{expression}"
        );
    }
}

#[test]
fn normal_form_input_plans_preserve_program_operand_effects() {
    let source = "typealias Type=.Type'builtin;pure comptime fn Tagged<T:Type>->Type {T}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build(value:Type)->bool {var seen=false;let output=Tagged<do {seen=true;value}>;checked(seen)}comptime fn Run()->bool {Build(bool)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(compile(&source.replace("seen=true", "seen=false"), Emit::Nir).is_err());
}

#[test]
fn normal_form_defaults_reuse_bound_inputs_without_unfolding_heads() {
    let source = "typealias Type=.Type'builtin;pure comptime fn Tagged<flag:bool,.copy:bool=flag>->Type {bool}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build(value:bool)->Type {var seen=false;let output=Tagged<do {seen=true;value}>;checked(seen);output}comptime fn True()->Type {Build(true)}comptime fn False()->Type {Build(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (name, expected) in [("True", true), ("False", false)] {
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, name, 100_000).unwrap()
        else {
            panic!("opaque normal-form head")
        };
        assert_eq!(
            arguments,
            vec![nir::Term::Bool(expected), nir::Term::Bool(expected)]
        );
    }
    assert!(compile(&source.replace("seen=true", "seen=false"), Emit::Nir).is_err());
}

#[test]
fn normal_form_spread_preparations_keep_effects_and_early_exits() {
    for arguments in [
        "...do {seen=true;(value,)}",
        "...do {seen=true;(value,)},...()",
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;pure comptime fn Tagged<T:Type>->Type {{T}}comptime fn checked(flag:bool)->bool where requires flag {{true}}comptime fn Build(value:Type)->bool {{var seen=false;let output=Tagged<{arguments}>;checked(seen)}}comptime fn Run()->bool {{Build(bool)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(compile(&source.replace("seen=true", "seen=false"), Emit::Nir).is_err());
    }
    let source = "typealias Type=.Type'builtin;pure comptime fn Tagged<T:Type>->Type {T}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Run()->bool {let output=Tagged<do {return true;bool}>;checked(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn computed_empty_normal_form_spreads_preserve_producer_effects() {
    let source = "typealias Type=.Type'builtin;pure comptime fn Tagged<T:Type>->Type {T}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build(value:Type)->bool {var seen=false;let output=Tagged<value,...do {seen=true;()}>;checked(seen)}comptime fn Run()->bool {Build(bool)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(compile(&source.replace("seen=true", "seen=false"), Emit::Nir).is_err());
}

#[test]
fn empty_tuple_context_keeps_tuple_identity_and_runtime_spread_effects() {
    for tuple in [false, true] {
        let prefix = if tuple {
            "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}pure comptime fn Tuple<...Es:List<Type>>->Type {bool}"
        } else {
            ""
        };
        let source = format!(
            "{prefix}fn target()->bool {{true}}fn done() {{()}}fn run()->bool {{var seen=false;let value=target(...do {{seen=true;()}});done();seen and value}}"
        );
        let Artifact::Nir(package) = compile_selected(&source, Emit::Nir, tuple).unwrap() else {
            panic!("NIR")
        };
        let run = package.functions.iter().find(|f| f.name == "run").unwrap();
        let mut empty_values = 0;
        nir::visit::expressions(run, &mut 100_000, |expr, _| {
            if matches!(&expr.kind, nir::program::ExprKind::Tuple(fields) if fields.is_empty()) {
                empty_values += 1;
                if tuple {
                    assert!(matches!(&expr.ty, nir::Term::NormalForm { arguments, .. }
                        if matches!(arguments.as_slice(), [nir::Term::TypeListValue(fields)] if fields.is_empty())));
                } else {
                    assert_eq!(expr.ty, nir::Term::Tuple(vec![]));
                }
            }
            Ok(())
        }).unwrap();
        assert_eq!(empty_values, 1);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(
            compile_selected(&format!("{prefix}fn bad()->bool {{()}}"), Emit::Nir, tuple).is_err()
        );
        assert!(
            compile_selected(
                &source.replace("seen=true;()", "seen=true;"),
                Emit::Nir,
                tuple
            )
            .is_err(),
            "a block without a result must remain void"
        );
    }
}

#[test]
fn type_list_collectors_preserve_program_effects_without_unfolding_tuple() {
    let source = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}pure comptime fn Tuple<...Es:List<Type>>->Type {bool}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build(value:Type)->Type {var seen=false;let output=Tuple<do {seen=true;value},bool>;checked(seen);output}comptime fn Run()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile_selected(source, Emit::Nir, true).unwrap() else {
        panic!("NIR")
    };
    let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
        luna::invocation::invoke(&package, "Run", 100_000).unwrap()
    else {
        panic!("opaque Tuple head")
    };
    assert_eq!(
        arguments,
        vec![nir::Term::TypeListValue(vec![
            nir::Term::Primitive(nir::Primitive::Char),
            nir::Term::Primitive(nir::Primitive::Bool)
        ])]
    );
    assert!(compile_selected(&source.replace("seen=true", "seen=false"), Emit::Nir, true).is_err());
}

#[test]
fn type_list_collectors_join_branch_effects_and_stop_after_return() {
    let prefix = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}pure comptime fn Tuple<...Es:List<Type>>->Type {bool}comptime fn checked(flag:bool)->bool where requires flag {true}";
    let source = format!(
        "{prefix}comptime fn Build(flag:bool,value:Type)->Type {{var seen=false;let output=Tuple<if flag {{do {{seen=true;value}}}} else {{do {{seen=true;bool}}}},.char'builtin>;checked(seen);output}}comptime fn Yes()->Type {{Build(true,bool)}}comptime fn No()->Type {{Build(false,bool)}}"
    );
    let Artifact::Nir(package) = compile_selected(&source, Emit::Nir, true).unwrap() else {
        panic!("NIR")
    };
    for name in ["Yes", "No"] {
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, name, 100_000).unwrap()
        else {
            panic!("opaque Tuple")
        };
        assert_eq!(
            arguments,
            vec![nir::Term::TypeListValue(vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Primitive(nir::Primitive::Char)
            ])]
        );
    }
    assert!(
        compile_selected(
            &source.replace("seen=true;bool", "seen=false;bool"),
            Emit::Nir,
            true
        )
        .is_err()
    );
    let early = format!(
        "{prefix}comptime fn Run(value:Type)->Type {{let output=Tuple<do {{return bool;value}},do {{checked(false);bool}}>;checked(false);output}}comptime fn Entry()->Type {{Run(bool)}}"
    );
    let Artifact::Nir(package) = compile_selected(&early, Emit::Nir, true).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Entry", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn contextual_normal_integer_branches_preserve_flow_and_default_values() {
    let source = "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn Tagged<N:Integer,.M:Integer=N>->Type {bool}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build(flag:bool)->Type {var seen=false;let output=Tagged<if flag {do {seen=true;7}} else {do {seen=true;-3}}>;checked(seen);output}comptime fn Yes()->Type {Build(true)}comptime fn No()->Type {Build(false)}";
    let Artifact::Nir(mut package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    // Exercise the retained contextual-input representation as well as the
    // source path, which can resolve the Integer domain early.
    fn contextual(expr: nir::program::Expr) -> nir::program::NormalFormArgument {
        use nir::program::{ExprKind as K, NormalFormArgument as A};
        let source = expr.source;
        match expr.kind {
            K::If {
                condition,
                then_branch,
                else_branch: Some(other),
            } => A::If {
                source,
                condition: *condition,
                then_input: Box::new(contextual(*then_branch)),
                else_input: Box::new(contextual(*other)),
            },
            K::Block { statements, result } => A::Block {
                source,
                statements,
                result: result.map(|value| Box::new(contextual(*value))),
            },
            K::Literal(nir::program::Literal::Integer(spelling)) => {
                let negative = spelling.starts_with('-');
                A::Integer {
                    source,
                    spelling: spelling.trim_start_matches('-').into(),
                    negative,
                }
            }
            kind => A::Value(nir::program::Expr { kind, ..expr }),
        }
    }
    let build = package
        .functions
        .iter_mut()
        .find(|f| f.name == "Build")
        .unwrap();
    let mut changed = false;
    nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
        if let nir::program::ExprKind::NormalFormType { arguments, .. } = &mut expr.kind {
            let old = std::mem::replace(
                &mut arguments[0],
                nir::program::NormalFormArgument::Unit {
                    source: expr.source,
                },
            );
            let nir::program::NormalFormArgument::Value(value) = old else {
                panic!("source input expression")
            };
            arguments[0] = contextual(value);
            changed = true;
        }
        Ok(())
    })
    .unwrap();
    assert!(changed);
    let build = package
        .functions
        .iter()
        .find(|f| f.name == "Build")
        .unwrap();
    elaboration::verification::flow::analyze_in_package(&package, build, &mut Default::default())
        .unwrap();
    let mut rejected = package.clone();
    let build = rejected
        .functions
        .iter_mut()
        .find(|f| f.name == "Build")
        .unwrap();
    nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
        if matches!(
            expr.kind,
            nir::program::ExprKind::Literal(nir::program::Literal::Bool(true))
        ) {
            expr.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
        }
        Ok(())
    })
    .unwrap();
    let build = rejected
        .functions
        .iter()
        .find(|f| f.name == "Build")
        .unwrap();
    assert!(
        elaboration::verification::flow::analyze_in_package(
            &rejected,
            build,
            &mut Default::default()
        )
        .is_err()
    );
    for (name, value) in [("Yes", 7), ("No", -3)] {
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, name, 100_000).unwrap()
        else {
            panic!("opaque head")
        };
        assert_eq!(
            arguments,
            vec![nir::Term::Integer(comptime_runtime::Integer::from_i128(value)); 2]
        );
    }
    assert!(compile(&source.replace("seen=true;-3", "seen=false;-3"), Emit::Nir).is_err());
}

#[test]
fn dependent_normal_integer_inputs_check_only_selected_branch_effects() {
    let source = "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn Tagged<T:Type,N:T>->Type {T}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build(flag:bool)->Type {var seen=false;let output=Tagged<Integer,if flag {do {seen=true;7}} else {do {seen=true;-3}}>;checked(seen);output}comptime fn Yes()->Type {Build(true)}comptime fn No()->Type {Build(false)}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for (name, value) in [("Yes", 7), ("No", -3)] {
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, name, 100_000).unwrap()
        else {
            panic!("opaque head")
        };
        assert_eq!(
            arguments,
            vec![
                nir::Term::Primitive(nir::Primitive::Integer),
                nir::Term::Integer(comptime_runtime::Integer::from_i128(value))
            ]
        );
    }
    assert!(compile(&source.replace("seen=true;-3", "seen=false;-3"), Emit::Nir).is_err());
}

#[test]
fn normal_tuple_inputs_preserve_element_effects_and_default_fields() {
    let source = "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn List<T>->Type {T}pure comptime fn Tuple<...Es:List<Type>>->Type {bool}pure comptime fn Tagged<P:Tuple<Integer,bool>,.Q:Tuple<Integer,bool> = P>->Type {bool}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build()->Type {var seen=false;let output=Tagged<(do {seen=true;7},true)>;checked(seen);output}comptime fn Run()->Type {Build()}";
    let Artifact::Nir(mut package) = compile_selected(source, Emit::Nir, true).unwrap() else {
        panic!("NIR")
    };
    // Retain the contextual tuple tree explicitly, independently of how early
    // source elaboration discovers this concrete tuple domain.
    let tuple = package
        .functions
        .iter()
        .find(|f| f.name == "Tuple")
        .unwrap()
        .declaration;
    let build = package
        .functions
        .iter_mut()
        .find(|f| f.name == "Build")
        .unwrap();
    let mut changed = false;
    nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
        if let nir::program::ExprKind::NormalFormType { arguments, .. } = &mut expr.kind {
            if let nir::program::NormalFormArgument::Value(value) = &arguments[0] {
                if let nir::program::ExprKind::Tuple(items) = &value.kind {
                    arguments[0] = nir::program::NormalFormArgument::Tuple {
                        source: value.source,
                        type_head: Some(tuple),
                        items: items
                            .iter()
                            .cloned()
                            .map(nir::program::NormalFormArgument::Value)
                            .collect(),
                    };
                    changed = true;
                }
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(changed);
    let build = package
        .functions
        .iter()
        .find(|f| f.name == "Build")
        .unwrap();
    elaboration::verification::flow::analyze_in_package(&package, build, &mut Default::default())
        .unwrap();
    let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
        luna::invocation::invoke(&package, "Run", 100_000).unwrap()
    else {
        panic!("opaque head")
    };
    let expected = nir::Term::TupleValue(vec![
        nir::Term::Integer(comptime_runtime::Integer::from_i128(7)),
        nir::Term::Bool(true),
    ]);
    assert_eq!(arguments, vec![expected.clone(), expected]);
    assert!(compile_selected(&source.replace("seen=true", "seen=false"), Emit::Nir, true).is_err());
}

#[test]
fn explicit_tuple_type_inputs_keep_type_construction_and_early_return() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        pure comptime fn Tagged<T:Type>->Type {T}
        comptime fn checked(flag:bool)->bool where requires flag {true}
        comptime fn Build(value:Type)->Type {
            var seen=false;
            let output=Tagged<Tuple<do {seen=true;value},bool>>;
            checked(seen);output
        }
        comptime fn Run()->Type {Build(.char'builtin)}";
    let Artifact::Nir(package) = compile_selected(source, Emit::Nir, true).unwrap() else {
        panic!("NIR")
    };
    let tuple = package
        .functions
        .iter()
        .find(|f| f.name == "Tuple")
        .unwrap()
        .declaration;
    let tagged = package
        .functions
        .iter()
        .find(|f| f.name == "Tagged")
        .unwrap()
        .declaration;
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::NormalForm {
            head: tagged,
            sort: nir::Sort::Type,
            arguments: vec![nir::Term::NormalForm {
                head: tuple,
                sort: nir::Sort::Type,
                arguments: vec![nir::Term::TypeListValue(vec![
                    nir::Term::Primitive(nir::Primitive::Char),
                    nir::Term::Primitive(nir::Primitive::Bool),
                ])],
            }],
        })
    );
    let early = source
        .replace("seen=true;value", "return bool;value")
        .replace("checked(seen)", "checked(false)");
    let Artifact::Nir(package) = compile_selected(&early, Emit::Nir, true).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn contextual_normal_matches_keep_pattern_bindings_and_guard_failure_effects() {
    let source = "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn Tagged<N:Integer,.M:Integer=N>->Type {bool}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build(flag:bool)->Type {var seen=false;let output=Tagged<flag match {selected if do {seen=true;selected}=>7,_=>-3}>;checked(seen);output}comptime fn Yes()->Type {Build(true)}comptime fn No()->Type {Build(false)}";
    let Artifact::Nir(mut package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let build = package
        .functions
        .iter_mut()
        .find(|f| f.name == "Build")
        .unwrap();
    let mut changed = false;
    nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
        use nir::program::{ExprKind as K, NormalFormArgument as A};
        if let K::NormalFormType { arguments, .. } = &mut expr.kind {
            if let A::Value(value) = &arguments[0] {
                if let K::Match {
                    value: subject,
                    arms,
                } = &value.kind
                {
                    arguments[0] = A::Match {
                        source: value.source,
                        value: *subject.clone(),
                        arms: arms
                            .iter()
                            .map(|arm| {
                                let body = match &arm.body.kind {
                                    K::Literal(nir::program::Literal::Integer(spelling)) => {
                                        A::Integer {
                                            source: arm.body.source,
                                            spelling: spelling.trim_start_matches('-').into(),
                                            negative: spelling.starts_with('-'),
                                        }
                                    }
                                    _ => A::Value(arm.body.clone()),
                                };
                                nir::program::NormalMatchArm {
                                    pattern: arm.pattern.clone(),
                                    guard: arm.guard.clone(),
                                    body,
                                }
                            })
                            .collect(),
                    };
                    changed = true;
                }
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(changed);
    let build = package
        .functions
        .iter()
        .find(|f| f.name == "Build")
        .unwrap();
    elaboration::verification::flow::analyze_in_package(&package, build, &mut Default::default())
        .unwrap();
    for (name, value) in [("Yes", 7), ("No", -3)] {
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, name, 100_000).unwrap()
        else {
            panic!("opaque head")
        };
        assert_eq!(
            arguments,
            vec![nir::Term::Integer(comptime_runtime::Integer::from_i128(value)); 2]
        );
    }
    let build = package
        .functions
        .iter_mut()
        .find(|f| f.name == "Build")
        .unwrap();
    nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
        if matches!(
            expr.kind,
            nir::program::ExprKind::Literal(nir::program::Literal::Bool(true))
        ) {
            expr.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
        }
        Ok(())
    })
    .unwrap();
    let build = package
        .functions
        .iter()
        .find(|f| f.name == "Build")
        .unwrap();
    assert!(
        elaboration::verification::flow::analyze_in_package(
            &package,
            build,
            &mut Default::default()
        )
        .is_err()
    );
}

#[test]
fn negative_comptime_integer_patterns_preserve_flow_refinements() {
    let source = "typealias Integer=.Integer'builtin;comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Run()->bool {let value:Integer=3;value match {3=>checked(true),_=>checked(false)}}";
    let Artifact::Nir(mut package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    let run = package
        .functions
        .iter_mut()
        .find(|f| f.name == "Run")
        .unwrap();
    nir::visit::expression_mut(&mut run.body, &mut 100_000, |expr, _| {
        use nir::program::{ExprKind as K, Literal, Pattern};
        if let K::Literal(Literal::Integer(value)) = &mut expr.kind {
            *value = "-3".into();
        }
        if let K::Match { arms, .. } = &mut expr.kind {
            arms[0].pattern = Pattern::Literal(Literal::Integer("-3".into()));
        }
        Ok(())
    })
    .unwrap();
    let run = package.functions.iter().find(|f| f.name == "Run").unwrap();
    elaboration::verification::flow::analyze_in_package(&package, run, &mut Default::default())
        .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let run = package
        .functions
        .iter_mut()
        .find(|f| f.name == "Run")
        .unwrap();
    nir::visit::expression_mut(&mut run.body, &mut 100_000, |expr, _| {
        if let nir::program::ExprKind::Match { arms, .. } = &mut expr.kind {
            arms[0].pattern =
                nir::program::Pattern::Literal(nir::program::Literal::Integer("3".into()));
        }
        Ok(())
    })
    .unwrap();
    let run = package.functions.iter().find(|f| f.name == "Run").unwrap();
    assert!(
        elaboration::verification::flow::analyze_in_package(&package, run, &mut Default::default())
            .is_err()
    );
}

#[test]
fn signed_numeric_source_patterns_keep_flow_and_matching() {
    let source = "typealias Integer=.Integer'builtin;comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Run()->bool {let value:Integer=-3;value match { -3=>checked(true),_=>checked(false)}}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "Run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert!(
        compile(
            &source.replace("value:Integer=-3", "value:Integer=3"),
            Emit::Nir
        )
        .is_err()
    );
}

#[test]
fn signed_numeric_patterns_cover_ranges_alternatives_and_float_zero() {
    for (ty, pattern, accepted, rejected) in [
        (".i32'builtin", "(-3) or -4", "-4", "3"),
        (".i8'builtin", "-128", "-128", "127"),
        (".i32'builtin", "-3..= -1", "-2", "0"),
        (".i32'builtin", "-0x3", "-3", "3"),
        (".f64'builtin", "-3.5", "-3.5", "3.5"),
        (".f64'builtin", "-0.0", "0.0", "1.0"),
    ] {
        let source = format!(
            "fn pick(value:{ty})->bool {{value match {{ {pattern}=>true,_=>false}}}}fn run()->bool {{pick({accepted}) and not pick({rejected})}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|failure| panic!("{pattern}: {failure}"))
        else {
            panic!("NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
    for pattern in ["-name", "-true", "--3", "-"] {
        let source =
            format!("fn bad(value:bool)->bool {{value match {{ {pattern}=>true,_=>false}}}}");
        assert!(compile(&source, Emit::Nir).is_err(), "{pattern}");
    }
}

#[test]
fn contextual_machine_integer_inputs_preserve_bounds_and_flow() {
    for (ty, text, number) in [(".i8'builtin", "-128", -128), (".u8'builtin", "255", 255)] {
        let source = format!(
            "typealias Type=.Type'builtin;pure comptime fn Tagged<N:{ty},.M:{ty}=N>->Type {{bool}}comptime fn checked(flag:bool)->bool where requires flag {{true}}comptime fn Build()->Type {{var seen=false;let output=Tagged<do {{seen=true;{text}}}>;checked(seen);output}}comptime fn Run()->Type {{Build()}}"
        );
        let Artifact::Nir(mut package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        let build = package
            .functions
            .iter_mut()
            .find(|f| f.name == "Build")
            .unwrap();
        fn contextual(expr: nir::program::Expr) -> nir::program::NormalFormArgument {
            use nir::program::{ExprKind as K, Literal, NormalFormArgument as A};
            let source = expr.source;
            match expr.kind {
                K::Block { statements, result } => A::Block {
                    source,
                    statements,
                    result: result.map(|value| Box::new(contextual(*value))),
                },
                K::Literal(Literal::SignedInteger(value)) => A::Integer {
                    source,
                    spelling: value.unsigned_abs().to_string(),
                    negative: value < 0,
                },
                K::Literal(Literal::UnsignedInteger(value)) => A::Integer {
                    source,
                    spelling: value.to_string(),
                    negative: false,
                },
                kind => A::Value(nir::program::Expr { kind, ..expr }),
            }
        }
        let mut changed = false;
        nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
            if let nir::program::ExprKind::NormalFormType { arguments, .. } = &mut expr.kind {
                if let nir::program::NormalFormArgument::Value(value) = &arguments[0] {
                    arguments[0] = contextual(value.clone());
                    changed = true;
                }
            }
            Ok(())
        })
        .unwrap();
        assert!(changed);
        let build = package
            .functions
            .iter()
            .find(|f| f.name == "Build")
            .unwrap();
        elaboration::verification::flow::analyze_in_package(
            &package,
            build,
            &mut Default::default(),
        )
        .unwrap();
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, "Run", 100_000).unwrap()
        else {
            panic!("opaque head")
        };
        assert_eq!(
            arguments,
            vec![nir::Term::Integer(comptime_runtime::Integer::from_i128(number)); 2]
        );
        let build = package
            .functions
            .iter_mut()
            .find(|f| f.name == "Build")
            .unwrap();
        nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
            use nir::program::{ExprKind as K, NormalFormArgument as A};
            if let K::NormalFormType { arguments, .. } = &mut expr.kind {
                let A::Block {
                    result: Some(result),
                    ..
                } = &mut arguments[0]
                else {
                    panic!("contextual block")
                };
                let A::Integer { spelling, .. } = result.as_mut() else {
                    panic!("contextual integer")
                };
                *spelling = if number < 0 { "129" } else { "256" }.into();
            }
            Ok(())
        })
        .unwrap();
        let build = package
            .functions
            .iter()
            .find(|f| f.name == "Build")
            .unwrap();
        let failure = elaboration::verification::flow::analyze_in_package(
            &package,
            build,
            &mut Default::default(),
        )
        .err()
        .expect("out-of-range contextual integer");
        assert!(format!("{failure:?}").contains("OutOfRange"), "{failure:?}");
        assert!(luna::invocation::invoke(&package, "Run", 100_000).is_err());
    }
}

#[test]
fn contextual_bitvec_integer_inputs_check_width_without_truncation() {
    for (width, maximum, overflow, words) in [
        (0, "0", "1", &[][..]),
        (1, "1", "2", &[1][..]),
        (8, "255", "256", &[255][..]),
        (32, "4294967295", "4294967296", &[4294967295][..]),
        (
            65,
            "36893488147419103231",
            "36893488147419103232",
            &[4294967295, 4294967295, 1][..],
        ),
        (
            129,
            "680564733841876926926749214863536422911",
            "680564733841876926926749214863536422912",
            &[4294967295, 4294967295, 4294967295, 4294967295, 1][..],
        ),
    ] {
        let source = "typealias Type=.Type'builtin;typealias Bits=.BitVec'builtin<8>;pure comptime fn Tagged<N:Bits,.M:Bits=N>->Type {bool}comptime fn checked(flag:bool)->bool where requires flag {true}comptime fn Build()->Type {var seen=false;let output=Tagged<do {seen=true;255}>;checked(seen);output}comptime fn Run()->Type {Build()}";
        let source = source
            .replace("<8>", &format!("<{width}>"))
            .replace("255", maximum);
        let Artifact::Nir(mut package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        let build = package
            .functions
            .iter_mut()
            .find(|f| f.name == "Build")
            .unwrap();
        nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
            use nir::program::{ExprKind as K, NormalFormArgument as A};
            if let K::NormalFormType { arguments, .. } = &mut expr.kind {
                let A::Value(value) = &arguments[0] else {
                    panic!("input expression")
                };
                let K::Block {
                    statements,
                    result: Some(tail),
                } = &value.kind
                else {
                    panic!("input block")
                };
                arguments[0] = A::Block {
                    source: value.source,
                    statements: statements.clone(),
                    result: Some(Box::new(A::Integer {
                        source: tail.source,
                        spelling: maximum.into(),
                        negative: false,
                    })),
                };
            }
            Ok(())
        })
        .unwrap();
        let build = package
            .functions
            .iter()
            .find(|f| f.name == "Build")
            .unwrap();
        elaboration::verification::flow::analyze_in_package(
            &package,
            build,
            &mut Default::default(),
        )
        .unwrap();
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, "Run", 100_000).unwrap()
        else {
            panic!("opaque head")
        };
        assert_eq!(arguments.len(), 2);
        for value in arguments {
            let nir::Term::BitVec(bits) = value else {
                panic!("bit value")
            };
            assert_eq!(bits.width(), width);
            assert_eq!(bits.words(), words);
        }
        for (spelling, negative) in [(overflow, false), ("1", true)] {
            let mut rejected = package.clone();
            let build = rejected
                .functions
                .iter_mut()
                .find(|f| f.name == "Build")
                .unwrap();
            nir::visit::expression_mut(&mut build.body, &mut 100_000, |expr, _| {
                use nir::program::{ExprKind as K, NormalFormArgument as A};
                if let K::NormalFormType { arguments, .. } = &mut expr.kind {
                    let A::Block {
                        result: Some(result),
                        ..
                    } = &mut arguments[0]
                    else {
                        panic!("input block")
                    };
                    *result = Box::new(A::Integer {
                        source: expr.source,
                        spelling: spelling.into(),
                        negative,
                    });
                }
                Ok(())
            })
            .unwrap();
            let build = rejected
                .functions
                .iter()
                .find(|f| f.name == "Build")
                .unwrap();
            let failure = elaboration::verification::flow::analyze_in_package(
                &rejected,
                build,
                &mut Default::default(),
            )
            .err()
            .expect("invalid bit value");
            assert!(format!("{failure:?}").contains("OutOfRange"), "{failure:?}");
            assert!(luna::invocation::invoke(&rejected, "Run", 100_000).is_err());
        }
    }
}

#[test]
fn delayed_normal_head_capabilities_are_retained_and_checked_at_formation() {
    for default in ["", ",.U:Type=T"] {
        let source = format!(
            "typealias Type=.Type'builtin;trait Allowed {{}} impl Allowed for bool {{}}
            pure comptime fn Guarded<T :- Allowed{default}>->Type {{T}}
            comptime fn Build(T:Type)->Type {{Guarded<T>}}
            comptime fn Good()->Type {{Build(bool)}}
            comptime fn Bad()->Type {{Build(.char'builtin)}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        let head = package
            .normal_forms
            .iter()
            .find(|head| head.parameters.len() == if default.is_empty() { 1 } else { 2 })
            .unwrap();
        assert!(
            head.requirements
                .iter()
                .any(|requirement| matches!(requirement, nir::program::TypePredicate::Trait(_)))
        );
        let value = luna::invocation::invoke(&package, "Good", 100_000).unwrap();
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { head: actual, .. }) = value
        else {
            panic!("opaque head")
        };
        assert_eq!(actual, head.declaration);
        assert!(luna::invocation::invoke(&package, "Bad", 100_000).is_err());
        assert!(luna::invocation::invoke(&package, "Good", 100_000).is_ok());
        let invalid_static = format!("{source}typealias Rejected=Guarded<.char'builtin>;");
        let failure = compile(&invalid_static, Emit::Nir)
            .err()
            .expect("known invalid capability must fail during elaboration");
        assert!(failure.contains("trait bound"), "{failure}");
        let mut without_body = package.clone();
        without_body
            .functions
            .retain(|function| function.declaration != head.declaration);
        nir::normal_forms::validate(&without_body).unwrap();
        assert!(luna::invocation::invoke(&without_body, "Good", 100_000).is_ok());
        assert!(luna::invocation::invoke(&without_body, "Bad", 100_000).is_err());
        let mut corrupted = package.clone();
        corrupted
            .normal_forms
            .iter_mut()
            .find(|h| h.declaration == head.declaration)
            .unwrap()
            .requirements
            .clear();
        assert!(
            nir::normal_forms::validate(&corrupted).is_err(),
            "dropping the capability must invalidate head metadata"
        );
    }
}

#[test]
fn normal_head_capabilities_substitute_value_inputs_and_defaults() {
    for (inputs, requires, known, supplied, invalid, value_first) in [
        (
            "T,Mode:Symbol",
            "where requires T :- Allowed<Mode>",
            "bool,.yes",
            "T,mode",
            "bool,.no",
            false,
        ),
        (
            "T,.Mode:Symbol=.yes",
            "where requires T :- Allowed<Mode>",
            "bool",
            "T,.Mode=mode",
            "bool,.Mode=.no",
            false,
        ),
        (
            "T :- Allowed<Mode>,Mode:Symbol",
            "",
            "bool,.yes",
            "T,mode",
            "bool,.no",
            false,
        ),
        (
            "Mode:Symbol,T :- Allowed<Mode>",
            "",
            ".yes,bool",
            "mode,T",
            ".no,bool",
            true,
        ),
        (
            ".Mode:Symbol=.yes,T :- Allowed<Mode>",
            "",
            "bool",
            ".Mode=mode,T",
            ".Mode=.no,bool",
            true,
        ),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;typealias Symbol=.Symbol'builtin;
            trait Allowed where mode:Symbol {{}}
            impl Allowed<.yes> for bool {{}}
            pure comptime fn Guarded<{inputs}>->Type {requires} {{T}}
            typealias Known=Guarded<{known}>;
            comptime fn Build(T:Type,mode:Symbol)->Type {{Guarded<{supplied}>}}
            comptime fn Good()->Type {{Build(bool,.yes)}}
            comptime fn Bad()->Type {{Build(bool,.no)}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|e| panic!("{inputs}: {e}"))
        else {
            panic!("NIR")
        };
        let comptime_interpretor::Value::Type(nir::Term::NormalForm { arguments, .. }) =
            luna::invocation::invoke(&package, "Good", 100_000).unwrap()
        else {
            panic!("normal form")
        };
        let mut expected = vec![
            nir::Term::Primitive(nir::Primitive::Bool),
            nir::Term::Symbol("yes".into()),
        ];
        if value_first {
            expected.reverse();
        }
        assert_eq!(arguments, expected);
        let failure = luna::invocation::invoke(&package, "Bad", 100_000).unwrap_err();
        assert!(failure.contains("TypeContract"), "{failure}");
        assert!(luna::invocation::invoke(&package, "Good", 100_000).is_ok());
        let rejected = format!("{source}typealias Rejected=Guarded<{invalid}>;");
        let failure = compile(&rejected, Emit::Nir)
            .err()
            .expect("known missing index capability");
        assert!(failure.contains("trait bound"), "{failure}");
    }
}

#[test]
fn scalar_trait_indices_preserve_domains_and_select_exact_instances() {
    for (domain, accepted, rejected) in [
        ("bool", "true", "false"),
        ("Integer", "3", "4"),
        ("Str", "\"yes\"", "\"no\""),
        ("char", "'a'", "'b'"),
    ] {
        for optional in [false, true] {
            let parameter = if optional {
                format!(".mode:{domain}={accepted}")
            } else {
                format!("mode:{domain}")
            };
            let argument = |value: &str| {
                if optional {
                    format!(".mode={value}")
                } else {
                    value.to_owned()
                }
            };
            let yes = argument(accepted);
            let source = format!(
                "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;
                typealias Str=.str'builtin;typealias char=.char'builtin;
                trait Allowed where {parameter} {{}}
                impl Allowed<{yes}> for bool {{}}
                pure comptime fn Guarded<T,mode:{domain}>->Type
                    where requires T :- Allowed<{}> {{T}}
                comptime fn Build(T:Type,mode:{domain})->Type {{Guarded<T,mode>}}
                comptime fn Good()->Type {{Build(bool,{accepted})}}
                comptime fn Bad()->Type {{Build(bool,{rejected})}}",
                argument("mode")
            );
            let Artifact::Nir(package) = compile(&source, Emit::Nir)
                .unwrap_or_else(|e| panic!("{domain}, optional={optional}: {e}"))
            else {
                panic!("NIR")
            };
            luna::invocation::invoke(&package, "Good", 100_000)
                .unwrap_or_else(|e| panic!("{domain}, optional={optional}: {e}"));
            let failure = luna::invocation::invoke(&package, "Bad", 100_000).unwrap_err();
            assert!(failure.contains("TypeContract"), "{domain}: {failure}");
            luna::invocation::invoke(&package, "Good", 100_000)
                .unwrap_or_else(|e| panic!("{domain}, optional={optional}: {e}"));
            let dispatch = format!(
                "typealias Integer=.Integer'builtin;typealias Str=.str'builtin;typealias char=.char'builtin;
                trait Choose where {parameter} {{comptime fn choose(self)->bool;}}
                impl Choose<{yes}> for bool {{comptime fn choose(self)->bool {{true}}}}
                impl Choose<{}> for bool {{comptime fn choose(self)->bool {{false}}}}
                comptime fn Good()->bool {{true.as(Choose<{yes}>).choose()}}
                comptime fn Bad()->bool {{true.as(Choose<{}>).choose()}}",
                argument(rejected), argument(rejected)
            );
            let Artifact::Nir(dispatched) =
                compile(&dispatch, Emit::Nir).unwrap_or_else(|e| panic!("{domain}: dispatch: {e}"))
            else {
                panic!("NIR")
            };
            assert_eq!(
                luna::invocation::invoke(&dispatched, "Good", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            assert_eq!(
                luna::invocation::invoke(&dispatched, "Bad", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(false)
            );
            let duplicate = format!(
                "{dispatch}impl Choose<{yes}> for bool {{comptime fn choose(self)->bool {{false}}}}"
            );
            assert!(
                compile(&duplicate, Emit::Nir).is_err(),
                "{domain}: overlapping implementation"
            );
            let missing = format!("{source}typealias Missing=Guarded<bool,{rejected}>;");
            assert!(
                compile(&missing, Emit::Nir).is_err(),
                "{domain}: missing instance"
            );
            let wrong = format!("{source}impl Allowed<{}> for char {{}}", argument(".wrong"));
            assert!(
                compile(&wrong, Emit::Nir).is_err(),
                "{domain}: wrong domain"
            );
            if optional {
                let default = format!(
                    "{source}pure comptime fn Default<T :- Allowed>->Type {{T}} typealias Selected=Default<bool>;"
                );
                compile(&default, Emit::Nir).unwrap_or_else(|e| panic!("{domain}: default: {e}"));
            }
        }
    }
}

#[test]
fn inherited_scalar_trait_indices_preserve_open_declaration_domains() {
    for (domain, yes, no) in [("bool", "true", "false"), ("Integer", "3", "4")] {
        let source = format!(
            "typealias Integer=.Integer'builtin;
            trait Root where mode:{domain} {{comptime fn selected(self)->bool;}}
            trait Middle :- Root<mode> where mode:{domain} {{}}
            trait Leaf :- Middle<mode> where mode:{domain} {{
                comptime fn inherited(self)->bool {{self.as(Root<mode>).selected()}}
            }}
            impl Root<{yes}> for bool {{comptime fn selected(self)->bool {{true}}}}
            impl Root<{no}> for bool {{comptime fn selected(self)->bool {{false}}}}
            impl Middle<{yes}> for bool {{}}
            impl Middle<{no}> for bool {{}}
            impl Leaf<{yes}> for bool {{}}
            impl Leaf<{no}> for bool {{}}
            comptime fn Good()->bool {{true.as(Leaf<{yes}>).inherited()}}
            comptime fn Bad()->bool {{true.as(Leaf<{no}>).inherited()}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|e| panic!("{domain}: {e}"))
        else {
            panic!("NIR")
        };
        for (entry, expected) in [("Good", true), ("Bad", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000)
                    .unwrap_or_else(|e| panic!("{domain} {entry}: {e}")),
                comptime_interpretor::Value::Bool(expected)
            );
        }
    }
}

#[test]
fn dependent_trait_index_domains_substitute_inherited_type_parameters() {
    for (parameter, argument) in [("mode:T", "T,mode"), ("mode:T,.copy:T=mode", "T,mode")] {
        let source = "typealias Type=.Type'builtin;
        trait Root where T,PARAMETER {comptime fn selected(self)->bool;}
        trait Leaf :- Root<ARGUMENT> where T,mode:T {
            comptime fn inherited(self)->bool {self.as(Root<ARGUMENT>).selected()}
        }
        impl Root<bool,true> for bool {comptime fn selected(self)->bool {true}}
        impl Leaf<bool,true> for bool {}
        comptime fn Good()->bool {true.as(Leaf<bool,true>).inherited()}";
        let source = source
            .replace("PARAMETER", parameter)
            .replace("ARGUMENT", argument);
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Good", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        for invalid in ["Root<bool,'x'>", "Leaf<bool,'x'>"] {
            let bad =
                format!("{source}comptime fn Wrong()->bool {{true.as({invalid}).selected()}}");
            let failure = compile(&bad, Emit::Nir)
                .err()
                .expect("wrong dependent input domain");
            assert!(failure.contains("type mismatch"), "{failure}");
        }
    }
}

#[test]
fn trait_value_inputs_check_late_domains() {
    for default in ["bool", "DefaultType()"] {
        for inputs in [
            "true",
            "true,.T=bool",
            ".T=bool,true",
            "value(),.T=ChosenType()",
        ] {
            let source = format!(
                "typealias Type=.Type'builtin;
                comptime fn DefaultType()->Type {{bool}}
                comptime fn ChosenType()->Type {{bool}}
                comptime fn value()->bool {{true}}
                trait Select where .T:Type={default},value:T {{comptime fn selected(self)->bool;}}
                impl Select<true> for bool {{comptime fn selected(self)->bool {{true}}}}
                comptime fn Good()->bool {{true.as(Select<{inputs}>).selected()}}"
            );
            let Artifact::Nir(package) =
                compile(&source, Emit::Nir).unwrap_or_else(|e| panic!("{default}, {inputs}: {e}"))
            else {
                panic!("NIR")
            };
            assert_eq!(
                luna::invocation::invoke(&package, "Good", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            for invalid in ["true,.T=.char'builtin", "'x'", "value(),.T=.char'builtin"] {
                let bad = format!(
                    "{source}comptime fn Wrong()->bool {{true.as(Select<{invalid}>).selected()}}"
                );
                let failure = compile(&bad, Emit::Nir)
                    .err()
                    .expect("late domain must be checked");
                assert!(failure.contains("type mismatch"), "{failure}");
            }
        }
    }
}

#[test]
fn fixed_width_integer_trait_indices_use_declared_ranges() {
    for (ty, yes, no, overflow) in [
        ("u8", "255", "0", "256"),
        ("BitVec<8>", "255", "0", "256"),
        (
            "BitVec<65>",
            "36893488147419103231",
            "0",
            "36893488147419103232",
        ),
        ("i8", "-128", "127", "128"),
        (
            "u128",
            "340282366920938463463374607431768211455",
            "0",
            "340282366920938463463374607431768211456",
        ),
        (
            "i128",
            "-170141183460469231731687303715884105728",
            "0",
            "170141183460469231731687303715884105728",
        ),
    ] {
        let domain =
            if let Some(width) = ty.strip_prefix("BitVec<").and_then(|s| s.strip_suffix('>')) {
                format!(".BitVec'builtin<{width}>")
            } else {
                format!(".{ty}'builtin")
            };
        let source = format!(
            "typealias Index={domain};
            trait Select where index:Index {{comptime fn selected(self)->bool;}}
            impl Select<{yes}> for bool {{comptime fn selected(self)->bool {{true}}}}
            impl Select<{no}> for bool {{comptime fn selected(self)->bool {{false}}}}
            comptime fn Good()->bool {{true.as(Select<{yes}>).selected()}}
            comptime fn Other()->bool {{true.as(Select<{no}>).selected()}}"
        );
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|e| panic!("{ty}: {e}"))
        else {
            panic!("NIR")
        };
        for (entry, expected) in [("Good", true), ("Other", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                comptime_interpretor::Value::Bool(expected)
            );
        }
        let bad = format!(
            "{source}impl Select<{overflow}> for .char'builtin {{comptime fn selected(self)->bool {{false}}}}"
        );
        assert!(compile(&bad, Emit::Nir).is_err(), "{ty}: overflow");
    }
}

#[test]
fn trait_value_domains_can_depend_on_self() {
    for parameters in ["value:Self", "value:Self,.copy:Self=value"] {
        let source = format!(
            "trait Indexed where {parameters} {{comptime fn selected(self)->bool;}}
            trait Leaf :- Indexed<value> where value:Self {{
                comptime fn inherited(self)->bool {{self.as(Indexed<value>).selected()}}
            }}
            impl Indexed<true> for bool {{comptime fn selected(self)->bool {{true}}}}
            impl Indexed<'x'> for .char'builtin {{comptime fn selected(self)->bool {{false}}}}
            impl Leaf<true> for bool {{}}
            impl Leaf<'x'> for .char'builtin {{}}
            comptime fn Good()->bool {{true.as(Leaf<true>).inherited()}}
            comptime fn Other()->bool {{'x'.as(Leaf<'x'>).inherited()}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        for (entry, expected) in [("Good", true), ("Other", false)] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                comptime_interpretor::Value::Bool(expected)
            );
        }
        for invalid in [
            "true.as(Indexed<'x'>).selected()",
            "'x'.as(Leaf<true>).inherited()",
        ] {
            let bad = format!("{source}comptime fn Wrong()->bool {{{invalid}}}");
            let failure = compile(&bad, Emit::Nir)
                .err()
                .expect("Self domain mismatch");
            assert!(failure.contains("type mismatch"), "{failure}");
        }
    }
}

#[test]
fn exact_float_trait_indices_support_closed_absence_queries() {
    let source = "typealias Float=.Float'builtin;
        trait Selected where value:Float {}
        impl Selected<1.25> for bool {}
        comptime fn present()->bool where requires bool :- Selected<1.25> {true}
        comptime fn absent()->bool where requires not (bool :- Selected<2.5>) {true}";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    for entry in ["present", "absent"] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn late_trait_domains_contextually_type_intrinsic_numeric_literals() {
    for (domain, literal, overflow) in [
        (".u8'builtin", "255", "256"),
        (".i8'builtin", "-128", "128"),
        (".BitVec'builtin<8>", "255", "256"),
        (".f32'builtin", "1.25", "1.0e999"),
    ] {
        for application in [
            format!("{literal}"),
            format!("{literal},.T=Index"),
            format!("{literal},.T=ChosenType()"),
        ] {
            let source = format!("typealias Type=.Type'builtin;typealias Index={domain};
                comptime fn ChosenType()->Type {{Index}}
                trait Selected where .T:Type=ChosenType(),value:T,.copy:T=value {{comptime fn selected(self)->bool;}}
                impl Selected<{literal}> for bool {{comptime fn selected(self)->bool {{true}}}}
                comptime fn Good()->bool {{true.as(Selected<{application}>).selected()}}");
            let Artifact::Nir(package) = compile(&source, Emit::Nir)
                .unwrap_or_else(|e| panic!("{domain}, {application}: {e}"))
            else {
                panic!("NIR")
            };
            assert_eq!(
                luna::invocation::invoke(&package, "Good", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            let invalid = format!(
                "{source}impl Selected<{overflow}> for .char'builtin {{comptime fn selected(self)->bool {{false}}}}"
            );
            assert!(compile(&invalid, Emit::Nir).is_err(), "{domain}: overflow");
        }
    }
    let duplicate = "typealias Type=.Type'builtin;trait Selected where .T:Type=.u8'builtin,first:T,.value:T=first {}impl Selected<.value=1,.value=2,3> for bool {}";
    assert!(
        compile(duplicate, Emit::Nir)
            .err()
            .unwrap()
            .contains("duplicate trait argument")
    );
}

#[test]
fn mixed_trait_arguments_expand_literal_and_computed_packets() {
    for bound_tuple in [false, true] {
        let tuple_declarations = "pure comptime fn List<T>->Type {T}pure comptime fn Tuple<...Es:List<Type>>->Type {bool}";
        for arguments in [
            "...(bool,true)",
            "...packet()",
            "...(),bool,...(true,)",
            "...empty(),...packet(),...empty()",
        ] {
            let source = format!(
                "typealias Type=.Type'builtin;
            {tuple_declarations}
            comptime fn empty(...items:T)->T where T {{items}}
            comptime fn wrong()->Tuple<Type,.char'builtin> {{(bool,'x')}}
            trait Selected where T,value:T,.copy:T=value {{comptime fn selected(self)->bool;}}
            comptime fn packet()->Tuple<Type,bool> {{(bool,true)}}
            impl Selected<bool,true> for bool {{comptime fn selected(self)->bool {{true}}}}
            comptime fn Good()->bool {{true.as(Selected<{arguments}>).selected()}}"
            );
            if !bound_tuple {
                let error = compile_selected(&source, Emit::Nir, false)
                    .err()
                    .expect("unbound constructors must not supply packet layout");
                assert!(
                    error.contains("variadic normal-form input requires a bound meta.List<T>"),
                    "{error}"
                );
                continue;
            }
            let Artifact::Nir(package) = compile_selected(&source, Emit::Nir, bound_tuple)
                .unwrap_or_else(|e| panic!("{arguments}: {e}"))
            else {
                panic!("NIR")
            };
            assert_eq!(
                luna::invocation::invoke(&package, "Good", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            for invalid in [
                "...(bool,'x')",
                "...true",
                "...(bool,true,false)",
                "...(bool,)",
                "...wrong()",
            ] {
                let bad = format!(
                    "{source}comptime fn Wrong()->bool {{true.as(Selected<{invalid}>).selected()}}"
                );
                assert!(
                    compile_selected(&bad, Emit::Nir, bound_tuple).is_err(),
                    "{invalid}"
                );
            }
        }
    }
}

#[test]
fn mixed_trait_packet_contexts_flow_into_blocks_branches_and_generic_calls() {
    for packet in ["do {(255,)}", "if true {(255,)} else {(0,)}", "packet(255)"] {
        let source = format!("pure comptime fn List<T>->.Type'builtin {{T}} pure comptime fn Tuple<...Es:List<.Type'builtin>>->.Type'builtin {{bool}} typealias Byte=.u8'builtin;
            trait Selected where T,value:T,flag:bool,.copy:T=value {{comptime fn selected(self)->bool;}}
            comptime fn packet(value:T)->Tuple<T> where T {{(value,)}}
            impl Selected<Byte,255,true> for bool {{comptime fn selected(self)->bool {{true}}}}
            comptime fn Good()->bool {{true.as(Selected<Byte,...{packet},true>).selected()}}");
        let Artifact::Nir(package) =
            compile_selected(&source, Emit::Nir, true).unwrap_or_else(|e| panic!("{packet}: {e}"))
        else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Good", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        let invalid = source.replace(
            &format!("Selected<Byte,...{packet},true>"),
            &format!("Selected<Byte,...{},true>", packet.replace("255", "256")),
        );
        let error = compile_selected(&invalid, Emit::Nir, true)
            .err()
            .expect("overflow");
        assert!(
            error.contains("range") || error.contains("fit"),
            "{packet}: {error}"
        );
    }
}

#[test]
fn type_preserving_packet_calls_disambiguate_multiple_trait_spreads() {
    for first in [
        "identity((255,))",
        "identity(identity((255,)))",
        "pack(255)",
        "pack(.items=(255,))",
    ] {
        let source = format!("typealias Byte=.u8'builtin;
            comptime fn identity(value:T)->T where T {{value}}
            comptime fn pack(...items:T)->T where T {{items}}
            trait Selected where left:Byte,right:Byte {{comptime fn selected(self)->bool;}}
            impl Selected<255,7> for bool {{comptime fn selected(self)->bool {{true}}}}
            comptime fn Good()->bool {{true.as(Selected<...{first},...identity((7,))>).selected()}}");
        let Artifact::Nir(package) =
            compile(&source, Emit::Nir).unwrap_or_else(|e| panic!("{first}: {e}"))
        else {
            panic!("NIR")
        };
        assert_eq!(
            luna::invocation::invoke(&package, "Good", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn guard_temporary_rewrapping_does_not_grant_copy_to_external_values() {
    for (implementation, accepted) in [("impl Duplicate for Flag {}", true), ("", false)] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            newtype Flag=bool;newtype Outer=Flag;{implementation}
            fn run()->bool {{let flag=false.as(Flag);true match {{
                _ if flag.as(Outer).as(Flag).as(bool)=>false,
                _=>not flag.as(bool),
            }}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("wrong stage")
        };
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            100_000,
        );
        if accepted {
            assert_eq!(result.unwrap(), comptime_interpretor::Value::Bool(true));
        } else {
            assert!(result.unwrap_err().contains("failure-path"));
        }
    }
}

#[test]
fn associated_newtype_constants_keep_generic_representation_and_error_payloads() {
    for generic in [false, true] {
        let (definition, subject) = if generic {
            (
                "struct Holder where T {}impl Source for Holder<T> where T {assoc flag:Flag=true.as(Flag);assoc result:!bool Flag=true.as(Flag);}",
                "Holder<bool>",
            )
        } else {
            (
                "impl Source for bool {assoc flag:Flag=true.as(Flag);assoc result:!bool Flag=true.as(Flag);}",
                "bool",
            )
        };
        let source = format!(
            "newtype Flag=bool;trait Source {{assoc flag:Flag;assoc result:!bool Flag;}}{definition}fn run()->bool {{let first={subject}.as(Source).flag;let second={subject}.as(Source).result match {{value! => value.as(bool),error failure=>failure}};first.as(bool) and second}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn interpreter_admission_rechecks_completed_newtype_dispatch_values() {
    let source = "newtype Flag=bool;trait Source {assoc flag:Flag;fn read()->bool;}impl Source for bool {assoc flag:Flag=true.as(Flag);fn read()->bool {Self.as(Source).flag.as(bool)}}fn run()->bool {bool.as(Source).read()}";
    let Artifact::Nir(mut package) = compile(source, Emit::Nir).unwrap() else {
        panic!("NIR")
    };
    // Supply the completed form explicitly: source acquisition may retain an
    // initializer, which exercises a different admission path.
    let implementation = package
        .implementations
        .iter_mut()
        .find(|implementation| {
            !implementation.associated.is_empty()
                || !implementation.associated_initializers.is_empty()
        })
        .unwrap();
    let (member, completed) = if let Some(value) = implementation.associated.first() {
        value.clone()
    } else {
        let (member, initializer) = implementation.associated_initializers.remove(0);
        (member, initializer.into_expression())
    };
    nir::completed_values::validate_with_newtypes(&completed, package.newtypes.iter(), &mut 10000)
        .unwrap();
    implementation.associated = vec![(member, completed.clone())];
    let interface = implementation.interface_instance().unwrap();
    package.dispatch_instances.clear();
    package
        .dispatch_instances
        .push(nir::program::DispatchInstance {
            interface: interface.declaration,
            interface_arguments: interface.arguments,
            subject: implementation.subject.clone(),
            implementation: implementation.declaration,
            arguments: vec![],
            methods: vec![],
            associated: vec![(member, completed)],
            associated_initializers: vec![],
        });
    comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    let mut forged = package.clone();
    for (_, value) in forged
        .implementations
        .iter_mut()
        .flat_map(|implementation| &mut implementation.associated)
        .chain(
            forged
                .dispatch_instances
                .iter_mut()
                .flat_map(|instance| &mut instance.associated),
        )
    {
        let nir::program::ExprKind::NewtypeConvert(payload) = &mut value.kind else {
            panic!("expected frozen wrapper")
        };
        assert!(matches!(
            payload.kind,
            nir::program::ExprKind::Literal(nir::program::Literal::Bool(_))
        ));
        // Keep both copies and their Bool annotation equal, but forge data of
        // another intrinsic domain. Instance equality alone cannot reject it.
        payload.kind = nir::program::ExprKind::Literal(nir::program::Literal::Char('x'));
    }
    assert!(comptime_interpretor::Interpreter::new(&forged, Default::default()).is_err());
    let mut invalid = package.clone();
    invalid.newtypes[0].representation = nir::Term::Primitive(nir::Primitive::Char);
    assert!(comptime_interpretor::Interpreter::new(&invalid, Default::default()).is_err());
}

#[test]
fn associated_nominal_constants_preserve_nested_generic_layouts() {
    for generic in [false, true] {
        let (implementation, subject) = if generic {
            (
                "impl Source for Holder<T> where T {assoc packet:Packet<bool> = (Packet<bool> {value:true});assoc choice:Choice<bool> = .some(true);}",
                "Holder<bool>",
            )
        } else {
            (
                "impl Source for bool {assoc packet:Packet<bool> = (Packet<bool> {value:true});assoc choice:Choice<bool> = .some(true);}",
                "bool",
            )
        };
        let source = format!(
            "struct Holder where T {{}}
            struct Packet where T {{value:T}}
            enum Choice where T {{none,some(T)}}
            trait Source {{assoc packet:Packet<bool>;assoc choice:Choice<bool>;}}
            {implementation}
            fn run()->bool {{let packet={subject}.as(Source).packet;
                let choice={subject}.as(Source).choice;
                packet.value and (choice match {{.some(value)=>value,.none=>false}})}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn interpreter_admission_rejects_forged_nominal_completed_dispatch_payloads() {
    use nir::program::{DispatchInstance, ExprKind as K, Literal};
    for (definition, initializer) in [
        ("struct Payload {flag:bool}", "(Payload {flag:true})"),
        ("enum Payload {some(bool),none}", ".some(true)"),
    ] {
        let source = format!(
            "{definition}trait Source {{assoc value:Payload;}}
            impl Source for bool {{assoc value:Payload={initializer};}}"
        );
        let Artifact::Nir(mut package) = compile(&source, Emit::Nir).unwrap() else {
            panic!("NIR")
        };
        let implementation = package
            .implementations
            .iter_mut()
            .find(|i| !i.associated.is_empty() || !i.associated_initializers.is_empty())
            .unwrap();
        let (member, completed) = if let Some(value) = implementation.associated.first() {
            value.clone()
        } else {
            let (member, initializer) = implementation.associated_initializers.remove(0);
            (member, initializer.into_expression())
        };
        implementation.associated = vec![(member, completed.clone())];
        let interface = implementation.interface_instance().unwrap();
        package.dispatch_instances = vec![DispatchInstance {
            interface: interface.declaration,
            interface_arguments: interface.arguments,
            subject: implementation.subject.clone(),
            implementation: implementation.declaration,
            arguments: vec![],
            methods: vec![],
            associated: vec![(member, completed)],
            associated_initializers: vec![],
        }];
        comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        for mutation in 0..4 {
            let mut forged = package.clone();
            // Forge both copies consistently: equality between template and
            // dispatch is necessary but cannot establish declaration layout.
            for (_, value) in forged
                .implementations
                .iter_mut()
                .flat_map(|i| &mut i.associated)
                .chain(
                    forged
                        .dispatch_instances
                        .iter_mut()
                        .flat_map(|i| &mut i.associated),
                )
            {
                let (declaration, fields) = match &mut value.kind {
                    K::Record {
                        declaration,
                        fields,
                    }
                    | K::Variant {
                        declaration,
                        fields,
                        ..
                    } => (declaration, fields),
                    _ => panic!("nominal constructor"),
                };
                match mutation {
                    0 => fields.clear(),
                    1 => {
                        fields[0] = nir::program::Expr {
                            ty: nir::Term::Primitive(nir::Primitive::Char),
                            kind: K::Literal(Literal::Char('x')),
                            source: fields[0].source,
                        }
                    }
                    2 => declaration.local = declaration.local.wrapping_add(10000),
                    _ => {
                        value.kind = match &value.kind {
                            K::Record {
                                declaration,
                                fields,
                            } => K::Variant {
                                declaration: *declaration,
                                index: 0,
                                fields: fields.clone(),
                            },
                            K::Variant {
                                declaration,
                                fields,
                                ..
                            } => K::Variant {
                                declaration: *declaration,
                                index: usize::MAX,
                                fields: fields.clone(),
                            },
                            _ => unreachable!(),
                        }
                    }
                }
            }
            assert!(
                comptime_interpretor::Interpreter::new(&forged, Default::default()).is_err(),
                "accepted mutation {mutation} of {definition}"
            );
        }
    }
}
