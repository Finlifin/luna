# 基本控制流

## Intro

控制流决定语句与表达式如何分支、重复与退出。Flurry 里 **`if` 与 `when` 是表达式**（分支结果可以赋给绑定）；**`while` 与 `for` 是语句**。提前退出用 `return` / `break` / `continue`；需要「带条件才退出」时，在其后写 **`while cond`**（不用 `if`，以免与作表达式前缀的 `if`/`when` 在出错时缠成歧义）。

与模式相关的形态（`if … is … do`、后缀 `match`、`while … is … do`）表面语法见下文；模式本身见 [模式匹配](./pattern-matching/index.md)。`inline` 控制流见 comptime 章。

## 语法

```text
if_expr       -> if expr block (else (block | if_expr))?
when_expr     -> when { when_arm* }
when_arm      -> expr => (block | statement)
               | else => (block | statement)

for_stmt      -> for label? pattern in expr block
while_stmt    -> while label? expr block
label         -> : id

while_is      -> while label? expr is pattern do block
while_is_arms -> while label? expr is do { case_arm* }

if_is         -> if expr is pattern do block (else (block | if_is))?
if_is_arms    -> if expr is do { case_arm* }
post_match    -> expr match { case_arm* }

case_arm      -> pattern => (block | statement)

return_stmt   -> return expr? (while expr)?
break_stmt    -> break id? (while expr)?
continue_stmt -> continue id? (while expr)?
```

列表分隔符见 [语法风格：分隔符](../grammar/index.md)：臂与臂之间为逗号；块内语句之间为分号（**仅当该项最后一个 token 为 `}` 时**省略其后分号）。

## 正式定义

### `if` / `else`

- 条件为布尔表达式；为真执行 then 块，否则走 `else`（若有）。
- 作为表达式时，各写出的分支结果类型须一致（可统一）。
- **无 `else` 的 `if` 表达式**：合法，正常完成时类型为 `void`（值 `()`）；静态确定所选分支不正常完成时为 `never`。
- `else if` 即 `else` 后接另一个 `if`。

常量布尔条件可以保留确定退出的信息。例如 `if true { return true; }` 不产生正常完成值，后面不需要补返回表达式。布尔 `and` / `or` 遵循短路规则：`true and do { return true; }` 确定退出，`false and do { return true; }` 则正常完成为 false；未知左值不能据此推断确定退出。BitVec 的按位 `and` / `or` 求值两侧，不采用布尔短路规则。未选中的分支仍需通过类型检查。

### `when`

- 按书写顺序求值各臂左侧的布尔表达式；执行**第一个**为真的臂。
- 兜底臂写作 `else => …`。
- 作为表达式时，各臂结果类型须一致。
- **无 `else` 的 `when` 表达式**：合法，正常完成时类型为 `void`（与无 `else` 的 `if` 相同）。按条件顺序可以确定所有可达路径都退出时，其类型为 `never`；例如 `when { false => false, true => return true }`。前面的可达且正常完成的 arm 不会被后面的退出 arm 覆盖。

### `for` / `while`

- `for pattern in expr block`：迭代 `expr`；每次将当前元素匹配到 `pattern`。容器可实现 `IntoIterator` 以提供默认迭代方式；迭代器实现 `Iter`。
- `while expr block`：条件为真则重复执行块。
- 二者为**语句**。
- **标签**：`for:name` / `while:name`。`break name` / `continue name` 作用到该标签；无标签则作用到最内层循环。label 与普通名称共用[命名空间](../code-organization/scope-and-paths.md)，同名局部绑定可以遮蔽它；选中非标签绑定时静态拒绝，不跳过该绑定重新寻找标签。

当前实现中，普通 `for` 在 HIR 中保留 iterable、模式、守卫、循环身份及主体，尚未展开为可执行 NIR。有界 `Integer` 范围的 `inline for` 已接通展开及 comptime guard，参见[inline 控制流](../comptime/inlining-comptime-code.md)。主语在元素绑定作用域之外解析；元素绑定只进入守卫和循环体，标签按上述名称规则解析。AST lowering 不再生成 `__into_iter__`／`__next__` 等约定方法调用；迭代协议 evidence 与 inline effect 的展开仍待接通，NIR 入口对此明确报错。

当循环没有可达的正常出口时，其内部类型为 `never`；这不为 `while` 增加结果值或带值 `break`。例如，下面两个函数都不需要在循环之后补一个 `bool` 值：前者不终止，后者通过 `return` 返回。

```flurry
fn spin()->bool {
    while true {}
}

fn ready()->bool {
    while true {
        return true;
    }
}
```

编译器保守判断出口：字面量布尔条件及其 `not` / `and` / `or` 组合可以排除不可达分支；未知条件下的 `break` 仍视为可能出口。嵌套循环的 `break` 按目标标签区分，handler 中跳出安装者循环的 `break` 也属于该循环的出口。已知字面量及元组、结构体、枚举构造值的 `match` 也按 arm 顺序排除不匹配的纯模式；例如 `true match { true => continue, false => break }` 不为所在循环提供出口。模式中的可执行计算、未知主语或无法静态判定的模式保持保守处理，不能遗漏其中的控制转移。此分析不证明任意程序的终止性，也不免除循环内部的类型与资源检查。

```flurry
enum Signal { idle, ready(Tuple<bool, bool>) }

fn wait_forever() -> bool {
    while true {
        Signal.ready((true, false)) match {
            .ready((true, false)) => continue,
            _ => break,
        }
    }
}
```

这里可以确定每次均选中第一个 arm，循环没有正常出口，因此函数不需要补一个 bool 结果。若载荷改为未知输入，则保留退出的可能性。主语构造本身仍须求值，guard 中的控制转移也仍须检查；不能因为最终模式已知就跳过这两部分。当前实现仅判定有直接表示的字面量，未规范化的数值拼写和超出分析预算的结构仍按未知处理。

### `while … is … do`

- `while expr is pattern do block`：每次先对 `expr` 匹配；成功则执行 `block`，失败则结束循环。
- `while expr is do { case_arm* }`：多臂形式。

### `if … is … do` 与 `match`

- `if expr is pattern do block`：单模式；可接 `else`。
- `if expr is do { case_arm* }`：多臂；其前可加 `inline`（见 comptime 章）。也可退化成在 `do` 前只写一个 `pattern`。
- `expr match { case_arm* }`：同一套多臂匹配的后缀写法，方便链式书写。
- 穷尽性与其它模式规则见 [模式匹配](./pattern-matching/index.md)。

### `return` / `break` / `continue`

- `return` / `return expr`：从当前函数返回。
- `break` / `break label`：退出循环；**不带值**。
- `continue` / `continue label`：进入下一轮；**不带值**。
- 均可后缀 **`while cond`**：仅当 `cond` 为真时执行该转移（例：`return x while ready`）。

## 常见示例

```flurry
-- if 作表达式：分支值参与后续计算
fn letter_grade(score: i32) -> str {
    if score >= 90 {
        "A"
    } else if score >= 80 {
        "B"
    } else if score >= 60 {
        "C"
    } else {
        "F"
    }
}

-- 无 else：整个 if 表达式类型为 void，只做副作用
fn maybe_log(ready: bool) {
    if ready {
        println("ready");
    }
}

-- when：多条件并列，先匹配先执行；else 兜底
fn http_class(code: i32) -> str {
    when {
        code >= 200 and code < 300 => "success",
        code >= 300 and code < 400 => "redirect",
        code >= 400 and code < 500 => "client_error",
        code >= 500 and code < 600 => "server_error",
        else => "unknown",
    }
}

-- for / while：语句；标签精确 break / continue
fn find_cell(grid: List<List<i32>>, target: i32) -> ?Tuple<i32, i32> {
    for:row r in 0..grid.len() {
        for:col c in 0..grid.get(r).len() {
            continue col while grid.get(r).get(c) < 0;
            if grid.get(r).get(c) == target {
                return (r, c);  -- 返回类型为 ?Tuple 时，由类型侧接收非 null 载荷
            }
        }
    }
    null
}

fn drain_queue(q: *Queue<Event>) {
    while not q.is_empty() {
        let ev = q.pop();
        handle(ev);
    }
}

-- 语句守卫：条件为真才转移（关键字是 while，不是 if）
fn first_even(nums: List<i32>) -> ?i32 {
    for n in nums {
        return n while n % 2 == 0;
    }
    null
}

fn skip_noise(lines: List<str>) {
    for line in lines {
        continue while line.len() == 0;
        process_line(line);
    }
}

-- while is：匹配失败即结束循环（须 do）
fn sum_available(iter: *Iter<?i32>) -> i32 {
    let total = 0;
    while iter.next() is n? do {
        total = total + n;
    }
    total
}

-- if is / match：控制流形态；模式细节见模式匹配章
fn describe_shape(shape: Shape) -> str {
    shape match {
        .circle(r) => "circle",
        .rectangle(w, h) => "rectangle",
        .point => "point",
    }
}

fn use_circle_only(shape: Shape) {
    if shape is .circle(r) do {
        draw_circle(r);
    } else {
        eprintln("not a circle");
    }
}

fn dispatch(shape: Shape) {
    if shape is do {
        .circle(r) => draw_circle(r),
        .rectangle(w, h) => draw_rect(w, h),
        .point => draw_point(),
    }
}
```

（`List` / `Queue` / `Shape` / `log` 等为示意宿主；以标准库为准。）

## 与 Rust / Zig / C / C++ 的关键差异

| 点 | Flurry |
|----|--------|
| 分支 | `if` / `when` 为表达式；循环为语句 |
| 多条件 | `when { expr => …, else => … }`（臂之间逗号） |
| 条件退出 | `return` / `break` / `continue` 后接 `while cond` |
| 模式分支 | `if … is … do`（可 `inline`）与后缀 `match` 并存 |
| 无限循环 | `while true`；无 `loop` |
| `break` | 不带值 |

## 运行时实现设计引导

1. `if` / `when`：求条件（`when` 顺序短接），进入对应块；表达式形式将块结果写入目标；无 `else` 且作表达式时结果为 `()`。
2. `while` / `for`：条件或迭代与块组成循环；标签对应可跳转的出口与继续点。
3. `while is`：每轮先匹配；失败则离开。
4. `return` / `break` / `continue`：先求可选 `while` 条件，再转移；`break` / `continue` 无值通道。
5. 静态错误：表达式分支类型不一、无效标签、非循环上下文中的 `break` / `continue`。

## 编译器实现引导

1. 按上节产生式解析；`when` 臂为 `expr =>`；守卫为 `while`；`is` 后须有 `do`；列表按项类插入 `,` 或 `;`。
2. `if` / `when` 走表达式类型检查；`while` / `for` 走语句。
3. 对具名 `break` / `continue` 按统一名称规则解析，再检查选中项是可用的包围循环标签；无标签时选择最内层循环。
4. `if is do` 与 `match` 共享 case 臂与穷尽性检查入口（与模式匹配章衔接）。
5. 与 [`docs/grammar/syntax`](../grammar/syntax) 不一致处，以本章已确认规则为准并回灌 grammar。
