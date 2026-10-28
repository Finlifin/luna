# Flurry implementation map

The active objective is implementation of the language defined by `docs/`. Individual runs close bounded engineering tasks; they do not close this objective. This map tracks dependency order and the evidence needed before a feature can be called implemented. It is an implementation plan, not a replacement language specification.

## Current executable boundary

Frontend artifact reuse (2026-10-23): `CompilationCache` now provides bounded process-local LRU reuse of completed artifacts, resolved protocols and source observations. Full requests plus cwd key entries; source membership/content and output-alias checks run on hits. Stale entries are evicted before recompilation, read/protection failures do not fall back to old results, and failed compilations are not retained. Fresh results are revalidated before insertion. No execution results, active inference state, persistent cache or daemon protocol are included. Tests prove immutable Arc reuse, source/dependency invalidation and recovery, directory changes, conservative key separation, LRU/zero-capacity behavior, and output hard-link rejection after a prior successful compile. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p luna --test compilation_cache --test packages` exited 0, 5 + 46 passed (`/tmp/luna-tools-fb7s59ay/result.json`).

Discovery revalidation (2026-10-23): observations now retain each scan root, canonical root, explicit package exclusions and relative membership. `changed_discovery()` reuses source discovery to report sorted added/removed files and directories by resolution ID; missing roots remove recorded members, while retargeted roots and scan failures reject. Source content checks remain separate. Tests cover new modules, non-source files, empty-directory changes, dependency isolation, root deletion/restoration, unsupported symlinks and same-content root retargeting. The process envelope carries these discovery inputs, and changes have S-expression Display with lossless native path bytes. In `/home/fin/luna`, the check wrapper with timeout 300 and `test -p luna --test packages --test frontend_cli` passed 46 + 172 (`/tmp/luna-tools-ykv8f9sk/result.json`); after extracting shared path formatting, `test -p luna --test packages compilation_` passed 5 (`/tmp/luna-tools-yixqtjz9/result.json`). Neither revalidation API is an atomic filesystem transaction or a complete cache-key validator.

Raw input revalidation (2026-10-23): observed compilation now uses a recording source loader, retaining the original UTF-8 text before SourceMap BOM/CRLF normalization and reusing the first loaded text for repeated filename requests. `SourceInput::raw_text` and `CompilationInputs::changed_sources()` enable exact file-byte comparison; deletion is a change, other read failures remain errors. The process envelope adds `raw-text` only when it differs from normalized `text`. Directory membership and complete cache keys still need separate revalidation. Normal compilation does not install the recording loader. BOM/CRLF, normalization-only edits, restoration, deletion, invalid UTF-8 replacement and read failures are covered. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p luna --test packages --test frontend_cli` exited 0, 45 + 172 passed (`/tmp/luna-tools-eip59t47/result.json`).

Process input export (2026-10-23): `--packages-v1 --with-inputs` wraps the selected operation result and its same-invocation source observations in a version-1 S-expression envelope. It shares compilation and selector resolution with existing actions, retains source-output alias checks and publishes only after the action succeeds. Duplicate flags and use outside package-set mode reject as usage errors. CLI tests cover HIR, NIR, reference execution, dependency snapshots, atomic preservation after frontend/action failure, source overwrite rejection and successful recovery. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli` exited 0 (`/tmp/luna-tools-si058z77/result.json`). The anar checkout location has been requested; its absence does not block independent luna implementation.

Build input observations (2026-10-23): `packages::compile_with_inputs` returns the NIR artifact with the source-map text and package/path identities consumed by that invocation, plus scanned directories for membership invalidation. Existing entry points retain their allocation behavior by leaving observation disabled. Directory ownership uses the same discovery path as compilation; text observations do not reread files afterward. This is not a complete cache key or an atomic filesystem snapshot; configuration, dependencies, target/compiler identity and input revalidation remain build-tool responsibilities. No confirmed anar checkout was found in the inspected environment; this provides the luna-side Rust API and package-set `--with-inputs` export. See `development/package-cli.md`. In `/home/fin/luna`, the check wrapper with `test -p luna --test packages` passed 44 (`/tmp/luna-tools-ca7wi2i0/result.json`); package CLI regression evidence is `/tmp/luna-tools-cwppg93t/result.json` for `test -p luna --test frontend_cli explicit_package_cli`, exit 0. Both used timeout 300.

Closure rewrite scheduling (2026-10-23): the NIR work stack carries separate capture and shared-body steps, preserving capture-type/initializer order and delaying `Arc::make_mut` until earlier captures succeed. Regression checks cover failure without cloning or touching later captures, recovery, and 157 nested shared closure bodies on a 2 MiB worker while preserving the original template. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p nir --test type_rewrite --test copy_budget` passed 8 + 13 (`/tmp/luna-tools-41vtyoi5/result.json`); the same wrapper with `test -p luna --test captured_memory --test function_type_dependencies --test cmtt_methods --test drop` passed 94 + 33 + 142 + 987 (`/tmp/luna-tools-vm072yon/result.json`).

NIR traversal extension (2026-10-23): the explicit work stack also covers conditions, returns, calls, aggregate fields, closure type components, effect resume/run/computation and request children. Effect identity/signature annotations retain their prior rewrite order. A call-order regression deliberately reverses parameter slot numbers while checking source-order visitation; mixed conditional/aggregate/return chains exercise the admitted copy depth on a 2 MiB worker. Specialized error-conversion and match metadata handling remains intact. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p nir --test type_rewrite --test copy_budget` passed 7 + 12 (`/tmp/luna-tools-75528wci/result.json`); the same wrapper with `test -p luna --test error_surface --test effect_memory_transfers --test quote_functions --test cmtt_methods` passed 117 + 85 + 103 + 142 (`/tmp/luna-tools-z7p2nrjs/result.json`).

NIR rewrite traversal (2026-10-23): ordinary block/preparation scopes, unary nodes, binary nodes and bit windows now use an explicit mutable-child work stack, preserving expression/type callback preorder, pruning and error short-circuiting. Specialized nodes whose metadata depends on rewritten children retain their dedicated traversal; this is not a claim that every NIR traversal is stackless. The small-worker copy test now mixes unary, binary and preparation scopes at the admitted depth. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p nir --test type_rewrite --test copy_budget` passed 6 + 12 (`/tmp/luna-tools-dc60w_a6/result.json`), and the same wrapper with `test -p luna --test quote_functions --test cmtt --test drop` passed 103 + 523 + 987 (`/tmp/luna-tools-yulwohar/result.json`).

Workspace integration evidence (2026-10-23): in `/home/fin/luna`, `python3 -B development/check.py --timeout 900 -- test --workspace --no-fail-fast` completed 197 groups with 8,032 passed, 2 failed and 11 ignored (`/tmp/luna-tools-duxn_obn/result.json`). Both failures were stale assertions: contextual constructor copy used the former literal depth limit, and raw contracted trait-value transport expected late InvalidCallPlan rather than the earlier Unsupported rejection. After correcting those assertions, `test -p elaboration --lib` passed 217 (`/tmp/luna-tools-0it7eazo/result.json`) and `test -p comptime_interpretor --test runtime_function_identities` passed 13 (`/tmp/luna-tools-oyu3ekfp/result.json`), using the check wrapper with timeout 300. These are targeted retests, not a second full-workspace pass. The real sysroot smoke command `python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/library-smoke.fl --sysroot library --interpret run` still exits 1 at multi-package HIR resolution (`/tmp/luna-tools-_yky17qz/result.json`): unresolved panic, String, Any and allocation/formatting/I/O hooks, as already documented in `library/README.md`. Library execution remains incomplete.

Quote expansion copy-depth integration (2026-10-23): the supported 32-frame quote chain generates more than 128 nested IR nodes because argument preparation, callee parameters and lexical cleanup retain separate scopes. NIR copy preflight now bounds the complete tree at 160 levels, while individual type terms remain capped at 128. Unary expression rewriting uses the small traversal frame rather than retaining the large operation dispatcher. A 2 MiB worker regression checks admitted-depth cloning, rewriting and destruction, rejection beyond the boundary, and recovery. This does not remove expansion-cycle checks or relax the 32-frame quote limit. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p luna --test quote_functions --test instantiation --test drop` passed 103 + 52 + 987 tests (`/tmp/luna-tools-ev3zsbb5/result.json`); `python3 -B development/check.py --timeout 300 -- test -p nir --test copy_budget --test type_rewrite` passed 12 + 5 (`/tmp/luna-tools-wd3j51la/result.json`). No new full-suite pass is claimed.

As of 2026-10-22, the connected compiler is substantially beyond the original first-order NIR milestone. Source compilation includes multi-package inputs and explicit dependency/protocol binding (`compiler/luna/src/packages.rs`), also exposed through the versioned `--packages-v1` process interface (see `development/package-cli.md`), generic and dependent calls, staged values, closures, trait dispatch, error/effect handling, CMTT and inline expansion. The reference execution paths instantiate concrete programs, perform their selected verification/lowering steps and execute them; they are not native code generation. Resource-checked execution has a distinct explicit protocol entry (`interpret_with_protocols`), so passing ordinary execution tests does not by itself establish ownership correctness.

Program verification is implemented separately from PTC: memory/resource analysis, runtime specifications, bounded polynomial range/sign/nonzero evidence and runtime checks have dedicated paths. IIR contains staging, cleanup and effect representations and transformations, rather than only a skeleton. Coverage is partial throughout; the presence of these paths or passing tests must not be read as language completion.

Concrete outstanding source boundaries include general ordinary `for` protocol/effect elaboration and empty `tuple_enumerate` body checking in runtime, open-function and generator contexts. Ordinary `for` interface signatures and direct-Iter/conversion selection are still unconfirmed: `docs/control-flow/basic.md` fixes protocol names only; `development/iteration-protocol-proposal.fl` is explicitly a proposal, and the legacy `library/builtin/protocols/iterator.fl` next-based protocol is not the intended lowering contract. The author has been asked to settle that specific interface; do not infer approval from silence. Signed `/` and remainder require a bound protocol; their absence must not be repaired by silently choosing a rounding policy. Explicitly rounded integer IR already executes and is verified. End-to-end native generation, complete standard-library coverage and full anar integration still require their own completion evidence.

Dependent callable types now bind explicit comptime Type/value inputs with lexical telescope scope, retain normal-form heads, and validate callable-local domains during comptime type-value inspection. Comptime closures can capture these callables. Runtime closures now retain finite sets of named-function and trait-method template identities and specialize their calls, including runtime if/match selection, mutable cells, cyclic local aliases, declaration arguments and nested closures. Domain analysis takes a bounded union of recognized initializers and assignments, then removes cells and dependent aliases with unknown producers. Actual values stay in runtime cells: construction snapshots and later private capture writes remain distinct. Selection effects run at initialization; internal TemplateEqual nodes compare the captured identity during dispatch without exposing source-level function equality. Trait identities retain interface, subject, member and method inputs independently of selected implementation-body identity; shared code cannot merge distinct method identities. Default methods and method value parameters participate in the same path. Finite runtime staged method identities retain interface preconditions, result postconditions and entry-value snapshots through specialization and nested capture. Raw NIR execution rejects stored methods whose preconditions would be erased; direct calls retain their contract checks. Comptime factories and generic associated initializers can transport contracted runtime staged method identities without executing runtime bodies. Capture-domain collection follows associated-value resolution so runtime selection retains each interface instance and its entry domain. Ordinary and comptime method values with preconditions remain restricted pending general callable program contracts. Factory integration verification (cwd `/home/fin/luna`): CMTT 523, elaboration 448, execution 1,365 and dependent-callable 27 tests passed in `/tmp/luna-tools-slq6bwgw/result.json`; that run stopped on obsolete expectations/reserved-word fixture names in `function_type_parameters`. After correcting those fixtures, `python3 -B development/check.py --timeout 300 -- test -p luna --test function_type_parameters --test nominal_value_parameters --test resources` exited 0 with 55 + 265 + 357 tests (`/tmp/luna-tools-mwedoycs/result.json`). Total distinct passing cases: 3,040. Contract regression evidence (cwd `/home/fin/luna`): `python3 -B development/check.py --timeout 300 -- test -p luna --test execution --test cmtt --test resources --test function_type_dependencies` exited 0 (`/tmp/luna-tools-swsbnkfe/result.json`, 2,269 tests); after adding the entry-snapshot mutation case, the focused `function_type_dependencies` rerun exited 0 with 25 tests (`/tmp/luna-tools-btgzynvn/result.json`). The formerly ignored `runtime_closures_capture_dependent_callable_templates` acceptance test now executes and checks that the logical capture survives in IIR. Closed immutable local pointer aliases now contribute template loads and stores to domain analysis. Direct pointer callees are read into fresh logical-callable temporaries before argument evaluation; argument writes cannot retarget the current invocation, and normal arguments still execute once. Returned, captured, retargeted or otherwise escaped pointers remain excluded until broader alias analysis is available. Arbitrary higher-order propagation remains incomplete; erasing captures or inserting dummy values is not a valid implementation.

Named runtime templates with explicit comptime inputs now retain declaration program contracts through runtime selection, nested captures and comptime factories. Declaration arguments distinguish target domains; entry snapshots and postconditions survive specialization. Ordinary non-staged callable contract erasure remains rejected. The resource-checked factory regression now rejects the illegal call during residual program-contract analysis, before interpretation. Explicit `assumes` supplies unknown entry evidence and retains its runtime failure check; arbitrary capture-domain correlations are not yet statically discharged. Validation in `/home/fin/luna`: elaboration 448 and dependent callables 29 passed (`/tmp/luna-tools-hyygbzr4/result.json`, subsequently stopped on an obsolete instantiation-only rejection expectation); execution 1,365, function-type parameters 55 and resources 357 passed after updating that expectation (`/tmp/luna-tools-xonx8gr1/result.json`). The final dependent-callable run including resource-checked cases passed 29 tests (`/tmp/luna-tools-ik5mbrcs/result.json`). Commands use `python3 -B development/check.py --timeout 300 -- test -p luna --test <group>`; 2,254 distinct cases passed.

Program-flow call analysis now resolves `runtime_instances` by full declaration/argument identity through `Package::runtime_body`, uses the residual parameter bindings and contracts, and does not reapply completed substitutions. Missing or duplicate direct targets reject rather than silently losing their contracts; lookup work is budgeted. A source-to-IIR regression checks two instances of one declaration inside a captured callback, static rejection of a false precondition, duplicate-instance rejection and recovery after exhausted fuel. Resource-checked source execution now rechecks residual program contracts, fixed-width arithmetic and bit index/window/update/write endpoints before publishing cleanup schedules. Regressions remove checked assumptions from already-elaborated arithmetic and zero-width window bodies, verifying that later transformations cannot rely solely on the earlier source check. The low-level cleanup planner remains available for injected-failure interpreter tests and does not claim source certification. Reference execution without resource protocols retains runtime checks. Bodies requiring higher-order or effect-result cleanup contexts now undergo residual direct-contract checking before the contextual-schedule skip. Context-dependent callback contracts and captured target/argument correlations still need broader static integration. Validation: in `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p luna --test function_type_dependencies --test elaboration --test execution --test integer_contracts --test resources` exited 0, 2,212 tests (`/tmp/luna-tools-0qe5fy1w/result.json`).

Residual-gate integration validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 300 -- test -p luna --test function_type_dependencies --test resources --test drop --test captured_memory` passed 31 + 357 + 987 + 94 tests (`/tmp/luna-tools-80pbh4i2/result.json`). After adding explicit-assumption failure evidence, the dependent-callable, CLI and source-test groups passed 31 + 170 + 6 (`/tmp/luna-tools-jgp3mjhq/result.json`). Total distinct cases: 1,645. The earlier low-level planner gate failed intentional contract-rejection cleanup tests; the final source-checked entrypoint boundary preserves those tests without weakening the source gate.

Higher-order residual-gate regression: a wrapper accepting an ordinary callable and invoking a locally stored staged target must establish the target precondition; adding the wrapper entry requirement discharges it. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 300 -- test -p luna --test function_type_dependencies --test resources` passed 32 + 357 (`/tmp/luna-tools-n3gcja34/result.json`); default calls 33, extended calls 35, CLI 170 and function-type parameters 55 passed (`/tmp/luna-tools-rbv9g7sk/result.json`). Total: 682 cases.

Residual arithmetic/endpoint validation: in `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p luna --test integer_contracts --test resources --test frontend_cli --test function_type_dependencies` exited 0, 573 cases (`/tmp/luna-tools-psi1wnad/result.json`). The expanded pointer-template ordering regression now budgets 2,000,000 steps for the additional residual branch/snapshot range analysis; its one-step failure and subsequent successful recovery remain checked. Temporary cost instrumentation was removed.

Residual program-contract diagnostics map generated source identities back through the graph expansion table before reporting checked-execution failure. A regression compares raw flow failure on an expanded call with the source-facing driver diagnostic and rejects leaked generated offsets. In `/home/fin/luna`, `python3 -B development/check.py --timeout 300 -- test -p luna --test function_type_dependencies --test frontend_cli` exited 0, 203 tests (`/tmp/luna-tools-aituqcsk/result.json`).

Runtime-body lookup now rejects duplicate/malformed selected residuals, non-runtime residual phases, remaining staged inputs, mismatched parameter arity, duplicate source/quote declarations and wrong source declaration argument counts. An invalid matching instance never falls back to source. `runtime_body_checked` preserves lookup diagnostics for imported callable evidence; the existing optional lookup remains a wrapper. Validation in `/home/fin/luna`: the captured-memory, drop and dependent-callable groups passed 94 + 987 + 33 (`/tmp/luna-tools-pz38ofaz/result.json`); two resource diagnostics initially regressed and were corrected through the checked lookup API. Resources then passed all 357 (`/tmp/luna-tools-0elelh1c/result.json`). `python3 -B development/check.py --timeout 300 -- test -p nir --test runtime_instances` passed all 3 focused cases (`/tmp/luna-tools-14bh19kz/result.json`), covering selection identity, malformed shadowing, precise diagnostics and recovery. Total distinct passing cases: 1,474.

`iir::template_storage::TemplateStorage` now provides the bounded finite-target representation contract, keeping the logical callable type separate from its tag storage and requiring identity translation across tables. Its public constructor resolves targets against source-package declarations, checks runtime phase and declaration argument domains, and substitutes the actual signature before admitting a target. Invocation preconditions and implementation-selection obligations remain separate checks. Source regression `template_storage_checks_source_declarations_and_recovers_after_rejection` covers Type/value arguments, invalid domains, mismatched signatures, missing/duplicate declarations and budget recovery. IIR now carries these identity tables separately from executable instances; the reference evaluator stores the real selected identity in capture cells. Validation rejects dangling or retyped identities, duplicate tables and residual runtime calls with staged inputs. Calls through finite-domain captures specialize before execution; branch dispatch retains runtime selection and ordinary argument evaluation counts. Direct-call specialization can expose higher-order inputs, but open higher-order entries, escaped-address domains and native layout lowering remain pending. Resource-checked execution observes TemplateEqual operands without transferring ownership or requiring Copy. Even singleton-domain dispatch retains an observational read so known target identity cannot authorize use of a moved capture; source rejection tests cover both singleton and multi-target domains. pointer loads/replacement still acquire actual Copy evidence. Source tests cover local-pointer writes and capture snapshots through interpret_with_protocols, and verify that raw execution does not bypass the pointer permission boundary. Do not replace the logical environment type with `u64`: type identity and type observation must retain the source callable type.

The chronological entries below record superseding changes. The dependency table is a navigation index, not a complete feature checklist; old prose saying a now-tested subsystem is wholly pending must not drive duplicate implementation.

The older `middle` type objects and disabled backend/typeck directories are implementation inputs to evaluate, not semantic foundations to preserve unconditionally. In particular, fixed-width literals, pure-head unfolding, and conflating runtime identity with type-core identity cannot become compatibility requirements.

## Dependency groups and exit evidence

| Group | Required behavior | Prerequisites and exit evidence | State |
|---|---|---|---|
| Source and diagnostics | Lexer, parser boundaries, source spans, grouped patterns, attributes, complete AST/HIR payloads, explicit unsupported diagnostics | CLI source fixtures, rejection cases, structured IR dumps, deterministic diagnostics | CLI/display and literal frontend established; trait required-function/associated-value declarations and file-qualified impl scopes retained in HIR; other syntax incomplete |
| Binding and organization | Package/module/static-container identities, imports, visibility, signature collection, recursion and query dependencies | Multi-file fixtures with identity and negative-query invalidation checks | Multi-package compilation with explicit dependency edges and protocol selectors is connected, alongside file/directory/inline modules, visibility and imports. See packages.rs, modules.rs and frontend_cli.rs tests. Incremental discovery invalidation and complete build-tool integration require separate evidence |
| Stable semantic representation | Immutable type core plus scoped substitution; rigid parameters, runtime binders and inference variables distinct; normal-form heads opaque | Representation tests: shared core/different environments, repeated identities, nested substitution, capture avoidance, rollback; S-expression Display | NIR type core/reference foundations, runtime abstraction and use-site scope checks and dependent function-signature instantiation implemented with 24 tests; dependent contract solving remains pending; monomorphic HIR elaboration now produces typed expression trees |
| Elaboration engine | Bidirectional expected information, candidate-local transactions, wait/wakeup obligations, dependency-aware failure and diagnostics | Expressions/calls checked through CLI; no success placeholders; candidate rejection/retry and cancellation fixtures | Monomorphic CLI path uses scoped PTC type equality; candidate region supports structural matching of two open substitutions while retaining original equality obligations, and joins whole-call type inference, ordered dependent substitution, PTC facts, original-relation obligations and directed wait/wakeup; Type-parameter generic calls and type-only requires on functions/structs/enums connected to source, including deferred nominal formation obligations and definition-scoped constrained defaults; transactional nested argument retries and early result-context propagation connected; partial callback contexts support lambdas, generic function values and nested generic calls through blocks/if/match/when, with body results feeding inference; proof-backed structural views connect constrained tuple/record/enum operations, coverage and higher-order calls/lambda/function-value contexts; function signature dependencies now resolve on demand with cycle/depth checks; observed calls retain definition-scoped deferred contracts and declaration obligations survive speculative rollback; general partial-type continuations and comptime scheduling integration pending |
| Comptime and freezing | Interpretation and invocation with common semantics; stable residual definitions and legal delays; type comptime completion at IIR entry | Definition-to-instance tests, required-computation failures, no duplicate impure execution, opaque NF identity tests | Concrete NIR interpreter kernel and source execution tests; explicit comptime/pure modifiers survive declaration scanning and lowering, NIR function phases are displayed and checked, and a dedicated first-order comptime interpreter entry validates call phases and enforces execution budgets; closed required computations now form aliases, explicit function signature types and computed BitVec widths by preparing reachable comptime bodies on demand; successful alias completion is reused per declaration, dependency cycles and shared execution budgets are checked, and unresolved trait-world dependencies explicitly block execution; first-order Type-parameter instances now substitute signatures and bodies for required computation and nested comptime execution; type-only declaration contracts now prepare on demand and discharge instance obligations in their recorded definition/caller scope before execution, including independent contract-operand formation checks; ordinary optional defaults now prepare on demand with definition-scoped prior-parameter bindings, and comptime defaults execute through the ordered call plan only when omitted; generic default binders no longer masquerade as caller inference evidence; Type-parameter/Type-result normal-form heads now parse, resolve, check their bodies/contracts and export opaque head-plus-argument types with a non-invocable NIR definition phase; canonical arbitrary-precision Integer head arguments now evaluate closed inputs, retain symbolic declaration inputs and coexist with Type arguments without unfolding; Bool and fixed-width signed/unsigned integer inputs now share stable-value checking and symbolic forwarding, including range checks and mixed input ordering; prior Type inputs can now determine later scalar/Type input types, with ordered substitution and proof-backed symbolic forwarding; owned NIR now exports ordered head input descriptors with explicit Type/value roles, declaration/binding identities and dependent input types, validated against body metadata before interpretation; concrete Type values now check applications of known heads against those descriptors, including dependent builtin input types and integer ranges; arbitrary dependent type expressions and other value-parameter normal forms, trait-world waits, general automatic invocation/splicing, higher-order phase types and loose residual checking remain pending; comptime_runtime now provides canonical arbitrary-precision Integer parsing, bounded arithmetic and checked fixed-width conversion, verified independently up to 1024-bit operands; concrete source comptime Integer values now reach NIR/interpretation with exact add/subtract/multiply/negation/comparison, literal matching and shared operation budgets; runtime storage phase inspection follows actual aggregate fields without unfolding opaque heads; concrete Type values now preserve primitive/nominal/structural/opaque identities through calls and substitution, and the explicit BitVec type-value constructor evaluates Integer widths without allocating bit storage; source unsuffixed integer literals now splice into checked fixed-width signed/unsigned proxies through 128 bits, with deferred generic defaulting and contextual patterns/defaults; exact decimal literal components and direct finite f32/f64 proxies now connect source context, generic defaulting, defaults and patterns without double rounding; general Float arithmetic, float overflow policy, target pointer-width plumbing, LiteralInto suffix protocols, full comptime phase authorization, residual scheduling and invocation pending |
| PTC and trait protocols | Type-only atomic propositions and theory solvers; implementation selection, associated declarations, capabilities through elaboration | Atom-by-atom accepted/rejected/unknown fixtures and solver isolation; actual LiteralInto/pattern protocol calls | Type equality with explicitly registered nominal constructor contracts (including inference, proof replay and concrete countermodel validation), closed type classification, integer difference constraints and mixed Boolean coordination implemented with scoped proofs/replay and validated type/integer countermodels; integer arithmetic capacity is bounded with Unknown on exhaustion; source type equality/Boolean requires connected; implementation indexing and tri-state selection connect candidate-local Type inference, proved/refuted requirements and replayable type-applicability evidence, with versioned world snapshots and fixed-interface package-ownership admission; type-only same-precision coherence now checks alpha-renamed heads, complete interface arguments and combined requirements without caller assumptions, with independent proof/report replay and world freshness; generic source trait implementation Type telescopes and type-only requires now reach NIR, definition-side member checks, parent obligations and statically selected method substitutions; compile-time concrete dispatch materialization follows function instances, closures, pattern computations and nominal capabilities and feeds a validated interpreter lookup table; generic implementation execution is connected for this Type-only fragment, while phase/environment delivery remains pending; concrete source trait/inherent static methods now check contracts and dispatch explicit interface views through real candidate resolution, including explicit Self substitution and subject-specialized default bodies, with by-value receiver calls/capture, evaluate-once ordering and definition-fixed same-interface/parent-interface calls inside default bodies and independent concrete parent implementation obligations, unambiguous inherited projections and default-body implicit capability lookup and ordinary generic-function positive trait bounds with caller-side obligation checks and generic nominal-type capability formation checks; generic inherent methods now use a separate name-filtered index with type evidence replay, coherent specialization and inherited head parameters/contracts, connected through function instances to execution, including named/optional inherent method arguments through the shared ordered call planner; trait calls now preserve interface entry names/defaults and definition-local preparation frames independently of selected implementation locals, across concrete/generic/default/parent calls; positive implementation trait bounds now qualify candidates before ranking using scoped assumptions, parent implications and recursive immutable-world resolution, with cycle/abstract absence pending and compile-time dispatch dependencies; complete Type-argument interface identities now survive NIR substitution, parent evidence, candidate queries and concrete dispatch, with NIR-only parameterized-interface execution tests; Type-parameterized trait declarations, implementation heads, explicit interface views, bounds and inherited/default member calls now also execute from source, including rigid symbolic queries and interface-only implementation parameters; trait Type-parameter bounds and type-only requirements now check interface formation, provide scoped default-body assumptions and materialize concrete dependencies; Self-dependent trait type requirements now check implementation subjects and concrete dispatch materialization, and export bounded scoped type guarantees through declared capabilities and parents into function/default-body checking; Self-dependent parameter capabilities now defer to implementation-subject checking, substitute fully in materialization/runtime validation, and propagate through parent/parameter edges for generic member lookup and bound forwarding with complete-source budget checks; value/dependent interface arguments, pointer adaptation, negative trait predicates and associated substitution remain pending |
| Functions and runtime types | Parameters/defaults, contracts, closures, dependent environments, user types, rows and dispatch | Call-order/evaluate-once tests; layout versus environment separation; all explicit lowering forms retained | Implemented paths include extended/default/dependent calls, closure environment parameters, nominal value parameters and higher-order dispatch; see corresponding luna test groups. General completeness remains unproven |
| Resources and runtime specification | Once/Clone/Copy, memory views, affine operations, safe-mode predicate limits and runtime checks | Positive/negative alias and lifecycle fixtures, higher-order effect propagation, cleanup paths; separate program-verification layer | Program verification, memory flow, moves, cleanup and runtime contracts are implemented in verification/ and exercised by resources, captured_memory, drop and staged_aggregate_contracts tests. Full resource semantics remains incomplete; PTC stays type-only |
| Effects and staging | Distinct error/effect ABIs, handlers and runtime instances, deterministic inline effects; rank-1/two-stage/single-splice CMTT and quoted arms | Semantic execution tests for handler cleanup, evaluation counts, staging boundaries and hygiene | Error/effect handling, CMTT, quote functions and inline controls have executable paths and dedicated tests. Empty tuple enumeration remains rejected; complete ABI/backend delivery is not established |
| Projection and synthesis | Pipeline projection, typed intrinsic/asm access, BitVec values/locations/unsafe construction, legal baseline on synthesis Unknown | Per-stage projection tests, bit width/ordering tests, baseline-versus-optimized behavioral checks | Arbitrary-width BitVec value kernel implements checked literal conversion, bitwise operations, concatenation, slices and widening with bounded work and S-expression Display; fixed-integer source views, concrete-width type identity, contextual integer literals, eager bitwise operations, concatenation, constant indices/slices, equality, checked widening/unsigned conversion and exact-width signed/IEEE reinterpretation now reach NIR execution; explicit builtin BitVec construction accepts checked literal and closed computed Integer widths without unfolding normal-form heads; dependent width/value-argument elaboration, runtime endpoint verification, general location mappings, unsafe construction and synthesis remain pending; static local/field bit writeback now preserves RHS snapshots and intervening writes for fixed integer/IEEE float/BitVec storage; float views preserve raw IEEE special-value bits; static segmented patterns retain radix/width, typed subpatterns and bindings (including unrestricted signed/IEEE scalar fields) through execution; bounded symbolic coverage handles bit-mask unions with cross-column correlations and Unknown on exhaustion, computed BitVec-valued segments now run under required-comptime phase/contract/budget checks and become fixed constraints used by coverage; general scalar computed-segment conversion and representation layouts remain pending |
| IIR and executable backend | Non-SSA high-level IIR; completed type comptime, explicit runtime environments; lowering to current LLVM | Validated IIR dumps then executable semantic tests; each backend action has target/dependency evidence | IIR staging, cleanup, effect handling and concrete instance execution are implemented; inspect compiler/iir/src and luna execution/instantiation paths. LLVM probe evidence is not end-to-end native compilation evidence |
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

The HIR `For` node preserves the iterable, pattern, guard, label identity and body. Executable elaboration is still rejected in `program.rs`. Do not wire the historical `library/builtin/protocols/iterator.fl` `Iterator.next` API directly: current `docs/control-flow/basic.md` names `Iter` and `IntoIterator`, while the author's intended iterator architecture uses inline functions plus inline effects. The callable protocol signatures, language-item selection and yield/handler mapping need a coherent contract before source `for` can be lowered. This is a semantic integration gap, not evidence that executable iteration is complete.

### Refutable inline Integer range patterns

Bounded Integer range expansion now selects refutable patterns through the existing required pattern inspector. Failed patterns skip the iteration guard and residual body; ordinary body/type/phase checking still runs without invoking nested inline computations. Open definition checking no longer rejects refutable patterns, while pure behavior witnesses continue to inspect the abstract body even when no iteration is selected. Tuple-enumeration restrictions are unchanged.

Unselected range pattern observations and guards now check free-input availability and call phase without invocation. This closes a discovered path that accepted runtime parameters, runtime aliases or runtime calls in a skipped comptime guard. The initial unsupported-pattern report is `/tmp/luna-tools-4gz4pv5o/result.json`; the skipped-guard failures are `/tmp/luna-tools-zjy7uedm/result.json` and `/tmp/luna-tools-z4olyimq/result.json`. The old rejection-only test for an open literal pattern was replaced by execution/instantiation coverage of the now-supported behavior.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test inline_control --test frontend_cli` passed, report `/tmp/luna-tools-7tet1fe5/result.json` (235 inline-control tests plus the CLI group). Cases include literal/or/as patterns, sequential transformations, failed-pattern guard non-invocation, empty ranges, unselected nested computation, labeled break/continue, open specialization and purity/phase/type rejection. Workspace formatting passed `/tmp/luna-tools-6ll4b14n/result.json`; `git diff --check` passed. Changes remain uncommitted on dev.

### Comptime owner phase in rejected range iterations

Corrected the unselected-range availability check to recognize the enclosing comptime function's phase during definition checking, before any required invocation is active. Its ordinary parameters and local values remain staged inputs; runtime functions retain their input/call restrictions. The first regression report is `/tmp/luna-tools-66z7d9l2/result.json`.

A source regression directly invokes the comptime function with both Boolean values, testing both a parameter guard and a mutable local alias guard. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test inline_control --test frontend_cli` passed 236 inline-control tests and 144 CLI tests, report `/tmp/luna-tools-4q8yddqf/result.json`. Formatting passed `/tmp/luna-tools-2gdqrkgm/result.json`; `git diff --check` passed. Documentation clarifies definition-time availability. Changes remain uncommitted on dev.

### Refutable tuple-enumeration index patterns

Tuple enumeration now filters by refutable comptime index patterns through the same required pattern inspector as ranges. Failed indices skip the guard and runtime element observation/body, but retain existing element-delivery and cleanup boundaries. Unselected index observations and guards use the staged-availability check; runtime elements remain unavailable to a comptime guard. Runtime element patterns still must be irrefutable, and empty tuple body checking remains unfinished.

Open tuple description accepts refutable indices while tracking whether indices are irrefutable for definite-completion inference. An index-filtered body that returns does not alone prove the loop returns. The initial rejected-index regression is `/tmp/luna-tools-z5cmz4yg/result.json`. Tests now execute literal/or/as index selection and open function filtering, reject runtime guard inputs and hidden type/purity errors, and reject a missing function result when all indices may be filtered.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test inline_control` passed 238 tests, report `/tmp/luna-tools-_k3mg71x/result.json`; `test -p luna --test drop tuple_enumeration` through the same wrapper passed 2 grouped tests, report `/tmp/luna-tools-tuf9_heo/result.json`. The cleanup matrix now includes `(0 or 2) as index` with both bound and ignored elements, checking actual destructor event order against the corresponding guard-filter case. Formatting passed `/tmp/luna-tools-ulr4g7i3/result.json`; `git diff --check` passed. Changes remain uncommitted on dev.

### Runtime tuple-element pattern filtering

Tuple enumeration now supports refutable runtime element patterns. The generated LetPattern has a failure continuation that skips the current unrolled iteration; iteration wrapping rewrites matching continue targets in prefix statements as well as in the source body. Open descriptions include both index and element refutability when deciding whether the loop can fall through. Static index/guard rejection still prevents runtime element observation.

The original runtime literal-pattern regression failed in `/tmp/luna-tools-vxh2jl20/result.json`. Source tests now execute Boolean element filtering, check one observation per reached element and none for statically filtered iterations, and reject missing return values and hidden impure bodies. The former rejection-only test for a Boolean element pattern was replaced by positive execution evidence.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test inline_control` passed 240 tests (`/tmp/luna-tools-0bs4wd19/result.json`); `test -p luna --test drop tuple_` through the same wrapper passed 17 tests (`/tmp/luna-tools-b_gex5ek/result.json`). A new IIR cleanup case preserves the whole matched enum until body exit and checks First/Second/Third destructor order across a failed middle element and continued later iteration. An earlier wildcard-payload version released ignored payloads at pattern commit; the final ownership-preserving test explicitly binds the whole element. Formatting passed `/tmp/luna-tools-pupcehq3/result.json`; `git diff --check` passed. Empty tuple body checking remains unfinished. Changes remain uncommitted on dev.

### CMTT range filtering and integrated flow-stack repair

The generator-specific inline range path now accepts refutable patterns. Definition checking retains conservative fallthrough when a pattern may fail; concrete matching skips the guard and code-construction body on failure. Existing checks still reject missing code returns and type/phase errors in unselected bodies. Added literal, Or/as and sequential-pattern generator cases, plus a nonterminating guard that must not be invoked for failed patterns. The first unsupported-pattern report is `/tmp/luna-tools-y4olewol/result.json`.

Cross-module integration exposed host stack overflow in the existing `effect_result_contracts_bind_completed_values_for_ordinary_and_quote_functions` test (`/tmp/luna-tools-0hnowu38/result.json`). A larger diagnostic-only stack passed the isolated case (`/tmp/luna-tools-w1nsigj2/result.json`), while the default stack reproduced it during flow verification of the quote expansion. Optional GDB was unavailable because ptrace was restricted (`/tmp/luna-tools-23goiefs/result.json`). Scope/block binding and control/effect/contract dispatch have now been extracted into non-inlined evaluators, so nested control flow does not reserve unrelated decoding/aggregate/pointer scratch storage in every frame. No recursion limit, proof budget or test-stack default changed. All temporary logging was removed.

Validation in `/home/fin/luna`, all using `python3 -B development/check.py --timeout 900 --`: `test -p luna --test cmtt --test cmtt_methods --test quote_functions --test inline_returns --test staged_aggregate_contracts` passed 505 + 133 + 103 + 2 + 31 tests (`/tmp/luna-tools-cdgvwfk0/result.json`), including the previously overflowing case on the default stack; `test -p elaboration --lib --test verification` passed 168 + 46 (`/tmp/luna-tools-xcw9rsvg/result.json`); `test -p luna --test inline_control --test match_cleanup` passed 240 + 13 (`/tmp/luna-tools-38rpchfn/result.json`). Formatting passed `/tmp/luna-tools-h8lppwrz/result.json`; `git diff --check` passed. Changes remain uncommitted on dev.

### CMTT tuple pattern filtering with live data

Generator tuple enumeration now accepts refutable index and element patterns. The existing generator match implementation exposes an already-evaluated packet entry point; tuple iterations reuse it without reevaluating the input or serializing pointer aliases into detached values. Failed pattern writes survive, mismatches skip the iteration guard/body, and description checking joins the fallback path conservatively. Current-loop continue is consumed even when the guard is unknown. Irrefutable patterns retain the existing Type-witness binding path. Empty tuple checking remains unfinished.

The original unsupported-pattern regression failed at `/tmp/luna-tools-i9r0_sob/result.json`. New tests cover literal/Or/as filters, live aliases, failed pattern writeback, skipped guards, continue, missing returns and hidden body type errors. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_methods --test quote_functions --test inline_control --test match_cleanup` passed 507 + 134 + 103 + 240 + 13 = 997 tests, exit 0 (`/tmp/luna-tools-91eq07cj/result.json`). Workspace formatting passed (`/tmp/luna-tools-_g2itpmj/result.json`); `git diff --check` passed. Documentation updated; changes remain uncommitted on dev.

CMTT tuple filtering follow-up: source execution now also verifies completed Type values and heterogeneous field types survive filtering; pattern-code generators preserve early returns and labeled outer break/continue; uncalled definitions reject runtime calls in both iteration and pattern guards and reject mutation of either index or element bindings. These checks exercise expression-code and pattern-code result paths separately. `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt tuple_` passed (exit 0, `/tmp/luna-tools-zu8newy5/result.json`); the isolated Type/heterogeneous case passed at `/tmp/luna-tools-8rl1l794/result.json`. Workspace formatting and diff whitespace checks passed (`/tmp/luna-tools-wvpsahmh/result.json`). No implementation change was needed for these additional cases; the regressions establish the broader scope of the shared packet implementation.

### Stateful generator range patterns and early iteration immutability

Generator ranges previously selected patterns through immutable normal-form evaluation. A sequential pattern or pattern guard that accessed the generator's live state failed with an unavailable comptime-input error (`/tmp/luna-tools-vdrvnhd8/result.json`). Concrete range iterations now use the shared already-evaluated generator match packet. Pattern effects survive failure and alternative fallback, successful pointer bindings keep aliases, and the iteration guard/body read the updated state. Endpoints still evaluate once, empty ranges still invoke no pattern computations, and construction work remains budgeted.

Definition-side range checking also allowed assignment to an index inside `and … is …` before marking the binding immutable (`/tmp/luna-tools-7f5p98aw/result.json`). The same omission allowed mutation of runtime tuple element bindings during sequential matching (`/tmp/luna-tools-0_b4jmc8/result.json`). Iteration bindings are now marked immutable before checking their patterns in generator ranges, generator tuple probes, ordinary inline expansion and open-function descriptions. This does not freeze pointer pointees. Positive live-pointer/state-order tests and negative uncalled-definition tests cover these boundaries.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_methods --test quote_functions --test inline_control` passed, exit 0 (`/tmp/luna-tools-jafib4h2/result.json`). The preceding tuple-filtered run including `--test drop` passed 24 + 5 + 17 + 30 tests (`/tmp/luna-tools-2k2hb1qx/result.json`), including resource cleanup. Formatting passed (`/tmp/luna-tools-hhv1vf1n/result.json`); `git diff --check` passed. Changes remain uncommitted on dev. Formal syntax does not define inline while/loop; no such syntax extension was introduced.

### Generator binding mutability and shared pattern state

Generator `var-else` previously reused immutable match-arm binding policy and rejected assignment inside its guard or sequential pattern. The shared packet matcher now accepts the declaration's binding mutability; ordinary matches and iteration patterns retain immutable binding policy. Successful matching exports updated values, including a reassigned pointer target. Failed candidate-local mutations do not leak into another candidate or the else scope, while writes to outer state survive. The original rejection is `/tmp/luna-tools-gp161cih/result.json`.

Irrefutable generator destructuring also used standalone required invocation, so sequential patterns could not read/write outer state or export newly produced live pointers (`/tmp/luna-tools-q2e6q_tj/result.json`). Nontrivial concrete data bindings now check and execute their pattern in the shared generator data frame; bare nonlive bindings retain their existing fast path and definition checks retain structural Type witnesses. Concrete irrefutable tuple iterations use the same path (`/tmp/luna-tools-jgwezx46/result.json` reproduced the missing state there). The tuple refutability probe now preserves symbolic mutable inputs, avoiding accidental replacement of a valid addressable local with its current literal value. Ordinary runtime local const materialization remains separate.

New source regressions cover mutable versus immutable declarations, failed guards with lexical shadowing, Or-candidate rollback, pointer rebinding, outer state changes from sequential patterns, pointer production from scalar inputs, and both filtered and irrefutable tuple traversal. Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_methods --test inline_control --test quote_functions` passed with exit 0 (`/tmp/luna-tools-d17k0e3c/result.json`). Workspace formatting passed (`/tmp/luna-tools-9u569_w4/result.json`); `git diff --check` passed. Documentation updated; changes remain uncommitted on dev.

### Local const pattern phase isolation after shared generator state

Added source-level phase-boundary regressions for ordinary runtime-local const sequential patterns. Comptime calls and mutations of locals owned by the pattern computation execute successfully. Runtime parameters, runtime local reads/writes and runtime calls remain rejected, including when the computation result is matched by `_`. These tests confirm that the generator state-frame changes do not widen local-const access; no implementation change was needed. Added a concrete sequential-pattern example to the comptime documentation.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt local_const_` passed both new tests (`/tmp/luna-tools-9cwkmxse/result.json`). `test -p luna --test normal_patterns --test comptime_memory --test staged_aggregate_contracts --test instantiation` through the same wrapper passed 14 + 12 + 31 + 51 tests, exit 0 (`/tmp/luna-tools-wjfeyy8i/result.json`). Formatting passed (`/tmp/luna-tools-5xwxi3hi/result.json`); diff whitespace checks passed. Changes remain uncommitted on dev.

### Boolean Type-guard evidence for dependent normal-form patterns

Normal-form patterns can now use symmetric Type equality and a bounded, non-mutating Boolean guard to establish the domain of later value parameters. Previously `Box<(Kind if bool == Kind),item>` was rejected despite the equivalent left-oriented equality working (`/tmp/luna-tools-_e1uxo3a/result.json`). Guard evidence combines conjunction, disjunction, negation, Boolean locals and constants. Alternative successful paths must agree on the type; impossible Boolean branches do not weaken common evidence. Calls, blocks, assignments and other unsupported observations provide no such evidence. The original guard still executes normally; normal-form heads remain opaque and no global solver assumption is introduced. General dynamically bound dependent Type domains remain unfinished.

An independent finite-model test checks 7,320 guard trees under both truth outcomes, three Type inputs and both Boolean input values. Any exact inferred type must match every satisfying model; impossible evidence must have none. Budget and depth exhaustion are also exercised. Source tests execute positive symmetric/compound guards and reject multiple-domain, unrelated-flag and effectful guards, including a nonterminating call that must not be invoked to infer the domain.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib type_guard_evidence` passed (`/tmp/luna-tools-161be0kr/result.json`); `test -p luna --test normal_patterns --test cmtt --test inline_control` through the same wrapper passed, exit 0 (`/tmp/luna-tools-25yzjhpl/result.json`). Formatting passed (`/tmp/luna-tools-kwiu32cp/result.json`); `git diff --check` passed. Type-pattern documentation includes the new supported case and its limits. Changes remain uncommitted on dev.

### Boolean value-guard evidence for dependent normal-form domains

A normal-form Boolean value parameter can now acquire a unique retained value from its guard, allowing later domains such as Config<true> or Config<false> to form. The original guarded-binding case failed at `/tmp/luna-tools-8e5m2lm9/result.json`. The bounded evaluator considers false and true separately, retaining unknown Boolean inputs as sets of possible outcomes. It supports Boolean literals/locals, equality, negation, conjunction and disjunction. Only a singleton successful input becomes domain evidence. Ambiguous or impossible guards produce no value; calls and writes are not interpreted. Float identity rules and opaque heads remain unchanged.

Positive source tests pass the resulting value to a function requiring the exact Config instance, not merely a shared field shape. Negative tests cover unrelated inputs, multiple admitted values, contradiction, nonterminating calls and assignment blocks. An independent test enumerates 9,520 Boolean expression trees and checks that the abstract result contains every concrete result for each subject/input assignment; budget/depth exhaustion are tested as well.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p elaboration --lib guard_evidence` passed 2 tests (`/tmp/luna-tools-ozigx1sj/result.json`); `test -p luna --test normal_patterns` passed 18 (`/tmp/luna-tools-8rsjagfs/result.json`); `test -p luna --test cmtt --test inline_control --test instantiation` passed, exit 0 (`/tmp/luna-tools-75_eglev/result.json`). Formatting passed (`/tmp/luna-tools-38rawtg0/result.json`); `git diff --check` passed. Type-pattern docs updated; changes remain uncommitted on dev.

### Exact scalar guard identities in normal-form patterns

Normal-form value patterns now derive exact retained input identities from equality guards over integers (including fixed widths), char, Symbol and str. Equality is symmetric and combines through the same bounded Boolean evidence kernel as Type guards; conjunction may retain a required identity, while alternative paths must agree. Atomic scalar observations are limited to locals, literals and integer negation. No call or block is invoked to obtain evidence. General inequalities and multiple admitted identities remain unknown; Float/f32/f64 equality remains excluded because numerical equality does not uniquely determine retained representation.

Type and scalar guards now share the generic evidence-combination implementation, with the existing finite-model tests exercising that common logic. Source tests instantiate six scalar domains, symmetric comparisons and conjunction/disjunction, then pass the resulting payload to a function requiring the exact dependent Config instance. Rejection cases cover multiple constants, inequality, an unrelated Boolean alternative, an unknown recursive call, a write and all three floating domains.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p elaboration --lib guard_evidence` passed 2 tests (`/tmp/luna-tools-3nbfozc9/result.json`); `test -p luna --test normal_patterns --test cmtt --test inline_control` passed, exit 0 (`/tmp/luna-tools-ske5yio6/result.json`). Formatting passed (`/tmp/luna-tools-b0snwxiv/result.json`); `git diff --check` passed. Documentation includes an exact-sized nominal example and floating identity limits. Changes remain uncommitted on dev.

### Conditional guard evidence across Type and value domains

Type, Boolean-value and exact-scalar guard evidence now handles if/else and expression-only blocks. Generic retained-identity evidence conjoins each branch outcome with the corresponding condition outcome, then merges alternatives; Boolean value analysis unions outcomes only from possible branches. Neither path invokes code, and statement-bearing blocks, calls and writes remain unsupported evidence sources. Boolean conjunction/alternative merging is shared by logical operators and conditional expressions. Work and recursion remain budgeted.

The new Type-guard case was rejected before implementation (`/tmp/luna-tools-ob97ihy4/result.json`). Source coverage now includes conditions that test the subject itself, external conditions selecting equivalent identities, scalar domains including negative integer literals, and rejection of differing identities, unconditional alternative success or unknown calls. Independent evaluators now execute branch selection for 1,500 additional Type-model trees and 896 additional Boolean-value-model trees, including expression-only branch blocks.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p elaboration --lib guard_evidence` passed 2 tests (`/tmp/luna-tools-nfzbdlrl/result.json`); `test -p luna --test normal_patterns --test cmtt --test inline_control` passed, exit 0 (`/tmp/luna-tools-5vqcau06/result.json`). Formatting passed (`/tmp/luna-tools-v6s_mcx5/result.json`); `git diff --check` passed. Type-pattern documentation updated; changes remain uncommitted on dev.

### Boolean equality preserves retained-input guard evidence

The shared identity-evidence kernel now handles equality between Boolean expressions, so `(Kind == bool) == true` and `(key == 7) == true` retain their original Type/value constraints. Both equality outcomes are analyzed; comparing a predicate with an unknown Boolean or with itself does not invent a unique input identity. Scalar equality over non-Boolean operands still uses the domain-specific rules, including floating-point exclusions. The initial source rejection is `/tmp/luna-tools-290madju/result.json`.

The independent Type model now includes 3,600 additional Boolean-equality trees. A separate performance regression showed that 40 nested `== true` wrappers exhausted a 128-step budget (`/tmp/luna-tools-jcy8o7ck/result.json`). A literal-side fast path now selects the other operand's required truth directly, preserving linear work for these wrappers without increasing any budget or depth limit.

Validation in `/home/fin/luna`, all through `python3 -B development/check.py --timeout 900 --`: integration `test -p luna --test normal_patterns --test cmtt --test inline_control` passed before the literal optimization (`/tmp/luna-tools-0twi1kxg/result.json`). After the optimization, `test -p elaboration --lib guard` passed (`/tmp/luna-tools-hkcj_0rr/result.json`), the isolated linear-budget regression passed (`/tmp/luna-tools-omf3q2gm/result.json`), and `test -p luna --test normal_patterns` passed (`/tmp/luna-tools-kqchegfc/result.json`). Formatting passed (`/tmp/luna-tools-y__5o4vv/result.json`); `git diff --check` passed. Documentation updated; changes remain uncommitted on dev.

### Mixed-domain comparisons in retained-input guards

Guards can now combine an established Type/value identity with an unrelated direct comparison without losing all domain evidence. A budgeted structural check recognizes comparison operands made from locals, literals, retained Type values and numeric negation. Such a comparison contributes Unknown, never an invented truth value or identity. Calls, blocks with writes, address-taking, dereferences and other unsupported operand computations are excluded. Boolean-input analysis separately handles Boolean equality and treats recognized non-Boolean comparisons as both possible outcomes.

The initial mixed-domain case failed at `/tmp/luna-tools-c0ndyo4p/result.json`. Source tests exercise Type, Boolean-value and integer-value domains against integer, fixed-width, character, floating, Symbol and string comparisons, executing both passing and failing inputs. Negative tests cover disjunction without unique identity, Boolean equality with an unknown comparison result, calls and writes. Independent model evaluation now varies the Boolean input and foreign integer comparison independently, checking soundness without assuming a correlation between them.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p luna --test normal_patterns --test cmtt --test inline_control` passed 22 + 513 + 241 tests (`/tmp/luna-tools-g68ymuqt/result.json`); final `test -p elaboration --lib guard_evidence` passed both model tests (`/tmp/luna-tools-s6os_gg0/result.json`). Formatting passed (`/tmp/luna-tools-3ssutal0/result.json`); `git diff --check` passed. Type-pattern docs updated; changes remain uncommitted on dev.

### BitVec equality identities in dependent normal-form patterns

Exact retained-value evidence now includes same-width BitVec equality. The normal-form matcher recognizes BitEqual and uses the existing budgeted literal reconstruction, preserving width and every stored bit. This extends the same guard-combination rules used for other exact scalar domains; it does not change bit-vector casts, writeback, runtime arithmetic or floating identity policy. The original unsupported dependent pattern is `/tmp/luna-tools-u41w2868/result.json`.

Source cases cover widths 0, 1, 8, 65 and 129, symmetric equality and a Boolean conjunction, with a receiver requiring the precise Config instance. Width zero correctly has no distinct alternative value. Negative cases reject multiple bit patterns, inequality, an unknown producer and candidate mutation. The explicit width-mismatch case has a positive control that changes only the producer width from 4 to 8.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_patterns --test pure_bitvec --test instantiation` passed 24 + 14 + 51 tests (`/tmp/luna-tools-alcp1wk4/result.json`). The added width-control check passed with `test -p luna --test normal_patterns bitvector_guard_evidence` through the same wrapper (`/tmp/luna-tools-6wuxd5xr/result.json`). Formatting passed (`/tmp/luna-tools-l1y77vmb/result.json`); `git diff --check` passed. Type-pattern docs include a BitVec-tagged nominal example. Changes remain uncommitted on dev.


### Computed retained-input witnesses and Type-pattern flow verification

Exact scalar guard witnesses now reuse budgeted constant reconstruction for readonly arithmetic and BitVec computations. Operand admission remains structural: user calls, writes, address-taking and pointer reads cannot supply evidence. Fixed-width final integer values must fit their declared domain. The original expression remains executable and is still verified; a mathematically valid final value cannot hide an intermediate machine overflow.

This exposed a separate flow-verification gap: NormalForm and TypeTuple patterns were unsupported whenever a function required arithmetic verification. The flow walker now visits retained declaration inputs and nested Type tuple patterns, preserves both possible head outcomes, executes guard analysis in order and merges failed-candidate effects. It does not unfold heads, assert global type equalities or invent concrete retained values. Analysis deliberately starts with unknown declaration input values; dependent types without usable range information remain conservative.

Validation in `/home/fin/luna`, through `python3 -B development/check.py --timeout 900 --`: `test -p luna --test normal_patterns --test pure_bitvec --test instantiation` passed 27 + 14 + 51 tests (`/tmp/luna-tools-bji8enn3/result.json`); two additional failed-guard/Type-tuple regressions brought normal_patterns to 29 passing (`/tmp/luna-tools-057xnyp1/result.json`). `test -p elaboration --lib verification::flow` passed 4 tests (`/tmp/luna-tools-4upofhes/result.json`), `test -p elaboration --lib guard` passed 11 (`/tmp/luna-tools-n782mnak/result.json`), and `test -p luna --test arithmetic_contexts` passed 10 (`/tmp/luna-tools-a1cw45oz/result.json`). Cases cover constant arithmetic, signed values, arbitrary integers, 65-bit operations, rejection of calls/writes/out-of-domain results, intermediate overflow, successful-guard ranges, failed-guard writes and nested Type tuples. Formatting passed (`/tmp/luna-tools-dv5_fzpg/result.json`); changes remain uncommitted on dev.


### Dynamic nominal member projections in arithmetic verification

The flow verifier now handles internal NominalType patterns from dynamic member projection using declaration input domains for structs, enums and newtypes, including compact all-Type signatures. It validates declaration/arity and walks fields in order without confusing constructor parameters with runtime fields. Unknown inputs remain unknown; this does not add head unfolding or assume formation results.

The source regression initially failed with unsupported-construct (`/tmp/luna-tools-8pjgi7ty/result.json`). New cases execute struct/enum aliases, chained aliases forwarding parent values, checked arithmetic boundaries, rejection without a sufficient guard, and a parent producer with a write whose invocation count must be exactly one. Targeted dynamic tests passed (`/tmp/luna-tools-231cvvch/result.json`). Formatting passed (`/tmp/luna-tools-vqmdu0c2/result.json`).

Broader checks uncovered pre-existing failures, not a clean suite: `test -p luna --test newtypes` reached 205 passes and two failures (`/tmp/luna-tools-zeeqk20e/result.json`): tuple_pointer_delivery_does_not_unfold_other_heads_or_hide_aliases and enum_pointer_delivery_requires_copy_and_rejects_hidden_pointers still expect NIR rejection of pointer-containing aggregate reads. Disabling this turn's NominalType arm reproduced both failures (`/tmp/luna-tools-b2aqtcw0/result.json`). `test -p luna --test nominal_value_parameters` reached 254 passes and one failure (`/tmp/luna-tools-kcosampq/result.json`): associated_scalar_equality_is_a_program_contract loses the checked Boolean equality at a call precondition. Disabling the same new arm reproduced it independently (`/tmp/luna-tools-wkw45l9y/result.json`). The temporary disabled arm was restored. These failures remain open: audit the pointer tests against current resource/provenance semantics, and repair associated scalar contract fact transport; do not weaken their semantic expectations simply to turn the suite green. A nested struct-member variant of the new fixture also exposed the existing unsupported struct-body declaration boundary; the successful chained-member case uses supported type aliases instead.

After restoring the implementation, `python3 -B development/check.py --timeout 900 -- test -p luna --test normal_patterns` passed all 31 tests (`/tmp/luna-tools-gz36mbw4/result.json`). All commands ran in `/home/fin/luna`; changes remain uncommitted on dev.


### Bounded disequality facts and stale pointer-delivery expectations

Resolved the three failures found by the previous integration checks. Tracing the associated-scalar test narrowed the actual failure to its `!=` subcase: equality already transported correctly. Bounded disequalities were discarded by retained-condition admission because both operands had finite interval enclosures. Intervals/convex polynomial rows cannot encode a missing interior point or the choice `left < right or left > right`. Retain these checked conditions, including negated equality, under the existing version/currentness checks and budget. No program facts move into PTC. The original source test now passes all equality, inequality, rejected-contract and dependent-domain cases (`/tmp/luna-tools-4szc70qo/result.json`). Temporary tracing was removed.

Two new prover tests cover bounded interior exclusions, proof replay, copying and common-branch joins, failure after joining an unproved branch, reassignment invalidation, and both concrete Boolean disequality refinements. Both passed (`/tmp/luna-tools-e8yp0ek8/result.json`); the complete verification-filtered library suite passed 56 tests (`/tmp/luna-tools-gpy3uhxt/result.json`).

The pointer-delivery rejection expectations contradicted the current contract in docs/resource-management/memory-effects.md: checked Copy permits reads of pointer-bearing aggregates while preserving payload provenance and lifetime. Replaced those obsolete NIR-rejection expectations with executable positive cases and explicit rejection of escaping local targets for both bound Tuple and enum payloads. Kept the opaque Identity head rejection and missing-Copy rejection. The initial new negatives already rejected escaping provenance but used a different diagnostic than the first assertion expected (`/tmp/luna-tools-xozde9_p/result.json`); the assertions now check the actual lifetime diagnostic rather than an unsupported-feature error.

Final validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test nominal_value_parameters --test newtypes --test normal_patterns` passed 255 + 207 + 31 tests (`/tmp/luna-tools-r8prk_39/result.json`). Formatting passed (`/tmp/luna-tools-3k4qe_a4/result.json`); `git diff --check` passed. The previous three failures are closed; the broader language implementation remains incomplete, including the separately observed nested struct-member declaration boundary. Changes remain uncommitted on dev.


### Nested struct/enum declarations and parent-instance capture

Closed the nested struct-member boundary found during dynamic projection work. Struct/enum AST lowering now admits nested struct/enum declarations in plain, public and private forms, preserving existing prefix processing and static namespace identity. Initial source reproduction failed at lowering (`/tmp/luna-tools-p2__cq8s/result.json`); parent-relative imports and nested enum construction now execute.

Generic nested types exposed several connected gaps. Extended nominal capture collection across lexical ancestors; struct/enum descriptors retain the same capture suffix as their constructed identities. Projected type and value formation accepts these declarations and carries complete parent argument domains through nested projections. Type-head classification distinguishes known projected types from callable projections so a deep named construction is not routed to explicit function instantiation. Nominal Self, inherent implementation metadata, and method signature prefixes now agree on captured parent parameters. Internal nominal input exposure retains capture arguments rather than truncating them to source clause arity. No new IR variants or head unfolding were introduced; existing Display implementations render the extended argument lists.

Source regressions cover static nested structs/enums, enum variant counts, relative imports, private-member denial across modules, parent Type captures and wrong-field rejection, a three-level generic construction with a Self-returning method, distinct parent Boolean-value instances in methods, and a dynamic nested struct member combined with checked arithmetic. A mistaken initial visibility expectation was corrected against docs/code-organization/visibility.md: default is package visibility, not private. Cross-module tests now explicitly declare private members. The static nested source fixture also required its field separator before the method; parser rejection was not counted as a semantic regression.

Validation in `/home/fin/luna`: `python3 -B development/check.py --timeout 900 -- test -p luna --test modules --test normal_patterns --test newtypes --test nominal_value_parameters` passed 53 + 31 + 207 + 255 tests (`/tmp/luna-tools-7x8f_f1r/result.json`). Targeted dynamic nested construction passed (`/tmp/luna-tools-cv4puph3/result.json`), and distinct value-capture methods passed (`/tmp/luna-tools-71pogti5/result.json`). Formatting passed (`/tmp/luna-tools-y7p15ktm/result.json`); git diff --check passed. Scope/path documentation now includes a generic nested type and Self example. Changes remain uncommitted on dev; this does not establish completion of all container forms or the language.


### Nested member constants, defaults and completed captured-domain admission

Runtime member-constant splice now binds the complete parent telescope, including lexical captures, and checks arity before substitution. Constants in a capture-only container are residual generic computations, not owner-keyed module constants. Symbolic value references recognize captured ancestor inputs during declaration checking. A cross-package case alternates Boolean and Type inputs across three container levels and verifies distinct values; an invalid outer trait input is still rejected even when the final constant could short-circuit to false. The initial failure was AbstractValueUnresolved (`/tmp/luna-tools-zz9nrjp_/result.json`); the final focused case passed (`/tmp/luna-tools-bczra6yf/result.json`).

A nested default depending on both a parent Type and parent value exposed prefix-only validation of completed nominal arguments (`/tmp/luna-tools-3h7y7wew/result.json`). Completed argument admission now checks scope/sort for the entire supplied vector before substituting the complete vector into each value domain. This validation does not execute inputs or change their source evaluation order. It handles own inputs stored before captured Type suffixes without adding a representation field or inventing symbolic domain evidence. Tests reject wrong domains, wrong sorts, unavailable symbolic evidence and exhausted budgets; successive invocations with different defaults preserve distinct Indexed identities. Temporary NIR diagnostics were removed.

Validation in `/home/fin/luna`, via `python3 -B development/check.py --timeout 900 --`: `test -p nir --test nominal_parameters --test newtypes --test normal_inputs --test type_computations` passed 59 tests (`/tmp/luna-tools-z9lq2m9d/result.json`). `test -p luna --test packages --test modules --test normal_patterns --test nominal_value_parameters --test newtypes` passed 41 + 53 + 32 + 255 + 207 tests (`/tmp/luna-tools-2yf5uv80/result.json`). Formatting passed (`/tmp/luna-tools-3r_2e_6b/result.json`); git diff --check passed. Scope/path docs explain captured defaults and instance-sensitive constants. Changes remain uncommitted on dev; overall language completion remains open.

### Checked nonzero signed division at the explicit IR boundary

The arithmetic flow verifier now replays a current nonzero divisor proof before splitting a signed divisor interval around zero. Both components use the existing truncating interval operation and their results are enclosed together. Quotient overflow remains mandatory for both division and remainder, including MIN % -1. Reassignment cannot reuse an obsolete nonzero premise. This stays in program verification; no PTC coupling or source-level signed rounding default was introduced. An initial source fixture hit the existing unsupported signed rounding protocol and was removed rather than changing that language policy.

The new NIR regression checks five dividend values against every nonzero i8 divisor for both operations, missing premises, reassignment to zero, MIN/-1 overflow, and exhausted-budget recovery. A separate nested-method regression verifies inherited parent trait contracts and rejects invalid parent instantiations.

Validation in `/home/fin/luna`: wrapper `development/check.py --timeout 900 -- test -p elaboration --lib verification --test verification` ran 56 library tests (integration names were filtered out), report `/tmp/luna-tools-8uzdvf5z/result.json`; the subsequent unfiltered `test -p elaboration --test verification` passed all 47 integration tests, report `/tmp/luna-tools-vtw_an40/result.json`. `test -p luna --test integer_contracts --test arithmetic_contexts` passed 12 + 10 tests, report `/tmp/luna-tools-uprb27ow/result.json`. Targeted parent-contract test passed, report `/tmp/luna-tools-9hsbeh8r/result.json`. Workspace formatting passed, report `/tmp/luna-tools-iu0nfpb0/result.json`; git diff --check passed. Changes remain uncommitted on dev.

### Lexical member projection and residual nominal value captures

A direct lexical `Inner<bool>.Result` worked in a method result annotation but failed inside a const initializer with a container argument-count mismatch (`/tmp/luna-tools-yncx3ui7/result.json`). Prepared type instances now return the complete input telescope, including captures, and reuse their prepared expressions. Captured checked values remain ComptimeValue inputs rather than being eagerly invoked while their enclosing generic definition is symbolic.

A three-level regression with captured T, U and value:T exposed a second boundary: nominal construction could not reify an authorized abstract value into its residual identity. Interpreter nominal input reification and nominal admission now use explicit declaration/domain authority. Newtype admission has an evidence-taking entrypoint that preserves capture-first ordering; existing completed admission stays closed. Scoped Type descriptions can transport these identities, while closed snapshot export still rejects them. Prebuilt TypeValue identities also undergo nominal-domain admission under a value scope; a new negative test caught and prevented a forged Char-indexed identity carrying a Bool-authorized value. Dispatch preparation retains generic newtype identities, matching its existing struct/enum behavior; concrete materialization still checks completed arguments. Normal-form argument reification retains its original closed rules.

Source tests cover lexical projections in annotations and const expressions, multiple ancestor Type/value inputs, distinct concrete instances, and wrong field/domain rejection. Interpreter tests cover ordinary and captured newtype inputs, absent authority, mismatched domains, forged prebuilt identities, and closed export rejection. NIR tests independently check value domains substituted from captured Type suffixes and budget-failure recovery. Scope/path docs include the lexical const example.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p luna --test modules --test normal_patterns --test nominal_value_parameters --test newtypes --test packages` passed 56 + 32 + 255 + 207 + 41 tests (`/tmp/luna-tools-4ibr8tap/result.json`). `test -p comptime_interpretor --test abstract_values --test nominal_types --test normal_values` passed 13 + 23 + 17 (`/tmp/luna-tools-svjx1vxl/result.json`). `test -p nir --test newtypes --test nominal_parameters` passed 18 + 20 (`/tmp/luna-tools-w0duwphj/result.json`). Formatting passed (`/tmp/luna-tools-gg5pkudh/result.json`); git diff --check passed. Changes remain uncommitted on dev; overall language completion remains open.
Final targeted rerun after separating nominal reification from normal-form reification: `test -p luna --test modules lexical_nested` passed both regressions (`/tmp/luna-tools-6q8lo0ad/result.json`).

### Residual type-list transport and invocation admission

Extended authorized residual Type reification compositionally through meta.List<Type>, tuples and records. New tests first reproduced two gaps (`/tmp/luna-tools-q5hqkuwu/result.json`): lists discarded value-parameter authority, while the prior residual Type path accepted non-Type roots. A shared Type-root checker now preserves explicit declaration scope for residual invocation and rejects value roots. Closed normal-term export and Type-only scoped export remain closed to symbolic value inputs. Budget failure does not poison later reification.

A source-defined `pack(value:bool)->List<Type>` returning two Indexed<value> identities exposed immutable admission still using Type-only copy accounting (`/tmp/luna-tools-tfcw7l5u/result.json`). Scoped immutable traversal now accounts for residual identities in lists and aggregates, followed by independent nominal formation/domain admission. The interpreter retains closed host-copy behavior when no scope is supplied.

The negative source fixture then exposed a raw invocation path that accepted a Char-indexed TypeList carrying a Bool-authorized symbol (`/tmp/luna-tools-lrnyd2s_/result.json`). Invocation parameter binding now checks embedded Type descriptions before entering the body, including inside tuples, records and newtype payloads; it does not dereference pointers or read storage independently. This shared binding path covers ordinary and instantiated calls. Tests reject absent authority and wrong domains, then successfully reuse the interpreter for valid direct and tuple-wrapped lists. Concrete Boolean instantiation remains exportable; symbolic identities remain unexportable as closed constants. No normal-form unfolding or PTC value reasoning was introduced.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p comptime_interpretor --lib --test abstract_values --test nominal_types --test normal_values` passed 66 + 15 + 23 + 17 tests (`/tmp/luna-tools-wzqzqofs/result.json`). `test -p luna --test type_spreads --test modules --test normal_patterns` passed 91 + 56 + 32 tests (`/tmp/luna-tools-0cj6py5l/result.json`). Formatting passed (`/tmp/luna-tools-kjk1gxaf/result.json`); git diff --check passed. Changes remain uncommitted on dev. Overall language completion remains open.

### Residual value arguments in opaque normal-form identities

Extended the prior nominal-only residual path to ordinary opaque normal forms, following docs/comptime/purity-and-normal-form.md and normalization-and-instantiation.md: declared input types permit checking an application before concrete values are known; its head is never unfolded. The source reproduction `Indexed<T,value:T>` under an authorized abstract Bool input failed with AbstractValueUnresolved (`/tmp/luna-tools-94_f91nv/result.json`). Normal-form construction now reifies scoped inputs and checks them with independent declaration/domain evidence. NIR exposes a budgeted evidence-taking admission entrypoint; closed admission supplies no authority. Domain substitution and recursive record/tuple payload checks remain in the original validator, and value authority cannot change an input's sort.

Interpreter description validation and invocation admission use the same explicit evidence. Source tests verify the exact Indexed<bool, symbol> identity, reject a forged Char domain, recover after rejection, reject closed export, and verify no Indexed body invocation occurred. Concrete inputs still yield closed exportable identities. NIR tests cover dependent record fields, unknown declarations, wrong domains, zero-budget failure and recovery, and a deliberately permissive callback that still cannot admit a value as Type. The comptime documentation now illustrates the distinction between symbolic type identity and observing a symbolic value.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p nir --test normal_inputs` passed 11 tests (`/tmp/luna-tools-f8ofp258/result.json`). `test -p comptime_interpretor --lib --test abstract_values --test normal_values --test nominal_types` passed 66 + 15 + 17 + 23 tests (`/tmp/luna-tools-qe_irt76/result.json`). `test -p luna --test type_spreads --test normal_patterns --test nominal_value_parameters --test newtypes` passed 92 + 32 + 255 + 207 tests (`/tmp/luna-tools-62j2t934/result.json`). Formatting passed (`/tmp/luna-tools-hqy6l2lw/result.json`); git diff --check passed. Changes remain uncommitted on dev; overall language completion remains open.

### Scoped comptime inputs bind dependent call signatures

A source `keep(comptime enabled:bool, value:Token<enabled>)->Token<enabled>` could not accept an authorized symbolic Boolean and its corresponding value-indexed record. The first reproduction failed during representation copying (`/tmp/luna-tools-rogfanxh/result.json`); further checking exposed closed-only nominal value admission. Scoped parameter binding now recognizes authorized abstract data, including leaves inside tuples/records, and reifies it through the existing residual substitution path. Later parameter types, result types and body annotations therefore retain the same declaration identity. Removed the now-unused closed input helper from this invocation path; completed constant export APIs are unchanged.

Representation copying of nominal instance arguments uses explicit Type/value scope; checkpointing a newtype uses the same scoped type-copy rule as other invocation values. Struct, enum and newtype `has_type` checks now accept symbolic indices only through exact declaration/domain evidence, preserving identity, payload and capability checks. No symbol is replaced by an arbitrary concrete representative, and pointers/storage are not newly snapshotted.

Source regressions exercise direct and forwarding calls for structs, enums and newtypes; each rejects substituting Token<true> for Token<symbol> and then successfully executes a concrete call. A dependent tuple-valued comptime input also retains its symbolic leaf while rejecting a changed concrete field. Targeted aggregate regression passed (`/tmp/luna-tools-xtzth5gm/result.json`).

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p comptime_interpretor --lib --test abstract_values --test nominal_types --test normal_values` passed 66 + 15 + 23 + 17 tests (`/tmp/luna-tools-b400rmts/result.json`). `test -p luna --test nominal_value_parameters --test newtypes --test type_spreads --test normal_patterns` passed 257 + 207 + 92 + 32 tests (`/tmp/luna-tools-7vonr83g/result.json`). Formatting passed (`/tmp/luna-tools-xlitokrx/result.json`); git diff --check passed. Changes remain uncommitted on dev; overall language completion remains open.

### User-defined comptime data participates in dependent calls

Dependent function-input binding previously recognized builtin aggregates but excluded Record, Variant and Newtype data entirely. A source regression with a concrete Key and Packet<key> failed before any symbolic case (`/tmp/luna-tools-hvub5pit/result.json`). The bounded data classifier now visits nominal payloads while preserving its rejection of callable/storage/pointer leaves; a partially immutable aggregate is not frozen. Complete declaration identity, enum tag and payload enter the existing residual substitution path.

A second source regression combined Key<symbol> with Packet<symbol,key> and failed during scoped reification (`/tmp/luna-tools-kmgijrh1/result.json`). Invocation reification now retains authorized value binders in a nominal value's own type arguments as well as its payload. Closed export entrypoints still use closed validation. No new IR or source syntax was introduced.

Regressions cover concrete and symbolic fields for struct, enum and newtype inputs, direct and forwarding calls, dependent results and rejection of changed key payloads. An indexed Key regression verifies its own symbolic environment survives nested nominal indexing. Interpreter boundary tests verify exact nominal identities and enum tags, absent authority rejection, closed export rejection, exhausted budgets and recovery.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p comptime_interpretor --lib --test abstract_values --test nominal_types --test normal_values` passed 66 + 16 + 23 + 17 tests (`/tmp/luna-tools-25tuhd7q/result.json`). `test -p luna --test nominal_value_parameters --test newtypes --test type_spreads --test normal_patterns` passed 259 + 207 + 92 + 32 tests (`/tmp/luna-tools-3_rlghiv/result.json`). Formatting passed (`/tmp/luna-tools-mf_wbmpr/result.json`); git diff --check passed. Changes remain uncommitted on dev; overall language completion remains open.

### Explicit clause instances share scoped data admission

`invoke_instantiated` previously screened all declaration inputs with an empty scope, while function-input admission recognized only a top-level abstract scalar. A source regression passing Key{flag:symbol} into a `where key:Key` function failed (`/tmp/luna-tools-kl879ohi/result.json`). The explicit entrypoint now uses its authorized Type/value scope, and declaration-domain checking uses the recursive evidence-aware validator rather than a scalar special case. Runtime/materialized graph entry behavior remains closed.

Regressions cover explicit invocation and a source `keep<key>` forwarding call, absent scope rejection, wrong nominal field domains, recovery with concrete inputs, and authorized Type parameters. Observing an unknown key.flag in an if still returns AbstractValueUnresolved; later false/true instances produce their respective results. This preserves the distinction between retaining an input identity and computing from its unknown value.

Validation in `/home/fin/luna`, using `python3 -B development/check.py --timeout 900 --`: `test -p comptime_interpretor --lib --test abstract_values --test normal_values --test nominal_types` passed 66 + 16 + 17 + 23 tests (`/tmp/luna-tools-svsd0g9i/result.json`). `test -p luna --test nominal_value_parameters --test type_spreads --test newtypes --test normal_patterns` passed 261 + 92 + 207 + 32 tests (`/tmp/luna-tools-5nuxz7gq/result.json`). Formatting passed (`/tmp/luna-tools-_wvdegls/result.json`); git diff --check passed. Changes remain uncommitted on dev; overall language completion remains open.

### Integration checkpoint: scoped inputs through closures and concrete IIR

After the scoped input work, ran the complete comptime_interpretor crate suite and the affected higher-level closure/CMTT/memory suites. No new implementation regression was found. Added two source regressions: a closure captures Token<symbol> and preserves its dependent environment/result identity while rejecting a call claiming Token<true>; a generic function signature calls a comptime type constructor with a symbolic Boolean, then concrete source calls instantiate distinct true/false IIR function keys. Both NIR execution and concrete IIR execution return the expected value, and cross-instance source arguments are rejected.

Validation in `/home/fin/luna`, via `python3 -B development/check.py --timeout 900 --`: `test -p comptime_interpretor` passed all 179 tests (`/tmp/luna-tools-1hvngf64/result.json`). `test -p luna --test closure_environment_parameters --test comptime_memory --test cmtt --test cmtt_methods --test quote_functions` passed 20 + 12 + 513 + 142 + 103 tests (`/tmp/luna-tools-x_5enjf7/result.json`). The new dependent closure regression passed (`/tmp/luna-tools-c_cvw42l/result.json`); the new source/NIR/IIR regression passed (`/tmp/luna-tools-fng92zb9/result.json`). Formatting passed (`/tmp/luna-tools-a7us5ok6/result.json`); git diff --check passed. This is stage integration evidence, not completion of the language or native backend. Changes remain uncommitted on dev.

### Staged parameters in callable type descriptions

Callable type syntax now lowers positional comptime parameters into HIR stage flags. Both static elaboration and computed FunctionType construction preserve them; computed Type domains become Type inputs, other comptime domains become ComptimeValue inputs, independently of the callable's own phase. Unnamed fn/comptime fn signatures are accepted as block-tail type expressions. NIR retains the input-stage vector in its S-expression dump, charges its copy cost, validates its arity, and preserves it through rewriting. The interpreter rejects malformed vectors before invoking parameter-domain computations and retains source evaluation order.

Validation in /home/fin/luna via development/check.py: comptime_interpretor function_types (6) and luna function_type_parameters (4) passed, including mismatch rejection, computed Type domains, phase independence, malformed NIR, recovery, rewriting and evaluation order (/tmp/luna-tools-g3x8as7e/result.json). Parse precedence (10) passed (/tmp/luna-tools-661zrgp3/result.json). Expanded luna closure_environment_parameters passed 20; open_functions passed 387 and failed 2 (/tmp/luna-tools-ntku62kv/result.json): generic_aggregate_forwarding_defers_pointer_layout_until_instantiation rejects the pure closure wrapper for missing P4 evidence, and ownership_only_closure_analysis_does_not_require_nominal_layouts requests unavailable nominal pointer layout. These failures are unresolved; this run does not establish whether they predate the change. Formatting passed (/tmp/luna-tools-amw9t4w2/result.json), as did git diff --check. Changes remain uncommitted on dev.

### Closure guard projection preserves generic ownership analysis

Resolved both failures from the callable-type integration run. prepare_closure_arguments already maps pointer evidence conditionally, but its subsequent guard-projection step unconditionally requested pointer_input_layout even when no symbolic target existed. It now requests that layout only when there is a nonempty target set to refine. Empty leaf entries remain intact, and concrete pointer origins still undergo the same guarded lifetime checks. This does not suppress a failed layout query or treat unknown pointers as absent.

Validation in /home/fin/luna via development/check.py: all 389 open_functions tests passed, including the previously failing generic pure forwarding case (concrete Ticket and pointer instances, expired-pointer rejection, duplicate-consumption rejection) and ownership-only closure analysis (/tmp/luna-tools-_ka3jfs2/result.json). captured_memory (94) and closure_environment_parameters (20) passed, including conditional payloads, recursive closure factories, cleanup and expired-target rejection (/tmp/luna-tools-zege6yzg/result.json). Formatting passed (/tmp/luna-tools-l7ndgv0j/result.json); git diff --check passed. Changes remain uncommitted on dev.

### Higher-order staged runtime callback specialization

A new end-to-end regression exposed that fn(comptime bool, bool) callbacks type-checked but failed IIR dependency materialization with "staged runtime function requires a specialized direct call" (/tmp/luna-tools-qlblolrz/result.json). Materialization now expands ordinary higher-order runtime calls whose callback interface contains staged inputs, using the existing contract-preserving argument/return expansion. The expanded argument frame forwards immutable named template identities and aliases, then feeds their calls into the existing staged instance machinery. Argument evaluations stay in the original preparation frame. Alias collection and copying are bounded; writes, address-taking and closure captures prevent propagation.

Source evidence: function_type_parameters (8), open_functions (389), closure_environment_parameters (20) passed (/tmp/luna-tools-3i659oma/result.json). New cases verify concrete IIR execution, rejection of runtime input to a comptime slot, distinct true/false target instances across repeated calls and nested forwarders, and single evaluation/source order of state-mutating ordinary arguments. This is not the full representation of runtime-selected template identities: conditional target selection, mutable/escaping template values and recursive higher-order specialization remain unfinished and are documented as implementation gaps. Changes remain uncommitted on dev.

The 7 materialization callable unit tests also passed (/tmp/luna-tools-brtcewic/result.json), including alias write/address/capture preservation and zero-fuel rejection. Formatting passed (/tmp/luna-tools-vro37t1r/result.json); git diff --check passed. An initial new unit fixture used Box instead of the required Arc closure body; that fixture compilation error was corrected before this successful run.

### Runtime selection of staged callback targets

Extended staged callback materialization beyond fixed named aliases. Immutable if/match/block selections now evaluate at their original binding point into a fresh runtime tag; every later use dispatches over the same tag to known template targets. Target calls use ordinary staged instance specialization. The selection's guards, branch statements and diverging exits remain executable in their original order. Normalization resolves these aliases before dependency discovery, including bindings outside the immediately expanded higher-order call. Writes, address-taking and closure captures still prevent this propagation.

A new source/IIR regression first reproduced the unsupported callback dependency (/tmp/luna-tools-3smleu0o/result.json). The implementation now passes both runtime choices while a later argument mutates the condition's original storage; repeated calls keep the initially selected target. Additional tests cover guarded match selection through aliases and early return before argument effects. function_type_parameters (11), open_functions (389), closure_environment_parameters (20) and quote_functions (103) all passed (/tmp/luna-tools-wf5xxh4d/result.json). Formatting passed (/tmp/luna-tools-mvk4b893/result.json), git diff --check passed. Mutable/escaping template values and recursive higher-order specialization remain open. Changes remain uncommitted on dev.

All 9 materialization unit tests also passed (/tmp/luna-tools-xaoo7hq1/result.json), covering alias observation barriers and conditional-call budget/scope behavior.

### Alias-dependent template selection and contract boundary

Conditional template extraction now resolves established immutable target aliases before enumerating alternatives. The previous path rejected an if branch returning a local alias even though the same target worked when named directly; reproduced in /tmp/luna-tools-wv3h5zf1/result.json. Alias substitution is bounded and retains the write/address/capture exclusions. Nested selections read previously saved tags, preserving each selection's original evaluation point rather than replaying its producer.

Source/IIR tests now cover chains of named aliases, nested selections with state-mutating conditions, repeated specialization and contract handling. The contract audit confirmed that a named function with program preconditions is rejected when converted to a plain callback type that cannot represent those preconditions. The accepted case preserves wrapper preconditions/results and target postconditions; invalid wrapper inputs and strengthened target preconditions remain rejected. No callable-contract representation support is claimed.

Validation in /home/fin/luna through development/check.py: function_type_parameters (14) and open_functions (389) passed (/tmp/luna-tools-r5j11zaf/result.json). Formatting passed (/tmp/luna-tools-3hqrg0_1/result.json), git diff --check passed. Changes remain uncommitted on dev.

The 9 materialization unit tests also passed (/tmp/luna-tools-z4bd6rk3/result.json).

### Staged trait methods as higher-order callback targets

Template alias forwarding now retains K::TraitMethod identities as well as named function identities. It leaves interface inputs, subject and method arguments intact until the existing method-call specialization selects the body and merges interface obligations. Previously a method assigned to a callback local and passed to a higher-order wrapper reached IIR as an unspecialized runtime template; /tmp/luna-tools-op9_tvih/result.json reproduced this failure.

New source/IIR tests cover repeated staged calls, runtime selection between value-indexed trait default methods (Select<true>/Select<false>), and an explicit runtime receiver before the comptime argument. A rejection test confirms that a method value cannot erase interface preconditions even when its implementation has none. Validation in /home/fin/luna via development/check.py: function_type_parameters (18) passed (/tmp/luna-tools-71xj5vfv/result.json); elaboration method_values (9) passed (/tmp/luna-tools-tlj8nfcq/result.json). Formatting passed (/tmp/luna-tools-o86h7ffi/result.json); git diff --check passed. Changes remain uncommitted on dev.

### Finite mutable staged-callback locals

Added a materialization pass for local staged callable storage with a finite, known target set. It collects the initializer and direct whole-local assignments, converts each selection into a shared runtime tag space, and rewrites reads into dispatch over concrete template identities. Selection code remains at each original initialization/assignment. Immutable aliases snapshot the tag through the existing selection-freezing pass. Address-taking and closure capture exclude a local from this transformation; unknown RHS target sources and projected assignments are not silently approximated.

The baseline mutable callback failed IIR materialization (/tmp/luna-tools-guzyq733/result.json). New source/IIR regressions verify old aliases after reassignment, callee selection before an argument reassigns that same callback, and loop rebinding with a side-effecting condition evaluated once per iteration. A new unit regression verifies that address escape prevents rewriting and zero fuel rejects the pass. One source fixture initially used a bare statement block as an argument; it was corrected to the specified do-block syntax.

Validation in /home/fin/luna via development/check.py: function_type_parameters (21), open_functions (389), closure_environment_parameters (20), quote_functions (103) passed (/tmp/luna-tools-k0uwalo7/result.json). Formatting passed (/tmp/luna-tools-0euj44zx/result.json), git diff --check passed. Escaping callback representation, assignment from unresolved target sources and recursive higher-order specialization remain unfinished. Changes remain uncommitted on dev.

All 10 materialization unit tests passed (/tmp/luna-tools-n9fh44tk/result.json).

### Transfers between finite staged-callback locals

Generalized mutable callback lowering from independent literal-target locals to connected local target domains. Initializers/assignments record dependencies on other callable locals; a bounded component walk establishes a common finite candidate table while allocating an independent runtime tag for each local. Assignment and self-assignment copy the current tag, and immutable saved aliases retain the earlier selection. Conditional/match RHS values preserve evaluation at the assignment site. Unknown source bindings, address escape, closure capture, mismatched domains and projected writes prevent conversion of the affected component.

The new cross-local regression first failed with an unspecialized staged runtime target (/tmp/luna-tools-d6b21ocm/result.json). Source/IIR tests now cover mutual assignment cycles, restoring from a saved immutable alias, self-assignment, conditional selection between locals, and callee evaluation before an argument reassigns it from another local. A unit test checks that escaping aliases and unknown incoming sources keep the component unchanged.

Validation in /home/fin/luna via development/check.py: function_type_parameters (23), open_functions (389), closure_environment_parameters (20), quote_functions (103) passed (/tmp/luna-tools-mil4xn2p/result.json). Formatting passed (/tmp/luna-tools-loaxzrns/result.json), git diff --check passed. Cross-interface escaping storage and recursive higher-order specialization remain unfinished. Changes remain uncommitted on dev.

All 11 materialization unit tests passed (/tmp/luna-tools-w5o1zbhp/result.json).

### Runtime factories returning staged callbacks

Concrete runtime factories returning a staged callable interface now expand before local target collection. Argument frames and factory body effects remain runtime expressions; CallPreparation is retained as a scoped selection boundary. The path also handles factories with their own ordinary comptime inputs after those inputs are bound. Runtime factory execution is not substituted with comptime invocation.

Explicit returns required an additional representation step: ordinary inline-return lowering used a generated enum slot containing the unspecialized callable. For factories whose completing return targets can be enumerated, a private bound function copy returns a tag through the existing return/cleanup machinery, then reconstructs the callable selection for downstream specialization. Result predicates retain their callable view through dispatch over the returned tag; addressed/captured result storage is excluded from this adaptation rather than silently changing its layout.

Reproductions: the initial factory failed in /tmp/luna-tools-e_dso_me/result.json; early return failed in /tmp/luna-tools-4t_2448k/result.json; a factory with its own comptime inputs failed in /tmp/luna-tools-a7lf9hrp/result.json. New source/IIR cases verify both runtime selections, side-effecting factories evaluated once, forwarding and reassignment, explicit early return, factory precondition rejection, and staged factory inputs without early runtime side effects.

Validation in /home/fin/luna via development/check.py: function_type_parameters (28), open_functions (389), closure_environment_parameters (20), quote_functions (103) passed (/tmp/luna-tools-g87vklwc/result.json). Formatting passed (/tmp/luna-tools-lr6j5l0z/result.json), git diff --check passed. Arbitrary recursive factories, unresolved target producers and general escaping storage remain unfinished. Changes remain uncommitted on dev.

All 11 materialization unit tests passed (/tmp/luna-tools-a6hxxp01/result.json).

### Trait-selected staged-callback factories

Factory specialization now routes ordinary-input trait methods returning staged callbacks through concrete interface selection, including when encountered inside another factory expansion. The method's interface type/capability obligations, program predicates, result binding and entry snapshots are merged into the bound body before either ordinary or staged-input expansion. Extracted the existing merge logic so both paths retain the same contract behavior.

The value-indexed default-method regression reproduced an unspecialized target failure (/tmp/luna-tools-knprz4ih/result.json). Source/IIR tests now cover Factory<true>/Factory<false> defaults, generic receiver forwarding into an implementation with weaker preconditions, interface-call rejection for a false precondition, and methods with their own comptime inputs returning distinct callback choices.

Validation in /home/fin/luna via development/check.py: function_type_parameters (31), quote_functions (103), trait_purity (20) passed (/tmp/luna-tools-dcykgi8y/result.json). Formatting passed (/tmp/luna-tools-0kkntb0w/result.json), git diff --check passed. Changes remain uncommitted on dev; general recursive/escaping template storage remains open.

All 11 materialization unit tests passed (/tmp/luna-tools-bvsss9gx/result.json).

### Early factory returns of local template choices; resource-call frames

Before enumerating explicit factory returns, the private bound body now lowers finite mutable callback locals and immutable aliases. This allows early return of a saved alias while the current local has already been rebound, preserving both choices. Reproduced the missing path in /tmp/luna-tools-6kd_d1da/result.json; the four runtime branch combinations now pass.

A resource-checked factory test exposed a separate IIR staging defect: specialization of an indirect source call kept an empty parameter_bindings vector even after selecting a concrete body (/tmp/luna-tools-3fbdrpeq/result.json). bind_call now publishes the specialized function's remaining runtime parameter frame. This lets memory/default analysis prepare the call rather than rejecting it with "memory call requires a complete unique parameter binding frame". Both early-return and fallthrough factory cases now run with a local resource destructor and preserve the returned callback choice.

Factory return conversion also visits executable closure capture initializers while leaving closure-body returns alone. A focused unit test checks these distinct scopes. Result postcondition storage boundaries remain separately guarded.

Validation in /home/fin/luna via development/check.py: iir staging tests (40) passed (/tmp/luna-tools-gkyaux50/result.json). Source function_type_parameters (33), captured_memory (94), open_functions (389), quote_functions (103) passed (/tmp/luna-tools-0h7ot68k/result.json). After the capture-return scope adjustment, all 12 materialization unit tests passed (/tmp/luna-tools-qlsmuyu1/result.json). Formatting passed (/tmp/luna-tools-6ezqkmln/result.json), git diff --check passed. Changes remain uncommitted on dev; general recursive/escaping template representation remains unfinished.

The final function_type_parameters source suite passed all 33 tests after the scope adjustment (/tmp/luna-tools-aybc9pxf/result.json).

### Template specialization budget/recovery and recursive-factory diagnostic

Added failure/recovery checks around the accumulated staged-callback paths. Instantiation with budgets 0, 1, 16 and 128 fails; retrying with sufficient fuel produces an identical IIR S-expression dump and executes successfully. The same source package can also instantiate an independent supported root after a recursive factory root fails.

Recursive factory calls remain an implementation gap. They previously fell through to the unrelated "staged runtime function requires a specialized direct call" diagnostic (/tmp/luna-tools-uurr40zr/result.json). Active factory re-entry now reports the factory name and explicitly identifies missing recursive finite-target analysis. This is not a claim that recursive factories are illegal language programs or implemented.

Validation in /home/fin/luna via development/check.py: all 35 function_type_parameters tests passed (/tmp/luna-tools-m1yu2d1a/result.json). Formatting passed (/tmp/luna-tools-vptodxui/result.json), git diff --check passed. Changes remain uncommitted on dev.

### Finite recursive staged-callback factories

Replaced the unsupported boundary for finite direct named factory call graphs with generated runtime helper functions returning a shared target tag. Graph discovery handles self recursion, mutual recursion and finite generic instances. Helpers preserve ordinary argument frames, contracts, effects and explicit returns; recursion is executed at runtime rather than expanded indefinitely. Graph planning and copies consume materialization fuel; helpers are published only after the complete plan is valid. Dynamic recursive trait edges, staged higher-order inputs and unbounded generic instance growth remain unsupported.

Resource-checked execution exposed duplicate temporary boundaries in the generated selection binding. Helper calls now retain separate expansion locations, and frozen selection bindings receive their own generated source identity. The regression verifies one destructor per executed recursive frame. Another regression verifies runtime depth exhaustion and successful subsequent invocation using the same interpreter. Added examples and implementation boundaries in docs/functions/parameters.md.

Validation in /home/fin/luna via development/check.py: function_type_parameters (39), captured_memory (94), open_functions (389), quote_functions (103) passed (/tmp/luna-tools-bxzycpwa/result.json). After adding function-copy/type budget accounting, all 12 materialization unit tests passed (/tmp/luna-tools-9y0rw8vi/result.json), and the 39 function_type_parameters tests passed again (/tmp/luna-tools-ulhkpv7p/result.json). Formatting passed (/tmp/luna-tools-00kpq7oi/result.json); git diff --check passed. Changes remain uncommitted on dev.

### Local storage of recursive factory selections

Recursive finite-target discovery now includes immutable callback local initializers and aliases. Helper generation encodes those initializers and returned local reads as tags, reconstructing dispatch only for remaining callable uses. This permits invoking a recursively produced callback inside the factory before returning it, without replaying its producer. Assigned, addressed or captured local storage remains excluded; unknown initializer targets still reject the plan.

The source regression initially failed with the recursive-factory unsupported diagnostic (/tmp/luna-tools-_xt1x2ad/result.json). It now checks both candidate targets, alias calls, early return, repeated downstream calls and exactly two helper executions. Additional cases cover dependent defaults with explicit override and preserved entry-snapshot postconditions. Resource execution now tests direct and saved-result returns in both recursive and base cases.

Validation in /home/fin/luna: development/check.py test -p luna --test function_type_parameters --test captured_memory passed 42 and 94 tests (/tmp/luna-tools-9df5oj3v/result.json). The expanded resource case passed separately (/tmp/luna-tools-3f4j5oj0/result.json). An additional exploratory variant containing an unconditional return followed by an unreachable template-valued tail still fails materialization (/tmp/luna-tools-3nnw67hi/result.json); unreachable template residual elimination remains unfinished, independent of the tested reachable local-storage paths. Changes remain uncommitted on dev.

### Unreachable template tails before dependency discovery

Resolved the unreachable template tail recorded above. A bounded pass removes block/CallPreparation suffixes after a definitely terminating statement (Never, return, break, continue, including discard wrappers), retaining the transfer and preceding evaluation. It runs before runtime factory discovery and in private recursive factory plans. This avoids discovering unspecialized callable values in code that cannot execute while preserving source-level elaboration checks.

The resource regression first reproduced the failure (/tmp/luna-tools-_fox0hux/result.json), then passed with an explicit return followed by an unreachable template tail (/tmp/luna-tools-a0gtmgdr/result.json). Source regressions also cover loop break/continue suffixes and a conditional early return whose other branch must execute the following state update.

Validation in /home/fin/luna via development/check.py: all 44 function_type_parameters tests passed (/tmp/luna-tools-ewwtb6wu/result.json). The inline_control, spread_control, match_cleanup, quote_functions, open_functions and captured_memory suites passed (/tmp/luna-tools-7mtmtdf0/result.json). Workspace formatting passed (/tmp/luna-tools-or2vnnd_/result.json), git diff --check passed. Changes remain uncommitted on dev. This removes proven-unreachable sequence tails; it is not a general control-flow optimizer or a termination proof.

### Mutable selections inside recursive factories

Extended recursive finite-target discovery to whole-local assignment RHS values, and encode those writes using the same target domain as initializers and returns. This supports saved immutable aliases, restoring prior selections, cross-local loop transfers and self-assignment. Call-site dispatch continues to freeze the callee before argument evaluation. Addressed/captured locals, projected writes and callback writes to external storage remain excluded rather than being silently reinterpreted as tags.

The mutable factory baseline failed with the unsupported-recursion diagnostic (/tmp/luna-tools-pw0d9e3y/result.json). New regressions cover both targets and restore branches, reassignment inside a runtime argument, loop transfers, plus rejection/recovery for addressed storage. The resource-checked recursive test also now reassigns and restores a saved callback before return, including an unreachable tail; per-frame cleanup still passes.

Validation in /home/fin/luna via development/check.py: all 48 function_type_parameters tests passed (/tmp/luna-tools-4y1lai0u/result.json), and all 12 materialization unit tests passed (/tmp/luna-tools-t2yr7h38/result.json). Workspace formatting passed (/tmp/luna-tools-4x4yqbz_/result.json); git diff --check passed. Changes remain uncommitted on dev.

### Discarded finite template selections

Materialization now converts discarded finite staged-callable selections into tag computations after factory exposure and alias forwarding. Calls, branch conditions, statements and control transfers are retained; only the unused concrete template target representation is removed. This fixes recursive factory calls used as statements, both inside their own bodies and in callers. Closures and unknown producers are not handled by this transformation.

The recursive statement case reproduced an unspecialized-runtime-template failure (/tmp/luna-tools-2g_uo4cn/result.json). New regressions cover recursive call effects, discarded immutable aliases, conditional selection with early return, and per-frame destruction when both inner and outer factory results are discarded. A separate successful regression verifies passing a saved recursive selection to an ordinary higher-order wrapper.

Validation in /home/fin/luna via development/check.py: all 52 function_type_parameters tests passed (/tmp/luna-tools-w_qrxso0/result.json); captured_memory (94), open_functions (389), quote_functions (103) passed (/tmp/luna-tools-6e1fhs93/result.json). Formatting passed (/tmp/luna-tools-d5hx3vpx/result.json); git diff --check passed. Changes remain uncommitted on dev.

### Budgeted factory graphs without a fixed node cap

Removed the fixed 64-instance rejection from finite factory discovery. Node membership now uses an instance-key index; cycle detection builds reverse edges and processes zero-outdegree nodes with a queue instead of repeatedly searching all nodes. Discovery, edge processing, instantiation and copying remain charged to the existing materialization allowance. Ordered discovery/queues retain deterministic helper identities and dumps.

A 70-node cycle reproduced the old hard-cap failure (/tmp/luna-tools-7_pmsaol/result.json) and now materializes and executes. The regression also checks low-budget failure followed by a successful retry and identical repeated dumps. A diamond-shaped shared dependency stays on the acyclic expansion path. A generic factory whose recursive type argument grows by taking an address exhausts its work budget and leaves an independent root usable.

Validation in /home/fin/luna via development/check.py: all 55 function_type_parameters tests passed (/tmp/luna-tools-koceqo83/result.json); all 12 materialization unit tests passed (/tmp/luna-tools-hky9tty3/result.json). Formatting passed (/tmp/luna-tools-raq__1gq/result.json); git diff --check passed. Changes remain uncommitted on dev. Unbounded generic instance families still have no finite residual representation; this change does not claim to implement them.

### Error-result surface retained through HIR

Investigation of error/recursive-factory interaction found that error-qualified types and elimination had no complete AST-to-HIR path. Added error row/result operands, dedicated propagation, and an error handler containing separate before/ordinary/after arms. Catch parsing now accepts patterns rather than identifiers only. Lowering checks trailing-single or leading/trailing-double catch placement, preserves arm scope, and retains success/error pattern wrappers and their local bindings. Updated the HIR open-function traversal and S-expression Display, including structured payload assertions.

Added error_surface frontend tests for rows, construction, propagation, catch roles and general patterns, ordinary match payload binding, invalid placement/count and binding leakage. The initial ordinary-match test exposed missing binding traversal through error pattern wrappers (/tmp/luna-tools-ex950ezl/result.json), which was fixed. These tests deliberately stop at HIR; they do not claim executable error ABI support.

Validation in /home/fin/luna via development/check.py: check -p luna passed (/tmp/luna-tools-rrk28q26/result.json). HIR/parser suites passed 85 tests, with 3 existing ignored doc tests (/tmp/luna-tools-3aitu8p5/result.json); strengthened HIR Display assertions passed separately (/tmp/luna-tools-_he0l2al/result.json). error_surface (5), enum_configurations (2), iteration (2), quote_functions (103) passed (/tmp/luna-tools-1n8g6kh5/result.json). Formatting passed (/tmp/luna-tools-na8vi3io/result.json); git diff --check passed. Changes remain uncommitted on dev.

Remaining error-system work: NIR error row/result representation and validation, success injection/error embedding, propagation and elimination elaboration, contracts/resource cleanup, and runtime result ABI. Keep this distinct from algebraic-effect continuation handling.

### Closed error-row representation plans

Added nir::error_results with checked private row storage, explicit identity evidence (Same/Distinct/Unknown), closed type validation and bounded work. Rows retain deterministic first occurrence order and deduplicate only established identities. Exact membership can reuse validated uniqueness without requesting unrelated new proofs. Error embedding records a complete tag map; residual selection records handled versus remaining inputs and does not constrain the independently constructed output row. Invalid tags/missing labels reject instead of silently dropping errors.

ResultType plans merge nested qualification, keep empty rows plain and preserve success normal-form heads. Transport explicitly distinguishes Plain, InjectSuccess and Embed, checks success compatibility independently, and rejects narrowing that would lose an error. Every descriptor/failure has an S-expression Display. The public API does not expose mutable row internals or authorize runtime payload reinterpretation.

Validation in /home/fin/luna: development/check.py test -p nir --test error_results --test normal_inputs passed 8 new layout/transport tests and 11 existing input tests (/tmp/luna-tools-p8npktfu/result.json). Tests cover value-indexed labels, reorder/widen/narrow, residual/output independence, Unknown identity, evidence-based equality, nested/empty qualification, preserved heads, malformed/open inputs, invalid tags, budget failure/retry and exact lookup without new proofs. Changes remain uncommitted on dev.

This is a complete closed-row planning component, not executable error handling. It is not yet wired into Term, expression elaboration or the interpreter. Open rows, substitution/revalidation, payload construction/propagation and cleanup execution remain outstanding.

### Generic error-row layout instantiation

Added ResultTemplate and InstantiatedResult to the error representation planner. Declaration-side labels do not carry tags; bounded mixed-sort substitution completes success/label types, validates all inputs including unused parameters, normalizes the resulting row and publishes an occurrence-to-tag map. Repeated or newly equal labels share a completed tag. Template construction validates complete parameter frames, rejects undeclared rigid dependencies and checks scope without unfolding normal-form heads. Frame/copy work is charged before allocation; descriptors and mappings have S-expression Display.

New tests exercise [T,U,T] merging versus distinct instantiation, value-indexed nominal errors, preserved normal-form success heads, invalid/unused inputs, duplicate or undeclared parameters, invalid occurrence indices, budget failure/retry and repeatable dumps. This extends the planning component; executable error terms/values remain unwired.

Validation in /home/fin/luna via development/check.py: error_results, nominal_parameters and normal_inputs all passed (/tmp/luna-tools-m0cy3564/result.json). Formatting passed (/tmp/luna-tools-9wjzybfq/result.json), git diff --check passed. Changes remain uncommitted on dev.

### ErrorResult integrated into stable Term infrastructure

Added Term::ErrorResult with full type labels and a success type, plus a constructor that merges nested qualifiers, removes exact duplicate labels and erases empty qualification without unfolding normal-form heads. Term rewriting/substitution, scope/sort validation, runtime-dependency discovery, rigid-parameter discovery, iterative disposal and S-expression Display now retain its children. Updated bounded type/copy walkers, completed-value preflight, normal-input validation, associated-type normalization, memory summary type bounds and interpreter type-size accounting. Coherence ownership stays conservative, and interpreter admission explicitly rejects this currently non-executable type instead of silently treating it as an ordinary admitted value.

The error layout planner now converts to/from Term. Generic templates reconstruct nested success qualifiers correctly, and the plain-success constructor rejects a hidden ErrorResult layer. New regressions check substitution-induced duplicate collapse, round trips, exact dumps, label-side runtime dependencies, malformed label sorts, scoped labels, depth-budget traversal and nested qualification.

Validation in /home/fin/luna via development/check.py: check -p luna passed (/tmp/luna-tools-brhi_yva/result.json); the full nir suite passed 303 tests (/tmp/luna-tools-cbeme4ls/result.json). After the nested-success boundary fix, all 14 error_results tests passed (/tmp/luna-tools-kk2snw9i/result.json). error_surface (5), effect_spreads (19), function_type_parameters (55) passed (/tmp/luna-tools-0z7j45qr/result.json). Formatting passed (/tmp/luna-tools-nvautkpe/result.json), git diff --check passed. Changes remain uncommitted on dev.

Remaining: source type formation and solver equality/row evidence, executable success/error values and embedding/propagation/elimination, resource cleanup and runtime ABI. Stable Term support does not claim these downstream paths are complete.

### Source error types and PTC set congruence

Connected ErrorQualifiedType to type-head recognition, type formation and signature-input readiness. Explicit singleton/list rows produce Term::ErrorResult, preserving generic labels and merging exact duplicates/nesting; empty rows erase. Added a distinct PTC error-row forward-congruence rule using only current branch equivalence classes. It treats labels as a set, permits permutation without assuming distinctness and coalesces labels only under established equality. Type input budgets/subterm discovery/bound detection traverse errors and success. This rule does not infer absence, disequality, injectivity or runtime representation equivalence.

Source tests cover closed declarations, aliases, generic labels, nested/empty rows, row permutation, non-type labels, incompatible result types and successful execution of an empty qualifier. Dedicated PTC tests cover unknown-label commutation, assumption-dependent duplicate collapse with proof replay/foreign-context rejection, raw empty/nested wrappers and non-proofs for unmatched labels or differing success types.

Validation in /home/fin/luna via development/check.py: error_rows (4), effect_rows (14), ptc (56) passed (/tmp/luna-tools-dprxquo1/result.json); error_surface (12) and effect_spreads (19) passed (/tmp/luna-tools-rnxrh359/result.json). Formatting passed (/tmp/luna-tools-6kreqatq/result.json), git diff --check passed. Changes remain uncommitted on dev.

Nonempty result construction/injection, branch remapping, propagation/elimination and resource execution are still missing. Interpreter admission remains closed for ErrorResult. Row equality must not bypass representation mapping when these paths are connected. General computed/open error rows and stronger row theory also remain incomplete.

### Evidence-based error-row disequality

Added an error-result refutation rule to PTC. With both residual success types proven ordinary, it can refute equality from success-type disequality or a label proven distinct from every label in the opposite row. It reuses the existing branch-local type-class disequality machinery; Unknown is never treated as absence. Opaque success residuals remain pending because they may contain further error qualification, and ordinary residual evidence may come from current branch equalities.

Tests cover closed-row missing labels, distinct success types, unknown versus assumed-distinct labels, foreign-context proof rejection and opaque residuals that become ordinary under an assumption. A finite-set oracle checks 256 closed-row/success combinations, including empty qualifiers and permutations, with proof replay for each result.

Validation in /home/fin/luna via development/check.py: error_rows (7), effect_rows (14), ptc (56) passed (/tmp/luna-tools-t1547tzt/result.json); after adding the oracle, all 8 error_rows tests passed (/tmp/luna-tools-fnso0kxe/result.json). Formatting passed (/tmp/luna-tools-p9bn99ih/result.json), git diff --check passed. Changes remain uncommitted on dev. This adds type-theory evidence only; executable error construction/propagation/cleanup remains outstanding.


### 错误结果构造与交付的执行接线

- NIR `ErrorConvert` 区分成功注入、失败构造与已有结果的嵌入；共用有界转换验证，补齐表达式遍历、替换、阶段检查、预算、常量物化及 S-expression。源码上下文向成功值及单标签错误载荷提供类型提示。
- IIR 校验转换和错误行内嵌类型，实例化收集错误标签的名义描述。参考解释器增加完整类型／分支／载荷表示，换序和扩行按标签映射、保持载荷一次求值；host admission 拒绝错误标签和载荷。修复 IIR 名义类型 admission 对已实例化描述错误地再次使用源参数表的问题。
- 验证（cwd `/home/fin/luna`）：`python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test effect_spreads --test function_type_parameters`，退出 0，19 + 19 + 55 项通过，`/tmp/luna-tools-zjan11ui/result.json`。`test -p nir -p iir -p comptime_interpretor` 经同一入口，退出 0，共 549 项通过，`/tmp/luna-tools-f0395nag/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-28tko35i/result.json`。
- 边界：错误传播／消去、前后 catch、仿射责任与清理、完整纯度分析、错误值的 normal-form 输入冻结尚未完成；直接 IIR 解释器结果不能当作资源安全认证。运行时转换只接受实例化后直接匹配的身份，未解决的表示等式继续拒绝。改动保留 dev 工作区，未提交。

### 显式错误传播与模式内嵌类型

- `expr!` 在明确函数结果上下文下形成普通 Match／Return：成功分支交付载荷，错误分支逐标签检查目标行并构造返回值。保留错误载荷和一次操作数求值，允许调用者成功类型不同；不增加隐藏返回或 continuation ABI。
- 新增 NIR ErrorResult 模式，标签使用完整类型身份，随泛型实例化替换；配套 S-expression、预算、绑定／表达式遍历、类型重写、解释器分支选择。IIR 单独验证模式内嵌类型，拒绝非 Type 标签和残留声明变量。内联返回沿用普通控制流改写。
- 验证（cwd `/home/fin/luna`）：`python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test quote_functions --test inline_control` 退出 0，25 + 103 + 241 项通过，证据 `/tmp/luna-tools-14kpa65j/result.json`；同入口 `test -p nir -p iir -p comptime_interpretor` 退出 0，549 项通过，证据 `/tmp/luna-tools-ly1ren8f/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-r792el9g/result.json`；`git diff --check` 通过。
- 未完成：源级错误模式消去、前后 catch、返回类型推断下的延迟传播检查、错误结果资源责任与完整清理接线。直接参考执行仍不构成资源安全认证。改动保留 dev，未提交。


### 源级错误模式与结果覆盖

- 普通 match 接入成功模式、错误载荷模式和错误通配符；单标签上下文提供载荷类型，多标签通过限定枚举 variant 选择完整身份（含具名字段与同头不同实参）。绑定收集遍历错误包装，支持 guard 回退、泛型载荷及 let-else；多标签无可识别类型的载荷模式仍诊断，不猜测身份。
- 覆盖矩阵拆分成功与每个错误分支，递归分析载荷，Never 成功主体为空域。增加 16 种有限分支集合的独立覆盖对照、预算耗尽及恢复检查；遗漏成功、错误或载荷值的源码拒绝例都检查实际诊断。
- 验证（cwd `/home/fin/luna`）：`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib coverage::` 退出 0，53 项通过，`/tmp/luna-tools-mfjsc5er/result.json`。同入口 `test -p luna --test error_surface --test normal_patterns --test list_patterns --test symbols` 退出 0，34 + 32 + 17 + 29 项通过，`/tmp/luna-tools-xuerwkzg/result.json`。全 workspace 格式化日志 `/tmp/luna-tools-eze1___t/result.json`。
- 后续：专用错误 handler、catch 顺序、错误结果清理责任与完整纯度接线；当前普通匹配参考执行不等于资源安全认证。改动保留 dev，未提交。

### 普通错误 handler 与独立输出行

- 新增 program/error_handlers.rs，接通无 catch 的 `expr! { ... }`：操作数一次求值，成功直达，普通 arm 按载荷匹配，未覆盖／guard 未通过的错误保留在结果中。完整覆盖按输入标签判定，分支产生的错误独立参与输出；允许重新产生刚处理的标签，支持上下文目标及结果推导。
- 降低沿用 Match、ErrorResult 模式、ErrorConvert 和普通 Return，内联保留返回边界；错误结果与 effect ABI 分离。模式和 guard 的类型事实通过已有分支作用域接口检查。覆盖检查增加实际预算计量，逐标签使用共享 elaboration 预算，失败预算也记账，Unknown 不用于删除输入标签。
- 验证（cwd `/home/fin/luna`）：`python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test inline_control --test effect_spreads` 退出 0，42 + 241 + 19 项通过，`/tmp/luna-tools-ukmiqrqd/result.json`；同入口 `test -p elaboration --lib coverage::` 退出 0，54 项通过，`/tmp/luna-tools-zje80no1/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-__1li8ga/result.json`；`git diff --check` 通过。
- 前后 catch 仍明确拒绝，待原始完整结果的观察／交付及资源责任接通；没有通过生成隐式副本模拟该行为。完整仿射清理、纯度和传播中的延迟函数返回推断仍未完成。保留 dev 工作区，未提交。

### 错误结果的独立清理执行

- Drop 清理布局将 ErrorResult 作为分支载荷处理，取得成功／各错误分支的字段与析构证据；执行复用 cleanup switch，内部编号为成功 0、错误行位置加 1，不改变错误类型身份。预置分支证据在执行任何析构前校验。
- 清理私有载荷投影携带完整结果类型和活动分支；结果类型或分支改变后，旧投影拒绝访问。清理复制／快照遍历纳入该值节点，但不自动授予源码 Copy。上下文需求扫描检查所有可能载荷，避免指针或 callable 责任藏在错误限定里。
- 补齐 implementation 主体资格及证据重放对 ErrorResult 的支持，否则普通名义 Drop 候选不能排除而使自动清理永久待定。适用性／一致性及 opaque index 的嵌套遍历保留标签和主体中的类型参数，不把开放输入错误缓存成闭合 identity。
- 验证（cwd `/home/fin/luna`）：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface` 退出 0，778 + 42 项通过，`/tmp/luna-tools-kb_fy9fy/result.json`。同入口 `test -p elaboration --test implementation` 退出 0，65 项通过，`/tmp/luna-tools-zgsleq0q/result.json`；`test -p comptime_interpretor --lib` 退出 0，67 项通过，`/tmp/luna-tools-lt6g2092/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-4xxqtrn6/result.json`，`git diff --check` 通过。
- 新用例验证活动分支唯一析构、错误分支证据无副作用拒绝、重复清理拒绝、析构失败保留写入并停止兄弟字段清理，以及类型／分支改变后投影失效。未完成：源码构造、模式、传播与 catch 的全路径责任转移和清理调度；独立清理程序通过不能替代这些检查。保留 dev 工作区，未提交。

### 错误结果的源码责任转移

错误构造现与普通聚合共用字段准备／转移规则。成功和错误载荷通过私有分支布局取得清理责任；整体移动、重新赋值、嵌套聚合和跨函数交付保持责任唯一。行扩展／重排映射符号路径、分支域及指针 guard，物理载荷位置保持不变，新增分支的不可达证据不能替代原有分支的 Unknown 来源。

新增 11 项源码测试覆盖：两种构造、移动和赋值、聚合、重复使用拒绝、栈指针逃逸拒绝、析构访问借用位置、谓词失败、部分构造失败、行映射及跨调用清理／借用依赖。仍未连接错误模式解构、传播和 catch 的全路径源码资源调度。

扩大回归发现新增映射局部变量增大递归分析栈帧，使原有固定 2 MiB 栈的深层内联测试溢出。已将映射提取为不内联的独立分析函数，保持原测试栈限制；定向复验和相关完整测试组均通过。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface`：退出 0，789 + 42 项通过，`/tmp/luna-tools-nc2piljz/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::`：退出 0，56 项通过，`/tmp/luna-tools-_ydfds2r/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-2y462y1z/result.json`；`git diff --check` 通过。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`：退出 0，67 项通过，`/tmp/luna-tools-soafg740/result.json`。本轮最终相关检查合计 954 项通过，改动留在 dev 工作区，未提交。

### 错误模式、传播与普通 handler 的源码资源调度

成功／错误模式按完整错误类型选择私有载荷分支，再复用字段转移、指针 guard 选择、残余责任和备选模式绑定规则。参考解释器的错误载荷匹配现在记录字段 0 的实际投影，供源码清理调度转移责任。备选模式的展开、导出绑定、绑定顺序及重映射均遍历错误模式；guard 失败保留原值，guard 不能消费仿射载荷。

传播最初的源码资源验收发现多个生成节点共用来源范围，导致 TemporaryEnd 无法唯一定位。传播和普通 handler 的生成读取／包装／Return 现使用已登记的独立 ExpansionSource，保留原始诊断来源。真正执行 inline 展开后的成功和错误路径均保持调用边界与析构顺序。

新增 10 项源码资源测试，覆盖两种模式、guard 回退与非法消费拒绝、借用载荷、嵌套备选模式、多错误标签备选／通配符、传播清理、真实内联展开及 handler 消费／重抛。前后 catch 与传播的延迟返回类型推断仍未实现。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test inline_control --test normal_patterns`：退出 0，799 + 42 + 241 + 32 项通过，`/tmp/luna-tools-wyl1_i9n/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::`：退出 0，56 项通过，`/tmp/luna-tools-k7sbc5fj/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-xmpfrey1/result.json`；`git diff --check` 通过。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib`：退出 0，67 项通过，`/tmp/luna-tools-29jk2xpl/result.json`。本轮最终相关检查合计 1,237 项通过，保留 dev 工作区改动，未提交。

### 前后 catch 的顺序控制流与资源检查

catch 以普通 Match／Block 传递同一个原始完整结果，不新增 continuation 或隐式 Copy。前置和普通 arm 正常完成时先丢弃其结果，再交付原值；后置 arm 交付最终结果或显式退出。保留整值时仅允许有 Copy 许可的重叠组成部分绑定，整体不正常完成的 arm 可直接消费。前置未匹配时继续，后置未匹配时保留原错误；输出行不吸收被丢弃的中间错误值。确定退出的前置／普通前缀可参与覆盖，但被普通完成分支遮住的后续退出不能据此消去错误。

新增 13 项源码资源测试和 5 项错误表面测试，覆盖顺序、成功跳过、原值身份、丢弃中间资源、非法消费拒绝、return／continue、guard 回退、谓词失败、泛型显式实例化、真正内联展开、后置解构，以及条件借用和重叠载荷位置的不同指针目标。

借用用例暴露并修复三处证据缺口：选择载荷时按实际类型筛选物理指针槽；错误模式用已知分支域排除不可能的成功分支；指针可能性分析识别 ErrorResult 的分支布局，区分未激活槽与未知地址。没有把 Unknown 当作无依赖，也未放松 Drop 对有效指针目标的检查。

仍有独立缺口：传播的延迟函数返回类型推断、完整纯度分析、错误 comptime 值冻结。泛型 catch 已支持实例化；仅由错误行反推类型实参仍可能 NeedsContext，本轮泛型验收显式提供类型实参，未将其报告为泛型推断完成。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test inline_control --test normal_patterns`：退出 0，812 + 47 + 241 + 32 项通过，`/tmp/luna-tools-ntxybw2v/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::`：退出 0，56 项通过，`/tmp/luna-tools-h9pf6z32/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-2kz1_u62/result.json`；`git diff --check` 通过。

最终相关检查合计 1,188 项通过。改动保留在 dev 工作区，未提交。

### 错误结果接入纯度与默认值行为分析

ErrorConvert 现在检查载荷计算并传递函数来源，构造／行扩展／重排不隐式调用函数或析构载荷。错误模式按完整类型定位载荷代码路径；循环中的构造、映射、绑定和赋值参与函数来源不动点，新增错误标签的不可达信息不覆盖原有未知来源。

修复清理行为分析的过度保守处理：已取得的清理选择可按剩余所有权树遍历，只检查实际剩余的组成部分。外层析构仍检查，未精化的值仍覆盖所有可能组成部分，未移动的兄弟字段不被跳过；该遍历不授予访问权限或 Copy，也不替代执行清理计划。

新增 9 项源码测试，覆盖构造／传播／catch、回调行重排、循环更新、不纯调用与未知函数来源拒绝、运行时默认值、已转移的不纯析构载荷，以及仍持有的不纯兄弟字段。测试需要析构证据时显式绑定 Copy／Drop 协议，没有通过默认假定无析构来放宽纯度。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test runtime_purity --test trait_purity --test default_calls --test error_surface`：退出 0，61 + 20 + 33 + 47 项通过，`/tmp/luna-tools-xvtaj6a5/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib program::purity::`：退出 0，7 项通过，`/tmp/luna-tools-axha_u4c/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-0p8spfi0/result.json`；`git diff --check` 通过。

最终相关检查合计 168 项通过。传播的延迟函数返回类型推断与错误 comptime 值冻结仍未实现；行为分析对未知代码身份和缺失清理证据仍保守失败。改动留在 dev，未提交。

### 闭包错误传播的延迟结果完成

未标注结果类型的普通闭包允许先检查传播输入，把错误出口保存在当前推断事务中；尾表达式／显式 return 提供成功结果后，合并传播标签、尾结果行与已观察的 return 错误行，完成失败出口及成功交付。显式 return 的转换逐个验证，不靠跳过类型检查接受不兼容成功结果。完成后仅导出普通 ErrorConvert／Match／Return。

新增状态随嵌套闭包、定义、默认输入与候选检查保存／恢复，使用独立展开来源识别传播出口。泛型 apply 和 inline apply 均能消费推断结果；资源执行确认成功路径与提前传播路径清理局部一次，错误路径不执行后续调用。具名函数签名规则不变；带 computation 结果的闭包仍要求明确结果上下文，错误 comptime 值冻结仍未实现。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test runtime_purity --test inline_control --test closure_environment_parameters`：退出 0，52 + 61 + 241 + 20 项通过，`/tmp/luna-tools-ztvf7zp2/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface inferred_error_callback`：退出 0，1 + 6 项通过，含随后新增的泛型／内联调用与资源执行用例，`/tmp/luna-tools-njjed064/result.json`。
- 本批新增 7 项测试；两轮最终相关检查覆盖 376 个不同用例。全 workspace 格式化退出 0，`/tmp/luna-tools-yap8dewi/result.json`；`git diff --check` 通过。改动保留 dev 工作区，未提交。

### 错误结果进入 CMTT 数据与闭包快照

生成器数据域检查递归检查错误标签与成功域；Never 成功域不要求构造值。错误结果重新物化使用已有受检 ErrorConvert 路径，载荷递归遵守生成器数据规则。闭包快照检查完整错误类型及载荷，required invocation 的活动数据扫描保留错误类型中的合法泛型参数。活动存储扫描也深入错误载荷，避免漏掉指针别名或混合 runtime 回调图。

新增 9 项源码测试，覆盖成功／错误交付、嵌套聚合与换序、runtime 函数身份、comptime 闭包快照、泛型标签、活动指针别名、先前 quote 的赋值隔离，以及指针逃逸／提前 runtime 调用／混合活动回调图拒绝。代码生成的数据重新物化与 normal-form 值实参冻结是不同边界；后者仍缺少错误值表示与完整身份支持，未报告为完成。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test cmtt --test cmtt_methods --test error_surface`：退出 0，522 + 142 + 53，共 717 项通过，`/tmp/luna-tools-4xpehk5k/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-kuwg1f4d/result.json`；`git diff --check` 通过。

改动保留 dev 工作区，未提交。

### 已知效应行下的闭包错误结果推断

高阶输入上下文独立保存已完成的 computation 行及尚未确定的结果域。泛型效应实参必须先完成；未知行不会凭闭包主体猜测。闭包检查取得这份允许行，在 computation 正文中推导结果；错误出口最终只改变结果中的错误行，外层效应行与 handler ABI 不变。普通／内联调用、显式 return 和泛型 callable 转发共用该路径。

缺少允许行的候选检查改为可等待上下文，修复泛型转发先探测闭包时的过早失败。已知允许行不包含请求操作仍拒绝。显式完整返回标注继续保留原有类型，不能因为正文是 Never 就改写公开结果。

新增 7 项源码测试：错误传播与 handler 组合、显式返回、未授权效应拒绝、泛型效应实参、普通结果类型不匹配拒绝、泛型 callable 转发以及成功／错误两条退出路径清理一次。未知效应行仍需补足上下文；normal-form 错误值冻结仍未实现。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test effect_spreads --test inline_control --test closure_environment_parameters`：退出 0，57 + 21 + 241 + 20 项通过，`/tmp/luna-tools-_pxhno5a/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop inferred_error_effect_callback`：退出 0，1 项通过，`/tmp/luna-tools-qsdpxoo7/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-4zbe61yw/result.json`；`git diff --check` 通过。

最终相关检查覆盖 340 个用例。改动留在 dev 工作区，未提交。

### 已知错误行的部分结果上下文与成功域推导

高阶上下文保留 `!E T` 中已知错误标签和待推导成功域，并可嵌入已知 computation 行。上下文提供单标签错误构造信息，成功值注入约定行，if／match／when 与泛型转发保留上下文。错误分支的 Never 成功域不会提前固定闭包结果；所有显式出口完成后统一校验交付，非法新增标签不能扩张声明行。

推断匹配器补充错误结果成功域的结构提案，仍以原始完整类型等式作最终义务。错误行不按位置配对，不从位置推断未知标签；行重排和重复标签由原有 PTC 规则判断。

新增 9 项测试，覆盖成功注入、两种分支顺序、显式失败／成功返回、效应组合、未知标签拒绝、match／when／泛型转发、错误行等式义务与资源清理。normal-form 错误值冻结和仅由错误行反推未知标签仍未实现。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test calls --test error_rows`：退出 0，38 + 8 项通过，`/tmp/luna-tools-opgnn_8i/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test effect_spreads --test closure_environment_parameters --test inline_control`：退出 0，63 + 21 + 20 + 241 项通过，`/tmp/luna-tools-1c2xh5gv/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop partial_error_callback`：退出 0，1 项通过，`/tmp/luna-tools-ud9c6yjg/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-tv6aih8c/result.json`；`git diff --check` 通过。

相关检查合计 392 项通过。改动保留 dev 工作区，未提交。

### 错误标签推导、后续证据收敛与实参行转换

新增有预算的错误行匹配模块：单一候选标签可提出替换，已注册 nominal 头排除不同生成类型，完整相同成功域下可用唯一未覆盖标签补齐唯一未知标签。集合合并不消耗重复标签；多个不同候选保持未决，opaque normal-form 头不反推实参。所有提案仍由原始完整类型等式校验。

修复两项集成缺口：匹配关系在推断状态实际改变后按共享预算迭代，后续实参可解除前面行的歧义；结构提案读取已经解决的替换组件。直接泛型调用完成推导后，为布局不同但语义兼容的错误实参补上 Embed，避免调用入口只按类型相等而忽略运行时标签排列。转换保留一次求值与原载荷责任。

新增 8 项测试，覆盖单标签／成功域独立推导、重复合并／唯一剩余标签、歧义排列与预算恢复、泛型枚举标签行重排、后续参数证据、opaque 头拒绝，以及行转换后拥有载荷清理一次。原先单标签保守拒绝的用例改为真正有多个候选的歧义用例，并另增单标签成功验收。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test calls --test obligations --test error_rows`：退出 0，41 + 9 + 8 项通过，`/tmp/luna-tools-sbztat_v/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test effect_spreads --test function_type_parameters --test inline_control --test closure_environment_parameters`：退出 0，67 + 21 + 55 + 241 + 20 项通过，`/tmp/luna-tools-hikhfbm9/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop inferred_generic_error_row_conversion`：退出 0，1 项通过，`/tmp/luna-tools-i5cledge/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-t_8wtrti/result.json`；`git diff --check` 通过。

相关检查合计 463 项通过。多候选错误行仍需其他证据或显式实参；错误 comptime 值的 normal-form 冻结仍待实现。改动留在 dev，未提交。

### 泛型产品实参内部的错误行布局转换

展开实参、固有方法、trait 方法验收通过；进一步定位并修复了元组内部错误值未重映射的问题。已完成泛型输入类型之间，通过受检结构视图解构元组／同序匿名记录，每个字段仅转移一次，递归重映射错误载荷，再构造目标产品。生产表达式只执行一次，正式 Tuple 保留 normal-form 头。仅检查明确结构，不强制求解无关关联类型；修复完整资源回归发现的两处过早关联类型查询。

新增 8 项测试：6 项源码执行／拒绝测试，1 项拥有字段清理测试，以及 1 项整个产品自定义 Drop 的预期行为回归。最后这一项仍未实现，显式 ignored，不能计入通过：当前 unpack/rebuild 产生的源产品清理会触发 Drop 精确签名／布局检查，无法正确转移整个产品的析构责任。普通拥有字段的清理和一次求值已经通过；不以它们证明整个产品 Drop 已完成。

下一步应实现受检的产品表示转移操作，整体搬运根析构责任与字段状态，而非用普通模式解构隐式丢弃／重建根责任。不能通过放宽 Drop 的 ABI 检查绕过问题。对应回归为 `inferred_error_product_conversion_does_not_add_outer_destructor_calls`；失败证据 `/tmp/luna-tools-9ot_h9qz/result.json`。指针目标与任意 nominal 容器内部转换也尚未接通。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test function_type_parameters --test inline_control --test effect_spreads`：退出 0，73 + 817 + 55 + 241 + 21 = 1,207 项通过，1 项上述待实现测试 ignored，`/tmp/luna-tools-quri8mse/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-w_lp1a0k/result.json`；`git diff --check` 通过。

改动留在 dev，未提交。产品表示转换尚未完整完成，目标保持进行中。

### 产品错误布局的整值所有权转移

以受检的 `ErrorConvert Product` 替代上一阶段的 unpack/rebuild。NIR 生成带预算的递归表示计划，IIR 验证同序字段、完整错误行集合及可信 Tuple 头；解释器只求值一次，整体搬运值并转换标签。根析构责任直接交付目标所有者，转换不执行 Drop，不放宽 Drop ABI。计划提供稳定的 S-expression Display。

内存、移动与纯度分析共用该计划，同步转换符号路径、分支域和物理指针 guard，并保留字段指针身份和函数值来源。恢复之前 ignored 的整个产品自定义 Drop 回归；新增指针别名／guard、纯函数字段来源、路径双向映射、预算耗尽、行扩张拒绝与未注册 Tuple 头拒绝测试。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，17 项通过，`/tmp/luna-tools-1z9za1i_/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test runtime_purity --test inline_control --test cmtt`：退出 0，819 + 73 + 62 + 241 + 522 = 1,717 项通过，无 ignored，`/tmp/luna-tools-jsuloi_7/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-vwgfkj2_/result.json`；`git diff --check` 通过。

相关检查合计 1,734 项通过。上一节整个产品 Drop 的缺口已补齐；指针目标、任意 nominal 容器内部布局转换与错误 comptime 值的 normal-form 冻结仍待实现。改动保留 dev 工作区，未提交。

### 错误 comptime 值的冻结、类型实参身份与 staged 重建

增加 `Term::ErrorValue { ty, label, value }`，保留完整错误类型与分支标签，数字 tag 不进入冻结分支身份。接通值导出、normal-form 参数域及载荷校验、解释器重建、staged 求值结果物化与常量输入提取。错误值可含已有支持的 nominal 载荷；成功／错误即使载荷相同仍是不同实参，normal-form 头不展开。

同步处理替换、作用域、预算、类型引用／依赖遍历、匹配、trait index admission／coherence 与类型应用的向前 congruence。泛型替换暴露嵌套错误成功域时同步展平冻结分支，避免类型已展平而载荷仍多一层。所有值身份关系仅作为类型应用的不可变输入证据，不增加一般程序值的 PTC 命题。NIR dump 使用显式 type／branch／payload S-expression。

新增 9 项回归覆盖冻结往返、缺失标签／错误载荷拒绝、域检查、隐藏未绑定槽、预算、标签替换、嵌套成功域展平、行换序与标签等式、nominal 载荷、opaque 头，以及普通 staged 参数执行。源码验收过程中发现并补齐了 staged dispatcher 原先缺少 ErrorResult 物化的入口。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results --test normal_inputs`：退出 0，19 + 12 项通过，`/tmp/luna-tools-ok1o1u8z/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test error_rows --test ptc --test implementation`：退出 0，9 + 56 + 65 项通过，`/tmp/luna-tools-nol3sa7a/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --test abstract_values`：退出 0，17 项通过，`/tmp/luna-tools-dh__61_2/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test runtime_purity --test staged_aggregate_contracts --test cmtt`：退出 0，76 + 62 + 31 + 522 项通过，`/tmp/luna-tools-nu3vtz3z/result.json`。补充成功／错误同载荷拒绝例后重跑 error_surface，77 项通过，`/tmp/luna-tools-cmxha4d4/result.json`。
- 最终全 workspace 格式化退出 0，`/tmp/luna-tools-xifoftxg/result.json`；`git diff --check` 通过。

最终相关测试组去重计 870 项通过。指针／活跃环境不是冻结许可范围；任意 nominal 容器内部布局转换仍待实现，staged 产品转换需先求值物化。未声明整个错误系统或语言完成。改动保留 dev 工作区，未提交。

### staged 产品转换与错误常量匹配

受检表示计划现在保留源错误类型，运行时转换与冻结值转换均核对源布局。新增有预算的冻结产品转换，递归保留元组／匿名记录字段，使用完整标签定位活动分支，再写入目标完整类型；拒绝源类型不匹配、字段不匹配与产品内部错误行扩张。冻结常量不引入独立求值或清理操作。

IIR staged 常量入口接通 Product／Embed，统一使用表示计划。新增常量折叠模块，把已完成的错误转换整理为可解构的常量构造，供 tuple／record 解构和错误模式匹配继续使用。错误模式支持 guard 回退与候选绑定隔离，未选中分支不求值；未知局部、需要用户调用的生产者仍由原来的求值边界处理，不借折叠隐式执行。正式 Tuple 仅由可信角色元数据授权。

新增 7 项测试覆盖冻结记录嵌套、错误源布局与预算恢复、直接 staged 产品转换及拒绝例、未完成输入拒绝、guard 顺序／绑定、成功及错误 Embed 匹配、深度上限，以及源码泛型 staged 嵌套产品调用。源码 fixture 初版错误地把泛型 T 载荷当成 bool，已按定义端语义修正为匹配后返回固定 bool，未放宽泛型检查。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，20 项通过，`/tmp/luna-tools-zaf59_95/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p iir --lib`：退出 0，46 项通过，`/tmp/luna-tools-ty1y2nhz/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test staged_aggregate_contracts`：退出 0，78 + 819 + 522 + 31 项通过，`/tmp/luna-tools-7c4veuj6/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-gpp9rhqo/result.json`；`git diff --check` 通过。

相关检查合计 1,516 项通过，无 ignored。上一节 staged 产品必须先经额外物化的限制已解除；未完成输入仍不属于常量折叠范围。指针目标与任意 nominal 容器内部的布局转换仍待实现。改动保留 dev 工作区，未提交。

### trait 记录索引中的错误值与 nominal 载荷

补齐记录索引校验对嵌套 ErrorValue／结构元组的递归域检查。错误分支检查完整结果类型及标签成员，成功分支检查成功域，空错误行与不匹配载荷拒绝；记录字段顺序不放宽。新增可返回具体失败的叶子校验入口，nominal 载荷必须取得声明布局并检查字段，符号值仍需独立域证据。

源码验收发现 impl 候选参数校验使用另一条旧记录入口，已同步接通声明布局与符号域证据。直接错误索引、记录中嵌套错误索引及 nominal 错误载荷均可在选中方法中重建。具体成功／错误索引即使载荷相同仍能选择不同实现；重复索引实现继续拒绝。

新增 6 项测试：两项 NIR 域／布局拒绝与预算恢复，四项源码覆盖直接错误索引、记录嵌套、nominal 载荷、具体分支实现选择及重复实现拒绝。未增加程序值的 PTC 命题，也未通过放宽域或布局校验实现接受。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p nir --test trait_parameters --test error_results --test normal_inputs`：退出 0，19 + 20 + 12 项通过，`/tmp/luna-tools-96lt6ks_/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test implementation --test error_rows --test ptc`：退出 0，65 + 9 + 56 项通过，`/tmp/luna-tools-m3v52x21/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test trait_purity --test staged_aggregate_contracts`：退出 0，82 + 20 + 31 项通过，`/tmp/luna-tools-_f5xyss5/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-itkfooa2/result.json`；`git diff --check` 通过。

相关检查合计 314 项通过，无 ignored。指针目标与任意 nominal 容器内部表示转换仍待实现；本轮不宣称所有类型容器与泛型组合均已覆盖。改动保留 dev 工作区，未提交。

### 关联错误类型的递归视图与冻结值重建

补齐解释器 selected_types 对 ErrorResult 标签／成功域的递归解析；原先该分支落入整体 clone，遗漏了已选关联类型证据，也没有正确计量这些子项。引入共享的 `Term::error_value` 重建规则，替换、关联类型正规化及解释器已选类型视图共用。成功值展平前检查内层完整类型对应原成功域；不匹配包装拒绝，错误分支自身的错误载荷不展平。normal-form 头继续保留。

源码泛型 trait 调用揭示另一条遗漏：候选方法的关联项等式收集未遍历错误结果，导致合法实现签名被拒绝。已补齐标签与成功域遍历；关联错误标签可穿过泛型方法调用，冲突签名继续拒绝。

新增 5 项测试，覆盖展平域证据及错误载荷保持、已选错误行解析／缺失证据／预算、关联成功域展平及不匹配拒绝、normal-form 头保持、源码关联错误标签与冲突实现。未用类型函数体推导头等式。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results --test normal_inputs --test trait_parameters`：退出 0，21 + 12 + 19 项通过，`/tmp/luna-tools-f1jiwhdo/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --lib selected_types`：退出 0，6 项通过，`/tmp/luna-tools-swxvf733/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test associated --test error_rows --test implementation`：退出 0，18 + 9 + 65 项通过，`/tmp/luna-tools-2gqzs1gi/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test trait_purity --test cmtt --test drop`：退出 0，83 + 20 + 522 + 819 项通过，`/tmp/luna-tools-vazft4bb/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-yxihn1cy/result.json`；`git diff --check` 通过。

相关检查合计 1,594 项通过，无 ignored。指针目标与任意 nominal 容器内部表示转换等尚未完成项仍保留，不宣称完整语言已实现。改动留在 dev 工作区，未提交。

### 分阶段替换中的待完成错误分支

修复冻结成功值过早展平：先得到错误成功域而尚无内层值时，保留原成功域和待完成载荷，多层包装不丢失域关系；后续值替换完成后再展平。空替换保持可继续完成，分阶段与同时替换得到相同最终结果。成功域和载荷仍是符号时，也不能先决定分支。Display 将该状态标为 `(branch pending-inner)`。

normal-form、nominal 和记录索引准入共用有预算的冻结载荷域检查；比较规范化的外层资格，但验证载荷时使用未展平的内层域。只有独立声明域证据才能接受符号载荷，闭合输入不能把它当作完成常量。已知模式选择返回未知；coherence 不据此证明不相交；需要区分未知分支的 impl 匹配保持 Pending，已有相同输入和普通绑定仍保留。解释器直接重建返回 AbstractValueUnresolved，避免伪造成功分支。

新增 6 项测试覆盖分阶段／同时替换一致、多层空替换与恢复、精确内层域证据及闭合拒绝、已知模式不选分支、重叠判断不误证不相交、执行边界拒绝重建。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results --test normal_inputs --test trait_parameters`：退出 0，23 + 13 + 19 项通过，`/tmp/luna-tools-tiz566_s/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --test abstract_values`：退出 0，18 项通过，`/tmp/luna-tools-5xc33t6v/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib`：退出 0，181 项通过，`/tmp/luna-tools-b5b76bee/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test implementation --test associated --test error_rows`：退出 0，65 + 18 + 9 项通过，`/tmp/luna-tools-mprbq5_c/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test cmtt --test trait_purity`：退出 0，83 + 522 + 20 项通过，`/tmp/luna-tools-qvqbtf3a/result.json`。
- 最终全 workspace 格式化退出 0，`/tmp/luna-tools-s61n0oqm/result.json`；`git diff --check` 通过。

相关检查合计 971 项通过，无 ignored。待完成分支仍需取得值证据，不宣称任意符号程序求值可完成；指针／任意 nominal 容器布局转换等剩余项不变。改动留在 dev 工作区，未提交。

### 泛型错误域实例化完成表达式转换

源码验证发现：当成功注入操作的泛型输入类型实例化为 ErrorResult，原 NIR 仍保留 Success，解释器按完整类型校验时拒绝合法调用。Expr 类型重写现在记录操作数重写前的域；仅在原本不是错误结果、重写后新暴露错误结果时，将 Success 完成为 Embed。原本已是不合法的 Success／ErrorResult 组合不自动修复，后续 IR 校验继续拒绝。

新增 5 项回归：NIR 新暴露域与非法旧 IR 的区别、普通 comptime Wrap 构造的 Key 与直接输入一致且成功／错误不混同、runtime 泛型成功／错误两条路径、关联成功域实例化、拥有型两条路径的一次求值及一次析构。源位置与原操作数表达式保持，normal-form 头不展开。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，24 项通过，`/tmp/luna-tools-guzm2s95/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control --test runtime_purity`：退出 0，86 + 820 + 522 + 241 + 62 项通过，`/tmp/luna-tools-9rkf5vh7/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-gpcvtykg/result.json`；`git diff --check` 通过。

相关检查合计 1,755 项通过，无 ignored。其余未完成项继续保留；本轮接通已确认错误资格语义的泛型实例化路径，不宣称整个语言完成。改动留在 dev 工作区，未提交。

### 泛型错误传播与 handler 的成功投影补全

源码复现 `forward(value:!Other T)->!Other T where T {let result=value!;result}` 在 T 实例化为 `!Failure bool` 后，原成功模式仍将裸 bool 绑定为完整 T，解释器报 Type；此前仅补全 Success→Embed 未覆盖消去路径。NIR 类型重写现在在新暴露的错误域上补全透明成功投影：保留定义时已知错误标签的处理，将新暴露的成功／错误重建为原 T，再执行既有成功路径。普通 handler、guard 拒绝后的余行、重复标签和分阶段类型替换使用同一规则，不改变未知泛型主体的程序意义，也不展开 normal-form 头。

拥有型回归进一步发现新增分支复用原 source 会使 cleanup storage mapping 不唯一。物化阶段为新增转换链分配独立来源表条目，保留 original/site，并检查预算；重复物化来源整理不继续生成条目。没有放宽 Drop ABI 或跳过清理。

新增 9 项回归，覆盖普通/inline 调用的执行次数与退出边界、required comptime invocation、普通及 guarded handler、重叠错误身份、拥有型载荷与 fallback 的恰好一次释放，以及 NIR 分阶段替换、重复补全稳定性、转换合法性和来源恢复。当前补全针对编译器生成的透明成功投影；不宣称任意用户模式体、所有 catch 组合与任意容器布局转换均已完成。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，92 + 822 + 522 + 241 项通过，`/tmp/luna-tools-kkvcqsr0/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，25 项通过，`/tmp/luna-tools-nfjkh8a9/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-b1s0hpkz/result.json`；`git diff --check` 通过。

相关检查共 1,702 项通过，无 ignored；dev 工作区保留改动，未提交。

### 泛型 catch 验证与显式 match 的 arm 顺序

前置 catch／普通 arm／后置 catch 在新暴露错误域下的普通和 inline 泛型调用已用实际执行事件验证：仅定义时错误进入 catch 阶段，三个阶段顺序保持，输入求值一次。拥有型 catch 返回原值和显式 match 重构原错误均验证一次析构。

源码进一步复现显式 match 的成功 arm 不在首位时仍报运行时 Type（`/tmp/luna-tools-g49cmzoz/result.json`）。原补全入口只检查第一个 arm，错误地依赖编译器生成的排列。现记录重写前所有透明成功投影的位置，逐一补全，并将新增分支插在对应成功 arm 后；既不越过前置通配分支，也不落入后置通配分支。保留原有 arm 顺序、类型身份和来源补全机制。新增 4 项源码回归覆盖非首位成功分支、前后通配优先级、catch 次数／顺序及拥有型清理。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop`：退出 0，94 + 823 项通过，`/tmp/luna-tools-dj5ituq9/result.json`。
- 增加前置通配回归后，`python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface`：退出 0，95 项通过，`/tmp/luna-tools-a5p4sgo_/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，25 项通过，`/tmp/luna-tools-0ocw4jni/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-42ntqxnk/result.json`；`git diff --check` 通过。

相关测试去重后 943 项通过，无 ignored。非透明成功模式体与成功模式 guard 的泛型投影仍不能由此次透明投影补全覆盖，不将这些路径宣称为完成。改动保留 dev 工作区，未提交。

### 泛型成功绑定与任意分支体分离

源码复现成功分支执行 `after(result)` 或绑定局部值时，实例化后仍将裸载荷交给要求完整 T 的调用，物化报 Type（`/tmp/luna-tools-1yljggn1/result.json`）。显式 match 的无 guard、简单绑定／通配成功模式现在在 elaboration 时引入新鲜物理载荷绑定及一次用户绑定初始化；类型替换只补全该初始化，分支体可正常调用、绑定、捕获和提前返回，不在每次变量读取时重复构造。`_!` 使用同样的抽象成功域恢复与作用域清理。

NIR 投影入口可定位这种初始化，新增标签继续保留 arm 优先级。物化来源整理扩展到复制后的完整分支体和闭包表达式；每个分支独立映射来源，但保留分支内部原有的词法共享位置，仅把新增转换与操作数拆成独立位置，避免重复清理边界。

新增源码调用／局部绑定／闭包捕获／return 回归和拥有型 `_!` 回归，扩展既有拥有型分支用例，验证成功、内层错误、外层错误均保持执行次数与一次析构。成功模式 guard、复杂解构模式及 inline match 的独立选择路径不在本次完成声明内。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，96 + 824 + 522 + 241 项通过，`/tmp/luna-tools-g43xfpil/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，25 项通过，`/tmp/luna-tools-syr3y44v/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-sfkwsm5f/result.json`；`git diff --check` 通过。

合计 1,708 项相关测试通过，无 ignored。dev 工作区保留改动，未提交。

### 泛型成功 guard 的载荷恢复与有序回退

源码复现简单成功模式 guard 在 T 实例化为错误限定类型后，仍通过旧绑定观察裸载荷，导致 Type 错误（`/tmp/luna-tools-uflglzmm/result.json`）。现在在原始覆盖检查之后，将该成功分支分为一次载荷恢复和内层 guarded match；guard 仍是原有 Pattern::Guard，接受后执行原分支体，拒绝后把同一值嵌回原错误行并按原顺序尝试剩余 arm。支持简单绑定与 `_!`，不重复输入求值，也不把已失败 guard 重新加入后续候选。后续候选复制沿用预算限制并分配独立的可追溯执行位置。

新增 7 项回归覆盖连续 guard、普通/inline 调用的次数和回退顺序、required comptime invocation、关联输入类型经 trait 方法接收恢复后的引用、拥有型借用后拒绝及 `_!` 两条路径的一次析构、禁止修改 guard 调用方状态，以及借用不自动授予 Copy 读取权限。trait 读取整个错误结果的试例需要独立 Copy/内存转移依据；本次没有放松这条边界，正例只使用合法借用契约。一般复杂解构成功模式和 inline match 的独立选择路径仍需继续完善。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，100 + 827 + 522 + 241 项通过，`/tmp/luna-tools-kfv6b5y9/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，25 项通过，`/tmp/luna-tools-zku6o8c_/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-udotr5jk/result.json`；`git diff --check` 通过。

合计 1,715 项相关测试通过，无 ignored。改动保留在 dev 工作区，未提交。

### 成功载荷内部 guard 与组合模式

源码复现 `(result if allowed(result.ref,flag))!` 在新暴露错误域下仍按裸载荷调用 guard，报 Type（`/tmp/luna-tools-q68q39d9/result.json`）。原入口仅识别外层 Guard(ErrorResult)，不能覆盖括号内 guard、计算模式或 or-pattern。成功模式恢复现保留原有内层模式，并以有预算和深度限制的遍历处理外层 Guard、Then、Remap 及全部为成功候选的 Or。模式在恢复后的 T 上执行，拒绝仍沿用同一个值和原有有序回退。

新增一组源码回归并扩展拥有型、required comptime 回归，覆盖 guard 位于限定内外、双层 guard、`and … is …` 位于限定内外、成功候选的 or-pattern 与绑定重映射。逐次验证输入／候选调用次数、接受／拒绝分支及一次析构。混合成功／错误候选和外层全值别名仍保留原 subject，不能错误地改成 payload；这些组合与 inline match 独立选择路径仍需后续完善。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，101 + 827 + 522 + 241 项通过，`/tmp/luna-tools-vmrpw9lk/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-glhtbjkg/result.json`；`git diff --check` 通过。

合计 1,691 项相关测试通过，无 ignored。本轮未改变 NIR 类型重写或 IR 结构，未重复无关检查。改动保留 dev 工作区，未提交。

### 不依赖成功载荷的混合候选与全值别名

源码复现 `((_! or _! or error Outer.Bad) as whole) if allowed(whole.ref,flag)` 在 T 新暴露 Inner 错误后漏掉该候选，连最终 guard 都不执行（`/tmp/luna-tools-xckctvvb/result.json`）。NIR 类型重写现分别保留原成功域与已知错误行；当成功域新暴露错误限定时，对不绑定／观察载荷的模式原位扩展标签测试。Guard、Or、Remap 和 Then 的控制结构及已有表达式保留，整个结果的 Binding／Observe 别名继续指向原完整输入，不复制 guard，也不把 or 拆成重复执行最终 guard 的独立 arm。

忽略载荷的内部 guard 和计算模式可沿原结构扩展；真正绑定／观察载荷的模式仍走值恢复路径。遍历明确停止于不同 subject 域：错误载荷、产品字段和 computed next-pattern 不使用外层行的规则。新增 4 项回归验证混合候选、全值引用 guard 的一次执行、接受与拒绝后的整值交付及一次析构、required comptime invocation，以及分阶段替换稳定性与错误载荷域不受误改。仍不宣称已完成所有同时导出载荷／全值的组合和嵌套产品模式。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，103 + 828 + 522 + 241 项通过，`/tmp/luna-tools-c0acy953/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p nir --test error_results`：退出 0，26 项通过，`/tmp/luna-tools-t2a2eh9r/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-vwmf4lb6/result.json`；`git diff --check` 通过。

合计 1,720 项相关测试通过，无 ignored。改动保留 dev 工作区，未提交。

### 结构字段、let 与计算模式的独立成功域

泛型类型替换现在携带模式原始 subject 类型，沿元组字段、匿名记录的具名字段、let 模式以及计算模式的结果分别传递；错误载荷使用完整标签类型。新增成功标签只补入该 subject 的成功通配模式，不跨字段传播，也不展开名义或 normal-form 头。修复前元组内 Inner 错误被错误拒绝（`/tmp/luna-tools-ei1lyxf9/result.json`）。

新增源码元组／let／逆序记录字段回归和计算模式域隔离 NIR 回归；扩展拥有型通配模式测试到这三种结构，覆盖成功、Inner、Outer 路径，逐一检查结果及一次析构。任意名义布局、正式 Tuple 的布局证据接入及一般嵌套载荷绑定仍需后续实现，未通过猜测类型实参放宽边界。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，105 + 828 + 522 + 241 项通过，`/tmp/luna-tools-dk8fw4vh/result.json`。
- 随后扩展清理场景，定向执行 `test -p luna --test drop generic_success_wildcard_retires_the_completed_payload_once`：退出 0，`/tmp/luna-tools-o7l3rry9/result.json`。
- `test -p nir --test error_results`：退出 0，27 项通过，`/tmp/luna-tools-s7if8x88/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-1dg04bsn/result.json`；`git diff --check` 通过。

相关测试去重 1,723 项通过，无 ignored。改动保留 dev 工作区，未提交。

### 嵌套结构成功绑定的延迟载荷恢复

源码 `(result!, flag) => after(result) and flag` 在 T 为错误限定类型时把裸载荷传给要求完整 T 的泛型调用，参考执行报 Type（`/tmp/luna-tools-togfkc1_/result.json`）。现在 elaboration 对无观察表达式的结构模式保留字段完整输入，使用该字段的成功通配测试；整个候选接受后才通过独立的 Match 初始化原始用户绑定。初始化复用已有泛型成功投影补全，因此分支体得到完整 T，多个字段分别使用自己的错误行，拒绝的候选不提前消费任何字段。

遍历带预算与深度上限，仅使用显式元组和匿名记录布局；观察表达式检测复用 NIR 有预算的完整模式遍历，避免把 guard/computed-pattern 需要提前读取的绑定错误延迟到分支体。未实现的嵌套观察期恢复、名义／正式 Tuple 布局接入保持待办，不以猜测头参数替代布局证据。

新增源码元组／多层元组／逆序记录字段绑定回归；拥有型回归扩展到记录和元组成功绑定，并新增双字段全部九种成功／Inner／Outer 组合，验证输入一次求值、仅整个候选接受后消费、拒绝后完整交付 fallback、两个 Item 各析构一次。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，106 + 829 + 522 + 241 = 1,698 项通过，无 ignored；`/tmp/luna-tools-2hftowfj/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-pshtfzv7/result.json`；`git diff --check` 通过。

改动保留 dev 工作区，未提交。

### let 成功绑定的同作用域恢复

`let result! = value else {return false;}; after(result)` 在 T 为错误限定类型时同样发生裸载荷交付 Type 错误（`/tmp/luna-tools-ecvften0/result.json`）。块 lowering 在已有 staged-pattern 物化和别名消除后，为不含观察表达式的 let 成功绑定追加恢复初始化；原声明与初始化保持同一作用域，失败分支继续发散且不能访问新绑定。复用结构模式的字段独立恢复，覆盖根成功绑定、元组和匿名记录；不引入会藏住用户绑定的内层 Block。

第一次集成检查发现检查器过早要求 quoted pattern 的 CodeEvidence 完成，破坏既有 CMTT let-else（`/tmp/luna-tools-1u76f7qx/result.json`）。延迟绑定判定现采用预算受限的模式结构检查，遇到 Guard、Then 或未完成 CodeEvidence 保留原路径；并同步用于结构 match，避免把待展开模式当作已知无观察模式。

新增源码 let 泛型绑定与 required comptime invocation 回归，扩展拥有型测试验证根／元组／记录的成功和 else 路径各析构一次。现有 CMTT quoted let-else 回归也已恢复。嵌套观察期恢复及正式 Tuple 布局证据仍待实现。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，108 + 829 + 522 + 241 = 1,700 项通过，无 ignored；`/tmp/luna-tools-pfirypnn/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-34xorxde/result.json`；`git diff --check` 通过。

改动保留 dev 工作区，未提交。

### 根成功 let guard 的观察期恢复

源码 `let (result! if allowed(result.ref,flag)) = value else {return false;}; after(result)` 在抽象 T 实例化为错误结果后，guard 仍收到裸载荷引用，执行报 Type（`/tmp/luna-tools-kvh65d_3/result.json`）。根成功绑定现在降低为带原始 guard 与发散 fallback 的 Match 初始化器，复用现有成功域恢复和有序拒绝逻辑。内部通过 Remap 导出新鲜绑定，Match 交付完整载荷后才建立声明的原始绑定；guard 内继续使用其原始词法绑定，不重写用户表达式或放松观察限制。

支持限定内外 guard 的单个成功载荷导出；未把多个计算结果导出、复杂结构 guard 或未知 pattern-code 强行套入这一路径。新增普通执行、拥有型接受／拒绝及一次析构、禁止修改调用方状态、required comptime invocation 四项回归，逐一检查输入、guard 和消费函数执行次数。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，110 + 831 + 522 + 241 = 1,704 项通过，无 ignored；`/tmp/luna-tools-_ingogxa/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-jz7ju0_q/result.json`；`git diff --check` 通过。

改动保留 dev 工作区，未提交。

### 成功 let 的计算模式多绑定交付

源码 `(result and measure(result.ref,flag) is extra)!` 原先在抽象 T 的错误限定完成后给计算函数传入裸载荷引用，报 Type（`/tmp/luna-tools-6bixg42v/result.json`）。成功 let 的观察期恢复现收集已确认模式导出，处理 Guard、Then、Remap 和导出一致的 Or；收集受预算和深度限制。多个导出通过内部结构交付包离开初始化 Match，再在声明原作用域解构，保留载荷类型和计算结果类型。内部包不替代语言 Tuple 的 normal-form 头，不复制原始拥有型载荷。

新增源码计算模式及最终 guard 回归，扩展 required comptime invocation 与拥有型原载荷回归，验证额外 bool 导出、一次计算、接受后交付和拒绝后原载荷一次清理。

独立试验发现计算模式新建 Item 时，资源层明确拒绝“computed pattern value needs independent resource and failure-path transfer”（`/tmp/luna-tools-6wpi4ft8/result.json`，位置 compiler/elaboration/src/verification/moves.rs 的 Then 分支）。该能力仍未实现，未放松检查；对应 fixture 改为明确验证这项拒绝义务，不能把它算作拥有型计算结果的支持。后续必须设计计算结果独立所有者及接受／失败两条清理路径，再将此用例推进为正例。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，111 + 832 + 522 + 241 = 1,706 项通过，无 ignored；`/tmp/luna-tools-0gmfiaov/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-u6r9k040/result.json`；`git diff --check` 通过。

改动保留 dev 工作区，未提交。

### 不导出绑定的拥有型计算结果

继续沿资源层分析确认，直接将拥有型计算结果交付 Then 需要独立候选所有者；但后项不向外导出绑定时，结果的完整生命周期可以封闭在观察区域内。源码复现 `let (_ and make() is (_ if flag)) = true else ...` 原先因计算结果资源转移缺失被拒绝（`/tmp/luna-tools-igut6pfe/result.json`）。现将这类非标量结果的后项匹配放入 bool 观察块，用既有 Match 清理承担实际构造载荷的责任，再以 bool 后项选择接受／拒绝。前项仍先匹配，未接受不求值计算；拥有值在进入外部 continuation 前结束。

初版裸 Match 被观察路径限制拒绝（`/tmp/luna-tools-ck35zxnr/result.json`），已改用完整观察区域检查，没有放宽外部状态或调用约束。降低保留原后项不可反驳性：已证明总能接受时 bool 后项使用 Wild，避免把原本合法的无 else 声明变成可反驳模式。模式导出收集扩展到完整结构与错误载荷，未知 CodeEvidence 保持原路径。

新增五项源码回归：对象成功／guard 拒绝时一次清理且先于 continuation、枚举按活动载荷清理、禁止修改调用方状态、前项拒绝时跳过构造、无 else 的拥有型通配计算。向外导出新拥有值仍明确拒绝；本次未实现该跨候选所有者协议。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，111 + 837 + 522 + 241 = 1,711 项通过，无 ignored；`/tmp/luna-tools-ck3_v9n_/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-9335weil/result.json`；`git diff --check` 通过。

改动保留 dev 工作区，未提交。

### 观察区域中的错误结果与程序拒绝边界

拥有型计算结果的封闭观察路径扩展到 ErrorResult。原先 `make()->!Failure Item` 被 guard call result 的布局白名单拒绝（`/tmp/luna-tools-eewg_k_x/result.json`）。现在调用返回、区域私有实参、内联 ErrorConvert 构造均使用现有清理证据及无数据指针证明；布局证明递归检查成功域和每个完整错误标签，保持类型头不展开，缺预算／未知布局／任一分支存储数据指针仍拒绝。

进一步复现返回 error 标签被拒绝摘要当作未知节点而认为可能程序拒绝（`/tmp/luna-tools-zjktc3p9/result.json`）。拒绝分析现在识别 ErrorConvert 为数据表示转换，继续检查操作数和目标域的析构拒绝义务；错误结果 ABI 不与程序规格拒绝混淆。新增单元用例覆盖成功／错误包装、拒绝操作数、拒绝清理，及带嵌套错误标签的指针布局、未知 normal-form、预算耗尽后恢复。

源码正例覆盖成功／错误活动载荷、普通调用／inline／区域内按值转发，均只析构实际 Item 一次；负例验证含外部指针的拥有对象不能通过其析构隐藏调用方状态修改。向计算模式外导出新拥有值的独立协议仍未完成。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test drop --test cmtt --test inline_control`：退出 0，111 + 839 + 522 + 241 = 1,713 项通过，`/tmp/luna-tools-b3ry3f_y/result.json`。
- `test -p elaboration --lib verification::rejection::tests`：13 项通过，`/tmp/luna-tools-zxl_uhnf/result.json`。
- `test -p elaboration --lib pointer_footprints`：2 项通过，`/tmp/luna-tools-2ogadb1e/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-aw1hcx02/result.json`；`git diff --check` 通过。

合计 1,728 项相关检查通过，无 ignored。改动保留 dev 工作区，未提交。

### 拥有型 newtype 的封闭观察与析构拒绝

源码计算模式构造 `Item.as(Inner).as(Outer)` 时，拒绝摘要将 NewtypeConvert 作为未知节点处理，普通供应调用被拒绝（`/tmp/luna-tools-amsig5zk/result.json`）。表示转换现与错误表示转换一样，遍历操作数并检查目标清理义务，不把转换本身当作程序规格拒绝。包装和表示的析构依然分别受检，没有展开或抹去类型身份。

新增源码正例验证普通／inline 构造、匹配接受／拒绝下的实际析构序列严格为 Outer、Inner、Item，各一次；负例分别令包装析构或原载荷析构拒绝，观察调用继续被拒绝。执行轨迹测试工具补上 newtype 的名义析构名称，避免已有运行路径成功却在统计时误判“非名义 receiver”。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test runtime_purity`：退出 0，841 + 62 项通过，`/tmp/luna-tools-oaobztzn/result.json`。
- `test -p elaboration --lib verification::rejection::tests`：退出 0，13 项通过，`/tmp/luna-tools-03peqjlc/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-gm1ajep4/result.json`；`git diff --check` 通过。

合计 916 项相关检查通过，无 ignored；未重复不受此次修改影响的 CMTT／inline_control 全组。跨候选导出新拥有值仍待实现。改动保留 dev 工作区，未提交。

### 计算模式资源路径的组合验收

新增多候选所有权回归，覆盖两个新建拥有值、候选内 guard 和共享最终 guard 的八种真假组合。逐次核验 first 临时先于 second 重试清理、最终 guard 不因其自身拒绝而重试 or 的另一分支、原始仿射 Subject 保留至选定分支结束再清理；输入／构造／析构次数均检查。

新增高阶泛型工厂回归，验证 `fn()->!Outer T` 在 T 为 `!Inner Item` 时仍将 Inner 视为成功域，Outer 拒绝，三种实际载荷各清理一次。工厂代码值的观察保留 Copy 协议要求：正例明确声明 Factory 的 Duplicate 证据，负例移除该证据并验证 Copy code observation 诊断。本轮补充组合证据，没有更改编译器语义或放宽资源约束。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop computed_`：退出 0，23 项通过，无 ignored；`/tmp/luna-tools-so075g27/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-984cvycw/result.json`；`git diff --check` 通过。

本轮仅修改回归测试和实现记录，未重复全量测试。新拥有值的跨候选导出协议仍是独立待办；上述证据仅覆盖不导出新拥有值的观察。改动保留 dev 工作区，未提交。

### 拥有型计算主体的标量字段交付

将计算模式的封闭观察 lowering 提取到 `program/computed_patterns.rs`，扩展为可导出 bool、char、固定宽度整数、f32/f64 与单位值字段。后项匹配和 guard 在原拥有值上完成；观察结束时清理原主体，再把已检查的标量绑定发布到后续模式作用域。新拥有值、指针和尚未确定的字段类型仍走原有独立资源协议检查，未被这条路径放行。

多个字段使用新鲜的编译器私有名义 struct 交付，具有单独的声明和字段布局，并纳入现有内部包元数据。没有为用户 Tuple 增加 Copy 实现或修改 normal-form 身份，也没有合成用户 Copy trait 实现；观察分析只对已标记的私有 struct 逐字段验证原始标量的 Copy 证据。函数体 lowering 之后补充输出包声明，避免生成引用缺少 IIR 声明。拒绝包的标量槽使用类型正确的惰性常量，false 判别在任何绑定发布前失败；有符号／无符号／浮点常量采用对应 NIR 值表示，不能用未转换的任意精度字面量冒充运行时 u8。

新增源码回归覆盖不可反驳结构字段解构（无需 else）、owner 先于后续调用清理、枚举与 guard 拒绝时不交付字段、带指针字段保持拒绝。全组检查确认原不导出拥有值的路径、计算模式、多候选与泛型错误绑定继续工作。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test inline_control --test runtime_purity`：退出 0，846 + 111 + 522 + 241 + 62 = 1,782 项通过，无 ignored；`/tmp/luna-tools-adpbnx2j/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-0j3w8woc/result.json`；`git diff --check` 通过。

新拥有值本身的跨候选导出仍待实现。改动保留 dev 工作区，未提交。

### 标量聚合交付与无载荷失败变体

计算模式交付扩展到由已确定标量组成的显式结构元组与匿名记录，递归形状检查带深度和共享预算。聚合字段仍需要该完整类型的 Copy 证据，不能因为其成员是标量而推导 Copy；新增正例分别声明 Pair／Fields 的 Duplicate 证据，并用移除证据的负例验证边界。

内部交付类型改为私有枚举：成功变体携带实际导出，失败变体无载荷，避免为拒绝路径构造聚合占位值。原后项已证明不可反驳时只生成成功变体和对应 arm，保持无需 else 的语义。私有包声明在函数体检查后发布到输出枚举表；观察资格逐变体检查字段形状与 Copy 证据。没有用户 Tuple 或匿名记录的隐式协议实现，也不展开不透明类型头。

新增组合验证：多候选字段重映射、拒绝后清理再重试、共享最终 guard 一次执行；必须执行的 comptime invocation 可访问私有包声明；元组和匿名记录字段接受／拒绝及原主体一次析构。所有相关正例与拒绝例均通过。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test inline_control --test runtime_purity`：退出 0，849 + 112 + 522 + 241 + 62 = 1,786 项通过，无 ignored；`/tmp/luna-tools-qfj9ddc4/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-a4q87aqm/result.json`；`git diff --check` 通过。

指针、新拥有值和未取得布局的 normal-form 聚合字段导出仍待独立实现。改动保留 dev 工作区，未提交。

### 正式 Tuple 的计算模式字段交付

计算模式 lowering 和私有交付包的资源检查均支持通过协议注册的 Tuple 布局；递归遍历带深度与工作预算，实际字段类型保留原始 normal-form 头。完整字段的 Copy 检查保持独立，不展开其他类型函数或合成 trait 实现。新增源码回归验证成功／guard 拒绝、一次析构、包字段头身份，以及移除 Tuple Copy 证据后拒绝执行。

验证目录 `/home/fin/luna`：计算模式定向 30 项通过（`/tmp/luna-tools-pwa3wqax/result.json`）；`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test type_spreads --test error_surface` 退出 0（`/tmp/luna-tools-1tiwkrcn/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-k00gxxc7/result.json`），`git diff --check` 通过。

探索嵌套案例时，为两个不同 Tuple 实例分别声明 Copy 会触发现有“implementation overlap is not provably disjoint”检查；未通过假定 normal-form 单射性绕过。本次端到端证据仅覆盖单层正式 Tuple，嵌套实例的协议组合仍需单独验证。指针和新拥有值跨候选导出仍不在本次范围。改动保留 dev 工作区，未提交。

### 嵌套 Tuple 与泛型清理候选的排除

前一轮有实现和验证进展。本轮确认两份 Tuple Copy 实现的互斥问题不能靠推定 normal-form 单射性解决；单个带析取类型约束的实现可以合法覆盖内外两层。新增嵌套源码用例通过泛型 `make<T>` 构造 `Item<T>`，验证接受／guard 拒绝、元组头保留和一次析构；分别移除外层或内层 Copy 条件后保持拒绝。

该用例暴露并修复了实际的候选推导缺口：查询 Tuple 的 Drop 时，`impl Retire for Item<T>` 会因为 T 无法推导而 pending。现在在参数推导前，对已注册 Tuple 与已登记名义构造子使用现有 PTC 分类不相容证明排除整个候选；仍保留原始头和参数，证明计入查询预算。不向任意不透明工厂推广，不增加 Tuple 单射性。底层双向回归检验证明重放、移除 Tuple 注册后拒绝重放且保留 pending、预算耗尽及新预算恢复。

验证目录 `/home/fin/luna`：
- 嵌套 Tuple 泛型源码回归退出 0：`/tmp/luna-tools-yainc68n/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test implementation`：66 项通过，退出 0；`/tmp/luna-tools-k1kvak_7/result.json`。
- 全 workspace 格式化退出 0：`/tmp/luna-tools-zeowmtki/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test type_spreads --test newtypes`：退出 0；`/tmp/luna-tools-jpzwz3fu/result.json`。

正式 Tuple 的嵌套字段交付与泛型拥有者清理已有端到端证据；不同 Tuple 实例的独立实现互斥证明未放宽。新拥有值跨候选导出仍是独立待办。改动保留 dev 工作区，未提交。

### 泛型计算字段的受证布局与参数化交付包

上轮有实现和验证进展。本轮先复现 `requires T == bool` 的字段仍被旧 lowering 当作未知拥有值拒绝（`/tmp/luna-tools-iysprs3_/result.json`），再接通已有 PTC 结构视图：只用等式证明确定导出形状，不改写导出字段的原始 T。结构视图查询计入 required 预算，递归深度受限。

私有枚举收集字段中的自由 Type 参数，并在使用点携带对应实参；声明字段继续保留原始参数和 normal-form 头。资源检查通过受预算约束的名义布局接口逐变体实例化字段，再检查标量形状和完整 Copy 资格。运行时依赖、外层未封闭绑定以及非 Type 自由参数未被这条路径放行。

新增源码验证包括：泛型 bool 字段及一次析构、泛型嵌套正式 Tuple 的接受／guard 拒绝与两层 Copy 负例、必须执行的 comptime invocation、`T == *bool` 不产生标量交付包且由内存来源检查拒绝。NIR 断言检查交付声明显式绑定 T，而非保留悬空字段参数。

验证目录 `/home/fin/luna`：
- 泛型标量修复后退出 0：`/tmp/luna-tools-xt4g0s3f/result.json`；泛型相关 3 项通过：`/tmp/luna-tools-vx5y16ie/result.json`；required invocation 回归退出 0：`/tmp/luna-tools-h2zlbbsj/result.json`。
- 相关五组检查 `/tmp/luna-tools-3zk7l8dv/result.json` 首轮退出 101：CMTT 522 项和 Drop 853 项通过；新指针负例已被拒绝，但诊断断言预期晚于实际发生的内存来源错误。修正断言后定向重跑退出 0（`/tmp/luna-tools-inuvbmzz/result.json`），未改动实现。
- 补跑首轮尚未执行的 `error_surface`、`inline_control`、`runtime_purity`：113 + 241 + 62 项通过，退出 0；`/tmp/luna-tools-u25ed0wm/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-7m2ha_dh/result.json`），`git diff --check` 通过。

新拥有值跨候选导出和运行时依赖字段的独立交付协议仍待实现。改动保留 dev 工作区，未提交。

### 多泛型字段交付与具名 Copy 聚合

上一轮有实现和验证进展。本轮先确认两个泛型字段通过不同候选、相反源码字段顺序重映射后仍按正确类型和值交付；最终共享 guard 只执行一次，候选临时 owner 在 guard 或重试前清理。各候选使用独立私有交付声明，明确绑定两个 Type 参数。

计算模式的受检导出布局扩展到具名 struct、enum、newtype：lowering 与资源检查分别通过已有名义布局接口递归实例化声明字段；enum 检查全部变体，newtype 检查表示，保留声明身份。共享预算和深度限制继续适用。只确认布局并不授予 Copy，完整导出字段及其成员仍由现有资源协议验证。没有指针、未知布局或非 Copy owner 的隐式跨候选交付。

新增源码回归覆盖具名结构、含载荷／空变体枚举、包装 newtype 的成功和 guard 拒绝，原 Holder 一次析构，根类型或叶子缺失 Copy 的拒绝，以及未选中的枚举变体含指针时也不得生成标量交付包。文档增加完整具名 Copy 字段示例。

验证目录 `/home/fin/luna`：
- 多泛型字段定向退出 0：`/tmp/luna-tools-82w6p9ol/result.json`。
- 具名聚合及负例定向退出 0：`/tmp/luna-tools-g0058uyt/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test newtypes --test error_surface --test cmtt --test inline_control --test runtime_purity`：856 + 207 + 113 + 522 + 241 + 62 = 2,001 项通过，退出 0，无 ignored；`/tmp/luna-tools-26__nri3/result.json`。
- 全 workspace 格式化退出 0：`/tmp/luna-tools-tfcwoe3b/result.json`；`git diff --check` 通过。

非 Copy 新拥有值的跨候选转移、运行时依赖和指针字段的独立来源交付仍待实现。改动保留 dev 工作区，未提交。

### 错误限定 Copy 字段与用户协议主体

上轮有实现和验证进展。本轮接通 ErrorResult 计算字段的导出形状：成功域与完整错误标签逐项递归检查，保留错误行的类型实参和实际值。与具名聚合相同，仍须取得整个错误结果及所有组件的显式／内建 Copy 证据，并通过清理资格检查；不因活动标签已知而忽略其他标签，也不触及代数效应 ABI。

首个源码回归暴露了入口不一致（`/tmp/luna-tools-6pnf56r1/result.json`）：底层候选选择／证据重放已接受错误限定主体，源码 impl 入口却拒绝它。修复源码主体资格检查，同时把错误结果加入 Copy 已支持表示的检查，与现有逐分支清理布局对齐。新增普通及受约束泛型 trait 实现的方法分派回归；typealias 指向错误结果不会增加身份或隐藏完整成功域／错误行。

计算字段回归覆盖成功 true／false、Failure<bool>／Failure<u8> 错误分支、guard 接受／拒绝、Holder 一次析构、整体 Copy 缺失、未活动标签 Copy 缺失、未活动指针错误标签的拒绝。接口分派回归覆盖普通和泛型实现各三种结果。

验证目录 `/home/fin/luna`：
- 错误字段定向退出 0：`/tmp/luna-tools-dh291yec/result.json`；错误结果用户实现分派定向退出 0：`/tmp/luna-tools-j66tb6ji/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test newtypes --test error_surface --test cmtt --test inline_control --test runtime_purity`：857 + 207 + 114 + 522 + 241 + 62 = 2,003 项通过，退出 0，无 ignored；`/tmp/luna-tools-5iv9pl6y/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-6cn_2qb2/result.json`），`git diff --check` 通过。

非 Copy 拥有值跨候选交付、运行时依赖与指针来源交付仍待实现。改动保留 dev 工作区，未提交。

### 错误前缀的定向缺失证明与整体 Drop

上轮有实现和验证进展。本轮以错误限定类型自定义 Drop 的源码回归继续推进，复现泛型清理候选 pending（`/tmp/luna-tools-atybhtm_/result.json`）。根因是错误标签缺失证明要求两边成功残余都封闭，导致 `!Failure T` 中明确存在的前缀也不能排除普通类型。

PTC 现在按方向检查标签缺失：只要求目标残余已知普通，并逐项证明源标签不同于目标全部完整标签；源的未知成功域可以增加标签，却不能删除已出现的标签。成功残余之间的结构不相容仍要求两边都封闭。新反例验证未知目标、未知标签等式以及源残余可能贡献目标额外标签时不能错误证明不相等；肯定结论均重放证明。

实现匹配在未能推导全部参数前，对明确错误前缀与普通类型尝试受预算约束的 PTC 排除证明，避免无关泛型 Drop 候选阻塞清理；没有用推导失败代替反证。新增双向实现选择／证明重放测试，并验证真正匹配的错误主体仍能推导参数和分派。

源码回归确认普通／泛型错误结果 Drop 在成功／错误分支均先执行整体析构，再只清理实际载荷；带载荷错误标签继续执行自己的析构后清理内部字段。整体有自定义 Drop 时，即使显式声明 Copy，也因非平凡清理责任被拒绝。

验证目录 `/home/fin/luna`：
- 自定义错误结果 Drop 定向退出 0：`/tmp/luna-tools-rput1y4v/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test error_rows --test implementation`：10 + 67 项通过，退出 0；`/tmp/luna-tools-09z8rbsq/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test newtypes`：859 + 114 + 207 项通过，退出 0；`/tmp/luna-tools-twc9_iia/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-l8f0n4zk/result.json`），`git diff --check` 通过。

非 Copy 计算结果跨候选交付仍待实现。改动保留 dev 工作区，未提交。

### 正式 Tuple 分类参与错误前缀排除

上轮有实现和验证进展。本轮复现泛型 `impl Retire for !Failure T` 阻塞正式 Tuple 成功载荷的自动清理（`/tmp/luna-tools-vnn2gmi_/result.json`）。此前普通类型的判定仅识别结构构造子，没有使用已注册 Tuple 的分类。

错误行缺失证明现在把已注册 Tuple 视为普通成功残余；实现匹配的受证提前排除也使用同一分类。没有展开 Tuple 头或反推其元素。新增 PTC 双向证明及重放测试，明确检查撤销注册后不能重放、普通不透明工厂没有这份结论，以及 Tuple 单射性仍不能被推导。

两个源码回归保留 NIR 中实际 Tuple normal-form 头，覆盖外层错误结果的成功／失败、泛型整体 Drop、以及 Tuple 第一个元素自身为另一错误结果时的四种分支组合。元组内错误留在元素里，不被提升到外层错误行；整体析构后只清理活动载荷。相关文档增加完整嵌套示例。

验证目录 `/home/fin/luna`：
- 两个源码定向回归退出 0：`/tmp/luna-tools-48xhg8qg/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib ptc::`：4 项通过，退出 0；`/tmp/luna-tools-0l8jjlsb/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test type_spreads`：861 + 114 + 92 项通过，退出 0；`/tmp/luna-tools-898mjtew/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-j6jgo63u/result.json`），`git diff --check` 通过。

非 Copy 计算结果跨候选交付仍待实现。改动保留 dev 工作区，未提交。

### 错误行规范顺序与 Drop 接收者 ABI

上轮有实现和验证进展。本轮复现语义相同但书写顺序相反的错误行选中同一 Drop 后，严格接收者 ABI 检查失败（`/tmp/luna-tools-acqnbg3y/result.json`）。没有放宽签名比较或把不同物理布局的指针强行传入析构函数。

`Term::error_result` 在原有扁平化成功限定层及精确去重之后，以稳定的完整 S-expression 类型键排列标签；参数替换重新构造时也取得统一顺序。保持全部类型实参、normal-form 头及完整标签自身的限定层，不通过排序引入新的类型等式。源码同一完整错误行不再因书写顺序不同而产生两个接收者布局。新增回归验证成功和两种错误分支经过换序转发后，整体 Drop 与活动载荷各清理一次，生产者／转发调用各一次。

底层物理布局仍保留独立顺序：`ResultType::to_term` 现在直接重建已检查布局，避免源类型构造器的规范化悄悄改变已有标签编号。NIR 与 IIR 的显式换序夹具改用原始物理类型构造，继续检验非恒等标签映射、冻结、guard、源域拒绝及预算恢复。源码测试改为检查实际 tag 指向的完整标签，不再假定 tag 等于源码书写位置。

验证目录 `/home/fin/luna`：
- Drop 换序回归修复后退出 0：`/tmp/luna-tools-q3_pap0y/result.json`。
- NIR `error_results`、`normal_inputs`、`trait_parameters`：29 + 13 + 19 项通过，退出 0；`/tmp/luna-tools-h63jr2y5/result.json`。早期三项物理布局测试因构造器将换序变成恒等而失败；调整为显式物理夹具后仍保留原映射及拒绝预期。
- IIR `error_transport` 5 项通过，退出 0；`/tmp/luna-tools-74mkx7aq/result.json`。
- 源码相关七组首轮 `/tmp/luna-tools-4nmg9q65/result.json`：CMTT 522、Drop 862 项通过；error_surface 的三项旧数字 tag 断言失败。更新为完整标签身份断言后补跑 error_surface、type_spreads、inline_control、newtypes、runtime_purity，114 + 92 + 241 + 207 + 62 项通过，退出 0；`/tmp/luna-tools-51dfaksw/result.json`。
- 解释器 `abstract_values` 退出 0：`/tmp/luna-tools-ap_27_s5/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-b_o5ju4v/result.json`），`git diff --check` 通过。

本轮覆盖相同错误标签集合的源码换序；不同类型之间的 Drop 责任适配及非 Copy 计算结果跨候选交付不由此完成。改动保留 dev 工作区，未提交。

### 冻结错误值的物理域转换与参数替换验收

上轮有实现和验证进展。本轮补齐规范行与显式物理域交界的证据，没有放宽已有实现。显式反序物理错误值冻结后保留原始域；直接作为规范化函数输入仍拒绝。通过现有受检 transport 进行完整标签映射后，成功、char 错误和 bool 错误均按目标域重建正确 tag 与载荷。首个直接往返试验 `/tmp/luna-tools-ljvzhso1/result.json` 验证了拒绝边界，最终回归同时检查拒绝和显式转换后的成功。

新增源码验收让两个 Type 参数决定错误标签：实例化为 char/bool 时换序，bool/char 时保序，bool/bool 时合并；成功和错误分支分别作为 normal-form comptime 输入，与独立的具体预期值比较。错误标签或载荷不一致的反例必须拒绝，避免只检查“能编译”而遗漏冻结值身份。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor --test abstract_values`：19 项通过，退出 0；`/tmp/luna-tools-8067_9kz/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface generic_frozen_error_inputs -- --nocapture`：含上述六组正／反例的定向回归通过，退出 0；`/tmp/luna-tools-59467oks/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-ftygpj7l/result.json`），`git diff --check` 通过。

本轮只增加验证及接口说明，未重复大范围测试。不同错误类型之间的 Drop 责任适配、非 Copy 计算结果跨候选交付仍未完成。改动保留 dev 工作区，未提交。

### 错误行扩大保留整体析构责任

复现了错误行扩大只转移载荷、静默丢弃源结果整体自定义 Drop 的漏洞（`/tmp/luna-tools-eapx_kaw/result.json`，新增拒绝测试在修复前失败）。移动检查现在在消费源值之前，从当前 CleanupSession 的已取得责任中检查源整体；源整体无自定义 Drop 时允许搬运，已选中／假设／待定责任则要求显式责任转移证据。检查不要求载荷平凡，成功和错误载荷自己的 Drop 仍执行且只执行一次。相同类型及规范化后的标签换序保持原行为。

这修复了隐式丢责任的路径，并未实现不同结果类型间的自定义 Drop 适配。相关实现限制已同步错误类型文档。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface`，864 + 115 项通过，退出 0，报告 `/tmp/luna-tools-_dtmp2t5/result.json`。中间一次编译因新增测试遗漏局部 import 失败，补齐后上述整组通过。全 workspace 格式化退出 0（`/tmp/luna-tools-ywfscmo0/result.json`），`git diff --check` 通过。改动保留 dev，未提交。

### 隐式错误转换的独立清理边界

带整体 Drop 的结果经 `make()!` 传播并重新包装返回时，外层隐式转换与传播 Match 共用源码位置，导致清理调度报告两个 TemporaryEnd 边界。修复前回归失败报告 `/tmp/luna-tools-55n6i7_k/result.json`。现在 `convert_error_value` 统一分配可追溯的生成位置，并移除调用方重复分配；转换和操作数各自保留清理入口，不削弱唯一性检查。

新增八个执行场景：直接传播、空 handler、先绑定再传播、条件分支，各覆盖成功与错误。每个场景验证构造函数只执行一次、源结果与新返回结果分别执行整体 Drop、活动载荷只析构一次。此处允许的部分移出仍经过现有析构访问检查；它不同于直接扩大错误行后丢弃源整体责任。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test inline_control --test runtime_purity`，865 + 115 + 522 + 241 + 62 = 1805 项通过，退出 0；报告 `/tmp/luna-tools-sshc7miq/result.json`。全 workspace 格式化退出 0（`/tmp/luna-tools-70rjcp7y/result.json`），`git diff --check` 通过。未运行全 workspace 测试；不同类型的整体 Drop 适配与非 Copy 计算模式结果跨候选交付仍待实现。改动保留 dev，未提交。

### 错误传播析构的实际内存行为验收

增加六组执行／拒绝配对：直接返回、传播、空 handler，各含成功和错误载荷。载荷析构通过保存的指针写回调用者布尔位置，整体析构调用普通函数；验证最终写入、整体析构与载荷析构顺序，以及后续语句只在清理后执行。配对反例把指针指向被退出函数的局部位置，必须因逃逸拒绝，而不是只检查无操作析构的次数。

探索发现明确的后续缺口：通过非 Copy 指针读取整个错误结果再匹配，在 elaboration 阶段即因缺少 Copy／内存转移契约支持而拒绝（`/tmp/luna-tools-1kwxy3ub/result.json`）。因此本轮没有声称完成任意整体载荷访问的 Drop，也未放宽指针读取限制。最终提交到工作区的测试使用已支持的字段级指针读取和写入，文档补充支持边界。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop error_propagation_ -- --nocapture`，退出 0，报告 `/tmp/luna-tools-2wxbq8nq/result.json`。全 workspace 格式化退出 0（`/tmp/luna-tools-25i710qq/result.json`），`git diff --check` 通过。本轮新增验证及限制说明，没有修改编译器实现，未重复全量测试，未提交。

### 错误结果的受检 Copy 指针读取与写回

补齐源码存储形状检查和内存摘要的 ErrorResult 分支：递归检查成功载荷与所有完整错误标签；无可构造成功值的 Never 不作为实际存储载荷。形成读取不推导 Copy，完整结果和组成类型仍由不可变协议世界检查复制资格与平凡清理。标量载荷错误结果可按既有 Copy 写回规则替换，保留自定义 Drop 拒绝。

嵌套指针读取揭示 `project_owned_storage` 只接受 nominal 变体（失败报告 `/tmp/luna-tools-207egdat/result.json`）；现按完整错误结果类型验证分支与载荷槽，保存匿名字段路径及分支 guard，继续重定位嵌套前缀。没有引入不带 guard 的错误载荷投影，也未开放任意指针聚合写回。

源码回归覆盖成功／错误标签、重复读取保留原值、整体与错误标签缺少 Copy、自定义 Drop 冲突、带指针成功／错误载荷的读取和局部来源逃逸拒绝，以及两种分支写回。存储投影单测检查嵌套 guard、非法标签／字段／不完整路径及预算耗尽。旧表面测试改到真正的资源执行入口验证缺少 Copy 被拒绝，不再要求形成阶段提前拒绝所有错误结果读取。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::memory::`：12 项通过，退出 0，`/tmp/luna-tools-61dzj052/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test runtime_purity --test newtypes`：drop 869 项通过，随后旧形成阶段拒绝测试失败，`/tmp/luna-tools-bunmrfr_/result.json`。
- 更新该测试后 `python3 -B development/check.py --timeout 900 -- test -p luna --test error_surface --test runtime_purity --test newtypes`：115 + 62 + 207 项通过，退出 0，`/tmp/luna-tools-kfimo6us/result.json`。

非 Copy 整体读取／借用匹配及自定义析构责任适配仍未实现。改动保留 dev，未提交。

### 错误结果函数载荷的读取快照与跨函数缺口

新增八个执行场景：成功／错误载荷中的 true／false 函数，分别经局部解引用和 quote fn 展开读取。读取后将原结果改写为另一函数，检验副本仍执行原目标。另有八个普通 fn／inline fn 拒绝场景，锁住当前不能取得调用目标时的安全拒绝，不把它们当成语言限制或已实现功能。

实际缺口已定位：`named_callable_result` 为指针参数构造借用根，但不从调用方传入这些借用根的函数载荷；`CallableInput` 目前只描述参数的拥有型字段，不能描述跨指针加载后的条件存储位置。`walk_read_pointer` 只能从本地存储取得函数快照，跨参数读取后的返回信息因此丢失。后续应加入有类型、带分支条件的借用 callable 输入，并在调用点传递到上下文分析；不能直接把未知目标当成已知，也不能仅检查返回表达式语法来猜测 getter 行为。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop copy_error_result_pointer_reads_preserve_callable -- --nocapture` 通过，退出 0，报告 `/tmp/luna-tools-pn4og2y_/result.json`。最初普通辅助函数正例失败 `/tmp/luna-tools-j9o6bfr1/result.json`，inline 辅助函数同样失败 `/tmp/luna-tools-5_7wmhgn/result.json`，据此保留明确的实现缺口。本轮新增测试与说明，未修改编译器实现，未提交。

### 只读调用的借用函数载荷返回上下文

接通调用方实际存储到被调函数借用根的函数身份与分支域。memory_call 在全部实参和默认值求值之后取得信息；只有已推导摘要不写内存且不会运行未建模清理时才建立本次调用的 BorrowedCode。对多个实际目标按分支条件合并函数候选与标签域，目标未知时不提供猜测证据。上下文返回分析将其安装到匹配参数、指针表示及 pointee 类型的合成借用根，正常分析完整函数体并取得返回信息，不对 getter 语法做特判，不缓存为类型固有属性。

普通 fn／inline fn 的原拒绝回归已改为实际成功执行：成功和错误载荷均保留返回的函数身份，调用后改写原结果不影响副本。新增标量函数指针的多层辅助函数转发、不同目标合并、不同错误分支目标合并、后续实参改写存储后读取最新函数，以及会写内存的辅助函数不能使用只读证据的拒绝例。初始化借用根时先安装函数字段，再安装标签域，避免字段替换逻辑清空刚建立的域。

仍未开放带任意写入／未建模清理的借用函数状态转换；嵌套指针参数和被调函数内部未知借用载荷调用也没有因此获得通用支持。当前信息仅在已验证的调用点内部传递，未扩展公开 CallableInput 的跨指针 ABI。后续需要把写入、别名关系和清理的实际作用纳入这类上下文。

验证目录 `/home/fin/luna`：
- 普通／inline 错误结果返回执行回归通过，报告 `/tmp/luna-tools-urkl4iec/result.json`。
- 多层转发、多个目标和实参后置修改回归通过，报告 `/tmp/luna-tools-l5o5qkny/result.json`；最初测试直接使用无 Copy 权限的解释器实例而失败，改为完整协议执行入口后通过，未放宽解释器权限。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test runtime_purity --test newtypes`：871 + 115 + 522 + 62 + 207 = 1777 项通过，退出 0，`/tmp/luna-tools-syopiy_k/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-o4_k8bs1/result.json`），`git diff --check` 通过。

改动保留 dev，未提交。全语言目标仍未完成。

### 聚合指针字段的借用函数载荷上下文

将只读调用的 BorrowedCode 从参数根扩展到已经过检查的 pointer_input_layout 叶。上下文现在记录参数位置及完整输入字段路径；同一参数的多个指针字段、错误结果成功／错误分支中的指针各有独立身份。安装信息时核对 BorrowedRoot 的参数、完整路径和 pointee 类型，不以共享物理槽或仅参数编号代替匹配。直接指针／已有指针表示包装继续使用原来的根路径；没有实际指针目标的参数跳过额外布局遍历。

新增执行验收覆盖一个结构体中的两个不同函数指针字段、错误结果两种活动分支的指针字段，以及运行时条件决定分支的共享载荷位置。实际返回函数随后被调用，结果分别验证 true／false，避免只检验“能形成 IR”。先前多个目标合并、实参后置修改和写调用拒绝也随定向组继续通过。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop borrowed_callable_ -- --nocapture`：通过，退出 0，`/tmp/luna-tools-zbev1asp/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test runtime_purity --test newtypes`：873 + 115 + 62 + 207 = 1257 项通过，退出 0，`/tmp/luna-tools-sfwht_he/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-qo4tm0zm/result.json`），`git diff --check` 通过。

已同步错误类型文档并补充聚合指针读取函数值的代码示例。连续多层指针加载、写入或未建模清理作用仍需后续接线；未改变只读证据的适用边界。改动保留 dev，未提交。

### 用实例的 Copy 依据细化借用函数返回的清理条件

修正“包中任意 Drop 实现使 Copy 聚合返回也无法传递函数身份”的过度保守行为。当摘要不写内存、返回值包含函数身份而粗略清理标志仍为真时，对实际实例的参数、主体、规格表达式及调用默认值检查清理资格：非内建简单表示需要完整 Copy 依据；嵌套调用另需独立的无清理摘要。未知调用、效应上下文或实际拥有型资源不获得放行。该检查有遍历／类型预算，不改写原摘要，也不把局部证据缓存为类型性质；没有函数返回信息需要恢复时不启动额外扫描。

新增正例在包内存在无关自定义 Drop 时，通过带局部 Copy 绑定的普通函数返回成功／错误分支中的函数值并实际调用。新增反例让普通函数自身或嵌套辅助函数的 Watch 析构写入调用者布尔位置，不能以只读借用信息取得返回函数身份；对应 quote fn 展开路径实际执行该清理并读到 true。默认值清理还受到独立 P3 稳定性／终止性要求，探索中的无证据资源默认值在 elaboration 阶段被拒绝，最终保留此拒绝断言，没有放宽默认值规则。遍历采用包含默认值、但不进入新建闭包延迟主体的入口，避免漏掉调用准备期间的清理。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop borrowed_callable_ -- --nocapture`：5 项通过，退出 0，`/tmp/luna-tools-6_j4snp3/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test runtime_purity --test newtypes`：875 + 115 + 522 + 62 + 207 = 1781 项通过，退出 0，`/tmp/luna-tools-7j40smmi/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-l2fi5pq4/result.json`），`git diff --check` 通过。

实际析构状态转换及任意写调用仍未纳入借用函数身份分析；此轮仅消除能够独立证明平凡清理的误拒绝。改动保留 dev，未提交。

### 平凡清理证明跨内部调用与递归实例传递

将 callee_cleanup_is_trivial 从单层检查扩展为有预算的具体实例工作列表。内部调用的摘要若已证明无清理则直接采用；若仅有粗略可能清理标记，则将其实际函数实例加入检查。按声明身份和完整实例实参去重，同一实例仅检查一次，不展开 normal-form 类型头，也不把调用环判作终止性证明。每个实例的参数、主体、规格和调用默认值仍通过原有 Copy／已知简单表示检查；任一实际拥有型资源、未知调用或效应使整个证明失败。工作列表仅存在于本次证明内，不跨世界缓存尚未完成的结果。

扩展执行回归：包中存在无关 Drop 时，通过多层普通辅助函数返回 Copy 错误结果的函数载荷；其中包含只观察 Copy 值的递归函数。拒绝回归增加递归分支中创建 Watch、由析构写入调用者位置的情况；quote fn 展开路径仍实际执行并得到 true，普通／inline 返回分析不得把这条路径当成无清理。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop borrowed_callable_ -- --nocapture`：通过，退出 0，`/tmp/luna-tools-eqgjrexs/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test runtime_purity`：875 + 62 = 937 项通过，退出 0，`/tmp/luna-tools-cpit3fmw/result.json`。
- 全 workspace 格式化退出 0（`/tmp/luna-tools-x78ylpv2/result.json`），`git diff --check` 通过。

真实析构写入与通用借用状态转换仍未实现。改动保留 dev，未提交。

### 仿射聚合值的独立平凡清理证据

将清理资格与 Copy 资格分开：CleanupSession 检查已取得的完整表示，只有根及全部字段、全部变体均为 NoCustomDrop 时才提供平凡清理证据。moves 在当前固定清理环境内保存已证明的类型，并传递给内部调用的返回分析；不向 Copy 协议增加实例，不改变移动规则。内部分析的预算现在也包含输入清理证据和效应输入的准备成本，避免子分析少计工作。

新增普通非 Copy View 多层转发及成功／错误分支 View 的执行验收；返回函数值实际调用成功。重复使用已移动 View、根有 Drop、字段有 Drop、非活动错误标签有 Drop 均不能借此绕过检查。写调用及真实自定义析构的状态转换仍未实现。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop borrowed_callable_ -- --nocapture`：7 项通过，退出 0，`/tmp/luna-tools-scivjn2r/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test runtime_purity --test newtypes`：877 + 115 + 522 + 62 + 207 = 1783 项通过，退出 0，`/tmp/luna-tools-o8_f87he/result.json`。
- 全 workspace 格式化退出 0，`/tmp/luna-tools-2fzdm6ug/result.json`；`git diff --check` 通过。

同步错误类型文档的当前实现边界。改动保留 dev，未提交。

### 多候选内存摘要保留所有候选的清理标记

修正 moves::memory_candidate_summary 合并遗漏：may_run_cleanup 与显式访问、may_reject 一样按所有候选取并，不能由首个候选决定。新增两种候选顺序下的实际执行与拒绝测试：两个只读加载器、其中一个带非 Copy 但平凡清理的局部 View，均成功执行；其中一个构造带析构写入的 Watch 时保持拒绝。直接检查两个候选的原始摘要，确认其显式访问都只读，但清理标记分别为 false/true。

修复前端到端拒绝例已被其他检查拦截（`/tmp/luna-tools-zhvrg9t1/result.json`），因此本次不声称复现了不安全执行；修复的是摘要信息合并不完整。检查中的下一缺口仍是 borrowed callable 在被调函数内部直接调用：memory::walk_read_pointer 目前只从 Local 根恢复 callable，参数根没有这类前提；moves 的 BorrowedCode 只在摘要已成功且只读时提供。下一步需要在内存摘要层表达带路径的借用代码前提，并处理写入使前提失效，不能直接提前无条件注入现有快照。

验证目录 `/home/fin/luna`：8 项 borrowed_callable 定向回归通过（`/tmp/luna-tools-leucofo1/result.json`）；补充原始摘要断言后，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test runtime_purity` 通过 878 + 62 = 940 项，退出 0，`/tmp/luna-tools-smhqtc2s/result.json`。全 workspace 格式化通过（`/tmp/luna-tools-vtt0z0hl/result.json`），`git diff --check` 通过。改动保留 dev，未提交。

### 内存摘要中的借用代码前提与内部调用传递

新增 memory::BorrowedCallableInputs 及 infer_with_borrowed_callable_inputs。外层路径选择实参中的指针，内层路径选择 pointee 中的函数字段；验证 runtime 参数、指针表示、带守卫的存储投影、代码实例及完整函数签名。缺失信息保持未知，空候选／NoReturn／Unknown 不能作为公开前提。前提进入 MemoryInstance 的缓存键及 Summary.borrowed_inputs，S-expression dump 显式列出条件；复制与遍历均计入预算。

参数根的 ReadPointer 可以据此前提恢复 named callable，支持读取后直接调用、作为普通高阶实参继续传递，以及指针跨内部 named 调用时按实际目标重建前提。多个实际目标必须全部提供代码证据；候选取并，不假定不同参数互不别名。直到写入转换得到建模前，整个固定点结果只要可能写内存或执行未建模清理，新查询就拒绝返回可用摘要。当前仅支持 named 代码和一层输入指针的 pointee 路径；不是 Copy 权限，也不是运行时存储有效性的证明。

新增源码形成后的摘要层验收覆盖：直接／高阶／指针转发调用、聚合指针字段、聚合函数字段、完整输入与摘要 dump、无前提查询不受污染、零预算、错误路径与不匹配签名、未知／空候选、前后写入与函数指针替换拒绝。同一辅助函数在一个固定点内接收不同代码前提，验证不能因缓存混用丢失后一个候选的读取、写入或析构；候选顺序均覆盖。

尚未接入 moves 的调用点快照与 luna 清理调用上下文。下一步需把 BorrowedCode 导出到摘要查询，并将完整证据随 NamedCall／调用检查上下文传递到被调函数资源分析；只改 memory_call 使摘要成功而跳过被调函数资源检查并不构成完整支持。匿名闭包环境、写入后的精确状态转换及多重指针加载仍待接线。

验证目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test runtime_purity --test newtypes`：880 + 115 + 522 + 62 + 207 = 1786 项通过，退出 0，`/tmp/luna-tools-rckdmzvn/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::memory`：12 项通过，退出 0，`/tmp/luna-tools-li4k462j/result.json`。
- workspace 格式检查通过，`/tmp/luna-tools-1e_x5esx/result.json`；`git diff --check` 通过。

中间失败分别是 fixture 中 `.*=` 被词法读成复合运算符、已有函数指针替换检查先于稳定前提检查拒绝，以及测试闭包参数的 Rust lifetime 推断；修正用例写法、准确诊断断言和参数标注后通过，未放宽安全检查。改动保留 dev，未提交。

### 借用函数调用接通资源检查与参考执行

moves 在全部实参／默认值求值后，当普通摘要因动态调用身份未知而失败时，从实际目标捕获 BorrowedCode，再查询条件化内存摘要。只有该查询成功，才把快照标为调用前提；原有只用于返回值恢复的快照不因此强制创建新调用上下文。NamedCall 与 Report 携带不透明的 BorrowedCode，复制计预算，Display 使用 S-expression 并保留函数候选、字段、捕获与标签域。调用点继续检查实际目标访问，被调函数使用匹配借用根检查 Copy、移动和清理；签名、上下文入口前提以及递归复用均核对同一份证据。

luna 的清理调度器为需要借用代码身份的实例建立逐调用清理上下文，不发布伪无条件根计划。IIR Schedule 增加 borrowed_callable_functions，解释器安装计划时核验实例及重复条目，调用时据此选择精确上下文或递归别名。缺失上下文时拒绝执行；外部无上下文调用也不能绕过此要求。旧的只按函数签名决定上下文的规则无法表达指针中代码身份，因此保留显式实例清单，而不是把所有指针参数函数都当成高阶函数。

执行验收覆盖直接借用调用、普通辅助函数转发、读出后传给高阶函数、结构体指针字段、所指聚合的函数字段，各自执行 read/no 两个实际目标；另有后续实参改写存储后读取新目标、同一辅助函数不同调用身份、递归上下文复用。缺少 Copy、候选写入或析构不能通过；篡改清单、重复清单条目、删除调用上下文均拒绝。同步 drop-elaboration 文档与可执行风格示例。

当前边界仍是 named 无环境代码及稳定只读调用。新的条件调用查询尚未接入之前“返回函数载荷”路径的独立平凡清理精化，包级粗略清理标记仍可能导致保守拒绝；不能把这当成语言限制。借用闭包环境、精确写入转换、多重间接加载与更完整的标签前提仍待实现。

验证目录 `/home/fin/luna`：
- 12 项 borrowed_callable 定向回归通过，退出 0，`/tmp/luna-tools-fk6nsk94/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test error_surface --test cmtt --test runtime_purity --test newtypes`：883 + 115 + 522 + 62 + 207 = 1789 项通过，退出 0，`/tmp/luna-tools-7ta4sf83/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p comptime_interpretor -p iir --lib`：69 + 46 = 115 项通过，退出 0，`/tmp/luna-tools-glfg7atd/result.json`。
- workspace 格式检查退出 0，`/tmp/luna-tools-sfhcim5b/result.json`；`git diff --check` 通过。

最初端到端入口在安装清理计划时报 InvalidCallPlan，据此补齐显式清单和解释器调用接线；另修正测试块表达式的 do 写法及实例身份读取方法。没有放宽 Copy、内存写入或清理检查。改动保留 dev，未提交。

### 循环与候选报告保留借用来源和隐式访问

补充执行验收确认借用函数在运行时选择多个指针目标，以及循环回边改写函数槽后重新调用，都保留正确代码身份。继续检查循环报告时发现：std::mem::take 隔离执行记录也抹去了当前调用的借用来源；Report::append 未合并 borrowed_roots 与 cleanup_call_accesses，导致循环内间接指针的清理访问无法导出，闭包清理访问也不能完整进入父报告。

新增有预算的来源复制与 take_nested_report，同一调用内的循环、模式准备、候选观察沿用来源注册表，执行记录仍隔离。循环未收敛时只保留不可变来源元数据，以支持回边携带刚产生的 pointee；未收敛的清理动作与访问记录仍丢弃。最终 append 同时合并来源与调用清理访问。独立函数、闭包及默认实参调用帧保持原有独立命名空间，不用这个同调用内合并代替帧切换。

扩展清理访问导出回归：循环内直接／间接借用、嵌套循环、循环调用闭包且由其析构访问直接／间接目标，均检查精确参数路径与写属性。最初回归 `/tmp/luna-tools-kv4hy0bf/result.json` 实际失败于丢失 enclosing invocation context。中间闭包用例误以显式写入测试 cleanup-only 导出，纠正为 Watch 析构写入后通过（`/tmp/luna-tools-eelq5ji1/result.json`），没有把显式访问混入清理专用接口。新增完整执行：循环两次调用闭包，Watch 恰好析构两次并更新调用者 bool，未重放分析轮次。同步清理文档。

验证目录 `/home/fin/luna`：
- 多目标与循环更新借用函数执行通过，`/tmp/luna-tools-rjgu542g/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`：885 + 522 + 62 + 115 + 207 = 1791 项通过，退出 0，`/tmp/luna-tools-u0mcrl72/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::moves`：20 项通过，退出 0，`/tmp/luna-tools-gu11cbfe/result.json`。
- workspace 格式检查通过，`/tmp/luna-tools-y5ou8c14/result.json`；`git diff --check` 通过。

改动保留 dev，未提交。借用闭包环境与可写调用中的代码状态转换仍是独立未完成项。

### 借用代码的分支缺失证据

补齐错误结果中借用函数调用的已知分支：原实现要求非活动分支也提供函数身份，实际执行准备失败于 dynamic call requires a richer memory contract（`/tmp/luna-tools-9fsn7uvr/result.json`）。BorrowedCallableInputs 的叶现用 BorrowedCallableValue 显式区分 Named 与 Absent；缺失条目仍是未知。Absent 必须指向函数叶，且输入指针路径或 pointee 字段路径有经过检查的变体条件。公开入口仍拒绝 Unknown、空候选和 NoReturn；内部固定点可把经验证的缺失分支转换为 bottom，沿辅助函数传递到已无外层变体包装的参数，不把它当成无条件代码前提。缓存键与 S-expression dump 保留这个区分。

调用点根据已准备参数的标签域记录不存在的指针叶；pointee 内的代码缺失则来自实际存储的标签域。资源检查导入不存在的输入时，核对相应参数标签前提；不为不存在的分支制造借用根或复制许可。未知标签保留所有候选，写入目标依旧使稳定只读查询失败。

执行验收覆盖成功／错误 View 两种分支直接调用及经辅助函数调用、指针所指 ErrorResult 函数字段、已知与运行时决定的标签，以及多层只读转发。拒绝例包括无条件标量／字段上的 Absent、非函数字段、非法变体、无证据查询，以及错误载荷中仍可能运行的写函数。同步错误类型文档的 call_at／invoke_action 示例。

验证目录 `/home/fin/luna`：
- 18 项 borrowed_callable 定向回归通过，`/tmp/luna-tools-0y5il2tg/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`：889 + 522 + 62 + 115 + 207 = 1795 项通过，退出 0，`/tmp/luna-tools-418lq6o5/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::`：59 项通过，退出 0，`/tmp/luna-tools-nlsh_b3e/result.json`。
- workspace 格式检查通过，`/tmp/luna-tools-r5i3qlmt/result.json`；`git diff --check` 通过。

最初 fixture 的 `p!=>` 被 lexer 识别为 `!=`，补空格后才取得上述语义失败证据。仍未开放借用闭包环境、写入后的精确代码状态转换或多重间接加载。改动保留 dev，未提交。

### 守卫调用的间接指针观察

memory guard 接受指针实参，再由普通调用的内存摘要与资源快照检查实际读取足迹。moves 不再要求观察实参之前已物化所有间接槽；按表达式验证观察行为和 Copy，随后完整执行分析验证来源、存活与初始化。观察前后的比较允许新增有精确 Indirect 来源的只读指针知识，仍要求已有指针事实、拥有资源与临时责任保持一致。同调用内的 guard 实参、match 和观察调用预检继续继承借用来源，不重复发布临时执行记录。

新增执行验收覆盖一至三层指针、真假分支、调用与析构各一次；拒绝写入、可能拒绝的 assumes 和悬空目标。同步 runtime-purity 示例，明确当前观察路径与一般有副作用 guard 的实现边界。

验证目录 `/home/fin/luna`：48 项 guard 定向回归通过，`/tmp/luna-tools-71e9g7jl/result.json`。第一次相关组检查 `/tmp/luna-tools-igqramzp/result.json` 在固定 2 MiB 栈的 inline-contract-cleanup 用例溢出；将观察预检及其临时报告移出递归 expr 栈帧，保持原测试栈大小，随后重新验证。

后续验证：固定 2 MiB 栈的内联契约回归通过，`/tmp/luna-tools-izmby7tb/result.json`。相关组 `/tmp/luna-tools-ltl9203p/result.json` 中 cmtt 522、drop 891、error_surface 115 项通过；newtypes 的旧断言依赖“指针实参在摘要入口直接拒绝”，改为检查摘要保留写足迹，仍保留资源执行拒绝断言。随后 newtypes 207 与 runtime_purity 62 项全部通过，`/tmp/luna-tools-9uvrlt8h/result.json`；最终五组共 1797 项通过。格式检查 `/tmp/luna-tools-368ucody/result.json` 与 `git diff --check` 通过。改动留在 dev，未提交。

### 内存摘要清理协议探测的索引与固定点共享

检查借用调用的清理精化入口时，确认公开条件化摘要仍在缺少独立清理证明时拒绝 may_run_cleanup；完整精化需要把协议选择证据同时传过 moves 与 luna 的上下文复查，不能只在调用点清掉标记。本次未放宽该边界。

消除 note_owned_cleanup 中 impl × trait 的交叉扫描：先收集含 drop 方法的 trait 身份，再检查实现，找到匹配即可结束。探测结果由单个 Infer 移至 Round，同一不可变包的函数实例和固定点迭代共享声明信息；不共享代码身份、存储事实或清理动作。预算仍覆盖声明、方法和实现遍历。

新增 96 组 trait/impl、8 层辅助调用的预算验收，分别验证有／无 Drop 实现，8192 预算内返回正确清理标记。最初 2048 预算不足以覆盖完整调用类型分析（`/tmp/luna-tools-1r4z6ayw/result.json`），调整的是该集成用例总预算，没有绕开计费。另验证 bool、指针、函数三种表示的 newtype 自定义析构都保留 may_run_cleanup，借用条件查询继续拒绝；这些用例在修改之前已通过（`/tmp/luna-tools-uqy5t94f/result.json`），排除了最初关于结构视图抹掉析构的疑点，不能宣称修复了此问题。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test newtypes`，893 + 207 = 1100 项通过，退出 0，`/tmp/luna-tools-vztoycv1/result.json`。改动保留 dev，未提交。

内存摘要单元检查 `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::memory` 通过，`/tmp/luna-tools-6i_aa5b2/result.json`。workspace 格式检查通过，`/tmp/luna-tools-99jwl4sf/result.json`；`git diff --check` 通过。

### 借用函数调用接通独立平凡清理证明

新增 infer_with_borrowed_callable_inputs_and_cleanup 与 moves 的对应上下文桥接。memory 固定点持有当前查询的 CleanupPolicy；对可能拥有清理责任的完整类型调用 is_trivial，并在本次固定点缓存已证明的类型。证明只消除这些类型的粗略清理可能性，不清掉其他调用合并来的标记、不授予 Copy、不改变显式写入检查。旧的无策略入口继续保守拒绝，查询缓存不跨 Drop 选择世界复用。

调用点的 Analyzer 使用当前资源分析的清理策略；luna 发布逐调用清理上下文前，通过 protocols 适配器重新建立同一 LangItem::Drop 选择世界的 CleanupSession，复查条件化足迹。这样合法调用不再只通过第一层然后在上下文复查失败。资源检查与实际清理计划仍正常生成。

新增端到端执行覆盖：动态选择两个借用函数，函数及转发辅助函数含无 Copy 的普通／嵌套聚合值；结果正确、选中函数恰好调用一次、没有伪造 Drop 调用。外层或字段有自定义析构仍拒绝，即使函数体为空。另一回归在同一包中依次使用 Retire、OtherDrop、Retire 协议，确认完整布局证明受选择世界约束，且每次无策略查询仍拒绝。同步 drop-elaboration 示例。

最初 check 失败于适配器使用了错误 SourceRange 命名空间，改为 nir::program::SourceRange。首次手工构造解释器的测试漏装 Copy 权限回调，在 ReadPointer 报 Unsupported；改为通过真实 copy::acquire 查询提供权限，未放宽解释器复制检查。

验证目录 `/home/fin/luna`：
- 21 项 borrowed_callable 定向回归通过，`/tmp/luna-tools-uavljq7x/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`：895 + 522 + 62 + 115 + 207 = 1801 项通过，退出 0，`/tmp/luna-tools-jr9mkyxj/result.json`。

当前只精化完整布局无用户析构的情形；自定义析构的精细读写行为、借用闭包环境及可写代码存储转换仍待实现。改动留在 dev，未提交。

验证补充：`test -p elaboration --lib verification::` 通过，`/tmp/luna-tools-blmecm2_/result.json`；workspace 格式检查通过，`/tmp/luna-tools-kl5gy8zu/result.json`；`git diff --check` 通过。

### 平凡清理证明的高阶与默认实参交叉验收

继续验证上一轮接口，而不是仅凭直接调用成功推断组合情形完成。新增源码执行矩阵：从指针读出代码后传给普通高阶函数、由局部闭包捕获并调用、经递归辅助函数转发，以及带聚合局部值的默认实参供应函数；各执行真假两组输入。辅助函数／闭包内均含无 Copy 的普通聚合责任，并在包中保留无关 Drop 实现。默认表达式显式通过 source selectors 绑定 Copy／Drop；未绑定版本仍拒绝 P3。最初用普通 compile 帮助函数导致未绑定协议拒绝，改用 compile_with_source_bindings 后全部通过，不是编译器缺陷修复。未知借用代码调用作为默认值仍需要独立 P3 证据，本用例使用可独立分析的 named 供应函数，不把已知调用点身份冒充声明侧证据。

新增拒绝用例：借用函数内部把无自定义清理的 Plain 交付两次，资源分析仍报告已移动／不可用；平凡清理证明不赋予 Copy。此次无需修改核心实现，现有接口正确覆盖上述组合。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop borrowed_callable -- --nocapture`，23 项通过，退出 0，`/tmp/luna-tools-_8uehl2y/result.json`。改动留在 dev，未提交。

### 借用调用的自定义析构无内存影响证明

CleanupPolicy 新增 is_memory_inert，默认仅接受原有 is_trivial 证据，保持已有策略的保守边界。CleanupSession 实现独立完整布局检查：收集选中析构实例，在同一协议世界查询每个方法的内存摘要，要求参数相对访问为空、没有未解释的嵌套清理，并排除程序谓词拒绝。析构体构造聚合值时摘要有粗略 may_reject 标记，使用既有 rejection Cache 和同一 CleanupSession 的 may_reject 精化，不直接清零。递归创建相同类型清理责任或相互依赖时保守返回未证明，深度和结构工作继续受预算限制。普通函数递归不由该证明承诺终止，内存摘要也不是 P3 完成证书。

条件化 memory 查询改用此证明并在本轮缓存成功类型。is_trivial 的“没有自定义析构”含义保持不变，Copy 与错误嵌入等消费者不会因空 Drop 获得新许可。实际资源计划不省略用户 Drop。

新增执行验收精确检查 entry → invoke → read → drop Outer → empty → drop Temporary → drop Inner，真假输入分别执行；覆盖外层、字段和析构体临时资源的自定义 Drop。拒绝用例覆盖析构写入、只读取接收者存储、assumes 失败、直接／相互递归清理依赖。已有外层／字段自定义析构拒绝用例改为实际拒绝谓词，因为空析构现已可证明；协议隔离用例同样保留真实行为差异。仿射重复交付反例改为带空 Drop 的 Plain，确认新证明不授予 Copy。

首次定向运行在 Outer 析构构造 Temporary 时被粗略 may_reject 拒绝（`/tmp/luna-tools-4gbxt5ex/result.json`），接入独立拒绝精化后 25 项定向回归通过（`/tmp/luna-tools-itwmbd5r/result.json`）。同步 drop-elaboration 文档，明确读足迹组合仍未支持。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，900 + 522 + 62 + 115 + 207 = 1806 项通过，退出 0，`/tmp/luna-tools-cxyje0i1/result.json`。改动留在 dev，未提交。

验证补充：`test -p elaboration --lib verification::` 通过，`/tmp/luna-tools-73wunmvw/result.json`；workspace 格式检查通过，`/tmp/luna-tools-ksv79q0v/result.json`；`git diff --check` 通过。

### 借用调用析构的接收者局部读取

将清理的无外部内存影响证明细化为：允许只读、parameter=0、无 input_fields、无 through 加载的接收者路径；任何写入或越过指针字段的外部读取仍未证明。字段路径可以由普通辅助函数访问后映射回来，不凭字段类型猜测来源。每项访问检查计预算。证明只说明不引入外部读写依赖，不替代实际 Drop 计划对剩余初始化状态和 Copy 权限的检查；CleanupPolicy 契约及文档同步说明这个区分。

新增执行验收覆盖析构读取 bool 字段、只加载指针字段值、将自身 bool 字段地址传给只读 helper，并验证 entry/invoke/read/drop Owned/peek 的执行顺序。新增成对用例：移出 token 后，Drop 读取 token.value 被拒绝，而只读仍在的 other 字段可以正常执行；未移出但尝试整体加载非 Copy 的 Token 也拒绝。现有拒绝用例新增接收者指针槽写入，并将外部读案例明确为 self.flag.*.*，保留对指针穿越的禁止。

验证目录 `/home/fin/luna`：定向 borrowed_callable 通过，`/tmp/luna-tools-9mposf01/result.json`；最终 `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，902 + 522 + 62 + 115 + 207 = 1808 项通过，退出 0，`/tmp/luna-tools-wspa67sb/result.json`。带外部读足迹的析构仍待组合，改动保留 dev，未提交。

验证补充：verification 单元组 59 项通过，`/tmp/luna-tools-4yhuwi0u/result.json`。新增断言格式检查最初失败，执行格式化后复查通过，`/tmp/luna-tools-_igw8ihy/result.json`；`git diff --check` 通过。

### 清理行为证明复用共享依赖

检查外部清理读足迹的组合路径时，确认仍需按实际清理位置映射访问，不能只用只读标志替代完整 footprint。本次未放宽外部读取。先消除嵌套析构跨内存查询的重复证明：CleanupSession 缓存完整类型的 is_memory_inert 成功结果，以及 may_reject 的否定结果（已经证明不拒绝）。缓存固定在该会话的 package／Drop protocol／World 内，保留完整 Term 键；查询失败、预算不足、递归未证明均不作为缓存事实。命中仍先对类型结构计预算，缓存插入同样计量克隆成本。资源计划和 is_trivial 的无用户析构语义不变。

新增 12 层清理依赖，每层析构创建两份下一层类型。50,000 预算内完成证明，重复查询在 64 预算内完成；零预算仍拒绝。先用不足预算查询后再以充足预算成功，确认失败不污染后续证据。不同 Reject 类型仍保留谓词拒绝，不因已有成功类型而误判。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test runtime_purity --test newtypes`，903 + 62 + 207 = 1172 项通过，退出 0，`/tmp/luna-tools-3lzva52j/result.json`。没有进行墙钟性能对比，结论仅为共享证明已缓存且上述预算验收通过。改动留在 dev，未提交。

验证补充：verification 单元组通过，`/tmp/luna-tools-vvbwnhic/result.json`；workspace 格式检查通过，`/tmp/luna-tools-qmrg4lsm/result.json`；`git diff --check` 通过。

### 外部清理读足迹的导出、重导入与调用者复查验收

沿现有 Report.cleanup_memory_accesses → RuntimeInstance.cleanup_accesses → memory 摘要 → 调用者资源复查验证完整路径。新增两个指针字段的运行时选择，以及枚举两个重叠载荷槽的间接读取：导出恰好两条 read_value 足迹，保留 parameter、through 字段路径与各自变体条件。将导出足迹放入实例覆盖层后，内存摘要重新包含每条访问；篡改读目标类型被导入检查拒绝，S-expression 保留 cleanup-accesses 与 read-value。

新增源执行矩阵覆盖直接调用、指针字段转发、枚举左右分支转发。正常目标均执行成功；在调用前移出原 Token 后，即使访问只发生于 Watch 析构，发布循环也重新检查调用者并拒绝失效目标。测试中多重指针字段读取改为先显式绑定已加载指针，再投影目标字段，符合现有 place lowering 边界。

本轮验证了已有普通调用的桥接，没有放宽借用代码查询。借用代码尚未支持的外部析构读取，仍需要解决“在条件化内存查询前取得完整清理访问”这一依赖；不能用仅有 read-only 的类型级证明发布缺失访问的摘要，也不能将不同代码上下文的清理完成状态混为同一无条件证据。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop cleanup_read -- --nocapture`，2 项测试（含导出／重导入及多组执行／拒绝场景）通过，退出 0，`/tmp/luna-tools-dgbzx_yp/result.json`。此次只扩展验收，未修改编译器实现；改动留在 dev，未提交。

### 类型相对的结构化析构读足迹

新增 drop::CleanupReadFootprint／CleanupReader 与 CleanupSession::read_footprint，实现在 drop/plan/reads.rs。足迹保留绑定的 Drop 协议、完整 subject、每个清理组件的 owned-layout receiver_path、接收者类型、选中 MethodInstance 以及方法相对的 Access。组件路径保留变体标记，不把重叠载荷或相同方法的不同字段合并。两层结构都有稳定 S-expression Display，字段只通过只读访问器公开。

遍历完整 acquired layout，使用 project_owned_storage 复核组件路径与类型。对每个选中方法查询带同一 CleanupPolicy 的内存摘要：写入、未解释的嵌套清理、未知调用、程序谓词拒绝均不能产生已建立足迹；粗略拒绝标记继续用独立 rejection 分析复核。当前可以表达方法体及普通辅助调用的外部读取；析构体新建临时资源若还有未组合的外部清理读取，仍返回未证明。足迹不证明字段仍存在、变体已选中、指针存活、Copy 或终止性，也不作为跨选择世界的授权令牌。

is_memory_inert 已复用 read_footprint，再筛除含 through 的外部读取；保留已有正向缓存及递归保护，消除原先重复的选中方法／拒绝检查实现。调用者可见的借用代码支持边界没有放宽，实际存储映射仍是后续接线。

新增验收覆盖普通类型、两个字段、枚举、嵌套枚举、newtype、泛型实例、错误结果及无自定义析构；逐项检查 receiver_path、投影类型、间接读取、协议身份、稳定 dump，以及零预算后恢复。拒绝写入、谓词失败和未知函数调用。

验证目录 `/home/fin/luna`：2 项新足迹定向测试通过，`/tmp/luna-tools-1udixh2q/result.json`。`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test runtime_purity --test newtypes`：907 + 62 + 207 = 1176 项通过，退出 0，`/tmp/luna-tools-tr6dgn8s/result.json`。改动留在 dev，未提交。

验证补充：verification 单元组通过，`/tmp/luna-tools-ifxx0b4_/result.json`；workspace 格式检查通过，`/tmp/luna-tools-490bs_nh/result.json`；`git diff --check` 通过。

### 析构读足迹组合到完整对象与外层字段

CleanupReadFootprint 新增 root_accesses 与 accesses_at：前者将所有组件读取组合为指向完整 subject 的参数零访问，后者把 subject 放到更大存储根的指定 owned path。复用 StorageTraversal 的 then_guarded／load 处理字段 owner、类型与存在条件，不手工拼接解引用前后的 guard。组件条件与外层 receiver 条件只留在所在存储根；每次 load 开始新的条件坐标。重建 Access 保留 read_value、终点类型和各加载阶段的条件，去重只针对相同静态访问，不改变实际析构执行次数。每次遍历、复制、union 都计预算。

is_memory_inert 已改用组合后的 root_accesses 判断外部加载；因此新路径进入既有调用检查，不是孤立的格式转换工具。accesses_at 仍只检查 typed storage path，不授予存活、初始化、Copy 或运行时标签事实；把参数零映射到实际调用根仍由资源分析负责。

新增外层 Envelope/Choice 与指针所指 Payload 同时为枚举的测试，覆盖一层与两层中间指针。四组外层／内层标签组合分别保留：外层 guard 路径带 Envelope 字段前缀，内部 guard 在加载后的根从空路径开始；间接层增加时阶段号相应变化。组合足迹可通过 RuntimeInstance 覆盖层重新导入 memory 摘要。拒绝错误接收者类型／字段路径，零预算失败后正常查询仍恢复，输出顺序稳定。

验证目录 `/home/fin/luna`：新增组合定向测试通过，`/tmp/luna-tools-jhx_xkx1/result.json`；`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test runtime_purity --test newtypes`，908 + 62 + 207 = 1177 项通过，退出 0，`/tmp/luna-tools-5qoq0l8k/result.json`。借用代码调用的外部析构读取尚未整体接通，本次完成的是完整对象及外层字段坐标中的足迹组合。改动留在 dev，未提交。


### 借用函数调用组合实际值中的外部析构读取

CleanupPolicy 新增完整 receiver-relative cleanup_reads 查询，CleanupSession 从已取得的类型清理足迹导出 root_accesses。memory 查询在当前 Round 内缓存成功足迹；构造、交付表达式及 owned 参数通过实际 pointer origins 映射第一个加载槽，后续加载与 guard 保持原来的阶段坐标。接收者自身的初始化、Copy、实际析构次数仍由资源计划检查。缺失来源保持 may_run_cleanup，不能发布不完整的只读摘要。

新增 memory/cleanup.rs，按 PointerInputLeaf 的物理字段与变体条件选择来源，再组合后续加载路径。任何含指针存储的赋值／指针写入，以及调用摘要导入的此类写入，均使存在外部清理读的查询保持未证明；构造时快照不是清理时状态转换证明。read_footprint 增加独立递归栈，避免 is_memory_inert 失败后回退完整足迹查询造成递归证明循环。全部推导仍计预算，缓存不跨协议选择世界。

执行验收覆盖直接构造、owned 参数交付、局部闭包、两个聚合字段、枚举两个分支；每条路径均检查正常执行与调用前移出目标被拒绝。两个字段指向同一目标时实际执行两次 Watch 析构。直接指针槽改写及辅助函数改写分别拒绝。辅助函数用例最初因 .*= 连写被词法识别为复合运算符而解析失败，修正为空格分隔；随后确认它在更早的 richer-contract 检查处拒绝，断言保留此合法拒绝阶段。

验证目录 /home/fin/luna：五组集成命令 `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，909 + 522 + 62 + 115 + 207 = 1815 项通过，退出 0，/tmp/luna-tools-enf62tue/result.json。随后扩展上述测试矩阵，`test -p luna --test drop borrowed_callable` 29 项通过，退出 0，/tmp/luna-tools-be2ynpm3/result.json。文档同步说明支持边界。改动保留 dev 工作区，未提交。

验证补充：verification 单元组通过，退出 0，/tmp/luna-tools-isvf1i0d/result.json；workspace 格式检查通过，/tmp/luna-tools-klrpljg_/result.json；git diff --check 通过。


### 外部析构读证明复用共享依赖

新增执行验收覆盖泛型资源构造与 owned 参数交付、析构内部新建有外部读取的临时资源、经双层指针读取目标；正常执行与调用前移出目标成对检查，现有组合逻辑通过，/tmp/luna-tools-ix3w9bzi/result.json。

新增 12 层共享析构依赖，每层析构创建两份下一层资源，最底层读取外部 bool。原实现无法在 100,000 查询预算内取得完整足迹，失败复现 /tmp/luna-tools-holq50wl/result.json。CleanupSession 现在按完整类型缓存成功的 CleanupReadFootprint，固定在当前 package、Drop 协议及 World 内；失败、未完成递归和预算不足不缓存。缓存命中与插入均对 subject、receiver path/type、method 泛型参数及各 Access 的路径、guard、目标类型计量复制成本。ParameterPlace 的成本计算仅向 verification 内部开放以共享规则。递归栈仍保护活动查询，缓存不含实际指针来源或存活证据。

修复后上述 12 层证明在 100,000 预算内完成，重复查询在 256 内返回相等的完整足迹。先以不足预算失败后可以恢复，零预算即使命中缓存也失败，另一有拒绝谓词的类型仍返回未证明；足迹含真实外部 read_value 访问且没有写入。定向验证 /tmp/luna-tools-avssc1ku/result.json。

验证目录 /home/fin/luna：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，911 + 522 + 62 + 115 + 207 = 1817 项通过，退出 0，/tmp/luna-tools-1j_rnzk6/result.json。workspace 格式检查通过，/tmp/luna-tools-d6zafidg/result.json；git diff --check 通过。没有进行墙钟基准测试，不宣称整编译器性能倍数提升。源码、测试及相关文档留在 dev 工作区，未提交。


### 所指枚举载荷的清理来源与最终字段条件

新增端到端矩阵暴露了两层缺失：Watch 析构复制所指 Choice 后匹配其指针载荷，借用函数调用的内存摘要已有访问，但实际 Drop 计划缺少条件化指针来源；补齐来源后，清理访问导出又把最终枚举载荷字段当普通 struct 字段投影。原失败 /tmp/luna-tools-d9b92a2c/result.json，定位后的导出失败 /tmp/luna-tools-0y78nbjw/result.json。

moves 在局部资源清理前用完整 readonly cleanup_reads 准备间接借用来源。已移出组件不重新取得清理责任；实际 acquired plan 仍检查父析构需要的字段。借用枚举的各载荷按独立条件取得 pointee，flow.pointers 与 pointer_guards 一起保存，不能以同一物理槽已有另一个分支的来源替代当前分支。聚合复制因此分别取得每个条件载荷。表达式临时资源及准备中的资源通过临时 Flow 快照执行相同准备，保留当前 borrowed_scope，实际资源状态和分支条件继续交给 plan_with_memory 检查。

cleanup_memory 导出新增最终字段的条件投影：将 footprint 最后加载阶段的 guard 映射到实际目标路径，用 project_owned_storage 检查带变体标记的投影并保持正确的加载阶段。拒绝不属于目标字段路径的 guard；继承的中间加载条件不丢弃。导出错误附带函数名以定位实际失败层。

验收覆盖 Choice 左右分支，Watch 直接构造／由 make 返回，分别绑定局部值／作为表达式临时值清理；每条路径正常执行，并在调用前移出 Token 时拒绝。另直接检查导出足迹：指针载荷本身的最终读取和经它解引用后的 bool 读取均保留标签 0、1，条件路径以各自正确根为空路径。定向通过 /tmp/luna-tools-iktiz0ho/result.json。

验证目录 /home/fin/luna：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，912 + 522 + 62 + 115 + 207 = 1818 项通过，退出 0，/tmp/luna-tools-i4mm16j1/result.json。workspace 格式检查通过，/tmp/luna-tools-ggdhzij6/result.json；git diff --check 通过。指针槽写入后的精确状态转换仍未实现。改动及文档保留 dev 工作区，未提交。

验证补充：verification 单元组通过，退出 0，/tmp/luna-tools-wsr3puks/result.json。


### Guard 中的指针选择结果与条件来源观察

新增 `read(choose(p))` 用例，choose 从所指枚举的左右载荷返回指针。先复现 memory guard 只允许 scalar/code 结果的拒绝，再复现资源观察把新取得的 GuardedIndirect 来源判为调用者修改，失败证据 /tmp/luna-tools-4fqnzk6k/result.json、/tmp/luna-tools-xmz6dqfy/result.json。

memory guard 允许 Pointer 结果，moves 的 observe_guard_call 同步允许，并对所有指针结果查询内存摘要，即使调用没有直接指针实参，也不绕过写入、未知清理和拒绝检查。返回地址的实际来源和存活继续由普通资源调用分析检查。

origins::observation_preserves_pointers 区分存储变化与新增只读知识：旧目标不得删除，其已有条件必须完全保留；新增目标必须对应报告中的准确 Indirect 槽，或 GuardedIndirect 的 base+fields 与带前缀 guards。没有来源、错误槽、改变标签或条件路径均拒绝；已知空集合不能被当成未知后再填充。比较及路径复制计预算。observe_checked_flow 继续核验状态、局部责任、临时责任及可完成性，只有上述来源有据的知识扩展获准。

端到端覆盖两种枚举标签及真假 guard 结果；选择器写入、assumes 拒绝、返回失效局部地址、枚举载荷中的失效地址均拒绝。新增单元用例核验同一槽增加另一个条件目标合法、旧候选删除和旧条件移除非法、新条件标签／前缀错误非法、零预算失败。直接以聚合值作为 guard 调用实参的旧限制仍存在，初始探测 /tmp/luna-tools-8chytkw1/result.json，不把本次指针结果支持说成一般聚合观察完成。

验证目录 /home/fin/luna：guard 定向组 50 项通过，/tmp/luna-tools-97llwu18/result.json。`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，914 + 522 + 62 + 115 + 207 = 1820 项通过，退出 0，/tmp/luna-tools-q17l8zd0/result.json。workspace 格式检查通过，/tmp/luna-tools-cygn5x9n/result.json；git diff --check 通过。源码与文档留在 dev 工作区，未提交。

验证补充：verification 单元组通过，退出 0，/tmp/luna-tools-e386s44j/result.json。


### Guard 的 Copy 聚合实参与内部指针依赖

memory guard 实参检查允许已知可读取表示，不把结构布局当成 Copy 证据；moves 的观察实参交付对 Copy 聚合保留完整表示检查，并继续取得清理拒绝证据。非 Copy 私有输入仍走原来的 pointer-free 所有权规则，诊断明确区分 Copy 资格与私有交付。被调函数只读检查从“实参本身是指针”扩展到 pointer_input_layout 中任意指针叶，布局查询在候选循环外执行；聚合不能隐藏外部写入或未解释清理。

端到端触发了结构体复制的来源缺失：从借用结构体复制整体时，未先单独加载过的普通指针字段被记录为 unknown。失败 /tmp/luna-tools-30lxzij8/result.json。聚合读取现在对无变体条件的借用槽复用 borrowed_pointer_contents，按已验证字段路径建立非 owned 间接目标；实际调用者仍检查真实存储。具体对象缺少来源时不制造此类前提。

验收覆盖普通与嵌套结构体、枚举左右分支，真假 guard 结果，并分别通过 owned 参数与指针复制传递。拒绝缺少 Copy、聚合字段写入外部目标、assumes 可能拒绝、调用前移出目标。初次负例按已有 richer-memory-contract 诊断拒绝，随后明确为 Copy／私有 pointer-free 所有权诊断；没有放松负例。一般带指针聚合的观察内构造、私有非 Copy 交付及依赖指针字段的默认实参仍未声称完成。

验证目录 /home/fin/luna：定向正例 /tmp/luna-tools-i4kpds9h/result.json，负例 /tmp/luna-tools-lakk_0iv/result.json 均通过。最终 `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，916 + 522 + 62 + 115 + 207 = 1822 项通过，退出 0，/tmp/luna-tools-xrqxygrl/result.json。workspace 格式检查通过，/tmp/luna-tools-wzzvqry4/result.json；git diff --check 通过。实现、测试与文档保留 dev 工作区，未提交。

验证补充：verification 单元组通过，退出 0，/tmp/luna-tools-i1jo841h/result.json。


### 观察区域内的指针聚合构造与私有所有权交付

observation_region 的聚合形状检查改用已知可读取表示，形状接纳不授予 Copy、私有所有权或清理权限。memory 的观察区域识别同步覆盖有聚合输入／结果的调用；函数返回的 owner 与字面构造采用相同区域规则。moves 统一 check_observation_cleanup：隐式清理不得可能拒绝，带指针布局还必须取得完整 readonly cleanup_reads。局部构造、实参交付和 owned 返回均调用该证据检查；观察前后状态比较继续拒绝消费调用者已有非 Copy 资源。

观察调用的指针内存摘要可以显式使用当前 CleanupPolicy，以组合私有资源的外部析构读取。初版全局启用导致 6 个效应处理器观察回归在析构选择仍 pending 时提前请求完整证明，证据 /tmp/luna-tools-z1_8esp4/result.json。已改为 memory_summary_with_cleanup_mode 的显式观察查询模式，普通调用仍保留原有延迟边界，不能将尚未完成的选择当成成功事实。区域中的普通指针实参也通过实际资源快照检查，避免工厂输入被错误视为 owned 聚合。

验收覆盖 struct、enum 左右分支在 guard 直接构造及局部构造后交付，真假结果均执行。含外部只读 Drop 的非 Copy Watch 分别在局部块清理、直接构造交付、工厂返回后交付、局部接收后再交付，8 组执行都恰好调用一次析构。拒绝析构写入、析构 assumes、工厂写入、消费 guard 外已有 Watch、已移出目标。原 owned_observation_factories_reject_unsafe_results_and_inputs 中 Holder(pointer) 工厂的 blanket 拒绝属于旧实现边界，现拆为真实执行正例；未知工厂、调用者 owner 消耗及失败清理的原负例全部保留。

验证目录 /home/fin/luna：54 项 guard 定向回归通过，/tmp/luna-tools-qjt3vrbd/result.json。最终 `python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，919 + 522 + 62 + 115 + 207 = 1825 项通过，退出 0，/tmp/luna-tools-0kz3sror/result.json。后续只修正过时的 pointer-free 诊断文字，无行为改动；git diff --check 通过。写入型 guard 的失败路径资源转移、依赖指针来源的默认实参等更一般能力仍未完成。源码、文档及验收留在 dev，未提交。

验证补充：verification 单元组通过，退出 0，/tmp/luna-tools-5dpf792_/result.json；workspace 格式检查通过，/tmp/luna-tools-99ogy076/result.json。


### 默认参数观察帧保留聚合内的指针前提

复现 guard 默认表达式直接读取前序 View.target 的 provenance 缺失，/tmp/luna-tools-r5v7ey54/result.json。check_guard_arguments 原先只给隔离默认帧安装代码与枚举信息；新增 install_observation_input_borrows，按 pointer_input_layout 为参数各指针叶建立独立借用前提和带标签的存储槽，并使用 fresh borrowed_scope。这些前提仅证明默认值检查在有效输入下成立，不发布为真实目标存活或不别名证据；普通 call_with_memory 仍将默认值摘要映射到实际实参并检查读取。后序未准备参数仍被既有检查拒绝。直接指针参数现在也可经过同一默认帧路径。

枚举供应函数默认值暴露前端 P3 的同类缺失：transfer_cleanup_is_pure 原先只为直接指针／指针 newtype 参数设置 borrowed_parameters。新增完整聚合 pointer_input_layout 的 BorrowedField 前提，保留带变体标记的 symbolic input path，并通过 analyze_in_package_with_field_context 检查复制、交付与清理。它仍只证明在有效输入下的 P3／清理行为，调用点负责真实生命周期；未选定 Copy／Drop 协议的例子不能借此取得 Copy 资格。

验收覆盖直接指针、struct 指针字段、只读 helper 默认供应、链式默认值、显式 .answer=false 覆盖，以及 enum 左右指针载荷。使用源协议绑定验证 Copy enum 的供应函数；拒绝供应函数写入外部目标，目标移出后触发默认读取也拒绝。初始命名覆盖用例误用冒号，已按现有语法修正为等号，未修改语言语法。guard_defaults 四项（含既有后序未准备参数负例）通过，/tmp/luna-tools-hm9umcgw/result.json。

验证目录 /home/fin/luna：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes`，922 + 522 + 62 + 115 + 207 = 1828 项通过，退出 0，/tmp/luna-tools-yrl0cnom/result.json。verification 单元组通过，/tmp/luna-tools-_1claotr/result.json；workspace 格式检查通过，/tmp/luna-tools-7kqr95k7/result.json；git diff --check 通过。后续仅补充前提不授予不别名证据的注释。实现、验收与文档保留 dev，未提交。

验证补充：default_calls 专用组 33 项通过，/tmp/luna-tools-hwzc9rlx/result.json；program::purity 单元组通过，/tmp/luna-tools-7h7dcxly/result.json，均退出 0。


### 观察区域中的局部／字段取址及默认结果链验收

先扩展默认结果链验收：指针默认值直接转发或由 choose 返回，再供后续默认值解引用；Copy 聚合默认值直接构造或由 make 返回，再供后续默认值读字段。两类路径均已通过，不修改其既有实现。随后复现 guard 内 `read(View {target:flag.ref})` 被 observation_region 的语法白名单提前拒绝，/tmp/luna-tools-gboga4i4/result.json。

observation_region 接纳 AddressOf，由普通资源分析验证位置存在和后续访问；memory::check_guard 为非区域式调用单独接纳 AddressOf，先用 pointer_operation_error 校验结果指针类型、位置结构及深度，再检查操作数观察行为。没有把取址当作读取值，也没有放宽写入、移动或目标生命周期规则。

执行矩阵覆盖直接指针调用、构造视图借用外部 bool、非 Copy Holder 的 bool 字段、观察块内先建局部值再立即借用，真假结果均执行。拒绝把已退出局部块的地址包装后传给读函数、消费既有 Holder 后读取其字段、被调函数沿视图写入目标。61 项 guard 回归通过，/tmp/luna-tools-68xd8uia/result.json；默认返回链验收也在该组中。

验证目录 /home/fin/luna：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，926 + 522 + 62 + 115 + 207 + 33 = 1865 项通过，退出 0，/tmp/luna-tools-1s1qj10q/result.json。workspace 格式检查通过，/tmp/luna-tools-ysztgzx4/result.json；git diff --check 通过。源码、验收及文档保留 dev 工作区，未提交。

验证补充：verification 单元组通过，退出 0，/tmp/luna-tools-rhqpo4f2/result.json。

### 观察块的指针结果与私有指针绑定

修复 guard 参数中的非空块返回外部地址被统一按 owned result 拒绝的问题。`observe_guard_result` 对 Pointer 结果不再要求标量／Copy 值分类；观察快照仍检查调用者资源不变，实际参数求值沿 `evaluate_pointer_value` 保留来源并在块退出时执行 `check_scope_targets`。`observation_region` 的可读值结构允许 Pointer，用于私有 let／pattern 绑定和局部重新赋值；这不许可写目标或改写调用者绑定。

新增执行矩阵覆盖真假 guard、嵌套块、循环、分支、非 Copy 容器字段、指针别名和局部指针重绑定；默认值矩阵增加块返回及 choose 后局部转发。重绑定用不同的初始／最终布尔目标检验结果。拒绝直接／别名／重绑定后的局部地址逃逸、移出对象后的字段访问、沿指针写入及修改调用者指针变量。首次复现 `/tmp/luna-tools-7zawv503/result.json`；扩展 guard 组先有 62 项通过，唯一失败是新负例把 `.* =` 写为 `.*=` 导致词法解析错误，已修正 fixture，未改变语法。

集成检查还发现旧测试把 `let pointer=outside.ref;true` 当作非法观察块；这是已被本次实现解除的结构白名单限制，并非语言语义禁止借用。保留其拒绝测试位置，改为实际沿该指针写入 `pointer.* = false`；合法局部指针借用由新增执行矩阵覆盖。该次结果为 drop 927 通过、此旧预期 1 失败，`/tmp/luna-tools-hsw_raq4/result.json`。

最终集成验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，928 + 522 + 62 + 115 + 207 + 33 = 1867 项通过，退出 0，`/tmp/luna-tools-uyu6kg5b/result.json`。workspace 格式检查退出 0，`/tmp/luna-tools-4db1ojgn/result.json`；`git diff --check` 通过。改动保留 dev，未提交。

verification 单元组通过，退出 0，`/tmp/luna-tools-6m4mdsre/result.json`。

### guard 中的指针值 match

复现 `read(flag match {true=>yes.ref,false=>no.ref})` 被旧 scalar／Copy code match 白名单拒绝，`/tmp/luna-tools-hqk1uyjk/result.json`。moves 和 memory 的观察入口现在将 Pointer 结果的 Match 交给已有 observation_region 及普通资源／内存遍历；没有在旧分支里丢弃绑定来源后单独放行结果类型。候选绑定、guard、分支结果和清理仍使用原有完整路径，实际调用负责检查最终地址。

执行矩阵覆盖真假分支、带 guard 的候选、分支内局部指针、Copy enum 两侧指针载荷和观察内临时构造的非 Copy enum。拒绝 caller-owned 非 Copy enum 被消费、移出目标后的载荷读取、分支局部地址逃逸，以及分支或候选 guard 修改外部存储。前三项定向测试通过 `/tmp/luna-tools-woooirmd/result.json`，随后新增私有／外部所有权矩阵随集成验证通过。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，932 + 522 + 62 + 115 + 207 + 33 = 1871 项通过，退出 0，`/tmp/luna-tools-bv_vuezf/result.json`。实现及文档保留 dev，未提交。此项只补足只读观察中的指针选择，不代表任意带副作用 guard 或全部资源语义完成。

verification 单元组退出 0，`/tmp/luna-tools-dc1y7nkq/result.json`；workspace 格式检查退出 0，`/tmp/luna-tools-9j9xdwcl/result.json`；`git diff --check` 通过。

### 已绑定 newtype 的观察转换

复现 `read(wrapped.as(*bool))` 在 guard 中被 conversion-result 白名单拒绝，`/tmp/luna-tools-t01u6jx8/result.json`。此前只有转换操作数不是已有位置时才进入完整观察区域；现在所有 NewtypeConvert 都进入该路径，以普通资源遍历检查实际复制／消费与来源，而不是给表示转换单独豁免。未展开 normal-form 类型头。

执行矩阵覆盖 bool 与指针表示、直接转换、局部复制后转换、观察内新构造包装的消费，以及真实真假结果。拒绝外层非 Copy 包装被消费、指针目标移出、块内地址逃逸和转换后写入目标。六组集成命令（目录 `/home/fin/luna`）`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls` 通过 935 + 522 + 62 + 115 + 207 + 33 = 1874 项，退出 0，`/tmp/luna-tools-6t9bm3gb/result.json`。

随后仅扩展测试验证函数表示转换后调用的 yes/no 身份与局部复制路径。首次未声明 Action 的 Duplicate，按独立函数值 Copy 要求正确拒绝，`/tmp/luna-tools-cs3hdht1/result.json`；补齐 fixture 的协议声明后执行通过，`/tmp/luna-tools-s07qky1v/result.json`，没有放松函数值 Copy 检查。另增缺失该 Copy 依据及被调函数 assumes false 的拒绝例。实现与文档留在 dev，未提交。

追加 `guard_newtype_` 三项测试通过，`/tmp/luna-tools-p47_zcoh/result.json`；全 workspace 格式化退出 0，`/tmp/luna-tools-dd78v218/result.json`；`git diff --check` 通过。追加阶段无生产代码修改，未重复六组集成。

### 内联指针选择器的执行次数与清理时序验收

补足 guard 指针路径的实际运行顺序验收，而非只验证结果值。`guard_pointer_selection_preserves_call_and_cleanup_order` 覆盖 match 返回／提前 return、普通调用／显式 inline 展开、单次 guard／or 短路以及真假实参，共 16 个运行场景。选择器局部 Watch 的 Drop 确实读取外部 bool（`self.target.*.*`），事件序列逐项核对 picked → retired → reading；假 guard 进入 fallback，短路只在需要时进行第二次调用。初版只加载指针字段，随后加上第二次解引用并重新验证，没有把未读取外部目标的测试当作外部清理证明。

新增拒绝矩阵覆盖 Drop 沿指针写入、Drop 中 assumes false、提前返回局部地址，普通与展开两种形式均拒绝。本轮未发现需修改生产实现的回归，没有为让测试通过改变语言语义。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop guard_pointer_`，12 项通过，退出 0，`/tmp/luna-tools-ux81os1k/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-njj5culh/result.json`；`git diff --check` 通过。只增验收及记录，不重复全量测试，不提交。

### 观察区域内的私有字段替换

复现 guard 中 `var view=View {value:not flag};view.value=flag` 被直接字段赋值白名单拒绝，`/tmp/luna-tools-fqvmh4we/result.json`。observation_region 的 Assign 现在按根是否属于区域局部绑定判断，不再要求字段路径为空；路径布局、存活状态、旧字段清理和新值获取仍由普通资源分析完成。WritePointer 是独立节点，未加入允许范围；外层根仍被排除。

新增执行矩阵覆盖普通／嵌套 bool 字段、指针字段改指向及先前复制的指针保持原目标。拒绝外层对象字段写入、借私有指针字段写外部目标、给字段重新安装失效地址后读取。另以执行事件逐项验证非 Copy Held 字段被替换时的清理：旧 Held 先 retired，随后 inspected，新 Held 在区域退出 retired，最后 selected 或 fallback；真假路径均只清理各值一次。

目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，943 + 522 + 62 + 115 + 207 + 33 = 1882 项通过，退出 0，`/tmp/luna-tools-c8riq3m_/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-x7jllew3/result.json`；`git diff --check` 通过。文档同步，改动留在 dev，未提交。

verification 单元组通过，退出 0，`/tmp/luna-tools-d79vc2gr/result.json`。

### 私有回调字段替换后的来源与拒绝证据

补充两个端到端测试检查新允许的私有字段赋值是否正确更新函数身份。yes/no 双向替换分别验证直接字段调用使用新目标、先前复制出的 saved 使用旧目标；存入会执行 assumes false 的函数再覆盖为 yes 可以调用，但仍调用被替换后的拒绝函数或保存的旧拒绝函数必须拒绝。共五个执行场景、两个拒绝场景，未发现需要修改生产实现的问题。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop guard_private_callback_`，2 项通过，退出 0，`/tmp/luna-tools-1mvxxx42/result.json`。本轮仅增加回归验收，不重复全量测试、不提交。

后续调查边界：memory/cleanup.rs 的 mutates_pointer_storage 仍在任何指针存储替换与外部清理读取共存时保守失败。不能仅删除该保护；进一步放宽需要证明私有槽未被取址／未进入捕获环境，或在状态转换后重新建立完整清理读取足迹，避免使用替换前的指针来源。本轮未放宽此限制。

### 私有指针重绑定与只读清理摘要组合

复现普通 observe 函数内 `var target=first;target=last;let watch=Watch {target:target}` 在 guard 调用时因 blanket mutates_pointer_storage 被拒绝，`/tmp/luna-tools-hr3a02uu/result.json`。新增有预算的整个函数体扫描，收集直接 Let 声明的 raw Pointer 变量，排除任何位置取址所引用的局部及闭包捕获（含恢复捕获值表达式引用）。闭包主体不向外导入候选，其自身摘要独立扫描。函数、闭包与默认表达式三个 Infer 入口接线；证明只保存在当前分析器，不跨查询缓存，也不加入 IR 身份。

仅候选变量本身的直接赋值不再触发清理摘要的全局指针存储变更屏障。原有 K::Assign 正常更新 environment 目标；已复制到 owner 的指针字段不随变量更新，因此旧／新 Watch 的外部清理读取分别保留。参数、捕获槽、聚合字段、被取址变量及 WritePointer 仍走原有保守路径。没有用该证明授予 Copy、不别名的目标关系或目标存活。

端到端覆盖真假目标、更新前后分别构造 Watch、旧／新目标移出时拒绝；无论取址发生在赋值之前还是之后，别名槽都保持保守拒绝。单元测试覆盖前后取址、捕获源与捕获槽不同身份、闭包局部不泄漏，以及零预算失败后重新查询恢复。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，948 + 522 + 62 + 115 + 207 + 33 = 1887 项通过，退出 0，`/tmp/luna-tools-eyx3d8ge/result.json`。verification 单元组通过，`/tmp/luna-tools-6g_jmdav/result.json`；之后只加强捕获测试中源／目标身份区别，未修改生产逻辑。文档同步，保留 dev 未提交。

加强后的 private_pointer_tests 三项通过，`/tmp/luna-tools-rv6u6o8v/result.json`；workspace 格式检查通过，`/tmp/luna-tools-p_v__dng/result.json`；`git diff --check` 通过。

### 私有聚合字段更新后的完整析构读取

复现 `watch.target=last` 与 Watch 的外部只读 Drop 组合被全局指针变更屏障拒绝，`/tmp/luna-tools-hwk2c14p/result.json`。将上一轮未取址／未捕获的局部存储证明从 raw Pointer 根推广为直接 Let 根（private_storage_roots），仍扫描整个主体并隔离闭包。直接 Assign 只有在该根上才免于 blanket 屏障；赋值完成、指针及函数来源更新后，必须对完整根再次调用 record_value_cleanup_reads，以捕获任意外层／嵌套析构对新目标的读取。旧访问保留，类型复制计入预算。WritePointer、参数／捕获槽及被取址根仍保守处理。

新增摘要级验收直接查看公开 memory Summary：直接字段、嵌套字段和整个子对象替换三类场景都必须包含参数 1 的 bool 读取，且没有写访问或未解释清理。主体只返回 true，不显式读取该参数，避免普通读取掩盖漏报的析构足迹。运行正例通过，新目标移出后拒绝，字段取址在赋值前／后两种情况都保持原有保守边界。

集成初次有旧断言失败（950 通过，1 失败，`/tmp/luna-tools-7ht2vewv/result.json`）：旧 borrowed-callable 用例在 watch 后创建 other，再令 watch 指向 other。直接更新现在可证明，普通清理准确发现 other 先退出、watch 后析构时目标 unavailable。将该分支断言更新为实际生命周期错误；通过 replace(slot,...) 写别名槽仍须原有更丰富状态转换证明，保持独立拒绝断言。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，951 + 522 + 62 + 115 + 207 + 33 = 1890 项通过，退出 0，`/tmp/luna-tools-1qk2eouh/result.json`。文档同步，改动留在 dev，未提交。

verification 单元组通过，`/tmp/luna-tools-ggimtk__/result.json`；workspace 格式检查通过，`/tmp/luna-tools-zo02xq5_/result.json`；`git diff --check` 通过。

### 解构绑定的私有存储与清理更新

复现 `var (target,flag)=(first,false);target=last` 与只读 Watch 清理组合被拒绝，`/tmp/luna-tools-habzstio/result.json`。私有存储扫描新增 LetPattern 的有预算绑定遍历：Binding 收集拥有根，Observe 排除候选观察别名，Remap 按源的拥有／别名属性传播，支持嵌套模式和选择模式；未解析 CodeEvidence 不发布证明。整个主体的取址／捕获排除仍在最后统一执行。赋值后的完整根清理读取复用上一轮实现。

新增执行矩阵覆盖 tuple 解构出的指针、解构资源的指针字段和 or 模式的重映射，真假目标均执行。拒绝解构变量／字段取址后的更新与外部清理组合，以及新目标移出后触发析构读取。单元测试核验重映射链保留拥有根、观察别名不升级为拥有根及零预算拒绝。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，953 + 522 + 62 + 115 + 207 + 33 = 1892 项通过，退出 0，`/tmp/luna-tools-xvpbt1o7/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-7nunvphl/result.json`；`git diff --check` 通过。文档同步，保留 dev 未提交。

verification 单元组通过，退出 0，`/tmp/luna-tools-8uy2qvwj/result.json`。

### 模式重映射证明的预算扩展性

将 collect_pattern_storage 的反复全表扫描替换为按源索引的后继表和新增事实工作列表。每个拥有／别名事实仅传播一次；未知源不建立事实，观察别名在最终根过滤时优先。构建索引、初始种子、工作项与边访问分别计入同一预算，不引入跨查询缓存或语义身份。

新增 1024 段逆序重映射链在 7000 单位预算内完成的压力验收；另验收有种子循环、无种子循环及拥有／观察别名合流。这里证明的是预算工作量不再因链长退化为平方级扫描，并非宣称 BTreeMap 操作的实际时间为严格线性。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::`，66 项通过，退出 0，`/tmp/luna-tools-w6bjdvl0/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-funnxqif/result.json`；`git diff --check` 通过。保留 dev 未提交。

源级 `destructured_private_pointer_` 两项通过，退出 0，`/tmp/luna-tools-3dnezimw/result.json`；本轮未重复六组集成。

### 私有字段清理读取在控制流合并中的验收

扩展 private_field_updates_publish_new_cleanup_reads，新增条件赋值、单次 while 更新和 while/break 更新。所有六类赋值形式均直接核验摘要含新目标参数的 bool 清理读取、无写访问及无未解释清理；主体返回 true，避免显式读取新目标掩盖析构足迹缺失。每种形式执行 first=false/true 两个输入，并保留新目标移出后的拒绝验收，共 12 个执行场景、6 个拒绝场景与 6 次摘要检查。

未发现生产实现回归，本轮只扩展行为验收。目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop private_field_updates_publish_new_cleanup_reads` 通过，退出 0，`/tmp/luna-tools-zi23_9w2/result.json`；不重复全量测试。改动留在 dev，未提交。

### 精确局部指针槽写入与清理依赖

复现 `let slot=watch.target.ref;slot.* = last` 在普通 observe 函数内被 pointer-store 白名单拒绝，`/tmp/luna-tools-xw3j9opo/result.json`。新增 write_local_pointer_slot：只接受唯一的 Local 存储目标，无剩余间接路径或变体条件；校验 storage 索引／binding／根类型及字段目标类型，排除 stable_capture_roots，要求新指针来源已知。更新普通 pointer 或 field_pointers 环境后，以整个根的新状态重新记录清理读取。外部、歧义、条件目标及捕获环境仍不发布转换摘要。

raw Pointer 的 WritePointer 不再在遍历前一律置全局清理变更标记，而必须完成上述检查或报错；其他写入仍走原有约束。写入目标在右侧求值前 resolve_local_loads，防止右侧改写参与寻址的槽后改变本次写入地址。普通 guard 观察区域本身的 WritePointer 白名单未放宽；该实现用于被调函数的完整内存摘要及资源验证。

验收覆盖局部 raw Pointer 槽、结构体指针字段、新目标析构读取的公开摘要、已保存旧指针副本，以及 `destination.*.* = do {destination.* = b.ref;last}` 的求值顺序；外部参数槽和两个可能局部目标均拒绝。存储更新后的目标存活仍由普通资源检查证明。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，956 + 522 + 62 + 115 + 207 + 33 = 1895 项通过，退出 0，`/tmp/luna-tools-kqjpfi8f/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-2y7842f1/result.json`；`git diff --check` 通过。文档同步，改动保留 dev 未提交。

verification 单元组通过，退出 0，`/tmp/luna-tools-lxjruvu_/result.json`。

### 多个局部候选指针槽的弱更新

复现条件选择两个局部槽后写入被 singleton 限制拒绝，`/tmp/luna-tools-ujty5u63/result.json`。内存摘要现与 moves::write_pointer 的既有语义一致：唯一目标强更新，多个已确认的局部无条件目标弱更新。每个候选都保留已知旧来源并合并新来源，随后重新记录对应拥有根的清理读取；缺失旧来源、空目标集合、外部／条件槽或捕获环境仍失败。集合复制与合并计入预算。

新增 raw pointer 局部与 struct 字段的真假选择执行，检验未选中的槽保持原值。独立检查两个公开返回指针摘要必须同时包含参数 0 与 1，避免强更新错误被运行结果掩盖。旧“多个局部候选必须拒绝”的实现边界测试改由新正例覆盖，原外部槽拒绝保留，并新增局部／外部混合候选拒绝。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls`，958 + 522 + 62 + 115 + 207 + 33 = 1897 项通过，退出 0，`/tmp/luna-tools-73qqji5c/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-_9ij7tg2/result.json`；`git diff --check` 通过。文档同步，改动留在 dev 未提交。

verification 单元组通过，退出 0，`/tmp/luna-tools-3li9bjjo/result.json`。

### 私有 BitVec 字段写回与动态位操作观察

复现 guard 辅助函数内部对 packet.word 的位写回被 memory 根局部变量限制拒绝，`/tmp/luna-tools-5hrifs5c/result.json`。BitAssign／BitUpdate 现在通过 StorageTraversal 校验完整直接字段路径与存储类型，保留位写回结构、索引和载荷的独立检查。observation_region 同步接纳区域自有根上的 BitAssign／BitUpdate，使确定性展开后的私有写回也可检查，未开放外部根或 BitWritePointer。

随后发现 rejection 查询遗漏动态 BitUpdate（`/tmp/luna-tools-4l96a6p4/result.json`）。补齐 BitUpdate、BitWritePointer、BitWindow、BitIndex 的所有操作数遍历；与既有 BitAssign／整数算术一样，程序验证仍承担范围和机器操作合法性，拒绝摘要只证明没有遗漏程序谓词／调用／清理的拒绝行为，不代替范围证明，不使用 PTC。

新增静态／动态位置、普通／嵌套字段、展开前／后、真假输入的 16 个执行组合，另验证动态窗口／索引读取。拒绝索引和载荷中的外部写入与 runtime predicate。负例初版动态窗口无可知宽度，改为单比特索引；封闭 assumes false 在 required comptime 阶段提前拒绝，改为依赖 p.* 的 runtime predicate 来真正覆盖拒绝摘要，未放宽语义（中间报告 `/tmp/luna-tools-_eqda2v6/result.json`、`/tmp/luna-tools-gvcaelv5/result.json`）。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls --test pure_bitvec`，961 + 522 + 62 + 115 + 207 + 33 + 14 = 1914 项通过，退出 0，`/tmp/luna-tools-lzy2xs83/result.json`。文档新增代码示例，改动留在 dev 未提交。

verification 单元组通过，`/tmp/luna-tools-bx40p0l_/result.json`；workspace 格式检查通过，`/tmp/luna-tools-bww_z1fa/result.json`；`git diff --check` 通过。

### 动态位观察的程序验证边界验收

新增 pure_bitvec 源级验收：guard 辅助函数中无范围证明的动态索引、字段位索引写回和动态窗口写回，普通 fn／inline fn 共六种组合都在程序验证阶段拒绝，诊断必须涉及 bound/range。另验证显式 `assumes i < 16` 的动态读取／写回：i=4 正常得到 true，i=20 在运行时以 ProgramContractRejected 退出，不能把成功路径范围事实当成移除该检查的依据。

初版误要求未绑定资源协议的普通解释入口提前拒绝合法 i=4；该入口实际正确执行运行时检查。已改为合法／非法输入的行为验收，而不是把资源观察路径的实现限制强加给普通解释入口。未修改生产代码。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test pure_bitvec`，16 项通过，退出 0，`/tmp/luna-tools-1hiix7h_/result.json`。仅新增测试，不重复全量集成，保留 dev 未提交。

### 观察区域中的私有指针写入与内联一致性

复现普通辅助调用可执行的局部指针写入，在确定性内联后被 observation_region 的结构白名单拒绝，`/tmp/luna-tools-4p4f93ut/result.json`。结构层接纳 WritePointer 与 BitWritePointer；moves 在写入地址求值后、右侧求值前检查完整目标集合，只允许当前观察区域的局部根，空集合或任一外部候选都拒绝。此检查按候选数计入预算，不用最终资源状态未变来替代写入权限。原有 Copy、初始化、来源更新、清理、位范围与生命周期检查继续执行；write_pointer 保留独立栈帧属性。

新增五类正例（标量、多个局部候选、结构体字段、指针槽重绑定、位写回），各执行真假输入及展开前／后，共 20 个场景。五类拒绝场景覆盖外部写入、混合目标、写后恢复和位指针写回。首次集成 962 个 drop 测试通过、1 项旧诊断断言失败，`/tmp/luna-tools-htrzzxny/result.json`；实际外部写入仍正确拒绝，诊断补回 observation region 上下文，不改变旧测试期望。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop --test cmtt --test runtime_purity --test error_surface --test newtypes --test default_calls --test pure_bitvec`，963 + 522 + 62 + 115 + 207 + 33 + 17 = 1919 项通过，退出 0，`/tmp/luna-tools-13bsouu9/result.json`。verification 单元组退出 0，`/tmp/luna-tools-woi5zir8/result.json`；workspace 格式检查退出 0，`/tmp/luna-tools-othgek7n/result.json`；`git diff --check` 通过。文档同步，保留 dev 未提交。一般有外部副作用的 guard 失败路径交付仍未在本轮实现。

### 局部函数槽别名写入与代码候选更新

首先检查写入地址求值顺序：普通标量写入、位载荷和位索引改写寻址槽的三种复现，公开 memory Summary 均保留参数 0 为唯一写目标。未发现生产缺陷，仅加入 memory_write_destinations_are_frozen_before_operand_mutations 回归，`/tmp/luna-tools-_ud6vyn_/result.json`。

继而复现局部函数槽的别名替换被拒绝，`/tmp/luna-tools-oh3313xe/result.json`。WritePointer 对 Function 值冻结写入目标后求值右侧，记录命名代码及本地闭包候选，再更新经身份／类型检查的局部无条件槽。多个候选分别从同一个 RHS 后环境执行替换，再合并环境，保留未选中槽的旧身份。环境、代码集合与字段路径的复制沿用预算检查；外部槽、条件槽和捕获根继续要求转换契约。更新整个拥有根的清理读取；保存的旧函数副本不被重写。

字段正例暴露两个关联缺口：无指针输入的 guard 辅助函数拒绝细化原先不带清理选择；memory 对所有普通聚合构造一律设置 may_reject。现在 guard 的细化查询使用当前清理选择，并同时拒绝摘要中的未知清理和外部写入；聚合载荷已遍历后，清理拒绝由选定 CleanupPolicy 判定，无选择仍保守。中间摘要明确显示 may_reject=true、may_run_cleanup=false，`/tmp/luna-tools-di_bjvh0/result.json`，并非运行时执行失败。

定向两项测试通过，`/tmp/luna-tools-g30cbj57/result.json`：根／字段替换与保存旧身份，展开前后执行；多候选真假输入；拒绝调用新拒绝函数、保存的旧拒绝函数、仍可能未覆盖的拒绝候选及外部槽写入。正例额外检查公开 memory Summary 不含拒绝或未知清理。文档同步，改动保留 dev 未提交。

集成首次 965 个 drop 测试通过、一项旧诊断断言失败，`/tmp/luna-tools-08jp3ekh/result.json`。析构中的外部指针槽自赋值仍拒绝，但改为更早报告 pointer store 缺少转换契约；仅为该明确用例接受具体诊断，其余外部析构访问与拒绝检查不变。

目录 `/home/fin/luna`，七组集成命令同上一节，966 + 522 + 62 + 115 + 207 + 33 + 17 = 1922 项通过，退出 0，`/tmp/luna-tools-x32fe_9h/result.json`。workspace 格式检查退出 0，`/tmp/luna-tools-hf3kaz6f/result.json`；`git diff --check` 通过。

verification 单元组退出 0，`/tmp/luna-tools-4s2sd219/result.json`。

### 函数槽替换后的清理边界与单目标开销

新增 guard 辅助函数内构造 Holder、通过字段别名替换函数后调用的清理验收：无操作／局部纯计算析构在展开前后均执行成功；assumes false 析构在展开前后拒绝；析构经指针字段写入外部 bool 同样拒绝。定向回归通过，`/tmp/luna-tools-3ptkcj60/result.json`。

write_local_code_slots 对唯一目标直接更新当前环境，省去整份环境的保存、复制和分支结果保存；只有多个候选才执行原来的分支快照和合并。未改变代码身份、清理读取及目标校验，不宣称未经测量的端到端性能增幅。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test drop local_code_` 退出 0，`/tmp/luna-tools-ame0nol9/result.json`；workspace 格式化退出 0，`/tmp/luna-tools-kmcyk7rz/result.json`；`git diff --check` 通过。未重复七组集成，保留 dev 未提交。

### 局部函数表的整体指针替换

复现含多个函数字段的 Copy 结构体经局部指针整体写回被拒绝，`/tmp/luna-tools-sme0v7q2/result.json`。WritePointer 现在根据完整拥有布局识别代码字段；对可读取且无数据指针的代码聚合复用 write_local_code_slots，逐字段替换代码来源并保留多目标分支合并。混合数据指针、外部／条件／捕获槽仍不放宽。资源层继续检查整体 Copy，不能用字段 Copy 替代。

新增普通／展开后的新表及旧副本各字段结果验收，另覆盖多个候选、嵌套字段、拒绝型新候选、未覆盖旧候选和缺少整体 Copy 的拒绝。首项定向通过，`/tmp/luna-tools-dtxkvzp6/result.json`。文档同步，改动保留 dev 未提交。

目录 `/home/fin/luna`，七组集成命令同前节，969 + 522 + 62 + 115 + 207 + 33 + 17 = 1925 项通过，退出 0，`/tmp/luna-tools-u1jm2fk8/result.json`。`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::memory::` 退出 0，`/tmp/luna-tools-4r87a8p8/result.json`；workspace 格式化退出 0，`/tmp/luna-tools-6qwe7761/result.json`；`git diff --check` 通过。

### 混合回调与数据指针聚合的整体替换

沿用局部函数表转换，扩展为无变体条件的指针字段：前端赋值形成采用与聚合读取相同的表示检查，Copy 资格仍交由资源层；memory 同时替换代码候选及 pointer_fields，并重新记录完整根的清理读取；moves 更新每个实际存储叶的来源，多个候选合并旧／新目标。RHS 来源或布局需要变体条件时继续拒绝，不丢弃条件后放行。外部、捕获与条件槽的内存转换仍未实现。普通数据指针聚合的被调函数摘要路径也仍与混合代码路径分开，不能泛称全部聚合写入完成。

基线 `/tmp/luna-tools-7qtasdym/result.json`；内存／资源实现后仍被前端旧表示限制拒绝，`/tmp/luna-tools-7zngnr88/result.json`；前端接通后定向通过，`/tmp/luna-tools-boa8yffe/result.json`。新增普通／展开后新回调、新指针与旧副本的结果检查，多候选真假输入，新目标移出、旧目标移出后旧副本访问的拒绝；旧目标移出不影响已强更新的新视图。公开 Summary 直接核验弱更新保留参数 0 和 1 的读取，避免只靠解释器运行掩盖静态摘要漏报。

目录 `/home/fin/luna`，七组集成命令同前节，972 + 522 + 62 + 115 + 207 + 33 + 17 = 1928 项通过，退出 0，`/tmp/luna-tools-ll3fg_tz/result.json`。随后仅加强摘要断言，三项 mixed_callback_view_ 测试通过，`/tmp/luna-tools-a0sglsyl/result.json`；workspace 格式化退出 0，`/tmp/luna-tools-6uzke_2m/result.json`；`git diff --check` 通过。文档同步，保留 dev 未提交。

verification 单元组退出 0，`/tmp/luna-tools-ro92_odr/result.json`。

### 普通指针视图的整体替换统一

复现不含回调的 View 整体替换仍走旧指针聚合限制，`/tmp/luna-tools-zylf9tlr/result.json`。memory 将具有聚合指针叶的类型也分派到统一 write_local_value_slots；裸指针及指针表示包装保留原路径。预扫描不再提前给显式 WritePointer 添加 blanket 标记：实际写入 visitor 必须完成局部来源转换或拒绝；导入调用足迹仍由 record_access 标记潜在变更。外部／捕获／条件槽没有放宽。

新增双指针字段与旧副本、展开前后行为检查；多个候选的公开 Summary 必须包含参数 0 和 1 的读取；嵌套于自定义析构资源中的 View 替换，函数体仅返回 true，摘要仍须包含新目标清理读取且无未知清理／外部写访问。拒绝新目标移出、旧目标移出后使用旧副本、缺少整体 Copy 和外部槽转换。首项定向通过，`/tmp/luna-tools-ow_7dsj7/result.json`。文档同步，保留 dev 未提交。

目录 `/home/fin/luna`，七组集成命令同前节，975 + 522 + 62 + 115 + 207 + 33 + 17 = 1931 项通过，退出 0，`/tmp/luna-tools-49ol9sld/result.json`。内存单元组退出 0，`/tmp/luna-tools-t6moa9nj/result.json`；workspace 格式化退出 0，`/tmp/luna-tools-upuo76fb/result.json`；`git diff --check` 通过。

### Comptime 回调聚合的 Copy／Drop 表示

扩大资源、捕获内存、comptime 内存、效应转移、执行及 elaboration 检查时，首个批次在构建阶段达到 900 秒超时（尚无测试输出），`/tmp/luna-tools-9j_7ans_/result.json`；同时尝试的格式化也达到 120 秒超时，`/tmp/luna-tools-k40txt2x/result.json`。两者均已确认终止，不计通过。磁盘剩余 13G；未证明超时根因，不归因于测试失败。

拆出的 comptime 混合视图用例发现 Copy／Drop 表示遍历遗漏 ComptimeFunction，导致聚合写回报 cleanup layout 未建立，`/tmp/luna-tools-aioo8fcu/result.json`。将该阶段化函数类型作为原子清理布局，并加入 Copy 的受支持表示集合；没有赋予隐式 Copy，没有擦除函数阶段。定向正例通过，`/tmp/luna-tools-4p4rb2hz/result.json`。

新增测试实际执行常量求值与 Type 形成中的混合视图替换；拒绝缺少聚合 Copy、缺少回调 Copy、以 runtime 函数替代 comptime 回调及临时目标过期。格式化重新执行成功，`/tmp/luna-tools-vvyqzenv/result.json`。文档同步，保留 dev 未提交。

扩展九组批次再次于构建阶段超时，无测试输出，`/tmp/luna-tools-ortk38of/result.json`。改为拆分检查：comptime_memory / drop / newtypes / runtime_purity 共 13 + 975 + 207 + 62 = 1257 项通过，`/tmp/luna-tools-6sakn6w_/result.json`。

捕获内存单独执行发现六个已有场景失败（88 通过），`/tmp/luna-tools-1v0i9bgn/result.json`。根因是 extend_storage_projection 的字段 owner 判断漏掉 Closure，而 owned_path_type 已支持其环境；补上匿名环境字段投影，不投影调用签名，不取消字段边界／变体条件。新增单元用例检查环境指针字段投影、加载后的 bool 类型、越界和预算耗尽。捕获内存 94 项与效应内存转移 85 项通过，`/tmp/luna-tools-oh5e3lf6/result.json`。这些失败在本次扩展验证发现，未通过基线对比确定首次引入时间。

resources 单独以 180 秒预算执行仍在编译 luna 测试目标时超时，未运行测试，`/tmp/luna-tools-43vz0f5o/result.json`；因此不再重复大批次。verification 单元组 67 项通过（含新增闭包存储投影用例），`/tmp/luna-tools-wsq90ut_/result.json`。格式化退出 0，`/tmp/luna-tools-b09tqxrh/result.json`；`git diff --check` 通过。resources 的完整验收仍缺失，不能把其他组通过外推为它已通过。

拆分 elaboration/execution 检查继续发现旧验收预期：聚合指针操作现在在前端形成、资源阶段检查 Copy／存活，原先两项前端拒绝断言改为形成成功；两个未注解的效应 lambda 先在结果推断阶段拒绝，保留该诊断，并补带结果注解的 lambda 检查效应行缺失；效应实例缺参仍拒绝，接受统一参数计数诊断。elaboration 448 项通过，`/tmp/luna-tools-2nax3jyi/result.json`。

execution 同次 1334 项通过、2 项失败。opaque Type 用例原先伪造未知 package 19 的 normal-form 头并期望成功，与已实现头描述符校验冲突；改为源码声明 Opaque<T> 并传其真实声明身份，仍验证头不展开，同时将伪造头加入拒绝矩阵。该项定向通过，`/tmp/luna-tools-eu7j2f0v/result.json`。

**尚待修复**：`compiler/luna/tests/execution.rs::writing_pointer_postconditions_export_joint_result_memory_relations` 的合法源在 887..979 发生 `verification-unknown (resource BudgetExhausted)`，`/tmp/luna-tools-2nax3jyi/result.json`。未提高预算、删断言或将其宣称通过。下一步定位联合结果／写后内存关系的证明工作量；resources 构建超时仍是独立验证缺口。改动留在 dev，未提交。

### 精确区间查询避免重复差分闭包

复现联合结果／写后内存后置条件用例在默认预算耗尽，`/tmp/luna-tools-7ys_64v5/result.json`。Polynomial::enclosure 原先在区间已为单点时仍遍历全部关系并重建差分图。现在完成区间求值及当前版本检查后，若上下界相等，直接返回这组足以确定值的区间前提；不提高预算、不跳过调用／内存契约，也不缓存跨状态证明。

原源级用例连同三个拒绝变体通过，`/tmp/luna-tools-n3djiyjf/result.json`。新增单元构造 16 段无关关系链，精确值查询在 100 单位预算内完成，只保留对应值的前提；重新赋值后旧查询必须 StaleValue。verification 单元组 68 项通过，`/tmp/luna-tools-mlnh_007/result.json`。此前记录的联合内存后置条件预算失败已由此修复；resources 构建超时仍是独立验证缺口。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 180 -- test -p elaboration --test verification`，47 项通过，`/tmp/luna-tools-feho73ts/result.json`。`python3 -B development/check.py --timeout 180 -- test -p luna --test execution --test pure_bitvec --test runtime_purity`，1336 + 17 + 62 = 1415 项通过，`/tmp/luna-tools-pk40ng54/result.json`。全 workspace 格式化退出 0，`/tmp/luna-tools-hzyt7o1d/result.json`；`git diff --check` 通过。改动留在 dev 未提交。

### 资源分析回调类型统一，关闭 resources 构建超时

发现大型 Analyzer<F,G> 将 delivery／shape 的不同闭包类型传播至整套递归分析方法；resources 直接使用大量不同 policy 闭包，导致反复单态化。内部 analyze_with_shapes_and_borrowed_code 入口将两类 FnMut 借用统一为动态回调，公开泛型接口、调用顺序、可变状态和预算均不变。不改测试分组或缩减测试。

改动后第一次 resources 构建及执行共 23.606 秒完成（此前 180／900 秒超时），`/tmp/luna-tools-kwdwaxnv/result.json`，351 通过、4 项旧预期失败。两项更新为更具体的清理证据缺失／模式越界诊断；两项区分 memory Summary 描述写入与 moves 准许 guard 写入，改为明确检查写入足迹，并单独验证 moves 仍拒绝外部写入；已有局部指针别名观察现作为正例。未删去拒绝行为检查。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 180 -- test -p luna --test resources`，355 项通过，退出 0，用时 4.537 秒，`/tmp/luna-tools-90nx0zd2/result.json`。这两个时长是本轮观测，不是严格基准测试。此前 resources 验收缺口现已关闭。workspace 格式化退出 0，`/tmp/luna-tools-ooz485v2/result.json`。改动留在 dev 未提交。

相关七组集成 resources / drop / cmtt / captured_memory / effect_memory_transfers / runtime_purity / newtypes 共 2300 项通过，`/tmp/luna-tools-t1_p92jt/result.json`；verification 单元组通过，`/tmp/luna-tools-nexxpqet/result.json`；`git diff --check` 通过。没有恢复自动提交。

### 名义指针包装的局部槽替换

复现指针 newtype 经局部地址写回被 raw Pointer 限制拒绝，`/tmp/luna-tools-ke__0gqf/result.json`。WritePointer 与 write_local_pointer_slot 按已建立的指针表示识别槽值，仍比较完整名义目标类型；不展开 normal-form 头，不绕过名义 Copy 或包装转换。根槽与字段槽复用原来的强／弱来源更新及整个根清理读取。

新增一层／两层 newtype、展开前后、新值／旧副本结果；根槽／字段槽多候选的运行结果和公开 Summary 同时保留参数 0、1；缺少 Copy、新目标移出、旧副本目标移出拒绝，旧目标移出不影响已更新的新值。定向初次通过，`/tmp/luna-tools-856s6yep/result.json`。

目录 `/home/fin/luna`，resources / drop / cmtt / captured_memory / effect_memory_transfers / runtime_purity / newtypes / comptime_memory 八组共 2316 项通过，`/tmp/luna-tools-glgbqqsr/result.json`。workspace 格式化退出 0，`/tmp/luna-tools-qb6ine_8/result.json`；`git diff --check` 通过。文档同步，改动留在 dev 未提交。

内存单元组通过，`/tmp/luna-tools-dqssldae/result.json`。

### Newtype 指针字段更新后的外层析构路径

新增摘要与执行验收发现：一／两层 newtype 字段更新后，摘要已准确包含新目标，但执行准备报 Drop field layout is missing，`/tmp/luna-tools-vseikd2d/result.json`。Drop access 的无标签路径 project 使用了只识别普通结构体的 field_type，漏掉内存分析导出的内部 newtype 表示步骤。改为复用 StorageTraversal 的类型、owner 与边界校验，再按已验证物理路径投影目标；不开放源码表示字段、不跳过变体条件。

定向通过，`/tmp/luna-tools-kyokww0n/result.json`。新验收中 observe 只返回 true，没有普通读取掩盖析构足迹；另检查更新后移出旧目标可清理、移出当前目标必须拒绝。文档同步，保留 dev 未提交。

目录 `/home/fin/luna`，前述八组集成共 2318 项通过，`/tmp/luna-tools-wfd8i0hv/result.json`；verification 单元组通过，`/tmp/luna-tools-70xl903u/result.json`。workspace 格式化退出 0，`/tmp/luna-tools-powhb9tp/result.json`；`git diff --check` 通过。改动留在 dev 未提交。

### 局部枚举整体指针替换

WritePointer 以整个值同步安装来源与标签条件，避免逐个物理载荷叶更新覆盖同槽的其他变体。多候选复用带标签的来源／guard 合并，空变体不抹掉未选中槽的旧依赖；memory 允许整体局部枚举替换，仍拒绝带条件的目标槽及外部／捕获槽。覆盖旧副本、空载荷、分支值、内联前后、混合回调载荷及 Copy／移出目标拒绝。

目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 180 -- test -p luna --test drop --test resources --test captured_memory --test runtime_purity`，四组通过，`/tmp/luna-tools-u9hgw11q/result.json`。全 workspace 格式化通过，`/tmp/luna-tools-7w8iapzf/result.json`，`git diff --check` 通过。改动留在 dev，未提交。

另发现含回调枚举经辅助函数参数传递后动态调用的 memory contract 缺口：`fn get(v:View)->bool {v match {.empty=>false,.left(f,p)=>f(p),.right(f,p)=>f(p)}}`，其中 View 为 `enum View {empty,left(Action,*bool),right(Action,*bool)}`，Action 为 `fn(*bool)->bool`。局部解构调用通过，经 get 传递仍报告 dynamic call requires a richer memory contract；复现日志 `/tmp/luna-tools-8r80ate0/result.json`。未以放宽未知调用检查掩盖该缺口。

### 含回调枚举的跨调用标签依据

关闭上一条 get(View) 的内存契约传递缺口。调用准备、guard 参数观察和 cleanup 辅助函数复核现在同时传递已校验的 EnumParameter。只有标签排除的代码叶才安装空 prepared-code 集；仍可能存在但没有身份的叶保持未知。校验参数布局、重复标签输入及代码身份与分支不存在的矛盾，不放宽公开 named-code 输入接口。

目录 `/home/fin/luna`，八组集成 drop/resources/captured_memory/runtime_purity/effect_memory_transfers/cmtt/comptime_memory/newtypes 共 2322 项通过，`/tmp/luna-tools-9v99vm2r/result.json`。新增用例覆盖 helper 参数、空分支、弱更新、内联前后；补充直接 guard 调用的正负配对后定向测试通过，`/tmp/luna-tools-17wgfgv0/result.json`。格式化通过，`/tmp/luna-tools-gqgfm966/result.json`，`git diff --check` 通过。改动留在 dev 未提交；不宣称未知回调或任意捕获代码均已支持。

verification 单元组通过，`/tmp/luna-tools-70_j65nn/result.json`。

### Guard 摘要复用与枚举返回链验收

同一次 observe_guard_call 中，指针观察已经取得的完整 memory Summary 直接用于后续拒绝行为检查，避免为相同参数、代码、标签与 cleanup world 重复求解。复用范围仅为当前调用候选的一次检查，不跨状态变更或候选事务。新增返回链用例验证代码和指针共同经 forward/make/get、多种退出路径及内联传递；目标移出／离开局部作用域仍拒绝。返回链初次即通过，属于已有能力的组合验收，不宣称新实现了返回值机制。

目录 `/home/fin/luna`，定向测试 `/tmp/luna-tools-zoew898v/result.json` 通过；drop/runtime_purity/captured_memory/resources 四组通过，`/tmp/luna-tools-esl57_et/result.json`。格式化通过，`/tmp/luna-tools-mmn2dnqp/result.json`；`git diff --check` 通过。改动留在 dev 未提交。

### 错误结果中的回调视图组合验收

本轮扩大前端集成检查：`python3 -B development/check.py --timeout 180 -- test -p luna --test error_surface --test execution --test elaboration`，115 + 1336 + 448 = 1899 项通过，`/tmp/luna-tools-fsv3vyr7/result.json`。新增 error_callback_views_preserve_memory_through_propagation_and_handlers，成功载荷内函数及指针经错误传播和 handler，再由观察区域调用；覆盖成功／错误、不同数据值、内联前后。定向通过，`/tmp/luna-tools-y81nwsxx/result.json`。这轮是组合验收，未发现或声称修复新语义缺口。workspace 格式化通过，`/tmp/luna-tools-qjxx8lvy/result.json`。改动留在 dev 未提交。

### 普通 for 接线前的协议核对与 elaboration 集成检查

普通 for 在 program.rs 仍显式拒绝，HIR 保留主体／模式／guard；library/builtin/protocols/iterator.fl 是旧拉取式 Iterator.next，docs/control-flow/basic.md 写 IntoIterator/Iter，而作者此前提出整体迁移 inline fn + inline effect。不能把范围 inline for 或旧 next 名称约定当作已完成的普通迭代协议。已就协议方向提出最小问题，尚未改写库或降码。

独立检查：目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 180 -- test -p elaboration` 全部通过，退出 0，`/tmp/luna-tools-g6kh_trb/result.json`。本轮没有发现该 crate 的基线失败，不因此宣称语言实现完成。改动留在 dev 未提交。

### 完成值中的错误载荷重建

constants.rs 将完成的 ErrorResult 序列化为 Success/Failure ErrorConvert，但 nir::completed_values 原先拒绝所有此类节点。NIR 定向复现 `/tmp/luna-tools-u2whg09l/result.json`。现按已有 error_results::validate_conversion 检查输入／输出域，且只允许完成载荷的 Success/Failure 封装；继续递归检查封闭性及预算，任意声明调用、错误域和不足预算均拒绝，不将 Embed/Product 执行程序视作完成值，不展开 normal-form 头。

目录 `/home/fin/luna`，NIR completed_values/associated_initializers/error_results 共 39 项通过，`/tmp/luna-tools-vr6y1qeu/result.json`。普通及泛型 associated error value 源码用例通过；error_surface/execution 集成通过，`/tmp/luna-tools-fpkkvfnk/result.json`。格式化通过 `/tmp/luna-tools-ahq4pogw/result.json`，`git diff --check` 通过。改动在 dev 未提交。

相邻待核对项：constants.rs 同样用 NewtypeConvert 重建包装值，而 completed_values 尚无该分支。接纳需要与名义包装的表示检查一致，不能仅因目标是 Nominal 就接受任意转换。

### 声明驱动的 newtype 完成值重建

关闭上一条 NewtypeConvert 的完成值缺口。新增 completed_values::validate_with_newtypes，以声明迭代器传入表示证据；复用 newtypes::conversion_from 检查完整名义实例与直接 Wrap 边，不猜测参数表示，不展开 normal-form 头。只有包装是重建，解包仍为执行操作。载荷递归使用同一声明证据与预算；旧无证据入口保持拒绝。

接线到 associated value 选择前后、泛型 associated 模板快速路径和解释器 dispatch 实例准入。NIR 覆盖泛型表示、错误结果嵌套、缺失／重复声明、错误载荷、解包及不足预算；源码覆盖普通／泛型实现的包装常量与错误载荷。

目录 `/home/fin/luna`：NIR completed_values 8 项通过 `/tmp/luna-tools-0tv_nxgj/result.json`；newtypes/error_surface 共 324 项通过 `/tmp/luna-tools-w4ul40j7/result.json`；elaboration associated 18 项通过 `/tmp/luna-tools-3n_esyab/result.json`；解释器 associated 过滤命中 0 项，未作为通过证据；随后完整 lib 69 项通过 `/tmp/luna-tools-l7o9ba8_/result.json`。格式化通过 `/tmp/luna-tools-ibqk5a0f/result.json`，`git diff --check` 通过。改动留在 dev 未提交。

### 泛型替换保持完成值的错误行封装

新增 NIR 复现：`Success(T, value)` 为完成值，T 代入带错误行的类型后，program 类型重写按既有规则将 Success 改成 Embed，原 completed_values 因只允许 Success/Failure 而拒绝。失败证据 `/tmp/luna-tools-hihu6bhy/result.json`。现允许对递归验证完成的载荷进行 Embed，复用原错误行转换校验，不允许丢失标签或改变成功域，也不放开任意调用。这里修正前两轮对“所有 row transport 都是待执行程序”的过窄分类；Product 的布局依据仍需另行接入。

目录 `/home/fin/luna`，NIR completed_values/error_results/associated_initializers 共 41 项通过，`/tmp/luna-tools-fgbx0g_p/result.json`；luna error_surface 116 项通过，`/tmp/luna-tools-x3rhtvin/result.json`。格式化通过 `/tmp/luna-tools-l9wix19j/result.json`，`git diff --check` 通过。改动留在 dev 未提交。

### 完成值的产品表示转换与正式 Tuple 证据

completed_values 新增 validate_with_layouts 接口，Product 转换复用 error_results 的完整布局迁移检查，并递归要求输入是完成值。结构元组／匿名记录可直接检查，正式 Tuple 必须提供已注册 head 证据；不把同形 normal-form 参数当作布局。既有 newtype 证据继续沿递归传递。泛型 associated 模板用绑定 Tuple 身份，World 用 PTC Context 中已注册的 tuple head（只取类型布局登记，不把程序规格交给 PTC），解释器用通过元数据校验的 Tuple head 集合；集合仅在准入阶段建立一次。

NIR 用例覆盖错误行重排、结构／正式元组、匿名记录、未注册头、缺失标签、成功域不匹配、字段改名及预算。目录 `/home/fin/luna`，completed_values/error_results 39 项通过 `/tmp/luna-tools-ags2o0uc/result.json`；newtypes/error_surface 324 项通过 `/tmp/luna-tools-bu6dn0r9/result.json`；解释器 lib 69 项通过 `/tmp/luna-tools-zsovvcaf/result.json`；elaboration associated 18 项通过 `/tmp/luna-tools-g0g1sfd_/result.json`。格式化通过 `/tmp/luna-tools-bhmmwya7/result.json`，`git diff --check` 通过。改动留在 dev 未提交。

### 完成值阶段集成与真实 dispatch 准入覆盖

NIR 全套 327 项通过 `/tmp/luna-tools-v4cch354/result.json`；comptime_interpretor 全套 187 项通过 `/tmp/luna-tools-q4a7vna4/result.json`。目录均为 `/home/fin/luna`，入口 development/check.py，退出 0。

新增 interpreter_admission_rechecks_completed_newtype_dispatch_values。初始源码 fixture 并未包含 demanded completed dispatch value，断言揭示旧源码用例无法单独证明该准入入口；现使用源码生成的类型／trait／implementation 元数据，显式建立完成值 dispatch 实例，验证合法值通过、改动 newtype 表示后拒绝。定向通过 `/tmp/luna-tools-5cfpdu14/result.json`。本轮未修改实现逻辑，补的是准入边界的真实覆盖。格式化通过 `/tmp/luna-tools-s6tot1d2/result.json`，`git diff --check` 通过。改动留在 dev 未提交。

### 完成字面量的域校验与解释器一致性

完成值原来对 Bool/Char/Str/Symbol 等仅做大小遍历，对常量节点的注解域没有统一校验。新增 Literal::has_primitive_type，核对所有内建字面量的域及固定整数的符号／范围；completed_values 与解释器 eval_literal 共用它，去除后者重复的数值类型检查。Unit 和 TypeValue 也核对注解与 Type sort。此检查不代替整数／浮点拼写解析和 BitVec 载荷准入。

新增 NIR 各字面量正负域、整数越界、伪造 Unit/TypeValue；dispatch 准入测试同时将实现与实例的 Bool 载荷改为 Char 而保留注解，确保表达式一致不能掩盖错误域。目录 `/home/fin/luna`，completed_values 11 项通过 `/tmp/luna-tools-0tptqrwp/result.json`；解释器全套 187 项通过 `/tmp/luna-tools-vc3p7c6c/result.json`；newtypes/execution 共 1545 项通过 `/tmp/luna-tools-9woccizq/result.json`。格式化通过 `/tmp/luna-tools-htfkaolp/result.json`，`git diff --check` 通过。改动留在 dev 未提交。

### 完成字面量的有界载荷准入

completed_values 对 Integer/Float 拼写和 BitVec 原始字块复用 comptime_runtime 的精确解析／from_words 校验；整数冻结形式的负号按解释器规则处理，浮点指数不经宿主浮点舍入。解析工作与缓冲大小受调用者剩余 fuel 约束，失败同样扣减，不额外创建独立的大预算。非法拼写、错误字块数和非零填充位不能作为完成值进入 associated/dispatch 路径。

正例包含负十六进制、大整数、超宿主宽度浮点指数、零宽与跨字块 BitVec；拒绝例覆盖非法数字／指数、缺块／多块／填充位；低预算失败后独立充足预算重试成功。目录 `/home/fin/luna`，completed_values 12 项通过 `/tmp/luna-tools-2m0wx2p0/result.json`；newtypes/execution 1545 项通过 `/tmp/luna-tools-70a34t18/result.json`。格式化通过 `/tmp/luna-tools-banub0m3/result.json`，`git diff --check` 通过。改动留在 dev 未提交。

### 结构产品完成值的字段校验

completed_values 不再仅递归遍历 Tuple/AnonymousRecord 的子表达式：现在验证字段数量及各字段类型，匿名记录还拒绝重复字段名；正式 Tuple 只通过已注册 head 与单个 TypeListValue 参数取得布局，保留原 normal-form 类型，不将同形头当作元组。名义 struct/enum 的完整声明布局校验仍是独立待办，本轮没有将结构产品规则假装推广到所有名义构造。

目录 `/home/fin/luna`，completed_values/associated_initializers 共 16 项通过 `/tmp/luna-tools-zdzx_rhb/result.json`；execution/cmtt 共 1858 项通过 `/tmp/luna-tools-ocntxp9u/result.json`。正负例覆盖字段数量／类型、重复名称、正式 Tuple 的注册及缺失证据。格式化通过 `/tmp/luna-tools-nsml4zby/result.json`，`git diff --check` 通过。改动在 dev 未提交。

2026-10-22：完成值准入补齐 struct／enum 的声明布局校验，覆盖构造器身份、构造种类、枚举标签、字段数量及预算内泛型字段实例化；关联值获取、泛型模板和解释器 dispatch 准入共用校验，递归保留声明证据，不展开 opaque normal-form。新增 NIR 拒绝例、嵌套组合和预算失败恢复，以及泛型／非泛型实现提供 struct、enum 关联常量的源码执行用例。工作目录 `/home/fin/luna`：`development/check.py --timeout 180 -- test -p nir --test completed_values --test associated_initializers` 17 项通过（`/tmp/luna-tools-klqx3tdd/result.json`）；luna execution/newtypes 1545 项通过（`/tmp/luna-tools-zcja5e91/result.json`），新增源码用例单独通过（`/tmp/luna-tools-rfcnh_oi/result.json`）。全 workspace fmt 完成（`/tmp/luna-tools-hdpa6llo/result.json`），git diff --check 通过。改动保留 dev 工作区，未提交。

2026-10-22：补齐关联值实例化的复制前预算：`copy_budget::charge_substitution` 保守计入表达式（含共享闭包体）与实参展开的乘积，避免仅分别检查输入大小后在 substitution 中放大分配。AssociatedInitializer、关联值获取和解释器 dispatch 完成值比对共用此检查。新增共享闭包体重复类型展开的预算失败／恢复测试，以及 struct／enum 模板和 dispatch 同时篡改字段数、字段类型、构造身份、构造种类／标签的解释器拒绝测试。工作目录 `/home/fin/luna`，所有命令通过 `python3 -B development/check.py --timeout 180 -- test ...`：nir associated_initializers 4 项通过（`/tmp/luna-tools-h6mw0plt/result.json`），luna execution/newtypes/cmtt 2068 项通过（`/tmp/luna-tools-_k6jp1ud/result.json`），新增 nominal dispatch 拒绝用例单独通过（`/tmp/luna-tools-5ms_38lr/result.json`），elaboration associated 通过（`/tmp/luna-tools-qxc5u82_/result.json`）。全 workspace fmt 与 git diff --check 通过。展开预算目前采用保守上界，并非精确内存计量；未提交。

2026-10-22：CMTT generator 的四处类型替换统一接入 `instantiate_parameters_bounded`：方法捕获、签名实例化、候选提示与最终类型计划均在复制展开类型前计费，失败也保留已消耗预算。新增重复嵌套类型参数的 expr code 转发测试，验证 runtime context 保留、两种布尔输入和错误实参拒绝。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 180 -- test -p luna --test cmtt` 522 项通过（`/tmp/luna-tools-ky11_p0d/result.json`），新增用例单独通过（`/tmp/luna-tools-eo1xzje3/result.json`）。全 workspace fmt 完成（`/tmp/luna-tools-u7oq5kqi/result.json`），git diff --check 通过；未提交。一般迭代协议和开放上下文的功能缺口仍在，不将本次预算修复视作这些功能已实现。

2026-10-22：CMTT 候选提示与子句默认值的八处 declaration substitution 接入展开前预算检查。NIR 提供 `charge_parameter_substitution` 分离分配计费与完整类型作用域准入，完整 `instantiate_parameters_bounded` 仍验证 caller frames；生成器处理签名片段时保留原有声明替换检查，不把缺少 enclosing binder 的片段当封闭类型。最初直接套完整类型入口导致 185 个 CMTT 回归失败，已定位并修复；未放宽完整类型准入。新增 NIR 测试覆盖重复展开预算、开放片段、缺少 frame 拒绝和带 frame 成功。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 180 -- test -p luna --test cmtt` 523 项通过（`/tmp/luna-tools-qwqqs8o8/result.json`）；`test -p nir --test type_computations` 13 项通过（`/tmp/luna-tools-j5200e2c/result.json`）。全 workspace fmt（`/tmp/luna-tools-hdbfm54g/result.json`）及 git diff --check 通过。未提交。

2026-10-22：阶段性集成先验证 nir／comptime_interpretor 共 520 项测试（`/tmp/luna-tools-9y09czm5/result.json`）。随后将表达式 substitution 的乘积上界计费改为依照 declaration identity 的实际替换位置计费，复用展开类型遍历；共享闭包体仍纳入遍历，组合表达式／类型深度共同受限，未使用输入只检查一次。AssociatedInitializer、关联值获取和解释器 dispatch 同步提供参数 binder。新增未使用大实参不虚增展开量、错误实参 sort 仍拒绝的回归，原重复展开预算失败与恢复用例继续通过。工作目录 `/home/fin/luna`，经 development/check.py：NIR associated_initializers 5 项通过（`/tmp/luna-tools-0dlyx0ge/result.json`），luna execution/newtypes/cmtt 通过（`/tmp/luna-tools-cnmt3t7n/result.json`）；中途变量名编译错误已修复后重跑。全 workspace fmt（`/tmp/luna-tools-z_g3ntmt/result.json`）及 git diff --check 通过，未提交。

2026-10-22：关联类型投影的完成 Type 模板改为复制前检查实际替换展开量；成员契约、接口类型命题与 capability 输入，以及保持开放 caller telescope 的规范化路径，同步进行实例化前计费。保留原有 scope/sort 检查及 opaque head 身份，不把内存规格引入 PTC。新增大型泛型关联 Type 的低预算失败、同 World 高预算重试成功及选择证据检查。工作目录 `/home/fin/luna`，development/check.py 检查：elaboration associated 19 项通过（`/tmp/luna-tools-v495ou5l/result.json`），luna execution 的 associated 组 114 项通过（`/tmp/luna-tools-osy4rwcq/result.json`），luna newtypes 211 项通过（`/tmp/luna-tools-ddbszvit/result.json`）。全 workspace fmt（`/tmp/luna-tools-_oy74sy2/result.json`）与 git diff --check 通过；未提交。

2026-10-22 阶段性集成证据：在 `/home/fin/luna`，通过 `python3 -B development/check.py --timeout 180 -- test -p elaboration -p iir` 验证 29 组共 683 项测试，全部通过（`/tmp/luna-tools-p6grcqbi/result.json`）。随后 `--timeout 300 -- test -p luna --test error_surface --test resources --test drop --test runtime_purity --test effect_spreads --test comptime_memory` 共 1554 项通过（`/tmp/luna-tools-svknyau2/result.json`），覆盖错误结果、资源交付与清理、纯度、效应展开和 comptime 内存行为。git diff --check 通过。构建入口正常自动清理 luna 可重建产物 1.2 GiB。此轮未修改实现，不将测试通过解释为尚未实现的一般迭代协议、开放闭包上下文或其他语言缺口已完成；下一阶段可直接推进功能，无需无改动重复这些集成组。未提交。

2026-10-22：修复 suppressed inline range 的阶段漏洞。未选分支／空外层迭代中的边界此前只检查 Integer 类型，可能接受 comptime 函数转发 runtime bool 参数；新增源码回归在修复前证实可被接受（`/tmp/luna-tools-kk7192k6/result.json`）。现在检查边界输入的 comptime 可用性及调用阶段，但不执行边界。检查外层空／失败迭代主体期间保留索引的 staged 身份并在退出时恢复，使内层合法依赖不被误拒绝。覆盖 runtime 参数与 runtime 调用、起止边界、未选分支／空范围，以及不终止 comptime 边界不执行和开放函数 staged 参数正例。文档同步说明。工作目录 `/home/fin/luna`：development/check.py inline_control 全部 243 项通过（`/tmp/luna-tools-anxhn_m2/result.json`），cmtt inline 相关组通过（`/tmp/luna-tools-5kdhmmb6/result.json`）。全 workspace fmt（`/tmp/luna-tools-1yx0wh29/result.json`）、git diff --check 通过，未提交。空 tuple 主体检查和一般 iterable 协议仍未实现。

2026-10-22：补齐 inactive inline if／when／模式选择输入的阶段检查，并检查 inline when 首次命中后未调用的条件。使用已完成普通 elaboration 的 selector 验证 comptime 输入，不再次执行 selector，也不递归限制分支主体中的普通 runtime if。新增拒绝例在修复前复现（`/tmp/luna-tools-pwa0bg4l/result.json`），修复后 inline_control 245 项通过（`/tmp/luna-tools-l4pjxkvh/result.json`）；空外层索引、不终止 comptime selector 与 runtime arm body 组合正例也已纳入，相关两项单独复核通过（`/tmp/luna-tools-tdlcjymg/result.json`）。cmtt inline 11 项通过（`/tmp/luna-tools-upmc32q2/result.json`）。工作目录 `/home/fin/luna`，均经 development/check.py；全 workspace fmt（`/tmp/luna-tools-1pfeyhv2/result.json`）及 git diff --check 通过。文档已同步，未提交。

2026-10-22：inline 模式分支补齐 guard／Then 观察计算的阶段准入；整个选择被抑制或前一 arm 已命中时也检查 runtime 依赖，但不调用未选计算。抑制的 HIR Match 用临时选择标记标识，退出时恢复；每个 arm 的模式绑定在主体检查期间保留 staged 身份并恢复外层集合，普通 runtime arm 控制流不提升阶段。修复前新增拒绝例失败（`/tmp/luna-tools-n7sjrs7w/result.json`）；修复后 inline_control 247 项通过，包含带不终止 guard／观察的正例和嵌套 staged 绑定（`/tmp/luna-tools-59zgq2if/result.json`），cmtt inline 11 项通过（`/tmp/luna-tools-xmnlgihr/result.json`）。工作目录 `/home/fin/luna`，经 development/check.py；全 workspace fmt（`/tmp/luna-tools-ck0_14b5/result.json`）与 git diff --check 通过。文档同步，未提交。

2026-10-22：修复 inline 模式阶段检查只使用最终导出 binder 的误拒绝。or-pattern 后续候选内的 guard binder、Observe 私有别名等不一定出现在最终 locals；改用已有 lexical free-binding 分析区分模式内部引用与外部依赖，再检查每个观察表达式的阶段。inline match、Integer range 模式和 tuple_enumerate 索引模式共用此入口；主体仍只获得正常导出绑定。新增第二候选 guard 正例在修改前失败（`/tmp/luna-tools-tbschvbj/result.json`），修复后 inline_control 248 项通过（`/tmp/luna-tools-qopc2bek/result.json`）；新增范围／tuple 索引执行和外部 runtime guard 拒绝后，branch_local 两项通过（`/tmp/luna-tools-xtqk0t54/result.json`）。cmtt inline 11 项通过（`/tmp/luna-tools-37fn7js3/result.json`）。工作目录 `/home/fin/luna`，经 development/check.py；全 workspace fmt（`/tmp/luna-tools-whc_x9ia/result.json`）及 git diff --check 通过，未提交。

2026-10-22：核查 CMTT 引用接收者后确认其固有／trait／默认方法调用、共享存储写回及 quote 快照已实现。`development/check.py --timeout 180 -- test -p luna --test cmtt_methods` 在 `/home/fin/luna` 完成 142 项测试，全部通过（`/tmp/luna-tools-ngo1f0a1/result.json`）。修正 docs/cmtt/index.md 中仍声称 `*self` 未接通的矛盾说明，并将生成器对 PositionalDependencyCatched 的旧引用接收者诊断改为实际未支持的 dependent parameter bindings。本轮未新增接收者实现；git diff --check 通过，未提交。

2026-10-22：程序验证层新增目标导向的多项式单调性范围收紧。普通 enclosure 不足时，在原输入区间盒上计算各偏导数区间；符号确定才按方向替换端点，保留重复变量相关性，再与原范围相交。共享步骤／整数／项数预算，次数上限不变；证明保留被消去变量的版本及原有关系，replay 使用同一规则。接入机器算术检查，源码 `u8` 在 `1<=x<=6` 下的 `x*x*x-x` 可证安全并执行为 210。测试覆盖负系数、多变量、内部极值不得仅凭端点证明、预算失败后恢复、输入更新使证明失效，以及最终抵消不掩盖中间溢出。工作目录 `/home/fin/luna`，development/check.py：elaboration verification 50 项通过（`/tmp/luna-tools-_qg47g6h/result.json`），luna resources 356 项通过（`/tmp/luna-tools-c8j2_v9z/result.json`），luna execution 1337 项通过（`/tmp/luna-tools-5p4cju0y/result.json`）。文档补充已执行示例与边界；全 workspace fmt（`/tmp/luna-tools-mifyt36s/result.json`）及 git diff --check 通过，未提交。该算法不完全，无法确定导数符号／缺少界／预算不足继续保留 Unknown，不进入 PTC。

2026-10-22：多项式单调性收紧接入程序 predicate 的区间判定，复用范围证明的端点规则；普通 enclosure 已决定真假时不增加收紧，线性式与单项式跳过无益尝试。保留严格／非严格／相等边界及版本校验。新增 `x³>=x` 在 `[1,10]` 可证、严格大于／相等无法普遍决定、改写输入使证据失效的验证测试；源码 `u8` 函数以 `ensures result` 返回 `x³>=x`，不需 assumes，改成严格 `>` 则拒绝。工作目录 `/home/fin/luna`，development/check.py：verification 51 项通过（`/tmp/luna-tools-f3k5z0gy/result.json`），luna execution/resources 通过（`/tmp/luna-tools-j080__1p/result.json`）。文档加入已验证契约示例。全 workspace fmt（`/tmp/luna-tools-z7f9964p/result.json`）与 git diff --check 通过，未提交；超出当前有界算法能力仍 Unknown，不涉及 PTC。

2026-10-22：程序整数多项式加入有限差分单调性回退。导数区间不能确定方向时，预算内展开 `p(x+1)-p(x)` 的非抵消项；总次数不增加，差分区间符号确定才采用端点。该规则覆盖非负整数上的 `x²-x`／`x³-x`，正负系数共用，仍保留输入版本与 proof replay。源码确认 `u8` 的 cubic 比较后置条件可含零，`u16` 的二变量三次式 `x*x*y-x*y` 在范围约束内安全且执行为 270。内部极值和中间溢出拒绝例继续通过。工作目录 `/home/fin/luna`，development/check.py：verification 52 项通过（`/tmp/luna-tools-byzuko54/result.json`），execution 1339／resources 356 项通过（`/tmp/luna-tools-87pn8gdo/result.json`）。文档同步，全 workspace fmt（`/tmp/luna-tools-vdeg6vdf/result.json`）及 git diff --check 通过，未提交。有限差分仍是保守有界推理，不表示所有三次规格均可判定。

2026-10-22：程序验证加入小整数区间盒的完整枚举回退，接入范围证明、predicate 与 proof replay。单调性仍不足时，对至多 256 个格点求值，覆盖内部极值；不枚举已经具有精确盒范围的单项式／仿射式，不使用部分枚举作为证据。共享原算术预算、输入版本和三次限制，不进入 PTC。300 组独立算术 oracle 现在要求全部真实范围可证，新增多变量三次内部极值、256 点边界、超限、预算失败／恢复及旧证据失效；源码 arch(x)=x*(4-x) 在 x<=4 下证明 result<=4，严格界和中间下溢拒绝。初次集成发现无益线性枚举耗尽两项契约预算，已跳过该路径，未提高预算。目录 /home/fin/luna，development/check.py --timeout 180：elaboration verification 57 项通过（/tmp/luna-tools-2s1cl4mt/result.json），luna execution/resources 1341+356 项通过（/tmp/luna-tools-i_cjvno9/result.json）。全 workspace fmt 通过（/tmp/luna-tools-au7dq8hh/result.json），git diff --check 通过；dev 未提交。大区间非单调多项式仍可能 Unknown。

2026-10-22：非单调范围回退新增单变量二次式精确顶点检查。以正分母的整数 floor/ceil 定位 -b/(2a)，仅求值区间端点和落在域内的两个邻接整数，不依赖枚举体积或浮点近似；负系数、负顶点、证明回放和原输入版本保持有效。源码 u16 的 x*(500-x) 在 x<=500 下证明机器乘法安全且结果<=62500；奇数顶点 501 对应两个峰值 62750；错误严格界、下溢及 x*(600-x) 溢出仍拒绝。目录 /home/fin/luna，development/check.py --timeout 180：elaboration verification 58 项通过（/tmp/luna-tools-cct7au21/result.json），luna execution/resources 1342+356 项通过（/tmp/luna-tools-dsf0wzqj/result.json）。全 workspace fmt（/tmp/luna-tools-flqil1dq/result.json）及 git diff --check 通过。超出已实现算法的大域三次／多变量极值仍可能 Unknown；未提交。

2026-10-22：程序范围求解支持无交叉项的多变量二次式分解，将每个变量的一元二次／线性范围与常数项相加，不枚举大区间笛卡尔积。分解遇到交叉项或三次项即退出，失败不发布局部结果，保留原变量版本和预算。新增大盒正负系数、线性／常数组合、独立笛卡尔穷举 oracle 与变量改写失效测试；源码 signed x,y 在 [-1000,1000] 内可证明 x*x+y*y+x+y>=0，严格界与错误交叉项拒绝。工作目录 /home/fin/luna，经 development/check.py --timeout 180：verification 60 项通过（/tmp/luna-tools-85kb3ss9/result.json）；execution 原有 1342 项通过，新用例初因示例格式解析失败（/tmp/luna-tools-sqf10glb/result.json），修正示例后单独通过（/tmp/luna-tools-94n0qts3/result.json）；resources 356 项通过（/tmp/luna-tools-c8hf16pg/result.json）。全 workspace fmt（/tmp/luna-tools-ovl4y1g6/result.json）及 git diff --check 通过；dev 未提交。一般带交叉项或大域三次极值仍未完全求解。

2026-10-22：组合整数有限差分、坐标单调性及残余二次式顶点求解，支持 x*(400-x)*y 等大域三次范围。差分符号不足时使用二次式／独立分量规则收紧；完成单调坐标端点替换后保留剩余二次式的内部极值。实现中特别区分连续导数与整数差分，不将整数顶点范围用来证明连续导数符号；新增 4x³-6x²+x 在 0、1 导数均正但函数下降的拒绝回归。新增大域正负缩放、消去变量版本失效和三次单调测试，源码 u32 最大 4000000000 可证，扩大输入导致溢出及错误严格界拒绝。目录 /home/fin/luna，development/check.py --timeout 180：verification 63 项通过（/tmp/luna-tools-uayn1qr0/result.json），execution/resources 1344+356 项通过（/tmp/luna-tools-zy1pq1_o/result.json）。全 workspace fmt（/tmp/luna-tools-2ff5yfca/result.json）及 git diff --check 通过；dev 未提交。算法仍不完全，未证明一般三次规格可决定。

2026-10-22：程序二次式范围增加对角占优平方和证据：无一次／三次项时，以精确整数系数检查每行 2*a_i>=sum(abs(b_ij))，证明非恒定部分非负；对相反数同样检查以证明上界。未改变一般二次型的 Unknown 边界，不调用 PTC。新增三变量平方和与常数偏移、大域正负界、变量改写失效及缺失对角／过强交叉项反例；源码平方和布尔契约正例、严格界／符号改变／中间溢出拒绝。目录 /home/fin/luna，经 development/check.py --timeout 180：verification 65 项通过（/tmp/luna-tools-b137xllh/result.json），execution/resources 1345+356 项通过（/tmp/luna-tools-zkgdn0zg/result.json）。全 workspace fmt（/tmp/luna-tools-5wmast8b/result.json）及 git diff --check 通过；dev 未提交。该条件只是充分判据，不声称所有半正定二次型可证。

2026-10-22：核查此前契约示例解析失败，确认 grammar/index.md 的中缀双侧空白规则：x*x 应拒绝，换行子句本身合法。Pratt 解析器对紧贴的 +、-、*、/ 给出运算符位置的明确空白诊断，保留右花括号后的新语句边界。新增空格／制表／换行正例、单侧或双侧缺空白拒绝，以及指针类型、解引用、泛型尖括号、一元负号与 brace 后新语句回归；没有放宽语言语义。目录 /home/fin/luna，经 development/check.py --timeout 180：parse 全组 73 项通过（/tmp/luna-tools-gfqbtmr5/result.json），补充边界后 trait_clauses 5 项通过（/tmp/luna-tools-svm2s_nx/result.json），luna execution 1345 项通过（/tmp/luna-tools-y6g3c8ow/result.json）。全 workspace fmt（/tmp/luna-tools-6evi64xu/result.json）及 git diff --check 通过；dev 未提交。

2026-10-22：新增 verification/quadratic.rs，用增广对称整数矩阵表示 2*p，通过正主元的 a*B-v*vᵀ 消元建立半正定／半负定证据，覆盖加权、平移、奇异平方和；不使用浮点或除法，矩阵最多16坐标且系数增长消耗共享预算。负主元和零主元非零剩余行只令证据失败，不发布反命题。新增加权仿射平方和、零常数／奇异主元、负值扰动、预算失败恢复与版本失效测试，以及源码正例、严格界／符号改变／中间溢出拒绝。集成发现上一轮中缀空白诊断误拦 #relay *bool，已让其尊重 stop_at_type_qualifier，并补效应／错误限定指针结果的解析回归。目录 /home/fin/luna，经 development/check.py --timeout 180：verification67（/tmp/luna-tools-uyy716l7/result.json）、parse74（/tmp/luna-tools-31mi0jwd/result.json）、execution1346/resources356（/tmp/luna-tools-kbw50bpz/result.json）全部通过；全 workspace fmt（/tmp/luna-tools-njz58lqv/result.json）及 git diff --check 通过。初次集成的资源两项解析回归（/tmp/luna-tools-49uvp47j/result.json）已修复，未提高预算；dev 未提交。该算法不覆盖一般三次规格，也不将证据失败解释为命题为假。

2026-10-22：精确半正定消元加入前一正主元整除，控制未归一化 Schur 更新的指数系数膨胀。16维 I+u*uᵀ 回归修复前耗尽默认预算（/tmp/luna-tools-_1aopr20/result.json），修复后原预算通过；每次整除检查余数，非零则停止该证明路径，不截断系数。独立验证遍历15625个小整数三阶对称矩阵，与全部主子式条件比较；另检查2/4/8/16维Gram矩阵的秩亏、零行、符号、负对角反例和预算失败后恢复。目录 /home/fin/luna，经 development/check.py --timeout 180：矩阵测试3项（/tmp/luna-tools-8uen1p47/result.json）、verification67（/tmp/luna-tools-ah1b4kpv/result.json）、execution1346/resources356（/tmp/luna-tools-d5m2nuej/result.json）全部通过。全 workspace fmt（/tmp/luna-tools-8f9cfg1q/result.json）及 git diff --check 通过；文档更新，dev 未提交。

2026-10-22：阶段性中间层集成：/home/fin/luna 经 development/check.py --timeout 300 -- test -p elaboration -p iir -p parse，44组共780项通过（/tmp/luna-tools-to7tdfgz/result.json）。核对 quote fn 值子句已有实现与回归，修正文档仍声称未接通的旧说明，加入实际执行的 Flag<bool> 推导／显式值／依赖默认示例；诊断分别指向未支持的子句协议和 contextual／dependent 参数，不再误称全部值泛型未实现。quote_functions 完整103项通过（/tmp/luna-tools-5jvyrpr4/result.json）；第一次调整诊断打破既有 contextual 文案锚点，拆分后恢复该诊断契约。全 workspace fmt（/tmp/luna-tools-z3b17hk5/result.json）及 git diff --check 通过，dev 未提交。本次是集成验证与能力说明纠错，不是新实现 quote 值泛型。

2026-10-22：半正定证据接入目标范围 [L,U]，分别检查 p-L 与 U-p；矩阵常数项从原值独立恢复，禁止上下界平移串扰。普通精确盒范围与单项／仿射路径跳过无收益检查。新增平方和加7、100减平方、双边不可能目标、旧版本失效，以及源码 i32最小值加平方／最大值减平方的安全执行；反向算术和中间平方溢出拒绝。目录 /home/fin/luna，经 development/check.py --timeout 180：verification68（/tmp/luna-tools-_j2l15ym/result.json），execution1347/resources356（/tmp/luna-tools-kb69i6z3/result.json）通过。文档加入已执行的机器边界示例；全 workspace fmt（/tmp/luna-tools-kxx5h4oy/result.json）及 git diff --check 通过；dev 未提交。仍沿用矩阵大小和算术预算，不声称一般程序规格完全可判定。

2026-10-22：新增 verification/cubic.rs，单变量三次式通过二次有限差分的顶点分段和精确整数二分定位符号变化，检查原区间端点与极值候选，不枚举大域、不使用浮点根；候选步进限制在 [lo,hi-1]，共享预算、原版本和replay保留。144组大域系数／区间组合与独立穷举极值一致，错误紧界拒绝；内部最大值、预算失败恢复及版本失效回归通过。原枚举超限回归改为仍需多变量推理的三次式，新增单变量正例，不再把已可证明的旧能力缺口当拒绝要求。源码u32的x*x*(3000-x)最大4000000000安全，严格界、减法下溢和更大域溢出拒绝。目录 /home/fin/luna，经 development/check.py --timeout 180：verification70（/tmp/luna-tools-s6rvi0kz/result.json）、execution1348/resources356（/tmp/luna-tools-n56fnis4/result.json）通过；全workspace fmt（/tmp/luna-tools-_drzfaz4/result.json）及git diff --check通过。文档同步，dev未提交；一般多变量三次规格仍不完全。

2026-10-22：独立分量范围组合扩展到三次多项式；每个变量分量复用单变量三次／二次求解，常数与线性分量合并，不递归枚举多维盒。共享预算及全部变量版本保留，混合乘积项仍拒绝该分解。新增正负大域三次分量加线性／常数项、错误紧界、任一输入改写失效及独立笛卡尔穷举核验。源码 x²(300-x)+y²(300-y)<=8000000 的布尔后置条件通过，严格界、减法下溢及大域溢出拒绝。枚举超限回归保留真正混合的 x*y² 项，避免将已可解的独立分量固化为失败预期。目录 /home/fin/luna，经 development/check.py --timeout 180：verification72（/tmp/luna-tools-it8_lmf9/result.json）、execution1349/resources356（/tmp/luna-tools-4gulelky/result.json）通过。全workspace fmt（/tmp/luna-tools-253exeaq/result.json）及git diff --check通过；文档同步，dev未提交。一般混合多变量三次规格仍不完全。

2026-10-22：近期解析诊断与程序规格求解改动后的 CMTT／inline 集成核验。目录 /home/fin/luna，python3 -B development/check.py --timeout 300 -- test -p luna --test cmtt --test cmtt_methods --test inline_control 全部通过（/tmp/luna-tools-oui1ajob/result.json）；覆盖代码／模式生成、引用方法与上下文、未选 inline 分支、嵌套阶段、元组枚举和资源传递。入口按授权自动清理 luna 可重建产物912.4MiB，未触及源码或 registry。空 tuple_enumerate 在 runtime/open/generator 路径仍明确拒绝，不能以非空异构元组回归通过宣称其完成；本次是集成验证，未新增这项能力，未提交。

2026-10-22：程序predicate在多项式输入缺少有限区间时可使用全局二次半正定证据。先验证原输入版本再归一化，不为无界Integer补造区间；严格／非严格／相等／不等比较使用独立零或±1界，证据失败仍Unknown，预算错误保留。矩阵形成提取为共享入口，范围证明路径保持原契约。新增无界仿射平方、严格正负式、等号边界、输入改写和预算恢复；源码使用comptime fn及luna::invocation验证，未放宽Integer的comptime-only或跨阶段Splice要求。目录 /home/fin/luna，经 development/check.py --timeout 180：verification74（/tmp/luna-tools-hr51ss7e/result.json）；execution原1349项通过，新fixture最初错误使用runtime Integer后又遗漏Splice，改为正确comptime invocation后新用例通过（/tmp/luna-tools-7o5aq7nx/result.json）；resources356通过（/tmp/luna-tools-_ti8i4zl/result.json）。全workspace格式检查（/tmp/luna-tools-lq90fjb8/result.json）及git diff --check通过；文档同步，dev未提交。

2026-10-22：无界一元二次predicate增加精确整数顶点证据，区分整数格点与实数半正定范围；按曲率只发布全局最小／最大值足以证明的比较，不将极值未排除零当作无根证据。新增330组系数、六种比较和两种真假目标的独立格点核验；源码comptime Integer可证明x*(x+1)>=0及2*x*x+x+1>0，等号处错误严格界拒绝，正式invocation验证大数输入。目录 /home/fin/luna，经development/check.py --timeout180：verification75（/tmp/luna-tools-zablix1j/result.json）、execution1351/resources356（/tmp/luna-tools-3ji6ao1r/result.json）通过。文档、全workspace fmt（/tmp/luna-tools-r_wh6sct/result.json）及git diff --check完成，dev未提交。一般整数无根判定和多变量整数格点优化仍不完全。

2026-10-22：修复 program-facts dump 将失效保留条件仍标为 established 的误导。版本检查通过／失效／有界检查未完成分别显示 established、stale-condition、unchecked-condition，保留条件与旧版本供排查，不改变 Facts 或求解结果。新增当前→改写→失效 dump 回归，验证稳定顺序、括号结构、旧／新版本可见、旧proof回放拒绝及格式化不恢复事实。目录 /home/fin/luna，development/check.py --timeout180 -- test -p elaboration --test verification 共76项通过（/tmp/luna-tools-j7n0t8hk/result.json）；全workspace fmt（/tmp/luna-tools-x1sf87ak/result.json）和git diff --check通过。文档同步，dev未提交；本次只修改可观察性，不新增语言语义。

2026-10-22：程序验证增加整数系数余数非零证据：非恒定系数gcd不整除常数时，证明三次以内多元式非零／等式恒假；使用精确Integer和共享预算，保留归一化前输入版本检查及证明回放。源码comptime三次契约、带符号系数、有／无界输入、预算恢复、旧版本失效与小整数反例核验通过。显式DivideTrunc/RemainderTrunc可复用证据验证100除以2*x+1；去掉偏移后拒绝。首次源码用例暴露已有signed /舍入协议未接入，改用现有显式IR验证且文档明确限制，未默认选取源码舍入策略。目录/home/fin/luna：development/check.py验证verification78（/tmp/luna-tools-8v9_o1rl/result.json），execution1353/resources356（/tmp/luna-tools-2r0lbzfb/result.json）；全workspace fmt（/tmp/luna-tools-uyovokrj/result.json）及git diff --check通过。dev未提交；一般整数无根判断仍不完全。

2026-10-22：整数非零证据扩展到有限模剩余类，gcd不足时检查模2/3/5的全部赋值；最多8变量、每模256赋值，找到模根只跳过当前证据，部分枚举及预算失败不证明无根。负系数用规范非负余数，精确Integer降到小模数后才使用有界宿主算术。支持x³-x+1与x²+x+y²+y-1等gcd为1的多元／三次非零规格。625组系数独立具体根／模根核验、预算恢复、输入改写失效及源码comptime调用覆盖。目录/home/fin/luna，development/check.py：verification80（/tmp/luna-tools-gqht1g8h/result.json）、execution1354/resources356（/tmp/luna-tools-wfehgdap/result.json）通过；全workspace fmt（/tmp/luna-tools-5xn85d6y/result.json）、git diff --check通过。正式trait文档确认signed /应由绑定协议选择，未默认指定舍入；一般整数无根判断仍不完全。dev未提交。

2026-10-22：重新核对实现基线，修正开头仍停留在早期NIR阶段的总述以及多包、函数、资源、效应和IIR整组Pending标记；依照当前入口和对应测试描述已连接路径，不宣称子系统完成。核实packages显式依赖输入、独立资源检查执行入口及IIR staging/cleanup/effects模块。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test packages --test modules --test frontend_cli：CLI144、modules56、packages41通过（/tmp/luna-tools-ijh9_1iq/result.json），git diff --check通过。本轮未新增语言能力；普通for与空tuple_enumerate检查仍为具体未完成边界，原生生成不能由解释器证据替代。dev未提交。

2026-10-22：非零证据加入素数幂模4/8/9，覆盖在模2/3/5均有根的x²-6、x²-21、x²-24；共享有界剩余类枚举，不改整数源操作语义。新增源码大数与负数调用、平方偏移反例，以及8变量256赋值完整证明／9变量策略上限不发布证据回归。目录/home/fin/luna，development/check.py：verification81（/tmp/luna-tools-nkof9rac/result.json）、execution1355/resources356（/tmp/luna-tools-d5eongin/result.json）通过；全workspace fmt（/tmp/luna-tools-rwkx0r7e/result.json）、git diff --check通过。普通for仍受正式IntoIterator/Iter表述、库Iterator::next与作者inline-effect方向的接口未统一限制，未擅自写死其中一种；本轮推进独立程序验证能力。dev未提交。

2026-10-22：首次本轮实际sysroot冒烟：/tmp/luna-library-smoke.fl仅包含fn run()->bool {true}，经development/check.py --timeout180 -- run -p luna -- /tmp/luna-library-smoke.fl --sysroot /home/fin/luna/library --emit nir --output /tmp/luna-library-smoke.lisp退出1（/tmp/luna-tools-0dymr9w7/result.json）。基线停在HIR名称解析：Option/Result的panic、traits/fmt/io的String、fmt的Any，以及collections/io/fmt的__builtin_*、__intrinsic_map_storage、todo均无定义。未用占位函数绕过，也未把隔离模块通过报告为完整sysroot通过。
修复真实language-items.conf遗漏optional-type=builtin::option::Option；加载真实manifest的测试保护默认类型构造器/运算符绑定。新增library_protocols直接include仓库primitives.fl与ops.fl，五种二元运算检查异构Rhs和Output，另测Rhs=Self默认值及Neg/Not关联结果，共2组通过（/tmp/luna-tools-5lvgirqt/result.json）。luna --lib通过（/tmp/luna-tools-7f5976q1/result.json）。这些测试不复制库声明、不依赖虚构运行时接口；下一步库集成须逐项落地上述运行时与类型依赖。dev未提交。

2026-10-22：新增development/library_check.py，将真实库声明、真实manifest与完整sysroot解释执行串为独立有界检查，始终使用cargo.py缓存入口；各阶段独立日志及总summary，失败后继续独立检查，任一失败总退出1。development/examples/library-smoke.fl实际使用?bool和Option.unwrap_or，sysroot成功还必须返回(bool true)。library/README.md明确当前不可用边界及复现入口，未加入占位panic/分配/IO实现。目录/home/fin/luna，python3 -B development/library_check.py --timeout180：声明与manifest通过，sysroot在既有未定义接口处失败，总退出1（/tmp/luna-library-mio7w9lh/summary.json）；非法非有限timeout已验证拒绝，git diff --check通过。panic终止/清理契约已向作者异步询问，等待期间不依赖该决定继续工作；整个语言目标不因此阻塞。dev未提交。

2026-10-22：修复循环标签出现在值表达式中导致AST lowering unreachable!崩溃。修复前回归明确复现panic（/tmp/luna-tools-8l0yna90/result.json）；现在显式处理LoopLabel并报告源码诊断，覆盖while/for/闭包引用，普通局部同名遮蔽仍可正常编译。删除与现行HIR保留for结构不符的旧降糖注释。扩大CLI检查发现上一轮新增optional manifest绑定使临时List/Tuple sysroot fixture缺少声明，已补充明确的测试用Option构造器并验证默认?bool绑定，以及错误Tuple配置被显式选择覆盖时仍保留Optional绑定；完整真实库冒烟不作fixture替换。目录/home/fin/luna，development/check.py --timeout180 -- test -p luna --test iteration --test frontend_cli：144 CLI+3 iteration通过（/tmp/luna-tools-j0nkwsti/result.json）；fmt（/tmp/luna-tools-hfto9uji/result.json）、git diff --check通过。dev未提交。

2026-10-22：整数非零剩余类证据接入当前区间事实。短区间只枚举可达余数，负端点规范化；无界输入及覆盖完整周期的区间保持全余数，256上限按实际域组合计数。移除仅对无界搜索成立的“必须含常数项”快捷限制，支持受限y与无界x的x²-y²-1及x²+y²非零契约。证明继续保留范围/版本，改写输入或放宽y包含0不复用旧结论。独立内核对1950组有符号区间/模数与逐整数枚举对照通过（/tmp/luna-tools-ahah0cxu/result.json）；最终verification82（/tmp/luna-tools-81funmoa/result.json）、execution1356/resources356（/tmp/luna-tools-4b5sl996/result.json）通过。目录/home/fin/luna，均经development/check.py；fmt --check（/tmp/luna-tools-895wbbt8/result.json）、git diff --check通过。文档同步，dev未提交。

2026-10-22：接通非零因子传播与归一多项式公共变量因子提取。先检查当前版本的非零条件/区间，再对严格降低次数的剩余多项式复用既有符号、范围及余数证据；不递归证明同一多项式，不绕过机器溢出。原源码x!=0且y!=0仍不能证明x*x*y!=0的失败已复现（/tmp/luna-tools-zuzyf3ex/result.json），修复后支持该契约及x*y*y+x；缺少因子前提、可为零的剩余因子拒绝。分支join丢失前提、输入改写与预算恢复回归覆盖。目录/home/fin/luna，经development/check.py：verification83（/tmp/luna-tools-cx9jjvvw/result.json）、execution1357/resources356（/tmp/luna-tools-q2o1dwow/result.json）通过；fmt（/tmp/luna-tools-5uv_74fb/result.json）、git diff --check通过。文档同步，dev未提交。

2026-10-22：补齐非零公共因子的反向蕴含：已保留的多项式非零/严格正负事实可推出每个公共变量因子非零，支持等式为假的分支和缩放后的单变量零比较；逐单项式确认公共因子，不能从x+y!=0推导x!=0。known_condition在归一化前验证原事实全部版本，目标之外的另一个因子改写、join丢失前提都使证据失效。原源码x*y!=0不能证明x/y各自非零已复现（/tmp/luna-tools-9wbqaq2f/result.json）。目录/home/fin/luna，development/check.py：verification84（/tmp/luna-tools-37icoo5d/result.json）、execution1358/resources356（/tmp/luna-tools-6wjruf2v/result.json）通过；fmt（/tmp/luna-tools-sy5la8qe/result.json）、git diff --check通过。文档同步，dev未提交。

2026-10-22：加入当前整数单项式的负/零/正有限符号域，结合已有区间和保留比较；重复因子按幂的奇偶性处理，负系数翻转、零因子归零，不为无界Integer虚构范围。支持严格/非严格/等于/不等于六种比较，不能决定时继续原有证据路径。修复前源码正乘负契约失败（/tmp/luna-tools-65sar88l/result.json）；新增196组因子域/幂/系数组合、六比较双真假目标与独立具体整数样本对照，源码覆盖严格前提放宽的拒绝。目录/home/fin/luna，development/check.py：verification85（/tmp/luna-tools-4pvm1xrw/result.json）、execution1359/resources356（/tmp/luna-tools-mm07d_13/result.json）通过；fmt（/tmp/luna-tools-dycxveve/result.json）、git diff --check通过。文档同步，dev未提交。

2026-10-22：有限符号域从单项式扩展到多项式求和，支持非负项+严格正项/对称负项契约，正负抵消时保守回退；同次只读查询缓存变量符号，不跨事实状态保存。修复前x>0不能证明x³+y²>0已复现（/tmp/luna-tools-he2xa_o6/result.json）。独立具体整数枚举核验49组符号域组合的加/乘表通过（/tmp/luna-tools-p7ydsw3b/result.json），源码覆盖混合三次项、严格前提放宽和减法反例。目录/home/fin/luna，development/check.py：verification85（/tmp/luna-tools-nrjmvvi2/result.json）、execution1360/resources356（/tmp/luna-tools-tmfq5wkw/result.json）通过；fmt（/tmp/luna-tools-5vwywfzv/result.json）、git diff --check通过。文档同步，dev未提交。

2026-10-22：新增luna --packages-v1多包进程入口，显式传入u32解析ID/u64语义身份/包名/OS路径、依赖别名、primary/builtin与HIR/NIR及源码协议选择；复用现有package frontend和原子输出发布，不添加隐式sysroot、anar配置语言或包路径身份猜测。路径经普通CLI选项解析的--边界保真，支持空格、旗标形路径和非UTF8路径。新增跨包调用、失败保持旧产物、依赖源码覆盖拒绝、协议来源名与alias区分、身份/边拒绝及builtin选择回归。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --lib --test frontend_cli --test packages --test modules：12单元+149CLI+56模块+41多包=258通过（/tmp/luna-tools-az9k2ixa/result.json）；fmt（/tmp/luna-tools-q6t8g77v/result.json）、git diff --check通过。接口文档development/package-cli.md已接入README。anar仓库仍未在本轮核查目录中找到；该入口可供构建工具调用，但不宣称anar或native backend完成。dev未提交。

2026-10-22：多包进程入口接通comptime invocation、IIR实例化、参考执行（可选inline/Copy/Drop清理）及资源分析，复用普通CLI互斥/预算规则。入口使用primary对应语义身份，不能误取依赖同名函数。回归发现资源执行层此前仅按叶名重新查找协议；新增内部Name/Identity选择，package frontend返回本次解析的协议身份，执行/分析沿用身份，不把限定路径降成叶名。覆盖重复协议名、嵌套模块限定路径、依赖alias拒绝、输出一致性、失败/低预算保留旧输出及依赖源码覆盖拒绝。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --lib --test frontend_cli --test packages --test execution --test resources：12+150+41+1360+356=1919通过（/tmp/luna-tools-p3nizv8b/result.json）。fmt通过（/tmp/luna-tools-3x1mtth9/result.json），git diff --check通过。development/package-cli.md与CLI帮助同步；仍为参考执行，不宣称native backend或anar实现完成。dev未提交。

2026-10-22：单文件/sysroot资源动作保留前端解析后的Copy/Drop身份，与多包入口共用Action执行路径；新增bound frontend内部结果，既有返回Artifact的调用保持兼容。修复限定路径在执行时按叶名重查的断点，并保持命令动作范围：sysroot manifest的Drop默认绑定不自动使仅Copy分析转为清理分析。新CLI回归覆盖根/嵌套同名协议、实际Ticket析构计划、builtin/std同名协议选择、显式选择覆盖manifest、inline路径与无Drop分析。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --lib --test frontend_cli --test modules --test packages：12+152+56+41=261通过（/tmp/luna-tools-6k2r9j2p/result.json）；fmt通过（/tmp/luna-tools-nn_g2s4r/result.json），git diff --check通过。接口文档同步；dev未提交。

2026-10-22：统一source selector阶段校验，修复七种算术协议在HIR/AST导出时被静默忽略，以及直接传入Copy/Drop/ListPatternView等selector绕过Options检查的问题。单文件CLI、多包CLI、frontend API与package API共用检查；package API明确拒绝AST，避免隐式输出NIR。新增13种CLI绑定×两入口×默认/HIR/AST拒绝与NIR实际解析失败、旧输出保护，并验证API在文件读取前拒绝不适用阶段。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --lib --test frontend_cli --test packages：13+153+41=207通过（/tmp/luna-tools-tjyug1n6/result.json）；fmt（/tmp/luna-tools-62hmp7i2/result.json）与git diff --check通过。接口文档同步，dev未提交。

2026-10-22：接通--list-pattern-protocol，支持单文件与packages-v1的NIR导出、实例化和参考执行，并可覆盖sysroot manifest已有默认。Options及现有消费者同步，绑定沿用源码包名/模块路径解析与阶段检查，不按trait拼写推断权限。新增可运行development/examples/list-patterns.fl与CLI回归：实际两元素匹配返回true、跨包绑定、错误alias/缺失/非trait拒绝、重复/缺值拒绝及manifest覆盖；阶段矩阵加入该选项。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --lib --test frontend_cli --test list_patterns：13+155+17=185通过（/tmp/luna-tools-nm6ccn90/result.json）；check -p luna --tests通过（/tmp/luna-tools-ldg8g3zr/result.json），检查所有测试目标编译而未全量执行；fmt（/tmp/luna-tools-7i4lp1bh/result.json）、git diff --check通过。dev未提交。

2026-10-22：新增CLI列表模式与资源检查组合验收，覆盖普通/quote fn提取器×可选inline展开，使用限定身份Copy/Drop绑定。每组正常列表匹配返回true；将提取器改为写观察对象后，检查拒绝且保留先前结果，验证CLI接线未绕过observation约束。本轮未修改语言实现，只增加此前API级覆盖之外的真实进程组合回归与接口说明。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test frontend_cli list_pattern_cli_checks：1组8次进程行为通过（/tmp/luna-tools-cpvbjt3m/result.json）；fmt（/tmp/luna-tools-llwi0d3m/result.json）与git diff --check通过。dev未提交。

2026-10-22：补充ListPatternView协议/长度边界回归。u8长度域下0/1/255项模式可编译，256项必须拒绝而非截断；无任何列表模式调用时，错误Element类型、长度返回类型、元素返回类型及comptime长度方法仍被拒绝。协议拒绝用例移除impl，排除实现签名不匹配掩盖绑定检查；非Type的Element在声明类型检查阶段即诊断，其余由ListPatternView检查拒绝。本轮新增测试，未修改实现。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test list_patterns：19通过（/tmp/luna-tools-jd38_7mv/result.json）；fmt（/tmp/luna-tools-idjx4gl7/result.json）与git diff --check通过。dev未提交。

2026-10-22：实现通配符取反的表示无关简化。在一般negated_pattern_domain检查前识别仅由not链与_组成的模式，按奇偶性生成空备选/通配符；不读取值、不引入绑定，不放宽其他取反模式的表示约束。新增泛型/指针主语、双重取反及带创建副作用与真实Drop的拥有型输入回归，分别观察创建/清理状态，验证恒定模式结果不抹除主语计算或清理。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test normal_patterns --test list_patterns --test float_ranges：34+19+16=69通过（/tmp/luna-tools-sul5ep9g/result.json）；fmt（/tmp/luna-tools-wcrf26nr/result.json）与git diff --check通过。正式模式文档同步。dev未提交。

2026-10-22：将取反模式的纯覆盖简化移至一般表示域检查之前，先完整检查内部模式并恢复locals，再化简无执行行为的候选。替代上一轮仅识别not/_语法链的特判，支持泛型not captured及其嵌套取反，保持内部绑定不导出；含guard/提取调用仍走原表示检查和正常执行。修复前泛型用例明确失败于negated pattern domain（/tmp/luna-tools-u5qoaiea/result.json）；修复后绑定作用域拒绝及既有副作用/清理回归通过。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test normal_patterns --test list_patterns --test float_ranges --test execution --test resources：35+19+16+1360+356=1786通过（/tmp/luna-tools-048jvvwv/result.json）；fmt（/tmp/luna-tools-lncyssox/result.json）、git diff --check通过。模式文档同步，dev未提交。

2026-10-22：解除字符串取反的前端表示域误拒绝，沿用既有字符串字面量匹配及not候选执行。修复前含str别名的正确源码在negated pattern domain失败（/tmp/luna-tools-z9bp825y/result.json）。新增comptime与参考执行覆盖匹配/不匹配、空串、Unicode及or组合。受检执行暴露既有str内存布局缺失（/tmp/luna-tools-4dy3lt5g/result.json）；保留拒绝并显式回归，不把Str加入intrinsic_pointer_value假定其布局/Copy权限，不宣称安全执行已接通。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test normal_patterns --test float_ranges：36+16=52通过（/tmp/luna-tools-shorhtvi/result.json）；fmt（/tmp/luna-tools-mhquuxq2/result.json）与git diff --check通过。正式模式文档注明限制，dev未提交。

2026-10-22：核对正式moves-and-initialization.md的TextPacket/take_view与drop-elaboration.md的PrintOnDrop，确认str是借用字符分配的视图，不能按无指针标量处理。将owned_fields对Str的误导性callable-fields错误改为明确的string-view memory provenance未实现及backing character storage有效性要求。新增直接str、记录字段、元组字段三种输入，即使显式Copy仍拒绝忘记借用来源；既有字符串取反测试更新诊断。尚需在输入/结果指针来源、表示运输和解释器视图投影之间完整接线，不能仅给intrinsic_pointer_value添加Str。目录/home/fin/luna，development/check.py：resources borrowed_string_views（/tmp/luna-tools-fnki5npw/result.json）、normal_patterns negated_string（/tmp/luna-tools-mlb57x7l/result.json）均通过；fmt（/tmp/luna-tools-yvg2ky_v/result.json）与git diff --check通过。文档说明修正为借用来源缺失，dev未提交。

2026-10-22：检查借用字符串接线时发现模式字面量比较绕过按内容大小计费：普通String/Symbol相等已计费，但match_literal对String/Symbol/BitVec仅执行宿主比较。现统一在扫描前从解释器共享预算扣除文本字节/bit words工作；类型不匹配保持快速false，不混淆Symbol与String。新增预算不足、Unicode长共同前缀相等/不等、空串、BitVec宽度不匹配回归。此修改不提供str借用来源支持。目录/home/fin/luna，development/check.py：comptime_interpretor --lib --test scalar_equality共71+5通过（/tmp/luna-tools-i356zpqu/result.json），luna normal_patterns 36通过（/tmp/luna-tools-i53i4b3_/result.json）；fmt（/tmp/luna-tools-xul0920p/result.json）与git diff --check通过。dev未提交。

2026-10-22：核验普通BitEqual已按words计费；另修复String/Symbol字面量执行每次Arc分配未按字节消耗预算的问题，在分配前从共享预算扣除长度工作，保留源码错误位置。新增同一IR节点重复求值：首次600字节文本成功、第二次超过剩余额度并返回StepsExhausted及原源码位置，覆盖String/Symbol。目录/home/fin/luna，development/check.py：comptime_interpretor --lib --test scalar_equality 72+5通过（/tmp/luna-tools-n579cn8s/result.json）；luna normal_patterns/execution 36+1360通过（/tmp/luna-tools-70fj_ymx/result.json）；fmt（/tmp/luna-tools-mkolqinu/result.json）与git diff --check通过。dev未提交。

2026-10-22：接通普通源语言test声明到参考测试入口。resolver为每个声明分配独立def与不可拼写内部名称（含文件/node身份及标签）；标签不占普通值命名空间，重复标签可共存。HIR复用无参void runtime函数并以is_test明确标记，Display同步；NIR维护并dump test-entry身份。luna --test及packages-v1 --test选择主包测试，逐个新解释器执行，返回S-expression passed/count报告；首个失败带标签/身份，旧输出保持。复用既有fuel、显式Copy/Drop及inline执行路径；块内未扫描test明确诊断，不分配冲突fallback身份。覆盖命名/匿名/模块、同名函数隔离、重复标签不可调用、依赖测试排除、无测试、Drop失败、无限循环预算及CLI互斥。更新旧“test不支持”回归为错误test正文拒绝；未调整语言预期以迁就实现。目录/home/fin/luna，development/check.py：hir/resolve 26通过（另3个原有ignored文档示例，/tmp/luna-tools-p4b52jhy/result.json）；luna --lib/frontend_cli/modules/packages 13+160+56+41=270通过（/tmp/luna-tools-0hplr6f9/result.json）；fmt（/tmp/luna-tools-l7nh0ecx/result.json）、git diff --check通过。development/source-tests.md说明命令与边界。template test、过滤、并行调度及native测试仍未实现。dev未提交。

2026-10-22：普通源测试增加结构化TestReport/TestResult与S-expression Display，run_report保留逐测试成功/失败证据；CLI适配器失败时输出完整报告到错误通道，状态1且保留原输出。谓词拒绝和单测试预算耗尽后继续其余测试，每项独立解释器/预算；主包过滤与重复标签身份保持。测试metadata和driver配置在执行前校验，拒绝重复/缺失/歧义声明以及无协议inline。新增混合报告、后续成功、双失败、预算隔离、错误配置/metadata回归。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test source_tests --test frontend_cli：2+161=163通过（/tmp/luna-tools-w_8r7hmb/result.json）；fmt（/tmp/luna-tools-bw_zxfkz/result.json）与git diff --check通过。development/source-tests.md同步，dev未提交。

2026-10-22：新增tests/language/main.fl的20项普通Flurry源测试，明确声明Byte与Copy/Drop协议，不依赖未完成sysroot。覆盖泛型、产品/枚举模式、or/not/guard、let-else、闭包、默认输入、trait调用、comptime分支、quote调用、短路、指针写入、循环、拓展变参与可观察Drop。CLI集成测试在受检执行与可选inline两模式各20成功，并故意改错一个默认参数谓词，验证仍执行全部20项、准确19通过/1失败。修正初稿误用未实现数值转换与inline let，改为明确类型局部和正式inline if调用。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test frontend_cli repository_language_suite通过（/tmp/luna-tools-5gy8dpf4/result.json）；文档中的实际run命令执行tests/language/main.fl并报告count20/failures0（/tmp/luna-tools-hbp79uzf/result.json）；fmt（/tmp/luna-tools-p2_ck37l/result.json）与git diff --check通过。tests/language/README.md及source-tests文档互链。dev未提交。

2026-10-22：新增--test-filter标签子串筛选，单文件/packages-v1与结构化run_report_filtered API一致；保持旧API默认全选。区分大小写，匿名名称anonymous，空串全选，无匹配明确count0；报告附filtered-out计数，仅计主包内排除项。筛选发生在源码/metadata检查之后，不能隐藏未选中测试的名称或类型错误；执行失败与退出码规则保持。修复实现过程中通用option_value拒绝合法空filter、动作依赖校验落入单个分支的问题，最终覆盖多种选项顺序/缺值/重复/无--test拒绝。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test source_tests --test frontend_cli：2+163=165通过（/tmp/luna-tools-whf3rs3c/result.json）；fmt --check（/tmp/luna-tools-27127i13/result.json）与git diff --check通过。开发接口及语言测试README同步，dev未提交。

2026-10-22：补充普通源测试的跨文件/嵌套目录/嵌套显式包集成回归。四文件同名shared测试、同名ready函数各按本模块解析（包含返回相反结果的私有函数）；重复运行报告一致，改变一个模块谓词准确得到1失败/3成功。加入嵌套dependency/lib.fl显式包后，主包仍仅4测试；切换primary到依赖只执行其1项失败测试，验证源码发现边界与测试语义身份不会合并。本轮新增进程级回归，未修改编译器实现。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test frontend_cli source_tests_discover_files：通过（/tmp/luna-tools-0hjyi44p/result.json）；fmt（/tmp/luna-tools-sph3udxn/result.json）与git diff --check通过。dev未提交。

Source test execution now shares immutable checked package preparation (Copy/Drop
context and optional inlining) across selected entries, uses declaration identities,
and indexes discovery once. Instance graphs, cleanup schedules and interpreters
remain independent with fresh per-stage budgets. Regression tests cover mixed
cleanup/assertion/budget outcomes, repeat invocation without source IR mutation,
and metadata checks before filtering. In /home/fin/luna, wrapper tests for execution,
resources and source_tests passed 1360 + 357 + 4 tests (exit 0):
/tmp/luna-tools-dp44eo4j/result.json. frontend_cli passed 164 tests in
/tmp/luna-tools-lctdp76r/stdout.log; that initial combined command exited 101 due to
two new source-test fixture errors, corrected and rerun in the passing command above.
No end-to-end complexity or measured speedup claim; per-entry acquisition remains.

2026-10-22：普通参考测试也按声明ID调用，复用不可变类型上下文和implementation索引；修复普通模式重新按名称查找而受检模式按身份调用的不一致。普通命名interpret仍保留歧义诊断，测试逐项实例化和独立预算不变。新增三个同名不同身份测试（成功/失败/成功），普通、受检、内联三模式重复运行均准确选择正文。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test source_tests --test frontend_cli --test execution：5+164+1360通过，退出0，/tmp/luna-tools-fdp9f3au/result.json。fmt通过（/tmp/luna-tools-sxx6ym7k/result.json），git diff --check通过；dev未提交。

2026-10-22：复现并修复执行入口的implementation索引缺少名义布局与声明参数域：Packet<Config{flag:true}>的泛型Copy在资源检查中错误保持deferred，证据/tmp/luna-tools-69v_ptwb/result.json退出101。新增implementation_world共享构建入口，供runtime、resources、invocation、comptime_keys、memoization使用；保持inherent与trait候选分离。新增struct/enum/newtype值索引的普通及inline受检执行，删除Copy后重复交付仍拒绝。目录/home/fin/luna，经development/check.py --timeout240 -- test -p luna --test nominal_value_parameters --test execution --test resources --lib：264+1360+357+13通过，退出0，/tmp/luna-tools-njz22so1/result.json。fmt（/tmp/luna-tools-d5pyy6xp/result.json）及git diff --check通过；语义说明同步，dev未提交。

2026-10-22：增加struct/enum/newtype值索引的泛型Drop可观察执行，两种受检执行模式均清理并写回调用者标记。继续复现默认参数P3检查遗漏符号值参数域：Flag<which>的泛型Copy误拒绝，/tmp/luna-tools-qwqre4cw/result.json退出101。purity的resource_world_parts现登记全部声明域并从共享fuel扣除收集工作；泛型默认参数可复制外部输入，缺少Copy或只有Flag<false>实现仍拒绝。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test runtime_purity --test nominal_value_parameters：63+265通过，/tmp/luna-tools-8vnq7mdw/result.json退出0。fmt（/tmp/luna-tools-9_9ko03c/result.json）和git diff --check通过，文档加入实际源码示例。dev未提交。

2026-10-22：完成名义值索引缓存入口的端到端回归：Key<Config{enabled:true}>的泛型Eq/Hash执行、固定Hash碰撞下的不同输入结果、重复调用命中；Config{enabled:false}输入拒绝、零预算调用失败后缓存仍保持两条且可再次命中。本轮新增行为验证，未新增缓存语义。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test execution memoized_nominal_value_index_keys：1通过，/tmp/luna-tools-2yp5y3ok/result.json退出0。fmt（/tmp/luna-tools-f9u9dqpj/result.json）及git diff --check通过，dev未提交。

2026-10-22：新增luna --list-tests及packages-v1同名动作，编译后输出test-inventory（身份、标签、count、filtered-out），不运行测试正文；discover/TestInventory API与执行入口共享身份/函数元数据验证和标签规则。支持标签过滤、主包选择及既有受保护输出路径；拒绝与执行/emit/inline动作混用。进程回归用assumes false和无限循环证明不执行，过滤不掩盖源码错误，重复标签保留独立身份。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test source_tests --test frontend_cli：6+165通过，/tmp/luna-tools-f1fo6ry7/result.json；末次CLI细化后test_inventory通过，/tmp/luna-tools-08djkdxm/result.json。fmt（/tmp/luna-tools-mttaz9sa/result.json）及git diff --check通过；开发接口文档同步，dev未提交。

2026-10-22：修复长平面列表模式生成线性Then链导致实例化深度失败。255项源码可elaborate却在1,000,000预算实例化报dispatch materialization budget exhausted，/tmp/luna-tools-k2gws6bl/result.json退出101。list_patterns lowering将每个别名观察步骤保留为独立Then，以无副作用Unit桥构成平衡序列，不复制主语且保持长度先行、候选绑定和短路顺序。255项完整匹配/首项失败/长度不匹配的实际提取次数255/1/0验证通过，资源检查执行也通过。目录/home/fin/luna：全list_patterns初次20项通过/tmp/luna-tools-kcxpc2nk/result.json；补充计数与受检回归后筛选list_patterns/frontend_cli，19+5通过/tmp/luna-tools-5as8ctos/result.json。fmt/tmp/luna-tools-vp3xsouy/result.json与git diff --check通过；文档同步，dev未提交。

2026-10-22：长列表平衡生成后的跨分组候选验证：255项or模式，末项guard读取首项绑定；true输入首候选末尾失败后二次完整观察，false输入首候选成功。逐项核对length/element调用序列，普通参考与受检执行结果一致。初次受检1,000,000步耗尽（/tmp/luna-tools-igq4mj76/result.json），5,000,000步通过（/tmp/luna-tools-2l4i98tx/result.json），不声称资源检查成本已优化。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test list_patterns long_list_alternatives：1通过退出0。本轮新增回归，不改变实现语义；dev未提交。

2026-10-22：优化受检解释器热路径的清理作用域查询。计划验证完成后建立具体InstanceKey到作用域引用的索引；copy、transfer、block、residual、discard与cleanup_site仅检查对应实例的上下文，不再逐次扫描全计划。索引构建、实例参数哈希工作与复制引用数量均计费；候选内的重复/上下文检查、清理动作及顺序保持。此前长列表双候选在1,000,000步耗尽，现恢复该预算并通过（/tmp/luna-tools-iaopxdd2/result.json，最终回归见下），不再依赖5,000,000步。目录/home/fin/luna，development/check.py --timeout240 -- test -p luna --test execution --test resources --test list_patterns：1361+357+21通过，/tmp/luna-tools-z2cr1p83/result.json退出0；comptime_interpretor --lib 72通过，/tmp/luna-tools-yukbq37z/result.json。fmt/tmp/luna-tools-j3qsrnzi/result.json及git diff --check通过，dev未提交。计划安装前的部分全局一致性验证仍保留线性/平方扫描，本次不声称所有清理成本均已优化。

2026-10-22：检查一般meta.List接线时复现阶段漏洞：普通fn参数List<Type>原可通过源码编译，/tmp/luna-tools-lbbb0nlh/result.json退出101（拒绝回归失败）。NIR comptime_storage现在识别可信TypeList头的所有元素域为comptime-only，并递归检查可信Tuple结构视图；不展开Opaque头，未知泛型Tuple结构仍由实例化后检查。新增直接/指针/struct/enum/newtype/回调隐藏列表拒绝、嵌套Tuple阶段检查，以及Identity<Type>保持不透明。一般值列表构造仍未实现。目录/home/fin/luna，development/check.py --timeout240：luna type_lists/execution 14+1361通过（/tmp/luna-tools-1mgi35bz/result.json）；nir phases16通过（/tmp/luna-tools-3lrgh0z5/result.json）。fmt（/tmp/luna-tools-oc5nsf6z/result.json）与git diff --check通过。文档同步，dev未提交。

2026-10-22：NIR阶段遍历补齐ErrorResult所有错误载荷及成功载荷、Computation返回载荷；效应身份不当作runtime数据字段。源码覆盖!Fault Type、携Type的Fault、指针间接隐藏Integer拒绝，同样签名在comptime函数中允许，普通!Fault bool保持合法；NIR computation的bool/Type/Integer阶段分别验证。初稿源码直接使用.Type'builtin被解析为错误标签后缀，改为明确别名后验证，未将该解析错误作为阶段漏洞证据。目录/home/fin/luna，development/check.py：error_surface117通过/tmp/luna-tools-xe98o4tu/result.json；补充comptime正例定向通过/tmp/luna-tools-3s35962f/result.json；nir phases17通过/tmp/luna-tools-quv8gt9x/result.json；execution1361通过/tmp/luna-tools-va08qlqq/result.json。fmt/tmp/luna-tools-ndg0c2ys/result.json及git diff --check通过，文档同步，dev未提交。

2026-10-22：复现泛型实例化后runtime签名未重新检查阶段：consume<Type>可生成runtime Type参数的IIR（/tmp/luna-tools-rhuv16h9/result.json拒绝测试失败）。materialization交付前对所有runtime具体签名调用存储阶段验证，使用原声明布局且不展开普通范式头。回归覆盖Type/Integer/Float直接、指针、Box<T>与错误结果输入拒绝，bool实例允许。目录/home/fin/luna，development/check.py --timeout240：execution1362通过/tmp/luna-tools-hyzs2lyv/result.json；nominal_staging/type_lists通过/tmp/luna-tools-hoz20cne/result.json；fmt/tmp/luna-tools-4h_y90px/result.json及git diff --check通过。文档同步，dev未提交。本次补齐具体签名交付边界，不宣称底层IIR独立校验器已完成所有阶段重检查。

2026-10-22：将具体runtime签名阶段检查推进至IIR独立校验器，并删除elaboration出口重复检查。NIR提供接受已解析名义布局的共享存储阶段入口；IIR使用具体nominals索引提供struct/enum/newtype实际字段，不依赖源泛型元数据。新增直接改写IIR名义字段为Type、直接改写runtime参数为Type均拒绝，Phantom<Type>实际bool字段保留合法。目录/home/fin/luna，development/check.py --timeout240：luna execution/nominal_staging/type_lists通过/tmp/luna-tools-ys5p_v4h/result.json；iir全crate通过/tmp/luna-tools-03a3h4nb/result.json，均退出0。fmt/tmp/luna-tools-tfknhdia/result.json及git diff --check通过。文档同步，dev未提交；本次范围为具体签名，非所有表达式阶段规则重演。

2026-10-22：复现IIR独立校验可接受runtime函数体内Type局部存储（/tmp/luna-tools-kz58jg89/result.json拒绝回归失败）。新增iir/storage.rs，使用有预算的pruned遍历检查runtime函数体/契约/runtime默认值，并按调用参数sort隔离comptime实参、保留两个TypeValue的类型观察；共享具体名义存储域检查，不展开普通头。目录/home/fin/luna，development/check.py --timeout240：execution1364通过/tmp/luna-tools-b1v6fvyr/result.json；iir全crate通过/tmp/luna-tools-mrqnvtzv/result.json。fmt/tmp/luna-tools-9wsv0m4z/result.json和git diff --check通过，文档同步，dev未提交。

2026-10-22：为递归存储阶段验证接入共享预算；IIR签名/函数体检查向NIR入口传递剩余fuel，具体名义解析器在克隆每个字段前计费，递归类型scope/hash/clone前计费。旧source API保持默认有界入口。新增256字段布局的小预算拒绝、充足预算恢复、零预算与缺失布局拒绝，失败不修改类型世界。目录/home/fin/luna，development/check.py --timeout240：nir phases18通过/tmp/luna-tools-zb509nca/result.json；luna execution1364/list_patterns21通过/tmp/luna-tools-9lk3g22p/result.json，含100万步长列表案例。fmt/tmp/luna-tools-obzh2534/result.json及git diff --check通过；文档同步，dev未提交。


### 编译期 ListPatternView 匹配

列表协议验证同时保留 size 域和方法执行阶段；两个方法必须同阶段，候选 lowering 要求与当前表达式阶段一致。复用现有 Observe/Then 降码、关联方法选择和 invocation，无新增模式执行器。已覆盖编译期常量、类型选择、泛型关联元素、返回 Type 的提取、长度及元素短路、空模式、预算耗尽与后续调用恢复；运行时路径与混合阶段拒绝保留。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 240 -- test -p luna --test list_patterns`，25 项通过，退出 0，报告 `/tmp/luna-tools-1zxcjo3r/result.json`；相关 comptime_memory 13 项通过，报告 `/tmp/luna-tools-vmbhfu8h/result.json`。全 workspace fmt 报告 `/tmp/luna-tools-ggoln5ek/result.json`。没有全量测试或提交。

边界：驱动仍绑定一个列表协议身份；不是同时注册 runtime/comptime 两套协议的实现。rest/endless 和拥有资源提取尚未完成。探索 Type 字段指针读取时得到 `Copy representation is not established for this value category`，没有绕过现有证据检查；Type 元素回归验证的是方法计算并返回类型。


### 显式 Type Copy 的表示证据

Copy acquire 已接受不可变 Type 描述的表示，仍要求具体 Copy 能力和平凡清理；不把 Type 新增为 intrinsic Copy，也不改变 runtime 存储检查。受需 comptime invocation 可读取、替换 Type 局部存储，列表协议可提取 Type 字段。回归覆盖 normal-form 头保留、缺少 Copy 拒绝及清理/阶段拒绝。此前类型字段的表示限制已在此路径解除；原始解释器和未安装资源证据的调用入口仍不自行取得 Copy 权限。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 240 -- test -p luna --test comptime_memory --test list_patterns` 40 项通过（`/tmp/luna-tools-hdx7r6o1/result.json`）；`-- test -p luna --test resources` 通过（`/tmp/luna-tools-9c__lnh3/result.json`）。fmt 退出 0（`/tmp/luna-tools-k8bv8b2_/result.json`），diff 检查通过。未提交。


### 显式 comptime invocation 的 Copy 资源绑定

新增 `invocation::invoke_with_protocols`，单文件及 packages-v1 的 `--invoke-comptime` 接受成对 Copy/Drop 选项。CLI 使用前端选定的声明身份，API 保留显式名称解析；Copy 表示/清理证明计入 invocation 剩余预算，并保留失败原因。未绑定不自动取得权限。验证覆盖 Type 指针传送、聚合复制/替换、缺少实现、非平凡清理、预算耗尽及后续恢复，以及两个 CLI 驱动的成功、无授权和不成对选项拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 240 -- test -p luna --test comptime_memory --test frontend_cli --lib` 193 项通过（`/tmp/luna-tools-20rkg5_j/result.json`）；添加聚合回归后 comptime_memory 15 项通过（`/tmp/luna-tools-x_fgj5cs/result.json`）。fmt 报告 `/tmp/luna-tools-x8nd7_x0/result.json`。未执行全量测试，未提交。


### 缓存 invocation 的资源授权

Memoized Invocation 的 LangItems 可绑定成对源 Copy/Drop；准备时检查身份并随 owned package 固定。未命中执行安装有预算的表示/清理证据回调，失败保留原因且不发布结果；未授权快照不共享授权快照的缓存。测试覆盖 Type 局部读取/替换、首次执行与命中、低预算失败后恢复和无授权失败不缓存。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 240 -- test -p luna --test execution memoized` 11 项通过（`/tmp/luna-tools-xqslh8gf/result.json`）；comptime_memory 16 项通过（`/tmp/luna-tools-nxmzbunw/result.json`）。fmt 退出 0（`/tmp/luna-tools-w5e98ego/result.json`），diff 检查通过；未提交。


### 源 Eq/Hash 的编译期资源权限

comptime_keys::Protocol 固定显式源 Copy/Drop 身份，在键方法执行中安装计入查询预算的 Copy 证据回调。没有改变不可变输入准入和源 Eq 判等规则。新增碰撞回归覆盖 Eq/Hash 内的局部聚合复制、缓存命中及未授权键失败不发布缓存。

`/home/fin/luna`：`python3 -B development/check.py --timeout 240 -- test -p luna --test comptime_memory` 17 项通过（`/tmp/luna-tools-kwz_j41t/result.json`）；`-- test -p luna --test execution comptime_` 通过（`/tmp/luna-tools-jo59484s/result.json`）。格式化退出 0（`/tmp/luna-tools-m930qnof/result.json`），diff 检查通过；未提交。


### 显式与缓存 invocation 的类型证明计费

解释器新增 with_budgeted_type_solver，同时用于契约检查与类型相等的正向/否定证明；剩余预算只减不增，耗尽不再次调用宿主。luna 的显式 invocation、memoization 正文及 comptime_keys 已通过公共 solve_metered 适配到 World::check_predicate_metered。旧 with_type_solver 保持兼容；受需 elaboration/type-computation 等其它宿主接线尚未在本次统一迁移，不能称全部宿主求解已统一计费。

`/home/fin/luna`：execution comptime_ 178 项通过（`/tmp/luna-tools-qak741s6/result.json`）；解释器 lib 73 项通过（`/tmp/luna-tools-yf68gb_p/result.json`）；execution memoized 11 项通过（`/tmp/luna-tools-jlt1_r2c/result.json`）。均通过 development/check.py，退出 0；fmt `/tmp/luna-tools-x5a18lq5/result.json`，diff 检查通过。未提交。


### 受需求值的类型证明计费

PreparedRequired 的类型求解器改为消耗 invocation 剩余预算，普通值解释器和 Type 结果 Session 均接入。Session 内部以 ProofSolver 区分旧回调和计费回调，公开旧 API 保持兼容。测试比较有/无非平凡契约的实际预算消耗，同时覆盖两类结果的低预算拒绝与新 invocation 恢复。不宣称 scope substitution 或所有编译器准备工作已统一计费。

`/home/fin/luna`：`development/check.py --timeout 240 -- test -p luna --test comptime_memory --test type_lists` 31 项通过（`/tmp/luna-tools-k6x29gsg/result.json`）；`-- test -p elaboration --lib` 200 项通过（`/tmp/luna-tools-ddb5ke1h/result.json`）。fmt `/tmp/luna-tools-dd00yfd7/result.json`，diff 检查通过；未提交。


### 运行时参考执行的类型证明预算与迁移回归

execution 的三个解释器入口改用 with_budgeted_type_solver，测试会话复用后自动覆盖。扩大回归发现上一轮 PreparedRequired 将单次证明额度扩大到全部 remaining，trait 预留计费使布尔泛型契约转发过快耗尽预算。现恢复默认单次上限，并取 remaining 的较小值；没有恢复独立免费预算或改变 trait 语义。

`/home/fin/luna`：首次 execution/source_tests 运行在泛型布尔 trait 转发处失败（`/tmp/luna-tools-8s5a8hft/result.json`）；修复后定向复现通过（`/tmp/luna-tools-c9q0ae6g/result.json`），完整 execution 1364 + source_tests 6 项通过（`/tmp/luna-tools-or5wvut6/result.json`）。均使用 development/check.py --timeout 240；fmt `/tmp/luna-tools-pthf0626/result.json`，diff 检查通过。未提交。


### 受检运行时指针 Copy 证明计费

CheckedSession 由独立预算 copy_delivery 回调改为 with_budgeted_pointer_copy_permission；直接取得具体 Copy/清理证据，将报告的证明消耗扣入运行时预算。错误附带 Copy 证据原因，保持同一协议和源布局世界。

`/home/fin/luna`：development/check.py --timeout 240 -- test -p luna --test execution --test resources --test list_patterns 共 1749 项通过（`/tmp/luna-tools-2yq_peew/result.json`）；补充诊断后 resources copy 定向组通过（`/tmp/luna-tools-_l0nnjpi/result.json`）。fmt `/tmp/luna-tools-p2badk6b/result.json`，diff 检查通过；未提交。


### 已有 trait 证据查找的内部共享预算

Solver::given_frames 不再为每次参数形成与类型相等证明重新分配完整 allowance；输入遍历、telescope 复制及 PTC 相等证明消耗 Solver.remaining，预算耗尽立即停止遍历。新增宽类型相等输入的低预算拒绝/恢复用例。仍发现未解决边界：Copy classify 调用的 given/resolve_instance 不向 acquire 返回完整子查询消耗，候选匹配亦有独立 allowance；先前入口计费不可解释为这部分已完全计入外层。

`/home/fin/luna`：elaboration lib 200 项（`/tmp/luna-tools-_d9krchr/result.json`）、capabilities 最终 38 项（`/tmp/luna-tools-6vky5cw9/result.json`）、luna nominal_value_parameters 265 与 resources 357 项（`/tmp/luna-tools-qyy_7rv5/result.json`）通过。均使用 development/check.py --timeout 240；fmt `/tmp/luna-tools-w3j3f6ss/result.json`，diff 检查通过；未提交。


### Copy 分类回传能力求解器计数

新增 World::given_metered 与 resolve_instance_metered，在成功/失败后均回传 Solver.remaining。Copy acquire 改用 classify_with_fuel，共享输入检查、已有证据查找及能力解析报告的消耗，不再丢弃这些计数。旧接口委托至新入口。仍未完成：implementation/applicability.rs::resolve_assessed_with 的候选匹配子预算完整回传，不能宣称所有嵌套证明已精确计费。

`/home/fin/luna`：capabilities 最终 39 项（`/tmp/luna-tools-u7zvf5ok/result.json`）、verification 85 项（`/tmp/luna-tools-kcyrpahy/result.json`）；luna comptime_memory 17、nominal_value_parameters 265、resources 357 项（`/tmp/luna-tools-mhba76xv/result.json`）通过。development/check.py --timeout 240，fmt `/tmp/luna-tools-dqf6tfez/result.json`，diff 检查通过；未提交。


### 候选搜索预算回传

Index::candidates_metered 直接更新搜索剩余计数，键构造和扫描失败也保留消耗；resolve_assessed_with_search_budget 将搜索消耗交还能力 Solver，同时保持原候选 allowance 分配。新增宽类型无候选搜索的耗尽与成功计费回归。这里覆盖候选筛选，不等于 HeadMatch 内部和所有条件证明的完整计费。

`/home/fin/luna`：capabilities 40、implementation 67 项通过（`/tmp/luna-tools-hukai1jh/result.json`）；comptime_memory 17、nominal_value_parameters 265 项通过（`/tmp/luna-tools-9uj865p0/result.json`）。使用 development/check.py --timeout 240；fmt `/tmp/luna-tools-pcddph_g/result.json`，diff 检查通过；未提交。首轮新增测试误访问私有字段导致编译失败，已改用 declarations()，没有修改 API 可见性。


### 候选结构推断及义务推进的共享额度

matching::propose_metered、Engine::match_many_metered 在失败后返回已有消耗，失败 trial 不发布推断赋值。advance_metered 在一个额度内推进多个义务，耗尽后未处理项继续排队；旧 advance 保留兼容。HeadMatch 使用这些接口，按关系数合并原先预留的匹配/证明 slots，而非把每个义务重置为整份预算。仍不宣称整个 HeadMatch 的全部准备/克隆/子查询消耗已回传到外层。

`/home/fin/luna`：matching 14、obligations 10、implementation 67 项通过（`/tmp/luna-tools-lxrtlzle/result.json`）。首次集成发现布尔方法契约受局部额度分区错误影响（`/tmp/luna-tools-0_62gr1a/result.json`），合并原预留 slots 后定向通过（`/tmp/luna-tools-k74wwugc/result.json`），最终 execution 1364 + nominal_value_parameters 265 项通过（`/tmp/luna-tools-7kjw6us1/result.json`）。均使用 development/check.py --timeout 240；fmt `/tmp/luna-tools-i1o2a86u/result.json`，diff 检查通过；未提交。


### 结构匹配输入先计费再复制

新增 inference_budget::copy_input，迭代检查 inference syntax 和 Term 深度/规模，在复制匹配工作项前扣费。未使用的 substitution 输入只统计复制结构，不做变量查找或求值。回归覆盖宽相等输入不能靠结构快捷路径绕过小预算，以及未使用外来变量不会被计费检查强制解析。该改动不宣称 trial 状态克隆、后续 alias resolve 和全部冻结工作已统一计费。

`/home/fin/luna`：implementation 67 项通过（`/tmp/luna-tools-v45wzov7/result.json`）；matching 最终 16 项通过（`/tmp/luna-tools-1am80300/result.json`）；luna execution boolean_ 定向组通过（`/tmp/luna-tools-byd8np59/result.json`）。使用 development/check.py --timeout 240；fmt `/tmp/luna-tools-_yyfhadf/result.json`，diff 检查通过；未提交。


### 匹配中的有界别名解析

InferenceContext::resolve_with_limits 计费遍历别名、冻结已完成类型及复制未解结构，matching::resolved_shape 使用该接口。已复制/计费且完全相等的输入跳过重复解析。HeadMatch 的共享推断额度合并输入观察、别名解析、匹配与每条关系证明的既有 slots，候选总预算保持不变。新增长别名链的低预算失败、保留末端未解变量与赋值后恢复回归。trial 状态克隆和其它旧 resolve 调用者仍未统一迁移。

`/home/fin/luna`：最终 implementation 67、matching 17 项通过（`/tmp/luna-tools-p2nl3tsf/result.json`），luna execution boolean_ 30 项通过（`/tmp/luna-tools-c8zrzuz7/result.json`）。集成曾因未给输入观察/解析分配 slots 而失败（`/tmp/luna-tools-gvo9w3ll/result.json`），已修复；没有提高用户或测试预算。development/check.py --timeout 240，fmt `/tmp/luna-tools-bruaeyz0/result.json`，diff 检查通过；未提交。


### 匹配赋值的有界 occurs-check

新增 InferenceContext::assign_with_limits，匹配提议改走该入口。别名/有效 substitution 依赖遍历共享 fuel，模板在扫描参数前计费；只有检查完成后才发布赋值。sort 本身为常量时间分派，本轮不为它增加无关遍历。新增低预算失败、循环依赖拒绝及后续合法赋值恢复回归。旧 assign 调用者及整个 trial 状态复制尚未统一迁移。

`/home/fin/luna`：implementation 67、matching 18 项通过（`/tmp/luna-tools-8iuv2ita/result.json`），luna execution boolean_ 30 项通过（`/tmp/luna-tools-9ocm4elj/result.json`）。使用 development/check.py --timeout 240；fmt `/tmp/luna-tools-7mdcktsa/result.json`，diff 检查通过；未提交。


### 义务准备阶段的预算与恢复

advance_metered 使用有界依赖扫描和冻结，包括稳定公式复制及 scope 域复制计费。准备阶段 Budget 错误保持 State::Ready 并重入 ready 队列，不把可重试耗尽记录为 InvalidInference；充分预算下无需伪造证明即可继续。候选共享额度纳入既有分区中的依赖扫描/冻结 slots，候选总预算不变。

`/home/fin/luna`：最终 implementation 67、matching 18、obligations 11 项通过（`/tmp/luna-tools-38fq_bkv/result.json`）；execution boolean_ 30 项通过（`/tmp/luna-tools-4lt905be/result.json`）。新增阶段未分配 slots 时曾触发泛型方法回归（`/tmp/luna-tools-aam6oiu7/result.json`），已修复，未增加测试预算。development/check.py --timeout 240；fmt `/tmp/luna-tools-e0pbkx9g/result.json`，diff 检查通过；未提交。


### 候选推断存储共享快照

InferenceContext::trial 改为共享不可变表，首次 fresh/assign 才分离变量表；赋值后的 InferenceTerm 通过 Arc 共享，分离表不再递归复制既有类型结构。提交仍检查父身份与 revision，活动变量的赋值不会跨候选传播。预算赋值在检查成功后才分离存储。新增回归覆盖共享、写入隔离、失败不分离以及提交后兄弟候选失效。这里只优化推断存储，Engine 的义务、scope 和 PTC context 复制以及写时表复制计费仍需独立处理。

/home/fin/luna：development/check.py --timeout 240 -- test -p elaboration --lib --test inference --test matching --test obligations --test implementation 共 312 项通过（/tmp/luna-tools-4pju8zks/result.json）；源级 execution/list_patterns/nominal_value_parameters/resources 共 2014 项通过（/tmp/luna-tools-def1xyd3/result.json）。全 workspace fmt 通过（/tmp/luna-tools-vg9n7ybc/result.json），git diff --check 通过。保留 dev 工作区，未提交。


### PTC 分支上下文共享只读存储

Context 的 scope、assumptions 和三类声明权限集合改为分别共享、写时分离；fork 不再深拷贝公式，theory_context/theory_in_scope 复用全局权限而清空局部假设。Proof/Witness 公开字段继续持有独立证据，回放要求精确相同的 scope、假设和权限。新增测试验证分支写入不传播、只分离被修改组件、全局上下文不继承局部矛盾，以及旧证明不能在已改变的分支回放。假设首次写入和证明构造仍需复制，未宣称所有复制已消除或计费。

/home/fin/luna：development/check.py --timeout 240 下 elaboration implementation 67、obligations 11 通过（/tmp/luna-tools-2a7xjh_1/result.json）；新增测试后 lib 203、capabilities 40、ptc 56、ptc_integer 9、ptc_kinds 7 通过（/tmp/luna-tools-1akqh6o9/result.json）；luna execution 1364、nominal_value_parameters 265 通过（/tmp/luna-tools-c93u00bf/result.json）。全 workspace fmt（/tmp/luna-tools-6paypdka/result.json）及 git diff --check 通过。dev 未提交。


### 完整候选事务的共享快照

Engine 的 scope、义务表、ready 集合、watcher 表及每个 watcher 集合改为写时分离。义务节点与 Requirement 独立共享；重新入队用新状态替换节点，避免复制即将失效的旧证明。结合前两轮的 inference/context 共享，trial 现在不遍历这些表或公式；首次修改仍可能复制表结构，未声称写入或全部预算计费为常数成本。64 个共同依赖变量的义务回归验证父子及兄弟等待状态隔离、未修改证明共享、完成提交及 stale candidate 拒绝。

/home/fin/luna：development/check.py --timeout 240 -- test -p elaboration --lib --test obligations --test matching --test implementation：204+11+18+67 项通过（/tmp/luna-tools-2294zjoh/result.json）；luna execution/list_patterns/nominal_value_parameters：1364+28+265 项通过（/tmp/luna-tools-b6r74bt7/result.json）。全 workspace fmt（/tmp/luna-tools-k2gz1d8x/result.json）及 git diff --check 通过。dev 未提交。


### 测试模板的语法与定义 HIR

开始接通 docs/functions/templates.md 的 template test：lexer 加入已由文档确定的 template 关键字；AST 区分 TestTemplateDef（名字、子句、正文）和 TestTemplateInstance（名字、模板表达式、正文）。解析保留 contextual 接口、qualified/参数化模板表达式和块边界，拒绝缺少名字/表达式/正文。resolver 为定义扫描锚定子句，lowering 保留定义的 contextual 输入及 splice，FnModifiers::is_test_template 和 S-expression dump 明确区分普通可执行测试。通过 pending_semantics 拒绝未展开模板生成 NIR；实例 contextual 绑定仍明确诊断，不伪造普通测试。尚未实现实例 HIR/CMTT 展开、唯一正文锚定参数验证及模板测试执行，后续应复用现有 code context 与卫生性机制。

/home/fin/luna：development/check.py --timeout 240 -- test -p lex -p parse 全部通过（/tmp/luna-tools-afemfnp4/result.json），包含新增 4 组模板语法测试；luna test_templates 2 项与 source_tests 6 项通过（/tmp/luna-tools-pmh6uhta/result.json）。首轮 HIR 用例缺少普通别名 u32/void 导入而失败（/tmp/luna-tools-h1r19bmm/result.json），已为独立 fixture 显式声明别名，没有新增隐式 builtin 名称。全 workspace fmt（/tmp/luna-tools-of9j9zeo/result.json）与 git diff --check 通过。dev 未提交。


### 测试模板正文入口身份

FnSig::test_template_body 提供唯一锚定子句输入的查询；lowering 对缺失/重复入口诊断。入口依赖参数角色与 HirId，不依赖 body 这个名字；optional/variadic 不计入，Type/TraitBound 简写计入，quote 不改变路由类别。HIR dump 增加 template-body-binding，保留 contextual 参数原类型。模板实例 HIR、contextual 正文绑定和 CMTT 展开仍未实现，pending_semantics 继续阻止执行。

/home/fin/luna：development/check.py --timeout 240，luna source_tests 6 与 test_templates 4 通过（/tmp/luna-tools-xkbt8ibq/result.json）；加强 dump 断言后模板 4 项复核通过（/tmp/luna-tools-5txqxt4l/result.json）。HIR display 检查暴露上一轮 FnModifiers 新字段遗漏一个手工 fixture（/tmp/luna-tools-cj7tamjp/result.json），补齐后 13 项通过（/tmp/luna-tools-5j51a16k/result.json）。全 workspace fmt（/tmp/luna-tools-obhe0jmq/result.json）与 git diff --check 通过。dev 未提交。


### 测试模板实例 HIR 与词法上下文

resolver 声明扫描记录显式 contextual 正文接口的端口名字（不是类型证据），实例按选中的 declaration 查找，因此不依赖模板 HIR 的生成顺序。实例使用独立 ItemKind/DefKind，保留模板表达式、fresh 端口 HirId 和 BodyId；Display 输出 test-template-instance/test-template-port。端口加入本实例 rib，只遮蔽本实例中的同名全局，私有模板局部不导入，兄弟实例身份不共享。模板实例使用普通测试的不可调用合成声明名字。端口类型/substitution/CMTT 展开仍未接通；computed/aliased 接口暂无法发现端口，明确拒绝。pending_semantics 保持 NIR 执行门禁。

/home/fin/luna：development/check.py --timeout 240 -- test -p luna --test test_templates --test modules --test source_tests：8+56+6 通过（/tmp/luna-tools-fhjl6tnp/result.json）；HIR display 13 通过（/tmp/luna-tools-37p9rp6w/result.json）。覆盖前向引用、限定路径/导入、实际独立文件模块、端口遮蔽全局函数、两次实例身份、私有局部拒绝、重复端口及非模板目标拒绝。首次独立模块 fixture 误写不存在的 mod 声明形式（/tmp/luna-tools-dkjyaiq2/result.json），按正式源码发现规则改为文件模块和 use 后通过。新增节点初次编译的 DefKind 穷举和 Symbol 非 Ord 问题已修复。全 workspace fmt（/tmp/luna-tools-mnnyo64t/result.json）与 git diff --check 通过。dev 未提交。


### 测试模板实例链接原声明接口

新增 ast_lowering/test_templates.rs，在 provider 完成所有文件与模块 owner 组装后链接实例。按已解析模板 declaration 找到唯一正文参数，核对扫描端口数量/次序/名字，再保存正文参数 HirId、原声明的 contextual 类型表达式，以及每个 instance port 对应的 formal HirId。HIR Display 输出 test-template-interface 和 formal 字段。接口表达式直接引用声明侧 HIR，不按调用方同名类型重解析；这只是接口身份链接，不是端口类型检查或 CMTT 展开。pending_semantics 仍阻止执行，后续连接需复用 ExpressionCode 检查和 splice substitution。

/home/fin/luna：development/check.py --timeout 240 -- test -p luna --test test_templates --test modules --test source_tests：9+56+6 项通过（/tmp/luna-tools-8pueag3h/result.json），新增调用方/声明方同名 Shared 类型不重绑定的回归，并核对两个实例均有已链接 interface/formal 身份。HIR display 13 项通过（/tmp/luna-tools-aw6zeuda/result.json）。全 workspace fmt（/tmp/luna-tools-ezh5wk92/result.json）及 git diff --check 通过。dev 未提交。


### 测试模板正文 splice 的隐式端口身份

LoweringContext 临时保存当前测试模板正文子句的声明 DefId 与公开端口；直接 splice 该参数时，为没有显式 substitution 的端口取得当前可见 Local/FnParam/ExpandedLocal 身份，并生成现有 CodeSubstitution HIR。显式项优先；同名全局定义、缺失/后续局部不会自动供给。退出 callable lowering 恢复外层映射；普通 code 值和其它 splice 路径不获得该名字匹配行为。这是定义端词法对应，端口类型检查与实际 CMTT 展开仍待接通，pending_semantics 不变。

/home/fin/luna：development/check.py --timeout 240 -- test -p luna --test test_templates --test cmtt --test source_tests 共 11+523+6 项通过（/tmp/luna-tools-vmz0l91c/result.json）。新增隐式两端口、显式覆盖并保留另一隐式项、全局同名及后声明局部拒绝用例。首次编译使用了未直接依赖的 symbol 路径，已改用 resolve::DefId；首轮新增 dump 断言误认为 port 直接输出字符串，已按现有 Ident S-expression 修正（失败 /tmp/luna-tools-d7erelcr/result.json）。全 workspace fmt（/tmp/luna-tools-75sujc64/result.json）及 git diff --check 通过。dev 未提交。


### 模板 splice 显式项和词法边界

直接模板正文 splice 在补隐式项前校验显式 substitution：拒绝重复/未知端口以及非 Local/FnParam 身份。声明端无实例时也检查重复上下文名字。新增回归比较实际 HirId，确认嵌套块中的端口取内层而非外层，并验证退出块后不能自动取得已失效名字。字面量/计算调用原本就由 parser 的绑定标识符语法拒绝，测试分别断言解析与 lowering 诊断，没有放宽该语法。类型与展开阶段仍未接通。

/home/fin/luna：development/check.py --timeout 240 -- test -p luna --test test_templates --test cmtt --test source_tests：14+523+6 通过（/tmp/luna-tools-9ddgxcsi/result.json）。首轮把字面量拒绝误期望为 lowering 诊断，已按实际阶段修正（/tmp/luna-tools-jk9pread/result.json）。全 workspace fmt（/tmp/luna-tools-lyk2k_v_/result.json）及 git diff --check 通过。dev 未提交。


### 测试模板 CMTT 展开与受检执行

测试模板正文参数接入现有 ExpressionCode；实例在声明 contextual 接口下检查，经端口身份对应和现有受检 substitution 接入 setup，最终 freshen 整个主体并生成普通 runtime 测试函数。保留普通测试与实例的声明顺序，无实例的定义也检查。正文结果遵循接口，最终测试函数返回 void。generator_frame 隔离展开阶段状态；测试运行器沿用已有资源检查、清理及可选内联。当前只支持单正文参数、显式 contextual 接口；附加子句输入/契约、显式应用参数及计算接口明确拒绝，函数模板未完成。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates --test source_tests --test execution --test resources：20+6+1364+357 项通过，退出 0（/tmp/luna-tools-os3lo6lx/result.json）。覆盖实例初始化与失败隔离、显式端口置换、跨文件模块、bool 正文结果、无实例定义诊断以及受检清理的普通/内联两种路径。CMTT 523 项此前同轮通过（/tmp/luna-tools-7gmeuwjm/stdout.log）；该轮模板组曾因 fixture 使用保留关键字 predicate 失败，已重命名后验证。扩大回归发现旧 execution fixture 使用新关键字 template 作局部名，改为 exemplar；此前失败证据 /tmp/luna-tools-0kcivsxm/result.json。dev 工作区保留，未提交。

### 测试模板可选子句与命名输入

保留唯一锚定正文入口，接通额外 Optional 子句及 `test ... use fixture<.name=value>`：定义检查使用抽象值，实例按声明顺序求取默认值，前项实际输入供后项使用；显式命名值取代默认值且保留使用侧解析身份。Type 输入保持类型身份并实例化 contextual 正文接口。拒绝重复、未知、占用正文入口的命名项以及错误值类型、自引用/后向默认依赖。默认值检查进入 required-comptime 上下文，避免把模块常量误判为需 Splice 的 runtime 读取。变参、子句契约及非显式接口仍待实现。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates --test cmtt --test source_tests：28+523+6 项通过，退出 0（/tmp/luna-tools-z5lei02m/result.json）。8 个新增测试覆盖计算默认值、后项依赖、Type 输入与接口专化、错误路由、未实例化默认值不调用、显式覆盖不调用被替代默认值、跨模块同名常量的声明侧/调用侧身份。中间失败包括 fixture 遗漏 builtin Type 写法（/tmp/luna-tools-8er1x69m）、Symbol 不支持 BTreeMap 的 Ord（/tmp/luna-tools-zcn7yfie）以及模块常量默认值的阶段处理缺失（/tmp/luna-tools-w5jfborc），均修复后验证。dev 未提交。

### 测试模板契约、提前返回与清理

生成函数保留 ProgramContract 的 requires/ensures；沿现有程序规格路径检查可支持的无副作用谓词。类型 requires 独立分类，在抽象定义检查中作为前提，在实例化时由当前 PTC 能力证明；无法建立时拒绝，不转换成 runtime 程序谓词。可选子句临时注册值域以正确分类值相等与类型相等，检查结束恢复元数据。decreases/outcome、old 入口快照等仍待支持。

抽取现有 quote 表达式检查为共用 helper，普通 quote 不带返回目标，测试实例正文显式带生成函数的 void 返回目标；不捕获外层 loop/resume。return 退出整个测试，正常后置条件检查和受检清理继续生效。正文内嵌普通 quote 仍拒绝非局部 return。生成契约使用既有 NIR ProgramContract/Function 及 Display，不新增私有运行时契约表示。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates 最终 34 项通过（/tmp/luna-tools-bmqz4g98/result.json）；同轮 cmtt 523、source_tests 6 通过（/tmp/luna-tools-uq178d1k/result.json），execution 1364、resources 357 通过（/tmp/luna-tools-4h8v68c2/stdout.log）。新增 6 项测试覆盖前/后置条件、类型前提、无实例定义拒绝、bool 值相等分类、正常/提前出口后置条件证明、返回目标隔离及普通/内联受检清理。

中间回归证据：/tmp/luna-tools-7ybisrm6 暴露实例正文丢失返回目标，已修复；/tmp/luna-tools-50bu91jf 的 false 后置条件由现有编译期规格检查拒绝，已修正测试预期；/tmp/luna-tools-rotmv11q 的析构用例读取嵌套指针字段触发现有 pointer-slot provenance 限制，未绕过检查，改用直接持有 bool 状态的 Ticket 及 *Ticket 上下文验证本次提前退出清理。后者不代表嵌套指针来源限制已解决。dev 未提交。

### 模板类型前提复用 implementation world

提取普通声明的完成态类型前提证明为 establish_type_requirement，由普通声明和测试模板共用：先 PTC，未建立时沿既有 associated type view 与已检查 implementation world 求证，保留 NotEntailed/Unknown/IllFormed 的不同诊断。程序谓词仍不进入该路径。模板定义的抽象类型前提保留在 context；具体实例证明后不再把实际实现改写成抽象假设，避免参数化证据遮蔽实现的关联类型。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates 最终 37 项通过（/tmp/luna-tools-2gsucwka/result.json）；同轮 test_templates/cmtt/execution 组合检查中 cmtt 523、execution 1364 通过（/tmp/luna-tools-63c016c_/stdout.log）。新增 3 项覆盖多 trait 合取的正文方法调用、缺失实现拒绝、析取不能授权任一分支的方法及关联类型后续约束。中间修复了 helper 提取的括号错误（/tmp/luna-tools-49e_6lvm）、fixture 误用 type 而非 assoc（/tmp/luna-tools-63c016c_），并根据 /tmp/luna-tools-5sammduf 的真实失败移除具体实例的多余抽象假设，恢复关联类型投影。dev 未提交。

### 模板前提的依赖准备与证据隔离

抽象测试模板检查先收集正向合取 trait 原子，复用 ready_signature_capabilities 的预算、就绪重试和失败回滚机制，使关联类型前提可出现在提供投影权限的 trait 前提之前。模板检查入口保存并恢复临时 capabilities，具体实例不复用抽象证据。析取分支及缺失前提不被提升为投影权限。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates --test cmtt：39+523 项通过，退出 0（/tmp/luna-tools-icoig6zs/result.json）。新增 2 项测试覆盖分离 requires 和合取中的逆序依赖、连续实例、缺失/析取权限拒绝，以及已成功实例不能为另一关联类型实例提供证据。全 workspace fmt 通过（/tmp/luna-tools-p_rkfgid/result.json），git diff --check 通过。dev 未提交。

### 模板默认值的 trait 前提准备

可选子句默认值检查在每个输入完成后，显式启用签名前提准备的 clause-prefix 检查，按已建立值域准备正向 trait 原子；下一项默认值可以使用前项的关联类型或受约束 comptime 方法。尚未完成的输入仍受 forbidden-default 与 readiness 检查保护，具体实例依然使用实际 implementation，显式覆盖跳过默认调用。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates --test cmtt：42+523 项通过，退出 0（/tmp/luna-tools-7snfwny8/result.json）。新增 3 项验证关联类型默认值随前项覆盖改变正文接口、错误正文拒绝、后项依赖拒绝、comptime 方法默认值调用与显式覆盖、缺失抽象 trait 前提拒绝。dev 未提交。

### 测试模板 old 后置条件与普通函数共用检查

提取已有 program_postconditions_with_result 中的谓词/old 作用域/快照处理为 program_postcondition_values，普通函数保持原有调用路径，测试模板在所有类型前提建立后通过同一路径检查 ensures。不可变 comptime 子句值已完成替换，不制造 runtime 参数快照；old 作用域在成功和失败时按既有逻辑恢复，不能逃逸到 requires、setup 或实例正文。此项不引入函数模板或可变 runtime 模板参数。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates --test cmtt --test execution：44+523+1364 项通过，退出 0（/tmp/luna-tools-vjdw7fkr/result.json）。新增 2 项覆盖不可变值 old、正常/提前返回、错误 old 后置条件拒绝、无冗余快照以及三个非法作用域。dev 未提交。

### 模板 return 的接入边界

connect_code 在无当前返回目标或处于闭包捕获帧的接入点，对实际 code body 执行有预算的当前运行时作用域遍历；发现外层 Return 时明确拒绝，防止将模板测试退出静默改写成嵌套闭包退出。遍历不进入正文内部闭包的独立主体，因此内部局部 return 和不含外层 return 的闭包接入继续成立。此项修复错误接入，不代表跨闭包/quote 的非局部返回已实现。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates 最终 46 项通过（/tmp/luna-tools-wzsow_q2/result.json）；同轮 cmtt 523 项通过（/tmp/luna-tools-nusw0qmw/stdout.log）。新增 2 项覆盖跨闭包、跨 quote 拒绝，以及无外层 return/内部局部 return 的实际执行。初次正例把函数调用直接放入 assumes，被既有无副作用谓词限制拒绝，改为先绑定调用结果再检查（失败证据 /tmp/luna-tools-nusw0qmw）。dev 未提交。

### 模板能力的仓库级 CLI 验收

tests/language 新增 fixtures.fl 的公开测试模板，以及 main.fl 中 6 个实例：跨文件上下文、显式 bool/Type 输入、不可变 old 后置条件、普通/提前退出的真实析构。源套件从 20 扩展到 26 项。frontend_cli 的 repository_language_suite 用真实 CLI 覆盖普通/内联执行、模板过滤及清单、普通谓词失败、两种模式下两个析构失败，以及过滤不能跳过错误模板正文的类型检查；临时项目同时复制所需模块。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test frontend_cli repository_language_suite：1 个集成测试通过，包含上述 8 条 CLI 路径（/tmp/luna-tools-91qr8deg/result.json）。初版正向 CLI 路径也通过（/tmp/luna-tools-_gbb_mrg/result.json）。dev 未提交。

本轮检查 contextual 接口别名路径：resolve scanner、HIR port linker、code_interface 均直接依赖显式 contextual 结构，通用 TypeAlias 求值也尚未承载该接口表示。没有以调用侧重解析或只复制端口名字来假装实现别名；后续需同时完成接口类型表示、声明侧 alias 求值及词法端口发现。此项仍未实现，不因上述 CLI 验收通过而视为完成。

### 测试模板已知元组变参与显式调用输入

单个 Varadic 入口在已建立元组布局下支持位置收集、普通 literal_spreads 展开以及整包命名输入。合成 tuple 输入在声明元素类型上下文下通过 required_typed_expression 检查及求值，保留已有 Tuple 协议；收集结果注册为一个子句值，可依赖前项 Type 并供后项默认值使用。数量、元素类型、溢出及整包/位置混合供应拒绝。meta.List、计算包展开及多变参入口仍明确不支持。

计算整包测试暴露显式输入在 TestTemplateInstance owner 下调用 comptime 函数时 ensure_contract 不接受无自有子句的实例；现将其与 const 初始化一样按空自有声明契约处理，所选模板契约仍在输入完成后检查，不授予提前假设。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates 最终 49 项通过（/tmp/luna-tools-6_6fp85d/result.json）；同轮 cmtt 523 项通过（/tmp/luna-tools-y45c1ac_/stdout.log）。新增 3 项覆盖元组收集/展开/整包、Type 输入与后续默认值、溢出与错误路由、显式 comptime 调用和 runtime 调用拒绝。中间修复 HIR Tuple 子元素存储类型（/tmp/luna-tools-87haxgen）、fixture 误用 .1 选择语法（/tmp/luna-tools-r91r5ejy）以及实例调用缺失契约问题（/tmp/luna-tools-pb3zrtm8）。dev 未提交。

### 测试模板计算元组包展开

位置收集改为记录显式元素/展开输入，由带预算的 collector 左到右处理。字面量 spread 仍先用普通 literal_spreads 展开取得目标元素上下文；计算 spread 经一次 required_typed_expression 得到完整 tuple 值与布局，再逐项检查类型和总数。最终使用 tuple_value_type 构造普通 Tuple 身份并对原声明域检查，不能直接把布局相同的域贴到值上。整包命名供应仍走原有显式输入路径；meta.List 与多入口未完成。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates 最终 51 项通过（/tmp/luna-tools-q3an9o9f/result.json）；同轮 cmtt 523 项通过（/tmp/luna-tools-c1y1d0ry/stdout.log）。新增 2 项覆盖计算展开与上下文元素交错、数量/字段类型/非 tuple/runtime producer 拒绝，以及 literal 数字上下文与已完成 u16 字段不能窄化成 u8 的区别。dev 未提交。

### 测试模板 meta.List<Type> 收集

collector 区分已绑定 meta.List<Type> 和元组域。前者以 Type 期望上下文求值每个输入，组装一个 Value::TypeList，支持空收集、字面量/计算列表 spread 和整包命名输入；spread 有元素预算。元组字面量展开推迟到确认元组域之后，防止把 tuple spread 偷换为类型列表收集。未绑定同名 List、非 Type 元素、tuple/list 混用和混合整包供应拒绝。一般 meta.List<T> 值收集与多个入口仍未完成。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates 最终 54 项通过（/tmp/luna-tools-6iag9c77/result.json）；同轮 cmtt 523 项通过（/tmp/luna-tools-_mt2b9_r/stdout.log）。新增 3 项覆盖上述路由与拒绝场景、索引读取实际类型元素及 normal-form 身份。最后一项最初误用保留字 opaque 作函数名（/tmp/luna-tools-x3j19cgg），并错误期望不透明 Identity<bool> 与 bool 自动满足不等式；现检查同头相等，同时断言后者保持 TypeContractUnresolved（原失败 /tmp/luna-tools-nuk4i22d），不把未知补为负命题。dev 未提交。

### 测试模板多入口显式路由

移除对多个 Varadic 声明的整体拒绝。先根据完整命名供应集合计算未供应入口，再接受位置/展开输入：仅剩一个候选时路由，零候选报重复供应，多候选报歧义。各命名包独立按原域检查，省略的 TypeList 可为空，非空 tuple 仍按数量拒绝；不按实参类型猜测目标，不在路由失败前调用 producer。

/home/fin/luna：python3 -B development/check.py --timeout 240 -- test -p luna --test test_templates 最终 57 项通过（/tmp/luna-tools-v_1d5rbj/result.json）；同轮 cmtt 523 项通过（/tmp/luna-tools-v7j98bzf/stdout.log）。新增 3 项覆盖完整命名、唯一剩余入口、tuple/TypeList 混合与空列表、错误数量/类型、歧义和重复供应必须先于会失败的 comptime 输入调用。多个未供应入口的位置分配及一般 meta.List<T> 收集仍待实现。dev 未提交。

### 真实库语法回归与跨 package 解析状态隔离

重新运行实际 library_check，发现新关键字 template 与 library/std/fmt.fl 的参数名冲突，导致 sysroot 提前停在语法阶段。将参数改为 format_string；新增直接遍历真实 library/*.fl 的词法/文件解析测试，不重写 fixture、不引入占位运行时实现。

同时修复 VfsScanner::parse_file 使用全局 diag_ctx.has_errors() 判断当前文件的错误：Parser::parse 返回该文件的成功标志，scanner 分别检查本文件 lexical errors 和解析结果，先前包的错误不再把后续正常包标记 FileParsingFailed。CLI 回归使用四个显式 package，覆盖词法/语法失败、一个/两个坏包、正常依赖不被连带拒绝以及修复后恢复。单 package 原有首个失败返回行为未改变。

/home/fin/luna：python3 -B development/library_check.py --timeout 240 初始失败证据 /tmp/luna-library-p1jwt1ah/summary.json；修复参数名后的 /tmp/luna-library-1u9wrp3e/summary.json 中 declarations 和 manifest 通过，sysroot 仍失败于真实未实现的 panic/String/分配、集合、I/O 等名称。python3 -B development/check.py --timeout 240 -- test -p luna --test frontend_cli one_malformed_source 通过（/tmp/luna-tools-12363r6j/result.json）；frontend_recovery 与 library_protocols 共 1+3 项通过（/tmp/luna-tools-6spdjxi2/result.json）。测试调整区分了普通 CLI 的摘要诊断与显式包入口的错误列表，也保留单包扫描失败退出规则；未将这些原有行为当成新回归。dev 未提交。

### 空 Tuple 模式按布局匹配

真实 primitives/meta.List/Tuple 源定义的组合测试暴露：`()` 在解析后是 Const pattern，绑定 Tuple<> 上下文下却得到 K::Tuple([])，此前误走通用相等比较。现将已检查的空 tuple 值降为 Pattern::Tuple([])，保留 K::Unit 的原分支；无需新增 tuple equality 协议。覆盖嵌套解构、穷尽 match、comptime 解构、非空/错误类型拒绝，以及真实 meta.List<Type> 计算展开、空/非空 Tuple normal-form 身份。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test tuple_type_syntax --test tuple_examples` 退出 0，4+1 项通过，后者编译/执行 60 个迁移示例（/tmp/luna-tools-4zdz0hmt/result.json）；`test -p luna --test library_protocols repository_tuple` 两项通过（/tmp/luna-tools-cppbrzmo/result.json）。格式化与定向 git diff --check 通过。dev 未提交。

扩大 library_protocols 检查未通过：repository_unary_binding_checks_nested_execution_and_materialization_limits 栈溢出（/tmp/luna-tools-m1gapfc5/result.json）。单独带临时阶段标记复现 /tmp/luna-tools-srfi0qf3/result.json：无 Not 绑定、80 层编译及执行成功；140 层编译成功，进入 execution::interpret 后栈溢出，未返回预期深度预算错误。临时标记已移除。此失败尚未定位，不标为既有基线，也不宣称整组通过；后续应修正执行准备/复制深度预算与实际栈开销的衔接，不能靠扩大测试线程栈掩盖。

### Materialization 的一元/二元依赖遍历改用显式工作栈

阶段标记确认上节 140 层 not 崩溃发生在 Builder::expr 的依赖遍历，copy preflight、实例复制、forwarding 与 normalize 均已完成。普通一元/二元子节点现与 block/preparation 共用 heap frontier，保留先序、类型依赖入队顺序、节点 fuel 与 semantic depth；语义层数仍受既有 128 上限约束，copy 的 160 上限未缩小。复杂节点仍走专用路径，不宣称所有遍历无递归。临时阶段标记均已移除。

原 140 层用例改为检查实际 materialization 深度错误（原来的 copy Depth 断言属于旧 128 copy 上限），并在同一包内继续执行 recovery。新增 40/140 层二元 IR 测试：浅树验证真实调用顺序 run/left/right，深树明确拒绝，失败后仍可执行独立入口。深树通过扩展已检查 IR 建立，以独立验证 materializer。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test instantiation --test library_protocols --test inline_returns` 退出 0（/tmp/luna-tools-hbgowr5o/result.json）。格式化、定向 git diff --check 通过。dev 未提交。

另有尚未修复的独立前端限制：将 `fn run()->bool {left() and right()` 后直接追加 140 次 ` and true`，在 Rust 测试默认线程栈中调用 frontend::compile_in_package 即栈溢出，尚未进入 instantiate。40 次成功。原始诊断 /tmp/luna-tools-7j8p_qsm/result.json，测试名 binary_dependency_chains_preserve_calls_and_recover_from_depth_exhaustion。当前保留的测试针对生成 IR，不覆盖此源代码前端问题；后续应单独检查解析/lowering/elaboration 的左结合二元链递归。

### 左结合逻辑源码链的 elaboration 栈安全

定位上节源码 and 链失败：lowering 已成功，进入 elaboration 后在递归逻辑前缀/输入事务中栈溢出（/tmp/luna-tools-_aj4odyn/result.json）。logical_expression 现收集左侧 and/or 前缀，最内层仍使用原有 checking_attempt 事务；得到完整前缀后逐层调用共用 logical_expression_step。不能完成的输入回滚并沿用原 peer inference，不改 Bool 的右侧类型前提与短路规则，也不改 BitVec 的双侧求值。未扩大线程栈或类型条件预算。

140 项 and/or、显式返回上下文及无期望类型的 let 初始化现在均返回 `type condition depth exhausted`，不再进程崩溃；并不宣称该深度的类型条件已获支持。40 项实际执行验证短路分支跳过 right、非短路分支恰好调用一次，并保留 first/right 顺序。临时 frontend 阶段标记已移除。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test instantiation` 55 项通过（/tmp/luna-tools-ofy9ehh9/result.json）；`test -p luna --test execution boolean` 30 项通过，含 BitVec 双侧求值、类型布尔前提和模式条件（/tmp/luna-tools-km176a5t/result.json）。格式化与定向 git diff --check 通过。dev 未提交。

### 类型命题分类不因深度退回程序谓词

extracted_refinement/is_type_assumption 共用显式 HIR 工作栈分类，同时记录是否存在类型观察。移除 observes_type 的 128 层 false 截断，避免过深的纯类型假设被识别成程序 assumes。纯布尔常量仍不构成类型观察，任意运行时操作数使表达式保持程序语法；遍历保持左到右与首个不支持项的短路行为。

type_predicate 的 not/and/or 构造改用显式后序工作栈，原 trait/equality 原子检查及具体错误保持不变。按 PTC 默认步骤/深度预算在构造阶段明确拒绝，不把求解失败转换为程序验证或执行。此项不扩展 PTC 到程序状态。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p elaboration --lib` 218 项通过（/tmp/luna-tools-cpx8eesg/result.json），新增 4096 层分类测试；`test -p luna --test execution refinement` 13 项通过（/tmp/luna-tools-t6wirorl/result.json），含深层 and/or 类型 refinement 的明确拒绝、正常嵌套公式和反例。格式化与定向 diff --check 通过。dev 未提交。

### 二元 AST lowering 显式后序遍历

lower_binary_expr 使用 Operand/Finish 工作栈处理所有既有标量二元 AST kind；其余表达式仍进入原 lowering。复用统一 binary_operator 映射，先处理左/右操作数，再按原顺序分配 HIR 子节点与父身份，不改变结合性、词法作用域或源位置。

长逻辑源码回归扩展到 512 项 and/or，覆盖返回上下文与 let 推断，仍正常返回后续 type-condition 深度诊断（并不声称 512 层求解/执行已支持）。新增执行用例检查左结合减法、块内遮蔽、指针布尔写回及 left/right 调用顺序。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test instantiation` 原有 55 项通过，新例最初因缺少运算符空格失败（/tmp/luna-tools-hmmdlsab/result.json）。中间 fixture 也触发了机器整数范围证明与 Integer 的 comptime-only 限制；最终用合法布尔写回和有界常量表达式隔离 lowering 行为，`... --test instantiation binary_lowering` 通过（/tmp/luna-tools-j467emhl/result.json）。`... --test execution` 全部 1367 项通过（/tmp/luna-tools-k0552klf/result.json）。格式化与定向 diff --check 通过。dev 未提交。

### quote fn contextual 输入的接入缺口

核对 docs/cmtt/contexts-and-substitution.md 与 quote-functions-and-templates.md：implicit quote 对应的是已建立的 runtime 接口绑定，不能靠同名/同型查找任意局部变量，也不能复制普通形参值槽来模拟对象身份。当前 program::ensure_signature 与 cmtt/functions::validate_quote_function 拒绝该标记；arguments/spreads 的普通 implicit 路由尚无 contextual 对应来源。quote expansion 对所有参数生成 prepared_connections 和 connections 两组新鲜值绑定，不能直接复用这条路径实现 contextual。

后续实现需联动三个边界：一是保留 quote 模板的参数绑定角色及接口身份（不要把它混入 comptime/runtime Sort）；二是调用路由读取已有 CMTT 对应，缺失或歧义不猜测，普通实参仍按原顺序求值一次；三是展开时将 contextual 参数直接对应到实际绑定，并一致替换正文、依赖类型、契约与 old 快照，清理不把借用的调用者对象当作普通形参拥有。多端口同对象、遮蔽、失败候选与别名消费必须有行为测试。仅解除 TPARAM_QUOTE 的拒绝检查是不正确实现。本轮未声称此能力已接通，也未添加猜测的显式供应语法。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test quote_functions` 103 项通过（/tmp/luna-tools-b6dp_h0a/result.json），确认当前普通值参数、契约、默认值、trait 展开和深度边界未回归。dev 未提交。

### Retained quote 模板的参数帧完整性

contextual 载体审计先暴露一个现有漏洞：phases::validate 会接受 quote 模板签名与 parameters 数量不一致，后续展开中的 LocalId 映射也不能区分重复参数身份。新增失败回归先确认此错误输入原先被接受（/tmp/luna-tools-ge3cptr6/result.json）。

阶段校验现在要求 quote 模板具有 callable signature、形参槽数量一致、绑定身份唯一，且 defaults 的槽位在界内并且不重复。保留原阶段/声明身份冲突优先诊断，不改变普通参数的求值与清理行为。测试覆盖五类损坏、每次恢复后的再次校验，以及原有 staged/trait/default quote 行为。此项是已有参数帧的完整性修复，尚未添加 contextual 参数角色或接通其源码。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p nir --test phases` 19 项通过（/tmp/luna-tools-083kpv1s/result.json）；`test -p luna --test quote_functions` 103 项通过（/tmp/luna-tools-b0ttc3zs/result.json）。格式化与定向 diff --check 通过。dev 未提交。

### 普通 callable 参数帧与 quote 共用完整性校验

新增普通 runtime/comptime Function 的损坏输入测试，确认缺少参数绑定原先仍通过阶段校验（/tmp/luna-tools-karltk9b/result.json）。抽取 validate_parameter_frame，共同检查 callable signature、参数数量、绑定身份唯一性、default 槽位范围与唯一性；quote 保留其原诊断文本。仅有声明身份而没有 body/参数数组的 runtime_function_identities 不套用有体函数的检查。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p nir --test phases` 21 项通过（/tmp/luna-tools-_w3oox8z/result.json）；`test -p luna --test execution --test quote_functions` 1367+103 项通过（/tmp/luna-tools-1vyj17v1/result.json）。格式化与定向 diff --check 通过。contextual 参数角色仍未接通；dev 未提交。

### 元组模式修复后的 workspace 集成验证

Type 受检值上的空模式 `()` 现降为 TypeTuple([])，与非空元组模式共用结构视图，不作为 Type 表达式求值。源码测试覆盖空/非空、嵌套、其他不透明头保持 Unknown、or 绑定对应、guard 失败后候选隔离，以及空元组模式不能覆盖全部 Type。表达式中的括号 tuple 仍不能充当类型。

/home/fin/luna：`python3 -B development/check.py --timeout 900 -- test --workspace` 退出 0（/tmp/luna-tools-v09wonpt/result.json），8120 项测试通过。此轮扩大检查用于前端、NIR 参数帧与解释器改动的阶段性集成，不代表尚未接入的 contextual quote 参数、一般 meta.List<T>、完整 sysroot 或后端已经完成。dev 改动未提交。

### 调用帧与默认实参作用域校验

phases 校验调用槽位的完整性、唯一性、默认值顺序及非空 parameter_bindings 的数量/唯一性；直接 Function/FunctionInstance 还要求绑定表与已保留目标声明一致。runtime 默认实参的自由绑定必须来自此前已准备的调用槽位，不能读取自身或调用者的任意局部变量。声明侧默认值单独按前序形参域检查，内部局部声明和合法闭包捕获不算自由输入。

注意 staged 默认实参在开放定义中可保留已替换的调用者 comptime 符号；不能提前套用 runtime 默认值帧的自由绑定规则。这一过强检查曾使 dependent_type_defaults_and_late_named_inputs_supply_context 和 ordinary_comptime_indices_in_capabilities_use_defaults_and_forwarded_evidence 失败（/tmp/luna-tools-ohfxumvo/result.json），现保留已有 staged 准备/专门化职责。

/home/fin/luna：源码 default 测试 execution/open_functions/quote_functions 共 270 项通过（/tmp/luna-tools-xcrmvne3/result.json）；解释器 --tests 194 项通过（/tmp/luna-tools-1a8k3vps/result.json）；nir phases 24 项通过（/tmp/luna-tools-a8lvwlbp/result.json）。格式化和定向 diff --check 通过，dev 未提交。

### 函数模板的语法与 AST 载体

核对 contextual quote 的文档示例发现，其上游 template fn 仍被解析器明确拒绝；现有 template test 的端口流程不能冒充函数模板实现。已新增 FunctionTemplateDef（包裹带修饰符的普通 Function 声明）、FunctionTemplateInstance（省略参数列表）和 FunctionTemplateInstanceWithParams（显式参数列表，含显式空列表）。后者复用 FunctionDefChildren 的 id/params/template/空clauses/body 布局。模板定义保留签名、子句与可选主体，实例的模板表达式不吞掉使用者主体，quote/inline/pure 等修饰符保持独立 AST 包装。

这一步仅接通语法与 AST；函数模板仍无 HIR 声明/实例载体、词法绑定链接和 elaboration，不能作为可执行功能宣称完成。下一步应保留声明侧签名作用域与实例侧新鲜参数身份，再接入签名复用和有主体模板的 CMTT 端口，而不是把模板作为普通函数降低。comptime 实例资格限制也属于后续语义检查。contextual quote 的绑定角色仍未接通。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p parse -p ast_lowering` 85 项通过（/tmp/luna-tools-cbzg_b4g/result.json），包含声明修饰符、无主体/块主体/表达式主体、带端口参数、具名/泛型模板表达式、显式空参数区别及错误恢复入口的拒绝例。格式化与定向 diff --check 通过；dev 未提交。

### 函数模板声明的 HIR 身份

FnModifiers 新增 is_function_template，Display 输出 function-template 字段。名称扫描穿透 FunctionTemplateDef 包装，仍用声明侧函数作用域解析参数、类型、默认值和主体；lowering 保留原 owner/节点表并标记模板类别，拒绝 comptime 函数模板。模板声明在 NIR 入口保留 function template expansion pending 诊断，不能提前注册成可执行普通函数。

/home/fin/luna：`test -p luna --test test_templates` 59 项通过（/tmp/luna-tools-b_pq8yfu/result.json），新增声明/修饰符/上下文参数/跨源模块导入和 comptime 拒绝检查；`test -p hir` Display 13 项通过（/tmp/luna-tools-vnuxos_0/result.json，3 个原有 doc 示例 ignored）。中间修正 fixture 误用 mod 文件声明，并补齐 Display 测试的 FnModifiers 字段。check -p luna、格式化与定向 diff --check 通过。

下一步仍是函数模板实例 HIR 和链接：省略参数列表时必须从已扫描模板取得声明接口名字，为实例建立新鲜参数身份，并在声明侧解释类型；不能在使用者作用域重解析模板签名。带主体模板的 contextual 端口和 quote fn contextual 输入也未接通。dev 未提交。

### 函数模板实例 HIR 与声明侧接口链接

新增 FunctionTemplateInstance / FunctionTemplateInput：分别保留模板表达式、声明侧 FnSig、显式参数列表（None 与 Some([]) 不合并）、实例输入与 formal 身份对应、实例修饰符及 BodyId。Display 输出独立的 function-template-instance/input S-expression。resolver 预扫描函数参数名字；省略参数的实例用 ExpandedLocal 建立新鲜身份，显式参数沿用普通参数 lowering。所有文件完成后链接原声明签名，不在实例作用域重解析类型，也不按名字替换声明侧类型身份。

已支持此阶段的静态具名模板头（含 NFApplication 外壳）、跨文件导入、前向声明、显式改名、空参数及 quote/inline/pure 实例修饰符；拒绝普通函数目标、参数数目错误、重复输入、comptime 实例和作用域泄漏。这里只形成和链接 HIR；实例的类型一致性检查、模板实参替换、签名转化为可执行 Fn、带主体模板的 contextual 端口及 quote fn contextual 输入仍未实现，NIR 保持 function template expansion 诊断。

/home/fin/luna：check -p luna 通过（/tmp/luna-tools-zbjt053t/result.json）；test -p hir -p resolve -p ast_lowering 共 30 项通过（/tmp/luna-tools-qq7ewwro/result.json）；test -p luna --test test_templates 共 62 项通过（/tmp/luna-tools-n1wfqa77/result.json），新增实例身份、跨模块类型遮蔽和拒绝例。格式化与定向 diff --check 通过；dev 未提交。

### 签名模板实例接入函数执行

elaboration 在临时 HIR arena/package 视图中实例化无共享主体的函数模板，原始 HIR 的模板声明/实例节点和 Display 保留。继承参数的身份映射同时应用于参数类型、返回类型、默认值、结果绑定及 requires/ensures/decreases；声明引用保持原 DefId，投影权限元数据随新表达式身份复制。实例签名使用独立身份空间，避免与已经 lowering 的主体冲突；普通、inline、quote 类别由实例决定，模板 pure 要求继续保留。模板声明接受抽象接口检查，但不能作为普通函数调用或取得函数值。

支持含普通 comptime 参数的依赖签名以及默认值中的基础表达式/条件控制流，实例走已有函数检查、默认调用、契约、quote 展开和执行路径。泛型子句替换、显式实例参数兼容性、共享 CMTT 正文/上下文端口、尚未覆盖的复杂签名表达式替换仍明确拒绝；不把这些限制写成语言约束。后续应继续扩展同一身份映射，而不是重新解析声明文本或展开 normal-form 类型头。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test test_templates --test default_calls --test function_type_dependencies --test quote_functions` 全部通过，证据 `/tmp/luna-tools-h_kt4_40/result.json`。新增执行测试覆盖三种实例类别、独立输入与默认绑定、返回契约拒绝、comptime Type 依赖、跨模块同名类型遮蔽，以及模板自身不能调用/取值。已格式化受影响文件，定向 diff --check 通过；dev 未提交。

### 模板签名中的局部绑定、模式与嵌套正文替换

扩展 function_templates::Rebind：局部 let/var、赋值、循环跳转、模式分支/守卫、结构构造、错误/效应分支和闭包正文使用统一身份映射。pattern helper 保留 tuple/record/enum/optional/error/or/then/bitvec 等模式的绑定及子表达式；嵌套 BodyId 与 BodyParam 同步复制到临时 package。映射以完整 HirId 为键分配新局部编号，避免不同源 owner 的相同 local_id 相撞；投影权限等元数据对全部映射节点复制。表达式、模式、类型参数、块及嵌套正文共享燃料与深度限制。

执行对照覆盖普通函数/模板的解构、局部变更、guard、or 整体绑定、同名遮蔽、捕获和嵌套闭包；调用轨迹验证默认值调用次数/顺序，显式覆盖不执行默认值。结构构造默认值及字段消费通过。循环默认值、写前序输入及局部 nominal 解构目前不能通过普通默认值的 P3 admission；模板保持同样拒绝，未将其改成语言限制。泛型子句、显式实例参数兼容性、共享 CMTT 正文与其它尚未支持的签名节点仍待实现。

/home/fin/luna：最终 `python3 -B development/check.py --timeout 300 -- test -p luna --test test_templates` 70 项通过（/tmp/luna-tools-zx6gxfyy/result.json）；同一实现下 default_calls / function_type_dependencies / quote_functions 分别 33/33/103 项通过（/tmp/luna-tools-636p3qmk/result.json）。该次组合运行另有一个新增 nominal 解构正例失败，确认普通函数基线也因 P3 拒绝后调整为拒绝对照；最终模板组已全部重跑通过。格式化与定向 diff --check 通过；dev 未提交。

### 模板闭包环境签名与资源默认值证据

补充 TyClosure / TyClosureEnvironment 替换：保留环境名字，重绑定字段类型、可选来源表达式及 callable signature。源测试覆盖普通/依赖 comptime Type 的返回闭包、将闭包传入另一模板实例、两份实例独立的可变捕获，以及捕获类型不符的拒绝。

澄清上一节的 nominal 解构限制：无 Drop 协议绑定时观察证明不会擅自接管 nominal owner，并非解构表达式缺失实现。本轮绑定 Copy/Drop、提供空析构后，直接 let 解构、经局部 cell 解构和 match 解构在普通函数/模板中均编译执行；析构写外部指针则仍被 P3 拒绝。不放宽 copy_type，不以字段布局代替资源协议，不修改 PTC 边界。

/home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test test_templates --test default_calls --test function_type_dependencies` 72+33+33=138 项通过（/tmp/luna-tools-mfz0dg7l/result.json）。已格式化修改文件；dev 未提交。泛型模板子句、显式实例参数一致性及共享 CMTT 正文仍未完成。

### 签名模板中的 tactic lambda 默认回调

Rebind 接通 TacticParameter 和 TacticArgument：占位参数保存其所属 tactic 边界内的位置，不错误映射成模板声明的 HIR 局部身份；边界正文、其中的普通参数引用与捕获仍通过既有 fresh-id 映射克隆。使默认闭包正文中的高阶调用及嵌套 tactic 可以随签名复用。

新增用例验证两份实例、嵌套 `_0` 的独立边界、默认回调显式覆盖、调用顺序，以及捕获模板 runtime 输入的默认闭包。参考执行与 LLVM O0/O3 均通过。缺失 callable 边界和超出目标 arity 的占位参数继续由普通 tactic 检查拒绝。默认值形成时直接调用未获 P3 证明的高阶函数仍拒绝；最初尝试的诊断证据 `/tmp/luna-tools-9u46pqj5/result.json`，没有为接入模板而降低默认值 admission。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test test_templates --test default_calls --test pure_callbacks --test elaboration` 退出 0（`/tmp/luna-tools-dls2es8d/result.json`）。格式化退出 0（`/tmp/luna-tools-0_lo1v1g/result.json`），git diff --check 通过。docs/functions/templates.md 增加已验证的默认回调与嵌套 tactic 示例。泛型子句替换、显式实例参数兼容性与共享 CMTT 主体未因本改动实现。dev 未提交。

### 签名模板默认值中的 CMTT 结构克隆

Rebind 新增 CodeQuote、CodePatternQuote、CodeSplice、ContextualExprType、ContextualPatternType。substitution 保留端口名并重绑定其表达式，contextual binding 保存名称与词法默认关联，HIR 身份通过同一 fresh-id 映射更新；已有 pattern 重绑定负责 quote 内的 pattern binder 和成功输出。每个子表达式沿原预算/深度检查克隆，未把代码值当作普通 runtime 数据或重新进行按名称捕获。

新增真实源码用例覆盖闭合表达式代码、单洞组合、显式替换、模板参数的隐式关联和 pattern quote。两份实例分别以 true/false 输入执行，默认值显式覆盖保留，参考执行和 LLVM O0/O3 一致。拒绝例确认双 splice 仍违反单洞限制，缺失词法端口不能被实例主体中的同名局部变量补足。docs/functions/templates.md 增加已验证的 contextual 默认值示例。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test test_templates --test cmtt --test default_calls` 退出 0（`/tmp/luna-tools-asxovi1m/result.json`）。格式化退出 0（`/tmp/luna-tools-7nlrpvf_/result.json`），git diff --check 通过。本轮接通签名内代码结构的克隆，不代表带共享 CMTT 主体的 template fn 已完成；其主体端口连接、泛型子句替换和显式实例参数兼容性仍待实现。dev 未提交。
