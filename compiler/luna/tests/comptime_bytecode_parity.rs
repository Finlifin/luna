//! Parity tests comparing bytecode compilation/execution against the tree interpreter.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::normal_forms::{Head, Kind, Parameter as NfParameter, Role};
use nir::program::{
    Expr, ExprKind as K, Function, FunctionDeclaration, FunctionPhase as Phase, LocalId, Package,
    SourceRange,
};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};

fn assert_sysroot_all_paths_bool(source: &str, entry: &str, expected: bool) {
    let mut package = sysroot::compile(source).unwrap();
    let entry_id = sysroot::source_function(&package, entry);

    let mut source_byte = None;
    let mut source_tree = None;
    sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, false, |vm| {
        let before = vm.bytecode_invocations();
        source_byte = Some(vm.invoke(entry_id, vec![]).unwrap());
        assert!(
            vm.bytecode_invocations() > before,
            "source bytecode path must invoke bytecode for {entry}"
        );
    });
    sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, true, |vm| {
        assert_eq!(vm.bytecode_invocations(), 0);
        source_tree = Some(vm.invoke(entry_id, vec![]).unwrap());
        assert_eq!(vm.bytecode_invocations(), 0);
    });
    assert_eq!(source_byte, Some(Value::Bool(expected)));
    assert_eq!(source_tree, Some(Value::Bool(expected)));

    let roots = [iir::InstanceKey {
        declaration: entry_id,
        arguments: vec![].into(),
    }];
    let budget = elaboration::ptc::Budget {
        steps: 1_000_000,
        ..Default::default()
    };
    package.dispatch_instances =
        elaboration::materialization::dispatch_instances_for_invocation(&package, &roots, budget)
            .unwrap();
    let graph =
        elaboration::materialization::instantiate(&package, &roots, Default::default(), budget)
            .unwrap();
    let mut iir_byte =
        Interpreter::from_iir_for_comptime(&graph, sysroot::COMPTIME_BUDGET).unwrap();
    let mut iir_tree =
        Interpreter::from_iir_for_comptime_tree(&graph, sysroot::COMPTIME_BUDGET).unwrap();
    let before = iir_byte.bytecode_invocations();
    let iir_byte_val = iir_byte.invoke_instance(&graph.roots[0], vec![]).unwrap();
    assert!(
        iir_byte.bytecode_invocations() > before,
        "IIR bytecode path must invoke bytecode for {entry}"
    );
    let iir_tree_val = iir_tree.invoke_instance(&graph.roots[0], vec![]).unwrap();
    assert_eq!(iir_tree.bytecode_invocations(), 0);
    assert_eq!(iir_byte_val, Value::Bool(expected));
    assert_eq!(iir_tree_val, Value::Bool(expected));
}

#[test]
fn template_equal_parity_between_tree_and_bytecode() {
    let package = sysroot::compile(
        "
        typealias Callback = fn(comptime T: Type, value: T) -> bool;
        fn yes(comptime T: Type, value: T) -> bool { true }
        fn no(comptime T: Type, value: T) -> bool { false }
        fn choose(flag: bool) -> Callback { if flag { yes } else { no } }
        fn trial(flag: bool) -> bool {
            let callback: Callback = choose(flag);
            let invoke = || callback(bool, true);
            invoke()
        }
        fn run() -> bool {
            trial(true) and not trial(false)
        }
        ",
    )
    .unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    assert!(
        graph.to_string().contains("(template-equal "),
        "expected materialized TemplateEqual in graph"
    );

    let mut template_equal_count = 0usize;
    for function in &graph.functions {
        iir::visit::expression(&function.function.body, &mut 100_000, |expr, _| {
            if matches!(expr.kind, K::TemplateEqual(..)) {
                template_equal_count += 1;
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(template_equal_count > 0);

    let mut iir_tree = Interpreter::from_iir(&graph, sysroot::COMPTIME_BUDGET)
        .unwrap()
        .with_reference_execution();
    let mut iir_byte = Interpreter::from_iir(&graph, sysroot::COMPTIME_BUDGET).unwrap();
    assert_eq!(
        iir_tree.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        iir_byte.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn general_patterns_parity_between_tree_and_bytecode() {
    let source = "
        enum Choice {
            none,
            single(Integer),
            pair(Integer, bool),
        }
        struct Boxed {
            tag: Integer,
            choice: Choice,
        }
        comptime fn classify(input: Boxed, extra: ?Integer) -> Integer {
            let base = input match {
                Boxed { tag: 1, choice: .pair(n, true) } if n > 0 => n + 10,
                Boxed { tag: 1, choice: .pair(n, _) } => n,
                Boxed { tag: _, choice: .single(n) } => n * 2,
                _ => 0,
            };
            let bonus = extra match {
                v? => v,
                null => 5,
            };
            base + bonus
        }
        comptime fn run() -> bool {
            let a = classify(Boxed { tag: 1, choice: .pair(7, true) }, 3);
            let b = classify(Boxed { tag: 1, choice: .pair(-2, true) }, null);
            let c = classify(Boxed { tag: 2, choice: .single(4) }, 1);
            let d = classify(Boxed { tag: 9, choice: .none }, null);
            a == 20 and b == 3 and c == 9 and d == 5
        }
    ";
    assert_sysroot_all_paths_bool(source, "run", true);
}

#[test]
fn dynamic_logical_and_or_parity_including_bitvec_eager_and_bool_short_circuit() {
    let source = "
        comptime fn run() -> bool {
            var calls = 0;
            var f_false = false;
            var f_true = true;

            -- Short-circuit AND: false LHS must not evaluate RHS.
            let r1 = do { calls = calls + 1; f_false } and do { calls = calls + 100; f_true };
            -- Short-circuit OR: true LHS must not evaluate RHS.
            let r2 = do { calls = calls + 1; f_true } or do { calls = calls + 1000; f_false };
            -- Non-short-circuit branches must evaluate RHS.
            let r3 = do { calls = calls + 1; f_true } and do { calls = calls + 10; f_false };
            let r4 = do { calls = calls + 1; f_false } or do { calls = calls + 20; f_true };

            let bool_ok = (not r1) and r2 and (not r3) and r4 and calls == 34;

            -- Dynamic BitVec AND / OR evaluate both sides eagerly.
            var bv_calls = 0;
            let mask_a: BitVec<4> = 0b1100;
            let mask_b: BitVec<4> = 0b1010;
            let expected_and: BitVec<4> = 0b1000;
            let expected_or: BitVec<4> = 0b1110;
            let bv_and = (do { bv_calls = bv_calls + 1; mask_a }
                and do { bv_calls = bv_calls + 10; mask_b }) == expected_and;
            let bv_or = (do { bv_calls = bv_calls + 100; mask_a }
                or do { bv_calls = bv_calls + 1000; mask_b }) == expected_or;

            -- 20-deep right-associated logical chain parity
            let deep = f_true and (f_true and (f_true and (f_true and (f_true
                and (f_true and (f_true and (f_true and (f_true and (f_true
                and (f_true and (f_true and (f_true and (f_true and (f_true
                and (f_true and (f_true and (f_true and (f_true and f_true))))))))))))))))));

            bool_ok and bv_and and bv_or and bv_calls == 1111 and deep
        }
    ";
    assert_sysroot_all_paths_bool(source, "run", true);
}

#[test]
fn nested_staged_normal_inputs_depth_gt_zero_parity() {
    let source = "
        pure comptime fn Inner<...items: meta.List<Type>> -> Type {
            Inner<...items>
        }
        pure comptime fn Outer<...groups: meta.List<Type>> -> Type {
            Outer<...groups>
        }
        comptime fn run() -> bool {
            let a = Outer<Inner<u8, bool>, Inner<Type>>;
            let b = Outer<Inner<u8, bool>, Inner<Type>>;
            a == b and b == Outer<Inner<u8, bool>, Inner<Type>>
        }
    ";
    assert_sysroot_all_paths_bool(source, "run", true);
}

#[test]
fn higher_order_non_type_normal_form_head_executes_identically_in_tree_and_bytecode() {
    let helper_decl = DeclarationId {
        package: 1,
        local: 9,
    };
    let nf_decl = DeclarationId {
        package: 1,
        local: 10,
    };
    let input_decl = DeclarationId {
        package: 1,
        local: 11,
    };
    let entry_decl = DeclarationId {
        package: 1,
        local: 12,
    };
    let helper_binding = LocalId { owner: 9, index: 0 };
    let input_binding = LocalId {
        owner: 10,
        index: 0,
    };
    let inner_callable = Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(Term::Primitive(Primitive::Type)),
    }));
    let helper_fn = Function {
        interface: FunctionDeclaration {
            source: SourceRange { start: 1, end: 5 },
            declaration: helper_decl,
            name: "id_type".to_string(),
            phase: Phase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            signature: inner_callable.clone(),
            parameters: vec![helper_binding],
            env_parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
        },
        body: Expr {
            source: SourceRange { start: 2, end: 4 },
            ty: Term::Primitive(Primitive::Type),
            kind: K::Local(helper_binding),
        },
        runtime_deliveries: Default::default(),
    };
    let outer_sig = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(inner_callable.clone()),
    };
    let body_sig = Term::Function {
        parameters: vec![],
        result: Box::new(inner_callable.clone()),
    };
    let nf_callable_ty = Term::NormalFormFunction(Box::new(outer_sig), None);
    let nf_head = Head {
        collector: None,
        role: Role::Opaque,
        declaration: nf_decl,
        parameters: vec![NfParameter {
            declaration: input_decl,
            binding: input_binding,
            kind: Kind::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        requirements: vec![],
    };
    let nf_fn = Function {
        interface: FunctionDeclaration {
            source: SourceRange { start: 6, end: 15 },
            declaration: nf_decl,
            name: "ho_nf".to_string(),
            phase: Phase::NormalForm,
            inline: false,
            pure: true,
            is_unsafe: false,
            signature: body_sig,
            parameters: vec![],
            env_parameters: vec![],
            type_parameters: vec![input_decl],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
        },
        body: Expr {
            source: SourceRange { start: 7, end: 14 },
            ty: inner_callable,
            kind: K::Function(helper_decl),
        },
        runtime_deliveries: Default::default(),
    };
    let entry_fn = Function {
        interface: FunctionDeclaration {
            source: SourceRange { start: 20, end: 50 },
            declaration: entry_decl,
            name: "entry".to_string(),
            phase: Phase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(nf_callable_ty.clone()),
            },
            parameters: vec![],
            env_parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
        },
        body: Expr {
            source: SourceRange { start: 25, end: 45 },
            ty: nf_callable_ty,
            kind: K::Function(nf_decl),
        },
        runtime_deliveries: Default::default(),
    };
    let package = Package {
        functions: vec![helper_fn, nf_fn, entry_fn],
        normal_forms: vec![nf_head],
        ..Default::default()
    };
    nir::phases::Inspection::new(&package)
        .check_package(&mut 10_000)
        .unwrap();
    let dispatched = elaboration::materialization::dispatch_instances_for_invocation(
        &package,
        &[iir::InstanceKey {
            declaration: entry_decl,
            arguments: vec![].into(),
        }],
        Default::default(),
    )
    .unwrap();
    assert!(dispatched.is_empty());

    let mut tree_vm = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
    let mut byte_vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let before = byte_vm.bytecode_invocations();
    let tree_val = tree_vm.invoke(entry_decl, vec![]).unwrap();
    let byte_val = byte_vm.invoke(entry_decl, vec![]).unwrap();
    assert_eq!(tree_vm.bytecode_invocations(), 0);
    assert_eq!(byte_vm.bytecode_invocations(), before + 1);
    assert_eq!(byte_val, tree_val);
    assert_eq!(byte_val, Value::Function(nf_decl));
}

#[test]
fn opaque_normal_form_receiver_reflection_method_resolves_and_rejects_in_vm() {
    let source = r#"
        pure comptime fn Identity<T> -> Type { T }
        comptime fn schema(...properties: T) -> T where T { properties }
        const COUNT_VAL: u32 = 1;
        const SAMPLE_RECORD = schema { flag: true, count: COUNT_VAL };
        typealias SampleRecord = SAMPLE_RECORD'type;

        comptime fn good_record() -> meta.List<Type> {
            SampleRecord.record_field_types()
        }

        comptime fn opaque_receiver_record() -> meta.List<Type> {
            Identity<SampleRecord>.record_field_types()
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let good = sysroot::source_function(&package, "good_record");
    let bad = sysroot::source_function(&package, "opaque_receiver_record");

    let mut byte_good = None;
    let mut tree_good = None;
    let mut byte_bad = None;
    let mut tree_bad = None;

    sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, false, |vm| {
        let before = vm.bytecode_invocations();
        byte_good = Some(vm.invoke(good, vec![]).unwrap());
        assert!(vm.bytecode_invocations() > before);
        byte_bad = Some(vm.invoke(bad, vec![]).unwrap_err().kind);
    });
    sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, true, |vm| {
        assert_eq!(vm.bytecode_invocations(), 0);
        tree_good = Some(vm.invoke(good, vec![]).unwrap());
        tree_bad = Some(vm.invoke(bad, vec![]).unwrap_err().kind);
        assert_eq!(vm.bytecode_invocations(), 0);
    });

    assert_eq!(byte_good, tree_good);
    assert_eq!(byte_bad, Some(ErrorKind::Type));
    assert_eq!(tree_bad, Some(ErrorKind::Type));
}
