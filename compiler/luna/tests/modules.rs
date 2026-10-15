//! Source discovery, module identity, resolution and execution through the driver.
use comptime_interpretor::{Budget, Interpreter, Value};
use luna::cli::{Emit, Options};
use luna::frontend::{self, Artifact};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Project(PathBuf);
impl Project {
    fn new(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "luna-modules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for (name, text) in files {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Self(root)
    }
    fn compile(&self) -> Result<nir::program::Package, String> {
        match frontend::compile(&Options {
            input: self.0.join("main.fl"),
            emit: Emit::Nir,
            output: None,
            sysroot: None,
            splice_protocol: None,
            meta_list_type: None,
            index_protocol: None,
            literal_into_protocol: None,
            arithmetic_protocols: Default::default(),
            tuple_type: None,
            optional_type: None,
        })? {
            Artifact::Nir(package) => Ok(package),
            _ => panic!("expected NIR"),
        }
    }
    fn run(&self) -> Value {
        let package = self.compile().unwrap();
        let entry = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .declaration;
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(entry, vec![])
            .unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn files_directories_and_mod_bodies_share_one_declaration_collection() {
    let project = Project::new(&[
        (
            "main.fl",
            "typealias Flag = true'type; fn run() -> (Flag,Flag,Flag) { (left.value(),nested.deep.read(),nested.value()) }",
        ),
        ("left.fl", "fn value() -> Flag { true }"),
        ("nested/mod.fl", "fn value() -> Flag { false }"),
        ("nested/deep.fl", "fn read() -> Flag { left.value() }"),
    ]);
    assert_eq!(
        project.run(),
        Value::Tuple(vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(false)
        ])
    );
}

#[test]
fn forward_imports_and_reexports_keep_function_identity() {
    let project = Project::new(&[
        (
            "main.fl",
            "use facade.value; fn run() -> true'type { value() and facade.value() and origin.value() }",
        ),
        ("facade.fl", "pub use origin.value;"),
        ("origin.fl", "pub fn value() -> true'type { true }"),
    ]);
    let package = project.compile().unwrap();
    assert_eq!(
        package
            .functions
            .iter()
            .filter(|f| f.name == "value")
            .count(),
        1
    );
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn source_discovery_rejects_file_directory_and_inline_collisions() {
    for files in [
        vec![
            ("main.fl", "fn run() {}"),
            ("same.fl", ""),
            ("same/child.fl", ""),
        ],
        vec![("main.fl", "mod same {}"), ("same.fl", "")],
    ] {
        assert!(Project::new(&files).compile().is_err());
    }
}

#[test]
fn unreferenced_source_errors_are_not_hidden() {
    assert!(
        Project::new(&[("main.fl", "fn run() {}"), ("unused.fl", "fn broken(")])
            .compile()
            .is_err()
    );
}

#[test]
fn nested_main_is_an_ordinary_module_not_another_package_root() {
    let project = Project::new(&[
        ("main.fl", "fn run() -> true'type { nested.main.value() }"),
        ("nested/main.fl", "fn value() -> true'type { true }"),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn private_declarations_are_available_to_children_but_not_siblings() {
    assert_eq!(Project::new(&[("main.fl", "mod service { private fn secret() -> true'type { true } mod child { fn read() -> true'type { secret() } } } fn run() -> true'type { service.child.read() }")]).run(), Value::Bool(true));
    for source in [
        "mod service { private fn secret() {} } fn run() { service.secret(); }",
        "mod service { private mod hidden { pub fn secret() {} } } fn run() { service.hidden.secret(); }",
        "mod service { private fn secret() {} } use service.secret; fn run() {}",
        "mod service { fn internal() {} } pub use service.internal; fn run() {}",
    ] {
        assert!(
            Project::new(&[("main.fl", source)]).compile().is_err(),
            "{source}"
        );
    }
}

#[test]
fn directory_without_mod_body_and_glob_import_chains_work() {
    let project = Project::new(&[
        (
            "main.fl",
            "use api.*; fn run() -> true'type { value() and deep.leaf.other() }",
        ),
        ("api.fl", "pub use bridge.*;"),
        ("bridge.fl", "pub use origin.value;"),
        ("origin.fl", "pub fn value() -> true'type { true }"),
        ("deep/leaf.fl", "fn other() -> true'type { true }"),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn repeated_imports_deduplicate_but_distinct_declarations_conflict() {
    assert_eq!(
        Project::new(&[
            (
                "main.fl",
                "use left.value; use left.value; fn run() -> true'type { value() }"
            ),
            ("left.fl", "fn value() -> true'type { true }"),
        ])
        .run(),
        Value::Bool(true)
    );
    assert!(
        Project::new(&[
            ("main.fl", "use left.value; use right.value; fn run() {}"),
            ("left.fl", "fn value() {}"),
            ("right.fl", "fn value() {}"),
        ])
        .compile()
        .is_err()
    );
}

#[test]
fn output_cannot_overwrite_an_automatically_discovered_source() {
    let project = Project::new(&[("main.fl", "fn run() {}"), ("other.fl", "fn keep() {}")]);
    let result = frontend::compile(&Options {
        input: project.0.join("main.fl"),
        emit: Emit::Nir,
        output: Some(project.0.join("other.fl")),
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        index_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
        tuple_type: None,
        optional_type: None,
    });
    assert!(result.err().unwrap().contains("discovered source"));
    assert_eq!(
        std::fs::read_to_string(project.0.join("other.fl")).unwrap(),
        "fn keep() {}"
    );
}

#[test]
fn glob_ambiguity_is_lazy_and_explicit_names_take_precedence() {
    let origins = [
        ("left.fl", "fn value() -> true'type { true }"),
        ("right.fl", "fn value() -> true'type { false }"),
    ];
    for imports in ["use left.*; use right.*;", "use right.*; use left.*;"] {
        let unused = format!("{imports} fn run() {{ }}");
        assert!(
            Project::new(&[("main.fl", &unused), origins[0], origins[1]])
                .compile()
                .is_ok()
        );
        let used = format!("{imports} fn run() -> true'type {{ value() }}");
        assert!(
            Project::new(&[("main.fl", &used), origins[0], origins[1]])
                .compile()
                .is_err()
        );
        let explicit = format!("{imports} use left.value; fn run() -> true'type {{ value() }}");
        assert_eq!(
            Project::new(&[("main.fl", &explicit), origins[0], origins[1]]).run(),
            Value::Bool(true)
        );
    }
}

#[test]
fn public_globs_filter_internal_names_and_reject_public_ambiguity() {
    assert_eq!(
        Project::new(&[
            (
                "main.fl",
                "use facade.value; fn run() -> true'type { value() }"
            ),
            ("facade.fl", "pub use implementation.*;"),
            (
                "implementation.fl",
                "fn hidden() {} pub fn value() -> true'type { true }"
            ),
        ])
        .run(),
        Value::Bool(true)
    );
    assert!(
        Project::new(&[
            ("main.fl", "fn run() {}"),
            ("facade.fl", "pub use left.*; pub use right.*;"),
            ("left.fl", "pub fn same() {}"),
            ("right.fl", "pub fn same() {}"),
        ])
        .compile()
        .is_err()
    );
}

#[test]
fn glob_cycles_converge_only_from_real_declarations() {
    assert_eq!(
        Project::new(&[
            ("main.fl", "use a.value; fn run() -> true'type { value() }"),
            ("a.fl", "use b.*;"),
            ("b.fl", "use a.*; fn value() -> true'type { true }"),
        ])
        .run(),
        Value::Bool(true)
    );
    assert!(
        Project::new(&[
            ("main.fl", "fn run() {}"),
            ("a.fl", "use b.*;"),
            ("b.fl", "use a.*;")
        ])
        .compile()
        .is_ok()
    );
}

#[test]
fn a_late_explicit_import_shadows_globs_before_downstream_binding() {
    let project = Project::new(&[
        ("main.fl", "use a.value; fn run() -> true'type { value() }"),
        ("a.fl", "use fallback.*; use b.value;"),
        ("b.fl", "use c.value;"),
        ("c.fl", "use actual.value;"),
        ("fallback.fl", "fn value() -> true'type { false }"),
        ("actual.fl", "fn value() -> true'type { true }"),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn aliases_preserve_identity_through_multiple_modules_and_type_names() {
    let project = Project::new(&[
        (
            "main.fl",
            "use facade.enabled as test_flag; use facade.Flag as Boolean; fn run() -> Boolean { test_flag() and origin.value() }",
        ),
        (
            "facade.fl",
            "pub use bridge.current as enabled; pub use origin.Flag;",
        ),
        ("bridge.fl", "use origin.value as current;"),
        (
            "origin.fl",
            "pub typealias Flag = true'type; pub fn value() -> Flag { true }",
        ),
    ]);
    let package = project.compile().unwrap();
    assert_eq!(
        package
            .functions
            .iter()
            .filter(|f| f.name == "value")
            .count(),
        1
    );
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn private_import_entries_are_visible_to_descendants_only() {
    let project = Project::new(&[
        ("main.fl", "fn run() -> true'type { service.child.read() }"),
        ("source.fl", "pub fn value() -> true'type { true }"),
        ("service/mod.fl", "private use source.value as internal;"),
        ("service/child.fl", "fn read() -> true'type { internal() }"),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
    for main in [
        "fn run() -> true'type { service.internal() }",
        "use service.internal; fn run() -> true'type { internal() }",
        "pub use service.internal; fn run() {}",
    ] {
        assert!(
            Project::new(&[
                ("main.fl", main),
                ("source.fl", "pub fn value() -> true'type { true }"),
                ("service.fl", "private use source.value as internal;")
            ])
            .compile()
            .is_err()
        );
    }
}

#[test]
fn accessible_private_alias_can_supply_an_explicit_public_entry() {
    let project = Project::new(&[
        (
            "main.fl",
            "use facade.exported; fn run() -> true'type { exported() }",
        ),
        ("origin.fl", "pub fn value() -> true'type { true }"),
        (
            "facade.fl",
            "private use origin.value as internal; pub use internal as exported;",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn aliases_cannot_widen_the_original_declaration_permission() {
    for origin in ["fn value() {}", "private fn value() {}"] {
        assert!(
            Project::new(&[
                ("main.fl", "pub use bridge.alias; fn run() {}"),
                ("origin.fl", origin),
                ("bridge.fl", "use origin.value as alias;")
            ])
            .compile()
            .is_err()
        );
    }
    assert!(Project::new(&[("main.fl", "mod service { private fn secret() {} mod child { pub use secret as leaked; } } fn run() {}")]).compile().is_err());
}

#[test]
fn private_globs_and_public_filtering_use_entry_permissions() {
    assert_eq!(
        Project::new(&[
            ("main.fl", "fn run() -> true'type { facade.child.read() }"),
            ("origin.fl", "pub fn value() -> true'type { true }"),
            ("facade/mod.fl", "private use origin.*;"),
            ("facade/child.fl", "fn read() -> true'type { value() }"),
        ])
        .run(),
        Value::Bool(true)
    );
    assert!(
        Project::new(&[
            ("main.fl", "use api.value; fn run() {}"),
            ("origin.fl", "pub fn value() {}"),
            ("facade.fl", "private use origin.value;"),
            ("api.fl", "pub use facade.*;"),
        ])
        .compile()
        .is_err()
    );
}

#[test]
fn repeated_entries_for_one_identity_merge_permissions_without_duplicates() {
    for imports in [
        "private use origin.value; pub use origin.value;",
        "pub use origin.value; private use origin.value;",
    ] {
        assert_eq!(
            Project::new(&[
                (
                    "main.fl",
                    "use facade.value; fn run() -> true'type { value() }"
                ),
                ("origin.fl", "pub fn value() -> true'type { true }"),
                ("facade.fl", imports),
            ])
            .run(),
            Value::Bool(true)
        );
    }
}

#[test]
fn package_and_parent_anchors_apply_to_the_entire_aliased_path() {
    let project = Project::new(&[
        (
            "main.fl",
            "fn global() -> true'type { true } fn run() -> true'type { layer.child.read() }",
        ),
        ("layer/mod.fl", "fn local() -> true'type { true }"),
        (
            "layer/child.fl",
            "use @global as root_value; use .local as parent_value; fn read() -> true'type { root_value() and parent_value() }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
    assert!(
        Project::new(&[(
            "main.fl",
            "fn global() {} mod parent { mod child { use .global; } } fn run() {}"
        )])
        .compile()
        .is_err()
    );
}

#[test]
fn grouped_imports_preserve_shared_prefixes_nested_paths_and_aliases() {
    let project = Project::new(&[
        (
            "main.fl",
            "use @api.{Flag as Boolean, inner.{yes as enabled, no as disabled}}; fn run() -> Boolean { enabled() and not disabled() }",
        ),
        ("api/mod.fl", "pub typealias Flag = true'type;"),
        (
            "api/inner.fl",
            "pub fn yes() -> Flag { true } pub fn no() -> Flag { false }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
    let artifact = frontend::compile(&Options {
        input: project.0.join("main.fl"),
        emit: Emit::Hir,
        output: None,
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        index_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
        tuple_type: None,
        optional_type: None,
    })
    .unwrap()
    .to_string();
    assert!(artifact.contains("(group (paths"), "{artifact}");
    assert!(artifact.contains("enabled") && artifact.contains("disabled"));
}

#[test]
fn grouped_globs_alias_containers_and_diamond_imports_share_identity() {
    let project = Project::new(&[
        (
            "main.fl",
            "use root.{left.*,right.*}; use root.left as selected; fn run() -> true'type { value() and selected.value() }",
        ),
        ("root/left.fl", "pub use @origin.value;"),
        ("root/right.fl", "pub use @origin.value;"),
        ("origin.fl", "pub fn value() -> true'type { true }"),
    ]);
    let package = project.compile().unwrap();
    assert_eq!(
        package
            .functions
            .iter()
            .filter(|f| f.name == "value")
            .count(),
        1
    );
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn private_name_shadows_public_glob_before_export_filtering() {
    assert!(
        Project::new(&[
            ("main.fl", "use facade.value; fn run() {}"),
            ("origin.fl", "pub fn value() {}"),
            ("hidden.fl", "pub fn value() {}"),
            ("mixed.fl", "pub use origin.*; private use hidden.value;"),
            ("facade.fl", "pub use mixed.*;"),
        ])
        .compile()
        .is_err()
    );
}

#[test]
fn imported_struct_identity_and_default_lookup_are_definition_scoped() {
    let project = Project::new(&[
        (
            "main.fl",
            "use model.FlagBox as Box; fn default_value() -> true'type { false } fn run() -> true'type { let box = Box {}; box.value }",
        ),
        (
            "model.fl",
            "fn default_value() -> true'type { true } pub struct FlagBox { value: true'type = default_value(), }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn enum_aliases_across_modules_preserve_nominal_variant_identity() {
    let project = Project::new(&[
        (
            "main.fl",
            "use model.State as Alias; fn run() -> true'type { let state = Alias.ready; model.check(state) }",
        ),
        (
            "model.fl",
            "pub fn check(value: State) -> true'type { value match { .waiting => false, .ready => true, } } pub enum State { waiting, ready, }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn enum_payload_aliases_and_field_types_resolve_in_the_defining_module() {
    let project = Project::new(&[
        (
            "main.fl",
            "use model.Message as Alias; fn run() -> true'type { model.check(Alias.data(true)) and model.check(.data(true)) }",
        ),
        (
            "model.fl",
            "typealias Flag = true'type; pub fn check(value: Message) -> Flag { value match { .none => false, .data(flag) => flag, } } pub enum Message { none, data(Flag), }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn imported_named_payload_defaults_keep_definition_scope() {
    let project = Project::new(&[
        (
            "main.fl",
            "use model.Message as Alias; fn flag() -> true'type { false } fn run() -> true'type { let message = Alias.data {}; model.check(message) }",
        ),
        (
            "model.fl",
            "fn flag() -> true'type { true } pub enum Message { data { value: true'type = flag() }, } pub fn check(message: Message) -> true'type { message match { .data { value } => value, } }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn qualified_variant_patterns_preserve_identity_through_module_and_type_aliases() {
    let project = Project::new(&[
        (
            "main.fl",
            "use model as m; use model.Message as Alias; fn check(value: Alias) -> true'type { value match { m.Message.none => false, Alias.data { flag } => flag, } } fn run() -> true'type { check(m.Message.data { flag: true }) and not check(Alias.none) }",
        ),
        (
            "model.fl",
            "pub enum Message { none, data { flag: true'type }, }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn qualified_patterns_cannot_bypass_module_visibility() {
    assert!(Project::new(&[
        ("main.fl", "fn check(value: public.Message) { value match { hidden.Secret.none => (), } } fn run() {}"),
        ("public.fl", "pub enum Message { none, }"),
        ("hidden.fl", "enum Secret { none, }"),
    ]).compile().is_err());
}

#[test]
fn imported_generic_function_keeps_definition_identity_and_infers_payload_type() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.keep as copy_value; enum Flag { yes, no, } fn run() -> true'type { copy_value(Flag.yes) match { .yes => true, .no => false, } }",
        ),
        ("helper.fl", "pub fn keep(value: T) -> T where T { value }"),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn explicit_and_contextual_generic_values_work_through_import_aliases() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.keep as identity; typealias Flag = true'type; fn run() -> Flag { let explicit = identity<Flag>; let contextual: fn(Flag) -> Flag = helper.keep; explicit(true) and contextual(true) }",
        ),
        ("helper.fl", "pub fn keep(value: T) -> T where T { value }"),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn imported_generic_optional_defaults_keep_definition_parameter_scope() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.build as make; fn run() -> (true'type, true'type) { let first = false; make(true, .first = true) }",
        ),
        (
            "helper.fl",
            "pub fn build(value: T, .first: true'type = false, .second: true'type = first) -> (T, true'type) where T { (value, second) }",
        ),
    ]);
    assert_eq!(
        project.run(),
        Value::Tuple(vec![Value::Bool(true), Value::Bool(true)])
    );
}

#[test]
fn imported_type_requirements_are_proved_in_the_callers_scope() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.convert as checked; fn run() -> true'type { checked(true) }",
        ),
        (
            "helper.fl",
            "pub fn convert(value: T) -> true'type where T, requires T == true'type { value }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
    assert!(
        Project::new(&[
            ("main.fl", "use helper.convert; fn run() { convert('x'); }"),
            (
                "helper.fl",
                "pub fn convert(value: T) -> true'type where T, requires T == true'type { value }"
            ),
        ])
        .compile()
        .is_err()
    );
}

#[test]
fn imported_nested_generic_calls_use_outer_type_context() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.factory as make; use helper.apply; fn run() -> true'type { apply(make(), true) }",
        ),
        (
            "helper.fl",
            "pub fn keep(value: T) -> T where T { value } pub fn factory() -> fn(T) -> T where T { keep<T> } pub fn apply(callback: fn(T) -> T, value: T) -> T where T { callback(value) }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn imported_generic_structs_use_declaration_identity_and_specialized_fields() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.Box as Container; fn run() -> true'type { let value = Container<true'type> { value: true }; helper.read(value) }",
        ),
        (
            "helper.fl",
            "pub struct Box where T { value: T, } pub fn read(value: Box<true'type>) -> true'type { value.value }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn imported_generic_enums_construct_and_match_specialized_payloads() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.Choice as Maybe; fn run() -> true'type { helper.read(Maybe<true'type>.value(true)) }",
        ),
        (
            "helper.fl",
            "pub enum Choice where T { none, value(T), } pub fn read(value: Choice<true'type>) -> true'type { value match { .none => false, .value(flag) => flag, } }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn imported_nominal_declarations_supply_generic_argument_inference() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.Box as Container; fn run() -> true'type { let result = helper.read(Container<true'type> { value: true }); result }",
        ),
        (
            "helper.fl",
            "pub struct Box where T { value: T, } pub fn read(value: Box<T>) -> T where T { value.value }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn imported_nominal_contracts_survive_aliases_and_generic_calls() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.Checked as Box; fn run() -> true'type { helper.read(Box<true'type> {}) }",
        ),
        (
            "helper.fl",
            "pub struct Checked where T, requires T == true'type { value: T = true, } pub fn read(value: Checked<U>) -> U where U, requires U == true'type { value.value }",
        ),
    ]);
    assert_eq!(project.run(), Value::Bool(true));
}

#[test]
fn comptime_modifiers_preserve_imports_visibility_and_function_phase() {
    let project = Project::new(&[
        (
            "main.fl",
            "use helper.choose; comptime fn run() -> true'type { choose(false) }",
        ),
        (
            "helper.fl",
            "pub pure comptime fn choose(value: true'type) -> true'type { not value }",
        ),
    ]);
    let package = project.compile().unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(entry, vec![])
            .unwrap(),
        Value::Bool(true)
    );
    let hidden = Project::new(&[
        (
            "main.fl",
            "comptime fn run() -> true'type { helper.hidden() }",
        ),
        (
            "helper.fl",
            "private comptime fn hidden() -> true'type { true }",
        ),
    ]);
    assert!(hidden.compile().is_err());
}

#[test]
fn required_comptime_type_formation_follows_imports_and_reexports() {
    let project = Project::new(&[
        (
            "main.fl",
            "use facade.Select; typealias Byte = Select(true); fn run() -> Byte { 255 }",
        ),
        ("facade.fl", "pub use helper.Select;"),
        (
            "helper.fl",
            "pub comptime fn Select(flag: true'type) -> .Type'builtin { if flag { .u8'builtin } else { .u16'builtin } }",
        ),
    ]);
    assert_eq!(project.run(), Value::UnsignedInteger(255));
}

#[test]
fn imported_normal_form_heads_preserve_declaration_identity() {
    let project = Project::new(&[
        (
            "main.fl",
            "use facade.Identity; typealias A = Identity<.u8'builtin>; fn same(value: A) -> helper.Identity<.u8'builtin> { value }",
        ),
        ("facade.fl", "pub use helper.Identity;"),
        (
            "helper.fl",
            "pub pure comptime fn Identity<T: .Type'builtin> -> .Type'builtin { T }",
        ),
    ]);
    let package = project.compile().unwrap();
    assert_eq!(
        package
            .functions
            .iter()
            .filter(|f| f.phase == nir::program::FunctionPhase::NormalForm)
            .count(),
        1
    );
}

#[test]
fn module_constants_preserve_import_visibility_and_comptime_dependencies() {
    let project = Project::new(&[
        (
            "main.fl",
            "use config.WIDTH; use config.BYTE; const NEXT = WIDTH + 1; typealias Bits = .BitVec'builtin<NEXT>; fn keep(value: BYTE) -> BYTE { value } comptime fn run() -> .Integer'builtin { NEXT }",
        ),
        (
            "config.fl",
            "pub const WIDTH: .Integer'builtin = 8; pub const BYTE: .Type'builtin = .u8'builtin; const PRIVATE = 9;",
        ),
    ]);
    let package = project.compile().unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    assert_eq!(
        Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke(entry, vec![])
            .unwrap(),
        Value::Integer(comptime_runtime::Integer::from_u128(9))
    );
    let private = Project::new(&[
        ("main.fl", "use config.PRIVATE; const VALUE = PRIVATE;"),
        ("config.fl", "private const PRIVATE = 9;"),
    ]);
    assert!(private.compile().is_err());
}

#[test]
fn generic_alias_parameters_are_private_to_the_definition_through_reexports() {
    let project = Project::new(&[
        (
            "main.fl",
            "use facade.Pair; typealias T = 'a''type; fn run() -> (Pair<true'type>, T) { ((true, false), 'x') }",
        ),
        ("facade.fl", "pub use shapes.Pair;"),
        ("shapes.fl", "pub typealias Pair<T> = (T, T);"),
    ]);
    assert_eq!(
        project.run(),
        Value::Tuple(vec![
            Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]),
            Value::Char('x')
        ])
    );
    let missing = Project::new(&[
        ("main.fl", "use shapes.Pair; fn bad(value: T) {}"),
        ("shapes.fl", "pub typealias Pair<T> = (T, T);"),
    ]);
    assert!(missing.compile().unwrap_err().contains("HIR lowering"));
}

#[test]
fn library_frontend_errors_retain_diagnostic_locations_and_causes() {
    for (source, stage, location) in [
        ("fn run( {", "parsing failed", "main.fl:1:"),
        (
            "fn run() { not_declared; }",
            "HIR lowering failed",
            "main.fl:1:",
        ),
        (
            "-- 注释😀\nfn run() { \0 }",
            "lexing failed",
            "main.fl:2:12:",
        ),
    ] {
        let error = Project::new(&[("main.fl", source)]).compile().unwrap_err();
        assert!(error.starts_with(stage), "{error}");
        assert!(error.contains(location), "{error}");
        assert!(error.contains("error[E"), "{error}");
        if source.contains("not_declared") {
            assert!(
                error.contains("unresolved")
                    || error.contains("Unresolved")
                    || error.contains("not_declared"),
                "{error}"
            );
        }
        assert!(
            !error.contains('\u{1b}'),
            "returned errors must not contain terminal escapes"
        );
    }
    let error = Project::new(&[
        ("main.fl", "fn run() { child.broken(); }"),
        ("child.fl", "pub fn broken() { not_declared; }"),
    ])
    .compile()
    .unwrap_err();
    assert!(error.contains("child.fl:1:"), "{error}");
}
