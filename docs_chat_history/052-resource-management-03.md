# 资源管理：临时值、模式绑定与跨检查阶段的契约

- 日期：2026-09-27
- global_index：052
- theme：resource-management
- subindex：03
- 本轮目标：消化上一轮作者回复，收束六项资源行为，按明确的 IR 检查位置继续临时值、模式与跨函数接口审阅。

## 作者输入与确认状态

作者本轮消息为“已回复”。对照 051 的原始 prompt，R1–R6 的预填及说明保持不变，仅末尾新增：

> 最后，safe mode的所有检查，都是在normalized ir上进行的，verified mode则是在instantiated ir上进行的。

按既有直接修改预填的工作方式，本轮将六项保留的答复记为采纳。作者答复的完整 prompt 已追加到 051；本轮没有把尚未回答的新题当作既定语义。

## 评估与关键衔接

资源的可观察次序已固定：局部按声明逆序；聚合先用户 Drop，再逆序清理剩余字段；覆盖赋值在右侧成功交付后结束旧责任。let、Copy 取得与清理控制流均按保留的 A 处理，不复问上一轮。

新增 IR 规则约束检查的承载位置。safe 不能依靠将未解决义务推迟到 instantiated IR 来通过；verified 的实例证明也不意味着 runtime 参数已求值。PTC 的类型命题职责与这两个程序验证检查位置分开记录。

后续问题集中于仍会改变程序接受性及析构行为的边界：临时 owner 的保活、通配模式是否移出、guard 失败后的责任状态、normalized IR 上的高阶条件接口、verified 契约的证明依赖。没有使用现有编译器源码作为语义依据，没有重新扩展 PTC。

安全等级现行正文的“验证会传播到调用方”尚需按 S5 选定具体含义；模式正文没有确定资源提交时机，按 S2/S3 提问。未擅自改动这些正式文档。

## 完整新 prompt

# 资源管理：临时值、模式绑定与契约边界

日期：2026-09-27；history：052；theme：resource-management；第 03 轮。

**DRAFT — NOT CANONICAL。** 上轮六项预填保持不变，按本轮回复收束为已采纳；以下五项是新的预填审阅。尚不修改正式正文。

## 一、已采纳的规则

| 项目 | 本轮承接的决定 |
|---|---|
| Copy 取得 | 用户名义类型显式 impl/derive；字段条件用于核验能力 |
| let/var | let 限制直接对名称作整值赋值；字段与经指针写入按对象契约检查 |
| 局部清理 | 作用域退出，按绑定声明逆序；重新赋值不重排位置 |
| 聚合清理 | 用户 Drop 在先，再按字段声明逆序清理剩余已初始化字段；元组、数组相应逆序 |
| 覆盖赋值 | 右侧先成功交付新值，再清理目标仍持有的旧值、安装新值；不回滚右侧自身效果 |
| Drop 控制流 | 自动 Drop 不新增可恢复错误返回；效应及所需 handler 环境须覆盖实际清理 |

部分移出继续由实际清理需求约束；不会仅因为类型有用户 Drop 就一概禁止，也不会为了通过检查自动重排析构。

你补充的检查位置按原话保存：

> 最后，safe mode的所有检查，都是在normalized ir上进行的，verified mode则是在instantiated ir上进行的。

因此，**safe 的全部检查以 normalized IR 为载体；verified 检查以 instantiated IR 为载体。** 这与 PTC 的职责是两个维度：PTC 只负责类型命题；内存、路径、状态与行为规格仍属于程序验证体系。

这里有几条直接的实现约束，无需再选择一次检查阶段：

1. safe 必须在 normalized IR 上得到足够的语义信息，包括责任转移、实际清理边、调用契约与语义作用域。不能把尚未解决的 safe 义务寄希望于未来具体实例恰好安全。
2. 表达式被拆成多条 IR 操作时，原来的临时值范围仍要保留；IR 操作边界不自动成为新的析构点。隐式清理可以先表示为带契约的语义动作，不要求在此展开所有析构实现。
3. verified 在 instantiated IR 上面对已经完成相关实例化的程序；runtime 参数仍可能是符号值，并非程序已经运行。
4. 两种检查之间传递契约及其有效依据，不能将程序状态作为 PTC 命题回灌。后续转换须保持已经检查的程序含义。

## 二、本轮的范围

局部绑定已有确定清理规则，接下来处理那些没有普通名称、没有立即选定匹配臂，或者隔着调用边界的资源。这些位置最容易出现“实现看似合理，实际析构行为却随写法改变”的漏洞。

下面的 Trace 是完整定义的教学类型；所有“预填答复”仍可修改。没有将本轮候选当作已存在的标准库接口。

~~~flurry
struct Trace {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn make_trace(id: u32) -> Trace {
    Trace { id: id }
}
~~~

## 三、五项预填

### S1. 临时 owner 的视图被保存时，是否自动延长 owner 的生命期？

现行 [接收者适配](/home/fin/luna/docs/type/receiver-adaptation.md) 已要求保留临时清理，但没有规定它的精确范围。

~~~flurry
fn temporary_read() {
    let number = make_trace(1).inspect();
    println("number {}", number);
}

fn temporary_pointer() {
    let pointer = make_trace(2).ref;
    println("number {}", pointer.inspect());
}

fn named_owner() {
    let owner = make_trace(3);
    let pointer = owner.ref;
    println("number {}", pointer.inspect());
}
~~~

- **A. 未被交付为 owner 的临时值在完整表达式结束时清理；保存视图不自动延长来源。** temporary_read 合法，先 `drop 1` 再 `number 1`；temporary_pointer 的后续读取非法；named_owner 合法，先 `number 3` 再 `drop 3`。
- **B. 初始化绑定时，把其结果依赖的临时 owner 一并延长到该绑定所在作用域。** temporary_pointer 可合法；方法返回的视图也须使用同一依赖规则，编译器需维护隐藏责任及固定清理位置。
- **C. 仅对直接取址等列明的语法形态延长。** 可少做依赖传播，但直接 `.ref` 与通过辅助函数取得同一视图可能有不同生命期，需要逐项定义例外。

**预填答复：A。** 用显式 owner 绑定表达跨语句保活，普通调用返回的独立数值不携带来源生命期。该规则同样适用于 `String.as_str()` 产生的视图。

这里“完整表达式”至少包括整个初始化右侧、整个赋值和整个表达式语句；一条调用链要完成本次访问后，才清理它仍持有的临时责任。普通 bool 条件求值引入的临时值在进入选定分支正文前结束；被作为 match 主体保留的临时值则支撑实际匹配及所选臂，直到整个 match 表达式结束。if-is 的匹配主体相应支撑所选分支。移动交付到绑定或结果中的 owner 不在源处再清理。

尚未完成的子表达式因错误或其他控制转移离开时，清理已经完成构造的临时值。需要跨该边保留的结果，必须在相应清理之后仍有效。

以上范围属于本题 A 的完整候选，而非从“完整表达式”四个字隐含推出。normalization 拆分调用链时必须保存这些范围。

### S2. `_` 匹配已有位置时，是否消耗该值？

[模式匹配](/home/fin/luna/docs/control-flow/pattern-matching/index.md) 只规定 `_`“匹配且不绑定”，没有确定资源行为。以下两个右侧有不同来源：

~~~flurry
fn discard_existing() {
    let value = make_trace(4);
    let _ = value;
    println("number {}", value.inspect());
}

fn discard_created() {
    let _ = make_trace(5);
    println("after");
}
~~~

- **A. 对已有位置，`_` 不建立按值绑定，因此不因这个模式额外移出；新产生而无人接收的值仍须清理。** discard_existing 输出 `number 4` 后 `drop 4`；结合 S1-A，discard_created 输出 `drop 5` 后 `after`。
- **B. 右侧先按通常按值交付规则消耗，再立即清理被忽略的结果。** discard_existing 后续读取非法；discard_created 的时序与 A 相同。
- **C. `_` 建立不可引用的隐藏 owner 绑定，清理到该绑定作用域结束。** 两例均转移责任；新值的析构时间与普通临时值不同。

**预填答复：A。** `_` 本身不要求移出已有位置，但不会取消右侧本来要执行的调用、副作用或资源交付。普通命名绑定仍按 Copy/move 规则交付值。

这也不能成为 pattern view 的死代码删除规则。语法制导协议若为后项建立了需要保留的内部元素或拥有值，即使当前子模式是 `_`，那些已生成的操作和后项依赖仍按契约处理。仅在副作用和依赖都允许时，才可消去操作。

### S3. 结构匹配成功但 guard 失败时，绑定的所有权何时交付？

现行 [模式匹配](/home/fin/luna/docs/control-flow/pattern-matching/index.md) 定义了 guard 与 `as` 绑定；[TuplePatternView 草稿](/home/fin/vilya/examples/2026-9-7-pattern-traits.fl) 又允许准备步骤和逐项访问改变状态。资源规则必须既支持普通回退，也保留真实副作用。

~~~flurry
enum Payload {
    item(Trace),
    empty,
}

fn choose(payload: Payload) {
    payload match {
        .item(value) if value.inspect() == 0 => println("zero"),
        .item(value) => println("other {}", value.inspect()),
        _ => (),
    }
}

test {
    choose(Payload.item(make_trace(6)));
}
~~~

- **A. 对已有位置的普通模式绑定，先准备可观察位置，guard 成功后才提交隐式的按值交付。** 本例可回退到第二臂，输出 `other 6`，最后只清理一次 Trace。准备期间按位置检查访问，不新增公开引用类型。
- **B. 绑定在结构子模式成功时立即按值交付。** guard 失败也不返还已移走的值；本例第二臂需要已经移出的 payload，因此拒绝。回退程序必须另写只观察的前置检查。
- **C. 首版只允许 Copy 绑定参与这种 guard，非 Copy 值先用显式条件观察后再解构。** 避免待提交绑定状态，但常见按内容选择并移出资源的写法更繁琐。

**预填答复：A。** 这个提交点只约束普通绑定造成的隐式 move。已经执行的 pattern view 调用、guard 写入、日志和资源动作按真实次序生效，不自动回滚。

guard 通过准备好的位置进行观察或契约允许的写入；若需要按值消耗尚未提交的绑定，则移到选定臂正文处理。guard 改写了 tag、长度或依赖事实时，后续匹配依据实际后置状态重新检查；不能沿用失效的结构匹配证据。

失败后继先兑现本次尝试所持有、且不再支撑后续访问的临时清理，再进入下一候选臂。自定义 pattern view 若自身已经消费或终结 subject，就不能套用“普通隐式 move 尚未提交”来恢复它；其后续匹配必须满足该操作真实交付的状态。

`as whole` 同样不产生两份独立所有权。部分绑定与整体绑定若最终重复消耗同一责任，由程序验证层拒绝；不用新增一套类型命题求解规则。

### S4. safe 高阶函数的调用条件，能否在 normalized IR 上推导并随接口交付？

二十例已确定不能只用函数形状和无序 effect 集合检查回调。本题补足默认接口策略，使它在你指定的检查阶段内闭合。

~~~flurry
fn inspect_text(text: *String) {
    println("{}", text.as_str());
}

fn append_text(text: *String) {
    text.push_str(" more");
}

fn keep_view(text: *String, action: fn(*String)) {
    let view = text.as_str();
    action(text);
    println("{}", view);
}

test {
    var text = "hello".to_string();
    keep_view(text.ref, inspect_text);
    keep_view(text.ref, append_text);
    -- 没有保证旧缓冲区被保留时，第二次调用不能通过。
}
~~~

- **A. 在 normalized IR 上推导带回调契约参数的关系摘要，并作为接口的一部分交付。** keep_view 得到“回调结束后旧 view 仍可读”的条件；调用点用已知回调摘要兑现它。函数定义和调用点均在各自 normalized IR 上检查。
- **B. 这类条件必须由作者显式声明；没有声明则拒绝 helper 定义。** 接口边界直接，但高阶辅助函数会需要更多 notation；对应表面写法仍需另行整理。
- **C. 只接受 normalized IR 中已经暴露具体调用目标的版本，未知回调接口不支持这些保留关系。** 实现较小，但本例通用 helper 无法成立，动态选择回调也明显受限。

**预填答复：A。** 关系摘要至少保留入口要求、各出口状态、效果上界、结果依赖和剩余责任。回调契约参数由程序验证层管理，不成为 PTC 的原子命题或新的类型工厂。

此时检查 helper 是在明确的回调条件下证明它；使用 helper 时再检查实参是否满足这些条件。不能保留一段没有条件接口、尚未检查的 helper 主体，等 instantiated IR 出现具体回调才补做 safe 检查。

普通函数值的表面类型可以相同，但其可用程序契约不能凭类型擦除而丢失。动态来源合流后，至少要有所有可选目标都满足的共同摘要；未知来源不能自动当作保留内存。无主体声明、外部函数和递归边界缺少必要契约时，应指出具体缺口，不默认 pure。

inspect_text 的系统输出并不妨碍保留 view；append_text 的潜在重分配才与本例要求冲突。诊断应指出旧字符区域，而不是笼统说“回调有副作用”。

### S5. safe 调用 verified 时，契约检查与被调方证明怎样交接？

[安全等级旧正文](/home/fin/luna/docs/program-verification/safety.md) 的调用表写着调用 verified 时“验证会传播到调用方”。[memory 草稿](/home/fin/vilya/examples/2026-9-16-memory-effects.fl) 则允许 safe 按已知契约调用 verified，而不重做完整强证明。你本轮明确的两种 IR 载体要求把这句“传播”的含义写清楚。

~~~flurry
verified fn checked_increment(value: u32) -> result: u32
where requires value < 4294967295u32,
    ensures result == value + 1
{
    value + 1
}

fn increment_checked(value: u32) -> u32 {
    assumes value < 4294967295u32;
    checked_increment(value)
}
~~~

- **A. safe 在 normalized IR 上按 verified 的声明契约完成调用方检查；被调方实现的证明义务在相应 instantiated IR 上兑现。** 契约使用有明确的证明依赖；被调实例证明失败时不得交付依赖它的程序。safe 不因此升级模式或改换检查载体。
- **B. 必须先完成具体 verified 实例证明，safe 的 normalized IR 检查才允许使用其保证。** 边界较直接，但检查调度会等待实例化结果，参数化调用与相互递归需要处理更多依赖。
- **C. safe 只使用一份能独立按 safe 规则成立的保守契约，verified 的额外保证不向它开放。** 两套检查较隔离，但已证明的库性质难以直接服务 safe 调用方。

**预填答复：A。** 调用方证明“满足入口契约即可安全地继续”；被调方证明“该实例的实现兑现此契约”。前者在 normalized IR 上完成，后者在 instantiated IR 上完成。程序交付要求有关证明依赖全部成立；不能把尚未验证的实现当作无条件可信。

示例中的 assumes 由 safe 检查器确认观察和检查是合法的；静态未知时，在加法前保留受检 runtime 条件。verified 证明在相同前提下加法不溢出且满足结果关系。检查阶段不同不改变 predicate 的含义，也不意味着 runtime 参数在实例化时已知。

调用者若无法表达、合法检查或满足某项入口要求，仍须拒绝或补充有效证据。高次数性质可以按有来源的整体契约使用，不要求 safe 展开被调方的完整证明；可自动展开的算术仍服从三次以内的边界。

此题选择的是跨接口的证明依赖规则，不重新讨论 PTC 的职责，也不要求 safe 跟随 verified 在 instantiated IR 上重新检查自己的整个函数体。

## 四、下一步与自检

- **已确认**：上一轮 R1–R6；safe 全部检查在 normalized IR，verified 在 instantiated IR；PTC 与程序验证的职责分离。
- **本轮待确认**：S1–S5，分别涉及临时保活、丢弃匹配、guard 提交点、高阶条件接口、跨模式契约交接。
- **内容规划**：这一轮收束后，资源正文已有消耗与复制、绑定与赋值、清理与失败、临时与模式、引用有效性、跨函数检查六条主线。完整模式协议和 verified 证明算法由各自主题承接。
- **语法／风格自检**：示例按现行 grammar/style 使用后缀投影、取址、解引用、模式 guard、普通枚举变体小写、requires/ensures 子句与结果绑定；花括号结尾项不加分号。Trace、Payload 及辅助函数均在本轮定义；String/println 沿用作者已有 API。
- **文档边界**：本轮更新 prompt 与 history；不改正式 docs，也不把候选示例宣称为已通过编译器验证。

## 落盘与自检

- 051 追加作者完整答复；本轮 prompt 清除旧题后写入新一轮，与本记录逐字一致。
- 五题各有 A/B/C 候选和可修改预填；六个 Flurry 代码块按 grammar/style 自检。
- 已检查代码块分隔符、括号、禁止混入的写法与本地链接；未运行或宣称通过 Flurry 编译器。
- 本轮仅更新 prompt、051 与本记录；未改正式 docs 或外部 proposal。

## 本轮最终回复

已归档你的回复，并更新 [prompt](/home/fin/luna/prompt)。

R1–R6 按保留的预填收束；检查位置已明确为 **safe → normalized IR，verified → instantiated IR**。

新一轮预填了五题：临时值保活、`_` 是否消耗、guard 失败时的所有权、高阶契约推导、safe／verified 契约交接。均配有代码和候选方案，已确定的规则不再重复提问。


## 作者答复（prompt，2026-09-27）

以下为作者编辑后的 prompt 全文，原样保存。S1、S2、S5 保留预填；S3、S4 后新增的作者纠正优先于仍留在正文中的候选答案，不能把两项旧预填记为已采纳。下一轮在 053 中继续资源管理主题，讨论最后一节 runtime 纯度。

# 资源管理：临时值、模式绑定与契约边界

日期：2026-09-27；history：052；theme：resource-management；第 03 轮。

**DRAFT — NOT CANONICAL。** 上轮六项预填保持不变，按本轮回复收束为已采纳；以下五项是新的预填审阅。尚不修改正式正文。

## 一、已采纳的规则

| 项目 | 本轮承接的决定 |
|---|---|
| Copy 取得 | 用户名义类型显式 impl/derive；字段条件用于核验能力 |
| let/var | let 限制直接对名称作整值赋值；字段与经指针写入按对象契约检查 |
| 局部清理 | 作用域退出，按绑定声明逆序；重新赋值不重排位置 |
| 聚合清理 | 用户 Drop 在先，再按字段声明逆序清理剩余已初始化字段；元组、数组相应逆序 |
| 覆盖赋值 | 右侧先成功交付新值，再清理目标仍持有的旧值、安装新值；不回滚右侧自身效果 |
| Drop 控制流 | 自动 Drop 不新增可恢复错误返回；效应及所需 handler 环境须覆盖实际清理 |

部分移出继续由实际清理需求约束；不会仅因为类型有用户 Drop 就一概禁止，也不会为了通过检查自动重排析构。

你补充的检查位置按原话保存：

> 最后，safe mode的所有检查，都是在normalized ir上进行的，verified mode则是在instantiated ir上进行的。

因此，**safe 的全部检查以 normalized IR 为载体；verified 检查以 instantiated IR 为载体。** 这与 PTC 的职责是两个维度：PTC 只负责类型命题；内存、路径、状态与行为规格仍属于程序验证体系。

这里有几条直接的实现约束，无需再选择一次检查阶段：

1. safe 必须在 normalized IR 上得到足够的语义信息，包括责任转移、实际清理边、调用契约与语义作用域。不能把尚未解决的 safe 义务寄希望于未来具体实例恰好安全。
2. 表达式被拆成多条 IR 操作时，原来的临时值范围仍要保留；IR 操作边界不自动成为新的析构点。隐式清理可以先表示为带契约的语义动作，不要求在此展开所有析构实现。
3. verified 在 instantiated IR 上面对已经完成相关实例化的程序；runtime 参数仍可能是符号值，并非程序已经运行。
4. 两种检查之间传递契约及其有效依据，不能将程序状态作为 PTC 命题回灌。后续转换须保持已经检查的程序含义。

## 二、本轮的范围

局部绑定已有确定清理规则，接下来处理那些没有普通名称、没有立即选定匹配臂，或者隔着调用边界的资源。这些位置最容易出现“实现看似合理，实际析构行为却随写法改变”的漏洞。

下面的 Trace 是完整定义的教学类型；所有“预填答复”仍可修改。没有将本轮候选当作已存在的标准库接口。

~~~flurry
struct Trace {
    id: u32,

    fn inspect(*self) -> u32 {
        self.id.*
    }
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn make_trace(id: u32) -> Trace {
    Trace { id: id }
}
~~~

## 三、五项预填

### S1. 临时 owner 的视图被保存时，是否自动延长 owner 的生命期？

现行 [接收者适配](/home/fin/luna/docs/type/receiver-adaptation.md) 已要求保留临时清理，但没有规定它的精确范围。

~~~flurry
fn temporary_read() {
    let number = make_trace(1).inspect();
    println("number {}", number);
}

fn temporary_pointer() {
    let pointer = make_trace(2).ref;
    println("number {}", pointer.inspect());
}

fn named_owner() {
    let owner = make_trace(3);
    let pointer = owner.ref;
    println("number {}", pointer.inspect());
}
~~~

- **A. 未被交付为 owner 的临时值在完整表达式结束时清理；保存视图不自动延长来源。** temporary_read 合法，先 `drop 1` 再 `number 1`；temporary_pointer 的后续读取非法；named_owner 合法，先 `number 3` 再 `drop 3`。
- **B. 初始化绑定时，把其结果依赖的临时 owner 一并延长到该绑定所在作用域。** temporary_pointer 可合法；方法返回的视图也须使用同一依赖规则，编译器需维护隐藏责任及固定清理位置。
- **C. 仅对直接取址等列明的语法形态延长。** 可少做依赖传播，但直接 `.ref` 与通过辅助函数取得同一视图可能有不同生命期，需要逐项定义例外。

**预填答复：A。** 用显式 owner 绑定表达跨语句保活，普通调用返回的独立数值不携带来源生命期。该规则同样适用于 `String.as_str()` 产生的视图。

这里“完整表达式”至少包括整个初始化右侧、整个赋值和整个表达式语句；一条调用链要完成本次访问后，才清理它仍持有的临时责任。普通 bool 条件求值引入的临时值在进入选定分支正文前结束；被作为 match 主体保留的临时值则支撑实际匹配及所选臂，直到整个 match 表达式结束。if-is 的匹配主体相应支撑所选分支。移动交付到绑定或结果中的 owner 不在源处再清理。

尚未完成的子表达式因错误或其他控制转移离开时，清理已经完成构造的临时值。需要跨该边保留的结果，必须在相应清理之后仍有效。

以上范围属于本题 A 的完整候选，而非从“完整表达式”四个字隐含推出。normalization 拆分调用链时必须保存这些范围。

### S2. `_` 匹配已有位置时，是否消耗该值？

[模式匹配](/home/fin/luna/docs/control-flow/pattern-matching/index.md) 只规定 `_`“匹配且不绑定”，没有确定资源行为。以下两个右侧有不同来源：

~~~flurry
fn discard_existing() {
    let value = make_trace(4);
    let _ = value;
    println("number {}", value.inspect());
}

fn discard_created() {
    let _ = make_trace(5);
    println("after");
}
~~~

- **A. 对已有位置，`_` 不建立按值绑定，因此不因这个模式额外移出；新产生而无人接收的值仍须清理。** discard_existing 输出 `number 4` 后 `drop 4`；结合 S1-A，discard_created 输出 `drop 5` 后 `after`。
- **B. 右侧先按通常按值交付规则消耗，再立即清理被忽略的结果。** discard_existing 后续读取非法；discard_created 的时序与 A 相同。
- **C. `_` 建立不可引用的隐藏 owner 绑定，清理到该绑定作用域结束。** 两例均转移责任；新值的析构时间与普通临时值不同。

**预填答复：A。** `_` 本身不要求移出已有位置，但不会取消右侧本来要执行的调用、副作用或资源交付。普通命名绑定仍按 Copy/move 规则交付值。

这也不能成为 pattern view 的死代码删除规则。语法制导协议若为后项建立了需要保留的内部元素或拥有值，即使当前子模式是 `_`，那些已生成的操作和后项依赖仍按契约处理。仅在副作用和依赖都允许时，才可消去操作。

### S3. 结构匹配成功但 guard 失败时，绑定的所有权何时交付？

现行 [模式匹配](/home/fin/luna/docs/control-flow/pattern-matching/index.md) 定义了 guard 与 `as` 绑定；[TuplePatternView 草稿](/home/fin/vilya/examples/2026-9-7-pattern-traits.fl) 又允许准备步骤和逐项访问改变状态。资源规则必须既支持普通回退，也保留真实副作用。

~~~flurry
enum Payload {
    item(Trace),
    empty,
}

fn choose(payload: Payload) {
    payload match {
        .item(value) if value.inspect() == 0 => println("zero"),
        .item(value) => println("other {}", value.inspect()),
        _ => (),
    }
}

test {
    choose(Payload.item(make_trace(6)));
}
~~~

- **A. 对已有位置的普通模式绑定，先准备可观察位置，guard 成功后才提交隐式的按值交付。** 本例可回退到第二臂，输出 `other 6`，最后只清理一次 Trace。准备期间按位置检查访问，不新增公开引用类型。
- **B. 绑定在结构子模式成功时立即按值交付。** guard 失败也不返还已移走的值；本例第二臂需要已经移出的 payload，因此拒绝。回退程序必须另写只观察的前置检查。
- **C. 首版只允许 Copy 绑定参与这种 guard，非 Copy 值先用显式条件观察后再解构。** 避免待提交绑定状态，但常见按内容选择并移出资源的写法更繁琐。

**预填答复：A。** 这个提交点只约束普通绑定造成的隐式 move。已经执行的 pattern view 调用、guard 写入、日志和资源动作按真实次序生效，不自动回滚。

guard 通过准备好的位置进行观察或契约允许的写入；若需要按值消耗尚未提交的绑定，则移到选定臂正文处理。guard 改写了 tag、长度或依赖事实时，后续匹配依据实际后置状态重新检查；不能沿用失效的结构匹配证据。

失败后继先兑现本次尝试所持有、且不再支撑后续访问的临时清理，再进入下一候选臂。自定义 pattern view 若自身已经消费或终结 subject，就不能套用“普通隐式 move 尚未提交”来恢复它；其后续匹配必须满足该操作真实交付的状态。

`as whole` 同样不产生两份独立所有权。部分绑定与整体绑定若最终重复消耗同一责任，由程序验证层拒绝；不用新增一套类型命题求解规则。

tuple pattern view的elaboration行为完全是编译期行为啊，何谈分步提交所有权。

### S4. safe 高阶函数的调用条件，能否在 normalized IR 上推导并随接口交付？

二十例已确定不能只用函数形状和无序 effect 集合检查回调。本题补足默认接口策略，使它在你指定的检查阶段内闭合。

~~~flurry
fn inspect_text(text: *String) {
    println("{}", text.as_str());
}

fn append_text(text: *String) {
    text.push_str(" more");
}

fn keep_view(text: *String, action: fn(*String)) {
    let view = text.as_str();
    action(text);
    println("{}", view);
}

test {
    var text = "hello".to_string();
    keep_view(text.ref, inspect_text);
    keep_view(text.ref, append_text);
    -- 没有保证旧缓冲区被保留时，第二次调用不能通过。
}
~~~

- **A. 在 normalized IR 上推导带回调契约参数的关系摘要，并作为接口的一部分交付。** keep_view 得到“回调结束后旧 view 仍可读”的条件；调用点用已知回调摘要兑现它。函数定义和调用点均在各自 normalized IR 上检查。
- **B. 这类条件必须由作者显式声明；没有声明则拒绝 helper 定义。** 接口边界直接，但高阶辅助函数会需要更多 notation；对应表面写法仍需另行整理。
- **C. 只接受 normalized IR 中已经暴露具体调用目标的版本，未知回调接口不支持这些保留关系。** 实现较小，但本例通用 helper 无法成立，动态选择回调也明显受限。

**预填答复：A。** 关系摘要至少保留入口要求、各出口状态、效果上界、结果依赖和剩余责任。回调契约参数由程序验证层管理，不成为 PTC 的原子命题或新的类型工厂。

此时检查 helper 是在明确的回调条件下证明它；使用 helper 时再检查实参是否满足这些条件。不能保留一段没有条件接口、尚未检查的 helper 主体，等 instantiated IR 出现具体回调才补做 safe 检查。

普通函数值的表面类型可以相同，但其可用程序契约不能凭类型擦除而丢失。动态来源合流后，至少要有所有可选目标都满足的共同摘要；未知来源不能自动当作保留内存。无主体声明、外部函数和递归边界缺少必要契约时，应指出具体缺口，不默认 pure。

inspect_text 的系统输出并不妨碍保留 view；append_text 的潜在重分配才与本例要求冲突。诊断应指出旧字符区域，而不是笼统说“回调有副作用”。

这种复杂场景，还是推荐用户用assumption吧，你不能把所有功能需求都推导最上层的程序验证，我们还有基于outcome separation logic的verified mode和runtime verification呢

### S5. safe 调用 verified 时，契约检查与被调方证明怎样交接？

[安全等级旧正文](/home/fin/luna/docs/program-verification/safety.md) 的调用表写着调用 verified 时“验证会传播到调用方”。[memory 草稿](/home/fin/vilya/examples/2026-9-16-memory-effects.fl) 则允许 safe 按已知契约调用 verified，而不重做完整强证明。你本轮明确的两种 IR 载体要求把这句“传播”的含义写清楚。

~~~flurry
verified fn checked_increment(value: u32) -> result: u32
where requires value < 4294967295u32,
    ensures result == value + 1
{
    value + 1
}

fn increment_checked(value: u32) -> u32 {
    assumes value < 4294967295u32;
    checked_increment(value)
}
~~~

- **A. safe 在 normalized IR 上按 verified 的声明契约完成调用方检查；被调方实现的证明义务在相应 instantiated IR 上兑现。** 契约使用有明确的证明依赖；被调实例证明失败时不得交付依赖它的程序。safe 不因此升级模式或改换检查载体。
- **B. 必须先完成具体 verified 实例证明，safe 的 normalized IR 检查才允许使用其保证。** 边界较直接，但检查调度会等待实例化结果，参数化调用与相互递归需要处理更多依赖。
- **C. safe 只使用一份能独立按 safe 规则成立的保守契约，verified 的额外保证不向它开放。** 两套检查较隔离，但已证明的库性质难以直接服务 safe 调用方。

**预填答复：A。** 调用方证明“满足入口契约即可安全地继续”；被调方证明“该实例的实现兑现此契约”。前者在 normalized IR 上完成，后者在 instantiated IR 上完成。程序交付要求有关证明依赖全部成立；不能把尚未验证的实现当作无条件可信。

示例中的 assumes 由 safe 检查器确认观察和检查是合法的；静态未知时，在加法前保留受检 runtime 条件。verified 证明在相同前提下加法不溢出且满足结果关系。检查阶段不同不改变 predicate 的含义，也不意味着 runtime 参数在实例化时已知。

调用者若无法表达、合法检查或满足某项入口要求，仍须拒绝或补充有效证据。高次数性质可以按有来源的整体契约使用，不要求 safe 展开被调方的完整证明；可自动展开的算术仍服从三次以内的边界。

此题选择的是跨接口的证明依赖规则，不重新讨论 PTC 的职责，也不要求 safe 跟随 verified 在 instantiated IR 上重新检查自己的整个函数体。

## 四、下一步与自检

- **已确认**：上一轮 R1–R6；safe 全部检查在 normalized IR，verified 在 instantiated IR；PTC 与程序验证的职责分离。
- **本轮待确认**：S1–S5，分别涉及临时保活、丢弃匹配、guard 提交点、高阶条件接口、跨模式契约交接。
- **内容规划**：这一轮收束后，资源正文已有消耗与复制、绑定与赋值、清理与失败、临时与模式、引用有效性、跨函数检查六条主线。完整模式协议和 verified 证明算法由各自主题承接。
- **语法／风格自检**：示例按现行 grammar/style 使用后缀投影、取址、解引用、模式 guard、普通枚举变体小写、requires/ensures 子句与结果绑定；花括号结尾项不加分号。Trace、Payload 及辅助函数均在本轮定义；String/println 沿用作者已有 API。
- **文档边界**：本轮更新 prompt 与 history；不改正式 docs，也不把候选示例宣称为已通过编译器验证。
