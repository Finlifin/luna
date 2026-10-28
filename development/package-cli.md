# Explicit package compilation, version 1

The product is one `luna` CLI binary. Independent anar development is cancelled;
future project configuration, dependency management and build scheduling belong
to luna. The explicit package and resident stdio modes remain interfaces of that
same executable, not separate products.

`luna --packages-v1` exposes the existing multi-package frontend as a process
interface. A build tool supplies identities and direct dependencies explicitly;
luna does not infer package identities from paths or discover a project configuration.

```sh
luna --packages-v1 \
  --package 7 701 client /project/client/main.fl \
  --package 9 901 utilities /project/utilities/lib.fl \
  --primary 7 \
  --dependency 7 util 9 \
  --emit nir --output /project/build/client.nir
```

Invalid dependency graphs are rejected before source loading. Diagnostics identify
the requester resolution ID, quoted alias, and provider resolution ID, and
distinguish absent packages, duplicate/empty aliases, self-dependencies, and
conflicting builtin bindings. Invalid input is not an internal resolver error.
Duplicate resolution IDs and semantic identities are reported separately.
For example: `dependency from package 1 via alias "missing" to package 9: provider package 9 is absent`.

Each `--package` consumes four arguments: a `u32` resolution ID, a `u64`
semantic identity, a UTF-8 package name, and an OS-native entry path. IDs use
unsigned decimal spelling. Resolution and semantic identities must each be
unique in the request. Paths containing spaces remain single process arguments;
there is no shell expansion inside luna. Paths beginning with `-` are accepted
as package entry values. `main.fl`/`lib.fl` retain the existing sorted module
source discovery rules; other entry names retain single-file behavior.

Each `--dependency` consumes requester resolution ID, UTF-8 import alias and
provider resolution ID. For the example above, client code imports `util`, while
source protocol selectors use the provider's declared name `utilities`. A
request must specify `--primary` exactly once. `--builtin ID` may designate one
supplied package as builtin; it does not load a sysroot or fabricate missing
packages or dependency edges.

Output stages are `hir` (the default), `nir`, `attributes`, and `merged-attributes`. The `attributes` stage
evaluates metadata into typed inspection descriptions with source/target
provenance; it does not authorize ABI/layout consumers or emit executable NIR.
`merged-attributes` additionally applies configuration-first, inner-to-outer
named-property selection per target. Its S-expression report retains the raw
objects and selected entry provenance. Positional entries fail this stage;
raw attribute inspection still accepts them. Neither stage authorizes consumers.
Ordinary `--output` and the
existing source protocol/type selector flags are supported: splice, meta-list,
tuple, index, list-pattern, literal-into, and the seven arithmetic/unary protocols.
Explicit source bindings require elaboration (`--emit nir`, `--emit attributes`, `--emit merged-attributes`, or an
execution/analysis action); syntax-only export rejects them instead of ignoring
them. This includes all seven arithmetic/unary selectors. Selectors keep their
existing `package.module.declaration` rules. Conflicting
or repeated ordinary options retain the single-entry CLI's validation.

The same package request also accepts `--interpret <entry>`,
`--invoke-comptime <entry>`, `--instantiate <entry>`, or
`--analyze-resources <entry>`, `--analyze-text-regions <entry>`, or `--test`, `--test-native`, `--emit-llvm <entry>`, `--emit-object <entry>`, `--emit-bitcode <entry>`, `--emit-asm <entry>`, or `--build <entry>` instead of `--emit`.
Ordinary source tests are described in [source-tests.md](source-tests.md). These actions elaborate NIR
and select the entry by the primary package's **semantic identity**, so an
identically named dependency function is not selected. Existing action rules
apply: `--fuel` is positive and bounded per stage/query, resource analysis
requires `--copy-protocol`, and checked interpretation uses both
`--copy-protocol` and `--drop-protocol`. `--inline-functions` requires both Copy/Drop bindings and supports interpretation,
tests, LLVM emission and native builds. Resource selectors are resolved in the same package set before
elaboration; qualified names refer to declared package names, not import aliases. The
resolved declaration identity is carried into execution and resource analysis;
it is not looked up again by leaf name. This also applies to ordinary
single-input/sysroot actions (`input.module.Trait`, `builtin.Trait`, etc.).
Manifest defaults do not enable cleanup analysis by themselves: an analysis
without `--drop-protocol` remains an ownership-obligation report.

For the example above, replace `--emit nir` with `--interpret run` to execute
client's runtime entry through the reference interpreter, or `--instantiate run`
to export its concrete IIR graph. Invocation only admits comptime entries;
interpretation and instantiation retain their runtime entry checks. Conflicting
actions and action-specific flags without their action are usage errors.

`--analyze-text-regions <entry>` selects a nongeneric ordinary runtime function
without unresolved type/capability assumptions. Runtime parameters remain symbolic
inputs; no argument values or Copy/Drop options are required. The result is the
stable `(text-region-summary ...)` S-expression describing character reads and
returned backing-region dependencies. It does not prove region liveness, cleanup,
or memory safety. Unsupported transfers and exhausted fuel return an error rather
than a partial summary. Like other text actions, failures preserve an existing
output file; `--with-inputs` wraps the summary with observed package inputs.
The resident compile-only stream does not accept this action.

```sh
luna source.fl --analyze-text-regions inspect --fuel 100000
luna source.fl --analyze-text-regions inspect --output text-regions.lisp
```

AST export remains unsupported in package-set mode.
`--emit-llvm` emits host LLVM IR; `--opt-level 0|1|2|3` optionally verifies and
optimizes it with the configured LLVM toolchain. Entry selection uses the primary
semantic identity, including when optional inlining is enabled. With
`--with-inputs`, LLVM text is escaped as `(llvm-ir (string "..."))` within the
result envelope; without it the output is raw LLVM IR. Source-alias protection
also covers all dependency inputs.

`--emit-asm <entry>` lowers the verified, optimized module to native host
assembly without invoking the linker. Output is UTF-8 text on stdout or at
`--output <path>`; the default optimization level is 0. It preserves the same
resource protocols, intrinsic bindings and entry adapter as object emission.
With `--with-inputs`, the result is `(host-assembly (string "..."))` within the
compilation-result envelope. The Rust toolchain API is `Toolchain::assembly`;
package actions use `Action::EmitHost { assembly: true, .. }`, with `false`
selecting LLVM text. Target mismatches are rejected before code generation.

`--build <entry> --output <path>` compiles and links the same host IR into an
executable, then atomically publishes the complete binary with executable
permissions. `--opt-level` and checked optional inlining have the same meaning
as in single-input builds. Compilation, resource admission, LLVM and linker
failures preserve the previous destination. `--with-inputs` is rejected for
binary builds: use LLVM emission when a text artifact with an input envelope
is required. The Rust API returns executable bytes from `Request::build`;
`Request::run` remains a text-only interface. The resident frontend stream
continues to reject execution and host actions.

`--emit-object <entry> --output <path>` returns the same verified and optimized
host module as a relocatable object without invoking the linker. It accepts the
same resource protocols, intrinsic bindings, result-discard and inlining options.
`Request::build` returns object bytes for `Action::Build { output: BinaryOutput::Object, .. }`;
the CLI publishes them atomically without adding executable permissions. Binary
output requires a path and cannot use `--with-inputs`. This does not define a
Flurry FFI ABI: the object still contains the selected development entry adapter.

`--emit-bitcode <entry> --output <path>` exports verified, optimized LLVM
bitcode with the configured target triple, data layout and semantic target
metadata. It accepts the same host options and resource protocols, requires a
binary destination, and rejects `--with-inputs`. It invokes neither llc nor the
linker; unavailable tools for later stages do not block this output. The Rust API
is `host::bitcode`, or `host::binary` with `BinaryOutput::Bitcode`; package requests
use `Action::Build { output: BinaryOutput::Bitcode, .. }`. Existing executable and
object outputs use the corresponding enum variants. Publication preserves previous
files on failure and does not add executable permissions.

This entry does not yet implement project configuration evaluation,
package resolution, dependency fetching or incremental build scheduling.

Exit status is 0 for successful compilation, 2 for malformed/unsupported
requests, and 1 for semantic package/identity/dependency errors, compilation or
I/O failures. Output goes to stdout unless `--output` is supplied. Complete
output is published atomically; failed compilation or action leaves existing output
intact. Output aliases of any supplied or discovered source are rejected using
the existing multi-package source protection, including dependency files.

Regression entry:

```sh
python3 -B development/check.py --timeout 180 -- \
  test -p luna --test frontend_cli explicit_package_cli
```

List patterns can bind a source-defined protocol explicitly:

```sh
luna development/examples/list-patterns.fl --interpret evaluate \
  --list-pattern-protocol input.ListView
```

This returns `(bool true)`. The same flag accepts a supplied provider name in
`--packages-v1` and overrides `list-pattern-protocol` in a sysroot manifest.
There is no implicit binding based on a trait's spelling.

With explicit Copy/Drop bindings, list extractors remain subject to resource
checks. Ordinary and `quote fn` implementations both preserve observation
restrictions, including when `--inline-functions` is enabled: writing through
the observed scrutinee is rejected before publishing a result. The binding flag
does not grant extractor mutation authority.

`--test-filter <text>` is accepted with `--test` in package-set mode and selects
matching primary-package labels after frontend validation.

## Observed source inputs (Rust API)

`packages::compile_with_inputs` accepts the explicit package/dependency set,
primary/builtin identities and protocol selectors and returns the NIR artifact
with `CompilationInputs`. The process interface exposes the same observations through
`--with-inputs`; neither interface implements project orchestration. Existing compile entry points do not retain copies of
source text.

Each `SourceInput` records the resolution ID, semantic identity, absolute
OS-native path and both the raw UTF-8 text read by the loader and the normalized source-map
text actually used in that invocation. The snapshot stays unchanged if the file is subsequently edited.
Sources retain request order, with each package entry first and its remaining
discovered sources in deterministic traversal order. Explicit nested package
boundaries are respected. A single-file package does not acquire unrelated
sibling files.

`directories` includes each scanned root and its discovered directories,
including empty ones. Cache reuse must check directory membership to detect new
or removed modules; checking only recorded source contents is insufficient.
The returned observations are not an atomic filesystem snapshot or a complete
cache key. The build tool must revalidate inputs and include compiler/target,
configuration, package identities, dependency edges and protocol bindings.
`SourceInput::raw_text` preserves BOM and CRLF exactly; `text` is normalized.
Both come from the same file read, and repeated requests for a filename within
an observed compilation reuse its first loaded text. `changed_sources()`
compares current file bytes against `raw_text`, reporting missing or modified
files and returning an error for other I/O failures. It does not revalidate
directory membership or the rest of the cache key and is not an atomic check.

`changed_discovery()` separately repeats each recorded scan with its original
explicit package exclusions. It reports sorted added/removed source files and
directories by resolution ID, without parsing or executing code. Missing roots
report their recorded members as removed. Unsupported symlinks, scan I/O errors
and a root resolving to a different canonical path return errors rather than a
successful validity result. Newly discovered non-source files do not invalidate
membership; empty directories do, since they participate in module discovery.
Both checks are necessary and neither substitutes for checking request options
or provides a filesystem transaction.

The envelope's `discovery` section records each scan root, canonical root,
exclusions and relative file/directory membership from the actual compilation.
It enables a process consumer to apply the same validation policy; it does not
claim that the compiler has revalidated the filesystem at publication time.

The version-1 S-expression display includes normalized `text` and an optional
`raw-text` field when normalization changed it; absent `raw-text` means the raw
text equals `text`. Both use the existing escaped string representation. It encodes
paths as hexadecimal `path-bytes`, using the producing platform's native
`OsStr::as_encoded_bytes` representation. These bytes preserve non-UTF-8 paths
locally; they are not a cross-platform path serialization. Compilation failure
returns an error and publishes neither an artifact nor a successful inventory.

### Process export

Add `--with-inputs` to a `--packages-v1` request to publish the operation result
and the observed inputs together:

```sh
luna --packages-v1 --package 7 701 client /project/client/main.fl \
  --primary 7 --emit nir --with-inputs --output /project/build/client.lisp
```

The output has the shape
`(compilation-result (version 1) (result ORIGINAL-RESULT) (compilation-inputs ...))`.
The inner result uses the existing operation's format. HIR/NIR export and the
existing execution, instantiation, analysis and test actions share this path.
The inputs come from the same compilation used by the selected operation.
There is no second file scan after execution and no separately published
sidecar that could become mismatched with the result.

Without this flag the existing output is unchanged. The flag is specific to
package-set mode and duplicate occurrences are usage errors. An operation
failure publishes no successful envelope; `--output` retains its existing
atomic publication and source-alias protection. Snapshot normalization, native
path encoding and cache-key limitations described above also apply here.

## Process-local frontend reuse

A long-lived Rust build driver can retain
`compilation_cache::CompilationCache::new(capacity)` and call
`cache.compile(&request)`. The result contains an immutable `Arc<CachedCompilation>`
(artifact, resolved protocol bindings and observed inputs) and a `reused` flag.
Only completed frontend artifacts are retained. Execution, source tests and
comptime entry invocation are not cached or performed by this method.

The cache uses a bounded least-recently-used entry set. Zero capacity disables
retention; `clear()` explicitly invalidates all entries. Full package requests
and the current working directory are conservative keys: identities, dependency
edges, selectors, stages, action flags and output paths are not silently ignored.
Compiler and target implementation identity are fixed by the running process;
this is not a persistent or cross-compiler cache format.

On a matching request, canonical package-entry targets, directory membership
and raw source contents are checked before reuse, and output aliases are checked again against every observed
source. A stale entry is removed and recompiled; an I/O or protection failure
removes that entry and returns an error. Compilation failures are not retained.
Fresh compilations are revalidated before insertion so an already-observed
change does not become a successful cache entry. Canonical entry targets are
checked around validation and compilation: retargeting a dependency symlink can
change source ownership exclusions even when its contents are byte-identical.
These target observations are process-local cache metadata, not an extension
to the serialized version-1 input format. Other cached requests remain
independent. These checks do not create an atomic filesystem transaction.

The ordinary single-shot CLI still compiles each request. The resident stdio
entry below uses this cache. Neither entry implements on-disk cache persistence
or project configuration discovery.

## Resident stdio compiler, version 1

Start `luna --packages-stream-v1 [--cache-entries N]`. The default capacity is
8 frontend artifacts; 0 disables retention. This is a serial request stream;
independent workers may use independent processes. It opens no listening socket.

Each request is one LF-terminated ASCII line. Encode each argument to the
`--packages-v1` parser (without the `--packages-v1` prefix) as hexadecimal bytes,
and separate arguments with exactly one ASCII space. `-` represents an empty
argument. No shell quoting or expansion occurs. On Unix the decoded bytes are
native `OsString` bytes, preserving non-UTF-8 paths; other platforms currently
require UTF-8 arguments. CRLF, blank lines, malformed hex and extra separators
are rejected as malformed frames. Requests are limited to 1 MiB excluding LF
and 4096 arguments. Oversized lines are drained without allocating the entire
line so a later request can still be handled.

For example, a Python client may encode an existing byte-argument list as:

```python
frame = b" ".join(arg.hex().encode("ascii") if arg else b"-" for arg in args) + b"\n"
```

The stream accepts HIR/NIR compilation, attribute inspection, and the usual package/type/protocol
selectors, including `--with-inputs`. It rejects execution, invocation, tests,
analysis and `--output`; those retain their existing single-shot entry points.
Successful compilation results are returned to the caller rather than written
to a destination file.

Each request receives one flushed, single-line S-expression:

```text
(package-response (version 1) (sequence 1) (exit-code 0)
  (reused false) (payload (string "ESCAPED-RESULT")))
```

The example is wrapped for readability; actual replies occupy one line.
Sequence numbers start at 1 and include rejected frames. Code 0 means success,
1 means compilation/input-validation failure, and 2 means framing or usage
failure. `reused` is true only for a validated cache hit. Payload contains the
usual artifact or input envelope as an escaped string on success, and a
message on failure. Compiler diagnostics can additionally appear on stderr;
clients should drain stderr separately from the framed stdout replies.

Successful payloads are formatted and escaped directly to the reply writer,
without first retaining a complete dump string. This preserves the version-1
wire format; a write failure can leave a partial frame and terminates transport.

A request failure does not terminate the stream. Closing stdin between frames
ends the process successfully; an unterminated final frame receives code 2.
Transport read/write/flush failure terminates the process with status 1.
Per-request failure is represented by its reply code, not the eventual process
exit status. Cached artifacts are process-local and are discarded on exit.

For native host actions, `--discard-result` explicitly retires the selected
entry's result instead of printing it. It requires both Copy and Drop selectors,
and works with `--build`, `--emit-llvm` and optional inlining. A checked NIR
wrapper owns and discards the returned value, so destructor selection and failure
propagation use ordinary resource verification. The primary semantic identity
still selects the entry. This does not change native source tests, which already
require void results, or enable host actions in the resident frontend stream.

### Host FMA 显式绑定

`--build` 和 `--emit-llvm` 接受 `--host-fma package.module.Enum.variant`，module 可省略。选择的 enum 必须标记 intrinsic_enum，变体为 unit。解析沿编译包集合的源声明身份进行，绑定在纯度检查前生效；不以 primary 包替代选择器中的包，不把同名标签合并。选择器保存在 Request Action 中，参与现有完整请求缓存键。当前每个命令只接受一个 FMA 绑定，不支持以该选项启动解释器或 native tests。

## Sysroot LLVM intrinsic defaults

For single-input compilation with `--sysroot`, version 1 of
`language-items.conf` accepts `host-intrinsic.<operation> = package.Enum.variant`
(with optional module components). `library/language-items.conf` binds all current
scalar operations to the real `builtin.math_intrinsics.target.Scalar` declarations.

Defaults are merged by operation. `--host-intrinsic operation=selector` and the
`--host-fma`, `--host-sqrt`, `--host-copysign` aliases override only that operation;
already-bound driver identities suppress corresponding defaults. Explicit
conflicts are still errors, rather than silently choosing one. Missing packages,
non-intrinsic enums and invalid variants retain the ordinary binding diagnostics.
The package-set protocol itself does not load this file. AST/HIR-only sysroot
commands do not resolve these semantic defaults.

Math is a module in `builtin/math_intrinsics.fl`, exported through `builtin.prelude`.
The single-input sysroot loader has six package slots and does not load a
separate math package. Repository manifest selectors use
`builtin.math_intrinsics.target.Scalar.<operation>`. Explicit package-set tests may
load the same source as a package named math and bind that package's identities;
this does not change sysroot defaults. Missing declarations do not authorize
an unresolved manifest selector.

The sysroot loader registers the available builtin, meta, build, attrs and
verification package identities as default prelude providers. Each contributes
an ordinary package-visible glob to every other package; std is a consumer,
not a default provider. Same-declaration re-exports coalesce, local declarations
win, and unrelated same-name declarations remain ambiguous. A designated
provider must define its prelude. Names alone do not grant this role.

The resolver exposes `build_package_set_with_preludes` for explicitly designated
provider IDs. The existing package-set CLI still designates only builtin through
its existing option; additional provider registration currently enters through
the sysroot path, not by guessing roles from arbitrary package names.


### Explicit source build targets

Package requests and resident frames accept `--build-target ARCH OS BITS` and
`--build-feature NAME=true|false` for the primary semantic package, plus
`--package-build-target ID ARCH OS BITS` and
`--package-build-feature ID NAME=true|false` for explicit semantic identities.
Features can precede targets; each selected package must have its own complete
target input. Empty names, duplicate targets/features, unknown identities,
primary/explicit overlap and unsupported pointer widths are errors. Resolution
IDs do not stand in for semantic IDs. No dependency inherits the primary input.

These inputs require NIR compilation or execution and are retained in the full
request cache key. Architecture, OS, width and feature presence/value changes
invalidate reuse; missing-input failures do not substitute old compiled values.
The primary width supplies semantic TargetData for NIR-only requests. Native
actions use their actual backend layout and reject a conflicting primary width.
The inputs describe the source build context; they do not select LLVM target
triples or CPU feature flags. Observation-selective reuse remains pending.

### Typed metadata for embedding callers

Rust callers can set `package_cli::Request::metadata` to `Some(Vec<Input>)`
using `build_context::metadata::Input` and validated `Entry` descriptors. The
compiler builds dependency aliases from that request's actual source graph;
metadata owners are semantic package identities. `None` means no metadata
configuration, while `Some(vec![])` installs a configuration with no supplied
provider inputs. Metadata requires NIR compilation or execution.

The complete request cache key includes field names, types, immutable values,
absence and dependency/provider identities. This is conservative structural
comparison, not a new type-equivalence rule. Invalid input, duplicate owners,
unknown owners and exhausted preparation budgets fail without returning cached
artifacts as fallback. Descriptor and name copies are charged before snapshot
construction. Source changes still use the ordinary source revalidation path.

Explicit package CLI and resident requests accept one `--package-metadata-v1 HEX`
argument. See [the versioned format](metadata-wire.md) for encoding, limits and
unsupported descriptors. The decoder produces the same typed request inputs;
hex spelling does not participate in cache identity. Observation-selective
reuse remains separate work.
