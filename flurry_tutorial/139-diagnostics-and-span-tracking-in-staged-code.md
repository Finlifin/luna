# Flurry 演进手记 (139)：诊断生成代码中的错误——源码溯源（Span Tracking）与两阶段报错定位

在上一回（138）中，我们见证了 CMTT 与第三卷资源管理系统（`Once`、`Drop`、可达性）的无缝合体，确信了“生成的代码绝不可能绕过移动和别名检查”。

但在真实的工程开发中，“编译器能抓出错误”只是及格线；**“编译器抓出错误后，能不能一针见血地指出到底是谁犯了错——是写代码生成器的库作者写错了模板，还是调用生成器的业务程序员传错了参数？”** 这才是决定一门元编程语言好不好用的生死线！

回想一下其他语言在元编程报错上的惨痛教训（`v0` 噩梦体验）：
- **C++ 模板的“万字天书”**：因为模板在定义期几乎不做语义检查，哪怕库作者在模板内部写错了一个字段名，也要等到用户实例化第 8 层模板时才轰然报错，打印出几百行内部实例化调用栈！
- **粗糙宏的“甩锅式报错”**：宏展开后丢失了每个 Token 的真实来源位置（Span），一旦出错，编译器只能指着最外层调用处的 `@my_macro(...)` 报一句笼统的“外围代码缺少标点或类型不匹配”，让你自己去盲猜宏展开后的第 9999 行内部变量 `__tmp_42` 到底是个啥！

今天，我们就来拆解 Flurry 编译器流水线（`docs/compilation/phases.md`、`docs/cmtt/index.md`、`docs/macro/construction-macro.md`）是如何通过 **Normalization / Instantiation 两阶段责任划分** 与 **多维源码来源链（Origin & Span Tracking）**，彻底终结元编程诊断噩梦的！

---

## 一、 核心基石：Normalization（定义期）vs Instantiation（实例化/接入期）的责任分明！

为什么 Flurry 的 CMTT 生成器、`quote fn` 和阶段化函数不会像 C++ 模板那样把所有错误都憋到最后才爆雷？
因为 Flurry 严格执行 **两阶段检查（Two-Phase Checking）**：

```
+-------------------------------------------------------------------------+
|               Flurry 元编程的两阶段错误定责模型                           |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ 第 1 阶段：Normalization（定义期检查 —— 抓“生成器作者”的 Bug！） ]      |
|  • 即使一个生成器 (`pure comptime fn`) 或 `quote fn` 从未被任何人调用，   |
|    编译器也会在定义期对它的所有分支（含未选中的 if/match/else 分支）       |
|    执行类型、阶段、单洞规则与可用契约检查！                               |
|  • 直接拦截：                                                            |
|    - 同一段 quote 内出现两个洞 `'(~<a> + ~<b>)`                          |
|    - 在 quote 内引用了既不在接口 `< Ψ |- T >` 也未绑定的幽灵名字          |
|    - 生成器分支漏返回或类型不兼容                                        |
|                                                                         |
|  ==================== [ 冻结边界 (Freeze Boundary) ] ===================  |
|                                                                         |
|  [ 第 2 阶段：Instantiation & Splice（接入期检查 —— 抓“调用者”的 Bug！） ]|
|  • 当调用者传入具体 `comptime` 参数并在目标位置执行 `~<...>` 接入时：      |
|  • 直接拦截：                                                            |
|    - 接入点未提供 `< needed: i32 |- i32 >` 所需的运行期绑定               |
|    - 两次接入对同一个运行期仿射资源 owner 造成重复消费（Use-After-Move）  |
|    - 代入具体 `Type` 后违反了 `where requires` 契约或 `defer` 类型要求    |
+-------------------------------------------------------------------------+
```

请仔细体会这套责任划分的威力：
**凡是生成器定义本身就不自洽的 Bug（哪怕写在某个从未被调用的冷门分支里），在库作者编译自己的代码时（Normalization 阶段）就会当场报错！** 绝不可能带着病溜进发布包去折磨下游用户！

---

## 二、 `v1` 实战：哪怕从未调用，未选中分支里的定义期错误也无处遁形！

在 `docs/cmtt/index.md` 中有一条非常硬核的规定：
**“未调用的生成器也检查定义，包括未选中的分支……即使从未调用，定义中的 else 也检查阶段与结果类型。”**

让我们对比一下正确的全分支自洽生成器与暗藏坏分支的生成器：

```flurry
-- ✅ 合法定义：哪怕 flag 为 false 的分支在下面的测试里没走到，
--    两个分支各自都满足 < x: i32 |- i32 > 接口与单洞契约，定义期顺利通过！
pure comptime fn build_transform(flag: bool) -> < x: i32 |- i32 > {
    if flag {
        return '(x + 1i32);
    } else {
        return '(x * 2i32);
    }
}

test "normalization checks all branches at definition time v1" {
    const code = build_transform(true);
    let res = ~[x = 20i32]<code>;
    assert(res == 21i32);
}
```

现在，假如库作者在 `else` 分支里手滑把返回类型写成了布尔表达式 `'(x > 0i32)`（与声明的 `< x: i32 |- i32 >` 不符），或者在 `else` 分支里偷偷写了两个洞 `'(~<a> + ~<a>)`，然后心存侥幸地想：“反正我只传 `flag = true`，`else` 分支永远不会跑，编译器应该发现不了吧？”

```flurry
-- ❌ 编译期（Normalization 定义阶段）直接拒绝！
-- 即使从未调用 broken_generator，或只计划传 flag = true，
-- 定义检查器依然会全面核验 else 分支，当场抓出 `< x: i32 |- bool >` 与 `< x: i32 |- i32 >` 不匹配！
pure comptime fn broken_generator(flag: bool) -> < x: i32 |- i32 > {
    if flag {
        return '(x + 1i32);
    } else {
        return '(x > 0i32);
    }
}
```

同样地，如果在同一个 Quote 里写了两个洞（违反单洞规则），编译器在定义期做线性节点计数时，诊断信息会**直接精准指向第二个接入点 `~<right>` 及其所属的 Quote 起始位置**，而不是等到展开后报一句莫名其妙的类型错误！

---

## 三、 多维源码溯源（Origin & Span Tracking）：拒绝笼统报错！

那么，当错误确实发生在展开或接入阶段（比如构造宏、`define` 模板宏或 `~<...>` 接入点）时，Flurry 编译器是怎么避免报出“第 9999 行内部临时变量出错”的呢？

在 Flurry 的 AST、HIR 与 IIR 中，每一个由元编程产生的节点都携带着一条结构化的**来源溯源链（Origin Chain）**，而不是只有一个扁平的文件行号：

```
  [ 生成节点的完整来源身份证 (Origin Chain) ]
  ┌─────────────────────────────────────────────────────────────────┐
  │ 1. 声明/模板来源 (Definition Span)：                             │
  │    代码片段是在哪个 quote / 哪个宏处理函数的第几行构造出来的？     │
  │ 2. 参数/插值来源 (Argument / Interpolation Span)：               │
  │    其中插入的 `$expr` 或绑定实参最初来自调用者源码的哪个表达式？   │
  │ 3. 展开/接入位置 (Call / Splice Site Span)：                     │
  │    最终是在哪个函数的哪一个 `~<...>` 或 `@macro(...)` 接入的？     │
  └─────────────────────────────────────────────────────────────────┘
```

### 3.1 构造宏与模板宏的四类边界定责

在 `docs/macro/construction-macro.md` 中，编译器明确规定了四类不同失败边界的精准归因——**“输入不符合规则、处理函数失败、输出类型不符或展开代码非法，应分别指明失败边界，不能统一报告成外围代码缺少标点”**：

| 失败发生阶段 | 真正的责任方 | 编译器诊断主锚点（Primary Span）与附注（Notes） |
| :--- | :--- | :--- |
| **1. 输入违反 AST / PEG 规则** | **宏调用者** | 主锚点指在 `@macro(...)` 内部写错的那个实参 Token；附注指向宏声明的参数规则或 `Entry` PEG 产生式 |
| **2. 宏处理函数求值失败 / 超预算** | **宏定义作者** | 主锚点指在宏处理函数内部触发断言失敗或耗尽步数的表达式；附注列出调用链 |
| **3. 插值转换失败 (`$expr`)** | **宏定义作者** | 主锚点指在 `{{ ... $bad_expr ... }}` 中不支持转换为 Token 的那个源表达式 |
| **4. 展开后代码违反外围类型/资源契约** | **双方关联溯源** | 主锚点指在生成片段中触发冲突的具体操作（关联回模板定义行），同时高亮调用处传入的实参来源！ |

---

## 四、 `v2` 进阶：追踪 CMTT 接入与 `quote fn` 契约的精准定位

让我们在 `v2` 中看一个结合了 `quote fn` 前后置契约（`requires` / `ensures`）与 CMTT 上下文替换的诊断定责案例（源自 `docs/cmtt/index.md`）。

在 `quote fn` 上，我们可以声明程序验证层的 `where requires ...`（前置条件）和 `ensures ...`（后置条件）：
- **后置条件 `ensures`**：代表 **`quote fn` 作者的承诺**！哪怕从未被调用，编译器在检查 `quote fn` 模板定义时也会核验其每条返回路径；
- **前置条件 `requires`**：代表 **调用者的义务**！当在调用点展开时，编译器会在调用处生成 `(require-predicate ...)` 检查义务，一旦调用者传入违反前置条件的参数，报错位置精准锚定在**调用处**！

```flurry
-- quote fn 带有明确的调用契约：
-- 调用者必须满足 requires value，模板自身保证 ensures result == not value'old
quote fn flip_true_only(value: bool) -> (result: bool)
where
    requires value,
    ensures result == not value'old
{
    return not value;
}

-- 同样，CMTT 上下文接口明确声明需要 `port: u16`
pure comptime fn make_port_check() -> < port: u16 |- bool > {
    return '(port == 443u16);
}

test "staged contract and span tracking v2" {
    -- 调用者传入 true，满足 requires value，顺利展开并返回 false
    let flipped = flip_true_only(true);
    assert(not flipped);

    -- 调用者显式供应 port = 443u16，上下文契约完整兑现
    let current_port: u16 = 443u16;
    const check_code = make_port_check();
    assert(~[port = current_port]<check_code>);
}
```

而一旦调用者在接入点 `~<check_code>` 漏供了 `port` 绑定，或者提供了类型错误的绑定，编译器会同时指出：
1. **接入点位置（Splice Span）**：这里缺少 `port: u16` 的绑定供应；
2. **代码接口来源（Interface Origin）**：该 `port: u16` 要求源自 `make_port_check` 返回类型 `< port: u16 |- bool >`！

```flurry
-- ❌ 编译期（Instantiation / Splice 接入阶段）直接拒绝！
-- 诊断精准定位到 `~<check_code>` 接入点：未能为接口 `< port: u16 |- bool >` 中的虚拟端口 `port` 提供运行期绑定！
fn reject_unsatisfied_splice_context() -> bool {
    const check_code = make_port_check();
    return ~<check_code>;
}
```

---

## 五、 本篇小结

| 编译阶段 / 机制 | 核心职责 | 拦截的典型错误 | 诊断溯源保障 |
| :--- | :--- | :--- | :--- |
| **Normalization（定义期）** | 按 ordinary/loose 规则检查未实例化的定义与全部分支 | 双洞 Quote、未声明绑定、分支类型不合、无效 `ensures` | 在库定义处直接报错，不把模板自身 Bug 甩给下游调用者 |
| **Freeze（冻结边界）** | 保存稳定绑定、契约与合法残余项，清理临时推导状态 | 非法冻结活跃运行期指针或未决错误 | 确保每次实例化都在干净独立的区域中进行 |
| **Instantiation / Splice** | 代入具体实参、完成 `defer` 计算、建立上下文替换并卫生展开 | 缺失上下文供应、接入后双重 Move、`requires` 违约 | 同时关联接入点 Span、实参来源 Span 与模板定义 Span |
| **IIR 验证器** | 检查类型 comptime 部分已全部完成、控制与环境交付一致 | 残留未完成的类型计算或悬空环境 | 每个诊断相关操作均保留完整 Origin 来源链 |

---

## 六、 留给下一篇的悬念

不知不觉间，我们已经走到了第六卷的倒数第二站！
回顾从第四卷（第 066 回）到第六卷（第 139 回）这波澜壮阔的 74 篇旅程：
- 我们从为什么拒绝 C 预处理器宏和 C++ 模板黑魔法出发；
- 走进了**双层类型理论（2LTT：$U_1$ 编译期宇宙与 $U_0$ 运行期宇宙）**；
- 打造了**两相 Trait 系统、`derive fn` 与精化/命题类型约束（PTC）**；
- 掌握了 **CMTT 的三类上下文代码值（表达式、模式、分支 Arm）、`quote fn`、`template fn`** 与**四级构造宏**；
- 最后领略了 **`BitVec`、组合子与张量代数的程序合成**！

在第六卷的收官之作中，我们将用一张宏大的全景架构图，把这 74 篇的所有核心概念串联成一座浑然天成的理论大厦，并为你揭开下一卷（第七卷）最令人神往的新篇章——**当控制流本身也变成可捕获、可恢复、可组合的一等公民：代数效应（`#Effects`）、限定 Continuation 与结构化并发**！

下一回 **《Flurry 演进手记 (140)：第六卷总结：从“写代码的人”进化为“教编译器写代码的人”》**，让我们共同见证这场“元编程即类型论”的华丽收官！
