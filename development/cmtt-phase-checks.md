# Runtime storage checks for contextual code

CMTT expression quotes, expression generator interfaces and pattern quotes now
share a complete nominal representation snapshot, including newtypes. Looking
through a newtype for storage phase does not equate its identity with its
representation or unfold an opaque normal-form head. Phantom input Types that
are absent from the representation remain legal.

The shared NIR storage inspection now descends through Optional payloads. A
runtime optional cannot hide Type, Integer, Float, Object or comptime function
storage, even when its current value might be absent. These optional domains
remain usable in comptime functions.

The original reproduction was incorrectly accepted before the change:

```flurry
typealias Type = .Type'builtin;
newtype Hidden = Type;
comptime fn identity(code: <|-Hidden>) -> <|-Hidden> { code }
```

No invocation or splice is needed: definitions must expose valid runtime code
interfaces. The failing pre-fix test report is
`/tmp/luna-tools-o1p9miui/result.json`.

The new source suite covers 180 combinations of underlying domain, direct or
nested nominal representation, expression results/contexts, optional and
function results, pattern subjects/outputs, and discarded quote declarations.
Positive tests execute ordinary newtype code and retain phantom Type inputs.
NIR tests independently exercise optional runtime parameters and permitted
comptime parameters.

Validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p nir --test phases
python3 -B development/check.py --timeout 900 -- test --release -p luna --test cmtt_phase_boundaries --test cmtt --test newtypes
```

The NIR command passed 35 phase tests (`/tmp/luna-tools-p4qb9285/result.json`).
Formatting and diff checks passed.

The source command passed all 527 CMTT tests and the three new boundary tests.
The broad newtypes target passed 199 tests but failed 12, so that command exited
101 (`/tmp/luna-tools-5fyupxi7/result.json`). Ten failures panic at
`program/type_defaults.rs:242` while preparing generic member inputs. The other
failures are `dynamic_chained_members_reuse_outer_inputs_and_self` (Type input
annotation) and `installed_effects_do_not_revive_wrapped_pointer_targets`
(diagnostic expectation). These were follow-up failures, not passing coverage; a before-change
comparison of that full target was not performed.

This change does not transport quoted code through the ordinary VM or complete
source builder default/method publication. It closes a storage-phase hole in the
existing code interface boundary needed by that work.

## Generic projection follow-up

Constant members capture the selected container inputs but do not own a nominal
clause telescope. Their expression preparation now bypasses member parameter
metadata after rejecting explicit member arguments; container formation and
constant initializer contracts still run. This removed the ten metadata lookup
panics in the first rerun (209 passed, two failed;
`/tmp/luna-tools-vl91ft3h/result.json`).

Resolver-identified type and constant members are now prepared before enum-group
namespace probing. Otherwise an alias chain such as
`Alias<value>.Captured.This` tries to complete an invocation-time Type parameter
as a static type annotation. Enum groups retain their namespace-first handling:
moving all projections ahead of that probe broke the existing alias/group test
in `/tmp/luna-tools-i5_q0t4f/result.json`; the final ordering is restricted to
members identified by the resolver.

The wrapped-pointer effect regression now recognizes the explicit diagnostic
that the pointer owner is not fully initialized. It also checks the same two
closure forms without consuming the owner, requiring each to return `true`.
This preserves the rejection requirement and verifies that a live owner is not
rejected indiscriminately.

Final validation from `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test newtypes --test enum_groups
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 211 newtype tests and seven enum-group tests passed (exit 0,
`/tmp/luna-tools-s9e6lnc0/result.json`). Formatting passed (exit 0,
`/tmp/luna-tools-1thfn6qg/result.json`); diff whitespace checks passed.
These close all twelve failures recorded above, without claiming that the
ordinary VM now transports quoted code or publishes source builder defaults.

## Shared contextual code witnesses

`nir::code_values::{ExpressionCode, PatternCode}` now owns the code witness
representation previously private to the elaborator. Both expression and
pattern generators, template consumers and splice expansion use these shared
structures through the existing imports. The S-expression dumps retain their
previous shape, including escaped port/output names and explicit pending-body
markers.

The shared interface validator checks runtime storage domains against the
provided nominal metadata and scope. Expression ports and default source keys
must refer to context identities; a default target may remain an external
lexical identity. Fixed dependencies are retained independently of exposed
ports. Quotation and pattern checking call this validator; body typing,
substitution, resource checking and rejection of pending bodies at emission
remain with their existing consumers. Public Rust construction is not proof
that a body is well typed or ready for publication.

This removes the elaboration-private representation dependency for future VM
and definition-builder consumers. It does not add an ordinary VM Value variant,
a contextual storage domain, Any transport, or source builder code defaults.
Those paths must retain code category and interface evidence rather than using
the quote's runtime result type as the type of the code value itself.

Validation from `/home/fin/luna` (dev/test profile):

```sh
python3 -B development/check.py --timeout 900 -- test -p nir --test code_values --test code_interfaces
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries
python3 -B development/command.py --timeout 120 -- cargo fmt --all
git diff --check
```

All 17 NIR tests passed (`/tmp/luna-tools-uu8kd2lm/result.json`), including
malformed interface rejection/recovery, fixed dependencies and external default
identities, comptime storage rejection, and pending-body preservation. All 527
CMTT tests and three phase-boundary tests passed
(`/tmp/luna-tools-zxvjy0yw/result.json`). Formatting completed successfully
(`/tmp/luna-tools-nkavxnfy/result.json`); diff whitespace checks passed.

## Invocation-owned VM code storage

The VM now carries `Value::Code(CodeValue)` with distinct expression and pattern
variants referencing the shared NIR witnesses. Owned reads, explicit owned
copies and rollback checkpoints retain the same immutable code identity,
context/default mappings and success bindings. They do not evaluate quoted
bodies; a nonterminating runtime body is valid to transport.

Transport charges interface names, domains and body traversal. Missing bodies,
nested unresolved pattern-code markers, malformed expression port/default
sources, exhausted budgets and excessive body nesting are rejected before a
copy is returned. Failure leaves the original owner intact; a later operation
with adequate budget can succeed. Raw Rust construction remains untrusted:
these representation checks do not certify body typing, scope substitution or
runtime resource obligations.

Code has no ordinary Term data domain. Typed immutable input admission and
normal-term/cache-key export reject it, also inside aggregates. Runtime VM
copies reject code with PhaseMismatch. Ordinary constant materialization has an
explicit code rejection rather than treating a quote as its runtime result.
The internal Value comparison uses shared witness identity; it does not provide
a source Eq protocol or claim equivalence of separately constructed programs.

This delivers invocation-owned transport only. Source quotation still uses the
elaborator's code-value path. Ordinary callable contextual domains, typed code
containers, Any transport and source builder default/method publication remain
to be connected; no library API is advertised as supporting those paths yet.

Validation from `/home/fin/luna` (dev/test profile):

```sh
python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib
python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_phase_boundaries --test comptime_memory
python3 -B development/check.py --timeout 900 -- test -p luna --test execution --test nominal_value_parameters memoized
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

All 208 VM tests passed, including six code-transfer tests
(`/tmp/luna-tools-vu_lm4tw/result.json`). The CMTT/phase/memory groups passed 547
tests (`/tmp/luna-tools-59p4z5mr/result.json`); the result-cache groups passed 14
(`/tmp/luna-tools-qc71fxts/result.json`). The earlier dev-stack failure and its
independent freezing repair are recorded in `development/comptime-vm.md`.
