# 替换与运行时依赖交付

## Intro

实现既可以依赖类型参数，也可以依赖具体的运行时对象。替换说明这些参数对应谁；环境说明操作执行时怎样取得它们。

依赖能够从输入恢复时，调用者无需反复填写。没有来源的依赖则要明确绑定，不能根据局部变量的名字或类型猜测。

## 语法

~~~text
quote_decl_clause -> quote decl_clause
normal_form_head_application -> expr<argument*>
optional_arg -> .id = expr
type_cast -> expr.as(expr)
closure_type -> %expr expr
~~~

开放实现视角使用已有表达式组合：`value.as(Trait)<.dependency = object>`。尖括号绑定选定实现的开放参数；它不为 Trait 增加参数，也不为随后调用的方法增加普通实参。

## 正式定义

### 替换的来源

实现参数可以由显式实参、主体与完整接口实例的匹配、已经交付的实现证据确定。这些来源共同施加约束，不按优先级覆盖彼此。

如果目标类型已经固定 allocator1，显式指定 allocator2 必须证明它们是同一依赖；不能重新绑定该目标。两个对象类型或内容相同，不足以证明其 runtime identity 相同。

没有来源的依赖保留为开放参数。调用能力交付前必须补齐；作用域中恰好存在一个同类型或同名对象，不是合法补齐依据。

### 开放实现视角

`value.as(Trait)` 先指定接口来源。该实现仍有开放参数时，可用范式应用封闭这些参数，再调用或保存相应视角。直接引用的参数名属于其公开绑定入口，不能当作不可观察的实现私有名称。

对 quote 依赖参数，绑定保存 runtime identity 与取得关系，不读取未来内容，也不因为尖括号应用而复制对象。阶段检查使用依赖参数原本的契约。

开放参数只封闭实现，不制造另一份全局 implementation。两次绑定可使用同一声明和不同环境；它们不会因环境不同取得覆盖其他实现的优先级。

### 依赖身份与取得方式

知道依赖是谁，不等于当前可以取得它。环境可以位于外层聚合、调用的隐藏参数、合法全局对象或封闭携带者。具体表示不强制逐值增加字段。

跨函数、返回值、捕获或动态封装边界时，必须保留操作实际需要的环境。依赖无法交付时应在边界拒绝，而不是在后续使用点寻找一个替代对象。Any/any 的放宽不创造环境。

泛型接口已经要求某能力时，其合法证据可携带所需环境。函数体若需要一个原接口没有来源的新依赖，必须通过可检查的参数或契约表达，不能隐式修改全部调用者的要求。

### 生命周期与状态

普通调用视角不重新分配原对象的清理责任。Drop 所需证据随责任建立而固定，移动时一起传递；后续 `.as(...)` 不改变它。依赖绑定也不延长外部对象的生命。

两条分支分别绑定不同依赖，汇合时必须保留相应关系。若接口允许封闭表示，可以携带分支对应的环境；否则需要显式调整接口。不能因为两边本体类型相同就抹去不同依赖。

## 常见示例

### 从目标类型恢复依赖

以下读数依赖一个校准对象。校准对象是运行时值，类型只保留它的身份：

~~~flurry
struct Calibration {
    offset: i32,
}

trait Measure {
    fn measure(*self) -> i32;
}

struct Sensor
where quote calibration: lift *Calibration
{
    value: i32,
}

impl Measure for Sensor<calibration>
where quote calibration: lift *Calibration
{
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
    let calibration = Calibration { offset: 3 }
    let sensor = Sensor<calibration.ref> { value: 10 }
    show_sensor(sensor.ref);
}
~~~

输出 `13`。调用 show_sensor 时，从参数目标恢复 calibration 的绑定；函数执行时仍要取得该校准对象。它的 offset 不在类型匹配时被读取。

Tensor 的 allocator 采用相同规则：目标类型参数已经记录 allocator 时，Drop、Clone 等操作从匹配取得同一个 identity，再通过环境兑现实际访问。它不需要另一套基于名称的 allocator 搜索。

### 显式绑定缺失依赖

当校准对象没有出现在 Reading 的类型中时，调用者明确选择它。此例独立于上一例：

~~~flurry
struct Calibration {
    offset: i32,
}

struct Reading {
    value: i32,
}

trait Measure {
    fn measure(*self) -> i32;
}

impl Measure for Reading
where quote calibration: lift *Calibration
{
    fn measure(*self) -> i32 {
        self.value.* + calibration.offset.*
    }
}

test {
    let first = Calibration { offset: 3 }
    let second = Calibration { offset: 8 }
    let reading = Reading { value: 10 }

    println("{}", reading.as(Measure)<.calibration = first.ref>.measure());
    println("{}", reading.as(Measure)<.calibration = second.ref>.measure());
}
~~~

输出 `13`、`18`。两次调用使用同一份 impl，绑定了不同环境；没有给 Reading 注册两份实现。

以下调用应拒绝，即使当前作用域恰好有名为 calibration 的变量：

~~~flurry
-- 接续上例的声明；拒绝例。
fn missing_binding(reading: *Reading, calibration: *Calibration) -> i32 {
    reading.as(Measure).measure()
}
~~~

修正是在实现视角上绑定 `.calibration = calibration`。仅把变量改成同名并不能修正。

### 保留环境的跨调用使用

已封闭的视角可以保存，在其依赖有效期间反复使用：

~~~flurry
-- 接续显式绑定例的声明。
fn show_calibrated(reading: *Reading, calibration: *Calibration) {
    let measured = reading.as(Measure)<.calibration = calibration>;
    println("{}", measured.measure());
    println("{}", measured.measure());
}
~~~

第二次调用复用绑定，不重新搜索校准对象。能否反复调用仍取决于实际方法和环境的资源契约；若第一次调用消耗了必需环境，就不能据“证据相同”允许第二次调用。

## 与 Rust / Zig / C / C++ 的关键差异

C 的显式上下文参数、C++ 的携带状态对象和 Zig 的显式参数都可传递操作环境。Rust trait 的类型参数和关联项不是本文 runtime identity 替换的完整对应物。[C11 §6.7.6.3](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf)、[C++ virtual](https://eel.is/c++draft/class.virtual)、[Zig comptime](https://ziglang.org/documentation/master/#comptime)、[Rust traits](https://doc.rust-lang.org/reference/items/traits.html)

Flurry 将“操作依赖哪个对象”纳入可检查的实现交付关系，但这不要求环境一定以内嵌字段表示。

## 运行时实现设计引导

1. 列出所选操作真正需要的 runtime identity。
2. 从已固定的环境关系取得对象，不按局部名称搜索。
3. 在跨边界时传递或封闭缺少的环境。
4. 调用时保留对象有效性、访问效果和仿射状态检查。
5. 自动清理使用已经随责任传递的证据。

共享外层环境可以减少重复存储；不同 identity 即使布局相同也不能被合并。临时视角的物理分配可消除，但不能消除其依赖和转换效果。

## 编译器实现引导

内部证据可组织为：

~~~text
Evidence = (impl_id, trait_instance, σ, obligations, environment_plan)
σ[comptime_parameter] = value or symbolic term
σ[runtime_dependency] = binding identity
environment_plan[identity] = legal access path
~~~

具体步骤：

1. 在候选独立状态中合并显式绑定、匹配结果和已有证据约束。
2. 对 quote 槽位建立 identity 关系，不执行对象内容读取。
3. 检查重复来源一致性；缺失来源与来源冲突分别诊断。
4. 实现选择完成后，按依赖图建立环境取得计划。
5. 在调用和逃逸边界检查取得计划可执行、依赖仍有效。
6. 保存证据或消去静态部分；布局选择不得改变 σ 的意义。

替换须避免捕获并区分定义端参数与查询元变量。相同源码名字不构成 identity；局部绑定一致改名与外部依赖相等也是不同关系。

类型引用以“可 intern 的带洞结构＋substitution”保存这些关系；替换组合生成新的引用关系，不原地修改共享结构。Environment plan 保留使用点如何取得依赖，不能由模板 ID 或 substitution 的存在单独证明访问有效。具体算法见[两层类型表示](../compilation/runtime-dependent-types.md)。

缓存分为候选集合、符号替换、使用点交付检查。前两者不能替代第三者随程序状态进行的有效性检查。可共享代码模板，但不能仅按依赖类型把两份语义证据合并。
