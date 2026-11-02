# Declaration-domain substitution cost

`substituted_trait_domain` prepares a declared input domain under the current
alias environment. It must preserve declaration identities, dependent value
domains, inherited inputs and Self substitution. Associated-type observation
still happens after substitution; an empty environment is not permission to
skip validation or return an unexamined domain.

The previous implementation constructed every declaration's complete binder
list before checking whether any input appeared in the requested domain or the
environment. Computing each binder's sort searched the declaration-domain table
again. Thus ordinary references to Type parameters repeatedly incurred nested
whole-package scans, including when there were no substitution inputs at all.

The new path sends empty environments directly to the existing checked
substitution operation. For nonempty environments it retains declaration order
and identity deduplication, but selects referenced, supplied identities before
obtaining their sorts. Value copying, budget accounting, checked substitution
and subsequent associated-type observation are unchanged. No invocation result
cache, package-specific rule or extra inference premise is introduced.

## Measurement setup

Baseline: `cd892033`, managed dev profile. Retained executables were built with
`development/build_host_driver.py`, using the same build profile and library
sources. The workload is `fn run() -> bool { true }` with the real sysroot and explicit
x86_64/Linux/64 target:

```sh
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-query-cost-baseline -- check /tmp/luna-sysroot-cost.fl --sysroot library --build-target x86_64 linux 64
python3 -B development/reproduce.py --timeout 120 --binary /tmp/luna-query-cost-fixed -- check /tmp/luna-sysroot-cost.fl --sysroot library --build-target x86_64 linux 64
```

Initial successful runs took 41.024 seconds before and 3.111 seconds after
(`/tmp/luna-tools-h84pgzz5/result.json` and
`/tmp/luna-tools-afprxmqt/result.json`, both exit 0). These are diagnostic dev
measurements, not a release throughput claim or a statistically isolated
benchmark. A temporary instrumented run took 40.95 seconds and located the bulk
of the cost in type annotation processing. All instrumentation was removed.

After the test processes finished, a sequential rerun of the same binaries and
arguments took **40.620 seconds before and 3.026 seconds after**, about 13.4x
faster for this workload. Child CPU time was 40.524 and 3.017 seconds respectively.
Both exited 0 (`/tmp/luna-tools-iv76900p/result.json`, rows in `stdout.log`). This
rerun had no concurrent task builds/tests; it remains a local dev measurement,
not a claim about every program or release compilation.

HIR emission alone took 1.304 seconds. The first targetless `check` took 37.434
seconds but rejected the sysroot because integer runtime proxies require target
pointer-width information; it is not a successful compilation measurement.

## Semantic regression coverage

From `/home/fin/luna`:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test trait_domain_substitution --test nominal_value_parameters --test newtypes --no-fail-fast -- --test-threads=1
```

All 520 cases passed (`/tmp/luna-tools-jf0dntfk/result.json`, exit 0).
The added case places a dependent, inherited trait domain and its default in a
world with 256 unrelated generic functions. It checks distinct true/false
instances, default substitution, runtime delivery and rejection of a char value
in the selected bool domain. Existing suites cover Self-dependent domains,
associated types, value-indexed nominals, aliases and late defaults. The scale
case has no fragile elapsed-time assertion.

Real sysroot CLI checks also passed without changing their 15-second
per-invocation deadline:

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli -- sysroot_ --test-threads=1
python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli -- shipped_clone_protocol_preserves_noncopy_sources shipped_operator_protocols_execute_with_default_and_named_types --test-threads=1
```

The first run passed 32 cases (`/tmp/luna-tools-l4n176oj/result.json`, exit 0);
the second passed both cases (`/tmp/luna-tools-n4r4f_ig/result.json`, exit 0).
Together these include all 18 CLI cases that previously timed out on the
baseline. Coverage includes real List/Tuple bindings, both math phases, half
float interpretation and LLVM O0/O3 execution, and Copy/Clone/operator
protocols. The five separate diagnostic/fixture failures documented during the
query-ownership work are not addressed or claimed fixed by this optimization.

Workspace format checking passed (`/tmp/luna-tools-wzv1gph7/result.json`, exit
0); `git diff --check` passed. Temporary timing instrumentation is absent from
the delivered implementation.

Remaining parameter-domain lookup and nonempty-environment enumeration still
use declaration tables. This change eliminates repeated construction of all
unrelated binders; it does not provide a persistent dependency-indexed query
system or complete all outstanding elaboration and sysroot functionality.
