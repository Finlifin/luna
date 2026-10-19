# 088 — 行多态（Row Polymorphism）：错误集合与代数效应的本质差异

> **阅读指南**：在上一回（第 087 回）认识 PTC 的 11 位理论求解器神探时，我们留了一个耐人寻味的悬念：在 Flurry 中，可恢复错误 `![E1, E2] T` 和代数效应 `#[eff1, eff2] T` 都由第 8 号神探 **`ROW`（行求解器）** 管理。为什么同样叫“行（Row）”，错误行会自动把重复的错误类型去重合并（`![E] ![E] T == ![E] T`），而效应行却顽固地保留每一个重复的效应标签（`#[eff] #[eff] T != #[eff] T`）？当行里出现未知行变量 $\rho$ 时，求解器又是怎样做“行合一（Row Unification）”的？本篇我们将彻底讲透 **行多态（Row Polymorphism）** 与 **作用域标签行（Scoped-Label Rows）**。

---

## 一、 痛点：为什么不能把“错误”和“效应”都当成普通的集合（Set）？

当我们组合多个可能失败或可能触发代数效应的函数时，最希望看到的是编译器能自动帮我们合并它们的签名，或者通过一个行变量（Row Variable）把“其余错误/其余效应”透传出去。这在类型理论中被称为 **行多态（Row Polymorphism）**。

### 尝试 `v0`：一刀切地用“无序不重复集合”建模所有行

在设计编译器的第一天，最容易冒出来的想法（`v0`）是：“管它是错误行 `![...]` 还是效应行 `#[...]`，统统当成数学上的无序集合（Set）不就好了吗？合并就是求并集（Union），重复的标签直接删掉！”

对于**可恢复错误（Error Rows）**，这个想法简直完美：
- 如果一个函数既可能在第一阶段抛出 `InputError`，又可能在第二阶段抛出 `InputError`，对调用者来说，这个函数失败时返回的错误原因依然只是 `InputError`。
- 因此，`![InputError] ![InputError] u32` 和单层的 `![InputError] u32` 在语义和 ABI 上完全等价！

但是，如果我们把这套“自动去重”的集合逻辑照搬到 **代数效应（Effect Rows）** 上，大祸就临头了！

想象一下，我们在写一个支持嵌套事务或嵌套数据源的效应程序，外层有一个 `handle` 处理器负责拦截外层的 `read_number` 效应，内层还有一个 `handle` 处理器负责拦截内层的 `read_number` 效应：

```flurry
effect fn read_number() -> u32;

-- 假设一段计算需要向【内层】处理器请求一次，再向【外层】处理器透传一次：
-- 它的类型携带了两层独立的效应标签：#[read_number] #[read_number] u32
```

如果在 `v0` 里，编译器自作聪明地把 `#[read_number] #[read_number] u32` 自动去重压缩成了单层的 `#[read_number] u32`：
- 当最内层的 `handle` 处理器接管并消解掉一层 `read_number` 后，剩下的类型就会直接变成纯值 `u32`！
- 本该由外层处理器拦截的那一层效应凭空蒸发了，或者反过来，内层处理器错误地把本属于外层作用域的效应一锅端掉了！

**核心痛点**：
1. **错误是“扁平的标签集合”**：调用者只关心“最终可能拿到哪几种错误类型”，不在乎同一个错误是从哪一层传上来的。
2. **效应是“带作用域层级的证据通道（Scoped Channels）”**：外层包裹一圈 `#[eff]`，代表外层多了一重待由对应 `handle` 消解的效应处理器边界！

---

## 二、 破局（v1）：一行两制——唯一标签行 vs. 作用域标签行

为了同时兼顾错误处理的简洁性与代数效应的作用域精确性，Flurry 的 `ROW` 理论求解器将行严格划分为两种截然不同的代数结构：

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                    ROW 求解器的“一行两制”对照图                         │
├────────────────────────────────────┬────────────────────────────────────┤
│ 1. 错误行 (Error Rows)             │ 2. 效应行 (Effect Rows)            │
│    语法: ![E1, E2] T               │    语法: #[eff1, eff2] T           │
├────────────────────────────────────┼────────────────────────────────────┤
│ ▶ 数学本质: 唯一标签集 (Unique Set)│ ▶ 数学本质: 作用域多重行 (Scoped)  │
│ ▶ 交换律: 任意两个错误标签均可交换 │ ▶ 交换律: 仅【不相等】的不同标签   │
│   ![A, B] T == ![B, A] T           │   可交换: #[a, b] T == #[b, a] T   │
│ ▶ 幂等去重: 相同错误自动合并去重！ │ ▶ 保留重数: 相同标签绝不合并去重！ │
│   ![E] ![E] T  ===  ![E] T         │   #[e] #[e] T  !==  #[e] T         │
│   (严格扩展 ⟨e | ρ⟩ 要求 Lacks)    │   (每消解一层 handle 只剥离一层 e) │
└────────────────────────────────────┴────────────────────────────────────┘
```

> **💡 核心心智模型：效应行里的重复标签代表什么？（千万别误解！）**
> 初学者看到 `#[read_number] #[read_number] u32` 时，最容易产生的一个误区是：“这难道代表函数体里调用了两次 `read_number()` 吗？”
> **绝对不是！**
> - 在同一个效应作用域内，无论你调用 `read_number()` 1 次、2 次还是在循环里调用 10000 次，它的类型都只是**单层**的 `#[read_number] u32`！
> - 只有当类型层面发生了**多层效应行叠加/嵌套遮蔽（Scoped Layering / Tunneling）**时，才会出现两个同名的 `read_number` 标签。每一个同名标签对应着一层独立的 `handle` 拦截边界，它们的相对顺序固定了哪一层归内层处理器、哪一层归外层处理器！

### 演进到 `v1`：用编译期类型相等测试验证“一行两制”

让我们直接写一段 Flurry 代码，交给 PTC 的 `ROW` 求解器来现场验证这两种行的恒等式：

```flurry
use std.prelude.*;

struct InputError {}

effect fn read_number() -> u32;

const SAME_ERROR_ROW =
    (![InputError] u32) == (![InputError] ![InputError] u32);

const SAME_EFFECT_ROW =
    (#[read_number] u32) == (#[read_number] #[read_number] u32);

comptime {
    -- ✅ 错误行是唯一标签集：重复的 InputError 会被自动合并，结果为 true！
    assumes SAME_ERROR_ROW;
    -- ✅ 效应行是作用域行：两层 read_number 与一层绝不相等，结果为 false（取反为 true）！
    assumes not SAME_EFFECT_ROW;
}

test {
    println("error rows equal: {}", SAME_ERROR_ROW);
    println("effect rows equal: {}", SAME_EFFECT_ROW);
}
```

运行这段测试，输出清晰地印证了我们的心智模型：
- `error rows equal: true`
- `effect rows equal: false`

同时，如果我们在一个只允许单层 `#[read_number] u32` 的类型位置，硬塞进一个双层嵌套的效应类型，编译器会当场拦截：

```flurry
effect fn read_number() -> u32;

type NestedReadFn = fn() -> #[read_number] #[read_number] u32;

fn broken_row_assumption(callback: NestedReadFn) {
    -- ❌ 编译期直接拒绝！
    -- ROW 求解器判定：#[read_number] #[read_number] u32 与 #[read_number] u32
    -- 在标签重数（Multiplicity）上不匹配，拒绝隐式压扁！
    let _: fn() -> #[read_number] u32 = callback;
}
```

---

## 三、 深入引擎室：行合一（Row Unification）与保护尾（Protected Tail）（v2）

当泛型函数里出现未知的**行变量（Row Variable）** $\alpha, \beta$ 时，`ROW` 求解器又是怎么解方程的呢？

假设求解器要把两行 $\langle a \mid \alpha \rangle$ 和 $\langle b \mid \beta \rangle$ 合一（Unify）：
1. **同头消去**：如果两个头部标签相等（$a = b$），直接消去头部，继续递归合一尾部 $\alpha = \beta$。
   - 例如：$\langle a \mid \alpha \rangle = \langle a \rangle$（右侧尾部为空行 $\emptyset$），消去 $a$ 后立即解出 $\alpha = \emptyset$！
2. **异头暴露（Expose）**：如果 $a \neq b$，说明左侧头部的 $a$ 必须藏在右侧的未知尾 $\beta$ 里面，而右侧头部的 $b$ 也必须藏在左侧的未知尾 $\alpha$ 里面！求解器会引入一个全新的自由行变量 $\gamma$，将方程重写为：
   $$\alpha = \langle b \mid \gamma \rangle, \quad \beta = \langle a \mid \gamma \rangle$$

但是，如果两边共享了**同一个未知尾变量** $\alpha$ 呢？这时候就会遇到两个著名的“无限行陷阱”！

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                 ROW 求解器的两大“无限行拦截器”                          │
├────────────────────────────────────┬────────────────────────────────────┤
│ 陷阱 1：自吞噬方程                 │ 陷阱 2：共享尾冲突                 │
│   α  =  ⟨ a | α ⟩                  │   ⟨ a | α ⟩  =  ⟨ b | α ⟩  (a ≠ b) │
├────────────────────────────────────┼────────────────────────────────────┤
│ 如果强行展开：                     │ 左右两边拖着同一个尾巴 α！         │
│ α = ⟨a | ⟨a | ⟨a | ...⟩⟩⟩          │ 在有限行里，左边比 α 多一个 a，    │
│ 这是一条无限长的行！               │ 右边比 α 多一个 b (且 a ≠ b)。     │
│                                    │                                    │
│ 🛡️ 拦截机制：Occurs-Check          │ 🛡️ 拦截机制：Protected Tail        │
│ 变量 α 出现在了非空前缀之后，      │ 将公共尾 α 放入 protected_tail 集合│
│ 直接判定无有限解 (NotEntailed)！   │ 禁止对受保护的公共尾做 Expose 拆分!│
└────────────────────────────────────┴────────────────────────────────────┘
```

### 演进到 `v2`：为什么需要 `protected_tail`？

想象一下，如果没有 `protected_tail`（受保护公共尾）机制，当求解器看到 $\langle a \mid \alpha \rangle = \langle b \mid \alpha \rangle$（其中 $a \neq b$）时，它会冒失地对右边的 $\alpha$ 调用 `Expose`，令 $\alpha = \langle a \mid \gamma \rangle$。但别忘了，左边的尾巴**也是同一个 $\alpha$**！把 $\alpha = \langle a \mid \gamma \rangle$ 代回左边，方程又变成了 $\langle a, b \mid \gamma \rangle = \langle b, a \mid \gamma \rangle$，从而陷入无限生成新变量的死循环！

通过先识别出左右两侧共享同一个未决行尾 $\alpha$ 并将其加入 `protected_tail` 禁止拆分，`ROW` 求解器能在 $O(1)$ 时间内识破 $\langle a \mid \alpha \rangle = \langle b \mid \alpha \rangle$（$a \neq b$）无解，保证行合一算法永远**严格终止（Termination）**！

---

## 本篇小结

| 特性维度 | 错误行（`![E1, E2] T`） | 效应行（`#[eff1, eff2] T`） |
| :--- | :--- | :--- |
| **理论模型** | 唯一标签集合行（Unique-Label Row） | 作用域有序多重行（Scoped-Label Row） |
| **重复标签行为** | 自动合并去重：`![E] ![E] T == ![E] T` | 严格保留重数与同标签相对顺序：`#[e] #[e] T != #[e] T` |
| **不同标签交换** | 任意交换顺序 | 仅在证明两个标签不相等（$l_1 \neq l_2$）时允许交换 |
| **行合一防死循环** | 严格扩展约束 $\text{Lacks}(\rho, e)$ + Occurs-Check | 同标签保序消去 + 公共尾保护（`protected_tail`） |

---

### 留给下一篇的悬念

我们已经见识了 `ROW` 求解器如何把错误和效应治理得井井有条。现在，让我们把目光投向第 9 号神探 **`BV`（定宽位向量求解器）** 与 Flurry 的底层数值系统。

在写网络协议解析、硬件驱动或紧凑张量量化（如 4-bit 量化权重、5-bit 颜色通道、24-bit 音频采样）时，C/Rust 程序员总是被迫在 `u8`、`u16`、`u32` 里手写各种容易出错的 `& 0x1F` 和移位掩码。

在 Flurry 中，我们可以直接使用任意位宽的整数（如 `u4`、`u5`、`i24`）以及位向量视图 `'bitvec`！但一个最让人好奇的问题是：**一个 5 比特宽的 `u5`，单独放在内存里明明也要占 1 个字节（8 比特），为什么编译器说它的位表示宽度依然严格是 5 位，绝不会悄悄变成 `u8`？**

下一篇 **《089 — 任意位宽整数（Arbitrary-Sized Integers）：从 `u4`、`i24` 到位表示契约》**，我们将揭晓逻辑位宽与物理存储布局的精妙分野！
