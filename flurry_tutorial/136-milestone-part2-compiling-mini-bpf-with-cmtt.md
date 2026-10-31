# Flurry 演进手记 (136)：第六卷里程碑（下）：用 CMTT 将 Mini-BPF AST 编译为类型安全的零开销原生匹配器

在上一回（135，第六卷里程碑上半部）中，我们用 `@define_macro2` 与 `@define_macro1` 为传感器网络网关搭建了 **Mini-BPF** 的语法前端，成功将声明式的数据包过滤规则解析成了强类型的编译期规则表示（`FilterAtom` 与 `FilterRule`）。

今天是第六卷里程碑的决胜下半场。
我们要扮演“编译器后端”的角色，但我们用的不是笨重的外部代码生成器，而是 Flurry 内建的 **CMTT（上下文模态类型理论）**：
1. 如何写一个纯编译期代码生成器 `compile_atoms`，在满足 **CMTT 单洞规则（Single-Hole Constraint）** 的前提下，把一组 `Array<FilterAtom, n>` 规则逐层包裹成一段开放表达式代码 **`< pkt: Packet |- bool >`**？
2. 如何结合 **`< Packet -| ... >` 模式代码**、**Quoted Arm（`quote fn ... (case pattern)`）** 与第五卷的 **`BitVec<8>` 位向量代数**，直接从匹配成功的数据包中提取负载长度或路由动作？

让我们立刻动手，把编译期的 `FilterAtom` 规则树变成零解释器开销的原生机器码。

---

## 一、 问题背景与设计思路：为什么禁止于同一个 Quote 里写 `'(~<left> and ~<right>)`？

当我们要把一串 `FilterAtom` 组合成一个合取（`and`）表达式代码 `< pkt: Packet |- bool >` 时，初学者最容易写出的 初始代码是这样的：

```flurry
-- ❌ 传统踩中 CMTT 单洞红线（编译期直接报错：）：
-- 试图在同一个 quote 主体里同时塞进两个 splice（~<left> 和 ~<right>）。
pure comptime fn bad_combine_and(
    left: < pkt: Packet |- bool >,
    right: < pkt: Packet |- bool >,
) -> < pkt: Packet |- bool > {
    '(~<left> and ~<right>)
}
```

### 1.1 回顾 CMTT 的“单洞链构造多步程序”法则

为什么 Flurry 要禁止在同一个 `'(...)` 里写两个 `~<...>`？
因为我们在第 121 回和 `docs/cmtt/quotation-and-stages.md` 中学过：**每段 Quote 的运行期主体至多允许一个代码接入点（Single Hole）**，以此保证上下文替换、控制流汇合与资源生命周期的线性可组合性。

那怎么用“每次只有一个洞”的 Quote 拼出包含 $N$ 个条件合取的复杂过滤程序呢？
就像我们在 `docs/cmtt/program-construction.md` 构造有限状态自动机 `build_step` 时那样——**用编译期循环 + 单洞链式包裹（Single-Hole Chaining）**。

```
+-------------------------------------------------------------------------+
|               CMTT 单洞链式包裹组装 Mini-BPF 过滤代码                     |
+-------------------------------------------------------------------------+
|                                                                         |
|  初始基底 (Base Code):                                                   |
|    code_0 = '(true)                          -- 0 个洞                  |
|                                                                         |
|  第 1 轮包裹 (接入 atom = len_le(1500)):                                  |
|    const prev = code_0;                                                 |
|    code_1 = '(pkt.payload_len <= 1500u16 and ~<prev>)  -- 仅 1 个洞      |
|                                                                         |
|  第 2 轮包裹 (接入 atom = dst_port_eq(443)):                              |
|    const prev = code_1;                                                 |
|    code_2 = '(pkt.dst_port == 443u16 and ~<prev>)      -- 仅 1 个洞      |
|                                                                         |
|  第 3 轮包裹 (接入 atom = proto_eq(6)):                                   |
|    const prev = code_2;                                                 |
|    code_3 = '(pkt.proto == 6u8 and ~<prev>)            -- 仅 1 个洞      |
+-------------------------------------------------------------------------+
                                   │
                                   ▼ 最终展开到 ~[pkt = incoming]<code_3>
   incoming.proto == 6u8 and incoming.dst_port == 443u16
       and incoming.payload_len <= 1500u16 and true
```

你看。每一层 `'(...)` 内部都**严格只有一个 `~<prev>` 接入点**，完全遵守单洞契约；而经过编译期 `while` 循环从后向前逐层包裹后，最终生成的运行期代码就是一条扁平、紧凑、按原顺序短路求值的原生 `and` 表达式。

---

## 二、 实战：用 CMTT 单洞链将 `Array<FilterAtom, n>` 编译为原生过滤器

让我们将这一思想写成完整可运行的 改进代码。注意看 `compile_atom_step` 是如何结合 `BitVec<8>` 位运算处理 `flags_mask` 的：

```flurry
pub struct Packet {
    pub proto: u8,
    pub src_port: u16,
    pub dst_port: u16,
    pub flags: u8,
    pub payload_len: u16,
}

pub enum FilterAtom {
    proto_eq(u8),
    src_port_eq(u16),
    dst_port_eq(u16),
    flags_mask(u8),
    len_le(u16),
}

-- 在编译期将单个 FilterAtom 包裹在已有代码 `rest` 外层（每层严格仅 1 个 ~<rest> 洞。）
pure comptime fn wrap_atom(
    atom: FilterAtom,
    rest: < pkt: Packet |- bool >,
) -> < pkt: Packet |- bool > {
    atom match {
        .proto_eq(expected) => '(pkt.proto == expected and ~<rest>),
        .src_port_eq(expected) => '(pkt.src_port == expected and ~<rest>),
        .dst_port_eq(expected) => '(pkt.dst_port == expected and ~<rest>),
        .flags_mask(mask) => '(((pkt.flags'bitvec and mask'bitvec) == mask'bitvec) and ~<rest>),
        .len_le(limit) => '(pkt.payload_len <= limit and ~<rest>),
    }
}

-- 从末尾向前遍历编译期规则数组，组装完整的 < pkt: Packet |- bool > 过滤代码
pure comptime fn compile_filter_chain(
    atoms: Array<FilterAtom, n>,
) -> < pkt: Packet |- bool >
where
    n: usize
{
    var code: < pkt: Packet |- bool > = '(true);
    var remaining = n;
    while remaining > 0usize {
        remaining -= 1usize;
        const current_atom = atoms[remaining];
        const previous = code;
        code = wrap_atom(current_atom, previous);
    }
    code
}

-- 运行期网卡收包过滤入口：编译期一次性展开，运行期零解释器开销。
fn accepts_https_syn(incoming: Packet) -> bool {
    const rules: Array<FilterAtom, 4> = [
        FilterAtom.proto_eq(6u8),
        FilterAtom.dst_port_eq(443u16),
        FilterAtom.flags_mask(0b0000_0010u8),
        FilterAtom.len_le(1500u16),
    ];
    const matcher = compile_filter_chain(rules);
    ~[pkt = incoming]<matcher>
}

test {
    let valid_pkt = Packet {
        proto: 6u8,
        src_port: 51234u16,
        dst_port: 443u16,
        flags: 0b0000_0010u8,
        payload_len: 128u16,
    }
    let bad_port_pkt = Packet {
        proto: 6u8,
        src_port: 51234u16,
        dst_port: 80u16,
        flags: 0b0000_0010u8,
        payload_len: 128u16,
    }

    assumes accepts_https_syn(valid_pkt);
    assumes not accepts_https_syn(bad_port_pkt);
}
```

请仔细观察 `accepts_https_syn`：
- `rules` 数组和 `compile_filter_chain(rules)` 全部在 `const` 编译期阶段执行完毕；
- 编译期常量 `expected`、`mask`、`limit` 被合法物化（Materialize）进了生成的运行期比较指令中；
- 在生成的目标机器码里，全然无存 `FilterAtom` 枚举，也未存 `while` 循环——只有 4 次紧挨着的寄存器比较指令。

---

## 三、 进阶：结合模式代码 `< Packet -| ... >` 与 `quote fn (case ...)` 提取字段

过滤数据包往往只是网关的第一步。一旦某个数据包匹配了特定模式，我们还希望**从数据包里解构并提取出关键绑定（比如目的端口 `port` 和负载长度 `len`），交给后续处理主体使用**。

这正是 CMTT **模式代码（Pattern Code，`< T -| Ψ >`）** 与 **函数级模式分支（`quote fn ... (case pattern)`）** 大显身手的舞台（`docs/cmtt/patterns-and-arms.md`）：
- **表达式代码 `< Ψ |- U >`**：消费上下文绑定 $\Psi$，算出结果 $U$；
- **模式代码 `< T -| Ψ >`**：匹配输入值 $T$，**一旦匹配成功，向外导出全新的上下文绑定 $\Psi$**。当它们在 `match` 分支 `~<pattern> => ~<body>` 中配对时，模式成功导出的 $\Psi$ 刚好严丝合缝地喂给右侧的 `~<body>`。

```
  incoming_pkt  ──►  ~<pattern> : < Packet -| port: u16, len: u16 >
                         │
                         ├─ 若匹配失败：免生 port 和 len，直接跳过本分支。
                         │
                         └─ 若匹配成功：导出新鲜绑定 { port: u16, len: u16 }
                                              │
                                              ▼ 定向供给右侧主体
                     ~<body>    : < port: u16, len: u16 |- u16 >
```

下面我们编写配对的 `< Packet -| ... >` 模式提取器与 `quote fn` 动作分派器：

```flurry
-- 接收配对的 CMTT 模式代码与主体代码：
-- 模式匹配 Packet 成功后产生 (port, len) 上下文，直接供给右侧 body 计算。
fn extract_from_packet(
    pkt: Packet,
    comptime pat: < Packet -| port: u16, len: u16 >,
    comptime body: < port: u16, len: u16 |- u16 >,
) -> u16 {
    pkt match {
        ~<pat> => ~<body>,
        _ => 0u16,
    }
}

-- 也可以用带 `case` 模式参数的 quote fn 将匹配模式与成功主体封装在一起
enum GatewayCommand {
    drop_all,
    forward_port(u16),
}

quote fn execute_command(cmd: GatewayCommand) -> u16;
quote fn execute_command(case .drop_all) -> u16 { 0u16 }
quote fn execute_command(case .forward_port(target_port)) -> u16 { target_port }

test {
    let tcp_pkt = Packet {
        proto: 6u8,
        src_port: 1000u16,
        dst_port: 8080u16,
        flags: 2u8,
        payload_len: 256u16,
    }

    -- 构造匹配 proto == 6u8 并导出 dst_port -> port, payload_len -> len 的模式代码
    const tcp_extract_pat: < Packet -| port: u16, len: u16 > =
        '<{ proto: 6u8, dst_port: port, payload_len: len, .. }>;
    const score_body: < port: u16, len: u16 |- u16 > =
        '(port + len);

    let extracted = extract_from_packet(tcp_pkt, tcp_extract_pat, score_body);
    assumes extracted == 8336u16;

    assumes execute_command(GatewayCommand.forward_port(443u16)) == 443u16;
    assumes execute_command(GatewayCommand.drop_all) == 0u16;
}
```

### 3.1 铁面无私的上下文定向检查

如果有人在编写 `body` 代码时，试图引用一个模式 `< Packet -| port: u16, len: u16 >` 根本没有导出的绑定名字（比如 `secret_key`），且接入位置也没有提供该绑定，会发生什么？

```flurry
-- ❌ 编译期直接报错：body 接口要求 < secret_key: u16 |- u16 >，
-- 但配对的模式只导出了 (port, len)，缺失的上下文绑定无法凭空产生。
fn reject_missing_pattern_binding(pkt: Packet) -> u16 {
    const pat: < Packet -| port: u16, len: u16 > =
        '<{ proto: 6u8, dst_port: port, payload_len: len, .. }>;
    const bad_body: < secret_key: u16 |- u16 > = '(secret_key);

    pkt match {
        ~<pat> => ~<bad_body>,
        _ => 0u16,
    }
}
```

---

## 四、 本篇小结：Mini-BPF 双篇里程碑全景回顾

| 阶段 | 输入载体 | 核心技术栈 | 输出产物 | 性能与安全收益 |
| :--- | :--- | :--- | :--- | :--- |
| **里程碑（上）135** | 声明式 DSL 源码 | `@define_macro2` (PEG) + `@define_macro1` | 强类型 `FilterRule` / `FilterAtom` 编译期数据 | 语法错误与类型越界在编译期双重拦截 |
| **里程碑（下）136** | `Array<FilterAtom, n>` | CMTT `< pkt: Packet \|- bool >` 单洞链循环包裹 + `BitVec<8>` | 零循环、零分支表的原生内联布尔表达式 | 消除运行期 AST 解释器开销 |
| **字段提取扩展** | 运行期 `Packet` | CMTT 模式代码 `< Packet -\| port, len >` + `quote fn (case ...)` | 定向绑定传递的原生 `match` 分支 | 匹配失败路径不泄露未初始化绑定 |

---

## 五、 下一篇预告

我们的 Mini-BPF 编译器已经能把声明式规则编译成飞快的原生代码了。

但是，所有写过编译器或自动机生成器的工程师心里都有一团挥之不去的阴影：
**“我怎么敢 100% 确信我写的 CMTT 代码生成器 `compile_filter_chain` 或状态机组装器 `build_step`，对所有边界输入（空输入、回边循环、重叠转移边、末尾报错状态）生成出来的原生代码，都跟标准参考规范分分毫无差？”**

如果我们为 20 种不同的协议自动机分别手写 20 套重复的初始化、状态追踪、日志记录与边界断言代码，测试文件很快就会变成难以维护的复制粘贴泥潭。
还记得我们在 `docs/functions/templates.md` 里见过的 **`template fn`** 和 **`template test`** 吗？

下一回 **《Flurry 演进手记 (137)：用 template test 为生成的自动机编写全覆盖性质测试》**，我们将用基于 CMTT 上下文注入的测试模板，为生成的代码打造一套严丝合缝的自动化验收靶场。
