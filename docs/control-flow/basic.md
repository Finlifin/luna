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
- **无 `else` 的 `if` 表达式**：合法，整个表达式的类型为 `void`（值 `()`）。
- `else if` 即 `else` 后接另一个 `if`。

### `when`

- 按书写顺序求值各臂左侧的布尔表达式；执行**第一个**为真的臂。
- 兜底臂写作 `else => …`。
- 作为表达式时，各臂结果类型须一致。
- **无 `else` 的 `when` 表达式**：合法，类型为 `void`（与无 `else` 的 `if` 相同）。

### `for` / `while`

- `for pattern in expr block`：迭代 `expr`；每次将当前元素匹配到 `pattern`。容器可实现 `IntoIterator` 以提供默认迭代方式；迭代器实现 `Iter`。
- `while expr block`：条件为真则重复执行块。
- 二者为**语句**。
- **标签**：`for:name` / `while:name`。`break name` / `continue name` 作用到该标签；无标签则作用到最内层循环。

### `while … is … do`

- `while expr is pattern do block`：每次先对 `expr` 匹配；成功则执行 `block`，失败则结束循环。
- `while expr is do { case_arm* }`：多臂形式。

### `if … is … do` 与 `match`

- `if expr is pattern do block`：单模式；可接 `else`。
- `if expr is do { case_arm* }`：多臂；其前可加 `inline`（见 comptime 章）。也可退化成在 `do` 前只写一个 `pattern`。
- `expr match { case_arm* }`：同一套多臂匹配的后缀写法，图书写方便。
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
3. 标签分辨到包围循环；解析 `break` / `continue` 目标。
4. `if is do` 与 `match` 共享 case 臂与穷尽性检查入口（与模式匹配章衔接）。
5. 与 [`docs/grammar/syntax`](../grammar/syntax) 不一致处，以本章已确认规则为准并回灌 grammar。
