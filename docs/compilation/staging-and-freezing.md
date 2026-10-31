# 两阶段计算与冻结

## Intro

定义检查可以交付一个尚待具体输入的计算，但不能把本次检查器的运行栈交给下次编译继续使用。

冻结保存的是稳定的绑定、契约和合法残余工作。实例化取得输入后，重新建立自己的局部状态，完成生成实例所需的计算。

## 语法

```text
defer_expr -> defer expr
comptime_block -> comptime { (definition | statement)* }
comptime_function -> pure? comptime fn id(parameter*) (-> result)? clauses? (block | (= expr))
normal_form_head_application -> expr<argument*>
```

Ordinary/loose 与 defer 的用户规则见[normalization 与 instantiation](../comptime/normalization-and-instantiation.md)。本章不增加源程序的“冻结”操作。

## 正式定义

### 两阶段交织

Normalization 依据当前契约完成规定的定义检查。需要具体值且输入已经具备时，相关 comptime 计算可以就地发生，并为后续 elaboration 提供信息。

Instantiation 代入具体输入，继续合法延迟的计算与检查。具体类型结果可能决定后续实参检查、方法选择或代码接入；后续检查也可能产生新的 comptime 请求。因此不能用一次不可返回的“先 elaboration、后 comptime”划分替代这组依赖。

类型计算依赖某个 runtime 对象时，只能使用已经准入的身份或代码描述，不读取未来内容。IIR 中必须完成类型的 comptime 部分；合法 runtime 依赖按[两层表示](runtime-dependent-types.md)保留。

### 冻结的内容

| 内容 | 定义端稳定产物可否保留 |
|---|---|
| 显式绑定的参数及可用契约 | 可以，保留绑定及前提 |
| 具有合法延迟依据的残余计算 | 可以，保留输入与控制依赖 |
| 有语义身份的 runtime 依赖项 | 可以，保留其绑定关系 |
| 本次推断的未解决临时洞、活动 waiter、continuation | 不可以 |
| 已失败或未完成的必检义务 | 不能冒充成功结果 |

Frozen 不表示所有值都已具体。它要求产物可由自身的稳定输入和绑定解释，不再依靠上一轮活动推断内存。

### 两种执行载体

| 载体 | 职责 |
|---|---|
| comptime interpretation | 在混合场景中就地解释相关 IR |
| comptime invocation | 调用由独立 comptime 函数形成的 engine 字节码 |

两种载体遵守相同的阶段、求值、类型与副作用语义。可以采用不同资源预算，预算差异不定义另一种语言子集。

### 范式、控制依赖与执行次数

范式应用保留头与实参，输入全部具体也不展开头的函数体。已合法保留的范式项不是尚欠执行的普通函数调用。

控制依赖尚未确定时，不能提前执行所有分支的 comptime 副作用。缺省计算、显式调用和延迟求值遵守各自规则；进入 query 或恢复 goal 不会获得重复执行许可。

## 常见示例

### 同一开放定义形成两个具体实例

```flurry
use std.prelude.*;

pure comptime fn SelectType(wide: bool) -> Type {
    if wide {
        i64
    } else {
        i32
    }
}

fn keep_selected(
    comptime wide: bool,
    value: defer SelectType(wide),
) -> defer SelectType(wide) {
    value
}

test {
    let narrow = keep_selected(false, 3i32);
    let wide = keep_selected(true, 7i64);
    println("{} {}", narrow, wide);
}
```

冻结定义时保存 wide 的绑定及依赖它的类型计算。两个调用分别恢复自己的实例状态，得到 i32 和 i64。不能把第一次调用留下的可变类型槽复用给第二次调用。

```text
冻结的定义：parameter wide；residual SelectType(wide)；输入/结果同型
实例一：wide ↦ false；类型计算 ↦ i32
实例二：wide ↦ true；类型计算 ↦ i64
```

以上是产物关系示意，不是 Flurry 语法。

### 普通计算与范式应用

```flurry
pure comptime fn Identity<T: Type> -> Type {
    T
}

typealias Element = Identity<u32>;

fn keep_element(value: Element) -> Element {
    value
}
```

Element 仍指向 Identity<u32>。keep_element 的参数与结果一致；编译器不从 Identity 的函数体推出 Element 等于 u32。

下面的独立函数应拒绝，因为没有给出从 u32 到 Element 的交付依据：

```flurry
fn invalid_element(value: u32) -> Element {
    value
}
```

### 实例中的编译时条件只选择所需工作

```flurry
fn select(comptime first: bool, left: i32, right: i32) -> i32 {
    inline if first {
        left
    } else {
        right
    }
}
```

first 确定本次实例接入哪个主体；left、right 仍是普通参数，调用处显式实参的求值遵守普通规则。选择主体不能反向删除实参准备期间应发生的行为。

## 与 Rust / Zig / C / C++ 的关键差异

Zig 的 comptime 参数也使具体输入参与实例化。Flurry 另有 ordinary 的定义端要求，以及不会展开的范式头；这些边界不能仅靠“编译时能算”统一解释。[Zig comptime 参数](https://ziglang.org/documentation/master/#Compile-Time-Parameters)

C++ 模板推导、约束与静态断言，C 的宏和静态断言，Rust 的 trait bound 与常量求值提供不同的对应手段；两阶段用户语义的对照见[comptime 章节](../comptime/normalization-and-instantiation.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

Runtime 实例不携带等待某个 comptime 类型结果的解释器调用。编译时参数从普通 runtime 传参中移除；需要的 runtime 对象、环境和程序检查则保留。

普通 Splice 方法中的 runtime 工作不是残余 comptime。完成其类型和实例选择后，仍按相应转换程序执行，不能因为它参与跨阶段交付而将其整体删除。

## 编译器实现引导

### 冻结算法

```text
freeze_definition(region):
    settle all required definition-time obligations
    classify remaining work using language delay rules
    reject non-exportable unresolved inference or failed prerequisites
    rewrite local references into explicit stable binders
    serialize contracts with premises and residual control/data dependencies
    ensure no live solver cells, waiters or continuations escape
    finish or cancel local tasks
    return immutable checked result
```

残余节点记录原计算、输入绑定、检查要求、合法延迟依据、控制条件与诊断来源。只保存函数名和参数文本不足以重建一次捕获了局部绑定的计算。

### 实例化算法

1. 在新区域中导入冻结结果，建立本次替换与 runtime 绑定关系。
2. 从所需产物反向请求依赖，激活输入与控制条件具备的计算。
3. 按 interpretation 或 invocation 的适用接口求值，发布结果及合法证据。
4. 恢复依赖这些结果的 elaboration，完成类型、选择与阶段检查。
5. 检验 PTC assumes 等实例义务，失败时禁止依赖它的实例交付。
6. 验证类型的 comptime 部分均已完成，形成 IIR。

如果求值请求的定义也需要 elaboration，建立实际依赖边，不能递归进入时直接标记成功。正常的签名递归与需要自身具体结果的计算环分别处理。

### 缓存与成本

缓存冻结的计算描述不等于缓存一次不纯执行。可复用结果需要合法身份、完整依赖和对应执行契约；需要发生的调用不能因上次返回了相同值而跳过。

定义产物按实际访问的绑定和依赖共享；实例状态独立。验收比较同一冻结结果多次实例化、不同输入顺序、失败后重试和取消后的再次请求，确保没有共享的临时赋值或重复执行。
