# 位向量模式

## Intro

位向量模式按固定进制把一段连续比特拆成若干段：数字字面段、comptime 段，或「子模式 : 宽度类型」。它只匹配 `BitVec`。跨元素匹配允许一个逻辑值占用多个 base 元素；未启用时，pattern 总宽度必须等于单个 base 的宽度。`BitVec` 上的连接、按位运算等不属于本章。

## 语法

```text
bit_vec_bin_pattern -> 0b segment+
bit_vec_oct_pattern -> 0o segment+
bit_vec_hex_pattern -> 0x segment+

segment -> id
         | integer
         | <expr>
         | (pattern : expr)
```

`pattern`、`expr` 同全局语法。类型位置写 `expr`。`0b` / `0o` / `0x` 选定进制；其后为一段或多段 `segment` 并置。

## 正式定义

普通值匹配以 BitVec 为主语；enum 表示模式的 base type 豁免及整体宽度规则见[enum](../../type/user-defined-types/enums.md)。该豁免不使普通整数值自动取得位视图。

### 适用对象

普通值匹配的被匹配值必须是 `BitVec`，否则为**静态错误**。enum 表示模式使用其独立的 base type 规则。

### 段

1. **字面段**：按当前进制书写的固定比特约束（例如 `10000010`、`10`）。
2. **`<expr>`**：编译期求出的固定段约束；须能确定本段宽度与比较值。
3. **`(pattern : expr)`**：按类型表达式给出的宽度切出一段，再对切出的值匹配内层 `pattern`（常见为绑定名）。该类型须有确定大小。

整条 pattern 的总宽度为各段宽度之和。

### Base type 与宽度

参与此类布局的类型须通过**属性**声明 base type（属性语法见 [属性系统](../../comptime/attribute.md)；数据定义体内的配置写法亦可）。Base type 的宽度是对齐基准。

- 整段 pattern 必须与适用的对齐约束在宽度上对齐。
- **未启用跨元素**时：pattern 总宽度必须**等于** base type 宽度。
- **启用跨元素**时：一个逻辑值可跨越多个 base 元素；总宽度须与该逻辑布局对齐（仍由属性声明；见下节示例中的 `.dst`）。

### 与运算的边界

本章只定义 **pattern**。对 `BitVec` 值做连接、按位与/或/非、宽度转换等，见类型与运算相关文档。

## 常见示例

```flurry
test {
    let num: f32 = 13.625;
    if num'bitvec is 0b(sign: u1) 10000010(exponent: u8) 10110100000000000000000(mantissa: BitVec<23>) {
        println("sign: {}, exponent: {}, mantissa: {}", sign, exponent, mantissa);
    }

    if num'bitvec is do {
        0b(sign: u1) 11111111(exponent: u8) (mantissa: BitVec<23>) if mantissa != 0 => println("NaN"),
        0b(sign: u1) 11111111(exponent: u8) 00000000000000000000000(mantissa: BitVec<23>) => println("Infinity"),
        _ => (),
    }
}
```

跨元素（base / DST 等用配置属性声明；变体右侧为 bitvec pattern）：

```flurry
enum Utf8CodePoint {
    .base_type u8,
    .discrete_tag true,
    .dst true,

    one: 0b0(ascii_char: BitVec<7>),
    two: 0b110(ys: BitVec<5>) 10(zs: BitVec<6>),
    three: 0b1110(xs: BitVec<4>) 10(ys: BitVec<6>) 10(zs: BitVec<6>),
    four: 0b11110(ws: BitVec<3>) 10(xs: BitVec<6>) 10(ys: BitVec<6>) 10(zs: BitVec<6>),
}
```


## 与 Rust / Zig / C / C++ 的关键差异

C / C++ / Rust / Zig **都没有**与「`0b`/`0o`/`0x` + 多段并置、按宽度切开并绑定」对等的一等模式语法。常见做法是：

| 语言侧常见手段 | 与 Flurry 的对照 |
|----------------|------------------|
| C/C++：结构体位域，或手写移位 / 掩码 | 位域是字段布局，不是 `match`/`is` 上的模式；掩码散落在表达式里 |
| Rust：对整数 `match`、或 crate 级 bitfield | 标准匹配不按比特段切开绑定；位操作仍多是掩码 |
| Zig：`packed struct`、`@bitCast` 等 | 按字段/重解释取值，不是进制前缀的分段 pattern |

Flurry 把分段约束写在 **pattern** 里（字面段、`<expr>`、`(pattern : expr)`），并在属性声明的 base / 跨元素布局上做宽度对齐检查。

## 运行时实现设计引导

1. **输入**：`BitVec` 值、分段 pattern、由属性得到的 base 宽度与是否跨元素。
2. **静态前置**：总宽度相对对齐约束合法；每段宽度可确定；普通值主语为 `BitVec`，enum 表示模式满足其 base type 规则。不满足则不得生成匹配代码。
3. **匹配步骤**：按文档锁定的比特序（须与类型布局一致）依次切段；字面/`<expr>` 段做相等比较；`(pattern : expr)` 段将切片交给内层模式。
4. **跨元素**：按 base 元素序列构成逻辑比特流后再切，或生成等价的多元素决策树；语义以逻辑比特布局为准。
5. **失败**：宽度不对齐、普通值主语非 `BitVec`、段宽无法确定 → 编译期拒绝。

## 编译器实现引导

1. **字面段词法**：语法上为 `id` 或 `integer`；分析阶段溯源回源码字形，再按 `0b` / `0o` / `0x` 进制解释为固定比特约束。
2. **解析**：`0b` / `0o` / `0x` 后接 `segment+`；`<expr>` 与 `(pattern : expr)` 进入既有表达式/模式管线。
3. **属性**：读取 base type 与跨元素相关属性（配置写法或 `^` 属性）；缺失 base type 或宽度冲突 → 诊断。
4. **类型与宽度**：普通值主语必须是 `BitVec`，enum 表示模式按其 base type 契约检查；对 `(pattern : expr)` 检查类型有确定大小并可得到宽度；对各段宽度求和并与对齐规则比较。
5. **降码**：生成可超优化的比较 / 掩码 / 决策树；不把具体优化算法升格为语义定义。
6. **与枚举 / DST 布局**：跨元素匹配依赖布局属性；布局属性的完整集合在数据/枚举文档定义，本章只消费「base 宽度」与「是否跨元素」两类事实。
