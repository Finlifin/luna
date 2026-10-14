# Flurry implementation map

The active objective is implementation of the language defined by `docs/`. Individual runs close bounded engineering tasks; they do not close this objective. This map tracks dependency order and the evidence needed before a feature can be called implemented. It is an implementation plan, not a replacement language specification.

## Current executable boundary

`luna` accepts source and exports AST, HIR, or type-elaborated NIR (`--emit nir`). It also explicitly invokes a concrete comptime entry with `--invoke-comptime <name>` under a configurable execution fuel budget, returning an S-expression value. The connected NIR path covers explicit function signatures, aliases, Type-parameter generic functions with definition-side checking and inferred/explicit instances and context-inferred function values, forward/recursive calls, higher-order function values, contextual/annotated lambdas with explicit structural capture environments, local inference, nominal structs including explicitly instantiated Type-parameter templates, with declaration-ordered named/default initialization, field reads/writes and named-field pattern destructuring, let/var rebinding checks, closed enums including explicit Type-parameter instances, with unit/positional/named variants and definition-scoped field defaults, nested payload binding and bounded constructor-matrix exhaustiveness checking, tuples with nested heterogeneous pattern destructuring and constructor-matrix coverage, booleans, strings/chars, if/when branches, returns, local assignment, ordered scalar matches with guards/bindings/exhaustiveness checks, sequential and-is patterns with candidate-local bindings and embedded computations, corresponding or-pattern binders, refutable let bindings with diverging failure branches, nested loops with lexical break/continue targets, named optional arguments and finite scalar/tuple default computations. It rejects unsupported protocols and retained pending stage/attribute information. Source-level generic direct calls now use candidate-local inference and obligation freezing; seven inference-store tests cover trial isolation, stale commit rejection, occurs checks and export of solved terms. Type equality/inequality and Boolean `requires` now provide definition-local assumptions and caller-proved instance obligations, including generic inference and function values. Runtime predicates stay outside PTC. Full PTC/trait integration, general numeric/operator protocols, dependent/value generics, general partial-type contextual elaboration, comptime phase authorization, resource/effect verification and executable code generation remain incomplete. NIR output is not a resource-verified IIR artifact. A concrete interpreter executes the supported value/control-flow fragment through the shared frontend with malformed NIR rejection and materialization budget checks. A phase-checked entry now executes closed first-order comptime computations required by type formation under shared fuel/depth limits; general eligibility, residual scheduling and invocation remain incomplete.

The older `middle` type objects and disabled backend/typeck directories are implementation inputs to evaluate, not semantic foundations to preserve unconditionally. In particular, fixed-width literals, pure-head unfolding, and conflating runtime identity with type-core identity cannot become compatibility requirements.

## Dependency groups and exit evidence

| Group | Required behavior | Prerequisites and exit evidence | State |
|---|---|---|---|
| Source and diagnostics | Lexer, parser boundaries, source spans, grouped patterns, attributes, complete AST/HIR payloads, explicit unsupported diagnostics | CLI source fixtures, rejection cases, structured IR dumps, deterministic diagnostics | CLI/display and literal frontend established; trait required-function/associated-value declarations and file-qualified impl scopes retained in HIR; other syntax incomplete |
| Binding and organization | Package/module/static-container identities, imports, visibility, signature collection, recursion and query dependencies | Multi-file fixtures with identity and negative-query invalidation checks | Single source-root discovery, file/directory/inline modules, static calls, imports and glob candidate propagation connected; 23 multi-file test groups plus 3 permission-core tests. Aliases, nested groups, anchors and private import entries connected. Multi-package loading/integration and incremental discovery invalidation pending |
| Stable semantic representation | Immutable type core plus scoped substitution; rigid parameters, runtime binders and inference variables distinct; normal-form heads opaque | Representation tests: shared core/different environments, repeated identities, nested substitution, capture avoidance, rollback; S-expression Display | NIR type core/reference foundations, runtime abstraction and use-site scope checks and dependent function-signature instantiation implemented with 24 tests; dependent contract solving remains pending; monomorphic HIR elaboration now produces typed expression trees |
| Elaboration engine | Bidirectional expected information, candidate-local transactions, wait/wakeup obligations, dependency-aware failure and diagnostics | Expressions/calls checked through CLI; no success placeholders; candidate rejection/retry and cancellation fixtures | Monomorphic CLI path uses scoped PTC type equality; candidate region supports structural matching of two open substitutions while retaining original equality obligations, and joins whole-call type inference, ordered dependent substitution, PTC facts, original-relation obligations and directed wait/wakeup; Type-parameter generic calls and type-only requires on functions/structs/enums connected to source, including deferred nominal formation obligations and definition-scoped constrained defaults; transactional nested argument retries and early result-context propagation connected; partial callback contexts support lambdas, generic function values and nested generic calls through blocks/if/match/when, with body results feeding inference; proof-backed structural views connect constrained tuple/record/enum operations, coverage and higher-order calls/lambda/function-value contexts; function signature dependencies now resolve on demand with cycle/depth checks; observed calls retain definition-scoped deferred contracts and declaration obligations survive speculative rollback; general partial-type continuations and comptime scheduling integration pending |
| Comptime and freezing | Interpretation and invocation with common semantics; stable residual definitions and legal delays; type comptime completion at IIR entry | Definition-to-instance tests, required-computation failures, no duplicate impure execution, opaque NF identity tests | Concrete NIR interpreter kernel and source execution tests; explicit comptime/pure modifiers survive declaration scanning and lowering, NIR function phases are displayed and checked, and a dedicated first-order comptime interpreter entry validates call phases and enforces execution budgets; closed required computations now form aliases, explicit function signature types and computed BitVec widths by preparing reachable comptime bodies on demand; successful alias completion is reused per declaration, dependency cycles and shared execution budgets are checked, and unresolved trait-world dependencies explicitly block execution; first-order Type-parameter instances now substitute signatures and bodies for required computation and nested comptime execution; type-only declaration contracts now prepare on demand and discharge instance obligations in their recorded definition/caller scope before execution, including independent contract-operand formation checks; ordinary optional defaults now prepare on demand with definition-scoped prior-parameter bindings, and comptime defaults execute through the ordered call plan only when omitted; generic default binders no longer masquerade as caller inference evidence; Type-parameter/Type-result normal-form heads now parse, resolve, check their bodies/contracts and export opaque head-plus-argument types with a non-invocable NIR definition phase; canonical arbitrary-precision Integer head arguments now evaluate closed inputs, retain symbolic declaration inputs and coexist with Type arguments without unfolding; Bool and fixed-width signed/unsigned integer inputs now share stable-value checking and symbolic forwarding, including range checks and mixed input ordering; prior Type inputs can now determine later scalar/Type input types, with ordered substitution and proof-backed symbolic forwarding; owned NIR now exports ordered head input descriptors with explicit Type/value roles, declaration/binding identities and dependent input types, validated against body metadata before interpretation; concrete Type values now check applications of known heads against those descriptors, including dependent builtin input types and integer ranges; arbitrary dependent type expressions and other value-parameter normal forms, trait-world waits, general automatic invocation/splicing, higher-order phase types and loose residual checking remain pending; comptime_runtime now provides canonical arbitrary-precision Integer parsing, bounded arithmetic and checked fixed-width conversion, verified independently up to 1024-bit operands; concrete source comptime Integer values now reach NIR/interpretation with exact add/subtract/multiply/negation/comparison, literal matching and shared operation budgets; runtime storage phase inspection follows actual aggregate fields without unfolding opaque heads; concrete Type values now preserve primitive/nominal/structural/opaque identities through calls and substitution, and the explicit BitVec type-value constructor evaluates Integer widths without allocating bit storage; source unsuffixed integer literals now splice into checked fixed-width signed/unsigned proxies through 128 bits, with deferred generic defaulting and contextual patterns/defaults; exact decimal literal components and direct finite f32/f64 proxies now connect source context, generic defaulting, defaults and patterns without double rounding; general Float arithmetic, float overflow policy, target pointer-width plumbing, LiteralInto suffix protocols, full comptime phase authorization, residual scheduling and invocation pending |
| PTC and trait protocols | Type-only atomic propositions and theory solvers; implementation selection, associated declarations, capabilities through elaboration | Atom-by-atom accepted/rejected/unknown fixtures and solver isolation; actual LiteralInto/pattern protocol calls | Type equality with explicitly registered nominal constructor contracts (including inference, proof replay and concrete countermodel validation), closed type classification, integer difference constraints and mixed Boolean coordination implemented with scoped proofs/replay and validated type/integer countermodels; integer arithmetic capacity is bounded with Unknown on exhaustion; source type equality/Boolean requires connected; implementation indexing and tri-state selection connect candidate-local Type inference, proved/refuted requirements and replayable type-applicability evidence, with versioned world snapshots and fixed-interface package-ownership admission; type-only same-precision coherence now checks alpha-renamed heads, complete interface arguments and combined requirements without caller assumptions, with independent proof/report replay and world freshness; generic source trait implementation Type telescopes and type-only requires now reach NIR, definition-side member checks, parent obligations and statically selected method substitutions; compile-time concrete dispatch materialization follows function instances, closures, pattern computations and nominal capabilities and feeds a validated interpreter lookup table; generic implementation execution is connected for this Type-only fragment, while phase/environment delivery remains pending; concrete source trait/inherent static methods now check contracts and dispatch explicit interface views through real candidate resolution, including explicit Self substitution and subject-specialized default bodies, with by-value receiver calls/capture, evaluate-once ordering and definition-fixed same-interface/parent-interface calls inside default bodies and independent concrete parent implementation obligations, unambiguous inherited projections and default-body implicit capability lookup and ordinary generic-function positive trait bounds with caller-side obligation checks and generic nominal-type capability formation checks; generic inherent methods now use a separate name-filtered index with type evidence replay, coherent specialization and inherited head parameters/contracts, connected through function instances to execution, including named/optional inherent method arguments through the shared ordered call planner; trait calls now preserve interface entry names/defaults and definition-local preparation frames independently of selected implementation locals, across concrete/generic/default/parent calls; positive implementation trait bounds now qualify candidates before ranking using scoped assumptions, parent implications and recursive immutable-world resolution, with cycle/abstract absence pending and compile-time dispatch dependencies; complete Type-argument interface identities now survive NIR substitution, parent evidence, candidate queries and concrete dispatch, with NIR-only parameterized-interface execution tests; Type-parameterized trait declarations, implementation heads, explicit interface views, bounds and inherited/default member calls now also execute from source, including rigid symbolic queries and interface-only implementation parameters; trait Type-parameter bounds and type-only requirements now check interface formation, provide scoped default-body assumptions and materialize concrete dependencies; Self-dependent trait type requirements now check implementation subjects and concrete dispatch materialization, and export bounded scoped type guarantees through declared capabilities and parents into function/default-body checking; Self-dependent parameter capabilities now defer to implementation-subject checking, substitute fully in materialization/runtime validation, and propagate through parent/parameter edges for generic member lookup and bound forwarding with complete-source budget checks; value/dependent interface arguments, pointer adaptation, negative trait predicates and associated substitution remain pending |
| Functions and runtime types | Parameters/defaults, contracts, closures, dependent environments, user types, rows and dispatch | Call-order/evaluate-once tests; layout versus environment separation; all explicit lowering forms retained | Syntax/HIR incomplete; semantics pending |
| Resources and runtime specification | Once/Clone/Copy, memory views, affine operations, safe-mode predicate limits and runtime checks | Positive/negative alias and lifecycle fixtures, higher-order effect propagation, cleanup paths; separate program-verification layer | Pending; never route program-state predicates through PTC |
| Effects and staging | Distinct error/effect ABIs, handlers and runtime instances, deterministic inline effects; rank-1/two-stage/single-splice CMTT and quoted arms | Semantic execution tests for handler cleanup, evaluation counts, staging boundaries and hygiene | Pending |
| Projection and synthesis | Pipeline projection, typed intrinsic/asm access, BitVec values/locations/unsafe construction, legal baseline on synthesis Unknown | Per-stage projection tests, bit width/ordering tests, baseline-versus-optimized behavioral checks | Arbitrary-width BitVec value kernel implements checked literal conversion, bitwise operations, concatenation, slices and widening with bounded work and S-expression Display; fixed-integer source views, concrete-width type identity, contextual integer literals, eager bitwise operations, concatenation, constant indices/slices, equality, checked widening/unsigned conversion and exact-width signed/IEEE reinterpretation now reach NIR execution; explicit builtin BitVec construction accepts checked literal and closed computed Integer widths without unfolding normal-form heads; dependent width/value-argument elaboration, runtime endpoint verification, general location mappings, unsafe construction and synthesis remain pending; static local/field bit writeback now preserves RHS snapshots and intervening writes for fixed integer/IEEE float/BitVec storage; float views preserve raw IEEE special-value bits; static segmented patterns retain radix/width, typed subpatterns and bindings (including unrestricted signed/IEEE scalar fields) through execution; bounded symbolic coverage handles bit-mask unions with cross-column correlations and Unknown on exhaustion, computed BitVec-valued segments now run under required-comptime phase/contract/budget checks and become fixed constraints used by coverage; general scalar computed-segment conversion and representation layouts remain pending |
| IIR and executable backend | Non-SSA high-level IIR; completed type comptime, explicit runtime environments; lowering to current LLVM | Validated IIR dumps then executable semantic tests; each backend action has target/dependency evidence | IIR skeleton; LLVM 21 C API smoke probe passed separately |
| Library and anar | Real builtin/std protocol implementations; math intrinsic wrappers; packages, targets, build graph and cache invalidation | Versioned producer/consumer interfaces, compiler-backed library tests, reproducible build fixtures | Library exists but language coverage unverified; anar repository not located |

Advanced verified-mode proof and synthesis extensions follow the executable foundation. Safe-mode runtime checks remain part of the resource model, not a deferred replacement for memory safety.

## Development approach

Graph engineering is disabled at the user's request. Continue implementation directly in dev with one agent by default, reusing the workspace and build cache. Use the dependencies above to choose concrete work; do not create task graphs, scheduling ledgers or grading loops. Existing run records preserve past evidence only.

Elaboration and comptime still need coherent obligation and continuation interfaces. Library work starts against concrete compiler capabilities, and anar work starts after locating its actual repository and instructions. These are implementation dependencies, not prescribed agent roles.

Every new IR includes Display. Run tests for affected behavior; reserve broader integration checks for changes that warrant them. After literal foundations, prioritize stable semantic representation and a minimal source-to-elaboration path.

Interpreter stack robustness: expression and pattern dispatch now use separate non-inlined evaluators to keep recursive debug-build frames small. The ordinary-default regression uses the default depth 128 again; an explicit 2 MiB host-stack regression checks recursive calls, deep expressions and deep patterns at depth 128, 256 and a requested usize::MAX (clamped to 256), plus recovery after each error. This verifies those paths in the development build, not arbitrary host stack sizes; a heap-frame interpreter remains a future robustness/scalability option.

Dynamic opaque type construction now has a dedicated NIR expression: ordinary comptime calls evaluate head inputs in order while preserving opaque identity, and required invocation carries head metadata without treating head bodies as executable dependencies. Dynamic Type inputs work for unconstrained heads; later dependent parameter checking and formation contracts awaiting those Type inputs still need residual scheduling. Source and malformed-NIR regression evidence is recorded in development/README.md.

Nested normal-form expression identities can now be recovered structurally for available Type/scalar inputs, allowing definition-side constraints and generic specialization to retain opaque heads. Calls and computed dynamic expressions remain execution nodes, not speculative elaboration-time invocations; general residual input/contract scheduling remains required.

Normal-form formation obligations now substitute both Type and scalar declaration inputs into type predicates; contracts such as `T == Bits<N>` are checked with actual input identities, including symbolic forwarding. Unrelated dynamic scalar inputs do not delay independent type constraints. Normal-form type contracts depending on unavailable comptime inputs now remain in stable head descriptors and are checked after constructor input evaluation through the host PTC callback. Dependent argument elaboration and residual trait selection still require further scheduling support.

Residual normal-form contract invocation now preserves definition-time checks for available independent requirements, evaluates dynamic constructor inputs once, and checks instantiated predicates before producing a Type. Unknown/missing solver results fail closed; source required invocation installs PTC without importing program-state predicates. The interpreter and NIR contract/metadata regression evidence is recorded in development/README.md.

Dependent normal-form inputs now support already synthesized expression types and residual integer-literal conversion after earlier dynamic Type inputs arrive. The owned contextual input node is traversed by specialization, phase checking, materialization and required computation; typed arguments use the PTC callback for nontrivial equality. General expressions that need unavailable expected information, Float/literal protocols and residual trait selection remain incomplete.

Contextual dependent head inputs now support ordinary conditionals and blocks: both branches retain type obligations, only the selected path executes effects, and converted literal inputs are invocation-local. Visitors preserve generic substitution and phase checks across the new input forms. General contextual patterns/calls, literal protocols and trait scheduling remain incomplete.

Contextual dependent head inputs now include matches using the existing pattern/coverage checks and shared pattern/guard execution. Generic and required-invocation visitors reach extractors embedded in patterns; arm results retain ordinary checks without running unselected effects. Expected-type-dependent generic calls, other contextual constructs and residual trait/literal protocols remain incomplete.

Value-producing `when` now composes with contextual dependent head inputs using existing conditional/block IR, preserving ordered conditions, fallback rules, arm scope and ordinary branch obligations. General expected-type-dependent call elaboration and residual trait/literal protocols remain incomplete.

Normal-form input identity now covers builtin Unicode Char and immutable UTF-8 text values as well as Type/Bool/integers, including dynamic construction, dependent input checks and type contracts. Text identity is content-based and display escaping is shared across semantic/program IR; aggregate and general user-defined comptime value freezing remain incomplete.

Normal-form input identities now include nested immutable builtin tuples, distinct from tuple types, with prior Type dependencies in individual fields. Contextual tuple inputs defer field checks/conversion until those Type inputs arrive; ordered evaluation and formation contracts retain whole-value identity. Shared bounded freezing covers source and interpreter construction, including host integer size checks. General user-defined aggregate freezing, contextual generic calls and residual trait/literal protocols remain incomplete.

Builtin BitVec values can now participate in opaque head identities with exact width/content equality and bounded freezing. Dependent integer literals convert once the expected BitVec type arrives, preserving zero-width and overflow rules. Symbolic-width solving and general user-defined aggregate freezing remain separate unfinished work.

Explicit NIR truncating quotient/remainder now share the exact arithmetic kernel and report source-located division-by-zero errors with interpreter recovery. Source `/` and `%` selection is pending the author's signed rounding decision; do not infer that decision from the historical docs or the kernel name. Independent compiler work can continue.

Definition-side type applications now retain symbolic declaration values nested inside tuple inputs, including nested Type fields, rather than prematurely invoking those values. Formation contracts can instantiate all nested fields without head unfolding. General symbolic arithmetic expressions and user-defined aggregate identities still need representation/scheduling support.

Unit is now a canonical comptime value in opaque normal-form inputs, including nested tuples and contextual empty blocks. Void-valued effects still run before construction. This completes another builtin value case; the standalone frontend still needs prelude/library integration and general user-defined value freezing.

Late expected Type information now determines contextual tuple/unit syntax consistently with ordinary bidirectional checking. Prepared choices are invocation-local; both branches retain checks, selected field calls preserve order, and empty blocks are not implicitly promoted to types. Expected-type-dependent generic calls and general residual trait/literal scheduling remain unfinished.

Tuple-value Type fields now participate in bounded PTC forward congruence, and source contexts register checked pure normal-form heads. No inverse head inference or standalone program-value propositions are added. Applicability/coherence/materialization visitors and conservative candidate indexing now account for tuple values; general value countermodels remain conservative.

Owned-package consumers can now restore the same declaration theory through `ptc::Context::from_package`; materialization uses it and rejects invalid normal-form descriptors transactionally. Context reconstruction imports neither caller premises nor body equations.

Required invocation during contract formation now depends on the enclosing contract only when its recorded scope actually uses definition premises. Binder-only contract operands support terminating direct/mutual comptime recursion without assuming their own unfinished contracts; genuine contract cycles and nontermination remain distinct failures.

Static normal-form identity recovery now includes concrete bit operations and scoped mutation-free local construction blocks. It preserves executable expressions, leaves calls residual and accounts for copied semantic structure. General symbolic arithmetic and contextual generic-call elaboration remain unfinished.

Module-level const declarations now have resolver identities and completed typed-value caching through required invocation. Scalar/tuple/Type values can feed width/type formation, comptime calls and type observation across modules, with cycles and invalid phase/contract dependencies rejected. General constant aggregates and runtime Splice elaboration remain unfinished; module constants are not a blanket runtime embedding permission.

Module constant representation now includes checked generic records and enums with nested fields. Nominal descriptors can be prepared on demand for forward required-computation dependencies; caller scope is restored and real layout/value cycles rejected. This does not yet provide general closure/reference constants, runtime Splice or aggregate normal-form value freezing.

Named interpreter function instances now retain restorable declaration/type-argument identity, distinct from anonymous closure environments. Source function constants remain blocked by the existing first-class comptime callable phase-type limitation; representation groundwork must not be mistaken for completed callable phase support. Const callee routing now uses its value expression.

First-class named comptime callable support now replaces the earlier representation-only restriction: phase-bearing function types flow through source syntax, inference, constants/records, generic callback production, indirect invocation and NIR phase checks. A PTC-proven callable view is retained at each indirect call site. Positive execution and phase-forgery/recovery tests cover the source and interpreter boundaries. Next callable work includes anonymous comptime environments and additional callable protocols; runtime Splice and comptime trait dispatch remain separate unfinished work.

Anonymous comptime closures now preserve phase and captured values, including nested constant environments. NIR capture initializers restore saved values without introducing caller-local aliases; all affected specialization, phase, dependency and materialization visitors cover them. Partial callback contexts preserve environment shape instead of dropping it. Captured-state mutation remains guarded until resource/effect checking is implemented; environment-qualified source syntax, pointer/minimum-place capture and full Once/Clone/Copy semantics remain outstanding.

Explicit closed closure environments now use the documented `%{name: T, ...} F` syntax from parsing through execution, with ordered fields, empty-context elimination and preserved callable phase. Source expressions have HIR nodes but cannot yet be discharged as provenance contracts; open context parameters and dependent/non-callable environments remain unfinished. Generic type aliases are still unsupported and need their own binder-aware lowering and instantiation path rather than treating them as opaque normal-form heads.


Generic type aliases now have binder-aware resolution/lowering, transparent type instantiation, argument-specific bounded caching and independently checked trait obligations. Residual top-level alias computations support nested aliases without introducing new type identity. This supersedes the earlier blanket unsupported status. Next: residual structural type construction for computed fields under symbolic binders; value/default alias parameters and broader dependent type formation remain unfinished. See the generic alias section in development/README.md for validation and the reproduced structural limitation.


Residual tuple type construction is now connected end to end, including computed fields inside generic aliases and Type-valued comptime function parameters. NIR has a dedicated displayed node with interpreter, specialization and traversal support; field order, failure, budget and phase checks have regressions. This resolves the earlier computed-tuple limitation. Continue with residual function/closure/pointer/nominal constructors and broader dependent formation; the tuple extension does not complete those constructors.


Computed function parameter/result types now have residual NIR constructors with phase-preserving interpretation and complete affected traversals. Type-valued factory arguments and generic alias computations reach actual runtime/comptime callbacks in the source tests. Continue with computed closure environments, pointers and nominal constructors; named/dependent function parameter protocols and broader resource semantics remain separate work.


Computed closed closure environments and pointer pointees now have end-to-end residual construction, displayed NIR nodes and traversal coverage. Runtime/comptime reader factories execute captured callbacks in tests; empty environments eliminate. Pointer type identity is supported here, not memory operations. Continue with nominal constructors under computed arguments and general dependency/resource semantics; open contexts and source/provenance bindings remain guarded.


Computed nominal Type arguments now have a residual NIR constructor that validates complete declaration contracts at invocation. Struct/enum factories and aliases reach real consumers; forward declaration tests cover requirement metadata availability. Dynamic trait-bound construction remains conservatively rejected until comptime dispatch can establish the required implementation world. Generic computed layouts, value-dependent nominal parameters and the broader dependent elaboration pipeline remain unfinished.


Generic alias expression arguments may now be invocation-time Type values. Fresh local bindings preserve single evaluation, order, ignored argument effects and nested/captured uses, with an explicit expression-expansion work limit. This closes the alias-expression gap left by structural type constructors. Dynamic trait evidence and annotation sites requiring not-yet-available Type inputs remain outstanding; do not infer general dependent elaboration from these Type-valued expression paths.


The CLI now exposes the supported comptime interpreter through explicit invocation, sharing source elaboration, phase checks, package type contracts, default argument plans and atomic output handling. Generic/required-argument entries need concrete source wrappers. This enables direct example execution without a Rust test harness; it does not close the remaining resource, dependent elaboration, trait phase, IIR or native backend work.


Comptime Type equality/inequality now drives executable control flow using positive equality/disequality proofs. Missing equality evidence is not false; opaque comparisons remain unresolved without proof. This connects PTC to Type-valued factory conditions while keeping program-state verification outside PTC. General Eq protocols, contextual refinement and dependent branch elaboration remain outstanding.


Type-valued quoted references and computed expression patterns now use candidate-scoped equality guards, preserving ordered evaluation, rollback, conservative coverage and opaque-head uncertainty. The quoted AST node now correctly resolves the outer name. Structural type decomposition, reflection classification APIs and dependent branch refinement still need their own implementation; no normal-form inversion is introduced.


Tuple type observation patterns now bind Type-valued elements, including nested/reference/or patterns and generic specialization. A dedicated NIR pattern preserves the distinction from value tuples and never unfolds opaque heads; missing shape evidence remains unresolved. Normal-form application patterns, open structural views and reflection/library protocol integration remain unfinished.


Normal-form application patterns now observe retained declaration heads and bind independent Type/value arguments, with named routing, nested decomposition and candidate rollback. Ordinary factories are not inverted. Dependent value arguments support binding after a quoted constant Type pattern supplies their type, including generic specialization and reverse-order named patterns. Arbitrary pattern-bound Type dependencies, computed head expressions, branch relation refinement and general view protocols remain open.

Computed scalar value patterns now share executable equality elaboration with ordinary comparisons. Candidate-local evaluation/rollback, early returns, skipped candidates and failure propagation are tested; builtin Bool/Char/Str/fixed integer equality has explicit NIR. IEEE floating-point comparisons are now supported; general Eq protocol selection and aggregate equality remain separate unfinished work.

Range patterns now execute for Integer/fixed-width integer/Char values, retaining inclusive bounds, candidate-local endpoint evaluation and control flow. Parsing no longer captures range guards into the upper endpoint, and the full range parses. Interval-union coverage is now implemented for literal/negated-literal endpoints, preserving product correlations; custom ordering protocol selection and general symbolic endpoints remain unfinished; no endpoint arithmetic or implicit boundary wrapping is introduced.

Fixed-width integer/Char ordering now works in ordinary expressions, including bidirectional literal context, extrema, generic type-equality specialization and ordered control flow. Exact Integer comparison behavior remains available. IEEE f32/f64 comparisons are now implemented with explicit ordered directions and NaN-aware results. Custom ordering protocols remain unfinished.

Pure scalar arm guards now participate in bounded interval coverage, with all comparison directions and boolean combinations. Canonicalizing guards after complete candidate patterns preserves ordered or-selection and rollback. Calls, arbitrary symbolic outer values and nonlinear predicates remain outside this proof fragment; this does not add program-state propositions to PTC.

Comptime trait/inherent method calls now preserve phase through contracts, body selection, default methods, first-class values and generic/parent dispatch. Phase mismatch is rejected at source checking and interpreter dispatch; runtime invocation is not implicitly promoted. This closes the blanket comptime-interface rejection for the supported by-value method fragment. Operator protocol language-item registration, method-local generic telescopes, reference adaptation and resource obligations remain open.

Required comptime execution now includes method dependencies and a projected, materialized snapshot of the already checked trait world. Function-body type annotations and BitVec widths can invoke required/default/generic comptime methods; snapshot projection leaves the final program intact. World preparation now precedes unrelated declaration queries, so module aliases, Type constants, function signatures and nominal fields can invoke methods. Nominal identity is independent of layout readiness. Queries required to form the interface world itself still need true suspension/retry; current member dependency collection conservatively includes all implementations of referenced members.


Required invocation fallback collection now follows checked implementation tables: an overridden default is not forced unless an implementation actually inherits it or another reference needs it. Snapshot-only contract projection preserves final default validation and avoids false alias cycles. Concrete-instance dependency slicing across implementations remains unfinished.


Required function snapshots now obtain optional-argument dependencies from actual call plans instead of eagerly forcing all default templates. Explicit arguments can break false default/alias dependencies; used defaults still carry generic trait dispatch. Final default validation/metadata are preserved. Per-parameter preparation now uses independent owner/index queries with obligation replay and cycle detection. Calls demand omitted positions only; final checking still completes every default. Concrete-instance trait dependency slicing and interface-world suspension remain unfinished.


Prefix expression parsing now honors the existing 90-level binding power and propagates caller delimiters. Arithmetic/boolean prefixes no longer swallow low-precedence binary tails; postfix calls/projections remain inside their operands. Parser structure and executable truth-table regressions cover this correction.


Builtin f32/f64 arithmetic and sign negation now have displayed NIR nodes and interpreted execution for ordinary/comptime functions, generic specialization, stable defaults and required type computations. Per-operation rounding, special values and order/phase/type rejection are tested. Current binary-result NaN canonicalization is an interpreter representation choice; cross-backend payload policy, native lowering, dynamic FP environment and general comptime Float remain unfinished. Source Integer division still awaits its rounding decision.


Scalar arithmetic/comparison operands now exchange type context before numeric defaulting, using transactional incomplete attempts. Nested float computation and generic calls can acquire precision from their peer. Arithmetic dispatch is split out to reduce recursive stack use, with a 40-level source regression. Backward inference through completed local bindings and a general stack-safe elaboration scheduler remain unfinished.


Documented float NaN/Infinity associated constants now resolve from actual type identity, aliases and positive generic equality evidence. They preserve ordinary same-named user members and opaque-head boundaries. Dynamic dependent associated values and general runtime Splice remain unfinished.


A checked fixed-width integer value kernel now provides explicit representation, conversion, arithmetic failures, internal truncating division and stable Display; interpreter value validation uses it. Source machine arithmetic remains gated on program-verification preconditions, and no automatic wrapping or implicit runtime assumption has been introduced. Next integration requires explicit safety obligations in the program-verification layer, independently of PTC.


The program-verification arithmetic foundation now has separate versioned scalar identities, canonical degree-three polynomials, bounded interval proofs, machine-range targets and premise-aware replay. Work/term/coefficient limits return Unknown without publishing proof; assignments invalidate stale versions. This is an independent component, not PTC or a complete nonlinear decision procedure. Source checked assumes now has a dedicated HIR/NIR/interpreter path with failure termination, specialization/closure traversal and bounded predicate-shape admission. It publishes no PTC facts. Named predicate contracts, control-flow and alias facts, and per-operation machine-safety obligations remain the next integration work.

Program range facts now have branch snapshots with shared unique assignment versions, transactional boolean refinement, unreachable paths, conservative joins and analysis-bound proof replay. This prepares the control-flow consumer without treating successful checks as type propositions. The next consumer must translate source/NIR scalar conditions at their actual execution points, invalidate writes/effects, and prove each machine operation's prerequisites; the new API alone does not enable machine arithmetic.

An explicit NIR flow-analysis consumer now initializes scalar domains and follows source-order checks, assignments, branches, short circuits, return, unknown calls and conservative loop boundaries. Source-to-NIR tests inspect its actual range snapshots and arithmetic results. It is not yet the mandatory machine-operation gate: wire obligation checking into the frontend and required-invocation paths before enabling machine operators. Default argument frames, closure/match flows, relational/unbounded domains, precise effects and loop invariants remain explicit limitations.

Fixed-width add/subtract/multiply/negate now pass a mandatory program-range gate in final elaboration and required comptime snapshots, then execute through checked machine arithmetic. Explicit checks and branch bounds establish per-operation representability; the interpreter retains defensive overflow/type validation. Phase traversal discovers operations in nested/default code, and unsupported analysis cannot silently bypass the gate. Default argument analysis now uses a separate parameter frame. Pointer-width data, closure/match/pattern flow, generic/relational range evidence, precise effects, loop invariants, integer division protocols and native lowering remain unfinished.

Closure range analysis now separates creation-time capture evaluation from an independent, full-parameter-domain invocation state. Captured scalar bounds survive outer reassignment, nested and restored comptime closures are checked, and delayed body facts never escape into the creator. Flow dumps identify closure nesting. General alias captures, callable result/effect contracts, generic range evidence and match/pattern flow remain next integration work.

Scalar match/let-pattern range flow now tracks ordered success/failure, literal exclusions, guards, bindings, or alternatives, remapping and transformation expressions. Failed-candidate writes survive while speculative bindings roll back; scalar guarded arms can authorize machine operations and nested closure captures. Tuple/record/variant/type-view analysis and stronger relational evidence remain unfinished.

Tuple element snapshots and nested tuple patterns now participate in range flow, including local copies/assignment, branch result/state joins, capture/default frames, or remapping and sequential candidate rollback. Snapshot values discard source-local identities to avoid aliasing mutable sources or independently numbered closure values. Record/variant/type-view patterns, full product correlations and precise alias/effect summaries remain unfinished.

Package-aware range flow now resolves generic nominal field schemas, retains record/enum identities in aggregate snapshots, reads known record fields and checks record/variant patterns. Different variant joins lose incompatible payload information; known mismatches skip payload matching. Nested/captured/restored aggregates are covered. Precise field paths/writes, type-view patterns, relational/disjunctive aggregate evidence and effect summaries remain unfinished.

Direct record-field writes now update snapshots precisely after RHS evaluation, preserving siblings/unrelated locals and invalidating the root version. Nominal schemas survive unknown values/calls so nested and generic writes can materialize conservative untouched-field domains. Snapshots and joins retain full nominal arguments. General alias writes, field-path predicate refinement and callable effect/result contracts remain unfinished.

2026-10-08: Direct owned record field predicates now feed versioned program range facts, with prefix-overlap invalidation, sibling preservation, branch joins, short-circuit registration and snapshot materialization for copies/captures/patterns. Predicate validation admits intrinsic field access while continuing to reject calls. See `development/examples/checked-field-predicates.fl` and the README verification entry (931 related regression tests plus 3 expanded focused tests). General aliases, cross-function memory contracts and relations between unknown fields remain open; this is not a PTC extension or a completed memory safety verifier.

2026-10-08: Program arithmetic now connects NIR symbolic add/subtract/multiply/negate to the degree-three polynomial kernel, and checked comparisons between expressions establish versioned inequalities. Bound contraction plus residual-expression enclosure supports unknown-value/field comparisons, correlated differences, checked sums and cubic bounds without routing program propositions through PTC. Relational dependencies participate in proof replay and conservative branch joins. Evidence: 897 related regression tests, 21 final kernel tests and 2 expanded source tests; see README and `checked-relational-arithmetic.fl`. Full polynomial elimination, relation transfer across value copies/captures, cross-function contracts and memory permissions remain open.

2026-10-08: The independent program verifier now supports budgeted transitive difference closure, arbitrary integer coefficient isolation with directed rounding, negative-cycle detection and common implied differences across branches/disjunctions. Matrix storage is explicitly capped and falls back to the existing bounded procedure. Source fixture: `development/examples/checked-transitive-arithmetic.fl`; evidence: 905 related regression tests plus expanded focused checks and actual comptime/dump execution. Next gaps include relational snapshot/capture transfer and general cross-function contracts; this is still not a complete cubic solver or memory verifier.

2026-10-08: Checked program relations now survive direct scalar/record value copies, nested/generic fields, field writes, pattern binding/remapping and closure capture into a distinct analysis lineage. Capture evidence freezes at its evaluation point and remaps only matching source identities; transitive differences project away uncaptured intermediates. Field identity alignment before joins preserves relations across independently allocated branch paths. Evidence: 907 related regression tests, expanded copy tests, NIR capture-order checks and actual comptime/dump execution. Remaining gaps include constructor-element/tuple/variant relational snapshot assembly, full resource traits and cross-function memory contracts; this does not grant aliasing or ownership rights.

2026-10-08: Aggregate relation transport now freezes constructor components in evaluation order and rebases tuple/record/known-variant payload paths to owned analysis identities. Nested/generic constructors, captures and destructuring preserve the justified relations; source writes, tag changes and calls cannot silently substitute newer values into earlier snapshots. Tuple shape survives havoc while observations do not. Evidence: 910 related regression tests, focused dump validation and actual comptime execution of `checked-aggregate-relations.fl`. Remaining priorities include aggregate-result joins, source-level function contracts and the broader resource/effect pipeline.

2026-10-08: Control-flow results now preserve common relations across compatible aggregate and fixed-width scalar alternatives, including literal pairs, reassigned versions, dead branches and guarded match scope exit. Composed difference enclosure can consume several graph edges, split shared coefficients and retain nonlinear residuals. Current verification: 917 related tests, exhaustive small weighted-model checks and CLI result 19969 from `checked-composed-differences.fl`; prior aggregate-join evidence is now documented in README. This remains a budgeted incomplete program solver. Cross-function contracts, tag-partitioned variant evidence and the full resource/effect pipeline remain open.

2026-10-08: Added bounded program predicate entailment with replayable lineage/version/relation evidence, using enclosure and opposite-condition refutation. Range-flow `assumes` points expose their precheck inputs for optional proof queries; successful checks cannot prove themselves retroactively. Verification: 921 related tests, exhaustive small-domain truth/Boolean cases, actual source check counts/failures, and predicate/flow S-expression reader checks. Function program contracts are still a separate required milestone: current normalized `Function.requirements` contains type propositions only, and runtime specifications must extend callable contracts and invocation boundaries rather than enter PTC. Redundant-check elimination is not enabled yet.

2026-10-08: Optional check elimination now uses precheck entailment/replay, retains predicate evaluation, and publishes a rewritten function only after successful reanalysis. Shared closure bodies are detached before collecting evidence; identical source spans cannot confuse sites. CLI comptime invocation runs the pass, while normalized frontend output retains the baseline checks. Evidence: 925 related regression tests plus 4 final focused tests, actual CLI success/rejection and rewritten NIR/proof-report formatting. Program requires/ensures and their representation on callable contracts remain the next separate implementation gap.

2026-10-08: Direct-call program `requires` now has independent NIR storage, declaration-side elaboration, entry assumptions, caller proof obligations and interpreter entry checks. Ordered argument snapshot transfer preserves correlations through later argument writes; defaults cannot assume an unproved entry contract. Bool facts, generic instances and required-comptime snapshots are connected. First-class uses that would erase preconditions and unsupported member contracts are rejected explicitly. Evidence: 998 related tests, expanded positive/negative source checks, CLI result 155 and NIR reader/formatter validation. Remaining program-contract work includes callable-type representation/subtyping, trait methods, ensures/outcomes, unbounded domains and the broader memory/resource/effect pipeline.

2026-10-08: Named result bindings and program `ensures` now survive lowering/NIR and are proved on every reachable ordinary function exit, with separate closure-return contexts and runtime enforcement before result delivery. Direct callers receive result-only scalar/aggregate guarantees; parameter-exit facts are deliberately not equated with original inputs. Evidence: 1009 related tests, final source positives/rejections, CLI result 101 and NIR/flow formatting checks. Next contract gaps include old-state/effect-aware parameter summaries, callable types/traits, outcomes and full resource verification; automatic proofs remain bounded/incomplete.

2026-10-08: Explicit `'old` entry snapshots now connect dedicated HIR lowering, NIR metadata/Display, postcondition elaboration, invocation environments and program range flow. Ordered direct-call frames transport result/input relations, including owned-record fields and quadratic/cubic guarantees, without equating bare exit parameters to original inputs. Frozen input evidence survives conservative havoc; writes cannot mutate snapshot slots. Evidence: 1012 related tests, exhaustive bounded source execution, scope/stale/false-contract rejection, runtime failure recovery, CLI result 42 and NIR/flow reader checks. Remaining priorities include callable/trait contract representation, heap/effect footprints and outcomes, resource traits, higher-order effects and the broader lowering pipeline. Program verification remains bounded and separate from PTC.

2026-10-08: Trait/inherent/comptime methods now carry checked program requires/ensures independently of PTC. Implementation substitution maps names and snapshots, proves explicit precondition weakening and preserves inherited guarantees with entry-state guards for wider domains. Abstract trait callers consume declaration guarantees; interpreter calls enforce the interface entry boundary before the selected implementation. Required-comptime snapshots use the same inherited contracts. Tail `if`/`match` and explicit returns check guarantees before joins. Evidence: 1014 related tests plus expanded exhaustive-byte source checks, forged-call/result rejection and recovery, CLI result 42 and NIR/flow formatting. Next contract work includes higher-order callable contract representation and resource/effect summaries; full lowering, resource traits and backend/library integration remain unfinished.

2026-10-08: Replaced the IIR placeholder with rooted, concrete high-level function/type/interface instances and stable S-expression dumps. Source-to-CLI instantiation closes recursive/generic/closure dependencies, retains defaults and program contracts, selects only demanded method bodies and preserves opaque heads. Publication rejects unresolved types, unavailable runtime identities and dangling callable/type links; source metadata and existing output files survive failed requests. Evidence: 970 related regression tests, 56 final focused tests and Emacs reader/formatter validation (README). Full graph execution/codegen adaptation, value-store evidence and independent IIR operation/resource validation remain unfinished; this milestone does not establish native execution or complete memory safety.

2026-10-08: The reference interpreter now accepts retained IIR graphs directly, resolves callable bodies by declaration plus concrete arguments, and validates concrete nominal values against retained definitions. Execution keeps interface entry checks and concrete function pre/postconditions without consulting source templates or substituting them again. Evidence: 791 affected regressions; 9 final source tests include 12 × 256 input differential contract executions, source-free graph execution, enum patterns/defaults and forged-result rejection/recovery. Native lowering, general runtime-dependent environment/value-store adapters and full resource validation remain open.

2026-10-08: IIR validation now checks retained definition identity/uniqueness, interface/dispatch correspondence and method calling conventions before reference execution. Fourteen metadata mutations exercise both publication validation and interpreter admission; 62 focused tests passed. This closes structural graph integrity gaps, not semantic proof replay, full operation/local-scope verification or resource analysis.

2026-10-08: Added `--interpret <runtime-entry>` for source-to-IIR reference execution, distinct from comptime invocation. Both entry paths now share ordinary zero-supplied-argument preparation, retaining dependent default evaluation. Existing output safeguards and per-stage fuel apply. Evidence: 456 related tests and actual CLI result 41. Corrected an old instantiation CLI test that had used unsupported `-o` and therefore exercised usage rejection rather than instantiation failure. Native output, resource/effect completeness and general runtime-dependent environments remain pending.

2026-10-08: Started resource availability with a separate bounded owned-position state model: per-field present/empty/conditional states, partial takes, empty-slot installation, branch joins and retained leaf responsibilities. Zero-field aggregates retain a token; failures are transactional. Six focused tests cover nested paths and exhaustive three-field joins. This is not yet wired into source acceptance: next work must resolve Copy through actual trait protocols, classify consuming/observing operations, connect control-flow events, and establish Drop/alias/dependency obligations separately. No implicit structural Copy or PTC program-state extension was introduced.

2026-10-08: Added declaration-identity-based Copy classification against the existing capability world, including builtin scalar/pointer classification, qualified explicit impls, given assumptions and pending results. Source tests prevent name-based or structural nominal Copy inference and check generic type conditions. Eleven focused tests passed. Builtin protocol declarations and language-item identity registration are still prerequisites to source resource enforcement; the legacy builtin file lacks Once and uses obsolete syntax. Selection evidence must be followed by representation/resource legality checks before implicit copies are authorized.

2026-10-08: Hardened the existing intrinsic language-item registry for resource protocol binding: package-qualified declaration IDs replace local-only handles, Once/Drop concepts exist, duplicate identical registration is idempotent and conflicting identities cannot overwrite one another. Stable iteration and S-expression dumps added; 63 related tests passed. Inspection confirmed that sysroot VFS scanning does not yet compile builtin declarations into the driver's NIR package. Source protocol binding and cross-package frontend integration remain prerequisites; no attribute syntax or name-based resource semantics were invented.

2026-10-08: Elaboration and the driver now accept an explicit semantic package identity instead of forcing owner declarations into package zero. Current-package declaration parameters, opaque heads, interfaces and implementations use the supplied identity from creation; ordinary compile remains identity zero. Evidence: 958 affected tests, including source tests with two package IDs and a >u32 identity, separate opaque/nominal identities for identical source and coexistence of two simple instance graphs. Cross-package resolution/imports and sysroot protocol loading are still pending; callers own build-identity allocation.

2026-10-08: Added explicit multi-package resolver construction with per-package roots, direct dependency aliases and combined AST file offsets. Actual parsed-source tests establish public imports/reexports, provider declaration identity, private/package rejection, absence of transitive dependency visibility, malformed input rejection and distinct clause AST references for identical sources. 104 related regressions plus 6 final package tests passed. Next integration: feed all package ASTs/scopes through HIR and preserve each owner's source package during elaboration; CLI dependency loading and builtin prelude remain unfinished.

2026-10-08: Connected explicit source package sets through resolver, combined HIR and elaboration into executable NIR/IIR. HIR owner provenance selects each declaration's semantic package, including generic parameters and opaque heads. The runtime reference API can select entry names within a package. Evidence: 972 affected regressions and 6 final multi-package source tests for generic execution, nominal identity, opaque preservation, imported trait contract inheritance, orphan/visibility rejection and same-name entry isolation. Remaining integration: CLI/build-tool dependency inputs, frozen package interfaces/cache, builtin source migration, prelude and language-item binding.

2026-10-08: Explicit builtin selection now supplies canonical builtin dependency aliases and ordinary package-root prelude globs through multi-package resolution/compilation. Source tests preserve normal shadowing, identity deduplication, ambiguity, permissions, no automatic reexport, and no self-prelude injection into builtin. 114 related tests passed. Scope dumps now cover all package roots with stable structural output; 7 final resolver tests and Emacs escaping/formatting validation passed. Remaining: actual builtin library migration, protocol registration, CLI/build input wiring and source resource enforcement.

2026-10-08: Migrated the actual builtin primitive source module to explicit intrinsic aliases and implemented the reserved bool type expression independently of Boolean literals. Cross-package tests verify 26 exact primitive identities, execution and comptime storage rejection through aliases. 1,007 affected tests passed. Remaining builtin work: root/prelude migration, protocol declarations and trusted language-item binding; no owned String representation or full resource enforcement is claimed.

2026-10-08: Added transactional, identity-checked source trait language-item binding and a driver Copy query consuming that registry. Cross-package source tests reject spelling-based designation and non-trait targets; missing/stale/builtin bindings fail at consumption. 31 focused tests passed. This is identity/kind validation, not full resource protocol contract validation or automatic sysroot registration.

2026-10-08: Added whole-local source NIR move analysis with ordered delivery, branch joins, lexical cleanup obligations, early exits and root replacement/reinitialization. Driver protocol bindings now select intrinsic-copy/affine-move behavior; declared Copy without legality evidence remains pending. 37 focused tests passed. The pass is explicitly invoked and is not yet a default safety gate; fields/observations, patterns, loops, closures/default frames and cleanup execution remain required.

2026-10-08: Extended source move analysis to instantiated struct/tuple shapes and nested local field places. Partial moves preserve siblings, whole use requires every field, and replacement returns precise outstanding cleanup responsibilities. Driver uses package metadata for this path. 40 focused tests passed. Aggregate destructor/cross-field dependency validation and temporary projection residuals are still required; no full cleanup authorization is inferred from shape alone.

2026-10-08: Added irrefutable tuple/record/generic pattern ownership transfer and typed residual temporary cleanup paths. Bindings acquire each component once; wildcards and omitted record fields retain cleanup obligations. Overlapping bindings and failure-path patterns remain explicit limitations. 43 focused tests passed; full match/loop/closure/default-frame transfer and executable cleanup remain outstanding.

2026-10-08: Added closure construction capture delivery and independent, nested invocation-state reports. Persistent captured responsibilities survive invocation return; uncalled environments retain cleanup obligations. 46 focused tests passed. Closure calls now explicitly require environment-state transfer instead of incorrectly consuming the whole callable. Repeated invocation support and complete closure cleanup/escape verification remain pending.

2026-10-08: Known local closure calls now import current environment state, analyze invocation, join capture exit states and write back without moving the whole callable. Local transfer/assignment provenance and receiver generations prevent stale writeback. 48 focused tests passed, including repeated read-only versus consuming calls and conditional consumption. Unknown/dynamic/temporary callable provenance, defaults and full dependency/cleanup checking remain required.

2026-10-08: Added budgeted ownership fixed points for loops, label-aware break/continue cleanup, return exclusion and speculative report/edge rollback. Pure Boolean constant conditions preserve dead-path behavior after while lowering. 50 focused tests passed, including per-iteration reinitialization, nested labeled cleanup, closure environments in loops and budget-failure recovery. Termination/dependency proofs and full cleanup/default/match integration remain separate requirements.

2026-10-08: Added isolated declaration-parameter ownership frames for ordinary call defaults, complete-slot/order checks and final availability validation before callee transfer. Source scalar-default cases and direct normalized affine-default rejection pass without weakening the existing P3 restriction. Moved source ownership coverage into an independent resources test group. 53 related tests passed. Closure defaults, broader purity evidence and complete resource enforcement remain outstanding.

2026-10-08: Added unguarded match ownership transfer with per-arm bindings, generic enum payloads, continuation joins and known-constructor pruning. Reports now retain decomposition obligations and zero-field cleanup tokens. 56 focused tests passed, including loop/closure interactions and explicit guarded-match limitations. Tentative failure-path semantics and full cleanup/dependency validation remain required.

2026-10-08: Added provisional match binding preparation and read-only Copy guard observation, committing cleanup/decomposition only on candidate success and retaining fallback state on failure. Guard conditions now appear in match reports. 58 focused tests passed. Effectful guards and non-Copy guard delivery still require general failure-path semantics; no runtime-effect rollback is assumed.

2026-10-08: Added refutable let-else resource paths: provisional bindings commit only on success, failure retains subject cleanup and must diverge, and break/continue integrate with loop availability. Pattern reports retain path reachability and guards. 61 focused tests passed, including normalized fallthrough rejection and recovery. Effectful guard transfer and complete cleanup/dependency enforcement remain required.

2026-10-08: Corrected top-level wildcard treatment of existing positions per temporary-value semantics; new-value evaluation/cleanup remains intact. Ownership flows now retain declaration order and record reverse local/field cleanup independently of LocalId numbering, without reordering bindings on assignment. 65 focused tests passed. These are responsibility records; actual user Drop calls, access/dependency validation and temporary lifetime lowering remain open.

2026-10-08: Added owned temporary field projection with one base evaluation, instantiated nested paths and Copy/move-specific residual state. Parent temporary type/state survives even after its last field moves, for subsequent destructor checks. 68 focused tests passed. Actual Drop invocation, full-expression lifetime boundaries and partial-construction cleanup remain open.

2026-10-08: Added explicit temporary-scope metadata for complete expressions, conditions, matches, returns, function results and default arguments, with nested/closure context restoration. 71 focused tests passed. These boundaries guide future cleanup lowering; actual Drop, dependency checking and abrupt-exit/partial-construction handling remain unfinished.

2026-10-08: Incomplete construction/preparation tracking now preserves completed fields, call arguments and synthesized captures on explicit return/break/continue. Pending-frame depth separates enclosing preparations from loop-local ones; closure invocation scopes isolate callers; report entries retain lexical anchors for later cleanup-order integration. Related 78 tests passed (`/tmp/luna-incomplete-preparations-verified.log`). Next resource work remains executable cleanup planning with local/temporary interleaving and dependency/Drop evidence, broader control/effect transfer semantics, and eventual default frontend enforcement. This analysis API is not yet a complete resource safety gate.

2026-10-08: Ownership state retains enclosing aggregate construction independently from field liveness. Bounded cleanup hierarchies and typed cleanup-site candidates preserve object-level obligations at retirement/replacement and interleave pending preparations with locals at control exits. Verified 86 related tests (`/tmp/luna-ordered-cleanup-final.log`). This resolves the state-loss prerequisite for user Drop planning; candidates are not executable plans. Further work must integrate temporary/pattern responsibilities, fixed Drop/environment evidence, user Drop access and field-transfer contracts, conditional execution and runtime lowering before resource checking becomes a default gate.

2026-10-08: Active temporary ownership now participates in control-flow state and ordered cleanup candidates. Semantic boundary completion, conditional acquisitions, branch exits, loop-local boundaries and closure isolation are covered; assignment retains RHS owners through replacement/writeback. Verified 93 related tests (`/tmp/luna-active-temporaries-final.log`). Next resource prerequisites: complete parent-object handling during pattern decomposition, bind actual Drop/environment evidence at ownership establishment, apply access/post-Drop transfer contracts, and lower conditional candidate execution before enabling resource enforcement by default.

2026-10-08: Supported structural patterns now retain one residual subject hierarchy, including enclosing object construction responsibilities and root-relative selected enum layouts. Binding transfers, guard rollback, let-else failure and arm return ordering preserve the correct owner. Verified 100 related tests (`/tmp/luna-pattern-owners-final.log`). Resource work can proceed to fixed Drop/environment evidence and contract-checked cleanup; general pattern views, conditional execution and runtime/default-gate integration remain outstanding.

2026-10-08: Added source `*self` declaration/body typing and reference-receiver metadata with trait mode checks, enabling the documented Drop signature. A language-item-bound Drop selection API now retains concrete method/type substitutions, instantiated program contracts, capability demands and world-scoped type evidence; unresolved cases never erase cleanup. Broad receiver/frontend/execution/resource regression: 723 tests; final focused selection checks: 10 tests (`/tmp/luna-drop-receiver-regression.log`, `/tmp/luna-drop-selection-final.log`). Next: bind selections and runtime environments at resource acquisition, preserve them through transfer, and check destructor accesses/post-state before integrating actual cleanup execution. Implicit reference-receiver call lowering and general pointer access still remain separate prerequisites.

Runtime pointer execution now reaches NIR/IIR and the reference interpreter: local/field addresses, intrinsic Copy reads/writes, field pointer projection and exact-target field reads, with weak storage identity and expired-target checks. Explicitly supplied reference receivers execute field-access bodies. Program arithmetic writes havoc alias-dependent observations. Evidence: 720 affected regression tests (`/tmp/luna-pointer-regression.log`) and 10 final pointer execution tests (`/tmp/luna-pointer-final2.log`). Next resource integration must supply access/dependency evidence, owner transfer and executable Drop before enabling a safety gate; dynamic pointer checks and destructor selection alone do not establish those contracts. General temporary addresses, receiver adaptation and struct-body Self binding remain missing. No commits made.

Nominal-body method binding is now implemented: struct/enum methods inherit parent Self/type/contract context and register in existing inherent selection/coherence. Qualified enum method calls no longer fail in the variant-construction probe. Source-to-IIR execution and rejection coverage passed in the 678-test affected regression (`/tmp/luna-nominal-methods-final.log`). This resolves the struct-body Self gap recorded in the pointer milestone; receiver adaptation, method-local generics and actual resource cleanup remain pending.

Builtin receiver adaptation now connects ordinary method syntax to `*self`: bounded shallow-to-deep pointer lookup, fixed explicit trait source, subject-specific capability lookup, existing pointer reuse, local/field address formation and intrinsic Copy pointer reads. Unresolved candidates and selected-call failures do not fall back. 745 affected tests passed (`/tmp/luna-receiver-regression.log`). User conversion protocols, temporary storage, owned pointee transfer and resource proof/cleanup execution remain outstanding.

Explicit dereferenced field places now preserve storage through read/write/address and reference-receiver adaptation, including nested generic fields and explicit pointer hops. They reuse pointer NIR instead of delivering the whole parent aggregate. 737 affected tests passed (`/tmp/luna-dereferenced-place-regression.log`). Static access proofs, resource transfers, destructor execution, temporary storage and indexed pointer places remain pending.

Explicit ownership analysis now consumes pointer operations for definite local targets, checking the same field availability used by moves. Provenance participates in branch/loop state and is invalidated on retirement/indirect slot writes. Unknown-call memory effects remain an explicit blocker after address exposure; packing pointers in aggregates cannot bypass this check. Evidence: 99 affected tests plus four final local-pointer test groups (`/tmp/luna-local-alias-regression2.log`, `/tmp/luna-local-alias-final3.log`). Next work needs interprocedural memory summaries and escape/dependency evidence before this can become default safety enforcement or executable Drop cleanup.

Body-derived pointer access summaries now cross eligible direct-call boundaries in explicit resource analysis. Parameter/field footprints are checked against caller ownership after source-order argument preparation; sequential aliases are legal, and later argument moves invalidate prepared targets correctly. Empty known footprints allow access-free calls after address exposure. Evidence: 509 affected tests (`/tmp/luna-memory-summary-regression.log`). Composition/recursion, generic/default/dynamic calls, escapes, pointer-slot effects and destructor/resource transfers still need richer summaries before default safety enforcement.

Generic/direct-call memory summary composition is connected to explicit resource analysis. Instantiated signatures and argument identities are retained; callee field footprints rebase through caller arguments, and signature disagreement/missing bodies/cycles fail explicitly. Evidence: 104 related regression tests and 3 final composition tests (`/tmp/luna-memory-compose-regression.log`, `/tmp/luna-memory-compose-final.log`). Recursive fixed points, default/dynamic calls, pointer-slot and escape effects, owned transfers and actual Drop remain outstanding.

Recursive memory footprints now converge using an iterative function-instance work queue and canonical access sets. Only a stable complete round is published; body errors/exhaustion discard provisional empty seeds. Structural type and shared work budgets bound growing instances. The initial recursive host-stack implementation failed a growth stress test and was replaced; current evidence is 108 affected regression tests plus 4 final fixed-point tests, including 151-function chains and exponential type growth (`/tmp/luna-memory-fixedpoint-regression.log`, `/tmp/luna-memory-fixedpoint-final.log`). This establishes footprints for supported recursive bodies, not termination or general resource safety.

Local pointer provenance now retains finite sets across branch/loop joins and simple pointer-valued selections. Reads/writes and composed-call footprints check every possible owned path; scope retirement and pointer-slot writes conservatively invalidate affected provenance. Evidence: 112 related tests (`/tmp/luna-pointer-sets-regression.log`), including all eight possible unavailable owners in a multi-branch target family. Relational path correlation and richer pointer result/escape effects remain pending.

Captured Drop selection now feeds a local access suitability check against ownership state and typed field layouts, without cleanup-time reselection. Partial field moves are admitted only when the captured body's supported footprint remains available; incomplete/conditional parent construction does not authorize a whole-object destructor. Raw pointer representation reads do not infer pointee lifetime. Evidence: 117 affected tests (`/tmp/luna-drop-access-final.log`). Acquisition-time binding/environment capture, retained program obligations, arbitrary post-Drop ownership and actual cleanup lowering remain unfinished.


Default memory arguments now use isolated prepared-parameter frames, declaration-ordered defaults after all explicit arguments, and caller ownership checks for each derived footprint. Pointer-representation copying is admitted by the source P3 default filter without admitting pointee reads or helper calls. Actual execution verifies single evaluation and override identity; malformed normalized plans are rejected. Evidence: 192 affected tests (`/tmp/luna-memory-default-resource-regression.log`, `/tmp/luna-memory-default-source-regression.log`). General stability evidence, owned defaults, dynamic-call contracts, pointer-result/escape provenance and executable Drop remain outstanding.


Pointer-valued blocks retain parameter provenance after their effects and local bindings have been analyzed. This covers nested defaults, wrapper arguments, read/write field paths, and rejection after owner moves. Evidence: 112 resource/Drop/instantiation regressions, two execution tests, and a final field-write test (`/tmp/luna-memory-block-*.log`). Conditional summary targets and interprocedural pointer-return/escape contracts remain open.


Conditional memory summary targets now form ordered finite sets, compose through field/default/wrapper paths and expand each access to all possible targets. Unknown alternatives remain unknown; duplicates collapse and expansion is budgeted. The default-result API now carries a target set, consumed by caller ownership analysis. Evidence: 115 resource/Drop/instantiation regressions, 3 execution tests and 1 dump test (`/tmp/luna-memory-alternatives-*.log`). Relational path predicates, pointer-return/escape contracts, general P3 proofs and executable cleanup remain outstanding.


Parameter/field-relative pointer results now participate in generic/recursive memory fixed points and caller ownership analysis. Call argument snapshots are propagated once through nested pointer calls, projections, reads/writes, assignment, blocks and branches. Local-root retirement invalidates returned local addresses, while opaque loaded pointers do not gain pointee evidence. Evidence: 121 related regressions, four execution tests, and five final result-contract groups (`/tmp/luna-pointer-results-*.log`). General storage/heap escapes, pointer stores, ownership transfers, path-correlated postconditions and executable cleanup remain incomplete.


Local pointer reassignment, conditional/short-circuit origin joins and budgeted loop origin fixed points are now supported in memory summaries. Unknown origins remain explicit; intermediate loop assignments contribute to the invariant even before early exits. Default frames isolate their local origin updates. Evidence: 126 resource/Drop/instantiation regressions and five execution tests (`/tmp/luna-memory-rebinding-regression2.log`, `/tmp/luna-memory-rebinding-execution.log`), including budget failure/retry for 12 rotating parameter origins. Pointer-slot stores, ownership transfer, relational final states, general escapes and executable cleanup remain open.


Local pointer-slot contents now use full place keys: exact writes replace origins, possible writes preserve old/new alternatives, and reads recover known stored origins. Parent replacement and scope retirement discard stale slot facts; copied pointer values keep their own origins. Evidence: 130 resource/Drop/instantiation regressions plus an execution/source-order test (`/tmp/luna-pointer-slots-regression2.log`, `/tmp/luna-pointer-slots-execution.log`). Aggregate-construction/move origin trees, interprocedural pointer-slot transfers, general escape checking and actual Drop cleanup remain unfinished.


Aggregate local delivery now carries per-field pointer origins through tuple/record construction, nested local/field moves, self-moves, replacement, blocks and conditional results. Field effects are captured in declaration order and pointee identities are preserved independently of container storage. Evidence: 133 affected regressions, one execution/call-order test and three final expanded source groups (`/tmp/luna-aggregate-origins-*.log`). Pattern/variant delivery, temporary projections, aggregate call-result contracts, interprocedural storage effects and executable cleanup remain open.


Pointer origins now transfer through let/let-else/match binding paths, including nested aggregates and enum payload constructors. Read-only guards can observe known pointer targets while checking slot/pointee availability; rejected candidates do not commit origin facts. Evidence: 136 related regressions, one actual subject-evaluation test and four final pattern groups (`/tmp/luna-pattern-origins-*.log`). Discriminant-correlated origins for differently shaped variants, match result delivery, general view patterns and interprocedural aggregate/storage contracts remain open.


Match now delivers pointer/aggregate origin trees through normal arm exits while preserving guard isolation, cleanup and ownership joins. Retired arm/pattern slots cannot escape as valid pointer results; divergent/failed candidates do not pollute result origins, and unknown live alternatives remain unknown. Evidence: 142 related regressions and three origin execution tests (`/tmp/luna-match-results-regression.log`, `/tmp/luna-match-results-execution.log`). Temporary projections, interprocedural match/aggregate contracts, variant-correlated origins and executable cleanup remain outstanding.


Temporary projections now deliver selected pointer/aggregate origin subtrees after a single origin-aware base evaluation, preserving residual parent/sibling cleanup. Evidence: 145 affected regressions and four execution tests (`/tmp/luna-projection-origins-regression.log`, `/tmp/luna-projection-origins-execution.log`). Interprocedural match/aggregate contracts, precise variant correlation, general storage/escape effects and executable Drop remain open.


Scalar/pointer match bodies now contribute composable memory footprints and pointer result origins, including readonly guards, explicit returns, OR/remapped bindings and recursive wrappers. Syntactic match facts are shared with ownership analysis. Evidence: 149 related regressions plus one recursive execution test (`/tmp/luna-memory-match-regression2.log`, `/tmp/luna-memory-match-execution.log`). Aggregate subject/result transfer contracts, arbitrary guard effects, variant-correlated origins and executable Drop remain outstanding.

Acquired structural cleanup plans now bridge fixed Drop selection and ownership-state/access checks: capture per-type field bindings before cleanup, then emit parent Drop, reverse remaining-field steps and ownership release without reselection. Trusted LangItems binding is available through luna::protocols. Evidence: 188 affected regressions, five dedicated groups and an Emacs-readable S-expression plan (`/tmp/luna-cleanup-plan-*.log`, `/tmp/luna-cleanup-plan-formatted.lisp`). Actual acquisition/move wiring, conditional/variant guards, runtime environments, general state-changing destructor contracts and executable cleanup remain required before this becomes automatic Drop.

2026-10-09: Acquired Drop cleanup plans now support enum payload dispatch and explicit relative-path variant evidence, including nested instantiated enums and partial payload moves. Evidence must be correlated through known enclosing variants; aggregate arity is checked against original state rather than sparse remaining cleanup children. Drop/resources/instantiation: 192 tests passed; enum plan dump passed Emacs parse/format structure verification. Next integration remains acquisition/transfer of these bindings with actual owner responsibilities and executable cleanup; conditional construction and general destructor post-state contracts are not supplied by this milestone.

2026-10-09: Wired acquired cleanup into source ownership analysis through an explicit immutable-world session. Acquisition happens at local/temporary/prepared-value responsibility establishment, captured child layouts are reusable after moves, and cleanup only looks up acquired choices. Supported candidates now carry checked plans; CLI `--drop-protocol` exposes the stage and preserves the existing obligation-only mode. Drop/resources/instantiation/frontend CLI: 255 passed, source example and complete report S-expression formatting verified. Remaining: runtime environment transport, guarded construction, general destructor state transitions and actual cleanup lowering/execution; this analysis does not execute Drop.

2026-10-09: Added concrete cleanup IR, lowering from revalidated acquired plans, explicit owner-type roots in materialization, and reference execution on real Storage with record/tuple/tagged-payload pointers. Explicit variant guards precede effects; actual captured IIR method instances execute; normal completion releases positions and runtime failure halts later steps without rollback. Seven execution groups now cover shared-storage writes/order, tag choice/mismatch, generic methods, type-only layouts and exhausted execution. 470 luna regression tests and 29 IIR/interpreter tests passed before the final two cases; all seven cleanup groups subsequently passed. Next remains inserting cleanup into actual source control-flow lifetimes, including partial representations, runtime environments and unwind obligations. The new standalone executor does not alter ordinary --interpret semantics.

2026-10-09: Added per-concrete-function cleanup schedules and source publication to a fixed point over ordinary functions plus captured destructor roots. The opt-in interpreter path now executes local cleanup at scope/function/return/loop/replacement/predicate-failure boundaries, including callee/caller responsibility transfer and preserved field storage. Failed destructors stop further cleanup. Eight source execution groups passed; combined luna regression 694 and IIR/interpreter regression 29 passed; schedule plus graph Display verified with Emacs. Remaining major integration work: temporary/prepared runtime frames, closure/default environments, postcondition result ownership, interface precondition frames and default driver integration. Ambiguous or unmapped sites currently reject publication instead of silently skipping cleanup.

2026-10-09: Mapped verified whole discarded results to source full-expression hooks. Constructors, call/generic/tuple/enum results, moved locals and moved fields now execute captured cleanup at the discard boundary. Failed evaluation acquires no result cleanup; failed destruction stops later cleanup. Five new execution groups passed in a 641-test luna regression, plus 29 IIR/interpreter tests; Display verified with Emacs. Remaining temporary work includes aggregate projection residuals, match/let-pattern subjects and prepared constructor/call frames, followed by closure/default environments and driver integration.

2026-10-09: Added runtime prepared-value frames for record/tuple/variant construction and calls, plus a typed local/prepared target union in scheduled cleanup IR. Frames are isolated per invocation, consumed on cleanup or successful transfer, and truncated on nonlocal exits without losing enclosing constructor frames. Seven execution groups validate interleaved order, normalized argument slots, nested/callee/loop failures and destructor failure. Luna regression 647 plus IIR/interpreter 29 passed before the final enclosing-frame test; all seven prepared groups then passed and Display was verified with Emacs. Default alias environments, interface precondition frames, aggregate temporary residuals and closure/postcondition ownership remain separate work.

### 2026-10-09 — Default cleanup context integration

Implemented concrete call-site/parameter context paths for default reports, IIR schedules and interpreter lookup. Caller prepared frames suspend during defaults; shared parameter storage transfers once to the callee or remains available for failure cleanup. Nested default rejection restores each responsibility boundary; destructor failure stops propagation. Missing contexts reject before effects. Six normalized execution groups and 654 affected luna tests plus 29 IIR/interpreter tests pass; executable dump verified by Emacs. See development/README.md for commands and evidence.

Next resource work: connect memory-summary defaults to context reports and establish pointer-parameter pointee provenance for scheduled destructor bodies. Do not treat normalized default tests as permission to widen source P3. Result postconditions, interface precondition preparation, closures and residual temporary projections still need scheduling. No commits made.

### 2026-10-09 — Scheduled borrowed Drop receivers

Captured Drop calls now supply conditional borrowed receiver provenance to method analysis; pointee storage never becomes a method-owned cleanup responsibility. Per-site acquired access checks remain authoritative for partial owners. Pointer-free aggregate construction footprints admit owned destructor locals and conservatively preserve possible cleanup rejection; pointer-bearing aggregates remain unsupported. Seven new end-to-end groups pass, including generic tuple/enum locals, parent writes observed by children, partial-field rejection and destructor-failure stopping. Final integration: 661 luna plus 11 ownership tests; dump formatting verified. Default shared storage is now also observed by the actual Input destructor on success and failure.

Next: unify memory-summary default environments and general pointer helper contracts with execution scheduling; do not grant arbitrary pointer parameters the acquired receiver assumption. No commits made.

### 2026-10-09 — Memory-call defaults join cleanup scheduling

Replaced footprint-only default handling in `memory_call` with isolated ownership evaluation plus external memory access checks. Prepared pointer aliases compose through separate borrowed roots, including chained defaults; recursive parameter IDs cannot collide with caller storage identities. Failure now publishes a `DefaultFailure` boundary and nested local cleanup report. Three execution groups pass, along with 663 affected luna tests and 11 ownership tests; S-expression dump verified. Normalized effectful defaults do not change source P3 admission. Next: general pointer helper invocation contracts and remaining cleanup footprint/scheduling gaps. No commits.

### 2026-10-09 — General pointer helpers in scheduled execution

Generalized conditional borrowed roots from a Drop receiver to indexed pointer parameters, with signature/duplicate/index checks. Non-entry helper assumptions require an inferred concrete memory footprint; callers retain actual-target checks, and no parameter disjointness is inferred. Six execution groups cover composed destructor helpers, overlapping arguments, generic returned/default aliases, recursive calls, rejection ordering and partial-owner rejection. Final checks: 670 luna and 11 ownership tests, plus dump structure verification. Remaining work includes pointer-bearing aggregate dependencies, contract-boundary scheduling, closures and temporary projection residuals. No commits.

### 2026-10-09 — Postcondition result ownership in executable schedules

Added return-position-aware postcondition failure boundaries to ownership reports and IIR schedules. Interpreter checks explicit returns before return cleanup, fallthrough after body cleanup, and never rechecks a completed explicit return. Named/unnamed result storage remains owned until all checks accept; failure cleans that result and surviving frame responsibilities, with destructor failure stopping subsequent cleanup. Four execution groups plus 674 luna and 29 core tests pass; dump validated. Source proof admission unchanged. Next scheduling gaps: interface precondition argument frames, closures, residual temporary projections. No commits.

### 2026-10-09 — Trait interface precondition preparation

Added an interface-domain failure boundary before prepared arguments transfer to the implementation. Declaration-side predicate evaluation and caller-side cleanup now use their respective environments. Both explicit and default argument preparation retain their responsibilities through the check; failures clean arguments, surrounding incomplete construction and caller locals in order, stopping on failed destruction. Three end-to-end groups and 677 luna plus 29 core tests pass; dump structure verified. Source proof admission unchanged. Remaining scheduling gaps: closures and residual temporary projections; pointer-bearing dependencies remain separate work. No commits.

### 2026-10-09 — Retained storage for residual field projections

Added temporary storage targets and explicit temporary-end boundaries. Field projection retains its base representation once; captured cleanup plans destroy only remaining responsibilities. Storage is suspended across ordinary calls/default contexts, and successful block cleanup follows analysis order rather than a fixed local/temporary ordering. Six new execution groups plus the upgraded discarded-projection regression pass; final verification is 683 luna plus 29 core tests, with S-expression dump verification. Remaining: closure environments, temporary pattern subjects/conditional guards, pointer-bearing aggregate dependencies. No commits.

### 2026-10-09 — Let-pattern subject storage and acquisition

Acquire subject cleanup before decomposition; retain the committed residual subject or failed candidate's intact subject at distinct runtime sources. Five execution/rejection groups cover struct, tuple, enum let-else, guards and moved-field access. Integration: 687 luna tests; final five-group run and dump verification passed. Next: runtime presence guards for branch-dependent match subjects and arm storage registration, then closure environment scheduling. No commits.

### 2026-10-09 — Match subjects and conditional temporary presence

Lower conditional temporary presence to optional storage targets; register subjects only for committed match arms and clean non-block arm bindings before leaving their environment. Missing optional slots skip cleanup, missing required slots remain errors, and duplicate storage identities cannot differ merely by optionality. Five execution groups pass, including guard rollback, dynamic enum branches, return transfer, repeated iterations and failing destruction. Final validation: 693 luna plus 29 core tests; dump structure verified. Next major resource gap is closure environment scheduling, with pointer-bearing aggregate contracts still separate. No commits.

### 2026-10-09 — Closure environment storage and construction

Added capture-aware cleanup pointer projections and source prepared capture frames. Construction-only closure reports no longer force source scheduling rejection; unevaluated closure bodies are excluded from the enclosing hook environment. Nested environment and discard cleanup now execute, and failed capture initialization destroys only completed captures plus caller responsibilities. Actual invocation remains explicitly rejected pending contextual scheduling and persistent environment-state writeback. Six new standalone/source groups; final verification 699 luna plus 29 core tests and dump structure check. No commits.

### 2026-10-09 — Local closure invocation scheduling

Publish concrete invocation reports under typed closure/default context paths. Share scheduled capture cells across calls and destructor projections, propagate partial capture availability, and preserve caller storage when entering/leaving closures. Captureless callable cleanup is atomic. Integration evidence: 704 luna and 29 core tests; final 11-group closure run adds failure/reuse, explicit-return contract isolation, same-site loop repetition and missing-context rejection. S-expression formatting verified. Source capture mutation remains gated; normalized storage tests do not authorize it. Next work includes higher-order callable provenance and pointer-bearing aggregate resource contracts. No commits.

### 2026-10-09 — Callable provenance through closure captures

Preserve recursive known callable definitions across environment construction and binding moves, then import them for nested invocation while obtaining resource availability from actual live owned positions. Added bounded extraction, initializer invalidation checks and explicit rejection of captured-callable replacement until provenance can be written back. Eight new groups cover nested transfer/reuse, repeated reads, failure cleanup, moves, non-invocation and stale provenance. Integration 716 tests; final targeted eight-group run passed. Remaining: callable return/parameter provenance, captured replacement and source mutation effects. No commits.

### 2026-10-09 — Callable arguments to local closures

Snapshot callable provenance during ordered argument preparation and install it into closure parameter frames. Context-specific nested invocation now handles owned callback captures and failure cleanup. Construction-only unknown callbacks become explicit deferred report obligations; executable cleanup rejects unresolved obligations. Eight source/report groups pass; integration 723 tests and ownership 11 tests pass, with both executable and report S-expression verification. Remaining: named higher-order function contexts, returned callable provenance, pointer-bearing callback contracts and source captured-state writes. No commits.

### 2026-10-09 — Conditional callable premises for named function analysis

Added validated concrete callable parameter inputs to ownership and acquired-cleanup analysis. Reports preserve exact definitions as explicit premises; callback resource consumption controls remaining cleanup, and same-typed bodies remain distinct. Five new groups pass. Verification: luna check; drop133/execution424/instantiation13 passed; a resources dump-prefix regression was fixed and all160 resources tests passed on rerun. Final S-expression verified. Named-call execution is not yet wired: next add ordered caller premise discharge, context-specific publication and interpreter selection, including contracts/defaults. No commits.

### 2026-10-09 — Named higher-order invocation contexts

Connect ordered prepared callable inputs to concrete named-function analysis and publish per-call cleanup contexts. Runtime selects those contexts before argument transfer; named contracts use the current named function, and missing contexts reject before body effects. Preserve nested callable provenance through defaults, forwarding and generic instances. Integration 738 plus core29 pass; final12 named-callback groups pass after four extra boundary groups. Executable S-expression verified. Defaults/contract failure fixtures preserve existing source admission. Next gaps include named function values as callbacks, indirect callees, returned callable provenance, pointer-bearing contracts and recursive context fixed points. No commits.

### 2026-10-09 — Named function values and known indirect callees

Track named/generic function identities through locals, parameters and captures; resolve known indirect calls before argument evaluation and reuse contextual publication. Validate callable premises against target phase, arity and exact instantiated signature. Recover formal bindings for indirect memory helpers; reject malformed frames instead of indexing out of bounds. Seven execution groups plus two resource validation groups; integration751 and final seven-group run passed. S-expression verified. Next: returned callable provenance and branch-selected targets; higher-order pointer contracts and recursive context fixed points remain open. No commits.

### 2026-10-09 — Callable value results and local factory returns

Preserve evaluated callable provenance through block scope exit, let/assignment and argument preparation. Record successful exit definitions separately from failure states and propagate a common result from known local closure calls; named references compare identity rather than use span. Clear unrelated argument/local provenance at unknown call boundaries. Seven execution groups cover returned environments, explicit returns, argument transfer, same-target joins, repeated-consumption rejection and failed construction; two report groups cover stale result prevention and normal/failure separation. Validation: drop159/execution424/instantiation13, then resources163 after disambiguating a fixture type; final7+2 targeted groups and both dumps verified. Remaining: named-function return propagation, differing branch targets, immediate returned-closure calls, aggregate fields and recursive summaries. No commits.

### 2026-10-09 — Named-function callable return summaries

Analyze instantiated named bodies with prepared callback premises and propagate their common successful result definition to callers. Support ordinary/generic factories, callback identity/forwarding and returned environment cleanup. Nested work is charged to the parent analysis budget; recursive dependencies stay unknown and exhaustion fails explicitly. Summaries do not replace materialization or acquired-cleanup verification. Integration769 and final7 execution +2 report groups pass; executable/report dumps verified. Remaining: differing branch targets, immediate returned-closure invocation, aggregate fields, pointer contracts and recursive fixed points. No commits.


### 2026-10-09 — Immediate callable invocation and full-expression receiver lifetime

Support evaluated temporary callees through synthetic ownership roots, runtime callee retention and full-expression residual transfer. Preserve argument/body failure cleanup, early returns, capture consumption and outer-call ordering; named temporary targets reuse concrete call frames. Integration779 and core29 pass; final12 targeted groups add repeated loop storage and chained receiver transfer. Executable S-expression verified. Remaining: differing branch targets, aggregate-field provenance, higher-order pointer contracts, recursive context fixed points and ordinary CLI schedule integration. No commits.


### 2026-10-09 — Resource-checked CLI reference execution

Connect `--interpret` plus explicit Copy/Drop identities to acquired cleanup publication and scheduled interpretation. Share protocol world construction with ownership-reporting CLI; prepare default wrappers before publication, propagate all failures without unchecked fallback, and retain atomic output behavior. Four new black-box groups, luna check and 664 integration tests pass; executable source example returns true. Remaining: automatic language-item registration/default safe execution, owned-default P3 evidence, differing callable branch targets and recursive contextual fixed points. No commits.


### 2026-10-09 — Stable aggregate construction in runtime defaults

Admit directly delivered owned record/variant/tuple defaults and conditional constructed results. Keep hidden owned cleanup and unknown effects outside the sufficient proof; owned-field construction requires infallible field computations so partial failure cannot hide Drop. Source preparation and scheduled execution preserve default result ownership, explicit overrides, repeated construction and nested cleanup. Integration669 plus final2 cleanup groups pass; executable S-expression and updated source example verified. Remaining: general totality/purity summaries, owned local transfer in defaults, callable defaults, automatic protocol registration and broader callable provenance. No commits.


### 2026-10-09 — Callable defaults from source

Separate code/environment construction from invocation in runtime default admission. Accept direct function values and closures with stable capture delivery; retain contextual execution, default-frame ownership checks and body contracts. Replace the hand-normalized default callback test with source compilation. Three new CLI groups and 672 integration tests pass, including distinct captured inputs, dormant bodies, returned callbacks, override, failure and affine-input rejection. Remaining: general default purity/totality summaries, owned-local transfer, automatic protocol registration and broader callable provenance. No commits.


### 2026-10-09 — Indirect named pointer-result calls

Share exact local callable resolution and formal-frame recovery between ordinary and pointer-result evaluation. Preserve pre-argument target selection and validate actual local availability before reusing the concrete memory summary. Four new groups cover writes, mutation ordering, moved-owner rejection, generic/factory identities; 774 integration tests pass. Remaining: temporary pointer callees, arbitrary higher-order memory summaries, differing callable targets and aggregate provenance. No commits.


### 2026-10-09 — Temporary named pointer calls and memory-neutral code values

Return pointer target sets from the temporary-callee path while retaining receiver lifetime and failure cleanup. Admit named function values/results in memory summaries without executing their bodies; preserve verified callable-result provenance after memory-checked calls. Five execution groups plus one summary group and 780 integration tests pass. Arbitrary captured callable memory contracts, branch-selected identities and aggregate provenance remain incomplete. No commits.


### 2026-10-09 — Local named aliases inside memory inference

Track named instances with pointer targets in the summary control-flow environment. Preserve exact agreement across continuing branches; merge loop assignments conservatively, isolate defaults and snapshot the callee before arguments. Reuse concrete nested summaries with recovered formal frames. Three summary groups plus one executable group added; 784 integration tests pass and summary S-expression verified. Remaining: different-target callable sets, callback parameter memory premises, aggregate fields and general closure memory contracts. No commits.


### 2026-10-09 — Function-valued expression results in memory summaries

Snapshot callable results while branch environments are active; propagate through blocks/if/match and select composite callees after their effects execute. Preserve early-return pointer evidence, generic identities and loop refresh; keep differing targets unknown. Integration788 plus final2 execution groups pass, including a callee-side write executed once. Remaining: multi-target call sets, aggregate callable fields and interprocedural callable-result premises. No commits.


### 2026-10-09 — Callable-result lattice across memory-summary calls

Export bottom/known-instance/unknown callable result evidence; propagate explicit and implicit returns through the existing recursive worklist, local environments and branch results. Revisit argument effects when pending callees resolve and keep truly non-returning callee paths separate. Integration793 and final4 summary groups pass, including generics, recursion, budgets and executable nested factory writes. New S-expression field verified. Remaining: distinct-target candidate sets, captured closures and callable input premises. No commits.


### 2026-10-09 — Finite named candidates in memory summaries

Retain canonical bounded candidate sets through local/control-flow/interprocedural joins. Derive all target contracts and union effects/results while preparing arguments once; reject missing contracts and unsupported multi-target defaults. Integration797 and final3 candidate groups pass; candidate dump verified. Memory inference now accepts distinct named targets, but acquired ownership/context publication still needs matching candidate support before end-to-end execution can accept them. Captured closure contracts also remain open. No commits.


### 2026-10-09 — Executable named-callee alternatives

Carry finite named alternatives through ownership joins, calls and successful returns. Validate all candidate memory contracts while preparing one argument frame; publish actual function cleanup scopes, preserve pointer origins and reject incomplete exported premises. Add candidate call/return S-expression evidence. Integration807 and final9 execution +2 report groups pass, covering generic/loop/failure/factory paths. Remaining: higher-order candidate inputs, alternative captured environments, owned-argument contracts and shared-default contracts. No commits.


### 2026-10-09 — Complete candidate premises for higher-order parameters

Export and import all named alternatives in `CallableParameter`, validate each target, preserve exact premises through contextual cleanup publication, and budget type/instance validation. Executable coverage now includes direct callbacks, forwarding, returned callbacks, default selection, concrete generic targets, converged loops, single argument evaluation and frame cleanup on rejection. Integration passed 812 tests; final candidate-focused tests passed 25 after two additional execution groups. S-expression report formatting checked. Captured candidate environments and general higher-order memory contracts remain incomplete; no commits.


### 2026-10-09 — Recursive captured callable evidence across boundaries

Separate recursive callable provenance from executable capture initializers. Carry complete captured named-candidate sets through parameters and successful returns; validate nested capture positions/types/targets under budget and retain current ownership state at invocation. Existing expression-only return projections remain unknown when insufficient. Execution covers forwarding, factories, nested environments, failure cleanup and consumed-capture rejection. Integration passed 819 tests; final captured-callable selection passed 21 tests and recursive S-expression formatting checks. Alternative closure environments and general higher-order memory contracts remain incomplete; no commits.


### 2026-10-09 — Captured callable replacement and normal-exit writeback

Record callable capture evidence alongside each successful ownership exit; merge and write it back to the live receiver. Preserve same-identity closures across named-capture loop widening without conflating generations. Enable runtime whole-callable capture assignment with normal mutability and assignment-only capture discovery. Eight execution scenarios cover replacement/return/loops, old and new environment cleanup, failure and self-move. Integration passed 1,073 tests across elaboration, execution, ownership and resources; final captured-callable selection passed 18. S-expression writeback dump checked. General captured memory mutation and distinct alternative closure environments remain incomplete; no commits.


### 2026-10-09 — Persistent runtime capture cells and ordinary writes

Unify runtime capture persistence across reference and cleanup-scheduled interpretation while preserving serializable comptime captures. Admit runtime scalar/field/bit capture assignments; validate owned replacements with bounded pointer-free layout checks and retain pointer-write obligations. Cover independent environments, nested value snapshots, factory/higher-order state, replacement cleanup, rejection and moved-field renewal. Integration passed 1,079 tests; interpreter crate passed 28, and the checked mutable-capture counter example returned true. General captured-pointer contracts and comptime mutation remain incomplete; no commits.


### 2026-10-09 — Mutable comptime captures with graph snapshots

Enable persistent comptime capture updates and freeze completed environments into immutable graph-construction expressions. Preserve shared closure identities, independent instances and current capture contents; reconstruct in a private comptime invocation namespace. Keep free storage/pointer rejection and bound graph copying; active cycles reject. Regression coverage includes nested sharing, fresh constant evaluations, defaults, record updates, callback replacement, type formation and source/synthesized binding collisions. Integration passed 1,080 tests; copy-budget tests passed 2 and interpreter tests passed 28. CLI shared-counter example returned `(1, 2)` and graph S-expression formatting passed. No commits.


### 2026-10-09 — Sound entry facts for repeated mutable closure calls

Reproduced and fixed check elision based on stale creation-time capture values. Use a bounded mutation/escape footprint to discard unstable invocation facts while preserving independent immutable captures and existing proof replay. Require per-invocation arithmetic range premises for mutable captures. Regression coverage spans scalar/field/bit/pointer/helper writes, early returns, correlated old-value copies and optimization agreement. Integration passed 1,084 tests; final mutable-capture selection passed 6, dump formatting passed, and runtime/comptime CLI examples still execute. Alias-sensitive entry invariants remain conservative; no commits.


### 2026-10-09 — Field-sensitive capture invariants and bit-write flow

Preserve untouched capture-field facts across repeated calls using bounded exact write paths; invalidate whole roots/ancestors and old dependent relations as required. Add BitAssign flow handling that evaluates RHS effects before invalidating the written storage value. Positive cases cover nested fields and bit slices; rejection cases cover parent replacement, stale sibling relations and RHS mutation. All 1,088 distinct drop/elaboration/execution/resources tests passed across integration and targeted correction runs, with the initial fixture parse failure recorded in README. Aliased/opaque capture preservation remains conservative. No commits.


### 2026-10-09 — Stable loop entry facts

Preserve untouched scalar and record-path entry facts through bounded loop-write analysis. Reset written paths at both boundaries and analyze body refinements separately; unknown alias effects retain full havoc. New execution and optimization regressions cover bit writes, nesting, backedges and breaks. Integration passed 683 execution/elaboration tests; numeric recurrence inference and contract-based call preservation remain future work. No commits.


### 2026-10-09 — Read-only call preservation in program flow

Connect completed memory summaries to range flow for direct named/concrete generic calls, including loops. Cache signature-checked write-free decisions within the analysis budget; preserve explicit argument effects and retain havoc for defaults, opaque calls and writers. Verify scalar/field/tuple/transitive proof retention, pointer readers and baseline/optimized behavior. All 686 distinct execution/elaboration tests passed across integration plus a corrected assertion's targeted rerun. Alias-specific writer preservation and default-expression summary composition remain incomplete. No commits.


### 2026-10-09 — Default-frame memory summary composition

Compose read-only callee evidence with selected defaults in normalized prepared-parameter order, using bounded argument memory summaries. Preserve explicit argument effects and keep conservative fallback on writes or unavailable evidence. Source cases plus a direct NIR alias-write regression passed; 688 execution/elaboration integration tests passed. P3 source stability restrictions remain unchanged. No commits.


### 2026-10-09 — Direct pointer write precision

Resolve bounded syntactic local/record addresses for range updates and loop footprints, preserving RHS order and unrelated fields. Retain full invalidation for unknown pointer targets. Execution/optimization regressions include parent replacement during RHS evaluation and stale loop facts. Integration passed 690 execution/elaboration tests. General pointer provenance remains incomplete; no commits.


### 2026-10-09 — Direct pointer read facts

Reuse scalar and record path facts for known direct-address reads, sharing field extraction with ordinary projections. Preserve copied snapshots and reject stale range proofs after writes/branches. Integration passed 693 execution/elaboration tests. General pointer provenance remains incomplete; no commits.


### 2026-10-09 — Caller-relative writer footprints

Map closed function write summaries through direct-address explicit arguments and invalidate only affected caller paths. Reuse complete summary caching and loop integration; unknown destinations retain full havoc. Integration passed 880 execution/elaboration/resources tests. Writer defaults and general pointer provenance remain incomplete. No commits.


### 2026-10-09 — Writer default origin composition

Map default pointer results and writes through the prepared frame, then compose with body writes before caller invalidation. Support forwarding/chains/branches/explicit overrides; fall back on prepared-frame rebinding or address exposure. Integration passed 882 tests, followed by one additional NIR rebinding regression. P3 source admission remains unchanged; no commits.


### 2026-10-09 — Frame-scoped local pointer origins

Track pointer origins as abstract value metadata through copies/aggregates/joins and consume evaluated origins for reads/writes. Separate creator/capture/default frame identities, erase metadata under havoc and preserve RHS destination snapshots. Integration passed 1,107 tests; S-expression formatter round-trip passed. Alias-aware call footprint rebasing and stronger loop alias invariants remain incomplete. No commits.


### 2026-10-09 — Evaluated alias origins at calls

Save explicit pointer origins in source evaluation order and use those snapshots for callee/default write rebasing. Later argument rebinding cannot redirect earlier writes. Integration passed 887 execution/elaboration/resources tests. Pointer result origins and stronger loop alias invariants remain incomplete. No commits.


### 2026-10-09 — Returned pointer origin rebasing

Rebase closed summary return targets through evaluated caller origins and defaults; retain only a common address and apply writes before returning alias evidence. Support generic, forwarding and recursive summaries, including subsequent reader/writer use. Integration passed 889 execution/elaboration/resources tests. Distinct-target sets and loop alias invariants remain incomplete. No commits.


### 2026-10-09 — Finite range-flow pointer targets

Retain bounded canonical target sets through joins, calls/returns and aggregates. Read range unions and weakly update possible write destinations, keeping strong updates only for singleton targets. Integration passed 1,115 tests; 16-target match/budget recovery and dump formatter checks passed. Loop alias invariant inference remains incomplete. No commits.


### 2026-10-09 — Stable loop pointer origins

Resolve unchanged local/record pointer sources during a bounded monotone loop footprint scan. Rescan to catch later writes invalidating earlier alias assumptions; preserve unrelated facts only after convergence. Integration passed 895 tests. Arithmetic loop invariants and loop-carried pointer rebinding remain conservative. No commits.


### 2026-10-09 — Computed loop pointer destinations

Compose stable origins through call returns, if/match alternatives and block results while independently scanning all effects and rescanning changed origins. Integration passed 897 execution/elaboration/resources tests. Pattern-local origins, opaque calls and arithmetic recurrence inference remain incomplete. No commits.

### 2026-10-09 — Default tuple variadics through execution and cleanup

Implemented source-ordered heterogeneous/empty positional collection, whole-value named supply, generic contextual inference, defaults and receiver/trait calls. Added explicit NIR/IIR `CallPreparation` instead of lowering input staging to ordinary source lets, preserving temporary-owner lifetimes and partial/failing preparation cleanup. Updated its consumers and S-expression Display; malformed preparation plans reject at the phase boundary. The 1,135-test execution/elaboration/resources/drop integration passed; an additional higher-order execution group, 25 library unit tests, 71 IR/interpreter tests and 56 call/verification/ownership tests passed. Formatter round-trip verified. Next parameter gaps: tuple spreads, attribute collections and custom collection protocols, plus first-class routing metadata. No commits.

### 2026-10-09 — Contextual tuple construction through type equations

Use PTC-proven structural tuple views for collection validation and element contexts, retaining final equality against the original expected type. Fixed generic collection under `T == (Byte, bool)` without unfolding normal-form heads. Execution/elaboration integration passed 727 tests, including opaque-head refusal and explicitly justified structural access. No commits.

### 2026-10-09 — Positional tuple spreads

Implemented once-evaluated packet unpacking into ordinary/generic/variadic calls, including named-input interleaving, receivers, effectful callees, empty packets, comptime invocation and divergence. Reuse `CallPreparation` with phase-validated plain tuple bindings; preserve residual cleanup identities, affine delivery, pointer origins and parameter proof facts. Direct known signatures/result contexts can guide a sole compound packet. Thirteen new groups; 1,151 execution/elaboration/resources/drop tests and 71 IR/interpreter tests passed, plus S-expression formatter round-trip. Remaining: custom/property protocols, mutually dependent unknown packet shapes and additional method/indirect packet context propagation. No commits.

### 2026-10-09 — Resolved callable contexts for spreads

Reuse resolved callees/receivers to guide compound packet typing for indirect functions, closures, abstract callable equations and trait/inherent methods. Preserve method optional/variadic metadata and source-order ownership. Fixed finalized inherent implementations contaminating the resource trait index; regressions verify Drop selection and denial of unjustified Copy. Seven new groups; 1,158 execution/elaboration/resources/drop tests passed. Unknown packet-shape dependency solving and custom/property protocols remain outstanding. No commits.

### 2026-10-10 — Layout contexts across multiple tuple packets

Infer packet widths from bounded syntax and known type/signature structure, distribute positional element contexts from both ends and solve one remaining width without evaluating producers or unfolding opaque heads. Added seven execution/diagnostic/cleanup groups; 1,165 execution/elaboration/resources/drop tests passed. Preserve source order and clean only initialized inputs on partial preparation. Mutually unresolved widths, custom/property protocols and constructor spreads remain outstanding. No commits.

### 2026-10-10 — Tuple enum constructor spreads

Share packet preparation between ordinary calls and tuple enum construction, using instantiated payload fields as contexts. Admit normalized variants at the CallPreparation phase boundary without changing temporary lifetime or binding restrictions. Five execution/diagnostic/cleanup groups cover explicit/contextual/generic construction, empty packet effects, partial preparation and projected-owner cleanup. Integration passed 1,170 tests. Named-field/property spreads and custom protocols remain outstanding. No commits.

### 2026-10-10 — Positional extended applications

Separate recognized callable heads from nominal construction and reuse ordinary routing for brace-call positional inputs. Support generic/variadic calls, defaults, closures, function parameters and receiver/static methods. Refuse colon properties with an attribute-protocol diagnostic instead of misrouting them as named supply. Four new test groups; 1,174 execution/elaboration/resources/drop tests passed. Next prerequisites for property collection: anonymous-record types/values and structural field access; computed callable/type heads also need general dispatch. No commits.

### 2026-10-10 — Named-product type core

Added record type representation, duplicate/sort validation, capture-avoiding substitution, interning/runtime environments and escaped S-expression Display. Audited recursive dependency and budget paths through NIR, IIR, PTC inputs, materialization, comptime term copying and implementation metadata. Four-crate tests passed 275 cases, followed by an additional nested type-argument environment regression; editor dump round-trip passed. Field-permutation identity awaits the author's decision; no record congruence/decomposition rules or runtime/source collection support are claimed. Next: record values/projections, resource shapes and property collection. No commits.

### 2026-10-10 — Anonymous-record resource layouts

Added record field ownership, temporary field types, pointer-free footprint recursion, bounded memory traversal and Drop acquisition/planning. Record-vs-known-nonrecord type-family disjointness enables sound rejection of unrelated destructor candidates without deciding record permutations. Six new test groups; 55 elaboration unit/PTC tests and 1,175 source integration tests passed. Runtime record values/projection/collection and field-permutation identity remain pending. No commits.

### 2026-10-10 — Anonymous-record values and storage

Added named interpreter record values, recursive typed-call validation with depth bounds, escaped Display, and label-preserving cleanup storage paths. Tests cover malformed input/recovery, empty products, scalar range checks, nested storage identity, selective retirement, stale labels and expired owners. Four-crate integration passed 289 tests; source execution/Drop passed 727 tests; value dump editor round-trip passed. NIR construction/projection and constant reification still need implementation before source property collection is available. No commits.

### 2026-10-10 — Anonymous-record construction IR

Added typed construction IR, field traversal/substitution/materialization, interpreter execution and aggregate cleanup scheduling. Integrated constant reification and verification consumers. Four direct IR tests plus a scheduled Drop fixture cover source-order evaluation, early control transfer, malformed shape/type rejection, specialization and partial construction cleanup. Four-crate integration passed 292 tests, then the added specialization regression passed; source integration passed 1,176 tests; S-expression editor round-trip passed. Next: field projection and source property routing; anonymous-record constant source coverage awaits producers. No commits.

### 2026-10-10 — Source property records and field reads

Connected property-only extended calls on direct single-collector functions to anonymous construction and whole-package delivery. Added field-read IR, phase/name validation, runtime projection and resource/residual cleanup paths; propagated established record contexts to numeric fields. Eight source groups cover nested/constant records, named supply versus colon properties, ordering, pointer origins, move rejection and partial/failing cleanup. Integration passed 1,184 source tests plus 294 IR/elaboration/interpreter tests; editor dump round-trip passed. Remaining: methods/function-value collection, mixed/custom protocols, general contextual inference, record writes/addresses, empty-property disambiguation and the pending permutation decision. No commits.

### 2026-10-10 — Method property collection

Extended property calls now reuse resolved inherent, trait-entry and static-method declaration metadata. Receiver values are bound before the anonymous property record, evaluated once, then delivered through ordinary call routing. Trait collection uses interface-entry parameter metadata; optional named parameters remain separate from colon property inputs. Contextual field types guide literals and remain available for unreachable call routing when a property producer diverges.

Source tests verify receiver/property/callee order, static and trait dispatch, numeric contexts, optional-name separation, duplicate/missing/wrong-type properties and missing collectors. Scheduled Drop tests cover normal delivery, early return during collection and runtime contract rejection, including destruction of the already evaluated receiver and acquired fields.

In `/home/fin/luna`, `python3 -B development/cargo.py check -p elaboration` exited 0 (`/tmp/luna-method-properties-check.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0: 502 execution, 262 elaboration, 189 resources and 235 Drop tests, 1,188 total (`/tmp/luna-method-properties-integration.log`). Existing ast_lowering/elaboration warnings remain. No new IR variants; existing CallPreparation/AnonymousRecord dumps are reused. Function-value protocol metadata, mixed inputs, custom collection protocols and general generic-method inference remain incomplete. Record permutation semantics remain undecided. Changes remain uncommitted on dev.

### 2026-10-10 — Anonymous record places and pointer projections

Anonymous records now support direct field assignment, nested/mixed nominal paths, field `.ref`, pointer-field projection and explicit dereferenced places. `FieldOwner` distinguishes nominal and anonymous layout families in program places and body-derived memory footprints; these are program-state identities, not PTC propositions. `PointerAnonymousField` retains the field label/index and pointer producer, with phase validation, specialization/traversal and escaped S-expression Display. The interpreter validates the selected record label and keeps weak storage identity across compatible whole-value replacement.

Ownership/provenance, memory summaries, Drop footprint access and range-flow paths recognize the new projections. Anonymous fields acquire scalar observation identities; direct writes, bit writeback, pointer writes, capture updates and loop effects invalidate affected facts. Tests verify source evaluation order, cross-function field footprints, sibling preservation, moved-field rejection/reinitialization, expired targets, replacement cleanup and malformed IR labels/indices/types. A closure test checks its captured record, preserving existing capture-by-value behavior rather than assuming updates mutate the outer record.

From `/home/fin/luna`, `python3 -B development/cargo.py check -p luna` exited 0 (`/tmp/luna-record-places-check.log`). `python3 -B development/cargo.py test -p nir -p iir -p elaboration -p comptime_interpretor` exited 0 with 295 tests (`/tmp/luna-record-places-ir.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0 with 507 execution, 263 elaboration, 190 resource and 236 Drop tests: 1,196 total (`/tmp/luna-record-places-integration.log`). Existing ast_lowering/elaboration warnings remain.

The new [record-places example](examples/record-places.fl) emits NIR and interprets `entry` as `(bool true)` using the wrapper (`/tmp/luna-record-places-example.log`, `/tmp/luna-record-places-example-run.log`). Both its complete NIR/Place dump and the deliberately escaped pointer IR test dump pass Emacs read/format/read (`/tmp/luna-record-places-formatted.lisp`, `/tmp/luna-record-pointer-formatted.lisp`). Existing restrictions on owned reads/writes through pointers and calls without sufficient memory contracts remain; this change does not add generalized pointee ownership transfer. Anonymous field-permutation identity and mixed/custom property collection remain pending. Changes stay uncommitted on dev.

### 2026-10-10 — Interleaved anchored inputs and property collection

Extended calls now route anchored positional inputs alongside a single property collector. Ordinary call inference/contextual checking handles the anchored inputs and dependent defaults; the complete property packet enters through the existing named whole-value route. The generated CallPreparation stages the receiver first and every explicit producer in source order, then assembles the record and runs the original call/default plan. Attributes can appear before, between or after anchored inputs. Additional positional values competing for the same collector reject as duplicate supply; multiple collection entrances and custom routing remain unsupported.

Tests cover all three source placements, numeric literal contexts, a default reading an earlier anchored input, inherent/trait receiver calls, comptime constant reification, missing/duplicate/wrong inputs, and scheduled cleanup on normal delivery, early exit at an anchored or property input, and runtime contract rejection. Existing property-only and record resource cases remain green. No new IR kind or field-order equality rule was introduced.

From `/home/fin/luna`, `python3 -B development/cargo.py check -p elaboration` exited 0 (`/tmp/luna-mixed-properties-check.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0 with 511 execution, 264 elaboration, 190 resources and 237 Drop tests: 1,202 total (`/tmp/luna-mixed-properties-integration.log`). `python3 -B development/cargo.py run -p luna -- development/examples/mixed-property-inputs.fl --interpret entry` exited 0 and returned `(bool true)` (`/tmp/luna-mixed-properties-example.log`). The [example](examples/mixed-property-inputs.fl) is self-contained. Existing warnings remain; changes stay uncommitted on dev.

### 2026-10-10 — Context propagation into collected properties

Property collection now retains an explicitly instantiated callee and uses its substituted parameter type for field contexts; final routing reuses that instance instead of evaluating type arguments again. Uninstantiated generic calls derive collector context through the ordinary call scheme, including nested result positions and contextual callable results. Structural views are requested only through the existing proof interface. Generic implementation method substitutions supply the concrete collector shape as before, now covered by a source regression.

PTC structural-view search now considers closed anonymous record shapes when equality is established. This adds no record constructor injectivity/congruence or permutation equation, and performs no normal-form body unfolding. A regression checks transitive scoped evidence, proof replay in the admitting context, refusal to replay in an unrelated context, exhausted budgets and refusal to choose a shape from a disjunction. Existing permutation-unknown tests still pass.

Source cases cover explicit aliases, computed Type arguments, one/two nested tuple result contexts, interleaved anchors, generic implementation methods, equality-constrained caller types and u8 overflow. The no-sysroot fixtures explicitly spell `.Type'builtin`; the normal-form rejection checks the actual unresolved structural-view diagnostic. Callee-bound/sibling-driven property inference and general method-owned generic parameters remain incomplete.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p elaboration --lib --test ptc` exited 0 with 29 unit and 27 PTC tests (`/tmp/luna-property-contexts-ptc.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0 with 514 execution, 265 elaboration, 190 resources and 237 Drop tests: 1,206 total (`/tmp/luna-property-contexts-integration.log`). Existing warnings remain; no new IR variant; changes stay uncommitted on dev.

### 2026-10-10 — Property packets participate in ordinary argument inference

Property collection now builds an elaboration-generated HIR `CollectedProperties` packet and sends it through the existing ordinary call checker, replacing the separate eager field-checking/inference path. Numeric and lambda fields can defer until another argument supplies the complete record type; argument attempts reuse the existing rollback of captures, return observations, scopes and pending uses. Explicit callee instances remain reused. After inference, source producers are staged in their original interleaving with anchors and receiver, and the completed packet is delivered once.

The internal packet retains its hypothetical completed record shape for inference even when a producer exits, as the previous typed proxy did. Before NIR is published, source staging propagates producer `Never` to the entire CallPreparation; no packet is constructed on that runtime exit path. This preserves the existing early-return/contract-rejection cleanup tests. `CollectedProperties` has no new source syntax and has a structural S-expression Display with nested field/name/span relationships.

New source tests cover sibling records before/after the collector, numeric default deferral, contextual lambda fields, producer evaluation exactly once in source order, overflow and incompatible field types. Existing explicit-instance, nested-result, method, comptime, affine and cleanup cases remain green. From `/home/fin/luna`, `python3 -B development/cargo.py check -p luna` exited 0 (`/tmp/luna-property-inference-check.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0 with 517 execution, 266 elaboration, 190 resource and 237 Drop tests: 1,210 total (`/tmp/luna-property-inference-integration.log`). `python3 -B development/cargo.py test -p hir --test display -- --nocapture` exited 0 with ten tests, including exhaustive expression coverage and a nested/escaped property packet (`/tmp/luna-property-hir-display.log`). The actual HIR dump passes Emacs read/format/read (`/tmp/luna-property-hir-formatted.lisp`). Multiple collection entrances, custom protocols and inference from partially known record fields remain incomplete. Changes stay uncommitted on dev.

### 2026-10-10 — Distinct tuple/property collection entrances

Whole named supplies reserve their parameter entries before positional routing, independent of where the names occur in source. Positional ambiguity counts exclude those reserved entries; named packages also contribute no spread slots or spread element type requirements. Duplicate whole supply and attempts to append positional inputs to a fully supplied collection still reject. One existing duplicate-spread fixture now rejects earlier as exceeding the remaining positional slots, and its diagnostic assertion records that change.

Extended calls select a default record collector when its signature/result context uniquely distinguishes it from tuple collectors. Unknown shapes remain eligible rather than being assumed incompatible; multiple eligible record entrances or multiple unsupplied positional collectors remain explicit ambiguity errors. Generic result contexts and resolved inherent/trait method substitutions participate in this selection; no parameter-name heuristics or normal-form unfolding are used.

When tuple collection already generated CallPreparation, property lowering reuses its receiver/positional steps and final parameter bindings, replaces the single record-packet step with source-interleaved field producers, and assembles the record afterward. Producers execute once; failed input acquisition retires only earlier responsibilities in reverse acquisition order. Tests cover both parameter declaration orders, all source placements, explicit whole-package timing, contextual spread literals, methods/defaults, contextual generic collector selection, normal/early/rejected cleanup and affine duplication rejection.

From `/home/fin/luna`, `python3 -B development/cargo.py check -p elaboration` exited 0 (`/tmp/luna-multiple-collectors-check.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0 with 522 execution, 267 elaboration, 191 resource and 238 Drop tests: 1,218 total (`/tmp/luna-multiple-collectors-integration.log`). The [multiple collectors example](examples/multiple-collectors.fl) interprets `entry` as `(bool true)` (`python3 -B development/cargo.py run -p luna -- development/examples/multiple-collectors.fl --interpret entry`, exit 0, `/tmp/luna-multiple-collectors-example.log`). Its emitted NIR passes Emacs read/format/read (`/tmp/luna-multiple-collectors-formatted.lisp`). Existing warnings remain. Custom collection protocols, ambiguous same-kind entry routing and extended spread syntax remain incomplete. No new IR variant; changes stay uncommitted on dev.


### 2026-10-10 — Inherent method-local generic inference

Inherent method invocation now retains implementation arguments fixed by receiver/static resolution and infers remaining method binders from ordinary inputs and result contexts. The ordinary generic call checker accepts an already-elaborated receiver prefix and fixed type arguments; it instantiates defaults in both binder groups, checks requirements after inference, and publishes only complete FunctionInstance arguments. Partial callable result contexts propagate through receiver, static, property and spread calls. Trait generic methods and generic method values remain explicit unsupported cases rather than leaking incomplete instances.

Property entrance selection and spread packet contexts abstract only the remaining method binders. Tests cover generic implementations, receiver/static invocation, numeric result contexts, sibling lambda inference, tuple/property collection, multiple collectors distinguished by the result, contextual spread packets, method bounds, incompatible inputs and evaluation exactly once. Scheduled cleanup covers owned receivers and partially acquired property fields on normal return, early exit and contract rejection. Generic defaults that lack the existing P3 stability evidence remain rejected.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0: 530 execution, 270 elaboration, 191 resources and 238 Drop tests (1,229 total; `/tmp/luna-generic-methods-integration.log`). Subsequently `python3 -B development/cargo.py test -p luna --test drop inherent_generic_methods_cleanup` exited 0 for the additional cleanup test (`/tmp/luna-generic-methods-cleanup.log`), bringing covered tests to 1,230. `python3 -B development/cargo.py run -p luna -- development/examples/generic-methods.fl --interpret entry` exited 0 with `(bool true)` (`/tmp/luna-generic-methods-example.log`). Emitting NIR with `--emit nir --output /tmp/luna-generic-methods.lisp` exited 0, and Emacs read/format/read preserved its structure (`/tmp/luna-generic-methods-formatted.lisp`). No new IR variants; existing warnings remain. Changes stay uncommitted on dev.


### 2026-10-10 — Generic inherent method values and explicit instantiation

Generic inherent methods now become ordinary function values using full function type contexts or partial higher-order contexts, including nested generic calls. Implementation arguments remain fixed even when absent from the method signature; only method-local binders are inferred. Completed instances use the existing type-application checks, phase checks and function-value parameter restrictions. Missing contexts defer through the existing argument transaction mechanism; missing bounds, incompatible fixed receiver types and unused unresolved binders reject.

Explicit method arguments are supported on static references/calls and receiver calls, including property collection and tuple spreads. `Holder<A>.method<B>` supplies B after the fixed implementation A. Receiver resolution retains its single elaborated receiver while instantiating the method; spread/property preparation retains the completed instance so type-producing arguments are not replayed. Arity and bounds are checked before publication. Generic trait methods and bound receiver method values remain outside this implementation.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0 with 538 execution, 274 elaboration, 191 resource and 240 Drop tests: 1,243 total (`/tmp/luna-method-values-integration.log`). Tests include explicit receiver evaluation once and cleanup of partial inputs on early return/contract rejection. A new negative fixture initially used unavailable bare `char` without sysroot; it now uses two valid bool type arguments to exercise arity rejection. `python3 -B development/cargo.py run -p luna -- development/examples/generic-method-values.fl --interpret entry` exited 0 with `(bool true)` (`/tmp/luna-method-values-example.log`). NIR emitted by `--emit nir --output /tmp/luna-method-values.lisp` passed Emacs read/format/read (`/tmp/luna-method-values-formatted.lisp`). Existing warnings remain; no new IR variant; changes remain uncommitted on dev.


### 2026-10-10 — Generic trait method declaration contracts

MethodContract now retains method-local type binders separately from enclosing Self/trait binders, plus capability requirements. Its S-expression Display exposes both. Program summaries preserve unsubstituted binders and substitute capabilities/type predicates alongside the signature. Implementation admission matches binder arity, substitutes trait/Self arguments and alpha-renames method binders to the implementation's identities before comparing signatures and type requirements. Capability implication uses trait method assumptions plus implementation assumptions against the completed implementation world, avoiding declaration-order dependence and rejecting stronger implementation requirements. Parent bounds and equalities can establish weaker requirements.

Generic trait declarations, implementations and default bodies can now elaborate to NIR. This is contract infrastructure, not completed generic trait invocation: selection still explicitly rejects generic trait calls, and concrete IIR interface materialization explicitly rejects remaining method templates. NIR dispatch template construction remains permitted; a combined example uncovered that applying the IIR guard there would incorrectly reject independent generic implementations alongside default generic methods. A regression covers that combination.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p nir -p elaboration -p luna --test calls --test elaboration --test execution --test resources --test drop` exited 0: 13 elaboration call, 6 NIR call, 279 source elaboration, 538 execution, 191 resource and 240 Drop tests (1,267 total; `/tmp/luna-trait-generics-integration.log`). New tests cover alpha renaming, generic enclosing traits/implementations, arity/signature mismatches, strengthened predicates/capabilities, parent evidence, later concrete implementations, default bodies, pending invocation rejection and summary substitution. `python3 -B development/cargo.py run -p luna -- development/examples/generic-trait-contracts.fl --emit nir --output /tmp/luna-trait-generics.lisp` exited 0 (`/tmp/luna-trait-generics-example.log`); Emacs read/format/read passed (`/tmp/luna-trait-generics-formatted.lisp`). Existing warnings remain. Changes stay uncommitted on dev; next work is method arguments through TraitMethod, specialization and dispatch.


### 2026-10-10 — Trait method argument transport and validation

NIR TraitMethod now carries a separate `method_arguments` vector, shown as `(method-arguments ...)` in Display and recursively substituted alongside subject/interface arguments. Phase validation checks both arities independently and instantiates the combined Self/interface/method telescope before comparing the callable signature. Verification summaries, definition defaults and interpreter-side interface program-contract substitution retain the method arguments. Dispatch specialization is still incomplete: materialization, IIR validation and interpreter entry paths explicitly reject nonempty method arguments instead of executing an unrelated unspecialized entry. Source generic trait invocation remains rejected; this milestone does not claim executable generic dispatch.

Regression testing exposed stack overflow before the existing 64-level signature dependency diagnostic. Field/associated-member checking was extracted from the large recursive expression dispatcher into `program/projections.rs`, with a non-inlined boundary so its temporaries do not inflate every signature dependency frame. The existing accepted 24-link chain and rejected 80-link chain now pass without reducing the depth budget.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p nir -p iir -p comptime_interpretor -p elaboration` exited 0 with 297 tests (`/tmp/luna-method-arguments-ir.log`). After the projection extraction, `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0: 539 execution, 279 elaboration, 191 resource and 240 Drop tests, 1,249 total (`/tmp/luna-method-arguments-integration.log`). New phase cases distinguish enclosing char from method bool, reject missing/excess method arguments and wrong signatures, and check nested argument substitution. The dedicated signature-depth test exited 0 (`/tmp/luna-method-arguments-stack.log`). Running `python3 -B development/cargo.py test -p luna --test execution generic_trait_reference_phase -- --nocapture` exited 0 and emitted a nonempty method-argument dump; Emacs read/format/read passed (`/tmp/luna-method-arguments-formatted.lisp`). Existing warnings remain. Changes stay uncommitted on dev. Next: specialization identity in dispatch/retained interface contracts, then source inference and execution.


### 2026-10-10 — Executable generic trait method dispatch

Generic trait invocation now infers method arguments after fixing Self and interface arguments, reusing ordinary generic input/default checking and replacing the provisional callable with its definition-fixed TraitMethod origin before publication. Bodyless interface declarations participate in that inference. Program contract inheritance alpha-renames method-local binders into implementation binders. Declared type/capability requirements remain checked even when the chosen body has weaker requirements.

Dispatch MethodInstance and retained MethodContract entries distinguish `(member, method arguments)`. Materialization specializes generic members on demand, retaining completed signatures, program specifications and method argument metadata; only demanded concrete bodies enter IIR. Repeated/recursive requests reuse the same instance. IIR validation and both interpreter paths select the exact specialization. The NIR interpreter validates generic body parameter prefixes, selected argument vectors and method-local arity, and checks declared type/capability contracts. Display includes method/type argument groups. Source generic trait invocations are now executable; generic trait method values, explicit method type applications and all contextual spread/multiple-collector cases still need further coverage/implementation.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p nir -p iir -p comptime_interpretor -p elaboration` exited 0 with 297 tests (`/tmp/luna-generic-dispatch-ir.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0: 547 execution, 281 elaboration, 191 resource and 241 Drop tests (1,260 total; `/tmp/luna-generic-dispatch-integration.log`). New cases cover simultaneous bool/char instances, generic impl/interface/receiver substitution, default bodies and recursion, contextual literals/callbacks, interface defaults, weaker implementation bounds, program preconditions, property collection, early/rejected-input cleanup, duplicate specializations and missing bodies/contracts. Statically false preconditions are rejected during source verification, not mislabeled runtime tests.

`python3 -B development/cargo.py run -p luna -- development/examples/generic-trait-dispatch.fl --interpret entry` exited 0 with `(bool true)` (`/tmp/luna-generic-dispatch-example.log`). `--instantiate entry --output /tmp/luna-generic-dispatch.lisp` exited 0; the concrete IIR dump passed Emacs read/format/read (`/tmp/luna-generic-dispatch-formatted.lisp`). Existing warnings remain. Changes stay uncommitted on dev.


### 2026-10-10 — Generic trait method values, explicit arguments and call contexts

Static generic trait method references now infer from complete function types or partial higher-order contexts, including nested generic calls, and accept explicit method type arguments. Instantiation fixes Self/interface arguments and retains the TraitMethod origin instead of publishing a direct reference to an interface declaration. Shared function-value checking now includes bodyless declarations and preserves phase, bound and parameter-protocol checks. Receiver explicit calls reuse their already-elaborated receiver. Property entrance selection and spread packet contextualization now account for remaining trait method binders, allowing result contexts to distinguish multiple collectors.

Tests cover context/explicit values in both interpreter paths, nested higher-order inference, parameterized traits and receivers, single receiver evaluation with literal/nonliteral spreads, explicit property contexts, multiple collector result contexts, missing bounds/context, wrong receiver types and type-argument arity. Additional tests preserve affine argument delivery and explicit generic receiver/partial-input cleanup. Method values with program preconditions or optional/variadic protocols still reject through existing unsupported-metadata boundaries; bound receiver method values remain separate unfinished work.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0: 552 execution, 283 elaboration, 192 resource and 242 Drop tests (1,269 total; `/tmp/luna-trait-method-values-integration.log`). `python3 -B development/cargo.py run -p luna -- development/examples/generic-trait-method-values.fl --interpret entry` exited 0 with `(bool true)` (`/tmp/luna-trait-method-values-example.log`). `--instantiate entry --output /tmp/luna-trait-method-values.lisp` exited 0; Emacs read/format/read preserved the concrete IIR dump (`/tmp/luna-trait-method-values-formatted.lisp`). No new IR variants; existing warnings remain. Changes stay uncommitted on dev.


### 2026-10-10 — Closed associated values and selected associated types

Trait and implementation NIR now retain associated-value contracts and selected values keyed by the originating interface member. AssociatedContract has a structural S-expression Display; trait/implementation dumps include associated entries. Admission distinguishes methods from associated values, checks duplicate names, mandatory initializers, missing/unknown values and declared value types, and substitutes Self/interface arguments into defaults. Ordinary defaults use the existing bounded required-comptime evaluation and frozen-value reconstruction. Type defaults instead retain stable definition-side type templates until selection; interpreting rigid T/Self as closed runtime types was rejected by the interpreter and is not used.

Explicit `Type.as(Interface<...>).member` queries resolve the complete interface instance through implementation selection evidence and substitute implementation arguments into its value. Selected Type values can form local annotations. This supports scalar/tuple/computed defaults, overrides, generic implementation Type values, interface parameters and identical subject/member names under distinct interface arguments, while preserving opaque normal-form heads. It does not yet implement symbolic associated projections in abstract trait signatures, dependent associated defaults, parameterized associated items, inherited associated lookup or runtime associated-value environments. Bound receiver method values were not introduced: current docs specify receiver calls but do not establish an implicit method-value capture rule.

From `/home/fin/luna`, `python3 -B development/cargo.py test -p nir -p iir -p intrinsic -p elaboration -p comptime_interpretor` exited 0 with 301 tests (`/tmp/luna-associated-ir.log`). `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop` exited 0 with 557 execution, 286 elaboration, 192 resource and 242 Drop tests (1,277 total; `/tmp/luna-associated-integration.log`). New cases cover defaults/overrides, Type/value distinction, stable template substitution, completed computations, same-subject interface argument separation, missing/wrong values, unavailable implementation evidence, integer range rejection and no normal-form unfolding. Negative assertions use actual parser/range/Unknown-equality diagnostics rather than assuming all failures are type mismatches.

`python3 -B development/cargo.py run -p luna -- development/examples/associated-values.fl --interpret entry` exited 0 with `(bool true)` (`/tmp/luna-associated-example.log`). `--emit nir --output /tmp/luna-associated-values.lisp` exited 0 and Emacs read/format/read preserved the new metadata dump (`/tmp/luna-associated-values-formatted.lisp`). Existing warnings remain. Changes stay uncommitted on dev.

### 2026-10-10 — Inherited associated lookup and callable values

Implemented complete-interface origin lookup for associated members, diamond deduplication, child-view evidence, source ambiguity diagnostics and explicit parent disambiguation. Associated function values now enter ordinary call preparation before static-method lookup, preserving defaults/overrides and spread/local invocation behavior. Direct runtime function declaration constants preserve identity without body invocation; phase validation permits identity transport at comptime while retaining invocation checks. Source verification: 561 execution, 289 elaboration, 192 resources, 242 Drop; lower-level suites: 297. See development/README.md for commands/logs and the corrected function-identity test assertion.

Remaining: required comptime computations that manipulate runtime function references need signature-only dependency preparation rather than treating every reference as an execution dependency. Abstract associated types, dependent/parameterized associated values and cross-stage module constant Splice remain unimplemented. Do not replace these with global rigid aliases or unfold normal-form heads.

### 2026-10-10 — Body-free runtime function identities in required invocation

Resolved the previous milestone's direct-function-reference limitation: comptime computations can select, transport and return runtime function identities, including completed generic instances, for associated defaults/overrides. Added explicit signature-only NIR metadata and interpreter values; no fabricated executable bodies. Function formation/phase checks remain mandatory, and IIR demands an executable definition. Removed the special direct-reference constant path. Verified 1,289 source tests and 301 lower-level tests; executable example and NIR formatter check are recorded in development/README.md.

Still incomplete: symbolic/dependent associated projections; required-invocation runtime trait-method values; defaults that require a not-yet-established interface world; cross-stage module constant Splice. Continue from these boundaries without treating identity transport as permission to execute runtime code at comptime.

### 2026-10-10 — Deferred concrete associated initializers

Split associated-member admission from initializer evaluation. After establishing the interface world, demand selected initializers with explicit dependency/cycle tracking; cache completed values and restore declaration order before publication. Concrete dependencies may use selected overrides, other implementations, Type values, capability-constrained function formation and comptime trait methods. Verified 1,295 source tests and the associated-dependencies executable/NIR example; logs and commands are in development/README.md.

The previous blanket limitation on interface-dependent defaults is resolved for concrete initializer queries. Remaining work is symbolic associated projections (especially abstract Self and generic defaults), parameterized associated items, and signature formation that depends on unfinished interfaces. Keep these distinct from initializer scheduling; do not unfold normal-form heads or send program predicates to PTC.

### 2026-10-10 — Runtime trait method identity transport

Implemented required-invocation transport of runtime trait methods using body-free signature metadata and a boxed `TraitMethodIdentity`. Constant materialization retains the originating TraitMethod, complete interface/subject and generic method arguments. Declaration requirements and program-precondition rejection are preserved; final body verification is unchanged. Added coverage for default/generic/inherited methods, generic implementation substitution, weaker implementation bounds, unchecked-interpreter invocation rejection and retained origin metadata.

Verified 1,299 source tests, 303 lower-level tests, the post-budget-change focused group, and the computed-trait-method-values CLI/dump example. Boxing fixes the small-host-stack regression introduced by the initial inline Value variant. See development/README.md for logs and the intentional call-site phase diagnostic update. The prior runtime-trait-value limitation is resolved; symbolic/dependent associated type/value elaboration remains the next substantial interface frontier.

### 2026-10-10 — Callable values do not recover declaration input protocols

Fixed associated callable invocation routing: retained declaration identity supplies dispatch, while the stored ordinary callable type supplies positional inputs. Direct declaration access continues to expose optional/named/collection entries. Preserved the existing positive positional-method-value example and added ordinary/extended/spread, tuple-packet and rejection regressions. Verified 1,304 source tests plus the associated-callable-inputs CLI/NIR example; commands and logs are in development/README.md. Generic callable metadata support and symbolic associated elaboration remain incomplete.

### 2026-10-10 — Implementation-local associated names

Added lexical access to an implementation's own associated declarations through selected-value queries: scalar/type values, sibling dependencies, generic implementation templates, nominal constructors, static interface queries and stored-callable invocation. Lookup is scoped to the enclosing implementation and keeps trait origin, cycle and copy-budget checks. Verified 1,311 source tests and the lexical-associated-values CLI/NIR example. Abstract trait names and signatures needing an unfinished interface world remain unsupported; do not replace those queries with declaration defaults or global rigid type aliases.


### 2026-10-10 — Lexical associated types in implementation signatures

Implementation signature formation now follows its own associated Type definitions before interface admission, retaining stable generic binders and Self. Cycle/depth and copy-budget guards apply; later admission continues to check member existence and method compatibility. Verified 1,315 source tests and the associated-signatures executable/NIR-formatting example; evidence is in development/README.md. The unfinished-signature limitation is resolved for lexical Type definitions and independent comptime computations, but not abstract trait projections or queries needing an unfinished implementation world.


### 2026-10-10 — Bound repeated associated signature formation

Reproduced and fixed exponential repeated evaluation of shared lexical associated Type dependencies. Cache completed rigid templates by declaration owner, charge all copies, and discard this pre-admission cache when the interface world is ready. Verified 41 associated-member source tests, including the previously budget-exhausting dependency chain and declaration/generic-instance separation. Abstract associated projection support remains unfinished; this cache does not substitute declaration defaults for selected implementation values.


### 2026-10-10 — Abstract associated Type representation

Added NIR `Term::AssociatedType` with a boxed `AssociatedTypeOrigin` (interface and member declaration identities), complete interface arguments and subject. Binder/hole/rigid substitution retains the projection and visits all inputs; scope, runtime-dependency and copy-budget walkers include it. PTC admits forward congruence only for identical origins and argument arity, without constructor disjointness or inverse inference. Implementation indexing treats unresolved projections conservatively; ownership does not look through their subject to claim a local result. Unresolved projections are rejected by completed comptime Type freezing and IIR materialization/validation.

This is a prerequisite, not source-feature completion. Next integrate abstract trait lexical names and explicit generic interface views with declaration-side formation obligations, then resolve substituted projections with the same selected implementation evidence used for member dispatch. Preserve complete interface origin through inherited lookup. Trait defaults must never stand in for an unknown override. Concrete resolution needs cycle/budget guards and traversal through expression signatures, method contracts and instantiated nominal fields before IIR publication.

The initial inline origin fields enlarged Term and regressed the existing signature-dependency stack test. Boxing origin metadata restored that test without changing the stack or semantic limits. Both Term and origin have S-expression Display implementations. Verification results are recorded in development/README.md.


### 2026-10-10 — Selected associated Type query API

`World::resolve_associated_type` now resolves one origin-fixed projection using capability-qualified implementation evidence, validates member/formation contracts and substitutes the selected Type template. The returned `AssociatedTypeResolution` retains type evidence and qualification. Missing values never use trait defaults; nested projections and normal-form heads stay residual. Verified 50 related lower-level tests and a source-to-query integration using a compiled generic implementation. Continue with bounded recursive projection closure and source abstract formation; the new one-step API is not a completed recursive normalizer or IIR rewrite pass.


### 2026-10-10 — Recursive projection closure and concrete layout roots

Added transactional recursive associated-Type normalization, with input-first selection, active-cycle detection, per-call completed-result caching, shared fuel, charged payload copies and dependency-ordered selection traces. Integrated normalization into IIR additional layout roots and verified a selected nominal layout from compiled source. Verified 1,319 source tests plus related query/capability/implementation and runtime-identity tests; details are in development/README.md.

Next connect declaration-side abstract projection formation and rewrite concrete instantiated function/contract/nominal-field types through the normalizer before IIR publication. Do not imply that layout-root support covers those paths. Dependent projections under uninstantiated function binders remain unresolved, while ordinary binder structure and opaque normal-form heads are preserved.


### 2026-10-10 — Nominal field projection materialization

Normalized associated projections in concrete struct and enum field instances, using the same fields for published layouts and dependency discovery. Verified generic instance separation, nested pointer payloads, cycle/missing-value rejection and 1,320 source integration tests. Source abstract formation remains pending. Next centralize type rewriting across expression/pattern annotations so function bodies, signatures and retained contracts cannot disagree during projection resolution; nominal formation predicates/capabilities also need that coordinated rewrite.


### 2026-10-10 — Function annotation projection rewriting

Centralized Expr/Pattern type traversal behind `rewrite_types`, preserving generic substitution, hidden callable identity arguments, pattern expressions and copy-on-write closure bodies. Connected associated normalization to instantiated function signatures, bodies, defaults and program pre/postcondition expression annotations. Added immutable-world builder caching with charged copies. Verified 332 lower-level tests and 1,321 source integration tests, including IIR execution of injected abstract NIR projections for bool/char generic instances. Next normalize type predicates/capabilities and retained interface method contracts before connecting source abstract formation; program predicates remain outside PTC.


### 2026-10-10 — Function/nominal type requirements and capabilities

Normalize associated projections before proving instantiated function and nominal type predicates; retain normalized nominal contracts. Normalize function/nominal capability operands and concrete interface dispatch inputs before selection. Verified 1,322 source tests plus the final projected subject/interface-argument capability case. Runtime program specifications remain outside PTC. Continue with retained interface-method signatures, predicates and program contracts, then declaration-side source projection formation; do not infer those features from the NIR boundary tests.


### 2026-10-10 — Retained interface-method contract normalization

Connected associated projection normalization to retained ordinary methods and concrete generic method specializations, including signatures, predicates, capability operands and runtime contract type annotations. Verified IIR execution of ordinary/generic default methods, stale call-site metadata rejection and normalized retained predicates; 1,323 source tests pass. Next address source-side abstract associated formation and implementation signature admission. Do not bootstrap interface-formation conditions from their own unresolved associated queries; those require explicit staged obligations rather than default substitution.


### 2026-10-10 — Source abstract associated Type formation

Trait-local associated Type names now form source-generated abstract projections in signatures and default bodies, retaining Self and full trait arguments without default substitution. Compiler-root instantiation executes one source default method under bool/default and char/override implementations. Fixed missing layouts for nominal instance arguments erased from normalized signatures. Verified 1,326 source tests and the abstract-associated-types NIR dump example.

Next: source call-site checking must normalize concrete projected signatures before input inference/checking; explicit implementation-method admission needs projection-bearing contract compatibility under the candidate's own associated definitions without assuming unadmitted global evidence. Generic explicit associated views and abstract non-Type associated values remain separate unfinished work. Preserve cycle guards and do not use defaults to discharge abstract equalities.


### 2026-10-10 — Ordinary and generic source method calls

Connected selected projection views to source call checking while retaining declaration-origin NIR signatures. Generic trait inference fixes enclosing inputs before projection normalization and restores original instantiated member signatures on published callees. Given abstract-interface calls can retain symbolic types. Verified real source default/override calls, generic method inference, forwarding, stored unannotated values, rejection cases and 1,331 integration tests; the abstract-associated-types example is now executable.

Next prioritize explicit implementation method admission against associated signatures. Remaining call paths include contextual higher-order types, spread preparation and required-comptime/reference-NIR execution. Do not treat the runtime IIR success as proof of those paths or replace unknown associated values with defaults.

### 2026-10-10 — Projection-bearing override signature admission

Implemented candidate-local associated Type equations for explicit method signature comparison, including transitive inherited defaults and generic implementation/method binders. Equations are dependency-checked and bounded, do not escape into global implementation evidence, and do not unfold normal-form heads. Verified generic bool/char execution, default dependency overrides, cycle/opacity/isolation rejection, 1,334 integration tests and executable/dump example validation.

Next connect explicit associated Type views on abstract subjects carrying given interface evidence. Keep complete interface/member identity and defer concrete value selection until instantiation; abstract non-Type values need a separate representation. Contextual higher-order conversion, spread preparation and required-comptime/reference-NIR projected execution also remain open. Work stays uncommitted on dev.

### 2026-10-10 — Explicit abstract Type views in ordinary definitions

Connected given-evidence projection formation for explicit associated Type views, including inherited origins and complete interface arguments. Ordinary generic signatures and bodies now execute under independently selected implementations; defaults remain opaque during definition checking. Verified 903 execution/elaboration tests and generic forwarding example execution/dump formatting.

Next remove the declaration-order limitation for explicit associated views in trait/implementation method signatures: these signatures are currently checked before the checked-trait catalog is built, and a source probe confirms failure. Establish declaration information without assuming implementation admission or evaluating defaults; preserve cycle/formation obligations. Abstract non-Type members and remaining higher-order/comptime paths remain unfinished. Work stays uncommitted on dev.

### 2026-10-10 — Declaration projections and override body equations

Added bounded declaration-only member/parent queries for explicit associated Type formation in required/default/override method signatures, with given constraints, trait Self and candidate-local own-interface naming. Forward parent declarations and method binders are supported; formation uses remain deferred to existing checks, and no partial trait contracts or implementation evidence are published.

Retained candidate associated definition equations by method identity for body checking after admission. A failing concrete body regression and a generic implementation test verify this connection; rejection tests guard against defaults becoming abstract facts, cross-implementation leakage, missing constraints, ambiguous inherited instances and cycles. All 1,344 integration tests pass; the explicit-Self/generic-forwarding example executes and its dump formats correctly.

Next address contextual function-value conversion for selected associated signatures and early projections that need unrelated concrete implementation selection. Do not confuse declaration evidence with selected implementation evidence. Abstract scalar members, spread preparation and required-comptime/reference-NIR execution remain open. Changes stay uncommitted on dev.

### 2026-10-10 — Selected associated method values

Connected ordinary, contextual generic and explicitly instantiated method values to selected callable views. Preserve the original inner TraitMethod signature and identity for NIR validation, expose the resolved signature to value contexts, and normalize fixed inputs before generic contextual inference. Verified annotations, higher-order/partial inference, function returns, closure capture and phase/type rejection: 914 execution/elaboration tests pass, plus executable example and dump validation.

Next extend the same coherence to ordinary function instances whose signatures contain associated projections, then remaining spread/comptime paths and early concrete selection. Do not infer those from the method-specific tests. Abstract non-Type members remain unfinished. Changes stay uncommitted on dev.

### 2026-10-10 — Ordinary instance views and generic forwarding

Ordinary explicit/context-completed function instances now use selected callable views without changing inner declaration types. Replaced the method-only unresolved-selection fallback with a bounded recursive definition-side type view that preserves given projections and resolves independent concrete parts. Strict normalization and IIR completion remain separate. Verified mixed generic forwarding, annotations, higher-order inputs/returns, type/phase/evidence rejection, 47 kernel tests and 1,352 source integration tests; example execution/dump formatting pass.

Next interleave projection resolution with generic inference when other arguments determine the projection subject. Explicit instantiation and generic forwarding here do not prove that implicit inference path. Early concrete selection, abstract scalar members, spread preparation and required-comptime/reference-NIR execution remain open. Changes stay uncommitted on dev.

### 2026-10-10 — Candidate-local projection refinement during inference

Added transactional, bounded refinement of stable candidate parameter/result components using established associated type views. Recheck obligations under candidate-local definition equations, expose selected parameter contexts to delayed closures, and normalize call results while retaining original callee signatures. Contextual ordinary generic function values also refine before completion. Verified query rollback and candidate isolation, implicit bool/char calls, delayed closures, named-input mutation order and non-injectivity rejection: 29 kernel tests and 1,357 integration tests pass; example execution/dump validation pass.

Next connect spread-specific generic inference to these views, preserving source evaluation order and declaration metadata. Refinement of stable components is not a general solver for open projections. Early concrete selection, abstract scalar members and required-comptime/reference-NIR execution remain open. Changes stay uncommitted on dev.

### 2026-10-10 — Selected spread contexts and result-driven refinement

Connected selected callable views and stable candidate refinement to spread context construction for methods, explicit ordinary instances and result-driven generic calls. Verified opaque packets with unannotated closures, multiple packet partitions, wrong element/width rejection and exact IIR function-entry order/count. All 1,362 integration tests pass; executable example and dump formatting pass.

Next address the verified boundary in `/tmp/luna-associated-spread-input-context.fl`: a normal input determines the subject needed by an opaque spread packet's closure, but packet checking currently precedes that inference. Preserve runtime ordering, packet lifetime boundaries and speculative-state isolation; do not re-execute comptime computations merely to obtain a hint. Early concrete selection, abstract scalar members and required-comptime/reference-NIR execution remain open. Changes stay uncommitted on dev.

### 2026-10-10 — Retained inputs jointly contextualize spread packets

Implemented bounded routing and iterative checking of ordinary/literal-packet inputs before opaque packet checking; retain successful expressions and stage them once in original source order. Leave numeric defaulting until complete call inference. Generic method-local projections now wait for inference rather than premature normalization. Verified receiver/trait methods, named/later inputs, ordering and affine early-exit cleanup.

The combined example exposed repeated nested projection queries exhausting materialization's conservative budget. Added immutable-world shared completed projection caching with transactional publication and charged reads. The example is now an execution regression. Verified 62 kernel tests and 1,370 source integration tests, example execution and dump formatting.

Next repair callable provenance through unpacked tuple packets for higher-order Drop publication. A concrete non-generic control reproduces the unresolved callable obligation, independent of the new preparation pass. Keep normal/early-return affine packet tests. Unknown-width partitioning and positional collection inference still use established fallback paths; early concrete selection, abstract scalar members and required-comptime/reference-NIR execution remain unfinished. Changes stay uncommitted on dev.


### 2026-10-10 — Abstract associated values and selected constants

Added origin-preserving NIR associated values and evidence-backed selection during IIR materialization. Supports trait defaults and overrides, generic/inherited views, dependent member types, aggregate constants and scalar parameter defaults without treating defaults as abstract facts. Added iterative mutable expression traversal and separated interpreter call preparation from recursive invocation; small-stack depth recovery and cleanup remain verified. 1,486 source tests and 355 kernel tests pass, plus executable example and S-expression formatting validation.

Next connect associated values to required-comptime/reference-NIR invocation and dependent associated initializers. Completed runtime instantiation does not establish those paths. General closure constant reconstruction and unresolved associated references in resource/program-contract analysis also remain unfinished. Preserve candidate-local equations, opaque normal-form heads and the separation of PTC from program verification. Changes remain uncommitted on dev.


### 2026-10-10 — Associated values in comptime snapshots

Connected demanded completed associated values to evidence-selected reference dispatch. Required-comptime snapshots retain code identities carried by associated values; runtime bodies remain unavailable to comptime invocation. Verified defaults/overrides, generic implementations, inherited aggregate members, callback selection, malformed/missing dispatch, budget recovery and phase rejection. Source execution/elaboration suites pass (943 tests), as do 355 kernel tests and executable/dump examples; earlier affected resource/instantiation suites also pass.

Next address deferred associated initializer dependencies and projected signatures in reference/comptime execution. Current interpreter installation deliberately accepts completed representations matching the selected implementation; it does not evaluate arbitrary initializer code or normalize opaque heads. Closure constant reconstruction and unresolved associated values in program-contract/resource analysis remain unfinished. Changes remain uncommitted on dev.


### 2026-10-10 — Dependent associated default evaluation

Added selected-implementation initializer contexts and per-selection recursion/cache handling. Trait default expressions retain abstract member references; each implementation publishes its own completed values. Lexical/explicit sibling references, forward edges, generic interface origins, aggregate computation and override-broken cycles execute; selected cycles and runtime initializer calls reject. Validated 1,497 source regressions, five focused tests/groups after the last guard adjustment, 241 kernel tests, executable fixture and readable S-expression dump.

Next investigate general initializer specialization on Self/type parameters and selected inherited-parent/cross-interface contexts. Direct sibling/default success does not establish these paths. Projected reference/comptime signatures, dependent member value types, closure reconstruction and unresolved program/resource analysis remain incomplete. Preserve symbolic defaults, per-implementation cache identity and opaque normal-form heads. Changes remain uncommitted on dev.


### 2026-10-10 — Parent initialization and Self-specialized snapshots

Connected initializer substitution to inherited member queries and required-comptime roots, preserving full origin/argument identity and selected implementation premises. Symbolic defaults now recognize hidden type-parameter operands; generic comptime bodies can demand forward associated dependencies. Independent definition checking rejects evidence laundering, and per-selection cycle checking covers indirect comptime recursion. Validated 1,507 source regressions, 241 kernel tests and executable/dump example.

Next resolve capability-assumption transport for private comptime snapshots. `/tmp/luna-associated-conditional-self.fl` reproduces failure for `read<Self>()` through a Parent implementation requiring `X :- Allowed`; direct projected access already succeeds. Preserve the distinction between given evidence and selected implementation identities, and keep these premises out of standalone runtime graphs. Projected member value types and closure reconstruction remain incomplete. Changes remain uncommitted on dev.


### 2026-10-10 — Private comptime capability premises

Resolved the conditional generic Self probe by passing selected-implementation capability premises into private dispatch preparation and the comptime interpreter. Given obligations remain separate from selected implementation identities; unknown methods/values cannot use defaults as executable evidence. Parent/parameter premise expansion is bounded and charged, and ordinary/runtime interpreter construction starts without premises. Validated 1,510 source regressions, 295 kernel tests, negative dispatch/isolation/recovery cases and executable/dump example.

Next address associated projected signatures and dependent member value types in reference/comptime execution. These still use unnormalized source signatures despite completed member selection. Keep the new premise set invocation-local; it does not provide runtime dictionaries for unknown methods. General closure reconstruction remains unfinished. Changes remain uncommitted on dev.


### 2026-10-10 — Selected projected signatures in reference execution

Added proof-backed reference dispatch retention for associated Type members and retained method signature dependencies. Interpreter selected views now check dependent associated values, callable signatures, nested parameter/results and explicit Type values without unfolding normal-form heads. Validated 1,513 source regressions, four targeted signature/default/type-result tests, 357 kernel tests and executable/dump example.

Next verify projected type predicates and nominal identity/field operations in reference/comptime execution. These require more than callable/value type compatibility. Also inspect projections occurring only in snapshot type operands and nested projected dispatch subjects; current positive tests do not establish those paths. Keep selected values separate from defaults and invocation-local given premises. General closure reconstruction remains unfinished. Changes remain uncommitted on dev.


### 2026-10-10 — Projected type predicate checking

Connected selected associated type views to source formation predicates, reference dependency discovery and required-comptime type contract checking. Preserve original-context proofs before selected-view fallback, Boolean structure and opaque heads. Required-comptime ordinary function instances now revalidate their type requirements with the snapshot host theory. Verified 1,519 source regressions, 298 kernel tests, negative/recovery/budget cases and executable/dump example.

Next verify nominal argument identity/field operations under selected associated types and source-order independence of early associated snapshot dependencies. Predicate-only reference dependencies are collected, but that does not prove every member was already completed when source formation requests its view. Do not solve this by replacing abstract defaults with concrete facts or moving program specifications into PTC. Changes remain uncommitted on dev.


### 2026-10-10 — Selected nominal arguments in reference/comptime values

Struct/enum values retain selected associated type arguments while preserving nominal declaration identity and opaque normal-form heads. Reference dependency collection keeps projection evidence and checks nominal formation/fields under selected inputs. Added six source regressions and one identity kernel regression; validated 1,525 source tests, 299 kernel tests and executable/readable-dump example.

Next investigate source-order independence of early associated snapshot dependencies and nested projected dispatch subjects. Standalone reference callers still prepare the dispatch table explicitly; runtime entry uses its materialization pipeline. General closure constant reconstruction is unfinished. Changes remain uncommitted on dev.


### 2026-10-10 — Associated initialization order

Complete selected Type templates before scalar associated initializers query the immutable capability world. Existing demand/cycle checking remains responsible for template dependencies. Verified all 24 declaration permutations, a later parent Type dependency and mismatched selected-type rejection in both orders. The five affected source suites pass 1,528 tests; executable example and S-expression formatting check pass.

Next fix projected subjects in trait method selection, then reference dispatch. Concrete source probe saved at `/tmp/luna-projected-subject-pending.fl`: `comptime fn forward()->bool where T :- Source, U :- Read, requires U == T.as(Source).Item {T.as(Source).Item.as(Read).read()}` with Source.Item=bool, Read for bool, and associated scalar calling `forward<bool,bool>()` rejects at the inner method call with no-applicable-candidate. First inspect why given evidence modulo the equality is not found. A direct projected capability predicate (`requires T.as(Source).Item :- Read`) instead fails HIR lowering, another missing path. Keep these as real implementation gaps, not rejection semantics. Changes remain on dev without commits.


### 2026-10-10 — Projected trait method subjects

Resolved `/tmp/luna-projected-subject-pending.fl`: static/receiver classification incorrectly queried primitive Type instead of the associated Type's contents. Static projected subjects now use existing given evidence modulo equality. Reference dispatch normalizes subject/interface inputs and retains original type dependencies; interpreter method lookup uses the same selected views. Default Self specialization, contextual lookup, callable values, comptime and runtime/reference paths are covered. Verified 1,533 source regressions, 299 kernel tests and executable/readable-dump example.

Next implement `requires T.as(Source).Item :- Read` through AST/HIR and capability formation, respecting type-only PTC. Separately `U :- Read<T.as(Source).Item>` cannot yet see the earlier T :- Source during capability argument formation. General projected associated-value lookup and capability checks in the interpreter still use raw keys and should share canonicalization once corresponding source paths are supported. Nested projection chains and general closure constant reconstruction remain open. Changes remain uncommitted on dev.


### 2026-10-10 — Positive capability requirements and clause dependencies

TraitBound AST lowers to operand-bearing HIR with S-expression Display. Positive requires atoms/conjunctions compose through existing capability evidence; selected projected inputs are checked at source calls and reference function instantiation. Nominal contracts retain capabilities, failed contract/signature queries restore prior capability maps, and later clause parameters can use earlier bounds. Verified 1,537 source tests and 309 kernel/HIR tests (3 existing doctests ignored); example executes true and HIR/NIR dumps read/format/read.

Next implement full Boolean trait propositions, especially documented negative requirements, without treating missing/unknown candidates as negation proof. Current positive conjunction extraction deliberately stops under not/or; these return an explicit not-implemented error. Review capability formation under these propositions, inherited contracts and implementation admission before expanding support. General projected associated-value lookup still uses raw interpreter keys; nested projection chains and closure constant reconstruction remain open. Changes stay uncommitted on dev.


### 2026-10-10 — Logical trait atoms in PTC

Added TraitQuery/Formula::Trait/Atom::Trait with ordered input identities, Boolean logic, Type-equality congruence, bounded validation, proof replay and Display. Trait truth assignments are deliberately not source witnesses; bare positive/negative queries remain Unknown without implementation-world evidence. Verified 246 elaboration tests, then the expanded eight-test trait group (including 108 finite Boolean oracle comparisons), 1,005 source regressions and formatted dump.

Next build the world-scoped proof adapter before adding NIR TypePredicate trait atoms and source not/or support. World::resolve_instance returns TypeResolution with index identity/revision plus per-candidate type assessments and capability qualifications; Index::type_resolution_is_current can check freshness. Negative evidence must retain the immutable world and candidate exclusions, require a closed query, and distinguish no applicable candidates from pending/conflict/budget failure. Never persist a bare negative Formula as an unconditional global PTC theorem. Existing Context proof replay checks exact assumptions, which can support invocation-local world assumptions once evidence is attached. Source negated/disjunctive trait predicates still explicitly reject as unimplemented. Work stays uncommitted on dev.


### 2026-10-10 — World-scoped trait proposition evidence

Added capabilities/propositions.rs: borrowed TraitDecision with established/excluded/pending states, exact-world and index freshness checks, retained candidate/given evidence, interface-formation checks and bounded inputs. World::prove_proposition collects atoms from goal and context, obtains local facts, and returns WorldProposition owning its private context, decisions and PTC result. Verification rejects different worlds; no standalone decision-to-proof API remains. Verified 256 elaboration tests followed by 16 capability tests after added conflict coverage/API cleanup, plus decision/proof S-expression formatting.

Next add the trait atom to NIR TypePredicate and all substitution/rewrite/display/phase/budget traversals, then lower source not/or to it. Route source contract proving and required-comptime host proving through World::prove_proposition, while retaining generic negative assumptions without global caching. Implementation applicability currently calls PTC directly: avoid an unsound partial integration that ignores trait predicates in candidate requirements or upgrades pending to false. Formation and source dispatch must still demand concrete executable identities, not a Boolean disjunction proof. Queries involving selected associated inputs also need normalization with retained equality evidence. Source negated/disjunctive trait requires are still unimplemented. Changes stay uncommitted on dev.


### 2026-10-10 — NIR Boolean trait predicates and source contracts

TypePredicate::Trait retains TraitRequirement and flows through substitution, Display, rigidity checks, PTC, selected type views, budget/coherence traversal and materialization type dependencies. Source not/or lowers to retained predicates; direct PTC proofs preserve generic assumptions, otherwise formation/comptime host/IIR checks use World::check_predicate. Explicit luna invocation now prepares the implementation index and uses the same host check. Verified 371 kernel tests, 1,541 source tests and runtime/comptime example plus NIR formatting.

Next fix conditional impl candidate applicability: Index::resolve_types_with currently runs ImplementationHead type assessment before Solver::qualify, so a trait predicate in head.requirements remains unknown before qualification can obtain world evidence. Add a properly scoped requirement proof hook after head substitution, preserving TypeEvidence replay/context identity, cycles, budgets and coherence checks. Do not fabricate applicability by assuming the very trait predicate to be established, or silently drop it. Associated/world queries with projected inputs and more general equality-based closure also need continuing audit. Changes remain uncommitted on dev.


### 2026-10-10 — Head matching separate from requirement discharge

Refactored ImplementationHead into match_types -> HeadMatch (private partial evidence, public type_arguments/requirements) and existing assess_subject_types discharging those obligations. Added replay with exact head/query/context/predicate identity and a distinct Display. False requirements can match a head but remain excluded at applicability; trait requirements remain explicit pending obligations. Tests show a matched negative condition can be proved in one World and not reused after adding an implementation. Verified 262 elaboration tests, 1,541 source regressions and readable dump.

Next integrate this matched-head stage with Solver/Index selection while keeping world-conditioned proof support distinct from plain TypeEvidence. Do not add a callback that simply returns PTC proofs with hidden world assumptions and then treats them as verifiable in the original Context. Preserve the active Solver recursion set through nested Boolean requirements, own world/index-bound support in Resolution, and require that support before publishing selected conditional evidence. Current match_types is useful groundwork, not completed source conditional-impl selection. Changes remain uncommitted on dev.


### 2026-10-10 — Explicit supply of runtime implicit parameters

Ordinary implicit parameters now accept explicit named supply through direct, generic, spread and method calls. They do not consume positional slots. Existing source-order preparation, defaults and affine cleanup are reused; owned inputs are transferred and dropped once. Missing automatic supply reports an implementation gap, and callable values requiring implicit protocol metadata still reject. Verified 1,664 execution/elaboration/resources/drop tests; log `/tmp/luna-implicit-final.log`.

Next implement the actual protocol-driven supplier and its declaration/selection contract, rather than searching lexical values by type. Comptime/quote combinations and implicit callable metadata remain open. This follows recent work on bounded effect usage through known factory returns, aggregate projections and selected factory/closure values; that analysis still does not constitute complete effect ownership or cleanup verification. Changes remain uncommitted on dev.


### 2026-10-10 — Post-do routing and closure boundaries

HIR retains trailing inputs as PostLambda arguments instead of calling the preceding application's result. Routing reserves the unique lambda-marked parameter, or the last anchored parameter without a marker; spread inference preserves this identity. Explicit lambdas and deferred block/expression bodies work through generic, indirect and method calls with ordinary capture ownership. Verified 1,669 source regressions and 10 HIR Display tests, plus a focused return-scope test.

Next implement tactic-lambda placeholder binding from the target signature, preserving nested marked/explicit lambda boundaries and full target arity. Do not infer arity from the largest placeholder or enable placeholders at unmarked parameters. Automatic implicit suppliers, quote contexts and callable protocol metadata remain separate unfinished work. Changes stay uncommitted on dev.


### 2026-10-10 — Contextual tactic lambdas

Lambda-marked inputs bind numeric placeholders from the full target callable signature, including generic retry/partial contexts. Nested marked inputs and explicit lambdas isolate their bindings; contextual result constructors require actual use of the current placeholder frame. Repeated placeholders share normal runtime parameter identities, including address/writeback and affine delivery. Post-do bodies retain deferred HIR identity. Verified 1,675 source regressions, 65 syntax/HIR tests and an executable example returning true.

Automatic implicit suppliers, comptime/quote parameter combinations and callable protocol metadata remain unfinished. The broader effect ownership/cleanup integration and language implementation are not complete. Changes remain uncommitted on dev.


### 2026-10-10 — Extended calls compose with post-do inputs

Preserve the extended-call node when appending post-do input, and carry that input through ordinary/mixed property preparation. Generic property packets, tuple collection and receiver methods retain producer order, late capture and affine cleanup. Verified 35 property tests and 11 post-do tests plus the expanded executable example. Custom supplier/collector protocols and quote integration remain open; changes stay uncommitted on dev.


### 2026-10-10 — Explicit sysroot package loading

Connect explicit sysroot compilation to the existing package-set frontend, retaining builtin prelude semantics and source-output alias protection for dependencies. Scope CLI entry selection to the primary package across invocation, interpretation, instantiation and resource analysis. Preserve unscoped library APIs and add scoped counterparts. The 158 relevant CLI/package/module/instantiation tests pass. Shipped library syntax migration and nested package-root discovery remain required; this is not standard-library completion. Changes remain uncommitted on dev.


### 2026-10-10 — Explicit source ownership during package discovery

Exclude canonical paths owned by other explicit package inputs during recursive discovery. Directory package roots prune their subtrees; single-file package inputs exclude only that file, retaining sibling modules. Do not infer package boundaries from filenames. Nested sysroot and dependency-edge isolation now pass alongside ordinary module behavior (143 relevant tests). Shipped library migration remains the next integration gap; changes stay uncommitted on dev.


### 2026-10-10 — Shipped library entry/prelude syntax migration

Remove obsolete external module declarations from builtin/std roots; directory discovery owns these modules. Add explicit builtin.prelude/std.prelude and retain public root exports. Core trait declarations now have public visibility and semicolon termination, use `not`, and retain corrected parent constraints. Rename the unusable reserved-keyword method Default.default to default_value (no current library callers).

`python3 -B development/cargo.py test -p luna --test frontend_cli shipped_library` in `/home/fin/luna` exits 0, checking actual builtin/std root and core-trait AST parsing (`/tmp/luna-library-root-syntax.log`). Whole builtin HIR compilation still exits 1 at iterator.fl's legacy `type Item` declaration (`/tmp/luna-library-migration-probe.log`). Remaining migration includes associated declarations, parameter defaults, Option/Result constructors, iterator protocols and missing library dependencies; root exports are not yet semantically validated as a complete library. Do not claim standard library completion or suppress these files. Changes remain uncommitted on dev.


### 2026-10-10 — Operator protocol declarations and the next elaboration gap

Migrate actual builtin/ops.fl to public traits, `assoc Output: Type`, required signature terminators, and `.Rhs: Type = Self` optional clause syntax. Keep the default semantics instead of silently converting to mandatory Rhs. Reserved-keyword method `not` becomes `invert`; IndexMut explicitly projects its parent's Output via Self.as(Index<Idx>).Output. Names alone do not bind language items.

`python3 -B development/cargo.py test -p luna --test frontend_cli shipped_` from `/home/fin/luna` exits 0 (2 tests; actual shipped roots/core/ops AST parsing and actual primitives+ops package HIR lowering), log `/tmp/luna-library-protocol-hir.log`. The isolated actual-source NIR probe in `/tmp/luna-ops-elaboration-lxxj41z6/lib.fl` exits 1 with `nominal type clause kind is not implemented`, log `/tmp/luna-library-protocol-nir.log`. This identifies optional type clause registration/routing, including trait-Self substitution in defaults, as required compiler work. Do not remove defaults to force the library through. Operator execution and full builtin compilation are not established. Changes stay uncommitted on dev.


### 2026-10-10 — Trait Type defaults and declaration identity repair

Register optional Type clauses on traits and route named overrides independently of positional mandatory inputs. Missing slots elaborate defaults in the definition scope, substitute preceding parameters and the actual subject's Self, and reject cyclic/self/later dependencies. Apply this to implementations, bounds/requires, inheritance, static/receiver methods (including pointer fallback) and associated projections. NIR interface instances carry full arguments; normal-form heads remain opaque. Other nominal/function optional clauses and non-Type trait arguments remain unfinished.

Actual ops execution exposed a pre-existing identity collision: NIR declarations used dense HIR-owner indexes while clause parameters used resolver definition indexes. Source declarations now use their resolver definition identities too (fallback retained for owners without source definitions), preventing Self/parameter aliasing. This affects exported declaration IDs; consumers should use declaration identities rather than assume dense owner numbering.

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli --test resources --test drop --test instantiation --test modules` exited 0 with 1,847 tests (793 execution, 348 elaboration, 17 packages, 86 CLI, 210 resources, 332 drop, 18 instantiation, 43 modules), log `/tmp/luna-trait-default-identities.log`. Tests cover actual shipped primitives+ops execution, named override, chained defaults, trait inheritance, associated projections, requires, pointer receiver selection, cross-package default scope and rejected argument routes. The full builtin/std library still requires migration; no claim of general default normalization completion. Changes remain uncommitted on dev.


### 2026-10-10 — Invoke trait Type defaults after concrete substitution

A default such as `Selected(Self)` previously invoked with a rigid trait Self and failed before post-result substitution could apply. Install the selected subject and preceding arguments in the definition substitution environment before elaborating/invoking the default; restore that environment on success/error. Type-expression Self observes this environment without rewriting the cached definition-side Self. Normal-form default heads remain opaque, with only their arguments substituted.

Verified conditional comptime selection for bool/char subjects, preceding inputs, multiple instances in the same program, computed aliases, and retained normal-form identity. `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli` in `/home/fin/luna` exited 0 (795+348+17+86 = 1,246 tests), log `/tmp/luna-trait-default-invocation-regression.log`. This demonstrates concrete default invocation; arbitrary symbolic default computations and other optional clause kinds are not claimed complete. Changes stay uncommitted on dev.


### 2026-10-10 — Check trait defaults without selecting them

Validate every trait Type default under its declaration binders even when unused or explicitly overridden. This elaborates a Type-valued expression without invoking it, and inspects type-bearing operands to reject self/later-parameter references hidden inside calls. Preserve the definition environment and restore all temporary local/substitution state. Actual default invocation remains a separate omitted-slot operation.

Regression evidence in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli` exited 0 (796 execution, 349 elaboration, 17 packages, 86 CLI = 1,248), log `/tmp/luna-trait-default-definition-regression.log`. Rejection cases cover invalid unused/overridden defaults and direct/call-contained self-reference. A recursive nonterminating comptime function used only as an unused/overridden default is accepted without invocation. Other optional clause forms and general symbolic normalization remain unfinished. Changes stay uncommitted on dev.


### 2026-10-10 — Shared nominal Type-default normalization

Extract type-clause routing, declaration validation and omitted-slot invocation into program/type_defaults.rs. Struct and enum Type defaults now share the trait path: named override, earlier-parameter dependencies, concrete comptime computation, full normalized argument vectors and definition-side scope. Bare names select defaults only when no mandatory input is missing; constructor field values/result context do not silently override defaults. Preserve type-formation constraints, method substitution and error-path alias-guard restoration. Typealias/function optional clauses and non-Type clause values remain unfinished.

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli --test resources --test drop --test modules --test instantiation` exited 0 (797 execution, 350 elaboration, 18 packages, 86 CLI, 210 resources, 332 drop, 43 modules, 18 instantiation = 1,854), log `/tmp/luna-nominal-default-regression.log`. Includes construction, inherited method use, enum payloads, computed defaults, cross-package identity/scope, explicit constraints and rejection cases. `python3 -B development/cargo.py run -p luna -- development/examples/type-defaults.fl --interpret run` exited 0 with `(bool true)`, log `/tmp/luna-type-default-example.log`. Changes remain uncommitted on dev.


### 2026-10-10 — Typealias defaults and computed type-argument plans

Resolve/lower OptionalParam nodes in typealias parameter lists into the same clause identities used by nominal declarations. Typealias defaults now support bare default application, named overrides, preceding parameters and definition-scope normalization while retaining alias transparency and opaque normal-form heads.

Optional struct/enum/alias applications in expression position now build explicit Type-valued argument plans. Evaluate explicit inputs in source order, bind computed inputs once, then supply omitted defaults in declaration order with references to prior bindings. Preserve residual comptime calls and wrap generated nominal/alias Type expressions in the binding block. This supports computation such as `Pair<First()>` with default second parameter equal to the first, without evaluating First twice; reversed named inputs retain source order. Unsupported non-Type and function optional clauses remain separate work.

Validation from `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli` exited 0 (799+351+18+86 = 1,254), log `/tmp/luna-alias-default-regression.log`. After strengthening result/order checks and adding a cross-package alias test, `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages defaults_` exited 0 (48+15+3 = 66, overlapping the earlier suite), log `/tmp/luna-alias-default-final.log`. `python3 -B development/cargo.py test -p resolve -p hir` exited 0 (20 tests, 3 existing ignored doctests), log `/tmp/luna-alias-default-resolution.log`. Updated `development/examples/type-defaults.fl` uses the defaulted Homogeneous alias and executes to `(bool true)` via the standard run wrapper. Changes remain uncommitted on dev.


### 2026-10-10 — Function Type defaults in calls, values and methods

Register/validate optional Type clauses on ordinary function signatures. Build a full type-argument template before call/context inference: mandatory binders remain inferable, optional defaults are fixed, and final inferred inputs substitute into that template. This preserves defaults instead of letting runtime inputs or callable context overwrite them. Explicit function/method applications support named optional inputs; method default substitution includes inherited parameters and nominal Self. All-optional functions can form values without an external callable context.

Validated direct calls, named overrides, interleaved mandatory/optional parameters, preceding-parameter defaults, higher-order/function-value contexts, inherent/trait methods, generic nominal Self and comptime default computation with explicitly supplied preceding types. The updated type-defaults example runs to `(bool true)`.

Checks in `/home/fin/luna`: the initial `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli --test resources --test drop` passed 800 execution, 352 elaboration and 332 drop tests before stopping on one existing CLI diagnostic expectation (`/tmp/luna-function-default-regression.log`). Restored the legacy unsupported-clause diagnostic category while preserving the more specific non-Type explanation. Then `python3 -B development/cargo.py test -p luna --test frontend_cli --test packages --test resources` exited 0 (86+19+210), log `/tmp/luna-function-default-final.log`. Thus all six affected suites have passing evidence (1,799 tests), with the final change limited to diagnostic wording.

Concrete remaining gap: `/tmp/luna-dependent-function-default.fl` calls `keep(true,true)` where `.B:Type=Selected(A)` branches on mandatory inferred A. The default currently invokes before A is inferred and exits 1 (`required comptime evaluation failed: interpretation Type`, `/tmp/luna-dependent-function-default.log`). The explicit `keep<bool>(true)` version is tested and works. Next integrate waiting default computations with partial call inference rather than treating the current eager template construction as complete. Non-Type optional clauses and normal-form-head defaults also remain unimplemented. Changes stay uncommitted on dev.


### 2026-10-11 — Resume computed Type defaults during call inference

Resolve the concrete gap recorded above: omitted function Type defaults whose computations depend on active inference variables now carry dependencies in the call plan. Once those inputs settle, invoke the default under definition scope and constrain the candidate slot; resume existing obligations before completion. Cache settled defaults across candidate retries and process dependent defaults in declaration order. Argument/result context cannot silently replace a default, and a missing mandatory input is not inferred by inverting arbitrary comptime code. Structural defaults retain their existing symbolic behavior; opaque normal-form heads remain opaque.

Tests cover direct calls, phantom defaults, contextual function values, callbacks awaiting their callable type, nontrivial bool/char default chains, inherent methods, mismatched contexts, unresolved dependencies and failed required invocation. The example now includes `Selected(A)` with A inferred from an ordinary argument. General computations over stable but symbolic caller binders, non-Type optional clauses and normal-form-head defaults remain unfinished.

The first expanded regression exposed a stack overflow in the existing 12-level generic-call test (`/tmp/luna-delayed-default-regression.log`), reproduced alone (`/tmp/luna-deep-call-probe.log`). Route application, extended application and projection through the small expression entrypoint instead of retaining the general dispatcher frame at each recursive call. Preserve the common effect-processing exit. The regression explicitly uses a 2 MiB thread stack and passes without increasing it.

Validation from `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli --test resources --test drop` exited 0: 801 execution, 353 elaboration, 19 packages, 86 CLI, 210 resources and 332 drop tests (1,801 total), log `/tmp/luna-delayed-default-final.log`. `python3 -B development/cargo.py run -p luna -- development/examples/type-defaults.fl --interpret run` exited 0 with `(bool true)`, log `/tmp/luna-delayed-default-example.log`. Changes remain uncommitted on dev.

Additional local validation: `python3 -B development/cargo.py test -p elaboration --test calls --test obligations` exited 0 (14 call inference + 9 obligation tests), log `/tmp/luna-delayed-default-kernel.log`.

### 2026-10-11 — Refresh associated callable views after default settlement

A concrete higher-order probe failed after a computed default selected B=bool: the callback parameter B.as(Source).Item remained an associated projection, so `not item` could not check against bool (`/tmp/luna-default-associated-callback.log`). Refresh candidate-local, evidence-backed callable type views immediately after each default constraint and after reapplying cached defaults to a rebuilt candidate. Retain the existing view-query budget and do not export inference state or unfold normal-form heads.

New execution coverage includes delayed closures, actually invoking the callback, contextual generic function values and different bool/char instances in the same program. A rejection test ensures a successful bool instance cannot authorize `not` on a later char callback. The standalone probe now executes to `(bool true)` (`/tmp/luna-default-associated-fixed.log`).

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages defaults` exited 0 (63 execution, 21 elaboration, 3 packages = 87 tests), log `/tmp/luna-default-associated-defaults.log`. `python3 -B development/cargo.py test -p luna --test execution --test elaboration associated` exited 0 (106 execution, 33 elaboration = 139 tests), log `/tmp/luna-default-associated-regression.log`; the two filtered groups overlap. Changes remain uncommitted on dev. This repairs concrete default/callable-view interaction, not general symbolic comptime evaluation.

### 2026-10-11 — Type-only normal-form default applications

Normal-form heads with all-Type clause inputs now accept optional Type clauses. Their type-position applications use the shared default routing/substitution and formation checks, then retain the original head and complete arguments in Term::NormalForm. Expression-position applications reuse the source-ordered, once-bound Type input plan but emit NormalFormType rather than NominalType. Constructing a type never invokes or unfolds the head body. Definition-side default validation also applies to these heads.

Coverage checks complete identity for omitted versus explicit defaults, positional rejection for named-only optional inputs, duplicate names, self-reference, body-opacity rejection, dependent defaults, computed defaults, overridden nonterminating defaults and once-only source-ordered inputs. New executable example: development/examples/normal-form-defaults.fl. Mixed Type/value telescopes with defaults still produce an explicit unsupported diagnostic; scalar and dependent-value defaults require extending the argument plan and validation rather than indexing the Type-only parameter vector with value slots.

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration normal_form` exited 0 (26 execution + 26 elaboration), log `/tmp/luna-normal-default-regression.log`. `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages defaults` exited 0 (65 execution + 22 elaboration + 3 packages), log `/tmp/luna-normal-default-other-defaults.log`; groups overlap. `python3 -B development/cargo.py run -p luna -- development/examples/normal-form-defaults.fl --invoke-comptime Defaulted` exited 0 and printed a retained normal-form head with two bool arguments, log `/tmp/luna-normal-default-example.log`. Changes remain uncommitted on dev.

### 2026-10-11 — Mixed normal-form default input plans

Accept supported scalar/value optional clauses on normal-form heads alongside Type clauses. Keep the full declaration telescope separate from the Type-only binder vector. A dedicated mixed application plan binds explicit inputs in source order, then evaluates omitted defaults in declaration order. Fresh bindings let later scalar/Type defaults reuse preceding values exactly once. Type-position applications run the completed input plan to form the opaque head identity; neither path invokes the head body.

Definition-side default checking uses declared input types, rejects self/later input references, and restores local, substitution, binding, forbidden-input and definition context state on errors. Construction rejects duplicates, wrong default types and missing mandatory inputs. New tests cover integer arithmetic defaults, chains, Type defaults computed from boolean inputs, source ordering, skipped nonterminating overrides, matching complete identities and non-invocation of heads.

Validation from `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages` exited 0 (807 execution + 356 elaboration + 19 packages = 1,182), log `/tmp/luna-mixed-default-final.log`. Updated development/examples/normal-form-defaults.fl includes Bits with a width-dependent storage default. `python3 -B development/cargo.py run -p luna -- development/examples/normal-form-defaults.fl --invoke-comptime PaddedBits` exited 0 with retained head arguments 4 and 5, log `/tmp/luna-mixed-default-example.log`.

Remaining verified gap: a defaulted mixed head with a value input whose type depends on an invocation-time Type input still needs the existing residual NormalFormArgument machinery integrated with the binding plan. `/tmp/luna-mixed-dynamic-default.fl` defines Head<T,value:T,.flag:bool=true> and Build(t:Type) returning Head<t,true>; compilation currently rejects bool against the unresolved declaration input (`/tmp/luna-mixed-dynamic-default.log`). Non-defaulted heads retain their original residual path. Arbitrary symbolic default computations and ordinary-function non-Type optional clauses remain unfinished. Changes stay uncommitted on dev.

### 2026-10-11 — Residual inputs in defaulted normal-form plans

Extend NormalFormType with an optional NormalFormInputPlan: a complete evaluation-order permutation and fresh per-slot bindings. The plan has stable S-expression Display and both phase/interpreter shape validation. Existing constructors keep plan=None and declaration-order semantics. Defaulted mixed constructors retain NormalFormArgument for inputs whose shapes depend on unresolved Type parameters instead of checking them prematurely against rigid declaration types.

Interpretation follows the explicit-input/default order, substitutes settled Type inputs into contextual argument annotations, converts/checks each input against its instantiated declaration type, and exposes its value to later defaults. Temporary plan bindings are removed on every return/error path. Final head identity uses declaration-order arguments, independent of evaluation order; head bodies remain uninvoked. Structural known-input recovery does not bypass a planned evaluation.

The original /tmp/luna-mixed-dynamic-default.fl probe now succeeds (`/tmp/luna-dynamic-plan-probe.log`). Tests cover invocation-time u8 literal conversion, copying a dependent input through a default, bool/i8 rejection and recovery, and malformed orders/binding identities rejected before input execution. The normal-form-defaults example now includes Dynamic, which forms Repeated<u8,200,.copy=200>.

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli` exited 0 (809 execution + 356 elaboration + 19 packages + 86 CLI = 1,270), log `/tmp/luna-dynamic-plan-regression.log`. `python3 -B development/cargo.py test -p nir -p iir` exited 0 (74 tests), log `/tmp/luna-dynamic-plan-ir.log`. `python3 -B development/cargo.py run -p luna -- development/examples/normal-form-defaults.fl --invoke-comptime Dynamic` exited 0, log `/tmp/luna-dynamic-plan-example.log`.

Remaining verified ordering gap: /tmp/luna-late-type-default.fl declares Head<.T:Type=bool,value:T> and constructs Head<200,.T=t>. The value appears before the Type needed for conversion, so execution currently rejects (`/tmp/luna-late-type-default.log`). Supporting this order requires separating effectful input evaluation from target-dependent conversion; do not fix it by silently moving the later Type computation before the value expression. Changes remain uncommitted on dev.

### 2026-10-11 — Stage early values until late Type inputs arrive

Resolve the previously recorded Head<200,.T=t> ordering gap. Planned normal-form invocation can now execute a contextual value input before its target Type is available. Invocation-local staged results retain evaluated values, tuple structure and selected literal identities; effectful conditions, blocks, match scrutinees and guards execute exactly once in source order. After the Type arrives, run the original all-branch type/literal preparation and convert the staged result without replaying expressions. This retains rejection of invalid unselected branches. Boxed immutable contextual arguments keep temporary literal keys stable; no staged interpreter state enters NIR or its caches.

Pending conversion is retried only when a Type input settles, with scans charged to execution fuel. Converted values become available to later defaults. Update the type of a reused default binding when a later explicit Type has become statically known. Early return skips later Type computations, block/match bindings retain lexical scope, and failure/depth exhaustion leaves the interpreter usable.

Tests cover explicit/default late Type suppliers, source call order, integer width checks, unselected overflow, tuple and match-bound values, default reuse, early return, and a 2 MiB stack/depth-exhaustion recovery case. The LateByte entry in development/examples/normal-form-defaults.fl forms an opaque head with u8,200,200.

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test frontend_cli --test packages` exited 0 (814 execution + 356 elaboration + 86 CLI + 19 packages = 1,275), log `/tmp/luna-late-type-regression.log`. `python3 -B development/cargo.py run -p luna -- development/examples/normal-form-defaults.fl --invoke-comptime LateByte` exited 0 with the retained head and expected inputs, log `/tmp/luna-late-type-example.log`. The original /tmp/luna-late-type-default.fl probe also succeeds (`/tmp/luna-late-type-fixed.log`). Ordinary function non-Type optional clauses and general symbolic comptime computations remain separate work. Changes stay uncommitted on dev.

### 2026-10-11 — Preserve symbolic types in structural normal-form default plans

The concrete generic alias probe /tmp/luna-symbolic-normal-default.fl failed because mixed defaults always sent the complete Type construction through concrete invocation, including caller-rigid T. Inspect an elaborated immutable input plan first: follow its validated binding/order map, retain Type identities, and recover scalar constants/structural operations under a bounded local environment. When the complete head application is known, recheck its type-formation requirements and preserve its opaque head instead of invoking the enclosing Type construction. Expressions requiring calls, control flow or contextual conversion still take the original invocation path; no head body is unfolded.

New checks cover generic aliases and function signatures, named input order, definition-package default scope and provider head identity, rejection of unsatisfied formation conditions, non-equality to the head body result, and a selected nonterminating default that must still fail invocation. The normal-form-defaults example now contains Layout/Packed and a generic signature preserving T.

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages normal_form` exited 0 (37 execution + 28 elaboration + 1 package = 66), log `/tmp/luna-symbolic-normal-regression.log`. The corresponding `defaults` filter exited 0 (69 + 23 + 4 = 96), log `/tmp/luna-symbolic-normal-defaults.log`; groups overlap. The expanded example's LateByte invocation exited 0 (`/tmp/luna-symbolic-normal-example.log`). The original generic alias probe now emits NIR with caller-rigid T preserved (`/tmp/luna-symbolic-normal-fixed.log`).

Remaining concrete gap: /tmp/luna-symbolic-computed-normal-default.fl uses .M:Integer=Next(N), where Next is a comptime function and N is concretely 4, alongside symbolic T. Whole-plan invocation still fails on the unrelated symbolic Type input (`/tmp/luna-symbolic-computed-normal-default.log`). This needs per-input invocation/partial construction rather than treating structural recovery as general symbolic computation. Changes remain uncommitted on dev.

### 2026-10-11 — Invoke concrete default inputs independently of symbolic Types

Resolve /tmp/luna-symbolic-computed-normal-default.fl: .M:Integer=Next(N) now invokes with concrete N=4 while unrelated caller-rigid T remains in the normal-form identity. Extend the existing required-invocation pipeline to accept a checked root and typed canonical input bindings. It still builds the checked invocation snapshot, validates phase/formation/program constraints, prepares dispatch and uses the same execution budget. Only the referenced completed bindings become invocation parameters; unrelated symbolic Type inputs are not forced through concrete interpretation.

For eligible mixed plans, traverse inputs in plan order, recover structural inputs, invoke remaining concrete computations and retain their canonical results for later defaults. Preflight contextual inputs before any invocation so falling back to the complete runtime conversion plan cannot replay previously executed work. Normal-form heads remain opaque and complete applications still undergo formation checking.

The interpreter's typed canonical-argument entry reuses normal-form value conversion, validates the callee's arguments, and bounds recursive conversion/clone work. Regression coverage includes computed default chains, tuple inputs, defaults returning Type, rejected nontermination, out-of-range/wrong-shaped invocation arguments and deep-structure rejection followed by successful reuse. The example adds ComputedLayout/ComputedPacked preserving generic T alongside NextStorage(4)=5.

Validation from `/home/fin/luna`: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli` exited 0 (817 execution + 357 elaboration + 20 packages + 86 CLI = 1,280), log `/tmp/luna-symbolic-input-invocation-regression.log`. The original probe now emits the expected symbolic head with concrete integer defaults (`/tmp/luna-symbolic-computed-fixed.log`). The updated example's LateByte invocation exits 0 (`/tmp/luna-symbolic-input-invocation-example.log`). Plans still requiring contextual conversion retain the whole-plan path; arbitrary computations over genuinely unknown inputs and ordinary-function non-Type optional clauses remain unfinished. Changes stay uncommitted on dev.

### 2026-10-11 — Mixed-sort declaration binder core

Before enabling ordinary function value clauses, add explicit DeclarationParameter identity/sort metadata to the stable type core, with S-expression Display. Term::abstract_parameters inserts a mixed Type/comptime-value/runtime binder frame while shifting existing outer references past it. Term::instantiate_parameters validates every supplied argument (including unused parameters), rejects duplicate declaration identities and substitutes by identity without unfolding heads. Expr::instantiate_parameters applies the same checked substitution to all type-bearing NIR structure; it deliberately does not manufacture values for local bindings or erase runtime parameters.

Route the existing Type-only abstraction/instantiation and mixed rigid-substitution entry points through this core. This fixes the old Type abstraction's capture of existing outer-frame references and turns previously ambiguous duplicate declarations into errors. Tests cover nested function binders, outer-scope preservation, mixed-domain arguments, unused-parameter sort checking, duplicate IDs, invalid caller scope, depth overflow, opaque-head preservation and original-template immutability.

Validation in `/home/fin/luna`: `python3 -B development/cargo.py test -p nir -p iir` exited 0 (79 tests), log `/tmp/luna-mixed-binders-facade.log`; `python3 -B development/cargo.py test -p elaboration` exited 0 (309 tests), log `/tmp/luna-mixed-binders-elaboration.log`; `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test frontend_cli --test resources --test drop` exited 0 (817 execution + 357 elaboration + 20 packages + 86 CLI + 210 resources + 332 drop = 1,822), log `/tmp/luna-mixed-binders-final.log`.

This is a binder-core implementation, not completion of ordinary function non-Type clauses. Function metadata still has Type-only declaration parameters; the next integration must carry value-parameter metadata and checked values through argument routing/instance identity, specialize value uses and erase comptime inputs before runtime calls. Source value types and program contracts remain distinct from semantic sorts and must be checked by their respective layers. Changes remain uncommitted on dev.


### Mixed declaration inputs in type contracts

`InterfaceInstance`, `TraitRequirement`, and `TypePredicate` now expose explicitly
sorted `instantiate_parameters`; existing type-only substitution delegates to it.
Unused inputs are validated even for empty interfaces and constant/empty predicates.
Opaque normal-form heads survive substitution; this does not extend PTC to program
specifications. Ordinary function value-input metadata, call routing and runtime
argument erasure remain incomplete.

Validation in `/home/fin/luna`, all exit 0:
- `python3 -B development/cargo.py test -p nir -p iir`: 81 tests,
  `/tmp/luna-contract-inputs.log`.
- `python3 -B development/cargo.py test -p elaboration -p luna --test instantiation`:
  18 tests, `/tmp/luna-contract-instantiation.log`.
- `python3 -B development/cargo.py test -p elaboration`: 309 tests,
  `/tmp/luna-contract-elaboration.log`.

Changes remain uncommitted on dev.


### Whole-function declaration substitution

`Function::instantiate_parameters` specializes signature, body, defaults, type
requirements, capabilities and both program contract expression lists together.
It retains local IDs, entry snapshots and declaration metadata; it performs no
runtime argument erasure. `instantiate_types` is the Type-only facade. Function
materialization now uses this operation instead of independently replacing fields.
The mixed-input regression checks opaque heads, contract operands, metadata and
failure without modifying the original template. Source-level ordinary function
value-parameter routing and erasure still remain outstanding.

Validation in `/home/fin/luna`, all exit 0:
- `python3 -B development/cargo.py test -p nir -p iir`: 82 tests,
  `/tmp/luna-function-specialization-ir.log`.
- `python3 -B development/cargo.py test -p luna --test instantiation --test execution --test frontend_cli`:
  921 tests, `/tmp/luna-function-specialization-integration.log`.
- After extending the failure assertion,
  `python3 -B development/cargo.py test -p nir --test types function_specialization`:
  1 test, `/tmp/luna-function-specialization-failure.log`.

No commit or push.


### Explicit Type-valued declaration clauses

Fixed declaration checking that rejected `T: Type` while accepting bare `T`.
Ordinary functions now elaborate the annotation before applying the unsupported
non-Type input restriction. Nominal telescope registration includes explicitly
typed parameters, and checks their annotations as Type for structs, enums,
traits, implementations and effect groups as well as aliases. An alias to Type
is accepted through ordinary annotation elaboration.

Source tests cover inferred and explicit calls, contextual function values,
optional clause defaults, trait methods, nominal construction, inherent methods,
forward declarations, effects, and rejection of violated equality/trait contracts
and non-Type arguments. This does not enable ordinary non-Type clause inputs or
explicit comptime runtime-function parameters.

Validation in `/home/fin/luna`, exit 0:
`python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages --test instantiation`
passed 819 execution, 358 elaboration, 20 packages and 18 instantiation tests
(1215 total); `/tmp/luna-explicit-type-clauses-regression.log`.
Changes remain in the dev working tree without commit or push.


### Removing selected function telescope inputs

`Term::bind_function_inputs` accepts one optional closed symbolic input per
signature parameter, removes supplied slots and renumbers retained dependencies.
It preserves the function frame even when empty, nested local binders, opaque
normal-form heads, closure environments and comptime-callable wrappers. It checks
original/result scope and input sorts including unused inputs. This is structural
specialization only: source value typing, contracts, stage checks, local-value
replacement and call-site erasure must still be integrated by the caller. No
source-level comptime-parameter support is claimed.

Validation in `/home/fin/luna`, all exit 0:
- `python3 -B development/cargo.py test -p nir -p iir`: 84 tests,
  `/tmp/luna-bind-function-inputs.log`.
- `python3 -B development/cargo.py check -p luna`:
  `/tmp/luna-bind-function-check.log`.
- After adding sequential-versus-simultaneous specialization coverage,
  `python3 -B development/cargo.py test -p nir --test types binding_function_inputs`:
  2 tests, `/tmp/luna-bind-function-focused.log`.

Working tree only, no commit or push.


### Staged input binding in executable function IR

Added `iir::staging::bind_inputs`: after declaration/type substitution, supplied
canonical staged inputs disappear from signature/parameter slots and are restored
as local constants in the body and independently evaluated contracts/defaults.
This preserves mutable local copies, addressability and capture identities without
replaying the original input computation. Defaults are dropped/renumbered according
to retained slots. Entry snapshots of removed parameters become constant bindings;
other snapshots remain. The pass returns a private function copy and charges work.
The iir crate now directly depends on comptime_runtime for canonical integer/bit
vector checks (lockfile updated locally).

Supported data: Type, unit, bool, char, strings, fixed integers, comptime Integer,
BitVec and tuples. Noncanonical/computational expressions, symbolic Type values,
wrong sorts/types, numeric overflow, invalid bit padding and exhausted budgets
reject. Unsupported value representations still require further work.

This is executable IR infrastructure, not source comptime-parameter support. Tests
compile ordinary source functions, explicitly mark the selected IR signature slot
as staged, specialize and invoke with only remaining runtime arguments. Source
parameter metadata, required invocation, instance-key wiring, dependent annotation
substitution and caller argument routing are still outstanding. The caller must
retain supplied values in instance identity and check formation/stage obligations.

Validation in `/home/fin/luna`, all final commands exit 0:
- `python3 -B development/cargo.py test -p nir -p iir`: 84 tests before the added
  canonical-data unit test; `/tmp/luna-staged-inputs-ir.log`.
- `python3 -B development/cargo.py test -p iir`: 8 tests including the new unit
  test; `/tmp/luna-staged-inputs-canonical.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test instantiation`:
  823 execution and 18 instantiation tests; `/tmp/luna-staged-inputs-regression.log`.
Four new execution tests cover mutation/capture, removed optional defaults,
contract/snapshot behavior and recovery, independent defaults, tuple values,
integer ranges, invalid inputs and immutable template preservation.

Working tree remains on dev; no commit/push.


### Staged direct-call routing and instance identities

`iir::staging::bind_call` now pairs the specialized function with a rewritten
runtime call. Existing declaration arguments remain the instance-key prefix;
canonical staged inputs follow in parameter order. Runtime argument slots are
renumbered without sorting the source-order evaluation list. Call-site defaults
restore erased constant bindings while retaining their runtime dependency frame.
Arity, duplicate slots, default ordering, preparation bindings, target identity,
callee signature, canonical inputs and fuel are checked before publication.

Four new execution tests build complete IIR graphs from checked source functions
whose selected signature slot is marked staged explicitly by the test. They verify
both Bool instances, generic type/value keys, empty runtime signatures, deduplication
by key, named input evaluation order, runtime default dependencies, malformed plans,
recovery and template immutability. Graph validation and Interpreter::from_iir /
invoke_instance succeed. This is still a lower-level integration test: frontend
comptime markers, required input invocation, recursive staging discovery and
production instance interning are not wired to this API yet.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p iir`: 7 tests,
  `/tmp/luna-staged-calls-ir.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test instantiation`:
  827 execution and 18 instantiation tests, `/tmp/luna-staged-calls-regression.log`.

Correction to the preceding staged-input note: its final iir run contained 7,
not 8, tests (1 unit + 2 traversal + 4 rewrite); the earlier nir/iir total of
84 before the new unit test remains as reported. No commit/push.


### Source comptime parameters reach runtime instances

The frontend now preserves explicit comptime parameter stages in function
signatures. Committed direct calls freeze staged inputs using the existing
required-invocation path; defaults receive preceding staged bindings. Generic
argument-type inference accepts staged slots without changing their delivery
stage. Constants are reified without retaining active interpreter state.

Materialization discovers canonical staged direct calls, interns their prepared
functions by complete type/value key and queues the rewritten runtime references.
Recursive calls with concrete staged values reuse those keys. Standalone runtime
functions with unbound staged inputs reject; IIR index validation independently
rejects residual staged parameters. Runtime function phase is unchanged.

Staged defaults are exempt from runtime P3 default rules and separately phase
checked as comptime expressions while remaining in NIR. Their bodies are invoked
only when selected; an overridden diverging default does not run. Runtime defaults
may still use preceding erased values through the restored local bindings.

New source tests cover literal/computed inputs, generic inference, optional inputs,
chained staged defaults, runtime default dependencies, runtime evaluation order,
overridden defaults, recursive instance reuse, runtime-input refusal and failed
required invocation. The CLI example `development/examples/staged-parameters.fl`
returns `(tuple (bool true) (unsigned-integer 8))`.

Remaining scope: forwarding/computing with a definition's still-unknown staged
inputs, general staged function values and trait dispatch, dependent annotations,
and comptime-only/graph value residualization need further implementation. Current
source coverage uses runtime-representable canonical data. Ordinary non-Type
clause parameters are still separate outstanding work. This is not complete
comptime parameter support or completion of Flurry.

Validation in `/home/fin/luna`, all final commands exit 0:
- `python3 -B development/cargo.py test -p nir -p iir -p elaboration`:
  394 tests, `/tmp/luna-source-staged-core.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation --test frontend_cli --test packages`:
  1314 tests (831 execution, 359 elaboration, 18 instantiation, 86 CLI, 20 packages),
  `/tmp/luna-source-staged-regression.log`.
- After adding recursive source discovery coverage,
  `python3 -B development/cargo.py test -p luna --test execution source_comptime_parameters`:
  5 tests including the new regression, `/tmp/luna-source-staged-final.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-parameters.fl --interpret run`:
  expected tuple above, `/tmp/luna-staged-example.log`.

All changes remain uncommitted on dev.


### Deferred forwarding of staged bindings

Definition checking now retains direct references to the current function's
staged parameters, including structural tuples, instead of trying to invoke an
unknown local. Defaults simultaneously substitute already prepared staged inputs;
newly inserted caller references are not recursively interpreted as callee-frame
references. `iir::visit::expression_mut_pruned` provides this bounded operation and
has a regression for self-referential substitution. Copy work is charged.

Instance binding substitutes these references only inside staged call arguments,
then ordinary materialization discovers the resulting concrete calls. Runtime
reads/captures retain their local identities. Forwarding works through generic
wrappers, recursive calls, closures, tuple construction and staged/runtime default
chains. Until more precise stage-aware flow facts exist, any write or address
escape of a forwarded binding anywhere in the template makes that forwarding
unsupported; this prevents substituting an entry constant for changed runtime
storage. Ordinary non-forwarding uses retain prior behavior. Runtime dependencies
inside a staged aggregate still reject.

Arbitrary computations over still-unknown staged inputs (beyond structural tuple
assembly), general function-value/trait dispatch and dependent residualization
remain outstanding. The example now includes a forwarding wrapper.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p iir`: 8 tests,
  `/tmp/luna-staged-forward-ir.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  836 execution, 360 elaboration, 18 instantiation tests,
  `/tmp/luna-staged-forward-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-parameters.fl --interpret run`:
  `(tuple (bool true) (unsigned-integer 8))`, `/tmp/luna-staged-forward-example.log`.

No commit/push; dev working tree retained.


### Effect-free staged expression folding

Deferred staged forwarding now admits boolean negation/conjunction/disjunction,
scalar equality, conditionals and empty-statement blocks, composed with tuples.
After enclosing inputs are substituted, direct-call routing folds this supported
residual structure into canonical data before instance binding. It preserves
short-circuit selection and checks type consistency, depth and work limits.
Generated bound inputs contain literals/aggregate data, not residual boolean
computations. Type equality is not handled by this scalar fold; arbitrary user
comptime calls still require the separate invocation path.

Source tests cover each operator, computed defaults, recursive boolean switching,
conditional integer payloads, machine integer equality and mixed tuples. IR tests
cover untaken inputs, selected failures, depth/fuel failure and recovery, and
refusal to treat Type equality as scalar equality. Newly selected constants are
constructed without cloning discarded expression branches.

General deferred user calls, arithmetic, nonempty blocks and richer stage-aware
flow analysis remain outstanding, alongside previously recorded function-value,
trait-dispatch and dependent-residualization work. The CLI example now selects a
value through a generic wrapper containing `not use_first`.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p iir`: 9 tests,
  `/tmp/luna-staged-fold-ir.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  838 execution, 360 elaboration, 18 instantiation tests,
  `/tmp/luna-staged-fold-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-parameters.fl --interpret run`:
  `(tuple (bool true) (unsigned-integer 8))`, `/tmp/luna-staged-fold-example.log`.

Changes remain uncommitted on dev.

### Staged arithmetic and local block evaluation

Direct staged forwarding now folds fixed-width integer addition, subtraction,
multiplication and negation, plus ordered integer/character comparisons. It uses
`comptime_runtime::machine::MachineInteger`, preserving width, overflow and work
limits. Source contracts remain program-verification obligations; specialization
cannot discharge a false caller precondition. Source integer division still lacks
its rounding protocol and is not introduced here. The prior arithmetic regression
log `/tmp/luna-staged-arithmetic-regression.log` records 360 elaboration, 841
execution and 18 instantiation passes; this is retained evidence, not a rerun.

This increment adds effect-free `do` blocks with local bindings in deferred staged
arguments, including nested blocks, shadowing, selected conditional branches and
parameter defaults. Checked local references are admitted during definition
checking; instance-time folding evaluates statements in order, substitutes only
canonical immutable data, and folds the result. Discarded expressions still
execute and fail normally. Environments are block-local, substitutions preserve
source locations, and traversal/copy work consumes the existing budget. The
runtime residual contains no operations from the completed staged computation.

Runtime dependencies (including discarded reads and runtime calls) remain
rejected. Arbitrary deferred user comptime calls, assignments, pattern bindings,
loops and general dependent residualization still need implementation; these are
implementation gaps, not language restrictions.

Validation in `/home/fin/luna`, all exit 0, using the authorized cargo wrapper:
- `python3 -B development/cargo.py test -p iir`: 11 tests;
  `/tmp/luna-staged-block-ir.log`. Includes use-before-initialization, duplicate
  binding, local type mismatch, nested scope escape, discarded overflow, and
  budget failure/recovery checks.
- `python3 -B development/cargo.py test -p luna --test execution source_comptime_forwarding`:
  10 tests; `/tmp/luna-staged-block-forwarding.log`.
- `python3 -B development/cargo.py test -p luna --test elaboration staged_`:
  3 tests; `/tmp/luna-staged-block-rejections.log`.
- `python3 -B development/cargo.py test -p luna --test instantiation`:
  18 tests; `/tmp/luna-staged-block-instances.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-arithmetic.fl --interpret run`:
  `(tuple (unsigned-integer 4) (unsigned-integer 14))`;
  `/tmp/luna-staged-block-example.log`.

Existing warnings remain. Changes are uncommitted on dev; no full-suite rerun.

### Structural pattern bindings in deferred staged blocks

Instance-time local block folding now supports checked irrefutable tuple, nested
binding, wildcard and unit patterns. The producer is folded once before any
binding; tuple shape/arity and duplicate local identities are checked. Bound
canonical values enter only the current block environment and consume copy/work
budget. Nested scopes and source locations retain the preceding block semantics.
Definition checking admits these structural patterns while retaining deferred
reads of enclosing comptime inputs. Computed/protocol patterns, refutable matching,
guards and alternative control flow still require broader deferred invocation;
this change does not bypass their checks or invoke a pattern-view protocol twice.

Source tests cover nested tuples, wildcard fields, default arguments, conditional
producers and shadowed destructuring. Specialized runtime bodies contain no staged
LetPattern nodes. Wildcard fields cannot discard a runtime read/call needed to
construct the input. IR tests cover malformed shapes, duplicate bindings, type of
producer, unsupported matching, depth/work limits and recovery after failure.

Validation in `/home/fin/luna`, all exit 0:
- `python3 -B development/cargo.py test -p iir`: 12 tests;
  `/tmp/luna-staged-pattern-ir.log`.
- `python3 -B development/cargo.py test -p luna --test execution source_comptime_`:
  17 tests; `/tmp/luna-staged-pattern-execution.log`.
- `python3 -B development/cargo.py test -p luna --test elaboration staged_`:
  4 tests; `/tmp/luna-staged-pattern-rejection.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-patterns.fl --interpret run`:
  `(tuple (unsigned-integer 3) (unsigned-integer 8))`;
  `/tmp/luna-staged-pattern-example.log`.

No full-suite run; existing warnings remain. Dev changes remain uncommitted.

### Execution contexts for staged call inputs

The NIR phase traversal now carries the execution context on each expression and
pattern work item. Arguments routed to non-runtime parameter slots are checked
under comptime; ordinary argument slots retain the caller's context. This follows
the recorded parameter index rather than argument-vector/source order, preserving
named argument routing. The context propagates through blocks, nested calls,
patterns, closures and other checked expressions without changing the owner's
function category. A runtime function called inside a staged argument still
rejects; a comptime call in an ordinary runtime argument still rejects. Invalid
parameter indices now produce a phase diagnostic rather than being traversed as
if they named a valid input.

Staged defaults use the same context-carrying traversal. The former temporary
Function/body clones for their phase checks have been removed. Defaults continue
to be checked even without a selected call site. Input availability, resource
legality and termination remain separate obligations; this phase pass does not
prove that a runtime local is available during comptime.

This is a prerequisite for general deferred invocation, not its implementation.
The current source classifier and runtime materializer still restrict deferred
inputs to the supported effect-free fragment. General invocation needs a demanded
comptime snapshot, concrete environment and dispatch preparation, shared work
budget, contract checks and canonical result freezing before IIR publication.

Validation in `/home/fin/luna`, all exit 0 after the final change:
- `python3 -B development/cargo.py test -p nir`: 81 tests,
  `/tmp/luna-staged-phase-nir.log`; new phase tests cover mixed-stage named
  argument reordering, nested calls/blocks, invalid indices and defaults.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  844 execution, 363 elaboration and 18 instantiation tests,
  `/tmp/luna-staged-phase-regression.log`. The broader check is justified by the
  shared phase traversal change.
- `git diff --check`: passed.

Existing warnings remain. No commit or push; changes retained on dev.

### Deferred user comptime invocation for concrete staged inputs

Definition checking now retains direct comptime calls whose inputs depend on the
current function's staged bindings. After enclosing type/value substitution,
materialization evaluates such staged argument trees with the existing comptime
interpreter in a private package snapshot. A synthetic comptime root preserves the
checked expression, demanded dispatch preparation selects concrete implementations,
and the existing type-predicate callback and executable pre/postconditions remain
active. Dispatch preparation returns its remaining traversal allowance to the
parent builder; interpretation and result freezing continue from that allowance.
The runtime instance is bound only after the result has been frozen into supported
canonical builtin data. Helper code and the synthetic entry do not enter the IIR
runtime graph. Failure publishes no partial graph or persistent interpreter state.

This supports direct and generic user helpers, helper-local loops/mutation,
conditional non-invocation, aggregate results, and calls in independent staged
defaults. A default that references an earlier still-deferred invocation is
explicitly rejected for now: the earlier substitution scheme copied expression
trees and would replay that invocation. A retained argument preparation frame is
required to remove this restriction without duplicating comptime effects. Runtime
reads cannot become comptime inputs merely by being passed through a helper.

Remaining limits: snapshot creation currently copies the checked package rather
than retaining only demanded bodies; general function-value/trait-call argument
classification, graph/closure/float result freezing, Type-only residualization and
more general staged statement/control-flow construction remain outstanding. The
new interpreter path does execute control flow inside admitted helper functions;
it is not another operator-only folding implementation. Normal-form heads remain
opaque, and program contracts are not transferred to PTC.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py check -p elaboration`:
  `/tmp/luna-staged-invoke-check.log` (before the final behavior tests).
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  848 execution, 364 elaboration, 18 instantiation tests;
  `/tmp/luna-staged-invoke-regression.log`.
- Focused tests additionally inspect absence of comptime helpers in runtime IIR,
  recursion/budget failure followed by successful independent instantiation,
  skipped recursive branches, loop execution, injected false pre/postconditions,
  aggregate/default results and the dependent-default replay rejection.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-invocation.fl --interpret run`:
  `(tuple (unsigned-integer 8) (unsigned-integer 3))`;
  `/tmp/luna-staged-invoke-example.log`.
- `git diff --check`: passed.

Existing warnings remain. Changes remain uncommitted on dev.

### Prepared frames for deferred staged default chains

The prior dependent-default restriction is removed. Definition checking retains
callee-frame references to earlier deferred inputs instead of copying their
producer expression trees. `iir::staging::bind_call_with` validates the complete
routing plan before invoking producers, then prepares staged inputs once in the
recorded explicit/default order. A private frame stores completed immutable
results; defaults substitute those results before the host evaluator runs. The
original `bind_call` uses the same path with the effect-free evaluator.
Materialization supplies the existing required-invocation evaluator when a
prepared expression contains a comptime call.

Forwarded input substitution now tracks call-frame masking. Default references
belong to the callee even when recursion reuses the caller's LocalIds; nested
calls preserve that distinction. Ordinary explicit references remain caller
references. Erased inputs used by runtime defaults retain the earlier constant
binding behavior. Depth, traversal and copying remain bounded.

Source tests cover dependent calls/default chains, recursive frame separation,
runtime defaults using completed staged values, overridden divergent defaults,
and default failure followed by successful independent preparation. A host
producer-count test confirms one invocation across three staged inputs, and
asserts that a malformed plan executes no producers. These tests replace the
previous rejection-only fixture; runtime dependencies still reject.

Validation in `/home/fin/luna`, all exit 0:
- `python3 -B development/cargo.py test -p iir`: 12 tests;
  `/tmp/luna-staged-frame-ir.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  851 execution, 364 elaboration and 18 instantiation tests;
  `/tmp/luna-staged-frame-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-invocation.fl --interpret run`:
  `(tuple (unsigned-integer 8) (unsigned-integer 3))`;
  `/tmp/luna-staged-frame-example.log`. The example now uses a dependent default.
- `git diff --check`: passed.

The private invocation snapshot still copies the checked package. General
function-value/trait input classification, richer frozen values and dependent
Type residualization remain outstanding. No commit/push; dev remains uncommitted.

### Deferred comptime trait dispatch and Type-only dependencies

Staged input classification now retains phase-correct TraitMethod references.
It also detects references to the current owner's Type parameters in direct
function-instance arguments and in trait subjects/interface/method arguments.
Such a call can wait for specialization even when it has no value argument that
mentions an enclosing comptime parameter. Known concrete inputs still use the
existing immediate required-invocation path. Opaque normal-form heads are not
unfolded to decide this dependency.

Concrete substitution and the existing private invocation snapshot then prepare
trait dispatch and execute the selected comptime method. No separate method
selection algorithm or runtime fallback was introduced. Source tests inspect
that the runtime graph contains only runtime functions and cover distinct
implementations, interface argument identities on the same subject, method-local
Type arguments, inherited/default bodies, default-input chains, value receivers,
and direct/generic helper calls depending only on Type inputs. Phase and trait
requirements retain their rejection behavior.

Before the change, the new generic trait probe failed during definition checking
with unresolved dispatch selection; `/tmp/luna-staged-trait-before.log` retains
that failure. It now succeeds after specialization. The runtime-method rejection
fixture was adjusted to the actual phase diagnostic, `callable type is unavailable
in this execution phase`; the operation remains rejected.

Validation in `/home/fin/luna`, final commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  89 execution and 18 elaboration tests;
  `/tmp/luna-staged-trait-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-trait-dispatch.fl --interpret run`:
  `(tuple (bool true) (bool false))`; `/tmp/luna-staged-trait-example.log`.
- `git diff --check`: passed.

Associated function values/closures and more general deferred statement trees
remain separate work, as do snapshot copying and richer frozen values. No full
suite rerun or commit/push; changes remain on dev.

### Associated callable values and closures in staged input computation

Deferred input classification now admits checked associated-value projections
and retains dependencies on the current owner's Type arguments in their subjects
and interface arguments. Existing concrete associated-value resolution supplies
the selected value before staged evaluation. This connects associated scalar
constants, comptime function identities and captured comptime closures to the
same private invocation path, without exporting intermediate closures into the
runtime graph or bypassing trait selection.

A shared-state closure probe exposed a missing Discard node in the deferred
expression subset. Discard now evaluates its operand and only discards the
result; both the effect-free fold and required invocation preserve failures.
The shared closure is constructed once by a local binding, and consecutive calls
observe its mutations. Separate materializations start from fresh invocation
state. Existing factory-returned closure calls and higher-order helper arguments
were also checked end to end; their intermediate closure values stay inside the
interpreter and only the final supported value is frozen.

Tests cover associated function/scalar/closure selection, parameterized interface
identity, dependent defaults, mutable local closure state, repeated independent
materialization, factory/higher-order invocation, absence of comptime closures in
runtime IR, and rejection of runtime input dependencies. IR discard tests cover
unknown locals, overflow, work exhaustion and recovery.

Validation in `/home/fin/luna`, final commands exit 0:
- `python3 -B development/cargo.py test -p iir`: 13 tests;
  `/tmp/luna-staged-associated-ir.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  93 execution and 19 elaboration tests;
  `/tmp/luna-staged-associated-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-associated-callable.fl --interpret run`:
  `(tuple (unsigned-integer 3) (unsigned-integer 8))`;
  `/tmp/luna-staged-associated-example.log`.
- `git diff --check`: passed.

This does not implement general closure-valued instance keys, arbitrary local
stage flow, or dependent Type residualization. Whole-package snapshot copying
also remains to be reduced. Existing warnings remain; no full-suite rerun or
commit/push, and dev changes remain uncommitted.

### Reuse immutable staged-invocation snapshots within one materialization

A materialization Builder now creates its checked package snapshot lazily and
reuses that code/metadata copy for subsequent staged-input invocations. Every
invocation replaces the synthetic entry's body and result signature, rebuilds
concrete dispatch, and constructs a fresh interpreter. It does not cache values,
closures, mutable environments, events, or invocation results. The scratch root
and dispatch table are cleared on success and failure before retaining the
snapshot. The snapshot is private to this Builder and is dropped with it.

Package-copy work is charged once; each new input expression and result type are
still charged, as are dispatch preparation, interpretation and freezing. The
first invocation still copies the complete package, and phase/interpreter
preparation still examines checked metadata; reducing that work to a demanded
subset is future work rather than a claimed result of this change.

A behavior/budget regression compiles 128 unrelated functions with flat bodies
and specializes 16 distinct staged inputs within 30,000 steps, checking all 16
results and exclusion of unused functions from IIR. Another regression alternates
Bool, machine integer, Char and Str results to check scratch signature replacement.
Existing trait-dispatch, closure state, contract failure and recovery tests pass
with snapshot reuse.

Validation in `/home/fin/luna`, final commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution staged_invocations_do_not_recopy`:
  1 test; `/tmp/luna-staged-snapshot-budget.log`.
- `python3 -B development/cargo.py test -p luna --test execution staged_snapshot_reuse`:
  1 test; `/tmp/luna-staged-snapshot-types.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  93 execution and 19 elaboration tests;
  `/tmp/luna-staged-snapshot-regression.log`.
- `python3 -B development/cargo.py test -p luna --test instantiation`:
  18 tests; `/tmp/luna-staged-snapshot-instances.log`.
- `git diff --check`: passed.

Separate observation: the first budget fixture used 128 functions whose bodies
contained 32 nested `not (...)` expressions. That fixture overflowed the test
thread's stack before this optimization (`/tmp/luna-staged-snapshot-before.log`),
and did so with the optimization as well. It is not evidence of budget exhaustion.
The final fixture uses 32 flat discarded expressions per unused body to isolate
copying cost. The nested-expression stack issue remains untriaged; it is not
claimed fixed by snapshot reuse.

No full-suite rerun or commit/push; changes remain uncommitted on dev.

### Fix scalar-expression stack growth in elaboration and flow verification

The previously recorded nested-expression failure was minimized to a single
function containing 32 nested `not (...)` expressions on the ordinary Rust test
thread stack. AST and HIR emission succeeded; NIR compilation overflowed the
stack. Disassembly of the pre-change `Checker::expr_context_inner` showed a stack
reservation of approximately 96 KiB per recursive dispatcher frame.

Unary negation/not, logical operators, comparisons and BitVec concatenation now
use dedicated non-inlined operator entrypoints outside the general expression
dispatcher. Their original expected-type handling, negative literal proxy paths,
Never handling, comparison direction and final computation handling are retained.
This does not increase the thread stack or relax type checks.

Expanded tests found the same structural problem in integer-negation program
verification. `Walker::inner` is now a small wrapper for budget/depth/reachability
checks and final Boolean facts. Integer and logical/scalar-comparison operations
run in separate frames; the general control-flow dispatcher remains non-inlined.
The existing arithmetic range proofs, polynomial refinement, source diagnostics,
short-circuit state/fact joins and recorded verification points are preserved.

`compiler/luna/tests/nesting.rs` launches isolated subprocess probes on normal
test-thread stacks. Its parent covers 8 operator scenarios across AST, HIR and
NIR (24 phase scenarios); NIR cases additionally execute and check the result.
Cases include not, and, or, equality, signed negation, BitVec complement and
concatenation, and nested Boolean conditions in a function requiring machine
arithmetic verification. The child helper is marked ignored for normal harness
execution and is explicitly invoked with `--ignored` by the parent, rather than
counted as an empty passing test.

Validation in `/home/fin/luna`:
- Baseline minimized probe: exit 101, NIR stack overflow;
  `/tmp/luna-nesting-before.log`.
- `python3 -B development/cargo.py test -p luna --test nesting`: exit 0,
  parent regression passed all 24 subprocess scenarios;
  `/tmp/luna-nesting-operators.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  exit 0, 862 execution, 366 elaboration and 18 instantiation tests;
  `/tmp/luna-nesting-regression.log`.
- `python3 -B development/cargo.py test -p elaboration --test verification`:
  exit 0, 32 tests; `/tmp/luna-nesting-verification.log`.
- `git diff --check`: passed.

This resolves the demonstrated scalar recursion failure; it is not a claim that
arbitrarily deep syntax or every control-flow nesting shape is now stack-safe.
Existing work/depth limits remain. No commit/push; dev changes remain uncommitted.

### Local control flow in deferred staged arguments

Deferred staged blocks now admit assignment to block-owned local bindings,
while-loop IR, local break/continue targets and executable predicate checks.
The source classifier tracks bindings and loop identities belonging to the
staged expression. It does not grant access to caller storage or caller loop
control. Field writes and nonlocal return/control flow remain outside this
subset. Concrete materialization routes these stateful nodes through the existing
comptime interpreter instead of the effect-free scalar fold.

Source cases exercise direct mutation, finite while loops, nested loops,
break/continue and default-argument frames. Generated runtime graphs contain
neither the staged loops nor their writes. Infinite loops consume invocation
fuel; failed predicates abort preparation, and later independent materialization
still succeeds. Negative cases retain rejection of caller-variable writes,
mutation of an enclosing staged parameter, runtime loop conditions and breaks
that would leave a caller-owned loop. Program predicates remain interpreter/
program-verification checks, not PTC assertions.

`development/examples/staged-local-control.fl` computes parity with a staged
u8 decrement loop. The loop guard supplies the ordinary machine-safety proof;
no extra assumption or weakened arithmetic rule is needed. Boundary tests cover
0 and 255 and inspect the absence of runtime loop nodes.

Validation in `/home/fin/luna`, final commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  96 execution and 20 elaboration tests;
  `/tmp/luna-staged-control-regression.log`.
- `python3 -B development/cargo.py test -p luna --test elaboration deferred_comptime_local_control_flow`:
  1 test after adding the caller-loop escape case;
  `/tmp/luna-staged-control-boundary.log` (included test, not an extra unique test).
- `python3 -B development/cargo.py test -p luna --test execution deferred_comptime_numeric_local_loop`:
  1 additional boundary test; `/tmp/luna-staged-control-numeric.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-local-control.fl --interpret run`:
  `(tuple (bool true) (bool false))`; `/tmp/luna-staged-control-example.log`.
- `git diff --check`: passed.

No full-suite rerun or commit/push; changes remain uncommitted on dev.

### Deferred staged pattern matching

Staged call inputs now defer checked match expressions until the enclosing
comptime values or declaration type arguments are available. The availability
walk collects pattern-owned binding identities (including `or` remapping) and
checks embedded guard/`and … is …` expressions, every arm body and the scrutinee.
Record/variant construction can supply intermediate matched values. TypeValue
nodes retain dependencies on declaration type parameters. Concrete matching uses
the existing comptime interpreter; no second matcher or PTC program-state rule
was introduced.

Source tests cover literal/tuple/record/variant patterns, nested matches, `or`
bindings, guard side-effect order, chained patterns, defaults, skipped divergent
calls, fuel failure followed by successful independent materialization, and
runtime-dependency rejection in scrutinees, guards, chained computations and
bodies. Runtime IIR is inspected for absence of staged matches. An opaque-head
case distinguishes Identity<bool> from bool without unfolding Identity's body.
Intermediate nominal values do not imply support for arbitrary nominal/closure
graphs as final instance-key inputs; those limits remain.

Validation in `/home/fin/luna`, commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  101 execution and 21 elaboration tests, including five new tests;
  `/tmp/luna-staged-match-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-patterns.fl --interpret run`:
  `(tuple (bool true) (bool false) (bool true) (bool false))`;
  `/tmp/luna-staged-match-example.log`.
- `git diff --check`: passed.

The first nominal/type-pattern fixture needed whitespace between `>` and `=>`
to avoid lexing `>=`; corrected before the successful regression run. No full
suite, commit or push; changes remain in the dev worktree.

### Deferred staged destructuring and let-else

The staged availability classifier now handles LetPattern through the same
bounded pattern walk as match, including initializer, embedded pattern
computations, optional guard and failure branch. This replaces the earlier
Wild/Unit/Tuple-only block special case. Concrete inputs containing LetPattern
use the existing interpreter, preserving candidate rollback, guard side effects,
and local loop transfers instead of adding another evaluator.

Four source tests cover guarded/refutable declarations, local break/continue,
`or` remapping, record destructuring, chained patterns, default parameter frames,
shadowed names in failure branches, initializer evaluation once, divergent
selected paths and independent recovery. Negative cases reject runtime inputs
in guards, chained patterns and else branches and caller-function returns.
Successful runtime graphs are checked for removal of staged destructuring and
loops. The existing rejection of caller-owned loop escapes remains in force;
this does not introduce staged nonlocal return semantics.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  104 execution + 22 elaboration tests;
  `/tmp/luna-staged-let-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-destructuring.fl --interpret run`:
  `(tuple (bool false) (bool false) (bool true))`;
  `/tmp/luna-staged-let-example.log`.
- `git diff --check`: passed.

No full-suite run or commit/push; changes remain on dev.

### Deferred staged aggregate field access

Checked nominal and anonymous field reads now participate in staged dependency
classification. Assignments may traverse structural field paths rooted in a
local owned by the staged expression; NIR Place paths cannot hide dereferences
or target computations. Caller roots and pointer operations remain unavailable.
Field operations use the existing comptime interpreter. Final arbitrary object
graph instance inputs remain unsupported; local aggregates here yield supported
scalar inputs after computation.

The anonymous-record case exposed missing CallPreparation classification:
extended-property calls had attempted required evaluation before their outer
comptime parameter was supplied. These nodes now share the availability walk
for local bindings and execute using the interpreter's dedicated temporary
lifetime behavior, without rewriting them into ordinary blocks.

Four new tests cover nominal and anonymous records, nested reads/writes,
replacement of a nested record, tuple destructuring into a record, default
frames, property evaluation order with mutation, and runtime IIR erasure.
Negative inputs reject caller field reads/writes, runtime field initializers
and pointer writes to caller storage.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  107 execution + 23 elaboration tests;
  `/tmp/luna-staged-fields-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-fields.fl --interpret run`:
  `(tuple (bool false) (bool true))`;
  `/tmp/luna-staged-fields-example.log`.
- `git diff --check`: passed.

No full suite or commit/push. Changes remain in the dev worktree.

### Deferred staged BitVec computation

Staged input classification now admits checked TakeBits, conversion, static
slices, concatenation, equality, bitwise Boolean operations and local BitAssign.
Only expression-owned structural places can be written. Concrete bit operations
use the existing interpreter, including eager evaluation of both BitVec and/or
operands. No source width, construction, narrowing or storage-validation checks
were bypassed; dynamic slice lowering is not added by this change.

Seven new tests cover nibble extraction/reordering, complement/masks, zero-width
slices, scalar and nested-field writeback, final BitVec instance inputs and
default frames, eager evaluation once, equality, zero-width keys and 129-bit
cross-word keys. Rejection tests cover runtime dependencies, writes to caller
storage or enclosing staged parameters, out-of-range slices/literals and implicit
narrowing. Runtime IIR checks confirm that the tested staged bit computations
were consumed during materialization.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  111 execution + 25 elaboration tests;
  `/tmp/luna-staged-bitvec-regression.log`.
- `python3 -B development/cargo.py test -p luna --test execution deferred_comptime_bitvec_instance_keys`:
  1 additional cross-width test; `/tmp/luna-staged-bitvec-widths.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-bitvec.fl --interpret run`:
  `(tuple (unsigned-integer 0) (unsigned-integer 255) (unsigned-integer 250) (unsigned-integer 63))`;
  `/tmp/luna-staged-bitvec-example.log`.
- `git diff --check`: passed.

An initial fixture incorrectly omitted the required default for an optional
`.bits` parameter; corrected to `=0` before regression validation. No full-suite
run or commit/push; work remains on dev.

### Local comptime closures in staged input expressions

Staged arguments now carry a separate closure-construction phase context through
direct call inputs, generic inference attempts/retries and staged defaults. This
does not set required-invocation mode: unresolved enclosing staged parameters
still defer rather than execute eagerly. Default-declaration checking resets the
caller context and restores it on success/error. Ordinary runtime positions
still reject a comptime lambda target, as covered by existing phase tests.

The staged availability walk validates closure environment construction, using
explicit capture values or checked lexical bindings. The closure body retains
its separately checked invocation phase and local control-flow namespace.
Forwarded canonical inputs and prepared default bindings now populate implicit
captures with explicit completed values; otherwise a synthetic invocation would
try to find the original caller binding. Local captured state remains owned by
the fresh interpreter invocation, not cached as an instance-key object graph.

Four new tests exercise direct staged capture, parameterized lambdas, repeated
mutation of closure-owned state, generic calls, concrete and deferred default
frames, nested closure captures/returns, runtime capture rejection and runtime
callee rejection. The generic/default test checks that closure nodes do not
survive in the generated runtime graph. Final closure/object-graph inputs are
still unsupported; conservative rejection after writes/address escape is not
relaxed by this work.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  115 execution + 26 elaboration tests;
  `/tmp/luna-staged-closures-regression.log`.
- `python3 -B development/cargo.py test -p iir`: 13 tests;
  `/tmp/luna-staged-closures-iir.log`.
- `python3 -B development/cargo.py test -p luna --test instantiation`: 18 tests;
  `/tmp/luna-staged-closures-instances.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-closures.fl --interpret run`:
  `(tuple (bool false) (bool true))`; `/tmp/luna-staged-closures-example.log`.
- `git diff --check`: passed.

The first test run reproduced the missing staged construction phase as
`lambda target requires a different execution phase`; fixed before the passing
regressions. No full suite or commit/push; changes remain on dev.

### Capture namespace correctness in staged substitution

Two regression cases reproduced a real capture bug: modifying a copied comptime
capture falsely invalidated the caller input, and default-input substitution
rewrote closure-owned reads to the original constant, yielding the wrong result.
The failing baseline is `/tmp/luna-staged-capture-before.log` (2 failures).

Forwarding now walks comptime capture namespaces explicitly: initializer reads
and writes belong to the parent environment, while captured bindings in the
body own independent storage. Mutation analysis excludes writes to those private
copies without excluding actual caller writes. Substitution masks capture and
parameter identities in staged closure bodies, substitutes capture initializers
in the parent frame, and keeps runtime closure forwarding's existing conservative
behavior. Defaults and frontend preparation now share bounded `prepare_input`
instead of separately rewriting closure bodies. Copies and nested scope walks
remain fuel/depth bounded.

Four new tests cover captured-copy mutation, repeated invocation state, concrete
and deferred default frames, nested/sibling capture independence, and continued
rejection after actual caller writes or address escape. The general comptime
regressions also preserve runtime closure forwarding and phase boundaries.

Validation in `/home/fin/luna`, final commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution staged_closure_capture_mutation`:
  4 new regressions; `/tmp/luna-staged-capture-scopes.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  115 execution + 26 elaboration tests; `/tmp/luna-staged-capture-regression.log`.
- `python3 -B development/cargo.py test -p iir`: 13 tests;
  `/tmp/luna-staged-capture-iir.log`.
- `python3 -B development/cargo.py test -p luna --test instantiation`: 18 tests;
  `/tmp/luna-staged-capture-instances.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-capture-copy.fl --interpret run`:
  `(tuple (bool true) (bool true))`; `/tmp/luna-staged-capture-example.log`.
- `git diff --check`: passed.

176 unique tests passed. No full suite or commit/push; changes remain on dev.
Final closure/object-graph instance inputs remain outside the supported subset.

### Staging integration and literal spread routing

A stage-integration pass ran the full luna execution/elaboration/instantiation
binaries after the recent shared substitution changes: 888 + 373 + 18 tests
passed (`/tmp/luna-staging-integration.log`). This run precedes the literal-spread
fix below, so it is not reported as a full-suite check of that later change.

A generic call with a literal tuple spread containing a staged comptime closure
then reproduced `lambda target requires a different execution phase`
(`/tmp/luna-staged-spread-probe.log`). The separate spread inference path had
checked the element without its destination's stage context and introduced a
runtime preparation temporary. When every expanded packet is a tuple literal,
spread_call now flattens its elements directly into ordinary argument routing.
That route already supplies both type and stage contexts, including generic
inference and method offsets. The previous literal spread path also flattened
such tuples without constructing packet storage; computed packet preparation is
unchanged. Width-overflow errors preserve the established spread diagnostic and
only remap errors anchored at the outer call.

Three new tests cover generic staged closure elements, mixed runtime/staged
source ordering, methods with defaults and runtime capture rejection. This does
not claim staged inputs extracted from arbitrary computed packets are complete;
that separate preparation path still needs stage-aware design and verification.

Validation in `/home/fin/luna`, final commands exit 0:
- Before the fix: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  1,279 integration tests; `/tmp/luna-staging-integration.log`.
- After the fix: `python3 -B development/cargo.py test -p luna --test execution --test elaboration spread`:
  38 execution + 7 elaboration tests; `/tmp/luna-staged-spread-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-literal-spreads.fl --interpret run`:
  `(tuple (bool false) (bool true))`; `/tmp/luna-staged-spread-example.log`.
- `git diff --check`: passed.

No repeat full suite, commit or push; changes remain on dev.

### Computed tuple spreads across staged and runtime inputs

Computed spread packets now retain the producer dependency chain needed by
staged call slots. NIR exposes a bounded preparation-dependency analysis shared
by phase validation, frontend availability checking and IIR materialization.
The existing bounded traversal moved from IIR to NIR; IIR re-exports it, so
consumers and traversal behavior retain the same API. No new IR node was added;
the selected computation uses existing structured expressions and their
S-expression Display.

Frontend call construction defers freezing while the outer spread preparation
is incomplete, then validates the selected computation as one staged input.
Packet contexts carry destination stage alongside type, including contextual
closures and generic inference attempts. Forwarding substitutes enclosing
comptime values into the selected preparation statements. Materialization
executes the whole selected computation once, replaces staged inputs, and
reconstructs only bindings still used by runtime siblings. Producers shared by
several staged arguments are selected once in source order. Intermediate values
remain inside the invocation rather than all becoming exported instance data.
The phase checker uses the same selected statement set; ordinary runtime
siblings are not silently promoted to comptime.

New cases cover all-staged and mixed-stage packets, generic/default inputs,
packet-local closures and explicit contextual closures, multiple packets with
method receivers, runtime sibling order, rejection of runtime producers/inputs,
and fuel failure followed by independent success. IIR tests verify one selected
producer occurrence, preservation of the runtime producer and statement order,
and removal of selection-only Type bindings at the preparation boundary.

A source probe involving an ordinary function's `comptime T: Type` parameter
exposed a pre-existing function-signature storage-phase restriction, before
packet handling. General support for that signature and Type residualization is
not claimed here. Exported packet values must still satisfy the existing frozen
value rules; arbitrary object/closure graphs are not yet exported by this path.

Validation in `/home/fin/luna`, final commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  894 execution + 375 elaboration + 18 instantiation tests;
  `/tmp/luna-staged-packet-integration.log`.
- `python3 -B development/cargo.py test -p luna --test execution computed_spreads_prepare_multiple`:
  1 subsequently added receiver/multi-packet test;
  `/tmp/luna-staged-packet-methods.log`.
- `python3 -B development/cargo.py test -p nir -p iir`:
  81 NIR + 14 IIR tests at that point; `/tmp/luna-staged-packet-ir.log`.
- `python3 -B development/cargo.py test -p iir --test staged_preparation`:
  2 preparation tests, including 1 additional Type-binding erasure case;
  `/tmp/luna-staged-packet-plan.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-computed-spreads.fl --interpret run`:
  `(tuple (tuple (bool false) (bool true)) (tuple (bool true) (bool false)))`;
  `/tmp/luna-staged-packet-example.log`.
- `git diff --check`: passed.

1,384 unique tests passed across the final checks. No commit/push; changes
remain in the dev worktree.

### Comptime-only inputs without runtime storage reconstruction

Runtime signature storage validation now inspects only runtime parameter slots
and the result. A staged Type/Integer input does not itself require runtime
storage. Instance binding now reconstructs only constants still referenced after
staged forwarding; closure initialization is distinguished from its private
capture namespace, so explicit completed captures do not falsely keep the
original caller binding alive.

This removes the signature/storage restriction reported in the preceding packet
work for non-dependent uses. Four new source tests cover unused Type/Integer
inputs, a 129-bit Integer key, forwarding, Type observation without unfolding
normal-form heads, defaults, computed packet inputs and Type captures inside a
staged closure. The latter inspects runtime IIR for absence of Type/Integer
storage. Runtime Type parameters, runtime Type results and runtime Integer
results remain rejected. Addressable or otherwise genuinely used runtime data
continues to receive restored bindings.

Dependent ordinary signatures such as
`fn identity(comptime T: Type, value: T) -> T { value }` still fail earlier in
type-expression elaboration (`type expression requires comptime elaboration not
yet implemented`), as recorded by `/tmp/luna-comptime-dependent-probe.log`.
This change does not claim that dependency/residualization work is complete.

Validation in `/home/fin/luna`, final commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  120 execution + 29 elaboration tests; `/tmp/luna-comptime-only-regression.log`.
- `python3 -B development/cargo.py test -p luna --test execution staged_`:
  18 tests including snapshot-budget and capture-scope regressions (overlaps the
  prior filter); `/tmp/luna-comptime-only-staging.log`.
- `python3 -B development/cargo.py test -p nir -p iir`:
  81 NIR + 15 IIR tests; `/tmp/luna-comptime-only-ir.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/comptime-type-inputs.fl --interpret run`:
  `(tuple (bool true) (bool false))`; `/tmp/luna-comptime-only-example.log`.
- `git diff --check`: passed.

No full-suite rerun or commit/push; changes remain on dev.

### Dependent staged IR scopes and default preparation

Staged input binding now rewrites the function's body annotations, program
contracts, defaults, trait requirements and type predicates together with its
signature. `Term::rebind_scope` substitutes and renumbers the current function
frame without removing it; nested function binders and more distant frames keep
their identities. Normal-form heads remain opaque. Defaults receive concrete
annotations before evaluation/substitution, including defaults delivered to the
remaining runtime call. Shared templates are unchanged on success or failure.

Preparation retains canonical symbols as each input completes. Later defaults
and instance keys reuse those symbols instead of repeatedly scanning prior
payloads. A budget regression supplies an 8 KiB string and 32 dependent default
arguments, checks concrete default types and exactly-once evaluation, and
finishes within 100,000 fuel. Rejection tests cover a wrong dependent input and
an evaluator failure followed by a successful invocation.

This is IR support, not completed source elaboration of
`fn identity(comptime T: Type, value: T) -> T`. The execution regression builds a
checked package and explicitly replaces its signature/body annotations with the
dependent telescope. Source type-expression lookup and dependent call checking
still need to introduce and instantiate ordinary Type parameter references;
the existing source probe rejects that syntax during elaboration.

Validation in `/home/fin/luna`, all commands exit 0:
- `python3 -B development/cargo.py test -p nir -p iir`: 82 NIR + 18 IIR tests;
  `/tmp/luna-dependent-scope-ir.log`.
- `python3 -B development/cargo.py test -p luna --test execution staged_`:
  19 tests, including dependent IR execution and the existing snapshot-budget
  regressions; `/tmp/luna-dependent-scope-execution.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration comptime`:
  120 execution + 29 elaboration tests (overlaps the staging filter);
  `/tmp/luna-dependent-scope-comptime.log`.
- `git diff --check`: passed.

Changes remain uncommitted on dev. No full-suite run.

### Source elaboration of ordinary dependent Type inputs

The source restriction from the preceding two entries is now removed for direct
ordinary comptime Type telescopes: `fn identity(comptime T: Type, value: T) -> T`
elaborates and executes. Resolved input identities map to slots in their own
function frame, independently of clause generics. Annotations can only use an
available earlier comptime Type input. Failed signature construction removes
its incomplete input mappings.

Direct calls substitute supplied type values into subsequent input types and
the result. Required computations that produce Type execute before checking the
dependent input, and their result is retained rather than executing that
computation again. Symbolic forwarding preserves the caller's frame. Defaults
keep their callee-local value bindings but acquire caller-scope type annotations
before substitution, including callers with differently ordered inputs.
Nested function types introduce a frame; closure checking opens that frame for
the body and restores it in the published callable type. PTC equality and
associated-type views receive a closed enclosing telescope, not free Bound
nodes. Opaque normal-form heads remain distinct from their definition bodies.

The comptime interpreter also substitutes Type inputs in argument/result
checks, body annotations and contracts for each invocation. Rewriting is
budgeted, leaves the shared declaration unchanged, and does not erase ordinary
value arguments. The source tests cover runtime invocation, generic-clause
forwarding into ordinary Type inputs, higher-order calls/lambdas, computed Type
arguments, multiple independent slots, nested results, staged defaults across
reordered frames, and required comptime invocation. Rejection tests cover wrong
value types, distinct unproved type inputs, runtime Type annotations and opaque
normal-form mismatches. Direct interpreter tests reject a bad input, then
successfully call the same declaration at Char and Bool without state leakage.

This does not complete all dependent elaboration. Runtime-dependent types and
value-dependent type computations remain separate work. The direct-call path
still needs a supplied type input before it can contextually check a dependent
argument; a general dependency scheduler for out-of-order/defaulted Type inputs
is not claimed. Arbitrary runtime defaults of an unknown T still require the
existing P3/resource evidence, rather than being accepted merely because a
later instance happens to choose Bool.

Validation in `/home/fin/luna`, commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  903 execution + 377 elaboration + 18 instantiation tests;
  `/tmp/luna-dependent-source-regression.log`.
- `python3 -B development/cargo.py test -p comptime_interpretor`:
  59 tests; `/tmp/luna-dependent-interpreter.log`.
- After adding generic-clause forwarding to the source fixture,
  `python3 -B development/cargo.py test -p luna --test execution dependent_comptime_type_inputs`:
  4 tests; `/tmp/luna-dependent-source-final.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/dependent-comptime-inputs.fl --interpret run`:
  `(tuple (bool true) (char 120) (bool false))`;
  `/tmp/luna-dependent-source-example.log`.
- `git diff --check`: passed.

Changes remain on dev, uncommitted.


### Mixed ordinary Type inputs and clause inference

Ordinary call dependency preparation is shared by direct and clause-generic
calls. Inference substitutes available ordinary Type inputs before abstracting
clause variables, while the emitted callee keeps its original dependent
signature. This supports concrete Type inputs, forwarding clause rigids,
Type defaults coupled to clause variables, and explicit arguments with optional
clause defaults. Runtime inputs retain their source order.

Runtime materialization now selects staged trait method bodies before applying
ordinary input specialization. Interface contracts remain retained. Interface
default expressions are rebound to implementation parameter identities, with
closure capture initialization handled separately from closure-owned bindings.
Tests cover inherent/trait methods, different implementation parameter names,
staged defaults, captured defaults, and once-only runtime arguments/defaults.

Validation in `/home/fin/luna`:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`
  passed 381 elaboration and 919 execution cases, then failed one newly added
  fixture because a runtime closure-call default lacked P3 evidence. Log:
  `/tmp/luna-mixed-dependent-regression.log`. The fixture was corrected to use
  comptime inputs/default evaluation, without relaxing language checks.
- `python3 -B development/cargo.py test -p luna --test execution staged_trait_calls_`
  passed the corrected captured-default case and an additional runtime-default
  evaluation-order case (2 tests, exit 0); `/tmp/luna-staged-trait-tests.log`.
- `python3 -B development/cargo.py test -p luna --test instantiation`
  passed 18 tests, exit 0; `/tmp/luna-mixed-dependent-instantiation.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/mixed-dependent-generics.fl --interpret run`
  returned `(tuple (tuple (bool true) (char 120)) (tuple (char 121) (bool false)) (tuple (bool false) (char 122)))`,
  exit 0; `/tmp/luna-mixed-dependent-example.log`.
- `git diff --check`: passed.

Remaining boundary, confirmed by `/tmp/luna-mixed-outer-input.fl`: forwarding
an enclosing ordinary `comptime T: Type` into a clause-generic call still fails
with `Shape(MissingArgument(0))`. Unlike a declaration rigid, this is a caller
telescope binding; inference needs explicit scope handling. No invented global
type identity or relaxed scope validation has been used to hide this gap.
Computed Type defaults depending on unresolved clause inputs also need further
scheduling work. Changes remain on dev, uncommitted.

### Dependency-directed preparation of ordinary Type defaults

Direct ordinary calls now prepare the compile-time dependencies needed for
input/result checking before constructing the runtime argument plan. This
replaces the preceding entry's requirement that a Type input already appear
before its dependent argument in source order. Missing optional Type inputs,
late named Type arguments, defaults computed from earlier comptime configuration
values, and chains of default Type inputs all supply contextual information.
Prepared expressions and canonical symbols are reused in the eventual call;
the runtime argument vector remains in source order, followed by missing
runtime/default inputs in declaration order.

`program/dependent_inputs.rs` owns the bounded per-call dependency state and
shared default instantiation. Dependency discovery includes internal default
annotations, not just published parameter/result types. Default value reads
resolve against declaration parameter identities. A dependency on a runtime
input is rejected instead of promoting it to comptime. Explicit overrides skip
their default expression, including a deliberately nonterminating Type default.
Symbolic Type defaults can forward the caller's bound input or clause generic
without forcing an unavailable invocation. Active dependency state is local to
the call and removed on failure.

Three execution tests cover configuration-derived Type defaults, late named
arguments, chained and symbolic forwarding, internal callable annotations,
source runtime order, and overridden-default suppression. A rejection group
covers runtime dependencies, incompatible values and required computation
failure. The direct-call scheduler does not yet promise residualization of an
arbitrary Type computation whose inputs are still symbolic, nor complete
integration of dependent inputs with every generic/packet inference protocol.

Validation in `/home/fin/luna`, all commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration dependent_type_`:
  3 execution + 1 elaboration tests; `/tmp/luna-dependent-default-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  906 execution + 378 elaboration + 18 instantiation tests;
  `/tmp/luna-dependent-default-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/dependent-default-inputs.fl --interpret run`:
  `(tuple (bool true) (char 120) (char 122))`;
  `/tmp/luna-dependent-default-example.log`.
- `git diff --check`: passed.

Changes remain uncommitted on dev.

### Dependent ordinary Type inputs supplied by computed packets

Computed `...packet` inputs can now supply the Type values required to check
later arguments in the same or another packet. Temporary packet bindings retain
a call-local link to their checked producer. Dependency preparation requests a
field's symbolic value, freezes the producer once, and reuses the same completed
tuple for other requested fields. Before publishing the call preparation, the
original producer is replaced by that frozen tuple; its source computation is
not emitted again for runtime execution. Pending producer entries are removed
when preparation succeeds or fails. Prechecked scalar spread inputs use the
same mechanism.

A callee's open telescope is not passed as a standalone expected tuple type.
Packets whose contextual type still has free binders are checked from their
own expression type, then each field is checked against the instantiated call
slot. This also preserves the enclosing frame for spreading runtime tuples
into higher-order callbacks. Runtime siblings keep their source order, and a
runtime packet accompanying a separately supplied Type still executes once at
runtime. A staged packet field can supply a scalar configuration dependency
for a default Type computation, not only a Type value directly.

Six execution regressions cover direct packets, multiple Type fields, multiple
packets, runtime sibling order and a once-only runtime producer, receiver calls,
staged defaults, enclosing higher-order frames and packet-derived Type-default
configuration. Rejection cases check inconsistent Type/value fields, a runtime
producer for a required comptime packet, and preservation of opaque normal-form
heads. Runtime graph inspection confirms the frozen generators are absent.

This does not complete dependent packet inference: producers must currently
expose an independently checkable tuple type when their expected type is open.
Arbitrary symbolic producer invocations and exported closure/object graphs
still require the broader residualization/materialization work. No change to
the language's contextual-typing contract is intended by these implementation
limits.

Validation in `/home/fin/luna`, commands exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration spread`:
  43 execution + 8 elaboration tests; `/tmp/luna-dependent-packet-spreads.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration dependent_`:
  32 execution + 12 elaboration tests; `/tmp/luna-dependent-packet-regression.log`.
- After adding the opaque-head rejection,
  `python3 -B development/cargo.py test -p luna --test elaboration dependent_type_packets`:
  1 test; `/tmp/luna-dependent-packet-rejections.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/dependent-packet-inputs.fl --interpret run`:
  `(tuple (tuple (bool true) (char 120)) (tuple (bool false) (bool true) (char 121)))`;
  `/tmp/luna-dependent-packet-example.log`.
- `git diff --check`: passed.

The filters overlap; no full-suite rerun. Changes remain uncommitted on dev.

### Dependent comptime value parameters instantiated at Type

Fixed `fn discard(comptime T: Type, comptime value: T) -> bool { true }`
called as `discard(Type, bool)`. Previously the source checker admitted the
call, but staged binding rejected the Type value as the wrong semantic sort.
The callable telescope now determines each input's domain after substituting
its preceding inputs. A comptime-value slot whose type becomes exactly Type
acquires the Type domain. Runtime slots do not acquire that permission, and an
opaque normal-form head is not treated as Type by inspecting its definition.

`nir::FunctionInputBinding` records the checked signature, replacement frame
and refined slots. Its immutable substitution is shared by signature/body,
default, trait-requirement and type-predicate rebinding. Partial specialization
updates retained input domains and their references, so binding T first and
value later agrees with binding both together. Nested binder identities remain
separate. General `substitute`/`rebind_scope` still enforce their original sort
checks; only the checked callable binding plan authorizes this refinement. The
plan has a deterministic S-expression Display, including escaped values.
`instantiate_function` performs all shape/domain checks before contract
callbacks and preserves the same refinement when removing the call frame.

IIR staging and the comptime interpreter use the checked plan. The interpreter
builds a new plan when a Type-valued input arrives, closes dependent annotation
checks, and keeps invocation state separate from the source template. Source
regressions cover default values, forwarding, clause-generic inputs, computed
packets, required invocation and failure/recovery. Runtime graph keys keep
Bool, Char and Identity<Bool> distinct. Runtime Type storage remains rejected.
Core regressions additionally cover partial/full composition, nested binders,
opaque heads, early rejection before callback side effects, body/contracts and
Display escaping. This is domain specialization for available Type values,
not general residualization of unresolved dependent value computations.

Validation in `/home/fin/luna`, commands exit 0:
- `python3 -B development/cargo.py test -p nir -p iir -p comptime_interpretor`:
  83 NIR + 19 IIR + 59 interpreter tests; `/tmp/luna-input-domain-kernels.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  914 execution + 380 elaboration + 18 instantiation tests;
  `/tmp/luna-input-domain-regression.log`.
- After strengthening opaque-head and Display checks,
  `python3 -B development/cargo.py test -p nir --test calls`: 7 tests;
  `/tmp/luna-input-domain-call-checks.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/dependent-type-values.fl --interpret run`:
  `(tuple (bool true) (bool true) (bool true))`;
  `/tmp/luna-input-domain-example.log`.
- `git diff --check`: passed.

Changes remain on dev, uncommitted.

### Caller-scoped generic inference and declaration substitution

The ordinary-Type forwarding limitation recorded under mixed generic inputs is
resolved for direct and trait calls, including cases where a clause argument
itself is inferred as an enclosing ordinary Type input. Explicit instantiation
and function values also retain the caller telescope.

Call inference now carries a validated caller telescope alongside the separate
clause substitution frame. Matching may extract references to that caller but
cannot extract a callback's own bound inputs. Stable equalities involving local
Type references are closed under the caller telescope before PTC sees them;
closed equalities retain the ordinary path. No local Type is replaced by an
invented global declaration. Candidate scope appears in the S-expression dump.
Settled callable contexts no longer remove an additional, absent inference frame.

NIR `instantiate_types_in_scope` validates substitution inputs against explicit
frames and preserves local-versus-caller depths through declaration abstraction
and substitution. The existing closed-input API remains strict. Source generic
calls, explicit function instances, trait calls and phase contract validation
use the scoped API where the caller's inputs are available. Canonical runtime
instance keys are still formed only after ordinary Type inputs become concrete.

Tests cover inference/result contexts, explicit/function-value uses, callback
scope escape rejection, wrong sorts/depths, failure recovery, distinct caller
Type inputs, trait calls, scope dumps and substitution composition. Example:
`development/examples/scoped-generic-inputs.fl`.

Validation in `/home/fin/luna`, successful commands exit 0:
- `python3 -B development/cargo.py test -p nir`: 84 tests;
  `/tmp/luna-scoped-nir-tests.log`.
- `python3 -B development/cargo.py test -p elaboration`: 311 tests at the initial
  integration point; `/tmp/luna-scoped-inference-kernels.log`.
- After the closed-equality fast path,
  `python3 -B development/cargo.py test -p elaboration --test calls --test obligations --test matching --test effect_inference`:
  45 tests; `/tmp/luna-scoped-inference-final.log`.
- After strengthening settled callback context handling,
  `python3 -B development/cargo.py test -p elaboration --test calls`:
  16 tests; `/tmp/luna-scoped-call-context-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  923 execution + 382 elaboration + 18 instantiation tests;
  `/tmp/luna-scoped-source-regression.log`.
- After explicit function-instance and callback-context changes,
  `python3 -B development/cargo.py test -p luna --test execution --test elaboration generic`:
  166 execution + 47 elaboration tests, including the new explicit function-value
  case; `/tmp/luna-scoped-generic-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/scoped-generic-inputs.fl --interpret run`:
  `(tuple (tuple (char 120) (char 120)) (tuple (bool true) (bool true)))`;
  `/tmp/luna-scoped-example.log`.
- `git diff --check`: passed.

This establishes caller-scoped Type substitution, not general symbolic execution
of arbitrary dependent computations or complete support for all scoped trait
requirements. Changes remain on dev, uncommitted.

### Infer independent inputs before dependent Type defaults

Mixed calls can now compute an ordinary Type default such as `Choose(G)` after
independent explicit inputs or result context determine clause parameter G.
Previously dependency preparation attempted invocation while G was unresolved.

An initial candidate checks only inputs whose types do not depend on ordinary
call inputs. Provisional Type slots allow the expected result to constrain
clause inputs without pretending the ordinary defaults have already completed.
Only clause substitutions whose collected relations are settled enter default
preparation. The final candidate checks the complete original signature after
actual Type defaults are computed. Defaults therefore cannot be overridden by
provisional result-context choices. Checked explicit expressions are cached and
reused; final runtime routing retains source order and once-only evaluation.
Effect-value preservation and handler discharge use the existing call rules.

Tests cover bool/char selection, numeric argument context from the result,
computed-type mismatch, failing required invocation, and side-effecting runtime
inputs checked in a different order from their execution. Example:
`development/examples/inferred-dependent-defaults.fl`.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  926 execution + 383 elaboration + 18 instantiation tests;
  `/tmp/luna-dependent-default-scheduling-regression.log`.
- After preserving effect values in the preliminary check,
  `python3 -B development/cargo.py test -p luna --test execution --test elaboration dependent_`:
  43 execution + 15 elaboration tests;
  `/tmp/luna-dependent-default-scheduling-final.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/inferred-dependent-defaults.fl --interpret run`:
  `(tuple (tuple (bool true) (bool false)) (tuple (char 120) (unsigned-integer 9)))`;
  `/tmp/luna-inferred-dependent-defaults-example.log`.
- `git diff --check`: passed.

Remaining: a computed clause default such as `.B:Type=Choose(A)` with A inferred
as an enclosing ordinary Type input still fails (`invalid binder reference 0:0`,
`/tmp/luna-scoped-computed-default.log`). Carrying a scoped Type through inference
is not sufficient to authorize arbitrary comptime computation over an unknown
Type. Deferred invocation and default-elaboration context need further work;
no symbolic Type has been treated as a completed concrete value to mask this.
Changes remain on dev, uncommitted.

### Resume independent inference before invoking Type defaults

Independent-input preparation now reaches a local fixed point rather than
checking each input against a single initial context. Each round refines
associated type views and retries inputs with newly available full/partial
callable context. Numeric defaulting runs only after contextual progress stops,
and only one successful default is applied before rebuilding constraints.
Completed expressions remain cached and the original call routing is retained.
The loop consumes the shared checking budget.

An ordinary Type computation that still contains the callee's unresolved clause
inputs now returns a context-dependent wait diagnostic before invocation. This
allows enclosing generic calls to retry it with numeric/contextual information.
Structural opaque heads remain eligible for symbolic construction; this is not
permission to execute arbitrary Type computations on unknown symbolic inputs.

Regressions cover an unannotated nested generic call with an identity lambda and
integer literal, later callable evidence selecting u64 before an otherwise
out-of-range i32 literal is defaulted, and a permanently unresolved callback
reporting a missing inference dependency rather than an interpreter Type error.
The `inferred` entry in `development/examples/inferred-dependent-defaults.fl`
exercises the no-result-annotation path.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration dependent_`:
  45 execution + 15 elaboration tests; `/tmp/luna-dependent-fixed-point-regression.log`.
- After adding the rejection test,
  `python3 -B development/cargo.py test -p luna --test elaboration dependent_type_defaults_report`:
  1 test; `/tmp/luna-default-wait-diagnostic.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/inferred-dependent-defaults.fl --interpret inferred`:
  `(bool true)`; `/tmp/luna-inferred-numeric-default-example.log`.
- `git diff --check`: passed.

The previously recorded scoped computed-clause-default limitation remains.
Changes remain on dev, uncommitted.

### Scoped structural ordinary defaults

Ordinary parameter defaults now accept clause Type arguments that refer to the
caller's ordinary Type inputs. `call_default` validates those arguments in the
caller scope, lifts them across the callee's parameter frame, and substitutes
into default annotations with both frames present. Subsequent dependency
preparation removes the callee frame using actual/defaulted ordinary inputs.
Closed arguments retain the existing substitution path.

This fixes `pair(comptime .T:Type=G,value:T,other:G)` called from
`forward(comptime A:Type,value:A)` with G inferred as A. Tests also cover staged
value/default chains, trait methods, nested callable types in defaults, and
rejection when different caller Type inputs would otherwise be captured by a
same-numbered callee binding. The `defaults` entry in
`development/examples/scoped-generic-inputs.fl` demonstrates the source path.
This change constructs structural Type values; arbitrary computed defaults on
unknown Types still require the deferred invocation work recorded above.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration scoped_structural_`:
  3 execution + 1 rejection tests; `/tmp/luna-scoped-default-scope-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration default`:
  154 execution + 38 elaboration tests (includes the new tests);
  `/tmp/luna-scoped-default-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/scoped-generic-inputs.fl --interpret defaults`:
  `(tuple (bool true) (bool true))`; `/tmp/luna-scoped-default-example.log`.
- `git diff --check`: passed.

Changes remain on dev, uncommitted.

### Ordinary comptime value indices and deferred normal-form construction

Ordinary comptime value parameters now retain a checked bound identity and input
domain, separately from ordinary Type parameters. Type annotations can use an
earlier Integer/bool input in an opaque normal-form head, and calls can forward
that value binding without attempting to interpret it prematurely. Failed
signature preparation removes its registered value inputs. Runtime parameters
and domain-mismatched values remain rejected.

Structural normal-input recovery can recognize these live ordinary bindings.
Staged input availability traversal now includes normal-form arguments, input
plans and embedded patterns. When an outer staged value is not yet concrete,
the constructor remains deferred. After specialization, normal-form construction
uses the existing required-invocation VM path, retaining the opaque head and
canonical input identities. Normal-form default arithmetic such as storage=N+1
is evaluated only after N arrives. Unbounded Integer arithmetic can likewise be
retained until its staged inputs become available, then runs through the VM
rather than the fixed-width folding path.

Source tests verify indexed signatures and forwarding, rejection of runtime and
wrong-domain inputs, distinct scalar-indexed instance keys, normal-form default
plans computing 5/6 from 4/5, and a staged sum producing the exact 129-bit value
2^128 without modifying its source template. Example:
`development/examples/dependent-value-inputs.fl`.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  933 execution + 386 elaboration + 18 instantiation tests;
  `/tmp/luna-dependent-values-regression.log`.
- After adding the wide-integer source regression,
  `python3 -B development/cargo.py test -p luna --test execution staged_unbounded_integer_`:
  1 test; `/tmp/luna-staged-unbounded-integer.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/dependent-value-inputs.fl --interpret run`:
  `(tuple (bool true) (bool true))`; `/tmp/luna-dependent-values-example.log`.
- `git diff --check`: passed.

This is support for bound static indices and staged construction plans, not
arbitrary symbolic arithmetic in unresolved function signature types or runtime
execution of opaque types with no concrete representation. Source Integer
division still requires the separate rounding-protocol implementation; this
change does not choose one. Changes remain on dev, uncommitted.

### Active signature prefixes for normal-form default plans

Signature checking now exposes earlier ordinary comptime parameters as typed
expression inputs to normal-form construction plans. It does not publish runtime
parameters or later declarations. Structural recovery recognizes their bound
identities while the enclosing signature is still being formed.

The checker retains each active signature's checked parameter prefix and removes
it on completion/failure. Call-scope lookup uses this actual prefix when a full
signature is not yet available. Type-clause defaults capture the caller scope
before entering their definition context, then use checked scoped substitution.
This enables `Shape<n, .storage=n>` and structural `Shape<n>` defaults in parameter
and result annotations, as well as ordinary Type inputs in `Wrapper<A>` defaults.
Opaque heads and binding depths remain unchanged.

Tests cover explicit/defaulted inputs and forwarding, rejected later/runtime
inputs and wrong domains, a Type default chain using caller parameter index 2,
and runtime specialization producing distinct Wrapper<bool>/Wrapper<char> keys
with all three head inputs equal to the selected Type. The dependent-value
example includes the named-input signature form.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  935 execution + 388 elaboration + 18 instantiation tests;
  `/tmp/luna-signature-prefix-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/dependent-value-inputs.fl --interpret run`:
  `(tuple (bool true) (bool true))`; `/tmp/luna-signature-prefix-example.log`.
- `git diff --check`: passed.

Arbitrary computations on unresolved inputs still need deferred elaboration;
this change does not evaluate a symbolic Type or integer as a concrete value.
Changes remain on dev, uncommitted.

### Mixed ordinary Type/value indices with dependent head defaults

Normal-form input domain substitution now uses the explicit caller scope. A head
such as `Tagged<T,value:T>` can consume both an ordinary Type binding and an
ordinary comptime value of that Type in a function signature. Scoped type
requirements use the active checked parameter prefix while the signature is
still under construction.

For mixed normal-form input/default plans, the checker retains the caller's
input scope while resolving names and premises in the head's definition context.
The temporary scope is restored on success or error. This enables `.copy:T=value`
without confusing declaration parameters with caller bindings. Active signature
prefixes take precedence while independently checking a declaration.

Tests cover signature forwarding and exact mixed-sort bound arguments, rejected
cross-Type domains and runtime indices, and concrete specialization with bool,
Type and tuple inputs. Runtime keys retain the opaque head with the canonical
Type/value/copied-value arguments; the Type-valued case uses the established
domain-refinement rules. Example entry: `mixed` in
`development/examples/dependent-value-inputs.fl`.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  936 execution + 389 elaboration + 18 instantiation tests;
  `/tmp/luna-mixed-value-domain-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/dependent-value-inputs.fl --interpret mixed`:
  `(tuple (bool true) (bool true))`; `/tmp/luna-mixed-value-domain-example.log`.
- `git diff --check`: passed.

Arbitrary computations over unresolved inputs remain outside this structural
substitution support. Changes remain on dev, uncommitted.

### Contextual generic function values in caller input scopes

Generic function values now infer clause arguments in the caller's ordinary
input scope. Full expected function types have their parameter frame removed
before supplying inference components; partial callable contexts are already
caller-relative. Template and final argument substitution use checked scoped
instantiation, including generic trait method values.

Regression cases cover implicit identity instantiation under an ordinary Type
parameter, nested function-valued results, trait method values, and callback
inference from a later argument with both bool and char specializations. A
negative case prevents fn(A)->B from equating distinct enclosing Type inputs.
The scoped-generic-inputs example adds a `callbacks` entry combining nested
function values and later-argument inference.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration generic`:
  169 execution + 48 elaboration tests; `/tmp/luna-contextual-generics-regression.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration context`:
  74 execution + 18 elaboration tests (overlap with the generic selection);
  `/tmp/luna-contextual-values-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/scoped-generic-inputs.fl --interpret callbacks`:
  `(tuple (bool true) (char 120))`; `/tmp/luna-contextual-values-example.log`.
- `git diff --check`: passed.

This does not implement arbitrary computation over unresolved type inputs or
dependent callbacks whose own argument bindings escape into inference. Changes
remain on dev, uncommitted.

### Computed clause defaults with scoped structural components

Settling pending function clause defaults now substitutes inferred dependencies
using the caller's checked input scope. Previously `(G, Choose(H))` failed with
an invalid binder reference when G was an enclosing ordinary Type input even
though H was closed and Choose(H) could run. The structural component now retains
its binding while the independent comptime invocation computes its result.

Execution tests cover both direct calls and contextual function values, ordinary
structural defaults, and bool/char specializations. The computed default is used
as an actual parameter and result type. A rejection test supplies a different
outer binding through the callback context and verifies that the default remains
a constraint. The example adds the `computed_defaults` entry.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration default`:
  160 execution + 40 elaboration tests; `/tmp/luna-computed-scoped-defaults-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/scoped-generic-inputs.fl --interpret computed_defaults`:
  `(tuple (char 120) (bool true))`; `/tmp/luna-computed-scoped-defaults-example.log`.
- `git diff --check`: passed.

This does not execute arbitrary comptime computations on unresolved outer Type
values. Changes remain on dev, uncommitted.

### Trait evidence at generic callback boundaries

Added execution coverage for parent-trait evidence passing through contextual
generic function values and later-argument callback inference. Both declaration
sugar (`A :- Enabled`) and explicit `requires A :- Enabled` are exercised, and
the callback dispatches the parent trait's method at runtime. Rejection coverage
checks that a function type context cannot supply missing trait evidence for an
otherwise unconstrained A, for either a local function value or callback input.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration generic_callback`:
  3 execution + 1 elaboration tests; `/tmp/luna-trait-callback-tests.log`.
- `git diff --check`: passed.

A separate probe exposed an unimplemented contract boundary:
`fn forward(comptime A:Type,value:A)->A where requires A :- Allowed` fails
with `type annotation requires an earlier comptime Type parameter` while
collecting signature capabilities, before its ordinary input bindings exist.
Probe: `/tmp/luna-scoped-trait-callback.fl`, output
`/tmp/luna-scoped-trait-callback.log`, command
`python3 -B development/cargo.py run -p luna -- /tmp/luna-scoped-trait-callback.fl --interpret run`,
exit 1. Moving the collector alone is insufficient: capability/requirement
instantiation and implementation applicability still assume closed terms.
Ordinary-input contracts need a scoped representation and specialization path
through all of these stages. The passing callback tests above use clause-bound
type variables and do not establish support for this ordinary-input case.
Changes remain on dev, uncommitted.

### Scoped declaration substitution for type contracts

NIR now exposes `instantiate_parameters_in_scope` for Term, TraitRequirement
and TypePredicate. It takes declared parameter sorts and caller sort frames,
validates argument arity/sorts/scope and resulting scope, and preserves logical
structure, trait identity and opaque heads. The existing Type-only scoped API
delegates to this mixed-sort primitive. Closed APIs remain strict.

Tests exercise mixed Type/comptime-value substitutions under nested function
binders, trait arguments, equality/not/and/or propositions, partial substitution
composition, unused invalid inputs, duplicate declarations, invalid output scope
and incorrectly sorted rigid occurrences. Dumps retain existing S-expression
structure and normal-form identity.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p nir --test scoped_contracts --test types`:
  3 scoped-contract + 51 type tests; `/tmp/luna-scoped-contracts.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration scoped_`:
  9 execution + 6 elaboration tests; `/tmp/luna-scoped-contracts-integration.log`.
- `git diff --check`: passed.

This supplies the checked substitution prerequisite for ordinary-input trait
contracts; it does not yet connect their signature collection, evidence scope
or specialization to trait selection. The ordinary-input probe documented above
remains unsupported. Changes remain on dev, uncommitted.

### Scoped lookup of existing trait evidence

`World::given_in_scope` validates the ordinary parameter telescope and type
subjects/arguments, then follows parent and parameter-capability edges with
scoped declaration substitution. Returned evidence retains the originating
assumption. Type equality queries with open operands carry the caller telescope;
closed operands retain the existing context-equality path. Invalid subjects,
invalid scopes, missing evidence and exhausted budgets cannot succeed. The closed
entry point also rejects malformed/open inputs rather than accepting an unchecked
syntactically identical assumption.

NIR InterfaceInstance now supports checked scoped declaration substitution.
Elaboration's symbolic interface evidence lookup uses the caller scope. This
connects the evidence lookup prerequisite, but ordinary-input signature contract
collection and implementation selection still need completion; the original
source probe is not yet supported.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p elaboration --test capabilities`:
  26 tests; `/tmp/luna-scoped-evidence.log`. New cases cover parent traversal,
  parameter edges and cycles, wrong binding identities, budget exhaustion,
  invalid scopes, assumption isolation and closed type equality.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration trait`:
  76 execution + 42 elaboration tests; `/tmp/luna-scoped-evidence-integration.log`.
- After preserving the closed equality fast path,
  `python3 -B development/cargo.py test -p luna --test execution --test elaboration generic_callback`:
  3 execution + 1 elaboration tests; `/tmp/luna-scoped-evidence-callbacks.log`.
- `git diff --check`: passed.

Changes remain on dev, uncommitted.

### Scoped capability member sources and inherited lookup

Member lookup now uses `capability_sources_in_scope` with the caller's sort
frames, and inherited-member traversal uses scoped InterfaceInstance
substitution. Parent interfaces remain beneath the child lookup root, preserving
child-member shadowing. Parameter capability edges retain their own subjects and
arguments. Invalid scopes, invalid inherited substitutions and exhausted lookup
budgets return errors instead of exposing an incomplete list of member sources.
The closed capability_sources entry point delegates with an empty scope.

Extended solver tests verify caller-bound parameter edges through parents,
child lookup roots, cyclic edges, missing caller slots, closed API rejection,
lookup budget failure and a malformed inherited edge that must not silently
remove a competing source.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p elaboration --test capabilities`:
  26 tests; `/tmp/luna-scoped-member-sources.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration trait`:
  76 execution + 42 elaboration tests; `/tmp/luna-scoped-member-integration.log`.
- `git diff --check`: passed.

Ordinary-input contract collection is still incomplete. Inspection also found
that ensure_contract's current mentions_value_parameter classifier routes every
ResolvedFnParam into program preconditions, even an ordinary comptime Type used
as the subject of a type proposition. Signature collection, this classification,
and deferred checking of callable-frame obligations must be connected together;
simply moving positive-bound collection after the parameters is insufficient.
Runtime materialization already rebinds staged function capabilities and checks
them, but the pre-specialization checking paths remain closed. Changes remain
on dev, uncommitted.

### Ordinary Type-input positive capability contracts

Ordinary comptime Type parameters now support positive trait requirements such
as `fn forward(comptime A:Type,value:A)->A where requires A :- Allowed`.
Signature capability collection retains the early clause-only pass and collects
ordinary-input bounds after their parameter bindings exist. Contract
classification distinguishes registered Type inputs from program value inputs,
and contract type lookup can read those bindings in their own declaration scope.

Clause application retains obligations that refer to the callee's ordinary
frame, after validating that frame; it does not mistake it for the caller frame.
Call input preparation includes dependencies occurring only in capabilities.
Both direct and generic call completion substitute inferred clause arguments
across the callee frame, supply actual ordinary Type inputs, and check each
positive bound against scoped evidence or the concrete implementation world.
The original obligations remain in NIR for staged rebinding and verification.
Generic callback capability substitution uses caller scope, allowing an ordinary
Type-input bound to justify a clause-generic callback.

Execution coverage includes the original failing callback example, generic
forwarding with an additional inferred clause parameter, and two Type inputs
used only by a parameterized capability contract. Rejection coverage checks
wrong actual types and an unconstrained outer input forwarded to a constrained
callee. Example: `development/examples/ordinary-type-capabilities.fl`.

Validation in `/home/fin/luna`, exit 0 unless stated:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  944 execution + 393 elaboration + 18 instantiation tests;
  `/tmp/luna-ordinary-trait-regression.log`.
- After strengthening validation of retained callee-frame obligations,
  `python3 -B development/cargo.py test -p luna --test execution --test elaboration ordinary_type_input`:
  3 execution + 1 elaboration tests; `/tmp/luna-ordinary-trait-tests.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/ordinary-type-capabilities.fl --interpret run`:
  `(tuple (bool true) (bool true))`; `/tmp/luna-ordinary-trait-example.log`.
- Indirect-erasure probe `/tmp/luna-ordinary-trait-erasure.fl`, run with the same
  driver and `--interpret run`, exit 1: `staged runtime function requires a
  specialized direct call`. Output: `/tmp/luna-ordinary-trait-erasure.log`.
- `git diff --check`: passed.

This does not yet support all ordinary-input propositions: negative/disjunctive
trait predicates and equality premises need scoped proposition handling;
associated types required while forming an earlier parameter need interleaved
bound collection rather than the late positive pass. Method contract conformance
with ordinary-input requirements and indirect staged function values remain
separate work. Changes remain on dev, uncommitted.

### Ordinary Type-input bounds during associated signature formation

Signature collection now separates explicit positive capability atoms from
conjunctions and publishes each once when its referenced ordinary inputs are
available. It revisits pending atoms between parameters and performs a final
checked pass before the result. Negation and disjunction never supply an atom.
The readiness walk handles structural types, applications and nested ordinary
references; unsupported expression shapes wait for normal final elaboration.
This enables separate A/B bounds to become available before their respective
associated-type value parameters, without exposing later parameters early.

Associated projection recognizes ordinary Type parameters locally, preserving
normal value-expression capture semantics elsewhere. Member type substitution
uses caller scope. Definition-side associated views track callable frames and
retain well-formed input-dependent projections without selecting an
implementation or asserting an equality. Invalid origins, members and member
sorts still fail; strict normalization still rejects unresolved projections.
Call checking resolves parameter/result views after ordinary inputs are supplied.
The callable view uses active signature prefixes as well as completed scopes.

Materialization now forms a projection view before staging, substitutes ordinary
inputs, then strictly normalizes resulting types. A separate staging-view cache
avoids repeatedly spending selection budgets; it is not shared with the strict
completed cache. Open projections are not cached. Post-staging normalization
preserves canonical non-Type instance-key values. The first broad regression
caught that distinction and a closure capture regression from globally treating
ordinary Type parameters as type heads; both were fixed before final validation.

Tests cover interleaved ordinary input bindings, inherited Source evidence,
associated parameters/results, forwarding, clause inference, missing evidence,
later-parameter rejection and wrong concrete argument types. View tests cover
open callable preservation, strict rejection, invalid members/sorts and cache
isolation/failure. Example entry `associated` in ordinary-type-capabilities.fl.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  946 execution + 394 elaboration + 18 instantiation tests;
  `/tmp/luna-associated-signature-regression.log`.
- `python3 -B development/cargo.py test -p elaboration --lib associated`:
  3 tests; `/tmp/luna-associated-view-unit.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/ordinary-type-capabilities.fl --interpret associated`:
  `(tuple (bool true) (char 120))`; `/tmp/luna-associated-signature-example.log`.
- `git diff --check`: passed.

Negative/disjunctive ordinary-input predicates, equality premises and general
method-contract conformance remain unfinished. This is not completion of the
full dependent-contract system. Changes remain on dev, uncommitted.

### Associated-type views in staged input/default binding

IIR staged call binding now accepts a shared evaluation/type-view context via
`bind_call_with_type_views`; the existing entry retains its identity-view
behavior. Materialization supplies its budgeted associated-type view. Supplied
values are still checked against their parameter domains, but the domains are
resolved after binding ordinary inputs. Rebound body/default/contract expression
annotations use the same view before forwarded local values are substituted.
Annotations retaining ordinary references are viewed under their residual
callable telescope. No value/type comparison was removed.

This fixes comptime values and defaults with types such as A.as(Source).Item:
Bool/Char values can be forwarded, defaulted from an earlier value and explicitly
overridden without comparing them against an unresolved projection. Tests cover
both implementations and reject a Char override for the Bool instance. Runtime
defaults still require their existing P3 evidence; this does not relax that rule.
Example entry `defaults` in ordinary-type-capabilities.fl.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p iir --test dependent_staging --test staged_preparation`:
  4 + 2 tests; `/tmp/luna-staging-views-iir.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  947 execution + 394 elaboration + 18 instantiation tests;
  `/tmp/luna-associated-default-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/ordinary-type-capabilities.fl --interpret defaults`:
  `(tuple (bool true) (bool false) (char 120))`; `/tmp/luna-associated-default-example.log`.
- `git diff --check`: passed.

A next boundary was verified with `/tmp/luna-associated-type-domain.fl`: when
Source.Item itself resolves to Type, the binding plan still classifies the input
as comptime-value before normalizing its domain. Running that probe through the
driver with `--interpret run` exits 1, `expected comptime-value, got type`
(`/tmp/luna-associated-type-domain.log`). The checked FunctionInputBinding plan
needs domain views before sort refinement, not only during later value checks.
Changes remain on dev, uncommitted.

### Domain views before ordinary input sort refinement

FunctionInputBinding now offers checked parameter-domain views before choosing
an input's semantic sort. Each domain is substituted using earlier inputs,
resolved under the retained parameter prefix, then checked for Type sort and
scope. Only a comptime-value parameter whose resolved domain is exactly Type is
promoted. The existing binding API retains its identity view; opaque normal-form
heads do not become Type by unfolding. Rebinding and residual signatures share
the resulting sort-refinement plan.

IIR uses this plan while preparing defaults, binding supplied inputs and routing
remaining runtime arguments. Materialization's existing associated view supplies
the domain resolution and budget accounting. This closes the previous
`expected comptime-value, got type` probe for Source.Item = Type. Frontend default
annotations are also resolved after substitution, before eager staged freezing;
otherwise direct calls compared a prepared Type value with its old projection
annotation even though forwarded calls reached later materialization correctly.

Tests cover partial/full binding composition, refined references, wrong sorts,
invalid scopes, view failure/recovery and opaque domains. End-to-end coverage
selects both Type and Bool domains for one function, forwards/defaults the
inputs, compares direct and forwarded keys and verifies distinct canonical
instance identities. Wrong value sorts remain rejected. Example entry:
`type_domain` in ordinary-type-capabilities.fl.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p nir --test types`:
  53 tests; `/tmp/luna-domain-view-nir.log`.
- After adding the opaque-domain assertion,
  `python3 -B development/cargo.py test -p nir --test types function_input_domain_views`:
  2 tests; `/tmp/luna-domain-view-unit.log`.
- `python3 -B development/cargo.py test -p iir --test dependent_staging --test staged_preparation`:
  4 + 2 tests; `/tmp/luna-domain-view-iir.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  948 execution + 395 elaboration + 18 instantiation tests;
  `/tmp/luna-domain-view-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/ordinary-type-capabilities.fl --interpret type_domain`:
  `(tuple (bool true) (bool true) (bool true))`; `/tmp/luna-domain-view-example.log`.
- `git diff --check`: passed.

Broader ordinary-input propositions and method-contract conformance are still
unfinished. Changes remain on dev, uncommitted.

### Associated domains in demand-driven call input preparation

When an ordinary comptime input is demanded by a later parameter type, input
preparation now resolves its substituted parameter domain before checking the
source expression. Default annotations are similarly viewed after substitution
before evaluating/reusing prepared local values. Previously a Bool argument was
compared directly with Source<bool>.Item on this earlier preparation path, even
though ordinary call checking and staged binding already resolved that member.

Compilation tests cover Bool/Char indices in an opaque Tagged head, a comptime
copy default demanded by the witness type, rejected wrong argument domains and
rejected mismatching copy indices. The example adds boolean_index and
character_index signatures; these are compilation examples requiring caller
witnesses, not executable construction of opaque values.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test elaboration demanded_associated`:
  1 test; `/tmp/luna-demanded-associated-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration dependent`:
  48 execution + 18 elaboration tests; `/tmp/luna-demanded-associated-dependent.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration default`:
  161 execution + 41 elaboration tests (overlap with the preceding selection);
  `/tmp/luna-demanded-associated-defaults.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/ordinary-type-capabilities.fl --emit nir`:
  successful compilation; `/tmp/luna-demanded-associated-example.log`.
- `git diff --check`: passed.

A separate verified gap remains when the demanded value's associated domain is
Type and the value occurs in a later type index. Probe
`/tmp/luna-demanded-type-domain.fl`, driver `--emit nir`, exits 1:
`dependent call input is not available: expected comptime-value, got type`
(`/tmp/luna-demanded-type-domain.log`). Frontend call substitution still uses
sort-preserving substitute_partial instead of the refined input-binding plan
for this path. Closed staged binding already handles the same domain refinement.
Changes remain on dev, uncommitted.

### Caller-scoped domain refinement for demanded call inputs

`nir::FunctionCallBinding` now checks a callee telescope against optional inputs
in explicit caller frames. It resolves supplied inputs' domains in declaration
order and permits ComptimeValue -> Type refinement only for an exact Type
domain. Substitution removes the callee frame, shifts caller inputs under nested
binders, rejects missing referenced inputs, and checks source/result scopes.
The original `substitute_partial` remains sort preserving. The binding plan has
a stable S-expression Display including caller frames and refined slots.

Frontend demanded-input/default preparation, ordinary calls and generic call
inference use this plan. The previously failing
`/tmp/luna-demanded-type-domain.fl` now compiles. Tests also cover Type-valued
associated domains through defaults, caller-bound Type inputs, clause-generic
calls, wrong value sorts and mismatched opaque indices. These are compilation
checks with supplied opaque witnesses, not executable witness construction.
`type_index` in ordinary-type-capabilities.fl demonstrates the caller-bound case.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p nir --test types`: 55 passed;
  `/tmp/luna-call-binding-nir.log`.
- `python3 -B development/cargo.py test -p luna --test elaboration --test execution --test instantiation`:
  397 + 948 + 18 passed; `/tmp/luna-call-binding-regression.log`.
- After expanding the generic-call assertion,
  `python3 -B development/cargo.py test -p luna --test elaboration demanded_associated`:
  2 passed; `/tmp/luna-call-binding-targeted.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/ordinary-type-capabilities.fl --emit nir`:
  passed; `/tmp/luna-call-binding-example.log`.
- `git diff --check`: passed.

Remaining separate gap: an ordinary comptime value inside an opaque type index
in a positive trait requirement is not yet available during definition contract
preparation. `/tmp/luna-call-binding-capability.fl` declares Source<bool>.Item =
Type, Allowed<T>, Allowed<Tagged<Type,bool>> for bool, then requires
`bool :- Allowed<Tagged<A.as(Source).Item,value>>` in accept(A,value). Driver
`--interpret run` exits 1, `required comptime input is not yet available in this
definition` (`/tmp/luna-call-binding-capability.log`). This precedes call binding;
ordinary capability substitution still needs integration once this definition
path is supported. No contract check was relaxed to accept it.
Changes remain on dev, uncommitted.

### Ordinary comptime indices in positive capability contracts

Definition contract elaboration now permits an ordinary comptime parameter of
that same definition as a symbolic normal-form input. Visibility remains limited
to the active signature, own contract, or visible body local. Positive capability
queries use the scoped call binding, then resolve associated Type views before
selecting implementations. Preparation runs only for requirements that actually
reference the ordinary parameter frame; method clause-only obligations retain
their existing dispatch path.

A directly forwarded comptime default now reuses the already prepared argument
and symbol. It does not invoke a synthetic computation over an open caller frame.
This supports `.copy=value` when a capability requires an opaque type indexed by
copy. Program-state predicates are not moved into PTC.

`development/examples/capability-indexed-inputs.fl` executes direct/defaulted,
forwarded, and explicitly instantiated generic calls. Rejection tests cover the
wrong Type index, overridden copy, wrong value domain and missing caller evidence.
The earlier `/tmp/luna-call-binding-capability.fl` failure is resolved.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  949 + 398 + 18 passed; `/tmp/luna-contract-value-regression.log`.
- After deleting the superseded Type-only default-forwarding branch,
  `python3 -B development/cargo.py test -p luna --test execution --test elaboration default`:
  162 + 42 passed; `/tmp/luna-contract-value-defaults.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/capability-indexed-inputs.fl --interpret run`:
  `(tuple (bool true) (bool true) (bool true))`; `/tmp/luna-contract-value-example.log`.
- `git diff --check`: passed.

Remaining independently reproduced gap: replacing the example's explicit
`generic<Type,bool>(bool,true)` with `generic(bool,true)` fails before inference
can settle the demanded comptime parameter domain T. Probe
`/tmp/luna-capability-index-inference.fl`, `--emit nir`, exit 1, `type mismatch:
expected (rigid ...), found (primitive Type)`;
`/tmp/luna-capability-index-inference.log`. Do not weaken value-domain checking:
independent inference must provide T before demand-driven input checking.
Changes remain on dev, uncommitted.

### Infer independent comptime domains demanded only by capabilities

Removed the independent-input inference shortcut that returned immediately when
all parameter annotations were independent of the ordinary telescope. Capability
contracts can demand an ordinary comptime argument even in that case, so its
clause-generic domain must be inferred before preparation. Checked independent
arguments continue to be reused in the final call plan.

The capability-indexed-inputs example and its execution test now call
`generic(bool,true)` without explicit type arguments. Additional execution cases
cover bool, char, Type and tuple domains through both direct and evidence-bearing
forwarded calls. Rejections cover wrong indices/domains, runtime input used as a
comptime argument, and incompatible expected result types.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  949 + 398 + 18 passed; `/tmp/luna-capability-inference-regression.log`.
- After adding domain coverage,
  `python3 -B development/cargo.py test -p luna --test execution --test elaboration inferred_comptime_capability`:
  1 execution test (four domains) + 1 rejection test passed;
  `/tmp/luna-capability-inference-domains.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/capability-indexed-inputs.fl --interpret run`:
  `(tuple (bool true) (bool true) (bool true))`; `/tmp/luna-capability-inference-example.log`.
- `git diff --check`: passed.

Next reproduced limitation: signature positive-capability readiness currently
checks available inputs, not the prerequisite evidence for associated projections.
In `/tmp/luna-capability-order.fl`, accept(A,value:A.as(Source).Item) has
`requires A.as(Source).Item :- Allowed, requires A :- Source`. With concrete
Source<bool>.Item = bool and Allowed<bool>, `--interpret run` fails during
signature elaboration: `associated projection with ordinary Type inputs requires
established trait evidence` (`/tmp/luna-capability-order.log`). This needs
prerequisite-aware capability scheduling, not permission to project without
Source evidence. Changes remain on dev, uncommitted.

### Function capability prerequisites determine preparation order

Function signature capability preparation now retries atoms waiting specifically
for associated-projection evidence. The Error marker is separate from missing
inference context and tactic failures. A successful atom publishes its capability;
waiting atoms retry only while other atoms make progress, with preparation fuel
charged on each attempt. At the final pass, unresolved prerequisites still fail.
Failed atoms restore their capability/context assumptions and pending uses;
completed declaration caches and their recorded uses remain intact.

Positive bound `+` combinations are split into individual signature atoms, as are
positive conjunctions. Nothing is extracted from `not` or `or`. This applies to
ordinary Type inputs and clause Type parameters. Tests exercise multi-level
associated projections, reversed source order, conjunction, a dependent trait
argument preceding Source within `+`, missing/self-dependent evidence, negative
or disjunctive non-evidence, and invalid members.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  951 + 400 + 18 passed; `/tmp/luna-capability-order-regression.log`.
- After extending to clause parameters,
  `python3 -B development/cargo.py test -p luna --test execution --test elaboration ordinary_capability_dependencies`:
  1 execution test (six variants) + 1 rejection test passed;
  `/tmp/luna-capability-order-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration associated`:
  113 + 38 passed; `/tmp/luna-capability-order-associated.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/capability-dependent-bounds.fl --interpret run`:
  `(tuple (bool true) (bool true))`; `/tmp/luna-capability-order-example.log`.
- `git diff --check`: passed.

Remaining scope: non-function declaration contracts still form positive bounds
in source order during the earlier interface-admission phase. Reproduced with
`/tmp/luna-capability-order-struct.fl`: Container<A> requires
`A.as(Source).Item :- Allowed` before `A :- Source`; driver `--interpret run`
exits 1 with `declaration associated projection requires a given interface
constraint` (`/tmp/luna-capability-order-struct.log`). That phase uses declaration
projection admission rather than the checked-world path, and needs the same
prerequisite discipline without publishing circular interface evidence.
Changes remain on dev, uncommitted.

### Declaration-side capability ordering and concrete nominal dependencies

Contract preparation now schedules positive requires atoms before forming the
remaining contract. Declaration associated projection admission reports missing
evidence as a capability prerequisite, without weakening its given-evidence rule.
Nominal/alias/trait clause-parameter sugar bounds are also collected before being
scheduled, so dependencies within `A :- Accepts<A.as(Source).Item> + Source` can
be established independently of source order.

IIR materialization normalizes queued capability subjects and arguments after
checking for given evidence and before requesting concrete dispatch. Reference
execution retains its existing projection-dependency path. This prevents a
successfully selected nominal/trait obligation from queuing an unresolved
associated projection as a completed IIR type. Struct construction and field
reads resolve instantiated field Type views before checking/returning values.

Execution tests cover struct associated-field construction/read, enum contracts,
alias declaration bounds and trait contracts. Rejection tests cover missing
projection evidence in structs, enums, aliases, traits and implementations.
A separate compilation test verifies reversed implementation prerequisites can
form; concrete selection of such an implementation remains a distinct gap below.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  952 + 401 + 18 passed; `/tmp/luna-declaration-order-regression.log`.
- After adding implementation definition coverage,
  `python3 -B development/cargo.py test -p luna --test elaboration implementation_projection_prerequisites`:
  1 passed; `/tmp/luna-declaration-order-implementation.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/declaration-dependent-bounds.fl --interpret run`:
  `(bool true)`; `/tmp/luna-declaration-order-example.log`.
- `git diff --check`: passed.

Remaining reproduced gap: candidate qualification in capabilities::Resolver does
not resolve associated Type projections in instantiated implementation capability
requirements before matching their subject. `/tmp/luna-implementation-projected-capability.fl`
contains Get for Holder<A> requiring A :- Source and A.as(Source).Item :- Allowed,
with Source<bool>.Item = bool and Allowed<bool>. Concrete `Holder<bool>.as(Get).get()`
exits 1 with pending implementation selection
(`/tmp/luna-implementation-projected-capability.log`). Fixing this must share
cycle/step state with candidate qualification; recursively creating independent
normalization/selection sessions would risk circular proofs or uncontrolled recursion.
Changes remain on dev, uncommitted.

### Projected implementation prerequisites share candidate search state

Candidate qualification now resolves associated Type projections in instantiated
capability subjects and interface arguments before matching. Associated Type
normalization accepts a query-local resolver callback, allowing nested projection
selection to use the current Solver's active obligations, depth limits and
remaining fuel. Each normalization receives a bounded portion of the remaining
search; failures do not replenish it or consume the entire search allowance by
default. Givens are checked before and after normalization, and opaque heads are
preserved. Cycle/pending results remain non-evidence.

Associated Type and associated value formation obligations use projected-input
selection too. This closes the second-stage failure where a candidate could be
selected but materialization failed on projections in its originating trait's
own capability contract. Existing public query entry points still own independent
root sessions; recursive queries inside qualification use the shared solver.

Tests cover projected subjects, trait arguments, nested projections, projected
interface-formation prerequisites, wrong selected domains, cross projection /
capability cycles, exhaustion/recovery, and definite false siblings after a
pending projection. Executable example:
`development/examples/implementation-dependent-projections.fl`.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p elaboration --test associated --test capabilities`:
  16 + 26 passed; `/tmp/luna-projected-qualification-unit.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  954 + 403 + 18 passed; `/tmp/luna-projected-qualification-regression.log`.
- Example driver `--interpret run`: `(bool true)`;
  `/tmp/luna-projected-qualification-example.log`.
- Nested formation probe `/tmp/luna-projection-interface-formation.fl`, driver
  `--interpret run`: `(bool true)`; `/tmp/luna-projection-interface-formation.log`.
- `git diff --check`: passed.

Next verified admission gap: sugar bounds and explicit requires are still
scheduled in separate phases for nominal declarations. In
`/tmp/luna-mixed-bound-prerequisites.fl`, Container has
`where A :- Accepts<A.as(Source).Item>, requires A :- Source`; driver
`--interpret run` exits 1, `declaration associated projection requires a given
interface constraint` (`/tmp/luna-mixed-bound-prerequisites.log`). These must share
one prerequisite schedule to preserve the author's equivalence between declaration
sugar and an explicit type-variable declaration plus requires. Changes remain on
dev, uncommitted.

### Declaration sugar and explicit requires share prerequisite preparation

Nominal parameter-bound preparation now includes explicit positive requires atoms
in its prerequisite schedule when sugar bounds are present. Function signature
formation keeps sugar atoms pending instead of eagerly rejecting a bound that
needs a later explicit constraint; the same pending list then receives explicit
requires and advances as ordinary comptime inputs become available. Synthetic
subjects retain the original clause parameter identities and source spans.

Execution coverage compares four equivalent spellings across functions, structs,
enums, traits and implementations (20 cases). Separate coverage checks dependencies
across two clause variables and from a clause bound to an ordinary comptime Type
input. Rejections ensure negation/disjunction cannot supply missing Source
evidence. The declaration-dependent-bounds example now mixes declaration sugar
with explicit requires. No syntax was relaxed: `T requires Trait` remains invalid.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  955 + 404 + 18 passed; `/tmp/luna-mixed-bound-regression.log`.
- After adding cross-input coverage,
  `python3 -B development/cargo.py test -p luna --test execution sugar_capability_dependencies`:
  1 passed; `/tmp/luna-mixed-bound-cross-inputs.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/declaration-dependent-bounds.fl --interpret run`:
  `(bool true)`; `/tmp/luna-mixed-bound-example.log`.
- `git diff --check`: passed.

The earlier `/tmp/luna-mixed-bound-prerequisites.fl` failure is resolved.
This completes the reproduced positive-capability ordering path, not the broader
unfinished compiler or the remaining negative/disjunctive ordinary-input contracts,
method contract conformance and other language features. Changes remain on dev,
uncommitted.

### Explicit input scopes for type-proposition contexts and evidence

PTC Context now has an explicit lexical sort-frame scope, constructed by
`Context::with_scope`; the default remains closed. Atom validation checks Type
terms against that scope, including trait inputs and classified types. Structural
view discovery accepts correctly scoped shapes. Proof and counterexample objects
retain the exact frames; replay rejects changed frame depth, width or sorts.
S-expression dumps include scope frames. `theory_context` deliberately drops
local frames together with caller premises, while forks retain them.

Scoped given-capability lookup uses raw equality facts only when its requested
input frames match the PTC context; legacy closed contexts retain the prior
binder-wrapper path. Thus A == B in the declared input scope can transport a
trait given on A to B without legitimizing a free-bound closed query.

Tests cover opaque congruence without unfolding, distinct input identities,
trait disjunction/negation logic, bad depth/index/sort, exact-scope proof and
counterexample replay, exhaustion-independent existing behavior, and structural
views across distinct outer/inner frames. This changes type-proposition scope
handling only; it does not route program or memory predicates through PTC.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p elaboration --tests`:
  318 tests across 20 suites; `/tmp/luna-scoped-ptc-regression.log`.
- After adding the multi-frame structural-view case,
  `python3 -B development/cargo.py test -p elaboration --test ptc scoped_`:
  6 passed; `/tmp/luna-scoped-ptc-frames.log`.
- `git diff --check`: passed.

Frontend integration remains required. Baseline probe
`/tmp/luna-ordinary-type-predicates.fl` defines same(comptime A:Type,comptime B:Type,
value:A)->B requiring A == B, then calls same(bool,bool,true). Driver
`--interpret run` fails with `ill-formed type equality: invalid type term:
InvalidBound { depth: 0, index: 0 }` (`/tmp/luna-ordinary-type-predicates.log`).
No source-level completion is claimed for that probe. Next steps: install scoped
definition assumptions; collect ordinary slots from nonpositive requirements;
defer their checks until actual inputs are prepared; apply the checked call-binding
substitution to predicates and check them under the caller's scope. Scoped trait
world propositions also need admission/replay integration for nonpositive queries.
Changes remain on dev, uncommitted.

### Ordinary-input type predicates reach definition and call checking

Definition contexts now install their function input frames before assuming
ordinary type requirements. Direct equality checks and inference obligations use
raw scoped terms when the PTC scope matches the caller telescope, preserving
A == B evidence through generic call inference; legacy closed contexts retain
binder wrapping. Scoped positive capability premises are also available for
local Boolean implications such as A(T) => A(T) or B(T).

Formation checks validate but defer requirements referencing ordinary inputs.
Demand preparation discovers inputs used only by these predicates, including
defaults. Both generic and ordinary direct calls instantiate clause parameters
in the callee frame, apply FunctionCallBinding to all predicate terms, resolve
associated views and discharge the result in the caller context. Closed residuals
may use world-backed proposition decisions. Open residuals without local proof
remain rejected; no implementation absence is invented for them.

Execution/rejection coverage includes equality-based returns, generic conversion,
forwarding, Type defaults, type disequality/disjunction, closed negative trait
requirements, positive given-to-disjunction reasoning, wrong actual inputs,
unproved forwarding, and refusal to bypass constraints through indirect staged
calls. Example: `development/examples/ordinary-type-predicates.fl`.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p elaboration --tests`:
  319 passed; `/tmp/luna-ordinary-predicate-solver.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`:
  959 + 405 + 18 passed; `/tmp/luna-ordinary-predicate-regression.log`.
- Example driver `--interpret run`: tuple of four true values;
  `/tmp/luna-ordinary-predicate-example.log`.
- Generic equality probe `/tmp/luna-ordinary-equality-generic.fl`, driver
  `--interpret run`: `(bool true)`; `/tmp/luna-ordinary-equality-generic.log`.
- `git diff --check`: passed.

Remaining concrete scope gap in world propositions:
`/tmp/luna-scoped-world-predicate.fl` has forward(comptime T:Type) requiring T :- A,
calling allowed(bool), whose requirement is not(bool :- Forbidden). The world
query also visits the caller's scoped A(T) assumption and rejects its Bound input:
`invalid trait proposition input: InvalidBound { depth: 0, index: 0 }`
(`/tmp/luna-scoped-world-predicate.log`, driver --interpret run exit 1).
Next work must validate/discharge world proposition atoms in their context frames,
retain scoped givens and formation checks, and keep abstract absence pending.
The current pass does not claim all ordinary-input negative/disjunctive contracts
are complete. Changes remain on dev, uncommitted.

### Scoped world propositions and ordinary-input forwarding

World proposition inputs now validate against the exact PTC context frames.
Given lookup preserves those frames, including multiple nested scopes. Interface
parameter requirements and capabilities use scoped substitution before decision.
An unresolved bound input remains Pending; a context-proved structural view may
supply a concrete input for implementation selection. Decisions retain the input
equality proofs in their S-expression dumps and remain tied to the borrowed world.
Ordinary-input predicate checking now permits this scoped world fallback.

Regression coverage checks closed negative queries alongside scoped givens,
equality-based forwarding, refusal of unproved abstract negation and disjunctive
input narrowing, invalid bound indices, multi-frame interface formation, and
proof replay. Given interfaces retain their existing implied capability behavior.

Validation in /home/fin/luna, all exit 0:
- python3 -B development/cargo.py test -p elaboration --tests
  (/tmp/luna-scoped-world-solver.log).
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  (960 + 406 + 18 passed; /tmp/luna-scoped-world-regression.log).
- Original /tmp/luna-scoped-world-predicate.fl driver probe now returns (bool true)
  (/tmp/luna-scoped-world-fixed.log).
- git diff --check.

This closes the concrete scoped-world gap above, not all higher-order or method
contract conformance work. Changes remain on dev, uncommitted.

### Method contract compatibility uses ordinary-input scope

Compatibility checking now creates the method's input sort frame while retaining
only implementation-block premises and instantiated interface-method premises.
Interface method predicates/capabilities substitute declaration parameters in
that frame. Capability conformance uses scoped world decisions; it does not
assume the implementation method's own demands. This accepts alpha-renamed
ordinary Type equalities and matching capability requirements, permits dropping
an unnecessary capability, and rejects stronger equalities or unrelated bounds.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py test -p luna --test elaboration ordinary_input_method_contracts
  (/tmp/luna-method-contract-unit.log).
- python3 -B development/cargo.py test -p luna --test elaboration --test execution --test instantiation
  (407 + 960 + 18 passed; /tmp/luna-method-contract-regression.log).
- git diff --check.

Remaining executable probe: /tmp/luna-method-ordinary.fl declares matching
Service.same(comptime A:Type,comptime B:Type,value:A)->B requiring A == B,
implements it for bool and calls bool.as(Service).same(bool,bool,true).
Originally failed method signature compatibility; now reaches the invocation and
fails `expected 1 arguments, got 0` at 272..292
(/tmp/luna-method-ordinary.log). The declaration tests do not claim this staged
trait method invocation is supported. Next inspect propagation of the implicit
Self clause argument through direct method input preparation/specialization.
Changes stay on dev, uncommitted.

### Staged trait calls carry complete entry substitutions and contracts

Direct method calls now pass Self, interface parameters and method parameters
through the same declaration-substitution lookup used by method inference.
This fixes ordinary capability/predicate checks receiving an empty substitution
for a trait entry.

During materialization a staged trait call selects its implementation body
before binding ordinary inputs. It preserves the interface type predicates,
capabilities and program contracts on that body, with interface declaration
substitution performed in the ordinary-input scope. Parameter/default and result
bindings are remapped to the selected body. Ordinary-input staging then closes
those contracts along with the body; method formation no longer prematurely
requires these bound inputs to be closed. A weaker implementation body therefore
does not erase the interface entry contract.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  (962 + 408 + 18 passed; /tmp/luna-staged-method-regression.log).
- Added afterward: python3 -B development/cargo.py test -p luna --test execution staged_trait_calls_retain_type_contracts
  (1 passed; /tmp/luna-staged-method-retained.log).
- python3 -B development/cargo.py run -p luna -- /tmp/luna-method-ordinary.fl --interpret run
  now returns (bool true); /tmp/luna-method-ordinary-fixed.log.
- git diff --check.

Coverage includes equality and capability checks, interface/method parameters,
default implementations and defaults, wrong actual Type inputs, and post-source
fault injection demonstrating retained type capabilities and runtime preconditions
when the selected body has weaker requirements. This fixes the invocation probe
from the previous section. General higher-order staged call support remains
separate. Changes stay on dev, uncommitted.

### Method signature equality under ordinary Type contracts

For matching ordinary function binders, method compatibility now compares their
parameter domains and results in the interface contract's input frame. This
allows A == B to justify an implementation using B where the interface uses A,
without adding implementation-only premises. Declaration checks retain complete
signature scope validation and parameter-sort matching.

IIR call binding still checks declaration identity, phase, arity and input sorts
before evaluation. When entry and body signatures differ, it now binds both to
the same completed ordinary inputs and compares the residual signatures and all
parameter domains, including erased ones. This enables the validated interface
adaptation without accepting an unrelated concrete signature.

Validation in /home/fin/luna, all exit 0:
- python3 -B development/cargo.py test -p luna --test execution --test elaboration method_signature_equivalence
  (2 passed; /tmp/luna-method-equivalence-tests.log).
- python3 -B development/cargo.py test -p iir --test dependent_staging --test staged_preparation
  (6 passed; /tmp/luna-method-equivalence-staging.log).
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  (964 + 409 + 18 passed; /tmp/luna-method-equivalence-regression.log).
- git diff --check.

Execution covers renamed ordinary Type parameters with interface A == B and
implementation parameter/result B. Rejection cases cover implementation-only
equality, insufficient disjunctive premises, phase/input-sort mismatch, unrelated
result types and erased parameter domains. Arbitrary subtyping or higher-order
staged values are not introduced. Changes remain on dev, uncommitted.

### Staged trait postconditions retain entry snapshots

The staged method entry-contract transfer now remaps each interface snapshot's
parameter to the selected body's parameter, reuses an existing body snapshot for
that input, remaps predicate references to the reused binding, and retains any
missing snapshots before ordinary-input specialization. This prevents interface
`value'old` references from becoming unbound and avoids duplicating snapshots
when the default body already contains the same contract.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py run -p luna -- /tmp/luna-staged-method-old.fl --interpret run
  now returns (bool true), previously MissingBinding at 120..125;
  /tmp/luna-staged-method-old-fixed.log.
- python3 -B development/cargo.py test -p luna --test execution staged_trait_postconditions
  /tmp/luna-staged-method-old-test.log.
- python3 -B development/cargo.py test -p luna --test execution staged_
  /tmp/luna-staged-method-old-regression.log.
- git diff --check.

The new test exercises runtime and comptime input snapshots with default bodies,
explicitly renamed result/input bindings, and inherited guarantees. Each of the
six variants executes successfully, then a post-source body mutation violates
the guarantee and must fail with ProgramContractRejected. Changes stay on dev,
uncommitted.

### Scalar evidence for direct runtime defaults in receiver calls

Runtime default preparation now accepts a direct input read when the current
scoped type contract proves a structural scalar type for that input. The retained
expression keeps its original bound annotation; the PTC structural view is only
stability evidence. Arbitrary abstract inputs and owned nominal types still
require the existing P3 evidence path. Opaque normal-form heads are not unfolded.

Example development/examples/staged-receiver-contracts.fl combines a runtime
receiver, ordinary Type equalities, equivalent interface/body signatures and a
forwarded optional parameter. The receiver flips a flag, the explicit argument
observes it, and the default reuses the prepared value. The execution test checks
both result values and exactly one call each to receiver/value.

Validation in /home/fin/luna, all exit 0:
- python3 -B development/cargo.py test -p luna --test execution staged_receiver_contract_defaults
  /tmp/luna-staged-receiver-test.log.
- python3 -B development/cargo.py test -p luna --test execution --test elaboration default
  /tmp/luna-default-scalar-evidence.log.
- After adding opaque-head rejection:
  python3 -B development/cargo.py test -p luna --test elaboration runtime_default_forwarding
  /tmp/luna-default-scalar-boundary.log.
- git diff --check.

The initial receiver probe failed at default preparation with the P3 evidence
message even with A == B and B == bool. The same program is now the executable
example above. This change establishes direct scalar forwarding only, not general
P3 inference for arbitrary default expressions. Changes stay on dev, uncommitted.

### Higher-order method signatures consume scoped contract equality

Method parameter/result comparison now uses bounded structural signature checks.
PTC proves leaf equalities in the current lexical context. When descending into
matching function binders, the checker appends their sort frame and lifts every
outer type assumption by one depth, including trait/kind predicates and Boolean
combinations. Pointer, comptime-function and tuple structure can be traversed;
normal-form heads are never unfolded. The entire parameter/result comparison
shares one fuel allowance, with separate bounded PTC query reservations.

The source probe /tmp/luna-method-higher-order.fl originally rejected an
interface callback fn(A)->A implemented as fn(B)->B under interface A == B.
It now executes to (bool true); /tmp/luna-method-higher-order-fixed.log.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py test -p elaboration --lib nested_signature
  (1 passed; /tmp/luna-nested-signature-unit.log).
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  /tmp/luna-nested-signature-regression.log.
- git diff --check.

Unit coverage includes two nested scopes, inner/outer same-index distinction,
invalid-depth rejection, budget exhaustion and subsequent recovery. Source tests
execute an equivalent callback and reject implementation-only or insufficient
disjunctive premises. This is contract-guided structural equality, not arbitrary
function subtyping or complete congruence under every type constructor. Changes
remain on dev, uncommitted.

### Scoped callbacks through nominal storage and admitted congruence

The signature checker now obtains forward-congruence child obligations from
PTC's existing admitted-head relation rather than maintaining separate constructor
rules. Nested callback equality consequently propagates through nominal and
admitted normal-form arguments, with no head unfolding or injectivity assumption.

Struct field projection now substitutes type parameters in the caller input frame.
Runtime-storage phase inspection likewise carries lexical frames when opening
function signatures and instantiating nominal fields/variants. Its memoization
includes scope for open terms; closed recursive terms canonicalize to empty scope
so recursive callback storage reaches the cycle guard instead of accumulating
irrelevant function frames indefinitely.

Example: development/examples/method-callback-contracts.fl executes a callback
stored in Box<fn(A)->A> through a method whose implementation uses B under A == B.
The initial probe failed method compatibility, then exposed closed-substitution
failures in field projection and runtime-storage inspection; all three paths are
now covered by the executable regression.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py test -p elaboration --lib nested_callbacks
  (1 passed; /tmp/luna-callback-congruence-unit.log).
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  (968 + 412 + 18 passed; /tmp/luna-callback-container-regression.log).
- After the closed-recursion cache refinement:
  python3 -B development/cargo.py test -p nir --test phases
  (5 passed; /tmp/luna-scoped-storage-phase.log), and
  python3 -B development/cargo.py test -p luna --test execution --test elaboration callback
  (/tmp/luna-callback-congruence-source.log).
- git diff --check.

Boundaries tested: unadmitted normal-form heads do not gain congruence; distinct
heads and underlying function types stay distinct; malformed nested references
and comptime-only nominal fields are rejected; recursive runtime callback storage
terminates without hiding forbidden sibling fields. Changes remain on dev,
uncommitted.

### Ordinary Type inputs through nominal construction and matching

NIR struct-field and enum-variant instantiation now expose explicit caller-scope
entrypoints while preserving closed wrappers. Source construction, payload
selection, record/enum patterns and field assignment use the caller frames.
Coverage specialization uses its existing type context's scope for payload/field
instantiation; coverage propositions themselves remain outside PTC. Phase storage
inspection shares the new NIR entrypoints.

Nominal field-default expressions have a separate scoped substitution entrypoint.
The legacy Expr substitution retains its existing behavior: changing it globally
broke an established associated-domain forwarding test, so the new operation is
used explicitly at nominal defaults. Nominal type-formation predicates now
substitute declaration parameters in the caller type context, allowing existing
ordinary-input premises to discharge their requirements without assuming them.

Example development/examples/ordinary-type-containers.fl constructs and updates
Box<T>, destructures it, constructs Choice<T>, and exhaustively matches its
payload under ordinary comptime Type inputs. Separate execution coverage checks
struct and named-enum defaults under T == bool.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py test -p nir --test scoped_contracts --test phases
  (4 + 5 passed; /tmp/luna-nominal-scope-core.log).
- python3 -B development/cargo.py test -p elaboration --lib coverage
  (17 passed; /tmp/luna-nominal-scope-coverage.log).
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  (970 + 414 + 18 passed; /tmp/luna-nominal-scope-regression.log).
- git diff --check.

Rejection cases cover unproved container constraints, wrong constructed/assigned
field types, incomplete enum matches, wrong scope sorts/argument counts and closed
API rejection of open substitutions. Nested callback field substitution preserves
outer bound depth. These changes do not claim every resource-verification path
or arbitrary runtime-dependent nominal representation is complete. Changes remain
on dev, uncommitted.

### Program contracts inspect nominal fields in the correct input frame

The program-flow walker now carries a type-input frame separately from symbolic
program-state frames. Nominal field/payload substitution uses that frame when
analyzing definitions. During call-contract evaluation, record projection uses
the evaluated input's retained nominal instance rather than a callee template's
unsubstituted Type-slot annotation; the declared field owner and package shape
are still checked. This prevents both closed-substitution failures and accidental
interpretation of a callee input index in the caller's frame.

The initial /tmp/luna-scoped-container-spec.fl probe failed with invalid-signature
while checking box.flag on Box<T>. New source coverage checks requires box.flag,
ensures result, field updates, enum destructuring, and forwarding from a caller
whose actual Type input occupies a different index. Wrong preconditions and false
results are still rejected. Runtime state predicates remain in flow verification,
not PTC.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py test -p luna --test execution --test elaboration container_program_contracts
  /tmp/luna-container-program-spec-tests.log.
- After adding cross-index forwarding:
  python3 -B development/cargo.py test -p luna --test execution container_program_contracts
  /tmp/luna-container-spec-forward.log.
- python3 -B development/cargo.py test -p elaboration --test verification
  (32 passed; /tmp/luna-container-spec-verification.log).
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  (971 + 415 + 18 passed; /tmp/luna-container-spec-regression.log).
- git diff --check.

This connects nominal program-contract reasoning for ordinary Type inputs. It
does not yet establish equivalent coverage in every ownership/drop/memory-summary
path. Changes remain on dev, uncommitted.

### Type-input metadata no longer blocks selective memory summaries

Dereferenced nominal field places now substitute fields in the caller input
scope, matching owned-field construction/projection. The memory summary builder
keeps ordinary Type-input positions and local annotations but does not classify
Sort::Type/Primitive::Type inputs as owned runtime storage or pointer roots.
Other sorts, pointers and unknown owned representations retain their checks.

Example development/examples/ordinary-type-memory-writes.fl writes only the flag
field through *Box<T>. Its caller proves a result using the untouched value
field. Before these changes the field place rejected the open type argument,
and after fixing that the missing summary conservatively invalidated the proof.
The example now executes to true. The test also inspects the summary: one write,
parameter index 1 and field index 1. Negative tests reject stale facts about the
written field and a wrong scalar write type.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py run -p luna -- /tmp/luna-staged-memory-summary.fl --interpret run
  (bool true); /tmp/luna-staged-memory-summary-fixed.log.
- python3 -B development/cargo.py test -p luna --test execution --test elaboration ordinary_type_
  (11 + 5 passed; /tmp/luna-ordinary-memory-tests.log).
- python3 -B development/cargo.py test -p elaboration --lib memory
  (3 passed; /tmp/luna-memory-summary-core.log).
- python3 -B development/cargo.py test -p luna --test execution --test elaboration memory
  (10 + 2 passed; /tmp/luna-ordinary-memory-regression.log).
- git diff --check.

This establishes summaries for Type-input functions with known pointer-field
footprints, not arbitrary owned abstract payloads or all higher-order staged
summary specializations. Changes remain on dev, uncommitted.

### Proven scalar identity admits dependent pointer operations

Pointer read/write admission now accepts the existing intrinsic value types or a
PTC structural view proved equal to such a type in the declaration context.
Unknown abstract pointees, owned nominal values and opaque normal-form heads
remain rejected by the existing transfer-contract boundary. Writes still check
the supplied value against the stored type; their pointer operand carries the
resulting identity view so operation operand annotations agree.

Source execution covers ordinary Type and clause Type parameters, direct pointer
reads/writes, nominal field reads/writes, literal writes and type-preserving
staging. Rejection coverage includes missing evidence, owned pointees, opaque
Identity<bool> heads and mismatched scalar values. No normal-form body is unfolded.

A separate memory-summary test confirms the existing forwarding path rebases
writes from set(T, pointer) to forward(X, Y, pointer): the final write identifies
parameter 2, field 1, and the caller retains its untouched value-field fact.

Validation in /home/fin/luna, all exit 0:
- python3 -B development/cargo.py run -p luna -- /tmp/luna-dependent-pointer-field.fl --interpret run
  (bool true); /tmp/luna-dependent-pointer-field-fixed.log.
- python3 -B development/cargo.py test -p luna --test execution --test elaboration pointer_
  (51 + 6 passed; /tmp/luna-pointer-type-evidence.log).
- python3 -B development/cargo.py test -p luna --test execution ordinary_type_memory_summaries_forward
  (1 passed; /tmp/luna-forward-memory-tests.log).
- git diff --check.

This admits operations from proved intrinsic type identity, not arbitrary Copy
implementations or general ownership transfer. Summary precision for abstract
payload accesses beyond known scalar fields remains separate. Changes remain on
dev, uncommitted.

### Memory footprints use locally proved scalar type identities

Function memory inference now installs a separate scoped type context from its
instantiated type requirements. Bounded, locally cached structural views classify
abstract annotations only when proved equal to intrinsic scalar primitives.
This supports scalar parameter ownership checks, scalar read/write access types,
and scalar result classification while retaining the original summary signature.
Program-state preconditions are not added to this type context.

The view deliberately does not materialize pointer roots, callable leaves or
aggregate layouts from type equality alone; those need their corresponding alias
and code-identity evidence. Missing proof, exhausted budgets and opaque types keep
the conservative behavior. Caches live within an individual inference instance.

Example development/examples/proved-scalar-memory-writes.fl changes a Box<T>
payload under T == bool while the caller retains a fact about its untouched flag.
The original probe failed to prove the caller postcondition; it now executes true.
Tests inspect the inferred Bool write and its parameter/field path, reject stale
facts about the changed field, exercise readonly scalar results, remove evidence
and retry, and ensure pointer/callable dependencies are not erased by scalar views.

Validation in /home/fin/luna, exit 0:
- python3 -B development/cargo.py run -p luna -- /tmp/luna-proven-scalar-memory.fl --interpret run
  (bool true); /tmp/luna-proven-scalar-memory-fixed.log.
- python3 -B development/cargo.py test -p luna --test execution --test elaboration memory_
  /tmp/luna-proved-memory-regression.log.
- python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation
  (977 + 418 + 18 passed; /tmp/luna-proved-memory-source.log).
- python3 -B development/cargo.py test -p elaboration --lib memory
  (3 passed; /tmp/luna-proved-memory-core.log).
- git diff --check.

General dependent pointer/callable result summaries and abstract owned aggregate
footprints still need richer dependency evidence. Changes stay on dev, uncommitted.

### Dependent Boolean inputs in program contracts

Program requires/ensures/assumes consumers now retain the proved Boolean view on the predicate expression, after checked type equality; declaration signatures and parameter bindings retain their dependent types. Flow derives scalar parameter domains and direct-call result domains from bounded, function-local type requirements, separately from value facts. Type equality alone cannot establish truth. Call postconditions now transfer through ordinary Type-input forwarding. Runtime assumes retains its check, including false-input failure.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation` exited 0 (978 execution + 419 elaboration + 18 instantiation; /tmp/luna-dependent-boolean-regression.log). Additional `--test execution dependent_boolean_assumes` exited 0 (runtime rejection plus exhausted-budget/fresh-retry checks; /tmp/luna-dependent-boolean-assumes.log). The dependent-boolean-contracts.fl CLI example returns `(bool true)` (/tmp/luna-dependent-boolean-example.log). Rejections cover false call premises, false callee guarantees, missing value proofs, absent type equality and opaque Identity<bool> heads. No commits.

Remaining: general dependent scalar operator expressions and closure-local type scopes need further integration. Flow deliberately does not reuse the enclosing type context for scalar result views inside closure bodies; aggregate/pointer evidence remains separate from scalar domain classification.

### Dependent Boolean scalar operations

Structural type inspection now consults scoped equality evidence for ordinary Bound inputs as well as rigid/associated inputs. Logical operand nodes carry their proved scalar views after elaboration, retaining original declaration signatures. This connects not/and/or on T == bool to NIR evaluation and Boolean program conditions, including two ordinary Type inputs related by equality. Tests cover true/false postconditions, absent/ambiguous type proofs, opaque normal-form heads, and runtime failure in selected versus skipped right-hand branches.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation` exited 0 (980 + 420 + 18 tests; /tmp/luna-dependent-logic-regression.log). `run -p luna -- development/examples/dependent-boolean-operators.fl --interpret run` exited 0 with three true Boolean results (/tmp/luna-dependent-logic-example.log). No commits.

A separate program-verifier precision gap was observed: `requires left or right` followed by `ensures result {left or right}` loses the disjunction when premise branches join, so it cannot yet prove this valid guarantee. This is a program-state fact preservation problem, not a type proposition. General dependent arithmetic and closure-local scopes also remain unfinished.

### Preserve established Boolean disjunctions in program verification

Facts now retains bounded compound conditions with versioned scalar references, independently of interval/inequality projection. Assumption remains transactional; forks copy only current conditions, and joins intersect conditions established on both paths. Assignments make old condition identities unusable. Renaming currently drops affected retained conditions through current-version filtering rather than inventing equivalence between old and new values. Predicate proofs can use retained facts and replay still checks lineage, scalar premises and entailment in the destination facts. Facts Display includes retained conditions as S-expressions. Scalar flow can refine a Boolean result from an established matching condition, fixing the previously documented `requires a or b` / `ensures result {a or b}` gap.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p elaboration --test verification` exited 0 (33 tests; /tmp/luna-retained-conditions-kernel.log), plus verification lib tests (15). Source regression `test -p luna --test execution --test elaboration --test instantiation` exited 0 (981 + 421 + 18; /tmp/luna-retained-conditions-source.log). New kernel checks cover non-injectivity of disjunction into either arm, both-sided versus one-sided joins, opposite assumptions, assignment invalidation, proof replay and transactional budget exhaustion/retry. Source checks cover unchanged branch paths, invalidated locals, false callers and dependent Boolean contracts. /tmp/luna-disjunctive-contract.fl executes to `(bool true)`. The dependent-boolean-operators example now uses the stronger disjunctive premise. No commits.

Remaining precision: compound facts currently match structural conditions over current identities; general Boolean consequence propagation, canonicalization and retention across value renaming/copies require further work. PTC remains uninvolved in program-state conditions.

### Compound program facts cross explicit value copies

Retained conditions now have bounded identity substitution for scalar comparisons, degree-limited polynomials, negation, conjunction and disjunction. Ordered copy installation transports a snapshot condition only if every referenced value has an explicit destination mapping, or remains current in the same lineage. No unbound source identity crosses invocation frames. Copies keep separate value versions, so source mutation does not invalidate an independent copy while destination mutation does invalidate its conditions. Control-flow renaming transports conditions onto fresh merge identities before intersecting both paths.

Compound condition matching admits commutative And/Or operands under a shared work budget. These are pure predicates over already evaluated values; runtime short-circuit evaluation order does not change. This connects local forwarding, captures, snapshots followed by source writes, and branches that swap the two Boolean values.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p elaboration --test verification` exited 0 (34 tests; /tmp/luna-copy-conditions-kernel.log); `test -p luna --test execution --test elaboration --test instantiation` exited 0 (982 + 422 + 18; /tmp/luna-copy-conditions-source.log). New rejection tests cover incomplete copies, destination overwrite, one-sided branch facts and closure capture of only one disjunct. The kernel test checks fresh merge IDs, commuted disjunctions, failed replay on old branches, and refusal to strengthen OR into AND. /tmp/luna-copy-disjunction.fl executes to true; a readable example is development/examples/copied-disjunctive-contracts.fl. No commits.

Remaining: general Boolean consequence propagation and associative/canonical predicate normalization. The previous limitations on copies and merge renaming are resolved for the explicitly mapped scalar-condition fragment; this does not grant any resource Copy capability or aliasing permission.

### Associative matching of established Boolean predicates

Revalidated that the branch example `requires a or b; if not a {b} else {a}` already succeeds through current copy/condition transfer. The next failing source case was forwarding `a or (b or c)` into `(a or b) or c`. Retained-condition matching now flattens only matching And or Or operators, then performs bounded multiset matching using the existing recursive equivalence relation. Mixed operators, negation boundaries and operand multiplicity remain intact; executable expressions are never reordered. Flattening observes the depth/term bounds and matching consumes shared fuel.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p elaboration --test verification` exited 0 (35 tests; /tmp/luna-associative-conditions-kernel.log), and `test -p luna --test execution --test elaboration --test instantiation` exited 0 (983 + 423 + 18; /tmp/luna-associative-conditions-source.log). Tests cover both OR-true and AND-false regrouping, operand permutation through calls, dropped disjuncts, changed operators, proof replay and exhaustion/recovery. /tmp/luna-disjunction-association.fl now executes to true; development/examples/associated-boolean-contracts.fl provides a readable example. No commits.

Remaining: distributive/idempotent Boolean normalization and general consequence propagation are not implemented by this matching rule. This change stays in the program verifier and does not route predicates through PTC.

### Dependent scalar domains in closure invocation frames

Revalidated ordinary T == u8 arithmetic and rejection of unproved overflow. Found that closure signature parameter types include an extra callable binder, whereas source-generated lambda body annotations retain the enclosing ordinary-input scope. Flow now removes that signature binder before initializing the independent invocation parameter domains, and applies enclosing scalar type evidence. Nested lambda bodies continue using the enclosing type context; their program-state frames and value premises remain independent. The prior blanket suppression of scalar result views in closure bodies is removed for this representation.

A further failure in a closure calling a clause-generic function exposed closed substitution of program contracts with open caller type arguments. FunctionInstance pre/postcondition instantiation now uses explicit caller input frames when declaration arguments contain open references, while preserving the existing closed-input contract path. Tests use the proper `where T, requires T == bool` declaration form.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation` exited 0 (984 + 424 + 18; /tmp/luna-dependent-scalar-closures-regression.log). New tests exercise nested Byte closures, Boolean checked assumes and generic call contracts, runtime false/overflow rejection, and refusal to borrow creator value guarantees for future invocation parameters. /tmp/luna-dependent-byte-closure.fl executes to unsigned integer 9. Readable example: development/examples/dependent-scalar-closures.fl. No commits.

Separate remaining staging gap: explicitly capturing an ordinary comptime Type parameter in a runtime closure (e.g. calling identity(T,item) inside it) currently reaches the comptime-only runtime-storage rejection. This needs capture/staging support; scalar domain evidence does not license runtime storage of Type values. General dependent binder forms in hand-constructed NIR also need their own closure scope contract; the source lambda path currently has runtime parameters only.

### Ordinary comptime Type references inside runtime lambdas

CaptureFrame now records the lambda's execution phase explicitly. When a runtime lambda reads an ordinary comptime Type input, elaboration emits its existing symbolic TypeValue instead of adding a Type-valued environment cell. The symbol remains in the enclosing ordinary-input scope and is closed during call instantiation. Runtime payload captures remain ordinary captures, including nested closures. The decision uses the lambda's phase rather than the temporary staged-argument checking context; comptime lambdas retain their legitimate Type-valued captures. Runtime Type storage and returns remain rejected.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation` exited 0 (986 + 425 + 18 tests; /tmp/luna-staged-type-closures-regression.log). Source tests execute bool/char specializations with parameterized lambdas and nested payload captures; NIR assertions verify three runtime lambdas contain no Type environment cell while payload cells remain. A comptime Choose function still captures Type and computes a typealias. Rejection tests cover runtime Type locals/results and mismatched type arguments. /tmp/luna-type-in-closure.fl and /tmp/luna-comptime-type-capture.fl both execute to true. Readable example: development/examples/staged-type-closure-inputs.fl. No commits.

This resolves the previous explicit ordinary Type capture blocker for runtime lambdas. General ordinary comptime value captures and interactions with nested comptime lambdas require separate staging support; no general capture erasure rule is claimed.

### Staged-only ordinary value references do not create runtime captures

Revalidated that forwarding comptime bool from a runtime lambda already executes correctly. Ordinary comptime Integer forwarding failed because capture discovery still allocated runtime storage for the erased reference. When elaboration checks a staged input inside a runtime lambda, an ordinary comptime value parameter now remains a symbolic local reference for later forwarding substitution, without adding a runtime capture. An additional runtime use of the same parameter still creates the normal capture; comptime lambdas retain their own capture rules.

Tests execute nested runtime closures forwarding Integer and bool, with normal-form Type defaults depending on Integer values. IIR checks verify three distinct tag specializations and unchanged normal-form argument identities, while NIR checks verify absent staged-only captures and retained mixed-use bool captures. Direct runtime Integer locals/results remain rejected.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p luna --test execution --test elaboration` exited 0 (988 + 426 tests; /tmp/luna-staged-value-closures-regression.log); `test -p luna --test instantiation` exited 0 (18 tests; /tmp/luna-staged-value-closures-instantiation.log). /tmp/luna-integer-in-closure.fl executes to true/false/true. Readable example: development/examples/staged-value-closure-inputs.fl. No commits.

Remaining: symbolic ordinary-input computations such as `choose(count == 3, item)` and a dependent scalar default `comptime .selected:bool = count == 3` are still evaluated too early and fail with MissingBinding. This is a deferred comptime-expression issue, separate from capture storage; direct forwarding and existing symbolic normal-form defaults are now connected through these lambdas. Forwarding after runtime writes/address escape still needs existing staging safety evidence.

### Deferred mathematical Integer comparisons and signed staged constants

The premature evaluation gap came from classifying mathematical Integer comparisons by their Bool result. Staged-input eligibility now admits comparison IntegerBinary nodes, and materialization routes them through the comptime interpreter rather than the fixed-width folding path. This preserves the original expression until ordinary comptime arguments are supplied, including nested runtime lambdas and dependent scalar defaults. Runtime dependencies still fail stage availability checks.

Six-operator tests exposed a second issue: materialized negative Integer literals use signed hexadecimal spelling, while IIR's canonical-input reader used the unsigned lexical parser directly. The reader now separates the sign, parses the magnitude with the existing bounded parser, and charges negation against the same budget, matching interpreter behavior. Source lexical rules are unchanged.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p iir --lib staging` exited 0 (7 tests; /tmp/luna-deferred-integer-iir.log); `test -p luna --test execution --test elaboration --test instantiation` exited 0 (990 + 427 + 18; /tmp/luna-deferred-integer-regression.log). Tests cover all six comparisons at negative/equal/129-bit values, nested closure arithmetic/comparison, dependent defaults, runtime-dependency rejection, malformed signed constants and budget exhaustion/retry. The initial combined probe /tmp/luna-deferred-integer-comparison.fl returns true/false/true/false. Readable example: development/examples/deferred-integer-comparisons.fl. No commits.

The previously recorded `count == 3` deferral and dependent-default gap is resolved for these scalar expressions. More general computations demanded during type formation still require their own completed symbolic value or a deferred elaboration dependency; this change does not manufacture symbolic results for arbitrary unknown computations.

### Deferred floating-point intermediate computations

Staged-input inspection now traverses checked f32/f64 FloatNegate and FloatBinary nodes, so a floating intermediate under an unresolved comptime branch no longer forces premature invocation. Materialization routes floating arithmetic and floating scalar comparisons through the existing IEEE interpreter instead of the scalar constant folder. The completed argument in this path remains an already supported Bool; no new floating specialization-key representation is introduced.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p luna --test execution --test elaboration deferred_` exited 0 (43 execution + 12 elaboration tests; /tmp/luna-deferred-float-regression.log), and `test -p luna --test instantiation` exited 0 (18 tests; /tmp/luna-deferred-float-instantiation.log). New cases cover true/false deferred branches, NaN equality/inequality, negative-zero division, and divergent f32/f64 rounding at 2^24. Runtime float inputs and runtime dependencies inside a conditional remain rejected. /tmp/luna-deferred-float.fl executes to true/false. Readable example: development/examples/deferred-float-computations.fl. No commits.

Remaining: Float32/Float64 values themselves are not yet supported by into_normal_term or IIR's staged constant key representation. Full ordinary comptime floating input specialization needs a deliberate bit-preserving value identity across NIR substitution, materialization and instance keys; numerical floating equality must not become type equality or erase NaN payload/signed-zero distinctions accidentally.

### Bit-preserving completed floating values and ordinary comptime inputs

NIR now represents completed Float32/Float64 values by their raw IEEE bit patterns with ComptimeValue sort, bounded leaf handling and stable `(float32-bits 0x...)` / `(float64-bits 0x...)` Display. Interpreter value freezing and typed reconstruction, IIR staged constant validation, normal-form input domains and known literal construction now preserve these values. Implementation indexing and effect argument identity recognize the new leaves. No normal-form head is unfolded and no floating numerical comparison is added to PTC equality.

Specialization keys deliberately retain width, signed-zero distinctions and NaN payload bits. Executable scalar comparison remains IEEE: positive and negative zero compare equal; NaN does not compare equal to itself. Runtime references, explicit/defaulted staged values, nested tuple values, forwarded inputs and symbolic normal-form defaults now use this representation consistently.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p nir -p iir -p comptime_interpretor` exited 0 (175 tests; /tmp/luna-floating-input-core.log); `test -p elaboration --test ptc --test effects --test implementation --test effect_rows` exited 0 (99 tests; /tmp/luna-floating-input-theories.log); `test -p luna --test execution --test elaboration --test instantiation` exited 0 (993 + 429 + 18; /tmp/luna-floating-input-source.log). Source tests verify four distinct zero/NaN identity instances, same-bit defaults, payload output, IEEE equality, Float32/Float64 tuple reconstruction, opaque type defaults and width/runtime-dependency rejection. NIR/IIR tests check leaf sort, scope, Display, wrong literal width and exhausted budgets. /tmp/luna-float-input.fl executes and retains its floating bits; readable example: development/examples/floating-comptime-inputs.fl. No commits.

The earlier missing floating specialization-key representation is resolved. This does not implement arbitrary user-defined comptime value identity/Eq/Hash protocols or unrestricted aggregate graph freezing, which remain separate work.

### Freeze computed floating results after staged invocation

Revalidated ordinary Type-dependent floating inputs, then found that a computed default such as `comptime .copy:T = value + 1.0` under T == f64 reached a different value-freezing boundary from literal inputs. The staged-invocation freezer now accepts Float32/Float64 results with exact expected-width checks, including recursive tuple results. This closes expression/default and comptime-call result paths onto the previously added bit-preserving instance representation.

Validation in /home/fin/luna: `python3 -B development/cargo.py test -p luna --test execution --test elaboration float` exited 0 (14 execution + 6 elaboration; /tmp/luna-computed-float-regression.log); `test -p luna --test instantiation` exited 0 (18; /tmp/luna-computed-float-instantiation.log). The new source test checks Type-dependent computed defaults, instance reuse for identical computed results, explicit override separation, a comptime function returning negative zero inside a runtime lambda, mixed f32/f64 tuple results, precision at 2^24 and negative infinity. /tmp/luna-dependent-float-default.fl now executes to 3.0/1.0. Readable example: development/examples/computed-float-defaults.fl. No commits.

Arbitrary user-defined comptime graphs and their identity protocols remain separate unfinished work; these scalar/tuple paths do not authorize a general object-graph freeze.

### Freeze anonymous records as ordinary staged inputs

Completed structural values now have `Term::RecordValue`, distinct from record types and positional tuples. Field labels/order survive freezing, instance keys, scoped substitution, associated normalization, bounded traversal, interpreter reconstruction and S-expression dumps. Property collectors can feed entire nested records to ordinary comptime parameters, including deferred reconstruction and opaque normal-form defaults. Type equality lifts forward through matching record-value labels inside type inputs; PTC still rejects standalone value propositions and does not invert opaque heads. Nominal/resource-bearing objects, pointers and executable values remain outside this freezing fragment.

Validation from `/home/fin/luna` (all exit 0):
- `python3 -B development/cargo.py test -p nir -p iir -p comptime_interpretor`: 178 tests, `/tmp/luna-record-value-core.log`; after adding reconstruction coverage, affected `record` filter: 24 tests, `/tmp/luna-record-value-core-final.log`.
- `python3 -B development/cargo.py test -p elaboration --test ptc --test associated --test implementation --test effects --test effect_rows`: 116 tests, `/tmp/luna-record-value-theories.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test instantiation`: 997 + 430 + 18 tests, `/tmp/luna-record-value-integration.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/staged-record-inputs.fl --interpret run`: returns `(true, false)`, `/tmp/luna-staged-record-example.log`.

Changes remain uncommitted on dev.

### Anonymous record pattern elaboration

Named structural products from property collection now support the ordinary `{field: pattern}` syntax, nested/omitted fields, `let`/`var`, refutable bindings, and boolean-product match coverage. NIR retains a distinct `AnonymousRecord` pattern with ordered names and children; S-expression dumps, type rewriting and visitors preserve it. Interpreter matching validates labels and rolls candidate bindings back through the existing failure path. Program flow, memory/callable projections, ownership transfer, effect usage and cleanup treat fields as anonymous record projections. Coverage specializes the named product without enumerating all values and checks its labels. Rest binding and custom pattern-view protocols remain separate unfinished work.

Checks from `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p nir -p iir -p comptime_interpretor`: 183 tests, `/tmp/luna-anon-pattern-core.log`.
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop`: 1001 + 431 + 210 + 333 tests, `/tmp/luna-anon-pattern-integration.log`.
- After tightening coverage labels, `python3 -B development/cargo.py test -p elaboration --lib coverage::`: 18 tests, `/tmp/luna-anon-pattern-coverage.log`.

The new source tests cover reordered/nested fields, callbacks and pointers, missing/duplicate labels, incomplete correlated matches, omitted-owner cleanup exactly once and double-consumption rejection. Example: `development/examples/anonymous-record-patterns.fl`. Work remains on dev without commits.

### Explicit machine-integer truncation operations

Interpreter and IIR staged folding now execute the existing `IntegerOp::DivideTrunc` / `RemainderTrunc` for fixed-width machine values through `MachineInteger::div_rem_trunc`. Both retain division-by-zero failures, signed minimum divided by -1 overflow, operand widths, truncation toward zero and the dividend sign for remainders. This connects an existing internal operation contract; it does not select source `/` rounding or complete source division's verification/protocol elaboration.

Checks from `/home/fin/luna`, exit 0: `python3 -B development/cargo.py test -p luna --test execution explicit_machine_division` (one matrix test over signed/unsigned 8/16/32/64/128-bit types, both operations, failures and recovery; `/tmp/luna-machine-division.log`); `python3 -B development/cargo.py test -p iir staging::tests::` (10 tests, `/tmp/luna-machine-division-staging.log`). The interpreter `machine` test filter selected zero tests and is not validation evidence. Changes remain uncommitted.

Program flow now verifies these explicit IR operations with bounded interval enclosures outside PTC. A divisor interval must exclude zero; quotient extrema use endpoint pairs on a divisor interval of one sign, while remainder bounds preserve the dividend sign and periodic interior values. Both operations verify that the quotient fits the machine domain, so `MIN % -1` is not accidentally approved. Missing/zero-crossing ranges remain rejected. Small signed interval enumeration independently checks every concrete quotient/remainder; budget failure/retry, unsigned nonzero contracts, signed positive/negative contracts, overflow rejection and materialized execution are covered.

Validation from `/home/fin/luna`, exit 0: `python3 -B development/cargo.py test -p elaboration --lib --test verification` (44 + 35 tests; `/tmp/luna-division-verification-regression.log`); `python3 -B development/cargo.py test -p luna --test execution explicit_machine_division` (3 matrix/integration tests; `/tmp/luna-division-proof-source.log`). Source division protocol selection and relational quotient postconditions beyond available interval evidence remain unfinished.

### Unsigned source division

Connected `/` for fixed-width unsigned integers to the explicit truncating operation, consistent with the `u32` quotient example in `docs/functions/contracts-and-effects.md`. The call/default pipeline admits deferred division inputs, preserves operand evaluation order, checks nonzero divisors and supports comptime defaults/overrides. Program flow retains the valid `quotient <= dividend` (and internal remainder) relation when the dividend is nonnegative and divisor positive, using a fresh result identity and only current-version operand evidence. This proves the documented quotient postcondition without treating division as a polynomial. A divisor-side write cannot transfer the old dividend bound to the new variable value.

Signed source division and mathematical Integer source rounding remain unsupported; `usize` source literals still need target pointer-width information. These are not claimed complete.

Validation from `/home/fin/luna`, exit 0: `python3 -B development/cargo.py test -p luna --test execution --test elaboration division` (6 execution + 2 rejection groups, `/tmp/luna-unsigned-division-regression.log`); `python3 -B development/cargo.py test -p elaboration --lib --test verification` (44 + 35, `/tmp/luna-unsigned-division-flow.log`); `python3 -B development/cargo.py run -p luna -- development/examples/unsigned-division.fl --interpret run` returns `(6, 6)` (`/tmp/luna-unsigned-division-example.log`). The width matrix covers u8/u16/u32/u64/u128, nonzero contracts, runtime check failure/recovery, computed defaults and explicit overrides. Changes remain on dev without commits.

### Unsigned source remainder

Connected fixed-width unsigned `%` to `RemainderTrunc`, including required comptime invocation and deferred defaults. Float, signed machine and mathematical Integer remainder selection remain explicitly rejected rather than falling through to floating division. Nonzero divisor verification is shared with division. Program flow now retains `remainder < divisor` for a positive divisor, alongside the existing nonnegative dividend bound, using versioned operands and a fresh result identity. A later divisor mutation invalidates the relation to its new value.

Validation from `/home/fin/luna`, exit 0: `python3 -B development/cargo.py test -p luna --test execution --test elaboration remainder` (2 execution + 2 rejection groups, `/tmp/luna-unsigned-remainder-regression.log`); integration `python3 -B development/cargo.py test -p luna --test execution --test elaboration` (1010 + 436 tests, `/tmp/luna-unsigned-arithmetic-integration.log`); `python3 -B development/cargo.py run -p luna -- development/examples/unsigned-remainder.fl --interpret run` returns `(2, true)` (`/tmp/luna-unsigned-remainder-example.log`). Width coverage is u8/u16/u32/u64/u128, with zero rejection/recovery, strict divisor postconditions, incorrect-postcondition rejection, staged defaults and explicit overrides. Changes remain uncommitted on dev.

### Quotient multiplication and copied polynomial bounds

Unsigned division now establishes `quotient * divisor <= dividend` from current nonnegative/positive operand evidence. This proves `(n / alignment) * alignment` safe without a redundant multiplication bound. Copying a computed polynomial result now projects existing inequalities through its exact expression equality, retaining only rewrites that decrease nonlinear monomials. This preserves degree-two/three upper and lower bounds in local and result bindings without unbounded equation closure. All facts remain versioned program verification facts, outside PTC. Cross-call examples use explicit entry snapshots (`n'old`) rather than claiming callee exit parameters equal caller inputs.

Validation from `/home/fin/luna`, exit 0: `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources` (1012 + 437 + 210 tests, `/tmp/luna-round-down-integration.log`); `python3 -B development/cargo.py test -p elaboration --lib --test verification` (44 + 35 tests, `/tmp/luna-round-down-verification.log`); `python3 -B development/cargo.py run -p luna -- development/examples/unsigned-round-down.fl --interpret run` returns 18 (`/tmp/luna-round-down-example.log`). Tests include u8/u32/u128 boundaries, caller contracts, degree-two/three result bounds, and rejection of unrelated/mutated multipliers. Work remains uncommitted on dev.

### Splice protocol identity and runtime staging boundary

`LangItem::Splice` now supports atomic, package-qualified trusted source binding.
`luna::protocols::splice::protocol` validates the bound declaration, its unique
Type-valued RuntimeTarget, and ordinary runtime `splice(comptime data: Self)`
method returning that exact associated projection. It rejects a comptime method,
a runtime data parameter, foreign/duplicate identities, receiver methods and
wrong result types. Trait spelling does not designate the language item.
The borrowed protocol descriptor has an S-expression Display.

The source fixture `development/examples/explicit-splice.fl` exercises explicit
trait invocation with distinct staged bool inputs and a tuple runtime target.
Execution regressions check both runtime constructor calls and that a failing
`assumes` inside splice runs only upon runtime invocation, not during instance
construction. This is protocol groundwork, not automatic insertion: source
constant runtime uses still reject until a trusted binding is supplied to the
elaborator and conversion calls are synthesized. Automatic aggregate derivation
and representation validation remain outstanding.

Validation in /home/fin/luna (exit 0):
- `python3 -B development/cargo.py test -p luna --test execution splice_`: 3 passed;
  /tmp/luna-splice-tests.log.
- `python3 -B development/cargo.py test -p intrinsic --test lang_items`: 5 passed;
  /tmp/luna-splice-lang-items.log.
- `git diff --check`: passed. Changes remain uncommitted on dev.

### Automatic module-constant Splice through ordinary trait calls

Added trusted `program::Protocols` inputs to elaboration, single-source frontend
and explicit package-set compilation. Shared Splice shape validation now lives
in `elaboration::splice`; the previous language-item driver wrapper delegates to
it. A runtime reference to a module constant checks the bound protocol, obtains
its ordinary selected method, and passes the frozen source through `finish_call`
as a staged input. Implementation qualification, associated result resolution,
ordinary type/capability requirements and downstream program/instance verification
remain on their existing call paths. There is no name-based trait designation or
conversion-body comptime invocation. Unbound legacy frontends retain rejection.

Added source checks for target-changing conversion, per-use constructor calls,
renamed protocol identity, foreign-package rejection, staged/runtime isolation,
ordinary argument and inferred local contexts, runtime assumption failures,
wrong result types, malformed method phase, structural input specialization and
missing implementation. A separate package test imports only the constant and
uses its provider's explicitly bound protocol identity.

The structural-value test exposed an existing mismatch: implementation candidate
qualification admitted record types, but definition elaboration rejected them
and the index reduced them to wildcard. Record subjects now enter definition
elaboration and have structural keys preserving ordered field labels and child
types. Label copying is budgeted; dumps escape labels. Index tests cover
precision, candidate preservation, label separation and budget recovery.

Current limits: automatic insertion is connected for module constant references,
not all cross-stage value forms; automatic aggregate derivation remains open.
CLI/sysroot automatic protocol installation remains open; explicit driver APIs
carry the binding. No semantics or source syntax were broadened to infer the
protocol from its spelling.

Validation in /home/fin/luna (all exit 0):
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test packages`:
  1018 + 437 + 21 = 1476 passed; /tmp/luna-auto-splice-integration.log.
- `python3 -B development/cargo.py test -p elaboration --test implementation --test capabilities --test associated`:
  40 + 29 + 16 = 85 passed; /tmp/luna-splice-implementation-tests.log.
- `git diff --check`: passed. Work remains uncommitted on dev.

### One-pass Splice source binding in CLI and build-driver entrypoints

`Options::splice_protocol` and CLI `--splice-protocol <trait|package::trait>` now
resolve an explicit protocol selection after HIR construction, before NIR
elaboration. The source selector produces the current package-qualified
DeclarationId in the same compilation. Unqualified selections require a unique
trait; package qualification uses supplied SourcePackage names (standalone input
is `input`). Missing/ambiguous packages and traits, malformed selectors, and
conflicts with explicit driver identities are errors. This does not implicitly
bind any source declaration by its spelling.

`packages::compile_with_splice` exposes the same one-pass selection to build
drivers, including nonzero resolution and semantic package identities. Single
source and sysroot compilation both honor the selector. Existing identity APIs
remain available. Protocol signatures are now validated eagerly at the completed
interface-world boundary, even without runtime constant uses; diagnostics point
to the selected declaration when available. AST/HIR output cannot silently ignore
a requested selector. Options consumers and tests were updated together.

`development/examples/automatic-splice.fl` runs directly through the CLI with
`--interpret run --splice-protocol Splice`. Black-box tests cover correct runtime
results, absence of accidental binding, qualification across sysroot packages,
ambiguous names, invalid signatures, missing/duplicate options, selector errors
and preservation of preexisting output files on failure. API tests also cover
selector/identity conflict and nonsemantic output rejection.

Validation in /home/fin/luna (all exit 0):
- `python3 -B development/cargo.py test -p luna --lib --test frontend_cli --test packages --test execution --test elaboration`:
  8 + 88 + 21 + 1018 + 437 = 1572 passed;
  /tmp/luna-splice-driver-regression.log.
- After adding the direct API rejection check,
  `python3 -B development/cargo.py test -p luna --test frontend_cli splice_selector`:
  3 passed; /tmp/luna-splice-selector-final.log.
- `python3 -B development/cargo.py check -p luna --tests`: passed;
  /tmp/luna-splice-cli-check.log.
- `python3 -B development/cargo.py run -p luna -- development/examples/automatic-splice.fl --interpret run --splice-protocol Splice`:
  `(tuple (tuple (bool true) (bool false)) (tuple (bool false) (bool true)))`;
  /tmp/luna-splice-cli-example.log.
- `git diff --check`: passed. Uncommitted dev changes retained.

Automatic protocol installation from library metadata, cross-stage use forms
beyond module constants and automatic aggregate derivation remain outstanding.

### Splice for comptime invocation results and staged cleanup scope repair

Automatic conversion now covers comptime function invocation results when a
Splice identity is bound. Direct/generic calls enter a staged source context
before result inference or spread routing; selected methods and stored comptime
callables use the shared finishing path. The runtime expected type checks the
conversion result rather than forcing the producer's source type. Type observing,
required comptime and existing staged-input contexts bypass conversion.

Spread and property collectors retain their resolved callee/receiver and finish
input preparation before conversion is inserted. This avoids applying a producer's
collection layout to the synthetic Splice call or replaying callee resolution.
Stored module-constant comptime functions used as callees are invoked in their
own phase; only the result crosses into runtime. Symbolic enclosing staged inputs
remain deferred until concrete instance materialization. Source normal-form heads
are still not ordinary callable producers.

Regressions cover direct/explicit-generic calls, trait methods, defaults and named
overrides, literal/nonliteral spreads, direct/method property collection, stored
comptime functions, large Integer inputs, delayed staged input specialization,
runtime-input rejection, runtime-call rejection and target mismatch. Runtime
traces contain conversion/constructor calls, not comptime producer calls.
`development/examples/splice-comptime-calls.fl` is the executable example.

Resource-checked conversion revealed an existing IIR binding defect: staged local
reconstruction wrapped an existing block in a second block with the same source
range, making ScopeEnd cleanup ambiguous. `iir::staging` now prepends reconstructed
bindings to an existing Block/CallPreparation scope instead of nesting an extra
scope. The strict unique-runtime-boundary check is retained, with a more precise
function/source/boundary diagnostic. Both block and expression-bodied Splice
methods returning a Drop type now execute cleanup; a failing destructor predicate
remains a runtime failure. No resource check was bypassed.

Validation in /home/fin/luna (exit 0):
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop`:
  1022 + 437 + 210 + 333 = 2002 passed;
  /tmp/luna-splice-call-integration.log.
- `python3 -B development/cargo.py test -p iir`: 23 passed;
  /tmp/luna-splice-calls-iir.log.
- Source example was run with `--interpret run --splice-protocol Splice` and is
  also included in the final execution regression. Result: three `(false,true)`
  pairs. CLI run output: /tmp/luna-splice-calls-example.log.
- `git diff --check`: passed. Uncommitted dev changes retained.

Automatic aggregate derivation, broader frozen source-object representations and
remaining higher-order cross-stage use forms are not claimed complete.

### Higher-order comptime producers preserve the final conversion boundary

Added an expression-identity-scoped callee-result context. Immediately invoked
comptime producer results are retained inside the next staged invocation instead
of first demanding Splice for the intermediate comptime callable. All source
arguments keep their existing phase contexts; a runtime factory taking an ordinary
lambda must not accidentally force a comptime lambda. The context is restored on
success and failure and propagates through a block's result without applying to
its statements. Direct, generic, method, spread and extended-property producer
paths share the same result-delivery check.

Source regressions cover nested factories, captured comptime closures, a staged
outer input, trait factories, packet/property collection and block-tail callees.
Runtime traces contain exactly one Splice for each final result and no comptime
producer calls. Tests reject escape of the intermediate comptime callable,
reading future runtime inputs, and bypassing conversion for a comptime-produced
runtime callable without a matching Splice implementation. Runtime factory
callbacks retain ordinary runtime behavior.

The three-level factory probe exposed a separate parser defect: left-associated
`fn() -> fn() -> T` replaced the first result instead of preserving both function
layers. Arrow now has right binding power, consistent with fn_type's recursive
result expression. Other infix operators retain their association and parentheses
still explicitly delimit scope. Parser tests and inline/aliased nested function
source tests cover the distinction; higher-order documentation was clarified.

Validation in /home/fin/luna (exit 0):
- `python3 -B development/cargo.py test -p luna --test execution --test elaboration --test resources --test drop`:
  1026 + 437 + 210 + 333 = 2006 passed;
  /tmp/luna-splice-higher-integration.log.
- `python3 -B development/cargo.py test -p parse`: 37 passed;
  /tmp/luna-splice-higher-parser.log.
- `python3 -B development/cargo.py run -p luna -- development/examples/splice-higher-order.fl --interpret run --splice-protocol Splice`:
  `(tuple (tuple (bool true) (bool false)) (tuple (bool false) (bool true)))`;
  /tmp/luna-splice-higher-example.log.
- `git diff --check`: passed. Changes remain uncommitted on dev.

These changes do not complete automatic Splice derivation, arbitrary frozen object
graphs or every staged-value flow form. No runtime address transfer was introduced.

### Semantic cache prerequisite for nominal comptime inputs

Inspection of docs/comptime/purity-and-normal-form.md (result reuse and Eq/Hash)
and the two-phase trait document showed that directly adding structural nominal
value keys would bypass a required semantic boundary. Nominal Splice input
freezing has therefore not been enabled using raw field equality.

Replaced the comptime_executor placeholder with a bounded pure-query cache:
- Query identity carries package, declaration and operation; ordered inputs are
  bucketed by driver-supplied Hash and compared using driver-supplied semantic Eq.
- Definition, protocol-world and target revisions form an explicit cache scope;
  changes clear entries. Scope and query identity have S-expression Display.
- Shared context accounts for query/key comparison work and protocol execution.
  Eq/Hash key formation requires uncached execution rather than recursive cache
  entry; the guard restores after Result failures. Ordinary nested evaluation
  after key formation is allowed.
- Entries publish only after successful protocol work and evaluation. Failed
  collisions, exhausted budgets, capacity failures and query errors do not add
  entries or erase existing valid results. Hits remain available at capacity.
- Results use immutable shared ownership rather than deep cloning on hits.

Core tests cover custom equality, forced hash collisions, argument order/arity,
query/package/operation identity, each invalidation revision, protocol/evaluation
failure, comparison budget recovery, reentrant key formation, ordinary nested
queries and capacity behavior. A luna integration test executes real comptime
Eq/Hash trait bodies for a nominal Key type. Eq deliberately ignores one field;
the tested pure query observes only the compared field, so reuse is compatible.
Distinct semantic inputs with the same hash evaluate separately. Changing Hash's
body phase to runtime is rejected by the comptime executor and publishes no key.
The new luna dependency is test-only; no production query path was silently
switched to this cache.

Validation in /home/fin/luna (exit 0):
- `python3 -B development/cargo.py test -p comptime_executor`: 6 passed;
  /tmp/luna-semantic-cache-tests.log.
- `python3 -B development/cargo.py test -p luna --test execution pure_query_cache`:
  1 source integration test passed; /tmp/luna-semantic-cache-source.log.
- `git diff --check`: passed. Work remains uncommitted on dev.

Remaining integration after the selector below: immutable nominal value freezing,
semantic key use in instance materialization, and automatic
Splice derivation. Cache callers must establish purity, complete dependencies and
compatibility of the query with input equality; Eq alone is not permission to
reuse a computation that can distinguish the compared inputs.


### Comptime key protocol selection

Added `elaboration::key_protocols` and the trusted language-item consumer
`luna::protocols::select_comptime_key`. Concrete selection uses the shared
implementation/capability world and retains its full scoped evidence. The result
preserves implementation/default method arguments, specialized interface and body
contracts, and remaining capabilities. Its Display is an S-expression. A selected
method is not an invocation authorization or evidence that query reuse respects
input equality.

Source coverage includes generic/default bodies, renamed bound traits, missing
bindings/implementations, runtime phases, exhausted query budgets, mismatched
package/world metadata, unsupported hash signatures, assumptions without bodies,
body type requirements, program contract retention and implementation trait bounds.
The existing semantic collision test now obtains source Eq/Hash through this path.
Hash currently accepts primitive integer result signatures. The execution adapter
below adds full-integer bucket hashing and generic key invocation.

Validation in `/home/fin/luna`, all exit 0:
- `python3 -B development/cargo.py check -p luna`;
  `/tmp/luna-key-selection-check.log`.
- `python3 -B development/cargo.py test -p luna --test execution comptime_key_selection`:
  4 passed; `/tmp/luna-key-selection-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution pure_query_cache`:
  1 passed; `/tmp/luna-key-cache-source.log`.
- `git diff --check` passed. Changes remain in the dev worktree, uncommitted.


### Uncached source key execution

`luna::comptime_keys::Protocol::prepare` now binds both Eq and Hash, proves their
remaining type capabilities and prepares transitive dispatch for their exact
instances. Each operation executes through the new host
`Interpreter::invoke_instantiated` entry, which reuses source FunctionInstance
specialization rather than removing generic binders or requirements. The usual
type and program checks remain active, and failed calls do not poison subsequent
invocations. Default bodies and generic methods calling another comptime trait
are exercised from source.

`Value::immutable_copy_work` checks bounded immutable data representation before
host inputs are cloned. It rejects pointers, storage and callable values, without
claiming to freeze nominal values or establish resource authorization. Invocation
work and input-copy work are charged to the cache context. Hash bucket generation
consumes the complete canonical arbitrary-precision Integer (also supports signed
and unsigned fixed-width results); it never requires a u64-compatible source
result. Eq still decides collisions. The semantic source cache test now uses this
production adapter instead of its local hand-written key protocol.

Still pending: nominal staged-input freezing, resource authorization,
query/equality compatibility evidence and automatic cache integration with staged
instance formation. Prepared adapters are compilation-local owned snapshots;
the owning cache must still invalidate its scope for dependency changes.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution comptime_key`:
  7 passed; `/tmp/luna-key-final-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution instantiated_host_invocation`:
  1 passed; `/tmp/luna-key-host-tests.log`.
- `python3 -B development/cargo.py test -p luna --test execution pure_query_cache`:
  1 passed; `/tmp/luna-key-adapter-cache-final.log`.
- `git diff --check`: passed. Work remains on dev, uncommitted.


### Typed immutable cache snapshots

Added `Interpreter::validate_immutable_input` and
`luna::comptime_keys::Protocol::admit`. Admission checks bounded immutable
representation, closed type shape/scope, exact value layout, nominal formation
requirements/capabilities and normal-form input contracts. It traverses nested
values and Type-valued data, including phantom nominal arguments. It does not
unfold normal-form heads. Computation/effect metadata admission is added by the following increment.

`Snapshot` owns immutable data behind private shared fields with read-only access
and an S-expression Display. Its provenance is tied to one prepared protocol;
foreign/old snapshots cannot be compared through a fresh preparation. No Rust
Eq/Hash implementation gives these snapshots structural semantic identity.
Source Eq/Hash still govern `KeyProtocol<Snapshot>`, and the existing collision
cache test now uses snapshots. Raw adapter calls also validate input formation.

Admission does not establish resource duplication, query/equality compatibility,
nominal staged-input delivery or automatic compiler memoization. Those remain
separate obligations; no structural nominal IIR key was introduced.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution comptime_snapshot_admission`:
  3 passed; `/tmp/luna-snapshot-tests.log` (fields/ranges, tags/payloads,
  phantom/nested constraints, opaque head contracts, malformed/open Type data,
  storage/callables, budget recovery, protocol-world mismatch).
- `python3 -B development/cargo.py test -p luna --test execution comptime_key`:
  7 passed; `/tmp/luna-snapshot-key-regression.log`.
- `python3 -B development/cargo.py test -p luna --test execution pure_query_cache`:
  1 passed; `/tmp/luna-snapshot-cache.log`.
- `git diff --check`: passed. Changes remain uncommitted on dev.


### Effect type admission and semantic key comparison

The NIR interpreter now retains immutable effect declaration metadata and rejects
duplicate identities or non-function operation signatures. Snapshot admission
checks computation-row operation identity, complete type arguments, instantiated
signature shape, requirements and capabilities. It preserves order/repetition and
does not invoke any request or install handlers. IIR execution has no source effect
world, so this metadata-driven admission is not granted there.

The real source Type Eq cache test exposed an existing PTC gap: congruence could
prove commuting rows equal, but distinct closed rows could not be refuted. Added
branch-local computation disjointness based on known constructor results and
label multiplicity intervals. Pending label relations increase upper bounds, not
lower bounds; no failed equality query is treated as inequality. Open/opaque tails remain conservative, since a result may introduce more effect
layers. Raw nested prefixes and proved result aliases are handled below. Nonempty computations are distinct from ordinary constructors;
empty raw wrappers do not acquire such a rule. All proof replay uses the same
scoped evidence. Normal-form heads remain opaque.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p elaboration --test ptc --test ptc_integer --test ptc_kinds --test effect_rows`:
  71 passed (10 effect-row, 45 core PTC, 9 integer, 7 classification);
  `/tmp/luna-effect-ptc-regression.log`. The finite row oracle now checks equality
  and inequality plus proof replay for 729 pairs; added open-tail regressions.
- `python3 -B development/cargo.py test -p luna --test execution effect`:
  54 passed; `/tmp/luna-effect-admission-source-regression.log`, including three
  effect snapshot/cache tests and existing direct/async/generic effect behavior.
- `git diff --check`: passed. Work remains uncommitted on dev.

Automatic compiler-cache installation, cross-stage nominal keys and resource
permission for frozen values remain pending; this change strengthens the Type
metadata and semantic comparison path needed by that integration.


### Scoped computation result views

PTC effect-row equality and inequality now share a bounded view of explicit
prefixes and branch-proved result aliases. The view borrows operations from the
immutable term table, preserving their identity and occurrence order; it does not
create a new canonical type or unfold any normal-form body. Multi-hop aliases
compose with the existing proved-different-label commutations. Empty wrappers
collapse semantically, and nonempty cyclic aliases remain pending. Proof replay
uses the same branch classes; no view is reused across assumption changes.

Source generic calls now discharge contracts requiring `#write T` to match
`#[read, write] bool` under `T == #read U` and `U == bool`. Missing/wrong premises
and an extra read layer reject. Added the runnable
`development/examples/effect-result-aliases.fl` example.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p elaboration --test ptc --test ptc_integer --test ptc_kinds --test effect_rows`:
  75 passed (14 effect-row, 45 core, 9 integer, 7 classification);
  `/tmp/luna-effect-tail-ptc-regression.log`.
- `python3 -B development/cargo.py test -p luna --test execution effect`:
  55 passed; `/tmp/luna-effect-tail-source-regression.log`.
- `python3 -B development/cargo.py run -p luna -- development/examples/effect-result-aliases.fl --interpret entry`:
  `(bool true)`; `/tmp/luna-effect-tail-example.log`.
- `git diff --check`: passed. Changes remain on dev, uncommitted.

General higher-rank/binder-sensitive effect equivalence and semantic nominal
staged-input/cache integration are not completed by this increment.


### Reusable prepared pure invocation

Added `luna::memoization::Invocation`, exported as
`luna::invocation::MemoizedInvocation`. This executes checked source functions,
not a caller-supplied evaluation closure. It owns a fixed package/dispatch world,
checks the ordinary comptime phase, trusts the documented pure promise, proves
concrete generic type contracts, and prepares typed key protocols per argument.
Normal-form heads and unmarked/runtime functions cannot enter this cache.

Compatibility decision: source Eq/Hash remains required, but result reuse also
requires identical immutable input representation. Thus an Eq implementation that
ignores a field cannot hide that field from an inspecting query. This is a stronger
query dependency discriminator, not structural replacement of semantic Eq.

Added optional publication to the executor cache. Unsupported successful result
representations return uncached; capacity-bound misses also run uncached. Execution,
budget and actual result formation-contract failures publish nothing. The existing
strict cache entry keeps its prior capacity error behavior. General dependent input
binding, resource authorization and automatic installation across compiler queries
remain separate work.

Validation in `/home/fin/luna`, exit 0:
- `python3 -B development/cargo.py test -p luna --test execution memoized_invocation`:
  5 passed; `/tmp/luna-memoized-invocation-tests.log` (ignored fields, generic/mixed
  inputs, type/program contracts, phase boundaries, capacity zero, world isolation,
  unsupported successful results and invalid returned Type formation).
- `python3 -B development/cargo.py test -p comptime_executor`:
  8 passed; `/tmp/luna-memoization-cache-regression.log`.
- `python3 -B development/cargo.py test -p luna --test execution pure_query_cache`:
  1 passed; `/tmp/luna-memoization-source-regression.log`.
- `git diff --check`: passed. Changes remain uncommitted on dev.


## Iteration protocol integration gap (2026-10-14)

The HIR `For` node preserves the iterable, pattern, guard, label identity and body. Executable elaboration is still rejected in `program.rs`. Do not wire the historical `library/builtin/iterator.fl` `Iterator.next` API directly: current `docs/control-flow/basic.md` names `Iter` and `IntoIterator`, while the author's intended iterator architecture uses inline functions plus inline effects. The callable protocol signatures, language-item selection and yield/handler mapping need a coherent contract before source `for` can be lowered. This is a semantic integration gap, not evidence that executable iteration is complete.
