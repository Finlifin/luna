# Flurry 演进手记 (167)：跨模块与跨包边界的效应隐藏与接口封装

> **阅读指南**：在上一回（第 166 回）结尾，我们提出了一个关乎大型软件工程质量的架构命题：代数效应在模块内部是组织复杂控制流（如游标回溯、局部缓存查询、中间状态机步进）的利器；但在跨模块（`mod`）、跨包（`package`）甚至跨语言（`^attrs.abi(.c)`）的公开 API 边界上，我们忌讳把内部实现细节的效应标签泄漏给外部调用者。结合 `docs/code-organization/visibility.md`、`docs/algebraic-effect-and-delimited-continuation/handling-capability.md` 与 `effect-abi.md`，本篇我们将系统阐述 **效应的可见性三档范围、模块边界内部消去（Internal Elimination）与效应抽象分层**。

---

## 一、 问题背景：“实现细节效应”顺着函数签名一路传染到顶层 `main`

假设我们在开发一个数据库查询引擎包 `query_engine`。在模块内部，为了实现表达式求值器的短期记忆优化，我们定义了一个内部效应 `effect lookup_scratch_cache(key: u32) -> ?u32;`。

### 省略封装地把内部效应暴露在公开 API 签名上

如果库作者在编写对外公开的入口函数 `pub fn evaluate_query(q: u32)` 时，偷懒没有在模块内部安装 Handler 把 `lookup_scratch_cache` 消去：

```flurry
-- 把内部私有的实现效应直接暴露给外部调用者
mod engine {
    -- 这本来只是引擎内部使用的临时缓存查询效应。
    private effect lookup_scratch_cache(key: u32) -> ?u32;

    -- 问题 1（可见性冲突与封装泄漏）：
    --   对外公开的 pub fn 签名上却挂着一个 private 效应 #[lookup_scratch_cache]。
    --   外部包的用户根本看不见 lookup_scratch_cache，怎么可能替你安装 Handler？
    -- 问题 2（接口脆弱性）：
    --   哪怕把 lookup_scratch_cache 标成 pub，一旦下个版本我们把内部算法从缓存改成了直接计算，
    --   所有外部调用者的类型签名都会被破坏。
}
```

**核心问题**：
并并非所有的效应都应该透传给最终用户。在良好的系统架构中，效应分为两类：
1. **契约级公开效应（Public Contract Effects）**：本就要求调用者提供环境或策略的效应（例如标准 I/O、用户自定义的日志接收器或异步 Runtime）；
2. **实现级内部效应（Internal Implementation Effects）**：仅用于在模块或包内部解耦复杂算法的局部控制流（如第 161 回的回溯游标 `#Stream`、局部缓存 `#ScratchCache`）。**这类效应必须在模块公开边界内部被 100% 拦截消去（Handled Internally）。**

---

## 二、 核心机制：三档可见性与“模块边界效应防火墙”

首先，让我们回顾 `docs/code-organization/visibility.md` 中 Flurry 的 **三档可见性规则（Visibility Tiers）**，它们同样严格支配着 `effect` 和 `effect enum` 声明：

| 声明修饰符 | 允许访问与处理（Handle）的范围 | 典型效应架构用途 |
| :--- | :--- | :--- |
| **`private`** | 仅限**声明所属模块（`mod`）及其子模块** | 模块内部算法专用的微效应（如解析游标、局部回溯），对兄弟模块完全隐形 |
| **无修饰（默认）** | **整个当前包（Package）内部**自由协作 | 包内各子模块（如 `parser` 与 `optimizer`）共享的内部管线效应，零泄漏到包外 |
| **`pub`** | 跨包全局公开（使用者须有合法可达路径） | 正式对外发布的 SDK 级效应契约（也可放在 `private mod` 中再通过 `pub use` 导出） |

下面来看看一个设计精良的模块是如何构建 **“效应防火墙（Effect Firewall）”** 的：

```text
  ┌─ 模块内部：`mod codec` (使用 private / 包内效应解耦复杂状态) ───────────┐
  │                                                                         │
  │  private effect read_cursor() -> u8;                                    │
  │                                                                         │
  │  -- 内部工作函数：自由使用 #[read_cursor] 效应，代码高度清爽            │
  │  private fn decode_pair() -> #[read_cursor] u32 {                       │
  │      let hi = read_cursor()#;                                           │
  │      let lo = read_cursor()#;                                           │
  │      (hi.as(u32) * 256u32) + lo.as(u32)                                 │
  │  }                                                                      │
  │                                  │                                      │
  │  ══════════════ [ 模块边界效应防火墙：内部安装 Handler ] ══════════════ │
  │                                  ▼                                      │
  │  -- 对外公开 API：在内部完整消去 #[read_cursor]，对外呈现纯 u32 签名    │
  │  pub fn decode_from_bytes(first: u8, second: u8) -> u32 {               │
  │      var step: u32 = 0u32;                                              │
  │      decode_pair()# {                                                   │
  │          read_cursor() => { ... }  -- 内部消化完毕                      │
  │      }                                                                  │
  │  }                                                                      │
  └──────────────────────────────────┬──────────────────────────────────────┘
                                     │ 暴露出的公开签名：fn(u8, u8) -> u32 (零效应!)
                                     ▼
  ┌─ 外部调用方 / 甚至 C 语言 FFI (`^attrs.abi(.c)`) ───────────────────────┐
  │  完全不知道 read_cursor 的存在，像调用普通纯函数一样直接拿结果          │
  └─────────────────────────────────────────────────────────────────────────┘
```

### 内部子模块协作 + 公开边界消去 + C ABI 零效应导出

让我们写一段结合了 `mod` 可见性、内部效应消去以及 `^attrs.abi(.c)` 跨语言导出的完整 Flurry 代码（参考 `visibility.md` 与 `effect-abi.md`）：

```flurry
use std.prelude.*;

mod protocol_codec {
    -- 1. 声明仅在 protocol_codec 及其子模块内可见的私有效应
    private effect pull_byte() -> u32;

    -- 子模块 worker 可以合法访问父模块的 private 声明 pull_byte。
    mod worker {
        pub fn read_header_sum() -> #[pull_byte] u32 {
            let magic = pull_byte()#;
            let version = pull_byte()#;
            magic + version
        }
    }

    -- 2. 模块公开入口：在边界内部安装 Handler 消去 #[pull_byte]。
    --    外部调用者看到的返回类型是干净的 u32，没有任何内部效应痕迹。
    pub fn parse_header(b0: u32, b1: u32) -> u32 {
        var idx: u32 = 0u32;
        worker.read_header_sum()# {
            pull_byte() => {
                idx += 1u32;
                if idx == 1u32 { b0 } else { b1 }
            },
        }
    }
}

-- 3. 甚至可以直接封装为 C ABI 导出函数（C ABI 要求效应行必须为空。）
^attrs.abi(.c)
pub fn exported_parse_header() -> u32 {
    protocol_codec.parse_header(40u32, 2u32)
}

test {
    println("header sum: {}", exported_parse_header());
}
```

---

## 三、 效应抽象转换（Effect Translation）与越权导出红线

除了在模块边界把内部效应消去变成“零效应”之外，还有一种常见的高级架构模式：**效应抽象层级转换（Effect Layer Translation）**。

比如：
- 底层存储子模块发出的是底层物理效应 `#RawBlockRead`；
- 中间业务模块拦截并处理了 `#RawBlockRead`，在它的 Handler 分支内部转换成了向更外层发出的高层效应 `#HighLevelAuditLog`。
- 此时，正如 `docs/algebraic-effect-and-delimited-continuation/handling-capability.md` 强调的：**内层分支虽然消去了 `#RawBlockRead`，但它自己发出的外层效应需求 `#HighLevelAuditLog` 必须诚实地保留在中间函数的签名上。**

### 隐藏实现模块路径（`private mod` + `pub use`）与拦截越权公开

同时，如果我们想把公开效应组织在私有的内部文件结构 `private mod internal_defs` 里，再在包根部统一导出，可以使用 `pub use`——但前提是**原始效应声明本身必须是 `pub`**。企图通过 `use ... as ...` 别名或 `pub use` 把一个原本只有包内可见或 `private` 可见的效应强行公开给外部，会被编译器静态报错：

```flurry
mod storage_pkg {
    -- 隐藏内部模块组织路径，但原始效应声明本身标为 pub
    private mod defs {
        pub effect audit_log(code: u32);
        -- 默认修饰符：仅限包内可见的内部效应
        effect raw_disk_seek(offset: u32) -> u32;
    }

    -- ✅ 合法再导出：原始声明 audit_log 是 pub，可通过 pub use 绕开 private mod 暴露给包外。
    pub use defs.audit_log;

    -- 负例 1：企图通过别名 + pub use 扩大并非 pub 原始声明的可见性上限
    use defs.raw_disk_seek as internal_seek;
    -- ❌ 编译期直接报错：(Visibility Escalation Error)
    -- raw_disk_seek 的原始声明只有包内可见许可，增加别名层数禁止将其升级为 pub。
    pub use internal_seek as public_disk_seek;
}

mod external_client {
    -- 负例 2：兄弟模块企图直接偷用 protocol_codec 内部的 private 效应
    -- ❌ 编译期直接报错。
    -- pull_byte 在 protocol_codec 中声明为 private，仅对其自身及子模块可见。
    use .protocol_codec.pull_byte;
}
```

还有一条跨语言边界红线（见 `effect-abi.md`）：**带有 `^attrs.abi(.c)` 的外部 ABI 入口函数，其效应行必须为空**。如果你忘记在 Flurry 包装层内部用 `# { ... }` 把效应消去，直接让一个 `^attrs.abi(.c)` 函数携带 `#some_effect` 签名，编译器同样会直接报错。

---

## 本篇小结

| 架构层次 / 机制 | Flurry 的可见性与效应规则 | 工程收益与安全边界 |
| :--- | :--- | :--- |
| **`private effect`** | 仅所属模块及子模块可见 | 将游标、回溯等微效应锁死在模块内部，外部零感知 |
| **默认包内可见 `effect`** | 同一个 `package` 内各模块直接协作 | 包内子系统无需繁琐标注 `pub`，同时天然防止向包外泄漏 |
| **`private mod` + `pub use`** | 原始声明为 `pub` 时可通过 `pub use` 导出 | 隐藏库内部目录拆分细节，提供干净扁平的顶层公开效应入口 |
| **边界消去与 `^attrs.abi(.c)`** | 在公开 API 或 FFI 包装层内 `# { ... }` 消去内部效应 | 对外交付纯净的无效应签名，安全对接外部调用者与 C ABI |

---

### 下一篇预告

到现在为止，我们的效应 Handler 并非但功能强大，而且模块边界清晰。

但是在高可靠系统（如金融账本、航空飞控或内核调度器）中，我们还有一个更高的追求：
当一个效应函数（如 `fn keep(flag: bool) -> result: #poll bool`）或一个有状态的 Handler 在运行期不断拦截请求、修改状态并恢复计算时，**我们怎么用数学级别的程序契约（`requires`、`ensures`、`outcomes`）和 `verified fn` 形式化验证，去证明效应完成后的结果一定满足后置条件？**
- 当函数返回类型是 `result: #poll bool` 时，`ensures` 契约里的 `result` 指的到底是 `poll()` 效应的返回值，还是整个计算正常完成后的最终返回值？
- 在经历了一次 `poll()#` 外部效应调用后，函数内部哪些局部变量的历史事实依然稳固成立，哪些暴露出去的指针或可变状态事实必须**自动失效（Fact Invalidation）**？

下一篇 **《Flurry 演进手记 (168)：用 `verified` 契约验证效应 Handler 的状态不变量》**，我们将结合 `docs/functions/contracts-and-effects.md` 与 `contracts-and-verification.md`，打通程序验证与代数效应的任督二脉。
