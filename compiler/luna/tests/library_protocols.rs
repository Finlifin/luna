//! Compile repository library declarations, rather than fixture copies.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;
use elaboration::arithmetic::Operator;
use support::compile_with_source_bindings;

const PRIMITIVES: &str = include_str!("../../../library/builtin/primitives.fl");
const OPERATORS: &str = include_str!("../../../library/builtin/ops.fl");
const META: &str = include_str!("../../../library/meta/lib.fl");
const TUPLE: &str = include_str!("../../../library/builtin/tuple.fl");
const RESOURCES: &str = include_str!("../../../library/builtin/resources.fl");
const TUPLE_RESOURCES: &str = include_str!("../../../library/builtin/tuple_resources.fl");

#[test]
fn repository_tuple_once_native_preserves_affine_fields() {
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
        "mod meta {{ {META} }} mod resources {{ {RESOURCES} }} mod tuples {{ {TUPLE} }} mod tuple_resources {{ {TUPLE_RESOURCES} }}\n{}",
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
                "input::meta::List",
                "--tuple-type",
                "input::tuples::Tuple",
                "--copy-protocol",
                "input::resources::Copy",
                "--drop-protocol",
                "input::resources::Drop",
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
    let source = format!(
        "mod meta {{ {META} }}\nmod resources {{ {} }}\n{TUPLE}\nmod tuple_resources {{ {TUPLE_RESOURCES} }}\n
        fn forward(value:T)->T where T:-resources.Once {{value}}
        fn run()->bool {{let (a,(b,),())=forward((true,(false,),()));a and not b}}",
        include_str!("../../../library/builtin/resources.fl"),
    );
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::meta::List"),
        tuple: Some("input::Tuple"),
        copy: Some("input::resources::Copy"),
        drop: Some("input::resources::Drop"),
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
                    "where T:-resources.Once",
                    &format!("where T:-resources.{capability}")
                ),
                &Default::default(),
                selectors
            )
            .is_err()
        );
    }
    let constrained = format!(
        "mod meta {{ {META} }} mod resources {{ {RESOURCES} }} {TUPLE}
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
        include_str!("../../../library/builtin/conversion.fl"),
        include_str!("../../../library/builtin/resources.fl"),
        include_str!("../../../development/examples/host-library-conversion.fl"),
    );
    let compile = |source: &str| {
        compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                copy: Some("input::resources::Copy"),
                drop: Some("input::resources::Drop"),
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
    use nir::{Primitive, Term};
    let source = format!(
        "{PRIMITIVES}\nmod meta {{ {META} }}\n{TUPLE}\n
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
            meta_list: Some("input::meta::List"),
            tuple: Some("input::Tuple"),
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
        let selector = format!("input::{name}");
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
    selectors.arithmetic[Operator::Add as usize] = Some("input::Add");
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
        let selector = format!("input::{name}");
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
    for binding in [None, Some("input::Not")] {
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
        include_str!("../../../library/meta/lib.fl"),
        include_str!("../../../library/builtin/tuple.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::meta::List"),
            tuple: Some("input::Tuple"),
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
        index: Some("input::Index"),
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
        include_str!("../../../library/builtin/resources.fl")
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
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
            let check5 = low.as(Ord).cmp(high) matches .Less; assumes check5;
            let check6 = high.as(Ord).cmp(low) matches .Greater; assumes check6;
            let check7 = low.as(Ord).cmp(low) matches .Equal; assumes check7;
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
        include_str!("../../../library/builtin/comparison.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
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
    let before = 'a'.as(Ord).cmp(character) matches .Less; assumes before;
    let after = character.as(Ord).cmp('a') matches .Greater; assumes after;
    let same_character = character.as(Ord).cmp('中') matches .Equal; assumes same_character;
    let le = character.as(Ord).le('中'); assumes le;
    let ge = character.as(Ord).ge('a'); assumes ge;
    let gt = character.as(Ord).gt('a'); assumes gt;
    let lt = 'a'.as(Ord).lt(character); assumes lt;
}}
"#,
        include_str!("../../../library/builtin/comparison.fl"),
        include_str!("../../../library/builtin/resources.fl")
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
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
        fn keep(value:Watch)->?Watch {{.Some(value)}}
        fn fallback(present:bool)->bool {{
            var payload_done=false;
            var fallback_done=false;
            do {{
                let value:?Watch=if present {{.Some(Watch {{seen:payload_done.ref}})}} else {{.None}};
                let selected=value.unwrap_or(Watch {{seen:fallback_done.ref}});
                assumes not payload_done;
                assumes fallback_done == present;
                let observed=selected.seen.*;
                assumes not observed;
            }};
            payload_done == present and fallback_done
        }}
        fn map_case(present:bool)->bool {{
            var done=false;
            let value:?Watch=if present {{.Some(Watch {{seen:done.ref}})}} else {{.None}};
            let mapped=value.map(inspect);
            done == present and mapped.unwrap_or(true)
        }}
        fn chain_case()->bool {{
            var done=false;
            do {{
                let value:?Watch=.Some(Watch {{seen:done.ref}});
                let chained=value.and_then(keep);
                assumes not done;
                let seen=chained.is_some();
                assumes seen;
            }};
            done
        }}
        fn run() {{
            let a=fallback(true);let b=fallback(false);
            let c=map_case(true);let d=map_case(false);let e=chain_case();
            assumes a and b and c and d and e;
        }}",
        include_str!("../../../library/builtin/option.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            optional: Some("input::Option"),
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
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
        fn reroute(value:Watch)->Result<bool,Watch> {{.Err(value)}}
        fn mapped_case(ok:bool)->bool {{
            var good_done=false;var bad_done=false;
            let value:Result<Watch,Watch> = if ok {{.Ok(Watch {{seen:good_done.ref}})}} else {{.Err(Watch {{seen:bad_done.ref}})}};
            let mapped=value.map(inspect);
            assumes good_done == ok;
            assumes not bad_done;
            let converted=mapped.map_err(inspect);
            assumes bad_done == not ok;
            converted match {{.Ok(flag)=>flag,.Err(flag)=>flag}}
        }}
        fn chained_case(ok:bool)->bool {{
            var done=false;
            let value:Result<Watch,Watch> = if ok {{.Ok(Watch {{seen:done.ref}})}} else {{.Err(Watch {{seen:done.ref}})}};
            let chained=value.and_then(reroute);
            assumes not done;
            let bad=chained.is_err();
            bad and done
        }}
        fn run() {{
            let a=mapped_case(true);let b=mapped_case(false);
            let c=chained_case(true);let d=chained_case(false);
            assumes a and b and c and d;
        }}",
        include_str!("../../../library/builtin/result.fl"),
        include_str!("../../../library/builtin/resources.fl"),
    );
    let package = compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input::Copy"),
            drop: Some("input::Drop"),
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
fn repository_value_combinators_own_callback_environments_on_both_branches() {
    for (input_type, method, returned, observe, active, inactive) in [
        (
            "Option<bool>",
            "map",
            "value",
            "mapped.unwrap_or(true)",
            ".Some(true)",
            ".None",
        ),
        (
            "Option<bool>",
            "and_then",
            "Option<bool>.Some(value)",
            "mapped.unwrap_or(true)",
            ".Some(true)",
            ".None",
        ),
        (
            "Result<bool,bool>",
            "map",
            "value",
            "mapped match {.Ok(v)=>v,.Err(v)=>v}",
            ".Ok(true)",
            ".Err(true)",
        ),
        (
            "Result<bool,bool>",
            "map_err",
            "value",
            "mapped match {.Ok(v)=>v,.Err(v)=>v}",
            ".Err(true)",
            ".Ok(true)",
        ),
        (
            "Result<bool,bool>",
            "and_then",
            "Result<bool,bool>.Ok(value)",
            "mapped match {.Ok(v)=>v,.Err(v)=>v}",
            ".Ok(true)",
            ".Err(true)",
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
                    use optional.Option;use fallible.Result;
                    struct State {{called:*bool,retired:*bool}}
                    impl Drop for State {{fn drop(*self) {{assumes not self.retired.*.*;self.retired.*.* = true;}}}}
                    fn run() {{
                        var called=false;var retired=false;
                        let state=State {{called:called.ref,retired:retired.ref}};
                        let callback=|value:bool| {{
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
                    include_str!("../../../library/builtin/option.fl"),
                    include_str!("../../../library/builtin/result.fl"),
                    include_str!("../../../library/builtin/resources.fl"));
                let package = compile_with_source_bindings(
                    &source,
                    &Default::default(),
                    luna::protocols::type_constructors::Selectors {
                        optional: Some("input::optional::Option"),
                        copy: Some("input::Copy"),
                        drop: Some("input::Drop"),
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
