# Property collection layout

Contract: `docs/functions/parameters.md`, especially the separation between
source producer evaluation and target packet layout. Property names select
field contexts; a call need not spell them in the target record's order.

The ordinary and partially inferred collection paths compute a permutation
from source fields to target slots. Retained peer contexts use the same name
mapping, including when a partial domain becomes fully known. This keeps
callback inference independent of argument spelling order.

Packet construction stages producers in source order before assembling the
record. Extended calls merge those producers back into their complete source
sequence, interleaving positional inputs and tuple collection. Only synthetic
packet staging is dismantled; nested producer computations remain intact.
Existing Block, CallPreparation, Let and AnonymousRecord NIR nodes and their displays
are reused. Global record type equality is unchanged.

A nested generic packet constructor can receive its target layout through the
expected return type. An already constructed independent record remains subject
to ordinary type equality when supplied as a whole parameter value.

The regression suite checks runtime and comptime results, exact invocation
order, early exits, heterogeneous field contexts, generic callbacks, variadic
and spread preparation, open/deferred forwarding and concrete IIR execution.
It also rejects missing, extra and duplicate properties. Derivation uses the
same path; its regression now deliberately reverses property spelling order.
The earlier standalone limitation in `repros/record-property-order.fl` is now
an acceptance regression.

## Validation

Working directory: `/home/fin/luna`.

The first check exposed fixture errors (assignment spacing and an unsupported
conversion) and missing alignment of retained generic contexts. After fixing
these, the next check passed 63 derivation tests and six property-layout tests;
one rejection fixture incorrectly expected an inline generic constructor to
ignore its expected type. The final fixture constructs an independent local
record before supplying it. A separate positive test preserves contextual
construction as supported behavior.

The open/deferred regression additionally exposed that packet-local staging
cannot use CallPreparation: its terminal node must be a call-like operation.
Packet-local staging now uses a normal Block; outer call preparation still
interleaves its producers. No phase validation rule is weakened.

Final commands (exit 0 unless explicitly stated otherwise):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test property_layout --test derivation
python3 -B development/reproduce.py --binary /tmp/luna-dev-002/target/release/luna -- development/repros/record-property-order.fl --emit nir
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Property layout: 9 passed; derivation: 63 passed.
  Report: `/tmp/luna-tools-os5xxml8/result.json`.
- Standalone reproduction now exits 0; report:
  `/tmp/luna-tools-3ssr5g9s/result.json`.
- Formatting: `/tmp/luna-tools-03xsb5ej/result.json`.

This change does not claim complete resource cleanup or custom collection
protocol support. It fixes ordinary property packet construction and inference
without changing the semantics of whole-record supply or type equality.

Existing property-related integration checks also exit 0:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test execution --test open_functions property_
```

Execution: 26 passed; open functions: 12 passed. Report:
`/tmp/luna-tools-q9cr2bd4/result.json`. This includes the old peer-order test
with its record-definition reordering workaround removed.
