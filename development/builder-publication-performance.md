# Checked builder publication performance

The executable [builder-publication-bench.rs](../compiler/elaboration/examples/builder-publication-bench.rs)
measures concrete struct publication against completed NIR. It compares:

- `reuse`: one `CheckedStructPublisher`, retaining admission indices and committed
  finite-representation proofs across all requests.
- `rebuild`: a fresh publisher for each request, receiving a checked package that
  includes all earlier results. This is a controlled rebuilding strategy, **not**
  an execution of a historical compiler revision.

Both paths publish the same declaration identities, names, field order and field
Types. The executable checks their complete generated descriptors for equality
after every sample and independently compares each field against its request.
It also requires identical fuel consumption across repeated samples of a case.

Each source struct has four bool fields. Each generated struct has a bool, an
optional bool, a by-value source struct and a pointer to the previous generated
struct (or the first source struct for the first request). This exercises shared
representation proofs and access to previously published identities without
making a full-world control exceed the bounded recursive traversal depth.

Fixture/request construction and equality assertions are outside timing. Publisher
initialization, checking, commit, output descriptor collection and publisher
teardown are inside. Each strategy gets an unmeasured warmup; measured execution
order alternates. CSV output includes median, minimum, maximum and metered work.
For an even sample count the reported median is the upper middle sample.

Run from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- run --release -p elaboration --example builder-publication-bench
python3 -B development/check.py --timeout 900 -- run --release -p elaboration --example builder-publication-bench -- --baseline 1024 --samples 5 --counts 64,256,1024
```

Measurement on 2026-10-27, implementation baseline `41984025` plus the benchmark:
AMD Ryzen 9 9950X3D, x86_64 Linux, rustc `1.99.0-nightly (73dc9167f 2026-08-01)`,
release profile through the normal build wrapper. The process was not CPU-pinned
and the machine was not reserved for benchmarking. Each row uses five samples.

| Baseline types | Generated types | Reuse median (ms) | Rebuild median (ms) | Rebuild / reuse | Reuse steps | Rebuild steps |
|---:|---:|---:|---:|---:|---:|---:|
| 64 | 64 | 0.130 | 0.738 | 5.67× | 9,403 | 22,444 |
| 64 | 256 | 0.480 | 5.262 | 10.97× | 37,555 | 139,300 |
| 64 | 1,024 | 1.918 | 61.102 | 31.86× | 150,499 | 1,344,340 |
| 1,024 | 64 | 0.319 | 9.637 | 30.25× | 11,323 | 145,324 |
| 1,024 | 256 | 0.614 | 32.695 | 53.29× | 39,475 | 630,820 |
| 1,024 | 1,024 | 2.078 | 181.166 | 87.18× | 152,419 | 3,310,420 |

All descriptor and work-repeatability assertions passed. Evidence:

- Default 64-type baseline: exit 0, `/tmp/luna-tools-y907t7k0/result.json`.
- 1,024-type baseline: exit 0, `/tmp/luna-tools-rvj453xz/result.json`.
- Minimum-case invocation (`--baseline 1 --samples 1 --counts 1,2`): exit 0,
  `/tmp/luna-tools-4k6z4us8/result.json`.
- Targeted rustfmt and `git diff --check`: exit 0.

The measured reuse path grows approximately with the number of new declarations
in this workload. Increasing the baseline from 64 to 1,024 types adds exactly
1,920 metered steps to each reuse case, reflecting one-time setup rather than a
per-request baseline traversal. The rebuilding control scales much worse as the
checked package grows.

These numbers do not measure parsing, source elaboration, full comptime function
execution, LLVM lowering or whole-program build time. They also do not measure
peak memory, generic declaration generation or new function/implementation
publication. Do not quote the ratios as an overall compiler speedup. The result
supports retaining the current per-publisher caches; subsequent builder work
should extend definition coverage rather than assume this benchmark establishes
the performance of those unimplemented paths.
