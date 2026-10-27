# Default derivation strategy selection

Contract: `docs/traits/derivation.md`, explicit/default selection and local
registration. This module is a semantic prerequisite; source-level default
selection and `attrs.derivation.default` evaluation are not connected yet.

## Admission

`derivation::Registry` stores declaration identities and complete trait argument
terms. The frontend must elaborate those terms and validate the declaration as
a strategy. Re-import of identical metadata is idempotent; conflicting metadata
for an existing identity fails atomically. Failed admission adds no bucket entry.

Registration ownership uses the strategy declaration's package, the trait head,
and trait arguments in declaration order. It does not inspect a future derived
subject. A local trait grants admission. An external trait needs a local nominal
identity before any open or opaque structural boundary. The traversal reuses the
existing ownership rules, without deriving ownership from a local function name
or from caller assumptions. Argument structures are charged before comparison,
including duplicate imports and local-trait registration.

## Selection

The frontend supplies the visible identity set and a candidate-local assessor.
The registry first filters by trait identity and visibility. The assessor must
settle the full requested interface instance, subject, ordinary strategy inputs
and requirements. A head match alone cannot report Applicable. Type obligations
use PTC; program predicates and generation computations stay with their own
checking/invocation systems. Pending means incomplete evidence, not rejection.

One applicable candidate is selectable only if no pending competitor remains.
Two applicable candidates are ambiguous even with further pending candidates.
No implementation precision, registration order or import order breaks ties.
Queries do not mutate the registry or cache partially assessed candidates;
budget failure permits a fresh retry after independent progress. The assessor
shares the caller's decreasing work budget. Stable S-expression dumps retain
candidate identities and distinguish missing, unique, pending and ambiguous.

## Frontend work remaining

Current elaboration expands derive requests before evaluating ordinary
attributes. Default registration requires completed, merged metadata and
candidate-local constraints first. The integration must preserve attribute
computation dependencies, resolver visibility, selected strategy provenance and
ordinary generated-implementation validation; it must not recognize a default
by path spelling or copy its registration marker onto the generated impl.

Generic strategy input environments, independent checking of unused strategies,
and dependency cycles remain separate unresolved frontend work. The registry
API does not claim to implement those capabilities or authorize generated code.

## Validation

Working directory `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test default_strategies
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Default strategies: 10 passed, exit 0;
  `/tmp/luna-tools-_7aefjg0/result.json`.
- Formatting: exit 0, `/tmp/luna-tools-x_r1k9ky/result.json`.
- Initial test compilation exposed a missing normal-form sort in the fixture
  and tests that referenced Display while the prior library build was still
  running. The final invocation rebuilt the completed source and passed.
- Tests validate this registry/selection contract. They do not claim source
  attribute registration, candidate constraint solving or generated code from
  an implicit default selection has been implemented.

Existing implementation ownership regression:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test implementation ownership
```

6 passed, exit 0; `/tmp/luna-tools-5mymo_lr/result.json`.

## Inspecting attributes before expansion

The attribute-only frontend now retains unexpanded strategy and derive-request
identities. It evaluates metadata through the ordinary comptime path before
strategy selection, then returns only the inspected attributes. The executable
compiler still expands derivation and cannot obtain an executable package from
this inspection path. Existing inspection for programs without derivation keeps
its previous flow.

This makes metadata on an unused strategy available without executing its body,
and one strategy's metadata is not multiplied by its explicit use sites. Raw
and merged inspection preserve original target identities and prefix merge
order. Required attribute helpers still discharge their own invocation contracts.
The result is not evidence that strategy bodies, defaults, or generated impls
have been validated. Generic/runtime-dependent attributes and attributes that
require a generated impl are not solved by this change.

This is the metadata-reading prerequisite for the registry above, not yet the
connection between evaluated default markers and default request selection.

Inspection validation in `/home/fin/luna` (all exit 0):

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation_attributes --test library_attributes --test derivation
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derivation attributes: 6 passed; existing library attributes: 6 passed;
  derivation: 74 passed. Report: `/tmp/luna-tools-3kfrnew4/result.json`.
- Formatting: `/tmp/luna-tools-0tt8454b/result.json`.
- The new suite uses the real frontend inspection entries and checks unchanged
  original targets, duplicate use sites, merging order, required helper contract
  rejection, unused divergent strategy bodies and executable-mode rejection.

## Consuming default marker metadata

`attributes::default_strategy_markers` reads the selected `derivation_default`
property after ordinary attribute merging. The target must still be an original
deriver definition and the selected value must have both the Bool domain and a
completed Boolean payload. False is retained explicitly, so an outer disabling
marker cannot accidentally leave an inner registration active. Diagnostics and
dumps preserve the selected property's source range.

The single-package and multi-package merged-attribute inspection entries invoke
this consumer and include its S-expression markers when present. Raw attribute
inspection continues to expose unvalidated metadata for diagnostics. Unknown
properties are left to their own consumers; malformed inner values that have
been overridden are not selected or validated as active default markers.

These markers are registration requests, not registered/applicable strategies.
They do not replace registry ownership checks, visibility, trait instantiation,
program requirements or generated implementation validation. The standard
`attrs.derivation.default` library entry and executable selection remain to be
connected; tests here use ordinary metadata values to exercise the consumer.

Marker validation commands in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test derivation_attributes --test library_attributes
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Derivation attributes: 10 passed; library attributes: 6 passed; exit 0,
  `/tmp/luna-tools-rfne90no/result.json`.
- Formatting: exit 0, `/tmp/luna-tools-rrv_vf_g/result.json`.
- The added source tests cover aliases, false overrides, overridden malformed
  metadata, wrong targets/domains, raw-versus-merged inspection and the real
  multi-package CLI path. Executable default requests remain rejected.

Consumer/merge validation:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test attribute_phases --test attribute_merge
```

Attribute merge: 5 passed; attribute consumers: 6 passed; exit 0,
`/tmp/luna-tools-wtbzo0q5/result.json`. The direct consumer test rejects forged
Bool domains, non-Boolean payloads and member targets, preserves diagnostic
source, and retries successfully after budget exhaustion.

## Reserved member imports

Qualified import paths now use the same reserved `default` member spelling as
expression projections. This includes nested grouped imports, relative member
paths and ordinary aliases such as `automatic`. Bare `default` declarations
and aliases remain reserved; this does not introduce escaped identifiers or a
new declaration syntax.

The public library declaration is still pending a language decision: permit the
reserved spelling in module member declarations, or introduce a general escaped
identifier syntax. Import parsing can be completed independently, but does not
by itself provide `attrs.derivation.default` or executable default selection.

Validation in `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p parse --tests
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

- Parser suites: exit 0, `/tmp/luna-tools-1sg41sr1/result.json`. Includes
  accepted qualified/grouped paths and rejected reserved bindings, aliases and
  malformed imports; parsed declarations also validate AST layout and EOF.
- Formatting: exit 0, `/tmp/luna-tools-u_l2nkue/result.json`.
