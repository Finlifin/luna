//! End-to-end tests for pooled-stack `Abi::Capturing` effects and
//! `builtin.Continuation` (`resume`, `resume_raw`, `abort_raw`, reverse `Drop`
//! unwinding, address-stable pooled stacks, and panic propagation) at `-O0`/`-O3`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-cap-cont-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn sysroot() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
    }

    fn build(&self, entry: &str, opt_level: &str) -> Output {
        let mut cmd = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"));
        cmd.current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                entry,
                "--output",
                "program",
                "--opt-level",
                opt_level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(Self::sysroot());
        cmd.output().unwrap()
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

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const PREAMBLE: &str = r#"
use std.prelude.*;
use std.os.linux;

fn cont_resume_raw(k: builtin.Continuation, arg_addr: usize, out_addr: usize) -> usize {
    unsafe { k.resume_raw(arg_addr, out_addr) }
}

fn cont_abort_raw(k: builtin.Continuation) {
    unsafe { k.abort_raw(); }
}

fn cont_crosses_c(k: *builtin.Continuation) -> bool {
    k.crosses_c_frame()
}

fn cont_thread_id(k: *builtin.Continuation) -> u64 {
    k.origin_thread_id()
}

fn addr_of(p: *T) -> usize where T {
    unsafe { builtin.mem.ptr_addr(p) }
}

fn trigger_panic(msg: str) -> NoReturn {
    builtin.control.runtime_panic(msg)
}
"#;

#[test]
fn capturing_effect_resume_and_cross_function_calls_execute_at_o0_and_o3() {
    let source = format!(
        r#"{PREAMBLE}
        effect yield_number(v: u32, catch continuation: builtin.Continuation);
        effect ask_step(x: u32, catch continuation: builtin.Continuation) -> u32;
        effect emit_item(v: u32, catch continuation: builtin.Continuation);

        fn helper(base: u32) -> #[yield_number, ask_step] u32
            where requires base < 1000
        {{
            yield_number(base)#;
            let next = ask_step(base + 2)#;
            next
        }}

        fn produce(log: *u32) -> #[yield_number, ask_step, emit_item] u32 {{
            var pinned: u64 = 12345;
            let addr_before = addr_of(pinned.ref);
            let a = helper(10);
            emit_item(a)#;
            let addr_after = addr_of(pinned.ref);
            if addr_before == addr_after and pinned == 12345 and a < 1000 and log.* < 10000 {{
                log.* = log.* + a;
                a + 100
            }} else {{
                0
            }}
        }}

        fn run() -> bool {{
            var prod_log: u32 = 0;
            var handler_log: u32 = 0;
            let out = produce(prod_log.ref)# {{
                yield_number(v, _k) => {{
                    if handler_log < 10000 and v < 1000 {{ handler_log = handler_log + v; }}
                    resume;
                    0
                }},
                ask_step(x, _k) => {{
                    resume (if x < 1000 {{ x * 2 }} else {{ 0 }});
                    0
                }},
                emit_item(v, _k) => {{
                    if handler_log < 10000 and v < 1000 {{ handler_log = handler_log + v; }}
                    resume;
                    0
                }},
            }};
            -- yield_number(10) -> handler_log += 10
            -- ask_step(12) -> resume 24 -> a = 24
            -- Stream.emit_item(24) -> handler_log += 24
            -- produce: prod_log += 24 -> prod_log = 24, handler_log = 34, out = 124
            out == 124 and prod_log == 24 and handler_log == 34
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "(bool true)");
    }
}

#[test]
fn capturing_effect_abandon_and_abort_raw_run_reverse_drop_on_suspended_stack() {
    let source = format!(
        r#"{PREAMBLE}
        effect pause(v: u32, catch continuation: builtin.Continuation) -> u32;

        struct TraceDrop {{
            tag: u32,
        }}
        impl Drop for TraceDrop {{
            fn drop(*self) {{
                let t = self.tag.*;
                if t == 1 {{ let _ = linux.write_str(2, "drop:1\n", 0); }}
                else if t == 2 {{ let _ = linux.write_str(2, "drop:2\n", 0); }}
                else {{ let _ = linux.write_str(2, "drop:0\n", 0); }}
            }}
        }}

        fn deep_suspend() -> #pause u32 {{
            let d1 = TraceDrop {{ tag: 1 }};
            let d2 = TraceDrop {{ tag: 2 }};
            let _ = pause(7)#;
            999
        }}

        fn outer_suspend() -> #pause u32 {{
            let d0 = TraceDrop {{ tag: 0 }};
            deep_suspend()
        }}

        typealias OptCont = ?builtin.Continuation;

        fn outer_suspend_opt() -> #pause OptCont {{
            let d0 = TraceDrop {{ tag: 0 }};
            let _ = deep_suspend();
            null
        }}

        fn run() -> bool {{
            -- 1. Implicit abandon in arm (`_`) runs reverse Drop on suspended stack.
            let abandoned_val = outer_suspend()# {{
                pause(v, _) => if v < 1000 {{ v + 1 }} else {{ 0 }},
            }};

            -- 2. Explicit continuation escape + abort_raw also runs reverse Drop.
            let captured = outer_suspend_opt()# {{
                pause(_v, k) => k,
            }};
            let escaped_ok = captured match {{
                k? => {{
                    let tid = cont_thread_id(k.ref);
                    let no_c = not cont_crosses_c(k.ref);
                    cont_abort_raw(k);
                    tid != 0 and no_c
                }},
                null => false,
            }};

            abandoned_val == 8 and escaped_ok
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "(bool true)");
        let stderr = String::from_utf8_lossy(&run.stderr);
        let expected = "drop:2\ndrop:1\ndrop:0\ndrop:2\ndrop:1\ndrop:0\n";
        assert_eq!(stderr, expected, "opt={opt}: stderr was:\n{stderr}");
    }
}

#[test]
fn capturing_effect_nontail_resume_raw_and_escaped_resume_execute_at_o0_and_o3() {
    let source = format!(
        r#"{PREAMBLE}
        effect ask(x: u32, catch continuation: builtin.Continuation) -> u32;

        fn multi_step() -> #ask u32 {{
            let a = ask(1)#;
            let b = ask(2)#;
            if a < 1000 and b < 1000 {{ a + b }} else {{ 0 }}
        }}

        enum StepOrCont {{
            done(u32),
            pending(builtin.Continuation),
        }}

        fn single_step() -> #ask StepOrCont {{
            let x = ask(5)#;
            StepOrCont.done(if x < 1000 {{ x * 3 }} else {{ 0 }})
        }}

        fn run() -> bool {{
            -- 1. Non-tail resume_raw inside handler arm across nested suspensions.
            let nontail = multi_step()# {{
                ask(x, k) => {{
                    var y: u32 = if x < 1000 {{ x * 10 }} else {{ 0 }};
                    var out: u32 = 0;
                    let ctrl = cont_resume_raw(k, addr_of(y.ref), addr_of(out.ref));
                    if ctrl == 0 and out < 10000 {{ out + 1 }} else {{ 0 }}
                }},
            }};

            -- 2. Escaping continuation resumed outside the EffectHandle scope.
            let saved = single_step()# {{
                ask(_x, k) => StepOrCont.pending(k),
            }};
            var escaped_out: StepOrCont = StepOrCont.done(0);
            let escaped_ok = saved match {{
                StepOrCont.pending(k) => {{
                    var arg: u32 = 14;
                    let ctrl = cont_resume_raw(k, addr_of(arg.ref), addr_of(escaped_out.ref));
                    let finished = escaped_out;
                    let val_ok = finished match {{
                        StepOrCont.done(v) => v == 42,
                        StepOrCont.pending(unused_k) => {{
                            cont_abort_raw(unused_k);
                            false
                        }},
                    }};
                    ctrl == 0 and val_ok
                }},
                StepOrCont.done(_) => false,
            }};

            -- multi_step: ask(1)->10, ask(2)->20 => body=30 => inner arm=31 => outer arm=32.
            nontail == 32 and escaped_ok
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "(bool true)");
    }
}

#[test]
fn panic_inside_suspended_stack_unwinds_and_propagates_ctrl_word_one() {
    let source = format!(
        r#"{PREAMBLE}
        effect pause(catch continuation: builtin.Continuation);

        struct TraceDrop {{
            tag: u32,
        }}
        impl Drop for TraceDrop {{
            fn drop(*self) {{
                let t = self.tag.*;
                if t == 1 {{ let _ = linux.write_str(2, "drop:body1\n", 0); }}
                else if t == 2 {{ let _ = linux.write_str(2, "drop:body2\n", 0); }}
                else {{ let _ = linux.write_str(2, "drop:outer\n", 0); }}
            }}
        }}

        fn panicky_body() -> #pause u32 {{
            let g1 = TraceDrop {{ tag: 1 }};
            pause()#;
            let g2 = TraceDrop {{ tag: 2 }};
            trigger_panic("inside-cont")
        }}

        fn run() -> bool {{
            let outer = TraceDrop {{ tag: 0 }};
            let _ = panicky_body()# {{
                pause(_k) => {{
                    resume;
                    0
                }},
            }};
            true
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert!(
            !run.status.success(),
            "panic inside continuation must propagate"
        );
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(
            stderr.contains("panic: inside-cont\ndrop:body2\ndrop:body1\ndrop:outer\n"),
            "opt={opt}: expected reverse drop across pooled stack + outer stack:\n{stderr}"
        );
    }
}

#[test]
fn capturing_partial_handler_fallback_across_distinct_result_types_at_o0_and_o3() {
    let source = format!(
        r#"{PREAMBLE}
        effect ask(x: u32, catch continuation: builtin.Continuation) -> u32;
        effect ping(v: u32, catch continuation: builtin.Continuation) -> u32;

        struct OuterBox {{
            a: u64,
            b: u64,
            c: u64,
        }}

        struct TraceDrop {{
            tag: u32,
        }}
        impl Drop for TraceDrop {{
            fn drop(*self) {{
                let t = self.tag.*;
                if t == 1 {{ let _ = linux.write_str(2, "drop:inner\n", 0); }}
                else if t == 2 {{ let _ = linux.write_str(2, "drop:mid\n", 0); }}
                else {{ let _ = linux.write_str(2, "drop:outer\n", 0); }}
            }}
        }}

        fn inner_ask(x: u32) -> #ask bool {{
            let _g = TraceDrop {{ tag: 1 }};
            let y = ask(x)#;
            y == 10
        }}

        fn mid_stage(x: u32) -> #[ask, ping] OuterBox {{
            let _g = TraceDrop {{ tag: 2 }};
            let p = ping(1)#;
            let matched = inner_ask(x)# {{
                ask(0, _k) => {{
                    resume 10;
                    true
                }},
            }};
            if matched {{
                let a_val: u64 = if p == 5 {{ 5 }} else {{ 0 }};
                OuterBox {{ a: a_val, b: 10, c: 99 }}
            }} else {{
                OuterBox {{ a: 0, b: 0, c: 0 }}
            }}
        }}

        fn run_outer(x: u32) -> OuterBox {{
            let _g = TraceDrop {{ tag: 0 }};
            mid_stage(x)# {{
                ask(1, _k) => {{
                    resume 10;
                    OuterBox {{ a: 9, b: 9, c: 9 }}
                }},
                ask(_other, _k) => OuterBox {{ a: 77, b: 88, c: 99 }},
                ping(v, _k) => {{
                    resume (if v < 1000 {{ v + 4 }} else {{ 0 }});
                    OuterBox {{ a: 0, b: 0, c: 0 }}
                }},
            }}
        }}

        fn run() -> bool {{
            let r0 = run_outer(0);
            let r1 = run_outer(1);
            let r2 = run_outer(2);
            r0.a == 5 and r0.b == 10 and r0.c == 99
                and r1.a == 5 and r1.b == 10 and r1.c == 99
                and r2.a == 77 and r2.b == 88 and r2.c == 99
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "(bool true)");
        let stderr = String::from_utf8_lossy(&run.stderr);
        let expected = "\
drop:inner\n\
drop:mid\n\
drop:outer\n\
drop:inner\n\
drop:mid\n\
drop:outer\n\
drop:inner\n\
drop:mid\n\
drop:outer\n";
        assert_eq!(stderr, expected, "opt={opt}: stderr was:\n{stderr}");
    }
}

#[test]
fn std_runtime_typed_continuation_abort_and_drop_execute_at_o0_and_o3() {
    let source = format!(
        r#"{PREAMBLE}
        use std.runtime.TypedContinuation;

        effect yield_u32(v: u32, catch continuation: builtin.Continuation) -> u32;

        struct Guard {{
            id: u32,
        }}
        impl Drop for Guard {{
            fn drop(*self) {{
                if self.id.* == 1 {{
                    let _ = linux.write_str(2, "drop:guard1\n", 0);
                }} else {{
                    let _ = linux.write_str(2, "drop:guard2\n", 0);
                }}
            }}
        }}

        enum Step {{
            done(u32),
            paused(TypedContinuation<u32, Step>),
        }}

        fn body_with_drop(tag: u32) -> #yield_u32 Step {{
            let _g = Guard {{ id: tag }};
            let x = yield_u32(5)#;
            Step.done(if x < 1000 {{ x * 2 }} else {{ 0 }})
        }}

        fn run() -> bool {{
            -- 1. Escaped TypedContinuation inspected and resumed via inner Continuation
            let step1 = body_with_drop(1)# {{
                yield_u32(_v, k) => Step.paused(unsafe {{ TypedContinuation<u32, Step>.from_raw(k) }}),
            }};
            let ok1 = step1 match {{
                Step.paused(tc) => {{
                    let no_c = not tc.ref.crosses_c_frame();
                    let tid = tc.ref.origin_thread_id();
                    var arg: u32 = 21;
                    var out: Step = Step.done(0);
                    let raw_k = tc.into_raw();
                    let ctrl = cont_resume_raw(raw_k, addr_of(arg.ref), addr_of(out.ref));
                    let next = out;
                    let val_ok = next match {{
                        Step.done(v) => v == 42,
                        Step.paused(unused) => {{
                            unused.abort();
                            false
                        }},
                    }};
                    no_c and tid != 0 and ctrl == 0 and val_ok
                }},
                Step.done(_) => false,
            }};

            -- 2. Escaped TypedContinuation aborted explicitly via .abort()
            let step2 = body_with_drop(2)# {{
                yield_u32(_v, k) => Step.paused(unsafe {{ TypedContinuation<u32, Step>.from_raw(k) }}),
            }};
            let ok2 = step2 match {{
                Step.paused(tc) => {{
                    tc.abort();
                    true
                }},
                Step.done(_) => false,
            }};

            -- 3. Unconsumed TypedContinuation dropped via implicit Drop for Continuation
            do {{
                let _dropped = body_with_drop(2)# {{
                    yield_u32(_v, k) => Step.paused(unsafe {{ TypedContinuation<u32, Step>.from_raw(k) }}),
                }};
            }}

            ok1 and ok2
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "(bool true)");
        let stderr = String::from_utf8_lossy(&run.stderr);
        let expected = "drop:guard1\ndrop:guard2\ndrop:guard2\n";
        assert_eq!(stderr, expected, "opt={opt}: stderr was:\n{stderr}");
    }
}

#[test]
fn std_runtime_typed_continuation_resume_with_executes_at_o0_and_o3() {
    let source = format!(
        r#"{PREAMBLE}
        use std.runtime.TypedContinuation;

        effect yield_u32(v: u32, catch continuation: builtin.Continuation) -> u32;

        enum Step {{
            done(u32),
            paused(TypedContinuation<u32, Step>),
        }}

        fn body() -> #yield_u32 Step {{
            let x = yield_u32(5)#;
            Step.done(if x < 1000 {{ x * 2 }} else {{ 0 }})
        }}

        fn run() -> bool {{
            let step1 = body()# {{
                yield_u32(_v, k) => Step.paused(unsafe {{ TypedContinuation<u32, Step>.from_raw(k) }}),
            }};
            step1 match {{
                Step.paused(tc) => tc.resume_with(21) match {{
                    Step.done(v) => v == 42,
                    Step.paused(unused) => {{
                        unused.abort();
                        false
                    }},
                }},
                Step.done(_) => false,
            }}
        }}
        "#
    );

    let case = NativeCase::new(&source);
    for opt in ["0", "3"] {
        let build = case.build("run", opt);
        assert_success(&build);
        let run = case.run();
        assert_success(&run);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "(bool true)");
    }
}
