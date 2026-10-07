# 配置、脚本与构建

## Intro

构建先准备输入，再编译使用这些输入的代码。需要生成源码或提供构造宏时，准备工作本身也是工程的一部分。

Flurry 的配置文件使用 comptime 计算。脚本可以服务构建，也可以承担独立选择的工程任务；它们的先后关系由实际依赖决定。

## 语法

~~~text
comptime_function -> pure? comptime fn id(parameter*) (-> result)? clauses? (block | (= expr))
comptime_block -> comptime { (definition | statement)* }
const_def -> const id (: expr)? = expr
macro_import -> @use path
~~~

配置程序复用普通表达式、定义、控制流与 build 提供的工程接口。package.fl 是包描述文件；本篇规定计算和交付关系，接口的具体字段与调用按 build 中相应定义使用。

## 正式定义

### 配置计算

配置文件使用 comptime Flurry，可以抽取函数、复用值并按输入选择结果。anar 消费工程描述和任务关系，luna 完成所需语言检查与编译。

配置程序计算工程安排，与被编译的定义读取自身 build 上下文是两个方向。前者组织某次任务的输入，后者观察已经绑定给该构建实例的信息；不能用一份不断变化的全局对象替代所有实例。

### 脚本的工作

Scripts 可以准备构造宏、生成源码或其他产物，也可以提供独立的工程任务。发现脚本、编译脚本和执行脚本是三个动作；目录扫描不会执行所有脚本。

任务向后续步骤交付的内容应明确，例如宏接口、注册的生成源码或其他构建输入。脚本局部变量不自动注入主源码包根；生成声明只有接入所属源码集合并通过检查后才成为程序的一部分。

### 宏与生成源码

构造宏必须在消费者源码解析前准备好。它可以来自依赖包，也可以由本包先于 src 编译的 scripts 提供。普通 use 不能替代这种准备关系，文件级 `@use` 则负责导入已准备好的宏接口。

生成源码须经明确纳入，按所属目标的源码、模块、可见性与类型规则检查。生成任务成功返回某份文本，不等于它已经是合法 Flurry 程序。

### 依赖失败

若解析 A 必须先执行 B，而 B 的准备又必须先解析同一个 A，则形成不能完成的准备依赖环。模块互相引用名称可以由静态声明求解，不能据此证明这个执行环也可完成。

任务失败、必要 comptime 求值失败或输入缺失时，依赖它的消费步骤不能以旧产物或占位结果继续成功。对外部副作用的回滚须由具体任务契约提供，构建系统不自动保证任意脚本事务化。

### 构建实例与复用

配置和任务复用依据实际输入与依赖。脚本执行的宿主环境与生成产物面向的目标分别记录；本机工具可以为其他目标生成输入，但不能把宿主信息误当作目标信息。

选择一个 umbrella 成员的目标，只调度其所需工作。共享配置可以影响多个目标，每个受影响的结果都要按依赖失效；共享配置本身不合并语言包身份。

## 常见示例

### 配置中的选择可以抽成函数

~~~flurry
pure comptime fn buffer_limit(debug: bool) -> u32 {
    if debug {
        8
    } else {
        64
    }
}

const BUFFER_LIMIT = buffer_limit(build.mode matches .debug);
~~~

这段可复用计算为后续工程接口提供一个值，不需要复制两份配置文本。函数的参数、求值与失败仍遵循普通 comptime 规则。

### 生成文本与注册源码分别完成

下面是生成脚本中的一个辅助函数：

~~~flurry
comptime fn packet_source() -> str {
    "pub struct Packet {}"
}
~~~

返回值只是源码文本。构建任务取得它并把相应生成文件纳入目标后，前端才建立 Packet 声明。仅定义或调用 packet_source，不会在另一个文件中凭空产生 Packet。

纳入目标的生成文件内容为：

~~~flurry
pub struct Packet {}
~~~

如果生成文件属于 generated.packet 模块，包根可以提供稳定入口：

~~~flurry
pub use generated.packet.Packet;
~~~

这里 generated.packet 是构建明确纳入后的模块路径，不是生成函数的局部名字。

### 先准备宏，再解析消费者

已有构造宏接口可以在脚本中定义一组模式组合：

~~~flurry
@use meta.macros.*;

const alternatives = @define_macro1 {
    fn(...patterns: Repetition<meta.ast.FlurryPattern>) -> TokenBuffer {
        let combined = patterns.fold({{ not _ }}, |acc, pattern_ast| {
            {{ $acc or $pattern_ast }}
        });
        {{ $combined => }}
    }
}
~~~

任务交付该宏接口并由消费者文件导入后，消费者可以写：

~~~flurry
use std.prelude.*;

test {
    let answer = "yes";
    if answer is do {
        @alternatives("yes", "Y") println("accepted"),
        _ => println("unknown"),
    }
}
~~~

宏路径的导入按实际交付入口指定，具体构造宏契约见[构造宏](../macro/construction-macro.md)。上述消费者的解析已经需要 alternatives，因此不能等解析完消费者后才准备这个宏。

### 三种依赖分别观察

~~~text
可完成：工具依赖 → 宏脚本 → 宏接口 → 消费者解析 → 消费者检查
可完成：生成任务 → 已注册的源码 → 源码解析 → 类型检查
不可完成：消费者解析 → 宏脚本准备 → 同一消费者解析
~~~

第三条的每一步都等待尚未发生的下一步，没有可先交付的输入。

## 与 Rust / Zig / C / C++ 的关键差异

Cargo 使用 TOML 描述包，并可执行 build.rs 完成准备工作；Flurry 的配置文件本身使用 comptime Flurry，工程接口由 build 提供。[Cargo workspace 配置](https://doc.rust-lang.org/cargo/reference/workspaces.html)、[build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)

Zig 的 build.zig 也是用语言编写构建图的实际对应物；Flurry 还需要把构造宏交付与消费者解析的先后关系纳入调度。[Zig 构建系统](https://ziglang.org/learn/build-system/)

C/C++ 工程可以通过 CMake 自定义命令声明生成物和依赖。这是生成源码任务的实际替代手段；Flurry 用自己的 comptime 配置与 anar 组织相应任务。[CMake 自定义命令](https://cmake.org/cmake/help/latest/command/add_custom_command.html)

## 运行时实现设计引导

构建脚本的执行与生成程序的 runtime 分别计账。脚本输出不能被移动到应用每次启动时执行；生成代码中的实际运行时表达式也不能因来源是脚本就提前执行。

交付给运行时后端的结果应包含确定的代码、数据和目标约束。宏接口、配置求值栈和目录扫描状态无需成为目标程序的运行时对象。

## 编译器实现引导

### 调度契约

将工作表示为具有输入、输出与失败状态的任务。宏准备、生成任务、消费者解析和后续检查分别建立依赖边。只有前置产物成功交付，消费者才能开始。

~~~text
schedule(selected_targets):
    required = collect_required_tasks(selected_targets)
    ready = tasks_whose_inputs_are_available(required)
    while ready is not empty:
        task = ready.pop()
        result = execute_or_reuse(task, task.actual_inputs)
        if result failed:
            mark_dependents_unavailable(task)
        else:
            publish_validated_outputs(task, result)
            enqueue_newly_ready_consumers(task)
    diagnose_required_unfinished_tasks()
~~~

有向环分析应给出真实的“需要哪个产物才能开始哪一步”链。不要把静态名称解析允许的循环导入边直接塞进这张执行依赖图；名称查询有自己的求解算法。

### 产物与失效

源码产物记录内容、所属目标、模块映射和生成来源。生成结果不能只凭路径存在就视为本次任务成功。输入变化后先使受影响的产物失效，再重新调度所需消费者。

缓存依赖至少覆盖实际读取的配置、源内容、目录结构、宏接口及影响结果的宿主／目标信息。只缓存配置文件文本，会漏掉其调用函数或依赖输入的变化。

配置求值可以发现新的必要任务；每次扩展图后检查前置关系，不能靠重复尝试掩盖自举环。对一份已经建立的任务图，依赖收集和拓扑调度为 O(V + E)；配置求值、宏执行、生成物大小和编译成本另计。

必要求值耗尽资源时报告相关任务、调用和输入；不能把截断的生成文本或部分配置当作成功结果交付。
