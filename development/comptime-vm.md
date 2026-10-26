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

The common HIR registration substrate is now implemented and used for generated
module definitions; its checks and remaining VM/elaboration work are tracked in
[definition-builders.md](definition-builders.md). This does not yet expose a
source-level definition builder.

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

## Associated type observation during invocation

Selected type views now traverse FunctionValue and ClosureSnapshot instance
arguments, retaining declaration/factory identities. The previous fallback cloned
these aggregate terms without resolving nested associated Type queries or charging
their traversal. Normalization now matches every Term variant explicitly so future
aggregate variants cannot silently enter that fallback.

The interpreter shares one metered selected-type-view entrypoint between reflection,
Type-value evaluation, record/variant construction, associated-value lookup and
method dispatch-key construction. Query work uses the current invocation allowance
instead of a fresh independent view allowance at each explicit transformation.
Failure retains consumed work and reports its original error category and source.
This does not yet replace the separate bounded views inside immutable has_type,
capability and identity-comparison predicates with a fully metered admission API.

Verification in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib --test nominal_types --test type_equality --test normal_values --test function_types --test record_values --test instance_budget
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

178 tests passed, exit 0, `/tmp/luna-tools-hp90ehse/result.json`. Two regressions
failed before the fix (`/tmp/luna-tools-8dg5aaee/result.json`): nested selected inputs
remained unresolved and a 100-input callable value fit a three-step view budget.
Final tests additionally exercise a real TypeValue evaluation with a 20-selection
chain, exhaustion and retry, and missing-evidence diagnostics retaining the source.
Formatting passed at `/tmp/luna-tools-k726hsld/result.json`; `git diff --check`
passed. No source reflection API or builder publication behavior is added here.

## Indexed declaration-world admission

Interpreter construction now builds the function index before checking body-free
runtime identities, reusing it for collision checks. Phase validation separately
indexes body and runtime-identity membership for retained quote templates and
runtime identities. Dispatch admission reuses an implementation-ID map instead
of scanning all implementations for each instance and again for capabilities.
Duplicate identities, missing implementations and omitted demanded methods keep
their rejection behavior. No function body, type contract or phase check is
removed. Indices add linear storage and replace repeated linear membership scans
with logarithmic lookup; other world-validation work retains its existing costs.

`compiler/comptime_interpretor/examples/world_construction.rs` measures only
`Interpreter::for_comptime` construction, including its phase checks. Package
allocation and VM destruction are outside the timed interval. Each row uses one
warmup and the median of seven measured runs. Each world has equally many simple
comptime functions and distinct body-free runtime identities. No timing assertion
is used as a test gate.

```sh
python3 -B development/check.py --timeout 900 -- run --release -p comptime_interpretor --example world_construction
```

Same-host release measurements, microseconds:

| Total declarations | Before | VM index only | VM and phase indices |
| --- | ---: | ---: | ---: |
| 2,000 | 1,319 | 759 | 232 |
| 8,000 | 25,502 | 11,232 | 1,042 |
| 32,000 | 387,470 | 198,505 | 4,393 |

Reports: `/tmp/luna-tools-2ajlzndp/result.json` (baseline),
`/tmp/luna-tools-zx2ug_dj/result.json` (intermediate), and
`/tmp/luna-tools-6ispd10o/result.json` (final), all exit 0. The largest fixture is
about 88x faster at world construction; this is not a whole-compiler or invocation
execution speedup, nor a benchmark of implementation-rich dispatch worlds.

Verification in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor -p nir --test runtime_function_identities --test phases --test nominal_types --test normal_inputs --test instance_budget
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

85 targeted tests passed at `/tmp/luna-tools-kqcdbw5c/result.json`; 119 VM library
tests passed at `/tmp/luna-tools-03jtgu97/result.json` (204 total). New checks cover
collisions at the end of a larger function catalog, duplicate bodies, successful
lookup, missing/duplicate implementation identities and repair. The first dispatch
fixture omitted its demanded method and correctly produced UnmaterializedDispatch;
the final test asserts that rejection before installing the explicit method and
checking success. Existing phase tests retain quote/runtime/body collision and
wrong-phase coverage. Formatting passed at `/tmp/luna-tools-osgt43d6/result.json`;
`git diff --check` passed. Live sessions still borrow a fixed immutable NIR world;
this optimization does not add active invocation suspension or world replacement.

## Local type-list indexing

Comptime reads from a local `meta.List<Type>` now borrow its immutable payload
when the index is an integer literal or a local integer. This also applies to
locals backed by mutable storage. Each read retains its execution ticks and
charges the selected type's description before cloning it; unrelated elements
are not copied again. Initial argument admission still validates the whole list.

Effectful index expressions retain the general evaluation path: the list value
is obtained before the index executes, even if the index replaces that local.
Cleanup-scheduled execution also retains the general path. This optimization
does not change normal-form identity or the representation of Tuple inputs.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --test type_list_index --test type_list_inputs --test instance_budget
```

All 21 tests passed (`/tmp/luna-tools-wiul9qbr/result.json`). The cost regression
compares one and 64 reads from 4096 elements, for literal/local indices and
ordinary/storage-backed bindings; the 63 extra reads consume fewer than 2000
steps in each case. This is a deterministic VM work-budget bound, not a measured
wall-clock speedup. Bounds failures, subsequent invocation recovery and
source-list replacement during index evaluation are checked separately.

### Paired indexing measurements

`python3 -B development/comptime_bench.py --bench type_list_index --iterations 100 --repeat 5`
measures three index forms on the same VM build. The general form wraps the
literal index in an empty block, selecting snapshot evaluation; it includes that
extra block's execution overhead. This comparison is not a historical binary
comparison. Each process warms up five invocations per workload. Initial argument
allocation, admission and invocation are timed; package and VM construction are
outside the measured region. The allocator counts allocation/reallocation calls
and requested bytes, not live or peak memory. Every result is checked.

Five-process medians on the current workspace after `f9a92c53`, 4096 elements:

| Index form | Reads/call | ns/call | Allocations/call | Requested bytes/call | VM steps/call |
|---|---:|---:|---:|---:|---:|
| Literal | 1 | 493,636 | 28,706.04 | 1,936,092 | 20,508 |
| Literal | 64 | 516,913 | 29,213.04 | 2,004,304 | 21,642 |
| Local | 64 | 560,874 | 29,169.04 | 2,234,580 | 21,265 |
| General block | 64 | 8,488,573 | 553,693.04 | 33,485,136 | 546,250 |

The 64-read literal workload is about 16.4x faster than its general-path pair,
with about 18.9x fewer allocation calls. These ratios describe this microbenchmark,
not whole compilation. Input admission still dominates a single leaf read and
is a separate optimization opportunity. Primitive type elements do not represent
the cost of copying deeply structured selected types.

Build report: `/tmp/luna-tools-pi_p2vly/result.json`. All five runs exited 0:
`/tmp/luna-tools-i05nyx0n/result.json`, `/tmp/luna-tools-ibehgv41/result.json`,
`/tmp/luna-tools-8yltm80h/result.json`, `/tmp/luna-tools-5a2haqsm/result.json`,
`/tmp/luna-tools-s0ffjald/result.json`.

### Lazy validation traversal state

Type-description validation now keeps its initial node and empty binder context
off the heap. It creates a pending-node vector only when visiting children and
shared binder frames only when entering function telescopes. Empty function
telescopes still introduce a frame; sibling nodes retain independent scope
contexts. Structural/scope validation and logical work charges are unchanged.

The same five-process benchmark command after this change gives these medians
for 4096 elements (the preceding table is the baseline):

| Index form | Reads/call | ns/call | Allocations/call | Requested bytes/call | VM steps/call |
|---|---:|---:|---:|---:|---:|
| Literal | 1 | 290,838 | 4,126.04 | 1,149,532 | 20,508 |
| Literal | 64 | 312,167 | 4,507.04 | 1,213,712 | 21,642 |
| Local | 64 | 356,129 | 4,463.04 | 1,443,988 | 21,265 |
| General block | 64 | 3,842,583 | 4,699.04 | 15,917,328 | 546,250 |

All 18 workload step counts exactly match the baseline. The literal single-read
case is about 1.70x faster and the general 64-read case about 2.21x faster; the
general path still copies the full list and requests substantially more bytes.
Remaining input admission, traversal and cloning costs are not eliminated.

The targeted command below passed 181 tests, including empty-frame scope,
sibling binder isolation, depth 128/129 and work 100,000/100,001 boundaries:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib --test type_list_index --test type_list_inputs --test instance_budget --test function_types --test environment_types --test normal_values --test type_equality
```

Test report: `/tmp/luna-tools-skgdmtqz/result.json`. Benchmark build:
`/tmp/luna-tools-hvy3hvcu/result.json`. All five runs exited 0:
`/tmp/luna-tools-97zpyz2b/result.json`, `/tmp/luna-tools-7jt7guge/result.json`,
`/tmp/luna-tools-yp74_5cn/result.json`, `/tmp/luna-tools-4iep_uuu/result.json`,
`/tmp/luna-tools-iku3sfei/result.json`.

### Allocation-free disposal of leaf descriptions

Freezing an invocation input releases the old type descriptions through
`Term::drop_iteratively`. That routine now handles its root before allocating a
pending-child vector. Leaf disposal therefore performs no allocation, including
owned scalar payloads and empty containers. Descendant disposal is still
iterative; rejection cleanup does not fall back to recursive host-stack drops.

The same five-process benchmark after this change, 4096 elements:

| Index form | Reads/call | ns/call | Allocations/call | Requested bytes/call | VM steps/call |
|---|---:|---:|---:|---:|---:|
| Literal | 1 | 265,908 | 30.04 | 920,156 | 20,508 |
| Literal | 64 | 285,273 | 411.04 | 984,336 | 21,642 |
| Local | 64 | 329,487 | 367.04 | 1,214,612 | 21,265 |
| General block | 64 | 3,854,703 | 603.04 | 15,687,952 | 546,250 |

Across all 18 workloads, allocation calls fall by exactly the list length and
VM steps remain identical. Fast-path times improve by about 8–9% in the rows
above; the general-path time is effectively unchanged. Remaining bytes include
input and frozen-list storage, which this change does not remove.

71 targeted tests passed: 33 NIR disposal/error-result tests at
`/tmp/luna-tools-m7az___k/result.json`, and 38 VM list/budget/normal-value tests at
`/tmp/luna-tools-vryat_lr/result.json`. The disposal tests count allocations on
the current thread and separately release a 65,536-level mixed tree on a bounded
host stack. Commands ran in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p nir --test type_disposal --test type_disposal_allocations --test error_results
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --test type_list_index --test type_list_inputs --test instance_budget --test normal_values
```

Benchmark build: `/tmp/luna-tools-1gie_dyk/result.json`. All five runs exited 0:
`/tmp/luna-tools-dlz4nte5/result.json`, `/tmp/luna-tools-yl5cph9d/result.json`,
`/tmp/luna-tools-9dd42r38/result.json`, `/tmp/luna-tools-q0bap4rb/result.json`,
`/tmp/luna-tools-v_77sh16/result.json`.
