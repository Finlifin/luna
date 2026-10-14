use luna::{
    frontend::Artifact,
    packages::{self, SourcePackage},
};
use resolve::DependencyEdge;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Inputs(PathBuf);
impl Inputs {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "luna-packages-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn package(&self, id: u32, name: &str, source: &str) -> SourcePackage {
        let entry = self.0.join(format!("{name}.fl"));
        std::fs::write(&entry, source).unwrap();
        SourcePackage {
            resolution_id: id,
            identity: u64::from(id) + 1000,
            name: name.into(),
            entry,
        }
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn dependency(requester: u32, alias: &str, provider: u32) -> DependencyEdge {
    DependencyEdge {
        requester,
        alias: alias.into(),
        provider,
    }
}

#[test]
fn automatic_splice_uses_provider_identity_across_package_boundaries() {
    let input = Inputs::new();
    let app = input.package(7, "app",
        "use support.{FLAG, Conversion}; fn entry()->(bool,bool) {bool.as(Conversion).splice(FLAG)}");
    let library = input.package(9, "library",
        "typealias Type=.Type'builtin;
         pub trait Conversion {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
         impl Conversion for bool {assoc RuntimeTarget:Type=(bool,bool);fn splice(comptime data:bool)->(bool,bool) {(data,not data)}}
         pub const FLAG=true;");
    let inputs = [app, library];
    let dependencies = [dependency(7, "support", 9)];
    let Artifact::Nir(baseline) = packages::compile(&inputs, &dependencies, 7).unwrap() else { panic!("NIR") };
    let id = baseline.traits.iter().find(|t| t.name == "Conversion").unwrap().declaration;
    assert_eq!(id.package, 1009);
    std::fs::write(&inputs[0].entry,
        "use support.FLAG; fn entry()->(bool,bool) {FLAG}").unwrap();
    let Artifact::Nir(named) = packages::compile_with_splice(&inputs, &dependencies, 7, None, "library::Conversion").unwrap() else { panic!("NIR") };
    assert_eq!(luna::execution::interpret(&named, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::Tuple(vec![comptime_interpretor::Value::Bool(true), comptime_interpretor::Value::Bool(false)]));
    let protocols = elaboration::program::Protocols { splice: Some(id), ..Default::default() };
    let Artifact::Nir(program) = packages::compile_with_protocols(&inputs, &dependencies, 7, None, &protocols).unwrap() else { panic!("NIR") };
    assert_eq!(luna::execution::interpret(&program, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::Tuple(vec![comptime_interpretor::Value::Bool(true), comptime_interpretor::Value::Bool(false)]));
    let foreign = elaboration::program::Protocols { splice: Some(nir::DeclarationId { package: 1007, ..id }), ..Default::default() };
    assert!(packages::compile_with_protocols(&inputs, &dependencies, 7, None, &foreign).is_err());
}
#[test]
fn imported_generic_function_executes_from_its_provider_package() {
    let input = Inputs::new();
    let app = input.package(
        7,
        "app",
        "use support.identity; fn entry()->true'type {identity(true)}",
    );
    let library = input.package(9, "library", "pub fn identity(value:T)->T where T {value}");
    let Artifact::Nir(program) =
        packages::compile(&[app, library], &[dependency(7, "support", 9)], 7).unwrap()
    else {
        panic!("NIR")
    };
    let function = program
        .functions
        .iter()
        .find(|f| f.name == "identity")
        .unwrap();
    assert_eq!(function.declaration.package, 1009);
    assert!(function.type_parameters.iter().all(|id| id.package == 1009));
    assert_eq!(
        luna::execution::interpret(&program, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
#[test]
fn cross_package_nominal_types_keep_provider_identity() {
    let input = Inputs::new();
    let app = input.package(1, "app", "use data.{Item, make}; fn entry()->Item {make()}");
    let library = input.package(
        2,
        "library",
        "pub struct Item {} pub fn make()->Item {Item {}}",
    );
    let Artifact::Nir(program) =
        packages::compile(&[app, library], &[dependency(1, "data", 2)], 1).unwrap()
    else {
        panic!("NIR")
    };
    let value = luna::execution::interpret(&program, "entry", 100_000).unwrap();
    assert!(
        matches!(value,comptime_interpretor::Value::Record {declaration,..} if declaration.package==1002)
    );
}
#[test]
fn visibility_failures_prevent_multi_package_elaboration() {
    for declaration in ["fn hidden() {}", "private fn hidden() {}"] {
        let input = Inputs::new();
        let app = input.package(1, "app", "use dep.hidden; fn entry(){hidden()}");
        let library = input.package(2, "library", declaration);
        let error = packages::compile(&[app, library], &[dependency(1, "dep", 2)], 1)
            .err()
            .unwrap();
        assert!(error.contains("resolution failed"), "{error}");
    }
}

#[test]
fn imported_opaque_heads_keep_their_provider_and_never_unfold() {
    let inputs = Inputs::new();
    let app = inputs.package(
        1,
        "app",
        "use types.Named; typealias Name=Named<\"hello\">; fn pass(value:Name)->Name {value}",
    );
    let lib = inputs.package(
        2,
        "library",
        "pub pure comptime fn Named<Name: \"\"'type> -> .Type'builtin {.u8'builtin}",
    );
    let Artifact::Nir(program) =
        packages::compile(&[app, lib], &[dependency(1, "types", 2)], 1).unwrap()
    else {
        panic!("NIR")
    };
    let graph = luna::instantiation::instantiate(&program, "pass", 100_000).unwrap();
    let nir::Term::Function { parameters, result } = &graph.functions[0].function.signature else {
        panic!("function")
    };
    assert!(matches!(&parameters[0].ty,nir::Term::NormalForm {head,..} if head.package==1002));
    assert_eq!(&parameters[0].ty, result.as_ref());
    assert!(
        graph
            .normal_forms
            .iter()
            .all(|h| h.declaration.package == 1002)
    );
}

#[test]
fn imported_trait_contracts_are_inherited_by_local_implementations() {
    let inputs = Inputs::new();
    let library = "typealias Byte=.u8'builtin; pub trait Step { fn next(value:Byte)->result:Byte where requires value < 255, ensures result == value'old + 1; } pub struct Remote {}";
    let app=inputs.package(1,"app","typealias Byte=.u8'builtin; use api.Step; struct Local {} impl Step for Local {fn next(value:Byte)->Byte {value + 1}} fn entry()->Byte {Local.as(Step).next(41)}");
    let lib = inputs.package(2, "library", library);
    let Artifact::Nir(program) =
        packages::compile(&[app, lib], &[dependency(1, "api", 2)], 1).unwrap()
    else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret(&program, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(42)
    );
    let app=inputs.package(1,"app","typealias Byte=.u8'builtin; use api.{Step,Remote}; impl Step for Remote {fn next(value:Byte)->Byte {value + 1}} fn entry(){}");
    let lib = inputs.package(2, "library", library);
    let error = packages::compile(&[app, lib], &[dependency(1, "api", 2)], 1)
        .err()
        .unwrap();
    assert!(
        error.to_lowercase().contains("ownership") || error.contains("orphan"),
        "{error}"
    );
}

#[test]
fn entry_selection_is_scoped_to_the_requested_package() {
    let inputs = Inputs::new();
    let app = inputs.package(1, "app", "fn entry()->true'type {true}");
    let lib = inputs.package(2, "library", "fn entry()->true'type {false}");
    let Artifact::Nir(program) = packages::compile(&[app, lib], &[], 1).unwrap() else {
        panic!("NIR")
    };
    assert!(
        luna::execution::interpret(&program, "entry", 100_000)
            .unwrap_err()
            .contains("ambiguous")
    );
    assert_eq!(
        luna::execution::interpret_in_package(&program, 1001, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret_in_package(&program, 1002, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
    assert!(luna::execution::interpret_in_package(&program, 1003, "entry", 100_000).is_err());
}

#[test]
fn builtin_prelude_uses_ordinary_glob_precedence_and_identity() {
    let inputs = Inputs::new();
    let foundation = "pub mod prelude {pub typealias Flag=true'type; pub fn identity(value:T)->T where T {value}}";
    for (source, expected) in [
        (
            "mod child {fn get()->Flag {identity(true)}} fn entry()->Flag {child.get()}",
            true,
        ),
        (
            "use builtin.prelude.*; fn entry()->Flag {identity(true)}",
            true,
        ),
        (
            "fn identity(value:Flag)->Flag {false} fn entry()->Flag {identity(true)}",
            false,
        ),
        ("mod builtin {} fn entry()->Flag {identity(true)}", true),
    ] {
        let app = inputs.package(1, "app", source);
        let builtin = inputs.package(2, "foundation", foundation);
        let Artifact::Nir(program) =
            packages::compile_with_builtin(&[app, builtin], &[], 1, Some(2)).unwrap()
        else {
            panic!("NIR")
        };
        assert_eq!(
            luna::execution::interpret_in_package(&program, 1001, "entry", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(expected)
        );
    }
}

#[test]
fn builtin_prelude_can_be_reexported_but_does_not_reexport_from_consumers() {
    let inputs = Inputs::new();
    let app = inputs.package(1, "app", "fn entry()->Flag {true}");
    let builtin = inputs.package(
        2,
        "foundation",
        "pub mod base {pub typealias Flag=true'type;} pub use base as prelude;",
    );
    let Artifact::Nir(program) =
        packages::compile_with_builtin(&[app, builtin], &[], 1, Some(2)).unwrap()
    else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret_in_package(&program, 1001, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let app = inputs.package(1, "app", "fn entry(){}");
    let builtin = inputs.package(
        2,
        "foundation",
        "pub mod prelude {pub typealias Flag=true'type;}",
    );
    let consumer = inputs.package(3, "consumer", "use facade.Flag;");
    assert!(
        packages::compile_with_builtin(
            &[app, builtin, consumer],
            &[dependency(3, "facade", 1)],
            1,
            Some(2)
        )
        .is_err()
    );
}

#[test]
fn prelude_conflicts_and_private_sources_are_rejected() {
    let inputs = Inputs::new();
    for foundation in [
        "mod prelude {pub typealias Flag=true'type;}",
        "pub mod prelude {typealias Flag=true'type;}",
        "fn hidden(){}",
        "pub mod prelude {pub typealias Flag=true'type;} fn probe()->Flag {true}",
    ] {
        let app = inputs.package(1, "app", "fn entry()->Flag {true}");
        let builtin = inputs.package(2, "foundation", foundation);
        assert!(
            packages::compile_with_builtin(&[app, builtin], &[], 1, Some(2)).is_err(),
            "{foundation}"
        );
    }
    let app = inputs.package(1, "app", "use extra.prelude.*; fn entry()->Flag {true}");
    let builtin = inputs.package(
        2,
        "foundation",
        "pub mod prelude {pub typealias Flag=true'type;}",
    );
    let other = inputs.package(
        3,
        "extra",
        "pub mod prelude {pub typealias Flag=true'type;}",
    );
    assert!(
        packages::compile_with_builtin(
            &[app, builtin, other],
            &[dependency(1, "extra", 3)],
            1,
            Some(2)
        )
        .is_err()
    );
}

#[test]
fn actual_builtin_primitive_module_exports_intrinsic_types() {
    let inputs = Inputs::new();
    let app=inputs.package(1,"app","use primitives.*; fn entry()->u8 {41} fn negate(value:bool)->Bool {not value} comptime fn Selected()->Type {u32} fn text(value:str)->str {value} fn character(value:char)->Char {value}");
    let primitives = SourcePackage {
        resolution_id: 2,
        identity: 1002,
        name: "primitives".into(),
        entry: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../library/builtin/primitives.fl"),
    };
    let Artifact::Nir(program) =
        packages::compile(&[app, primitives], &[dependency(1, "primitives", 2)], 1).unwrap()
    else {
        panic!("NIR")
    };
    assert_eq!(
        luna::execution::interpret_in_package(&program, 1001, "entry", 100_000).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(41)
    );
    assert_eq!(
        luna::invocation::invoke(&program, "Selected", 100_000).unwrap(),
        comptime_interpretor::Value::Type(nir::Term::Primitive(nir::Primitive::U32))
    );
    let function = program
        .functions
        .iter()
        .find(|f| f.name == "negate")
        .unwrap();
    let mut interpreter =
        comptime_interpretor::Interpreter::new(&program, Default::default()).unwrap();
    for input in [false, true] {
        assert_eq!(
            interpreter
                .invoke(
                    function.declaration,
                    vec![comptime_interpretor::Value::Bool(input)]
                )
                .unwrap(),
            comptime_interpretor::Value::Bool(!input)
        );
    }
}

#[test]
fn bool_remains_reserved_in_binding_positions() {
    let inputs = Inputs::new();
    for source in [
        "fn bool() {}",
        "typealias bool=true'type;",
        "fn entry(){let bool=true;}",
    ] {
        let app = inputs.package(1, "app", source);
        assert!(packages::compile(&[app], &[], 1).is_err(), "{source}");
    }
}

#[test]
fn primitive_library_aliases_preserve_exact_type_identity() {
    use nir::Primitive as P;
    let aliases = [
        ("u8", P::U8),
        ("u16", P::U16),
        ("u32", P::U32),
        ("u64", P::U64),
        ("u128", P::U128),
        ("usize", P::Usize),
        ("i8", P::I8),
        ("i16", P::I16),
        ("i32", P::I32),
        ("i64", P::I64),
        ("i128", P::I128),
        ("isize", P::Isize),
        ("f32", P::F32),
        ("f64", P::F64),
        ("Bool", P::Bool),
        ("char", P::Char),
        ("Char", P::Char),
        ("str", P::Str),
        ("Symbol", P::Symbol),
        ("Integer", P::Integer),
        ("Int", P::Integer),
        ("Float", P::Float),
        ("void", P::Void),
        ("Unit", P::Void),
        ("noreturn", P::Never),
        ("Never", P::Never),
        ("Type", P::Type),
    ];
    let mut source = String::from("use primitives.*; ");
    for (index, (name, primitive)) in aliases.iter().enumerate() {
        let phase = if matches!(primitive, P::Integer | P::Float | P::Type) {
            "comptime "
        } else {
            ""
        };
        source.push_str(&format!(
            "{phase}fn Hold{index}(value:{name})->{name} {{value}} "
        ));
    }
    let inputs = Inputs::new();
    let app = inputs.package(1, "app", &source);
    let library = SourcePackage {
        resolution_id: 2,
        identity: 1002,
        name: "primitives".into(),
        entry: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../library/builtin/primitives.fl"),
    };
    let Artifact::Nir(program) =
        packages::compile(&[app, library], &[dependency(1, "primitives", 2)], 1).unwrap()
    else {
        panic!("NIR")
    };
    for (index, (_, primitive)) in aliases.iter().enumerate() {
        let function = program
            .functions
            .iter()
            .find(|f| f.name == format!("Hold{index}"))
            .unwrap();
        let nir::Term::Function { parameters, result } = &function.signature else {
            panic!("signature")
        };
        assert_eq!(parameters[0].ty, nir::Term::Primitive(*primitive));
        assert_eq!(result.as_ref(), &nir::Term::Primitive(*primitive));
    }
}

#[test]
fn imported_comptime_primitives_cannot_be_hidden_in_runtime_signatures() {
    let inputs = Inputs::new();
    for name in ["Integer", "Int", "Float", "Type"] {
        let app = inputs.package(1, "app", &format!(
            "use primitives.*; typealias Hidden = {name}; fn Hold(value:Hidden)->Hidden {{value}}"
        ));
        let library = SourcePackage {
            resolution_id: 2,
            identity: 1002,
            name: "primitives".into(),
            entry: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../library/builtin/primitives.fl"),
        };
        let error = match packages::compile(&[app, library], &[dependency(1, "primitives", 2)], 1) {
            Err(error) => error,
            Ok(_) => panic!("runtime storage accepted {name}"),
        };
        assert!(
            error.contains("comptime-only value cannot inhabit runtime storage"),
            "{name}: {error}"
        );
    }
}

#[test]
fn source_language_items_bind_provider_identity_not_import_spelling() {
    use intrinsic::{LangItem, LangItemDef, LangItems};
    let inputs = Inputs::new();
    let app = inputs.package(
        1,
        "app",
        "use protocols.Reproduce; trait Copy {} fn entry()->bool {true}",
    );
    let provider = inputs.package(
        2,
        "protocols",
        "pub trait Reproduce {} pub fn value()->bool {true}",
    );
    let Artifact::Nir(program) =
        packages::compile(&[app, provider], &[dependency(1, "protocols", 2)], 1).unwrap()
    else {
        panic!("NIR")
    };
    let designated = program
        .traits
        .iter()
        .find(|t| t.name == "Reproduce")
        .unwrap()
        .declaration;
    let lookalike = program
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    assert_eq!(designated.package, 1002);
    assert_eq!(lookalike.package, 1001);
    let mut items = LangItems::new();
    assert_eq!(items.get(LangItem::Copy), None);
    items
        .bind_source_traits(&program, &[(LangItem::Copy, designated)])
        .unwrap();
    assert_eq!(
        items.get(LangItem::Copy),
        Some(LangItemDef::UserDef(designated))
    );
    let before = items.to_string();
    let ordinary_function = program
        .functions
        .iter()
        .find(|f| f.name == "value")
        .unwrap()
        .declaration;
    assert!(
        items
            .bind_source_traits(&program, &[(LangItem::Clone, ordinary_function)])
            .is_err()
    );
    assert_eq!(items.to_string(), before);
    assert!(
        items
            .bind_source_traits(&program, &[(LangItem::Copy, lookalike)])
            .is_err()
    );
    assert_eq!(items.to_string(), before);
}

#[test]
fn explicit_nested_package_roots_are_not_rediscovered_as_local_modules() {
    let inputs = Inputs::new();
    let nested = inputs.0.join("vendor");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("lib.fl"), "pub fn provided()->bool {true}").unwrap();
    std::fs::write(inputs.0.join("main.fl"), "use support.provided; fn entry()->bool {provided()}").unwrap();
    let app = SourcePackage { resolution_id: 1, identity: 101, name: "app".into(), entry: inputs.0.join("main.fl") };
    let library = SourcePackage { resolution_id: 2, identity: 202, name: "library".into(), entry: nested.join("lib.fl") };
    let sources = [app, library];
    let Artifact::Nir(program) = packages::compile(&sources, &[dependency(1, "support", 2)], 1).unwrap() else { panic!("NIR") };
    let providers: Vec<_> = program.functions.iter().filter(|f| f.name == "provided").collect();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].declaration.package, 202);
    assert_eq!(luna::execution::interpret_in_package(&program, 101, "entry", 100_000).unwrap(), comptime_interpretor::Value::Bool(true));
    // Removing the edge must not make a dependency reachable via filesystem layout.
    std::fs::write(inputs.0.join("main.fl"), "use vendor.lib.provided; fn entry()->bool {provided()}").unwrap();
    assert!(packages::compile(&sources, &[], 1).is_err());
    // Without an explicit package input the same directory is ordinary source.
    let Artifact::Nir(program) = packages::compile(&sources[..1], &[], 1).unwrap() else { panic!("NIR") };
    assert!(program.functions.iter().filter(|f| f.name == "provided").all(|f| f.declaration.package == 101));
}

#[test]
fn explicit_single_file_dependency_is_excluded_without_hiding_sibling_modules() {
    let inputs = Inputs::new();
    let app = SourcePackage { resolution_id: 1, identity: 101, name: "app".into(), entry: inputs.0.join("main.fl") };
    std::fs::write(&app.entry, "use dep.provided; fn entry()->bool {provided() and helper.local()}").unwrap();
    std::fs::write(inputs.0.join("helper.fl"), "pub fn local()->bool {true}").unwrap();
    let library = inputs.package(2, "library", "pub fn provided()->bool {true}");
    let Artifact::Nir(program) = packages::compile(&[app, library], &[dependency(1, "dep", 2)], 1).unwrap() else { panic!("NIR") };
    assert_eq!(program.functions.iter().filter(|f| f.name == "provided").count(), 1);
    assert_eq!(luna::execution::interpret_in_package(&program, 101, "entry", 100_000).unwrap(), comptime_interpretor::Value::Bool(true));
}


#[test]
fn trait_type_defaults_keep_definition_package_scope() {
    let inputs = Inputs::new();
    let app=inputs.package(1,"app","use support.Pick;typealias Choice=.char'builtin;struct Host {} impl Pick for Host {fn pick(value:bool)->bool {value}} fn entry()->bool {Host.as(Pick).pick(true)}");
    let library=inputs.package(2,"support","typealias Type=.Type'builtin;typealias Choice=bool;pub trait Pick where .T:Type=Choice {fn pick(value:T)->bool;}");
    let Artifact::Nir(program)=packages::compile(&[app,library],&[dependency(1,"support",2)],1).unwrap() else {panic!("NIR")};
    assert_eq!(luna::execution::interpret_in_package(&program,1001,"entry",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
}


#[test]
fn nominal_type_defaults_keep_provider_scope_and_identity() {
    let inputs=Inputs::new();
    let app=inputs.package(1,"app","use support.Holder;typealias Choice=.char'builtin;fn entry()->bool {Holder {value:true}.value}");
    let library=inputs.package(2,"support","typealias Choice=bool;pub struct Holder where .T:.Type'builtin=Choice {value:T,}");
    let Artifact::Nir(program)=packages::compile(&[app,library],&[dependency(1,"support",2)],1).unwrap() else {panic!("NIR")};
    assert_eq!(luna::execution::interpret_in_package(&program,1001,"entry",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
    assert_eq!(program.structs.iter().find(|item| item.name=="Holder").unwrap().declaration.package,1002);
}


#[test]
fn typealias_defaults_keep_definition_scope_across_packages() {
    let inputs=Inputs::new();
    let app=inputs.package(1,"app","use support.Alias;typealias Choice=.char'builtin;fn entry()->Alias {true}");
    let library=inputs.package(2,"support","typealias Choice=bool;pub typealias Alias<.T:.Type'builtin=Choice> = T;");
    let Artifact::Nir(program)=packages::compile(&[app,library],&[dependency(1,"support",2)],1).unwrap() else {panic!("NIR")};
    assert_eq!(luna::execution::interpret_in_package(&program,1001,"entry",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
}

#[test]
fn symbolic_normal_form_defaults_retain_provider_scope_and_head_identity() {
    let inputs=Inputs::new();
    let app=inputs.package(1,"app","use support.Head;typealias Choice=.char'builtin;typealias Alias<T> = Head<T,true>;fn keep(value:Alias<T>)->Head<T,true,.U=bool> where T {value} fn entry()->bool {true}");
    let library=inputs.package(2,"support","typealias Type=.Type'builtin;typealias Choice=bool;pub pure comptime fn Head<T,N:bool,.M:bool=N,.U:Type=Choice>->Type {T}");
    let Artifact::Nir(program)=packages::compile(&[app,library],&[dependency(1,"support",2)],1).unwrap() else {panic!("NIR")};
    let keep=program.functions.iter().find(|f| f.name=="keep").unwrap();
    let nir::Term::Function {parameters,result}=&keep.signature else {panic!("signature")};
    assert_eq!(parameters[0].ty,**result);
    let nir::Term::NormalForm {head,arguments,..}=&**result else {panic!("head identity")};
    assert_eq!(head.package,1002);
    assert_eq!(arguments.last(),Some(&nir::Term::Primitive(nir::Primitive::Bool)));
    assert_eq!(luna::execution::interpret_in_package(&program,1001,"entry",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
}

#[test]
fn source_tuple_selectors_bind_provider_identities_before_elaboration() {
    use luna::protocols::type_constructors::Selectors;
    let input = Inputs::new();
    let app = input.package(7, "app", "use core.Tuple; typealias Byte=.u8'builtin;
        fn entry()->Tuple<bool,Byte> {(true,9)}
        fn check()->bool {let (flag,byte)=entry();flag and byte==9}");
    let builtin = input.package(9, "core", "use sequence.List; typealias Type=.Type'builtin;
        pub pure comptime fn Tuple<...Es:List<Type>> -> Type {bool}");
    let meta = input.package(11, "sequence", "typealias Type=.Type'builtin;
        pub pure comptime fn List<T> -> Type {T}");
    let packages = [app, builtin, meta];
    let dependencies = [dependency(7,"core",9), dependency(9,"sequence",11)];
    let selectors = Selectors { meta_list: Some("sequence::List"), tuple: Some("core::Tuple"), ..Default::default() };
    let Artifact::Nir(program) = packages::compile_with_source_protocols(&packages,&dependencies,7,None,&Default::default(),selectors).unwrap() else { panic!("NIR"); };
    let tuple = program.normal_forms.iter().find(|h|h.role == nir::normal_forms::Role::Tuple).unwrap();
    assert_eq!(tuple.declaration.package,1009);
    let entry = program.functions.iter().find(|f|f.name=="entry").unwrap();
    assert!(matches!(&entry.body.ty, nir::Term::NormalForm {head,..} if *head == tuple.declaration));
    assert_eq!(luna::execution::interpret(&program,"check",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
    let conflict = elaboration::program::Protocols { tuple:Some(nir::DeclarationId {package:1007,..tuple.declaration}),..Default::default() };
    assert!(packages::compile_with_source_protocols(&packages,&dependencies,7,None,&conflict,selectors).err().unwrap().contains("conflicts"));
    for selector in ["Tuple","missing::Tuple","core::Missing","core::Tuple::nested"] {
        assert!(packages::compile_with_source_protocols(&packages,&dependencies,7,None,&Default::default(),Selectors {tuple:Some(selector),..selectors}).is_err(),"{selector}");
    }
    std::fs::write(&packages[2].entry,"pub struct List {}").unwrap();
    let error = packages::compile_with_source_protocols(&packages,&dependencies,7,None,&Default::default(),selectors).err().unwrap();
    assert!(error.contains("pure normal-form"), "{error}");
    let wrong = input.package(13,"wrong","typealias Type=.Type'builtin; pub pure comptime fn List<A,B> -> Type {A}");
    let error = packages::compile_with_source_protocols(&[wrong],&[],13,None,&Default::default(),
        Selectors {meta_list:Some("wrong::List"),..Default::default()}).err().unwrap();
    assert!(error.contains("invalid constructor signature"), "{error}");
}

#[test]
fn source_tuple_selectors_follow_provider_module_paths() {
    use luna::protocols::type_constructors::Selectors;
    let input=Inputs::new();
    let app=input.package(7,"app","use support.types.Tuple; fn entry()->Tuple<bool,bool> {(true,false)}
        fn check()->bool {let (a,b)=entry();a and not b}");
    let library=input.package(9,"library","typealias Type=.Type'builtin;
        pub mod meta {pub pure comptime fn List<T>->Type {T}}
        pub mod types {pub pure comptime fn Tuple<...Es:meta.List<Type>>->Type {bool}}
        mod other {pure comptime fn Tuple<...Es:meta.List<Type>>->Type {bool}}");
    let inputs=[app,library];
    let dependencies=[dependency(7,"support",9)];
    let selectors=Selectors {meta_list:Some("library::meta::List"),tuple:Some("library::types::Tuple"),..Default::default()};
    let Artifact::Nir(program)=packages::compile_with_source_protocols(&inputs,&dependencies,7,None,&Default::default(),selectors).unwrap() else {panic!("NIR")};
    let tuple=program.normal_forms.iter().find(|head|head.role==nir::normal_forms::Role::Tuple).unwrap();
    assert_eq!(tuple.declaration.package,1009);
    assert_eq!(luna::execution::interpret(&program,"check",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
    for wrong in ["support::types::Tuple","app::types::Tuple","library::Tuple"] {
        assert!(packages::compile_with_source_protocols(&inputs,&dependencies,7,None,&Default::default(),
            Selectors {tuple:Some(wrong),..selectors}).is_err(),"accepted {wrong}");
    }
}

#[test]
fn source_tuple_selectors_walk_directory_only_modules() {
    let input=Inputs::new();
    let root=input.0.join("library");
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("lib.fl"),"typealias Type=.Type'builtin;
        mod meta {pure comptime fn List<T>->Type {T}}
        fn pair()->types.tuples.Tuple<bool,bool> {(true,false)}
        fn check()->bool {let (a,b)=pair();a and not b}").unwrap();
    std::fs::write(root.join("types/tuples.fl"),"pub pure comptime fn Tuple<...Es:meta.List<Type>>->Type {bool}").unwrap();
    let library=SourcePackage {resolution_id:9,identity:1009,name:"library".into(),entry:root.join("lib.fl")};
    let Artifact::Nir(program)=packages::compile_with_source_protocols(&[library],&[],9,None,&Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list:Some("library::meta::List"),tuple:Some("library::types::tuples::Tuple"),..Default::default()
        }).unwrap() else {panic!("NIR")};
    assert_eq!(luna::execution::interpret(&program,"check",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
}

#[test]
fn module_qualified_splice_binding_selects_the_declared_provider_trait() {
    let input=Inputs::new();
    let app=input.package(7,"app","use support.FLAG; fn entry()->(bool,bool) {FLAG}");
    let library=input.package(9,"library","typealias Type=.Type'builtin;
        pub mod conversion {
            pub trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
            impl Splice for bool {assoc RuntimeTarget:Type=(bool,bool);fn splice(comptime data:bool)->(bool,bool) {(data,not data)}}
        }
        mod decoy {trait Splice {}}
        pub const FLAG=true;");
    let inputs=[app,library];
    let dependencies=[dependency(7,"support",9)];
    let Artifact::Nir(program)=packages::compile_with_splice(&inputs,&dependencies,7,None,"library::conversion::Splice").unwrap() else {panic!("NIR")};
    assert_eq!(luna::execution::interpret(&program,"entry",100_000).unwrap(),
        comptime_interpretor::Value::Tuple(vec![comptime_interpretor::Value::Bool(true),comptime_interpretor::Value::Bool(false)]));
    for wrong in ["Splice","library::conversion","library::Splice","support::conversion::Splice","library::conversion::::Splice","library::conversion::Splice::nested"] {
        assert!(packages::compile_with_splice(&inputs,&dependencies,7,None,wrong).is_err(),"accepted {wrong}");
    }
    let source=std::fs::read_to_string(&inputs[1].entry).unwrap();
    std::fs::write(&inputs[1].entry,source.replace("mod decoy {trait Splice {}}","")).unwrap();
    let Artifact::Nir(unique)=packages::compile_with_splice(&inputs,&dependencies,7,None,"Splice").unwrap() else {panic!("NIR")};
    assert_eq!(luna::execution::interpret(&unique,"entry",100_000).unwrap(),
        luna::execution::interpret(&program,"entry",100_000).unwrap());
}

#[test]
fn public_nested_nominal_functions_execute_from_an_importing_package() {
    let input=Inputs::new();
    let app=input.package(7,"app","use support.{Flag,Choice};fn entry()->bool {Flag.read() and Choice.read()}");
    let library=input.package(9,"library","pub struct Flag {pub fn read()->bool {true}}
        pub enum Choice {yes,pub fn read()->bool {true}}");
    let Artifact::Nir(program)=packages::compile(&[app,library],&[dependency(7,"support",9)],7).unwrap() else {panic!("NIR")};
    assert_eq!(luna::execution::interpret(&program,"entry",100_000).unwrap(),comptime_interpretor::Value::Bool(true));
    assert!(program.functions.iter().filter(|function|function.name=="read").all(|function|function.declaration.package==1009));
}

#[test]
fn source_type_constructors_preserve_their_own_opaque_heads() {
    use luna::protocols::type_constructors::Selectors;
    let input=Inputs::new();
    let app=input.package(7,"app","use core.Tuple; fn pair()->Tuple<bool,bool> {(true,false)} fn entry()->bool {let (a,b)=pair();a and not b}");
    let core=input.package(9,"core","use sequence.List; typealias Type=.Type'builtin; pub pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}");
    let meta=input.package(11,"sequence","typealias Type=.Type'builtin; pub pure comptime fn List<T>->Type {List<T>}");
    let packages=[app,core,meta];
    let dependencies=[dependency(7,"core",9),dependency(9,"sequence",11)];
    let selectors=Selectors {meta_list:Some("sequence::List"),tuple:Some("core::Tuple"),..Default::default()};
    let Artifact::Nir(program)=packages::compile_with_source_protocols(&packages,&dependencies,7,None,&Default::default(),selectors).unwrap() else {panic!("NIR")};
    assert_eq!(program.normal_forms.len(),2);
    assert_eq!(program.functions.iter().filter(|f|matches!(f.name.as_str(),"Tuple"|"List") && f.phase==nir::program::FunctionPhase::NormalForm).count(),2);
    let tuple=program.normal_forms.iter().find(|head|head.role==nir::normal_forms::Role::Tuple).unwrap();
    let pair=program.functions.iter().find(|f|f.name=="pair").unwrap();
    assert!(matches!(&pair.body.ty,nir::Term::NormalForm {head,..} if *head==tuple.declaration));
    assert_eq!(luna::execution::interpret(&program,"entry",100000).unwrap(),comptime_interpretor::Value::Bool(true));
}
