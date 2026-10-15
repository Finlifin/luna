use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "newtypes-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile(&Options {
        input: path.clone(),
        emit,
        output: None,
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        tuple_type: None,
        optional_type: None,
        index_protocol: None,
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
    let error = luna::resources::analyze_with_protocols(&package, "twice", "Duplicate", None, 100_000).unwrap_err();
    assert!(error.contains("Moved") || error.contains("moved"), "{error}");
}
