# Flurry 演进手记 (186)：运行时依赖类型的工程模式——多实例身份隔离、外层环境共享与逃逸防线

在[上一篇（第 185 篇）](./185-interleaving-elaboration-comptime-and-freezing.md)中，我们理清了编译期类型构造头与类型计算函数的区别。

在信号处理、图计算与硬件驱动开发中，我们经常需要让一个轻量级的值（如 4 字节的整数读数或数组下标）在类型层面上绑定到某个具体的**运行时对象身份**（如特定的传感器校准器 `Calibration` 或存储池 `NodeStorage`）。这就是我们在第四卷认识的 **运行时依赖类型（Runtime-Dependent Types）**。

今天，我们通过一个双通道传感器校准系统，总结在工程中使用运行时依赖类型的三大核心法则：
1. **相同内存布局下的严格身份隔离**：为什么 `Sensor<first.ref>` 与 `Sensor<second.ref>` 布局完全相同（都只占 4 字节），却在静态类型上严格区分？
2. **外层容器与函数参数的环境供给**：如何在不给每个 `Sensor` 实例增加冗余指针字段的前提下，安全地读取所依赖的运行时校准数据？
3. **局部依赖的作用域逃逸防线**：为什么绑定了局部栈变量 `local_cal.ref` 的依赖类型禁止直接作为函数返回值逃逸？

---

## 一、 实战建模：零指针体积膨胀的通道校准传感器

假设每个传感器采样值本身只是一个 4 字节的原始整数 `raw: i32`，而它的缩放比例存放在运行时的 `Calibration` 结构体中：

```flurry
struct Calibration {
    scale: i32,
}

-- Sensor 的类型身份依赖于运行期校准器指针 cal
struct Sensor
where
    quote cal: lift *Calibration
{
    raw: i32,
}

-- 在显式传入 cal: *Calibration 的函数签名中，
-- 第二个参数的类型 Sensor<cal> 直接绑定到第一个参数 cal 的运行期身份。
unsafe fn read_scaled(
    cal: *Calibration,
    sensor: Sensor<cal>,
) -> i32 {
    sensor.raw * cal.scale.*
}
```

让我们仔细观察 `Sensor<cal>` 与 `read_scaled` 的设计：

```text
  ┌─────────────────────────────────────────────────────────────────────┐
  │ 1. 物理内存布局 (Value Body)                                         │
  │    Sensor<cal> 在内存中只存储 `raw: i32` (4 字节)，不内联 8 字节指针 │
  ├─────────────────────────────────────────────────────────────────────┤
  │ 2. 静态类型身份 (Type Identity)                                      │
  │    • left  的类型是 Sensor<first.ref>                                │
  │    • right 的类型是 Sensor<second.ref>                               │
  │    • 哪怕 first 和 second 的 scale 数值相同，两个类型也并不相等       │
  ├─────────────────────────────────────────────────────────────────────┤
  │ 3. 运行期环境获取 (Environment Supply)                               │
  │    在 read_scaled(cal, sensor) 中，cal 通过函数形参显式供应并受类型校验│
  └─────────────────────────────────────────────────────────────────────┘
```

来看一个包含两个独立校准通道的测试：

```flurry
test isolated_channel_calibrations {
    let first = Calibration { scale: 2 }
    let second = Calibration { scale: 10 }

    let left = Sensor<first.ref> { raw: 15 }
    let right = Sensor<second.ref> { raw: 15 }

    -- 物理布局验证：每个 Sensor 只占 4 字节 (i32)
    comptime {
        assumes Type.size_of(left'type) == 4;
        assumes left'type != right'type;
    }

    unsafe {
        assumes read_scaled(first.ref, left) == 30;
        assumes read_scaled(second.ref, right) == 150;
    }
}
```

---

## 二、 复合结构中的多通道依赖绑定

当我们需要在一个外层结构体或函数中同时管理两个绑定到不同校准器的 `Sensor` 时，可以通过 `where quote ...` 子句将外层环境的绑定清晰地转发给各个字段：

```flurry
struct DualChannelReading
where
    quote cal_a: lift *Calibration,
    quote cal_b: lift *Calibration
{
    ch_a: Sensor<cal_a>,
    ch_b: Sensor<cal_b>,
}

unsafe fn total_scaled_reading(
    cal_a: *Calibration,
    cal_b: *Calibration,
    dual: DualChannelReading<cal_a, cal_b>,
) -> i32 {
    read_scaled(cal_a, dual.ch_a) + read_scaled(cal_b, dual.ch_b)
}
```

因为 `dual.ch_a` 和 `dual.ch_b` 的类型分别锁死了 `cal_a` 和 `cal_b`，在实现 `total_scaled_reading` 时，难以误把 `dual.ch_b` 传给 `read_scaled(cal_a, ...)`。

---

## 三、 负例：跨通道混用校准器与局部依赖逃逸

### 负例 1：拿通道 A 的校准器去读取通道 B 的传感器

```flurry
unsafe fn mismatched_calibration(
    cal_a: *Calibration,
    cal_b: *Calibration,
    sensor_b: Sensor<cal_b>,
) -> i32 {
    -- ❌ 编译期报错：第一个参数传入了 cal_a，因此第二个参数要求类型为 Sensor<cal_a>，
    -- 但实际传入的 sensor_b 类型为 Sensor<cal_b>，依赖身份失配。
    read_scaled(cal_a, sensor_b)
}
```

### 负例 2：返回依赖函数内部局部栈变量的类型

运行时依赖类型所引用的对象必须在类型使用处仍然有效且可访问。如果一个函数试图返回绑定在函数局部栈变量 `local_cal.ref` 上的 `Sensor<local_cal.ref>`，在编译期会被立即拦截：

```flurry
fn leaked_local_dependency() -> Sensor<local_cal.ref> {
    let local_cal = Calibration { scale: 2 }
    -- ❌ 编译期报错：返回类型引用了函数内部的局部栈变量 local_cal，
    -- 函数返回后 local_cal 即被销毁，调用方无法在外部合法取得该依赖环境。
    Sensor<local_cal.ref> { raw: 10 }
}
```

如果需要跨函数返回带校准的数据，应当由调用方传入长寿命的 `cal: *Calibration`（如 `fn create_sensor(cal: *Calibration, raw: i32) -> Sensor<cal>`），使返回的依赖类型锚定在调用方作用域内依然有效的对象身份上。

---

## 四、 本篇小结

| 工程维度 | 核心契约 | 带来的保障 |
| :--- | :--- | :--- |
| **零膨胀值布局** | `Sensor<cal>` 的值本体只保存数据字段 `raw: i32` | 在数组与紧凑结构体中保持与裸整数一致的缓存密度 |
| **严格身份判等** | 比较完整类型时同时核验类型构造头与所绑定的运行时身份 `cal` | 编译期避免“拿通道 A 的校准参数算通道 B 的数据” |
| **作用域逃逸检查** | 依赖项必须在结果跨越的作用域边界外仍然有效且可取得 | 防止返回依赖已销毁局部栈变量的悬空依赖类型 |

---

## 五、 下一篇预告

在开发包含数百个模块的大型工程时，我们希望修改一个函数的内部实现时，不会迫使整个工程重新检查所有上层模块；同时，当我们在代码中使用 `build.mode` 等编译期配置或定义相互递归的函数与常量时，需要清楚哪些依赖结构是合法的无环图，哪些会构成非法的编译期求值死循环。

下一篇 **[Flurry 演进手记 (187)：大型工程的接口解耦与编译期依赖边界——函数签名隔离、构建配置感知与求值环路防范](./187-fine-grained-incremental-queries-and-cycle-recovery.md)**，我们将探讨模块化工程的最佳实践。
