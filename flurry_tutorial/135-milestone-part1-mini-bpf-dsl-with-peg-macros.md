# Flurry 演进手记 (135)：第六卷里程碑（上）：用 PEG 构造宏设计一门迷你数据包过滤语言（Mini-BPF DSL）

欢迎来到《Flurry 演进手记》第六卷的综合实战里程碑。
在前面的篇章中，我们先后打造了工业级网络协议解析器（第 1–5 卷）与静态形状张量引擎（第 4 卷）。现在，在我们的高性能传感器网络网关入口处，每秒钟有数百万个原始网络帧（`Packet`）涌入网卡。

网关运维和安全团队向我们提出了一个核心诉求：
他们希望在网关代码里直接用类似 **BPF（Berkeley Packet Filter）/ tcpdump** 的声明式领域特定语言（DSL）编写过滤规则，例如：
- “只放行协议号为 `6`（TCP）、目的端口为 `443`、且携带 `SYN` 标志位的数据包。”
- “或者放行协议号为 `17`（UDP）且负载长度不超过 `512` 字节的数据包。”

我们将用**上下两篇（135 & 136）**完成这个令人兴奋的里程碑工程：
- **上半篇（本篇 135）**：设计 **Mini-BPF 的编译期规则中间表示（Rule IR）**，并使用第 130 回学到的 **`@define_macro2` PEG 构造宏**，把直观的过滤规则 DSL 在编译期解析为强类型的 Flurry 规则树。
- **下半篇（下一篇 136）**：使用 **CMTT（上下文模态类型理论）** 的表达式代码 `< pkt: Packet |- bool >` 与模式代码 `< Packet -| ... >`，在编译期把这棵规则树直接编译成**零解释器开销的原生机器判断代码**。

---

## 一、 问题背景与设计思路：为什么不能在运行期解析过滤规则字符串？

在传统的网络网关实现中（传统方案），如果要支持灵活的过滤规则，通常只有两条路可走：

```
  [ ❌ 传统方案 方案 A：在运行期解析字符串并在解释器循环里跑规则 ]
  let rule = parse_bpf_at_runtime("proto == 6 and dst_port == 443");
  for pkt in incoming_packets {
      if interpret_rule(rule, pkt) { ... } -- 每个包都要走一遍 AST 递归解释器 + 虚表分支。
  }
  • 问题背景：运行期 AST 解释器的分支预测失败率极高，吞吐量直接暴跌一个数量级。
          而且如果规则字符串拼错了字段名（如写成 `dst_prot == 443`），要等到线上运行才报错。

  [ ❌ 传统方案 方案 B：让工程师手写深层嵌套的布尔逻辑与位掩码代码 ]
  if (pkt.proto == 6 and pkt.dst_port == 443 and (pkt.flags'bitvec[1] == 1)) or ...
  • 问题背景：几十条复杂网关策略手写起来极易漏括号、算错位偏移，更无法作为结构化规则树做静态分析与自动机测试。
```

而在 Flurry 中，我们可以鱼与熊掌兼得：
1. **对人友好**：在源码里用 `@mini_bpf { ... }` 块状构造宏写出清晰可读的过滤规则 DSL；
2. **对编译器友好**：`@define_macro2` 在编译期按 PEG 文法将其转换为强类型的 `FilterRule` 枚举/结构体（拼错任何关键字或传入错误类型，编译期直接报错。）；
3. **对 CPU 友好**：下一回（136）再用 CMTT 将 `FilterRule` 在编译期彻底展开为原生内联机器码，运行期**零解释器、零动态分配**。

---

## 二、 架构设计：从 `@mini_bpf { ... }` 到强类型 `FilterRule` IR

在动手写 PEG 构造宏之前，一条铁打的编译器工程准则是：**先把目标 AST / IR 的数据结构设计干净。**
为什么 `@define_macro2` 不应该直接生成杂乱的底层 `if-else` Token，而是应该生成构造**强类型 `FilterRule` 编译期数据**的表达式？

请看这张**两段式解耦架构图**：

```
+-------------------------------------------------------------------------+
|               Mini-BPF 引擎的“语法前端 + CMTT 后端”解耦架构               |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ 源码层：用户编写的 DSL ]                                              |
|  const GATEWAY_RULE = @mini_bpf {                                       |
|      all_of(proto == 6, dst_port == 443, max_len <= 1500)               |
|  };                                                                     |
|         │                                                               |
|         ▼  第 135 回（本篇）：@define_macro2 PEG 解析与 TokenBuffer 映射 |
|  [ 编译期数据层：强类型 FilterRule IR ]                                  |
|  FilterRule.all3(                                                       |
|      FilterAtom.proto_eq(6u8),                                          |
|      FilterAtom.dst_port_eq(443u16),                                    |
|      FilterAtom.len_le(1500u16),                                        |
|  )                                                                      |
|         │                                                               |
|         ▼  第 136 回（下篇）：CMTT 上下文代码生成 (`compile_bpf_rule`)    |
|  [ 运行期代码层：零开销 CMTT 开放代码 < pkt: Packet |- bool > ]          |
|  '(pkt.proto == 6u8 and pkt.dst_port == 443u16 and pkt.payload_len <= 1500u16)|
+-------------------------------------------------------------------------+
```

把“PEG 语法解析”与“CMTT 代码生成”通过 `FilterRule` 数据结构解耦，有三大无可比拟的优势：
1. **构造宏极度简单**：`@define_macro2` 只需要把 DSL 翻译成 `FilterAtom` / `FilterRule` 的构造调用，不需要在無类型的 `TokenBuffer` 里操心变量卫生性和上下文绑定。
2. **免费享受静态类型检查**：如果用户在 DSL 里写了 `proto == 99999`，映射成 `FilterAtom.proto_eq(99999)` 后，因为 `proto_eq` 要求 `u8`，Flurry 类型检查器会当场报出精准的 `u8` 范围错误。
3. **规则树可分析、可打印、可组合**：因为 `FilterRule` 是正经的编译期数据，你可以用普通的 `pure comptime fn` 统计它有几个条件、对它做否定翻转、或者传给下一回的 CMTT 编译器。

---

## 三、 实战：定义数据包模型 `Packet` 与 `FilterRule` 编译期 IR

让我们先把传感器网关的数据包结构 `Packet`、原子过滤条件 `FilterAtom` 以及复合规则 `FilterRule` 扎扎实实地写出来：

```flurry
-- 传感器网关的数据包头结构
pub struct Packet {
    pub proto: u8,
    pub src_port: u16,
    pub dst_port: u16,
    pub flags: u8,
    pub payload_len: u16,
}

-- Mini-BPF 的原子过滤谓词描述（编译期纯数据）
pub enum FilterAtom {
    proto_eq(u8),
    src_port_eq(u16),
    dst_port_eq(u16),
    flags_mask(u8),
    len_le(u16),
}

-- 支持逻辑组合的复合过滤规则树
pub enum FilterRule {
    atom(FilterAtom),
    not_rule(FilterAtom),
    all2(FilterAtom, FilterAtom),
    all3(FilterAtom, FilterAtom, FilterAtom),
    any2(FilterAtom, FilterAtom),
}

-- 编译期自检工具：统计一条规则中包含多少个原子检查项
pure comptime fn count_atoms(rule: FilterRule) -> usize {
    rule match {
        .atom(_) => 1usize,
        .not_rule(_) => 1usize,
        .all2(_, _) => 2usize,
        .all3(_, _, _) => 3usize,
        .any2(_, _) => 2usize,
    }
}

test {
    const tcp_https = FilterRule.all3(
        FilterAtom.proto_eq(6u8),
        FilterAtom.dst_port_eq(443u16),
        FilterAtom.len_le(1500u16),
    );
    assumes count_atoms(tcp_https) == 3usize;
}
```

---

## 四、 进阶：用 `@define_macro2` 编写 `@mini_bpf` 的 PEG 语法前端

现在，我们要在宏准备脚本（`scripts/`）中定义 `@mini_bpf` 构造宏。
我们希望用户在 `@mini_bpf { ... }` 里可以直接用极其清爽的语法书写原子条件列表，例如：
```flurry
@mini_bpf {
    proto == 6u8;
    dst_port == 443u16;
    max_len <= 1500u16;
}
```
我们来设计它的 PEG 产生式：
1. **入口规则 `Entry`**：接收一序列以分号结尾的原子子句 `Clause*`；
2. **有序选择 `Clause`**：依次识别 `"proto" ~ "==" ~ FlurryExpr ~ ";"`、`"dst_port" ~ "==" ~ FlurryExpr ~ ";"`、`"src_port" ~ "==" ~ FlurryExpr ~ ";"`、`"max_len" ~ "<=" ~ FlurryExpr ~ ";"`；
3. **映射处理函数**：将解析出的每条子句映射为 `FilterAtom` 构造器，并组装为编译期数组 `Array<FilterAtom, n>`。

```flurry
@use meta.macros.*;
use meta.ast.*;

-- 在宏脚本中定义基于 PEG 文法的 @mini_bpf 构造宏
const mini_bpf_clauses = @define_macro2 {
    -- 1. PEG 语法规则：有序选择匹配各类数据包过滤子句
    Entry -> Clause*;
    Clause -> ProtoClause | DstPortClause | SrcPortClause | MaxLenClause;
    ProtoClause   -> "proto"    ~ "==" ~ FlurryExpr ~ ";";
    DstPortClause -> "dst_port" ~ "==" ~ FlurryExpr ~ ";";
    SrcPortClause -> "src_port" ~ "==" ~ FlurryExpr ~ ";";
    MaxLenClause  -> "max_len"  ~ "<=" ~ FlurryExpr ~ ";";

    -- 2. 映射函数：将解析出的 AST 节点转换为强类型的 FilterAtom 构造表达式
    fn(clauses: Repetition<FlurryExpr>) -> TokenBuffer {
        let items = clauses.fold({{}}, |acc, clause_expr| {
            {{ $acc $clause_expr, }}
        });
        {{ [ $items ] }}
    }
}

-- 同时我们也提供一个接收单条规则表达式的 AST 构造宏辅助器
const bpf_all3 = @define_macro1 {
    fn(
        a: meta.ast.FlurryExpr,
        b: meta.ast.FlurryExpr,
        c: meta.ast.FlurryExpr,
    ) -> TokenBuffer {
        {{
            FilterRule.all3($a, $b, $c)
        }}
    }
}

test {
    const https_rule = @bpf_all3(
        FilterAtom.proto_eq(6u8),
        FilterAtom.dst_port_eq(443u16),
        FilterAtom.len_le(1500u16),
    );

    assumes count_atoms(https_rule) == 3usize;
}
```

### 4.1 双重静态防线：语法错拦在 PEG，类型错拦在 IR

来看看这套设计在面对错误输入时有如何静态拦截错误：
- 如果运维人员在 `@mini_bpf_clauses { ... }` 里写了一个不存在的协议字段名 `unknown_field == 123;`，**第 1 道防线（PEG `Entry` 解析器）** 会在语法分析阶段直接报错拒绝；
- 如果字段名写对了，但把端口号写成了一个超出 `u16` 范围的非法值或者字符串 `"http"`，**第 2 道防线（`FilterAtom` 的静态类型检查）** 会在接回调用位置后立即报错：

```flurry
-- ❌ 编译期直接拒绝：FilterAtom.dst_port_eq 要求 u16 参数，传入字符串 "https" 被类型检查器静态拦截
comptime {
    let invalid_rule = @bpf_all3(
        FilterAtom.proto_eq(6u8),
        FilterAtom.dst_port_eq("https"),
        FilterAtom.len_le(1500u16),
    );
}
```

---

## 五、 本篇小结

| 里程碑组件 | 选用技术 | 承担的核心职责 |
| :--- | :--- | :--- |
| **`Packet` 结构体** | 标准运行期 `struct` | 承载网卡收到的协议号、源/目的端口、标志位与负载长度 |
| **`FilterAtom` / `FilterRule`** | 编译期可冻结 `enum` 数据 | 作为前端宏与后端代码生成器之间的强类型中间表示（IR） |
| **`@mini_bpf_clauses` / `@bpf_all3`** | `@define_macro2` (PEG) & `@define_macro1` | 在编译期验证 DSL 语法结构，并将其映射为 `FilterRule` 构造代码 |
| **双重错误拦截** | PEG `Entry` 规则 + `FilterAtom` 类型签名 | 拼写/语法错误在 PEG 解析期拦截；端口/位宽范围错误在类型检查期拦截 |

---

## 六、 下一篇预告

现在，我们的网关已经能在编译期拿到一棵完全经过语法和类型检验的 `FilterRule` 规则树（或者一组 `Array<FilterAtom, n>` 规则表）了。

接下来是最激动人心的下半场：
怎么把这棵活在编译期（$U_1$）的 `FilterRule` 数据树，直接变成运行期（$U_0$）跑在每条网卡收包队列上的**零开销原生匹配代码**？
- 我们不想在运行期写一个 `match rule { .proto_eq(p) => ... }` 去一遍遍解释 `rule`（那又退回解释器老路了。）；
- 我们希望写一个 `pure comptime fn compile_rule(rule: FilterRule) -> < pkt: Packet |- bool >`，在**编译期**对 `rule` 做模式匹配和循环组装，生成出一段纯粹只依赖运行期上下文 `pkt: Packet` 的 CMTT 开放代码。
- 甚至，我们还想结合 CMTT 的**模式代码 `< Packet -| ... >`** 和 **`BitVec<8>` 位向量切片**，直接把 TCP `SYN/ACK` 标志位检查与字段提取融合成一条原生匹配臂。

下一回 **《Flurry 演进手记 (136)：第六卷里程碑（下）：用 CMTT 将 Mini-BPF AST 编译为类型安全的零开销原生匹配器》**，让我们完成 Mini-BPF 编译器的最后一击。
