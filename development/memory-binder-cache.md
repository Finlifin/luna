# Memory inference declaration-sort cache

Memory summaries instantiate generic function and closure signatures repeatedly
while discovering callees and revisiting their callers. Previously each binder
lookup rebuilt the package-wide declaration-domain map, including validation of
unrelated declaration metadata. This repeats immutable work inside one query.

`Round` now lazily retains the completed declaration-sort map for its immutable
package. Named bodies, provisional signatures, closure passes, and named callable
premises (including borrowed callable slots) share it. Empty binder lists do not
initialize it. Each lookup still charges for the requested binder vector. The
first scan retains all original validation and budget checks; an unsuccessful
scan leaves the cache absent. A new inference query constructs a new round.

This is metadata reuse, not a cache of provisional memory contracts. Type/value
sorts still come from declarations, and the fixed-point worklist, footprint
validation, recursive-instance identities, and rejection behavior are unchanged.
Standalone method-selection helpers outside a round retain their existing scan.

## Validation scope

The unit regression checks mixed type/value binders against the original helper,
then resolves ten subsequent vectors with exactly the vector-size budget. It
also checks insufficient lookup fuel, recovery after a failed initial scan,
fresh-query visibility of changed metadata, and duplicate-declaration rejection.
This demonstrates elimination of repeated metadata scans; it is not a wall-clock
benchmark or evidence that all resource-verification costs have been resolved.

The earlier error-result observer fixture still uses its explicit 1,000,000-unit
budget. No compiler default budget is increased by this change.

## Checks

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib -p luna --test library_protocols --test sysroot_preludes
python3 -B development/check.py --timeout 900 -- test --release -p luna --test method_memory_contracts
python3 -B development/check.py --timeout 900 -- test --release -p luna --test nominal_value_parameters member_functions_read_nested_record_value_parameters -- --exact
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The first command passed 284 elaboration unit tests, 80 luna unit tests,
25 library protocol tests (including interpreted/native observer cleanup), and
5 sysroot tests. Report: `/tmp/luna-tools-jr8mhvai/result.json`, exit 0.
The method suite passed all 9 cases, including recursive budget exhaustion and
recovery. Report: `/tmp/luna-tools-p6g_60ez/result.json`, exit 0.
Formatting and diff checks passed; formatting report:
`/tmp/luna-tools-cnhcz5ui/result.json`, exit 0.
The nested record value-parameter regression passed, including distinct result
origins, malformed argument rejection, low-budget failure and subsequent recovery.
Report: `/tmp/luna-tools-gujj5ai8/result.json`, exit 0.
