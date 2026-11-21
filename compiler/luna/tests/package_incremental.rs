//! End-to-end integration tests for Layer 3A package-level incremental builds
//! and multi-core parallel compilation (`ParsedPackage`, `VerifiedUpstream`,
//! `--cache-dir`, `--no-cache`, `-j`/`--jobs`).

use luna::cli::{self, Command};
use luna::frontend::{self, Artifact};
use luna::package_cache::{
    ArtifactKind, CachedParsedFile, CachedParsedPackage, DiskPackageCache, PackageSnapshotKey,
    Sha256, VerifiedUpstreamCertificate, decode_parsed_package, decode_verified_upstream,
    encode_parsed_package, encode_verified_upstream, global_elaborated_cache,
};
use parse::ast::{Ast, NodeBuilder, NodeKind};
use parse::parser::Parser;
use rustc_span::source_map::{FilePathMapping, SourceMap};
use rustc_span::{BytePos, DUMMY_SP, FileName, Span};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "luna-pkg-incr-{label}-{}-{nonce}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let full = self.path(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, text).unwrap();
        full
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn repo_library() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn run_luna(args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_luna"))
        .args(args)
        .output()
        .expect("failed to execute luna")
}

fn collect_flpkg_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                visit(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("flpkg") {
                out.push(p);
            }
        }
    }
    visit(dir, &mut out);
    out.sort();
    out
}

fn setup_minimal_sysroot(ws: &TempWorkspace, builtin_lib: &str) {
    ws.write("library/builtin/lib.fl", builtin_lib);
    for pkg in ["std", "meta", "build", "attrs", "verification"] {
        ws.write(&format!("library/{pkg}/lib.fl"), "pub mod prelude {}\n");
    }
    ws.write("library/language-items.conf", "version = 1\n");
}

#[test]
fn parsed_package_and_verified_upstream_codecs_roundtrip_and_reject_corruption() {
    let sample = "\
use std.prelude.*;
pub typealias Word = .u32'builtin;
pub fn id(f: fn(Word) -> Word) -> .ok { .ok }
pub fn check(x: Word) -> Word { x }
";
    // Parse with BytePos(1) offset so byte 0 doesn't alias DUMMY_SP.
    let sm = SourceMap::new(FilePathMapping::empty());
    let _ = sm.new_source_file(FileName::Custom("dummy".into()), " ".into());
    let sf = sm.new_source_file(FileName::Custom("mod.fl".into()), sample.into());
    let diag = diagnostic::DiagnosticContext::new(&sm);
    let mut budget = macro_expander::MacroBudget::default();
    let (expanded, lex_errs) = macro_expander::expand_source_file(&sm, &sf, &mut budget).unwrap();
    assert!(lex_errs.is_empty());
    let mut parser = Parser::new(&sm, expanded.tokens, expanded.symbols, expanded.start_pos);
    assert!(parser.parse(&diag));
    let parsed_ast = parser.finalize();
    let expected_dump = parsed_ast.dump_to_s_expression(parsed_ast.root, &sm);

    // Also build a synthetic AST with a genuine zero-width span at byte 0 + DUMMY_SP.
    let mut synth = Ast::new(BytePos(1));
    let zw = NodeBuilder::new(NodeKind::Null, Span::new(BytePos(1), BytePos(1))).build(&mut synth);
    let dm = NodeBuilder::new(NodeKind::Wildcard, DUMMY_SP).build(&mut synth);
    let root = NodeBuilder::new(NodeKind::FileScope, Span::new(BytePos(1), BytePos(2)))
        .add_multiple_children(vec![zw, dm])
        .build(&mut synth);
    synth.root = root;

    let pkg = CachedParsedPackage {
        files: vec![
            CachedParsedFile {
                rel_path: "mod.fl".into(),
                content_hash: Sha256::hash(sample.as_bytes()),
                ast: Some(parsed_ast.clone()),
            },
            CachedParsedFile {
                rel_path: "empty_span.fl".into(),
                content_hash: [7u8; 32],
                ast: Some(synth),
            },
            CachedParsedFile {
                rel_path: "unparsed.fl".into(),
                content_hash: [9u8; 32],
                ast: None,
            },
        ],
    };

    let ws = TempWorkspace::new("codec");
    let cache = DiskPackageCache::new(ws.path("cache"));
    let key = PackageSnapshotKey(Sha256::hash(b"parsed-key"));
    let encoded = encode_parsed_package(&pkg).unwrap();
    cache
        .store(&key, ArtifactKind::ParsedPackage, &encoded)
        .unwrap();
    let loaded_bytes = cache.load(&key, ArtifactKind::ParsedPackage).unwrap();
    let decoded = decode_parsed_package(&loaded_bytes).unwrap();
    assert_eq!(decoded.files.len(), 3);
    assert!(decoded.files[2].ast.is_none());

    // Rebase into a completely new SourceMap at an arbitrary start_pos.
    let sm2 = SourceMap::new(FilePathMapping::empty());
    let _ = sm2.new_source_file(FileName::Custom("pad".into()), "x".repeat(4096));
    let sf2 = sm2.new_source_file(FileName::Custom("mod.fl".into()), sample.into());
    let mut rebased = decoded.files[0].ast.clone().unwrap();
    rebased.rebase(sf2.start_pos);
    assert_eq!(
        rebased.dump_to_s_expression(rebased.root, &sm2),
        expected_dump
    );

    // Check zero-width offset vs DUMMY_SP preservation across rebase.
    let mut synth_decoded = decoded.files[1].ast.clone().unwrap();
    synth_decoded.rebase(BytePos(777));
    assert_eq!(
        synth_decoded.get_span(zw),
        Some(Span::new(BytePos(777), BytePos(777)))
    );
    assert_eq!(synth_decoded.get_span(dm), Some(DUMMY_SP));

    // Corruptions must cleanly error without panicking or OOM.
    assert!(decode_parsed_package(&[]).is_err());
    assert!(decode_parsed_package(&encoded[..encoded.len() - 5]).is_err());
    let mut with_trailing = encoded.clone();
    with_trailing.push(0);
    assert!(decode_parsed_package(&with_trailing).is_err());
    let mut bad_ver = encoded.clone();
    bad_ver[0] = 99;
    assert!(decode_parsed_package(&bad_ver).is_err());
    let mut oom_count = vec![1, 0, 0, 0];
    oom_count.extend_from_slice(&u64::MAX.to_le_bytes());
    assert!(decode_parsed_package(&oom_count).is_err());

    // VerifiedUpstreamCertificate roundtrip & corruption rejection.
    let cert = VerifiedUpstreamCertificate {
        upstream_identities: vec![1, 2, 3, 4, 5, 6],
    };
    let cert_bytes = encode_verified_upstream(&cert);
    let cert_key = PackageSnapshotKey(Sha256::hash(b"verified-key"));
    cache
        .store(&cert_key, ArtifactKind::VerifiedUpstream, &cert_bytes)
        .unwrap();
    assert_eq!(
        decode_verified_upstream(
            &cache
                .load(&cert_key, ArtifactKind::VerifiedUpstream)
                .unwrap()
        )
        .unwrap(),
        cert
    );
    assert!(decode_verified_upstream(&cert_bytes[..cert_bytes.len() - 1]).is_err());
    let mut cert_trailing = cert_bytes.clone();
    cert_trailing.extend_from_slice(&[1, 2]);
    assert!(decode_verified_upstream(&cert_trailing).is_err());
}

#[test]
fn real_sysroot_cold_vs_warm_cache_reuse_and_parity_across_downstream_edits() {
    let ws = TempWorkspace::new("real-sysroot-parity");
    let cache_dir = ws.path("cache");
    let input = ws.write(
        "input.fl",
        "\
use std.prelude.*;
fn helper(x: u32) -> u32 where requires x < 1000 {
    x + 7
}
fn main() -> bool {
    helper(35) == 42
}
",
    );

    let lib = repo_library();
    let cold = run_luna(&[
        "check",
        input.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ]);
    assert!(
        cold.status.success(),
        "cold check failed: {}",
        String::from_utf8_lossy(&cold.stderr)
    );

    let flpkg_after_cold = collect_flpkg_files(&cache_dir);
    assert!(
        flpkg_after_cold.len() >= 2,
        "expected ParsedPackage + VerifiedUpstream .flpkg entries, got {flpkg_after_cold:?}"
    );

    // Edit downstream source (adding multiple declarations, which shifts downstream/sysroot
    // DefId indices) and re-run with same --cache-dir.
    let updated = ws.write(
        "input.fl",
        "\
use std.prelude.*;
struct ExtraBox { value: u32 }
fn added_one(v: u32) -> u32 { v }
fn helper(x: u32) -> u32 where requires x < 1000 {
    x + 8
}
fn main() -> bool {
    let b = ExtraBox { value: added_one(helper(34)) };
    b.value == 42
}
",
    );

    let warm_dump = run_luna(&[
        "dump",
        updated.to_str().unwrap(),
        "--emit",
        "nir",
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ]);
    assert!(
        warm_dump.status.success(),
        "warm dump failed: {}",
        String::from_utf8_lossy(&warm_dump.stderr)
    );
    let flpkg_after_warm = collect_flpkg_files(&cache_dir);
    assert_eq!(
        flpkg_after_warm, flpkg_after_cold,
        "editing downstream input must reuse exact same upstream .flpkg cache entries"
    );

    let nocache_dump = run_luna(&[
        "dump",
        updated.to_str().unwrap(),
        "--emit",
        "nir",
        "--sysroot",
        lib.to_str().unwrap(),
        "--no-cache",
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ]);
    assert!(nocache_dump.status.success());
    assert_eq!(
        warm_dump.stdout, nocache_dump.stdout,
        "warm cache NIR dump must match --no-cache cold dump identically"
    );

    let warm_exec = run_luna(&[
        "interpret",
        updated.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert!(
        warm_exec.status.success(),
        "{}",
        String::from_utf8_lossy(&warm_exec.stderr)
    );
    assert_eq!(warm_exec.stdout, b"(bool true)\n");
}

#[test]
fn upstream_invalidation_catches_flow_pure_and_syntax_errors() {
    let ws = TempWorkspace::new("upstream-inval");
    let cache_dir = ws.path("cache");
    let lib = ws.path("library");
    let valid_builtin = "\
pub typealias u8 = .u8'builtin;
pub typealias bool = .bool'builtin;
pub mod prelude {
    pub use .bool;
    pub use .u8;
    pub use .safe_inc;
}
pub pure fn safe_inc(x: u8) -> u8 where requires x < 200 {
    x + 1
}
";
    setup_minimal_sysroot(&ws, valid_builtin);
    let input = ws.write("input.fl", "fn main() -> bool { safe_inc(1) == 2 }");

    // 1. Cold build succeeds.
    let ok = run_luna(&[
        "check",
        input.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    // 2. Upstream flow arithmetic overflow invalidates cache & fails.
    ws.write(
        "library/builtin/lib.fl",
        "\
pub typealias u8 = .u8'builtin;
pub typealias bool = .bool'builtin;
pub mod prelude { pub use .bool; pub use .u8; pub use .safe_inc; }
pub fn safe_inc(x: u8) -> u8 { x + 1 }
",
    );
    let flow_err = run_luna(&[
        "check",
        input.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert!(!flow_err.status.success(), "upstream overflow must fail");
    let flow_stderr = String::from_utf8_lossy(&flow_err.stderr);
    assert!(flow_stderr.contains("flow-error"), "{flow_stderr}");

    // 3. Upstream P4 purity violation invalidates cache & fails.
    ws.write(
        "library/builtin/lib.fl",
        "\
pub typealias u8 = .u8'builtin;
pub typealias bool = .bool'builtin;
pub mod prelude { pub use .bool; pub use .u8; pub use .safe_inc; }
pub pure fn safe_inc(x: u8) -> u8 where requires x < 200 { x + 1 }
pub pure fn bad_ptr(external: *bool) -> bool { external.* = true; true }
",
    );
    let pure_err = run_luna(&[
        "check",
        input.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert!(
        !pure_err.status.success(),
        "upstream pure violation must fail"
    );
    let pure_stderr = String::from_utf8_lossy(&pure_err.stderr);
    assert!(pure_stderr.contains("lacks P4 evidence"), "{pure_stderr}");

    // 4. Upstream syntax error invalidates ParsedPackage AST cache & fails.
    ws.write("library/builtin/lib.fl", "pub fn broken( {");
    let syn_err = run_luna(&[
        "check",
        input.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
    ]);
    assert!(!syn_err.status.success(), "upstream syntax error must fail");
}

#[test]
fn scoped_build_mode_and_target_invalidation_and_corrupt_cache_recovery() {
    let ws = TempWorkspace::new("scope-and-corrupt");
    let cache_dir = ws.path("cache");
    let lib = ws.path("library");
    setup_minimal_sysroot(
        &ws,
        "pub typealias bool = .bool'builtin; pub mod prelude { pub use .bool; }\n",
    );
    let input = ws.write("input.fl", "fn main() -> bool { true }");

    let run = |extra: &[&str]| {
        let mut args = vec![
            "check",
            input.to_str().unwrap(),
            "--sysroot",
            lib.to_str().unwrap(),
            "--cache-dir",
            cache_dir.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let out = run_luna(&args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run(&[
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--build-mode",
        "debug",
    ]);
    let count_base = collect_flpkg_files(&cache_dir).len();

    // Downstream-only build.mode change must NOT bust upstream cache.
    run(&[
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--build-mode",
        "release",
    ]);
    assert_eq!(collect_flpkg_files(&cache_dir).len(), count_base);

    // Upstream package build.mode change DOES bust VerifiedUpstream certificate.
    run(&[
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--package-build-mode",
        "1=release",
    ]);
    let count_up_mode = collect_flpkg_files(&cache_dir).len();
    assert!(count_up_mode > count_base);

    // Build target pointer width change DOES bust VerifiedUpstream certificate.
    run(&[
        "--build-target",
        "i386",
        "linux",
        "32",
        "--build-mode",
        "release",
    ]);
    assert!(collect_flpkg_files(&cache_dir).len() > count_up_mode);

    // Corrupt every .flpkg file on disk; compiler must cleanly fall back and succeed.
    for path in collect_flpkg_files(&cache_dir) {
        std::fs::write(&path, b"CORRUPTED\x00\xff").unwrap();
    }
    run(&["--build-target", "x86_64", "linux", "64"]);
}

#[test]
fn downstream_failures_still_rejected_under_warm_upstream_cache() {
    let ws = TempWorkspace::new("downstream-reject");
    let cache_dir = ws.path("cache");
    let lib = repo_library();

    // Warm the upstream cache with a valid program.
    let valid = ws.write("valid.fl", "use std.prelude.*; fn main() -> bool { true }");
    let warm = run_luna(&[
        "check",
        valid.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ]);
    assert!(
        warm.status.success(),
        "{}",
        String::from_utf8_lossy(&warm.stderr)
    );

    // 1. Downstream arithmetic overflow under warm upstream cache.
    let overflow = ws.write(
        "overflow.fl",
        "use std.prelude.*; fn bad(x: u32) -> u32 { x + 1 }",
    );
    let res = run_luna(&[
        "check",
        overflow.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ]);
    assert!(!res.status.success(), "downstream overflow must still fail");
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(stderr.contains("flow-error"), "{stderr}");

    // 2. Downstream P4 pure fn violation under warm upstream cache.
    let impure = ws.write(
        "impure.fl",
        "use std.prelude.*; pure fn bad(external: *bool) -> bool { external.* = true; true }",
    );
    let res = run_luna(&[
        "check",
        impure.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ]);
    assert!(
        !res.status.success(),
        "downstream pure violation must still fail"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(stderr.contains("lacks P4 evidence"), "{stderr}");

    // 3. Downstream trait contract strengthening under warm upstream cache.
    let strengthen = ws.write(
        "strengthen.fl",
        "\
use std.prelude.*;
trait Positive {
    fn check(x: u32) -> u32;
}
struct Bad {}
impl Positive for Bad {
    fn check(x: u32) -> u32 where requires x > 10 { x }
}
",
    );
    let res = run_luna(&[
        "check",
        strengthen.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--cache-dir",
        cache_dir.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ]);
    assert!(
        !res.status.success(),
        "downstream contract strengthening must still fail"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(stderr.contains("strengthens"), "{stderr}");
}

#[test]
fn parallel_jobs_determinism_matches_sequential() {
    let ws = TempWorkspace::new("parallel-det");
    let lib = repo_library();
    let entry = ws.write(
        "pkg/main.fl",
        "use std.prelude.*; use extra.add_two; fn main() -> bool { add_two(40) == 42 }",
    );
    ws.write(
        "pkg/extra.fl",
        "pub fn add_two(x: u32) -> u32 where requires x < 1000 { x + 2 }",
    );

    let seq = run_luna(&[
        "dump",
        entry.to_str().unwrap(),
        "--emit",
        "nir",
        "--sysroot",
        lib.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--no-cache",
        "--jobs",
        "1",
    ]);
    let par = run_luna(&[
        "dump",
        entry.to_str().unwrap(),
        "--emit",
        "nir",
        "--sysroot",
        lib.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--no-cache",
        "-j",
        "8",
    ]);
    assert!(
        seq.status.success(),
        "{}",
        String::from_utf8_lossy(&seq.stderr)
    );
    assert!(
        par.status.success(),
        "{}",
        String::from_utf8_lossy(&par.stderr)
    );
    assert_eq!(seq.stdout, par.stdout);

    // Also verify deterministic error reporting across worker counts.
    let bad = ws.write("bad.fl", "use .missing_a.*;\n");
    ws.write("pkg/extra.fl", "fn broken_a() { let = ; }");
    let seq_err = run_luna(&[
        "check",
        entry.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--no-cache",
        "--jobs",
        "1",
    ]);
    let par_err = run_luna(&[
        "check",
        entry.to_str().unwrap(),
        "--sysroot",
        lib.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--no-cache",
        "--jobs",
        "8",
    ]);
    assert!(!seq_err.status.success());
    assert!(!par_err.status.success());
    assert_eq!(seq_err.stderr, par_err.stderr);
    let _ = bad;
}

#[test]
fn cli_incremental_flags_validation_and_forwarding() {
    let parse = |args: &[&str]| cli::parse(args.iter().map(OsString::from));

    let Command::Check(opts) = parse(&[
        "check",
        "input.fl",
        "--cache-dir",
        "/tmp/custom-cache",
        "--jobs",
        "16",
    ])
    .unwrap() else {
        panic!("expected Check");
    };
    assert_eq!(
        opts.incremental.cache_dir,
        Some(PathBuf::from("/tmp/custom-cache"))
    );
    assert!(!opts.incremental.no_cache);
    assert_eq!(opts.incremental.jobs, Some(16));

    let Command::Check(opts) = parse(&["check", "input.fl", "--no-cache", "-j", "4"]).unwrap()
    else {
        panic!("expected Check");
    };
    assert!(opts.incremental.no_cache);
    assert_eq!(opts.incremental.jobs, Some(4));

    let Command::CheckPackages(req) = parse(&[
        "packages",
        "check",
        "--package",
        "1",
        "101",
        "app",
        "input.fl",
        "--primary",
        "1",
        "--cache-dir",
        "/tmp/pkg-cache",
        "-j",
        "12",
    ])
    .unwrap() else {
        panic!("expected CheckPackages");
    };
    assert_eq!(
        req.options.incremental.cache_dir,
        Some(PathBuf::from("/tmp/pkg-cache"))
    );
    assert_eq!(req.options.incremental.jobs, Some(12));

    let Command::Packages(dump_req) = parse(&[
        "packages",
        "dump",
        "--emit",
        "nir",
        "--package",
        "1",
        "101",
        "app",
        "input.fl",
        "--primary",
        "1",
        "--cache-dir",
        "/tmp/pkg-cache",
        "-j",
        "12",
    ])
    .unwrap() else {
        panic!("expected Packages");
    };
    assert_eq!(
        dump_req.options.incremental.cache_dir,
        Some(PathBuf::from("/tmp/pkg-cache"))
    );
    assert_eq!(dump_req.options.incremental.jobs, Some(12));

    let Command::CheckPackages(req_nc) = parse(&[
        "packages",
        "check",
        "--package",
        "1",
        "101",
        "app",
        "input.fl",
        "--primary",
        "1",
        "--no-cache",
    ])
    .unwrap() else {
        panic!("expected CheckPackages");
    };
    assert!(req_nc.options.incremental.no_cache);

    for invalid in [
        vec!["check", "input.fl", "--cache-dir", "/tmp/c", "--no-cache"],
        vec!["check", "input.fl", "--jobs", "0"],
        vec!["check", "input.fl", "-j", "0"],
        vec!["check", "input.fl", "--jobs", "abc"],
        vec!["check", "input.fl", "--jobs", "2", "-j", "4"],
        vec!["check", "input.fl", "--no-cache", "--no-cache"],
        vec!["check", "input.fl", "--cache-dir", "a", "--cache-dir", "b"],
    ] {
        assert!(
            parse(&invalid).is_err(),
            "accepted invalid args: {invalid:?}"
        );
    }
}

static ELAB_CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn parse_in_proc_nir_options(
    input: &Path,
    lib: &Path,
    cache_dir: Option<&Path>,
    no_cache: bool,
) -> luna::cli::Options {
    let mut args = vec![
        "dump",
        input.to_str().unwrap(),
        "--emit",
        "nir",
        "--sysroot",
        lib.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
    ];
    if let Some(dir) = cache_dir {
        args.push("--cache-dir");
        args.push(dir.to_str().unwrap());
    }
    if no_cache {
        args.push("--no-cache");
    }
    let Command::Compile(opts) = cli::parse(args.into_iter().map(OsString::from)).unwrap() else {
        panic!("expected Compile");
    };
    opts
}

#[test]
fn in_process_elaborated_body_cache_reuses_upstream_bodies_with_exact_nir_parity() {
    let _guard = ELAB_CACHE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let ws = TempWorkspace::new("inproc-elab-parity");
    let cache_dir = ws.path("cache");
    let lib = repo_library();

    let first_input = ws.write(
        "first.fl",
        "\
use std.prelude.*;
fn helper(x: u32) -> u32 where requires x < 1000 { x + 2 }
fn main() -> bool { helper(40) == 42 }
",
    );
    let second_input = ws.write(
        "second.fl",
        "\
use std.prelude.*;
struct ExtraBox { value: u32 }
fn added_one(v: u32) -> u32 { v }
fn added_two(v: u32) -> u32 where requires v < 1000 { v + 1 }
fn helper(x: u32) -> u32 where requires x < 1000 { x + 8 }
fn main() -> bool {
    let b = ExtraBox { value: added_one(added_two(41)) };
    let h = helper(34);
    b.value == h
}
",
    );

    let before = global_elaborated_cache().stats();
    let len_before = global_elaborated_cache().len();
    let Artifact::Nir(first_nir) = frontend::compile(&parse_in_proc_nir_options(
        &first_input,
        &lib,
        Some(&cache_dir),
        false,
    ))
    .unwrap() else {
        panic!("expected NIR");
    };
    let after_cold = global_elaborated_cache().stats();
    assert_eq!(global_elaborated_cache().len(), len_before + 1);
    assert_eq!(after_cold.hits, before.hits);

    let Artifact::Nir(warm_nir) = frontend::compile(&parse_in_proc_nir_options(
        &second_input,
        &lib,
        Some(&cache_dir),
        false,
    ))
    .unwrap() else {
        panic!("expected NIR");
    };
    let after_warm = global_elaborated_cache().stats();
    assert_eq!(after_warm.hits, after_cold.hits + 1);

    let Artifact::Nir(nocache_nir) =
        frontend::compile(&parse_in_proc_nir_options(&second_input, &lib, None, true)).unwrap()
    else {
        panic!("expected NIR");
    };
    assert_eq!(
        warm_nir.to_string(),
        nocache_nir.to_string(),
        "warm in-process elaborated cache NIR must match --no-cache identically"
    );

    let upstream_fns = |pkg: &nir::program::Package| -> Vec<(nir::DeclarationId, String)> {
        pkg.functions
            .iter()
            .filter(|f| f.declaration.package != 0)
            .map(|f| (f.declaration, f.name.clone()))
            .collect()
    };
    assert_eq!(
        upstream_fns(&first_nir),
        upstream_fns(&warm_nir),
        "upstream DeclarationIds must remain invariant across downstream additions"
    );
}

#[test]
fn in_process_elaborated_body_cache_invalidates_on_upstream_body_change() {
    let _guard = ELAB_CACHE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let ws = TempWorkspace::new("inproc-elab-inval");
    let cache_dir = ws.path("cache");
    let lib = ws.path("library");

    let mk_builtin = |val: u8, body_safe: bool| {
        if body_safe {
            format!(
                "\
pub typealias u8 = .u8'builtin;
pub typealias bool = .bool'builtin;
pub mod prelude {{
    pub use .bool;
    pub use .u8;
    pub use .upstream_val;
    pub use .upstream_inc;
}}
pub pure fn upstream_val() -> u8 {{ {val} }}
pub fn upstream_inc(x: u8) -> u8 where requires x < 200 {{ x + 1 }}
"
            )
        } else {
            "\
pub typealias u8 = .u8'builtin;
pub typealias bool = .bool'builtin;
pub mod prelude {
    pub use .bool;
    pub use .u8;
    pub use .upstream_val;
    pub use .upstream_inc;
}
pub pure fn upstream_val() -> u8 { 10 }
pub fn upstream_inc(x: u8) -> u8 { x + 1 }
"
            .to_string()
        }
    };

    setup_minimal_sysroot(&ws, &mk_builtin(10, true));
    let input1 = ws.write(
        "input1.fl",
        "fn main() -> u8 { let _ = upstream_inc(5); upstream_val() }",
    );
    let input2 = ws.write(
        "input2.fl",
        "fn helper() -> bool { true } fn main() -> u8 { let _ = upstream_inc(5); upstream_val() }",
    );

    let s0 = global_elaborated_cache().stats();
    let len0 = global_elaborated_cache().len();
    let Artifact::Nir(cold_nir) = frontend::compile(&parse_in_proc_nir_options(
        &input1,
        &lib,
        Some(&cache_dir),
        false,
    ))
    .unwrap() else {
        panic!("expected NIR");
    };
    let s1 = global_elaborated_cache().stats();
    assert_eq!(s1.hits, s0.hits);
    assert_eq!(global_elaborated_cache().len(), len0 + 1);

    let Artifact::Nir(warm_nir) = frontend::compile(&parse_in_proc_nir_options(
        &input2,
        &lib,
        Some(&cache_dir),
        false,
    ))
    .unwrap() else {
        panic!("expected NIR");
    };
    let s2 = global_elaborated_cache().stats();
    assert_eq!(s2.hits, s1.hits + 1);
    assert_ne!(cold_nir.functions.len(), warm_nir.functions.len());

    // Changing an upstream body in builtin invalidates both VerifiedUpstream and ElaboratedBase.
    setup_minimal_sysroot(&ws, &mk_builtin(77, true));
    let Artifact::Nir(updated_nir) = frontend::compile(&parse_in_proc_nir_options(
        &input2,
        &lib,
        Some(&cache_dir),
        false,
    ))
    .unwrap() else {
        panic!("expected NIR");
    };
    let s3 = global_elaborated_cache().stats();
    assert_eq!(
        s3.hits, s2.hits,
        "stale elaborated cache must not hit after upstream edit"
    );
    assert_eq!(global_elaborated_cache().len(), len0 + 2);
    let updated_val_fn = updated_nir
        .functions
        .iter()
        .find(|f| f.name == "upstream_val")
        .unwrap();
    assert!(
        updated_val_fn.body.to_string().contains("77"),
        "expected updated constant 77 in upstream_val body: {}",
        updated_val_fn.body
    );

    // Introducing an overflow bug in upstream fails cleanly instead of reusing stale bodies.
    setup_minimal_sysroot(&ws, &mk_builtin(10, false));
    let Err(err) = frontend::compile(&parse_in_proc_nir_options(
        &input2,
        &lib,
        Some(&cache_dir),
        false,
    )) else {
        panic!("expected upstream overflow failure");
    };
    assert!(err.contains("flow-error"), "{err}");
}
