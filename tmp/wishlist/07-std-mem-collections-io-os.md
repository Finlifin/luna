# 07. `std` 核心运行时库 Wishlist（`mem` / `collections` / `string` / `fmt` / `io` / `path` / `fs` / `time` / `env` / `process` / `ffi` / `os` / `simd`）

> 本文档为 `std` 核心运行时基础库的演进需求与完整规格清单（Wave 1）。覆盖内存管理与分配器生态、拥有型容器与运行时依赖 SoA/Arena、字符串与格式化、I/O 与文件系统、时间/环境/进程、以及 C FFI、多平台 OS 抽象与可移植 SIMD。并发、同步、异步运行时、网络与效应控制后端由后续 `08-effects-async-concurrency-net-runtime.md` 接续。

---

## 1. 范围、权威来源与归属包

### 1.1 权威依据与边界

按 [AGENTS.md](../../AGENTS.md) 语义权威顺序采信：
1. **作者明确确认**与当前正式文档：
   - [docs/type/runtime-dependent-type.md](../../docs/type/runtime-dependent-type.md) 与 [docs/compilation/runtime-dependent-types.md](../../docs/compilation/runtime-dependent-types.md)（运行时依赖类型、`quote` 环境交付与零存储开销布局原则）
   - [docs/code-organization/builtin-packages.md](../../docs/code-organization/builtin-packages.md)（六大内置包边界、`std.prelude` 显式导入、`str` 归 `builtin`、`String` 归 `std`、`mem` 命名）
   - [docs/grammar/index.md](../../docs/grammar/index.md) 与 [docs/grammar/syntax](../../docs/grammar/syntax)（点分路径、`PascalCaseErr` 错误类型、`PascalCase` 错误变体、`snake_case` 普通枚举变体、`N: usize` 通用子句参数、无 `extern` / `Option` / `Result` / `handles`）
   - [docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md)（官方属性统一通过 `^attrs.*` 封装 `__flurry_*` 键，支持 `^attrs.abi(.c)`、`^attrs.packed.{byte, bit}`、`^attrs.mem.volatile`、`^attrs.comptime_phase`、`^attrs.stable_since`）
   - [docs/resource-management/once-clone-copy.md](../../docs/resource-management/once-clone-copy.md)、[drop-elaboration.md](../../docs/resource-management/drop-elaboration.md)、[pointers.md](../../docs/resource-management/pointers.md)、[bindings.md](../../docs/resource-management/bindings.md)
   - [docs/algebraic-effect-and-delimited-continuation/error-handling.md](../../docs/algebraic-effect-and-delimited-continuation/error-handling.md) 与 [effect-abi.md](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)
   - [docs/type/user-defined-types/structs.md](../../docs/type/user-defined-types/structs.md)、[unions.md](../../docs/type/user-defined-types/unions.md)、[reflection-and-layout.md](../../docs/type/reflection-and-layout.md)
   - [docs/pipeline-projection/intrinsics.md](../../docs/pipeline-projection/intrinsics.md) 与 [declarations-and-interfaces.md](../../docs/pipeline-projection/declarations-and-interfaces.md)
   - 注：[docs/others/ffi.md](../../docs/others/ffi.md) 当前为空文件（0 字节），`std.ffi` 规范依托 `AGENTS.md`、`official-attribute-keys.md`、`effect-abi.md` 及现有 C ABI 后端事实整理，并在第 5 节单列未冻结裁决项。
2. **现状与非权威参考**：
   - 现有代码与日志基线：[library/std/](../../library/std/)、`library/STRUCTURE.md`、`library/README.md`、`issues/0001`、`issues/0003`、`ai_dev_logs/runtime_abi_inventory_2026-11-21.md`、`ai_dev_logs/builtin_packages_gap_report_2026-11-21.md`、`tmp/silver-design/04-compiler-wishlist.md`。
   - **历史草案截断边界**：仅最近半年的 `2026-5` 至 `2026-10` 窗口草案（本文中仅 `proposal_trace/2026-8-5-typed-Soa.fl`）可作问题域与意图参考，**仍不构成语义权威**，且一切语法与语义细节均须以 `docs/` 与 `AGENTS.md` 重写对齐；**严禁采信或引用任何 `2024-*`、`2025-*`、`2026-1..4-*` 草案或 `flurry_tutorial/`**。

### 1.2 归属包与 `std` 模块树

严格遵守“六大内置包（`builtin`、`meta`、`build`、`attrs`、`std`、`verification`）”约束，所有实现位于 `library/std/`，由 [library/std/lib.fl](../../library/std/lib.fl) 组织公开门面（实现目录采用 `*_impl/` 或 `platform/` 避免目录同名冲突）：

| 公开模块路径 | 职责与核心导出 | 底层实现位置 | 跨包协作归属 |
|---|---|---|---|
| `std.prelude` | 显式导入门面（`use std.prelude.*;`）：`Box`、`Vec`、`String`、`Slice`、`MutSlice`、`format`/`print`/`println`、`Display`/`Debug` 及 `builtin.prelude.*` | `library/std/lib.fl` | 依赖 `builtin.prelude` |
| `std.mem` | 分配器协议与生态、智能指针（`Box`/`Rc`/`Arc`/`Weak`/`Cow`/`Unique`）、`Layout`/`RawBlock`、`MaybeUninit`/`ManuallyDrop` 与内存工具 | `library/std/mem_impl/` | 依赖 `builtin.mem`、`std.os` |
| `std.collections` | 序列（`Vec`/`VecDeque`/`SmallVec`）、映射/集合/堆（`HashMap`/`HashSet`/`BTreeMap`/`BTreeSet`/`BinaryHeap`/`BitSet`/`RingBuffer`）、运行时依赖 `Arena`/`SlotMap`/`SoA`、`Slice`/`MutSlice` 算法 | `library/std/collections_impl/` | 依赖 `builtin.Array`、`RangeBounds`、`Hasher`、`meta` 反射 |
| `std.string` | `String`、`Utf8Err`、`FromUtf8Err`、`FromBytesWithNulErr`（与 `str` 严格分离） | `library/std/string_impl/` | 复用 `builtin.text` 与 `str` 视图证明 |
| `std.fmt` | 两相格式扫描 `FormatPlan`/`Parse`、`FormatErr`、`format`/`format_into`/`format_debug`/`write_fmt`/`print*`/`eprint*`、结构化 `Debug*` 构造器 | `library/std/fmt_impl/` | 导出 `builtin.fmt.{Display, Debug, ByteWriter}` |
| `std.io` | `Reader`/`Write`/`BufRead`/`Seek`/`SeekFrom`、`BufReader`/`BufWriter`/`LineWriter`/`Cursor`、`Stdin`/`Stdout`/`Stderr`、`IoErr`/`IoErrKind`、流组合子 | `library/std/io_impl/` | 依赖 `std.os`、`std.string` |
| `std.path` | 跨平台 `Path`、`PathBuf`、`Component`、`Prefix`、`PathErr` 与分隔符规范 | `library/std/path_impl/`（待建） | 依赖 `std.ffi.OsStr` / `OsString` |
| `std.fs` | `File`、`OpenOptions`、`Metadata`、`FileType`、`Permissions`、`DirEntry`、`ReadDir`、`FsErr` 与目录/文件操作 | `library/std/fs_impl/` | 依赖 `std.path`、`std.time`、`std.os` |
| `std.time` | `Duration`、`Instant`、`SystemTime`、`TimeErr`、`sleep` | `library/std/time_impl/` | 依赖 `std.os` 时钟原语 |
| `std.env` | 进程参数/环境变量/工作目录/`current_exe`/`temp_dir`/`VarErr`、`std.env.consts` | `library/std/env_impl/`（待建） | 依赖 `build.target` 与启动参数 ABI |
| `std.process` | `Command`、`Child`、管道句柄、`ExitCode`、`ExitStatus`、`Stdio`、`Output`、`ProcessErr`、`id`/`exit`/`abort`/`panic` | `library/std/process_impl/` | 依赖 `std.io`、`std.env`、`std.os` |
| `std.ffi` | C ABI 原语类型（`c_char`..`c_void`, `VaList`）、`CStr`/`CString`、`OsStr`/`OsString`、相关错误类型 | `library/std/ffi_impl/`（待建） | 依赖 `attrs.abi`、NIR/LLVM C ABI 降码 |
| `std.os` | 结构化平台抽象：`unix`、`linux`、`macos`、`windows`、`wasi` | `library/std/platform/` | 通过 `build.target` 条件选择与外部 ABI/intrinsic 接入 |
| `std.simd` | 可移植向量 `Simd<T, N>` 与掩码 `Mask<T, N>` | `library/std/simd_impl/`（待建） | 依赖 `attrs.layout.align` 与 LLVM 向量降码 |

### 1.3 五大强制架构红线

1. **运行时依赖类型 Allocator 范式（禁止硬编码 `SystemAllocator`）**：所有堆分配容器、智能指针、拥有型字符串/路径与缓冲 I/O 均必须采用与 [library/std/mem_impl/boxed.fl](../../library/std/mem_impl/boxed.fl) 一致的签名：
   ```flurry
   where
       T,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   ```
   容器本体只保存数据指针/长度/容量等字段，使用全局分配器时存储零开销；所有分配、重分配与 `Drop` 释放均通过环境交付参数 `alloc`（`env_parameters`）执行。
2. **原生 `?T` 与 `!E T`（零 `Option`/`Result` 枚举）**：可空用 `?T` / `null` / `pat?`，可错用 `!E T` / `![E1, E2] T` / `error val` / `pat!` / `error pat` / `expr! { catch pat => ... }`。
3. **错误与普通枚举命名分界**：错误类型必须 `PascalCaseErr` 且变体为 `PascalCase`（如 `IoErr.UnexpectedEof`、`FsErr.NotFound`、`VarErr.NotPresent`）；普通枚举变体必须 `snake_case`（如 `SeekFrom.start`、`Cow.borrowed`、`Component.root_dir`、`Stdio.piped`）。
4. **类型参数与语法规范**：值参数直接使用 Flurry 子句 `N: usize`（禁止虚构定长语法修饰符）；路径全用点分 `a.b.c`（禁止双冒号路径分隔符）；无 `extern` 关键字（改用 `^attrs.abi(...)`）；无 `handles` 函数签名修饰；位解释使用 `'bitvec`（无 `transmute`）；`where`/`requires` 换行时 `{` 另起对齐一行，以 `}` 结束的语句/定义不写尾随 `;`。
5. **值/指针与阶段分离**：`str`、`Slice<T>`、`CStr`、`OsStr`、`Path` 是可 `Copy` 的借用视图描述符（受可达性/text-region 证明约束）；需要就地修改的连续切片使用显式可变视图 `MutSlice<T>`（`Once`）。`comptime` 与 `runtime` 若需同名协议，分置 `pure comptime trait` 与 runtime `trait`（无 `comptime_` 前缀）。

### 1.4 六大内置包防膨胀边界与外置排除清单（Tier 1 vs Tier 2/3）

依据 `AGENTS.md` 六内置包原则与全局三层包架构边界（详参 [10-official-protocol-packages-and-boundary.md](./10-official-protocol-packages-and-boundary.md)），`std`（Tier 1）**保持精简的核心系统/内存/集合/IO 底座定位，严禁将高层协议与重型领域库塞入 `std` 或六大内置包**：

| 领域能力 | 禁止纳入 `std` / 六大内置包的内容 | 正确归属层（Tier 2 / Tier 3） | 本文档 `std` 仅提供的最小底座 |
|---|---|---|---|
| **正则表达式** | 禁止将 `regex`、DFA/NFA 引擎或 `RegexMatcher` 放入 `std` 或 `builtin` | Tier 2 `regex_protocol` + Tier 3 正则库 | 仅 `std.string.String` 与 `builtin` 通用模式视图 |
| **序列化** | 禁止 JSON / CBOR / TOML / YAML / XML 解析器与 `Serialize` trait 放入 `std` | Tier 2 `serialization` + Tier 3 `json_lib` 等 | 仅 `std.io.{Reader, Write}`、`std.fmt.Debug`/`Display` |
| **网络应用协议 / 加密** | 禁止 HTTP / TLS / WebSocket / SQL / ORM / 哈希摘要 / AEAD / HMAC | Tier 2 `http`、`wire`、`database`、`crypto`、`digest` | 仅在 `08` 提供原始 socket/FD 与最小运行时 ABI |
| **时间与历法** | 禁止公历/农历历法转换、时区名称与 `tzdata` 数据库 | Tier 2 `datetime` | `std.time` 仅限单调 `Instant`、墙上 `SystemTime` 与 `Duration` |
| **随机数** | 禁止 PRNG / CSPRNG 算法与分布采样器放入 `std` | Tier 2 `random` | 仅在 `std.os` 提供系统熵源系统调用原语（`getrandom`） |
| **压缩与多字符集** | 禁止 gzip/zstd/deflate 与非 UTF-8 文本编解码表（GBK/Shift-JIS 等） | Tier 2 `compression`、`encoding` | `std.string` 严格限定 UTF-8，`std.ffi` 仅限原生字节视图 |
| **媒体 / 遥测 / 执行器** | 禁止图像/音频/色彩空间、具体日志 exporter、具体 async 执行器、ML 张量后端 | Tier 2 `media`、`telemetry`、`async_runtime`、`tensor_protocol` | `std.simd` 仅提供便携硬件向量原语；调度底座留给 `08` |

> **边界集合容器审计标记（`std.collections` vs Tier 2 `collections_ext`）**：`Vec`、`VecDeque`、`HashMap`、`HashSet`、`BTreeMap`、`BTreeSet`、`BinaryHeap`、`BitSet` 与 `Slice`/`MutSlice` 属于核心标准集合；而 **`SoA`、`SlotMap`、`SmallVec`、`RingBuffer` 属于高扩展性/特定数据导向设计（DOD）或静态栈内分配的边界容器**。本文按完整需求规格定义其依赖类型与内存接口，同时在表 2 及 **§5 Q11** 显式标记其 `std.collections` vs Tier 2 `collections_ext` 归属待作者最终划界。

---

## 2. 现状与缺口全景总表

> 说明：状态标注依据当前仓库 `library/std/`、`compiler/` 与 `issues/` 实测状态。

| ID | 模块与主题 | 现状（已落地） | 核心缺口（Wishlist 目标） | 关联编译器 / 上游阻塞 | 优先级 |
|---|---|---|---|---|---|
| `WL-07-01` | `std.mem` 基础设施与 `Allocator` 协议 | `AllocErr`、`ReallocErr`、`RawBlock`、`Layout`、`Allocator`、`SystemAllocator`、`DefaultAllocator`、`FixedBufferAllocator`、`ArenaAllocator` 已落地 | 补充 `Layout`/`RawBlock` 辅助方法与 `Display`/`Eq`/`Hash`；为 `Allocator` 增加默认 `grow`/`shrink` 与 `OwningAllocator` 归属探测协议；`SystemAllocator.reallocate` 接入直调 `realloc` 快速路径 | `std.os` 多平台分配后端；`Layout.of<T>()` 的 `comptime` 支持 | **P0** |
| `WL-07-02` | `std.mem` 高级分配器生态 | 仅 `SystemAllocator`、`FixedBufferAllocator`、`ArenaAllocator` | 新增 `BumpAllocator`、`SlabAllocator`、`PoolAllocator<T>`、`ScratchAllocator`（帧作用域）、`TrackingAllocator`（泄漏统计）、`FailoverAllocator`；为 `FixedBufferAllocator`/`ArenaAllocator` 补 `Checkpoint`/`rewind` | 无硬阻断；`ScratchAllocator` 与 Silver Layer 4 `FrameArena` 协同 | **P1** |
| `WL-07-03` | `std.mem` 智能指针家族 | 仅 `Box<T, .A, .allocator>`（基础操作、`Drop`、`Debug`） | 补全 `Box`（`from_raw`/`into_raw`/`leak`/`try_clone`/`Eq`/`Ord`/`Hash`/`Display`/`Deref`）；新增 `Rc<T, .A, .allocator>`、`RcWeak`、`Arc<T, .A, .allocator>`、`ArcWeak`、`Cow<B, .A, .allocator>`（含 `ToOwned`）、`Unique<T>` | `Arc` 阻塞于 `std.sync.atomic` 与 `Send`/`Sync`（Wave 2）；`Box`/`Rc`/`Arc` 解引用人体工学依赖 `Deref`/`DerefMut`（`01`）；泛型闭包内 env `Drop` 见 `issues/0001` | **P0** (`Unique`/`Box`/`Rc`/`Cow`) / **P1** (`Arc`) |
| `WL-07-04` | `std.mem` 内存工具与未初始化类型 | 仅 `builtin.mem` 底层 `raw_read`/`raw_write`/字节操作与 `align_up_usize` | 与 `04`（`WL-04-26/27`）对齐：**由 `builtin.mem` 作为权威原语归属并由 `std.mem` 重导出**：`unsafe swap`、`unsafe replace`、`take`、`forget`、`drop`、`unsafe zeroed`、`MaybeUninit`、`ManuallyDrop`、`align_up`/`align_down`/`is_aligned`/`align_offset`、`size_of`/`align_of`/`needs_drop` | `ManuallyDrop`/`MaybeUninit` 依赖 `program_verification/drop` 确认 `union` 免除自动 `Drop`；`needs_drop` 需 `meta`/`Type.predicates` 暴露非平凡 `Drop` 查询 | **P0** |
| `WL-07-05` | `std.collections` 序列与小容器 | `Vec<T, .A, .allocator>`、`VecDeque<T, .A, .allocator>` 基础方法及消费迭代器已落地 | `Vec` 补齐 `retain`/`dedup*`/`drain`/`splice`/`split_off`/`resize*`/`extend*`/`iter_ref`/`Index`/`IndexMut`/`FromIterator`；`VecDeque` 补齐 `make_contiguous`/`as_slices`/`rotate_*`/`insert`/`remove`/`drain`/`retain`；规划 `SmallVec<T, N: usize, .A, .allocator>`（**边界标记：待 Q11 裁定归属 `std.collections` 或 `collections_ext`**） | `SmallVec` 硬依赖 `builtin.Array<T, N>` 与 `MaybeUninit<T>`；`drain`/`splice` 范围接口依赖 `RangeBounds`（`01`）；`retain`/`sort_by` 等闭包跨泛型 env 见 `issues/0001` | **P0** (`Vec`/`VecDeque`) / **P1** (`SmallVec`) |
| `WL-07-06` | `std.collections` 散列映射与集合 | `HashMap<K, V, .A, .allocator>` 与 `HashSet<T, .A, .allocator>` 已落地（SplitMix64 + SOA 开放寻址） | 增加 `.H: Type = DefaultBuildHasher` 与 `.hasher` 参数；补齐完整 `Entry`（`OccupiedEntry`/`VacantEntry`）；消除 `find_slot` 不必要的 `K :- Clone` 约束；补齐 `get_ref`/`keys`/`values`/`iter_ref`/`drain`/`retain` 与 `HashSet` 集合代数 | `Hasher`/`BuildHasher` 协议接入（`01`/`02`）；无 `Clone` 查找需健全的借用哈希与指针 `Eq` 契约 | **P0** |
| `WL-07-07` | `std.collections` 有序树、堆、位集与环形缓冲 | 无 | 新增 `BTreeMap<K, V, .A, .allocator>`（含 `Entry` 与 `range`）、`BTreeSet<T, .A, .allocator>`、`BinaryHeap<T, .A, .allocator>`（含 `PeekMut`）、`BitSet<.A, .allocator>`，以及 `RingBuffer<T, N: usize>`（**边界标记：待 Q11 裁定归属 `std.collections` 或 `collections_ext`**） | `RingBuffer` 依赖 `builtin.Array<T, N>` + `MaybeUninit<T>`；`BTree*` 区间查询依赖 `RangeBounds`（`01`） | **P1** |
| `WL-07-08` | `std.collections` 运行时依赖 `Arena` / `SlotMap` / `SoA`（Silver Layer 4） | 无（仅 `docs/type/runtime-dependent-type.md` 与 `proposal_trace/2026-8-5-typed-Soa.fl` 意图原型） | 新增 `Arena<T, .A, .allocator>` + `SlotMap<V, .A, .allocator>` + `NodeIndex` 4B/8B 轻量句柄；新增基于反射生成的类型化 `SoA<T, .A, .allocator>`（**边界标记：待 Q11 裁定保留在 `std.collections` 抑或部分迁移至 Tier 2 `collections_ext`**） | `tmp/silver-design/04` **P1-4**、`StructBuilder` 生成带 `quote` 子句参数类型、`meta.reflection.reflect_struct`（`05` `WL-05-12`），以及 **`__flurry_align`（`^attrs.layout.align`，待在 `official-attribute-keys.md` 登记，`WL-05-16`）** | **P1**（Silver 核心） |
| `WL-07-09` | `std.collections` `Slice<T>`、`MutSlice<T>` 与切片算法 | 仅 `Slice<T>`（`len`/`slice`/`subslice_from`/`split_at`/`get`/`first`/`last`/`ListPatternView`/`Eq`/`Debug`） | 新增 `MutSlice<T>` 可变切片视图；补齐 `binary_search*`、`sort*`、`select_nth_unstable*`、`partition_point`、`chunks*`、`rchunks*`、`windows`、`split*`、`rotate_left`/`rotate_right`、`reverse`、`copy_from_slice`/`clone_from_slice` | `MutSlice<T>` 需要独占访问/别名分离证明契约（`docs/resource-management/pointers.md`）；`sort_by` 闭包 env 交付（`issues/0001`） | **P0** |
| `WL-07-10` | `std.string` 拥有型 UTF-8 字符串 | `String<.A, .allocator>`、`Utf8Err` 已落地（RFC 3629 校验、基本修改与查询、`Display`/`Debug`/`Eq`/`Ord`/`Hash`） | 新增 `FromUtf8Err<.A, .allocator>` 保留原 `Vec<u8>`；补齐 `as_str`、`as_bytes`、`insert*`、`remove`、`retain`、`drain`、`replace_range`、`split_off`、`lines`/`chars`/`char_indices`/`trim*`/`split*`/`replace` | `as_str` 硬阻塞于 `str` backing-region 资源证明（`issues/0003`, `native-text-method-provenance.fl`） | **P0** |
| `WL-07-11` | `std.fmt` 编译期检查格式化 | `FormatPlan`/`FormatErr`/`Parse`/`format`/`format_into`/`format_debug`/`write_fmt`/`println`/`eprintln`、`{key: value}` 选项与 IEEE 浮点 `Display` 已落地 | 补齐 `print`/`eprint` 变参格式化；对接 `04`（`WL-04-21`）权威规划：将 `FormatErr`/`Formatter`/`DebugStruct..DebugSet` 归入 `builtin.fmt` 并由 `std.fmt` 重导出，`ComptimeDisplay`/`ComptimeDebug`（同名 `fmt`）接入 `comptime_format` 及 comptime 浮点 `Display`；补齐标准错误类型 `Display` | `04` `WL-04-21`、`pure comptime trait meta.fmt.ComptimeDebug`；`Debug.write_debug` vs `Debug.fmt` 演进见 `04` 与 §3.3.3 | **P0**（`print` 变参） / **P1**（Builder/ComptimeDebug） |
| `WL-07-12` | `std.io` 协议、缓冲与组合子 | `Reader`/`Write`/`ByteWriter`/`BufReader`/`BufWriter`/`Cursor`/`Stdin`/`Writer`/`IoErr` 已落地 | 扩充 `IoErr` + `IoErrKind`；新增 `BufRead`、`Seek`、`SeekFrom`、`LineWriter`；泛化 `Cursor<T>` 支持 `Slice<u8>` 与 `Vec<u8>`；新增 `copy`/`copy_buf`/`Take`/`Chain`/`Empty`/`Sink`/`Repeat`、`IoSlice*`、安全切片 `read`/`write` 方法 | 多平台 `IoErr` errno 映射；`StdoutLock`/`StderrLock` 依赖 Wave 2 `std.sync` | **P0** |
| `WL-07-13` | `std.path` 跨平台路径 | 无 | 新增 `Path`、`PathBuf<.A, .allocator>`、`Component`、`Prefix`、`PathErr` 与 POSIX/Windows 解析逻辑 | 依赖 `std.ffi.OsStr` / `OsString`（`WL-07-16`） | **P1** |
| `WL-07-14` | `std.fs` 文件系统 | 仅 `File`（comptime `str` 路径 `open`/`create`/`write_all`、`read_byte`/`write_bytes`/`write_string`/`close`/`Drop`）与 comptime 路径辅助函数 | 支持运行时 `Path`/`str` 路径；新增 `FsErr`、`OpenOptions`、`Metadata`、`FileType`、`Permissions`、`DirEntry`、`ReadDir`、`Seek for File`、`try_clone`/`sync_all`/`set_len`/`create_dir*`/`remove_dir*`/`rename`/`copy`/`canonicalize`/`symlink*` | `std.os` 动态路径、stat、目录遍历与权限系统调用原语 | **P1** |
| `WL-07-15` | `std.time` 时钟与时长 | 仅 `Duration`（纳秒归一化、整数转换与 checked/saturating 算术、`Splice`/`Eq`/`Ord`/`Hash`/`Debug`） | 补全 `Duration` 浮点/运算符/`Display`；新增 `Instant`、`SystemTime`、`TimeErr`、`sleep` | `std.os` 单调/墙上时钟与睡眠原语（`clock_gettime`/`nanosleep` 等） | **P1** |
| `WL-07-16` | `std.env` 运行时环境与构建常量 | 无 | 新增 `args`/`args_os`、`var`/`var_os`/`vars`/`vars_os`、`unsafe set_var`/`remove_var`、`current_dir`/`set_current_dir`、`current_exe`、`temp_dir`、`VarErr`、`std.env.consts` | 主入口 `argc`/`argv`/`envp` 运行时捕获或 OS 读接口；`std.env.consts` 依赖 `build.target` 完整目标枚举 | **P1** |
| `WL-07-17` | `std.process` 子进程与终止契约 | 仅 `id() -> u32` 与 `ExitCode` | 新增 `Command`、`Child`、`ChildStdin`/`ChildStdout`/`ChildStderr`、`ExitStatus`、`Stdio`、`Output`、`ProcessErr`、`exit`、`abort`、`panic`（含调用点位置诊断能力提案） | `std.os` 进程派生/管道/等待原语；**`panic` 硬阻塞于终止/清理/捕获契约（§3.5.3, Q1），调用位置另需 `__flurry_caller_location` 属性与 `meta.ast.SourceSpan` 支持** | **P0**（`panic`/`exit`/`abort`） / **P1**（`Command`/`Child`） |
| `WL-07-18` | `std.ffi` C ABI 与系统字符串 | 仅 `^attrs.abi(.c)` 支持闭合标量/指针/void 运行时导出（`attributes.rs`, `control.rs`）；`docs/others/ffi.md` 为空 | 新增 C 原语类型表（`c_char`..`c_void`, `VaList`）、`CStr`/`CString`/`NulErr`/`IntoStringErr`、`OsStr`/`OsString`、C 导入声明（无 `extern`）、`.system`/`.win64`/`.sysv64` 约定、C struct/union 聚合传递、C fn 指针、`LiteralInto<.cstr, CStr>` | 扩展 `attrs.abi`、在 `official-attribute-keys.md` 登记 `__flurry_ffi_link`/`__flurry_ffi_link_name`、NIR/LLVM 外部 C 符号 `declare`、SysV/Win64/AAPCS 聚合 ABI 分类 | **P0** |
| `WL-07-19` | `std.os` / `std.platform` 多平台系统接口 | 仅 `x86_64` Linux 11 个 `platform.ll` intrinsic 与 `std.os.unix` 包装 | 结构化拆分 `unix`、`linux`、`macos`、`windows`、`wasi` 模块；通过 `build.target` 条件编译组织；补齐文件系统/时钟/进程/环境变量/动态库与错误映射；规划从硬编码 `platform.ll` 迁向通用 `^attrs.abi` C/syscall 绑定 | 编译期 `build.target` 条件编译与 `std.ffi` 外部符号接入（Q8） | **P1** |
| `WL-07-20` | `std.simd` 可移植向量 | 无 | 新增 `Simd<T, N: usize>`、`Mask<T, N: usize>`、`SimdElement`/`MaskElement`、构造/load/store/gather/scatter/swizzle/shuffle/归约/bitmask 与安全/对齐契约 | `tmp/silver-design/04` **P1-5**：在 `official-attribute-keys.md` 登记 `__flurry_align`（`^attrs.layout.align`，`05` `WL-05-16`）与 NIR/LLVM 向量类型 `<N x T>` 及目标 SIMD intrinsic 降码 | **P1**（Silver Layer 4 加速） |

---

## 3. 详细规格设计

### 3.1 `std.mem`：分配器生态、智能指针与内存工具

#### 3.1.1 布局、错误与 `Allocator` 核心协议

现有 [allocator.fl](../../library/std/mem_impl/allocator.fl) 与 [layout.fl](../../library/std/mem_impl/layout.fl) 已确立正确基线。Wishlist 要求保留已有签名并补齐以下通用契约与默认方法：

```flurry
-- std.mem 统一错误与块模型（继承并扩展现有定义）
pub enum AllocErr {
    OutOfMemory,
    InvalidLayout,
    Unsupported,
}

pub enum ReallocErr {
    OutOfMemory,
    InvalidLayout,
    Unsupported,
}

pub struct RawBlock {
    raw_addr: usize,
    raw_layout: Layout,
}

pub trait Allocator {
    fn allocate(*self, layout: Layout) -> !AllocErr RawBlock;
    fn allocate_zeroed(*self, layout: Layout) -> !AllocErr RawBlock;
    unsafe fn deallocate(*self, block: RawBlock);
    unsafe fn reallocate(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock;

    -- 语义精确的增长与收缩默认方法（避免调用方重复编写拷贝与释放样板）
    unsafe fn grow(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock
    where
        requires new_layout.byte_size() >= block.byte_size(),
    {
        self.reallocate(block, new_layout)
    }

    unsafe fn grow_zeroed(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock
    where
        requires new_layout.byte_size() >= block.byte_size(),
    {
        let old_size = block.byte_size_usize() match {
            v? => v,
            null => return error ReallocErr.InvalidLayout,
        }
        let new_size = new_layout.byte_size_usize() match {
            v? => v,
            null => return error ReallocErr.InvalidLayout,
        }
        let grown = self.grow(block, new_layout)!;
        if new_size > old_size {
            let tail_addr = builtin.mem.checked_add_usize(grown.address(), old_size) match {
                addr? => addr,
                null => return error ReallocErr.InvalidLayout,
            }
            builtin.mem.set_bytes(tail_addr, 0, new_size - old_size);
        }
        grown
    }

    unsafe fn shrink(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock
    where
        requires new_layout.byte_size() <= block.byte_size(),
    {
        self.reallocate(block, new_layout)
    }
}

-- 可选归属判定能力（供 FailoverAllocator 与调试断言使用）
pub trait OwningAllocator :- Allocator {
    fn owns(*self, block: RawBlock) -> bool;
}
```

补充缺口：
- `Layout` 需补全 `Eq`、`Hash`、`Display`（如 `"Layout(size=16, align=8)"`）、`dangling() -> Self`、`align_to(self, align: u64) -> ?Self`、`extend(self, next: Self) -> ?Tuple<Self, u64>`、`extend_packed(self, next: Self) -> ?Tuple<Self, u64>`，并将 `of<T>()` 与 `array<T>(count)` 接入 `comptime` 常量求值。
- `RawBlock` 需补全 `Eq`、`Debug`、`split_at(self, offset: usize) -> ?Tuple<RawBlock, RawBlock>`、`as_mut_slice(self) -> MutSlice<u8>`。
- `AllocErr` 与 `ReallocErr` 需补全 `Display` 实现（当前仅实现 `Debug`）。

#### 3.1.2 全量分配器生态

| 分配器类型 | 子句签名（运行时依赖范式） | 存储布局与核心操作 | 契约与目标用途 |
|---|---|---|---|
| `SystemAllocator` / `DefaultAllocator` | 无额外参数；静态全局只读单例 `GLOBAL: SystemAllocator` | 零大小字段（ZST）；委托 OS 堆（`malloc`/`posix_memalign`/`realloc`/`free` 或系统页分配） | 现有 `reallocate` 每次强制分配+拷贝+释放，须补充对齐兼容时的系统 `realloc` 原地扩展路径；`GLOBAL.ref` 满足静态全局生存期 |
| `FixedBufferAllocator` | 无 backing；通过 `from_raw_block(block: RawBlock)` 或 `from_slice(buf: MutSlice<u8>)` 构造 | `{ base_addr, cap_bytes, offset, live_count }` | 已实现；补充 `Checkpoint`（`checkpoint(*self) -> usize` / `rewind(*self, mark: usize)`）与 `OwningAllocator`（`base <= addr and addr + size <= base + cap`） |
| `ArenaAllocator` | `where .Backing: Type = DefaultAllocator, requires Backing :- Allocator, quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref` | 侵入式链表 chunk（`ArenaChunkHeader`）；`new`、`with_chunk_size`、`total_allocated`、`chunk_count`、`reset` | 补充 `reset_retaining_capacity(*self)`（保留首个大 chunk 复用于每帧清空）、`checkpoint(*self) -> ArenaCheckpoint`、`rewind(*self, cp)` 与 `scope(lambda f: %c fn(*ArenaAllocator<...>) -> R)` |
| `BumpAllocator` | `where .Backing: Type = DefaultAllocator, requires Backing :- Allocator, quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref` | 极简向下/向上指针递增 chunk（无 `tail_reclaim` 检查，适合极热小对象批次）；支持 `alloc_layout_fast` 与 `iter_chunks` | `deallocate` 恒为 no-op（除非最后一次 chunk 重置）；`reallocate` 仅当 `new_size <= old_size` 原地缩容，否则重分配拷贝；`Drop` 归还所有 chunk 到 `backing` |
| `SlabAllocator` | `where .Backing: Type = DefaultAllocator, requires Backing :- Allocator, quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref` | 定长大小类（或固定 `slot_layout: Layout`）与侵入式单向空闲链表 `free_head: usize`；按 slab 页向 `backing` 批申请 | `allocate` 在 `layout` 兼容 `slot_layout` 时 O(1) 弹链表，超规返回 `AllocErr.InvalidLayout`（或配合 `FailoverAllocator` 回退）；`deallocate` O(1) 压回空闲链表；`Drop` 释放所有 slab 到 `backing` |
| `PoolAllocator` | `where T, .Backing: Type = DefaultAllocator, requires Backing :- Allocator, quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref` | 基于 `SlabAllocator` 专化单个元素类型 `T` 的对齐/步长与对象数量预设 `with_capacity_per_slab(count: usize)` | 既实现 `Allocator`（支持作为 `Box<T, .A = PoolAllocator<T>, .allocator = pool.ref>` 的分配器！），又提供类型化快速路径 `acquire(*self, value: T) -> !AllocErr Box<T, ...>` |
| `ScratchAllocator` | `where .Backing: Type = DefaultAllocator, requires Backing :- Allocator, quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref` | 持有一组可切换/嵌套水位的 `ArenaAllocator` 帧栈（`depth: usize`, `high_water: usize`） | 专为渲染/UI 帧设计：`begin_frame(*self) -> ScratchGuard` / `end_frame(*self, mark)` / `with_scratch(lambda f)`，退帧即 O(1) 回收全部临时分配 |
| `TrackingAllocator` | `where .Inner: Type = DefaultAllocator, requires Inner :- Allocator, quote .inner: lift *Inner = DefaultAllocator.GLOBAL.ref` | `{ allocated_bytes: usize, freed_bytes: usize, live_bytes: usize, peak_bytes: usize, alloc_count: usize, dealloc_count: usize, realloc_count: usize }` | 零侵入包装任意 `Inner :- Allocator`；每次操作经 `quote inner` 转发并原子/顺序更新统计；提供 `snapshot(*self)`、`reset_peak(*self)`、`has_leaks(*self) -> bool` |
| `FailoverAllocator` | `where .Primary: Type, .Fallback: Type = DefaultAllocator, requires Primary :- OwningAllocator, requires Fallback :- Allocator, quote .primary: lift *Primary, quote .fallback: lift *Fallback = DefaultAllocator.GLOBAL.ref` | 零或轻量状态结构体；`allocate` 先试 `primary`，仅在 `AllocErr.OutOfMemory` 时回退 `fallback` | `deallocate` 与 `reallocate` 必须通过 `primary.as(OwningAllocator).owns(block)` 路由回原分配器（不可把 `Fallback` 块错误释放给 `Primary`） |

新增分配器完整类型头与扩展契约：

```flurry
pub struct BumpAllocator
where
    .Backing: Type = DefaultAllocator,
    requires Backing :- Allocator,
    quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref,
{
    head_chunk: usize,
    cursor: usize,
    end: usize,
    chunk_bytes: usize,
    total_bytes: usize,
}

pub struct SlabAllocator
where
    .Backing: Type = DefaultAllocator,
    requires Backing :- Allocator,
    quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref,
{
    slot_layout: Layout,
    slots_per_slab: usize,
    slab_head: usize,
    free_head: usize,
    live_slots: usize,
}

pub struct PoolAllocator
where
    T,
    .Backing: Type = DefaultAllocator,
    requires Backing :- Allocator,
    quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref,
{
    slab: SlabAllocator<.Backing = Backing, .backing = backing>,
}

pub struct ScratchAllocator
where
    .Backing: Type = DefaultAllocator,
    requires Backing :- Allocator,
    quote .backing: lift *Backing = DefaultAllocator.GLOBAL.ref,
{
    arena: ArenaAllocator<.Backing = Backing, .backing = backing>,
    depth: usize,
    high_water: usize,
}

pub struct AllocStats {
    allocated_bytes: usize,
    freed_bytes: usize,
    live_bytes: usize,
    peak_bytes: usize,
    alloc_count: usize,
    dealloc_count: usize,
    realloc_count: usize,
}

pub struct TrackingAllocator
where
    .Inner: Type = DefaultAllocator,
    requires Inner :- Allocator,
    quote .inner: lift *Inner = DefaultAllocator.GLOBAL.ref,
{
    allocated_bytes: usize,
    freed_bytes: usize,
    live_bytes: usize,
    peak_bytes: usize,
    alloc_count: usize,
    dealloc_count: usize,
    realloc_count: usize,
}

pub struct FailoverAllocator
where
    .Primary: Type,
    .Fallback: Type = DefaultAllocator,
    requires Primary :- OwningAllocator,
    requires Fallback :- Allocator,
    quote .primary: lift *Primary,
    quote .fallback: lift *Fallback = DefaultAllocator.GLOBAL.ref,
{
    fallback_count: usize,
}
```

#### 3.1.3 智能指针与运行时依赖环境交付

所有分配型智能指针严格遵守：容器本体仅存受管理的地址/元数据，**绝不在结构体中硬编码全局 `SystemAllocator`，也不额外保存 `*A` 指针**（由类型参数 `quote .allocator` 对应的编译期环境交付计划自动传递到构造函数、`Clone`、`make_mut` 与 `Drop`）：

```flurry
-- 1. Unique<T>: 非空、仿射（非 Copy）裸所有权指针构建块，自身不携带 Drop 或 Allocator
pub struct Unique where T {
    raw_addr: usize,
}

impl Unique<T> where T {
    pub pure fn dangling() -> Self {
        Unique<T> { raw_addr: Type.align_of(T) }
    }

    pub unsafe fn new_unchecked(addr: usize) -> Self
    where
        requires addr != 0,
    {
        Unique<T> { raw_addr: addr }
    }

    pub fn new(ptr: *T) -> ?Self {
        let addr = ptr'bitvec.as(usize);
        if addr == 0 {
            null
        } else {
            Unique<T> { raw_addr: addr }
        }
    }

    pub pure fn as_addr(self) -> usize {
        self.raw_addr
    }

    pub unsafe fn as_ptr(*self) -> *T {
        self.raw_addr.*'bitvec.as(*T)
    }

    pub fn cast(self) -> Unique<U> where U {
        Unique<U> { raw_addr: self.raw_addr }
    }
}

-- 2. Rc<T, .A, .allocator> 与 RcWeak<T, .A, .allocator>: 单线程引用计数指针
struct RcInner where T {
    strong: usize,
    weak: usize,
    value: MaybeUninit<T>,
}

pub struct Rc
where
    T,
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    raw_addr: usize,
}

pub struct RcWeak
where
    T,
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    raw_addr: usize,
}

struct RcOffsets {
    strong: usize,
    weak: usize,
    value: usize,
}

pure fn rc_offsets() -> RcOffsets where T {
    let strong_off = Type.field_offset_of(RcInner<T>, .strong) match {
        v! => v,
        error _ => std.process.abort(),
    }
    let weak_off = Type.field_offset_of(RcInner<T>, .weak) match {
        v! => v,
        error _ => std.process.abort(),
    }
    let val_off = Type.field_offset_of(RcInner<T>, .value) match {
        v! => v,
        error _ => std.process.abort(),
    }
    RcOffsets { strong: strong_off, weak: weak_off, value: val_off }
}

pure fn rc_field_addr(base: usize, off: usize) -> usize {
    builtin.mem.checked_add_usize(base, off) match {
        addr? => addr,
        null => std.process.abort(),
    }
}

impl Clone for Rc<T, .A = A, .allocator = alloc>
where
    T,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    fn clone(*self) -> Self {
        let addr = self.raw_addr.*;
        let offs = rc_offsets<T>();
        let strong_addr = rc_field_addr(addr, offs.strong);
        let cur = unsafe { builtin.mem.raw_read<usize>(strong_addr) }
        -- 强计数溢出检查：运行时分支显式中止，绝不回绕导致 UAF
        if cur == builtin.mem.usize_max() {
            std.process.abort();
        }
        unsafe { builtin.mem.raw_write<usize>(strong_addr, cur + 1); }
        Rc<T, .A = A, .allocator = alloc> { raw_addr: addr }
    }
}

impl Drop for Rc<T, .A = A, .allocator = alloc>
where
    T,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    fn drop(*self) {
        let addr = self.raw_addr.*;
        if addr == 0 {
            return;
        }
        self.raw_addr.* = 0;
        let offs = rc_offsets<T>();
        let strong_addr = rc_field_addr(addr, offs.strong);
        let next_strong = unsafe { builtin.mem.raw_read<usize>(strong_addr) } - 1;
        unsafe { builtin.mem.raw_write<usize>(strong_addr, next_strong); }
        if next_strong == 0 {
            -- 所有强引用归零：先就地析构载荷 T，再释放隐含的一份弱引用
            let val_addr = rc_field_addr(addr, offs.value);
            unsafe {
                let val_ptr = val_addr'bitvec.as(*MaybeUninit<T>);
                val_ptr.assume_init_drop();
            }
            let weak_addr = rc_field_addr(addr, offs.weak);
            let next_weak = unsafe { builtin.mem.raw_read<usize>(weak_addr) } - 1;
            unsafe { builtin.mem.raw_write<usize>(weak_addr, next_weak); }
            if next_weak == 0 {
                let layout = Layout.of<RcInner<T>>() match {
                    l? => l,
                    null => return,
                }
                unsafe {
                    alloc.as(Allocator).deallocate(RawBlock.from_raw_parts(addr, layout));
                }
            }
        }
    }
}

impl Rc<T, .A = A, .allocator = alloc>
where
    T,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    pub fn new(value: T) -> !AllocErr Self {
        let layout = Layout.of<RcInner<T>>() match {
            l? => l,
            null => return error AllocErr.InvalidLayout,
        }
        let block = alloc.as(Allocator).allocate(layout)!;
        let addr = block.address();
        let inner = RcInner<T> {
            strong: 1,
            weak: 1,
            value: MaybeUninit<T>.new(value),
        }
        unsafe { builtin.mem.raw_write<RcInner<T>>(addr, inner); }
        Rc<T, .A = A, .allocator = alloc> { raw_addr: addr }
    }

    pub fn strong_count(*self) -> usize {
        let offs = rc_offsets<T>();
        unsafe { builtin.mem.raw_read<usize>(rc_field_addr(self.raw_addr.*, offs.strong)) }
    }

    pub fn weak_count(*self) -> usize {
        let offs = rc_offsets<T>();
        let raw_weak = unsafe { builtin.mem.raw_read<usize>(rc_field_addr(self.raw_addr.*, offs.weak)) }
        if raw_weak == 0 { 0 } else { raw_weak - 1 }
    }

    pub fn downgrade(*self) -> RcWeak<T, .A = A, .allocator = alloc> {
        let addr = self.raw_addr.*;
        let offs = rc_offsets<T>();
        let weak_addr = rc_field_addr(addr, offs.weak);
        let cur = unsafe { builtin.mem.raw_read<usize>(weak_addr) }
        if cur == builtin.mem.usize_max() {
            std.process.abort();
        }
        unsafe { builtin.mem.raw_write<usize>(weak_addr, cur + 1); }
        RcWeak<T, .A = A, .allocator = alloc> { raw_addr: addr }
    }

    pub fn ptr_eq(this: *Self, other: *Self) -> bool {
        this.raw_addr.* == other.raw_addr.*
    }

    pub unsafe fn get_mut(*self) -> ?*T {
        if self.strong_count() == 1 and self.weak_count() == 0 {
            let offs = rc_offsets<T>();
            rc_field_addr(self.raw_addr.*, offs.value)'bitvec.as(*T)
        } else {
            null
        }
    }

    pub fn try_unwrap(self) -> !Self T {
        if self.strong_count() != 1 {
            return error self;
        }
        let addr = self.raw_addr;
        forget(self);
        let offs = rc_offsets<T>();
        let value = unsafe { builtin.mem.raw_read<T>(rc_field_addr(addr, offs.value)) }
        let strong_addr = rc_field_addr(addr, offs.strong);
        let weak_addr = rc_field_addr(addr, offs.weak);
        unsafe { builtin.mem.raw_write<usize>(strong_addr, 0); }
        let next_weak = unsafe { builtin.mem.raw_read<usize>(weak_addr) } - 1;
        unsafe { builtin.mem.raw_write<usize>(weak_addr, next_weak); }
        if next_weak == 0 {
            let layout = Layout.of<RcInner<T>>() match {
                l? => l,
                null => return value,
            }
            unsafe { alloc.as(Allocator).deallocate(RawBlock.from_raw_parts(addr, layout)); }
        }
        value
    }

    pub unsafe fn as_ptr(*self) -> *T {
        let offs = rc_offsets<T>();
        rc_field_addr(self.raw_addr.*, offs.value)'bitvec.as(*T)
    }
}

impl Rc<T, .A = A, .allocator = alloc>
where
    T :- Clone,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    pub fn make_mut(*self) -> !AllocErr *T {
        if self.strong_count() != 1 {
            let cloned = unsafe { self.as_ptr().as(Clone).clone() }
            let replacement = Rc<T, .A = A, .allocator = alloc>.new(cloned)!;
            drop(replace(self, replacement));
        } else if self.weak_count() > 0 {
            -- 异常安全次序：先分配新控制块，分配成功后才从旧控制块移出 T，杜绝 OOM 时双析构
            let layout = Layout.of<RcInner<T>>() match {
                l? => l,
                null => return error AllocErr.InvalidLayout,
            }
            let block = alloc.as(Allocator).allocate(layout)!;
            let new_addr = block.address();
            let old_addr = self.raw_addr.*;
            let offs = rc_offsets<T>();
            let moved = unsafe { builtin.mem.raw_read<T>(rc_field_addr(old_addr, offs.value)) }
            let new_inner = RcInner<T> {
                strong: 1,
                weak: 1,
                value: MaybeUninit<T>.new(moved),
            }
            unsafe { builtin.mem.raw_write<RcInner<T>>(new_addr, new_inner); }
            let old_strong = rc_field_addr(old_addr, offs.strong);
            let old_weak = rc_field_addr(old_addr, offs.weak);
            unsafe {
                builtin.mem.raw_write<usize>(old_strong, 0);
                let cur_weak = builtin.mem.raw_read<usize>(old_weak);
                builtin.mem.raw_write<usize>(old_weak, cur_weak - 1);
            }
            let replacement = Rc<T, .A = A, .allocator = alloc> { raw_addr: new_addr }
            forget(replace(self, replacement));
        }
        unsafe { self.as_ptr() }
    }
}

impl Clone for RcWeak<T, .A = A, .allocator = alloc>
where
    T,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    fn clone(*self) -> Self {
        let addr = self.raw_addr.*;
        if addr != 0 {
            let offs = rc_offsets<T>();
            let weak_addr = rc_field_addr(addr, offs.weak);
            let cur = unsafe { builtin.mem.raw_read<usize>(weak_addr) }
            if cur == builtin.mem.usize_max() {
                std.process.abort();
            }
            unsafe { builtin.mem.raw_write<usize>(weak_addr, cur + 1); }
        }
        RcWeak<T, .A = A, .allocator = alloc> { raw_addr: addr }
    }
}

impl Drop for RcWeak<T, .A = A, .allocator = alloc>
where
    T,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    fn drop(*self) {
        let addr = self.raw_addr.*;
        if addr == 0 {
            return;
        }
        self.raw_addr.* = 0;
        let offs = rc_offsets<T>();
        let weak_addr = rc_field_addr(addr, offs.weak);
        let next_weak = unsafe { builtin.mem.raw_read<usize>(weak_addr) } - 1;
        unsafe { builtin.mem.raw_write<usize>(weak_addr, next_weak); }
        if next_weak == 0 {
            let layout = Layout.of<RcInner<T>>() match {
                l? => l,
                null => return,
            }
            unsafe { alloc.as(Allocator).deallocate(RawBlock.from_raw_parts(addr, layout)); }
        }
    }
}

impl RcWeak<T, .A = A, .allocator = alloc>
where
    T,
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    pub pure fn dangling() -> Self {
        RcWeak<T, .A = A, .allocator = alloc> { raw_addr: 0 }
    }

    pub fn upgrade(*self) -> ?Rc<T, .A = A, .allocator = alloc> {
        let addr = self.raw_addr.*;
        if addr == 0 {
            return null;
        }
        let offs = rc_offsets<T>();
        let strong_addr = rc_field_addr(addr, offs.strong);
        let cur = unsafe { builtin.mem.raw_read<usize>(strong_addr) }
        if cur == 0 {
            return null;
        }
        if cur == builtin.mem.usize_max() {
            std.process.abort();
        }
        unsafe { builtin.mem.raw_write<usize>(strong_addr, cur + 1); }
        Rc<T, .A = A, .allocator = alloc> { raw_addr: addr }
    }

    pub fn strong_count(*self) -> usize {
        let addr = self.raw_addr.*;
        if addr == 0 {
            0
        } else {
            let offs = rc_offsets<T>();
            unsafe { builtin.mem.raw_read<usize>(rc_field_addr(addr, offs.strong)) }
        }
    }

    pub fn weak_count(*self) -> usize {
        let addr = self.raw_addr.*;
        if addr == 0 {
            0
        } else if self.strong_count() == 0 {
            0
        } else {
            let offs = rc_offsets<T>();
            unsafe { builtin.mem.raw_read<usize>(rc_field_addr(addr, offs.weak)) } - 1
        }
    }

    pub fn ptr_eq(this: *Self, other: *Self) -> bool {
        this.raw_addr.* == other.raw_addr.*
    }
}
```

> **说明（`RcInner.value` 与悬空强弱计数不变式）**：控制块内将载荷声明为 `MaybeUninit<T>` 并通过 `Type.field_offset_of` 单独更新计数字段，可彻底避免在 `strong == 0` 析构之后误将已消费槽位重新写回内存；`Rc.try_unwrap` 当 `strong == 1` 且 `weak > 0` 时同样保留控制块至最后一个 `RcWeak` 释放，仅将 `strong` 归零并递减隐含弱计数。

- **`Arc<T, .A, .allocator>` 与 `ArcWeak<T, .A, .allocator>`**：
  - 结构与 `Rc` 对称，但头部的 `strong: AtomicUsize` 与 `weak: AtomicUsize` 使用原子操作（`Ordering.relaxed` 增加引用、`Ordering.release` 减少计数 + `fence(Ordering.acquire)` 后析构），要求 `T :- Send + Sync`（允许在多线程间共享）以及 `A :- Allocator + Sync`（或静态全局 `DefaultAllocator.GLOBAL.ref`）。
  - 命名消歧：`std.mem` 同时包含单线程与原子引用计数时，建议主模块分别暴露 `std.mem.rc.{Rc, Weak}` 与 `std.mem.arc.{Arc, Weak}`，并在 `std.mem` 根同时再导出 `Rc`、`Arc`、`RcWeak`、`ArcWeak`（`Weak` 若指 `RcWeak` 易与 `Arc` 混淆，具体见 §5 Q8/跨模块约定或随 `Arc` 迁入 `std.sync.Arc` 时明确）。
- **`Box<T, .A, .allocator>` 补全**：
  - 保留 [boxed.fl](../../library/std/mem_impl/boxed.fl) 已有 API，补充：`pub unsafe fn from_raw(block: RawBlock) -> Self`、`pub fn into_raw(self) -> RawBlock`、`pub fn as_ptr(*self) -> *T`、`pub unsafe fn leak(self) -> *T`、`pub fn try_clone(*self) -> !AllocErr Self where T :- Clone`、`impl Eq`/`Ord`/`Hash`/`Display`、`impl Deref`/`DerefMut`（`assoc Target = T`）。
- **`TryClone`、`ToOwned` 与 `Cow<B, .A, .allocator>`**：
  ```flurry
  pub trait TryClone {
      fn try_clone(*self) -> !AllocErr Self;
  }

  pub trait ToOwned
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
  {
      assoc Owned: Type;

      fn to_owned(*self) -> !AllocErr Owned;
  }

  pub enum Cow
  where
      B,
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
      requires B :- ToOwned<.A = A, .allocator = allocator>,
  {
      borrowed(B),
      owned(B.as(ToOwned<.A = A, .allocator = allocator>).Owned),
  }

  impl Cow<B, .A = A, .allocator = alloc>
  where
      B,
      A: Type,
      requires A :- Allocator,
      quote alloc: lift *A,
      requires B :- ToOwned<.A = A, .allocator = alloc>,
  {
      pub fn is_borrowed(*self) -> bool {
          self.* match {
              .borrowed(_) => true,
              .owned(_) => false,
          }
      }

      pub fn is_owned(*self) -> bool {
          not self.is_borrowed()
      }

      pub fn to_mut(*self) -> !AllocErr *B.as(ToOwned<.A = A, .allocator = alloc>).Owned {
          self.* match {
              .borrowed(b) => {
                  let owned = b.as(ToOwned<.A = A, .allocator = alloc>).to_owned()!;
                  self.* = Cow<B, .A = A, .allocator = alloc>.owned(owned);
              },
              .owned(_) => {},
          }
          self.* match {
              .owned(ref_owned) => ref_owned.ref,
              .borrowed(_) => std.process.abort(),
          }
      }

      pub fn into_owned(self) -> !AllocErr B.as(ToOwned<.A = A, .allocator = alloc>).Owned {
          self match {
              .borrowed(b) => b.as(ToOwned<.A = A, .allocator = alloc>).to_owned(),
              .owned(o) => o,
          }
      }

      pub fn try_clone(*self) -> !AllocErr Self
      where
          B :- Copy,
          requires B.as(ToOwned<.A = A, .allocator = alloc>).Owned :- TryClone,
      {
          self.* match {
              .borrowed(b) => Cow<B, .A = A, .allocator = alloc>.borrowed(b),
              .owned(o) => {
                  let cloned = o.ref.as(TryClone).try_clone()!;
                  Cow<B, .A = A, .allocator = alloc>.owned(cloned)
              },
          }
      }
  }
  ```
  - `str` 实现 `ToOwned<.A = A, .allocator = alloc>` 产出 `String<.A = A, .allocator = alloc>`；`Slice<T>`（`T :- Clone`）产出 `Vec<T, .A = A, .allocator = alloc>`；`CStr` 产出 `CString<...>`；`OsStr` 产出 `OsString<...>`；`Path` 产出 `PathBuf<...>`；一般 `*T`（`T :- Clone`）产出 `T`。

#### 3.1.4 内存工具、未初始化控制与正反例（与 `04` `builtin.mem` 协同）

> **跨包归属声明（对齐 [04-builtin-types-math-iterators-fmt.md](./04-builtin-types-math-iterators-fmt.md) `WL-04-26/27`）**：底层未初始化与原始内存原语（`MaybeUninit<T>`、`ManuallyDrop<T>`、`unsafe fn swap`、`unsafe fn replace`、`unsafe fn zeroed`、`fn forget`、`size_of` / `align_of` / `needs_drop`）的**权威实现归属 `builtin.mem`**，`std.mem` 对其进行**公开重导出**并补充高层安全辅助函数（如 `take`、`drop` 与对齐运算）。

- **未初始化与禁止自动析构容器**（`builtin.mem` 原生定义，`std.mem` 重导出；以下展示其通用 `union` 语义基础）：
  ```flurry
  -- builtin.mem / std.mem 重导出契约（safety 与 04 WL-04-26 对齐）
  pub use builtin.mem.{
      MaybeUninit, ManuallyDrop,
      swap, replace, forget, zeroed, drop_in_place,
      size_of, align_of, size_of_val, align_of_val, needs_drop,
  }

  -- union 机制示例：union 赋值与离开作用域不自动执行旧成员 Drop
  union MaybeUninitPayload where T {
      uninit: void,
      value: T,
  }

  pub struct MaybeUninitLocalExample where T {
      inner: MaybeUninitPayload<T>,
  }

  impl MaybeUninitLocalExample<T> where T {
      pub pure fn uninit() -> Self {
          MaybeUninitLocalExample<T> { inner: MaybeUninitPayload<T> { uninit: {} } }
      }

      pub pure fn new(value: T) -> Self {
          MaybeUninitLocalExample<T> { inner: MaybeUninitPayload<T> { value: value } }
      }

      pub pure fn zeroed() -> Self {
          builtin.mem.make_zeroed_uninit<T>()
      }

      -- 覆盖前不析构旧未初始化槽位（依赖 union 免除旧成员 Drop）
      pub fn write(*self, value: T) -> *T {
          self.inner.value = value;
          self.as_ptr()
      }

      pub unsafe fn assume_init(self) -> T {
          unsafe { self.inner.value }
      }

      pub unsafe fn assume_init_read(*self) -> T {
          unsafe { builtin.mem.raw_read<T>(self.as_ptr()'bitvec.as(usize)) }
      }

      pub unsafe fn assume_init_drop(*self) {
          let val = unsafe { self.assume_init_read() }
          drop(val);
      }

      pub fn as_ptr(*self) -> *T {
          self.inner.ref'bitvec.as(*T)
      }
  }
  ```
  > 依据 [docs/type/user-defined-types/unions.md](../../docs/type/user-defined-types/unions.md)，`union` 赋值与离开作用域不自动执行旧成员 `Drop`；因此 `MaybeUninitPayload<T>` 可作为构建 `MaybeUninit<T>`、`ManuallyDrop<T>` 与 `std.mem.forget` 的通用机制，前提是 `program_verification/src/drop/` 明确将包含仅构造态 `union` 成员的封装视为免自动析构。
- **核心内存函数（`builtin.mem` 重导出 + `std.mem` 扩展，安全级别与 `04` 一致）**：
  - `pub unsafe fn swap(a: *T, b: *T) where T`（与 `04` `WL-04-27` 一致：**标为 `unsafe`**，因为 `*T` 允许存在活跃别名指针；安全处理 `a == b`，用 `copy_nonoverlapping`/`read`/`write` 交换而不触发 `Drop`）
  - `pub unsafe fn replace(dest: *T, src: T) -> T where T`（同 `04` 标为 `unsafe`，移入 `src` 并返回原值，不销毁旧值）
  - `pub unsafe fn take(dest: *T) -> T where T :- Default`（封装 `unsafe { replace(dest, T.as(Default).default_value()) }`）
  - `pub fn drop(value: T) where T {}`（安全函数：显式提前结束仿射值寿命）
  - `pub fn forget(value: T) where T`（安全函数：构造 `ManuallyDrop<T>.new(value)` 后不析构，安全但可能泄漏资源，符合 Flurry 仿射语义）
  - `pub unsafe fn zeroed() -> T where T`（同 `04` `WL-04-27` 标为 `unsafe`；仅当 `MaybeUninit<T>.zeroed()` 构造成员时可用 `pure`）
  - `pub pure fn align_up(addr: usize, align: usize) -> ?usize`、`pub pure fn align_down(addr: usize, align: usize) -> ?usize`、`pub pure fn is_aligned(addr: usize, align: usize) -> bool`、`pub pure fn align_offset(addr: usize, align: usize) -> ?usize`
  - `pub pure fn size_of() -> usize where T`（与 `04` `WL-04-27` 一致转发 `Type.size_of(T)`）、`pub pure fn align_of() -> usize where T`、`pub pure fn needs_drop() -> bool where T`（待 PTC/反射原子支持）。
  - **无 `transmute` 规则**：Flurry 刻意不提供 `std.mem.transmute` 或 `bitcast`；安全与不安全的位重解释统一使用 `'bitvec`（如 `val'bitvec.as(Target)`）。

- **正例与拒绝例（运行时依赖 Allocator 身份与生命期边界）**：
  ```flurry
  -- 正例：同一类型的 Box 在独立 FixedBufferAllocator 实例间保持类型隔离，zero storage overhead
  fn use_custom_allocators(buf_a: RawBlock) -> !AllocErr i32 {
      var fba_a = FixedBufferAllocator.from_raw_block(buf_a);
      var box_a = Box<i32, .A = FixedBufferAllocator, .allocator = fba_a.ref>.new(10)!;
      box_a.get()
  }

  -- 拒绝例 1（类型身份冲突）：不得把 fba_a 分配的 Box 交付给要求 fba_b 的位置
  fn reject_mismatched_allocator(
      a: *FixedBufferAllocator,
      b: *FixedBufferAllocator,
      boxed: Box<i32, .A = FixedBufferAllocator, .allocator = a>,
  ) -> Box<i32, .A = FixedBufferAllocator, .allocator = b> {
      boxed -- 编译错误：类型依赖项 a 与 b 身份不同，不可混同
  }

  -- 拒绝例 2（Drop 环境悬空）：在 Box 仍存活时提前销毁它所依赖的局部 allocator
  fn reject_dangling_allocator_on_drop(
      buf: RawBlock,
      out: *FixedBufferAllocator,
  ) -> Box<i32, .A = FixedBufferAllocator, .allocator = out> {
      var fba = FixedBufferAllocator.from_raw_block(buf);
      let boxed = Box<i32, .A = FixedBufferAllocator, .allocator = fba.ref>.new(42)!;
      boxed -- 编译错误：1) fba.ref 身份与 out 不等；2) 即使返回类型引用 fba，fba 在离开作用域时销毁导致 Drop 环境悬空
  }
  ```

---

### 3.2 `std.collections`：序列、映射/集合/堆、运行时依赖 Arena/SoA 与切片算法

#### 3.2.1 序列与双端/小容量容器

1. **`Vec<T, .A = DefaultAllocator, quote .allocator = DefaultAllocator.GLOBAL.ref>`**（扩展 [vec.fl](../../library/std/collections_impl/vec.fl)）：
   - 已有能力保留：`new`、`with_capacity`、`len`、`capacity`、`is_empty`、`as_addr`、`as_slice`、`set_len`、`reserve`、`shrink_to_fit`、`push`、`pop`、`insert`、`remove`、`swap_remove`、`swap`、`replace`、`set`、`truncate`、`clear`、`reverse`、`append`（支持跨 allocator `.B`）、`with_ref`、`for_each_ref`、`try_for_each_ref`、`get`/`first`/`last`、`contains`、`try_clone`、`Drop`、`IntoIterator`/`VecIter`、`Eq`、`ByteWriter`/`Write`、`Debug`、`ListPatternView`/`StableListPatternView`。
   - **Wishlist 新增完整 API**：
     - 容量精细控制：`reserve_exact(*self, additional: usize) -> !AllocErr void`、`shrink_to(*self, min_capacity: usize) -> !AllocErr void`。
     - 原地过滤与去重：
       - `retain(*self, pred: %c fn(*T) -> bool) where c: Type.vars.Closure`
       - `dedup(*self) where T :- Eq`（需按引用比较 `*T` 或 `PartialEq`/借用 `Eq`！当前 `Eq.eq(self, other: Self)` 按值消费，参见 §5 与 `01`！）
       - `dedup_by(*self, same_bucket: %c fn(*T, *T) -> bool) where c: Type.vars.Closure`
       - `dedup_by_key(*self, key: %c fn(*T) -> K) where K :- Eq, c: Type.vars.Closure`
     - 范围抽取、拼接、分裂与缩放：
       - `drain(*self, range: R) -> VecDrain<T, .A = A, .allocator = allocator> where R :- RangeBounds<usize>`（`VecDrain` 持有对原 `Vec` 的 `*Vec` 独占借用与区间边界；即使消费者中途 `break` 或直接 `drop(drain)`，其 `Drop` 也必须先销毁区间内剩余元素并把尾部元素前移填补空洞，维持 `Vec` 长度一致与仿射不变量！）
       - `splice(*self, range: R, replace_with: I) -> !AllocErr VecSplice<T, I, .A = A, .allocator = allocator> where R :- RangeBounds<usize>, I :- Iterator, requires I.as(Iterator).Item == T`
       - `split_off(*self, at: usize) -> !AllocErr Self`（检查 `at <= len`；若越界需定义显式错误或 panic，见 §5 Q1）
       - `resize(*self, new_len: usize, value: T) -> !AllocErr void where T :- Clone`
       - `resize_with(*self, new_len: usize, f: %c fn() -> T) -> !AllocErr void where c: Type.vars.Closure`
       - `extend(*self, iter: I) -> !AllocErr void where I :- Iterator, requires I.as(Iterator).Item == T`
       - `extend_from_slice(*self, slice: Slice<T>) -> !AllocErr void where T :- Clone`
     - 切片与迭代访问：`as_mut_slice(*self) -> MutSlice<T>`、`as_ptr(*self) -> *T`、`iter_ref(*self) -> #emit_item<*T> void`（或 `VecRefIter`）、`into_boxed_slice(self) -> !AllocErr Box<Slice<T>, .A = A, .allocator = allocator>`、`Index<usize>` / `IndexMut<usize>`、`FromIterator<T>`（使用默认全局分配器，或提供 `from_iter_in` 显式传 allocator）。
2. **`VecDeque<T, .A = DefaultAllocator, quote .allocator = DefaultAllocator.GLOBAL.ref>`**（扩展 [vec_deque.fl](../../library/std/collections_impl/vec_deque.fl)）：
   - 补齐：`reserve_exact(*self, additional: usize) -> !AllocErr void`、`shrink_to_fit(*self) -> !AllocErr void`、`shrink_to(*self, min_capacity: usize) -> !AllocErr void`、`make_contiguous(*self) -> MutSlice<T>`（环形双段存在时线性旋转对齐到索引 0，O(N) 原地旋转或重排）、`as_slices(*self) -> Tuple<Slice<T>, Slice<T>>`、`as_mut_slices(*self) -> Tuple<MutSlice<T>, MutSlice<T>>`、`rotate_left(*self, k: usize)`、`rotate_right(*self, k: usize)`、`insert(*self, index: usize, value: T) -> !AllocErr bool`、`remove(*self, index: usize) -> ?T`、`swap_remove_front(*self, index: usize) -> ?T`、`swap_remove_back(*self, index: usize) -> ?T`、`truncate(*self, len: usize)`、`retain(*self, pred)`、`drain(*self, range)`、`split_off(*self, at: usize) -> !AllocErr Self`、`append(*self, other: *VecDeque<T, .A = B, .allocator = b>)`、`resize`、`resize_with`、`contains(*self, x: *T) where T :- Eq`、`binary_search*`、`iter_ref`、`Index<usize>`/`IndexMut<usize>`。
3. **`SmallVec<T, N: usize, .A = DefaultAllocator, quote .allocator = DefaultAllocator.GLOBAL.ref>`**（注：采用 Flurry 标准值参数 `N: usize`，无多余修饰关键词；边界归属见 Q11）：
   - 表示：支持内联栈存储 `Array<MaybeUninit<T>, N>` 与堆溢出退化态 `{ raw_addr, raw_len, raw_cap }`（通过 `quote .allocator` 分配，ZST 永不溢出）：
   ```flurry
   enum SmallVecRepr
   where
       T,
       N: usize,
   {
       inline(Array<MaybeUninit<T>, N>, usize),
       heap(usize, usize, usize), -- raw_addr, raw_len, raw_cap
   }

   pub struct SmallVec
   where
       T,
       N: usize,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       repr: SmallVecRepr<T, N>,
   }
   ```
   - 完整 API 与 `Vec` 对齐：`new() -> Self`、`from_slice(slice: Slice<T>) -> !AllocErr Self where T :- Clone`、`len(*self) -> usize`、`capacity(*self) -> usize`、`inline_capacity(*self) -> usize`、`spilled(*self) -> bool`、`push(*self, value: T) -> !AllocErr void`、`pop(*self) -> ?T`、`insert(*self, index: usize, value: T) -> !AllocErr bool`、`remove(*self, index: usize) -> ?T`、`swap_remove(*self, index: usize) -> ?T`、`truncate(*self, new_len: usize)`、`clear(*self)`、`reserve(*self, additional: usize) -> !AllocErr void`、`shrink_to_fit(*self) -> !AllocErr void`、`into_vec(self) -> !AllocErr Vec<T, .A = A, .allocator = allocator>`、`as_slice(*self) -> Slice<T>`、`as_mut_slice(*self) -> MutSlice<T>`、`retain`、`drain`、`Drop`、`IntoIterator`、`TryClone`、`Eq`、`Debug`。

#### 3.2.2 映射、集合、有序树、堆、位集与环形缓冲区

1. **`HashMap<K, V, .H: Type = DefaultBuildHasher, .A: Type = DefaultAllocator, ...>`**（升级 [hash_map.fl](../../library/std/collections_impl/hash_map.fl)）：
   - **完整签名与自定义散列器**：
     ```flurry
     pub struct HashMap
     where
         K,
         V,
         .H: Type = DefaultBuildHasher,
         requires H :- BuildHasher,
         .A: Type = DefaultAllocator,
         requires A :- Allocator,
         quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
     {
         raw_addr: usize,
         raw_len: usize,
         raw_tombstones: usize,
         raw_cap: usize,
         hash_builder: H, -- DefaultBuildHasher 为 ZST（如 FNV/SipHash 固定种子或随机种子生成器），零或极小开销
     }
     ```
     补充构造器 `with_hasher(hash_builder: H) -> Self` 与 `with_capacity_and_hasher(capacity: usize, hash_builder: H) -> !AllocErr Self`。
   - **移除冗余 `K :- Clone` 约束**：当前 `hash_map.fl` 在 `find_slot`/`probe_for_insert`/`get`/`remove`/`contains_key` 中调用 `existing.as(Clone).clone()`，导致非 `Clone` 键无法查询！须升级为基于 `*K` 指针借用完成 `Hash` 与 `Eq`（配合 `Hash`/`Eq` 协议对指针接收者的支持或 `Borrow` 协议支持 `str`/`Slice<u8>` 异构查找 `K :- Borrow<Q>`）。
   - **完整 `Entry` 协议**：
     ```flurry
     pub struct OccupiedEntry
     where
         K,
         V,
         .H: Type = DefaultBuildHasher,
         requires H :- BuildHasher,
         .A: Type = DefaultAllocator,
         requires A :- Allocator,
         quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
     {
         map: *HashMap<K, V, .H = H, .A = A, .allocator = allocator>,
         slot: usize,
     }

     pub struct VacantEntry
     where
         K,
         V,
         .H: Type = DefaultBuildHasher,
         requires H :- BuildHasher,
         .A: Type = DefaultAllocator,
         requires A :- Allocator,
         quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
     {
         map: *HashMap<K, V, .H = H, .A = A, .allocator = allocator>,
         key: K,
         hash: u64,
         slot: usize,
     }

     pub enum Entry
     where
         K,
         V,
         .H: Type = DefaultBuildHasher,
         requires H :- BuildHasher,
         .A: Type = DefaultAllocator,
         requires A :- Allocator,
         quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
     {
         occupied(OccupiedEntry<K, V, .H = H, .A = A, .allocator = allocator>),
         vacant(VacantEntry<K, V, .H = H, .A = A, .allocator = allocator>),
     }

     pub struct OccupiedErr
     where
         K,
         V,
         .H: Type = DefaultBuildHasher,
         requires H :- BuildHasher,
         .A: Type = DefaultAllocator,
         requires A :- Allocator,
         quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
     {
         entry: OccupiedEntry<K, V, .H = H, .A = A, .allocator = allocator>,
         value: V,
     }
     ```
     核心方法：
     - `HashMap`: `entry(*self, key: K) -> !AllocErr Entry<K, V, .H = H, .A = A, .allocator = allocator>`、`get_ref(*self, key: *K) -> ?*V`、`get_key_value(*self, key: *K) -> ?Tuple<*K, *V>`、`try_insert(*self, key: K, value: V) -> ![AllocErr, OccupiedErr<K, V, .H = H, .A = A, .allocator = allocator>] *V`（冲突时通过 `OccupiedErr` 原样交回调用方 `value: V` 与 `OccupiedEntry`，满足 `PascalCaseErr` 与仿射保活！）、`remove_entry(*self, key: K) -> ?Tuple<K, V>`、`retain(*self, pred: %c fn(*K, *V) -> bool)`、`drain(*self)`、`keys(*self)`、`values(*self)`、`iter_ref(*self)`、`shrink_to_fit(*self) -> !AllocErr void`、`Eq`（支持不同构造顺序下的相等比较）。
     - `OccupiedEntry`: `key(*self) -> *K`、`get(*self) -> *V`、`into_mut(self) -> *V`、`insert(*self, value: V) -> V`、`remove(self) -> V`、`remove_entry(self) -> Tuple<K, V>`。
     - `VacantEntry`: `key(*self) -> *K`、`into_key(self) -> K`、`insert(self, value: V) -> *V`（因 `entry()` 已确保表容量，此处无分配失败）。
     - `Entry`: `or_insert(self, default: V) -> *V`、`or_insert_with(self, default: %c fn() -> V) -> *V`、`or_insert_with_key(self, default: %c fn(*K) -> V) -> *V`、`or_default(self) -> *V where V :- Default`、`and_modify(self, f: %c fn(*V)) -> Self`、`key(*self) -> *K`。
   - **重要资源契约**：`VacantEntry` 持有所有权键 `key: K`，若用户拿到 `Entry.vacant` 后既不调用 `insert` 也不调用 `into_key` 而直接丢弃 `Entry`，结构化 `Drop` 自动释放 `key`，零内存泄漏！

2. **`HashSet<T, .H = DefaultBuildHasher, .A = DefaultAllocator, quote .allocator = DefaultAllocator.GLOBAL.ref>`**（升级 [hash_set.fl](../../library/std/collections_impl/hash_set.fl)）：
   - 继承 `HashMap<T, void, .H, .A, .allocator>`，补齐 `.H` 自定义散列器、`get_ref(*self, value: *T) -> ?*T`、`get_or_insert(*self, value: T) -> !AllocErr *T`、`get_or_insert_with(*self, value: *T, f)`、`replace(*self, value: T) -> !AllocErr ?T`、`take(*self, value: *T) -> ?T`、`retain`、`drain`、`is_superset`、`union` / `intersection` / `difference` / `symmetric_difference`（返回惰性 `Iterator` 适配器）、`Eq`、`shrink_to_fit`。

3. **`BTreeMap<K, V, .A = DefaultAllocator, quote .allocator = DefaultAllocator.GLOBAL.ref>` 与 `BTreeSet<T, .A, .allocator>`**：
   - 基于 B-Tree（分支因子 `B = 6`，节点内最多 `11` 个键与 `12` 个子指针），按 `K :- Ord` 维护全局有序性；内部/叶子节点统一通过 `quote .allocator` 分配与回收：
   ```flurry
   pub struct BTreeMap
   where
       K,
       V,
       requires K :- Ord,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       root_addr: usize,
       len: usize,
       height: usize,
   }

   pub struct BTreeOccupiedEntry
   where
       K,
       V,
       requires K :- Ord,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       map: *BTreeMap<K, V, .A = A, .allocator = allocator>,
       node_addr: usize,
       slot: usize,
   }

   pub struct BTreeVacantEntry
   where
       K,
       V,
       requires K :- Ord,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       map: *BTreeMap<K, V, .A = A, .allocator = allocator>,
       key: K,
       node_addr: usize,
       slot: usize,
   }

   pub enum BTreeEntry
   where
       K,
       V,
       requires K :- Ord,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       occupied(BTreeOccupiedEntry<K, V, .A = A, .allocator = allocator>),
       vacant(BTreeVacantEntry<K, V, .A = A, .allocator = allocator>),
   }

   pub struct BTreeSet
   where
       T,
       requires T :- Ord,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       inner: BTreeMap<T, void, .A = A, .allocator = allocator>,
   }
   ```
   - `BTreeMap` API：`new`、`len`、`is_empty`、`clear`、`get_ref(*self, key: *K) -> ?*V`、`get(*self, key: *K) -> ?V where V :- Copy`、`get_key_value(*self, key: *K) -> ?Tuple<*K, *V>`、`contains_key(*self, key: *K) -> bool`、`insert(*self, key: K, value: V) -> !AllocErr ?V`、`remove(*self, key: *K) -> ?V`、`remove_entry(*self, key: *K) -> ?Tuple<K, V>`、`first_key_value(*self) -> ?Tuple<*K, *V>`、`last_key_value(*self) -> ?Tuple<*K, *V>`、`pop_first(*self) -> ?Tuple<K, V>`、`pop_last(*self) -> ?Tuple<K, V>`、`entry(*self, key: K) -> BTreeEntry<K, V, .A = A, .allocator = allocator>`、`range(*self, bounds: R) where R :- RangeBounds<K>`、`split_off(*self, key: *K) -> !AllocErr Self`、`append(*self, other: *BTreeMap<K, V, .A = B, .allocator = b>) -> !AllocErr void`、`retain`、`keys`、`values`、`iter_ref`、`into_iter`、`TryClone`、`Drop`、`Eq`、`Ord`、`Hash`、`Debug`。
   - `BTreeSet` 暴露对称有序集合接口（`first`、`last`、`pop_first`、`pop_last`、`range`、`take`、`replace`、`split_off`、`append`、`union`、`intersection`、`difference`、`symmetric_difference`、`is_subset`、`is_superset`、`is_disjoint`、`retain`、`Eq`、`Ord`、`Hash`、`Debug`）。

4. **`BinaryHeap<T, .A = DefaultAllocator, quote .allocator = DefaultAllocator.GLOBAL.ref>`**：
   ```flurry
   pub struct BinaryHeap
   where
       T,
       requires T :- Ord,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       data: Vec<T, .A = A, .allocator = allocator>,
   }

   pub struct PeekMut
   where
       T,
       requires T :- Ord,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       heap: *BinaryHeap<T, .A = A, .allocator = allocator>,
       sift: bool,
   }
   ```
   - 核心 API：`new`、`with_capacity`、`len`、`capacity`、`is_empty`、`clear`、`reserve`、`reserve_exact`、`shrink_to_fit`、`push(*self, item: T) -> !AllocErr void`、`pop(*self) -> ?T`、`peek_ref(*self) -> ?*T`、`peek(*self) -> ?T where T :- Copy`、`peek_mut(*self) -> ?PeekMut<T, .A = A, .allocator = allocator>`（`PeekMut.Drop` 自动 `sift_down(0)` 恢复堆序，提供 `pop(self) -> T` 与 `with_ref`）、`drain(*self)`、`into_sorted_vec(self) -> Vec<T, .A = A, .allocator = allocator>`、`append`、`retain`、`TryClone`、`IntoIterator`、`Debug`。

5. **`BitSet<.A = DefaultAllocator, quote .allocator = DefaultAllocator.GLOBAL.ref>`**：
   ```flurry
   pub struct BitSet
   where
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       words: Vec<u64, .A = A, .allocator = allocator>,
       bit_len: usize,
   }
   ```
   - 与固定宽度整数位集 `builtin.BitVec<n>` 不同，`BitSet` 是动态可增长堆分配位集。
   - API：`new`、`with_capacity_bits(bits: usize)`、`len_bits(*self) -> usize`、`count_ones(*self) -> usize`、`is_empty(*self) -> bool`、`clear(*self)`、`set(*self, bit: usize) -> !AllocErr bool`、`unset(*self, bit: usize) -> bool`、`toggle(*self, bit: usize) -> !AllocErr bool`、`contains(*self, bit: usize) -> bool`、`first_set(*self) -> ?usize`、`next_set(*self, from_bit: usize) -> ?usize`、`union_with`、`intersect_with`、`difference_with`、`symmetric_difference_with`、`is_subset`、`is_disjoint`、`shrink_to_fit`、`iter_ones`、`TryClone`、`Drop`、`Eq`、`Debug`。

6. **`RingBuffer<T, N: usize>`**（无堆分配的固定容量 Inline 环形队列，无 `.A`/`.allocator` 参数；边界归属见 Q11）：
   ```flurry
   pub enum RingFullErr {
       Full,
   }

   pub struct RingBuffer
   where
       T,
       N: usize,
       requires N > 0,
   {
       storage: Array<MaybeUninit<T>, N>,
       head: usize,
       len: usize,
   }
   ```
   - 核心方法：`new()`、`capacity() -> usize`（返回 `N`）、`len(*self) -> usize`、`is_empty(*self) -> bool`、`is_full(*self) -> bool`、`push_back(*self, value: T) -> !T void`（满时归还所有权 `error value`，不丢失仿射资源！）或 `try_push_back(*self, value: T) -> !RingFullErr void`（若需丢弃传入值）与 `push_back_overwrite(*self, value: T) -> ?T`（覆盖并弹出最旧元素）、`push_front`、`pop_front(*self) -> ?T`、`pop_back(*self) -> ?T`、`front_ref`、`back_ref`、`get_ref(index: usize)`、`clear(*self)`、`Drop`（仅按 `head .. head + len` 模 `N` 析构活跃元素）。

#### 3.2.3 运行时依赖 Arena、`SlotMap` 与类型化 `SoA`（Silver Layer 4；边界归属见 Q11）

这是 Flurry 运行时依赖类型与无开销句柄的核心应用（依据 [docs/type/runtime-dependent-type.md](../../docs/type/runtime-dependent-type.md) 与 [tmp/silver-design/04-compiler-wishlist.md](../../tmp/silver-design/04-compiler-wishlist.md) P1-4，意图原型另见最近期 `proposal_trace/2026-8-5-typed-Soa.fl`）：

1. **`Arena<T, .A, .allocator>` 与 `NodeIndex<.arena>`**：
   ```flurry
   pub struct NodeIndex
   where
       T,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
       quote .arena: lift *Arena<T, .A = A, .allocator = allocator>,
   {
       index: u32,
   }

   impl Clone for NodeIndex<T, .A = A, .allocator = alloc, .arena = arena_ref>
   where
       T,
       A: Type,
       requires A :- Allocator,
       quote alloc: lift *A,
       quote arena_ref: lift *Arena<T, .A = A, .allocator = alloc>,
   {
       fn clone(*self) -> Self {
           NodeIndex<T, .A = A, .allocator = alloc, .arena = arena_ref> { index: self.index.* }
       }
   }

   impl Copy for NodeIndex<T, .A = A, .allocator = alloc, .arena = arena_ref>
   where
       T,
       A: Type,
       requires A :- Allocator,
       quote alloc: lift *A,
       quote arena_ref: lift *Arena<T, .A = A, .allocator = alloc>,
   {}

   pub struct Arena
   where
       T,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       items: Vec<T, .A = A, .allocator = allocator>,
   }

   pub struct SlotKey
   where
       V,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
       quote .map: lift *SlotMap<V, .A = A, .allocator = allocator>,
   {
       index: u32,
       generation: u32,
   }

   enum SlotEntry where V {
       occupied(u32, V),
       vacant(u32, u32), -- generation, next_free
   }

   pub struct SlotMap
   where
       V,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       slots: Vec<SlotEntry<V>, .A = A, .allocator = allocator>,
       free_head: u32,
       len: usize,
   }
   ```
   - **关键不变量**：`Type.size_of(NodeIndex<...>) == 4`、`Type.size_of(SlotKey<...>) == 8`（仅保存紧凑索引/代数，不随每个句柄拷贝 `*Arena`/`*SlotMap` 指针！）。`arena_ref`/`map_ref` 作为运行期环境依赖（`env_parameters`）在操作点按共享环境取得。
   - 方法：`Arena` 暴露 `new()`、`alloc(*self, value: T) -> !AllocErr NodeIndex<T, .A = A, .allocator = alloc, .arena = self>`、`get_ref(*self, handle: NodeIndex<T, .A = A, .allocator = alloc, .arena = self>) -> *T`、`len(*self) -> usize`、`clear(*self)`（清空并递增版本/要求无残留句柄）；`NodeIndex` 经 `quote .arena` 环境可直接提供 `get_ref(self) -> *T` 与 `with_ref(self, f)`。
   - **正反例（跨存储隔离）**：
     ```flurry
     -- 拒绝例：两个独立 Arena 的句柄绝对不可混用
     fn reject_cross_arena(
         left: *Arena<i32>,
         right: *Arena<i32>,
         handle_from_left: NodeIndex<i32, .arena = left>,
     ) -> *i32 {
         right.get_ref(handle_from_left) -- 编译错误：left 与 right 的 runtime identity 不同
     }
     ```
2. **`SlotMap<V, .A, .allocator>` 与 `SecondaryMap<K, U, .A, .allocator>`**：
   - 使用 64 位紧凑代际索引（`index: u32, generation: u32`），区分已释放槽位与复用槽位（ABA 安全）；支持 `insert(*self, value: V) -> !AllocErr SlotKey<...>`、`remove(*self, key: SlotKey<...>) -> ?V`、`get_ref(*self, key: SlotKey<...>) -> ?*V`、`contains_key(*self, key: SlotKey<...>) -> bool`、`clear(*self)`、`retain`、`drain`、`iter_ref`、`into_iter`。
3. **类型化 `SoA<T, .A, .allocator>`（基于编译期结构反射生成，边界归属见 Q11）**：
   - 利用 `meta.reflection.reflect_struct`（`05` `WL-05-12`）与现有 `meta.struct_field_names(T)` / `meta.struct_field_types(T)` 配合 `meta.definitions.StructBuilder`，在编译期自动构造 `StructOfArray<T, .A, .allocator>`：为 `T` 的每个字段 `f_i: F_i` 分配独立且支持缓存行对齐（提案属性 `^attrs.layout.align(64)`，待在 `official-attribute-keys.md` 登记 `__flurry_align`，见 `05` `WL-05-16`）的连续数组列（避免读取单列时加载无关冷字段），同时维护共享的 `len: usize`、`cap: usize`。
   - 提供列访问与惰性模式视图：
     - `push(*self, value: T) -> !AllocErr SoaIndex<...>`、`pop(*self) -> ?T`、`swap_remove(*self, idx: SoaIndex<...>) -> ?T`、`reserve(*self, add: usize) -> !AllocErr void`、`clear(*self)`、`Drop`（逐列析构初始化元素并释放各列到 `alloc`）；
     - `slice_of(comptime field: Symbol) -> Slice<FieldType>` 与 `mut_slice_of(comptime field: Symbol) -> MutSlice<FieldType>`（零分支 SIMD 遍历直接消费各字段切片！）；
     - 结合 `case deref(*self)` 视图模式：当模式仅绑定 `.op` 与 `.span` 而以 `_` 忽略 `.lhs` 时，仅加载对应两列，完全跳过未绑定字段的内存读取。

#### 3.2.4 `Slice<T>`、`MutSlice<T>` 与完整切片算法

为保持 `Slice<T>`（`Copy`，不可变视图）与原地可变操作的资源隔离，新增独占/可变切片视图 `MutSlice<T>`：

```flurry
pub enum BinarySearchResult {
    found(usize),
    missing(usize),
}

pub struct MutSlice where T {
    raw_addr: usize,
    raw_len: usize,
}

impl MutSlice<T> where T {
    pub pure fn len(*self) -> usize {
        self.raw_len.*
    }

    pub pure fn is_empty(*self) -> bool {
        self.raw_len.* == 0
    }

    pub fn reborrow(*self) -> MutSlice<T> {
        MutSlice<T> { raw_addr: self.raw_addr.*, raw_len: self.raw_len.* }
    }

    pub fn as_slice(*self) -> Slice<T> {
        unsafe { Slice<T>.from_raw_parts(self.raw_addr.*, self.raw_len.*) }
    }

    pub fn swap(*self, a: usize, b: usize) -> bool
    where
        requires not Type.predicates.is_comptime_only(T),
        requires not Type.predicates.is_not_sized(T),
    {
        let len = self.len();
        if a >= len or b >= len {
            return false;
        }
        if a == b or Type.size_of(T) == 0 {
            return true;
        }
        let elem_stride = Type.stride_of(T);
        if elem_stride == 0 {
            return true;
        }
        let off_a = builtin.mem.checked_mul_usize(a, elem_stride) match {
            v? => v,
            null => return false,
        }
        let off_b = builtin.mem.checked_mul_usize(b, elem_stride) match {
            v? => v,
            null => return false,
        }
        let addr_a = builtin.mem.checked_add_usize(self.raw_addr.*, off_a) match {
            v? => v,
            null => return false,
        }
        let addr_b = builtin.mem.checked_add_usize(self.raw_addr.*, off_b) match {
            v? => v,
            null => return false,
        }
        unsafe {
            let pa = addr_a'bitvec.as(*T);
            let pb = addr_b'bitvec.as(*T);
            builtin.mem.swap<T>(pa, pb);
        }
        true
    }
}
```
- `MutSlice<T>`（实现 `Once` 而**不实现 `Copy`/`Clone`**，防止同一可变切片被隐式双份复制破坏独占顺序假设）。
- `Slice<T>` 完整只读算法（拓展 [slice.fl](../../library/std/collections_impl/slice.fl)）：
  - 二分查找（返回原生枚举 `BinarySearchResult`，`snake_case` 变体）：
    - `binary_search(*self, x: *T) -> BinarySearchResult where T :- Ord`
    - `binary_search_by(*self, f: %c fn(*T) -> Order) -> BinarySearchResult where c: Type.vars.Closure`
    - `binary_search_by_key(*self, b: *B, f: %c fn(*T) -> B) -> BinarySearchResult where B :- Ord, c: Type.vars.Closure`
    - `partition_point(*self, pred: %c fn(*T) -> bool) -> usize where c: Type.vars.Closure`
  - 区间与分隔迭代器（遵循 `builtin.Iterator` `inline fn iterate` + `emit_item` 协议）：
    - `chunks(*self, chunk_size: usize)`、`chunks_exact(*self, chunk_size: usize)`（附带 `.remainder() -> Slice<T>`）、`rchunks(*self, chunk_size: usize)`、`rchunks_exact(*self, chunk_size: usize)`
    - `windows(*self, size: usize)`
    - `split(*self, pred: %c fn(*T) -> bool)`、`rsplit(*self, pred)`、`splitn(*self, n: usize, pred)`、`split_inclusive(*self, pred)`
    - `iter(*self)`（产出 `*T` 或 `T where T :- Copy`）
  - 谓词与前后缀：`contains(*self, x: *T) -> bool where T :- Eq`、`starts_with(*self, needle: Slice<T>) -> bool where T :- Eq`、`ends_with(*self, needle: Slice<T>) -> bool where T :- Eq`、`strip_prefix(*self, prefix: Slice<T>) -> ?Slice<T> where T :- Eq`、`strip_suffix(*self, suffix: Slice<T>) -> ?Slice<T> where T :- Eq`。
- `MutSlice<T>` 完整原地算法：
  - 排序与选择（稳定归并排序须声明方法级分配器子句用于临时归并缓冲）：
    - `sort(*self) -> !AllocErr void where T :- Ord, .A: Type = DefaultAllocator, requires A :- Allocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref`
    - `sort_by(*self, compare: %c fn(*T, *T) -> Order) -> !AllocErr void where c: Type.vars.Closure, .A: Type = DefaultAllocator, requires A :- Allocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref`
    - `sort_by_key(*self, f: %c fn(*T) -> K) -> !AllocErr void where K :- Ord, c: Type.vars.Closure, .A: Type = DefaultAllocator, requires A :- Allocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref`
    - `sort_unstable(*self) where T :- Ord`（pdqsort，O(1) 栈空间，零堆分配）
    - `sort_unstable_by(*self, compare: %c fn(*T, *T) -> Order)`
    - `sort_unstable_by_key(*self, f: %c fn(*T) -> K) where K :- Ord`
    - `select_nth_unstable(*self, index: usize) -> Tuple<MutSlice<T>, *T, MutSlice<T>> where T :- Ord`（及 `select_nth_unstable_by`）
  - 就地重排与填充：
    - `reverse(*self)`、`rotate_left(*self, mid: usize)`、`rotate_right(*self, k: usize)`
    - `fill(*self, value: T) where T :- Clone`、`fill_with(*self, f: %c fn() -> T)`
    - `copy_from_slice(*self, src: Slice<T>) -> bool where T :- Copy`、`clone_from_slice(*self, src: Slice<T>) -> bool where T :- Clone`、`swap_with_slice(*self, other: *MutSlice<T>) -> bool`
    - `split_at_mut(*self, mid: usize) -> ?Tuple<MutSlice<T>, MutSlice<T>>`、`chunks_mut(*self, chunk_size: usize)`、`chunks_exact_mut(*self, chunk_size: usize)`、`rchunks_mut(*self, chunk_size: usize)`（均保持不重叠独占，不提供会别名重叠元素的可变 `windows_mut`）。

---

### 3.3 `std.string` 与 `std.fmt`：拥有型文本与两相格式化

#### 3.3.1 `String`、`Utf8Err` 与 `FromUtf8Err`

保留并扩展 [string.fl](../../library/std/string_impl/string.fl)：
- **错误载荷保活**：当前 `String.from_utf8` 返回 `!Utf8Err Self` 并在失败时丢弃传入的 `Vec<u8>`。Wishlist 必须补充 `FromUtf8Err` 让调用方取回原缓冲：
  ```flurry
  pub struct FromUtf8Err
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
  {
      bytes: Vec<u8, .A = A, .allocator = allocator>,
      err: Utf8Err,
  }

  impl FromUtf8Err<.A = A, .allocator = alloc>
  where
      A: Type,
      requires A :- Allocator,
      quote alloc: lift *A,
  {
      pub fn utf8_error(*self) -> Utf8Err {
          self.err.*
      }

      pub fn as_bytes(*self) -> Slice<u8> {
          self.bytes.as_slice()
      }

      pub fn into_bytes(self) -> Vec<u8, .A = A, .allocator = alloc> {
          self.bytes
      }
  }
  ```
  - `String.from_utf8(bytes: Vec<u8, .A = A, .allocator = alloc>) -> !FromUtf8Err<.A = A, .allocator = alloc> Self`。
  - 注：`Utf8Err` 须补齐 `Display` 与 `error_len(*self) -> ?usize`（指示无效序列字节长 1..3，截断结尾返回 `null`）。
- **`String<.A, .allocator>` Wishlist 新增 API**：
  - 借用视图：`as_str(*self) -> str`（受 `issues/0003` `str` backing-region 资源证明阻塞）、`as_bytes(*self) -> Slice<u8>`。
  - 容量与原位修改：`reserve_exact(*self, additional: usize) -> !AllocErr void`、`shrink_to(*self, min_capacity: usize) -> !AllocErr void`、`insert(*self, idx: usize, ch: char) -> ![AllocErr, Utf8BoundaryErr] void`（检查 `is_char_boundary(idx)`）、`insert_str(*self, idx: usize, text: str)`、`remove(*self, idx: usize) -> ?char`、`retain(*self, pred: %c fn(char) -> bool)`、`split_off(*self, at: usize) -> !AllocErr Self`、`drain(*self, range: R) -> StringDrain<...>`（产出 `char` 并在异常/早退 `Drop` 时收缩空洞保持有效 UTF-8！）、`replace_range(*self, range: R, replace_with: str) -> !AllocErr void`。
  - 文本遍历与切片（部分能力可由 `builtin.text.TextBytes` / `str` 视图统一提供）：`chars(*self)`、`char_indices(*self)`、`bytes(*self)`、`lines(*self)`、`split(*self, pat)`、`rsplit`、`split_whitespace`、`trim` / `trim_start` / `trim_end`、`strip_prefix` / `strip_suffix`、`replace(from: str, to: str) -> !AllocErr Self`、`repeat(n: usize) -> !AllocErr Self`、`to_ascii_lowercase` / `to_ascii_uppercase` / `to_lowercase` / `to_uppercase`（Unicode 大小写折叠表）。
  - 协议补全：`From<str>`（使用默认全局分配器）、`Add<str>` / `Concat`（见 `01` 队列）、`FromIterator<char>`、`FromIterator<str>`、`ToOwned` 实现。
  - **安全红线**：`String` **不**直接实现可写入任意单字节的 `ByteWriter`（否则 `write_byte(0xFF)` 会破坏 UTF-8 不变量）；向 `String` 写入格式文本请继续使用 `std.fmt.format_into` 或 `push_str` / `push_char`。

#### 3.3.2 `CStr` / `CString` 与 `OsStr` / `OsString`（归属 `std.ffi`，`std.string` 可选再导出）

```flurry
pub struct CStr {
    raw_addr: usize,
    len_with_nul: usize,
}

pub enum FromBytesWithNulErr {
    InteriorNul(usize),
    NotNulTerminated,
}

pub struct NulErr
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    pos: usize,
    bytes: Vec<u8, .A = A, .allocator = allocator>,
}

pub struct CString
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    bytes: Vec<u8, .A = A, .allocator = allocator>,
}

pub struct IntoStringErr
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    inner: CString<.A = A, .allocator = allocator>,
    err: Utf8Err,
}

pub struct OsStr {
    raw_addr: usize,
    raw_len: usize,
}

pub struct OsString
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    bytes: Vec<u8, .A = A, .allocator = allocator>,
}
```

1. **`CStr` / `CString<.A, .allocator>`**：
   - `CStr`（`Copy`, `Clone`, `Eq`, `Ord`, `Hash`, `Debug`）：保证指向以 `0u8` 结尾的连续可读 C 字符串且中间无 NUL；提供 `from_bytes_with_nul(bytes: Slice<u8>) -> !FromBytesWithNulErr CStr`、`from_bytes_until_nul(bytes: Slice<u8>) -> !FromBytesWithNulErr CStr`、`unsafe from_ptr(ptr: *c_char) -> CStr`、`as_ptr(self) -> *c_char`、`to_bytes(self) -> Slice<u8>`、`to_bytes_with_nul(self) -> Slice<u8>`、`to_str(self) -> !Utf8Err str`、`count_bytes(self) -> usize`、`is_empty(self) -> bool`，以及带方法级分配器子句的：
     ```flurry
     pub fn to_string_lossy(self) -> !AllocErr Cow<str, .A = A, .allocator = allocator>
     where
         .A: Type = DefaultAllocator,
         requires A :- Allocator,
         quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
     ```
   - `CString` 方法：`new(bytes: Vec<u8, .A = A, .allocator = alloc>) -> !NulErr<.A = A, .allocator = alloc> Self`、`from_str(s: str) -> ![AllocErr, NulErr<.A = A, .allocator = alloc>] Self`、`from_vec_with_nul(v: Vec<u8, .A = A, .allocator = alloc>) -> !FromBytesWithNulErr Self`、`into_bytes(self) -> Vec<u8, .A = A, .allocator = alloc>`、`into_bytes_with_nul(self) -> Vec<u8, .A = A, .allocator = alloc>`、`into_string(self) -> !IntoStringErr<.A = A, .allocator = alloc> String<.A = A, .allocator = alloc>`、`as_c_str(*self) -> CStr`、`as_ptr(*self) -> *c_char`、`into_raw(self) -> *c_char`、`unsafe from_raw(ptr: *c_char) -> Self`、`TryClone`、`Drop`、`Eq`、`Ord`、`Hash`、`Debug`。
   - 字面量构造协议：`impl LiteralInto<.cstr, CStr> for str`（在编译期校验无内嵌 NUL 并追加 `\0` 写入 `.rodata`！）。
2. **`OsStr` / `OsString<.A, .allocator>`**：
   - 平台原生潜在非 UTF-8 文本表示（Unix 为任意 `[u8]`、Windows 为 WTF-8、WASI 为 UTF-8）。
   - `OsStr`（`Copy`, `Clone`, `Eq`, `Ord`, `Hash`, `Debug`）：`from_str(s: str) -> OsStr`（零开销）、`to_str(self) -> ?str`、`len(self) -> usize`、`is_empty(self) -> bool`、`as_encoded_bytes(self) -> Slice<u8>`、`unsafe from_encoded_bytes_unchecked(bytes: Slice<u8>) -> OsStr`、`std.os.unix.ffi.OsStrExt`（`from_bytes` / `as_bytes`），以及带方法级分配器子句的 `to_string_lossy(self) -> !AllocErr Cow<str, .A = A, .allocator = allocator> where .A: Type = DefaultAllocator, requires A :- Allocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref`。
   - `OsString`：`new()`、`with_capacity(cap: usize)`、`from_str(s: str)`、`push(*self, s: OsStr) -> !AllocErr void`、`push_str(*self, s: str) -> !AllocErr void`、`clear(*self)`、`capacity(*self)`、`reserve(*self, add: usize)`、`shrink_to_fit(*self)`、`as_os_str(*self) -> OsStr`、`into_string(self) -> !Self String<.A = A, .allocator = alloc>`（UTF-8 校验失败时原样交回 `error self`！）、`into_encoded_bytes(self)`、`TryClone`、`Drop`、`Eq`、`Ord`、`Hash`、`Debug`。

#### 3.3.3 `std.fmt` 完整演进（与 `04` `builtin.fmt` 协同）

现有 [parse.fl](../../library/std/fmt_impl/parse.fl) 与 [arguments.fl](../../library/std/fmt_impl/arguments.fl) 已采用 Flurry 属性格式化语法 `{key: value}`（如 `{format: .hex, align_to: .right, len: 12, placeholder: '0'}` 与 `{format: .debug}`），通过 `FormatPlan.parse` + `Parse(format)` 在编译期生成变参域检查，不保留旧版 `{:?}` / `{:x}`。结合 [04-builtin-types-math-iterators-fmt.md](./04-builtin-types-math-iterators-fmt.md) `WL-04-21` 的跨包设计，Wishlist 需补齐以下 5 项缺口：
1. **`print` / `eprint` 变参格式化**：当前 `print(comptime text: str)` 仅接受无格式单字符串，须升级为与 `println` 对称：
   ```flurry
   pub fn print(comptime format: str, ...varargs: defer Parse(format)) -> !IoErr void {
       var output = stdout();
       write_fmt_tuple(output.ref, format, varargs)
   }

   pub fn eprint(comptime format: str, ...varargs: defer Parse(format)) -> !IoErr void {
       var output = stderr();
       write_fmt_tuple(output.ref, format, varargs)
   }
   ```
2. **两相同名 `ComptimeDisplay` / `ComptimeDebug` 与编译期浮点 `Display`**：
   - 依 `AGENTS.md` §3 及 `04` `WL-04-21` 纠偏：`builtin.fmt.Display`（方法 `fmt`）保留纯运行期实现；`meta.fmt` 独立定义 `pure comptime trait ComptimeDisplay { comptime fn fmt(self, .format: str = "") -> str; }` 与 `pure comptime trait ComptimeDebug { comptime fn fmt(self) -> str; }`（与运行期 `Debug` 统一使用同名 `fmt`，废止旧 `comptime_fmt` / `write_debug` 后缀命名，且仅接受 `self` 值引用）。
   - `std.fmt.comptime_format` 对 `{format: .debug}` 分支取消 `assumes false;`，改为分派至 `ComptimeDebug.fmt`，并接通 `f16`/`bf16`/`f32`/`f64` 的 `ComptimeDisplay.fmt`。
3. **结构化 `Formatter` 与 `Debug*` 构造器（`builtin.fmt` 定义，`std.fmt` 重导出）**：
   - `FormatErr`、`Formatter`、`DebugStruct`、`DebugList`、`DebugTuple`、`DebugSet`、`DebugMap` 统一由 `builtin.fmt` 提供权威定义并由 `std.fmt` 重导出（`pub use builtin.fmt.{FormatErr, Formatter, DebugStruct, DebugList, DebugTuple, DebugSet, DebugMap, Display, Debug, ByteWriter}`）：
   ```flurry
   -- Debug* 构造器均由 Formatter 创建，零堆分配，返回 !FormatErr
   pub struct Formatter {
       is_alternate: bool,
       width: ?usize,
       precision: ?usize,
       fill: char,
       sign_plus: bool,
   }

   pub struct DebugStruct {
       state_ptr: usize,
   }

   impl DebugStruct {
       pub fn field(*self, name: str, value: *V) -> *Self where V :- Debug;
       pub fn field_with(*self, name: str, value_fn: %c fn(*Formatter) -> !FormatErr void) -> *Self
       where
           c: Type.vars.Closure;
       pub fn finish(*self) -> !FormatErr void;
       pub fn finish_non_exhaustive(*self) -> !FormatErr void;
   }
   ```
   （`DebugList`、`DebugTuple`、`DebugSet`、`DebugMap` 由 `Formatter.debug_*` 构造，支持 `entry`/`key`/`value`/`finish`；另在 `04` §3.5.1 & §5 协调 `Debug.write_debug(*ByteWriter)` 向 `Debug.fmt(*Formatter)` 的渐进迁移或统一适配层。）
4. **标准库错误与指针格式化**：为 `*T` 实现 `Debug`（十六进制地址如 `0x7fff...`），并为 `AllocErr`、`ReallocErr`、`IoErr`、`FsErr`、`PathErr`、`TimeErr`、`VarErr`、`ProcessErr`、`Utf8Err`、`FromUtf8Err`、`FormatErr` 全部补齐人类可读的 `Display` 实现。
5. **未知选项键诊断严格化**：`parse_int_options` / `parse_float_options` / `parse_text_options` 现阶段在运行时静默忽略未知选项键，应在 `FormatPlan`（结合目标类型或通用允许键集）阶段给出明确的编译期拒绝诊断。

---

### 3.4 `std.io`、`std.path` 与 `std.fs`：流式 I/O、路径与文件系统

#### 3.4.1 `std.io`：全面错误分类、协议补全与流组合子

1. **`IoErrKind` 与 `IoErr`**（扩展 [errors.fl](../../library/std/io_impl/errors.fl)，错误变体严格使用 `PascalCase`）：
   ```flurry
   pub enum IoErrKind {
       not_found,
       permission_denied,
       connection_refused,
       connection_reset,
       connection_aborted,
       not_connected,
       addr_in_use,
       addr_not_available,
       broken_pipe,
       already_exists,
       would_block,
       invalid_input,
       invalid_data,
       timed_out,
       write_zero,
       interrupted,
       unsupported,
       unexpected_eof,
       out_of_memory,
       is_a_directory,
       not_a_directory,
       directory_not_empty,
       read_only_filesystem,
       resource_busy,
       deadlock,
       crosses_devices,
       too_many_links,
       invalid_filename,
       storage_full,
       not_seekable,
       quota_exceeded,
       file_too_large,
       other,
   }

   pub enum IoErr {
       Os(i64),
       Kind(IoErrKind),
       WriteZero,
       UnexpectedEof,
       InvalidInput,
       InvalidData,
       Unsupported,
   }

   impl IoErr {
       pub pure fn from_raw_os_error(code: i64) -> Self {
           IoErr.Os(code)
       }

       pub pure fn raw_os_error(self) -> ?i64 {
           self match {
               .Os(code) => code,
               _ => null,
           }
       }

       pub pure fn kind(self) -> IoErrKind {
           -- 完整表驱动映射到各平台 std.os 错误码，此处示意典型映射骨架
           self match {
               .Os(code) => {
                   if code == 2 { IoErrKind.not_found }
                   else if code == 13 or code == 1 { IoErrKind.permission_denied }
                   else if code == 17 { IoErrKind.already_exists }
                   else if code == 4 { IoErrKind.interrupted }
                   else if code == 11 { IoErrKind.would_block }
                   else if code == 32 { IoErrKind.broken_pipe }
                   else { IoErrKind.other }
               },
               .Kind(k) => k,
               .WriteZero => IoErrKind.write_zero,
               .UnexpectedEof => IoErrKind.unexpected_eof,
               .InvalidInput => IoErrKind.invalid_input,
               .InvalidData => IoErrKind.invalid_data,
               .Unsupported => IoErrKind.unsupported,
           }
       }
   }
   ```
2. **补充核心 I/O 协议**：
   - **`Reader` 补充安全切片方法**：除已有 `unsafe read_raw(*self, dst: usize, len: usize)` 与 `read_byte` 外，增加 `fn read(*self, buf: MutSlice<u8>) -> !Err usize`、`fn read_exact_slice(*self, buf: MutSlice<u8>) -> ![Err, IoErr] void`、`fn bytes(self) -> BytesIter<Self>`、`fn take(self, limit: u64) -> Take<Self>`、`fn chain(self, next: R2) -> Chain<Self, R2> where R2 :- Reader, requires R2.as(Reader).Err == Err`。
   - **`Write` 补充安全切片与刷新方法**：增加 `fn write(*self, buf: Slice<u8>) -> !Err usize`、`fn write_all_slice(*self, buf: Slice<u8>) -> !Err void`、`fn write_vectored(*self, bufs: Slice<IoSlice>) -> !Err usize`（默认写第一个非空块）。
   - **`BufRead`（`trait BufRead :- Reader`）**：
     ```flurry
     pub trait BufRead :- Reader {
         fn fill_buf(*self) -> !Err Slice<u8>;
         fn consume(*self, amt: usize);

         fn has_data_left(*self) -> !Err bool {
             let buf = self.fill_buf()!;
             not buf.is_empty()
         }

         fn skip_until(*self, delim: u8) -> !Err usize {
             var read_total: usize = 0;
             while true {
                 let avail = self.fill_buf()!;
                 if avail.is_empty() {
                     return read_total;
                 }
                 var i: usize = 0;
                 while i < avail.len() {
                     let byte = avail.get(i) match {
                         b? => b,
                         null => break,
                     }
                     if byte == delim {
                         let used = i + 1;
                         self.consume(used);
                         return read_total + used;
                     }
                     i = i + 1;
                 }
                 let chunk_len = avail.len();
                 self.consume(chunk_len);
                 read_total = read_total + chunk_len;
             }
             read_total
         }

         fn read_until(
             *self,
             delim: u8,
             buf: *Vec<u8, .A = A, .allocator = a>,
         ) -> ![Err, AllocErr] usize
         where
             A: Type,
             requires A :- Allocator,
             quote a: lift *A,
         {
             var total: usize = 0;
             while true {
                 let avail = self.fill_buf()!;
                 if avail.is_empty() {
                     return total;
                 }
                 var i: usize = 0;
                 var found = false;
                 while i < avail.len() {
                     let b = avail.get(i) match {
                         v? => v,
                         null => break,
                     }
                     buf.push(b)!;
                     i = i + 1;
                     if b == delim {
                         found = true;
                         break;
                     }
                 }
                 self.consume(i);
                 total = total + i;
                 if found {
                     return total;
                 }
             }
             total
         }

         fn read_line(
             *self,
             buf: *String<.A = A, .allocator = a>,
         ) -> ![Err, AllocErr, Utf8Err] usize
         where
             A: Type,
             requires A :- Allocator,
             quote a: lift *A,
         {
             var raw = Vec<u8, .A = A, .allocator = a>.new();
             let n = self.read_until(10, raw.ref)!;
             let text = String<.A = A, .allocator = a>.from_utf8(raw)! {
                 catch e => return error e.utf8_error(),
             }
             buf.push_str(text.as_str())!;
             n
         }
     }
     ```
     为 `BufReader<R, .A, .allocator>` 与 `Cursor<T>` 实现 `BufRead`（现有 `BufReader` 的 `refill`/`consume`/`read_until`/`read_line` 对接该 trait）。
   - **`Seek` 与 `SeekFrom`**：
     ```flurry
     pub enum SeekFrom {
         start(u64),
         end(i64),
         current(i64),
     }

     pub trait Seek {
         assoc Err: Type;

         fn seek(*self, pos: SeekFrom) -> !Err u64;

         fn rewind(*self) -> !Err void {
             let _ = self.seek(SeekFrom.start(0))!;
         }

         fn stream_position(*self) -> !Err u64 {
             self.seek(SeekFrom.current(0))
         }
     }
     ```
     为 `File`、`Cursor<T>`、`BufReader<R :- Seek>`（`seek` 时清掉或复用内部预读缓冲）、`BufWriter<W :- Write + Seek>`（`seek` 前先 `flush_buf()`）实现 `Seek`。
3. **缓冲、句柄与流组合子**：
   ```flurry
   pub struct LineWriter
   where
       W :- Write,
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       inner: BufWriter<W, .A = A, .allocator = allocator>,
   }

   pub struct Cursor where T {
       inner: T,
       pos: u64,
   }

   pub struct Take where R :- Reader {
       inner: R,
       limit: u64,
   }

   pub struct Chain
   where
       R1 :- Reader,
       R2 :- Reader,
       requires R1.as(Reader).Err == R2.as(Reader).Err,
   {
       first: R1,
       second: R2,
       done_first: bool,
   }

   pub struct Empty {}
   pub struct Sink {}
   pub struct Repeat {
       byte: u8,
   }
   ```
   - **`LineWriter<W, .A, .allocator>`**：封装 `BufWriter<W, .A, .allocator>`，每次遇到 `\n`（字节 `10`）即冲刷整行；`Drop` 自动冲刷剩余字节。
   - **`Cursor<T>` 泛化**：将当前绑定到 `Vec<u8>` 的 `Cursor` 泛化为 `Cursor<T>`（`T` 可以是只读 `Slice<u8>`（零分配只读游标 + `BufRead` + `Seek`）、`MutSlice<u8>` 或 `Vec<u8, .A, .allocator>`（支持 `Write` + `ByteWriter` + `Seek`））。
   - **标准流 `Stdout` / `Stderr`**：当前 `stdout()` / `stderr()` 返回无锁 `Copy` `Writer`（多线程写会交错）。引入可锁定的 `Stdout` / `Stderr`（含 `lock(*self) -> StdoutLock` / `StderrLock`，依赖 Wave 2 `std.sync.Mutex`），保留轻量 `Writer` 或统一导出。
   - **工具函数与零开销流适配器**：
     - `pub fn copy(reader: *R, writer: *W) -> ![R.as(Reader).Err, W.as(Write).Err] u64 where R :- Reader, W :- Write`（栈内 8 KiB 缓冲循环搬运，遇 `Interrupted` 重试）
     - `pub fn copy_buf(reader: *R, writer: *W) -> ![R.as(BufRead).Err, W.as(Write).Err] u64 where R :- BufRead, W :- Write`
     - `Empty`（`Reader + BufRead + Seek + Copy`）、`Sink`（`Write + ByteWriter + Copy`）、`Repeat`（`Reader + Copy`）、`IoSlice` / `IoSliceMut`。

#### 3.4.2 `std.path`：跨平台路径代数

- **`Prefix` 与 `Component`**（普通枚举使用 `snake_case` 变体）：
  ```flurry
  pub enum Prefix {
      verbatim(OsStr),
      verbatim_unc(OsStr, OsStr),
      verbatim_disk(u8),
      device_ns(OsStr),
      unc(OsStr, OsStr),
      disk(u8),
  }

  pub enum Component {
      prefix(Prefix),
      root_dir,
      cur_dir,
      parent_dir,
      normal(OsStr),
  }
  ```
- **`PathErr`**：
  ```flurry
  pub enum PathErr {
      StripPrefixFailed,
      InteriorNul,
      InvalidEncoding,
      PathTooLong,
  }
  ```
- **`Path`**（借用视图，内部包装 `OsStr`；`Copy`, `Clone`, `Eq`（按组件规范等价比较！）, `Ord`, `Hash`, `Debug`）：
  - 无分配只读查询：`new(s: str) -> Path`、`from_os_str(s: OsStr) -> Path`、`as_os_str(self) -> OsStr`、`to_str(self) -> ?str`、`is_absolute(self) -> bool`、`is_relative(self) -> bool`、`has_root(self) -> bool`、`parent(self) -> ?Path`、`ancestors(self) -> Ancestors`、`file_name(self) -> ?OsStr`、`file_stem(self) -> ?OsStr`、`file_prefix(self) -> ?OsStr`、`extension(self) -> ?OsStr`、`strip_prefix(self, base: Path) -> !PathErr Path`、`starts_with(self, base: Path) -> bool`、`ends_with(self, child: Path) -> bool`、`components(self) -> Components`、`iter(self)`、`display(self)`。
  - **方法级分配器子句**（因 `Path` 本身是零分配 `Copy` 视图，所有构造拥有型结果的方法均在方法签名声明 `.A` 与 `quote .allocator`）：
    ```flurry
    impl Path {
        pub fn to_string_lossy(self) -> !AllocErr Cow<str, .A = A, .allocator = allocator>
        where
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

        pub fn to_path_buf(self) -> !AllocErr PathBuf<.A = A, .allocator = allocator>
        where
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

        pub fn join(self, path: Path) -> !AllocErr PathBuf<.A = A, .allocator = allocator>
        where
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

        pub fn with_file_name(self, name: OsStr) -> !AllocErr PathBuf<.A = A, .allocator = allocator>
        where
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

        pub fn with_extension(self, ext: OsStr) -> !AllocErr PathBuf<.A = A, .allocator = allocator>
        where
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
    }
    ```
- **`PathBuf<.A = DefaultAllocator, requires A :- Allocator, quote .allocator = DefaultAllocator.GLOBAL.ref>`**：
  ```flurry
  pub struct PathBuf
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
  {
      inner: OsString<.A = A, .allocator = allocator>,
  }
  ```
  - 方法：`new`、`with_capacity`、`from_os_string`、`from_str`、`as_path(*self) -> Path`、`push(*self, path: Path) -> !AllocErr void`、`pop(*self) -> bool`、`set_file_name(*self, file_name: OsStr) -> !AllocErr void`、`set_extension(*self, extension: OsStr) -> !AllocErr bool`、`clear(*self)`、`capacity(*self) -> usize`、`reserve(*self, additional: usize) -> !AllocErr void`、`shrink_to_fit(*self) -> !AllocErr void`、`into_os_string(self) -> OsString<.A = A, .allocator = allocator>`、`TryClone`、`Drop`、`Eq`、`Ord`、`Hash`、`Debug`。
- **分隔符常量**：`MAIN_SEPARATOR: char`（Unix `'/'`，Windows `'\\'`）、`MAIN_SEPARATOR_STR: str`、`is_separator(c: char) -> bool`（由 `build.target.os` 在编译期确定）。

#### 3.4.3 `std.fs`：运行时路径文件系统与元数据

当前 [file.fl](../../library/std/fs_impl/file.fl) 仅接收 `comptime path: str`。Wishlist 将全系 `fs` 接口升级为**运行时 `Path`（与 `str`/`*String` 经 `AsPath` 适配）**：

```flurry
pub enum FsErr {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    IsADirectory,
    NotADirectory,
    DirectoryNotEmpty,
    InvalidPath,
    TooManySymlinks,
    NameTooLong,
    CrossesDevices,
    ReadOnlyFilesystem,
    StorageFull,
    NotSupported,
    Io(IoErr),
}

pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    mode: u32,
    custom_flags: i32,
}

pub struct File {
    raw_handle: i64,
}

pub struct DirEntry
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    full_path: PathBuf<.A = A, .allocator = allocator>,
    name_offset: usize,
}

pub struct ReadDir
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    dir_handle: usize,
    base_path: PathBuf<.A = A, .allocator = allocator>,
}
```
- **`FsErr` 转换**：提供 `FsErr.from_io(err: IoErr) -> Self` 与 `into_io(self) -> IoErr` 转换；或直接返回 `!IoErr` 并在 `IoErr.kind()` 区分（归并决策见 §5，确保所有错误路径符合 `PascalCaseErr` + `PascalCase` 变体规范）。
- **`OpenOptions` 与 `File` 完整操作**：
  - `OpenOptions`：`new() -> OpenOptions`、`read(*self, flag: bool) -> *Self`、`write(*self, flag: bool) -> *Self`、`append(*self, flag: bool) -> *Self`、`truncate(*self, flag: bool) -> *Self`、`create(*self, flag: bool) -> *Self`、`create_new(*self, flag: bool) -> *Self`、`open(*self, path: Path) -> !IoErr File`（Unix 扩展 `mode(*self, mode: u32)`、`custom_flags(*self, flags: i32)`）。
  - `File`：`open(path: Path) -> !IoErr File`、`create(path: Path) -> !IoErr File`、`create_new(path: Path) -> !IoErr File`、`options() -> OpenOptions`、`sync_all(*self) -> !IoErr void`、`sync_data(*self) -> !IoErr void`、`set_len(*self, size: u64) -> !IoErr void`、`metadata(*self) -> !IoErr Metadata`、`try_clone(*self) -> !IoErr File`（`fcntl(F_DUPFD_CLOEXEC)` / `DuplicateHandle`）、`set_permissions(*self, perm: Permissions) -> !IoErr void`、`close(*self) -> !IoErr void`、`Drop`、`Reader`、`Write`、`ByteWriter`、`Seek`。
- **元数据与目录遍历**：
  - `FileType`（`is_dir`, `is_file`, `is_symlink`, `Copy`, `Eq`, `Debug`）、`Permissions`（`readonly`, `set_readonly`, Unix `mode` / `set_mode`）、`Metadata`（`file_type`, `is_dir`, `is_file`, `is_symlink`, `len`, `permissions`, `modified`, `accessed`, `created`, Unix `uid`/`gid`/`ino`/`dev`/`nlink`）。
  - `DirEntry`（`path(*self) -> Path`、`to_path_buf(*self) -> !AllocErr PathBuf<.A = A, .allocator = allocator>`、`file_name(*self) -> OsStr`、`metadata(*self) -> !IoErr Metadata`、`file_type(*self) -> !IoErr FileType`）与 `ReadDir`（实现 `Iterator<Item = !IoErr DirEntry<.A = A, .allocator = allocator>>` 与 `Drop` 自动调用 `closedir`/`FindClose`）。
- **独立模块函数（全部支持运行时 `Path`，其中分配返回值函数均显式携带 `.A` / `quote .allocator` 子句）**：
  ```flurry
  pub fn read(
      path: Path,
  ) -> ![IoErr, AllocErr] Vec<u8, .A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn read_to_string(
      path: Path,
  ) -> ![IoErr, AllocErr, Utf8Err] String<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn canonicalize(
      path: Path,
  ) -> ![IoErr, AllocErr] PathBuf<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn read_link(
      path: Path,
  ) -> ![IoErr, AllocErr] PathBuf<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
  ```
  - 无堆分配辅助函数：`write_bytes(path: Path, contents: Slice<u8>) -> !IoErr void`、`write_str(path: Path, contents: str) -> !IoErr void`、`create_dir(path: Path) -> !IoErr void`、`create_dir_all(path: Path) -> !IoErr void`、`remove_file(path: Path) -> !IoErr void`、`remove_dir(path: Path) -> !IoErr void`、`remove_dir_all(path: Path) -> !IoErr void`、`rename(from: Path, to: Path) -> !IoErr void`、`copy(from: Path, to: Path) -> !IoErr u64`、`hard_link(original: Path, link: Path) -> !IoErr void`、`symlink_file` / `symlink_dir` / `symlink`、`metadata(path: Path) -> !IoErr Metadata`、`symlink_metadata(path: Path) -> !IoErr Metadata`、`exists(path: Path) -> bool`、`set_permissions(path: Path, perm: Permissions) -> !IoErr void`。

---

### 3.5 `std.time`、`std.env`、`std.process`：时钟、环境与进程契约

#### 3.5.1 `std.time`

保留 [duration.fl](../../library/std/time_impl/duration.fl) 全部现有定义，补全以下内容：
- **`Duration` 扩展**：常量 `MAX`、`SECOND`、`MILLISECOND`、`MICROSECOND`、`NANOSECOND`；浮点换算 `from_secs_f64` / `from_secs_f32`（需校验非负、非 NaN、非无穷且不溢出 `u64` 秒，越界返回 `!TimeErr` 或 `?Duration`）、`as_secs_f64` / `as_secs_f32`、`div_duration_f64` / `div_duration_f32`；`saturating_mul`；实现 `Add<Duration>`、`Sub<Duration>`、`Mul<u32>`、`Div<u32>` 与 `Display`。
- **`TimeErr`**：
  ```flurry
  pub enum TimeErr {
      Overflow,
      NonMonotonic(Duration),
      InvalidFloatDuration,
      ClockUnavailable(i64),
  }
  ```
- **`Instant`**（单调时钟，`CLOCK_MONOTONIC` / `mach_continuous_time` / `QueryPerformanceCounter`）：
  `now() -> Instant`、`duration_since(*self, earlier: Instant) -> Duration`、`checked_duration_since(*self, earlier: Instant) -> ?Duration`、`saturating_duration_since(*self, earlier: Instant) -> Duration`、`elapsed(*self) -> Duration`、`checked_add(*self, duration: Duration) -> ?Instant`、`checked_sub(*self, duration: Duration) -> ?Instant`、`Copy`、`Clone`、`Eq`、`Ord`、`Hash`、`Debug`。
- **`SystemTime`**（墙上时钟，`CLOCK_REALTIME` / `GetSystemTimePreciseAsFileTime`）：
  `pub const UNIX_EPOCH: SystemTime`、`now() -> SystemTime`、`duration_since(*self, earlier: SystemTime) -> !TimeErr Duration`（若发生 NTP 回拨返回 `error TimeErr.NonMonotonic(diff)`）、`elapsed(*self) -> !TimeErr Duration`、`checked_add(*self, duration: Duration) -> ?SystemTime`、`checked_sub(*self, duration: Duration) -> ?SystemTime`、`Copy`、`Clone`、`Eq`、`Ord`、`Hash`、`Debug`。
- **`sleep(dur: Duration)`**：同步挂起当前 OS 线程至少 `dur` 时长（内部自动重试 `EINTR` 并扣除已睡时间）。注：在异步 `async effect` 调度器内禁止直接调用阻塞式 `std.time.sleep`，应使用 Wave 2 的异步定时器效应。

#### 3.5.2 `std.env`

- **错误类型**：
  ```flurry
  pub enum VarErr {
      NotPresent,
      NotUnicode,
      InvalidKey,
  }

  pub enum JoinPathsErr {
      ContainsSeparator(usize),
      AllocFailed(AllocErr),
  }
  ```
- **核心接口（全部分配返回值与集合函数均声明标准 `.A` / `quote .allocator` 子句）**：
  ```flurry
  pub fn args() -> !AllocErr Vec<String<.A = A, .allocator = allocator>, .A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn args_os() -> !AllocErr Vec<OsString<.A = A, .allocator = allocator>, .A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn var(key: OsStr) -> ![VarErr, AllocErr] String<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn var_os(key: OsStr) -> !AllocErr ?OsString<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn vars() -> !AllocErr Vec<Tuple<String<.A = A, .allocator = allocator>, String<.A = A, .allocator = allocator>>, .A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn vars_os() -> !AllocErr Vec<Tuple<OsString<.A = A, .allocator = allocator>, OsString<.A = A, .allocator = allocator>>, .A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn current_dir() -> ![IoErr, AllocErr] PathBuf<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn current_exe() -> ![IoErr, AllocErr] PathBuf<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn temp_dir() -> !AllocErr PathBuf<.A = A, .allocator = allocator>
  where
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

  pub fn join_paths(paths: I) -> ![AllocErr, JoinPathsErr] OsString<.A = A, .allocator = allocator>
  where
      I :- Iterator,
      requires I.as(Iterator).Item == OsStr,
      .A: Type = DefaultAllocator,
      requires A :- Allocator,
      quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
  ```
  - **线程安全契约**：`unsafe fn set_var(key: OsStr, value: OsStr) -> !VarErr void` 与 `unsafe fn remove_var(key: OsStr) -> !VarErr void` **必须标注为 `unsafe`**，因为 POSIX `setenv`/`unsetenv` 非线程安全，与并发 `getenv` 存在数据竞争。
  - 无分配路径操作：`set_current_dir(path: Path) -> !IoErr void`、`split_paths(unparsed: OsStr) -> SplitPaths`。
- **`std.env.consts`**：由编译期 `build.target` 派生以下 `pub const` 字符串常量：
  `OS`（`"linux"` / `"macos"` / `"windows"` / `"freebsd"` / `"wasi"`）、`FAMILY`（`"unix"` / `"windows"` / `"wasm"`）、`ARCH`（`"x86_64"` / `"aarch64"` / `"riscv64"` / `"wasm32"`）、`EXE_EXTENSION`（`""` / `"exe"` / `"wasm"`）、`EXE_SUFFIX`（`""` / `".exe"` / `".wasm"`）、`DLL_PREFIX`（`"lib"` / `""`）、`DLL_SUFFIX`（`".so"` / `".dylib"` / `".dll"`）、`DLL_EXTENSION`（`"so"` / `"dylib"` / `"dll"`）。

#### 3.5.3 `std.process` 与 `panic` 终止/清理契约（显式 Blocker）

1. **进程管理与子进程管道**（扩展 [process.fl](../../library/std/process_impl/process.fl)）：
   ```flurry
   pub enum ProcessErr {
       SpawnFailed(IoErr),
       WaitFailed(IoErr),
       PipeFailed(IoErr),
       InvalidCommand,
       AlreadyWaited,
       KilledBySignal(i32),
   }

   pub enum Stdio {
       inherit,
       piped,
       null_device,
       from_file(File),
   }

   pub struct ExitStatus {
       raw_code: i32,
   }

   pub struct Output
   where
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       status: ExitStatus,
       stdout: Vec<u8, .A = A, .allocator = allocator>,
       stderr: Vec<u8, .A = A, .allocator = allocator>,
   }

   pub struct Child {
       raw_handle_or_pid: i64,
       stdin: ?ChildStdin,
       stdout: ?ChildStdout,
       stderr: ?ChildStderr,
       waited: bool,
   }

   pub struct Command
   where
       .A: Type = DefaultAllocator,
       requires A :- Allocator,
       quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
   {
       program: OsString<.A = A, .allocator = allocator>,
       args: Vec<OsString<.A = A, .allocator = allocator>, .A = A, .allocator = allocator>,
       env_clear_flag: bool,
       current_dir_opt: ?PathBuf<.A = A, .allocator = allocator>,
       stdin_cfg: Stdio,
       stdout_cfg: Stdio,
       stderr_cfg: Stdio,
   }
   ```
   - `ExitStatus`：`success(self) -> bool`、`code(self) -> ?i32`（被信号终止返回 `null`）、`signal(self) -> ?i32`（Unix）、`Eq`、`Copy`、`Clone`、`Debug`、`Display`。
   - `ChildStdin`（`Write + ByteWriter + Drop`）、`ChildStdout`（`Reader + Drop`）、`ChildStderr`（`Reader + Drop`）。
   - `Child`（`Once`，`Drop` 自动关闭剩余管道，并规定是否自动 `wait` 防僵尸进程的策略）：`id(*self) -> u32`、`kill(*self) -> !IoErr void`、`wait(*self) -> !IoErr ExitStatus`、`try_wait(*self) -> !IoErr ?ExitStatus`，以及方法级分配器签名（因 `Child` 自身不带 `.A`）：
     ```flurry
     impl Child {
         pub fn wait_with_output(
             self,
         ) -> ![IoErr, AllocErr] Output<.A = A, .allocator = allocator>
         where
             .A: Type = DefaultAllocator,
             requires A :- Allocator,
             quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
     }
     ```
     （注：同步读取双管道必须用 `poll`/`epoll`/多线程以防缓冲区满死锁。）
   - `Command`（`impl Command<.A = A, .allocator = alloc> where A: Type, requires A :- Allocator, quote alloc: lift *A`）：`new(program: OsStr) -> !AllocErr Self`、`arg(*self, arg: OsStr) -> !AllocErr *Self`、`args`、`env`、`env_remove`、`env_clear`、`current_dir(*self, dir: Path) -> !AllocErr *Self`、`stdin(*self, cfg: Stdio) -> *Self`、`stdout(*self, cfg: Stdio) -> *Self`、`stderr(*self, cfg: Stdio) -> *Self`、`spawn(*self) -> !IoErr Child`、`status(*self) -> !IoErr ExitStatus`、`output(*self) -> ![IoErr, AllocErr] Output<.A = A, .allocator = alloc>`。
   - `id() -> u32`（已落地）、`exit(code: i32) -> NoReturn`（立即终止进程，不执行局部栈 `Drop`，视约定决定是否冲刷 stdio 缓冲）、`abort() -> NoReturn`（立即异常中止进程 `SIGABRT`/`@llvm.trap()`，绝不运行 `Drop`）。

2. **🔴 显式 Blocker：`std.process.panic` 终止与清理契约**：
   依据 [library/README.md:393-395](../../library/README.md)、[development/library-prototypes/README.md:10](../../development/library-prototypes/README.md) 与 [docs/algebraic-effect-and-delimited-continuation/effect-abi.md:195](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)，`panic` 的运行时实现及 `OptionalUnwrap.unwrap` / `ErrorResultUnwrap.unwrap` / 容器越界防呆路径目前被以下四个未定契约直接阻塞：
   - **(a) 终止模式（Immediate Abort vs Controlled Unwind）**：
     - 若选择 **Abort 模式**：`panic` 打印诊断与调用栈后调用 `abort()`，不运行任何调用栈上的 `Drop`（同当前 `@flurry_check_failure()` / `@llvm.trap()` 契约，但需要标准化的错误输出与位置信息）；
     - 若选择 **Controlled Unwind 模式**：`panic` 利用 Flurry 控制后端的**隐藏中止通道**（见 `effect-abi.md`）沿调用栈逐帧执行剩余仿射资源的 `Drop`（逆序清理），并在遇到 `^attrs.abi(.c)` C 帧边界时按约定 trap（防止异常展开跨过 C ABI）。
   - **(b) 可捕获性（Task/Thread Isolation）**：多线程或 `async` 任务中单个子任务发生 `panic` 时，是立刻杀掉整个 OS 进程，还是可由 `JoinHandle` / `catch_unwind` 边界接住并转化为任务失败？
   - **(c) 对 `NoReturn`、`pure fn`（P4/P5）与 comptime 的影响**：
     - `panic(msg: str) -> NoReturn` 必须保持**空效应行**（否则任何 `unwrap` 与数组/切片索引都会把 `#Panic` 传染到全程序签名）。
     - 依据 [compiler/luna/src/host/llvm/attributes.rs:53-56](../../compiler/luna/src/host/llvm/attributes.rs)，出现 `@llvm.trap` 会剥夺函数的 `nounwind` / `willreturn` / `memory(none)` / `speculatable` 属性；需明确 `pure fn`（如 `Slice.get` vs `Slice.unwrap`）是否允许调用 `panic`。
     - 在 `comptime` 求值中触发 `panic` 必须作为明确的编译期失败诊断（`meta.compilation.err` 语义），不可在编译期 VM 中吞掉或延迟到运行时。
   - **(d) Hook 与调用位置元信息（新增前置 Wishlist 需求）**：当前 `docs/comptime/official-attribute-keys.md`、`library/attrs/` 与 `05` 尚未登记调用点位置捕获属性及来源位置类型；需同步决定是否新增 `__flurry_caller_location`（包装为 `^attrs.*` 并配合 `meta.ast.SourceSpan` / 轻量只读 `CallerLocation` 描述符）供 `panic` 默认注入源码位置，以及是否支持全局 panic hook（`static` 同步注册）。

---

### 3.6 `std.ffi`、`std.os`/`platform` 与 `std.simd`：外部 ABI、多平台抽象与可移植向量

#### 3.6.1 `std.ffi` 完整 C ABI 规范（无 `extern` 关键字）

依据 `AGENTS.md` §5、[docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md) 与 [docs/algebraic-effect-and-delimited-continuation/effect-abi.md](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)，Flurry **不引入 `extern` 关键字**，全部 C ABI 导入/导出与调用约定通过 `^attrs.abi(...)` 与无主体函数声明表达：

1. **C ABI 类型映射表**（通过 `comptime` 读取 `build.target` 自动绑定正确目标位宽）：
   ```flurry
   pub pure comptime fn CChar() -> Type {
       if build.target.os == .linux and (build.target.arch == .aarch64 or build.target.arch == .arm or build.target.arch == .riscv64) {
           u8
       } else {
           i8
       }
   }

   pub pure comptime fn CLong() -> Type {
       if build.target.os == .windows or build.target.pointer_width == 32 {
           i32
       } else {
           i64
       }
   }

   pub pure comptime fn CULong() -> Type {
       if build.target.os == .windows or build.target.pointer_width == 32 {
           u32
       } else {
           u64
       }
   }

   pub typealias c_schar = i8;
   pub typealias c_uchar = u8;
   pub typealias c_char = CChar();
   pub typealias c_short = i16;
   pub typealias c_ushort = u16;
   pub typealias c_int = i32;
   pub typealias c_uint = u32;
   pub typealias c_long = CLong();
   pub typealias c_ulong = CULong();
   pub typealias c_longlong = i64;
   pub typealias c_ulonglong = u64;
   pub typealias c_size_t = usize;
   pub typealias c_ssize_t = isize;
   pub typealias c_ptrdiff_t = isize;
   pub typealias c_intptr_t = isize;
   pub typealias c_uintptr_t = usize;
   pub typealias c_float = f32;
   pub typealias c_double = f64;
   pub enum c_void {} -- 零变体不可构造类型，仅允许构造 *c_void 指针，禁止按值实例化或解引用
   ```
   - `VaList`：平台相关 C 可变参数状态结构体（对应 LLVM `va_list`、`@llvm.va_start`、`@llvm.va_end`、`@llvm.va_copy` 与 `va_arg`；仿射 `Once`，不可 `Copy`）。
2. **`^attrs.abi(...)` 约定扩展与外部符号导入**：
   - 支持规范约定符号：
     - `.c`：目标默认 C 调用约定（已接通导出路径 `control.rs:184`）
     - `.system`：目标系统 API 默认约定（Windows x86 上对应 `stdcall`，Win64 对应 `win64`，Unix/macOS/Linux 对应 `c`）
     - `.win64`：x86_64 Microsoft ABI（`win64cc`）
     - `.sysv64`：x86_64 System V ABI（`x86_64_sysvcc`）
     - `.aapcs`：ARM AAPCS（`arm_aapcscc`）
   - **无主体外部 C 声明**（需在 `elaboration` 与 LLVM 后端打通 `ItemKind.FnDeclaration` 发射 `declare`）：
     ```flurry
     ^attrs.abi(.c)
     ^attrs.ffi.link(.lib = "c", .symbol = "strlen") -- 需在 official-attribute-keys.md 登记 __flurry_ffi_link
     pub unsafe fn strlen(s: *c_char) -> c_size_t;

     -- 导出给 C 调用的 Flurry 函数（须满足空效应行、无隐式 env_parameters）
     ^attrs.abi(.c)
     pub fn flurry_callback(x: c_int) -> c_int {
         x * 2
     }
     ```
   - **拒绝例（C ABI 边界违规）**：
     ```flurry
     -- 拒绝例 1：带效应行的函数禁止直接跨 C ABI 边界（effect-abi.md）
     effect read_cfg() -> c_int;
     ^attrs.abi(.c)
     pub fn bad_export() -> #read_cfg c_int {
         read_cfg()#
     }

     -- 拒绝例 2：带隐式运行时依赖参数（如 quote .allocator）的函数不可直接按 C ABI 导出
     ^attrs.abi(.c)
     pub fn bad_alloc_export(v: Vec<u8, .A = FixedBufferAllocator, .allocator = alloc>) -> c_int
     where
         quote alloc: lift *FixedBufferAllocator,
     {
         0
     }
     ```
3. **缺口清单**：
   - C `struct`/`union` 聚合类型跨 `^attrs.abi(...)` 参数与返回值传递（按目标 SysV AMD64 two-eightbyte / Win64 / AAPCS64 HFA 规则分类降码；当前 `control.rs:200` 仅支持标量、指针、`void` 与 `Never`）。
   - C ABI 函数指针类型支持（`^attrs.abi(.c) fn(c_int) -> c_int` 或属性化 `fn_type`）与 C 可变参外部调用（如 `printf`/`open`/`ioctl`）。
   - `CStr` 编译期字面量后缀 `LiteralInto<.cstr, CStr> for str`。
   - 效应恢复与 C 帧屏障运行时检查（在跨越 C 帧捕获的 continuation 上执行丢弃/中止时触发 trap，见 `effect-abi.md`）。

#### 3.6.2 `std.os` / `std.platform`：多平台结构化抽象

当前 [platform/linux.fl](../../library/std/platform/linux.fl) 仅通过 11 个硬编码 LLVM `platform.ll` intrinsic 支持 `x86_64` Linux。Wishlist 规定演进为清晰的**平台分层树**：
- 使用编译期 `inline if build.target.os == .linux` 等分支按目标平台组织公开导出与实现（无 C 预处理器 `#ifdef`）：
  - **`std.os.unix`**：全部类 Unix 共用抽象：`RawFd = i32`、`OwnedFd` / `BorrowedFd`（`Once` / `Drop` 关闭）、`Errno`、`io`（`read`/`write`/`pread`/`pwrite`/`readv`/`writev`/`pipe`/`pipe2`/`dup`/`fcntl`/`ioctl`）、`fs`（`open`/`openat`/`close`/`fstat`/`stat`/`lstat`/`mkdir`/`rmdir`/`unlink`/`rename`/`symlink`/`readlink`/`realpath`/`opendir`/`readdir`/`closedir`/`fsync`/`ftruncate`）、`process`（`fork`/`execve`/`waitpid`/`kill`/`getpid`/`_exit`）、`time`（`clock_gettime`/`nanosleep`）、`mmap`（`mmap`/`munmap`/`mprotect`）、`env`（`getcwd`/`chdir`/`getenv`/`setenv`/`unsetenv`）。
  - **`std.os.linux`**：Linux 专有系统调用与特性（`syscall`、`getdents64`、`statx`、`epoll_create1`/`epoll_ctl`/`epoll_wait`、`eventfd`/`timerfd`/`signalfd`、`futex`、`clone3`、`memfd_create`、`getrandom`）。
  - **`std.os.macos`**：Darwin/macOS/iOS 抽象（`kqueue`/`kevent`、`mach_absolute_time`、`os_unfair_lock`/`ulock`、`_NSGetExecutablePath`/`_NSGetArgv`/`_NSGetEnviron`、Darwin 64-bit `stat` 结构布局、`posix_spawn`、`clonefile`）。
  - **`std.os.windows`**：Win32 / NT 抽象（`RawHandle = *c_void`、`RawSocket = usize`、`OwnedHandle`、`BorrowedHandle`、`GetLastError`、`CreateFileW`/`ReadFile`/`WriteFile`/`CloseHandle`/`SetFilePointerEx`/`GetFileInformationByHandleEx`/`FindFirstFileW`/`FindNextFileW`/`FindClose`/`CreateDirectoryW`/`RemoveDirectoryW`/`DeleteFileW`/`MoveFileExW`/`CreateProcessW`/`WaitForSingleObject`/`GetExitCodeProcess`/`TerminateProcess`/`CreatePipe`/`QueryPerformanceCounter`/`GetSystemTimePreciseAsFileTime`/`Sleep`/`VirtualAlloc`/`VirtualFree`/`HeapAlloc`/`HeapFree`/`GetEnvironmentVariableW`/`SetEnvironmentVariableW`/`GetCommandLineW`/`GetModuleFileNameW`）。
  - **`std.os.wasi`**：WASI WebAssembly 系统接口（`fd_read`/`fd_write`/`fd_seek`/`fd_close`/`path_open`/`fd_readdir`/`clock_time_get`/`proc_exit`/`args_get`/`environ_get`/`random_get`）。
- **底层接入架构演进**：现阶段通过扩展 `platform.ll` 可快速补齐 `clock_gettime`/`stat`/`readdir`/`pipe`/`waitpid`；中长期当 `std.ffi` 的无主体 `^attrs.abi(.c)` / `^attrs.abi(.system)` 外部声明落地后，应把 libc/Win32 绑定迁回纯 `std.os.*` 源码声明，淘汰维护成本高昂的编译器内置 `Operation.Linux*` 枚举扩张（见 §5 Q8）。

#### 3.6.3 `std.simd`：可移植向量与掩码（Silver Layer 4）

依据 `tmp/silver-design/04-compiler-wishlist.md`（P1-5）与 `docs_chat_history/071:39`、`076:42`，`std.simd` 作为标准库普通泛型模块实现（不引入特殊语言关键字），依赖编译器补全 `<N x T>` LLVM 向量原语与 `^attrs.layout.align(align_bytes)` 属性：

```flurry
pub pure comptime fn is_supported_lane_count(n: usize) -> bool {
    n == 1 or n == 2 or n == 4 or n == 8 or n == 16 or n == 32 or n == 64
}

pub trait SimdElement {
    assoc MaskElem: Type; -- 例如 f32/i32/u32 -> i32, f64/i64/u64 -> i64, u8/i8 -> i8, u16/i16 -> i16
}

pub trait MaskElement {}

-- 内部通过编译器目标向量原语绑定 <N x T> 物理表示，显式对齐以消除非对齐拆分与伪共享
pub struct Simd
where
    T,
    requires T :- SimdElement,
    N: usize,
    requires is_supported_lane_count(N),
{
    lanes: Array<T, N>, -- 经由目标向量表示映射为 LLVM <N x T>
}

pub struct Mask
where
    T,
    requires T :- MaskElement,
    N: usize,
    requires is_supported_lane_count(N),
{
    lanes: Array<T, N>, -- 全 1（-1）表示 true，全 0 表示 false；支持向目标 predicate/bitmask 降码
}

impl Simd<T, N>
where
    T,
    requires T :- SimdElement,
    N: usize,
    requires is_supported_lane_count(N),
{
    pub pure fn splat(value: T) -> Self;
    pub pure fn from_array(arr: Array<T, N>) -> Self;
    pub pure fn to_array(self) -> Array<T, N>;
    pub fn from_slice(slice: Slice<T>) -> ?Self;
    pub unsafe fn load_aligned(ptr: *T) -> Self;
    pub unsafe fn load_unaligned(ptr: *T) -> Self;
    pub unsafe fn store_aligned(self, ptr: *T);
    pub unsafe fn store_unaligned(self, ptr: *T);
    pub fn select(mask: Mask<T.as(SimdElement).MaskElem, N>, true_vals: Self, false_vals: Self) -> Self;
    pub pure fn cast(self) -> Simd<U, N>
    where
        U,
        requires U :- SimdElement;
    pub pure fn swizzle(self, comptime idxs: meta.List<usize>) -> Simd<T, M>
    where
        M: usize,
        requires is_supported_lane_count(M);
    pub fn reduce_sum(self) -> T;
    pub fn reduce_min(self) -> T;
    pub fn reduce_max(self) -> T;
}

impl Mask<T, N>
where
    T,
    requires T :- MaskElement,
    N: usize,
    requires is_supported_lane_count(N),
{
    pub pure fn splat(value: bool) -> Self;
    pub pure fn all(self) -> bool;
    pub pure fn any(self) -> bool;
    pub pure fn test(self, lane: usize) -> bool;
    pub fn set(*self, lane: usize, value: bool);
    pub pure fn to_bitmask(self) -> u64;
    pub pure fn from_bitmask(bitmask: u64) -> Self;
    pub pure fn first_set(self) -> ?usize;
}
```

完整公开方法族：
- **构造与读写**：`splat`、`from_array`、`to_array`、`from_slice`、`load_or` / `load_or_default`、`unsafe load_aligned`、`unsafe load_unaligned`、`unsafe store_aligned`、`unsafe store_unaligned`、`gather_or(slice: Slice<T>, idxs: Simd<usize, N>, or: Self) -> Self`、`gather_select`、`scatter(slice: MutSlice<T>, idxs: Simd<usize, N>)`、`scatter_select`。
- **算术、位运算与转换**：实现 `Add`/`Sub`/`Mul`/`Div`/`Rem`/`Neg`/`Copy`/`Clone`/`Eq`/`Debug`（逐通道比较由 `simd_eq`/`simd_ne`/`simd_lt`/`simd_le`/`simd_gt`/`simd_ge` 返回 `Mask`）；补充 `saturating_add`/`saturating_sub`、`bit_and`/`bit_or`/`bit_xor`/`shl`/`shr`、`abs`、`min`/`max`/`clamp`、`cast`（目标类型 `U :- SimdElement` 在 `where` 子句中声明）；浮点向量专属 `mul_add(self, b: Self, c: Self) -> Self`（FMA）、`sqrt`、`floor`、`ceil`、`round`、`trunc`、`is_nan`、`is_finite`、`is_infinite`。
- **通道重排与水平归约**：`select`、`swizzle`（输出通道数 `M: usize` 与索引越界均在 `where` / `comptime` 校验）、`simd_swizzle2`、`rotate_elements_left`、`rotate_elements_right`、`reverse`、`interleave`、`deinterleave`、`reduce_sum`、`reduce_product`、`reduce_min`、`reduce_max`、`reduce_and`、`reduce_or`、`reduce_xor`。
- **缓存行对齐联动**：配合提案属性 `^attrs.layout.align(64)` / `^attrs.layout.align(128)`（`tmp/silver-design/04` P1-5；须先在 `docs/comptime/official-attribute-keys.md` 与 `library/attrs` 登记 `__flurry_align`，见 `05` `WL-05-16`），支持对齐 `SoA` 列缓冲起始地址与无锁共享结构消除伪共享。

---

## 4. 排期优先级与分阶段交付路线

```text
Phase 1（P0 编译器与 builtin 地基解锁）
  ├── builtin/01/02 前置：Array<T, N>、Hasher/BuildHasher、RangeBounds、Deref/DerefMut、ComptimeDebug
  ├── 编译器前置：issues/0001 闭包 env 交付、issues/0003 str provenance、union 免 Drop 验证、std.process.panic 契约裁决
  └── FFI 与 ABI 前置：^attrs.abi(.c/.system/.win64/.sysv64) 无主体外部声明 + 链接属性 + C struct ABI
        │
        ▼
Phase 2（P0 std 核心容器、内存工具与 I/O 完备化）
  ├── std.mem: Unique<T>、MaybeUninit<T>、ManuallyDrop<T>、swap/replace/take/forget/drop/zeroed/align_down、Rc/RcWeak、Cow/ToOwned、Box 补全
  ├── std.collections: MutSlice<T> + Slice 排序/二分/分块/窗口算法、Vec/VecDeque 补全（drain/retain/dedup/split_off）、HashMap(.H + Entry + 去 Clone 查询)/HashSet 升级
  ├── std.string & std.fmt: FromUtf8Err、String.as_str/drain/replace_range、print/eprint 变参、DebugStruct* 与错误 Display
  └── std.io: IoErrKind、BufRead、Seek/SeekFrom、LineWriter、Cursor<T>、copy/Take/Chain/Empty/Sink
        │
        ▼
Phase 3（P1 系统编程与 OS 生态全面打通）
  ├── std.ffi & std.os: 完整 C 类型表 + CStr/CString + OsStr/OsString + unix/linux/macos/windows/wasi 分层实现
  ├── std.path & std.fs: Path/PathBuf/Component/Prefix + 运行时路径 File/OpenOptions/Metadata/ReadDir/DirEntry
  ├── std.time & std.env & std.process: Instant/SystemTime/sleep、args/var/set_var/consts、Command/Child/exit/abort/panic
  ├── std.mem 扩展分配器: BumpAllocator、SlabAllocator、PoolAllocator、ScratchAllocator、TrackingAllocator、FailoverAllocator
  └── std.collections 扩展结构: SmallVec<T, N>、BTreeMap/BTreeSet、BinaryHeap、BitSet、RingBuffer<T, N>
        │
        ▼
Phase 4（P1/P2 Silver Layer 4 运行时依赖容器与便携 SIMD）
  ├── Arena<T> + SlotMap<K, V> + 4B/8B NodeIndex<arena.ref> 句柄（打通 P1-4 隐式环境交付与悬垂失效检查）
  ├── 反射驱动类型化 SoA<T> + case deref 惰性列访问 + ^attrs.layout.align(64/128)
  └── std.simd.Simd<T, N> & Mask<T, N>（打通 P1-5 LLVM 向量与目标指令降码）
```

**阶段验收基线**：
- 每阶段必须在真实 `--sysroot library`（而非孤立 test stub）下验证，覆盖 interpreter 与 LLVM `-O0`/`-O3` 原生执行；
- `Box`/`Rc`/`Vec`/`String`/`HashMap`/`BTreeMap`/`BufReader`/`Command`/`PathBuf`/`Arena` 必须同时覆盖默认 `DefaultAllocator`（验证 `Type.size_of` 无额外 allocator 指针开销）与自定义局部 `FixedBufferAllocator`/`ArenaAllocator`/`TrackingAllocator`（验证零泄漏与错误传播路径提前清理）。

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### Q1. `std.process.panic` 的终止模式、清理义务与可捕获边界（Hard Blocker）
- **来源依据**：[library/README.md:393-395](../../library/README.md)、[development/library-prototypes/README.md:10](../../development/library-prototypes/README.md)、[docs/algebraic-effect-and-delimited-continuation/effect-abi.md:195](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)、[compiler/luna/src/host/llvm/attributes.rs:53-56](../../compiler/luna/src/host/llvm/attributes.rs)。
- **受影响行为**：`OptionalUnwrap.unwrap`、`ErrorResultUnwrap.unwrap`、`Slice`/`Vec` 越界索引、`SoA` 调试检查、`Rc`/`Arc` 计数溢出及 `Mutex` 中毒契约。
- **最小决策选项**：
  - *选项 A（推荐，分阶段兼容）*：定义 `panic(msg: str) -> NoReturn` 为**非正常进程/任务终止**，默认采用可控展开（沿 `effect-abi.md` 隐藏中止通道逆序执行栈上 `Drop`，在 `^attrs.abi(.c)` C 帧屏障处 trap，并允许构建配置选择 `panic = .abort` 省去展开元数据）；保持空效应行，禁止未标 `unsafe` 的用户代码捕获，仅允许线程池/异步任务监督边界观测任务异常终止。
  - *选项 B*：规定 `panic` 永久等价于 immediate abort（打印诊断后直接 `abort()`，不运行任何 `Drop`）。

### Q2. `DefaultAllocator` OOM 策略与 `Clone` vs `try_clone` 的分裂
- **来源依据**：[library/std/mem_impl/allocator.fl](../../library/std/mem_impl/allocator.fl)、[library/std/collections_impl/vec.fl:640](../../library/std/collections_impl/vec.fl)、[library/builtin/protocols/resources.fl:11-13](../../library/builtin/protocols/resources.fl)。
- **冲突现状**：`Allocator.allocate` 返回 `!AllocErr RawBlock`，而 `builtin.Clone.clone(*self) -> Self` 不能返回 `!AllocErr`，迫使 `Box`/`Vec`/`String`/`HashMap`/`HashSet` 无法实现标准 `Clone`（及 `Copy` 约束之外的泛型 `Clone`），只能各自提供固有 `try_clone() -> !AllocErr Self`；同时所有简单操作（`Vec.push`、`String.push_char`、`HashMap.insert`）均传染 `!AllocErr`。
- **最小决策选项**：
  - *选项 A（推荐，显式安全优先）*：保留 `Allocator.allocate -> !AllocErr RawBlock` 与容器方法显式 `!AllocErr` 作为系统级默认契约；同时在 `std.mem` 新增标准 `trait TryClone { fn try_clone(*self) -> !AllocErr Self; }`（及 `Box`/`Vec`/`String` 等全部实现 `TryClone`），仅对纯递增引用计数且绝不失败的 `Rc`/`Arc`/`RcWeak` 实现 `builtin.Clone`。
  - *选项 B*：为 `DefaultAllocator` 提供 infallibly aborting 包装（OOM 直接 `abort`/`panic`），允许基于全局分配器的特化实例额外提供不带 `!AllocErr` 的包装与 `Clone`。

### Q3. 泛型与高阶方法内闭包捕获外部 `RuntimeEnvParameter`（`quote .allocator` / `quote arena`）的 ABI 交付
- **来源依据**：[issues/0001:318-366](../../issues/0001-runtime-dependent-origin-and-default-param.md)、[compiler/materialization/src/lib.rs:2803-2814](../../compiler/materialization/src/lib.rs)、[library/std/mem_impl/boxed.fl](../../library/std/mem_impl/boxed.fl)。
- **受影响行为**：`Box.with_ref`、`Vec.try_for_each_ref`、`Vec.retain`、`SliceMut.sort_by`、`HashMap.entry.or_insert_with` 等在泛型体内使用闭包的场景（Stage 2A 已撞此边界并记录 `Limitations:`）。
- **最小决策**：需裁定当泛型闭包特化后才显现需要调用者持有的 `RuntimeEnvParameter`（如闭包内调用自定义分配器 `String` 的方法或分配）时，materialization 如何向闭包调用隐式交付这些 `env_parameters` 而不污染用户显式声明的 `%{ captured_fields }` 捕获字段集合。

### Q4. 运行时 `str` / `Slice<u8>` 视图的区域证明与 `String.as_str` / `OsStr.to_str` 放行边界
- **来源依据**：[library/STRUCTURE.md:23-24](../../library/STRUCTURE.md)、`development/repros/native-text-method-provenance.fl`、[docs/resource-management/memory-views.md](../../docs/resource-management/memory-views.md)。
- **受影响行为**：`String.as_str(*self) -> str`、`CStr.to_str(*self) -> !Utf8Err str`、`OsStr.to_str(*self) -> ?str` 及 `String.lines`/`split`/`trim` 返回 `str` 子视图（若返回 `str` 需编译器 `text_provenance` 证明该 `str` 不活过原拥有者 `String`/`CString` 扩容或释放）。
- **最小决策**：明确 `pointer_input_layout` 与跨方法借用生命周期证明如何将 `String` / `Vec<u8>` 底层堆缓冲区关联到返回的 `str` / `Slice<T>` 视图，从而解锁安全模式下的零拷贝字符串切片 API。

### Q5. `std.ffi` 外部符号绑定机制与 `docs/others/ffi.md` 冻结内容
- **来源依据**：[docs/others/ffi.md](../../docs/others/ffi.md)（目前为空）、[docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md)、[compiler/elaboration/src/program/attributes.rs:108-124](../../compiler/elaboration/src/program/attributes.rs)、[compiler/luna/src/host/llvm/control.rs:166-250](../../compiler/luna/src/host/llvm/control.rs)。
- **受影响行为**：如何声明从 libc / 动态库 / C 静态库导入的外部 C 函数（特别是 C 可变参数 `...`、`link_name` 重命名、链接库列表 `link`）、支持的全部 `^attrs.abi(...)` 约定列表（`.system`/`.win64`/`.sysv64`/`.aapcs`），以及 C struct/union 的目标 ABI 聚合分类规则。
- **最小决策**：建议在 `library/attrs` 登记 `__flurry_ffi_link` / `__flurry_ffi_link_name`（通过 `^attrs.ffi.link(...)` / `^attrs.ffi.link_name(...)` 暴露，绝不引入 `extern` 关键字），并允许无主体的 `^attrs.abi(...) unsafe fn ...;` 声明在 LLVM 后端降码为 `declare`；同步补全 [docs/others/ffi.md](../../docs/others/ffi.md)。

### Q6. `MutSlice<T>`、`Deref`/`DerefMut` 与 `Index`/`IndexMut` 的借用/别名所有权规则
- **来源依据**：[docs/resource-management/pointers.md](../../docs/resource-management/pointers.md)、[docs/traits/protocols.md](../../docs/traits/protocols.md)、[library/builtin/protocols/ops.fl:82-90](../../library/builtin/protocols/ops.fl)、[library/std/collections_impl/slice.fl](../../library/std/collections_impl/slice.fl)。
- **受影响行为**：`Vec[i]`、`HashMap[k]`、`Box.*`、`Rc.*`、以及 `Vec.as_mut_slice(*self) -> MutSlice<T>` 后调用原地排序与 `split_at_mut`。
- **最小决策**：Flurry 原生 `*T` 允许顺序别名且可 `Copy`，而 `MutSlice<T>` 的原地排序、切分与扩容安全性要求防止 `Vec` 在 `MutSlice<T>` 存活期间重新分配缓冲。需明确 `MutSlice<T>`（以及 `RefMut` / `OccupiedEntry`）是否直接借由 `&reachability_set` 可达性类型限制宿主容器的并发写操作，或统一以 `Once` 借用契约 + `program_verification` 独占约束检查。

### Q7. `Cow` 与 `ToOwned` / `Borrow` 的值描述符适配（`str`/`Slice<T>` vs `*T`）
- **来源依据**：[library/builtin/primitives.fl](../../library/builtin/primitives.fl)、[library/std/collections_impl/slice.fl](../../library/std/collections_impl/slice.fl)。
- **冲突点**：Rust 中 `str` 和 `[T]` 是 DST，故 `Borrow<B>` 统一返回 `&B` 指针；但在 Flurry 中，`str`、`Slice<T>`、`CStr`、`OsStr`、`Path` 本身就是值类型视图描述符（`{ addr, len }`），而普通结构体 `T` 借用是 `*T`（指针）。若强行要求 `String` 实现 `Borrow<str>` 返回 `*str` 会多出一层无意义的临时指针间接层。
- **最小决策**：建议区分 `BorrowView<V>`（按值返回可 `Copy` 视图描述符，如 `String.as_str() -> str`）与 `Borrow<T>`（返回 `*T`）；`Cow<B, .A, .allocator>` 参数 `B` 直接接收视图类型本身（如 `Cow<str>`、`Cow<Slice<u8>>`、`Cow<*MyStruct>`），避免双重间接。

### Q8. 平台系统原语（`std.os`）长期路线：LLVM `platform.ll` Intrinsic vs `^attrs.abi` 外部绑定
- **来源依据**：[compiler/luna/src/host/llvm/platform.ll](../../compiler/luna/src/host/llvm/platform.ll)、[compiler/nir/src/intrinsics.rs:101-111](../../compiler/nir/src/intrinsics.rs)、[library/std/platform/linux.fl](../../library/std/platform/linux.fl)。
- **受影响行为**：`std.fs` 目录遍历/元数据、`std.time.Instant`/`SystemTime`、`std.env`、`std.process.Command` 与非 Linux（macOS/Windows/WASI）平台移植。
- **最小决策**：当前每增加一个 OS 调用都要修改编译器 [compiler/nir/src/intrinsics.rs](../../compiler/nir/src/intrinsics.rs) 中的 `Operation`、`language-items.conf` 与 `platform.ll`。一旦 Q5（`^attrs.abi` 无主体外部 C 声明）落地，建议作者确认是否将 `std.os.*` 全部迁至纯粹 `std` 源码的 C ABI / 平台系统库声明，使编译器摆脱对具体 OS syscall 清单的硬编码耦合。

### Q9. `std.simd` 越界/非原生向量宽度的可移植语义与目标特性分派
- **来源依据**：[docs_chat_history/071-pipeline-projection-01.md:39](../../docs_chat_history/071-pipeline-projection-01.md)、[docs_chat_history/076-pipeline-projection-04.md:42](../../docs_chat_history/076-pipeline-projection-04.md)、[tmp/silver-design/04-compiler-wishlist.md](../../tmp/silver-design/04-compiler-wishlist.md) P1-5。
- **受影响行为**：如在不支持 AVX-512 的宿主上实例化 `Simd<f32, 16>`，或 `build.target.features` 为基准 SSE2/NEON 时如何使用高吞吐向量宽度。
- **最小决策**：明确 `Simd<T, N>` 是否由 LLVM 后端自动合法化拆分（例如 `Simd<f32, 16>` 拆为两条 AVX2 或四条 SSE2 寄存器操作，保证全平台可编译），以及是否需要运行时 CPUID 分派辅助原语（或仅保留编译期 `build.target.features` 静态分派）。

### Q10. `SoA<T>` 编译期生成带 `quote` 运行时依赖参数的 Normal-Form 类型构造器
- **来源依据**：`proposal_trace/2026-8-5-typed-Soa.fl`（仅作最近 6 个月意图参考）、[library/meta/definitions/structs.fl](../../library/meta/definitions/structs.fl)、[docs/type/reflection-and-layout.md](../../docs/type/reflection-and-layout.md)。
- **受影响行为**：`SoA<T, .A, .allocator>` 与 `NodeIndex<.arena = soa.ref>` 的实现路径。
- **最小决策**：当前 `meta.definitions.StructBuilder` 仅支持生成普通结构体字段，尚不支持向生成结构体附加 `where .A: Type, quote .allocator: lift *A` 子句、`^attrs.layout.align(...)` 属性与配套 `impl Drop`（跨阶段 `FnBuilder`/`ImplBuilder` 缺失，见 `builtin_packages_gap_report` §2.2）。需确定 `SoA<T>` 是等待 `StructBuilder` + `ImplBuilder` 完备后在纯库层构建，还是配合通用编译期字段列表组合构造（如 `SoAColumns<meta.struct_field_types(T)>`）。

### Q11. `std.collections` 边界容器（`SoA`、`SlotMap`、`SmallVec`、`RingBuffer`）与 Tier 2 `collections_ext` 的归属划界
- **来源依据**：`AGENTS.md` 六内置包边界原则、[docs/code-organization/builtin-packages.md](../../docs/code-organization/builtin-packages.md)、[10-official-protocol-packages-and-boundary.md](./10-official-protocol-packages-and-boundary.md)。
- **冲突与权衡**：为防止 Tier 1 六大内置包（尤其是 `std`）持续膨胀，`SoA`、`SlotMap`、`SmallVec`、`RingBuffer` 这类针对特定数据导向布局（DOD）、游戏/图形引擎场景或定长栈内环缓冲的结构体，究竟留在 `std.collections` 中还是从 `std` 拆出至 Tier 2 官方扩展包 `collections_ext`？其中 `SoA<T>`、`Arena<T>` + `NodeIndex` 与 Silver 渲染/UI 节点图强耦合（`tmp/silver-design/04` P1-4），且依赖标准库内的 `Allocator` 协议与 `meta.reflection.reflect_struct`（`05` `WL-05-12`）/ `struct_field_names`。
- **最小决策选项**：
  - *选项 A（推荐，核心分层）*：保留通用基础容器（`Vec`/`VecDeque`/`HashMap`/`HashSet`/`BTreeMap`/`BTreeSet`/`BinaryHeap`/`BitSet`/`Slice`/`MutSlice`）与最小 `Arena` + `NodeIndex` 在 Tier 1 `std.collections`；将专门化的 `SoA<T>`、`SlotMap`、`SecondaryMap`、`SmallVec`、`RingBuffer`（连同不可变/持久数据结构与图容器）划入 Tier 2 官方包 `collections_ext`（依赖 `std.mem.Allocator` 与 `builtin.Array`）。
  - *选项 B*：鉴于 `SmallVec`/`RingBuffer` 在无堆系统编程及 `std` 自身实现中高频复用、而 `Arena`/`SlotMap`/`SoA` 是 Flurry 运行时依赖类型的标杆容器，将其保留在 `std.collections` 中，仅将持久化不可变容器与通用图结构划入 `collections_ext`。
