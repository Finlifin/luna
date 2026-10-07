# 内存效果

## Intro

一次调用可以只改一个计数器，也可以替换整个缓冲区。调用方需要知道它影响哪里，以及调用后哪些访问仍然成立。

内存效果描述这些行为；前后条件与结果关系补足它们的状态含义。声明一次 write，不等于已经取得任意写入许可。

## 语法

~~~text
outcome_clause -> outcomes expr
memory_predicate -> predicate id(parameter*) (-> expr)? clauses? block
requires_clause -> requires expr
ensures_clause -> ensures expr
~~~

程序 predicate 使用普通 `predicate` 声明，不加 comptime。区域参数可通过 quote 参数保留表达式及其绑定。read/write 等接受区域表达式，示例使用已有的尖括号或谓词调用形式。

## 正式定义

### 六种效果

| 效果 | 描述的行为 | 不自动提供的保证 |
|---|---|---|
| read | 读取指定区域 | 目标存活、初始化和类型合法仍需检查 |
| write | 修改指定区域，撤销受影响旧内容事实 | 不自动释放对象，也不自动使所有指针失效 |
| kill | 终结指定区域的当前世代 | 不递归析构所有保存其地址的对象 |
| connect | 目标保留来源提供的依赖 | 不取得来源所有权，不使失效来源复活 |
| move | 转移值、要求与可转移责任 | 不自动终结所拥有的所有外部分配 |
| system | 外部可观察作用，如输出 | 不替代真实 read/write/kill 或逃逸契约 |

复制视图时，connect 传播其实际目标要求，不把中间变量的存储变成来源。move 的目的位置由赋值、参数或返回动作确定。

常量读取可以在对调用者无额外有效性负担时省略表面标注；但实际操作仍要满足值与布局条件。接收者形式不决定效果：`*self` 不自动表示读写整个对象，按值 self 也不自动表示释放全部可达资源。

### outcomes 是行为上界

没有显式 outcomes 时，编译器从实际操作及已交付契约推导可用摘要。显式 outcomes 限制允许行为，编译器仍须核验实现；不能在遗漏 write 后自动扩大声明使其通过。

效果合取表达允许范围或相应规格条件，不规定执行次序。read 与 kill 同时出现在上界里，不能为"先终结，再读取"提供合法性。

完整调用契约还须保存入口前提、各出口状态、结果依赖、责任转移及保持关系。无序效果集合无法表示这些全部信息。

### 条件与出口

条件摘要依据入口快照选择适用行为。条件未证真不等于已证假；当条件未知时，编译器保留所有可行路径的效果，只保留它们共同保证的性质。

成功、错误、无结果等出口分别检查。容器扩容失败是否保留旧分配，是该操作的契约；不能从错误类型或函数名字推出强失败保证。

### 规格的所属层

MemoryEffect predicate 构造程序行为规格，不执行它所描述的内存动作。返回 MemoryEffect 的 predicate 中，裸 id 参数是 `quote id: lift Any` 的简写；普通 predicate 默认返回 bool，不自动采用这种 quote 参数解释。程序状态、路径、区域和内存效果由独立的程序验证体系处理，即使其表达式出现在限定类型或子句中，也不成为 PTC 的类型原子命题。

## 常见示例

### 同时声明读取与写入

~~~flurry
predicate modify(quote target: lift Any) -> MemoryEffect {
    read<target> and write<target>
}

fn set_if_zero(target: *i32, value: i32)
where outcomes modify(target)
{
    if target.* == 0 {
        target.* = value;
    }
}
~~~

modify 描述目标允许被读取和修改。它不隐含 kill；对仍存活的 i32 槽位写入后，其他合法指针会读到新值。

### 显式上界不能掩盖实际写入

~~~flurry
-- 拒绝：实际行为不被只读上界允许。
fn wrong_read_only(target: *i32)
where outcomes read<target>
{
    target.* = 0;
}
~~~

### 读取指针表示与读取目标不同

~~~flurry
struct PointerBox {
    pointer: *i32,

    fn pointer_value(*self) -> *i32
    where outcomes read<self.pointer>
    {
        self.pointer.*
    }
}

test {
    let number = 6i32;
    let holder = PointerBox { pointer: number.ref }
    let pointer = holder.pointer_value();
    println("{}", pointer.*);
}
~~~

pointer_value 读取存放指针的字段，结果继续携带 number 的依赖；输出时的解引用才读取 i32 目标。因此字段读取上界不能抹去返回指针的依赖关系。

### 清理效果属于函数行为

~~~flurry
struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

fn quiet_body() {
    let value = Trace { id: 7 }
}
~~~

quiet_body 的完整行为包含输出。只扫描显式函数调用而漏掉清理，会错误地把它当成无外部效果。

## 与 Rust / Zig / C / C++ 的关键差异

这些语言的普通函数签名没有与整套六种效果及状态关系对等的通用入口。Rust 的引用和生命周期、C 的 restrict、C++ 的资源包装以及 Zig 的显式 allocator 参数，分别表达部分访问或责任约束，不能单独代替完整关系契约。

编译器内部也会区分内存读取、写入等效果；LLVM 的相关属性是后端优化契约，不能只按名称与源语言效果互换。[LLVM 函数属性](https://llvm.org/docs/LangRef.html#function-attributes)

## 运行时实现设计引导

效果描述不要求每次 read/write 都进入运行时日志。静态已验证的行为直接执行；需要动态保持的条件由具体检查或协议承担。

受检调用在取得前置依据后执行，并按实际出口交付后置状态。系统动作和 allocator 行为不能因为没有最终返回值或发生在私有实现中而自动删除。

## 编译器实现引导

编译器为每个操作应用顺序转移，再生成边界摘要：

~~~text
State = 位置状态、责任、区域与世代、依赖、内容事实、有效环境
Contract = 入口要求、可能效果、各出口状态关系、结果要求、剩余责任
~~~

1. 应用操作前提，区分不成立与未知。
2. 记录真实读取、写入、终结和外界动作。
3. 更新状态、内容版本及反向依赖。
4. 纳入所有隐式清理和被调用契约。
5. 在边界隐藏已证明不可观察的私有动作。
6. 核验可见行为被显式上界允许，交付其余关系。

分支合成取可能影响的并集、共同保证的交集，并保留预算内可表达的条件关联。循环和递归采用有预算的不动点；超限返回未知或保守摘要，不默认为无效果。
