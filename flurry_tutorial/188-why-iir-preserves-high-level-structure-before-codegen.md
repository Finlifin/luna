# Flurry 演进手记 (188)：高阶控制流、闭包与双轨效应的协同法则——结构化循环、按需捕获与 `!Err` / `#Effect` 组合

在[上一篇（第 187 篇）](./187-fine-grained-incremental-queries-and-cycle-recovery.md)中，我们探讨了大型工程中的接口解耦与编译期依赖边界。

在真实的系统编程中，各种语言特性从来不是孤立出现的。一个几行的核心函数里，往往同时包含：
1. **带值返回的结构化循环（`loop { ... break expr; }`）**；
2. **捕获局部上下文的高阶闭包（`|value| value + base`）**；
3. **可恢复错误（`!Err`）与代数效应（`#[eff]`）的组合调用**。

今天，我们通过三段典型的代码，梳理这些高阶特性组合使用时的语义保证与零开销边界。

---

## 一、 结构化循环与带值 `break` 表达式

在 Flurry 中，`loop`、`while`、`for` 不仅是控制流语句，还可以作为表达式通过 `break expr;` 直接产生计算结果，同时严格保证循环内部局部资源的确定性析构（`Drop`）：

```flurry
fn even_by_loop(limit: u32) -> bool {
    var index: u32 = 0;
    var even = true;
    loop {
        if index == limit {
            break even;
        }
        index += 1;
        even = not even;
    }
}

test structured_loop_value {
    assumes even_by_loop(0) == true;
    assumes even_by_loop(3) == false;
    assumes even_by_loop(4) == true;
}
```

使用 `loop { ... break even; }` 的好处非常直观：
- 不需要在循环外声明未初始化的临时结果变量；
- 编译器能够直接验证每一条 `break` 分支都提供了类型一致的 `bool` 值，并在跳出循环的精确边界按逆序清理循环体内的局部资源。

---

## 二、 闭包捕获与高阶调用的零分配内联

再看一段将捕获了局部变量 `base` 的闭包传给高阶函数 `apply` 的代码：

```flurry
fn apply(step: %c fn(i32) -> i32, value: i32) -> i32
where
    c: Type.vars.Closure
{
    step(value)
}

fn show_shifted(base: i32) -> i32 {
    let shift = |value: i32| value + base;
    apply(shift, 5)
}

test closure_capture_and_apply {
    assumes show_shifted(10) == 15;
}
```

在 `show_shifted` 中：
- `shift` 捕获了外层参数 `base: i32`（`Copy` 标量）；
- `apply` 通过闭包环境限定参数 `%c fn(i32) -> i32` 精确承接了闭包携带的环境 `c`，无需将闭包装箱（Box）到堆内存上；
- 当高阶调用在编译期就地展开时，`apply(shift, 5)` 直接化简为 `5 + base`，与手写算术表达式具有完全相同的零分配性能。

---

## 三、 错误返回（`!Err`）与代数效应（`#[eff]`）的双轨协同

让我们再次巩固第七卷学过的核心原则：**错误结果（`!Err`）与代数效应（`#[eff]`）各司其职，拥有不同的语义与调用约定，且函数签名绝不添加 `handles` 子句。**

来看一段同时使用代数效应 `#[read_number]` 读取输入、并用错误类型 `!InputErr` 拦截负数的完整示例：

```flurry
effect read_number() -> i32;

enum InputErr {
    Negative(i32),
}

-- 同时声明代数效应 #[read_number] 与错误返回 !InputErr
fn read_non_negative() -> #[read_number] !InputErr i32 {
    let value = read_number()#;
    return error InputErr.Negative(value) while value < 0;
    value
}

-- 在外层用 # { ... } 消去 read_number 效应，保留 !InputErr 错误供上层处理
fn demo() -> !InputErr i32 {
    read_non_negative()# {
        read_number() => -3,
    }
}
```

观察这两条轨道在 `read_non_negative` 与 `demo` 中的分工：

```text
  ┌─────────────────────────────────────────────────────────────────────┐
  │ 轨道 A：代数效应 #[read_number]                                      │
  │   • 发起请求：read_number()#                                         │
  │   • 就地消去：read_non_negative()# { read_number() => -3 }           │
  │   • 语义特点：由外层处理器提供环境输入，直接返回 (-3) 零栈切换开销     │
  ├─────────────────────────────────────────────────────────────────────┤
  │ 轨道 B：错误结果 !InputErr                                           │
  │   • 短路返回：return error InputErr.Negative(value) while value < 0; │
  │   • 传播或消去：通过后缀 ! 传播，或通过 expr! { ... } 分支处理          │
  │   • 语义特点：单向短路退出当前函数，走紧凑的值返回通道，不含 resume 权 │
  └─────────────────────────────────────────────────────────────────────┘
```

在 `demo()` 中，因为 `read_number` 效应已经被 `# { ... }` 处理消去，所以 `demo` 的返回类型只剩下 `!InputErr i32`，对外完全隐藏了内部曾经使用过 `read_number` 效应的细节。

---

## 四、 拒绝例：编译期类型位置不能混入运行期条件变量

虽然运行期控制流（`if`、`loop`、`!Err`、`#Effect`）非常灵活，但正如我们在第 179 篇强调的：**所有类型的编译期部分（`comptime`）必须在编译期完全确定**，不能让运行期布尔条件流入纯编译期类型构造函数：

```flurry
pure comptime fn SelectIntType(flag: bool) -> Type {
    inline if flag { i32 } else { i64 }
}

fn bad_runtime_type_branch(x: i32) -> i32 {
    let runtime_flag: bool = x > 0;
    -- ❌ 编译期拒绝：SelectIntType 要求编译期已知的 bool 以确定变量的内存类型，
    -- 不能传入运行期才计算出的布尔变量 runtime_flag。
    let val: SelectIntType(runtime_flag) = x;
    val
}
```

---

## 五、 本篇小结

| 特性组合 | 推荐写法 | 语义与性能保障 |
| :--- | :--- | :--- |
| **结构化循环求值** | `loop { if cond { break val; } ... }` | 无需可变未初始化临时变量，跳出时自动按逆序执行局部 `Drop` |
| **栈上闭包传递** | `step: %c fn(A) -> B where c: Type.vars.Closure` | 显式保留闭包环境 `c`，零堆分配、支持就地内联 |
| **`#Effect` + `!Err` 协同** | `-> #[eff] !Err T`，用 `# { ... }` 消效应、用 `! { ... }` 消错误 | 尾恢复效应零切栈，错误返回走高效值通道，边界清晰正交 |

---

## 六、 下一篇预告

在系统编程、网络协议解析和硬件寄存器操作中，`BitVec<n>` 与 `'bitvec` 视图是我们最常打交道的工具之一。
- 当我们写下 `let snap = word'bitvec;` 和就地重叠写回 `word'bitvec[4..12] = word'bitvec[0..8];` 时，如何保证重叠区间 `[4..8]` 绝不会被边读边写地踩坏？
- 为什么 `BitVec<n>` 上的按位 `and` / `or` 与布尔类型 `bool` 上的短路 `and` / `or` 有着不同的求值契约？
- 什么时候把 `BitVec` 通过 `.as(TargetType)` 转换为目标类型是完全安全的，什么时候必须包裹在 `unsafe` 中并验证位模式有效性？

下一篇 **[Flurry 演进手记 (189)：`BitVec` 位级代数与安全边界全解——值快照、重叠写回、非短路求值与 `unsafe` 重解释](./189-implementing-verified-bitvec-synthesis-with-fallback.md)**，我们将作为转入终极实战前的压轴篇章，完整梳理 `BitVec` 的安全契约。
