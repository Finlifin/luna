# Literal frontend integration

The source-to-HIR literal path now preserves numeral spelling, decodes fixed text, and retains suffix conversions for later elaboration. This is a bounded milestone toward full Flurry implementation; it does not execute numeric arithmetic or choose a LiteralInto implementation.

## Delivered behavior

- Decimal, binary, octal, hexadecimal and scientific tokens reach the appropriate AST/HIR nodes. Arbitrarily long numeral spellings survive without fixed-width overflow, floating-point rounding or zero fallbacks. Invalid radix digits, mixed hex letter case and malformed scientific forms diagnose.
- Strings and characters share the documented escape language and Unicode scalar restrictions. Adjacent strings separated only by whitespace, including an empty gap, concatenate after per-leaf decoding. Character literals, quote forms, and take-view chains keep distinct boundaries.
- Suffixes remain one `LiteralConversion` request; expression and ordinary literal-pattern paths preserve their operand and suffix. Comments are filtered from the parser's token stream with identifier-symbol indices remapped; source gaps still prevent concatenation across comments.
- BitVec-prefixed patterns retain BitVec AST identity. Their currently missing lowering and currently missing interpolation lowering diagnose explicitly. Existing output files survive failures.
- New HIR payloads have stable S-expression Display. The implementation map records the remaining compiler spine and dependencies without presenting crate skeletons as completed features.

## Evidence

Working directory: `/home/fin/luna`.

| Check | Actual result | Output |
|---|---|---|
| `cargo test -p lex -p parse -p hir -p ast_lowering -p luna --locked --offline --target-dir /tmp/luna-dev-002/target` | Exit 0; 106 tests passed, four existing doctests ignored | `/tmp/luna-dev-002/repaired-tests.log` |
| `cargo check -p luna --locked --offline --target-dir /tmp/luna-dev-002/target` | Exit 0; existing warnings remain | `/tmp/luna-dev-002/repaired-check.log` |
| Changed Rust file `rustfmt --check --edition 2024 --config skip_children=true` | Exit 0; 16 files checked | `/tmp/luna-dev-002/repaired-format.log` |
| Run example exported with `luna ... --output /tmp/luna-dev-002/example.hir.lisp`, then Emacs reader/formatter | Exit 0 | `/tmp/luna-dev-002/emacs.log` |
| `python3 development/runs/002-literal-foundations/verify-workspace.py` | Exit 0; protected regular-file baseline unchanged, dev and HEAD preserved | `/tmp/luna-dev-002/repaired-workspace.json` |

Tests comprise 18 lexer groups, nine parser groups, nine HIR Display groups, four decoder groups, seven CLI unit tests, 32 earlier CLI integration tests, and 27 new literal CLI groups. Table-driven groups exercise multiple inputs. The first integrated run had one diagnostic failure; its log remains `/tmp/luna-dev-002/integration-tests.log`. The HIR owner repaired only the BitVec unsupported diagnostic, followed by the affected integration check.

## Graph and integration state

Three writers used separate copies of the same dirty dev snapshot; root owned AST/parser interfaces and integration. The HIR worker received the explicit StringConcat interface from root. Scoped integration checked destination hashes before copying each delta; the one subsequent diagnostic repair also checked its original hash. Exact baseline, write boundaries, dispatches, limits and evidence are recorded in `graph.json`; the reviewable delta against that baseline is `/tmp/luna-dev-002/final.patch`.

The first independent grade passed 12/13 applicable criteria and found two C12 lexical boundary defects. The lexer owner, parser owner and independent test owner repaired them: block-ending character statements preserve Char tokens for context-sensitive parsing, and interpolation retains nested lexical content. The second independent grade passed **13/13 applicable criteria** after independently rerunning all **106 tests**, cargo check and 13 additional boundary probes. The acceptance loop is complete for this milestone; the full-language objective remains active. The build-and-grade loop allows at most two grading rounds and routes defects to their owner. C1, C2, C7 and C8 are record-class criteria; their scheduling evidence is attested by the run log, while outcome checks require fresh reruns.

Development remains on `dev`; no commit, push, merge, deployment, publication or external message occurred. Failed/new duplicate build caches were cleared with cargo clean after disk quota failures; source, snapshots and logs were retained. The grader independently executed the first 97 test binaries successfully; its initial fresh build hit disk quota, which is recorded separately from source failures. After cache cleanup and code repair, the independent cargo test/check runs both completed successfully using the existing task target. No dependency or backend change was needed. Existing unrelated modifications remain intact. This check covers regular source/artifact files in the captured baseline, excluding build outputs and symlinks.

Independent final evidence: `/tmp/luna-dev-002/grader2-tests.log`, `grader2-check.log`, `grader2-probes-checked.json`, `grader2-workspace.json`, and `grader2-artifacts.json`; verdicts are preserved in `acceptance.json`.

Observed limits: 5/5 workers including root, peak concurrency 4/4, waves 5/6, retries 1/1, depth 1/1, worker turns 9/16, grading rounds 2/2, elapsed 3377/5400 seconds.

## Remaining language work

Numeral strings are HIR syntax, not a bigint/Float value implementation. LiteralInto selection, interpolation, full BitVec pattern semantics, type checking, comptime execution, validated NIR/IIR and executable generation remain outstanding. Quote token coverage is not CMTT elaboration. Old disabled backend/typeck code is not validated by these targeted checks. The broader objective remains active; next work starts with stable semantic representations and a minimal elaboration path as described in `development/implementation-plan.md`.
