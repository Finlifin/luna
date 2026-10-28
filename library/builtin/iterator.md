# Iterator selection

`Iterator.nth(index: usize) -> ?Item` consumes the iterator and returns its
zero-based item. It stops production immediately after that item. Skipped items
and the remaining source are retired; the source cannot be resumed afterward.
An empty source or a position beyond its end returns `null`.

`Iterator.last() -> ?Item` consumes the entire iterator. Each replaced item is
retired, and ownership of the final item moves to the caller. Empty input returns
`null`. Neither method requires `Item` to implement `Clone` or `Copy`.

```flurry
let first: ?bool = false;
let second: ?bool = true;
let selected = first.as(Iterator).chain(second).as(Iterator).nth(1);
let found = selected matches true?;
assumes found;
```

An optional item adds a second optional layer to the result. A source containing
one `null` item returns a present `null` payload, not an absent result. Both
methods preserve this distinction.

These are inline consumers of `emit_item`. The supported native path jointly
expands producer, adapters and handler. The regression suite checks complete
expansion, resource cleanup, and LLVM O0/O3 execution; the sysroot fixture checks
the ordinary public imports. Unexpanded resource-checked interpretation still
rejects some pointer-bearing items: `nth` needs request-side return provenance,
and `last` needs captured-pointer writeback evidence. These checks remain enabled.

Checks from the repository root:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_iteration
python3 -B development/library_check.py --release --timeout 900 --stage sysroot-iteration
```
