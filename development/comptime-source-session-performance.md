# Source VM session preparation measurements

The `source_session` benchmark compares paths in the same binary, rather than
claiming a historical revision or whole-compiler speedup. It constructs synthetic
NIR worlds outside the timed region: each world has 1, 32 or 256 functions, traits,
implementations and explicit dispatch instances. Function bodies contain either
zero or 64 local reads. The timed invocation calls the first function directly;
the fixture measures the cost of retaining a dispatch-rich declaration world,
not trait method execution throughput.

Five paths separate preparation from invocation:

- `cold-prepare`: validate and construct a fresh source interpreter each time.
- `warm-prepare`: construct fresh state from a successfully prepared session.
- `cold-call`: cold preparation plus input admission, compilation and execution.
- `warm-call`: fresh state with retained declaration and bytecode preparation.
- `resident-call`: execute repeatedly in an already prepared interpreter.

All paths include their local VM destruction where applicable. Fixture generation
and initial warmup are excluded. Every invocation checks its alternating boolean
input against the returned value. Result memoization is absent. Assertions also
check that only cold calls compile code in the timed region. A counting allocator
records allocation and reallocation calls, which adds instrumentation overhead;
resident calls retain their ordinary event trace. Allocation counts do not report
bytes or peak resident memory. Timing is diagnostic, never a correctness gate.

Run through the managed build/cache entry point:

```sh
python3 -B development/comptime_bench.py --bench source_session --iterations 100 --repeat 3
```

Each process executes three rounds and reverses mode order in the middle round.
The reported medians combine nine observations per workload from three processes.

## Measured results

Implementation: `f21901baa7e1db0c0ccd28c42cb6bda37a98d559`, plus this benchmark.
Host: AMD Ryzen 9 9950X3D; `rustc 1.99.0-nightly (73dc9167f 2026-08-01)`;
managed release profile. These measurements use the allocation instrumentation
above and do not isolate the host from other activity.

Median nanoseconds per operation, with 64 local reads per function:

| Functions / traits / impls | Cold prepare | Warm prepare | Cold call | Warm call | Resident call |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 each | 3,010 | 290 | 7,524 | 1,965 | 1,691 |
| 32 each | 81,874 | 290 | 86,748 | 2,025 | 1,678 |
| 256 each | 720,867 | 295 | 725,065 | 2,116 | 1,719 |

Warm preparation used zero allocations and one budget step at all six world/body
sizes. At 256 declarations per category and 64 reads, cold preparation used
15,149 allocation/reallocation calls; warm invocation used 10, versus about 9
for resident invocation. Only cold calls compiled bytecode, exactly once per
invocation; alternating input/result assertions passed in all paths.

The 256/64 fixture's roughly 2,444x preparation ratio is a same-binary cold/warm
comparison, not a before/after release result. A whole compilation still performs
parsing, elaboration, graph specialization, contracts, source result admission
and other work. The remaining warm/resident gap is small in this fixture, so it
alone does not justify another invocation micro-optimization. The persistent
PTC propagation trail and backtracking remain a separate architectural priority.

Raw evidence (all exit 0):

- Release build: `/tmp/luna-tools-mzqdutis/result.json`.
- Runs: `/tmp/luna-tools-k9zqob_0/result.json`,
  `/tmp/luna-tools-gcid7_hj/result.json`, `/tmp/luna-tools-hoqwx26b/result.json`.
  Each corresponding `stdout.log` contains all 90 measured rows.
- Formatting: `/tmp/luna-tools-sn0lpxzg/result.json`; `git diff --check` passed.

The initial fixture gave its comptime trait method an ordinary callable signature;
world admission rejected it with `InvalidCallPlan` before measurement
(`/tmp/luna-tools-sxtf6m_t/result.json`). The fixture now carries the comptime
callable phase in its signature. No interpreter check or semantic expectation was
changed. This commit adds a benchmark and measurement record, not VM behavior;
no whole-workspace test run was needed.
