# Flurry 演进手记 (096)：给类型立下能力契约——trait 声明、关联项 assoc 与 impl Trait for Type

> **系列导读**：欢迎来到《Flurry 演进手记》**第五卷：契约式多态——Trait 系统、两相一致性与自动派生（096 – 115）**！在第四卷中，我们领略了双层类型理论（2LTT）、编译期宇宙 $U_1$ 与 AST 元编程的威力，用编译期函数构造出了静态形状张量 `Tensor<T, ...Shape>`。但在大型系统工程中，如果所有多态代码都只靠编译期鸭子类型（Duck Typing）连接，报错就会推迟到实例化深处，且无法表达跨模块的严密契约。从今天起，我们将围绕一个贯穿第五卷的核心实战工程——**“支持零成本泛型、编译期预计算与按需动态擦除的遥测与张量序列化引擎（Telemetry & Tensor Serialization Framework）”**，全面解锁 Flurry 的 **Trait 名义接口体系**！

---

## 一、为什么仅有编译期鸭子类型还不够？版本 `v0` 的“实例化深渊”

假设我们在自动驾驶或工业机器人的遥测系统中，有两类数据源：基础的温度计传感器 `Gauge` 和恒定基准源 `FixedReading`。我们想写一个通用的采集打印函数 `show_reading`。

如果像 C++ 早期模板或纯粹的编译期鸭子类型那样，在 `v0` 中不对类型参数 `T` 施加任何接口契约，直接去调用 `value.read()`：

```flurry
struct Gauge {
    value: u32,
}

-- v0：没有任何接口约束的“盲目泛型”
fn show_reading_v0(value: *T) -> void
where T
{
    -- ❌ 编译期在定义处就会拦截！
    -- Flurry 拒绝像 C++ 模板那样“等到实例化时再去赌 T 有没有 read 方法”！
    -- 在定义检查阶段，未知类型 T 没有任何证据表明它拥有 read() 操作！
    println("reading: {}", value.read());
}
```

为什么 Flurry 坚决拒绝“无约束的模板鸭子类型”？
1. **定义端无法独立检查（No Definition-Site Checking）**：如果允许在未声明约束的情况下调用 `value.read()`，那么函数定义本身是否合法就成了未知数；一旦下游用户传错了一个类型，报错信息会炸裂在十几层模板调用栈的最深处。
2. **同名不等于同契约（Accidental Name Collision）**：假设 `FileStream` 有一个 `read()`（从磁盘读字节并修改文件游标），而传感器 `Gauge` 也有一个 `read()`（无副作用读取当前刻度）。名字相同，根本不能证明它们履行相同的业务与资源契约！

为了在**定义端**就锁定能力契约，同时保持零运行时开销的静态单态化调用，我们需要 **名义接口抽象（Nominal Interface Abstraction）——`trait` 与 `impl`**。

---

## 二、迭代演进 `v1`：声明 `trait`、默认方法与 `impl Trait for Type`

在 Flurry 中，`trait` 定义了一组有名字、有契约的操作与关联项；而 `impl Trait for Type` 则显式声明某个主体类型如何兑现这份契约。

让我们看看最基础的语法结构：

```text
trait_def      -> trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
```

在 `trait` 内部：
- **必需方法**：只写函数签名，末尾以分号 `;` 结束；
- **默认方法**：直接提供普通函数体 `{ ... }`，它可以调用同一 `trait` 内的其他成员，而无需每个实现者重复编写。

让我们用 `trait Reading` 重构出 `v1` 版本的传感器采集器：

```flurry
-- 1. 声明名义接口 Reading
trait Reading {
    -- 必需方法：只有签名，以分号结尾
    fn read(*self) -> u32;

    -- 默认方法：提供函数体，基于 self.read() 计算是否超过警戒线
    fn is_high(*self, threshold: u32) -> bool {
        return self.read() >= threshold;
    }
}

struct Gauge {
    value: u32,
}

struct FixedReading {}

-- 2. 为 Gauge 兑现 Reading 契约（沿用默认的 is_high）
impl Reading for Gauge {
    fn read(*self) -> u32 {
        return self.value.*;
    }
}

-- 3. 为 FixedReading 兑现 Reading 契约
impl Reading for FixedReading {
    fn read(*self) -> u32 {
        return 100u32;
    }
}

-- 4. 在函数签名中用 where T :- Reading 声明能力约束
fn show_reading(value: *T) -> void
where T :- Reading
{
    println("reading: {}, high: {}", value.read(), value.is_high(80u32));
}

test {
    let gauge = Gauge { value: 27u32 };
    let fixed = FixedReading {};
    -- 零成本静态分派：无需把对象包装成虚表指针，直接编译为确定函数调用！
    show_reading(gauge.ref);
    show_reading(fixed.ref);
}
```

### Code Cartoon：Trait 定义检查与显式视角消歧（`.as(Trait)`）

当一个类型同时实现了两个碰巧都有同名方法 `read` 的不同接口（例如 `Reading` 和 `RawRegister`）时，或者当默认方法需要明确引用当前接口的成员时，Flurry 提供了极其干净的后缀类型视角语法 **`expr.as(Trait)`**：

```text
                  [ gauge: Gauge ]
                    /          \
   impl Reading for Gauge     impl RawRegister for Gauge
       fn read() -> u32           fn read() -> u32
                    \          /
         gauge.as(Reading).read()  <── 显式指定接口来源，零运行时开销消歧！
```

---

## 三、终极演进 `v2`：用统一关联项 `assoc` 表达关联类型、关联常量与 GAT

在设计通用遥测与张量采样接口 `SampleSource` 时，我们很快会遇到更深层的需求：
1. 不同的传感器采样出的数据类型不同：`Counter` 采样出 `u32`，而高精度陀螺仪 `Gyroscope` 采样出 `(f32, f32, f32)`。这个输出类型不是调用方随便指定的泛型输入，而是**由主体类型唯一决定的关联输出类型**！
2. 每个传感器不仅有关联类型，还有编译期已知的默认缓冲区容量常量 `capacity: usize`；
3. 甚至，当我们要从张量数据源上切出一个指定长度的窗口视图时，这个关联类型本身还可以带参数（即在 Rust 中被称为**泛型关联类型 GAT** 的高级特性）！

在很多语言里，“关联类型（`type Item;`）”、“关联常量（`const CAP: usize;`）”和“泛型关联类型（`type View<'a, const N: usize>;`）”是三套割裂的语法。
而在 Flurry 中，得益于双层类型理论（2LTT）将类型 `Type` 视为编译期一等值，这三者被**完美统一到了同一个关键字 `assoc` 之下**：

```text
assoc_decl -> assoc id(<parameter*>)?: expr (= default)? clauses?
```

看看 `assoc` 的统一威力：
- **关联类型**：`assoc Element: Type;`（因为 `Type` 本身就是类型的值域！）
- **带默认值的关联常量**：`assoc capacity: usize = 1;`
- **带参数的关联项（天然支持 GAT）**：`assoc Batch(count: usize): Type = [count]Element;`

让我们写出 `v2` 版本的 `SampleSource` 接口与实现：

```flurry
trait SampleSource {
    -- 1. 关联类型：采样元素的类型
    assoc Element: Type;

    -- 2. 带默认值的关联整数常量：默认采样深度为 1
    assoc capacity: usize = 1usize;

    -- 3. 带参数的关联类型（GAT）：按给定窗口大小生成批量数组类型，默认使用 [n]Element
    assoc Window(n: usize): Type = [n]Element;

    -- 必需方法：返回关联类型 Element
    fn sample(*self) -> Element;

    -- 默认方法：通过 Self.as(SampleSource).capacity 读取关联常量
    fn declared_capacity(*self) -> usize {
        return Self.as(SampleSource).capacity;
    }
}

struct Counter {
    value: u32,
}

impl SampleSource for Counter {
    assoc Element: Type = u32;
    -- capacity 未显式覆盖，自动沿用默认值 1usize
    -- Window(n) 未显式覆盖，自动代入 Element 得到 [n]u32

    fn sample(*self) -> u32 {
        return self.value.*;
    }
}

test {
    let counter = Counter { value: 12u32 };
    -- 通过 Type.as(Trait).Assoc 投影访问关联类型
    let item: Counter.as(SampleSource).Element = counter.sample();
    let window: Counter.as(SampleSource).Window(2usize) = [item, item];
    println("{} {} {}", item, window[0usize], counter.declared_capacity());
}
```

### ⚠️ 核心契约边界：实现（`impl`）绝不能违背接口（`trait`）的承诺！

在 Flurry 中，`impl Trait for Type` 不是简单的“名字填空”，编译器会严格执行**接口契约替换检查（Contract Substitution Check）**：

1. **禁止遗漏或跨相拼凑**：没有默认值的 `assoc` 项和方法必须全部提供；不能拿另一个同名 `trait` 或另一个阶段（Phase）的同名方法充数。
2. **禁止制造循环自证**：关联项之间可以相互依赖（例如上面的 `Window(n)` 依赖 `Element`），编译器会建立有向无环依赖图（DAG）代入检查；但如果写出循环依赖的关联项定义，编译器会直接拒绝：

```flurry
trait BrokenCycle {
    assoc A: Type = B;
    -- ❌ 编译期直接拒绝！关联项 A 与 B 构成循环定义，不能作为自身成立的证明！
    assoc B: Type = A;
}
```

3. **前置条件只能减弱、结果保证只能保持或加强**：如果 `trait` 的方法没有要求特定前置条件或错误/效应，具体 `impl` 的方法**绝对不能**私自增加调用者未准备好的前置要求、错误类型或代数效应！

---

## 四、本节核心概念速查表

| 语法形式 | 名称与角色 | 核心语义规则 |
| :--- | :--- | :--- |
| `trait Name { ... }` | 名义接口声明 | 建立独立的接口身份；方法签名相同不会让两个 trait 成为同一接口 |
| `assoc Name: Type;` | 关联类型声明 | `assoc` 统一表达关联类型与关联值；可带默认值 `= DefaultType` |
| `assoc Name: usize = 1;` | 关联常量声明 | 声明与主体类型绑定的常量值，可通过 `Self.as(Trait).Name` 访问 |
| `assoc Name(p: T): Type;` | 参数化关联项 (GAT) | 关联项直接携带参数列表，无需额外语法扩展即可表达泛型关联类型 |
| `impl Trait for Type { ... }` | 接口实现声明 | 为允许用户实现的主体类型兑现契约；默认项自动补齐，契约不可削弱 |
| `expr.as(Trait)` | 显式接口视角 | 当多个接口存在同名成员或需读取关联项时，明确指定成员来源 |

---

## 五、下一期预告

今天我们学会了用 `where T :- Reading` 为泛型参数施加单个 Trait 约束。
但在构建真实的遥测与序列化引擎时，一个函数往往需要参数**同时满足多个接口**（例如既能读取 `Reading`，又有名字 `Named`，还能序列化为 JSON `Encode<.json>`）。更棘手的是，有时为了防止把标记为“已禁用或高危敏感（`Disabled` / `Secret`）”的传感器数据不小心发往公网日志，我们还需要声明一条**否定约束（Negative Constraint）**：“要求类型 `T` 实现了 `Reading` 和 `Named`，但**绝没有实现 `Disabled`**”！

在 Flurry 中，为什么正向约束用 `where T :- A + B`，而否定约束却**严禁**塞进 `+` 列表里，必须写进 `requires not (T :- Disabled)`？编译器又是如何证明一个类型“没有实现某个 Trait”的？

下一期 **《Flurry 演进手记 (097)：组合多个 Trait 约束——where T :- A + B 正向约束与 requires 否定约束》**，我们将揭开正向能力合取与负向逻辑命题的协同法则！
