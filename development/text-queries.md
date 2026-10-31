# UTF-8 byte observations

The runtime `builtin.TextBytes` and pure comptime `meta.ComptimeTextBytes`
traits expose `value.byte_length() -> u64` and `value.byte(index: u64) -> ?u8`.
Their respective default preludes import both protocols; comptime selection
prefers the pure comptime trait. Explicit runtime protocol selection cannot
bypass phase checking. The former free-function wrappers, including the
`comptime_` spellings, are removed.

Positions count UTF-8 bytes rather than Unicode scalar values. Empty strings
and all out-of-range indices return null; length and index domains remain u64
on every target. Runtime implementation is in builtin/text_impl/bytes.fl;
comptime implementation is in meta/data/text.fl, with centralized intrinsic
bindings in meta/text_impl/bindings.fl. These observations do not implement
Unicode decoding or owned String storage.

NIR represents observations with `(text-query byte-length ...)` and
`(text-query byte ...)`. An internal tuple packet preserves left-to-right,
once-only argument evaluation and early exit. This packet does not redefine
source Tuple types. Phase validation checks packet arity, operand domains and
result domain. Traversals, substitution, budgets and resource/purity checks
visit the operands; immutable text observation does not grant arbitrary owners
Copy. Required comptime materialization invokes the VM operation.

The VM reads immutable UTF-8 storage directly. LLVM compares the u64 index
unsigned against the length before computing and loading the byte address.
No source pointer is exposed. Out-of-range reads construct an absent optional.

## Validation

Working directory: `/home/fin/luna`. All commands below exited 0.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p nir --test phases
python3 -B development/check.py --timeout 900 -- test --release -p luna -p elaboration -p comptime_interpretor --test text_bytes --test symbols --test pure_bitvec
python3 -B development/check.py --timeout 900 -- test --release -p luna -p elaboration -p comptime_interpretor --test sysroot_preludes --test text_bytes
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

- NIR phases: 31 passed; report `/tmp/luna-tools-jr9vi7zl/result.json`.
- Text, Symbol and pure BitVec: 63 passed; report
  `/tmp/luna-tools-byniud64/result.json`.
- Final public module exposure and text regression: 6 passed; report
  `/tmp/luna-tools-rio0_px9/result.json`.
- Format check: `/tmp/luna-tools-9f06gfoy/result.json`.

Tests cover multibyte UTF-8, empty strings, length boundaries, maximum u64
indices, both execution phases, once-only ordered operands, early exit and
invalid domains. Native assertions run at LLVM O0 and O3. The public sysroot
fixture imports `builtin.text`; a private sibling file alone was insufficient.
The Symbol budget regression now expects the existing lookup-budget diagnostic:
lookup charges precede literal encoding, so the previous literal-budget wording
was stale. Retry and deterministic output assertions remain in place.

The first two reports precede the public namespace correction; the final suite
checks that correction and re-executes the byte tests. Temporary report paths
are local evidence, not repository artifacts. No full workspace test was run.

## Phase-specific receiver methods

The public byte observations now belong to the two phase-specific traits.
Comptime self uses value references; runtime self retains the ordinary str
value/region contract. Both protocols are imported by their owning package's
prelude. No source `comptime_` method prefix or primitive-name special dispatch
was introduced. Standalone fixtures include the shipped definitions and their
centralized bindings; production examples use public imports only.

The migration exposed missing trait-callee handling in text-region inference.
It now reuses `MethodCallees` to select a concrete implementation, keyed by its
full interface signature and subject, before deriving read/return dependencies.
Direct calls, method identities and stable local aliases follow the same code
identity path. Interface predicates still require composed contracts and are
rejected by the existing selector; this change does not erase their obligations
or turn a region summary into proof of memory access permission.

The comptime differential test previously used the runtime reference engine,
which cannot model comptime value-reference self. `Interpreter::for_comptime_tree`
now selects the tree evaluator while retaining comptime phase validation,
value references and input admission. Both invocation and contextual fragment
bytecode routing are disabled; default interpreter routing is unchanged. Tests
compare bytecode and tree results and assert the reference route executes no
bytecode invocations or staged input fragments.

The public sysroot qualified-method example includes comptime byte reads with
the same method spelling as runtime, including multibyte text, empty text and
bounds. Runtime reference/LLVM tests exercise the method forms separately.
No Unicode decoding, owned String implementation or Symbol byte protocol is
claimed by this migration.


### Native resource integration still missing

The real-sysroot native attempt for runtime methods remains rejected:
`/tmp/luna-library-cqywo09q/sysroot-qualified-methods/result.json` exits 1 with
`string-view memory provenance is not implemented`. `pointer_input_layout`
explicitly rejects `BorrowedInputLeaf::Text`; source-region inference alone is
not proof that backing storage remains live. The expected-success program is
retained at `development/repros/native-text-method-provenance.fl`, separate
from the passing comptime/public-prelude integration. It must eventually run
with ordinary resource checks enabled; no unsafe escape or forced inlining is
used to hide this gap.

The next required connection is between text backing-region dependencies and
resource call/return lifetimes, including guarded aggregate leaves. Copying a
str descriptor must preserve its region dependency rather than granting an
unconditional lifetime. This is an existing missing native memory contract,
not a change to byte indexing or trait phase semantics.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test -p luna --test text_bytes --test comptime_bytecode_observations --test text_regions --no-fail-fast`:
  58 tests passed, exit 0; `/tmp/luna-tools-sokv77ul/result.json`.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test text_bytes --test comptime_bytecode_observations --test text_regions --test sysroot_preludes --no-fail-fast`:
  the 11 prelude tests passed at `/tmp/luna-tools-6i0g0kty/result.json`;
  this initial combined command exited 101 for the subsequently repaired
  method-summary and reference-engine tests. It is not a whole-command pass.
- `python3 -B development/check.py --timeout 900 -- test -p luna --test text_iteration_prototype`:
  three tests passed, exit 0; `/tmp/luna-tools-5p8hd7rz/result.json`.
  The prototype still checks that resource-verified string execution rejects
  missing backing-region provenance; no expected failure was removed.
- `python3 -B development/library_check.py --stage sysroot-qualified-methods --stage sysroot-meta --timeout 300`:
  sysroot-meta passed at `/tmp/luna-library-jitz9apt/sysroot-meta/result.json`.
  The combined run exited 1; the first qualified fixture needed explicit
  inline selection of its comptime constant, and the next run exposed the
  native provenance blocker described above.
- Final `python3 -B development/library_check.py --stage sysroot-qualified-methods --timeout 300`:
  the original qualified runtime operations and new comptime text observations
  passed one native test, exit 0; `/tmp/luna-library-unhgxhn2/summary.json`.
  The unresolved runtime text test is retained in the dedicated repro.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  exit 0; `/tmp/luna-tools-hrdkro9j/result.json`; `git diff --check` passed.

No full language suite or performance benchmark was run.
