# 局部依赖与暂停

## Intro

检查 lambda 时，编译器可能还不知道它的参数类型。此时应让能提供类型的其他工作继续，而不是把整个调用连同剩余实参一起停住。

局部调度负责保存这种未完成工作，并在相关信息变化后重试。它不能替类型规则决定一个程序是否合法。

## 语法

```text
application -> expr(argument*)
lambda -> |anchored_parameter*| (-> expr)? expr
```

Goal、wait ticket、region 和 ready queue 是实现概念，源码不因此增加等待表达式或隐式异步调用。

## 正式定义

程序取得相同的合法推断信息时，检查工作的调度次序不能改变最终类型、实现选择或生成程序的行为。临时缺少输入不等于输入不存在，也不等于义务已经成立。

普通 runtime 表达式的求值不因类型检查暂停而发生。编译器恢复检查也不能重复源程序要求只执行一次的 comptime 工作。

等待必须有可解释的对象。若缺少的不是待产生的信息，而是操作根本不受支持、求解预算已经耗尽或必需前提无法证明，应按实际原因处理。

定义端允许保留的残余计算与等待队列不同。前者按[冻结规则](staging-and-freezing.md)交付；当前会话中的未决推断不能仅因队列为空就变成成功结果。

## 常见示例

### 挂起子工作，让信息生产者继续

```flurry
use std.prelude.*;

fn apply_first(function: fn(T) -> U, value: T) -> U
where T, U
{
    function(value)
}

test {
    println("{}", apply_first(|item| item + 1, 41i32));
}
```

一条可行的检查路径如下；这张表不规定唯一执行顺序。

| 步骤 | 工作 | 结果 |
|---|---|---|
| 1 | 为本次调用建立 T、U 的新鲜推断位置 | 签名关系可用 |
| 2 | 检查 lambda，需要输入类型 | 保存这部分工作的进度 |
| 3 | 检查 41i32 | T 被约束为 i32 |
| 4 | 再次观察 lambda 输入 | 继续检查并得到 i32 结果 |
| 5 | 完成调用 | 输出 42 的 runtime 程序 |

若第 2 步捕获整个父调用且不再安排第 3 步，这个实现会制造自己的死锁。应暂停需要等待的子工作，同时保留父调用继续安排兄弟工作的能力。

### 生产者失败不能留下永久等待

接续 apply_first，以下是拒绝例：

```flurry
fn missing_input() -> i32 {
    apply_first(|item| item + 1, unknown_value)
}
```

unknown_value 没有绑定。相关输入检查失败后，lambda 的等待应结束或关联到该失败；诊断不能只有“lambda 类型未知”，更不能无限等待一个不存在的生产者。

### 不必等待无关的完整结果

```flurry
struct Link {
    value: u32,
    next: ?*Link,
}

const POINTER_BYTES = Type.size_of(*Link);
```

指针大小不要求先取得 Link 的完整按值布局。观察“这个值是指针”与取得 pointee 的全部表示不是同一个依赖，避免多余等待是正确依赖建模的一部分。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 编译器用局部推断状态管理未知类型，也用 query 组织按需计算。本章进一步规定 Flurry 的局部暂停不能与持久缓存混为一份状态。[Rust 类型推断](https://rustc-dev-guide.rust-lang.org/type-inference.html)、[Rust queries](https://rustc-dev-guide.rust-lang.org/query.html)

局部调度没有直接的 C、C++ 或 Zig 源语法对应物；程序员使用类型标注、泛型实参或 comptime 输入提供信息。编译器内部如何安排这项工作不应变成另一个用户可见的等待协议。

## 运行时实现设计引导

编译器局部暂停不是源程序的代数效应请求。编译器可以用状态机或一次性 continuation 实现等待，但这些控制对象不会因实现选择而成为目标程序中的隐藏任务。

生成程序仍按其自身调用、闭包、效应和清理契约执行。编译器采用多个工作线程也不自动使源程序并行。

## 编译器实现引导

### 工作与观察

区域保存本次推断变量、可见事实、候选状态及局部任务。已有语法和签名字段直接读取；能沿递归路径完成的计算直接返回。仅在工作需要独立保存进度时建立 goal。

```text
observe(request, context):
    inspect facts whose premises are valid in context
    perform locally justified progress
    if established: return Known(result, evidence)
    if a required input has a pending producer:
        return Blocked(input_identity, observed_generation)
    return domain_specific_unresolved_or_failure
```

Known 只说明所请求的观察已有依据，不表示整个类型或任务全部完成。Blocked 票据描述再次尝试的触发条件，不承诺一次唤醒就一定能完成。

### 无遗漏的等待登记

1. 观察时保存依赖的身份和版本。
2. 登记 watcher，再核对版本；若已变化，立即将工作入队。
3. 对同一个有效任务代次去重，避免多个依赖重复恢复同一 continuation。
4. 运行时重新观察前提，而不是盲目使用唤醒前缓存的结果。
5. 依赖合并、更换代表元或失败时，将变化传给原观察者。
6. 任务结束、回退或取消时清理其登记；旧票据不能唤醒新任务的同号槽位。

单线程也可能在观察与登记间调用其他逻辑，因此不能只依靠“没有并行写入”排除遗漏。多线程实现还须为登记、版本读取和发布提供相应同步。

### 提交与失败

```text
publish(update):
    validate the complete update
    commit facts and their premises
    collect affected observations
    enqueue work without re-entering the partially updated solver

finish(region):
    drain runnable work
    classify every required unfinished obligation
    propagate failed producers and diagnose actual cycles
    export only language-permitted residual work
    reject remaining required unknowns
    end or cancel all local tasks before returning
```

Union-find 合并必须迁移或等价转发观察关系。任务依赖多个输入时，要保留准确的重新观察条件；粗粒度唤醒可以使用，但不能把无效唤醒当作进展。

### 验收与成本

覆盖兄弟参数供给、登记间更新、代表元合并、重复唤醒、生产者失败、取消、候选回退和合法残余交付。用不同合法出队顺序验证语义结果一致；资源预算单独报告，不能伪装为类型反证。

调度成本按实际任务数、登记数和唤醒次数计。对动态创建任务、反复观察的系统，不能把固定有限 DAG 的 O(V + E) 遍历成本当作整个 elaboration 的复杂度。
