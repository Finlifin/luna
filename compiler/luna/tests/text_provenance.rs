//! End-to-end real-sysroot tests for `TextProvenance::VerifiedByRegionAnalysis`,
//! `str` method cleanup, borrowed-code/effect `str` contexts, intrinsic `str`
//! provenance grounding, and negative dangling/ungrounded `str` rejections.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use elaboration::{
    capabilities::World,
    implementation::{ImplementationHead, Index},
    ptc::{Budget, Context},
    verification::memory,
};
use intrinsic::{LangItem, LangItems};
use luna::protocols;
use nir::program::{ExprKind as K, Package};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-text-prov-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                "0",
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for NativeCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn with_sysroot_world<T>(package: &Package, run: impl FnOnce(&World<'_>, &LangItems) -> T) -> T {
    let context = Context::from_package(package).unwrap();
    let mut index = Index::default();
    for i in package
        .implementations
        .iter()
        .filter(|implementation| implementation.interface.is_some())
    {
        index
            .insert(
                ImplementationHead {
                    parameter_types: i.parameter_types.clone(),
                    declaration: i.declaration,
                    interface: i.interface,
                    interface_arguments: i.interface_arguments.clone(),
                    subject: i.subject.clone(),
                    type_parameters: i.type_parameters.clone(),
                    requirements: i.requirements.clone(),
                },
                Budget::default(),
            )
            .unwrap();
    }
    let copy_protocol = package
        .traits
        .iter()
        .find(|t| t.name == "Copy")
        .expect("Copy trait in sysroot")
        .declaration;
    let drop_protocol = package
        .traits
        .iter()
        .find(|t| t.name == "Drop")
        .expect("Drop trait in sysroot")
        .declaration;
    let mut items = LangItems::new();
    items
        .bind_source_traits(
            package,
            &[
                (LangItem::Copy, copy_protocol),
                (LangItem::Drop, drop_protocol),
            ],
        )
        .unwrap();
    run(
        &World {
            index: &index,
            implementations: &package.implementations,
            traits: &package.traits,
            assumptions: &[],
            context: &context,
        },
        &items,
    )
}

fn user_function<'a>(package: &'a Package, name: &str) -> &'a nir::program::Function {
    let id = support::source_function(package, name);
    package
        .functions
        .iter()
        .find(|function| function.declaration == id)
        .expect("user function body")
}

fn first_trait_method_callee(function: &nir::program::Function) -> nir::program::Expr {
    let mut found = None;
    nir::visit::expression(&function.body, &mut 20_000, |node, _| {
        if found.is_none() && matches!(node.kind, K::TraitMethod { .. }) {
            found = Some(node.clone());
        }
        Ok(())
    })
    .unwrap();
    found.expect("trait method callee")
}

#[test]
fn native_text_method_provenance_repro_and_chained_str_methods_pass() {
    let repro = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../development/repros/native-text-method-provenance.fl");
    let sysroot = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let repro_output = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .arg("test")
        .arg(&repro)
        .arg("--sysroot")
        .arg(&sysroot)
        .output()
        .unwrap();
    assert!(
        repro_output.status.success(),
        "repro test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&repro_output.stdout),
        String::from_utf8_lossy(&repro_output.stderr)
    );

    let source = r#"
        fn inspect_byte_length(s: str) -> u64 {
            s.byte_length()
        }

        fn inspect_contains(haystack: str, needle: str) -> bool {
            haystack.contains(needle)
        }

        fn chained_checks(value: str) -> bool {
            (not value.is_empty())
                and value.starts_with("a")
                and value.ends_with("z")
                and value.contains("🦀")
                and (value.find("🦀") matches 1?)
                and (value.byte(4) matches 128?)
                and (value.byte(6) matches null)
                and value.byte_length() == 6
        }

        fn run() -> bool {
            let text = "a🦀z";
            let empty_ok = "".is_empty() and "".byte_length() == 0;
            empty_ok
                and inspect_byte_length(text) == 6
                and inspect_contains(text, "🦀")
                and chained_checks(text)
        }
    "#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert!(
        build.status.success(),
        "native build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = case.run();
    assert!(
        run.status.success(),
        "native run failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "(bool true)\n");

    let package = support::compile(source).unwrap();
    with_sysroot_world(&package, |world, items| {
        for fn_name in ["inspect_byte_length", "inspect_contains", "chained_checks"] {
            let func = user_function(&package, fn_name);
            let report = protocols::analyze_cleanup_with_borrowed_code_context(
                &package,
                world,
                items,
                func,
                2_000_000,
                Budget::default(),
                &[],
                &[],
                &[],
                vec![],
                &[],
                &[],
            )
            .unwrap_or_else(|e| panic!("cleanup analysis failed for {fn_name}: {e}"));
            assert!(report.deferred_calls.is_empty(), "{report}");
        }

        for method_caller in ["inspect_byte_length", "inspect_contains"] {
            let callee = first_trait_method_callee(user_function(&package, method_caller));
            let cleanup = protocols::analyze_method_cleanup(
                &package,
                world,
                items,
                &callee,
                200_000,
                Budget::default(),
            )
            .unwrap_or_else(|e| panic!("analyze_method_cleanup failed for {method_caller}: {e}"));
            assert!(cleanup.report.deferred_calls.is_empty(), "{cleanup}");
        }
    });
}

#[test]
fn borrowed_code_and_higher_order_str_contexts_verify_cleanly() {
    let source = r#"
        struct Check {
            check_fn: fn(str) -> bool
        }

        trait Runner {
            fn exec(*self, s: str, f: fn(str) -> bool) -> bool;
        }

        struct Host {}

        impl Runner for Host {
            fn exec(*self, s: str, f: fn(str) -> bool) -> bool {
                f(s)
            }
        }

        fn has_prefix_a(s: str) -> bool {
            s.starts_with("a")
        }

        fn apply_direct(s: str, f: fn(str) -> bool) -> bool {
            f(s)
        }

        fn apply_struct(check: Check, s: str) -> bool {
            (check.check_fn)(s)
        }

        fn run() -> bool {
            let check = Check { check_fn: has_prefix_a };
            let host = Host {};
            apply_direct("abc", has_prefix_a)
                and apply_struct(check, "abc")
                and host.exec("abc", has_prefix_a)
        }
    "#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert!(
        build.status.success(),
        "build failed:\nstderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = case.run();
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout), "(bool true)\n");

    let borrowed_source = r#"
        struct Check {
            check_fn: fn(str) -> bool
        }

        fn has_prefix_a(s: str) -> bool {
            s.starts_with("a")
        }

        fn apply_borrowed(check: *Check, s: str) -> bool {
            (check.check_fn.*)(s)
        }
    "#;

    let package = support::compile(source).unwrap();
    let has_prefix = user_function(&package, "has_prefix_a");
    let apply_direct = user_function(&package, "apply_direct");
    let apply_struct = user_function(&package, "apply_struct");

    let named_code = memory::CallableResult::Known(iir::InstanceKey {
        declaration: has_prefix.declaration,
        arguments: vec![].into(),
    });
    let inputs = BTreeMap::from([(
        memory::CallableInput {
            parameter: 1,
            fields: vec![],
        },
        named_code.clone(),
    )]);
    let mut fuel = 200_000;
    let direct_summary = memory::infer_with_borrowed_callable_inputs_and_cleanup(
        &package,
        apply_direct.declaration,
        &[],
        &inputs,
        &BTreeMap::new(),
        &[],
        &BTreeMap::new(),
        None,
        &mut fuel,
    )
    .expect("higher-order str callback memory inference must accept VerifiedByRegionAnalysis");
    assert!(direct_summary.accesses.is_empty(), "{direct_summary}");

    let borrowed_pkg = support::compile(borrowed_source).unwrap();
    let borrowed_prefix = user_function(&borrowed_pkg, "has_prefix_a");
    let apply_borrowed = user_function(&borrowed_pkg, "apply_borrowed");
    let borrowed_named = memory::CallableResult::Known(iir::InstanceKey {
        declaration: borrowed_prefix.declaration,
        arguments: vec![].into(),
    });
    let borrowed_inputs = BTreeMap::from([(
        memory::CallableInput {
            parameter: 0,
            fields: vec![],
        },
        BTreeMap::from([(
            vec![0],
            memory::BorrowedCallableValue::Named(borrowed_named),
        )]),
    )]);
    let borrowed_summary = with_sysroot_world(&borrowed_pkg, |world, items| {
        let intrinsic::LangItemDef::UserDef(drop_protocol) = items.get(LangItem::Drop).unwrap()
        else {
            panic!("Drop trait")
        };
        let session = elaboration::verification::PackageSession::new(&borrowed_pkg);
        let mut cleanup = session
            .cleanup_session(world, drop_protocol, Budget::default())
            .with_text_provenance(memory::TextProvenance::VerifiedByRegionAnalysis);
        memory::infer_with_borrowed_callable_inputs_and_cleanup(
            &borrowed_pkg,
            apply_borrowed.declaration,
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &borrowed_inputs,
            Some(&mut cleanup),
            &mut fuel,
        )
    })
    .expect("borrowed-code str memory inference must accept VerifiedByRegionAnalysis");
    assert!(
        borrowed_summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.read_value),
        "{borrowed_summary}"
    );

    with_sysroot_world(&package, |world, items| {
        let run_fn = user_function(&package, "run");
        let run_report = protocols::analyze_cleanup_with_borrowed_code_context(
            &package,
            world,
            items,
            run_fn,
            500_000,
            Budget::default(),
            &[],
            &[],
            &[],
            vec![],
            &[],
            &[],
        )
        .expect("cleanup analysis on run succeeds");
        assert!(run_report.deferred_calls.is_empty(), "{run_report}");

        let direct_call = run_report
            .named_calls
            .iter()
            .find(|call| matches!(call.callee.kind, K::Function(id) if id == apply_direct.declaration))
            .expect("apply_direct named call context");
        assert!(!direct_call.callables.is_empty(), "{run_report}");
        let direct_report = protocols::analyze_cleanup_with_borrowed_code_context(
            &package,
            world,
            items,
            apply_direct,
            500_000,
            Budget::default(),
            &[],
            &direct_call.callables,
            &direct_call.enums,
            direct_call.effects.clone(),
            &[],
            &direct_call.borrowed_code,
        )
        .expect("cleanup with callable parameter over str succeeds");
        assert!(direct_report.deferred_calls.is_empty(), "{direct_report}");

        let struct_call = run_report
            .named_calls
            .iter()
            .find(|call| matches!(call.callee.kind, K::Function(id) if id == apply_struct.declaration))
            .expect("apply_struct named call context");
        assert!(!struct_call.callables.is_empty(), "{run_report}");
        let struct_report = protocols::analyze_cleanup_with_borrowed_code_context(
            &package,
            world,
            items,
            apply_struct,
            500_000,
            Budget::default(),
            &[],
            &struct_call.callables,
            &struct_call.enums,
            struct_call.effects.clone(),
            &[],
            &struct_call.borrowed_code,
        )
        .expect("cleanup with aggregate callable field over str succeeds");
        assert!(struct_report.deferred_calls.is_empty(), "{struct_report}");

        let mut mem_fuel = 500_000;
        let inferred_mem = protocols::infer_borrowed_callable_memory(
            &package,
            world,
            items,
            apply_struct.declaration,
            &[],
            &struct_call.callables,
            &struct_call.enums,
            &struct_call.effects,
            &struct_call.borrowed_code,
            &mut mem_fuel,
            Budget::default(),
            apply_struct.body.source,
        )
        .expect("infer_borrowed_callable_memory over str succeeds");
        assert!(inferred_mem.accesses.is_empty(), "{inferred_mem}");

        let method_callee = first_trait_method_callee(run_fn);
        let method_cleanup = protocols::analyze_method_cleanup_with_context(
            &package,
            world,
            items,
            &method_callee,
            500_000,
            Budget::default(),
            &[],
            &direct_call.callables,
            &[],
            vec![],
            &[],
            &[],
        );
        // `direct_call.callables` has parameter index 1 while `Runner.exec` takes `(self, s, f)` at parameter 2.
        assert!(method_cleanup.is_err());

        let mut method_callables = direct_call.callables.clone();
        method_callables[0].parameter = 2;
        let method_cleanup = protocols::analyze_method_cleanup_with_context(
            &package,
            world,
            items,
            &method_callee,
            500_000,
            Budget::default(),
            &[],
            &method_callables,
            &[],
            vec![],
            &[],
            &[],
        )
        .expect("analyze_method_cleanup_with_context with str and callback succeeds");
        assert!(
            method_cleanup.report.deferred_calls.is_empty(),
            "{method_cleanup}"
        );
    });

    // Negative: ungrounded raw_read<str>(0) inside a higher-order or borrowed-code
    // callback context must fail closed rather than bypassing text-region verification.
    let bad_callback_source = r#"
        struct Check {
            check_fn: fn(str) -> bool
        }

        fn has_prefix_a(s: str) -> bool {
            s.starts_with("a")
        }

        fn bad_callback_forge(f: fn(str) -> bool) -> bool {
            let zero: usize = 0;
            f(unsafe { builtin.mem.raw_read<str>(zero) })
        }

        fn bad_borrowed_forge(check: *Check) -> bool {
            let zero: usize = 0;
            (check.check_fn.*)(unsafe { builtin.mem.raw_read<str>(zero) })
        }

        fn run() -> bool {
            bad_callback_forge(has_prefix_a)
        }
    "#;
    let bad_pkg = support::compile(bad_callback_source).unwrap();
    let bad_prefix = user_function(&bad_pkg, "has_prefix_a");
    let bad_cb_fn = user_function(&bad_pkg, "bad_callback_forge");
    let bad_borrowed_fn = user_function(&bad_pkg, "bad_borrowed_forge");
    let bad_run_fn = user_function(&bad_pkg, "run");

    let bad_cb_inputs = BTreeMap::from([(
        memory::CallableInput {
            parameter: 0,
            fields: vec![],
        },
        memory::CallableResult::Known(iir::InstanceKey {
            declaration: bad_prefix.declaration,
            arguments: vec![].into(),
        }),
    )]);
    let bad_cb_mem_err = memory::infer_with_borrowed_callable_inputs_and_cleanup(
        &bad_pkg,
        bad_cb_fn.declaration,
        &[],
        &bad_cb_inputs,
        &BTreeMap::new(),
        &[],
        &BTreeMap::new(),
        None,
        &mut 200_000,
    )
    .expect_err("higher-order callback forging str via raw_read must fail closed");
    assert!(
        bad_cb_mem_err.contains("backing-region provenance"),
        "{bad_cb_mem_err}"
    );

    let bad_borrowed_inputs = BTreeMap::from([(
        memory::CallableInput {
            parameter: 0,
            fields: vec![],
        },
        BTreeMap::from([(
            vec![0],
            memory::BorrowedCallableValue::Named(memory::CallableResult::Known(iir::InstanceKey {
                declaration: bad_prefix.declaration,
                arguments: vec![].into(),
            })),
        )]),
    )]);
    let bad_borrowed_err = memory::infer_with_borrowed_callable_inputs_and_cleanup(
        &bad_pkg,
        bad_borrowed_fn.declaration,
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &bad_borrowed_inputs,
        None,
        &mut 200_000,
    )
    .expect_err("borrowed-code function forging str via raw_read must fail closed");
    assert!(
        bad_borrowed_err.contains("backing-region provenance"),
        "{bad_borrowed_err}"
    );

    with_sysroot_world(&bad_pkg, |world, items| {
        let run_err = protocols::analyze_cleanup_with_borrowed_code_context(
            &bad_pkg,
            world,
            items,
            bad_run_fn,
            500_000,
            Budget::default(),
            &[],
            &[],
            &[],
            vec![],
            &[],
            &[],
        )
        .expect_err("protocols cleanup on caller of higher-order forged str must fail closed");
        assert!(
            run_err.reason.contains("backing-region provenance"),
            "{run_err}"
        );

        let clean_call_source = r#"
            fn has_prefix_a(s: str) -> bool { s.starts_with("a") }
            fn bad_callback_forge(f: fn(str) -> bool) -> bool {
                let zero: usize = 0;
                f(unsafe { builtin.mem.raw_read<str>(zero) })
            }
            fn pass_clean() -> bool {
                let f: fn(str) -> bool = has_prefix_a;
                f("abc")
            }
        "#;
        let _ = clean_call_source;
        // Construct CallableParameter directly from `bad_prefix` for `bad_cb_fn` parameter 0.
        let callable_param = elaboration::verification::moves::CallableParameter {
            choices: vec![],
            enum_captures: BTreeMap::new(),
            parameter: 0,
            fields: vec![],
            definition: nir::program::Expr {
                source: bad_prefix.body.source,
                ty: bad_prefix.signature.clone(),
                kind: K::Function(bad_prefix.declaration),
            },
            alternatives: vec![],
            captures: BTreeMap::new(),
        };
        let cb_cleanup_err = protocols::analyze_cleanup_with_borrowed_code_context(
            &bad_pkg,
            world,
            items,
            bad_cb_fn,
            500_000,
            Budget::default(),
            &[],
            &[callable_param],
            &[],
            vec![],
            &[],
            &[],
        )
        .expect_err("protocols cleanup with callback parameter forging str must fail closed");
        assert!(
            cb_cleanup_err.reason.contains("backing-region provenance"),
            "{cb_cleanup_err}"
        );
    });
}

#[test]
fn effect_adjacent_str_borrows_verify_and_effect_request_carrying_str_fails_closed() {
    let source = r#"
        effect step() -> bool;

        fn compute_with_str(initial: str) -> #step str {
            var current: str = initial;
            let override_it = step()#;
            if override_it {
                current = "override";
            }
            current
        }

        fn handle_str(initial: str) -> str {
            compute_with_str(initial)# {
                step() => do { resume true; },
            }
        }

        fn run() -> bool {
            let out = handle_str("seed");
            out.contains("over") and out.byte_length() == 8
        }
    "#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert!(
        build.status.success(),
        "effect-adjacent str build failed:\nstderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = case.run();
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout), "(bool true)\n");

    let package = support::compile(source).unwrap();
    let handle_fn = user_function(&package, "handle_str");
    with_sysroot_world(&package, |world, items| {
        let report = protocols::analyze_cleanup_with_borrowed_code_context(
            &package,
            world,
            items,
            handle_fn,
            200_000,
            Budget::default(),
            &[],
            &[],
            &[],
            vec![],
            &[],
            &[],
        )
        .expect("effect-handling str function passes cleanup analysis");
        assert!(report.deferred_calls.is_empty(), "{report}");
    });

    // Handler arm memory analysis accepts str-containing closed handler arms.
    let arm_source = r#"
        effect pick(flag: bool) -> bool;
        fn choose(fallback: str) -> #pick str {
            let picked = pick(true)#;
            if picked { "left" } else { fallback }
        }
        fn run_arm(s: str) -> str {
            choose(s)# {
                pick(flag) => do {
                    let candidate = if flag { "abc" } else { "xyz" };
                    resume candidate.starts_with("a");
                },
            }
        }
    "#;
    let arm_pkg = support::compile(arm_source).unwrap();
    let run_arm = user_function(&arm_pkg, "run_arm");
    let mut handler = None;
    nir::visit::expression(&run_arm.body, &mut 20_000, |node, _| {
        if handler.is_none()
            && let K::EffectHandle { handlers, .. } = &node.kind
        {
            handler = Some(handlers[0].clone());
        }
        Ok(())
    })
    .unwrap();
    let handler = handler.expect("effect handler");
    let arm_summary =
        memory::infer_handler_arm_memory(&arm_pkg, &handler, &handler.arms[0], &mut 200_000)
            .expect("infer_handler_arm_memory succeeds for effect-adjacent str");
    assert!(arm_summary.accesses.is_empty(), "{arm_summary}");

    // Negative: a handler arm forging `str` via `raw_read<str>(0)` must be rejected.
    let bad_arm_source = r#"
        effect pick(flag: bool) -> bool;
        fn choose(fallback: str) -> #pick str {
            let picked = pick(true)#;
            if picked { "left" } else { fallback }
        }
        fn run_bad_arm(s: str) -> str {
            choose(s)# {
                pick(_flag) => do {
                    let zero: usize = 0;
                    let forged = unsafe { builtin.mem.raw_read<str>(zero) };
                    resume forged.is_empty();
                },
            }
        }
    "#;
    let bad_arm_pkg = support::compile(bad_arm_source).unwrap();
    let run_bad_arm = user_function(&bad_arm_pkg, "run_bad_arm");
    let mut bad_handler = None;
    nir::visit::expression(&run_bad_arm.body, &mut 20_000, |node, _| {
        if bad_handler.is_none()
            && let K::EffectHandle { handlers, .. } = &node.kind
        {
            bad_handler = Some(handlers[0].clone());
        }
        Ok(())
    })
    .unwrap();
    let bad_handler = bad_handler.expect("bad effect handler");
    let bad_arm_err = memory::infer_handler_arm_memory(
        &bad_arm_pkg,
        &bad_handler,
        &bad_handler.arms[0],
        &mut 200_000,
    )
    .expect_err("handler arm forging str via raw_read must fail closed");
    assert!(
        bad_arm_err.contains("backing-region provenance"),
        "{bad_arm_err}"
    );

    // Negative: unverified standalone `infer_closure_with_effect_inputs` fails closed on `str`.
    let bad_closure_source = r#"
        fn make_bad() -> bool {
            let f = || -> bool {
                let zero: usize = 0;
                let forged = unsafe { builtin.mem.raw_read<str>(zero) };
                forged.is_empty()
            };
            f()
        }
    "#;
    let bad_closure_pkg = support::compile(bad_closure_source).unwrap();
    let make_bad = user_function(&bad_closure_pkg, "make_bad");
    let mut closure_expr = None;
    nir::visit::expression(&make_bad.body, &mut 20_000, |node, _| {
        if closure_expr.is_none() && matches!(node.kind, K::Closure { .. }) {
            closure_expr = Some(node.clone());
        }
        Ok(())
    })
    .unwrap();
    let closure_expr = closure_expr.expect("closure expression");
    let closure_err = memory::infer_closure_with_effect_inputs(
        &bad_closure_pkg,
        &closure_expr,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut 200_000,
    )
    .expect_err("unverified infer_closure_with_effect_inputs over str must fail closed");
    assert!(
        closure_err.contains("string-view memory provenance"),
        "{closure_err}"
    );

    // Negative: transmitting `str` through an EffectRequest payload must still fail closed.
    let bad_effect_source = r#"
        effect leak_text(s: str) -> bool;
        fn send_text(s: str) -> #leak_text bool {
            leak_text(s)#
        }
        fn run() -> bool {
            send_text("secret")# {
                leak_text(x) => do { resume x.is_empty(); },
            }
        }
    "#;
    let bad_case = NativeCase::new(bad_effect_source);
    let bad_build = bad_case.build();
    assert!(
        !bad_build.status.success(),
        "effect request transmitting str unexpectedly built"
    );
    let stderr = String::from_utf8_lossy(&bad_build.stderr);
    assert!(
        stderr.contains("text effect") || stderr.contains("text"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn ungrounded_intrinsic_str_and_dangling_str_pointer_reject_cleanly() {
    let forged_source = r#"
        fn forge() -> str {
            let zero: usize = 0;
            unsafe { builtin.mem.raw_read<str>(zero) }
        }

        fn run() -> bool {
            forge().is_empty()
        }
    "#;
    let forged_case = NativeCase::new(forged_source);
    let forged_build = forged_case.build();
    assert!(
        !forged_build.status.success(),
        "raw_read<str> unexpectedly succeeded"
    );
    let forged_stderr = String::from_utf8_lossy(&forged_build.stderr);
    assert!(
        forged_stderr.contains("backing-region provenance"),
        "unexpected stderr: {forged_stderr}"
    );

    let forged_pkg = support::compile(forged_source).unwrap();
    let forge_fn = user_function(&forged_pkg, "forge");
    let text_err = memory::infer_text_regions(&forged_pkg, forge_fn.declaration, &[], &mut 200_000)
        .expect_err("raw_read<str> must fail text_regions");
    assert!(text_err.contains("backing-region provenance"), "{text_err}");
    with_sysroot_world(&forged_pkg, |world, items| {
        let cleanup_err = protocols::analyze_cleanup_with_borrowed_code_context(
            &forged_pkg,
            world,
            items,
            forge_fn,
            200_000,
            Budget::default(),
            &[],
            &[],
            &[],
            vec![],
            &[],
            &[],
        )
        .expect_err(
            "analyze_cleanup_with_borrowed_code_context must reject ungrounded raw_read<str>",
        );
        assert!(
            cleanup_err.reason.contains("backing-region provenance"),
            "{cleanup_err}"
        );
    });

    let dangling_source = r#"
        fn leak() -> *str {
            let local = "ephemeral";
            local.ref
        }

        fn run() -> bool {
            let _ = leak();
            true
        }
    "#;
    let dangling_case = NativeCase::new(dangling_source);
    let dangling_build = dangling_case.build();
    assert!(
        !dangling_build.status.success(),
        "dangling *str return unexpectedly succeeded"
    );
    let dangling_stderr = String::from_utf8_lossy(&dangling_build.stderr);
    assert!(
        dangling_stderr.contains("text descriptor address")
            || dangling_stderr.contains("local")
            || dangling_stderr.contains("escape")
            || dangling_stderr.contains("borrow"),
        "unexpected stderr: {dangling_stderr}"
    );
}

#[test]
fn cleanup_graph_extension_preserves_text_provenance_for_late_enum_rounds() {
    // Valid case: optional/error-result layout extension + owned resource cleanup + str methods.
    let valid_source = r#"
        enum TextErr { Failed }

        fn check_text(flag: bool, input: str) -> !TextErr bool {
            if not flag {
                return error TextErr.Failed;
            }
            input.contains("ok") and (input.byte(0) matches 111?)
        }

        fn run() -> bool {
            let ok = check_text(true, "ok")! {
                TextErr.Failed => false,
            };
            ok
        }
    "#;
    let valid_case = NativeCase::new(valid_source);
    let valid_build = valid_case.build();
    assert!(
        valid_build.status.success(),
        "valid build failed:\nstderr: {}",
        String::from_utf8_lossy(&valid_build.stderr)
    );
    let valid_run = valid_case.run();
    assert!(valid_run.status.success());
    assert_eq!(String::from_utf8_lossy(&valid_run.stdout), "(bool true)\n");

    // Negative case: generated error-result enums + late forged raw_read<str>(0) must STILL fail closed!
    let late_forged_source = r#"
        enum TextErr { Failed }

        fn late_forge(flag: bool) -> !TextErr str {
            if not flag {
                return error TextErr.Failed;
            }
            let zero: usize = 0;
            unsafe { builtin.mem.raw_read<str>(zero) }
        }

        fn run() -> bool {
            let s = late_forge(true)! {
                TextErr.Failed => "",
            };
            s.is_empty()
        }
    "#;
    let late_case = NativeCase::new(late_forged_source);
    let late_build = late_case.build();
    assert!(
        !late_build.status.success(),
        "late raw_read<str> after enum generation unexpectedly built"
    );
    let late_stderr = String::from_utf8_lossy(&late_build.stderr);
    assert!(
        late_stderr.contains("backing-region provenance"),
        "unexpected stderr: {late_stderr}"
    );
}
