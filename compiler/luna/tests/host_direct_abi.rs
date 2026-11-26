//! Native ABI tests: explicit harnesses probe receiver routing and cleanup
//! control, including handlers installed by actual Flurry source.
#[macro_use]
#[path = "support/effect_fixture.rs"]
mod effect_fixture;

#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

#[test]
fn direct_entries_forward_environment_and_invoke_for_each_request() {
    let package = support::compile(effect_fixture!(
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
        fn run()->bool {let code=relay;true}"
    ));
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
        define {{ i128, ptr }} @bump(ptr %env, i128 %value, ptr %task) {{
            %old = load i128, ptr %env
            %next = add i128 %old, 1
            store i128 %next, ptr %env
            %result = add i128 %value, %old
            %pair = insertvalue {{ i128, ptr }} zeroinitializer, i128 %result, 0
            ret {{ i128, ptr }} %pair
        }}
        define i32 @main() {{
            %task = alloca %flurry.task
            call void @flurry_task_init(ptr %task, ptr null)
            %one = alloca i128
            %two = alloca i128
            store i128 1, ptr %one
            store i128 10, ptr %two
            %ar = call {{ i128, ptr }} @flurry_{index}(i128 40, i128 7, ptr @bump, ptr %one, ptr %task)
            %a = extractvalue {{ i128, ptr }} %ar, 0
            %a_ctrl = extractvalue {{ i128, ptr }} %ar, 1
            %a_normal = icmp eq ptr %a_ctrl, null
            %br = call {{ i128, ptr }} @flurry_{index}(i128 100, i128 9, ptr @bump, ptr %two, ptr %task)
            %b = extractvalue {{ i128, ptr }} %br, 0
            %b_ctrl = extractvalue {{ i128, ptr }} %br, 1
            %b_normal = icmp eq ptr %b_ctrl, null
            %n = load i128, ptr %one
            %m = load i128, ptr %two
            %a_ok = icmp eq i128 %a, 50
            %b_ok = icmp eq i128 %b, 130
            %n_ok = icmp eq i128 %n, 3
            %m_ok = icmp eq i128 %m, 12
            %ab = and i1 %a_ok, %b_ok
            %nm = and i1 %n_ok, %m_ok
            %data = and i1 %ab, %nm
            %both_normal = and i1 %a_normal, %b_normal
            %ok = and i1 %data, %both_normal
            %status = select i1 %ok, i32 0, i32 1
            ret i32 %status
        }}"
    );
    let rejected = ir
        .replace(
            "ret { i128, ptr } %pair",
            "ret { i128, ptr } { i128 0, ptr inttoptr (i32 1 to ptr) }",
        )
        .replace("_ctrl, null", "_ctrl, inttoptr (i32 1 to ptr)")
        .replace("%a, 50", "%a, 0")
        .replace("%b, 130", "%b, 0")
        .replace("%n, 3", "%n, 2")
        .replace("%m, 12", "%m, 11");
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
        define {{ i1, ptr }} @ready(ptr %env, ptr %task) {{
            %n = load i32, ptr %env
            %next = add i32 %n, 1
            store i32 %next, ptr %env
            ret {{ i1, ptr }} {{ i1 true, ptr null }}
        }}
        define i32 @main() {{
            %count = alloca i32
            store i32 0, ptr %count
            %task = alloca %flurry.task
            call void @flurry_task_init(ptr %task, ptr null)
            %r = call {{ i1, ptr }} @flurry_{index}(i1 true, ptr @ready, ptr %count, ptr %task)
            %good = extractvalue {{ i1, ptr }} %r, 0
            %ctrl = extractvalue {{ i1, ptr }} %r, 1
            %good_ok = icmp eq ptr %ctrl, null
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
                %task = alloca %flurry.task
                call void @flurry_task_init(ptr %task, ptr null)
                %r = call {{ i1, ptr }} @flurry_{index}(i1 {valid}, ptr %inner, ptr %outer, ptr %task)
                %result = extractvalue {{ i1, ptr }} %r, 0
                %i = load i1, ptr %inner
                %o = load i1, ptr %outer
                %both = and i1 %i, %o
                %result_ok = icmp eq i1 %result, {valid}
                %ctrl = extractvalue {{ i1, ptr }} %r, 1
                %failed = icmp eq ptr %ctrl, inttoptr (i32 1 to ptr)
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
    assert!(!ir.contains("thread_local"));
    let head = format!("@flurry_{destructor}(");
    assert!(ir.contains(&head));
    let ir = ir.replace("define i32 @main()", "define i32 @unused_main()");
    let ir = format!(
        "{ir}\ndefine i32 @main() {{\n  %result = call i1 @flurry_entry()\n  %status = select i1 %result, i32 0, i32 1\n  ret i32 %status\n}}"
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
        let scalar = format!("define internal i1 @flurry_{index}_scalar(");
        let marker = if ir.contains(&scalar) {
            scalar
        } else {
            format!("define internal {{ i1, ptr }} @flurry_{index}(")
        };
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
    assert!(
        !body("relay").contains("alloca i8, align 2"),
        "{}",
        body("relay")
    );
    assert!(body("relay").contains("extractvalue { i1, ptr }"));
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
        let scalar = format!("define internal i1 @flurry_{index}_scalar(");
        let marker = if ir.contains(&scalar) {
            scalar
        } else {
            format!("define internal {{ i1, ptr }} @flurry_{index}(")
        };
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
    assert!(
        !body("relay").contains("alloca i8, align 2"),
        "{}",
        body("relay")
    );
    assert!(body("relay").contains("extractvalue { i1, ptr }"));
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

fn checked_module(package: &nir::program::Package) -> (String, luna::host::Prepared) {
    let protocol = |name| {
        package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration
    };
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        1_000_000,
        protocol("Duplicate"),
        protocol("Retire"),
    )
    .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 1_000_000).unwrap();
    assert!(!ir.contains("thread_local"));
    (
        ir.replace("define i32 @main()", "define i32 @unused_main()"),
        prepared,
    )
}

#[test]
fn c_entry_traps_or_delivers_the_declared_result() {
    let package = sysroot::compile(
        "
        trait Duplicate {} trait Retire { fn drop(*self); }
        ^attrs.abi(.c) fn flurry_boundary(v: u32) -> u32 { assumes v != 0; v }
        fn run() -> bool { flurry_boundary(7) == 7 }
    ",
    )
    .unwrap();
    let (ir, _) = checked_module(&package);
    for level in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let tools = luna::host::Toolchain {
            optimization: level,
            ..Default::default()
        };
        for arg in [0, 7] {
            let harness = format!(
                "{ir}\ndefine i32 @main() {{\n %v = call i32 @flurry_boundary(i32 {arg})\n %status = sub i32 %v, 7\n ret i32 %status\n}}"
            );
            let out = tools.execute_test(&harness, std::time::Duration::from_secs(5));
            if arg == 0 {
                let failure = out.unwrap_err();
                assert!(failure.contains("native test failed (signal"), "{failure}");
                assert!(failure.contains("runtime-check-failure"), "{failure}");
            } else {
                out.unwrap();
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
#[test]
fn a_migrated_task_keeps_its_pool_and_cancellation_identity() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         effect probe()->bool;
         fn relay()->#probe bool {probe()#}
         fn run()->bool {let f=relay; true}",
    );
    let (ir, prepared) = checked_module(&package);
    let index = prepared
        .graph()
        .functions
        .iter()
        .position(|f| f.function.name == "relay")
        .unwrap();
    let ir = format!(
        "{ir}
        declare i32 @pthread_create(ptr, ptr, ptr, ptr)
        declare i32 @pthread_join(i64, ptr)
        define {{ i1, ptr }} @probe(ptr %pool, ptr %task) {{
          %owner = call ptr @flurry_task_pool(ptr %task)
          %same = icmp eq ptr %owner, %pool
          %ctrl = call ptr @flurry_task_poll(ptr %task)
          %p = insertvalue {{ i1, ptr }} zeroinitializer, i1 %same, 0
          %answer = insertvalue {{ i1, ptr }} %p, ptr %ctrl, 1
          ret {{ i1, ptr }} %answer
        }}
        define ptr @worker(ptr %job) {{
          %input = load ptr, ptr %job
          %slot = getelementptr ptr, ptr %job, i32 1
          %pool = load ptr, ptr %slot
          %actual = call ptr @flurry_task_pool(ptr %input)
          %owns = icmp eq ptr %actual, %pool
          %r = call {{ i1, ptr }} @flurry_{index}(ptr @probe, ptr %pool, ptr %input)
          %v = extractvalue {{ i1, ptr }} %r, 0
          %ctrl = extractvalue {{ i1, ptr }} %r, 1
          %tag = select i1 %v, ptr %pool, ptr %ctrl
          %entry = call {{ i1, ptr }} @flurry_task_entry(ptr %input)
          %entry_ctrl = extractvalue {{ i1, ptr }} %entry, 1
          %same = icmp eq ptr %entry_ctrl, %ctrl
          %valid = and i1 %owns, %same
          %checked = select i1 %valid, ptr %tag, ptr null
          ret ptr %checked
        }}
        define i32 @main() {{
          %pool1 = alloca i8
          %pool2 = alloca i8
          %one = alloca %flurry.task
          %two = alloca %flurry.task
          call void @flurry_task_init(ptr %one, ptr %pool1)
          call void @flurry_task_init(ptr %two, ptr %pool2)
          call void @flurry_task_cancel(ptr %one)
          %job1 = alloca [2 x ptr]
          %job2 = alloca [2 x ptr]
          store ptr %one, ptr %job1
          store ptr %two, ptr %job2
          %slot1 = getelementptr ptr, ptr %job1, i32 1
          %slot2 = getelementptr ptr, ptr %job2, i32 1
          store ptr %pool1, ptr %slot1
          store ptr %pool2, ptr %slot2
          %thread = alloca i64
          %result = alloca ptr
          %created = call i32 @pthread_create(ptr %thread, ptr null, ptr @worker, ptr %job1)
          %started = icmp eq i32 %created, 0
          br i1 %started, label %join, label %fail
        join:
          %id = load i64, ptr %thread
          %joined = call i32 @pthread_join(i64 %id, ptr %result)
          %finished = icmp eq i32 %joined, 0
          br i1 %finished, label %check, label %fail
        check:
          %cancelled = load ptr, ptr %result
          %ours = icmp eq ptr %cancelled, %one
          %other = call ptr @worker(ptr %job2)
          %normal = icmp eq ptr %other, %pool2
          %ok = and i1 %ours, %normal
          %code = select i1 %ok, i32 0, i32 1
          ret i32 %code
        fail:
          ret i32 2
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
fn indirect_calls_share_one_entry_shape_across_certifications() {
    let package = support::compile(
        "
        trait Duplicate {} trait Retire {fn drop(*self);}
        fn total(flag: bool) -> bool {flag}
        fn guarded(flag: bool) -> bool {assumes flag; flag}
        fn apply(f: fn(bool)->bool, flag: bool) -> bool {f(flag)}
        fn run() -> bool {apply(total,true) and apply(guarded,true)}",
    );
    let (ir, prepared) = checked_module(&package);
    let id = |name| {
        prepared
            .graph()
            .functions
            .iter()
            .position(|f| f.function.name == name)
            .unwrap()
    };
    let index = id("apply");
    let total = id("total");
    let guarded = id("guarded");
    assert!(ir.contains(&format!("define internal i1 @flurry_{total}_scalar(")));
    assert!(!ir.contains(&format!("define internal i1 @flurry_{guarded}_scalar(")));
    let with_main = format!(
        "{ir}
        define i32 @main() {{
          %task = alloca %flurry.task
          call void @flurry_task_init(ptr %task, ptr null)
          %a = call {{i1, ptr}} @flurry_{index}(ptr @flurry_{total}, i1 false, ptr %task)
          %b = call {{i1, ptr}} @flurry_{index}(ptr @flurry_{guarded}, i1 false, ptr %task)
          %ca = extractvalue {{i1, ptr}} %a, 1
          %cb = extractvalue {{i1, ptr}} %b, 1
          %normal = icmp eq ptr %ca, null
          %failed = icmp eq ptr %cb, inttoptr (i32 1 to ptr)
          %ok = and i1 %normal, %failed
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
        .execute_test(&with_main, std::time::Duration::from_secs(5))
        .unwrap();
    }
}
