# 跨阶段值传递

## Intro

编译时得到的结果，可以决定生成的程序使用什么值。这个过程既要确定目标类型，也要保留构造该目标值所需的操作。

有些结果可以直接表示为常量，有些需要运行时转换。知道输入来自编译时，并不足以判断转换是否有运行时代价。

## 语法

普通值跨阶段使用不要求一个统一的显式 splice 表达式。以下接口描述其转换方法；声明、关联类型和参数形式复用[全局语法](../grammar/syntax)。

```flurry
trait Splice {
    assoc RuntimeTarget: Type;
    fn splice(comptime data: Self) -> RuntimeTarget;
}
```

相关局部形式：

```text
comptime_parameter -> comptime parameter
lift_type -> lift expr
```

`quote` 的代码表示与 contextual modal 类型规则属于代码引用和生成机制；普通值转换不依赖于在这里展开其表面语法。

## 正式定义

### 类型与阶段

类型在 comptime/runtime 之间可以有不同关系：

| 情形 | 跨阶段使用 |
|---|---|
| 类型仅在 comptime 存在 | 其值进入 runtime 时，需要对应的运行时目标类型 |
| 类型在两阶段都有对应物 | runtime 一侧为该类型的 proxy，按相应规则构造其值 |
| 结果只作为 comptime 输入使用 | 不因参与类型或实例化计算就要求具有 runtime 表示 |

这些是通用分类。`Integer`、`Float` 是仅编译时类型的例子；`str`、`Symbol` 有对应的两阶段表示。具体字面量规则见[字面量](../basics/literals.md)。

### Splice 是普通 runtime 方法

`Splice.splice` 接受 comptime 输入，返回 `RuntimeTarget` 的 runtime 值。它本身是普通 runtime 方法：编译时根据输入实例化，运行时调用相应实例。

因此，不能把“完成实例化”解释成“已经在编译时执行了转换方法的全部主体”。若实例中的操作可以直接物化为常量，编译器可以采用保持行为的表示；否则保留必要的 runtime 调用和构造过程。

跨阶段结果必须满足目标类型的表示、布局与初始化要求。comptime 源值可被执行器保存，不等于其内部地址或对象图可以原样成为目标程序中的值。

### 有限自动派生

Splice 使用显式 trait。struct、enum、union、closure 等指定类型可以在满足条件时自动派生；它不是任意类型都自动获得的能力。

自动派生需要成员或捕获满足相应转换条件，并能建立合法的目标表示。共享引用、循环结构、未初始化字段和 Drop 字段属于自动派生的拒绝情况。复杂类型通过显式实现表达精确的转换语义。

自动派生失败不等于禁止为该类型显式实现 Splice；显式实现仍须满足目标类型与 trait 检查规则。

### Quote 与 lift 的边界

运行时的实际值不能在编译时被读取。quote 看似引用运行时内容，操作的却是可在编译时持有的符号或代码表示，而不是未来运行时内存中的数据。

lift 将相应运行时类型的表示带入编译时类型操作。代码引用还有 contextual modal 类型所约束的 quote/splice；代码值本身属于 comptime 值，但其代码拼接规则与普通数据转换不同。

这一区分不改变单向的数据边界：持有运行时代码的表示，不等于已经执行了该代码。

## 常见示例

### 常量结果供运行时使用

```flurry
fn print_total() {
    println("{}", 1 + 23);
}
```

加法先得到 comptime 值 24；该值按对应类型的规则供运行时打印使用。这里仍有 runtime 打印操作。

### 编译时参数不需要作为参数跨界

```flurry
fn choose_value(comptime first: bool, left: i32, right: i32) -> i32 {
    inline if first {
        left
    } else {
        right
    }
}
```

`first` 在实例化中决定分支，运行时不再传入它。这与返回一个需要由 Splice 构造的 runtime 值是两回事。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | 常量求值能产生常量上下文允许的值；Flurry 还用带 comptime 输入的 runtime Splice 方法表达转换。[Rust 常量表达式](https://doc.rust-lang.org/reference/const_eval.html#constant-expressions) |
| Zig | 编译时类型和值可用于生成 runtime 代码；Flurry 普通值转换有显式 Splice trait 契约。[Zig comptime](https://ziglang.org/documentation/master/#comptime) |
| C11 | 无对等的 Splice trait；实际可用静态初始化数据和普通构造函数组合实现。[C11 草案 §6.7.9](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | 常量表达式与普通构造过程可组合使用；Flurry 的转换输入明确处于 comptime，并在 runtime 调用实例。[C++ 常量表达式](https://eel.is/c++draft/expr.const) |

## 运行时实现设计引导

### 执行转换实例

1. 取得已完成实例化的 Splice 方法；comptime 输入已用于决定该实例。
2. 执行实例中的 runtime 操作，取得 `RuntimeTarget` 值。
3. 将结果交给使用它的 runtime 操作，遵守目标值的初始化与资源契约。

若编译器以静态数据或直接构造替代调用，必须保持原转换的可观察行为。不能仅因输入在编译时已知就删除实例中的副作用。

不变量：目标程序不使用 comptime engine 的宿主地址作为自己的有效对象地址。转换代价由具体实例决定，不承诺 O(1) 或零分配。

## 编译器实现引导

### 跨界降码

1. 识别结果是否真正进入 runtime 数据流。只用于类型检查或 comptime 参数的结果不走普通值转换。
2. 根据输入类型及目标约束，检查可用的 Splice 实现与其关联目标类型。
3. 以 comptime 源值实例化普通 runtime 方法；检查该实例及目标表示。
4. 将调用结果接入 runtime IR。存在直接常量表示时，进行保持语义的物化或简化。
5. 若目标表示不可建立、实例化失败或实现不满足要求，报告编译错误。

### 自动派生判定

对进入有限派生范围的聚合类型，执行以下保守检查：

1. 枚举派生所需的字段、分支或捕获信息。
2. 检查成员 Splice 能力及目标表示；信息不足时不能以原始字节复制代替证明。
3. 检查共享引用、循环结构、未初始化字段与 Drop 字段等拒绝条件。
4. 检查目标聚合的布局与构造是否满足相应类型规则。
5. 条件全部成立后生成实现，并按该环境的 coherence 规则检查。

对有限已知结构的遍历本身可按成员数量线性进行；它不包含递归实例化、成员自定义转换或布局计算的代价。遇到循环和资源耗尽时必须结束当前派生尝试，不能无限展开或伪造成功。
