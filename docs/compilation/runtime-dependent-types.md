# 运行时依赖类型的表示

## Intro

两个传感器可以使用相同的数据结构，却依赖不同的校准对象。编译器应共享它们重复的类型结构，同时保留每个引用到底指向哪个依赖。

为此，类型表示分成两层：底层是可以 intern 的不可变带洞结构，引用端以 substitution 解释这些洞。共享结构不抹去对象身份。

## 语法

```text
quote_decl_clause -> quote decl_clause
lift_type -> lift expr
normal_form_head_application -> expr<argument*>
```

洞、类型模板及 substitution 的内部表示不增加源语法。用户仍用已有的 quote 依赖参数和类型应用表达关系，见[运行时依赖类型](../type/runtime-dependent-type.md)。

## 正式定义

### 类型依赖与对象内容

类型可以依赖 comptime 可计算输入，或具有 identity 的 runtime 值。后者参与类型及操作环境，不因此成为编译时已知内容。

两个对象具有相同类型、相同当前内容或相似取得路径，都不足以证明它们是同一个依赖。依赖一经固定，不能为了匹配另一个接口而任意改绑。

### 本体与环境

值本体保存其自身数据；操作环境提供解释或使用这些数据所需的对象。多个值可以从共同外层取得同一个环境，不要求每个值逐个保存依赖指针。

知道依赖身份，与在某个使用点能够取得它，是两项要求。跨调用、返回或捕获边界时，必须保留合法交付方式。依赖参数不自动延长被依赖对象的有效期。

### 类型完成条件

一个具体实例的类型 comptime 部分必须已经完成。Runtime 依赖仍可以是实例参数、局部绑定或已交付环境中的符号项；它们无须变为常量。

合法保留的范式头也属于类型结果。不能为消除表示中的洞而展开 Identity<T>，或把 runtime identity 读取成当前对象内容。

类型引用中每个槽位都有合法替换，与把所有共享模板改写成完全无洞的对象，是不同的要求。前者足以保留完整的已实例化类型关系。

## 常见示例

### 同一类型结构，不同校准依赖

```flurry
use std.prelude.*;

struct Calibration {
    offset: i32,
}

struct Sensor
where quote calibration: lift *Calibration
{
    value: i32,

    fn measure(*self) -> i32 {
        self.value.* + calibration.offset.*
    }
}

fn show_sensor(sensor: *Sensor<calibration>)
where quote calibration: lift *Calibration
{
    println("{}", sensor.measure());
}

test {
    let first = Calibration { offset: 3 }
    let second = Calibration { offset: 8 }
    let first_sensor = Sensor<first.ref> { value: 10 }
    let second_sensor = Sensor<second.ref> { value: 10 }

    show_sensor(first_sensor.ref);
    show_sensor(second_sensor.ref);
}
```

输出 13、18。类型匹配恢复 calibration 对应谁；读取 offset 发生于方法的 runtime 执行。

一种内部表示如下。H0 是模板洞，R_first 与 R_second 是依赖项的身份，不是源代码中的名称字符串。

```text
共享 core：Apply(SensorDeclaration, [H0])
洞签名： H0 : runtime dependency, lift *Calibration

first_sensor 的类型引用： (core, { H0 ↦ R_first })
second_sensor 的类型引用：(core, { H0 ↦ R_second })
```

两个引用具有相同的 core，仍须比较各自 substitution。不能仅凭 core 的 intern ID 相同就认定整个类型相同。

### 跨函数保持已固定依赖

接续上例的声明：

```flurry
fn forward_sensor(sensor: Sensor<calibration>) -> Sensor<calibration>
where quote calibration: lift *Calibration
{
    sensor
}
```

输入与输出使用同一 calibration 绑定。移动 sensor 的值本体不会把依赖换成另一个对象；调用边界须继续提供操作所需的环境。

### 运行时依赖不强制增加逐值字段

```flurry
struct SensorPair
where quote calibration: lift *Calibration
{
    first: Sensor<calibration>,
    second: Sensor<calibration>,
}
```

两个字段可从共同外层取得校准环境。该声明不要求两个 Sensor 各自增加一个指针，也不保证任意环境优化都合法；环境取得与资源责任必须仍能兑现。

## 与 Rust / Zig / C / C++ 的关键差异

C 的句柄加上下文参数、C++ 的有状态对象、Zig 的显式上下文和 Rust 的类型及资源协议，都能表达相关的实现手段。Flurry 在类型依赖中直接保存 runtime identity，不能把它简化成这些语言的静态类型参数。对应关系见[运行时依赖类型的对照](../type/runtime-dependent-type.md#与-rust--zig--c--c-的关键差异)。

本章的两层结构是编译器表示。它本身不为源程序增加一种新的泛型、对象表示或运行时反射机制。

## 运行时实现设计引导

对操作实际使用的依赖，生成从形参、外层聚合、合法全局位置或封闭携带者取得环境的计划。相同身份可以有不同合法取得路径；路径不作为类型身份本身。

共享代码模板时，分别传递各次调用的环境。不能因为两次调用来自同一源码位置，就复用上次执行捕获的具体对象。

本体布局只依赖实际影响表示的输入。依赖若仅用于方法操作，不应无条件加入 sizeof 图。值相关大小仍须有合法的 runtime 表示计划，不能与“编译器尚未完成类型计算”混同。

## 编译器实现引导

### 三种占位分开表示

| 对象 | 含义 | 能否由推导器随意赋值 |
|---|---|---|
| 模板洞 | 不可变类型项的显式接口槽位 | 通过引用端替换解释，不改共享节点 |
| Runtime binder 项 | 已建立的对象身份与绑定关系 | 不能重新选择对象 |
| 推断元变量 | 本次调用或候选的新鲜未知项 | 在其约束允许的范围内求解 |

定义端的刚性参数同样不能被当作本次调用的自由未知量。临时推断状态保持在局部 solver 中，不写入共享类型模板。

### 表示与构造

以下名称为内部数据结构示意，不要求具体枚举或内存布局：

```text
TypeCore = immutable_type_term + ordered_hole_signature
TypeRef = (interned_core, substitution)
EnvironmentPlan[dependency_identity] = legal_access_path

make_type_reference(expression, context):
    elaborate the required type computation
    evaluate required comptime inputs; preserve valid normal-form heads
    collect dependency terms without reading runtime contents
    abstract parameter positions into scoped holes
    validate each substitution against its hole's contract
    intern the immutable core
    return TypeRef(core, substitution)
```

统一类型项保存声明身份、类型构造、静态输入及合法范式。洞签名说明每个位置的语义种类与前置依赖；后面的槽位在前面替换建立的环境下检查。类型参数和值参数也可使用对应槽位，不能只靠相同整数编号混合不同语义域。

同一依赖的重复出现必须保留相同绑定关系。规范化洞编号时可一致重命名局部 binder，但不能把两个不同外部对象变成同一个槽位来提高共享率。

### 替换组合

```text
compose(σ, θ)[hole] = substitute_capture_avoiding(σ[hole], θ)
```

θ 将 σ 值中的局部引用映射到新上下文；进入 binder 时调整索引或一致改名，避免捕获。可以保存惰性组合，观察时沿组合取得结果，并在适当位置压缩；无论采用何种存储，语义都与逐项的无捕获替换一致。

引用新的环境只产生新 TypeRef 或 substitution，不原地修改已 intern 的 core。试探失败后丢弃局部替换，不影响其他引用。

### 相等、散列与缓存

相同 core 是比较的起点。继续解释有关槽位，分别按类型等式、comptime 值关系和 runtime identity 比较。不同 core 也不能仅凭 ID 不同就永久断言语义不等；普通计算结果与符号关系仍按类型系统规定处理。

散列只用于选择比较候选，不能作为逻辑证明。模板缓存可按不可变结构复用；语义查询还要纳入相关替换、前提和环境。使用点的可取得性与有效性检查不能由模板命中代替。

持久缓存若抽象掉外部依赖，必须把它们变成显式缓存参数，并在命中时重新绑定。禁止持久化进程内地址、活动 solver 节点或上次调用环境的裸引用。

### IIR 入口验证

对实例及其有效绑定环境 Γ，逐项检查：

1. 每个类型洞都有种类及阶段正确的 substitution。
2. 替换中的 runtime 项由 Γ 或明确交付的环境绑定；没有悬空依赖。
3. 解释替换后不存在未解决的必需推断变量。
4. 类型中的必需 comptime 计算已完成，合法范式保留自身身份。
5. 需要的操作环境具有受检的交付关系。

这一过程不清除合法 runtime binder，也不要求先构造 SSA 值。后续重命名或降码须同时更新相关类型引用与环境映射。

验收覆盖 core 相同而 substitution 不同、重复依赖槽位、嵌套 binder 组合、候选回退、跨调用重绑定及本体布局与环境分离。Intern 成本按实际访问的共享结构计；替换深度和语义相等求解另计，不能把所有类型比较宣称为一次指针比较。
