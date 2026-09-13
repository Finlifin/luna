# 安全等级

Flurry 为函数和代码块提供三个安全等级，每个等级对应不同的静态检查强度和使用场景。等级之间有明确的边界规则，可以在同一代码库中共存。

## `normal`（默认）

不加任何关键字标注的函数和块处于 `normal` 等级，这是日常编程的默认选择。

`normal` 代码受到以下静态保证：

- **仿射类型**：非 `Copy` 值至多被使用一次，防止 double-free。
- **可达性分析**：指针和引用必须在其指向数据的生命周期内使用，防止悬垂指针。
- **副作用追踪**：通过效应系统显式标注函数的副作用，编译器对效应传播进行静态检查。
- **Normalization**：在函数定义端进行符号执行和类型检查，尽早暴露错误。

```flurry
fn process(data: *Config) -> String {
    data.name.clone()
}
```

## `unsafe`

在函数或代码块上标注 `unsafe`，显式 opt-out 编译器的部分静态检查：

```flurry
unsafe fn raw_memcpy(dst: *u8, src: *u8, len: usize) {
    -- 指针算术、无边界检查的访问等底层操作
}

fn safe_wrapper(dst: *u8, src: *u8, len: usize) {
    -- 调用 unsafe 函数需要在 unsafe 块内
    unsafe {
        raw_memcpy(dst, src, len);
    }
}
```

`unsafe` 的合理使用场景：

- 系统调用和驱动接口
- FFI（与 C 库交互）
- 特定数据结构的底层实现（如锁无关的并发容器）
- 性能极端敏感的内部路径

**原则**：`unsafe` 不意味着"无规则"。开发者有责任在 `unsafe` 边界处手动维护那些编译器不再强制执行的不变量，并通过文档说明。未来，`verified` 等级可以用于机器验证这些不变量。

## `verified`

在函数上标注 `verified`，要求编译器对函数的行为进行更强的形式化验证：

```flurry
verified fn binary_search(arr: Slice<i32>, target: i32) -> ?usize
    requires arr.is_sorted()
    ensures result? => arr[result?] == target
{
    ...
}
```

`verified` 函数通过 `requires`（前置条件）和 `ensures`（后置条件）子句声明逻辑规范。编译器（结合外部 SMT 求解器或证明助手）负责验证实现满足规范。

设计目标包括与以下形式化方法的集成：

- **Outcome Separation Logic（OSL）**
- **Reachability Logic（RL）**
- **Matching Logic**

`verified` 主要面向需要高可信度的核心组件：密码学原语、内核关键路径、金融逻辑等。

> 注：`verified` 等级的完整编译器支持尚在开发中，上述语法为设计意图。

## 等级之间的调用规则

| 调用方 → 被调方 | `normal` | `unsafe` | `verified` |
|----------------|----------|----------|------------|
| `normal`       | ✓        | 需要 `unsafe { }` 块 | ✓（验证会传播到调用方） |
| `unsafe`       | ✓        | ✓        | ✓ |
| `verified`     | ✓（需证明调用满足前置条件） | 需要 `unsafe { }` 块并提供证明 | ✓ |

## 安全哲学

Flurry 的安全等级体系体现了以下设计理念：

1. **分层控制**：不同场景需要不同程度的保证。一刀切的"全部 safe"无法满足系统编程需求；一刀切的"允许一切"则丢失了工程价值。
2. **显式边界**：`unsafe` 必须明确标注，不能无声地在代码中扩散。这使代码审查和安全审计有明确的聚焦点。
3. **可验证的 `unsafe`**：`verified` 等级提供了一条将 `unsafe` 代码纳入形式化保证的路径，使"先写 unsafe，后验证"成为可行的工程流程。
