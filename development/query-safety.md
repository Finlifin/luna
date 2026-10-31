# Query engine shared-state safety

The query engine remains a serial evaluator. It is used by middle's provider
queries and the interface driver; this change does not introduce incremental
invalidation or a parallel compiler scheduler.

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
is deliberately serialized. Dependency edges, revision invalidation, cross-
query parallel scheduling and cross-thread wait-cycle detection remain future
work; no implementation of those capabilities is claimed here.

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
