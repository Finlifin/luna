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

## Session ownership and HIR publication

The D02/D10 query work still has only the package HIR query in production. Before
adding more cached compiler results, the input and result ownership boundary now
stops claiming lifetimes or thread safety that its fields do not provide.

`Db<'session>` retains `HirQueryInput<'session>` with a borrowed source map and
an owned `Arc<DiagnosticContext<'session>>`. Both source drivers pass these
dependencies directly. They no longer cast a diagnostic context to a raw
`DiagnosticContext<'static>` pointer, and lowering uses safe accessors. A clone
of the database shares the same invalidated input and cache; it cannot extend
the source map's lifetime. `Compiler` distinguishes its short borrow from the
session lifetime instead of relying on covariance through mutable query state.

`HirPackageBox::new` now accepts a higher-ranked arena-building callback. The
callback must return a package for any borrow lifetime of the owner's arena;
it cannot return a package borrowed from an unrelated, shorter-lived arena.
The arena is boxed and dropped after the package. Lifetime erasure remains
encapsulated inside this constructor, and the accessor narrows references
without a second transmute. The lowering provider no longer erases lifetimes.
Owned results can outlive the database and inputs, while borrowed HIR views
remain tied to their result owner.

Removed manual `Send`/`Sync` implementations from Db, HirQueryInput,
HirPackageBox and TyCtxt. Their natural field bounds determine which transfers
are valid: moving an unaliased HIR owner is distinct from sharing its mutable
arena through Arc. The query engine remains synchronized for data that actually
supports shared use; compiler arenas do not inherit that permission.

The unused Salsa storage/marker layer imposed the unsafe database bounds but
did not perform any production query. It is removed, including its dependency
closure. Existing dependency versions are unchanged; the lockfile removes 16
unused packages. Memoization, invalidation and cycle handling remain in the
existing query engine.

This does not add persistent queries, diagnostic replay, source-edit reuse or a
parallel compiler scheduler. Other arena/interner lifetime-erasure internals
are not certified by these changes. Language semantics and library interfaces
are unchanged.

Verification in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p middle -p query
  -p interface -p ast_lowering` exited 0
  (`/tmp/luna-tools-_l_w5o7n/result.json`): 21 unit/integration tests and five
  compile-fail doctests passed; five pre-existing doctests remain ignored.
  Checks cover foreign-arena rejection, input/session lifetime escape, actual
  thread bounds, results retained after input/database destruction, and the
  existing shared-engine invalidation and recovery behavior.
- Linux dependency metadata with `--offline --locked --filter-platform
  x86_64-unknown-linux-gnu` exited 0 (`/tmp/luna-tools-matgv_87/result.json`).
  The unfiltered metadata attempt requested an uncached Redox dependency
  (`/tmp/luna-tools-t96m936n/result.json`, exit 101); no dependency was downloaded
  and no surviving package version was upgraded.
- Serial driver regression command: `python3 -B development/check.py --timeout
  900 -- test -p luna --test modules --test packages --test frontend_cli --test
  sysroot_preludes --no-fail-fast -- --test-threads=1`
  (`/tmp/luna-tools-tc4yom99/result.json`, exit 101). All 56 module tests, 50
  package tests and 15 sysroot tests pass; frontend_cli has 229 passes and 23
  failures. The real sysroot tests cover public staged methods, text, hashing,
  object reflection, definition builders, qualified values and iteration.
  An earlier parallel run is recorded separately in
  `/tmp/luna-tools-th2m9zem/result.json`; it is not the serial result.
- Five frontend_cli failures are diagnostic/old-assertion failures. Repeating
  those exact tests on unmodified HEAD `40b7740b` reproduces all five
  (`/tmp/luna-tools-vhyab7rl/result.json`, exit 101). The other 18 serial
  failures hit that fixture's 15-second subprocess deadline. No expectation or
  deadline was changed to make the suite pass.
- All 18 timeout cases reproduce serially on the same unmodified HEAD
  (`/tmp/luna-tools-48t__pdp/result.json`, exit 101). The union of the two
  baseline failure sets exactly matches the current 23 failing tests. The
  baseline comparison temporarily restored only this change's files and then
  restored their current contents; unrelated worktree edits were preserved.
  This is a baseline comparison, not a claim that the full driver suite passes.
- Final `cargo fmt --all -- --check` through the command wrapper exited 0
  (`/tmp/luna-tools-07rapooj/result.json`), as did the middle/query doctests after
  updating API comments (`/tmp/luna-tools-i5dwuwgr/result.json`).
  `git diff --check` passed.
