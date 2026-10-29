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
