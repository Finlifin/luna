//! Black-box checks for development/runs/001-frontend-entry/contract.md.
//! The fixtures use grammar/syntax function, block, literal, use and if forms.
//! No fixture requires a standard-library API or completed type checking.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const VALID: &str = "fn requested_entry() {}\n";
const OLD_OUTPUT: &[u8] = b"previous output: preserve these bytes\n\0";

#[test]
fn derivation_strategy_hir_preserves_generation_inputs_and_members() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait Label {fn label(self)->bool;} derive fn configured(flag:bool) for Label where T {assoc Input:.Type'builtin = T; fn label(self)->bool {flag}} fn run() {}");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert!(dump.contains("(deriver "), "{dump}");
    assert!(dump.contains("(name \"configured\")"), "{dump}");
    assert!(
        dump.contains("(resolved-fn-param (name \"flag\")"),
        "{dump}"
    );
    assert!(dump.contains("(associated "), "{dump}");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    failure(&output, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("derivation expansion"));
}

#[test]
fn derivation_requests_preserve_phase_strategy_and_subject_without_impl_evidence() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait Label {} struct Record {} derive fn configured for Label {} derive Label.use(configured) for Record; comptime derive Label for Record where T;");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(derive-request ").count(), 2, "{dump}");
    assert!(dump.contains("(phase runtime)"), "{dump}");
    assert!(dump.contains("(phase comptime)"), "{dump}");
    assert!(dump.contains("(strategy (some "), "{dump}");
    assert!(!dump.contains("(impl (definition"), "{dump}");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    failure(&output, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("derivation expansion"));
}

#[test]
fn derivation_strategy_bindings_do_not_escape_or_alias() {
    for source in [
        "trait Label {} derive fn configured(flag:bool,flag:bool) for Label {}",
        "trait Label {} derive fn configured(flag:bool) for Label {} fn leak()->bool {flag}",
        "trait Label {} derive fn configured for Label {} derive fn configured for Label {}",
        "derive fn configured for Missing {}",
        "trait Label {} derive Label.use(missing) for Missing;",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        failure(&output, 1);
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("panicked"),
            "{source}"
        );
    }
}

#[test]
fn pattern_code_hir_preserves_interface_quote_and_fresh_success_outputs() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "fn run(value:bool)->bool {
        const pattern:<bool -| flag:bool> = '[flag=bound]<bound>;
        let ~[flag=first]<pattern> = value;
        let ~[flag=second]<pattern> = value;
        first and second
    }",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert!(dump.contains("(contextual-pattern-type "), "{dump}");
    assert!(dump.contains("(code-pattern-quote "), "{dump}");
    assert_eq!(dump.matches("(code-pattern-splice ").count(), 2, "{dump}");
    assert!(dump.contains("(resolved-local (name \"first\")"), "{dump}");
    assert!(dump.contains("(resolved-local (name \"second\")"), "{dump}");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    success(&output);
}

#[test]
fn pattern_code_bindings_are_scoped_to_quote_or_match_success() {
    for source in [
        "fn run() {const pattern='[flag=missing]<bound>;}",
        "fn run()->bool {const pattern='<bound>;bound}",
        "fn run(value:bool)->bool {const pattern='<bound>;value match {~[flag=found]<pattern> => found,_ => found}}",
        "fn run(value:bool) {const pattern='<bound>;let (~[flag=same]<pattern>,same)=(value,value);}",
        "fn run(value:bool) {const pattern:<missing -| missing:bool> = '<bound>;}",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        failure(&output, 1);
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("panicked"),
            "{source}"
        );
    }
}

#[test]
fn inline_control_flow_keeps_its_expansion_requirement_in_hir() {
    let sandbox = Sandbox::new();
    for (body, control) in [
        ("inline if true {} else {}", "(if "),
        ("inline when {true=>{},else=>{},}", "(when "),
        ("inline if true is true do {} else {}", "(match "),
        ("inline for item in (true,false) {let used=item;}", "(for "),
    ] {
        sandbox.write("input.fl", &format!("fn run() {{{body}}}"));
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        let dump = success(&output);
        assert!(dump.contains("(inline-control "), "{body}: {dump}");
        assert!(dump.contains(control), "{body}: {dump}");
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        if control == "(if " || control == "(when " || control == "(match " {
            success(&output);
            continue;
        }
        failure(&output, 1);
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("inline for currently requires a bounded comptime Integer range"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    sandbox.write("input.fl", "fn run() {inline true;}");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    failure(&output, 1);
}

#[test]
fn function_type_telescope_bindings_are_ordered_nested_and_scoped() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "typealias Type=.Type'builtin; typealias T=bool;
        typealias Callback=fn(comptime T:Type,value:T)->fn(comptime T:Type,other:T)->T;
        typealias After=T;",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = success(&output);
    let targets = dump
        .split("(resolved-local (name \"T\") (target ")
        .skip(1)
        .map(|tail| tail.split("))").next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 3, "{dump}");
    assert_ne!(
        targets[0], targets[1],
        "nested T must have a fresh identity: {dump}"
    );
    assert_eq!(
        targets[1], targets[2],
        "result must use the inner T: {dump}"
    );
    for source in [
        "typealias Callback=fn(comptime T:.Type'builtin)->T; typealias Leaked=T;",
        "typealias Callback=fn(comptime T:.Type'builtin); typealias Leaked=T;",
        "typealias Callback=fn(value:T,comptime T:.Type'builtin)->T;",
        "typealias Callback=fn(comptime T:.Type'builtin,comptime T:.Type'builtin)->T;",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        failure(&output, 1);
        assert!(
            !details(&output).contains("panicked"),
            "{}",
            details(&output)
        );
    }
}

#[test]
fn function_type_parameters_keep_names_modifiers_and_optional_syntax() {
    let sandbox = Sandbox::new();
    for (parameter, flags, shape) in [
        ("implicit bool", 1, "positional"),
        ("quote bool", 4, "positional"),
        ("error bool", 8, "positional"),
        ("lambda bool", 16, "positional"),
        ("assoc bool", 32, "positional"),
        ("catch bool", 64, "positional"),
        ("implicit comptime bool", 3, "positional"),
        ("value:bool", 0, "positional-dependency-catched"),
        (".value:bool", 128, "positional-dependency-catched"),
        ("comptime .value:bool", 130, "positional-dependency-catched"),
        ("...values:bool", 0, "variadic"),
        ("itself", 0, "itself"),
        ("*itself", 0, "itself"),
    ] {
        sandbox.write(
            "input.fl",
            &format!("typealias Callback=fn({parameter})->bool; fn entry() {{}}"),
        );
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        let dump = success(&output);
        assert!(
            dump.contains(&format!("(flags {flags})")),
            "{parameter}: {dump}"
        );
        assert!(dump.contains(&format!("({shape} ")), "{parameter}: {dump}");
        if parameter.contains("value") {
            assert!(
                dump.contains("(name \"value") || dump.contains("(name \"values"),
                "{parameter}: {dump}"
            );
        }
        if parameter.contains("itself") {
            assert!(
                dump.contains(if parameter.starts_with('*') {
                    "(ref true)"
                } else {
                    "(ref false)"
                }),
                "{dump}"
            );
        }
        // Retaining syntax must not silently authorize an unsupported callable ABI.
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        if parameter == "value:bool" {
            success(&output);
            continue;
        }
        failure(&output, 1);
        assert!(
            details(&output).contains("function type parameter"),
            "{parameter}: {}",
            details(&output)
        );
    }
    sandbox.write("input.fl", "typealias Callback=fn(.value:bool)->bool;");
    let output = sandbox.run(&["input.fl", "--emit", "ast"]);
    assert!(success(&output).contains("OptionalTypeParam"));
    sandbox.write("input.fl", "typealias Callback=fn(value:bool)->bool;");
    let output = sandbox.run(&["input.fl", "--emit", "ast"]);
    assert!(!success(&output).contains("OptionalTypeParam"));
}

#[test]
fn tuple_types_remain_explicit_through_both_cli_drivers() {
    let sandbox = Sandbox::new();
    let declarations = "typealias Type=.Type'builtin;
        pure comptime fn List<T> -> Type {T}
        pure comptime fn Tuple<...Es:List<Type>> -> Type {bool}
        trait Duplicate {} trait Retire {fn drop(*self);}";
    for mut base in [
        vec!["input.fl"],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "17",
            "input",
            "input.fl",
            "--primary",
            "1",
        ],
    ] {
        base.extend([
            "--meta-list-type",
            "input.List",
            "--tuple-type",
            "input.Tuple",
        ]);
        for invalid in [
            "fn entry(value:(bool,bool)) {}",
            "fn entry()->(bool,) {(true,)}",
            "typealias Wrong=(); fn entry() {}",
            "typealias Wrong=Tuple<(bool,bool)>; fn entry() {}",
            "comptime fn Wrong(T:Type)->Type {(T,bool)} fn entry() {}",
        ] {
            sandbox.write("input.fl", &format!("{declarations} {invalid}"));
            sandbox.write("result.lisp", "previous artifact");
            let mut args = base.clone();
            args.extend(["--emit", "nir", "--output", "result.lisp"]);
            let output = sandbox.run(&args);
            failure(&output, 1);
            assert!(
                details(&output).contains("value, not a type"),
                "{invalid}: {}",
                details(&output)
            );
            assert_eq!(
                fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
                "previous artifact"
            );
        }
        sandbox.write(
            "input.fl",
            &format!(
                "{declarations}
            typealias Flag=((bool));
            typealias Pair=(Tuple<bool,bool>);
            fn entry()->Flag {{
                let pair:Pair=(true,false);
                let (a,b)=pair;
                let singleton:Tuple<bool> = (a,);
                let (saved,)=singleton;
                let empty:Tuple<> = ();
                saved and not b
            }}"
            ),
        );
        let mut args = base.clone();
        args.extend([
            "--interpret",
            "entry",
            "--copy-protocol",
            "input.Duplicate",
            "--drop-protocol",
            "input.Retire",
        ]);
        let output = sandbox.run(&args);
        assert_eq!(success(&output).trim(), "(bool true)");
        args.push("--inline-functions");
        let output = sandbox.run(&args);
        assert_eq!(success(&output).trim(), "(bool true)");
    }
}

#[test]
fn tuple_selectors_work_in_single_file_and_sysroot_commands() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "typealias Type=.Type'builtin;
        pure comptime fn List<T> -> Type {T}
        pure comptime fn Tuple<...Es:List<Type>> -> Type {bool}
        fn pair()->Tuple<bool,.u8'builtin> {(true,9)}
        fn run()->bool {let (flag,byte)=pair();flag and byte==9}",
    );
    let output = sandbox.run(&[
        "input.fl",
        "--interpret",
        "run",
        "--meta-list-type",
        "input.List",
        "--tuple-type",
        "input.Tuple",
    ]);
    assert_eq!(success(&output).trim(), "(bool true)");
    for args in [
        vec!["input.fl", "--tuple-type", "input.Tuple"],
        vec![
            "input.fl",
            "--emit",
            "ast",
            "--meta-list-type",
            "input.List",
        ],
        vec![
            "input.fl",
            "--interpret",
            "run",
            "--tuple-type",
            "input.Tuple",
            "--tuple-type",
            "input.Tuple",
        ],
        vec!["input.fl", "--interpret", "run", "--meta-list-type"],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    let missing = sandbox.run(&[
        "input.fl",
        "--interpret",
        "run",
        "--tuple-type",
        "input.Tuple",
    ]);
    failure(&missing, 1);
    assert!(String::from_utf8_lossy(&missing.stderr).contains("meta.List binding"));

    for name in ["builtin", "std", "meta"] {
        fs::create_dir_all(sandbox.path(&format!("library/{name}"))).unwrap();
    }
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {} use meta.List;
        typealias Type=.Type'builtin; pub pure comptime fn Tuple<...Es:List<Type>> -> Type {bool}",
    );
    sandbox.write("library/std/lib.fl", "");
    sandbox.write(
        "library/meta/lib.fl",
        "typealias Type=.Type'builtin; pub pure comptime fn List<T> -> Type {T} pub mod prelude {}",
    );
    sandbox.write(
        "input.fl",
        "use builtin.Tuple; fn run()->Tuple<bool,.u8'builtin> {(true,9)}",
    );
    let output = sandbox.run(&[
        "input.fl",
        "--sysroot",
        "library",
        "--interpret",
        "run",
        "--meta-list-type",
        "meta.List",
        "--tuple-type",
        "builtin.Tuple",
    ]);
    assert_eq!(
        success(&output).trim(),
        "(tuple (bool true) (unsigned-integer 9))"
    );
}

#[test]
fn splice_selector_runs_automatic_conversion_and_rejects_invalid_bindings() {
    let sandbox = Sandbox::new();
    let run = |args: &[&str]| {
        let mut args = args.to_vec();
        args.extend([
            "--meta-list-type",
            "input.TupleElements",
            "--tuple-type",
            "input.Tuple",
        ]);
        sandbox.run(&args)
    };
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/automatic-splice.fl"),
    );
    for selector in ["Splice", "input.Splice"] {
        let output = run(&[
            "input.fl",
            "--interpret",
            "run",
            "--splice-protocol",
            selector,
        ]);
        assert_eq!(
            success(&output).trim(),
            "(tuple (tuple (bool true) (bool false)) (tuple (bool false) (bool true)))"
        );
    }
    failure(&run(&["input.fl", "--interpret", "run"]), 1);
    for selector in [
        "Missing",
        "missing.Splice",
        "input.",
        ".Splice",
        "input.Splice.extra",
    ] {
        sandbox.write("result", OLD_OUTPUT);
        failure(
            &run(&[
                "input.fl",
                "--interpret",
                "run",
                "--splice-protocol",
                selector,
                "--output",
                "result",
            ]),
            1,
        );
        assert_eq!(fs::read(sandbox.path("result")).unwrap(), OLD_OUTPUT);
    }
    for args in [
        vec!["input.fl", "--splice-protocol"],
        vec!["input.fl", "--splice-protocol", "Splice"],
        vec!["input.fl", "--emit", "ast", "--splice-protocol", "Splice"],
        vec![
            "input.fl",
            "--interpret",
            "run",
            "--splice-protocol",
            "Splice",
            "--splice-protocol",
            "Splice",
        ],
    ] {
        failure(&run(&args), 2);
    }
    sandbox.write("input.fl", "trait NotSplice {} fn run()->bool {true}");
    let output = sandbox.run(&[
        "input.fl",
        "--interpret",
        "run",
        "--splice-protocol",
        "NotSplice",
    ]);
    failure(&output, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("RuntimeTarget"));
}

#[test]
fn splice_selector_disambiguates_sysroot_protocols() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write("library/builtin/lib.fl", "pub mod prelude {}");
    sandbox.write("library/std/lib.fl", "typealias Type=.Type'builtin;
        pub trait Conversion {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;}
        impl Conversion for bool {assoc RuntimeTarget:Type=bool;fn splice(comptime data:bool)->bool {not data}}
        pub const FLAG=false;");
    sandbox.write(
        "input.fl",
        "use std.FLAG; trait Conversion {} fn run()->bool {FLAG}",
    );
    let output = sandbox.run(&[
        "input.fl",
        "--sysroot",
        "library",
        "--interpret",
        "run",
        "--splice-protocol",
        "std.Conversion",
    ]);
    assert_eq!(success(&output).trim(), "(bool true)");
    let ambiguous = sandbox.run(&[
        "input.fl",
        "--sysroot",
        "library",
        "--interpret",
        "run",
        "--splice-protocol",
        "Conversion",
    ]);
    failure(&ambiguous, 1);
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("ambiguous"));
}

#[test]
fn splice_selector_api_rejects_conflicting_identity_and_nonsemantic_output() {
    use luna::{
        cli::{Emit, Options},
        frontend,
    };
    let sandbox = Sandbox::new();
    let mut options = Options {
        comptime_intrinsics: Vec::new(),
        input: sandbox.write(
            "input.fl",
            include_str!("../../../development/examples/automatic-splice.fl"),
        ),
        emit: Emit::Nir,
        output: None,
        build_modes: Default::default(),
        build_targets: Default::default(),
        sysroot: None,
        splice_protocol: Some("Splice".into()),
        meta_list_type: None,
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        iterator_protocol: None,
        into_iterator_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
        tuple_type: None,
    };
    let foreign = elaboration::program::Protocols {
        splice: Some(nir::DeclarationId {
            package: 999,
            local: 0,
        }),
        ..Default::default()
    };
    let error = frontend::compile_in_package_with_protocols(&options, 0, &foreign)
        .err()
        .expect("conflicting binding was accepted");
    assert!(error.contains("conflicts"), "{error}");
    for emit in [Emit::Ast, Emit::Hir] {
        options.emit = emit;
        let error = frontend::compile(&options)
            .err()
            .expect("selector was silently ignored");
        assert!(error.contains("requires NIR"), "{error}");
    }
}

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        // create_dir, rather than create_dir_all, ensures an old run cannot
        // silently supply files to this run. Each subprocess has its own cwd.
        loop {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("luna-frontend-cli-{}-{serial}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create test directory {}: {error}", path.display()),
            }
        }
    }

    fn path(&self, name: impl AsRef<Path>) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
        command
            .current_dir(&self.0)
            .env_remove("FLURRY_SYSROOT")
            .env("NO_COLOR", "1")
            .env("RUST_BACKTRACE", "0");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        run_command(self.command().args(args))
    }
}

fn run_command(command: &mut Command) -> Output {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A concurrent fork may briefly retain the just-closed executable writer.
    // Retry only failed exec, never a child that has actually started.
    let launch = Instant::now();
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error)
                if error.raw_os_error() == Some(26)
                    && launch.elapsed() < Duration::from_millis(100) =>
            {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("cannot start CLI/native test: {error}"),
        }
    };
    // Drain both pipes while waiting: large diagnostics must not deadlock on a
    // full OS pipe. Keep bounded evidence even if error recovery loops noisily.
    fn drain(mut pipe: impl Read) -> Vec<u8> {
        const EVIDENCE_LIMIT: usize = 1024 * 1024;
        let mut evidence = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => return evidence,
                Ok(count) => {
                    let keep = count.min(EVIDENCE_LIMIT.saturating_sub(evidence.len()));
                    evidence.extend_from_slice(&buffer[..keep]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => panic!("cannot collect CLI output: {error}"),
            }
        }
    }
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_reader = thread::spawn(move || drain(stdout));
    let stderr_reader = thread::spawn(move || drain(stderr));
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            // Reap even when kill races with normal process completion.
            let _ = child.kill();
            break child.wait().unwrap();
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = Output {
        status,
        stdout: stdout_reader.join().unwrap(),
        stderr: stderr_reader.join().unwrap(),
    };
    assert!(
        !timed_out,
        "CLI exceeded 15 seconds: {command:?}\n{}",
        details(&output)
    );
    output
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn details(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn success(output: &Output) -> &str {
    assert_eq!(output.status.code(), Some(0), "{}", details(output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked at"), "{}", details(output));
    std::str::from_utf8(&output.stdout).expect("dump must be UTF-8")
}

fn failure(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", details(output));
    assert!(output.stdout.is_empty(), "{}", details(output));
    assert!(!output.stderr.is_empty(), "{}", details(output));
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("panicked at"),
        "{}",
        details(output)
    );
}

// This deliberately does not prescribe node names, indentation or field order.
// It rejects progress text outside the tree and balances parentheses while
// respecting escaped quotes, backslashes and parentheses inside strings.
fn single_sexpression(text: &str) {
    let text = text.trim();
    assert!(text.starts_with('('), "not a tree: {text}");
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in text.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else {
            match character {
                '"' => quoted = true,
                '(' => depth += 1,
                ')' => {
                    assert!(depth > 0, "unmatched ')': {text}");
                    depth -= 1;
                    if depth == 0 {
                        assert_eq!(offset + 1, text.len(), "text after root: {text}");
                    }
                }
                _ => {}
            }
        }
    }
    assert!(!quoted && !escaped, "unterminated string: {text}");
    assert_eq!(depth, 0, "unbalanced tree: {text}");
}

fn hir_dump(output: &Output) -> &str {
    let text = success(output);
    single_sexpression(text);
    assert!(text.trim_start().starts_with("(hir-package"), "{text}");
    text
}

fn ast_dump(output: &Output) -> &str {
    let text = success(output);
    single_sexpression(text);
    assert!(!text.trim_start().starts_with("(hir-package"), "{text}");
    text
}

#[test]
fn help_succeeds_without_reading_an_input() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["--help"]);
    let help = success(&output);
    for option in ["--emit", "--output", "--sysroot", "--help"] {
        assert!(help.contains(option), "missing {option}: {help}");
    }
    assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 0);
}

#[test]
fn missing_input_is_a_usage_error() {
    let sandbox = Sandbox::new();
    for args in [vec![], vec!["--emit", "ast"], vec!["--output", "dump.lisp"]] {
        failure(&sandbox.run(&args), 2);
        assert!(!sandbox.path("dump.lisp").exists());
    }
}

#[test]
fn unknown_options_and_invalid_emit_are_usage_errors() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    for args in [
        vec!["input.fl", "--unknown"],
        vec!["--unknown", "input.fl"],
        vec!["input.fl", "--emit", "llvm"],
        vec!["input.fl", "--emit", ""],
    ] {
        failure(&sandbox.run(&args), 2);
    }
}

#[test]
fn missing_option_values_are_usage_errors() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    for option in ["--emit", "--output", "--sysroot"] {
        failure(&sandbox.run(&["input.fl", option]), 2);
    }
}

#[test]
fn duplicate_input_or_options_are_usage_errors() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    sandbox.write("other.fl", VALID);
    sandbox.write("dump.lisp", OLD_OUTPUT);
    for args in [
        vec!["input.fl", "other.fl"],
        vec!["input.fl", "input.fl"],
        vec!["input.fl", "--emit", "ast", "--emit", "ast"],
        vec!["input.fl", "--emit", "ast", "--emit", "hir"],
        vec![
            "input.fl",
            "--output",
            "dump.lisp",
            "--output",
            "other.lisp",
        ],
        vec!["input.fl", "--sysroot", "library", "--sysroot", "library"],
    ] {
        failure(&sandbox.run(&args), 2);
        assert_eq!(fs::read(sandbox.path("dump.lisp")).unwrap(), OLD_OUTPUT);
        assert!(!sandbox.path("other.lisp").exists());
    }
}

#[test]
fn explicit_input_defaults_to_hir_on_stdout() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    let output = sandbox.run(&["input.fl"]);
    assert!(hir_dump(&output).contains("requested_entry"));
    assert_eq!(
        fs::read(sandbox.path("input.fl")).unwrap(),
        VALID.as_bytes()
    );
    for name in ["test.fl", "ast.lisp", "hir.lisp"] {
        assert!(
            !sandbox.path(name).exists(),
            "unexpected implicit file {name}"
        );
    }
}

#[test]
fn ast_selection_outputs_only_the_requested_tree() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    let output = sandbox.run(&["input.fl", "--emit", "ast"]);
    assert!(ast_dump(&output).contains("requested_entry"));
}

#[test]
fn explicit_hir_matches_the_default() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    let default = sandbox.run(&["input.fl"]);
    let explicit = sandbox.run(&["input.fl", "--emit", "hir"]);
    hir_dump(&default);
    hir_dump(&explicit);
    assert_eq!(default.stdout, explicit.stdout);
}

#[test]
fn empty_input_does_not_require_an_optional_sysroot() {
    let sandbox = Sandbox::new();
    sandbox.write("empty.fl", "");
    hir_dump(&sandbox.run(&["empty.fl"]));
    ast_dump(&sandbox.run(&["empty.fl", "--emit", "ast"]));
}

#[test]
fn unicode_and_spaces_work_in_input_and_output_paths() {
    let sandbox = Sandbox::new();
    let input = sandbox.write("输入 文件.fl", VALID);
    let target = sandbox.path("输出 树.lisp");
    let output = run_command(
        sandbox
            .command()
            .arg(&input)
            .arg("--emit")
            .arg("ast")
            .arg("--output")
            .arg(&target),
    );
    assert!(success(&output).is_empty(), "{}", details(&output));
    let tree = fs::read_to_string(target).unwrap();
    single_sexpression(&tree);
    assert!(tree.contains("requested_entry"));
    assert_eq!(fs::read(input).unwrap(), VALID.as_bytes());
}

#[test]
fn explicit_output_replaces_old_dump_only_after_success() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    for emit in ["hir", "ast"] {
        let expected = sandbox.run(&["input.fl", "--emit", emit]);
        success(&expected);
        single_sexpression(std::str::from_utf8(&expected.stdout).unwrap());
        sandbox.write("dump.lisp", OLD_OUTPUT);
        let actual = sandbox.run(&["input.fl", "--emit", emit, "--output", "dump.lisp"]);
        assert!(success(&actual).is_empty(), "{}", details(&actual));
        assert_eq!(
            fs::read(sandbox.path("dump.lisp")).unwrap(),
            expected.stdout
        );
    }
}

#[test]
fn legacy_filenames_are_neither_implicit_input_nor_implicit_outputs() {
    let sandbox = Sandbox::new();
    sandbox.write("chosen.fl", VALID);
    // Even an unreadable-as-source legacy input must not affect chosen.fl.
    sandbox.write("test.fl", b"\xff\x00legacy input");
    sandbox.write("ast.lisp", b"old AST\x00");
    sandbox.write("hir.lisp", b"old HIR\x00");
    for emit in ["hir", "ast"] {
        let output = sandbox.run(&["chosen.fl", "--emit", emit]);
        assert!(success(&output).contains("requested_entry"));
        single_sexpression(std::str::from_utf8(&output.stdout).unwrap());
        assert_eq!(
            fs::read(sandbox.path("chosen.fl")).unwrap(),
            VALID.as_bytes()
        );
        assert_eq!(
            fs::read(sandbox.path("test.fl")).unwrap(),
            b"\xff\x00legacy input"
        );
        assert_eq!(fs::read(sandbox.path("ast.lisp")).unwrap(), b"old AST\x00");
        assert_eq!(fs::read(sandbox.path("hir.lisp")).unwrap(), b"old HIR\x00");
    }
}

fn stage_failure_preserves_outputs(source: &str, emissions: &[&str]) {
    let sandbox = Sandbox::new();
    sandbox.write("invalid.fl", source);
    for emit in emissions {
        failure(&sandbox.run(&["invalid.fl", "--emit", emit]), 1);
        sandbox.write("existing.lisp", OLD_OUTPUT);
        failure(
            &sandbox.run(&["invalid.fl", "--emit", emit, "--output", "existing.lisp"]),
            1,
        );
        assert_eq!(fs::read(sandbox.path("existing.lisp")).unwrap(), OLD_OUTPUT);
        failure(
            &sandbox.run(&["invalid.fl", "--emit", emit, "--output", "new.lisp"]),
            1,
        );
        assert!(!sandbox.path("new.lisp").exists());
        assert_eq!(
            fs::read(sandbox.path("invalid.fl")).unwrap(),
            source.as_bytes()
        );
    }
}

#[test]
fn lexical_failure_never_publishes_a_dump() {
    stage_failure_preserves_outputs("const BAD = \"unterminated", &["ast", "hir"]);
}

#[test]
fn parse_failure_never_publishes_a_dump() {
    stage_failure_preserves_outputs("fn broken(\n", &["ast", "hir"]);
}

#[test]
fn resolve_failure_never_publishes_hir() {
    stage_failure_preserves_outputs("use package_that_does_not_exist;\n", &["hir"]);
}

#[test]
fn unresolved_function_body_never_publishes_hir() {
    // Body names are diagnosed while the HIR query lowers the function. A
    // successful module scan must not hide errors emitted by this later work.
    stage_failure_preserves_outputs("fn broken() { unknown_value; }\n", &["hir"]);
}

#[test]
fn old_view_lowers_to_hir_but_rejects_noncontract_use() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn f() { 1'old; }\n");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("(take-old")
    );
    stage_failure_preserves_outputs("fn f() { 1'old; }\n", &["nir"]);
}

#[test]
fn lowering_invalid_test_definition_preserves_outputs() {
    stage_failure_preserves_outputs("test sample {missing;}\n", &["hir"]);
}

#[test]
fn lowering_invalid_builtin_operand_preserves_outputs() {
    stage_failure_preserves_outputs("fn f() { 1'builtin; }\n", &["hir"]);
}

#[test]
fn lowering_rejection_fixtures_are_valid_ast() {
    let sandbox = Sandbox::new();
    for (source, payload) in [
        ("fn f() { 1'old; }\n", "old"),
        ("test sample {missing;}\n", "sample"),
        ("fn f() { 1'builtin; }\n", "builtin"),
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "ast"]);
        assert!(ast_dump(&output).contains(payload));
    }
}

// Read one complete node so operand assertions cannot accidentally match a
// sibling expression or the declaration elsewhere in the package dump.
fn hir_form<'a>(tree: &'a str, tag: &str) -> &'a str {
    let prefix = format!("({tag} ");
    let start = tree
        .find(&prefix)
        .unwrap_or_else(|| panic!("missing {tag}: {tree}"));
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in tree[start..].char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else {
            match character {
                '"' => quoted = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return &tree[start..start + offset + 1];
                    }
                }
                _ => {}
            }
        }
    }
    panic!("unterminated {tag}: {tree}");
}

#[test]
fn lowering_recognized_views_keep_their_operand_in_hir() {
    let sandbox = Sandbox::new();
    // This checks AST-to-HIR routing only. It does not assert that applying
    // multi_deref to an integer passes a later type-checking phase.
    sandbox.write(
        "input.fl",
        "fn f() { let value = 1; value'bitvec; value'type; value'multi_deref; }\n",
    );
    let output = sandbox.run(&["input.fl"]);
    let tree = hir_dump(&output);
    for tag in ["take-bit-vec", "type-of", "multi-deref"] {
        let form = hir_form(tree, tag);
        assert!(form.contains("(resolved-local "), "lost operand in {form}");
        assert!(form.contains("(name \"value\")"), "wrong operand in {form}");
        assert!(!form.contains("(invalid)"), "invalid operand in {form}");
    }
}

#[test]
fn lowering_builtin_symbol_retains_its_name_in_hir() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn f() { .u8'builtin; }\n");
    let output = sandbox.run(&["input.fl"]);
    let form = hir_form(hir_dump(&output), "builtin");
    assert!(
        form.contains("(name \"u8\")"),
        "wrong builtin payload: {form}"
    );
}

#[test]
fn ast_selection_stops_before_name_resolution() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "use package_that_does_not_exist;\n");
    let output = sandbox.run(&["input.fl", "--emit", "ast"]);
    assert!(ast_dump(&output).contains("package_that_does_not_exist"));
}

#[test]
fn source_io_errors_are_diagnosed_without_publishing() {
    let sandbox = Sandbox::new();
    sandbox.write("invalid-utf8.fl", b"\xff\xfe");
    fs::create_dir(sandbox.path("directory.fl")).unwrap();
    for source in ["absent.fl", "directory.fl", "invalid-utf8.fl"] {
        sandbox.write("dump.lisp", OLD_OUTPUT);
        failure(&sandbox.run(&[source, "--output", "dump.lisp"]), 1);
        assert_eq!(fs::read(sandbox.path("dump.lisp")).unwrap(), OLD_OUTPUT);
    }
}

#[test]
fn output_io_errors_are_diagnosed_without_panicking() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    fs::create_dir(sandbox.path("directory.lisp")).unwrap();
    for target in ["directory.lisp", "missing-parent/dump.lisp"] {
        failure(&sandbox.run(&["input.fl", "--output", target]), 1);
        assert_eq!(
            fs::read(sandbox.path("input.fl")).unwrap(),
            VALID.as_bytes()
        );
    }
}

#[test]
fn explicit_invalid_sysroots_are_errors() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "");
    sandbox.write("file-sysroot", "not a directory");
    fs::create_dir(sandbox.path("empty-sysroot")).unwrap();
    for sysroot in ["absent-sysroot", "file-sysroot", "empty-sysroot"] {
        sandbox.write("dump.lisp", OLD_OUTPUT);
        failure(
            &sandbox.run(&["input.fl", "--sysroot", sysroot, "--output", "dump.lisp"]),
            1,
        );
        assert_eq!(fs::read(sandbox.path("dump.lisp")).unwrap(), OLD_OUTPUT);
    }
}

#[test]
fn explicit_valid_sysroot_is_accepted() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn entry()->Flag {true}");
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {pub typealias Flag=bool;}",
    );
    sandbox.write("library/std/lib.fl", "pub mod prelude {}");
    let library = sandbox.path("library");
    let output = run_command(
        sandbox
            .command()
            .arg("input.fl")
            .arg("--sysroot")
            .arg(library),
    );
    hir_dump(&output);
}

#[test]
fn explicit_sysroot_loads_builtin_prelude_and_keeps_std_import_explicit() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {pub typealias Flag=bool;pub fn ready()->Flag {true}}",
    );
    sandbox.write(
        "library/std/lib.fl",
        "pub mod prelude {pub fn standard()->bool {ready()}}",
    );
    sandbox.write(
        "input.fl",
        "use std.prelude.*;fn run()->Flag {standard() and ready()}",
    );
    let output = sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool true)"
    );
    sandbox.write("input.fl", "fn run()->bool {standard()}");
    failure(
        &sandbox.run(&["input.fl", "--sysroot", "library", "--emit", "nir"]),
        1,
    );
    sandbox.write(
        "input.fl",
        "fn ready()->bool {false} fn run()->Flag {ready()}",
    );
    let output = sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool false)"
    );
}

#[test]
fn sysroot_imports_five_preludes_with_deduplication_shadowing_and_ambiguity() {
    let sandbox = Sandbox::new();
    for name in ["builtin", "meta", "build", "attrs", "verification", "std"] {
        fs::create_dir_all(sandbox.path(format!("library/{name}"))).unwrap();
    }
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {pub fn ready()->bool {true}}",
    );
    sandbox.write(
        "library/meta/lib.fl",
        "pub mod prelude {pub use builtin.prelude.ready;pub fn metadata()->bool {ready()}}",
    );
    sandbox.write(
        "library/build/lib.fl",
        "pub mod prelude {pub fn configured()->bool {metadata()}}",
    );
    sandbox.write(
        "library/attrs/lib.fl",
        "pub mod prelude {pub fn attributed()->bool {configured()}}",
    );
    sandbox.write(
        "library/verification/lib.fl",
        "pub mod prelude {pub fn specified()->bool {attributed()}}",
    );
    sandbox.write(
        "library/std/lib.fl",
        "pub mod prelude {pub fn standard()->bool {specified()}}",
    );
    sandbox.write(
        "input.fl",
        "fn run()->bool {ready() and metadata() and configured() and attributed() and specified()}",
    );
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    sandbox.write(
        "input.fl",
        "fn ready()->bool {false} fn run()->bool {ready()}",
    );
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool false)"
    );
    sandbox.write("input.fl", "fn run()->bool {standard()}");
    failure(
        &sandbox.run(&["input.fl", "--sysroot", "library", "--emit", "nir"]),
        1,
    );
    sandbox.write("input.fl", "use std.prelude.*;fn run()->bool {standard()}");
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    sandbox.write(
        "library/meta/lib.fl",
        "pub mod prelude {pub fn ready()->bool {false} pub fn metadata()->bool {true}}",
    );
    sandbox.write("input.fl", "fn run()->bool {ready()}");
    failure(
        &sandbox.run(&["input.fl", "--sysroot", "library", "--emit", "nir"]),
        1,
    );
    sandbox.write(
        "input.fl",
        "use builtin.prelude.ready;fn run()->bool {ready()}",
    );
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
}

#[test]
fn explicit_sysroot_protects_dependency_sources_and_reports_invalid_packages() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write("input.fl", "fn run()->bool {true}");
    sandbox.write("library/builtin/lib.fl", "pub mod prelude {}");
    let source = "pub fn standard()->bool {true}";
    sandbox.write("library/std/lib.fl", source);
    failure(
        &sandbox.run(&[
            "input.fl",
            "--sysroot",
            "library",
            "--output",
            "library/std/lib.fl",
        ]),
        1,
    );
    assert_eq!(
        fs::read_to_string(sandbox.path("library/std/lib.fl")).unwrap(),
        source
    );
    #[cfg(unix)]
    {
        fs::hard_link(sandbox.path("library/std/lib.fl"), sandbox.path("alias.fl")).unwrap();
        failure(
            &sandbox.run(&["input.fl", "--sysroot", "library", "--output", "alias.fl"]),
            1,
        );
        assert_eq!(
            fs::read_to_string(sandbox.path("alias.fl")).unwrap(),
            source
        );
    }
    sandbox.write("library/std/lib.fl", "fn broken(");
    sandbox.write("result", OLD_OUTPUT);
    failure(
        &sandbox.run(&["input.fl", "--sysroot", "library", "--output", "result"]),
        1,
    );
    assert_eq!(fs::read(sandbox.path("result")).unwrap(), OLD_OUTPUT);
    fs::remove_file(sandbox.path("library/std/lib.fl")).unwrap();
    failure(&sandbox.run(&["input.fl", "--sysroot", "library"]), 1);
}

#[test]
fn explicit_sysroot_cli_entries_are_selected_from_primary_package() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {} trait Duplicate {} trait Retire {fn drop(*self);}",
    );
    sandbox.write("library/std/lib.fl", "pub fn run()->bool {false} pub comptime fn compute()->bool {false} pub fn dependency_only()->bool {false}");
    sandbox.write(
        "input.fl",
        "fn run()->bool {true} comptime fn compute()->bool {true}",
    );
    for args in [
        vec!["--interpret", "run"],
        vec!["--invoke-comptime", "compute"],
        vec![
            "--interpret",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ],
        vec!["--instantiate", "run"],
        vec!["--analyze-resources", "run", "--copy-protocol", "Duplicate"],
    ] {
        let mut command = vec!["input.fl", "--sysroot", "library"];
        command.extend_from_slice(&args);
        let output = sandbox.run(&command);
        assert!(output.status.success(), "{args:?}: {}", details(&output));
        if args[0] == "--interpret" || args[0] == "--invoke-comptime" {
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "(bool true)"
            );
        }
    }
    for mode in ["--interpret", "--instantiate", "--invoke-comptime"] {
        failure(
            &sandbox.run(&["input.fl", "--sysroot", "library", mode, "dependency_only"]),
            1,
        );
    }
    failure(
        &sandbox.run(&[
            "input.fl",
            "--sysroot",
            "library",
            "--analyze-resources",
            "dependency_only",
            "--copy-protocol",
            "Duplicate",
        ]),
        1,
    );
}

#[test]
fn explicit_sysroot_can_be_nested_inside_primary_source_directory() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {pub typealias Flag=bool;}",
    );
    sandbox.write("library/std/lib.fl", "pub fn ready()->Flag {true}");
    sandbox.write("main.fl", "use std.ready; fn run()->Flag {ready()}");
    let output = sandbox.run(&["main.fl", "--sysroot", "./library", "--interpret", "run"]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool true)"
    );
}

fn rejects_alias(sandbox: &Sandbox, input: &str, output: impl AsRef<OsStr>) {
    let before = fs::read(sandbox.path(input)).unwrap();
    for emit in ["hir", "ast"] {
        let result = run_command(
            sandbox
                .command()
                .arg(input)
                .arg("--emit")
                .arg(emit)
                .arg("--output")
                .arg(output.as_ref()),
        );
        failure(&result, 1);
        assert_eq!(fs::read(sandbox.path(input)).unwrap(), before);
    }
}

#[test]
fn output_must_not_overwrite_input_by_the_same_or_normalized_path() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    fs::create_dir(sandbox.path("subdir")).unwrap();
    for alias in ["input.fl", "./input.fl", "subdir/../input.fl"] {
        rejects_alias(&sandbox, "input.fl", alias);
    }
    rejects_alias(&sandbox, "input.fl", sandbox.path("input.fl"));
}

#[cfg(unix)]
#[test]
fn output_must_not_overwrite_input_through_symlinks() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    symlink("input.fl", sandbox.path("alias.fl")).unwrap();
    rejects_alias(&sandbox, "input.fl", "alias.fl");
    rejects_alias(&sandbox, "alias.fl", "input.fl");
    assert_eq!(
        fs::read_link(sandbox.path("alias.fl")).unwrap(),
        Path::new("input.fl")
    );
}

#[cfg(unix)]
#[test]
fn output_must_not_overwrite_input_through_hardlinks() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    fs::hard_link(sandbox.path("input.fl"), sandbox.path("alias.fl")).unwrap();
    rejects_alias(&sandbox, "input.fl", "alias.fl");
    assert_eq!(
        fs::read(sandbox.path("alias.fl")).unwrap(),
        VALID.as_bytes()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn failing_output_device_is_reported_without_panicking() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    failure(&sandbox.run(&["input.fl", "--output", "/dev/full"]), 1);
}

#[test]
fn repeated_dumps_preserve_nested_bodies_and_escaped_text() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        concat!(
            "fn first() {}\n",
            "fn second() { first(); }\n",
            "fn nested() {\n",
            "    if true {\n",
            "        let message = \"payload_marker ) ( quote \\\" slash \\\\\";\n",
            "    } else {}\n",
            "}\n",
        ),
    );
    for emit in ["ast", "hir"] {
        let initial = sandbox.run(&["input.fl", "--emit", emit]);
        let tree = if emit == "hir" {
            hir_dump(&initial)
        } else {
            ast_dump(&initial)
        };
        for payload in ["first", "second", "nested", "payload_marker"] {
            assert!(tree.contains(payload), "missing {payload}: {tree}");
        }
        for _ in 0..3 {
            let repeated = sandbox.run(&["input.fl", "--emit", emit]);
            success(&repeated);
            assert_eq!(
                initial.stdout, repeated.stdout,
                "nondeterministic {emit} dump"
            );
        }
    }
}

#[test]
fn function_clause_parameters_resolve_and_preserve_defaults_and_quote_shape() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn keep(value: T) -> T where T { value } fn options(value: T) -> T where T, .flag: true'type = false, quote ...code: T { value }");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = success(&output);
    assert!(dump.contains("resolved-clause-param"));
    assert!(dump.contains("(default (expr"));
    assert!(dump.contains("(quote (parameter (variadic"));
    assert!(dump.contains("(definition (some (def-id"));
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("comptime/generic/contract"));
}

#[test]
fn trait_bound_clause_keeps_its_parameter_and_full_bound_expression() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "trait A {} trait B {} fn keep(value: T) -> T where T:- A + B { value }",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = success(&output);
    assert!(dump.contains("(trait-bound (name (ident (name \"T\")"));
    assert!(dump.contains("resolved-clause-param"));
    assert!(dump.contains("(binary"));
}

#[test]
fn clause_names_are_scoped_and_duplicate_parameters_are_diagnosed() {
    let sandbox = Sandbox::new();
    for source in [
        "fn bad(value: T) where T, T {}",
        "fn good(value: T) where T {} fn bad(value: T) {}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

#[test]
fn generic_type_bodies_and_parent_traits_retain_their_scopes() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait A {} trait B {} trait Combined :- A + B {} struct Box where T { value: T, } enum Maybe where T { none, some(T), }");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = success(&output);
    assert!(dump.contains("(super-traits (some (expr"));
    assert!(dump.matches("(definition (some (def-id").count() >= 2);
    assert!(dump.matches("resolved-clause-param").count() >= 2);
}

#[test]
fn labeled_contracts_keep_labels_and_runtime_parameter_references_in_hir() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "fn check(flag: true'type) where requires:input flag, ensures:output flag {} ",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = success(&output);
    assert!(dump.contains("(label (some (ident (name \"input\")"));
    assert!(dump.contains("(label (some (ident (name \"output\")"));
    assert!(dump.contains("resolved-fn-param"));
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    let dump = success(&output);
    assert!(dump.contains("(program-requires (expr"));
    assert!(dump.contains("(program-ensures (expr"));
}

#[test]
fn implementation_heads_and_members_share_their_scanned_generic_scope() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        trait Marker where T {}
        struct Box where T { value: T, }
        impl Box<T> where T {
            fn keep(value: T) -> T { helper(value) }
            fn helper(value: T) -> T { value }
        }
        impl Marker<T> for Box<T> where T {
            fn keep(value: T) -> T { helper(value) }
            fn helper(value: T) -> T { value }
        }
        fn keep(value: true'type) -> true'type { value }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert!(
        dump.matches("resolved-clause-param").count() >= 10,
        "{dump}"
    );
    assert!(dump.contains("(impl"), "{dump}");
    assert_eq!(
        output.stdout,
        sandbox.run(&["input.fl", "--emit", "hir"]).stdout
    );
}

#[test]
fn implementation_scope_does_not_leak_into_siblings_or_outer_functions() {
    let sandbox = Sandbox::new();
    for source in [
        "struct Box where T { value: T, } impl Box<T> where T { fn keep(value: T) -> T { value } } fn bad(value: T) {}",
        "struct Box {} impl Box { fn hidden() {} } fn bad() { hidden(); }",
        "struct Box where T { value: T, } impl Box<T> where T {} impl Box<T> {}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        assert!(!output.status.success(), "{}", details(&output));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

#[test]
fn implementation_scopes_are_file_qualified() {
    let sandbox = Sandbox::new();
    sandbox.write("main.fl", "fn entry() {}");
    for module in ["left", "right"] {
        sandbox.write(&format!("{module}.fl"), "struct Box where T { value: T, } impl Box<T> where T { fn keep(value: T) -> T { value } }");
    }
    let output = sandbox.run(&["main.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert!(dump.matches("resolved-clause-param").count() >= 8, "{dump}");
}

#[test]
fn required_trait_functions_preserve_signatures_without_inventing_bodies() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait Transform where T { fn apply(value: T) -> T; fn identity(value: T) -> T { value } } trait Other { fn apply(value: true'type) -> true'type; }");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(fn-declaration ").count(), 2, "{dump}");
    assert!(dump.matches("resolved-clause-param").count() >= 4, "{dump}");
    // Typed contracts are retained, but only the actual default has a body.
    // NIR emission itself still does not grant runtime execution permission.
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = String::from_utf8(output.stdout).unwrap();
    assert_eq!(dump.matches("(method ").count(), 3, "{dump}");
    assert_eq!(dump.matches("(body ").count(), 1, "{dump}");
}

#[test]
fn associated_values_keep_identity_types_defaults_and_parameter_scopes() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        trait Source where T {
            assoc Element: T;
            assoc enabled: true'type = true;
            assoc choose<flag: true'type>: true'type = flag;
            fn read(value: Element) -> Element;
        }
        struct Box {}
        impl Source<true'type> for Box {
            assoc Element: true'type = false;
            assoc enabled: true'type = true;
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(associated ").count(), 5, "{dump}");
    assert!(dump.contains("resolved-fn-param"), "{dump}");
    assert!(dump.contains("(fn-declaration "), "{dump}");
    assert!(!sandbox.run(&["input.fl", "--emit", "nir"]).status.success());
}

#[test]
fn associated_declarations_reject_duplicate_members_and_parameter_leaks() {
    let sandbox = Sandbox::new();
    for source in [
        "trait Source { assoc enabled: true'type; assoc enabled: true'type; }",
        "trait Source { assoc choose<flag: true'type, flag: true'type>: true'type = flag; }",
        "trait Source { assoc choose<flag: true'type>: true'type = flag; assoc bad: true'type = flag; }",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        assert!(!output.status.success(), "{}", details(&output));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

#[test]
fn closure_environment_hir_preserves_binding_sources_and_quoted_names() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "fn make(flag: true'type) -> %{saved: true'type = flag} fn() -> true'type { || flag }",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert!(dump.contains("(ty-closure (context"), "{dump}");
    assert!(dump.contains("(closure-environment (bindings"), "{dump}");
    assert!(
        dump.contains("(environment-binding (name (ident (name \"saved\")"),
        "{dump}"
    );
    assert!(dump.contains("(source (some (expr"), "{dump}");
    // Preserving this syntax does not authorize ignoring its provenance contract.
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("dependency/provenance checking"));
}

#[test]
fn comptime_invocation_specializes_type_dispatch_and_preserves_failed_output() {
    let sandbox = Sandbox::new();
    let run = |args: &[&str]| {
        let mut args = args.to_vec();
        args.extend([
            "--meta-list-type",
            "input.TupleElements",
            "--tuple-type",
            "input.Tuple",
        ]);
        sandbox.run(&args)
    };
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/staged-comptime-dispatch.fl"),
    );
    let output = run(&["input.fl", "--invoke-comptime", "entry"]);
    assert_eq!(success(&output).trim(), "(tuple (bool true) (bool true))");
    let output = run(&["input.fl", "--invoke-comptime", "defaulted"]);
    assert_eq!(success(&output).trim(), "(bool true)");
    sandbox.write("result.lisp", OLD_OUTPUT);
    failure(
        &run(&[
            "input.fl",
            "--invoke-comptime",
            "entry",
            "--fuel",
            "1",
            "--output",
            "result.lisp",
        ]),
        1,
    );
    assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
    let output = run(&[
        "input.fl",
        "--invoke-comptime",
        "entry",
        "--output",
        "result.lisp",
    ]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp"))
            .unwrap()
            .trim(),
        "(tuple (bool true) (bool true))"
    );
}

#[test]
fn comptime_invocation_executes_real_closures_defaults_and_writes_complete_values() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.0.join("input.fl"),
        include_str!("../../../development/examples/computed-environment-types.fl"),
    )
    .unwrap();
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "run_comptime"]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(integer (sign positive) (hex \"9\"))"
    );
    let output = sandbox.run(&[
        "input.fl",
        "--invoke-comptime",
        "PointerBool",
        "--output",
        "value.lisp",
    ]);
    assert!(output.status.success(), "{}", details(&output));
    assert!(output.stdout.is_empty());
    assert!(
        fs::read_to_string(sandbox.0.join("value.lisp"))
            .unwrap()
            .contains("pointer")
    );
    fs::write(
        sandbox.0.join("input.fl"),
        "comptime fn run(.value: .u8'builtin = 7) -> .u8'builtin { value }",
    )
    .unwrap();
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "run"]);
    assert!(output.status.success(), "{}", details(&output));
    assert!(!output.stdout.is_empty());
}

#[test]
fn comptime_invocation_rejects_wrong_entries_and_preserves_output_on_failure() {
    let sandbox = Sandbox::new();
    for (source, entry, diagnostic) in [
        ("fn run() {}", "run", "not a comptime function"),
        ("comptime fn run() {}", "missing", "no function named"),
        (
            "comptime fn run(value: true'type) {}",
            "run",
            "requires arguments",
        ),
        (
            "comptime fn run() where T {}",
            "run",
            "requires generic arguments",
        ),
        (
            "typealias Type = .Type'builtin; pure comptime fn Head<T> -> Type { T }",
            "Head",
            "not a comptime function",
        ),
        (
            "mod one { comptime fn run() {} } mod two { comptime fn run() {} }",
            "run",
            "ambiguous",
        ),
    ] {
        fs::write(sandbox.0.join("input.fl"), source).unwrap();
        fs::write(sandbox.0.join("value.lisp"), OLD_OUTPUT).unwrap();
        let output = sandbox.run(&[
            "input.fl",
            "--invoke-comptime",
            entry,
            "--output",
            "value.lisp",
        ]);
        assert_eq!(output.status.code(), Some(1), "{}", details(&output));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{}",
            details(&output)
        );
        assert!(output.stdout.is_empty());
        assert_eq!(fs::read(sandbox.0.join("value.lisp")).unwrap(), OLD_OUTPUT);
    }
}

#[test]
fn comptime_invocation_fuel_bounds_nontermination_and_does_not_execute_runtime_code() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.0.join("input.fl"),
        "comptime fn run() { while true {} }",
    )
    .unwrap();
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "run", "--fuel", "50"]);
    assert_eq!(output.status.code(), Some(1), "{}", details(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("StepsExhausted"),
        "{}",
        details(&output)
    );
    fs::write(
        sandbox.0.join("input.fl"),
        "fn hidden() {} comptime fn run() { hidden(); }",
    )
    .unwrap();
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "run"]);
    assert_eq!(output.status.code(), Some(1), "{}", details(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("runtime"),
        "{}",
        details(&output)
    );
}

#[test]
fn comptime_invocation_uses_type_solver_and_preserves_outputs_on_contract_failure() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("input.fl"), "typealias Type = .Type'builtin; struct Checked where T, requires T == true'type { value: T } comptime fn CheckedType(value: Type) -> Type { Checked<value> } comptime fn run() -> Type { CheckedType('z''type) }").unwrap();
    fs::write(sandbox.0.join("value.lisp"), OLD_OUTPUT).unwrap();
    let output = sandbox.run(&[
        "input.fl",
        "--invoke-comptime",
        "run",
        "--output",
        "value.lisp",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", details(&output));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("TypeContractRejected"),
        "{}",
        details(&output)
    );
    assert_eq!(fs::read(sandbox.0.join("value.lisp")).unwrap(), OLD_OUTPUT);
}

#[test]
fn comptime_invocation_defaults_share_the_declared_parameter_scope() {
    let sandbox = Sandbox::new();
    fs::write(sandbox.0.join("input.fl"), "typealias Int = .Integer'builtin; comptime fn run(.first: Int = 7, .second: Int = first + 2) -> Int { second }").unwrap();
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "run"]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(integer (sign positive) (hex \"9\"))"
    );
}

#[test]
fn invocation_check_elision_preserves_success_and_contract_rejection() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/checked-predicate-proofs.fl"),
    );
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "run"]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "(unsigned-integer 3)"
    );
    sandbox.write(
        "input.fl",
        "comptime fn run() { assumes true; assumes false; }",
    );
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "run"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("ProgramContractRejected"),
        "{}",
        details(&output)
    );
}

#[test]
fn instantiate_emits_graph_and_preserves_output_on_failure() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/instantiated-functions.fl"),
    );
    let success = sandbox.run(&["input.fl", "--instantiate", "entry"]);
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    assert!(String::from_utf8_lossy(&success.stdout).starts_with("(iir-package"));
    sandbox.write("output.lisp", OLD_OUTPUT);
    for entry in ["missing", "identity"] {
        let rejected = sandbox.run(&[
            "input.fl",
            "--instantiate",
            entry,
            "--output",
            "output.lisp",
        ]);
        assert_eq!(rejected.status.code(), Some(1));
        assert_eq!(fs::read(sandbox.path("output.lisp")).unwrap(), OLD_OUTPUT);
    }
    let exhausted = sandbox.run(&[
        "input.fl",
        "--instantiate",
        "entry",
        "--fuel",
        "1",
        "--output",
        "output.lisp",
    ]);
    assert!(!exhausted.status.success());
    assert_eq!(fs::read(sandbox.path("output.lisp")).unwrap(), OLD_OUTPUT);
}

#[test]
fn runtime_reference_cli_executes_direct_effect_environments() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "effect input()->bool; effect read()->bool; \
         fn callee(value:bool)->#read bool {(read()#) and value} \
         fn caller()->#[input,read] bool {callee(input()#)} \
         fn entry()->bool {caller()# {input()=>true,read()=>true,}}",
    );
    let result = sandbox.run(&["input.fl", "--interpret", "entry"]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "(bool true)"
    );
}

#[test]
fn runtime_reference_cli_executes_instances_and_ordered_defaults() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/instantiated-functions.fl"),
    );
    let result = sandbox.run(&["input.fl", "--interpret", "entry"]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "(unsigned-integer 41)"
    );
    sandbox.write(
        "defaults.fl",
        "fn entry(.first: true'type = true, .second: true'type = first)->true'type { second }",
    );
    let result = sandbox.run(&[
        "defaults.fl",
        "--interpret",
        "entry",
        "--output",
        "value.lisp",
    ]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(sandbox.path("value.lisp"))
            .unwrap()
            .trim(),
        "(bool true)"
    );
}

#[test]
fn runtime_reference_cli_failures_preserve_existing_artifact() {
    let sandbox = Sandbox::new();
    sandbox.write("output.lisp", OLD_OUTPUT);
    let cases = [
        (
            "fn entry(value:true'type)->true'type {value}",
            "requires arguments",
        ),
        ("fn entry(value:T)->T where T {value}", "generic arguments"),
        (
            "comptime fn entry()->true'type {true}",
            "not a runtime function",
        ),
        ("fn other() {}", "no function named"),
        ("fn entry() { entry() }", "exhausted"),
    ];
    for (source, diagnostic) in cases {
        sandbox.write("input.fl", source);
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--fuel",
            "1000",
            "--output",
            "output.lisp",
        ]);
        assert_eq!(result.status.code(), Some(1), "{source}");
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .to_lowercase()
                .contains(diagnostic),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read(sandbox.path("output.lisp")).unwrap(), OLD_OUTPUT);
    }
    sandbox.write("input.fl", "fn entry() {}");
    for extra in [
        vec!["--emit", "nir"],
        vec!["--instantiate", "entry"],
        vec!["--invoke-comptime", "entry"],
        vec!["--interpret", "entry"],
    ] {
        let mut args = vec!["input.fl", "--interpret", "entry"];
        args.extend(extra);
        assert_eq!(sandbox.run(&args).status.code(), Some(2));
    }
}

#[test]
fn resource_cli_emits_obligations_with_explicit_protocol_identity() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait Duplicate {} struct Holder {ptr:*true'type,} fn entry() {let value=true; let holder=Holder {ptr:value.ref}; let {ptr:p}=holder; let seen=p.*;}");
    let arguments = [
        "input.fl",
        "--analyze-resources",
        "entry",
        "--copy-protocol",
        "Duplicate",
    ];
    let result = sandbox.run(&arguments);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let dump = String::from_utf8(result.stdout).unwrap();
    assert!(dump.starts_with("(resource-analysis "), "{dump}");
    assert!(
        dump.contains("(copy-protocol ") && dump.contains("(stage ownership-obligations)"),
        "{dump}"
    );
    assert!(dump.contains("(move-report "), "{dump}");
    let result = sandbox.run(&[
        "input.fl",
        "--analyze-resources",
        "entry",
        "--copy-protocol",
        "Duplicate",
        "--output",
        "report.lisp",
    ]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(sandbox.path("report.lisp")).unwrap(),
        dump
    );
}

#[test]
fn resource_cli_reports_failures_without_replacing_existing_output() {
    let sandbox = Sandbox::new();
    for (source, entry, protocol, fuel, diagnostic) in [
        (
            "trait Duplicate {} struct Token {} fn entry() {let a=Token {}; let b=a; let c=a;}",
            "entry",
            "Duplicate",
            "100000",
            "unavailable",
        ),
        (
            "trait Duplicate {} struct Token {} impl Duplicate for Token {} fn entry() {let a=Token {}; let b=a;}",
            "entry",
            "Duplicate",
            "100000",
            "representation and resource legality",
        ),
        (
            "trait Duplicate {} fn entry() {let a=true; let b=a;}",
            "entry",
            "Duplicate",
            "1",
            "budget",
        ),
        (
            "trait Duplicate {} fn entry() {}",
            "missing",
            "Duplicate",
            "100000",
            "was not found",
        ),
        (
            "trait Duplicate {} fn entry() {}",
            "entry",
            "Missing",
            "100000",
            "was not found",
        ),
        (
            "trait Duplicate where T {} fn entry() {}",
            "entry",
            "Duplicate",
            "100000",
            "no explicit type parameters",
        ),
        (
            "trait Duplicate {} fn entry(value:T) where T {}",
            "entry",
            "Duplicate",
            "100000",
            "nongeneric wrapper",
        ),
        (
            "trait Duplicate {} comptime fn entry() {}",
            "entry",
            "Duplicate",
            "100000",
            "ordinary runtime",
        ),
        (
            "trait Duplicate {} mod a {fn entry() {}} mod b {fn entry() {}}",
            "entry",
            "Duplicate",
            "100000",
            "ambiguous",
        ),
        (
            "mod a {trait Duplicate {}} mod b {trait Duplicate {}} fn entry() {}",
            "entry",
            "Duplicate",
            "100000",
            "ambiguous",
        ),
        (
            "mod a {trait Duplicate where T {}} mod b {trait Duplicate {}} fn entry() {}",
            "entry",
            "Duplicate",
            "100000",
            "ambiguous",
        ),
        (
            "mod a {trait Duplicate {}} mod b {trait Duplicate where T {}} fn entry() {}",
            "entry",
            "Duplicate",
            "100000",
            "ambiguous",
        ),
    ] {
        sandbox.write("input.fl", source);
        sandbox.write("report.lisp", OLD_OUTPUT);
        let result = sandbox.run(&[
            "input.fl",
            "--analyze-resources",
            entry,
            "--copy-protocol",
            protocol,
            "--fuel",
            fuel,
            "--output",
            "report.lisp",
        ]);
        assert_eq!(
            result.status.code(),
            Some(1),
            "{source}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(diagnostic),
            "{source}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
        assert_eq!(fs::read(sandbox.path("report.lisp")).unwrap(), OLD_OUTPUT);
    }
}

#[test]
fn resource_cli_rejects_missing_binding_and_conflicting_modes() {
    let sandbox = Sandbox::new();
    for args in [
        vec!["input.fl", "--analyze-resources", "entry"],
        vec!["input.fl", "--copy-protocol", "Duplicate"],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--emit",
            "nir",
        ],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--interpret",
            "entry",
        ],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--instantiate",
            "entry",
        ],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--invoke-comptime",
            "entry",
        ],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
        ],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--copy-protocol",
            "Duplicate",
        ],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--fuel",
            "0",
        ],
    ] {
        let result = sandbox.run(&args);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn resource_cleanup_cli_emits_captured_plans_with_explicit_drop_identity() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl","trait Duplicate {} trait Retire {fn drop(*self);} struct Ticket {} enum Choice {empty,some(Ticket)} impl Retire for Ticket {fn drop(*self) {}} fn entry(value:Choice) {let next=value;}");
    let args = [
        "input.fl",
        "--analyze-resources",
        "entry",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
        "--output",
        "report.lisp",
    ];
    let result = sandbox.run(&args);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let dump = fs::read_to_string(sandbox.path("report.lisp")).unwrap();
    for expected in [
        "(drop-protocol ",
        "(stage acquired-cleanup-plans)",
        "(planned-cleanup ",
        "(switch-variant ",
        "(invoke-drop ",
    ] {
        assert!(dump.contains(expected), "missing {expected}: {dump}");
    }
    assert_eq!(
        dump.matches("(planned-cleanup ").count(),
        1,
        "a move must not retain the old owner's cleanup: {dump}"
    );
}

#[test]
fn resource_cleanup_cli_rejects_invalid_contracts_and_preserves_output() {
    let sandbox = Sandbox::new();
    for (source, protocol, expected) in [
        (
            "trait Duplicate {} trait Retire {fn drop(*self);} struct Ticket {} fn entry() {let ticket=Ticket {};}",
            "Missing",
            "was not found",
        ),
        (
            "trait Duplicate {} trait Retire {} struct Ticket {} fn entry() {let ticket=Ticket {};}",
            "Retire",
            "lacks its drop method",
        ),
        (
            "trait Duplicate {} trait Retire {fn drop(*self);} struct Ticket {flag:bool,} struct Holder {child:Ticket,} impl Retire for Holder {fn drop(*self) {let flag=self.child.flag.*;}} fn entry() {let holder=Holder {child:Ticket {flag:true}};let child=holder.child;}",
            "Retire",
            "unavailable",
        ),
    ] {
        sandbox.write("input.fl", source);
        sandbox.write("report.lisp", "keep existing report");
        let result = sandbox.run(&[
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            protocol,
            "--output",
            "report.lisp",
        ]);
        assert_eq!(
            result.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read_to_string(sandbox.path("report.lisp")).unwrap(),
            "keep existing report"
        );
    }
    for args in [
        vec!["input.fl", "--drop-protocol", "Retire"],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--drop-protocol",
            "Retire",
        ],
        vec![
            "input.fl",
            "--analyze-resources",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
        ],
    ] {
        assert_eq!(sandbox.run(&args).status.code(), Some(2));
    }
}

#[test]
fn checked_interpret_cli_executes_closures_defaults_and_generic_cleanup() {
    let sandbox = Sandbox::new();
    for body in [
        "struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry()->bool {let item=Item {};true}",
        "struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} typealias Callback=%{item:Item} fn()->bool; fn make()->Callback {let item=Item {flag:true};|| item.flag} fn apply(callback:Callback)->bool {callback()} fn entry()->bool {let first=apply(make());let second=make()();first and second}",
        "struct Item {} impl Retire for Item {fn drop(*self) {}} fn entry(.flag:bool=true)->bool {let item=Item {};flag}",
        "struct Item {} impl Retire for Item {fn drop(*self) {}} fn consume(value:T) where T {} fn entry()->bool {consume(Item {});true}",
    ] {
        sandbox.write(
            "input.fl",
            format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {body}"),
        );
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ]);
        assert_eq!(
            result.status.code(),
            Some(0),
            "{body}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "(bool true)"
        );
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "result.lisp",
        ]);
        assert_eq!(
            result.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
        assert_eq!(
            fs::read_to_string(sandbox.path("result.lisp"))
                .unwrap()
                .trim(),
            "(bool true)"
        );
    }
}

#[test]
fn checked_interpret_cli_rejects_invalid_ownership_and_executes_drop_contracts() {
    let sandbox = Sandbox::new();
    for (body, fuel, diagnostic) in [
        (
            "struct Item {} impl Retire for Item {fn drop(*self) {assumes false;}} fn entry()->bool {let item=Item {};true}",
            "100000",
            "ProgramContractRejected",
        ),
        (
            "struct Item {} impl Retire for Item {fn drop(*self) {}} fn take(item:Item) {} fn entry()->bool {let item=Item {};take(item);take(item);true}",
            "100000",
            "unavailable",
        ),
        (
            "struct Item {} impl Retire for Item {fn drop(*self) {assumes false;}} typealias Callback=%{item:Item} fn()->Item; fn make()->Callback {let item=Item {};|| item} fn entry()->bool {let result=make()();true}",
            "100000",
            "ProgramContractRejected",
        ),
        (
            "struct Item {} impl Retire for Item {fn drop(*self) {assumes false;}} fn entry(.flag:bool=true)->bool {let item=Item {};flag}",
            "100000",
            "ProgramContractRejected",
        ),
        ("fn entry()->bool {true}", "1", "budget"),
    ] {
        sandbox.write(
            "input.fl",
            format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {body}"),
        );
        sandbox.write("result.lisp", OLD_OUTPUT);
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--fuel",
            fuel,
            "--output",
            "result.lisp",
        ]);
        assert_eq!(
            result.status.code(),
            Some(1),
            "{body}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(diagnostic),
            "{body}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
        assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
    }
}

#[test]
fn checked_interpret_cli_binds_unique_protocol_identities_before_execution() {
    let sandbox = Sandbox::new();
    for (traits, copy, drop, diagnostic) in [
        (
            "trait Duplicate {} trait Retire {fn drop(*self);}",
            "Missing",
            "Retire",
            "was not found",
        ),
        (
            "trait Duplicate {} trait Retire {fn drop(*self);}",
            "Duplicate",
            "Missing",
            "was not found",
        ),
        (
            "trait Duplicate where T {} trait Retire {fn drop(*self);}",
            "Duplicate",
            "Retire",
            "no explicit type parameters",
        ),
        (
            "trait Duplicate {} mod a {trait Retire {fn drop(*self);}} mod b {trait Retire {fn drop(*self);}}",
            "Duplicate",
            "Retire",
            "ambiguous",
        ),
        (
            "mod a {trait Duplicate {}} mod b {trait Duplicate {}} trait Retire {fn drop(*self);}",
            "Duplicate",
            "Retire",
            "ambiguous",
        ),
    ] {
        sandbox.write("input.fl", format!("{traits} fn entry()->bool {{true}}"));
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--copy-protocol",
            copy,
            "--drop-protocol",
            drop,
        ]);
        assert_eq!(
            result.status.code(),
            Some(1),
            "{traits}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(diagnostic),
            "{traits}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn checked_interpret_cli_requires_both_protocols_and_one_execution_mode() {
    let sandbox = Sandbox::new();
    for suffix in [
        vec!["--copy-protocol", "Duplicate"],
        vec!["--drop-protocol", "Retire"],
        vec![
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--emit",
            "nir",
        ],
        vec![
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--instantiate",
            "entry",
        ],
        vec![
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--invoke-comptime",
            "entry",
        ],
        vec![
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--drop-protocol",
            "Retire",
        ],
    ] {
        let mut args = vec!["input.fl", "--interpret", "entry"];
        args.extend(suffix);
        assert_eq!(sandbox.run(&args).status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn owned_default_cli_constructs_and_delivers_aggregate_results() {
    let sandbox = Sandbox::new();
    for body in [
        "fn make()->Item {Item {flag:true}} fn target(.item:Item=make())->bool {item.flag} fn entry()->bool {target()}",
        "fn target(pointer:*bool,.item:Item=do {Item {flag:pointer.*}})->bool {item.flag} fn entry()->bool {var flag=true;target(flag.ref)}",
        "fn target(pointer:*bool,ready:bool,.item:Item=do {Item {flag:pointer.*}})->bool {item.flag} fn entry()->bool {var flag=false;target(flag.ref,do {flag=true;true})}",
        "fn entry(.item:Item=do {Item {flag:true}})->bool {item.flag}",
        "struct Box {item:Item,} impl Retire for Box {fn drop(*self) {}} fn entry(.box:Box=do {Box {item:Item {flag:true}}})->bool {box.item.flag}",
        "enum Choice {none,some(Item)} fn entry(.choice:Choice=Choice.some(Item {flag:true}))->bool {choice match {.none=>false,.some(item)=>item.flag,}}",
        "fn entry(.pair:Tuple<Item,bool> = do {(Item {flag:true},true)})->bool {let (item,flag)=pair;item.flag and flag}",
        "fn target(flag:bool,.item:Item=if flag {Item {flag:true}} else {Item {flag:false}})->bool {item.flag} fn entry()->bool {let yes=target(true);let no=target(false);yes and not no}",
        "fn target(flag:bool,.item:Item=flag match {true=>Item {flag:true},false=>Item {flag:false},})->bool {item.flag} fn entry()->bool {target(true)}",
        "fn entry(.item:Item=do {let flag=true;Item {flag:flag}})->bool {item.flag}",
    ] {
        sandbox.write("input.fl", format!("pure comptime fn List<T>->.Type'builtin {{T}} pure comptime fn Tuple<...Es:List<.Type'builtin>>->.Type'builtin {{bool}} trait Duplicate {{}} trait Retire {{fn drop(*self);}} struct Item {{flag:bool,}} impl Retire for Item {{fn drop(*self) {{}}}} {body}"));
        let result = sandbox.run(&[
            "input.fl",
            "--meta-list-type",
            "input.List",
            "--tuple-type",
            "input.Tuple",
            "--interpret",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ]);
        assert_eq!(
            result.status.code(),
            Some(0),
            "{body}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "(bool true)",
            "{body}"
        );
    }
}

#[test]
fn owned_default_cli_runs_cleanup_only_for_the_supplied_value() {
    let sandbox = Sandbox::new();
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {let flag=self.flag.*;assumes flag;}} fn target(.item:Item=do {Item {flag:false}})->bool {true} fn omitted()->bool {target()} fn supplied()->bool {target(.item=Item {flag:true})}";
    sandbox.write("input.fl", source);
    let good = sandbox.run(&[
        "input.fl",
        "--interpret",
        "supplied",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
    ]);
    assert_eq!(
        good.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&good.stdout).trim(), "(bool true)");
    sandbox.write("result.lisp", OLD_OUTPUT);
    let bad = sandbox.run(&[
        "input.fl",
        "--interpret",
        "omitted",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
        "--output",
        "result.lisp",
    ]);
    assert_eq!(
        bad.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&bad.stderr)
    );
    assert!(String::from_utf8_lossy(&bad.stderr).contains("ProgramContractRejected"));
    assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
}

#[test]
fn owned_default_cli_rejects_hidden_cleanup_reads_and_unknown_calls() {
    let sandbox = Sandbox::new();
    for declaration in [
        "fn target(.flag:bool=do {Item {flag:true};true})->bool {flag}",
        "fn target(.flag:bool=do {let unused=Item {flag:true};true})->bool {flag}",
        "fn target(.flag:bool=do {let {flag}=Item {flag:true};flag})->bool {flag}",
        "fn target(.flag:bool=(Item {flag:true}).flag)->bool {flag}",
        "fn target(.item:Item=do {let old=Item {flag:true};Item {flag:true}})->bool {item.flag}",
        "fn make()->Item {make()} fn target(.item:Item=make())->bool {item.flag}",
        "fn target(pointer:*bool,.item:Item=do {pointer.* = true;Item {flag:pointer.*}})->bool {item.flag}",
    ] {
        sandbox.write("input.fl", format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} struct Item {{flag:bool,}} impl Retire for Item {{fn drop(*self) {{assumes false;}}}} {declaration} fn entry() {{}}"));
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ]);
        assert_eq!(
            result.status.code(),
            Some(1),
            "{declaration}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("P3 stability/termination"),
            "{declaration}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn owned_default_cli_checks_arithmetic_before_publishing() {
    let sandbox = Sandbox::new();
    for (operation, accepted) in [("value + 1", false), ("value + 0", true)] {
        sandbox.write("input.fl", format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} struct Item {{flag:bool,}} impl Retire for Item {{fn drop(*self) {{}}}} typealias Byte=.u8'builtin; struct Pair {{item:Item,count:Byte,}} fn target(value:Byte,.pair:Pair=do {{Pair {{item:Item {{flag:true}},count:{operation}}}}})->bool {{pair.count==value}} fn entry()->bool {{target(255)}}"));
        sandbox.write("result.lisp", OLD_OUTPUT);
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "result.lisp",
        ]);
        assert_eq!(
            result.status.code(),
            Some(if accepted { 0 } else { 1 }),
            "{operation}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        if accepted {
            assert_eq!(
                fs::read_to_string(sandbox.path("result.lisp"))
                    .unwrap()
                    .trim(),
                "(bool true)"
            );
        } else {
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains("machine arithmetic range is not proven")
            );
            assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
        }
    }
}

#[test]
fn callable_default_cli_separates_construction_from_callback_execution() {
    let sandbox = Sandbox::new();
    for body in [
        "fn apply(.callback:fn()->bool=|| true)->bool {callback()} fn entry()->bool {apply()}",
        "fn yes()->bool {true} fn apply(.callback:fn()->bool=yes)->bool {callback()} fn entry()->bool {apply()}",
        "fn apply(flag:bool,.callback:%{flag:bool} fn()->bool=|| flag)->bool {callback()} fn entry()->bool {let first=apply(true);let second=apply(false);first and not second}",
        "struct Local {} impl Retire for Local {fn drop(*self) {}} fn apply(.callback:fn()->bool=|| {let local=Local {};true})->bool {callback()} fn entry()->bool {apply()}",
        "fn ignore(.callback:fn()->bool=|| {assumes false;true})->bool {true} fn entry()->bool {ignore()}",
        "typealias Callback=fn()->bool; fn make(.callback:Callback=|| true)->Callback {callback} fn entry()->bool {make()()}",
    ] {
        sandbox.write(
            "input.fl",
            format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} {body}"),
        );
        let result = sandbox.run(&[
            "input.fl",
            "--interpret",
            "entry",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ]);
        assert_eq!(
            result.status.code(),
            Some(0),
            "{body}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "(bool true)"
        );
    }
}

#[test]
fn callable_default_cli_propagates_failure_and_explicit_override() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait Duplicate {} trait Retire {fn drop(*self);} fn apply(.callback:fn()->bool=|| {assumes false;true})->bool {callback()} fn failing()->bool {apply()} fn supplied()->bool {apply(.callback=|| true)}");
    let good = sandbox.run(&[
        "input.fl",
        "--interpret",
        "supplied",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
    ]);
    assert_eq!(
        good.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    sandbox.write("result.lisp", OLD_OUTPUT);
    let bad = sandbox.run(&[
        "input.fl",
        "--interpret",
        "failing",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
        "--output",
        "result.lisp",
    ]);
    assert_eq!(
        bad.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&bad.stderr)
    );
    assert!(String::from_utf8_lossy(&bad.stderr).contains("ProgramContractRejected"));
    assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
}

#[test]
fn callable_default_cli_cannot_duplicate_an_affine_prepared_parameter() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait Duplicate {} trait Retire {fn drop(*self);} struct Item {flag:bool,} impl Retire for Item {fn drop(*self) {}} fn apply(item:Item,.callback:%{item:Item} fn()->bool=|| item.flag)->bool {callback()} fn entry()->bool {apply(Item {flag:true})}");
    let result = sandbox.run(&[
        "input.fl",
        "--interpret",
        "entry",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
    ]);
    assert_eq!(
        result.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("default preparation consumed a parameter"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
}

#[test]
fn effect_hir_preserves_operation_identity_groups_and_modifiers() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        effect read() -> bool;
        async inline effect wait(flag: bool) -> bool;
        effect enum Report {
            count() -> bool,
            inline message(value: bool),
            async wait() -> bool,
        }
        fn names() { read; Report.count; }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(effect-operation ").count(), 5, "{dump}");
    assert_eq!(dump.matches("(effect-group ").count(), 1, "{dump}");
    assert!(dump.contains("(async true) (inline true)"), "{dump}");
    assert!(dump.contains("(async false) (inline true)"), "{dump}");
    assert!(dump.contains("(async true) (inline false)"), "{dump}");
    assert!(!dump.contains("(fn-declaration "), "{dump}");
    assert!(!dump.contains("(invalid"), "{dump}");
    let repeated = sandbox.run(&["input.fl", "--emit", "hir"]);
    assert_eq!(output.stdout, repeated.stdout);
}

#[test]
fn effect_hir_resolves_group_and_operation_type_parameters() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        effect identity(value: T) -> T where T;
        effect enum Channel where T {
            send(value: T),
            receive() -> T,
        }
        effect enum Other { receive() -> bool, }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(effect-operation ").count(), 4, "{dump}");
    assert_eq!(dump.matches("(effect-group ").count(), 2, "{dump}");
}

#[test]
fn effect_declarations_reject_duplicate_names_and_leaked_parameters() {
    for source in [
        "effect read(); effect read();",
        "effect enum IO { read(), read(), }",
        "effect read(value: bool, value: bool);",
        "effect generic(value: T) where T; effect leaked(value: T);",
        "effect enum Generic where T { read() -> T, } effect leaked() -> T;",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        assert!(!output.status.success(), "accepted: {source}");
        assert!(output.stdout.is_empty(), "{source}");
    }
}

#[test]
fn effect_requests_require_an_explicit_allowed_row() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "effect read() -> bool; fn entry() -> bool { read()# }",
    );
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!output.status.success());
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        errors.contains("effect request requires an explicit allowed effect row"),
        "{errors}"
    );
    assert!(!errors.contains("panicked"), "{errors}");
    assert!(output.stdout.is_empty());
}

#[test]
fn effect_handlers_keep_requests_guards_and_resume_structure() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        effect echo(value: bool) -> bool;
        fn source() -> #[echo] bool { echo(true)# }
        fn entry() -> bool {
            source()# {
                echo(value) if value => {
                    resume false while value;
                    echo(value)#
                },
                echo(other) => other,
            }
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(effect-handle ").count(), 1, "{dump}");
    assert_eq!(dump.matches("(effect-request ").count(), 2, "{dump}");
    assert_eq!(dump.matches("(resume ").count(), 1, "{dump}");
    assert!(dump.contains("(guard (some"), "{dump}");
    assert!(dump.contains("(resolved-local"), "{dump}");
    assert!(!dump.contains("(match "), "{dump}");
}

#[test]
fn nested_effect_handlers_keep_outer_request_and_inner_computation_separate() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        effect read() -> bool;
        fn source() -> #[read] bool { read()# }
        fn entry() -> bool {
            (source()# { read() => read()#, })# { read() => true, }
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(effect-handle ").count(), 2, "{dump}");
    assert_eq!(dump.matches("(effect-request ").count(), 2, "{dump}");
    assert!(!dump.contains("(match "), "{dump}");
}

#[test]
fn effect_handler_bindings_do_not_escape_their_arm() {
    for source in [
        "effect echo(value: bool) -> bool; fn bad() { echo(value)# { echo(value) => value, } }",
        "effect echo(value: bool) -> bool; fn bad() { echo(true)# { echo(value) => value, echo(other) => value, } }",
        "effect echo(value: bool) -> bool; fn bad() { let result = echo(true)# { echo(value) => value, } value }",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        assert!(!output.status.success(), "accepted: {source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

#[test]
fn effect_group_qualifications_and_attributed_arms_lower_without_losing_structure() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        effect enum IO { read() -> bool, write(value: bool), }
        fn whole() -> #IO bool { IO.read()# }
        fn single() -> #IO.read bool { IO.read()# }
        fn entry() -> bool {
            whole()# { ^attrs.marker IO.read() => true, }
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    assert_eq!(dump.matches("(effect-qualified-type ").count(), 2, "{dump}");
    assert_eq!(dump.matches("(effect-handle ").count(), 1, "{dump}");
    assert_eq!(dump.matches("(effect-request ").count(), 2, "{dump}");
    assert!(dump.contains("case arm attribute"), "{dump}");
}

#[test]
fn effect_nir_checks_signature_types_and_rejects_ordinary_callable_uses() {
    for source in [
        "effect bad(value: true);",
        "effect bad() -> false;",
        "effect read() -> bool; fn bad() -> bool { read() }",
        "effect read() -> bool; fn bad() { let callback = read; }",
        "effect read(value: T) -> T where T; fn bad() -> bool { read<bool>(true) }",
        "effect enum IO { read() -> bool, } fn bad() -> bool { IO.read() }",
        "effect bad(value: bool) where requires value;",
        "effect bad(value: bool) where ensures value;",
        "effect bad(.value: bool = true);",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(!output.status.success(), "accepted: {source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("panicked"),
            "{source}"
        );
    }
}

#[test]
fn effect_nir_dump_preserves_checked_operation_metadata() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "effect read() -> bool; async inline effect wait(value: bool); fn entry() -> bool { true }",
    );
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    let dump = success(&output);
    single_sexpression(dump);
    assert_eq!(dump.matches("(effect-declaration ").count(), 2, "{dump}");
    assert!(dump.contains("(abi async) (inline true)"), "{dump}");
    assert!(dump.contains("(abi direct) (inline false)"), "{dump}");
    let repeat = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert_eq!(output.stdout, repeat.stdout);
}

#[test]
fn effect_signature_formation_does_not_assume_missing_group_contracts() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "struct Checked where T, requires T == bool {} effect enum IO where U { send(value: Checked<U>), }");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
}

#[test]
fn shipped_library_files_use_current_syntax() {
    let sandbox = Sandbox::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "library/builtin/lib.fl",
        "library/std/lib.fl",
        "library/builtin/traits.fl",
        "library/builtin/comparison.fl",
        "library/builtin/resources.fl",
        "library/builtin/ops.fl",
        "library/builtin/iterator.fl",
        "library/builtin/option.fl",
        "library/builtin/result.fl",
        "library/builtin/primitives.fl",
    ] {
        let output = run_command(
            sandbox
                .command()
                .arg(repository.join(relative))
                .args(["--emit", "ast"]),
        );
        assert!(output.status.success(), "{relative}: {}", details(&output));
    }
}

#[test]
fn shipped_operator_protocols_execute_with_default_and_named_types() {
    let sandbox = Sandbox::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["primitives", "ops"] {
        fs::copy(
            repository.join(format!("library/builtin/{name}.fl")),
            sandbox.path(format!("{name}.fl")),
        )
        .unwrap();
    }
    sandbox.write("lib.fl", "pub use primitives.*; pub use ops.*; impl Add for bool {assoc Output:Type=bool;fn add(self,rhs:bool)->bool {self or rhs}} impl Add<.Rhs=bool> for char {assoc Output:Type=bool;fn add(self,rhs:bool)->bool {rhs}} fn run()->bool {false.as(Add).add(true) and 'x'.as(Add<.Rhs=bool>).add(true)}");
    let output = sandbox.run(&["lib.fl", "--emit", "hir"]);
    hir_dump(&output);
    let output = sandbox.run(&["lib.fl", "--interpret", "run"]);
    assert!(output.status.success(), "{}", details(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool true)"
    );
}

#[test]
fn shipped_optional_module_resolves_without_enum_substitute() {
    let sandbox = Sandbox::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["primitives", "option"] {
        fs::copy(
            repository.join(format!("library/builtin/{name}.fl")),
            sandbox.path(format!("{name}.fl")),
        )
        .unwrap();
    }
    sandbox.write(
        "lib.fl",
        "pub use primitives.*;pub use option.OptionalOps;
        fn run()->bool {let value:?bool=true;value.as(OptionalOps<bool>).unwrap_or(false)}",
    );
    let output = sandbox.run(&["lib.fl", "--emit", "hir"]);
    hir_dump(&output);
}

#[test]
fn draft_result_unwrap_composes_with_shipped_result_callbacks() {
    let sandbox = Sandbox::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["primitives", "result", "result_unwrap"] {
        fs::copy(
            repository.join(if name == "result_unwrap" {
                format!("development/library-prototypes/builtin/{name}.fl")
            } else {
                format!("library/builtin/{name}.fl")
            }),
            sandbox.path(format!("{name}.fl")),
        )
        .unwrap();
    }
    sandbox.write(
        "lib.fl",
        "pub use primitives.*;pub use result.ErrorResultOps;pub use result_unwrap.ErrorResultUnwrap;
        fn panic(message:str)->noreturn {panic(message)}
        fn good()->!u8 bool {true}
        fn bad()->!u8 bool {error 7}
        fn forbidden_bool(value:bool)->bool {forbidden_bool(value)}
        fn forbidden_byte(value:u8)->u8 {forbidden_byte(value)}
        fn forbidden_result(value:bool)->!u8 bool {forbidden_result(value)}
        fn invert(value:bool)->!u8 bool {not value}
        fn run()->bool {
            good().as(ErrorResultOps<bool,u8>).is_ok() and bad().as(ErrorResultOps<bool,u8>).is_err() and
            good().as(ErrorResultUnwrap<bool,u8>).unwrap() and bad().as(ErrorResultUnwrap<bool,u8>).unwrap_err()==7 and
            not good().as(ErrorResultOps<bool,u8>).map(|value| not value).as(ErrorResultUnwrap<bool,u8>).unwrap() and
            bad().as(ErrorResultOps<bool,u8>).map_err(|value| value==7).as(ErrorResultUnwrap<bool,bool>).unwrap_err() and
            good().as(ErrorResultOps<bool,u8>).and_then(invert).as(ErrorResultOps<bool,u8>).is_ok() and
            good().as(ErrorResultOps<bool,u8>).and_then(|value| {let result:!u8 bool=not value;return result;}).as(ErrorResultOps<bool,u8>).is_ok() and
            bad().as(ErrorResultOps<bool,u8>).and_then(forbidden_result).as(ErrorResultOps<bool,u8>).is_err() and
            bad().as(ErrorResultOps<bool,u8>).map(forbidden_bool).as(ErrorResultUnwrap<bool,u8>).unwrap_err()==7 and
            good().as(ErrorResultOps<bool,u8>).map_err(forbidden_byte).as(ErrorResultUnwrap<bool,u8>).unwrap()
        }",
    );
    let output = sandbox.run(&["lib.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
}

#[test]
fn draft_result_unwrap_executes_from_an_importing_package() {
    let sandbox = Sandbox::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    fs::copy(
        repository.join("library/builtin/primitives.fl"),
        sandbox.path("library/builtin/primitives.fl"),
    )
    .unwrap();
    fs::copy(
        repository.join("library/builtin/result.fl"),
        sandbox.path("library/std/result.fl"),
    )
    .unwrap();
    fs::copy(
        repository.join("development/library-prototypes/builtin/result_unwrap.fl"),
        sandbox.path("library/std/result_unwrap.fl"),
    )
    .unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub use primitives.*;pub mod prelude {pub use primitives.*;}",
    );
    // A test-only diverging panic implementation makes the native error-result module
    // checkable. No assertion below invokes panic or claims its API is shipped.
    sandbox.write(
        "library/std/lib.fl",
        "pub use result.ErrorResultOps;pub use result_unwrap.ErrorResultUnwrap;fn panic(message:str)->noreturn {panic(message)}",
    );
    sandbox.write(
        "input.fl",
        "use std.{ErrorResultOps,ErrorResultUnwrap};
        fn good()->!u8 bool {true}
        fn bad()->!u8 bool {error 7}
        fn run()->bool {good().as(ErrorResultOps<bool,u8>).map(|value| not value).as(ErrorResultOps<bool,u8>).is_ok() and bad().as(ErrorResultUnwrap<bool,u8>).unwrap_err()==7}",
    );
    let output = sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
}

#[test]
fn draft_optional_unwrap_composes_with_shipped_operations() {
    let sandbox = Sandbox::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["primitives", "option", "option_unwrap"] {
        fs::copy(
            repository.join(if name == "option_unwrap" {
                format!("development/library-prototypes/builtin/{name}.fl")
            } else {
                format!("library/builtin/{name}.fl")
            }),
            sandbox.path(format!("{name}.fl")),
        )
        .unwrap();
    }
    // Test-only diverging panic; exercised success branches do not invoke it.
    sandbox.write(
        "lib.fl",
        "pub use primitives.*;pub use option.OptionalOps;pub use option_unwrap.OptionalUnwrap;
        fn panic(message:str)->noreturn {panic(message)}
        fn some()->?bool {true}
        fn none()->?bool {null}
        fn nested()->??bool {none()}
        fn forbidden(value:bool)->bool {forbidden(value)}
        fn forbidden_optional(value:bool)->?bool {forbidden_optional(value)}
        fn invert(value:bool)->?bool {not value}
        fn run()->bool {
            some().as(OptionalOps<bool>).is_some() and none().as(OptionalOps<bool>).is_none() and
            some().as(OptionalUnwrap<bool>).unwrap() and
            some().as(OptionalOps<bool>).unwrap_or(false) and none().as(OptionalOps<bool>).unwrap_or(true) and
            not some().as(OptionalOps<bool>).map(|value| not value).as(OptionalUnwrap<bool>).unwrap() and
            none().as(OptionalOps<bool>).map(forbidden).as(OptionalOps<bool>).is_none() and
            some().as(OptionalOps<bool>).and_then(invert).as(OptionalOps<bool>).is_some() and
            none().as(OptionalOps<bool>).and_then(forbidden_optional).as(OptionalOps<bool>).is_none() and
            nested().as(OptionalUnwrap<?bool>).unwrap().as(OptionalOps<bool>).is_none()}",
    );
    let output = sandbox.run(&["lib.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
}

#[test]
fn optional_type_does_not_use_enum_names_or_shapes() {
    let sandbox = Sandbox::new();
    for definition in [
        "enum Maybe where T {Absent,Present(T)}",
        "enum Maybe where T {Present(T),Absent}",
        "enum Maybe where T {Absent,Present(bool)}",
        "enum Maybe where T {Absent,Present(T),Other}",
        "enum Maybe where T,E {Absent,Present(T)}",
        "struct Maybe where T {value:T}",
    ] {
        sandbox.write("input.fl", &format!("{definition} enum Option where T {{Wrong(T)}} fn run()->bool {{let value:?bool=true;value match {{flag?=>flag,null=>false}}}}"));
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert_eq!(success(&output).trim(), "(bool true)");
    }
}

#[test]
fn removed_optional_type_selector_is_rejected_in_both_cli_modes() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn run()->bool {true}");
    for mut args in [
        vec!["input.fl"],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "701",
            "input",
            "input.fl",
            "--primary",
            "1",
        ],
    ] {
        args.extend(["--optional-type", "input.Maybe", "--interpret", "run"]);
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(2), "{}", details(&output));
        assert!(details(&output).contains("--optional-type"));
    }
}

#[test]
fn optional_null_and_patterns_preserve_each_nested_layer() {
    let sandbox = Sandbox::new();
    for definition in [
        "enum Maybe where T {Absent,Present(T)}",
        "enum Maybe where T {Present(T),Absent}",
    ] {
        sandbox.write(
            "input.fl",
            &format!(
                "{definition}
            fn classify(value:??bool)->.u8'builtin {{value match {{
                inner? => inner match {{flag? => if flag {{2}} else {{3}},null=>1}},null=>0}}}}
            fn make(flag:bool)->?bool {{if flag {{true}} else {{null}}}}
            fn run()->bool {{classify(null)==0 and classify(do {{let inner:?bool=null;inner}})==1 and
                classify(do {{let inner:?bool=true;inner}})==2 and classify(do {{let inner:?bool=false;inner}})==3 and
                (make(true) match {{value?=>value,null=>false}}) and
                (make(false) match {{_?=>false,null=>true}})}}"
            ),
        );
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert_eq!(success(&output).trim(), "(bool true)");
    }
    for body in [
        "fn run()->bool {null}",
        "fn run()->bool {true match {value?=>value,_=>false}}",
        "fn run()->bool {let value:?bool=null;value match {flag?=>flag}}",
    ] {
        sandbox.write(
            "input.fl",
            &format!("enum Maybe where T {{Absent,Present(T)}} {body}"),
        );
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert!(!output.status.success(), "{body}");
    }
}

#[test]
fn optional_patterns_export_bindings_and_keep_failure_scopes_separate() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "enum Maybe where T {Missing,Value(T)}
        fn take(value:?bool)->bool {let inner?=value else {return false;};inner}
        fn choose(value:?bool)->bool {value match {flag? or flag?=>flag,null=>false}}
        fn run()->bool {take(true) and not take(null) and choose(true) and not choose(null)}",
    );
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
    sandbox.write(
        "input.fl",
        "enum Maybe where T {Missing,Value(T)}
        fn run()->bool {let value:?bool=null;value match {flag?=>flag,null=>flag}}",
    );
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert!(!output.status.success());
    assert!(
        details(&output).contains("Unresolved identifier"),
        "{}",
        details(&output)
    );
}

#[test]
fn optional_success_injection_preserves_existing_layers() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl","enum Maybe where T {Absent,Present(T)}
        fn classify(value:??bool)->.u8'builtin {value match {inner?=>inner match {flag?=>if flag {2} else {3},null=>1},null=>0}}
        fn identity(value:T)->T where T {value}
        fn choose(flag:bool)->?bool {if flag {true} else {null}}
        fn number()->?.u8'builtin {9}
        fn run()->bool {let cleared:?bool=null;let configured:?bool=true;
            let same:?bool=configured;let missing:?bool=identity(null);
            classify(null)==0 and classify(cleared)==1 and classify(configured)==2 and classify(same)==2 and
            classify(false)==3 and (choose(true) match {flag?=>flag,null=>false}) and
            (choose(false) match {_?=>false,null=>true}) and
            (number() match {value?=>value==9,null=>false}) and
            (missing match {_?=>false,null=>true})}");
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
}

#[test]
fn generic_optional_parameters_infer_payloads_without_extra_layers() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "enum Maybe where T {Absent,Present(T)}
        fn keep(value:?T)->?T where T {value}
        fn nested(value:??T)->??T where T {value}
        fn pair(value:?T,other:T)->?T where T {value}
        fn run()->bool {
            let present:?bool=true;let empty:?bool=null;
            let a=keep(true);let b=keep(present);let c=keep(empty);
            let d=nested(empty);let e=pair(true,false);
            (a match {flag?=>flag,null=>false}) and
            (b match {flag?=>flag,null=>false}) and
            (c match {_?=>false,null=>true}) and
            (d match {inner?=>inner match {_?=>false,null=>true},null=>false}) and
            (e match {flag?=>flag,null=>false})}",
    );
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
    for expression in ["pair(true,'x')", "keep(null)"] {
        sandbox.write("input.fl",&format!("enum Maybe where T {{Absent,Present(T)}} fn keep(value:?T)->?T where T {{value}} fn pair(value:?T,other:T)->?T where T {{value}} fn run()->bool {{let result={expression};true}}"));
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert!(!output.status.success(), "{expression}");
    }
}

#[test]
fn optional_propagation_has_a_structured_hir_dump_and_converts_missing_result_type() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "enum Maybe where T {Absent,Present(T)}
        fn lookup(flag:bool)->?bool {if flag {true} else {null}}
        fn convert(flag:bool)->?.u8'builtin {let value=lookup(flag)?;if value {9} else {8}}
        comptime fn absent()->?bool {null}
        comptime fn propagate()->?.u8'builtin {let value=absent()?;9}
        comptime fn check()->bool {propagate() match {_?=>false,null=>true}}
        fn run()->bool {(convert(false) match {_?=>false,null=>true}) and
            (convert(true) match {value?=>value==9,null=>false})}",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    assert!(hir_dump(&output).contains("(option-propagate (value "));
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "check"]);
    assert_eq!(success(&output).trim(), "(bool true)");
}

#[test]
fn inferred_optional_callbacks_complete_missing_returns_from_success_paths() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl","enum Maybe where T {Absent,Present(T)}
        fn lookup(flag:bool)->?bool {if flag {true} else {null}}
        fn apply(action:fn(bool)->?T,flag:bool)->?T where T {action(flag)}
        fn run()->bool {
            let yes=apply(|flag| {let value=lookup(flag)?;not value},true);
            let no=apply(|flag| {let value=lookup(flag)?;not value},false);
            let byte=apply(|flag| {lookup(flag)?;let result:.u8'builtin=9;result},true);
            let missing_byte=apply(|flag| {lookup(flag)?;let result:.u8'builtin=9;result},false);
            let nested=apply(|flag| {let inner=apply(|other| {let value=lookup(other)?;not value},flag);let value=inner?;value},true);
            let number=apply(|flag| {lookup(flag)?;9},true);
            (nested match {value?=>not value,null=>false}) and (number match {value?=>value==9,null=>false}) and
            (yes match {value?=>not value,null=>false}) and (no match {_?=>false,null=>true}) and
            (byte match {value?=>value==9,null=>false}) and (missing_byte match {_?=>false,null=>true})}");
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
    sandbox.write(
        "input.fl",
        "enum Maybe where T {Absent,Present(T)}
        fn lookup(flag:bool)->?bool {null}
        fn apply(action:fn(bool)->?T)->?T where T {action(true)}
        fn run()->bool {let value=apply(|flag| {lookup(flag)?;null});true}",
    );
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert!(!output.status.success());
}

#[test]
fn inferred_optional_callbacks_defer_explicit_missing_returns() {
    let sandbox = Sandbox::new();
    let prefix = "enum Maybe where T {Absent,Present(T)}
        fn lookup(flag:bool)->?bool {if flag {true} else {null}}
        fn apply(action:fn(bool)->?T,flag:bool)->?T where T {action(flag)}";
    for body in [
        "if not flag {return null;} lookup(flag)?;9",
        "if not flag {return null;} lookup(flag)?;return 9;",
        "if flag {return 9;} return null;",
        "if not flag {return null;} let inner=apply(|other| {if not other {return null;} true},flag);inner?;9",
    ] {
        sandbox.write(
            "input.fl",
            &format!(
                "{prefix}
            fn run()->bool {{
                let yes=apply(|flag| {{{body}}},true);
                let no=apply(|flag| {{{body}}},false);
                (yes match {{value?=>value==9,null=>false}}) and
                (no match {{_?=>false,null=>true}})}}"
            ),
        );
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert_eq!(success(&output).trim(), "(bool true)", "{body}");
    }
    for body in [
        "return null;",
        "if flag {return null;} lookup(flag)?;return null;",
        "if flag {return null;} let inner=apply(|other| {true},flag);return null;",
        "if flag {return null;} if flag {return true;} return 9;",
    ] {
        sandbox.write(
            "input.fl",
            &format!(
                "{prefix}
            fn run()->bool {{let value=apply(|flag| {{{body}}},true);true}}"
            ),
        );
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert!(!output.status.success(), "accepted {body}");
    }
}

#[test]
fn partial_optional_branches_infer_from_later_success_values() {
    let sandbox = Sandbox::new();
    let prefix = "enum Maybe where T {Absent,Present(T)}
        fn keep(value:?T)->?T where T {value}
        fn apply(action:fn(bool)->?T,flag:bool)->?T where T {action(flag)}
        fn captured(action:%{base:.u8'builtin} fn(bool)->?T,flag:bool)->?T where T {action(flag)}";
    for body in [
        "if flag {null} else {9}",
        "if flag {null} else {base}",
        "flag match {true=>null,false=>9}",
        "when {flag=>null,else=>9}",
        "if flag {let unused=true;null} else {let value:.u8'builtin=9;value}",
        "flag match {true=>null,false=>{let value:.u8'builtin=9;value}}",
        "when {flag=>null,else=>if flag {null} else {9}}",
    ] {
        for flag in ["true", "false"] {
            let apply = if body.contains("base") {
                "captured"
            } else {
                "apply"
            };
            for input in [
                format!("keep({body})"),
                format!("{apply}(|flag| {{{body}}},flag)"),
            ] {
                sandbox.write(
                    "input.fl",
                    &format!(
                        "{prefix} fn run()->bool {{let flag={flag};let base:.u8'builtin=9;
                    let result={input};result match {{value?=>not flag and value==9,null=>flag}}}}"
                    ),
                );
                let output = sandbox.run(&["input.fl", "--interpret", "run"]);
                assert_eq!(success(&output).trim(), "(bool true)", "{input}, {flag}");
            }
        }
    }
}

#[test]
fn optional_propagation_supplies_context_to_generic_producers() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "typealias Byte=.u8'builtin;
        enum Maybe where T {Absent,Present(T)}
        fn empty()->?T where T {null}
        fn consume(value:bool)->Byte {9}
        fn local()->?Byte {let value:bool=empty()?;9}
        fn argument()->?Byte {consume(empty()?)}
        fn symbol()->?Byte {let value:bool=null?;9}
        fn nested()->??Byte {let value:?Byte=empty()?;value}
        fn run()->bool {
            (local() match {_?=>false,null=>true}) and
            (argument() match {_?=>false,null=>true}) and
            (symbol() match {_?=>false,null=>true}) and
            (nested() match {_?=>false,null=>true})}",
    );
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
}

#[test]
fn optional_branch_probes_reach_later_evidence_through_nested_nulls() {
    let sandbox = Sandbox::new();
    for shape in ["if", "match", "when"] {
        let mut missing = "null".to_string();
        for _ in 0..32 {
            missing = match shape {
                "if" => format!("if flag {{{missing}}} else {{null}}"),
                "match" => format!("flag match {{true=>{missing},false=>null}}"),
                _ => format!("when {{flag=>{missing},else=>null}}"),
            };
        }
        for flag in ["true", "false"] {
            sandbox.write(
                "input.fl",
                &format!(
                    "enum Maybe where T {{Absent,Present(T)}}
                fn keep(value:?T)->?T where T {{value}}
                fn run()->bool {{let flag={flag};let byte:.u8'builtin=9;
                    let value=keep(if flag {{{missing}}} else {{byte}});
                    value match {{value?=>not flag and value==9,null=>flag}}}}"
                ),
            );
            let output = sandbox.run(&["input.fl", "--interpret", "run"]);
            assert_eq!(success(&output).trim(), "(bool true)", "{shape}, {flag}");
        }
    }
    sandbox.write(
        "input.fl",
        "enum Maybe where T {Absent,Present(T)}
        fn apply(action:?(fn(bool)->?T))->?T where T {let action=action?;action(true)}
        fn run()->bool {let value=apply(if false {null} else {
            |flag| {if flag {return true;} else {return false;}}
        });value match {value?=>value,null=>false}}",
    );
    let output = sandbox.run(&["input.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
    for missing in [
        "true match {true=>null}",
        "if true {null} else {.Absent(9)}",
    ] {
        sandbox.write(
            "input.fl",
            &format!(
                "enum Maybe where T {{Absent,Present(T)}}
            fn keep(value:?T)->?T where T {{value}}
            fn run()->bool {{let value=keep(if false {{{missing}}} else {{true}});true}}"
            ),
        );
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert!(!output.status.success(), "accepted {missing}");
    }
}

#[test]
fn optional_callback_tails_use_explicit_return_payload_evidence() {
    let sandbox = Sandbox::new();
    let prefix = "enum Maybe where T {Absent,Present(T)}
        fn lookup(flag:bool)->?bool {if flag {true} else {null}}
        fn apply(action:fn(bool)->?T,flag:bool)->?T where T {action(flag)}";
    for body in [
        "if flag {return 9;} null",
        "let byte:.u8'builtin=9;if flag {return byte;} null",
        "if flag {return 9;} if flag {null} else {null}",
        "if flag {return 9;} flag match {true=>null,false=>null}",
        "if flag {return 9;} when {flag=>null,else=>null}",
        "lookup(flag)?;if flag {return 9;} null",
    ] {
        for flag in ["true", "false"] {
            sandbox.write(
                "input.fl",
                &format!(
                    "{prefix} fn run()->bool {{
                let result=apply(|flag| {{{body}}},{flag});
                result match {{value?=>{flag} and value==9,null=>not {flag}}}}}"
                ),
            );
            let output = sandbox.run(&["input.fl", "--interpret", "run"]);
            assert_eq!(success(&output).trim(), "(bool true)", "{body}, {flag}");
        }
    }
}

#[test]
fn shipped_clone_protocol_preserves_noncopy_sources() {
    let sandbox = Sandbox::new();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in [
        "primitives",
        "traits",
        "resources",
        "comparison",
        "conversion",
    ] {
        fs::copy(
            repository.join(format!("library/builtin/{name}.fl")),
            sandbox.path(format!("{name}.fl")),
        )
        .unwrap();
    }
    // Formatting is outside this fixture. This local type only supplies the
    // opaque signatures of Display/Debug; neither formatting method is called.
    let prefix="pub use primitives.*;pub use traits.*;struct String {}
        struct Ticket {flag:bool}
        impl Once for Ticket {}
        impl Clone for Ticket {fn clone(*self)->Self {Ticket {flag:self.flag}}}
        impl Drop for Ticket {fn drop(*self) {}}
        fn require_once(value:*T) where T :- Once {}
        fn clone_generic(value:*T)->T where T :- Clone {require_once(value);value.*.as(Clone).clone()}";
    sandbox.write(
        "lib.fl",
        &format!(
            "{prefix}
        fn run()->bool {{let source=Ticket {{flag:true}};
            let first=source.as(Clone).clone();let second=clone_generic(source.ref);
            source.flag and first.flag and second.flag}}"
        ),
    );
    let output = sandbox.run(&["lib.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
    // The concrete method footprint must also pass resource-checked execution.
    let checked = sandbox.run(&[
        "lib.fl",
        "--interpret",
        "run",
        "--copy-protocol",
        "Copy",
        "--drop-protocol",
        "Drop",
    ]);
    assert_eq!(success(&checked).trim(), "(bool true)");
    for invalid in [
        prefix.replace("impl Once for Ticket {}", ""),
        prefix.replace("fn clone(*self)->Self", "fn clone(self)->Self"),
    ] {
        sandbox.write("lib.fl", &format!("{invalid} fn run()->bool {{true}}"));
        assert!(
            !sandbox
                .run(&["lib.fl", "--interpret", "run"])
                .status
                .success()
        );
    }
    sandbox.write("lib.fl","pub use primitives.*;pub use traits.*;struct String {}
        struct Key {value:u8}
        impl Eq for Key {fn eq(self,other:Self)->bool {self.value == other.value}}
        impl Ord for Key {fn cmp(self,other:Self)->Order {
            if self.value < other.value {.less} else if self.value == other.value {.equal} else {.greater}
        }}
        impl Hash for Key {fn hash(self)->u64 {7}}
        fn run()->bool {
            Key {value:1}.as(Ord).lt(Key {value:2}) and
            not Key {value:2}.as(Ord).lt(Key {value:2}) and
            not Key {value:3}.as(Ord).lt(Key {value:2}) and
            Key {value:1}.as(Ord).le(Key {value:2}) and
            Key {value:2}.as(Ord).le(Key {value:2}) and
            not Key {value:3}.as(Ord).le(Key {value:2}) and
            not Key {value:1}.as(Ord).gt(Key {value:2}) and
            not Key {value:2}.as(Ord).gt(Key {value:2}) and
            Key {value:3}.as(Ord).gt(Key {value:2}) and
            not Key {value:1}.as(Ord).ge(Key {value:2}) and
            Key {value:2}.as(Ord).ge(Key {value:2}) and
            Key {value:3}.as(Ord).ge(Key {value:2}) and
            Key {value:1}.as(Hash).hash() == 7}");
    let output = sandbox.run(&["lib.fl", "--interpret", "run"]);
    assert_eq!(success(&output).trim(), "(bool true)");
}

#[test]
fn field_pointer_reads_require_a_target_and_copy_payload() {
    let sandbox = Sandbox::new();
    for definition in [
        "struct Owned {} struct Cell {value:Owned} fn read(cell:*Cell)->Owned {cell.value}",
        "struct Cell {value:*bool} fn read(cell:*Cell)->.u8'builtin {cell.value}",
        "struct Cell {value:bool} fn bad(cell:*Cell) {cell.value=true;}",
    ] {
        sandbox.write("input.fl", definition);
        assert!(
            !sandbox.run(&["input.fl", "--emit", "nir"]).status.success(),
            "accepted {definition}"
        );
    }
    sandbox.write("input.fl","fn packet(...properties:T)->T where T {properties}
        fn run()->bool {let flag=true;let middle=flag.ref;let record=packet {value:middle.ref};
            let pointer=record.ref;let value:bool=pointer.value;let raw=pointer.value;value and raw.*.*.*}");
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    sandbox.write(
        "input.fl",
        "struct Cell {value:*bool}
        fn identity(value:T)->T where T {value}
        fn read(cell:*Cell)->bool {cell.value}
        fn run()->bool {let flag=true;let cell=Cell {value:flag.ref};let pointer=cell.ref;
            let value=identity(pointer.value);value.*.* and read(pointer)}",
    );
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--interpret", "run"])).trim(),
        "(bool true)"
    );
}

#[test]
fn sysroot_language_items_bind_real_list_and_tuple_definitions_by_default() {
    let sandbox = Sandbox::new();
    for name in ["builtin", "std", "meta"] {
        fs::create_dir_all(sandbox.path(&format!("library/{name}"))).unwrap();
    }
    sandbox.write("library/builtin/lib.fl","pub typealias Type=.Type'builtin;pub use tuple.Tuple;pub use ops.*;pub use comparison.*;pub mod math {pub use math_intrinsics.*;} pub mod prelude {pub use tuple.Tuple;}");
    sandbox.write(
        "library/builtin/resources.fl",
        include_str!("../../../library/builtin/resources.fl"),
    );
    sandbox.write(
        "library/builtin/tuple.fl",
        include_str!("../../../library/builtin/tuple.fl"),
    );
    fs::create_dir_all(sandbox.path("library/attrs")).unwrap();
    sandbox.write(
        "library/attrs/lib.fl",
        include_str!("../../../library/attrs/lib.fl"),
    );
    sandbox.write(
        "library/builtin/comparison.fl",
        include_str!("../../../library/builtin/comparison.fl"),
    );
    sandbox.write(
        "library/builtin/ops.fl",
        include_str!("../../../library/builtin/ops.fl"),
    );
    sandbox.write("library/meta/lib.fl", include!("support/meta_source.rs"));
    sandbox.write("library/std/lib.fl", "pub mod prelude {}");
    sandbox.write(
        "library/language-items.conf",
        // This fixture supplies list/tuple and numeric protocols, not the
        // iterator or list-pattern implementations from the complete sysroot.
        include_str!("../../../library/language-items.conf")
            .lines()
            .filter(|line| {
                !line.starts_with("iterator-protocol =")
                    && !line.starts_with("into-iterator-protocol =")
                    && !line.starts_with("list-pattern-protocol =")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    sandbox.write(
        "library/builtin/math_intrinsics.fl",
        include_str!("../../../library/builtin/math_intrinsics.fl"),
    );
    sandbox.write("input.fl","fn pair()->Tuple<bool,.u8'builtin> {(true,9)} fn present()->?bool {true} fn run()->bool {let (flag,byte)=pair();flag and byte==9 and (present() match {value?=>value,null=>false})}");
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    let output = sandbox.run(&["input.fl", "--sysroot", "library", "--emit", "nir"]);
    let dump = success(&output);
    assert!(dump.contains("normal-form"), "{dump}");
    assert!(dump.contains("type-list-value"), "{dump}");
    let original = fs::read_to_string(sandbox.path("input.fl")).unwrap();
    sandbox.write("input.fl","use builtin.Add;typealias Type=.Type'builtin;
        struct Flag {value:bool}
        impl Add<.Rhs=bool> for Flag {assoc Output:Type=bool;fn add(self,rhs:bool)->bool {self.value and rhs}}
        fn run()->bool {Flag {value:true} + true}");
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    sandbox.write("input.fl", "struct Flag {value:bool}
        impl builtin.Eq for Flag {fn eq(self,other:Self)->bool {self.value==other.value}}
        impl builtin.Ord for Flag {fn cmp(self,other:Self)->builtin.Order {
            if not self.value and other.value {.less} else if self.value==other.value {.equal} else {.greater}
        }}
        fn run()->bool {Flag {value:false} < Flag {value:true} and Flag {value:true}==Flag {value:true} and Flag {value:false}!=Flag {value:true}}");
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    for (trait_name, method, symbol) in [("Neg", "neg", "-"), ("Not", "invert", "not ")] {
        sandbox.write("input.fl",format!("use builtin.{trait_name};typealias Type=.Type'builtin;
            struct Flag {{value:bool}}
            impl {trait_name} for Flag {{assoc Output:Type=bool;fn {method}(self)->bool {{self.value}}}}
            fn run()->bool {{ {symbol}Flag {{value:true}} }}"));
        assert_eq!(
            success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"]))
                .trim(),
            "(bool true)"
        );
    }
    sandbox.write("input.fl", original);
    // Defaults do not override explicit driver choices.
    sandbox.write(
        "library/language-items.conf",
        format!(
            "version = 1\nmeta-list-type = meta.List\ntuple-type = missing.Tuple\n{}",
            include_str!("../../../library/language-items.conf")
                .lines()
                .filter(|line| line.starts_with("host-intrinsic."))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    failure(
        &sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"]),
        1,
    );
    assert_eq!(
        success(&sandbox.run(&[
            "input.fl",
            "--sysroot",
            "library",
            "--interpret",
            "run",
            "--tuple-type",
            "builtin.tuple.Tuple"
        ]))
        .trim(),
        "(bool true)"
    );
    sandbox.write(
        "library/language-items.conf",
        include_str!("../../../library/language-items.conf")
            .lines()
            .filter(|line| {
                !line.starts_with("iterator-protocol =")
                    && !line.starts_with("into-iterator-protocol =")
                    && !line.starts_with("list-pattern-protocol =")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    sandbox.write(
        "library/builtin/math_intrinsics.fl",
        include_str!("../../../library/builtin/math_intrinsics.fl"),
    );
    sandbox.write("input.fl","typealias Type=.Type'builtin;pure comptime fn Tuple<...Es:meta.List<Type>>->Type {Tuple<...Es>} fn run()->Tuple<bool> {(true,)}");
    let shadow = sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"]);
    failure(&shadow, 1);
    assert!(String::from_utf8_lossy(&shadow.stderr).contains("multi-package elaboration failed"));
}

#[test]
fn sysroot_language_item_configuration_is_validated_without_clobbering_output() {
    let sandbox = Sandbox::new();
    for name in ["builtin", "std"] {
        fs::create_dir_all(sandbox.path(&format!("library/{name}"))).unwrap();
    }
    sandbox.write("library/builtin/lib.fl", "pub mod prelude {}");
    sandbox.write("library/std/lib.fl", "");
    sandbox.write("input.fl", "fn run()->bool {true}");
    for config in [
        "",
        "version = 2",
        "version = 1\nversion = 1",
        "version = 1\nunknown = x.T",
        "version = 1\ntuple-type = Tuple",
        "version = 1\ntuple-type = x..T",
        "version = 1\ntuple-type = x.T\ntuple-type = x.T",
        "version = 1\ntuple-type x.T",
    ] {
        sandbox.write("library/language-items.conf", config);
        sandbox.write("dump.lisp", OLD_OUTPUT);
        let output = sandbox.run(&[
            "input.fl",
            "--sysroot",
            "library",
            "--emit",
            "nir",
            "--output",
            "dump.lisp",
        ]);
        failure(&output, 1);
        assert!(String::from_utf8_lossy(&output.stderr).contains("language-items.conf"));
        assert_eq!(fs::read(sandbox.path("dump.lisp")).unwrap(), OLD_OUTPUT);
    }
    sandbox.write(
        "library/language-items.conf",
        format!("version = 1\n#{}", "x".repeat(65536)),
    );
    let oversized = sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"]);
    failure(&oversized, 1);
    assert!(String::from_utf8_lossy(&oversized.stderr).contains("size limit"));
    let valid = "version = 1\n# No type constructors selected here.\n";
    sandbox.write("library/language-items.conf", valid);
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    fs::hard_link(
        sandbox.path("library/language-items.conf"),
        sandbox.path("config-alias"),
    )
    .unwrap();
    for output in ["library/language-items.conf", "config-alias"] {
        failure(
            &sandbox.run(&[
                "input.fl",
                "--sysroot",
                "library",
                "--emit",
                "nir",
                "--output",
                output,
            ]),
            1,
        );
        assert_eq!(
            fs::read_to_string(sandbox.path("library/language-items.conf")).unwrap(),
            valid
        );
    }
}

#[test]
fn defer_retains_hir_boundaries_and_resolves_its_operand() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "typealias Type=.Type'builtin;
        comptime fn Select(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn keep(comptime flag:bool,value:defer Select(flag))->defer Select(flag) {value}",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    // The parameter appears in both signature and body metadata; the result
    // annotation appears only in the signature.
    assert_eq!(dump.matches("(defer ").count(), 3, "{dump}");
    assert!(dump.contains("application"), "{dump}");
    sandbox.write("input.fl", "fn missing(value:defer UnknownType(false)) {}");
    failure(&sandbox.run(&["input.fl", "--emit", "hir"]), 1);
}

#[test]
fn arithmetic_protocol_cli_checks_binding_shape_and_source_identity() {
    let sandbox = Sandbox::new();
    let source="typealias Type=.Type'builtin;
        trait Operation where Rhs {assoc Output:Type;fn add(self,rhs:Rhs)->Output;}
        struct Flag {value:bool}
        impl Operation<bool> for Flag {assoc Output:Type=bool;fn add(self,rhs:bool)->bool {self.value and rhs}}
        fn native()->.u8'builtin {7 + 1}
        fn run()->bool {(Flag {value:true} + true) and native() == 8}";
    sandbox.write("input.fl", source);
    assert_eq!(
        success(&sandbox.run(&[
            "input.fl",
            "--add-protocol",
            "input.Operation",
            "--interpret",
            "run"
        ]))
        .trim(),
        "(bool true)"
    );
    failure(&sandbox.run(&["input.fl", "--interpret", "run"]), 1);
    failure(
        &sandbox.run(&[
            "input.fl",
            "--add-protocol",
            "input.Flag",
            "--interpret",
            "run",
        ]),
        1,
    );
    failure(
        &sandbox.run(&[
            "input.fl",
            "--add-protocol",
            "missing.Operation",
            "--interpret",
            "run",
        ]),
        1,
    );
    failure(
        &sandbox.run(&[
            "input.fl",
            "--add-protocol",
            "input.Operation",
            "--add-protocol",
            "input.Operation",
        ]),
        2,
    );
    failure(&sandbox.run(&["input.fl", "--add-protocol"]), 2);
    for declaration in [
        "trait Operation {assoc Output:Type;fn add(self,rhs:Self)->Output;}",
        "trait Operation where Rhs {fn add(self,rhs:Rhs)->bool;}",
        "trait Operation where Rhs {assoc Output:Type;fn add(*self,rhs:Rhs)->Output;}",
        "trait Operation where Rhs {assoc Output:Type;fn add(self,rhs:bool)->Output;}",
        "trait Operation where Rhs {assoc Output:Type;fn add(self,rhs:Rhs)->bool;}",
    ] {
        sandbox.write(
            "input.fl",
            format!("typealias Type=.Type'builtin;{declaration} fn run()->bool {{true}}"),
        );
        let output = sandbox.run(&[
            "input.fl",
            "--add-protocol",
            "input.Operation",
            "--interpret",
            "run",
        ]);
        failure(&output, 1);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("arithmetic"),
            "{declaration}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn unary_protocol_cli_validates_identity_shape_and_native_fallback() {
    let sandbox = Sandbox::new();
    for (option, method, symbol) in [
        ("--neg-protocol", "neg", "-"),
        ("--not-protocol", "invert", "not "),
    ] {
        let source=format!("typealias Type=.Type'builtin;
            trait Operation {{assoc Output:Type;fn {method}(self)->Output;}}
            struct Flag {{value:bool}}
            impl Operation for Flag {{assoc Output:Type=bool;fn {method}(self)->bool {{self.value}}}}
            fn run()->bool {{ {symbol}Flag {{value:true}} }}
            fn minimum()->.i8'builtin {{ -128 }}
            fn negative()->.i8'builtin {{let value:.i8'builtin=7; -value}}
            fn native_not()->bool {{not false}}
            fn bits()->.BitVec'builtin<8> {{not 0b00001111}}");
        sandbox.write("input.fl", &source);
        for (entry, expected) in [
            ("run", "(bool true)"),
            ("minimum", "(signed-integer -128)"),
            ("negative", "(signed-integer -7)"),
            ("native_not", "(bool true)"),
        ] {
            let output =
                sandbox.run(&["input.fl", option, "input.Operation", "--interpret", entry]);
            let result = success(&output);
            assert_eq!(result.trim(), expected, "{option} {entry}");
        }
        let bits = sandbox.run(&["input.fl", option, "input.Operation", "--interpret", "bits"]);
        assert_eq!(
            success(&bits).trim(),
            "(bitvec (width 8) (hex \"000000f0\"))"
        );
        failure(&sandbox.run(&["input.fl", "--interpret", "run"]), 1);
        failure(
            &sandbox.run(&["input.fl", option, "input.Flag", "--emit", "nir"]),
            1,
        );
        failure(
            &sandbox.run(&["input.fl", option, "missing.Operation", "--emit", "nir"]),
            1,
        );
        failure(
            &sandbox.run(&[
                "input.fl",
                option,
                "input.Operation",
                option,
                "input.Operation",
            ]),
            2,
        );
        failure(&sandbox.run(&["input.fl", option]), 2);
        for declaration in [
            format!("trait Operation where T {{assoc Output:Type;fn {method}(self)->Output;}}"),
            format!("trait Operation {{fn {method}(self)->bool;}}"),
            format!("trait Operation {{assoc Output:Type;fn {method}(*self)->Output;}}"),
            format!("trait Operation {{assoc Output:Type;fn {method}(self,rhs:bool)->Output;}}"),
            format!("trait Operation {{assoc Output:Type;fn {method}(self)->bool;}}"),
        ] {
            sandbox.write(
                "input.fl",
                format!("typealias Type=.Type'builtin;{declaration} fn run()->bool {{true}}"),
            );
            let output =
                sandbox.run(&["input.fl", option, "input.Operation", "--interpret", "run"]);
            failure(&output, 1);
            assert!(String::from_utf8_lossy(&output.stderr).contains("protocol"));
        }
    }
}

#[test]
fn index_protocol_cli_checks_binding_and_declaration_shape() {
    let sandbox = Sandbox::new();
    let source = "trait Access where K {fn index(self,idx:K)->bool;}
        struct Flag {value:bool}
        impl Access<bool> for Flag {fn index(self,idx:bool)->bool {self.value and idx}}
        fn run()->bool {Flag {value:true}[true]}";
    sandbox.write("input.fl", source);
    assert_eq!(
        success(&sandbox.run(&[
            "input.fl",
            "--index-protocol",
            "input.Access",
            "--interpret",
            "run"
        ]))
        .trim(),
        "(bool true)"
    );
    failure(&sandbox.run(&["input.fl", "--interpret", "run"]), 1);
    for selector in [
        "input.Flag",
        "missing.Access",
        "Access",
        "input.",
        "input.missing.Access",
    ] {
        failure(
            &sandbox.run(&[
                "input.fl",
                "--index-protocol",
                selector,
                "--interpret",
                "run",
            ]),
            1,
        );
    }
    for args in [
        vec!["input.fl", "--index-protocol"],
        vec!["input.fl", "--index-protocol", "input.Access"],
        vec![
            "input.fl",
            "--index-protocol",
            "input.Access",
            "--index-protocol",
            "input.Access",
            "--interpret",
            "run",
        ],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    for declaration in [
        "trait Access {fn index(self,idx:bool)->bool;}",
        "trait Access where K {fn other(self,idx:K)->bool;}",
        "trait Access where K {fn index(*self,idx:K)->bool;}",
        "trait Access where K {fn index(self,idx:bool)->bool;}",
    ] {
        sandbox.write("input.fl", format!("{declaration}fn run()->bool {{true}}"));
        let output = sandbox.run(&[
            "input.fl",
            "--index-protocol",
            "input.Access",
            "--interpret",
            "run",
        ]);
        failure(&output, 1);
        assert!(String::from_utf8_lossy(&output.stderr).contains("Index protocol"));
    }
}

#[test]
fn index_protocol_sysroot_defaults_preserve_explicit_pointer_access() {
    let sandbox = Sandbox::new();
    for name in ["builtin", "std"] {
        fs::create_dir_all(sandbox.path(&format!("library/{name}"))).unwrap();
    }
    sandbox.write(
        "library/builtin/lib.fl",
        "pub typealias Type=.Type'builtin;pub use ops.*;pub mod prelude {}",
    );
    sandbox.write(
        "library/builtin/ops.fl",
        include_str!("../../../library/builtin/ops.fl"),
    );
    sandbox.write("library/std/lib.fl", "pub mod prelude {}");
    sandbox.write(
        "library/language-items.conf",
        "version = 1\nindex-protocol = builtin.ops.Index\n",
    );
    sandbox.write(
        "input.fl",
        "use builtin.Index;typealias Type=.Type'builtin;typealias Word=.u16'builtin;
        struct Slot {p:*Word}
        impl Index<bool> for Slot {assoc Output:Type=Word;fn index(self,idx:bool)->*Word {self.p}}
        fn run()->Word {var word:Word=0;Slot {p:word.ref}[true].* = 17;word}",
    );
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(unsigned-integer 17)"
    );
    let output = sandbox.run(&[
        "input.fl",
        "--sysroot",
        "library",
        "--interpret",
        "run",
        "--index-protocol",
        "input.Missing",
    ]);
    failure(&output, 1);
}

#[test]
fn checked_interpret_supplies_resource_identities_before_pure_validation() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "pure comptime fn List<T>->.Type'builtin {T} pure comptime fn Tuple<...Es:List<.Type'builtin>>->.Type'builtin {bool} trait Duplicate {} trait Retire {fn drop(*self);} struct Flag {value:bool} impl Duplicate for Flag {} pure fn duplicate(value:Flag)->Tuple<Flag,Flag> {(value,value)} fn run()->bool {let (first,second)=duplicate(Flag {value:true});first.value and second.value}");
    let result = sandbox.run(&[
        "input.fl",
        "--meta-list-type",
        "input.List",
        "--tuple-type",
        "input.Tuple",
        "--interpret",
        "run",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
    ]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap().trim(),
        "(bool true)"
    );
}

#[test]
fn optional_inlining_cli_requires_checked_execution_and_preserves_cleanup() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {ready:bool}
        impl Retire for Held {fn drop(*self) {assumes self.ready.*;}}
        inline fn consume(held:Held)->bool {true}
        fn entry()->bool {consume(Held {ready:true})}",
    );
    let output = sandbox.run(&[
        "input.fl",
        "--interpret",
        "entry",
        "--inline-functions",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
    ]);
    assert_eq!(success(&output).trim(), "(bool true)");
    let reordered = sandbox.run(&[
        "--inline-functions",
        "--drop-protocol",
        "Retire",
        "input.fl",
        "--copy-protocol",
        "Duplicate",
        "--interpret",
        "entry",
    ]);
    assert_eq!(success(&reordered).trim(), "(bool true)");
    for args in [
        vec!["input.fl", "--inline-functions"],
        vec!["input.fl", "--interpret", "entry", "--inline-functions"],
        vec![
            "input.fl",
            "--interpret",
            "entry",
            "--inline-functions",
            "--inline-functions",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ],
        vec![
            "input.fl",
            "--instantiate",
            "entry",
            "--inline-functions",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    sandbox.write(
        "input.fl",
        "trait Duplicate {} trait Retire {fn drop(*self);}
        struct Held {ready:bool}
        impl Retire for Held {fn drop(*self) {assumes self.ready.*;}}
        inline fn consume(held:Held)->bool {true}
        fn entry()->bool {consume(Held {ready:false})}",
    );
    let output = sandbox.run(&[
        "input.fl",
        "--interpret",
        "entry",
        "--inline-functions",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
    ]);
    failure(&output, 1);
}

#[test]
fn literal_into_cli_runs_suffix_conversion_and_checks_usage() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/literal-suffix.fl"),
    );
    assert_eq!(
        success(&sandbox.run(&[
            "input.fl",
            "--literal-into-protocol",
            "input.Convert",
            "--interpret",
            "run"
        ]))
        .trim(),
        "(bool true)"
    );
    failure(&sandbox.run(&["input.fl", "--interpret", "run"]), 1);
    for selector in ["Convert", "input.Missing", "missing.Convert", "input.run"] {
        failure(
            &sandbox.run(&[
                "input.fl",
                "--literal-into-protocol",
                selector,
                "--interpret",
                "run",
            ]),
            1,
        );
    }
    for args in [
        vec!["input.fl", "--literal-into-protocol"],
        vec!["input.fl", "--literal-into-protocol", "input.Convert"],
        vec![
            "input.fl",
            "--literal-into-protocol",
            "input.Convert",
            "--literal-into-protocol",
            "input.Convert",
            "--emit",
            "nir",
        ],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    assert!(success(&sandbox.run(&["--help"])).contains("--literal-into-protocol"));
}

#[test]
fn literal_into_cli_overrides_sysroot_defaults() {
    let sandbox = Sandbox::new();
    for name in ["builtin", "std"] {
        fs::create_dir_all(sandbox.path(&format!("library/{name}"))).unwrap();
    }
    sandbox.write("library/builtin/lib.fl", "pub mod prelude {} pub typealias Symbol=.Symbol'builtin;typealias Char=.char'builtin; pub trait Convert where suffix:Symbol,R {fn literal_into(comptime value:Self)->R;} impl Convert<.flag,bool> for Char {fn literal_into(comptime value:Char)->bool {false}}");
    sandbox.write("library/std/lib.fl", "pub mod prelude {}");
    sandbox.write(
        "library/language-items.conf",
        "version = 1\nliteral-into-protocol = builtin.Convert\n",
    );
    sandbox.write(
        "input.fl",
        "use builtin.Convert;typealias Symbol=.Symbol'builtin;typealias Char=.char'builtin;
        trait Custom where suffix:Symbol,R {fn literal_into(comptime value:Self)->R;}
        impl Custom<.flag,bool> for Char {fn literal_into(comptime value:Char)->bool {true}}
        fn run()->bool {'x'flag}",
    );
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool false)"
    );
    assert_eq!(
        success(&sandbox.run(&[
            "input.fl",
            "--sysroot",
            "library",
            "--interpret",
            "run",
            "--literal-into-protocol",
            "input.Custom"
        ]))
        .trim(),
        "(bool true)"
    );
}

#[test]
fn literal_suffix_result_inference_requires_a_unique_complete_candidate() {
    let sandbox = Sandbox::new();
    let base = include_str!("../../../development/examples/literal-suffix.fl");
    let inferred = base.replace("'x'flag\n}", "let result = 'x'flag; result\n}");
    assert_ne!(base, inferred);
    sandbox.write("input.fl", &inferred);
    let args = [
        "input.fl",
        "--literal-into-protocol",
        "input.Convert",
        "--interpret",
        "run",
    ];
    assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    let second =
        "impl Convert<.flag,Char> for Char {fn literal_into(comptime value:Char)->Char {value}}";
    sandbox.write("input.fl", format!("{inferred}\n{second}"));
    let ambiguous = sandbox.run(&args);
    failure(&ambiguous, 1);
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("ambiguous or unresolved"));
    sandbox.write("input.fl", format!("{base}\n{second}"));
    assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    let contextual_call = base.replace("'x'flag\n}", "identity('x'flag)\n}");
    sandbox.write(
        "input.fl",
        format!("{contextual_call}\n{second}\nfn identity(value:T)->T where T {{value}}"),
    );
    assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    // A different suffix cannot pollute the result choice, regardless of order.
    for source in [
        format!("{}\n{inferred}", second.replace(".flag,", ".other,")),
        format!("{inferred}\n{}", second.replace(".flag,", ".other,")),
    ] {
        sandbox.write("input.fl", source);
        assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    }
    let scoped = format!("{inferred}\n{second}").replace(
        "fn run() -> bool",
        "fn selected() -> bool where requires Char :- Convert<.flag,bool>",
    );
    sandbox.write(
        "input.fl",
        format!("{scoped}\nfn run()->bool {{selected()}}"),
    );
    assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    sandbox.write("input.fl", "typealias Symbol=.Symbol'builtin;typealias Char=.char'builtin;
        trait Convert where suffix:Symbol,R {fn literal_into(comptime value:Self)->R;}
        impl Convert<.flag,R> for Char where R {fn literal_into(comptime value:Char)->R {while true {}}}
        fn run()->bool {let result='x'flag;result}");
    let unresolved = sandbox.run(&args);
    failure(&unresolved, 1);
    assert!(
        String::from_utf8_lossy(&unresolved.stderr).contains("ambiguous or unresolved"),
        "{}",
        details(&unresolved)
    );
    let generic = inferred
        .replace("for Char", "for T where T")
        .replace("comptime value: Char", "comptime value: T")
        .replace("value == 'x'", "true");
    sandbox.write("input.fl", generic);
    assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
}

#[test]
fn literal_result_inference_uses_implementation_equalities_without_assuming_them() {
    let sandbox = Sandbox::new();
    let source = "typealias Symbol=.Symbol'builtin;typealias Char=.char'builtin;
        trait Convert where suffix:Symbol,R {fn literal_into(comptime value:Self)->R;}
        impl Convert<.flag,R> for Char where R, requires R == bool {
            fn literal_into(comptime value:Char)->R {true}
        }
        fn run()->bool {let value='x'flag;value}";
    let args = [
        "input.fl",
        "--literal-into-protocol",
        "input.Convert",
        "--interpret",
        "run",
    ];
    sandbox.write("input.fl", source);
    assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    let impossible = "impl Convert<.flag,R> for Char where R, requires (R == bool) and (R == Char) {fn literal_into(comptime value:Char)->R {while true {}}}";
    for source in [
        format!("{source}\n{impossible}"),
        format!("{impossible}\n{source}"),
    ] {
        sandbox.write("input.fl", source);
        assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    }

    let disjunction = source
        .replace("requires R == bool", "requires (R == bool) or (R == Char)")
        .replace("->R {true}", "->R {while true {}}");
    sandbox.write("input.fl", disjunction);
    let unresolved = sandbox.run(&args);
    failure(&unresolved, 1);
    assert!(String::from_utf8_lossy(&unresolved.stderr).contains("ambiguous or unresolved"));
    let unsupported = format!(
        "trait Missing {{}} {}",
        source.replace(
            "requires R == bool",
            "requires (R == bool) and (Char :- Missing)"
        )
    );
    sandbox.write("input.fl", unsupported);
    let rejected = sandbox.run(&args);
    failure(&rejected, 1);
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("ambiguous or unresolved"),
        "{}",
        details(&rejected)
    );
}

#[test]
fn deferred_definition_attributes_keep_target_scope_and_application_order() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        ^attrs.note("outer")
        ^attrs.note("inner")
        fn selected() {}
        mod nested {
            ^attrs.note("quote\" and slash\\")
            fn member() {}
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(deferred-attribute ").count(),
        3,
        "{attributes}"
    );
    assert_eq!(
        attributes
            .matches("(target (owner-id 1)) (lexical-owner (owner-id 0))")
            .count(),
        2,
        "{attributes}"
    );
    assert!(
        attributes.contains("(target (owner-id 3)) (lexical-owner (owner-id 2))"),
        "{attributes}"
    );
    assert!(
        attributes.find("inner").unwrap() < attributes.find("outer").unwrap(),
        "{attributes}"
    );
    for expression in [
        r#"attrs.note("outer")"#,
        r#"attrs.note("inner")"#,
        r#"attrs.note("quote\" and slash\\")"#,
    ] {
        assert!(
            attributes.contains(&format!("(expression {expression:?})")),
            "{attributes}"
        );
    }
    assert_eq!(
        attributes.matches("(hir-expression ").count(),
        3,
        "{attributes}"
    );
    assert!(attributes.contains("(unresolved-ident "), "{attributes}");
    let again = sandbox.run(&["input.fl", "--emit", "hir"]);
    assert_eq!(dump, hir_dump(&again));
    let rejected = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains(
            "expression elaboration requires a protocol or construct not yet implemented"
        ),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
}

#[test]
fn definition_attribute_expressions_resolve_lexical_names_and_prior_parameters() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        fn annotation(value: bool) -> bool { value }
        ^annotation(true)
        fn selected(first: bool, ^annotation(first) second: bool) {}
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(hir-expression ").count(),
        2,
        "{attributes}"
    );
    assert_eq!(
        attributes
            .matches("(resolved-def (name \"annotation\")")
            .count(),
        2,
        "{attributes}"
    );
    assert!(attributes.contains("(resolved-fn-param "), "{attributes}");
    assert!(!attributes.contains("(unresolved-ident "), "{attributes}");
    // HIR retention is not authorization to treat a bool as an attribute Object.
    let checked = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!checked.status.success(), "{}", details(&checked));
}

#[test]
fn block_function_modifiers_reach_checked_nir_without_pending_attributes() {
    for modifier in [
        "inline",
        "pure",
        "comptime",
        "pure comptime",
        "quote",
        "unsafe",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write(
            "input.fl",
            format!("fn enclosing() {{ {modifier} fn selected(value: bool) -> bool {{ value }} }}"),
        );
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(output.status.success(), "{modifier}: {}", details(&output));
        let dump = String::from_utf8_lossy(&output.stdout);
        for flag in modifier.split_whitespace() {
            let expected = match flag {
                "comptime" => "(phase comptime)".into(),
                "quote" => "(quote-template ".into(),
                _ => format!("({flag} true)"),
            };
            assert!(dump.contains(&expected), "{modifier}: {dump}");
        }
    }
}

#[test]
fn source_tests_can_call_block_local_functions() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "test local {inline fn answer(flag:bool)->bool {not flag} let answer=answer(false); assumes answer;}");
    let output = sandbox.run(&["input.fl", "--test"]);
    assert!(
        success(&output).contains("(count 1)"),
        "{}",
        details(&output)
    );
}

#[test]
fn block_declarations_are_checked_even_when_unused() {
    for body in [
        "fn invalid()->bool {false;}",
        "inline fn invalid()->bool {false;}",
        "typealias Invalid=missing;",
        "fn duplicate() {} fn duplicate() {}",
        "do {fn hidden()->bool {true}}; hidden();",
        "^unknown.attribute inline fn invalid()->bool {true}",
    ] {
        let sandbox = Sandbox::new();
        sandbox.write("input.fl", format!("fn enclosing() {{{body}}}"));
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(!output.status.success(), "{body}: {}", details(&output));
    }
}

#[test]
fn block_definition_attributes_keep_nested_prefixes_and_outer_bindings() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        fn annotation(value: bool) -> bool { value }
        fn enclosing(flag: bool) {
            ^annotation(flag)
            ^annotation(false)
            fn selected(annotation: bool) {}
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(deferred-attribute ").count(),
        2,
        "{attributes}"
    );
    assert_eq!(
        attributes
            .matches("(resolved-def (name \"annotation\")")
            .count(),
        2,
        "{attributes}"
    );
    assert!(attributes.contains("(resolved-fn-param "), "{attributes}");
    assert!(!attributes.contains("(unresolved-ident "), "{attributes}");
    assert!(
        attributes.find("annotation(false)").unwrap()
            < attributes.find("annotation(flag)").unwrap(),
        "{attributes}"
    );
    assert_eq!(
        attributes
            .matches("(target (owner-id 3)) (lexical-owner (owner-id 2))")
            .count(),
        2,
        "{attributes}"
    );
    let checked = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!checked.status.success(), "{}", details(&checked));
}

#[test]
fn configurations_belong_to_the_enclosing_definition_before_prefix_attributes() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        .mode options.make(true),
        ^attrs.note("outer")
        ^attrs.note("inner")
        struct Selected {
            .mode false,
            .label "quoted",
            value: bool,
        }
        mod nested {
            .mode true,
            fn member() {}
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(deferred-configuration ").count(),
        4,
        "{attributes}"
    );
    assert_eq!(
        attributes.matches("(deferred-attribute ").count(),
        2,
        "{attributes}"
    );
    assert!(attributes.contains("(target (owner-id 0)) (lexical-owner (owner-id 0)) (key \"mode\") (expression \"options.make(true)\")"),"{attributes}");
    assert!(
        attributes.contains("(target (owner-id 1)) (lexical-owner (owner-id 1)) (key \"mode\")"),
        "{attributes}"
    );
    assert!(
        attributes.contains("(target (owner-id 2)) (lexical-owner (owner-id 2)) (key \"mode\")"),
        "{attributes}"
    );
    assert!(
        attributes.find("quoted").unwrap() < attributes.find("inner").unwrap(),
        "{attributes}"
    );
    assert!(
        attributes.find("inner").unwrap() < attributes.find("outer").unwrap(),
        "{attributes}"
    );
    let result = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains(
            "expression elaboration requires a protocol or construct not yet implemented"
        ),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for source in [
        ".mode , fn next() {}",
        "struct Broken { .mode , value:bool, }",
        "mod broken { .mode , fn next() {} }",
    ] {
        sandbox.write("bad.fl", source);
        let result = sandbox.run(&["bad.fl", "--emit", "ast"]);
        assert!(!result.status.success(), "{source}");
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("Expected a value after configuration name")
        );
    }
}

#[test]
fn object_expressions_preserve_interleaved_source_entries() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        fn mark(value:bool)->bool {value}
        fn base()->bool {true}
        fn build() {
            let value = {first:mark(true),base(),second:{nested:"quote\""},first:false};
            let empty = {};
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let object = hir_form(dump, "object");
    let first = object.find("first").unwrap();
    let positional = object.find("(object-value ").unwrap();
    let second = object.find("second").unwrap();
    let last = object.rfind("first").unwrap();
    assert!(
        first < positional && positional < second && second < last,
        "{object}"
    );
    assert_eq!(object.matches("(object-property ").count(), 4, "{object}");
    assert!(dump.contains("(object (entries ()))"), "{dump}");
    assert!(object.contains("nested"));
    let result = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(
        !result.status.success(),
        "runtime Object construction must be rejected"
    );
}

#[test]
fn field_attribute_chains_keep_member_targets_and_application_order() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        struct Pair {
            ^attrs.note("outer")
            ^attrs.note("inner")
            left: bool,
            ^attrs.note("alias")
            typealias Flag = bool;
            ^attrs.note("right")
            right: bool,
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let fields = hir_form(dump, "fields");
    assert_eq!(fields.matches("(field (id ").count(), 2, "{fields}");
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(deferred-attribute ").count(),
        4,
        "{attributes}"
    );
    for (offset, expected_count) in [2, 1].into_iter().enumerate() {
        let start = fields.match_indices("(field (id ").nth(offset).unwrap().0;
        let member = hir_form(&fields[start..], "field");
        let id = hir_form(member, "hir-id");
        assert_eq!(
            attributes.matches(&format!("(target {id})")).count(),
            expected_count,
            "{attributes}"
        );
    }
    assert_eq!(
        attributes.matches("(lexical-owner (owner-id 1))").count(),
        4,
        "{attributes}"
    );
    assert!(
        attributes.find("inner").unwrap() < attributes.find("outer").unwrap(),
        "{attributes}"
    );
    assert!(
        attributes.find("outer").unwrap() < attributes.find("right").unwrap(),
        "{attributes}"
    );
    assert!(
        attributes.contains("(target (owner-id 2)) (lexical-owner (owner-id 1))"),
        "{attributes}"
    );
    let result = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("struct member attribute"));
    let prefixes = (0..140)
        .map(|i| format!("^attrs.note({i})\n"))
        .collect::<String>();
    sandbox.write(
        "deep.fl",
        &format!("struct Deep {{{prefixes} value:bool,}}"),
    );
    let output = sandbox.run(&["deep.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(attributes.matches("(deferred-attribute ").count(), 140);
    assert!(attributes.find("note(139)").unwrap() < attributes.find("note(0)").unwrap());
}

#[test]
fn interface_bodies_preserve_nested_definitions() {
    let sandbox = Sandbox::new();
    for (index, body) in [
        "trait Container {struct Nested {flag:bool,} enum Choice {First, Second} mod helpers {fn ready()->bool {true}}}",
        "struct Container {} impl Container {struct Nested {flag:bool,} enum Choice {First, Second} mod helpers {fn ready()->bool {true}}}",
        "trait Interface {} struct Container {} impl Interface for Container {struct Nested {flag:bool,} enum Choice {First, Second} mod helpers {fn ready()->bool {true}}}",
    ].into_iter().enumerate() {
        sandbox.write("input.fl", body);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        let dump = hir_dump(&output);
        for name in ["Nested", "flag", "Choice", "First", "Second", "helpers", "ready"] {
            assert!(dump.contains(name), "case {index} lost {name}: {dump}");
        }
        let checked = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(
            !checked.status.success()
                && String::from_utf8_lossy(&checked.stderr)
                    .contains("elaboration failed:")
                && !String::from_utf8_lossy(&checked.stderr).contains("panicked"),
            "{}",
            details(&checked)
        );
    }
}

#[test]
fn interface_body_statements_cannot_disappear_from_checked_programs() {
    let sandbox = Sandbox::new();
    for prefix in [
        "trait Interface",
        "struct Container {} impl Container",
        "trait Interface {} struct Container {} impl Interface for Container",
    ] {
        for statement in ["let ignored = true;", "ignored: true,", "while true {}"] {
            sandbox.write(
                "input.fl",
                format!("{prefix} {{{statement}}} fn run()->bool {{true}}"),
            );
            let output = sandbox.run(&["input.fl", "--emit", "nir"]);
            assert!(
                !output.status.success(),
                "silently discarded {statement}: {}",
                details(&output)
            );
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("unsupported trait/implementation body entry"),
                "{}",
                details(&output)
            );
        }
    }
}

#[test]
fn interface_imports_are_lexical_bindings_not_method_requirements() {
    let sandbox = Sandbox::new();
    let helpers = "mod helpers {pub fn yes()->bool {true} pub fn no()->bool {false} pub typealias Flag=bool;}";
    for imports in [
        "use @helpers.yes;use @helpers.no;use @helpers.Flag;",
        "use @helpers.*;",
        "private use @helpers.*;",
        "pub use @helpers.{yes,no,Flag};",
        "use @helpers.{yes,no,Flag};",
        "use @helpers.yes as yes;use @helpers.no as no;use @helpers.Flag as Flag;",
    ] {
        for body in [
            format!(
                "trait Ready {{{imports} fn ready(self)->Flag {{yes() and not no()}}}} struct Value {{}} impl Ready for Value {{}} fn run()->bool {{Value{{}}.as(Ready).ready()}}"
            ),
            format!(
                "trait Ready {{fn ready(self)->bool;}} struct Value {{}} impl Ready for Value {{{imports} fn ready(self)->Flag {{yes() and not no()}}}} fn run()->bool {{Value{{}}.as(Ready).ready()}}"
            ),
            format!(
                "struct Value {{}} impl Value {{{imports} fn ready(self)->Flag {{yes() and not no()}}}} fn run()->bool {{Value{{}}.ready()}}"
            ),
        ] {
            sandbox.write("input.fl", format!("{helpers}{body}"));
            let output = sandbox.run(&["input.fl", "--interpret", "run"]);
            assert_eq!(success(&output).trim(), "(bool true)");
        }
    }
    sandbox.write("input.fl", format!("{helpers} trait Ready {{fn ready(self)->bool;}} struct Value {{}} impl Ready for Value {{use @helpers.yes as ready;}}"));
    let missing = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("missing required trait method"),
        "{}",
        details(&missing)
    );
    sandbox.write("input.fl", format!("{helpers} trait Ready {{use @helpers.yes as local_only;fn ready(self)->bool {{local_only()}}}} fn outside()->bool {{local_only()}}"));
    let leaked = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!leaked.status.success(), "import escaped its lexical scope");
    assert!(!String::from_utf8_lossy(&leaked.stderr).contains("panicked"));
}

#[test]
fn nominal_body_imports_support_methods_fields_and_variants() {
    let sandbox = Sandbox::new();
    let helpers = "mod helpers {pub fn yes()->bool {true} pub typealias Flag=bool;}";
    for imports in [
        "use @helpers.{yes,Flag};",
        "private use @helpers.*;",
        "pub use @helpers.yes;pub use @helpers.Flag;",
        "use @helpers.yes as yes;use @helpers.Flag as Flag;",
    ] {
        for body in [
            format!(
                "struct Value {{{imports} flag:Flag=yes(),fn ready(self)->Flag {{self.flag and yes()}}}} fn run()->bool {{Value{{}}.ready()}}"
            ),
            format!(
                "enum Value {{{imports} First(Flag), Second, fn ready(self)->Flag {{yes()}}}} fn run()->bool {{Value.First(true).ready()}}"
            ),
        ] {
            sandbox.write("input.fl", format!("{helpers}{body}"));
            let output = sandbox.run(&["input.fl", "--interpret", "run"]);
            assert_eq!(success(&output).trim(), "(bool true)");
        }
    }
    // Visibility checking still belongs to name resolution; accepting the
    // wrapper during lowering must not make a package-only source public.
    for definition in ["struct Value", "enum Value", "trait Value"] {
        sandbox.write("input.fl", format!("mod helpers {{fn hidden()->bool {{true}}}} {definition} {{pub use @helpers.hidden;}}"));
        let rejected = sandbox.run(&["input.fl", "--emit", "hir"]);
        assert!(!rejected.status.success(), "{}", details(&rejected));
    }
}

#[test]
fn interface_nested_modules_are_checked_as_lexical_namespaces() {
    let sandbox = Sandbox::new();
    let module = "mod helpers {typealias Flag=bool;fn identity(value:T)->T where T {value} mod nested {fn ready()->bool {true}} fn ready()->Flag {identity(nested.ready())}}";
    for body in [
        format!(
            "trait Ready {{{module} fn ready(self)->bool {{helpers.ready()}}}} struct Value {{}} impl Ready for Value {{}} fn run()->bool {{Value{{}}.as(Ready).ready()}}"
        ),
        format!(
            "trait Ready {{fn ready(self)->bool;}} struct Value {{}} impl Ready for Value {{{module} fn ready(self)->bool {{helpers.ready()}}}} fn run()->bool {{Value{{}}.as(Ready).ready()}}"
        ),
        format!(
            "struct Value {{}} impl Value {{{module} fn ready(self)->bool {{helpers.ready()}}}} fn run()->bool {{Value{{}}.ready()}}"
        ),
        format!(
            "struct Value {{{module} flag:bool=helpers.ready(),fn ready(self)->bool {{self.flag and helpers.ready()}}}} fn run()->bool {{Value{{}}.ready()}}"
        ),
        format!(
            "enum Value {{{module} First,fn ready(self)->bool {{helpers.ready()}}}} fn run()->bool {{Value.First.ready()}}"
        ),
    ] {
        for visibility in ["", "pub ", "private "] {
            sandbox.write(
                "input.fl",
                body.replacen("mod helpers", &format!("{visibility}mod helpers"), 1),
            );
            let output = sandbox.run(&["input.fl", "--interpret", "run"]);
            assert_eq!(success(&output).trim(), "(bool true)");
        }
        sandbox.write("input.fl", body.replace("identity(nested.ready())", "'x'"));
        let invalid = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(
            !invalid.status.success(),
            "nested function body was not checked"
        );
        assert!(
            String::from_utf8_lossy(&invalid.stderr).contains("elaboration failed:"),
            "{}",
            details(&invalid)
        );
    }
}

#[test]
fn enum_variant_attributes_preserve_payloads_and_target_identity() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"enum Choice {
        ^attrs.note("outer") ^attrs.note("inner") Empty,
        ^attrs.note("tuple") Pair(bool, bool),
        ^attrs.note("record") Record {flag:bool,},
    }"#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let variants = hir_form(dump, "variants");
    assert_eq!(variants.matches("(variant (id ").count(), 3, "{variants}");
    for name in ["Empty", "Pair", "Record", "flag"] {
        assert!(variants.contains(name), "{variants}");
    }
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(deferred-attribute ").count(),
        4,
        "{attributes}"
    );
    assert_eq!(
        attributes
            .matches("(target (hir-id (owner 1) (local 0)))")
            .count(),
        2,
        "{attributes}"
    );
    assert_eq!(
        attributes.matches("(lexical-owner (owner-id 1))").count(),
        4,
        "{attributes}"
    );
    assert!(attributes.find("inner").unwrap() < attributes.find("outer").unwrap());
    let checked = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!checked.status.success());
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("enum variant attribute"),
        "{}",
        details(&checked)
    );
    sandbox.write(
        "nested.fl",
        r#"enum Choice {
        ^attrs.note("record") Record {^attrs.note("field") flag:bool,},
        ^attrs.note("group") Group.{^attrs.note("leaf") Leaf,},
    }"#,
    );
    let output = sandbox.run(&["nested.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    for name in ["Record", "flag", "Group", "Leaf"] {
        assert!(dump.contains(name), "{dump}");
    }
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(deferred-attribute ").count(),
        4,
        "{attributes}"
    );
    for tag in ["field", "variant"] {
        for (start, _) in dump.match_indices(&format!("({tag} (id ")) {
            let member = hir_form(&dump[start..], tag);
            let id = hir_form(member, "hir-id");
            assert_eq!(
                attributes.matches(&format!("(target {id})")).count(),
                1,
                "{attributes}"
            );
        }
    }
    let checked = sandbox.run(&["nested.fl", "--emit", "nir"]);
    assert!(!checked.status.success());
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("enum variant attribute"),
        "{}",
        details(&checked)
    );
    sandbox.write(
        "field.fl",
        "enum Choice {Record {^attrs.note(true) flag:bool,}} ",
    );
    let checked = sandbox.run(&["field.fl", "--emit", "nir"]);
    assert!(!checked.status.success());
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("struct field attribute"),
        "{}",
        details(&checked)
    );
}

#[test]
fn catch_parameters_do_not_degrade_to_ordinary_inputs() {
    let sandbox = Sandbox::new();
    for source in [
        "fn receive(catch continuation:bool)->bool {continuation} fn run()->bool {receive(true)}",
        "trait Handler {fn receive(catch continuation:bool)->bool;}",
        "effect receive(catch continuation:bool)->bool;",
        "comptime fn receive(catch continuation:bool)->bool {continuation}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        let dump = hir_dump(&output);
        assert!(dump.contains("(flags 64)"), "{dump}");
        let checked = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(!checked.status.success(), "catch role was erased");
        assert!(
            String::from_utf8_lossy(&checked.stderr)
                .contains("catch parameters require continuation binding"),
            "{}",
            details(&checked)
        );
    }
}

#[test]
fn parameter_attributes_preserve_bindings_defaults_and_roles() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"fn selected(
        ^attrs.note("outer") ^attrs.note("inner") value:bool,
        ^attrs.note("default").enabled .fallback:bool=value,
        ^attrs.note("implicit") implicit context:bool,
        ^attrs.note("stage") comptime stage:bool,
    )->bool {value and fallback and context and stage}"#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let parameters = hir_form(dump, "params");
    assert_eq!(parameters.matches("(fn-param ").count(), 4, "{parameters}");
    for index in 0..4 {
        assert!(
            parameters.contains(&format!("(index {index})")),
            "{parameters}"
        );
    }
    assert!(parameters.contains("(flags 1)"), "{parameters}");
    assert!(parameters.contains("(flags 2)"), "{parameters}");
    assert!(parameters.contains("optional"), "{parameters}");
    let attributes = hir_form(dump, "definition-attributes");
    assert_eq!(
        attributes.matches("(deferred-attribute ").count(),
        5,
        "{attributes}"
    );
    assert_eq!(
        attributes
            .matches("(target (hir-id (owner 1) (local 1)))")
            .count(),
        2,
        "{attributes}"
    );
    assert_eq!(
        attributes.matches("(lexical-owner (owner-id 1))").count(),
        5,
        "{attributes}"
    );
    assert!(attributes.find("inner").unwrap() < attributes.find("outer").unwrap());
    assert!(attributes.contains(".enabled"), "{attributes}");
    let checked = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(!checked.status.success());
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("function parameter attribute"),
        "{}",
        details(&checked)
    );
}

#[test]
fn interface_typealiases_are_definitions_not_abstract_methods() {
    let sandbox = Sandbox::new();
    for source in [
        "trait Ready {typealias Flag=bool;fn ready(self)->Flag {true}} struct Value {} impl Ready for Value {} fn run()->bool {Value{}.as(Ready).ready()}",
        "trait Ready {fn ready(self)->bool;} struct Value {} impl Ready for Value {typealias Flag=bool;fn ready(self)->Flag {true}} fn run()->bool {Value{}.as(Ready).ready()}",
        "struct Value {} impl Value {typealias Subject=Self;fn identity(self)->Subject {self} fn ready(self)->bool {true}} fn run()->bool {Value{}.identity().ready()}",
        "trait Identity {typealias Subject=Self;fn identity(self)->Subject {self}} struct Value {} impl Identity for Value {} fn run()->bool {let value=Value{}.as(Identity).identity();true}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert_eq!(success(&output).trim(), "(bool true)");
    }
    for ordinary in ["typealias Element=bool;", "const Element=bool;"] {
        sandbox.write("input.fl", format!("typealias Type=.Type'builtin;trait Collection {{assoc Element:Type;}} struct Value {{}} impl Collection for Value {{{ordinary}}}"));
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("missing required associated value"),
            "{}",
            details(&output)
        );
    }
}

#[test]
fn interface_constants_remain_checked_lexical_definitions() {
    let sandbox = Sandbox::new();
    let protocol = "typealias Type=.Type'builtin;trait Splice {assoc RuntimeTarget:Type;fn splice(comptime data:Self)->RuntimeTarget;} impl Splice for bool {assoc RuntimeTarget:Type=bool;fn splice(comptime data:bool)->bool {data}}";
    for source in [
        "trait Ready {const FLAG:bool=true;fn ready(self)->bool {FLAG}} struct Value {} impl Ready for Value {} fn run()->bool {Value{}.as(Ready).ready()}",
        "trait Ready {fn ready(self)->bool;} struct Value {} impl Ready for Value {const FLAG:bool=true;fn ready(self)->bool {FLAG}} fn run()->bool {Value{}.as(Ready).ready()}",
        "struct Value {} impl Value {const FLAG:bool=true;fn ready(self)->bool {FLAG}} fn run()->bool {Value{}.ready()}",
    ] {
        sandbox.write("input.fl", format!("{protocol}{source}"));
        let output = sandbox.run(&[
            "input.fl",
            "--interpret",
            "run",
            "--splice-protocol",
            "input.Splice",
        ]);
        assert_eq!(success(&output).trim(), "(bool true)");
        sandbox.write(
            "input.fl",
            format!(
                "{protocol}{}",
                source.replace("FLAG:bool=true", "FLAG:bool='x'")
            ),
        );
        let invalid = sandbox.run(&[
            "input.fl",
            "--emit",
            "nir",
            "--splice-protocol",
            "input.Splice",
        ]);
        assert!(!invalid.status.success());
        assert!(
            String::from_utf8_lossy(&invalid.stderr).contains("elaboration failed:"),
            "{}",
            details(&invalid)
        );
    }
}

#[test]
fn interface_aliases_preserve_generic_formation_requirements() {
    let sandbox = Sandbox::new();
    let checked = "struct Checked where T, requires T == bool {value:T,}";
    for (body, required, unconstrained) in [
        (
            "trait Ready where T, requires T == bool {typealias Item=Checked<T>;fn ready(self,value:Item)->bool {value.value}} struct Value {} impl Ready<bool> for Value {} fn run()->bool {Value{}.as(Ready<bool>).ready(Checked<bool>{value:true})}",
            "trait Ready where T, requires T == bool",
            "trait Ready where T",
        ),
        (
            "struct Value where T {} impl Value<T> where T, requires T == bool {typealias Item=Checked<T>;fn ready(self,value:Item)->bool {value.value}} fn run()->bool {Value<bool>{}.ready(Checked<bool>{value:true})}",
            "impl Value<T> where T, requires T == bool",
            "impl Value<T> where T",
        ),
        (
            "trait Ready where T {fn ready(self,value:Checked<bool>)->bool;} struct Value where T {} impl Ready<T> for Value<T> where T, requires T == bool {typealias Item=Checked<T>;fn ready(self,value:Item)->bool {value.value}} fn run()->bool {Value<bool>{}.as(Ready<bool>).ready(Checked<bool>{value:true})}",
            "impl Ready<T> for Value<T> where T, requires T == bool",
            "impl Ready<T> for Value<T> where T",
        ),
    ] {
        sandbox.write("input.fl", format!("{checked}{body}"));
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert_eq!(success(&output).trim(), "(bool true)");
        sandbox.write(
            "input.fl",
            format!("{checked}{}", body.replace(required, unconstrained)),
        );
        let invalid = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(
            !invalid.status.success(),
            "alias lost its formation requirement"
        );
        assert!(
            String::from_utf8_lossy(&invalid.stderr).contains("elaboration failed:"),
            "{}",
            details(&invalid)
        );
    }
}

#[test]
fn interface_aliases_inherit_trait_bounds_without_inventing_evidence() {
    let sandbox = Sandbox::new();
    let prefix =
        "trait Allowed {} impl Allowed for bool {} struct Checked where T :- Allowed {value:T,}";
    for (body, bounded, unbounded) in [
        (
            "trait Ready where T :- Allowed {typealias Item=Checked<T>;fn ready(self,value:Item)->bool {true}} struct Value {} impl Ready<bool> for Value {} fn run()->bool {Value{}.as(Ready<bool>).ready(Checked<bool>{value:true})}",
            "trait Ready where T :- Allowed",
            "trait Ready where T",
        ),
        (
            "struct Value where T {} impl Value<T> where T :- Allowed {typealias Item=Checked<T>;fn ready(self,value:Item)->bool {true}} fn run()->bool {Value<bool>{}.ready(Checked<bool>{value:true})}",
            "impl Value<T> where T :- Allowed",
            "impl Value<T> where T",
        ),
        (
            "trait Ready :- Allowed {typealias Item=Checked<Self>;fn ready(self,value:Item)->bool {true}} struct Value {} impl Allowed for Value {} impl Ready for Value {} fn run()->bool {Value{}.as(Ready).ready(Checked<Value>{value:Value{}})}",
            "trait Ready :- Allowed",
            "trait Ready",
        ),
    ] {
        sandbox.write("input.fl", format!("{prefix}{body}"));
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert_eq!(success(&output).trim(), "(bool true)");
        sandbox.write(
            "input.fl",
            format!("{prefix}{}", body.replace(bounded, unbounded)),
        );
        let invalid = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(!invalid.status.success(), "alias fabricated trait evidence");
        assert!(
            String::from_utf8_lossy(&invalid.stderr).contains("elaboration failed:"),
            "{}",
            details(&invalid)
        );
    }
}

#[test]
fn interface_newtypes_keep_nominal_identity_and_explicit_views() {
    let sandbox = Sandbox::new();
    for body in [
        "trait Ready {newtype Flag=bool;newtype Other=bool;newtype Wrapped<T> = T;fn ready(self)->bool {true.as(Flag).as(bool).as(Wrapped<bool>).as(bool)}} struct Value {} impl Ready for Value {} fn run()->bool {Value{}.as(Ready).ready()}",
        "trait Ready {fn ready(self)->bool;} struct Value {} impl Ready for Value {newtype Flag=bool;newtype Other=bool;newtype Wrapped<T> = T;fn ready(self)->bool {true.as(Flag).as(bool).as(Wrapped<bool>).as(bool)}} fn run()->bool {Value{}.as(Ready).ready()}",
        "struct Value {} impl Value {newtype Flag=bool;newtype Other=bool;newtype Wrapped<T> = T;fn ready(self)->bool {true.as(Flag).as(bool).as(Wrapped<bool>).as(bool)}} fn run()->bool {Value{}.ready()}",
    ] {
        sandbox.write("input.fl", body);
        let output = sandbox.run(&["input.fl", "--interpret", "run"]);
        assert_eq!(success(&output).trim(), "(bool true)");
        sandbox.write(
            "input.fl",
            body.replace(
                "true.as(Flag).as(bool).as(Wrapped<bool>).as(bool)",
                "let flag:Other=true.as(Flag);true",
            ),
        );
        let invalid = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(!invalid.status.success(), "distinct newtypes became equal");
        assert!(
            String::from_utf8_lossy(&invalid.stderr).contains("elaboration failed:"),
            "{}",
            details(&invalid)
        );
    }
}

#[test]
fn tuple_enumeration_view_retains_operand_and_loop_structure_in_hir() {
    let sandbox = Sandbox::new();
    for (body, runtime_view) in [
        ("let view=values'tuple_enumerate;", true),
        (
            "inline for (index,element) in values'tuple_enumerate {let used=element;}",
            false,
        ),
    ] {
        sandbox.write(
            "input.fl",
            &format!("pure comptime fn List<T>->.Type'builtin {{T}} pure comptime fn Tuple<...Es:List<.Type'builtin>>->.Type'builtin {{bool}} fn run(values:Tuple<bool,bool>) {{{body}}}"),
        );
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        let dump = success(&output);
        assert!(dump.contains("(take-tuple-enumerate (value"), "{dump}");
        assert!(dump.contains("values"), "{dump}");
        assert!(!dump.contains("(invalid)"), "{dump}");
        let output = sandbox.run(&[
            "input.fl",
            "--emit",
            "nir",
            "--meta-list-type",
            "input.List",
            "--tuple-type",
            "input.Tuple",
        ]);
        if runtime_view {
            failure(&output, 1);
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                diagnostic.contains("comptime-only value cannot inhabit runtime storage")
                    || diagnostic.contains("runtime elements cannot be evaluated at comptime"),
                "{diagnostic}"
            );
        } else {
            assert!(success(&output).starts_with("(nir-package"));
        }
    }
    sandbox.write(
        "input.fl",
        "fn run() {inline for (index,element) in (true,false)'tuple_enumerate {let used=element;}}",
    );
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(success(&output).starts_with("(nir-package"));
    sandbox.write("input.fl", "fn run() {let view=missing'tuple_enumerate;}");
    failure(&sandbox.run(&["input.fl", "--emit", "hir"]), 1);
}

#[test]
fn explicit_package_cli_compiles_cross_package_calls_and_preserves_outputs() {
    let sandbox = Sandbox::new();
    sandbox.write("client.fl", "use dep.answer;fn run()->bool {answer()}");
    sandbox.write("provider.fl", "pub fn answer()->bool {true}");
    let args = [
        "--packages-v1",
        "--package",
        "7",
        "701",
        "client",
        "client.fl",
        "--package",
        "9",
        "901",
        "provider",
        "provider.fl",
        "--primary",
        "7",
        "--dependency",
        "7",
        "dep",
        "9",
        "--emit",
        "nir",
    ];
    let dump = success(&sandbox.run(&args)).to_owned();
    assert!(dump.contains("answer") && dump.contains("run"), "{dump}");
    let mut output_args = args.to_vec();
    output_args.extend(["--output", "result.lisp"]);
    success(&sandbox.run(&output_args));
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp"))
            .unwrap()
            .trim(),
        dump.trim()
    );
    // Source failures leave previous artifacts intact, including dependencies.
    sandbox.write("provider.fl", "pub fn answer()->bool {missing}");
    failure(&sandbox.run(&output_args), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp"))
            .unwrap()
            .trim(),
        dump.trim()
    );
    sandbox.write("provider.fl", "pub fn answer()->bool {true}");
    *output_args.last_mut().unwrap() = "provider.fl";
    failure(&sandbox.run(&output_args), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("provider.fl")).unwrap(),
        "pub fn answer()->bool {true}"
    );
}

#[test]
fn explicit_package_cli_rejects_incomplete_or_unsupported_requests() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn run() {}");
    for args in [
        vec!["--packages-v1"],
        vec!["--packages-v1", "--primary", "1"],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "18446744073709551616",
            "app",
            "input.fl",
            "--primary",
            "1",
        ],
        vec![
            "--packages-v1",
            "--package",
            "-1",
            "1",
            "app",
            "input.fl",
            "--primary",
            "1",
        ],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "1",
            "app",
            "input.fl",
            "--primary",
            "1",
            "--emit",
            "ast",
        ],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "1",
            "app",
            "input.fl",
            "--primary",
            "1",
            "--primary",
            "1",
        ],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "1",
            "app",
            "input.fl",
            "--primary",
            "1",
            "--sysroot",
            "library",
        ],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    assert!(success(&sandbox.run(&["--packages-v1", "--help"])).contains("--packages-v1"));
}

#[test]
fn explicit_package_cli_resolves_source_protocols_by_provider_name() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "provider space.fl",
        "pub trait Merge where Rhs {assoc Output:.Type'builtin;fn add(self,rhs:Rhs)->Output;}",
    );
    sandbox.write("client space.fl", "use dependency.Merge;struct Flag {value:bool}
        impl Merge<bool> for Flag {assoc Output:.Type'builtin=bool;fn add(self,rhs:bool)->bool {self.value and rhs}}
        fn run()->bool {Flag {value:true} + true}");
    let args = [
        "--packages-v1",
        "--package",
        "3",
        "300",
        "client",
        "client space.fl",
        "--package",
        "5",
        "500",
        "provider",
        "provider space.fl",
        "--primary",
        "3",
        "--dependency",
        "3",
        "dependency",
        "5",
        "--emit",
        "nir",
    ];
    failure(&sandbox.run(&args), 1);
    let mut selected = args.to_vec();
    selected.extend(["--add-protocol", "provider.Merge"]);
    success(&sandbox.run(&selected));
    *selected.last_mut().unwrap() = "dependency.Merge";
    failure(&sandbox.run(&selected), 1);
}

#[test]
fn explicit_package_cli_checks_identities_edges_and_flag_named_inputs() {
    let sandbox = Sandbox::new();
    sandbox.write("--source.fl", "fn run()->bool {true}");
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "100",
        "app",
        "--source.fl",
        "--primary",
        "1",
        "--emit",
        "nir",
    ];
    success(&sandbox.run(&base));
    for extra in [
        vec!["--package", "1", "101", "other", "--source.fl"],
        vec!["--package", "2", "100", "other", "--source.fl"],
        vec!["--dependency", "1", "missing", "2"],
        vec!["--dependency", "1", "self", "1"],
        vec!["--builtin", "2"],
    ] {
        let mut args = base.to_vec();
        args.extend(extra);
        failure(&sandbox.run(&args), 1);
    }
}

#[test]
fn explicit_package_cli_designates_the_supplied_builtin_package() {
    let sandbox = Sandbox::new();
    sandbox.write("app.fl", "fn run()->Byte {7}");
    sandbox.write(
        "core.fl",
        "pub mod prelude {pub typealias Byte=.u8'builtin;}",
    );
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "100",
        "app",
        "app.fl",
        "--package",
        "2",
        "200",
        "core",
        "core.fl",
        "--primary",
        "1",
        "--dependency",
        "1",
        "builtin",
        "2",
        "--emit",
        "nir",
    ];
    failure(&sandbox.run(&base), 1);
    let mut args = base.to_vec();
    args.extend(["--builtin", "2"]);
    success(&sandbox.run(&args));
}

#[test]
fn explicit_package_cli_runs_actions_in_primary_semantic_identity() {
    let sandbox = Sandbox::new();
    sandbox.write("client.fl", "use dep.answer; trait Duplicate {} trait Retire {fn drop(*self);} fn run()->bool {answer()} comptime fn compute()->bool {true}");
    sandbox.write("provider.fl", "pub trait Duplicate {} pub trait Retire {fn drop(*self);} pub fn answer()->bool {true} pub fn run()->bool {false} pub comptime fn compute()->bool {false} pub fn dependency_only()->bool {false}");
    let base = [
        "--packages-v1",
        "--package",
        "7",
        "701",
        "client",
        "client.fl",
        "--package",
        "9",
        "901",
        "provider",
        "provider.fl",
        "--primary",
        "7",
        "--dependency",
        "7",
        "dep",
        "9",
    ];
    for action in [
        vec!["--interpret", "run"],
        vec!["--invoke-comptime", "compute"],
        vec![
            "--interpret",
            "run",
            "--copy-protocol",
            "provider.Duplicate",
            "--drop-protocol",
            "provider.Retire",
        ],
        vec![
            "--interpret",
            "run",
            "--inline-functions",
            "--copy-protocol",
            "provider.Duplicate",
            "--drop-protocol",
            "provider.Retire",
        ],
        vec!["--instantiate", "run"],
        vec![
            "--analyze-resources",
            "run",
            "--copy-protocol",
            "provider.Duplicate",
            "--drop-protocol",
            "provider.Retire",
        ],
    ] {
        let mut args = base.to_vec();
        args.extend(&action);
        let output = sandbox.run(&args);
        let dump = success(&output);
        if action[0] == "--interpret" || action[0] == "--invoke-comptime" {
            assert_eq!(dump.trim(), "(bool true)");
        } else {
            assert!(
                dump.starts_with(if action[0] == "--instantiate" {
                    "(iir-package"
                } else {
                    "(resource-analysis"
                }),
                "{dump}"
            );
        }
        args.extend(["--output", "result.lisp"]);
        success(&sandbox.run(&args));
        assert_eq!(
            fs::read_to_string(sandbox.path("result.lisp"))
                .unwrap()
                .trim(),
            dump.trim()
        );
    }
    for action in ["--interpret", "--instantiate", "--invoke-comptime"] {
        let mut args = base.to_vec();
        args.extend([action, "dependency_only", "--output", "result.lisp"]);
        sandbox.write("result.lisp", "previous");
        failure(&sandbox.run(&args), 1);
        assert_eq!(
            fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
            "previous"
        );
    }
    // Duplicate leaf names are legal: the selected source identity survives
    // elaboration, including nested-module qualification.
    sandbox.write("provider.fl", "pub mod memory {pub trait Duplicate {} pub trait Retire {fn drop(*self);}} pub fn answer()->bool {true}");
    for selector in ["provider.memory", "dep.memory"] {
        let copy = format!("{selector}.Duplicate");
        let drop = format!("{selector}.Retire");
        let mut args = base.to_vec();
        args.extend([
            "--interpret",
            "run",
            "--copy-protocol",
            &copy,
            "--drop-protocol",
            &drop,
        ]);
        let output = sandbox.run(&args);
        if selector == "provider.memory" {
            assert_eq!(success(&output).trim(), "(bool true)");
        } else {
            failure(&output, 1);
        }
    }
    let provider = fs::read(sandbox.path("provider.fl")).unwrap();
    let mut overwrite = base.to_vec();
    overwrite.extend(["--interpret", "run", "--output", "provider.fl"]);
    failure(&sandbox.run(&overwrite), 1);
    assert_eq!(fs::read(sandbox.path("provider.fl")).unwrap(), provider);
    for extra in [
        vec!["--interpret", "run", "--emit", "nir"],
        vec!["--interpret", "run", "--invoke-comptime", "compute"],
        vec!["--interpret", "run", "--inline-functions"],
        vec!["--interpret", "run", "--fuel", "0"],
        vec!["--fuel", "1"],
    ] {
        let mut args = base.to_vec();
        args.extend(extra);
        failure(&sandbox.run(&args), 2);
    }
    let mut args = base.to_vec();
    args.extend([
        "--interpret",
        "run",
        "--fuel",
        "1",
        "--output",
        "result.lisp",
    ]);
    failure(&sandbox.run(&args), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
        "previous"
    );
}

#[test]
fn single_input_resource_actions_keep_qualified_source_identities() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "trait Duplicate {} trait Retire {fn drop(*self);} mod selected {pub trait Duplicate {} pub trait Retire {fn drop(*self);}} struct Ticket {} impl selected.Retire for Ticket {fn drop(*self) {}} fn run()->bool {let ticket=Ticket {}; true}");
    // Select a nested declaration despite duplicate leaf names in the root.
    for action in ["--interpret", "--analyze-resources"] {
        let args = [
            "input.fl",
            action,
            "run",
            "--copy-protocol",
            "input.selected.Duplicate",
            "--drop-protocol",
            "input.selected.Retire",
        ];
        let output = sandbox.run(&args);
        let dump = success(&output);
        if action == "--interpret" {
            assert_eq!(dump.trim(), "(bool true)");
        } else {
            assert!(dump.contains("(invoke-drop "), "{dump}");
        }
    }
}

#[test]
fn sysroot_resource_actions_preserve_explicit_selection_and_action_scope() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {} pub trait Duplicate {} pub trait Retire {fn drop(*self);}",
    );
    sandbox.write(
        "library/std/lib.fl",
        "pub trait Duplicate {} pub trait Retire {fn drop(*self);}",
    );
    sandbox.write(
        "library/language-items.conf",
        "version = 1\ncopy-protocol = builtin.Duplicate\ndrop-protocol = builtin.Retire\n",
    );
    sandbox.write("input.fl", "fn run()->bool {true}");
    for provider in ["builtin", "std"] {
        let copy = format!("{provider}.Duplicate");
        let drop = format!("{provider}.Retire");
        let mut args = vec![
            "input.fl",
            "--sysroot",
            "library",
            "--interpret",
            "run",
            "--copy-protocol",
            &copy,
            "--drop-protocol",
            &drop,
        ];
        assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
        args.push("--inline-functions");
        assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    }
    let output = sandbox.run(&[
        "input.fl",
        "--sysroot",
        "library",
        "--analyze-resources",
        "run",
        "--copy-protocol",
        "std.Duplicate",
    ]);
    let dump = success(&output);
    assert!(dump.contains("(stage ownership-obligations)"), "{dump}");
    assert!(!dump.contains("(drop-protocol "), "{dump}");
}

#[test]
fn all_source_bindings_require_elaboration_in_both_cli_modes() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn run()->bool {true}");
    for flag in [
        "--list-pattern-protocol",
        "--add-protocol",
        "--sub-protocol",
        "--mul-protocol",
        "--div-protocol",
        "--rem-protocol",
        "--neg-protocol",
        "--not-protocol",
        "--index-protocol",
        "--ord-protocol",
        "--eq-protocol",
        "--literal-into-protocol",
        "--splice-protocol",
        "--meta-list-type",
        "--tuple-type",
    ] {
        for base in [
            vec!["input.fl"],
            vec![
                "--packages-v1",
                "--package",
                "1",
                "701",
                "input",
                "input.fl",
                "--primary",
                "1",
            ],
        ] {
            for stage in [None, Some("hir"), Some("ast")] {
                let mut args = base.clone();
                if let Some(stage) = stage {
                    args.extend(["--emit", stage]);
                }
                args.extend([flag, "input.Missing", "--output", "result.lisp"]);
                sandbox.write("result.lisp", "previous");
                failure(&sandbox.run(&args), 2);
                assert_eq!(
                    fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
                    "previous"
                );
            }
            let mut args = base;
            args.extend(["--emit", "nir", flag, "input.Missing"]);
            // It must now attempt semantic binding, rather than reject the action.
            failure(&sandbox.run(&args), 1);
        }
    }
}

#[test]
fn list_pattern_cli_binds_single_package_and_dependency_protocols() {
    let sandbox = Sandbox::new();
    let source = include_str!("../../../development/examples/list-patterns.fl");
    sandbox.write("input.fl", source);
    for mode in ["--emit", "--interpret", "--instantiate"] {
        let entry = if mode == "--emit" { "nir" } else { "evaluate" };
        let output = sandbox.run(&[
            "input.fl",
            mode,
            entry,
            "--list-pattern-protocol",
            "input.ListView",
        ]);
        let dump = success(&output);
        if mode == "--interpret" {
            assert_eq!(dump.trim(), "(bool true)");
        }
    }
    sandbox.write("client.fl", "use dep.evaluate; fn run()->bool {evaluate()}");
    let base = [
        "--packages-v1",
        "--package",
        "7",
        "701",
        "client",
        "client.fl",
        "--package",
        "9",
        "901",
        "provider",
        "input.fl",
        "--primary",
        "7",
        "--dependency",
        "7",
        "dep",
        "9",
        "--interpret",
        "run",
    ];
    let mut args = base.to_vec();
    args.extend(["--list-pattern-protocol", "provider.ListView"]);
    assert_eq!(success(&sandbox.run(&args)).trim(), "(bool true)");
    for selector in ["dep.ListView", "provider.Missing", "provider.Repeated"] {
        let mut args = base.to_vec();
        args.extend(["--list-pattern-protocol", selector]);
        failure(&sandbox.run(&args), 1);
    }
    failure(&sandbox.run(&base), 1);
    for extra in [
        vec!["--list-pattern-protocol"],
        vec![
            "--list-pattern-protocol",
            "input.ListView",
            "--list-pattern-protocol",
            "input.ListView",
        ],
    ] {
        let mut args = vec!["input.fl", "--emit", "nir"];
        args.extend(extra);
        failure(&sandbox.run(&args), 2);
    }
}

#[test]
fn list_pattern_cli_overrides_sysroot_manifest_default() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write("library/builtin/lib.fl", "pub mod prelude {}");
    sandbox.write("library/std/lib.fl", "");
    sandbox.write(
        "library/language-items.conf",
        "version = 1\nlist-pattern-protocol = builtin.Missing\n",
    );
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/list-patterns.fl"),
    );
    let args = [
        "input.fl",
        "--sysroot",
        "library",
        "--interpret",
        "evaluate",
    ];
    failure(&sandbox.run(&args), 1);
    let mut selected = args.to_vec();
    selected.extend(["--list-pattern-protocol", "input.ListView"]);
    assert_eq!(success(&sandbox.run(&selected)).trim(), "(bool true)");
}

#[test]
fn list_pattern_cli_checks_extractor_observation_with_cleanup_and_inlining() {
    let sandbox = Sandbox::new();
    let ordinary = format!(
        "{}\ntrait Duplicate {{}} trait Retire {{fn drop(*self);}}",
        include_str!("../../../development/examples/list-patterns.fl")
    );
    let quoted = ordinary
        .replace(
            "fn list_pattern_view_get_len(*self) -> Size {",
            "quote fn list_pattern_view_get_len(*self) -> Size {",
        )
        .replace(
            "fn list_pattern_view_get_element(*self, comptime index: Size) -> bool {",
            "quote fn list_pattern_view_get_element(*self, comptime index: Size) -> bool {",
        );
    for source in [&ordinary, &quoted] {
        for inline in [false, true] {
            let mut args = vec![
                "input.fl",
                "--interpret",
                "evaluate",
                "--list-pattern-protocol",
                "input.ListView",
                "--copy-protocol",
                "input.Duplicate",
                "--drop-protocol",
                "input.Retire",
                "--output",
                "result.lisp",
            ];
            if inline {
                args.push("--inline-functions");
            }
            sandbox.write("input.fl", source);
            success(&sandbox.run(&args));
            assert_eq!(
                fs::read_to_string(sandbox.path("result.lisp"))
                    .unwrap()
                    .trim(),
                "(bool true)"
            );
            // A list extractor observes the scrutinee. Compiling its body is
            // insufficient: checked execution must reject writes through it.
            sandbox.write(
                "input.fl",
                &source.replace("self.value.*", "self.value.* = false; false"),
            );
            let output = sandbox.run(&args);
            failure(&output, 1);
            assert!(
                details(&output).contains("observation"),
                "{}",
                details(&output)
            );
            assert_eq!(
                fs::read_to_string(sandbox.path("result.lisp"))
                    .unwrap()
                    .trim(),
                "(bool true)"
            );
        }
    }
}

#[test]
fn source_tests_run_named_anonymous_and_module_bodies() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "fn check()->bool {true} fn named()->bool {false} test named {let result=check();assumes result;} test {assumes true;} mod nested {test named {assumes true;}}");
    let hir = sandbox.run(&["input.fl", "--emit", "hir"]);
    assert!(success(&hir).contains("(test true)"));
    let nir = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert_eq!(success(&nir).matches("(test-entry ").count(), 3);
    let output = sandbox.run(&["input.fl", "--test"]);
    let dump = success(&output);
    assert!(dump.contains("(count 3)"), "{dump}");
    assert_eq!(dump.matches("(passed ").count(), 3);
    assert!(dump.contains("anonymous"));
    sandbox.write(
        "input.fl",
        "test broken {assumes false;} test later {assumes true;}",
    );
    sandbox.write("result.lisp", "previous");
    let output = sandbox.run(&["input.fl", "--test", "--output", "result.lisp"]);
    failure(&output, 1);
    assert!(details(&output).contains("broken"));
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
        "previous"
    );
}

#[test]
fn source_tests_select_primary_package_and_validate_actions() {
    let sandbox = Sandbox::new();
    sandbox.write("client.fl", "test {assumes true;}");
    sandbox.write("provider.fl", "test dependency {assumes false;}");
    let base = [
        "--packages-v1",
        "--package",
        "7",
        "701",
        "client",
        "client.fl",
        "--package",
        "9",
        "901",
        "provider",
        "provider.fl",
        "--primary",
        "7",
    ];
    let mut args = base.to_vec();
    args.push("--test");
    assert!(success(&sandbox.run(&args)).contains("(count 1)"));
    for extra in [
        vec!["--test", "--test"],
        vec!["--test", "--interpret", "run"],
        vec!["--interpret", "run", "--test"],
        vec!["--test", "--emit", "nir"],
        vec!["--test", "--fuel", "0"],
    ] {
        let mut args = vec!["client.fl"];
        args.extend(extra);
        failure(&sandbox.run(&args), 2);
    }
    sandbox.write("client.fl", "test {assumes missing;}");
    failure(&sandbox.run(&["client.fl", "--test"]), 1);
}

#[test]
fn source_tests_use_checked_cleanup_and_bounded_execution() {
    let sandbox = Sandbox::new();
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} struct Ticket {} impl Retire for Ticket {fn drop(*self) {assumes true;}} test cleanup {let ticket=Ticket {}; }";
    sandbox.write("input.fl", source);
    let base = [
        "input.fl",
        "--test",
        "--copy-protocol",
        "input.Duplicate",
        "--drop-protocol",
        "input.Retire",
    ];
    assert!(success(&sandbox.run(&base)).contains("(count 1)"));
    let mut inline = base.to_vec();
    inline.push("--inline-functions");
    assert!(success(&sandbox.run(&inline)).contains("(count 1)"));
    sandbox.write("input.fl", &source.replace("assumes true", "assumes false"));
    let output = sandbox.run(&base);
    failure(&output, 1);
    assert!(details(&output).contains("cleanup"));
    sandbox.write("input.fl", "test spin {while true {}}");
    let output = sandbox.run(&["input.fl", "--test", "--fuel", "1000"]);
    failure(&output, 1);
    assert!(details(&output).contains("spin"));
    sandbox.write("input.fl", "fn ordinary() {}");
    assert!(success(&sandbox.run(&["input.fl", "--test"])).contains("(count 0)"));
}

#[test]
fn source_test_labels_do_not_become_callable_values() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "test same {} test same {} test {same();}");
    failure(&sandbox.run(&["input.fl", "--test"]), 1);
    sandbox.write("input.fl", "test same {} test same {}");
    assert!(success(&sandbox.run(&["input.fl", "--test"])).contains("(count 2)"));
    sandbox.write("input.fl", "fn outer() {test nested {}} ");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    failure(&output, 1);
    assert!(!details(&output).contains("panicked"));
}

#[test]
fn source_tests_collect_independent_failures_and_continue_after_budget_exhaustion() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl","test rejected {assumes false;} test spinning {while true {}} test succeeds {assumes true;}");
    sandbox.write("result.lisp", "previous");
    let output = sandbox.run(&[
        "input.fl",
        "--test",
        "--fuel",
        "1000",
        "--output",
        "result.lisp",
    ]);
    failure(&output, 1);
    let report = String::from_utf8_lossy(&output.stderr);
    assert!(report.contains("(count 3) (failures 2)"), "{report}");
    assert_eq!(report.matches("(failed ").count(), 2, "{report}");
    assert_eq!(report.matches("(passed ").count(), 1, "{report}");
    for name in ["rejected", "spinning", "succeeds"] {
        assert!(report.contains(name), "{report}");
    }
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
        "previous"
    );
}

#[test]
fn repository_language_suite_runs_as_checked_source_tests() {
    let sandbox = Sandbox::new();
    sandbox.write("main.fl", include_str!("../../../tests/language/main.fl"));
    sandbox.write(
        "fixtures.fl",
        include_str!("../../../tests/language/fixtures.fl"),
    );
    let args = [
        "main.fl",
        "--test",
        "--copy-protocol",
        "input.Duplicate",
        "--drop-protocol",
        "input.Retire",
    ];
    let output = sandbox.run(&args);
    let report = success(&output);
    assert!(report.contains("(count 28) (failures 0)"), "{report}");
    assert_eq!(report.matches("(passed ").count(), 28);
    let mut inline = args.to_vec();
    inline.push("--inline-functions");
    let output = sandbox.run(&inline);
    assert!(success(&output).contains("(count 28) (failures 0)"));
    let mut filtered = args.to_vec();
    filtered.extend(["--test-filter", "template_"]);
    let output = sandbox.run(&filtered);
    assert!(success(&output).contains("(count 6) (failures 0) (filtered-out 22)"));
    let output = sandbox.run(&["main.fl", "--list-tests", "--test-filter", "template_"]);
    let inventory = success(&output);
    assert!(
        inventory.contains("(count 6) (filtered-out 22)"),
        "{inventory}"
    );
    assert!(
        !inventory.contains("flag_case"),
        "template definitions must not become runnable tests"
    );
    sandbox.write(
        "main.fl",
        &include_str!("../../../tests/language/main.fl").replace(
            "assumes first == 7 and second == 9;",
            "assumes first == 8 and second == 9;",
        ),
    );
    let output = sandbox.run(&args);
    failure(&output, 1);
    let report = String::from_utf8_lossy(&output.stderr);
    assert!(report.contains("(count 28) (failures 1)"), "{report}");
    assert_eq!(report.matches("(passed ").count(), 27);
    assert!(report.contains("default_argument"));
    sandbox.write(
        "main.fl",
        &include_str!("../../../tests/language/main.fl")
            .replace("ticket.ready.* = true;", "ticket.ready.* = false;"),
    );
    for mode in [&args[..], &inline[..]] {
        let output = sandbox.run(mode);
        failure(&output, 1);
        let report = String::from_utf8_lossy(&output.stderr);
        assert!(report.contains("(count 28) (failures 2)"), "{report}");
        assert_eq!(report.matches("(passed ").count(), 26);
        assert!(report.contains("template_cleanup") && report.contains("template_early_cleanup"));
    }
    sandbox.write(
        "main.fl",
        &include_str!("../../../tests/language/main.fl").replace(
            "template_boolean_result use typed_case { true }",
            "template_boolean_result use typed_case { 'x' }",
        ),
    );
    let output = sandbox.run(&["main.fl", "--test", "--test-filter", "generic"]);
    failure(&output, 1);
    assert!(details(&output).contains("type mismatch"));
    assert!(!details(&output).contains("(test-results"));
}

#[test]
fn source_test_filters_select_labels_without_skipping_frontend_checks() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "test good_one {} test bad {assumes false;} test good_two {} test {}",
    );
    for base in [
        vec!["input.fl"],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "701",
            "input",
            "input.fl",
            "--primary",
            "1",
        ],
    ] {
        for (filter, count, excluded) in [
            ("good", 2, 2),
            ("anonymous", 1, 3),
            ("absent", 0, 4),
            ("GOOD", 0, 4),
        ] {
            let mut args = base.clone();
            args.extend(["--test", "--test-filter", filter]);
            let output = sandbox.run(&args);
            let report = success(&output);
            assert!(
                report.contains(&format!(
                    "(count {count}) (failures 0) (filtered-out {excluded})"
                )),
                "{report}"
            );
        }
        let mut args = base;
        args.extend(["--test", "--test-filter", ""]);
        let output = sandbox.run(&args);
        failure(&output, 1);
        assert!(details(&output).contains("(count 4) (failures 1) (filtered-out 0)"));
    }
    for args in [
        vec!["input.fl", "--test-filter", "good"],
        vec!["input.fl", "--test", "--test-filter"],
        vec![
            "input.fl",
            "--test",
            "--test-filter",
            "good",
            "--test-filter",
            "bad",
        ],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    sandbox.write("input.fl", "test good {} test bad {unknown();}");
    failure(
        &sandbox.run(&["input.fl", "--test", "--test-filter", "good"]),
        1,
    );
}

#[test]
fn source_tests_discover_files_without_merging_same_label_or_private_helpers() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("nested")).unwrap();
    sandbox.write(
        "main.fl",
        "use alpha.ready; test shared {let result=ready();assumes result;}",
    );
    sandbox.write(
        "alpha.fl",
        "pub fn ready()->bool {true} test shared {let result=ready();assumes result;}",
    );
    sandbox.write(
        "beta.fl",
        "fn ready()->bool {false} test shared {let result=ready();assumes not result;}",
    );
    sandbox.write(
        "nested/gamma.fl",
        "fn ready()->bool {true} test shared {let result=ready();assumes result;}",
    );
    let args = ["main.fl", "--test", "--test-filter", "shared"];
    let output = sandbox.run(&args);
    let report = success(&output).to_owned();
    assert!(report.contains("(count 4) (failures 0)"), "{report}");
    assert_eq!(report.matches("(passed ").count(), 4);
    assert_eq!(success(&sandbox.run(&args)), report);
    sandbox.write(
        "beta.fl",
        "fn ready()->bool {false} test shared {let result=ready();assumes result;}",
    );
    let output = sandbox.run(&args);
    failure(&output, 1);
    let report = String::from_utf8_lossy(&output.stderr);
    assert!(report.contains("(count 4) (failures 1)"), "{report}");
    assert_eq!(report.matches("(passed ").count(), 3);
    // An explicitly supplied nested package is not rediscovered as a module
    // of the primary package, and its tests have their own semantic identity.
    sandbox.write(
        "beta.fl",
        "fn ready()->bool {false} test shared {let result=ready();assumes not result;}",
    );
    fs::create_dir_all(sandbox.path("dependency")).unwrap();
    sandbox.write("dependency/lib.fl", "test shared {assumes false;}");
    let packages = [
        "--packages-v1",
        "--package",
        "1",
        "701",
        "app",
        "main.fl",
        "--package",
        "2",
        "901",
        "dependency",
        "dependency/lib.fl",
        "--test",
        "--test-filter",
        "shared",
    ];
    let mut primary = packages.to_vec();
    primary.extend(["--primary", "1"]);
    assert!(success(&sandbox.run(&primary)).contains("(count 4) (failures 0)"));
    let mut dependency = packages.to_vec();
    dependency.extend(["--primary", "2"]);
    let output = sandbox.run(&dependency);
    failure(&output, 1);
    assert!(details(&output).contains("(count 1) (failures 1)"));
}

#[test]
fn test_inventory_lists_without_executing_and_preserves_action_boundaries() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "test bad {assumes false;} test spin {while true {}} test {} test bad {}",
    );
    let output = sandbox.run(&["input.fl", "--list-tests"]);
    success(&output);
    let dump = String::from_utf8(output.stdout).unwrap();
    assert!(dump.starts_with("(test-inventory"), "{dump}");
    assert!(dump.contains("(count 4)"), "{dump}");
    assert!(!dump.contains("passed"));
    assert_eq!(dump.matches("(name (string \"bad\"))").count(), 2);
    for args in [
        vec!["input.fl", "--list-tests", "--test-filter", "bad"],
        vec![
            "--packages-v1",
            "--package",
            "1",
            "701",
            "app",
            "input.fl",
            "--primary",
            "1",
            "--list-tests",
            "--test-filter",
            "bad",
        ],
    ] {
        let output = sandbox.run(&args);
        success(&output);
        let dump = String::from_utf8(output.stdout).unwrap();
        assert!(dump.contains("(count 2) (filtered-out 2)"), "{dump}");
    }
    for args in [
        vec!["input.fl", "--list-tests", "--test"],
        vec!["input.fl", "--test", "--list-tests"],
        vec!["input.fl", "--list-tests", "--interpret", "run"],
        vec!["input.fl", "--list-tests", "--emit", "nir"],
        vec!["input.fl", "--list-tests", "--inline-functions"],
    ] {
        assert_eq!(sandbox.run(&args).status.code(), Some(2), "{args:?}");
    }
    sandbox.write("input.fl", "test invalid {unknown();}");
    assert_eq!(
        sandbox
            .run(&["input.fl", "--list-tests", "--test-filter", "absent"])
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn comptime_invocation_uses_explicit_resource_bindings_in_both_drivers() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl","typealias Type=.Type'builtin;
        trait Duplicate {}trait Retire {fn drop(*self);}
        impl Duplicate for Type {}
        comptime fn Select()->Type {let selected=bool;let saved=selected.ref.*;selected.ref.* = .u8'builtin;saved}");
    for prefix in [
        vec!["input.fl"],
        vec![
            "--packages-v1",
            "--package",
            "0",
            "17",
            "app",
            "input.fl",
            "--primary",
            "0",
        ],
    ] {
        let mut args = prefix.clone();
        args.extend([
            "--invoke-comptime",
            "Select",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
        ]);
        let output = sandbox.run(&args);
        success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("bool"));
        let mut args = prefix.clone();
        args.extend(["--invoke-comptime", "Select"]);
        failure(&sandbox.run(&args), 1);
        let mut args = prefix;
        args.extend([
            "--invoke-comptime",
            "Select",
            "--copy-protocol",
            "Duplicate",
        ]);
        failure(&sandbox.run(&args), 2);
    }
}

#[test]
fn one_malformed_source_does_not_mark_later_modules_as_parse_failures() {
    for invalid in ["fn broken( {", "fn broken() { 0xGG }"] {
        let sandbox = Sandbox::new();
        sandbox.write("app.fl", "use good.value; fn run()->bool {value()}");
        let args = [
            "--packages-v1",
            "--package",
            "1",
            "101",
            "app",
            "app.fl",
            "--package",
            "2",
            "102",
            "bad",
            "a_bad.fl",
            "--package",
            "3",
            "103",
            "good",
            "z_good.fl",
            "--package",
            "4",
            "104",
            "other",
            "y_bad.fl",
            "--dependency",
            "1",
            "good",
            "3",
            "--primary",
            "1",
            "--emit",
            "hir",
        ];
        sandbox.write("y_bad.fl", "fn initially_valid()->bool {true}");
        sandbox.write("a_bad.fl", invalid);
        sandbox.write("z_good.fl", "pub fn value()->bool {true}");
        let output = sandbox.run(&args);
        failure(&output, 1);
        let message = details(&output);
        assert_eq!(message.matches("FileParsingFailed").count(), 1, "{message}");
        assert!(!message.contains("z_good"), "{message}");
        sandbox.write("y_bad.fl", "fn another( {");
        let output = sandbox.run(&args);
        failure(&output, 1);
        let message = details(&output);
        assert_eq!(message.matches("FileParsingFailed").count(), 2, "{message}");
        assert!(!message.contains("z_good"), "{message}");
        sandbox.write("y_bad.fl", "fn other_repaired()->bool {true}");
        sandbox.write("a_bad.fl", "fn repaired()->bool {true}");
        success(&sandbox.run(&args));
    }
}

#[test]
fn explicit_package_cli_exports_observed_inputs_with_the_selected_result() {
    let sandbox = Sandbox::new();
    sandbox.write("client.fl", "use dep.answer;fn run()->bool {answer()}");
    sandbox.write("provider.fl", "pub fn answer()->bool {true}");
    let base = [
        "--packages-v1",
        "--package",
        "7",
        "701",
        "client",
        "client.fl",
        "--package",
        "9",
        "901",
        "provider",
        "provider.fl",
        "--primary",
        "7",
        "--dependency",
        "7",
        "dep",
        "9",
        "--with-inputs",
    ];
    for action in [
        vec!["--emit", "hir"],
        vec!["--emit", "nir"],
        vec!["--interpret", "run"],
    ] {
        let mut args = base.to_vec();
        args.extend(action);
        let result = sandbox.run(&args);
        let dump = success(&result);
        assert!(
            dump.starts_with("(compilation-result (version 1) (result "),
            "{dump}"
        );
        assert!(dump.contains("(compilation-inputs (version 1)"), "{dump}");
        assert!(
            dump.contains("(source (resolution-id 7) (identity 701)"),
            "{dump}"
        );
        assert!(
            dump.contains("(source (resolution-id 9) (identity 901)"),
            "{dump}"
        );
        assert!(
            dump.contains("(text (string \"pub fn answer()->bool {true}\"))"),
            "{dump}"
        );
        if args.contains(&"--interpret") {
            assert!(dump.contains("(result (bool true))"), "{dump}");
        }
    }
    let mut duplicate = base.to_vec();
    duplicate.push("--with-inputs");
    failure(&sandbox.run(&duplicate), 2);
    failure(&sandbox.run(&["client.fl", "--with-inputs"]), 2);
}

#[test]
fn explicit_package_cli_input_envelope_is_published_only_after_success() {
    let sandbox = Sandbox::new();
    sandbox.write("client.fl", "fn run()->bool {true}");
    let mut args = vec![
        "--packages-v1",
        "--package",
        "1",
        "101",
        "client",
        "client.fl",
        "--primary",
        "1",
        "--with-inputs",
        "--interpret",
        "run",
        "--output",
        "result.lisp",
    ];
    success(&sandbox.run(&args));
    let completed = fs::read_to_string(sandbox.path("result.lisp")).unwrap();
    assert!(completed.contains("(result (bool true))"));
    // A missing entry fails after frontend input capture, before publication.
    let entry = args.iter().position(|arg| *arg == "run").unwrap();
    args[entry] = "missing";
    failure(&sandbox.run(&args), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
        completed
    );
    args[entry] = "run";
    sandbox.write("client.fl", "fn run()->bool {missing}");
    failure(&sandbox.run(&args), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp")).unwrap(),
        completed
    );
    sandbox.write("client.fl", "fn run()->bool {false}");
    *args.last_mut().unwrap() = "client.fl";
    failure(&sandbox.run(&args), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("client.fl")).unwrap(),
        "fn run()->bool {false}"
    );
    *args.last_mut().unwrap() = "result.lisp";
    success(&sandbox.run(&args));
    let updated = fs::read_to_string(sandbox.path("result.lisp")).unwrap();
    assert!(updated.contains("(result (bool false))"));
    assert!(updated.contains("(text (string \"fn run()->bool {false}\"))"));
}

#[test]
fn resident_package_compiler_reuses_and_recovers_across_live_requests() {
    use std::io::{BufRead, BufReader, Write};
    let sandbox = Sandbox::new();
    sandbox.write("client.fl", "fn run()->bool {true}");
    let mut child = Command::new(env!("CARGO_BIN_EXE_luna"))
        .args(["--packages-stream-v1", "--cache-entries", "1"])
        .current_dir(&sandbox.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if send.send(line).is_err() {
                break;
            }
        }
    });
    let encode = |args: &[&str]| {
        args.iter()
            .map(|arg| {
                if arg.is_empty() {
                    "-".into()
                } else {
                    arg.as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let args = [
        "--package",
        "1",
        "101",
        "client",
        "client.fl",
        "--primary",
        "1",
        "--emit",
        "nir",
        "--with-inputs",
    ];
    let mut exchange = |frame: &str| {
        writeln!(input, "{frame}").unwrap();
        input.flush().unwrap();
        match receive.recv_timeout(Duration::from_secs(30)) {
            Ok(line) => line.unwrap(),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("stream reply: {error}");
            }
        }
    };
    let first = exchange(&encode(&args));
    assert!(
        first.contains("(exit-code 0)") && first.contains("(reused false)"),
        "{first}"
    );
    assert!(first.contains("compilation-inputs"));
    assert!(exchange(&encode(&args)).contains("(reused true)"));
    assert!(exchange("invalid hex").contains("(exit-code 2)"));
    sandbox.write("client.fl", "fn run()->bool {false}");
    let changed = exchange(&encode(&args));
    assert!(
        changed.contains("(exit-code 0)") && changed.contains("(reused false)"),
        "{changed}"
    );
    assert!(changed.contains("fn run()->bool {false}"));
    sandbox.write("client.fl", "fn run()->bool {missing}");
    assert!(exchange(&encode(&args)).contains("(exit-code 1)"));
    sandbox.write("client.fl", "fn run()->bool {true}");
    assert!(exchange(&encode(&args)).contains("(exit-code 0)"));
    let mut action = args[..7].to_vec();
    action.extend(["--interpret", "run"]);
    assert!(exchange(&encode(&action)).contains("(exit-code 2)"));
    let mut output = args[..7].to_vec();
    output.extend(["--output", "forbidden.lisp"]);
    assert!(exchange(&encode(&output)).contains("(exit-code 2)"));
    assert!(!sandbox.path("forbidden.lisp").exists());

    drop(exchange);
    drop(input);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
    for invalid in [
        vec!["--packages-stream-v1", "--cache-entries", "-1"],
        vec!["--packages-stream-v1", "--emit", "nir"],
    ] {
        failure(&sandbox.run(&invalid), 2);
    }
}

#[test]
fn native_inline_tests_preserve_package_identity_and_resource_bindings() {
    let sandbox = Sandbox::new();
    sandbox.write("support.fl", "pub trait Duplicate {} pub trait Retire {fn drop(*self);}
        impl Duplicate for bool {} impl Duplicate for *T where T {}
        struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        pub inline effect get()->bool;
        pub inline fn subject(target:*bool)->#get bool {let watch=Watch {target:target};get()#}
        test shared {assumes false;}");
    sandbox.write("app.fl", "use support.get;use support.subject;
        fn shared()->bool {false}
        test shared {let seen=false;let result=subject(seen.ref)# {get()=>true,};assumes result and seen;}
        test shared {let seen=false;let result=subject(seen.ref)# {get()=>do {resume true;},};assumes result and seen;}");
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "app.fl",
        "--package",
        "2",
        "202",
        "support",
        "support.fl",
        "--dependency",
        "1",
        "support",
        "2",
        "--test-native",
        "--inline-functions",
        "--copy-protocol",
        "support.Duplicate",
        "--drop-protocol",
        "support.Retire",
    ];
    for level in ["0", "3"] {
        let mut app = base.to_vec();
        app.extend(["--primary", "1", "--opt-level", level]);
        let output = sandbox.run(&app);
        let report = success(&output);
        assert!(report.contains("(count 2) (failures 0)"), "{report}");
        let mut dependency = base.to_vec();
        dependency.extend(["--primary", "2", "--opt-level", level]);
        let output = sandbox.run(&dependency);
        failure(&output, 1);
        let report = details(&output);
        assert!(report.contains("(count 1) (failures 1)"), "{report}");
    }
}

#[test]
fn package_llvm_dump_selects_primary_identity_and_preserves_inputs() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    sandbox.write("app.fl", "use support.answer;fn run()->bool {answer()}");
    let library = "pub trait Duplicate {} pub trait Retire {fn drop(*self);}impl Duplicate for bool {}pub inline fn answer()->bool {true}fn run()->bool {false}";
    sandbox.write("support.fl", library);
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "app.fl",
        "--package",
        "2",
        "202",
        "support",
        "support.fl",
        "--dependency",
        "1",
        "support",
        "2",
        "--emit-llvm",
        "run",
    ];
    for (primary, expected) in [
        ("1", b"(bool true)\n".as_slice()),
        ("2", b"(bool false)\n".as_slice()),
    ] {
        for level in ["0", "3"] {
            let mut args = base.to_vec();
            args.extend([
                "--primary",
                primary,
                "--inline-functions",
                "--copy-protocol",
                "support.Duplicate",
                "--drop-protocol",
                "support.Retire",
                "--opt-level",
                level,
            ]);
            let output = sandbox.run(&args);
            let ir = success(&output);
            assert!(ir.contains("@main("), "{ir}");
            let bytes = luna::host::executable(ir, &Default::default()).unwrap();
            let path = sandbox.path("native");
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let result = run_command(&mut Command::new(&path));
            assert!(result.status.success());
            assert_eq!(result.stdout, expected);
        }
    }
    let mut observed = base.to_vec();
    observed.extend(["--primary", "1", "--with-inputs"]);
    let output = sandbox.run(&observed);
    let envelope = success(&output);
    assert!(
        envelope.contains("(result (llvm-ir (string \""),
        "{envelope}"
    );
    assert!(envelope.contains("\\n"));
    observed.extend(["--output", "support.fl"]);
    failure(&sandbox.run(&observed), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("support.fl")).unwrap(),
        library
    );
}

#[test]
fn package_native_build_preserves_identity_resources_and_previous_artifacts() {
    let sandbox = Sandbox::new();
    let support = "pub trait Duplicate {}pub trait Retire {fn drop(*self);}impl Duplicate for bool {}impl Duplicate for *T where T {}struct Watch {target:*bool}impl Retire for Watch {fn drop(*self) {self.target.*.* = true;}}pub inline effect get()->bool;pub inline fn answer(target:*bool)->#get bool {let watch=Watch {target:target};get()#}fn run()->bool {false}";
    sandbox.write("support.fl", support);
    sandbox.write("app.fl", "use support.get;use support.answer;fn run()->bool {let seen=false;let result=answer(seen.ref)# {get()=>true,};result and seen}");
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "app.fl",
        "--package",
        "2",
        "202",
        "support",
        "support.fl",
        "--dependency",
        "1",
        "support",
        "2",
        "--build",
        "run",
        "--inline-functions",
        "--copy-protocol",
        "support.Duplicate",
        "--drop-protocol",
        "support.Retire",
    ];
    for level in ["0", "3"] {
        for (primary, expected) in [
            ("1", b"(bool true)\n".as_slice()),
            ("2", b"(bool false)\n".as_slice()),
        ] {
            let mut args = base.to_vec();
            args.extend([
                "--primary",
                primary,
                "--opt-level",
                level,
                "--output",
                "native",
            ]);
            let output = sandbox.run(&args);
            assert_eq!(success(&output), "");
            let output = run_command(&mut Command::new(sandbox.path("native")));
            assert!(output.status.success(), "{}", details(&output));
            assert_eq!(output.stdout, expected);
        }
    }
    let mut args = base.to_vec();
    args.extend(["--primary", "1", "--output", "native"]);
    let previous = fs::read(sandbox.path("native")).unwrap();
    let output = run_command(
        sandbox
            .command()
            .args(&args)
            .env("LUNA_LLVM_CONFIG", sandbox.path("absent-llvm")),
    );
    failure(&output, 1);
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), previous);
    let output = run_command(
        sandbox
            .command()
            .args(&args)
            .env("LUNA_CC", sandbox.path("absent-linker")),
    );
    failure(&output, 1);
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), previous);
    success(&sandbox.run(&args));
    let recovered = run_command(&mut Command::new(sandbox.path("native")));
    assert_eq!(success(&recovered), "(bool true)\n");
    let previous = fs::read(sandbox.path("native")).unwrap();
    let mut exhausted = args.clone();
    exhausted.extend(["--fuel", "1"]);
    failure(&sandbox.run(&exhausted), 1);
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), previous);
    let mut observed = args.clone();
    observed.push("--with-inputs");
    failure(&sandbox.run(&observed), 2);
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), previous);
    args.pop();
    args.push("support.fl");
    failure(&sandbox.run(&args), 1);
    assert_eq!(
        fs::read_to_string(sandbox.path("support.fl")).unwrap(),
        support
    );
    #[cfg(unix)]
    {
        fs::hard_link(sandbox.path("support.fl"), sandbox.path("alias")).unwrap();
        args.pop();
        args.push("alias");
        failure(&sandbox.run(&args), 1);
        assert_eq!(
            fs::read_to_string(sandbox.path("support.fl")).unwrap(),
            support
        );
    }
    let mut missing_output = base.to_vec();
    missing_output.extend(["--primary", "1"]);
    failure(&sandbox.run(&missing_output), 2);
}

#[test]
fn package_discard_result_uses_selected_identity_and_checked_drop() {
    let sandbox = Sandbox::new();
    sandbox.write("support.fl","pub trait Duplicate {}pub trait Retire {fn drop(*self);}impl Duplicate for bool {}pub struct Token {valid:bool}impl Retire for Token {fn drop(*self) {assumes self.valid.*;self.valid.* = false;}}pub inline fn make()->Token {Token {valid:true}}fn run()->Token {Token {valid:false}}");
    sandbox.write(
        "app.fl",
        "use support.Token;use support.make;fn run()->Token {make()}",
    );
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "app.fl",
        "--package",
        "2",
        "202",
        "support",
        "support.fl",
        "--dependency",
        "1",
        "support",
        "2",
        "--discard-result",
        "--inline-functions",
        "--copy-protocol",
        "support.Duplicate",
        "--drop-protocol",
        "support.Retire",
    ];
    for level in ["0", "3"] {
        for primary in ["1", "2"] {
            let mut args = base.to_vec();
            args.extend([
                "--primary",
                primary,
                "--build",
                "run",
                "--output",
                "native",
                "--opt-level",
                level,
            ]);
            success(&sandbox.run(&args));
            let result = run_command(&mut Command::new(sandbox.path("native")));
            if primary == "1" {
                assert_eq!(success(&result), "(unit)\n");
            } else {
                assert!(!result.status.success());
                assert!(result.stdout.is_empty());
            }
        }
    }
    let mut args = base.to_vec();
    args.extend(["--primary", "1", "--emit-llvm", "run", "--with-inputs"]);
    let output = sandbox.run(&args);
    assert!(success(&output).contains("(result (llvm-ir (string"));
}

#[test]
fn intrinsic_view_hir_preserves_tag_call_and_explicit_result_boundary() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "enum Inst {fma,barrier}typealias F=.f32'builtin;fn run(a:F,b:F,c:F)->F {Inst.fma'intrinsic(a,b,c).as(F)}fn wait() {Inst.barrier'intrinsic();}");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let hir = success(&output);
    assert_eq!(hir.matches("(take-intrinsic (tag ").count(), 2, "{hir}");
    assert!(hir.contains("(cast (value "), "{hir}");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    failure(&output, 1);
    assert!(
        details(&output).contains("intrinsic_enum"),
        "{}",
        details(&output)
    );
}

#[test]
fn intrinsic_surface_does_not_bypass_operand_resolution_or_unsafe_admission() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "enum Inst {op}fn run() {Inst.op'intrinsic(missing);}",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    failure(&output, 1);
    assert!(details(&output).contains("missing"));
    sandbox.write(
        "input.fl",
        "enum Inst {op}fn run()->bool {unsafe {Inst.op'intrinsic(true).as(bool)}}",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let hir = success(&output);
    assert!(hir.contains("(take-intrinsic (tag "));
    assert!(hir.contains("unsafe-block-expr"), "{hir}");
    failure(&sandbox.run(&["input.fl", "--emit", "nir"]), 1);
    sandbox.write("input.fl","enum Inst {op}template fn signature(value:bool)->bool;fn wrap use signature {Inst.op'intrinsic(value).as(bool)}fn run()->bool {wrap(true)}");
    success(&sandbox.run(&["input.fl", "--emit", "hir"]));
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    failure(&output, 1);
    assert!(
        !details(&output).contains("panicked"),
        "{}",
        details(&output)
    );
}

#[test]
fn unsafe_hir_retains_nested_scopes_and_function_call_boundary() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl","unsafe fn raw(value:bool)->bool {value}fn run()->bool {unsafe {let value=unsafe {raw(true)};value}}");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let hir = success(&output);
    assert_eq!(hir.matches("(unsafe-block-expr ").count(), 2, "{hir}");
    assert_eq!(hir.matches("(unsafe true)").count(), 1, "{hir}");
    assert!(hir.contains("(unsafe false)"));
    assert!(!hir.contains("unsafe function admission"));
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(success(&output).contains("(unsafe true)"));
    assert!(!details(&output).contains("panicked"));
    sandbox.write("input.fl", "fn run()->bool {unsafe {missing}}");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    failure(&output, 1);
    assert!(details(&output).contains("missing"));
}

#[test]
fn callable_type_modifiers_survive_hir_without_erasing_admission() {
    let sandbox = Sandbox::new();
    for (prefix, field) in [
        ("unsafe", "unsafe"),
        ("pure", "pure"),
        ("inline", "inline"),
        ("spec", "spec"),
        ("verified", "verified"),
    ] {
        sandbox.write(
            "input.fl",
            &format!("typealias Callback={prefix} fn(comptime T:.Type'builtin,value:T)->T;"),
        );
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        let hir = success(&output);
        assert!(hir.contains("(ty-modified-fn "), "{hir}");
        assert!(hir.contains(&format!("({field} true)")), "{hir}");
        assert!(hir.contains("(comptime false)"), "{hir}");
        assert!(hir.contains("(ty-fn-arrow "), "{hir}");
        assert_eq!(
            hir.contains("function type modifiers"),
            prefix != "unsafe",
            "{hir}"
        );
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        if prefix == "unsafe" {
            success(&output);
        } else {
            failure(&output, 1);
        }
        assert!(!details(&output).contains("panicked"));
    }
    sandbox.write("input.fl", "typealias Callback=pure comptime inline unsafe fn(comptime T:.Type'builtin,value:T)->unsafe fn(T)->T;");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let hir = success(&output);
    assert_eq!(hir.matches("(ty-modified-fn ").count(), 2, "{hir}");
    assert!(hir.contains("(fn-type-modifiers (pure true) (comptime true) (inline true) (unsafe true) (spec false) (verified false))"), "{hir}");
    assert!(hir.contains("(fn-type-modifiers (pure false) (comptime false) (inline false) (unsafe true) (spec false) (verified false))"), "{hir}");
    sandbox.write(
        "input.fl",
        "typealias Callback=unsafe fn(comptime T:.Type'builtin,value:T)->Missing;",
    );
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    failure(&output, 1);
    assert!(details(&output).contains("Missing"));
}

#[test]
fn intrinsic_enum_attribute_preserves_family_identity_without_binding_an_operation() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "^intrinsic_enum enum TargetOps {fma,barrier}enum Ordinary {fma,barrier}^intrinsic_enum enum OtherTarget {fma}");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let hir = success(&output);
    assert_eq!(hir.matches("(intrinsic true)").count(), 2, "{hir}");
    assert_eq!(hir.matches("(intrinsic false)").count(), 1, "{hir}");
    assert!(!hir.contains("definition attribute or modifier"), "{hir}");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    let nir = success(&output);
    assert_eq!(nir.matches("(intrinsic true)").count(), 2, "{nir}");
    assert_eq!(nir.matches("(intrinsic false)").count(), 1, "{nir}");
    for name in ["TargetOps", "Ordinary", "OtherTarget"] {
        assert!(nir.contains(&format!("(name \"{name}\")")), "{nir}");
    }
    sandbox.write("input.fl", "^intrinsic_enum enum TargetOps {fma}fn run()->bool {unsafe {TargetOps.fma'intrinsic(true).as(bool)}}");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(success(&output).contains("(intrinsic (intrinsic-tag"));
    let output = sandbox.run(&["input.fl", "--build", "run", "--output", "native"]);
    failure(&output, 1);
    assert!(
        details(&output).contains("intrinsic target behavior"),
        "{}",
        details(&output)
    );
}

#[test]
fn intrinsic_enum_attribute_requires_an_enum_and_preserves_other_attribute_checks() {
    let sandbox = Sandbox::new();
    for source in [
        "^intrinsic_enum fn run()->bool {true}",
        "^intrinsic_enum struct NotTags {}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        failure(&output, 1);
        assert!(
            details(&output).contains("intrinsic_enum requires an enum declaration"),
            "{}",
            details(&output)
        );
    }
    for source in [
        "^unhandled ^intrinsic_enum enum Tags {fma}",
        "^intrinsic_enum ^unhandled enum Tags {fma}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "hir"]);
        let hir = success(&output);
        assert!(hir.contains("(intrinsic true)"), "{hir}");
        assert!(hir.contains("definition attribute"), "{hir}");
        failure(&sandbox.run(&["input.fl", "--emit", "nir"]), 1);
    }
}

#[test]
fn source_intrinsic_output_does_not_follow_surrounding_context_or_late_casts() {
    let sandbox = Sandbox::new();
    for body in [
        "Ops.op'intrinsic(true)",
        "let result:bool=Ops.op'intrinsic(true);result",
        "let result=Ops.op'intrinsic(true);result.as(bool)",
        "(do {Ops.op'intrinsic(true)}).as(bool)",
    ] {
        sandbox.write(
            "input.fl",
            &format!("^intrinsic_enum enum Ops {{op}} unsafe fn run()->bool {{{body}}}"),
        );
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        failure(&output, 1);
        assert!(
            !details(&output).contains("panicked"),
            "{}",
            details(&output)
        );
    }
    for body in [
        "Ops.op'intrinsic(true).as(bool)",
        "(Ops.op'intrinsic(true)).as(bool)",
    ] {
        sandbox.write(
            "input.fl",
            &format!("^intrinsic_enum enum Ops {{op}} unsafe fn run()->bool {{{body}}}"),
        );
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        assert!(success(&output).contains("(intrinsic (intrinsic-tag"));
    }
}

#[test]
fn source_intrinsic_checks_permission_phase_and_arguments_before_publication() {
    let sandbox = Sandbox::new();
    for (source, diagnostic) in [
        (
            "^intrinsic_enum enum Ops {op} fn run()->bool {Ops.op'intrinsic(true).as(bool)}",
            "unsafe",
        ),
        (
            "^intrinsic_enum enum Ops {op} comptime fn run()->bool {unsafe {Ops.op'intrinsic(true).as(bool)}}",
            "comptime",
        ),
        (
            "^intrinsic_enum enum Ops {op} unsafe fn run()->bool {Ops.op'intrinsic(missing).as(bool)}",
            "missing",
        ),
        (
            "^intrinsic_enum enum Ops {op} unsafe fn run()->bool {Ops.op'intrinsic(not 4).as(bool)}",
            "",
        ),
        (
            "^intrinsic_enum enum Ops {op(bool)} unsafe fn run()->bool {Ops.op'intrinsic(true).as(bool)}",
            "payload-free",
        ),
        (
            "^intrinsic_enum enum Ops {op} unsafe fn run()->bool {Ops.op'intrinsic(true).as(missing)}",
            "missing",
        ),
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        failure(&output, 1);
        assert!(
            details(&output).contains(diagnostic),
            "{}",
            details(&output)
        );
        assert!(!details(&output).contains("panicked"));
    }
}

#[test]
fn host_fma_cli_binds_qualified_source_before_purity_and_runs_example() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        include_str!("../../../development/examples/host-intrinsic-fma.fl"),
    );
    for level in ["0", "3"] {
        success(&sandbox.run(&[
            "input.fl",
            "--host-fma",
            "input.machine.Scalar.fused",
            "--build",
            "run",
            "--opt-level",
            level,
            "--output",
            "native",
        ]));
        let output = run_command(&mut Command::new(sandbox.path("native")));
        assert_eq!(success(&output), "(float32 (bits 41880000))\n");
    }
    let output = sandbox.run(&[
        "input.fl",
        "--host-fma",
        "input.machine.Scalar.fused",
        "--emit-llvm",
        "run",
    ]);
    assert!(success(&output).contains("@llvm.fma.f32"));
    // An unbound pure declaration cannot claim target behavior on its own.
    failure(&sandbox.run(&["input.fl", "--emit", "nir"]), 1);
}

#[test]
fn host_fma_cli_rejects_invalid_selectors_and_preserves_output() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "^intrinsic_enum enum Ops {op} enum Ordinary {op} fn run()->bool {true}",
    );
    sandbox.write("native", "previous");
    for selector in [
        "Ops.op",
        "missing.Ops.op",
        "input.Missing.op",
        "input.Ordinary.op",
        "input.Ops.missing",
        "input..op",
    ] {
        let output = sandbox.run(&[
            "input.fl",
            "--build",
            "run",
            "--host-fma",
            selector,
            "--output",
            "native",
        ]);
        failure(&output, 1);
        assert_eq!(fs::read(sandbox.path("native")).unwrap(), b"previous");
    }
    for args in [
        vec!["input.fl", "--emit", "nir", "--host-fma", "input.Ops.op"],
        vec![
            "input.fl",
            "--emit-llvm",
            "run",
            "--host-fma",
            "input.Ops.op",
            "--host-fma",
            "input.Ops.op",
        ],
        vec!["input.fl", "--emit-llvm", "run", "--host-fma"],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    sandbox.write(
        "input.fl",
        "^intrinsic_enum enum Ops {op} fn run()->bool {unsafe {Ops.op'intrinsic(true).as(bool)}}",
    );
    let output = sandbox.run(&[
        "input.fl",
        "--build",
        "run",
        "--host-fma",
        "input.Ops.op",
        "--output",
        "native",
    ]);
    failure(&output, 1);
    assert!(
        details(&output).contains("requires 3 operands"),
        "{}",
        details(&output)
    );
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), b"previous");
}

#[test]
fn host_fma_package_cli_uses_declared_package_identity() {
    let sandbox = Sandbox::new();
    let source = include_str!("../../../development/examples/host-intrinsic-fma.fl");
    sandbox.write("one.fl", source);
    sandbox.write("two.fl", source);
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "one",
        "one.fl",
        "--package",
        "2",
        "202",
        "two",
        "two.fl",
        "--primary",
        "1",
    ];
    let mut args = base.to_vec();
    args.extend([
        "--host-fma",
        "one.machine.Scalar.fused",
        "--emit-llvm",
        "run",
    ]);
    // Both source packages contain pure wrappers: binding one cannot authorize
    // the unrelated same-spelled tag in the other package.
    failure(&sandbox.run(&args), 1);
    sandbox.write(
        "two.fl",
        "^intrinsic_enum enum Scalar {fused} fn run()->bool {true}",
    );
    let output = sandbox.run(&args);
    assert!(success(&output).contains("@llvm.fma.f32"));
    let mut args = base.to_vec();
    args.extend([
        "--host-fma",
        "one.machine.Scalar.fused",
        "--build",
        "run",
        "--output",
        "native",
    ]);
    success(&sandbox.run(&args));
    assert_eq!(
        success(&run_command(&mut Command::new(sandbox.path("native")))),
        "(float32 (bits 41880000))\n"
    );
}

#[test]
fn host_fma_binding_preserves_resources_purity_and_unsafe_responsibility() {
    let sandbox = Sandbox::new();
    let header = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for .f32'builtin {} impl Duplicate for bool {}\n";
    sandbox.write(
        "input.fl",
        &format!(
            "{header}{}",
            include_str!("../../../development/examples/host-intrinsic-fma.fl")
        ),
    );
    success(&sandbox.run(&[
        "input.fl",
        "--host-fma",
        "input.machine.Scalar.fused",
        "--copy-protocol",
        "Duplicate",
        "--drop-protocol",
        "Retire",
        "--build",
        "run",
        "--output",
        "native",
    ]));
    assert_eq!(
        success(&run_command(&mut Command::new(sandbox.path("native")))),
        "(float32 (bits 41880000))\n"
    );
    let old = fs::read(sandbox.path("native")).unwrap();
    for (body, diagnostic) in [
        (
            "fn run()->F {let x:F=1.0;Ops.op'intrinsic(x,x,x).as(F)}",
            "unsafe",
        ),
        (
            "comptime fn run()->F {let x:F=1.0;unsafe {Ops.op'intrinsic(x,x,x).as(F)}}",
            "comptime",
        ),
        (
            "pure fn run(p:*F)->F {unsafe {Ops.op'intrinsic(p.*,p.*,p.*).as(F)}}",
            "P4 evidence",
        ),
    ] {
        sandbox.write(
            "input.fl",
            &format!("^intrinsic_enum enum Ops {{op}} typealias F=.f32'builtin;{body}"),
        );
        let output = sandbox.run(&[
            "input.fl",
            "--host-fma",
            "input.Ops.op",
            "--build",
            "run",
            "--output",
            "native",
        ]);
        failure(&output, 1);
        assert!(
            details(&output).contains(diagnostic),
            "{}",
            details(&output)
        );
        assert!(!details(&output).contains("panicked"));
        assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
    }
}

#[test]
fn repository_math_package_runs_generic_fma_and_rejects_nonfloat_inputs() {
    let sandbox = Sandbox::new();
    let math =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/builtin/math_intrinsics.fl");
    let math = math.to_str().unwrap();
    sandbox.write(
        "app.fl",
        include_str!("../../../development/examples/host-math-fma.fl"),
    );
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "app.fl",
        "--package",
        "2",
        "202",
        "math",
        math,
        "--dependency",
        "1",
        "math",
        "2",
        "--primary",
        "1",
        "--host-fma",
        "math.target.Scalar.fma",
        "--host-sqrt",
        "math.target.Scalar.sqrt",
        "--host-copysign",
        "math.target.Scalar.copysign",
        "--host-intrinsic",
        "floor=math.target.Scalar.floor",
        "--host-intrinsic",
        "ceil=math.target.Scalar.ceil",
        "--host-intrinsic",
        "trunc=math.target.Scalar.trunc",
        "--host-intrinsic",
        "roundeven=math.target.Scalar.roundeven",
        "--host-intrinsic",
        "round=math.target.Scalar.round",
        "--host-intrinsic",
        "fabs=math.target.Scalar.fabs",
        "--host-intrinsic",
        "sin=math.target.Scalar.sin",
        "--host-intrinsic",
        "cos=math.target.Scalar.cos",
        "--host-intrinsic",
        "exp=math.target.Scalar.exp",
        "--host-intrinsic",
        "exp2=math.target.Scalar.exp2",
        "--host-intrinsic",
        "log=math.target.Scalar.log",
        "--host-intrinsic",
        "log2=math.target.Scalar.log2",
        "--host-intrinsic",
        "log10=math.target.Scalar.log10",
        "--host-intrinsic",
        "minimum=math.target.Scalar.minimum",
        "--host-intrinsic",
        "maximum=math.target.Scalar.maximum",
        "--host-intrinsic",
        "bswap=math.target.Scalar.bswap",
        "--host-intrinsic",
        "fshl=math.target.Scalar.fshl",
        "--host-intrinsic",
        "fshr=math.target.Scalar.fshr",
        "--host-intrinsic",
        "minimumnum=math.target.Scalar.minimumnum",
        "--host-intrinsic",
        "maximumnum=math.target.Scalar.maximumnum",
        "--host-intrinsic",
        "isnan=math.target.Scalar.isnan",
        "--host-intrinsic",
        "isinf=math.target.Scalar.isinf",
        "--host-intrinsic",
        "isfinite=math.target.Scalar.isfinite",
        "--host-intrinsic",
        "isnormal=math.target.Scalar.isnormal",
        "--host-intrinsic",
        "issubnormal=math.target.Scalar.issubnormal",
        "--host-intrinsic",
        "signbit=math.target.Scalar.signbit",
        "--host-intrinsic",
        "add_sat=math.target.Scalar.add_sat",
        "--host-intrinsic",
        "sub_sat=math.target.Scalar.sub_sat",
        "--host-intrinsic",
        "min=math.target.Scalar.min",
        "--host-intrinsic",
        "max=math.target.Scalar.max",
        "--host-intrinsic",
        "abs=math.target.Scalar.abs",
        "--host-intrinsic",
        "bit_and=math.target.Scalar.bit_and",
        "--host-intrinsic",
        "bit_or=math.target.Scalar.bit_or",
        "--host-intrinsic",
        "bit_xor=math.target.Scalar.bit_xor",
        "--host-intrinsic",
        "bitreverse=math.target.Scalar.bitreverse",
        "--host-intrinsic",
        "ctpop=math.target.Scalar.ctpop",
        "--host-intrinsic",
        "ctlz=math.target.Scalar.ctlz",
        "--host-intrinsic",
        "cttz=math.target.Scalar.cttz",
        "--host-intrinsic",
        "pow=math.target.Scalar.pow",
        "--host-intrinsic",
        "powi=math.target.Scalar.powi",
        "--host-intrinsic",
        "ldexp=math.target.Scalar.ldexp",
    ];
    for level in ["0", "3"] {
        let mut args = base.to_vec();
        args.extend(["--build", "run", "--output", "native", "--opt-level", level]);
        success(&sandbox.run(&args));
        assert_eq!(
            success(&run_command(&mut Command::new(sandbox.path("native")))),
            "(bool true)\n"
        );
    }
    let mut args = base.to_vec();
    args.extend(["--emit-llvm", "run"]);
    let output = sandbox.run(&args);
    let ir = success(&output);
    assert!(
        ir.contains("@llvm.fma.f32") && ir.contains("@llvm.fma.f64"),
        "{ir}"
    );
    sandbox.write("app.fl",&format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} impl Duplicate for .f32'builtin {{}} impl Duplicate for .f64'builtin {{}} impl Duplicate for bool {{}}\n{}",include_str!("../../../development/examples/host-math-fma.fl")));
    let mut checked = base.to_vec();
    checked.extend([
        "--build",
        "run",
        "--output",
        "native",
        "--copy-protocol",
        "app.Duplicate",
        "--drop-protocol",
        "app.Retire",
        "--opt-level",
        "3",
    ]);
    success(&sandbox.run(&checked));
    assert_eq!(
        success(&run_command(&mut Command::new(sandbox.path("native")))),
        "(bool true)\n"
    );
    let old = fs::read(sandbox.path("native")).unwrap();
    sandbox.write("app.fl", "fn run()->bool {math.fma<bool>(true,true,true)}");
    let mut args = base.to_vec();
    args.extend(["--build", "run", "--output", "native"]);
    let output = sandbox.run(&args);
    failure(&output, 1);
    assert!(
        details(&output).contains("type requirement"),
        "{}",
        details(&output)
    );
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
    sandbox.write(
        "app.fl",
        "fn run()->.f32'builtin {let a:.f32'builtin=1.0;let b:.f64'builtin=2.0;math.fma(a,b,a)}",
    );
    failure(&sandbox.run(&args), 1);
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
}

#[test]
fn native_tests_bind_host_fma_for_single_source_and_real_math_package() {
    let sandbox = Sandbox::new();
    let declarations = include_str!("../../../development/examples/host-intrinsic-fma.fl");
    sandbox.write(
        "single.fl",
        &format!("{declarations}\ntest fused {{let value = run(); assumes value == 17.0;}}"),
    );
    for level in ["0", "3"] {
        let output = sandbox.run(&[
            "single.fl",
            "--test-native",
            "--host-fma",
            "input.machine.Scalar.fused",
            "--opt-level",
            level,
        ]);
        assert!(success(&output).contains("(count 1) (failures 0)"));
    }
    for action in ["--test", "--list-tests", "--emit"] {
        let mut args = vec!["single.fl", action];
        if action == "--emit" {
            args.push("nir");
        }
        args.extend(["--host-fma", "input.machine.Scalar.fused"]);
        let output = sandbox.run(&args);
        failure(&output, 2);
        assert!(details(&output).contains("--host-fma requires"));
    }
    let output = sandbox.run(&[
        "single.fl",
        "--test-native",
        "--host-fma",
        "input.machine.Scalar.missing",
    ]);
    failure(&output, 1);
    let math =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/builtin/math_intrinsics.fl");
    sandbox.write(
        "app.fl",
        &format!(
            "{}\ntest fused {{let value = run(); assumes value;}}\ntest failing {{let value = run(); assumes not value;}}",
            include_str!("../../../development/examples/host-math-fma.fl")
        ),
    );
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "app.fl",
        "--package",
        "2",
        "202",
        "math",
        math.to_str().unwrap(),
        "--dependency",
        "1",
        "math",
        "2",
        "--primary",
        "1",
        "--host-fma",
        "math.target.Scalar.fma",
        "--host-sqrt",
        "math.target.Scalar.sqrt",
        "--host-copysign",
        "math.target.Scalar.copysign",
        "--host-intrinsic",
        "floor=math.target.Scalar.floor",
        "--host-intrinsic",
        "ceil=math.target.Scalar.ceil",
        "--host-intrinsic",
        "trunc=math.target.Scalar.trunc",
        "--host-intrinsic",
        "roundeven=math.target.Scalar.roundeven",
        "--host-intrinsic",
        "round=math.target.Scalar.round",
        "--host-intrinsic",
        "fabs=math.target.Scalar.fabs",
        "--host-intrinsic",
        "sin=math.target.Scalar.sin",
        "--host-intrinsic",
        "cos=math.target.Scalar.cos",
        "--host-intrinsic",
        "exp=math.target.Scalar.exp",
        "--host-intrinsic",
        "exp2=math.target.Scalar.exp2",
        "--host-intrinsic",
        "log=math.target.Scalar.log",
        "--host-intrinsic",
        "log2=math.target.Scalar.log2",
        "--host-intrinsic",
        "log10=math.target.Scalar.log10",
        "--host-intrinsic",
        "minimum=math.target.Scalar.minimum",
        "--host-intrinsic",
        "maximum=math.target.Scalar.maximum",
        "--host-intrinsic",
        "bswap=math.target.Scalar.bswap",
        "--host-intrinsic",
        "fshl=math.target.Scalar.fshl",
        "--host-intrinsic",
        "fshr=math.target.Scalar.fshr",
        "--host-intrinsic",
        "minimumnum=math.target.Scalar.minimumnum",
        "--host-intrinsic",
        "maximumnum=math.target.Scalar.maximumnum",
        "--host-intrinsic",
        "isnan=math.target.Scalar.isnan",
        "--host-intrinsic",
        "isinf=math.target.Scalar.isinf",
        "--host-intrinsic",
        "isfinite=math.target.Scalar.isfinite",
        "--host-intrinsic",
        "isnormal=math.target.Scalar.isnormal",
        "--host-intrinsic",
        "issubnormal=math.target.Scalar.issubnormal",
        "--host-intrinsic",
        "signbit=math.target.Scalar.signbit",
        "--host-intrinsic",
        "add_sat=math.target.Scalar.add_sat",
        "--host-intrinsic",
        "sub_sat=math.target.Scalar.sub_sat",
        "--host-intrinsic",
        "min=math.target.Scalar.min",
        "--host-intrinsic",
        "max=math.target.Scalar.max",
        "--host-intrinsic",
        "abs=math.target.Scalar.abs",
        "--host-intrinsic",
        "bit_and=math.target.Scalar.bit_and",
        "--host-intrinsic",
        "bit_or=math.target.Scalar.bit_or",
        "--host-intrinsic",
        "bit_xor=math.target.Scalar.bit_xor",
        "--host-intrinsic",
        "bitreverse=math.target.Scalar.bitreverse",
        "--host-intrinsic",
        "ctpop=math.target.Scalar.ctpop",
        "--host-intrinsic",
        "ctlz=math.target.Scalar.ctlz",
        "--host-intrinsic",
        "cttz=math.target.Scalar.cttz",
        "--host-intrinsic",
        "pow=math.target.Scalar.pow",
        "--host-intrinsic",
        "powi=math.target.Scalar.powi",
        "--host-intrinsic",
        "ldexp=math.target.Scalar.ldexp",
        "--test-native",
    ];
    for level in ["0", "3"] {
        let mut args = base.to_vec();
        args.extend(["--opt-level", level, "--test-filter", "fused"]);
        let output = sandbox.run(&args);
        assert!(success(&output).contains("(count 1) (failures 0)"));
    }
    let source = fs::read_to_string(sandbox.path("app.fl")).unwrap();
    sandbox.write("app.fl", &format!("trait Duplicate {{}} trait Retire {{fn drop(*self);}} impl Duplicate for .f32'builtin {{}} impl Duplicate for .f64'builtin {{}} impl Duplicate for bool {{}}\n{source}"));
    let mut checked = base.to_vec();
    checked.extend([
        "--copy-protocol",
        "app.Duplicate",
        "--drop-protocol",
        "app.Retire",
        "--test-filter",
        "fused",
        "--opt-level",
        "3",
    ]);
    let output = sandbox.run(&checked);
    assert!(success(&output).contains("(count 1) (failures 0)"));
    let output = sandbox.run(&base);
    failure(&output, 1);
    assert!(
        details(&output).contains("(count 2) (failures 1)"),
        "{}",
        details(&output)
    );
}

#[test]
fn repository_resource_package_executes_native_clone_and_cleanup() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "app.fl",
        include_str!("../../../development/examples/host-library-resources.fl"),
    );
    let resources =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/builtin/resources.fl");
    for body in ["self.*", "Self { counter: self.counter.* }"] {
        let source = include_str!("../../../development/examples/host-library-resources.fl")
            .replace("\n        self.*\n", &format!("\n        {body}\n"));
        sandbox.write("app.fl", &source);
        for level in ["0", "3"] {
            let output = sandbox.run(&[
                "--packages-v1",
                "--package",
                "1",
                "101",
                "app",
                "app.fl",
                "--package",
                "2",
                "202",
                "resources",
                resources.to_str().unwrap(),
                "--dependency",
                "1",
                "resources",
                "2",
                "--primary",
                "1",
                "--copy-protocol",
                "resources.Copy",
                "--drop-protocol",
                "resources.Drop",
                "--test-native",
                "--opt-level",
                level,
            ]);
            assert!(success(&output).contains("(count 4) (failures 0)"));
        }
    }
}

#[test]
fn host_sqrt_selector_runs_generic_code_and_rejects_conflicts() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "mod machine {^intrinsic_enum pub enum Scalar {radical}}
        pure fn root(x:T)->T where T, requires T == .f32'builtin or T == .f64'builtin {
            unsafe {machine.Scalar.radical'intrinsic(x).as(T)}
        }
        fn run()->.f64'builtin {root<.f64'builtin>(81.0)}
        test {let value=root<.f32'builtin>(16.0); assumes value == 4.0;}",
    );
    let selector = "input.machine.Scalar.radical";
    for level in ["0", "3"] {
        success(&sandbox.run(&[
            "input.fl",
            "--host-sqrt",
            selector,
            "--build",
            "run",
            "--output",
            "native",
            "--opt-level",
            level,
        ]));
        assert_eq!(
            success(&run_command(&mut Command::new(sandbox.path("native")))),
            "(float64 (bits 4022000000000000))\n"
        );
        assert!(
            success(&sandbox.run(&[
                "input.fl",
                "--host-sqrt",
                selector,
                "--test-native",
                "--opt-level",
                level
            ]))
            .contains("(count 1) (failures 0)")
        );
    }
    let old = fs::read(sandbox.path("native")).unwrap();
    let output = sandbox.run(&[
        "input.fl",
        "--host-sqrt",
        selector,
        "--host-fma",
        selector,
        "--build",
        "run",
        "--output",
        "native",
    ]);
    failure(&output, 1);
    assert!(
        details(&output).contains("cannot bind both"),
        "{}",
        details(&output)
    );
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
    for args in [
        vec!["input.fl", "--host-sqrt", selector, "--emit", "nir"],
        vec![
            "input.fl",
            "--host-sqrt",
            selector,
            "--host-sqrt",
            selector,
            "--emit-llvm",
            "run",
        ],
        vec!["input.fl", "--emit-llvm", "run", "--host-sqrt"],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    for (source, expected) in [
        (
            "fn run()->F {let x:F=4.0; unsafe {Ops.sqrt'intrinsic(x,x).as(F)}}",
            "requires 1 operands",
        ),
        (
            "fn run()->F {unsafe {Ops.sqrt'intrinsic(true).as(F)}}",
            "must match",
        ),
        (
            "comptime fn run()->F {let x:F=4.0; unsafe {Ops.sqrt'intrinsic(x).as(F)}}",
            "comptime",
        ),
    ] {
        sandbox.write(
            "input.fl",
            &format!("^intrinsic_enum enum Ops {{sqrt}} typealias F=.f32'builtin; {source}"),
        );
        let output = sandbox.run(&[
            "input.fl",
            "--host-sqrt",
            "input.Ops.sqrt",
            "--build",
            "run",
            "--output",
            "native",
        ]);
        failure(&output, 1);
        assert!(details(&output).contains(expected), "{}", details(&output));
        assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
    }
}

#[test]
fn host_copysign_selector_runs_generic_code_and_rejects_conflicts() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "mod machine {^intrinsic_enum pub enum Scalar {radical}}
        pure fn root(x:T,y:T)->T where T, requires T == .f32'builtin or T == .f64'builtin {
            unsafe {machine.Scalar.radical'intrinsic(x,y).as(T)}
        }
        fn run()->.f64'builtin {root<.f64'builtin>(3.5,-0.0)}
        test {let value=root<.f32'builtin>(3.5,-0.0); assumes value == -3.5;}",
    );
    let selector = "input.machine.Scalar.radical";
    for level in ["0", "3"] {
        success(&sandbox.run(&[
            "input.fl",
            "--host-copysign",
            selector,
            "--build",
            "run",
            "--output",
            "native",
            "--opt-level",
            level,
        ]));
        assert_eq!(
            success(&run_command(&mut Command::new(sandbox.path("native")))),
            "(float64 (bits c00c000000000000))\n"
        );
        assert!(
            success(&sandbox.run(&[
                "input.fl",
                "--host-copysign",
                selector,
                "--test-native",
                "--opt-level",
                level
            ]))
            .contains("(count 1) (failures 0)")
        );
    }
    let old = fs::read(sandbox.path("native")).unwrap();
    let output = sandbox.run(&[
        "input.fl",
        "--host-copysign",
        selector,
        "--host-fma",
        selector,
        "--build",
        "run",
        "--output",
        "native",
    ]);
    failure(&output, 1);
    assert!(
        details(&output).contains("cannot bind both"),
        "{}",
        details(&output)
    );
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
    for args in [
        vec!["input.fl", "--host-copysign", selector, "--emit", "nir"],
        vec![
            "input.fl",
            "--host-copysign",
            selector,
            "--host-copysign",
            selector,
            "--emit-llvm",
            "run",
        ],
        vec!["input.fl", "--emit-llvm", "run", "--host-copysign"],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    for (source, expected) in [
        (
            "fn run()->F {let x:F=4.0; unsafe {Ops.sqrt'intrinsic(x).as(F)}}",
            "requires 2 operands",
        ),
        (
            "fn run()->F {unsafe {Ops.sqrt'intrinsic(true,true).as(F)}}",
            "must match",
        ),
        (
            "comptime fn run()->F {let x:F=4.0; unsafe {Ops.sqrt'intrinsic(x,x).as(F)}}",
            "comptime",
        ),
    ] {
        sandbox.write(
            "input.fl",
            &format!("^intrinsic_enum enum Ops {{sqrt}} typealias F=.f32'builtin; {source}"),
        );
        let output = sandbox.run(&[
            "input.fl",
            "--host-copysign",
            "input.Ops.sqrt",
            "--build",
            "run",
            "--output",
            "native",
        ]);
        failure(&output, 1);
        assert!(details(&output).contains(expected), "{}", details(&output));
        assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
    }
}

#[test]
fn repeated_host_intrinsic_bindings_keep_package_identities_separate() {
    let sandbox = Sandbox::new();
    let library = "^intrinsic_enum pub enum Ops {root}
        pub pure fn root(value:T)->T where T, requires T == .f32'builtin or T == .f64'builtin {
            unsafe {Ops.root'intrinsic(value).as(T)}
        }";
    sandbox.write("left.fl", library);
    sandbox.write("right.fl", library);
    sandbox.write("app.fl","fn run()->bool {left.root<.f32'builtin>(4.0) == 2.0 and right.root<.f64'builtin>(9.0) == 3.0}
        test {let ok=run();assumes ok;}");
    let base = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "app.fl",
        "--package",
        "2",
        "202",
        "left",
        "left.fl",
        "--package",
        "3",
        "303",
        "right",
        "right.fl",
        "--dependency",
        "1",
        "left",
        "2",
        "--dependency",
        "1",
        "right",
        "3",
        "--primary",
        "1",
        "--host-intrinsic",
        "sqrt=left.Ops.root",
        "--host-intrinsic",
        "sqrt=right.Ops.root",
    ];
    for level in ["0", "3"] {
        let mut args = base.to_vec();
        args.extend(["--build", "run", "--output", "native", "--opt-level", level]);
        success(&sandbox.run(&args));
        assert_eq!(
            success(&run_command(&mut Command::new(sandbox.path("native")))),
            "(bool true)\n"
        );
        let mut args = base.to_vec();
        args.extend(["--test-native", "--opt-level", level]);
        assert!(success(&sandbox.run(&args)).contains("(count 1) (failures 0)"));
    }
    let mut args = base.to_vec();
    args.extend(["--emit-llvm", "run"]);
    let baseline = success(&sandbox.run(&args)).to_owned();
    assert!(baseline.contains("@llvm.sqrt.f32") && baseline.contains("@llvm.sqrt.f64"));
    // Identical repeated bindings and the legacy spelling are idempotent.
    args.extend([
        "--host-intrinsic",
        "sqrt=left.Ops.root",
        "--host-sqrt",
        "left.Ops.root",
    ]);
    assert_eq!(success(&sandbox.run(&args)), baseline);
    let old = fs::read(sandbox.path("native")).unwrap();
    let mut conflict = base.to_vec();
    conflict.extend([
        "--host-intrinsic",
        "copysign=left.Ops.root",
        "--build",
        "run",
        "--output",
        "native",
    ]);
    let output = sandbox.run(&conflict);
    failure(&output, 1);
    assert!(
        details(&output).contains("cannot bind both"),
        "{}",
        details(&output)
    );
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
}

#[test]
fn generic_host_intrinsic_cli_rejects_invalid_bindings_without_replacing_output() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "^intrinsic_enum enum Ops {tag} typealias F=.f32'builtin;
        fn run()->F {let x:F=4.0;unsafe {Ops.tag'intrinsic(x).as(F)}}",
    );
    let base = [
        "input.fl",
        "--build",
        "run",
        "--output",
        "native",
        "--host-intrinsic",
    ];
    let mut args = base.to_vec();
    args.push("sqrt=input.Ops.tag");
    success(&sandbox.run(&args));
    let old = fs::read(sandbox.path("native")).unwrap();
    for (binding, status, reason) in [
        ("sqrt", 2, "requires operation="),
        ("sqrt=", 2, "must not be empty"),
        (
            "mystery=input.Ops.tag",
            2,
            "unknown host intrinsic operation",
        ),
        ("sqrt=absent.Ops.tag", 1, "package is missing"),
        ("sqrt=input.Ops.missing", 1, "declared unit variant"),
        ("sqrt=input.Ops", 1, "selector requires"),
        ("copysign=input.Ops.tag", 1, "requires 2 operands"),
    ] {
        let mut args = base.to_vec();
        args.push(binding);
        let output = sandbox.run(&args);
        failure(&output, status);
        assert!(details(&output).contains(reason), "{}", details(&output));
        assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
    }
    for args in [
        vec![
            "input.fl",
            "--host-intrinsic",
            "sqrt=input.Ops.tag",
            "--emit",
            "nir",
        ],
        vec![
            "input.fl",
            "--test",
            "--host-intrinsic",
            "sqrt=input.Ops.tag",
        ],
        vec![
            "input.fl",
            "--build",
            "run",
            "--output",
            "native",
            "--host-intrinsic",
        ],
    ] {
        failure(&sandbox.run(&args), 2);
    }
    let mut args = base.to_vec();
    args.extend(["sqrt=input.Ops.tag", "--host-fma", "input.Ops.tag"]);
    let output = sandbox.run(&args);
    failure(&output, 1);
    assert!(details(&output).contains("cannot bind both"));
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), old);
}

#[test]
fn sysroot_repository_resource_defaults_execute_cleanup_and_reject_double_move() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub use resources.Drop; pub mod prelude {}",
    );
    sandbox.write("library/std/lib.fl", "pub mod prelude {}");
    sandbox.write(
        "library/builtin/resources.fl",
        include_str!("../../../library/builtin/resources.fl"),
    );
    // Isolate the real resource module and manifest bindings from unfinished
    // String/panic modules. This is not the full-library smoke test.
    let config = include_str!("../../../library/language-items.conf")
        .lines()
        .filter(|line| {
            line.starts_with("version")
                || line.starts_with("copy-protocol")
                || line.starts_with("drop-protocol")
        })
        .collect::<Vec<_>>()
        .join("\n");
    sandbox.write("library/language-items.conf", config);
    let source = "use builtin.Drop;
        struct Watch {seen:*bool}
        impl Drop for Watch {fn drop(*self) {assumes not self.seen.*.*;self.seen.*.* = true;}}
        fn run()->bool {var seen=false;do {let watch=Watch {seen:seen.ref};};seen}";
    sandbox.write("input.fl", source);
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    for level in ["0", "3"] {
        success(&sandbox.run(&[
            "input.fl",
            "--sysroot",
            "library",
            "--build",
            "run",
            "--opt-level",
            level,
            "--output",
            "program",
        ]));
        let output = Command::new(sandbox.path("program")).output().unwrap();
        assert_eq!(success(&output).trim(), "(bool true)");
    }
    sandbox.write(
        "input.fl",
        format!("{source}\ntest cleanup {{let result=run();assumes result;}}"),
    );
    for action in ["--test", "--test-native"] {
        success(&sandbox.run(&["input.fl", "--sysroot", "library", action]));
    }
    for action in [
        vec!["--interpret", "run"],
        vec!["--test"],
        vec!["--test-native"],
    ] {
        let mut args = vec!["input.fl", "--sysroot", "library", "--inline-functions"];
        args.extend(action);
        success(&sandbox.run(&args));
    }
    success(&sandbox.run(&[
        "input.fl",
        "--sysroot",
        "library",
        "--build",
        "run",
        "--inline-functions",
        "--discard-result",
        "--output",
        "program",
    ]));
    let output = Command::new(sandbox.path("program")).output().unwrap();
    assert_eq!(success(&output).trim(), "(unit)");
    sandbox.write(
        "input.fl",
        source.replace(
            "let watch=Watch {seen:seen.ref};",
            "let watch=Watch {seen:seen.ref};let moved=watch;let again=watch;",
        ),
    );
    sandbox.write("program", "previous");
    let rejected = sandbox.run(&[
        "input.fl",
        "--sysroot",
        "library",
        "--build",
        "run",
        "--output",
        "program",
    ]);
    failure(&rejected, 1);
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        diagnostic.contains("unavailable") || diagnostic.contains("moved"),
        "{diagnostic}"
    );
    assert_eq!(fs::read(sandbox.path("program")).unwrap(), b"previous");
}

#[test]
fn sysroot_incomplete_resource_defaults_never_fall_back_to_unchecked_execution() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.path("library/builtin")).unwrap();
    fs::create_dir_all(sandbox.path("library/std")).unwrap();
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod prelude {} pub trait Copy {} pub trait Drop {fn drop(*self);}",
    );
    sandbox.write("library/std/lib.fl", "pub mod prelude {}");
    sandbox.write(
        "input.fl",
        "fn run()->bool {true} test simple {let value=run();assumes value;}",
    );
    for binding in [
        "copy-protocol = builtin.Copy",
        "drop-protocol = builtin.Drop",
        "",
    ] {
        sandbox.write(
            "library/language-items.conf",
            format!("version = 1\n{binding}\n"),
        );
        for action in [
            vec!["--interpret", "run"],
            vec!["--emit-llvm", "run"],
            vec!["--test"],
            vec!["--test-native"],
        ] {
            for inline in [false, true] {
                // With neither binding, ordinary unchecked developer execution
                // stays available; explicitly requesting inlining requires both.
                if binding.is_empty() && !inline {
                    continue;
                }
                let mut args = vec!["input.fl", "--sysroot", "library", "--output", "previous"];
                args.extend(action.iter().copied());
                if inline {
                    args.push("--inline-functions");
                }
                sandbox.write("previous", "keep");
                let output = sandbox.run(&args);
                failure(&output, 1);
                let diagnostic = String::from_utf8_lossy(&output.stderr);
                assert!(
                    diagnostic.contains("Copy") || diagnostic.contains("Drop"),
                    "{diagnostic}"
                );
                assert_eq!(fs::read(sandbox.path("previous")).unwrap(), b"keep");
            }
        }
        sandbox.write("previous", "keep");
        let output = sandbox.run(&[
            "input.fl",
            "--sysroot",
            "library",
            "--build",
            "run",
            "--discard-result",
            "--output",
            "previous",
        ]);
        failure(&output, 1);
        assert_eq!(fs::read(sandbox.path("previous")).unwrap(), b"keep");
    }
}

#[test]
fn sysroot_host_intrinsic_defaults_build_real_math_and_allow_explicit_override() {
    let sandbox = Sandbox::new();
    for name in ["builtin", "std"] {
        fs::create_dir_all(sandbox.path(format!("library/{name}"))).unwrap();
    }
    sandbox.write(
        "library/builtin/lib.fl",
        "pub mod math {pub use comparison.{Eq,Ord,Order};pub use math_intrinsics.*;} pub mod prelude {pub use math;pub use primitives.*;pub use math.{Eq,Ord,Order};}",
    );
    sandbox.write(
        "library/builtin/resources.fl",
        include_str!("../../../library/builtin/resources.fl"),
    );
    sandbox.write(
        "library/builtin/primitives.fl",
        include_str!("../../../library/builtin/primitives.fl"),
    );
    fs::create_dir_all(sandbox.path("library/attrs")).unwrap();
    sandbox.write(
        "library/attrs/lib.fl",
        include_str!("../../../library/attrs/lib.fl"),
    );
    sandbox.write(
        "library/builtin/comparison.fl",
        include_str!("../../../library/builtin/comparison.fl"),
    );
    sandbox.write("library/std/lib.fl", "pub mod prelude {}");
    sandbox.write(
        "library/builtin/math_intrinsics.fl",
        include_str!("../../../library/builtin/math_intrinsics.fl"),
    );
    let config = include_str!("../../../library/language-items.conf")
        .lines()
        .filter(|line| {
            line.starts_with("version")
                || line.starts_with("host-intrinsic.")
                || line.starts_with("copy-protocol")
                || line.starts_with("drop-protocol")
                || line.starts_with("eq-protocol")
                || line.starts_with("ord-protocol")
        })
        .collect::<Vec<_>>()
        .join("\n");
    sandbox.write("library/language-items.conf", &config);
    sandbox.write(
        "input.fl",
        "fn run()->bool {
        math.fma<f64>(3.0,4.0,5.0) == 17.0
        and math.sqrt<f64>(4.0) == 2.0
        and math.wrapping_abs<i8>(-5) == 5
    }",
    );
    for level in ["0", "3"] {
        success(&sandbox.run(&[
            "input.fl",
            "--fuel",
            "1000000",
            "--sysroot",
            "library",
            "--build",
            "run",
            "--opt-level",
            level,
            "--output",
            "native",
        ]));
        let output = Command::new(sandbox.path("native")).output().unwrap();
        assert_eq!(success(&output).trim(), "(bool true)");
    }
    let bad = config.replace(
        "host-intrinsic.sqrt = builtin.math_intrinsics.target.Scalar.sqrt",
        "host-intrinsic.sqrt = missing.Ops.sqrt",
    );
    sandbox.write("library/language-items.conf", &bad);
    let previous = fs::read(sandbox.path("native")).unwrap();
    let output = sandbox.run(&[
        "input.fl",
        "--fuel",
        "1000000",
        "--sysroot",
        "library",
        "--build",
        "run",
        "--output",
        "native",
    ]);
    failure(&output, 1);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("host intrinsic package is missing or ambiguous")
    );
    assert_eq!(fs::read(sandbox.path("native")).unwrap(), previous);
    for override_args in [
        ["--host-sqrt", "builtin.math_intrinsics.target.Scalar.sqrt"],
        [
            "--host-intrinsic",
            "sqrt=builtin.math_intrinsics.target.Scalar.sqrt",
        ],
    ] {
        let mut args = vec![
            "input.fl",
            "--fuel",
            "1000000",
            "--sysroot",
            "library",
            "--build",
            "run",
            "--output",
            "native",
        ];
        args.extend(override_args);
        success(&sandbox.run(&args));
        assert_eq!(
            success(&Command::new(sandbox.path("native")).output().unwrap()).trim(),
            "(bool true)"
        );
    }
    sandbox.write("library/language-items.conf", &config);
    for source in [
        "use builtin.math as explicit_math; use math.sqrt as square_root; fn run()->bool { explicit_math.sqrt<f64>(4.0) == square_root<f64>(4.0) }",
        "mod math { fn marker()->bool {true} } use builtin.math as explicit_math; fn run()->bool { math.marker() and explicit_math.sqrt<f64>(4.0) == 2.0 }",
    ] {
        sandbox.write("input.fl", source);
        success(&sandbox.run(&[
            "input.fl",
            "--fuel",
            "1000000",
            "--sysroot",
            "library",
            "--build",
            "run",
            "--output",
            "native",
        ]));
        assert_eq!(
            success(&Command::new(sandbox.path("native")).output().unwrap()).trim(),
            "(bool true)"
        );
    }
    // All three public paths must retain the identity bound by the manifest.
    // The implementation uses math, the call uses a prelude trait, and a second
    // alias imports that same trait explicitly: no duplicate candidate is valid.
    sandbox.write(
        "input.fl",
        "use builtin.math.Eq as Same;
        struct Flag {value:bool}
        impl math.Eq for Flag {fn eq(self,other:Self)->bool {self.value==other.value}}
        impl math.Ord for Flag {fn cmp(self,other:Self)->math.Order {
            if self.value==other.value {.equal} else if self.value {.greater} else {.less}
        }}
        fn run()->bool {
            Flag {value:false}.as(Eq).eq(Flag {value:false})
            and Flag {value:true}.as(Same).eq(Flag {value:true})
            and Flag {value:false} < Flag {value:true}
        }",
    );
    assert_eq!(
        success(&sandbox.run(&["input.fl", "--sysroot", "library", "--interpret", "run"])).trim(),
        "(bool true)"
    );
    // Exporting the module does not introduce all mathematical function names.
    sandbox.write("input.fl", "fn run()->f64 {sqrt<f64>(4.0)}");
    failure(
        &sandbox.run(&["input.fl", "--sysroot", "library", "--emit", "nir"]),
        1,
    );
}

#[test]
fn object_type_is_available_in_comptime_signatures_but_not_runtime_values() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "typealias Metadata = .Object'builtin; comptime fn forward(value:Metadata)->Metadata {value}");
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    assert!(success(&output).contains("Object"));
    for source in [
        "typealias Metadata = .Object'builtin; fn forward(value:Metadata)->Metadata {value}",
        "typealias Metadata = .Object'builtin; struct Holder {value:Metadata} fn forward(value:Holder)->Holder {value}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        failure(&output, 1);
        assert!(!details(&output).contains("panicked"));
    }
}

#[test]
fn comptime_object_expressions_evaluate_mixed_nested_entries_in_order() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", r#"
        typealias Metadata = .Object'builtin;
        comptime fn flip(flag:*bool)->bool {flag.* = not flag.*;flag.*}
        comptime fn forward(value:Metadata)->Metadata {value}
        comptime fn build()->Metadata {
            var flag = false;
            forward({first:flip(flag.ref),flip(flag.ref),nested:{value:flip(flag.ref)},first:flip(flag.ref)})
        }
    "#);
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "build"]);
    let dump = success(&output);
    assert!(dump.trim().starts_with("(object "), "{dump}");
    assert_eq!(dump.matches("(key \"first\")").count(), 2, "{dump}");
    assert_eq!(dump.matches("(key \"nested\")").count(), 1, "{dump}");
    let values: Vec<_> = dump
        .match_indices("(bool ")
        .map(|(i, _)| dump[i..].split(')').next().unwrap())
        .collect();
    assert_eq!(
        values,
        vec!["(bool true", "(bool false", "(bool true", "(bool false"],
        "{dump}"
    );
}

#[test]
fn object_entries_propagate_returns_without_evaluating_later_entries() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        typealias Metadata = .Object'builtin;
        comptime fn forever()->bool {forever()}
        comptime fn build()->Metadata {
            let result = {first:do {return {};},late:forever()};
            result
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "build"]);
    assert_eq!(success(&output).trim(), "(object)");
}

#[test]
fn object_expressions_do_not_hide_invalid_names_or_runtime_calls() {
    let sandbox = Sandbox::new();
    for source in [
        "comptime fn build()->.Object'builtin {let result={first:missing()};result}",
        "fn runtime()->bool {true} comptime fn build()->.Object'builtin {let result={first:runtime()};result}",
        "fn run()->.Object'builtin {let result={first:true};result}",
    ] {
        sandbox.write("input.fl", source);
        let output = sandbox.run(&["input.fl", "--emit", "nir"]);
        failure(&output, 1);
        assert!(!details(&output).contains("parsing failed"));
        assert!(!details(&output).contains("panicked"));
    }
}

#[test]
fn pure_comptime_objects_preserve_type_values_and_nested_data() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "pure comptime fn build()->.Object'builtin {let result={ty:bool,nested:{flag:true}};result}",
    );
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "build"]);
    let dump = success(&output);
    assert!(dump.contains("(type (primitive Type))"), "{dump}");
    assert!(dump.contains("(bool true)"), "{dump}");
    assert!(dump.contains("(key \"nested\")"), "{dump}");
}

#[test]
fn object_constants_materialize_nested_typed_entries_without_merging() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        typealias Metadata = .Object'builtin;
        pure comptime fn Identity<T> -> .Type'builtin {T}
        comptime fn make()->Metadata {
            let result = {flag:true, false, nested:{ty:Identity<bool>}, flag:false};
            result
        }
        const META:Metadata = make();
        comptime fn build()->Metadata {
            const LOCAL = META;
            LOCAL
        }
    "#,
    );
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "build"]);
    let dump = success(&output);
    assert_eq!(dump.matches("(key \"flag\")").count(), 2, "{dump}");
    assert!(dump.contains("(key \"nested\")"), "{dump}");
    assert!(dump.contains("(type (primitive Type))"), "{dump}");
    assert!(dump.contains("normal-form"), "{dump}");
    assert_eq!(dump.matches("(bool false)").count(), 2, "{dump}");
}

#[test]
fn object_constants_do_not_hide_pointer_escape() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        const BAD = do {
            let flag = true;
            let result = {nested:{pointer:flag.ref}};
            result
        };
        comptime fn build()->.Object'builtin {BAD}
    "#,
    );
    let output = sandbox.run(&["input.fl", "--invoke-comptime", "build"]);
    failure(&output, 1);
    let diagnostic = details(&output);
    assert!(
        diagnostic.contains("storage pointer cannot be materialized")
            || diagnostic.contains("ExpiredPointer"),
        "{diagnostic}"
    );
}

#[test]
fn object_constants_cannot_transport_metadata_into_runtime() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        const META = {flag:true};
        fn run()->.Object'builtin {META}
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "nir"]);
    failure(&output, 1);
    assert!(!details(&output).contains("parsing failed"));
    assert!(!details(&output).contains("panicked"));
}

#[test]
fn source_attribute_inspection_evaluates_calls_in_application_order() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        comptime fn metadata(flag:bool)->.Object'builtin {
            let result = {mode:flag}; result
        }
        ^metadata(true) ^metadata(false)
        struct Decorated { .mode false }
        fn run() {}
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "attributes"]);
    let dump = success(&output);
    assert_eq!(dump.matches("(evaluated-attribute ").count(), 3, "{dump}");
    assert!(dump.contains("(configuration \"mode\")"), "{dump}");
    assert_eq!(dump.matches("(bool false)").count(), 2, "{dump}");
    assert_eq!(dump.matches("(bool true)").count(), 1, "{dump}");
    assert!(
        dump.rfind("(bool false)").unwrap() < dump.find("(bool true)").unwrap(),
        "{dump}"
    );
    failure(&sandbox.run(&["input.fl", "--emit", "nir"]), 1);
}

#[test]
fn source_attribute_inspection_keeps_lexical_constant_bindings() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        const FLAG=false;
        comptime fn metadata(flag:bool)->.Object'builtin {
            let result = {mode:flag}; result
        }
        mod nested {
            const FLAG=true;
            ^metadata(FLAG) fn inner() {}
        }
        ^metadata(FLAG) fn outer() {}
    "#,
    );
    let output = sandbox.run(&["input.fl", "--emit", "attributes"]);
    let dump = success(&output);
    assert_eq!(dump.matches("(evaluated-attribute ").count(), 2, "{dump}");
    assert_eq!(dump.matches("(bool true)").count(), 1, "{dump}");
    assert_eq!(dump.matches("(bool false)").count(), 1, "{dump}");
}

#[test]
fn failed_attribute_inspection_preserves_existing_output() {
    let sandbox = Sandbox::new();
    for source in [
        "^true fn run() {}",
        "^missing() fn run() {}",
        "fn runtime()->bool {true} ^runtime() fn run() {}",
    ] {
        sandbox.write("input.fl", source);
        fs::write(sandbox.path("result.lisp"), OLD_OUTPUT).unwrap();
        let output = sandbox.run(&[
            "input.fl",
            "--emit",
            "attributes",
            "--output",
            "result.lisp",
        ]);
        failure(&output, 1);
        assert!(
            !details(&output).contains("parsing failed"),
            "{}",
            details(&output)
        );
        assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
    }
}

#[test]
fn package_attribute_inspection_resolves_provider_functions() {
    let sandbox = Sandbox::new();
    sandbox.write("client.fl", "use dep.metadata; ^metadata(true) fn run() {}");
    sandbox.write(
        "provider.fl",
        "pub comptime fn metadata(flag:bool)->.Object'builtin {let result={enabled:flag};result}",
    );
    let output = sandbox.run(&[
        "--packages-v1",
        "--package",
        "7",
        "701",
        "client",
        "client.fl",
        "--package",
        "9",
        "901",
        "provider",
        "provider.fl",
        "--primary",
        "7",
        "--dependency",
        "7",
        "dep",
        "9",
        "--emit",
        "attributes",
        "--with-inputs",
    ]);
    let dump = success(&output);
    assert_eq!(dump.matches("(evaluated-attribute ").count(), 1, "{dump}");
    assert!(dump.contains("(property \"enabled\")"), "{dump}");
    assert!(dump.contains("(bool true)"), "{dump}");
    assert!(
        dump.contains("(source (resolution-id 7) (identity 701)"),
        "{dump}"
    );
    assert!(
        dump.contains("(source (resolution-id 9) (identity 901)"),
        "{dump}"
    );
}

#[test]
fn source_attribute_merge_selects_outer_prefix_and_retains_origin() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        r#"
        comptime fn metadata(flag:bool)->.Object'builtin {
            let result = {mode:flag}; result
        }
        ^metadata(true) ^metadata(false)
        struct Decorated { .mode false }
    "#,
    );
    let luna::cli::Command::Compile(options) = luna::cli::parse([
        sandbox.path("input.fl").into_os_string(),
        "--emit".into(),
        "attributes".into(),
    ])
    .unwrap() else {
        panic!()
    };
    let luna::frontend::Artifact::Attributes(attributes) =
        luna::frontend::compile(&options).unwrap()
    else {
        panic!()
    };
    let merged = elaboration::program::attributes::merge(
        &attributes,
        100,
        &mut comptime_runtime::Budget::default(),
    )
    .unwrap();
    assert_eq!(merged.len(), 1);
    let selected = merged[0].get("mode").unwrap();
    assert!(matches!(
        selected.value.kind,
        nir::program::ExprKind::Literal(nir::program::Literal::Bool(true))
    ));
    assert!(std::ptr::eq(selected.origin, attributes.last().unwrap()));
    assert_eq!(selected.entry, Some(0));
    assert_eq!(
        selected.value.ty,
        nir::Term::Primitive(nir::Primitive::Bool)
    );
}

#[test]
fn hir_retains_source_identity_frontier_including_clause_parameters() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "struct Many where A, B, C, D { first: A }");
    let output = sandbox.run(&["input.fl", "--emit", "hir"]);
    let dump = hir_dump(&output);
    let frontier: usize = dump
        .split("(next-definition ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let owners = dump.matches("(owner (id ").count();
    assert!(
        frontier >= owners + 4,
        "source clause IDs must remain reserved: {dump}"
    );
    let again = sandbox.run(&["input.fl", "--emit", "hir"]);
    assert_eq!(dump, hir_dump(&again));
}

#[test]
fn text_region_cli_reports_source_dependencies_and_preserves_failed_output() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "typealias str=.str'builtin;
        fn invoke(callback:%{value:str} fn()->str)->str {callback()}
        fn run(value:str)->str {let observed=value == \"x\";invoke(||value)}
        fn generic(value:T)->T where T {value}
        comptime fn compile_time()->bool {true}
        fn unknown(callback:fn(str)->str,value:str)->str {callback(value)}",
    );
    let output = sandbox.run(&["input.fl", "--analyze-text-regions", "run"]);
    let dump = success(&output);
    single_sexpression(dump);
    assert!(dump.starts_with("(text-region-summary"), "{dump}");
    assert!(
        dump.contains("(reads (static-text-region) (parameter-text-region 0 (fields)))"),
        "{dump}"
    );
    assert!(
        dump.contains("(returned (view (fields) (origins (parameter-text-region 0 (fields)))))"),
        "{dump}"
    );
    for (entry, message) in [
        ("missing", "was not found"),
        ("generic", "nongeneric wrapper"),
        ("compile_time", "ordinary runtime"),
        ("unknown", "stable local callable"),
    ] {
        sandbox.write("result.lisp", OLD_OUTPUT);
        let output = sandbox.run(&[
            "input.fl",
            "--analyze-text-regions",
            entry,
            "--output",
            "result.lisp",
        ]);
        failure(&output, 1);
        assert!(details(&output).contains(message), "{}", details(&output));
        assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
    }
    let output = sandbox.run(&["input.fl", "--analyze-text-regions", "run", "--fuel", "1"]);
    failure(&output, 1);
    assert!(
        details(&output).contains("budget exhausted"),
        "{}",
        details(&output)
    );
    let output = sandbox.run(&[
        "input.fl",
        "--analyze-text-regions",
        "run",
        "--output",
        "result.lisp",
    ]);
    assert!(success(&output).is_empty());
    assert_eq!(
        fs::read_to_string(sandbox.path("result.lisp"))
            .unwrap()
            .trim(),
        dump.trim()
    );
}

#[test]
fn text_region_cli_rejects_conflicting_actions_and_permissions() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", VALID);
    for extra in [
        vec!["--emit", "nir"],
        vec!["--interpret", "requested_entry"],
        vec!["--test"],
        vec!["--copy-protocol", "Copy"],
        vec!["--drop-protocol", "Drop"],
        vec!["--analyze-text-regions", "requested_entry"],
        vec!["--fuel", "0"],
    ] {
        let mut arguments = vec!["input.fl", "--analyze-text-regions", "requested_entry"];
        arguments.extend(extra);
        failure(&sandbox.run(&arguments), 2);
    }
}

#[test]
fn text_region_cli_jointly_expands_inline_effects_with_one_budget() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "input.fl",
        "typealias str=.str'builtin;typealias Unit=.void'builtin;
        inline effect deliver(value:str);
        inline fn produce(value:str)->#deliver Unit {deliver(value)#;}
        fn run(value:str)->str {
            produce(value)# {deliver(text)=>{let byte=.str_byte'builtin(text,0);return text;},};
            \"unused\"
        }",
    );
    let arguments = [
        "input.fl",
        "--analyze-text-regions",
        "run",
        "--inline-functions",
    ];
    let output = sandbox.run(&arguments);
    let dump = success(&output);
    single_sexpression(dump);
    assert!(
        dump.contains("(reads (parameter-text-region 0 (fields)))"),
        "{dump}"
    );
    assert!(
        dump.contains("(returned (view (fields) (origins (parameter-text-region 0 (fields)))))"),
        "{dump}"
    );
    sandbox.write("result.lisp", OLD_OUTPUT);
    let mut limited = arguments.to_vec();
    limited.extend(["--fuel", "1", "--output", "result.lisp"]);
    failure(&sandbox.run(&limited), 1);
    assert_eq!(fs::read(sandbox.path("result.lisp")).unwrap(), OLD_OUTPUT);
    assert_eq!(success(&sandbox.run(&arguments)), dump);
}

#[test]
fn text_region_cli_selects_primary_package_and_tracks_provider_calls() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "client.fl",
        "use dep.echo; fn run(value:.str'builtin)->.str'builtin {echo(value)}",
    );
    sandbox.write(
        "provider.fl",
        "pub inline fn echo(value:.str'builtin)->.str'builtin {value} fn run()->.str'builtin {\"wrong\"}",
    );
    let output = sandbox.run(&[
        "--packages-v1",
        "--package",
        "7",
        "701",
        "client",
        "client.fl",
        "--package",
        "9",
        "901",
        "provider",
        "provider.fl",
        "--primary",
        "7",
        "--dependency",
        "7",
        "dep",
        "9",
        "--analyze-text-regions",
        "run",
        "--inline-functions",
        "--with-inputs",
    ]);
    let dump = success(&output);
    single_sexpression(dump);
    assert!(dump.contains("(text-region-summary"), "{dump}");
    assert!(
        dump.contains("(parameter-text-region 0 (fields))"),
        "{dump}"
    );
    assert!(!dump.contains("(static-text-region)"), "{dump}");
    assert!(
        dump.contains("(source (resolution-id 9) (identity 901)"),
        "{dump}"
    );
}

#[test]
fn sysroot_meta_math_invokes_vm_and_folds_into_runtime_code() {
    let sandbox = Sandbox::new();
    sandbox.write("input.fl", "pure comptime fn root()->f64 {meta.math.sqrt<f64>(81.0)}\nfn run()->bool {inline if root() == 9.0 {true} else {false}}");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let root = root.to_str().unwrap();
    let output = sandbox.run(&[
        "input.fl",
        "--sysroot",
        root,
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--invoke-comptime",
        "root",
    ]);
    success(&output);
    let output = sandbox.run(&[
        "input.fl",
        "--sysroot",
        root,
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--emit-llvm",
        "run",
    ]);
    success(&output);
    let ir = String::from_utf8(output.stdout).unwrap();
    assert!(!ir.contains("call double @llvm.sqrt"), "{ir}");
}
