# Runtime str integration boundary

Audit at `d2f1c492` on `dev`. This is an implementation dependency analysis, not
an extension to language semantics or a claim of runtime str safety support.
Authority remains the resource-management documents and ordinary call rules.

## Reproduced production failure

From `/home/fin/luna`:

```text
python3 -B development/check.py --timeout 900 -- run -p luna -- test development/repros/native-text-method-provenance.fl --sysroot library
```

Exit 1, `/tmp/luna-tools-d50d_anj/result.json`. The real sysroot source test reaches
resource-checked native compilation, then rejects a `str` input with
`string-view memory provenance is not implemented` and a `text-input` leaf.
An earlier attempt to reuse the binary was unavailable because normal cache
maintenance had removed it (`/tmp/luna-tools-e_8_u_9w/result.json`); the Cargo
wrapper rebuilt it for the actual reproduction.

## Existing evidence and its limits

- `memory::borrowed_input_layout` distinguishes `BorrowedInputLeaf::Text` from
  pointer slots. Text leaves retain logical input paths, physical storage paths
  and variant guards. A pointer to a str descriptor remains a pointer leaf; it
  is not interchangeable with the descriptor's backing character region.
- `memory::infer_text_regions` computes source-body read and return dependencies.
  Its `TextRegionOrigin` is Static or parameter-relative. It tracks descriptor
  writes, closure calls, recursive dependencies, entry snapshots, preconditions
  and postconditions. It is not just a literal-only scan.
- `luna::text_regions` exposes that analysis as a diagnostic entrypoint. It does
  not certify the ordinary resource/native pipeline.
- `TextRegionSummary::reads` explicitly excludes implicit cleanup footprints.
  Parameter-relative dependencies also do not prove a caller's region live.
- `memory::pointer_input_layout` deliberately rejects text leaves. Many move,
  call, capture and effect consumers still use this pointer-only interface.
  Filtering out text there would erase obligations, not integrate the analysis.

## Required production connections

The integration must carry text-region obligations alongside pointer origins,
without pretending a str descriptor is an addressable character pointer slot.
The existing logical/physical path and variant-guard distinction must survive.

1. Parameter admission in `moves.rs` currently builds pointer borrows and
   `BorrowedField`s from `pointer_input_layout`. It needs distinct region inputs,
   supplied by the caller and tied to the relevant input path and active variant.
2. `Flow` / `ValueOrigins` currently describe owned places and pointer targets.
   A text descriptor copy or move must retain its backing dependency; replacing
   its descriptor updates that dependency. Killing descriptor storage must not
   kill independently owned backing storage, nor may copying a descriptor extend
   the backing storage's lifetime.
3. Ordinary calls must substitute text summaries using the delivered arguments,
   establish required regions at each read, and attach returned dependencies to
   the actual result. Callee-local descriptor identities must not escape as
   caller roots. Closures, branch joins and early exits need the same transfer.
4. Cleanup must export character reads as region obligations and rebase them
   through the current owned value, just as `memory/cleanup.rs` and
   `moves/cleanup_memory.rs` currently do for pointer reads. A source summary that
   checks only explicit calls cannot replace this step. Effect transfer and
   cleanup after rejection must preserve these obligations too.
5. Only then can individual pointer-only consumers migrate to the borrowed-input
   API. A successful standalone text summary is not blanket permission to skip
   the remaining consumers or to add Copy/Clone evidence.

The initial native acceptance should include the existing literal-backed repro,
a forwarded parameter whose caller supplies a live backing region, branch and
closure transport, descriptor replacement, and rejection of a dead backing
region. Include guarded optional/error payloads and cleanup reads before claiming
aggregate support. These are behavior requirements, not just constructor tests.

The compiler-operation bindings for str remain on their legacy lowering path.
Typed signatures and explicit staged callbacks are available, but switching those
bindings alone cannot establish the missing resource evidence. Symbol and integer
operation wrappers do not have a borrowed-character-region obligation.

The existing dependency analysis and layout tests passed separately:
`development/check.py --timeout 900 -- test -p luna --test text_regions --test
text_view_layout`, exit 0 (`/tmp/luna-tools-w5ybmspv/result.json`). This proves
those existing analyses remain usable; it does not overturn the native failure
above. No resource admission rule or library str binding changed in this audit.
