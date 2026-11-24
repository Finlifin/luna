# CMTT 代码组装 (`library/meta/code`)

`library/meta/code/` 提供已类型检查的 CMTT quoted code 元数据模型、错误分类与代码证据组合原语，并通过 `library/meta/lib.fl` 在 `meta` 根导出（与 `meta/reflection`、`meta/definitions` 约定一致）。严格遵守两阶段、rank-1 上下文、每段 quote 至多一个 splice，以及不把 `quote fn` 当作 `comptime fn` 的边界。

## 已导出 API（`meta.*`）

- `types.fl`:
  - `CodeErr`：代码组装与证据恢复错误枚举（`InvalidCodeKind`、`MissingProgramBody`、`InterfaceMismatch`、`EmptySequence`、`IndexOutOfBounds`、`UnresolvedContext`、`DuplicatePort`、`ControlEscape`）。
  - `CodeKind`：区分 `expression` 与 `pattern` 两类 code 证据。
  - `ContextEntry`、`CodeContext`、`Substitution`、`CodeInterfaceSummary`：描述 rank-1 runtime 上下文条目、端口映射与上下文接口摘要，方法均使用 comptime 值引用 `self`。
- `combinators.fl`:
  - `Any` 证据转发与分支选择：`keep(value: Any) -> Any`、`select_any(flag: bool, when_true: Any, when_false: Any) -> Any`（待 open-function 补齐 `Any` 域上的 `pure` 行为证据后升格为 `pure comptime fn`）。
  - 表达式代码组合：`keep_expr`、`select_expr`、`first_expr`、`last_expr`、`count_exprs`（基于 `Any` 证据恢复与 variadic 代码集合 `'tuple_enumerate`）。

## 阶段与边界说明

- 编译器 `elaboration` 内部已提供通用的卫生多表达式块序列组装引擎（`assemble_code_sequence`）与 `Any`/变长代码集合支持。
- `meta.code` 命名空间级 `pub mod code` 需目录命名拆分为 `code_impl/` 或编译器目录模块可见性支持（当前 `code/` 目录由 `vfs_scanner` 保留为包内私有模块，因此统一从 `meta.*` 导出）。
- 用户侧一等 `List<Code>` / `.code_sequence'builtin` 组合原语仍等待 `Term::ExprCode` 与跨 crate `nir`/`comptime_interpretor`/`bindings.fl` 接线完成后再纳入公开签名，不在库层用 `Option`/`Result` 枚举或特判绕过单孔约束。
