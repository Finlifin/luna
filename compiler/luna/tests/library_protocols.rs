//! Compile repository library declarations, rather than fixture copies.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use elaboration::arithmetic::Operator;
use support::compile_with_source_bindings;

const PRIMITIVES: &str = include_str!("../../../library/builtin/primitives.fl");
const OPERATORS: &str = include_str!("../../../library/builtin/protocols/ops.fl");
const TUPLE: &str = include_str!("../../../library/builtin/types/tuple.fl");
const RESOURCES: &str = include_str!("../../../library/builtin/protocols/resources.fl");

#[test]
fn repository_list_pattern_protocol_retains_observation_write_checks() {
    let fixture = include_str!("../../../development/examples/sysroot-list-patterns.fl").replace(
        "        self.value.*",
        "        self.value.* = true; self.value.*",
    );
    let source = format!(
        "{PRIMITIVES} mod resources {{ {RESOURCES} }} use resources.*;
        mod patterns {{ {} }} use patterns.*;
        {fixture}
        fn run()->bool {{exact(Repeated {{length:2,value:false}})}}",
        support::with_attrs(include_str!(
            "../../../library/builtin/protocols/patterns.fl"
        )),
    );
    let target = luna::host::Toolchain::default()
        .target_layout()
        .unwrap()
        .semantic_target()
        .unwrap();
    let package = support::compile_with_source_bindings_for_target(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.resources.Copy"),
            drop: Some("input.resources.Drop"),
            list_pattern: Some("input.patterns.ListPatternView"),
            ..Default::default()
        },
        Some(target),
    )
    .unwrap();
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap_err();
    assert!(error.contains("guard pointer observation"), "{error}");
}

#[test]
fn repository_meta_lists_carry_homogeneous_comptime_values() {
    let meta = include!("support/meta_source.rs");
    let source = format!(
        "{PRIMITIVES} mod meta {{ {meta} }}
        comptime fn flags() -> meta.List<bool> {{ [true, false] }}
        comptime fn choose(xs: meta.List<bool>, index: Integer) -> bool {{ xs[index] }}
        comptime fn nested() -> meta.List<meta.List<bool>> {{ [flags(), [false, true]] }}
        comptime fn numbers() -> meta.List<Integer> {{ [17, 42] }}
        struct Row {{ flag: bool }}
        comptime fn rows() -> meta.List<Row> {{ [Row {{flag:true}}, Row {{flag:false}}] }}
        comptime fn empty() -> meta.List<bool> {{ [] }}
        comptime fn early() -> bool {{ let xs: meta.List<bool> = [false, do {{return true;}}]; false }}
        comptime fn check() -> bool {{
            let xs = flags();
            let layers = nested();
            choose(xs, 0) and not choose(xs, 1) and layers[1][1] and numbers()[1] == 42
                and rows()[0].flag and not rows()[1].flag and early()
        }}
        const saved = flags();
        const checked = check();
        comptime fn run() -> bool {{ checked and choose(saved, 0) }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.meta.List"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    let result = vm.invoke(support::id(&package, "flags"), vec![]).unwrap();
    assert!(
        matches!(vm.invoke(support::id(&package, "empty"), vec![]).unwrap(),
        Value::List { values, .. } if values.is_empty())
    );
    let Value::List { ty, values } = &result else {
        panic!("{result}")
    };
    assert_eq!(values, &[Value::Bool(true), Value::Bool(false)]);
    assert!(matches!(ty, nir::Term::NormalForm { .. }));
    assert!(
        result
            .to_string()
            .starts_with("(comptime-list (type (normal-form")
    );
    for bad in [
        source.replace("[true, false]", "[true, 42]"),
        source.replace("choose(xs, 0)", "choose(xs, 2)"),
        source.replace("choose(xs, 0)", "choose(xs, -1)"),
        source.replace(
            "comptime fn run() -> bool { checked and choose(saved, 0) }",
            "fn run() -> meta.List<bool> { [true] }",
        ),
    ] {
        assert!(compile_with_source_bindings(&bad, &Default::default(), selectors).is_err());
    }
}

#[test]
fn repository_meta_list_clause_inputs_preserve_value_and_type_identity() {
    let meta = include!("support/meta_source.rs");
    let source = format!(
        "{PRIMITIVES} mod meta {{ {meta} }}
        comptime fn first()->bool where flags:meta.List<bool> {{flags[0]}}
        comptime fn flags()->meta.List<bool> {{[true,false]}}
        pure comptime fn Packet<bits:meta.List<bool>>->Type {{bool}}
        comptime fn FirstPacket()->Type {{Packet<[true,false]>}}
        comptime fn SecondPacket()->Type {{Packet<[false,true]>}}
        const passed = first<[true,false]>() and not first<[false,true]>() and first<flags()>();
        comptime fn run()->bool {{passed}}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.meta.List"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let first = luna::invocation::invoke(&package, "FirstPacket", 100_000).unwrap();
    let second = luna::invocation::invoke(&package, "SecondPacket", 100_000).unwrap();
    assert_ne!(first, second);
    let Value::Type(nir::Term::NormalForm { arguments, .. }) = first else {
        panic!("expected opaque head")
    };
    assert!(
        matches!(arguments.as_slice(), [nir::Term::NominalValue {ty, variant: None, fields}]
        if matches!(ty.as_ref(), nir::Term::NormalForm {..}) && fields == &[nir::Term::Bool(true), nir::Term::Bool(false)])
    );
    for bad in [
        source.replace("first<[true,false]>()", "first<[]>()"),
        source.replace("first<[true,false]>()", "first<[true,42]>()"),
    ] {
        assert!(compile_with_source_bindings(&bad, &Default::default(), selectors).is_err());
    }
}

#[test]
fn repository_meta_list_clauses_substitute_element_domains_and_defaults() {
    let meta = include!("support/meta_source.rs");
    let source = format!(
        "{PRIMITIVES} mod meta {{ {meta} }}
        comptime fn first() -> T where T, values: meta.List<T> {{ values[0] }}
        comptime fn defaulted() -> bool where .values: meta.List<bool> = [true, false] {{ values[0] }}
        comptime fn SelectedType() -> Type {{ first<Type, [bool]>() }}
        comptime fn run() -> bool {{
            let selected = first<bool, [true, false]>();
            selected and not first<bool, [false, true]>()
                and defaulted<>() and not defaulted<.values = [false]>()
                and first<bool, [true, false]>()
        }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.meta.List"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "SelectedType", 100_000).unwrap(),
        Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
    );
    for bad in [
        source.replace("first<Type, [bool]>()", "first<Type, [true]>()"),
        source.replace("first<bool, [true, false]>()", "first<bool, [bool]>()"),
    ] {
        assert!(compile_with_source_bindings(&bad, &Default::default(), selectors).is_err());
    }
    let empty = source.replace("defaulted<>()", "defaulted<.values = []>()");
    let package = compile_with_source_bindings(&empty, &Default::default(), selectors).unwrap();
    assert!(luna::invocation::invoke(&package, "run", 100_000).is_err());
}

#[test]
fn repository_meta_value_list_variadics_preserve_order_and_identity() {
    let meta = include!("support/meta_source.rs");
    let source = format!(
        "{PRIMITIVES} mod meta {{ {meta} }}
        pure comptime fn Packet<...values: meta.List<bool>> -> Type {{ bool }}
        pure comptime fn DefaultPacket<.tag: bool = true, ...values: meta.List<bool>> -> Type {{ bool }}
        comptime fn flags() -> meta.List<bool> {{ [false, true] }}
        comptime fn empty_flags() -> meta.List<bool> {{ [] }}
        comptime fn first_flag() -> bool {{ true }}
        comptime fn last_flag() -> bool {{ false }}
        pure comptime fn Generic<T, ...values: meta.List<T>> -> Type {{ T }}
        pure comptime fn Nested<...values: meta.List<meta.List<bool>>> -> Type {{ bool }}
        comptime fn GenericDirect() -> Type {{ Generic<bool, true, false, true> }}
        comptime fn GenericSpread() -> Type {{ Generic<bool, true, ...flags()> }}
        comptime fn NestedDirect() -> Type {{ Nested<[true], [false, true]> }}
        comptime fn NestedSpread() -> Type {{ Nested<[true], ...[[false, true]]> }}
        comptime fn Ordered() -> Type {{ Packet<first_flag(), ...empty_flags(), ...flags(), last_flag()> }}
        comptime fn Early() -> Type {{ Packet<first_flag(), ...do {{return bool;}}, last_flag()> }}
        comptime fn PlannedOrdered() -> Type {{ DefaultPacket<first_flag(), .tag = last_flag(), ...empty_flags(), ...flags()> }}
        comptime fn PlannedEarly() -> Type {{ DefaultPacket<first_flag(), ...do {{return bool;}}, last_flag()> }}
        comptime fn Empty() -> Type {{ Packet<> }}
        comptime fn Direct() -> Type {{ Packet<true, false, true> }}
        comptime fn Spread() -> Type {{ Packet<true, ...flags()> }}
        comptime fn Whole() -> Type {{ Packet<.values = [true, false, true]> }}
        comptime fn Reverse() -> Type {{ Packet<false, true, true> }}
        comptime fn DefaultEmpty() -> Type {{ DefaultPacket<> }}
        comptime fn DefaultDirect() -> Type {{ DefaultPacket<true, false, true> }}
        comptime fn DefaultSpread() -> Type {{ DefaultPacket<true, ...flags()> }}
        comptime fn DefaultWhole() -> Type {{ DefaultPacket<.values = [true, false, true]> }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.meta.List"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    let invoke = |name| luna::invocation::invoke(&package, name, 100_000).unwrap();
    assert_eq!(invoke("GenericDirect"), invoke("GenericSpread"));
    assert_eq!(invoke("NestedDirect"), invoke("NestedSpread"));
    for (name, calls) in [
        (
            "Ordered",
            vec!["Ordered", "first_flag", "empty_flags", "flags", "last_flag"],
        ),
        ("Early", vec!["Early", "first_flag"]),
        (
            "PlannedOrdered",
            vec![
                "PlannedOrdered",
                "first_flag",
                "last_flag",
                "empty_flags",
                "flags",
            ],
        ),
        ("PlannedEarly", vec!["PlannedEarly", "first_flag"]),
    ] {
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let value = vm.invoke(support::id(&package, name), vec![]).unwrap();
        assert_eq!(support::calls(&package, &vm), calls);
        if name.ends_with("Early") {
            assert_eq!(
                value,
                Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
            );
        }
    }
    assert!(luna::invocation::invoke(&package, "Ordered", 1).is_err());
    assert_eq!(invoke("Direct"), invoke("Spread"));
    assert_eq!(invoke("Direct"), invoke("Whole"));
    assert_ne!(invoke("Direct"), invoke("Reverse"));
    assert_eq!(invoke("DefaultDirect"), invoke("DefaultSpread"));
    assert_eq!(invoke("DefaultDirect"), invoke("DefaultWhole"));
    for (name, slot) in [("Empty", 0), ("DefaultEmpty", 1)] {
        let Value::Type(nir::Term::NormalForm { arguments, .. }) = invoke(name) else {
            panic!("opaque head required")
        };
        assert!(
            matches!(&arguments[slot], nir::Term::NominalValue { variant: None, fields, .. } if fields.is_empty())
        );
    }
    for bad in [
        source.replace("Packet<true, false, true>", "Packet<true, bool>"),
        source.replace("...flags()", "...(false, true)"),
        source.replace(
            "Packet<true, false, true>",
            "Packet<true, .values = [false]>",
        ),
    ] {
        assert!(compile_with_source_bindings(&bad, &Default::default(), selectors).is_err());
    }
}

#[test]
fn repository_meta_dependent_collectors_share_defaulted_argument_plans() {
    let meta = include!("support/meta_source.rs");
    let source = format!(
        "{PRIMITIVES} mod meta {{ {meta} }}
        pure comptime fn Packet<T, .tag: bool = true, ...values: meta.List<T>> -> Type {{ T }}
        comptime fn flags() -> meta.List<bool> {{ [false, true] }}
        comptime fn Direct() -> Type {{ Packet<bool, true, false, true> }}
        comptime fn Spread() -> Type {{ Packet<bool, true, ...flags()> }}
        comptime fn Whole() -> Type {{ Packet<bool, .values = [true, false, true]> }}
        comptime fn Empty() -> Type {{ Packet<bool> }}
        comptime fn EmptyWhole() -> Type {{ Packet<bool, .values = []> }}
        comptime fn SelectedType() -> Type {{ bool }}
        comptime fn ByteType() -> Type {{ .u8'builtin }}
        comptime fn TypeType() -> Type {{ Type }}
        comptime fn BoolListType() -> Type {{ meta.List<bool> }}
        comptime fn first() -> bool {{ true }}
        comptime fn Late() -> Type {{ Packet<.values = [first(), false, true], SelectedType()> }}
        comptime fn LateEmpty() -> Type {{ Packet<.values = [], SelectedType()> }}
        comptime fn NumericDirect() -> Type {{ Packet<.u8'builtin, 1, 255> }}
        comptime fn NumericLate() -> Type {{ Packet<.values = [1, 255], ByteType()> }}
        comptime fn TypesDirect() -> Type {{ Packet<Type, bool, .u8'builtin> }}
        comptime fn TypesLate() -> Type {{ Packet<.values = [bool, .u8'builtin], TypeType()> }}
        comptime fn NestedDirect() -> Type {{ Packet<meta.List<bool>, [true], [false]> }}
        comptime fn NestedLate() -> Type {{ Packet<.values = [[true], [false]], BoolListType()> }}
        comptime fn WrongElement() -> Type {{ Packet<.values = [true], ByteType()> }}
        comptime fn Overflow() -> Type {{ Packet<.values = [256], ByteType()> }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.meta.List"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    let invoke = |name| luna::invocation::invoke(&package, name, 100_000).unwrap();
    assert_eq!(invoke("Direct"), invoke("Spread"));
    assert_eq!(invoke("Direct"), invoke("Whole"));
    assert_eq!(invoke("Empty"), invoke("EmptyWhole"));
    assert_eq!(invoke("Direct"), invoke("Late"));
    assert_eq!(invoke("Empty"), invoke("LateEmpty"));
    assert_eq!(invoke("NumericDirect"), invoke("NumericLate"));
    assert_eq!(invoke("TypesDirect"), invoke("TypesLate"));
    assert_eq!(invoke("NestedDirect"), invoke("NestedLate"));
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "Late"), vec![]).unwrap(),
        invoke("Direct")
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["Late", "first", "SelectedType"]
    );
    for name in ["WrongElement", "Overflow"] {
        assert!(vm.invoke(support::id(&package, name), vec![]).is_err());
        assert_eq!(
            vm.invoke(support::id(&package, "Direct"), vec![]).unwrap(),
            invoke("Direct")
        );
    }
}

#[test]
fn optional_combinators_preserve_affine_payloads_and_conditional_callbacks() {
    let source = format!(
        "mod resources {{ {RESOURCES} }} mod optional {{ {} }}\n{}",
        include_str!("../../../library/builtin/types/option.fl"),
        include_str!("../../../development/examples/host-library-optional-combinators.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.resources.Copy"),
            drop: Some("input.resources.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true),
    );
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .unwrap()
        .declaration;
    let prepared =
        luna::host::prepare_with_protocols(&package, "verify", 1_000_000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1_000_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn native_wrapper_once_preserves_affine_payloads_without_granting_copy() {
    let source = format!(
        "mod resources {{ {RESOURCES} }}\n{}",
        include_str!("../../../development/examples/host-library-native-once.fl")
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.resources.Copy"),
            drop: Some("input.resources.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );
    for wrapper in ["?Token", "!bool Token"] {
        let source = format!(
            "{RESOURCES}
            struct Token {{}}
            fn forward(value:T)->T where T:-Once {{value}}
            fn run() {{let original:{wrapper}=Token {{}};
                let first=forward(original);let second=forward(original);
            }}"
        );
        let package = support::compile(&source);
        let failure =
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
                .unwrap_err();
        assert!(failure.contains("unavailable value"), "{failure}");
    }
    for bound in ["Clone", "Copy"] {
        let source = format!(
            "{RESOURCES}
            struct Token {{}}
            fn require(value:T) where T:-{bound} {{}}
            fn run() {{let value:?Token=Token {{}};require(value);}}"
        );
        assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
    }
}

#[test]
fn native_wrapper_copy_requires_every_payload_and_preserves_pointer_lifetime() {
    let resources = format!(
        "mod resources {{ {RESOURCES} }}\nmod native_resources {{ {} }}\nuse resources.*;",
        include_str!("../../../library/builtin/protocols/native_resources.fl")
    );
    let selectors = || luna::protocols::type_constructors::Selectors {
        copy: Some("input.resources.Copy"),
        drop: Some("input.resources.Drop"),
        ..Default::default()
    };
    for (wrapper, initial) in [
        ("?Token", "Token {}"),
        ("!bool Token", "Token {}"),
        ("!Token bool", "true"),
        ("!?Token bool", "true"),
    ] {
        let source = format!(
            "{resources}
            struct Token {{}}
            fn require(value:T) where T:-Copy {{}}
            fn run() {{let value:{wrapper}={initial};require(value);}}"
        );
        let error =
            compile_with_source_bindings(&source, &Default::default(), selectors()).unwrap_err();
        assert!(
            error.contains("trait bound") || error.contains("capability"),
            "{error}"
        );
    }
    let source = format!(
        "{resources}
        fn escape()->?*bool {{
            var local=true;
            let original:?*bool=local.ref;
            original.as(Clone).clone()
        }}
        fn run() {{let dangling=escape();}}"
    );
    let package = compile_with_source_bindings(&source, &Default::default(), selectors()).unwrap();
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap_err();
    assert!(
        error.contains("local") || error.contains("escape"),
        "{error}"
    );
}

#[test]
fn repository_tuple_once_native_preserves_affine_fields() {
    let meta = include!("support/meta_source.rs");
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    let directory = std::env::temp_dir().join(format!(
        "luna-library-tuple-once-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let source = format!(
        "mod meta {{ {meta} }} mod resources {{ {RESOURCES} }} mod tuples {{ {TUPLE} }}\n{}",
        include_str!("../../../development/examples/host-library-tuple-once.fl"),
    );
    fs::write(directory.join("source.fl"), source).unwrap();
    for level in ["0", "3"] {
        let executable = format!("program-{level}");
        let output = Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&directory)
            .args([
                "source.fl",
                "--build",
                "run",
                "--output",
                &executable,
                "--opt-level",
                level,
                "--meta-list-type",
                "input.meta.List",
                "--tuple-type",
                "input.tuples.Tuple",
                "--copy-protocol",
                "input.resources.Copy",
                "--drop-protocol",
                "input.resources.Drop",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            directory.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(directory.join(executable)).output().unwrap();
        assert!(
            output.status.success(),
            "{}: {output:?}",
            directory.display()
        );
        assert_eq!(output.stdout, b"(bool true)\n", "{}", directory.display());
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn tuple_implementation_infers_one_complete_type_list_input() {
    let meta = include!("support/meta_source.rs");
    let source = format!(
        "mod meta {{ {meta} }}\nmod resources {{ {} }}\n{TUPLE}\ntrait TupleMarker {{}} impl TupleMarker for Tuple<.Es = Es> where Es:meta.List<.Type'builtin> {{}}\n
        fn forward(value:T)->T where T:-TupleMarker {{value}}
        fn run()->bool {{let (a,(b,),())=forward((true,(false,),()));a and not b}}",
        include_str!("../../../library/builtin/protocols/resources.fl"),
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input.meta.List"),
        tuple: Some("input.Tuple"),
        copy: Some("input.resources.Copy"),
        drop: Some("input.resources.Drop"),
        ..Default::default()
    };
    let metadata = compile_with_source_bindings(
        &source.replace("forward((true,(false,),()))", "(true,(false,),())"),
        &Default::default(),
        selectors,
    )
    .unwrap();
    let tuple_head = support::id(&metadata, "Tuple");
    let tuple = |fields| nir::Term::NormalForm {
        head: tuple_head,
        sort: nir::Sort::Type,
        arguments: vec![nir::Term::TypeListValue(fields)],
    };
    let subject = tuple(vec![
        nir::Term::Primitive(nir::Primitive::Bool),
        tuple(vec![nir::Term::Primitive(nir::Primitive::Bool)]),
        tuple(vec![]),
    ]);
    let implementation = metadata.implementations.iter().find(|implementation| matches!(&implementation.subject, nir::Term::NormalForm {head,..} if *head==tuple_head)).unwrap();
    let head = elaboration::implementation::ImplementationHead {
        declaration: implementation.declaration,
        type_parameters: implementation.type_parameters.clone(),
        parameter_types: implementation.parameter_types.clone(),
        subject: implementation.subject.clone(),
        interface: implementation.interface,
        interface_arguments: implementation.interface_arguments.clone(),
        requirements: implementation.requirements.clone(),
    };
    let context = elaboration::ptc::Context::from_package(&metadata).unwrap();
    let budget = elaboration::ptc::Budget::default();
    let matched = head
        .match_types_with_layouts(
            &subject,
            &[],
            &context,
            budget,
            &Default::default(),
            Some(&metadata),
        )
        .unwrap();
    head.verify_head_match_with_layouts(
        &matched,
        &subject,
        &[],
        &context,
        budget,
        &Default::default(),
        Some(&metadata),
    )
    .unwrap();
    for invalid in 0..3 {
        let mut changed = metadata.clone();
        let list = changed
            .normal_forms
            .iter()
            .position(|head| head.role == nir::normal_forms::Role::TypeList)
            .unwrap();
        match invalid {
            0 => changed.normal_forms[list].role = nir::normal_forms::Role::Opaque,
            1 => {
                changed.normal_forms.remove(list);
            }
            _ => changed
                .normal_forms
                .push(changed.normal_forms[list].clone()),
        }
        assert!(
            head.match_types_with_layouts(
                &subject,
                &[],
                &context,
                budget,
                &Default::default(),
                Some(&changed)
            )
            .is_err()
        );
        assert!(
            head.verify_head_match_with_layouts(
                &matched,
                &subject,
                &[],
                &context,
                budget,
                &Default::default(),
                Some(&changed)
            )
            .is_err()
        );
    }
    let mut index = elaboration::implementation::Index::default();
    index.set_normal_form_metadata(metadata.normal_forms.clone());
    index.insert(head.clone(), budget).unwrap();
    let selected = index
        .resolve_types(head.interface.unwrap(), &[], &subject, &context, budget)
        .unwrap();
    assert!(matches!(
        selected.selection,
        elaboration::implementation::Selection::Selected(_)
    ));
    index.set_normal_form_metadata(metadata.normal_forms.clone());
    assert!(index.type_resolution_is_current(&selected));
    index.set_normal_form_metadata(vec![]);
    assert!(!index.type_resolution_is_current(&selected));
    let missing = index
        .resolve_types(head.interface.unwrap(), &[], &subject, &context, budget)
        .unwrap();
    assert!(!matches!(
        missing.selection,
        elaboration::implementation::Selection::Selected(_)
    ));
    index.set_normal_form_metadata(metadata.normal_forms.clone());
    assert!(!index.type_resolution_is_current(&selected));
    // Updating unrelated declaration layouts must retain representation roles.
    index.set_nominal_layouts(
        vec![nir::program::Struct {
            declaration: nir::DeclarationId {
                package: 900,
                local: 1,
            },
            name: "Unrelated".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            fields: vec![],
        }],
        vec![],
        vec![],
    );
    let recovered = index
        .resolve_types(head.interface.unwrap(), &[], &subject, &context, budget)
        .unwrap();
    assert!(matches!(
        recovered.selection,
        elaboration::implementation::Selection::Selected(_)
    ));
    assert!(index.to_string().contains("(normal-form-head"));
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
    for capability in ["Copy", "Clone"] {
        assert!(
            compile_with_source_bindings(
                &source.replace(
                    "where T:-TupleMarker",
                    &format!("where T:-resources.{capability}")
                ),
                &Default::default(),
                selectors
            )
            .is_err()
        );
    }
    let constrained = format!(
        "mod meta {{ {meta} }} mod resources {{ {RESOURCES} }} {TUPLE}
        trait SingleBool {{}}
        impl SingleBool for Tuple<...Es> where Es:meta.List<.Type'builtin>, requires Tuple<...Es> == Tuple<bool> {{}}
        fn only(value:T)->T where T:-SingleBool {{value}}
        fn run()->bool {{let (flag,)=only((true,));flag}}"
    );
    let package =
        compile_with_source_bindings(&constrained, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        constrained.replace("only((true,))", "only((true,false))"),
        constrained.replace("only((true,))", "only(())"),
    ] {
        assert!(compile_with_source_bindings(&invalid, &Default::default(), selectors).is_err());
    }
}

#[test]
fn repository_conversion_protocols_select_full_interfaces_and_transfer_ownership() {
    let source = format!(
        "mod conversion {{ {} }}\nmod resources {{ {} }}\n{}",
        include_str!("../../../library/builtin/protocols/conversion.fl"),
        include_str!("../../../library/builtin/protocols/resources.fl"),
        include_str!("../../../development/examples/host-library-conversion.fl"),
    );
    let compile = |source: &str| {
        compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input.resources.Copy"),
                drop: Some("input.resources.Drop"),
                ..Default::default()
            },
        )
    };
    let package = compile(&source).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000,)
            .unwrap(),
        Value::Bool(true)
    );
    // Sharing the same subject and trait head does not grant another interface.
    let wrong_target = source.replace(
        "convert<Flag, .u8'builtin>(Flag {value: false})",
        "convert<Flag, .u16'builtin>(Flag {value: false})",
    );
    assert!(compile(&wrong_target).is_err());
    // The protocol consumes its receiver; conversion cannot clone an affine field.
    let reused = source.replace(
        "    assumes not retired;",
        "    let second = convert<Packet, Token>(packet); retire(second);\n    assumes not retired;",
    );
    let package = compile(&reused).unwrap();
    let error = luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
        .unwrap_err();
    assert!(error.contains("unavailable"), "{error}");
}

#[test]
fn repository_tuple_and_meta_definitions_preserve_identity_and_execute() {
    let meta = include!("support/meta_source.rs");
    use nir::{Primitive, Term};
    let source = format!(
        "{PRIMITIVES}\nmod meta {{ {meta} }}\n{TUPLE}\n
        typealias Pair<T> = Tuple<T, bool>;
        comptime fn PairType(T: Type) -> Type {{ Pair<T> }}
        comptime fn Elements() -> meta.List<Type> {{ [u8, bool] }}
        comptime fn PacketType() -> Type {{ Tuple<...Elements()> }}
        comptime fn EmptyType() -> Type {{ Tuple<> }}
        fn pair(value: T) -> Pair<T> where T {{ (value, true) }}
        fn run() -> bool {{
            let packet: Tuple<Pair<u8>, Tuple<bool>, Tuple<>> =
                (pair<u8>(9), (false,), ());
            let ((number, flag), (other,), ()) = packet;
            number == 9 and flag and not other
        }}"
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let fields = vec![
        Term::Primitive(Primitive::U8),
        Term::Primitive(Primitive::Bool),
    ];
    assert_eq!(
        luna::invocation::invoke(&package, "Elements", 100_000).unwrap(),
        Value::TypeList(fields.clone())
    );
    for (entry, fields) in [("PacketType", fields), ("EmptyType", vec![])] {
        let Value::Type(Term::NormalForm {
            head, arguments, ..
        }) = luna::invocation::invoke(&package, entry, 100_000).unwrap()
        else {
            panic!("{entry}: Tuple lost its normal-form identity");
        };
        assert_eq!(head, support::id(&package, "Tuple"));
        assert_eq!(arguments, vec![Term::TypeListValue(fields)]);
    }
}

#[test]
fn repository_operator_protocols_support_distinct_rhs_and_output() {
    for (operator, name, symbol) in [
        (Operator::Add, "Add", "+"),
        (Operator::Subtract, "Sub", "-"),
        (Operator::Multiply, "Mul", "*"),
        (Operator::Divide, "Div", "/"),
        (Operator::Remainder, "Rem", "%"),
    ] {
        let source = format!(
            "{PRIMITIVES}\n{OPERATORS}\n
            struct Flag {{enabled:bool}}
            impl {name}<.Rhs=bool> for Flag {{assoc Output:Type=bool;
                fn {}(self,rhs:bool)->bool {{self.enabled and rhs}}}}
            fn run()->bool {{(Flag {{enabled:true}} {symbol} true)
                and not (Flag {{enabled:true}} {symbol} false)}}",
            operator.method()
        );
        let mut selectors = luna::protocols::type_constructors::Selectors::default();
        let selector = format!("input.{name}");
        selectors.arithmetic[operator as usize] = Some(&selector);
        let package =
            compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn repository_operator_defaults_and_unary_outputs_are_executable() {
    let source = format!(
        "{PRIMITIVES}\n{OPERATORS}\n
        struct Flag {{enabled:bool}}
        impl Add for Flag {{assoc Output:Type=Flag;
            fn add(self,rhs:Flag)->Flag {{Flag {{enabled:self.enabled or rhs.enabled}}}}}}
        fn run()->bool {{(Flag {{enabled:false}} + Flag {{enabled:true}}).enabled}}"
    );
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[Operator::Add as usize] = Some("input.Add");
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for (operator, name, symbol) in [
        (Operator::Negate, "Neg", "-"),
        (Operator::Invert, "Not", "not "),
    ] {
        let source = format!(
            "{PRIMITIVES}\n{OPERATORS}\n
            struct Flag {{enabled:bool}}
            impl {name} for Flag {{assoc Output:Type=bool;
                fn {}(self)->bool {{self.enabled}}}}
            fn run()->bool {{ {symbol}Flag {{enabled:true}} }}",
            operator.method()
        );
        let mut selectors = luna::protocols::type_constructors::Selectors::default();
        let selector = format!("input.{name}");
        selectors.arithmetic[operator as usize] = Some(&selector);
        let package =
            compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn repository_library_sources_lex_and_parse_without_fixture_rewrites() {
    use rustc_span::source_map::FilePathMapping;
    use rustc_span::{FileName, SourceMap};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let mut pending = vec![root];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "fl") {
                files.push(path);
            }
        }
    }
    files.sort();
    assert!(!files.is_empty());
    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        let map = SourceMap::new(FilePathMapping::empty());
        let file =
            map.new_source_file(FileName::Custom(path.display().to_string()), source.clone());
        let (tokens, symbols, errors) = lex::lex(&source, file.start_pos);
        assert!(errors.is_empty(), "{}: {errors:?}", path.display());
        let mut parser = parse::parser::Parser::new(&map, tokens, symbols, file.start_pos);
        let root = parser
            .try_file_scope()
            .unwrap_or_else(|error| panic!("{}: {error:?}", path.display()));
        assert_ne!(root, 0, "{}: missing file AST", path.display());
        assert_eq!(
            parser.peek_next_token().kind,
            lex::TokenKind::Eof,
            "{}: unparsed suffix",
            path.display()
        );
    }
}

#[test]
fn repository_unary_binding_checks_nested_execution_and_materialization_limits() {
    for binding in [None, Some("input.Not")] {
        for depth in [80, 140] {
            let mut selectors = luna::protocols::type_constructors::Selectors::default();
            selectors.arithmetic[Operator::Invert as usize] = binding;
            let source = format!(
                "{PRIMITIVES}\n{OPERATORS}\nfn run()->bool {{ {}true }} fn unused()->bool {{ {}true }} fn recovery()->bool {{true}}",
                "not ".repeat(depth),
                "not ".repeat(140)
            );
            let package =
                compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
            let result = luna::execution::interpret(&package, "run", 100_000);
            if depth == 80 {
                assert_eq!(result.unwrap(), Value::Bool(true));
            } else {
                let error = result.unwrap_err();
                assert!(
                    error.contains("dispatch materialization budget exhausted"),
                    "{error}"
                );
            }
            assert_eq!(
                luna::execution::interpret(&package, "recovery", 100_000).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn repository_tuple_head_preserves_type_identity_and_value_layout() {
    let source = format!(
        "{PRIMITIVES}\nmod meta {{ {} }}\n{}\n
        trait Pair where T {{assoc Item:Type;}}
        struct Flag {{enabled:bool}}
        impl Pair<bool> for Flag {{assoc Item:Type=Tuple<bool,bool>;}}
        fn pair()->Flag.as(Pair<bool>).Item {{(true,false)}}
        fn run()->bool {{
            let (a,b)=pair();
            let empty:Tuple<> = ();
            let single:Tuple<bool> = (a,);
            let (value,)=single;
            value and not b
        }}
        comptime fn PairType()->Type {{Tuple<bool,bool>}}",
        include!("support/meta_source.rs"),
        include_str!("../../../library/builtin/types/tuple.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let Value::Type(nir::Term::NormalForm {
        head, arguments, ..
    }) = luna::invocation::invoke(&package, "PairType", 100_000).unwrap()
    else {
        panic!("library Tuple identity must retain its declared head")
    };
    assert_eq!(head, support::id(&package, "Tuple"));
    assert_eq!(
        arguments,
        vec![nir::Term::TypeListValue(vec![
            nir::Term::Primitive(
                nir::Primitive::Bool
            );
            2
        ])]
    );
}

#[test]
fn repository_index_protocol_preserves_pointer_output_and_evaluation_order() {
    let source = format!(
        "{PRIMITIVES}\n{OPERATORS}\n
        struct Slot {{pointer:*u16}}
        impl Index<bool> for Slot {{
            assoc Output:Type=u16;
            fn index(self,idx:bool)->*u16 {{self.pointer}}
        }}
        impl IndexMut<bool> for Slot {{
            fn index_mut(*self,idx:bool)->*u16 {{self.*.pointer}}
        }}
        fn target(pointer:*u16)->Slot {{Slot {{pointer:pointer}}}}
        fn key()->bool {{true}}
        fn replacement()->u16 {{9}}
        fn run()->u16 {{var word:u16=0;target(word.ref)[key()].* = replacement();word}}
        fn explicit_mut()->u16 {{
            var word:u16=0;
            var slot=Slot {{pointer:word.ref}};
            slot.ref.as(IndexMut<bool>).index_mut(true).* = 17;
            word
        }}"
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        index: Some("input.Index"),
        ..Default::default()
    };
    let package = compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::UnsignedInteger(9)
    );
    assert_eq!(
        support::calls(&package, &vm),
        vec!["run", "target", "key", "index", "replacement"]
    );
    assert_eq!(
        luna::execution::interpret(&package, "explicit_mut", 100_000).unwrap(),
        Value::UnsignedInteger(17)
    );
    let wrong_output = source.replace(
        "fn index_mut(*self,idx:bool)->*u16 {self.*.pointer}",
        "fn index_mut(*self,idx:bool)->*u16 {true}",
    );
    assert_ne!(wrong_output, source);
    assert!(compile_with_source_bindings(&wrong_output, &Default::default(), selectors).is_err());
    let invalid = format!("{source}\nfn bad()->u16 {{var word:u16=0;target(word.ref)[true]}}");
    assert!(compile_with_source_bindings(&invalid, &Default::default(), selectors).is_err());
}

#[test]
fn repository_parent_associated_signatures_are_generic_and_order_independent() {
    let parent = "impl Index<bool> for Slot<T> where T {
        assoc Output:Type=T;
        fn index(self,idx:bool)->*T {self.pointer}
    }";
    let child = "impl IndexMut<bool> for Slot<T> where T {
        fn index_mut(*self,idx:bool)->*T {self.*.pointer}
    }";
    for child_first in [false, true] {
        let implementations = if child_first {
            format!("{child}\n{parent}")
        } else {
            format!("{parent}\n{child}")
        };
        let source = format!(
            "{PRIMITIVES}\n{OPERATORS}\n
            struct Slot where T {{pointer:*T}}
            {implementations}
            fn run()->bool {{
                var word=false;
                var slot=Slot<bool> {{pointer:word.ref}};
                slot.ref.as(IndexMut<bool>).index_mut(true).* = true;
                word
            }}"
        );
        let package =
            compile_with_source_bindings(&source, &Default::default(), Default::default()).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        for invalid_parent in [
            "",
            "impl Index<bool> for Slot<T> where T {
                assoc Output:Type=T;
                fn index(self,idx:bool)->bool {true}
            }",
        ] {
            let invalid = source.replace(parent, invalid_parent);
            assert!(
                compile_with_source_bindings(&invalid, &Default::default(), Default::default(),)
                    .is_err(),
                "a missing or invalid parent cannot justify the child signature"
            );
        }
        for wrong in ["*u16", "*Identity<T>"] {
            let wrong_child = format!(
                "impl IndexMut<bool> for Slot<T> where T {{
                fn index_mut(*self,idx:bool)->{wrong} {{while true {{}}}}
            }}"
            );
            let invalid = source.replace(child, &wrong_child);
            let invalid = format!("{invalid}\npure comptime fn Identity<T>->Type {{T}}");
            let error =
                compile_with_source_bindings(&invalid, &Default::default(), Default::default())
                    .unwrap_err();
            assert!(
                error.contains("does not match its trait signature"),
                "{error}"
            );
        }
    }
}

#[test]
fn repository_resource_protocols_clone_scalars_and_preserve_pointer_targets() {
    let mut checks = String::new();
    for ty in [
        "u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32", "i64", "i128", "f32", "f64",
    ] {
        let literal = if ty.starts_with('f') { "7.0" } else { "7" };
        checks.push_str(&format!("let original_{ty}:.{ty}'builtin={literal}; let copy_{ty}=original_{ty}.as(Clone).clone(); assumes copy_{ty} == original_{ty};\n"));
    }
    let source = format!(
        "{}\n{}
        fn run() {{
            {checks}
            let tag:.Symbol'builtin=.ready;let cloned_tag=tag.as(Clone).clone();
            assumes tag == .ready and cloned_tag == tag;
            let empty=do {{}};empty.as(Clone).clone();
            let flag=true;let copy=flag.as(Clone).clone();
            let integer:.u32'builtin=41;let duplicate=integer.as(Clone).clone();
            let float:.f64'builtin=17.0;let same=float.as(Clone).clone();
            var target=false;let pointer=target.ref;
            let alias=pointer.as(Clone).clone();alias.* = true;
            assumes flag and copy and integer == duplicate and same == float and target;
        }}",
        PRIMITIVES,
        include_str!("../../../library/builtin/protocols/resources.fl")
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100000).unwrap(),
        Value::Unit
    );
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .unwrap()
        .declaration;
    let prepared = luna::host::prepare_with_protocols(&package, "run", 100000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 100000).unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn repository_comparison_protocols_execute_integer_boundaries_and_defaults() {
    let mut checks = String::new();
    let mut entries = Vec::new();
    let mut combined = String::new();
    for (ty, low, high) in [
        ("u8", "0", "255"),
        ("u16", "0", "65535"),
        ("u32", "0", "4294967295"),
        ("u64", "0", "18446744073709551615"),
        ("u128", "0", "340282366920938463463374607431768211455"),
        ("i8", "-128", "127"),
        ("i16", "-32768", "32767"),
        ("i32", "-2147483648", "2147483647"),
        ("i64", "-9223372036854775808", "9223372036854775807"),
        (
            "i128",
            "-170141183460469231731687303715884105728",
            "170141183460469231731687303715884105727",
        ),
        ("bool", "false", "true"),
    ] {
        let entry = format!("check_{ty}");
        entries.push(entry.clone());
        let ty = if ty == "bool" {
            "bool".into()
        } else {
            format!(".{ty}'builtin")
        };
        let body = format!(
            "{{let low:{ty}={low};let high:{ty}={high};
            let check1 = low.as(Eq).eq(low) and low.as(Eq).ne(high); assumes check1;
            let check2 = low.as(Ord).lt(high) and not high.as(Ord).lt(low); assumes check2;
            let check3 = low.as(Ord).le(low) and high.as(Ord).ge(high); assumes check3;
            let check4 = high.as(Ord).gt(low) and not low.as(Ord).gt(high); assumes check4;
            let check5 = low.as(Ord).cmp(high) matches .less; assumes check5;
            let check6 = high.as(Ord).cmp(low) matches .greater; assumes check6;
            let check7 = low.as(Ord).cmp(low) matches .equal; assumes check7;
        }}"
        );
        checks.push_str(&format!("fn {entry}() {body}"));
        combined.push_str(&body);
    }
    // Keep the entire batch in one function: cleanup lookup must not rescan
    // every unrelated site at each temporary-expression boundary.
    checks.push_str(&format!("fn check_all() {{{combined}}}"));
    entries.push("check_all".into());
    let source = format!(
        "{}\n{}\n{checks}",
        support::with_attrs(include_str!(
            "../../../library/builtin/numeric/comparison.fl"
        )),
        include_str!("../../../library/builtin/protocols/resources.fl")
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .unwrap()
        .declaration;
    for entry in entries {
        let budget = if entry == "check_all" {
            500_000
        } else {
            100_000
        };
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, &entry, "Copy", "Drop", budget)
                .unwrap(),
            Value::Unit
        );
        let prepared =
            luna::host::prepare_with_protocols(&package, &entry, budget, copy, drop).unwrap();
        let ir = luna::host::llvm_ir_prepared(&prepared, budget).unwrap();
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}

#[test]
fn repository_comparison_protocols_compare_characters_and_symbols() {
    let source = format!(
        r#"{}
{}
fn symbol_equal(left: .Symbol'builtin, right: .Symbol'builtin) -> bool {{
    left.as(Eq).eq(right)
}}
fn run() {{
    let symbol_same = symbol_equal(.ready, .ready); assumes symbol_same;
    let symbol_other = symbol_equal(.ready, .waiting); assumes not symbol_other;
    let symbol_ne = .ready.as(Eq).ne(.waiting); assumes symbol_ne;
    let character = '中';
    let supplementary = character.as(Ord).lt('😀'); assumes supplementary;
    let zero = '\0'.as(Ord).lt('a'); assumes zero;
    let equal = character.as(Eq).eq('中'); assumes equal;
    let different = character.as(Eq).ne('文'); assumes different;
    let before = 'a'.as(Ord).cmp(character) matches .less; assumes before;
    let after = character.as(Ord).cmp('a') matches .greater; assumes after;
    let same_character = character.as(Ord).cmp('中') matches .equal; assumes same_character;
    let le = character.as(Ord).le('中'); assumes le;
    let ge = character.as(Ord).ge('a'); assumes ge;
    let gt = character.as(Ord).gt('a'); assumes gt;
    let lt = 'a'.as(Ord).lt(character); assumes lt;
}}
"#,
        support::with_attrs(include_str!(
            "../../../library/builtin/numeric/comparison.fl"
        )),
        include_str!("../../../library/builtin/protocols/resources.fl")
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
            .unwrap(),
        Value::Unit
    );
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .unwrap()
        .declaration;
    let prepared =
        luna::host::prepare_with_protocols(&package, "run", 100_000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn repository_option_operations_preserve_affine_payloads_and_cleanup() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool}}
        impl Drop for Watch {{fn drop(*self) {{assumes not self.seen.*.*; self.seen.*.* = true;}}}}
        fn inspect(value:Watch)->bool {{not value.seen.*}}
        fn keep(value:Watch)->?Watch {{value}}
        fn fallback(present:bool)->bool {{
            var payload_done=false;
            var fallback_done=false;
            do {{
                let value:?Watch=if present {{Watch {{seen:payload_done.ref}}}} else {{null}};
                let selected=value.as(OptionalOps<Watch>).unwrap_or(Watch {{seen:fallback_done.ref}});
                assumes not payload_done;
                assumes fallback_done == present;
                let observed=selected.seen.*;
                assumes not observed;
            }};
            payload_done == present and fallback_done
        }}
        fn map_case(present:bool)->bool {{
            var done=false;
            let value:?Watch=if present {{Watch {{seen:done.ref}}}} else {{null}};
            let mapped=value.as(OptionalOps<Watch>).map(inspect);
            done == present and mapped.as(OptionalOps<bool>).unwrap_or(true)
        }}
        fn chain_case()->bool {{
            var done=false;
            do {{
                let value:?Watch=Watch {{seen:done.ref}};
                let chained=value.as(OptionalOps<Watch>).and_then(keep);
                assumes not done;
                let seen=chained.as(OptionalOps<Watch>).is_some();
                assumes seen;
            }};
            done
        }}
        fn lazy_case(present:bool)->bool {{
            var payload_done=false;var fallback_done=false;var called=false;
            do {{
                let captured=Watch {{seen:fallback_done.ref}};
                let called_pointer=called.ref;
                let callback=|| {{
                    assumes not called_pointer.*;
                    called_pointer.* = true;
                    captured
                }};
                let input:?Watch=if present {{Watch {{seen:payload_done.ref}}}} else {{null}};
                let selected=input.as(OptionalOps<Watch>).unwrap_or_else(callback);
                assumes called == not present;
                assumes fallback_done == present;
                assumes not payload_done;
                let observed=selected.seen.*;
                assumes not observed;
            }};
            payload_done == present and fallback_done
        }}
        fn run() {{
            let lazy_present=lazy_case(true);let lazy_absent=lazy_case(false);
            assumes lazy_present and lazy_absent;
            let a=fallback(true);let b=fallback(false);
            let c=map_case(true);let d=map_case(false);let e=chain_case();
            assumes a and b and c and d and e;
        }}",
        include_str!("../../../library/builtin/types/option.fl"),
        include_str!("../../../library/builtin/protocols/resources.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100000).unwrap(),
        Value::Unit
    );
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .unwrap()
        .declaration;
    let prepared = luna::host::prepare_with_protocols(&package, "run", 100000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 100000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn repository_result_operations_preserve_both_affine_branches() {
    let source = format!(
        "{}\n{}\n
        struct Watch {{seen:*bool}}
        impl Drop for Watch {{fn drop(*self) {{assumes not self.seen.*.*; self.seen.*.* = true;}}}}
        fn inspect(value:Watch)->bool {{not value.seen.*}}
        fn reroute(value:Watch)->!Watch bool {{error value}}
        fn mapped_case(ok:bool)->bool {{
            var good_done=false;var bad_done=false;
            let value:!Watch Watch = if ok {{Watch {{seen:good_done.ref}}}} else {{error Watch {{seen:bad_done.ref}}}};
            let mapped=value.as(ErrorResultOps<Watch,Watch>).map(inspect);
            assumes good_done == ok;
            assumes not bad_done;
            let converted=mapped.as(ErrorResultOps<bool,Watch>).map_err(inspect);
            assumes bad_done == not ok;
            converted match {{flag! =>flag,error flag=>flag}}
        }}
        fn chained_case(ok:bool)->bool {{
            var done=false;
            let value:!Watch Watch = if ok {{Watch {{seen:done.ref}}}} else {{error Watch {{seen:done.ref}}}};
            let chained=value.as(ErrorResultOps<Watch,Watch>).and_then(reroute);
            assumes not done;
            let bad=chained.as(ErrorResultOps<bool,Watch>).is_err();
            bad and done
        }}
        struct Observed {{seen:*bool,done:*bool}}
        impl Drop for Observed {{fn drop(*self) {{
            assumes self.seen.*.* and not self.done.*.*;
            self.done.*.* = true;
        }}}}
        fn observe(value:*Observed) {{
            assumes not value.seen.*.* and not value.done.*.*;
            value.seen.*.* = true;
        }}
        fn observed_case(ok:bool)->bool {{
            var seen=false;var done=false;
            let value:!Observed Observed = if ok {{Observed {{seen:seen.ref,done:done.ref}}}}
                else {{error Observed {{seen:seen.ref,done:done.ref}}}};
            let first=value.as(ErrorResultOps<Observed,Observed>).inspect(observe);
            assumes seen == ok and not done;
            let second=first.as(ErrorResultOps<Observed,Observed>).inspect_err(observe);
            assumes seen and not done;
            let retained=second.as(ErrorResultOps<Observed,Observed>).is_ok();
            retained == ok and done
        }}
        fn run() {{
            let a=mapped_case(true);let b=mapped_case(false);
            let c=chained_case(true);let d=chained_case(false);
            let e=observed_case(true);let f=observed_case(false);
            assumes a and b and c and d and e and f;
        }}",
        include_str!("../../../library/builtin/types/result.fl"),
        include_str!("../../../library/builtin/protocols/resources.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        package.enums.is_empty(),
        "native error operations must not introduce a wrapper enum"
    );
    let reroute = package
        .functions
        .iter()
        .find(|function| function.name == "reroute")
        .unwrap();
    assert!(matches!(reroute.body.ty, nir::Term::ErrorResult { .. }));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Unit
    );
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .unwrap()
        .declaration;
    let prepared =
        luna::host::prepare_with_protocols(&package, "run", 1_000_000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1_000_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn repository_value_combinators_own_callback_environments_on_both_branches() {
    for (input_type, method, parameters, returned, observe, active, inactive) in [
        (
            "!bool bool",
            "as(ErrorResultOps<bool,bool>).inspect",
            "value:*bool",
            "assumes value.*;",
            "mapped match {v! =>v,error v=>v}",
            "true",
            "error true",
        ),
        (
            "!bool bool",
            "as(ErrorResultOps<bool,bool>).inspect_err",
            "value:*bool",
            "assumes value.*;",
            "mapped match {v! =>v,error v=>v}",
            "error true",
            "true",
        ),
        (
            "?bool",
            "as(OptionalOps<bool>).unwrap_or_else",
            "",
            "true",
            "mapped",
            "null",
            "true",
        ),
        (
            "?bool",
            "as(OptionalOps<bool>).map",
            "value:bool",
            "value",
            "mapped.as(OptionalOps<bool>).unwrap_or(true)",
            "true",
            "null",
        ),
        (
            "?bool",
            "as(OptionalOps<bool>).and_then",
            "value:bool",
            "do {let result:?bool=value; result}",
            "mapped.as(OptionalOps<bool>).unwrap_or(true)",
            "true",
            "null",
        ),
        (
            "!bool bool",
            "as(ErrorResultOps<bool,bool>).map",
            "value:bool",
            "value",
            "mapped match {v! =>v,error v=>v}",
            "true",
            "error true",
        ),
        (
            "!bool bool",
            "as(ErrorResultOps<bool,bool>).map_err",
            "value:bool",
            "value",
            "mapped match {v! =>v,error v=>v}",
            "error true",
            "true",
        ),
        (
            "!bool bool",
            "as(ErrorResultOps<bool,bool>).and_then",
            "value:bool",
            "do {let result:!bool bool=value;result}",
            "mapped match {v! =>v,error v=>v}",
            "true",
            "error true",
        ),
    ] {
        for invoked in [false, true] {
            for consume_capture in [false, true] {
                let input = if invoked { active } else { inactive };
                let consume = if consume_capture {
                    "let moved = state;"
                } else {
                    ""
                };
                let source=format!("mod optional {{
{}
}} mod fallible {{
{}
}} {}\n
                    use optional.OptionalOps;use fallible.ErrorResultOps;
                    struct State {{called:*bool,retired:*bool}}
                    impl Drop for State {{fn drop(*self) {{assumes not self.retired.*.*;self.retired.*.* = true;}}}}
                    fn run() {{
                        var called=false;var retired=false;
                        let state=State {{called:called.ref,retired:retired.ref}};
                        let callback=|{parameters}| {{
                            assumes not state.called.*;
                            state.called.* = true;
                            {consume}
                            {returned}
                        }};
                        let input:{input_type} = {input};
                        let mapped=input.{method}(callback);
                        let observed={observe};
                        assumes observed and called == {invoked} and retired;
                    }}",
                    include_str!("../../../library/builtin/types/option.fl"),
                    include_str!("../../../library/builtin/types/result.fl"),
                    include_str!("../../../library/builtin/protocols/resources.fl"));
                let package = compile_with_source_bindings(
                    &source,
                    &Default::default(),
                    luna::protocols::type_constructors::Selectors {
                        copy: Some("input.Copy"),
                        drop: Some("input.Drop"),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    luna::execution::interpret_with_protocols(
                        &package, "run", "Copy", "Drop", 100000
                    )
                    .unwrap(),
                    Value::Unit
                );
                let copy = package
                    .traits
                    .iter()
                    .find(|t| t.name == "Copy")
                    .unwrap()
                    .declaration;
                let drop = package
                    .traits
                    .iter()
                    .find(|t| t.name == "Drop")
                    .unwrap()
                    .declaration;
                let prepared =
                    luna::host::prepare_with_protocols(&package, "run", 100000, copy, drop)
                        .unwrap();
                let ir = luna::host::llvm_ir_prepared(&prepared, 100000).unwrap();
                for optimization in [
                    luna::host::OptimizationLevel::O0,
                    luna::host::OptimizationLevel::O3,
                ] {
                    luna::host::Toolchain {
                        optimization,
                        ..Default::default()
                    }
                    .execute_test(&ir, std::time::Duration::from_secs(5))
                    .unwrap();
                }
            }
        }
    }
}

#[test]
fn repository_error_operations_preserve_rows_when_callbacks_add_errors() {
    let source = format!(
        "{}\n{}\n
        enum InputErr {{Missing}}
        enum ComputeErr {{Rejected}}
        fn input(present:bool)->!InputErr bool {{if present {{true}} else {{error InputErr.Missing}}}}
        fn step(value:bool)->!ComputeErr bool {{if value {{error ComputeErr.Rejected}} else {{false}}}}
        fn mapped(present:bool)->![InputErr,ComputeErr] bool {{
            input(present).as(ErrorResultOps<bool,InputErr>).map(step)
        }}
        fn retried()->![InputErr,ComputeErr] bool {{mapped(false)! {{
            InputErr.Missing=>mapped(true),ComputeErr.Rejected=>false,
        }}}}
        fn run()->bool {{
            (mapped(false) match {{error InputErr.Missing=>true,error ComputeErr.Rejected=>false,_! =>false}}) and
            (mapped(true) match {{error ComputeErr.Rejected=>true,error InputErr.Missing=>false,_! =>false}}) and
            (retried() match {{error ComputeErr.Rejected=>true,error InputErr.Missing=>false,_! =>false}})
        }}
        fn verify() {{let passed=run();assumes passed;}}",
        include_str!("../../../library/builtin/types/result.fl"),
        include_str!("../../../library/builtin/protocols/resources.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        package.enums.len(),
        2,
        "only source error labels are nominal enums"
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
            .unwrap(),
        Value::Bool(true)
    );
    let copy = package
        .traits
        .iter()
        .find(|item| item.name == "Copy")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|item| item.name == "Drop")
        .unwrap()
        .declaration;
    let prepared =
        luna::host::prepare_with_protocols(&package, "verify", 100_000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}
