# Flurry 演进手记 (101)：为什么编译期算好的 Hash 能直接带进运行时——Eq/Hash 与跨阶段 Splice 契约

> **系列导读**：在第 100 期里程碑中，我们见证了 Trait 在编译期宇宙（Comptime Phase）与运行期宇宙（Runtime Phase）中的双重生活，以及两相为什么必须共享同一套全局一致性（Shared Coherence）。今天，我们将继续深入探索两个宇宙交界处的三大核心基础设施：支撑编译期记忆化缓存与静态查找表的 **`Eq` / `Hash` 两相契约**，以及负责将编译期计算结果安全运送到运行期程序的跨次元传送门——**`Splice` 跨阶段值传递协议**！

---

## 一、编译期与运行期是个“平行宇宙”：为什么不能直接把编译期内存拷进可执行文件？

在编写遥测与张量序列化引擎时，我们经常希望在编译期预先算好一张“传感器 ID 到校准参数”的静态查找表（Static Lookup Table），或者在编译期算好一组常量配置结构体，然后在运行期直接使用它们。

在初学者眼里（`v0` 的朴素想象），这似乎再简单不过了：“编译期算出了一个值，编译器直接把当时内存里的字节 `memcpy` 塞进可执行文件的 `.rodata` 段不就行了吗？”

**大错特错！** 让我们看看编译期（Comptime Engine）与运行期（Target Runtime）之间的三道天堑：
1. **类型表示可能根本不同**：比如编译期无界整数 `Integer` 和任意精度浮点数 `Float` **只存在于编译期宇宙**，运行期目标机器上只有定宽的 `i32`、`u64`、`f64`！
2. **宿主地址空间与目标机器完全隔离**：编译期解释器运行在你的开发机（比如 x86_64 Linux 宿主机）上，而编译出的目标程序可能运行在一块 32 位 ARM 嵌入式传感器芯片上。编译期对象里的宿主内存指针或对象图地址，到了目标程序里就是彻头彻尾的非法野指针！
3. **资源与生命周期契约**：如果一个结构体包含带有运行期析构责任（`Drop`）的句柄或共享引用，怎么能靠盲目的字节拷贝跨越阶段？

为了在两个独立宇宙之间建立一条**既类型安全、又遵守资源契约**的通道，Flurry 设计了三大特殊的跨阶段基石 Trait：**`Eq`、`Hash` 与 `Splice`**。

---

## 二、`Eq` 与 `Hash` 的两相默契：静态查找表与编译期 Query 缓存

在 Flurry 中，`Eq`（相等性比较）和 `Hash`（哈希计算）不仅服务于运行期的哈希表，它们的 **Comptime 相（标注了 `^comptime_phase` 的成员，由 `comptime impl` 或 `comptime derive` 兑现）** 更是编译器内部运作的核心齿轮：

| Trait 阶段 | Phase 归属与实现方式 | 在编译期与跨阶段中的核心职责 |
| :--- | :--- | :--- |
| **`Eq` / `Hash` 的 Comptime 相** | 带有 `^comptime_phase` 属性，由 `comptime impl` / `comptime derive` 提供 | 参与编译时值（Comptime Values）的 **Query 缓存比较与键处理**，以及编译期静态查找表构建 |
| **`Eq` / `Hash` 的 Runtime 相** | 默认相（无 `^comptime_phase`），由普通 `impl` / `derive` 提供 | 在运行期对目标值执行相等判断与哈希计算 |

### 1. 为什么编译期建的静态表，运行期查表 100% 命中？

回到上一期（100）我们学过的 **“互补实现与两相共享 Coherence”**。
当你在编译期为一个传感器标识符类型 `SensorKey` 计算哈希桶位置时，调用的是 `SensorKey` 通过 `comptime impl`（或 `comptime derive`）提供的 `Hash` 和 `Eq` 的 **Comptime 相**；而当程序在运行期拿着实时收到的 `SensorKey` 去同一个表里查找时，调用的是通过普通 `impl`（或 `derive`）提供的 **Runtime 相**。

因为 Flurry 强制 `Eq` 和 `Hash` 的两相**共享同一个候选集、结构精度排序与全局一致性（Shared Coherence）**，任何人都无法为同一个 `SensorKey` 注入两份同精度重叠冲突的实现（更不会在缺相时偷偷退回低精度实现），从而在制度上保障了编译期分桶与运行期查表的一致性！

### ⚠️ 核心边界：`Eq` / `Hash` 缓存相等 $\neq$ PTC 逻辑等式！

在这里有一条必须牢记的编译器内部红线：
> **`Eq` 和 `Hash` 的 Comptime 相仅用于编译期值的 Query 缓存键处理与数据比较；绝不能把用户自定义的 `Eq`/`Hash` 结果直接当成 PTC（证明论式类型检查器）中的逻辑类型等式证明！**

为什么？因为用户完全可以手写一个“不管什么值都返回 `true`”的胡闹 `comptime impl Eq` 实现；如果编译器把 `a == b`（调用 `Eq`）直接等同于类型系统里的逻辑完全同一（Logical Equality），整个形式化验证与类型安全大厦就会被轻易攻破！

---

## 三、跨次元传送门：揭秘 `trait Splice` 协议

现在，让我们解决最核心的问题：当一段编译期计算产生了一个结果 `data`，并且这个结果**真正需要进入运行期数据流**时，它是如何跨越次元壁的？

在 Flurry 中，普通值跨阶段使用不需要你在代码里到处写特殊的拼接符号，编译器会自动接入标准库中的 **`Splice` 协议**：

```flurry
trait Splice {
    -- 关联类型：跨阶段进入运行期后的目标类型！
    assoc RuntimeTarget: Type;

    -- 注意两个关键点：
    -- 1. 没有 ^comptime_phase 属性 ──► 属于默认的 Runtime Phase（由普通 impl Splice 兑现）！
    -- 2. data 带有 comptime 参数标记，但 fn 本身没有 comptime 修饰 ──► 编译期按 data 实例化，运行期执行！
    fn splice(comptime data: Self) -> RuntimeTarget;
}
```

### Code Cartoon：为什么 `Splice.splice` 既属于默认 Runtime Phase，又是“带 `comptime` 参数的 Runtime 方法”？

还记得我们在第 100 期学过的正交原则吗？仔细看 `fn splice(comptime data: Self) -> RuntimeTarget`：
1. 它上面**没有** `^comptime_phase` 属性，因此它属于默认的 Runtime Phase，由普通的 `impl Splice for T` 实现；
2. 它的 `fn` 前面也**没有** `comptime`，而参数 `comptime data: Self` 带有编译期阶段标记！这意味着 **`Splice.splice` 是一个接收编译期输入进行实例化、但在运行期执行的目标构造方法**！

```text
[ 编译期宇宙 (Comptime) ]                     [ 运行期宇宙 (Runtime) ]
  计算出 comptime 值：data: Self
              │
              ├── 1. 作为 comptime 实参传入 ──► 实例化 Splice.splice<data>()
              │                                （目标类型为 Self.as(Splice).RuntimeTarget）
              │                                        │
              └───────────────────────────────► 2. 在运行期执行该实例！
                                                   （若无副作用且可直表为常量，
                                                     编译器可优化物化为静态常量；
                                                     否则保留必要的 runtime 构造操作！）
```

为什么要把 `Splice.splice` 设计成“编译期根据 `data` 实例化、运行期执行构造”？
因为有些编译期值（比如 `1 + 23` 得到 `24`）固然可以直接物化成运行期机器指令里的立即数常量；但另一些复杂的跨阶段对象，在进入运行期时**必须执行运行期的内存分配、资源注册或代理对象初始化**！如果把 `splice` 当成纯粹在编译期跑完的函数，目标程序里根本拿不到合法的运行期资源！

---

## 四、并不是所有编译期值都需要 `Splice`：三种跨阶段命运

为了避免过度使用 `Splice`，Flurry 编译器在降码（Lowering）时会精准识别编译期值的去向，将它们分为三种截然不同的情形：

### 情形 1：结果仅作为 `comptime` 输入使用（零 `Splice` 开销，运行期完全擦除！）

如果一个编译期值只是用来控制 `inline if` 分支或者作为泛型类型参数，它**根本不会进入运行期数据流**，因此完全不需要经过 `Splice`！

```flurry
-- first 只在编译期实例化时决定选哪条分支，
-- 运行期生成的机器码里既没有 first 参数，也不需要调用 Splice！
fn choose_value(comptime first: bool, left: i32, right: i32) -> i32 {
    return inline if first {
        left
    } else {
        right
    };
}
```

### 情形 2：编译期常量结果进入运行期数据流（触发 `Splice` 转换）

```flurry
fn print_total() -> void {
    -- 1 + 23 先在编译期算出 comptime 值 24，
    -- 因为它要传给运行期的 println，编译器通过对应类型的跨阶段规则将其物化为运行期值！
    println("{}", 1 + 23);
}
```

---

## 五、`Splice` 的有限自动派生与四大禁区

对于我们自己定义的遥测配置结构体（`struct`）、枚举（`enum`）、联合体（`union`）或闭包（`closure`），难道每次都要手写 `impl Splice` 吗？

不需要！Flurry 为满足安全条件的聚合类型提供了 **`Splice` 的有限自动派生（Limited Automatic Derivation）**：只要所有字段/变体成员都满足 `Splice` 转换条件，且目标类型的内存布局合法，编译器就会自动为你派生 `Splice`！

但是，为了守住我们在本节开头提到的内存与资源底线，编译器在自动派生 `Splice` 时，如果遇到以下**四大禁区**，会**立即拒绝自动派生**：

```text
[ Splice 自动派生的四大安全禁区（编译期直接拒绝自动派生） ]
  ├── 1. 含有共享引用 / 指针字段（宿主编译期内存地址绝不能原样带入目标运行期！）
  ├── 2. 含有循环结构（Cyclic Structures，防止无限展开）
  ├── 3. 含有未初始化字段（Uninitialized Fields，破坏目标类型有效性契约）
  └── 4. 含有带 Drop 析构责任的字段（Drop Fields，编译期对象不能逃避或伪造运行期析构责任！）
```

让我们看一段正例与拒绝例的直观对比：

```flurry
-- ✅ 正例：纯值字段组成的遥测配置结构体，所有成员均可安全跨阶段，自动支持跨阶段传递！
struct CalibrationConfig {
    offset: i32,
    scale: u32,
    enabled: bool,
}

-- ❌ 拒绝自动派生例 1：包含指向编译期宿主内存的裸指针/引用字段
struct HostPointerBad {
    ptr: *i32,
}

-- ❌ 拒绝自动派生例 2：包含带有运行期 Drop 清理责任的字段
struct ArmedResourceBad {
    handle: OwnedFile, -- OwnedFile 实现了 Drop
}
```

> 💡 **工程贴士**：自动派生失败，并不意味着禁止你为该类型**显式手写 `impl Splice`**！如果你的自定义类型确实需要跨阶段传递（例如在编译期记录文件路径字符串，在 `fn splice(comptime data: Self)` 的运行期函数体里重新打开文件构造运行期句柄），你完全可以显式编写一份符合契约的 `impl Splice`！

---

## 六、本节核心概念速查表

| 核心机制 | 关键语法 / 接口 | 语义边界与安全铁律 |
| :--- | :--- | :--- |
| **`Eq` / `Hash` 编译期相** | `comptime impl Eq` / `Hash` | 支撑编译期 Query 缓存与静态查找表；不可替代 PTC 的逻辑类型等式 |
| **`Splice` 跨阶段协议** | `trait Splice { assoc RuntimeTarget: Type; fn splice(comptime data: Self) -> RuntimeTarget; }` | `splice` 是接收 `comptime` 参数的 **Runtime 方法**：编译期按值实例化，运行期产出目标值 |
| **宿主地址隔离铁律** | 编译期引擎 $\not\to$ 运行期指针 | 目标程序绝不把编译期解释器的宿主内存地址当作有效运行期对象地址 |
| **`Splice` 有限自动派生** | 适用于 `struct` / `enum` / `union` / `closure` | 自动检查所有成员；遇到**共享引用、循环结构、未初始化字段、Drop 字段**时严禁自动派生 |

---

## 七、下一期预告

到这里，我们已经打通了 Trait 在编译期与运行期两个宇宙的底层脉络。
现在，让我们回到日常写得最多、最顺手的 Flurry 表面语法：
- 当你为自定义传感器度量写下 `a + b`、`-value`、`not flag` 时；
- 当你写下指针解引用 `ptr.*`、取址 `val.ref`、数组下标 `tensor[i]` 时；
- 甚至当你写下酷炫的自定义物理量字面量后缀 **`32px`**、**`25ms`**，或者用 **`checked_count(12).use(handler)`** 优雅地恢复错误时——

在这些看似浑然天成的语法糖背后，编译器到底连接了哪些正规的 **Trait 语言协议（Language Protocols）**？如果右操作数是一个需要上下文推导类型的枚举字面量（如 `value + .Ready`），算术协议又是如何反向为右操作数提供类型上下文的？

下一期 **《Flurry 演进手记 (102)：揭秘语法糖背后的正规军——算术、比较、索引与 LiteralInto 语言协议全览》**，我们将为你献上一份完整的 Flurry 语言级协议藏宝图！
