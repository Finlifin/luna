# Flurry 语言文档

Flurry 是一门面向系统级编程的语言，在正确性、性能、抽象能力、开发体验与语义健全性之间寻求统一。本文档集从八个核心维度组织 Flurry 的设计与用法；各维度下的详细内容仍在逐步完善中。

→ [语言愿景与定位](introduction.md)

---

## 基础语言

| 文档 | 内容 |
|------|------|
| [词法结构](basics/lexical.md) | 标识符、关键字、运算符空格规则、注释、分号省略 |
| [字面量](basics/literals.md) | 整数、浮点、字符、字符串、布尔、符号；后缀与 comptime 类型 |
| [单元 / null / undefined](basics/unit-null-undefined.md) | `()`、`null`、`_`、`undefined`（非字面量） |
| [基本数据类型](basics/primitives.md) | 整数、浮点、bool、char、void、指针、编译期类型 |
| [符号](basics/symbols.md) | 入口重定向 → 字面量章「符号」 |
| [变量绑定](memory-management/bindings.md) | `let`、`const`、作用域与遮蔽 |
| [控制流](control-flow/index.md) | 总览：基本控制流与模式匹配 |
| [基本控制流](control-flow/basic.md) | `if`/`when`、`for`/`while`、标签、`break`/`continue`/`return` |
| [模式匹配](control-flow/pattern-matching/index.md) | 模式语法、`match`/`if is`/`while is`、穷尽性 |
| [位向量模式](control-flow/pattern-matching/bitvec.md) | `BitVec` 分段匹配、base type、跨元素 |
| [函数](functions/index.md) | 定义、方法、各种参数种类、返回值、错误处理（`!`） |
| [Trait 系统](traits/index.md) | `trait`、`impl`、`derive`、`extend`、动态多态、`using` |

---

## 八个核心维度

### 1. 编译时计算（Comptime）

编译时计算确定值、类型和代码结构。normalization 依据操作契约检查定义；instantiation 代入具体输入，继续求值与检查。

| 文档 | 内容 |
|------|------|
| [编译时计算总览](comptime/index.md) | 心智模型与阅读路线 |
| [Normalization 与 instantiation](comptime/normalization-and-instantiation.md) | ordinary/loose、PTC、defer、query 图与执行器 |
| [Comptime 函数、纯性与范式](comptime/purity-and-normal-form.md) | 编译时函数、子句参数、默认值、pure 与缓存 |
| [Comptime 块与 inline 控制流](comptime/inlining-comptime-code.md) | 分支检查、编译时选择、展开与 runtime 主体 |
| [跨阶段值传递](comptime/quoting-and-splicing.md) | Splice、proxy、quote/lift 边界 |
| [Trait 两相](comptime/the-comptime-phases-of-traits.md) | 方法阶段、共享 coherence、Eq/Hash 与 Splice |
| [静态形状的张量加法](comptime/static-shaped-tensors.md) | 形状广播与分阶段执行案例 |
| [属性系统](comptime/attribute.md) | 编译时元数据 |
| [编译流水线](compilation/phases.md) | 阶段输入/输出与实现边界 |

### 2. 类型系统

类型可以参与编译时计算；子句参数、操作契约和类型约束支持定义端检查。

| 文档 | 内容 |
|------|------|
| [编译时计算与类型](type/boostrap.md) | 类型主题与 comptime 各章的阅读入口 |
| [精化类型](type/refined-type.md) | 带谓词约束的类型 |
| [闭包类型](type/qualified-types/closure-type.md) | 闭包与限定类型 |

→ [type/](type/)（含设计备忘 `prompt.fl`）

### 3. 性能与零开销抽象

抽象通过编译期特化消除运行时代价。

| 文档 | 内容 |
|------|------|
| [Once / Clone / Copy](memory-management/once-clone-copy.md) | 仿射类型、移动语义、复制语义 |
| [指针与引用](memory-management/pointers.md) | `*T`、`Slice<T>`、可达性安全 |
| [Drop 精化](memory-management/drop-elaboration.md) | RAII、自动资源释放、Drop Trait |

### 4. 元编程

模板宏替换 token；构造宏由编译时程序处理输入并生成代码。

| 文档 | 内容 |
|------|------|
| [模板宏](macro/template-macro.md) | 文件内捕获、替换与按组展开 |
| [构造宏](macro/construction-macro.md) | AST、PEG、token 与原始文本输入 |
| [属性系统](comptime/attribute.md) | 属性 Object、configuration 与读取 |

### 5. 代数效应

代数效应与限定续延为异步、状态、错误、生成器等提供统一语义。

| 文档 | 内容 |
|------|------|
| [代数效应与限定续延](algebraic-effect-and-delimited-continuation/index.md) | `effect`、`#`、处理器、续延、预制处理器、异步效应 |

→ [concurrency/](concurrency/)（含设计备忘 `prompt.fl`）

### 6. 安全等级（`normal` · `unsafe` · `verified`）

| 文档 | 内容 |
|------|------|
| [安全等级](safety/index.md) | 三个等级的语义、调用规则、安全哲学 |

### 7. 流水线映射

源码经 AST → HIR → Normalization → Instantiation → Codegen 逐步 lowering。

→ [compilation/phases.md](compilation/phases.md)

### 8. 模式匹配

模式是一等公民，支持结构解构、位向量、自定义 pattern view 等扩展方向。

→ [control-flow/pattern-matching/index.md](control-flow/pattern-matching/index.md)

---

## 参考

- [grammar/syntax](grammar/syntax) — 语法定义草稿
- 各目录下的 `prompt.fl` — 设计备忘与示例片段（非正式规范）
- [docs/OLD-2024/](OLD-2024/SUMMARY.md) — 2024 年旧版设计文档（供参考，部分内容已过时）
