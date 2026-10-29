# Borrowed comptime Object queries

The VM can observe an owned local Object without allocating a discarded whole
Object snapshot. Length, positional key/type/value, and named-value queries use
this path when the selector is a literal or local read. Length also supports
storage and comptime value references. No result-cache admission is changed.

The borrowed representation walk retains the previous validation, depth checks
and fuel charges. Returned values still receive their own snapshots. Effectful
selectors, computed subjects, cleanup schedules, and reference payload queries
retain ordinary packet evaluation. In particular, no user code or payload read
runs while a reference's storage lock is held. This is an allocation reduction;
validation still visits all entries and is not constant-time Object lookup.

The differential tests compare the two paths for all five queries across fuel
and depth limits, including failure and recovery. They also cover local selector
bounds, missing bindings, wrong domains, locked storage, and reference reads.
Existing source tests cover argument order, early return, nested optionals,
normal-form Type identity, and malformed or hidden entries.

## Reproduce the measurements

From `/home/fin/luna`:

```sh
python3 -B development/comptime_bench.py --bench object_queries --repeat 3 --iterations 30
```

The benchmark constructs NIR before timing, invokes the VM directly, and counts
allocations. Each invocation performs 32 queries. Input cloning and invocation
admission are included equally in both paths. The general path wraps the Object
local in a transparent block, forcing snapshot evaluation; the block adds two
fuel ticks per query. This is a comparison of two current evaluation paths,
not a historical-version or whole-compiler benchmark. Timing is not a test gate.

Three-process medians on 2026-10-29, with 1024 Boolean properties:

| Query | Path | Time/invocation | Allocations/invocation | Allocated bytes/invocation |
| --- | --- | ---: | ---: | ---: |
| Length | General | 1.659 ms | 33926.1 | 7893407.3 |
| Length | Borrowed | 0.693 ms | 1062.1 | 239711.3 |
| Value at index 0 | General | 1.682 ms | 33991.1 | 7900631.3 |
| Value at index 0 | Borrowed | 0.698 ms | 1127.1 | 246935.3 |

Raw reports: `/tmp/luna-tools-whehjvsm/result.json`,
`/tmp/luna-tools-rfehqna4/result.json`, `/tmp/luna-tools-ftwv8so1/result.json`.
The direct differential tests, unlike this block-based benchmark, execute the
same packet in both paths and require identical fuel consumption.

## Verification

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_meta_objects --test builder_invocations --test comptime_receivers --test compiler_operation_bindings
python3 -B development/host_examples.py object-queries --release-driver --compare-optimization
```

Both test commands exited 0: 201 VM unit tests and 110 source integration tests.
Reports: `/tmp/luna-tools-5ls3gonm/result.json` and
`/tmp/luna-tools-q0ecok0p/result.json`. Native O0/O3 executions both passed;
summary: `/tmp/luna-host-examples-rc3sjxqg/summary.json`.

This does not add field-default or method publication to source definition
builders. Those remain separate integration work.
