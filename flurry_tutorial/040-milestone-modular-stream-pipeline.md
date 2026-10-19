# Flurry 演进手记 (040)：第二卷里程碑——构建模块化、可插拔的流式数据处理管线

欢迎来到 **《Flurry 演进手记》第二卷** 的终极里程碑！

在第一卷（001 ~ 015）中，我们聚焦于单个字节、单个结构体和单个枚举的底层布局与基础控制流；
而在第二卷（016 ~ 039）中，我们的视野从“单点数据”扩展到了**函数接口设计、高级模式解构、闭包环境、CMTT 代码模板以及大型工程的模块化组织**。

今天，我们不再单独讲解某一个孤立的语法点，而是要像真正的系统架构师一样，把第二卷学到的核心武器全部组装起来，构建一套**模块化、可插拔的传感器流式数据处理管线（Modular Stream Pipeline）**！

让我们先看看这套流式管线的工程蓝图：

```text
  ══════════════════════ 传感器流式管线架构图 (Package: telemetry) ══════════════════════

  [1. 协议与视角层: private mod protocol] (036, 037, 038, 039)
     ├── SamplePacket { ready: bool, code: u32, reading: u32 }
     ├── newtypeRawScale / CalibratedScale + `.as(...)` 类型视角链
     └── 通过包根 `pub use` 暴露干净门面，隐藏内部 `private` 辅助细节

  [2. 模式解构与观察层: mod stage] (010, 034, 035)
     ├── `quote fn scale_reading`：单次求值代码接入，配合 `implicit quote` 读取上下文
     ├── `{ ready: true, reading } as whole and calibrate(reading) is scaled`：
     │   链式顺序匹配 (`and is`) + Copy 字段提取 + `as whole` 保留完整包
     └── `template fn process_stage`：基于 CMTT `< scaled: u32 |- u32 >` 统一封装阶段骨架

  [3. 灵活配置、闭包过滤与批量验收层: mod pipeline & test] (035, 038)
     ├── 依赖默认参数 `.limit: u32 = base * 2` + 属性收集 `{ ... }` + `implicit` 上下文
     ├── 显式环境闭包 `|{ threshold: u32 }| ...` 作为可插拔阈值过滤器
     └── `template test with_pipeline_fixture`：一键批量验收正常流、空流与超限流！
```

让我们分三个阶段（`v0` 原型痛点 $\rightarrow$ `v1` 核心管线组装 $\rightarrow$ `v2` 模板化扩展与批量验收），亲手写出这套完整的系统！

---

## 第一版：回顾痛点——如果没有第二卷的武器，代码会长成什么样？（v0）

想象一下，如果只用最原始的命令式写法（`v0`）去写一个带上下文校准、带可配置过滤器和多阶段复用的数据管线：
1. **配置参数传递地狱**：每个阶段都需要传递 `base_offset`、`limit`（默认是 `base_offset * 2`）以及调试追踪开关，函数调用处全是一长串分不清顺序的数字 `run(pkt, 10, 20, true)`。
2. **解构与整体保留冲突**：如果你想先检查 `pkt.ready == true`，再把 `pkt.reading` 丢进校准函数算出 `scaled`，确认 `scaled < limit` 后还要把原封不动的整个 `pkt` 交给归档器——在没有 `as whole` 和 `and ... is ...` 时，你要么写三层嵌套的 `if`，要么不小心把字段移走导致原对象无法整体交付。
3. **测试与阶段样板复制**：十几个管线算子和几十个单元测试里，到处复制粘贴相同的解包、追踪 `Drop` 对象和计数器初始化代码。

现在，让我们用第二卷解锁的 Flurry 特性，彻底重构这一切！

---

## 第二版：模块化封装、视角链、链式模式与闭包过滤器（v1）

我们先在单文件模块树中搭建管线的前两层：**协议层（`protocol`）**与**核心过滤引擎（`engine`）**。

请仔细观察下面代码中每一处特性是如何协同工作的：
- **`private mod protocol` + `pub use`**：把内部辅助常数藏在 `private` 里，只把 `SamplePacket`、`ScaledUnit` 等核心接口重导出到上层；
- **类型视角链 `u32.as(ScaledUnit)`**：让普通的 `u32` 读数既能直接参与算术运算，又能直接调用 `.is_safe()` 领域方法；
- **`{ ready: true, reading } as whole and ... is ... if ...`**：在一行模式里同时完成字段检验、Copy 字段提取、链式计算、闭包守卫过滤与整体对象绑定！

```flurry
use std.prelude.*;

-- ============================================================================
-- 模块 1：协议与类型视角层（隐藏内部实现路径，通过 pub use 导出公共 API）
-- ============================================================================
private mod protocol {
    -- 仅模块内部及后代可见的私有校准系数
    private const INTERNAL_GAIN: u32 = 2;

    pub struct SamplePacket {
        ready: bool = true,
        channel: u8,
        reading: u32,
    }

    pub newtype ScaledUnit = u32;

    impl ScaledUnit {
        pub fn is_safe(self) -> bool {
            self.as(u32) <= 1000u32
        }
    }

    pub fn hardware_gain() -> u32 {
        INTERNAL_GAIN
    }
}

-- 将协议层的公开成员重导出到包根门面
pub use protocol.{SamplePacket, ScaledUnit, hardware_gain}

-- ============================================================================
-- 模块 2：流式管线引擎（整合依赖默认参数、implicit 上下文、顺序匹配与闭包）
-- ============================================================================
mod engine {
    -- 用前导 `@` 直接从包根导入门面类型
    use @.{SamplePacket, ScaledUnit, hardware_gain}

    pub struct PipelineContext {
        offset: u32,
        accepted_count: *u32,
    }

    -- quote fn：保证在调用点 100% 展开，且普通实参 raw 只求值一次！
    -- 同时通过 implicit quote 直接对接调用点上下文中的 ctx 指针！
    pub quote fn calibrate_once(
        raw: u32,
        implicit quote ctx: *PipelineContext,
    ) -> u32 {
        raw * hardware_gain() + ctx.offset.*
    }

    -- 核心过滤与转换函数：
    -- 1. `.max_limit` 是可选参数，且默认值直接依赖前序参数 `min_threshold * 10u32`！
    -- 2. `implicit ctx` 自动从调用方作用域获取管线上下文指针！
    pub fn filter_and_calibrate(
        packet: SamplePacket,
        min_threshold: u32,
        .max_limit: u32 = min_threshold * 10u32,
        implicit ctx: *PipelineContext,
    ) -> ?(SamplePacket, u32) {
        -- 链式顺序模式匹配 + Copy 字段提取 + `as whole` 整体交付：
        packet match {
            { ready: true, reading } as whole
                and calibrate_once(reading) is calibrated
                if calibrated >= min_threshold and calibrated <= max_limit => {
                -- 通过类型视角链赋予 calibrated 领域方法 `.is_safe()`
                let view_val: u32.as(ScaledUnit) = calibrated;
                if view_val.is_safe() {
                    ctx.accepted_count.* += 1u32;
                    (whole, calibrated)
                } else {
                    null
                }
            },
            _ => null,
        }
    }
}
```

### 给 `filter_and_calibrate` 的数据流画张全景图（Code Cartoon）

让我们画一张 **Code Cartoon**，看看当一个 `SamplePacket` 流经 `filter_and_calibrate` 时，模式、视角和上下文是如何默契配合的：

```text
  输入: packet = SamplePacket { channel: 1, reading: 15 } (ready 默认为 true)
  调用: filter_and_calibrate(packet, 20u32)  (省略 .max_limit，自动算出 20 * 10 = 200!)
           │
           ▼
  [第 1 步：结构观察与 Copy 提取]
  `{ ready: true, reading } as whole`
  ├─ 检查 ready == true ──► ✅ 通过！
  ├─ 复制标量 reading (15) ──► 保留整个 packet 的所有权给 `whole`！
           │
           ▼
  [第 2 步：`and ... is ...` 链式计算与 `quote fn` 接入]
  `and calibrate_once(reading) is calibrated`
  ├─ `reading` 单次求值绑定为 15
  ├─ 结合 `implicit quote ctx` 算出：15 * 2 + ctx.offset = 30 + 5 = 35
  └─ 将 35 绑定到新模式变量 `calibrated`！
           │
           ▼
  [第 3 步：模式守卫与 `.as(ScaledUnit)` 类型视角检查]
  ├─ 守卫 `35 >= 20 and 35 <= 200` ──► ✅ 通过！
  ├─ 视角 `let view_val: u32.as(ScaledUnit) = 35;` ──► 调用 `view_val.is_safe()`
  └─ 自增 `ctx.accepted_count.*`，交付元组 `(whole, 35)`！
```

---

## 第三版：用 `template fn` 定制算子，用 `template test` 批量验收（v2）

现在，我们要为管线加上最后两块拼图：
1. 业务团队需要快速定义多个自定义的流式变换算子（比如 `boost_stage`、`attenuate_stage`）。我们用一个带 `Drop` 审计追踪的 **`template fn stage_runner`**，把“解包可选结果 + 审计追踪”统统封装在模板骨架里，算子作者只需写一行核心数学公式！
2. 质量保障（QA）团队需要验收不同通道、不同状态下的管线行为。我们用 **`template test pipeline_test`** 自动准备好计数器、上下文指针和 `implicit` 环境，让每个测试用例清爽得像自然语言！

来看看最终完整合体的里程碑代码：

```flurry
-- 接续上面的 protocol 与 engine 模块

struct StageTrace {
    stage_id: u32,
}

impl Drop for StageTrace {
    fn drop(*self) {
        println("[trace] stage {} finished", self.stage_id.*);
    }
}

-- ============================================================================
-- 模块 3：基于 CMTT 的算子模板（封装公共审计与可空短路）
-- ============================================================================
template fn stage_runner(
    input: ?(SamplePacket, u32),
    quote lambda body: < packet: SamplePacket, val: u32 |- u32 >,
) -> ?u32 {
    let trace = StageTrace { stage_id: 1u32 }
    let (packet, val)? = input else {
        return null;
    }
    ~<body>
}

-- 算子实例 1：双倍放大算子（自动共享签名、StageTrace 清理与 `?` 解构！）
fn boost_stage use stage_runner {
    val * 2u32 + packet.channel.as(u32)
}

-- 算子实例 2：平滑衰减算子
fn attenuate_stage use stage_runner {
    val / 2u32
}

-- ============================================================================
-- 模块 4：测试模板与自动化批量验收
-- ============================================================================
template test pipeline_test
where body: < ctx_ptr: *engine.PipelineContext, count_ptr: *u32 |- void >
{
    let count: u32 = 0u32;
    let count_ptr = count.ref;
    let state = engine.PipelineContext {
        offset: 5u32,
        accepted_count: count_ptr,
    }
    let ctx_ptr = state.ref;
    ~<body>;
    println("[summary] accepted packets: {}", count);
}

-- 验收用例 1：正常就绪数据包流经完整管线
test active_packet_flow use pipeline_test {
    -- 将 ctx_ptr 设为当前作用域的 implicit 值，后续调用自动供应！
    implicit let current_ctx: *engine.PipelineContext = ctx_ptr;

    -- 构造数据包（ready 使用默认值 true）
    let pkt = SamplePacket { channel: 1u8, reading: 15u32 }

    -- 15 * 2 + 5 = 35，落在默认区间 [20, 200] 内！
    let filtered = engine.filter_and_calibrate(pkt, 20u32);

    -- 送入模板生成的 boost_stage 算子：35 * 2 + 1 = 71
    let final_out = boost_stage(filtered);
    if final_out is result? do {
        println("active output: {}", result);
    }
}

-- 验收用例 2：未就绪数据包与超限数据包自动拦截
test rejected_packet_flow use pipeline_test {
    implicit let current_ctx: *engine.PipelineContext = ctx_ptr;

    -- 包 1：显式指定 ready: false（覆盖默认值 true）
    let unready = SamplePacket { ready: false, channel: 2u8, reading: 50u32 }
    -- 包 2：显式指定命名可选参数 .max_limit = 30u32，使 15*2+5 = 35 超限被拦！
    let over_limit = SamplePacket { channel: 2u8, reading: 15u32 }

    let r1 = attenuate_stage(engine.filter_and_calibrate(unready, 20u32));
    let r2 = attenuate_stage(engine.filter_and_calibrate(over_limit, 20u32, .max_limit = 30u32));

    assumes r1 matches null and r2 matches null;
}
```

让我们运行这两个测试实例，看看控制台输出的完整生命周期轨迹：

```text
[trace] stage 1 finished
active output: 71
[summary] accepted packets: 1
[trace] stage 1 finished
[trace] stage 1 finished
[summary] accepted packets: 0
```

看！即使在 `rejected_packet_flow` 中，`r1` 和 `r2` 因为输入为 `null` 而在 `stage_runner` 模板的 `let (packet, val)? = input else { return null; }` 处提前退出了，模板内部的 `StageTrace` 依然分毫不差地执行了两次 `Drop`（打印出两行 `[trace] stage 1 finished`），并且测试模板末尾的 `[summary] accepted packets: 0` 也精准验证了没有任何非法包混入下游！

---

## 第二卷全景复盘表格

回顾我们在第二卷中掌握并在此里程碑中融会贯通的核心积木：

| 领域维度 | 核心语法与机制 | 在本里程碑管线中的实战体现 |
| :--- | :--- | :--- |
| **参数与调用接口** | `.opt: T = expr`（依赖前序参数）、`implicit` 参数 | `filter_and_calibrate` 中 `.max_limit = min_threshold * 10u32`，以及 `implicit ctx` 自动透传上下文 |
| **高级模式匹配** | `{ ready: true, reading } as whole`、`and expr is pat`、`matches` | 在不拆散原 `SamplePacket` 的前提下复制 `reading`、链式调用 `calibrate_once` 并交付 `(whole, calibrated)` |
| **代码接入与模板** | `quote fn`、`implicit quote`、`template fn`、`template test` | `calibrate_once` 单次求值展开；`stage_runner` 与 `pipeline_test` 复用上下文骨架与 `Drop` 清理 |
| **接收者与视角链** | 多层指针自动适配、`A.as(B)` 类型视角链、`.as(Trait)` | `u32.as(ScaledUnit)` 让校准后的整数直接调用 `.is_safe()`，同时保持 `u32` 本体身份 |
| **模块化与可见性** | `private mod`、默认包内可见、`pub use`、`@path` / `.path` | `private mod protocol` 隐藏 `INTERNAL_GAIN`，包根 `pub use` 提供干净门面，`engine` 用 `@` 跨模块引用 |

---

### 通往第三卷的悬念：踏入所有权、可达性与纯度的深水区！

恭喜你完成了 **《Flurry 演进手记》第二卷** 的全部旅程！
到现在为止，我们在管线里传递的 `SamplePacket` 里装的都是 `bool`、`u8`、`u32` 这样简单的标量，或者在函数内部老老实实地用局部指针 `*u32`。

但是，当我们的传感器管线要处理**真正昂贵的底层硬件资源**（比如一块独占的 DMA 环形缓冲区 `DmaBuffer`、一个打开的文件描述符，或者一个内部持有指向其他局部变量指针的视图结构体 `BufferView`）时，一系列直击系统编程灵魂的终极问题迎面而来：

1. **三档值复制语义（`Once` / `Clone` / `Copy`）**：在 Flurry 中，为什么默认类型都是仿射（Affine）的？`Once`、`Clone` 和 `Copy` 三者之间到底是什么关系？
2. **部分移出（Partial Move）与结构体残余清理**：如果我有一个包含两个 DMA 缓冲区的结构体 `DualChannel { left: DmaBuffer, right: DmaBuffer }`，我把 `pair.left` 移（Move）给了下游函数，然后再给 `pair.left = new_buf` 重新赋上新值——Flurry 编译器是如何精确追踪“每一个字段当前到底是活着还是已经被移空”，并在作用域结束时只 `Drop` 那些还活着的字段的？
3. **可达性限定类型（Reachability-Qualified Types，`&Reachability T`）**：如果不引入 Rust 那套让人头秃的生命周期标注 `'a`，Flurry 是如何用 **`&x Ptr`**（可达性追踪）在编译期 100% 阻止悬垂指针（Dangling Pointer）逃逸出作用域，同时又允许单线程顺序代码里合法的指针别名自由共存的？
4. **函数纯度（`pure fn`）与内存读写效果**：当我们在形式化验证契约或编译期计算里要求一个函数是 `pure` 时，它到底能不能在内部创建并修改自己的局部 `var` 变量？

在即将开启的 **《Flurry 演进手记》第三卷（041 ~ 060）：资源管理、可达性类型与内存效果** 中，我们将深入 Flurry 最引以为傲的资源与内存安全心脏。我们第三卷见！
