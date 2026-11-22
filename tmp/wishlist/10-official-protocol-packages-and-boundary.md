# WL-10: Tier 2 官方薄协议包生态架构与六内置包边界规范

> **文档角色**：本文档定义 Flurry 生态系统的**三层包边界规范（Tier 1 六内置包 / Tier 2 官方薄协议包 / Tier 3 社区实现与派生器包）**、针对 Wishlist `01`–`09` 的**防膨胀越界审计表**，以及全部 **17 组 Tier 2 官方薄协议包**的完整 Flurry 接口规格、派生器接入契约、一致性（coherence）/ 孤儿规则适配、`allocator` 透传规范与阶段演进路线（owner: `@eco-author`）。
> **核心原则**：严格保持 `builtin`、`meta`、`build`、`attrs`、`std`、`verification` 六个内置包的精简正交性；所有领域协议（序列化、遥测、密码学、随机数、异步执行器调度协议、HTTP/Wire、SQL、压缩/转码、校验/Schema、CLI/配置、日历/时区、音视频/色彩、张量后端/代数、属性测试、FFI 编组、正则/解析器协议、扩展集合）全部置于六内置包之外的 Tier 2 官方薄协议包中，绝不污染 Tier 1 语言与系统基线。

---

## 1. 范围、权威来源与归属包

### 1.1 三层包生态架构总纲

Flurry 将语言内置基础设施、官方领域协议契约与具体工程实现严格划分为三层，形成单向有向无环依赖（Tier 3 -> Tier 2 -> Tier 1）：

| 层级 | 包集合 | 核心职责 | 严禁包含的内容（反膨胀红线） |
|---|---|---|---|
| **Tier 1：六个内置包**（编译器耦合与系统基线） | 严格且仅限 6 个包：`builtin`、`meta`、`build`、`attrs`、`std`、`verification` | 编译器内建原语与集中绑定（`primitives`/`Tuple`/`Array`/`BitVec`/`?T`/`!E T`）、`Once`/`Clone`/`Copy`/`Drop`、运算符与模式视图协议、`builtin.math`/`meta.math`（LLVM intrinsic 泛型包装）、CMTT 与 `Type.*`/PTC/宏反射、官方 `__flurry_*` 属性包装、`build` 构建上下文、跨平台 OS/IO/内存分配器/核心容器/`std.ffi`/`std.simd`/最小 `Continuation`+`StackPool` ABI、`verification` 验证契约。 | **绝不包含**：`regex`（禁止进入 `std` 或 `builtin`）、JSON/TOML/YAML/CBOR/Protobuf 序列化、HTTP/TLS/gRPC/WebSocket、SQL/ORM、密码学原语/摘要、PRNG/CSPRNG 引擎、历法/时区/`tzdata`、压缩算法、UTF-8 之外文本编码/Base64、图像/音频/色彩编码、ML/张量运行时后端、具体日志导出器、具体异步执行器调度器、CLI/配置文件解析器、校验/Schema 引擎、Fuzz/属性测试框架。 |
| **Tier 2：官方薄协议包**（Thin Protocol Packages，**位于六内置包之外**） | 由官方组织维护的标准独立包：`serialization`、`telemetry`、`crypto`、`digest`、`random`、`async_runtime`、`http`、`wire`、`database`、`compression`、`encoding`、`validation`、`schema`、`cli`、`config`、`datetime`、`media`、`color`、`numerics`、`tensor_protocol`、`testing`、`ffi_bridge`、`plugin_abi`、`regex_protocol`、`parser_protocol`、`collections_ext` | 仅定义参数化 `trait`（如 `trait Serialize where format: Symbol`）、`pure comptime trait` 编译时对偶、薄 `effect` / `effect enum` 接口（如 `Log`、`system_clock`、`random_bytes`）、统一领域错误枚举（`*Err`）、纯值描述符与领域元数据属性构造器（**严禁使用 `__flurry_*` 前缀**），使任意 Tier 3 生态库可通过具名派生策略（如 `derive Serialize<.json>.use(json_lib.without_allocation) for Data;`）与 `.use(handler)` 无缝互操作。 | **绝不包含**：格式解析器/编码器状态机（如 JSON lexer、CBOR decoder）、具体加密算法（SHA-256/AES/ChaCha20）、OS 线程/事件循环实现（`io_uring`/`epoll`）、数据库 wire 协议（Postgres/MySQL）、`tzdata` 数据库、PNG/Opus 编解码器、CUDA/cuBLAS/Vulkan 内核调度引擎、DFA/NFA 引擎或隐式全局分配。 |
| **Tier 3：生态实现与派生器包**（Community & Vendor Implementations） | `json_lib`、`cbor_lib`、`toml_lib`、`regex_dfa`、`postgres`、`sqlite`、`blake3`、`chacha_poly`、`io_uring_rt`、`epoll_rt`、`silver`、`cuda`、`zstd_flurry`、`otel_exporter` 等 | 实现 Tier 2 协议接口与导出具名派生策略（`pub derive fn without_allocation for Serialize<.json>`、`pub derive fn by_column_name for FromRow<.postgres>`），提供外源类型 `.as(View)` `newtype` 适配器、具体效应处理器（`.use(io_uring_rt)`、`.use(otel_handler)`）及跨管线后端（`cuda_compile`）。 | 遵守普通包归属与一致性规则（无六内置包孤儿豁免）；严禁直接读写 `.symbol'builtin` 或构造 `__flurry_*` 官方编译器内部键。 |

### 1.2 权威语义来源与最新性边界

严格遵守 [AGENTS.md](../../AGENTS.md) 确立的语义权威顺位与最新性截止线：

1. **第一顺位（最高权威）**：作者确认规则及 `AGENTS.md` 第 5 节不可破坏的语义边界（无 `Option`/`Result` 枚举、严格点分路径 `a.b.c`、运行时依赖类型 `allocator` 范式、`PascalCaseErr`/`PascalCase` 错误变体/`snake_case` 普通变体、仅六个内置包、集中 `.symbol'builtin` 绑定、官方 `^attrs.*` 封装 `__flurry_*` 且 Tier 2 禁用 `__flurry_*`、`pure comptime trait` 与 runtime `trait` 同名无前缀且 comptime 使用 `self` 值引用）。
2. **第二顺位（现行正式文档 `docs/`）**：
   - 包与 Prelude 边界：[docs/code-organization/builtin-packages.md](../../docs/code-organization/builtin-packages.md)
   - Trait 声明、派生、一致性与协议：[docs/traits/declarations.md](../../docs/traits/declarations.md)、[derivation.md](../../docs/traits/derivation.md)、[coherence.md](../../docs/traits/coherence.md)、[protocols.md](../../docs/traits/protocols.md)、[implementation-resolution.md](../../docs/traits/implementation-resolution.md)、[dynamic-interfaces.md](../../docs/traits/dynamic-interfaces.md)
   - 属性、阶段与类型系统：[docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md)、[attribute.md](../../docs/comptime/attribute.md)、[the-comptime-phases-of-traits.md](../../docs/comptime/the-comptime-phases-of-traits.md)、[static-shaped-tensors.md](../../docs/comptime/static-shaped-tensors.md)、[docs/type/receiver-adaptation.md](../../docs/type/receiver-adaptation.md)、[type_id.md](../../docs/type/type_id.md)、[type-identity.md](../../docs/type/type-identity.md)、[runtime-dependent-type.md](../../docs/type/runtime-dependent-type.md)、[newtype-and-typealias.md](../../docs/type/user-defined-types/newtype-and-typealias.md)
   - 错误与代数效应：[docs/algebraic-effect-and-delimited-continuation/error-handling.md](../../docs/algebraic-effect-and-delimited-continuation/error-handling.md)、[algebraic-effect.md](../../docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md)、[async-algebraic-effect.md](../../docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md)、[effect-abi.md](../../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)、[handling-capability.md](../../docs/algebraic-effect-and-delimited-continuation/handling-capability.md)、[direct-passing.md](../../docs/algebraic-effect-and-delimited-continuation/direct-passing.md)
   - 管线投影、语法与资源：[docs/pipeline-projection/declarations-and-interfaces.md](../../docs/pipeline-projection/declarations-and-interfaces.md)、[backends-and-artifacts.md](../../docs/pipeline-projection/backends-and-artifacts.md)、[docs/grammar/index.md](../../docs/grammar/index.md)、[docs/grammar/syntax](../../docs/grammar/syntax)、[docs/resource-management/once-clone-copy.md](../../docs/resource-management/once-clone-copy.md)
3. **近期 `proposal_trace/` 参考（严格截止 `2026-5` 至 `2026-10`，严禁引用任何 `2024-*` / `2025-*` / `2026-1..4-*` 草案）**：
   - [2026-8-13-derivation.fl](../../proposal_trace/2026-8-13-derivation.fl)（`Serialize` 参数化派生策略与 `.as(MySlice)` 适配意图，已在本文中按现行 `docs/` 现代化——如用 `String` + `ByteWriter` 零分配模式、合规 `and`/`not` 逻辑与正式包归属规则替代草稿写法）
   - [2026-9-18-handling-capability-by-trait-system.fl](../../proposal_trace/2026-9-18-handling-capability-by-trait-system.fl)（`HandleError`/`HandleEffect`、错误命名与 `where` 大括号对齐约定）
   - [2026-9-19-regex.fl](../../proposal_trace/2026-9-19-regex.fl)（`RegexMatcher` 作为 CMTT + `TuplePatternView` 消费者，明确移出 `std`/`builtin` 放入 `regex_protocol` + Tier 3）
   - [2026-8-26-pipeline-projection.fl](../../proposal_trace/2026-8-26-pipeline-projection.fl)（`^cuda.global`、`meta.compilation.IIR.Func` 与 `meta.runtime.invoke`）
   - [2026-9-7-tensor-algebra.fl](../../proposal_trace/2026-9-7-tensor-algebra.fl)（`Tensor` 运行时依赖分配器与 `SpaceApply`/`VariadicCollector` 设计意图，明确将具体张量/后端归入 Tier 2/3）
4. **现状盘点参考（非语义权威）**：[library/STRUCTURE.md](../../library/STRUCTURE.md)、[library/std/mem_impl/boxed.fl](../../library/std/mem_impl/boxed.fl)、[tmp/silver-design/04-compiler-wishlist.md](../silver-design/04-compiler-wishlist.md) 及已落盘 Wishlist 文件（`02`、`04`、`05`、`07`）。

### 1.3 严格六内置包排除黑名单（6-Builtin-Package Exclusion Blacklist）

凡列入下表的领域能力**一律禁止**进入 `builtin`、`meta`、`build`、`attrs`、`std`、`verification` 六个内置包；任何将其放入 Tier 1 的 PR、Wishlist 条目或库源码均判定为越界膨胀：

| 排除类别 | 严禁放入 Tier 1（六内置包）的具体内容 | Tier 1 中仅允许保留的最小正交基线 | 正确的 Tier 2 / Tier 3 归属 |
|---|---|---|---|
| **正则表达式（`regex`）** | `RegexMatcher`、NFA/DFA 引擎、PCRE/Unicode 正则编译与匹配器（**`regex` 严禁进入 `std` 或 `builtin`！**） | `builtin` 仅保留 `TuplePatternView`/`ConcatPatternView`；`meta` 仅保留通用 CMTT `< Ψ \|- T >` 与 `code_match`/`code_sequence` | Tier 2：`regex_protocol`；Tier 3：`regex_dfa`、`pike_nfa` |
| **序列化与数据格式** | `Serialize`/`Deserialize`、JSON/TOML/YAML/XML/CBOR/Protobuf/MessagePack 读写器与派生器 | `builtin.fmt` 仅保留 `Display`/`Debug`/`ByteWriter`；`std.fmt` 仅保留编译期格式字符串检查 | Tier 2：`serialization`；Tier 3：`json_lib`、`cbor_lib`、`toml_lib` |
| **网络应用层与安全传输** | HTTP/1.1/2/3、gRPC、WebSocket、TLS/QUIC、URL/URI 完整解析与中间件 | `std.io` 同步字节流；`08` 中 `std.net` 仅保留最小 OS socket 抽象（`TcpStream`/`TcpListener`/`UdpSocket`） | Tier 2：`http`、`wire`；Tier 3：`h2_flurry`、`rustls_bridge` |
| **数据库与持久化查询** | SQL 方言（Postgres/MySQL/SQLite）、连接池、ORM、Migrations、`FromRow`/`ToSql` | 无（仅使用 `std.io` / `std.ffi`） | Tier 2：`database`；Tier 3：`postgres`、`sqlite3` |
| **密码学与哈希摘要** | SHA-2/3、BLAKE3、HMAC、AES-GCM、ChaCha20-Poly1305、Ed25519、HKDF、恒定时间/清零协议 | `builtin.Hash`/`Hasher`/`BuildHasher` 仅服务 `HashMap`/`HashSet` 非密码学散列 | Tier 2：`digest`、`crypto`；Tier 3：`blake3`、`sha2`、`chacha_poly` |
| **随机数引擎（RNG）** | PCG、Xoshiro、ChaCha20-DRBG、概率分布采样、`RngCore`/`random_bytes` 效应 | `std.os.*` 仅提供原始 OS 熵系统调用封装（如 `getrandom`），不封装 RNG 引擎或分布 | Tier 2：`random`；Tier 3：`pcg_rng`、`chacha_rng` |
| **历法、时区与格式** | 公历/ISO-8601/农历转换、IANA `tzdata`、RFC3339 格式化、`system_clock` 效应 | `std.time` 仅保留单调 `Instant`、`Duration`、原始时间戳 `SystemTime` 与 `sleep` | Tier 2：`datetime`；Tier 3：`tzdata_iana`、`chrono_flurry` |
| **数据压缩与非 UTF-8 编码** | gzip/zlib/zstd/lz4/brotli；UTF-16/Shift-JIS/GB18030/Latin-1、Base64/Hex/Percent | `builtin.text`（`str`）与 `std.string`（`String`）仅处理 UTF-8（RFC 3629） | Tier 2：`compression`、`encoding`；Tier 3：`zstd_lib`、`base64_lib` |
| **多媒体与色彩空间** | PNG/JPEG/WebP/QOI、WAV/Opus/FLAC、sRGB/OKLCH/Display-P3/ACEScg 转换矩阵 | `std.simd` 仅提供通用可移植向量原语，不内置像素/色彩/音频类型 | Tier 2：`media`、`color`；Tier 3：`png_flurry`、`color_math` |
| **ML 与张量运行时后端** | 动态 `Tensor` 运行时库、自动微分磁带、cuBLAS/CUDA/Metal/Vulkan-Compute 内核与算子库 | `09` 仅定义 `^attrs.compilation.pipeline`、`meta.compilation.IIR`、`meta.runtime.invoke` 与静态形状编译时阶段语义 | Tier 2：`numerics`、`tensor_protocol`；Tier 3：`cuda`、`cpu_blas`、`vulkan_tensor` |
| **具体遥测与日志导出器** | `Log`/`Trace`/`Metric` 效应、OTLP/Prometheus/JSON 结构化日志导出与采样器 | 仅 `std.fmt` 终端 `print`/`println`/`eprintln` | Tier 2：`telemetry`；Tier 3：`otel_exporter`、`fmt_logger` |
| **具体异步执行器调度器** | `io_uring` 事件循环、`epoll`/`kqueue` reactor、多线程 work-stealing 线程池、嵌入式 executor | `builtin.Continuation`、`std.runtime.StackPool`/`BoundedStackPool` 与语言级 `async effect` 挂起/取消 ABI | Tier 2：`async_runtime`；Tier 3：`io_uring_rt`、`epoll_rt`、`silver` |
| **CLI 与配置文件加载** | POSIX/GNU flags 解析、子命令路由、`.env`/INI/TOML/YAML 配置加载与 `config_get` 效应 | `std.env.args` / `std.env.var` 仅提供原始 OS 字符串向量读取 | Tier 2：`cli`、`config`；Tier 3：`clap_flurry`、`dotenv_lib` |
| **数据校验与 Schema 生成** | 规则校验器（`min_len`/`range`）、JSON Schema / OpenAPI / Protobuf 描述生成 | PTC 类型谓词与 `verification` 程序验证契约（与运行时领域输入校验正交） | Tier 2：`validation`、`schema`；Tier 3：`validator_lib`、`openapi_gen` |
| **属性测试与基准测试引擎** | Shrinker/Fuzzer 引擎、统计基准测试计时器、JUnit/TAP 测试报告器 | 核心语法 `test {}` 与 `template test` 钩子 | Tier 2：`testing`；Tier 3：`quickcheck_flurry`、`criterion_flurry` |

### 1.4 Wishlist `01`–`09` 防膨胀越界审计表

下表对全套 Wishlist（`01`–`09`）中的关键/边界项目进行显式三层划分审计；对于已落盘文档（`02`、`04`、`05`、`07`）标明实测位置与纠正动作，对于尚未落盘的规划文档（`01`、`03`、`06`、`08`、`09`）依 Board #2/#10 作用域契约预先锚定防膨胀红线：

| 文档 | 审计状态 | 边界/敏感项目 | 当前定位 / 审计发现 | 裁定归属层 | 纠正与约束动作 |
|---|---|---|---|---|---|
| **`01`（语法协议与操作符）** | 待落盘（按 Board #2 作用域审计） | `Eq`、`Ord`、`Hash`/`Hasher`/`BuildHasher`、`Index`/`SpaceApply`、`Deref`/`Reference`、`Concat`、`LiteralInto`、`Splice` | 全部属于语言语法接线点（`docs/traits/protocols.md`），服务基础集合与语法糖 | **Tier 1（`builtin`）** | 严禁在 `01` 中掺入领域编码运算符（如 JSON 路径、SQL 运算符、regex 字面量后缀）。 |
| **`02`（模式匹配与视图）** | **已落盘审计**（[02-pattern-matching-and-views.md](02-pattern-matching-and-views.md)） | `ListPatternView`、`MapPatternView`、`RangePatternView`、`ConcatPatternView`、`TuplePatternView`、`TypePatternView`、`PatternFunction`、`RegexMatcher` 示例 | 协议本身属于语言级模式视图基础设施；`02` 已将 `WL-02-15`（line 78）与示例 C（line 1299 `-- 模块：user_regex`）明确划归 Tier 2/3 生态包，并声明严禁将 `regex` 放入 `builtin` 或 `std`（`02` 中已无任何 `std.regex` 越界） | **协议 Tier 1（`builtin`/`meta`）；`RegexMatcher` 属于 Tier 2 `regex_protocol` + Tier 3 `regex_dfa`/`user_regex`** | 保持当前清晰分界：Tier 1 仅保留通用 `TuplePatternView`/`ConcatPatternView`；`user_regex`/`regex_dfa` 实现 Tier 2 `regex_protocol` 与 Tier 1 视图协议。 |
| **`03`（派生与标准协议）** | 待落盘（按 Board #10 作用域审计） | `Eq`、`Ord`、`Hash`、`Debug`、`Display`、`Clone`、`Copy`、`Default`、`Splice`、`ToTokens` vs `Serialize`/`Deserialize` | 核心内建派生器属 Tier 1；`Serialize`/`Deserialize` 绝对禁止加入 `builtin`/`std` | **核心派生 Tier 1；`Serialize` 派生展示 Tier 2 `serialization` + Tier 3 `json_lib`** | 在 `03` 中严格只将 `Eq`/`Ord`/`Hash`/`Debug`/`Display`/`Clone`/`Copy`/`Default`/`Splice`/`ToTokens` 归入 Tier 1；`derive Serialize<.json>.use(json_lib.without_allocation) for Data;` 仅作为跨层生态用例交叉引用本文档（`10`）。 |
| **`04`（`builtin` 与 `meta.math`）** | **已落盘审计**（[04-builtin-types-math-iterators-fmt.md](04-builtin-types-math-iterators-fmt.md)） | 原语类型、`?T`/`!E T`、`Tuple`/`Array`/`Slice`、`builtin.math`/`meta.math`、`iterator`、`fmt`、`text` (`str`)、`resources`、`mem` | 已严格遵守六内置包规则：`math` 归 `builtin.math`/`meta.math`（非第 7 包），`Once` 单一全称实现，`Option`/`Result` 零引入，`str` 仅含 UTF-8 | **Tier 1（`builtin`/`meta`）** | 保持精简；严禁将非 UTF-8 编码表、代数环/域 trait、随机数或正则放入 `builtin`。 |
| **`05`（`meta`/CMTT/宏/反射）** | **已落盘审计**（[05-meta-cmtt-macros-reflection.md](05-meta-cmtt-macros-reflection.md)） | `Type.*`、PTC A01–A19、`meta.reflection`、`meta.definitions`、`meta.data`、`meta.code`、`meta.ast`、`meta.macros`、`meta.compilation`、`meta.runtime.bounded_stack_bytes` | 全部属于编译器元编程与反射基础设施；第 1.3 节已显式划定程序验证归 `verification`、不污染 PTC | **Tier 1（`meta`/`attrs`）** | 严禁在 `meta` 中内置 JSON/OpenAPI Schema 模型或具体正则编译器（仅提供 `code_match`/`code_sequence`/`RecordInfo` 通用砖块）。 |
| **`06`（`attrs`/`build`/`verification`）** | 待落盘（按官方规范审计） | `__flurry_*` 官方键、`^attrs.*`、`build.*`、`verification` 契约 | 仅承载编译器消费者与构建系统消费的官方键与基线接口 | **Tier 1（`attrs`/`build`/`verification`）** | 严禁将 Tier 2 领域元数据（如 `serialization_rename`、`db_column`、`cli_flag`、`validate_min_len`）注册为 `__flurry_*` 内部键。 |
| **`07`（`std` 核心运行时库）** | **已落盘审计**（[07-std-mem-collections-io-os.md](07-std-mem-collections-io-os.md)） | 1. 明确排除了 `regex`/HTTP/TLS/SQL/JSON/crypto；<br>2. **边界项**：`SoA<T>`、`SlotMap<K, V>`、`Arena<T>`+`NodeIndex`、`SmallVec<T, N>`、`RingBuffer<T, N>`、`BitSet`、高级分配器（`SlabAllocator`/`PoolAllocator`/`ScratchAllocator`/`TrackingAllocator`/`FailoverAllocator`）；<br>3. `std.time`（`Instant`/`Duration`/`SystemTime`）；<br>4. `std.os.linux.getrandom` | `07` 已将 `SoA`/`SlotMap`/`SmallVec`/`RingBuffer` 标注为待作者裁决边界项（line 77, §5 Q11）；`std.time` 严守单调/时间戳底线；`getrandom` 属于 OS 原始系统调用包装 | **`Vec`/`VecDeque`/`HashMap`/`HashSet`/`BTree*`/`BinaryHeap`/`Slice`/`MutSlice`/`Arena` 归 Tier 1；`SoA`/`SlotMap`/图/持久化容器及争议项见 §3.17 与 §5 Q3** | 明确区分 Tier 1 系统基线容器与 Tier 2 `collections_ext` 协议边界；绝不允许日历/时区、`tzdata` 或 RNG 分布引擎进入 `std.time`/`std.os`。 |
| **`08`（效应、并发、网络与运行时）** | 待落盘（按 Board #10 作用域审计） | `builtin.Continuation`、`HandleError`/`HandleEffect`、`std.runtime.StackPool`/`BoundedStackPool`、`async effect` 核心、`std.sync`、`std.net` 基础 socket vs `Log`/`Clock`/`Random` 效应与具体 `io_uring`/线程池执行器 | 核心 ABI 与 OS 同步原语属于 Tier 1；可插拔异步调度引擎与领域效应属于 Tier 2/3 | **核心 ABI / `std.sync` / 原始 socket 归 Tier 1；`Executor`/`Reactor`/`Log`/`Random`/`Clock` 归 Tier 2；`io_uring_rt`/`otel` 归 Tier 3** | 严禁在 `08` 中把 `io_uring` 事件循环、`Log` 效应、CSPRNG 或 HTTP 客户端写入 `std`。 |
| **`09`（管线投影、张量代数与验证）** | 待落盘（按 Board #10 作用域审计） | `^attrs.compilation.pipeline`、`meta.compilation.IIR.Func`、`meta.runtime.invoke`、`verification` 边界 vs `Tensor` 运行时后端（`cuda`/`cpu_blas`/Metal/WGPU）、代数抽象、`GpuProjection` | 编译管线投影机制与静态形状编译期检查归 Tier 1；运行时张量存储/算子派发/GPU 内核启动库属于 Tier 2/3 | **投影机制与静态形状阶段规则归 Tier 1；`numerics`/`tensor_protocol` 归 Tier 2；`cuda`/`cpu_blas` 归 Tier 3** | 严禁将 `cuda`、`cuda_compile`、cuBLAS 或动态张量调度库写入 `builtin`/`std`/`meta`。 |

### 1.5 Tier 1 -> Tier 2 -> Tier 3 包依赖拓扑与归属原则

```text
Tier 1 (6 Builtin Packages — Lean Compiler & OS Baseline)
  builtin ──▶ meta ──▶ attrs ──▶ build ──▶ std ──▶ verification
     ▲         ▲        ▲         ▲         ▲
     └─────────┴────────┴─────────┴─────────┘ (仅单向依赖 Tier 1，严禁反向依赖 Tier 2/3)

Tier 2 (Official Thin Protocol Packages — Outside the 6 Builtin Packages)
  [基础零/低耦合协议]
    serialization   digest   random   telemetry   datetime   color   numerics   collections_ext
         │             │       │          │                                           │
         ▼             ▼       ▼          │                                           ▼
    encoding/compress  crypto  (async_runtime ◀── std.runtime.StackPool)        tensor_protocol
         │                                │                                           ▲
         ▼                                ▼                                           │
    validation / schema     wire ──▶ http / database         cli / config        (ffi_bridge / plugin_abi)
    regex_protocol / parser_protocol                         testing ──▶ random

Tier 3 (Ecosystem Implementation & Deriver Packages)
  json_lib, cbor_lib, blake3, chacha_poly, pcg_rng, io_uring_rt, epoll_rt, silver,
  postgres, zstd_lib, tzdata_iana, regex_dfa, cuda, cuda_compile, cpu_blas, otel_exporter
```

---

## 2. 现状与缺口全景总表

当前仓库仅有 Tier 1 六内置包的局部源码基线（`library/builtin`、`library/meta`、`library/attrs`、`library/build`、`library/std`、`library/verification`），Tier 2 官方薄协议包目录与包管理分发通道尚未建立，同时编译器层面尚缺支持“外包零特权、参数化 `trait`、零分配/运行时依赖 `allocator` 透传派生”的若干关键能力。以下 `WL-10-01` 至 `WL-10-22` 覆盖编译器支撑缺口及全部 17 组 Tier 2 协议包需求：

| ID | 领域 / 协议包 | 归属层与包路径 | 核心内容 / 符号 | 现状（核查仓库事实） | 优先级 | 核心编译器与 Tier 1 依赖 |
|---|---|---|---|---|---|---|
| `WL-10-01` | 参数化 Trait 与 `for<Symbol>` 语义对齐 | 编译器 + 语言规范 | `trait Name where p: Symbol`（与 `for<Symbol>` 关系裁决）、成员类型应用 `x.method<.tag>()` | `docs/traits/derivation.md` 与 `protocols.md` 已定义 `where format: Symbol` 及 `student.serialize<.json>()`；编译器部分泛型值参数 `trait` 已接入，但开放 `Symbol` 参数化派生匹配与 `for<Symbol>` 简写关系尚待收口（见 §5 Q1） | **P0** | `compiler/elaboration`、`normalization`、`docs/traits/declarations.md` |
| `WL-10-02` | 外源类型适配与 `derive ... .use(...)` 一致性 | 编译器 + Trait 系统 | `derive Trait<.tag>.use(strategy) for Type;`、`newtype` `.as(View)`、`^attrs.derivation.default` 孤儿限制 | `docs/traits/derivation.md`、`coherence.md`、`receiver-adaptation.md` 已定规范；编译器 `derivation` 对策略参数、`.use(...)` 显式路径及双向 `newtype` cast 视角待全路径打通 | **P0** | `compiler/elaboration`、`coherence`、`receiver_adaptation` |
| `WL-10-03` | Tier 2 非 `__flurry_*` 元数据属性隔离与合并 | 编译器 + `attrs` + `meta` | 领域属性（如 `^serialization.attrs.rename`）非 `__flurry_*` 键、多属性不互盖策略 | `official-attribute-keys.md` 禁止 Tier 2 使用 `__flurry_*`；但当前 `attribute.md` 浅合并会在多次使用 `^attrs.metadata(...)` 时覆写同名 `__flurry_metadata` 键（见 §5 Q4） | **P0** | `compiler/elaboration`（`attributes`）、`meta.reflection` |
| `WL-10-04` | 跨 Trait/Deriver 的 `allocator` 环境透传 | 编译器 + `std.mem` | `derive ... for Data<.A, .allocator>`、泛型 trait 方法上的 `quote .allocator: lift *A` 环境合成 | `library/std/mem_impl/boxed.fl` 已示范结构体级 `quote .allocator`；`issues/0001` 记录闭包/trait 方法/派生产物中的 `RuntimeEnvParameter` 透传仍受阻 | **P0** | `compiler/elaboration`（`runtime_env`）、`materialization`、`issues/0001` |
| `WL-10-05` | Tier 2 双相 `pure comptime trait` 同名协议接入 | 编译器 + `meta` | 运行时 `trait` 与编译时 `pure comptime trait Comptime*` 同名方法优先级与 `self` 值引用 | `docs/comptime/the-comptime-phases-of-traits.md` 已确立规则；编译器按表达阶段切换优先层尚待全量接入 | **P0** | `compiler/elaboration`（`member_resolution`）、`comptime_eval` |
| `WL-10-06` | `(1) serialization` | Tier 2 `serialization` | `Serialize`、`Deserialize`、`StreamingSerialize`、`ZeroCopyDeserialize`、`Schema`（`format: Symbol`）；`Comptime*` 对偶；`SerializeErr`/`DeserializeErr`/`SchemaErr`；领域属性（无 `__flurry_*`） | 仓库无 `serialization` 包；仅在 `proposal_trace/2026-8-13-derivation.fl` 留有草案，且 `docs/traits/protocols.md` 引用示例路径 | **P0** | `WL-10-01`–`05`、`builtin.fmt.ByteWriter`、`std.mem.Allocator`、`meta.reflection` |
| `WL-10-07` | `(2) telemetry` | Tier 2 `telemetry` | `Severity`、`TelemetryErr`、`LogValue`、`StructuredField`、`LogField`、`effect enum {Log, Trace, Metric}`、`ComptimeLogScalar` | 仓库空白；无统一非侵入效应日志/追踪/指标接口 | **P1** | `docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md`、`builtin.fmt` |
| `WL-10-08` | `(3) crypto` & `digest` | Tier 2 `digest` + `crypto` | `Digest<alg: Symbol>`、`ExtendableOutput`、`ComptimeDigest`、`Mac`、`Aead`、`Kdf`、`Signer`、`Verifier`、`KeyExchange`、`ConstantTimeEq`、`Zeroize`、`effect csprng_fill` | 仓库空白；`^attrs.mem.volatile` 已在编译器端支持字段级易失写（助力 `Zeroize`） | **P1** | `builtin.Array`、`std.mem.Allocator`、`^attrs.mem.volatile` |
| `WL-10-09` | `(4) random` | Tier 2 `random` | `RngCore`、`SeedableRng`、`CryptoRng`、`Distribution<T>`、`Uniform<T>`、`ComptimeRngCore`、`effect {random_bytes, random_u64}`、`RngErr` | 仓库空白；`std.os.linux` 尚未包装 `getrandom` 到独立协议 | **P1** | `builtin.Array`、`std.collections.MutSlice`、`algebraic-effect` |
| `WL-10-10` | `(5) async_runtime` | Tier 2 `async_runtime` | `Executor`、`TaskSpawner`、`Reactor`、`Timer`、`AsyncReader`、`AsyncWriter`、`PollState<T>`、`WakerHandle`、`async effect {io_wait, sleep_duration}`、`AsyncRuntimeOps` | 仓库空白；依赖 `08` 中的 `std.runtime.StackPool`、`builtin.Continuation` 与 `async effect` 调度 ABI | **P0** | `08` 最小协程栈池 ABI、`async-algebraic-effect.md`、`std.time.Duration` |
| `WL-10-11` | `(6) http` & `wire` | Tier 2 `wire` + `http` | `Codec<proto: Symbol>`、`Uri`、`WireErr`、`Method`、`StatusCode`、`Version`、`Headers`、`HttpBody`、`HttpRequest`/`HttpResponse`、`HttpService`、`Middleware`、`HttpErr` | 仓库空白；需 `async_runtime` 与 `std.mem` 分配器范式支撑 | **P1** | `async_runtime`、`builtin.fmt.ByteWriter`、`std.mem.Allocator` |
| `WL-10-12` | `(7) database` | Tier 2 `database` | `ToSql<dialect: Symbol>`、`FromSql<dialect: Symbol>`、`FromRow<dialect: Symbol>`、`Connection`、`Transaction`、`SqlValueRef`、`IsolationLevel`、`SqlErr`、`database.attrs` | 仓库空白；依赖 `WL-10-01..04` 与记录字段反射 | **P1** | `WL-10-01..04`、`meta.reflection`、`std.mem.Allocator` |
| `WL-10-13` | `(8) compression` & `encoding` | Tier 2 `compression` + `encoding` | `Compress<codec: Symbol>`、`Decompress<codec: Symbol>`、`TextCodec<charset: Symbol>`、`WireEncode<scheme: Symbol>`、对应 `Comptime*` 对偶、`CompressErr`/`CodecErr` | 仓库空白；`builtin.text` 与 `std.string` 仅保留 UTF-8 基线 | **P2** | `builtin.fmt.ByteWriter`、`std.collections.{Slice, MutSlice, Vec}` |
| `WL-10-14` | `(9) validation` & `schema` | Tier 2 `validation` + `schema` | `Validate<profile: Symbol>`、`ValidationReport`、`SchemaDescribe<spec: Symbol>`、`SchemaDoc`、`ComptimeValidate`、`ComptimeSchemaSummary`、`ValidationErr`/`SchemaSpecErr`、`validation.attrs` | 仓库空白；依赖匿名记录/字段属性反射（`05` `WL-05-12`/`16`） | **P2** | `meta.reflection`、`WL-10-03`、`std.mem.Allocator` |
| `WL-10-15` | `(10) cli` & `config` | Tier 2 `cli` + `config` | `FromArgs<style: Symbol>`、`ArgDescriptor`、`ConfigLoad<format: Symbol>`、`effect {config_get, config_require}`、`CliErr`/`ConfigErr`、`cli.attrs` | 仓库空白；基于 `std.env` 原始参数与代数效应配置读取 | **P2** | `std.env`、`std.collections.Slice`、`meta.reflection` |
| `WL-10-16` | `(11) datetime` | Tier 2 `datetime` | `Calendar<system: Symbol>`、`TimeZone`、`DateTimeFormat<fmt: Symbol>`、`CivilDate`/`CivilTime`/`UtcOffset`、`effect {system_clock, local_timezone}`、`Comptime*`、`DateTimeErr` | 仓库空白；严格与 `std.time`（仅 `Instant`/`Duration`/`SystemTime`）解耦 | **P2** | `std.time.{Duration, SystemTime}`、`builtin.fmt.ByteWriter` |
| `WL-10-17` | `(12) media` & `color` | Tier 2 `color` + `media` | `ColorSpace<space: Symbol>`、`PixelLayout`、`ImageDescriptor`、`ImageEncode<fmt: Symbol>`、`ImageDecode<fmt: Symbol>`、`AudioCodec<fmt: Symbol>`、`ColorErr`/`MediaErr` | 仓库空白；可与 `std.simd` 对接但零内建编解码算法 | **P3** | `builtin.Array`、`builtin.fmt.ByteWriter`、`std.mem.Allocator` |
| `WL-10-18` | `(13) numerics` & `tensor_protocol` | Tier 2 `numerics` + `tensor_protocol` | `Semigroup`/`Monoid`/`Group`/`Ring`/`Field`/`VectorSpace`/`InnerProductSpace`/`NormedSpace`、`TensorStorage<device: Symbol>`、`TensorOp<backend: Symbol>`、`Differentiable<mode: Symbol>`、`GpuProjection<target: Symbol>`、`NumericErr`/`TensorErr` | 仓库仅在 `docs/comptime/static-shaped-tensors.md` 与 `proposal_trace/2026-9-7-tensor-algebra.fl` 有概念原型；无库协议 | **P2** | `docs/pipeline-projection/`、`meta.compilation.IIR`、`std.mem.Layout` |
| `WL-10-19` | `(14) testing` | Tier 2 `testing` | `Arbitrary<strategy: Symbol>`、`Shrink`、`Bencher`、`TestReporter`、`TestOutcome`、`TestCaseInfo`、`TestErr`、与 `template test` 集成 | 编译器仅支持基础 `test {}` 块；`template test` 与属性测试协议均待接通 | **P1** | `random.RngCore`、`builtin.iterator.emit_item`、`template test` 语法 |
| `WL-10-20` | `(15) ffi_bridge` & `plugin_abi` | Tier 2 `ffi_bridge` + `plugin_abi` | `FfiMarshal<abi: Symbol>`、`StableAbi`、`AbiDescriptor`、`PluginHeader`、`FfiMarshalErr`、集成 `Type.type_id` 与 `^attrs.stable_since` | `docs/type/type_id.md` 标记不稳定且尚未接入编译器 NIR/LLVM；`std.ffi` 仅部分支持标量 C ABI | **P1** | `docs/type/type_id.md`、`^attrs.stable_since`、`^attrs.abi`、`std.ffi` |
| `WL-10-21` | `(16) regex_protocol` & `parser_protocol` | Tier 2 `regex_protocol` + `parser_protocol` | `PatternCompile<flavor: Symbol>`、`RegexMatchOps`、`RegexSpan`、`RegexFlags`、`RegexErr`、`Parser<grammar: Symbol>`、`ParseCursor`、`ComptimeParserCompiler`、`ParseErr` | `02` WL-02-13..15 已确立 `TuplePatternView` 与 Tier 2/3 `user_regex` 示例；需抽离可复用 Tier 2 编译协议并打通 CMTT `code_match`/`code_sequence` | **P0**（契约）/ **P1**（实现） | `meta.patterns.TuplePatternView`、`builtin.ConcatPatternView`、`meta.code` |
| `WL-10-22` | `(17) collections_ext` | Tier 2 `collections_ext` | `PersistentSeq`、`PersistentMap`、`GraphView`、`MutGraph`、`GenerationalArena`、`SoaTable`、`GraphErr`；同时收容从 `std.collections` 界定拆出的扩展 DOD/图/持久化协议 | `07` 已实现/规划 `Vec`/`VecDeque`/`HashMap`/`HashSet`/`SlotMap`/`SoA`/`SmallVec`/`RingBuffer` 并挂起 Q11 划界 | **P2** | `std.mem.Allocator`、`builtin.iterator.emit_item`、运行期依赖类型（`07`/`05`） |

---

## 3. 详细规格设计

> **Flurry 规范声明**：本节全部接口代码严格遵守 `docs/` 与 `AGENTS.md` 规范——使用原生 `?T` / `!E T` / `![E1, E2] T`（零 `Option`/`Result`）、点分路径（零 `::`）、`and`/`or`/`not`、`--`/`{- -}` 注释、`PascalCaseErr`/`PascalCase` 错误变体/`snake_case` 普通变体、集中绑定别名（零使用处 `.symbol'builtin`）、运行时依赖 `allocator` 子句（零硬编码 `SystemAllocator`）、同名双阶段 `pure comptime trait`（`self` 值引用、零 comptime 指针）、多行 `where`/`requires` 后对齐 `{`、无尾随 `};`、Type 返回函数 `PascalCase`，以及 **Tier 2 领域元数据禁止使用 `__flurry_*` 前缀**。

### 3.0 Tier 2 跨包通用机制、一致性（Coherence）与属性隔离契约

在展开 17 个官方协议包前，必须先确立 4 条跨越所有 Tier 2 包的编译器与设计不变式：

1. **参数化协议与派生策略（`where format: Symbol` + `derive ... .use(...)`）**
   - Tier 2 协议采用带 `Symbol` 领域标签的参数化 `trait`（如 `pub trait Serialize where format: Symbol`），使不同格式/方言/算法实例（`Serialize<.json>`、`Serialize<.cbor>`、`Digest<.sha256>`、`FromRow<.postgres>`）拥有彼此独立的完整接口实例身份，互不冲突。
   - **默认策略禁止越权**：依 [docs/traits/derivation.md](../../docs/traits/derivation.md#显式与默认选择)，`^attrs.derivation.default` 注册要求**要么策略绑定到本地 trait，要么首个本地名义身份前不得出现自由参数**；`Symbol` 值在任何包均可构造，不授予包归属权。因此，Tier 3 实现包（如 `json_lib`）不得为 `Serialize<.json>` 注册全局默认策略，而是导出具名策略（如 `pub derive fn without_allocation for Serialize<.json>`），由类型定义所在包显式写出：
     ```flurry
     derive Serialize<.json>.use(json_lib.without_allocation) for Data;
     ```
2. **外源类型一致性（Coherence）与 `.as(View)` 显式视角适配**
   - 依 [docs/traits/coherence.md](../../docs/traits/coherence.md#定义归属)，`Symbol`、`str`、数值等普通值不授予孤儿豁免；Tier 3 包为 Tier 1 外源类型实现 Tier 2 外部 trait（例如在 `json_lib` 中写 `impl Serialize<.json> for std.collections.Slice<u8>`）**必须被编译器静态拒绝**。
   - 标准解法：Tier 3 包定义透明布局零开销的 `newtype` 包装（如 `pub newtype JsonSlice<T> = Slice<T>;`），并利用 [docs/type/receiver-adaptation.md](../../docs/type/receiver-adaptation.md#类型视角) 提供的 `.as(View)` 类型视角在调用点显式或绑定处声明适配：
     ```flurry
     -- 在 json_lib 包内：JsonSlice 为本地名义类型，满足有序本地身份规则
     use serialization.Serialize;
     use std.collections.Slice;

     pub newtype JsonBytes = Slice<u8>;

     impl Serialize<.json> for JsonBytes {
         fn serialize_into(*self, writer: *W) -> ![SerializeErr, W.as(ByteWriter).Err] void
         where
             W :- ByteWriter,
         {
             -- 按 JSON 字节数组或 base64 编码写入 writer
             writer.write_str("[]")
         }
     }

     -- 调用端通过视角适配器使用，不污染全局 Slice<u8> 实现世界
     fn emit_bytes(bytes: Slice<u8>.as(JsonBytes), out: *W) -> ![SerializeErr, W.as(ByteWriter).Err] void
     where
         W :- ByteWriter,
     {
         bytes.serialize_into<.json>(out)
     }
     ```
3. **Tier 2 领域元数据属性（严禁 `__flurry_*` 前缀）**
   - 依 [docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md)，`__flurry_*` 仅限编译器核心消费者与 `library/attrs/` 使用；Tier 2 领域属性（如字段重命名 `^serialization.attrs.rename("id")`、数据库列名 `^database.attrs.column("user_id")`、校验范围 `^validation.attrs.min_len(3)`）**一律使用领域前缀普通键**（如 `{ serialization_rename: name }`、`{ validation_min_len: n }`），避免重复叠加属性时触发 `__flurry_metadata` 浅覆盖（参见 §5 Q4），亦绝不占用 `__flurry_*` 保留空间。
4. **反例：跨层越界与规范违例的静态拒绝（通用负例集）**
   ```flurry
   -- 负例 1（拒绝：外源类型孤儿冲突）：json_lib 既不拥有 Serialize，也不拥有 Slice<u8>；.json Symbol 不授予归属权
   {-
   impl serialization.Serialize<.json> for std.collections.Slice<u8> {
       fn serialize_into(*self, writer: *W) -> ![serialization.SerializeErr, W.as(builtin.fmt.ByteWriter).Err] void
       where
           W :- builtin.fmt.ByteWriter,
       {
           writer.write_str("[]")
       }
   }
   -}

   -- 负例 2（拒绝：Tier 2 领域属性非法冒用官方 __flurry_* 前缀）
   {-
   pub pure comptime fn bad_rename(name: str) -> meta.Object {
       { __flurry_serialization_rename: name } -- 编译/规范审计拒绝：非 attrs 官方键禁止 __flurry_* 前缀
   }
   -}

   -- 负例 3（拒绝：在 pure comptime trait 中使用 comptime 指针或 *self）
   {-
   pub pure comptime trait BadComptimeDigest
   where
       alg: Symbol,
   {
       comptime fn update(*self, data: meta.List<u8>); -- 编译错误：comptime 无指针与 *self，应为 `self` 值引用
   }
   -}

   -- 负例 4（拒绝：硬编码 SystemAllocator 或隐藏分配依赖）
   {-
   pub struct BadHttpHeaders {
       raw: std.collections.Vec<u8, .A = std.mem.SystemAllocator, .allocator = std.mem.SystemAllocator.GLOBAL.ref>,
   } -- 规范拒绝：必须通过 `where .A: Type = DefaultAllocator, requires A :- Allocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref` 透传
   -}
   ```

---

### 3.1 包 `(1)`：`serialization`（序列化与结构化数据交换协议）

**职责定位**：提供格式无关、零隐式堆分配的编译期与运行时序列化/反序列化/流式编码/零拷贝解码与结构描述协议。**Tier 2 包内不包含任何 JSON/CBOR/TOML/YAML 状态机**；具体格式逻辑由 Tier 3（`json_lib`、`cbor_lib`、`toml_lib`）实现或派生。

#### 3.1.1 核心错误、阶段对偶与协议定义

```flurry
use builtin.{Array, bool, str, Symbol, Type, u8, usize, void}
use builtin.fmt.ByteWriter;
use meta.{List, Object}
use std.collections.{Slice, Vec}
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum SerializeErr {
    UnsupportedType,
    InvalidValue,
    DepthLimitExceeded,
    SequenceLengthUnknown,
    WriterFailed,
    AllocFailed,
    Custom(str),
}

pub enum DeserializeErr {
    UnexpectedEof,
    SyntaxError,
    TypeMismatch,
    MissingField,
    DuplicateField,
    UnknownField,
    NumericOverflow,
    InvalidUtf8,
    TrailingData,
    DepthLimitExceeded,
    AllocFailed,
    Custom(str),
}

pub enum SchemaErr {
    UnsupportedType,
    CycleDetected,
    InvalidMetadata,
}

pub enum SchemaKind {
    unit_kind,
    bool_kind,
    signed_int_kind(u16),
    unsigned_int_kind(u16),
    float_kind(u16),
    char_kind,
    string_kind,
    bytes_kind,
    optional_kind,
    seq_kind,
    map_kind,
    struct_kind,
    enum_kind,
    tuple_kind,
}

pub struct SchemaField {
    wire_name: str,
    required: bool,
    type_name: str,
}

pub struct SchemaDescriptor
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    kind: SchemaKind,
    name: str,
    fields: Vec<SchemaField, .A = A, .allocator = allocator>,
}

-- 1. 核心序列化协议（默认 serialize_into 写入 ByteWriter 实现零堆分配；默认方法 serialize 支持自定义 allocator）
pub trait Serialize
where
    format: Symbol,
{
    fn serialize_into(*self, writer: *W) -> ![SerializeErr, W.as(ByteWriter).Err] void
    where
        W :- ByteWriter;

    fn serialize(*self) -> ![SerializeErr, AllocErr] Vec<u8, .A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
    {
        var buffer = Vec<u8, .A = A, .allocator = allocator>.new();
        self.serialize_into(buffer.ref)!;
        buffer
    }
}

-- 2. 流式增量序列化协议（适用于大对象/分块网络发送，不要求一次性落入连续 Vec）
pub enum StreamStep {
    more_needed,
    completed,
}

pub trait StreamingSerialize
where
    format: Symbol,
{
    assoc EncoderState: Type;

    fn begin_stream(*self) -> !SerializeErr EncoderState;
    fn step_stream(state: *EncoderState, writer: *W) -> ![SerializeErr, W.as(ByteWriter).Err] StreamStep
    where
        W :- ByteWriter;
    fn finish_stream(state: EncoderState) -> !SerializeErr void;
}

-- 3. 拥有型反序列化协议（按调用方提供的 runtime-dependent allocator 构造目标值）
pub trait Deserialize
where
    format: Symbol,
{
    fn deserialize(input: Slice<u8>) -> ![DeserializeErr, AllocErr] Self
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

-- 4. 零拷贝反序列化协议（视图直接借用 input，可达性受原 Slice<u8> 寿命约束，零 Allocator 需求）
pub trait ZeroCopyDeserialize
where
    format: Symbol,
{
    fn deserialize_borrowed(input: Slice<u8>) -> !DeserializeErr Self;
}

-- 5. 结构模式描述协议（编译时 Schema 推导）
pub trait Schema
where
    format: Symbol,
{
    pure comptime fn schema_kind() -> !SchemaErr SchemaKind;
    pure comptime fn schema_fields() -> !SchemaErr List<SchemaField>;
}

-- 6. 编译时同名双相协议（self 值引用语义，零指针、零 comptime_ 前缀）
pub pure comptime trait ComptimeSerialize
where
    format: Symbol,
{
    comptime fn serialize(self) -> !SerializeErr List<u8>;
    comptime fn serialize_str(self) -> !SerializeErr str;
}

pub pure comptime trait ComptimeDeserialize
where
    format: Symbol,
{
    comptime fn deserialize(input: List<u8>) -> !DeserializeErr Self;
}
```

#### 3.1.2 领域元数据属性（无 `__flurry_*`）

```flurry
pub mod attrs {
    use builtin.{bool, str}
    use meta.Object;

    pub pure comptime fn rename(wire_name: str) -> Object {
        { serialization_rename: wire_name }
    }

    pub const skip: Object = { serialization_skip: true }
    pub const skip_if_null: Object = { serialization_skip_if_null: true }
    pub const flatten: Object = { serialization_flatten: true }

    pub pure comptime fn default_with(provider_name: str) -> Object {
        { serialization_default_fn: provider_name }
    }

    pub pure comptime fn tag_field(tag_name: str) -> Object {
        { serialization_tag_field: tag_name }
    }
}
```

#### 3.1.3 完整工作示例：零分配 JSON 派生、CBOR 派生与运行期 Allocator 透传

```flurry
-- 在 Tier 3 包 json_lib 中定义具名派生策略（不注册默认策略）
mod json_lib {
    use builtin.{bool, str, Symbol, Type, void}
    use builtin.fmt.{ByteWriter, Display}
    use meta.List;
    use meta.reflection;
    use meta.reflection.{AttributeArg, AttributeInfo}
    use serialization.{Deserialize, DeserializeErr, Serialize, SerializeErr}
    use std.collections.{Slice, Vec}
    use std.mem.{AllocErr, Allocator, DefaultAllocator}

    pure comptime fn has_attr_flag(attrs: List<AttributeInfo>, key: Symbol) -> bool {
        inline for item in attrs {
            item.named_args.get(key) match {
                arg? => {
                    arg match {
                        .bool_lit(flag) => return flag,
                        _ => {},
                    }
                },
                null => {},
            }
        }
        false
    }

    pure comptime fn get_attr_str(attrs: List<AttributeInfo>, key: Symbol) -> ?str {
        inline for item in attrs {
            item.named_args.get(key) match {
                arg? => {
                    arg match {
                        .str_lit(text) => return text,
                        _ => {},
                    }
                },
                null => {},
            }
        }
        null
    }

    pub derive fn without_allocation for Serialize<.json> {
        const info = reflection.reflect_struct(Self) match {
            ok! => ok,
            error _ => meta.compilation.err("Serialize<.json> requires nominal struct"),
        }

        fn serialize_into(*self, writer: *W) -> ![SerializeErr, W.as(ByteWriter).Err] void
        where
            W :- ByteWriter,
        {
            writer.write_byte(123u8)!; -- '{'
            var emitted_any = false;
            inline for field in info.fields {
                const should_skip = has_attr_flag(field.attributes, .serialization_skip);
                inline if not should_skip {
                    if emitted_any {
                        writer.write_byte(44u8)!; -- ','
                    }
                    const wire = get_attr_str(field.attributes, .serialization_rename) match {
                        renamed? => renamed,
                        null => field.name match {
                            sym? => sym'tag_name,
                            null => meta.compilation.err("Serialize<.json> requires named struct fields"),
                        },
                    }
                    const offset = field.byte_offset match {
                        off? => off,
                        null => meta.compilation.err("bit-packed field without byte offset is unsupported"),
                    }
                    writer.write_byte(34u8)!;
                    writer.write_str(wire)!;
                    writer.write_str("\":")!;
                    let field_addr = builtin.mem.checked_add_usize(self'bitvec.as(usize), offset) match {
                        addr? => addr,
                        null => return error SerializeErr.InvalidValue,
                    }
                    let field_ptr = unsafe { field_addr'bitvec.as(*field.FieldType()) }
                    field_ptr.as(Serialize<.json>).serialize_into(writer)!;
                    emitted_any = true;
                }
            }
            writer.write_byte(125u8) -- '}'
        }
    }
}

-- 在 Tier 3 包 cbor_lib 中定义二进制定长编码策略
mod cbor_lib {
    use builtin.fmt.ByteWriter;
    use serialization.{Serialize, SerializeErr}

    pub derive fn canonical_ordered for Serialize<.cbor> {
        fn serialize_into(*self, writer: *W) -> ![SerializeErr, W.as(ByteWriter).Err] void
        where
            W :- ByteWriter,
        {
            -- 按 RFC 8949 规范化键顺序编码到 writer
            writer.write_byte(160u8)
        }
    }
}

-- 在用户业务包中：同时派生多种格式策略并支持自定义 FixedBufferAllocator 透传
use serialization.Serialize;
use std.collections.Vec;
use std.mem.{AllocErr, Allocator, DefaultAllocator, FixedBufferAllocator, RawBlock}
use std.string.String;

pub struct Data
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    ^serialization.attrs.rename("user_id")
    id: u64,
    ^serialization.attrs.skip
    internal_token: u32,
    label: String<.A = A, .allocator = allocator>,
}

-- 为闭合/参数化实例派生 JSON 与 CBOR，并正确透传 Data 自身的 allocator 环境
derive Serialize<.json>.use(json_lib.without_allocation) for Data<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A;

derive Serialize<.cbor>.use(cbor_lib.canonical_ordered) for Data<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A;

fn encode_in_arena(
    item: *Data<.A = FixedBufferAllocator, .allocator = fba_ptr>,
    scratch_fba: *FixedBufferAllocator,
) -> ![SerializeErr, AllocErr] Vec<u8, .A = FixedBufferAllocator, .allocator = scratch_fba>
where
    quote fba_ptr: lift *FixedBufferAllocator,
{
    -- 使用显式依赖的 scratch_fba 分配序列化输出缓冲，Data 内部使用 fba_ptr，全程零全局堆分配
    item.as(Serialize<.json>).serialize<.A = FixedBufferAllocator, .allocator = scratch_fba>()
}
```

---

### 3.2 包 `(2)`：`telemetry`（结构化日志、分布式追踪与指标协议）

**职责定位**：通过薄代数 `effect enum` 与零分配结构化字段访问器解耦应用埋点与日志/追踪/指标收集后端。**Tier 2 不包含格式化落盘器、采样器、环形队列或 OTLP/Prometheus 客户端**。

```flurry
use builtin.{bool, f64, i64, str, Symbol, Type, u64, void}
use std.collections.Slice;

pub enum Severity {
    trace,
    debug,
    info,
    warn,
    error,
    fatal,
}

pub enum TelemetryErr {
    SinkUnavailable,
    BufferFull,
    InvalidSpan,
    ExportFailed,
}

pub newtype SpanId = u64;

pub enum FieldScalar {
    bool_val(bool),
    i64_val(i64),
    u64_val(u64),
    f64_val(f64),
    str_val(str),
    bytes_val(Slice<u8>),
    symbol_val(Symbol),
}

pub struct LogField {
    key: str,
    value: FieldScalar,
}

pub trait LogFieldSink {
    fn emit_field(*self, key: str, value: FieldScalar);
}

pub trait LogValue {
    fn emit_log_value(*self, key: str, sink: *S)
    where
        S :- LogFieldSink;
}

pub trait StructuredField {
    fn record_fields(*self, sink: *S)
    where
        S :- LogFieldSink;
}

-- 单相编译期日志标量摘要接口（非 runtime LogValue 的双相孪生，直接返回 FieldScalar 值）
pub pure comptime trait ComptimeLogScalar {
    comptime fn to_log_scalar(self) -> FieldScalar;
}

-- 零分配核心效应：直接通过 handler 安装接入控制台、OTLP、或测试收集器
pub effect enum Log {
    event(level: Severity, target: str, message: str, fields: Slice<LogField>),
    enabled(level: Severity, target: str) -> bool,
}

pub effect enum Trace {
    enter_span(name: str, target: str, fields: Slice<LogField>) -> SpanId,
    exit_span(id: SpanId),
    record_field(id: SpanId, field: LogField),
}

pub effect enum Metric {
    counter_add(name: str, delta: u64, labels: Slice<LogField>),
    gauge_set(name: str, value: f64, labels: Slice<LogField>),
    histogram_record(name: str, value: f64, labels: Slice<LogField>),
}

pub mod attrs {
    use builtin.str;
    use meta.Object;

    pub const redact: Object = { telemetry_redact: true }

    pub pure comptime fn field_name(name: str) -> Object {
        { telemetry_field_name: name }
    }
}
```

- **派生器接入、效应拦截工作例与负例**：

```flurry
use builtin.{str, u64}
use std.collections.Slice;
use std.fmt.println;
use telemetry.{FieldScalar, Log, LogField, LogFieldSink, Severity, SpanId, StructuredField, Trace}

pub struct RequestMeta {
    request_id: u64,
    ^telemetry.attrs.redact
    auth_token: str,
}

derive StructuredField.use(telemetry.by_fields) for RequestMeta;

fn handle_request(meta_info: *RequestMeta) -> #[Log, Trace] u64 {
    let empty_fields: Slice<LogField> = Slice<LogField>.empty();
    let span = Trace.enter_span("http.handle", "api", empty_fields)#;
    Log.event(Severity.info, "api", "processing request", empty_fields)#;
    Trace.exit_span(span)#;
    meta_info.request_id.*
}

fn run_with_console_telemetry(meta_info: *RequestMeta) -> u64 {
    handle_request(meta_info)# {
        Log.event(_level, target, message, _fields) => println("[{}] {}", target, message)! {
            catch _ => {},
        },
        Log.enabled(_level, _target) => true,
        Trace.enter_span(_name, _target, _fields) => 1u64.as(SpanId),
        Trace.exit_span(_id) => {},
        Trace.record_field(_id, _field) => {},
    }
}
```

> **负例**：严禁在 `telemetry` 内置 `^attrs.effects.direct_passing` 修饰带参 `Log.event(...)`（`direct_passing` 仅适用于零参操作，见 [direct-passing.md](../../docs/algebraic-effect-and-delimited-continuation/direct-passing.md#参数不符合该-abi)）。

---

### 3.3 包 `(3)`：`digest` 与 `crypto`（密码学摘要与原语协议）

**职责定位**：分离无密钥散列摘要（`digest`）与带密钥密码学原语（`crypto`），全部以算法 `Symbol` 参数化（如 `.sha256`、`.blake3`、`.aes_256_gcm`、`.ed25519`、`.hkdf_sha256`、`.x25519`）。**Tier 2 绝不实现具体 S-box、轮函数或椭圆曲线运算**。

#### 3.3.1 `digest` 包完整签名

```flurry
use builtin.{Array, str, Symbol, Type, u8, usize, void}
use std.collections.{MutSlice, Slice}

pub enum DigestErr {
    InvalidLength,
    AlreadyFinalized,
    OutputTooSmall,
    UnsupportedAlgorithm,
}

pub trait Digest
where
    alg: Symbol,
{
    assoc DIGEST_BYTES: usize;
    assoc BLOCK_BYTES: usize;

    fn new() -> Self;
    fn update(*self, data: Slice<u8>);
    fn finalize(self) -> Array<u8, DIGEST_BYTES>;
    fn reset(*self);

    fn digest_once(data: Slice<u8>) -> Array<u8, DIGEST_BYTES> {
        var hasher = Self.new();
        hasher.ref.update(data);
        hasher.finalize()
    }
}

pub trait ExtendableOutput
where
    alg: Symbol,
{
    fn new_xof() -> Self;
    fn absorb(*self, data: Slice<u8>);
    fn squeeze(*self, output: MutSlice<u8>) -> !DigestErr void;
}

-- 编译期可校验哈希（例如构建脚本内容摘要、编译期常量查找表校验）
pub pure comptime trait ComptimeDigest
where
    alg: Symbol,
{
    assoc DIGEST_BYTES: usize;
    assoc BLOCK_BYTES: usize;

    comptime fn new() -> Self;
    comptime fn update(self, data: meta.List<u8>);
    comptime fn finalize(self) -> Array<u8, DIGEST_BYTES>;
    comptime fn reset(self);

    comptime fn digest_once(data: meta.List<u8>) -> Array<u8, DIGEST_BYTES> {
        var hasher = Self.new();
        hasher.update(data);
        hasher.finalize()
    }
}
```

#### 3.3.2 `crypto` 包完整签名

```flurry
use builtin.{Array, bool, Symbol, Type, u8, usize, void}
use std.collections.{MutSlice, Slice, Vec}
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum CryptoErr {
    InvalidKey,
    InvalidNonce,
    AuthenticationFailed,
    BufferTooSmall,
    InvalidSignature,
    KeyGenerationFailed,
    UnsupportedAlgorithm,
    EntropyFailed,
    AllocFailed,
}

-- 恒定时间相等比较（禁止短路提前返回，避免侧信道计时泄漏）
pub trait ConstantTimeEq {
    fn ct_eq(*self, other: *Self) -> bool;
}

-- 内存清零协议（实现须保证借助 ^attrs.mem.volatile 防止编译器死存储消除）
pub trait Zeroize {
    fn zeroize(*self);
}

pub trait Mac
where
    alg: Symbol,
{
    assoc KEY_BYTES: usize;
    assoc TAG_BYTES: usize;

    fn new_from_key(key: Slice<u8>) -> !CryptoErr Self;
    fn update(*self, data: Slice<u8>);
    fn finalize(self) -> Array<u8, TAG_BYTES>;
    fn verify_tag(self, expected_tag: Slice<u8>) -> !CryptoErr void;
}

pub trait Aead
where
    alg: Symbol,
{
    assoc KEY_BYTES: usize;
    assoc NONCE_BYTES: usize;
    assoc TAG_BYTES: usize;

    fn new(key: Array<u8, KEY_BYTES>) -> !CryptoErr Self;

    fn seal_in_place(
        *self,
        nonce: Array<u8, NONCE_BYTES>,
        aad: Slice<u8>,
        buffer: MutSlice<u8>,
        tag_out: MutSlice<u8>,
    ) -> !CryptoErr void;

    fn open_in_place(
        *self,
        nonce: Array<u8, NONCE_BYTES>,
        aad: Slice<u8>,
        buffer: MutSlice<u8>,
        tag: Slice<u8>,
    ) -> !CryptoErr void;

    fn seal(
        *self,
        nonce: Array<u8, NONCE_BYTES>,
        aad: Slice<u8>,
        plaintext: Slice<u8>,
    ) -> ![CryptoErr, AllocErr] Vec<u8, .A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

    fn open(
        *self,
        nonce: Array<u8, NONCE_BYTES>,
        aad: Slice<u8>,
        ciphertext_with_tag: Slice<u8>,
    ) -> ![CryptoErr, AllocErr] Vec<u8, .A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

pub trait Kdf
where
    alg: Symbol,
{
    fn derive_key(
        *self,
        ikm: Slice<u8>,
        salt: ?Slice<u8>,
        info: Slice<u8>,
        output: MutSlice<u8>,
    ) -> !CryptoErr void;
}

pub trait Signer
where
    alg: Symbol,
{
    assoc Signature: Type;
    assoc VerifyingKey: Type;

    fn sign(*self, message: Slice<u8>) -> !CryptoErr Signature;
    fn verifying_key(*self) -> VerifyingKey;
}

pub trait Verifier
where
    alg: Symbol,
{
    assoc Signature: Type;

    fn verify(*self, message: Slice<u8>, signature: *Signature) -> !CryptoErr void;
}

pub trait KeyExchange
where
    alg: Symbol,
{
    assoc PublicKey: Type;
    assoc SharedSecret: Type;

    fn public_key(*self) -> PublicKey;
    fn diffie_hellman(self, peer_public: *PublicKey) -> !CryptoErr SharedSecret;
}

pub effect csprng_fill(output: MutSlice<u8>) -> !CryptoErr void;
```

- **派生模式与安全性负例**：Tier 3/用户可编写 `derive Zeroize.use(crypto.zeroize_fields) for SecretKey;`（要求所有字段均实现 `Zeroize` 或为基础标量/`Array`/`BitVec`）。**负例**：为 `Aead.open` 返回明文却在 `AuthenticationFailed` 分支未清零原位 `buffer` 或跳过 `ConstantTimeEq` 用普通 `==` 比对 MAC tag，违背安全契约，静态/审计拒绝。

---

### 3.4 包 `(4)`：`random`（伪随机数生成器与效应熵源协议）

**职责定位**：定义核心 RNG trait、密码学安全标记、可复现种子协议、泛型分布采样以及用于纯化/注入随机性的薄代数效应。**不包含 PRNG 状态机、系统熵池缓存或复杂统计分布实现**。

```flurry
use builtin.{Array, bool, f64, i64, Type, u32, u64, u8, usize, void}
use std.collections.MutSlice;

pub enum RngErr {
    EntropyUnavailable,
    InvalidRange,
    EmptySlice,
    ReseedFailed,
}

pub trait RngCore {
    fn next_u32(*self) -> u32;
    fn next_u64(*self) -> u64;
    fn fill_bytes(*self, dest: MutSlice<u8>);
    fn try_fill_bytes(*self, dest: MutSlice<u8>) -> !RngErr void;
}

-- 密码学不可预测性标记契约（需要真随机或 CSPRNG）
pub trait CryptoRng :- RngCore {}

pub trait SeedableRng :- RngCore {
    assoc SEED_BYTES: usize;

    fn from_seed(seed: Array<u8, SEED_BYTES>) -> Self;
    fn from_rng(source: *R) -> !RngErr Self
    where
        R :- RngCore;
}

pub trait Distribution
where
    T,
{
    fn sample(*self, rng: *R) -> T
    where
        R :- RngCore;
}

pub struct Uniform
where
    T,
{
    low: T,
    high_exclusive: T,
}

pub trait RngExt :- RngCore {
    fn sample_with(*self, dist: *D) -> T
    where
        T,
        D :- Distribution<T>,
    {
        dist.sample(self)
    }

    fn gen_bool(*self, probability: f64) -> !RngErr bool {
        if probability < 0.0 or probability > 1.0 {
            return error RngErr.InvalidRange;
        }
        let scale: f64 = (self.next_u64() / 2048u64).as(f64) / 9007199254740992.0;
        scale < probability
    }
}

pub pure comptime trait ComptimeRngCore {
    comptime fn next_u32(self) -> u32;
    comptime fn next_u64(self) -> u64;
}

-- 薄随机效应：允许业务函数仅声明 `#random_bytes` 而无需绑定具体 PRNG，在测试中用确定性种子 handler 拦截
pub effect random_bytes(output: MutSlice<u8>) -> !RngErr void;
pub effect random_u64() -> !RngErr u64;
```

- **负例**：函数签名要求密码学随机密钥生成 `fn gen_session_key(rng: *R) where R :- CryptoRng` 时，传入仅实现 `RngCore`（未实现 `CryptoRng`）的 Tier 3 `pcg_rng.Pcg64` 必须在编译期直接拒绝，不得因为同有 `next_u64` 而允许降级。

---

### 3.5 包 `(5)`：`async_runtime`（可插拔异步执行器、Reactor 与异步 I/O 协议）

**职责定位**：在 Tier 1（`08` 中 `builtin.Continuation`、`std.runtime.StackPool` 与 `async effect` 挂起/取消 ABI）之上，定义 `io_uring_rt`、`epoll_rt`、嵌入式裸机调度器及 `silver` UI 主循环均可互操作的标准协议层。**注意**：Flurry 语法中不存在 `async fn` 关键字（见 `docs/grammar/syntax`），所有异步挂起能力均通过 **`async effect`** 与非阻塞 `PollState` 组合表达，并由 `.use(runtime)` 传入执行器与底层栈池。

```flurry
use builtin.{bool, Type, u64, u8, usize, void}
use std.collections.{MutSlice, Slice}
use std.io.IoErr;
use std.time.{Duration, Instant}

pub enum AsyncRuntimeErr {
    ExecutorShutdown,
    TaskCancelled,
    TimerElapsed,
    QueueFull,
    SpawnFailed,
    Timeout,
    IoFailed(IoErr),
}

pub newtype TaskId = u64;
pub newtype RawIoHandle = usize;

pub enum Interest {
    readable,
    writable,
    both,
}

pub enum PollState
where
    T,
{
    ready(T),
    pending,
}

pub trait WakerHandle {
    fn wake(*self);
    fn wake_by_ref(*self);
}

pub trait Executor {
    fn block_on(*self, entry: %c fn() -> R) -> !AsyncRuntimeErr R
    where
        R,
        c: Type.vars.Closure;

    -- 协作式单步推进（专供 Silver UI 帧循环与单核嵌入式事件循环复用）
    fn tick(*self) -> usize;
    fn is_idle(*self) -> bool;
}

pub trait TaskSpawner {
    assoc JoinHandle: Type;

    fn spawn(*self, task: %c fn() -> R) -> !AsyncRuntimeErr JoinHandle
    where
        R,
        c: Type.vars.Closure;
}

pub trait Reactor {
    assoc SourceHandle: Type;

    fn register(*self, handle: RawIoHandle, interest: Interest, waker: WakerHandle) -> !AsyncRuntimeErr SourceHandle;
    fn reregister(*self, source: *SourceHandle, interest: Interest) -> !AsyncRuntimeErr void;
    fn deregister(*self, source: SourceHandle) -> !AsyncRuntimeErr void;
    fn poll_events(*self, timeout: ?Duration) -> !AsyncRuntimeErr usize;
}

pub trait Timer {
    fn sleep_for(*self, duration: Duration) -> !AsyncRuntimeErr void;
    fn sleep_until(*self, deadline: Instant) -> !AsyncRuntimeErr void;
}

-- 非阻塞 Poll 接口（供定制状态机或 Reactor 组合）
pub trait AsyncReader {
    fn poll_read(*self, buf: MutSlice<u8>, waker: *WakerHandle) -> !AsyncRuntimeErr PollState<usize>;
}

pub trait AsyncWriter {
    fn poll_write(*self, buf: Slice<u8>, waker: *WakerHandle) -> !AsyncRuntimeErr PollState<usize>;
    fn poll_flush(*self, waker: *WakerHandle) -> !AsyncRuntimeErr PollState<void>;
    fn poll_close(*self, waker: *WakerHandle) -> !AsyncRuntimeErr PollState<void>;
}

-- 语言级 async effect 协议组：调用方以 `pending = io_read(...)#; pending.await` 请求，由 `.use(rt)` 调度器提供栈池与恢复
pub effect enum AsyncRuntimeOps {
    async yield_now(),
    async sleep(duration: Duration) -> !AsyncRuntimeErr void,
    async io_wait(handle: RawIoHandle, interest: Interest) -> !AsyncRuntimeErr void,
}

-- 基于 async effect 的高级异步读写入口（无需 async fn 关键字，由普通 fn 声明效应行）
pub fn read_async(reader: *R, buf: MutSlice<u8>, handle: RawIoHandle, waker: *WakerHandle)
    -> #AsyncRuntimeOps.io_wait !AsyncRuntimeErr usize
where
    R :- AsyncReader,
{
    while true {
        let state = reader.poll_read(buf, waker)!;
        state match {
            .ready(n) => return n,
            .pending => {
                let pending_wait = AsyncRuntimeOps.io_wait(handle, Interest.readable)#;
                pending_wait.await!;
            },
        }
    }
    0usize
}
```

- **互操作与负例**：`silver` UI 运行时只需实现 `Executor`（提供 `tick` 在 VSync 帧内推进任务）及封装 `std.runtime.BoundedStackPool`，即可直接承载 `.use(silver_rt)` 运行任意依赖 `#AsyncRuntimeOps` 的网络/文件协程。**负例**：写出 `pub async fn read_async(...)`（非法关键字）或在 Tier 1 `std.runtime` 内直接依赖 `io_uring_rt` 具体结构体均被拒绝。

---

### 3.6 包 `(6)`：`wire` 与 `http`（网络帧编解码与 HTTP 协议族）

**职责定位**：`wire` 定义通用网络分帧与 URI 模型；`http` 定义版本无关的 HTTP 请求/响应/头/状态码/流式体与服务/中间件契约。**Zero HTTP parser、zero HPACK/QPACK、zero TLS**。

#### 3.6.1 `wire` 包完整签名

```flurry
use builtin.{bool, str, Symbol, Type, u16, u8, usize, void}
use builtin.fmt.ByteWriter;
use std.collections.{MutSlice, Slice, Vec}
use std.io.IoErr;
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.string.String;

pub enum WireErr {
    IncompleteFrame,
    FrameTooLarge,
    MalformedFrame,
    InvalidUri,
    ProtocolViolation,
    IoFailed(IoErr),
    AllocFailed,
}

pub struct UriView {
    scheme: ?str,
    authority: ?str,
    path: str,
    query: ?str,
    fragment: ?str,
}

pub struct OwnedUri
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    raw: String<.A = A, .allocator = allocator>,
    scheme_len: usize,
    authority_start: usize,
    authority_len: usize,
    path_start: usize,
    path_len: usize,
    query_start: usize,
    query_len: usize,
    fragment_start: usize,
    fragment_len: usize,
}

pub trait Codec
where
    proto: Symbol,
{
    assoc Item: Type;
    assoc FrameErr: Type;

    -- 返回 null 表示缓冲区数据尚不足一个完整帧；否则返回 (解码对象, 消费字节数)
    fn decode(*self, src: MutSlice<u8>) -> ![FrameErr, WireErr] ?Tuple<Item, usize>;

    fn encode(*self, item: *Item, dst: *W) -> ![FrameErr, WireErr, W.as(ByteWriter).Err] void
    where
        W :- ByteWriter;
}
```

#### 3.6.2 `http` 包完整签名

```flurry
use async_runtime.{PollState, WakerHandle}
use builtin.{bool, str, Type, u16, u64, u8, usize, void}
use std.collections.{Slice, Vec}
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.string.String;

pub enum HttpErr {
    InvalidMethod,
    InvalidStatus,
    InvalidHeaderName,
    InvalidHeaderValue,
    BodyTooLarge,
    ConnectionClosed,
    Timeout,
    CodecFailed,
    AllocFailed,
}

pub enum Method {
    get,
    head,
    post,
    put,
    delete,
    connect,
    options,
    trace,
    patch,
    custom(str),
}

pub enum Version {
    http_1_0,
    http_1_1,
    http_2,
    http_3,
}

pub newtype StatusCode = u16;

impl StatusCode {
    pub const OK: StatusCode = 200u16.as(StatusCode);
    pub const CREATED: StatusCode = 201u16.as(StatusCode);
    pub const BAD_REQUEST: StatusCode = 400u16.as(StatusCode);
    pub const UNAUTHORIZED: StatusCode = 401u16.as(StatusCode);
    pub const NOT_FOUND: StatusCode = 404u16.as(StatusCode);
    pub const INTERNAL_SERVER_ERROR: StatusCode = 500u16.as(StatusCode);

    pub pure fn is_success(self) -> bool {
        let code = self.as(u16);
        code >= 200u16 and code < 300u16
    }

    pub pure fn is_client_error(self) -> bool {
        let code = self.as(u16);
        code >= 400u16 and code < 500u16
    }
}

pub struct HeaderEntry
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    name: String<.A = A, .allocator = allocator>,
    value: String<.A = A, .allocator = allocator>,
}

pub struct Headers
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    entries: Vec<HeaderEntry<.A = A, .allocator = allocator>, .A = A, .allocator = allocator>,
}

pub trait HttpBody {
    assoc Data: Type;

    fn is_end_stream(*self) -> bool;
    fn size_hint(*self) -> ?u64;
    fn next_chunk(*self, waker: *WakerHandle) -> !HttpErr PollState<?Data>;
}

pub struct HttpRequest
where
    B,
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    method: Method,
    uri: wire.OwnedUri<.A = A, .allocator = allocator>,
    version: Version,
    headers: Headers<.A = A, .allocator = allocator>,
    body: B,
}

pub struct HttpResponse
where
    B,
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    status: StatusCode,
    version: Version,
    headers: Headers<.A = A, .allocator = allocator>,
    body: B,
}

pub trait HttpService {
    assoc ReqBody: Type;
    assoc ResBody: Type;
    assoc Err: Type;

    fn call(
        *self,
        req: HttpRequest<ReqBody, .A = A, .allocator = allocator>,
    ) -> !Err HttpResponse<ResBody, .A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

pub trait Middleware {
    assoc Wrapped: Type;

    fn wrap(self, inner: S) -> Wrapped
    where
        S :- HttpService;
}
```

---

### 3.7 包 `(7)`：`database`（方言参数化数据库、参数绑定与行映射协议）

**职责定位**：统一 SQL 方言参数绑定（`ToSql<dialect>`）、值解码（`FromSql<dialect>`）、行反序列化（`FromRow<dialect>`）以及连接/事务抽象（`Connection`、`Transaction`）。**Zero Postgres/SQLite/MySQL wire code or connection pooling in Tier 2**。

```flurry
use builtin.{bool, f64, i64, str, Symbol, Type, u64, u8, usize, void}
use meta.Object;
use std.collections.{Slice, Vec}
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.string.String;

pub enum SqlErr {
    ConnectionFailed,
    QueryFailed,
    SyntaxError,
    ConstraintViolation,
    TypeMismatch,
    NullUnexpected,
    ColumnNotFound,
    TransactionAborted,
    SerializationFailure,
    ConnectionClosed,
    AllocFailed,
}

pub enum IsolationLevel {
    read_uncommitted,
    read_committed,
    repeatable_read,
    serializable,
}

pub enum SqlValueRef {
    null_val,
    bool_val(bool),
    i64_val(i64),
    f64_val(f64),
    text_val(str),
    bytes_val(Slice<u8>),
}

pub trait SqlParamSink {
    fn push_param(*self, value: SqlValueRef) -> !SqlErr void;
}

pub trait RowAccessor {
    fn column_count(*self) -> usize;
    fn column_index(*self, name: str) -> ?usize;
    fn column_value(*self, index: usize) -> !SqlErr SqlValueRef;
}

pub trait ToSql
where
    dialect: Symbol,
{
    fn to_sql(*self, sink: *S) -> !SqlErr void
    where
        S :- SqlParamSink;
}

-- 解码目标本身携带分配器环境（如 String<.A = A, .allocator = alloc>），无需额外方法级冲突子句
pub trait FromSql
where
    dialect: Symbol,
{
    fn from_sql(value: SqlValueRef) -> ![SqlErr, AllocErr] Self;
}

-- 行解码协议：对于含分配器参数的 Self（如 User<.A = A, .allocator = alloc>），字段解码直接使用 Self 携带的 (A, alloc)
pub trait FromRow
where
    dialect: Symbol,
{
    fn from_row(row: *R) -> ![SqlErr, AllocErr] Self
    where
        R :- RowAccessor;
}

pub trait Transaction {
    fn commit(self) -> !SqlErr void;
    fn rollback(self) -> !SqlErr void;
}

pub trait Connection
where
    dialect: Symbol,
{
    assoc Tx: Type;
    assoc RowIter: Type;

    fn execute(*self, sql: str, params: Slice<dyn ToSql<dialect>>) -> !SqlErr u64;
    fn query(*self, sql: str, params: Slice<dyn ToSql<dialect>>) -> !SqlErr RowIter;
    fn begin_tx(*self, isolation: IsolationLevel) -> !SqlErr Tx;
}

pub mod attrs {
    use builtin.str;
    use meta.Object;

    pub pure comptime fn column(name: str) -> Object {
        { database_column: name }
    }

    pub pure comptime fn table(name: str) -> Object {
        { database_table: name }
    }

    pub const primary_key: Object = { database_primary_key: true }
    pub const skip: Object = { database_skip: true }
}
```

#### 3.7.1 完整工作示例：`derive FromRow<.postgres>.use(pg_lib.by_column_name) for User`

```flurry
-- Tier 3 包 pg_lib 导出派生策略（只使用普通领域键名 database_column）
mod pg_lib {
    use builtin.{str, Symbol, Type}
    use database.{FromRow, FromSql, RowAccessor, SqlErr}
    use meta.List;
    use meta.reflection;
    use meta.reflection.{AttributeArg, AttributeInfo}
    use std.mem.{Allocator, DefaultAllocator, MaybeUninit}

    pure comptime fn get_attr_str(attrs: List<AttributeInfo>, key: Symbol) -> ?str {
        inline for item in attrs {
            item.named_args.get(key) match {
                arg? => {
                    arg match {
                        .str_lit(text) => return text,
                        _ => {},
                    }
                },
                null => {},
            }
        }
        null
    }

    fn retire_value(value: T)
    where
        T,
    {}

    pub derive fn by_column_name for FromRow<.postgres> {
        const info = reflection.reflect_struct(Self) match {
            ok! => ok,
            error _ => meta.compilation.err("FromRow<.postgres> requires nominal struct"),
        }

        unsafe fn drop_initialized_prefix(base_addr: usize, count: usize) {
            var idx: usize = 0usize;
            inline for field in info.fields {
                if idx < count {
                    const offset = field.byte_offset match {
                        off? => off,
                        null => 0usize,
                    }
                    let addr = builtin.mem.checked_add_usize(base_addr, offset) match {
                        a? => a,
                        null => 0usize,
                    }
                    if addr != 0usize {
                        retire_value<field.FieldType()>(builtin.mem.raw_read<field.FieldType()>(addr));
                    }
                }
                idx = idx + 1usize;
            }
        }

        fn from_row(row: *R) -> ![SqlErr, std.mem.AllocErr] Self
        where
            R :- RowAccessor,
        {
            {-
                编译期展开 struct 字段映射：按列名从 row 读取并经字段各自类型（含 User 自身携带的
                .A / .allocator 环境）调用 FromSql<.postgres>.from_sql 解码。若中途失败，立即清理已写入前缀
                字段，杜绝部分初始化资源泄漏。
            -}
            var out = MaybeUninit<Self>.uninit();
            let base_addr = out.as_ptr()'bitvec.as(usize);
            var init_count: usize = 0usize;
            inline for field in info.fields {
                const col_name = get_attr_str(field.attributes, .database_column) match {
                    renamed? => renamed,
                    null => field.name match {
                        sym? => sym'tag_name,
                        null => meta.compilation.err("FromRow<.postgres> requires named fields"),
                    },
                }
                const offset = field.byte_offset match {
                    off? => off,
                    null => meta.compilation.err("bit-packed field without byte offset is unsupported"),
                }
                let col_idx = row.column_index(col_name) match {
                    idx? => idx,
                    null => {
                        unsafe { drop_initialized_prefix(base_addr, init_count); }
                        return error SqlErr.ColumnNotFound;
                    },
                }
                let raw_val = row.column_value(col_idx) match {
                    v! => v,
                    error err => {
                        unsafe { drop_initialized_prefix(base_addr, init_count); }
                        return error err;
                    },
                }
                let parsed = field.FieldType()
                    .as(FromSql<.postgres>)
                    .from_sql(raw_val) match {
                        v! => v,
                        error err => {
                            unsafe { drop_initialized_prefix(base_addr, init_count); }
                            return error err;
                        },
                    }
                let dst_addr = builtin.mem.checked_add_usize(base_addr, offset) match {
                    addr? => addr,
                    null => {
                        retire_value<field.FieldType()>(parsed);
                        unsafe { drop_initialized_prefix(base_addr, init_count); }
                        return error SqlErr.TypeMismatch;
                    },
                }
                unsafe {
                    builtin.mem.raw_write<field.FieldType()>(dst_addr, parsed);
                }
                init_count = init_count + 1usize;
            }
            unsafe { out.assume_init() }
        }
    }
}

-- 用户领域包
use database.FromRow;
use std.mem.{Allocator, DefaultAllocator}
use std.string.String;

^database.attrs.table("users")
pub struct User
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    ^database.attrs.primary_key
    id: i64,
    ^database.attrs.column("display_name")
    name: String<.A = A, .allocator = allocator>,
}

derive FromRow<.postgres>.use(pg_lib.by_column_name) for User<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A;
```

---

### 3.8 包 `(8)`：`compression` 与 `encoding`（流式压缩与非 UTF-8 / 线编码协议）

**职责定位**：将有损/无损块压缩协议（`.gzip`、`.zlib`、`.zstd`、`.lz4`、`.brotli`）归入 `compression`，将一切 UTF-8 之外的字符集转换（`.utf16_le`、`.shift_jis`、`.gb18030`、`.latin1`）与线格式编解码（`.base64_std`、`.base64_url`、`.hex_lower`、`.percent`）归入 `encoding`，彻底保持 `builtin.text` 与 `std.string` 的纯粹 UTF-8 语义。

```flurry
-- 包：compression
use builtin.{str, Symbol, Tuple, Type, u8, usize, void}
use std.collections.{MutSlice, Slice, Vec}
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum CompressErr {
    BufferTooSmall,
    CorruptStream,
    UnsupportedCodec,
    InvalidDictionary,
    UnexpectedEof,
    AllocFailed,
}

pub enum FlushMode {
    none,
    sync,
    finish,
}

pub enum CompressStatus {
    needs_input,
    has_more_output,
    finished,
}

pub trait Compress
where
    codec: Symbol,
{
    -- 返回 Tuple<已消费输入字节数, 已写入输出字节数, 流状态>
    fn compress_chunk(
        *self,
        input: Slice<u8>,
        output: MutSlice<u8>,
        flush: FlushMode,
    ) -> !CompressErr Tuple<usize, usize, CompressStatus>;

    fn compress_to_vec(
        *self,
        input: Slice<u8>,
    ) -> ![CompressErr, AllocErr] Vec<u8, .A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

    fn reset(*self);
}

pub trait Decompress
where
    codec: Symbol,
{
    fn decompress_chunk(
        *self,
        input: Slice<u8>,
        output: MutSlice<u8>,
    ) -> !CompressErr Tuple<usize, usize, CompressStatus>;

    fn decompress_to_vec(
        *self,
        input: Slice<u8>,
    ) -> ![CompressErr, AllocErr] Vec<u8, .A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

    fn reset(*self);
}

pub pure comptime trait ComptimeCompress
where
    codec: Symbol,
{
    comptime fn compress_chunk(self, input: meta.List<u8>) -> !CompressErr meta.List<u8>;
}

pub pure comptime trait ComptimeDecompress
where
    codec: Symbol,
{
    comptime fn decompress_chunk(self, input: meta.List<u8>) -> !CompressErr meta.List<u8>;
}
```

```flurry
-- 包：encoding
use builtin.{str, Symbol, Type, u8, usize, void}
use builtin.fmt.ByteWriter;
use meta.List;
use std.collections.Slice;

pub enum CodecErr {
    MalformedInput,
    UnmappableCharacter,
    InvalidPadding,
    OutputTooSmall,
    UnsupportedCharset,
    AllocFailed,
}

pub trait TextCodec
where
    charset: Symbol,
{
    fn decode_to_utf8(*self, input: Slice<u8>, out: *W) -> ![CodecErr, W.as(ByteWriter).Err] usize
    where
        W :- ByteWriter;

    fn encode_from_utf8(*self, input: str, out: *W) -> ![CodecErr, W.as(ByteWriter).Err] usize
    where
        W :- ByteWriter;

    fn decoded_len_upper_bound(*self, input_len: usize) -> usize;
}

pub trait WireEncode
where
    scheme: Symbol,
{
    fn encode_wire(*self, input: Slice<u8>, out: *W) -> ![CodecErr, W.as(ByteWriter).Err] void
    where
        W :- ByteWriter;

    fn decode_wire(*self, input: str, out: *W) -> ![CodecErr, W.as(ByteWriter).Err] void
    where
        W :- ByteWriter;
}

pub pure comptime trait ComptimeTextCodec
where
    charset: Symbol,
{
    comptime fn decode_to_utf8(self, input: List<u8>) -> !CodecErr str;
    comptime fn encode_from_utf8(self, input: str) -> !CodecErr List<u8>;
}

pub pure comptime trait ComptimeWireEncode
where
    scheme: Symbol,
{
    comptime fn encode_wire(self, input: List<u8>) -> !CodecErr str;
    comptime fn decode_wire(self, input: str) -> !CodecErr List<u8>;
}
```

---

### 3.9 包 `(9)`：`validation` 与 `schema`（领域输入校验与结构模式描述协议）

**职责定位**：提供声明式输入校验（`Validate<profile: Symbol>`）、多错误汇总容器 `ValidationReport` 及外部规范导出（`SchemaDescribe<spec: Symbol>`，如 `.json_schema_2020`、`.openapi_3_1`、`.protobuf_v3`）。严格区别于编译期类型系统 PTC（`docs/type/propositional-constraints/`）与程序形式验证（`verification`）：本包只服务**运行时业务数据合法性校验与文档契约导出**。

```flurry
-- 包：validation
use builtin.{bool, i64, str, Symbol, Type, usize, void}
use std.collections.Vec;
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.string.String;

pub enum ValidationErr {
    RuleFailed,
    MissingRequired,
    OutOfRange,
    InvalidLength,
    PatternMismatch,
    InvalidFormat,
    CustomRuleFailed,
    AllocFailed,
}

pub struct ValidationIssue
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    field_path: String<.A = A, .allocator = allocator>,
    code: str,
    message: str,
}

pub struct ValidationReport
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    issues: Vec<ValidationIssue<.A = A, .allocator = allocator>, .A = A, .allocator = allocator>,
}

impl ValidationReport<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A,
{
    pub fn new() -> Self {
        ValidationReport<.A = A, .allocator = alloc> {
            issues: Vec<ValidationIssue<.A = A, .allocator = alloc>, .A = A, .allocator = alloc>.new(),
        }
    }

    pub fn is_valid(*self) -> bool {
        self.issues.is_empty()
    }

    pub fn push_issue(*self, field_path: str, code: str, message: str) -> !AllocErr void {
        var path_buf = String<.A = A, .allocator = alloc>.from_str(field_path)!;
        self.issues.push(ValidationIssue<.A = A, .allocator = alloc> {
            field_path: path_buf,
            code: code,
            message: message,
        })
    }
}

pub trait Validate
where
    profile: Symbol,
{
    fn validate(*self) -> !ValidationErr void;

    fn validate_into(*self, report: *ValidationReport<.A = A, .allocator = allocator>) -> !AllocErr bool
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

pub pure comptime trait ComptimeValidate
where
    profile: Symbol,
{
    comptime fn validate(self) -> !ValidationErr void;
}

pub mod attrs {
    use builtin.{i64, str, usize}
    use meta.Object;

    pub pure comptime fn min_len(limit: usize) -> Object {
        { validation_min_len: limit }
    }

    pub pure comptime fn max_len(limit: usize) -> Object {
        { validation_max_len: limit }
    }

    pub pure comptime fn range_i64(low: i64, high: i64) -> Object {
        { validation_range_i64: { min: low, max: high } }
    }

    pub const non_empty: Object = { validation_non_empty: true }

    pub pure comptime fn custom_rule(rule_name: str) -> Object {
        { validation_custom_rule: rule_name }
    }
}
```

```flurry
-- 包：schema
use builtin.{bool, str, Symbol, Type, usize, void}
use std.collections.Vec;
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.string.String;

pub enum SchemaSpecErr {
    UnsupportedType,
    RecursiveCycle,
    InvalidConstraint,
    AllocFailed,
}

pub enum SchemaPrimitiveKind {
    boolean_kind,
    integer_kind,
    float_kind,
    string_kind,
    bytes_kind,
    object_kind,
    array_kind,
    enum_kind,
}

pub struct SchemaProperty
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    name: String<.A = A, .allocator = allocator>,
    required: bool,
    type_ref: String<.A = A, .allocator = allocator>,
}

pub struct SchemaDoc
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    title: String<.A = A, .allocator = allocator>,
    kind: SchemaPrimitiveKind,
    properties: Vec<SchemaProperty<.A = A, .allocator = allocator>, .A = A, .allocator = allocator>,
}

pub trait SchemaDescribe
where
    spec: Symbol,
{
    fn describe_schema() -> ![SchemaSpecErr, AllocErr] SchemaDoc<.A = A, .allocator = allocator>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

-- 单相编译期 Schema 轻量摘要协议（非 runtime SchemaDescribe 的双相孪生）
pub pure comptime trait ComptimeSchemaSummary
where
    spec: Symbol,
{
    comptime fn schema_title(self) -> !SchemaSpecErr str;
    comptime fn schema_kind(self) -> !SchemaSpecErr SchemaPrimitiveKind;
}
```

#### 3.9.1 派生工作示例与负例

```flurry
use std.mem.{Allocator, DefaultAllocator}
use std.string.String;
use validation.Validate;

pub struct SignupForm
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    ^validation.attrs.non_empty
    ^validation.attrs.min_len(3usize)
    ^validation.attrs.max_len(64usize)
    username: String<.A = A, .allocator = allocator>,

    ^validation.attrs.range_i64(13i64, 150i64)
    age: i64,
}

derive Validate<.strict>.use(validator_lib.by_field_rules) for SignupForm<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A;

-- 负例（拒绝）：不得用 `Validate` 的运行时业务检查结果替代 `requires` 前置条件或 PTC 类型等式
{-
fn bad_trusted_age(form: *SignupForm) -> bool
where
    requires form.age.* >= 13i64, -- 编译错误：Validate<.strict> 是普通运行时方法，不向 PTC 注入静态不变式
{
    form.validate<.strict>()!;
    true
}
-}
```

---

### 3.10 包 `(10)`：`cli` 与 `config`（命令行解析协议与配置读取效应）

**职责定位**：基于 `std.env.args` 提供样式参数化的命令行反序列化（`FromArgs<style: Symbol>`，如 `.gnu`、`.posix`、`.subcommand`）与分层配置读取协议/代数效应（`ConfigLoad<format: Symbol>`、`effect config_get`）。**Tier 2 不内置 getopt/clap 状态机或 TOML/INI 解析器**。

```flurry
-- 包：cli
use builtin.{bool, char, str, Symbol, Type, usize, void}
use std.collections.Slice;
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum CliErr {
    MissingArgument,
    UnknownFlag,
    InvalidValue,
    DuplicateFlag,
    MissingSubcommand,
    HelpRequested,
    VersionRequested,
    AllocFailed,
}

pub struct ArgDescriptor {
    long_name: ?str,
    short_name: ?char,
    positional_index: ?usize,
    takes_value: bool,
    required: bool,
    help_text: str,
}

pub trait FromArgs
where
    style: Symbol,
{
    fn arg_descriptors() -> Slice<ArgDescriptor>;

    fn from_args(args: Slice<str>) -> ![CliErr, AllocErr] Self
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

pub mod attrs {
    use builtin.{bool, char, str, usize}
    use meta.Object;

    pub pure comptime fn short(flag: char) -> Object {
        { cli_short: flag }
    }

    pub pure comptime fn long(flag: str) -> Object {
        { cli_long: flag }
    }

    pub pure comptime fn positional(index: usize) -> Object {
        { cli_positional: index }
    }

    pub pure comptime fn env_fallback(var_name: str) -> Object {
        { cli_env_fallback: var_name }
    }

    pub pure comptime fn help(text: str) -> Object {
        { cli_help: text }
    }

    pub const subcommand: Object = { cli_subcommand: true }
}
```

```flurry
-- 包：config
use builtin.{str, Symbol, Type, u8}
use std.collections.Slice;
use std.io.IoErr;
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum ConfigErr {
    KeyNotFound,
    ParseFailed,
    TypeMismatch,
    SourceUnavailable,
    IoFailed(IoErr),
    AllocFailed,
}

pub trait ConfigLoad
where
    format: Symbol,
{
    fn load_config(bytes: Slice<u8>) -> ![ConfigErr, AllocErr] Self
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

-- 薄配置读取效应：调用方无需直接耦合环境变量或特定文件格式，测试中可纯内存提供
pub effect config_get(key: str) -> ?str;
pub effect config_require(key: str) -> !ConfigErr str;
```

#### 3.10.1 工作示例：`derive FromArgs<.gnu>` 与 `config_get` 效应组合

```flurry
use builtin.{bool, str, Type}
use cli.FromArgs;
use config.{ConfigErr, config_get}
use std.collections.Slice;
use std.mem.{AllocErr, Allocator, DefaultAllocator}
use std.string.String;

pub struct AppOptions
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    ^cli.attrs.short('v')
    ^cli.attrs.long("verbose")
    verbose: bool,

    ^cli.attrs.long("endpoint")
    ^cli.attrs.env_fallback("APP_ENDPOINT")
    endpoint: String<.A = A, .allocator = allocator>,
}

derive FromArgs<.gnu>.use(cli_lib.derive_parser) for AppOptions<.A = A, .allocator = alloc>
where
    A: Type,
    requires A :- Allocator,
    quote alloc: lift *A;

fn resolve_endpoint(args: Slice<str>)
    -> #config_get ![cli.CliErr, AllocErr] String<.A = A, .allocator = allocator>
where
    .A: Type = DefaultAllocator,
    requires A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    let opts = AppOptions<.A = A, .allocator = allocator>
        .as(FromArgs<.gnu>)
        .from_args<.A = A, .allocator = allocator>(args)!;
    config_get("APP_ENDPOINT_OVERRIDE")# match {
        override_url? => String<.A = A, .allocator = allocator>.from_str(override_url)!,
        null => opts.endpoint,
    }
}
```

---

### 3.11 包 `(11)`：`datetime`（历法、时区与日期格式化协议）

**职责定位**：严格与 Tier 1 `std.time`（仅提供单调 `Instant`、纳秒 `Duration` 与原始时间戳 `SystemTime`）解耦。公历/ISO-8601/农历等历法转换、IANA `tzdata` 时区偏移计算、RFC3339 文本格式化及可拦截系统时钟效应全部置于 `datetime`。

```flurry
use builtin.{bool, i128, i32, i64, str, Symbol, Tuple, u32, u8, void}
use builtin.fmt.ByteWriter;
use std.time.SystemTime;

pub enum DateTimeErr {
    InvalidDate,
    InvalidTime,
    AmbiguousLocalTime,
    NonExistentLocalTime,
    UnknownTimeZone,
    ParseFailed,
    FormatFailed,
    Overflow,
}

pub enum Weekday {
    monday,
    tuesday,
    wednesday,
    thursday,
    friday,
    saturday,
    sunday,
}

pub struct CivilDate {
    year: i32,
    month: u8,
    day: u8,
}

pub struct CivilTime {
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

pub struct UtcOffset {
    total_seconds: i32,
}

pub trait Calendar
where
    system: Symbol,
{
    fn to_epoch_days(*self, date: CivilDate) -> !DateTimeErr i64;
    fn from_epoch_days(*self, epoch_days: i64) -> !DateTimeErr CivilDate;
    fn days_in_month(*self, year: i32, month: u8) -> !DateTimeErr u8;
    fn is_leap_year(*self, year: i32) -> bool;
    fn weekday_of(*self, date: CivilDate) -> !DateTimeErr Weekday;
}

pub trait TimeZone {
    fn zone_name(*self) -> str;
    fn offset_at_utc(*self, utc_epoch_nanos: i128) -> !DateTimeErr UtcOffset;
    fn resolve_local(*self, date: CivilDate, time: CivilTime) -> !DateTimeErr i128;
}

pub trait DateTimeFormat
where
    fmt: Symbol,
{
    fn format_timestamp(
        *self,
        utc_epoch_nanos: i128,
        offset: UtcOffset,
        out: *W,
    ) -> ![DateTimeErr, W.as(ByteWriter).Err] void
    where
        W :- ByteWriter;

    fn parse_timestamp(*self, input: str) -> !DateTimeErr Tuple<i128, UtcOffset>;
}

pub pure comptime trait ComptimeCalendar
where
    system: Symbol,
{
    comptime fn to_epoch_days(self, date: CivilDate) -> !DateTimeErr i64;
    comptime fn from_epoch_days(self, epoch_days: i64) -> !DateTimeErr CivilDate;
    comptime fn days_in_month(self, year: i32, month: u8) -> !DateTimeErr u8;
    comptime fn is_leap_year(self, year: i32) -> bool;
    comptime fn weekday_of(self, date: CivilDate) -> !DateTimeErr Weekday;
}

pub pure comptime trait ComptimeDateTimeFormat
where
    fmt: Symbol,
{
    comptime fn parse_timestamp(self, input: str) -> !DateTimeErr Tuple<i128, UtcOffset>;
}

-- 可测试时钟与时区效应：将“获取当前时间”与 std.time 原生不可控硬件读取彻底隔离
pub effect system_clock() -> SystemTime;
pub effect local_timezone() -> str;
```

- **边界反例**：禁止在 `std.time` 中添加 `Instant.to_rfc3339`、`SystemTime.year()` 或加载 `/usr/share/zoneinfo`；任何需要历法与时区的调用方必须导入 Tier 2 `datetime` 并接装 Tier 3 `tzdata_iana`。

---

### 3.12 包 `(12)`：`color` 与 `media`（色彩空间与图像/音频编解码协议）

**职责定位**：`color` 为图形/UI 库（如 Silver Layer 4）提供 `ColorSpace<space: Symbol>`（`.srgb`、`.linear_srgb`、`.oklch`、`.display_p3`、`.aces_cg`）与像素内存布局枚举；`media` 提供图像/音频编解码抽象，并支持零拷贝写入与显式 `allocator` 分配解码输出。

```flurry
-- 包：color
use builtin.{Array, bool, f32, Symbol, u8, usize}

pub enum ColorErr {
    OutOfGamut,
    UnsupportedTransfer,
    InvalidChannelCount,
}

pub enum PixelLayout {
    rgba8_unorm,
    bgra8_unorm,
    rgb8_unorm,
    rgba16_float,
    r8_unorm,
    yuv420_planar,
}

impl PixelLayout {
    pub pure fn bits_per_pixel(self) -> usize {
        self match {
            .rgba8_unorm => 32usize,
            .bgra8_unorm => 32usize,
            .rgb8_unorm => 24usize,
            .rgba16_float => 64usize,
            .r8_unorm => 8usize,
            .yuv420_planar => 12usize,
        }
    }

    pub pure fn is_planar(self) -> bool {
        self match {
            .yuv420_planar => true,
            _ => false,
        }
    }
}

pub trait ColorSpace
where
    space: Symbol,
{
    assoc CHANNELS: usize;

    fn to_linear_xyz(*self, channels: Array<f32, CHANNELS>) -> Array<f32, 3usize>;
    fn from_linear_xyz(*self, xyz: Array<f32, 3usize>) -> !ColorErr Array<f32, CHANNELS>;
}
```

```flurry
-- 包：media
use builtin.{Symbol, Tuple, Type, u16, u32, u8, usize, void}
use builtin.fmt.ByteWriter;
use color.PixelLayout;
use std.collections.{MutSlice, Slice, Vec}
use std.io.IoErr;
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum MediaErr {
    InvalidHeader,
    UnsupportedFormat,
    CorruptBitstream,
    DimensionOverflow,
    BufferTooSmall,
    IoFailed(IoErr),
    AllocFailed,
}

pub struct ImageDescriptor {
    width: u32,
    height: u32,
    layout: PixelLayout,
    stride_bytes: usize,
}

pub enum AudioSampleFormat {
    u8_unsigned,
    i16_le,
    i24_le,
    i32_le,
    f32_le,
}

pub struct AudioDescriptor {
    sample_rate_hz: u32,
    channels: u16,
    sample_format: AudioSampleFormat,
}

pub trait ImageEncode
where
    fmt: Symbol,
{
    fn encode_image(
        *self,
        desc: ImageDescriptor,
        pixels: Slice<u8>,
        out: *W,
    ) -> ![MediaErr, W.as(ByteWriter).Err] void
    where
        W :- ByteWriter;
}

pub trait ImageDecode
where
    fmt: Symbol,
{
    fn probe_header(*self, input: Slice<u8>) -> !MediaErr ImageDescriptor;

    fn decode_into(*self, input: Slice<u8>, dst: MutSlice<u8>) -> !MediaErr ImageDescriptor;

    fn decode_vec(
        *self,
        input: Slice<u8>,
    ) -> ![MediaErr, AllocErr] Tuple<ImageDescriptor, Vec<u8, .A = A, .allocator = allocator>>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

pub trait AudioCodec
where
    fmt: Symbol,
{
    -- 返回 Tuple<已消费编码字节数, 已写入 PCM 字节数>
    fn decode_frames(*self, input: Slice<u8>, pcm_out: MutSlice<u8>) -> !MediaErr Tuple<usize, usize>;

    fn encode_frames(
        *self,
        desc: AudioDescriptor,
        pcm_in: Slice<u8>,
        out: *W,
    ) -> ![MediaErr, W.as(ByteWriter).Err] usize
    where
        W :- ByteWriter;
}
```

---

### 3.13 包 `(13)`：`numerics` 与 `tensor_protocol`（代数结构、张量后端与 GPU 投影协议）

**职责定位**：将抽象代数层级（群/环/域/向量空间/内积空间）放入 `numerics`，将异构张量存储、内核算子派发、自动微分与 GPU 管线投影协议（衔接 [docs/pipeline-projection/](../../docs/pipeline-projection/index.md)）放入 `tensor_protocol`。**Tier 1 `builtin.math` 保持精简为 LLVM 标量/SIMD intrinsic 包装；Tier 2 绝不实现 BLAS/cuBLAS/CUDA/WGPU 具体驱动**。

```flurry
-- 包：numerics
use builtin.Type;

pub enum NumericErr {
    DivisionByZero,
    DomainError,
    SingularMatrix,
    DimensionMismatch,
}

pub trait Semigroup {
    fn op_combine(*self, rhs: *Self) -> Self;
}

pub trait Monoid :- Semigroup {
    fn identity() -> Self;
}

pub trait Group :- Monoid {
    fn inverse(*self) -> Self;
}

pub trait Ring {
    fn zero() -> Self;
    fn one() -> Self;
    fn ring_add(*self, rhs: *Self) -> Self;
    fn ring_neg(*self) -> Self;
    fn ring_mul(*self, rhs: *Self) -> Self;
}

pub trait CommutativeRing :- Ring {}

pub trait Field :- CommutativeRing {
    fn try_recip(*self) -> !NumericErr Self;

    fn field_div(*self, rhs: *Self) -> !NumericErr Self {
        let inv = rhs.try_recip()!;
        self.ring_mul(inv.ref)
    }
}

pub trait VectorSpace
where
    Scalar :- Field,
{
    fn vadd(*self, rhs: *Self) -> Self;
    fn vscale(*self, scalar: *Scalar) -> Self;
}

pub trait InnerProductSpace
where
    Scalar :- Field,
{
    fn dot(*self, rhs: *Self) -> Scalar;
}

pub trait NormedSpace
where
    Scalar :- Field,
{
    fn norm(*self) -> Scalar;
}

pub pure comptime trait ComptimeRing {
    comptime fn zero() -> Self;
    comptime fn one() -> Self;
    comptime fn ring_add(self, rhs: Self) -> Self;
    comptime fn ring_neg(self) -> Self;
    comptime fn ring_mul(self, rhs: Self) -> Self;
}

pub pure comptime trait ComptimeField :- ComptimeRing {
    comptime fn try_recip(self) -> !NumericErr Self;

    comptime fn field_div(self, rhs: Self) -> !NumericErr Self {
        let inv = rhs.try_recip()!;
        self.ring_mul(inv)
    }
}
```

```flurry
-- 包：tensor_protocol
use builtin.{Symbol, Tuple, Type, u8, usize, void}
use meta.compilation;
use std.collections.{MutSlice, Slice}
use std.mem.{AllocErr, Layout}

pub enum TensorErr {
    ShapeMismatch,
    BroadcastFailed,
    DeviceMismatch,
    OutOfDeviceMemory,
    UnsupportedDtype,
    KernelLaunchFailed,
    AllocFailed,
}

pub trait TensorStorage
where
    device: Symbol,
{
    assoc Buffer: Type;
    assoc DType: Type;

    fn allocate_storage(*self, elem_count: usize, elem_layout: Layout) -> !TensorErr Buffer;
    fn copy_from_host(*self, dst: *Buffer, src: Slice<u8>) -> !TensorErr void;
    fn copy_to_host(*self, src: *Buffer, dst: MutSlice<u8>) -> !TensorErr void;
    fn shape_dims(*self, buf: *Buffer) -> Slice<usize>;
}

pub trait TensorOp
where
    backend: Symbol,
{
    assoc Storage: Type;

    fn matmul(
        *self,
        a: *Storage,
        b: *Storage,
        out: *Storage,
        m: usize,
        n: usize,
        k: usize,
    ) -> !TensorErr void;

    fn elementwise_add(*self, a: *Storage, b: *Storage, out: *Storage, len: usize) -> !TensorErr void;

    fn broadcast_add(
        *self,
        a: *Storage,
        a_shape: Slice<usize>,
        b: *Storage,
        b_shape: Slice<usize>,
        out: *Storage,
        out_shape: Slice<usize>,
    ) -> !TensorErr void;
}

pub trait Differentiable
where
    mode: Symbol,
{
    assoc Tangent: Type;
    assoc Cotangent: Type;

    fn jvp(*self, tangent: Tangent) -> Tuple<Self, Tangent>;
    fn vjp(*self, cotangent: Cotangent) -> Self;
}

-- 对接 ^attrs.compilation.pipeline 与 meta.compilation.IIR.Func 的 GPU 编译与启动抽象
pub trait GpuProjection
where
    target: Symbol,
{
    assoc KernelHandle: Type;
    assoc LaunchConfig: Type;

    fn compile_entry(comptime ir: compilation.IIR.Func) -> !TensorErr KernelHandle;
    unsafe fn dispatch(*self, kernel: *KernelHandle, config: LaunchConfig) -> !TensorErr void;
}
```

- **负例**：禁止将 `cuda`、`cuda_compile` 或具体的多维动态 `Tensor` 结构放入 Tier 1（`builtin`/`std`/`09`）；`09` 中的静态形状张量仅验证编译期 `broadcastable`/`broadcast_shape` 阶段规则与通用投影机制。

---

### 3.14 包 `(14)`：`testing`（属性测试、收缩、基准测量与测试报告协议）

**职责定位**：配合语言内建的 `test {}` 与 `template test id clauses block`（见 [docs/grammar/syntax](../../docs/grammar/syntax)），提供基于 `random.RngCore` 的属性生成（`Arbitrary<strategy>`）、基于 `builtin.iterator.emit_item` 推送式迭代的最小反例收缩（`Shrink`）、微基准接口（`Bencher`）和结构化结果报告（`TestReporter`）。

```flurry
use builtin.{bool, str, Symbol, Type, u64, usize, void}
use builtin.iterator.emit_item;
use random.RngCore;
use std.time.Duration;

pub enum TestErr {
    AssertionFailed,
    ShrunkCounterexample,
    Timeout,
    SetupFailed,
    ReporterFailed,
    RngFailure,
}

pub enum TestOutcome {
    passed,
    failed,
    ignored,
    panicked,
}

pub struct TestCaseInfo {
    name: str,
    module_path: str,
    is_ignored: bool,
}

pub trait Arbitrary
where
    strategy: Symbol,
{
    fn arbitrary(rng: *R, size_hint: usize) -> !TestErr Self
    where
        R :- RngCore;
}

pub trait Shrink {
    inline fn shrink(*self) -> #emit_item<Self> void;
}

pub trait Bencher {
    fn iter_bench(*self, body: %c fn())
    where
        c: Type.vars.Closure;

    fn black_box(value: T) -> T
    where
        T;

    fn report_bytes(*self, bytes_per_iter: u64);
}

pub trait TestReporter {
    fn on_suite_start(*self, total_cases: usize);
    fn on_case_result(*self, info: TestCaseInfo, outcome: TestOutcome, elapsed: Duration);
    fn on_suite_finish(*self, passed_count: usize, failed_count: usize, ignored_count: usize);
}

-- 与核心语法 `template test` 集成示例（依 `docs/grammar/syntax`：`template test id clauses block`）
pub template test property_case
where
    T :- Arbitrary<.uniform> + Shrink,
    body: < sample: T |- bool >,
    .iterations: usize = 100usize,
{
    var i: usize = 0usize;
    while i < iterations {
        -- 由外部驱动或测试执行器注入 RngCore 实例生成样本并检验 ~[sample = val]<body>
        i = i + 1usize;
    }
}

test roundtrip_identity use property_case<u32, .iterations = 200usize> {
    sample + 0u32 == sample
}
```

---

### 3.15 包 `(15)`：`ffi_bridge` 与 `plugin_abi`（跨边界编组与稳定 ABI 握手协议）

**职责定位**：在 Tier 1 `std.ffi`（`c_*` 标量、`CStr`/`CString`、`^attrs.abi(...)`）与 [docs/type/type_id.md](../../docs/type/type_id.md)（128 位 Merkle `Type.type_id` + `^attrs.stable_since`）基础上，提供安全的跨语言编组（`FfiMarshal<abi: Symbol>`）与动态插件/共享库 ABI 版本握手契约（`StableAbi`、`PluginHeader`）。**全程严禁使用 `extern` 关键字**。

```flurry
-- 包：ffi_bridge
use builtin.{str, Symbol, Type, usize, void}
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum FfiMarshalErr {
    NullPointer,
    InvalidUtf8,
    LayoutMismatch,
    AbiVersionMismatch,
    SymbolNotFound,
    BufferOverflow,
    AllocFailed,
}

pub trait FfiMarshal
where
    abi: Symbol,
{
    assoc ForeignRepr: Type;

    unsafe fn to_foreign(*self) -> ![FfiMarshalErr, AllocErr] ForeignRepr
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

    unsafe fn from_foreign(raw: ForeignRepr) -> ![FfiMarshalErr, AllocErr] Self
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}
```

```flurry
-- 包：plugin_abi
use builtin.{str, Type, u128, u64, usize, void}
use ffi_bridge.FfiMarshalErr;

^attrs.stable_since("1.0.0")
pub struct AbiDescriptor {
    type_id_lo: u64,
    type_id_hi: u64,
    min_version: str,
    byte_size: usize,
    byte_align: usize,
}

pub trait StableAbi {
    pure comptime fn abi_descriptor() -> AbiDescriptor;

    fn verify_remote_abi(remote: AbiDescriptor) -> !FfiMarshalErr void {
        let local = Self.as(StableAbi).abi_descriptor();
        if local.type_id_lo != remote.type_id_lo
            or local.type_id_hi != remote.type_id_hi
            or local.byte_size != remote.byte_size
            or local.byte_align != remote.byte_align
        {
            return error FfiMarshalErr.AbiVersionMismatch;
        }
    }
}

-- C ABI 插件握手头（仅使用官方 ^attrs.abi(.c) 与 ^attrs.stable_since，严禁 extern）
^attrs.stable_since("1.0.0")
pub struct PluginHeader {
    magic: u64,
    api_type_id_lo: u64,
    api_type_id_hi: u64,
}

^attrs.abi(.c)
pub unsafe fn check_plugin_header(header: *PluginHeader, expected_lo: u64, expected_hi: u64) -> bool {
    if header.magic.* != 0x464C5259504C474Eu64 {
        return false;
    }
    header.api_type_id_lo.* == expected_lo and header.api_type_id_hi.* == expected_hi
}
```

- **负例**：若插件结构体未标注 `^attrs.stable_since("1.0.0")` 或在 `"1.1.0"` 中悄然调整了字段对齐/类型，`type_id` 的四元联合稳定契约（[type_id.md](../../docs/type/type_id.md)）会自动生成新的 128 位摘要，导致 `verify_remote_abi` 返回 `FfiMarshalErr.AbiVersionMismatch`，阻止二进制内存错位解读。

---

### 3.16 包 `(16)`：`regex_protocol` 与 `parser_protocol`（正则编译协议与解析器组合子协议）

**职责定位**：本包是将 `regex` 与复杂文法解析**彻底移出 `std` 与 `builtin`**的关键桥梁（对应 [02-pattern-matching-and-views.md](02-pattern-matching-and-views.md) 中 Tier 2/3 `user_regex` 示例的通用官方协议抽象）：
- `regex_protocol` 定义编译期正则模式到 `meta.patterns.TuplePatternView` 及 `< input: str |- ?CaptureLayout(pattern) >` CMTT 代码生成的标准契约；具体 DFA/NFA 状态机（如 `regex_dfa`、`pike_nfa`）由 Tier 3 实现。
- `parser_protocol` 定义基于流式游标的 LL(1)/PEG/Pratt 解析器接口与编译期 CMTT 解析器生成对偶。

```flurry
-- 包：regex_protocol
use builtin.{bool, str, Symbol, Type, usize, void}
use builtin.iterator.emit_item;

pub enum RegexErr {
    EmptyPattern,
    SyntaxError,
    UnbalancedParen,
    InvalidQuantifier,
    CaptureOverflow,
    StateLimitExceeded,
    UnsupportedFeature,
}

pub struct RegexSpan {
    start: usize,
    end: usize,
}

pub struct RegexFlags {
    case_insensitive: bool = false,
    multi_line: bool = false,
    dot_matches_newline: bool = false,
    unicode: bool = true,
}

impl RegexFlags {
    pub const DEFAULT: RegexFlags = RegexFlags {
        case_insensitive: false,
        multi_line: false,
        dot_matches_newline: false,
        unicode: true,
    }
}

pub trait RegexMatchOps {
    fn is_match(*self, input: str) -> bool;
    fn find_first(*self, input: str) -> ?RegexSpan;
    inline fn find_all_spans(*self, input: str) -> #emit_item<RegexSpan> void;
}

-- 编译期编译模式并返回 CMTT 匹配代码（配合 meta.patterns.TuplePatternView 接入 `match` 模式匹配）
pub trait PatternCompile
where
    flavor: Symbol,
{
    assoc CompiledMatcher: Type;

    pure comptime fn is_valid_pattern(pattern: str) -> bool;

    -- 若 pattern 合法返回 Tuple<...> 捕获布局；语法非法时回退返回 void，确保 ?CaptureLayout(pattern) 始终为合法 Type
    pure comptime fn CaptureLayout(pattern: str) -> Type;

    comptime fn compile_pattern(
        pattern: str,
        .flags: RegexFlags = RegexFlags.DEFAULT,
    ) -> !RegexErr < input: str |- ?CaptureLayout(pattern) >;
}
```

```flurry
-- 包：parser_protocol
use builtin.{Any, str, Symbol, Type, usize}

pub enum ParseErr {
    UnexpectedToken,
    UnexpectedEof,
    BacktrackLimitExceeded,
    AmbiguousGrammar,
    InvalidSyntax,
    AllocFailed,
}

pub struct ParseCursor {
    input: str,
    byte_offset: usize,
}

pub trait Parser
where
    grammar: Symbol,
{
    assoc Output: Type;

    fn parse_prefix(*self, cursor: *ParseCursor) -> !ParseErr Output;
    fn parse_all(*self, input: str) -> !ParseErr Output;
}

-- 单相编译期文法编译器协议（生成 CMTT 解析代码，非 runtime Parser.parse_prefix/parse_all 的双相孪生）
pub pure comptime trait ComptimeParserCompiler
where
    grammar: Symbol,
{
    comptime fn compile_parser(spec: str) -> !ParseErr < input: str |- !ParseErr Any >;
}
```

#### 3.16.1 跨层桥接示例（Tier 1 `TuplePatternView` / `ConcatPatternView` + Tier 2 `regex_protocol` + Tier 3 `regex_dfa`）与负例

```flurry
-- Tier 3 包 regex_dfa（非 std、非 builtin）实现 Tier 2 PatternCompile<.dfa_linear> 与 Tier 1 TuplePatternView
use builtin.{str, Type, usize}
use builtin.protocols.patterns.ConcatPatternView;
use meta.patterns.{PatternInput, TuplePatternSlot, TuplePatternStart, TuplePatternView, PatternViewErr}
use regex_protocol.{PatternCompile, RegexErr, RegexFlags, RegexMatchOps, RegexSpan}

fn parse_uri_with_dfa(uri: str) -> ?str {
    -- Tier 1 ConcatPatternView 先按固定前缀 `++` 剥离 "Bearer " 等静态字面前缀；复杂正则捕获交由 Tier 3 RegexMatcher
    uri match {
        "ssh://" ++ _ => {
            -- Tier 3 类型的普通构造，编译期自动触发 `meta.patterns.TuplePatternView` + `code_match`/`code_sequence` 组装状态机
            let matcher = regex_dfa.RegexMatcher { input: uri }
            matcher match {
                ("(ssh)://([^@/?#]+)@([^:/?#]+)", protocol, user, host) => protocol,
                _ => null,
            }
        },
        _ => null,
    }
}

-- 负例（拒绝）：严禁在 std 或 builtin 中声明或重导出 regex_protocol / RegexMatcher
{-
use std.regex.RegexMatcher; -- 编译错误：std 中不得存在 regex 模块
-}
```

---

### 3.17 包 `(17)`：`collections_ext`（持久化容器、图协议与扩展 DOD 数据导向集合）

**职责定位**：收容超出 Tier 1 `std.collections` 最小系统基线的复杂领域容器协议：不可变/结构共享持久化序列与映射（RRB-Tree/HAMT）、通用有向/无向图遍历与修改协议，以及面向 ECS / 场景图的高级代际句柄与面向列（SoA）抽象协议。

```flurry
use builtin.{bool, str, Tuple, Type, usize, void}
use builtin.iterator.emit_item;
use std.collections.{MutSlice, Slice}
use std.mem.{AllocErr, Allocator, DefaultAllocator}

pub enum GraphErr {
    NodeNotFound,
    EdgeNotFound,
    CycleDetected,
    InvalidGeneration,
    IndexOutOfBounds,
    AllocFailed,
}

-- 1. 不可变/持久化序列协议（结构共享，依赖调用方传递的 allocator）
pub trait PersistentSeq {
    assoc Element: Type;

    fn len(*self) -> usize;
    fn get(*self, index: usize) -> ?*Element;

    fn push_back_persistent(*self, value: Element) -> !AllocErr Self
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

    fn update_persistent(*self, index: usize, value: Element) -> ![GraphErr, AllocErr] Self
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

-- 2. 不可变/持久化映射协议（HAMT / 持久化 B-Tree）
pub trait PersistentMap {
    assoc Key: Type;
    assoc Value: Type;

    fn len(*self) -> usize;
    fn lookup(*self, key: *Key) -> ?*Value;

    fn insert_persistent(*self, key: Key, value: Value) -> !AllocErr Self
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;

    fn remove_persistent(*self, key: *Key) -> !AllocErr Tuple<Self, ?Value>
    where
        .A: Type = DefaultAllocator,
        requires A :- Allocator,
        quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
}

-- 3. 通用图视图与可变图协议（支持 push-based emit_item 遍历而零迭代器堆分配）
pub trait GraphView {
    assoc NodeId: Type;
    assoc EdgeId: Type;
    assoc NodeWeight: Type;
    assoc EdgeWeight: Type;

    fn node_count(*self) -> usize;
    fn edge_count(*self) -> usize;
    fn node_weight(*self, node: NodeId) -> ?*NodeWeight;
    fn edge_weight(*self, edge: EdgeId) -> ?*EdgeWeight;
    inline fn neighbors(*self, node: NodeId) -> #emit_item<Tuple<EdgeId, NodeId>> void;
}

pub trait MutGraph :- GraphView {
    fn add_node(*self, weight: NodeWeight) -> !AllocErr NodeId;
    fn add_edge(*self, from: NodeId, to: NodeId, weight: EdgeWeight) -> ![GraphErr, AllocErr] EdgeId;
    fn remove_node(*self, node: NodeId) -> ?NodeWeight;
    fn remove_edge(*self, edge: EdgeId) -> ?EdgeWeight;
}

-- 4. 代际索引容器与面向列（SoA）表抽象（配合运行时依赖类型 `quote table: lift *Self`）
pub trait GenerationalArena {
    assoc Handle: Type;
    assoc Value: Type;

    fn insert(*self, value: Value) -> !AllocErr Handle;
    fn remove(*self, handle: Handle) -> ?Value;
    fn get(*self, handle: Handle) -> ?*Value;
    fn contains(*self, handle: Handle) -> bool;
}

pub trait SoaTable {
    assoc RowRecord: Type;

    fn row_count(*self) -> usize;
    fn push_row(*self, row: RowRecord) -> !AllocErr usize;
    fn swap_remove_row(*self, index: usize) -> !GraphErr void;
}
```

#### 3.17.1 `std.collections`（Tier 1）vs `collections_ext`（Tier 2）边界对照表

| 容器 / 协议 | 当前 `07` 中的状态 | 归属边界定位 | 判定理由 |
|---|---|---|---|
| `Vec`、`VecDeque`、`HashMap`、`HashSet`、`Slice`、`MutSlice` | Tier 1 `std.collections` | **确定归属 Tier 1** | 系统编程最小通用基线（`std.fmt`/`std.io`/`std.fs` 均直接依赖） |
| `BTreeMap`、`BTreeSet`、`BinaryHeap` | Tier 1 `std.collections` | **推荐保留 Tier 1（待 §5 Q3 最终确认）** | 标准有序映射与优先队列，OS/调度器/编译器基础设施普遍需要 |
| `Arena<T>` + `NodeIndex<arena.ref>` | `07` 规划于 `std.collections` | **边界项（推荐保留 `Arena` 作运行时依赖类型示范，见 §5 Q3）** | `docs/type/runtime-dependent-type.md` 与 `Silver P1-4` 核心验证载体 |
| `SmallVec<T, N>`、`RingBuffer<T, N>`、`BitSet` | `07` 已标注边界（§5 Q11） | **边界项（见 §5 Q3 两方案对比）** | 嵌入式/无堆高频容器 vs 精简 `std` 体积权衡 |
| `SlotMap<K, V>`、`SoA<T>` | `07` 已标注边界（§5 Q11） | **强烈建议上移/协议化至 Tier 2 `collections_ext` + Tier 3 ECS/DOD 库（见 §5 Q3）** | 领域特定 DOD/ECS 布局模式，避免重型反射/列类型构造器耦合进 Tier 1 `std` |
| `PersistentSeq`、`PersistentMap`、`GraphView`/`MutGraph`、`SkipList`、`RoaringBitSet`、`BPlusTree` | 未进 `07` | **确定归属 Tier 2 `collections_ext` + Tier 3** | 高级持久化/图论算法族，严禁进入 Tier 1 `std` |

---

## 4. 排期优先级与分阶段交付路线

```text
Phase A（P0 编译器支撑底座与 Tier 1 反膨胀清理）
  ├── 编译器前置：WL-10-01（参数化 trait & for<Symbol> 裁决）、WL-10-02（derive .use() 与外源类型 coherence）、
  │              WL-10-03（非 __flurry_* 领域键隔离）、WL-10-04（issues/0001 quote .allocator 透传）、
  │              WL-10-05（同名 pure comptime trait 优先级）
  ├── Tier 1 纠偏：校验 `02` 已落地的 `user_regex` 生态示例与 Tier 2 `regex_protocol` 契约衔接，并锁定 `03`/`06`/`08`/`09` 边界
  └── 最小运行时底座（08 前置）：builtin.Continuation + std.runtime.StackPool + async effect 调度 ABI
        │
        ▼
Phase B（P0/P1 核心生态协议首波落地 — 解锁应用与基础设施）
  ├── serialization (WL-10-06) + Tier 3 json_lib.without_allocation / cbor_lib 端到端验证
  ├── async_runtime (WL-10-10) + Tier 3 io_uring_rt / Silver UI tick 互操作验证
  ├── regex_protocol (WL-10-21) + Tier 3 regex_dfa（TuplePatternView + CMTT）验证
  ├── telemetry (WL-10-07)、random (WL-10-09)、testing (WL-10-19)
  └── digest + crypto (WL-10-08)、http + wire (WL-10-11)、database (WL-10-12)、ffi_bridge + plugin_abi (WL-10-20)
        │
        ▼
Phase C（P2/P3 领域完备化与系统集成）
  ├── compression + encoding (WL-10-13)、validation + schema (WL-10-14)
  ├── cli + config (WL-10-15)、datetime (WL-10-16)
  ├── numerics + tensor_protocol (WL-10-18) 对接 pipeline-projection 与 cuda/cpu_blas
  └── media + color (WL-10-17)、collections_ext (WL-10-22)（依 §5 Q3 最终划界落位）
```

### 4.1 关键验收测试轮廓（含负例验证）

1. **包边界静态扫描门禁**：对六内置包（`library/{builtin,meta,build,attrs,std,verification}`）执行禁止符号检查——出现 `regex`/`RegexMatcher`、`Serialize`/`Deserialize`、`HttpRequest`、`SqlErr`、`Digest`/`Aead`、`Calendar`/`TimeZone`、`Zstd`/`Base64` 或 `io_uring` 具体循环即失败。
2. **Coherence 与 `.as(View)` 编译失败/通过矩阵**：
   - 正例：`derive Serialize<.json>.use(json_lib.without_allocation) for Data;` 与 `slice.as(json_lib.JsonBytes).serialize_into<.json>(out)` 在无 `SystemAllocator` 的栈上 `FixedBufferAllocator` 下编译通过并零泄漏运行；
   - 负例（compile-fail）：在 `json_lib` 中写 `impl Serialize<.json> for std.collections.Slice<u8>` 或 `^attrs.derivation.default derive fn ... for Serialize<.json>` 必须被一致性与默认策略规则拒绝。
3. **非 `__flurry_*` 与双相/单相 `pure comptime trait` 门禁**：检查 Tier 2 所有属性构造器返回键均不含 `__flurry_` 前缀；检查所有 `Comptime*` `pure comptime trait` 方法在 comptime 解释器下不依赖 `*self`/`.ref`/`.*`；检查全部双相孪生协议（`ComptimeSerialize`、`ComptimeDeserialize`、`ComptimeDigest`、`ComptimeRngCore`、`ComptimeCompress`、`ComptimeDecompress`、`ComptimeTextCodec`、`ComptimeWireEncode`、`ComptimeValidate`、`ComptimeCalendar`、`ComptimeDateTimeFormat`、`ComptimeRing`、`ComptimeField`）与其 runtime `trait` 保持严格同名方法，与单相编译期摘要/生成协议（`ComptimeLogScalar`、`ComptimeSchemaSummary`、`ComptimeParserCompiler`）界限清晰。
4. **Allocator 身份隔离与 `Zeroize` / `CryptoRng` 门禁**：跨不同 `FixedBufferAllocator` 的 `Data` / `ValidationReport` / `HttpRequest` 不可混用；`Pcg64` 不可满足 `CryptoRng`；`PluginHeader` 改变字段布局后 `verify_remote_abi` 必须因 `type_id` 变化返回 `AbiVersionMismatch`。

---

## 5. 待作者裁决的语义空缺与冲突（Open Questions）

### Q1. `for<Symbol> trait<Symbol>` vs `trait Serialize where format: Symbol`（高优先）
- **精确来源依据**：[docs/grammar/syntax](../../docs/grammar/syntax#L218)（`for_type -> for<parameter*> expr`）、[docs/type/type-identity.md](../../docs/type/type-identity.md#L44)（"`for<parameter*> expr` 是纯编译时函数的简写，不能由这个简写引入另一套运行时多态交付机制"）、[docs/traits/declarations.md](../../docs/traits/declarations.md#L12)（`trait_def -> (pure comptime)? trait id (:- expr)? clauses? { ... }`）、[docs/traits/protocols.md](../../docs/traits/protocols.md#L29)（`student.serialize<.json>()`）、[proposal_trace/2026-8-13-derivation.fl](../../proposal_trace/2026-8-13-derivation.fl#L8-L10)。
- **冲突与空缺**：现行文法仅有具名声明 `trait Serialize where format: Symbol { ... }`，而部分设计讨论记作 `for<Symbol> trait<Symbol>`。目前文法中既没有高阶匿名 `trait { ... }` 表达式产生式，也未定义 `pure comptime fn Serialize<format: Symbol> -> Trait` 返回一等 `Trait` 值时的包归属权（coherence）与实现索引键规则。
- **推荐方案（Option A）**：坚持以具名声明 **`pub trait Serialize where format: Symbol { ... }`** 作为唯一规范的 Tier 2 参数化 trait 写法；将 `for<Symbol> ...` 严格保留给 `type-identity.md` 所定义的纯编译时函数类型/值缩写，不开放匿名的头等 `trait` 值构造器，确保 `docs/traits/coherence.md` 基于名义 `TraitId` 的静态索引与重叠检查保持完备可判定。
- **备选方案（Option B）**：若未来确实需要高阶 trait 家族，需在文法中正式引入 `Trait` 宇宙/类型描述、匿名 `trait` 表达式，并明确正常型头 `Trait` 构造器的不可展开性及包归属绑定规则。

### Q2. Tier 2 官方薄协议包的命名空间与包管理识别约定（高优先）
- **精确来源依据**：[docs/code-organization/builtin-packages.md](../../docs/code-organization/builtin-packages.md#L22-L41)（明确仅存在 6 个内置包）、[AGENTS.md](../../AGENTS.md) 第 1 节（包与工程统一在 `luna` 单二进制 CLI 管理）。
- **空缺描述**：Tier 2 包必须置于 6 个内置包之外，但官方维护的薄协议包（如 `serialization`、`http`、`async_runtime`、`crypto`）需要清晰的命名空间约定，避免与社区包重名抢占，同时不能享有编译器暗中偏袒或特殊 prelude 注入特权。
- **推荐方案（Option A）**：在包注册表/组织命名空间使用官方组织域标识（如工程依赖声明为官方组织下的 `serialization`、`http` 等，源码内 `use serialization.Serialize;` 直接使用短包名且**不注入**隐式 prelude），与普通 Tier 3 包遵守完全相同的 `luna` 依赖解析与孤儿规则。
- **备选方案（Option B）**：统一采用层级包前缀（如 `flurry_proto.serialization` 或 `std_proto.serialization`，源码写成 `use flurry_proto.serialization.Serialize;`），以命名空间前缀直观区隔 Tier 2 与 Tier 3，但会加长日常导入路径。

### Q3. 边界集合项在 `std.collections`（`07`）vs `collections_ext`（`10`）之间的最终划界（中/高优先）
- **精确来源依据**：[07-std-mem-collections-io-os.md](07-std-mem-collections-io-os.md#L77)（line 77 与 §5 Q11 边界审计标记）、[tmp/silver-design/04-compiler-wishlist.md](../silver-design/04-compiler-wishlist.md)（P1-4 `NodeIndex` 与 `SoA`）、[docs/type/runtime-dependent-type.md](../../docs/type/runtime-dependent-type.md#L41-L139)。
- **边界项清单**：`SmallVec<T, N>`、`RingBuffer<T, N>`、`BitSet`、`BTreeMap`/`BTreeSet`、`BinaryHeap`、`Arena<T>`+`NodeIndex`、`SlotMap<K, V>`、`SoA<T>`，以及 `07` 中的专用分配器（`SlabAllocator`/`PoolAllocator`/`ScratchAllocator`/`TrackingAllocator`/`FailoverAllocator`）。
- **推荐方案（Option A — 最小基线 + Silver 运行时依赖保留）**：在 Tier 1 `std.collections` 保留 `Vec`、`VecDeque`、`HashMap`、`HashSet`、`BTreeMap`、`BTreeSet`、`BinaryHeap`、`Slice`/`MutSlice`，以及作为运行时依赖类型最小规范示范与系统无堆队列的 `Arena<T>`（含 `NodeIndex`）和 `RingBuffer<T, N>`；将依赖复杂结构反射生成与重度 DOD/ECS 抽象的 **`SoA<T>`、`SlotMap<K, V>`**，以及图/持久化集合统一迁入 Tier 2 `collections_ext` 协议与 Tier 3 实现包；`SmallVec`/`BitSet` 及专用分配器视 `std` 体积预算二选一（建议 `SmallVec` 留 `std.collections`、`PoolAllocator`/`SlabAllocator` 移入外部 `allocators_ext` 或保持在 `std.mem`）。
- **备选方案（Option B — 极简 `std`）**：`std.collections` 仅保留 `Vec`、`VecDeque`、`HashMap`、`HashSet`、`Slice`/`MutSlice`，将 `BTree*`、`BinaryHeap`、`BitSet`、`SmallVec`、`RingBuffer`、`SlotMap`、`SoA` 全部移出至 Tier 2/3；`Arena` 仅留在 `std.mem` 的 `ArenaAllocator` 层而不对外暴露集合级 `SlotMap`/`SoA`。

### Q4. Tier 2 领域元数据多属性叠加时的合并机制（`attrs.metadata` 浅覆盖冲突）（高优先）
- **精确来源依据**：[docs/comptime/official-attribute-keys.md](../../docs/comptime/official-attribute-keys.md#L10)（规则 3："用户自定义元数据的内部载荷应通过 `^attrs.metadata({ ... })` 封装；用户自建属性对象中的非官方业务字段不使用 `__flurry_` 前缀"）vs [docs/comptime/attribute.md](../../docs/comptime/attribute.md#L212-L218)（`merge_attributes` 按顶层键执行浅覆盖 `result[key] = value`）。
- **冲突现象**：若两个独立属性 `^validation.attrs.non_empty` 与 `^validation.attrs.min_len(3usize)` 都按规则 3 第一句返回 `attrs.metadata({ ... })`，则两者顶层键均为 `__flurry_metadata`，经 `merge_attributes` 浅合并后外层属性将**彻底覆盖抹掉**内层的 `min_len` 载荷！
- **推荐方案（Option A）**：允许 Tier 2 领域包与用户属性直接返回**带包前缀的普通非 `__flurry_*` 顶层键对象**（如 `{ validation_non_empty: true }`、`{ validation_min_len: 3usize }`，符合 `official-attribute-keys.md` 规则 3 后半句），在 `merge_attributes` 下天然共存互不覆盖；将 `^attrs.metadata({ ... })` 仅用于单次打包附着或由编译器支持嵌套 `Object` 深合并/列表收集。
- **备选方案（Option B）**：扩展 `attrs.metadata` 为多键值收集或为 `merge_attributes` 引入同名 `__flurry_metadata` 条目列表保留语义（要求修改 `attribute.md` 既有合并伪代码）。

### Q5. `Deserialize` 等构造函数型协议对“自身携带 vs 不携带 `allocator` 的 `Self`”统一签名机制（中优先）
- **精确来源依据**：[library/std/mem_impl/boxed.fl](../../library/std/mem_impl/boxed.fl#L12-L21)、[docs/traits/declarations.md](../../docs/traits/declarations.md#L88-L90)、[issues/0001](../../issues/0001-runtime-dependent-origin-and-default-param.md)。
- **冲突与空缺**：当 `Self` 是无堆 POD（`struct Point { x: i32, y: i32 }`）时，`Deserialize<.json>` 完全不需要分配器参数；当 `Self` 本身带有运行时依赖分配器参数（`struct Data where .A: Type = DefaultAllocator, quote .allocator: lift *A = ...`）时，调用 `Data<.A = FixedBufferAllocator, .allocator = fba.ref>.as(Deserialize<.json>).deserialize(bytes)` 需要将 `Data` 类型参数上的 `alloc` 自动用作内部字段 `String<.A = A, .allocator = alloc>` 的构造环境，而方法签名上额外的默认 `.A` 会形成两套独立但易混淆的分配器通道。
- **推荐方案**：区分两种模式并在派生器中统一检查：对于自身带 `quote .allocator` 参数的类型，`impl Deserialize<.json> for Data<.A = A, .allocator = alloc>` 优先复用类型本身的 `alloc`（或者方法级 `.A` 仅用于解析过程中的临时 scratch 缓冲）；同时推进 `issues/0001` 解决泛型方法/闭包内隐式 `RuntimeEnvParameter` 透传阻塞。

### Q6. 确认 Tier 2/3 开放 `Symbol` 协议禁止默认派生策略与 `regex_protocol` 对接（中优先）
- **精确来源依据**：[02-pattern-matching-and-views.md](02-pattern-matching-and-views.md#L1294-L1299)（`WL-02-15` 与示例 C 已按 Tier 2/3 `user_regex` 消费示例对齐）、[docs/traits/derivation.md](../../docs/traits/derivation.md#L59)。
- **需确认事项**：确认 Tier 2 官方薄协议包（`serialization`、`database`、`regex_protocol` 等）内部**不为开放 `Symbol` 参数注册默认 `^attrs.derivation.default` 策略**（防止阻断 Tier 3 生态多策略竞争），并明确 `02` 示例 C 中的 `user_regex` 模式编译器在完整生态中通过实现 Tier 2 `regex_protocol.PatternCompile<.dfa_linear>` 与 Tier 1 `TuplePatternView` 进行标准分包组织。
