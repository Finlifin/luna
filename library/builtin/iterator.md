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

## Flattening nested iterators

`Iterator.flatten()` consumes an iterator whose `Item` implements `Iterator`.
It returns `FlattenIterator<Self>` and removes exactly one iterator layer.
Each inner iterator finishes before the next outer item is requested. Empty
inner iterators emit nothing; an empty outer iterator never starts an inner
one. `IntoIterator` conversion is not implicit in this adapter.

```flurry
let a: ?bool = false;
let b: ?bool = true;
let first: ??bool = a;
let second: ??bool = b;
first.as(Iterator).chain(second).as(Iterator).flatten().as(Iterator)
    .foreach(|value| { {- Consume false, then true. -} });
```

An additional optional layer remains part of the item: flattening `???bool`
produces `?bool` items, so a present inner `null` is still emitted. Noniterable
items are rejected when selecting `flatten`. Neither items nor inner iterators
need `Clone` or `Copy`. Downstream early exit retires the current item, remaining
inner items and outer remainder through the normal resource checks.

The implementation uses the same inline `emit_item` protocol as `flat_map`,
without introducing a mapper closure. `FlattenIterator` is exported from builtin;
`Iterator` remains available through the default prelude. The library regression
checks ordinary interpretation, joint expansion with no remaining handler/request
nodes, and LLVM O0/O3 execution. The sysroot iteration stage also checks the public
interface and resource cleanup without pasting library source into the test.
