# 04 — `builtin` 全包与 `meta.math` 完整规格与缺口 Wishlist

> **状态**：Wave 1 详细设计规格与缺口清单（owner: `builtin-author`）  
> **目标范围**：`builtin` 全部子包/模块（primitives、固有方法、原生可选/错误组合子、`math`、`iterator`、`fmt`、`text`、`resources`、`mem`、集中绑定与 `language-items.conf`）以及 `meta.math` 和对应 `PureComptime*` 编译时对偶 trait 家族。

---

## 1. 范围、权威来源与归属包

### 1.1 覆盖范围与六个内置包边界

Flurry 严格仅设 **六个内置包**：`builtin`、`meta`、`build`、`attrs`、`std`、`verification`。本规格书负责的模块与跨包接口边界如下：

1. **包归属硬约束**：
   - `math` 必须归属 `builtin.math`（并通过 `builtin.prelude` 默认导出）与 `meta.math`，**绝不**另立第七个内置包。
   - `str`（不可变 UTF-8 文本视图）属于 `builtin`（`builtin.primitives` 与 `builtin.text`）；可增长的堆分配 `String` 属于 `std`（`std.string`）。
   - 内存相关模块统一命名为 `mem`（`builtin.mem` 与 `std.mem`）。
   - `Tuple<...Es>`、`Array<T, N>`、原生可选 `?T`、原生错误结果 `!E T` 与多错误行 `![E1, E2, ...Rest] T`、基础资源协议（`Once`/`Copy`/`Clone`/`Drop`）、迭代器（`Iterator`/`IntoIterator`/`FromIterator`/`emit_item`）、格式化协议（`Display`/`Debug`/`ByteWriter`）与 `builtin.mem` 原始/不安全内存原语全部隶属 `builtin`；对应的编译时纯特质对偶（`PureComptime*`）与编译时高精度数学 `meta.math` 隶属 `meta`。
   - **切片与数组归属对齐**：当前实现中 `Slice<T>` 暂存于 `library/std/collections_impl/slice.fl`（由 `std.collections.Slice` 与 `std.prelude.Slice` 导出），而 `Array<T, N>` 尚无绑定且未进入 `builtin.primitives`。从语言核心看，`Array<T, N>`（定长内联数组）必须纳入 `builtin` 集中绑定；`Slice<T>` 作为 `str` 底层等价表示、`ListPatternView`、`VariadicCollector` 及无分配器字节/元素视图基础，设计上应评估是否由 `std.collections` 下沉至 `builtin`（并通过 `std.collections`/`std.prelude` 重导出），参见 §3.1.9 与 §5 Q1。
   - **prelude 导入约定**：`builtin`、`meta`、`build`、`attrs`、`verification` 的 `prelude` 默认自动导入（不包含提供者自身）；`std.prelude` 必须显式导入。因此进入 `builtin.prelude` 的类型与 trait 直接决定用户无需显式导入即可获得的开箱语言体验（含 `u8..f64`、`bool`、`char`、`str`、`Symbol`、`void`、`NoReturn`/`never`、`Tuple`、`Array`、`OptionalOps`、`OptionalUnwrap`、`ErrorResultOps`、`ErrorResultUnwrap`、`Eq`/`Ord`/`Order`/`Hash`/`Clone`/`Copy`/`Drop`/`Once`/`Display`/`Debug`/`ByteWriter`/`Iterator`/`IntoIterator`/`FromIterator`/`FromStr`/`builtin.math`）。
2. **集中绑定层规则（ZERO use-site `.symbol'builtin`）**：
   - 禁止在任何普通方法体、trait 实现、签名子句或用户代码中直接使用 `.symbol'builtin`。
   - 所有 `.symbol'builtin` 原语绑定必须集中定义在 `library/builtin/primitives.fl`、`library/builtin/bindings.fl`、`library/builtin/types/tuple.fl`、`library/builtin/numeric/math_intrinsics.fl` 与 `library/meta/bindings.fl` 等集中绑定层中；其余库模块通过具名导出别名（`bool`、`u32`、`Tuple`、`Array`、`BitVec` 等）引用。
3. **内置包孤儿规则豁免（`AGENTS.md` §3）**：
   - 六个内置包均可直接为编译器 builtin 类型编写固有 `impl` 与 trait `impl`（如 `impl u32 { ... }`、`impl Display for ?T where ...`、`impl ComptimeTextBytes for str`），不受用户包孤儿规则限制；但不免除一致性、重叠/冲突实现检查与语义约束。
4. **严格语言红线**：
   - **严禁 `Option` 或 `Result` 枚举**：必须且只能使用语言原生 `?T`（`null`、`x?`）与 `!E T` / `![E1, E2, ...Rest] T`（`error e`、`x!`）。现有 `library/builtin/types/option.fl` 与 `library/builtin/types/result.fl` 仅为实现文件命名，其公开契约只能是针对原生 `?T` 与 `!E T` 的组合子 trait（`OptionalOps` / `OptionalUnwrap` 与 `ErrorResultOps` / `ErrorResultUnwrap`）。
   - **统一方法与双阶段规则**：单阶段类型方法直接定义在该类型的固有 `impl` 中，不另造自由函数或承载 trait。同一类型同时存在于 runtime 与 comptime 且双阶段签名/语义需要区分时（如 `?T`、`!E T`、`str`、`Iterator`、`Display`），分别通过 runtime `trait` 与 `pure comptime trait` 提供**同名方法**（禁止 `comptime_` 前缀或 `_comptime` 后缀），且 comptime 一侧一律使用 `self` 值引用语义（严禁 `*self`、`.ref`、`.*`、`*List<T>`）。
   - **统一 `Once` 规则**：全语言只允许单一全称实现 `impl Once for T where T {}`，严禁为各标量、元组、数组、指针、可选或错误结果重复编写 `impl Once`。

### 1.2 权威语义依据与非权威参考边界

| 类别 | 路径 | 权威性说明 |
|---|---|---|
| 协作协议与硬性规范 | `AGENTS.md` | 最高协作规范（双阶段规则、集中绑定、错误命名、allocator 范式、`Once` 规则） |
| 基础类型与词法 | `docs/basics/{primitives,literals,unit-null-undefined,lexical}.md` | 标量、字面量、`void`/`null`/`NoReturn` 基础定义 |
| 限定与结构类型 | `docs/type/qualified-types/{optional-type,error-qualified-type,error-type}.md` | 原生 `?T`、`!E T`、异构错误行 `![E1, E2, ...Rest] T` |
| 类型身份、反身、NF、布局与约束 | `docs/type/{self-reference,type-identity,runtime-dependent-type,comptime-gradual-dependent-type,receiver-adaptation,rows,reflection-and-layout}.md`, `docs/type/propositional-constraints/atoms.md`, `docs/type/user-defined-types/{structs,enums}.md` | `Self`/`Itself`/`itself`、`Tuple` 非特判 NF 头、A01-A19 原子、布局与对齐反射、位域与枚举 |
| 空文档占位（须显式标注空白） | `docs/others/arbitary-sized-integers.md` (0B), `docs/type/reachability-type.md` (0B), `docs/type/propositional-constraints/occurrence-typing.md` (0B), `docs/type/qualified-types/reachability-type-also.md` (0B) | 正文尚未撰写，需在 §5 中提交作者裁决 |
| 资源与内存模型 | `docs/resource-management/{once-clone-copy,drop-elaboration,pointers,memory-views,moves-and-initialization,memory-effects,reachability,contracts-and-verification,runtime-purity}.md` | 仿射移动与隐式析构、Pin-free 移动与重初始化、指针与内存视图、P0-P5 纯度分级 |
| 位向量与安全模型 | `docs/control-flow/{basic,pattern-matching/index,pattern-matching/bitvec}.md`, `docs/program-synthesis/bitvec-algebra.md`, `docs/program-verification/safety.md` | `BitVec<N>` 局部写回、`never` 控制流、unsafe 责任边界 |
| Trait、Comptime 与效应 | `docs/traits/{index,protocols,coherence,derivation,implementation-resolution}.md`, `docs/comptime/{the-comptime-phases-of-traits,purity-and-normal-form,quoting-and-splicing,official-attribute-keys,inlining-comptime-code}.md`, `docs/algebraic-effect-and-delimited-continuation/{error-handling,handling-capability,direct-passing}.md`, `docs/pipeline-projection/{intrinsics,target-validation-and-assembly}.md`, `docs/code-organization/builtin-packages.md`, `docs/grammar/{index,syntax}` | Trait 解析与一致性、`^attrs.*` 唯一合法官方属性通道、效应直传、`inline fn iterate`、`target.*` 与 intrinsic 模型 |
| 已确认历史讨论 | `docs_chat_history/{016-control-flow-10,032-type-system-04,037-type-system-09,038-type-system-10,045-type-system-16,046-trait-implementation-02}.md` | `Array`/`Slice` 模式与 `VariadicCollector`、`newtype.as(Underlying)` 视角等已确认结论 |
| 现状与编译器审计（非语义权威） | `library/{builtin,meta,std,attrs,language-items.conf}`, `compiler/{nir/src/{types,intrinsics}.rs,elaboration/src/program/*}`, `ai_dev_logs/{builtin_packages_gap_report_2026-11-21,codebase_audit_report_2026-11-22,runtime_abi_inventory_2026-11-21}.md`, `issues/0003-builtin-packages-compiler-blockers.md`, `tmp/silver-design/04-compiler-wishlist.md`, `development/{library-prototypes/*,repros/native-text-method-provenance.fl}` | 仅用于盘点现状代码、compiler blocker 与运行时 ABI |
| 明确排除参考源 | `proposal_trace/`（`2026-05` 前历史文件如 `2024-*`/`2025-*`/`2026-1..4-*`；仅 `2026-5..10` 可作对照且一切以 `docs/` 为准）、`prompt.fl`、`docs/OLD-2024/`、`flurry_tutorial/` | 禁止当作正式语言语义权威 |

---

## 2. 现状与缺口全景总表

当前 `library/builtin` 已有 27 个 `.fl` 文件（7,229 行），`library/meta` 已有 36 个 `.fl` 文件（6,734 行），基础骨架已建立，但距离完整自举与最终设计范式仍存在系统性缺口，汇总如下（共 28 项）：

| ID | 优先级 | 所属子模块 | 现状基线 | 关键缺口与偏差 | 目标层 / 责任归属 |
|---|---|---|---|---|---|
| `WL-04-01` | P1 | `builtin.primitives` (`bool`) | `primitives.fl` 已绑定 `.bool'builtin`，`comparison`/`traits`/`display`/`debug`/`resources` 已实现基础 trait；**零固有方法** | 缺少 `bool` 固有方法（`then`、`then_some`、`not`?），缺少可选桥接 | `library/builtin` + `meta` |
| `WL-04-02` | P0 | `builtin.primitives` (定长整数) | `u8..u128`/`usize`/`i8..i128`/`isize` 已绑定；`conversions.fl` 含 `TryFrom`/`TryInto`；`math_intrinsics.fl` 仅暴露自由函数 `saturating_add`/`saturating_sub`/`min`/`max`/`bit_reverse`/`count_ones`/`leading_zeros`/`trailing_zeros`/`byte_swap`/`rotate_left`/`rotate_right`/`wrapping_abs` | 1. 无任何固有 `impl` 方法；2. 缺全套 `checked_*`、`wrapping_*`、`overflowing_*`、`carrying_*`、`borrowing_*`、`widening_mul`、`div_euclid`/`rem_euclid`/`div_ceil`/`div_floor`/`is_power_of_two`/`next_power_of_two`/`midpoint`/`ilog*`/`isqrt`/`abs_diff`/`signum`/`is_positive`/`is_negative`、`from_be`/`from_le`/`to_be`/`to_le`/`*_bytes` 及 `MIN`/`MAX`/`BITS` 常量；3. 缺溢出/进位目标 intrinsic | `library/builtin` + `compiler/nir` |
| `WL-04-03` | P0 | `builtin.primitives` (浮点 `f16`/`bf16`/`f32`/`f64`) | `primitives.fl` 已绑定 4 种浮点；`math_intrinsics.fl` 仅提供 `isnan`/`isinf`/`isfinite`/`isnormal`/`issubnormal`/`signbit` 等自由函数 | 1. 无任何固有 `impl`；2. 缺全部 IEEE 常量（`NAN`/`INFINITY`/`NEG_INFINITY`/`EPSILON`/`MIN`/`MAX`/`MIN_POSITIVE`/`RADIX`/`MANTISSA_DIGITS`/`DIGITS`/`*_EXP`）；3. 缺 `classify` (`FpCategory`)、`is_nan`/`is_infinite`/`is_finite`/`is_normal`/`is_subnormal`/`is_sign_positive`/`is_sign_negative`、`total_cmp`、`to_bits`/`from_bits`、`clamp`、`lerp`、`to_degrees`/`to_radians`、`to_*_bytes`/`from_*_bytes` 及数学方法转发 | `library/builtin` + `compiler/nir` |
| `WL-04-04` | P1 | `builtin.primitives` (`char`) | `conversions.fl` 已有 `from_u32`、`as_u32`、`len_utf8`、`is_ascii*`（11 个）、`to_ascii_lowercase`/`uppercase`、`eq_ignore_ascii_case` | 缺 Unicode 一般分类与大小写转换（`is_alphabetic`/`is_numeric`/`is_alphanumeric`/`is_whitespace`/`is_control`/`is_lowercase`/`is_uppercase`/`to_lowercase`/`to_uppercase`）、`to_digit`/`from_digit`、`len_utf16`、`encode_utf8`/`encode_utf16`、`escape_debug`/`escape_default`/`escape_unicode`、`MIN`/`MAX` 及 `PureComptimeChar` 对偶 | `library/builtin` + `meta` |
| `WL-04-05` | P0 | `builtin.text` (`str`) | `builtin/text_impl/bytes.fl` 提供 9 方法 trait `TextBytes`；`meta/data/text.fl` 提供 11 方法 `ComptimeTextBytes` | 1. 受 `str` provenance 追踪缺失阻塞（`development/repros/native-text-method-provenance.fl`、`issues/0003`）；2. `str` 固有方法被外部 `TextBytes` 替代；3. 缺 `slice`/`get`/`split*`/`rsplit*`/`trim*`/`lines`/`bytes`/`chars`/`char_indices`/`parse<T>`/`starts_with`/`ends_with`/`strip_prefix`/`strip_suffix`/`replace`/`replacen`/`match_indices`/`eq_ignore_ascii_case`/`to_ascii_*` 及其纯编译时 `ComptimeText` 同名对偶 | `library/builtin` + `meta` + `compiler/elaboration` |
| `WL-04-06` | P1 | `builtin.primitives` (`Symbol`/`void`/`never`/`NoReturn`/`*T`) | `primitives.fl` 定义 `NoReturn = .NoReturn'builtin`，而 `docs/control-flow/basic.md` 称 `never`；`*T` 有 `Clone`/`Copy`/`Eq`/`Hash` | 1. `NoReturn` 与 `never` 名称分裂待统一；2. `*T` 缺少完整指针固有方法（`addr`/`with_addr`/`map_addr`/`is_null`/`cast`/`offset`/`add`/`sub`/`wrapping_*`/`offset_from`/`read`/`write`/`copy_to`/`swap`/`replace`/`as_unaligned`/`slice_from_raw_parts`）；3. `Symbol` 与 `void` 补全方法及双阶段对齐 | `library/builtin` + `docs` |
| `WL-04-07` | P1 | `builtin` (`&r T` 可达性引用) | `docs/grammar/syntax` 与 `reachability.md` 定义 `&r T` 与 `Type.vars.ReachSet` | `docs/type/reachability-type.md` 与 `reachability-type-also.md` 为 0 字节空文件；`*T` 与 `&r T` 的协变/转换/借用边界、多集合 `&r1 &r2 T` 合并规则及 `str`/`Slice<T>` 返回依赖契约尚未在编译器端收口 | `docs` + `compiler/elaboration` |
| `WL-04-08` | P1 | `builtin` (`BitVec<N>`) | `primitives.fl` 绑定 `BitVec<n: Integer>`，支持 `'bitvec[lo..hi]` 视图及 `^attrs.packed.bit` 降码 | 缺标准固有操作库（`from_int`/`to_int`/`from_bytes`/`to_bytes`/`slice`/`concat`/`zero_extend`/`sign_extend`/`truncate`/`count_ones`/`leading_zeros`/`trailing_zeros`/`rotate_*`/`extract_bits`/`insert_bits`）及与 `Eq`/`Ord`/`Hash`/`Clone`/`Copy`/`Display`/`Debug` 完整接入 | `library/builtin` + `compiler/elaboration` |
| `WL-04-09` | P1 | `builtin` (任意位宽整数 `Int<W>`/`Uint<W>`) | `docs/type/user-defined-types/{structs,enums}.md` 使用 `u3` 与 `ArbitrarySizedInteger`；`primitives.fl` 尚未绑定；LLVM 后端仅接通至 128 位标量 | 1. `docs/others/arbitary-sized-integers.md` 为 0 字节空文件；2. 表面命名（`Int<W>`/`Uint<W>` vs `u<N>`/`i<N>` vs `ArbitrarySizedInteger<W, .signed>`）未定；3. >128 位 `ArbitrarySizedInteger` runtime 原语与算术指令生成未实现 | `docs` + `library/builtin` + `compiler` |
| `WL-04-10` | P0 | `builtin.types` (`Tuple<...Es>`) | `builtin/types/tuple.fl` 仅声明 `pub const Tuple: pure comptime fn<...Es: meta.List<Type>> -> Type = .Tuple'builtin;`；仅存在 `'tuple_enumerate`/`'tuple_indexed` 视图 | 1. 缺通用操作（`len`/`is_empty`/`get`/`first`/`last`/`head`/`tail`/`concat`/`split`/`reverse`/`zip`）；2. 缺针对 `Tuple<...Es>` 的通用 trait 实现（`Clone`/`Copy`/`Eq`/`Ord`/`Hash`/`Default`/`Debug`/`TuplePatternView`）；3. 缺多阶段 `meta.List` 操作、元组位置索引语法（`expr . int`）及不引入编译器特判的通用约束推理 | `library/builtin` + `meta` + `compiler` |
| `WL-04-11` | P0 | `builtin.primitives` (`Array<T, N>`) | 文档多处使用 `Array<i32, n>` 与 `[a, b]` 字面量，但 `primitives.fl` 与 `compiler/nir/src/types.rs` 无 `Array` 绑定或方法 | 1. 集中绑定 `Array<T, n: usize>` 及编译器数组类型表示；2. 全套方法（`len`/`is_empty`/`get`/`first`/`last`/`as_slice`/`as_mut_ptr`/`map`/`try_map`/`zip`/`each_ref`/`split_at`/`swap`/`reverse`/`fill`/`from_fn`/`try_from_fn`/`from_repeat`）；3. `Clone`/`Copy`/`Eq`/`Ord`/`Hash`/`Default`/`Debug`/`Drop`/`Index`/`IndexMut`/`Iterator`/`IntoIterator`/`ListPatternView`/`StableListPatternView` | `library/builtin` + `compiler` |
| `WL-04-12` | P0 | `builtin` / `std` (`Slice<T>`) | `std/collections_impl/slice.fl` 仅有 `from_raw_parts`/`empty`/`len`/`is_empty`/`get`/`first`/`last`/`slice`/`subslice_from`/`split_at`/`ListPatternView`/`Eq`/`Debug`/`Clone`/`Copy` | 1. 归属分裂（`std` vs `builtin`）；2. 仅实现不可变视图的一部分，缺 `split_first`/`split_last`/`chunks`/`chunks_exact`/`rchunks`/`windows`/`split`/`rsplit`/`splitn`/`binary_search`/`binary_search_by`/`sort`/`sort_by`/`sort_unstable*`/`swap`/`reverse`/`rotate_left`/`rotate_right`/`fill`/`fill_with`/`copy_from_slice`/`clone_from_slice`/`starts_with`/`ends_with`/`contains`/`iter`/`MemoryView` 可变分割与区间不重叠契约 | `library/builtin` + `std` + `compiler` |
| `WL-04-13` | P0 | `builtin.types` (`?T`) 与 `meta` | `OptionalOps` 与 `ComptimeOptionalOps` 仅 10 方法（`is_some`/`is_none`/`unwrap_or`/`unwrap_or_else`/`map`/`and_then`/`filter`/`inspect`/`or_else`/`zip_with`）；`Copy`/`Eq`/`Ord`/`Hash`/`Debug` 已实现 | 缺 `is_some_and`/`is_none_or`、`unwrap`/`expect`（`OptionalUnwrap` 原型见 `development/library-prototypes/builtin/option_unwrap.fl`）、`map_or`/`map_or_else`/`or`/`xor`/`zip`/`unzip`/`flatten`/`take`/`replace`/`get_or_insert`/`get_or_insert_with`/`ok_or`/`ok_or_else`/`as_ptr`/`copied`/`cloned`/`transpose_err`；缺非 `Copy` `?T` 的 `Clone`、`Default`、`Display` 及完整 `ComptimeOptionalOps`/`ComptimeOptionalUnwrap` | `library/builtin` + `meta` |
| `WL-04-14` | P0 | `builtin.types` (`!E T` 与 `![...] T`) 与 `meta` | `ErrorResultOps` 与 `ComptimeErrorResultOps` 仅 9 方法（`is_ok`/`is_err`/`inspect`/`inspect_err`/`map`/`map_err`/`and_then`/`or_else`/`unwrap_or_else`）；单一错误 `!E T` 已实现 `Copy`/`Eq`/`Ord`/`Hash`/`Debug` | 缺 `is_ok_and`/`is_err_and`、`ok`/`err`、`unwrap`/`expect`/`unwrap_err`/`expect_err`（`ErrorResultUnwrap`）、`unwrap_or`/`map_or`/`map_or_else`/`flatten`/`transpose_opt`/`into_ok`/`into_err`；**完全缺失**多错误行 `![E1, E2, ...Rest] T` 行辅助方法（`narrow`/`catch_one`/`map_row`/`widen_row`）及异构错误行 `Clone`/`Copy`/`Eq`/`Ord`/`Hash`/`Debug`；受编译器多错误 struct matching (`cc178adb`) 与 staging closure 阻塞 | `library/builtin` + `meta` + `compiler` |
| `WL-04-15` | P0 | `builtin.math` 与 `meta.math` | 仅覆盖 44 个 `target.Scalar` 标量操作；`compiler/nir/src/intrinsics.rs` `Operation` 仅支持单标量参数或 `(Scalar, i32)`（`powi`/`ldexp`）与内存原语 | 缺三角/双曲/特殊函数（`tan`/`asin`/`acos`/`atan`/`atan2`/`sinh`/`cosh`/`tanh`/`asinh`/`acosh`/`atanh`/`cbrt`/`hypot`/`expm1`/`log1p`/`erf`/`erfc`/`tgamma`/`lgamma`/`remquo`/`fdim`/`fma` 扩展），缺整数溢出/进位/宽乘/饱和乘 intrinsic，缺 `f16`/`bf16` 三角/指数/对数扩展；需扩展 `nir::intrinsics::Operation` 与 host/comptime 映射 | `library/builtin` + `meta` + `compiler/nir` |
| `WL-04-16` | P1 | `builtin.math` (SIMD / 向量数学) | `docs/pipeline-projection/target-validation-and-assembly.md` 展示 `X86Instructions.*'intrinsic`，但无统一 SIMD 类型与可移植向量数学家族 | 缺通用定宽向量类型（`Simd<T, N>` 或向量 intrinsic 家族 `vec_add`/`vec_fma`/`vec_sqrt`/`vec_sin`/`vec_reduce_*`）及 `build.target.features` 条件目标校验对接 | `library/builtin` + `compiler/nir` + `llvm` |
| `WL-04-17` | P1 | `builtin.math.consts` 与 `meta.math.consts` | 无任何数学常量模块；无 `Float` 高精度 `meta.math.consts` | 缺面向 `f16`/`bf16`/`f32`/`f64` 的 `builtin.math.consts` 与面向编译时 `Float` 的 `meta.math.consts`（`PI`/`TAU`/`E`/`FRAC_PI_*`/`LN_*`/`LOG2_*`/`LOG10_*`/`SQRT_2`/`FRAC_1_SQRT_2`/`PHI`/`EGAMMA` 等完整套件） | `library/builtin` + `meta` |
| `WL-04-18` | P0 | `builtin.iterator` (适配器与消费者) | 基于 `inline fn iterate` + `inline effect emit_item<T>`；已有 16 适配器与 16 消费者 | 1. 缺适配器：`intersperse`、`zip`、`peekable`、`cycle`、`rev`；缺边界 trait `DoubleEndedIterator`、`ExactSizeIterator`、`FusedIterator`；2. 缺消费者：`sum`、`product`（含 `Sum`/`Product` trait）、`partition`、`unzip`、`eq`/`ne`/`cmp`/`lt`/`le`/`gt`/`ge`/`lexicographical_cmp`；3. `min`/`max` 过度约束 `Item :- Copy`；4. 需彻底确立 push vs pull（`Continuation` / 协程悬挂 vs 步进游标）边界 | `library/builtin` + `compiler/continuation` |
| `WL-04-19` | P0 | `builtin.iterator` (生成器与原语实现) | 当前 `iterator.fl` 无任何生成器，也未给 `Range*`、`Array`、`str`、`?T` 提供 `Iterator`/`IntoIterator` | 缺 `once`、`once_with`、`empty`、`repeat`、`repeat_with`、`repeat_n`、`successors`、`from_fn` 完整生成器；缺 `Array<T, N>`、`Slice<T>`、`Range`/`RangeInclusive`、`str`（`bytes`/`chars`/`char_indices`/`lines`）与 `?T`/`!E T` 的标准迭代实现 | `library/builtin` |
| `WL-04-20` | P0 | `meta` (`PureComptimeIterator`) | `meta` 中 `List`/`Object`/`str` 仅各自硬编码 `foreach`/`fold`/`filter`，无统一 `PureComptimeIterator` | 缺与 runtime `Iterator` 方法名完全一致的 `pure comptime trait PureComptimeIterator`、`PureComptimeIntoIterator`、`PureComptimeFromIterator` 及其值引用适配器与消费者全集 | `library/meta` |
| `WL-04-21` | P1 | `builtin.fmt` 与 `meta.fmt` | 已有 `ByteWriter`、`Display`（含属性化 `FormatOptions`）、`Debug`（`write_debug`）；`FormatErr` 暂在 `std.fmt_impl.parse` | 1. **架构违规**：`Display` 含 `^attrs.comptime_phase comptime fn comptime_fmt` 违反双阶段同名规则（`AGENTS.md` §3）；2. 缺 `Formatter` 状态机与 `DebugStruct`/`DebugTuple`/`DebugList`/`DebugMap`/`DebugSet` 构建器；3. `LowerHex`/`UpperHex`/`Binary`/`Octal`/`LowerExp`/`UpperExp`/`Pointer` 独立 trait 与现有 `FormatOptions` 属性方案存在设计冲突待决；4. `FormatErr` 应归入 `builtin.fmt` | `library/builtin` + `meta` + `std` |
| `WL-04-22` | P1 | `builtin.fmt` (复合原语实现) | `Display` 与 `Debug` 仅实现标量及单层 `str`/`Symbol`；`f16`/`bf16`/`f32`/`f64` 仅实现 runtime `Display` | 缺 `f16`/`bf16`/`f32`/`f64` 的编译时格式化、`?T`、`!E T`、`![E1, E2, ...Rest] T`、`Tuple<...Es>`、`Array<T, N>`、`*T`、`BitVec<W>` 的 `Display`/`ComptimeDisplay`/`Debug` 实现 | `library/builtin` + `meta` |
| `WL-04-23` | P0 | `builtin.text` (`UTF-8` 与 `FromStr`) | `TextBytes` 仅 9 个字节操作；无 `Utf8Err`、`FromStr` 或文本解析 error 枚举 | 缺 `Utf8Err` 与 `str` 验证（`from_utf8`/`from_utf8_unchecked`）、UTF-8 码点与字素边界（与 `std.unicode` 分层）、`FromStr` trait（`!Self.Err Self`）与全套 `ParseBoolErr`/`ParseIntErr`/`ParseFloatErr`/`ParseCharErr` 原生实现 | `library/builtin` + `meta` |
| `WL-04-24` | P0 | `builtin.resources` | 已有单一全称 `impl Once for T where T {}`；已为标量、`*T`、`str`、`?T where T :- Copy`、`!E T where ... :- Copy` 实现 `Copy`/`Clone` | 1. 缺非 `Copy` 的 `?T` 与 `!E T` 的 `Clone`；2. 缺异构错误行 `![E1, E2, ...Rest] T`、`Array<T, N>`、`Tuple<...Es>` 的 `Clone`/`Copy`/`Drop`；3. 缺运行时依赖类型实例（如带环境参数 `.A`/`.allocator`）的自动 `Drop` 编译器支持（`issues/0003`） | `library/builtin` + `compiler/elaboration` |
| `WL-04-25` | P2 | `builtin.resources` (Pin-free 自引用与移动) | `moves-and-initialization.md` 规定值移动后原槽位失效、无隐式析构与隐式固定需求；`self-reference.md` 实为反身代词 `Self`/`Itself`/`itself` | 缺少正式文档化的 Pin-free 自引用类型交互规范（显式可移动性、`Moved`/reinit 规则、是否需要标记协议/禁止裸内部指针悬垂的契约与与 `verification` 分层） | `docs` + `builtin` + `verification` |
| `WL-04-26` | P0 | `builtin.mem` (安全与非安全载体类型) | `builtin/mem_impl/raw.fl` 仅有原始整数地址读写函数；`Layout` 暂置于 `std.mem_impl.layout` | 1. `Layout` 应提升至 `builtin.mem`（供 `std` 与无堆包复用）；2. 缺 `Align`（2 的幂次精化/结构）、`NonNull<T>`、`MaybeUninit<T>`、`ManuallyDrop<T>`、`TypeMarker<T>`（Flurry 风格零大小型标，取代或对齐 `PhantomData<T>`）；3. 缺 `size_of_val<T>(*T)` 与 `align_of_val<T>(*T)` | `library/builtin` + `std` + `compiler` |
| `WL-04-27` | P0 | `builtin.mem` (`raw`/`volatile`/`transmute`/`MemoryView`) | `raw.fl` 命名用 `copy_bytes`/`copy_bytes_nonoverlapping`，地址用 `usize` 而非 `*T`；`^attrs.mem.volatile` 已接通 struct/字段 | 1. 缺基于 `*T` 的 `raw_read`/`raw_write`/`copy`/`copy_nonoverlapping`/`write_bytes`/`zeroed`/`swap`/`replace`/`drop_in_place`/`read_unaligned`/`write_unaligned`；2. 缺 `volatile_read<T>`/`volatile_write<T>` 与纯度 P0 接线；3. 缺 `transmute<Src, Dst>` 编译时布局等大校验与对齐安全边界；4. 缺 `MemoryView`（`disjoint`/`within`/`separated_access`）与区域分离谓词形式化定义 | `library/builtin` + `compiler/nir` + `llvm` |
| `WL-04-28` | P0 | `library/language-items.conf` & 编译器 | 当前绑定 `copy`/`drop`/`splice`/`meta-list`/`tuple`/`ops`/`iterator`/`ord`/`eq`/`host-intrinsic.*`/`comptime-intrinsic.*` | 需扩充 `array-type`、`optional-ops-protocol`/`unwrap-protocol`、`error-ops-protocol`/`unwrap-protocol`、`from-str-protocol`、`pure-comptime-iterator-protocol`、`volatile-intrinsic`/`overflow-intrinsic` 等注册键与多参数 `nir::intrinsics::Operation` 支持 | `library/language-items.conf` + `compiler` |

---

## 3. 详细规格设计

> 本节中**所有**代码块均严格遵循 Flurry 正式语法：点分路径 `a.b.c`、原生 `?T`/`!E T`/`![...]/null`/`pat?`/`pat!`/`error pat`、`and`/`or`/`not` 逻辑运算、`--` 与 `{- ... -}` 注释、`.ref` / `.*` / `.as(T)` / `.dyn(Trait)` 后缀运算、多行 `where`/`requires` 后换行对齐 `{`、中缀 `+ - * / % < > <= >= == !=` 两侧空格、`.symbol'builtin` 仅出现在标注的集中绑定层、`PureComptime*` 仅使用 `self` 值引用语义。

### 3.1 原语类型与基础复合类型完整方法集

#### 3.1.1 集中绑定层规范（`library/builtin/primitives.fl` 演进）

现行 `library/builtin/primitives.fl` 需补齐 `Array`、`Slice`（若确定下沉）、`never` 别名及 `ArbitrarySizedInteger` 集中绑定，其余一切模块严禁再出现 `.symbol'builtin`：

```flurry
-- 仅允许出现在 library/builtin/primitives.fl（集中绑定层）
pub typealias bool = .bool'builtin;
pub typealias Type = .Type'builtin;
pub const Any: Type = .Any'builtin;
pub typealias Integer = .Integer'builtin;
pub typealias Float = .Float'builtin;
pub typealias BitVec<n: Integer> = .BitVec'builtin<n>;
pub typealias char = .char'builtin;
pub typealias str = .str'builtin;
pub typealias Symbol = .Symbol'builtin;
pub typealias void = .void'builtin;

-- NoReturn 与 never 规范统一（详审 §5 Q2）
pub typealias NoReturn = .NoReturn'builtin;
pub typealias never = NoReturn;

-- 新增集中绑定：定长内联数组、任意位宽整数家族（详审 §3.1.7 与 §3.1.9）
pub const Array: pure comptime fn<T: Type, n: usize> -> Type = .Array'builtin;
pub const Int: pure comptime fn<bits: Integer> -> Type = .ArbitraryInt'builtin;
pub const Uint: pure comptime fn<bits: Integer> -> Type = .ArbitraryUint'builtin;
```

#### 3.1.2 `bool` 完整方法集（`WL-04-01`）

`bool` 除已实现的 `Once`（经全称 `impl Once for T where T {}`）、`Clone`、`Copy`、`Eq`、`Ord`、`Hash`、`Display`、`Debug`、`Splice` 与原生逻辑运算 `not`/`and`/`or` 外（注：现有 `library/builtin/protocols/ops.fl` 中 `Not` / `BitAnd` / `BitOr` / `BitXor` 尚未全面接线，参考 `01` OQ-01-01），在 `builtin` 固有 `impl bool` 中提供以下**完整方法集**（含运行期与 `pure comptime` 阶段同步标注）：

```flurry
impl bool {
    -- 若为 true 则返回 some val?，否则返回 null（急切求值版与惰性闭包版）
    pub inline fn then_some(self, value: T) -> ?T where T {
        if self {
            return value;
        }
        null
    }

    pub inline fn then(self, f: %c fn() -> T) -> ?T
    where
        T,
        c: Type.vars.Closure,
    {
        if self {
            return f();
        }
        null
    }

    -- 若为 false 则将 err 提升为错误结果，否则返回 void
    pub inline fn ok_or(self, err: E) -> !E void where E {
        if self {
            return ();
        }
        error err
    }

    pub inline fn ok_or_else(self, f: %c fn() -> E) -> !E void
    where
        E,
        c: Type.vars.Closure,
    {
        if self {
            return ();
        }
        error f()
    }

    pub pure fn select(self, on_true: T, on_false: T) -> T where T {
        if self {
            return on_true;
        }
        on_false
    }

    pub pure fn to_u8(self) -> u8 {
        if self {
            return 1u8;
        }
        0u8
    }

    pub pure fn parse(input: str) -> !ParseBoolErr bool {
        builtin.text.parse_bool(input)
    }
}
```

#### 3.1.3 定长整数家族完整方法集（`WL-04-02`）

覆盖全部 12 个定长机器整数类型：**无符号** `{u8, u16, u32, u64, u128, usize}` 与 **有符号** `{i8, i16, i32, i64, i128, isize}`。

为避免遗漏并彻底消除重复逻辑，下面先给出完整方法契约矩阵与具体签名（以通用无符号示例 `u32` 与有符号示例 `i32` 全量展开，其余位宽 `W in {8, 16, 32, 64, 128, size}` 同形映射，零省略）：

1. **全量关联常量**（全 12 个整数类型均具备）：
   - `pub const MIN: Self`（无符号为 `0`，有符号为 `-2^(BITS - 1)`）
   - `pub const MAX: Self`（无符号为 `2^BITS - 1`，有符号为 `2^(BITS - 1) - 1`）
   - `pub const BITS: u32`（`8`, `16`, `32`, `64`, `128`, `usize.BITS` 由目标指针位宽决定）

2. **全量方法分组清单（全部 12 类型共有方法，零 `...`）**：
   - **位计数与位重排**：`count_ones(self) -> u32`, `count_zeros(self) -> u32`, `leading_ones(self) -> u32`, `trailing_ones(self) -> u32`, `leading_zeros(self) -> u32`, `trailing_zeros(self) -> u32`, `rotate_left(self, n: u32) -> Self`, `rotate_right(self, n: u32) -> Self`, `swap_bytes(self) -> Self`, `reverse_bits(self) -> Self`, `from_be(x: Self) -> Self`, `from_le(x: Self) -> Self`, `to_be(self) -> Self`, `to_le(self) -> Self`
   - **字节序数组转换**：`to_be_bytes(self) -> Array<u8, BYTES>`, `to_le_bytes(self) -> Array<u8, BYTES>`, `to_ne_bytes(self) -> Array<u8, BYTES>`, `from_be_bytes(bytes: Array<u8, BYTES>) -> Self`, `from_le_bytes(bytes: Array<u8, BYTES>) -> Self`, `from_ne_bytes(bytes: Array<u8, BYTES>) -> Self`（其中 `BYTES = BITS / 8`，对 `usize`/`isize` 通过 `comptime Type.size_of(usize)` 自动确定）
   - **Checked（返回 `?Self`）**：`checked_add`, `checked_sub`, `checked_mul`, `checked_div`, `checked_div_euclid`, `checked_rem`, `checked_rem_euclid`, `checked_neg`, `checked_shl`, `checked_shr`, `checked_pow`, `checked_ilog`, `checked_ilog2`, `checked_ilog10`, `checked_isqrt`, `checked_next_multiple_of`
   - **Wrapping（两补码回绕）**：`wrapping_add`, `wrapping_sub`, `wrapping_mul`, `wrapping_div`, `wrapping_div_euclid`, `wrapping_rem`, `wrapping_rem_euclid`, `wrapping_neg`, `wrapping_shl`, `wrapping_shr`, `wrapping_pow`
   - **Saturating（钳位饱和）**：`saturating_add`, `saturating_sub`, `saturating_mul`, `saturating_div`, `saturating_pow`
   - **Overflowing（返回 `Tuple<Self, bool>`）**：`overflowing_add`, `overflowing_sub`, `overflowing_mul`, `overflowing_div`, `overflowing_div_euclid`, `overflowing_rem`, `overflowing_rem_euclid`, `overflowing_neg`, `overflowing_shl`, `overflowing_shr`, `overflowing_pow`
   - **多精度进位/借位/双宽**：`carrying_add(self, rhs: Self, carry: bool) -> Tuple<Self, bool>`, `borrowing_sub(self, rhs: Self, borrow: bool) -> Tuple<Self, bool>`, `widening_mul(self, rhs: Self) -> Tuple<Self, UnsignedSelf>`, `carrying_mul(self, rhs: Self, carry: Self) -> Tuple<Self, Self>`
   - **欧几里得、整除向上/向下、幂与对数、中点**：`div_euclid`, `rem_euclid`, `div_ceil`, `div_floor`, `next_multiple_of`, `midpoint`, `pow`, `ilog`, `ilog2`, `ilog10`, `isqrt`, `min`, `max`, `clamp`, `from_str_radix(src: str, radix: u32) -> !ParseIntErr Self`

3. **无符号专属方法**（`u8, u16, u32, u64, u128, usize`）：
   - `is_power_of_two(self) -> bool`
   - `next_power_of_two(self) -> Self`
   - `checked_next_power_of_two(self) -> ?Self`
   - `abs_diff(self, other: Self) -> Self`
   - `checked_add_signed(self, rhs: SignedSelf) -> ?Self`
   - `wrapping_add_signed(self, rhs: SignedSelf) -> Self`
   - `saturating_add_signed(self, rhs: SignedSelf) -> Self`
   - `overflowing_add_signed(self, rhs: SignedSelf) -> Tuple<Self, bool>`
   - `checked_sub_signed(self, rhs: SignedSelf) -> ?Self`
   - `wrapping_sub_signed(self, rhs: SignedSelf) -> Self`
   - `saturating_sub_signed(self, rhs: SignedSelf) -> Self`
   - `overflowing_sub_signed(self, rhs: SignedSelf) -> Tuple<Self, bool>`
   - `cast_signed(self) -> SignedSelf`

4. **有符号专属方法**（`i8, i16, i32, i64, i128, isize`）：
   - `abs(self) -> Self`
   - `unsigned_abs(self) -> UnsignedSelf`
   - `checked_abs(self) -> ?Self`
   - `wrapping_abs(self) -> Self`
   - `saturating_abs(self) -> Self`
   - `overflowing_abs(self) -> Tuple<Self, bool>`
   - `abs_diff(self, other: Self) -> UnsignedSelf`
   - `signum(self) -> Self`
   - `is_positive(self) -> bool`
   - `is_negative(self) -> bool`
   - `checked_add_unsigned(self, rhs: UnsignedSelf) -> ?Self`
   - `wrapping_add_unsigned(self, rhs: UnsignedSelf) -> Self`
   - `saturating_add_unsigned(self, rhs: UnsignedSelf) -> Self`
   - `overflowing_add_unsigned(self, rhs: UnsignedSelf) -> Tuple<Self, bool>`
   - `checked_sub_unsigned(self, rhs: UnsignedSelf) -> ?Self`
   - `wrapping_sub_unsigned(self, rhs: UnsignedSelf) -> Self`
   - `saturating_sub_unsigned(self, rhs: UnsignedSelf) -> Self`
   - `overflowing_sub_unsigned(self, rhs: UnsignedSelf) -> Tuple<Self, bool>`
   - `cast_unsigned(self) -> UnsignedSelf`
   - `strict_abs_neg` 排除：溢出语义明确由 `checked_*`/`overflowing_*` 表达，默认纯算术运算符遵循 Flurry 语言溢出陷阱/回绕契约

完整具体签名示例（以 `u32` 和 `i32` 完整展示所有签名，其余位宽 `u8/u16/u64/u128/usize` 与 `i8/i16/i64/i128/isize` 按其 `Self`/`UnsignedSelf`/`SignedSelf`/`BYTES` 一一对应实例化）：

```flurry
-- 1) 无符号完整示例：impl u32（u8/u16/u64/u128/usize 同构展开，SignedSelf 对应 i8/i16/i32/i64/i128/isize）
impl u32 {
    pub const MIN: u32 = 0u32;
    pub const MAX: u32 = 4294967295u32;
    pub const BITS: u32 = 32u32;

    -- 位计数与字节序
    pub pure fn count_ones(self) -> u32 { builtin.math.count_ones(self) }
    pub pure fn count_zeros(self) -> u32 { Self.BITS - self.count_ones() }
    pub pure fn leading_ones(self) -> u32 { self.invert().leading_zeros() }
    pub pure fn trailing_ones(self) -> u32 { self.invert().trailing_zeros() }
    pub pure fn leading_zeros(self) -> u32 { builtin.math.leading_zeros(self) }
    pub pure fn trailing_zeros(self) -> u32 { builtin.math.trailing_zeros(self) }
    pub pure fn rotate_left(self, n: u32) -> Self { builtin.math.rotate_left(self, n) }
    pub pure fn rotate_right(self, n: u32) -> Self { builtin.math.rotate_right(self, n) }
    pub pure fn swap_bytes(self) -> Self { builtin.math.byte_swap(self) }
    pub pure fn reverse_bits(self) -> Self { builtin.math.bit_reverse(self) }
    pub pure fn from_be(x: Self) -> Self { builtin.math.target_from_be(x) }
    pub pure fn from_le(x: Self) -> Self { builtin.math.target_from_le(x) }
    pub pure fn to_be(self) -> Self { builtin.math.target_to_be(self) }
    pub pure fn to_le(self) -> Self { builtin.math.target_to_le(self) }
    pub pure fn to_be_bytes(self) -> Array<u8, 4> { builtin.math.int_to_be_bytes(self) }
    pub pure fn to_le_bytes(self) -> Array<u8, 4> { builtin.math.int_to_le_bytes(self) }
    pub pure fn to_ne_bytes(self) -> Array<u8, 4> { builtin.math.int_to_ne_bytes(self) }
    pub pure fn from_be_bytes(bytes: Array<u8, 4>) -> Self { builtin.math.int_from_be_bytes(bytes) }
    pub pure fn from_le_bytes(bytes: Array<u8, 4>) -> Self { builtin.math.int_from_le_bytes(bytes) }
    pub pure fn from_ne_bytes(bytes: Array<u8, 4>) -> Self { builtin.math.int_from_ne_bytes(bytes) }

    -- Checked
    pub pure fn checked_add(self, rhs: Self) -> ?Self {
        let (res, overflow) = self.overflowing_add(rhs);
        if overflow { return null; }
        res
    }
    pub pure fn checked_sub(self, rhs: Self) -> ?Self {
        let (res, overflow) = self.overflowing_sub(rhs);
        if overflow { return null; }
        res
    }
    pub pure fn checked_mul(self, rhs: Self) -> ?Self {
        let (res, overflow) = self.overflowing_mul(rhs);
        if overflow { return null; }
        res
    }
    pub pure fn checked_div(self, rhs: Self) -> ?Self {
        if rhs == 0u32 { return null; }
        self / rhs
    }
    pub pure fn checked_div_euclid(self, rhs: Self) -> ?Self { self.checked_div(rhs) }
    pub pure fn checked_rem(self, rhs: Self) -> ?Self {
        if rhs == 0u32 { return null; }
        self % rhs
    }
    pub pure fn checked_rem_euclid(self, rhs: Self) -> ?Self { self.checked_rem(rhs) }
    pub pure fn checked_neg(self) -> ?Self {
        if self == 0u32 { return 0u32; }
        null
    }
    pub pure fn checked_shl(self, rhs: u32) -> ?Self {
        if rhs >= Self.BITS { return null; }
        self.wrapping_shl(rhs)
    }
    pub pure fn checked_shr(self, rhs: u32) -> ?Self {
        if rhs >= Self.BITS { return null; }
        self.wrapping_shr(rhs)
    }
    pub pure fn checked_pow(self, exp: u32) -> ?Self { builtin.math.checked_uint_pow(self, exp) }
    pub pure fn checked_ilog(self, base: Self) -> ?u32 { builtin.math.uint_checked_ilog(self, base) }
    pub pure fn checked_ilog2(self) -> ?u32 {
        if self == 0u32 { return null; }
        (Self.BITS - 1u32) - self.leading_zeros()
    }
    pub pure fn checked_ilog10(self) -> ?u32 { builtin.math.uint_checked_ilog10(self) }
    pub pure fn checked_isqrt(self) -> ?Self { self.isqrt() }
    pub pure fn checked_next_multiple_of(self, rhs: Self) -> ?Self {
        if rhs == 0u32 { return null; }
        let rem = self % rhs;
        if rem == 0u32 { return self; }
        self.checked_add(rhs - rem)
    }
    pub pure fn checked_next_power_of_two(self) -> ?Self {
        if self == 0u32 { return 1u32; }
        let shift = (Self.BITS - 1u32) - (self - 1u32).leading_zeros();
        if shift >= Self.BITS - 1u32 and self > 1u32.wrapping_shl(Self.BITS - 1u32) {
            return null;
        }
        if self == 1u32 { return 1u32; }
        let bits = Self.BITS - (self - 1u32).leading_zeros();
        if bits >= Self.BITS { return null; }
        1u32.checked_shl(bits)
    }
    pub pure fn checked_add_signed(self, rhs: i32) -> ?Self { builtin.math.uint_checked_add_signed(self, rhs) }
    pub pure fn checked_sub_signed(self, rhs: i32) -> ?Self { builtin.math.uint_checked_sub_signed(self, rhs) }

    -- Wrapping
    pub pure fn wrapping_add(self, rhs: Self) -> Self {
        let (res, _) = self.overflowing_add(rhs);
        res
    }
    pub pure fn wrapping_sub(self, rhs: Self) -> Self {
        let (res, _) = self.overflowing_sub(rhs);
        res
    }
    pub pure fn wrapping_mul(self, rhs: Self) -> Self {
        let (res, _) = self.overflowing_mul(rhs);
        res
    }
    pub pure fn wrapping_div(self, rhs: Self) -> Self { self / rhs }
    pub pure fn wrapping_div_euclid(self, rhs: Self) -> Self { self / rhs }
    pub pure fn wrapping_rem(self, rhs: Self) -> Self { self % rhs }
    pub pure fn wrapping_rem_euclid(self, rhs: Self) -> Self { self % rhs }
    pub pure fn wrapping_neg(self) -> Self { 0u32.wrapping_sub(self) }
    pub pure fn wrapping_shl(self, rhs: u32) -> Self { builtin.math.wrapping_shl(self, rhs) }
    pub pure fn wrapping_shr(self, rhs: u32) -> Self { builtin.math.wrapping_lshr(self, rhs) }
    pub pure fn wrapping_pow(self, exp: u32) -> Self { builtin.math.wrapping_int_pow(self, exp) }
    pub pure fn wrapping_add_signed(self, rhs: i32) -> Self { self.wrapping_add(rhs.cast_unsigned()) }
    pub pure fn wrapping_sub_signed(self, rhs: i32) -> Self { self.wrapping_sub(rhs.cast_unsigned()) }

    -- Saturating
    pub pure fn saturating_add(self, rhs: Self) -> Self { builtin.math.saturating_add(self, rhs) }
    pub pure fn saturating_sub(self, rhs: Self) -> Self { builtin.math.saturating_sub(self, rhs) }
    pub pure fn saturating_mul(self, rhs: Self) -> Self {
        let (res, overflow) = self.overflowing_mul(rhs);
        if overflow { return Self.MAX; }
        res
    }
    pub pure fn saturating_div(self, rhs: Self) -> Self { self / rhs }
    pub pure fn saturating_pow(self, exp: u32) -> Self { builtin.math.saturating_uint_pow(self, exp) }
    pub pure fn saturating_add_signed(self, rhs: i32) -> Self {
        self.checked_add_signed(rhs) match {
            v? => v,
            null => if rhs < 0i32 { return Self.MIN; } else { return Self.MAX; },
        }
    }
    pub pure fn saturating_sub_signed(self, rhs: i32) -> Self {
        self.checked_sub_signed(rhs) match {
            v? => v,
            null => if rhs > 0i32 { return Self.MIN; } else { return Self.MAX; },
        }
    }

    -- Overflowing & Multi-precision
    pub pure fn overflowing_add(self, rhs: Self) -> Tuple<Self, bool> { builtin.math.uadd_with_overflow(self, rhs) }
    pub pure fn overflowing_sub(self, rhs: Self) -> Tuple<Self, bool> { builtin.math.usub_with_overflow(self, rhs) }
    pub pure fn overflowing_mul(self, rhs: Self) -> Tuple<Self, bool> { builtin.math.umul_with_overflow(self, rhs) }
    pub pure fn overflowing_div(self, rhs: Self) -> Tuple<Self, bool> { (self / rhs, false) }
    pub pure fn overflowing_div_euclid(self, rhs: Self) -> Tuple<Self, bool> { (self / rhs, false) }
    pub pure fn overflowing_rem(self, rhs: Self) -> Tuple<Self, bool> { (self % rhs, false) }
    pub pure fn overflowing_rem_euclid(self, rhs: Self) -> Tuple<Self, bool> { (self % rhs, false) }
    pub pure fn overflowing_neg(self) -> Tuple<Self, bool> { (self.wrapping_neg(), self != 0u32) }
    pub pure fn overflowing_shl(self, rhs: u32) -> Tuple<Self, bool> { (self.wrapping_shl(rhs), rhs >= Self.BITS) }
    pub pure fn overflowing_shr(self, rhs: u32) -> Tuple<Self, bool> { (self.wrapping_shr(rhs), rhs >= Self.BITS) }
    pub pure fn overflowing_pow(self, exp: u32) -> Tuple<Self, bool> { builtin.math.overflowing_uint_pow(self, exp) }
    pub pure fn overflowing_add_signed(self, rhs: i32) -> Tuple<Self, bool> { builtin.math.uint_overflowing_add_signed(self, rhs) }
    pub pure fn overflowing_sub_signed(self, rhs: i32) -> Tuple<Self, bool> { builtin.math.uint_overflowing_sub_signed(self, rhs) }
    pub pure fn carrying_add(self, rhs: Self, carry: bool) -> Tuple<Self, bool> { builtin.math.uint_carrying_add(self, rhs, carry) }
    pub pure fn borrowing_sub(self, rhs: Self, borrow: bool) -> Tuple<Self, bool> { builtin.math.uint_borrowing_sub(self, rhs, borrow) }
    pub pure fn widening_mul(self, rhs: Self) -> Tuple<Self, Self> { builtin.math.uint_widening_mul(self, rhs) }
    pub pure fn carrying_mul(self, rhs: Self, carry: Self) -> Tuple<Self, Self> { builtin.math.uint_carrying_mul(self, rhs, carry) }

    -- 数学与域操作
    pub pure fn is_power_of_two(self) -> bool {
        self != 0u32 and builtin.math.bit_and(self, self - 1u32) == 0u32
    }
    pub pure fn next_power_of_two(self) -> Self { builtin.math.unwrap_next_pow2(self) }
    pub pure fn div_euclid(self, rhs: Self) -> Self { self / rhs }
    pub pure fn rem_euclid(self, rhs: Self) -> Self { self % rhs }
    pub pure fn div_ceil(self, rhs: Self) -> Self {
        let q = self / rhs;
        let r = self % rhs;
        if r > 0u32 { return q + 1u32; }
        q
    }
    pub pure fn div_floor(self, rhs: Self) -> Self { self / rhs }
    pub pure fn next_multiple_of(self, rhs: Self) -> Self {
        let r = self % rhs;
        if r == 0u32 { return self; }
        self + (rhs - r)
    }
    pub pure fn midpoint(self, rhs: Self) -> Self {
        builtin.math.wrapping_lshr(builtin.math.bit_xor(self, rhs), 1u32)
            + builtin.math.bit_and(self, rhs)
    }
    pub pure fn pow(self, exp: u32) -> Self { builtin.math.unwrap_uint_pow(self, exp) }
    pub pure fn ilog(self, base: Self) -> u32 { builtin.math.uint_ilog(self, base) }
    pub pure fn ilog2(self) -> u32 { (Self.BITS - 1u32) - self.leading_zeros() }
    pub pure fn ilog10(self) -> u32 { builtin.math.uint_ilog10(self) }
    pub pure fn isqrt(self) -> Self { builtin.math.uint_isqrt(self) }
    pub pure fn abs_diff(self, other: Self) -> Self { if self >= other { return self - other; } other - self }
    pub pure fn cast_signed(self) -> i32 { builtin.math.bitcast_u32_to_i32(self) }
    pub pure fn clamp(self, lo: Self, hi: Self) -> Self { builtin.math.max(lo, builtin.math.min(self, hi)) }
    pub pure fn from_str_radix(src: str, radix: u32) -> !ParseIntErr Self { builtin.text.parse_int_radix<Self>(src, radix) }
}
```

有符号整数 `i32`（`i8/i16/i64/i128/isize` 同构展开）在共享上述 `count_ones..from_ne_bytes`、`checked_*`/`wrapping_*`/`saturating_*`/`overflowing_*`（除 `checked_neg(MIN) == null`、`wrapping_neg(MIN) == MIN`、`saturating_neg(MIN) == MAX`、`saturating_add`/`sub` 使用有符号 LLVM intrinsic、`saturating_mul` 对 `iMIN * -1` 钳位至 `MAX`、`overflowing_div(MIN, -1) == (MIN, true)`、`overflowing_div_euclid`/`rem_euclid` 处理正余数规范外）基础上，新增/特化全部有符号方法：

```flurry
impl i32 {
    pub const MIN: i32 = -2147483648i32;
    pub const MAX: i32 = 2147483647i32;
    pub const BITS: u32 = 32u32;

    -- 有符号专属完整签名（其余共享方法同 u32 对应 i32 契约）
    pub pure fn abs(self) -> Self { builtin.math.signed_abs_trap_min(self) }
    pub pure fn unsigned_abs(self) -> u32 { self.wrapping_abs().cast_unsigned() }
    pub pure fn checked_abs(self) -> ?Self {
        if self == Self.MIN { return null; }
        self.wrapping_abs()
    }
    pub pure fn wrapping_abs(self) -> Self { builtin.math.wrapping_abs(self) }
    pub pure fn saturating_abs(self) -> Self {
        if self == Self.MIN { return Self.MAX; }
        self.wrapping_abs()
    }
    pub pure fn overflowing_abs(self) -> Tuple<Self, bool> {
        (self.wrapping_abs(), self == Self.MIN)
    }
    pub pure fn abs_diff(self, other: Self) -> u32 {
        if self >= other {
            return self.wrapping_sub(other).cast_unsigned();
        }
        other.wrapping_sub(self).cast_unsigned()
    }
    pub pure fn signum(self) -> Self {
        if self > 0i32 { return 1i32; }
        if self < 0i32 { return -1i32; }
        0i32
    }
    pub pure fn is_positive(self) -> bool { self > 0i32 }
    pub pure fn is_negative(self) -> bool { self < 0i32 }
    pub pure fn div_euclid(self, rhs: Self) -> Self { builtin.math.sint_div_euclid(self, rhs) }
    pub pure fn rem_euclid(self, rhs: Self) -> Self { builtin.math.sint_rem_euclid(self, rhs) }
    pub pure fn div_ceil(self, rhs: Self) -> Self { builtin.math.sint_div_ceil(self, rhs) }
    pub pure fn div_floor(self, rhs: Self) -> Self { builtin.math.sint_div_floor(self, rhs) }
    pub pure fn checked_add_unsigned(self, rhs: u32) -> ?Self { builtin.math.sint_checked_add_unsigned(self, rhs) }
    pub pure fn wrapping_add_unsigned(self, rhs: u32) -> Self { self.cast_unsigned().wrapping_add(rhs).cast_signed() }
    pub pure fn saturating_add_unsigned(self, rhs: u32) -> Self { builtin.math.sint_saturating_add_unsigned(self, rhs) }
    pub pure fn overflowing_add_unsigned(self, rhs: u32) -> Tuple<Self, bool> { builtin.math.sint_overflowing_add_unsigned(self, rhs) }
    pub pure fn checked_sub_unsigned(self, rhs: u32) -> ?Self { builtin.math.sint_checked_sub_unsigned(self, rhs) }
    pub pure fn wrapping_sub_unsigned(self, rhs: u32) -> Self { self.cast_unsigned().wrapping_sub(rhs).cast_signed() }
    pub pure fn saturating_sub_unsigned(self, rhs: u32) -> Self { builtin.math.sint_saturating_sub_unsigned(self, rhs) }
    pub pure fn overflowing_sub_unsigned(self, rhs: u32) -> Tuple<Self, bool> { builtin.math.sint_overflowing_sub_unsigned(self, rhs) }
    pub pure fn cast_unsigned(self) -> u32 { builtin.math.bitcast_i32_to_u32(self) }
    pub pure fn carrying_add(self, rhs: Self, carry: bool) -> Tuple<Self, bool> { builtin.math.sint_carrying_add(self, rhs, carry) }
    pub pure fn borrowing_sub(self, rhs: Self, borrow: bool) -> Tuple<Self, bool> { builtin.math.sint_borrowing_sub(self, rhs, borrow) }
    pub pure fn widening_mul(self, rhs: Self) -> Tuple<u32, Self> { builtin.math.sint_widening_mul(self, rhs) }
    pub pure fn midpoint(self, rhs: Self) -> Self { builtin.math.sint_midpoint(self, rhs) }
}
```

#### 3.1.4 浮点家族完整方法集（`f16`, `bf16`, `f32`, `f64`）（`WL-04-03`）

```flurry
pub enum FpCategory {
    nan,
    infinite,
    zero,
    subnormal,
    normal,
}

impl Clone for FpCategory {
    fn clone(*self) -> Self { self.* }
}
impl Copy for FpCategory {}
```

全部 4 种浮点类型（`f16`/`bf16` 存储位宽 16 位，`Bits = u16`，`Bytes = 2`；`f32` `Bits = u32`，`Bytes = 4`；`f64` `Bits = u64`，`Bytes = 8`）共享完全一致的**全量固有常量与方法集合**（以 `f32` 与 `f16`/`bf16`/`f64` 参数值矩阵给出，零省略）：

| 常量 / 方法类别 | `f16` / `bf16` / `f32` / `f64` 完整成员清单 |
|---|---|
| IEEE 关联常量 | `RADIX: u32 = 2`, `MANTISSA_DIGITS: u32` (`11`/`8`/`24`/`53`), `DIGITS: u32` (`3`/`2`/`6`/`15`), `EPSILON: Self`, `MIN: Self`, `MAX: Self`, `MIN_POSITIVE: Self`, `MIN_EXP: i32` (`-13`/`-125`/`-125`/`-1021`), `MAX_EXP: i32` (`16`/`128`/`128`/`1024`), `MIN_10_EXP: i32`, `MAX_10_EXP: i32`, `NAN: Self`, `INFINITY: Self`, `NEG_INFINITY: Self` |
| IEEE 分类与符号 | `classify(self) -> FpCategory`, `is_nan(self) -> bool`, `is_infinite(self) -> bool`, `is_finite(self) -> bool`, `is_normal(self) -> bool`, `is_subnormal(self) -> bool`, `is_sign_positive(self) -> bool`, `is_sign_negative(self) -> bool`, `copysign(self, sign: Self) -> Self`, `signum(self) -> Self` |
| 取整与浮点分解 | `floor(self) -> Self`, `ceil(self) -> Self`, `round(self) -> Self`, `round_ties_even(self) -> Self`, `trunc(self) -> Self`, `fract(self) -> Self`, `ldexp(self, exp: i32) -> Self` |
| 比较、范围与插值 | `total_cmp(self, other: Self) -> Order`, `min(self, other: Self) -> Self`（IEEE `minimumnum`）, `max(self, other: Self) -> Self`（IEEE `maximumnum`）, `minimum(self, other: Self) -> Self`（propagates NaN）, `maximum(self, other: Self) -> Self`（propagates NaN）, `clamp(self, min: Self, max: Self) -> Self`, `lerp(self, other: Self, t: Self) -> Self`（等价精确插值 `fma(t, other - self, self)` 或单调边界保真），`midpoint(self, other: Self) -> Self` |
| 位表示与字节序 | `to_bits(self) -> Bits`, `from_bits(bits: Bits) -> Self`, `to_be_bytes(self) -> Array<u8, BYTES>`, `to_le_bytes(self) -> Array<u8, BYTES>`, `to_ne_bytes(self) -> Array<u8, BYTES>`, `from_be_bytes(bytes: Array<u8, BYTES>) -> Self`, `from_le_bytes(bytes: Array<u8, BYTES>) -> Self`, `from_ne_bytes(bytes: Array<u8, BYTES>) -> Self`, `next_up(self) -> Self`, `next_down(self) -> Self` |
| 初等与超越函数转发 | `abs`, `recip`, `mul_add` (`fma`), `div_euclid`, `rem_euclid`, `powi`, `powf`, `sqrt`, `cbrt`, `hypot`, `exp`, `exp2`, `expm1`, `exp_m1`, `ln`, `log`, `log2`, `log10`, `ln_1p`, `log1p`, `to_degrees`, `to_radians`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `sin_cos`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`, `erf`, `erfc`, `tgamma`, `lgamma`, `remquo`, `fdim` |

`f32` 全量签名声明示例（`f16`/`bf16`/`f64` 与之同构，常量取 IEEE 754-2008 binary16 / bfloat16 / binary64 精确值，`Bits` 分别为 `u16`/`u16`/`u64`，`BYTES` 分别为 `2`/`2`/`8`）：

```flurry
impl f32 {
    pub const RADIX: u32 = 2u32;
    pub const MANTISSA_DIGITS: u32 = 24u32;
    pub const DIGITS: u32 = 6u32;
    pub const EPSILON: f32 = 1.19209290e-7f32;
    pub const MIN: f32 = -3.40282347e+38f32;
    pub const MAX: f32 = 3.40282347e+38f32;
    pub const MIN_POSITIVE: f32 = 1.17549435e-38f32;
    pub const MIN_EXP: i32 = -125i32;
    pub const MAX_EXP: i32 = 128i32;
    pub const MIN_10_EXP: i32 = -37i32;
    pub const MAX_10_EXP: i32 = 38i32;
    pub const NAN: f32 = Self.from_bits(0x7fc00000u32);
    pub const INFINITY: f32 = Self.from_bits(0x7f800000u32);
    pub const NEG_INFINITY: f32 = Self.from_bits(0xff800000u32);

    pub pure fn is_nan(self) -> bool { builtin.math.isnan(self) }
    pub pure fn is_infinite(self) -> bool { builtin.math.isinf(self) }
    pub pure fn is_finite(self) -> bool { builtin.math.isfinite(self) }
    pub pure fn is_normal(self) -> bool { builtin.math.isnormal(self) }
    pub pure fn is_subnormal(self) -> bool { builtin.math.issubnormal(self) }
    pub pure fn is_sign_positive(self) -> bool { not self.is_sign_negative() }
    pub pure fn is_sign_negative(self) -> bool { builtin.math.signbit(self) }
    pub pure fn classify(self) -> FpCategory {
        if self.is_nan() { return FpCategory.nan; }
        if self.is_infinite() { return FpCategory.infinite; }
        if self == 0.0f32 { return FpCategory.zero; }
        if self.is_subnormal() { return FpCategory.subnormal; }
        FpCategory.normal
    }

    -- 位操作与 IEEE 754 totalOrder
    pub pure fn to_bits(self) -> u32 { builtin.math.f32_to_bits(self) }
    pub pure fn from_bits(bits: u32) -> f32 { builtin.math.f32_from_bits(bits) }
    pub pure fn to_be_bytes(self) -> Array<u8, 4> { self.to_bits().to_be_bytes() }
    pub pure fn to_le_bytes(self) -> Array<u8, 4> { self.to_bits().to_le_bytes() }
    pub pure fn to_ne_bytes(self) -> Array<u8, 4> { self.to_bits().to_ne_bytes() }
    pub pure fn from_be_bytes(bytes: Array<u8, 4>) -> Self { Self.from_bits(u32.from_be_bytes(bytes)) }
    pub pure fn from_le_bytes(bytes: Array<u8, 4>) -> Self { Self.from_bits(u32.from_le_bytes(bytes)) }
    pub pure fn from_ne_bytes(bytes: Array<u8, 4>) -> Self { Self.from_bits(u32.from_ne_bytes(bytes)) }

    pub pure fn total_cmp(self, other: Self) -> Order {
        -- 将 IEEE-754 符号-绝对值位模式线性映射为有符号可比较次序
        let mut_a = self.to_bits().cast_signed();
        let mut_b = other.to_bits().cast_signed();
        let a_mask = builtin.math.wrapping_lshr(
            builtin.math.wrapping_ashr(mut_a, 31i32).cast_unsigned(),
            1u32,
        ).cast_signed();
        let b_mask = builtin.math.wrapping_lshr(
            builtin.math.wrapping_ashr(mut_b, 31i32).cast_unsigned(),
            1u32,
        ).cast_signed();
        let a_key = builtin.math.bit_xor(mut_a, a_mask);
        let b_key = builtin.math.bit_xor(mut_b, b_mask);
        a_key.cmp(b_key)
    }

    pub pure fn clamp(self, min: Self, max: Self) -> Self {
        builtin.math.maximumnum(min, builtin.math.minimumnum(self, max))
    }
    pub pure fn lerp(self, other: Self, t: Self) -> Self {
        builtin.math.fma(t, other - self, self)
    }
    pub pure fn midpoint(self, other: Self) -> Self { builtin.math.f32_midpoint(self, other) }
    pub pure fn to_degrees(self) -> Self { self * (180.0f32 / builtin.math.consts.f32.PI) }
    pub pure fn to_radians(self) -> Self { self * (builtin.math.consts.f32.PI / 180.0f32) }
    pub pure fn abs(self) -> Self { builtin.math.abs(self) }
    pub pure fn copysign(self, sign: Self) -> Self { builtin.math.copysign(self, sign) }
    pub pure fn signum(self) -> Self { if self.is_nan() { return Self.NAN; } 1.0f32.copysign(self) }
    pub pure fn recip(self) -> Self { 1.0f32 / self }
    pub pure fn mul_add(self, b: Self, c: Self) -> Self { builtin.math.fma(self, b, c) }
    pub pure fn floor(self) -> Self { builtin.math.floor(self) }
    pub pure fn ceil(self) -> Self { builtin.math.ceil(self) }
    pub pure fn trunc(self) -> Self { builtin.math.trunc(self) }
    pub pure fn fract(self) -> Self { self - self.trunc() }
    pub pure fn round(self) -> Self { builtin.math.round(self) }
    pub pure fn round_ties_even(self) -> Self { builtin.math.roundeven(self) }
    pub pure fn min(self, other: Self) -> Self { builtin.math.minimumnum(self, other) }
    pub pure fn max(self, other: Self) -> Self { builtin.math.maximumnum(self, other) }
    pub pure fn minimum(self, other: Self) -> Self { builtin.math.minimum(self, other) }
    pub pure fn maximum(self, other: Self) -> Self { builtin.math.maximum(self, other) }
    pub pure fn sqrt(self) -> Self { builtin.math.sqrt(self) }
    pub pure fn cbrt(self) -> Self { builtin.math.cbrt(self) }
    pub pure fn hypot(self, other: Self) -> Self { builtin.math.hypot(self, other) }
    pub pure fn powi(self, n: i32) -> Self { builtin.math.powi(self, n) }
    pub pure fn powf(self, n: Self) -> Self { builtin.math.pow(self, n) }
    pub pure fn ldexp(self, exp: i32) -> Self { builtin.math.ldexp(self, exp) }
    pub pure fn exp(self) -> Self { builtin.math.exp(self) }
    pub pure fn exp2(self) -> Self { builtin.math.exp2(self) }
    pub pure fn expm1(self) -> Self { builtin.math.expm1(self) }
    pub pure fn ln(self) -> Self { builtin.math.log(self) }
    pub pure fn log(self, base: Self) -> Self { builtin.math.log(self) / builtin.math.log(base) }
    pub pure fn log2(self) -> Self { builtin.math.log2(self) }
    pub pure fn log10(self) -> Self { builtin.math.log10(self) }
    pub pure fn log1p(self) -> Self { builtin.math.log1p(self) }
    pub pure fn sin(self) -> Self { builtin.math.sin(self) }
    pub pure fn cos(self) -> Self { builtin.math.cos(self) }
    pub pure fn tan(self) -> Self { builtin.math.tan(self) }
    pub pure fn sin_cos(self) -> Tuple<Self, Self> { (builtin.math.sin(self), builtin.math.cos(self)) }
    pub pure fn asin(self) -> Self { builtin.math.asin(self) }
    pub pure fn acos(self) -> Self { builtin.math.acos(self) }
    pub pure fn atan(self) -> Self { builtin.math.atan(self) }
    pub pure fn atan2(self, other: Self) -> Self { builtin.math.atan2(self, other) }
    pub pure fn sinh(self) -> Self { builtin.math.sinh(self) }
    pub pure fn cosh(self) -> Self { builtin.math.cosh(self) }
    pub pure fn tanh(self) -> Self { builtin.math.tanh(self) }
    pub pure fn asinh(self) -> Self { builtin.math.asinh(self) }
    pub pure fn acosh(self) -> Self { builtin.math.acosh(self) }
    pub pure fn atanh(self) -> Self { builtin.math.atanh(self) }
    pub pure fn erf(self) -> Self { builtin.math.erf(self) }
    pub pure fn erfc(self) -> Self { builtin.math.erfc(self) }
    pub pure fn tgamma(self) -> Self { builtin.math.tgamma(self) }
    pub pure fn lgamma(self) -> Tuple<Self, i32> { builtin.math.lgamma(self) }
    pub pure fn remquo(self, rhs: Self) -> Tuple<Self, i32> { builtin.math.remquo(self, rhs) }
    pub pure fn fdim(self, rhs: Self) -> Self { builtin.math.fdim(self, rhs) }
    pub pure fn div_euclid(self, rhs: Self) -> Self { builtin.math.f32_div_euclid(self, rhs) }
    pub pure fn rem_euclid(self, rhs: Self) -> Self { builtin.math.f32_rem_euclid(self, rhs) }
    pub pure fn next_up(self) -> Self { builtin.math.f32_next_up(self) }
    pub pure fn next_down(self) -> Self { builtin.math.f32_next_down(self) }
}
```

#### 3.1.5 `char` 完整 Unicode 标量方法集（`WL-04-04`）

`library/builtin/numeric/conversions.fl` 已有 `from_u32`/`as_u32`/`len_utf8`/`is_ascii*`（11 项）/`to_ascii_lowercase`/`to_ascii_uppercase`/`eq_ignore_ascii_case`。完整补齐如下，形成无省略全集（Unicode 一般分类表与大小写映射表作为 `builtin.text.unicode_tables` 纯只读数据内置于 sysroot，同时提供 `pure comptime impl` / `ComptimeCharOps` 同名支持）：

```flurry
impl char {
    pub const MIN: char = '\x{0}';
    pub const MAX: char = '\x{10FFFF}';
    pub const REPLACEMENT_CHARACTER: char = '\x{FFFD}';
    pub const UNICODE_VERSION: Tuple<u8, u8, u8> = (16u8, 0u8, 0u8);

    -- 已有保留：from_u32, as_u32, len_utf8,
    -- is_ascii, is_ascii_uppercase, is_ascii_lowercase, is_ascii_alphabetic,
    -- is_ascii_digit, is_ascii_hexdigit, is_ascii_alphanumeric, is_ascii_whitespace,
    -- is_ascii_control, is_ascii_graphic, is_ascii_punctuation,
    -- to_ascii_lowercase, to_ascii_uppercase, eq_ignore_ascii_case

    pub pure fn len_utf16(self) -> usize {
        if self.as_u32() >= 0x10000u32 { return 2usize; }
        1usize
    }

    pub pure fn is_digit(self, radix: u32) -> bool { self.to_digit(radix).is_some() }

    pub pure fn to_digit(self, radix: u32) -> ?u32 {
        if radix < 2u32 or radix > 36u32 { return null; }
        let code = self.as_u32();
        let digit = when {
            code >= 0x30u32 and code <= 0x39u32 => code - 0x30u32,
            code >= 0x61u32 and code <= 0x7Au32 => (code - 0x61u32) + 10u32,
            code >= 0x41u32 and code <= 0x5Au32 => (code - 0x41u32) + 10u32,
            else => return null,
        }
        if digit < radix { return digit; }
        null
    }

    pub pure fn from_digit(num: u32, radix: u32) -> ?char {
        if radix < 2u32 or radix > 36u32 or num >= radix { return null; }
        if num < 10u32 { return char.from_u32(0x30u32 + num); }
        char.from_u32(0x61u32 + (num - 10u32))
    }

    -- Unicode 一般属性查询（UCD derived properties，运行时与纯编译时共用只读查找表）
    pub pure fn is_alphabetic(self) -> bool { builtin.text.unicode.is_alphabetic(self.as_u32()) }
    pub pure fn is_numeric(self) -> bool { builtin.text.unicode.is_numeric(self.as_u32()) }
    pub pure fn is_alphanumeric(self) -> bool { self.is_alphabetic() or self.is_numeric() }
    pub pure fn is_whitespace(self) -> bool { builtin.text.unicode.is_whitespace(self.as_u32()) }
    pub pure fn is_control(self) -> bool { builtin.text.unicode.is_control(self.as_u32()) }
    pub pure fn is_lowercase(self) -> bool { builtin.text.unicode.is_lowercase(self.as_u32()) }
    pub pure fn is_uppercase(self) -> bool { builtin.text.unicode.is_uppercase(self.as_u32()) }

    -- 完整 Unicode 大小写映射（1-to-many，返回最多 3 个码点的迭代器）
    pub pure fn to_lowercase(self) -> ToLowercaseIter { builtin.text.unicode.to_lowercase(self) }
    pub pure fn to_uppercase(self) -> ToUppercaseIter { builtin.text.unicode.to_uppercase(self) }

    -- UTF-8 与 UTF-16 编码到调用方提供的切片视图
    pub fn encode_utf8(self, dst: Slice<u8>) -> !EncodeUtf8Err str {
        builtin.text.encode_char_utf8(self, dst)
    }
    pub fn encode_utf16(self, dst: Slice<u16>) -> !EncodeUtf16Err Slice<u16> {
        builtin.text.encode_char_utf16(self, dst)
    }

    -- 转义迭代器（零分配，按 `Iterator` 逐字符产出）
    pub pure fn escape_debug(self) -> EscapeDebugIter { builtin.text.escape_char_debug(self) }
    pub pure fn escape_default(self) -> EscapeDefaultIter { builtin.text.escape_char_default(self) }
    pub pure fn escape_unicode(self) -> EscapeUnicodeIter { builtin.text.escape_char_unicode(self) }
}
```

#### 3.1.6 `str` 运行时固有方法集与 `meta` 同名编译时对偶（`WL-04-05`）

**架构清理**：当前 `builtin.text_impl.bytes.TextBytes` 仅为绕过旧限制的过渡承载 trait；正式设计中 `str` 具备以下完整的运行时方法（可由 `impl str` 固有实现，一旦编译器修复 `str` provenance 追踪与切片返回即可落地）以及在 `meta` 包中的 `pure comptime trait ComptimeTextOps`（使用**完全相同的无 `comptime_` 前缀方法名**与 **`self` 值引用语义**，无任何 `*self`/`.ref`/`.*`）：

- **运行期完整方法集合（`impl str`，支持 `&self` 可达性传递 `&self str`）**：
  - 长度与边界：`len(self) -> usize`（兼容已有 `byte_length(self) -> u64`）、`is_empty(self) -> bool`、`is_ascii(self) -> bool`、`is_char_boundary(self, index: usize) -> bool`、`as_bytes(self) -> &self Slice<u8>`、`as_ptr(self) -> *u8`、`byte(self, index: usize) -> ?u8`
  - 安全切分（所有返回值继承源 `str` 的可达性集合 `&self`）：`get(self, start: usize, end: usize) -> ?&self str`、`slice(self, start: usize, end: usize) -> ?&self str`、`split_at(self, mid: usize) -> ?Tuple<&self str, &self str>`、`split_at_checked(self, mid: usize) -> ?Tuple<&self str, &self str>`、`floor_char_boundary(self, index: usize) -> usize`、`ceil_char_boundary(self, index: usize) -> usize`
  - 迭代与视图：`bytes(self) -> BytesIter`（产出 `u8`）、`chars(self) -> CharsIter`（产出 `char`）、`char_indices(self) -> CharIndicesIter`（产出 `Tuple<usize, char>`）、`lines(self) -> LinesIter`（识别 `\n` 与 `\r\n`）、`split_whitespace(self) -> SplitWhitespaceIter`、`split_ascii_whitespace(self) -> SplitAsciiWhitespaceIter`
  - 前后缀与剥离：`starts_with(self, pat: str) -> bool`、`ends_with(self, pat: str) -> bool`、`strip_prefix(self, prefix: str) -> ?&self str`、`strip_suffix(self, suffix: str) -> ?&self str`
  - 修剪与分隔（基于单模式 `pat: str` 与通用 `P :- Pattern`）：`trim(self) -> &self str`、`trim_start(self) -> &self str`、`trim_end(self) -> &self str`、`trim_matches(self, pred: %c fn(char) -> bool) -> &self str`、`trim_start_matches(...)`、`trim_end_matches(...)`、`trim_ascii(self) -> &self str`、`trim_ascii_start(self) -> &self str`、`trim_ascii_end(self) -> &self str`、`split_once(self, sep: str) -> ?Tuple<&self str, &self str>`、`rsplit_once(self, sep: str) -> ?Tuple<&self str, &self str>`、`split(self, sep: str) -> SplitIter`、`rsplit(self, sep: str) -> RSplitIter`、`split_inclusive(self, sep: str) -> SplitInclusiveIter`、`splitn(self, n: usize, sep: str) -> SplitNIter`、`rsplitn(self, n: usize, sep: str) -> RSplitNIter`、`split_terminator(self, sep: str) -> SplitTerminatorIter`、`rsplit_terminator(self, sep: str) -> RSplitTerminatorIter`
  - 查找、匹配与解析：`contains(self, pat: str) -> bool`、`find(self, pat: str) -> ?usize`、`rfind(self, pat: str) -> ?usize`、`matches(self, pat: str) -> MatchesIter`、`rmatches(self, pat: str) -> RMatchesIter`、`match_indices(self, pat: str) -> MatchIndicesIter`、`rmatch_indices(self, pat: str) -> RMatchIndicesIter`、`eq_ignore_ascii_case(self, other: str) -> bool`、`parse(self) -> !T.as(FromStr).Err T where T :- FromStr`
  - 需分配的文本变换与编译期对称分配语义归属：
    - `replace(self, from: str, to: str)`、`replacen(...)`、`to_ascii_lowercase(self)`、`to_ascii_uppercase(self)`、`to_lowercase(self)`、`to_uppercase(self)`、`repeat(self, n: usize)` 需要运行期堆分配，因此**归属于 `std.string.String`（或接收 allocator 的 `builtin` 纯原型签名留给 `std`）**，不在无分配器 `builtin` 中硬编码默认堆分配！但在 `meta` 中由于 comptime VM 自管文本值生命周期，`meta.ComptimeTextOps` 直接提供编译时值返回的 `replace`/`replacen`/`concat`/`to_ascii_lowercase`/`to_ascii_uppercase`！
  - 编译时专属对偶（`library/meta/data/text.fl` 中的 `pure comptime trait ComptimeTextOps`）：
    - 使用**与运行期完全相同的成员名**（`byte_length`, `len`, `byte`, `is_empty`, `is_ascii`, `is_char_boundary`, `slice`, `split_at`, `starts_with`, `ends_with`, `strip_prefix`, `strip_suffix`, `find`, `rfind`, `contains`, `split_once`, `rsplit_once`, `trim`, `trim_start`, `trim_end`, `trim_ascii`, `eq_ignore_ascii_case`, `lines`, `bytes`, `chars`, `char_indices`, `split`, `concat`, `replace`, `replacen`, `to_ascii_lowercase`, `to_ascii_uppercase`, `parse<T>`），全部采用 `self` 值引用语义。

**编译器前置阻塞（关联 `issues/0003` 与 `development/repros/native-text-method-provenance.fl`）**：当前编译器在返回 `&self str` 或内部指向 `str` 数据区的字节操作时无法正确保留/推导 `pointer_input_layout` provenance，导致原生 `str` 方法内部读取 `self` 地址触发错判；该阻塞是解锁整个 `WL-04-05` 的唯一前置。

#### 3.1.7 `Symbol`、`void`、`NoReturn`/`never`、`*T`、`&r T` 与 `BitVec<N>`（`WL-04-06` ~ `WL-04-08`）

1. **`Symbol`、`void`、`never` / `NoReturn`**：
   - `Symbol`：保持名义内部化（interned）标识符语义。固有方法补全 `as_str(self) -> str`（零分配返回指向静态内部符号表的只读文本视图，待 `WL-04-07` 静态可达性集合规约确定后可进一步附加具名静态可达性集合）与 `len(self) -> usize`、`is_empty(self) -> bool`（补齐目前分散在 `bindings.fl` 的 `symbol_byte_length`/`symbol_byte` 至固有 `impl Symbol`，并保留 `Eq`/`Ord`/`Hash`/`Clone`/`Copy`/`Display`/`Debug`）。
   - `void`：单值 `()` 零大小类型。保留 `Clone`/`Copy`/`Eq`/`Ord`/`Hash`/`Default`/`Debug`/`Splice`（`fn splice(comptime data: void) -> void`），不额外堆积无意义方法。
   - **`NoReturn` vs `never`**：正式词法 doc（`docs/basics/lexical.md:95`）与现行 `primitives.fl` 使用 `NoReturn`，而控制流 doc（`docs/control-flow/basic.md`）使用 `never`。二者指代同一底类型（bottom type `.NoReturn'builtin`）。本规格暂时提供 `pub typealias never = NoReturn;` 统一别名，并在 §5 Q2 提交作者最终裁决以哪个名称作为唯一正式名称。`NoReturn`/`never` 支持隐式收敛转换（coercion）至任意期望类型 `T`，并在控制流分析中不要求 `Drop` 或 `Copy`；验证层不可达分支沿用已有 `assumes false;`（静态断言不可达），运行期新增 `unsafe fn NoReturn.unreachable_unchecked() -> NoReturn`（与普通 `abort()` trap 区分）及 `impl Clone/Eq/Ord/Hash/Debug for never`（不可达上下文调用安全，绝不可在安全代码中构造实例）。

2. **裸指针 `*T` 完整方法集（`WL-04-06`）**：
   Flurry 的 `*T` 不区分 `*const`/`*mut`，读写许可由可达性与 `unsafe` 契约控制（`docs/resource-management/pointers.md`）。在 `impl *T where T` 中提供完整指针操作集（全部按 `unsafe` 或安全元信息精确分级）：

```flurry
impl *T where T {
    -- 安全元数据与地址投影
    pub pure fn addr(self) -> usize { builtin.mem.ptr_addr(self) }
    pub pure fn is_null(self) -> bool { self.addr() == 0usize }
    pub pure fn cast(self) -> *U where U { builtin.mem.ptr_cast<T, U>(self) }

    -- provenance-preserving 地址重建（遵守 strict provenance）
    pub pure fn with_addr(self, addr: usize) -> *T { builtin.mem.ptr_with_addr(self, addr) }
    pub pure fn map_addr(self, f: %c fn(usize) -> usize) -> *T
    where
        c: Type.vars.Closure,
    {
        self.with_addr(f(self.addr()))
    }
    pub pure fn without_provenance(addr: usize) -> *T { builtin.mem.ptr_without_provenance<T>(addr) }

    -- 指针算术（有边界条件为 unsafe；回绕语义允许纯安全算术，但禁止任意解引用）
    pub unsafe fn offset(self, count: isize) -> *T { builtin.mem.ptr_offset(self, count) }
    pub unsafe fn add(self, count: usize) -> *T { builtin.mem.ptr_add(self, count) }
    pub unsafe fn sub(self, count: usize) -> *T { builtin.mem.ptr_sub(self, count) }
    pub pure fn wrapping_offset(self, count: isize) -> *T { builtin.mem.ptr_wrapping_offset(self, count) }
    pub pure fn wrapping_add(self, count: usize) -> *T { builtin.mem.ptr_wrapping_add(self, count) }
    pub pure fn wrapping_sub(self, count: usize) -> *T { builtin.mem.ptr_wrapping_sub(self, count) }
    pub unsafe fn byte_offset(self, count: isize) -> *T { self.cast<u8>().offset(count).cast<T>() }
    pub unsafe fn byte_add(self, count: usize) -> *T { self.cast<u8>().add(count).cast<T>() }
    pub unsafe fn byte_sub(self, count: usize) -> *T { self.cast<u8>().sub(count).cast<T>() }
    pub pure fn wrapping_byte_offset(self, count: isize) -> *T { self.cast<u8>().wrapping_offset(count).cast<T>() }
    pub unsafe fn offset_from(self, origin: *T) -> isize { builtin.mem.ptr_offset_from(self, origin) }
    pub unsafe fn byte_offset_from(self, origin: *U) -> isize where U { builtin.mem.ptr_byte_offset_from(self, origin) }
    pub pure fn align_offset(self, align: usize) -> usize { builtin.mem.ptr_align_offset(self, align) }

    -- 读写、交换、析构原语（全部标注 unsafe）
    pub unsafe fn read(self) -> T { builtin.mem.read<T>(self) }
    pub unsafe fn read_volatile(self) -> T { builtin.mem.volatile_read<T>(self) }
    pub unsafe fn read_unaligned(self) -> T { builtin.mem.read_unaligned<T>(self) }
    pub unsafe fn write(self, value: T) { builtin.mem.write<T>(self, value) }
    pub unsafe fn write_volatile(self, value: T) { builtin.mem.volatile_write<T>(self, value) }
    pub unsafe fn write_unaligned(self, value: T) { builtin.mem.write_unaligned<T>(self, value) }
    pub unsafe fn write_bytes(self, val: u8, count: usize) { builtin.mem.write_bytes<T>(self, val, count) }
    pub unsafe fn copy_to(self, dst: *T, count: usize) { builtin.mem.copy<T>(self, dst, count) }
    pub unsafe fn copy_to_nonoverlapping(self, dst: *T, count: usize) { builtin.mem.copy_nonoverlapping<T>(self, dst, count) }
    pub unsafe fn copy_from(self, src: *T, count: usize) { builtin.mem.copy<T>(src, self, count) }
    pub unsafe fn copy_from_nonoverlapping(self, src: *T, count: usize) { builtin.mem.copy_nonoverlapping<T>(src, self, count) }
    pub unsafe fn swap(self, other: *T) { builtin.mem.swap<T>(self, other) }
    pub unsafe fn replace(self, src: T) -> T { builtin.mem.replace<T>(self, src) }
    pub unsafe fn drop_in_place(self) { builtin.mem.drop_in_place<T>(self) }

    -- 受检对齐与非空视图桥接
    pub pure fn is_aligned(self) -> bool {
        let align = Type.align_of(T);
        (self.addr() % align) == 0usize
    }
    pub pure fn as_non_null(self) -> ?NonNull<T> { NonNull.new(self) }
    pub unsafe fn as_ref_unchecked(self) -> *T { self }
}
```

3. **可达性限定类型 `&r T`（`WL-04-07`）**：
   - 依据 `docs/grammar/syntax:211` 与 `docs/resource-management/reachability.md`，`&r T` 不是引用类型，而是**对值类型 `T` 附加外部有效性约束 `r: Type.vars.ReachSet`**；多个限定 `&r1 &r2 T` 表示合并两个可达性集合。
   - `&r T` 不对应任何额外运行时存储开销（与 `T` 布局同构），也不在 `builtin` 中单独制造 `Ref<T>` 包装类型；但它直接影响 `str`、`Slice<T>` 与 `*T` 的返回类型契约（如 `fn first_word(s: &r str) -> &r str`、`fn get(*self, idx: usize) -> &self *T`）。由于 `docs/type/reachability-type.md` 当前仍为 0 字节，在 §5 Q3 明确提交可达性子类型化与默认 `'static` 静态集合规约待决项。

4. **`BitVec<N>` 固有方法集（`WL-04-08`）**：
   - 基于 `docs/control-flow/pattern-matching/bitvec.md`、`docs/program-synthesis/bitvec-algebra.md` 与 `docs/type/user-defined-types/structs.md`（支持 `^attrs.packed.bit`、`value'bitvec[lo..hi]`、位模式拼接与局部写回语义）：
   - 在 `impl BitVec<N> where N: Integer, requires N >= 0` 中提供完整操作集：
     - 常量：`pub const WIDTH: Integer = N;`、`pub const ZERO: Self`、`pub const ALL_ONES: Self`
     - 切片与拼接（均受编译期/约束边界检验 `0 <= lo <= hi <= N`）：`slice(self, comptime lo: Integer, comptime hi: Integer) -> BitVec<hi - lo>`、`concat(self, rhs: BitVec<M>) -> BitVec<N + M> where M: Integer, requires M >= 0`、`repeat(self) -> BitVec<N * K> where K: Integer, requires K >= 1`
     - 位宽扩展与截断：`zero_extend(self) -> BitVec<M> where M: Integer, requires M >= N`、`sign_extend(self) -> BitVec<M> where M: Integer, requires M >= N and N >= 1`、`truncate(self) -> BitVec<M> where M: Integer, requires 0 <= M and M <= N`
     - 位查询与移位/旋转：`get_bit(self, idx: usize) -> ?bool`、`with_bit(self, idx: usize, bit: bool) -> ?Self`、`count_ones(self) -> usize`、`count_zeros(self) -> usize`、`leading_zeros(self) -> usize`、`trailing_zeros(self) -> usize`、`rotate_left(self, n: usize) -> Self`、`rotate_right(self, n: usize) -> Self`、`reverse_bits(self) -> Self`
     - 与整数互转：`from_u64(v: u64) -> Self`（高位截断/零扩展）、`from_i64(v: i64) -> Self`（符号扩展/截断）、`to_u64_trunc(self) -> u64`、`to_u64_checked(self) -> ?u64`、`to_i64_sign_ext(self) -> i64`、`to_le_bytes(self) -> Array<u8, (N + 7) / 8>`、`to_be_bytes(self) -> Array<u8, (N + 7) / 8>`
     - 标准协议：`Clone`、`Copy`、`Eq`、`Ord`（无符号位字典序）、`Hash`、`Default`、`Debug`（格式如 `0b...` 或 `BitVec<N>(0x...)`），原生按位逻辑使用 `not`/`and`/`or`（待与 `01` OQ-01-01 协同决定 `ops.fl` 中 `Not`/`BitAnd`/`BitOr`/`BitXor`/`Shl`/`Shr` 是否接入或废弃）

#### 3.1.8 任意位宽整数 `Int<W>` / `Uint<W>`（`ArbitrarySizedInteger`）（`WL-04-09`）

`docs/type/user-defined-types/enums.md:33` 与 `structs.md:169` 已提及无符号与有符号 `ArbitrarySizedInteger` / `u3` 家族，但 `docs/others/arbitary-sized-integers.md` 尚为空白，`ai_dev_logs/codebase_audit_report_2026-11-22.md:88` 明确记录 >128 位的 runtime 原语尚未接入 LLVM。设计规范如下：
1. **类型构造器（不新增特设关键字或编译器专有类型槽）**：
   - 在 `builtin.primitives` 定义统一构造器 `ArbitrarySizedInteger<bits: Integer, .signed: bool = false> -> Type`，并导出常规泛型别名 `Uint<bits: Integer>`（等价 `.signed = false`）与 `Int<bits: Integer>`（等价 `.signed = true`），以及 `bits in 1..=128` 或任意 `bits >= 1` 的通用实例；是否在词法/prelude 为常见位宽或一般 `u<N>`/`i<N>` 提供语法糖归 §5 Q4 裁决。
   - **位数 0 与位数边界**：`Uint<0>` / `Int<0>` 禁止实例化（`requires bits >= 1`）；`bits == 1` 时 `Uint<1>` 取值 `{0, 1}`，`Int<1>` 取值 `{-1, 0}`。
2. **与 `BitVec<W>` 的清晰分工**：
   - `BitVec<W>` 是**纯位向量几何视图**（支持位拼接、局部切片写入 `bits[lo..hi]`、位模式解构，不预设算术溢出语义）；`Uint<W>` / `Int<W>` 是**算术整数族**（具备两补码算术、`+ - * / %`、比较、溢出检查与与定长 `u8..u128`/`i8..i128` 相同的全部 `checked_*`/`wrapping_*`/`saturating_*`/`overflowing_*`/`carrying_*` 方法集，以及按最小 `ceil(W / 8)` 或目标 ABI 齐位宽降码为 LLVM `i<W>` 类型）。
3. **LLVM 后端与 runtime ABI 契约**：
   - `W <= 128` 直接映射为 LLVM 整数类型 `i<W>`，并在 C ABI 边界按目标平台规则（例如 8/16/32/64/128 对齐零扩展或符号扩展）传递；
   - `W > 128` 映射为 LLVM 任意位宽 `i<W>`（并由 LLVM DAG/Legalizer 降码为多 limb 算术或内置 compiler-rt `mul`/`udiv`/`sdiv` 调用，禁止静默截断为 `i128`）。

#### 3.1.9 `Tuple<...Es>`、`Array<T, N>` 与 `Slice<T>`（`WL-04-10` ~ `WL-04-12`）

##### A. `Tuple<...Es>` — 彻底遵循通用机制（零特判，Zero Special Compiler Rules）

依据 `AGENTS.md` §5 与 `docs/traits/implementation-resolution.md:217-223`：
- `Tuple` 仅在 `builtin/types/tuple.fl` 通过集中绑定定义一次：`pub const Tuple: pure comptime fn<...Es: meta.List<Type>> -> Type = .Tuple'builtin;`。
- Normal-form 头绝不自动展开，**禁止**编译器将 `meta.List<Type>` 硬编码为专用参数槽或为 `Tuple` 新增任何独立参数类别。当手中已有 `Es: meta.List<Type>` 值时，**必须**优先使用具名赋值 `Tuple<.Es = Es>`，避免无谓拆包重组 `Tuple<...Es>`（仅当需要与额外类型拼接 `Tuple<Head, ...Tail>` 时使用展开）。
- **核心通用操作集（全量清单，依赖通用 `'tuple_indexed` / `'tuple_enumerate` 视图、通用多阶段 `meta.List` 操作及编译器通用位置投影支持）**：

```flurry
pure comptime fn TupleElemType(es: meta.List<Type>, idx: usize) -> Type
where
    requires Integer.from_usize(idx) < es.length(),
{
    es.get(Integer.from_usize(idx)) match {
        elem? => elem,
        null => Any,
    }
}

pure comptime fn tuple_slice_types(
    es: meta.List<Type>,
    start: usize,
    end: usize,
) -> meta.List<Type>
where
    requires start <= end and Integer.from_usize(end) <= es.length(),
{
    es.slice(Integer.from_usize(start), Integer.from_usize(end)) match {
        sliced? => sliced,
        null => [],
    }
}

impl Tuple<.Es = Es>
where
    Es: meta.List<Type>,
{
    pub const LEN: usize = Es.length().to_usize();

    pub pure fn len(self) -> usize { Self.LEN }
    pub pure fn is_empty(self) -> bool { Self.LEN == 0usize }

    -- 编译时已知下标元素读取与不可变/可变指针视图
    -- 注：返回类型通过一般 meta.List 操作计算得出，编译器需支持通用 `TupleGet` 投影与移动跟踪
    pub pure fn get(self, comptime idx: usize) -> TupleElemType(Es, idx)
    where
        requires idx < Self.LEN,
    {
        builtin.types.tuple_get(idx, self)
    }

    pub pure fn first(self) -> TupleElemType(Es, 0usize)
    where
        requires Self.LEN >= 1usize,
    {
        self.get(0usize)
    }

    pub pure fn last(self) -> TupleElemType(Es, Self.LEN - 1usize)
    where
        requires Self.LEN >= 1usize,
    {
        self.get(Self.LEN - 1usize)
    }

    -- 解构分离与拼接（构造新的正常型 Tuple<.Es = ResultEs>）
    pub fn head_tail(
        self,
    ) -> Tuple<TupleElemType(Es, 0usize), Tuple<.Es = tuple_slice_types(Es, 1usize, Self.LEN)>>
    where
        requires Self.LEN >= 1usize,
    {
        builtin.types.tuple_split_head(self)
    }

    pub fn concat(self, rhs: Tuple<.Es = RightEs>) -> Tuple<.Es = Es.concat(RightEs)>
    where
        RightEs: meta.List<Type>,
    {
        builtin.types.tuple_concat(self, rhs)
    }

    pub fn push_back(self, value: V) -> Tuple<.Es = Es.concat([V])> where V {
        builtin.types.tuple_concat(self, (value,))
    }

    pub fn split_at(
        self,
        comptime mid: usize,
    ) -> Tuple<Tuple<.Es = tuple_slice_types(Es, 0usize, mid)>, Tuple<.Es = tuple_slice_types(Es, mid, Self.LEN)>>
    where
        requires mid <= Self.LEN,
    {
        builtin.types.tuple_split_at(mid, self)
    }
}
```

- **通用协议实现（利用 `'tuple_indexed` 在实例内展开，全成员严格逐槽检查，绝不靠特判越权）**：
  - `impl Clone for Tuple<.Es = Es> where Es: meta.List<Type>, requires Es.all(|E| E :- Clone) { ... }`（按槽位正向顺序逐一 `.clone()` 并构造目标元组；中途若存在未来带异常效应，已构造前缀按 Drop elaboration 自动逆序回收）
  - `impl Copy for Tuple<.Es = Es> where Es: meta.List<Type>, requires Es.all(|E| E :- Copy) {}`
  - `impl Eq for Tuple<.Es = Es> where Es: meta.List<Type>, requires Es.all(|E| E :- Eq) { ... }`（`'tuple_indexed` 短路比较所有槽位；零元组 `Tuple<>` 恒返 `true`）
  - `impl Ord for Tuple<.Es = Es> where Es: meta.List<Type>, requires Es.all(|E| E :- Ord) { ... }`（字典序逐槽 `cmp`；遇到非 `.equal` 立即返回）
  - `impl Hash for Tuple<.Es = Es> where Es: meta.List<Type>, requires Es.all(|E| E :- Hash) { ... }`
  - `impl Default for Tuple<.Es = Es> where Es: meta.List<Type>, requires Es.all(|E| E :- Default) { ... }`
  - `impl Debug for Tuple<.Es = Es> where Es: meta.List<Type>, requires Es.all(|E| E :- Debug) { ... }`（调用 `DebugTuple` 构建器；单元素元组输出 `(x,)`）
  - **编译器通用前提**：当前尚缺少在 `where` 子句中将高阶 `meta.List` 谓词 `Es.all(...)` 展开为一组同质/异质 trait bound 的通用机制（同时支持 `TuplePatternView`、`VariadicCollector` 以及词法层 `t.0` / 位置投影），这是解除 `Tuple` 通用 trait 实现的唯一通用机制要求（绝不为 `Tuple` 写死特例 pass）。

##### B. `Array<T, N>` 完整固有方法集与协议实现（`WL-04-11`）

```flurry
impl Array<T, n>
where
    T,
    n: usize,
{
    pub const LEN: usize = n;

    pub pure fn len(*self) -> usize { n }
    pub pure fn is_empty(*self) -> bool { n == 0usize }

    pub pure fn as_slice(*self) -> &self Slice<T> {
        unsafe { Slice.from_raw_parts(self.as_ptr().addr(), n) }
    }
    pub pure fn as_ptr(*self) -> *T { builtin.mem.array_as_ptr<T, n>(self) }
    pub pure fn as_mut_ptr(*self) -> *T { builtin.mem.array_as_ptr<T, n>(self) }

    pub pure fn get(*self, index: usize) -> ?&self *T {
        if index >= n { return null; }
        unsafe { self.as_ptr().add(index) }
    }
    pub pure fn first(*self) -> ?&self *T { self.get(0usize) }
    pub pure fn last(*self) -> ?&self *T {
        if n == 0usize { return null; }
        self.get(n - 1usize)
    }

    pub fn swap(*self, i: usize, j: usize) -> ?void {
        if i >= n or j >= n { return null; }
        if i != j {
            unsafe { self.as_ptr().add(i).swap(self.as_ptr().add(j)); }
        }
        ()
    }

    pub fn reverse(*self) {
        var lo: usize = 0usize;
        var hi: usize = n;
        while lo < hi {
            hi = hi - 1usize;
            if lo < hi {
                unsafe { self.as_ptr().add(lo).swap(self.as_ptr().add(hi)); }
            }
            lo = lo + 1usize;
        }
    }

    pub fn fill(*self, value: T) where requires T :- Clone {
        var i: usize = 0usize;
        while i < n {
            unsafe {
                self.as_ptr().add(i).drop_in_place();
                self.as_ptr().add(i).write(value.clone());
            }
            i = i + 1usize;
        }
    }

    pub fn fill_with(*self, f: %c fn() -> T) where c: Type.vars.Closure {
        var i: usize = 0usize;
        while i < n {
            unsafe {
                self.as_ptr().add(i).drop_in_place();
                self.as_ptr().add(i).write(f());
            }
            i = i + 1usize;
        }
    }

    -- 按元素消耗并映射（使用 MaybeUninit 保障中途异常/早退时无泄漏与无 double-drop）
    pub fn map(self, f: %c fn(T) -> U) -> Array<U, n>
    where
        U,
        c: Type.vars.Closure,
    {
        builtin.mem.array_map<T, U, n, c>(self, f)
    }

    pub fn try_map(self, f: %c fn(T) -> !E U) -> !E Array<U, n>
    where
        U,
        E,
        c: Type.vars.Closure,
    {
        builtin.mem.array_try_map<T, U, E, n, c>(self, f)
    }

    pub fn zip(self, other: Array<U, n>) -> Array<Tuple<T, U>, n> where U {
        builtin.mem.array_zip<T, U, n>(self, other)
    }

    pub pure fn each_ref(*self) -> Array<&self *T, n> {
        builtin.mem.array_each_ref<T, n>(self)
    }

    pub fn split_at(
        self,
        comptime mid: usize,
    ) -> Tuple<Array<T, mid>, Array<T, n - mid>>
    where
        requires mid <= n,
    {
        builtin.mem.array_split_at<T, n, mid>(self)
    }

    -- 静态工厂构造
    pub fn from_fn(f: %c fn(usize) -> T) -> Self where c: Type.vars.Closure {
        builtin.mem.array_from_fn<T, n, c>(f)
    }
    pub fn try_from_fn(f: %c fn(usize) -> !E T) -> !E Self
    where
        E,
        c: Type.vars.Closure,
    {
        builtin.mem.array_try_from_fn<T, E, n, c>(f)
    }
    pub fn from_repeat(value: T) -> Self where requires T :- Clone {
        Self.from_fn(|_| value.clone())
    }
}
```

`Array<T, n>` 必须实现的全部标准协议（覆盖仿射移动与逆序/正向元素析构）：
- `impl Clone for Array<T, n> where T, n: usize, requires T :- Clone`
- `impl Copy for Array<T, n> where T, n: usize, requires T :- Copy`
- `impl Drop for Array<T, n> where T, n: usize`（无 `T :- Copy` 或编译期 `Type.predicates.needs_drop(T)` 时，逐槽调用元素 `Drop`）
- `impl Eq for Array<T, n> where T, n: usize, requires T :- Eq`
- `impl Ord for Array<T, n> where T, n: usize, requires T :- Ord`
- `impl Hash for Array<T, n> where T, n: usize, requires T :- Hash`
- `impl Default for Array<T, n> where T, n: usize, requires T :- Default`
- `impl Debug for Array<T, n> where T, n: usize, requires T :- Debug`
- `impl Index<usize> for Array<T, n> where T, n: usize` (`Output = T`) 与 `impl IndexMut<usize> for Array<T, n>`
- `impl Iterator for Array<T, n>`（消耗所有权 `ArrayIter<T, n>`，通过 `IntoIterator` 产生）与 `impl ListPatternView` / `StableListPatternView for Array<T, n> where T :- Copy`

##### C. `Slice<T>` 完整视图、分割、排序与内存契约（`WL-04-12`）

现行 `library/std/collections_impl/slice.fl` 已有 `from_raw_parts`/`empty`/`len`/`is_empty`/`as_raw_addr`/`get`/`first`/`last`/`slice`/`subslice_from`/`split_at`/`ListPatternView`/`Eq`/`Debug`/`Clone`/`Copy`。补齐以下完整方法集，区分只读观测与需要可写可达性许可（由指针来源与 `MemoryView` 支撑）的就地变换：

- **只读视图与切片分割**（继承 `&self` 可达性）：
  - `as_ptr(*self) -> *T`、`as_ptr_range(*self) -> Tuple<*T, *T>`
  - `get(*self, index: usize) -> ?T`（`T :- Copy`）与 `get_ptr(*self, index: usize) -> ?&self *T`（适用于任意 `T`，含仿射/非 `Copy` 元素！）
  - `first_ptr(*self) -> ?&self *T`、`last_ptr(*self) -> ?&self *T`
  - `split_first(*self) -> ?Tuple<&self *T, &self Slice<T>>`、`split_last(*self) -> ?Tuple<&self *T, &self Slice<T>>`
  - `chunks(*self, chunk_size: usize) -> !InvalidArgErr ChunksIter<T>`（或受 `requires chunk_size > 0usize` 静态约束的 `ChunksIter<T>`）
  - `chunks_exact(*self, chunk_size: usize) -> ChunksExactIter<T>`（暴露 `.remainder() -> Slice<T>`）
  - `rchunks(*self, chunk_size: usize) -> RChunksIter<T>`、`rchunks_exact(*self, chunk_size: usize) -> RChunksExactIter<T>`
  - `windows(*self, size: usize) -> WindowsIter<T>`（`size >= 1`）
  - `split(*self, pred: %c fn(*T) -> bool) -> SliceSplitIter<T, c>`、`rsplit(...)`、`split_inclusive(...)`、`splitn(n: usize, pred)`、`rsplitn(n: usize, pred)`
  - `starts_with(*self, needle: Slice<T>) -> bool where T :- Eq`、`ends_with(*self, needle: Slice<T>) -> bool where T :- Eq`、`contains(*self, x: *T) -> bool where T :- Eq`
  - `iter(*self) -> SliceIter<T>`（产出 `&self *T`）
- **二分搜索（仅需 `*self` 与 `Eq`/`Ord`）**：
  - `binary_search(*self, x: *T) -> BinarySearch where T :- Ord`（规范：严禁定义二元枚举替代原生可选/错误，同时也不滥用 `!E T` 错误通道表示二分未命中这一正常查找结果，因此采用普通枚举 `pub enum BinarySearch { found(usize), not_found(usize) }`，遵守普通枚举变体 `snake_case` 规范！）
  - `binary_search_by(*self, f: %c fn(*T) -> Order) -> BinarySearch`
  - `binary_search_by_key(*self, key: *K, extract: %c fn(*T) -> K) -> BinarySearch where K :- Ord`
  - `partition_point(*self, pred: %c fn(*T) -> bool) -> usize`
- **就地可变算法（需切片具备可写权限与内部元素分离性保障）**：
  - `swap(*self, i: usize, j: usize) -> ?void`
  - `swap_with_slice(*self, other: *Slice<T>) -> ?void`（要求两切片等长且内存区域互不相交 `MemoryView.disjoint(self, other)`）
  - `reverse(*self)`
  - `rotate_left(*self, mid: usize)`、`rotate_right(*self, k: usize)`
  - `fill(*self, value: T) where T :- Clone`、`fill_with(*self, f: %c fn() -> T)`
  - `copy_from_slice(*self, src: Slice<T>) -> !SliceCopyLenErr void where T :- Copy`
  - `clone_from_slice(*self, src: Slice<T>) -> !SliceCopyLenErr void where T :- Clone`
  - `sort(*self) where T :- Ord`、`sort_by(*self, cmp: %c fn(*T, *T) -> Order)`、`sort_by_key(*self, key: %c fn(*T) -> K) where K :- Ord`
  - `sort_unstable(*self) where T :- Ord`、`sort_unstable_by(...)`、`sort_unstable_by_key(...)`
  - `select_nth_unstable(*self, index: usize) -> Tuple<&self Slice<T>, &self *T, &self Slice<T>> where T :- Ord`
  - `Index<usize>` / `IndexMut<usize>`、`Ord`（`where T :- Ord`）、`Hash`（`where T :- Hash`）、`MemoryView`（提供 `split_at_mut_pair` / `disjoint_subslice` 验证支持）。

---

### 3.2 原生可选 `?T` 与原生错误结果 `!E T` / `![E1, E2, ...Rest] T` 完整规格（`WL-04-13`, `WL-04-14`）

Flurry **绝不使用**用户态 `Option` 或 `Result` 枚举；原生可选 `?T`（`null` / `val?`）与原生错误结果 `!E T` / `![E1, E2, ...Rest] T`（`error e` / `val!` / `error err`）具有语言内置判别标签、ABI 与所有权规则。所有组合子通过 `builtin`（运行期）与 `meta`（纯编译期）对偶 trait 家族暴露，且**双阶段使用完全相同的方法名**。

#### 3.2.1 运行期 `?T` 全量组合子（`OptionalOps<T>` 与 `OptionalUnwrap<T>`）

现行 `library/builtin/types/option.fl` 仅实现 10 个基础组合子；参考 `development/library-prototypes/builtin/option_unwrap.fl` 并按最终语言设计补齐全部 30 个操作（`OptionalUnwrap` 拆出可能陷入/中止控制流的非纯解包方法，保持 `OptionalOps` 可被纯上下文按需约束）：

```flurry
pub trait OptionalOps
where
    T,
{
    fn is_some(self) -> bool;
    fn is_none(self) -> bool;
    fn is_some_and(self, pred: %c fn(T) -> bool) -> bool where c: Type.vars.Closure;
    fn is_none_or(self, pred: %c fn(T) -> bool) -> bool where c: Type.vars.Closure;

    fn unwrap_or(self, fallback: T) -> T;
    fn unwrap_or_else(self, fallback: %c fn() -> T) -> T where c: Type.vars.Closure;
    fn unwrap_or_default(self) -> T where requires T :- Default;

    fn map(self, f: %c fn(T) -> U) -> ?U where U, c: Type.vars.Closure;
    fn map_or(self, default_val: U, f: %c fn(T) -> U) -> U where U, c: Type.vars.Closure;
    fn map_or_else(
        self,
        default_fn: %c1 fn() -> U,
        f: %c2 fn(T) -> U,
    ) -> U
    where
        U,
        c1: Type.vars.Closure,
        c2: Type.vars.Closure;

    fn and_then(self, f: %c fn(T) -> ?U) -> ?U where U, c: Type.vars.Closure;
    fn or(self, fallback: ?T) -> ?T;
    fn or_else(self, fallback: %c fn() -> ?T) -> ?T where c: Type.vars.Closure;
    fn xor(self, other: ?T) -> ?T;
    fn filter(self, pred: %c fn(*T) -> bool) -> ?T where c: Type.vars.Closure;
    fn inspect(self, observer: %c fn(*T)) -> ?T where c: Type.vars.Closure;

    fn zip(self, other: ?U) -> ?Tuple<T, U> where U;
    fn zip_with(
        self,
        other: ?U,
        combine: %c fn(T, U) -> V,
    ) -> ?V
    where
        U,
        V,
        c: Type.vars.Closure;
    fn unzip(self) -> Tuple<?A, ?B>
    where
        A,
        B,
        requires T == Tuple<A, B>;
    fn flatten(self) -> ?Inner
    where
        Inner,
        requires T == ?Inner;

    -- 就地状态变更（仅运行期通过可变 `*self` 指针完成，保留严格 Drop/移动契约）
    fn take(*self) -> ?T;
    fn take_if(*self, pred: %c fn(*T) -> bool) -> ?T where c: Type.vars.Closure;
    fn replace(*self, value: T) -> ?T;
    fn get_or_insert(*self, value: T) -> *T;
    fn get_or_insert_with(*self, f: %c fn() -> T) -> *T where c: Type.vars.Closure;
    fn get_or_insert_default(*self) -> *T where requires T :- Default;

    -- 与原生错误结果互转及内部指针视图
    fn ok_or(self, err: E) -> !E T where E;
    fn ok_or_else(self, err: %c fn() -> E) -> !E T where E, c: Type.vars.Closure;
    fn as_ptr(*self) -> ?*T;
    fn copied(self) -> ?Inner
    where
        Inner,
        requires T == *Inner,
        requires Inner :- Copy;
    fn cloned(self) -> ?Inner
    where
        Inner,
        requires T == *Inner,
        requires Inner :- Clone;
    fn transpose_err(self) -> !E ?OkVal
    where
        OkVal,
        E,
        requires T == !E OkVal;
}

pub trait OptionalUnwrap :- OptionalOps<T>
where
    T,
{
    fn unwrap(self) -> T;
    fn expect(self, message: str) -> T;
    unsafe fn unwrap_unchecked(self) -> T;
}
```

#### 3.2.2 纯编译期 `?T` 对偶（`meta.ComptimeOptionalOps<T>` 与 `ComptimeOptionalUnwrap<T>`）

严格遵守 `AGENTS.md` §3 与 §5：
- 使用独立 `pure comptime trait`，所有与运行期对应的操作保持**完全同名**（无 `comptime_` 前缀）。
- 全套签名与实现采用 **`self` 值引用语义**（**零 `*self`、`.*`、`.ref`**）。
- 对于运行期基于 `*self` 就地变更状态的 `take`/`replace`/`get_or_insert*`，在无指针的 comptime 语境下分为两类语义：
  - 返回更新后状态对或采用值引用写回语义：`take(self) -> ?T`（将 `self` 原槽位置为 `null` 并返回旧值）、`replace(self, value: T) -> ?T`（以 `self` 值引用置换并返回旧值）、`get_or_insert(self, value: T) -> T` / `get_or_insert_with(self, f: %c comptime fn() -> T) -> T`（若原槽位为 `null` 则写入 `self` 并返回该值，不暴露裸指针 `*T`！）；
  - `as_ptr(*self) -> ?*T` 由于 **comptime 严禁指针**，**故意不纳入** `ComptimeOptionalOps`。

```flurry
-- 归属 library/meta/data/optional.fl（无 *self / .ref / .*，无 comptime_ 前缀）
pub pure comptime trait ComptimeOptionalOps
where
    T,
{
    comptime fn is_some(self) -> bool;
    comptime fn is_none(self) -> bool;
    comptime fn is_some_and(self, pred: %c comptime fn(T) -> bool) -> bool where c: Type.vars.Closure;
    comptime fn is_none_or(self, pred: %c comptime fn(T) -> bool) -> bool where c: Type.vars.Closure;

    comptime fn unwrap_or(self, fallback: T) -> T;
    comptime fn unwrap_or_else(self, fallback: %c comptime fn() -> T) -> T where c: Type.vars.Closure;
    comptime fn unwrap_or_default(self) -> T where requires T :- ComptimeDefault;

    comptime fn map(self, f: %c comptime fn(T) -> U) -> ?U where U, c: Type.vars.Closure;
    comptime fn map_or(self, default_val: U, f: %c comptime fn(T) -> U) -> U where U, c: Type.vars.Closure;
    comptime fn map_or_else(
        self,
        default_fn: %c1 comptime fn() -> U,
        f: %c2 comptime fn(T) -> U,
    ) -> U
    where
        U,
        c1: Type.vars.Closure,
        c2: Type.vars.Closure;

    comptime fn and_then(self, f: %c comptime fn(T) -> ?U) -> ?U where U, c: Type.vars.Closure;
    comptime fn or(self, fallback: ?T) -> ?T;
    comptime fn or_else(self, fallback: %c comptime fn() -> ?T) -> ?T where c: Type.vars.Closure;
    comptime fn xor(self, other: ?T) -> ?T;
    comptime fn filter(self, pred: %c comptime fn(T) -> bool) -> ?T where c: Type.vars.Closure;
    comptime fn inspect(self, observer: %c comptime fn(T)) -> ?T where c: Type.vars.Closure;

    comptime fn zip(self, other: ?U) -> ?Tuple<T, U> where U;
    comptime fn zip_with(
        self,
        other: ?U,
        combine: %c comptime fn(T, U) -> V,
    ) -> ?V
    where
        U,
        V,
        c: Type.vars.Closure;
    comptime fn unzip(self) -> Tuple<?A, ?B>
    where
        A,
        B,
        requires T == Tuple<A, B>;
    comptime fn flatten(self) -> ?Inner
    where
        Inner,
        requires T == ?Inner;

    -- 编译期值引用槽位原位更新（绝不返回 *T 指针）
    comptime fn take(self) -> ?T;
    comptime fn replace(self, value: T) -> ?T;
    comptime fn get_or_insert(self, value: T) -> T;
    comptime fn get_or_insert_with(self, f: %c comptime fn() -> T) -> T where c: Type.vars.Closure;

    comptime fn ok_or(self, err: E) -> !E T where E;
    comptime fn ok_or_else(self, err: %c comptime fn() -> E) -> !E T where E, c: Type.vars.Closure;
    comptime fn transpose_err(self) -> !E ?OkVal
    where
        OkVal,
        E,
        requires T == !E OkVal;
}

pub pure comptime trait ComptimeOptionalUnwrap :- ComptimeOptionalOps<T>
where
    T,
{
    comptime fn unwrap(self) -> T;
    comptime fn expect(self, message: str) -> T;
}
```

**`?T` 协议补全清单**：
- `impl Clone for ?T where T, requires T :- Clone`（解除当前仅有 `where T :- Copy` 的限制，允许对仿射可克隆载荷调用 `.clone()`！）
- `impl Default for ?T where T`（始终返回 `null`）
- `impl Display for ?T where T, requires T :- Display` 与 `impl ComptimeDisplay for ?T where T :- ComptimeDisplay`（格式约定见 §3.5.3）

#### 3.2.3 原生错误结果 `!E T` 与多错误行 `![E1, E2, ...Rest] T` 完整规格（`WL-04-14`）

1. **单一错误结果 `!E T` 组合子（`ErrorResultOps<T, E>` 与 `ErrorResultUnwrap<T, E>`）**：

```flurry
pub trait ErrorResultOps
where
    T,
    E,
{
    fn is_ok(self) -> bool;
    fn is_err(self) -> bool;
    fn is_ok_and(self, pred: %c fn(T) -> bool) -> bool where c: Type.vars.Closure;
    fn is_err_and(self, pred: %c fn(E) -> bool) -> bool where c: Type.vars.Closure;

    fn ok(self) -> ?T;
    fn err(self) -> ?E;

    fn unwrap_or(self, fallback: T) -> T;
    fn unwrap_or_else(self, fallback: %c fn(E) -> T) -> T where c: Type.vars.Closure;
    fn unwrap_or_default(self) -> T where requires T :- Default;

    fn map(self, f: %c fn(T) -> U) -> !E U where U, c: Type.vars.Closure;
    fn map_err(self, f: %c fn(E) -> F) -> !F T where F, c: Type.vars.Closure;
    fn map_or(self, default_val: U, f: %c fn(T) -> U) -> U where U, c: Type.vars.Closure;
    fn map_or_else(
        self,
        fallback: %c1 fn(E) -> U,
        f: %c2 fn(T) -> U,
    ) -> U
    where
        U,
        c1: Type.vars.Closure,
        c2: Type.vars.Closure;

    fn and_then(self, f: %c fn(T) -> !E U) -> !E U where U, c: Type.vars.Closure;
    fn or_else(self, f: %c fn(E) -> !F T) -> !F T where F, c: Type.vars.Closure;
    fn inspect(self, observer: %c fn(*T)) -> !E T where c: Type.vars.Closure;
    fn inspect_err(self, observer: %c fn(*E)) -> !E T where c: Type.vars.Closure;

    fn flatten(self) -> !E Inner
    where
        Inner,
        requires T == !E Inner;
    fn transpose_opt(self) -> ?!E Inner
    where
        Inner,
        requires T == ?Inner;

    fn as_ok_ptr(*self) -> ?*T;
    fn as_err_ptr(*self) -> ?*E;
    fn copied(self) -> !E Inner
    where
        Inner,
        requires T == *Inner,
        requires Inner :- Copy;
    fn cloned(self) -> !E Inner
    where
        Inner,
        requires T == *Inner,
        requires Inner :- Clone;

    -- 基于底层 NoReturn 不可达性安全消去单侧分支
    fn into_ok(self) -> T where requires E == NoReturn;
    fn into_err(self) -> E where requires T == NoReturn;
}

pub trait ErrorResultUnwrap :- ErrorResultOps<T, E>
where
    T,
    E,
{
    fn unwrap(self) -> T;
    fn expect(self, message: str) -> T;
    fn unwrap_err(self) -> E;
    fn expect_err(self, message: str) -> E;
    unsafe fn unwrap_unchecked(self) -> T;
    unsafe fn unwrap_err_unchecked(self) -> E;
}
```

对应 `library/meta/data/errors.fl` 中的 `pure comptime trait ComptimeErrorResultOps` 与 `ComptimeErrorResultUnwrap` 使用**同名**（无 `comptime_` 前缀）、**`self` 值引用语义**（零 `*self`、`.*`、`.ref`），显式省略运行期指针/未检查方法（`as_ok_ptr`/`as_err_ptr`/`copied`/`cloned`/`unwrap_unchecked`/`unwrap_err_unchecked`），并将 `inspect`/`inspect_err` 回调从 `*T`/`*E` 改为值引用 `T`/`E`：

```flurry
pub pure comptime trait ComptimeErrorResultOps
where
    T,
    E,
{
    comptime fn is_ok(self) -> bool;
    comptime fn is_err(self) -> bool;
    comptime fn is_ok_and(self, pred: %c comptime fn(T) -> bool) -> bool where c: Type.vars.Closure;
    comptime fn is_err_and(self, pred: %c comptime fn(E) -> bool) -> bool where c: Type.vars.Closure;

    comptime fn ok(self) -> ?T;
    comptime fn err(self) -> ?E;

    comptime fn unwrap_or(self, fallback: T) -> T;
    comptime fn unwrap_or_else(self, fallback: %c comptime fn(E) -> T) -> T
    where
        c: Type.vars.Closure;
    comptime fn unwrap_or_default(self) -> T where requires T :- ComptimeDefault;

    comptime fn map(self, f: %c comptime fn(T) -> U) -> !E U where U, c: Type.vars.Closure;
    comptime fn map_err(self, f: %c comptime fn(E) -> F) -> !F T where F, c: Type.vars.Closure;
    comptime fn map_or(self, default_val: U, f: %c comptime fn(T) -> U) -> U
    where
        U,
        c: Type.vars.Closure;
    comptime fn map_or_else(
        self,
        fallback: %c1 comptime fn(E) -> U,
        f: %c2 comptime fn(T) -> U,
    ) -> U
    where
        U,
        c1: Type.vars.Closure,
        c2: Type.vars.Closure;

    comptime fn and_then(self, f: %c comptime fn(T) -> !E U) -> !E U
    where
        U,
        c: Type.vars.Closure;
    comptime fn or_else(self, f: %c comptime fn(E) -> !F T) -> !F T
    where
        F,
        c: Type.vars.Closure;
    comptime fn inspect(self, observer: %c comptime fn(T)) -> !E T where c: Type.vars.Closure;
    comptime fn inspect_err(self, observer: %c comptime fn(E)) -> !E T where c: Type.vars.Closure;

    comptime fn flatten(self) -> !E Inner
    where
        Inner,
        requires T == !E Inner;
    comptime fn transpose_opt(self) -> ?!E Inner
    where
        Inner,
        requires T == ?Inner;

    comptime fn into_ok(self) -> T where requires E == NoReturn;
    comptime fn into_err(self) -> E where requires T == NoReturn;
}

pub pure comptime trait ComptimeErrorResultUnwrap :- ComptimeErrorResultOps<T, E>
where
    T,
    E,
{
    comptime fn unwrap(self) -> T;
    comptime fn expect(self, message: str) -> T;
    comptime fn unwrap_err(self) -> E;
    comptime fn expect_err(self, message: str) -> E;
}
```

2. **多错误行 `![E1, E2, ...Rest] T` 原生组合子与标准 Trait 一致性**：
   依据 `docs/type/qualified-types/error-type.md` 与 `docs/type/rows.md`，`!E T` 实质上是单元素错误行 `![E] T` 的语法糖。对于一般错误行 `![...Es] T`（其中 `Es: meta.List<Type>`，支持扩展与去重并集），补全以下通用行辅助能力：

```flurry
pub trait ErrorRowOps
where
    T,
    Es: meta.List<Type>,
{
    fn is_ok(self) -> bool;
    fn is_err(self) -> bool;
    fn ok(self) -> ?T;
    fn inspect(self, observer: %c fn(*T)) -> ![.Es = Es] T where c: Type.vars.Closure;
    fn map(self, f: %c fn(T) -> U) -> ![.Es = Es] U where U, c: Type.vars.Closure;

    -- 从当前错误行精准剥离一种错误变体 E 并交由处理器，其余错误不变保留至剩余行
    fn catch_one(
        self,
        handler: %c fn(E) -> ![.Es = Es.remove(E)] T,
    ) -> ![.Es = Es.remove(E)] T
    where
        E,
        c: Type.vars.Closure,
        requires Es.contains(E);

    -- 错误行窄化/收敛：当子行 Subset 被 Es 包含或所有已移除变体已处理完毕时显式收缩
    fn widen_row(self) -> ![.Es = Es.union(Extra)] T
    where
        Extra: meta.List<Type>;

    fn narrow_row(self) -> ![.Es = TargetEs] T
    where
        TargetEs: meta.List<Type>,
        requires Es.is_subset_of(TargetEs);

    -- 将整个错误行映射合并为单一外层错误类型 F
    fn map_row(self, f: %c fn(Tuple<.Es = Es>) -> F) -> !F T
    where
        F,
        c: Type.vars.Closure;
}
```

- **异构错误行资源与常规 trait 实现**（均受限于通用 `Es.all(...)` 谓词展开与当前 `issues/0003` 记录的 `cc178adb` 多错误 struct matching 阻塞）：
  - `impl Clone for ![.Es = Es] T where T, Es: meta.List<Type>, requires T :- Clone and Es.all(|E| E :- Clone)`
  - `impl Copy for ![.Es = Es] T where T, Es: meta.List<Type>, requires T :- Copy and Es.all(|E| E :- Copy)`
  - `impl Eq for ![.Es = Es] T where T, Es: meta.List<Type>, requires T :- Eq and Es.all(|E| E :- Eq)`
  - `impl Ord for ![.Es = Es] T where T, Es: meta.List<Type>, requires T :- Ord and Es.all(|E| E :- Ord)`（标签先按规范类型顺序/比较，载荷再比 `cmp`）
  - `impl Hash for ![.Es = Es] T where T, Es: meta.List<Type>, requires T :- Hash and Es.all(|E| E :- Hash)`
  - `impl Debug for ![.Es = Es] T where T, Es: meta.List<Type>, requires T :- Debug and Es.all(|E| E :- Debug)`

---

### 3.3 `builtin.math` 与 `meta.math` 完整数学体系（`WL-04-15` ~ `WL-04-17`）

> 归属铁律：`math` 严格归入 `builtin.math`（通过 `builtin.prelude` 默认暴露）与 `meta.math`，绝不另设第 7 个内置包。

#### 3.3.1 缺失 LLVM / Libm 标量与整数 Intrinsic 全量清单（`WL-04-15`）

当前 `target.Scalar`（`library/builtin/numeric/math_intrinsics.fl` 与 `library/meta/numeric/scalar_math.fl`）已覆盖 44 个变体（`fma`, `sqrt`, `copysign`, `floor`, `ceil`, `trunc`, `roundeven`, `round`, `fabs`, `sin`, `cos`, `exp`, `exp2`, `log`, `log2`, `log10`, `pow`, `powi`, `ldexp`, `minimum`, `maximum`, `minimumnum`, `maximumnum`, `isnan`, `isinf`, `isfinite`, `isnormal`, `issubnormal`, `signbit`, `bitreverse`, `ctpop`, `ctlz`, `cttz`, `bswap`, `fshl`, `fshr`, `add_sat`, `sub_sat`, `min`, `max`, `abs`, `bit_and`, `bit_or`, `bit_xor`），并已对 `f16`/`bf16` 支持其中 20 个基础 IEEE 分类、舍入、`fma`/`sqrt`/`copysign`/`fabs`/`minimum`/`maximum`/`minimumnum`/`maximumnum`/`ldexp`。

需补齐以下**全部缺失家族**（同时注册运行期 `target.Scalar` 与编译期 `meta.math` 对偶 `pure comptime fn`）：

| 家族类别 | 需补全的操作名称 | 支持类型范围 | 后端降低契约（LLVM Intrinsic / Libm / Legalize） |
|---|---|---|---|
| **三角与反三角** | `tan`, `asin`, `acos`, `atan`, `atan2`, `sincos` | `f16`, `bf16`, `f32`, `f64` | LLVM 20+ `llvm.tan`/`asin`/`acos`/`atan`/`atan2`/`sincos` 或 libm fallback（`f16`/`bf16` 经 `f32` 精确升宽后回舍入） |
| **双曲与反双曲** | `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh` | `f16`, `bf16`, `f32`, `f64` | `llvm.sinh`/`cosh`/`tanh` 及 libm `asinh`/`acosh`/`atanh`（编译期 VM 与宿主机双端保证 IEEE 边缘值如 `-0.0`/`NaN`/`Inf` 一致） |
| **幂、根与精确差值** | `cbrt`, `hypot`, `expm1`, `log1p`, `fdim`, `remquo`, `nextafter` | `f16`, `bf16`, `f32`, `f64` | libm `cbrt`/`hypot`/`expm1`/`log1p`/`fdim`/`remquo`/`nextafter`（`remquo` 返回 `Tuple<T, i32>`，需扩充 `nir::intrinsics::Operation` 多返回值支持） |
| **特殊数学函数** | `erf`, `erfc`, `tgamma`, `lgamma` | `f16`, `bf16`, `f32`, `f64` | libm 映射（`lgamma` 提供 `lgamma_r` 无数据竞争可重入入口并返回 `Tuple<T, i32>` 符号） |
| **已存超越函数的 `f16`/`bf16` 扩展** | `sin`, `cos`, `exp`, `exp2`, `log`, `log2`, `log10`, `pow`, `powi` | 补齐 `f16`, `bf16` | 目前 `math_intrinsics.fl` 对上述 9 项仅接通 `f32`/`f64`；补全 `f16`/`bf16` 与 `meta.math` 对偶 |
| **浮点位重解释** | `f16_to_bits`/`from_bits`, `bf16_to_bits`/`from_bits`, `f32_to_bits`/`from_bits`, `f64_to_bits`/`from_bits` | 浮点 <-> `u16`/`u32`/`u64` | LLVM `bitcast`，绝对不改变 bit pattern（含静默/信号 NaN 负载位）；`comptime` VM 保留精确位图 |
| **有符号/无符号区分与整数溢出** | `uadd_with_overflow`, `sadd_with_overflow`, `usub_with_overflow`, `ssub_with_overflow`, `umul_with_overflow`, `smul_with_overflow` | 全部定长整数 + `Uint<W>`/`Int<W>` | LLVM `llvm.{u,s}{add,sub,mul}.with.overflow.*`，直接返回 `Tuple<T, bool>`（与 Flurry 无特殊结构体槽兼容） |
| **饱和算术与有/无符号极值分离** | `uadd_sat`, `sadd_sat`, `usub_sat`, `ssub_sat`, `smin`, `smax`, `umin`, `umax` | 全部定长整数 | 修复现行 `target.Scalar.{add_sat, sub_sat, min, max}` 按有符号/无符号语义自动依赖绑定参数猜测的隐患，在 NIR intrinsic 中显式区分有/无符号 opcode |
| **精确移位与多精度乘加** | `wrapping_shl`, `wrapping_lshr`, `wrapping_ashr`, `uint_widening_mul`, `sint_widening_mul`, `uint_carrying_add`, `uint_borrowing_sub` | 全部定长整数 | 移位量按 `rhs % BITS` 掩码，禁止产生 LLVM `poison`；`widening_mul` 使用 2 倍位宽乘法再拆分高低半部分（或 `llvm.mul` 升宽） |

**纯度分级注解**：纯 IEEE 标量数学（不依赖浮点环境舍入模式全局状态异常或可假设默认 round-to-nearest-ties-to-even）属于 P4/P5 可纯化算术，而读/写硬件 FCSR/MXCSR 状态的未封装指令则降级为 P0。

#### 3.3.2 通用 Vector / SIMD 数学家族（`WL-04-16`）

参考 `docs/pipeline-projection/target-validation-and-assembly.md`，将定宽向量数学从平台内联汇编封装上升为可移植 `builtin.math.simd`（不绑死单一 ISA）：

```flurry
-- 仅允许出现在集中绑定层：library/builtin/numeric/simd.fl
pub typealias Simd<T, lanes: usize> = .Simd'builtin<T, lanes>;

pub pure fn vec_add(a: Simd<T, n>, b: Simd<T, n>) -> Simd<T, n>
where
    T,
    n: usize,
    requires Simd<T, n> :- SimdElementNumeric,
{
    unsafe { target.Vector.add'intrinsic(a, b).as(Simd<T, n>) }
}
```

**全量 SIMD 数学操作家族**（覆盖 `u8..u64`/`i8..i64`/`f16`/`bf16`/`f32`/`f64`，合法 `lanes` 为目标检查支持的 2 的幂次，或由 LLVM Vector Legalizer 拆分/标量化）：
- 算术与 FMA：`vec_add`, `vec_sub`, `vec_mul`, `vec_div`, `vec_rem`, `vec_neg`, `vec_fma`, `vec_abs`
- 舍入与数学：`vec_sqrt`, `vec_floor`, `vec_ceil`, `vec_trunc`, `vec_roundeven`, `vec_round`, `vec_sin`, `vec_cos`, `vec_exp`, `vec_exp2`, `vec_log`, `vec_log2`, `vec_pow`, `vec_minimum`, `vec_maximum`, `vec_minimumnum`, `vec_maximumnum`
- 按道位操作与比较：`vec_bit_and`, `vec_bit_or`, `vec_bit_xor`, `vec_bit_not`, `vec_shl`, `vec_shr`, `vec_select(mask: SimdMask<n>, on_true, on_false)`
- 水平归约（Horizontal Reductions）：`vec_reduce_add`, `vec_reduce_mul`, `vec_reduce_and`, `vec_reduce_or`, `vec_reduce_xor`, `vec_reduce_min`, `vec_reduce_max`, `vec_reduce_fadd_ordered(init, v)`（保留 IEEE 严格加法结合序）
- **平台特性门控**：与 `build.target.features` 集成——若程序请求使用固定硬件宽度（如 `f32` x 8）并投射到显式 GPU/SIMD pipeline，由后端 target validator 检验向量 ABI 与目标特性；普通宿主调用则由 LLVM backend 根据实际 target-feature 决定发射原生 SIMD 指令或安全标量化。

#### 3.3.3 `builtin.math.consts` 与 `meta.math.consts` 数学常量套件（`WL-04-17`）

同时为 `f16`、`bf16`、`f32`、`f64` 与编译时高精度 `Float`（置于 `meta.math.consts`）提供**全量 22 个标准常量**（精度按 IEEE round-to-nearest-even 舍入至对应格式最后一位，禁止运行期表达式求值引入误差）：

| 常量名 | 数学含义 | 运行期子模块 | 编译时子模块 |
|---|---|---|---|
| `PI` | $\pi$ | `builtin.math.consts.{f16, bf16, f32, f64}.PI` | `meta.math.consts.{f16, bf16, f32, f64, float}.PI` |
| `TAU` | $2\pi$ | 同上 | 同上 |
| `FRAC_PI_2` | $\pi / 2$ | 同上 | 同上 |
| `FRAC_PI_3` | $\pi / 3$ | 同上 | 同上 |
| `FRAC_PI_4` | $\pi / 4$ | 同上 | 同上 |
| `FRAC_PI_6` | $\pi / 6$ | 同上 | 同上 |
| `FRAC_PI_8` | $\pi / 8$ | 同上 | 同上 |
| `FRAC_1_PI` | $1 / \pi$ | 同上 | 同上 |
| `FRAC_2_PI` | $2 / \pi$ | 同上 | 同上 |
| `FRAC_2_SQRT_PI` | $2 / \sqrt{\pi}$ | 同上 | 同上 |
| `E` | 自然对数底 $e$ | 同上 | 同上 |
| `LN_2` | $\ln 2$ | 同上 | 同上 |
| `LN_10` | $\ln 10$ | 同上 | 同上 |
| `LOG2_E` | $\log_2 e$ | 同上 | 同上 |
| `LOG2_10` | $\log_2 10$ | 同上 | 同上 |
| `LOG10_E` | $\log_{10} e$ | 同上 | 同上 |
| `LOG10_2` | $\log_{10} 2$ | 同上 | 同上 |
| `SQRT_2` | $\sqrt{2}$ | 同上 | 同上 |
| `FRAC_1_SQRT_2` | $1 / \sqrt{2}$ | 同上 | 同上 |
| `SQRT_3` | $\sqrt{3}$ | 同上 | 同上 |
| `PHI` | 黄金比例 $(1 + \sqrt{5}) / 2$ | 同上 | 同上 |
| `EGAMMA` | 欧拉-马斯刻若尼常数 $\gamma$ | 同上 | 同上 |

示例规范（以 `builtin.math.consts.f64` 为例，保留精确 17 位十进制有效数字以上，全常量无省略）：

```flurry
pub mod consts {
    pub mod f64 {
        pub const PI: f64 = 3.14159265358979323846f64;
        pub const TAU: f64 = 6.28318530717958647692f64;
        pub const FRAC_PI_2: f64 = 1.57079632679489661923f64;
        pub const FRAC_PI_3: f64 = 1.04719755119659774615f64;
        pub const FRAC_PI_4: f64 = 0.78539816339744830962f64;
        pub const FRAC_PI_6: f64 = 0.52359877559829887308f64;
        pub const FRAC_PI_8: f64 = 0.39269908169872415481f64;
        pub const FRAC_1_PI: f64 = 0.31830988618379067154f64;
        pub const FRAC_2_PI: f64 = 0.63661977236758134308f64;
        pub const FRAC_2_SQRT_PI: f64 = 1.12837916709551257390f64;
        pub const E: f64 = 2.71828182845904523536f64;
        pub const LN_2: f64 = 0.69314718055994530942f64;
        pub const LN_10: f64 = 2.30258509299404568402f64;
        pub const LOG2_E: f64 = 1.44269504088896340736f64;
        pub const LOG2_10: f64 = 3.32192809488736234787f64;
        pub const LOG10_E: f64 = 0.43429448190325182765f64;
        pub const LOG10_2: f64 = 0.30102999566398119521f64;
        pub const SQRT_2: f64 = 1.41421356237309504880f64;
        pub const FRAC_1_SQRT_2: f64 = 0.70710678118654752440f64;
        pub const SQRT_3: f64 = 1.73205080756887729353f64;
        pub const PHI: f64 = 1.61803398874989484820f64;
        pub const EGAMMA: f64 = 0.57721566490153286061f64;
    }
}
```

---

### 3.4 `builtin.iterator` 与 `meta.PureComptimeIterator` 完整规格（`WL-04-18` ~ `WL-04-20`）

#### 3.4.1 核心推式迭代模型与去冗余修正

`builtin.iterator` 以 Flurry 内联代数效应 `emit_item` 为主轴，实现零闭环堆栈分配的内联推式流水线（现有基线位于 `library/builtin/protocols/iterator.fl`）：

```flurry
pub inline effect emit_item(value: T) where T;

pub trait Iterator {
    assoc Item: Type;

    inline fn iterate(self) -> #emit_item<Item> void;
    -- 全部默认适配器与消费者方法见下表与完整签名定义
}
```

**现行约束过度修复**：现行 `iterator.fl` 要求 `min(self) -> ?Item` 与 `max(self) -> ?Item` 满足 `Item :- Ord + Copy`，这是不合理的过渡限制——推式消费可在累加器 `var best: ?Item` 中按所有权保存当前最值，胜出时直接覆盖旧值（触发旧值正常 `Drop`），仅需要 **`requires Item :- Ord`**（无需 `Copy`）。

#### 3.4.2 适配器全集与边界特质（`WL-04-18`）

1. **完整适配器清单（全部 21 个适配器，零 `...`）**：
   - **已有 16 个（保留并增强）**：`map`, `filter`, `filter_map`, `map_while`, `scan`, `inspect`, `take`, `take_while`, `skip`, `skip_while`, `step_by`, `chain`, `enumerate`, `flat_map`, `flatten`, `intersperse_with`。
   - **新增补全 5 个**：
     - `intersperse(self, separator: Item) -> IntersperseIterator<Self> where requires Item :- Clone`
     - `zip(self, other: J) -> ZipIterator<Self, J.as(IntoIterator).Iter> where J, requires J :- IntoIterator`
     - `peekable(self) -> PeekableIterator<Self> where requires Self :- StepIterator`（或协程支持；见 §3.4.3 Push vs Pull 架构界定）
     - `cycle(self) -> CycleIterator<Self> where requires Self :- Clone`
     - `rev(self) -> RevIterator<Self> where requires Self :- DoubleEndedIterator`
   - 附加常用零开销适配器：`copied(self) -> CopiedIterator<Self> where Inner, requires Item == *Inner and Inner :- Copy`、`cloned(self) -> ClonedIterator<Self> where Inner, requires Item == *Inner and Inner :- Clone`、`fuse(self) -> FuseIterator<Self>`、`map_windows`（若接入切片缓冲或固定 `Array` 窗口）。

2. **边界能力子特质**：

```flurry
-- 步进（Pull）辅助协议：允许单步推进与可中断并发步进
pub trait StepIterator :- Iterator {
    fn next(*self) -> ?Item;

    fn size_hint(self) -> Tuple<usize, ?usize> { (0usize, null) }

    -- 默认直接由 next 驱动 push 效应，避免所有集合写两份迭代循环
    inline fn iterate(self) -> #emit_item<Item> void {
        var iter = self;
        while true {
            iter.next() match {
                value? => emit_item<Item>(value)#;,
                null => break,
            }
        }
    }
}

-- 双端迭代协议：支持反向全流推送（纯 push `rev_iterate`）以及步进反向消费
pub trait DoubleEndedIterator :- StepIterator {
    fn next_back(*self) -> ?Item;

    inline fn rev_iterate(self) -> #emit_item<Item> void {
        var iter = self;
        while true {
            iter.next_back() match {
                value? => emit_item<Item>(value)#;,
                null => break,
            }
        }
    }
}

pub trait ExactSizeIterator :- StepIterator {
    fn exact_len(*self) -> usize;
}

pub trait FusedIterator :- StepIterator {}
```

#### 3.4.3 深度架构分析：Push（`emit_item`）vs Pull（`next`）边界

以 `inline fn iterate(self) -> #emit_item<Item> void` 为核心的推式迭代器虽然能够对单流转换（`map`/`filter`/`take`/`flat_map`/`fold`/`find`）实现完全内联、无控制流状态机膨胀与短路返回，但存在本质界限：

| 操作 / 特性 | 纯推式 `inline fn iterate` 是否胜任 | 本质原因 | Flurry 正式方案设计 |
|---|---|---|---|
| `map`/`filter`/`take`/`skip`/`chain`/`flat_map`/`fold`/`any`/`all`/`find` | **胜任（最优）** | 仅存在单主循环生产者；短路通过内联 handler 中 `return` 提前跳出推流完成，无需堆帧 | 保持为基于 `#emit_item` 的纯内联推式默认实现 |
| `cycle` | **胜任** | `while true { let cloned = self.clone(); cloned.iterate(); }`（空流早判优化需记录是否至少产出过 1 项，全空直接终止避免死循环） | 纯推式实现，要求 `Self :- Clone` |
| `rev`（整体逆序迭代） | **仅纯全流反转胜任** | 若目标仅消费逆序整条流，可增加独立 `rev_iterate`；但 `seq.take(3).rev()` 截断后反转、前后交替消元则不能靠单侧推流完成 | 结合 `DoubleEndedIterator.next_back` + `RevIterator` |
| `zip`、`eq`、`cmp`、`lexicographical_cmp`、`peekable` | **不可仅靠单端内联推式实现** | 两条任意推式流（如两个 `flat_map`）各自持有调用栈控制权，无法由单个静态 `handle` 同时交叉推进而不挂起其中至少一方 | **混合分层（推荐方案）**：<br>1. **Fast Path**：当两端都实现 `StepIterator`（涵盖 `Array`、`Slice`、`Vec`、`Range`、`Map`、`Filter`、`Take`、`Enumerate`、`Repeat`、`Successors`、`FromFn` 等 90% 常见源）时，`ZipIterator` / `eq` / `cmp` / `peekable` 直接以零堆分配、无协程挂起状态机执行锁步 `next()`；<br>2. **General Path**：若其中至少一方为纯推式复杂生产者（如任意嵌套 `flat_map`），通过 `builtin.Continuation` 一次性非逃逸协程挂起（在 `#emit_item` 处 yield）把任意 push `Iterator` 自动桥接为按需步进流（联动 §5 Q5） |

#### 3.4.4 消费者、累计特质与多集合比较全集（`WL-04-18`）

1. **完整消费者清单（全部 26 个消费者，零 `...`）**：
   - `foreach(self, f: %c fn(Item))`
   - `try_foreach(self, f: %c fn(Item) -> !E void) -> !E void`
   - `fold(self, init: A, f: %c fn(A, Item) -> A) -> A`
   - `try_fold(self, init: A, f: %c fn(A, Item) -> !E A) -> !E A`
   - `reduce(self, f: %c fn(Item, Item) -> Item) -> ?Item`
   - `try_reduce(self, f: %c fn(Item, Item) -> !E Item) -> !E ?Item`
   - `any(self, pred: %c fn(*Item) -> bool) -> bool`
   - `all(self, pred: %c fn(*Item) -> bool) -> bool`
   - `find(self, pred: %c fn(*Item) -> bool) -> ?Item`
   - `find_map(self, f: %c fn(Item) -> ?U) -> ?U`
   - `position(self, pred: %c fn(*Item) -> bool) -> ?usize`
   - `rposition(self, pred: %c fn(*Item) -> bool) -> ?usize`（需要 `Self :- DoubleEndedIterator + ExactSizeIterator`）
   - `count(self) -> usize`
   - `nth(self, index: usize) -> ?Item`
   - `last(self) -> ?Item`
   - `collect(self) -> C where C :- FromIterator<Item>`
   - `min(self) -> ?Item where requires Item :- Ord`
   - `max(self) -> ?Item where requires Item :- Ord`
   - `min_by(self, compare: %c fn(*Item, *Item) -> Order) -> ?Item`
   - `max_by(self, compare: %c fn(*Item, *Item) -> Order) -> ?Item`
   - `min_by_key(self, key: %c fn(*Item) -> K) -> ?Item where K, requires K :- Ord`
   - `max_by_key(self, key: %c fn(*Item) -> K) -> ?Item where K, requires K :- Ord`
   - **新增聚合**：
     - `sum(self) -> S where S, requires S :- Sum<.A = Item>`
     - `product(self) -> P where P, requires P :- Product<.A = Item>`
     - `partition(self, pred: %c fn(*Item) -> bool) -> Tuple<B, B> where B, requires B :- Default + Extend<Item>`
     - `unzip(self) -> Tuple<A, B> where First, Second, A, B, requires Item == Tuple<First, Second> and A :- Default + Extend<First> and B :- Default + Extend<Second>`
   - **新增字典序与相等性消费者**（支持对称跨类型 `IntoIterator` 比较）：
     - `eq(self, other: J) -> bool where J, requires J :- IntoIterator and Item :- Eq and J.as(IntoIterator).Iter.as(Iterator).Item == Item`
     - `ne(self, other: J) -> bool`（同上条件，取 `not self.eq(other)`）
     - `cmp(self, other: J) -> Order where J, requires J :- IntoIterator and Item :- Ord and J.as(IntoIterator).Iter.as(Iterator).Item == Item`
     - `lt` / `le` / `gt` / `ge(self, other: J) -> bool`（同上 `Ord` 条件，基于 `cmp` 实现）
     - `eq_by(self, other: J, eq_fn: %c fn(*Item, *J.as(IntoIterator).Iter.as(Iterator).Item) -> bool) -> bool`
     - `cmp_by(self, other: J, cmp_fn: %c fn(*Item, *J.as(IntoIterator).Iter.as(Iterator).Item) -> Order) -> Order`

```flurry
pub trait Sum
where
    .A: Type = Self,
{
    fn sum(iter: I) -> Self
    where
        I,
        requires I :- Iterator and I.as(Iterator).Item == A;
}

pub trait Product
where
    .A: Type = Self,
{
    fn product(iter: I) -> Self
    where
        I,
        requires I :- Iterator and I.as(Iterator).Item == A;
}

pub trait Extend
where
    T,
{
    fn extend(*self, iter: I)
    where
        I,
        requires I :- IntoIterator and I.as(IntoIterator).Iter.as(Iterator).Item == T;

    fn extend_one(*self, item: T) {
        self.extend(builtin.iterator.once(item));
    }
}
```

#### 3.4.5 生成器与原语迭代接入（`WL-04-19`）

```flurry
pub fn empty() -> EmptyIter<T> where T { EmptyIter {} }
pub fn once(value: T) -> OnceIter<T> where T { OnceIter { value: value } }
pub fn once_with(f: %c fn() -> T) -> OnceWithIter<T, c>
where
    T,
    c: Type.vars.Closure,
{
    OnceWithIter { make: f }
}
pub fn repeat(value: T) -> RepeatIter<T> where T, requires T :- Clone {
    RepeatIter { value: value }
}
pub fn repeat_with(f: %c fn() -> T) -> RepeatWithIter<T, c>
where
    T,
    c: Type.vars.Closure,
{
    RepeatWithIter { make: f }
}
pub fn repeat_n(value: T, count: usize) -> RepeatNIter<T> where T, requires T :- Clone {
    RepeatNIter { value: value, remaining: count }
}
pub fn successors(first: ?T, succ: %c fn(*T) -> ?T) -> SuccessorsIter<T, c>
where
    T,
    c: Type.vars.Closure,
{
    SuccessorsIter { next: first, step: succ }
}
pub fn from_fn(f: %c fn() -> ?T) -> FromFnIter<T, c>
where
    T,
    c: Type.vars.Closure,
{
    FromFnIter { next: f }
}
```

全量生成器结构体（`EmptyIter`、`OnceIter`、`OnceWithIter`、`RepeatIter`、`RepeatWithIter`、`RepeatNIter`、`SuccessorsIter`、`FromFnIter`）均同时实现 `Iterator` 与 `StepIterator`（适用的亦实现 `DoubleEndedIterator` / `ExactSizeIterator` / `FusedIterator`），并且：
- `OnceIter<T>` 在 `iterate(self)` 中直接移动交付所拥有的仿射元素（不要求 `T :- Copy` 或 `Clone`）；`EmptyIter<T>` 直接正常结束；
- `RepeatNIter<T>` 当 `remaining == 0usize` 时**不调用 `clone()`**，当 `remaining > 0usize` 仅克隆前 `remaining - 1usize` 次，末次直接移动原始 `value` 所有权！
- 原语接入补全：`Array<T, n>` (`IntoIterator` + `iter(*self)`), `Slice<T>` (`IntoIterator` + `iter(*self)`), `Range<Idx>` / `RangeInclusive<Idx>`, `str` (`bytes()` / `chars()` / `char_indices()` / `lines()`), `?T` (`IntoIterator` 产生 0 或 1 项), `!E T` (`IntoIterator` 正常项产出 `T`、错误态空结束；`FromIterator` 支持将元素迭代汇聚为 `!E C` 短路早退！)。

#### 3.4.6 `meta.PureComptimeIterator` 完整对偶家族（`WL-04-20`）

依据 `AGENTS.md` §3 双阶段同名规范与 `self` 值引用原则：
- 在 `library/meta/protocols/iterator.fl`（由 `meta.prelude` 导出）定义 `PureComptimeIterator`、`PureComptimeIntoIterator`、`PureComptimeFromIterator`、`PureComptimeExtend`、`PureComptimeSum`、`PureComptimeProduct`。
- **关键不变量**：
  1. 与 `builtin.Iterator` 使用**完全相同的全部方法名**（无 `comptime_` 前缀）。
  2. 全套采用 `self` 值引用语义——**严禁 `*self`、`.ref`、`.*`**！`any`/`all`/`find`/`position`/`min_by`/`max_by` 等谓词签名接收 `Item` 值引用（`%c comptime fn(Item) -> bool`），而不是 `*Item` 指针。
  3. 允许捕获纯/非纯编译时状态的闭包（区分 `pure comptime trait` 与其中具体实现 `comptime fn` 约束）。

```flurry
pub pure comptime trait PureComptimeIterator {
    assoc Item: Type;

    comptime fn iterate(self) -> #emit_item<Item> void;

    comptime fn foreach(self, f: %c comptime fn(Item)) where c: Type.vars.Closure;
    comptime fn try_foreach(self, f: %c comptime fn(Item) -> !E void) -> !E void
    where
        E,
        c: Type.vars.Closure;
    comptime fn fold(self, init: A, f: %c comptime fn(A, Item) -> A) -> A
    where
        A,
        c: Type.vars.Closure;
    comptime fn try_fold(self, init: A, f: %c comptime fn(A, Item) -> !E A) -> !E A
    where
        A,
        E,
        c: Type.vars.Closure;
    comptime fn reduce(self, f: %c comptime fn(Item, Item) -> Item) -> ?Item
    where
        c: Type.vars.Closure;
    comptime fn try_reduce(self, f: %c comptime fn(Item, Item) -> !E Item) -> !E ?Item
    where
        E,
        c: Type.vars.Closure;

    comptime fn any(self, pred: %c comptime fn(Item) -> bool) -> bool where c: Type.vars.Closure;
    comptime fn all(self, pred: %c comptime fn(Item) -> bool) -> bool where c: Type.vars.Closure;
    comptime fn find(self, pred: %c comptime fn(Item) -> bool) -> ?Item where c: Type.vars.Closure;
    comptime fn find_map(self, f: %c comptime fn(Item) -> ?U) -> ?U
    where
        U,
        c: Type.vars.Closure;
    comptime fn position(self, pred: %c comptime fn(Item) -> bool) -> ?usize where c: Type.vars.Closure;
    comptime fn count(self) -> usize;
    comptime fn nth(self, index: usize) -> ?Item;
    comptime fn last(self) -> ?Item;
    comptime fn collect(self) -> C where C, requires C :- PureComptimeFromIterator<Item>;
    comptime fn sum(self) -> S where S, requires S :- PureComptimeSum<.A = Item>;
    comptime fn product(self) -> P where P, requires P :- PureComptimeProduct<.A = Item>;
    comptime fn partition(self, pred: %c comptime fn(Item) -> bool) -> Tuple<B, B>
    where
        B,
        c: Type.vars.Closure,
        requires B :- ComptimeDefault + PureComptimeExtend<Item>;
    comptime fn unzip(self) -> Tuple<A, B>
    where
        First,
        Second,
        A,
        B,
        requires Item == Tuple<First, Second>
            and A :- ComptimeDefault + PureComptimeExtend<First>
            and B :- ComptimeDefault + PureComptimeExtend<Second>;
    comptime fn min(self) -> ?Item where requires Item :- ComptimeOrd;
    comptime fn max(self) -> ?Item where requires Item :- ComptimeOrd;
    comptime fn min_by(self, compare: %c comptime fn(Item, Item) -> Order) -> ?Item
    where
        c: Type.vars.Closure;
    comptime fn max_by(self, compare: %c comptime fn(Item, Item) -> Order) -> ?Item
    where
        c: Type.vars.Closure;
    comptime fn min_by_key(self, key: %c comptime fn(Item) -> K) -> ?Item
    where
        K,
        c: Type.vars.Closure,
        requires K :- ComptimeOrd;
    comptime fn max_by_key(self, key: %c comptime fn(Item) -> K) -> ?Item
    where
        K,
        c: Type.vars.Closure,
        requires K :- ComptimeOrd;
    comptime fn eq(self, other: J) -> bool
    where
        J,
        requires J :- PureComptimeIntoIterator
            and Item :- ComptimeEq
            and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn ne(self, other: J) -> bool
    where
        J,
        requires J :- PureComptimeIntoIterator
            and Item :- ComptimeEq
            and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn cmp(self, other: J) -> Order
    where
        J,
        requires J :- PureComptimeIntoIterator
            and Item :- ComptimeOrd
            and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn lt(self, other: J) -> bool
    where
        J,
        requires J :- PureComptimeIntoIterator
            and Item :- ComptimeOrd
            and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn le(self, other: J) -> bool
    where
        J,
        requires J :- PureComptimeIntoIterator
            and Item :- ComptimeOrd
            and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn gt(self, other: J) -> bool
    where
        J,
        requires J :- PureComptimeIntoIterator
            and Item :- ComptimeOrd
            and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn ge(self, other: J) -> bool
    where
        J,
        requires J :- PureComptimeIntoIterator
            and Item :- ComptimeOrd
            and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn eq_by(
        self,
        other: J,
        eq_fn: %c comptime fn(Item, J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item) -> bool,
    ) -> bool
    where
        J,
        c: Type.vars.Closure,
        requires J :- PureComptimeIntoIterator;
    comptime fn cmp_by(
        self,
        other: J,
        cmp_fn: %c comptime fn(Item, J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item) -> Order,
    ) -> Order
    where
        J,
        c: Type.vars.Closure,
        requires J :- PureComptimeIntoIterator;

    -- 编译时适配器与运行期严格同名（返回对应 Comptime*Adapter 值语义包装体）
    comptime fn map(self, f: %c comptime fn(Item) -> U) -> ComptimeMapIterator<Self, U, c>
    where
        U,
        c: Type.vars.Closure;
    comptime fn filter(self, pred: %c comptime fn(Item) -> bool) -> ComptimeFilterIterator<Self, c>
    where
        c: Type.vars.Closure;
    comptime fn filter_map(self, f: %c comptime fn(Item) -> ?U) -> ComptimeFilterMapIterator<Self, U, c>
    where
        U,
        c: Type.vars.Closure;
    comptime fn map_while(self, f: %c comptime fn(Item) -> ?U) -> ComptimeMapWhileIterator<Self, U, c>
    where
        U,
        c: Type.vars.Closure;
    comptime fn scan(self, init: A, f: %c comptime fn(A, Item) -> ?U) -> ComptimeScanIterator<Self, A, U, c>
    where
        A,
        U,
        c: Type.vars.Closure;
    comptime fn inspect(self, f: %c comptime fn(Item)) -> ComptimeInspectIterator<Self, c>
    where
        c: Type.vars.Closure;
    comptime fn take(self, count: usize) -> ComptimeTakeIterator<Self>;
    comptime fn take_while(self, pred: %c comptime fn(Item) -> bool) -> ComptimeTakeWhileIterator<Self, c>
    where
        c: Type.vars.Closure;
    comptime fn skip(self, count: usize) -> ComptimeSkipIterator<Self>;
    comptime fn skip_while(self, pred: %c comptime fn(Item) -> bool) -> ComptimeSkipWhileIterator<Self, c>
    where
        c: Type.vars.Closure;
    comptime fn step_by(self, step: usize) -> ComptimeStepByIterator<Self>;
    comptime fn chain(self, next: J) -> ComptimeChainIterator<Self, J.as(PureComptimeIntoIterator).Iter>
    where
        J,
        requires J :- PureComptimeIntoIterator and J.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == Item;
    comptime fn enumerate(self) -> ComptimeEnumerateIterator<Self>;
    comptime fn flat_map(self, f: %c comptime fn(Item) -> J) -> ComptimeFlatMapIterator<Self, J, c>
    where
        J,
        c: Type.vars.Closure,
        requires J :- PureComptimeIterator;
    comptime fn flatten(self) -> ComptimeFlattenIterator<Self> where requires Item :- PureComptimeIterator;
    comptime fn intersperse(self, sep: Item) -> ComptimeIntersperseIterator<Self>
    where
        requires Item :- ComptimeClone;
    comptime fn intersperse_with(self, make: %c comptime fn() -> Item) -> ComptimeIntersperseWithIterator<Self, c>
    where
        c: Type.vars.Closure;
    comptime fn zip(self, other: J) -> ComptimeZipIterator<Self, J.as(PureComptimeIntoIterator).Iter>
    where
        J,
        requires J :- PureComptimeIntoIterator;
    comptime fn cycle(self) -> ComptimeCycleIterator<Self>
    where
        requires Self :- ComptimeClone;
    -- 注：因 comptime 无指针语义且暂定保持推式边界，故意不设 copied/cloned（需 *Inner），并将 rposition/rev/peekable 留待 ComptimeStep/DoubleEnded 边界扩展。
}

pub pure comptime trait PureComptimeIntoIterator {
    assoc Item: Type;
    assoc Iter: Type;

    comptime fn into_iter(self) -> Iter;
}

pub pure comptime trait PureComptimeFromIterator
where
    T,
{
    comptime fn from_iter(iter: I) -> Self
    where
        I,
        requires I :- PureComptimeIterator and I.as(PureComptimeIterator).Item == T;
}

pub pure comptime trait PureComptimeExtend
where
    T,
{
    comptime fn extend(self, iter: I)
    where
        I,
        requires I :- PureComptimeIntoIterator and I.as(PureComptimeIntoIterator).Iter.as(PureComptimeIterator).Item == T;

    comptime fn extend_one(self, item: T) {
        self.extend(meta.iterator.once(item));
    }
}

pub pure comptime trait PureComptimeSum
where
    .A: Type = Self,
{
    comptime fn sum(iter: I) -> Self
    where
        I,
        requires I :- PureComptimeIterator and I.as(PureComptimeIterator).Item == A;
}

pub pure comptime trait PureComptimeProduct
where
    .A: Type = Self,
{
    comptime fn product(iter: I) -> Self
    where
        I,
        requires I :- PureComptimeIterator and I.as(PureComptimeIterator).Item == A;
}
```

---

### 3.5 `builtin.fmt` 与 `builtin.text` 完整规格（`WL-04-21` ~ `WL-04-23`）

#### 3.5.1 `builtin.fmt` 违规清理、格式化特质与 `Formatter` / `Debug*` 构建器（`WL-04-21`）

1. **现行架构偏差纠偏**：
   - **违规修复**：现行 `library/builtin/protocols/formatting/display.fl` 在 `Display` 内声明 `^attrs.comptime_phase comptime fn comptime_fmt(self, .format: str = "") -> str;`，**明确违反 `AGENTS.md` §3 双阶段同名原则**（不得添加 `comptime_` 前缀）。纠偏方案：`Display` 保持纯粹运行期契约；在 `meta`（`library/meta/fmt/display.fl`）中定义独立 `pub pure comptime trait ComptimeDisplay { comptime fn fmt(self, .format: str = "") -> str; }` 与 `ComptimeDebug { comptime fn fmt(self) -> str; }`，使用**完全相同的方法名 `fmt`** 且遵循 `self` 值引用语义！
   - **错误类型归属上移**：将 `FormatErr` 从 `library/std/fmt_impl/parse.fl` 提升至 `builtin.fmt` 并由 `std.fmt` 重导出：

```flurry
pub enum FormatErr {
    UnclosedField(usize),
    UnmatchedClose(usize),
    UnsupportedField(usize),
    InvalidSpec(usize),
    WriteFailed,
}
```

2. **`ByteWriter`、`Formatter` 与结构化 `Debug` 构建器**：

```flurry
pub trait ByteWriter {
    assoc Err: Type;

    fn write_byte(*self, byte: u8) -> !Err void;
    unsafe fn write_raw_bytes(*self, src: usize, len: usize) -> !Err void;
    fn write_str(*self, text: str) -> !Err void;
    fn write_char(*self, ch: char) -> !Err void {
        var buf: Array<u8, 4> = [0u8, 0u8, 0u8, 0u8];
        let encoded = ch.encode_utf8(buf.as_slice()) match {
            s! => s,
            error _ => return self.write_str("\x{FFFD}"),
        }
        self.write_str(encoded)
    }
}

pub struct ByteWriterView {
    data: *void,
    write_byte_fn: unsafe fn(*void, u8) -> !FormatErr void,
    write_raw_bytes_fn: unsafe fn(*void, usize, usize) -> !FormatErr void,
    write_str_fn: unsafe fn(*void, str) -> !FormatErr void,
}

impl ByteWriterView {
    pub fn from_writer(writer: *W) -> Self
    where
        W,
        requires W :- ByteWriter,
    {
        -- 零分配类型擦除视图：将具体 W.as(ByteWriter).Err 映射为 FormatErr.WriteFailed
        builtin.fmt.make_writer_view<W>(writer)
    }
}

pub struct Formatter {
    writer: *ByteWriterView,
    options: FormatOptions,
    is_alternate: bool,
    width: ?usize,
    precision: ?usize,
    fill: char,
    align: FormatAlign,
    sign_plus: bool,
    sign_aware_zero_pad: bool,
}

impl Formatter {
    pub fn write_str(*self, s: str) -> !FormatErr void { builtin.fmt.formatter_write_str(self, s) }
    pub fn write_char(*self, c: char) -> !FormatErr void { builtin.fmt.formatter_write_char(self, c) }
    pub fn pad(*self, s: str) -> !FormatErr void { builtin.fmt.formatter_pad(self, s) }
    pub fn pad_integral(
        *self,
        is_nonnegative: bool,
        prefix: str,
        buf: str,
    ) -> !FormatErr void {
        builtin.fmt.formatter_pad_integral(self, is_nonnegative, prefix, buf)
    }

    pub pure fn alternate(*self) -> bool { self.is_alternate.* }
    pub pure fn sign_plus(*self) -> bool { self.sign_plus.* }
    pub pure fn sign_minus(*self) -> bool { not self.sign_plus.* }
    pub pure fn width(*self) -> ?usize { self.width.* }
    pub pure fn precision(*self) -> ?usize { self.precision.* }
    pub pure fn fill(*self) -> char { self.fill.* }

    pub fn debug_struct(*self, name: str) -> DebugStruct { builtin.fmt.new_debug_struct(self, name) }
    pub fn debug_tuple(*self, name: str) -> DebugTuple { builtin.fmt.new_debug_tuple(self, name) }
    pub fn debug_list(*self) -> DebugList { builtin.fmt.new_debug_list(self) }
    pub fn debug_map(*self) -> DebugMap { builtin.fmt.new_debug_map(self) }
    pub fn debug_set(*self) -> DebugSet { builtin.fmt.new_debug_set(self) }
}
```

完整 `Debug*` 构建器方法集（支持链式配置、缩进美化 `#?` 与安全错误状态传播）：
- **`DebugStruct`**：`field(*self, name: str, value: *V) -> *Self where V :- Debug`、`field_with(*self, name: str, value_fn: %c fn(*Formatter) -> !FormatErr void) -> *Self`、`finish(*self) -> !FormatErr void`、`finish_non_exhaustive(*self) -> !FormatErr void`
- **`DebugTuple`**：`field(*self, value: *V) -> *Self where V :- Debug`、`field_with(*self, value_fn: %c fn(*Formatter) -> !FormatErr void) -> *Self`、`finish(*self) -> !FormatErr void`
- **`DebugList`**：`entry(*self, entry: *V) -> *Self where V :- Debug`、`entries(*self, entries: I) -> *Self where I :- IntoIterator`、`entry_with(*self, entry_fn: %c fn(*Formatter) -> !FormatErr void) -> *Self`、`finish(*self) -> !FormatErr void`
- **`DebugSet`**：`entry(*self, entry: *V) -> *Self where V :- Debug`、`entries(*self, entries: I) -> *Self`、`entry_with(*self, entry_fn)`、`finish(*self) -> !FormatErr void`
- **`DebugMap`**：`entry(*self, key: *K, value: *V) -> *Self where K :- Debug, V :- Debug`、`key(*self, key: *K) -> *Self`、`value(*self, value: *V) -> *Self`、`entries(*self, entries: I) -> *Self`、`finish(*self) -> !FormatErr void`

3. **格式化协议家族补齐与现行 `{format: .hex}` 设计调和**：

```flurry
-- 保留属性化 Display / FormatOptions 快速路径（可无 Formatter 分配直接向任意 ByteWriter 输出）
pub trait Display {
    fn fmt(*self, output: *W, comptime .format: str = "") -> !W.as(ByteWriter).Err void
    where
        W,
        requires W :- ByteWriter;
}

-- 统一 Debug 与进位/指数/指针格式化特质家族（接收 *Formatter，与 Formatter 格式指令对接）
pub trait Debug {
    fn fmt(*self, f: *Formatter) -> !FormatErr void;
}
pub trait Binary { fn fmt(*self, f: *Formatter) -> !FormatErr void; }
pub trait Octal { fn fmt(*self, f: *Formatter) -> !FormatErr void; }
pub trait LowerHex { fn fmt(*self, f: *Formatter) -> !FormatErr void; }
pub trait UpperHex { fn fmt(*self, f: *Formatter) -> !FormatErr void; }
pub trait LowerExp { fn fmt(*self, f: *Formatter) -> !FormatErr void; }
pub trait UpperExp { fn fmt(*self, f: *Formatter) -> !FormatErr void; }
pub trait Pointer { fn fmt(*self, f: *Formatter) -> !FormatErr void; }
```

> **设计调和声明（详列于 §5 Q6）**：现行 `library/builtin/protocols/formatting/README.md` 与 `library/std/fmt_impl/README.md` 曾通过 `FormatOptions`（如 `.format = "{format: .hex}"`）在单条 `Display.fmt` 中路由十六进制与定点精度，明确“不再拆出 `LowerHex`/`UpperHex`/`Binary`/`Octal`”。本规格给出完整多 trait 模型与现行单 trait `FormatOptions` 路由两套设计，并建议以 `Display`/`Debug`/`Binary`/`Octal`/`LowerHex`/`UpperHex`/`LowerExp`/`UpperExp`/`Pointer` 兼容二者：默认无格式修饰 `{}` 走 `Display`；显式扩展特质提供独立语义（如对自定义句柄提供 `Pointer` 或 `LowerHex` 输出），而无需把整数十六进制选项从 `Display` 中剔除。同时将现有 `Debug.write_debug` 统一收敛到 `Debug.fmt(*self, f: *Formatter)`（迁移期提供桥接默认适配器）。

#### 3.5.2 复合原语 `Display` / `ComptimeDisplay` / `Debug` 补齐（`WL-04-22`）

| 类型 | 运行期 `Display` / `Debug` | `Binary`/`Octal`/`*Hex`/`*Exp`/`Pointer` | `meta.ComptimeDisplay` (`fmt(self, .format)`) |
|---|---|---|---|
| `f16`/`bf16`/`f32`/`f64` | 已有 `Display` / `Debug` | 补齐 `LowerExp`/`UpperExp` | **新增补全**：Dragonbox/Ryu 编译时短十进制与科学计数法输出 |
| `?T` | `Debug`: `value?` 显示 `T?`，`null` 显示 `null`；`Display`: 若 `T :- Display`（需明确是否允许或仅提供 `Debug`） | — | `ComptimeDebug` 对 `?T` |
| `!E T` / `![.Es = Es] T` | `Debug`: `val!` 或 `error <E>` | — | `ComptimeDebug` 对 `!E T` 与异构错误行 |
| `Tuple<.Es = Es>` | `Debug`（利用 `'tuple_indexed` + `DebugTuple` 产出 `(a, b)` 或 `(a,)`） | — | `ComptimeDebug`（利用 `'tuple_indexed` 值语义遍历） |
| `Array<T, n>` | `Debug`（通过 `DebugList` 产出 `[a, b, ...]`） | — | `ComptimeDebug` 对 `Array<T, n>` |
| `Slice<T>` | `Debug`（放开不必要 `T :- Copy`，支持仅 `T :- Debug` 引用访问！） | — | `ComptimeDebug` |
| `*T` | `Debug`（`0x...`十六进制地址） | `Pointer`（全平台指针宽十六进制输出） | **故意省略**（comptime 无指针） |
| `BitVec<N>` | `Debug`（`0b...` 或 `0x...`） | `Binary`, `Octal`, `LowerHex`, `UpperHex` | `ComptimeDisplay` / `ComptimeDebug` |

#### 3.5.3 `builtin.text` UTF-8、码点与 `FromStr` 协议（`WL-04-23`）

1. **反膨胀硬边界**：
   `builtin.text` **仅承担**无堆分配基础文本原语：UTF-8 编码合法性校验（`from_utf8` / `Utf8Err`）、UTF-8 码点流与 ASCII 属性、`str` 字节/字符边界切分、单/谓词查找与基础数值解析（`FromStr`）。
   - **明确禁止内置于 `builtin` 或 `std`**：正则表达式引擎（`RegexMatcher`，严格属 Tier 2/3 生态包）、复杂 Unicode 算法（UAX #29 字素簇分割、NFC/NFD 正规化、UCA 排序、多语言本地化大小写规则）、遗留字符集编解码（GBK/Shift-JIS）。若需扩展字素分割，只保留高层 Unicode 生态包接口边界。

2. **UTF-8 验证原语与错误类型**：

```flurry
pub enum Utf8Err {
    IncompleteSequence { valid_up_to: usize },
    InvalidByte { valid_up_to: usize, error_len: u8 },
}

impl Utf8Err {
    pub pure fn valid_up_to(self) -> usize {
        self match {
            Utf8Err.IncompleteSequence { valid_up_to } => valid_up_to,
            Utf8Err.InvalidByte { valid_up_to, .. } => valid_up_to,
        }
    }

    pub pure fn error_len(self) -> ?u8 {
        self match {
            Utf8Err.IncompleteSequence { .. } => null,
            Utf8Err.InvalidByte { error_len, .. } => error_len,
        }
    }
}

pub pure fn from_utf8(bytes: Slice<u8>) -> !Utf8Err &bytes str {
    builtin.text.validate_utf8_and_wrap(bytes)
}

pub unsafe fn from_utf8_unchecked(bytes: Slice<u8>) -> &bytes str {
    builtin.text.wrap_utf8_unchecked(bytes)
}
```

3. **`FromStr` 与 `ComptimeFromStr` 及解析错误家族**：

```flurry
pub enum ParseBoolErr { InvalidFormat }

pub enum ParseCharErr {
    EmptyString,
    TooManyChars,
}

pub enum ParseIntErr {
    Empty,
    InvalidDigit,
    PosOverflow,
    NegOverflow,
    Zero,
}

pub enum ParseFloatErr {
    Empty,
    Invalid,
}

pub trait FromStr {
    assoc Err: Type;

    fn from_str(input: str) -> !Err Self;
}

pub pure comptime trait ComptimeFromStr {
    assoc Err: Type;

    comptime fn from_str(input: str) -> !Err Self;
}
```

- 实现矩阵：`bool` (`"true"` / `"false"` -> `!ParseBoolErr bool`)、`char` (恰好单 Unicode scalar -> `!ParseCharErr char`)、全部 12 种定长机器整数 (`u8..u128`, `usize`, `i8..i128`, `isize` -> `!ParseIntErr Self`)、全部 4 种浮点 (`f16`, `bf16`, `f32`, `f64` -> `!ParseFloatErr Self`，使用精确十进制转二进制舍入 `Eisel-Lemire` + fallback) 及 `meta.ComptimeFromStr` 精确同形实现。

---

### 3.6 `builtin.resources` 与 `builtin.mem` 完整规格（`WL-04-24` ~ `WL-04-27`）

#### 3.6.1 `Once`, `Clone`, `Copy`, `Drop` 与运行期依赖类型析构（`WL-04-24`）

1. **`Once` 单一全称契约（严禁逐类型重复实现）**：
   保持 `library/builtin/protocols/resources.fl` 中的唯一规范定义，全仓库**绝不**再为标量、枚举、结构体、元组、数组、可选或错误结果编写重复 `impl Once`：

```flurry
pub trait Once {}
impl Once for T where T {}

pub trait Clone :- Once {
    fn clone(*self) -> Self;
}

pub trait Copy :- Clone {}

pub trait Drop {
    fn drop(*self);
}
```

   （对应的纯编译期对偶 trait `meta.ComptimeClone` 签名采用 **`comptime fn clone(self) -> Self;`**，使用 `self` 值引用语义、零 `*self`。）

2. **复合原语的 `Clone` / `Copy` / `Drop` 补齐**：
   - **`?T`**：拆分为两条互不冲突的实现——`impl Clone for ?T where T, requires T :- Clone`（不再要求 `Copy`！）与 `impl Copy for ?T where T, requires T :- Copy {}`。
   - **`!E T` 与异构错误行 `![.Es = Es] T`**：放宽 `Clone` 约束至 `T :- Clone and E :- Clone`（对错误行则 `T :- Clone and Es.all(|E| E :- Clone)`），解除现行 `native_resources.fl` 仅覆盖 `Copy` 的限制。
   - **`Array<T, n>`、`Tuple<.Es = Es>`、`Uint<W>`、`Int<W>`、`BitVec<N>`**：统一补全 `Clone`/`Copy` 与基于通用 `needs_drop` 传播的 `Drop` Elaboration。
3. **运行期依赖类型实例的 `Drop`（联动 `issues/0003`）**：
   依据 `docs/type/runtime-dependent-type.md` 与 `docs/resource-management/drop-elaboration.md`，类似 `Vec<T, .A = A, .allocator = alloc>` 的容器将运行期分配器指针提升为类型级 `quote .allocator: lift *A` 依赖参数，因此其析构声明为：

```flurry
impl Drop for Vec<T, .A = A, .allocator = alloc>
where
    T,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    fn drop(*self) {
        self.clear_elements();
        dealloc_vec_buffer<T, A>(alloc, self.raw_addr.*, self.raw_cap.*);
    }
}
```

   **编译器支持缺口**：Elaboration 与 Drop elaboration pass 在合成隐式退场清理点、部分移动（partial move）回退分支或错误早退 `!` 传播清理块时，必须同步捕获并向 `Drop.drop` 交付类型实例附带的 lifted runtime environment 参数（与普通方法调用走统一环境交付计划），而不是因类型表找不到无参 `Drop` 签名而跳过或崩溃。

#### 3.6.2 Pin-Free 移动语义与自引用边界（`WL-04-25`）

- **文档纠偏**：`docs/type/self-reference.md` 规范的是 `Self`（构造名）、`Itself`（当前目标完整类型）与 `itself`（实例指代）三个**反身代词**；Flurry 的 **Pin-free（无需 `Pin<T>`）移动与重新初始化**模型实际规定在 `docs/resource-management/moves-and-initialization.md` 与 `docs/resource-management/drop-elaboration.md`。
- **核心设计边界**：
  1. 值默认按位移动（bitwise move）；源位置在移动后进入“已移出（moved-out）”逻辑状态，不再参与隐式 `Drop`，直到全槽位或逐字段合法重初始化。
  2. 原生值语义不引入类似 Rust 的 `Unpin` / `Pin<P>` 双重包装传染；自引用数据结构在安全层优先通过基于基地偏移量（relative offset / handle index）表达，或在 `builtin` 内建协程/状态机边界严格由编译器管理槽位冻结周期，确保用户层安全代码无法观测到悬垂自引用内部指针。关于是否需要额外的不可移动标记或 `^attrs.mem.immovable` 协议，列入 §5 Q7。

#### 3.6.3 `builtin.mem` 核心载体与非安全原语全集（`WL-04-26`, `WL-04-27`）

1. **`Layout`（由 `std.mem` 提升至 `builtin.mem`）与 `Align`**：
   将现行 `library/std/mem_impl/layout.fl` 中的 `Layout` 移入 `builtin.mem`（`std.mem` 保留透传重导出），并新增强类型对齐载体 `Align`：

```flurry
pub struct Align {
    pow2_val: usize,
}

impl Clone for Align {
    fn clone(*self) -> Self { self.* }
}
impl Copy for Align {}

impl Align {
    pub pure fn new(align: usize) -> ?Self {
        if align.is_power_of_two() {
            return Self { pow2_val: align };
        }
        null
    }

    pub unsafe fn new_unchecked(align: usize) -> Self { Self { pow2_val: align } }
    pub pure fn of() -> Self where T { Self { pow2_val: Type.align_of(T) } }
    pub pure fn as_usize(self) -> usize { self.pow2_val }
}
```

2. **`NonNull<T>`、`MaybeUninit<T>`、`ManuallyDrop<T>` 与 `TypeMarker<T>`（全部成员零省略）**：

```flurry
-- 1) 零开销非空指针（支持 ?NonNull<T> 的编译器空指针 niche 优化契约）
pub struct NonNull
where
    T,
{
    raw: *T,
}

impl Clone for NonNull<T> where T {
    fn clone(*self) -> Self { self.* }
}
impl Copy for NonNull<T> where T {}

impl NonNull<T>
where
    T,
{
    pub pure fn dangling() -> Self {
        let align = Type.align_of(T);
        let addr = if align == 0usize { 1usize } else { align };
        Self { raw: builtin.mem.ptr_without_provenance<T>(addr) }
    }

    pub pure fn new(ptr: *T) -> ?Self {
        if ptr.is_null() { return null; }
        Self { raw: ptr }
    }

    pub unsafe fn new_unchecked(ptr: *T) -> Self { Self { raw: ptr } }
    pub pure fn addr(self) -> usize { self.raw.addr() }
    pub pure fn as_ptr(self) -> *T { self.raw }
    pub pure fn cast(self) -> NonNull<U> where U { NonNull { raw: self.raw.cast<U>() } }
    pub unsafe fn as_ref(self) -> *T { self.raw }
    pub unsafe fn offset(self, count: isize) -> Self { Self { raw: self.raw.offset(count) } }
    pub unsafe fn add(self, count: usize) -> Self { Self { raw: self.raw.add(count) } }
    pub unsafe fn sub(self, count: usize) -> Self { Self { raw: self.raw.sub(count) } }
    pub unsafe fn read(self) -> T { self.raw.read() }
    pub unsafe fn write(self, value: T) { self.raw.write(value) }
}

-- 2) 未初始化存储包装：编译器不为其载荷生成自动 Drop，也不假定任何位有效性预设
pub struct MaybeUninit
where
    T,
{
    storage: builtin.compiler.UninitStorage<T>,
}

impl MaybeUninit<T>
where
    T,
{
    pub pure fn uninit() -> Self { builtin.mem.make_uninit<T>() }
    pub pure fn zeroed() -> Self { builtin.mem.make_zeroed_uninit<T>() }
    pub fn new(value: T) -> Self {
        var slot = Self.uninit();
        unsafe { slot.as_mut_ptr().write(value); }
        slot
    }

    pub pure fn as_ptr(*self) -> *T { builtin.mem.uninit_as_ptr<T>(self) }
    pub pure fn as_mut_ptr(*self) -> *T { builtin.mem.uninit_as_ptr<T>(self) }

    pub fn write(*self, value: T) -> *T {
        let p = self.as_mut_ptr();
        unsafe { p.write(value); }
        p
    }

    pub unsafe fn assume_init(self) -> T { builtin.mem.uninit_assume_init<T>(self) }
    pub unsafe fn assume_init_read(*self) -> T { self.as_ptr().read() }
    pub unsafe fn assume_init_drop(*self) { self.as_mut_ptr().drop_in_place(); }

    pub pure fn uninit_array() -> Array<MaybeUninit<T>, n> where n: usize {
        Array<MaybeUninit<T>, n>.from_fn(|_| MaybeUninit<T>.uninit())
    }
    pub unsafe fn array_assume_init(arr: Array<MaybeUninit<T>, n>) -> Array<T, n>
    where
        n: usize,
    {
        builtin.mem.array_assume_init<T, n>(arr)
    }
}

-- 3) 显式抑制自动 Drop 展开的所有权包装（允许在 Drop 前安全移动出内部值或按自定义顺序析构）
pub struct ManuallyDrop
where
    T,
{
    storage: builtin.compiler.ManuallyDropStorage<T>,
}

impl ManuallyDrop<T>
where
    T,
{
    pub fn new(value: T) -> Self { builtin.mem.manually_drop_new<T>(value) }
    pub fn into_inner(slot: Self) -> T { builtin.mem.manually_drop_into_inner<T>(slot) }
    pub pure fn as_ptr(*self) -> *T { builtin.mem.manually_drop_ptr<T>(self) }
    pub unsafe fn take(*self) -> T { self.as_ptr().read() }
    pub unsafe fn drop(*self) { self.as_ptr().drop_in_place(); }
}

-- 4) 零大小类型标记（TypeMarker<T>）：保留型变与泛参存在性，无运行期大小，不触发 Drop
pub struct TypeMarker
where
    T,
{}

impl Clone for TypeMarker<T> where T {
    fn clone(*self) -> Self { TypeMarker {} }
}
impl Copy for TypeMarker<T> where T {}
impl Default for TypeMarker<T> where T {
    fn default_value() -> Self { TypeMarker {} }
}
```

3. **类型大小/对齐、原始/易失读写、`transmute` 边界与 `MemoryView`（`WL-04-27`）**：

```flurry
-- 运行期/多态尺寸与对齐查询（完善 Type.size_of(T) 的通用接口）
pub pure fn size_of() -> usize where T { Type.size_of(T) }
pub pure fn align_of() -> usize where T { Type.align_of(T) }
pub pure fn size_of_val(val: *T) -> usize where T { builtin.mem.val_size<T>(val) }
pub pure fn align_of_val(val: *T) -> usize where T { builtin.mem.val_align<T>(val) }
pub pure fn needs_drop() -> bool where T { builtin.mem.type_needs_drop<T>() }

-- 安全漏值原语（显式消费仿射值而不执行 Drop：合法不破坏内存安全，仅放弃清理）
pub fn forget(value: T) where T {
    let _ = ManuallyDrop.new(value);
}

-- 统一类型化指针内存操作（收编 raw.fl 的 copy_bytes/raw_read 等裸 usize 接口）
pub unsafe fn read(src: *T) -> T where T { builtin.mem.intrinsic_read<T>(src) }
pub unsafe fn write(dst: *T, value: T) where T { builtin.mem.intrinsic_write<T>(dst, value) }
pub unsafe fn read_unaligned(src: *T) -> T where T { builtin.mem.intrinsic_read_unaligned<T>(src) }
pub unsafe fn write_unaligned(dst: *T, value: T) where T { builtin.mem.intrinsic_write_unaligned<T>(dst, value) }
pub unsafe fn copy(src: *T, dst: *T, count: usize) where T {
    copy_bytes(dst.addr(), src.addr(), count * Type.size_of(T));
}
pub unsafe fn copy_nonoverlapping(src: *T, dst: *T, count: usize) where T {
    copy_bytes_nonoverlapping(dst.addr(), src.addr(), count * Type.size_of(T));
}
pub unsafe fn write_bytes(dst: *T, byte: u8, count: usize) where T {
    set_bytes(dst.addr(), byte, count * Type.size_of(T));
}
pub unsafe fn swap(x: *T, y: *T) where T {
    if x.addr() != y.addr() {
        let tmp = x.read();
        copy_nonoverlapping(y, x, 1usize);
        y.write(tmp);
    }
}
pub unsafe fn replace(dst: *T, src: T) -> T where T {
    let old = dst.read();
    dst.write(src);
    old
}
pub unsafe fn drop_in_place(dst: *T) where T {
    builtin.mem.intrinsic_drop_in_place<T>(dst);
}
pub unsafe fn zeroed() -> T where T {
    MaybeUninit<T>.zeroed().assume_init()
}

-- Volatile / MMIO 访问（与 ^attrs.mem.volatile 协同：发射 load volatile / store volatile，归入纯度 P0）
pub unsafe fn volatile_read(src: *T) -> T where T {
    builtin.mem.intrinsic_volatile_read<T>(src)
}
pub unsafe fn volatile_write(dst: *T, value: T) where T {
    builtin.mem.intrinsic_volatile_write<T>(dst, value)
}

-- 受编译期等大布局契约保护的位级重解释（仅当编译期能证 Type.size_of(Src) == Type.size_of(Dst) 时放行实例化）
pub unsafe fn transmute(src: Src) -> Dst
where
    Src,
    Dst,
    requires Type.size_of(Src) == Type.size_of(Dst),
{
    builtin.mem.intrinsic_transmute<Src, Dst>(src)
}
```

- **`MemoryView` 区域不重叠与可分离视图契约**：
  依据 `docs/resource-management/memory-views.md` 与 `docs/resource-management/contracts-and-verification.md`，`builtin.mem` 提供纯地址区间不相交校验与切片拆分辅助谓词/合约接口，供集合（如 `Slice.split_at_disjoint`、`Array` 同时修改两个不同索引）及 `verification` 验证层桥接（严禁让 PTC 读取任何运行时内存状态！）：

```flurry
pub pure fn addresses_disjoint(
    a_addr: usize,
    a_bytes: usize,
    b_addr: usize,
    b_bytes: usize,
) -> bool {
    if a_bytes == 0usize or b_bytes == 0usize {
        return true;
    }
    let a_end = a_addr + a_bytes;
    let b_end = b_addr + b_bytes;
    (a_end <= b_addr) or (b_end <= a_addr)
}

pub trait MemoryView {
    assoc Element: Type;

    fn view_range(*self) -> Tuple<usize, usize>;
    fn is_disjoint_from(*self, other: *Other) -> bool
    where
        Other,
        requires Other :- MemoryView;
}
```

---

### 3.7 `language-items.conf`、NIR Intrinsics 与编译器支撑需求（`WL-04-28`）

1. **`library/language-items.conf` 建议新增/对齐条目**：

```text
-- 类型绑定新增
array-type = builtin.primitives.Array
uninit-storage-type = builtin.compiler.UninitStorage
manually-drop-type = builtin.compiler.ManuallyDropStorage
arbitrary-int-type = builtin.primitives.ArbitrarySizedInteger

-- 协议绑定新增（与已有 copy-protocol / iterator-protocol 并列）
optional-ops-protocol = builtin.types.OptionalOps
optional-unwrap-protocol = builtin.types.OptionalUnwrap
error-ops-protocol = builtin.types.ErrorResultOps
error-unwrap-protocol = builtin.types.ErrorResultUnwrap
error-row-ops-protocol = builtin.types.ErrorRowOps
from-str-protocol = builtin.text.FromStr
extend-protocol = builtin.iterator.Extend
sum-protocol = builtin.iterator.Sum
product-protocol = builtin.iterator.Product
pure-comptime-iterator-protocol = meta.iterator.PureComptimeIterator
step-iterator-protocol = builtin.iterator.StepIterator
double-ended-iterator-protocol = builtin.iterator.DoubleEndedIterator

-- host-intrinsic.* 与 comptime-intrinsic.* 扩展家族
host-intrinsic.tan, asin, acos, atan, atan2, sincos
host-intrinsic.sinh, cosh, tanh, asinh, acosh, atanh
host-intrinsic.cbrt, hypot, expm1, log1p, erf, erfc, tgamma, lgamma, remquo, fdim, nextafter
host-intrinsic.uadd_with_overflow, sadd_with_overflow, usub_with_overflow, ssub_with_overflow, umul_with_overflow, smul_with_overflow
host-intrinsic.uint_widening_mul, sint_widening_mul, wrapping_shl, wrapping_lshr, wrapping_ashr
host-intrinsic.smin, smax, umin, umax, uadd_sat, sadd_sat, usub_sat, ssub_sat
host-intrinsic.f16_to_bits, f16_from_bits, bf16_to_bits, bf16_from_bits, f32_to_bits, f32_from_bits, f64_to_bits, f64_from_bits
host-intrinsic.volatile_read, volatile_write, drop_in_place, transmute
comptime-intrinsic.<与上述纯数学、位重解释、溢出及字符表对应的全集对偶>
```

2. **`compiler/nir/src/intrinsics.rs` 与 Elaboration 最小通用改造清单**：
   - 将 `nir::intrinsics::Operation` 从“单输出标量或 `(Scalar, i32)`”推广为结构化签名模型（支持 `Tuple<T, bool>` 溢出结果、`Tuple<T, i32>` 浮点余数/符号返回、`f16`/`bf16` 全超越/指数降码及 SIMD 向量参数）；
   - 将有/无符号饱和与极值显式分裂为独立 opcode（消除依赖实参上下文推断有/无符号性的脆弱启发）；
   - 补齐 `str` backing-region provenance 传递（解除 `development/repros/native-text-method-provenance.fl`）、多错误行结构化匹配（解除 `cc178adb`）以及运行期依赖类型环境 `Drop` thunk 合成（解除 `issues/0003`）。

---

### 3.8 端到端用法正例与必须拒绝的反例

#### 正例 A：混合原语运算、原生 `?T` / `!E T` 与零 `Option`/`Result`

```flurry
use builtin.math.consts.f64;

fn parse_normalized_angle(raw: str) -> ![ParseFloatErr, OutOfRangeErr] f64 {
    let trimmed = raw.trim();
    let deg = trimmed.parse<f64>()!;
    if not deg.is_finite() or deg < -360.0f64 or deg > 360.0f64 {
        return error OutOfRangeErr.InvalidBounds;
    }
    let rad = deg.to_radians();
    (rad.sin() * rad.cos()).clamp(-1.0f64, 1.0f64)
}
```

#### 正例 B：运行期推式/步进迭代 vs 编译期同名 `PureComptimeIterator`

```flurry
comptime fn sum_even_squares(xs: meta.List<i32>) -> i32 {
    xs.iter()
        .filter(|x| (x % 2i32) == 0i32)
        .map(|x| x * x)
        .sum()
}

fn sum_runtime(xs: Slice<i32>) -> i32 {
    xs.iter()
        .copied()
        .filter(|x| (x.* % 2i32) == 0i32)
        .map(|x| x * x)
        .sum()
}
```

#### 正例 C：FFI 易失 MMIO + `MaybeUninit` 零拷贝构造（无 `extern` / 无 `handles`）

```flurry
use attrs.abi;
use builtin.mem.{MaybeUninit, volatile_read, volatile_write}

^attrs.abi(.c)
pub unsafe fn read_device_words(reg_addr: *u32) -> Array<u32, 4> {
    var out = MaybeUninit<u32>.uninit_array<4>();
    var i: usize = 0usize;
    while i < 4usize {
        let word = volatile_read<u32>(reg_addr);
        let _ = out.get(i).unwrap().write(word);
        i = i + 1usize;
    }
    MaybeUninit<u32>.array_assume_init<4>(out)
}
```

#### 反例（编译器 / 静态检查 / 规范审计必须拒绝）

1. **反例 1（违反集中绑定或引入 `Option`/`Result` 与禁止语法）**：
   - 在任意业务函数内直接写 `.u32'builtin`、`.Tuple'builtin`；
   - 定义替代原生 `?T` / `!E T` 的双变体错误/可选包装枚举；
   - 在 Flurry 代码中使用双冒号路径、C 风格注释、符号逻辑运算符或 `extern` / `handles` 关键字。
   - **拒绝原因**：违反集中绑定隔离、原生限定类型契约及正式语法规范（`AGENTS.md` §5、`docs/grammar/syntax`）。
2. **反例 2（违反 Comptime 无指针与双阶段同名规则）**：
   - 在 `PureComptimeIterator`、`ComptimeOptionalOps` 或 `ComptimeDisplay` 中写 `*self`、`.ref`、`.*` 或 `*meta.List<T>`，或命名方法为 `comptime_map` / `comptime_fmt`。
   - **拒绝原因**：comptime 严禁指针表示与暴露；双阶段同名规范禁止 `comptime_` 前缀（`AGENTS.md` §3, §5）。
3. **反例 3（违反单一全称 `Once` 或越权使用 `transmute`）**：
   - 在任意库文件中编写 `impl Once for bool {}` 或 `impl Once for MyType {}`；
   - 试图执行 `unsafe { builtin.mem.transmute<u32, u64>(1u32) }`。
   - **拒绝原因**：`Once` 已由唯一全称 `impl Once for T where T {}` 赋予全部类型；`transmute` 的 `requires Type.size_of(Src) == Type.size_of(Dst)` 编译时谓词直接拒绝不等大重解释。
4. **反例 4（Tier 1 生态膨胀越界）**：
   - 将 `RegexMatcher`（正则引擎）、`Serialize`（JSON/CBOR/TOML）、HTTP/TLS/Crypto 或重型 UCA/Locale 排序表塞入 `builtin.text` 或 `std`。
   - **拒绝原因**：违背 6 个内置包极简基线边界（Tier 2 官方薄协议包 `regex`/`serialization` 等独立置于内置包之外）。

---

## 4. 排期优先级与分阶段交付路线

| 阶段 | 包含项目 ID | 核心前置依赖 | 验收标准（Exit Criteria） |
|---|---|---|---|
| **Phase 0（P0 核心地基与阻塞清理）** | `WL-04-02`, `WL-04-03`, `WL-04-10`, `WL-04-11`, `WL-04-13`, `WL-04-14`, `WL-04-24`, `WL-04-26`, `WL-04-27`, `WL-04-28` | 1. NIR/LLVM：溢出整数/浮点位重解释/`UninitStorage`/`ManuallyDropStorage`/`Array` 类型绑定；<br>2. Elaboration：`str` provenance、多错误行匹配（`cc178adb`）、运行期 env `Drop` thunk 合成、通用 `meta.List` `where` 展开 | 定长整数与浮点全套固有方法可编译运行；`Array<T, N>` + `MaybeUninit`/`ManuallyDrop`/`NonNull`/`Layout` 支撑 `std.collections.Vec` 消除硬编码地址算术；`?T`/`!E T` 解包与 `Clone` 解锁；`Display` 移除违规 `comptime_fmt` |
| **Phase 1（P0 文本、数学与双阶段迭代闭环）** | `WL-04-05`, `WL-04-12`, `WL-04-15`, `WL-04-18`, `WL-04-19`, `WL-04-20`, `WL-04-23` | `str` provenance 修复；push/pull 边界裁决（§5 Q5）；`Slice<T>` 包归属确定（§5 Q1）；新增 `target.Scalar` & `meta.math` 映射 | `builtin.text` + `FromStr`（`ParseIntErr`/`ParseFloatErr`/`Utf8Err`）无堆验证与数值解析端到端跑通；`builtin.iterator` 补齐 `sum`/`product`/`partition`/`unzip`/`zip`/`cycle`/`rev`/全部生成器，并与 `meta.PureComptimeIterator` 实现同名对称；`Slice<T>` 完成全部切片/二分/排序算法 |
| **Phase 2（P1 完善原语、常量、SIMD 与格式化体系）** | `WL-04-01`, `WL-04-04`, `WL-04-06`, `WL-04-07`, `WL-04-08`, `WL-04-09`, `WL-04-16`, `WL-04-17`, `WL-04-21`, `WL-04-22` | 作者对 §5 Q2/Q3/Q4/Q6 裁决；SIMD target feature 映射；>128 位 LLVM `ArbitrarySizedInteger` legalizer；Unicode 压缩只读数据表 | `bool`/`char`/`Symbol`/`*T`/`BitVec<N>`/`Uint<W>`/`Int<W>` 全套方法就绪；`builtin.math.consts` 与 `meta.math.consts` 全 22 常量精确落地；`Formatter`、`DebugStruct..DebugSet` 与全复合原语 `Display`/`Debug`/`ComptimeDisplay` 通过集成测试 |
| **Phase 3（P2 高级内存契约与验证衔接）** | `WL-04-25`, `MemoryView` 形式化拓展 | `verification`（`WL-09`）规约协同；`docs/resource-management/` 与空白 `docs/` 补全 | `MemoryView` 不相交证明与 `Slice.swap_with_slice`/`Array` 双写解构在静态检查及证明层全线打通；Pin-free 自引用边界明确书面落地 |

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

| 编号 | 议题 | 冲突 / 空白精确来源 | 备选方案与最小推荐决定 |
|---|---|---|---|
| **Q1** | `Slice<T>` 的最终包归属 | 现状置于 `library/std/collections_impl/slice.fl`（导出于 `std.collections`/`std.prelude`）；但 `docs/control-flow/pattern-matching/index.md:853`、`docs/resource-management/memory-views.md` 与 `str` 等价设计高度底层，`Array.as_slice` 与 `char.encode_utf8`（属 `builtin`）若需返回或接收 `Slice<u8>`，则会引发 `builtin` 反向依赖 `std` 的循环问题 | **选项 A（推荐）**：将 `Slice<T>` 迁入 `builtin`（例如 `builtin.mem.Slice` 或 `builtin.Slice`，并由 `std.collections.Slice` 与 `std.prelude.Slice` 保持透明重导出；同时明确 `builtin.mem` 为 `MaybeUninit`/`ManuallyDrop`/`Layout` 唯一源，`std.mem` 只做重导出，并与 `07` 对齐 `unsafe swap`/`unsafe zeroed` 契约）；<br>**选项 B**：保持 `Slice<T>` 在 `std`，`builtin` 仅操作原生 `*T` + `usize` 或 `str`，待 `std` 层再包装 |
| **Q2** | 底类型唯一正式表面名：`NoReturn` vs `never` | `docs/basics/lexical.md:95`、`docs/type/comptime-gradual-dependent-type.md:334` 与 `library/builtin/primitives.fl:13` 使用 `NoReturn`；`docs/control-flow/basic.md:43,53,126` 全部称为 `never` | **选项 A（推荐）**：以 `NoReturn` 作为集中绑定规范实体名，同时在 `builtin.prelude` 保留 `pub typealias never = NoReturn;`（零额外成本）；<br>**选项 B**：以 `never` 替换 `NoReturn` 并废弃后者 |
| **Q3** | 可达性限定 `&r T` 的空文档与子类型规约 | `docs/type/reachability-type.md`（0 字节）、`docs/type/qualified-types/reachability-type-also.md`（0 字节）；目前仅见 `docs/grammar/syntax:211` 与 `docs/resource-management/reachability.md:131` | 请作者确认：<br>1. `&r1 &r2 T <: &r1 T` 交集放宽/子类型方向；<br>2. 字面量（如 `str` 字面量、只读全局常量、`Symbol.as_str()`）默认是否携带由语言/prelude 导出的静态具名 `ReachSet` 常量（如 `builtin.reachability.STATIC`）而非引入未定义生命周期语法；<br>3. `str`/`Slice<T>` 原生方法返回切片时是隐式绑定 `&self`（receiver reachability）还是必须显式在签名写作 `&self str` |
| **Q4** | `ArbitrarySizedInteger` 空白规范、`u<N>`/`i<N>` 语法及 >128b ABI | `docs/others/arbitary-sized-integers.md`（0 字节）；`docs/type/user-defined-types/structs.md:169` 出现 `u3`，`enums.md:33` 称 `ArbitrarySizedInteger`；`ai_dev_logs/codebase_audit_report_2026-11-22.md:88` 报告 >128 位 runtime 尚缺 | 请作者裁决：<br>1. 正式表面构造器命名选用 `Int<W>`/`Uint<W>` 还是 `ArbitrarySizedInteger<W, .signed = ...>`（以及是否对 `u1..u128`/`i1..i128` 批量导出或由词法支持）；<br>2. `W > 128` 时在 LLVM host ABI 中的对齐准则（按 64/128 位自然对齐还是紧凑字节对齐）与 `u128` 截断禁止边界 |
| **Q5** | `builtin.iterator` 推式/拉式协同边界（`zip`/`peekable`/`DoubleEndedIterator`/`eq`/`cmp`） | `library/builtin/protocols/iterator.fl` 纯推式 `#emit_item` 无法独立同时驱动两条任意推流 | **选项 A（混合推荐）**：引入 `StepIterator :- Iterator`（`fn next(*self) -> ?Item`）与 `DoubleEndedIterator :- StepIterator`，对常见可步进数据源走零开销锁步路径，仅对纯推式不可步进源通过 `builtin.Continuation` 自动协程挂起桥接；<br>**选项 B（纯协程统一）**：不公开 `StepIterator` trait，所有多流操作内部统一使用 compiler-inlined 非逃逸 `Continuation` 挂起 `#emit_item` |
| **Q6** | `builtin.fmt` 进制/指数特质 vs `FormatOptions` 分歧与 `Display`/`Debug` 双阶段收口 | `library/builtin/protocols/formatting/README.md` 与 `library/std/fmt_impl/README.md` 明确“不再拆出 `LowerHex`/`UpperHex`/`Binary`/`Octal`”；而多协议需求包含 `Formatter` 与独立进位/指数/`Pointer`特质；且 `display.fl` 现存 `comptime_fmt` 违反 `AGENTS.md` §3 同名规则（同时需与 `07` 对齐 `DebugStruct..DebugMap` 究竟置于 `builtin.fmt` 还是 `std.fmt`） | 请作者裁决：<br>1. 是否接受**兼容双轨**：保留 `Display` 的 `{format: .hex}` 属性快速通道（无堆/无状态机），同时将 `Formatter`/`FormatErr`/`DebugStruct..DebugSet` 与 `LowerHex`/`UpperHex`/`Binary`/`Octal`/`LowerExp`/`UpperExp`/`Pointer` 置于 `builtin.fmt` 并由 `std.fmt` 重导出；<br>2. 确认立即删除 `Display.comptime_fmt` 并迁移至 `meta.ComptimeDisplay`（方法名严格统一为 `fmt`），以及将 `Debug.write_debug`（与对应 `ComptimeDebug`）统一更名为 `fmt` |
| **Q7** | Pin-free 模型下的显式自引用/不可移动契约边界 | `docs/resource-management/moves-and-initialization.md` 确定按值移动不设 `Pin<T>`，但 `docs/type/self-reference.md` 实为反身代词，暂无针对内核侵入式节点或协程自引用帧的类型级禁止移动契约 | 请作者裁决：异步/协程内部栈帧或未来侵入式链表节点是全靠“编译器内部控制 + 相对偏移 + `unsafe`”实现，还是允许显式不可移动语义（例如否定 `not Movable` 约束或 `^attrs.mem.immovable`） |
| **Q8** | `Tuple<...Es>` 位置索引与 `where` 高阶 `meta.List` 谓词展开 | `docs/grammar/syntax:153` 目前仅允许 `expr . id`（无 `t.0` 数字成员投影）；`docs/traits/implementation-resolution.md:217-223` 仍只展示单条件示例，未定 `Es.all(\|E\| E :- Trait)` 的正式谓词语法 | 请作者裁决：<br>1. 语法层面是否补充 `projection -> expr . (id | decimal_lit)` 支持 `t.0` / `t.1`，还是统一以方法 `t.get(0usize)` 与模式 `(a, b) = t` / `'tuple_indexed` 访问；<br>2. `where requires Es.all(...)` 作为通用可变参 trait bound 展开机制的正式语义与合法性边界（同步联动 `WL-03` 与 `WL-05`） |
