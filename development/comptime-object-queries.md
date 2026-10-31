# Borrowed comptime Object queries

The VM can observe an owned local Object without allocating a discarded whole
Object snapshot. Length, positional key/type/value, and named-value queries use
this path when the selector is a literal or local read. Length also supports
storage and comptime value references. The later reference extension below also
covers positional and named queries. No result-cache admission is changed.

The borrowed representation walk retains the previous validation, depth checks
and fuel charges. Returned values still receive their own snapshots. Effectful
selectors, computed subjects and cleanup schedules retain ordinary packet
evaluation. Reference payload queries originally used that fallback too; the
extension below admits only selectors that cannot reacquire storage. This is an allocation reduction;
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

## Reference queries through the public Object methods

Object's five query methods now call the centralized named compiler operations
directly, keeping their typed signatures and comptime value-reference receivers.
The previous extra ordinary helper call copied the complete receiver before the
query. The helper functions remain available; direct raw builtin spelling stays
at the binding declaration.

The VM borrowed path now also handles key, EntryType, value and get on storage
or a comptime value reference. A literal selector or an owned Integer/Symbol
local cannot mutate or reacquire the locked receiver. Reference/storage selectors
retain the ordinary snapshot path: a selector may be a sibling field in the same
root, so evaluating it under the object lock would incorrectly fail. Computed
selectors still use the fallback, even if their apparent result is scalar.

The complete input representation is inspected before selection, with the same
fuel and depth charges as snapshot evaluation. Selected payloads still receive
independent snapshots. Optional layers, ordered duplicate properties, the last
named-entry rule and original Type identities are unchanged. No user code runs
under the query's storage lock; this remains an allocation optimization rather
than constant-time validation.

The differential fuel/depth/recovery matrix now runs all five operations against
owned Objects, storage and value references. A separate sibling-reference test
requires the fallback and successfully reads the selected payload. The benchmark
adds an ordinary comptime call receiving its Object by value reference, with the
same borrowed/general comparison inside that callee.

Reference-receiver measurements, using the same benchmark command above
(three processes, 30 measured invocations after five warmups per case), with
1024 Boolean properties and 32 reads:

| Query | Path | Time/invocation | Allocations/invocation | Allocated bytes/invocation |
| --- | --- | ---: | ---: | ---: |
| Length | General | 1.693 ms | 33935.1 | 7895316.7 |
| Length | Borrowed | 0.692 ms | 1071.1 | 241620.7 |
| Value at index 0 | General | 1.673 ms | 34001.1 | 7902596.7 |
| Value at index 0 | Borrowed | 0.691 ms | 1137.1 | 248900.7 |

Build report: `/tmp/luna-tools-lr19y3fd/result.json`. Measurement reports:
`/tmp/luna-tools-t7u3hnrp/result.json`, `/tmp/luna-tools-4vlkfky8/result.json`,
`/tmp/luna-tools-6403p2j8/result.json`; all exited 0. The value-reference caller
and its admission are inside the measured invocation. This still compares two
current paths, not historical compiler versions. The block control adds 64
steps per invocation; direct differential tests require identical fuel.
No invocation result cache participates.

Final validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib
python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_meta_objects --test compiler_operation_bindings --test comptime_object_storage
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-meta-object-methods.fl --sysroot library --build-target x86_64 linux 64 --invoke-comptime run
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-meta-object-methods.fl --sysroot library --test-native
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All exited 0: 202 VM tests (`/tmp/luna-tools-c2dt8mcx/result.json`), 25 Object,
five operation-binding and four Object storage tests
(`/tmp/luna-tools-in3cf2q2/result.json`). The real sysroot invocation returned
`(bool true)` (`/tmp/luna-tools-_7wba7_t/result.json`); its added staged native
test passed (`/tmp/luna-tools-pk9j8wdq/result.json`). Formatting passed at
`/tmp/luna-tools-yqcdgcv7/result.json`, and diff checks passed. The existing
computed-selector fallback test now also verifies that a plain reference payload
query succeeds on the borrowed path. No ownership or storage permission was
added by this optimization.

## Fallible ordered Object traversal

The public inherent Object methods now include `try_foreach`, `try_fold`, and
`try_flat_map`. They use ordinary comptime closures and native error-qualified
results, without new compiler operations or extension traits. Each traversal
captures the input snapshot once, visits one-entry Objects in order, stops at
the first language error and preserves its payload. Prior callback effects are
not rolled back. Partial fold/flatten results stay local and are not returned
as success. Empty input skips the callback; fold returns its seed and flatten
returns an empty Object. Flatten retains complete entries, including unnamed
entries, duplicate keys, optional payload layers and nominal Type identities.

`development/examples/sysroot-meta-object-fallible.fl` exercises the real
prelude and library package. It also checks retained closure state after failure
and that edits to a callback's entry do not overwrite the original Object.
Local rejection cases cover runtime use, wrong entry type, wrong accumulator
result type and a visitor returning a non-void success payload.

The existing attribute-export test incorrectly required positive per-member
`comptime_phase` attribute counts after the library's migration to
`pure comptime trait`. It now permits zero library phase attributes while still
requiring exactly one user attribute and accounting for every evaluated
attribute. The first run had 33 passes and this stale assertion failure;
`/tmp/luna-tools-nm0dp18f/result.json` preserves that result.

Validation from `/home/fin/luna`, final commands all exited 0:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_objects
python3 -B development/check.py --timeout 900 -- test -p luna --test sysroot_preludes shipped_object_fallible_traversal
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

Reports: `/tmp/luna-tools-x6xjt5k2/result.json` (34 Object tests),
`/tmp/luna-tools-9so3immj/result.json` (real sysroot invocation, one integration
test), and `/tmp/luna-tools-n__0g8sr/result.json` (format check). The sysroot
command includes explicit x86_64/Linux/64-bit target configuration, required by
the shipped package's integer runtime proxies. No full suite was run.
Concatenation retains its existing validation/copy costs; these methods make no
linear-time construction claim or transaction rollback promise.
