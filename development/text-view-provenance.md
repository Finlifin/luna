# Text iteration prerequisite: borrowed storage provenance

`development/library-prototypes/builtin/text_iteration.fl` composes the existing
checked byte query with inline item delivery. It is intentionally outside the
sysroot and has no public export. The algorithm works in unchecked reference
execution; this does not prove a valid lifetime for the text view.

## Actual blocking paths

- `program/places.rs::readable_storage_input` does not form a runtime `str`
  pointer load. A proposed `Clone` implementation using `self.*` therefore fails
  during elaboration. No `str` resource implementation was added to the library.
- `verification/memory.rs::owned_fields` explicitly rejects `str` because its
  backing character storage has no represented provenance. This also rejects
  passing a text view inside the proposed Bytes iterator, even when the fixture
  happens to construct it from a literal.
- Existing resource tests already require that nominal wrappers, tuples and
  explicit Copy evidence cannot turn a borrowed text view into pointer-free
  data. This is a safety boundary to preserve, not an incidental missing match
  arm to bypass.

## Required implementation

The next implementation needs an explicit backing-region dependency for text
views, separate from the slot storing the view. Literal and Symbol-name storage
can provide immutable static origins; input views need symbolic origins supplied
by callers. Returning or embedding a view must preserve those origins through
aggregates, calls, closures and inline delivery. Copying the view transports its
dependency without acquiring ownership of the backing storage. Reading bytes
must retain the region's liveness and read footprint; bounded byte access does
not itself prove a valid region lifetime.

Do not map Str to a pointer-free scalar or an unrestricted mutable byte pointer.
The current pointer-layout helpers model typed storage paths, not a variable-size
immutable character region. They must not silently reinterpret the view's host
pointer/length representation as an existing scalar pointer contract.

Acceptance needs successful static-backed and caller-backed view transport plus
rejection of dead owners, escaped local backing storage and invalidated views.
Only after those checks should the text resource protocols and public bytes
iterator enter the compiled library. The deferred String/allocator work must
use the same dependency representation rather than introducing another model.

## Current executable evidence

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_iteration_prototype
```

Report `/tmp/luna-tools-3ucejhzl/result.json`, exit 0: two tests pass. Both execute
the prototype before and after inline expansion in the unchecked interpreter,
and separately assert that resource-checked execution rejects the missing view
provenance. Expected bytes are specified independently for ASCII, two-/three-/
four-byte UTF-8 characters and NUL. Empty input, filter composition, continue,
break and lexical return are exercised. These are algorithm and rejection
checks, not successful safe-mode or native execution of this feature.

## Implemented layout stage

`memory::borrowed_input_layout` now describes text-view slots separately from
ordinary pointer leaves. Text leaves retain a logical input path, physical
storage path, and enclosing variant guards. A pointer to a text slot remains an
ordinary pointer with Str pointee; it is not the view's backing character region.
Tuple heads use admitted layout metadata; arbitrary normal-form heads remain
opaque. Records, newtypes, enums, native optional/error payloads and captured
closure environments preserve their existing layout identities.

The existing `pointer_input_layout` API consumes this shared traversal but
rejects any text leaf, including its S-expression path in the diagnostic. Other
legacy Str rejection points remain in place. This refactors real layout consumers
without granting them missing provenance evidence. Unknown layouts, exhausted
budgets and excessive depth remain errors; failed queries leave no partial result
that can be used as a successful layout.

This stage does not supply backing-region origins, transport through function
summaries, or lifetime verification. Those remain required before the prototype
can be exported. Layout discovery is the input to that implementation, not a
replacement for it.

### Layout validation

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- check --release -p elaboration --tests -p luna --test text_view_layout
python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib -p luna --test text_view_layout --test text_iteration_prototype --test library_protocols
python3 -B development/check.py --timeout 900 -- test --release -p luna --test resources borrowed_string_views_do_not_gain_pointer_free_layout_from_copy -- --exact
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The compile check passed (`/tmp/luna-tools-4y5h7yti/result.json`, exit 0).
The combined execution passed 286 elaboration unit tests, 80 luna unit tests,
25 library protocol tests, 2 prototype tests, and 1 source layout test
(`/tmp/luna-tools-gukf1ysj/result.json`, exit 0). The source test retains a real
Tuple normal-form head and checks nested enum/newtype text slots and guards.
Formatting passed (`/tmp/luna-tools-rhi0lfot/result.json`, exit 0), as did the
diff check. Runtime text-region transport is still not implemented.
The existing str/Copy regression also passed (one test, three source shapes;
`/tmp/luna-tools-578yimc5/result.json`, exit 0). Explicit Copy evidence still does
not authorize dropping the backing-storage dependency.
