# Comptime invocation execution and definition builders

## Scope and current boundary

Invocation now has a linear engine-code path alongside the migration tree
interpreter. `comptime_compiler` lowers supported closed function bodies before
execution; `comptime_interpretor::bytecode` runs a program counter, fixed dense
registers and explicit lexical scope records. Mixed-stage interpretation and
scheduled runtime cleanup continue to use the existing interpreter. This is not
yet a complete bytecode VM: calls, general primitives, effect/query suspension,
specialized-code caching and the unified value representation remain outstanding.

Compilation traverses control with a bounded heap worklist, including untaken
branches. Unsupported operations select the old body path before any bytecode
instruction executes; execution errors never trigger fallback. Constants borrow
immutable typed NIR leaves for this invocation; they are not frozen query-cache
artifacts. Immutable package-body code can be reused within one interpreter.
No opcode delegates a child expression tree back to `eval`.

Supported bodies include existing literal/frozen-value/Type leaves, reads,
local initialization and whole-slot assignment, blocks, boolean short circuit,
boolean/BitVec negation, branches, loops, break/continue and return. Integer
arithmetic, exact/machine floating arithmetic, scalar comparisons and admitted
comptime scalar intrinsics execute linear instructions,
using the same checked primitive operations as tree interpretation. Temporary
registers are allocated densely independent of LocalId values. Scope exit and
loop transfers clear introduced bindings; imported inputs are restored on both
success and failure so later contract checks retain prior mutations.

Reads reuse the checked invocation value-copy/value-reference mechanism.
Whole receiver updates use the same comptime value-reference storage writes;
they do not introduce pointers or require runtime Copy. Phase admission,
parameter binding, requirements, preconditions and postconditions remain at the
ordinary invocation boundary. Runtime IIR cleanup is not silently bypassed.

Compilation currently has a separate 100,000-work-item limit. Traversal, worklist
scheduling, instruction fixups and register allocation are bounded. Execution
charges frame storage, each opcode and existing scalar/value operations against
the invocation budget. Cost counts can differ between the two carriers, as
allowed by docs/comptime/normalization-and-instantiation.md. There is no speedup
claim: temporary specialized bodies and closure bodies still compile per call;
package-body reuse does not establish a persistent cross-query code identity.

`bytecode_invocations()` exposes actual route selection for verification. Tests
must inspect this counter in addition to result equality, so a successful tree
fallback cannot masquerade as bytecode delivery. The compiler's S-expression
Display preserves source ranges, local-slot mapping, jumps and constant dumps.

On the remaining tree path, ordinary single-owner locals use directly indexed slots. Mixed-owner captures,
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

## Builtin list-pattern extraction (2026-10-28)

The shipped `meta.List` comptime pattern contribution now binds the converted
usize index before indexing `self`. This uses the VM's existing local-list leaf
read: only the selected element is copied at the indexing operation. Passing
the list to the contribution and immutable input validation still have costs;
this does not make the entire pattern match constant-time. Index conversion
retains its target-width check and ordinary once-only evaluation.

The `meta_list_pattern_cost` integration test elaborates the actual shipped
contribution and a baseline with its previous inline conversion expression.
For `[first, ..._]` over a `List<Type>`, invocation steps are identical on the
32- and 64-bit targets tested:

| Elements | Previous | Local index |
| --- | --- | --- |
| 1 | 304 | 309 |
| 512 | 11,546 | 10,529 |

The large case saves 1,017 charged steps (8.8%); the small case pays five steps
for the extra binding. These are deterministic VM work counters, not elapsed
time or a general compiler speedup. The regression checks a size-dependent
saving rather than pinning exact tick totals. Empty input and budget exhaustion
are also covered. No copy admission, resource check or general index evaluator
was disabled.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test meta_list_pattern_cost --test comptime_integer_conversion --test list_patterns -- --nocapture`:
  exit 0, 2 cost tests, 5 conversion tests and 41 pattern tests passed;
  `/tmp/luna-tools-0l2xu64l/result.json`.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-meta-patterns`:
  exit 0, three real-sysroot native tests passed, including optional payloads,
  failed pattern candidates and preserved normal-form Type heads;
  `/tmp/luna-library-rbz7j2wc/summary.json`.

The initial unqualified method-call probe was not a valid interface-selection
test: importing `ListPatternView` alone does not supply its capability in the
current proposition context. Direct calls must use `.as(ListPatternView)` or
an appropriate capability assumption. The subsequent method-subject repair
keeps the original normal-form type when structural probing returns
`UnvalidatedModel`, then performs ordinary implementation selection. Other
solver failures still propagate; no normal-form body is unfolded.

The regression now covers explicit interface calls, out-of-bounds errors and
recovery on both target widths. `normal_form_methods` covers static trait
selection and rejects inheriting bool's interface through `Identity<bool>`.
Real-sysroot tests also verify a generated receiver runs exactly once. This
does not grant arbitrary opaque heads implementation eligibility or permit
inherent implementations outside the existing local-nominal ownership rule.

Method-subject repair validation, from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test meta_list_pattern_cost --test normal_form_methods --test trait_phases --test extended_calls --test enum_groups --test temporary_method_cleanup -- --nocapture`:
  exit 0, all 80 tests passed; `/tmp/luna-tools-yll_25h1/result.json`.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-meta-patterns`:
  exit 0, all three native tests passed; `/tmp/luna-library-pm4k_j92/summary.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; `/tmp/luna-tools-ue6oyl1g/result.json`. `git diff --check` passed.

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

## Value-reference append measurement (2026-10-29)

The append benchmark now uses `ArgumentPassing::ComptimeReference` and ordinary
assignment, matching the current receiver model. It no longer measures the
legacy source-pointer path. This measurement adds no invocation result cache.
`memoization::Invocation::prepare` is an explicit opt-in API requiring a pure
comptime function; production callers do not automatically route every call
through it. Purity alone is not an economic reason to cache cheap invocations.
Validation of unchanged data and reuse of an entire invocation result are
separate optimization questions.

Command, run from `/home/fin/luna` against `7a739c9e` plus this benchmark change:

```sh
python3 -B development/comptime_bench.py --bench list_append --iterations 3 --repeat 3
```

Each process warms up five times per case, then measures three invocations.
The table gives medians across three processes. Package construction, parser,
elaboration and LLVM are outside the timer. Execution, final result reading,
length assertions and VM event recording remain inside. Allocated MB means
cumulative requested bytes divided by 1,000,000, not peak live memory. Small
sample sizes support identifying allocation paths, not stable speedup promises.

- **Batch** passes the receiver once, then performs all writes in that callee.
- **Per append** makes a distinct value-reference call for every write.
- **Fused** uses the simple suffix recognized by the in-place optimization.
- **General** wraps the same suffix in a block to retain ordinary snapshot
  evaluation. Both cases assert the final list length after every invocation.

| Elements | Count | Calls | Path | ms/invocation | Allocated MB/invocation |
|---|---:|---|---|---:|---:|
| Bool | 256 | Batch | Fused | 1.118 | 1.937 |
| Bool | 256 | Per append | Fused | 2.072 | 13.765 |
| Bool | 1024 | Batch | Fused | 10.746 | 7.730 |
| Bool | 1024 | Batch | General | 15.014 | 184.994 |
| Bool | 1024 | Per append | Fused | 22.393 | 189.264 |
| Bool | 1024 | Per append | General | 26.039 | 366.527 |
| Type | 256 | Batch | Fused | 2.050 | 1.175 |
| Type | 256 | Per append | Fused | 3.086 | 3.549 |
| Type | 1024 | Batch | Fused | 27.963 | 4.689 |
| Type | 1024 | Batch | General | 23.748 | 91.619 |
| Type | 1024 | Per append | Fused | 42.659 | 36.211 |
| Type | 1024 | Per append | General | 38.510 | 123.141 |

In-place extension reduces allocated bytes but does not remove quadratic work.
The Type workload is slower on the fused path in this sample, despite allocating
less memory. There is also substantial allocation outside the extension itself:
`Interpreter::bind_invocation_inputs` calls `read_value_reference`, which creates
a checkpoint snapshot before checking the parameter domain. Repeating that
operation for a growing list copies the prefix again at every call boundary.
The batch/per-append comparison includes all call overhead, so it is not a
precise attribution of every byte or nanosecond to that one function.

The next implementation target is bounded borrowed input validation without
an unnecessary full snapshot, retaining type admission, representation/depth
limits and dependent-input materialization where needed. It should be measured
against both element domains and both calling patterns. Neither skipping
impure calls nor caching cheap results is required for this work.

Build evidence: `/tmp/luna-tools-w1pu3ibm/result.json` (exit 0). Three final
measurement runs exited 0: `/tmp/luna-tools-gnu74gr3/result.json`,
`/tmp/luna-tools-4retdt5m/result.json`, `/tmp/luna-tools-ty4ftwkw/result.json`.
All 24 cases per process completed; no global test-suite run was needed for a
benchmark-only execution change.

The existing memoization phase/failure regression was also rerun successfully:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution memoized_invocation_failures_and_phase_checks_do_not_publish_results
```

`/tmp/luna-tools-ju14y54l/result.json` exited 0 (one selected test). It rejects
impure and runtime functions at preparation and checks failure publication;
it is not evidence of an automatic cost-based cache admission policy.

## Borrowed receiver admission without list snapshots (2026-10-29)

Following the measurement above, ordinary (`Sort::Runtime`) comptime
value-reference arguments now retain a short storage borrow while checking the
parameter domain and embedded type descriptions. Their List/TypeList checkpoint
work is traversed without allocating a new list buffer. This is not cached
admission: every call still validates its current contents. Type and comptime
value inputs that materialize dependent terms retain the original snapshot
path, as do explicit program-contract snapshots. Other representations retain
the existing checkpoint implementation as a fallback.

The borrowed walker shares checkpoint depth checking and charges the same work
as copying. Differential unit tests sweep fuel and depth limits across plain,
nested, malformed and symbolic lists; they compare error kind, source and
remaining fuel. A separate input-binding regression verifies that an incorrect
payload is rejected without changing the receiver's storage.

Baseline: `0566877b`. Optimized: that revision plus this receiver-binding change.
The same benchmark command, three processes, three timed iterations and five
warmups were used; entries below are medians, not a general performance promise.
All 24 workload VM step counts are unchanged. Requested MB remains cumulative
allocation, not peak resident memory.

| 1024 elements | Calls | Path | Before ms | After ms | Before MB | After MB |
|---|---|---|---:|---:|---:|---:|
| Bool | Batch | Fused | 10.746 | 10.324 | 7.730 | 7.730 |
| Bool | Per append | Fused | 22.393 | 18.762 | 189.264 | 11.034 |
| Bool | Per append | General | 26.039 | 23.032 | 366.527 | 188.297 |
| Type | Batch | Fused | 27.963 | 28.158 | 4.689 | 4.689 |
| Type | Per append | Fused | 42.659 | 40.279 | 36.211 | 6.880 |
| Type | Per append | General | 38.510 | 36.237 | 123.141 | 93.810 |

Repeated Bool append requests about 94% fewer allocated bytes in the fused
case. Full validation still scales with list length, and the Type fused path
remains slower than general evaluation in these samples. No invocation result
cache, purity inference, or automatic cache admission policy was added.

Build: `/tmp/luna-tools-chvplrr3/result.json`; measurements:
`/tmp/luna-tools-85os5wg0/result.json`, `/tmp/luna-tools-acroitmj/result.json`,
`/tmp/luna-tools-5wtxkuo5/result.json` (all exit 0). VM unit tests:
`/tmp/luna-tools-39tlx1i7/result.json` (195 passed).

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_receivers --test comptime_list_operations --test comptime_clause_inputs --test builder_invocations
```

The second command passed 164 tests, report
`/tmp/luna-tools-f05rd9tw/result.json`. The rebuilt release CLI also passed
`sysroot-meta-lists.fl`, `sysroot-edited-struct.fl`, and `sysroot-edited-enum.fl`
using `development/reproduce.py --timeout 120 --binary
/tmp/luna-dev-002/target/release/luna -- development/examples/<name>.fl
--sysroot library --test-native`. Reports respectively:
`/tmp/luna-tools-ii5j9ekx/result.json`, `/tmp/luna-tools-5fsy39sf/result.json`,
`/tmp/luna-tools-izvznfdb/result.json` (all exit 0).

## Borrowed aggregate admission (2026-10-29)

Ordinary comptime reference inputs now walk tuples, anonymous/nominal records,
variants, optional/error/newtype wrappers and nested Type descriptions without
constructing a discarded checkpoint value. Instance-argument work checking is
shared with the copying path. Resource representations retain their existing
checkpoint behavior. Input-domain and embedded-type admission still execute on
every invocation; this change adds no result or validation cache.

The differential checkpoint test covers valid and malformed nested values,
nominal arguments, multibyte field names, both phase settings, fuel 0–99 and
depth limits 1, 2 and 128. It compares error kind, source location and remaining
fuel against actual checkpoint copying. All 196 VM unit tests passed:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
```

Report: `/tmp/luna-tools-ooclq9t5/result.json` (exit 0).

A new invocation-only benchmark performs 16 reference calls per invocation on
an aggregate containing N four-boolean tuples. It includes ordinary input
cloning, binding and execution, but excludes parsing, elaboration and VM setup.
Its structural tuples exercise internal representation, not source Tuple type
identity. Run from `/home/fin/luna`:

```sh
python3 -B development/comptime_bench.py --bench borrowed_aggregate --repeat 3 --iterations 100
```

Baseline is `b86a30af` plus the benchmark; after is the same benchmark with the
borrowed aggregate walker. Each process runs five warmups. Times below are
medians of three processes; allocations are allocator calls, not retained bytes.

| Width | Before ms | After ms | Before allocations | After allocations | VM steps |
|---:|---:|---:|---:|---:|---:|
| 1 | 0.0113 | 0.0099 | 189.04 | 157.04 | 421 |
| 64 | 0.2528 | 0.1946 | 2530.04 | 1426.04 | 16486 |
| 1024 | 4.2260 | 2.9110 | 36266.04 | 19738.04 | 261286 |

Step counts are identical before and after. The largest case requests about
46% fewer allocations and takes about 31% less time in these samples. Full
validation remains linear in aggregate size on every call; these numbers do
not establish a general compiler speedup or remove repeated admission work.

Baseline measurement reports: `/tmp/luna-tools-mi33o5r4/result.json`,
`/tmp/luna-tools-uugzo08p/result.json`, `/tmp/luna-tools-wk7lu39g/result.json`.
Optimized reports: `/tmp/luna-tools-0och76or/result.json`,
`/tmp/luna-tools-9vn8jd1x/result.json`, `/tmp/luna-tools-3vxmcnc9/result.json`.
All six exited 0.

Source integration also passed all 26 tests (17 memory, 9 receivers):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_receivers --test comptime_memory
```

Report: `/tmp/luna-tools-sq896ebw/result.json` (exit 0). Workspace format check
and `git diff --check` passed; no full workspace test run was performed.

## Ordinary value input admission (2026-10-29)

The borrowed representation walker now also checks ordinary by-value arguments
in place before binding the original argument into the invocation environment.
Previously this path constructed and immediately discarded a full checkpoint.
The value is already owned by the invocation; it does not need another snapshot
solely for representation and domain checks. Dependent comptime inputs still
materialize their terms, and owned Storage still uses its checked read path.
No invocation result or admission result is cached.

A domain regression supplies a Type input followed by an ordinary bool/char
input, through both value and reference transport. Correct domains bind;
incorrect domains fail without binding the rejected value. The checkpoint
failure/fuel differential tests and malformed list/object admission tests remain
active. All 197 VM unit tests passed:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
```

Report: `/tmp/luna-tools-ansixmss/result.json`, exit 0.

The aggregate benchmark now measures both ordinary value and reference calls,
with the same 16 calls per invocation, five warmups and 100 measured invocations
per process. Baseline is `fdc1bf4f` plus the extended benchmark. Before and after
use three processes each:

```sh
python3 -B development/comptime_bench.py --bench borrowed_aggregate --repeat 3 --iterations 100
```

| Transport / width | Before median ms | After median ms | Before allocations | After allocations | Steps, unchanged |
|---|---:|---:|---:|---:|---:|
| Value / 1 | 0.01347 | 0.01255 | 235.04 | 201.04 | 549 |
| Value / 64 | 0.40948 | 0.34678 | 4656.04 | 3483.04 | 26694 |
| Value / 1024 | 7.93760 | 5.74580 | 69176.04 | 51615.04 | 425094 |
| Reference / 1024 | 2.85387 | 2.79485 | 19738.04 | 18705.04 | 261286 |

Reference calls also benefit from eliminating the outer caller's by-value input
checkpoint. Input production, argument transfer and type rebinding still allocate;
full validation remains per call. These samples show about 28% less elapsed time
and 25% fewer allocations for the largest by-value workload, not an end-to-end
compiler performance claim.

Before reports: `/tmp/luna-tools-0hb4lbon/result.json`,
`/tmp/luna-tools-1c7csmcq/result.json`, `/tmp/luna-tools-ufzob628/result.json`.
After reports: `/tmp/luna-tools-wloeys6o/result.json`,
`/tmp/luna-tools-x1a9sbn8/result.json`, `/tmp/luna-tools-2oq58xql/result.json`.
All six exited 0. Work directory: `/home/fin/luna`.

Source integration passed with the current library and builder implementations:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_receivers --test comptime_list_operations --test comptime_clause_inputs --test builder_invocations
```

Report: `/tmp/luna-tools-958d_d5e/result.json` (exit 0). Formatting check report:
`/tmp/luna-tools-iic5b8mz/result.json` (exit 0); `git diff --check` passed.

## Owned list append and direct library operation binding (2026-10-29)

The VM can now fuse `Assign(local, ListConcat(Local(local), suffix))` for an
owned List or TypeList. Eligibility requires comptime execution without a runtime
cleanup schedule, an unprojected owned slot, matching list domains and a suffix
made only of literals, Type values or local reads. Storage and value references
retain their existing separate paths. A suffix that can call user code retains
the general snapshot path, because it could replace the receiver after the left
read. Self-append obtains its suffix snapshot before mutating the destination.

The borrowed left inspection preserves representation validation and its work
charge without allocating the discarded copy. RHS/local evaluation preserves
depth boundaries. Combined size/work checks remain active; failed combined
validation truncates the tentative append, retains spent budget and restores VM
depth. This is not a linear-time list builder: repeated complete validation still
costs work proportional to the growing value.

The typed library methods concat, append and append_all now call the named
compiler operation binding directly. The ordinary bindings.list_concat wrapper
remains available, but no longer hides append's read/concatenate/write shape
behind an additional function invocation. Raw builtin spelling remains only at
the central binding declaration. This reconnects the existing value-reference
fusion to real source library methods. The new owned-local benchmark measures
direct NIR assignment, not automatic inlining of every source `xs.concat(...)`
method call.

Measurement from `/home/fin/luna`:

```sh
python3 -B development/comptime_bench.py --bench list_append --iterations 3 --repeat 3
```

Build report: `/tmp/luna-tools-8h3ok5yq/result.json`. Three process reports:
`/tmp/luna-tools-2t8mix_7/result.json`, `/tmp/luna-tools-_dg7qj1_/result.json`,
`/tmp/luna-tools-xviwhe5p/result.json`; all exit 0. Each process warms up five
invocations per case, then measures three. Tables give process medians for 1024
appends. General wraps the same suffix in a transparent block to select snapshot
evaluation; it has two extra charged steps per append on these owned cases.
These are paired paths in one build, not historical whole-compiler speedups.
The timer includes invocation, final result reading, assertions and VM events;
package construction/elaboration are excluded. No invocation result cache runs.
Allocated MB means cumulative requested bytes, not peak live memory.

| Owned element | Path | ms/invocation | Allocations/invocation | Allocated MB/invocation |
| --- | --- | ---: | ---: | ---: |
| Bool | Fused | 12.290 | 57389 | 6.597 |
| Bool | General | 14.912 | 67624 | 184.990 |
| Type | Fused | 21.050 | 31764 | 3.573 |
| Type | General | 24.048 | 34826 | 91.617 |

Reference-receiver batch cases remain in the benchmark. For Bool at 1024 items,
fused/general medians were 10.755/14.864 ms and 7.730/184.993 allocated MB.
Short sampling supports identifying the allocation path, not a stable speedup
promise. Different fuel totals reflect the separately charged paths, not cache
hits or elided user effects.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_list_operations --test builder_invocations --test compiler_operation_bindings
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-edited-struct.fl --sysroot library --test-native
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-meta-lists.fl --sysroot library --test-native
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All exited 0: 201 VM tests (`/tmp/luna-tools-clkpmrjc/result.json`), 54 List,
72 builder and five operation-binding tests (`/tmp/luna-tools-wfz3ncs3/result.json`),
four native struct cases (`/tmp/luna-tools-o5m_wj4z/result.json`) and one native
meta list case (`/tmp/luna-tools-deztbnu7/result.json`). The budget sweep now also
covers owned literal/local suffixes and self-append, checking unchanged values
on failure and successful retry. Existing source tests cover saved snapshots,
projected receiver updates, optional/nested/Type lists, incompatible domains,
readonly rebinding and effectful suffix ordering. Formatting passed at
`/tmp/luna-tools-ad5qnvyj/result.json`; whitespace checks passed.

## Bounded normal-term freezing on the dev test stack

During contextual code transport work, the unoptimized VM regression run
aborted in `normal_input_data_tests::invocation_aggregates_retain_named_callables_without_exporting_them_as_data`
while freezing the existing 130-level tuple input. The host stack overflowed
before the depth guard returned (`/tmp/luna-tools-8lrj794o/result.json`). This
run did not establish whether the overflow predated the transport changes.

Normal-term export now lives in `normal_terms.rs`. Its small recursive
dispatcher selects separate, non-inlined aggregate handlers; scalar temporary
storage no longer occupies every aggregate recursion frame. The depth-128
boundary, node/payload charging, traversal order, callable identity handling,
and iterative release of rejected data remain unchanged. No stack-size override
or release-profile workaround was used.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/debug/build/comptime_interpretor/574bc1b682d8916e/out/comptime_interpretor-574bc1b682d8916e -- normal_input_data_tests
```

The full VM target passed in the working tree (208 tests, including the code
transport work committed separately; `/tmp/luna-tools-vu_lm4tw/result.json`).
The focused existing normal-input group also passed independently through the
same built harness (`/tmp/luna-tools-5yw5t_ey/result.json`). The source CMTT,
phase-boundary and comptime-memory groups passed all 547 tests
(`/tmp/luna-tools-59p4z5mr/result.json`).

## Bounded canonical-input decoding on the dev test stack

The execution regression `canonical_comptime_arguments_validate_shapes_and_recover`
previously overflowed the host stack while decoding a 140-level canonical tuple.
The decoder's large recursive frame exhausted the stack before its depth guard.

Decoding now uses a small shared dispatcher and separate non-inlined scalar and
container handlers in `normal_values`. The depth-128 boundary, budget charging,
field traversal, callable reconstruction and final type checks are preserved.
This complements normal-term freezing above; no thread-stack override or release
build is needed. Tests accept 127 nested tuples, reject 128 and 140 with
`DepthExhausted`, and check budget exhaustion, malformed fields and subsequent
successful decoding in the same interpreter.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_values
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib
python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test builder_invocations
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All completed successfully: two focused cases (`/tmp/luna-tools-lmo8ec2l/result.json`),
210 VM tests (`/tmp/luna-tools-b15ta7rg/result.json`), 1365 execution and 73 builder
cases (`/tmp/luna-tools-j401lu05/result.json`). The formerly aborting canonical-input
case is included without a skip. Formatting passed at
`/tmp/luna-tools-5hn11z7j/result.json`. These checks validate decoding and its
consumers; they do not establish completion of the entire comptime subsystem.

## Linear invocation validation (2026-10-29)

Working directory: `/home/fin/luna`.

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p luna --test control --test comptime_bytecode`:
  passed; `/tmp/luna-tools-cnrruz1s/result.json`. Source invocation checks actual
  bytecode routing, Type values, mutation, loops, early return, receiver storage
  updates without runtime Copy and resource exhaustion. Compiler tests check
  unsupported/invalid control admission, deterministic dumps and 2,048 nested
  blocks without recursive compiler traversal. This is not evidence that all
  interpreter admission or value walkers now support that nesting depth.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --test instance_budget --test function_types --test environment_types --test type_list_inputs --test abstract_values`:
  passed; `/tmp/luna-tools-bfax1k35/result.json`. Covers existing contract,
  abstract-value, instance-budget and immutable collection boundaries.

Initial source test failure used an illegal reassignment of `let`; the fixture
now uses `var`. A second failure showed that short-circuit classification must
use the boolean operand domain, since an early-returning expression can have a
bottom result type. Lowering was corrected rather than removing the early-return
case or counting its fallback as success.

Final stage checks:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  passed; `/tmp/luna-tools-c5etb30j/result.json`. Includes failed bytecode-frame
  recovery in the same interpreter, preservation of prior writes, lexical local
  removal and dense allocation for sparse/mixed-owner LocalIds.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p luna --test control --test comptime_bytecode --test comptime_receivers --test comptime_local_types --test library_protocols`:
  passed; `/tmp/luna-tools-f3zhy1y2/result.json`. Includes real library operations
  and native tuple affine-field checks alongside source bytecode assertions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  passed; `/tmp/luna-tools-1c0hukc1/result.json`. `git diff --check` passed.

## Shared scalar execution (2026-10-30)

Integer arithmetic (arbitrary precision and target machine integers), negation,
and scalar comparisons now compile into explicit operand registers and checked
operations. The tree interpreter calls the same signature, operand and value
implementation. Invalid signatures fail before evaluating operands; integer
operand validation precedes the next operand's effects. Comparisons preserve
their evaluate-both-then-check behavior. Target widths, overflow, zero division,
abstract values and primitive work budgets keep their existing semantics.

The compiler emits operand-free descriptors with S-expression dumps; no scalar
instruction recursively evaluates a child tree. Source tests assert the actual
bytecode route for arithmetic loops and ordered operand effects. Target-integer
checks compare both execution paths and require bytecode even on failed calls.
This does not yet provide bytecode calls, float arithmetic, container operations
or a persistent compiled-code cache; no wall-time speedup is claimed.

Validation from `/home/fin/luna` (all exit 0):

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --test target_integers --test scalar_equality --test exact_floats --test abstract_values`: `/tmp/luna-tools-3ukl6uvp/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p luna --test control --test comptime_bytecode --test comptime_integer_division --test comptime_integer_conversion`: `/tmp/luna-tools-918yvt4x/result.json`.
- After adding explicit failure-order coverage: `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::tests`: `/tmp/luna-tools-ldgagel8/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: `/tmp/luna-tools-e8wc5f09/result.json`. `git diff --check` passed.

## Immutable package-code reuse (2026-10-30)

The interpreter now retains lowered code for immutable, unspecialized package
functions, identified by declaration within that interpreter's fixed function
table. This cache borrows NIR for the interpreter lifetime and stores neither
results nor environments. Impure calls and cheap operations may reuse code,
but execute on every invocation; the Type-only result-cache policy is unchanged.
Input binding, phase admission, requirements, pre/postconditions and execution
budgets remain outside the code-cache decision.

Temporary input-specialized bodies and closures are deliberately excluded until
their code ownership and stable identity are represented. A closure's origin is
not sufficient identity for its specialized body. Unsupported lowering decisions
are retained for stable package bodies; invalid control and compile-budget errors
are not. The cache admits at most 100,000 combined entries, instructions, local
mappings and register slots. It clears when the next admission exceeds that
limit, and declines to retain an oversized program. This bounds retained code
without making any cache hit a semantic requirement.

`bytecode_compilations()` reports lowering attempts independently of executed
bodies. Tests verify reuse across different arguments, repeated impure receiver
writes, unsupported callers, repeatable arithmetic failures and bounded eviction.
This is interpreter-local reuse, not persistent query caching or a replacement
for the forthcoming VM call stack.

Validation from `/home/fin/luna` (all exit 0):

- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test comptime_receivers --test comptime_local_types`: `/tmp/luna-tools-2o3slpkb/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --test target_integers --test function_types --test instance_budget`: `/tmp/luna-tools-0rtl3w_7/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: `/tmp/luna-tools-lh5szs5h/result.json`; `git diff --check` passed.

## Floating scalar instructions (2026-10-30)

Float negation and arithmetic now use the shared scalar instruction pipeline.
The tree interpreter calls the same operand-free implementation. Exact `Float`
retains decimal add/subtract/multiply and negation; no implicit exact-division
rounding policy is introduced. f16/bf16/f32/f64 retain their existing arithmetic
implementations and bit-preserving sign negation (including zero and NaN).
Signature checks precede operand evaluation, unresolved abstract operands stop
evaluation in order, and concrete operand types are checked after evaluation.

Tests require actual bytecode execution and code reuse while asserting known
round-to-even results, NaN payload/sign preservation, exact decimal results and
rejection of exact division. Source nested arithmetic checks different input
values against one retained code object. `meta.math` wrappers still route through
their intrinsic calls; scalar instruction support does not imply those calls
have been migrated into the bytecode engine.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --test exact_floats --test scalar_equality --test abstract_values`: exit 0, `/tmp/luna-tools-qhivl28m/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test exact_floats --test comptime_bytecode`: exit 0, `/tmp/luna-tools-jzpt2iy2/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: exit 0, `/tmp/luna-tools-15_4nvxi/result.json`; `git diff --check` passed.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli sysroot_half_float_arithmetic_rounds_once_and_preserves_special_values -- --exact`: exit 0, `/tmp/luna-tools-tcpztxaz/result.json`. Real sysroot f16/bf16 computation passed comptime invocation, runtime interpretation, and LLVM O0/O3 native execution.

## Checked scalar intrinsic instructions (2026-10-30)

Linear code now admits the registered comptime scalar intrinsic family through
one generic path. Signature validation accepts operand types without retaining
operand expression trees, sharing NIR's existing tag/arity/domain/result checks.
Execution first checks that a concrete comptime binding exists, then evaluates
inputs once in source order, then invokes the same scalar operation implementation
as the tree interpreter. Host bindings remain rejected. An unsafe lexical wrapper
passes through to its body after source admission; it grants no new VM capability.

Half-format FMA still calls the bounded exact fused implementation with one
rounding. Target integer widths still come from the consuming package. Execution
failures never restart in the tree interpreter. Calls to library wrappers still
use existing call preparation, but their intrinsic bodies can now execute linear
code; actual bytecode routing is asserted for the shipped meta math wrappers.
An additional direct intrinsic fixture checks three operand mutations in one
bytecode frame, across two executions of retained code.

A first dump assertion incorrectly expected a bare `void` type. It now checks
the existing NIR `(primitive void)` rendering; neither type formatting nor
intrinsic semantics were changed to satisfy that assertion.

Validation from `/home/fin/luna` (final checks exit 0):

- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_math`: `/tmp/luna-tools-w7hobhv3/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_math intrinsic_operands_execute_in_order_inside_one_bytecode_frame -- --exact`: `/tmp/luna-tools-k2ncikzd/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p nir -p comptime_compiler --lib --test control`: `/tmp/luna-tools-i3kjfibs/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: `/tmp/luna-tools-bf4pnhex/result.json`; `git diff --check` passed.
