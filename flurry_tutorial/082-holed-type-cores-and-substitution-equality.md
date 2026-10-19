# Flurry 演进手记 (082)：为什么带洞的 Core 不等于完整类型——NodeIndex、共享环境与替换相等性

在上一回（081）中，我们见证了运行时依赖类型 `NodeIndex<storage.ref>` 的神奇之处：它的运行期值体只占 4 字节（`index: u32`），却能在静态类型上精确区分 `NodeIndex<first_storage.ref>` 与 `NodeIndex<second_storage.ref>`。

当时我们留下了一个关乎编译器底层生死存亡的架构悬念：
在一个大型张量或传感器工程里，可能会有成百上千个不同运行期实例特化出来的依赖类型（比如 `Sensor<first.ref>`、`Sensor<second.ref>`……）。
1. 如果编译器为每个运行期绑定都克隆一整棵类型语法树，编译期内存会瞬间爆炸；
2. 反过来，如果编译器把 `Sensor` 的公共结构抽成一个可驻留（Interned）的“带洞骨架” `Sensor<H0>`，那么当检查两个类型是否相等时，**为什么绝对不能认为“共享同一个 Core 就等于类型相等”**？
3. 当一个复合结构体 `SensorPair` 内部同时包含两个依赖同一个 `calibration` 的 `Sensor<calibration>` 字段时，编译器怎么做到既不重复存储环境指针，又能准确找到环境？

今天，我们就来拆解 Flurry 编译器中这条不可逾越的语义红线（`AGENTS.md` §5）：**可驻留的带洞类型核（`TypeCore`）、引用端替换（Reference-Side `Substitution`）与独立的环境获取计划（`EnvironmentPlan`）**。

---

## 一、 破局与痛点：为什么不能把运行期变量直接塞进驻留池（Intern Pool）？

在传统编译器中，为了加速类型比较（让 `TypeA == TypeB` 变成 $O(1)$ 的整数 ID 比较），通常会将所有类型做哈希驻留（Hash Consing / Interning）。

但一旦引入**运行时依赖类型**，天真的驻留方案（`v0`）立刻会撞上两堵墙：

```
  [ ❌ v0 方案 A：把局部运行期变量 ID 直接写进驻留的 Type 结构里 ]
  • 痛点：局部运行期变量随着每个函数作用域生灭，根本无法跨作用域稳定复用类型结构！
          成千上万个局部实例会把全局驻留池撑爆成垃圾场。

  [ ❌ v0 方案 B：驻留时抹掉运行期变量，只留一个洞 H0，然后直接用 Core ID 判等 ]
  • 痛点：Sensor<first.ref> 的 Core 是 Apply(SensorDecl, [H0])
          Sensor<second.ref> 的 Core 也是 Apply(SensorDecl, [H0])
          如果只比对 Core ID，编译器会荒谬地判定 Sensor<first.ref> == Sensor<second.ref>！
```

怎么同时兼顾**“公共结构 $O(1)$ 驻留共享”**与**“运行期依赖身份严格区分”**？

---

## 二、 核心心智模型：`TypeRef = (Interned TypeCore, Substitution)`

Flurry 的解法极其优雅：**将“可共享的带洞骨架（`TypeCore`）”与“引用端携带的填洞映射（`Substitution`）”彻底解耦！**

一个完整的类型引用（`TypeRef`）由两部分组成，而底层的运行期环境获取方式（`EnvironmentPlan`）则作为第三维信息另行维护：

```
+-------------------------------------------------------------------------+
|                    Flurry 运行时依赖类型的三元架构                        |
+-------------------------------------------------------------------------+
|                                                                         |
|  1. 带洞类型核 (Interned TypeCore) —— 全局驻留、完全不可变：              |
|     Core_Sensor = Apply(SensorDeclaration, [H0])                        |
|     其中 H0 是抽象模板洞 (Template Hole: lift *Calibration)              |
|                                                                         |
|  2. 引用端替换表 (Reference-Side Substitution) —— 区分实例身份的关键：    |
|     • first_sensor 的类型  = (Core_Sensor, { H0 ↦ R_first  })           |
|     • second_sensor 的类型 = (Core_Sensor, { H0 ↦ R_second })           |
|     ★ 完整类型相等性判定：                                                |
|       TypeA == TypeB  <==>  (CoreA == CoreB) AND (SubstA == SubstB)！   |
|                                                                         |
|  3. 环境获取计划 (EnvironmentPlan) —— 降码阶段如何拿到校准数据：          |
|     • 可能是当前栈帧局部变量、外层宿主结构转发，或伴随环境参数             |
|     • 绝不混入类型相等性比较（怎么拿到指针，不改变它是哪个指针）！         |
+-------------------------------------------------------------------------+
```

### 2.1 严格区分三种“占位符”！

在阅读或开发 Flurry 编译器时，千万不能把以下三种名字里都带“占位/变量”的概念混为一谈：

| 占位符种类 | 记号示例 | 存活位置 | 角色与可变性 |
| :--- | :--- | :--- | :--- |
| **1. 模板洞（Template Hole）** | `H0`, `H1` | 驻留的 `TypeCore` 内部 | **不可变接口槽位**。代表类型定义声明的运行期依赖形参位置 |
| **2. 运行期绑定项（Runtime Binder Term）** | `R_first`, `R_second` | 引用端的 `Substitution` 映射值中 | **固定运行期对象身份**。代表某个具体的运行期变量/引用的稳定身份 |
| **3. 推导元变量（Inference Metavariable）** | `?T0`, `?M1` | 局部 Elaboration 求解器表格中 | **待求解的临时未知量**。严禁写入全局共享的 `TypeCore` 或跨定义冻结！ |

---

## 三、 `v1` 升级：校准传感器 `Sensor<calibration>` 的完整类型相等性

让我们用一个工程感极强的传感器校准（`Calibration` + `Sensor`）场景，把 `TypeCore` 与 `Substitution` 的工作过程展现出来。

每个 `Sensor` 本身只存储一个未经校准的原始读数 `raw: i32`（4 字节），而它的偏置和缩放系数存放在运行期校准对象 `Calibration` 中：

```flurry
struct Calibration {
    offset: i32,
    scale: i32,
}

struct Sensor
where
    quote calibration: lift *Calibration
{
    raw: i32,

    -- 读取校准后的数值：自动从类型环境中的 calibration 获取 scale 和 offset
    pub fn reading(self: *Self) -> i32 {
        return self.raw * calibration.scale + calibration.offset;
    }
}

test "shared TypeCore does NOT mean equal types v1" {
    let first = Calibration { offset: 2, scale: 3 };
    let second = Calibration { offset: 2, scale: 3 };

    -- first_sensor 的内部表示为：(Core_Sensor, { H0 ↦ R_first })
    let first_sensor = Sensor<first.ref> { raw: 4 };
    -- second_sensor 的内部表示为：(Core_Sensor, { H0 ↦ R_second })
    let second_sensor = Sensor<second.ref> { raw: 4 };

    assert(first_sensor.reading() == 14);
    assert(second_sensor.reading() == 14);

    -- 尽管它们共享同一个驻留的 Core_Sensor，且运行期读数都是 14，
    -- 但因为引用端 Substitution 中 { H0 ↦ R_first } != { H0 ↦ R_second }，
    -- 它们的完整类型严格不相等！
    assert(first_sensor'type == Sensor<first.ref>);
    assert(second_sensor'type == Sensor<second.ref>);
    assert(first_sensor'type != second_sensor'type);
}
```

如果有人写了一个只接受 `Sensor<first.ref>` 的校准汇总函数，却误把 `second_sensor` 传了进去：

```flurry
-- ❌ 编译期直接拒绝！共享 Core_Sensor 不等于完整类型相等：
-- 替换表 { H0 ↦ R_first } 与 { H0 ↦ R_second } 不匹配！
fn reject_mismatched_substitution(
    first: *Calibration,
    second: *Calibration,
) {
    let bad: Sensor<first> = Sensor<second> { raw: 4 };
}
```

---

## 四、 `v2` 进阶：复合结构 `SensorPair` 中的环境共享与转发

现在，让我们考虑一个更妙的组合场景：
我们要定义一个差分传感器对 `SensorPair`，它内部包含两个传感器字段 `first` 和 `second`，而且这两个子传感器**共享同一个运行期校准配置 `calibration`**！

如果用传统的胖结构体方案，每个 `Sensor` 存一个指针，`SensorPair` 就要存两遍完全相同的 `*Calibration` 指针（浪费 16 字节）。
而在 Flurry 中，当 `SensorPair` 声明了 `where quote calibration: lift *Calibration`，并在字段里写下 `first: Sensor<calibration>` 和 `second: Sensor<calibration>` 时，会发生什么？

```
  [ SensorPair<calibration.ref> 的内部替换与内存布局 ]

  • 外层 TypeCore (Core_SensorPair):
      包含模板洞 H_outer : lift *Calibration
      字段 first  类型引用: (Core_Sensor, { H0 ↦ H_outer })  <-- 洞到洞的复合转发！
      字段 second 类型引用: (Core_Sensor, { H0 ↦ H_outer })

  • 物理内存布局 (Value Body):
      +------------------+------------------+
      | first.raw (4B)   | second.raw (4B)  |  ==> 总共仅 8 字节！零冗余指针！
      +------------------+------------------+
```

看！`first` 和 `second` 的引用端替换表直接把内层的洞 `H0` 映射到了外层 `SensorPair` 的洞 `H_outer`！当调用 `pair.total()` 在内部调用 `self.first.reading() + self.second.reading()` 时，编译器通过独立维护的 `EnvironmentPlan`，直接把外层 `SensorPair` 的 `calibration` 环境同时供给两个子字段调用！

让我们写出这段极度紧凑的 `v2` 代码：

```flurry
struct SensorPair
where
    quote calibration: lift *Calibration
{
    -- 两个子字段直接引用外层环境的 calibration，绝不在每个字段里重复塞指针！
    first: Sensor<calibration>,
    second: Sensor<calibration>,

    pub fn total(self: *Self) -> i32 {
        return self.first.reading() + self.second.reading();
    }
}

test "composed runtime dependent environment sharing v2" {
    let calibration = Calibration { offset: 1, scale: 10 };

    let pair = SensorPair<calibration.ref> {
        first: Sensor<calibration.ref> { raw: 2 },
        second: Sensor<calibration.ref> { raw: 5 },
    };

    -- (2 * 10 + 1) + (5 * 10 + 1) = 21 + 51 = 72
    assert(pair.total() == 72);
    -- 验证物理骨骼：两个 Sensor 只存了两个 i32 (4 + 4 = 8 字节)，没有夹带任何指针！
    assert(Type.size_of(pair'type) == 8);
}
```

`Type.size_of(pair'type) == 8`！
既有依赖类型级别的静态安全，又有手写 C 语言裸 `int32_t[2]` 的极致内存密度。这就是把 `TypeCore`、`Substitution` 与 `EnvironmentPlan` 拆成正交三元组带来的威力。

---

## 五、 本篇小结

| 架构组件 | 核心职责 | 语义红线（AGENTS.md §5） |
| :--- | :--- | :--- |
| **带洞类型核 (`TypeCore`)** | 提取去掉具体运行期绑定后的公共类型结构（含 `H0, H1...`）并全局驻留 | 严禁把局部推导元变量 `?T` 或瞬时状态写进共享 `TypeCore` |
| **引用端替换 (`Substitution`)** | 在使用端记录每个洞 `Hi` 填入的具体运行期身份 `R_x` 或外层洞 `H_outer` | **共享 `TypeCore` 绝不等于完整类型相等**；判等必须同时比对 `Substitution` |
| **环境获取计划 (`EnvironmentPlan`)** | 记录降码阶段从哪里取得运行期环境值（局部栈、外层转发、隐式参数） | 环境怎么取得属于编译表示细节，不改变类型身份，也不强迫值体内联指针 |

---

## 六、 留给下一篇的悬念

到现在为止，我们已经学会了怎么**构造**带 Normal-Form 头的泛型类型（如 `Array<T, N>`、`Tuple<...>`、`Box<T, value>`），也学会了怎么在编译期用 `Type.size_of` 观察它们。

但是，在编写静态形状张量引擎 `Tensor<T, S>` 的广播（Broadcasting）与算子派发器时，我们经常需要反过来做一件事——**拆解（Deconstruct）一个已知的 `Type`**！
比如：给你一个任意的 `T: Type`，我想用 `match` 直接匹配它：
- “如果 `T` 是元素固定为 `u32` 的数组 `Array<'u32, length>`，请帮我把它的长度 `length` 抽出来！”
- “如果 `T` 是任意元素的数组 `Array<Element, length>`，请同时帮我把元素类型绑定到 `Element`、长度绑定到 `length`！”

慢着！在 `Array<'u32, length>` 里，为什么 `u32` 前面要加一个单引号写成 **`'u32`**，而 `length` 前面却不加单引号？如果不加这个单引号会发生什么？

下一回 **《Flurry 演进手记 (083)：在类型自身上做模式匹配——expr<type_pattern_argument*> 类型模式详解》**，我们将揭晓类型模式匹配与 `'id` 语法的精妙设计！
