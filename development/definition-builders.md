# Definition builder implementation boundary

The intended feature covers most definition kinds, including the SoA example's
struct construction. This file tracks implementation, not new Flurry syntax.

## Implemented: atomic HIR registration

`hir::definitions::DefinitionBatch` reserves owner identities and accepts the
existing `ItemKind` representations. It is not restricted to struct fields:
functions, declarations, structs, enums, traits, implementations, effects,
containers, aliases, and other supported HIR kinds use the same registration path.
`Invalid` is rejected. Function and template definitions must supply their bodies.

The batch holds exclusive access to its target package, but does not alter it
while constructing drafts. Previously published definitions remain available
through a read-only view. A failed structural validation or exhausted validation
budget leaves owner, body, identity, and lexical-parent tables unchanged. Drafts
can be corrected and retried. Dropping a batch does not consume package owner IDs.

Publication checks:

- Every reserved owner has a description; published owners/metadata are not replaced.
- Definition identities are valid and unique, including against the target package.
- Function bodies exist; body roots and formal inputs belong to the right owner.
- Container references exist and form an acyclic graph. Forward references within
  a completed batch are supported.
- Lexical parent links exist and are acyclic.
- Indexed HIR nodes agree with their owner/local identities; their parent links
  exist and reach the owner's root without cycles. An indexed root must refer to
  the canonical definition item.

Container, lexical-parent, and node-parent traversal reuse validated paths rather
than repeatedly traversing entire ancestor chains. Body replacement uses an ID
index. Tests exercise 1,024 nested owners and 1,024 node parents with a validation
budget proportional to their size.

Only after every check passes are all table entries installed. No user callback
or fallible semantic check runs during that installation. Draft and diagnostic
Display implementations use stable S-expressions, including escaped names, node
indices, parent relationships, and separately ordered bodies.

The actual AST-lowering provider now uses this path to construct directory-only
module definitions. Scope identities are deduplicated before reservation, matching
the previous sequential-registration behavior. Existing source module linking,
elaboration, and execution remain consumers of ordinary HIR.

## Still required for source-level builders

This is **not** a working `StructBuilder.new/add_field/build` API. There is no
new runtime/VM builder value, field-reflection API, or source-language builder
method in this change. The SoA example still cannot run through this feature.

The next integration must transport owned, invocation-local definition drafts
from the VM to a candidate elaboration world. This registration batch can install
their HIR, but is not a transaction for the entire elaborator: name resolution,
signature/body checking, type obligations, phase checks, and coherence still need
their normal passes and candidate rollback. Structural registration must never be
treated as proof of those semantic properties. Neither published HIR nor frozen
comptime values may contain active inference variables, waiters, or solver
transactions.

Builder-specific assembly must reuse this path rather than making a separate
nominal-type registry for SoA. Generated bodies must keep their CMTT context and
source provenance. Normal-form heads remain opaque; construction of a structural
view does not identify a type-producing function application with its body result.

The first source-level acceptance remains: inspect fields, construct SoA columns
and defaults in a comptime loop, finalize the definition, add methods, and use the
result. Broader definition kinds need equivalent positive/rejection tests.

## Validation

Working directory: `/home/fin/luna`.

```sh
python3 -B development/check.py --timeout 900 -- test --release -p hir -p luna --test definitions --test display --test modules --test packages --no-fail-fast
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

The implementation run completed with exit 0: 7 definition-registration tests,
16 Display tests, 56 module tests, and 48 package tests (127 total). Evidence:
`/tmp/luna-tools-y_2y9jer/result.json`, with per-suite summaries in `stdout.log`.
The final formatting check also exited 0:
`/tmp/luna-tools-8mz6eg2k/result.json`.
These checks establish structural registration and the existing module consumer;
they do not establish source-level definition-builder support.
