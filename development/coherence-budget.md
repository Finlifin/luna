# Coherence budgets in a complete builtin package

Loading all builtin modules exposed budget exhaustion between independently
declared iterator adapters, and then between pointer and optional resource
implementations. Coherence previously divided solver work among every
same-interface/same-precision pair, including pairs whose rigid outer type
constructors already establish disjointness.

Index coherence now reuses PTC's constructor identities for constant-work outer
head separation. Reports expose `(distinct-type-heads)` and replay recomputes
the decision from the exact indexed pair. Same heads, Unit aliases, projections,
opaque computations and callable binders retain ordinary proof obligations.
This neither unfolds normal-form heads nor assumes constructor injectivity.

Only remaining solver obligations divide the group allowance. Both pair scans
are reserved before traversal; exhaustion still rejects, and no global budget
was raised. Pair report size remains quadratic. Report identity, revision,
coverage and ordering checks still prevent forged or stale acceptance.

## Validation

Working directory: `/home/fin/luna`.

```text
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test implementation --lib -p luna --test sysroot_preludes --test library_hash
```

Report `/tmp/luna-tools-cwhb0hzp/result.json`: elaboration library 283 passed,
implementation 74 passed, luna library 80 passed, library_hash 9 passed.
The overall command exited 101 because a new sysroot fixture reused a consumed
Once optional; that separate fixture was corrected without changing ownership.

The new coherence regression mixes 70 distinct nominal families with a genuine
same-family constrained pair. It checks adequate proof allowance, forged direct
assessments, low-budget failure, duplicate implementations and stale replay.
PTC unit tests compare direct separation with ordinary proofs and retain the
full-query boundary for shared heads and Unit aliases. Existing forged proof
tests now use a same-family constrained pair so they still exercise proof trees.
