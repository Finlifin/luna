# Flurry 演进手记 (088)：行多态（Row Polymorphism）——错误行与效应行的语义差异

> **所属卷**：第四卷：元编程即类型论——走进两级类型论 (2LTT) 与编译期宇宙 (066–095)  
> **上一篇**：[087：读懂类型约束与诊断信息——组合约束推理、残余条件与签名设计](./087-ptc-theory-solvers-and-diagnostics.md)  
> **下一篇**：[089：任意位宽整数（Arbitrary-Sized Integers）：从 `u4`、`i24` 到位表示契约](./089-arbitrary-sized-integers-and-bit-widths.md)  
> **参考规范**：[`docs/type/rows.md`](../docs/type/rows.md)

---

## 一、 为什么错误行与效应行无法共用同一套去重规则？

在 Flurry 中，行（Row）用于描述一个类型中可能出现的错误分支（`![E1, E2] T`），或者一段计算需要使用的代数效应（`#[eff1, eff2] T`）。通过行多态（Row Polymorphism），函数可以只处理自己关心的那部分错误或效应，将其余部分保留在行中传递给调用方。

虽然错误行和效应行在表面语法上都写成方括号列表，但它们携带的语义信息并互异：
1. **错误行（Error Rows）描述结果分支集合**：
   - 调用方关心的是“计算失败时可能返回哪几种错误类型”。同一个错误类型即便在函数内部的多个步骤中都可能触发，最终返回给调用方的错误类型集合亦免于因此多出一个重复分支。
   - 因此，错误行是**唯一标签行（Unique-Label Row）**，相同的错误类型在合并时会自动去重：`![InputError] ![InputError] u32 == ![InputError] u32`。
2. **效应行（Effect Rows）还描述同种操作由哪一层处理器（Handler）接管**：
   - 效应非但标识操作本身，还对应外围处理器的作用域层级。当内层效应处理器在处理 `read_number` 时，自身又向外层请求了一次 `read_number`，内层与外层的同名效应属于两个各异的处理层次。
   - 因此，效应行是**保留同标签层次的行（Scoped-Label Row）**，同名效应的多层限定无法自动合并去重：`#[read_number] #[read_number] u32 != #[read_number] u32`。

```text
  ┌────────────────────────────────────┬────────────────────────────────────┐
  │ 1. 错误行 (Error Rows)             │ 2. 效应行 (Effect Rows)            │
  │    语法: ![E1, E2] T               │    语法: #[eff1, eff2] T           │
  ├────────────────────────────────────┼────────────────────────────────────┤
  │ • 语义模型: 唯一标签集             │ • 语义模型: 保留同标签层次的行     │
  │ • 交换规则: 不同错误标签可交换顺序 │ • 交换规则: 仅已证明不相等的标签   │
  │   ![A, B] T == ![B, A] T           │   可交换: #[a, b] T == #[b, a] T   │
  │ • 重复标签: 相同错误自动合并去重   │ • 重复标签: 保留层次，不合并去重   │
  │   ![E] ![E] T == ![E] T            │   #[e] #[e] T != #[e] T            │
  └────────────────────────────────────┴────────────────────────────────────┘
```

---

## 二、 错误行示例：多次传播同一错误免增重复分支

先看错误行的组合方式。下面的 `byte_pair` 连续调用了两次 `checked_byte`，两次调用都通过 `!` 传播 `InputError`，但 `byte_pair` 的返回类型只需列出一个 `InputError`：

```flurry
use std.prelude.*;

enum InputError {
    out_of_range(u32),
}

fn checked_byte(value: u32) -> ![InputError] u32 {
    if value <= 255 {
        value
    } else {
        error InputError.out_of_range(value)
    }
}

fn byte_pair(left: u32, right: u32) -> ![InputError] Tuple<u32, u32> {
    let first = checked_byte(left)!;
    let second = checked_byte(right)!;
    (first, second)
}

test {
    byte_pair(12, 300) match {
        (left, right)! => println("pair: {}, {}", left, right),
        error .out_of_range(value) => println("invalid byte: {}", value),
    }
}
```

输出为 `invalid byte: 300`。错误的身份是 `InputError`，具体哪一个数值越界由变体载荷 `value` 说明，返回类型无需记录该错误来自第几次调用。

---

## 三、 效应行示例：重复标签代表处理层次，而非调用次数

阅读效应行时需要注意一个常见区分：**效应行中的重复标签表示嵌套的处理层次，不表示调用次数。**
- 在同一个处理器作用域内，连续调用两次 `read_number()#`，使用的仍然是同一个处理入口，函数签名依然是单层的 `#[read_number]`；
- 当内层 `# { ... }` 处理器在消解一层 `read_number` 的同时，又在处理器分支内部向外层发起 `read_number()#` 请求时，才会在嵌套层次上区分内层入口与外层入口：

```flurry
use std.prelude.*;

effect read_number() -> u32;

-- 连续调用两次 read_number()#，共享同一层处理入口，类型仍为单层 #[read_number]
fn number_pair() -> #[read_number] Tuple<u32, u32> {
    let first = read_number()#;
    let second = read_number()#;
    (first, second)
}

-- 内层处理器接管 number_pair 的请求，并在分支内部向外层再次请求 read_number()#
fn translated_pair() -> #[read_number] Tuple<u32, u32> {
    number_pair()# {
        read_number() => {
            let outside = read_number()#;
            outside / 2
        },
    }
}

test {
    let (left, right) = translated_pair()# {
        read_number() => 10,
    }
    println("pair: {}, {}", left, right);
}
```

输出为 `pair: 5, 5`。`number_pair` 中的两次请求都进入 `translated_pair` 的内层处理器，而内层处理器中的 `read_number()#` 则路由到 `test` 块安装的外层处理器（取得 `10` 并折半为 `5`）。

---

## 四、 编译期验证：错误行合并与效应行分层

我们可以直接在 `comptime` 块中验证这两类限定在类型归一化时的等价关系：

```flurry
use std.prelude.*;

enum InputError {
    out_of_range(u32),
}

effect read_number() -> u32;

typealias OneError = ![InputError] u32;
typealias RepeatedError = ![InputError] ![InputError] u32;
typealias OneLayer = #[read_number] u32;
typealias TwoLayers = #[read_number] #[read_number] u32;

comptime {
    -- 错误行自动合并相同的错误类型
    assumes OneError == RepeatedError;
    -- 效应行保留同名效应的处理层次，单层与双层类型互异
    assumes OneLayer != TwoLayers;
}
```

相应地，如果一个函数签名携带双层效应 `#[read_number] #[read_number] u32`，禁止于未消解其中一层的情况下直接赋给只接受单层 `#[read_number] u32` 的函数类型位置：

```flurry
effect read_number() -> u32;

typealias NestedReadFn = fn() -> #[read_number] #[read_number] u32;

fn reject_implicit_effect_flattening(callback: NestedReadFn) {
    -- 编译错误：#[read_number] #[read_number] u32 与 #[read_number] u32 的效应层次不同
    let _: fn() -> #[read_number] u32 = callback;
}
```

此外，当行中包含带类型参数的标签时，只有已证明互异的两个效应标签才允许交换顺序；在参数尚未确定、两个标签实例化后可能相等的情况下，编译器免随意调换它们的顺序，以保证处理器的绑定层次不会因实例化而改变。

---

## 五、 本篇小结

| 特性维度 | 错误行（`![E1, E2] T`） | 效应行（`#[eff1, eff2] T`） |
| :--- | :--- | :--- |
| **语义模型** | 唯一标签行（Unique-Label Row） | 保留同标签层次的行（Scoped-Label Row） |
| **重复标签规则** | 自动合并去重：`![E] ![E] T == ![E] T` | 保留层次与相对顺序：`#[e] #[e] T != #[e] T` |
| **多次调用同一操作** | 免增重复错误标签 | 共享同一层处理入口，免增重复效应标签 |
| **不同标签顺序交换** | 任意交换顺序 | 仅在两个标签已证明互异时可交换 |

---

## 六、 下一篇预告

除了类型与行约束外，系统编程中另一类常见的静态约束是位宽与数值范围。下一篇 **[089：任意位宽整数（Arbitrary-Sized Integers）：从 `u4`、`i24` 到位表示契约](./089-arbitrary-sized-integers-and-bit-widths.md)** 将介绍 Flurry 中任意位宽整数的逻辑位宽、独立存储布局以及与 `BitVec<n>` 的转换规则。
