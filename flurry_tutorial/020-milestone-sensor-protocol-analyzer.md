# Flurry 演进手记 (020)：第一卷里程碑——组装一台完整的 8 位传感器协议分析仪

从 [第 001 篇](./001-a-world-in-a-byte.md) 工位上那张皱巴巴的 8 比特传感器手册开始，一路披荆斩棘走到第 020 篇，我们已经完成了 **《Flurry 演进手记》第一卷（基础语法、控制流与数据结构）** 的全部核心积木探索。

在过去 19 篇里，我们报错了“死记硬背语法手册”的枯燥学法，而是顺着硬件解码、抗噪重试、通道热切换、字节拼接和抽象封装的真实核心问题，看着 Flurry 的每一项设计从代码中“长”了出来。

今天是第一卷的**收官里程碑（Capstone Milestone）**。我们不再只写零散的演示片段，而是要把前 19 篇铸造的所有武器融为一炉，组装出一台**五脏俱全、端到端闭环的 8 位传感器协议分析仪与通道监控固件**。

---

## 全景蓝图：我们要造一台什么样的分析仪？

下面先俯瞰这台协议分析仪的五大核心层次，以及每一层对应了我们在第一卷哪几篇学到的语言特性：

```text
 ╔═════════════════════════════════════════════════════════════════════════════╗
 ║              8-Bit Sensor Protocol Analyzer (第一卷全景架构)                 ║
 ╚═════════════════════════════════════════════════════════════════════════════╝

   [原始串口字节 raw: Byte]
             │
             ▼
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ 第 1 层：物理比特与定宽协议层 (001, 013, 014, 015, 016)                       │
 │  • 定宽模式 `enum Packet` (.base_type Byte, .discrete_tag true)              │
 │  • 位向量模式 `0xA (payload: Nibble)` 与 `BitVec` 拼接 `++` / 原位回写 `[..]` │
 └───────────────────────────────────┬─────────────────────────────────────────┘
                                     │  !DecodeErr Packet
                                     ▼
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ 第 2 层：名义类型隔离与封装视图层 (012, 017, 018, 019)                        │
 │  • `newtype ChannelId = u8`：防止通道号与普通读数混用                         │
 │  • `struct VerifiedSample` + `pub case critical_heat(*self) -> ?Tuple<u8>`  │
 │    隐藏内部校准换算细节，向外暴露声明式模式视图                                │
 └───────────────────────────────────┬─────────────────────────────────────────┘
                                     │  ![DecodeErr, SensorErr] ?VerifiedSample
                                     ▼
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ 第 3 层：对偶错误处理与抗噪重试流水线 (002, 006, 007, 008, 009, 010)          │
 │  • 错误行 `![DecodeErr, SensorErr]` 与后缀 `!` / `?` 传播                    │
 │  • `while :retry_loop` + `! { catch ... }` 前后置 catch 统一审计与重试        │
 │  • `return ... while cond` 与 `continue ... while cond` 转移守卫            │
 └───────────────────────────────────┬─────────────────────────────────────────┘
                                     │  成功样本交付 / 连续故障触发热切换
                                     ▼
 ┌─────────────────────────────────────────────────────────────────────────────┐
 │ 第 4 层：状态监控、指针投影与失败安全热切换 (003, 016, 018)                   │
 │  • `let` 绑定名称锁定 vs 内部字段/原位比特段更新 `status_reg'bitvec[0..4] += 1`│
 │  • 后缀指针 `.ref`、`.*` 与结构体指针字段投影                                 │
 │  • `var` 覆盖赋值四步核心规则：先求值新通道，失败 100% 保留旧通道，成功自动 Drop   │
 └─────────────────────────────────────────────────────────────────────────────┘
```

---

## 第一步：从 naive 的“一锅粥”脚本 到分层架构

如果在没有学过第一卷之前，让我们写这样一个包含“解码 + 校验 + 告警分类 + 3 次重试 + 备用通道切换 + 状态寄存器位更新”的模块，写出来的代码往往是一个塞满了机制数字 `0xF0`、`255`、多层 `if-else` 缩进、且一旦切换失败就会把旧通道误关闭的百行面条函数。

而在 Flurry 中，我们可以清晰地分四个模块段落（Stage 1 ~ Stage 4）将它优雅地搭建起来。

### Stage 1：定宽协议定义与强类型身份（回顾 001, 005, 016, 018）

首先，我们定义基础的 `Byte`、`Nibble` 透明别名，用 `newtype ChannelId = u8` 隔离通道号身份，并用 `.discrete_tag true` 定义零额外开销的 8 位协议包 `Packet` 以及 16 位上行汇总帧组装器：

```flurry
use std.prelude.*;

typealias Byte = u8;
typealias Nibble = BitVec<4>;

-- [018] newtype 建立独立类型身份：防止把传感器读数当成通道号传参。
newtype ChannelId = u8;

-- [001 & 014] 零额外 Tag 开销的定宽模式枚举：
-- 编译器在定义时自动证明 0x00、0xA_xxxx 与 0xF_0001 编码完全互斥。
enum Packet {
    .base_type Byte,
    .discrete_tag true,

    empty = 0x00,
    data: 0xA (payload: Nibble),
    reset_cmd = 0xF1,
}

-- [016] 用 BitVec 拼接 `++` 组装 16 位上行遥测帧：
-- 高 8 位为通道号，低 8 位为 Packet 原始编码字节
pure fn pack_uplink_frame(ch: ChannelId, pkt: Packet) -> u16 {
    let ch_bits = (ch.as(u8))'bitvec,
    let pkt_bits = pkt'bitvec;
    (ch_bits ++ pkt_bits).as(u16)
}
```

### Stage 2：错误行建模与带 `case` 视图的领域样本（回顾 002, 012, 019）

接着，我们定义解码错误 `DecodeErr` 和传感器量程错误 `SensorErr`（遵守错误类型以 `Err` 结尾、错误变体用 `PascalCase` 的核心规则）。
当成功提取出有效读数后，我们把它封装进 `VerifiedSample` 结构体中，并通过 `pub case` 视图向外提供 `.critical_heat(celsius)` 和 `.normal_heat(celsius)` 两种匹配视角，隐藏内部“读数乘以 8 加 10”的温度换算公式：

```flurry
-- [002] 错误枚举：类型名以 Err 结尾，变体名使用 PascalCase
enum DecodeErr {
    CorruptedByte(Byte),
    UnexpectedReset,
}

enum SensorErr {
    HardwareOverflow,
}

-- [012 & 019] 封装良好的领域样本：隐藏内部 raw_nibble，提供 case 匹配视图
struct VerifiedSample {
    raw_nibble: u8,

    fn to_celsius(*self) -> u8 {
        self.raw_nibble * 8u8 + 10u8
    }

    -- [019] 自定义匹配视图：当换算温度 >= 90°C 时匹配成功并交出摄氏度
    pub case critical_heat(*self) -> ?Tuple<u8> {
        let c = self.to_celsius();
        if c >= 90u8 {
            (c,)
        } else {
            null
        }
    }

    -- [019] 正常温度视图
    pub case normal_heat(*self) -> ?Tuple<u8> {
        let c = self.to_celsius();
        if c < 90u8 {
            (c,)
        } else {
            null
        }
    }
}
```

### Stage 3：单字节解码、错误行汇合与前后置 `catch` 重试循环（回顾 002, 007, 008, 009）

现在，我们把 `decode_raw_byte`（产生 `!DecodeErr`）和 `validate_packet`（产生 `!SensorErr`）通过错误行 `![DecodeErr, SensorErr]` 串联起来，并编写带 3 次自动重试与统一诊断日志的 `poll_with_retry`：

```flurry
fn decode_raw_byte(raw: Byte) -> !DecodeErr Packet {
    raw'bitvec match {
        0x00 => Packet.empty,
        0xA (payload: Nibble) => Packet.data { payload: payload },
        0xF1 => Packet.reset_cmd,
        _ => error DecodeErr.CorruptedByte(raw),
    }
}

fn validate_packet(pkt: Packet) -> ![DecodeErr, SensorErr] ?VerifiedSample {
    pkt match {
        .empty => null,
        .reset_cmd => error DecodeErr.UnexpectedReset,
        .data { payload: nib } => {
            let val = nib.as(u8);
            -- [007] 语句转移守卫：当 val == 15 (0xF) 时报硬件溢出错误
            return error SensorErr.HardwareOverflow while val == 15u8;
            VerifiedSample { raw_nibble: val }
        },
    }
}

-- [002] 错误行自动汇合：后缀 `!` 将 DecodeErr 与 SensorErr 统一嵌入返回类型
fn read_once(raw: Byte) -> ![DecodeErr, SensorErr] ?VerifiedSample {
    let pkt = decode_raw_byte(raw)!;
    let sample = validate_packet(pkt)!;
    sample
}

-- [002 & 008] 带标签循环 + 前后置 catch 错误消去块
fn poll_with_retry(port_value: Byte, max_retries: u8) -> ![DecodeErr, SensorErr] ?VerifiedSample {
    var attempts = 0u8;

    while :retry_loop true {
        let result = read_once(port_value)! {
            -- 1. 前置 catch：任何错误发生时，第一时间统一记账
            catch _ => {
                attempts += 1u8;
                println("[Retry {}] Fault detected on sensor line:", attempts);
            },
            -- 2. 具体错误分支：分类打印诊断详情
            DecodeErr.CorruptedByte(b) => println("  -> Bad wire byte: {}", b),
            DecodeErr.UnexpectedReset => println("  -> Unexpected reset command"),
            SensorErr.HardwareOverflow => println("  -> Sensor ADC overflow (0xF)"),
            -- 3. 后置 catch：汇合控制流，未满次数则重试，否则原样向上返回错误结果
            catch failure => {
                continue retry_loop while attempts < max_retries;
                return failure;
            },
        }
        result
    }
}
```

### Stage 4：带硬件寄存器位段回写与失败安全热切换的 `AnalyzerEngine`（回顾 003, 016）

最后，我们构建常驻内存的分析仪引擎 `AnalyzerEngine`。它非但持有当前活跃的 `ChannelId`（并在 `Drop` 时关闭硬件通道），还维护着一个 8 位的硬件状态寄存器 `hw_status_reg: Byte`：
- 低 4 位 `hw_status_reg'bitvec[0..4]`：记录累计收到的有效样本数（每次原位复合赋值 `+= 1u4`）；
- 高 4 位 `hw_status_reg'bitvec[4..8]`：记录最近一次告警标志（高温告警时原位回写为 `0xE`）。

```flurry
enum SwitchErr {
    ChannelOffline(u8),
}

struct AnalyzerEngine {
    active_channel: ChannelId,
    hw_status_reg: Byte = 0x00,

    pub fn open(ch: ChannelId, simulate_offline: bool) -> !SwitchErr AnalyzerEngine {
        let raw_ch = ch.as(u8);
        println("Opening hardware channel {}...", raw_ch);
        return error SwitchErr.ChannelOffline(raw_ch) while simulate_offline;
        AnalyzerEngine { active_channel: ch }
    }

    pub fn ingest_byte(*self, raw: Byte) {
        -- [010 & 019] 对偶模式 (? 与 !) 与自定义 case 视图的正交嵌套。
        poll_with_retry(raw, 2u8) match {
            -- 成功、非空、且命中高温告警视图
            (.critical_heat(celsius))?! => {
                println("ALERT on ch {}: Critical temperature {} C!",
                    self.active_channel.as(u8), celsius);
                -- [016] 原位回写状态寄存器的高 4 位为告警码 0xE，低 4 位计数 +1
                self.hw_status_reg'bitvec[4..8] = 0xE;
                self.hw_status_reg'bitvec[0..4] += 1u4;
            },
            -- 成功、非空、且为正常温度视图
            (.normal_heat(celsius))?! => {
                println("OK on ch {}: Normal temperature {} C",
                    self.active_channel.as(u8), celsius);
                self.hw_status_reg'bitvec[0..4] += 1u4;
            },
            -- 成功但为空心跳包 (0x00)
            null! => {
                println("Heartbeat on ch {}", self.active_channel.as(u8));
            },
            -- 重试耗尽后依然失败
            error _ => {
                println("Channel {} permanently faulted after retries!",
                    self.active_channel.as(u8));
            },
        }
    }
}

impl Drop for AnalyzerEngine {
    fn drop(*self) {
        println("Releasing hardware channel {} (final status_reg={})",
            (self.active_channel.*).as(u8), self.hw_status_reg.*);
    }
}

-- [003] 失败安全的热切换：右侧 open 失败时，不提前销毁 target 原有的旧通道。
fn hot_swap_channel(
    target: *AnalyzerEngine,
    backup_ch: ChannelId,
    simulate_offline: bool,
) -> !SwitchErr void {
    target.* = AnalyzerEngine.open(backup_ch, simulate_offline)!;
}
```

---

## 端到端点火测试与编译期防线验收

让我们写一个完整的 `test` 块，把正常报文、空心跳包、高温报文、损坏噪声字节以及一次失败的热切换、一次成功的热切换全部跑一遍：

```flurry
test {
    -- 1. 打开主通道 1（注意：engine 用 var 声明以允许直接替换，或者通过 engine.ref 替换）
    var engine = AnalyzerEngine.open(1u8.as(ChannelId), false)! {
        .ChannelOffline(_) => return,
    }

    -- 2. 喂入一个空心跳包 (0x00)
    engine.ingest_byte(0x00);

    -- 3. 喂入一个正常数据包 0xA5 (payload = 5 -> 5 * 8 + 10 = 50°C)
    engine.ingest_byte(0xA5);

    -- 4. 喂入一个高温数据包 0xAC (payload = 12 -> 12 * 8 + 10 = 106°C >= 90°C!)
    engine.ingest_byte(0xAC);

    -- 5. 喂入一个线路噪声损坏字节 0x37（触发 2 次重试后报错）
    engine.ingest_byte(0x37);

    -- 6. 尝试热切换到离线通道 9（故意失败。验证通道 1 完好无损。）
    hot_swap_channel(engine.ref, 9u8.as(ChannelId), true)! {
        .ChannelOffline(ch) => {
            println("Hot-swap to ch {} failed! Staying on ch {}",
                ch, engine.active_channel.as(u8));
        },
    }

    -- 7. 热切换到备用通道 2（成功。此时旧的通道 1 立即执行 Drop，随后切入通道 2）
    hot_swap_channel(engine.ref, 2u8.as(ChannelId), false)! {
        .ChannelOffline(_) => (),
    }

    -- 8. 在新通道 2 上继续处理一个数据包 0xA2 (payload = 2 -> 26°C)
    engine.ingest_byte(0xA2);
}
```

### 如果有人试图破坏规则？编译器防线一览（❌）

在这套不到 150 行的固件代码里，Flurry 编译器在背后为我们筑起了五道静态安全防线：

```flurry
test {
    -- ❌ 防线 1 (001/016)：位宽越界拦截。Nibble 是 4 位，字面量 16 编译期报错。
    let bad_pkt = Packet.data { payload: 16 }

    -- ❌ 防线 2 (016)：禁止静默截断。16 位上行帧禁止直接 .as(u8)，必须先切片 [0..8]。
    let bad_byte = pack_uplink_frame(1u8.as(ChannelId), Packet.empty).as(u8);

    -- ❌ 防线 3 (018)：newtype 身份隔离。禁止将普通 u8 读数直接当成 ChannelId 传给 open。
    let _ = AnalyzerEngine.open(1u8, false);

    -- ❌ 防线 4 (002)：错误行未处理拦截。无法忽略 poll_with_retry 返回的 ![DecodeErr, SensorErr]。
    let raw_sample: ?VerifiedSample = poll_with_retry(0xA5, 2u8);

    -- ❌ 防线 5 (003)：let 绑定名整值替换拦截。
    let fixed_engine = AnalyzerEngine { active_channel: 1u8.as(ChannelId) }
    fixed_engine = AnalyzerEngine { active_channel: 2u8.as(ChannelId) }
}
```

---

## 第一卷（001–020）通关武器库总表

在结束第一卷之际，让我们把这 20 篇解锁的核心心智模型整理成一张速查勋章表：

| 领域 | 核心语法与工具 | 一句话心智模型（Flurry 哲学） | 对应篇章 |
| :--- | :--- | :--- | :--- |
| **词法与分号** | `--`、中缀空白、`}` 省略分号 | 运算符两侧留白消除 `<` 歧义；块结尾 `}` 天然自带语句边界 | 001, 004 |
| **字面量与空值** | `.symbol`、`LiteralInto`、`()`、`null`、`undefined` | 区分“无返回值 `()`”、“可选缺席 `null`”与“未初始化 `undefined`” | 005, 006 |
| **控制流与守卫** | 后缀 `match`、`if/while ... is ... do`、`return ... while` | 数据流从左往右走；模式层守卫用 `if`，语句跳转守卫用 `while` | 001, 007, 008, 009 |
| **模式匹配进阶** | `(p)` vs `(p,)`、浮点 `NaN` 覆盖、`pub case` 视图 | 模式与表达式严格对偶；`case` 视图在隐匿内部布局的前提下支持按需解构 | 010, 011, 019 |
| **错误与容错** | `?T`、`![E1, E2] T`、`error expr`、`! { catch ... }` | 零栈展开开销的结果 ABI；前后置 `catch` 将审计、分类与重试解耦 | 002 |
| **绑定与资源** | `let` vs `var`、`.ref`、`.*`、覆盖赋值四步核心规则 | `let` 只焊死名牌不冻结对象；覆盖赋值先求新值再 Drop 旧值，失败保平安 | 003 |
| **用户定义类型** | `struct`、`enum`（含分组与定宽）、`union`、`newtype` | 定宽 `enum` 编译期证互斥；`union` 赋值非自动 Drop；`newtype` 零成本隔离身份 | 001, 012–018 |
| **BitVec 代数** | `'bitvec`、`++`、`[lo..hi]`、`and`/`or`/`not`、原位回写 | 告别手算掩码移位；`and`/`or` 双侧求值不短路；切片左值直接原位回写 | 001, 015, 016 |

---

### 迈向第二卷的悬念：函数的十八般武艺

看着我们刚刚完成的 `AnalyzerEngine`，它已经非常健壮了。但是，随着这套固件被推广到全公司的十几个产品线，新的需求像雪片一样飞来：

1. **配置参数爆炸**：`poll_with_retry` 现在只有 `max_retries` 一个配置项。如果还要支持 `backoff_ms`（退避时间）、`log_enabled`（是否打日志）、甚至让某个参数的默认值**直接根据前一个参数算出来**（比如 `max_timeout = max_retries * 100`），难道我们要像 C++ 一样写十几个重载函数，或者强迫调用者每次都传一个笨重的 `Config` 结构体吗？
2. **像写 JSON 一样调用函数**：能否直接写 ` AnalyzerEngine.open { channel: 1, retries: 3, verbose: true } `，让函数自动把未声明的命名属性收集成一个匿名结构体？
3. **插件化回调与零开销内联**：如果用户想传入一个自定义的滤波闭包 `|sample| sample.to_celsius() > 50` 来过滤数据，当这个闭包捕获了外部计数器时，Flurry 是如何通过 `%c fn` 追踪闭包环境的生死与清理顺序的？`inline fn` 和 `quote fn` 又是如何做到比 C 宏更安全、且保证**实参永远只求值一次**的？

休息一下，给自己倒杯咖啡。
在下一篇——也是 **第二卷《函数的十八般武艺——参数协议、模式链、闭包与模板 (021–040)》** 的开篇之作 **《Flurry 演进手记 (021)：实参求值的核心规则——源码从左到右求值、方法接收者与失败回滚边界》** 中，我们将正式开启一段全新的进化旅程。
