# Source-bound inline iteration

The driver now accepts `--iterator-protocol package.module.Trait`. Package
compilation and the `iterator-protocol` sysroot configuration key use the same
declaration resolver. Explicit selectors override configuration; an identity
already supplied by the embedding driver suppresses the configuration default.
The shipped sysroot does not enable this binding by default yet.

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

This is the binding and validation prerequisite for ordinary `for`, not its
executable lowering. Consumer lowering, per-item pattern/guard scopes, loop
transfers and cleanup still need to be connected to joint inline expansion.
IntoIterator conversion is not implemented here. No pull-based `next` ABI,
optional result encoding, or new IR representation is introduced.

## Validation

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
