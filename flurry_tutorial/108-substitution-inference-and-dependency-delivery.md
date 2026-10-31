# Flurry 演进手记 (108)：开放实现绑定是怎么传递环境的——替换推导（Substitution Inference）与依赖交付

在开发工业传感器网关与底层张量库时，我们经常遇到一个让接口设计者头疼不已的两难困境：

假设我们定义了一个干净利落的标准测量接口 `trait Measure { fn measure(*self) -> i32; }`。
接口签名本身非常简单，排斥接收任何额外参数。
但是，当我们为具体的传感器读数 `Sensor` 或 `Reading` 实现 `Measure` 时，真实的工程需求来了：
**原始硬件读数必须加上一个外部运行时校准器对象 `calibration: *Calibration` 里的偏移量 `offset`，才能算出真正的测量结果。**
或者在张量库里，`Tensor` 的 `Clone` / `Drop` 接口签名本身未带分配器参数，但在执行时却必须访问该张量绑定的运行时内存分配器 `allocator`。

如果缺少从方法接收者反向推导环境依赖的能力，通常只能在以下两种做法中选择：
1. **污染 Trait 签名**：被迫把 `fn measure(*self, cal: *Calibration)` 写进 `Measure` 接口里——结果那些全然无需校准器的简单传感器也被迫陪绑传一个空指针；
2. **把指针冗余塞进每个对象字段里**：逼着每个 4 字节的 `Reading` 结构体都额外存一个 8 字节的 `*Calibration` 指针字段，内存瞬间膨胀 3 倍。

在 Flurry 中，我们全然无须做这种妥协。
因为 Flurry 的 `impl` 块允许通过 `where quote calibration: lift *Calibration` 声明**运行时依赖参数**，并通过一套严密的**替换推导（Substitution Inference）与环境交付（Environment Delivery）**机制，既能从目标类型自动恢复依赖，也能用 `reading.as(Measure)<.calibration = ...>` 显式注入环境。

今天，我们就来理解这套连接类型替换与运行时环境的精妙引擎。

---

## 既不想污染 Trait 签名，又不想把指针塞满每个小结构体

让我们先把问题具体化。我们有一个校准参数结构体 `Calibration` 和一个纯净的 `Measure` 接口：

```flurry
struct Calibration {
    offset: i32,
}

trait Measure {
    -- 接口保持严格纯净。不强迫所有实现者都传 Calibration 参数
    fn measure(*self) -> i32;
}
```

现在，某款温度传感器的测量逻辑是 `self.value.* + calibration.offset.*`。
既然 `measure(*self)` 的圆括号参数列表里没有 `calibration`，这个运行时的 `calibration` 对象到底怎么交给 `impl Measure` 呢？

在 Flurry 中，答案分为两种场景：
- **场景一**：传感器类型本身就已经通过运行时依赖类型参数记录了它属于哪个校准器（如 `Sensor<calibration>`）——此时编译器能**自动从目标类型恢复依赖**。
- **场景二**：读数结构体只是一个纯粹的裸值（如 `struct Reading { value: i32 }`），类型里根本没有记录校准器——此时该 `impl` 拥有一个**开放实现参数（Open Implementation Parameter）**，调用者可以在调用视图上用 `<.calibration = ...>` 显式绑定。

让我们依次演进这两种场景。

---

## 从目标类型自动恢复依赖（Target Type Substitution Recovery）

还记得我们在第 073 篇学过的 `quote` 子句参数与 `lift` 吗？
当我们在定义 `Sensor` 结构体时，写下 `where quote calibration: lift *Calibration`，`Sensor<calibration.ref>` 的类型身份里就记录了它绑定的那个运行时校准器对象的身份（Identity）——注意，它无需把指针作为冗余物理字段塞进每个实例里，也不在编译期偷读 `offset` 的运行时内容。

当我们为 `Sensor<calibration>` 实现 `Measure` 时，编译器会自动完成推导：

```flurry
struct Calibration {
    offset: i32,
}

trait Measure {
    fn measure(*self) -> i32;
}

-- Sensor 在类型层记录了它依赖哪个运行时 calibration 对象身份
struct Sensor
where quote calibration: lift *Calibration
{
    value: i32,
}

-- impl 头直接匹配 Sensor<calibration>。
impl Measure for Sensor<calibration>
where quote calibration: lift *Calibration
{
    fn measure(*self) -> i32 {
        -- 直接访问该运行时依赖对象 calibration。
        self.value.* + calibration.offset.*
    }
}

fn show_sensor(sensor: *Sensor<calibration>)
where quote calibration: lift *Calibration
{
    -- 无需传任何额外参数。
    -- 编译器从 sensor 的类型 *Sensor<calibration> 自动推导出替换 σ[calibration]，
    -- 并在运行时通过环境计划（Environment Plan）取得该 calibration 对象。
    println("{}", sensor.measure());
}

test {
    let calibration = Calibration { offset: 3 }
    let sensor = Sensor<calibration.ref> { value: 10 }
    show_sensor(sensor.ref);
}
```

运行这段代码，直接输出 `13`。

下面拆解调用 `sensor.measure()` 时，编译器在幕后建立的**实现证据（Evidence）**：

```
+-------------------------------------------------------------------------+
|  从目标类型恢复替换（Substitution）与环境交付                  |
|                                                                         |
|  调用点：sensor.measure()，其中 sensor : *Sensor<calibration>           |
|                                                                         |
|  编译器生成的实现证据 Evidence：                                        |
|  +-------------------------------------------------------------------+  |
|  | 1. 选定实现 (impl_id)   : Measure for Sensor<calibration>         |  |
|  | 2. 替换映射 (σ)         :                                         |  |
|  |    • 从目标类型 Sensor<calibration> 匹配恢复：                    |  |
|  |      σ[calibration] = 调用上下文中的 calibration 绑定身份         |  |
|  | 3. 环境取得计划 (Environment Plan) :                              |  |
|  |    • 说明在执行 measure() 时，从哪条合法访问路径取得该对象        |  |
|  +-------------------------------------------------------------------+  |
|  💡 工程彩蛋：                                                          |
|  Flurry 标准库中 `Tensor` 的内存分配器（Allocator）正是采用这套规则     |
|  当 `Tensor` 类型参数记录了 allocator 身份时，`Clone`、`Drop` 等操作    |
|  直接从类型匹配中恢复同一个 allocator identity 并通过环境兑现访问，     |
|  完全不需要搞一套按变量名乱猜的隐式搜索                                 |
+-------------------------------------------------------------------------+
```

---

## 开放实现视角绑定——`reading.as(Measure)<.calibration = ...>`

那么，如果我们的结构体只是一个最普通的 4 字节裸读数 `struct Reading { value: i32 }`，它的类型上面**根本没有** `<calibration>` 参数呢？
这时候，我们依然可以为 `Reading` 编写一个依赖 `calibration` 的 `Measure` 实现：

```flurry
struct Reading {
    value: i32,
}

-- 注意看。主体 Reading 的类型里并没有出现 calibration。
-- 因此，calibration 成为了这份 impl 的一个“开放参数（Open Parameter）”。
impl Measure for Reading
where quote calibration: lift *Calibration
{
    fn measure(*self) -> i32 {
        self.value.* + calibration.offset.*
    }
}
```

因为从 `reading: Reading` 的类型里根本推导不出 `calibration` 是谁，所以在调用 `measure()` 之前，这个开放参数还没有来源。

怎么在调用点把校准器交给这份实现？
使用 Flurry 的**开放实现视角语法**：**`value.as(Trait)<.dependency = object>`**。

```flurry
test {
    let first = Calibration { offset: 3 }
    let second = Calibration { offset: 8 }
    let reading = Reading { value: 10 }

    -- 1. 临时绑定 first 校准器调用 measure() ──► 输出 13
    println("{}", reading.as(Measure)<.calibration = first.ref>.measure());

    -- 2. 同一份 impl，换绑 second 校准器调用 measure() ──► 输出 18
    println("{}", reading.as(Measure)<.calibration = second.ref>.measure());
}
```

甚至，你还可以把已经封闭好开放参数的接口视角保存到一个局部变量里，在依赖对象的有效期内反复调用，避免每次重复书写 `<.calibration = ...>`：

```flurry
fn show_calibrated(reading: *Reading, calibration: *Calibration) {
    -- 将封闭了 .calibration 环境的接口视角保存为 measured
    let measured = reading.as(Measure)<.calibration = calibration>;

    -- 后续调用直接复用已固定的绑定与环境关系。
    println("{}", measured.measure());
    println("{}", measured.measure());
}
```

```
+-------------------------------------------------------------------------+
|  开放实现视角 `reading.as(Measure)<.calibration = c>`          |
|                                                                         |
|   reading : *Reading                                                    |
|      │                                                                  |
|      ▼  .as(Measure) ──► 选中 `impl Measure for Reading`                |
|   [ 开放实现视图：尚缺开放参数 `calibration` ]                          |
|      │                                                                  |
|      ▼  <.calibration = first.ref> ──► 封闭开放参数，建立环境访问计划   |
|   [ 完整已封闭视角：主体=*Reading, σ[calibration]=first.ref ]           |
|      │                                                                  |
|      ▼  .measure()                                                      |
|   执行 `self.value.* + first.offset.*` ===> 13                          |
+-------------------------------------------------------------------------+
|  ⚠️ 注意：                                                              |
|  • 尖括号 `<.calibration = ...>` 绑定的是**所选 impl 的开放参数**，     |
|    它既没有修改 `Measure` Trait 的签名，也没有给 `measure()` 增加实参   |
|  • 两次绑定 `<.calibration = first.ref>` 与 `<.calibration = second.ref>`|
|    复用的是同一份全局 `impl`，并不会制造两份冲突的全局实现              |
+-------------------------------------------------------------------------+
```

---

## 编译期安全红线：不靠“局部变量刚好同名”来瞎猜依赖

有些支持隐式参数（Implicits）的语言（如 Scala 2）会在当前作用域里随便找一个同类型或者同名的局部变量自动塞进去，导致“改了个局部变量名或者多引进了一个变量，代码行为就悄悄变了”的棘手维护问题。

在 Flurry 中，`docs/type/substitution-inference.md` 立下了一条核心规则：
> **没有来源的依赖保留为开放参数，调用能力交付前必须显式补齐；作用域中恰好存在一个同类型或同名对象，不是合法补齐依据。**

来看这个被编译器编译期报错的经典负例：

```flurry
-- ❌ 编译期直接报错。
-- 哪怕当前函数的参数名就叫 `calibration`，类型也是 `*Calibration`，
-- 编译器亦免于靠“变量同名或同类型”去自动猜测开放参数绑定。
fn missing_binding(reading: *Reading, calibration: *Calibration) -> i32 {
    reading.as(Measure).measure()
}

-- ✅ 正确写法：必须通过 `<.calibration = calibration>` 显式绑定开放参数。
fn valid_binding(reading: *Reading, calibration: *Calibration) -> i32 {
    reading.as(Measure)<.calibration = calibration>.measure()
}
```

此外，还要记住两条重要的生命周期与身份原则：
1. **无法强行改绑已固定的依赖**：如果目标类型 `Sensor<cal1.ref>` 已经固定了 `cal1`，你禁用 `<.calibration = cal2.ref>` 去强行覆盖它（除并非编译器能证明两者是同一个 runtime identity）。而且两个对象哪怕字段内容一模一样，也不代表它们的 runtime identity 相同。
2. **知道是谁 $\neq$ 当前能拿到**：跨函数返回值、闭包或动态接口边界时，操作所需的环境必须真正随边界交付；如果外部环境已经销毁失效，编译器会在边界处直接报错，免于在后续使用点随便抓一个替代品顶包。

---

## 本篇小结

| 机制 / 语法 | 适用场景 | 编译器行为与安全边界 |
| :--- | :--- | :--- |
| **`impl Trait for Type<dep>`**<br>（从目标类型恢复） | 主体类型本身已携带运行时依赖参数（如 `Sensor<calibration>` 或带 allocator 的 `Tensor`） | 匹配目标类型时自动恢复替换映射 $\sigma[\text{dep}]$，无需调用方重复手写绑定 |
| **`val.as(Trait)<.dep = obj>`**<br>（开放实现显式绑定） | 主体类型未携带该依赖（如 `Reading`），`impl` 含有未决的开放参数 | 在接口视角上显式封闭开放参数并交付环境；可保存封闭后的视角反复调用 |
| **禁止按名/按类型盲猜** | 开放参数未从目标类型恢复、且未显式 `<.dep = ...>` 绑定 | **编译期直接报错。** 即使当前作用域存在同名同类型的局部变量也不隐式吸附 |
| **替换 $\sigma$ 与环境计划分离** | 所有携带依赖的实现调用 | 替换 $\sigma$ 确定“依赖是谁（Identity）”，环境计划（Environment Plan）确保“执行时如何合法访问它” |

---

### 下一篇预告

到现在为止，我们已经解决了 Trait 在动态分派（`dyn Trait`）和运行时环境传递（开放实现绑定）上的难题。
接下来，我们要正式向第五卷的核心工程目标——**工业级零拷贝通用编解码器框架（Universal Zero-Copy Codec Framework：`Encode` / `Decode`）**发起总攻了。

但在动手写 `trait Encode` 的第一行代码前，我们就遇到了一个所有序列化框架都会撞上的拦路虎：
- 当我们把传感器报文编码写入**内存固定数组 `RingBuffer`** 时，可能只会发生 `![BufferOverflowError]`，而且是纯同步操作（没有任何代数效应 `#`）；
- 当我们把同样的报文编码写入**网络 Socket 流 `TcpStream`** 时，非但可能发生 `![IoError, TimeoutError]`，而且每次写入还会触发异步调度效应 **`#[async_write]`**。
- 当我们写入一个**无限增容的内存 `ByteVec`** 时，它甚至**永免报错（空错误行 `![ ]`）**，也没有任何效应。

如果我们在 `trait Encode` 的签名里把错误类型写死成 `IoError`，内存编码器就得陪着处理断无可能发生的 `IoError`；如果写死成无错误，网络编码器又没法报错。

**怎么让同一个 Trait 方法，根据不同的实现者或底层写入流，优雅地多态化它的“错误行（`!Err`）”和“效应行（`#Eff`）”？**
下一篇 **`109-associated-error-and-effect-rows-in-traits.md`**，我们将揭开关联项与**错误行/效应行多态（Row Polymorphism）**的强强联合。
