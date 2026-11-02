# Runtime type-reference integration boundary

Audit baseline: `afeb60a0` on `dev`. This is an implementation checkpoint, not a
language amendment. Authority: `docs/compilation/runtime-dependent-types.md`,
especially the distinction between immutable cores, substitutions and legal
environment acquisition. The D06 finding in the 2026-11-02 morning report remains
open; empty argument lists at the two existing production sites are not themselves
an incorrect implementation of those sites' current contracts.

## Existing paths and their contracts

- `compiler/nir/src/types.rs`: `TypeInterner::abstract_runtime_with_budget`
  abstracts explicitly supplied runtime identities and their checked domains into
  ordered holes. Repeated identities share a hole. Dependency contracts may use
  preceding dependencies; unrelated identities cannot be merged for sharing.
  Construction is bounded and structural. `TypeRef::check_with_budget` remains
  responsible for invoking the consumer's semantic contract checks.
- `compiler/iir/src/lib.rs`: `CallableIdentity::intern` retains completed,
  environment-free callable identity evidence. `closed_signature` explicitly
  requires no holes or arguments. The materializer uses this for function values
  embedded in type identities, not for all executable expression annotations.
  The comptime interpreter also consumes `closed_signature` when recovering
  callable evidence from retained IIR. Relaxing this method would invalidate
  both consumers without supplying a runtime environment.
- `compiler/iir/src/template_storage.rs`: `TemplateStorage` records selected
  staged runtime function identities with matching environment-free signatures.
  It explicitly excludes closure environments. Replacing its empty substitution
  with arbitrary runtime arguments would change this storage contract, not
  implement general dependent type transport.
- `iir::FunctionInstance.function` still owns a `nir::program::Function`.
  Ordinary signatures, expression annotations, captures and contracts therefore
  still carry `Term` rather than a production reference-side substitution.
- `materialization::instantiate_with_types` accepts a set of externally available
  `RuntimeId`s and forwards it to IIR. `completed_type` checks scope and membership
  in that set. It does not supply a dependency's domain, an acquisition path, or
  ownership/lifetime evidence. The field comment already states this limitation.

## Required implementation sequence

1. Establish the producer-side dependency evidence for ordinary annotations.
   Each external runtime identity needs its checked domain and a legal acquisition
   path from the use site's environment. Preserve transitive dependencies used
   only in parameter contracts. A bare membership set cannot supply this evidence.
   Keep local runtime binders, inference metas and core holes distinct.
2. Carry that evidence through ordinary materialization inputs and outputs.
   Use the existing telescope abstraction to form cores and check substitutions;
   keep acquisition information separate from the interned structure. Do not
   create an unused parallel table while continuing to interpret raw annotations
   as the only authoritative types.
3. Update consumers of ordinary annotation types together: IIR validation,
   semantic equality/formation, resource checking, layout where statically
   justified, invocation environments and lowering. Materialize or observe a
   reference through its use-site substitution with bounded work. Core identity
   alone must never establish complete type equality.
4. Connect source elaboration and environment capture to these inputs. A retained
   `RuntimeId` is an identity term, not permission to evaluate its runtime contents
   during comptime. Preserve normal-form heads and keep program-state obligations
   out of PTC. Unsupported runtime-dependent layouts remain explicit failures.

## Acceptance evidence still needed

- Two source-produced references share a core while their runtime identities stay
  distinct; repeated use of one identity retains aliasing and hole correspondence.
- Nested callable scopes, dependent hole domains, substitutions and capture paths
  survive materialization without capture or loss of contract-only dependencies.
- Missing, wrong-domain, unavailable or invalid-lifetime dependencies are rejected
  by the responsible layers. Trial failure and budget exhaustion publish no partial
  reference/environment and permit retry.
- Runtime acquisition follows the declared source evaluation order and does not
  read runtime contents during comptime. Code generation and execution consume
  the produced environment; Rust-only constructors do not prove this source path.
- Dumps display core, substitution and environment acquisition separately as
  stable S-expressions. Closed callable/template evidence continues to obey its
  existing environment-free contract unless a separate representation is added.

No new production runtime-dependent annotation path or EnvironmentPlan has been
implemented by this audit. The next implementation must start with typed
producer-side dependency evidence, rather than weakening the closed identity
containers or assigning domains to the current untyped availability set.

## Verification for this audit

Working directory: `/home/fin/luna`.

```text
python3 -B development/check.py --timeout 900 -- test -p nir -p iir --test types --test callable_type_cores
```

Exit 0; the three binaries passed 4, 13 and 65 tests respectively. Evidence:
`/tmp/luna-tools-5b_zu53o/result.json` and its `stdout.log` / `stderr.log`.
These checks establish the existing core/reference and closed callable contracts,
not the missing source-to-runtime environment transport described above.
