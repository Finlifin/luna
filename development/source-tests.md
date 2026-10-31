# Ordinary source tests

The source test driver discovers ordinary `test name { ... }` and `test { ... }`
declarations in the selected primary package, including its modules:

```flurry
fn ready() -> bool { true }

test readiness {
    let result = ready();
    assumes result;
}

test {
    assumes true;
}
```

```sh
luna input.fl --test --fuel 100000
```

Tests have independent declaration identities and no parameters. Their bodies
are checked as ordinary runtime functions returning void. A test label does not
introduce a callable source name or conflict with a function of the same name.
HIR marks test functions; NIR includes explicit `(test-entry ...)` identities.
The output is an S-expression test report containing passed declarations and a
count and failure count. Unnamed tests are reported as `anonymous` with their unique identities.

`--test` uses reference execution, with a fresh interpreter for each test.
`--fuel` has the same per-stage/per-query meaning as `--interpret`, applied to
each test. Execution failures, rejected predicates and exhausted per-test budgets
are recorded with their labels and identities; remaining tests still run with
fresh interpreters and budgets. Any failure makes the command exit with status 1
and sends the complete report to stderr; stdout and existing output files are
left untouched. A successful report goes to stdout or `--output`. No tests produces an explicit count of zero.

`--copy-protocol` and `--drop-protocol` together enable the same resource checks
and cleanup as `--interpret`. `--inline-functions` requires both. Without them,
reference execution does not constitute a resource-safety check. Source protocol
selector options are available. `--packages-v1 ... --test` executes only tests in
the designated primary package; supplied dependency sources are still compiled.

Tests inside runtime blocks are diagnosed rather than assigned an unscanned identity.
The parser now preserves `template test name where ... { ... }` and
`test name use expression { ... }` as separate AST forms. Template definitions
retain their contextual clause inputs, body and splice in HIR, marked as
`test-template` rather than a runnable test. Instances retain their selected template expression, fresh lexical context ports
and body in a dedicated HIR node. Declaration scanning makes explicit contextual
interfaces available across forward references, qualified paths, imports and
source modules; only the declared ports are imported into the instance body.
After all files are lowered, instances link to the actual body parameter and
contextual annotation of their selected declaration. Each instance port records
its corresponding formal binding; interface types retain declaration-scope
resolution even when the caller defines a same-named type.
A direct splice of the template body parameter now records missing port
substitutions using visible local identities at that splice site. Explicit
substitutions take precedence; global definitions and later locals cannot
supply a missing port. Explicit entries must name declared, distinct ports and
refer to local bindings; a global declaration cannot supply a port. Duplicate
context names are rejected even without an instance. Nested scopes select the
innermost local and do not leak bindings after the scope closes. This applies
only to declared test-template body ports.
Port types and substitutions are checked through the existing CMTT expression-code
pipeline. Instances expand into ordinary zero-argument runtime test functions and
retain discovery order alongside ordinary tests. Setup locals are fresh for each
instance; the checked runner verifies and cleans the combined body, including
optional whole-package inlining. Unused definitions still have their setup checked.
The contextual body result follows its declared interface; the generated test
function itself returns void.

```flurry
typealias V = .void'builtin;
template test fresh_flag where body: < flag: *bool |- V > {
    var value = false;
    let flag = value.ref;
    ~<body>;
    assumes value;
}
test changes_flag use fresh_flag {
    assumes not flag.*;
    flag.* = true;
}
```

The executable subset requires one anchored body clause parameter and an explicit
contextual interface. Additional optional clauses support checked defaults and
explicit named application inputs. Defaults are checked in declaration order;
later defaults see earlier actual inputs. Explicit inputs use call-site definitions
and do not invoke the defaults they replace. Unused templates check defaults
without invoking them. Type inputs specialize the contextual body interface.
Supported program `requires` and `ensures` predicates are retained on generated
test functions. Type prerequisites are proved at instantiation using the same PTC and checked
implementation-world path as ordinary declarations; program
predicates remain in the program verification layer. Unprovable postconditions
are diagnosed on normal and early exits. A template body's `return` targets the
generated test; ordinary quotes nested inside that body retain independent
control boundaries. Postconditions share the ordinary function old-value checking
scope; immutable comptime clause values can use `old` without runtime snapshots.
`old` remains unavailable in preconditions, setup and instance bodies. Variadic
collection supports entrances with established tuple layouts or the bound
meta.List<Type>; general meta.List<T> value collection, ambiguous positional routing and
decreases/outcome specifications remain unimplemented. Aliased
or computed body interfaces are not yet available to lexical port discovery.
Function templates, parallel scheduling, native execution and an assertion/reporting
library remain unimplemented. `assumes` is the existing runtime-checked program
predicate, not a new test API.

Rust callers can use `luna::testing::run_report` to inspect structured per-test
results, including mixed successes and failures. Invalid driver configuration or
inconsistent test identity metadata is rejected before running tests.

A standalone checked integration suite lives in [tests/language](../tests/language/README.md),
with both ordinary and optional-inline execution coverage.

Use `--test-filter <text>` with `--test` to run only labels containing the given
case-sensitive substring. Duplicate labels select every matching declaration;
`anonymous` selects unnamed tests. An empty filter selects all tests. No match
is a successful report with count zero and an explicit `filtered-out` count.
Filtering happens after compilation and metadata validation, so invalid source
in an unselected test still fails compilation. The report's filtered count only
includes tests in the primary package. Rust callers can use
`run_report_filtered` for the same selection behavior.

```sh
luna tests/language/main.fl --test --test-filter generic \
  --copy-protocol input::Duplicate --drop-protocol input::Retire
```

Checked test runs prepare the package's Copy/Drop context and optional whole-package
inlining once, on the first selected test. Tests invoke their declaration identities
directly. Each test still acquires its own instance graph and cleanup schedule and
receives a fresh interpreter and full per-stage budget; a failure cannot consume the
next test's execution budget. Preparation failures are reported for each selected
test without retrying identical preparation. Discovery indexes function declarations
once and preserves the source test list's order. This does not make the full runner
linear: per-entry instantiation and resource verification still perform their own work.

Reference runs likewise share their immutable runtime type context and implementation
index, and invoke test entries by declaration ID. Duplicate diagnostic names never
select a different body or make identity-based test execution ambiguous. Named
`--interpret` entry selection retains its ordinary ambiguity checks. Both modes
initialize shared preparation lazily, so a filter selecting no tests executes none
and does not initialize an interpreter or acquire an instance graph.

`--list-tests` compiles and validates tests, then emits their identities and labels
without running their bodies. It accepts the same `--test-filter` and explicit
package selection as `--test`, and cannot be combined with execution actions,
`--emit`, or `--inline-functions`. Compile-time work required to elaborate the
source still occurs; this is not a syntax-only scan. Output uses the ordinary
protected `--output` path or stdout:

```sh
luna tests/language/main.fl --list-tests --test-filter generic
```

The output is `(test-inventory (test (declaration ...) (name (string "..."))) ...
(count N) (filtered-out M))`. Entries preserve test order and duplicate labels
retain distinct declaration identities. Listing reports no pass/fail status.
Rust clients use `luna::testing::discover` and inspect `TestInventory` directly.


Optional template inputs configure shared setup without consuming the body slot:

```flurry
typealias V = .void'builtin;
template test configured_flag
where body: < flag: bool |- V >, .initial: bool = false
{
    let flag = initial;
    ~<body>;
}
test default_flag use configured_flag { assumes not flag; }
test enabled_flag use configured_flag<.initial = true> { assumes flag; }
```

A duplicate or unknown named input is rejected, including attempts to supply the
anchored body by name. Each default and explicit input must match the declared
domain; template inputs remain comptime values, while exposed context ports are
runtime bindings in the generated test function.


Trait prerequisites can authorize template setup operations. Conjunctive bounds
such as `T :- Read + Tag` supply both interfaces; a disjunction supplies neither
branch individually to the generic body. Each actual input still has to establish
the whole prerequisite, including associated-type obligations. An unresolved
proof is diagnosed rather than interpreted as either satisfied or false.

Positive conjunctive trait prerequisites are prepared with the ordinary signature
readiness protocol before checking the abstract template contract. An associated
type bound may precede the trait bound that permits its projection, either in
separate requires clauses or in one conjunction. Missing prerequisites and
prerequisites present only under disjunction are still rejected. This temporary
abstract evidence is restored after definition checking; separate actual
instances select and prove their own implementation evidence.


Default checking also prepares trait evidence incrementally as each preceding
clause input becomes available. Thus `.U: Type = T.as(Source).Item` and value
defaults invoking constrained comptime methods can use a declared `T :- Source`
prerequisite. This does not permit a default to read a later input: only the
completed clause prefix participates in readiness. Actual invocations resolve
the selected implementation, and explicit named inputs skip the replaced default.


A template body's return cannot currently cross a closure or independent quote
at the splice site. Such insertion is diagnosed instead of retargeting the
return to the nested function. The bounded connection check visits the current
runtime scope and preserves returns owned by closures inside the body itself.
Bodies without an outer return can still be spliced into closures. Implementing
nonlocal return transport across these boundaries remains separate work.


Tuple variadic template inputs use element type contexts while collecting
positional arguments, so unsuffixed numbers use the declared element type.
Literal tuple spreads share ordinary normal-form literal expansion. A named
whole-tuple input can come from a comptime producer; mixing named and positional
supply for the same entrance is rejected. Collected values are available to
later defaults and may depend on preceding optional Type inputs. This initial
support also has a separate meta.List<Type> collection path.


Computed tuple spreads now evaluate the packet once before distributing its
completed values. They can interleave with positional inputs; the collector
checks element types and total arity under a bounded traversal. Literal spreads
retain contextual numeric typing, while computed fields retain their completed
types (for example, a u16 field is not silently retagged as u8). The collected
value receives the ordinary Tuple type, which is checked against the original
domain; structural layout evidence does not replace normal-form identity.


The bound `meta.List<Type>` entrance collects Type values into one TypeList,
including an empty collection. Literal and computed list spreads require that
same protocol domain; tuples are not treated as TypeLists. Whole-list named
inputs remain supported and cannot be mixed with positional supply. Normal-form
Type heads are retained as values rather than unfolded. General value lists and ambiguous positional allocation remain unimplemented.


Multiple collection entrances accept complete named packages. Positional inputs
and spreads are routed only when exactly one collection entrance remains without
a named package, independently of named-argument order. With no eligible entrance,
extra positional inputs are duplicate supply; with several, they are ambiguous.
Routing is validated before any supplied producer is invoked. Omitted TypeList
entrances collect empty lists; omitted nonempty tuple entrances fail arity checks.

## LLVM native tests

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-tests.fl --test-native --opt-level 3
```

`--test-native` compiles each selected test through the host LLVM pipeline and
executes it in a fresh process. Tests are selected by declaration identity, so
repeated labels do not merge tests. `--test-filter`, primary-package selection,
and paired `--copy-protocol` / `--drop-protocol` options work as for reference
tests. `--opt-level 0..3` selects LLVM optimization (default 0).

The CLI defaults to a 10-second execution deadline per native test, starting
after compilation and successful child launch. A transient `ETXTBSY` launch
failure is retried for at most 100 ms; other launch errors are reported directly.
`--test-timeout-ms <positive integer>` overrides this deadline
in milliseconds and is accepted only with `--test-native`, including package-set
commands. Zero, signed/decimal values, overflow and repeated options are rejected. A trap, nonzero exit, deadline expiry or compilation failure is
reported for that test; remaining selected tests continue. The native child is
reaped before its temporary workspace is removed. Child stdin is closed. Output is drained during execution; failure diagnostics
include the first 16 KiB of each of stdout and stderr, with control characters
escaped and truncation marked. Invalid UTF-8 is displayed lossily. Successful
test output is discarded. Capture does not wait for inherited output descriptors
to close after the direct child exits. A failed run exits 1, reports all
results on stderr and preserves an existing `--output` file.

`--fuel` limits compiler work, not native instructions. LLVM/compiler subprocess
time remains bounded by the outer `development/check.py` invocation. The
embedding API `testing::run_native_report_filtered` accepts an explicit runtime
deadline and Toolchain. Native tests retain current host feature limits; unsupported
programs are reported as failed tests. `--inline-functions` is supported with
paired Copy/Drop bindings. It lazily expands a source copy once after filtering,
then regenerates checked instances and cleanup schedules separately for each test
identity. The embedding API `run_native_report_filtered_with_inlining` exposes
this option; the existing API retains its non-inlining behavior. A filter selecting
no tests performs neither expansion nor LLVM execution. `--list-tests` remains
discovery-only and does not invoke LLVM.

## Native inlining across packages and failed preparation

The native inlining regression now executes two identically labelled primary-package
tests which import an inline effect and an inline function owning a destructor from
a dependency. Qualified Copy/Drop identities resolve to that dependency. The
dependency's failing test with the same label is excluded; selecting the dependency
as primary runs only that failing declaration. O0 and O3 both preserve these results.

An embedding-API regression uses the same immutable Package before and after a
failed low-budget expansion. Both selected declarations report preparation failure;
a subsequent normal-budget run succeeds with their original identities, and the
input Package dump is unchanged throughout. Filtering everything out avoids both
expansion and LLVM execution even with the low budget and a missing toolchain.
Missing protocol bindings still fail configuration validation.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli native_inline_tests_preserve_package_identity_and_resource_bindings`: exit 0, `/tmp/luna-tools-3db5r3s_/result.json`.
- `python3 -B development/check.py --timeout 300 -- test -p luna --test source_tests native_inline_preparation_failure_preserves_source_and_recovers`: exit 0, `/tmp/luna-tools-_suox9ea/result.json`.
- Workspace formatting: exit 0, `/tmp/luna-tools-il2oz0e4/result.json`.

These are additional integration/recovery checks, not new effect ABI support.
Changes remain uncommitted on dev; the full workspace test suite was not rerun.
