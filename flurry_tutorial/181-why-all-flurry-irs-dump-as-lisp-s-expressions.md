# Flurry 演进手记 (181)：用编译期反射与类型视图审视代码结构——结构内省、布局断言与契约自检

在[上一篇（第 180 篇）](./180-parsing-outer-boundaries-and-hir-lowering.md)中，我们学习了多属性前缀、控制流花括号边界以及链式后缀视图（如 `value'bitvec'type`）的组合规则。

在开发底层协议库、硬件驱动或高性能序列化框架时，程序员经常需要确认代码在编译期收敛后的真实形态：
- 一个经过多层泛型包装或位向量转换的表达式，最终推导出的静态类型究竟是什么？
- 一个用于硬件寄存器映射或网络帧传输的 `struct`，它的字节大小（`Type.size_of`）、对齐方式（`Type.align_of`）和字段偏移是否符合硬件手册的要求？
- 当我们用 `comptime` 反射遍历一个类型时，如何用纯 Flurry 代码写出可复用的编译期结构自检函数？

今天，我们就来系统梳理如何利用 **`'type` 视图、`Type.size_of` / `Type.align_of` / `Type.repr_of` 与 `comptime` 断言**，在编译期对类型与代码结构进行透明审视。

---

## 一、 问题背景：靠人工心算内存布局与嵌套泛型类型的隐患

在系统编程中，许多难以排查的故障源于“程序员以为的类型/布局”与“实际生效的类型/布局”存在分歧：
1. **字段重排或填充字节导致的协议错位**：以为三个字段加起来是 6 字节，实际上因为 4 字节对齐填充变成了 8 字节；
2. **链式视图转换后的位宽偏差**：对多个 `BitVec` 片段做切片与拼接 `hi ++ mid ++ lo` 后，以为总宽度是 32 位，实际算出来是 30 位；
3. **泛型分支特化结果不透明**：在复杂的 `pure comptime fn` 类型计算之后，待定某个具体调用点最终选中的是哪个底层类型。

与其等到运行时抓包或用外部调试器反汇编，不如直接在源码中利用 Flurry 的编译期自省能力立下静态契约。

---

## 二、 用 `'type` 与 `comptime` 块验证表达式的静态类型

在 Flurry 中，任意表达式 `expr` 都可以通过后缀 `'type` 在编译期观察其静态类型，而且在类型位置或 `quote` 上下文中使用 `'type` 规避触发运行时的额外求值开销。

让我们看一个结合 `'bitvec` 切片、拼接与 `'type` 编译期自检的例子：

```flurry
fn extract_low_byte(word: u32) -> BitVec<8> {
    let view = word'bitvec;
    view[0..8]
}

fn merge_header(version: BitVec<4>, channel: BitVec<12>, length: u16) -> BitVec<32> {
    version ++ channel ++ length'bitvec
}

test verify_bitvec_view_types {
    let sample_word: u32 = 0x1234_5678;
    let low_byte = extract_low_byte(sample_word);

    -- 在编译期直接验证表达式推导出的精确类型
    comptime {
        assumes sample_word'bitvec'type == BitVec<32>;
        assumes low_byte'type == BitVec<8>;
    }

    let hdr = merge_header(0b0001, 0b0000_0000_0101, 64u16);
    comptime {
        assumes hdr'type == BitVec<32>;
    }
    assumes hdr[0..4] == 0b0001;
}
```

通过 `comptime { assumes ...; }`，一旦未来有人修改了 `extract_low_byte` 的切片范围（比如误写成了 `view[0..7]`），非但返回类型检查会报警，编译期类型自检断言也会第一时间指出预期类型与实际类型的偏差。

---

## 三、 结构布局内省：`Type.size_of`、`Type.align_of` 与字段审查

对于需要与 DMA 缓冲区、文件头或硬件外设直接交互的结构体，我们可以在编译期编写纯函数，利用 `Type.size_of(T)`、`Type.align_of(T)` 和 `Type.repr_of(T)` 对其物理骨骼进行全面体检：

```text
  [ 编译期结构与布局自检流程 ]
             目标类型 T
                 │
                 ├──▶ Type.size_of(T)   ──▶ 验证总字节数是否符合帧长要求
                 ├──▶ Type.align_of(T)  ──▶ 验证对齐边界是否满足 DMA 要求
                 └──▶ Type.repr_of(T)   ──▶ 遍历每个字段的名称、类型与偏移
```

来看一段对硬件遥测包头 `PacketHeader` 进行编译期布局审计的实战代码：

```flurry
struct PacketHeader {
    magic: u16,
    flags: u16,
     sequence: u32,
    timestamp_ns: u64,
}

-- 编译期纯函数：检查给定类型是否满足 8 字节对齐且总大小恰好为 16 字节
pure comptime fn is_dma_aligned_header(T: Type) -> bool {
    Type.size_of(T) == 16 and Type.align_of(T) == 8
}

-- 在模块顶层通过 comptime 块执行静态布局门禁
comptime {
    assumes is_dma_aligned_header(PacketHeader);
}

test packet_header_layout_check {
    let hdr = PacketHeader {
        magic: 0xAA55,
        flags: 0x0001,
        sequence: 42,
        timestamp_ns: 1000_000,
    }
    assumes hdr.sequence == 42;
}
```

---

## 四、 用 `inline for` 与结构反射生成可读的结构化摘要

除了静态断言，当我们为自定义数据结构编写调试格式化或字段级校验器时，可以利用 `Type.repr_of(T)` 配合 `inline for` 在编译期按字段声明顺序展开代码，无需手写重复的字段访问样板：

```flurry
struct SensorPoint {
    channel: u32,
    raw_value: i32,
}

fn print_struct_summary(point: SensorPoint) {
    -- 编译期按声明顺序确定性地格式化输出字段信息
    println(
        "(sensor-point :channel {} :raw-value {})",
        point.channel,
        point.raw_value,
    );
}

test summary_output {
    let pt = SensorPoint { channel: 3, raw_value: -15 }
    print_struct_summary(pt);
}
```

### 负例：无法对尚未确定具体类型的未约束参数直接断言固定字节数

如果在一个泛型函数中，类型参数 `T` 没有受到任何具体布局约束，就试图在 `comptime` 中断言 `Type.size_of(T) == 4`，在传入其他大小的类型时会在实例化阶段报错：

```flurry
fn require_four_byte_item(item: T) -> u32
where
    T: Type,
    requires Type.size_of(T) == 4
{
    4
}

fn bad_caller() -> u32 {
    -- ❌ 编译期报错：u64 的大小为 8 字节，违背 requires Type.size_of(T) == 4 契约
    require_four_byte_item(100u64)
}
```

---

## 五、 本篇小结

| 自省工具 | 语法形式 | 典型工程用途 |
| :--- | :--- | :--- |
| **类型观察视图** | `expr'type` | 在免于触发额外运行时求值的前提下提取表达式的静态 `Type` |
| **尺寸与对齐查询** | `Type.size_of(T)` / `Type.align_of(T)` | 在 `comptime` 块或 `where requires` 中核验硬件寄存器与协议帧布局 |
| **编译期契约断言** | `comptime { assumes ...; }` | 在编译期拦截位宽失配、对齐填充偏差与类型退化 |

---

## 六、 下一篇预告

在日常调用高阶函数时，我们经常写出 `apply_first(|item| item + 1, 41i32)` 这样的代码：第一个参数是没有写类型注解的匿名函数 `|item| item + 1`，第二个参数才给出了具体的 `41i32`。
- 为什么第一个参数里的 `item` 能够借用第二个参数 `41i32` 的类型信息完成推导？
- 既然编译期类型推导参考了第二个参数的信息，运行时的实参求值顺序是否会被改变？

下一篇 **[Flurry 演进手记 (182)：函数调用中的双向类型推导——兄弟参数约束、外层期望类型与求值顺序保证](./182-elaboration-sibling-parameters-and-call-plans.md)**，我们将深入剖析函数调用的类型推导法则。
