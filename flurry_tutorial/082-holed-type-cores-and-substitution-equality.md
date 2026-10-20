# Flurry 演进手记 (082)：运行时依赖类型的等价与替换——动态环境、绑定身份与跨作用域传递

在上一篇（081）中，我们通过 `NodeIndex<storage.ref>` 初识了运行时依赖类型：一个在内存里只占 4 字节（仅含 `index: u32`）的结构体，却能在类型层面记住自己属于哪个运行期存储池 `storage`，从而静态拦截跨池下标混用。

在此基础上，本篇我们将深入探讨运行时依赖类型在真实工程中的三个核心问题：
1. **类型等价边界**：两个同样由 `Sensor<cal>` 构造出来的类型，满足什么条件才算同一个类型？
2. **复合聚合体的环境共享**：当一个外层结构体 `SensorPair<cal>` 内部包含两个 `Sensor<cal>` 字段时，子字段如何共享外层环境而不重复存储指针？
3. **跨函数传递与环境可达性**：当把依赖运行期环境的实例传给下游函数时，参数签名该如何表达环境关联？

---

## 一、 运行时依赖类型的等价准则：结构头 + 依赖绑定身份

在 Flurry 中，像 `Sensor<cal>` 这样的运行时依赖类型，其完整类型身份由两部分共同决定：
1. **类型构造器结构（Type Structure）**：即 `Sensor` 定义本身；
2. **依赖参数的绑定身份（Bound Dependency Identity）**：即传入 `<...>` 的运行期对象身份 `cal`。

```
+-------------------------------------------------------------------------+
|                 运行时依赖类型的完整相等性判定准则                        |
+-------------------------------------------------------------------------+
|                                                                         |
|   Sensor<cal_a>  ==  Sensor<cal_b>                                      |
|         ▲                                                               |
|         └── 充要条件：                                                   |
|             1. 具有相同的类型构造器头 `Sensor`                            |
|             2. 依赖槽位绑定的是同一个运行期身份 (`cal_a` 与 `cal_b` 同一)  |
|                                                                         |
|   ⚠️ 注意：即便 `cal_a.*` 与 `cal_b.*` 的字段数值完全相同，               |
|            只要它们来自两个独立的绑定实例，类型就不相等                   |
+-------------------------------------------------------------------------+
```

---

## 二、 实战演练：传感器校准环境 `Sensor<calibration>`

让我们构建一个传感器校准模型：每个传感器通道绑定一个校准配置 `Calibration`（包含比例因子 `scale` 与零点偏移 `offset`）。`Sensor` 实例本身在内存中只保存原始读数 `raw: i32`（4 字节），校准配置通过类型依赖环境提供：

```flurry
struct Calibration {
    scale: i32,
    offset: i32,
}

struct Sensor
where
    quote calibration: lift *Calibration,
{
    raw: i32,

    -- 访问指针环境 calibration：集中在 unsafe 方法中并在 calibration 存活的作用域内调用
    unsafe fn reading(*self) -> i32 {
        self.raw.* * calibration.scale.* + calibration.offset.*
    }
}

test sensor_dependency_identity {
    let first_cal = Calibration { scale: 2, offset: 1 }
    let second_cal = Calibration { scale: 2, offset: 1 }

    let first_sensor = Sensor<first_cal.ref> { raw: 10 }
    let second_sensor = Sensor<second_cal.ref> { raw: 10 }

    unsafe {
        assumes first_sensor.reading() == 21;
        assumes second_sensor.reading() == 21;
    }

    -- 同一绑定身份构造出的类型相等
    assumes first_sensor'type == Sensor<first_cal.ref>;
    -- 不同绑定身份构造出的类型不相等（即便校准数值完全一致）
    assumes first_sensor'type != second_sensor'type;
}
```

如果尝试把绑定了 `second_cal.ref` 的传感器赋给要求 `Sensor<first_cal.ref>` 的位置，类型检查会直接拒绝：

```flurry
-- [编译期拒绝示例] 不同校准环境身份的 Sensor 类型不可混用
-- fn mix_calibrated_sensors(
--     first_cal: *Calibration,
--     second_cal: *Calibration,
-- ) -> void {
--     let s1 = Sensor<first_cal> { raw: 10 }
--     -- 编译错误：期望 Sensor<first_cal>，实际为 Sensor<second_cal>
--     let s2: Sensor<first_cal> = Sensor<second_cal> { raw: 10 }
-- }
```

---

## 三、 复合结构体中的环境共享与转发

在实际工程中，我们经常把多个同通道的采样值组合进一个聚合结构（例如差分传感器对 `SensorPair`）。如果 `first` 和 `second` 两个子传感器共享同一个校准环境 `calibration`，外层结构体可以直接将自己的依赖参数转发给内部字段：

```
+-------------------------------------------------------------------------+
|                SensorPair<calibration> 的环境转发与内存布局              |
+-------------------------------------------------------------------------+
|  外层类型：SensorPair<calibration>                                      |
|    ├── 字段 first  : Sensor<calibration>   (转发外层 calibration 环境)   |
|    └── 字段 second : Sensor<calibration>   (转发外层 calibration 环境)   |
|                                                                         |
|  物理内存布局：                                                         |
|    [ first.raw: i32 (4B) | second.raw: i32 (4B) ]  --> 总计仅 8 字节     |
+-------------------------------------------------------------------------+
```

来看代码实现：

```flurry
struct SensorPair
where
    quote calibration: lift *Calibration,
{
    -- 将外层 calibration 依赖直接转发给两个子字段的类型
    first: Sensor<calibration>,
    second: Sensor<calibration>,

    unsafe fn total(*self) -> i32 {
        self.first.reading() + self.second.reading()
    }
}

test composed_environment_sharing {
    let cal = Calibration { scale: 3, offset: 2 }
    let pair = SensorPair<cal.ref> {
        first: Sensor<cal.ref> { raw: 4 },
        second: Sensor<cal.ref> { raw: 6 },
    }

    -- 内存体积仅为两个 i32 字段之和（8 字节），不包含任何冗余指针
    assumes Type.size_of(SensorPair<cal.ref>) == 8;

    unsafe {
        -- (4 * 3 + 2) + (6 * 3 + 2) = 14 + 20 = 34
        assumes pair.total() == 34;
    }
}
```

当调用 `pair.total()` 时，外层 `SensorPair` 的 `calibration` 环境会自动转发给 `self.first.reading()` 和 `self.second.reading()`，既保证了两个子传感器一定使用同一份校准参数，又保持了紧凑的 8 字节内存布局。

---

## 四、 跨函数传递依赖类型：显式连接环境参数

当编写一个接收运行时依赖类型的通用函数时，只需将所依赖的环境对象作为参数传入，并在依赖类型参数中引用该形参即可：

```flurry
unsafe fn compute_calibrated_diff(
    cal: *Calibration,
    left: Sensor<cal>,
    right: Sensor<cal>,
) -> i32 {
    left.reading() - right.reading()
}

test cross_function_dependent_call {
    let cal = Calibration { scale: 5, offset: 0 }
    let s_high = Sensor<cal.ref> { raw: 12 }
    let s_low = Sensor<cal.ref> { raw: 7 }

    unsafe {
        let diff = compute_calibrated_diff(cal.ref, s_high, s_low);
        assumes diff == 25;
    }
}
```

注意 `compute_calibrated_diff` 的参数列表：`left: Sensor<cal>` 和 `right: Sensor<cal>` 都引用了第一个参数 `cal: *Calibration`。这在函数边界上立下了静态契约：调用方传入的 `left` 和 `right` 必须绑定在同一个 `cal` 指针身份上，任何跨通道混传都会在调用点被静态拦截。

---

## 五、 本篇小结

| 核心机制 | 语言行为与契约 | 工程价值 |
| :--- | :--- | :--- |
| **依赖类型相等性** | 同时核对类型构造器与依赖参数的绑定身份 | 阻止内容碰巧相同但归属不同上下文的句柄或数据包混用 |
| **复合环境转发** | 外层 `quote` 依赖参数可直接传给内层字段类型 `Sensor<calibration>` | 多个子组件共享同一环境，内存布局保持零冗余指针开销 |
| **跨函数依赖签名** | 形参类型直接引用同签名中的前置环境形参 `Sensor<cal>` | 在函数调用边界静态强制多个实参归属于同一运行期环境 |

---

## 六、 下一篇预告

既然类型本身可以携带编译期参数或依赖参数，那么当我们在编写泛型代码或编译期反射逻辑时，能不能反过来像匹配普通枚举值一样，直接用 `match` 或 `is` **对一个 `Type` 进行模式匹配**，从中解构出元素类型 `T` 或数组长度 `N`？

下一篇 **《Flurry 演进手记 (083)：在类型自身上做模式匹配——`expr<type_pattern_argument*>` 类型模式详解》**，我们将探索类型级模式匹配的用法。
