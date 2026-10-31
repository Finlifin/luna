use std::{
    ffi::OsString,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptimizationLevel {
    #[default]
    O0,
    O1,
    O2,
    O3,
}
impl OptimizationLevel {
    fn name(self) -> &'static str {
        match self {
            Self::O0 => "O0",
            Self::O1 => "O1",
            Self::O2 => "O2",
            Self::O3 => "O3",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Toolchain {
    pub llvm_config: PathBuf,
    pub linker: PathBuf,
    pub optimization: OptimizationLevel,
}

/// Layout reported by the selected LLVM toolchain, not by the compiler process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetLayout {
    pub triple: String,
    pub data_layout: String,
    /// Storage width of an ordinary address-space-zero pointer, in bits.
    pub pointer_bits: u32,
}
impl TargetLayout {
    /// Checked semantic facts for front-end invocation configuration. Do not
    /// infer them from the compiler process or change primitive type identity.
    pub fn semantic_target(&self) -> Result<nir::target::TargetData, String> {
        nir::target::TargetData::new(self.pointer_bits).map_err(str::to_owned)
    }
}
impl std::fmt::Display for TargetLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(host-target (triple {:?}) (data-layout {:?}) (pointer-bits {}))",
            self.triple, self.data_layout, self.pointer_bits
        )
    }
}

// Only consume LLVM's canonical printed headers, never parse user LLVM syntax.
fn module_header<'a>(module: &'a str, name: &str) -> Option<&'a str> {
    module.lines().find_map(|line| {
        line.strip_prefix(name)?
            .strip_prefix(" = \"")?
            .strip_suffix('"')
    })
}

fn check_semantic_target(module: &str, pointer_bits: u32) -> Result<(), String> {
    let Some(reference) = module
        .lines()
        .find_map(|line| line.strip_prefix("!flurry.target = "))
    else {
        return Ok(());
    };
    let malformed = || "malformed Flurry semantic target metadata".to_owned();
    let id = reference
        .strip_prefix("!{!")
        .and_then(|s| s.strip_suffix('}'))
        .ok_or_else(malformed)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed());
    }
    let prefix = format!("!{id} = !{{i32 ");
    let width = module
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|s| s.strip_suffix('}'))
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(malformed)?;
    if width != pointer_bits {
        return Err(format!(
            "host semantic pointer width mismatch: input {width}, toolchain {pointer_bits}"
        ));
    }
    Ok(())
}
impl Default for Toolchain {
    fn default() -> Self {
        Self {
            optimization: OptimizationLevel::O0,
            llvm_config: std::env::var_os("LUNA_LLVM_CONFIG")
                .unwrap_or_else(|| "llvm-config-21".into())
                .into(),
            linker: std::env::var_os("LUNA_CC")
                .unwrap_or_else(|| "cc".into())
                .into(),
        }
    }
}
fn run(program: &std::path::Path, args: &[OsString]) -> Result<Vec<u8>, String> {
    run_with_timeout(program, args, std::time::Duration::from_secs(120))
}

fn run_with_timeout(
    program: &std::path::Path,
    args: &[OsString],
    timeout: std::time::Duration,
) -> Result<Vec<u8>, String> {
    run_with_limits(program, args, timeout, 64 * 1024 * 1024)
}

fn run_with_limits(
    program: &std::path::Path,
    args: &[OsString],
    timeout: std::time::Duration,
    stdout_limit: usize,
) -> Result<Vec<u8>, String> {
    if timeout.is_zero() {
        return Err("host tool timeout must be positive".into());
    }
    // opt emits complete LLVM modules on stdout. Reject oversized output rather
    // than handing a truncated module or target description to the next stage.
    let (mut stdout, out) = Capture::with_limit(stdout_limit)?;
    let (mut stderr, err) = Capture::new()?;
    let mut child = spawn_with_timeout(
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err),
        std::time::Duration::from_secs(1).min(timeout),
        &program.display().to_string(),
    )?;
    let start = std::time::Instant::now();
    let outcome = loop {
        if let Err(error) = stdout.drain().and_then(|()| stderr.drain()) {
            break Err(error);
        }
        if stdout.truncated {
            break Err(format!("host tool stdout exceeds {stdout_limit} bytes"));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // A descendant may still hold an output descriptor after the
                // direct child exits. Require EOF within the same deadline.
                let drained = collect_tool_tail(&mut stdout, &mut stderr, start, timeout);
                break drained.and_then(|()| {
                    if stdout.truncated {
                        Err(format!("host tool stdout exceeds {stdout_limit} bytes"))
                    } else if status.success() {
                        Ok(())
                    } else {
                        Err(format!("{} failed ({status})", program.display()))
                    }
                });
            }
            Ok(None) => {}
            Err(error) => break Err(format!("cannot wait for {}: {error}", program.display())),
        }
        if start.elapsed() >= timeout {
            break Err(format!("{} timed out after {timeout:?}", program.display()));
        }
        std::thread::sleep(
            std::time::Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
        );
    };
    if outcome.is_err() {
        stop_child_group(&mut child);
        let _ = stdout.drain();
        let _ = stderr.drain();
    }
    outcome.map_err(|mut error| {
        stdout.append("stdout", &mut error);
        stderr.append("stderr", &mut error);
        error
    })?;
    Ok(stdout.bytes)
}
// A successful direct child does not end the deadline: inherited output
// descriptors may still be written by descendants. Never turn a partial module
// collected at the deadline into successful compiler output.
fn collect_tool_tail(
    stdout: &mut Capture,
    stderr: &mut Capture,
    start: std::time::Instant,
    timeout: std::time::Duration,
) -> Result<(), String> {
    collect_output_tail(stdout, stderr, start, timeout, true)
}

fn collect_output_tail(
    stdout: &mut Capture,
    stderr: &mut Capture,
    start: std::time::Instant,
    timeout: std::time::Duration,
    reject_stdout_overflow: bool,
) -> Result<(), String> {
    loop {
        if start.elapsed() >= timeout {
            return Err(format!(
                "host tool output collection timed out after {timeout:?}"
            ));
        }
        stdout.drain()?;
        stderr.drain()?;
        if (reject_stdout_overflow && stdout.truncated) || (stdout.eof && stderr.eof) {
            return Ok(());
        }
        std::thread::sleep(
            std::time::Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
        );
    }
}

// A concurrent fork can briefly retain a CLOEXEC write descriptor until exec,
// even after our writer has closed it. Retry only ETXTBSY, before any child has
// started; never retry an executed test or hide other launch failures.
fn spawn_native(command: &mut Command) -> Result<std::process::Child, String> {
    // Parallel LLVM/test processes can delay exec beyond 100ms. Bound the
    // launch phase separately from execution, which starts after spawn succeeds.
    spawn_native_with_timeout(command, std::time::Duration::from_secs(1))
}
fn spawn_native_with_timeout(
    command: &mut Command,
    timeout: std::time::Duration,
) -> Result<std::process::Child, String> {
    spawn_with_timeout(command, timeout, "native test")
}
fn spawn_with_timeout(
    command: &mut Command,
    timeout: std::time::Duration,
    description: &str,
) -> Result<std::process::Child, String> {
    use std::os::unix::process::CommandExt;
    // The child leads a private group before exec. Error cleanup must reach
    // inherited writers even if the direct child has already exited.
    command.process_group(0);
    let start = std::time::Instant::now();
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error) if error.raw_os_error() == Some(26) && start.elapsed() < timeout => {
                std::thread::sleep(
                    std::time::Duration::from_millis(2)
                        .min(timeout.saturating_sub(start.elapsed())),
                );
            }
            Err(error) => return Err(format!("cannot start {description}: {error}")),
        }
    }
}
/// Only accepts children created by spawn_with_timeout, which establishes
/// their private process group. Descendants that deliberately leave the group
/// are outside this bounded runner's process ownership model.
fn stop_child_group(child: &mut std::process::Child) {
    unsafe extern "C" {
        fn kill(pid: std::os::raw::c_int, signal: std::os::raw::c_int) -> std::os::raw::c_int;
    }
    if let Ok(pid) = i32::try_from(child.id()) {
        if pid > 0 {
            // SAFETY: kill takes integer values, not pointers. The negative
            // nonzero PID identifies only the group created for this child;
            // The Unix host runner uses SIGKILL (9).
            unsafe { kill(-pid, 9) };
        }
    }
    // Reap the direct child, also retaining a PID-local fallback if group
    // signaling failed. try_wait may already have reaped it during collection.
    let _ = child.kill();
    let _ = child.wait();
}

// Poll nonblocking readers in the deadline loop: no reader thread can remain
// blocked on an inherited descriptor after the test process has exited.
struct Capture {
    reader: std::os::unix::net::UnixStream,
    bytes: Vec<u8>,
    truncated: bool,
    eof: bool,
    limit: usize,
}
impl Capture {
    const LIMIT: usize = 16 * 1024;

    fn new() -> Result<(Self, Stdio), String> {
        Self::with_limit(Self::LIMIT)
    }

    fn with_limit(limit: usize) -> Result<(Self, Stdio), String> {
        let (reader, writer) = std::os::unix::net::UnixStream::pair()
            .map_err(|e| format!("cannot capture subprocess output: {e}"))?;
        reader
            .set_nonblocking(true)
            .map_err(|e| format!("cannot prepare subprocess capture: {e}"))?;
        let fd: std::os::fd::OwnedFd = writer.into();
        Ok((
            Self {
                reader,
                bytes: Vec::new(),
                truncated: false,
                eof: false,
                limit,
            },
            Stdio::from(fd),
        ))
    }

    fn drain(&mut self) -> Result<(), String> {
        use std::io::Read;
        let mut buffer = [0; 4096];
        // Bound work even when the child writes continuously, so both streams
        // and the execution deadline get serviced.
        for _ in 0..64 {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => {
                    let keep = n.min(self.limit - self.bytes.len());
                    self.bytes.extend_from_slice(&buffer[..keep]);
                    self.truncated |= keep < n;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(format!("cannot read subprocess output: {e}")),
            }
        }
        Ok(())
    }

    fn append(&self, name: &str, failure: &mut String) {
        // Compiler stdout may hold a complete module (up to 64MiB). Diagnostics
        // retain only a bounded prefix, without reducing successful output.
        let shown = self.bytes.len().min(Self::LIMIT);
        if shown != 0 {
            // Debug escaping keeps terminal control bytes inert. TestReport
            // subsequently quotes this complete diagnostic as an S-expression.
            failure.push_str(&format!(
                "\n{name}: {:?}",
                String::from_utf8_lossy(&self.bytes[..shown])
            ));
        }
        if self.truncated || shown < self.bytes.len() {
            failure.push_str(&format!("\n{name}: [truncated after {shown} bytes]"));
        }
    }
}
struct NativeOutput {
    status: std::process::ExitStatus,
    stdout: Capture,
    stderr: Capture,
}
impl NativeOutput {
    fn diagnostic(&self, mut failure: String) -> String {
        self.stdout.append("stdout", &mut failure);
        self.stderr.append("stderr", &mut failure);
        failure
    }
}
impl Toolchain {
    /// Execute a fresh native test process. The deadline bounds execution,
    /// not compilation; each compiler tool has its own 120-second deadline.
    /// Callers may impose a shorter overall build deadline with the wrappers.
    pub fn execute_test(&self, ir: &str, timeout: std::time::Duration) -> Result<(), String> {
        let result = self.run_test_ir(ir, timeout)?;
        if result.status.success() {
            Ok(())
        } else {
            Err(result.diagnostic(format!("native test failed ({})", result.status)))
        }
    }

    fn run_test_ir(&self, ir: &str, timeout: std::time::Duration) -> Result<NativeOutput, String> {
        use std::os::unix::fs::PermissionsExt;
        if timeout.is_zero() {
            return Err("native test timeout must be positive".into());
        }
        let bytes = self.compile(ir)?;
        let work = Workspace::new()?;
        let path = work.0.join("test-program");
        std::fs::write(&path, bytes).map_err(|e| format!("cannot write native test: {e}"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot prepare native test: {e}"))?;
        let (mut stdout, out) = Capture::new()?;
        let (mut stderr, err) = Capture::new()?;
        let mut child = spawn_native(
            Command::new(path)
                .stdin(Stdio::null())
                .stdout(out)
                .stderr(err),
        )?;
        let start = std::time::Instant::now();
        let outcome = loop {
            if let Err(error) = stdout.drain().and_then(|()| stderr.drain()) {
                break Err(error);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Exact-output comparison needs the complete inherited
                    // streams, not only the direct process's exit status.
                    // Status-only execution permits bounded truncation, but
                    // neither mode may abandon writers before the deadline.
                    break collect_output_tail(&mut stdout, &mut stderr, start, timeout, false)
                        .map(|()| status);
                }
                Ok(None) => {}
                Err(error) => break Err(format!("cannot wait for native test: {error}")),
            }
            if start.elapsed() >= timeout {
                break Err(format!("native test timed out after {timeout:?}"));
            }
            std::thread::sleep(
                std::time::Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
            );
        };
        if outcome.is_err() {
            stop_child_group(&mut child);
        }
        let drained = stdout.drain().and_then(|()| stderr.drain());
        let status = outcome
            .and_then(|status| drained.map(|()| status))
            .map_err(|mut failure| {
                stdout.append("stdout", &mut failure);
                stderr.append("stderr", &mut failure);
                failure
            })?;
        Ok(NativeOutput {
            status,
            stdout,
            stderr,
        })
    }

    #[cfg(test)]
    pub(super) fn execute_test_ir(&self, ir: &str) -> Result<std::process::Output, String> {
        self.execute_test_ir_with_timeout(ir, std::time::Duration::from_secs(10))
    }

    #[cfg(test)]
    fn execute_test_ir_with_timeout(
        &self,
        ir: &str,
        timeout: std::time::Duration,
    ) -> Result<std::process::Output, String> {
        let result = self.run_test_ir(ir, timeout)?;
        // Output comparison must never mistake a retained prefix for complete
        // output. Unlike the status-only runner, reject truncation on success too.
        if result.stdout.truncated || result.stderr.truncated {
            return Err(result.diagnostic("native test captured output exceeds limit".into()));
        }
        Ok(std::process::Output {
            status: result.status,
            stdout: result.stdout.bytes,
            stderr: result.stderr.bytes,
        })
    }

    fn directory(&self) -> Result<PathBuf, String> {
        let output = run(&self.llvm_config, &["--bindir".into()])?;
        let path = std::str::from_utf8(&output)
            .map_err(|_| "LLVM bindir is not UTF-8")?
            .trim();
        // An empty path would turn `directory.join("opt")` into a PATH
        // lookup, silently selecting tools outside the configured toolchain.
        if path.is_empty() {
            return Err("LLVM bindir is empty".into());
        }
        let directory = std::fs::canonicalize(path)
            .map_err(|e| format!("cannot resolve LLVM bindir {path:?}: {e}"))?;
        if !directory.is_dir() {
            return Err(format!("LLVM bindir {path:?} is not a directory"));
        }
        Ok(directory)
    }
    pub fn target_layout(&self) -> Result<TargetLayout, String> {
        let directory = self.directory()?;
        self.target_layout_in(&directory)
    }

    fn target_layout_in(&self, directory: &std::path::Path) -> Result<TargetLayout, String> {
        let triple = run(&self.llvm_config, &["--host-target".into()])?;
        let triple = std::str::from_utf8(&triple)
            .map_err(|_| "LLVM host target is not UTF-8")?
            .trim();
        Self::probe_layout(directory, triple)
    }

    fn probe_layout(directory: &std::path::Path, triple: &str) -> Result<TargetLayout, String> {
        if triple.is_empty()
            || !triple
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            return Err("LLVM host target is empty or malformed".into());
        }
        let work = Workspace::new()?;
        let source = work.0.join("target.ll");
        // Fold sizeof(ptr) with LLVM's actual target layout. The default
        // address space need not have an explicit p: entry in a layout string.
        std::fs::write(&source,
            "define i64 @flurry_pointer_bits() {\n %p = getelementptr ptr, ptr null, i64 1\n %n = ptrtoint ptr %p to i64\n %bits = mul i64 %n, 8\n ret i64 %bits\n}\n")
            .map_err(|e| format!("cannot write target probe: {e}"))?;
        let printed = run(
            &directory.join("opt"),
            &[
                "-S".into(),
                "-passes=instcombine".into(),
                format!("-mtriple={triple}").into(),
                source.into_os_string(),
                "-o".into(),
                "-".into(),
            ],
        )?;
        let printed =
            std::str::from_utf8(&printed).map_err(|_| "LLVM target layout is not UTF-8")?;
        let pointer_bits = printed
            .lines()
            .find_map(|line| line.trim().strip_prefix("ret i64 "))
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|width| *width > 0 && *width % 8 == 0)
            .ok_or("LLVM did not resolve target pointer width")?;
        Ok(TargetLayout {
            pointer_bits,
            triple: module_header(printed, "target triple")
                .ok_or("LLVM target triple missing")?
                .into(),
            data_layout: module_header(printed, "target datalayout")
                .filter(|s| !s.is_empty())
                .ok_or("LLVM target data layout missing")?
                .into(),
        })
    }

    fn optimize(
        &self,
        directory: &std::path::Path,
        source: &std::path::Path,
        output: &std::path::Path,
        text: bool,
    ) -> Result<(), String> {
        let target = self.target_layout_in(directory)?;
        // Parse and print before overriding anything. LLVM also handles escaped
        // strings and bitcode here, so conflicting input cannot evade checking.
        let canonical = run(
            &directory.join("opt"),
            &[
                "-S".into(),
                "-passes=no-op-module".into(),
                source.as_os_str().to_owned(),
                "-o".into(),
                "-".into(),
            ],
        )?;
        let canonical = std::str::from_utf8(&canonical).map_err(|_| "LLVM module is not UTF-8")?;
        check_semantic_target(canonical, target.pointer_bits)?;
        for (header, expected) in [
            ("target triple", &target.triple),
            ("target datalayout", &target.data_layout),
        ] {
            if let Some(actual) = module_header(canonical, header).filter(|s| !s.is_empty()) {
                if actual != expected {
                    return Err(format!(
                        "host {header} mismatch: input {actual:?}, toolchain {expected:?}"
                    ));
                }
            }
        }
        let mut args = vec![
            format!("-mtriple={}", target.triple).into(),
            format!("-passes=default<{}>", self.optimization.name()).into(),
            "-verify-each".into(),
            source.as_os_str().to_owned(),
            "-o".into(),
            output.as_os_str().to_owned(),
        ];
        if text {
            args.push("-S".into());
        }
        run(&directory.join("opt"), &args)?;
        validate_artifact(output, "optimized LLVM")
    }
    pub fn optimized_ir(&self, ir: &str) -> Result<String, String> {
        let directory = self.directory()?;
        let work = Workspace::new()?;
        let source = work.0.join("module.ll");
        let output = work.0.join("optimized.ll");
        std::fs::write(&source, ir).map_err(|e| format!("cannot write LLVM module: {e}"))?;
        self.optimize(&directory, &source, &output, true)?;
        let text = std::fs::read_to_string(output)
            .map_err(|e| format!("cannot read optimized LLVM: {e}"))?;
        // LLVM's ModuleID comment names its temporary input path. It is not IR
        // semantics and must not make otherwise identical dumps nondeterministic.
        Ok(text
            .lines()
            .filter(|line| !line.starts_with("; ModuleID ="))
            .map(|line| format!("{line}\n"))
            .collect())
    }
    fn compile_bitcode_in(
        &self,
        directory: &std::path::Path,
        ir: &str,
        work: &Workspace,
    ) -> Result<PathBuf, String> {
        let source = work.0.join("module.ll");
        let bitcode = work.0.join("module.bc");
        std::fs::write(&source, ir).map_err(|e| format!("cannot write LLVM module: {e}"))?;
        run(
            &directory.join("llvm-as"),
            &[
                source.into_os_string(),
                "-o".into(),
                bitcode.clone().into_os_string(),
            ],
        )?;
        validate_artifact(&bitcode, "LLVM bitcode")?;
        let optimized = work.0.join("optimized.bc");
        self.optimize(directory, &bitcode, &optimized, false)?;
        Ok(optimized)
    }

    pub(super) fn bitcode(&self, ir: &str) -> Result<Vec<u8>, String> {
        let work = Workspace::new()?;
        let directory = self.directory()?;
        let path = self.compile_bitcode_in(&directory, ir, &work)?;
        std::fs::read(path).map_err(|e| format!("cannot read LLVM bitcode: {e}"))
    }

    fn compile_native_in(
        &self,
        ir: &str,
        work: &Workspace,
        assembly: bool,
    ) -> Result<PathBuf, String> {
        let directory = self.directory()?;
        let optimized = self.compile_bitcode_in(&directory, ir, work)?;
        let object = work.0.join(if assembly { "module.s" } else { "module.o" });
        run(
            &directory.join("llc"),
            &[
                if assembly {
                    "-filetype=asm"
                } else {
                    "-filetype=obj"
                }
                .into(),
                "-relocation-model=pic".into(),
                optimized.into_os_string(),
                format!("-{}", self.optimization.name()).into(),
                "-o".into(),
                object.clone().into_os_string(),
            ],
        )?;
        validate_artifact(
            &object,
            if assembly {
                "native assembly"
            } else {
                "native object"
            },
        )?;
        Ok(object)
    }

    /// Verified host assembly using the same target and optimization as objects.
    /// No linker is launched; assembly retains the supplied IR's entry ABI.
    pub fn assembly(&self, ir: &str) -> Result<String, String> {
        let work = Workspace::new()?;
        let path = self.compile_native_in(ir, &work, true)?;
        std::fs::read_to_string(path).map_err(|e| format!("cannot read native assembly: {e}"))
    }

    pub(super) fn object(&self, ir: &str) -> Result<Vec<u8>, String> {
        let work = Workspace::new()?;
        let object = self.compile_native_in(ir, &work, false)?;
        std::fs::read(object).map_err(|e| format!("cannot read native object: {e}"))
    }

    pub(super) fn compile(&self, ir: &str) -> Result<Vec<u8>, String> {
        let work = Workspace::new()?;
        let object = self.compile_native_in(ir, &work, false)?;
        let executable = work.0.join("program");
        run(
            &self.linker,
            &[
                object.into_os_string(),
                // LLVM may lower scalar math intrinsics to the host libm ABI.
                "-lm".into(),
                "-o".into(),
                executable.clone().into_os_string(),
            ],
        )?;
        validate_artifact(&executable, "linked executable")?;
        std::fs::read(executable).map_err(|e| format!("cannot read linked executable: {e}"))
    }
}
/// A successful process exit does not establish artifact delivery. Check before
/// another tool consumes the path or the caller reads/publishes its contents.
/// This is a delivery check, not object-format or executable-ABI validation.
fn validate_artifact(path: &std::path::Path, description: &str) -> Result<(), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("cannot inspect {description} artifact: {e}"))?;
    if !metadata.is_file() {
        return Err(format!("{description} artifact is not a regular file"));
    }
    if metadata.len() == 0 {
        return Err(format!("{description} artifact is empty"));
    }
    Ok(())
}

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "luna-host-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("cannot create host workspace: {e}")),
            }
        }
        Err("cannot reserve a host workspace".into())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn llvm_configuration_requires_a_real_tool_directory_and_recovers() {
        use std::os::unix::fs::PermissionsExt;
        let work = Workspace::new().unwrap();
        let config = work.0.join("llvm-config");
        let selected = work.0.join("selected tools");
        let tools = Toolchain {
            llvm_config: config.clone(),
            ..Default::default()
        };
        for (body, expected) in [
            ("printf ' \\n\\t'".to_owned(), "LLVM bindir is empty"),
            (
                format!("printf '%s\\n' '{}'", selected.display()),
                "cannot resolve LLVM bindir",
            ),
        ] {
            std::fs::write(&config, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o700)).unwrap();
            let error = tools
                .optimized_ir("define i32 @main() {ret i32 0}")
                .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        std::fs::write(&selected, b"not a directory").unwrap();
        let error = tools.directory().unwrap_err();
        assert!(error.contains("is not a directory"), "{error}");
        std::fs::remove_file(&selected).unwrap();
        std::fs::create_dir(&selected).unwrap();
        assert_eq!(tools.directory().unwrap(), selected.canonicalize().unwrap());
        let alias = work.0.join("alias");
        std::os::unix::fs::symlink(&selected, &alias).unwrap();
        std::fs::write(
            &config,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", alias.display()),
        )
        .unwrap();
        assert_eq!(tools.directory().unwrap(), selected.canonicalize().unwrap());
    }

    #[test]
    fn successful_llvm_stages_require_artifacts_and_recover() {
        use std::os::unix::fs::PermissionsExt;
        let real = Toolchain::default();
        let directory = real.directory().unwrap();
        let work = Workspace::new().unwrap();
        let quote =
            |path: &std::path::Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
        let script = |path: &std::path::Path, body: &str| {
            std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        let config = work.0.join("llvm-config");
        script(
            &config,
            &format!(
                "case \"$1\" in --bindir) printf '%s\\n' {};; *) exec {} \"$@\";; esac",
                quote(&work.0),
                quote(&real.llvm_config)
            ),
        );
        let tools = Toolchain {
            llvm_config: config,
            ..real
        };
        let source = "define i32 @main() {ret i32 0}";
        for (stage, description) in [
            ("llvm-as", "LLVM bitcode"),
            ("opt", "optimized LLVM"),
            ("llc", "native object"),
        ] {
            for action in [": > \"$destination\"", "mkfifo \"$destination\""] {
                for tool in ["llvm-as", "opt", "llc"] {
                    script(
                        &work.0.join(tool),
                        &format!("exec {} \"$@\"", quote(&directory.join(tool))),
                    );
                }
                let dispatch = if stage == "opt" {
                    format!(
                        "case \"$*\" in *-passes=default*) ;; *) exec {} \"$@\";; esac\n",
                        quote(&directory.join(stage))
                    )
                } else {
                    String::new()
                };
                script(
                    &work.0.join(stage),
                    &format!(
                        "{dispatch}previous=\"\"\nfor argument in \"$@\"; do\n if [ \"$previous\" = -o ]; then destination=\"$argument\"; fi\n previous=\"$argument\"\ndone\n{action}"
                    ),
                );
                let error = tools.object(source).unwrap_err();
                assert!(
                    error.contains(&format!("{description} artifact")),
                    "{stage}: {error}"
                );
                if stage == "llc" {
                    assert!(!tools.bitcode(source).unwrap().is_empty());
                    let error = tools.assembly(source).unwrap_err();
                    assert!(error.contains("native assembly artifact"), "{error}");
                }
                script(
                    &work.0.join(stage),
                    &format!("exec {} \"$@\"", quote(&directory.join(stage))),
                );
                assert!(!tools.object(source).unwrap().is_empty());
            }
        }
    }

    #[test]
    fn compiler_tool_capture_keeps_complete_modules_and_reports_failures() {
        let output = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &[
                "-c".into(),
                "i=0; while [ $i -lt 5000 ]; do printf 0123456789; i=$((i+1)); done".into(),
            ],
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(output, b"0123456789".repeat(5000));
        let error = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf 'explanation\\033'; printf 'failure\\033' >&2; exit 7".into(),
            ],
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(error.contains("failed") && error.contains("7"), "{error}");
        assert!(error.contains("stdout: \"explanation\\u{1b}"), "{error}");
        assert!(error.contains("stderr: \"failure\\u{1b}"), "{error}");
        assert!(!error.contains('\x1b'));
    }

    #[test]
    fn compiler_failure_diagnostics_bound_both_streams() {
        let error = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &["-c".into(), "i=0; while [ $i -lt 5000 ]; do printf 0123456789; printf abcdefghij >&2; i=$((i+1)); done; exit 7".into()],
            Duration::from_secs(5),
        ).unwrap_err();
        assert!(
            error.contains("stdout: [truncated after 16384 bytes]"),
            "{error}"
        );
        assert!(
            error.contains("stderr: [truncated after 16384 bytes]"),
            "{error}"
        );
        assert!(error.len() < Capture::LIMIT * 3);
        let recovered = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &["-c".into(), "printf recovered".into()],
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(recovered, b"recovered");
    }

    #[test]
    fn compiler_tool_flood_cannot_starve_deadline() {
        let start = std::time::Instant::now();
        let error = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &[
                "-c".into(),
                "while :; do printf diagnostic >&2; done".into(),
            ],
            Duration::from_millis(150),
        )
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("truncated"), "{error}");
        assert!(error.len() < Capture::LIMIT * 2);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn exited_compiler_output_collection_keeps_its_deadline() {
        let (mut stdout, out) = Capture::with_limit(1024).unwrap();
        let (mut stderr, err) = Capture::new().unwrap();
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf module; printf diagnostic >&2"])
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if started.elapsed() > Duration::from_secs(5) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("fixture did not exit");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        // The direct child has already succeeded. Its output is still pending,
        // as it can be when an inherited writer keeps the pipe active.
        let error = collect_tool_tail(
            &mut stdout,
            &mut stderr,
            std::time::Instant::now() - Duration::from_secs(1),
            Duration::from_millis(1),
        )
        .unwrap_err();
        assert!(error.contains("output collection timed out"), "{error}");
        // A separate, live collection budget recovers the complete buffered
        // output; timing out did not fabricate successful truncated data.
        collect_tool_tail(
            &mut stdout,
            &mut stderr,
            std::time::Instant::now(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(stdout.bytes, b"module");
        assert_eq!(stderr.bytes, b"diagnostic");
    }

    #[test]
    fn compiler_collection_waits_for_delayed_inherited_writers() {
        let output = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf prefix; (sleep 0.05; printf suffix; printf diagnostic >&2) &".into(),
            ],
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(output, b"prefixsuffix");

        // Silence is not EOF, even when stdout is already closed. A delayed
        // stderr writer must remain under the collection deadline as well.
        let error = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf prefix; (exec 1>&-; sleep 0.2) &".into(),
            ],
            Duration::from_millis(50),
        )
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("prefix"), "{error}");

        // Delayed output must obey the same size limit as direct output.
        let error = run_with_limits(
            std::path::Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf 12345; (sleep 0.05; printf 67890) &".into(),
            ],
            Duration::from_secs(5),
            9,
        )
        .unwrap_err();
        assert!(error.contains("stdout exceeds 9 bytes"), "{error}");
        let output = run_with_timeout(
            std::path::Path::new("/bin/sh"),
            &["-c".into(), "printf recovered".into()],
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(output, b"recovered");
    }

    #[test]
    fn failed_tool_collection_terminates_inherited_writers() {
        for (parent, limit, expected) in [
            ("wait", 1024, "timed out"),
            ("exit 0", 1024, "timed out"),
            ("wait", 3, "stdout exceeds"),
        ] {
            let work = Workspace::new().unwrap();
            let marker = work.0.join("late-write");
            let script = format!("(printf ready; sleep 0.3; printf leaked > \"$1\") & {parent}");
            let error = run_with_limits(
                std::path::Path::new("/bin/sh"),
                &[
                    "-c".into(),
                    script.into(),
                    "fixture".into(),
                    marker.clone().into_os_string(),
                ],
                Duration::from_millis(100),
                limit,
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
            if limit == 1024 {
                assert!(error.contains("ready"), "writer did not start: {error}");
            }
            // The writer must not outlive failed collection and perform a
            // delayed side effect. This also covers an already-reaped parent.
            std::thread::sleep(Duration::from_millis(400));
            assert!(
                !marker.exists(),
                "orphaned writer after {expected}: {parent}"
            );
        }
        assert_eq!(
            run_with_timeout(
                std::path::Path::new("/bin/sh"),
                &["-c".into(), "printf recovered".into()],
                Duration::from_secs(2)
            )
            .unwrap(),
            b"recovered",
        );
    }

    #[test]
    fn compiler_tool_never_publishes_truncated_successful_output() {
        for limit in [0, 5, 9] {
            let error = run_with_limits(
                std::path::Path::new("/bin/sh"),
                &["-c".into(), "printf 0123456789".into()],
                Duration::from_secs(5),
                limit,
            )
            .unwrap_err();
            assert!(
                error.contains(&format!("stdout exceeds {limit} bytes")),
                "{error}"
            );
        }
        let output = run_with_limits(
            std::path::Path::new("/bin/sh"),
            &["-c".into(), "printf 0123456789".into()],
            Duration::from_secs(5),
            10,
        )
        .unwrap();
        assert_eq!(output, b"0123456789");
    }

    #[test]
    fn layout_probe_uses_target_pointer_width_without_executing_target_code() {
        let directory = Toolchain::default().directory().unwrap();
        for (triple, width) in [("i386-pc-linux-gnu", 32), ("x86_64-pc-linux-gnu", 64)] {
            let layout = Toolchain::probe_layout(&directory, triple).unwrap();
            assert_eq!(layout.pointer_bits, width);
            assert_eq!(
                layout
                    .semantic_target()
                    .unwrap()
                    .integer_layout(nir::Primitive::Usize),
                Some((false, width))
            );
            assert_eq!(layout.triple, triple);
            assert!(
                layout
                    .to_string()
                    .contains(&format!("(pointer-bits {width})"))
            );
        }
        assert!(Toolchain::probe_layout(&directory, "").is_err());
        assert!(Toolchain::probe_layout(&directory, "x86_64\n-pc-linux").is_err());
    }

    #[test]
    fn host_rejects_conflicting_module_targets_before_optimization() {
        let toolchain = Toolchain::default();
        let target = toolchain.target_layout().unwrap();
        assert!(target.to_string().starts_with("(host-target (triple \""));
        let foreign = if target.triple.starts_with("i386-") {
            "x86_64-pc-linux-gnu"
        } else {
            "i386-pc-linux-gnu"
        };
        for header in [
            format!("target triple = {foreign:?}"),
            "target datalayout = \"E-p:32:32\"".into(),
        ] {
            let source = format!("{header}\ndefine i32 @main() {{ret i32 0}}");
            assert!(
                toolchain
                    .optimized_ir(&source)
                    .unwrap_err()
                    .contains("mismatch")
            );
            assert!(toolchain.compile(&source).unwrap_err().contains("mismatch"));
            assert!(toolchain.bitcode(&source).unwrap_err().contains("mismatch"));
            assert!(
                toolchain
                    .assembly(&source)
                    .unwrap_err()
                    .contains("mismatch")
            );
        }
        // Failure must not poison subsequent requests on the same toolchain.
        let source = format!(
            "target triple = {:?}\ntarget datalayout = {:?}\ndefine i32 @main() {{ret i32 0}}",
            target.triple, target.data_layout
        );
        assert!(toolchain.execute_test_ir(&source).unwrap().status.success());
    }

    #[test]
    fn native_optimization_resolves_target_before_pointer_layout() {
        for optimization in [OptimizationLevel::O0, OptimizationLevel::O3] {
            let toolchain = Toolchain {
                optimization,
                ..Toolchain::default()
            };
            // Test the native ABI against an independently compiled Rust
            // pointer size; production code never derives layout from Rust.
            let source = format!(
                "define i64 @pointer_size() {{ %p = getelementptr ptr, ptr null, i64 1\n %n = ptrtoint ptr %p to i64\n ret i64 %n }}\ndefine i32 @main() {{ %n = call i64 @pointer_size()\n %bad = icmp ne i64 %n, {}\n %status = zext i1 %bad to i32\n ret i32 %status }}",
                std::mem::size_of::<*const ()>()
            );
            let ir = toolchain.optimized_ir(&source).unwrap();
            assert!(ir.contains("target triple ="), "{ir}");
            assert!(ir.contains("target datalayout ="), "{ir}");
            let repeated = toolchain.optimized_ir(&ir).unwrap();
            assert_eq!(ir, repeated);
            assert!(toolchain.execute_test_ir(&ir).unwrap().status.success());
        }
    }

    #[test]
    fn native_launch_retries_transient_text_busy_but_bounds_persistent_failure() {
        use std::os::unix::fs::PermissionsExt;
        let bytes = Toolchain::default()
            .compile("define i32 @main() { ret i32 7 }")
            .unwrap();
        let work = Workspace::new().unwrap();
        let path = work.0.join("busy-program");
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        assert_eq!(
            Command::new(&path).spawn().unwrap_err().raw_os_error(),
            Some(26)
        );
        let release = std::thread::spawn(move || {
            // Reproduce a contention interval longer than the former 100ms cap.
            std::thread::sleep(Duration::from_millis(200));
            drop(writer);
        });
        let mut child = spawn_native(&mut Command::new(&path)).unwrap();
        release.join().unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(7));

        // LLVM tools use the same launch guard. A successful launch followed
        // by exit 7 must be reported once as execution failure, never retried.
        let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(writer);
        });
        let failure = run_with_timeout(&path, &[], Duration::from_secs(2)).unwrap_err();
        release.join().unwrap();
        assert!(
            failure.contains("failed") && failure.contains('7'),
            "{failure}"
        );

        let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let started = std::time::Instant::now();
        let failure =
            spawn_native_with_timeout(&mut Command::new(&path), Duration::from_millis(30))
                .unwrap_err();
        assert!(failure.contains("cannot start native test"), "{failure}");
        assert!(started.elapsed() >= Duration::from_millis(30));
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(writer);
        let mut child = spawn_native(&mut Command::new(&path)).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(7));
    }

    fn output_ir(tail: &str) -> String {
        format!(
            r#"
@message = private constant [4 x i8] c"hi\0A\1B"
declare i64 @write(i32, ptr, i64)
define i32 @main() {{
entry:
  %a = call i64 @write(i32 1, ptr @message, i64 4)
  %b = call i64 @write(i32 2, ptr @message, i64 4)
  {tail}
}}
"#
        )
    }

    #[test]
    fn native_output_failure_captures_both_streams_and_escapes_controls() {
        let error = Toolchain::default()
            .execute_test(&output_ir("ret i32 7"), Duration::from_secs(2))
            .unwrap_err();
        assert!(error.contains("native test failed"), "{error}");
        assert!(error.contains(r#"stdout: "hi\n\u{1b}""#), "{error}");
        assert!(error.contains(r#"stderr: "hi\n\u{1b}""#), "{error}");
        assert!(!error.contains('\x1b'));
        Toolchain::default()
            .execute_test(&output_ir("ret i32 0"), Duration::from_secs(2))
            .unwrap();
    }

    #[test]
    fn native_comparison_runner_bounds_execution_and_preserves_output() {
        let tools = Toolchain::default();
        let looping =
            output_ir("br label %entry").replace("entry:", "start:\n  br label %entry\nentry:");
        let error = tools
            .execute_test_ir_with_timeout(&looping, Duration::from_millis(100))
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.len() < Capture::LIMIT * 10);
        for status in [0, 7] {
            let output = tools
                .execute_test_ir(&output_ir(&format!("ret i32 {status}")))
                .unwrap();
            assert_eq!(output.status.code(), Some(status));
            assert_eq!(output.stdout, b"hi\n\x1b");
            assert_eq!(output.stderr, b"hi\n\x1b");
        }
        assert!(
            tools
                .execute_test_ir_with_timeout("invalid IR", Duration::ZERO)
                .unwrap_err()
                .contains("timeout must be positive")
        );
    }

    #[test]
    fn native_comparison_collects_inherited_output_and_bounds_silent_writers() {
        // The direct process exits before the writer emits anything. An empty
        // nonblocking read is not evidence that the program output is complete.
        fn source(delay: u32, count: usize) -> String {
            format!(
                "@bytes = private constant [{count} x i8] zeroinitializer
                 declare i32 @fork()
                 declare i32 @usleep(i32)
                 declare i64 @write(i32, ptr, i64)
                 declare void @_exit(i32) noreturn
                 define i32 @main() {{
                 entry:
                   %pid = call i32 @fork()
                   %failed = icmp slt i32 %pid, 0
                   br i1 %failed, label %failure, label %started
                 failure:
                   ret i32 9
                 started:
                   %child = icmp eq i32 %pid, 0
                   br i1 %child, label %writer, label %parent
                 parent:
                   ret i32 0
                 writer:
                   %wait = call i32 @usleep(i32 {delay})
                   %out = call i64 @write(i32 1, ptr @bytes, i64 {count})
                   %err = call i64 @write(i32 2, ptr @bytes, i64 {count})
                   call void @_exit(i32 0)
                   unreachable
                 }}"
            )
        }
        let tools = Toolchain::default();
        let output = tools.execute_test_ir(&source(50000, 4)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, [0; 4]);
        assert_eq!(output.stderr, [0; 4]);
        let error = tools
            .execute_test_ir_with_timeout(&source(300000, 4), Duration::from_millis(50))
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        let error = tools.execute_test_ir(&source(50000, 20000)).unwrap_err();
        assert!(error.contains("captured output exceeds limit"), "{error}");
        // The status-only API still accepts noisy successful programs, while
        // draining both inherited streams to avoid hiding a stalled writer.
        tools
            .execute_test(&source(50000, 20000), Duration::from_secs(2))
            .unwrap();
        let output = tools.execute_test_ir(&source(1000, 4)).unwrap();
        assert_eq!(output.stdout, [0; 4]);
    }

    #[test]
    fn native_comparison_runner_rejects_truncated_success() {
        let tools = Toolchain::default();
        for fd in [1, 2] {
            let source = format!(
                "@data = private constant [20000 x i8] zeroinitializer\ndeclare i64 @write(i32, ptr, i64)\ndefine i32 @main() {{ %n = call i64 @write(i32 {fd}, ptr @data, i64 20000)\n ret i32 0 }}"
            );
            let error = tools.execute_test_ir(&source).unwrap_err();
            assert!(error.contains("captured output exceeds limit"), "{error}");
            assert!(error.contains("truncated after 16384 bytes"), "{error}");
            // Status-only execution still accepts a successful noisy test.
            tools.execute_test(&source, Duration::from_secs(2)).unwrap();
        }
    }

    #[test]
    fn native_output_flood_is_bounded_and_cannot_disable_timeout() {
        let ir = output_ir("br label %entry");
        let error = Toolchain::default()
            .execute_test(
                &ir.replace("entry:", "start:\n  br label %entry\nentry:"),
                Duration::from_millis(150),
            )
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("stdout: [truncated"), "{error}");
        assert!(error.contains("stderr: [truncated"), "{error}");
        assert!(error.len() < Capture::LIMIT * 10);
    }
}
