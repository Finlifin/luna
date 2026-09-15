# 错误行与效应行

## Intro

行描述一个类型中可以出现的错误分支，或一段计算可以使用的效应。开放行还保留一个尚未列出的余部，使函数能够处理自己关心的部分，并把其余部分交还调用方。

错误与效应保留的信息不同。错误行区分结果分支；效应行还区分同一种操作由哪一层处理。因此两者不能共用去重规则。

## 语法

~~~text
error_type -> !expr expr
effect_type -> #expr expr
~~~

行表达式使用已有参数和表达式语法。本篇的 ⟨a | ρ⟩、空行 ∅、Lacks、Split、Embed 均为数学或内部记号，不是开放行的新增源码写法。

## 正式定义

### 标签和开放尾部

错误标签是完整错误类型实例，效应标签是完整独立操作实例。参数参与身份，不能只比较声明名。分组组织多个操作，展开后按各操作实例参与行规则。

⟨a | ρ⟩ 表示一个标签及余行；∅ 表示闭空行。开放尾部不是“默认没有别的标签”。标签含未知参数时，相等与不等都可能需要证明。

### 错误：唯一标签

错误行是有限的唯一标签行。同一错误身份不因经过多个处理器而变成不同结果分支。

严格扩展 ⟨e | ρ⟩ 要求 Lacks(ρ,e)，即余行不含 e。Lacks 是否定命题，不是搜索时暂时没看见。

行的错误合并可以去重相同完整标签；合并与严格扩展不是同一个操作。载荷及参数关系必须按标签身份契约协调。

### 效应：保留同标签层次

效应行允许重复标签。已证明不同的标签可以交换；同标签 occurrence 的相对顺序保留，不能去重或任意置换。

重复表示处理层次，不表示执行次数。连续调用两次同一效应可以使用同一个处理入口。处理器分支向外调用同种效应时，则需要外层 occurrence。

符号标签未证明不同时不能交换。某个参数实例化后使两个标签相等，先前未经依据的排序可能已经改变处理含义。

### 相等、包含与嵌入

行相等保留上述代数。错误行包含可以支持分支嵌入，但实际转换仍需保留标签与载荷。

效应嵌入必须说明每个源 occurrence 对应哪个目标处理入口，并保持已有路由。存在某个嵌入不等于已经确定唯一嵌入；多个可能路由不能由求解器任选。

RowContains 只表达存在性。它不能独自提供 split 结果，也不能提供 handler 路由。Closure 的载荷是依赖环境，不进入行代数。

### 组合与限定归一

组合计算先在共同允许行下检查子计算。开放类型方案可以通过实例化与统一适配共同环境。固定闭行或独立刚性行则需要有依据的嵌入，不能只把不同闭行宣告相等。

同类 effect 前缀按行连接解释，保留重复层次；同类 error 前缀合并唯一错误分支。空限定消去。跨种类限定不由这些规则自动交换。

## 常见示例

先看错误行。两道检查可以返回同一种错误；传播两次不要求返回类型列出两个同名分支：

~~~flurry
enum InputError {
    out_of_range(u32),
}

fn checked_byte(value: u32) -> ![InputError] u32 {
    if value <= 255 {
        value
    } else {
        error InputError.out_of_range(value)
    }
}

fn byte_pair(left: u32, right: u32) -> ![InputError] Tuple<u32, u32> {
    let first = checked_byte(left)!;
    let second = checked_byte(right)!;
    (first, second)
}

test {
    byte_pair(12, 300) match {
        (left, right)! => println("pair: {}, {}", left, right),
        error .out_of_range(value) => println("invalid byte: {}", value),
    }
}
~~~

输出 `invalid byte: 300`。错误身份是 `InputError`，载荷说明哪一个数失败。`byte_pair` 的行不记录错误可能来自第几次调用。

效应也不按调用次数增长，但嵌套处理会区分同名入口：

~~~flurry
effect read_number() -> u32;

fn number_pair() -> #[read_number] Tuple<u32, u32> {
    let first = read_number()#;
    let second = read_number()#;
    (first, second)
}

fn translated_pair() -> #[read_number] Tuple<u32, u32> {
    number_pair()# {
        read_number() => {
            let outside = read_number()#;
            outside / 2
        },
    }
}

test {
    let (left, right) = translated_pair()# {
        read_number() => 10,
    }
    println("pair: {}, {}", left, right);
}
~~~

输出 `pair: 5, 5`。两次主体请求共享内层入口，内层分支中的请求共享外层入口。若把两个同名入口去重，内层分支就无法按这个路由取得外层数值。

同类限定的规则也可以直接写成类型别名。以下代码接续上面的声明：

~~~flurry
typealias OneError = ![InputError] u32;
typealias RepeatedError = ![InputError] ![InputError] u32;
typealias OneLayer = #[read_number] u32;
typealias TwoLayers = #[read_number] #[read_number] u32;

comptime {
    assumes OneError == RepeatedError;
    assumes OneLayer != TwoLayers;
}
~~~

第一项归一时合并同一错误身份；第二项保留同名效应的两层。开放行把这些关系继续带到尚未确定的余部，其内部表示和统一算法见后文；不能用当前已经列出的标签代替对整个余部的证明。

## 与 Rust / Zig / C / C++ 的关键差异

Rust Result 和 Zig error union 是错误结果的实际对应手段；C 使用错误码和显式结果结构，C++ 使用异常或结果类型。scoped effect 行没有这些语言中的直接对应构造，实际替代包括回调、显式上下文及处理协议。其标签层次可以参考 scoped labels 的理论，但 Flurry 的完整实例身份与运行规则由本篇确定。[Rust Result](https://doc.rust-lang.org/std/result/)、[Scoped labels](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/scopedlabels.pdf)

## 运行时实现设计引导

错误值携带当前分支及载荷；嵌入可重编码判别，但保留错误身份和载荷含义。效应计算则需要取得相应处理入口，静态路由可以降低为直接参数或调用入口。

同标签 occurrence 不能在运行时仅按标签名查询。可为静态已解析 occurrence 分配槽位或传递显式证据；优化必须保持源与目标入口映射。

多次调用同一入口不要求复制环境。新增处理层则可能需要不同入口，即使其声明身份相同。

## 编译器实现引导

### 变量与结果

定义端行参数是刚性的；调用推断产生的元变量可以在作用域和阶段允许时赋值。两者必须在内部表示上区分。

Split 返回头签名、余行及选择证据。错误 split 还提供余行 lacks；效应 split 保留被选 occurrence 与跨越不同标签的交换证据。

### Expose 与统一骨架

以下算法针对有限扩展行，标签比较交 PTC。

~~~text
Expose(kind, wanted, row, protected_tail):
    row := current_substitution(row)
    if row is an extension and its head provably equals wanted:
        return head_signature, tail(row), selection_evidence

    if row is an extension and its head provably differs from wanted:
        result := Expose(kind, wanted, tail(row), protected_tail)
        rebuild skipped head and record valid exchange
        return result

    if row is closed empty:
        return conflict

    if row is assignable metavariable beta:
        if beta aliases protected_tail:
            return structural_conflict
        gamma := fresh row metavariable in permitted scope
        Bind(beta, Extend(wanted, gamma))
        if kind is error: require Lacks(gamma, wanted)
        return exposed_head, gamma, extension_evidence

    return residual
~~~

头身份未决时保留残余或由布尔层探索有依据的等式分支，不能直接当作不同。保护尾部按当前替换后的身份比较，防止别名绕过保护。

~~~text
Unify(kind, left, right):
    apply current substitutions
    if same row term: succeed
    if either is assignable metavariable:
        Bind with kind, occurs, scope and lacks checks
    else if left is an extension:
        protect current ultimate tail of left
        expose left head in right using that protection
        unify head signatures
        unify remaining rows
    else if both are empty:
        succeed
    else:
        conflict or residual according to known structure
~~~

Bind 检查 kind、occurs-check、参数逃逸以及已有 lacks。所有替换、标签事实与交换证据参加决策回退；撤销标签不等前提时也撤销依赖它的规范化。

### 终止边界

~~~text
⟨a | α⟩ = ⟨a⟩           gives α = ∅
α = ⟨a | α⟩             has no finite solution
⟨a | α⟩ = ⟨b | α⟩      has no finite solution when a ≠ b
~~~

第三式需要公共尾部保护；单靠普通 occurs-check 可能反复扩展同一尾部而不终止。不能把有限行问题“逼近”为无限行。

两个不同元变量则可有共享余行解：

~~~text
⟨a | α⟩ = ⟨b | β⟩
α = ⟨b | γ⟩
β = ⟨a | γ⟩
~~~

错误行把 a、b 的排除要求传给 γ；效应行不为扩展本身添加 lacks。α、β 若为刚性参数，这只是待证关系，不执行赋值。

### 包含、并合与复杂度

唯一错误行包含检查显式标签并保留开放尾部关系，不能只检查已知头部就宣布成功。effect 一般嵌入返回 occurrence 映射，不由 A17 替代。

闭行可以索引：error 每键唯一，effect 每键保存有序 occurrence 序列。链表式重复 expose 的朴素成本可达 O(n²)，还需加标签等式、签名统一及约束传播成本。

符号标签、任意开放连接和跨理论关系不承诺完整统一。必须报告卡在哪个标签身份、尾部或路由选择，而不是仅显示“行不匹配”。
