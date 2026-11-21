# struct

## Intro

struct 把几个有名字的值组成一个值。字段名称决定构造输入对应到哪里，声明顺序决定初始化先后；这两件事都不直接规定物理布局。将初始化顺序固定为声明顺序，使副作用的执行时机可预测，同时允许编译器独立选择物理布局以优化对齐和缓存利用。

独立的 struct 声明具有独立类型身份，即使它们声明了相同字段。

## 语法

~~~text
struct_def -> struct id clauses? { (configuration | struct_field | definition | statement)* }
struct_field -> id: expr (= expr)?
struct_construction -> expr { property* }
property -> id: expr
~~~

实际字段和构造项用逗号分隔。以右大括号结束的定义或语句不再补分号。属性与子句沿用全局语法。

## 正式定义

### 构造完整性

所有构造项完全指名；不使用位置项或字段名称简写。未知字段与重复指定同一字段都是静态错误。

每个字段必须有唯一初始化来源：显式输入优先，否则使用声明默认表达式；两者都没有则拒绝，不自动填零或调用默认 trait。

按字段声明顺序处理初始化。每个字段先选择实际来源，再求值并按字段类型检查/适配。显式输入覆盖默认值时，默认表达式不执行。

默认表达式在每次实际需要的构造中求值一次，不因定义中只有一份文本就跨实例共享结果。纯编译时缓存仍遵循已有执行规则。

### 表示与使用

普通 struct 不由初始化顺序承诺物理字段顺序。表示由类型、目标与明确配置决定。字段结构信息与布局信息可分别查询。

构造过程中的资源清理与部分对象访问受初始化和资源契约约束；字段默认语法本身不授予提前使用 self 的权利。

### 紧凑布局：`attrs.packed.byte` 与 `attrs.packed.bit`

需要关闭编译器重排与填充时，应通过官方属性（见[官方属性键集中规范](../../comptime/official-attribute-keys.md)）显式选择**字节级**或**位级**紧凑布局，禁止使用裸键或定义体内 `.packed` 配置：

- `^attrs.packed.byte` 展开为 `{ __flurry_packed_mode: .byte }`；
- `^attrs.packed.bit` 展开为 `{ __flurry_packed_mode: .bit }`。

两者共用单一内部键 `__flurry_packed_mode`（`Symbol` 域，`.byte` | `.bit`），因此外层前缀属性会自动覆盖内层同键设置，两者互斥。`__flurry_packed_mode` 仅适用于 `struct` 定义。

#### 1. `attrs.packed.byte`（字节级紧凑布局）

- **物理布局**：按字段声明顺序连续排列，消除字段间及尾部填充：`offset(f_0) = 0`，`offset(f_i) = sum_{j < i} size_of(f_j)`，总大小 `size = sum_i size_of(f_i)`，对齐 `align = 1`。
- **允许的字段类型**：支持任意合法 runtime 字段类型（标量、指针、函数指针、闭包、`str`、`Symbol`、普通 struct、嵌套 `packed.byte`/`packed.bit` struct、`enum`、`Once`/`Drop` 资源以及运行时依赖类型）。
- **字段指针限制**：禁止取得字段的对齐指针（`*FieldType`，如 `s.field.ref` 或 `p.field` 直接取址），允许持有整体结构指针 `*S`；字段读写（`s.f`、`p.f`、`s.f = v`、`p.f.* = v`）按未对齐（`align 1`）访问执行，降码为紧致聚合（`LLVM <{ ... }>` + `align 1`）。
- 不受下述 256 位宽度警告限制。

#### 2. `attrs.packed.bit`（位级紧凑布局，直接翻译为 `BitVec<W>`）

`attrs.packed.bit` 直接按位紧密打包，并在语义与中端直接翻译为 `BitVec<W>`（见 [BitVec 代数与程序合成](../../program-synthesis/bitvec-algebra.md)），从而完整复用 `BitVec` 切片/拼接代数与 `bitvec_opt` 优化及证明管线：

- **位序与偏移**：按字段声明顺序 **LSB-first（最低有效位优先）** 连续排布，与 `BitVec` 下标（索引 `0` 为最低有效位）一致：
  - `bit_offset(f_0) = 0`
  - `bit_offset(f_i) = sum_{j < i} bit_width(f_j)`
  - 逻辑总位宽 `W = sum_i bit_width(f_i)`
  - 字段 `f_i` 占据半开位段 `[bit_offset(f_i) .. bit_offset(f_i) + bit_width(f_i))`。在最高位在左的 `BitVec` 拼接与位模式语法（`hi ++ lo`、`0b...`）中，先声明的字段位于低位，即 `s'bitvec == f_{k-1}'bitvec ++ ... ++ f_0'bitvec`。
- **允许的字段类型（仅限纯位可表示 POD 类型）**：
  - `bool`：**1 位**（`false = 0b0`，`true = 0b1`）；
  - 定宽整数（`u8`..`u128`、`i8`..`i128` 二补码、`usize`/`isize` = 目标指针位宽，以及任意定宽 `uN`/`iN` = `N` 位）；
  - `BitVec<N>`：`N` 位（含 `N = 0`）；
  - IEEE-754 浮点（`f16`/`bf16` = 16 位、`f32` = 32 位、`f64` = 64 位）；
  - `char`：32 位 Unicode 标量值编码（构造与写回须维持标量值合法性不变量）；
  - 定宽位编码 `enum`（固定比特宽度的离散/位模式枚举，不含非位紧凑载荷或 `Drop`）：其精确位宽 `width`；
  - `newtype`：仅当其直接表示属于允许的位可表示类型且无 `Drop` 时可用（保持原位宽并维持其不变量）；
  - 嵌套的 `attrs.packed.bit` `struct`：直接按其精确逻辑位宽 `W_inner` 嵌入，不在字段边界补齐字节；
  - `Void` 或空 `attrs.packed.bit` struct：`0` 位（`W = 0`，等价于 `BitVec<0>`，`size = 0`、`align = 1`）。
- **拒绝的字段类型（静态错误）**：所有指针（`*T`、函数指针、闭包）、`str`、`Symbol`、纯编译时类型、普通 `struct`、`attrs.packed.byte` `struct`、一般 `Tuple`、一般 `union`、非位编码的 Tagged-Union `enum`、`?T`、`!E T`、`#E T`、运行时依赖类型、携带 `Drop` 或非位 POD 资源契约的类型，以及按值直接/间接递归环。
- **独立存储与嵌套宽度**：作为独立值/字段（包括嵌入 `attrs.packed.byte` struct）存放时，其存储布局直接等于 `BitVec<W>` 的目标布局（`size_of(S) == size_of(BitVec<W>)`，`align_of(S) == align_of(BitVec<W>)`）；仅当嵌套进另一个 `attrs.packed.bit` struct 时才按精确逻辑位宽 `W` 无填充拼接。
- **字段访问与指针禁令**：严禁对 `attrs.packed.bit` struct 的任意字段取任何指针（即使该字段偶然落在字节边界上也不允许）；允许持有整体结构指针 `*S`。
- **构造、读写与 `bitvec_opt` 共享管线**：
  - **构造**：严格按字段声明顺序依次求值各字段输入/默认表达式，随后按声明位序执行 `BitConcat` 组装为 `BitVec<W>`；
  - **读取**：对底层 `BitVec<W>` 执行定长提取 `bits[lo..hi]` 并解码回字段类型；
  - **写入与复合赋值**：对底层 `BitVec<W>` 执行非原子、非 volatile 的读-改-写 `bits[lo..hi] = encode(rhs)`（遵守 [BitVec 局部写回语义](../../program-synthesis/bitvec-algebra.md#局部写回)）；
  - **整体位视图与转换**：`s'bitvec` 直接产出完整的 `BitVec<W>`（无未定义填充位）；`bits.as(S)` 遵守 `BitVec` 转换规则（当任一字段仅承认受限编码如 `char` 或稀疏定宽 `enum` 时需要 `unsafe` 与对应前提）。由于上述操作均直接翻译为 `BitVec<W>` 代数节点（`Concat`、`Extract`、`Update`、`And`/`Or`/`Not`），`attrs.packed.bit` 自动共享 `bitvec_opt` 的代数重写、SAT 等价证明、CEGIS 目标合成与 `O(ceil(W/W_target))` 基准降码，不再维护第二套位域优化管线。
- **超过 256 位发出非致命编译警告（`W > 256`）**：
  - 当任意具体 `attrs.packed.bit` struct 的逻辑总位宽 `W > 256` 位（即 `W >= 257`）时，编译器**发出非致命警告**（`Level::Warning`），说明结构体名称、实际总位宽 `W` 及 256 位阈值，并提示若非严格按位打包可改用 `attrs.packed.byte` 或拆分为更小的位结构；
  - `W <= 256` 不发出该警告；`W > 256` 仅触发警告而不阻断编译，仍按 `BitVec<W>` 基准多字操作正常降码。对泛型 `attrs.packed.bit` struct，该警告按具体单态实例检查（同一具体实例去重）。

#### 3. 当前实现现状

- `library/attrs` 已提供 `^attrs.packed.byte` 与 `^attrs.packed.bit`（生成 `{ __flurry_packed_mode: .byte | .bit }`），`library/meta` 已提供 `Type.packed_mode() -> ?Symbol` 与 `Type.is_packed() -> bool`。
- **已接通的字节级紧凑链路**：编译器当前通过过渡键 `attrs.layout.packed`（`{ packed: bool }`）完整支持字节级紧凑布局检查、禁用字段对齐指针及 LLVM `<{ ... }>` (`align 1`) 代码生成。
- **待接通的编译器工作**：`elaboration` / `nir` 消费 `__flurry_packed_mode`（`PackedMode` 替换 `packed: bool`）、`attrs.packed.bit` 的字段可位表示校验与到 `BitVec<W>` / `bitvec_opt` 的直接降码，以及超过 256 位时的 `Level::Warning` 诊断通道。

## 常见示例

下面用一个会打印字段名的普通函数，让初始化顺序可以直接观察：

~~~flurry
fn supplied_width() -> u32 {
    println("width");
    640
}

fn default_height() -> u32 {
    println("default height");
    480
}

fn supplied_height() -> u32 {
    println("explicit height");
    720
}

struct Window {
    width: u32,
    height: u32 = default_height(),

    fn area(self) -> u32 {
        self.width * self.height
    }
}

test {
    let first = Window { width: supplied_width() }
    let second = Window {
        height: supplied_height(),
        width: supplied_width(),
    }
    let third = Window { width: 320 }

    println("areas: {}, {}, {}", first.area(), second.area(), third.area());
}
~~~

输出依次为：

~~~text
width
default height
width
explicit height
default height
areas: 307200, 460800, 153600
~~~

构造 `second` 时，虽然输入先写 `height`，仍先执行声明在前的 `width`。它的默认高度函数不执行。构造 `third` 又执行一次默认表达式，说明默认值不是在类型定义时只求一次。

以下是分别应被拒绝的构造；它们不是上面可运行示例的一部分：

~~~flurry
let missing = Window {}
let duplicate = Window { width: 1, width: 2 }
let unknown = Window { width: 1, depth: 2 }
~~~

三者分别缺少必填字段、重复提供同一字段、提供不存在的字段。

### 紧凑布局示意（设计示例）

> 以下示例展示 `attrs.packed.byte` 与 `attrs.packed.bit` 的目标设计语义；编译器当前仍在将下游布局消费者从过渡键 `.packed` 迁至 `__flurry_packed_mode` 与 `BitVec<W>`/`bitvec_opt` 降码。

~~~flurry
^attrs.packed.byte
struct WireHeader {
    tag: u8,
    payload_len: u64,
}

^attrs.packed.bit
struct FlagsByte {
    ready: bool,         -- bit 0
    priority: u3,        -- bits 1..4
    channel: BitVec<4>,  -- bits 4..8
}
-- FlagsByte 总位宽 W = 8，等价于 BitVec<8>（priority 为 1..4、channel 为 4..8）；
-- 若将 > 256 位的 POD 结构体标注为 ^attrs.packed.bit，编译器将发出非致命警告。
~~~

## 与 Rust / Zig / C / C++ 的关键差异

Rust 允许字段简写，Flurry 要求完整指名。C++ 聚合初始化有自己的顺序及 designated initializer 限制；不能把这些语法限制搬入 Flurry。C 的省略初始化规则也不能作为 Flurry 补零依据；Zig 有声明默认字段值。[Rust struct 表达式](https://doc.rust-lang.org/reference/expressions/struct-expr.html)、[C++ 聚合初始化](https://eel.is/c++draft/dcl.init.aggr)

## 运行时实现设计引导

建立初始化计划，按声明索引而非输入文本顺序执行。每一步记录字段、输入表达式、转换及已初始化状态，供资源契约使用。

物理布局可与初始化计划分离。优化不能重排可观察副作用，也不能执行被覆盖的默认表达式。

## 编译器实现引导

~~~text
plan_constructor(type, supplied):
    index := declared_fields_by_name(type)
    reject_unknown_and_duplicate_names(supplied, index)
    plan := []
    for field in declaration_order(type):
        source := explicit_input(field) or declared_default(field)
        if source absent: reject_missing_field(field)
        plan.append(check_and_adapt(source, field.type))
    return plan
~~~

这是内部伪代码，or 表示选择初始化来源，不是先求值两个候选。

字段索引构建 O(n)，散列表输入对应均摊 O(n+m)，求值和类型检查另计。完整性检查不应因某个输入尚未求值就跳过重复标签诊断。阶段延迟保留具体字段义务，不能交付缺字段实例。
