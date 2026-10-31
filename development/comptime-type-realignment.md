# Comptime/type implementation realignment — 2026-10-29

## Authority and scope

The divergence report in `ai_dev_logs/` is an implementation audit, not a
replacement language specification. The author excludes its chapter 7 proposal
for the bidirectional elaborator, not all of chapter 7. Elaboration continues
to follow `docs/compilation/elaboration.md`, `local-dependencies.md`,
`candidate-solving.md` and `staging-and-freezing.md`.

The foremost development principle is now recorded in AGENTS.md: reject
short-term test pass rate chasing. A regression that encodes an implementation
restriction must be checked against the language contract, not preserved merely
because it passed before.

Do not adopt these report claims as new requirements:

- Optional/error methods use separate runtime and comptime traits by the
  author's later decision. Do not migrate them back to inherent methods on
  the report's authority.
- Pure invocation and opaque normal-form formation are distinct. Do not
  unfold an NF head or turn every Type-returning invocation into an NF.
- Interned core identity does not establish complete type equality; reference
  substitutions and dependency environments retain their own meaning.
- Invocation result caching remains restricted to pure Type results, as the
  author requested. A general VM does not imply caching every invocation.

## First correction: ordinary normal-form inference

`type_resolution::matching::decompose` admitted argument proposals only for
Tuple/List-classified heads, and only for a single argument. The same operation
now accepts any matching NF declaration identity and argument count. It proposes
substitutions inside the existing candidate transaction and retains the full
original type equality obligation. It grants no injectivity, disjointness,
layout, implementation-subject eligibility or function-body unfolding.

The distinction is existential proposal versus equality proof: choosing a
candidate input that makes the original equality hold is permitted; deducing
that two existing inputs are equal from equal factory results is not. PTC's
non-injectivity tests remain unchanged. Conflicting input proposals, different
heads, arity differences and failed candidate export remain checked.

Tests previously asserting that removing a builtin classification must disable
inference now check identical ordinary inference behavior. Source coverage uses
a user-defined, two-input NF with a Type and an Integer input, with no Tuple or
List selector; it also rejects a different required output and body unfolding.

## Mainline work still required

The following are implementation work, not features completed by this change:

1. Implement the `comptime_compiler` engine-bytecode carrier specified in
   `docs/comptime/purity-and-normal-form.md` and connect actual invocation to
   it. Initial control lowering and invocation execution are now connected;
   full operation/call coverage and stable code caching remain open. Explicit
   execution frames must retain
   work across host-query suspension without replaying completed effects;
   runtime IIR simulation and mixed-stage interpretation retain their distinct
   responsibilities. AST-shape peepholes are not a substitute for this work.
2. Unify comptime values and their checked freeze/restore transport. Remove
   NF-only input representations and Type-list-specific value privileges via
   ordinary value and binding mechanisms. Preserve dependent domains,
   evaluation order, contracts and opaque identity throughout migration.
3. Connect the documented TypeCore/TypeRef representation to production
   consumers; retain substitution and EnvironmentPlan checks. A tested but
   unused interner is not delivery of this architecture.
4. Connect genuine blocked work and invocation dependencies to the documented
   local/query scheduling model. Do not replace the author's elaboration
   design with the report's proposed one, or erase useful partial observations
   merely because they are not full evaluation.

The existing TypeList term variants, NF domain restrictions, builtin
classifications and contextual-input evaluator remain migration work. The
builtin-binding bootstrap is now integrated as described below; this does not
complete general VM intrinsic binding.

## Builtin callable bindings

Tuple and meta.List are source constants bound to `.Tuple'builtin` and
`.List'builtin`, respectively. Both use a shared signature registry and the
same callable telescope registration, alias resolution, Type application and
invocation metadata. The registry declares each builtin's signature; it does
not introduce another NF input category or recognize a public source name.
The driver selects canonical declaration identities. Rebinding a builtin under
another constant preserves that identity and validates the annotation.

Named and unnamed fixed signature inputs retain positional slots; an unnamed
input introduces no public argument label. Collected inputs retain their
ordinary callable collector position. Checked builtin heads enter the same
forward congruence path as checked source NF heads, with no body equations.
Neither List nor Tuple publishes a fake executable function body.

Materialization's annotation visitor also sees invocation identity values.
Those values now retain their sort and reach the existing domain-aware checks;
they are not submitted to the associated-Type normalizer or restricted to the
trait-index value representation. The real-sysroot Tuple/list-input case covers
this boundary alongside runtime tuple layout.

This closes the earlier bootstrap patch's Tuple-only registration path. Other
compiler operations still use the old operation-binding path, and Type-list
values still have specialized internal representations. These are not claimed
to be solved by replacing the two public bindings.

## Validation

Working directory: `/home/fin/luna`.

- `python3 -B development/check.py --timeout 900 -- test -p type_resolution -p ptc`:
  passed; `/tmp/luna-tools-9lzcq8x9/result.json`. Initial runs identified obsolete
  role-dependent rejection expectations; updates retain proof replay and add
  conflicting-interface rejection. PTC proof rules are unchanged.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_form_inference`:
  passed; `/tmp/luna-tools-wip_yjuz/result.json`.

- `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_form_inference --test normal_form_methods --test normal_patterns --test type_lists --test default_calls --test extended_calls`:
  passed; `/tmp/luna-tools-arlren1y/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  passed; `/tmp/luna-tools-tyiqkze3/result.json`. `git diff --check` passed.

The worktree also contains unrelated author edits; these are excluded from
both implementation commits. Neither stage is evidence of VM completion.

Builtin-binding validation (same working directory):

- `python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli builtin_`:
  passed; `/tmp/luna-tools-svfsgr15/result.json`. Includes real-sysroot Type
  invocation/runtime tuple layout, renamed canonical bindings, forward aliases,
  dynamic callees, unnamed inputs, invalid signatures and recovery.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test library_protocols`:
  passed; `/tmp/luna-tools-e7p46nx0/result.json`. Checks real library sources,
  homogeneous lists, dependent collectors, ordinary values and native affine
  tuple fields. The later unnamed-signature correction has separate CLI
  coverage; the library uses named signature inputs.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  passed; `/tmp/luna-tools-nfqsod3i/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_form_methods --test normal_patterns --test type_lists --test list_patterns --test extended_calls --test default_calls --test tuple_type_syntax`:
  passed; `/tmp/luna-tools-snqw1qs3/result.json`. The initial run exposed the
  missing computed NF receiver-domain path. NF applications now use the existing
  checked expression-domain observation for static methods and associated types;
  direct and aliased builtin callables have regression coverage.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`:
  final formatting passed; `/tmp/luna-tools-q9iw7jcb/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p materialization --lib`:
  passed; `/tmp/luna-tools-cn644dhi/result.json`.

## Invocation control carrier

The next delivered stage replaces the empty `comptime_compiler` crate with
bounded linear control lowering and connects it to actual comptime invocation.
Scope, validation and remaining architecture work are recorded in
[comptime-vm.md](comptime-vm.md#scope-and-current-boundary). No general call-stack
or query suspension capability is claimed by the first control-carrier stage.
