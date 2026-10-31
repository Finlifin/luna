# Query engine shared-state safety

The query engine remains a serial evaluator. It is used by middle's provider
queries and the interface driver. It now supports session-local dependency
invalidation, but does not introduce a parallel compiler scheduler.

Previously QueryEngine and QueryCache manually implemented Sync while storing
mutable state in RefCell. The single-threaded convention did not enforce the
shared-reference contract exposed through salsa's database. Cache and stack
state now use mutexes, and the unchecked Sync implementations are removed.
Auto traits enforce the stored types' actual thread-safety bounds.

An engine owns a reentrant evaluation gate. Competing threads wait for the
current complete evaluation; nested calls on its owning thread continue and
retain normal cycle detection. The gate holds no mutex during provider work
or hooks and releases ownership on unwinding. Stack inspection returns a
snapshot instead of a borrow that could outlive a synchronization boundary.

The start hook runs only after the cache slot and stack frame are established,
inside the guarded unwind path. Reentry observes the existing cycle; panic
poisons the slot and restores the stack. Cached reads and successful results
keep their existing memoization behavior.

Caches belong to one engine. Direct cache mutation is not an incremental
invalidation API and must not race with evaluation. Providers must not wait
for another thread to re-enter the same engine, because provider evaluation
is deliberately serialized. Persistent fingerprints, unchanged-result reuse,
cross-session storage, parallel scheduling and cross-thread wait-cycle detection
remain future work.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p query -p middle -p interface --no-fail-fast`:
  nine query tests passed; middle/interface compiled successfully (no unit
  tests, four pre-existing ignored doctests), exit 0;
  `/tmp/luna-tools-o3330h0o/result.json`.
- Tests cover memoization, recursive cycles, simultaneous same-key reads,
  a held provider with a waiting competitor, provider-panic ownership release,
  start-hook panic poisoning, and start-hook reentry.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test subcommands`:
  seven end-to-end CLI tests passed, exit 0;
  `/tmp/luna-tools-0zhafu7g/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  exit 0; `/tmp/luna-tools-j8ksqww5/result.json`; `git diff --check` passed.

No full language test suite, race detector or throughput benchmark was run.

## Observed dependencies and input replacement

Each typed cache slot has an identity independent of its diagnostic key text.
Every query read records an edge, including cache hits and failed reads.
Completing or unwinding an evaluation replaces its old observed read set;
obsolete branch dependencies no longer cause invalidation.

`QueryEngine::update_input` serializes input mutation with query evaluation and
marks the input query and its transitive consumers dirty. Mutation from an
active provider is rejected before executing the callback. Invalidation runs
both before and after the callback, including unwinding: an intermediate read
inside the callback cannot leave an obsolete result cached. Dirty poisoned
queries can retry after input replacement. Independent cache clones start cold;
shared results require sharing the cache and its owning engine.

`Db::set_hir_input` uses this boundary. Database clones share the input mutex,
engine and HIR memo table, so replacing input through either clone invalidates
both views and observed consumers. The old HIR result remains owned. This does
not resolve the database's other raw-pointer lifetimes or make all compiler
arenas safe for parallel use. The driver still constructs a new database per
compilation; persistent incremental compilation is not claimed.

`dependency_dump()` exposes a sorted S-expression with escaped diagnostic labels.
Its numeric identities are session-local tokens, not persistent fingerprints.
Low-level cache mutation bypasses dependency tracking; input writers must use
the engine API. The dependency graph currently lives for the engine lifetime.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p query -p middle -p interface --no-fail-fast`:
  16 tests passed, four existing doctests ignored, exit 0;
  `/tmp/luna-tools-xqlf02d5/result.json`.
  Covers cached and transitive reads, diagnostic-key collisions, branch edge
  replacement, panic recovery, reentrant mutation rejection, intermediate
  reads during mutation, cold clones and shared database input replacement.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test subcommands`:
  seven tests passed, including native build/execution and package checking,
  exit 0; `/tmp/luna-tools-3rizjmod/result.json`.

The database test uses a controlled HIR provider to test the input boundary;
it is not an end-to-end source-edit incremental compilation test. No full
language suite, race detector or performance benchmark was run for this stage.
