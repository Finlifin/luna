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
   it. It is currently a placeholder. Explicit execution frames must retain
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

The pending Tuple builtin-binding work remains uncommitted. Its symbol-specific
bootstrap is not evidence that general builtin callable binding is complete;
review it against the ordinary callable path before integrating it. The existing
TypeList term variants, NF domain restrictions, builtin classifications and
contextual-input evaluator have not been removed by this inference correction.

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

The worktree also contains earlier Tuple migration and unrelated author edits;
these are not part of this inference change or evidence of VM completion.
