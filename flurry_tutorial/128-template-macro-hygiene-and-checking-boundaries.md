# Flurry 演进手记 (128)：模板宏的边界与卫生性——为什么模板宏先接回调用位置再做语法检查

在上一回（127）中，我们初识了 Flurry 最轻量的元编程工具——**模板宏（Template Macro，`define`）**。只需寥寥几行 `define constant_function 2 { fn $0() -> i32 { $1 } }`，就能用 `$constant_function one 1` 快速消除文件内的机械重复代码。

但是，任何经历过 C/C++ `#define` 宏地狱的系统程序员，心里都会本能地拉响警报：
1. 如果我在模板宏的替换体里把 `$0` 或 `$value` 写了两次，传入一个带副作用的函数调用 `produce_number()`，它会求值一次还是两次？
2. 如果按组重复的模板宏 `define constants 2 ...` 在调用时少传了一个 Token（比如传了 5 个而不是 6 个 Token），或者忘记写结尾的 `$`，会发生什么？
3. 为什么 Flurry 的语法规范（`docs/grammar/syntax`）专门强调：**“token macros；先接回调用位置，再按该位置的普通语法检查”**？

今天，我们就来深入剖析模板宏的词法展开模型、参数重复求值陷阱、分组对齐边界，以及何时必须果断停手、升级到 `quote fn` 或构造宏！

---

## 一、 破局与痛点：把“Token 替换”误当成“函数调用”的惨剧

假设我们在编写网络协议状态机时，想写一个快速求值两次并相加的辅助工具。如果一个习惯了 `quote fn` 的同学顺手用模板宏写下了 `v0`：

```flurry
-- ❌ v0 灾难现场：把模板宏当成带参数求值的函数来用！
define bad_twice(x) {
    $x + $x
}

fn tick_counter(counter: *u32) -> u32 {
    counter.* += 1;
    return counter.*;
}

test "template macro double evaluation trap" {
    var count: u32 = 0;
    -- 调用者以为 tick_counter 只会执行 1 次，结果返回 1 + 1 = 2；
    -- 实际上展开后变成了：tick_counter(count.ref) + tick_counter(count.ref)！
    -- counter 被偷偷加了 2 次，算出了 1 + 2 = 3！
    let result = $bad_twice(tick_counter(count.ref));
    assert(count == 2);
    assert(result == 3);
}
```

### 1.1 为什么模板宏不做“一次求值绑定”？

因为模板宏的设计定位就是**词法阶段（Lexical Stage）的纯 Token 序列搬运工**！
在 Flurry 编译器内部（`docs/macro/template-macro.md`），模板宏的替换算法极其纯粹：

```text
substitute(body, captured):
    output = empty_token_buffer()
    for part in body:
        if part is a parameter_reference:
            append(output, captured[part.parameter])
        else:
            append(output, part)
    return output
```

请看这张展开流水线对比图：

```
+-------------------------------------------------------------------------+
|               模板宏 ($id)  vs  Quote 函数 (quote fn)                    |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ 模板宏：$bad_twice(tick_counter(c)) ]                                 |
|  1. 词法捕获：捕获括号内的原始 Token 序列 `tick_counter ( c )`            |
|  2. 机械替换：凡是出现 `$x` 的地方，原样粘贴一遍 Token 序列               |
|  3. 接回原位：把 `tick_counter(c) + tick_counter(c)` 塞回调用点          |
|  4. 语法与类型检查：按展开后的两遍调用进行语法解析、类型与资源检查！      |
|                                                                         |
|  [ Quote 函数：quote fn good_twice(x: u32) -> u32 { x + x } ]           |
|  1. 语法与类型检查：定义期即完成 AST/HIR 与类型检查                       |
|  2. 实参一次求值：调用点先求值一次 `let fresh_x = tick_counter(c)`       |
|  3. 卫生展开：在逻辑完成边界内计算 `fresh_x + fresh_x`，绝不重算实参！    |
+-------------------------------------------------------------------------+
```

> **💡 核心心智模型：捕获 Token 绝不等于捕获表达式的运行期结果！**
> 在模板宏中，同一个参数 `$0` 或 `$value` 在替换体中出现 $K$ 次，就会把对应的 Token 序列原封不动地插入 $K$ 次。编译器**绝不会**因为两段代码来自同一个宏参数，就替你把它们合并成一次求值！如果你需要“普通实参严格求值一次 + 卫生局部变量”，请立刻放下 `define`，改用 **`quote fn`**！

---

## 二、 `v1` 深入机制：为什么“先接回调用位置，再做普通语法检查”？

初学者常问：“既然模板宏这么简单，为什么它能生成像半截声明、函数头或者一串 `const` 这样在普通函数里根本写不出来的东西？”

答案就在于它的展开时机：**先接回调用位置，再做语法与语义检查**！
- 在 `define` 定义内部，花括号 `{ token* }` 装的只是**尚未经过语法树（AST）解析的 Token 序列**。
- 当你在调用处写下 `$constant_function one 1` 时，词法展开器把 `$0` 换成 `one`、`$1` 换成 `1`，拼出 `fn one() -> i32 { 1 }`，然后**直接放回调用所在的文件作用域位置**。
- 接下来，Flurry 的语法分析器（Parser）、名称解析器（Name Resolver）、类型检查器（Elaborator）和资源检查器才会登场，像检查手写代码一样严格检查展开后的结果！

让我们看看模板宏的三种标准捕获形态在 `v1` 中的正确工程用法：

```flurry
-- 形态 1：固定 Token 个数捕获 `define id N { ... }`
-- 每次严格捕获紧随其后的 2 个单 Token（$0 和 $1，从 0 开始编号）
define constant_function 2 {
    fn $0() -> i32 {
        return $1;
    }
}

$constant_function one 1
$constant_function two 2

-- 形态 2：按组重复捕获 `define id N ... { ... }`
-- 将结束标记 `$` 之前的 Token 流按每 2 个一组切分，逐组代入并拼接
define constants 2 ... {
    const $0: i32 = $1;
}

$constants FIRST 10 SECOND 20 THIRD 30 $

-- 形态 3：命名参数捕获 `define id(name*) { ... }`
-- 圆括号内可传入包含多个 Token 的子序列（如 `1 + 2`），以 `$value` 引用
define assert_positive(value) {
    assert(($value) > 0);
}

test "three template macro forms v1" {
    assert(one() == 1);
    assert(two() == 2);
    assert(FIRST + SECOND + THIRD == 60);
    $assert_positive(1 + 2)
}
```

---

## 三、 `v2` 进阶：模板宏的四大硬边界与编译期拒绝例

正因为模板宏工作在词法 Token 层面，同时又必须把结果交回语言前端做完整检查，它有着四条绝对不能逾越的硬边界：

### 边界 1：文件内顺序可见性（File-Local & Source-Order）
模板宏**只在当前源文件内有效**，且必须**先按源码顺序定义、后使用**。你不能用 `use` 把另一个文件的 `define` 模板宏导进来（跨文件宏必须用下一回的构造宏 `@define_macro1/2` 和 `@use`）。

### 边界 2：捕获对齐与参数越界检查
- 在 `define id N` 中，替换体只能引用 `$0` 到 `$N-1`。
- 在 `define id N ...` 中，调用点直到结尾 `$` 之间的 Token 总数**必须能被 $N$ 整除**，且决不能漏写结尾的 `$`！一旦输入不足或分组不齐，编译器立即报错，绝不会用“未初始化的空 Token”继续糊弄展开：

```flurry
define pairs 2 ... {
    const $0: i32 = $1;
}

-- ❌ 编译期直接拒绝！按每组 2 个 Token 切分，但结束符 `$` 前只有 3 个 Token（A, 1, B），无法完成分组！
$pairs A 1 B $
```

### 边界 3：不提供自动词法卫生隔离（No Automatic Binding Hygiene）
与拥有独立绑定身份和上下文接口 `< Ψ |- T >` 的 CMTT 不同，模板宏只是把 Token 塞回调用位置。
- 如果模板宏展开体里引用了一个名字 `helper`，它会在**调用位置的作用域**里查找 `helper`；
- 如果模板宏展开体在同一个块里定义了局部变量 `let tmp = 1;`，而在同一作用域连调两次该宏，或者外层后续代码误依赖了它，都会直接按展开后的普通作用域规则碰撞或暴露！
- 更重要的是，**模板宏绝不是法外之地**：展开后的代码如果违反了类型匹配、移动后使用（Use-After-Move）或未处理错误，编译器会结合**调用位置、宏定义位置与参数来源**给出完整的展开链诊断！

```flurry
define declare_temp(val) {
    let leaked_name: u32 = $val;
}

-- ❌ 编译期直接拒绝！展开后的代码同样接受严格的类型检查，无法将 bool 赋给 u32
fn reject_type_error_after_expansion() {
    $declare_temp(true)
}
```

---

## 四、 本篇小结：什么时候用 `define`，什么时候果断升级？

| 检查维度 | `define` 模板宏 | `quote fn` / CMTT | 构造宏 (`@define_macro1/2`) |
| :--- | :--- | :--- | :--- |
| **工作阶段与粒度** | 词法阶段，纯 Token 序列替换 | Elaboration / NIR 阶段，带类型与上下文的代码值 | 语法解析阶段，AST / PEG / TokenBuffer 转换 |
| **可见性范围** | **仅限当前文件内**，先定义后使用 | 遵循标准模块可见性（可 `pub` 跨模块导入） | 跨文件/跨包准备，通过 `@use path` 导入 |
| **实参求值次数** | `$x` 出现几次就粘贴几次（可能重复求值！） | 普通运行期实参**严格求值一次** | 由宏生成的目标代码结构决定 |
| **局部绑定卫生性** | ❌ 直接拼回调用处作用域，无自动重命名隔离 | ✅ 自动分配新鲜局部绑定身份，接口显式声明 `< Ψ \|- T >` | 需由宏作者管理生成的 `TokenBuffer` 符号 |
| **最佳适用场景** | 文件内寥寥数行的轻量重复声明（如成批生成常量表、简单包装） | 绝大多数需要内联展开、保类型安全与资源安全的参数化代码 | 跨文件语法扩展、对 AST 做结构化改写或嵌入自定义 DSL |

---

## 五、 留给下一篇的悬念

既然 `define` 模板宏只能活在单个源文件里，而且它是个“瞎子”——它只看得见扁平的 Token，根本分不清传进来的是一个模式（Pattern）、一个表达式（Expression）还是一个结构体声明！

如果我们要写一个**跨整个工程复用的宏**（比如上一卷状态机里用到的多模式组合器 `@alternatives("yes", "Y")`），要求：
1. 可以通过 `@use` 在任何源文件里导入；
2. 编译器和 IDE 在解析调用点 `@alternatives("yes", "Y")` 时，就明确知道括号里的参数必须按 **Flurry 模式（`FlurryPattern`）** 语法规则来解析；
3. 宏函数能在编译期拿到结构化的 AST 节点列表，用 `fold` 把它们拼装成一个新的 `TokenBuffer`（`{{ $acc or $pattern_ast }}`）返回！

下一回 **《Flurry 演进手记 (129)：处理结构化 AST 输入——构造宏 @define_macro1 与 {{ ... }} Token Buffer》**，我们将正式解锁 Flurry 强大的四级构造宏体系！
