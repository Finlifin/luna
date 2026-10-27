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
