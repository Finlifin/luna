//! Unit & repository integration tests for package-level snapshot caching,
//! parallel SCC wave scheduling, and deterministic SHA-256 fingerprinting.

use elaboration::program::Protocols;
use luna::package_cache::{
    ArtifactKind, BinaryReader, BinaryWriter, CacheStats, CodecError, DiskPackageCache,
    MemoryPackageCache, PackageSnapshotKey, Sha256, UpstreamKeyInputs, compute_unit_merkle_key,
    compute_upstream_key, compute_upstream_key_with_jobs, execute_build_plan_parallel,
    parallel_map_ordered, partition_closed_upstream, schedule_package_waves,
};
use luna::packages::SourcePackage;
use nir::target::TargetData;
use resolve::DependencyEdge;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

fn to_hex(bytes: &[u8; 32]) -> String {
    PackageSnapshotKey(*bytes).to_hex()
}

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("luna-{label}-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn pkg(resolution_id: u32, identity: u64, name: &str, entry: PathBuf) -> SourcePackage {
    SourcePackage {
        resolution_id,
        identity,
        name: name.into(),
        entry,
    }
}

fn edge(requester: u32, alias: &str, provider: u32) -> DependencyEdge {
    DependencyEdge {
        requester,
        alias: alias.into(),
        provider,
    }
}

#[test]
fn sha256_matches_nist_vectors() {
    assert_eq!(
        to_hex(&Sha256::hash(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        to_hex(&Sha256::hash(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        to_hex(&Sha256::hash(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn binary_codec_roundtrips_and_rejects_corruption() {
    let mut writer = BinaryWriter::new();
    writer.write_u8(7);
    writer.write_bool(true);
    writer.write_u32(42);
    writer.write_u64(9001);
    writer.write_array32(&[0xAB; 32]);
    writer.write_str("std.collections.Vec");
    writer.write_bytes(b"\x00\x01\x02");

    let encoded = writer.into_vec();
    let mut reader = BinaryReader::new(&encoded);
    assert_eq!(reader.read_u8().unwrap(), 7);
    assert!(reader.read_bool().unwrap());
    assert_eq!(reader.read_u32().unwrap(), 42);
    assert_eq!(reader.read_u64().unwrap(), 9001);
    assert_eq!(reader.read_array32().unwrap(), [0xAB; 32]);
    assert_eq!(reader.read_str().unwrap(), "std.collections.Vec");
    assert_eq!(reader.read_bytes().unwrap(), b"\x00\x01\x02");
    assert_eq!(reader.remaining(), 0);
    assert_eq!(reader.read_u8(), Err(CodecError::UnexpectedEof));

    let bad_bool = [2u8];
    assert_eq!(
        BinaryReader::new(&bad_bool).read_bool(),
        Err(CodecError::InvalidTag(2))
    );
}

#[test]
fn partition_closed_upstream_handles_sysroot_dag_reverse_dep_and_single_package() {
    // 1. Standard 6-package sysroot clique + user primary(0).
    let sysroot_pkgs: Vec<_> = std::iter::once(pkg(0, 100, "input", PathBuf::from("input.fl")))
        .chain(
            ["builtin", "std", "meta", "build", "attrs", "verification"]
                .into_iter()
                .enumerate()
                .map(|(idx, name)| {
                    pkg(
                        (idx + 1) as u32,
                        (idx + 1) as u64,
                        name,
                        PathBuf::from(format!("library/{name}/lib.fl")),
                    )
                }),
        )
        .collect();
    let mut sysroot_edges = Vec::new();
    for req in 0..=6u32 {
        for prov in 1..=6u32 {
            if req != prov {
                sysroot_edges.push(edge(req, sysroot_pkgs[prov as usize].name.as_str(), prov));
            }
        }
    }
    let part = partition_closed_upstream(&sysroot_pkgs, &sysroot_edges, 0);
    assert_eq!(part.upstream, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(part.downstream, vec![0]);
    assert_eq!(part.upstream_edges.len(), 30);
    assert_eq!(part.cross_and_downstream_edges.len(), 6);

    // 2. Multi-package workspace DAG + peer depending on primary.
    let ws_pkgs = vec![
        pkg(0, 10, "app", PathBuf::from("app/main.fl")),
        pkg(1, 11, "lib_a", PathBuf::from("lib_a/lib.fl")),
        pkg(2, 12, "lib_b", PathBuf::from("lib_b/lib.fl")),
        pkg(9, 19, "tests_peer", PathBuf::from("tests/lib.fl")),
    ];
    let ws_edges = vec![
        edge(0, "lib_a", 1),
        edge(1, "lib_b", 2),
        edge(9, "app", 0),
        edge(9, "lib_b", 2),
    ];
    let part = partition_closed_upstream(&ws_pkgs, &ws_edges, 0);
    assert_eq!(part.upstream, vec![1, 2]);
    assert_eq!(part.downstream, vec![0, 9]);
    assert_eq!(
        part.upstream_edges
            .iter()
            .map(|e| (e.requester, e.alias.as_str(), e.provider))
            .collect::<Vec<_>>(),
        vec![(1, "lib_b", 2)]
    );

    // 3. Single standalone package.
    let solo = vec![pkg(0, 1, "input", PathBuf::from("input.fl"))];
    let solo_part = partition_closed_upstream(&solo, &[], 0);
    assert!(solo_part.upstream.is_empty());
    assert_eq!(solo_part.downstream, vec![0]);
}

#[test]
fn schedule_package_waves_groups_sysroot_scc_and_parallelizes_independent_diamond_branches() {
    // Diamond + mutual cycle: {1, 2} form a cyclic base (wave 0),
    // 3 -> {1, 2} and 4 -> 1 run together in wave 1,
    // 5 -> {3, 4} runs in wave 2, and standalone 6 runs in wave 0 alongside {1, 2}.
    let pkgs = vec![
        pkg(1, 1, "builtin", PathBuf::from("b.fl")),
        pkg(2, 2, "std", PathBuf::from("s.fl")),
        pkg(3, 3, "left", PathBuf::from("l.fl")),
        pkg(4, 4, "right", PathBuf::from("r.fl")),
        pkg(5, 5, "app", PathBuf::from("a.fl")),
        pkg(6, 6, "util", PathBuf::from("u.fl")),
    ];
    let edges = vec![
        edge(1, "std", 2),
        edge(2, "builtin", 1),
        edge(3, "builtin", 1),
        edge(3, "std", 2),
        edge(4, "builtin", 1),
        edge(5, "left", 3),
        edge(5, "right", 4),
    ];
    let plan = schedule_package_waves(&pkgs, &edges);
    assert_eq!(plan.waves.len(), 3, "{plan:#?}");
    // Wave 0 has unit {1, 2} and standalone unit {6}.
    assert_eq!(plan.waves[0].len(), 2);
    assert_eq!(plan.units[plan.waves[0][0]].packages, vec![1, 2]);
    assert_eq!(plan.units[plan.waves[0][1]].packages, vec![6]);
    // Wave 1 has independent diamond branches {3} and {4}.
    assert_eq!(plan.waves[1].len(), 2);
    assert_eq!(plan.units[plan.waves[1][0]].packages, vec![3]);
    assert_eq!(plan.units[plan.waves[1][1]].packages, vec![4]);
    // Wave 2 has consumer {5}.
    assert_eq!(plan.waves[2].len(), 1);
    assert_eq!(plan.units[plan.waves[2][0]].packages, vec![5]);
}

#[test]
fn execute_build_plan_parallel_runs_wave_concurrently_preserves_order_and_stops_on_wave_failure() {
    // Three independent root units in wave 0, plus one dependent consumer in wave 1.
    let pkgs = vec![
        pkg(1, 1, "a", PathBuf::from("a.fl")),
        pkg(2, 2, "b", PathBuf::from("b.fl")),
        pkg(3, 3, "c", PathBuf::from("c.fl")),
        pkg(4, 4, "consumer", PathBuf::from("consumer.fl")),
    ];
    let edges = vec![edge(4, "a", 1), edge(4, "b", 2), edge(4, "c", 3)];
    let plan = schedule_package_waves(&pkgs, &edges);
    assert_eq!(plan.waves.len(), 2);
    assert_eq!(plan.waves[0].len(), 3);

    // 1. Prove Wave 0 genuinely runs all 3 units concurrently via a Barrier!
    let barrier = Barrier::new(3);
    let par_results = execute_build_plan_parallel(&plan, 3, |unit, deps| -> Result<u32, String> {
        if unit.wave == 0 {
            barrier.wait();
            Ok(unit.packages[0] * 10)
        } else {
            Ok(deps.iter().copied().copied().sum::<u32>() + unit.packages[0])
        }
    })
    .unwrap();
    let seq_results = execute_build_plan_parallel(&plan, 1, |unit, deps| -> Result<u32, String> {
        if unit.wave == 0 {
            Ok(unit.packages[0] * 10)
        } else {
            Ok(deps.iter().copied().copied().sum::<u32>() + unit.packages[0])
        }
    })
    .unwrap();
    assert_eq!(par_results, vec![10, 20, 30, 64]);
    assert_eq!(par_results, seq_results);

    // 2. Failing wave 0 units return deterministic first error and never execute wave 1!
    let wave1_ran = std::sync::atomic::AtomicBool::new(false);
    let err = execute_build_plan_parallel(&plan, 4, |unit, _deps| {
        if unit.wave == 1 {
            wave1_ran.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        match unit.packages[0] {
            2 => Err("err-unit-2"),
            3 => Err("err-unit-3"),
            n => Ok(n),
        }
    })
    .unwrap_err();
    assert_eq!(err, "err-unit-2");
    assert!(!wave1_ran.load(std::sync::atomic::Ordering::SeqCst));

    // Also test parallel_map_ordered directly.
    let items: Vec<usize> = (0..16).collect();
    let doubled = parallel_map_ordered(&items, 4, |n| Ok::<_, ()>(n * 2)).unwrap();
    assert_eq!(doubled, (0..16).map(|n| n * 2).collect::<Vec<_>>());
}

#[test]
fn upstream_key_is_stable_across_downstream_edits_and_invalidates_on_any_upstream_change() {
    let root = temp_dir("pkg-cache-invalidation");
    let user_dir = root.join("user");
    let dep_a_dir = root.join("dep_a");
    let dep_b_dir = root.join("dep_b");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::create_dir_all(&dep_a_dir).unwrap();
    std::fs::create_dir_all(&dep_b_dir).unwrap();

    let user_main = user_dir.join("main.fl");
    let dep_a_lib = dep_a_dir.join("lib.fl");
    let dep_b_lib = dep_b_dir.join("lib.fl");
    let config_file = root.join("language-items.conf");

    std::fs::write(&user_main, "fn main() -> bool { true }").unwrap();
    std::fs::write(&dep_a_lib, "pub fn answer() -> u32 { 42 }").unwrap();
    std::fs::write(&dep_b_lib, "pub fn flag() -> bool { true }").unwrap();
    std::fs::write(&config_file, "version = 1\n").unwrap();

    let packages = vec![
        pkg(0, 100, "user", user_main.clone()),
        pkg(1, 1, "dep_a", dep_a_lib.clone()),
        pkg(2, 2, "dep_b", dep_b_lib.clone()),
    ];
    let upstream_edges = vec![edge(1, "dep_b", 2)];
    let mut protocols = Protocols::default();
    protocols.build_modes.insert(1, "debug".into());
    protocols.build_modes.insert(100, "debug".into());

    let target64 = TargetData::new(64).unwrap();
    let target32 = TargetData::new(32).unwrap();
    let extra_configs = vec![config_file.clone()];

    let compute_key = |prots: &Protocols,
                       tgt: Option<TargetData>,
                       compiler_tag: [u8; 32],
                       selector_tag: &[u8]| {
        compute_upstream_key(&UpstreamKeyInputs {
            all_packages: &packages,
            upstream_ids: &[1, 2],
            upstream_edges: &upstream_edges,
            builtin: Some(1),
            preludes: &[1],
            extra_config_files: &extra_configs,
            target: tgt,
            protocols: prots,
            selector_tag,
            compiler_override: Some(compiler_tag),
        })
        .unwrap()
    };

    let compiler_v1 = [1u8; 32];
    let compiler_v2 = [2u8; 32];
    let (base_key, manifest) = compute_key(&protocols, Some(target64), compiler_v1, b"sel-v1");
    assert_eq!(manifest.packages.len(), 2);

    // 1. Downstream source edits + downstream-only build.mode edits MUST NOT invalidate upstream!
    std::fs::write(&user_main, "fn main() -> bool { false }\nfn extra() {}").unwrap();
    let mut downstream_mode_changed = protocols.clone();
    downstream_mode_changed
        .build_modes
        .insert(100, "release".into());
    let (after_downstream, _) = compute_key(
        &downstream_mode_changed,
        Some(target64),
        compiler_v1,
        b"sel-v1",
    );
    assert_eq!(after_downstream, base_key);

    // 2. Upstream source byte change MUST invalidate.
    std::fs::write(&dep_a_lib, "pub fn answer() -> u32 { 43 }").unwrap();
    let (after_src_edit, _) = compute_key(&protocols, Some(target64), compiler_v1, b"sel-v1");
    assert_ne!(after_src_edit, base_key);

    // 3. Adding a new .fl file inside dep_a MUST invalidate.
    std::fs::write(
        dep_a_dir.join("extra.fl"),
        "pub fn extra() -> bool { true }",
    )
    .unwrap();
    let (after_new_file, _) = compute_key(&protocols, Some(target64), compiler_v1, b"sel-v1");
    assert_ne!(after_new_file, after_src_edit);

    // 4. Editing language-items.conf MUST invalidate.
    std::fs::write(&config_file, "version = 1\n# changed\n").unwrap();
    let (after_cfg, _) = compute_key(&protocols, Some(target64), compiler_v1, b"sel-v1");
    assert_ne!(after_cfg, after_new_file);

    // 5. Target pointer width change MUST invalidate.
    let (after_target, _) = compute_key(&protocols, Some(target32), compiler_v1, b"sel-v1");
    assert_ne!(after_target, after_cfg);

    // 6. Upstream build.mode change MUST invalidate.
    let mut upstream_mode_changed = protocols.clone();
    upstream_mode_changed
        .build_modes
        .insert(1, "release".into());
    let (after_up_mode, _) = compute_key(
        &upstream_mode_changed,
        Some(target64),
        compiler_v1,
        b"sel-v1",
    );
    assert_ne!(after_up_mode, after_cfg);

    // 7. Compiler binary fingerprint change MUST invalidate.
    let (after_compiler, _) = compute_key(&protocols, Some(target64), compiler_v2, b"sel-v1");
    assert_ne!(after_compiler, after_cfg);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn upstream_key_parallel_hashing_matches_sequential_on_real_repository_sysroot() {
    let repo_library = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let temp_user = temp_dir("sysroot-fingerprint-user");
    let user_file = temp_user.join("main.fl");
    std::fs::write(&user_file, "fn run() -> bool { true }").unwrap();

    let mut packages = vec![pkg(0, 100, "input", user_file)];
    let names = ["builtin", "std", "meta", "build", "attrs", "verification"];
    for (idx, name) in names.iter().enumerate() {
        packages.push(pkg(
            (idx + 1) as u32,
            (idx + 1) as u64,
            name,
            repo_library.join(name).join("lib.fl"),
        ));
    }
    let mut upstream_edges = Vec::new();
    for req in 1..=6u32 {
        for prov in 1..=6u32 {
            if req != prov {
                upstream_edges.push(edge(req, names[(prov - 1) as usize], prov));
            }
        }
    }

    let extra_configs = vec![repo_library.join("language-items.conf")];
    let protocols = Protocols::default();
    let target = TargetData::new(64).unwrap();
    let inputs = UpstreamKeyInputs {
        all_packages: &packages,
        upstream_ids: &[1, 2, 3, 4, 5, 6],
        upstream_edges: &upstream_edges,
        builtin: Some(1),
        preludes: &[1, 3, 4, 5, 6],
        extra_config_files: &extra_configs,
        target: Some(target),
        protocols: &protocols,
        selector_tag: b"default-sysroot",
        compiler_override: None,
    };

    let (seq_key, seq_manifest) = compute_upstream_key_with_jobs(&inputs, 1).unwrap();
    let (par_key, par_manifest) = compute_upstream_key_with_jobs(&inputs, 6).unwrap();
    assert_eq!(seq_key, par_key);
    assert_eq!(seq_manifest, par_manifest);
    assert_eq!(seq_manifest.packages.len(), 6);
    assert!(
        seq_manifest
            .packages
            .iter()
            .all(|entry| !entry.files.is_empty())
    );

    let _ = std::fs::remove_dir_all(temp_user);
}

#[test]
fn unit_merkle_key_propagates_upstream_dependency_changes() {
    let root = temp_dir("unit-merkle-key");
    let entry_a = root.join("a.fl");
    std::fs::write(&entry_a, "fn a() -> bool { true }").unwrap();
    let packages = vec![pkg(1, 1, "a", entry_a)];
    let protocols = Protocols::default();
    let inputs = UpstreamKeyInputs {
        all_packages: &packages,
        upstream_ids: &[1],
        upstream_edges: &[],
        builtin: None,
        preludes: &[],
        extra_config_files: &[],
        target: None,
        protocols: &protocols,
        selector_tag: b"",
        compiler_override: Some([9u8; 32]),
    };

    let dep_k1 = PackageSnapshotKey([0x11; 32]);
    let dep_k2 = PackageSnapshotKey([0x22; 32]);
    let (key1, _) = compute_unit_merkle_key(&inputs, &[(0, dep_k1)], 2).unwrap();
    let (key1_again, _) = compute_unit_merkle_key(&inputs, &[(0, dep_k1)], 1).unwrap();
    let (key2, _) = compute_unit_merkle_key(&inputs, &[(0, dep_k2)], 2).unwrap();
    assert_eq!(key1, key1_again);
    assert_ne!(key1, key2);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn disk_package_cache_roundtrip_and_corruption_rejection() {
    let dir = temp_dir("disk-pkg-cache");
    let cache = DiskPackageCache::new(dir.clone());
    let key = PackageSnapshotKey(Sha256::hash(b"sysroot-base-v1"));
    let other_key = PackageSnapshotKey(Sha256::hash(b"sysroot-base-v2"));
    let payload = b"(elaborated-base (items 42) (verified-upstream 7))";

    assert_eq!(cache.load(&key, ArtifactKind::ElaboratedBase), None);

    let stored_path = cache
        .store(&key, ArtifactKind::ElaboratedBase, payload)
        .unwrap()
        .expect("enabled cache returns file path");
    assert!(stored_path.is_file());

    // Exact load succeeds.
    assert_eq!(
        cache.load(&key, ArtifactKind::ElaboratedBase).as_deref(),
        Some(payload.as_slice())
    );
    // Mismatched ArtifactKind cleanly misses.
    assert_eq!(cache.load(&key, ArtifactKind::ResolvedBase), None);
    // Mismatched key cleanly misses.
    assert_eq!(cache.load(&other_key, ArtifactKind::ElaboratedBase), None);

    // Bit-flip corruption in payload is caught by SHA-256 verification.
    let mut corrupted = std::fs::read(&stored_path).unwrap();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xFF;
    std::fs::write(&stored_path, &corrupted).unwrap();
    assert_eq!(cache.load(&key, ArtifactKind::ElaboratedBase), None);

    // Truncated cache file is safely rejected.
    corrupted.truncate(20);
    std::fs::write(&stored_path, &corrupted).unwrap();
    assert_eq!(cache.load(&key, ArtifactKind::ElaboratedBase), None);

    // Disabled cache performs no I/O.
    let disabled = DiskPackageCache::disabled();
    assert_eq!(
        disabled
            .store(&key, ArtifactKind::ElaboratedBase, payload)
            .unwrap(),
        None
    );
    assert_eq!(disabled.load(&key, ArtifactKind::ElaboratedBase), None);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn memory_package_cache_lru_hits_eviction_and_stats() {
    let cache: MemoryPackageCache<String> = MemoryPackageCache::new(2);
    let k1 = PackageSnapshotKey([1u8; 32]);
    let k2 = PackageSnapshotKey([2u8; 32]);
    let k3 = PackageSnapshotKey([3u8; 32]);
    let kind = ArtifactKind::ElaboratedBase;

    assert!(cache.is_empty());
    assert_eq!(cache.get(k1, kind), None);

    cache.insert(k1, kind, Arc::new("one".into()));
    cache.insert(k2, kind, Arc::new("two".into()));
    assert_eq!(cache.len(), 2);

    // Touch k1 so k2 becomes least-recently-used.
    assert_eq!(cache.get(k1, kind).as_deref(), Some(&"one".to_string()));
    cache.insert(k3, kind, Arc::new("three".into()));

    assert_eq!(cache.len(), 2);
    assert_eq!(cache.get(k2, kind), None);
    assert_eq!(cache.get(k1, kind).as_deref(), Some(&"one".to_string()));
    assert_eq!(cache.get(k3, kind).as_deref(), Some(&"three".to_string()));
    assert_eq!(cache.stats(), CacheStats { hits: 3, misses: 2 });

    cache.clear();
    assert!(cache.is_empty());
}

#[test]
fn resolved_source_ids_collect_deterministically() {
    let ids: BTreeSet<u32> = [3, 1, 2].into_iter().collect();
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec![1, 2, 3]);
}
