# 契约、推导与验证

## Intro

一个安全调用需要知道入口条件、调用会做什么，以及各条返回路径留下什么。调用方使用这份契约，不必每次展开实现。

常见下标、字段分离和资源交付可以自动检查。复杂关系由显式规格和证明补足；能够合法运行检查的条件，也可以在使用之前受检兑现。

## 语法

~~~text
verification_clause -> requires expr
    | ensures expr
    | decreases expr
outcome_clause -> outcomes expr
assumes_statement -> assumes expr
program_predicate -> predicate id(parameter*) (-> expr)? clauses? block
verified_function -> verified fn id(parameter*) (-> result)? clauses? block
~~~

`parameter`、`result`、`clauses` 同全局语法；程序 predicate 不标 comptime。

## 正式定义

### 契约的四个部分

| 部分 | 调用方需要取得的内容 |
|---|---|
| 入口要求 | 本次访问、运算、环境及责任转移的前提 |
| 行为上界 | 读取、修改、终结和外部可观察的范围 |
| 出口关系 | 正常及其他出口中仍成立的状态、资源与保持关系 |
| 结果交付 | 返回值及外部保留值携带的依赖、初始化和责任 |

效果上界不能代替其余三项。两个未知回调的函数类型相同，也不意味着它们对旧缓冲区具有同样的保持保证。

### 默认推导的范围

普通操作按实际状态变化推导，不要求为每次 Copy、move 或字段写入手写标注。显式契约仍核验实现；没有主体的外部声明必须交付足够的已核验或明确可信契约。

safe 自动算术限制在总次数不超过三次的多元多项式片段，并受展开项数和步骤预算约束。总次数是单项式中指数之和：三因子乘积是三次，两个平方的乘积是四次。这一限制的目的是保证自动证明的可预测性——三次以内的多项式不等式有成熟的判定算法，超出此范围则需要用户显式提供证明策略。

该限制针对自动证明，不限制程序本身能计算什么。引入任意多的中间等式也不能绕过分析预算，不能据此宣称所有高次关系都被自动证明。

普通机器运算仍须在执行前满足溢出、除零和范围前提。把可能溢出的乘法先执行再检查结果大小，不能证明原乘法合法。

### assumes 与运行检查

assumes 提供当前位置需要的条件。若已经静态证明，编译器可以消除冗余检查；若静态未知且有合法判定器，编译器在依赖该条件的动作之前保留运行检查。

检查只能发布实际建立的事实。它不能创造所有权、恢复已移出的值，或使旧世代指针复活。失败路径不得继续使用成功事实；检查失败也不等于数学谓词为 false 的普通返回值。

状态事实绑定其来源和版本。检查后若目标被修改、终结或环境改变，事实须按实际依赖失效。并发或可重入环境中，一次瞬时观察还需持续保持协议。

### verified 与调用边界

safe 按被调用函数的声明契约检查自己的操作，不因调用 verified 而改换模式。verified 实现需要证明兑现该契约；依赖它的程序只有在相关证明依赖成立后才能交付。

基于 outcome separation logic 的验证可以处理更强的状态和资源关系。调用方可以按前提使用已经证明的整体契约，不必重新展开完整证明。

复杂高阶关系优先通过用户给出的 assumption 和显式契约表达。safe 不负责把任意回调的全局行为自动提升成一套最强接口。

### 动态证据的边界

运行时验证可以判定范围、世代，或通过租约和监控保持某种关系。然而，前后内容相同不能证明期间从未写入；超时不能证明任意回调总会终止。

没有实际判定或保持设施的性质，需要静态契约、verified 证明或明确的可信实现边界。裸声明不成为优化器无条件相信的事实。

## 常见示例

### 普通下标检查

~~~flurry
predicate valid_index(index: usize, length: usize) {
    index < length
}

fn read_at(values: Array<i32, n>, index: usize) -> i32
where n: usize
{
    assumes valid_index(index, n);
    values[index]
}
~~~

调用中 n 已知不意味着 index 已知。需要运行检查时，检查发生在元素访问之前。

### 三维寻址先证明机器运算合法

~~~flurry
fn read_3d(
    values: Array<i32, n>,
    width: usize,
    height: usize,
    depth: usize,
    x: usize,
    y: usize,
    z: usize,
) -> i32
where n: usize
{
    assumes width > 0usize and height > 0usize and depth > 0usize;
    assumes x < width and y < height and z < depth;
    assumes height <= n / width;
    let plane = width * height;
    assumes depth <= n / plane;
    let row = z * height + y;
    let offset = row * width + x;
    values[offset]
}
~~~

在数学整数中，目标关系是 `((z * height + y) * width + x) < depth * height * width <= n`，总次数最高三次。程序先检查正维度和容量，再执行各步运算，从而使中间值也处于可表示范围。

如果预算内未能完成某个组合证明，编译器应报告尚缺的具体前提，不能因为整体式子为三次就声称必然可解。

### 高次判定可以有安全的执行算法

~~~flurry
predicate four_dimensions_fit(a: usize, b: usize, c: usize, d: usize, limit: usize) {
    if a == 0usize or b == 0usize or c == 0usize or d == 0usize {
        true
    } else {
        if b > limit / a {
            false
        } else {
            let ab = a * b;
            if c > limit / ab {
                false
            } else {
                let abc = ab * c;
                d <= limit / abc
            }
        }
    }
}

fn check_dimensions(a: usize, b: usize, c: usize, d: usize, limit: usize) {
    assumes four_dimensions_fit(a, b, c, d, limit);
}
~~~

这个算法判定数学乘积是否不大于 limit，但不先计算可能溢出的四连乘。判定算法与整体数学规格的对应关系须由合法证明支撑；调用 safe 不需要重新展开全部高次推理。

零维路径判定为 true，也不意味着任意重排的机器连乘都安全；某些顺序会在乘到零之前溢出。

### safe 使用 verified 的结果契约

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

调用方兑现入口界限；被调方证明结果关系。runtime 参数并不因为 verified 实例化就成为已知常量。

## 与 Rust / Zig / C / C++ 的关键差异

C/C++ 的运行时断言和库契约、Rust 的受检 API 与 unsafe 封装、Zig 的运行时安全检查，都是处理部分前提的实际手段；它们不与这里按状态版本交付事实、在 safe 与 verified 间传递证明依赖的完整协议一一对应。

Flurry 的 assumes 在静态未知时依赖合法运行检查，不能照搬后端的无条件假设指令来实现它。[LLVM assume intrinsic](https://llvm.org/docs/LangRef.html#llvm-assume-intrinsic)

## 运行时实现设计引导

一次受检条件按以下次序执行：先建立合法观察资格，再运行判定器，随后仅在成功路径发布事实并执行受保护动作。失败路径按真实控制流处理已有临时与清理责任。

带状态的动态协议应同时说明取得、保持和释放：租约何时阻止冲突动作，取消或错误出口怎样撤销登记，外部终结是否能与使用并发。单个 bool 结果不代替这些协议。

## 编译器实现引导

### 两种检查载体

safe 的全部检查位于 normalized IR；verified 检查位于 instantiated IR。前者必须保留责任、语义清理、调用契约和环境，不能把未解决的 safe 义务推迟到以后某个实例恰好可证。

PTC 提供类型、布局和已选实现依据。程序验证层独立维护内存状态、路径条件、算术、证明预算和运行检查，不把这些嵌入层规格改造成类型原子。

### 有预算的传播

~~~text
对每个控制流块：
  合并前驱中的可能效果与共同保证
  顺序应用操作及清理契约
  维护区域身份、内容版本、责任与依赖
  尝试预算内的区间、分离及三次以内算术推导
  对未知前提选择显式受检依据、已证明契约或报告缺口
  把各出口状态交给后继
~~~

循环和递归采用有上限的不动点与保守汇总。Proved、Refuted、Unknown 必须区分；预算耗尽不是真，也不是具体反例。

### 证明依赖与失效

使用 verified 契约时，编译器记录实例证明依赖；只有入口已兑现且依赖证明成立，才能交付程序。事实还记录生成它的检查、状态版本和有效范围。

优化不得先借一次检查建立依据，再删除仍是该依据唯一来源的检查路径。调用或写入可能破坏的事实须在后续使用前失效或重新建立。

高阶接口缺少必要关系时，编译器诊断指出具体的逃逸、旧区域保持或重复责任义务。对复杂回调优先要求显式契约，不无限制地生成并求解高阶条件网络。
