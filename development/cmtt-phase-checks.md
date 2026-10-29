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
