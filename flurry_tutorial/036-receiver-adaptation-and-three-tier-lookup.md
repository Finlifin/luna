# Flurry 演进手记 (036)：指针层级再深也能一键调用——方法接收者适配与三层查找优先级

在 [上一篇 (035)](./035-function-templates-and-test-templates.md) 的结尾，我们提出了一个在日常系统开发中几乎每分钟都在发生的问题：

假设我们有一个传感器脉冲计数器结构体 `Counter`：

```flurry
struct Counter {
    value: u32,

    fn read(*self) -> u32 {
        self.value
    }
}
```

在实际工程中，持有 `Counter` 的方式千姿百态：
- 有时候你手里直接拿着值 `counter: Counter`；
- 有时候你拿着一级指针 `pointer: *Counter`；
- 有时候在二级索引表或回调上下文里，你拿到的是**二级指针 `outer: **Counter`**。

如果每次调用方法或读取字段前，程序员都得在脑子里数清楚“我现在手里到底是几级指针？该方法要的是 `self` 还是 `*self`？我该补 `.ref` 还是补几个 `.*`？”，代码很快就会变成满屏的 `outer.*.read()` 或 `counter.ref.read()`。

更进一步：当我们的结构体同时实现了两个都带有 `fn code(*self) -> u32` 方法的接口（比如 `WireCode` 和 `DisplayCode`），在泛型函数里调 `status.code()` 时，万一实例化传入的具体类型同時实现了这两个接口，是否会突然报歧义错误或者调错方法？

今天，我们将深入 `docs/type/receiver-adaptation.md` 与 `docs/type/member-projection.md`，把 Flurry 的**方法接收者适配**、**三层查找优先级**与**定义端固定成员来源**看清全貌。

---

## 一、 手动解引用/取址的繁琐 vs 零拷贝自动接收者适配

如果我们没有任何自动适配（像C 语言函数调用一样），写出来的 初始代码会充满机械的样板噪音：

```flurry
-- 假设没有接收者适配与目标引导读取，每一步都得手工对齐指针层级
fn manual_verbose() {
    let counter = Counter { value: 7 }
    let pointer: *Counter = counter.ref;
    let outer: **Counter = pointer.ref;

    -- 手动解引用一层才能凑齐 read(*self) 需要的 *Counter：
    let r1 = outer.*.read();
    -- 手动取址才能在值上调 read(*self)：
    let r2 = counter.ref.read();
}
```

而在 Flurry 中，当你写下 `outer.read()`、`counter.read()` 或 `pointer.value` 时，编译器的**接收者适配引擎（Receiver Adaptation）**会自动为你铺平道路，而且**不会偷偷复制整個 `Counter` 对象**。

来看看官方规范中的标准示例：

```flurry
use std.prelude.*;

struct Counter {
    value: u32,

    fn read(*self) -> u32 {
        -- self 是 *Counter，self.value 是字段指针 *u32，
        -- 因为函数返回目标明确是 u32，自动触发 Copy 读取。
        self.value
    }
}

test {
    let counter = Counter { value: 7 }
    let pointer = counter.ref;
    let outer = pointer.ref;

    -- 1. 无目标类型上下文：保留原始投影结果（字段指针 *u32）
    let field = pointer.value;

    -- 2. 有明确目标类型 u32：触发目标引导读取（Target-Guided Read），自动读取 Copy 值。
    let snapshot: u32 = pointer.value;

    -- 3. 二级指针 outer (**Counter) 直接调用 .read()。
    println("{} {} {}", field.*, snapshot, outer.read());
}
```

运行这段代码，输出为：

```text
7 7 7
```

### `outer.read()` 的自动寻路 结构示意

下面来看看当编译器看到 `outer.read()`（其中 `outer` 是 `**Counter`，而 `Counter.read` 需要 `*Counter`）时，内部是如何一步步走通的：

```text
  调用表达式：outer.read()
  初始接收者形态：outer : **Counter
      │
      ├─► [深度 0: **Counter] 有没有叫 `read` 的成员？ ──► 没有。
      │      │ (执行 1 次内建指针解引用 `.*`)
      ▼      ▼
      ├─► [深度 1: *Counter]  有没有叫 `read` 的成员？ ──► 没有。
      │      │ (再执行 1 次内建指针解引用 `.*`)
      ▼      ▼
      └─► [深度 2: Counter]   找到了 `Counter.read(*self)`。
             │
             ▼
  [末端指针适配检查]：
  `Counter.read` 要求接收者是 `*Counter`。
  Wait! 难道我们要先把 `Counter` 整个复制一份，再对副本取地址 `.ref` 吗？
  ❌ 不。
  编译器发现刚才在 [深度 1] 时手里就已经拿着现成的 `*Counter` 指针（即 `outer.*`）。
  直接把 `outer.*` 交给 `Counter.read(*self)`，全程零对象拷贝。
```

仔细体会这条核心规则（详见 `docs/type/receiver-adaptation.md`）：
> **当已有指针能直接满足方法需求时，不先复制所指对象再取副本地址。且接收者表达式只求值一次，候选搜索过程跳过执行任何运行时动作，只有最终选中的路径才生成解引用或取址指令。**

那如果 `Counter` 是一个**唯一无二（并非 `Copy`）**的仿射资源对象，而有人试图通过指针上的“目标引导读取”把它自动搬出来呢？

```flurry
struct AffineSensor {
    id: u32,
}

-- ❌ 编译期直接报错。
fn bad_auto_read(ptr: *AffineSensor) -> AffineSensor {
    -- 错误。目标引导读取（自动插入内建指针读取 `.*`）要求最终读出的值必须具备 `Copy` 能力。
    -- 编译器不会利用隐式读取规则把并非 Copy 的仿射对象从指针后面偷偷搬走。
    ptr
}
```

---

## 二、 当多个候选同时存在——Flurry 的“三层查找优先级”

现在你可能会问：如果一个对象外层包了指针，或者实现了自定义智能指针的 `Deref` 接口，又或者带有显式类型视角（我们下一篇会详谈），编译器到底是按什么顺序搜索方法的？

Flurry 制定了严格、确定、不回退掩盖错误的**三层查找顺序（Three-Tier Lookup Order）**：

```text
  ══════════════════ 方法查找的三层严格优先级 ══════════════════

  [第 1 层：显式视角顺序]
  先按类型上的显式视角顺序（如 `A.as(B).as(C)` 中的 `C -> B -> A`）选择当前视角。
        │
        ▼
  [第 2 层：解引用深度由浅入深]
  在当前视角内，从原接收者形态（深度 0）出发，
  依次沿【内建指针解引用 `.*`】与【唯一确定的 `Deref` 路径】由浅到深步进：
  例如：**T (深度 0) ──► *T (深度 1) ──► T (深度 2) ──► T.Target (深度 3)...
        │
        ▼
  [第 3 层：同一形态下的成员来源与实现精度]
  在当前深度的形态上：
  - 先看当前命题上下文已提供的 trait 能力与类型固有成员；
  - 若同一 trait 查询有多个实现，按 implementation 结构精度（更具体的优先）选择；
  - 选定方法后，若方法签名需要 `*self`，在末端做一次必要的内建取址 `.ref` 或 `Reference` 适配。
  ══════════════════════════════════════════════════════════════
```

注意两条防踩坑边界：
1. **无法靠“少取一次地址”在不同 Trait 之间消歧**：
   假设 `TraitA.foo(self)` 按值接收，而 `TraitB.foo(*self)` 按指针接收。如果你拿着值 `x` 调用 `x.foo()`，编译器在同一个解引用深度上同时看到了 `TraitA.foo` 和 `TraitB.foo`——此时它们来自两个各异的 Trait 来源，**直接报同名歧义错误**。编译器不会隐式地说“因为 `TraitA.foo(self)` 无须取地址 `.ref`，所以帮你在两个毫无关系的 Trait 里挑了 `TraitA`”。
2. **选定来源后报错不回退**：
   一旦高优先级层找到了匹配的方法来源，如果随后因为你传错了参数类型或违反了资源契约而报错，编译器会**直接报告该调用的错误**，不会悄悄退回更深层的解引用去调另一个同名方法来掩盖你的 bug。

---

## 三、 定义端固定成员来源——为什么泛型实例化不“临阵倒戈”？

接下来看本篇最核心的架构级特性：**成员来源稳定性（Source Stability，详见 `docs/type/member-projection.md`）**。

假设我们定义了两个接口，它们都有一个叫 `code(*self) -> u32` 的方法：
- `WireCode`：返回底层硬件线缆编号（比如 `7`）；
- `DisplayCode`：返回界面显示编号（比如 `107`）。

我们的状态结构体 `Status` **同时实现了这两个接口**。
现在，我们写了一个泛型函数 `protocol_code(status: *T) -> u32 where T :- WireCode`，并在里面调用了 `status.code()`。

请思考：当我们把同时具备 `WireCode` 和 `DisplayCode` 的 `status: Status` 传给 `protocol_code(status.ref)` 时，会发生什么？会报歧义吗？

来看看实际代码：

```flurry
use std.prelude.*;

trait WireCode {
    fn code(*self) -> u32;
}

trait DisplayCode {
    fn code(*self) -> u32;
}

struct Status {
    value: u32,
}

impl WireCode for Status {
    fn code(*self) -> u32 {
        self.value.*
    }
}

impl DisplayCode for Status {
    fn code(*self) -> u32 {
        self.value.* + 100
    }
}

-- 在定义端：上下文只有 `T :- WireCode` 约束。
-- 因此 `status.code()` 的来源在定义检查时就已经被永久锁定为 `WireCode.code`。
fn protocol_code(status: *T) -> u32
where T :- WireCode {
    status.code()
}

test {
    let status = Status { value: 7 }

    -- 1. 在 具体类型 Status 上直接调，用 .as(Trait) 显式指明来源：
    println("wire: {}", status.as(WireCode).code());
    println("display: {}", status.as(DisplayCode).code());

    -- 2. 传入泛型函数 protocol_code：稳稳输出 7，不因 Status 还有 DisplayCode 而产生歧义。
    println("generic wire: {}", protocol_code(status.ref));
}
```

运行这段代码，输出：

```text
wire: 7
display: 107
generic wire: 7
```

下面对比 C++ 模板的“实例化时按名字盲搜”与 Flurry 的“定义端锁定来源”：

```text
  C++ 模板式的脆弱查找：
  定义 template<class T> f(T x) { x.code(); } 时只看个名字 "code"；
  等到实例化 T = Status 时再去找 "code" ──► 撞上同名方法或被意外劫持。

  Flurry 的定义端固定来源（Source Pinning）：
  [阶段 1：定义检查 `protocol_code`]
  看到 `where T :- WireCode` ──► 将 `status.code()` 静态绑定为槽位：(Trait: WireCode, Method: code)
                                      │
                                      ▼ (来源焊死。)
  [阶段 2：实例化 `T = Status`]
  直接去查 `impl WireCode for Status` 的 `code` 实现。
  哪怕 `Status` 后来还实现了 `DisplayCode.code`，甚至有个固有字段/方法，
  也不重新竞争、不篡改已锁定的 `WireCode` 来源。
```

那如果在 `test` 块里，我们对 `status` **省略 `.as(WireCode)` 也省略 `.as(DisplayCode)`**，直接裸写 `status.code()` 呢？

```flurry
-- ❌ 编译期直接报错。
fn ambiguous_call(status: Status) -> u32 {
    -- Status 同时实现了 WireCode.code 和 DisplayCode.code，
    -- 在同一视角、同一解引用深度下存在两个平级的不同来源，编译器报错猜测，要求显式 .as(...) 消歧。
    status.code()
}
```

---

## 本篇小结

| 核心机制 | 行为与规则 | 解决的问题 |
| :--- | :--- | :--- |
| **方法接收者适配** | 沿内建 `.*` 与唯一 `Deref` 从浅到深寻路，末端按需补一次 `.ref` / `Reference` | 拿着二级指针 `**Counter` 也能直接写 `outer.read()`，且复用已有指针、零对象拷贝 |
| **目标引导读取** | 当绑定/参数/返回值有明确目标类型且目标值可 `Copy` 时，自动读取字段指针 | `let field = ptr.value;` 保有 `*u32` 指针，而 `let snap: u32 = ptr.value;` 直接拿到 `u32` 值 |
| **三层查找优先级** | 1. 显式视角顺序 $\rightarrow$ 2. 解引用深度（浅到深） $\rightarrow$ 3. 实现结构精度 | 查找次序 100% 确定可预测；选定高优来源后报错不回退 |
| **定义端固定来源** | 参数化代码在定义检查时锁定成员所属的接口身份，实例化只负责兑现该接口 | 告别 C++ 模板实例化时的同名方法劫持与意外歧义 |

---

### 下一篇预告

在本篇的最后，我们看到了 `status.as(WireCode).code()` 和 `status.as(DisplayCode).code()`。
这只是 `.as(...)` 在值表达式上用来指定 Trait 来源的用法。

但是，在刚刚讲三层查找顺序的第 1 层时，我们提到了一个神秘词：**“类型标注上的显式视角顺序”**。
什么意思？难道在 Flurry 里，我可以写这样一个变量声明吗：

```flurry
let protocol: u32.as(DecimalCode).as(ProtocolCode) = 7;
```

- 一个变量的类型怎么能写成 `u32.as(DecimalCode).as(ProtocolCode)`？
- 这个变量 `protocol` 的本体类型到底是 `u32`、`DecimalCode` 还是 `ProtocolCode`？
- 当我调用 `protocol.category()` 时，为什么它会按 **`ProtocolCode -> DecimalCode -> u32`** 从右往左倒着找方法？而且为什么 `ProtocolCode` 是直接跟最左边的 `u32` 建立双向转换关系，而非串糖葫芦式的 `u32 -> DecimalCode -> ProtocolCode`？

在下一篇 **《Flurry 演进手记 (037)：同一数值的多种人格——`.as(View)` 类型视角链与显式接口消歧》** 中，我们将揭秘这个让底层标量同时拥有多层领域方法视图的这一设计。
