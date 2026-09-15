# 固定位宽 enum：从指令编码到判别条件

## Intro

一个编码可以同时保存变体身份和载荷。指令中的固定功能位说明是哪种操作，寄存器编号则作为载荷；不必在原编码旁边再添加一个 tag。

本例使用 RISC-V 的 ADD 与 SUB，展示整体宽度、互斥性、覆盖与载荷取得如何分别检查。重点是把已经明确的编码要求变成可验证条件。

## 语法

~~~text
enum_variant -> id: pattern
    | id { struct_field* }
configuration -> .id expr
~~~

这是与表示有关的局部语法，完整声明见[enum](enums.md)。位段采用[位向量模式](../../control-flow/pattern-matching/bitvec.md)的写法；本例在一个 32 位指令数值内匹配，不涉及跨元素解析。

## 正式定义

### 外部编码

从指令数值的最低有效位编号为 0。两条指令的格式如下：

| 位位置 | 宽度 | ADD | SUB |
|---|---:|---|---|
| 31..25 | 7 | 0000000 | 0100000 |
| 24..20 | 5 | rs2 | rs2 |
| 19..15 | 5 | rs1 | rs1 |
| 14..12 | 3 | 000 | 000 |
| 11..7 | 5 | rd | rd |
| 6..0 | 7 | 0110011 | 0110011 |

这是指令编码事实，来自 [RISC-V 指令表](https://docs.riscv.org/reference/isa/unpriv/rv-32-64g.html)。位宽合计 7+5+5+3+5+7=32；本例不把指令字节序、寄存器运行结果或内存装载方式并入 Flurry 类型规则。

### 在类型中解释

以 u32 为 base type，整个表示为 32 位。固定部分参与判别，三个 5 位寄存器编号为载荷。enum 的 width 若同时给出，不能要求在 base type 外额外增加判别宽度。

ADD 与 SUB 的共同固定位在 bit 30 上要求相反，因此互斥。互斥只证明一个编码不会同时归入这两个变体，不证明它们覆盖全部 u32，也不证明覆盖完整指令集。

普通 u32 值做位模式匹配时仍须取得 BitVec；enum 表示模式使用自己的 base type 豁免。两种使用位置不能因采用同一掩码算法而混淆。

## 常见示例

先把编码表写成类型声明。每个模式从高位到低位列出指令的六个部分：

~~~flurry
enum ArithmeticInstruction {
    .base_type u32,
    .discrete_tag true,

    add: 0b0000000 (rs2: u5) (rs1: u5) 000 (rd: u5) 0110011,
    sub: 0b0100000 (rs2: u5) (rs1: u5) 000 (rd: u5) 0110011,
}

comptime {
    assumes Type.size_of(ArithmeticInstruction) == Type.size_of(u32);
}
~~~

`rs2`、`rs1`、`rd` 各占 5 位，三个固定段分别占 7、3、7 位，合计 32 位。大小检查把“整个表示放在一个 u32 内”写成可检验要求。ADD 与 SUB 的差别直接写在第一个固定段中；三个载荷槽的取值不影响这两个变体之间的判别。

接下来写一个接收原始指令字的诊断函数。它在普通 `u32` 的位视图上使用同样的模式，并把匹配得到的寄存器编号用于输出：

~~~flurry
fn describe_instruction(word: u32) {
    word'bitvec match {
        0b0000000 (rs2: u5) (rs1: u5) 000 (rd: u5) 0110011 => {
            println("ADD x{}, x{}, x{}",
                rd.as(u32), rs1.as(u32), rs2.as(u32));
        },
        0b0100000 (rs2: u5) (rs1: u5) 000 (rd: u5) 0110011 => {
            println("SUB x{}, x{}, x{}",
                rd.as(u32), rs1.as(u32), rs2.as(u32));
        },
        _ => println("not an ADD/SUB encoding"),
    }
}

test {
    describe_instruction(0x003100b3);
    describe_instruction(0x403100b3);
    describe_instruction(0x00000000);
}
~~~

输出为：

~~~text
ADD x1, x2, x3
SUB x1, x2, x3
not an ADD/SUB encoding
~~~

编码中的载荷顺序是 `rs2`、`rs1`、`rd`，输出中的操作数顺序则是 `rd`、`rs1`、`rs2`。绑定名称使这次重排明确可见，无需在打印处重新计算位偏移。

这里有两个不同的输入域：声明中的模式约束 enum 的表示；诊断函数接收任意 `u32`，所以显式取得 `'bitvec` 并保留兜底分支。零值就是未覆盖的见证。函数没有把任意指令字直接转换成 `ArithmeticInstruction`。

修改模式也能立即得到检查边界。例如把 SUB 的首段改成 `0000000`，两个变体就接纳相同编码，无法证明互斥；把任一 `u5` 改成 `u6`，模式总宽度变为 33，与 `u32` 的表示冲突。

若另外增加 guard，掩码互斥仍能证明完整模式互斥；掩码兼容则不足以证明带 guard 的交集非空。

## 与 Rust / Zig / C / C++ 的关键差异

C/C++ 可用掩码、移位和位域描述编码；Rust 可用整数操作及模式判断，Zig 可用 packed 表示与重解释。这些是实际替代手段，但位域布局本身不等于 Flurry 的模式判别与载荷绑定。更完整对照见[BitVec](../../control-flow/pattern-matching/bitvec.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

降码可以先检查共同 opcode 与功能位，再以 bit 30 区分两个变体，最后按需要提取寄存器槽。优化前后必须对所有 base type 编码保持相同接纳集合。

进入变体后只提取实际使用载荷；不为识别操作而强制读取所有逻辑字段。构造采用同一固定位与载荷位置，保证构造结果能被对应判别条件接受。

## 编译器实现引导

本例的固定位可以内部表示为：

~~~text
M     = 0xfe00707f
V_ADD = 0x00000033
V_SUB = 0x40000033

matches_add(word) iff (word AND M) = V_ADD
matches_sub(word) iff (word AND M) = V_SUB
~~~

这些是内部数学记号。三个寄存器槽不在 M 的固定部分中。

1. 确认整体位宽和 base type。
2. 将每个定宽模式编为固定位条件及载荷位置表。
3. 检查所有片段宽度与覆盖位置合法。
4. 计算共同固定位冲突，证明 ADD/SUB 互斥。
5. 单独进行覆盖查询；本例存在全不匹配的 u32 见证。
6. 为构造和判别使用同一编码描述。
7. 优化后比较接纳条件，保持合法域和载荷提取一致。

数学互斥检查：

~~~text
(V_ADD XOR V_SUB) AND (M AND M) = 0x40000000
~~~

结果非零，表示存在共同固定位冲突。这一证据不需要枚举全部 2³² 个编码。

此例可作为 BV 快路径、enum 整体宽度、构造/判别一致性和反例诊断的共同验收。一般 guard 或未知宽度仍遵循 PTC 的残余规则。
