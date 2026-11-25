# 视图提取（View Taking）

Flurry 使用后缀语法 `expr'view_kind`（可选地附带索引或参数）从表达式或位置提取特定语义视图，而不改变语言的核心运算符优先级。所有源码级 view kind 在 AST lowering 阶段进入专门的 HIR 节点并在后续编译阶段按各自的静态契约检查与降码。

## 已接通的源码级 view kind

### `'type`

提取操作数的静态类型身份 `Type`，但不求值该操作数。

```flurry
inline for arg in args'tuple_enumerate {
    const T = arg'type;
    inline if T :- Display {
        write_display(arg);
    }
}
```

- 操作数仅在未求值类型观测作用域中检查，不会捕获运行时变量，也不会触发运行时副作用或消费仿射资源。
- 在开放函数、泛型上下文或受 `Type.predicates.*` / trait bound 精化的分支中，`'type` 保持严谨（rigid）类型身份，直接参与 PTC 谓词与 occurrence typing。

### `'tuple_enumerate`

用于 `inline for element in tuple'tuple_enumerate` 的逐元素静态展开视图。

- 每次迭代直接绑定当前元组元素本身（`element`），**不**生成 `(index, element)` 二元组。
- 异构元组不会被收窄为单一运行期元素类型：每次展开保留各自字段的静态类型与资源所有权；未访问字段在 `break`、`return` 或错误传播时按作用域次序清理。
- 支持空元组 `()`：空元组循环执行零次，但其模式、守卫与循环体仍会按抽象元素类型完成一次静态类型与阶段检查。
- 合法操作数要求以下任一静态证据：
  1. 结构上已确立为具体元组（如 `Tuple<A, B>`、`()` 或通过 `==` 已知）；或
  2. PTC 已证明 `Type.predicates.is_tuple(T)`（例如参数来自带 `where ensures Type.predicates.is_tuple(R)` 的 deferred producer、签名 `requires Type.predicates.is_tuple(T)`、`refines` 或作用域内的 `assumes Type.predicates.is_tuple(...)`）。
- 编译器不会通过窥探类型生产函数体来绕过上述契约要求。

### `'tuple_indexed`

用于 `inline for (index, element) in tuple'tuple_indexed` 的带编译期下标静态展开视图。

- 与 `'tuple_enumerate` 遵循相同的合法元组操作数与异构字段所有权规则，仅额外在每次展开绑定从 `0` 开始的编译期 `Integer` 常量索引 `index`（支持模式 `(index_pat, element_pat)` 或通配符 `_`；两个子模式不能绑定同一标识符）。
- `index_pat` 作为编译期常量阶段绑定参与循环展开、守卫及循环体中的 `const` / `inline if` / `comptime` 参数计算；仅允许在 `inline for` 的迭代器位置使用。

### `'tag_name`

从枚举值获取当前激活变体的标签名称（`str`），用于调试、格式化及反射相关设施。

### `'old`

用于在 `ensures` 后置条件中引用函数入口处参数状态的快照（如 `param'old`），按函数契约的入口快照边界采集，不可用于普通函数体表达式。

### `'bitvec`

用于 `BitVec<N>` 的位向量视图投影与位切片定位，服务于读/写位置语义及边界验证（参见 BitVec 与位域文档）。

### `'attribute`

仅可在 `comptime` 上下文调用，读取附着在类型、实例或目标声明上的合并后 `meta.Object` 元数据（包含官方 `__flurry_*` 条目与用户属性）；目标无任何属性时返回空 `{}`。

- 操作数在类型/目标位置检查，不会消费运行时资源或执行运行时求值，仅允许在 `comptime` 阶段读取（在 runtime 值上下文或 quoted runtime body 中使用会被拒绝）。
- 与命令行 `--emit merged-attributes` 输出使用同一合并属性视图。

### `'multi_deref`

用于沿嵌套指针链执行多级解引用（`***...*T -> T`），单层解引用统一使用 `ptr.*`。

- 要求操作数至少具备一层指针类型（`*T`、`*(*T)` 等），沿结构化类型逐层穿透至最终非指针目标 `T`。
- 中间指针层自动按固有指针读取；最终值读取要求目标类型满足 `Copy` 或内建可读取原语约束，避免绕过 `Once` 所有权移动检查。
- 支持位置写回与成员投影：作为左值或投影基址（如 `pp'multi_deref = v`、`pp'multi_deref.field`、`pp'multi_deref.ref`、`pp'multi_deref'bitvec[lo..hi] = rhs`）时，自动穿透外层指针并定位到最内层 `*T` 位置进行安全读/写或受控借用。

## 编译器内部专用 view kind

- `'builtin` 与 `'intrinsic`（例如 `.Tuple'builtin`）仅限六个内置包（`builtin`、`meta`、`build`、`attrs`、`std`、`verification`）内部的集中绑定定义使用。
- 普通用户代码及库高层实现禁止直接书写 `'builtin` 或 `'intrinsic`，必须通过公开导出名称或集中绑定访问。
