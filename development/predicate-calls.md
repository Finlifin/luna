# Ordinary calls in checked program decisions

`assumes` belongs to program verification. Its Boolean decision may call an
ordinary runtime function when the compiler can certify the actual computation
as a stable, finitely completing observation. Neither a Boolean return type nor
a source `pure` marker is that certificate. This does not send program predicates
to PTC or turn a callee's body into a caller-visible specification.

The elaborator now retains call-observation obligations for whole-package
behavior validation. They reuse the bounded stability/termination analysis used
for defaults, including scoped staged-input evidence and private structural
observation views. Unknown callees, external writes and uncertified recursion or
loops remain rejected. Argument computations and selected trait implementations
are checked as part of the call, not merely the outer callable's identity.

Source admission and retained-IR admission remain separate. The NIR candidate
walker only permits investigating supported expression shapes containing calls;
`checkable_predicate` still rejects calls without consulting any function world.
After requesting the behavior proof, source lowering evaluates the condition
once as an ordinary `if`, with an empty successful branch and a rejecting check
in the false branch. This preserves argument evaluation and
short circuiting while retaining facts on the successful control-flow edge.
Replacing the condition with a temporary Boolean lost arithmetic relations in
`assumes value < 255 and ready()`; the branch form avoids that loss.

A checked opaque call such as `assumes small(value)` does not automatically
publish `value < 255` from the implementation of `small`. That requires separate
specification/contract elaboration. The ordinary verifier still checks arithmetic,
resource delivery and cleanup, including argument temporaries, before publication.

## Remaining boundaries

- This stage certifies runtime calls. Comptime calls still require phase-aware
  behavior evidence: receiver value references cannot be treated as private
  runtime value parameters, and runtime Copy/Drop is not comptime permission.
  The positive Type-producing invocation regression is retained with an explicit
  ignore reason; it currently fails admission and is not counted as supported.
- Calls in retained contract expressions and computed pattern predicates do not
  gain admission through the source decision path. General program-predicate
  declarations and specification-to-decision correspondence remain separate work.
- Finite analysis is bounded and conservative. Rejection for unavailable evidence
  is not a proof of divergence or instability.

## Verification

Working directory: `/home/fin/luna`.

- `python3 -B development/check.py --timeout 900 -- test -p luna --test
  predicate_calls --test error_result_flow --test runtime_purity` exited 0
  (`/tmp/luna-tools-osiaf87y/result.json`): runtime decision, short-circuit order,
  failure, arithmetic facts and behavior-boundary checks passed; the comptime
  positive regression is explicitly ignored. Its earlier execution failed at
  call-stability admission (`/tmp/luna-tools-fuh4f29u/result.json`, exit 101).
- `python3 -B development/check.py --timeout 900 -- test -p nir --test
  predicate_reads` exited 0 (`/tmp/luna-tools-mugznymm/result.json`), including
  independent rejection of calls by retained predicate admission.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test
  elaboration -- program_assumes checked_field_predicates` exited 0
  (`/tmp/luna-tools-jxstk1wc/result.json`): the four directly affected checks
  retain failure, observation invalidation and unsupported-call rejection.
- The wider elaboration suite had 23 failures
  (`/tmp/luna-tools-gdhc8_1s/result.json`, exit 101). Disabling only the new
  admission branch reproduced the exact same 23 failing tests
  (`/tmp/luna-tools-leqdpgu3/result.json`, exit 101). Their expectations were not
  changed to accommodate these failures; this is not a passing full-suite claim.
- `python3 -B development/library_check.py --timeout 900 --stage
  sysroot-qualified-methods` passed native execution with the real sysroot
  (`/tmp/luna-library-tdohqbd2/summary.json`), including a direct
  `assumes checked.as(OptionalOps<bool>).is_some()` call. The first attempt
  exposed duplicate cleanup-boundary matches from two checks sharing the same
  source range (`/tmp/luna-library-n9th3spm/summary.json`, failed). The successful
  branch now contains only `Unit`; only the rejecting branch has a check site.
- Workspace format checking exited 0
  (`/tmp/luna-tools-svidlr05/result.json`); `git diff --check` passed.
