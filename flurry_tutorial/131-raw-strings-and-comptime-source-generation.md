# Flurry 演进手记 (131)：原始文本宏的妙用——@builtin.str.raw { ... } 与编译期源码生成脚本

在上一回（130）中，我们用 `@define_macro2` 和 PEG 文法在 Flurry 里内嵌了自定义的 DSL。但无论是 `@define_macro1`、`@define_macro2` 还是 `^macro3`，它们都有一个共同的前提：**花括号 `{ ... }` 里面的内容，必须先通过 Flurry 的词法分析器（Lexer）切分成合法的 Flurry Token！**

这就引出了一个非常现实的工程问题：
如果我想在代码里直接嵌入一段正则表达式、一段包含奇特标点的硬件配置文件、一段包含反斜杠和未配对引号的文本模板，甚至里面到处都是 Flurry 自己的敏感字符 `$`、`@`、`^`——如果先过一遍常规词法分析器，词法分析器看到孤立的 `\` 或 `$` 就会当场报词法错误！

在其他语言里，设计者不得不去给字符串字面量发明复杂的定界符语法（比如 Rust 的 `r###"..."###`）。而在 Flurry 中，依靠构造宏的第 4 级输入模式（**原始文本输入 `^macro4`**），原始字符串根本不需要破坏字面量词法，它本身就是一个内置的原始文本构造宏——**`@builtin.str.raw { ... }`**！

今天，我们就来拆解 `@builtin.str.raw { ... }` 的底层原理，并顺着它走进 Flurry 的 **`scripts/` 构建脚本与编译期源码生成体系（`docs/code-organization/configuration-and-scripts.md`）**！

---

## 一、 破局与痛点：为什么普通字符串应付不了多行配置与代码模板？

我们先回顾一下 Flurry 普通双引号字符串 `"..."` 的规则（`docs/basics/literals.md`）：
1. **反斜杠转义**：`\\`、`\"`、`\n`、`\t`、`\$` 等需要反斜杠转义，任何未定义的 `\x` 序列都是静态词法错误。
2. **`$` 编译期插值**：在普通双引号字符串里，`$name` 或 `${expr}` 会自动触发编译期字符串插值（等价于调用 `std.fmt.comptime_format`）！虽然单独的 `{` 和 `}` 在普通字符串里只是普通字符，但只要你想写字面量美元符号 `$`，就必须转义写成 `\$`！

```flurry
const VERSION: str = "0.1.0";
-- 普通双引号字符串：自动将 $VERSION 插值为 "0.1.0"，字面量 $ 必须写成 \$
const BANNER: str = "Flurry $VERSION (cost \$5)";
```

当我们要在代码里塞入一大段包含 `$`、`@`、`^`、`\` 的传感器固件配置或正则表达式时（`v0` 痛点），满屏的 `\\` 和 `\$` 会让人瞬间眼花缭乱！

---

## 二、 `v1` 升级：`@builtin.str.raw { ... }` 为什么能绕过词法分析？

来看看构造宏的第 4 种输入模式（`mode == SOURCE`，对应 `^macro4`）：
当解析器识别到 `@builtin.str.raw { ... }` 时，它**不会**对花括号内部的区域运行常规的 Flurry 词法分词，而是直接根据平衡花括号范围提取**原始源码字符序列（`original_source_text(source_region)`）**！

```
+-------------------------------------------------------------------------+
|        ^macro3 (Token 输入)  vs  ^macro4 / @builtin.str.raw (原始文本)   |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ ^macro3 (mode == TOKENS) ]                                           |
|  源码区域 ──► Flurry 词法分析器 (Lexer) ──► TokenBuffer                  |
|  ⚠️ 限制：内部空白被归一化，且非法转义或孤立字符会在词法阶段直接报错！     |
|                                                                         |
|  [ ^macro4 / @builtin.str.raw (mode == SOURCE) ]                        |
|  源码区域 ──► 直接截取原始字符 (original_source_text) ──► 交付 `str`！   |
|  ✅ 优势：100% 保留原始空白、换行与字形，$ @ ^ \ 等特殊符号无需任何转义！  |
|  ★ 编译器铁律：绝不能先切成 Token 再拼回去冒充原始文本（那会丢失空白！）  |
+-------------------------------------------------------------------------+
```

让我们在 `v1` 中对比普通插值字符串与 `@builtin.str.raw { ... }` 的清爽体验：

```flurry
const VERSION: str = "0.1.0";
const BANNER: str = "Flurry $VERSION";

-- 使用 @builtin.str.raw 原样保留内部的换行、空格以及 $ @ ^ 等特殊标点！
const SAMPLE = @builtin.str.raw {
    name = sample
    description = punctuation: $ @ ^
}

const RINGS = @builtin.str.raw {
    Three rings for the Elven-kings under the sky
}

test "raw string macro and comptime string interpolation v1" {
    assert(BANNER == "Flurry 0.1.0");
    assert(SAMPLE != "");
    assert(RINGS != "");
}
```

> **🔍 语法小贴士：为什么 `@builtin.str.raw` 比 `r#"..."#` 更统一？**
> 因为在 Flurry 里，`@path { ... }` 本来就是块状构造宏的标准调用语法！把原始字符串设计成 `@builtin.str.raw { ... }`，不仅复用了统一的构造宏管线，而且只要内部的花括号 `{` 与 `}` 保持配对平衡，你在里面写多少个双引号 `"`、反斜杠 `\`、美元符 `$` 都完全不需要转义，也不会改变外围 Flurry 代码的语法！

---

## 三、 `v2` 进阶：当生成文本变成源码——`scripts/`、宏准备与构建依赖图

学会了用 `@builtin.str.raw` 和编译期函数构造大段文本后，一个更大的工程场景浮出水面：
在大型工程里，我们经常需要写**构建配置（`package.fl`）**与**构建脚本（`scripts/`）**——比如根据硬件描述文件在编译前生成源码文件（如 `pub struct Packet {}`），或者编译准备好前面学过的 `@alternatives` 构造宏供主源码 `src/` 使用！

这时候，很多同学会犯一个概念混淆的错误：**以为在某个 `comptime fn` 里返回了一段字符串 `"pub struct Packet {}"`，另一个源文件里就能自动多出一个 `Packet` 类型！**

### 3.1 铁律一：“生成文本”与“注册并纳入源码目标”是两码事！

在 Flurry 的构建模型（`docs/code-organization/configuration-and-scripts.md`）中，配置与脚本遵循严格的交付边界：

```
  [ 步骤 1：脚本计算生成文本 ]
  comptime fn packet_source() -> str {
      @builtin.str.raw { pub struct Packet {} }
  }
  （此时返回值仅仅是一份普通的 `str` 数据，绝不会凭空注入包根命名空间！）
         │
         ▼
  [ 步骤 2：构建任务将文本显式注册为目标的生成源码模块 (如 generated.packet) ]
  （由构建系统 anar 建立任务依赖边，将生成文件纳入目标源码集）
         │
         ▼
  [ 步骤 3：编译器前端 luna 解析并检查 generated.packet 模块 ]
  （生成源码必须完整通过语法、模块、可见性与类型检查！包根再通过
    `pub use generated.packet.Packet;` 暴露稳定入口）
```

让我们看看配置文件与源码生成脚本的标准配合写法：

```flurry
-- 1. 在配置或构建脚本中，可以用 pure comptime fn 抽取配置逻辑
pure comptime fn buffer_limit(debug: bool) -> u32 {
    if debug {
        return 8;
    } else {
        return 64;
    }
}

-- 2. 生成源码文本的辅助函数：返回 str，交由构建任务注册进 generated.packet 模块
comptime fn packet_source() -> str {
    @builtin.str.raw {
        pub struct Packet {
            pub id: u32,
        }
    }
}

test "configuration helper and source generation text v2" {
    assert(buffer_limit(true) == 8);
    assert(buffer_limit(false) == 64);
    assert(packet_source() != "");
}
```

当构建任务将 `packet_source()` 的产物注册为 `generated.packet` 模块并通过检查后，包根就可以干净利落地导出它：
```flurry
pub use generated.packet.Packet;
```

### 3.2 铁律二：宏准备与消费者解析的拓扑序，严禁自举死锁环！

为什么构造宏（如 `@alternatives`）必须放在依赖包或者本包先于 `src/` 编译的 `scripts/` 目录里，而不能跟使用它的函数随便混写在同一个正在被解析的普通文件里互相等？

请看这三条依赖链的生死对比（`docs/code-organization/configuration-and-scripts.md`）：

```
  ✅ 可完成的合法依赖链 1（构造宏准备）：
     工具依赖 ──► 宏脚本 (scripts/) ──► 交付宏接口 ──► 消费者源码解析 (@use) ──► 类型检查

  ✅ 可完成的合法依赖链 2（源码生成任务）：
     生成任务 (scripts/) ──► 注册生成源码 ──► 源码解析 ──► 类型检查

  ❌ 绝不可完成的死锁依赖环（自举环）：
     消费者文件 A 解析 ──► 依赖宏 M 准备 ──► 宏 M 的准备又要求先解析同一个消费者文件 A！
```

> **💡 核心心智模型：静态声明的循环引用 $\neq$ 构建准备任务的循环依赖！**
> 在普通 Flurry 代码里，两个函数 `is_even` 和 `is_odd` 可以互相调用，两个模块可以互相引用类型声明——因为它们的语法已经解析完毕，编译器只需要做静态符号求解。
> 但是！如果**解析文件 A 的语法**本身就需要先执行构造宏 M，而**准备构造宏 M** 又必须先解析完整的文件 A，那么每一步都在苦等尚未发生的下一步！这种准备依赖环在逻辑上是不可能完成的，构建系统会直接诊断并拒绝该循环依赖！

```flurry
-- ❌ 编译期直接拒绝！仅在脚本或局部调用返回源码文本的函数，不会自动在当前模块凭空产生 Packet 类型声明
fn reject_unregistered_generated_type() {
    let text = packet_source();
    let p = Packet { id: 1 };
}
```

---

## 四、 本篇小结

| 核心概念 | 语法 / 机制 | 关键契约与不变量 |
| :--- | :--- | :--- |
| **原始字符串宏** | `@builtin.str.raw { ... }` | 基于 `^macro4`（`mode == SOURCE`），在常规词法切分前直接截取平衡花括号内的原始字符 |
| **普通字符串插值** | `"Hello $NAME"` / `"${A}"` | 仅限嵌入 `comptime` 值，等价于 `std.fmt.comptime_format`；字面 `$` 需转义为 `\$` |
| **配置文件与脚本** | `package.fl` 与 `scripts/` | 使用 `comptime` Flurry 编写；发现脚本、编译脚本与执行脚本三步分离 |
| **生成源码纳入** | 生成 `str` $\rightarrow$ 注册模块 $\rightarrow$ 检查 | 返回文本不等于合法程序；必须显式注册进目标源码集并通过完整语言检查 |
| **宏准备依赖序** | 宏准备 $\rightarrow$ `@use` $\rightarrow$ 消费者解析 | 构造宏必须在消费者解析前就绪；严禁形成“解析 A $\rightarrow$ 准备宏 B $\rightarrow$ 解析 A”的死锁环 |

---

## 五、 留给下一篇的悬念

学到这里，请低头看看我们腰间挂满的“元编程兵器谱”：
1. 普通泛型函数与 **`inline fn`**；
2. 编译期计算函数 **`comptime fn` / `pure comptime fn`**；
3. 带上下文模态类型的代码生成 **`CMTT`（`'(...)`、`'<...>`、`quote fn`、`template fn`）**；
4. 词法模板宏 **`define`** 与四级构造宏 **`@define_macro1/2/3/4`**。

面对一个具体的工程任务时，很多同学难免会犯选择困难症：**“我手头这个需求，到底该写 `comptime fn`、`inline fn`、`quote fn`，还是直接上 `@define_macro1/2` 宏？”**

下一回 **《Flurry 演进手记 (132)：宏、Comptime 函数、inline fn 与 CMTT——四套元编程工具该如何选型？》**，我们将绘制一张一图胜千言的**“Flurry 元编程选型决策树”**，把这四套工具的职责边界锤炼成直觉！
