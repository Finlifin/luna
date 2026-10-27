# Complete sysroot loading

The compiled library tree now contains implemented interfaces rather than
unresolved historical prototypes. Panic-dependent optional/error extraction,
String-returning Display/Debug, and legacy collections/formatting/I/O drafts
are preserved in `development/library-prototypes/`. Their missing dependencies
and reintroduction requirements are listed there. No panic stub, fake String,
allocation placeholder, or legacy intrinsic shim was added to production.

The builtin root/prelude exports implemented optional/error-result operations,
resource protocols, Hash, conversion/default traits, iteration and arithmetic.
std retains its builtin root re-export and explicitly imported builtin prelude.
Owned strings, containers, formatting, I/O and panic integration remain required
work; their former unfinished names are no longer advertised by the sysroot.

The isolation implements stage B of the accepted builtin-package plan. It does
not complete the plan's std stage E. Historical success-path unwrap tests now
identify their draft source explicitly and still make no panic-support claim.
The Hash fixture no longer injects a fake String merely to load builtin traits.

Complete loading exposed a separate coherence budget defect, fixed in commit
`3a9fd7db`; details and compiler checks are in `coherence-budget.md`.

## Validation

Working directory: `/home/fin/luna`.

```text
python3 -B development/check.py --timeout 900 -- test --release -p luna --test sysroot_preludes
```

Report `/tmp/luna-tools-yz10__wc/result.json`: exit 0, all five tests passed.
The new test loads the actual repository library tree, with no replacement
providers, and runs exactly one native test combining Tuple, optional operations,
math.sqrt and ordinary for. A first fixture attempt repeated a consumed Once
optional; the final fixture uses separate query/iteration values, preserving
ownership checks.

```text
python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/repros/ordinary-for-inline-effect.fl --sysroot /home/fin/luna/library --test-native --inline-functions --copy-protocol builtin.resources.Copy --drop-protocol builtin.resources.Drop --fuel 4000000
```

Report `/tmp/luna-tools-v7cemn04/result.json`: exit 0, `(count 1) (failures 0)`.
This closes the full-library name-resolution blocker recorded in
`iterator-protocol.md`, using the native runner to supply target layout facts.
Target-free interpretation is not thereby granted host pointer-width knowledge.

The mixed compiler/library check in `/tmp/luna-tools-cwhb0hzp/result.json`
passed all nine library_hash tests (including native/target-sized paths), plus
the compiler suites recorded separately. Its overall exit 101 was the corrected
Once fixture above, not a Hash failure.

```text
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli shipped_
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli draft_result_unwrap_executes_from_an_importing_package
```

Reports `/tmp/luna-tools-1fxffaa_/result.json` and
`/tmp/luna-tools-_vbmo9gf/result.json`: exit 0, six and one tests passed.
An initial migration run found an additional dynamically constructed
option_unwrap path; it now points to the archive and labels the test as a draft
test. The shipped syntax list only enumerates actual shipped files.

Formatting check passed (`/tmp/luna-tools-5gxf5nsx/result.json`, exit 0), as did
`git diff --check`. A byte comparison against HEAD confirmed all six archived
whole-file drafts are unchanged; the formatting trait fragment is preserved
separately. No full-workspace test run was performed.
