# 09. 标准库（`std`）、官方协议包边界与生态就绪度差距审计

> **审计对象**：Flurry Tier 1 `std`（22 个 `.fl` 源码文件 / 7,925 LOC + 15 个 `.md` 文档）、六个内置包整体配套平衡、Tier 2 官方薄协议包（[tmp/wishlist/10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md) 17 组）与 Tier 3 生态工具链/编译器就绪度
> **检查基线**：仓库 `dev` 分支（`HEAD` 当前源码状态，只读核查，未执行 `cargo build` / `cargo test`）
> **总结论**：**语言设计合规性高、系统覆盖度处于早期基线，Tier 2/3 生态尚处于零实现前置阶段**。`library/std` 的 22 个 `.fl` 文件已严格遵循 `AGENTS.md` §5 红线（零 `Option`/`Result` 枚举、零 `::` 路径、零使用处 `.symbol'builtin`、`PascalCaseErr` 错误命名、全部堆容器采用 `DefaultAllocator` + `quote .allocator` 零存储开销运行时依赖范式）；但对照完整 `docs/` 与 Wishlist `07`/`08`/`10`，核心仅完成基础内存管理、基础集合、`String`/UTF-8、属性化 `std.fmt`、同步字节 `std.io`、编译期路径版 `File`、`Duration` 与 `ExitCode`/`id`，而 `std.path`/`std.env`/`std.ffi`/`std.simd`/`std.runtime`/`std.sync`/`std.net` 及全部 **17 组 Tier 2 官方薄协议包**均处于空白或空占位；同时 `Vec.remove` 早退留值（double-drop 隐患）、`BufReader`/`BufWriter` 溢出静默吞错、动态文件路径缺失与 `issues/0001`/`0003`/`0004`、`Eq`/`Ord`/`Index` 按值消耗等编译器缺口构成了生态推进的硬阻断。

---

## 1. 审计范围、检查基线与设计标尺

### 1.1 实测代码与文档范围

本轮基于仓库实时代码与配置文件进行逐行核查，不直接转抄 `ai_dev_logs/`，且严格不引用 2026-05 之前的 `proposal_trace/`、`OLD-2024/` 或 `flurry_tutorial/`：

1. **`library/std/` 全量源码（22 个 `.fl` 文件 / 7,925 LOC）与 15 个 `.md` 模块说明**（共 37 个文件）：
   - 包根与显式 prelude：[library/std/lib.fl](file:///home/fin/luna/library/std/lib.fl)（102 LOC）、[library/std/README.md](file:///home/fin/luna/library/std/README.md)（50 行）。
   - 内存与分配器（`mem_impl/`）：[allocator.fl](file:///home/fin/luna/library/std/mem_impl/allocator.fl)（295 LOC）、[layout.fl](file:///home/fin/luna/library/std/mem_impl/layout.fl)（166 LOC）、[boxed.fl](file:///home/fin/luna/library/std/mem_impl/boxed.fl)（155 LOC）、[arena.fl](file:///home/fin/luna/library/std/mem_impl/arena.fl)（604 LOC）。
   - 集合（`collections_impl/`）：[slice.fl](file:///home/fin/luna/library/std/collections_impl/slice.fl)（216 LOC）、[vec.fl](file:///home/fin/luna/library/std/collections_impl/vec.fl)（896 LOC）、[vec_deque.fl](file:///home/fin/luna/library/std/collections_impl/vec_deque.fl)（737 LOC）、[hash_map.fl](file:///home/fin/luna/library/std/collections_impl/hash_map.fl)（1,228 LOC）、[hash_set.fl](file:///home/fin/luna/library/std/collections_impl/hash_set.fl)（200 LOC）。
   - 文本与格式化（`string_impl/`, `fmt_impl/`）：[string.fl](file:///home/fin/luna/library/std/string_impl/string.fl)（927 LOC）、[parse.fl](file:///home/fin/luna/library/std/fmt_impl/parse.fl)（307 LOC）、[arguments.fl](file:///home/fin/luna/library/std/fmt_impl/arguments.fl)（193 LOC）。
   - I/O、文件、时间、进程与平台层：[errors.fl](file:///home/fin/luna/library/std/io_impl/errors.fl)（68 LOC）、[reader/mod.fl](file:///home/fin/luna/library/std/io_impl/reader/mod.fl)（254 LOC）、[writer/mod.fl](file:///home/fin/luna/library/std/io_impl/writer/mod.fl)（248 LOC）、[buffered.fl](file:///home/fin/luna/library/std/io_impl/buffered.fl)（547 LOC）、[file.fl](file:///home/fin/luna/library/std/fs_impl/file.fl)（211 LOC）、[duration.fl](file:///home/fin/luna/library/std/time_impl/duration.fl)（341 LOC）、[process.fl](file:///home/fin/luna/library/std/process_impl/process.fl)（71 LOC）、[linux.fl](file:///home/fin/luna/library/std/platform/linux.fl)（78 LOC）、[unix.fl](file:///home/fin/luna/library/std/platform/unix.fl)（81 LOC）。
   - 占位目录（仅 `.md`）：[runtime/README.md](file:///home/fin/luna/library/std/runtime/README.md)、[sync/README.md](file:///home/fin/luna/library/std/sync/README.md)。
2. **六个内置包全体体量基线与基础设施**：
   - `builtin`（28 `.fl` / 8,970 LOC）、`meta`（36 `.fl` / 6,757 LOC）、`attrs`（10 `.fl` / 99 LOC）、`build`（5 `.fl` / 121 LOC）、`std`（22 `.fl` / 7,925 LOC）、`verification`（1 `.fl` / 2 LOC）——总计 **102 个 `.fl` 文件 / 23,874 LOC**。
   - 绑定与结构文档：[library/language-items.conf](file:///home/fin/luna/library/language-items.conf)（136 行）、[library/STRUCTURE.md](file:///home/fin/luna/library/STRUCTURE.md)、[library/README.md](file:///home/fin/luna/library/README.md)。
3. **编译器与生态支持基线**：
   - 单二进制驱动与包接口：[compiler/luna/Cargo.toml](file:///home/fin/luna/compiler/luna/Cargo.toml)、[compiler/luna/src/cli.rs](file:///home/fin/luna/compiler/luna/src/cli.rs)、[compiler/luna/src/package_cli.rs](file:///home/fin/luna/compiler/luna/src/package_cli.rs)、[compiler/luna/src/packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs)、[compiler/anar/Cargo.toml](file:///home/fin/luna/compiler/anar/Cargo.toml)（LSP/IDE 内部库，非独立二进制）。
   - 宿主运行期与属性/内建表：[compiler/luna/src/host/llvm/platform.ll](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.ll)（233 行）、[control.rs](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs)、[compiler/elaboration/src/program/attributes.rs](file:///home/fin/luna/compiler/elaboration/src/program/attributes.rs)、[compiler/nir/src/intrinsics.rs](file:///home/fin/luna/compiler/nir/src/intrinsics.rs)。
   - 近期示例与测试套件：[development/examples/](file:///home/fin/luna/development/examples/)（含 [host-println.fl](file:///home/fin/luna/development/examples/host-println.fl)、[host-stdout.fl](file:///home/fin/luna/development/examples/host-stdout.fl)、[library-smoke.fl](file:///home/fin/luna/development/examples/library-smoke.fl)）、[development/library_check.py](file:///home/fin/luna/development/library_check.py)（38 个阶段）、[compiler/luna/tests/std_allocators_collections_io.rs](file:///home/fin/luna/compiler/luna/tests/std_allocators_collections_io.rs)、[library_std.rs](file:///home/fin/luna/compiler/luna/tests/library_std.rs)、[library_std_format.rs](file:///home/fin/luna/compiler/luna/tests/library_std_format.rs)、[build_and_std_stage2c.rs](file:///home/fin/luna/compiler/luna/tests/build_and_std_stage2c.rs)。
4. **已知未决阻断**：[issues/0001-runtime-dependent-origin-and-default-param.md](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md)、[issues/0003-builtin-packages-compiler-blockers.md](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md)、[issues/0004-macro-system-carbon.md](file:///home/fin/luna/issues/0004-macro-system-carbon.md)、[development/repros/native-text-method-provenance.fl](file:///home/fin/luna/development/repros/native-text-method-provenance.fl)。

> [!NOTE]
> **路径与命名核实说明**：任务提示中提及的 `vilya/` 目录在当前仓库并不存在（`proposal_trace` 为指向外部历史目录的链接，依 `AGENTS.md` §1 属于排除口径），仓库内近期 `.fl` 示例位于 [development/examples/](file:///home/fin/luna/development/examples/) 与 [development/repros/](file:///home/fin/luna/development/repros/)；硬盘上实际存在的 Wishlist 对应文件名为 [07-std-mem-collections-io-os.md](file:///home/fin/luna/tmp/wishlist/07-std-mem-collections-io-os.md)、[08-effects-async-concurrency-net-runtime.md](file:///home/fin/luna/tmp/wishlist/08-effects-async-concurrency-net-runtime.md)、[09-verification-synthesis-and-tensors.md](file:///home/fin/luna/tmp/wishlist/09-verification-synthesis-and-tensors.md)、[10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md) 与 [00-index-and-roadmap.md](file:///home/fin/luna/tmp/wishlist/00-index-and-roadmap.md)。

### 1.2 设计标尺与双轨分层

- **Core Norm Target（核心规范标尺）**：`AGENTS.md` §1/§3/§5 与现行正式 `docs/`（[builtin-packages.md](file:///home/fin/luna/docs/code-organization/builtin-packages.md)、[configuration-and-scripts.md](file:///home/fin/luna/docs/code-organization/configuration-and-scripts.md)、[safety.md](file:///home/fin/luna/docs/program-verification/safety.md)、`docs/algebraic-effect-and-delimited-continuation/*`、`docs/resource-management/*`、`docs/traits/*`）。规定：仅 6 个内置包、`std.prelude` 显式导入、`Once` 全称实现、无 `Option`/`Result`、点分路径、`PascalCaseErr` 错误规范、运行时依赖类型 `allocator` 范式、`luna` 单二进制 CLI。
- **Wishlist Extension Target（扩展路线标尺）**：`tmp/wishlist/07`（20 项 `WL-07-01..20`）、`08`（18 项 `WL-08-01..18`）、`09`（16 项 `WL-09-01..16`）、`10`（5 项编译器前置 `WL-10-01..05` + **17 组 Tier 2 官方薄协议包 `WL-10-06..22`**）及 [tmp/silver-design/04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md)（`P0-1..P2-3`）。严格守卫三层防膨胀黑名单：**`regex`、序列化/JSON、HTTP/TLS、SQL、密码学/摘要、RNG、历法/tzdata、压缩、多媒体/色彩、ML/张量驱动、具体遥测与具体异步调度器严禁进入六个内置包（Tier 1）**。

---

## 2. 成熟度总览与统计表

### 2.1 分层成熟度总览

| 层级 / 子域 | 当前仓库现状 | 目标成熟度 | 主要阻断 / 核心评价 |
|---|---|---|---|
| **Tier 1 `std.mem`** | `Allocator`/`SystemAllocator`/`FixedBufferAllocator`/`ArenaAllocator`/`Box`/`Layout`/`RawBlock` 已运行，缺 `Rc`/`Arc`/`Cow`/`MaybeUninit`/`ManuallyDrop`/扩展分配器 | `🟡 Partial` | 缺少 `union` 免 `Drop` 验证、`std.sync.atomic` 与 `Deref`/`DerefMut` |
| **Tier 1 `std.collections`** | `Vec`/`VecDeque`/`HashMap`/`HashSet`/`Slice` 已工作，缺 `MutSlice`/`BTree*`/`BinaryHeap`/`BitSet`/`Entry`/无 `Clone` 查询/`Index`/`IndexMut` | `🟡 Partial` | `Index`/`Eq`/`Ord` 按值消耗 `self`、`issues/0001` 闭包 `env`、缺少 `&r` 切片所有权证明 |
| **Tier 1 `std.string` & `std.fmt`** | `String`（RFC 3629 + `Display`）与 `{format: .debug, ...}` + IEEE 浮点已可用；缺 `String.as_str`/`FromUtf8Err`/位置命名参数/`ComptimeDebug` | `🟡 Partial` / `⚠️ Divergent` | `as_str` 阻塞于 `text_provenance`（`issues/0003`）；`comptime_format` 对 `.debug` 执行 `assumes false;` |
| **Tier 1 `std.io` & `std.fs` & `std.path`** | `Reader`/`Write`/`ByteWriter`/`BufReader`/`BufWriter`/`Cursor`/`Stdin`/`File` 可用；`File` 仅限 `comptime path: str`，无 `Path`/`Seek`/`Metadata`/`ReadDir` | `🟡 Partial` / `🔴 Missing` | 缺动态路径/目录/stat OS 原语；`BufReader`/`BufWriter` 存在溢出静默错误 |
| **Tier 1 `std.time` / `env` / `process` / `os` / `ffi` / `simd`** | `Duration` + `id()`/`ExitCode` + 11 个 Linux `platform.ll` intrinsic；无 `Instant`/`SystemTime`/`env`/`Command`/`panic`/`ffi`/`simd` | `🟡 Partial` / `🔴 Missing` | `^attrs.abi` 仅支持闭合 `.c` 标量导出；无 `extern` 替代导入；`panic` 合同未裁决 |
| **Tier 1 `std.runtime` / `std.sync` / `std.net` (`08`)** | 仅 `std/runtime/README.md` 与 `std/sync/README.md` 3 行占位，无 `std.net` 目录 | `🔴 Missing` | 缺 `builtin.Continuation`、`HandleError`/`HandleEffect`、`StackPool`、原子/锁/socket |
| **Tier 2 官方薄协议包 (`10`)** | 仓库 **0/17 组**（`0` 个 `.fl` 文件），编译器 5 项前置（`WL-10-01..05`）部分未通 | `🔴 Missing` | `issues/0001` 阻塞 `quote .allocator` 跨 trait 交付；`metadata` 浅覆盖待解 |
| **Tier 3 生态就绪度与六包平衡** | `luna` 单二进制 CLI 有显式 `--packages-v1`/`lsp`/`fmt`，无 `package.fl` DAG/lockfile/registry；`std` 严格合规 | `🟡 Partial` / `🔴 Missing` | 需打通 `issues/0001`/`0003`/`0004`、`build` DAG、FFI 导入与 `library-smoke.fl` sysroot |

### 2.2 全量特性差距（`GAP-09-01` ~ `GAP-09-39`）分布统计

> 下表与第 3 节全部 **39** 项 `GAP-09-NN` 严格一一对应并数值相等。

| 状态等级 | 数量 | 占比 | 对应条目编号 |
|---|---|---|---|
| `🟢 Full`（完全对齐并完整验证） | **1** | 2.6% | `GAP-09-32` |
| `🟡 Partial`（核心基线已实现但存在缺口） | **16** | 41.0% | `GAP-09-01..03, 05..07, 10, 12, 14, 15, 17, 18, 20, 33, 34, 37` |
| `🟠 Frontend-Only`（前端或语法层已支持但生态交付未闭环） | **1** | 2.6% | `GAP-09-28` |
| `🔴 Missing`（零实现或仅空占位） | **17** | 43.6% | `GAP-09-04, 08, 09, 11, 16, 19, 21, 23..27, 29..31, 35, 38` |
| `⚠️ Divergent`（违背 `docs/` / `AGENTS.md` 或存在阶段性语义偏差） | **3** | 7.7% | `GAP-09-13, 22, 36` |
| `⚪ Deferred`（按协议明确的本阶段暂缓项） | **1** | 2.6% | `GAP-09-39` |
| **合计** | **39** | **100%** | `GAP-09-01` .. `GAP-09-39` |

### 2.3 健全性 · 完备性 · 鲁棒性（`SND` / `CMP` / `ROB`）问题严重度分布

> 下表与第 6 节全部 **18** 项 `SND-09-01..07`、`CMP-09-01..06`、`ROB-09-01..05` 严格对齐。

| 类别 | `Critical` (`P0/blocker`) | `High` (`P1`) | `Medium` (`P2`) | `Low` (`P3`) | 小计 |
|---|---|---|---|---|---|
| **健全性（`SND-09-01..07`）** | 2 | 5 | 0 | 0 | **7** |
| **完备性（`CMP-09-01..06`）** | 5 | 1 | 0 | 0 | **6** |
| **鲁棒性（`ROB-09-01..05`）** | 1 | 3 | 1 | 0 | **5** |
| **合计** | **8** | **9** | **1** | **0** | **18** |

---

## 3. 全量特性差距矩阵（`GAP-09-NN`）

> 规范标尺列明确区分 **Core (`docs/` / `AGENTS.md`)** 与 **Wishlist (`07` / `08` / `09` / `10` / `Silver`)**。

| 编号 | 能力 / 模块项 | 规范来源区分 | 成熟度 | 现状与源码证据 | 核心缺口与阻断原因 |
|---|---|---|---|---|---|
| `GAP-09-01` | `std.mem` 基础设施：`AllocErr` / `ReallocErr` / `RawBlock` / `Layout` / `Allocator` / `SystemAllocator` | Core (`AGENTS.md` §5) + Wishlist `WL-07-01` | `🟡 Partial` | [allocator.fl:12-265](file:///home/fin/luna/library/std/mem_impl/allocator.fl#L12-L265), [layout.fl:24-166](file:///home/fin/luna/library/std/mem_impl/layout.fl#L24-L166) 已实现基础布局与 `Allocator` | `AllocErr`/`ReallocErr`/`Layout` 缺 `Display`；`Layout` 缺 `Eq`/`Hash`/`dangling`/`align_to`/`extend` 及 `of<T>`/`array<T>` 的 comptime 求值；`Allocator` 缺默认 `grow_zeroed`/`shrink`/`OwningAllocator`；`SystemAllocator.reallocate` 仅走 alloc+copy+free 未对接 `realloc` |
| `GAP-09-02` | 高级分配器生态：`FixedBufferAllocator`、`ArenaAllocator` 及 `Bump`/`Slab`/`Pool`/`Scratch`/`Tracking`/`Failover` | Core + Wishlist `WL-07-02` | `🟡 Partial` | [arena.fl:54-604](file:///home/fin/luna/library/std/mem_impl/arena.fl#L54-L604) 已实现 `FixedBufferAllocator` 与侵入式链表 `ArenaAllocator`（带 `.Backing` 与 `Drop`） | `FixedBufferAllocator` 缺 `Drop` 泄漏检测与 `Checkpoint`/`rewind`；`ArenaAllocator` 缺 `reset_retaining_capacity`/`scope`；`Bump`/`Slab`/`Pool`/`Scratch`/`Tracking`/`Failover` 全部未实现 |
| `GAP-09-03` | 智能指针家族：`Box` 完整化与 `Rc`/`Arc`/`Cow`/`Unique` | Core + Wishlist `WL-07-03` | `🟡 Partial` | [boxed.fl:12-155](file:///home/fin/luna/library/std/mem_impl/boxed.fl#L12-L155) 已实现 zero-overhead `Box<T, .A, .allocator>` 与 `Drop` | `Box` 缺 `from_raw`/`into_raw`/`leak`/`Eq`/`Ord`/`Hash`/`Display`/`Deref`；`Unique`/`Rc`/`RcWeak`/`Arc`/`ArcWeak`/`Cow`/`ToOwned` 全部缺失；`Arc` 阻塞于 `std.sync.atomic` 与 `Send`/`Sync` |
| `GAP-09-04` | 内存工具与未初始化原语：`MaybeUninit`、`ManuallyDrop`、`swap`/`replace`/`take`/`forget`/`drop`/`zeroed`/`needs_drop` | Core + Wishlist `WL-07-04` (`04` `WL-04-26/27`) | `🔴 Missing` | 仅 `builtin.mem` 提供 `raw_read`/`raw_write`/`copy*`/`set_bytes`/`compare_bytes`/`checked_*` 与 `align_up_usize` | 全量 `library/` 无 `MaybeUninit`/`ManuallyDrop` 定义；硬阻塞于 `union` 免 `Drop` 验证及 `needs_drop` 反射（`BufWriter` 等因无法绕过自定义 `Drop` 而缺少 `into_inner`） |
| `GAP-09-05` | 序列容器：`Vec`、`VecDeque`、`SmallVec` | Core + Wishlist `WL-07-05` | `🟡 Partial` | [vec.fl:1-896](file:///home/fin/luna/library/std/collections_impl/vec.fl#L1-L896) 与 [vec_deque.fl:1-737](file:///home/fin/luna/library/std/collections_impl/vec_deque.fl#L1-L737) 已完成基础增删改、ZST 无堆优化、消费迭代与自定义 allocator | `Vec` 缺 `retain`/`dedup*`/`drain`/`splice`/`split_off`/`resize*`/`extend*`/`iter_ref`/`Index`/`IndexMut`/`FromIterator`；`VecDeque` 缺 `make_contiguous`/`as_slices`/`rotate_*`；`SmallVec` 缺失（依赖 `Array<T, N>` 与 `MaybeUninit`） |
| `GAP-09-06` | 散列集合：`HashMap`、`HashSet` | Core + Wishlist `WL-07-06` | `🟡 Partial` | [hash_map.fl:1-1228](file:///home/fin/luna/library/std/collections_impl/hash_map.fl#L1-L1228)（SoA 单 slab + tombstone + rehash）与 [hash_set.fl:1-200](file:///home/fin/luna/library/std/collections_impl/hash_set.fl#L1-L200) 已运行 | 缺 `.H: BuildHasher` 参数（固定使用 unseeded `mix_hash`）；`find_slot`/`contains_key`/`remove`/`get_cloned` 强制 `K :- Clone`；缺 `Entry`（`OccupiedEntry`/`VacantEntry`）、`get_ref`/`keys`/`values`/`iter_ref`/`retain`/`drain` 与 `HashSet` 集合代数 |
| `GAP-09-07` | 切片视图与算法：`Slice<T>`、`MutSlice<T>`、排序/二分/窗口 | Core + Wishlist `WL-07-09` | `🟡 Partial` | [slice.fl:1-216](file:///home/fin/luna/library/std/collections_impl/slice.fl#L1-L216) 实现只读 `Slice<T>`（`T :- Copy`）、`ListPatternView`/`StableListPatternView` | 无 `MutSlice<T>`（缺少与宿主容器的可达性 `&r`/独占写证明）；无 `binary_search*`/`sort*`/`chunks*`/`windows`/`split*`/`rotate_*`；`Slice<T>` 未受 `&r` 区域约束防止悬垂 |
| `GAP-09-08` | 有序树、堆、位集与环形缓冲：`BTreeMap`/`BTreeSet`/`BinaryHeap`/`BitSet`/`RingBuffer` | Wishlist `WL-07-07` | `🔴 Missing` | 仓库零实现 | 需 `RangeBounds`（`01`）与 `MaybeUninit`（`RingBuffer`），且 `SmallVec`/`RingBuffer` 归属待 `WL-07` Q11 裁定 |
| `GAP-09-09` | 运行时依赖句柄集合：`Arena<T>` + `SlotMap` + `SoA<T>` + `NodeIndex` | Core (`runtime-dependent-type.md`) + Wishlist `WL-07-08` / Silver `P1-4` | `🔴 Missing` | 仓库零实现 | 阻塞于 `StructBuilder` 生成带 `quote` 子句与 `Drop` 的类型、`meta.reflection` 与 `issues/0001` 闭包 `env` 交付；归属边界见 `10` Q3 / `07` Q11 |
| `GAP-09-10` | `std.string.String` UTF-8 文本与修改查询 | Core (`AGENTS.md` §5) + Wishlist `WL-07-10` | `🟡 Partial` | [string.fl:1-927](file:///home/fin/luna/library/std/string_impl/string.fl#L1-L927) 已完成 RFC 3629 校验、`from_utf8(_lossy)`、增量构建、`Display`/`Debug`/`Eq`/`Ord`/`Hash` | `Utf8Err` 缺 `Display`；缺 `FromUtf8Err<.A, .allocator>`（目前 `from_utf8` 失败直接丢弃传入 `Vec<u8>`）；缺 `insert*`/`remove`/`retain`/`drain`/`replace_range`/`chars`/`lines`/`trim*`/`split*` |
| `GAP-09-11` | `String.as_str` 与零拷贝借用 `str` 子视图 | Core (`issues/0003`) + Wishlist `WL-07-10` / Q4 | `🔴 Missing` | [native-text-method-provenance.fl](file:///home/fin/luna/development/repros/native-text-method-provenance.fl) 已记录前端阻断 | 编译器 `pointer_input_layout` 拒绝 `str` backing-region 输入；阻塞 `String.as_str`、`CStr.to_str` 及零拷贝行/切片视图 |
| `GAP-09-12` | `std.fmt` 编译期检查格式化与输出入口 | Core + Wishlist `WL-07-11` | `🟡 Partial` | [parse.fl:1-307](file:///home/fin/luna/library/std/fmt_impl/parse.fl#L1-L307) 与 [arguments.fl:1-193](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L1-L193) 支持 `{key: value}` 属性规格、`{format: .debug}`、`println`/`eprintln`/`format`/`format_into`/`write_fmt` | `print`/`eprint`（`:16, :160`）仅收 `comptime text: str` 未变参化；不支持 `{0}`/`{name}`；缺 `DebugStruct*` 构建器；所有 `std` 错误与结构均缺 `Display` |
| `GAP-09-13` | `comptime_format` 与 `Debug`/`Display` 双阶段对偶 | Core (`the-comptime-phases-of-traits.md`) + Wishlist `WL-07-11` / `WL-04-21` | `⚠️ Divergent` | [arguments.fl:98-122](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L98-L122) 依赖 `Display.comptime_fmt` 并在 `.debug` 分支（`:113`）直接写 `assumes false;` | 遇到 `{format: .debug}` 在 `comptime_format` 中触发 `assumes false;` 断言失败；`Display.comptime_fmt` 与 `WL-04-21` 规划的纯 `meta.fmt.ComptimeDisplay`/`ComptimeDebug` 分层尚未收敛 |
| `GAP-09-14` | `std.io` 协议与缓冲 I/O：`Reader`/`Write`/`Cursor`/`BufReader`/`BufWriter` | Core + Wishlist `WL-07-12` | `🟡 Partial` | [reader/mod.fl](file:///home/fin/luna/library/std/io_impl/reader/mod.fl), [writer/mod.fl](file:///home/fin/luna/library/std/io_impl/writer/mod.fl), [buffered.fl](file:///home/fin/luna/library/std/io_impl/buffered.fl) 已完成同步读写与 EINTR 重试 | 缺 `IoErrKind` 与 `Display for IoErr`、`BufRead`、`Seek`/`SeekFrom`、`LineWriter`、`BufWriter.into_inner`、`copy`/`Take`/`Chain`/`Empty`/`Sink`、`StdinLock`/`StdoutLock` |
| `GAP-09-15` | `std.fs` 文件系统句柄、打开选项与目录/元数据 | Core + Wishlist `WL-07-14` | `🟡 Partial` | [file.fl:1-211](file:///home/fin/luna/library/std/fs_impl/file.fl#L1-L211) 已支持 `File`（`Drop`/显式 `close`、`Reader`/`Write`/`ByteWriter`）与一次性读写 | **所有打开/创建/删除/读写辅助均强制 `comptime path: str`**，无法使用运行时计算路径；缺 `append`/`OpenOptions`/`Seek`/`Metadata`/`ReadDir`/`create_dir`/`rename`/`sync_all` |
| `GAP-09-16` | `std.path` 跨平台路径：`Path`、`PathBuf`、`Component` | Wishlist `WL-07-13` | `🔴 Missing` | 仓库零实现 | 依赖 `std.ffi.OsStr` / `OsString`（`WL-07-16`）与动态路径 I/O |
| `GAP-09-17` | `std.time` 时长与时钟：`Duration`、`Instant`、`SystemTime`、`sleep` | Core + Wishlist `WL-07-15` | `🟡 Partial` | [duration.fl:1-341](file:///home/fin/luna/library/std/time_impl/duration.fl#L1-L341) 完整实现纳秒归一化 `Duration` 及 `checked_*`/`saturating_*` | `Duration` 缺 `Display` 与运算符 impl；`Instant`、`SystemTime`、`TimeErr`、`sleep` 全部缺失（缺单调/墙上时钟宿主 intrinsic） |
| `GAP-09-18` | `std.env` 与 `std.process`（`Command`/`Child`/`exit`/`abort`/`panic`） | Core (`library/README.md:393-395`) + Wishlist `WL-07-16/17` | `🟡 Partial` | [process.fl:1-71](file:///home/fin/luna/library/std/process_impl/process.fl#L1-L71) 仅实现 `id() -> u32` 与 `ExitCode` | `std.env` 完全缺失（无 `args`/`var`/`current_dir`/`consts`）；`Command`/`Child`/`Stdio`/`exit`/`abort` 缺失；**`panic` 为 `library/README.md:395` 记录的硬阻断** |
| `GAP-09-19` | `std.ffi` C ABI、原始 C 类型、`CStr`/`CString`/`OsStr`/`OsString`、无 `extern` 导入 | Core (`AGENTS.md` §5, `docs/others/ffi.md`) + Wishlist `WL-07-18` | `🔴 Missing` | [attributes.rs:108-125](file:///home/fin/luna/compiler/elaboration/src/program/attributes.rs#L108-L125) 与 [control.rs:198-224](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L198-L224) 仅支持闭合标量 `^attrs.abi(.c)` 导出 | 无 C 类型表、`CStr`/`OsStr`；缺 `__flurry_ffi_link`/`link_name`（无 `extern` 外部函数 `declare`）；缺 `.system`/`.win64`/`.sysv64`/`.aapcs` 与 C struct 聚合 ABI |
| `GAP-09-20` | `std.os` / `std.platform` 平台系统调用适配层 | Core + Wishlist `WL-07-19` / Q8 | `🟡 Partial` | [linux.fl:1-78](file:///home/fin/luna/library/std/platform/linux.fl#L1-L78), [unix.fl:1-81](file:///home/fin/luna/library/std/platform/unix.fl#L1-L81), [platform.ll:1-233](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.ll#L1-L233) 仅封装 11 个 Linux x86_64 intrinsic | 无 `clock_gettime`/`stat`/`getdents`/`fork`/`execve`/`socket`/`getrandom` 等系统调用；无 macOS/Windows/WASI 实现与 `build.target` 条件切换 |
| `GAP-09-21` | `std.simd` 可移植 SIMD 向量与掩码原语 | Wishlist `WL-07-20` + Silver `P1-5` | `🔴 Missing` | 仓库零实现 | 缺 `__flurry_align`（`^attrs.layout.align`）、`nir`/`llvm` 向量类型 `<N x T>` 与目标 intrinsic 降码 |
| `GAP-09-22` | `std` 模块文档与 `docs/` 同步一致性 | Core (`AGENTS.md`) | `⚠️ Divergent` | [collections.md:9](file:///home/fin/luna/library/std/collections.md#L9) 仍称 `map`/`set`/切片未实现；[std/README.md:42](file:///home/fin/luna/library/std/README.md#L42) 仍称 `process`/`time` 未实现；[docs/others/ffi.md](file:///home/fin/luna/docs/others/ffi.md) 与 [software-transactional-memory.md](file:///home/fin/luna/docs/concurrency/software-transactional-memory.md) 为 0 字节 | 文档状态滞后于源码（易误导审计与用户）；`ffi.md` 与 `stm.md` 规范文件空白 |
| `GAP-09-23` | `builtin.Continuation`、`std.runtime.TypedContinuation` 与 `StackPool`/`BoundedStackPool`/`GuardedStackPool` | Core (`effect-abi.md`) + Wishlist `WL-08-05..07` + Silver `P0-3`/`P1-1` | `🔴 Missing` | [std/runtime/README.md](file:///home/fin/luna/library/std/runtime/README.md) 仅 3 行空占位；无 `builtin.Continuation` 定义 | 缺无公开类型参数的原生 `Continuation`、LLVM stack 切换、C 帧屏障 trap 与有界/保护栈池分配器 |
| `GAP-09-24` | `builtin.effects` 处理能力协议与结构化 `async effect`（`TaskScope` / `CancelToken`） | Core (`handling-capability.md`, `async-algebraic-effect.md`) + Wishlist `WL-08-01..04, 08..09` | `🔴 Missing` | 编译器已有同步直接效应与 `DirectPassing` 属性；[std/runtime/](file:///home/fin/luna/library/std/runtime/README.md) 与 `builtin.effects` 均零代码 | 缺 `HandleError`/`HandleEffect`/`DirectEffectHandler`/`ResumptiveEffectHandler` 注册与 `Future`/`TaskScope`/`JoinHandle`/`CancelToken`/`select`/`race` |
| `GAP-09-25` | `std.sync.atomic`、`Send`/`Sync` 标记协议、锁与线程/通道 | Wishlist `WL-08-11..14` | `🔴 Missing` | [std/sync/README.md](file:///home/fin/luna/library/std/sync/README.md) 仅 3 行空占位 | 缺 `Send`/`Sync`、`Atomic*`（LLVM 原子指令）、`Mutex`/`RwLock`/`Condvar`/`OnceLock`、`mpsc`/`mpmc`/`oneshot`/`broadcast` 与 `Thread`/`ThreadScope` |
| `GAP-09-26` | `std.sync.stm`（BTO 软件事务内存、`OrderedLock`、`atomic(...)`） | Core (`docs/concurrency/prompt.fl`) + Wishlist `WL-08-15` | `🔴 Missing` | 零源码与零编译器降码；[software-transactional-memory.md](file:///home/fin/luna/docs/concurrency/software-transactional-memory.md) 为 0 字节 | 缺 `TVar`/`TArray`/`TQueue`/`OrderedLock`/`retry`/`or_else` 及 `atomic` 事务日志与回滚约束降码 |
| `GAP-09-27` | `std.net` L3/L4 原始 OS 套接字（`Tcp*`/`UdpSocket`/`Unix*`/`IpAddr`） | Wishlist `WL-08-16..18` | `🔴 Missing` | 仓库无 `library/std/net*` 目录 | 缺地址类型、`ToSocketAddrs`（allocator 子句）、同步 socket 与异步非阻塞桥接（严格排除 HTTP/TLS） |
| `GAP-09-28` | Tier 2 生态编译器 5 项前置（`Symbol` 参数化 Trait/派生、`newtype .as(View)`、非 `__flurry_*` 元数据合并、跨 trait `quote .allocator`、`Comptime*` 双相） | Core (`docs/traits/`, `docs/comptime/`) + Wishlist `WL-10-01..05` | `🟠 Frontend-Only` | 前端部分支持泛型参数 `trait`、`newtype` 与 `^attrs.metadata`；`issues/0001` 记录闭包/trait 特化 env 交付阻塞 | 开放 `Symbol` 参数化派生策略（`derive ... .use(...)`）未通；多次 `^attrs.metadata` 浅覆盖待解决（`00` `OQ-35`）；[issues/0001:318-366](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L318-L366) 硬阻断 trait/闭包容器环境传递 |
| `GAP-09-29` | Tier 2 核心基础设施薄协议包：`serialization`、`random`、`async_runtime`、`testing`、`regex_protocol`/`parser_protocol` | Wishlist `WL-10-06, 09, 10, 19, 21` | `🔴 Missing` | 仓库外置包 0 实现（仅编译器内建 `test {}` 语法与 `ListPatternView`） | 无法独立引入标准化序列化、`Executor`/`Reactor`、属性测试或 `regex_protocol`/`TuplePatternView` 外部生态库 |
| `GAP-09-30` | Tier 2 服务、安全与数据薄协议包：`telemetry`、`crypto`/`digest`、`http`/`wire`、`database` | Wishlist `WL-10-07, 08, 11, 12` | `🔴 Missing` | 仓库零实现 | 阻塞于 `GAP-09-28`（`WL-10-01..05`）、`async_runtime`、`std.net` 与匿名记录字段反射 |
| `GAP-09-31` | Tier 2 编码/校验/配置/历法/媒体/张量/FFI/扩展集合薄协议包（其余 8 组） | Wishlist `WL-10-13..18, 20, 22` | `🔴 Missing` | 仓库零实现；[tensor/static-shaped-tensors.md](file:///home/fin/luna/docs/comptime/static-shaped-tensors.md) 仅文档 | `ffi_bridge`/`plugin_abi` 阻塞于 `Type.type_id` 与 ABI 导出；`tensor_protocol` 阻塞于 `pipeline-projection` IIR 访问器；`collections_ext` 待 `07` Q11 划界 |
| `GAP-09-32` | `std` 语法红线与 Runtime-Dep `allocator` 范式合规性（零 `Option`/`Result`、零 `::`、零使用处 `.builtin`） | Core (`AGENTS.md` §5) | `🟢 Full` | Python 全扫描验证：22 个 `.fl` 中 `::` = 0、`Option<`/`Result<` = 0、`.builtin` = 0；全部 14 个运行时依赖分配类型（`Box`/`ArenaAllocator`/`Vec`/`VecIter`/`VecDeque`/`VecDequeIter`/`HashMap`/`HashMapIter`/`HashSet`/`HashSetIter`/`String`/`Cursor`/`BufReader`/`BufWriter`）均采用 `DefaultAllocator` / `.Backing` + `quote` 范式 | 已达标；后续新增模块与 Tier 2 包须维持此静态扫描门禁 |
| `GAP-09-33` | 五大活性内置包配套成熟度平衡（`builtin`/`meta`/`attrs`/`build`/`std`） | Core (`AGENTS.md` §1, §3) | `🟡 Partial` | `builtin`(8,970L) 与 `meta`(6,757L) 较充实；`attrs`(99L)、`build`(121L) 极薄；`builtin` 缺 `Array` 集中绑定/`HandleError`/`Continuation`/`panic`；`meta` 缺 `FnBuilder`/`ImplBuilder`/匿名记录字段反射；[meta/macros_impl/attributes.fl:1](file:///home/fin/luna/library/meta/macros_impl/attributes.fl#L1) 仅再导出 `macro3`/`macro4`（漏 `ast`/`peg`） | [attributes.rs:126-159](file:///home/fin/luna/compiler/elaboration/src/program/attributes.rs#L126-L159) 已通过 `domain` 放行 `__flurry_macro_*` 与 `__flurry_stable_since`，但仍拒绝 `__flurry_align`/`__flurry_purity_*`/`__flurry_ffi_*`/`__flurry_pipeline*` 等新键；`build` 缺 `package`/`targets`/`tasks`/`generated` |
| `GAP-09-34` | Full Sysroot 可用性、`language-items.conf` 与集成验证覆盖 | Core (`library/README.md:393-396`, `library_check.py`) | `🟡 Partial` | `std` 有 4 组独立原生测试文件运行真实 `--sysroot library`；但 `library-smoke.fl` 不含 `std`，`development/library_check.py` 38 个 stage 无 `std` 专项阶段 | `library/README.md:393-396` 明示 `panic` 等导致完整 sysroot 冒烟仍受限；`issues/0001:208` 记录含开发版 `std` 的空程序存在 `flow` 预算耗尽问题；`language-items.conf` 缺 `iterator-emit-effect`、`array-type` 等扩展键 |
| `GAP-09-35` | `luna` 单二进制包管理、工程清单 (`package.fl`)、依赖解析与构建 DAG（已取消独立 `anar`） | Core (`AGENTS.md` §1, `configuration-and-scripts.md`) + Wishlist `06` | `🔴 Missing` | [cli.rs:61-72](file:///home/fin/luna/compiler/luna/src/cli.rs#L61-L72) 与 [package_cli.rs](file:///home/fin/luna/compiler/luna/src/package_cli.rs) 仅提供低层显式包图命令行 `--packages-v1` / `--packages-stream-v1`；[anar/Cargo.toml](file:///home/fin/luna/compiler/anar/Cargo.toml) 为内部 LSP 库 | 无 `package.fl` 清单读取与图构建、无语义化版本锁文件（lockfile）、无本地/远程仓库缓存拉取、无多目标构建任务（`build.package`/`targets`/`tasks`）统一入口 |
| `GAP-09-36` | 阻断 `std` 与 Tier 2 的编译器契约偏差（`Eq`/`Ord`/`Index` 按值 `*self` 与 `attributes.rs` 白名单） | Core (`AGENTS.md`, `docs/traits/protocols.md`) + `00` §6.1 #7 | `⚠️ Divergent` | [comparison.fl:6-13](file:///home/fin/luna/library/builtin/numeric/comparison.fl#L6-L13) `Eq.eq`/`Ord.cmp` 与 [ops.fl:85](file:///home/fin/luna/library/builtin/protocols/ops.fl#L85) `Index.index` 均以按值 `self`（而非 `*self`）定义 | 导致 `Vec.eq`/`VecDeque.eq`/`String.eq`/`String.cmp`（[vec.fl:755](file:///home/fin/luna/library/std/collections_impl/vec.fl#L755), [string.fl:840](file:///home/fin/luna/library/std/string_impl/string.fl#L840)）调用后即消耗所有权，且全量 `std` 集合无法实现 `Index`；[attributes.rs:140-159](file:///home/fin/luna/compiler/elaboration/src/program/attributes.rs#L140-L159) 仍残留旧 `"packed"` 键同时封堵 `__flurry_align`/`__flurry_ffi_*`/`__flurry_purity_*` 等新官方键（与 §5 `DIV-09-03` 对应） |
| `GAP-09-37` | 生态开发体验（DX）：`Display` 普及度、诊断可读性与 `Lsp`/`Fmt`  tooling | Core (`AGENTS.md` §3, `stdlib` ergonomics) | `🟡 Partial` | `luna fmt`/`luna lsp`/`DiskPackageCache` 已存在；`library/std/` 共 13 个 `impl Debug`，仅 **1 个 `impl Display`**（`String`） | 所有 `std` 错误（`AllocErr`/`ReallocErr`/`Utf8Err`/`IoErr`/`FormatErr`）与 `Duration`/`ExitCode`/`Layout` 仅支持 `Debug` 而不支持 `{}` `Display`，阻碍真实应用错误呈现 |
| `GAP-09-38` | 跨平台（Linux/macOS/Windows/WASI）与外部 C/第三方生态接入度 | Core + Wishlist `WL-07-18/19` | `🔴 Missing` | [platform.ll:1-233](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.ll#L1-L233) 硬编码 x86_64 Linux 11 个 libc 包装 | 每新增一个系统调用都必须同时修改 `nir/intrinsics.rs`、`language-items.conf` 与 `platform.ll`（`WL-07` Q8 指出必须迁往无 `extern` 的 `^attrs.abi` C 符号导入方案） |
| `GAP-09-39` | `verification` 内置包高级程序验证 API（`SpecInt`/`Perm`/`PointsTo`/`lemma`） | Core (`AGENTS.md` §3, [verification/README.md](file:///home/fin/luna/library/verification/README.md)) + Wishlist `09` | `⚪ Deferred` | [library/verification/lib.fl:1-2](file:///home/fin/luna/library/verification/lib.fl#L1-L2) 仅保留包身份与空 `pub mod prelude {}` | 按作者先前决定本阶段暂缓高级库交付，但保留清晰包边界与依赖方向（不能阻碍 `builtin.mem` 基础安全检查） |

---

## 4. 关键缺口深度剖析（含源码证据）

### 4.1 编译器硬阻断反向卡死 `std` 核心设计（`issues/0001`、`0003`、`0004` 与 `Eq`/`Index` 签名）

当前限制 `library/std` 与 Tier 2 演进的并非单纯"代码没写完"，而是 4 组底层通用机制未闭环：

1. **闭包/泛型方法特化才显现的 `RuntimeEnvParameter` 交付失效（[issues/0001:318-366](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L318-L366)）**：
   - **实证**：`e14afe49` 已修通作用域内直接闭合的局部环境传递；但在 `Box.write_debug` 调用 `with_ref(|inner| ...)` 或 `Vec.write_debug` 调用 `try_for_each_ref(|elem| ...)` 时，若元素为携带自定义局部 `FixedBufferAllocator` 的 `String` 等类型，特化后才出现的操作环境参数（`RuntimeId`）不在外层泛型闭包的声明捕获字段列表 `%{ v: T }` 中，在 [compiler/materialization/src/lib.rs:2848](file:///home/fin/luna/compiler/materialization/src/lib.rs#L2848)（`collect_closure_allowed_bindings`）收集允许绑定集合后，于 [compiler/materialization/src/lib.rs:3061-3101](file:///home/fin/luna/compiler/materialization/src/lib.rs#L3061-L3101) 被 `closure_allowed` 检查截停并报错 `materialized call cannot deliver runtime environment parameter`。
   - **当前后果**：`Box`/`Vec` `impl Debug` 被迫展开重复的 `raw_read`/`raw_write` 逻辑绕过高阶闭包组合子；若不从通用环境交付约定上根治，后续 `Vec.retain`、`MutSlice.sort_by`、`HashMap.entry.or_insert_with` 与 Tier 2 `Serialize` 派生均无法组合工作。
2. **运行时 `str` 视图缺少 Backing-Region Provenance 证明（[issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md), [native-text-method-provenance.fl](file:///home/fin/luna/development/repros/native-text-method-provenance.fl)）**：
   - **实证**：`pointer_input_layout` 目前拒绝 `str` backing-region 输入。因此 `library/std/string_impl/string.fl` 有 927 LOC 却无法安全提供 `String.as_str(*self) -> str`、zero-copy `lines`/`split`/`trim` 返回 `str` 切片或未来的 `CStr.to_str`；而 [file.fl:29-41,92-141](file:///home/fin/luna/library/std/fs_impl/file.fl#L29-L141) 更被迫将全部文件路径签名锁死在 `where path: str` / `comptime path: str` 上（形成动态文件 I/O 的致命可用性断层）。
3. **`builtin.Eq`/`Ord`/`Index` 按值接收者（`self` vs `*self`）导致仿射容器语义扭曲**：
   - **实证**：[comparison.fl:6-13](file:///home/fin/luna/library/builtin/numeric/comparison.fl#L6-L13) 定义 `fn eq(self, other: Self) -> bool` 与 `fn cmp(self, other: Self) -> Order`，[ops.fl:85](file:///home/fin/luna/library/builtin/protocols/ops.fl#L85) 定义 `fn index(self, idx: Idx) -> *Output`。在 Flurry 的 `Once` 默认仿射契约下，[vec.fl:755](file:///home/fin/luna/library/std/collections_impl/vec.fl#L755)（`Vec.eq`）与 [string.fl:840,853](file:///home/fin/luna/library/std/string_impl/string.fl#L840-L858)（`String.eq`/`cmp`）一旦用 `a == b` 或 `a < b` 比较，就会立刻消费并析构 `a` 与 `b`；同时 `Vec`/`VecDeque`/`HashMap` 全员无法实现 `Index`（消费 `self` 后返回指向已释放缓冲的 `*Output` 悬垂指针必定不健全），只能额外提供 `equals(*self, other: *...)` 和 `get_cloned` 作为补丁。
4. **`^attrs.*` 顶层新官方键白名单封堵、`macro` 模块漏导出与无 `extern` FFI 缺失**：
   - **实证**：[attributes.rs:126-159](file:///home/fin/luna/compiler/elaboration/src/program/attributes.rs#L126-L159) 的 `prepare_phase_attributes` 现已通过 `domain` 放行 `__flurry_macro_kind` / `__flurry_macro_variadic` / `__flurry_macro_grammar` / `__flurry_stable_since` 及另外 7 个键（包括遗留 `"packed"` at `:145`），但在包导出层 [meta/macros_impl/attributes.fl:1](file:///home/fin/luna/library/meta/macros_impl/attributes.fl#L1) 仍只重导出 `attrs.macros.{macro3, macro4}`（遗漏 `ast`/`peg`）；同时 `:140-159` 仍拒绝 `std`/Tier 2 所需的 `__flurry_align`、`__flurry_ffi_link*`、`__flurry_effect_strategy`、`__flurry_purity_*`、`__flurry_pipeline*` 等新官方键，且对 `__flurry_abi`（`:108-125` & [control.rs:198-224](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L198-L224)）强检 `name == "c"` 与闭标量签名，使 `std.os`/`std.ffi` 被迫把系统调用逐个硬编码进 [nir/src/intrinsics.rs:101-111](file:///home/fin/luna/compiler/nir/src/intrinsics.rs#L101-L111) 与 [platform.ll](file:///home/fin/luna/compiler/luna/src/host/llvm/platform.ll)。

### 4.2 `std` 内部系统能力覆盖断层：从玩具原型到系统编程的 4 道坎

尽管已有的 `Vec`/`VecDeque`/`HashMap`/`String`/`BufReader`/`BufWriter` 在设计质量（运行时依赖 allocator、ZST 处理、RFC 3629、OOM 错误显式返回）上非常扎实，但与真实 CLI 工具、编译器前端（自举）或网络服务需求相比，仍有 4 类不可绕过的系统缺失：

1. **动态文件路径、目录与环境参数缺失**：程序拿不到 `std.env.args()`，拿不到 `std.env.var(...)`，连打开命令行传入的一个动态文件名 `File.open` 都做不到（[file.fl:29](file:///home/fin/luna/library/std/fs_impl/file.fl#L29)）。
2. **`MaybeUninit`/`ManuallyDrop` 与 `union` 免 `Drop` 缺失**：[buffered.fl:312-442](file:///home/fin/luna/library/std/io_impl/buffered.fl#L312-L442) 中 `BufReader` 提供 `into_inner(self) -> R`（`:67`），而 `BufWriter` 因实现了 `impl Drop` 触发先 `flush_buf()` 再释放在结构上无法直接拆出 `self.inner`，被迫不提供 `into_inner`；所有元素原地借用操作（`Box.with_ref`、`HashMap` 探查与克隆）都通过 `raw_read<T>` 把值搬到栈再 `raw_write<T>` 写回，无法在原址上构造可控未初始化槽。
3. **`panic` 终止契约未决导致的通用缺失**：[library/README.md:393-396](file:///home/fin/luna/library/README.md#L393-L396) 指出 `OptionalUnwrap.unwrap`、`ErrorResultUnwrap.unwrap`（`builtin/types/{option,result}_unwrap.fl`）仍处于无法脱离 stub 单独交付状态；同时 `std` 中无统一崩溃入口，`fmt_impl/` 用 `assumes false;` 作为不可达断言兜底，使得生产边界错误语义不明确。
4. **Tier 1 `std.runtime` / `std.sync` / `std.net`（Wave 2）完全空白**：`std.sync.atomic` 缺失同步阻塞了 `Arc`/`ArcWeak`（`WL-07-03`）；`builtin.Continuation` 与 `BoundedStackPool` 缺失直接硬阻断 Silver GUI（`P0-3`/`P1-1`/`P1-3`）及异步运行时；`std.net` 缺失使语言尚不具备任何网络 I/O 能力。

### 4.3 Tier 2 官方薄协议包与 Tier 3 生态工具链差距

1. **Tier 2 官方薄协议包全量空白（0 / 17 组）**：[10-official-protocol-packages-and-boundary.md](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md) 系统规划了从 `(1) serialization`、`telemetry`、`crypto`/`digest`、`random`、`async_runtime`、`http`/`wire`、`database` 到 `testing`、`ffi_bridge`、`regex_protocol`/`parser_protocol`、`collections_ext` 共 **17 组**非内置协议包，但在编译器端：
   - 开放 `Symbol` 标签参数化派生策略 `derive Trait<.tag>.use(strategy)` 尚未全面闭环（`WL-10-01`/`02`）；
   - 多属性叠加 `__flurry_metadata` 浅覆盖尚未解决（`WL-10-03`），使得 Tier 2 的非 `__flurry_*` 领域属性易发生相互覆盖；
   - `Type.type_id` 与结构化跨包插件 ABI 尚未落地（`WL-10-20`）。
2. **`luna` 单二进制的工程管理层级停留在低层驱动**：依据 `AGENTS.md` §1，独立 `anar` 计划已取消，现存 [compiler/anar](file:///home/fin/luna/compiler/anar/Cargo.toml) 仅为内嵌 LSP/IDE crates；但 [compiler/luna/src/cli.rs](file:///home/fin/luna/compiler/luna/src/cli.rs) 与 [packages.rs](file:///home/fin/luna/compiler/luna/src/packages.rs) 仅提供需要调用方（如测试脚本）显式传入 `--package <id> <name> <entry> --dependency ...` 的底层包集合编译接口，尚缺少用户可执行的 `package.fl` 声明式解析、多包工程根目录发现、版本锁文件、第三方拉取缓存以及 `library/build/` 端到端任务 DAG 执行器（目前 `library/build/` 仅 5 个文件 121 LOC，只暴露 `mode`/`target`/`package_metadata` 只读观测）。

---

## 5. 架构偏离与技术债清单（`⚠️ Divergent` 与关键设计债务专项）

> 注：经全量扫描核实，`library/std` 的 22 个 `.fl` 文件**已清零**早期历史原型常见偏离项（`Option`/`Result` 枚举 = **0**、`::` 路径 = **0**、使用处直写 `.symbol'builtin` = **0**、`SystemAllocator` 硬编码进容器 = **0**）。`5.1` 与第 3 节全部 **3 项 `⚠️ Divergent`**（`GAP-09-13, 22, 36`）严格 **1:1 对齐**，`5.2` 单列非 `⚠️ Divergent` 的跨层结构性技术债。

### 5.1 架构偏离项（`DIV-09-01` ~ `DIV-09-03`，1:1 映射 `⚠️ Divergent`）

| 编号 | 对应 `GAP` | 偏离项 | 源码证据与定位 | 违背 / 偏离的规范 | 影响与修复要求 |
|---|---|---|---|---|---|
| **DIV-09-01** | `GAP-09-13` | `comptime_format` 对 `.debug` 格式位硬编码 `assumes false;` 且 `Display` 携带 `comptime_fmt` 耦合 | [arguments.fl:98-119](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L98-L119), [parse.fl:124,150,172..302](file:///home/fin/luna/library/std/fmt_impl/parse.fl#L124-L302) | `AGENTS.md` §1（禁止占位规避）& `00` §6.1 #5（`WL-04-21` 双阶段格式化分离） | 编译期 `comptime_format("{format: .debug}", x)` 会直接命中 `:113` 触发异常；须接入 `pure comptime trait`（`ComptimeDisplay`/`ComptimeDebug`）并与 runtime `ByteWriter` 解耦 |
| **DIV-09-02** | `GAP-09-22` | `std` 模块文档陈述过时且 `docs/others/ffi.md` / `docs/concurrency/software-transactional-memory.md` 为 0 字节 | [collections.md:9](file:///home/fin/luna/library/std/collections.md#L9), [std/README.md:42](file:///home/fin/luna/library/std/README.md#L42), [docs/others/ffi.md](file:///home/fin/luna/docs/others/ffi.md) | `AGENTS.md` 目录与文档同步要求 | `collections.md` 仍称 `map`/`set`/切片未实现、`std/README.md:42` 仍称 `process`/`time` 未实现；两份核心规范文档空白使 `std.ffi` 与 `std.sync.stm` 缺乏权威参考文本 |
| **DIV-09-03** | `GAP-09-36` | `builtin.Eq`/`Ord`/`Index` 按值接收 `self`；`attributes.rs` 保留 `"packed"` 并阻断新 `__flurry_*` 键与多 ABI；`meta` 漏导出 `ast`/`peg` | [comparison.fl:6-13](file:///home/fin/luna/library/builtin/numeric/comparison.fl#L6-L13), [ops.fl:85](file:///home/fin/luna/library/builtin/protocols/ops.fl#L85), [attributes.rs:108-159](file:///home/fin/luna/compiler/elaboration/src/program/attributes.rs#L108-L159), [control.rs:198-224](file:///home/fin/luna/compiler/luna/src/host/llvm/control.rs#L198-L224), [meta/macros_impl/attributes.fl:1](file:///home/fin/luna/library/meta/macros_impl/attributes.fl#L1) | `docs/resource-management/once-clone-copy.md`、`docs/traits/protocols.md`、`docs/comptime/official-attribute-keys.md`、`AGENTS.md` §5 | 导致 `Vec.eq`/`String.eq`/`cmp`（[vec.fl:755](file:///home/fin/luna/library/std/collections_impl/vec.fl#L755), [string.fl:840](file:///home/fin/luna/library/std/string_impl/string.fl#L840)）消费操作数且全 `std` 集合无法实现 `Index`；阻断 `std.simd` 对齐、`std.ffi` 非 `.c`/导入 ABI 及 `^attrs.macros.{ast,peg}` 重导出 |

### 5.2 非 `⚠️ Divergent` 跨层结构性技术债（`TD-09-NN`）

| 编号 | 技术债项 | 关联范围 | 现状与待定收敛决策 |
|---|---|---|---|
| **TD-09-01** | Tier 1 内部与 Tier 2 边界归属待定（`Slice<T>`/`Layout`/`FormatErr` 及 `SmallVec`/`SlotMap`/`SoA` 分流） | `GAP-09-07`, `GAP-09-09`, `GAP-09-33`; [00-index-and-roadmap.md:338-349](file:///home/fin/luna/tmp/wishlist/00-index-and-roadmap.md#L338-L349)（#5, #6, #9）; [slice.fl](file:///home/fin/luna/library/std/collections_impl/slice.fl) | 若把 `Array.as_slice`/`char.encode_utf8` 纳入 `builtin`，现置 `std` 的 `Slice<T>` 将造成包循环依赖；`FormatErr`/`Formatter` 与 `SmallVec`/`SlotMap`/`SoA` 是否留在 `std` 抑或拆往 Tier 2 `collections_ext` 需收敛收口以防 `std` 膨胀 |

---

## 6. 健全性 · 完备性 · 鲁棒性专项审计（`SND` / `CMP` / `ROB`）

### 6.1 健全性问题（`SND-09-01` ~ `SND-09-07`）

- **`SND-09-01` [Critical (`P0/blocker`)] `Vec.remove` 早退留值导致仿射元素 Double-Drop / 内存损坏**：
  在 [vec.fl:352-377](file:///home/fin/luna/library/std/collections_impl/vec.fl#L352-L377) 中，`Vec.remove(*self, index: usize) -> ?T` 先通过 `raw_read<T>` 读出 `value`，随后若进入元素搬移分支 `if Type.size_of(T) != 0 and index < new_len` 且 `elem_stride<T>()` 或 `checked_mul_usize(tail_count, stride)` 返回 `null` 时（`:361-369`）：
  ```flurry
  if Type.size_of(T) != 0 and index < new_len {
      let stride = elem_stride<T>() match {
          s? => s,
          null => return value,
      }
      ...
  }
  self.raw_len.* = new_len;
  value
  ```
  函数在**未执行 `self.raw_len.* = new_len;`（`:376`）且未左移后续元素**的情况下提前 `return value`。同一仿射值既已返回给调用方，又继续滞留在原索引槽位并被计入 `raw_len`，在 `Vec.drop()` 时必然再次析构（Double-Drop）。应当先验证全部偏移/算术布局再 `raw_read<T>`。

- **`SND-09-02` [High (`P1`)] `Box.with_ref` 与 `HashMap` 内部槽位搬出-写回模式违背地址稳定性并隐藏异常泄漏风险**：
  [boxed.fl:116-124](file:///home/fin/luna/library/std/mem_impl/boxed.fl#L116-L124)（`with_ref`）与 [hash_map.fl:648-653, 892-897, 972-983](file:///home/fin/luna/library/std/collections_impl/hash_map.fl#L648-L653)（`find_slot`、`get_cloned`、`try_clone`）均通过 `var val = unsafe { builtin.mem.raw_read<T>(addr) }` 把堆上元素按位拷贝到栈变量再 `raw_write<T>(addr, val)` 写回：
  1. 破坏堆元素地址稳定性和未来自引用结构安全（元素在 `f(val.ref)` 期间地址已变成栈地址而非堆地址）；
  2. 虽当前避开了 `?`/`!` 早退，但若未来引入异常展开/panic（或 `f` 触发 trap），原堆槽位内将遗留一份已转至栈上且会被栈析构的位镜像，造成双析构。应逐步改为直接构造指向槽位的 `*T`（待编译器支持 `Box` 内指针解引用/安全偏移证明）。

- **`SND-09-03` [High (`P1`)] `FixedBufferAllocator` 缺失泄漏防线与 `ArenaAllocator` 错误路径资源泄漏**：
  - [arena.fl:138-159](file:///home/fin/luna/library/std/mem_impl/arena.fl#L138-L159)：`FixedBufferAllocator` 无 `impl Drop`，且不追踪与底层 `RawBlock` 的存活作用域绑定，即使 `live_count > 0` 销毁也不会触发任何检查；非尾部 `dealloc_raw` 仅递减 `live_count` 而不回收字节，反复小释放会导致缓冲提前耗尽。
  - [arena.fl:327-335](file:///home/fin/luna/library/std/mem_impl/arena.fl#L327-L335)：`ArenaAllocator.reset` 若重建旧块 `Layout.new(hdr.block_size, hdr.block_align)` 返回 `null`，直接 `continue` 静默泄漏整块 slab；同时 [arena.fl:507-512](file:///home/fin/luna/library/std/mem_impl/arena.fl#L507-L512)（`allocate_zeroed`）在 `alloc_raw(layout)!` 成功后若 `byte_size_usize()` 返回 `null`，直接 `return error AllocErr.InvalidLayout` 而未归还/回退刚申请的块。

- **`SND-09-04` [High (`P1`)] `Slice<T>` 无可达性生命周期绑定且 `slot_address` 溢出静默归零**：
  - [slice.fl:11-56](file:///home/fin/luna/library/std/collections_impl/slice.fl#L11-L56)：`Slice<T>` 仅为裸 `{ raw_addr: usize, raw_len: usize }` 且无条件实现 `Copy`（`where T`），无 `&r` 可达性域或 `quote` 来源绑定。安全代码一旦经 `Vec.list_pattern_view_rest`（[vec.fl:878-887](file:///home/fin/luna/library/std/collections_impl/vec.fl#L878-L887)）拿到 `Slice<T>`，在原 `Vec` 扩容或离开作用域后，`Slice<T>` 成为未受编译器阻止的悬垂句柄。
  - [slice.fl:36-43](file:///home/fin/luna/library/std/collections_impl/slice.fl#L36-L43)：`slot_address` 在 `checked_mul_usize` / `checked_add_usize` 返回 `null` 时静默返回 `0`（空地址），随后直接喂给 `unsafe { raw_read<T>(0) }`。

- **`SND-09-05` [Critical (`P0/blocker`)] `BufReader.read_raw` 与 `BufWriter.buffer_byte` 在地址算术溢出时静默返回成功/EOF**：
  - [buffered.fl:239-243, 268-272](file:///home/fin/luna/library/std/io_impl/buffered.fl#L239-L272)：`BufReader.read_raw` 中 `checked_add_usize(self.buf.as_addr(), pos)` 若返回 `null`，代码走 `null => return 0` ——把指针溢出故障伪装成正常的 `0` 字节 EOF，破坏调用方对数据完整性的判断；
  - [buffered.fl:375-383](file:///home/fin/luna/library/std/io_impl/buffered.fl#L375-L383)：`BufWriter.buffer_byte` 中两次 `checked_add_usize` 遭遇 `null` 直接 `null => return`（静默返回 `!W.Err void` 成功并永久丢弃当前字节）。

- **`SND-09-06` [High (`P1`)] `write_debug_with_format` 吞并 `ProbeWriter` 错误返还成功**：
  [arguments.fl:47-51](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L47-L51) 中，当格式化带对齐宽度的 `.debug` 参数时，若目标 `Debug.write_debug(probe.ref)` 返回错误，分支写为 `error _ => return,`——在返回类型 `!W.as(ByteWriter).Err void` 下直接返回成功 `void`，静默中断后续格式段与错误传播。

- **`SND-09-07` [High (`P1`)] `Stdin`/`Writer` 原始非拥有句柄缺少作用域保护且 `File.close` 缺负句柄守卫**：
  [reader/mod.fl:121-131](file:///home/fin/luna/library/std/io_impl/reader/mod.fl#L121-L131)（`Stdin`）与 [writer/mod.fl:67-77](file:///home/fin/luna/library/std/io_impl/writer/mod.fl#L67-L77)（`Writer`）均包装裸 `i32` 却实现 `Copy`；且 [fs_impl/file.fl:85-89](file:///home/fin/luna/library/std/fs_impl/file.fl#L85-L89) 的 `File.close(*self)` 未像 `Drop`（`:21`）那样先检查 `fd >= 0`，重复调用 `close` 将对 `-1` 发起系统调用并返回 spurious `EBADF`。

### 6.2 完备性问题（`CMP-09-01` ~ `CMP-09-06`）

- **`CMP-09-01` [Critical (`P0/blocker`)] `std.fs` 全线依赖编译期路径（无法操作任何运行时路径）**：
  [fs_impl/file.fl:29, 36, 92, 96, 111, 123, 129](file:///home/fin/luna/library/std/fs_impl/file.fl#L29-L129) 全部限定 `where path: str` 或 `comptime path: str`。无法打开从 CLI 参数、环境变量或配置拼接得到的运行时路径，阻断一切实际工程程序。
- **`CMP-09-02` [Critical (`P0/blocker`)] `Eq`/`Ord`/`Index` 值传递限制与集合索引接口全线空缺**：
  全 `library/std/` 没有任何集合实现 `Index` / `IndexMut`（[ops.fl:82-90](file:///home/fin/luna/library/builtin/protocols/ops.fl#L82-L90)），且 `Vec`/`VecDeque`/`String` 的 `Eq`/`Ord` 消耗双方所有权。
- **`CMP-09-03` [Critical (`P0/blocker`)] `HashMap` 强制 `K :- Clone`、固定无种子散列、无 `Entry` 与借用视图**：
  [hash_map.fl:235-245, 616, 649-653, 799, 834](file:///home/fin/luna/library/std/collections_impl/hash_map.fl#L614-L660) 固定使用无随机化 `mix_hash`（易遭 HashDoS 退化），`find_slot` 等内部路径每次查找都对表内键及探查键各 `clone()` 一次，阻止非 `Clone` 仿射键使用 `HashMap` 并缺少 `Entry`/`get_ref`/`keys`/`values`/`iter_ref`。
- **`CMP-09-04` [Critical (`P0/blocker`)] `String.as_str`、`FromUtf8Err` 载荷保留及核心数据结构缺位**：
  [string.fl:333-340](file:///home/fin/luna/library/std/string_impl/string.fl#L333-L340) `from_utf8` 验证失败时仅返回 `Utf8Err` 并丢弃原 `Vec<u8>` 堆缓冲，未实现保留缓冲的 `FromUtf8Err<.A, .allocator>`；`String.as_str` 阻断于 `text_provenance`；`MutSlice`、`BTree*`、`BinaryHeap`、`Path`/`PathBuf`、`std.env`、`std.ffi`、`std.simd` 仍为零实现。
- **`CMP-09-05` [High (`P1`)] `std.fmt` 入口缺口与标准类型 `Display` / `BufWriter.into_inner` 缺失**：
  `print`/`eprint`（[arguments.fl:16, 160](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L16-L162)）仅收单一 `comptime text: str` 而不支持格式化变参；全 `std` 所有 `*Err` 结构体/枚举与 `Duration`/`ExitCode`/`Layout` 均缺 `Display` 实现；`BufWriter` 缺少 `into_inner`。
- **`CMP-09-06` [Critical (`P0/blocker`)] Wave 2 并发/异步/网络与全部 17 组 Tier 2 协议包及工程管理链归零**：
  `std.runtime`、`std.sync`、`std.sync.stm`、`std.net`、17 组 Tier 2 薄协议包（`WL-10-06..22`）及 `package.fl` 构建系统均未落地；`platform/linux.fl` 仅支持 11 个 Linux x86_64 C 桥接调用。

### 6.3 鲁棒性问题与 `library/std/` 逐文件量化普查（`ROB-09-01` ~ `ROB-09-05`）

- **`ROB-09-01` [Critical (`P0/blocker`)] `std.fmt` 中充斥 11 处 `assumes false;` 运行/编译期陷阱**：
  全 `library/std` 共 20 处 `assumes`，其中 **11 处为 `assumes false;`**（[parse.fl:124, 150, 172, 186, 208, 222, 246, 261, 287, 302](file:///home/fin/luna/library/std/fmt_impl/parse.fl#L120-L307) ×10 + [arguments.fl:113](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L113) ×1）。[parse.fl:287, 302](file:///home/fin/luna/library/std/fmt_impl/parse.fl#L283-L307) 将错误分支硬压成 `assumes false;` 并返回 `[]`，而非结合验证证明或显式错误返回（[arguments.fl:113](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L113) 更会在 `{format: .debug}` 调用 `comptime_format` 时真实命中）。
- **`ROB-09-02` [High (`P1`)] `ByteWriter.write_str` 逐字节写入与 `format_into` 双重冗余分配**：
  `Vec<u8>.write_str`（[vec.fl:811-827](file:///home/fin/luna/library/std/collections_impl/vec.fl#L811-L827)）与 `BufWriter.write_str`（[buffered.fl:535-546](file:///home/fin/luna/library/std/io_impl/buffered.fl#L535-L546)）仍使用 `while index < len` 逐字节调用 `str_byte` + `push`/`buffer_byte` 写入 `str`，未利用 `str` 内存块批量拷贝；`format_into`（[arguments.fl:138-152](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L138-L152)）先建临时 `Vec<u8>`，再 `from_utf8_lossy` 转 `String`，再 `target.push_string`，产生两次堆缓冲/复制开销。
- **`ROB-09-03` [High (`P1`)] 硬编码 Linux 魔数与局部逻辑重复**：
  [reader/mod.fl:74, 146, 151](file:///home/fin/luna/library/std/io_impl/reader/mod.fl#L140-L157)、[writer/mod.fl:42, 54](file:///home/fin/luna/library/std/io_impl/writer/mod.fl#L40-L60) 与 [file.fl:168, 173](file:///home/fin/luna/library/std/fs_impl/file.fl#L166-L176) 多处硬编码 Linux `EIO` (`5`) 与 `EINVAL` (`22`) 魔数构造 `IoErr.Os(...)`；[linux.fl:71-78](file:///home/fin/luna/library/std/platform/linux.fl#L71-L78) 硬编码 x86_64/glibc `open` 位标志；[string.fl:788-805](file:///home/fin/luna/library/std/string_impl/string.fl#L788-L805) 存在 `chars < 10000000` 人工截断与 `O(n)` UTF-8 计数；[duration.fl:28-60](file:///home/fin/luna/library/std/time_impl/duration.fl#L28-L60) 重复内联实现 FNV-1a 字节混合。
- **`ROB-09-04` [Medium (`P2`)] `BufWriter.drop` 刷新错误静默丢失与零容量静默夹断**：
  [buffered.fl:439-441](file:///home/fin/luna/library/std/io_impl/buffered.fl#L439-L441) `let _ = self.flush_buf();` 静默忽略销毁写回失败（与缺失 `into_inner` 叠加导致无法恢复末次写错）；[buffered.fl:40, 325](file:///home/fin/luna/library/std/io_impl/buffered.fl#L40) 对传入 `capacity = 0` 静默夹断成 `1`；[mem_impl/arena.fl:138-159](file:///home/fin/luna/library/std/mem_impl/arena.fl#L138-L159) 中段释放仅降计数器不收缩偏移。
- **`ROB-09-05` [High (`P1`)] Sysroot 集成检查链路断层（`library_check.py` 缺 `std` 阶段，冒烟未过全库）**：
  [development/library_check.py](file:///home/fin/luna/development/library_check.py)（共 38 个 stage）全部聚焦 `builtin`/`meta`/`attrs`/`build`，完全没有把 `std` 原生测试纳入统一门禁；同时 [library/README.md:393-396](file:///home/fin/luna/library/README.md#L393-L396) 记录全库 `sysroot` 仍受 `panic` 与空程序 `flow` 预算问题（[issues/0001:208](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L208)）制约，导致 [library-smoke.fl](file:///home/fin/luna/development/examples/library-smoke.fl) 不敢导入任何 `std` 符号。

#### `library/std/**/*.fl` 全部 22 个源码文件量化普查表（实时脚本核实）

| 相对路径（`library/std/` 下） | LOC | 顶层 `pub` | `impl` 内 `pub` | `unsafe` | `assumes` | 独立 `_ =>`（不含 `error _`） | `impl Drop` | 主要安全/质量观察 |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| `collections_impl/hash_map.fl` | 1,228 | 2 | 17 | 63 | 2 | 0 | 2 | `SND-09-02`, `CMP-09-03`（SoA + SplitMix64 + 强制 `K :- Clone`） |
| `collections_impl/hash_set.fl` | 200 | 2 | 13 | 0 | 0 | 0 | 0 | 复用 `HashMap<T, void>` 结构化 `Drop`，缺集合代数运算符 |
| `collections_impl/slice.fl` | 216 | 1 | 11 | 8 | 0 | 0 | 0 | `SND-09-04`（无 `&r` 区域、`Copy`、溢出归零） |
| `collections_impl/vec.fl` | 896 | 2 | 30 | 46 | 0 | 0 | 2 | `SND-09-01`（`remove` 早退留值）, `ROB-09-02`（逐字节 `write_str`） |
| `collections_impl/vec_deque.fl` | 737 | 2 | 18 | 25 | 0 | 0 | 2 | ZST 支持与成对环形 `Drop` 完整，`Eq` 受按值 `self` 约束 |
| `fmt_impl/arguments.fl` | 193 | 11 | 0 | 0 | 1 | 0 | 0 | `DIV-09-01`, `SND-09-06`（另 `:50` 含 1 处 `error _ =>`）, `CMP-09-05`, `ROB-09-02` |
| `fmt_impl/parse.fl` | 307 | 5 | 10 | 0 | 13 | 1 | 0 | `ROB-09-01`（10 处 `assumes false;`；1 处独立 `_ =>` `:97` + 5 处 `error _ =>`） |
| `fs_impl/file.fl` | 211 | 6 | 7 | 6 | 0 | 0 | 1 | `CMP-09-01`（强制 `comptime` 路径）, `SND-09-07`（重复 `close`） |
| `io_impl/buffered.fl` | 547 | 2 | 22 | 11 | 0 | 0 | 1 | `SND-09-05`（溢出静默 EOF/丢字节）, `ROB-09-02/04`（缺 `into_inner`） |
| `io_impl/errors.fl` | 68 | 2 | 0 | 0 | 1 | 0 | 0 | `check` 正确校验 `INT64_MIN`，`IoErr` 缺 `Display` 与 `IoErrKind` |
| `io_impl/reader/mod.fl` | 254 | 4 | 7 | 5 | 0 | 0 | 0 | `SND-09-07`（`Stdin` Copy 句柄）, `ROB-09-03`（魔数 errno `5`/`22`） |
| `io_impl/writer/mod.fl` | 248 | 7 | 3 | 12 | 2 | 0 | 0 | `SND-09-07`（`Writer` Copy 句柄）, `ROB-09-03`（魔数 errno `5`/`22`） |
| `lib.fl` | 102 | 11 | 33 | 0 | 0 | 0 | 0 | 公开根命名空间与 `std.prelude`（11 项顶层 `pub` 导出/模块、33 条嵌套 `pub use`） |
| `mem_impl/allocator.fl` | 295 | 6 | 7 | 7 | 0 | 0 | 0 | 运行时依赖 `Allocator`/`DefaultAllocator` 基线；缺 `Display` 与 `realloc` |
| `mem_impl/arena.fl` | 604 | 2 | 11 | 17 | 0 | 0 | 1 | `SND-09-03`（`FixedBufferAllocator` 无 Drop、`Arena.reset`/`zeroed`） |
| `mem_impl/boxed.fl` | 155 | 1 | 8 | 14 | 0 | 0 | 1 | 零开销 allocator + `Drop` 示范；`SND-09-02`（`with_ref` 栈搬出） |
| `mem_impl/layout.fl` | 166 | 1 | 10 | 0 | 0 | 0 | 0 | 缺 `Eq`/`Hash`/`Display`/`dangling`/comptime 求值 |
| `platform/linux.fl` | 78 | 21 | 1 | 12 | 0 | 0 | 0 | `CMP-09-06`（仅 11 个 Linux C-shim intrinsic，x86_64 常量） |
| `platform/unix.fl` | 81 | 11 | 0 | 2 | 0 | 0 | 0 | 正确区分 `close` 不重试 `EINTR` 与读写重试 |
| `process_impl/process.fl` | 71 | 4 | 3 | 0 | 0 | 0 | 0 | 仅 `id` 与 `ExitCode`（缺 `Display`/进程启动/`panic`/`exit`） |
| `string_impl/string.fl` | 927 | 2 | 33 | 3 | 1 | 0 | 0 | 完整 RFC 3629 + `Display`；`CMP-09-04`（缺 `as_str`/`FromUtf8Err`） |
| `time_impl/duration.fl` | 341 | 7 | 19 | 0 | 0 | 0 | 0 | 算术健壮（`u128` 检查），缺 `Display`/运算符与 `Instant`/`SystemTime` |
| **合计（22 文件 + 15 `.md`）** | **7,925** | **112** | **263** | **231** | **20** | **1** | **10** | `Option`/`Result` = 0，`::` = 0，`.builtin` = 0，`SystemAllocator` 跨文件污染 = 0 |

> 测试基线对照：仓内 [std_allocators_collections_io.rs](file:///home/fin/luna/compiler/luna/tests/std_allocators_collections_io.rs)（899 行 / 7 个原生测试用例）、[build_and_std_stage2c.rs](file:///home/fin/luna/compiler/luna/tests/build_and_std_stage2c.rs)（702 行 / 5 个原生测试用例）、[library_std.rs](file:///home/fin/luna/compiler/luna/tests/library_std.rs)（589 行）与 [library_std_format.rs](file:///home/fin/luna/compiler/luna/tests/library_std_format.rs)（377 行）覆盖已有公开接口，但在上述 `SND-09-01/03/05/06/07` 异常/溢出边界与空 sysroot 冒烟链路上存在明显盲区。

---

## 7. 修复路径、依赖与验收建议

按"先修复已有 unsound bug、打通编译器硬阻断 -> 补齐 `std` 系统/并发基础设施 -> 解锁 Tier 2 官方薄协议与工程 DAG"的纵向可验收路线排期：

### Phase 0（P0 — 止血修复 + 解锁编译器核心硬阻断，预计前置优先）
1. **立即修复 `std` 现有 `P0` 健全性与鲁棒性漏洞**：
   - 修复 [vec.fl:352-377](file:///home/fin/luna/library/std/collections_impl/vec.fl#L352-L377) `Vec.remove`（先检查 stride/乘法再 `raw_read<T>` 并确保 `raw_len` 更新不可跳过）(`SND-09-01`)；
   - 修复 [buffered.fl:239-272, 375-383](file:///home/fin/luna/library/std/io_impl/buffered.fl#L239-L383) 溢出静默返还 `0`/`void` 漏洞（`SND-09-05`）与 [arguments.fl:48-51](file:///home/fin/luna/library/std/fmt_impl/arguments.fl#L48-L51) `ProbeWriter` 吞错问题（`SND-09-06`）；
   - 替换 `fmt_impl/parse.fl` 的 `assumes false;` 为显式状态检查或传播（`ROB-09-01`），修复 `arguments.fl:113` `.debug` 编译期格式化分支（接入 `ComptimeDebug` 或给出可控编译错误）。
2. **打通卡死 `std` 与 Tier 2 的 5 项编译器基础能力**：
   - 解决 [issues/0001:318-366](file:///home/fin/luna/issues/0001-runtime-dependent-origin-and-default-param.md#L318-L366) 闭包/泛型方法特化 `RuntimeEnvParameter` 隐式操作环境交付；
   - 解决 [issues/0003](file:///home/fin/luna/issues/0003-builtin-packages-compiler-blockers.md) / `pointer_input_layout` 的 `str` backing-region provenance，解锁 `String.as_str`（`GAP-09-11`）；
   - 将 `builtin.Eq`/`Ord`/`Index` 迁移至 `*self`（`DIV-09-03`）并开放 [attributes.rs:140-159](file:///home/fin/luna/compiler/elaboration/src/program/attributes.rs#L140-L159) 新 `__flurry_*` 键白名单、多 ABI 入口及 `meta/macros_impl/attributes.fl` 重导出；
   - 裁决并落地 `std.process.panic` 终止/逆序 `Drop` 合同（`library/README.md:395`），补齐 `Array`、`MaybeUninit`/`ManuallyDrop`（确认 `union` 免 `Drop`）。

### Phase 1（P1 — `std` 系统编程完备化 + `std.runtime` / `std.sync` / `std.net` 落地）
1. **解除 `std.fs` / `std.ffi` / `std.os` / `std.env` / `std.time` / `std.process` 不可用局面**：
   - 通过 `^attrs.abi` + `__flurry_ffi_link` 支持无 `extern` 外部 C 符号 `declare` 与聚合 ABI（`GAP-09-19`），摆脱对固定 11 项 `platform.ll` intrinsic 的硬编码；
   - 实现运行时路径版 `File`/`OpenOptions`/`Metadata`/`ReadDir`、`Path`/`PathBuf`、`std.env`、`Instant`/`SystemTime`/`sleep`、`Command`/`Child`/`exit`；
   - 为 `HashMap` 引入 `.H: BuildHasher`、无 `Clone` 探查（基于借用 `hash`/`eq`）及 `Entry` API；补齐 `MutSlice<T>`、`String` 切片算法与 `FromUtf8Err`，为全 `std` 错误与基础类型补齐 `Display`。
2. **落地 Wave 2 并发、控制与网络基线（`08` + Silver P0/P1）**：
   - 落地 `builtin.Continuation`（C 帧屏障 + `Once`）、`HandleError`/`HandleEffect` trait、`std.runtime.{TypedContinuation, StackPool, BoundedStackPool, TaskScope, CancelToken}`；
   - 落地 `std.sync.atomic`（`Send`/`Sync` 解锁 `Arc`/`ArcWeak`）、`Mutex`/`RwLock`/`Condvar`/通道/`Thread`，以及 `std.net` 原始 socket。

### Phase 2（P2 — Tier 2 官方薄协议包、工程构建 DAG 与可移植加速）
1. **打通 Tier 2 其余协议能力并发布首批官方薄协议包（[10](file:///home/fin/luna/tmp/wishlist/10-official-protocol-packages-and-boundary.md)）**：
   - 完成 `derive Trait<.tag>.use(strategy)`、非 `__flurry_*` 深度合并元数据与双相 `Comptime*` 解析（`WL-10-01..05`）；
   - 率先交付 `(1) serialization`、`(5) async_runtime`、`(4) random`、`(14) testing`、`(16) regex_protocol`/`parser_protocol` 等高频协议包（严守不放入 Tier 1）。
2. **完善 `luna` 单二进制包/构建工作流与 `std.simd` / `Arena`/`SoA`**：
   - 在 `luna` 与 `library/build/` 落地 `package.fl` 声明式解析、多包依赖缓存/锁定与 `build.{package, targets, tasks}` DAG；
   - 落地 `^attrs.layout.align` 与 `std.simd`、`BTree*`/`BinaryHeap` 与界定收敛后的 `Arena`/`SlotMap`/`SoA`（[07](file:///home/fin/luna/tmp/wishlist/07-std-mem-collections-io-os.md) Q11）。

### Phase 3（P3 — 全栈验收门禁升级与跨平台扩展）
1. 将 `std`（含自定义 `FixedBufferAllocator`/`ArenaAllocator` 泄漏检查、溢出/空输入边界与跨模块 `serialization`/`async_runtime` 协同）纳入 [development/library_check.py](file:///home/fin/luna/development/library_check.py) 强制阶段，并令真实导入 `std.prelude.*` 的 `library-smoke.fl` 在解释器与 LLVM `-O0`/`-O3` 全线通过；
2. 补齐 `docs/others/ffi.md`、`docs/concurrency/software-transactional-memory.md` 与多平台（macOS/Windows/WASI）OS 适配层。
