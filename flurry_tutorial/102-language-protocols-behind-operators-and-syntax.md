# Flurry 演进手记 (102)：揭秘语法糖背后的正规军——算术、比较、索引与 LiteralInto 语言协议全览

> **系列导读**：在上一期（101）中，我们认识了守护跨阶段值传递的 `Splice` 与 `Eq`/`Hash` 协议。事实上，在 Flurry 中，Trait 并非止是供用户编写业务多态的工具，它更是**连接编译器表面语法与底层语义的唯一正规桥梁**。当你写下 `a + b`、`not flag`、`ptr.*`、`tensor[i]`、甚至自带物理单位的字面量 `32px` 以及错误恢复 `.use(handler)` 时，编译器素来免于搞任何“无从寻获就随便猜一个函数名”的黑盒机制，而是精准地将其转化为标准的 **Trait 语言协议（Language Protocols）** 查询。今天，我们将全面揭开这支语法糖背后的正规军。

---

## 一、为什么“鸭子类型运算符重载”是严重问题？

在 C++ 中，运算符重载和自定义字面量往往依赖复杂的 ADL（参数依赖查找）和重载决议：只要某个命名空间里有一个长得像 `operator+` 的函数，编译器就会把它卷进来，一旦遇到隐式类型转换，报错信息和实际选中的函数常常让人怀疑人生。

Flurry 从一开始就确立了一条核心规则：
> **所有可定制的语法操作（运算符、解引用、下标、字面量后缀、错误与效应处理器）统一通过绑定于编译环境的名义 Trait 协议连接。**
> 语法本身负责规定**求值顺序、控制流与短路规则**；而 Trait 实现负责提供**具体的计算入口与关联结果类型**。

### Flurry 语言协议全景映射图

```text
================================================================================
                    Flurry 表面语法 ──► 底层 Trait 协议全景图
================================================================================
  [ 二元算术 / 比较 ]   lhs + rhs           ──► Add<.Rhs = RhsType>.add(lhs, rhs)
  [ 一元取负 / 逻辑并非 ] -val / not val      ──► Neg.neg(val) / Not.invert(val)
  [ 指针解引用 / 取址 ] expr.* / expr.ref   ──► Deref / Reference 协议
  [ 下标访问 / 空间应用 ] arr[i] / f x      ──► Index / SpaceApply 协议
  [ 自定义字面量后缀 ]  32px / 25ms         ──► LiteralInto<.px, R>.literal_into(32)
  [ 错误恢复处理器 ]    expr.use(handler)   ──► HandleError<E, R>.handle_error(...)
================================================================================
```

---

## 二、二元算术 `Add` 与一元逻辑并非 `Not` 协议

在我们的遥测与张量框架中，假设有一个状态标志结构体 `Flag { enabled: bool }`。我们希望：
1. 可以直接用 `flag + true` 将一个 `Flag` 与普通 `bool` 合并（使用 `Add` 协议）；
2. 可以直接用 `not flag` 对标志取反（使用 `builtin.Not` 协议）。

在 Flurry 中，二元算术协议（如 `Add`）并未强迫右操作数类型（`Rhs`）和返回值类型（`Output`）必须等于左操作数 `Self`。`Rhs` 是接口的参数（默认通常为 `Self`，也可显式指定 `.Rhs = ...`），而 `Output` 则是关联类型 `assoc Output: Type`：

```flurry
use builtin.Not;

struct Flag {
    enabled: bool,
}

-- 1. 实现二元加法协议 Add<.Rhs=bool>
impl Add<.Rhs=bool> for Flag {
    assoc Output: Type = bool;

    fn add(self, rhs: bool) -> bool {
        self.enabled or rhs
    }
}

-- 2. 实现一元逻辑非协议 Not（注意方法名为 invert，无 Rhs 参数）
impl Not for Flag {
    assoc Output: Type = bool;

    fn invert(self) -> bool {
        not self.enabled
    }
}

test {
    let f = Flag { enabled: false }
    -- 左右操作数严格按源码顺序各求值一次，然后调用 Add<.Rhs=bool>.add
    let merged: bool = f + true;
    -- 调用 Not.invert
    let inverted: bool = not Flag { enabled: false }
    println("{} {}", merged, inverted); -- 输出：true true
}
```

### 💡 双向类型推导亮点：当右操作数需要上下文时，协议如何反向赋能？

此外，Flurry 的运算符协议查询非单向固定的。
假设我们为 `u8` 实现了与枚举 `Choice` 相加的协议 `impl Add<.Rhs=Choice> for u8`。那么在调用处，即使你把右操作数写成缩略枚举字面量 `.ready`（甚至套在一层泛型函数 `identity(.ready)` 里），编译器在看到左操作数是 `u8` 且存在唯一适用的 `Add<.Rhs=Choice>` 实现时，会**自动把 `Choice` 作为期望类型上下文反向注入给右操作数**。

```flurry
enum Choice {
    ready,
    other,
}

impl Add<.Rhs=Choice> for u8 {
    assoc Output: Type = bool;

    fn add(self, rhs: Choice) -> bool {
        self == 9u8 and (rhs match {
            .ready => true,
            .other => false,
        })
    }
}

fn identity(value: T) -> T
where T
{
    value
}

fn check_ready(value: u8) -> bool {
    -- ✅ 编译器发现 u8 唯一适用的非标量加法协议要求 Rhs = Choice，
    -- 于是自动将 Choice 上下文传入 identity(.ready)，无需啰嗦地写 Choice.ready。
    value + identity(.ready)
}
```

---

## 三、实战改进写法：用 `LiteralInto` 打造零开销物理量后缀（`32px`）

在传感器工程与图形/张量渲染中，把像素 `Pixel`、毫秒 `Milliseconds` 或赫兹 `Hertz` 全都裸写成 `u32`，是酿成单位混淆 Bug 的温床（史上著名的火星气候探测者号坠毁，正是因为把磅力秒和牛顿秒混为一谈。）。

在 Flurry 中，你可以直接在数字、字符或字符串字面量后面紧跟一个标识符后缀（如 `32px`），编译器会自动将其转化为对 **`LiteralInto<suffix, R> for S`** 协议的查询。

让我们看看 `32px` 是如何工作的：
- **字面量主体 `S`**：整数字面量的编译期类型为 `Integer`；
- **后缀 Symbol**：`px` 对应符号 `.px`；
- **目标类型 `R`**：由上下文期望类型提供（如 `Pixel`）或由唯一实现推导。

```flurry
struct Pixel {
    value: u32,

    fn new(value: u32) -> Self {
        Self { value: value }
    }
}

-- 为编译期整数 Integer 实现后缀为 .px、目标类型为 Pixel 的字面量转换协议。
impl LiteralInto<.px, Pixel> for Integer {
    -- 注意：literal_into 是 runtime 相的普通函数，但接收 comptime value: Integer。
    fn literal_into(comptime value: Integer) -> Pixel {
        Pixel.new(value.as(u32))
    }
}

test {
    -- 写下 32px，编译器自动查询 LiteralInto<.px, Pixel> for Integer。
    let width: Pixel = 32px;
    println("{}", width.value); -- 输出：32
}
```

仔细品味一下 `fn literal_into(comptime value: Integer) -> Pixel` 的设计：
这正是我们在第 100、101 期刚刚学过的 **“带 `comptime` 参数的 Runtime 相方法”**。
- 因为 `value: Integer` 是 `comptime` 参数，你可以在方法签名或内部利用编译期信息做范围与格式校验；
- 同时，因为同一个后缀 `.px` 可以为不同的目标类型 `R`（比如 `impl LiteralInto<.px, Pixel> for Integer` 和 `impl LiteralInto<.px, SubPixelF32> for Integer`）分别实现，只要调用处给出了明确的目标类型上下文（如 `let width: Pixel = 32px;`），两者属于不同的完整 Trait 实例，**完全可以和平共处**。

---

## 四、控制流与能力的良好合流：`HandleError` 与 `.use(handler)`

除了算术和字面量，我们在第二卷（第 032 期）学过的**局部错误恢复语法 `expr.use(handler)`**，其底层同样是由标准的 Trait 协议驱动的。

当一个函数可能返回错误 `!LimitErr u32`，而你在调用处写下 `checked_count(12).use(handler)` 时：
1. **控制流由语法规定**：如果 `checked_count(12)` 正常返回 `u32`，**根本不会调用 `handler`**，直接交付成功值；只有当它走错误分支返回 `LimitErr` 时，才转入处理器分支。
2. **处理入口由 Trait 协议提供**：编译器查询 `handler` 的类型是否实现了 **`HandleError<LimitErr, u32>`**，并在错误分支接入它的 `handle_error` 方法。

```flurry
trait HandleError
where E, R
{
    fn handle_error(*self, value: E) -> R;
}

enum LimitErr {
    TooLarge(u32),
}

-- 定义一个“超限自动截断到上限值”的错误处理器
struct ClampTo {
    limit: u32,
}

impl HandleError<LimitErr, u32> for ClampTo {
    fn handle_error(*self, value: LimitErr) -> u32 {
        value match {
            .TooLarge(_) => self.limit.*,
        }
    }
}

fn checked_count(count: u32) -> !LimitErr u32 {
    if count <= 8u32 {
        count
    } else {
        error LimitErr.TooLarge(count);
    }
}

test {
    -- 声明 handler 具备处理 LimitErr 的能力，并通过 .use(handler) 接入协议。
    let handler handles LimitErr = ClampTo { limit: 8u32 }
    let result = checked_count(12u32).use(handler);
    println("count: {}", result); -- 输出：count: 8
}
```

### ⚠️ 协议安全规则：找到一个 Trait 方法 $\neq$ 豁免控制流与内存效果检查

在使用任何语言协议时，请牢记 Flurry 编译器的两条红线：
1. **自己写的同名 Trait 免于自动抢占运算符**：`Add`、`Not`、`LiteralInto` 等协议身份是由编译环境绑定的内置声明；你在自己的业务模块里随便写一个同名的 `trait Add`，免令 `+` 号变成调你的接口。
2. **协议入口的副作用与契约必须如实入账**：把 `a.add(b)` 写成 `a + b`，或者把错误处理写成 `.use(handler)`，**不会豁免该方法的前置条件（`requires`）、阶段限制或内存效果（`outcomes`）**。如果 `handler` 在恢复错误时修改了某块外部内存，或者抛出了新的未处理错误，这些效果与错误依然会严格计入外层函数的整体账本中。

---

## 五、本节核心概念速查表

| 表面语法 | 底层连接的 Trait 协议 | 关键关联项 / 方法签名 |
| :--- | :--- | :--- |
| `lhs + rhs` | `Add<.Rhs=R>` | `assoc Output: Type;` + `fn add(self, rhs: R) -> Output` |
| `-val` / `not val` | `Neg` / `builtin.Not` | `assoc Output: Type;` + `fn neg(self)` / `fn invert(self)` |
| `ptr.*` / `val.ref` | `Deref` / `Reference` | 控制指针解引用与取址及接收者自动适配 |
| `32px` | `LiteralInto<.px, R> for Integer` | `fn literal_into(comptime value: Integer) -> R` |
| `expr.use(handler)` | `HandleError<E, R>` | `fn handle_error(*self, value: E) -> R`（仅在错误分支触发） |
| `obj.method<.json>()` | 参数化 Trait 成员简写 | 当 `method` 来自参数化 `Serialize<format>` 时，`.json` 自动应用到接口参数 |

---

## 六、下一期预告

在刚才的协议全景表里，无从得知你有没有注意到一行神秘的条目：
> **“变参收集、隐式传参 ──► 明确的协议及参数来源”**

回想一下我们在第二卷学过的函数参数：
- 位置变参 `...items: T` 默认会把传入的多个位置实参收集成一个元组 `Tuple`；
- 扩展调用属性变参 `...properties: T` 默认会把 `func { width: 2, height: 3 }` 里的命名属性收集成一个匿名结构体（Record）；
- 带有 `lambda` 标记的参数支持超级简写的占位符闭包 `_0 < _1`（Tactic Lambda）和尾随 `do` 块。

如果我们正在为遥测系统设计一个构建器，想定制 `...items`、`...properties` 或 `implicit` 参数的收集与供应协议，同时又要保证实参的求值顺序和兄弟参数不被篡改，编译器在 **参数 Elaboration（展开与接入）** 阶段到底制定了怎样的精确规则？

下一期 **《Flurry 演进手记 (103)：定制你自己的变参与属性收集器——参数收集协议与 Elaboration 接入》**，我们将深入函数调用的参数准备流水线。
