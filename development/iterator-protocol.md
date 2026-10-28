# Source-bound inline iteration

The driver now accepts `--iterator-protocol package.module.Trait`. Package
compilation and the `iterator-protocol` sysroot configuration key use the same
declaration resolver. Explicit selectors override configuration; an identity
already supplied by the embedding driver suppresses the configuration default.
The shipped sysroot binds `builtin.iterator.Iterator` by default.

Elaboration checks the selected trait before checking consumer bodies:

- A parameter-free trait declares one Type-valued `Item`.
- Runtime `iterate(self)` consumes its sole receiver and has no method inputs.
- Its result is a void computation with exactly one delivery operation.
- That operation is a direct inline effect taking one `Self.Item` and returning
  void. Its declaration comes from the computation signature, not spelling.

The actual builtin Iterator passes this check. An unrelated trait named
Iterator cannot replace an explicitly selected differently named trait. A
renamed delivery effect works, while wrong item types, additional delivery
arguments, reference receivers, non-computation results and ordinary effects
are rejected. Term inspection is bounded before recursive comparison; effect
formation uses the existing checked declaration-instantiation path.

Ordinary `for` now lowers through this contract. It evaluates its iterable once
before installing the delivery handler, selects `iterate` through ordinary
trait evidence, and retains ordinary call preparation and interface contracts.
The handler matches the delivered argument directly: success enters the current
iteration, and a catch-all branch retires a rejected item. Guards retain the
existing observation and transactional-delivery restrictions.

Each successful delivery gets its own loop scope. A source `continue` becomes
an exit from that scope, while `break` targets the enclosing producer scope and
`return` retains the lexical function target. Both normal and labelled nested
transfers pass execution tests. Generated control/cleanup sites receive distinct
source identities with provenance; existing NIR Display handles the result.

IntoIterator container conversion is now implemented as described below. NIR resume does
not yet encode an explicit lexical target: an outer handler's resume is rejected
inside a generated delivery handler rather than accidentally resuming delivery.
A nested explicit source handler inside the loop can still resume itself.
Pointer-bearing lexical captures can require joint expansion before resource
verification; unexpanded reference execution reports its missing memory contract.

## Binding-stage validation

Working directory: `/home/fin/luna`.

```text
python3 -B development/check.py --timeout 900 -- test --release -p luna --test iterator_protocol --test library_iteration --lib
```

The first run passed all 80 library unit tests, including configuration
precedence. New fixtures then failed because an effect-qualified result needed
a type alias and the repository iterator needed target pointer-width facts.
Those test inputs were corrected. Report:
`/tmp/luna-tools-f9n44340/result.json` (exit 101).

```text
python3 -B development/check.py --timeout 900 -- test --release -p luna --test iterator_protocol --test library_iteration
```

Passed: 4 protocol tests and 33 existing iterator tests, including affine
cleanup, short-circuiting, nested adapters and joint expansion. Report:
`/tmp/luna-tools-ha7c8wnl/result.json` (exit 0).

```text
python3 -B development/check.py --timeout 900 -- check --release -p luna --tests
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

The all-test-target compile initially found a preexisting reference to the
removed `Selectors.optional` field in normal_patterns (report
`/tmp/luna-tools-04hq6tav/result.json`, exit 101). Commit `0bc8bf23` fixes that
test separately; its focused depth/recovery test passed. The repeated
all-test-target check then passed, report
`/tmp/luna-tools-ey_neq2c/result.json` (exit 0). This compiles the test targets;
it does not claim a full-suite execution. Formatting passed, report
`/tmp/luna-tools-ni8ggt0g/result.json` (exit 0). Existing warnings remain.

## Ordinary-for execution validation

Working directory: `/home/fin/luna`.

```text
python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_iteration --test iteration --test iterator_protocol --test sysroot_preludes
```

Final report `/tmp/luna-tools-eblidfdu/result.json`: exit 0, 53 tests passed
(39 library iteration, 4 syntax/control identity, 6 protocol, 4 sysroot).
The six new library loop cases exercise checked reference execution after joint
expansion and native LLVM execution at O0 and O3. They cover failed patterns,
empty input, guards, nested labelled continue/break, lexical returns, generic
trait evidence, outer effect routing during iterable construction, and affine
cleanup of rejected/current/unproduced items. The affine capture case also
asserts the explicit memory-contract rejection of unexpanded execution.
The CLI sysroot test compiles and executes exactly one source test using the
repository iterator and resource modules, automatic prelude import, configuration
binding and native inline expansion.

`python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
passed, report `/tmp/luna-tools-fkuhwdar/result.json` (exit 0). `git diff --check`
also passed. No full-workspace test execution was needed for this scoped change.

Intermediate runs exposed and repaired synthetic cleanup-site collisions and
rejected-item temporary storage mapping. The final lowering reuses handler
argument matching instead of introducing an extra LetPattern temporary. Fixture
corrections use Flurry do-blocks, checked arithmetic bounds and automatic file
modules; no external guard mutation was permitted to bypass observation checks.

### Historical full sysroot blocker

```text
python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/repros/ordinary-for-inline-effect.fl --sysroot /home/fin/luna/library --interpret run --inline-functions --copy-protocol builtin.resources.Copy --drop-protocol builtin.resources.Drop --fuel 4000000
```

Report `/tmp/luna-tools-_mrfvo79/result.json`: exit 1 at package resolution.
Existing builtin option_unwrap/result_unwrap reference unresolved panic; traits
references String; std collections/fmt/io reference unresolved String/Any and
legacy intrinsic names. These files were not changed by this iteration stage.
The repro is verified by the minimal sysroot integration fixture above, but the
complete shipped library must still have these independent resolution defects
repaired before that exact full-sysroot command can pass.

Subsequent [sysroot integration](sysroot-integration.md) moved unfinished
prototypes out of the compiled tree and repaired coherence budget allocation.
The same repro now passes against the complete repository sysroot through
`--test-native`, which also supplies the required target layout facts.

## IntoIterator conversion

The source-bound `IntoIterator` contract has one Type-valued `Iter` and a runtime,
by-value `into_iter(self) -> Iter`. Its driver identity is selected with
`--into-iterator-protocol`; the shipped configuration binds
`builtin.iterator.IntoIterator`, exported by builtin and its prelude.

Loop lowering checks declared Iterator evidence first, then declared IntoIterator
evidence. This fixes the operation in a generic body rather than changing it
when a concrete instantiation also implements Iterator. Without declared evidence,
a concrete Iterator query must select an implementation or establish absence;
conflict/deferred results are not silently retried as conversions. Absence permits
one ordinary IntoIterator call. Its result must independently satisfy Iterator.
Generic container loops explicitly require both the conversion and Iterator
membership of their associated Iter.

The source expression and conversion execute once before the generated receiver,
loop target and delivery handler. The existing ordinary-call path checks method
contracts and transfers ownership. Container residuals retire normally; the
converted iterator then uses the existing delivery and loop cleanup machinery.
No new NIR node or alternate effect ABI is introduced.

### Conversion validation

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test iterator_protocol --test library_iteration --test sysroot_preludes
python3 -B development/check.py --timeout 900 -- check --release -p luna --tests
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
git diff --check
```

The execution command passed all 54 tests (8 protocol, 41 iteration, 5 sysroot),
report `/tmp/luna-tools-_sptv96s/result.json`, exit 0. Conversion cases cover
single evaluation, cleanup of container residuals before consumption, empty
iterators, direct-implementation precedence, and stable generic conversion when
the actual input also implements Iterator. The library harness checks interpreted
execution and LLVM O0/O3; the full-sysroot native fixture also uses a non-inline
conversion method. Rejection cases cover malformed protocol shape and a converted
value without Iterator evidence. No recursive conversion is attempted.

Formatting passed with report `/tmp/luna-tools-73pbu4r_/result.json`, exit 0;
`git diff --check` also passed.
All luna test targets compile successfully; check report
`/tmp/luna-tools-fedzux28/result.json`, exit 0. This compiles the updated public
Options consumers without executing the entire workspace test suite.

## First-item reduction

`Iterator.reduce(f: %c fn(Item, Item) -> Item) -> ?Item` consumes the iterator
through its inline delivery effect. The first item seeds the accumulator; only
subsequent items call `f`, in source order. Empty input returns null, singleton
input returns its item without calling `f`, and nested optional payloads retain
their layers. Each callback receives ownership of both its accumulator and the
new item. No identity value, Copy or Clone requirement is introduced.

```flurry
let first: ?bool = true;
let second: ?bool = false;
let result = first.as(Iterator).chain(second).as(Iterator)
    .reduce(|left, right| left and right);
let correct = result matches false?;
assumes correct;
```

The method follows the existing joint inline expansion path. The unexpanded
reference interpreter still rejects borrowed accumulator move/reinitialization
without an environment contract, as with fold. Pointer-bearing replacement
additionally requires an environment writeback contract in that unexpanded
path. These rejections remain tested; expansion does not erase resource checks.
Expanded tests verify callback order, absence and singleton behavior, present
null payloads, affine accumulator cleanup, and repeated capture-consumption
rejection. Native sysroot tests also verify retirement of an unused callback's
capture. Optional result matches in the examples are ordinary expressions;
`assumes` then checks the resulting bool without demanding unsupported observed
optional matching.

The complete sysroot iteration stage now has six native tests. Its preparation
budget is 1000000: the old 200000 budget could leave the new reductions
unexpanded while traversing the complete sysroot. This changes the test's
budget, not the memory checks or semantics.

Validation (working directory `/home/fin/luna`, unless noted):

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_iteration`:
  44 passed; `/tmp/luna-tools-flqr5iny/result.json`. New reduction cases check
  expanded resource-aware interpretation and native O0/O3 execution, while
  asserting the documented unexpanded refusals.
- After adding reduction to the repeated-capture rejection group,
  `python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_iteration repository_iterator_rejects_repeated_consumption_of_callback_capture`:
  passed; `/tmp/luna-tools-g_zsel_9/result.json`. The producer emits three items
  so reduction must invoke its callback twice.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-iteration`:
  six passed; `/tmp/luna-library-36aa78f3/summary.json`.
- `python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-dev-002/target/release/luna -- development/examples/sysroot-iteration.fl --sysroot library --test-native --inline-functions --fuel 1000000 --opt-level 3`:
  six passed; `/tmp/luna-tools-5xul652r/result.json`.
- From `/home/fin/luna/development`, `python3 -B -m unittest test_library_check`:
  all 13 runner tests passed with the updated required native count.
