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
