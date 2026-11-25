# Flurry 官方属性键集中规范

## 规则

1. **统一前缀**：所有 Flurry 官方保留属性键（由编译器各阶段、标准库或官方宏系统识别与消费的键）**必须以 `__flurry_` 开头**，禁止引入无 `__flurry_` 前缀的官方属性键（如 `packed`、`abi`、`flurry_macro_kind` 等）。
2. **禁止裸附 key（含定义作用域内的 `.id expr` configuration），必须包装在 `attrs` 中**：
   - 定义作用域内的 `configuration`（`.id expr`，如结构体内的 `.packed true`）使用的是裸键语法；因此官方属性**不再使用定义作用域 `.id expr`（configuration）或 `^{ key: value }` 裸对象形式直接定义或附着**。
   - 所有官方属性必须通过 `attrs` 包的公开常量或 `pure comptime fn` 构造器（如 `^attrs.abi(.c)`、`^attrs.layout.packed`、`^attrs.comptime_phase`、`^attrs.effects.direct_passing`、`^attrs.metadata(...)`、`^attrs.stable_since(...)`）对外暴露，避免在社区中形成直接依赖裸键名或用 `.id expr` 拼写官方属性的习惯。
   - 只有 `library/attrs/` 的内部实现模块与编译器对应的属性消费者允许引用 `__flurry_*` 内部键名。
3. **与普通用户数据区分**：用户自定义元数据的内部载荷应通过 `^attrs.metadata({ ... })` 封装；用户自建属性对象中的非官方业务字段不使用 `__flurry_` 前缀。

---

## 官方属性键集中登记表

> 新增任何官方属性键前，必须先在本表登记，并在 `library/attrs/` 提供公开包装入口。

| 规范内部键名 | `attrs` 公开包装入口 | 值域 | 消费方与现状 |
| --- | --- | --- | --- |
| `__flurry_stable_since` | `attrs.stable_since(version: str)`（`attrs.prelude`） | `str` | ABI `type_id` 规范版本锚点（[type_id.md](../type/type_id.md)；构造器已按规范命名，消费者待接通） |
| `__flurry_comptime_phase` | `attrs.comptime_phase`（`attrs.prelude`） | `bool` | trait 成员阶段检查（当前源码仍使用旧键 `comptime_phase`，待统一迁移） |
| `__flurry_packed_mode` | `attrs.packed.byte`、`attrs.packed.bit`（保留 `attrs.layout` 过渡） | `Symbol`（`.byte` \| `.bit`） | 结构体紧凑布局：`byte` 为 `align = 1` 任意 runtime 字段（禁对齐字段指针）；`bit` 为 POD 位打包直降 `BitVec<W>`（`W > 256` 警告；`attrs` 构造器已落地，消费者待接通至 `PackedLayoutMode`） |
| `__flurry_direct_passing` | `attrs.effects.direct_passing` | `bool` | 零参非 `async` 直接传递效应检查（当前源码仍使用旧键 `direct_passing`，待统一迁移） |
| `__flurry_abi` | `attrs.abi(name: Symbol)`（`attrs.prelude`） | `Symbol` | 函数调用约定（已接通 `^attrs.abi(.c)` 与 LLVM C wrapper/trampoline 降码，`fa4cd077`） |
| `__flurry_purity` | `attrs.purity.{p0, p1, p2, p3, p4, p5}` | `Symbol`（`.p0` \| `.p1` \| `.p2` \| `.p3` \| `.p4` \| `.p5`） | 运行时函数纯度层级约束（已接通 P0–P5 编译期纯度与终止／无 trap 检查） |
| `__flurry_metadata` | `attrs.metadata(data: Object)`（`attrs.prelude`） | `Object` | 用户自定义元数据包装（当前源码仍使用旧键 `metadata`，待统一迁移） |
| `__flurry_derivation_default` | `attrs.derivation`（`default` 成员导出待语法放行） | `bool` | 默认派生策略标记（当前源码仍使用旧键 `derivation_default`，待统一迁移） |
| `__flurry_volatile` | `attrs.mem.volatile`（或 `attrs.volatile` 包装，禁止裸附） | `bool` | 易失内存／MMIO 访问控制（已端到端接通 `struct`/`newtype`/字段级 `load volatile`/`store volatile` 与纯度 P0 降级，`ef1eac47`） |
| `__flurry_quote_fn_template_for_match_arms` | `attrs.quote_fn_template.for_match_arms` | `bool` | 标记无体 `template quote fn` 为 quoted match arm 模板（`use expr` in `match`） |
| `__flurry_quote_fn_template_allow_non_exhaustive_instance` | `attrs.quote_fn_template.allow_non_exhaustive_instance` | `bool` | 允许单条 `quote fn ... (case ...) use template` 实例非穷尽（最终 `match` 仍须穷尽） |
| `__flurry_pattern_stable` | `attrs.pattern.stable` | `bool` | 标记用户 `case` 视图可稳定重复求值／去重并纳入 `pattern_automata` 决策 DAG |
| `__flurry_macro_*`（预留） | 待在 `library/attrs/` 新增对应宏包装构造器 | — | 宏系统展开元数据（禁止使用未加 `__` 前缀或裸附的 `flurry_macro_kind`） |
