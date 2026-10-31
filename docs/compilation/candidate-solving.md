# 候选求解与提交

## Intro

找到一个能工作的实现，还不一定能够结束选择。另一个更精确的实现可能仍在等待类型信息。

编译器需要分别保存每个候选的依据，直到规则允许作出唯一选择。试探过程中得到的临时结论不能提前改变外围程序。

## 语法

```text
impl_trait_def -> comptime? impl expr for expr clauses? { (assoc_decl | definition | statement)* }
requires_clause -> requires expr
trait_bound_decl_clause -> id :- expr
```

完整适用性与结构精度规则见[实现选择](../traits/implementation-resolution.md)。本章说明如何完成这些规则及其与 comptime 的交界。

## 正式定义

### 候选状态

每个候选保留已适用、已排除或仍未决的状态及依据。没有证明 requires，不等于证明 requires 为假；搜索预算耗尽也不证明某类型没有实现某 trait。

选择适用候选中结构精度最高者。更精确候选仍未决时不能提交较宽候选；同精度的竞争也必须完成排除或冲突判断。名称来源先按名称和接收者规则确定，不让不同 trait 的同名成员只凭结构精度竞争。

接口实参仍未知时，各候选持有自己的替换。不能让第一个试探永久赋值公共参数，再用这个赋值排除其他解。

### 可见事实

试探事实属于候选自己的上下文；分支假设还须保留适用前提。只有提交的结果及有效证据可以向外围发布。

失败候选产生的检查错误先作为候选失败理由。其他候选成功时，不把已经放弃的试探错误作为整个程序的错误发布；最终无解时，可以选择有用的候选失败原因解释调用为何不成立。

### 编译时执行的边界

试探只运行可隔离、可安全重放的检查和允许的计算。不可安全重放的 comptime 执行留到选择已经确定之后。

若必须先执行未提交候选的这类操作才能判定其适用性，调用需要显式消歧或可用的契约依据；不能自行尝试外部副作用，也不能无限等待一个不会出现的选择结果。

这不要求所有 comptime 函数都 pure。源程序明确要求的一次普通执行可以提供后续信息；其控制依赖及必要执行次数必须保留。任务恢复从已记录的执行进度继续，不能把重新检查当作重放已发生工作。

### 与 PTC 的交界

PTC 接收类型命题。内存视图、运行时可达关系和程序状态规格交给程序验证层，即使这些规格伴随某项类型或接口交付，也不由此进入 PTC。

PTC 的 Unknown 保留原因。存在真实待产生输入时可以登记等待；预算耗尽、未支持片段和无法构造见证不能伪装成待输入，也不能转换为负命题。

## 常见示例

### 更精确实现决定实际调用

```flurry
use std.prelude.*;

trait Describe {
    fn describe(*self) -> str;
}

struct Sample
where T
{
    value: T,
}

impl Describe for Sample<T>
where T
{
    fn describe(*self) -> str {
        "sample"
    }
}

impl Describe for Sample<i32> {
    fn describe(*self) -> str {
        "signed sample"
    }
}

test {
    let signed = Sample<i32> { value: 7 }
    let flag = Sample<bool> { value: true }
    println("{}", signed.describe());
    println("{}", flag.describe());
}
```

输出 signed sample、sample。若查询主体暂时是 Sample<M>，M 表示本次推断的未知类型，索引必须保留 Sample<i32> 这个兼容候选；不能只取得宽泛实现后立刻提交。

| 后续信息 | 选择依据 |
|---|---|
| M 确定为 i32 | 精确实现适用，按精度选它 |
| M 确定为 bool | 精确实现被排除，宽泛实现适用 |
| M 仍未决且精确实现未排除 | 保留选择义务 |

### 一次明确要求的 comptime 执行

以下为有意失败的独立示例：

```flurry
comptime fn require_enabled(enabled: bool) {
    if not enabled {
        meta.compilation.err("feature is required");
    }
}

comptime {
    require_enabled(false);
}
```

调用是源程序明确要求的执行，其诊断不能因编译器重试其他工作而被丢弃。这与“试探一个后来被放弃的候选，暂存它的检查失败理由”不同。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 推断上下文支持 snapshot 和 rollback，可用于试探类型关系；这类回退管理推断状态，不等于对任意外部执行提供事务。Flurry 还明确限定候选中不可重放的 comptime 工作。[Rust 推断快照](https://rustc-dev-guide.rust-lang.org/type-inference.html#snapshots)

C++ 模板推导及约束也参与适用性判断，Zig 可用 comptime 分支选择代码，C 可显式选择函数。Flurry 的结构精度选择规则及真正差异见[实现选择的对照](../traits/implementation-resolution.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

普通静态调用已经取得 implementation 身份、替换和需要的环境。Runtime 不枚举候选后尝试哪一份 requires 能成立。

环境中保存的 runtime 对象仍按操作契约使用；环境的存在不会允许动态切换已经固定的 implementation 身份。动态接口的实际分派则按其独立协议执行。

## 编译器实现引导

### 检索完整性

候选索引只能减少无关工作，不能遗漏可能在合法替换下适用的实现。未知类型位置须访问兼容的更精细结构键，具体键的构造及深度规则见[结构索引](../traits/implementation-resolution.md#编译器实现引导)。

```text
resolve(goal):
    candidates = retrieve_all_compatible_heads(goal)
    for candidate in candidates:
        state = isolated_trial(parent_context)
        check head, phase, implementation eligibility and requirements
        record applicability, substitution, evidence and blockers
        retain a stable trial result; discard uncommitted mutations
    winner = select_by_confirmed_precision_rules(results)
    if winner exists:
        revalidate the dependencies used by its trial
        commit substitution, evidence and term result together
        enqueue affected work
    else:
        return ambiguity, genuine wait, legal residual or failure
```

### 事务范围

回退同时覆盖类型赋值、候选集合、term hole、事实前提、证明状态、暂存诊断与等待登记。PTC 学习结论可以保留，但其 activation guard 必须继续限制适用范围。

共享的不可变类型模板可以复用；候选不得原地修改它们。试探替换是局部对象，只有提交后才能成为外围类型引用的一部分。

不可重放的调用不能仅靠 snapshot 包裹：外部操作不会因还原一张替换表而消失。对可重试任务，记录已经提交的调用结果和执行进度，重试检查时从合法继续点推进。

### 最终化与诊断

提交之前核对候选世界、接口实例、phase 和依赖前提仍有效。依赖变化后，旧试探成功不自动继续有效。

无法选择时区分名称歧义、实现冲突、未决高精度候选、缺少环境与求解资源耗尽。缓存未决结果时保留其原因和版本；不能将 Unknown 永久记为“不实现”。

验收至少覆盖：宽泛候选先完成、精确候选后适用；失败试探不残留赋值；局部分支事实不泄漏；重复唤醒不重放 comptime 操作；最终失败诊断包含相关来源。索引访问、候选数、求解和执行成本分别统计。
