# Trait implementation phases

Semantic authority: [trait comptime phases](../docs/comptime/the-comptime-phases-of-traits.md).
Runtime and comptime implementation contributions belong to the same selected
implementation relationship. Missing phase support must not fall back to a less
specific implementation. Trait member phase and a function's comptime keyword
are distinct decisions; lowering an implementation must not rewrite every member
into a comptime function.

## Current foundation

HIR `ImplDef` retains an explicit `ImplementationPhase` with Runtime and Comptime
variants. Ordinary source implementations and the existing inherent-definition
assembler use Runtime. A `comptime impl` keyword wrapper is consumed during AST
lowering and records Comptime on the implementation rather than leaving the
keyword as an unevaluated generic attribute. HIR S-expression dumps include the
phase. General generated-definition assembly preserves the field supplied by its
caller.

This does not implement two-phase trait selection. Elaboration currently rejects
Comptime contributions explicitly before admitting implementation heads to the
runtime selection index. The rejection also applies to generated HIR: it is not
merely a source-parser restriction. Existing ordinary implementations keep their
current behavior.

## Member metadata validation

`attrs.comptime_phase` is an ordinary Object constant exported through its
prelude. Attribute inspection evaluates it through the ordinary package and
alias resolution paths. The `attributes::member_phases` consumer accepts merged
metadata and returns source-located `MemberPhase` records. It checks trait or
trait-implementation method/associated-declaration targets, the selected Bool
payload and its original domain. It uses the existing override order, preserves
unrelated keys for other consumers, and charges a shared budget. Failure does
not publish a partial plan or mutate HIR. Records have S-expression Display.

This validator is not yet wired into executable elaboration. Attribute inspection
does not by itself authorize phase selection, and the existing explicit rejection
of comptime implementation selection remains in force.

## Shared relationship index

The implementation index now has `insert_phase` and `select_phase`. Complementary
contributions with identical alpha-bound normalized heads, ordered parameter
domains, interface inputs and requirements share one relationship entry. Each
contribution retains its declaration identity. Ordinary `insert` keeps runtime
behavior. Same-phase duplicates still participate in ordinary coherence; open
interface identities cannot be paired. Insertion failure leaves the index and
its revision unchanged; successful additions invalidate older query results.

Phase lookup consumes the common type-resolution result, preserves deferred or
conflicting selection, and reports a missing phase on the selected relationship
without searching less precise candidates. Index and phase-result dumps include
both relationship and contribution identities. This remains index-level work:
source attribute application, phase-specific member completeness, shared
associated-item checking, and executable dispatch are still unconnected. The
pairing API expects already-normalized applicability predicates; it does not
prove arbitrary logical equivalence between differently represented conditions.

## Remaining integration

- Apply validated phase metadata to trait methods and associated members
  independently of their function execution signature, integrating its required
  computations with interface preparation rather than eagerly running bodies.
- Feed normalized source contributions into the shared relationship index,
  select the relationship first, then require the requested phase. Preserve
  shared associated-item consistency and phase-specific member completeness.
- Carry phase through checked implementation metadata, member lookup, execution,
  and generated-definition publication. Add missing-phase and non-fallback tests.
- Connect dynamic trait-method clause preparation without erasing interface
  origin, defaults, preconditions, postconditions, or selected instance identity.

The existing dynamic inherent-method implementation is not evidence that trait
phase selection or dynamic trait-method clauses are complete.

## Validation

In `/home/fin/luna`,
`python3 -B development/check.py --timeout 900 -- test --release -p luna -p hir
-p elaboration --test trait_phases --test display --test definitions
--test definition_interfaces --test generated_traits` passed 51 tests
(`/tmp/luna-tools-xujgg3q8/result.json`, exit 0). Source tests check both phase dumps,
independent member modifiers, explicit refusal of unfinished phase selection, and
ordinary implementation elaboration. Existing generated traits and definitions
remain accepted. The generated-implementation test was then extended to all four
trait/inherent and runtime/comptime combinations; its suite passed separately
(`/tmp/luna-tools-n4fvchr3/result.json`). Workspace formatting passed
(`/tmp/luna-tools-wk9lspyk/result.json`).

Member metadata validation passed 20 tests with
`python3 -B development/check.py --timeout 900 -- test --release -p luna
-p elaboration --test attribute_phases --test attribute_merge
--test library_attributes --test trait_phases`
(`/tmp/luna-tools-69fwiv54/result.json`, exit 0). These cover selected overrides,
target and Bool-domain rejection, budget failure/retry, unrelated metadata,
real attrs package/prelude resolution, and the existing implementation-phase
boundary. They do not demonstrate executable two-phase selection.

Shared relationship indexing passed 85 tests (73 existing implementation tests
and 12 phase tests) with
`python3 -B development/check.py --timeout 900 -- test --release -p elaboration
--test implementation_phases --test implementation`
(`/tmp/luna-tools-9z5hynz0/result.json`, exit 0). Checks cover complementary
generic heads in both insertion orders, mixed-sort domains, free identities,
opaque heads, missing-phase non-fallback, deferred selection, coherence conflicts,
open interfaces, stale queries and atomic budget retry. The preceding run
(`/tmp/luna-tools-r1hy0go2/result.json`, exit 101) reproduced unbound requirement
capture against the library compiled before the scope fix; the final run includes
that fix and passes its rejection regression. Workspace formatting passed
(`/tmp/luna-tools-l2puwhb9/result.json`); `git diff --check` also passed.
