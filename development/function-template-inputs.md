# Definition-time function-template clause applications

The D11 rejection of every explicit `fn name use template<...>` application is
removed. Expansion now retains a `FunctionTemplateApplication` in HIR, including
the original application expression and formal-to-instance clause identities.
It rebinds the signature and shared body without replacing source expressions
with argument syntax. Explicit applications remove those inputs from the later
runtime function's generic telescope; a bare `use template` still inherits it.

Elaboration prepares the target declaration and uses the existing ordinary
clause binder for source order, named/positional arguments, dependent domains,
spreads and defaults. It checks target formation requirements and capabilities
before publishing frozen instance inputs. Type inputs and ordinary value inputs
use the existing typed NIR value representations. Copies are charged, opaque
heads remain opaque, and repeated demands reuse the completed definition inputs.
This does not memoize arbitrary invocation results or add a template-specific
type-equality rule.

Application evaluation uses the enclosing definition environment. In particular,
a required comptime call while computing an argument must not request the
instance's own not-yet-substituted contract. The real StructBuilder fixture
exposed that premature dependency; separating the input environment fixes it
without skipping any target requirement.

Nested definitions prepare the enclosing contract and evaluate in its staged
parameter frame. Type and value inputs retain their dependent domains and use
that same frame for checking enclosing evidence. Completed inputs are then
lifted into the nested definition's lexical telescope; temporary binding maps
are restored on success and failure. Ordinary runtime inputs are not admitted
as definition-time values. This reuses explicit inputs, not model selection or
an expansion of implicit type inference.

The HIR table participates in package copy budgeting and omission, and has a
sorted S-expression dump retaining the application and clause correspondence.
Runtime code receives neither a template lookup nor an extra code argument.

Remaining template work includes zero/multiple shared-body inputs, general
compatibility proofs between deferred annotations and definition-time input
computations that require a not-yet-available lexical value. Existing ordinary
clause-binder limitations remain; this change does not invent a separate
variadic protocol or claim all template forms are complete.

## Verification

Working directory: `/home/fin/luna`.

- The initial explicit Type application regression failed at the blanket
  rejection (`/tmp/luna-tools-e3ryu8wt/result.json`, exit 101).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test
  function_template_inputs --test test_templates` passed
  (`/tmp/luna-tools-4sa0jxkj/result.json`, exit 0), covering fixed inputs,
  generic instances, callable/optional values, defaults, shared contextual
  bodies, invalid applications and opaque-head preservation.
- The first real sysroot invocation failed when argument evaluation tried to
  elaborate the unformed instance's contract
  (`/tmp/luna-library-nsnc0025/summary.json`). After correcting the environment,
  `python3 -B development/library_check.py --timeout 900 --stage
  sysroot-template-inputs` passed all three native tests
  (`/tmp/luna-library-tfnhdpbb/summary.json`). They use public OptionalOps and
  ErrorResultOps methods, a StructBuilder-produced type with a dependent Type
  default, and ordinary/quote shared-body instances.
- The extended nested regression first exposed an unavailable parent Type
  binding (`/tmp/luna-tools-vmgjcf52/result.json`, exit 101), then missing
  enclosing contract preparation (`/tmp/luna-tools-d6txg3at/result.json`, exit
  101). The corrected environment passes all 10 input tests and 114 existing
  template tests with the same luna command
  (`/tmp/luna-tools-znw0hu03/result.json`, exit 0). Rejection cases cover runtime
  values at definition formation and missing outer Type evidence.
- `python3 -B development/check.py --timeout 900 -- test -p hir` passes all 80
  tests (`/tmp/luna-tools-ma3n7f8g/result.json`, exit 0); the three existing
  documentation examples remain ignored. This covers publication and copy
  budgeting regressions alongside the structured application dump.
- After the lexical environment changes, the same sysroot command passes all
  three native tests again (`/tmp/luna-library-56wbvr9p/summary.json`, exit 0).

## Block-local definitions in inherited code

Function signatures now scan blocks in parameter annotations/defaults and return
annotations; clause defaults and requirements receive the same lexical scanning.
Each block keeps its own scope. This fixes missing local names before template
expansion without making default-local names visible in the instance body.

Template expansion reserves fresh owner and definition identities for the local
declaration subtree before cloning expressions. Calls to later local declarations
therefore resolve consistently, including nested functions. Copied functions,
constants, type aliases and newtypes keep their original declaration kind,
contracts and modifiers. Clause parameters and staged function parameters receive
fresh definition identities too. Resolver-selected trait sources, member access
and source provenance follow the binding map; lookup is not repeated in the
instance's textual scope. Attributes are retargeted and copied with budget checks.
The enclosing package fork remains unpublished until ordinary checking succeeds.

The new source tests use the real sysroot and cover generic template inputs,
local aliases, newtypes, nested helpers, forward references, fresh generic/staged
identities, inaccessible default-local names and rejected runtime captures.
The native example uses an inherited default helper calling public OptionalOps
through both ordinary and quote instances.

Limits remain explicit: local struct/enum/trait/impl and nested template
declaration cloning is not implemented by this change. Derivation drafts do not
automatically acquire permission to clone local declarations. Local functions
still conservatively inherit their parent's entire clause telescope. A function
declared inside a clause's own default can consequently receive that unsupplied
input even when its body does not use it. Precise lexical dependency analysis
must fix this; no arbitrary value is substituted. The positive regression
`local_comptime_calls_in_clause_defaults_need_precise_lexical_dependencies`
is retained with an explicit ignore reason, not counted as supported behavior.

Verification from /home/fin/luna:

- Before implementation, the two initial regressions failed during default name
  resolution and shared-body substitution (`/tmp/luna-tools-b7s6k8d_/result.json`,
  exit 101).
- `python3 -B development/check.py --timeout 900 -- test -p luna --test
  function_template_local_definitions --test function_template_inputs --test
  test_templates --no-fail-fast`: 133 passed, one known unsupported regression
  ignored (`/tmp/luna-tools-0263towa/result.json`, exit 0).
- `python3 -B development/check.py --timeout 900 -- test -p elaboration -p
  resolve`: all 369 checks passed (`/tmp/luna-tools-giqnb5lv/result.json`,
  exit 0). An initial run found a unit-test draft initializer missing its new
  state field; that initializer was fixed without changing the expected behavior.
- `python3 -B development/check.py --timeout 900 -- run -p luna -- test
  development/examples/template-local-definitions.fl --sysroot library`:
  the native test passed (`/tmp/luna-tools-ygaqacc7/result.json`, exit 0).
- Workspace format checking passed (`/tmp/luna-tools-xit52r9x/result.json`,
  exit 0); `git diff --check` passed.
