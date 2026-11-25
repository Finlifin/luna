//! Content-addressed package snapshot caching, closed-upstream partitioning,
//! and parallel SCC wave scheduling for package-level incremental builds.
//!
//! This module provides Layer 1 of package-level incremental compilation:
//! - Deterministic FIPS 180-4 SHA-256 fingerprinting (no external crate deps)
//! - Checked length-bounded binary encoding helpers for `.flpkg` snapshots
//! - Closed-upstream DAG partitioning (`partition_closed_upstream`)
//! - Tarjan SCC condensation + topological wave scheduling (`schedule_package_waves`)
//! - Scoped multi-threaded wave & source-hashing execution (`parallel_map_ordered`,
//!   `execute_build_plan_parallel`) with deterministic output ordering and first-error
//!   reporting regardless of worker thread count
//! - Two-tier storage: thread-safe in-process LRU (`MemoryPackageCache`) and
//!   atomic content-addressed `.flpkg` disk cache (`DiskPackageCache`).

use crate::packages::SourcePackage;
use lex::Symbol;
use parse::ast::{Ast, NodeKind};
use rustc_span::{BytePos, DUMMY_SP, Span};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const MAX_CODEC_BYTES: usize = 256 * 1024 * 1024;

const MAX_CODEC_ELEMENTS: usize = MAX_CODEC_BYTES / 4;

const DISK_MAGIC: [u8; 4] = *b"FLPK";

const DISK_FORMAT_VERSION: u32 = 1;

const KEY_DOMAIN_V1: &[u8] = b"flurry-pkg-snapshot-key-v1";

const PARSED_PKG_KEY_DOMAIN_V1: &[u8] = b"flurry-parsed-pkg-v1";

const PARSED_PKG_CODEC_VERSION: u32 = 1;

const VERIFIED_UPSTREAM_CODEC_VERSION: u32 = 1;

const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Deterministic FIPS 180-4 SHA-256
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0u8; 64],
            buffer_len: 0,
            total_len: 0,
        }
    }

    pub fn hash(data: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self
            .total_len
            .checked_add(data.len() as u64)
            .expect("sha256 total byte length overflow");
        if self.buffer_len > 0 {
            let needed = 64 - self.buffer_len;
            if data.len() < needed {
                self.buffer[self.buffer_len..self.buffer_len + data.len()].copy_from_slice(data);
                self.buffer_len += data.len();
                return;
            }
            self.buffer[self.buffer_len..64].copy_from_slice(&data[..needed]);
            let block = self.buffer;
            compress_block(&mut self.state, &block);
            self.buffer_len = 0;
            data = &data[needed..];
        }
        while data.len() >= 64 {
            let (block, rest) = data.split_at(64);
            let block: &[u8; 64] = block.try_into().expect("64-byte slice");
            compress_block(&mut self.state, block);
            data = rest;
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    pub fn update_len_prefixed(&mut self, bytes: &[u8]) {
        self.update(&(bytes.len() as u64).to_le_bytes());
        self.update(bytes);
    }

    pub fn update_str(&mut self, text: &str) {
        self.update_len_prefixed(text.as_bytes());
    }

    pub fn update_u32(&mut self, value: u32) {
        self.update(&value.to_le_bytes());
    }

    pub fn update_u64(&mut self, value: u64) {
        self.update(&value.to_le_bytes());
    }

    pub fn update_bool(&mut self, value: bool) {
        self.update(&[u8::from(value)]);
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let total_bits = self.total_len.wrapping_mul(8);
        self.update(&[0x80]);
        if self.buffer_len > 56 {
            self.buffer[self.buffer_len..64].fill(0);
            let block = self.buffer;
            compress_block(&mut self.state, &block);
            self.buffer_len = 0;
        }
        self.buffer[self.buffer_len..56].fill(0);
        self.buffer[56..64].copy_from_slice(&total_bits.to_be_bytes());
        let final_block = self.buffer;
        compress_block(&mut self.state, &final_block);

        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

fn compress_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in w.iter_mut().take(16).enumerate() {
        let base = i * 4;
        *word = u32::from_be_bytes([
            block[base],
            block[base + 1],
            block[base + 2],
            block[base + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

// ---------------------------------------------------------------------------
// Checked Binary Codec
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    UnexpectedEof,
    InvalidUtf8,
    LengthLimitExceeded,
    InvalidTag(u8),
    InvalidStructure(&'static str),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => f.write_str("unexpected end of package cache stream"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8 string in package cache stream"),
            Self::LengthLimitExceeded => {
                f.write_str("slice length exceeds maximum allowed snapshot bound")
            }
            Self::InvalidTag(tag) => write!(f, "invalid enum tag {tag} in package cache stream"),
            Self::InvalidStructure(reason) => {
                write!(f, "invalid package snapshot structure: {reason}")
            }
        }
    }
}

impl std::error::Error for CodecError {}

#[derive(Clone, Debug, Default)]
pub struct BinaryWriter {
    buf: Vec<u8>,
}

impl BinaryWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn write_u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    pub fn write_u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_array32(&mut self, value: &[u8; 32]) {
        self.buf.extend_from_slice(value);
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_u64(bytes.len() as u64);
        self.buf.extend_from_slice(bytes);
    }

    pub fn write_str(&mut self, text: &str) {
        self.write_bytes(text.as_bytes());
    }
}

pub struct BinaryReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> BinaryReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(CodecError::LengthLimitExceeded)?;
        if end > self.data.len() {
            return Err(CodecError::UnexpectedEof);
        }
        let slice = &self.data[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    pub fn read_u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }

    pub fn read_bool(&mut self) -> Result<bool, CodecError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(CodecError::InvalidTag(other)),
        }
    }

    pub fn read_u32(&mut self) -> Result<u32, CodecError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| CodecError::UnexpectedEof)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn read_u64(&mut self) -> Result<u64, CodecError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| CodecError::UnexpectedEof)?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub fn read_array32(&mut self) -> Result<[u8; 32], CodecError> {
        self.take(32)?
            .try_into()
            .map_err(|_| CodecError::UnexpectedEof)
    }

    pub fn read_bytes(&mut self) -> Result<&'a [u8], CodecError> {
        let raw_len = self.read_u64()?;
        let len = usize::try_from(raw_len).map_err(|_| CodecError::LengthLimitExceeded)?;
        if len > MAX_CODEC_BYTES {
            return Err(CodecError::LengthLimitExceeded);
        }
        self.take(len)
    }

    pub fn read_str(&mut self) -> Result<&'a str, CodecError> {
        let bytes = self.read_bytes()?;
        std::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)
    }
}

// ---------------------------------------------------------------------------
// Deterministic Parallel Execution Primitives
// ---------------------------------------------------------------------------

/// Determine worker concurrency from explicit override, `LUNA_JOBS`, or host CPU count.
pub fn resolve_jobs(explicit: Option<usize>) -> usize {
    explicit
        .filter(|&jobs| jobs > 0)
        .or_else(|| {
            std::env::var("LUNA_JOBS")
                .ok()
                .and_then(|raw| raw.trim().parse::<usize>().ok())
                .filter(|&jobs| jobs > 0)
        })
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
        })
        .clamp(1, 64)
}

/// Execute `f` over `items` with up to `jobs` scoped worker threads while
/// guaranteeing output order and returning the deterministic earliest-index error.
pub fn parallel_map_ordered<T, R, E, F>(items: &[T], jobs: usize, f: F) -> Result<Vec<R>, E>
where
    T: Sync,
    R: Send,
    E: Send,
    F: Fn(&T) -> Result<R, E> + Sync,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }
    if jobs <= 1 || items.len() == 1 {
        let mut results = Vec::with_capacity(items.len());
        for item in items {
            results.push(f(item)?);
        }
        return Ok(results);
    }

    let worker_count = jobs.min(items.len()).max(1);
    let next_index = AtomicUsize::new(0);
    let earliest_error = AtomicUsize::new(usize::MAX);
    let slots: Vec<Mutex<Option<Result<R, E>>>> =
        (0..items.len()).map(|_| Mutex::new(None)).collect();

    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let builder = std::thread::Builder::new().stack_size(WORKER_STACK_BYTES);
            let handle = builder
                .spawn_scoped(scope, || {
                    loop {
                        let idx = next_index.fetch_add(1, Ordering::Relaxed);
                        if idx >= items.len() {
                            break;
                        }
                        if idx > earliest_error.load(Ordering::Acquire) {
                            break;
                        }
                        let outcome = f(&items[idx]);
                        let failed = outcome.is_err();
                        *slots[idx].lock().expect("parallel slot lock poisoned") = Some(outcome);
                        if failed {
                            earliest_error.fetch_min(idx, Ordering::Release);
                        }
                    }
                })
                .expect("failed to spawn package-cache worker thread");
            handles.push(handle);
        }
        for handle in handles {
            handle.join().expect("package-cache worker thread panicked");
        }
    });

    let mut ordered = Vec::with_capacity(items.len());
    for slot in slots {
        let item = slot
            .into_inner()
            .expect("parallel slot lock poisoned")
            .expect("completed item slot must be populated unless an earlier error aborted it");
        ordered.push(item?);
    }
    Ok(ordered)
}

// ---------------------------------------------------------------------------
// Closed-Upstream Partitioner & Tarjan SCC Parallel Wave Scheduler
// ---------------------------------------------------------------------------

/// Result of splitting a package graph around `primary` into an upward-closed
/// upstream set `B` (zero transitive dependencies on `primary`) and downstream `D`.
#[derive(Clone, Debug)]
pub struct UpstreamPartition {
    pub upstream: Vec<u32>,
    pub downstream: Vec<u32>,
    pub upstream_edges: Vec<resolve::DependencyEdge>,
    pub cross_and_downstream_edges: Vec<resolve::DependencyEdge>,
}

/// Partition `packages` into `(upstream, downstream)` around `primary`.
///
/// Any package that is `primary` or transitively depends on `primary` belongs to
/// `downstream`. All remaining packages have no dependency path to `primary`
/// or `downstream` and form an upward-closed cacheable upstream base.
pub fn partition_closed_upstream(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
    primary: u32,
) -> UpstreamPartition {
    let mut reverse_deps: HashMap<u32, Vec<u32>> = HashMap::new();
    for edge in dependencies {
        reverse_deps
            .entry(edge.provider)
            .or_default()
            .push(edge.requester);
    }

    let mut downstream_set = BTreeSet::new();
    let mut queue = VecDeque::new();
    if packages.iter().any(|p| p.resolution_id == primary) {
        downstream_set.insert(primary);
        queue.push_back(primary);
    }
    while let Some(node) = queue.pop_front() {
        if let Some(dependents) = reverse_deps.get(&node) {
            for &dependent in dependents {
                if downstream_set.insert(dependent) {
                    queue.push_back(dependent);
                }
            }
        }
    }

    let mut upstream = Vec::new();
    let mut downstream = Vec::new();
    for package in packages {
        if downstream_set.contains(&package.resolution_id) {
            downstream.push(package.resolution_id);
        } else {
            upstream.push(package.resolution_id);
        }
    }

    let mut upstream_edges = Vec::new();
    let mut cross_and_downstream_edges = Vec::new();
    for edge in dependencies {
        if !downstream_set.contains(&edge.requester) && !downstream_set.contains(&edge.provider) {
            upstream_edges.push(edge.clone());
        } else {
            debug_assert!(
                downstream_set.contains(&edge.requester),
                "upstream package {} cannot depend on downstream package {}",
                edge.requester,
                edge.provider
            );
            cross_and_downstream_edges.push(edge.clone());
        }
    }

    UpstreamPartition {
        upstream,
        downstream,
        upstream_edges,
        cross_and_downstream_edges,
    }
}

/// One strongly connected component (often a single package, or a mutually
/// importing group like the 6 sysroot packages) scheduled as an atomic build unit.
#[derive(Clone, Debug)]
pub struct PackageUnit {
    pub unit_id: usize,
    pub packages: Vec<u32>,
    pub internal_edges: Vec<resolve::DependencyEdge>,
    pub dependency_units: Vec<usize>,
    pub wave: usize,
}

/// Condensed DAG of `PackageUnit`s grouped into parallel execution waves.
#[derive(Clone, Debug)]
pub struct PackageBuildPlan {
    pub units: Vec<PackageUnit>,
    pub waves: Vec<Vec<usize>>,
}

/// Condense `(packages, dependencies)` via Tarjan's SCC algorithm and assign
/// deterministic topological waves so independent units run in parallel.
pub fn schedule_package_waves(
    packages: &[SourcePackage],
    dependencies: &[resolve::DependencyEdge],
) -> PackageBuildPlan {
    let mut nodes: Vec<u32> = packages.iter().map(|p| p.resolution_id).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let n = nodes.len();
    if n == 0 {
        return PackageBuildPlan {
            units: Vec::new(),
            waves: Vec::new(),
        };
    }

    let index_by_id: HashMap<u32, usize> =
        nodes.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut adj = vec![Vec::<usize>::new(); n];
    for edge in dependencies {
        if let (Some(&u), Some(&v)) = (
            index_by_id.get(&edge.requester),
            index_by_id.get(&edge.provider),
        ) {
            adj[u].push(v);
        }
    }
    for list in &mut adj {
        list.sort_unstable();
        list.dedup();
    }

    struct Tarjan<'a> {
        adj: &'a [Vec<usize>],
        next_index: usize,
        indices: Vec<Option<usize>>,
        lowlink: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        sccs: Vec<Vec<usize>>,
    }

    impl<'a> Tarjan<'a> {
        fn visit(&mut self, v: usize) {
            let idx = self.next_index;
            self.next_index += 1;
            self.indices[v] = Some(idx);
            self.lowlink[v] = idx;
            self.stack.push(v);
            self.on_stack[v] = true;

            for &w in &self.adj[v] {
                if self.indices[w].is_none() {
                    self.visit(w);
                    self.lowlink[v] = self.lowlink[v].min(self.lowlink[w]);
                } else if self.on_stack[w] {
                    self.lowlink[v] = self.lowlink[v].min(self.indices[w].unwrap());
                }
            }

            if self.lowlink[v] == idx {
                let mut component = Vec::new();
                while let Some(w) = self.stack.pop() {
                    self.on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                component.sort_unstable();
                self.sccs.push(component);
            }
        }
    }

    let mut tarjan = Tarjan {
        adj: &adj,
        next_index: 0,
        indices: vec![None; n],
        lowlink: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        sccs: Vec::new(),
    };
    for v in 0..n {
        if tarjan.indices[v].is_none() {
            tarjan.visit(v);
        }
    }

    let scc_count = tarjan.sccs.len();
    let mut node_to_raw_scc = vec![0usize; n];
    for (scc_idx, component) in tarjan.sccs.iter().enumerate() {
        for &node in component {
            node_to_raw_scc[node] = scc_idx;
        }
    }

    let mut raw_deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); scc_count];
    for (u, neighbors) in adj.iter().enumerate() {
        let su = node_to_raw_scc[u];
        for &v in neighbors {
            let sv = node_to_raw_scc[v];
            if su != sv {
                raw_deps[su].insert(sv);
            }
        }
    }

    // In Tarjan's post-order, a dependency `sv` of `su` always appears before `su`.
    let mut raw_waves = vec![0usize; scc_count];
    for su in 0..scc_count {
        let wave = raw_deps[su]
            .iter()
            .map(|&sv| raw_waves[sv] + 1)
            .max()
            .unwrap_or(0);
        raw_waves[su] = wave;
    }

    let mut order: Vec<usize> = (0..scc_count).collect();
    order.sort_by_key(|&raw_id| (raw_waves[raw_id], nodes[tarjan.sccs[raw_id][0]]));

    let mut raw_to_unit = vec![0usize; scc_count];
    for (unit_id, &raw_id) in order.iter().enumerate() {
        raw_to_unit[raw_id] = unit_id;
    }

    let mut pkg_to_unit: HashMap<u32, usize> = HashMap::new();
    for (unit_id, &raw_id) in order.iter().enumerate() {
        for &node in &tarjan.sccs[raw_id] {
            pkg_to_unit.insert(nodes[node], unit_id);
        }
    }

    let max_wave = order.iter().map(|&r| raw_waves[r]).max().unwrap_or(0);
    let mut waves = vec![Vec::new(); max_wave + 1];
    let mut units = Vec::with_capacity(scc_count);

    for (unit_id, &raw_id) in order.iter().enumerate() {
        let wave = raw_waves[raw_id];
        waves[wave].push(unit_id);

        let pkg_ids: Vec<u32> = tarjan.sccs[raw_id].iter().map(|&idx| nodes[idx]).collect();
        let mut dependency_units: Vec<usize> =
            raw_deps[raw_id].iter().map(|&sv| raw_to_unit[sv]).collect();
        dependency_units.sort_unstable();
        dependency_units.dedup();

        let mut internal_edges: Vec<resolve::DependencyEdge> = dependencies
            .iter()
            .filter(|edge| {
                pkg_to_unit.get(&edge.requester) == Some(&unit_id)
                    && pkg_to_unit.get(&edge.provider) == Some(&unit_id)
            })
            .cloned()
            .collect();
        internal_edges.sort_by(|a, b| {
            (a.requester, &a.alias, a.provider).cmp(&(b.requester, &b.alias, b.provider))
        });

        units.push(PackageUnit {
            unit_id,
            packages: pkg_ids,
            internal_edges,
            dependency_units,
            wave,
        });
    }

    PackageBuildPlan { units, waves }
}

/// Execute every wave in `plan` in topological order, running units within the
/// same wave in parallel with up to `jobs` worker threads.
///
/// If any unit in wave `k` returns an error, wave `k + 1` is never launched and
/// the deterministic first-failing unit's error in wave `k` is returned.
pub fn execute_build_plan_parallel<R, E, F>(
    plan: &PackageBuildPlan,
    jobs: usize,
    execute_unit: F,
) -> Result<Vec<R>, E>
where
    R: Send + Sync,
    E: Send,
    F: Fn(&PackageUnit, &[&R]) -> Result<R, E> + Sync,
{
    let mut completed: Vec<Option<R>> = (0..plan.units.len()).map(|_| None).collect();
    for wave_units in &plan.waves {
        let wave_results = parallel_map_ordered(wave_units, jobs, |&unit_id| {
            let unit = &plan.units[unit_id];
            let mut dep_refs = Vec::with_capacity(unit.dependency_units.len());
            for &dep_id in &unit.dependency_units {
                let dep_out = completed[dep_id]
                    .as_ref()
                    .expect("dependency unit must finish in an earlier wave");
                dep_refs.push(dep_out);
            }
            execute_unit(unit, &dep_refs)
        })?;

        for (&unit_id, result) in wave_units.iter().zip(wave_results) {
            completed[unit_id] = Some(result);
        }
    }

    Ok(completed
        .into_iter()
        .map(|slot| slot.expect("every scheduled unit must complete"))
        .collect())
}

// ---------------------------------------------------------------------------
// Content-Addressed Package Snapshot Key & Parallel Fingerprinting
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageSnapshotKey(pub [u8; 32]);

impl PackageSnapshotKey {
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            use std::fmt::Write;
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }

    pub fn prefix_dir(self) -> String {
        format!("{:02x}", self.0[0])
    }
}

impl fmt::Debug for PackageSnapshotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PackageSnapshotKey({})", self.to_hex())
    }
}

impl fmt::Display for PackageSnapshotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Return a deterministic SHA-256 fingerprint for the currently running compiler
/// binary (falling back to build metadata only if the executable cannot be read).
fn read_elf_gnu_build_id(path: &Path) -> Option<Vec<u8>> {
    use std::io::Seek;

    const PT_NOTE: u32 = 4;
    const NT_GNU_BUILD_ID: u32 = 3;
    const MAX_PHDR_BYTES: usize = 64 * 1024;
    const MAX_NOTE_BYTES: usize = 64 * 1024;

    let mut file = File::open(path).ok()?;
    let mut ehdr = [0u8; 64];
    file.read_exact(&mut ehdr).ok()?;
    if &ehdr[0..4] != b"\x7fELF" {
        return None;
    }
    let ei_class = ehdr[4];
    let little_endian = match ehdr[5] {
        1 => true,
        2 => false,
        _ => return None,
    };
    let read_u16 = |b: &[u8]| -> u16 {
        let arr: [u8; 2] = b[..2].try_into().unwrap();
        if little_endian {
            u16::from_le_bytes(arr)
        } else {
            u16::from_be_bytes(arr)
        }
    };
    let read_u32 = |b: &[u8]| -> u32 {
        let arr: [u8; 4] = b[..4].try_into().unwrap();
        if little_endian {
            u32::from_le_bytes(arr)
        } else {
            u32::from_be_bytes(arr)
        }
    };
    let read_u64 = |b: &[u8]| -> u64 {
        let arr: [u8; 8] = b[..8].try_into().unwrap();
        if little_endian {
            u64::from_le_bytes(arr)
        } else {
            u64::from_be_bytes(arr)
        }
    };

    let (e_phoff, e_phentsize, e_phnum) = match ei_class {
        1 => {
            let phoff = u64::from(read_u32(&ehdr[28..32]));
            let phentsize = usize::from(read_u16(&ehdr[42..44]));
            let phnum = usize::from(read_u16(&ehdr[44..46]));
            if phentsize < 32 {
                return None;
            }
            (phoff, phentsize, phnum)
        }
        2 => {
            let phoff = read_u64(&ehdr[32..40]);
            let phentsize = usize::from(read_u16(&ehdr[54..56]));
            let phnum = usize::from(read_u16(&ehdr[56..58]));
            if phentsize < 56 {
                return None;
            }
            (phoff, phentsize, phnum)
        }
        _ => return None,
    };

    if e_phnum == 0 || e_phnum > 512 {
        return None;
    }
    let total_ph_bytes = e_phentsize.checked_mul(e_phnum)?;
    if total_ph_bytes > MAX_PHDR_BYTES {
        return None;
    }
    file.seek(std::io::SeekFrom::Start(e_phoff)).ok()?;
    let mut ph_buf = vec![0u8; total_ph_bytes];
    file.read_exact(&mut ph_buf).ok()?;

    for i in 0..e_phnum {
        let ph = &ph_buf[i * e_phentsize..(i + 1) * e_phentsize];
        if read_u32(&ph[0..4]) != PT_NOTE {
            continue;
        }
        let (p_offset, p_filesz, p_align) = if ei_class == 1 {
            (
                u64::from(read_u32(&ph[4..8])),
                u64::from(read_u32(&ph[16..20])),
                u64::from(read_u32(&ph[28..32])),
            )
        } else {
            (
                read_u64(&ph[8..16]),
                read_u64(&ph[32..40]),
                read_u64(&ph[48..56]),
            )
        };
        if p_filesz == 0 || p_filesz > MAX_NOTE_BYTES as u64 {
            continue;
        }
        if file.seek(std::io::SeekFrom::Start(p_offset)).is_err() {
            continue;
        }
        let mut note_buf = vec![0u8; p_filesz as usize];
        if file.read_exact(&mut note_buf).is_err() {
            continue;
        }
        let align = if ei_class == 2 && p_align == 8 { 8 } else { 4 };
        let align_up = |n: usize| n.checked_add(align - 1).map(|v| (v / align) * align);
        let mut off = 0usize;
        while off
            .checked_add(12)
            .is_some_and(|hdr_end| hdr_end <= note_buf.len())
        {
            let namesz = read_u32(&note_buf[off..off + 4]) as usize;
            let descsz = read_u32(&note_buf[off + 4..off + 8]) as usize;
            let ntype = read_u32(&note_buf[off + 8..off + 12]);
            let name_start = off + 12;
            let Some(name_end) = name_start.checked_add(namesz) else {
                break;
            };
            let Some(desc_start) = align_up(name_end) else {
                break;
            };
            let Some(desc_end) = desc_start.checked_add(descsz) else {
                break;
            };
            if desc_end > note_buf.len() {
                break;
            }
            if ntype == NT_GNU_BUILD_ID
                && (1..=64).contains(&descsz)
                && &note_buf[name_start..name_end] == b"GNU\0"
            {
                return Some(note_buf[desc_start..desc_end].to_vec());
            }
            let Some(next_off) = align_up(desc_end) else {
                break;
            };
            off = next_off;
        }
    }
    None
}

pub fn compiler_fingerprint() -> [u8; 32] {
    static FINGERPRINT: OnceLock<[u8; 32]> = OnceLock::new();
    *FINGERPRINT.get_or_init(|| {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(build_id) = read_elf_gnu_build_id(&exe) {
                let mut hasher = Sha256::new();
                hasher.update_str("flurry-compiler-elf-build-id-v1");
                hasher.update(&build_id);
                hasher.update_str(env!("CARGO_PKG_VERSION"));
                hasher.update_str(std::env::consts::ARCH);
                hasher.update_str(std::env::consts::OS);
                return hasher.finalize();
            }
            if let Ok(meta) = fs::metadata(&exe) {
                let mut hasher = Sha256::new();
                hasher.update_str("flurry-compiler-stat-v1");
                hasher.update_u64(meta.len());
                match meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                {
                    Some(dur) => {
                        hasher.update_bool(true);
                        hasher.update_u64(dur.as_secs());
                        hasher.update_u32(dur.subsec_nanos());
                    }
                    None => hasher.update_bool(false),
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    hasher.update_u64(meta.dev());
                    hasher.update_u64(meta.ino());
                    hasher.update_u64(meta.ctime() as u64);
                    hasher.update_u64(meta.ctime_nsec() as u64);
                }
                hasher.update_str(env!("CARGO_PKG_VERSION"));
                hasher.update_str(std::env::consts::ARCH);
                hasher.update_str(std::env::consts::OS);
                return hasher.finalize();
            }
        }
        let mut fallback = Sha256::new();
        fallback.update_str("flurry-compiler-fallback-v1");
        fallback.update_str(env!("CARGO_PKG_VERSION"));
        fallback.update_str(std::env::consts::ARCH);
        fallback.update_str(std::env::consts::OS);
        fallback.finalize()
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredPackageSources {
    pub resolution_id: u32,
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpstreamSourceManifest {
    pub packages: Vec<DiscoveredPackageSources>,
}

pub struct UpstreamKeyInputs<'a> {
    pub all_packages: &'a [SourcePackage],
    pub upstream_ids: &'a [u32],
    pub upstream_edges: &'a [resolve::DependencyEdge],
    pub builtin: Option<u32>,
    pub preludes: &'a [u32],
    pub extra_config_files: &'a [PathBuf],
    pub target: Option<nir::target::TargetData>,
    pub protocols: &'a elaboration::program::Protocols,
    pub selector_tag: &'a [u8],
    pub compiler_override: Option<[u8; 32]>,
}

struct HashedPackageData {
    resolution_id: u32,
    discovered: DiscoveredPackageSources,
    digest: [u8; 32],
}

fn package_boundary_path(package: &SourcePackage) -> Result<(u32, PathBuf), String> {
    let entry = fs::canonicalize(&package.entry)
        .map_err(|e| format!("cannot inspect package {}: {e}", package.name))?;
    let path = if matches!(
        package.entry.file_name().and_then(|n| n.to_str()),
        Some("lib.fl" | "main.fl")
    ) {
        entry
            .parent()
            .ok_or_else(|| format!("package {} entry has no parent", package.name))?
            .to_path_buf()
    } else {
        entry
    };
    Ok((package.resolution_id, path))
}

fn hash_single_package_sources(
    package: &SourcePackage,
    boundaries: &[(u32, PathBuf)],
    is_builtin: bool,
    is_prelude: bool,
) -> Result<HashedPackageData, String> {
    let mut hasher = Sha256::new();
    hasher.update_u32(package.resolution_id);
    hasher.update_u64(package.identity);
    hasher.update_str(&package.name);
    hasher.update_bool(is_builtin);
    hasher.update_bool(is_prelude);

    let canonical_entry = fs::canonicalize(&package.entry)
        .map_err(|e| format!("cannot canonicalize package {}: {e}", package.name))?;
    hasher.update_str(&canonical_entry.to_string_lossy());

    let directory = package
        .entry
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let filename = package
        .entry
        .file_name()
        .ok_or_else(|| format!("package {} entry has no filename", package.name))?;

    let mut files = vec![PathBuf::from(filename)];
    let mut directories = Vec::new();

    if matches!(filename.to_str(), Some("lib.fl" | "main.fl")) {
        let mut exclusions: Vec<PathBuf> = boundaries
            .iter()
            .filter(|(id, _)| *id != package.resolution_id)
            .map(|(_, path)| path.clone())
            .collect();
        if let Some(scripts_dir) = crate::macro_host::scripts_exclusion(&package.entry) {
            exclusions.push(scripts_dir);
        }
        let (discovered_files, discovered_dirs) =
            crate::frontend::discover_sources_excluding(&directory, &exclusions)?;
        files = discovered_files;
        directories = discovered_dirs;
        if !files.contains(&PathBuf::from(filename)) {
            files.push(PathBuf::from(filename));
        }
    }

    directories.sort();
    directories.dedup();
    files.sort();
    files.dedup();

    hasher.update_u64(directories.len() as u64);
    for dir in &directories {
        hasher.update_str(&dir.to_string_lossy());
    }

    hasher.update_u64(files.len() as u64);
    for rel in &files {
        let full = directory.join(rel);
        let bytes =
            fs::read(&full).map_err(|e| format!("cannot read source '{}': {e}", full.display()))?;
        hasher.update_str(&rel.to_string_lossy());
        hasher.update_len_prefixed(&bytes);
    }

    if let Some(scripts_dir) = crate::macro_host::find_package_scripts_dir(&package.entry) {
        hasher.update_bool(true);
        let (mut script_files, mut script_dirs) =
            crate::frontend::discover_sources_excluding(&scripts_dir, &[])?;
        script_dirs.sort();
        script_dirs.dedup();
        script_files.sort();
        script_files.dedup();
        hasher.update_u64(script_dirs.len() as u64);
        for dir in &script_dirs {
            hasher.update_str(&dir.to_string_lossy());
        }
        hasher.update_u64(script_files.len() as u64);
        for rel in &script_files {
            let full = scripts_dir.join(rel);
            let bytes = fs::read(&full)
                .map_err(|e| format!("cannot read macro script '{}': {e}", full.display()))?;
            hasher.update_str(&rel.to_string_lossy());
            hasher.update_len_prefixed(&bytes);
        }
    } else {
        hasher.update_bool(false);
    }

    let root = fs::canonicalize(&directory).unwrap_or(directory);
    Ok(HashedPackageData {
        resolution_id: package.resolution_id,
        discovered: DiscoveredPackageSources {
            resolution_id: package.resolution_id,
            root,
            files,
            directories,
        },
        digest: hasher.finalize(),
    })
}

/// Compute a deterministic, content-addressed `PackageSnapshotKey` for an
/// upward-closed upstream package subset using default parallelism (`resolve_jobs(None)`).
pub fn compute_upstream_key(
    inputs: &UpstreamKeyInputs<'_>,
) -> Result<(PackageSnapshotKey, UpstreamSourceManifest), String> {
    compute_upstream_key_with_jobs(inputs, resolve_jobs(None))
}

/// Compute a deterministic `PackageSnapshotKey` using up to `jobs` threads to hash
/// upstream packages concurrently while folding results in canonical order.
pub fn compute_upstream_key_with_jobs(
    inputs: &UpstreamKeyInputs<'_>,
    jobs: usize,
) -> Result<(PackageSnapshotKey, UpstreamSourceManifest), String> {
    compute_unit_merkle_key(inputs, &[], jobs)
}

/// Extend [`compute_upstream_key_with_jobs`] with Merkle keys of direct dependency
/// units so multi-wave package graphs propagate upstream invalidations transitively.
pub fn compute_unit_merkle_key(
    inputs: &UpstreamKeyInputs<'_>,
    dependency_keys: &[(usize, PackageSnapshotKey)],
    jobs: usize,
) -> Result<(PackageSnapshotKey, UpstreamSourceManifest), String> {
    let boundaries: Vec<(u32, PathBuf)> = inputs
        .all_packages
        .iter()
        .map(package_boundary_path)
        .collect::<Result<Vec<_>, _>>()?;

    let upstream_set: BTreeSet<u32> = inputs.upstream_ids.iter().copied().collect();
    let prelude_set: BTreeSet<u32> = inputs.preludes.iter().copied().collect();

    let mut upstream_packages: Vec<&SourcePackage> = inputs
        .all_packages
        .iter()
        .filter(|pkg| upstream_set.contains(&pkg.resolution_id))
        .collect();
    upstream_packages.sort_by_key(|pkg| pkg.resolution_id);

    let hashed_packages = parallel_map_ordered(&upstream_packages, jobs, |&pkg| {
        hash_single_package_sources(
            pkg,
            &boundaries,
            inputs.builtin == Some(pkg.resolution_id),
            prelude_set.contains(&pkg.resolution_id),
        )
    })?;

    let mut hasher = Sha256::new();
    hasher.update_str(std::str::from_utf8(KEY_DOMAIN_V1).expect("ASCII domain tag"));
    let compiler = inputs
        .compiler_override
        .unwrap_or_else(compiler_fingerprint);
    hasher.update(&compiler);

    // 1. Canonical upstream package source & directory digests.
    hasher.update_u64(hashed_packages.len() as u64);
    let mut manifest_packages = Vec::with_capacity(hashed_packages.len());
    for entry in hashed_packages {
        hasher.update_u32(entry.resolution_id);
        hasher.update(&entry.digest);
        manifest_packages.push(entry.discovered);
    }

    // 2. Canonical upstream dependency edges (filtering out any accidental non-upstream edge).
    let mut sorted_edges: Vec<(u32, &str, u32)> = inputs
        .upstream_edges
        .iter()
        .filter(|e| upstream_set.contains(&e.requester) && upstream_set.contains(&e.provider))
        .map(|e| (e.requester, e.alias.as_str(), e.provider))
        .collect();
    sorted_edges.sort();
    hasher.update_u64(sorted_edges.len() as u64);
    for (requester, alias, provider) in sorted_edges {
        hasher.update_u32(requester);
        hasher.update_str(alias);
        hasher.update_u32(provider);
    }

    // 3. Direct dependency unit Merkle keys (sorted by unit_id).
    let mut sorted_dep_keys = dependency_keys.to_vec();
    sorted_dep_keys.sort();
    sorted_dep_keys.dedup();
    hasher.update_u64(sorted_dep_keys.len() as u64);
    for (unit_id, key) in sorted_dep_keys {
        hasher.update_u64(unit_id as u64);
        hasher.update(&key.0);
    }

    // 4. Extra config files (e.g. language-items.conf), sorted by canonical string path.
    let mut configs: Vec<&PathBuf> = inputs.extra_config_files.iter().collect();
    configs.sort();
    configs.dedup();
    hasher.update_u64(configs.len() as u64);
    for path in configs {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| path.to_string_lossy());
        hasher.update_str(&name);
        match fs::read(path) {
            Ok(bytes) => {
                hasher.update_bool(true);
                hasher.update_len_prefixed(&bytes);
            }
            Err(_) => hasher.update_bool(false),
        }
    }

    // 5. Semantic target data.
    match inputs.target {
        Some(target) => {
            hasher.update_bool(true);
            hasher.update_u32(target.pointer_bits());
        }
        None => hasher.update_bool(false),
    }

    // 6. Protocols scoped strictly to upstream semantic identities (never downstream build.mode!).
    let upstream_identities: BTreeSet<u64> = upstream_packages.iter().map(|p| p.identity).collect();
    hash_scoped_protocols(&mut hasher, inputs.protocols, &upstream_identities);

    // 7. Caller-encoded selector tag (e.g. CLI protocol/selector overrides).
    hasher.update_len_prefixed(inputs.selector_tag);

    Ok((
        PackageSnapshotKey(hasher.finalize()),
        UpstreamSourceManifest {
            packages: manifest_packages,
        },
    ))
}

fn hash_scoped_protocols(
    hasher: &mut Sha256,
    protocols: &elaboration::program::Protocols,
    upstream_identities: &BTreeSet<u64>,
) {
    match protocols.dispatch_fuel {
        Some(fuel) => {
            hasher.update_bool(true);
            hasher.update_u64(fuel as u64);
        }
        None => hasher.update_bool(false),
    }

    let impl_pkgs: BTreeSet<u64> = protocols
        .builtin_implementation_packages
        .intersection(upstream_identities)
        .copied()
        .collect();
    hasher.update_u64(impl_pkgs.len() as u64);
    for id in impl_pkgs {
        hasher.update_u64(id);
    }

    let names: BTreeMap<u64, &str> = protocols
        .package_names
        .iter()
        .filter(|(id, _)| upstream_identities.contains(id))
        .map(|(&id, name)| (id, name.as_str()))
        .collect();
    hasher.update_u64(names.len() as u64);
    for (id, name) in names {
        hasher.update_u64(id);
        hasher.update_str(name);
    }

    let versions: BTreeMap<u64, &str> = protocols
        .package_versions
        .iter()
        .filter(|(id, _)| upstream_identities.contains(id))
        .map(|(&id, ver)| (id, ver.as_str()))
        .collect();
    hasher.update_u64(versions.len() as u64);
    for (id, ver) in versions {
        hasher.update_u64(id);
        hasher.update_str(ver);
    }

    let modes: BTreeMap<u64, &str> = protocols
        .build_modes
        .iter()
        .filter(|(id, _)| upstream_identities.contains(id))
        .map(|(&id, mode)| (id, mode.as_str()))
        .collect();
    hasher.update_u64(modes.len() as u64);
    for (id, mode) in modes {
        hasher.update_u64(id);
        hasher.update_str(mode);
    }

    let targets: BTreeMap<u64, &elaboration::program::build_context::Target> = protocols
        .build_targets
        .iter()
        .filter(|(id, _)| upstream_identities.contains(id))
        .map(|(&id, target)| (id, target))
        .collect();
    hasher.update_u64(targets.len() as u64);
    for (id, target) in targets {
        hasher.update_u64(id);
        hasher.update_str(&target.arch);
        hasher.update_str(&target.os);
        hasher.update_u32(target.layout.pointer_bits());
        hasher.update_u64(target.features.len() as u64);
        for (feat, enabled) in &target.features {
            hasher.update_str(feat);
            hasher.update_bool(*enabled);
        }
    }

    match &protocols.build_metadata {
        Some(meta) => {
            hasher.update_bool(true);
            let aliases: BTreeMap<u64, &BTreeMap<String, u64>> = meta
                .aliases
                .iter()
                .filter(|(id, _)| upstream_identities.contains(id))
                .map(|(&id, map)| (id, map))
                .collect();
            hasher.update_u64(aliases.len() as u64);
            for (pkg, map) in aliases {
                hasher.update_u64(pkg);
                hasher.update_u64(map.len() as u64);
                for (alias, provider) in map {
                    hasher.update_str(alias);
                    hasher.update_u64(*provider);
                }
            }

            let inputs: BTreeMap<u64, &BTreeMap<String, (nir::Term, nir::Term)>> = meta
                .inputs
                .iter()
                .filter(|(id, _)| upstream_identities.contains(id))
                .map(|(&id, map)| (id, map))
                .collect();
            hasher.update_u64(inputs.len() as u64);
            for (pkg, map) in inputs {
                hasher.update_u64(pkg);
                hasher.update_u64(map.len() as u64);
                for (key, (ty, val)) in map {
                    hasher.update_str(key);
                    hasher.update_str(&ty.to_string());
                    hasher.update_str(&val.to_string());
                }
            }
        }
        None => hasher.update_bool(false),
    }

    let hash_decl = |hasher: &mut Sha256, decl: Option<nir::DeclarationId>| match decl {
        Some(d) => {
            hasher.update_bool(true);
            hasher.update_u64(d.package);
            hasher.update_u32(d.local);
        }
        None => hasher.update_bool(false),
    };
    let hash_decl_str =
        |hasher: &mut Sha256, item: &Option<(nir::DeclarationId, String)>| match item {
            Some((d, variant)) => {
                hasher.update_bool(true);
                hasher.update_u64(d.package);
                hasher.update_u32(d.local);
                hasher.update_str(variant);
            }
            None => hasher.update_bool(false),
        };
    let hash_intrinsic_list =
        |hasher: &mut Sha256,
         items: &[((nir::DeclarationId, String), nir::intrinsics::Operation)]| {
            let mut entries: Vec<(u64, u32, &str, String)> = items
                .iter()
                .map(|((d, v), op)| (d.package, d.local, v.as_str(), format!("{op:?}")))
                .collect();
            entries.sort();
            hasher.update_u64(entries.len() as u64);
            for (pkg, local, variant, op) in entries {
                hasher.update_u64(pkg);
                hasher.update_u32(local);
                hasher.update_str(variant);
                hasher.update_str(&op);
            }
        };

    hash_decl_str(hasher, &protocols.host_fma);
    hash_decl_str(hasher, &protocols.host_sqrt);
    hash_decl_str(hasher, &protocols.host_copysign);
    hash_intrinsic_list(hasher, &protocols.comptime_intrinsics);
    hash_intrinsic_list(hasher, &protocols.host_intrinsics);

    for decl in [
        protocols.copy,
        protocols.drop,
        protocols.index,
        protocols.iterator,
        protocols.into_iterator,
        protocols.ordering,
        protocols.equality,
        protocols.list_pattern,
        protocols.comptime_list_pattern,
        protocols.stable_list_pattern,
        protocols.list_pattern_endless,
        protocols.stable_list_pattern_endless,
        protocols.range_end,
        protocols.range_pattern,
        protocols.stable_range_pattern,
        protocols.literal_into,
        protocols.handle_error,
        protocols.splice,
        protocols.meta_list,
        protocols.tuple,
    ] {
        hash_decl(hasher, decl);
    }
    for decl in protocols.arithmetic {
        hash_decl(hasher, decl);
    }
}

// ---------------------------------------------------------------------------
// Two-Tier Storage: DiskPackageCache (.flpkg) & MemoryPackageCache<T>
// ---------------------------------------------------------------------------

/// Driver-level configuration for package snapshot caching and parallel workers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IncrementalConfig {
    pub cache_dir: Option<PathBuf>,
    pub no_cache: bool,
    pub jobs: Option<usize>,
}

impl IncrementalConfig {
    pub fn cache_enabled(&self) -> bool {
        if self.no_cache {
            return false;
        }
        if let Ok(raw) = std::env::var("LUNA_NO_CACHE") {
            let trimmed = raw.trim();
            if trimmed == "1"
                || trimmed.eq_ignore_ascii_case("true")
                || trimmed.eq_ignore_ascii_case("yes")
            {
                return false;
            }
        }
        true
    }

    pub fn effective_jobs(&self) -> usize {
        resolve_jobs(self.jobs)
    }

    pub fn disk_cache(&self) -> DiskPackageCache {
        if !self.cache_enabled() {
            return DiskPackageCache::disabled();
        }
        DiskPackageCache::new(
            self.cache_dir
                .clone()
                .unwrap_or_else(DiskPackageCache::default_location),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ArtifactKind {
    ResolvedBase = 1,
    ElaboratedBase = 2,
    ParsedPackage = 3,
    VerifiedUpstream = 4,
}

impl ArtifactKind {
    fn from_u32(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::ResolvedBase),
            2 => Some(Self::ElaboratedBase),
            3 => Some(Self::ParsedPackage),
            4 => Some(Self::VerifiedUpstream),
            _ => None,
        }
    }
}

static TMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Content-addressed `.flpkg` on-disk package snapshot store with integrity
/// checks (magic + version + kind + key + SHA-256 payload checksum + atomic rename).
#[derive(Clone, Debug)]
pub struct DiskPackageCache {
    root: PathBuf,
    enabled: bool,
}

impl DiskPackageCache {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            root: PathBuf::new(),
            enabled: false,
        }
    }

    pub fn default_location() -> PathBuf {
        if let Some(dir) = std::env::var_os("LUNA_CACHE_DIR").filter(|s| !s.is_empty()) {
            return PathBuf::from(dir);
        }
        if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|s| !s.is_empty()) {
            return PathBuf::from(xdg).join("luna").join("pkg-cache");
        }
        if let Some(home) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
            return PathBuf::from(home)
                .join(".cache")
                .join("luna")
                .join("pkg-cache");
        }
        std::env::temp_dir().join("luna-pkg-cache")
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entry_path(&self, key: &PackageSnapshotKey) -> PathBuf {
        self.root
            .join(format!("v{DISK_FORMAT_VERSION}"))
            .join(key.prefix_dir())
            .join(format!("{}.flpkg", key.to_hex()))
    }

    /// Load and validate a cached payload. Any I/O error, header mismatch, truncation,
    /// or checksum mismatch cleanly returns `None` so the caller rebuilds from source.
    pub fn load(&self, key: &PackageSnapshotKey, expected_kind: ArtifactKind) -> Option<Vec<u8>> {
        if !self.enabled {
            return None;
        }
        let path = self.entry_path(key);
        let bytes = fs::read(path).ok()?;
        const HEADER_LEN: usize = 4 + 4 + 4 + 32 + 8 + 32;
        if bytes.len() < HEADER_LEN {
            return None;
        }
        if bytes[0..4] != DISK_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
        if version != DISK_FORMAT_VERSION {
            return None;
        }
        let kind = ArtifactKind::from_u32(u32::from_le_bytes(bytes[8..12].try_into().ok()?))?;
        if kind != expected_kind {
            return None;
        }
        if bytes[12..44] != key.0 {
            return None;
        }
        let payload_len =
            usize::try_from(u64::from_le_bytes(bytes[44..52].try_into().ok()?)).ok()?;
        if payload_len > MAX_CODEC_BYTES || bytes.len() != HEADER_LEN + payload_len {
            return None;
        }
        let expected_hash: [u8; 32] = bytes[52..84].try_into().ok()?;
        let payload = &bytes[84..];
        if Sha256::hash(payload) != expected_hash {
            return None;
        }
        Some(payload.to_vec())
    }

    /// Atomically write `(key, kind, payload)` using a same-directory temp file + rename.
    pub fn store(
        &self,
        key: &PackageSnapshotKey,
        kind: ArtifactKind,
        payload: &[u8],
    ) -> Result<Option<PathBuf>, String> {
        if !self.enabled {
            return Ok(None);
        }
        if payload.len() > MAX_CODEC_BYTES {
            return Err("snapshot payload exceeds maximum cache size".into());
        }
        let target = self.entry_path(key);
        let parent = target
            .parent()
            .ok_or_else(|| "cache target path has no parent directory".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create cache dir '{}': {e}", parent.display()))?;

        let seq = TMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(
            ".{}.{}.{}.tmp",
            key.to_hex(),
            std::process::id(),
            seq
        ));

        let payload_hash = Sha256::hash(payload);
        let write_result = (|| -> std::io::Result<()> {
            let mut file = File::create(&tmp)?;
            file.write_all(&DISK_MAGIC)?;
            file.write_all(&DISK_FORMAT_VERSION.to_le_bytes())?;
            file.write_all(&(kind as u32).to_le_bytes())?;
            file.write_all(&key.0)?;
            file.write_all(&(payload.len() as u64).to_le_bytes())?;
            file.write_all(&payload_hash)?;
            file.write_all(payload)?;
            file.sync_data()?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "cannot write cache entry '{}': {err}",
                tmp.display()
            ));
        }

        if let Err(err) = fs::rename(&tmp, &target) {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "cannot publish cache entry '{}': {err}",
                target.display()
            ));
        }

        Ok(Some(target))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

struct MemoryCacheInner<T> {
    capacity: usize,
    entries: HashMap<(PackageSnapshotKey, ArtifactKind), Arc<T>>,
    order: VecDeque<(PackageSnapshotKey, ArtifactKind)>,
    stats: CacheStats,
}

/// Thread-safe bounded in-process LRU cache for deserialized/frozen package bases.
pub struct MemoryPackageCache<T> {
    inner: Mutex<MemoryCacheInner<T>>,
}

impl<T> MemoryPackageCache<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(MemoryCacheInner {
                capacity,
                entries: HashMap::new(),
                order: VecDeque::new(),
                stats: CacheStats::default(),
            }),
        }
    }

    pub fn get(&self, key: PackageSnapshotKey, kind: ArtifactKind) -> Option<Arc<T>> {
        let mut inner = self.inner.lock().expect("memory cache lock poisoned");
        let full_key = (key, kind);
        if let Some(value) = inner.entries.get(&full_key).cloned() {
            inner.stats.hits += 1;
            if let Some(pos) = inner.order.iter().position(|k| *k == full_key) {
                inner.order.remove(pos);
                inner.order.push_back(full_key);
            }
            Some(value)
        } else {
            inner.stats.misses += 1;
            None
        }
    }

    pub fn insert(&self, key: PackageSnapshotKey, kind: ArtifactKind, value: Arc<T>) {
        let mut inner = self.inner.lock().expect("memory cache lock poisoned");
        if inner.capacity == 0 {
            return;
        }
        let full_key = (key, kind);
        if inner.entries.contains_key(&full_key) {
            inner.entries.insert(full_key, value);
            if let Some(pos) = inner.order.iter().position(|k| *k == full_key) {
                inner.order.remove(pos);
            }
            inner.order.push_back(full_key);
            return;
        }
        if inner.entries.len() == inner.capacity {
            if let Some(evicted) = inner.order.pop_front() {
                inner.entries.remove(&evicted);
            }
        }
        inner.entries.insert(full_key, value);
        inner.order.push_back(full_key);
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().expect("memory cache lock poisoned");
        inner.entries.clear();
        inner.order.clear();
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("memory cache lock poisoned")
            .entries
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> CacheStats {
        self.inner.lock().expect("memory cache lock poisoned").stats
    }
}

// ---------------------------------------------------------------------------
// Portable ParsedPackage & VerifiedUpstream Snapshot Codecs + Global Caches
// ---------------------------------------------------------------------------

/// Compute a deterministic key for a package's parsed AST snapshot.
pub fn compute_parsed_package_key(
    package_name: &str,
    files: &[(PathBuf, &[u8])],
) -> PackageSnapshotKey {
    let mut sorted: Vec<(&Path, &[u8])> = files.iter().map(|(p, b)| (p.as_path(), *b)).collect();
    sorted.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut hasher = Sha256::new();
    hasher.update_str(std::str::from_utf8(PARSED_PKG_KEY_DOMAIN_V1).expect("ASCII domain tag"));
    let compiler = compiler_fingerprint();
    hasher.update(&compiler);
    hasher.update_str(package_name);
    hasher.update_u64(sorted.len() as u64);
    for (rel, bytes) in sorted {
        hasher.update_str(&rel.to_string_lossy());
        hasher.update_len_prefixed(bytes);
    }
    PackageSnapshotKey(hasher.finalize())
}

/// Deterministic byte tag for protocol selectors so upstream verification caches
/// invalidate when any source protocol or intrinsic selector changes.
pub fn encode_selectors_tag(
    selectors: &crate::protocols::type_constructors::Selectors<'_>,
) -> Vec<u8> {
    let crate::protocols::type_constructors::Selectors {
        dispatch_fuel,
        host_fma,
        host_sqrt,
        host_copysign,
        comptime_intrinsics,
        host_intrinsics,
        copy,
        drop,
        index,
        iterator,
        into_iterator,
        ordering,
        equality,
        list_pattern,
        comptime_list_pattern,
        stable_list_pattern,
        list_pattern_endless,
        stable_list_pattern_endless,
        range_end,
        range_pattern,
        stable_range_pattern,
        literal_into,
        handle_error,
        arithmetic,
        splice,
        meta_list,
        tuple,
    } = selectors;

    let mut writer = BinaryWriter::new();
    let write_opt_str = |w: &mut BinaryWriter, value: Option<&str>| match value {
        Some(s) => {
            w.write_bool(true);
            w.write_str(s);
        }
        None => w.write_bool(false),
    };
    let write_intrinsics =
        |w: &mut BinaryWriter, items: &[(nir::intrinsics::Operation, String)]| {
            let mut entries: Vec<(&str, &str)> = items
                .iter()
                .map(|(op, sel)| (op.name(), sel.as_str()))
                .collect();
            entries.sort();
            w.write_u64(entries.len() as u64);
            for (op, sel) in entries {
                w.write_str(op);
                w.write_str(sel);
            }
        };

    match dispatch_fuel {
        Some(fuel) => {
            writer.write_bool(true);
            writer.write_u64(*fuel as u64);
        }
        None => writer.write_bool(false),
    }
    write_opt_str(&mut writer, *host_fma);
    write_opt_str(&mut writer, *host_sqrt);
    write_opt_str(&mut writer, *host_copysign);
    write_intrinsics(&mut writer, comptime_intrinsics);
    write_intrinsics(&mut writer, host_intrinsics);
    for opt in [
        *copy,
        *drop,
        *index,
        *iterator,
        *into_iterator,
        *ordering,
        *equality,
        *list_pattern,
        *comptime_list_pattern,
        *stable_list_pattern,
        *list_pattern_endless,
        *stable_list_pattern_endless,
        *range_end,
        *range_pattern,
        *stable_range_pattern,
        *literal_into,
        *handle_error,
        *splice,
        *meta_list,
        *tuple,
    ] {
        write_opt_str(&mut writer, opt);
    }
    for opt in arithmetic {
        write_opt_str(&mut writer, *opt);
    }
    writer.into_vec()
}

#[derive(Clone, Debug)]
pub struct CachedParsedFile {
    pub rel_path: String,
    pub content_hash: [u8; 32],
    pub ast: Option<Ast>,
}

#[derive(Clone, Debug, Default)]
pub struct CachedParsedPackage {
    pub files: Vec<CachedParsedFile>,
}

fn bounded_count(
    reader: &mut BinaryReader<'_>,
    min_item_bytes: usize,
) -> Result<usize, CodecError> {
    let raw = reader.read_u64()?;
    let count = usize::try_from(raw).map_err(|_| CodecError::LengthLimitExceeded)?;
    if count > MAX_CODEC_ELEMENTS {
        return Err(CodecError::LengthLimitExceeded);
    }
    if min_item_bytes > 0 {
        let needed = count
            .checked_mul(min_item_bytes)
            .ok_or(CodecError::LengthLimitExceeded)?;
        if needed > reader.remaining() {
            return Err(CodecError::UnexpectedEof);
        }
    }
    Ok(count)
}

fn encode_ast(writer: &mut BinaryWriter, ast: &Ast) -> Result<(), CodecError> {
    ast.validate_structure()
        .map_err(|_| CodecError::InvalidStructure("invalid AST layout before encoding"))?;
    if ast.nodes.is_empty()
        || ast.nodes.len() != ast.spans.len()
        || ast.nodes.len() != ast.children_start.len()
        || ast.nodes.len() != ast.children_len.len()
    {
        return Err(CodecError::InvalidStructure(
            "mismatched AST parallel vector lengths",
        ));
    }

    let mut children = ast.children.clone();
    let mut strings: Vec<String> = Vec::new();
    let mut string_ids: HashMap<Symbol, u32> = HashMap::new();

    for (idx, &kind) in ast.nodes.iter().enumerate().skip(1) {
        if kind == NodeKind::Id {
            let start = ast.children_start[idx] as usize;
            let len = ast.children_len[idx] as usize;
            if len != 2 || start.checked_add(1).is_none_or(|end| end >= children.len()) {
                return Err(CodecError::InvalidStructure(
                    "invalid NodeKind::Id storage slots",
                ));
            }
            let hi = children[start];
            let lo = children[start + 1];
            // SAFETY: Validated AST `NodeKind::Id` nodes produced by Parser (or
            // decoded in this process) store `(hi, lo)` from `Symbol::to_raw_parts`.
            let sym = unsafe { Symbol::from_raw_parts(hi, lo) };
            let table_idx = match string_ids.get(&sym) {
                Some(&existing) => existing,
                None => {
                    let next = u32::try_from(strings.len())
                        .map_err(|_| CodecError::LengthLimitExceeded)?;
                    strings.push(sym.as_str().to_owned());
                    string_ids.insert(sym, next);
                    next
                }
            };
            children[start] = table_idx;
            children[start + 1] = 0;
        }
    }

    writer.write_u64(strings.len() as u64);
    for text in &strings {
        writer.write_str(text);
    }

    writer.write_u32(ast.root);
    writer.write_u64(ast.nodes.len() as u64);
    let base = ast.file_byte_pos.0;
    for i in 0..ast.nodes.len() {
        writer.write_u32(ast.nodes[i] as u32);
        let span = ast.spans[i];
        if i == 0 || span == DUMMY_SP {
            writer.write_bool(true);
        } else {
            let rel_lo = span
                .lo()
                .0
                .checked_sub(base)
                .ok_or(CodecError::InvalidStructure("span lo precedes file start"))?;
            let rel_hi = span
                .hi()
                .0
                .checked_sub(base)
                .ok_or(CodecError::InvalidStructure("span hi precedes file start"))?;
            writer.write_bool(false);
            writer.write_u32(rel_lo);
            writer.write_u32(rel_hi);
        }
        writer.write_u32(ast.children_start[i]);
        writer.write_u32(ast.children_len[i]);
    }

    writer.write_u64(children.len() as u64);
    for word in children {
        writer.write_u32(word);
    }
    Ok(())
}

fn decode_ast(reader: &mut BinaryReader<'_>) -> Result<Ast, CodecError> {
    let string_count = bounded_count(reader, 8)?;
    let mut interned_parts = Vec::with_capacity(string_count);
    for _ in 0..string_count {
        let sym = Symbol::intern(reader.read_str()?);
        interned_parts.push(sym.to_raw_parts());
    }

    let root = reader.read_u32()?;
    // Minimum per node: u32 kind (4) + bool is_dummy (1) + u32 start (4) + u32 len (4) = 13 bytes.
    let node_count = bounded_count(reader, 13)?;
    if node_count == 0 {
        return Err(CodecError::InvalidStructure("missing AST sentinel node 0"));
    }

    let mut nodes = Vec::with_capacity(node_count);
    let mut spans = Vec::with_capacity(node_count);
    let mut children_start = Vec::with_capacity(node_count);
    let mut children_len = Vec::with_capacity(node_count);
    for idx in 0..node_count {
        let kind_raw = reader.read_u32()?;
        let kind = NodeKind::from_u32(kind_raw).ok_or(CodecError::InvalidStructure(
            "unknown AST NodeKind discriminant",
        ))?;
        if idx == 0 && kind != NodeKind::Invalid {
            return Err(CodecError::InvalidStructure(
                "AST sentinel node 0 must be Invalid",
            ));
        }
        let is_dummy = reader.read_bool()?;
        let span = if is_dummy {
            DUMMY_SP
        } else {
            let rel_lo = reader.read_u32()?;
            let rel_hi = reader.read_u32()?;
            if rel_lo > rel_hi {
                return Err(CodecError::InvalidStructure("inverted AST span bounds"));
            }
            let lo = rel_lo.checked_add(1).ok_or(CodecError::InvalidStructure(
                "AST span lo overflows BytePos(1) base",
            ))?;
            let hi = rel_hi.checked_add(1).ok_or(CodecError::InvalidStructure(
                "AST span hi overflows BytePos(1) base",
            ))?;
            Span::new(BytePos(lo), BytePos(hi))
        };
        let start = reader.read_u32()?;
        let len = reader.read_u32()?;
        nodes.push(kind);
        spans.push(span);
        children_start.push(start);
        children_len.push(len);
    }

    let children_count = bounded_count(reader, 4)?;
    if children_count == 0 {
        return Err(CodecError::InvalidStructure(
            "missing AST children sentinel 0",
        ));
    }
    let mut children = Vec::with_capacity(children_count);
    for _ in 0..children_count {
        children.push(reader.read_u32()?);
    }

    for (idx, &kind) in nodes.iter().enumerate().skip(1) {
        if kind == NodeKind::Id {
            let start = children_start[idx] as usize;
            let len = children_len[idx] as usize;
            if len != 2 || start.checked_add(1).is_none_or(|end| end >= children.len()) {
                return Err(CodecError::InvalidStructure(
                    "invalid decoded NodeKind::Id storage slots",
                ));
            }
            let sym_idx = children[start] as usize;
            if children[start + 1] != 0 {
                return Err(CodecError::InvalidStructure(
                    "corrupt NodeKind::Id symbol sentinel",
                ));
            }
            let &(hi, lo) = interned_parts
                .get(sym_idx)
                .ok_or(CodecError::InvalidStructure(
                    "AST NodeKind::Id string table index out of bounds",
                ))?;
            children[start] = hi;
            children[start + 1] = lo;
        }
    }

    let ast = Ast {
        nodes,
        spans,
        children_start,
        children_len,
        children,
        root,
        file_byte_pos: BytePos(1),
    };
    if root != 0 && ast.get_node_kind(root).is_none() {
        return Err(CodecError::InvalidStructure("AST root out of bounds"));
    }
    ast.validate_structure()
        .map_err(|_| CodecError::InvalidStructure("AST failed structural validation"))?;
    Ok(ast)
}

pub fn encode_parsed_package(pkg: &CachedParsedPackage) -> Result<Vec<u8>, CodecError> {
    let mut writer = BinaryWriter::new();
    writer.write_u32(PARSED_PKG_CODEC_VERSION);
    writer.write_u64(pkg.files.len() as u64);
    for file in &pkg.files {
        writer.write_str(&file.rel_path);
        writer.write_array32(&file.content_hash);
        match &file.ast {
            Some(ast) => {
                writer.write_bool(true);
                encode_ast(&mut writer, ast)?;
            }
            None => writer.write_bool(false),
        }
        if writer.as_slice().len() > MAX_CODEC_BYTES {
            return Err(CodecError::LengthLimitExceeded);
        }
    }
    Ok(writer.into_vec())
}

pub fn decode_parsed_package(data: &[u8]) -> Result<CachedParsedPackage, CodecError> {
    if data.len() > MAX_CODEC_BYTES {
        return Err(CodecError::LengthLimitExceeded);
    }
    let mut reader = BinaryReader::new(data);
    let version = reader.read_u32()?;
    if version != PARSED_PKG_CODEC_VERSION {
        return Err(CodecError::InvalidStructure(
            "unsupported parsed package codec version",
        ));
    }
    // Minimum per file: 8-byte path len + 32-byte hash + 1-byte has_ast flag = 41 bytes.
    let file_count = bounded_count(&mut reader, 41)?;
    let mut files = Vec::with_capacity(file_count);
    for _ in 0..file_count {
        let rel_path = reader.read_str()?.to_owned();
        let content_hash = reader.read_array32()?;
        let has_ast = reader.read_bool()?;
        let ast = if has_ast {
            Some(decode_ast(&mut reader)?)
        } else {
            None
        };
        files.push(CachedParsedFile {
            rel_path,
            content_hash,
            ast,
        });
    }
    if reader.remaining() != 0 {
        return Err(CodecError::InvalidStructure(
            "trailing bytes in parsed package snapshot",
        ));
    }
    Ok(CachedParsedPackage { files })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedUpstreamCertificate {
    pub upstream_identities: Vec<u64>,
}

pub fn encode_verified_upstream(cert: &VerifiedUpstreamCertificate) -> Vec<u8> {
    let mut writer = BinaryWriter::new();
    writer.write_u32(VERIFIED_UPSTREAM_CODEC_VERSION);
    writer.write_u64(cert.upstream_identities.len() as u64);
    for &id in &cert.upstream_identities {
        writer.write_u64(id);
    }
    writer.into_vec()
}

pub fn decode_verified_upstream(data: &[u8]) -> Result<VerifiedUpstreamCertificate, CodecError> {
    let mut reader = BinaryReader::new(data);
    let version = reader.read_u32()?;
    if version != VERIFIED_UPSTREAM_CODEC_VERSION {
        return Err(CodecError::InvalidStructure(
            "unsupported verified upstream certificate version",
        ));
    }
    let count = bounded_count(&mut reader, 8)?;
    let mut upstream_identities = Vec::with_capacity(count);
    for _ in 0..count {
        upstream_identities.push(reader.read_u64()?);
    }
    if reader.remaining() != 0 {
        return Err(CodecError::InvalidStructure(
            "trailing bytes in verified upstream certificate",
        ));
    }
    Ok(VerifiedUpstreamCertificate {
        upstream_identities,
    })
}

pub fn global_parsed_cache() -> &'static MemoryPackageCache<CachedParsedPackage> {
    static CACHE: OnceLock<MemoryPackageCache<CachedParsedPackage>> = OnceLock::new();
    CACHE.get_or_init(|| MemoryPackageCache::new(256))
}

pub fn global_verified_cache() -> &'static MemoryPackageCache<VerifiedUpstreamCertificate> {
    static CACHE: OnceLock<MemoryPackageCache<VerifiedUpstreamCertificate>> = OnceLock::new();
    CACHE.get_or_init(|| MemoryPackageCache::new(64))
}

pub fn global_elaborated_cache()
-> &'static MemoryPackageCache<elaboration::program::CachedUpstreamBodies> {
    static CACHE: OnceLock<MemoryPackageCache<elaboration::program::CachedUpstreamBodies>> =
        OnceLock::new();
    CACHE.get_or_init(|| MemoryPackageCache::new(32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_fingerprint_is_deterministic_and_non_zero() {
        let first = compiler_fingerprint();
        let second = compiler_fingerprint();
        assert_eq!(first, second);
        assert_ne!(first, [0u8; 32]);

        #[cfg(target_os = "linux")]
        {
            let exe = std::env::current_exe().unwrap();
            let build_id = read_elf_gnu_build_id(&exe)
                .expect("Linux test binary should expose NT_GNU_BUILD_ID");
            assert!(!build_id.is_empty());
        }
    }
}
