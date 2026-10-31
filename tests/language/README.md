# Standalone Flurry language tests

Run the 26 source tests (20 ordinary tests and 6 template instances) with explicit resource protocols:

```sh
python3 -B development/check.py --timeout 240 -- run -p luna -- \
  tests/language/main.fl --test \
  --copy-protocol input::Duplicate --drop-protocol input::Retire
```

Add `--test-filter generic` to run the two generic-call tests only.
Add `--inline-functions` to exercise optional expansion before resource checking.
The suite covers generic calls, tuples and records, enum payloads, alternatives,
negation and guards, let-else, closures, defaults, trait dispatch, comptime
branch selection, quote calls, short-circuiting, pointer writes, loops, extended
arguments and observable destruction. Template instances cover imported setup,
explicit value and Type inputs, old-value postconditions, and destruction on
normal and early exits. Shared templates live in `fixtures.fl`. It declares the protocols it uses and does
not depend on unfinished sysroot APIs.

The CLI integration test runs both modes and mutates one predicate to verify
that a genuine failure is reported alongside the other 25 successes. It also
checks template-only discovery/filtering, two destructor failures in both modes,
and a malformed template body that must still fail compilation when its name is
excluded by the test filter:

```sh
python3 -B development/check.py --timeout 240 -- \
  test -p luna --test frontend_cli repository_language_suite
```

This suite is a small source-level integration set, not complete language
coverage. Compiler crates retain their more extensive behavior and rejection
tests. See [the test runner interface](../../development/source-tests.md).
