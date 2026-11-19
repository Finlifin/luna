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

### `'tag_name`

从枚举值获取当前激活变体的标签名称（`str`），用于调试、格式化及反射相关设施。

### `'old`

用于在 `ensures` 后置条件中引用函数入口处参数状态的快照（如 `param'old`），按函数契约的入口快照边界采集，不可用于普通函数体表达式。

### `'bitvec`

用于 `BitVec<N>` 的位向量视图投影与位切片定位，服务于读/写位置语义及边界验证（参见 BitVec 与位域文档）。

### `'multi_deref`

用于多级指针解引用位置访问；单层解引用统一使用 `ptr.*`。

## 编译器内部专用 view kind

- `'builtin` 与 `'intrinsic`（例如 `.Tuple'builtin`）仅限六个内置包（`builtin`、`meta`、`build`、`attrs`、`std`、`verification`）内部的集中绑定定义使用。
- 普通用户代码及库高层实现禁止直接书写 `'builtin` 或 `'intrinsic`，必须通过公开导出名称或集中绑定访问。
