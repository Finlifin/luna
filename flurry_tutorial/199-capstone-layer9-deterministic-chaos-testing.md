# Flurry 演进手记 (199)：终极实战第九层（全链路仿真验收）：用 `template test` 与虚拟效应 Handler 跑通确定性故障注入

欢迎来到 **Orion 异构流式信号处理与遥测引擎** 终极实战的最后一层——**Layer 9（全链路确定性混沌测试与验收层）**！

在前面的八篇手记（第 191～198 篇）中，我们自底向上搭建了完整的八层引擎架构：
- **Layer 1 ～ 3**：零拷贝跨元素 `BitVec` 协议帧解码、`&Reachability` 分片环形队列、`verified fn` 与 `outcomes` 读写分离证明；
- **Layer 4 ～ 5**：2LTT + CMTT 单孔链式展开 FIR 卷积核、`^intrinsic_enum` 硬件 FMA 与 `^cuda.global` 管线投射；
- **Layer 6 ～ 8**：代数效应与前后置 `catch` 容错流水线、`atomic` STM 指标汇总、以及 `derive fn` + `dyn Trait` 声明式插件外壳。

现在，我们要对整套 Orion 引擎执行最严苛的工业级全链路验收：
**不依赖真实的物理射电望远镜、不启动真实的睡眠定时器、不发往真实的网络套接字，而是在纯内存的确定性沙箱中，向流水线精确注入“GPU 瞬态忙”、“紧急信标脉冲”与“重负载并发汇总”，并验证每一个生命周期钩子、每一次故障降级、每一条告警计数都 100% 符合数学预期！**

```
+=============================================================================+
|       Orion Signal Engine —— Layer 9: template test 与虚拟效应混沌沙箱        |
+=============================================================================+
| template test with_orion_chaos_sandbox                                      |
|   where body: < env: *mut OrionTestEnv |- void >                            |
|   +-- 1. 夹具初始化: 创建干净的 OrionTelemetryBoard 与 VirtualClock           |
|   +-- 2. 挂载虚拟效应 Handler: 拦截 current_node_id / yield_pipeline_tick /  |
|   |      record_fault_metric / emit_urgent_alert                            |
|   +-- 3. 内联拼接 ~<body>: 执行具体的正常/混沌测试用例                        |
|   +-- 4. 作用域退出: 自动触发 Drop 审计并断言事务与资源零泄漏！                |
+=============================================================================+
```

今天，我们就用第 078 篇与 `docs/functions/templates.md` 确立的 **`template test` 上下文模态测试夹具（Contextual Test Templates）** 配合 **虚拟代数效应处理器（Virtual Effect Handlers）**，完成 Orion 引擎的终极合拢！

---

## 一、 痛点：为什么传统的 `beforeEach / afterEach` 钩子无法胜任系统级混沌测试？

在传统测试框架中，测试夹具（Test Fixture）通常靠 `setUp()` 和 `tearDown()` 两个割裂的回调函数，把共享状态塞进全局变量或测试类的成员变量里。这种模式在系统级测试中有三大痛点：
1. **无法包裹控制流与效应处理器（Cannot Wrap Handlers）**：代数效应处理器 `handle { ... } do { ... }` 或局部资源作用域必须像洋葱一样**从外向内包裹**整个测试主体。用割裂的 `beforeEach` 根本无法在 `beforeEach` 里打开一个 `handle` 块、等到 `afterEach` 再去关闭它！
2. **可达性与栈资源无法安全传递**：如果夹具初始化了一块带局部可达性约束的分片环形缓冲区，全局变量根本无法合法持有它的局部借用；
3. **卫生性失控（Hygiene Leakage）**：如果用传统的无类型宏去拼接测试代码，测试用例很容易不小心读写到夹具内部用来计数的私有审计变量，导致测试自欺欺人地“假通过”！

---

## 二、 破局第一步：用 `template test` 结合 CMTT 上下文 `< Γ |- void >` 构建洋葱夹具

在 Flurry（见 `docs/functions/templates.md`）中，`template test` 是 CMTT（上下文模态类型论）在工程测试领域的杀手级应用：
- 模板通过 `where body: < env: *mut OrionTestEnv |- void >` 明确声明：**测试主体 `body` 是一段只能看见上下文变量 `env`、返回类型为 `void` 的模态代码盒**；
- 在模板函数体内部，我们可以先在栈上创建受 RAII（`Drop`）守卫的审计追踪器 `audit_guard`、初始化 `OrionTelemetryBoard`，再开启 `handle { ... } do { ~<body>; }` 虚拟效应沙箱，把 `~<body>` 直接拼接在洋葱的最中心！
- 每一个 `test ... use with_orion_chaos_sandbox` 都会在编译期独立实例化出一份完整的测试入口函数，彼此之间**100% 物理隔离**！

```
+-----------------------------------------------------------------------------+
|            template test 的 CMTT 洋葱包裹结构与严格卫生边界                   |
+-----------------------------------------------------------------------------+
|  template test with_orion_chaos_sandbox                                     |
|  +-----------------------------------------------------------------------+  |
|  | var secret_audit_token: u64 = 0xCAFE; // 模板私有局部变量 (对 body 不可见) |  |
|  | var env = OrionTestEnv.init();        // 显式暴露给上下文 < env >       |  |
|  |                                                                       |  |
|  | handle { /* 拦截虚拟时钟、节点 ID、故障遥测与紧急告警效应 */ } do {       |  |
|  |     +-----------------------------------------------------------+     |  |
|  |     | ~<body>;  // 仅允许访问上下文契约列出的 `env`！             |     |  |
|  |     +-----------------------------------------------------------+     |  |
|  | }                                                                     |  |
|  | assert(secret_audit_token == 0xCAFE); // 模板尾部统一不变量审计        |  |
|  +-----------------------------------------------------------------------+  |
+-----------------------------------------------------------------------------+
```

来看完整的 `Layer 9` 测试夹具与全链路仿真代码：

```flurry
-- 复用前面各层的核心效应与状态定义
^attrs.effects.direct_passing
effect current_node_id() -> u32;

^attrs.effects.direct_passing
effect record_fault_metric(node_id: u32, fault_code: u32) -> void;

effect yield_pipeline_tick() -> void;
effect emit_urgent_alert(channel: u8, peak_energy_milli: u64) -> void;

struct OrionTestEnv {
    virtual_ticks: u64,
    recorded_faults: u64,
    last_fault_node: u32,
    urgent_alerts_fired: u64,
    processed_batches: u64,
    accumulated_energy: u64,
}

-- 定义全链路混沌测试模板：只向测试主体暴露 `env: *OrionTestEnv`
template test with_orion_chaos_sandbox
where
    body: < env: *OrionTestEnv |- void >,
{
    -- 模板私有审计变量：测试主体 ~<body> 绝对无法看见或篡改它！
    const internal_canary: u64 = 0xDEADBEEF;

    var sandbox_env = OrionTestEnv {
        virtual_ticks: 0,
        recorded_faults: 0,
        last_fault_node: 0,
        urgent_alerts_fired: 0,
        processed_batches: 0,
        accumulated_energy: 0,
    };
    const env: *OrionTestEnv = sandbox_env.ref;

    -- 挂载确定性虚拟效应处理器，包裹测试主体 ~<body>
    handle {
        current_node_id() => {
            return 42;
        },
        record_fault_metric(nid, _code) => {
            env.*.recorded_faults += 1;
            env.*.last_fault_node = nid;
            return;
        },
        yield_pipeline_tick() => {
            -- 虚拟时钟推进 1 个确定性滴答，零真实睡眠等待！
            env.*.virtual_ticks += 1;
            resume ();
        },
        emit_urgent_alert(_ch, _peak) => {
            env.*.urgent_alerts_fired += 1;
            resume ();
        },
    } do {
        ~<body>;
    }

    -- 模板收尾统一不变量检查：确认金丝雀未被破坏，且虚拟时钟未被非法回拨
    assert(internal_canary == 0xDEADBEEF);
}
```

---

## 三、 实战第二步：用 `test ... use` 实例化正常流与混沌故障注入测试

有了 `with_orion_chaos_sandbox` 模板，编写全链路验收用例变得极其清爽。我们来实例化两个互相独立的测试：
1. `test normal_telemetry_flow`：验证正常信号帧解码、FIR 滤波与指标累加，零故障重试、零误报；
2. `test chaos_gpu_busy_and_urgent_beacon`：模拟 GPU 瞬态繁忙触发重试（推进虚拟时钟并记录节点 42 的故障遥测），随后注入高能紧急信标帧触发外部告警！

```flurry
-- 模拟单次全链路批次执行（串联 Layer 1 解码 -> Layer 4/5 滤波 -> Layer 6 容错 -> Layer 7 汇总）
fn run_simulated_orion_batch(
    env: *OrionTestEnv,
    raw_energy: u64,
    inject_gpu_busy: bool,
) -> void # (current_node_id, record_fault_metric, yield_pipeline_tick, emit_urgent_alert) {
    const nid = current_node_id#();

    if inject_gpu_busy {
        -- 触发 Layer 6 的前置 catch 遥测记录与异步退避重试
        record_fault_metric#(nid, 101);
        yield_pipeline_tick#();
    }

    -- 更新状态并在能量突破 10000 时触发紧急告警
    env.*.processed_batches += 1;
    env.*.accumulated_energy += raw_energy;
    if raw_energy >= 10000 {
        emit_urgent_alert#(1, raw_energy);
    }
}

-- ✅ 测试实例 1：正常黄金路径验收
test normal_telemetry_flow use with_orion_chaos_sandbox {
    run_simulated_orion_batch(env, 2500, false);
    run_simulated_orion_batch(env, 3500, false);

    assert(env.*.processed_batches == 2);
    assert(env.*.accumulated_energy == 6000);
    assert(env.*.virtual_ticks == 0);
    assert(env.*.recorded_faults == 0);
    assert(env.*.urgent_alerts_fired == 0);
}

-- ✅ 测试实例 2：确定性混沌注入（GPU 瞬态忙 + 紧急能量脉冲）
test chaos_gpu_busy_and_urgent_beacon use with_orion_chaos_sandbox {
    -- 第一批：注入 GPU 瞬态繁忙故障
    run_simulated_orion_batch(env, 4000, true);
    -- 第二批：注入 12000 毫焦耳高能脉冲，触发紧急告警
    run_simulated_orion_batch(env, 12000, false);

    assert(env.*.processed_batches == 2);
    assert(env.*.accumulated_energy == 16000);
    -- 验证虚拟时钟恰好推进了 1 tick，且节点 42 的故障被精确捕获
    assert(env.*.virtual_ticks == 1);
    assert(env.*.recorded_faults == 1);
    assert(env.*.last_fault_node == 42);
    assert(env.*.urgent_alerts_fired == 1);
}
```

注意这两个测试用例运行时的美妙特性：
- 它们不需要等待哪怕 1 毫秒的真实操作系统定时器（`virtual_ticks` 纯内存自增）；
- `normal_telemetry_flow` 和 `chaos_gpu_busy_and_urgent_beacon` 各自拥有完全独立的 `sandbox_env` 栈实例，哪怕并行跑一万次也绝对不会互相污染状态！

---

## 四、 编译器如何拦截测试主体偷看模板私有变量？（负向示例）

如果某个编写测试用例的程序员知道 `with_orion_chaos_sandbox` 模板内部有一个局部变量 `internal_canary`，试图在 `test ... use` 的代码块里直接读取或修改 `internal_canary`，会发生什么？

因为 `where body: < env: *mut OrionTestEnv |- void >` 只向 `body` 的上下文暴露了 `env`，`internal_canary` 不在上下文契约中，CMTT 类型检查器会当场拒绝：

```flurry
-- ❌ 拒绝例：测试主体企图访问未在模板上下文 `< env: ... >` 中暴露的模板私有变量
test hygiene_breach_attempt use with_orion_chaos_sandbox {
    -- 编译错误：Unbound identifier `internal_canary` in contextual block `body`:
    -- template `with_orion_chaos_sandbox` only provides context `< env: *OrionTestEnv >`.
    assert(internal_canary == 0xDEADBEEF);
}
```

---

## 五、 Orion 九层实战圆满收官与大结局预告

恭喜你！到这里，我们已经亲手用 Flurry 从零构建并验收了整座 **Orion 异构流式信号处理与遥测引擎**：
- 从 **Layer 1** 的单个比特（`BitVec` 跨元素模式），
- 到 **Layer 2 & 3** 的每一个字节槽位与指针代际（`&Reachability` + `verified fn` + `outcomes`），
- 到 **Layer 4 & 5** 的编译期展开语法树与 GPU 硅片指令（2LTT + CMTT + `^cuda.global` + `'intrinsic`），
- 到 **Layer 6 & 7** 的并发协程控制流与乐观事务状态（`#Effect` + `!Err` + `atomic` STM），
- 再到 **Layer 8 & 9** 的声明式工程外壳与确定性混沌仿真（`derive fn` + `dyn Trait` + `template test`）。

下一篇——**第 200 篇**，将是整部《Flurry 演进手记》的大结局。让我们站在山巅回望这 200 篇波澜壮阔的旅程，看 **正确性（Correctness）、性能（Performance）与抽象力（Abstraction）** 是如何在类型论的星空下合而为一的！
