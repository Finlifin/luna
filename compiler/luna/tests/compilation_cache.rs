use luna::{
    compilation_cache::CompilationCache,
    frontend::Artifact,
    package_cli::{self, Request},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "luna-compilation-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn request(&self, file: &str) -> Request {
        package_cli::parse([
            "--package".into(),
            "1".into(),
            "101".into(),
            "client".into(),
            self.0.join(file).into_os_string(),
            "--primary".into(),
            "1".into(),
            "--emit".into(),
            "nir".into(),
        ])
        .unwrap()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn result(value: &luna::compilation_cache::CompilationOutcome) -> comptime_interpretor::Value {
    let Artifact::Nir(package) = &value.compilation.artifact else {
        panic!("expected NIR")
    };
    luna::execution::interpret(package, "run", 100_000).unwrap()
}
#[test]
fn completed_artifacts_reuse_and_invalidate_without_caching_failures() {
    let files = Files::new();
    files.write("client.fl", "fn run()->bool {true}");
    let request = files.request("client.fl");
    let mut cache = CompilationCache::new(2);
    let first = cache.compile(&request).unwrap();
    assert!(!first.reused);
    let again = cache.compile(&request).unwrap();
    assert!(again.reused && Arc::ptr_eq(&first.compilation, &again.compilation));
    files.write("client.fl", "fn run()->bool {false}");
    let changed = cache.compile(&request).unwrap();
    assert!(!changed.reused);
    assert_eq!(result(&changed), comptime_interpretor::Value::Bool(false));
    assert_eq!(result(&first), comptime_interpretor::Value::Bool(true));
    files.write("client.fl", "fn run()->bool {missing}");
    assert!(cache.compile(&request).is_err());
    assert!(cache.is_empty());
    files.write("client.fl", "fn run()->bool {true}");
    assert!(!cache.compile(&request).unwrap().reused);
    std::fs::remove_file(files.0.join("client.fl")).unwrap();
    std::fs::create_dir(files.0.join("client.fl")).unwrap();
    assert!(cache.compile(&request).is_err());
    assert!(cache.is_empty());
    std::fs::remove_dir(files.0.join("client.fl")).unwrap();
    files.write("client.fl", "fn run()->bool {true}");
    assert!(!cache.compile(&request).unwrap().reused);
    assert!(cache.compile(&request).unwrap().reused);
}
#[test]
fn discovery_membership_invalidates_but_unrelated_files_do_not() {
    let files = Files::new();
    files.write("lib.fl", "fn run()->bool {true}");
    let request = files.request("lib.fl");
    let mut cache = CompilationCache::new(1);
    cache.compile(&request).unwrap();
    files.write("notes.txt", "not a module");
    assert!(cache.compile(&request).unwrap().reused);
    files.write("added.fl", "pub fn added()->bool {true}");
    assert!(!cache.compile(&request).unwrap().reused);
    assert!(cache.compile(&request).unwrap().reused);
    std::fs::remove_file(files.0.join("added.fl")).unwrap();
    assert!(!cache.compile(&request).unwrap().reused);
    std::fs::create_dir(files.0.join("empty")).unwrap();
    assert!(!cache.compile(&request).unwrap().reused);
    std::fs::remove_dir(files.0.join("empty")).unwrap();
    assert!(!cache.compile(&request).unwrap().reused);
}
#[test]
fn request_keys_and_lru_capacity_bound_reuse() {
    let files = Files::new();
    files.write("entry.fl", "fn run()->bool {true}");
    let a = files.request("entry.fl");
    let mut b = a.clone();
    b.packages[0].identity = 202;
    let mut c = a.clone();
    c.options.emit = luna::cli::Emit::Hir;
    let mut cache = CompilationCache::new(2);
    cache.compile(&a).unwrap();
    let other = cache.compile(&b).unwrap();
    let Artifact::Nir(package) = &other.compilation.artifact else {
        unreachable!()
    };
    assert!(
        package
            .functions
            .iter()
            .all(|f| f.declaration.package == 202)
    );
    assert!(cache.compile(&a).unwrap().reused);
    assert!(!cache.compile(&c).unwrap().reused);
    assert_eq!(cache.len(), 2);
    assert!(cache.compile(&a).unwrap().reused);
    assert!(!cache.compile(&b).unwrap().reused);
    let mut invalid = a.clone();
    invalid.options.tuple_type = Some("missing.Tuple".into());
    assert!(cache.compile(&invalid).is_err());
    assert_eq!(cache.len(), 2);
    cache.clear();
    assert!(cache.is_empty());
    let mut disabled = CompilationCache::new(0);
    assert!(!disabled.compile(&a).unwrap().reused);
    assert!(!disabled.compile(&a).unwrap().reused);
    assert!(disabled.is_empty());
}
#[test]
fn cache_hits_recheck_output_alias_protection() {
    let files = Files::new();
    files.write("entry.fl", "fn run()->bool {true}");
    let mut request = files.request("entry.fl");
    let output = files.0.join("result.lisp");
    request.options.output = Some(output.clone());
    let mut cache = CompilationCache::new(1);
    cache.compile(&request).unwrap();
    std::fs::hard_link(files.0.join("entry.fl"), &output).unwrap();
    assert!(
        cache
            .compile(&request)
            .err()
            .unwrap()
            .contains("output must not refer to source")
    );
    assert!(cache.is_empty());
    assert_eq!(
        std::fs::read_to_string(files.0.join("entry.fl")).unwrap(),
        "fn run()->bool {true}"
    );
    std::fs::remove_file(output).unwrap();
    assert!(!cache.compile(&request).unwrap().reused);
}

#[test]
fn dependency_contents_and_edges_participate_in_reuse() {
    let files = Files::new();
    files.write("client.fl", "use dep.answer;fn run()->bool {answer()}");
    files.write("provider.fl", "pub fn answer()->bool {true}");
    let mut request = files.request("client.fl");
    request.packages.push(luna::packages::SourcePackage {
        resolution_id: 2,
        identity: 202,
        name: "provider".into(),
        entry: files.0.join("provider.fl"),
    });
    request.dependencies.push((1, "dep".into(), 2));
    let mut cache = CompilationCache::new(2);
    let first = cache.compile(&request).unwrap();
    assert_eq!(result(&first), comptime_interpretor::Value::Bool(true));
    assert!(cache.compile(&request).unwrap().reused);
    files.write("provider.fl", "pub fn answer()->bool {false}");
    let changed = cache.compile(&request).unwrap();
    assert!(!changed.reused);
    assert_eq!(result(&changed), comptime_interpretor::Value::Bool(false));
    let mut invalid = request.clone();
    invalid.dependencies.clear();
    assert!(cache.compile(&invalid).is_err());
    assert!(cache.compile(&request).unwrap().reused);
    std::fs::remove_file(files.0.join("provider.fl")).unwrap();
    assert!(cache.compile(&request).is_err());
    assert!(cache.is_empty());
    files.write("provider.fl", "pub fn answer()->bool {true}");
    assert!(!cache.compile(&request).unwrap().reused);
}

#[test]
fn merged_attributes_track_constructor_changes_and_recover_from_invalid_objects() {
    let files = Files::new();
    files.write("client.fl", "^attrs.abi(.c) fn run() {}");
    let constructor = include_str!("../../../library/attrs/lib.fl");
    files.write("attrs.fl", constructor);
    let mut request = files.request("client.fl");
    request.options.emit = luna::cli::Emit::MergedAttributes;
    request.packages.push(luna::packages::SourcePackage {
        resolution_id: 2,
        identity: 202,
        name: "attrs".into(),
        entry: files.0.join("attrs.fl"),
    });
    request.dependencies.push((1, "attrs".into(), 2));
    let mut cache = CompilationCache::new(2);
    let first = cache.compile(&request).unwrap();
    let report = first.compilation.artifact.to_string();
    assert!(report.contains("(key \"abi\") (entry 0)"), "{report}");
    assert!(cache.compile(&request).unwrap().reused);

    files.write(
        "attrs.fl",
        &constructor.replace("{abi: name}", "{abi: name, abi: .changed}"),
    );
    let changed = cache.compile(&request).unwrap();
    assert!(!changed.reused);
    let changed_report = changed.compilation.artifact.to_string();
    assert!(
        changed_report.contains("(key \"abi\") (entry 1)"),
        "{changed_report}"
    );
    assert!(
        changed_report.contains("(symbol \"changed\")"),
        "{changed_report}"
    );
    assert_eq!(first.compilation.artifact.to_string(), report);

    files.write("attrs.fl", &constructor.replace("{abi: name}", "{name}"));
    let failure = cache
        .compile(&request)
        .err()
        .expect("positional metadata must fail merging");
    assert!(failure.contains("positional"), "{failure}");
    assert!(
        cache.is_empty(),
        "a failed replacement must evict the stale selection"
    );
    let mut raw = request.clone();
    raw.options.emit = luna::cli::Emit::Attributes;
    assert!(!cache.compile(&raw).unwrap().reused);
    assert!(cache.compile(&raw).unwrap().reused);
    assert!(
        cache.compile(&request).is_err(),
        "raw inspection cannot satisfy a merged request"
    );

    files.write("attrs.fl", constructor);
    let recovered = cache.compile(&request).unwrap();
    assert!(!recovered.reused);
    assert_eq!(recovered.compilation.artifact.to_string(), report);
    assert!(cache.compile(&request).unwrap().reused);
    assert!(!cache.compile(&raw).unwrap().reused);
    assert!(cache.compile(&raw).unwrap().reused);
}

#[cfg(unix)]
#[test]
fn retargeted_package_entry_invalidates_ownership_even_with_identical_text() {
    use std::os::unix::fs::symlink;
    let files = Files::new();
    std::fs::create_dir(files.0.join("app")).unwrap();
    files.write("app/lib.fl", "fn run()->bool {true}");
    let same = "pub fn identity()->bool {true}";
    files.write("app/a.fl", same);
    files.write("app/b.fl", same);
    let alias = files.0.join("dependency.fl");
    symlink(files.0.join("app/a.fl"), &alias).unwrap();
    let request = package_cli::parse([
        "--package".into(),
        "1".into(),
        "101".into(),
        "app".into(),
        files.0.join("app/lib.fl").into_os_string(),
        "--package".into(),
        "2".into(),
        "202".into(),
        "dependency".into(),
        alias.clone().into_os_string(),
        "--dependency".into(),
        "1".into(),
        "dep".into(),
        "2".into(),
        "--primary".into(),
        "1".into(),
        "--emit".into(),
        "nir".into(),
    ])
    .unwrap();
    let mut cache = CompilationCache::new(1);
    let first = cache.compile(&request).unwrap();
    assert!(cache.compile(&request).unwrap().reused);
    std::fs::remove_file(&alias).unwrap();
    symlink(files.0.join("app/b.fl"), &alias).unwrap();
    let changed = cache.compile(&request).unwrap();
    assert!(
        !changed.reused,
        "a dependency retarget changes ownership exclusions, even when its bytes match"
    );
    let owned = |outcome: &luna::compilation_cache::CompilationOutcome, name: &str| {
        outcome
            .compilation
            .inputs
            .sources
            .iter()
            .any(|s| s.resolution_id == 1 && s.path.ends_with(name))
    };
    assert!(!owned(&first, "a.fl") && owned(&first, "b.fl"));
    assert!(owned(&changed, "a.fl") && !owned(&changed, "b.fl"));
    assert!(cache.compile(&request).unwrap().reused);
}

#[cfg(unix)]
#[test]
fn retargeted_root_rebuilds_and_failed_replacement_is_not_cached() {
    use std::os::unix::fs::symlink;
    let files = Files::new();
    for name in ["old", "new"] {
        std::fs::create_dir(files.0.join(name)).unwrap();
    }
    files.write("old/lib.fl", "fn run()->bool {true}");
    files.write("new/lib.fl", "fn run()->bool {false}");
    let alias = files.0.join("current");
    symlink(files.0.join("old"), &alias).unwrap();
    let request = files.request("current/lib.fl");
    let mut cache = CompilationCache::new(1);
    let first = cache.compile(&request).unwrap();
    assert_eq!(result(&first), comptime_interpretor::Value::Bool(true));
    std::fs::remove_file(&alias).unwrap();
    symlink(files.0.join("new"), &alias).unwrap();
    let changed = cache.compile(&request).unwrap();
    assert!(!changed.reused);
    assert_eq!(result(&changed), comptime_interpretor::Value::Bool(false));
    assert!(cache.compile(&request).unwrap().reused);
    std::fs::remove_file(&alias).unwrap();
    symlink(files.0.join("absent"), &alias).unwrap();
    assert!(cache.compile(&request).is_err());
    assert!(cache.is_empty());
    std::fs::remove_file(&alias).unwrap();
    symlink(files.0.join("old"), &alias).unwrap();
    let recovered = cache.compile(&request).unwrap();
    assert!(!recovered.reused);
    assert_eq!(result(&recovered), comptime_interpretor::Value::Bool(true));
}
