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
