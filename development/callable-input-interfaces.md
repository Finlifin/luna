# 函数值的公开参数接口

调用者按目标接口供应实参。实现的槽位、域和责任符合目标时，作者确认允许按槽位适配。下面的两个实现可以使用同一个接收接口。

```flurry
typealias Factory = pure comptime fn<...Items: meta.List<Type>> -> Type;

pure comptime fn First<...Left: meta.List<Type>> -> Type { bool }
pure comptime fn Second<...Right: meta.List<Type>> -> Type { bool }

comptime fn select(flag: bool) -> Factory {
    if flag { First } else { Second }
}

comptime fn forward(factory: Factory) -> Factory { factory }

comptime fn Apply(factory: Factory, items: meta.List<Type>) -> Type {
    factory<.Items = items>
}
```

在 `Apply` 里，`.Items = items` 直接交付已有的完整列表。即使运行到这里时已经选中 `First`，写 `.Left = items` 仍会报未知名字。最后得到的类型以实际选中的 `First` 或 `Second` 为头，保留该次应用的实参。[完整程序](repros/dynamic-nf-interface-forwarding.fl) · [参数与供应规则](../docs/functions/parameters.md)

## 需要分别保存的信息

| 信息 | 约束与归属 |
| --- | --- |
| 声明 telescope | 各输入的 sort、域、绑定位置和结果依赖。签名内的一致重命名保持绑定关系。 |
| 公开供应接口 | 哪些槽可具名供应、如何收集、缺省时使用哪个提供者。名字按目标接口解释。 |
| 实现身份 | 所选代码、声明实参、接收者与捕获环境。形成要求和进入主体的责任在该声明处检查。 |
| 本次输入计划 | callee、源码位置、接收槽、待决域与已经取得的值，生命周期限于本次调用。 |

公开入口不同的两个接口可以适配。比较接口 Type 本身是否相等仍待作者决定。独立的 Type 表示、PTC 与缓存键必须能一致解释这一区别；以裸 telescope 为键的全局名字表无法保存同形接口各自的入口。[依赖函数类型](../docs/functions/higher-order-functions.md) · [两种名字的观察程序](repros/callable-interface-name-observation.fl)

## 当前源码入口

- 直接调用源码 NF 声明时，声明提供公开名、默认表达式及其定义作用域。无标注的模块常量直接转交该声明时也可继承这些供应方式。
- 显式标注的模块常量采用自己的接口。透明类型别名和常量重新导出保留这一选择。若目标槽必填，即使实际实现有可选默认值，调用者也须供应该槽。
- 源码函数形参或直接调用结果的类型标注明确给出 NF 接口时，elaboration 从形参的声明或函数返回声明取得公开收集名。选择、转发和再次传参均按实际接收位置检查。builtin 集中绑定与用户声明采用同一机制。[公开接口来源](../compiler/elaboration/src/program/normal/supply.rs) · [动态应用](../compiler/elaboration/src/program/normal/dynamic_inputs.rs)
- NIR 的头描述记录收集槽位与声明公开名，dump 中例如 `(collector (slot 0 (name "Left")))`。单独的 `NormalFormFunction` Type 和 v1 metadata 目前只传 telescope 与槽位。如果在调用点只有这个 Type，当前可按位置供应；一般计算所得的 Type、局部存储和聚合投影的具名接口仍待传送。[NIR 头描述](../compiler/nir/src/normal_forms.rs) · [wire 编码](../compiler/luna/src/build_context/metadata/wire.rs)

### 已接通：共同的声明 telescope 构造

`DeclarationTelescope` 按顺序抽象源码和已发布声明的域。一个槽的域相对已完成前缀检查，成功后才加入自己的绑定；嵌套函数引入新帧时，外层引用只移位一次。打开签名时，私有输入身份须与捕获的外层声明互斥。形成和打开使用调用者的额度。2,000 槽的用例覆盖宽签名，嵌套往返用例覆盖绑定捕获和重试。[构造器](../compiler/nir/src/callable_types/telescope.rs) · [作用域往返](../compiler/nir/tests/normal_form_callables.rs)

普通声明、VM 接收世界、类型解析和阶段检查共用这个过程。比较方法的跨阶段贡献时，两边各自按声明身份建一次槽位索引，再比较各域。512 槽的用例覆盖这条路径。实际不匹配、额度耗尽和未完成的前提各自传播。[完整参数组](../compiler/nir/src/function_parameters.rs) · [接收世界检查](../compiler/type_resolution/tests/callable_world.rs)

## 路由和执行

callee 先取得，显式输入在源码位置各求值一次。固定包的实际宽度决定下一个展开进入哪个槽。空包也占执行位置；位于具名实参之前的可变值读取须保留当时的快照。缺省值随后在提供者的词法作用域读取所需的前置槽位。提前返回或退出循环会跳过后续输入。依赖域允许调整检查次序，执行仍遵循原调用的控制流。[调用规则](../docs/functions/calls-and-returns.md) · [源码路由](../compiler/elaboration/src/program/declaration_inputs/routing.rs)

动态 NF 应用通过 `apply-normal-form` 让输入帧打开目标 telescope。源码检查保留输入的域及已观察身份；原 invocation 执行生产者，检查实际头的形成条件，然后构造应用。比如 `factory<...prefix(), 1, 255>` 中固定包可以到执行时才交付 `u8`，后两个文字依次按该域检查。当前 VM 支持输出 `Type` 的 NF，普通值结果仍缺少结果域交付。[两种执行器的用例](../compiler/luna/tests/normal_input_plans.rs) · [源码区域与调用期观察](elaboration-state-and-publication.md)

## 独立 Type 与跨包接口的完成条件

1. 受检接口能随 Type 值、别名、聚合字段、函数返回值和生成定义传送。保存公开名、槽位、域与阶段，以及默认、隐式、收集协议所需的提供者。默认提供者须有定义作用域、所需前缀和稳定的代码或模板引用。运行期默认表达式的纯度、有限完成及清理要求仍适用。[接口默认值](../docs/functions/comptime_fn.md)
2. 每次适配检查槽数、sort、作用域、域、阶段、unsafe 责任和契约。NIR 值准入与 VM 对活动 callable 的准入都应基于同一接收世界。类型解析、替换、PTC、实现索引与关联项重建须保留完整接口。取得下一槽的域需要执行时，由原调用持有等待与恢复状态。
3. 格式版本应区分带接口的记录与旧记录；缺失的公开入口只能由有权的声明提供。读取独立产物后，也须能用目标名完成调用并拒绝实现的私有名。世界快照和接口变化应使依赖它们的代码和查询失效。活动槽值、waiter 与候选事务留在 invocation 内。
4. optional、implicit、拓展收集、trait、closure、quote 与 builder 沿用相应声明的阶段和责任。目标接口若更换默认提供者，调用时应执行目标提供者，并在完整输入准入后进入所选实现。

## 验证

```sh
python3 -B development/check.py --timeout 900 -- run -p luna -- comptime development/repros/dynamic-nf-interface-forwarding.fl --entry verify --sysroot library --build-target x86_64 linux 64 --fuel 10000000
python3 -B development/check.py --timeout 900 -- test -p luna --test declaration_input_plans --test normal_input_plans
```

第一条命令在真实 sysroot 下执行两个选择分支，预期 `(bool true)`。定向用例还检查公开名与实现名不同、重新导出、必填位置、完整列表和 builtin 绑定。[源码用例](../compiler/luna/tests/declaration_input_plans.rs)
