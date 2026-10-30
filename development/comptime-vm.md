# Comptime invocation execution and definition builders

## Scope and current boundary

Invocation now has a linear engine-code path alongside the migration tree
interpreter. `comptime_compiler` lowers supported closed function bodies before
execution; `comptime_interpretor::bytecode` runs a program counter, fixed dense
registers and explicit lexical scope records. Mixed-stage interpretation and
scheduled runtime cleanup continue to use the existing interpreter. This is not
yet a complete bytecode VM: trait calls, general primitives, effect/query suspension,
specialized-code caching and the unified value representation remain outstanding.

Compilation traverses control with a bounded heap worklist, including untaken
branches. A function body or default expression selects its carrier before that
execution unit starts. A suspended caller is never replayed when a default or
callee needs tree interpretation; execution errors never trigger fallback.
Constants, type descriptors and default templates own bounded copies of immutable
NIR data; active frames retain
the code through Arc. These are not frozen query-cache artifacts. Immutable
package-body code can be reused within one interpreter.
No opcode delegates a child expression tree back to `eval`.

Supported bodies include existing literal/frozen-value/Type leaves, reads,
record/variant construction, local initialization, whole-slot/record-field assignment and
field reads, blocks,
boolean short circuit,
boolean/BitVec negation, branches, loops, break/continue and return. Integer
arithmetic, exact/machine floating arithmetic, scalar comparisons and admitted
comptime scalar intrinsics execute linear instructions,
using the same checked primitive operations as tree interpretation. Ordinary
calls to named functions, function instances and local callable values
suspend the caller and run the callee on an explicit activation stack. Temporary
registers are allocated densely independent of LocalId values. Scope exit and
loop transfers clear introduced bindings; imported inputs are restored on both
success and failure so later contract checks retain prior mutations.

Reads reuse the checked invocation value-copy/value-reference mechanism.
Whole receiver updates use the same comptime value-reference storage writes;
they do not introduce pointers or require runtime Copy. Phase admission,
parameter binding, requirements, preconditions and postconditions remain at the
ordinary invocation boundary. Runtime IIR cleanup is not silently bypassed.

Compilation uses the caller's remaining invocation budget. Traversal, worklist
scheduling, metadata copying, instruction fixups and register allocation consume
that budget, including work before unsupported or failed compilation. Stable-code
cache hits do not repeat compilation charges; cache storage remains independently
bounded at 100,000 weight units. Execution charges frame storage, each opcode and
existing scalar/value operations against the same invocation budget.
`bytecode_compilation_steps()` reports cumulative compilation charges separately
from total work so observation-cost measurements do not hide code generation.
Cost counts can differ between the two carriers, as
allowed by docs/comptime/normalization-and-instantiation.md. There is no speedup
claim: input-dependent specialized bodies and closure bodies still compile per call;
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

## Owned invocation activations (2026-10-30)

Invocation lifecycle now lives in `comptime_interpretor::invocation`. Admission,
entry, input/contract preparation, result delivery and caller restoration are
separate operations. `begin_invocation` returns a heap-owned activation holding
the callable, input environment, body ownership, instantiated return type, input
substitution and saved caller state. `complete_invocation` performs delivery and
restores the caller after either success or failure. Preparation failures also
restore an already-entered activation.

Immutable package bodies remain borrowed. Closure bodies retain their existing
Arc; specialized bodies acquire owned storage, with no reference into a temporary
preparation stack frame. Effect identity allocation remains monotonic across
restoration; transfer state remains engine-owned. Runtime cleanup, call context,
phase admission, pre/postconditions and result validation remain in their original
order. The recursive call driver and its existing depth limit are unchanged.
This creates the owned lifecycle boundary needed for explicit call-stack work;
it does not yet claim stack-independent invocation.

New tests suspend a prepared closure activation, reject a child's wrong return
type, then resume the parent with its captured environment intact. Other tests
cover nested context restoration, monotonic effect identities, failed entry and
failed preparation.

The broad execution suite exposed five elaboration failures in generic method
values. An isolated archive of HEAD `0ddcb2ba` at
`/tmp/luna-invocation-baseline-ey2lstdq` reproduced the same five failures and
1,360 passes (`/tmp/luna-tools-jxz9y5db/result.json`), using
`python3 -B development/check.py --timeout 900 -- test -p luna --test execution`.
No fixture or expected result was relaxed. The affected cases are:

- `associated_callable_values_retain_interface_and_method_type_arguments`
- `computed_associated_trait_method_values_preserve_origins_and_generic_arguments`
- `generic_associated_method_values_infer_remaining_parameters_from_context`
- `generic_method_values_accept_explicit_method_type_arguments`
- `generic_trait_method_values_use_context_or_explicit_arguments`

They fail before execution with the diagnostic that a generic function value
requires instantiation through a call or function type context. This remains an
open frontend issue, not evidence that the execution suite fully passes.

Final targeted validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --test instance_budget --test function_types --test environment_types --test abstract_values`: exit 0, `/tmp/luna-tools-d4k05lki/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test effect_contracts --test snapshot_contracts --test comptime_bytecode --test comptime_receivers --test comptime_local_types`: exit 0, `/tmp/luna-tools-h78_9lan/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: exit 0, `/tmp/luna-tools-p0glcb66/result.json`; `git diff --check` passed.
- Final `python3 -B development/check.py --timeout 900 -- test -p luna --test execution`: exit 101, 1,360 passed and the same five baseline failures, `/tmp/luna-tools-lqej_gyj/result.json`.

## Generic method-value application repair (2026-10-30)

The five baseline failures above are now resolved. Normal-form application
probing previously elaborated a projected head as a complete standalone value
before supplying its explicit clause inputs. Generic method projections therefore
failed with a missing-context diagnostic even when `<...>` provided the inputs.
The application boundary now resolves a callee, then dispatches either a stored
normal-form callable or a declaration-side method packet with its supplied inputs.
Method origin, fixed receiver/interface arguments, bounds and expected function
type validation continue through the existing method-instantiation machinery.
There is no head unfolding, fallback after a semantic failure, or test expectation
relaxation. This follows the function-value input/identity contracts in
`docs/functions/comptime_fn.md` and `docs/traits/declarations.md`.

Validation from `/home/fin/luna` (all exit 0):

- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution`: all 1,365 passed, including the five previously failing cases; `/tmp/luna-tools-kjm9dim_/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test elaboration generic`: 51 passed, including explicit method-value missing-bound and conflicting-context rejection; `/tmp/luna-tools-fgr183pf/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_form_methods --test normal_form_inference --test comptime_clause_inputs`: passed, `/tmp/luna-tools-rzby59po/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: passed, `/tmp/luna-tools-k0c6a58s/result.json`; `git diff --check` passed.

## Resumable linear execution frames (2026-10-30)

Bytecode frames now retain their code reference, PC, register slots, lexical
scopes, imported-binding map and remaining environment across returns from the
instruction driver. Initialization, bounded instruction execution and environment
restoration are separate operations. The ordinary invocation path uses the same
resume API in 1,024-instruction batches; this is an internal execution boundary,
not language-level yielding, asynchronous cancellation or an effect handler.

Completion and errors make the frame terminal. Resuming a terminal frame fails
without executing instructions or charging more work. Earlier storage writes
remain observable after environment restoration; introduced locals do not leak.
A zero-sized batch makes no progress or budget charge. Tests compare execution
in batches of 1, 2, 7 and 1,024 instructions, requiring identical final values,
restored bindings, scope cleanup and remaining budgets. Failed frames retain
their state unchanged on a rejected resume. Frame dumps use S-expressions with
PC, terminal state, live slots, scopes and imported bindings.

This is a resumable instruction driver, not yet a bytecode CALL/RETURN stack.
Code references still obey their existing package or invocation-body lifetime;
no active frame is frozen or entered in a query cache. Invocation contracts and
runtime scheduled cleanup remain on their established paths.

Validation from `/home/fin/luna` (all exit 0):

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`: 220 passed, `/tmp/luna-tools-gchzklmu/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test library_meta_math --test comptime_receivers --test comptime_local_types`: passed, `/tmp/luna-tools-avfhkxz7/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: passed, `/tmp/luna-tools-vas6rugn/result.json`; `git diff --check` passed.

## Independent compiled-code ownership (2026-10-30)

Compiled programs no longer borrow the original function expression. Constants
own copies of their NIR leaves; scalar descriptors and intrinsic binding evidence
are shared between the instructions that validate and execute them. Execution
frames retain the whole immutable program through Arc, so removing its cache
entry or releasing the temporary source body cannot invalidate a suspended PC.
This removes the self-reference obstacle to storing invocation and execution
frames together in an explicit call stack. No unsafe lifetime extension is used.

Every owned payload is preflighted through the existing NIR copy-work and depth
checks. Pending intrinsic evidence includes its signature type in that charge.
The code cache uses a conservative retention charge that includes copied payloads
and lowering work, rather than counting only instruction slots. These units are
not byte sizes or wall-time measurements. Type-depth exhaustion retains the
shared preflight's resource-budget error classification. Compilation still has
the existing 100,000-work limit, which now also covers the owned copies.

Tests compile a 4 KiB constant, reject insufficient copy fuel, release the input
and inspect retained code. Deep type constants are rejected before recursive
copying. A suspended frame resumes successfully after releasing both source NIR
and the code cache. The current declaration-keyed cache policy is unchanged;
closure/specialization reuse and CALL/RETURN dispatch remain separate work.

Validation from `/home/fin/luna` (final checks exit 0):

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p comptime_compiler -p nir --lib --test control`: `/tmp/luna-tools-ax_aqnnv/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations`: all 73 passed, `/tmp/luna-tools-d9d9nd4p/result.json`. The first run exposed 40 fixture-loading errors because real meta code used `attrs.comptime_phase` but the fixture omitted the attrs package. Commit `20d53dc2` supplies the real package and dependency edges, without changing compiler checks or builder expectations.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test library_meta_math --test comptime_local_types`: `/tmp/luna-tools-y0j2xnnx/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: `/tmp/luna-tools-u3qd42ae/result.json`; `git diff --check` passed.

## Explicit call and return stack (2026-10-30)

Ordinary by-value calls now lower to callee selection, call preparation,
per-argument validation/storage and invocation instructions. Each argument is
checked for a valid unfilled destination before its expression executes; argument
expressions execute in source order, independent of parameter positions. Nested
argument calls retain separate pending argument slots. Compiler output and live
frames include these states in their S-expression dumps.

The invocation driver owns a stack of activations and suspended bytecode frames.
A call suspends after its invocation instruction; only delivery of the callee's
checked result makes the parent resumable. Returning, failing preparation,
failing contracts and exhausting budgets all restore the entered activations in
reverse order. The implementation shares callable parameter resolution and the
existing invocation admission, specialization, binding and delivery checks with
the tree carrier. Generic function-instance leaves use their existing checked
instantiation path. Normal-form application is not reinterpreted as body invocation.

A compiled parent can call a body that still requires tree interpretation, and
that body can call compiled functions. Carrier selection happens before each
body starts; no already-executed parent work is replayed. The native recursion
limit remains because contracts and migration boundaries still use native calls.
This does not establish unrestricted-depth execution or a performance speedup.

Trait-method call plans, effect environments and IIR/scheduled cleanup remain
on the migration path. In particular an active
effect environment selects tree evaluation before body execution: scope-targeted
control transfer has not yet been implemented for suspended bytecode frames.
These execution restrictions do not change language-level call semantics.

Regression coverage includes nested source-order argument effects, recursive
calls, generic instances, contract rejection and recovery, mixed carrier calls,
invalid slots rejected before argument effects, and the single-delivery frame
protocol. Deliberately malformed NIR tests check dynamic contract enforcement;
the source verifier still rejects unproven contracts.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib`:
  222 interpreter tests, including pending-call delivery and invalid-slot checks;
  report `/tmp/luna-tools-ealltweb/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  six compiler tests, including normal-form application exclusion and dumps;
  report `/tmp/luna-tools-kmwvqbph/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test library_meta_math --test comptime_local_types --test execution`:
  bytecode, meta math, local types and 1,365 execution cases passed;
  report `/tmp/luna-tools-pnigxly_/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test builder_invocations`:
  six bytecode tests (including the added mixed-carrier and depth-recovery checks)
  and 73 builder tests passed; report `/tmp/luna-tools-61t9fq9m/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  report `/tmp/luna-tools-nxizh3sv/result.json`.

During test development, source fixtures with deliberately unproven contracts
were correctly rejected by elaboration. They were replaced with valid source
followed by explicit NIR corruption, so the VM tests exercise the intended
boundary without weakening source verification. The deep-recursion comparison
also exposed the existing tree expression-depth limit; shallow inputs still
compare both carriers, while deeper inputs check the explicit frame path and
recovery after its own invocation-depth limit.

## Suspended default-expression evaluation (2026-10-30)

Default arguments now participate in compiled call preparation. Their checked
NIR templates are retained by the code with bounded copying; the invocation
driver selects and runs each default in its own environment and execution frame.
That environment contains prepared declaration-parameter bindings, not the
caller's locals. Returning a default value restores the environment for later
defaults before resuming the caller. A default can invoke functions and nested
defaults without adding native call frames on supported compiled paths.

The shared driver distinguishes invocation completion (which checks function
contracts and restores activation state) from default completion (which retains
the enclosing invocation identity). Default errors and control flow propagate
through suspended frames without entering the pending callee or repeating
explicit argument effects. Unsupported default expressions select tree execution
independently; the enclosing compiled caller remains suspended.

The call plan still requires all explicit arguments before defaults and defaults
in increasing parameter order, with unique valid slots and the matching
parameter-binding list. Package phase validation rejects malformed plans; the
executor also checks each step before evaluating its expression. Calls without
defaults do not copy argument values into an unused default environment.

A local selected generic instance substitutes its declaration inputs into the
default through the same budgeted helper used by tree evaluation. This supports
the existing dynamic-instance preparation carrier; it does not add new source
function-value default protocols. Trait-interface defaults remain on the tree path with their existing semantics.
Value-reference argument migration is described below.

The separate `bytecode_default_evaluations()` counter records actual compiled
default executions; `bytecode_invocations()` continues to count function bodies
only. Default code is currently compiled per evaluation, not persistently cached.
No invocation result cache eligibility changes, and no speedup claim, follow
from this migration.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  223 tests passed, including defensive default-order checks before evaluating
  the next expression; report `/tmp/luna-tools-6brxjbe6/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  seven tests passed, including bounded owned default templates and escaped
  S-expression dumps; report `/tmp/luna-tools-9giivlgb/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_defaults --test comptime_bytecode --test comptime_clause_inputs --test execution --test builder_invocations --test library_meta_math`:
  seven default-frame tests, six ordinary bytecode tests, 43 clause-input tests,
  1,365 execution cases, 73 builder cases and nine shipped meta-math tests passed;
  report `/tmp/luna-tools-psw1x2ev/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-owjlk_2f/result.json`.

The failure/recovery source test uses valid recursive default computation and
checks that exhaustion prevents callee entry. Unproven machine arithmetic stays
rejected by source verification. Malformed ordering is tested both at package
admission and directly at the executor boundary; the latter is an internal test,
not a new public validation bypass. Selected-instance substitution and early
control-flow tests construct NIR carriers explicitly and do not claim new source
syntax support.

## Compiled value-reference argument transport (2026-10-30)

Compiled calls now support comptime receiver references, including whole locals,
nominal/anonymous field paths and computed temporary bases. The lowering keeps
local-slot identity instead of reading a snapshot. Computed bases execute once
and receive private invocation storage. Reference argument plans are checked
before evaluating their expressions and remain restricted to an explicit,
non-default receiver at parameter zero.

Both execution carriers share promotion of a binding to invocation storage and
checked field projection. Existing reference handles keep their rebind permission
when forwarded. Whole-value replacement through a no-rebind handle is rejected;
projecting a field retains the existing rule permitting mutation of that field.
The per-call permission can further prohibit replacing the selected receiver.
This adds no source pointer type or runtime Copy/Drop obligation.

Receiver selection precedes later arguments. Aliases therefore observe updates
made by later arguments, and structural paths are revalidated before entry and
on subsequent access. Invalidating a path does not roll back effects that already
ran, and failure still restores the suspended invocation state. Reference paths
and permissions have stable S-expression dumps and bounded lowering/runtime work.

The source tests exercise compiled callers as well as callees: whole-value writes,
field receivers, forwarding, reading through a let receiver, field updates
through a no-rebind root, a default reading self, and temporary receivers.
An internal malformed-NIR test invalidates an acquired path in a later argument
and verifies rejection before entry while preserving the completed storage write.
Record-field access and record construction are covered by the following steps;
this does not introduce new trait-call or effect support.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  224 tests passed, including later-argument path invalidation and rejecting
  reference defaults/non-receiver references before their expressions execute;
  report `/tmp/luna-tools-vrhtcbkd/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  eight tests passed, including reference path and permission dumps;
  report `/tmp/luna-tools-ye_opow4/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_references --test comptime_bytecode --test comptime_bytecode_defaults --test comptime_memory --test execution --test builder_invocations`:
  reference, ordinary call, default, comptime memory, 1,365 execution and 73
  builder cases passed; report `/tmp/luna-tools-21zabtal/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-41l6stgr/result.json`.

## Compiled record-field access and assignment (2026-10-30)

Nominal and anonymous record fields now have bytecode reads. Direct local reads
borrow the existing value/storage/reference and copy only the selected field;
computed bases execute once and their temporary aggregate is released after
selection. The shared reader retains nominal owner checks, anonymous field name
and index checks, result-type validation for anonymous fields, and bounded value
copying. This is the same local-reader mechanism used by builder interpretation,
without embedding builder-specific rules in the VM.

Assignments now validate root binding presence before evaluating the RHS, then
resolve their complete field path after RHS evaluation. The tree and bytecode
paths share comptime writes for plain values, promoted storage and forwarded
references. This preserves sibling writes and receiver mutations performed by
the RHS, including promotion of a previously plain root to reference storage.
Runtime pointer and scheduled cleanup paths retain their existing implementation.

Regression cases check compiled field getters/setters, computed bases, writes to
siblings during RHS evaluation, invalid anonymous names/indices/result types,
and missing assignment roots rejecting before RHS effects. The following step
adds record construction; the complete aggregate and collection instruction
families are not yet migrated.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  226 tests passed, including anonymous-field rejection and pre-RHS binding
  validation; report `/tmp/luna-tools-t6ced0tn/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  eight tests passed; report `/tmp/luna-tools-fc2tvzts/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_references --test comptime_bytecode --test comptime_bytecode_defaults --test execution --test builder_invocations`:
  reference/field, call and default groups, 1,365 execution cases and 73 builder
  cases passed; report `/tmp/luna-tools-8nxkhm_u/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-yzkbz3eh/result.json`.

## Compiled record construction (2026-10-30)

Named and anonymous records now construct values through bytecode after their
field computations complete. Their final assembly and validation use the same
helper as tree evaluation, retaining generic arguments, nominal declaration
identity, anonymous labels, selected associated types and complete-value checks.
Fields are ordinary values; no library-specific or tuple normal-form rules were
added. Runtime interpretation keeps its prepared-value cleanup boundaries.

Struct initialization follows declaration order, as specified by
`docs/type/user-defined-types/structs.md`, regardless of the spelling order of
named inputs. Lowering preserves the checked NIR initialization plan. This is
different from ordinary function argument source order. The first regression
fixture incorrectly expected source order; checking the formal struct contract
showed that the existing initialization plan was correct, so its assertion now
requires the documented declaration order without changing production semantics.

A construction input that exits early skips remaining fields and final assembly.
A failed complete-value check does not publish a partial record or replay field
effects. Compiler templates own bounded type metadata, register inputs preserve
nested computations, and the record instruction has an S-expression dump.

Source tests cover reverse-spelled fields, generic record arguments, nested
anonymous records collected by a generic callable, and the complete constructor /
receiver mutation / field-read path. Other aggregate and collection families,
trait calls, active effects and scheduled cleanup remain separate migration work.

Default-override tests additionally confirm that an explicit field skips the
default computation, while an omitted field invokes it once. Empty records retain
their nominal identity and complete-value validation.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  227 tests passed, including constructor rejection after completed effects and
  early exit before later fields; report `/tmp/luna-tools-izisphp0/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  eight tests passed; report `/tmp/luna-tools-810poenn/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_records --test comptime_bytecode --test comptime_bytecode_references --test comptime_bytecode_defaults --test builder_invocations --test execution`:
  record/call/reference/default groups, 73 builders and 1,365 execution cases
  passed; report `/tmp/luna-tools-5pn9srb0/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_records`:
  all three final record tests passed, including the added default-override and
  empty-record cases; report `/tmp/luna-tools-4nq1larg/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-dgyabdvq/result.json`.

## Compiled enum variant construction (2026-10-30)

Enum payload computations now lower to registers and a checked variant
construction instruction. The instruction retains the declaration, generic
arguments, selected member index and payload values. Its completion path shares
associated-type selection and complete-value validation with tree interpretation.
The member index is logical identity; target encoding and layout continue to
follow the existing enum representation rules.

Payload evaluation preserves the checked initialization plan and ordinary early
control flow. An early return from one payload expression skips the remaining
payloads and construction. Invalid member indices, missing payloads and wrong
payload domains are rejected; failure does not poison subsequent invocations.
Source tests check nullary, positional and named variants, generic arguments,
initialization order and actual bytecode routing alongside tree results.

Record and variant evaluators and their completion checks now live together in
`compiler/comptime_interpretor/src/aggregate_values.rs`. Tree evaluation retains
its prepared-value cleanup scopes; the compiled path reuses assembly without
re-entering expression-tree evaluation. S-expression dumps preserve nested enum
heads and selected members.

Enum matching, remaining aggregate/collection operations, trait call plans and
active effect environments are still separate migration work. This step does not
change their semantics or claim new layout/ABI support.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_variants --test comptime_bytecode_records --test comptime_bytecode_references --test builder_invocations --test execution`:
  variant, record, reference, 73 builder and 1,365 execution cases passed before
  extracting the shared aggregate module;
  report `/tmp/luna-tools-zd8ngqu6/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  227 tests passed after extraction; report `/tmp/luna-tools-mgjfv38d/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_variants --test comptime_bytecode_records --test builder_invocations`:
  both variant tests, three record tests and 73 builder tests passed after
  extraction; report `/tmp/luna-tools-j_hnb38s/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  nine tests passed, including nested variant dumps;
  report `/tmp/luna-tools-l7w0hi25/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-o3owwu6c/result.json`.

## Compiled optional and error construction (2026-10-30)

Optional absence/presence and error success/failure injection, row embedding and
product representation transport now have bytecode instructions. Preparation
checks the conversion plan and admits the target type before evaluating inputs;
completion validates the actual payload after evaluation. Both carriers share
these checks. Invalid metadata therefore cannot run operand effects, while a
wrong actual payload does not undo effects that already occurred.

Optional construction preserves every absence layer. Error transport preserves
full label identity and payloads even when branch indices change. Product
transport preserves field types and does not introduce aggregate covariance:
reordering equivalent error rows inside a record is supported; widening a
record field's error row through product transport is rejected. The first new
product test incorrectly requested widening and failed on both carriers; it was
corrected to test reordering, with widening retained as a rejection case and its
operand separately checked to be valid.

Preparation and completion appear separately in S-expression dumps. Bytecode
still excludes scheduled cleanup and active effect environments; the tree path
keeps its prepared-value scopes. Error propagation and pattern matching remain
separate migration work. Invocation result-cache eligibility and error ABI are
unchanged. No performance speedup is claimed without measurement.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_qualified --test execution --test builder_invocations`:
  exit 0; two source-level routing/identity tests, 1,365 execution cases and 73
  builder cases passed; report `/tmp/luna-tools-01krofsn/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  exit 0; 229 tests passed, including pre-input rejection, post-input failure and
  record transport; report `/tmp/luna-tools-2ecpqgn1/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  exit 0; nine compiler/control tests passed;
  report `/tmp/luna-tools-xtq0ekvp/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::tests::product_error_transport_preserves_labels_and_payloads_inside_records`:
  exit 0 after strengthening the rejection fixture with a valid-operand check;
  report `/tmp/luna-tools-apf6xs6e/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; report `/tmp/luna-tools-f6h587sg/result.json`.

## Compiled structural matching and error propagation (2026-10-30)

NIR error propagation is ordinary matching followed by a return, so the VM now
compiles general match control flow rather than adding an error-only evaluator.
The subject is evaluated once. Structural candidate matching shares the existing
pattern journal, including alternative rollback, binding remapping, nominal and
normal-form head identity, optional layers and error labels. Candidate bindings
are published to registers only after the whole structural pattern succeeds.
Each arm has a lexical scope. Guard failure removes its new bindings while
retaining already completed outer mutations; return, break and continue use the
existing control-transfer machinery.

Outer pattern guard wrappers execute from the innermost outward, followed by an
explicit arm guard, using normal compiled expressions. Their calls can suspend
and return through the VM's invocation stack. Pattern-local computation within a
structural child or alternative (`Then` or nested `Guard`) still selects the tree
carrier before execution starts. This is an implementation boundary, not a
restriction on source-language patterns. Scheduled cleanup and active effects
also retain their existing tree carrier.

Owned structural pattern copies are bounded before cloning. Exceeding the
representation's safe clone depth selects the tree carrier; it must not be
reported as exhaustion of execution steps. The existing small-host-stack depth
regression exposed that distinction during this change, and its original
`DepthExhausted` expectation is preserved. Actual compilation fuel exhaustion
remains a separate failure. Matching instructions, bindings, branches and the
non-exhaustive trap all have stable S-expression dumps.

Validation from `/home/fin/luna`, all final commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  230 tests passed, including invalid guard/exhaustion diagnostics, binding
  cleanup and subsequent recovery; report `/tmp/luna-tools-chm57pwr/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_matching --test execution --test builder_invocations`:
  four matching/propagation source tests, 1,365 execution cases and 73 builders
  passed; report `/tmp/luna-tools-m8n3fj2u/result.json`. Source tests assert actual
  bytecode invocation counts as well as results and tree-carrier agreement.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  11 tests passed, including guard branches, binding dumps, bounded copies and
  refusal of unsupported pattern computations before execution;
  report `/tmp/luna-tools-so1ami_d/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-twh07q9f/result.json`.

## Compiled destructuring declarations (2026-10-30)

`LetPattern` now uses the same structural candidate and compiled guard preparation
as match arms. Its initializer executes before the candidate scope is opened.
Successful declarations commit new bindings into the enclosing lexical scope;
failed candidates discard their new bindings before entering `else`, retaining
writes to outer places. This permits mutable destructured bindings to be used
and updated by subsequent statements and releases them at the enclosing exit.

A rejected declaration without `else` reports non-exhaustive matching. A malformed
NIR `else` which completes normally reports invalid control flow; valid return,
break and continue transfers retain their existing semantics. Source tests cover
enum/optional declarations, guard effects, early return and continue, with actual
bytecode routing checked against the tree carrier. Internal tests cover both
rejection diagnostics without weakening the front-end rule that `else` must exit.

Pattern preparation is shared with match lowering. Nested executable pattern
children, active effects and scheduled cleanup retain the same migration limits.
An initial test also encountered unsupported `TypedWith` AST lowering for
`(null: ?bool)`; the declaration/continue test supplies its type using a separate
typed declaration. Subsequent review found that the grammar retains this syntax
but does not establish its result semantics, while historical HIR classifies it
as a proposition. Author clarification is pending; this is not evidence that
value-preserving type ascription is a confirmed missing feature.

Default-template regression coverage now keeps both routes: a structural match
is compiled, while sequential pattern computation uses the tree carrier. Both
must preserve the caller's single evaluation of explicit arguments.

Validation from `/home/fin/luna`, all final commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  231 tests passed; report `/tmp/luna-tools-2i80r44m/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_bindings --test comptime_bytecode_matching --test comptime_bytecode_defaults --test comptime_bytecode_references --test execution --test builder_invocations`:
  two declaration, four matching, seven default, four reference, 1,365 execution
  and 73 builder cases passed; report `/tmp/luna-tools-mh1w653f/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  12 tests passed, including declaration scope/diagnostic dumps;
  report `/tmp/luna-tools-eavlxbuo/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-eeoj3vol/result.json`.

## Compiled tuple and ordered Object assembly (2026-10-30)

Tuple value assembly now follows the existing NIR element order and ordinary
control transfers. This does not construct or unfold a Tuple type, change its
normal-form head, or privilege type-list inputs. Tuple construction followed by
destructuring can remain in one compiled invocation.

Object construction retains ordered positional/property entries, duplicate keys
and each entry's declared domain. A preparation instruction checks the phase,
result type and metadata arity before evaluating entries. Each evaluated entry
is validated and charged before the next entry starts. Completion assembles the
owned Object without sorting or merging keys. An early return skips later
entries and assembly. Tree interpretation uses the shared preparation and cost
checks while retaining its existing prepared-value cleanup order.

Object edits and queries still have their original evaluators; active effects,
scheduled cleanup and nested executable patterns remain separate migration work.
The Object constructor descriptor and instructions provide escaped S-expression
dumps, including duplicate property names and positional entries.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  exit 0, 232 tests passed, including metadata rejection before effects and
  per-entry validation before later effects;
  report `/tmp/luna-tools-drnxsb1a/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_aggregates --test library_meta_objects --test comptime_object_storage --test execution --test builder_invocations`:
  exit 101. Both new source tests, four Object snapshot tests, 1,365 execution
  cases and 73 builders passed. The meta Object suite passed 24 of 25 cases;
  `repository_package_exports_object_for_user_attribute_constructors` failed
  during multi-package attribute elaboration with `6877..6882: expression
  elaboration requires a protocol or construct not yet implemented`.
  Report `/tmp/luna-tools-9f_ykvjn/result.json`.
- Baseline reproduction: temporarily restored the three modified implementation
  files to commit `76c18b52`, ran
  `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects repository_package_exports_object_for_user_attribute_constructors`,
  then restored all in-progress contents in a `finally` block. The baseline
  failed with the same diagnostic (exit 101);
  report `/tmp/luna-tools-gq79hgby/result.json`. This pre-existing integration
  failure remains open; it is not counted as a passing library check.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --test control`:
  exit 0, 13 tests passed, including Object key escaping and ordering;
  report `/tmp/luna-tools-u088sq4_/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; report `/tmp/luna-tools-rkm6its8/result.json`.

## Resolve the meta.Object package integration baseline (2026-10-30)

The previously recorded attribute-inspection failure was an incomplete explicit
package fixture, not an Object invocation failure. Its unresolved HIR expression
was `attrs`: the real meta optional/error protocols use `attrs.comptime_phase`,
but this fixture supplied only meta and builtin. It now supplies the real attrs
package and both dependency edges (meta to attrs, attrs to builtin). The user
attribute still must appear exactly once; library phase attributes are checked
separately rather than suppressed to preserve the old total count.

Elaboration now reports an unresolved expression's name and points to lexical
imports/package dependencies instead of treating it as an unimplemented
construct. Existing negative attribute/configuration tests check the actual
missing names. Deferred HIR inspection continues to retain unresolved names;
executable elaboration still rejects them. No implicit package injection or
name-based dependency inference was added.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects repository_package_exports_object_for_user_attribute_constructors`:
  exit 0; the previously failing explicit-package integration now succeeds;
  report `/tmp/luna-tools-r8yr0vpw/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects --test frontend_cli`:
  exit 101; frontend CLI had 240 passes and 12 failures. Both updated missing-name
  diagnostic tests passed. The remaining failures expose fixture issues outside
  this change: three direct-written sources lack bool bindings, six assertions
  compare source bytes without the fixture-added bool alias, two copied library
  paths no longer exist, and one sysroot fixture defines bool twice. Their
  diagnostics occur in source resolution, filesystem reads or source-byte
  assertions, not the changed unresolved-expression branch. Full failure names
  and output: `/tmp/luna-tools-hj397x34/stdout.log`. They remain open; no full
  frontend-suite pass is claimed. Cargo stopped before the Object suite, so that
  suite was scheduled separately.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects`:
  exit 0; all 25 public Object tests passed;
  report `/tmp/luna-tools-cjbsa1zf/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; report `/tmp/luna-tools-5wy2bw2_/result.json`.

## Restore frontend source-protection checks (2026-10-30)

Nine of the recorded frontend fixture failures are corrected without changing
compiler behavior or weakening their success/rejection requirements. Three
comptime CLI fixtures now use the existing standalone primitive-binding writer,
so their bool signatures reach invocation and contract checks. Source-protection
and observed-input assertions capture the complete written source before running
the compiler, including those explicit fixture bindings. They continue to check
exact bytes after attempted overwrites and exact escaped contents in reports;
artifact preservation, hard-link protection, native execution and type-contract
failure checks remain intact.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli explicit_package_cli`:
  exit 0; report `/tmp/luna-tools-3lwoqh7s/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli -- --exact comptime_invocation_executes_real_closures_defaults_and_writes_complete_values comptime_invocation_rejects_wrong_entries_and_preserves_output_on_failure comptime_invocation_uses_type_solver_and_preserves_outputs_on_contract_failure explicit_package_cli_compiles_cross_package_calls_and_preserves_outputs explicit_package_cli_exports_observed_inputs_with_the_selected_result explicit_package_cli_input_envelope_is_published_only_after_success explicit_sysroot_protects_dependency_sources_and_reports_invalid_packages package_llvm_dump_selects_primary_identity_and_preserves_inputs package_native_build_preserves_identity_resources_and_previous_artifacts`:
  exit 0; all nine selected tests passed, 243 filtered out;
  report `/tmp/luna-tools-ig048hap/result.json`.

The remaining three recorded frontend failures concern shipped-library fixture
paths/dependencies and a duplicated primitive binding; they are not addressed by
this source-protection correction. No full frontend-suite rerun was needed for
these bounded fixture changes.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; report `/tmp/luna-tools-zxlthwpb/result.json`.

## Exercise the remaining fixtures against the real sysroot (2026-10-30)

The three remaining recorded failures now use a raw copy of the repository's
library source hierarchy and language-item manifest. They no longer flatten old
module paths or inject standalone bool aliases into library files. Semantic runs
supply an explicit x86_64/Linux/64-bit target, required by the full library's
runtime integer proxies. The HIR dump is written to a file and checked there so
the test runner's bounded stdout capture cannot truncate the tree.

Operator checks use local user types with the actual builtin Add protocol,
covering its default Self input and named Rhs input without a foreign-trait /
foreign-type implementation. Clone tests obtain Once from the library's blanket
implementation and retain both source-preserving cloning and resource-checked
execution. Their rejection cases actually request a missing Clone implementation
or supply an incompatible receiver; lack of a redundant Once impl is no longer
incorrectly treated as failure. Comparison and Hash behavior remain checked.

The List/Tuple test uses public primitive names from the prelude. It alters only
the tuple selector when testing explicit CLI overrides, preserving unrelated
library registrations, and still rejects replacing the registered tuple head
with an unrelated source definition. This closes the previously recorded three
fixture failures without changing compiler semantics.

Validation from `/home/fin/luna`, final commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli -- --exact shipped_operator_protocols_execute_with_default_and_named_types shipped_clone_protocol_preserves_noncopy_sources sysroot_language_items_bind_real_list_and_tuple_definitions_by_default`:
  all three passed, 249 filtered out;
  report `/tmp/luna-tools-74qdbwjp/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli sysroot_language_items_bind_real_list_and_tuple_definitions_by_default`:
  passed after replacing legacy primitive accesses with prelude names;
  report `/tmp/luna-tools-mfg4dnt3/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  report `/tmp/luna-tools-pfq_lkt4/result.json`.

Together with the preceding nine-case run, every failure from the recorded
12-case frontend list has now passed its focused regression. The entire 252-case
suite has not been rerun after these fixture corrections; no new full-suite
result is claimed.

## Object query and edit bytecode

Object Length, Key, EntryType, Value, NamedValue and Concat, Append, Replace,
Remove now compile into checked operation metadata plus ordinary operand code.
The prepare instruction validates phase and result/input contracts before any
operand runs. Completion consumes the evaluated packet through the same helpers
as the tree interpreter; no instruction evaluates a NIR expression tree.
Metadata has bounded copying and an S-expression Display.

Ordered duplicate keys, last-key domain mismatch, nested optionals, checked
indices, replacement domains and functional snapshots retain their existing
rules. Computed selectors run after taking the receiver snapshot. Failed edits
cannot publish a partially modified receiver; receiver writeback remains the
ordinary assignment path used by meta.Object.set.

A source regression asserts actual bytecode invocation counts for all operation
families and for EntryType. Its initial combined body exposed the existing
TypeEqual fallback: the correct result alone did not prove bytecode execution.
The final regression observes EntryType separately and checks its exact Type
value in the harness; the library integration tests still cover type comparison.
TypeEqual compilation remains unimplemented. The VM currently uses evaluated
packets, not the tree interpreter's borrowed local-query optimization; migration
of that optimization and wall-clock performance measurements remain outstanding.
No speedup is claimed from instruction coverage alone.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  233 passed, including a tree/bytecode matrix for every operation, wrong payloads,
  missing keys, bounds, duplicate domains, recovery, phase and pre-input checks.
  Exit 0, `/tmp/luna-tools-ynmardig/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler`:
  13 passed, exit 0, `/tmp/luna-tools-nnztdibu/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects --test comptime_object_storage --test comptime_bytecode_aggregates --test execution --test builder_invocations`:
  builder_invocations passed all 73 cases; the initial aggregate regression failed
  its bytecode-count assertion on TypeEqual, stopping the remaining suites.
  Exit 101, `/tmp/luna-tools-kjew2mvt/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_aggregates --test library_meta_objects --test comptime_object_storage --test execution`:
  final source aggregates 3, storage 4, execution 1365 and real-library Object 25
  passed. Exit 0, `/tmp/luna-tools-a16iqp4s/result.json`.

An earlier command named a nonexistent `comptime_builders` target and ran no
tests (`/tmp/luna-tools-tmm7ub7x/result.json`); the correct target above is
`builder_invocations`. These checks do not establish full language completion.

- Final `cargo fmt --all -- --check` through `development/command.py`: exit 0,
  `/tmp/luna-tools-36yph1kc/result.json`.
- Final `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_aggregates`
  after binding Type explicitly in the bootstrap fixture: 3 passed, exit 0,
  `/tmp/luna-tools-ymi01ja5/result.json`.

## Type equality in bytecode

TypeEqual now has explicit contract preparation, operand-to-Type-description
validation and proof instructions. The left value is validated before the right
operand executes. Both carriers share `type_equality.rs`, retaining the existing
host solver interface and metered proof work: structural identity proves true;
otherwise equality and its negation are asked separately. Neither Unknown nor
Rejected alone proves inequality. An unresolved comparison reports
TypeContractUnresolved, and exhausted proof fuel stops before another query.
Normal-form heads remain opaque; no body unfolding or value-cache expansion was
introduced. The Object source regression now includes EntryType equality in the
same bytecode invocation.

Normal-form construction itself still uses its existing input-plan interpreter.
An initial source test incorrectly expected its caller and comparison callee to
both compile; the count exposed the caller fallback. The final test constructs
an opaque Type first and directly verifies that its comparison compiles, remains
unresolved, and permits subsequent successful invocations. Existing factory
integration tests also retain the construction-plus-comparison path. New compiler
metadata uses the ordinary bounded copy mechanism and S-expression Display.

Validation from `/home/fin/luna`, final commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  existing 233 tests passed after implementation extraction;
  `/tmp/luna-tools-6w7zi4oa/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::type_equality_tests`:
  3 new tests passed across both carriers, including proof query order, rejection
  versus negation, malformed operands, budget exhaustion and recovery;
  `/tmp/luna-tools-_wocfq37/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler`:
  14 passed, including instruction ordering, bounded compilation and stable dump;
  `/tmp/luna-tools-r6_f3rs5/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test comptime_bytecode_aggregates --test execution --test library_meta_objects`:
  source types 1, aggregates 3, execution 1365 and Object library 25 passed;
  `/tmp/luna-tools-_e924lbn/result.json`.

Initial test-only failures were corrected: Rust enum/assignment field spelling
(`/tmp/luna-tools-v1cfsd2r/result.json`), an expected dump missing primitive nodes
(`/tmp/luna-tools-okhdl0zu/result.json`), and the normal-form caller's execution
count (`/tmp/luna-tools-dkz8upix/result.json`). No semantic checks were relaxed.
- Final `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0, `/tmp/luna-tools-xbf_o692/result.json`; `git diff --check` also passed.

## Explicit continuations for delayed normal-form inputs

Normal-form construction cannot be treated as eager untyped argument assembly:
its source order can put a contextual value before the Type that checks it.
`stage_normal_argument` and `finish_staged_normal_argument` now traverse those
inputs with explicit work stacks. Staging handles lists, tuples, selected
branches, blocks and matches; completion combines values after contextual
preparation supplies the required domains. List traversal schedules one child
at a time, avoiding a work queue proportional to the entire unevaluated width.

Logical depth limits, per-node ticks, late-domain validation, literal identities
and head opacity remain unchanged. Scopes unwind on error, return and exhausted
fuel while preserving mutations of bindings that existed on entry. Type-list
spread checks still run before advancing to the next item; tuple interpretation
still follows the prepared contextual domain. No special Tuple head or package
name was added to dispatch.

This removes host recursion through staged input containers, not all recursion
from comptime invocation: ordinary expressions and guards still use shared
evaluation, and normal-form input plans are not yet bytecode. Pending values are
invocation-local and are not eligible for freezing into code/result caches.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  236 existing tests passed, `/tmp/luna-tools-akuikzah/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_inputs::tests`:
  2 new tests passed. A 512 KiB thread stack exercises nesting through the
  128-node limit and subsequent recovery. A fuel sweep verifies nested scope
  cleanup on values, errors and returns, retaining outer binding effects.
  `/tmp/luna-tools-65dfw743/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test builder_invocations --test comptime_bytecode_types`:
  execution 1365, builder invocations 73 and bytecode Type equality 1 passed;
  `/tmp/luna-tools-5ib8arju/result.json`. Execution coverage includes late Type
  inputs, checked unselected branches, tuple/match staging, list spreads,
  source-order effects, defaults and early transfers.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  passed, `/tmp/luna-tools-qyx46qj7/result.json`; `git diff --check` passed.

## Structural Type constructors in bytecode

Computed FunctionType, FunctionResultType, ClosureType, OptionalType and
PointerType now compile with owned, bounded metadata and ordinary operand
instructions. Preparation checks output/input domains and parameter-flag arity
before evaluation. Each operand is converted to a Type description immediately;
FunctionResultType also validates its signature before evaluating its output.
Construction retains callable wrappers, invocation phase, runtime/Type/comptime
value parameter sorts, closure environments and empty-environment elimination.

The tree evaluator and VM share the checked operations in `type_constructors.rs`.
This extracts the two former construction evaluators from the interpreter root;
there is no tree-evaluation opcode and no package/name-specific shortcut.
Metadata and instructions have stable S-expression displays. Type construction
is independent of runtime pointer values: PointerType creates a Type value, not
a comptime pointer or reference storage.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  existing 238 tests passed, `/tmp/luna-tools-9sm5mx5v/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::type_constructor_tests`:
  the new two-carrier validation test passed, covering malformed output/flags,
  non-Type payloads, non-callable signatures, suppressed later effects and
  successful reuse after failure; `/tmp/luna-tools-zv_544y_/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler`:
  15 passed, including metadata staging/phase display and interleaved operand
  checking; `/tmp/luna-tools-gbic8v6z/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test function_type_dependencies --test execution`:
  source bytecode types 2, function dependencies 37 and execution 1365 passed;
  `/tmp/luna-tools-pffao6ci/result.json`. The new source test asserts actual
  bytecode counts, exact Type values and agreement with tree interpretation for
  each constructor family and both comptime input sorts.

Normal-form input-plan and nominal-type construction compilation remain separate
unfinished work. This stage does not claim wall-clock speedups or completion of
all type factories.
- Final `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  passed, `/tmp/luna-tools-q4_06plr/result.json`; `git diff --check` passed.

## Borrowed Object queries in bytecode

Object queries with a local receiver and a local/literal selector now compile
to a dedicated read instruction. Owned scalar selectors and literals can be
read while observing the receiver; the entire Object representation is still
checked, but no receiver snapshot is retained. Only the selected result is
copied. Local selectors backed by storage/references take the snapshot path,
releasing the receiver lock before reading a possibly aliased sibling. Computed
receivers/selectors keep their ordinary operand code and snapshot semantics.

This is a generic local-read optimization, independent of library names. Phase,
output-domain, payload, last-key, bounds and representation checks remain in the
shared Object helpers. The instruction carries bounded literal metadata and
source locations and has an escaped, stable S-expression display.

Validation from `/home/fin/luna`, final commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  239 passed, `/tmp/luna-tools-rx9mwe5e/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::object_borrow_tests`:
  sibling-reference reads, locked-receiver rejection and recovery passed;
  `/tmp/luna-tools-2byjqe10/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler`:
  16 passed, including local versus computed selector routing and escaped dumps;
  `/tmp/luna-tools-3zmdsbyc/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects --test comptime_object_storage --test comptime_bytecode_aggregates`:
  Object library 25, storage 4 and aggregates 3 passed;
  `/tmp/luna-tools-1bz9lodd/result.json`.
- `LUNA_BENCH_ITERATIONS=10 python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --bench object_queries`:
  all measured results matched expectations;
  `/tmp/luna-tools-exo4haox/result.json`.

The allocation-counting benchmark warms compilation before measurement. For a
1024-entry Object read 32 times per invocation, measured requested bytes were:

| Receiver / query | Snapshot path | Borrowed path |
| --- | ---: | ---: |
| Value / length | 7,902,890 | 242,026 |
| Value / indexed value | 7,914,210 | 249,250 |
| Reference / length | 7,907,994 | 247,130 |
| Reference / indexed value | 7,919,370 | 254,410 |

These totals include the benchmark's input cloning and invocation overhead.
The control path wraps the receiver in a transparent block to force snapshots;
this is a same-build path comparison, not a historical-version baseline.
Timing came from the managed development profile and ten iterations; no release
speedup claim is made. Representation validation still scales with Object size.
An initial compile used the u64 rewrite-budget API with a usize compiler budget;
it was corrected to the existing bounded substitution-copy API before testing
(`/tmp/luna-tools-irgce4m_/result.json`).
- Final managed `cargo fmt --all -- --check` passed:
  `/tmp/luna-tools-_kn0wfr8/result.json`; `git diff --check` passed.

## List construction, concatenation and length in bytecode

TypeList (the existing NIR name for generic list construction), ListConcat and
ListLength now compile into ordinary operand code with checked list descriptors.
Construction retains the registered head and actual element domain. Type-valued
inputs are converted immediately; ordinary payload validation remains at the
completed aggregate boundary. Concatenation validates its left input before the
right expression runs, retains both source snapshots and enforces element and
structural work limits before returning a result. No List spelling or Tuple head
is used for dispatch.

Shared helpers in `list_construction.rs` and `list_operations.rs` serve both
execution carriers. A local length observation reuses the existing borrowed
length helper and exact declared-domain check; it does not copy or scan every
element. Computed operands retain their ordinary evaluation path. Descriptors
and operations have bounded copying and stable S-expression displays.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  240 existing tests passed, exit 0, `/tmp/luna-tools-vschfyxk/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::list_tests`:
  2 new two-carrier tests passed for domain rejection before suffix effects,
  immutable input preservation, length identity and distinct Type-conversion /
  ordinary aggregate validation boundaries; `/tmp/luna-tools-vgdq26gr/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler`:
  17 passed, `/tmp/luna-tools-f6u_nrju/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_lists --test comptime_list_operations --test type_lists --test dependent_list_collectors --test execution`:
  the new source bytecode test passed, then 53 of 54 real-library list tests
  passed. The explicit-package CLI fixture failed before VM execution because
  it omitted the public bool import and current attrs dependency; exit 101,
  `/tmp/luna-tools-tr3kp3ny/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_list_operations ordinary_package_uses_public_list_operations_without_builtin_access`:
  the fixture passed with public `builtin.bool` and explicit package dependencies,
  preserving the original result assertion; `/tmp/luna-tools-b5_dvfp0/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_lists --test dependent_list_collectors --test execution`:
  type lists 28, dependent collectors 1 and execution 1365 passed, exit 0;
  `/tmp/luna-tools-kpw63s59/result.json`.

The initial source test incorrectly compared comptime lists through the public
runtime-only Interpreter::new entry, which rejects their transport. The source
regression instead checks exact list values and actual bytecode counts; the new
internal tests compare both carriers in comptime mode. Initial extraction also
required fixing macro scope and an iterator spelling error before tests ran.

List indexing is not yet compiled. Self-append currently follows compiled
concatenation plus assignment where eligible, rather than the tree evaluator's
fused append optimization; porting that optimization remains necessary for
allocation-sensitive builders. No general list speedup is claimed by this stage.
- Final managed `cargo fmt --all -- --check` passed:
  `/tmp/luna-tools-dvnp8igb/result.json`; `git diff --check` passed.

## List indexing in bytecode and shared borrowed reads

TypeListIndex (the existing generic list-index NIR node) now compiles with a
checked result/list/index contract. Computed receivers and indices evaluate
through ordinary code, checking the list identity before index effects and
preserving the receiver snapshot. Local receivers with a literal/local index
use a borrowed read and copy only the selected value. Negative, oversized and
out-of-range indices fail without authorizing a different element.

The former separate type-list and value-list local evaluators are consolidated
in `list_index.rs`. Their old lock order could reject a valid index reference
into a sibling field of the same storage: it tried reading the index while still
holding the receiver lock. Both carriers now snapshot and release that receiver
before reading a storage-backed selector. Owned integer selectors keep the
borrowed path. Representation kind and declared list domain are checked before
index evaluation, including deliberately malformed internal carriers; domain
inspection is metered without scanning unrelated elements.

Read-only local/literal selector metadata and evaluation are now shared with
Object queries in `leaf_inputs.rs`; no selector opcode evaluates a general NIR
tree. Metadata and instructions have bounded copying and S-expression Display.

Validation from `/home/fin/luna`, all commands exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  242 existing tests passed, `/tmp/luna-tools-8d5lu7gy/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::list_tests`:
  4 passed, including the new sibling-reference and domain-before-index-effects
  tests across both carriers; `/tmp/luna-tools-lrbaq5k0/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler`:
  18 passed, including literal/local versus computed index routing;
  `/tmp/luna-tools-ju2m0opw/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_lists --test comptime_list_operations --test type_lists --test execution`:
  source bytecode lists 2, real meta.List 54, type lists 28 and execution 1365
  passed; `/tmp/luna-tools-qyijh35j/result.json`. Source regressions assert actual
  bytecode invocation counts for bounds errors, recovery, old-snapshot selection,
  early return and Type-valued elements.
- `LUNA_BENCH_ITERATIONS=10 python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --bench type_list_index`:
  all benchmark results matched; `/tmp/luna-tools-vyklvjr4/result.json`.

For 4096 Type elements and 64 reads per invocation, the allocation-counting
benchmark requested 1,182,656 bytes with a literal index and 1,395,412 with a local
index, versus 15,978,432 on its computed-index snapshot control path. Input
cloning and invocation costs remain included. This is a warmed same-build path
comparison in the managed development profile, not a historical or release
speedup result. Existing list-loop cost regressions also passed. Fused append
and normal-form input-plan bytecode migration remain outstanding.

- Final managed `cargo fmt --all -- --check` passed, exit 0:
  `/tmp/luna-tools-6ipfkx2u/result.json`; `git diff --check` passed.

## Transactional list append in bytecode

Root-local self-concatenating assignments now compile to a borrowed admission
check, ordinary suffix code, and a transactional append. Fusion requires a local
suffix, a constant list, or a list of local/type/literal leaves; that eligibility
scan consumes compilation fuel. Computed suffixes retain concatenation snapshots.
No library name, Type-list identity exception or recursive tree opcode is used.

Owned locals retain their allocation. Shared root storage and projected comptime
receivers resolve the target after the suffix has been copied and all read locks
have been released. Checked extension rolls back its appended range on budget or
representation failure. Input admission and combined structural work are still
charged; avoiding copying does not remove repeated validation scans. Runtime
pointer writes retain the existing tree path and are not covered by this change.

Validation from `/home/fin/luna`:

- Managed `test -p comptime_interpretor --lib bytecode::list_tests`: seven tests
  passed after final additions, exit 0, `/tmp/luna-tools-esdc00ae/result.json`.
  Coverage includes buffer reuse for Bool/Type lists, root/projected references,
  aliased suffixes, malformed suffix domains/payloads, budget rollback and recovery.
- Managed `test -p luna --test comptime_bytecode_lists --test comptime_list_operations --test type_lists --test execution`:
  3, 54, 28 and 1365 passed, exit 0, `/tmp/luna-tools-xhelgubc/result.json`.
  Source tests preserve saved copies and a callback that overwrites the receiver
  during suffix evaluation. This preceded the final compiler eligibility fuel
  charge and final unit-test additions; no source behavior was changed afterward.
- Managed `test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  compiler 19 and interpreter unit 246 passed, along with earlier integration
  groups, but the run exited 101 on
  `type_list_inputs::local_list_length_still_charges_large_element_type_descriptions`;
  `/tmp/luna-tools-biwn5zqs/result.json`. Do not describe the whole suite as passed.
- A temporary diagnostic in that unchanged test compiled its body explicitly
  with 100,000 fuel before invocation. Width 1 with 64/1 reads compiled; width
  4096 with 64 reads returned `Error::Budget`, before executing a length read.
  Managed `test -p comptime_interpretor --test type_list_inputs local_list_length_still_charges_large_element_type_descriptions -- --nocapture`
  exited 101, `/tmp/luna-tools-mibhd7f4/result.json`. The diagnostic was removed;
  the test has no diff. Repeated type metadata copies hit the fixed compiler
  budget in the unchanged ListLength lowering. No baseline checkout run was
  performed. This remains a follow-up, not a reason to raise the test budget or
  omit representation checks.
- `LUNA_BENCH_ITERATIONS=5 python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --bench list_append`:
  exit 0, `/tmp/luna-tools-5wgaha8g/result.json`. Existing benchmark, five warmups,
  five measured invocations, development profile, same-build snapshot control.
  For 1024 owned Bool appends: fused 7,254,472 bytes/invocation versus snapshot
  185,877,448; Type appends: 4,533,744 versus 92,807,200. This is allocation
  evidence, not a historical/release speedup or linear-time construction claim.
- Managed `cargo fmt --all -- --check`: exit 0,
  `/tmp/luna-tools-ogrftw_k/result.json`; `git diff --check` passed.

Remaining priorities include repeated-metadata compilation costs, normal-form
plan bytecode, and eliminating redundant admission walks through sound reusable
validation evidence rather than bypassing checks.

## Charge bytecode compilation to the invocation budget

The large-element ListLength failure above exposed an unrelated fixed compiler
ceiling: a caller with ten million remaining steps still received StepsExhausted
at 100,000 compilation units. Meanwhile every successful or unsupported
compilation probe was free to the caller. Compilation now uses the remaining
invocation budget and deducts the compiler's consumed work on all outcomes.
Resource failure does not select another carrier or publish code. Stable-code
cache hits avoid repeated compilation; the independent cache storage bound is
unchanged. `bytecode_compilation_steps()` exposes cumulative charged work for
profiling (saturating at u64::MAX).

The original 4096-wide, 64-read length test retains its budget and bounds and now
also requires actual bytecode execution. Three observation-cost tests retain
their original read-cost thresholds but subtract the separately measured compile
work; compilation itself has budget-sweep, failure/recovery and cache-hit tests.
These functions have comptime inputs and currently specialize/recompile per call,
so warming them does not establish a reusable compiled body. An initial warmed
measurement assumption was rejected by the compilation counters and removed.
The next optimization must address redundant specialization and metadata copies,
not pretend those costs disappeared.

The source infinite-loop test retains the 80-step rejection (now before execution)
and additionally verifies execution exhaustion with the default budget. A separate
stale source assertion expected bool matching to use the tree carrier; structural
matching was already compiled, so it now requires bridge, pattern and digits to
execute exactly once each.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --tests`:
  all 22 groups passed (455 tests including 250 library tests), exit 0,
  `/tmp/luna-tools-9xqg0x4b/result.json`. Includes the formerly failing large-element
  test, index/length costs, instance budgets, representation admission and recovery.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test comptime_bytecode_lists --test comptime_list_operations --test execution`:
  6, 3, 54 and 1365 passed, exit 0, `/tmp/luna-tools-h3qxjibz/result.json`.
- Initial targeted budget runs exposed compile cost in the observation metrics;
  `/tmp/luna-tools-gve8hysx/result.json` and `/tmp/luna-tools-k45oomt9/result.json`
  exited 101. A missing telemetry-field initialization in the IIR constructor
  was fixed after `/tmp/luna-tools-e7usqlwq/result.json`. The AddressOf carrier
  expectation was corrected after `/tmp/luna-tools-7isbjnoo/result.json`.
- The first source run exposed the two test boundaries described above, exit 101,
  `/tmp/luna-tools-mm5r1l_d/result.json`; the successful source rerun is listed above.
- Managed `cargo fmt --all -- --check`: `/tmp/luna-tools-_w8knakq/result.json`;
  `git diff --check` passed. No test budget or observation threshold was increased.

This fixes caller-controlled resource accounting; it does not reduce metadata
copy work. Cold calls can now exhaust their supplied budget earlier because
compilation is no longer uncharged. Specialization/normal-form bytecode work
remains outstanding; this is not a completed VM claim.

## Reuse input-independent prepared bodies

A comptime input no longer automatically forces a fresh body clone, type rewrite
and compilation on every call. A bounded structural scan identifies package
bodies with no Bound or Hole terms. It enters every embedded type payload,
patterns, normal-form inputs, captures and shared closure bodies without unfolding
normal-form heads. Nested binders conservatively count as dependent; this is not
a complete free-variable minimization pass.

The first preparation still performs the original type rewrite and scope checks,
including pattern-domain completion. Only its successful immutable result can
enter the per-interpreter prepared-body cache. Dependencies are recorded as a
classification only; input-dependent bodies still specialize separately. Bodies
and classification entries share a 100,000-work storage bound, independent of
the existing code-cache bound. Eviction cannot invalidate active Arc-owned bodies.
Closures have no declaration cache key. Parameter admission, signature/result
binding, requirements, pre/postconditions and actual body effects still run for
every invocation. This is code reuse, not result memoization.

The bytecode cache compiles the prepared body supplied by the invocation boundary
rather than reopening the raw declaration. Prepared input-independent package
bodies are stable code; bodies containing active holes or input references are
not. Copy preflight now traverses type payloads before cloning input bodies,
replacing the old expression-only count. The shared walk also exposed a missing
intrinsic payload: pending scalar evidence is now inspected and charged, so a
hidden dependent intrinsic cannot become a shared specialization.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p nir --lib --test copy_budget`:
  8 library and 16 copy-budget tests passed, exit 0,
  `/tmp/luna-tools-kwo7miiy/result.json`. Includes hidden pattern, closure,
  normal-form and intrinsic types, traversal failure and non-mutating scans.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  all 457 tests passed (252 library plus integration groups), exit 0,
  `/tmp/luna-tools-7v6hl52x/result.json`. New cases distinguish shared bodies
  from dependent Type results, reject publication on failure, and verify eviction
  with a live body. Repeated observation tests retain their original thresholds,
  now measure warmed calls, and require zero recompilation plus one cached code
  decision. The first cold-cost run failed the observation threshold at
  `/tmp/luna-tools-fi0iibqm/result.json`; no threshold was increased.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test comptime_bytecode_lists --test comptime_list_operations --test function_type_dependencies --test execution`:
  6, 4, 54, 37 and 1365 passed, exit 0,
  `/tmp/luna-tools-cb1d9e63/result.json`. This preceded the pending-intrinsic
  traversal addition and explicit comptime annotation in the new source case.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_lists`:
  all 4 passed after those changes, exit 0,
  `/tmp/luna-tools-z8z4p5v4/result.json`. The new case asserts the parameter is
  ComptimeValue, supplies true/false/true lists, observes distinct results, and
  requires three executions with one compilation.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_math --test comptime_clause_inputs --test function_type_dependencies`:
  9, 43 and 37 passed on the final implementation, exit 0,
  `/tmp/luna-tools-i255o4n_/result.json`.
- `LUNA_BENCH_ITERATIONS=10 python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --bench type_list_index`:
  exit 0, `/tmp/luna-tools-c4ccr8z8/result.json`. Five warmups, ten measured
  invocations, development profile, unchanged benchmark. For 4096 Type elements
  and 64 literal-index reads: 959,840 allocated bytes and 408.10 allocations per
  invocation. The earlier indexing-stage log recorded 1,182,656 bytes and
  1522.10 allocations, but that is a historical revision comparison, not a
  controlled release timing claim. Input binding and admission costs remain.

Remaining work includes dependent-body specialization/code reuse, reducing
repeated input binding and type metadata copies, and normal-form plan bytecode.
The cache deliberately keeps no mutable environments, invocation values or
active inference state.

- Final managed `cargo fmt --all -- --check` passed, exit 0:
  `/tmp/luna-tools-r6ezrx36/result.json`; `git diff --check` passed.

## Export owned comptime inputs without an intermediate Value copy

Input binding formerly checkpoint-copied every staged argument before forming a
dependent Term. The consuming exporter itself traversed that copy by reference
and cloned its payloads into the resulting Term, so immutable owned arguments
paid for an avoidable intermediate Value representation.

Owned comptime inputs now use the existing borrowed checkpoint validation and
export directly into an independent Term. Type inputs likewise avoid copying the
Value wrapper and its entire type payload before the actual binding. This is a
general representation path, not a List/Type-list head rule. Storage and projected
reference inputs retain checked snapshots; runtime input handling is unchanged.

`normal_terms` has one borrowed conversion core. Consuming export still wraps it
with the original iterative release, including on failure for deeply nested host
data. Owned storage snapshots use that consuming path. Abstract declaration-domain
authority, shape checks, depth/size limits and spent-budget accounting are shared;
a borrowed value does not authorize a storage handle or unknown abstract input as
a normal term. No invocation result is cached.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  all 457 existing tests passed, exit 0, `/tmp/luna-tools-lzavjd5t/result.json`.
  This preceded the two new tests and the explicit consuming path for owned
  snapshots, which restores their original iterative-release behavior.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_terms::borrowed_input_tests`:
  two tests passed on the final implementation, exit 0,
  `/tmp/luna-tools-t2139asp/result.json`. Budget sweeps compare borrowed/consuming
  outcomes and work while preserving the input, reject unauthorized abstract
  inputs and storage export, and verify that subsequent owned/shared/projected
  writes cannot change already-bound substitution terms.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_clause_inputs --test function_type_dependencies --test comptime_list_operations --test builder_invocations --test execution`:
  clause inputs 43, function dependencies 37, list operations 54, builders 73
  and execution 1365 passed, exit 0, `/tmp/luna-tools-6lm2hkeb/result.json`.
- `LUNA_BENCH_ITERATIONS=10 python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --bench type_list_index`:
  exit 0, `/tmp/luna-tools-bbnsxm8h/result.json`. Unchanged development-profile
  benchmark, five warmups and ten measurements. The 4096-element, 64-literal-read
  case allocates 730,336 bytes/invocation versus 959,840 in the immediately prior
  recorded revision (`/tmp/luna-tools-c4ccr8z8/result.json`); both charge 21,644
  steps. This is a reduction in allocated bytes without removing validation
  charges, not a controlled release timing result. Benchmark input clones and
  remaining telescope/substitution copies are still included.
- An initial extraction incorrectly left consuming cleanup in the borrowed
  method; Rust rejected it before tests ran, `/tmp/luna-tools-ylufke1p/result.json`.
  Cleanup was restored to the consuming wrapper before the successful runs.
- Managed `cargo fmt --all -- --check` passed, exit 0,
  `/tmp/luna-tools-yyz6pi3r/result.json`; `git diff --check` passed.

Repeated telescope preparation, substitution-term copies, dependent-body code
reuse and normal-form plan bytecode remain separate work. This change removes
one intermediate immutable representation, not all input copying.

## Bind invocation inputs incrementally

`FunctionInputBuilder` now carries a completed substitution prefix through one
invocation. It prepares each parameter domain on demand, moves each staged input
into the substitution once and constructs the residual signature at completion.
The borrowed whole-input and type-view APIs share this implementation. Failed
pushes preserve the completed prefix; retained parameters, Type-domain refinement,
callable phase wrappers and variadic positions retain their existing contracts.
The builder has a deterministic S-expression Display.

The interpreter creates the builder only at the first materialized static input,
then supplies any preceding ordinary parameters as retained slots. Ordinary-only
calls do not allocate builder state. Representation admission, abstract-input
authority, storage snapshots and per-argument checks remain in place. No invocation
result caching or library-head specialization is introduced.

Validation from `/home/fin/luna` (all listed runs exited 0):

- `python3 -B development/check.py --timeout 900 -- test -p nir --lib --tests`:
  `/tmp/luna-tools-q5mnxnqq/result.json`. Includes refinement failure/recovery,
  opaque NF results, retained indices and phase/variadic wrapper checks.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-f9fmp31e/result.json`, after lazy builder initialization.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_clause_inputs --test function_type_dependencies --test dependent_list_collectors --test builder_invocations`:
  `/tmp/luna-tools-arq2jc8n/result.json`, after lazy initialization; all four
  source-level suites passed.
- Before lazy initialization, the same source suites plus
  `--test comptime_list_operations --test execution` passed:
  `/tmp/luna-tools-85648og7/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib staged_binding_after_ordinary_prefix_preserves_dependent_domains`:
  `/tmp/luna-tools-vwpr06p7/result.json`. Additional regression covers an ordinary
  prefix before a static Type argument, dependent suffix admission/rejection,
  owned and storage-backed inputs, and the final retained signature.
- `LUNA_BENCH_ITERATIONS=10 python3 -B development/check.py --timeout 900 -- test -p nir --bench input_binding`:
  `/tmp/luna-tools-94te7st_/result.json`. Same-build comparison of incremental
  binding against repeated whole-prefix binding, with three warmups and ten
  measurements. Both produce equal bindings. Each Type input carries a
  64-element structural type payload; this is an internal representation probe,
  not a new language-level Tuple identity rule.

| Inputs | Repeated-prefix allocated bytes/bind | Incremental allocated bytes/bind |
| --- | ---: | ---: |
| 1 | 7,561 | 3,921 |
| 16 | 588,744 | 58,912 |
| 64 | 8,457,096 | 234,928 |

This isolates allocation costs in telescope binding; it is not a historical
compiler or release-profile end-to-end timing comparison. Complex dependent
domains still require substitution and scope checking, including retained-prefix
work. Dependent-body code reuse and normal-form plan bytecode remain unfinished.

Final managed `cargo fmt --all -- --check` passed:
`/tmp/luna-tools-jmdo_0v2/result.json`; `git diff --check` passed.

## Execute sequential normal-form constructors as bytecode

Normal-form construction without an input-reordering plan now compiles when its
inputs are ordinary expression arguments (including the existing spread-value
wrapper). Begin/prepare/bind/finish instructions retain per-constructor state in
the invocation frame. Nested constructors use distinct registers; all operand
expressions use the ordinary bytecode compiler and invocation machinery. No opcode
calls the NIR evaluator and no definition body is unfolded.

Tree and bytecode paths share constructor preflight and input-domain checks.
Before operand effects, the interpreter validates the head, arity, result kind
and the next instantiated input domain. Each completed input is exported through
the existing normal-term admission path. Final construction uses the existing
representation validation and instantiated requirements. Early control transfer
exits without completing the constructor. Metadata and pending state have
S-expression dumps. Compilation work and owned type copies remain budgeted.

Validation from `/home/fin/luna`, all successful runs below exited 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-vfzc0cyn/result.json`, after shared domain checking and the
  initial differential identity/effect-order regression.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_type_tests`:
  `/tmp/luna-tools-zw4sp6p1/result.json`, final three tests. Tree/bytecode cases
  check dependent domains before effects, final requirements and recovery,
  unknown heads, bad arity/result kinds, and early return propagation.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test normal_form_inference --test normal_form_methods --test execution`:
  `/tmp/luna-tools-4vbn8axm/result.json`. Source-level wrappers and nested
  `Identity` applications preserve opaque heads for changing Type inputs;
  invocation counters prove that these constructors actually use bytecode.
  The type-bytecode, inference, method and execution suites all passed.
- Initial test scaffolding failed Rust compilation because a sibling module
  could not access the private bytecode test runner, then because Flow does not
  implement PartialEq (`/tmp/luna-tools-fmn_6gx9/result.json` and
  `/tmp/luna-tools-30fuqmk2/result.json`). Tests were placed beneath the bytecode
  module and destructure Flow before comparing values; production visibility and
  Flow semantics were not changed. An initial formatter run preceded creation
  of the test module and failed; subsequent formatting completed successfully.

Contextual literals, contextual containers and explicit preparation/reordering
plans still select the tree path before execution. This is the sequential
constructor foundation, not completion of normal-form plan bytecode. Their
staging, late domain resolution and temporary binding cleanup remain required.

Final managed `cargo fmt --all -- --check` passed, exit 0,
`/tmp/luna-tools-zjt6k2op/result.json`; `git diff --check` passed.

## Compile contextual scalar normal inputs

Sequential normal-form inputs now retain integer spelling/sign and unit markers
in bytecode metadata until preceding inputs establish their domain. The prepare
instruction calls the same contextual literal conversion used by tree execution;
a separate load moves the prepared value into its operand register. This supports
arbitrary-precision integers, signed/unsigned fixed-width integers and BitVec
conversion without treating any named type constructor as special. Unit retains
its existing distinction between a Void type value and a void value. Owned
spelling copies are charged during compilation; conversion remains budgeted.

Inspection also found that a malformed ordinary input annotated Never could
produce a value and reach bytecode NF completion. Binding now rejects that
fallthrough, matching the tree argument boundary; genuine return/other transfer
never reaches binding. A differential regression deliberately constructs such
malformed NIR rather than relaxing its annotation or accepting the value.

Validation from `/home/fin/luna`, all exit 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_type_tests`:
  `/tmp/luna-tools-v6gwqebh/result.json`, five tests. New cases cover dependent
  scalar domains, signed/unsigned limits, a large Integer, BitVec, incompatible
  domains, both unit interpretations and Never fallthrough rejection.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types`:
  `/tmp/luna-tools-xmb69zcm/result.json`. Source `Shape<input, 17>` agrees with
  tree evaluation, preserves the opaque head and increments bytecode execution
  counters for distinct type arguments.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-2vnt0uq2/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_form_inference --test normal_form_methods --test builder_invocations --test execution`:
  `/tmp/luna-tools-dobt0rah/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  `/tmp/luna-tools-tosgo9gt/result.json`; `git diff --check` passed.

Contextual containers/control and explicit preparation/reordering plans remain
on the preselected tree path. This stage extends scalar contextual inputs only;
it does not claim complete normal-form plan compilation or a timing speedup.

## Compile contextual normal-form containers

Owned input-shape metadata now describes scalar, list and tuple nodes with stable
registers and source locations. A bounded worklist builds the metadata and
operand tasks without recursively compiling expressions. Invocation-local
preparation checks the complete input against its instantiated domain before any
operand runs; the existing list-domain helper is shared with tree preparation.

Container append instructions follow each child immediately. Spread validation,
Type-list conversion and size checks therefore precede later operand effects.
Tuple type-description conversion remains after all children, matching tree
ordering. Completion reuses existing list validation and contextual tuple
construction, including the registered Tuple NF head. Preparation stores literals,
domains and accumulators only in the current frame, never the code cache. Metadata
and pending state retain S-expression dumps. Compilation accounts for shape width,
node work, copied types and spelling; excessive contextual depth selects the tree
path before execution, where the existing depth error applies.

Validation from `/home/fin/luna`, successful runs exit 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_type_tests`:
  `/tmp/luna-tools-6rbc5vcj/result.json`. Nested lists, heterogeneous tuples,
  registered tuple type heads, failed spreads before later effects and invalid
  contextual literals before any effects agree between tree and bytecode in
  comptime mode; existing scalar and failure/recovery cases remain covered.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-o47b_6lt/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler --lib normal_types::tests`:
  `/tmp/luna-tools-weka7w_4/result.json`. Additional compiler test checks fuel
  exhaustion, nested depth admission/rejection and escaped dump spelling.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types`:
  `/tmp/luna-tools-jmk9vo0u/result.json`, final source fixtures. Explicit expected
  NF terms and execution counters cover nested list payloads, named whole Tuple
  collection and a contextual list whose Integer/u8 element domain arrives only
  during invocation.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_clause_inputs --test dependent_list_collectors --test builder_invocations --test execution`:
  `/tmp/luna-tools-c7maikd_/result.json`.

Initial source-fixture failures were corrected against the language contract:
`named_arg -> .id = expr` requires a leading dot; ordinary anchored parameters
are positional, unlike whole variadic supplies (`docs/functions/parameters.md`,
explicit named supply). Tuple keeps `.Es=...`. Results containing ordinary
meta.List values require comptime admission, so a runtime `Interpreter::new`
was not a valid tree oracle. The source test now asserts complete expected NF
payloads; differential container tests explicitly use comptime mode for both
engines. Failure evidence: `/tmp/luna-tools-d9_vj3rl/result.json`,
`/tmp/luna-tools-p5xc4un_/result.json`, `/tmp/luna-tools-be4u07g3/result.json`,
`/tmp/luna-tools-5vmw_k1m/result.json`. No production grammar, admission rule or
semantic expectation was weakened to make those fixtures pass.

Contextual if/match/block arguments and explicit preparation/reordering plans
still select tree execution before effects. Their staging and cleanup remain the
next NF compilation work; this does not claim complete plan support or a measured
performance gain.

Final managed `cargo fmt --all -- --check` passed, exit 0,
`/tmp/luna-tools-27rpqgl6/result.json`; `git diff --check` passed.

## Compile contextual normal-form choices and blocks

Input shapes now include if and block nodes. Their domain preparation validates
both branches, condition types and empty-block result contracts before operand
execution, retaining the existing contextual typing order. The compiler emits
ordinary branch labels and scope tasks via the bounded worklist. Only the selected
branch executes. Result-transfer instructions move values between registers and
reject malformed Never fallthrough at contextual boundaries.

Blocks use the VM's existing scope/unwind machinery, including normal completion
and early return. Local bindings do not become part of reusable shape metadata.
Shape dumps include branch domains and children, and code dumps retain explicit
branch/result/scope instructions. No opcode evaluates an NIR tree.

Validation from `/home/fin/luna` (successful runs exit 0):

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-2ntjus0z/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types`:
  `/tmp/luna-tools-n9sjb1in/result.json`. Dynamic-domain conditional literals,
  nested blocks and early return execute as bytecode and produce explicit
  expected NF terms. An unselected overflowing u8 branch is still rejected.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_clause_inputs --test builder_invocations --test execution`:
  `/tmp/luna-tools-gl7ro80a/result.json`.
- After strengthening overflow checks to assert ErrorKind::Type, reran
  `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib contextual_choices_prepare`
  and `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types contextual_normal_control_flow`:
  `/tmp/luna-tools-wsgxjgy1/result.json` and
  `/tmp/luna-tools-_3h02qs_/result.json`. Differential cases verify condition
  effects, selected branch effects, rejection before effects, local cleanup,
  void block completion and early transfer.
- The initial integration check found a missing label-table argument at the
  compile_input caller (`/tmp/luna-tools-14mx2nms/result.json`); it was connected
  before successful compiler/interpreter and source checks.

Contextual match and explicit preparation/reordering plans remain unsupported
by bytecode and select tree execution before effects. No complete-plan support
or measured performance gain is claimed by this stage.

Final managed `cargo fmt --all -- --check` passed, exit 0,
`/tmp/luna-tools-cky4cgee/result.json`; `git diff --check` passed.

## Compile contextual normal-form matches

Contextual matches now use the same bounded structural-pattern preparation,
pattern instruction, guard code and scope machinery as ordinary VM matches.
Input-shape metadata retains arm domains and explicit guard types; preparation
checks every arm before the subject executes. The subject is evaluated once,
failed candidates leave their scope, and only the selected contextual body runs.
Result transfer preserves Never fallthrough rejection without completing an NF
after early control transfer. Match shape dumps retain arm/guard/body structure.

Validation from `/home/fin/luna`, successful runs exit 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-1stgwqyk/result.json`. New differential cases cover outer
  pattern guards and explicit arm guards, subject/guard effects, rejected-arm
  binding cleanup, skipped body effects, prechecking unselected overflowing
  literals, non-exhaustive matches, early guard return, invalid guard types and
  Never-annotated subjects incorrectly returning ordinary values.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types`:
  `/tmp/luna-tools-7abip1w3/result.json`. Invocation counters and explicit NF
  expectations verify dynamic-domain guarded matching, unselected overflow
  rejection and early arm return in source programs.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_matching --test comptime_clause_inputs --test builder_invocations --test execution`:
  `/tmp/luna-tools-h6ba15f1/result.json`.
- Initial integration checks found the new local-register map was not yet
  passed by the compile_input caller (`/tmp/luna-tools-pyejdh7z/result.json`,
  `/tmp/luna-tools-azm7of42/result.json`). The caller was connected and the now
  unreachable NormalFormArgument fallback removed before successful runs.

Explicit preparation/reordering plans still choose tree execution before effects.
The ordinary pattern compiler's existing exclusions (nested executable pattern
forms) also apply to contextual matches; they are not silently interpreted by a
bytecode opcode. This completes lowering for the current contextual argument
node kinds only when their contained expressions and patterns are compilable,
not full NF plan support.

Final managed `cargo fmt --all -- --check` passed, exit 0,
`/tmp/luna-tools-2mvtq7rf/result.json`; `git diff --check` passed.

## Shared normal-form binding state

Tree evaluation (including reordered and pending inputs) and sequential bytecode
now use the same invocation-local binder. It records declaration slots separately
from completion-order substitutions. Slots hold indices into owned terms, so the
tree path no longer clones each completed residual input to maintain both orders.
Finalization restores declaration order by moving terms; already ordered inputs
return their existing vector. Duplicate/out-of-range binding fails before state
mutation, and incomplete inputs cannot produce a completed normal form. State
has an S-expression Display including pending slots.

This is the shared-state foundation for VM input plans, not bytecode support for
preparations or deferred-input continuations. Those plans still select the tree
path before executing any effects. Opaque head identity and pending literal
staging remain unchanged.

Validation from `/home/fin/luna`, successful runs exit 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-5h3w7n09/result.json`. Includes out-of-order binding,
  duplicate and missing slots, deferred overflow followed by successful reuse,
  declaration-order output and plan-local cleanup.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test comptime_clause_inputs --test builder_invocations`:
  `/tmp/luna-tools-sd0uq48l/result.json`.
- After strengthening the deferred-input test to check the complete NF argument
  vector, `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib planned_contextual_inputs_recover_after_deferred_literal_failure`:
  `/tmp/luna-tools-ym_25d7s/result.json`.
- Managed `cargo fmt --all`: `/tmp/luna-tools-vktlo6x5/result.json`;
  `git diff --check` passed.
- Initial test compilation failed because the test module needed the qualified
  `normal_types::Bindings` path (`/tmp/luna-tools-9p5czfpp/result.json`);
  corrected before the successful runs above.

## Deferred contextual tuples retain abstract Type authority

While preparing VM input-plan work, a differential regression exposed a mismatch:
sequential tree/bytecode construction accepted an authorized `TypeParameter`,
but the staged tuple finisher accepted only `Value::Type`. Consequently a tuple
input arriving before its contextual Type rejected a valid abstract Type that
was accepted in declaration order. This contradicted the caller-scope preservation
in `docs/type/comptime-gradual-dependent-type.md` and the order-independent
normal-form argument binding contract.

All three paths now share `finish_contextual_tuple`. It observes Type operands
through `type_description_term`, including its existing authority check, and
uses the registered opaque tuple constructor with its formation requirements.
Value tuples retain their ordinary behavior; this does not introduce a new
Tuple type identity or source syntax. All operands are still evaluated before
completion, and completion never evaluates an expression again.

Validation from `/home/fin/luna`:

- The new regression initially failed only for the deferred input order,
  `/tmp/luna-tools-ke9d1yg7/result.json`, exit 101. Sequential tree, bytecode and
  declaration-order plan cases passed before the failing case.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-ghy7fy0i/result.json`, exit 0. The regression checks exact
  opaque head and argument identity in all four paths, repeated rejection of
  foreign Type parameters, recovery and temporary-binding cleanup.
- Managed `cargo fmt --all`: `/tmp/luna-tools-0bsfi6kj/result.json`, exit 0;
  `git diff --check` passed.

VM lowering of preparation/reordering plans remains unfinished. The corrected
finisher is shared groundwork, not a claim that those plans now use bytecode.

Source regression check also passed (exit 0):
`python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test comptime_clause_inputs --test builder_invocations`,
`/tmp/luna-tools-8lni84lz/result.json`.

## Bytecode evaluation of deferred contextual operands

`stage_normal_argument` now compiles and executes supported comptime operands as
linear staged fragments before their contextual domain arrives. This is connected
to existing NF and nominal input-plan evaluation, not a test-only entrypoint.
The compiler reuses contextual control lowering and ordinary expression opcodes;
staged instructions retain literal placeholders, list spread flags, tuple children
and selected branch results separately from language Values. No opcode evaluates
an NIR tree or re-executes a staged operand during final assembly.

Compiled metadata uses register keys. After execution a bounded adapter translates
those keys to the boxed invocation-owned NIR identities expected by the migration
finisher. No NIR address or invocation value enters reusable code. State and code
have S-expression dumps; ordinary frames allocate no staged state until needed.
The existing abstract Type authority and opaque normal-form head checks remain in
shared completion helpers.

Fragment calls enter the existing invocation driver. Compilable default expressions
use an explicit stack of VM frames; unsupported defaults retain their normal
migration boundary. Unsupported operand compilation selects the tree path before
that operand executes, while compile failures and execution failures propagate
without replay. Compilation, key translation and execution consume the caller's
budget. Logical contextual depth and imported environment mutations are restored
on completion, transfer and error.

Validation from `/home/fin/luna`, final successful runs exit 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-3rz7lvvn/result.json`. Covers owned staged code and exact
  compilation-work bounds, small-stack/depth limits, abstract Type inputs,
  selected branches and guarded matches, deferred literal rejection, list
  spreads, effect timing, scope cleanup, recovery and unsupported-code selection.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test comptime_clause_inputs --test builder_invocations --test execution`:
  `/tmp/luna-tools-1olk58z9/result.json`. New source cases assert staged-fragment
  and default-frame counters, exact NF/list identity across u8 and Integer,
  one-time mutation before a late Type input observes it, and an early return
  skipping a nonterminating later Type expression.
- Managed `cargo fmt --all`: `/tmp/luna-tools-7n8ioai3/result.json`;
  `git diff --check` passed.

Intermediate checks found two incomplete initialization sites while integrating
state (`/tmp/luna-tools-8idahs2m/result.json`, missing IIR telemetry initializer;
`/tmp/luna-tools-tzjezrri/result.json`, optional staged-state access), both fixed.
The original fixed-budget scope test failed at 39 steps because compilation now
exhausted its budget before executing the assignment
(`/tmp/luna-tools-c8dz64w7/result.json`). It now derives its range from measured
compilation work, requires exhaustion both before and after the effect, and keeps
exact final mutation, flow and cleanup assertions. This reflects budgeted
compilation rather than weakening operand behavior.

Remaining work: outer preparation/reordering scheduling and domain specialization
still use the migration evaluator. Staged fragments are compiled per invocation;
no end-to-end speedup or complete input-plan bytecode support is claimed. The next
integration should move outer plan scheduling into VM continuations while retaining
this distinct staged-result channel and the shared normal-form binding state.

## Bytecode fragments for inputs with known contextual domains

A separate `compile_contextual_input` entrypoint now emits whole-shape preflight
followed by ordinary contextual input code. The value exits through the usual
fragment result channel. NF and nominal input plans use this path for ready inputs,
complementing staged fragments for inputs whose domains are still pending.
Unsupported code is selected before that operand executes; execution errors never
replay the operand through the tree path.

Invocation-local construction state now separates optional NF head/binding state
from prepared contextual values. A standalone fragment has no synthetic head,
parameter declaration or normal-form identity. Nested real NF constructors retain
independent state and opaque identity. The compiler owns the contextual domain
and shape, charges their copies, and dumps the preflight in S-expression form.

Validation from `/home/fin/luna`, successful runs exit 0:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-nkpyxsh8/result.json`. New cases verify preflight precedes
  operand effects even for an invalid unselected branch, valid literal formation,
  nested opaque NF identity and owned compiler metadata with bounded work.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test comptime_clause_inputs --test builder_invocations --test execution`:
  `/tmp/luna-tools-13o6z1xw/result.json`. Source cases compare ready and deferred
  argument orders across u8 and Integer, assert actual fragment/default counters,
  and check early return from a ready input as well as a staged input.
- Managed `cargo fmt --all`: `/tmp/luna-tools-zr0zsz_9/result.json`;
  `git diff --check` passed.
- The initial compiler dump assertion expected bare `u8` rather than NIR's
  `(primitive u8)` rendering (`/tmp/luna-tools-q7bxx10w/result.json`); corrected
  the assertion to use the established Term display, without changing semantics.

Outer input-plan scheduling, binding export from preparation expressions and
specialization still use the migration path. Ready and staged fragments currently
compile per invocation; this is execution coverage, not a measured speedup or a
claim that complete input plans are now linear VM programs.

## Input preparation fragments export only committed packet bindings

NF, nominal and alias plans now send supported packet-preparation expressions
through the VM fragment driver. `NormalFormPreparation::local_bindings` supplies
the same outer tuple-pattern contract to plan validation and export selection.
The VM resolves export registers before execution, rejects binding collisions,
and moves committed values back only after successful completion. Producer-local
variables are not exported. Errors and transfers retain caller mutations without
leaking packet bindings; the enclosing plan still removes its bindings on exit.
Empty and trailing-empty packets execute even though they export no values.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p nir --test free_bindings`:
  `/tmp/luna-tools-p7nv6jyu/result.json`, exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-e7i6srxd/result.json`, exit 0. New differential tests cover
  successful export, temporary scope cleanup, return, missing input, failed
  pattern match, collision rejection/recovery, budgets exhausted before and
  after effects, enclosing NF cleanup and trailing-empty packet execution.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_spreads --test comptime_clause_inputs --test comptime_bytecode_types`:
  `/tmp/luna-tools-ysuoovsz/result.json`, exit 0. Preparation counters verify
  mixed packets, late Type inputs, repeated loop activations and empty tails.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters`:
  final `/tmp/luna-tools-jg3pttgc/result.json`, exit 101: 254 passed, 11 failed.
  The added dynamic alias preparation counter passed. All 11 failures report
  `unknown trait method` in associated-value/capability elaboration.
- The same nominal suite on baseline `2b908d71` produced the identical 11 failing
  test names and 11 `unknown trait method` diagnostics:
  `/tmp/luna-tools-5h4ne1fj/result.json`, exit 101. For this comparison only the
  six task-owned tracked source/test files were temporarily replaced with their
  HEAD contents; all were restored before the final check. Unrelated worktree
  modifications were preserved throughout. This suite is not reported as green.
- Final managed `cargo fmt --all`: `/tmp/luna-tools-knw8qfx6/result.json`, exit 0;
  `git diff --check` passed.

Initial test development found that a zero-step budget exhausted interpreter
initialization rather than the tested fragment; the sweep now sets its budget
immediately before evaluation (`/tmp/luna-tools-hpk4pplo/result.json` failure,
subsequent checks above pass). Some initially chosen pure-Type/empty packet
examples were already lowered to ordinary blocks and therefore had no preparation
nodes. Their counter assertions were replaced with cases confirmed to use plans;
NIR inspection established the distinction, without altering existing semantic
assertions (`/tmp/luna-tools-g7a976dc/result.json`,
`/tmp/luna-tools-47cow62z/result.json`, `/tmp/luna-tools-qclh2ecw/result.json`).

Outer input-plan orchestration and specialization remain in the migration
evaluator, and fragment compilation is still per invocation. The next VM step is
to suspend/resume the plan itself using the existing ready, staged and preparation
fragment channels. Separately, the confirmed baseline trait-method failures remain
unfinished frontend work; this stage does not repair or mask them.

## Explicit normal-form input-plan continuations

Normal-form plans now keep preparation position, source input order, pending boxed
inputs, completed substitutions and outstanding fragment requests in a resumable
frame. The driver evaluates preparation, contextual-ready and staged requests and
supplies a matching completion before advancing. Pending literal storage remains
stable when the frame moves. Semantic failures terminate the frame; protocol
misuse cannot advance an outstanding request or resume a completed frame. The
frame has a stable S-expression dump. Binding snapshots use the bounded value
representation copier before committing either the substitution or local export.

This is the scheduling-state migration, not complete plan bytecode support. The
plan driver still runs inside the migration evaluator; connecting these frames
to the invocation driver's stack, migrating nominal/alias scheduling and compiling
reusable plan templates remain unfinished. No speedup is claimed.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-f258yejx/result.json`, exit 0. Includes resume protocol,
  movement of a suspended frame with pending literals, budget-failure atomicity,
  terminal failure behavior, existing preparation exports and cleanup tests.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test type_spreads --test comptime_clause_inputs --test comptime_bytecode_types --test nominal_value_parameters`:
  `/tmp/luna-tools-1zm1usyi/result.json`, exit 0 after the terminal-state
  guard addition. Covers argument order, deferred
  domains, transfers, trailing empty packets and repeated plan activation.
- Managed `cargo fmt --all`: `/tmp/luna-tools-mp2nw1qm/result.json`, exit 0;
  `git diff --check` passed.

The 11 associated-call failures recorded in the preceding stage were separately
fixed by `17d321f4`; the nominal-value suite now passes all 265 cases. This does
not imply that all frontend or VM work is complete.

## One driver for compiled fragments, defaults and invocations

The fragment evaluator no longer recursively enters a second invocation driver
when a compiled operand calls a function. `run_executions` now drives both ordinary
invocations and borrowed fragment roots using the same activation/default stack.
Completion restores every child environment and invocation context before delivery;
fragment roots retain their frame so staged results and preparation exports remain
available to their callers. Unsupported bodies still select the migration route
before their execution. No new opcode interprets NIR recursively.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-nct9e88q/result.json`, exit 0. Added a fragment-root regression
  with nine compiled closure activations, alternating failed result delivery and
  success on the same interpreter. It checks root mutations, caller effect state,
  logical depth, pattern state and activation cleanup after both outcomes.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test comptime_bytecode_defaults --test comptime_bytecode_types --test comptime_bytecode_references --test type_spreads --test comptime_clause_inputs`:
  `/tmp/luna-tools-5sm31wv0/result.json`, exit 0. Covers nested defaults, failed
  default entry, early returns, references and contextual input fragments.
- Managed `cargo fmt --all`: `/tmp/luna-tools-y0qpbufe/result.json`, exit 0;
  `git diff --check` passed.

The outer normal-plan scheduler is still driven from the migration evaluator;
nominal/alias scheduling and a stack-integrated plan continuation are unfinished.
This change removes a duplicate driver and a native call boundary between compiled
fragments and compiled invocations, not every native recursion boundary. Contract
and unsupported-body evaluation still use their existing migration paths.

## Normal-plan requests run on the shared activation stack

A normal-form plan can now serve as a root of the invocation driver. Its compiled
preparation, ready and staged inputs become stack entries with explicit completion
state, so the plan no longer calls a nested fragment driver for each operand.
Function calls and defaults inside those inputs use the same stack. Input completion
centralizes preparation export, staged-result extraction, logical-depth restoration
and caller-environment recovery. Nominal/alias callers reuse the same start/finish
entrypoints without maintaining a second version of those rules. Input frames and
completion state expose S-expression dumps.

The entry into a normal-form plan is still the migration evaluator. Compiling a
function that contains a plan still selects that carrier before body execution;
a bytecode plan-suspension instruction and nested plan activation remain unfinished.
Nominal/alias outer scheduling is also still on its existing migration path. No
whole-VM completion or performance improvement is claimed by this stage.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_defaults --test comptime_bytecode_types --test type_spreads --test comptime_clause_inputs --test nominal_value_parameters`:
  `/tmp/luna-tools-wdegx_il/result.json`, exit 0. Covers default calls and transfers,
  contextual domains, source-ordered/reordered inputs and nominal/alias consumers.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-qo5bpl3u/result.json`, exit 0. Extended packet cleanup coverage
  to value-before-Type staging, and checked that compiled-input budget failure
  restores caller state and leaves the suspended plan terminal.
- Managed `cargo fmt --all`: `/tmp/luna-tools-t0y2a4yc/result.json`, exit 0;
  `git diff --check` passed.

## Bytecode suspension for nested normal-form plans

Compiled bodies now emit a `NormalPlan` suspension instruction for planned NF
construction. It owns an immutable, bounded-copy template and registers the
plan's free value bindings. Execution moves live caller locals into the plan
activation, preserving other registers and lexical scopes in the suspended caller.
The unified stack interleaves owned plan activations, input fragments, defaults
and function calls. Completion removes only plan-local bindings and restores the
borrowed caller locals before returning a value, transfer or error. An opcode never
recursively evaluates its plan.

This removes the body-wide migration fallback solely caused by a normal-form input
plan. The existing migration entry remains available for mixed/tree interpretation.
Nominal/alias outer plans, unsupported operand bodies and contract evaluation still
retain migration paths. Per-input specialization and fragment compilation remain;
no end-to-end speedup is claimed.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-5z1shhpv/result.json`, exit 0. New compiler coverage checks
  owned templates, free-versus-plan-local bindings, deterministic dumps, copy
  budget exhaustion and malformed-plan fallback. Packet tests compare tree and
  bytecode execution for ready/deferred order, failure cleanup and empty tails.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test comptime_bytecode_defaults --test comptime_bytecode_types --test type_spreads --test comptime_clause_inputs --test nominal_value_parameters`:
  `/tmp/luna-tools-z1xt515e/result.json`, exit 0 for the implementation before the
  final malformed-plan diagnostic mapping and additional regression assertions.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types`:
  `/tmp/luna-tools-taa7m0hv/result.json`, exit 0 on the final implementation.
  A nested reordered-plan example checks opaque identity, caller mutation,
  repeated invocation and actual compiled-root/staged-fragment counters. Existing
  early-return cases now also require the enclosing function to execute bytecode.
- Managed `cargo fmt --all`: `/tmp/luna-tools-w4s66g1v/result.json`, exit 0;
  `git diff --check` passed.

## Resumable nominal and alias input collection

Nominal/newtype and typealias inputs now use an explicit collection frame and the
same fragment/activation driver as NF plans. The scheduler retains source order,
preparation position, stable pending input storage, completed declaration slots
and terminal state. Nominal capture domains keep fixed-point wakeup: own inputs
may precede the captured Type inputs needed to finish them. Typealias value-domain
checks remain distinct. The driver returns a separate input-completion signal;
it does not manufacture a language value before declaration admission.

The original newtype capture-first admission, type requirements, capability checks
and transparent alias expansion still run after collection. These final steps and
the nominal/alias bytecode suspension entry are not migrated yet. NF plan semantics
and opaque normal-form heads are unchanged.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-yypnk2wp/result.json`, exit 0. Added a suspended-input test
  with a trailing captured Type, frame movement, deferred integer formation,
  input completion distinct from expression results, and terminal-state rejection.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test type_spreads --test comptime_bytecode_types`:
  `/tmp/luna-tools-wr1wedpe/result.json`, exit 0. Includes nominal/alias early
  returns, dependent domains, alias packet effects and captured newtype admission.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test newtypes`:
  `/tmp/luna-tools-j4gnj61f/result.json`, exit 0, all 211 cases.
- The first combined source run (`/tmp/luna-tools-57vcantx/result.json`) passed
  `comptime_clause_inputs` but stopped in `newtypes`: its standalone fixture
  writer omitted the ordinary bool binding, causing HIR errors before semantic
  execution. It now uses the shared primitive-binding fixture helper; existing
  expectations were retained. No production primitive lookup was changed.
- Managed formatting check: `/tmp/luna-tools-13jk9h2s/result.json`, exit 0;
  `git diff --check` passed.

## Nominal and alias plans on the activation stack

`TypeInputPlan` now suspends compiled callers for NF, nominal/newtype and alias
plans. Nominal input collection, declaration admission and alias-body execution
have explicit boundaries on the shared driver. Admission retains capture-first
newtype validation, type requirements and capabilities; alias specialization and
body execution start only after admission succeeds. A compiled alias body can
suspend for nested plans and calls without recursively starting another VM driver.
Plan completion restores caller bindings and removes plan-local bindings on both
success and failure. Tree entrypoints borrow their original expressions; compiled
plans retain immutable templates. Frame dumps use S-expressions.

Template copying now charges the work performed by `Expr::clone`, which shares
closure bodies. Alias specialization separately charges type rewriting, including
those shared bodies, after admission. The existing low-budget rejection test
caught premature charging of specialization work; its expectations were retained.
Unsupported bodies and contract evaluation still have migration paths. Input
fragments and specialized alias bodies are still compiled per use; this change
makes execution resumable, but does not establish an end-to-end speedup.

A new source case combines an opaque NF head, transparent alias expansion,
value-indexed nominal construction, reordered inputs, one caller mutation and
an early return before a divergent later input. It compares tree/bytecode results,
repeats invocation and requires the enclosing function to execute as bytecode.

An independent unresolved issue was found while constructing that case:
`value: T` remains a `ComptimeValue` binder when T becomes Type, while its supplied
Type value has Type sort. Nominal admission rejects it before alias expansion.
Both tree and bytecode paths exhibit this mismatch. The retained reproducer is
`development/repros/nominal-type-valued-index.fl`; it requires a general dependent
binder/residual-value solution, not an exception for any particular library type.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-9xzd83ly/result.json`, exit 0. Includes alias admission before
  body specialization under a tight budget, rejection recovery, input cleanup,
  dependent nominal captures and malformed input order.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test type_spreads --test comptime_clause_inputs --test newtypes --test comptime_bytecode_types`:
  `/tmp/luna-tools-jvzn6q3i/result.json`, exit 0, including the new nested-plan
  and early-return source regression.
- Managed `cargo fmt --all`: `/tmp/luna-tools-uh_pai9b/result.json`, exit 0;
  `git diff --check` passed.

## Domain-admitted Type-valued nominal inputs

Nominal declaration admission now resolves parameter domains using completed
inputs before choosing their instance sorts. This implements the rule documented
in `docs/type/comptime-gradual-dependent-type.md`: a completed Type domain takes a
Type input, even when the definition could only represent it as a dependent value
binder. Captured suffixes are resolved by dependency rather than source position;
cycles, mismatched domains and budget exhaustion remain distinct failures.

`AdmittedArguments` retains checked inputs and provides bounded substitution for
terms, type predicates and trait requirements. Definition occurrences must still
match their declared sorts. Ordinary fixed-sort substitution remains strict; no
blanket permission to replace a value binder by a Type was added. Opaque heads
remain opaque. This evidence now guards nominal/alias VM contract substitution,
alias-body specialization and later inputs whose domains use already completed
Type-valued inputs. Unchanged sorts retain the existing substitution path.

The saved Type-valued nominal repro now succeeds. Source coverage checks both VM
paths, a trailing bool input, rejection/recovery through an injected NIR alias
contract with the real PTC solver, repeated invocation and actual bytecode entry.
The injected contract isolates VM admission from elaboration's static discharge;
it is not evidence for all source constraint elaboration paths.

A prior NIR test incorrectly rejected `[bool, Type]` for a value followed by its
captured Type domain. It is now an explicit positive case based on the documented
completed-domain rule. Wrong-domain, unauthorized symbolic-value, malformed-sort
and budget cases remain negative. New NIR coverage also verifies opaque identity,
checked predicate substitution and the unchanged strict substitution API.

This stage covers type formation and VM contracts. General layout substitution,
function/NF dependent binder completion and open caller scopes have other consumers
of the fixed-sort APIs and are not claimed migrated here. The sort-changing input
path currently reconstructs admission evidence while specializing later inputs;
its performance still needs measurement and incremental evidence reuse.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p nir -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-49o94x_c/result.json`, exit 0, including completed/captured
  domains, alias budget ordering, nested aggregate admission and failure recovery.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test nominal_value_parameters --test type_spreads --test newtypes --test comptime_clause_inputs`:
  `/tmp/luna-tools-9q0k68s1/result.json`, exit 0, including the Type-valued
  nominal/alias regression with a subsequent bool parameter.
- Managed formatting: `/tmp/luna-tools-91ubdqpg/result.json`, exit 0;
  `git diff --check` passed.

## Domain-aware nominal layout substitution

`DeclarationSubstitution` checks the argument scope and completed parameter sorts
for structural queries. A dependent domain equal to Type requires a Type input;
a domain that stays an opaque head is not unfolded to justify a sort change.
Its caller frames are explicit, and substitution shifts references beneath nested
function binders. The checked context has an S-expression dump. This is structural
substitution, not full value admission: scalar ranges, aggregate payload evidence,
formation requirements and capabilities retain their own owning boundaries.

Struct fields, enum payloads, newtype representations and nominal frozen-value
layout queries now use this context. Budgeted queries charge domain completion and
replacement payloads before copying; representation checks preserve budget failure
and retry behavior. Elaborated field reads, multi-level type observation, direct
writes, pointer projections/writes and declaration-owned field defaults use the
same domain-aware substitution. Ordinary fixed-sort substitution remains strict.

The initial source regression formed `Key<Type, bool>` successfully but failed
on construction with `expected comptime-value, got type`. After layout migration,
a nested `Outer<Type, bool>.inner.payload` case exposed the remaining fixed-sort
field-observation path; that path was corrected without removing the regression.
The final source cases instantiate and execute IIR for nested struct construction,
field reads/writes, pointer writes, defaults, enum payload matching and newtype
conversion. Wrong-domain Type/value exchanges remain rejected. These are compiler
and IIR execution checks, not LLVM host execution benchmarks.

General function/NF binder completion and declaration contract consumers beyond
the previously migrated nominal/alias VM path are still separate migration work.
This stage does not claim every fixed-sort consumer or all dependent language
features have been migrated.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p nir -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-soq1v6nd/result.json`, exit 0. Includes new concrete/open-scope
  layout cases, nested binder capture avoidance and preserved budget errors.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test newtypes --test comptime_bytecode_types --test type_spreads --test comptime_clause_inputs`:
  `/tmp/luna-tools-qc98up3k/result.json`, exit 0 on the final implementation.
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test representations --test checked_definition_publisher --test checked_struct_publisher`:
  `/tmp/luna-tools-ho77fa12/result.json`, exit 0, including the added Type-valued
  struct/enum budget-failure recovery test and existing publication atomicity tests.
- Managed formatting: `/tmp/luna-tools-a76zxsuk/result.json`, exit 0;
  `git diff --check` passed.

## Completed domains in NF and ordinary callable instances

NF input preparation, stored-type admission and formation requirements now use
source domains when substituting completed arguments. An input declared as
`value: T` can receive a Type when T completes to Type, including when subsequent
inputs contain a nominal type indexed by that value. Opaque NF heads remain
opaque. This uses the same structural substitution evidence as nominal layouts;
it does not authorize arbitrary Type/value sort exchanges.

NF domain checks collect the transitive dependency closure of the current input,
not the entire earlier telescope. The first implementation revisited every prefix
and failed the existing 100-independent-bool-input budget test. That test remains
unchanged and now passes. Substitution, domain copying and predicate expansion
consume the shared invocation budget.

Ordinary callable admission, signature/body specialization, default arguments,
formation predicates, deferred capabilities and invocation conditions use declared
domains as well. Predicate substitution retains the ordinary input frame from the
function signature, so a requirement on an ordinary Type argument still waits for
that argument. Cached callable bodies do not suppress current contract checks;
rejected formation does not populate the instance cache. Frozen callable recovery
and explicit host invocation use the same domain-aware specialization.

Source regressions cover VM/tree results, repeated calls, a nominal default and
field read, wrong-domain rejection, and dynamic formation rejection with the PTC
solver installed. The real sysroot regression stores `Key<Type, bool>` through
`Object.append`, recovers it with `Object.value`, then constructs and compares an
opaque normal form. It specifies an explicit build target because the shipped
sysroot also contains target-dependent runtime definitions.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p nir -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-h85qvoh8/result.json`, exit 0 on the final implementation.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_clause_inputs --test comptime_bytecode_types --test default_calls --test nominal_value_parameters --test type_spreads --test newtypes`:
  `/tmp/luna-tools-e6nh4qy7/result.json`, exit 0, including formation rejection
  with an installed PTC solver and repeated failed formation without caching.
- Managed formatting: `/tmp/luna-tools-8559v_cw/result.json`, exit 0.

The broader sysroot run `/tmp/luna-tools-d9mzuda0/result.json` failed. Three old
fixture tests omit the ordinary bool binding; the tuple/optional/math/iteration
integration fails with `field projection requires a supported struct`. The new
Object test initially lacked target pointer-width information; its invocation
now supplies `--build-target x86_64 linux 64`. The other four failures remain
separate unresolved work; no clean-baseline run was made to attribute them to a
particular historical commit. This stage does not claim the full sysroot suite
passes, nor that all trait/IIR substitution consumers have been migrated.

- Final sysroot check:
  `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_meta_object_preserves_type_valued_indices_in_normal_inputs`:
  `/tmp/luna-tools-r76hdp6c/result.json`, exit 0. `git diff --check` passed.

## Sysroot fixture follow-up

The four outstanding sysroot failures are resolved. Minimal builtin fixtures now
bind bool as an ordinary builtin symbol and export it through their preludes;
the iterator fixture also exports its existing primitive bindings. The composite
native case selects `OptionalOps<bool>` explicitly, reflecting the author's
runtime/comptime trait split instead of the superseded inherent optional method.
The tuple, optional, LLVM math and inline iteration checks remain intact.

`python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes`
from `/home/fin/luna` completed with exit 0; all eight cases passed, including
native host execution and the real meta Object/NF integration. Evidence:
`/tmp/luna-tools-_1k2pcrb/result.json`. `git diff --check` passed. This corrects
stale fixtures, not compiler semantics or expected program behavior.

## Completed domains in runtime function materialization

Runtime functions now specialize dependent clause inputs by their completed
source domains. The new source regression first failed at IIR formation with
`expected comptime-value, got type`. It now executes ordinary, inline and staged
runtime calls taking `Key<Type, bool>`, with a retained formation requirement and
field access. The function template transformation keeps its ordinary input frame
for body annotations, defaults and contracts, and copies each output field once.

Declaration-world queries expose bounded domain copies. Concrete materialization
admits value inputs before specializing templates; Type-only declarations reuse
strict substitution and the already completed instance-key Type checks. This
avoids redundantly traversing large unused Type arguments. The original phantom
input/copy-expansion regression still fits its unchanged 3000-step budget while
the repeated-type body still exhausts it. No special library-name path was added.

Open runtime identities and finite template-storage tables use the same completed
domain rule. The table regression covers Type-valued dependent indices, malformed
inputs, budget failure, and successful construction after failure. Trait method
selection and other independent fixed-sort consumers remain separate migration
work; this stage does not claim all dependent declarations are implemented.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p nir -p iir -p materialization --lib --tests -- --skip nested_scalar_comparisons_and_blocks_reach_logical_depth_limits`:
  `/tmp/luna-tools-0ivzitms/result.json`, exit 0. One known failing deep-fold test
  was excluded only for this follow-up run, not deleted or ignored in source.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test instantiation instance_copy_budget_counts_expanded_parameter_occurrences_and_recovers -- --nocapture`:
  `/tmp/luna-tools-ver86xlu/result.json`, exit 0 with the original budget assertions.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test default_calls --test comptime_clause_inputs --test sysroot_preludes`:
  `/tmp/luna-tools-r79z0j2q/result.json`, exit 0 before the final Type-only fast-path
  optimization. Includes the real sysroot and runtime specialization regressions.
- Managed formatting: `/tmp/luna-tools-hp6l0i7s/result.json`, exit 0.

Expanded checks exposed additional failures, retained for follow-up:

- The unrestricted NIR/IIR/materialization run
  `/tmp/luna-tools-t2mzoq05/result.json` aborted in
  `staging::tests::nested_scalar_comparisons_and_blocks_reach_logical_depth_limits`
  with a host stack overflow. The staging folder was not changed in this stage.
- The unrestricted instantiation run `/tmp/luna-tools-wb5ht9aw/result.json`
  aborted in `concrete_graph_executes_without_source_templates` with a host stack
  overflow. It also exposed the redundant-input budget regression fixed above.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test instantiation --test staged_aggregate_contracts --test test_templates -- --skip concrete_graph_executes_without_source_templates`:
  `/tmp/luna-tools-2wz2egw5/result.json`, exit 101 overall. The remaining 56
  instantiation tests and all 31 staged aggregate tests passed; test_templates
  had 67 failures, with missing ordinary bool bindings in its old fixture path.
  No template tests were changed to suppress these failures. No clean-baseline
  run was made to attribute the stack failures to a historical commit.

Final source regression:
`python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters type_valued_indices_specialize_runtime_function_instances`:
`/tmp/luna-tools-_tblahne/result.json`, exit 0. `git diff --check` passed.

## Staged folding reaches its logical depth limit without stack overflow

The previously failing IIR scalar/block/discard depth regression now passes on
the unchanged test-thread stack. General folding used one large match containing
all branch-local Expr temporaries. In the unoptimized x86_64 test binary,
`objdump -Cd` showed a 0x2ff8-byte (12280-byte) general-fold frame, compounded by
recursive dispatch and local preparation. A depth limit alone could not prevent
host stack exhaustion before reaching that limit.

Node-specific general folding now lives in `staging/general.rs`, separated from
dispatch and final validation. The dispatcher forwards result slots instead of
retaining temporaries for unrelated branches. The measured block path uses a
0x6c8-byte validation frame, 0x3b8-byte dispatch frame and 0x10a8-byte block frame
(6952 bytes together). Other recursion frames remain unchanged; these are frame
sizes for this build, not a release performance benchmark.

No stack-size override, reduced language depth limit, budget change or test skip
was introduced. Symbol conversion, field/scalar operations, block local binding,
discarded evaluation, result-type validation and source propagation retain their
existing behavior. The runtime IIR execution stack overflow is still separate.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p iir --lib nested_scalar_comparisons_and_blocks_reach_logical_depth_limits -- --nocapture`:
  `/tmp/luna-tools-6vezg22u/result.json`, exit 0, including depths 16, 64, 127,
  128 and 160 and successful recovery after depth rejection.
- `python3 -B development/check.py --timeout 900 -- test -p iir --lib --tests`:
  `/tmp/luna-tools-b1o2jct3/result.json`, exit 0 without exclusions.
- Managed formatting: `/tmp/luna-tools-xmsl_0nc/result.json`, exit 0;
  `git diff --check` passed.

## Runtime tree execution preserves depth failure and recovery

The second recorded stack overflow had two causes. Runtime tree invocations
entered the linear bytecode driver even when its existing eligibility rules
could never select bytecode. The tree recursion retained that driver's large
frame at every call. The shared eligibility query now selects tree evaluation
after ordinary invocation admission and preparation, and rejoins the same
completion/restoration boundary. Cleanup schedules, effects, phase checks and
bytecode eligibility are unchanged.

This restored the original 20-level runtime regression. Extending it to require
normal rejection at the default 128-call depth budget exposed a second oversized
frame: metadata, enum/text observations and integer conversions had accumulated
inside the common expression dispatcher. Those evaluators now live in
`observations.rs`, retaining their original checks and behavior. In the same
unoptimized x86_64 binary, objdump reports the common eval_inner frame changing
from 0x4548 bytes (17736) to 0x48 bytes (72). These are build-specific stack-frame
measurements, not a throughput claim.

The source IIR test now attempts 200 recursive calls, checks DepthExhausted, then
successfully reuses the same interpreter for its original entry. It still checks
execution after discarding the source package and absence of runtime template
instantiation events. Neither host stack size nor default depth budget changed.
Tree execution still uses native recursion; this is not a claim that arbitrary
user-configured depth limits or every unsupported-bytecode path are stackless.

Validation from `/home/fin/luna`:

- The isolated original failure was reproduced in
  `/tmp/luna-tools-sv8c7iwj/result.json`; the stronger depth case initially
  overflowed in `/tmp/luna-tools-k_ua2dov/result.json` after the routing fix alone.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test instantiation concrete_graph_executes_without_source_templates -- --nocapture`:
  `/tmp/luna-tools-jb2w64n_/result.json`, exit 0 with the added depth/recovery check.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib --tests`:
  `/tmp/luna-tools-7dorjura/result.json`, exit 0 on the complete implementation.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test instantiation --test staged_aggregate_contracts --test sysroot_preludes`:
  `/tmp/luna-tools-e37e3zpr/result.json`, exit 0 without exclusions: all 57
  instantiation, 31 staged aggregate and 8 sysroot cases passed.
- Managed formatting: `/tmp/luna-tools-62uq4jx5/result.json`, exit 0;
  `git diff --check` passed. The earlier template fixture failures remain separate.

## Template fixture bindings restored

The test_templates compile entry now uses the existing standalone fixture writer
for both its main source and sibling modules. This appends an ordinary bool
binding when needed, retaining source offsets and existing explicit bindings.
Production primitive lookup and all test expectations are unchanged.

`python3 -B development/check.py --timeout 900 -- test -p luna --test test_templates`
from `/home/fin/luna`: `/tmp/luna-tools-fzhxygw6/result.json`, exit 0, all 81 cases
passed without exclusions. This resolves the 67 earlier fixture failures rather
than changing template semantics. `git diff --check` passed.

### Implementation argument admission follows completed domains

Implementation head admission now uses bounded `DeclarationSubstitution` for
parameter domains. A dependent input completed to `Type` can carry a type value
without making later parameter substitution fail the original fixed-sort check.
Value/layout admission and captured-domain evidence remain mandatory. Regression
coverage includes later parameters, wrong sorts, budget exhaustion and recovery.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`: exit 0,
  `/tmp/luna-tools-6fr6bhcg/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: exit 0,
  `/tmp/luna-tools-upa5yo_6/result.json`.

The source regressions for `Key<Type, bool>` inherent/trait method selection still
fail before admission: head inference creates fixed-sort metavariables and leaves
head obligations unsettled (`/tmp/luna-tools-54lpr6cz/result.json`). These pending
source tests remain outside this admission commit. Head inference and proof replay
still need completed-domain integration; this stage does not claim source support.

### Domain-aware implementation evidence replay

Implementation requirement specialization, head-match replay, and final type
evidence replay now use bounded declaration-domain substitution. Replay checks
completed domains rather than comparing actual arguments to source binder sorts.
The regression builds independent reflexive proofs for a nominal subject, trait
argument, and requirement containing a Type-valued dependent input; it also rejects
forged argument sorts, changed type identity, missing requirements, and exhausted
budgets, then verifies recovery. No value equations are introduced into PTC.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`: exit 0,
  `/tmp/luna-tools-dmjwa9ec/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test trait_phases --test trait_purity`:
  exit 0, `/tmp/luna-tools-n7rbjoj5/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: exit 0,
  `/tmp/luna-tools-6i4u3jgv/result.json`; `git diff --check`: exit 0.

Remaining: implementation head inference still allocates source-sort metavariables.
The fixed-sort `InferenceTerm::Instantiate` and matching decomposition cannot yet
represent a dependent input whose completed domain changes its sort to Type. The
two pending source regressions remain uncommitted; replay support alone does not
establish end-to-end implementation selection.

### Staged dependent-domain implementation inference

Head inference can now resume when provisional assignments complete the domain
of an unresolved input and change its argument sort. A bounded, candidate-local
inspection supplies the known inputs; declaration-domain substitution specializes
the remaining head. Each round removes known parameters and consumes the existing
inference budget, reserving a separate replay share. The original declaration's
head proofs and completed requirements are checked before returning the match.
Fixed-sort inference variables and ordinary substitutions remain strict.

Inherent method signature specialization now uses the same domain-aware input
substitution as ordinary calls. Both pending source regressions execute
`Key<Type,bool>` methods through IIR: inherent selection and explicit trait
selection. Solver coverage additionally checks reversed dependency order, repeated
input conflicts, invalid sorts/domains, budget exhaustion and subsequent recovery.
No constructor-name exceptions or program-value PTC equations were added.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`: exit 0,
  `/tmp/luna-tools-1ctngd0d/result.json`.
- Final affected suites after removing an unnecessary domain clone:
  `python3 -B development/check.py --timeout 900 -- test -p type_resolution -p luna --test implementation --test nominal_value_parameters --test trait_phases --test trait_purity`:
  exit 0, 78 implementation, 272 nominal input, 30 phase, and 21 purity tests;
  `/tmp/luna-tools-ez103db4/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0, `/tmp/luna-tools-b6fgyah8/result.json`; `git diff --check`: exit 0.

This closes the two source selection failures recorded above. Unresolved domains
remain pending; staging does not invert opaque normal-form heads or authorize
export of incomplete candidate state. Other fixed-sort consumers still need their
own contract audit; this is not a claim that all dependent-type paths are complete.

### Type-valued trait interface indices reach runtime dispatch

Trait argument admission now completes declared domains (including `Self`) before
checking argument sorts. Structural/value admission remains separate. Direct
interface inputs whose declared value sort awaits domain completion retain a
pending relation; staged head matching must establish and replay that relation.
Elaboration, NIR phase checks, dispatch specialization, and retained runtime
interface signatures now use declaration-domain substitution. Program contracts
have a bounded domain-aware substitution entrypoint retaining ordinary input
frames, predicate expressions and snapshot identities.

The source regression executes `Read<Type,bool>` on `Key<Type,bool>` and rejects
`Read<Type,char>`. The real-sysroot regression uses `Object.append` and typed
`Object.value`, reads the payload in a function-local comptime declaration, then
constructs the runtime value and dispatches the parameterized trait method. This
does not claim implicit splicing of arbitrary comptime nominal values: the initial
file-scope constant transfer was rejected for lacking a Splice protocol.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p nir -p type_resolution -p luna --test calls --test trait_parameters --test implementation --test nominal_value_parameters --test trait_phases --test trait_purity --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-9qxumqei/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p nir -p type_resolution -p materialization`:
  exit 0, `/tmp/luna-tools-ot5qpbtt/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: exit 0,
  `/tmp/luna-tools-yfz8dogh/result.json`; `git diff --check`: exit 0.

Remaining audits include capability/parent traversal, staged method contracts and
program-summary consumers still using fixed-sort substitutions. Passing this
ordinary runtime dispatch path does not establish those paths.

Final source recheck after adding the mismatched interface rejection and reusing
the elaboration substitution inputs:
`python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test sysroot_preludes --test trait_phases --test trait_purity`: exit 0,
`/tmp/luna-tools-o6is6dk4/result.json` (273 + 9 + 30 + 21 cases).

### Parent interfaces and capability propagation preserve dependent domains

Added a structural trait-input substitution entrypoint for `Self` followed by the
interface inputs, plus bounded interface/requirement mapping. Inherited member
lookup, capability lookup roots, subject-guarantee propagation, given-evidence
traversal, trait formation, impl qualification and materialized impl capabilities
now use completed declaration domains. Value admission and formation obligations
remain independent; parent traversal does not promote a parent to a lookup root.

A source regression executes both explicit `Child<Type,bool>` parent-method
lookup and a generic `K :- Child<Type,bool>` call through IIR. Solver regressions
cover parent-to-capability propagation, exact type identity, subject guarantees,
low-budget rejection/recovery, and impl qualification with/without its required
capability. The original source failure was fixed-sort member lookup
(`/tmp/luna-tools-qaaz6kop/result.json`).

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution -p nir -p materialization`:
  exit 0, `/tmp/luna-tools-pz9e6rja/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: exit 0,
  `/tmp/luna-tools-7wlfb4ys/result.json`; `git diff --check`: exit 0.

Remaining: staged method/program-summary consumers and other fixed-sort contract
paths require separate audits. This change establishes these traversal paths,
not arbitrary runtime/comptime dispatch completion.

Final source verification:
`python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test trait_phases --test trait_purity --test sysroot_preludes`:
exit 0, `/tmp/luna-tools-_jqbk15q/result.json`.

### Runtime trait program summaries retain dependent domains and scope

Added a bounded method-summary constructor using declaration input domains. It
instantiates the callable signature in caller scope, then shifts substituted
caller inputs beneath the method's ordinary input frame for program predicates,
type requirements and capabilities. Program contracts, local/result identities,
snapshots and retained method parameters survive summary construction. Flow
analysis now uses these summaries and distinguishes exhausted substitution fuel
from malformed signatures.

The source regression exercises `Read<Type,bool>` with a runtime precondition:
`read(true)` executes through IIR and `read(false)` is rejected for its
precondition. Previously the valid call failed with `invalid-signature`
(`/tmp/luna-tools-3tgeadzt/result.json`). The NIR regression checks that a caller
Type input and a method-local Type input remain distinct in the actual predicate
expression as well as capability arguments, with scope rejection and budget
recovery. No program-state proposition is routed through PTC.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p nir -p program_verification`:
  exit 0, `/tmp/luna-tools-y6vifml_/result.json`.
- After the flow budget diagnostic mapping:
  `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test trait_purity --test staged_aggregate_contracts --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-n9a8286g/result.json` (275 + 21 + 31 + 9 cases).
- Final predicate capture regression:
  `python3 -B development/check.py --timeout 900 -- test -p nir --test calls`:
  exit 0, `/tmp/luna-tools-6yt1tlrs/result.json`.

Remaining: staged method specialization and memory-method contract consumers
still need domain-aware substitution audits; this stage closes the ordinary
runtime flow-summary failure.

Final formatting: `python3 -B development/command.py --timeout 120 -- cargo fmt --all`,
exit 0, `/tmp/luna-tools-w9m33d4x/result.json`; `git diff --check`: exit 0.

### Staged entry contracts and memory inference use completed input domains

Staged trait materialization now specializes requirements, capabilities and
program predicates through declaration-domain evidence before the existing local
binding alignment. The source regression executes a Type-valued interface with
a comptime ordinary input and rejects its false precondition. The original
materialization failure is recorded in `/tmp/luna-tools-54ydaxg7/result.json`.

Composed memory method contracts now use the same domain evidence. Their named
body and callable/closure signatures, immutable value observations, operation
identities and text-region body specialization no longer impose source binder
sorts on completed Type-valued inputs. A memory query caches immutable declared
domains, not argument-derived sorts. Its regression alternates scalar and Type
inputs in the same query, rejects mismatched arguments, and checks failed scans
and subsequent queries cannot reuse incomplete or stale evidence.

The composed-method source regression removes the implementation's inherited
precondition and still observes the interface's pointer read and rejection edge,
without modifying the package. The text regression preserves the caller's backing
region through `echo<Type,bool>` and checks budget recovery. The direct composed
query originally failed (`/tmp/luna-tools-3trf0m3w/result.json`); merely executing a
pointer call had not exposed that missing analysis path.

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p program_verification -p materialization`:
  exit 0, `/tmp/luna-tools-03fgnugt/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test method_memory_contracts --test staged_aggregate_contracts --test captured_memory --test effect_memory_transfers --test text_regions --test instantiation --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-8ldq5d9t/result.json`, no exclusions.
- Final additional text-origin regression:
  `python3 -B development/check.py --timeout 900 -- test -p luna --test text_regions type_valued_dependent_inputs -- --nocapture`:
  exit 0, `/tmp/luna-tools-bhmrnraq/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`: exit 0,
  `/tmp/luna-tools-dsqud678/result.json`; `git diff --check`: exit 0.

These results close the staged and composed-memory failures above. They do not
claim every declaration-substitution consumer elsewhere in the compiler has
been migrated or that arbitrary dependent program verification is complete.

### Associated items specialize through declaration domains

Associated defaults and selected implementation values now use their declared
input domains, allowing a dependent value parameter to receive a Type value.
The associated query engine applies the same bounded substitution to member
types, requirements, capabilities, selected templates and preserved projections.
Selection evidence, member identity, scope checks and completed-value validation
remain required. Normal-form heads are not unfolded.

NIR expression rewriting now represents a completed Type-valued dependent input
as TypeValue, rather than leaving a ComptimeValue node around a Type term. This
also supports an ordinary `assoc data:T=index` when T becomes Type.

Source regressions execute concrete and generic defaults, inherited lookup, and
the same dependent associated declaration with both boolean and Type inputs.
They reject a mismatched nominal index. Independent NIR and solver regressions
cover representation, exact selected arguments, invalid domains, budget failure
and recovery. Initial source failures are preserved in
`/tmp/luna-tools-jec50f_f/result.json` (default substitution),
`/tmp/luna-tools-5fa2synr/result.json` (generic selected value), and
`/tmp/luna-tools-cjuv1uxe/result.json` (dependent Type-value representation).

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p nir -p type_resolution`:
  exit 0, `/tmp/luna-tools-piqix059/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test trait_phases --test trait_purity --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-shlfshrt/result.json`, no exclusions.
- `python3 -B development/check.py --timeout 900 -- test -p materialization -p program_verification`:
  exit 0, `/tmp/luna-tools-29lv_trn/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-75u0awz7/result.json`; `git diff --check`: exit 0.

Other declaration-substitution consumers still require audit. This stage does
not claim all associated initializer or dependent specification paths complete.

### Chained associated initializers and explicit method overrides

Checker input-domain substitution is now reusable across type terms and
interface queries. A selected associated initializer uses that context for Self
and interface inputs, so a dependent default can read another associated member
from the same selected implementation, including an explicit override.

Explicit method compatibility now specializes the public signature, clause
input domains, type requirements and capabilities using completed declaration
domains. Candidate associated equations use the interface domain context.
Program-contract inheritance separately specializes predicates in the actual
method's ordinary input frame; predicates still belong to program verification.
The source regressions execute chained defaults and overrides, reject a stale
associated result type, execute an explicit method with an associated signature,
reject a false inherited precondition, and reject a strengthened precondition.

Initial failures: `/tmp/luna-tools-vid6t9xu/result.json` (chained initializer),
`/tmp/luna-tools-nhng6axy/result.json` (override signature), and
`/tmp/luna-tools-lq8myl9g/result.json` (program-contract inheritance).

Validation in `/home/fin/luna`:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test trait_phases --test trait_purity --test method_memory_contracts --test staged_aggregate_contracts --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-gr6fhg0g/result.json`, no exclusions.
- `python3 -B development/check.py --timeout 900 -- test -p program_verification`:
  exit 0, `/tmp/luna-tools-2keg4ll3/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-svz2g8pj/result.json`; `git diff --check`: exit 0.
- Expanded `python3 -B development/check.py --timeout 900 -- test -p elaboration -p program_verification`:
  exit 101, `/tmp/luna-tools-9vnhr__t/result.json`. It stops at
  `failed_effect_inference_does_not_unfold_opaque_inputs_or_mutate_scheme`:
  inference succeeds where the test expects failure. This test directly uses
  the Scheme re-export from type_resolution. `git diff --exit-code HEAD --
  compiler/type_resolution compiler/nir compiler/elaboration/tests/effect_inference.rs
  compiler/elaboration/src/lib.rs` exits 0 against the pre-stage HEAD ed391bd2.
  The inference implementation, its dependencies and this test were not changed
  in this stage; no separate baseline build was run. Its semantic expectation
  and implementation still need investigation; the expanded suite is not green.

Remaining declaration-only and contextual-code substitution consumers are not
claimed complete by this stage.

### Retained effect heads: inference proposals versus inverse equations

The effect-inference failure from the previous stage was an obsolete rejection
expectation, not a missing requirement to forbid all matching below a normal-form
head. The authority is docs/type/type-identity.md (retained head identity and the
separate matching relation), docs/type/type-patterns.md (observing retained head
arguments), and docs/compilation/elaboration.md (call-local inference variables
versus rigid definition inputs). No language rule was changed for a test.

The matcher already proposes substitutions from syntactically equal retained
heads and keeps the original application equality as an obligation. It does not
unfold a body or assert an inverse theorem. The repaired regression distinguishes
that valid proposal from three failures: a bare result type, a different head,
and an incompatible expected result. It also checks that an assumed equality
between two head applications does not prove equality of their rigid arguments.
The original scheme remains unchanged after both successful and failed queries.

A source integration regression infers a user-defined normal-form input through
a callback's effect signature, executes the matching handler through IIR, and
rejects replacing the retained handler label with the function body's bare type.
No compiler implementation or semantic documentation needed modification.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test effect_inference`:
  exit 0, `/tmp/luna-tools-2xfiu9yw/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_form_inference`:
  exit 0, `/tmp/luna-tools-wrnnnv5v/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p elaboration`:
  exit 0, `/tmp/luna-tools-dn5x_plh/result.json`, no exclusions. This closes
  the expanded-suite failure recorded in the previous stage.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-gr7qgzqm/result.json`; `git diff --check`: exit 0.

### Declaration projections and concrete parent obligations

Declaration-only member-domain lookup and inherited interface traversal now use
completed input domains while retaining the caller telescope. These paths still
read declaration contracts without manufacturing implementation evidence.
Concrete implementation checks specialize parent and Self-capability obligations
through the same bounded trait input context before proving them.

Source coverage includes public function signatures returning associated types,
lookup through a value-indexed child trait, and a default method whose declared
parent fixes Type-valued inputs. Missing parent implementations, a different
parent instance and a wrong associated argument type remain rejected.
The original combined public/inherited regression failed at the concrete child
implementation (`/tmp/luna-tools-bwoyzx7d/result.json`); a temporary diagnostic
trace identified check_interfaces as the remaining failure after declaration
substitution was migrated. The trace code was removed before final validation.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test trait_phases --test trait_purity --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-x64ytm5z/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-16dxwph4/result.json`; `git diff --check`: exit 0.

The remaining contextual-code and ordinary-argument requirement substitution
paths still require their own audit; this is not a blanket completion claim.

### Ordinary argument requirements retain declaration domains

Ordinary-input type requirements and capability checks now specialize the callee
signature and obligations through declaration input domains. The ordinary frame
and the existing shift of caller arguments remain explicit before substituting
ordinary argument bindings. A Type-valued declaration index therefore no longer
causes unrelated ordinary Type requirements to fail with a source-sort mismatch.

Regressions cover equality and trait requirements, generic forwarding, and a
caller ordinary Type binding used both as a declaration index and as an ordinary
argument (`keep<Type,U>(U,value)`). False equality inputs and missing trait
implementations remain rejected. Both initial positive cases failed in
`/tmp/luna-tools-39dcdee4/result.json`; the final forwarding coverage passes.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test instantiation --test trait_phases --test trait_purity --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-k_z7bzps/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-2wleg9h7/result.json`; `git diff --check`: exit 0.

Contextual-code method contracts and other unreviewed substitution consumers
remain outside this stage's completion claim.

### Contextual-code contracts and generator captures complete input domains

Contextual-code method program refinement now specializes predicates with
bounded declaration-domain substitution, charging the required-invocation
budget before continuing data-only implication checking. Generator signature
substitution uses the same domain context. Inference schemes explicitly supply
their external generic frame: validating these open signatures as closed terms
caused nine regressions, now fixed without weakening scope validation.

Captured value clauses are classified after completing their domains. A clause
whose domain becomes Type retains its original declaration identity and receives
the ordinary Type-value representation; it does not gain a fabricated rigid Type
binder. Source type lookup likewise checks the completed domain before accepting
a captured clause as a Type. The new cases execute code methods with public
preconditions, reject strengthening and invalid invocation, generate nominal
values with both Type and scalar indices, and reject a wrong nominal index.
Initial failures: `/tmp/luna-tools-5c9a4f5j/result.json`,
`/tmp/luna-tools-1mpwwuk8/result.json`, `/tmp/luna-tools-9hjbsuf9/result.json`,
and `/tmp/luna-tools-zqidwfm_/result.json`. The closed-scope regression is recorded
in `/tmp/luna-tools-3s9v57d1/result.json` and resolved by explicit scheme frames.

The broad CMTT suite also contained an obsolete rejection of inference through
a retained normal-form pattern subject. As established in the preceding effect
inference audit, this is a candidate proposal, not head unfolding or an inverse
equality theorem. That test now accepts matching retained heads while rejecting
conflicting indices and a bare body-result type.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt_methods`:
  exit 0, `/tmp/luna-tools-m5zqyswr/result.json`, all 144 cases.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test quote_functions --test nominal_value_parameters --test trait_phases --test trait_purity --test sysroot_preludes`:
  exit 101, `/tmp/luna-tools-rgt2wl9s/result.json`. CMTT (580), phase boundaries
  (3), and nominal parameters (288) pass. quote_functions then fails 100 of 104
  cases during HIR lowering because its standalone fixture omits primitive
  bindings, notably bool. This happens before the modified elaboration paths;
  that test entrypoint still needs migration. Do not report this command green.
- Remaining groups run explicitly:
  `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test trait_phases --test trait_purity --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-ee4d56c9/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-koj69jek/result.json`; `git diff --check`: exit 0.

Quote-function expansion has separate substitution consumers; this stage does
not claim that every contextual-code pathway has been migrated.

### Standalone quote fixtures load primitive bindings

The quote_functions entrypoint now uses the shared standalone primitive-binding
fixture writer, matching other no-sysroot compiler tests. Production primitive
lookup remains ordinary builtin.prelude lookup; no compiler keyword or fallback
was restored. Existing quote expectations and compiler implementation are unchanged.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test quote_functions`:
  exit 0, `/tmp/luna-tools-8p_b8c5z/result.json`, all 104 cases without exclusions.
- `git diff --check`: exit 0.
This resolves the fixture failures recorded in the previous stage.

### Quote expansion specializes declaration domains

Quote expansion now caches immutable declaration input domains instead of source
sorts. Template specialization uses the bounded Function domain substitution,
and public trait program contracts are specialized in the completed method's
ordinary input frame before existing binding remapping. Domain collection,
selected domain copies and substitutions consume the expansion budget.

The new source cases cover default and explicit quote implementations with
Type-valued interface indices, nonempty inherited preconditions, and a direct
quote function that references an ordinary input twice. IIR event observations
show one evaluation of that input and no quote runtime call. Invalid input
domains and false interface preconditions remain rejected. The original template
failure is `/tmp/luna-tools-6yo_x2_k/result.json`.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test quote_functions --test cmtt_methods --test cmtt_phase_boundaries --test nominal_value_parameters --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-rfopj35w/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-kt2kvfrz/result.json`; `git diff --check`: exit 0.

This closes the exercised quote expansion paths, not all remaining declaration
substitution consumers or the full CMTT implementation.

### Partial enum contexts complete dependent index domains

Enum payload inference now consumes known contextual arguments in declaration
order, completing each later domain before assigning its inference sort. Only
unknown inputs become open inference slots. Payload field types use declaration
domains when specialized; final constructor admission and field compatibility
checks remain unchanged. The shared trait-domain prefix substitution also uses
bounded domain evidence, so a later Type parameter can follow an index whose
completed domain is Type.

The source regression first exposed the prefix error in
`/tmp/luna-tools-o6v2ezz_/result.json`; fixing that revealed the unresolved partial
constructor context in `/tmp/luna-tools-dgcimokp/result.json`. Final cases execute
complete and partial enum contexts, tuple payload spreads and record defaults,
and reject a distinct nominal index. No concrete enum name is recognized by the
implementation, and normal-form heads remain opaque.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test cmtt_methods --test quote_functions --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-we46dsbe/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-xyn5t139/result.json`; `git diff --check`: exit 0.

### Inherited declaration inputs complete method type defaults

Type-clause default completion now uses declaration-domain substitution in the
saved caller scope. A method can inherit `T, index:T` from its owner and complete
its own default when `T` is Type and `index` is a Type value. Previously the final
substitution still required the source comptime-value sort; the executable
reproduction failed with `expected comptime-value, got type` in
`/tmp/luna-tools-9h16uys4/result.json`.

Regression coverage executes nominal defaults, explicitly supplied function
inputs, inherited method defaults, defaults containing the inherited nominal
index, and explicit overrides. It also checks ordinary scalar indices and
rejects mismatched nominal identities. Default evaluation order, cyclic/default
reference checks, and declaration admission remain in place.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test cmtt_methods --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-1ecicncb/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-9z5ra111/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test default_calls`:
  exit 0, `/tmp/luna-tools-dp82id8n/result.json`, no exclusions.

Separate open inference gap: removing `<Type,bool>` from the call in
`type_valued_indices_complete_function_type_defaults` reports `NeedsContext`
(`/tmp/luna-tools-h0d_64te/result.json`). Explicit inputs work. Ordinary call
Scheme inference retains fixed parameter sorts; unlike implementation matching,
it still needs domain staging when solving a preceding input changes a later
input's sort. This change fixes default substitution, not that inference path.

### Ordinary call inference stages dependent declaration domains

The previously recorded `read(Tagged<Type,bool>{...})` inference gap is closed
for ordinary generic calls. A declaration-aware Scheme entrypoint observes
candidate-local proposals, completes input domains in declaration order, and
rebuilds the candidate if a dependent input changes sort. It retains inference
slot identities, including provisional ordinary Type inputs, and replays actual
argument/result relations against the domain-specialized signature. Previously
known proposals are constrained again. Candidate completion still discharges
all relations; the caller still performs declaration admission and requirements.

This path is connected to independent ordinary input preparation, final generic
call checking and generic call descriptions used by deferred signatures.
Domain completion uses the shared DeclarationSubstitution, preserves the caller
scope, and does not unfold normal-form heads. Staging and observations have
bounded work; exhausted candidates do not mutate the reusable Scheme.

Tests cover Type-valued and scalar indices, defaults, deferred forwarding,
ordinary comptime Type inputs, shared-index conflicts, invalid input domains,
and declaration requirements. Solver tests additionally cover provisional slots,
caller-bound Type values, scope escape, expected-result conflicts, insufficient
information, budget failure and recovery using the unchanged Scheme.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`:
  exit 0, `/tmp/luna-tools-9h7hyw2c/result.json`, no exclusions.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test default_calls --test open_functions --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-nbpgjdwo/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-9j3485zj/result.json`; `git diff --check`: exit 0.

Remaining scope: effect input inference, spread-specific inference and CMTT
entrypoints still contain fixed-sort Scheme consumers. This is not a claim that
all dependent inference paths are complete.

### Effect formation and inference complete dependent Type inputs

Effect request inference now uses declaration-domain staging. Formation admits
arguments against completed domains and specializes signatures, type predicates
and capability requirements through DeclarationSubstitution; it no longer rejects
Type-valued indices solely because their source binder was comptime-value.

A shared bounded effect-signature specialization method is used by IIR
materialization and memory/move effect analysis. Formation, allowed-row identity,
handler routing, ABI restrictions and resource obligations remain distinct
checks. The initial source reproduction failed during formation in
`/tmp/luna-tools-wi4stvme/result.json`; after fixing formation it reached the
fixed-sort materialization failure in `/tmp/luna-tools-nphe29u1/result.json`.

Defaults now wait for parameter-domain dependencies as well as expression reads.
The effect-group result-context case exposed the missing T dependency of a
`.copy:T=index` default in `/tmp/luna-tools-hdq_r4mn/result.json`. Completing the
shared dependency calculation fixes this without treating the installed handler
row as an inference source.

Source cases execute inferred and explicit operations, effect groups, scalar
indices, result context and dependent group defaults. They reject a different
handler identity and a false declaration requirement. Formation tests check
completed signature identity, invalid domains, false predicates, exhausted
budgets and recovery. This does not integrate spread-specific or CMTT inference
consumers with domain staging.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p nir -p elaboration -p materialization -p program_verification`:
  exit 0, `/tmp/luna-tools-xy5j590t/result.json`, no exclusions.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test default_calls --test effect_contracts --test effect_spreads --test effect_memory_transfers --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-49f0s0iu/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-up0nigo1/result.json`; `git diff --check`: exit 0.

### CMTT generator calls share dependent-domain inference

Generator call candidates now use declaration-domain staging. Deferred Type and
value defaults both retain inference slots; dependency availability is inspected
in declaration form, while default values constrain their candidate slots only
after preparation. This removes the separate rigid-value replacement path that
lost domain evidence when a default completed to a Type value.

Generator inference views use the candidate's completed input sort for unresolved
placeholders. Definition checking retains abstract deferred inputs of either
sort without executing defaults; real invocations still require completion.
The initial Type-indexed generator failed in
`/tmp/luna-tools-8byw7rxe/result.json`; the deferred index default failed in
`/tmp/luna-tools-liywpirl/result.json`. Broader checks exposed a definition-side
forwarding regression (`/tmp/luna-tools-0u416hh6/result.json`), fixed by extending
the existing abstract-default handling to value slots rather than executing or
inventing defaults during definition checking.

Source regressions execute data-input and quoted-code-input inference,
Type-valued defaults in generated nominal types and scalar-index equivalents.
A conflicting generated result type remains rejected. Existing generator tests
cover default ordering, forwarding, invocation effects and phase boundaries.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_methods --test cmtt_phase_boundaries --test nominal_value_parameters --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-77inh35c/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-acyknh6u/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution --test calls`:
  exit 0, `/tmp/luna-tools-k6cam0w_/result.json`, no exclusions.

Remaining consumers include spread-specific inference and the speculative
CMTT data-result hint path. This does not claim all inference entrypoints are
integrated with domain staging.

### Packet and speculative inference use completed declaration domains

Ordinary functions, methods, deferred signatures and effect requests now pass
input declaration identities through packet evidence collection. Packet and
property collector candidates use the shared domain-staging entrypoint, including
open-call input preparation. Enum packet contexts retain their already prepared
fixed-sort frame. The original simple callback/packet case already passed via
final ordinary-call checking (`/tmp/luna-tools-_yl2cx1h/result.json`); this change
makes the intermediate contexts use the same domain evidence instead of relying
on that later recovery.

CMTT generic data-result hints and contextual property selection also use domain
staging. Static value hints whose completed domain is Type now delegate to static
Type evidence with the same depth limit. The nested ordinary comptime producer
case failed with NeedsContext in `/tmp/luna-tools-jpawr6g7/result.json`; this
fix supplies its completed nominal result without executing the producer.

Initial parameter registration now includes positional value clauses for
contextual data consumers (quoted input, ordinary data output). Previously their
signature domain table contained the value domain but their parameter identity
list omitted that input, causing `normal-form value argument is not an available
declaration input` (`/tmp/luna-tools-kajwitn5/result.json`). The source regression
uses a direct comptime binding and then consumes its data value.

Source cases cover callback contexts from computed packets, ordinary and method
calls, deferred result annotations, exact effect identities and nested ordinary
comptime producers. An IIR event assertion verifies before/packet/after execute
once each in source order. Temporary diagnostic instrumentation was removed.

Remaining separate limitation: a contextual data consumer called directly inside
another generator's ordinary argument, for example
`~<build(inspect('(Tagged<Type,bool>{value:false})))>`, still reports that nested
ordinary-expression scheduling is unsupported. This is recorded by
`/tmp/luna-tools-fx9paypv/result.json`; accepting its declaration and direct
binding does not close that scheduling gap.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test extended_calls --test effect_spreads --test open_functions --test cmtt --test cmtt_methods --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-1u27a69a/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-0b3og_ud/result.json`; `git diff --check`: exit 0.

### Nested contextual data arguments and dependent result contexts

The direct argument example recorded above now executes at the shared
source-ordered generator input boundary, including when the caller provides a
partial result context. Contextual consumers pass that context into their normal
call candidate; ordinary expression checking still never executes these calls.
The original failure is reproduced by the new source test in
`/tmp/luna-tools-7lswj5xi/result.json` (exit 101).

Completing this path exposed a distinct inference issue: a partial context for
`Tagged<T,index:T>` exported the same unknown-value marker as a fixed value
domain. When the nested call inferred `T=Type`, importing that marker created a
stale comptime-value constraint against a Type argument and left inference
waiting (`/tmp/luna-tools-hw9vor5f/result.json`, exit 101).
Domain staging now records inputs whose domain remains an unresolved clause
input. Only those slots export a dependent unknown-value context. Importing the
context uses a settled candidate component's sort when available, preserving
nested shape and all known evidence. Fixed-domain unknown values retain their
original sort; the existing rejection regression remains unchanged. Failed
context constraints and budget exhaustion do not mutate the candidate.

Source regressions cover dependent Type indices, incompatible expected indices,
source-order mutation with each nested argument evaluated once, postcondition
failure, and unused definitions that must not execute their calls. Solver tests
cover optional, tuple, and record nesting, budget failure/recovery, and rejection
of conflicting known values. This is not general scheduling for contextual calls
inside arbitrary ordinary expressions, nor a replacement of construction
interpretation by the eventual unified invocation VM; those remain open work.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution --test calls`:
  exit 0, `/tmp/luna-tools-m_eyrl2p/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test cmtt --test cmtt_methods --test cmtt_phase_boundaries --test sysroot_preludes`:
  exit 0, `/tmp/luna-tools-3np6ano2/result.json`, no exclusions.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-f945ch2u/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p type_resolution`:
  exit 0, `/tmp/luna-tools-6klgr5oj/result.json`.

### Dynamic normal-form callable bytecode

Type-directed `Call` nodes whose callee has a normal-form callable signature now
compile through the linear call protocol. Callee selection runs before arguments;
each source-ordered argument is frozen to its ordinary residual input term before
the next expression runs. Completion uses the existing normal-form domain and
requirement checks and returns the opaque head application, never a head-body
invocation. Computed callee expressions are compiled through the same expression
worklist. No new AST evaluator or normal-form argument language was added.

Call instructions retain the normal-form signature and expected input count.
Pending bytecode calls distinguish ordinary argument values from frozen normal
inputs, with S-expression dumps for both. Arity is checked before arguments;
duplicate slots and illegal defaults fail at their source-order boundary. Invalid
input domains are still rejected by the common completion routine. Existing
ordinary call handling and fallback for other unsupported constructs remain.
This closes dynamic normal-form application fallback, not the whole invocation
VM migration. Source routing of dynamic named/default clauses is unchanged.

The compiler regression previously required this construct to return Unsupported;
it now checks the distinct, deterministic normal-form call dump. Differential
execution tests cover computed callee effects, argument effects, duplicate slots,
arity, illegal defaults, invalid domains and recovery. Source tests assert exactly
one compiled invocation per wrapper and no head entry events, including failed
and successful requires checks. The real shipped sysroot test passes List and
Tuple as callable arguments and checks their original identities.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-n_hj10gm/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test sysroot_preludes --test normal_form_inference --test normal_form_methods --test library_meta_objects`:
  exit 0, `/tmp/luna-tools-b1mq67fh/result.json`.
- After extending the source case with requires rejection/recovery:
  `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types dynamic_type_factories_execute_in_bytecode_without_entering_their_heads`:
  exit 0, `/tmp/luna-tools-w8olpj3k/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-rtllj14d/result.json`; `git diff --check`: exit 0.

### Computed ordinary callees in invocation bytecode

Ordinary Call lowering no longer restricts the callee to a function declaration,
static instance, or local binding. The expression worklist compiles the callee
first, followed by source-ordered arguments and the existing call suspension.
Unsupported callee expressions still reject the whole compiled carrier before
execution, so this does not silently omit unsupported operations.

A callee obtained through a block, branch, field, or another call can retain a
selected declaration instance. When an existing prepared call carries declaration
default templates, both tree interpretation and bytecode now substitute those
templates from the selected instance for all computed callee forms. Direct static
instances and trait methods keep their existing preparation. This does not grant
parameter names or default protocols to ordinary function values; source routing
still follows docs/functions/calls-and-returns.md.

Source tests require compiled invocation counts for blocks, nested selector
calls, callable struct fields, conditional selection, and early return that skips
argument evaluation. The existing selected-instance default regression exercises
both local and computed carriers, comparing bytecode with interpretation.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test comptime_bytecode_defaults`:
  exit 0, `/tmp/luna-tools-qjgppebl/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-129w2a34/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-sd3vxpfu/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_references --test comptime_bytecode_types --test library_meta_objects --test comptime_clause_inputs`:
  exit 0, `/tmp/luna-tools-r8nmy0sx/result.json`.

### Closure construction in invocation bytecode

Closure construction now emits a checked constructor, source-ordered capture
instructions, and a completion instruction. Explicit capture expressions use the
ordinary instruction worklist; implicit captures read lexical slots through the
same snapshot operation as interpretation. The closure body stays an owned Arc
and enters the existing invocation compiler when called. No instruction evaluates
its body or a capture expression with the tree evaluator.

The interpreter and bytecode share callable/capture shape validation. Each
construction allocates fresh capture cells, while repeated calls retain that
closure's updates. Type failures, duplicate capture bindings, and early exits
preserve capture order and do not publish a partial closure. Constructor metadata
and pending capture environments have deterministic S-expression dumps. Runtime
cleanup and mixed-stage interpretation retain their existing execution paths.

Source regressions require compiled construction and callback invocations for
returned closures, independent instances, local mutable captures and empty
captures. Differential tests exercise explicit capture effects, shape failures,
per-slot rejection, duplicate identities, early return and recovery. A shipped
sysroot example folds a List with a privately captured counter and verifies both
callback state retention and isolation from the outer binding.

Remaining performance work: `invocation_code` caches stable declaration bodies,
but an owned closure body currently compiles on every invocation. Reusing its
immutable code must remain separate from invocation-result caching and must not
retain captured environments as cache entries.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-jz5f761j/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_closures --test comptime_list_operations --test sysroot_preludes`:
  closure and List groups passed (1 and 54 cases); overall exit 101 due to the
  new sysroot fixture's missing whitespace before assignment,
  `/tmp/luna-tools-5827anrv/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_closures --test sysroot_preludes --test comptime_bytecode_defaults --test comptime_bytecode_references`:
  closure, defaults and references groups passed. Overall exit 101 because the
  new sysroot fixture transported its comptime result into runtime without a
  bound Splice protocol, `/tmp/luna-tools-eeykqzz_/result.json`. The fixture now
  invokes its comptime entry directly; it does not change cross-stage semantics.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_list_fold_retains_comptime_callback_state`:
  exit 0, `/tmp/luna-tools-9ptfjsoz/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-s35snhmc/result.json`; `git diff --check`: exit 0.

### Reuse immutable owned invocation code

Owned invocation bodies now share a bounded compilation cache alongside immutable
package declarations. The owned lookup uses a process-local Arc address only as
an index and validates a live weak identity before reusing code. It is not a
language type identity or a persistent cache key. The cache does not hold a strong
source-body reference or any live captured environment; copy-on-write replacement
receives its own code. Unsupported lowering can be remembered, while compilation
errors and budget exhaustion are never published as cache entries.

Both entry classes share the existing work-weight quota. Replacement subtracts
the old weight, and capacity eviction clears both classes. Invocation results,
argument admission, contracts, effects and capture cells are not cached. Default
expression execution retains its existing compilation path; this change routes
owned invocation frames, including closure bodies, through code reuse.

The callback-loop regression executes two rounds of 100 state-changing callback
calls. It observes 202 compiled invocation frames but only two compilations
(the wrapper and callback), with fresh capture cells on the second round. The
independent-closure case also checks sharing of body code without sharing cells.
Cache tests cover live identity, copy-on-write separation, source lifetime,
unsupported-code reuse, exhausted compilation/retry, replacement accounting and
joint declaration/owned capacity eviction. These are compilation-work results,
not a wall-clock speedup claim or evidence of package-level world reuse.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-c7xvfpl8/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_closures --test comptime_bytecode_defaults --test comptime_bytecode_types --test comptime_list_operations`:
  exit 0, `/tmp/luna-tools-qm2sb36z/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-4u2w4wrj/result.json`; `git diff --check`: exit 0.

### Reuse prepared default-expression code

Default execution frames now enter the same owned-code compilation cache as
closure invocation frames. The immutable template Arc is the code identity;
every evaluation still creates its current declaration-side environment, reads
the prepared inputs, calls producers, checks contracts and delivers a new value.
Dynamically specialized templates with distinct Arc identities remain distinct.
This does not cache defaults' results or change the Type-only invocation-result
cache policy.

A two-round source loop supplies 100 different explicit arguments per round.
All 200 default evaluations enter their producer, compute the per-input result
and contribute to the expected sum. The four code bodies (wrapper, callee,
producer and default expression) compile once each. Existing default failure,
early-control-flow, source-order and selected-instance cases remain in the
same focused suite.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_defaults`:
  exit 0, `/tmp/luna-tools-83c4wozk/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-9zxt0nmi/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-ip0zjlbp/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test default_calls --test comptime_clause_inputs --test comptime_bytecode_closures --test library_meta_objects`:
  exit 0, `/tmp/luna-tools-yr08jax3/result.json`.

### Compile BitVec selections and concatenation

BitSlice, BitIndex, BitWindow and BitConcat now lower to linear bytecode.
Operand preparation remains source ordered: a non-bit first operand fails
before later expressions execute, while range checks follow endpoint evaluation.
Tree interpretation and bytecode share value-level width, bounds and budget
checks. Opcodes contain typed registers and immutable operation descriptors,
with S-expression dumps; they do not recursively interpret source expressions.

Source tests require actual bytecode entry and explicit expected values for
indexing, windows, concatenation, zero-width values and a 512-bit input. Negative
and excessive endpoints are compared with tree interpretation, followed by
successful calls on the same interpreter. Direct NIR tests cover early operand
failure and side effects. This stage does not compile BitConvert, TakeBits or
bit-place writeback, and does not complete symbolic-width support.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-0hr41s1w/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_bits --test comptime_bytecode_closures --test comptime_bytecode_defaults`:
  exit 0, `/tmp/luna-tools-pu2askh4/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution bitvec_runtime_`:
  exit 0, 10 selected tests, `/tmp/luna-tools-8sg3q8fd/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-es20h1kg/result.json`; `git diff --check`: exit 0.

### Compile BitVec equality and representation conversion

BitEqual and BitConvert now use the typed-register bit-operation instructions.
Tree interpretation shares the same value implementation. Equality checks both
widths and charges comparison work; conversion preserves the existing target
layout checks, zero extension and exact signed/IEEE representations. Neither
path evaluates an expression tree from an opcode. Operand preparation preserves
early type failure before subsequent effects.

Source coverage checks actual compiled entry for widening, unsigned delivery,
signed reinterpretation, signed zero, infinity, NaN payloads and equality.
Direct NIR coverage rejects narrowing even for a small stored value, mismatched
float width and restricted bool representations, then successfully converts on
the same interpreter. TakeBits and bit-place writeback remain separate work;
this does not expand which resource-bearing representations may be constructed.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_bits`:
  exit 0, `/tmp/luna-tools-i22jidgh/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-gzvd1zsx/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution reinterpretation`:
  exit 0, two selected tests, `/tmp/luna-tools-rj534srk/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-7pl4s5up/result.json`; `git diff --check`: exit 0.

### Compile defined representation reads

TakeBits now lowers its operand to a register and invokes the shared value-level
representation reader. Scalar reads retain target-specific width and domain
checks; encoded enums retain nominal identity, declared layout and payload
construction checks. Reading a representation produces a value, not a retained
place. The helper is separated from the tree dispatcher and consumes no Expr.

Source tests require compiled entry for encoded signed payloads, f16/bf16 raw
encodings and BitVec identity. NIR tests reject output-width mismatches, an
out-of-domain scalar, unsupported bool reading and a non-BitVec output, then
accept a valid read on the same interpreter. Existing encoded-enum tests cover
NIR/IIR metadata checks and generic wide payloads. Bit-place writeback remains
uncompiled; this step does not add general object representation reading.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_bits --test encoded_enums`:
  exit 0, `/tmp/luna-tools-2m81o7ix/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-lmxr6jea/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-8g8ykqfs/result.json`; `git diff --check`: exit 0.

### Compile fixed-range bit writeback

BitAssign now lowers binding admission, RHS evaluation and final writeback to
separate linear instructions. The write instruction resolves the current root
and field path after the RHS. It shares checked value replacement with tree
interpretation and commits only after width/domain checks and bounded value
construction succeed. Storage cells remain shared; a previously read BitVec
remains a snapshot. Dumps include the storage/output types, range and field path.

Source coverage requires compiled entry for overlapping and nested ranges,
empty ranges, snapshots and RHS writes to both the target and sibling fields.
Malformed NIR cases verify that failed replacement retains completed RHS writes
without a partial bit update, then successfully update on the same interpreter.
Dynamic BitUpdate selections and runtime pointer writes are not compiled here;
this stage provides their reusable final value-write operation.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_bits`:
  exit 0, `/tmp/luna-tools-iyew8yfh/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-1gh__dm3/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution writeback`:
  exit 0, 16 selected tests, `/tmp/luna-tools-x2ihouf6/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-chy6_xai/result.json`; `git diff --check`: exit 0.

### Compile dynamic bit selections and writeback

BitUpdate now compiles to an immutable typed-register plan plus begin, select
and finish instructions. Admission checks the binding and the complete selection
shape before endpoint effects. Each selection evaluates its endpoints in source
order, checks the range, then composes the relative offset before the next
selection runs. The RHS runs only after selection succeeds. Final writeback
uses the frozen range and rereads current storage through the shared checked
replacement helper, preserving RHS mutations without reevaluating indices.

NIR exposes an expression-free BitSelectionLayout and common admission checker;
existing NIR validation and the VM use the same width/type rules. Plans own
registers, types and source locations, not expression trees. Layouts, inputs,
plans and instructions all have structured S-expression displays. Frame-local
range state does not enter code/result caches.

Source coverage requires bytecode entry for dynamic windows, nested static/index
selection, empty windows, RHS index/storage mutation and failure/recovery. NIR
coverage observes endpoint order, preflight rejection without effects, range
failure before later selections/RHS and successful nested offset composition.
Runtime pointer writeback stays on its existing runtime path; comptime does not
acquire pointer semantics from this implementation.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_bits`:
  exit 0, `/tmp/luna-tools-jlb_w72k/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p nir -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-lsmdf42v/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution writeback`:
  exit 0, `/tmp/luna-tools-7axc13rv/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-1pw0d97i/result.json`; `git diff --check`: exit 0.

### Compile text and name observations

TextQuery, SymbolName and EnumName now lower to a typed admission check followed
by operand bytecode and a value observation. Tree execution shares both helpers;
it borrows input/output types rather than copying them on each observation.
Enum observations retain nominal checks, residual delivery and name-copy budget.
Text queries retain UTF-8 byte indexing and optional out-of-range results.

Source coverage invokes the shipped text wrappers and checks compiled frame
counts, empty/multibyte strings, maximum indices, symbol/enum names and early
return. NIR tests check that invalid observation shapes fail before operand
effects. The hash fixture now supplies the primitives module imported by the
shipped comparison module, rather than flattening those declarations at root.

The full shipped text-hash compilation regression remains explicitly ignored:
its result is correct, but both wrapper and method fall back, with zero compiled
frames. Diagnostic lowering identified TraitMethod and BitVec And/Or as the
remaining unsupported nodes. The expectation of two compiled frames is retained
for the next implementation stage, not weakened to count fallback as completion.
Temporary lowering diagnostics were removed.

Validation in /home/fin/luna:
- Initial observation test selected the runtime byte wrapper because of a
  duplicate leaf name; renamed the new entry to read_byte. Initial library_hash
  run failed on the fixture's missing primitives module. After fixing that
  dependency, 11 existing tests passed and the new compilation regression failed
  with 0 rather than 2 compiled frames (`/tmp/luna-tools-8rfc5cjy/result.json`).
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-grjzpamh/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_observations --test library_hash`:
  exit 0, `/tmp/luna-tools-jtlvuad0/result.json`; observation suite and 11 hash
  tests passed, one known incomplete compilation regression ignored as above.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-_i_ldog3/result.json`; `git diff --check`: exit 0.

### Compile eager bitwise operations and checked program predicates

BitVec And/Or now evaluate both operands in source order and share bounded
value operations with tree execution. Boolean And/Or retain their existing
short-circuit branches. Program Check/Require/EnsurePredicate nodes compile
shape admission, condition evaluation and checked delivery separately. Shape
inspection is charged during compilation; failed conditions record their check
and reject through the shared predicate completion helper. Runtime cleanup
continues through the original scheduled interpreter path.

The shipped string-hash body now compiles as a whole and produces the documented
FNV-1a values for empty, UTF-8 and embedded-NUL strings. A direct invocation of
that actual library declaration asserts one compiled frame per call. Source
regressions distinguish eager zero/all-ones bit operations from Boolean short
circuiting and exercise predicate rejection followed by successful reuse.

The outer trait-call regression remains ignored. Investigation confirmed that
simply treating TraitMethod as a function constant is insufficient: interface
program conditions, declaration-side argument bindings and default environments
must survive dispatch. Next work must integrate that public contract into call
preparation and continuation delivery, not bypass it for the hash example.
Initial hash-body testing exposed the additional unsupported CheckPredicate;
it was implemented rather than deleting the library's assumes. Temporary
lowering diagnostics were removed.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_hash shipped_text_hash_body_uses_bytecode_for_all_bytes`:
  exit 0 after bitwise/predicate support, `/tmp/luna-tools-qpsw9z5v/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-l6d_zuak/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_bits --test library_hash`:
  exit 0, `/tmp/luna-tools-py_lpzx5/result.json`; one known outer trait-call
  compilation regression remains ignored, not reported as completed.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-lw4oari8/result.json`; `git diff --check`: exit 0.

### Compile static trait calls with public interface contracts

Direct TraitMethod callees now preserve their immutable identity in call
preparation. The VM obtains the public interface program and declaration-side
bindings before resolving the implementation. Defaults therefore observe the
interface's parameter names, even when the implementation uses different names.
Standalone method values retain the existing admission/erasure restrictions.
Interface contract lookup is shared with the tree path.

Nonempty interface programs suspend through a boxed caller-local continuation:
entry observations and snapshots, entry checks, implementation invocation, exit
checks, then result delivery. Checks execute as ordinary contract fragments via
the invocation driver, not recursively from a bytecode instruction. Contract
fragments are distinct from default evaluations in execution accounting. Live
argument, snapshot and result environments remain in the frame, never in a code
cache. Bytecode remains restricted to invocation paths without runtime cleanup
schedules; scheduled runtime execution retains its existing cleanup behavior.

Source tests cover different interface/body parameter names, dependent defaults,
explicit overrides, method values, specialized interfaces, precondition failure
before body entry, postcondition failure after body completion, successful reuse
and nested independent entry snapshots. To test the retained precondition itself,
the test elaborates a checked assumes then removes that caller proof from NIR;
it does not weaken source proof admission. A runtime Interpreter::new comparison
was inappropriate for a comptime self reference, so the source test checks actual
compiled entry and explicit expected values in the correct comptime mode.

The complete shipped string-hash regression is enabled and passes: both wrapper
and library implementation enter bytecode. Dynamic ComptimeFunctionInstance
callee construction still uses its existing migration path; this stage does not
claim all staging expressions have linear lowering.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-d_u6f13l/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_traits --test library_hash`:
  exit 0, `/tmp/luna-tools-59vqbmyp/result.json`; 13 hash tests, none ignored.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_traits --test comptime_bytecode_defaults --test library_comptime_comparison`:
  exit 0, `/tmp/luna-tools-jgt71us9/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution trait_optional_arguments`:
  exit 0, four selected tests, `/tmp/luna-tools-gfa1_iwd/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test execution staged_trait_calls`:
  exit 0, five selected tests, `/tmp/luna-tools-_6svghld/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-hipxfp2v/result.json`; `git diff --check`: exit 0.

### Compile dynamic function and trait instances

ComptimeFunctionInstance now lowers to admission, ordinary linear clause-input
code, and instance completion. The immutable descriptor contains declaration and
packet/result types, never executable child expressions or invocation values.
Admission runs before clause effects; completion validates current packet values
and uses the existing declaration-domain and requirement checks on cache hits.
The tree and bytecode paths share admission and packet residualization.

Direct dynamic trait callees retain their interface identity until BeginCall.
This preserves interface programs, declaration-side parameter bindings and the
frozen clause inputs used to specialize defaults. Stored method values retain
the existing contract-erasure restrictions. Clause return/control transfer skips
completion and invocation. New descriptors and suspended instance state have
S-expression dumps.

A source test exposed an existing elaboration boundary: saving a callable with
optional parameters as an ordinary function value still reports missing parameter
protocol metadata. This change does not fix that boundary. Source tests separately
exercise ordinary stored instances and direct dynamic calls with optional inputs;
no source-admission check was weakened. Another initial test incorrectly required
an arbitrary explicit argument to equal a clause input; its interface contract was
corrected to describe the actual returned argument.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_traits --test dynamic_trait_clauses --test comptime_clause_inputs`:
  exit 0, 61 tests, `/tmp/luna-tools-3hhhbu3l/result.json`. New source tests
  assert compiled entry counts, not just computed values.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-0yie8zjn/result.json`, including effect-free rejection
  of malformed instance metadata in both execution routes and recovery.
  An earlier unit build exposed a private test helper; its visibility is now
  shared within the crate under cfg(test), without changing the runtime API.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_hash --test library_comptime_comparison`:
  exit 0, `/tmp/luna-tools-btm_gg6m/result.json`; library comparisons and
  hash behavior remain valid, including compiled hash wrapper/body coverage.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0, `/tmp/luna-tools-duzcp96s/result.json`; `git diff --check`: exit 0.

### Linear sequential pattern computations

The pattern compiler now flattens Then and Guard chains with a metered heap
worklist. First-pattern success precedes expression execution and matching of
its result; each failure branches to the enclosing candidate's existing scope
rollback. Completed external effects are retained. Expressions use ordinary
invocation continuations, including calls, defaults and control transfers.
The same lowering serves expression matches, let patterns and contextual/NF
input matches. No instruction recursively executes a pattern expression.

Structural patterns remain atomic. Executable patterns nested under structural
fields, Observe, Remap or Or still select the tree path before any execution;
this stage does not claim complete pattern linearization. CodeEvidence remains
non-executable. Previous unsupported-code cache tests now use CodeEvidence;
the staged-input test covers both linear Then and a still unsupported Observe
wrapper, checking that neither route replays an operand effect.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-ckrigjdp/result.json`. Earlier runs identified the
  second pattern-lowering consumer in normal_types and two obsolete fallback
  assertions; all consumers and independent fallback checks were updated.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_patterns --test comptime_bytecode_defaults --test comptime_bytecode_types`:
  exit 0, `/tmp/luna-tools-dwzosu2l/result.json`. Tests assert compiled frame
  counts as well as results, ordering, early return and successful bindings.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-z65ex58l/result.json`; `git diff --check`: exit 0.

Separate open design boundary: docs/functions/higher-order-functions.md and
parameters.md define default evaluation and ordinary higher-order identities,
but do not settle whether different default computations can inhabit the same
optional-parameter callable type. Asked the author; no local-source tracking or
implicit protocol erasure was introduced while that decision is pending.

### Compile candidate-local alternatives, observations and binding remaps

Pattern lowering now linearizes Or, Binding, Observe and Remap around sequential
computations. Each alternative gets its own scope; failure restores candidate
bindings before the next alternative, while success commits into the containing
candidate. An outer guard failure goes to the next arm, never back into an
already successful Or. Remap validates duplicate sources/targets before inner
computation, then requires actual exports of that successful candidate and
performs simultaneous replacement. Ambient locals cannot impersonate exports.
Observation aliases are admitted only when absent and are removed on every exit.

VM scopes now journal old pattern bindings separately from ordinary local
initialization. Commit merges the journal into its parent; failure, scope unwind,
early transfer and frame restoration roll it back. Ordinary assignment effects
are not journaled. This also fixes restoration of an existing local temporarily
shadowed by a pattern. The bytecode route remains comptime-only without runtime
cleanup schedules; runtime origin/rank responsibility stays in its own matcher.
Instructions, rollback state and export registers appear in S-expression dumps.

Structural fields still use atomic structural matching. Computation nested inside
those fields remains a whole-body fallback before effects. The staged fallback
regression now uses a Tuple field containing Then, since Observe is supported.
Deep Or chains use the metered worklist; deeply owned structural patterns keep
the existing representation-depth fallback. No semantic acceptance was weakened.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-2h_a4nly/result.json`. Earlier compiler assertions
  describing Binding as an owned leaf and Or as a recursive carrier were updated
  alongside independent structural depth and fuel checks.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::pattern_tests`:
  exit 0, `/tmp/luna-tools-k9965asv/result.json`; four tests compare tree and VM
  rollback, effects, simultaneous remaps, illegal exports, aliases and abnormal
  exits, including recovery in the same interpreter.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_patterns --test comptime_bytecode_defaults --test comptime_bytecode_types --test negated_nominal_patterns`:
  exit 0, `/tmp/luna-tools-i7_qmwwp/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_patterns --test comptime_bytecode_bindings --test comptime_bytecode_matching --test comptime_bytecode_closures`:
  exit 0, `/tmp/luna-tools-g6kuzduw/result.json`; includes the explicit outer-guard
  no-retry regression and checks actual compiled entry counts.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-93wi7eyn/result.json`; `git diff --check`: exit 0.

### Compile computations inside value-structure patterns

Value tuple, struct, variant, anonymous-record, optional-present and error-result
patterns now lower to shape admission followed by ordered field observations and
child code. NIR carries a shallow structure descriptor, with a borrowed view for
the tree matcher and a metered owned copy for bytecode. Both engines use the same
shape/identity/arity checks and per-field observation routines. Anonymous-record
labels are checked when their field is reached: failure of an earlier field
skips later label validation as well as later computation.

Projected pattern registers retain only a parent register and field index. The
VM follows these read-only paths with metered traversal against the current
activation's subject. It does not clone every aggregate on the path; wildcard
fields have no value destination at all. Actual pattern bindings retain the
existing value-copy semantics. Projections never enter immutable code caches.
S-expression dumps include shallow shapes, escaped names and projection links.
No benchmark speedup is claimed by this structural change.

TypeTuple, nominal Type and normal-form argument patterns keep their existing
parameter-domain conversion and provenance checks. Computations nested in those
type patterns still select the tree path before execution. The depth/fallback
regressions now exercise TypeTuple; deep value tuple and Or patterns are linear.
This is value decomposition, not a special Tuple type-identity or NF rule.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p nir -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-82p3iqqh/result.json`; includes ordered malformed-label
  rejection, prior-effect preservation, shape dumps and independent fallback.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_patterns --test comptime_bytecode_matching --test comptime_bytecode_qualified --test comptime_bytecode_types --test library_comptime_comparison`:
  exit 0, `/tmp/luna-tools-ju1tv758/result.json`. An initial source test omitted
  whitespace between the success-pattern suffix and arrow, lexing as !=; the
  fixture spelling was corrected without changing parsing rules.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_patterns --test negated_nominal_patterns --test normal_patterns`:
  exit 0, `/tmp/luna-tools-loczbr7b/result.json`; includes nested alternatives,
  cross-field bindings, all supported value shapes and compiled entry counts.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-0flzwp6v/result.json`; `git diff --check`: exit 0.

### Compile computations inside Type patterns

TypeTuple, nominal Type and retained normal-form patterns now lower to explicit
admission, ordered field conversion and completion instructions. Child patterns
use the ordinary expression worklist, so calls, alternatives and early returns
inside a Type pattern no longer force whole-body tree fallback. The two engines
share admission and conversion routines. Nominal domains use the completed
parameter telescope; NF domains use the preceding input prefix, preserving head
identity without inspecting the head's body. Prefix arguments are borrowed,
not cloned into a growing temporary vector. Domain substitution is budgeted.

Active conversion state belongs to the invocation frame and candidate scope.
Rollback removes that state and restores bindings; already executed assignments
remain visible. Completion releases conversion state, and code caches retain
neither subjects nor environments. State and instructions have S-expression
Display implementations. Wildcards retain each existing domain-validation rule.
Generated struct/enum metadata uses its shared owner while a domain is converted,
so runtime-generated definitions do not require a borrowed package lifetime.

CodeEvidence is still definition-only and cannot execute. Its fallback regression
checks that the earlier operand effect occurs exactly once before rejection.
Deep TypeTuple lowering now uses the metered worklist instead of owned recursive
pattern copying. This extends the existing checked Type views; it does not add
library-name matching or change Tuple/NF identity. No benchmark speedup is claimed.

Source coverage uses a stable Type equality guard before a dependent NF value
pattern. An unconstrained Type binder alone does not establish that domain in the
current elaborator; contextual pattern elaboration remains a separate gap. The
nominal Type decomposition is internal NIR and is tested there, not by inventing
nominal source pattern syntax. The source test compares tree and VM results and
requires both the containing function and nested call to execute as bytecode.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p nir -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-_qalynql/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bytecode::pattern_tests`:
  exit 0, `/tmp/luna-tools-ygqz1haj/result.json`; includes dependent nominal
  domains and TypeTuple failure/early-return recovery in tree and VM.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_patterns --test normal_patterns --test comptime_bytecode_types --test negated_nominal_patterns --test library_comptime_comparison`:
  exit 0, `/tmp/luna-tools-7q880ugo/result.json`. Initial source fixtures exposed
  the contextual-domain limitation and incorrectly used NF syntax for a nominal
  head; corrected coverage uses supported source evidence plus direct NIR tests.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-c5tvyian/result.json`; `git diff --check`: exit 0.

### Compile checked structure observations

Struct field names/types and all five enum structure queries now use the shared
observation admission/value path. The compiler emits admission, the subject's
ordinary expression code, and one observation. Subject calls therefore suspend
and resume on the invocation stack instead of forcing the containing body to the
tree evaluator. Queries preserve declaration order, applied parameter domains,
opaque field Types and the existing live generated-definition world.

Admission still checks comptime phase and the exact nested result-list domains
before subject evaluation. Never operands preserve early return. Reflection and
immutable result admission retain their budgets, formation constraints, selected
associated-type checks and diagnostics. No reflected result or declaration-world
snapshot is embedded in cached code; this is not layout or access authorization.
The new observation kinds have S-expression dumps through the existing adapter.

The compiler publisher and BuildStruct/BuildEnum operations themselves still use
the tree path. This stage accelerates their reflection inputs and consumers; it
does not claim that the whole definition-builder pipeline is compiled.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-u_uvlsl7/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib reflection`:
  exit 0, `/tmp/luna-tools-gqsu0pxf/result.json`; struct-node domain rejection,
  budget exhaustion and recovery exercise both tree and bytecode execution.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test source_reflection --test comptime_bytecode_observations`:
  exit 0, `/tmp/luna-tools-1khnt0sh/result.json`; all seven structure queries
  and subject calls are counted as compiled invocations, including early return
  and recovery. The outdated manual package fixture omitted ordinary bool/attrs
  dependencies; it now uses the actual sysroot with an explicit target.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test builder_invocations enum_reflection`:
  exit 0, `/tmp/luna-tools-2etenclp/result.json`; live generated enum observation,
  subject effects, phase rejection and early return.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-ug11qhkt/result.json`; `git diff --check`: exit 0.

### Compile checked struct and enum publication

BuildStruct and BuildEnum now lower to explicit definition-build admission and
publication instructions. The packet is ordinary expression code: nested calls
resume on the invocation stack, and early return leaves before any publisher
request. Tree execution shares the same admission and value-publication helpers.
The descriptors carry only the construction kind and result Type, with stable
S-expression dumps; no request values, publisher closures, prepared candidates or
published identities enter the immutable code cache.

This preserves the existing two-step host publication contract. Request decoding,
duplicate/path/shape/encoding checks and Type admission precede host preparation.
The host receives reserved remaining work and cannot refund it. Returned metadata
must match the request before installation/commit; rejected descriptors, failed
commits and abandoned candidates do not publish a partial definition. Symbol and
fixed-width enum encodings retain their distinct validation and identities.

The host publisher remains a synchronous compiler service using ordinary checked
elaboration; it is not arbitrary source evaluation inside a VM instruction.
This closes the previously recorded BuildStruct/BuildEnum tree fallback. It does
not implement new fn/trait/impl builders or claim a measured performance gain.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-2w8_rm9_/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib enum_build`:
  exit 0, `/tmp/luna-tools-f7fqa05y/result.json`; the existing 13 publisher-boundary
  tests now run with fresh fixtures in both tree and bytecode modes, including
  forged host metadata, commit failure, encoding checks and budget reservation.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test source_reflection --test builder_invocations`:
  exit 0, `/tmp/luna-tools-9kg0ed3a/result.json`; 73 builder and 14 reflection
  cases, covering live publication and downstream use of generated declarations.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test source_reflection source_struct_build_uses_checked_publication_without_replaying_argument_effects`:
  exit 0, `/tmp/luna-tools-rha8r3m6/result.json`; the strengthened input-packet
  test executes one nested Type-returning call, retains argument effect order,
  and counts every entered function as a compiled invocation.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-pmxwtz0u/result.json`; `git diff --check`: exit 0.
- `python3 -B development/library_check.py --stage sysroot-builders --stage sysroot-enum-builder --timeout 900`:
  exit 0, `/tmp/luna-library-t40jvoi5/summary.json`; all four struct and three
  enum native cases pass through the actual sysroot, including field transforms,
  reflection and batch composition of generated definitions.

### Compile source-owned build metadata reads

BuildMetadata now lowers to phase admission, ordinary selector expression code,
and one lookup/decode instruction. Nested selector calls run on the invocation
stack, while return exits before lookup. Both engines share selected-entry decode
and the exact owner/provider/alias diagnostics for unknown aliases versus known
dependencies with unavailable inputs. Malformed unselected values remain unread.

Compiled descriptors retain only the immutable snapshot already present in NIR;
alias strings and stored terms are charged before copying. They contain neither
selector results nor decoded metadata values. Saved/forwarded callables retain
their defining package's dependency mapping. Repeated calls still evaluate their
selector and decode the selected value. Dump output includes escaped aliases,
owner/provider identities and explicit unavailable entries.

The build-context fixtures now supply the real primitive package and import bool
as an ordinary alias, including resident CLI frames. build's TargetFeatures also
imports builtin.bool explicitly; no keyword fallback is introduced.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-mvzvejgy/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor metadata`:
  exit 0, `/tmp/luna-tools-ggsddpye/result.json`; includes descriptor copy budgets
  and dump escaping, selected-only decoding, distinct missing-input diagnostics,
  selector effects exactly once, phase rejection, early return and recovery.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test build_context`:
  exit 0, `/tmp/luna-tools-yrjsatif/result.json`; all 33 cases, including saved
  origin, resident invalidation/recovery, explicit compiled activation counts,
  and real-sysroot metadata/target execution at LLVM O0 and O3. Compiled entry
  counts include the anonymous saved-callable initializer, not just named events.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-6pxvedvj/result.json`; `git diff --check`: exit 0.

### Compile concrete dynamic BitVec construction

The existing BitVecType primitive now lowers to domain admission, ordinary width
expression code and checked Type construction. Width values stay ordinary Integer
values in VM registers; they are not packed into the structural Type operand
channel. Tree and bytecode paths share the original width conversion and budget
checks. Invalid input/result domains reject before operand effects; negative or
unrepresentable widths reject after once-only operand evaluation. Return skips
construction, and later invocations recover after errors or exhausted budgets.

This covers the existing concrete-width representation, including zero width.
It does not introduce symbolic widths, change normal-form identity, or replace
ordinary Tuple/List definitions with special type constructors. No result-cache
policy changes or measured speedup are claimed.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-udfav44t/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib bitvec_type_tests`:
  exit 0, `/tmp/luna-tools-nz3iv20a/result.json`; tree/VM effect ordering, width
  range, domain rejection, early return, budget exhaustion and recovery.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_types --test comptime_bytecode_bits`:
  exit 0, `/tmp/luna-tools-_l5n8o6e/result.json`; ordinary typealias application
  resumes the width callback, and both named calls are counted as compiled.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-a2flqn5n/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-meta-lists.fl --sysroot library --test-native`:
  exit 0, `/tmp/luna-tools-8r7dey4b/result.json`; the native collector case now
  includes mapping computed prefix offsets through the public BitVec binding.

### Compile exact integer conversions and representation reads

ComptimeIntegerConvert and IntegerRepresentation now use the shared scalar
instruction path. Their operand expressions execute on the VM invocation stack;
these primitives no longer force the enclosing function back to recursive tree
interpretation. Both carriers share signature admission and final value checks.
Phase and signature rejection precede operand effects; exact conversion checks
range after evaluating the operand once and never truncates overflowing values.
Representation reads mask using the selected target width, not host pointer size.
Early return bypasses conversion, and errors do not poison later calls.

Scalar dumps identify both operations explicitly. Code caching retains operation
descriptors only; invocation result caching is unchanged. This removes a fallback
path without claiming a measured speedup or completing other unsupported nodes.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-t_xsplgb/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib scalar_conversion`:
  exit 0, `/tmp/luna-tools-thndcwq2/result.json`; tree/VM admission, operand
  effects, overflow, missing target, phase rejection and recovery.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_integer_conversion`:
  exit 0, `/tmp/luna-tools-t9gfk9u0/result.json`; six source cases, including
  compiled activation counts, signed exact conversion, single evaluation, early
  return, runtime residual rejection and representation widths 8/32/64/128.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-b2o1lnqi/result.json`.
- `python3 -B development/library_check.py --stage hash-protocols --stage sysroot-hash --timeout 900`:
  exit 0, `/tmp/luna-library-8bvbgrsz/summary.json`; 13 Hash protocol cases
  and three real-sysroot native cases, including runtime/comptime agreement.
- `python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-meta-bindings.fl --sysroot library --build-target x86_64 linux 64 --invoke-comptime run`:
  exit 0 with `(bool true)`, `/tmp/luna-tools-ail7xc_t/result.json`; public
  meta.to_usize/from_usize compose with List mapping and checked struct building.
- `git diff --check`: exit 0. Existing unrelated unreachable-pattern warnings
  remain; no full-workspace test claim is made.

### Compile checked newtype payload transfers

NewtypeConvert now emits a preparation instruction followed by ordinary operand
code and a completion instruction. Preparation checks the direct representation
edge in the current declaration world and charges the existing plan budget before
operand effects. The invocation frame owns that plan; cached code holds only
bounded copies of the input/output types. Completion moves the whole payload,
checks its input/output domains and exact wrapper identity, and consumes the plan.
No field view, transitive cast, inherited method or resource capability is added.
Tree interpretation and bytecode share both checks, including IIR's instantiated
representation lookup. S-expression dumps expose the preparation and transfer.

The integration cases exposed two frontend defects, fixed in the same path:
- A generic comptime call used as a field receiver must establish its result
  domain without capturing unavailable invocation-time locals. Speculative
  expression checking now instantiates that domain in the current phase; the
  eventual expression still performs the producer's work.
- An explicit method receiver cast selects a trait view only when its target
  resolves to a trait declaration. A Type-producing binding such as meta.List
  instead yields an ordinary converted receiver. Both ordinary and CMTT method
  selection use this distinction, without a library-name exception.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-m0vj9spr/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib newtype_vm`:
  exit 0, `/tmp/luna-tools-p_98d8we/result.json`; tree/VM live-edge revalidation,
  pre-operand rejection, forged identity/payload rejection after effects,
  recovery and dump operation names.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test newtypes`:
  exit 0, `/tmp/luna-tools-1ha5i__z/result.json`; 212 cases. The new regression
  counts every compiled helper activation through nested generic wrappers and
  struct payload projection, and verifies early return skips the helper call.
- `python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-meta-bindings.fl --sysroot library --build-target x86_64 linux 64 --invoke-comptime run`:
  exit 0 with `(bool true)`, `/tmp/luna-tools-pab55u8t/result.json`. The actual
  sysroot List is wrapped, passed through a comptime function, unwrapped and
  mapped before composing with integer conversion and StructBuilder reflection.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-aoaxd_hb/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test newtypes --test comptime_receivers --test source_reflection --test comptime_list_operations --test cmtt_methods`:
  `/tmp/luna-tools-4xycfeck/result.json`; CMTT methods passed all 144 cases,
  then List tests exposed a fixture missing the ordinary BitVec binding used by
  its shared example (57 passed, one failed before elaboration). Added the same
  BitVec alias as builtin.primitives; no compiler fallback or expectation changed.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_receivers --test source_reflection --test newtypes --test comptime_list_operations`:
  exit 0, `/tmp/luna-tools-0wc4twn_/result.json`; 58 List, nine receiver,
  212 newtype and 14 reflection cases passed after the fixture correction.
- Final formatting: `/tmp/luna-tools-vo65d5vu/result.json`, exit 0. Existing
  unrelated unreachable-pattern warnings remain; no full-workspace or measured
  performance claim is made.

### Ordered Object interval editing

ObjectEdit now includes Slice, with two exact Integer endpoints in the ordinary
operand packet. The shared tree/VM operation validates both endpoint domains
before evaluating the packet, then checks 0 <= start <= end <= entry count.
Selection moves complete typed entries, retaining positional entries, duplicate
keys and payload identity. Work is charged against the existing invocation budget;
this remains a functional value operation, not a writable or zero-copy view.

meta.Object.slice exposes that checked primitive through a bound compiler symbol.
meta.Object.replace_range composes slices and concat, supporting insertion and
removal without changing either input snapshot. Bounds failure is an invocation
error, consistent with existing single-entry Object edits. Evaluation order and
early returns follow normal call rules. No new opcode interpreter or result-cache
policy is introduced; ObjectEdit's existing S-expression Display names Slice.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-qpoegwbv/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib object_slice`:
  exit 0, `/tmp/luna-tools-315vwln_/result.json`; both carriers check malformed
  domains before effects, bad bounds/values after effects, and recovery without
  mutation of the input snapshot.
- `python3 -B development/check.py --timeout 900 -- test -p nir --test phases --test type_rewrite --test copy_budget`:
  exit 0, `/tmp/luna-tools-98dqufh_/result.json`; Slice is included in operation
  enumeration for phase checks, bounded copying, rewriting and Display.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects`:
  `/tmp/luna-tools-ltlm_i7_/result.json`; 27 cases passed, one new evaluation-order
  fixture failed parsing because its bare trailing Object was parsed as a block.
  Corrected it to bind the Object before returning it, without changing the case.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects object_range_arguments`:
  exit 0, `/tmp/luna-tools-b0yo1llg/result.json`; the corrected case checks receiver,
  endpoints and replacement order (1234), single evaluation, and early return.
  The preceding passing range cases include negative/reversed/oversized bounds,
  recovery, declared domains, duplicate-key shadowing, optional layers, retained
  normal-form heads and exact compiled activation counts for both public methods.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-q43t6gwd/result.json`; `git diff --check`: exit 0.
- `python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-meta-object-methods.fl --sysroot library --test-native`:
  exit 0, `/tmp/luna-tools-y5hzmze3/result.json`; the public methods execute in
  the actual sysroot, and their comptime observations stage into a passing native
  test. Existing unrelated compiler warnings remain; no full-workspace claim.

### Borrow Object intervals without materializing discarded entries

The local Object instruction now handles queries and Slice with up to two leaf
selectors. It retains complete signature/representation validation. Only owned
scalar or literal selectors may be read while the receiver is borrowed; selectors
that can reference the same storage retain the snapshot fallback and release the
receiver lock before reading. Computed selectors remain ordinary packet code, so
user effects never execute under a receiver lock. Runtime selector transport uses
a fixed two-slot array, without adding per-query vector allocations.

Slice copies only selected entries after the full input check. Selected payloads
use the existing checkpoint rules, preserving nested identity and scope checks.
Even an empty slice rejects invalid unselected Type payloads. The public Object
method invokes its bound primitive directly, avoiding an extra by-value forwarding
wrapper; no definition-name exception, result caching or view semantics is added.

Validation in /home/fin/luna:
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor`:
  exit 0, `/tmp/luna-tools-n07fxdvc/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_compiler -p comptime_interpretor object`:
  exit 0, `/tmp/luna-tools-fue7f39e/result.json`; includes compiler selection of
  leaf versus computed boundaries, dump shape, Object validation and edits.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects --test comptime_bytecode_aggregates`:
  exit 0, `/tmp/luna-tools-acz6aj3w/result.json`; public range cases now count
  two compiled activations for slice and ten for composed replacement, reflecting
  removal of the forwarding activation. Query side effects retain snapshots.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib local_object`:
  exit 0, `/tmp/luna-tools-udtbtrk2/result.json`; shared-storage bound references,
  locked receiver rejection, bounds/budget failure recovery, and invalid
  unselected Type rejection on both tree and compiled carriers.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0, `/tmp/luna-tools-bz9wkatx/result.json`; `git diff --check`: exit 0.

Measurement command:
`python3 -B development/comptime_bench.py --bench object_queries --repeat 3 --iterations 10`.
Release build: `/tmp/luna-tools-hyup91ux/result.json`; measurement reports:
`/tmp/luna-tools-s0snizif/result.json`, `/tmp/luna-tools-q8ky4a90/result.json`,
`/tmp/luna-tools-avk_apr3/result.json`, all exit 0.

The added Slice workload is printed as query=None. It performs 32 one-entry
slices per invocation, with five warmups and ten timed invocations per process.
Below are medians across three processes for a reference receiver. Both paths
use the same build; a transparent receiver block forces the general snapshot
path and adds evaluation ticks. This is a path comparison, not a historical
baseline. Parsing, elaboration and setup are excluded; invocation input copying,
execution, result checking and allocator instrumentation are included.

| Input entries | Borrowed ms | Snapshot ms | Borrowed allocations | Snapshot allocations | Borrowed bytes | Snapshot bytes |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 0.007824 | 0.010876 | 118.1 | 182.1 | 19719 | 58375 |
| 64 | 0.051737 | 0.114596 | 181.1 | 2261.1 | 34326 | 540406 |
| 1024 | 0.726964 | 1.724899 | 1141.1 | 33941.1 | 257994 | 7921450 |

For 1024 entries, the measured path takes about 42% of snapshot time and requests
about 3.4% of its allocations. These are allocator calls and requested bytes, not
retained memory. VM steps remain close: 452208 versus 452400. Full representation
checking is still linear in input size on every read; this work does not fix
quadratic repeated List admission or imply a whole-compiler speedup.

Final sysroot check:
`python3 -B development/check.py --timeout 900 -- run -p luna -- development/examples/sysroot-meta-object-methods.fl --sysroot library --test-native`:
exit 0, `/tmp/luna-tools-stqodfdf/result.json`; public Object edits still stage
into a passing native test. Existing unrelated compiler warnings remain.


## Invocation depth independent of native recursion (2026-10-30)

Compiled invocation frames now count against the caller's original `Budget.depth`
without retaining a native evaluation depth charge while suspended on the heap.
Entry and preparation still carry the native charge; tree fallback retains it
until delivery. Both counters unwind on preparation, execution and delivery
errors. Nested compiled calls no longer look like top-level invocations when
resetting cleanup failure and pending effect transfer state.

This does not increase the native evaluator or recursive representation limits.
The migration tree evaluator remains bounded independently of a large requested
invocation budget. IIR construction initializes the same invocation accounting;
it retains its existing native depth policy and execution path.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode --test comptime_bytecode_defaults`:
  passed, `/tmp/luna-tools-k_2530iq/result.json`. Includes a 2 MiB thread running
  1,001 simultaneous compiled activations, budget exhaustion and reuse of the
  same VM; tree execution rejects deep recursion and remains reusable.
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  308 passed, `/tmp/luna-tools-7hgg145u/result.json`, including caller state
  restoration and execution failure paths.

Remaining limits include recursive value/type inspection and native execution of
unsupported bodies and contract boundaries. This change does not make every
comptime operation iterative or remove invocation and instruction budgets.

The final boundary assertion (1,024 active calls allowed, 1,025 rejected) passed
with `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode heap_activations_honor_requested_depth_and_recover_on_small_host_stack`:
`/tmp/luna-tools-14aqcsm5/result.json`, exit 0. Formatting and `git diff --check`
also passed.

## Ordinary List carriers for Type elements (2026-10-30)

The VM now admits `Value::List { ty, values: Vec<Value::Type> }` against the
ordinary bound List<Type> domain. The declared list identity and each Type value
are checked using the same List admission and element traversal as other domains.
Length/index observations, concatenation, receiver append and normal-input
spreads accept this carrier. Mixed compact/ordinary append preserves the target
carrier and restores its original contents if the final check fails.

Struct and enum construction decode either carrier at the checked host boundary;
malformed element domains still fail before commit. This is preparatory consumer
migration: source Type-list literals and reflection still commonly produce
`Value::TypeList`, and the legacy enum variant has not been removed.

Freezing an ordinary Type-element list deliberately uses the existing canonical
`Term::TypeListValue` encoding. It must not introduce a second identity for the
same normal-form argument. The generic frozen NominalValue decoder continues to
reject a second Type-list encoding. Moving all producers and subsequently the
frozen representation requires a coordinated change to the remaining NIR
consumers, equality and tuple layout views; this stage does not claim that work
is complete.

Checks from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`:
  310 passed, `/tmp/luna-tools-l153uznw/result.json`. Includes both builder
  execution paths, host-metadata rejection before commit, malformed element
  recovery and mixed-carrier append rollback.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_list_type_evidence ordinary_type_lists`:
  passed, `/tmp/luna-tools-frkdrj2c/result.json`. Exercises actual ordinary-list
  inputs through public List methods and equal NF identities for named/spread
  inputs in both representations, followed by malformed-input recovery.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_list_type_evidence --test comptime_list_operations --test comptime_bytecode_lists`:
  all three groups passed, `/tmp/luna-tools-l4icv3f8/result.json`, exit 0.
- Formatting check and `git diff --check` passed.
