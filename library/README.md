# Library integration

Option now provides `unwrap_or_else(fallback)`; Result provides
`unwrap_or_else(fallback)` and `or_else(recover)`. These methods accept captured
closures through the existing `%c fn` protocol. They consume the input once;
the callback is invoked once for None/Err and is not invoked for Some/Ok.
An unused callback still releases its captured resources. Result's recovery
callback receives the error payload and may return a different error type;
these are ordinary enum operations, not changes to the language error ABI.

Run the repository implementations with:

```sh
python3 -B development/host_examples.py library-result library-option-callback --release-driver --compare-optimization --timeout 900
```

The examples check both branches, successful and unsuccessful recovery,
callback invocation count, and exactly-once cleanup of affine captured state.
They do not require panic or the incomplete owned String implementation.

The sysroot manifest now supplies LLVM scalar defaults as
`host-intrinsic.sqrt = math::target::Scalar::sqrt` (and one entry for each
supported operation). A selected sysroot no longer needs the full list of
`--host-intrinsic` arguments. Explicit CLI selections override the corresponding
operation only; driver-bound identities also take precedence. Unknown operations,
duplicate keys and malformed selectors fail before compilation. The normal
intrinsic-enum identity/signature checks still apply. Explicit `--packages-v1`
clients do not implicitly load a sysroot and continue to supply their bindings.
This configuration support does not resolve the incomplete modules listed below.


The real Tuple and meta.List modules can be used as separate packages:
`python3 -B development/host_examples.py library-tuple --release-driver --compare-optimization`.
The runner declares the Tuple package's dependency on meta and binds both normal-form
heads explicitly. Tuple now spells its Type domain with the builtin identity, so
loading that module does not require the entire builtin prelude. This preserves
its existing signature and opaque identity. The example consumes each tuple once;
it does not infer a Copy implementation from the component types.

`builtin/tuple_resources.fl` adds `Once` for `Tuple<...Es>` with one
`Es: meta.List<Type>` input. It belongs to the same builtin package as the resource
traits. The constructor in `tuple.fl` remains independently loadable: loading it
as another package does not also install the trait implementation. This does not
grant Clone or Copy, and field cleanup keeps its existing responsibilities.

Run the actual modules together in their trait-owning package with
`python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_protocols repository_tuple_once_native_preserves_affine_fields`.
This O0/O3 native test forwards empty, singleton and nested affine tuples through
`T :- resources.Once`; it checks evaluation count and field retirement order.
The source is `development/examples/host-library-tuple-once.fl`. Its modules are
assembled from the repository declarations by the test; it is not a standalone
sysroot or a successful full-library smoke check.

The repository library is not yet a working complete sysroot. Check the actual
files and default language-item bindings from the repository root with:

```sh
python3 -B development/library_check.py --release --timeout 900
```

The command runs the real library declaration tests, checks the repository
language-item manifest, executes the resource, comparison, math, Option and Result
native suites plus Tuple/meta.List tests (comparison, math, Option, Result
and Tuple use the actual resources package for Copy/Drop checks), then compiles and interprets
`development/examples/library-smoke.fl` against this directory. The smoke
program uses `?bool` and the real Option methods; it must return `(bool true)`.
Every stage uses the shared Cargo/cache wrapper with closed stdin, a deadline
and saved logs. `--release` applies to every Cargo stage and reuses the current
release cache; omitting it keeps the development profile. A JSON summary links each result. A failed stage does not skip
independent later checks, and any failure makes the command fail.

For focused changes, select stages explicitly:

```sh
python3 -B development/library_check.py --release --stage native-tuple
python3 -B development/library_check.py --release --stage native-math --stage manifest
```

`--stage` can be repeated; duplicate names execute once, in normal dependency
order. Reports and terminal output mark `scope: selected` and list the selected
stages. A successful selection is not a successful complete-library check.
The default still runs all ten stages, including the failing full sysroot.
Tuple tests cover empty, singleton, nested comptime type-list construction,
single evaluation and transfer of affine fields through nested destructuring.
The resource case verifies that each field is retired exactly once and in the
explicit consumption order, using the actual resources.Drop protocol.

`builtin/conversion.fl` contains the existing `Into<Target>` and `From<Source>`
protocols without formatting or allocation dependencies. `builtin.traits` still
re-exports both protocols, preserving the builtin/prelude paths. They are ordinary
parameterized traits: selecting `Into<bool>` does not establish `Into<u16>`, and
calling `into` consumes its receiver under the normal resource rules. There is no
automatic blanket conversion or new implicit-conversion rule.

`python3 -B development/library_check.py --release --stage native-conversion`
runs three source tests against this module and the real resources protocols:
generic From/Into round-trip with one input evaluation, distinct target interface
selection, and affine field transfer with exactly one cleanup. The matching
`library-conversion` host example also runs under O0/O3. Declaration tests exercise
the interpreter and reject an absent target implementation and a consumed receiver.

The declaration suite also lexes and parses every actual library `.fl` file,
so keyword additions and grammar changes cannot silently break unexecuted
modules. This syntax check is separate from name resolution and execution.

The declaration tests cover the binary operator protocols, their distinct RHS
and associated output types, the default RHS type, and unary results. The Index
check also executes the real Index/IndexMut declarations, checks pointer results
and explicit writeback, and verifies receiver/key/method/RHS evaluation order.
IndexMut implementations may retain the parent associated projection or use its
proved concrete/generic result type. Tests reverse parent/child declaration order
and reject mismatched outputs and opaque normal-form substitutions. These tests
do not establish that the whole sysroot can compile. Do not replace library files
with fixture copies or supply placeholder runtime functions to make the smoke
program pass.

Current full-sysroot blockers include unresolved `panic`, `String`, formatting
`Any`, allocation/collection and I/O `__builtin_*` calls, and map-storage hooks.
`panic` also needs an explicit termination/cleanup contract before its runtime
implementation can be chosen. The smoke check remains failing until these
real dependencies are implemented. `language-items.conf` binds the actual
Option type; the historical commented language-item annotations in source do
not perform compiler binding.

The manifest also binds `builtin::resources::Copy` and `builtin::resources::Drop`.
Execution selects resource checking from the resolved identities, including
manifest defaults, rather than only from explicit command-line arguments.
This applies to interpretation, native emission/builds and source tests;
explicit selections still override defaults. Ownership-only analysis retains
its explicit action scope. These bindings do not repair the unresolved modules
listed above or make the full sysroot smoke program pass.

`--inline-functions` and `--discard-result` can use the same sysroot defaults;
they no longer require repeating the pair on the command line. Naming a sysroot
does not itself supply either protocol: missing or incomplete bindings fail
before execution/publication. With neither binding and neither special option,
the existing unchecked developer execution path remains available.

## Standalone Option value operations

`builtin/option.fl` contains the real Option definition and its `is_some`,
`is_none`, `unwrap_or`, `map`, and `and_then` methods. The panic-dependent
`unwrap` implementation lives in `builtin/option_unwrap.fl`; the full builtin
package still includes it and still requires a real panic implementation.
No method or failure contract has been replaced with a placeholder.

Run `python3 -B development/host_examples.py library-option --release-driver --compare-optimization`
to load Option and the real resource protocols as separate packages. The runner
binds `?T` to that Option declaration and checks both LLVM O0/O3 outputs.
Library tests additionally cover affine payloads: unused `unwrap_or` fallbacks
are retired, `map` consumes the selected payload once, and `and_then` preserves
ownership of its returned payload until the caller consumes it. Query methods
take `self` by value and can therefore retire a non-Copy payload.

Option's `map`/`and_then` and Result's `map`/`map_err`/`and_then` accept
`%c fn(...)` callbacks, with `c: Type.vars.Closure`. The context remains part of
the interface; captured environments are neither erased nor implicitly boxed.
Ordinary functions infer the empty context. A non-Copy environment is moved
into the combinator and retired even on a branch that does not invoke it;
captures consumed by an invocation are not retired again.
Calls can provide a positional declaration-argument prefix such as `map<bool>`
and infer the remaining context from the callback. Supplied arguments stay
fixed; a conflicting callback is rejected. This also works through generic
trait methods and ordinary functions. The current partial-prefix path excludes
declarations with optional clause parameters and named/spread declaration
arguments; those still use their existing complete routing rules. An unresolved
partial instance cannot be stored as a function value without completing it.

Run `python3 -B development/host_examples.py library-option-callback --release-driver --compare-optimization`
for a callback owning non-Copy state, tested on invoked and uninvoked branches.

## Standalone Result value operations

`builtin/result.fl` provides the ordinary library enum Result and its `is_ok`,
`is_err`, `map`, `map_err`, and `and_then` methods. It is distinct from the
language error-result ABI. The original panic-dependent `unwrap` and
`unwrap_err` methods remain in `builtin/result_unwrap.fl`, included by the full
builtin package.

Run `python3 -B development/host_examples.py library-result --release-driver --compare-optimization`
for the real module with resource checks. Library regressions cover both
non-Copy payload branches: map only consumes an Ok payload, map_err only
consumes an Err payload, and and_then preserves ownership when a callback
returns an Err payload. By-value queries retire whichever payload is present.

## Standalone comparison protocols

`builtin/comparison.fl` owns Eq, Ord and Order; `builtin/traits.fl` re-exports
the same declarations. It has no String, formatting, allocation or panic dependency.
Concrete Eq/Ord implementations cover bool (false precedes true) and the ten
fixed-width integer types u8 through u128 and i8 through i128, plus Unicode scalar
characters and target-sized usize/isize. Symbol implements Eq by name. Default ne/lt/le/gt/ge
methods use the declared protocol implementations. Executing platform-sized
comparisons requires an explicit target; declarations do not select a host width.
Floating implementations are not included; this is not a floating total-order API.

`--ord-protocol comparison::Ord` binds `<`, `<=`, `>`, and `>=` on user-defined
types to the protocol's `lt`, `le`, `gt`, and `ge` methods. Generic Ord bounds and
the library's default methods participate in ordinary method selection. Both
operands are evaluated once in source order and the result must be bool.
Native scalar comparisons keep their built-in operation, including IEEE NaN
behavior. The sysroot manifest supplies `builtin::comparison::Ord` by default;
explicit CLI/driver bindings take precedence.

`--eq-protocol comparison::Eq` similarly binds `==` and `!=` on user-defined
types to `eq` and `ne`, including generic Eq bounds and the default `ne` body.
An overridden `ne` is called directly. The sysroot manifest supplies
`builtin::comparison::Eq`. Results must be bool; operand order and early exits
follow ordinary calls. Type equality remains a type proof: an opaque normal-form
application is not unfolded, and an unknown relation is not turned into false
equality or true inequality. Native scalar and BitVec equality retain their
existing semantics.

Run `python3 -B development/host_examples.py custom-order --release-driver --compare-optimization`
for a Version record with lexicographic ordering, generic `<`, all six comparisons,
and resource-checked native execution using the real comparison/resources packages.

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- --packages-v1 \
  --package 1 101 app development/examples/host-library-comparison.fl \
  --package 2 202 comparison library/builtin/comparison.fl \
  --package 3 303 resources library/builtin/resources.fl \
  --dependency 1 comparison 2 --dependency 1 resources 3 --primary 1 \
  --copy-protocol resources::Copy --drop-protocol resources::Drop \
  --test-native --opt-level 3
```

Six tests exercise boolean order, signed 128-bit extrema, platform integers, Unicode
characters, symbolic names, default methods and a
generic `lower` constrained by Ord + Copy. The declaration suite additionally
executes every supported scalar's extrema with resource checks in the reference
interpreter and LLVM O0/O3. Splitting this dependency-free module does not make the
whole sysroot complete.

## Standalone host math

`math/lib.fl` exports `pure fn fma(a: T, b: T, c: T) -> T`,
`pure fn sqrt(value: T) -> T`, and
`pure fn copysign(magnitude: T, sign: T) -> T`, and the unary wrappers
`abs`, `floor`, `ceil`, `trunc`, `roundeven`, `round`, `sin`, `cos`, `exp`, `exp2`, `log`,
`log2`, `log10`, plus binary `pow(base, exponent)`, `minimum(a, b)`, `maximum(a, b)`, `minimumnum(a, b)` and `maximumnum(a, b)` for f32 and
f64, with the exact type requirement `T == f32 or T == f64`. Each generic
body wraps its explicitly bound LLVM operation. Its pending NIR signature is
specialized and validated before host admission; LLVM receives only a concrete
f32 or f64 operation. Both explicit type arguments and inference from typed
arguments work. Mixed precisions and non-float inputs are rejected.

`copysign` takes only the sign bit from its second input, retaining all other
bits of its first input, including NaN payload and quiet/signaling state. This
follows the [LLVM copysign contract](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-copysign-intrinsic).
Both arguments retain ordinary once-only, source-order evaluation.

Rounding results retain their input floating type. `floor` rounds toward negative
infinity, `ceil` toward positive infinity, `trunc` toward zero, and `roundeven` to
nearest with halfway cases resolved toward the even integer. `round` resolves
halfway cases away from zero, following [LLVM round](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-round-intrinsic). They preserve signed
zero and infinities; NaN inputs produce NaNs, without a payload-preservation
promise. `roundeven` is independent of the runtime rounding mode. These wrappers
follow the [LLVM rounding intrinsic contracts](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-roundeven-intrinsic),
and do not perform float-to-integer conversion.

The transcendental functions follow the corresponding
[LLVM math intrinsic contracts](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-sin-intrinsic).
`log` is the natural logarithm; `log2` and `log10` select their stated bases.
`pow` takes a floating exponent of the same type as its base. The backend does not
request approximate fast-math flags. Accuracy follows the target implementation;
these wrappers do not promise bit-identical transcendental results across targets,
NaN payload preservation, or compile-time evaluation.

This package can be built independently of the unfinished full sysroot:

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- --packages-v1 \
  --package 1 101 app development/examples/host-math-fma.fl \
  --package 2 202 math library/math/lib.fl \
  --dependency 1 math 2 --primary 1 \
  --host-intrinsic fma=math::target::Scalar::fma \
  --host-intrinsic sqrt=math::target::Scalar::sqrt \
  --host-intrinsic copysign=math::target::Scalar::copysign \
  --host-intrinsic floor=math::target::Scalar::floor \
  --host-intrinsic ceil=math::target::Scalar::ceil \
  --host-intrinsic trunc=math::target::Scalar::trunc \
  --host-intrinsic roundeven=math::target::Scalar::roundeven \
  --host-intrinsic round=math::target::Scalar::round \
  --host-intrinsic fabs=math::target::Scalar::fabs \
  --host-intrinsic sin=math::target::Scalar::sin \
  --host-intrinsic cos=math::target::Scalar::cos \
  --host-intrinsic exp=math::target::Scalar::exp \
  --host-intrinsic exp2=math::target::Scalar::exp2 \
  --host-intrinsic log=math::target::Scalar::log \
  --host-intrinsic log2=math::target::Scalar::log2 \
  --host-intrinsic log10=math::target::Scalar::log10 \
  --host-intrinsic minimum=math::target::Scalar::minimum \
  --host-intrinsic maximum=math::target::Scalar::maximum \
  --host-intrinsic minimumnum=math::target::Scalar::minimumnum \
  --host-intrinsic maximumnum=math::target::Scalar::maximumnum \
  --host-intrinsic isnan=math::target::Scalar::isnan \
  --host-intrinsic isinf=math::target::Scalar::isinf \
  --host-intrinsic isfinite=math::target::Scalar::isfinite \
  --host-intrinsic isnormal=math::target::Scalar::isnormal \
  --host-intrinsic issubnormal=math::target::Scalar::issubnormal \
  --host-intrinsic signbit=math::target::Scalar::signbit \
  --host-intrinsic add_sat=math::target::Scalar::add_sat \
  --host-intrinsic sub_sat=math::target::Scalar::sub_sat \
  --host-intrinsic min=math::target::Scalar::min \
  --host-intrinsic max=math::target::Scalar::max \
  --host-intrinsic abs=math::target::Scalar::abs \
  --host-intrinsic bitreverse=math::target::Scalar::bitreverse \
  --host-intrinsic ctpop=math::target::Scalar::ctpop \
  --host-intrinsic ctlz=math::target::Scalar::ctlz \
  --host-intrinsic cttz=math::target::Scalar::cttz \
  --host-intrinsic bswap=math::target::Scalar::bswap \
  --host-intrinsic fshl=math::target::Scalar::fshl \
  --host-intrinsic fshr=math::target::Scalar::fshr \
  --host-intrinsic pow=math::target::Scalar::pow --host-intrinsic powi=math::target::Scalar::powi --host-intrinsic ldexp=math::target::Scalar::ldexp --build run --output /tmp/flurry-math
python3 -B development/command.py --timeout 10 -- /tmp/flurry-math
```

The output is `(bool true)`, checking results at both widths. Native tests also
cover O0/O3 and paired Copy/Drop protocols. The math package is not currently
automatically loaded by the standalone sysroot driver; use the explicit package
input above. Its runtime intrinsic is not executable by the reference interpreter.
Other math functions, generic tag families and non-scalar intrinsic signatures remain unfinished.

The same explicit host binding is available to native source tests:

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- --packages-v1 \
  --package 1 101 app development/examples/host-math-tests.fl \
  --package 2 202 math library/math/lib.fl \
  --dependency 1 math 2 --primary 1 \
  --host-intrinsic fma=math::target::Scalar::fma \
  --host-intrinsic sqrt=math::target::Scalar::sqrt \
  --host-intrinsic copysign=math::target::Scalar::copysign \
  --host-intrinsic floor=math::target::Scalar::floor \
  --host-intrinsic ceil=math::target::Scalar::ceil \
  --host-intrinsic trunc=math::target::Scalar::trunc \
  --host-intrinsic roundeven=math::target::Scalar::roundeven \
  --host-intrinsic round=math::target::Scalar::round \
  --host-intrinsic fabs=math::target::Scalar::fabs \
  --host-intrinsic sin=math::target::Scalar::sin \
  --host-intrinsic cos=math::target::Scalar::cos \
  --host-intrinsic exp=math::target::Scalar::exp \
  --host-intrinsic exp2=math::target::Scalar::exp2 \
  --host-intrinsic log=math::target::Scalar::log \
  --host-intrinsic log2=math::target::Scalar::log2 \
  --host-intrinsic log10=math::target::Scalar::log10 \
  --host-intrinsic minimum=math::target::Scalar::minimum \
  --host-intrinsic maximum=math::target::Scalar::maximum \
  --host-intrinsic minimumnum=math::target::Scalar::minimumnum \
  --host-intrinsic maximumnum=math::target::Scalar::maximumnum \
  --host-intrinsic isnan=math::target::Scalar::isnan \
  --host-intrinsic isinf=math::target::Scalar::isinf \
  --host-intrinsic isfinite=math::target::Scalar::isfinite \
  --host-intrinsic isnormal=math::target::Scalar::isnormal \
  --host-intrinsic issubnormal=math::target::Scalar::issubnormal \
  --host-intrinsic signbit=math::target::Scalar::signbit \
  --host-intrinsic add_sat=math::target::Scalar::add_sat \
  --host-intrinsic sub_sat=math::target::Scalar::sub_sat \
  --host-intrinsic min=math::target::Scalar::min \
  --host-intrinsic max=math::target::Scalar::max \
  --host-intrinsic abs=math::target::Scalar::abs \
  --host-intrinsic bitreverse=math::target::Scalar::bitreverse \
  --host-intrinsic ctpop=math::target::Scalar::ctpop \
  --host-intrinsic ctlz=math::target::Scalar::ctlz \
  --host-intrinsic cttz=math::target::Scalar::cttz \
  --host-intrinsic bswap=math::target::Scalar::bswap \
  --host-intrinsic fshl=math::target::Scalar::fshl \
  --host-intrinsic fshr=math::target::Scalar::fshr \
  --host-intrinsic pow=math::target::Scalar::pow --host-intrinsic powi=math::target::Scalar::powi --host-intrinsic ldexp=math::target::Scalar::ldexp --test-native --opt-level 3
```

`--test-filter` and paired Copy/Drop bindings work with this path. Reference
`--test` does not execute target intrinsics and rejects host selectors.
Supply all thirty-eight bindings when loading this package: even unused generic declarations
require a checked target signature. The selectors are driver configuration, not
language-level recognition of operation names.

The repeatable form supports multiple independent declarations for each operation:

```sh
--host-intrinsic fma=math::target::Scalar::fma \
--host-intrinsic sqrt=math::target::Scalar::sqrt \
--host-intrinsic copysign=math::target::Scalar::copysign \
--host-intrinsic floor=math::target::Scalar::floor \
--host-intrinsic ceil=math::target::Scalar::ceil \
--host-intrinsic trunc=math::target::Scalar::trunc \
--host-intrinsic roundeven=math::target::Scalar::roundeven \
--host-intrinsic round=math::target::Scalar::round \
--host-intrinsic fabs=math::target::Scalar::fabs \
--host-intrinsic sin=math::target::Scalar::sin \
--host-intrinsic cos=math::target::Scalar::cos \
--host-intrinsic exp=math::target::Scalar::exp \
--host-intrinsic exp2=math::target::Scalar::exp2 \
--host-intrinsic log=math::target::Scalar::log \
--host-intrinsic log2=math::target::Scalar::log2 \
--host-intrinsic log10=math::target::Scalar::log10 \
--host-intrinsic minimum=math::target::Scalar::minimum \
--host-intrinsic maximum=math::target::Scalar::maximum \
--host-intrinsic minimumnum=math::target::Scalar::minimumnum \
--host-intrinsic maximumnum=math::target::Scalar::maximumnum \
  --host-intrinsic isnan=math::target::Scalar::isnan \
  --host-intrinsic isinf=math::target::Scalar::isinf \
  --host-intrinsic isfinite=math::target::Scalar::isfinite \
  --host-intrinsic isnormal=math::target::Scalar::isnormal \
  --host-intrinsic issubnormal=math::target::Scalar::issubnormal \
  --host-intrinsic signbit=math::target::Scalar::signbit \
  --host-intrinsic add_sat=math::target::Scalar::add_sat \
  --host-intrinsic sub_sat=math::target::Scalar::sub_sat \
  --host-intrinsic min=math::target::Scalar::min \
  --host-intrinsic max=math::target::Scalar::max \
  --host-intrinsic abs=math::target::Scalar::abs \
--host-intrinsic bitreverse=math::target::Scalar::bitreverse \
--host-intrinsic ctpop=math::target::Scalar::ctpop \
--host-intrinsic ctlz=math::target::Scalar::ctlz \
--host-intrinsic cttz=math::target::Scalar::cttz \
--host-intrinsic bswap=math::target::Scalar::bswap \
--host-intrinsic fshl=math::target::Scalar::fshl \
--host-intrinsic fshr=math::target::Scalar::fshr \
--host-intrinsic pow=math::target::Scalar::pow --host-intrinsic powi=math::target::Scalar::powi --host-intrinsic ldexp=math::target::Scalar::ldexp
```


These options work with single-file and explicit-package build, LLVM output,
and native tests. Repeating the same declaration/operation is idempotent,
including a matching legacy flag; assigning different operations to the same
resolved declaration is rejected. An operation can be bound to multiple tags
in different packages without merging their type identities.

## Resource protocols without formatting dependencies

`builtin/resources.fl` contains Once, Clone, Copy and Drop. `builtin/traits.fl`
re-exports them, preserving the existing prelude and qualified trait access.
Scalar Clone implementations read their receiver; ordinary pointer Clone keeps
the original target dependency. Copy delivery remains a representation copy and
does not invoke Clone. Fixed-width integers, f32/f64, bool and void have checked
execution coverage; usize/isize declarations are present but their host numeric
representation is still pending.

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- --packages-v1 \
  --package 1 101 app development/examples/host-library-resources.fl \
  --package 2 202 resources library/builtin/resources.fl \
  --dependency 1 resources 2 --primary 1 \
  --copy-protocol resources::Copy --drop-protocol resources::Drop \
  --test-native --opt-level 3
```

The four tests exercise scalar and pointer cloning, scope cleanup, and implicit
Copy versus explicit Clone. The library checker now runs this native suite and
the math native suite independently before attempting the full sysroot smoke
program. A working module does not make the full-sysroot check successful.

`development/repros/host-clone-result-pointer-provenance.fl` now passes: static
trait methods preserve returned aggregate pointer origins after implementation
selection. Both whole-value copies and field reconstruction retain the original
targets, including nested generic aggregates. Local escapes and access after
moving the owner remain rejected. Trait methods with program predicates still
require a composed memory contract; that separate limitation remains explicit.

`abs<T>` binds `fabs=math::target::Scalar::fabs` and lowers to LLVM fabs.
It clears the sign bit, including negative zero and infinity, and preserves NaN
payloads and quiet/signaling bits, following the [LLVM fabs contract](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-fabs-intrinsic).

`minimum(a, b)` and `maximum(a, b)` are same-width f32/f64 wrappers of
LLVM minimum/maximum: they propagate NaN, and order negative zero below positive
zero. They do not promise a particular NaN payload. See the
[LLVM 21 minimum contract](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-minimum-intrinsic).

`bit_and<T>(left, right)`, `bit_or<T>(left, right)` and `bit_xor<T>(left, right)`
combine fixed-width bit patterns, preserving the common input/output type.
They accept u8/u16/u32/u64/u128, i8/i16/i32/i64/i128, and usize/isize,
including negative signed values. Platform integer layouts come from the
explicit package target; usize/isize never become u64/i64 type identities. Both operands are evaluated exactly once in source order;
zero and all-one masks do not short-circuit. Bind the corresponding `bit_and`,
`bit_or`, and `bit_xor` tags in `math::target::Scalar`. These source target
intrinsics lower to LLVM `and`/`or`/`xor` instructions, not named LLVM functions.
BitVec is not admitted by these bindings and retains its own
`and`/`or`/`not` operations. A target-independent binding can retain a platform
scalar type, but layout-dependent verification and LLVM emission require an
explicit target width. For admitted intrinsic nodes, program
analysis computes exact results when both operand intervals are singletons,
and conservative scalar bounds for masks (for example `x and 15` is in 0..15).
Signed OR bounds preserve the sign bit. Unknown cases retain the full result
domain; these rules do not introduce a general bitvector solver or expose
unproved polynomial relations.

`bit_reverse<T>(value)` and `count_ones<T>(value)` accept u8/u16/u32/u64/u128/usize,
returning the same type. They wrap [LLVM bitreverse](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-bitreverse-intrinsic)
and [LLVM ctpop](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-ctpop-intrinsic).
Zero is valid for both. These bindings do not admit signed or BitVec types.
For usize, verification and LLVM emission use the explicitly selected target
pointer width; the source type remains usize.

`leading_zeros<T>(value)` and `trailing_zeros<T>(value)` use the same unsigned
integer domain and return type. Zero returns the input bit width. Their LLVM
[ctlz](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-ctlz-intrinsic) and
[cttz](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-cttz-intrinsic)
calls always pass `false` for the zero-is-poison flag; that flag is not a source
operand. Inputs are evaluated once before the intrinsic call.

`byte_swap<T>(value)` reverses byte order without reversing bits inside each byte,
returning the same u16/u32/u64/u128/usize type. Bind
`bswap=math::target::Scalar::bswap`. The domain follows the
[LLVM 21 bswap contract](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-bswap-intrinsics):
a positive multiple of 16 bits. For usize this restriction is checked against
the selected target, including after generic specialization; 8-, 24-, and other
odd-byte pointer widths are rejected. An unspecified layout remains deferred,
not silently inferred from the host. This wrapper does not admit u8, signed,
floating-point, or BitVec types. Its input is evaluated once.

`math.funnel_shift_left(a, b, amount)` and `funnel_shift_right` concatenate
`a` as the high half and `b` as the low half, shift by `amount` modulo the
bit width, and return the high/low half respectively. All three inputs and
the result have the same unsigned type (`u8` through `u128`, or `usize`).
`rotate_left(value, amount)` and `rotate_right` reuse the bound value for
both halves; argument expressions are evaluated once. Zero/full-width shifts
return `a` for left funnel shifts and `b` for right funnel shifts.
See the [LLVM funnel shift contract](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-fshl-intrinsic).

`minimumnum` and `maximumnum` wrap the corresponding LLVM intrinsics. A single NaN (quiet or signaling)
yields to the numeric operand; two NaNs yield NaN.
Signed zero is ordered negative before positive. No fast-math flags are added;
NaN payload preservation is not promised. These differ from `minimum` and
`maximum`, which propagate even a single quiet NaN. See the
[LLVM 21 minimumnum contract](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-minimumnum-intrinsic).

The current LLVM 21 host pipeline expands minimumNumber/maximumNumber to
`llvm.minimum`/`llvm.maximum` plus explicit single-NaN selection. This preserves
the signed-zero contract on the tested host, where directly lowering
`llvm.minimumnum` did not. Both ordinary arguments are evaluated once, in source
order; the expansion only uses their resulting values.

`isnan`, `isinf`, and `isfinite` accept either f32 or f64 and return bool. They wrap
[LLVM is.fpclass](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-is-fpclass-intrinsic)
with fixed masks: both NaN classes, both infinities, or all finite classes respectively.
Finite values include signed zero and subnormal values. These operations do not assume
fast-math or provide general static floating-point contract proofs. Bind the three
matching Scalar tags alongside the other math operations.

`isnormal` and `issubnormal` distinguish normal values from subnormals; both exclude zero.
`signbit` reports the sign of every bit pattern, including negative zero and negative
NaNs. These three functions accept f32/f64 and return bool. Bind their matching Scalar tags.

`saturating_add<T>(a, b)` and `saturating_sub<T>(a, b)` accept matching signed or
unsigned integer types of 8, 16, 32, 64 or 128 bits, or usize/isize, and return the same type. They
clamp the mathematical result to that type's bounds. The `add_sat`/`sub_sat`
tags lower to [LLVM saturation intrinsics](https://releases.llvm.org/21.1.0/docs/LangRef.html#saturation-arithmetic-intrinsics), selecting signedness from the checked source type.
Mixed widths or signedness are rejected. Ordinary arithmetic overflow rules are unchanged.
Run the multi-package example with `python3 -B development/host_examples.py saturating-integers --opt-level 3`.

`min<T>(a, b)` and `max<T>(a, b)` select the smaller/larger matching integer
(8, 16, 32, 64 or 128 bits, or usize/isize). Platform bounds use the selected target.
Signedness comes from the checked source type;
these wrap LLVM `smin`/`smax` or `umin`/`umax`, respectively. Both arguments are
always evaluated once in source order. Float operands use the separate
`minimum`/`maximum` or `minimumnum`/`maximumnum` interfaces.
See the [LLVM integer min/max contract](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-smin-intrinsic).
Run `python3 -B development/host_examples.py integer-minmax --compare-optimization`
for signed/unsigned 128-bit boundaries and a bounded clamp example.

### 有符号整数 wrapping_abs

`math.wrapping_abs<T>(value)` 接受 i8／i16／i32／i64／i128／isize，返回相同类型。负数取相反数；最小有符号整数保持原值，因此不能为全部输入承诺结果非负。输入按普通调用规则求值一次。现有浮点 `math.abs` 保持原接口。

该函数显式导出后置条件 `result == value'old or result == -value'old`。顺序有意义：最小整数先满足左侧，不会在右侧执行溢出的取负。定义在 requires 允许的全部类型上验证，调用者可通过公共契约取得结果关系，无须直接使用 intrinsic。泛型 isize 案例若尚未指定目标，则对 TargetData 允许的全部布局分别检查；证明不为包选择目标，具体执行仍要求布局。

包装通过 `--host-intrinsic abs=math::target::Scalar::abs` 绑定到 `llvm.abs`，固定传入 `is_int_min_poison=false`，遵循 [LLVM 21 abs 语义](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-abs-intrinsic)。无符号输入不在此包装的域内；目标 intrinsic 的 comptime invocation 仍不受支持。

运行 `python3 -B development/host_examples.py wrapping-abs --compare-optimization` 可检查跨包包装、普通实参求值次数、i32／i128 最小值和 O0/O3 一致性。

程序验证器在已绑定的 abs intrinsic 位置按输入区间推导结果区间。例如输入在 `[-40, 30]` 时结果在 `[0, 40]`；输入只含 INT_MIN 时结果仍是 INT_MIN，同时包含 INT_MIN 和其下一整数时必须保留完整有符号范围。此分析用于验证契约，不是执行 comptime intrinsic，也不自动为一般函数调用生成新的关系契约。

在 intrinsic 位置还保留结果与已求值输入的关系：非负输入的结果等于输入，排除 INT_MIN 的负区间结果等于输入的相反数；其余情况保留这两种候选。显式函数契约可据此证明并导出关系，例如在有限负区间内证明 `result + value'old == 0`。后续改写原变量不会改写已求值实参的身份。

The single-input sysroot loader discovers `math/lib.fl` when present, alongside
the existing optional packages. A present math directory without `lib.fl` is an
error; an absent package does not authorize an unresolved manifest selector.

## Integer exponentiation

`math.powi<T>(base: T, exponent: i32) -> T` supports f32 and f64. The
signed exponent is a distinct operand type, including negative exponents and
INT_MIN. Both operands follow ordinary single-evaluation, source-order rules.
The wrapper lowers to LLVM powi rather than converting the exponent to float.
LLVM leaves the multiplication/rounding sequence unspecified, so this wrapper
does not promise bitwise equality with pow or a particular repeated-multiply
algorithm. See [LLVM powi](https://releases.llvm.org/20.1.0/docs/LangRef.html#llvm-powi-intrinsic).

Run `python3 -B development/host_examples.py math-powi --release-driver --compare-optimization`.
The sysroot manifest includes powi; package-set clients supply
`--host-intrinsic powi=math::target::Scalar::powi` along with the other operations.

Aggregate summaries are published atomically. If summary publication fails,
the runner returns 125, prints the complete current evidence with
`summary_saved: false`, and stops before launching another stage. The old file
remains intact but is not the current terminal result. This infrastructure
failure differs from an ordinary failed test, after which independent stages
continue as before.

## Binary exponent scaling

`math.ldexp<T>(value: T, exponent: i32) -> T` wraps LLVM ldexp for f32/f64.
It scales the input directly by a power of two; it does not first construct a
floating power and multiply. That distinction matters when the intermediate
power would overflow even though the scaled result is finite. Source operands
are evaluated once, in order. The host uses its existing default floating-point
environment and does not enable fast-math or flush-to-zero flags.

`python3 -B development/host_examples.py math-ldexp --release-driver --compare-optimization`
checks finite scaling, subnormal results, halfway underflow, negative zero,
extreme signed exponents, overflow and operand order. Package-set clients bind
`--host-intrinsic ldexp=math::target::Scalar::ldexp`; sysroot defaults include it.
See the [LLVM ldexp definition](https://reviews.llvm.org/D14327).
