# 模式匹配

## Intro

模式描述如何拆开、识别一个值。

若干模式与表达式形态成对出现：`pat?` 对偶于 option 的消除/传播，`pat!` 对偶于错误成功值一侧，`error pat` 对偶于 `error expr` 构造。多臂匹配须穷尽（或用 `_`）；单臂 `is` 不强制穷尽。自定义匹配视图用 **`case`**；控制流形态见 [基本控制流](../basic.md)。

## 语法

```text
-- 使用处（与 basic 一致）
if_is         -> if expr is pattern do block (else ...)?
if_is_arms    -> if expr is do { case_arm* }
post_match    -> expr match { case_arm* }
while_is      -> while label? expr is pattern do block
while_is_arms -> while label? expr is do { case_arm* }
case_arm      -> pattern => (block | statement)

let_decl      -> let pattern (: type_expr)? = expr (else block)?
var_decl      -> var pattern (: type_expr)? = expr (else block)?

-- 模式（局部；与 grammar 对齐的增量以确认后的为准）
pattern -> ... | pattern? | pattern! | error pattern
         | pattern or pattern
         | pattern and expr is pattern
         | pattern as id
         | pattern if expr
         | pattern (pattern*)
         | pattern { ... }
         | list / range / bitvec / symbol / literal / id / _

-- case 定义（常见于 struct / impl 体内）
case_def -> pub? case id (parameter*) -> expr clauses* block
```

`parameter` 常见为 `self` 或 `*self`。`clauses` 为 `where` 等子句。`case_arm*` 等列表的分隔符见 [语法风格：分隔符](../../grammar/index.md)（臂用逗号；语句用分号；**仅当项的最后 token 为 `}` 时**省略其后分号）。

## 正式定义

### 使用位置与穷尽性

| 位置 | 说明 |
|------|------|
| `if expr is pattern do block` | 单模式；失败可走 `else`；不强制穷尽 |
| `if expr is do { case_arm* }` | 多臂；须穷尽或含 `_` |
| `expr match { case_arm* }` | 同上，后缀写法 |
| `while … is … do` | 见基本控制流 |
| `let` / `var` | 左侧为模式（可解构）；可接 `else`（见下） |

### `let` / `var` 解构与 `else`

`let` 与 `var` 的左侧都是**模式**，因此可以直接解构：

```flurry
let (x, y) = position;
var { name, age } = student;
```

当模式**可能失败**（可反驳）时，可在末尾接 `else` 块，规则**类比 Rust 的 `let … else`**（非 Flurry 对 Rust 的承诺，仅锚定行为）：

- 写法：`let pattern = expr else block` / `var pattern = expr else block`
- 模式匹配成功：绑定引入后方作用域，继续执行。
- 匹配失败：进入 `else` 块；该块必须**发散**（如 `return` / `break` / `continue` 等，使控制流不再回到「使用这些绑定」的后方代码）。

不可反驳的模式（如单纯标识符、穷尽的元组拆开）不必写 `else`。

### 基本模式

- **字面量**：与值相等则匹配。
- **标识符**：绑定到该值。
- **`_`**：匹配且不绑定。
- **符号**：`.ok` 等，见字面量章。
- **元组 / 列表**：按位置拆开；列表可用 `...rest` 收尾。
- **结构体 / 记录**：`{ field: pat, … }` 或缩写字段。
- **应用**：`pat(pat*)`、`.variant(pat*)`、`.variant { … }` 等，匹配变体或可应用头。
- **范围**：`a..b`、`a..=b`、`..=b`、`a..` 等（与表达式范围形态对应的模式侧）。
- **`or`**：`p1 or p2`；两侧绑定结构须兼容。
- **`and is`**：`pat and expr is pat`（可连锁）；先完整匹配左侧，再对 `expr` 继续匹配。
- **`as`**：`pat as name`，匹配 `pat` 同时把整体绑到 `name`。
- **守卫**：`pat if cond`；`cond` 为真才成功（模式层用 `if`，与语句守卫 `while` 不同）。

### 与表达式对偶的模式

| 模式 | 对偶的表达式侧 |
|------|----------------|
| `pat?` | option 的消除 / 传播（`?`） |
| `pat!` | 错误限定类型成功值一侧的消除 / 传播（`!`） |
| `error pat` | 错误分支构造 `error expr` |

### 位向量模式

仅适用于 `BitVec`。见 [位向量模式](bitvec.md)。

### `case` 定义的自定义模式

在类型或 `impl` 体中声明：

```text
case name(self_or_ref_self, ...) -> ReturnType
where ...
{ ... }
```

- 参数一般含 `self` 或 `*self`。
- **返回类型写在签名上**；解构类 case 一般为 **option 包一层元组**（如 `?Tuple<...>`）：`null` 表示「本 case 不匹配」，有值则载荷供匹配臂绑定。
- 匹配侧写作 `.name` 或 `.name(pat*)`，与符号 / 变体风格一致。
- 未在模式中用到的字段，实现不必强制读取——这是用 `case` 做视图、而不是普通函数强行返回完整元组的原因之一。
- **纯度**与**可分析性**各有分级：本章**留空待定**（纯度细节见内存管理相关章；可分析性另订）。

## 常见示例

```flurry
-- 字面量 / 范围 / or / _
fn classify_score(score: i32) -> str {
    score match {
        100 => "perfect",
        90..=99 => "excellent",
        60..=89 => "pass",
        0 or 1 => "near_zero",
        _ => "other",
    }
}

-- 符号与枚举变体
fn status_message(status: Status) -> str {
    status match {
        .ok => "ok",
        .pending => "wait",
        .error => "fail",
    }
}

fn handle_msg(msg: Msg) {
    if msg is do {
        .quit => shutdown(),
        .write(text) => write_all(text),
        .move { x, y } => move_to(x, y),
    }
}

-- 结构体、列表、as、守卫
fn on_y_axis(point: Point) {
    if point is { x: 0, y } do {
        println("{}", y);
    }
}

fn split_head(items: List<i32>) {
    items match {
        [] => println("empty"),
        [only] => println("{}", only),
        [first, second, ...rest] => println("{}, {}, {}", first, second, rest),
    }
}

fn same_xy(point: Point) {
    if point is { x, y } if x == y do {
        println("diagonal");
    }
}

fn keep_move(msg: Msg) {
    if msg is .move { x, y } as whole do {
        record(whole);
        move_to(x, y);
    }
}

-- and is：链式继续匹配
fn enroll(student: Student, course: Course) {
    if student is { class_id: id? }
        and get_class(id) is class?
        and get_major(class.major_id) is major?
        if course.satisfied(major)
    do {
        accept(student, course);
    }
}

-- 与表达式对偶：? / ! / error
fn from_option(opt: ?i32) -> i32 {
    if opt is n? do {
        n
    } else {
        0
    }
}

fn from_result(result: !IoError i32) -> i32 {
    if result is v! do {
        v
    } else if result is error e do {
        eprintln("{}", e);
        0
    } else {
        0
    }
}

-- let / var：解构；可反驳模式可接 else（else 须发散）
fn unpack_pair(pair: Tuple<i32, i32>) -> i32 {
    let (x, y) = pair;
    x + y
}

fn require_adult(student: ?Student) -> str {
    let s? = student else {
        return "missing";
    };
    let .adult(display, _) = s else {
        return "minor";
    };
    display
}

fn mut_coords(start: Point) {
    var { x, y } = start;
    x = x + 1;
    y = y + 1;
    save(x, y);
}

-- case：自定义匹配视图（返回 ?Tuple；null 表示本 case 不匹配）
struct Student {
    name: str,
    age: usize,

    pub case adult(*self) -> ?Tuple<str, usize> {
        if self.age >= 18 {
            (self.name, self.age)
        } else {
            null
        }
    }
}

fn greet(student: Student) {
    student match {
        .adult(name, age) => greet_adult(name, age),
        _ => greet_minor(),
    }
}


```

（`Status` / `Msg` / `Point` / `Student` 等为示意；`?Tuple<>` 表示无元素载荷的 option，若标准库写法不同以库为准。谓词型 `case` 是否改用 `-> bool` 仍可选待定。）

## 与 Rust / Zig / C / C++ 的关键差异

| 点 | Flurry |
|----|--------|
| 多臂入口 | `if … is do { }` 与后缀 `match`；前者可 `inline` |
| 必须 `do` | 与 `expr { }` 类构式消歧 |
| `?` / `!` / `error` | 模式与表达式消除/构造对偶 |
| 自定义匹配 | `case` 声明视图，匹配侧 `.name` |
| `let` / `var` 解构与 `else` | 模式可反驳时接 `else`；规则类比 Rust `let … else` |

## 运行时实现设计引导

1. 将模式编译为检验与绑定序列；`or` 为选择，`and is` 为顺序依赖。
2. `?` / `!` / `error` 与对应类型的标签/载荷布局一致。
3. 多臂：按序尝试，穷尽性由静态检查保证；`_` 为收尾。
4. `case`：调用（或内联）定义体；`null` 则试下一臂；有值则按元组载荷绑定臂参数。未提及的字段可不上来。
5. 失败：非穷尽多臂、绑定形状不一致、无效位段 → 静态错误。

## 编译器实现引导

1. 模式 AST 与 grammar 前缀/中缀/后缀分层一致；确认后回灌 `docs/grammar/syntax`（含 `case_def`）。
2. `if is do` / `match` 共享臂类型检查与穷尽性分析。
3. `case`：解析签名上的返回类型与子句；按 option+元组约定检查体；匹配侧 `.name` 解析到 case 声明。
4. 与效应 / 纯度 / 可分析性分级的接口预留；**分级表本章不写**（待定）。
5. `let` / `var`：复用同一模式检查器；可反驳模式的 `else` 须检查发散。

---

## 纯度与可分析性（待定）

- **纯度**：分级存在；细则见内存管理相关章节。
- **可分析性**：分级存在；定义待定。
