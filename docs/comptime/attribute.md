# 属性系统

## Intro

属性把编译时信息附着到代码元素上。读取这些信息的编译器功能或工具，按相应契约决定怎样处理该元素。

属性表达式计算出的是元数据。某个键是否影响布局、诊断或生成代码，由使用这个键的功能定义。

## 语法

`expr`、`definition`、`statement`、`parameter` 与分支产生式沿用[全局语法](../grammar/syntax)。以下是属性相关的局部形式：

```text
attribute -> ^expr
multi_element -> base_element
    | ^expr multi_element
attributed_definition -> ^expr definition
attributed_statement -> ^expr statement
attributed_parameter -> ^expr parameter
attributed_condition_arm -> ^expr condition_arm
attributed_case_arm -> ^expr case_arm
property -> id: expr
configuration -> .id expr
```

属性表达式可使用对象表达式，例如 `^{ some_attr: 1 }`。多个前缀属性依次写在被修饰的元素之前。

对象表达式已在 AST lowering 中进入专用 HIR Object 节点。普通项与命名项按源码次序保存在同一个列表中，嵌套对象和重复键也保留原结构；前端不因存储方便而提前分组、合并或重排这些项。这只是表达式表示，Object 的类型检查和 comptime 值操作尚未接通。

multi_element 是语法元素列表的通用元记号，base_element 为该列表原本允许的元素。前缀可递归嵌套；这不为任意内部操作数或原始 token 序列增加属性语法。

`property` 是对象或扩展调用中的命名项。`configuration` 写在定义作用域内部，为所属定义编码属性，例如结构体中的 `.packed true`。字段、参数和一般项之间的逗号，遵循[语法风格](../grammar/index.md#分隔符)。

## 正式定义

### 属性的值与目标

`^` 后的表达式在 comptime 求值，结果必须为 `Object`。对象中的键和值构成附着到目标代码元素的属性；不能以运行时才取得的值代替所需的编译时结果。

属性可以修饰定义、语句、参数和分支等代码元素。前缀属性属于被修饰的元素；configuration 属于容纳它的定义，不属于紧随其后的字段或成员。

以 multi 规则解析的列表在每个元素前接受属性链，嵌套列表独立应用此规则；没有固定的属性前缀层数上限。每层仍修饰同一个紧随元素，而非整份列表。具体解析边界和实参示例见[源码准备、解析与 HIR](../compilation/syntax-and-hir.md)。

当前前端会为尚未求值的定义属性保留结构化 HIR 表达式、原始文本、源码范围、目标定义和词法 owner，并在 HIR 中按同一目标的应用顺序记录。文件模块、嵌套模块和 struct 内的 configuration 也保留配置键、完整值表达式和所属目标；configuration 不建立字段或子定义，同一目标的内部配置先于前缀属性记录。函数参数、struct 字段、enum variant（含子 enum）及 record variant 字段上的多层前缀属性同样保留；目标记录区分整个定义与成员自身的 HirId，成员属性不会误附着到其所属类型，载荷与声明顺序也不会因属性包装而丢失。表达式中的可解析名称按原词法作用域绑定，尚未解析的名称保留为待处理节点；这不会把 `attrs` 的拼写当成编译器特殊指令。该记录还不是经过类型检查或求值的 Object；通用属性求值完成前，仍阻止生成已检查 NIR。

属性表达式遵循[普通名称解析](../code-organization/scope-and-paths.md)。attrs 提供语言的属性构造接口，可以写完整路径或经 use 引入名字；属性位置不额外优先搜索 attrs。

属性值的结构与属性键的含义分开：生成一个具有某个键的 Object，不会自行赋予该键布局或代码变换能力。键的接受位置、值类型和作用由相应功能的契约规定。

### 多个前缀属性

同一个元素上的多个前缀属性从右到左应用；纵向排列时，离元素最近的一项先应用。后应用的对象覆盖同名键的先前值，其他键保留。

例如，外侧属性将 `some_attr` 设为 1、内侧设为 2，最终读取到的该键为 1。这是属性的应用顺序。

### Configuration 与属性读取

configuration 通过键名和编译时值给所属定义提供属性。结构体中的 `.packed true` 将 `packed` 的值编码为 `true`；它本身不是一个运行时数据字段。

模块主体与文件作用域也可以包含 configuration：文件中的配置附着到文件模块，目录 mod.fl 中的配置附着到目录模块。见[模块属性](../code-organization/modules-and-sources.md#模块的属性)。

同一目标同时具有 configuration 和前缀属性时，先应用 configuration，再按从内到外的顺序应用前缀属性。因此，前缀属性可以覆盖内部 configuration 提供的同名键。

`meta.attrs_of` 读取目标的属性 Object。读取某个键后如何解释其值，属于该键的使用者；通用属性机制不替代相应功能对值的检查。

与数据定义一起执行的 comptime 代码遵循 [normalization 与 instantiation](./normalization-and-instantiation.md) 的规则。属性计算、读取与其他编译时操作不能绕过已有的依赖和阶段约束。

## 常见示例

### 指定函数 ABI

```flurry
^attrs.abi(.c)
fn identity(value: u32) -> u32 {
    value
}
```

`attrs.abi(.c)` 提供函数的 C ABI 属性。函数声明和调用见[定义、调用与返回](../functions/calls-and-returns.md#函数-abi)。

### 直接提供元数据

以下 `some_attr` 是示意元数据键，不指定额外的运行时行为。

```flurry
^{ some_attr: true }
fn example() {}

comptime {
    let attributes = meta.attrs_of(example);
    let value = attributes.get(.some_attr, bool);
}
```

### 同名键的覆盖

```flurry
^{ some_attr: 1 }
^{ some_attr: 2 }
fn annotated() {}
```

`annotated` 的 `some_attr` 最终为 1：最近的属性先应用，外侧属性随后覆盖该键。

### 覆盖定义内部的配置

```flurry
^{ some_attr: 1 }
struct Configured {
    .some_attr 2,
    value: u32,
}
```

`Configured` 的 `some_attr` 最终为 1：configuration 先提供值 2，前缀属性再覆盖它。

### 在定义内部配置属性

```flurry
struct PackedPair {
    .packed true,
    first: u8,
    second: u16,
}
```

`.packed true` 修饰 `PackedPair`。`first`、`second` 才是其实例的数据字段；具体布局约束由 `packed` 的契约规定。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与区别 |
|---|---|
| Rust | 内置属性常用 meta-item 形式，其中的表达式须宏展开为规定的字面量形式；Flurry 的属性表达式计算为 Object。[Rust 属性](https://doc.rust-lang.org/reference/attributes.html#meta-item-attribute-syntax) |
| Zig | 无对等的通用 Object 属性附着机制；布局等需求使用 `packed struct` 等类型构造及编译时设施。Flurry 通过属性值把元数据交给使用者。[Zig 文档](https://ziglang.org/documentation/master/#packed-struct) |
| C11 | 无对等的通用属性对象；实际可使用编译器扩展属性表达特定需求，例如 GCC 的类型属性。Flurry 的元数据由编译时表达式产生。[GCC 属性](https://gcc.gnu.org/onlinedocs/gcc/Attributes.html) |
| C++ | `[[...]]` 属性的参数形式由各属性规定；Flurry 使用统一的 Object 结果承载键值，而非把每个键都定义为独立参数语法。[C++ 属性语法](https://eel.is/c++draft/dcl.attr.grammar) |

## 运行时实现设计引导

属性处理发生在编译时。运行时实现接收的是已经兑现相应属性契约的代码或数据表示。

可按以下接口检查生成结果：

1. 枚举实际影响目标程序的属性使用者，例如布局计算或分支生成。
2. 对每个使用者取得已经验证的结果，而不是在 runtime 重新计算属性表达式。
3. 按该使用者的契约生成布局、指令或数据；其代价与行为属于相应功能。
4. 仅供编译期工具消费的信息，不因附着到定义就自动成为实例字段。

不变量：属性元数据的存储不能与用户数据字段混淆。属性表达式在 comptime 中的执行，也不能被无意复制为每次运行时调用的副作用。

## 编译器实现引导

### 附着目标与值计算

1. 解析 `^expr` 时记录目标代码元素及表达式；解析 configuration 时记录所属定义及键值表达式。
2. 保留各前缀属性的嵌套次序，不能在收集时按键名排序。
3. 依编译时求值规则取得各属性 Object；结果类型不符或求值失败时报告编译错误。
4. 将属性对象交给合并操作，并将最终元数据提供给反射读取和属性使用者。
5. 使用者验证自己负责的键值契约，再生成后续分析结果。诊断同时指向属性来源和使用目标。

configuration 与普通 property 需要保留不同的语义目标：前者写入所属定义的元数据，后者是当前对象或调用中的命名项。

### 属性的合并

输入为 configuration 已提供的属性映射，以及按源码从外到内排列的前缀属性对象序列。以下伪代码只规定应用与覆盖，不额外规定表达式求值的调度顺序：

```text
merge_attributes(configuration, objects):
    result = copy_map(configuration)
    for object in reverse(objects):
        for (key, value) in object.entries:
            result[key] = value
    return result
```

后置条件：有前缀属性提供的键，其最终值来自源码中最外侧包含该键的对象；其他键保留 configuration 的值。若 configuration 与前缀属性合计有 K 个键值项，采用散列表时合并的期望时间为 O(K)，结果空间为不同键的数量；属性表达式自身的计算代价另计。

属性读取与布局等查询形成依赖时，应保留依赖关系和计算状态。出现循环或资源耗尽，诊断需指出相关属性计算，不能返回一个伪造的空 Object 使后续检查继续通过。
