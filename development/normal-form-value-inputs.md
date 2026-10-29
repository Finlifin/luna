# Normal-form value parameters

The author confirmed that `meta.List<Type>` is an ordinary comptime value
domain, including when used by Tuple. It must not introduce a special NF
parameter category.

This change removes `normal_forms::Kind::TypeList`. List parameters now use
the same `Kind::Value` path as other values: substitute earlier inputs into
the declared domain, then check the argument against that domain. A parameter
mentioning a domain no longer grants it a list representation. Interpreter,
IIR, and materialization consult the declaration's layout metadata instead.

Tests cover one dependent `List<T>` signature with Type and bool elements,
wrong elements and domains, missing declaration metadata, exhausted budgets
and subsequent successful checks. Existing tuple fixtures now supply the
list declaration they depend on rather than inventing authority at a parameter.

## Validation

Working directory: `/home/fin/luna`. Commands use the managed build cache.

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p iir -p program_verification -p type_resolution tuple --no-fail-fast
python3 -B development/check.py --timeout 900 -- test -p nir -p iir -p comptime_interpretor list --no-fail-fast
python3 -B development/check.py --timeout 900 -- test -p luna --test execution normal
python3 -B development/check.py --timeout 900 -- test -p luna --test library_protocols repository_meta_
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The first three test commands exited 0: respectively 24, 44, and 65 tests
passed. Output anchors are their `test result` lines; the dependent-domain
regression is `dependent_list_parameters_use_the_same_value_contract_for_types_and_bools`.
The repository meta tests also exited 0 with 5 passing tests; formatting and
diff whitespace checks passed.

An earlier broader run used `test -p nir -p iir -p type_resolution
-p comptime_interpretor -p materialization -p program_verification --no-fail-fast`
through the same wrapper. It exposed the outdated fixtures repaired above,
and an unmodified IIR test,
`staging::tests::nested_scalar_comparisons_and_blocks_reach_logical_depth_limits`,
aborted with a stack overflow. The targeted reruns do not exercise that test;
this work does not claim the broad suite passed or establish a baseline for it.

## Remaining work

The frozen `TypeListValue` representation and collection-specific expression
nodes still exist. Tuple's source-function registration and its requested
builtin callable constant binding also remain to be migrated. Removing the
parameter category does not complete those changes, nor establish unrestricted
comptime value support throughout the compiler.

## Normal-form callable type descriptors

`NormalFormFunction(signature, collector)` now represents the calling form
independently of ordinary `ComptimeFunction`. The inner signature owns the
dependent binder frame; the optional collector index carries no assumption
about Tuple, list element types, or a particular collection declaration.
Specializing earlier inputs renumbers the collector; supplying the collector
removes that slot. Rewriting, scoped validation, type matching, PTC identity,
associated-type normalization, budget walks, and dumps retain this wrapper.

Source annotations lower through the general callable telescope. The resulting
type can be returned as a comptime Type value and transported by metadata wire
tag 35. Runtime storage and quoted runtime contexts reject it. Acquiring and
applying callable values, and registering Tuple's builtin binding, are still
pending; a type descriptor alone does not implement those operations.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- check -p luna
python3 -B development/check.py --timeout 900 -- test -p nir -p ptc -p type_resolution --no-fail-fast
python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli --test metadata_wire normal_form_callable
python3 -B development/check.py --timeout 900 -- test -p nir -p type_resolution --test normal_form_callables --test matching normal_form_
```

All exited 0. The broad core test command passed 797 tests. After the final
phase-preservation and inference checks were added, the source/metadata group
passed 4 tests and the specialization/matching group passed 4 tests. These
include collector mismatch isolation, dependent input specialization, runtime
and quote-context rejection, and metadata round trips.

## Callable values and constant aliases

Source normal-form heads now yield ordinary function identities carrying their
normal-form callable types. Required comptime evaluation and dispatch preparation
carry declaration contracts without demanding or executing head bodies. Constant
aliases, including identities returned by comptime functions, apply the original
head with its formation constraints and argument collection. Metadata carries
the collector position independently of parameter domains. Runtime storage,
ordinary invocation, and forged callable annotations remain rejected.

This stage supports resolved constant aliases, not arbitrary local or dynamic
normal-form callees. Tuple's builtin constant registration remains pending.

Validation from `/home/fin/luna`, all commands below exited 0:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli normal_form_callable
python3 -B development/check.py --timeout 900 -- test -p nir --test normal_inputs --test phases --test normal_form_callables
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p materialization -p iir -p program_verification --lib normal_form
python3 -B development/check.py --timeout 900 -- check -p comptime_interpretor -p elaboration -p iir -p intrinsic -p program_verification -p type_resolution --all-targets
python3 -B development/command.py --timeout 120 -- cargo fmt --all
```

The source group passed 5 tests, NIR groups 60, and filtered library groups 3
(the IIR and materialization library filters selected no tests). Checks include
named versus collected alias inputs, dependent domains, metadata mismatch,
bodyless callable values, ordinary-call rejection, and recovery after rejection.
Reports: `/tmp/luna-tools-zib6o8jm`, `/tmp/luna-tools-wlnrir2_`,
`/tmp/luna-tools-8pn0qyof`, `/tmp/luna-tools-vve6zs9z`, and
`/tmp/luna-tools-xdfbk2kj` respectively.

An earlier multi-package `test ... normal_form --no-fail-fast` attempt including
all luna test binaries failed during linking with a disk quota error; logging
also hit quota (`/tmp/luna-tools-r398kete/result.json`). It is not a passing
test run. Validation was split into the explicit entries above.

## Callable snapshot transport

Invocation snapshots already encode named callables as `FunctionValue`; decoding
now accepts a normal-form callable domain too. The decoder checks the registered
head's full callable type, rejects applied arguments and unknown identities, and
does not require an executable body. The same path handles nested record fields.
Ordinary callable annotations do not admit a normal-form head.

Validation from `/home/fin/luna` (exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_form
python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli normal_form_callable
python3 -B development/command.py --timeout 120 -- cargo fmt --all
```

The VM filter passed 3 tests and the source filter 5. Reports are
`/tmp/luna-tools-poqj6dve`, `/tmp/luna-tools-rl3dhlso`, and
`/tmp/luna-tools-_zha49d3`. Snapshot checks cover record round trips, invalid
identity/domain/arguments, exhausted budget, and recovery after rejection.
Source coverage now also passes a head through an ordinary comptime parameter.

Dynamic local application remains a separate IR gap. A local `const` inside a
comptime body is evaluated by that invocation; evaluating its initializer during
elaboration would change phase, ordering, and effects. The next implementation
must retain the callee expression and form the opaque result at invocation time,
using its declaration contract. A lookup in the elaborator's constant map cannot
replace this operation.

```sh
python3 -B development/reproduce.py --timeout 60 -- development/repros/normal-form-local-application.fl --invoke-comptime Run
```

At the snapshot-transport stage, this reproduction exited 1 at `factory<bool>` with `explicit instantiation
requires a resolved generic function` (`/tmp/luna-tools-4d3xbza9`). It selects a
head using an invocation parameter and expects an opaque `Identity<bool>` result;
this is an unsupported positive case, not a language rejection rule.

## Dynamic application at invocation time

The recorded local application now succeeds. NIR retains the callee and its
source-ordered arguments using the existing type-directed `Call` shape. A
`NormalFormFunction` callee identifies NF application; Display emits
`normal-form-apply`. It is not an ordinary invocation of the head's body.
The phase verifier requires comptime execution, a Type result (or divergence),
and an explicit argument frame without executable defaults or receiver transport.

The interpreter evaluates the callee once, verifies its declaration and full
callable type, evaluates inputs once in source order, and forms the opaque type
through the same domain and formation-contract checks as static NF applications.
NF arguments are not frozen at an earlier staging boundary: invocation-local
values, including mutable cells, must be observed at their actual evaluation site.
Required type positions use this operation through required comptime evaluation.

Supported dynamic inputs include positional and dependent parameters, a final
positional collection, and a single whole-collection expansion. Tests use both
`List<Type>` and `List<bool>`, plus a `List<T>` domain dependent on a preceding
Type input. Named entries and mixed expansions for dynamic callees still need
the callable-entry metadata and routing work; no guessed label mapping was added.
Tuple's builtin constant registration remains pending.

Validation from `/home/fin/luna`, all exited 0:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli normal_form_
python3 -B development/check.py --timeout 900 -- test -p luna --test normal_form_methods --test normal_patterns --test type_lists
python3 -B development/check.py --timeout 900 -- test -p nir --test phases --test normal_inputs --test normal_form_callables
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib normal_form
python3 -B development/reproduce.py --timeout 60 -- development/repros/normal-form-local-application.fl --invoke-comptime Run
python3 -B development/command.py --timeout 120 -- cargo fmt --all
```

The groups passed 6, 74, 60, and 3 tests respectively (143 total). Reports:
`/tmp/luna-tools-d4dnpwcd`, `/tmp/luna-tools-41nv8b25`,
`/tmp/luna-tools-0tcg8viq`, `/tmp/luna-tools-qv0om_ff`,
`/tmp/luna-tools-yolizove`, and `/tmp/luna-tools-spdql79r`.
The reproduction returned the opaque Identity head applied to bool. Source
checks also cover two dynamically selected heads, callable parameters, computed
callees in type positions, once-only source ordering, and a selected head's
formation-contract rejection followed by a successful independent invocation.
