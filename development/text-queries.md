# UTF-8 byte observations

`builtin.text` publicly exports `byte_length(str) -> u64` and
`byte(str, u64) -> ?u8`, plus explicit `comptime_byte_length` and
`comptime_byte` counterparts. Byte positions count UTF-8 bytes rather than
Unicode scalar values. Empty strings and all out-of-range indices return null;
length and index domains remain u64 on every target.

The implementation module is `text_bytes.fl`; the package root publishes the
`text` namespace. These operations are prerequisites for library text/Symbol
hashing, not completed Hash implementations.

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
