# Request-relative lexical handler returns

## Reproduced boundary

`compiler/luna/tests/handler_lexical_returns.rs` isolates pointer returns without
iterator traits, optional layouts, affine payloads or captured slot writeback.

- A handler returning its installer's captured pointer works through an ordinary
  function call. Returning the function's own local target is still rejected.
- An ordinary direct effect handler returning its request parameter fails with
  `pointer target provenance is not established`.
- Reading that parameter before returning it reaches a different failure:
  `every callable candidate requires a memory contract: unknown memory target`.
- Declaring the same operation `inline effect` and jointly expanding the handler
  and request makes both valid cases executable. Expansion still rejects a
  returned address into the caller's local storage.

These refusals are implementation gaps, not language rules. The tests explicitly
mark the temporary refusal assertions; replace them with successful execution
when the ordinary request-relative path is implemented. Keep the positive
capture cases and negative lifetime cases. Do not require inlining in the final
ordinary effect implementation.

## Why an admission check cannot simply be removed

`moves::HandlerReturnValue` stores installer-relative `Place` targets. Handler
parameter memory is represented separately by virtual roots described in
`Report.borrowed_roots`. `moves/effects.rs::check_handler_return_origins` refuses
to publish those virtual roots as installer targets. That check is necessary:
a request may originate in a callee whose locals are retired during lexical
return. Equal LocalIds or pointee types do not imply equal storage/lifetimes.

`memory/effects.rs::infer_handler_arm_memory` also distinguishes lexical return
from ordinary operation completion. Its private analysis body observes scalar
lexical return expressions without joining them into the operation's result.
Pointer exits need their own summary, not `memory.result_targets` for the resume
value. Reading the parameter first cannot supply the missing exit substitution.

`moves/effects.rs::effect_return_checkpoint` currently joins the installed
lexical-return candidates at suspension checkpoints. It has no request argument
substitution there. `EffectResultInput` routes ordinary completions through row
slots, fallbacks and call boundaries; lexical results need equally explicit
routing. Reusing the unqualified global candidate union for request-relative
roots would attach one request's argument to another request's exit.

## Implementation boundary

1. Represent lexical exit targets separately from normal operation results.
   Distinguish installer targets from symbolic request-parameter paths. Retain
   operation identity, ABI, full instantiated signature and route/installation
   identity; include guarded optional/variant leaves and callable captures.
2. Instantiate request-relative leaves from already evaluated request inputs,
   preserving argument order and pending ownership. Never evaluate inputs again
   or read their sources before their evaluation. Unmatched/unknown leaves stay
   unknown; missing evidence cannot become an empty successful mapping.
3. Route those exits through ordinary calls, forwarding, fallback and nested
   handlers. Requester-local targets must fail when their frame is retired;
   pointers into installer-owned storage may survive only with their live owner.
   Nested lexical returns must retain the correct lexical return destination.
4. Join only applicable exit candidates, including guarded absent payloads.
   Charge path/candidate/capture work to the existing shared budget. Publish
   complete candidates atomically; a failed attempt must not seed later proofs.
5. Integrate memory access summaries, move verification, cleanup publication and
   runtime planning together. Do not claim the feature complete from a synthetic
   substitution helper or a successful inlined program alone.

Required follow-up coverage includes a request made by a non-inline callee,
requester-local dangling storage, multiple requests with distinct arguments,
nested/fallback routing, pointer-bearing affine aggregates and optional results,
plus budget failure/recovery. The existing library `find`/`nth` cases then provide
public-interface acceptance. `last`/`reduce` capture writeback is a separate
obligation and must not be silently waived by this change.

## Current evidence

From `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test handler_lexical_returns
```

Exit 0, two tests (multiple cases); `/tmp/luna-tools-xcgsk2oe/result.json`.
This establishes the boundary and lifetime checks above. It does **not** deliver
ordinary request-relative lexical pointer returns or remove the documented
iterator limitations. No verifier admission rules were relaxed.
