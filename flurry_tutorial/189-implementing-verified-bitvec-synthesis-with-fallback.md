# Flurry 演进手记 (189)：`BitVec` 位级代数与安全边界全解——值快照、重叠写回、非短路求值与 `unsafe` 重解释

在[上一篇（第 188 篇）](./188-why-iir-preserves-high-level-structure-before-codegen.md)中，我们梳理了结构化循环、闭包与效应/错误双轨控制流的协同法则。

在系统编程、网络协议解析和硬件寄存器操作中，位域操作（Bit Manipulation）无处不在。通过 `BitVec<n>` 与 `'bitvec` 视图，我们可以像操作数组一样直观地书写位切片 `word'bitvec[0..8]`、位拼接 `hi ++ lo` 以及原地写回 `word'bitvec[4..12] = word'bitvec[0..8]`。

为了让位操作既拥有等同于手写移位掩码指令的性能，又具备严格的安全性，Flurry（见 `docs/program-synthesis/bitvec-algebra.md`）对 `BitVec` 确立了四条核心语义契约：
1. **值快照读取（Value Snapshot）与位置写回（Place Writeback）严格区分**；
2. **重叠位区间原地写回保证先读后写，绝不发生自我踩踏**；
3. **按位逻辑算子（`and`、`or`、`not`）对操作数完整求值，绝不短路跳过副作用**；
4. **向 `BitVec<m>` 和 `uM` 以外的目标类型 `T`（如定宽枚举或有符号类型）执行 `.as(T)` 重解释时，必须满足相等位宽且显式置于 `unsafe` 边界内**。

今天，我们就逐一通过代码实例掌握这四条关键规则。

---

## 一、 值快照 vs 重叠位置写回：杜绝边读边写的自我覆盖

在手写 C 语言位操作或内存拷贝时，一个经典的隐患是**源区间与目标区间重叠（Overlapping Bit Ranges）**。
而在 Flurry 中，`'bitvec` 视图根据它出现在右值还是赋值号左值位置，具有清晰的两阶段语义：

1. **值快照读取（Value Snapshot）**：
   当你写下 `let snapshot = word'bitvec;` 时，它立即提取 `word` 当前的 32 位纯值快照，而不是借用指针。即使下一行修改了 `word`，`snapshot` 的值也保持不变。
2. **位置写回（Place Writeback）**：
   当你对一个可修改的存储位置（`var` 变量或可写指针解引用 `ptr.*`）执行 `word'bitvec[4..12] = word'bitvec[0..8];` 时，语义严格保证**先完整求值右侧的 `word'bitvec[0..8]` 8 位快照，再原子性地更新左侧的 `[4..12]` 位段**。

```text
  [ 重叠位区间写回：word'bitvec[4..12] = word'bitvec[0..8] ]
    步骤 1：先从 word 读取 [0..8] 位的不可变值快照 rhs_snap (8 bits)
    步骤 2：清除 word 中 [4..12] 位的旧比特，将 rhs_snap 写入 [4..12] 位
    ═══▶ 哪怕源区间 [0..8] 与目标区间 [4..12] 在 [4..8] 重叠，也绝不发生自我覆盖。
```

来看代码验证：

```flurry
fn snapshot_vs_writeback(initial_word: u32) -> Tuple<BitVec<8>, u32> {
    var word: u32 = initial_word;

    -- 1. 值快照：立即拷贝一份 word 当前的 32 位纯值快照
    let snapshot: BitVec<32> = word'bitvec;

    -- 2. 重叠位置写回：先求值右侧快照 word'bitvec[0..8]，再写回左侧位区间 [4..12]
    word'bitvec[4..12] = word'bitvec[0..8];

    (snapshot[0..8], word)
}
```

---

## 二、 按位 `and` / `or` 的完整求值保证（不短路）

在布尔类型 `bool` 上，`false and side_effect()` 会短路跳过右侧求值。
但是，**在 `BitVec<n>` 上的 `and`、`or` 是全位宽按位算子（Bitwise Operators），不是短路控制流算子。**

```flurry
fn produce_bits(counter: *u32) -> BitVec<8> {
    counter.* += 1;
    0x5A
}

fn preserve_input_evaluation(counter: *u32) -> BitVec<8> {
    let zero: BitVec<8> = 0x00;
    -- 尽管在位代数上 `0x00 and X` 的结果位恒为 `0x00`，
    -- 但按位运算符依然严格按从左到右顺序完整求值两侧操作数，
    -- produce_bits(counter) 的调用与状态更新绝对不会被跳过。
    zero and produce_bits(counter)
}
```

无论编译器对纯位向量表达式做怎样的掩码与移位合并优化，所有输入操作数的求值顺序、求值次数与资源清理契约都 100% 保持不变。

---

## 三、 `.as(T)` 类型转换的安全边界与 `unsafe` 重解释

对 `bits: BitVec<n>` 调用 `bits.as(T)` 时，Flurry 按目标类型 `T` 划分为安全的扩宽转换与要求 `unsafe` 的等宽重解释：

1. **安全的无损转换（目标为 `BitVec<m>` 或 `uM`，且 $m \ge n$，无需 `unsafe`）**：
   - 将 `BitVec<n>` 转换为等宽或更宽的 `BitVec<m>` 或无符号整数 `uM`（如 `BitVec<4>.as(u8)`、`BitVec<8>.as(u8)`、`BitVec<14>.as(u16)`），语义为确定的高位补零扩展或位保持。
2. **其他类型 `T` 的等宽重解释构造（要求 `Type.bit_width_of(T) == n` 且必须使用 `unsafe`）**：
   - 当目标类型 `T` 是 `BitVec<m>` 与 `uM` 以外的其他类型（例如只定义了 `0` 和 `1` 两个合法标签的定宽枚举 `ReadyFlag`，或有符号类型等），并非任意 $n$ 比特都能无条件作为安全转换目标；
   - 因此，将 `BitVec<8>` 通过 `.as(ReadyFlag)` 重解释为 `ReadyFlag` **必须置于 `unsafe` 块或 `unsafe fn` 内**，并由调用者（或前置契约 `requires`）保证位模式的有效性：

```flurry
enum ReadyFlag {
    .base_type u8,
    .discrete_tag true,

    idle:  0b0000_0000,
    ready: 0b0000_0001,
}

fn decode_flag(bits: BitVec<8>) -> ReadyFlag
where
    requires bits == 0x00 or bits == 0x01
{
    -- 具备前置契约证明 bits 只可能是 0x00 或 0x01，在窄 unsafe 块内执行重解释构造
    unsafe { bits.as(ReadyFlag) }
}
```

---

## 四、 拒绝例：不可写位置写回、隐式截断与缺少 `unsafe` 的重解释

让我们通过三个编译期拒绝例，巩固 `BitVec` 的安全防线。

### 拒绝例 1：试图对不可变 `let` 绑定执行位置写回

```flurry
fn write_to_immutable() -> u32 {
    let word: u32 = 0x1234_5678;
    -- ❌ 编译期拒绝：let 绑定 word 是不可变绑定，不是可写位置（var 或指针解引用），
    -- 不能对其执行 'bitvec 位置切片写回。
    word'bitvec[0..8] = 0xAA;
    word
}
```

### 拒绝例 2：试图用 `.as(u8)` 直接截断更宽的整数 `u16`

在 Flurry 中，`.as(T)` 绝不偷偷丢弃高位比特。如果要取 `u16` 的低 8 位，必须先通过 `'bitvec[0..8]` 显式切出 8 位，再调用 `.as(u8)`：

```flurry
fn reject_implicit_narrowing(wide: u16) -> u8 {
    -- ❌ 编译期拒绝：不能通过 .as(u8) 对 u16 执行隐式窄化截断。
    -- ✅ 正确写法：wide'bitvec[0..8].as(u8)
    wide.as(u8)
}
```

### 拒绝例 3：未加 `unsafe` 直接将任意 `BitVec<8>` 重解释为受限枚举

```flurry
fn invalid_flag_cast(bits: BitVec<8>) -> ReadyFlag {
    -- ❌ 编译期拒绝：ReadyFlag 并非所有 8 位模式均合法，
    -- 从 BitVec<8> 到 ReadyFlag 的重解释必须处于 unsafe 边界内。
    bits.as(ReadyFlag)
}
```

---

## 五、 本篇小结

| 操作类别 | 语法示例 | 安全契约与行为保证 |
| :--- | :--- | :--- |
| **值快照读取** | `let snap = word'bitvec;` | 立即生成独立的不可变位向量值拷贝，不受原变量后续修改影响 |
| **位置切片写回** | `word'bitvec[4..12] = rhs;` | 仅允许作用于可写位置；先求值右侧快照再更新目标位段，重叠区间绝不踩踏 |
| **显式位切片截断** | `wide'bitvec[0..8].as(u8)` | 禁止 `.as(T)` 隐式截断高位，所有窄化操作必须显式写出位区间 `[lo..hi]` |
| **受限类型重解释** | `unsafe { bits.as(ReadyFlag) }` | 当目标类型仅部分位模式有效时，必须显式标记 `unsafe` 并保证位模式合法 |

---

## 六、 迈向终极全栈项目（190–200）

至此，我们已经完整掌握了 Flurry 语言从基础语法、资源可达性、2LTT/CMTT 元编程、Trait 多态、代数效应与并发，到异构管线投影与形式化验证的全部核心法则。

从下一篇（**第 190 篇**）开始，我们将进入本教程最后 11 篇（第 190–200 篇）的**终极综合实战**：自底向上设计并实现一个横跨全部八卷语言特性的工业级系统——**Orion 异构流式信号处理与遥测引擎（Orion Signal Engine）**。
