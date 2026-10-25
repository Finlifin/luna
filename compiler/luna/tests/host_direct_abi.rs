//! Native ABI tests: explicit harnesses probe receiver routing and cleanup
//! control, including handlers installed by actual Flurry source.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn direct_entries_forward_environment_and_invoke_for_each_request() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias U=.u128'builtin;typealias Void=.void'builtin;
        effect change(ignored:Void,value:U)->U;
        ^attrs.effects.direct_passing effect bias()->U;
        fn unit() {}
        fn probe(value:U)->#[change,bias] U {
            let first=change(unit(),value)#;
            let second=change(unit(),first)#;
            let extra=bias()#;
            assumes second <= 1000; assumes extra <= 1000;
            second + extra
        }
        fn relay(value:U)->#[bias,change] U {probe(value)}
        fn run()->bool {let code=relay;true}",
    );
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1000000,
        package
            .traits
            .iter()
            .find(|t| t.name == "Duplicate")
            .unwrap()
            .declaration,
        package
            .traits
            .iter()
            .find(|t| t.name == "Retire")
            .unwrap()
            .declaration,
    )
    .unwrap();
    let index = prepared
        .graph()
        .functions
        .iter()
        .position(|instance| instance.function.name == "relay")
        .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1000000).unwrap();
    assert!(ir.contains("ptr %effect1_entry, ptr %effect1_env"), "{ir}");
    let ir = ir.replace("define i32 @main()", "define i32 @unused_main()");
    let ir = format!(
        "{ir}
        define i128 @bump(ptr %env, i128 %value) {{
            %old = load i128, ptr %env
            %next = add i128 %old, 1
            store i128 %next, ptr %env
            %result = add i128 %value, %old
            ret i128 %result
        }}
        define i32 @main() {{
            %one = alloca i128
            %two = alloca i128
            store i128 1, ptr %one
            store i128 10, ptr %two
            %a = call i128 @flurry_{index}(i128 40, i128 7, ptr @bump, ptr %one)
            store i1 false, ptr @flurry_failed
            %b = call i128 @flurry_{index}(i128 100, i128 9, ptr @bump, ptr %two)
            %n = load i128, ptr %one
            %m = load i128, ptr %two
            %a_ok = icmp eq i128 %a, 50
            %b_ok = icmp eq i128 %b, 130
            %n_ok = icmp eq i128 %n, 3
            %m_ok = icmp eq i128 %m, 12
            %ab = and i1 %a_ok, %b_ok
            %nm = and i1 %n_ok, %m_ok
            %ok = and i1 %ab, %nm
            %status = select i1 %ok, i32 0, i32 1
            ret i32 %status
        }}"
    );
    let rejected = ir.replace("%old = load i128, ptr %env", "store i1 true, ptr @flurry_failed\n%old = load i128, ptr %env")
        .replace("%a, 50", "%a, 0").replace("%b, 130", "%b, 0")
        .replace("%n, 3", "%n, 2").replace("%m, 12", "%m, 11")
        .replace("%status = select i1 %ok", "%failed = load i1, ptr @flurry_failed\n%stopped = and i1 %ok, %failed\n%status = select i1 %stopped");
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
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&rejected, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn contextual_direct_entry_without_acquired_cleanup_is_rejected() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Watch {target:*bool, previous:*bool}
        impl Retire for Watch {fn drop(*self) {
            assumes self.previous.*.*;
            assumes not self.target.*.*;
            self.target.*.* = true;
        }}
        effect ready()->bool;
        fn probe(value:Watch)->#ready bool {let first=ready()#; ready()# and first}
        fn relay(first:*bool, second:*bool, seed:*bool)->#ready bool {
            let outer=Watch {target:second,previous:first};
            probe(Watch {target:first,previous:seed})
        }
        fn run()->bool {let code=relay; true}",
    );
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1000000,
        package
            .traits
            .iter()
            .find(|t| t.name == "Duplicate")
            .unwrap()
            .declaration,
        package
            .traits
            .iter()
            .find(|t| t.name == "Retire")
            .unwrap()
            .declaration,
    )
    .unwrap();
    let error = luna::host::llvm_ir_prepared(&prepared, 1000000).unwrap_err();
    assert!(
        error.contains("has no acquired cleanup schedule"),
        "{error}"
    );
}

#[test]
fn direct_entry_normal_completion_checks_owned_parameter_destructor() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
        impl Duplicate for bool {}
        struct Watch {valid:bool}
        impl Retire for Watch {fn drop(*self) {assumes self.valid.*;}}
        effect ready()->bool;
        fn probe(value:Watch)->#ready bool {ready()#}
        fn relay(valid:bool)->#ready bool {probe(Watch {valid:valid})}
        fn run()->bool {let code=relay;true}",
    );
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1000000,
        package
            .traits
            .iter()
            .find(|t| t.name == "Duplicate")
            .unwrap()
            .declaration,
        package
            .traits
            .iter()
            .find(|t| t.name == "Retire")
            .unwrap()
            .declaration,
    )
    .unwrap();
    let index = prepared
        .graph()
        .functions
        .iter()
        .position(|instance| instance.function.name == "relay")
        .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1000000)
        .unwrap()
        .replace("define i32 @main()", "define i32 @unused_main()");
    let native = format!(
        "{ir}
        define i1 @ready(ptr %env) {{
            %n = load i32, ptr %env
            %next = add i32 %n, 1
            store i32 %next, ptr %env
            ret i1 true
        }}
        define i32 @main() {{
            %count = alloca i32
            store i32 0, ptr %count
            %good = call i1 @flurry_{index}(i1 true, ptr @ready, ptr %count)
            %good_failed = load i1, ptr @flurry_failed
            %good_ok = xor i1 %good_failed, true
            %n = load i32, ptr %count
            %count_ok = icmp eq i32 %n, 1
            %a = and i1 %good, %good_ok
            %ok = and i1 %a, %count_ok
            %status = select i1 %ok, i32 0, i32 1
            ret i32 %status
        }}"
    );
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&native, std::time::Duration::from_secs(5))
        .unwrap();
        // Cleanup failure is fatal in the current host contract. A missing
        // destructor would instead let the same machine call succeed.
        let invalid = native.replace("(i1 true, ptr @ready", "(i1 false, ptr @ready");
        let error = luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&invalid, std::time::Duration::from_secs(5))
        .unwrap_err();
        assert!(error.contains("native test failed"), "{error}");
    }
}

#[test]
fn outlined_source_handler_failure_unwinds_handler_and_requester() {
    let package = support::compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        effect ready()->bool;
        fn subject(target:*bool)->#ready bool {let owner=Watch {target:target};ready()#}
        fn scenario(valid:bool,inner:*bool,outer:*bool)->bool {
            subject(outer)# {ready()=>do {
                let owner=Watch {target:inner};assumes valid;true
            },}
        }
        fn run()->bool {let inner=false;let outer=false;scenario(true,inner.ref,outer.ref)}",
    );
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1000000,
        package
            .traits
            .iter()
            .find(|t| t.name == "Duplicate")
            .unwrap()
            .declaration,
        package
            .traits
            .iter()
            .find(|t| t.name == "Retire")
            .unwrap()
            .declaration,
    )
    .unwrap();
    let index = prepared
        .graph()
        .functions
        .iter()
        .position(|f| f.function.name == "scenario")
        .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1000000)
        .unwrap()
        .replace("define i32 @main()", "define i32 @unused_main()");
    for valid in [true, false] {
        let native = format!(
            "{ir}
            define i32 @main() {{
                %inner = alloca i1
                %outer = alloca i1
                store i1 false, ptr %inner
                store i1 false, ptr %outer
                %result = call i1 @flurry_{index}(i1 {valid}, ptr %inner, ptr %outer)
                %i = load i1, ptr %inner
                %o = load i1, ptr %outer
                %both = and i1 %i, %o
                %result_ok = icmp eq i1 %result, {valid}
                %failed = load i1, ptr @flurry_failed
                %failure_ok = icmp eq i1 %failed, {failed}
                %a = and i1 %both, %result_ok
                %ok = and i1 %a, %failure_ok
                %status = select i1 %ok, i32 0, i32 1
                ret i32 %status
            }}",
            failed = !valid
        );
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&native, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}

#[test]
fn direct_nonlocal_return_is_suspended_during_destructor_invocation() {
    let package = support::compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for bool {}impl Duplicate for *T where T {}
        struct Watch {target:*bool}
        impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}}
        effect read()->bool;
        fn subject(target:*bool)->#read bool {let owner=Watch {target:target};read()#}
        fn install(target:*bool)->bool {subject(target)# {read()=>do {return true;},};false}
        fn run()->bool {let seen=false;let result=install(seen.ref);result and seen}",
    );
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1000000,
        package
            .traits
            .iter()
            .find(|t| t.name == "Duplicate")
            .unwrap()
            .declaration,
        package
            .traits
            .iter()
            .find(|t| t.name == "Retire")
            .unwrap()
            .declaration,
    )
    .unwrap();
    let destructor = prepared
        .graph()
        .functions
        .iter()
        .position(|f| f.function.name == "drop")
        .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1000000).unwrap();
    let head = format!("define internal void @flurry_{destructor}(ptr %arg0) {{\nentry:\n");
    assert!(ir.contains(&head), "destructor machine signature missing");
    let ir = ir
        .replace(
            &head,
            &format!("{head}  call void @check_control_is_suspended()\n"),
        )
        .replace("define i32 @main()", "define i32 @unused_main()");
    let ir = format!(
        "{ir}
        define void @check_control_is_suspended() {{
            %pending = load ptr, ptr @flurry_effect_control
            %clear = icmp eq ptr %pending, null
            br i1 %clear, label %good, label %bad
        bad:
            call void @llvm.trap()
            unreachable
        good:
            ret void
        }}
        define i32 @main() {{
            %result = call i1 @flurry_entry()
            %status = select i1 %result, i32 0, i32 1
            ret i32 %status
        }}"
    );
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
fn machine_return_destinations_are_only_allocated_for_handler_installers() {
    let package = support::compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        fn plain(value:bool)->bool {return value;}
        fn relay()->#read bool {read()#}
        fn install()->bool {relay()# {read()=>do {return true;},};false}
        fn run()->bool {plain(install())}",
    );
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1000000,
        package
            .traits
            .iter()
            .find(|t| t.name == "Duplicate")
            .unwrap()
            .declaration,
        package
            .traits
            .iter()
            .find(|t| t.name == "Retire")
            .unwrap()
            .declaration,
    )
    .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1000000).unwrap();
    let body = |name: &str| {
        let index = prepared
            .graph()
            .functions
            .iter()
            .position(|f| f.function.name == name)
            .unwrap();
        let marker = format!("define internal i1 @flurry_{index}(");
        ir.split_once(&marker)
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0
    };
    // Plain has only its ordinary parameter slot, with no control token/result.
    assert_eq!(
        body("plain").matches("alloca i1").count(),
        1,
        "{}",
        body("plain")
    );
    assert!(!body("relay").contains("icmp eq ptr"), "{}", body("relay"));
    assert!(body("relay").contains("@flurry_effect_control"));
    assert!(body("install").contains("icmp eq ptr"));
    let ir = ir.replace("define i32 @main()", "define i32 @unused_main()");
    let ir = format!(
        "{ir}\ndefine i32 @main() {{\n %value = call i1 @flurry_entry()\n %status = select i1 %value, i32 0, i32 1\n ret i32 %status\n}}\n"
    );
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
fn machine_loop_destinations_are_only_allocated_for_handler_exit_targets() {
    let package = support::compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}impl Duplicate for bool {}
        effect read()->bool;
        fn plain()->bool {while true {break;}true}
        fn relay()->#read bool {while true {return read()#;}}
        fn install()->bool {
            while true {relay()# {read()=>do {break;},};return false;}
            true
        }
        fn run()->bool {plain() and install()}",
    );
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1000000,
        package
            .traits
            .iter()
            .find(|t| t.name == "Duplicate")
            .unwrap()
            .declaration,
        package
            .traits
            .iter()
            .find(|t| t.name == "Retire")
            .unwrap()
            .declaration,
    )
    .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1000000).unwrap();
    let body = |name: &str| {
        let index = prepared
            .graph()
            .functions
            .iter()
            .position(|f| f.function.name == name)
            .unwrap();
        let marker = format!("define internal i1 @flurry_{index}(");
        ir.split_once(&marker)
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0
    };
    // An ordinary loop never needs dynamic exit targets. The loop in relay
    // likewise forwards pending exits without claiming them.
    assert_eq!(
        body("plain").matches("alloca i1").count(),
        0,
        "{}",
        body("plain")
    );
    assert!(!body("relay").contains("icmp eq ptr"), "{}", body("relay"));
    assert!(body("relay").contains("@flurry_effect_control"));
    assert!(body("install").contains("icmp eq ptr"));
    let ir = ir.replace("define i32 @main()", "define i32 @unused_main()");
    let ir = format!(
        "{ir}\ndefine i32 @main() {{\n %value = call i1 @flurry_entry()\n %status = select i1 %value, i32 0, i32 1\n ret i32 %status\n}}\n"
    );
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
