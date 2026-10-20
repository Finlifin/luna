---
name: flurry-tutorial-writer
description: >-
  Write, plan, and review episodes of the Flurry programming language tutorial (`flurry_tutorial/`).
  Use this skill whenever the user asks to write new `flurry_tutorial` episodes, refine existing
  tutorial posts, plan the tutorial roadmap, or iterate on the tutorial writing methodology.
  Enforces the "Rust Baby Steps" (problem-driven iterative design) + "Lin Clark Code Cartoons"
  (visual mental models) style and strict adherence to `docs/` language semantics.
---

# Flurry Tutorial Writer (`flurry-tutorial-writer`)

This skill codifies the narrative style, pedagogical structure, visual conventions, and strict language-design verification rules for writing the `flurry_tutorial/` series (envisioned as a multi-hundred-episode progressive journey into Flurry).

## 1. Core Pedagogical Philosophy

Never write a dry "feature catalog" or reference manual entry. Every episode must feel like an engaging engineering story combining two classic blog styles:

1. **Niko Matsakis's *Rust Baby Steps* Style (Iterative Problem-Solving)**:
   - **Start with a concrete system programming requirement** (e.g., parsing an 8-bit hardware sensor packet, hot-swapping a channel handle on failure, slicing a buffer without overflow).
   - **Write `v0` first**: Show what happens when we write it the naive way (or with habits carried over from C/C++/Rust/Zig). Point out the concrete pain points, bugs, or cognitive friction in `v0`.
   - **Iterate (`v0` → `v1` → `v2`...)**: Introduce Flurry language features *only* as the natural solution to the pain points exposed in the previous version.
   - **Show rejected code (`-- ❌ 编译期直接拒绝！`)**: Explicitly show small snippets of buggy code that Flurry's compiler catches at definition or compile time, explaining *why* it is rejected.

2. **Lin Clark's *Code Cartoons* Style (Visual Mental Models)**:
   - Before or right after introducing a core mechanism, draw a clear **ASCII / Unicode box-and-arrow cartoon** (or Mermaid diagram when appropriate) showing what the compiler, type system, or memory layout is actually doing.
   - Give abstract concepts physical personas or visual metaphors (e.g., `let` welds the *name tag* to the box without freezing the contents inside; BitVec patterns slice from MSB on the left to LSB on the right; Error Rows act as labeled railway tracks).

3. **Zero Ecosystem Assumption**:
   - Flurry's standard library and built-in packages are still under design. Focus strictly on **the language itself** (syntax, semantics, type system, comptime, resource management, algebraic effects, verification).
   - Only use `use std.prelude.*;` (with `println` / `eprintln`) or explicitly declare any helper types/functions (`struct`, `enum`, `trait`, `fn`) inside the episode's code snippets.

---

## 2. Mandatory Pre-Writing Verification (Strict `docs/` Compliance)

**NEVER guess or hallucinate Flurry syntax or semantics from Rust, Zig, or `OLD-2024/`.**
Before writing an episode on any topic:

1. **Consult `docs/README.md`**, `docs/grammar/syntax`, and `docs/grammar/index.md`.
2. **Read the specific authoritative `docs/<topic>.md` files** for the features covered in the episode.
3. **Verify every code snippet against the Flurry Syntax Checklist**:
   - **Comments**: Line comments use `-- ...` (NEVER `//`). Block comments use `{- ... -}`.
   - **Semicolons**: Statements end with `;`, **EXCEPT** when the final token of a statement or definition is `}`—in that case, **omit the trailing `;`** (do not write `};`).
   - **Infix Whitespace**: Binary operators `+`, `-`, `*`, `/`, `%`, `<`, `>`, `<=`, `>=` **must have whitespace on both sides** (`a + b`, `a < b`). Without whitespace on both sides, `<` and `>` parse as generic brackets (`BitVec<4>`) and `-` parses as unary negation (`-42`).
   - **Boolean & BitVec Logic**: Use `and`, `or`, `not` (NEVER `&&`, `||`, `!`).
   - **Pointers & Views (Postfix!)**:
     - Address-of: `expr.ref` (NEVER `&expr`).
     - Dereference: `expr.*` (NEVER `*expr` in expressions; `*T` is only for pointer types).
     - Take view: `expr'view_name` (e.g., `raw'bitvec`, `val'type`, `msg'tag_name`).
     - Type cast: `expr.as(T)` (postfix function-like keyword).
   - **Types (Prefix!)**:
     - Pointer type: `*T`
     - Optional type: `?T` (with `null` for absence, `expr?` for propagation, `pat?` for matching presence).
     - Error-qualified type: `!Err T` or `![Err1, Err2] T` (with `error expr` to construct, `expr!` to propagate, `expr! { ... }` to eliminate, `val!` and `error pat` in `match`).
     - Effect-qualified type: `#Eff T`.
   - **Control Flow & Pattern Matching**:
     - Postfix match: `expr match { pat => body, ... }` (arms separated by commas `,`).
     - Single-pattern `if` / `while`: `if expr is pat do { ... }`, `while expr is pat do { ... }` (`do` is mandatory!).
     - Statement guards on jumps: `return expr while cond;`, `break label while cond;`, `continue label while cond;` (keyword is `while`, NEVER `if`!).
     - Loop labels: `while :label cond { ... }`, `for :label pat in expr { ... }`.
   - **Naming Conventions**:
     - Types & Traits: `PascalCase` (including functions returning `Type`, e.g., `Tuple`).
     - Error types: `PascalCase` ending in `Err` (e.g., `DecodeErr`).
     - Error enum variants: `PascalCase` (e.g., `DecodeErr.BadByte`).
     - Ordinary enum variants, functions, variables, modules, effects, custom `case`: `snake_case` (e.g., `Packet.empty`, `Message.move_to`).
     - Constants: `SCREAMING_SNAKE_CASE`.
   - **Semantic Red Lines (`AGENTS.md` §5)**:
     - `let` only forbids whole-value assignment to the binding name; it does NOT freeze fields or pointees.
     - Both `let` and `var` require an initializer (`var x: u32;` is illegal).
     - Struct construction requires explicit field names (`Point { x: 1, y: 2 }`), no field punning (`Point { x, y }` is for patterns, not construction).
     - Normal-form heads (`pure comptime fn Foo<...> -> Type`) do not unfold their identity.
     - **Trait Comptime Phase vs. Function Execution Stage**: Trait items belong to the runtime phase by default; items marked with `^comptime_phase` (`^attrs.comptime_phase`) belong to the comptime phase. Phase attribution (`^comptime_phase`) is orthogonal to function execution stage (`comptime fn` vs `fn`). Ordinary `impl` / `derive` implement the runtime phase; `comptime impl` / `comptime derive` implement the comptime phase (`^comptime_phase` items). Never claim that `comptime fn` alone defines the comptime phase of a trait!
     - **Comptime Structural Reflection**: The primary everyday metaprogramming pattern is `Type.repr_of(T)` + `inline if` / `inline for field in info.fields` + `val'get_field(field.name)` (or `'tuple_enumerate`), often packaged inside `derive fn`.

---

## 3. Episode Structure Template

Each file in `flurry_tutorial/` should be named `NNN-slug.md` (e.g., `001-a-world-in-a-byte.md`) and follow this arc:

1. **Title**: `# Flurry 演进手记 (NNN)：<副标题——从痛点/旧习惯到新心智模型>`
2. **The Scenario (需求引入)**:
   - Connect to the previous episode's cliffhanger (if part of an arc) and present a concrete, relatable engineering requirement.
3. **Version 0 (`v0` —— 直觉写法与痛点暴露)**:
   - Show the naive implementation.
   - Draw a **Code Cartoon** illustrating what goes wrong or why it's fragile.
4. **Iterative Upgrades (`v1`, `v2`, ...)**:
   - Introduce 1–2 tightly related Flurry features per iteration.
   - Include **Code Cartoons** for memory layout, control flow, or type checking phases.
   - Use `> **🔍 语法小贴士**` or `> **💡 核心心智模型**` callouts for lexical/syntactic rules so they don't interrupt the main narrative flow.
   - Include at least one **negative example (`-- ❌ 编译期直接拒绝！`)** showing what happens when a contract, width, or rule is violated.
5. **Summary Table (`本篇小结`)**:
   - A compact table comparing `v0` → `v1` → `v2` and listing the core language building blocks unlocked.
6. **Cliffhanger (`留给下一篇的悬念`)**:
   - Point out a natural next requirement that the current code cannot yet handle cleanly, leading directly into Episode `NNN + 1`.
