# Flurry 演进手记 (183)：局部类型推导与递归布局边界——闭包接收者链、错误隔离与自引用指针

在[上一篇（第 182 篇）](./182-elaboration-sibling-parameters-and-call-plans.md)中，我们看到了函数调用如何结合外层期望类型与兄弟实参，自动推导出 `apply_first(|item| item + 1, 41i32)` 中闭包参数 `item: i32` 的类型。

在实际工程中，我们还会遇到另外两类与局部依赖和类型布局密切相关的场景：
1. **闭包内的字段访问与方法调用链**：当闭包参数类型由后续兄弟实参确定时，闭包体内部的字段投影（`item.count`）与方法调用（`item.scale(2)`）如何顺畅推导？如果某个实参写错了变量名，为什么不会引发连环的虚假报错？
2. **自引用结构体与指针布局边界**：在定义链表、树节点或侵入式队列时，为什么 `struct Link { next: ?*Link, value: i32 }` 合法，甚至可以在定义旁直接查询 `Type.size_of(*Link)`，而按值内嵌自身 `struct BadLink { next: BadLink, value: i32 }` 却会报错？

今天，我们就来厘清这两条直接影响日常编码体验的语言规则。

---

## 一、 闭包内的成员访问与局部推导链

在 Flurry 中，当匿名函数 `|item| ...` 的参数类型依赖外部调用上下文时，一旦兄弟实参或外层返回类型确定了 `item` 的具体类型，闭包体内部的字段读取、运算符重载与方法接收者适配（Receiver Adaptation）都会随之自动解开：

```flurry
struct Reading {
    channel: u8,
    millivolts: i32,
}

impl Reading {
    pub pure fn calibrated(*self, offset: i32) -> i32 {
        self.millivolts.* + offset
    }
}

fn transform_reading(
    mapper: fn(Reading) -> i32,
    sample: Reading,
) -> i32 {
    mapper(sample)
}

test closure_member_inference {
    let input = Reading { channel: 1, millivolts: 250 }

    -- 第二个实参 input 确定了闭包参数 r 的类型为 Reading，
    -- 因此闭包内的 r.calibrated(10) 自动完成接收者适配 (.ref) 与方法绑定。
    let out = transform_reading(|r| r.calibrated(10), input);
    assumes out == 260;
}
```

```text
  [ 闭包成员访问的推导传递链 ]
    实参 2: input : Reading
              │
              ▼ (通过 transform_reading 签名确定 T = Reading)
    实参 1: |r| r.calibrated(10)
              │
              ├──▶ 确定 r : Reading
              ├──▶ 在 Reading 上查找方法 calibrated(*self, i32) -> i32
              └──▶ 自动对 r 执行接收者取址适配 r.ref.calibrated(10)，返回 i32
```

### 错误隔离：上游实参报错时，免生虚假的次生噪音

如果我们在调用 `transform_reading(|r| r.calibrated(10), unknown_value)` 时，偶有疏忽把第二个实参的名字拼错了（写了未声明的 `unknown_value`），会发生什么？

```flurry
fn missing_input() -> i32 {
    -- ❌ 编译期报错：无从寻获标识符 unknown_value
    transform_reading(|r| r.calibrated(10), unknown_value)
}
```

在这种情况下，根本原因是 `unknown_value` 未定义。类型检查器只会报告 `unknown_value` 未定义这一处真实错误，而免因 `r` 没拿到类型再去额外抱怨“在未知类型 `r` 上无从寻获方法 `calibrated`”——这保证了编译诊断始终指向真正需要修复的代码位置。

---

## 二、 递归数据结构的布局边界：指针大小 vs 值布局

在构建链表、红黑树或图结构时，类型定义往往需要引用自身（自引用类型）。
请对比下面这两种写法：

```flurry
-- ✅ 合法：通过可选指针 ?*Link 引用自身
struct Link {
    next: ?*Link,
    value: i32,
}

const PTR_BYTES: usize = Type.size_of(*Link);
const NODE_BYTES: usize = Type.size_of(Link);
```

为什么 `Link` 内部包含了 `?*Link`，却不会陷入“算 `Link` 的大小需要先算 `Link` 的大小”的死循环？

因为在 Flurry 的内存模型中，**指针类型的布局（`*T` 或 `?*T`）与被指向类型的值布局（`T`）是严格分离的**：
- 只要 `Link` 是一个已声明的合法类型名，指向它的指针 `*Link` 和可选指针 `?*Link` 的大小与对齐就只取决于目标平台的地址宽度（例如在 64 位平台上固定为 8 字节）；
- 计算 `*Link` 的大小**无需等待 `Link` 内部字段的布局计算完成**；
- 进而，计算 `Link` 自身的值布局时，只需将 `next: ?*Link`（8 字节）与 `value: i32`（4 字节，加上对齐填充共 16 字节）相加，依赖关系是完全无环的。

```text
  [ 自引用指针结构体的无环布局计算 ]
    Type.size_of(*Link) ───(仅取决于平台指针宽度)───▶ 8 字节

    Type.size_of(Link)  ───┬── 字段 1: ?*Link ─────▶ 8 字节
                           └── 字段 2: i32    ─────▶ 4 字节 (对齐后总计 16 字节)
```

---

## 三、 负例：按值直接或间接包含自身的无限大小结构体

相反，如果忘记写指针符号 `*`，试图在结构体内部直接**按值（By Value）**内嵌自身，或者两个结构体 `A` 和 `B` 互相按值内嵌对方，就会形成真正的值布局依赖环路（无限大小类型），在编译期被直接拦截：

```flurry
struct BadLink {
    -- ❌ 编译期报错：没有通过指针间接引用，而是试图按值内嵌 BadLink 自身，
    -- 导致 BadLink 的物理内存大小无限膨胀，构成非法的递归值布局环路。
    next: BadLink,
    value: i32,
}
```

要修复此类错误，只需将递归字段改为指针（`*BadLink`、`?*BadLink`）或切片引用，用固定宽度的指针打破值包含环路。

---

## 四、 本篇小结

| 场景 | 语言规则 | 实际效果 |
| :--- | :--- | :--- |
| **闭包内字段与方法推导** | 兄弟实参或外层类型确定闭包参数类型后，自动完成内部字段投影与接收者适配 | 在高阶管道中写 `\|r\| r.method()` 流畅自然，无需手工标注参数类型 |
| **上游错误静默隔离** | 当提供类型信息的实参本身有语法或名称错误时，只报主因错误 | 避免因一处拼写错误刷出满屏“闭包参数类型未知”的次生报错 |
| **指针大小独立性** | `*T` 与 `?*T` 的大小只取决于目标架构地址宽度，独立于 `T` 的内部字段布局 | 支持链表、树等自引用指针结构体的定义与编译期 `size_of` 查询 |
| **按值递归环拦截** | 严禁结构体直接或间接按值包含自身 | 在编译期避免无限大小类型 |

---

## 五、 下一篇预告

当我们的工程中存在通用泛型实现（如 `impl Describe for Sample<T> where T`）与针对特定类型的特化实现（如 `impl Describe for Sample<i32>`）时：
- Flurry 是按什么规则在多个匹配的 `impl` 候选中选出唯一赢家的？
- 为什么“写更长的 `requires` 子句”不会提高实现的优先级，唯有“暴露更具体的类型结构”才会优先？
- 当两个实现具有相同的结构特指度（例如 `Pair<T, i32>` 与 `Pair<i32, U>` 在面对 `Pair<i32, i32>` 时），应该如何消歧？

下一篇 **[Flurry 演进手记 (184)：Trait 实现特化与消歧指南——结构精度法则、候选竞争与编译期冲突诊断](./184-candidate-solving-isolated-probing-and-commit.md)**，我们将深入讲解多态候选的选择规则。
