# BitVec 代数与程序合成

## Intro

`BitVec<N>` 表示 N 个比特。程序可以切出一段、连接几段，或逐位组合两个值，而不必在每一步手算整数掩码与偏移。

这些操作描述输入比特怎样形成输出比特。编译器保留这种关系，先生成正确实现，再搜索更适合目标的实现。搜索是否成功，不改变程序的含义。

## 语法

```text
bitvec_type -> BitVec<expr>
take_view -> expr'bitvec
bitvec_operation -> expr ++ expr
    | expr and expr
    | expr or expr
    | not expr
    | expr == expr
    | expr != expr
index_application -> expr[expr]
range -> expr .. expr
type_cast -> expr.as(expr)
assignment -> expr = expr
```

以上复用[全局语法](../grammar/syntax)。`BitVec<N>` 的 N 是 comptime 宽度；下标参数可以是单个索引，也可以是半开范围。位置是否合法、表达式属于哪种操作，由类型与上下文决定。

比特段模式另见[位向量模式](../control-flow/pattern-matching/bitvec.md)。本章不增加 xor、移位、旋转或 bitcast 关键字。

## 正式定义

### 值与阶段

`BitVec<N>` 的值由 N 个已定义比特组成，N 是非负整数。定义检查时 N 可以仍为受约束参数，具体实例交付时必须确定；内容可以来自 runtime。

位向量没有自带的有符号解释。它可以作为普通参数、结果和字段，并具备 Copy 能力。复制只产生相同比特的值，不复制源对象的拥有责任、访问资格或运行时环境。

编译器知道表达式结构，不意味着知道表达式未来的输入。位向量值、编译器保存的映射结构和 quoted code 分别遵守各自的阶段规则。

`BitVec<0>` 只有一个空值。空值是拼接的单位元；对空值取反仍为空，同宽空值比较相等。零宽度没有合法的单比特索引。

### 宽度、顺序与表示

索引 0 表示最低有效位；索引增大时向高位移动。`a ++ b` 把 a 放在高端、b 放在低端；位模式文本从高位写到低位。

例如，8 位的 0xAF 与 8 位的 0x3C 拼接为 16 位的 0xAF3C。已经取得的位向量运算不随宿主字节序改变；读取和写入存储表示时，再按目标的布局与字节序完成映射。

位宽由被观察的位表示决定。例如 u5 的位表示是 5 位，不因单独存储可能占用一个字节就变成 8 位。聚合的字段位置与填充由其表示契约规定，不能按声明次序猜测，也不能无条件以存储字节数代替有效位表示宽度。布局观察见[类型反射与布局](../type/reflection-and-layout.md)。

### 字面量与运算

有明确 `BitVec<N>` 目标时，整数字面量必须满足 0≤value<2ᴺ；超出范围为静态错误。前导零和进制字形不覆盖目标宽度，也不授权截断。N=0 时，整数 0 对应唯一空值。

| 操作 | 输入要求 | 结果 |
|---|---|---|
| `a ++ b` | 宽度分别为 m、n | BitVec<m+n> |
| `a and b`、`a or b` | 两个输入同宽 N | 逐位组合，BitVec<N> |
| `not a` | 宽度 N | 只翻转这 N 位，BitVec<N> |
| `a == b`、`a != b` | 两个输入同宽 | bool |
| `bits[i]` | 0≤i<N | BitVec<1> |
| `bits[lo..hi]` | 0≤lo≤hi≤N，hi−lo 在 comptime 可确定 | BitVec<hi−lo> |

`and`、`or` 在 BitVec 上按普通操作数规则求值两侧，各一次。它们不按 bool 的短路规则跳过另一侧。结果恒零或恒一，也不能据此删除输入表达式的可观察行为。

索引、切片起点可以来自 runtime，结果宽度仍须在 comptime 确定。移动起点取固定四位与取任意运行时长度是不同的接口；后者不能以未完成的 comptime 宽度交付这个类型。

边界条件沿用[契约与验证](../resource-management/contracts-and-verification.md)：已知非法时拒绝；静态未知时需要相应契约或合法的受检依据。计算范围端点的机器运算也必须先满足其前提，不能先溢出再检查。空切片同样要求端点合法。

异或可由 and、or、not 组合；固定移位和旋转可用切片与拼接表达。数值算术及有符号比较使用相应数值解释，不因内部求解器支持某种运算就自动成为 BitVec 的源操作。

### 值读取与位置写入

读取 `value'bitvec` 得到该次读取的位值。保存它以后，源对象的修改不会使这个值随之变化。编译器可以消除实际复制，但必须保持这一可观察结果。

对合法可写位置直接取位视图并选择位段，可以更新原位置。赋值的右侧必须与被更新位段同宽，字面量接受该位段提供的目标宽度。已经读出的 BitVec 值不会因曾经来自某个对象而继续指向该对象。

更新仍要求原位置可访问、相应状态已初始化、旧责任得到处理，并保持后续使用所需的合法表示。相同物理位置的重叠赋值先取得右侧值，再提交更新；不能一边覆盖一边从已经改变的右侧位置继续读取。

局部位更新不自动提供 atomic 或 volatile 行为。实际读改写可能覆盖比逻辑位段更大的存储单元；两个逻辑位段不相交，不能单独证明它们可以并发写入。

### 转换

| 目标 | 条件 | 行为 |
|---|---|---|
| BitVec<M> | M≥N | 高端补零，原 N 位保持 |
| 无符号整数 uM | M≥N | 高端补零，按无符号编码交付 |
| 其他 T | 与 T 的规定表示位宽一致 | 按该表示重解释，不做数值舍入或数值转换 |

向更窄 BitVec 或无符号整数直接 `.as(T)` 为静态错误，即使当前值恰好放得下。丢弃哪些位必须通过显式切片表达，再转换为所需目标。

重解释的一般入口要求 unsafe。若目标契约已保证所有相应位型合法，而且没有额外构造责任，可以安全交付。一般 T 的 unsafe 位转换仍须维护其合法编码、不变量、环境和资源条件；同宽只是其中一个要求。

### 已定义表示与资源

普通 BitVec 的每一位都有确定值。安全读取要求实际观察的位已初始化、访问合法。含未定义填充的对象不能直接整体读成普通 BitVec；可以观察已经定义的字段，或由底层接口在满足初始化契约后交付完整表示。取视图不会默默把填充补零，unsafe 也不使无效读取成为定义值。

地址比特相同不证明目标仍然有效；拥有者的表示可以被观察，不意味着能安全地重建两个独立拥有者。类型需要的运行时依赖环境也不能从裸位串凭空取得。移动、初始化与清理继续遵守[资源契约](../resource-management/moves-and-initialization.md)。

### 合成与失败

每个合法映射都有基准降码。编译器可以搜索等价实现，但只有正确性依据成立时才替换。预算耗尽、求解器 Unknown 或没有更优候选时，保留已有合法实现。

优化等级和目标代价模型可以影响选择，不能改变结果、输入求值、失败路径和资源责任。有限无环映射是合成区域的形态，不要求用户整个程序无循环，也不承诺全程序最优。

## 常见示例

### 连接字节与交换位置

```flurry
use std.prelude.*;

pure fn join_bytes(high: u8, low: u8) -> u16 {
    (high'bitvec ++ low'bitvec).as(u16)
}

pure fn swap_bytes(word: u16) -> u16 {
    let bits = word'bitvec;
    (bits[0..8] ++ bits[8..16]).as(u16)
}

test {
    println("{}", join_bytes(0xAF, 0x3C));
    println("{}", swap_bytes(0x1234));
}
```

输出 44860、13330，分别是 0xAF3C、0x3412。第二个函数把原低字节移到结果高端；位序不需要借助机器字节序推断。

### 字段组装与分段读取

这里定义一个教学用的 16 位头：高三位保存 kind，接着五位保存 flags，低八位保存 length。

```flurry
use std.prelude.*;

pure fn pack_header(kind: u3, flags: u5, length: u8) -> u16 {
    (kind'bitvec ++ flags'bitvec ++ length'bitvec).as(u16)
}

fn describe_header(word: u16) {
    word'bitvec match {
        0b(kind: u3) (flags: u5) (length: u8) => {
            println("{} {} {}",
                kind'bitvec.as(u8), flags'bitvec.as(u8), length);
        },
    }
}

test {
    let word = pack_header(5, 17, 42);
    println("{}", word);
    describe_header(word);
}
```

输出 45354 和 `5 17 42`。三个字段宽度合计 16，构造与解构使用同一顺序；模式没有额外固定位，因此覆盖所有 u16 编码。

### 用基础代数组合异或

```flurry
use std.prelude.*;

pure fn different_bits(left: BitVec<8>, right: BitVec<8>) -> BitVec<8> {
    (left and (not right)) or ((not left) and right)
}

test {
    println("{}", different_bits(0xAF, 0x3C).as(u8));
}
```

输出 147，即 0x93。not 只翻转操作数的八位；编译器可以识别等价目标操作，不要求程序员写目标指令。

### 动态起点，固定结果宽度

```flurry
use std.prelude.*;

fn window(word: u16, start: usize) -> BitVec<4> {
    assumes start <= 12usize;
    word'bitvec[start..(start + 4usize)]
}

test {
    println("{}", window(0xABCD, 4usize).as(u8));
}
```

输出 12。检查在计算上界和读取之前；结果始终为四位。`assumes` 的成功条件足以保证端点在 16 位以内，也保证这里的加法不会溢出。

### 写回与已经读出的值

```flurry
use std.prelude.*;

test {
    var word: u16 = 0x1234;
    let snapshot = word'bitvec;
    word'bitvec[0..8] = 0xAB;
    println("{} {}", word, snapshot.as(u16));
}
```

输出 `4779 4660`。word 变为 0x12AB，snapshot 保持 0x1234。snapshot 是值，不是持续观察 word 的存储引用。

重叠更新也以已经取得的右侧值为准：

```flurry
use std.prelude.*;

test {
    var word: u16 = 0x1234;
    word'bitvec[4..12] = word'bitvec[0..8];
    println("{}", word);
}
```

输出 4932，即 0x1344。右侧先得到 0x34；后续写入不能污染这份输入。

### 扩宽载荷与显式丢位

```flurry
use std.prelude.*;

pure fn assemble_payload(high: BitVec<5>, low: BitVec<6>) -> u32 {
    (high ++ low).as(u32)
}

pure fn low_byte(word: u16) -> u8 {
    word'bitvec[0..8].as(u8)
}

test {
    println("{}", assemble_payload(0b10101, 0b101010));
    println("{}", low_byte(0x1234));
}
```

输出 1386、52。第一个函数在 11 位载荷的高端补 21 个零；第二个函数明确选择低八位，才转换为 u8。

### 空位串

```flurry
use std.prelude.*;

test {
    let empty: BitVec<0> = 0;
    let bits: BitVec<8> = 0xAF;
    println("{} {}", (empty ++ bits) == bits, (bits ++ empty) == bits);
    println("{}", bits[8..8] == (not empty));
}
```

输出 `true true` 和 `true`。空结果不需要读取第一个元素，然而形成操作数的行为和动态端点检查仍不能无条件删除。

### 结果恒零仍要执行输入

```flurry
use std.prelude.*;

fn produce_bits() -> BitVec<8> {
    println("produce");
    0xAF
}

test {
    let zero: BitVec<8> = 0;
    let result = zero and produce_bits();
    println("{}", result.as(u8));
}
```

依次输出 produce、0。位运算的结果可以直接使用零，produce_bits 的输出行为仍须执行一次。

### 同宽不等于任意编码合法

```flurry
use std.prelude.*;

enum ReadyFlag {
    .base_type u8,
    .discrete_tag true,

    idle: 0b00000000,
    ready: 0b00000001,
}

unsafe fn flag_from_bits(bits: BitVec<8>) -> ReadyFlag
where requires bits == 0 or bits == 1
{
    bits.as(ReadyFlag)
}

test {
    unsafe {
        let flag = flag_from_bits(1);
        flag match {
            .idle => println("idle"),
            .ready => println("ready"),
        }
    }
}
```

输出 ready。ReadyFlag 只有两个合法编码；八位的 2 与它同宽，却不满足构造前提。这里的 unsafe 边界承接位重解释责任，入口契约仍需成立。

### 拒绝例

以下片段分别展示不同的静态错误，不构成一个成功测试。

```flurry
test {
    let bits: BitVec<4> = 16;
}

fn mismatched(left: BitVec<4>, right: BitVec<8>) -> BitVec<8> {
    left and right
}

fn narrow(bits: BitVec<16>) -> u8 {
    bits.as(u8)
}

fn empty_index(bits: BitVec<0>) -> BitVec<1> {
    bits[0]
}
```

分别违反字面量范围、同宽操作、禁止直接窄化和索引边界。结果目标类型不能修复操作本身不合法的输入宽度。

## 与 Rust / Zig / C / C++ 的关键差异

| 语言中的实际手段 | 对照 |
|---|---|
| C：无对等的类型化位段拼接与写回视图；使用无符号整数掩码、移位或位域 | Flurry 直接保留片段宽度和映射关系；物理读改写仍有实际访问要求 |
| Rust：整数位运算及库封装 | Rust 的整数按位操作与短路布尔操作有不同运算符；Flurry 在 BitVec 上赋予 and/or 逐位、双侧求值的契约。[Rust 运算符](https://doc.rust-lang.org/reference/expressions/operator-expr.html) |
| Zig：任意位宽整数、packed 表示、`@bitCast` | 这些可表达位布局及重解释；Flurry 的切片、拼接和位置更新由 BitVec 代数统一描述，扩宽另有补零契约。[Zig 文档](https://ziglang.org/documentation/master/#bitCast) |
| C++：无对等的完整 BitVec 映射机制；整数位运算、`std::bitset` 与 `std::bit_cast` 分别承担部分用途 | `std::bit_cast` 有同大小及类型条件，不能仅因名字相近就等同于 Flurry 的扩宽或任意类型 unsafe 构造。[C++ bit_cast](https://eel.is/c++draft/bit.cast) |

这些手段也可以被优化。Flurry 的语义不以“其他语言不能生成相同指令”为前提。

## 运行时实现设计引导

### 基准表示与操作

选择足以保存 N 位的机器字或机器字数组，记录尾部有效位。物化布局由目标 ABI 决定；抽象运算不要求每个中间结果都分配存储。

1. 在表示边界按目标布局取得源比特，验证必要的访问与初始化要求。
2. 拼接把右输入安置在低位，左输入安置在其上方；跨机器字时按片段拼装。
3. 逐位运算仅作用于有效位；not 的尾部结果须清除不属于 N 位的实现填充。
4. 切片先完成边界检查，再按偏移提取；零宽度直接产生空值，不执行首元素访问。
5. 扩宽在高位填零；转换成其他类型时兑现其表示、环境与资源交付要求。

避免在宿主语言中生成移位量等于机器字宽的未定义操作。整字拼接、零宽度、单字内提取和跨字提取应分别处理，再由目标优化消除多余分支。

在机器字宽 W 下，物化 N 位的空间与逐字操作成本为 O(⌈N/W⌉)；固定的小宽度可以由有限条目标指令完成。这个基准不依赖搜索器找到优化。

### 局部写回

```text
输入：合法源位置 p、半开位段 [lo, hi)、已求出的右侧值 r
前提：右侧宽度 = hi - lo；边界合法；访问与原值责任已兑现

1. 保持右侧值 r，不能继续依赖将被覆盖的右侧存储。
2. 确定目标存储单元及保留位，建立实际读写足迹。
3. 按合法读改写计划替换选定位，保持其余位。
4. 核验重新作为原类型使用所需的表示与状态条件。
5. 提交状态变化，使依赖旧内容的事实按内存规则失效。
```

若计划需要读取尚未初始化的保留位，该计划本身不合法；必须采用已有依据支持的写入方式或诊断缺失前提。原子、volatile 和设备访问需要相应专用契约，不能沿用普通读改写并声称等价。

运行时代码只执行已经生成的实现与必需检查，不在每次计算 BitVec 时重新启动合成器。

## 编译器实现引导

### 类型与位置分开保留

AST lowering 将取视图直接形成专用 BitVecView HIR 节点，并保留操作数与源码位置。Elaboration 区分读值需求和赋值位置需求；类型同为某宽度的 BitVec，不代表两个表达式具有相同存储来源。

宽度约束、下标结果宽度与转换合法性参与类型检查；具体读取、边界前提、初始化、责任及写回状态交给程序检查。`hi−lo` 可静态确定，不意味着 hi 和 lo 本身必须是常量。

Normalization 交付规定的检查依据；实例化完成类型中的 comptime 宽度。IIR 可以继续保留拼接、提取和更新等高级操作，不要求先变成 SSA 或机器掩码。阶段边界见[IIR 与代码交付](../compilation/iir-and-codegen.md)。

### 提取有限映射

以下为内部表示示意，不是标准库名称：

```text
Node = Input(width, identity)
     | Constant(width, bits)
     | Concat(left, right)
     | Extract(input, start, width)
     | And(left, right) | Or(left, right) | Not(input)
     | Equal(left, right)
     | ZeroExtend(input, width)

Region = 输入值 + 定义域条件 + 输出节点 + 行为边界
```

进入该纯映射前，输入的读取与调用已经有明确语义位置；需要保留的行为留在边界中。写回把计算结果与目标位置契约接合，不作为可任意交换的纯节点。

对相同节点可共享结构，但不能仅凭表达式字形相同合并不同内存版本的输入。展开循环或跨函数扩大区域，需要已有的展开和纯度依据；有限映射不反向强迫普通调用内联。

### 基准降码、搜索与核验

```text
reference = 按节点定义产生基准实现
best = reference
在搜索预算内：
    candidate = 依据目标指令和已确认代数生成候选
    若静态宽度或目标条件不合法：丢弃 candidate
    proof = 检查合法输入域内的定义性与输出关系
    若 proof 成立且代价可接受：best = candidate
    若获得反例：拒绝该候选并保留反例
    若 Unknown 或预算耗尽：保留 best
交付 best 及其正确性依据
```

对已隔离的纯位映射，核验条件为：

```text
P(x) => defined(candidate, x) and candidate(x) = reference(x)
```

P 包含合法输入范围和可用条件。候选指令的移位范围、表示前提等必须进入 defined；只比较若干样本的输出不能替代全域依据。外部行为由区域边界另行保持，位值相等不证明一次调用或清理可以删除。

基于已核验恒等式的重写可以直接交付依据；一般候选可以编码为定宽电路并查询反例。若采用反例引导搜索，每次反例用于约束下一轮候选，预算仍限制整个过程。搜索空间、机器指令集和代价模型明确后才能讨论其中的最优性。[Souper](https://github.com/google/souper)提供了基于 SMT 的 IR 超优化实现参照。

### 零宽度、求解入口与缓存

外部固定宽度 BV 后端可以只接受正宽度。先消去空拼接、空提取和空比较，再编码剩余项；同时保留原程序必须执行的输入、检查和清理。SMT-LIB 的 extract 使用含末端的索引，本章的切片是半开区间，翻译时须显式换算；动态起点也不能假冒成 SMT 的静态提取参数。[SMT-LIB FixedSizeBitVectors](https://smt-lib.org/theories-FixedSizeBitVectors.shtml)

三个入口可以复用底层编码，但职责分别保存：

| 入口 | 负责的判断 |
|---|---|
| PTC 的 BV theory | 类型命题所需的定宽关系、表示模式集合关系 |
| 合成候选核验 | 选定区域中替代程序的定义性和等价性 |
| 程序规格验证 | 状态、内存、资源及用户契约 |

内部核验能力不扩大 safe 对程序规格的自动证明范围，也不要求普通 BitVec 操作进入 verified。PTC Unknown、优化候选未知和程序验证前提未兑现分别诊断，不能互相替代。

缓存键至少涵盖规范化映射、宽度、定义域、目标语义与指令特征、代价模型和检查器版本。只有这些依据一致时才能复用候选及其证明；一次失败搜索不证明更优实现不存在。

### 验收不变量

- 任意合法输入上，基准实现与节点定义一致；合成前后结果一致，候选不增加未定义执行。
- 空宽度、机器字边界、跨字拼接、尾部无效位与动态起点均有边界覆盖。
- 值快照不随源对象写入改变，重叠更新先保留完整右侧。
- 超时与 Unknown 保留合法代码；不同目标或不同定义域不误用缓存。
- 结果常量化不删除必须保留的调用、读取、失败或清理；受限编码不能借同宽转换绕过契约。
