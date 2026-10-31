# Comptime code sessions

`comptime_interpretor::CodeSession` owns an `Arc<Package>` and the bounded
bytecode, input-independent body and default-specialization caches for that
exact immutable source allocation. `luna::memoization::Invocation` retains one
session across evaluations, including calls whose result is not `Type` and
therefore bypass result memoization entirely.

`Interpreter::with_code_session` lends those caches to a fresh interpreter.
Only an identical source world can use them; equality of declaration IDs or
structural similarity of packages is insufficient. Owning the allocation also
prevents address reuse and makes `Arc::make_mut` produce a distinct snapshot.
A different package or an IIR interpreter uses its own local caches. The scope
returns completed code on success, error and unwind, but never publishes caches
from a replacement interpreter belonging to another world.

This retains code, not execution state: invocation frames, local storage, events,
proof callbacks, cleanup responsibilities and result values remain local. The
existing cache bounds and compilation-failure publication rules still apply.
Cache hits avoid compilation charges; world construction and actual execution
still consume each invocation's budget. No wall-clock speedup is claimed from
budget tests alone.

An explicit invocation with no declaration binders now retains the ordinary
function's stable identity after checking input arity. Actual generic functions
keep the specialization and contract path. Without this distinction the driver
would rebuild an owned body for every ordinary call and defeat session reuse.

## Verification

From `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p luna --test runtime_function_identities --test comptime_memory
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --test instance_budget --test nominal_types
```

The tests cover repeated execution with zero warm bytecode compilations,
changed bodies using the same declaration IDs, unwind recovery, and actual
driver calls with alternating boolean inputs. The latter must execute on every
call, retain zero cached results, and consume less fuel after code preparation.

The checks passed: 38 runtime identity tests and 18 driver memory tests
(`/tmp/luna-tools-gvuba7qo/result.json`). Additional `instance_budget` and
`nominal_types` checks passed 6 and 23 tests respectively using the same wrapper
(`/tmp/luna-tools-e1g6ol27/result.json`). Formatting and `git diff --check` passed.

## Remaining work

Trait evidence and layout tables are still rebuilt; source function, runtime-identity
and effect declaration indexes are shared.
This is not yet a
shared VM world, persistent disk cache, or a suspended elaboration session.
Input-dependent generated packages and IIR instance graphs need their own
explicit immutable lifetime and identity before their caches can be retained.
A generic body whose parameter telescope depends on ordinary invocation values
can still require a fresh per-call body after declaration specialization.

## Generic declaration specialization

The source session also retains closed, noncapturing declaration specializations
keyed by declaration identity and the complete ordered argument list. The cache
has at most 64 entries and a 100,000-work retention allowance; oversized entries
are not retained. Key construction and comparisons consume invocation fuel.
Eviction discards code handles and permits ordinary re-specialization.

Only completed substitutions formed outside active abstract type/value scopes
are admitted. Each hit still admits declaration inputs, checks current type
requirements and capabilities, and checks the expected callable signature.
Proof outcomes are never part of the reusable code. Capabilities now run before
both local and session cache hits; previously they only ran on misses.

The cached closure representation has an empty environment and capture list;
it is a prepared function body, not an invocation closure with captured storage.
This retained body identity allows the bytecode cache to survive VM teardown.
Independent ordinary input binding and execution still occur on every call.

Targeted checks:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --test instance_budget --test runtime_function_identities --test nominal_types
python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_memory
```

Generic session regressions cover distinct declaration inputs, cold/warm code
compilation, changed proof callbacks (Satisfied, Rejected and Unknown), recovery
after rejection, bounded eviction, and driver calls which bypass result caching.

The generic stage passed 8 instance-budget, 23 nominal-type and 38 runtime
identity tests (`/tmp/luna-tools-pcspcsx8/result.json`), plus 19 driver memory
tests (`/tmp/luna-tools-xawxep4u/result.json`). The affected dynamic-instance
unit group also passed:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib dynamic_instances
```

Evidence: `/tmp/luna-tools-_zbya17g/result.json`. No wall-clock benchmark or
whole-workspace test run was performed for this stage.

## Ordinary input binding on retained code

Input-independent preparation now also keys owned function bodies by live code
identity. Declaration-specialized functions and ordinary closures reuse their
prepared body even when they have ordinary parameters. Captured environments
and arguments are still bound independently on every invocation. Only immutable
code is retained; weak source identities avoid keeping discarded closure bodies
alive or confusing reused allocation addresses with old code.

This uses the same contextual-type scan and specialization checks as declaration
bodies. A body with types dependent on the current parameter telescope is marked
dependent and rewritten each time. Both owned and declaration entries share the
existing 100,000-work retention bound.

Verification commands:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib input_bodies
python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_memory --test comptime_bytecode_closures --test scoped_comptime_callables
python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_clause_inputs
```

The source regression invokes `keep(value:T)->T where T` through a code session
with alternating boolean values. Each invocation returns its own argument;
only the first compiles bytecode. Unit checks exercise changing dependent Type
inputs, distinct source bodies and release of the original source allocation.

This stage passed the input-body unit group (3 tests), bytecode closures (2),
driver memory (20), scoped callables (10), and declaration clause-input tests.
Evidence: `/tmp/luna-tools-g_87w65z/result.json`,
`/tmp/luna-tools-grpz6w2a/result.json`, and
`/tmp/luna-tools-v9f_3oxd/result.json`. Formatting and diff checks passed.

## Shared source declaration index

`Interpreter::for_comptime_in_session` reuses the source snapshot's function
index while constructing fresh VM state. The index owns declaration IDs and
positions only. Each VM binds those positions to its own immutable Package
slice and returns references with that slice's lifetime. No self-referential
VM/package allocation or unsafe lifetime extension is needed.

The session holds the Package allocation alive and shares the position map
through `Arc`. Only the exact source allocation can reuse it. Different source
snapshots use the ordinary constructor and recheck duplicate declarations;
constructor failure never replaces the session's existing index. The luna
invocation driver uses this constructor for its unchanged source snapshot.

Phase, formation and trait checks still run per construction, and their existing
world-work reservation remains in force. This stage removes repeated function
index allocation/building; it does not claim to eliminate declaration-world
construction. Trait and layout tables are still local. Runtime-identity and effect indexes
now use the shared typed declaration index described below.
IIR instance lookup retains its separate graph authority.

Validation commands:

```sh
python3 -B development/check.py --timeout 900 -- check -p luna
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib code_session
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p luna --test runtime_function_identities --test instance_budget --test comptime_memory --test comptime_iir_bytecode
```

The tests inspect actual index allocation sharing and fresh event state, retain
the index after exhaustion, reject duplicate IDs in another snapshot, and run
both source and concrete IIR invocation paths.

The source-index stage passed `check -p luna`, the code-session unit check and
all four listed integration groups. Evidence:
`/tmp/luna-tools-eqs77556/result.json`, `/tmp/luna-tools-ukcivhz2/result.json`,
and `/tmp/luna-tools-5pk4hmgb/result.json`. Formatting and diff checks passed.

## Runtime identity and effect declaration indexes

The position index is now generic over declaration type. Function bodies,
body-free runtime identities and effect declarations bind distinct typed slices
through one immutable source index bundle. A source session retains that bundle
once; warm VM construction shares its maps without rebuilding them or retaining
execution state.

Runtime identity signatures and collisions with executable function declarations
are still checked during construction. A cached body-free identity remains
transport metadata and cannot authorize comptime execution. Effect signatures
are also rechecked; sharing their lookup index does not install handlers or
satisfy an effect capability. IIR keeps separate authority and empty source
catalogs as before.

Verification commands:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib code_session
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p luna --test runtime_function_identities --test effect_contracts --test comptime_memory --test comptime_iir_bytecode
```

The unit test uses nonempty runtime-identity and effect catalogs, checks shared
index allocations and original declaration references, and verifies fresh
execution state. The warm-session integration test explicitly attempts to invoke
a body-free runtime identity and must still receive `PhaseMismatch`.

This stage passed the code-session unit check and all four integration groups
(71 tests total). Evidence: `/tmp/luna-tools-7d3tc8kq/result.json` and
`/tmp/luna-tools-3b8d28x6/result.json`. Formatting and diff checks passed.

## Declaration parameter and method metadata

The immutable source bundle also retains declaration parameter domains and
sorts, method parameter bindings, and method-origin ambiguity information.
These owned maps are shared through `Arc`. They contain source declaration
metadata, not active inference variables, invocation values or proof outcomes.

Domain collection and validation run on the first successful source-world
construction. Subsequent construction for the exact same immutable Package can
reuse that completed result, avoiding repeated recursive Term copies and their
associated work charges. A changed Package obtains its own domains. The source
world's ordinary traversal reservation and remaining formation checks still run.
The nonempty domain regression confirms a bool domain remains ComptimeValue,
a changed snapshot can use char instead, and warm construction saves fuel.

Method bindings and origin ambiguity depend only on the source traits. Selected
implementations, caller assumptions, associated values and instantiated method
contracts remain VM-local and are rebuilt/checked as before. Runtime method
interface bindings remain distinct from implementation-body parameter names.

Verification commands:

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib code_session
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p luna --test runtime_function_identities --test instance_budget --test comptime_memory --test comptime_clause_inputs --test comptime_bytecode_traits --test trait_phases
python3 -B development/check.py --timeout 900 -- test -p luna --test comptime_bytecode_traits
```

All groups passed (189 distinct tests). The final method test additionally runs
default parameters, named overrides, stored methods and generic trait calls
across fresh VMs in a shared session. Evidence:
`/tmp/luna-tools-4int1nsu/result.json`, `/tmp/luna-tools-2_19jpbf/result.json`,
and `/tmp/luna-tools-wtabqcrk/result.json`. Formatting and diff checks passed.
