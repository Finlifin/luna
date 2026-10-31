# Flurry 演进手记 (198)：综合实战第八层（插件与配置层）：用 `derive fn`、`dyn Trait` 与扩展调用实现声明式配置热加载

在前面的七篇手记中，我们的 **Orion 异构流式信号处理引擎** 已经完成了从底层二进制帧解码（Layer 1）、分片环形队列与形式化验证（Layer 2 & 3）、CMTT 编译期卷积核生成与 GPU 投射（Layer 4 & 5）、异步容错流水线（Layer 6）到 STM 实时指标汇总（Layer 7）的全链路内核建设。

现在，我们要为引擎披上优雅易用的外部外壳——**Layer 8（插件与声明式配置层）**。在实际工程交付中，这一层需要解决三个核心诉求：
1. **前向依赖的通道窗口参数推导**：配置通道采样窗口时，调用者往往只传总容量 `total_slots`（或者再传一个起始偏移 `.start`），希望 `.count` 默认自动等于 `total_slots - start`，且被编译器证明默认值求值稳定无副作用；
2. **DSL 风格的声明式扩展调用（Extended Application）**：用类似配置描述语言的 `{ key: value, ...preset }` 语法收集任意键值属性与位置路由项；
3. **零样板二进制序列化与热插拔输出插件（`derive fn` + `dyn Trait`）**：将遥测快照自动派生为紧凑二进制格式，并通过显式的 `.dyn(TelemetrySink)` 动态胖指针分发给一组运行时可热切换的输出插件（如本地环形日志、UDP 广播器）。

```
+=============================================================================+
|          Orion Signal Engine —— Layer 8: 声明式配置与热插拔插件层             |
+=============================================================================+
| 1. 前向依赖可选参数 (.start: u32 = 0, .count: u32 = total - start) [需 >= P3]|
| 2. 扩展调用块 mount_orion_profile { station_code: 1024, ...preset_ch() }    |
| 3. derive PacketChecksum.use(xor_checksum) for TelemetryPacket + .dyn(Sink) |
+=============================================================================+
```

今天，我们就把第二卷的函数参数协议（`docs/functions/parameters.md`）与第三卷的 Trait 派生及动态接口（`docs/traits/derivation.md`、`docs/traits/dynamic-interfaces.md`）融会贯通，构建 Orion 引擎的第八层。

---

## 一、 实战第一步：用前向依赖可选参数（P3 稳定观测守卫）简化窗口配置

在配置 Orion 信道采样范围时，函数 `make_channel_window` 需要三个参数：`total: u32`、可选的起始槽位 `.start: u32`（默认为 `0`），以及可选的激活槽位数 `.count: u32`（默认填满剩余所有槽位 `total - start`）。

在许多语言中，默认参数表达式里根本禁止引用前面的形参，逼着你写一堆 `Option<u32>` 再去函数体里 `unwrap_or(total - start)`。
而在 Flurry（见第 030 篇与 `docs/functions/parameters.md`）中：
- 可选参数的默认值表达式按**从左到右**的严格顺序求值，**后续可选参数的默认值可以直接引用前面已经绑定的必填参数或可选参数**（如 `.count: u32 = total - start`）。
- 同时，为了防止默认值求值引发隐蔽的全局状态污染或求值顺序问题，编译器强制要求**所有默认参数表达式必须满足至少 P3（稳定环境观测，Stable Observation）纯度**，且严禁反向引用排在它后面的参数。

```flurry
struct ChannelWindow {
    total: u32,
    start: u32,
    count: u32,
}

-- ✅ 正向示例：.count 的默认值直接前向依赖前面已绑定的 total 与 start。
pure fn make_channel_window(
    total: u32,
    .start: u32 = 0,
    .count: u32 = total - start,
) -> ChannelWindow {
    ChannelWindow {
        total: total,
        start: start,
        count: count,
    }
}

pure fn test_window_defaults() -> Tuple<ChannelWindow, ChannelWindow> {
    -- 1. 只传 total = 64：start 自动为 0，count 自动推导为 64 - 0 = 64
    let full_win = make_channel_window(64);

    -- 2. 传 total = 64, .start = 16：count 自动利用传入的 start = 16 推导为 64 - 16 = 48。
    let tail_win = make_channel_window(64, .start = 16);

    (full_win, tail_win)
}
```

### 编译器如何拦截反向依赖与不纯的默认值表达式？（负向示例）

如果有人在默认值里反向引用后面的参数（`.first: u32 = second, .second: u32 = 1`），或者在默认值里偷偷调用了一个具有 P0 副作用的计数器函数，编译器会立即拦截：

```flurry
-- ❌ 负例 1：默认参数表达式违反从左到右绑定顺序，反向引用了排在后面的参数
pure fn bad_backward_default(
    -- 编译错误：Default expression for `.first` cannot reference parameter `second`
    -- declared later in the parameter list.
    .first: u32 = second,
    .second: u32 = 8,
) -> u32 {
    first + second
}
```

---

## 二、 实战第二步：用 `...items`、`...properties` 与花括号扩展调用打造声明式 DSL

当运维人员初始化整个 Orion 节点时，他们希望既能传入位置序列项（或通过 `...preset_channel()` 展开位置元组），又能传入任意键值配置属性（如 `station_code`、`active_slots`）。

在 Flurry 中（见第 025 篇与 `docs/functions/parameters.md`）：
- `...items: Tuple<u32, bool>` 收集所有位置序列项（包括 `...expr` 展开的元组元素）；
- `...properties: Props` 将花括号中所有 `key: value` 属性收集为一个保留各字段名和精确类型的匿名结构记录（Record）；被调函数通过 PTC 字段约束（`requires properties:: station_code : u32 and properties:: active_slots : u32` 以及可选属性的 `inline if properties:: ...`）在函数体内直接读取所需属性字段；
- 调用时直接使用花括号扩展调用语法 `mount_orion_profile { ... }`（语法规则为 `extended_application -> expr { (property | extend_arg | expr)* }`；花括号扩展调用接受 `key: value` 属性、`...expr` 展开项和位置表达式，区分于圆括号调用中的 `.id = expr` 命名实参）。

```
+-----------------------------------------------------------------------------+
|            Flurry 花括号扩展调用 (Extended Application) 分流机制             |
+-----------------------------------------------------------------------------+
|  mount_orion_profile {                                                      |
|      station_code: 1024,       ---> 归入 ...properties 属性记录字段          |
|      active_slots: 64,         ---> 归入 ...properties 属性记录字段          |
|      ...preset_channel(),      ---> 展开归入 ...items: Tuple<u32, bool>     |
|  }                                                                          |
+-----------------------------------------------------------------------------+
```

来看完整的代码实现：

```flurry
struct OrionNodeSummary {
    active_slots: u32,
    enabled_channels: u32,
    station_code: u32,
}

-- 同时收集位置元组 (...items) 与匿名属性记录 (...properties)，
-- 并在 PTC 字段约束下直接访问 `properties.active_slots` 与 `properties.station_code`
pure fn mount_orion_profile(
    ...items: Tuple<u32, bool>,
    ...properties: Props,
) -> OrionNodeSummary
where
    Props: Type,
    requires properties:: station_code : u32
        and properties:: active_slots : u32
{
    let (primary_ch, is_on) = items;
    let active_ch: u32 = if is_on { primary_ch + 1 } else { 0 };

    -- 可选属性字段：若调用方提供了 extra_slots: u32，则在编译期选入此分支并累加
    let total_slots = inline if properties:: extra_slots : u32 {
        properties.active_slots + properties.extra_slots
    } else {
        properties.active_slots
    };

    OrionNodeSummary {
        active_slots: total_slots,
        enabled_channels: active_ch,
        station_code: properties.station_code,
    }
}

pure fn preset_channel() -> Tuple<u32, bool> {
    (0u32, true)
}

pure fn build_default_station_profile() -> OrionNodeSummary {
    -- 使用花括号扩展调用语法收集位置展开与命名属性，由 mount_orion_profile 直接消费属性并构造摘要
    mount_orion_profile {
        station_code: 9001u32,
        active_slots: 96u32,
        ...preset_channel(),
        region_tag: "deep-space-array-north",
    }
}
```

---

## 三、 实战第三步：`derive fn` 编译期自动派生与 `.dyn(Trait)` 热插拔插件分发

配置加载完毕后，Layer 7 汇总出的遥测数据包 `TelemetryPacket` 需要序列化并推送给后端插件列表。
1. **编译期反射派生（`derive fn`）**：通过 `derive PacketChecksum.use(xor_checksum) for TelemetryPacket`，在编译期遍历结构体字段生成零运行时反射开销的校验和计算实现；
2. **显式动态分发（`.dyn(TelemetrySink)`）**：在 Flurry（见第 106 篇与 `docs/traits/dynamic-interfaces.md`）中，将具体插件实例转为动态接口胖指针**免于发生隐式强转**，必须显式调用 `.dyn(TelemetrySink)`。

```flurry
trait PacketChecksum {
    pure fn compute_checksum(*self) -> u64;
}

struct TelemetryPacket {
    station_code: u64,
    total_frames: u64,
    peak_energy: u64,
}

-- 为只有 u64 字段的遥测包编写具名编译期派生策略
derive fn xor_checksum for PacketChecksum {
    pure fn compute_checksum(*self) -> u64 {
        -- 在 runtime 函数体中绑定编译期反射结果必须使用 const
        const repr = Type.repr_of(Self);
        var acc: u64 = 0x5A5A_5A5A;
        inline if repr is do {
            .`struct` => {
                inline for (_, field) in repr.fields.enumerate() {
                    acc = acc * 31 + self'get_field(field.name).*;
                }
            },
            _ => {},
        }
        acc
    }
}

-- 启用自动派生：编译期直接生成零开销的字段展开代码
derive PacketChecksum.use(xor_checksum) for TelemetryPacket;

-- 定义热插拔遥测输出插件接口（对象安全，支持 dyn TelemetrySink）
trait TelemetrySink {
    fn push_packet(*self, pkt: TelemetryPacket, checksum: u64) -> u64;
}

struct RingLogSink {
    written_packets: u64,
    last_checksum: u64,
}

impl TelemetrySink for RingLogSink {
    fn push_packet(*self, _pkt: TelemetryPacket, checksum: u64) -> u64 {
        self.written_packets.* += 1;
        self.last_checksum.* = checksum;
        self.written_packets.*
    }
}

-- ✅ 正向示例：显式通过 .dyn(TelemetrySink) 构造动态胖指针并完成多态分发
fn broadcast_to_active_sink(
    sink_ptr: *dyn TelemetrySink,
    pkt: TelemetryPacket,
) -> u64 {
    let chk = pkt.ref.compute_checksum();
    sink_ptr.push_packet(pkt, chk)
}

fn run_plugin_demo() -> u64 {
    var local_logger = RingLogSink { written_packets: 0, last_checksum: 0 }
    let pkt = TelemetryPacket {
        station_code: 9001,
        total_frames: 4096,
        peak_energy: 8800,
    }

    -- 显式通过 .ref.dyn(TelemetrySink) 将具体类型指针转换为动态 Trait 对象指针
    broadcast_to_active_sink(local_logger.ref.dyn(TelemetrySink), pkt)
}
```

---

## 四、 本层通关小结与下一层预告

在 **Layer 8（插件与配置层）** 中，我们体验了 Flurry 将“高级工程人体工学”与“底层零开销抽象”有机结合的威力：
- 用前向依赖可选参数（`.count: u32 = total - start`）与 P3 纯度守卫消除了繁琐的 `Option` 样板代码；
- 用 `...items`、`...properties` 与花括号扩展调用实现了类型安全的声明式配置 DSL；
- 用 `derive fn` 与显式 `.dyn(TelemetrySink)` 实现了零样板序列化校验与热插拔插件多态。

至此，**Orion 异构流式信号处理与遥测引擎** 的八大业务与架构层（Layer 1 ～ Layer 8）已经全部落成。
但是在将它发射升空或部署到无人值守的深空地面站之前，我们还有最后一道生死攸关的关卡：
**如何在一台普通的开发机上，以 100% 可复现、确定性的方式，对整套引擎进行包含丢包、GPU 瞬态忙、恶劣尖峰脉冲在内的全链路混沌测试（Deterministic Chaos Testing）？**

在下一篇 **第 199 篇（Layer 9：全链路仿真验收层）** 中，我们将使用 Flurry 独特的 **`template test` 测试模板与虚拟代数效应处理器**，为 Orion 引擎跑通全链路确定性验收。
