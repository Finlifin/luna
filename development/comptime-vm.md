# Comptime invocation execution and definition builders

## Scope and current boundary

The invocation engine executes typed NIR and validated IIR. It still uses an
expression dispatcher, not a complete bytecode VM. The current optimization
work targets invocation frames and lexical-scope bookkeeping without changing
phase admission, contracts, abstract values, storage identity, effect transfer,
or the step/depth budgets.

Ordinary single-owner locals use directly indexed slots. Mixed-owner captures,
out-of-order identities, and sufficiently sparse indices transition to an
ordered map. The numeric size of a LocalId cannot request an equally large
allocation. A scope snapshot records binding presence, not a copy of values:
assignments to existing bindings survive scope exit; newly introduced bindings
do not. Up to 64 register slots use an inline liveness bitmap; other frames use
sorted identities. Snapshots remain valid after a frame changes representation.

Without a cleanup schedule, successful/ordinary failed evaluation can return
without dispatching empty cleanup hooks. Effect-control transfer still passes
through its scope handler. Scheduled IIR continues through the existing delivery
and cleanup machinery.

## Repeatable measurement

From the repository root:

```sh
python3 -B development/comptime_bench.py --repeat 5
```

The entrypoint builds the release benchmark through the managed Cargo cache and
runs each sample with a deadline and saved evidence. `--binary PATH` measures a
saved executable without rebuilding. `LUNA_BENCH_ITERATIONS` selects the number
of invocations per workload (default 20,000).

The benchmark constructs NIR and an interpreter before timing. It checks results
and reports nanoseconds, allocator calls, and consumed steps per invocation for
0/1/8/64 parameters, with either no extra reads or 256 local reads. Inputs are
Bool values. Allocator counting is enabled for both versions; timings include
that instrumentation and argument-vector construction. The benchmark does not
measure package admission, parsing, elaboration, complex Type computation,
builder activity, or end-to-end compilation. It is evidence for these invocation
workloads only, not a whole-compiler speedup claim.

### 2026-10-25 comparison

Five process samples per version, 20,000 invocations per workload; median ns:

| Extra local reads | Parameters | Before | After | Reduction | Allocations before → after | Steps (both) |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | 0 | 105 | 98 | 7% | 0 → 0 | 2 |
| 0 | 1 | 231 | 184 | 20% | 7 → 5 | 7 |
| 0 | 8 | 702 | 508 | 28% | 7 → 5 | 28 |
| 0 | 64 | 4849 | 2944 | 39% | 22 → 5 | 196 |
| 256 | 1 | 6176 | 5405 | 12% | 7 → 5 | 519 |
| 256 | 8 | 6906 | 5700 | 17% | 7 → 5 | 540 |
| 256 | 64 | 11346 | 8139 | 28% | 22 → 5 | 708 |

The baseline used the same workspace/compiler and benchmark with only the new
frame, scope-snapshot, and cleanup fast-path changes disabled. It was built before
the optimized version. These are sequential local samples, not CPU-pinned or
statistically controlled measurements. Empty-call differences are small enough
that system noise may explain them. The allocation reductions are deterministic.

Build evidence: `/tmp/luna-tools-4w86_lj8/result.json` (baseline) and
`/tmp/luna-tools-stxa6wew/result.json` (optimized), both exit 0.
Per-run evidence and full workload output (`stdout.log` beside `result.json`):

- Baseline: `/tmp/luna-tools-09xm2ufo/`, `/tmp/luna-tools-wfox93k5/`,
  `/tmp/luna-tools-dhknt46e/`, `/tmp/luna-tools-ok_y8l9k/`, `/tmp/luna-tools-_rm3bpff/`.
- Optimized: `/tmp/luna-tools-29xjue77/`, `/tmp/luna-tools-qukxnnnj/`,
  `/tmp/luna-tools-tciem5g4/`, `/tmp/luna-tools-si733c2j/`, `/tmp/luna-tools-sq5mwsxk/`.

The initial attempt to run all interpreter tests failed during linking with a
disk-quota error and an evidence-write error (`/tmp/luna-tools-9u6vb09g/result.json`,
Cargo exit 101 / wrapper 125). It is not counted as passing validation. Subsequent
validation uses selected groups after the cache's automatic recovery.

### Validation

Working directory: `/home/fin/luna`.

- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib --test abstract_values --test instance_budget --test function_types --test environment_types --test type_list_inputs`: exit 0, **121 passed**;
  `/tmp/luna-tools-wnuq_23p/result.json`. This run includes sparse/owner identities,
  64/65-slot snapshots, disappearance of dead-register tails, representation
  transitions, budget failures, abstract inputs, and existing memory/closure tests.
- The first selected attempt found a test still constructing `BTreeMap` directly
  (`/tmp/luna-tools-3cacbwn6/result.json`, exit 101). It now constructs `Environment`;
  the budget/storage-identity assertions are unchanged. The successful run above
  supersedes that compile failure.
- The first combined Luna group (`comptime_memory`, `resources`,
  `effect_memory_transfers`, `pure_bitvec`, `cmtt`) failed with storage quota and
  evidence-write errors. Terminal evidence for `/tmp/luna-tools-9o13vp8x/result.json`
  reports Cargo 101, wrapper 125, `report_saved: false`; the on-disk result may
  still say running. This failed attempt is not passing integration evidence.
- Retrying the end-to-end path separately:
  `python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_memory`
  exited 0, **17 passed**, `/tmp/luna-tools-75lqvh9f/result.json`.
- The next group run, `/tmp/luna-tools-vtycqhz0/result.json`, completed **523 CMTT
  tests successfully**, then failed one effect-memory test (90 passed / 1 failed).
  The failed expectation rejected a scalar lexical return. That expectation was
  stale: [direct passing](../docs/algebraic-effect-and-delimited-continuation/direct-passing.md)
  preserves lexical return targets, and the existing resources test
  `handler_lexical_data_return_keeps_parameter_accesses_separate_from_completion`
  already checks that a scalar lexical result contributes accesses without
  becoming the operation's normal completion result. The test now checks this
  accepted behavior and adds a pointer-valued exit that still requires a separate
  exit summary. No production effect/memory admission rule was relaxed.
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test resources --test effect_memory_transfers --test pure_bitvec --no-fail-fast`:
  exit 0, **365 + 91 + 29 = 485 passed**, `/tmp/luna-tools-_oovaltl/result.json`.
  Together with the interpreter, CMTT, and comptime-memory groups, **1,146 distinct
  tests passed** for this work. This is targeted validation, not a full workspace run.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
  exited 0 after the final test correction, `/tmp/luna-tools-y885s07x/result.json`;
  `git diff --check` passed.

Removal/retention compacts a register frame once dead slots exceed the bounded
slack, so iteration cost follows live bindings rather than the largest historical
frame. This preserves the intended resource-accounting bound for repeated scope
snapshots after large temporary scopes.

## Definition-builder gap

The author's SoA example is
`/home/fin/vilya/examples/2026-8-5-typed-Soa.fl`, starting at `StructOfArray`.
Its `StructBuilder.new`, `add_field`, and `build` operations are not implemented
in the compiler/library. `library/meta/lib.fl` currently exposes the foundational
`List<Type>` head. Existing CMTT expression/pattern/arm generation and internal
materialization of already-declared nominals do not implement definition builders.

The author's intended scope is that most kinds of definition can be constructed
through builders. Therefore the implementation boundary must support a common
definition-registration/elaboration path, with per-kind payloads for structs,
enums, functions, traits, implementations, and other supported definitions. Do
not implement an isolated SoA-only struct shortcut and report this feature done.

Required integration work:

- Separate mutable, invocation-owned builder state from completed definition
  descriptions. An unfinished builder is not a frozen Type or completed definition.
- Feed generated declarations through normal name binding, signature/body
  elaboration, phase checks, contracts, and applicable coherence checks. Preserve
  hygienic identities and source/expansion diagnostics; reject incomplete or
  invalid definitions rather than manufacturing executable IR.
- Preserve normal-form heads and the distinction between type identity and an
  internal structural view. Builder output must not make `Identity<T>` equal to T.
- Support generated bodies through the established CMTT machinery, including
  contexts and ordinary-argument single evaluation. Do not freeze active inference
  variables, candidate transactions, or waiters into VM values.
- First acceptance: inspect input fields, generate SoA column fields/defaults in
  a comptime loop, finalize the struct, attach methods, and compile/use the result.
  Cover duplicate names, invalid field types/defaults, failed finalization and
  recovery, scope escape, and repeat instantiation. Broader definition kinds need
  their own acceptance cases against the shared path.

These are implementation requirements and a recorded gap, not newly introduced
source APIs or a claim that the SoA example can currently run.
