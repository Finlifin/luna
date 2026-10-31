# Flurry 演进手记 (190)：系统架构设计：从硬件传感器采集、零拷贝缓冲到 SIMD/GPU 滤波内核

> **所属卷**：第八卷：打通异构硬件与工程架构——流水线投影、高级类型工程与综合实战 (171–200)  
> **上一篇**：[189：高可靠位级协议工程——`BitVec` 代数、双向编解码与原位字段更新](./189-implementing-verified-bitvec-synthesis-with-fallback.md)  
> **下一篇**：[191：第一层（协议层）：用定宽 `enum` 与跨元素 `BitVec` 模式实现零拷贝帧解码](./191-capstone-layer1-zero-copy-frame-decoder.md)  
> **参考规范**：[`docs/README.md`](../docs/README.md), [`docs/type/runtime-dependent-type.md`](../docs/type/runtime-dependent-type.md), [`docs/algebraic-effect-and-delimited-continuation/index.md`](../docs/algebraic-effect-and-delimited-continuation/index.md)

---

## 1. 工程背景：构建端到端异构信号处理系统 Orion-DSP

在前面的章节中，我们系统学习了 Flurry 的核心特性：定宽位向量与模式匹配、可达性类型与内存视图、形式化验证契约、两级类型论（2LTT）、上下文模态类型论（CMTT）、Trait 与自动派生、代数效应与软件事务内存（STM），以及面向异构硬件的流水线投影。

从本篇（第 190 篇）到第 200 篇，我们将通过一个完整的综合实战项目——**Orion-DSP 异构流式信号处理与遥测系统**，将所有核心语言特性串联进一套分层清晰的工程架构中。

Orion-DSP 面向高吞吐多通道传感器数据流处理场景，核心需求包括：
1. **底层二进制帧高速接入**：传感器以固定 32 位控制字发来采样流与校准帧，需要在零堆分配的前提下完成协议校验与位域提取；
2. **零拷贝分片环形缓冲**：解码后的采样帧进入多通道无锁环形队列，读写两侧的缓冲区切片需要静态保证无悬垂引用且读写区间互彼此分离；
3. **带通道校准身份的物理量转换**：每个传感器通道绑定独立的校准参数（`ChannelCalibration`），必须在类型层面防止将 A 通道的数据误用 B 通道的校准参数计算；
4. **编译期展开与异构加速的 FIR 滤波**：根据编译期已知的滤波器阶数与稀疏系数生成展开的卷积代码，并投影到设备端并行内核；
5. **容错管线、实时指标聚合与确定性混沌测试**：高频解码错误走值返回路径，异步调度与监控走代数效应处理器，多通道指标通过 STM 原子聚合，全链路支持纯内存确定性故障注入测试。

---

## 2. Orion-DSP 九层全栈架构蓝图

我们将 Orion-DSP 划分为九个正交的工程层级（对应第 191 至 199 篇）：

```
  =======================================================================================
                    [ Orion-DSP —— 九层全栈架构蓝图 (190-200) ]
  =======================================================================================
   第 191 篇 │ Layer 1: 协议解码层   │ 定宽 enum + 'bitvec 视图 + ++ 位拼接与原位写回
             │                       │ (零拷贝提取同步字、通道号与采样值，编译期穷尽性证明)
  -----------+-----------------------+---------------------------------------------------
   第 192 篇 │ Layer 2: 内存与租约层 │ 可达性限定 &reachability + 世代生命周期隔离
             │                       │ (静态锁定分片队列切片的来源边界，阻止悬垂指针逃逸)
  -----------+-----------------------+---------------------------------------------------
   第 193 篇 │ Layer 3: 形式化验证层 │ verified fn + where requires/ensures + outcomes
             │                       │ (数学证明环形缓冲区掩码寻址永不越界、双缓冲原地写入分离)
  -----------+-----------------------+---------------------------------------------------
   第 194 篇 │ Layer 4: 元编程生成层 │ 2LTT + CMTT 带上下文代码类型 < ctx |- T >
             │                       │ (在编译期根据稀疏滤波器系数自动裁剪零乘项并展开卷积)
  -----------+-----------------------+---------------------------------------------------
   第 195 篇 │ Layer 5: 异构加速层   │ ^gpu.kernel 流水线投影 + 'intrinsic(...).as(T)
             │                       │ (将滤波内核投射到设备端并行执行，保持主机/设备类型契约)
  -----------+-----------------------+---------------------------------------------------
   第 196 篇 │ Layer 6: 容错效应层   │ #Effects !Err T + 前后置 catch + 效应 Handler
             │                       │ (区分硬件瞬态错误重试与不可恢复错误短路，解耦异步调度策略)
  -----------+-----------------------+---------------------------------------------------
   第 197 篇 │ Layer 7: 事务协同层   │ atomic(...) STM 事务块 + 纯度与效应隔离契约
             │                       │ (无死锁地并发更新多通道吞吐量、峰值能量与告警水位)
  -----------+-----------------------+---------------------------------------------------
   第 198 篇 │ Layer 8: 配置与插件层 │ derive fn 反射派生 + dyn Trait + 扩展调用 { k: v }
             │                       │ (声明式构建流水线配置，支持按需擦除的动态告警插件)
  -----------+-----------------------+---------------------------------------------------
   第 199 篇 │ Layer 9: 混沌仿真验收 │ template test + 确定性效应 Handler 故障注入
             │                       │ (在不启动真实硬件的前提下跑通全链路故障恢复与不变量检验)
  =======================================================================================
```

---

## 3. 核心领域模型与跨层接口契约

在进入各层实现之前，我们先定义贯穿整个系统的顶层领域类型与跨层契约。

### 3.1 帧头位域与运行时依赖采样窗口

Orion 协议帧头包含 4 位版本号、12 位通道号与 16 位采样点计数。解析出的采样窗口类型直接依赖运行时的采样点数：

```flurry
struct FrameHeader {
    version: BitVec<4>,
    channel_id: BitVec<12>,
    sample_cnt: u16,
}

-- 采样窗口的长度依赖运行时绑定 count
struct SampleWindow
where
    quote count: lift usize,
{
    ptr: *i16,
}
```

### 3.2 传感器校准身份与静态前置契约

为了防止不同通道的采样数据混用校准参数，我们将通道校准器的指针通过 `lift` 绑定在通道数据包类型上，并用 `where requires` 契约消除运行期除零与越界风险：

```flurry
struct ChannelCalibration {
    gain_num: i32,
    gain_den: i32,
    offset: i32,
}

struct CalibratedPacket
where
    quote cal: lift *ChannelCalibration,
    quote len: lift usize,
{
    window: SampleWindow<len>,
}

verified fn read_calibrated_sample(
    cal: *ChannelCalibration,
    packet: CalibratedPacket<cal, n>,
    index: usize,
) -> i32
where
    n: usize,
    requires index < n and cal.gain_den.* != 0,
{
    let raw_val = unsafe { packet.window.ptr[index] }.as(i32);
    (raw_val * cal.gain_num.*) / cal.gain_den.* + cal.offset.*
}
```

这段签名同时提供了三层静态保护：
1. **依赖身份一致性**：调用 `read_calibrated_sample(cal_a, packet_b, 0)` 时，如果 `packet_b` 绑定的是 `cal_b`，类型检查会直接报错混用；
2. **索引边界证明（`index < n`）**：调用方必须证明 `index` 在窗口长度之内；
3. **非零除数证明（`cal.gain_den.* != 0`）**：在编译期避免整数除零异常。

### 3.3 错误返回（`!OrionErr`）与代数效应（`#Effects`）分轨设计

在 Flurry 中，值级错误返回与代数效应具有不同的运行期机制与适用边界：
- **高频、局部可恢复的数据包错误**（如同步头丢失、校验和失配）使用 `!OrionErr` 错误返回路径，具备紧凑的值返回开销；
- **横切整个系统的环境交互能力**（如遥测指标上报、异步让出、虚拟时钟读取）声明为 `effect`，由外层处理器统一拦截与注入：

```flurry
-- 轨道 A：确定性错误返回枚举（以 Err 结尾，变体采用 PascalCase）
enum OrionErr {
    UnsupportedVersion(BitVec<4>),
    TruncatedFrame(usize, usize),
    CrcMismatch(u16, u16),
    ZeroGainDenominator,
}

-- 轨道 B：环境能力与可观测性代数效应
effect emit_metric(channel: u16, sample_count: usize) -> void;
effect read_monotonic_ns() -> u64;
```

### 3.4 编译期阶数特化的 FIR 滤波接口

对于信号处理核心的有限冲激响应（FIR）滤波器，我们通过编译期参数接收滤波器阶数 `taps` 与系数数组：

```flurry
pure comptime fn FirCoeffTable(taps: usize) -> Type {
    Array<i32, taps>
}

verified fn apply_fir_step(
    comptime taps: usize,
    coeffs: FirCoeffTable(taps),
    history: *Array<i32, taps>,
) -> i32
where
    requires taps > 0,
{
    var acc: i32 = 0;
    var i: usize = 0;
    while i < taps {
        acc = acc + coeffs[i] * history.*[i];
        i = i + 1;
    }
    acc
}
```

---

## 4. 编译期边界拦截：防止跨层契约违规

在 Orion-DSP 的类型契约下，任何绕过长度证明或混用通道校准指针的调用都会在编译期被直接拦截：

```flurry
-- [编译期负例] 混用通道校准指针且未满足索引边界
-- fn architectural_contract_violation(
--     cal_a: *ChannelCalibration,
--     cal_b: *ChannelCalibration,
--     pkt_a: CalibratedPacket<cal_a, 64>,
-- ) -> i32 {
--     -- 错误 1：混用 cal_b 读取绑定了 cal_a 的数据包
--     -- 错误 2：未证明 cal_b.gain_den.* != 0
--     -- 错误 3：索引 64 违背 index < 64
--     read_calibrated_sample(cal_b, pkt_a, 64)
-- }
```

---

## 5. 本篇小结

| 架构原则 | Flurry 核心特性支撑 | 工程收益 |
| :--- | :--- | :--- |
| **零拷贝位级安全** | 定宽 `enum` + `'bitvec` 视图 + 运行时依赖类型 | 消除手写移位掩码错误与包头长度失配风险 |
| **静态资源与可达性边界** | `&reachability` + 内存视图分离证明 | 无引用计数开销，静态阻止悬垂指针逃逸与并发写冲突 |
| **双轨控制流解耦** | `!OrionErr` 错误返回 + `effect` 处理器 | 高频错误保持紧凑值返回，环境依赖支持纯内存仿真测试 |
| **单源异构与编译期生成** | 2LTT + CMTT + `^gpu.kernel` 流水线投影 | 编译期自动裁剪展开卷积算子，无缝投射至加速器硬件 |

下一篇（**第 191 篇**），我们将正式编写 Orion-DSP 的第一层代码——**用定宽 `enum` 与跨元素 `BitVec` 模式实现零拷贝帧解码器**。
