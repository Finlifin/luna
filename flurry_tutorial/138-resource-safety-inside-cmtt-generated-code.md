# Flurry 演进手记 (138)：当 CMTT 遇到资源管理——保证生成代码中的仿射值与指针绝不泄漏

在上一回（137）中，我们用 `template fn` 和 `template test` 为生成的自动机搭建了自动化测试靶场，并初步见识了模板内部 `RequestTrace` 对象的 `Drop` 清理。

现在，我们要直面所有系统级编程语言在设计“元编程 / 代码生成”时最头疼的终极难题：**当编译期的代码生成（第六卷 CMTT）遇上运行期的所有权、仿射资源（`Once`）与指针可达性（第三卷）时，安全防线会不会被击穿？**

请你思考三个极具迷惑性的场景：
1. **代码值本身是可复制的编译期值**：我可以写 `const code2 = code1;`，把一段 CMTT 代码值复制十份。但如果这段代码内部**消费（Move）**了一个外部运行期的唯一资源 `buf: DmaBuffer`，复制代码值并在运行期接入两次，岂不是会导致同一个 `DmaBuffer` 被释放两次（Double-Free）？
2. **模式代码导出了未使用的资源绑定**：如果一段模式代码 `< Frame -| header: DmaBuffer, tag: u8 >` 匹配成功后解构出了两个绑定，但配对的表达式主体 `< tag: u8 |- u8 >` 只用了 `tag` 而没管 `header`，那个没人管的 `header` 会发生内存泄漏吗？
3. **`quote fn` 提前返回或未使用的实参**：如果调用 `quote fn` 时传入了一个拥有所有权的资源实参，但 `quote fn` 根本没用这个形参，或者中途 `return` 跳出了，这个实参的资源责任归谁？

今天，我们就来拆解 Flurry 编译器中 **CMTT 上下文替换与资源管理系统的联动核心规则（`docs/cmtt/contexts-and-substitution.md` & `docs/cmtt/index.md`）**。

---

## 一、 问题背景与设计思路：为什么“编译期代码共享”绝不等于“运行期资源复制”？

在深入规则之前，我们必须用一张图把 **`U_1` 编译期的代码值复制** 与 **`U_0` 运行期的资源所有权移动** 彻底分清：

```
+-------------------------------------------------------------------------+
|          U_1 编译期代码值复制  vs  U_0 运行期仿射资源消费                  |
+-------------------------------------------------------------------------+
|                                                                         |
|  [ U_1 编译期 (Comptime) ]                                               |
|  const consume_code: < buf: DmaBuffer |- u32 > = '( release(buf) );     |
|  const alias_code = consume_code;                                       |
|  • 完全合法  `consume_code` 只是一份蓝图（代码描述值），具有编译期 Copy。 |
|  • 注意：在创建和复制 `consume_code` 时，运行期的 `buf` 根本还没被读取    |
|                                                                         |
|  ==================== [ ~<...> Splice 接入边界 ] ======================  |
|                                                                         |
|  [ U_0 运行期 (Runtime) ]                                                |
|  情况 A：分别用两份不同的运行期资源 b1, b2 供应两次接入：                  |
|          ~[buf = b1]<consume_code> + ~[buf = b2]<alias_code>            |
|          ──► ✅ 完全合法  b1 和 b2 各自只被 Move 了一次                   |
|                                                                         |
|  情况 B：把同一个运行期唯一资源 b1 供应给两次接入：                        |
|          ~[buf = b1]<consume_code> + ~[buf = b1]<alias_code>            |
|          ──► ❌ 编译期移动检查（Move Checker）直接报错拦截（Use-After-Move）  |
+-------------------------------------------------------------------------+
```

> **💡 核心心智模型（`docs/cmtt/index.md` & `contexts-and-substitution.md`）：**
> **“复制代码值不复制其运行时资源。代码共享不等于资源共享或复制。两次接入如果使用同一份运行时 owner，必须检查生成后的真实使用顺序。”**
> 换句话说：在编译期，你可以像复印施工图纸一样把 `consume_code` 复印一百份；但到了运行期施工现场（Splice 接入后），每一份接入都会还原成对真实运行期绑定对象的实际访问与移动操作，**第三卷的移动检查器（Move Checker）、借用/别名检查器与可达性检查器，会对接入后的真实控制流执行 100% 严苛的核验**。

---

## 二、 实战：用同一份 CMTT 代码模板安全消费两份不同的仿射资源

下面我们定义一个代表独占硬件描述符的仿射资源结构体 `OwnedDescriptor`（实现了 `Drop`，因此它只有 `Once` 唯一所有权责任，不具备 `Copy` 能力）。

我们来看看如何创建一份消费该资源的 CMTT 开放代码 `consume_desc`，并通过显式上下文替换 `~[desc = ...]` 安全地处理不同的运行期实例：

```flurry
-- 带有 Drop 责任的仿射资源（仅具备 Once 移动语义，不可隐式 Copy。）
struct OwnedDescriptor {
    slot_id: u32,
}

impl Drop for OwnedDescriptor {
    fn drop(*self) {
        let _ = self.slot_id.*;
    }
}

-- 按值接收并消费 OwnedDescriptor 的运行期函数
fn finalize_descriptor(d: OwnedDescriptor) -> u32 {
    d.slot_id + 100u32
}

test {
    let first_res = OwnedDescriptor { slot_id: 1u32 }
    let second_res = OwnedDescriptor { slot_id: 2u32 }

    -- 1. 在编译期定义一份消费虚拟端口 `desc` 的开放代码，并为其创建别名
    const consume_code: < desc: OwnedDescriptor |- u32 > =
        '(finalize_descriptor(desc));
    const alias_code = consume_code;

    -- 2. 两次接入分别绑定到两个独立的运行期 owner（first_res 与 second_res）：
    --    first_res 在第 1 次接入中被合法 Move，second_res 在第 2 次接入中被合法 Move。
    let r1 = ~[desc = first_res]<consume_code>;
    let r2 = ~[desc = second_res]<alias_code>;

    assumes r1 == 101u32;
    assumes r2 == 102u32;
}
```

如果有人妄图利用 `consume_code` 和 `alias_code` 是两个不同的编译期常量名，把**同一个运行期仿射资源 `first_res`** 供应给它们消费两次：

```flurry
-- ❌ 编译期直接拒绝：尽管使用了两份代码值，但接入后它们操作的是同一个运行期绑定 first_res，
-- 第二次接入触发 Use-After-Move（移动后重复消费仿射资源），被资源检查器静态拦截。
fn reject_double_move_via_splice() -> u32 {
    let first_res = OwnedDescriptor { slot_id: 1u32 }
    const consume_code: < desc: OwnedDescriptor |- u32 > =
        '(finalize_descriptor(desc));
    const alias_code = consume_code;

    let a = ~[desc = first_res]<consume_code>;
    let b = ~[desc = first_res]<alias_code>;
    a + b
}
```

---

## 三、 进阶：`quote fn` 与模式接入中的四大资源与指针核心规则

除了防止显式 Splice 的双重消费，当我们使用 `quote fn`、指针接收者 `*self` 以及模式代码 `< T -| Ψ >` 时，Flurry 编译器还严格守护着以下四条生死防线（`docs/cmtt/index.md` & `docs/cmtt/patterns-and-arms.md`）：

### 防线 1：`quote fn` 未使用的资源实参也必须求值并妥善清理
在 `quote fn` 应用时，普通运行期实参严格按源码顺序各求值一次，并绑定到新鲜的局部形参身份上：
- 如果某个实参是一个拥有 `Drop` 责任的 `OwnedDescriptor`，哪怕 `quote fn` 的函数体里**完全没有用到这个形参**，或者在中途通过 `return` 提前结束了本次 `quote fn` 应用，**该未返回资源依然会在本次 `quote fn` 的逻辑完成边界内自动执行 `Drop` 清理**，绝不泄漏。
- 反之，如果该资源被作为返回值交付出去，它的所有权责任就会干净利落地移交给外层调用者，绝不会在 `quote fn` 退出时被误析构。

### 防线 2：模式代码 `< T -| Ψ >` 多导出的未用资源绑定同样履行清理
在 `docs/cmtt/patterns-and-arms.md` 中规定：“模式可以导出更多绑定，但未被主体使用的资源仍须履行清理责任。”
并且，尝试匹配一个模式时，**绝不能在尚未确认匹配成功前就提前消费掉后续失败分支仍需使用的输入值**。

### 防线 3：`*self` 引用接收者与指针目标身份不可伪造
在固有 `quote fn` 方法（如 `impl Flag { quote fn flip(*self) { ... } }`）或上下文替换中：
- `*self` 保留指向原运行期对象位置的指针/引用，展开 `quote fn` **绝不会悄悄复制被引用的底层对象**；
- 在做上下文替换 `~[port = replacement]` 时，普通值形参可以建立新鲜绑定，但**指针指向的目标对象身份与运行时依赖类型环境（081–082 回）绝不会因为改了个接口名字就变成新对象**。两个表达式接口即便映射到同一个可变对象，接入后也必须接受严格的别名与读写冲突检查。

### 防线 4：严禁将运行期活跃指针冻结进编译期数据
正如 `docs/cmtt/index.md` 所强调的：**“数据冻结不接受指针、可调用身份或活动存储，即使它们包在 nominal 对象里。”** 你不能在编译期造一个悬垂的运行期栈地址塞进 Quote 里逃逸出去。

下面通过示例代码验证 `quote fn` 的资源移交、未用实参自动清理与 `*self` 原地修改：

```flurry
struct BufferState {
    count: u32,
}

impl BufferState {
    -- *self 保留指向原位置的可变引用，展开不会复制 BufferState
    quote fn bump(*self) {
        self.count.* += 1u32;
    }
}

-- 接收两个仿射资源：kept 被按值返回移交给调用者，dropped_early 在内部提前退出时自动清理。
quote fn select_resource(
    flag: bool,
    kept: OwnedDescriptor,
    dropped_early: OwnedDescriptor,
) -> OwnedDescriptor {
    if flag {
        -- 提前完成本次 quote 应用：kept 移交给调用者，dropped_early 在此逻辑边界自动触发 Drop。
        return kept;
    }
    kept
}

test {
    var state = BufferState { count: 10u32 }
    state.bump();
    assumes state.count == 11u32;

    let d1 = OwnedDescriptor { slot_id: 7u32 }
    let d2 = OwnedDescriptor { slot_id: 8u32 }

    let winner = select_resource(true, d1, d2);
    assumes winner.slot_id == 7u32;
}
```

---

## 四、 本篇小结

| 场景 / 操作 | 编译期 ($U_1$) 行为 | 运行期 ($U_0$) 接入后检查契约 |
| :--- | :--- | :--- |
| **复制代码值 (`const c2 = c1`)** | 合法复制编译期代码蓝图 | 不复制运行期资源；若两次接入绑定同一仿射资源，报 **Use-After-Move** |
| **`quote fn` 传入未用/未返回资源** | 建立新鲜形参绑定与逻辑完成边界 | 实参严格求值一次；未返回的资源在 `quote fn` 逻辑退出点自动执行 `Drop` |
| **`quote fn` 按值返回资源** | 生成结果汇合槽位 | 所有权责任完整移交给调用者，不产生多余复制或双重析构 |
| **`*self` 引用接收者 / 上下文替换** | 保持原绑定或替换映射关系 | 保留原对象存储身份，不复制底层对象，严格执行别名与可达性检查 |
| **尝试将活跃指针作为编译期数据冻结** | ❌ **直接拒绝** | 防止宿主编译期内存地址非法混入运行期目标程序 |

---

## 五、 下一篇预告

学到这里，你一定会为 Flurry 编译器的严谨感到安心：不管是类型不匹配、缺少上下文绑定，还是接入后发生了仿射资源的双重移动（Use-After-Move），编译器都能在编译期把 Bug 抓出来。

但紧接着，所有在实际工程里深度使用过元编程的开发者都会问出一个最揪心的体验问题：
**“当一段经过了三层 `pure comptime fn` 组装、又通过 `quote fn` 展开、甚至外层还套了 `@define_macro1` 的代码报错时，编译器的报错信息（Diagnostics）到底长什么样？”**
- 它会不会像 C++ 模板实例化报错那样，甩给你 300 行让人绝望的内部展开栈？
- 或者像拙劣的文本宏那样，只指着最后调用处的一行 `~<generated>` 说“这里类型不对”，却根本不告诉你那个错误的类型是从哪一层代码生成器、哪一个源码位置（Span）引进来的？

下一回 **《Flurry 演进手记 (139)：诊断生成代码中的错误——源码溯源（Span Tracking）与两阶段报错定位》**，我们将揭秘 Flurry 是如何通过精细的源码来源链（Origin & Span Tracking）与 Normalization / Instantiation 两阶段责任划分，让元编程报错像普通代码一样清晰易读的。
