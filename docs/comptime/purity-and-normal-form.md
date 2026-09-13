# Comptime 函数、纯性与范式

## Intro

编译时函数用普通的计算步骤产生编译器需要的结果。结果可以是数值，也可以是类型；它何时能被算出，取决于输入何时具备。

`pure` 表达的是结果对输入的依赖承诺。它不改变函数的执行阶段，也不保证计算一定终止。

## 语法

以下复用[全局语法](../grammar/syntax)中的 `parameter`、`result`、`clauses`、`expr` 和 `block`。

```text
comptime_function -> pure? comptime fn id(parameter*) (-> result)? clauses? (block | (= expr))
normal_form_head_def -> pure comptime fn id<parameter*> (-> result)? clauses? (block | (= expr))
normal_form_head_application -> expr<argument*>
comptime_parameter -> comptime parameter
optional_decl_clause -> .id : expr = expr
defer_expr -> defer expr
```

普通函数的参数可以单独标为 comptime；这不等于将整个函数声明为 comptime。

## 正式定义

### Comptime 函数与 comptime 参数

comptime 函数在编译时执行。函数中的运算必须在相应阶段可用；不能通过给 runtime 函数传入常量，绕过函数本身的阶段要求。

普通 runtime 函数可以接收 comptime 参数。具体输入用于实例化；运行时调用相应实例时不再传递这些参数，其余参数与函数体中的 runtime 工作保留。

comptime 函数的定义采用 loose 检查强度，但仍经过 normalization；其输入确定后的调用完成相应 instantiation 与执行。

### Pure 承诺

`pure comptime fn` 的 pure 是开发者提供的可信承诺，编译器不验证函数实现是否确实纯。使用者必须保证参与结果复用的输入足以决定对应结果，不能把未表达的可变环境依赖隐藏在 pure 承诺后面。

这一承诺与 PTC 的逻辑命题不同。编译器信任函数的纯性标记，不等于允许用户向 PTC 添加任意逻辑公理。

comptime 函数并不普遍要求 pure、终止或全定义。编译时计算可以产生程序化诊断；这些操作是否适合复用，不能仅由“发生于编译时”推断。

### 范式头与子句参数

范式头用编译时输入确定一个结果。类型也可以作为输入或结果；返回类型的定义使用类型命名风格。

函数、类型及相关定义中的参数化内容以子句参数表达。编译器在定义端依据其约束检查操作，在实例化端代入具体输入。它不是把函数体先复制出来、再完全放弃定义端检查的许可。

范式头的尖括号调用采用真实表达式参数，不能把它当作纯词法替换。

### 可选子句参数默认值

可选子句参数的默认值在 normalization 中直接执行，不要求该表达式纯。需要延迟时由 `defer` 包裹；不能仅因默认值依赖未知输入而静默改变这条规则。

调用点元数据通过独立的 implicit 参数机制传入，不用可选子句默认值代替。默认表达式的合法名称仍由参数与作用域规则决定；defer 不创造可见性。

### 资源失败

允许一般编译时计算，不意味着保证其终止。工具链可以对 interpretation 和 execution 使用不同预算；必需求值因预算耗尽而失败时，编译不能依靠尚未得到的结果继续生成实例。

## 常见示例

### 返回一个编译时值

```flurry
pure comptime fn square(value: Integer) -> Integer {
    value * value
}

const AREA = square(12);
```

`AREA` 的初始化计算得到 144。pure 描述调用的依赖承诺；乘法和函数调用仍需实际完成。

### 返回一个类型

```flurry
pure comptime fn Identity<T: Type> -> Type {
    T
}

typealias Element = Identity<i32>;
```

这里的输入和结果都是类型，不需要先构造一个 i32 值。

### 普通函数接收编译时参数

```flurry
fn choose(comptime left_side: bool, left: i32, right: i32) -> i32 {
    inline if left_side {
        left
    } else {
        right
    }
}
```

生成的实例保留普通参数与返回值。comptime 参数决定分支，不把函数整体转成编译时函数。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与差异 |
|---|---|
| Rust | `const fn` 的函数体受常量表达式规则限制；Flurry comptime 的 pure 是另外的可信承诺。[Rust const fn](https://doc.rust-lang.org/reference/const_eval.html#const-functions) |
| Zig | 同一普通函数可由 comptime 调用解释执行；Flurry 显式区分 comptime 函数与 runtime 函数。[Zig 编译时表达式](https://ziglang.org/documentation/master/#Compile-Time-Expressions) |
| C11 | 无对等的返回类型的编译时函数；实际可用 typedef、宏或外部生成器组织类型。[C11 草案 §6.7.8、§6.10](https://www.open-std.org/jtc1/sc22/wg14/www/docs/n1570.pdf) |
| C++ | 类型计算可用别名模板及模板中的类型成员；Flurry 的编译时函数可以返回 Type 值。[C++ 别名模板](https://eel.is/c++draft/temp.alias) |

## 运行时实现设计引导

1. 对普通函数的具体实例，从运行时参数表中去掉 comptime 输入。
2. 保留剩余参数的传递、返回值与必要的 runtime 操作。
3. 若编译时函数结果被运行时使用，按目标类型的 [Splice 规则](./quoting-and-splicing.md)处理，而不是默认任意结果都能直接放入常量区。

不变量：运行时不调用 comptime engine 来补算已经要求确定的子句参数。函数的运行时代价来自实例，不由 pure 标记直接给出。

## 编译器实现引导

### 函数执行

1. normalization 检查函数签名、可确定的表达式及相应契约，保留对未知输入的依赖。
2. 独立 comptime 函数降为 engine 字节码；将输入、返回值和诊断信息连接到 query 图。
3. 调用时绑定 comptime 输入，执行字节码，取得结果或失败。
4. 对普通 runtime 函数，绑定 comptime 参数后输出其 runtime 实例，不执行仍需 runtime 参数的主体。

两种执行载体及其语义一致性见[两阶段检查](./normalization-and-instantiation.md#两种执行载体)。

### 结果复用与 Eq/Hash

参与 query 缓存的编译时值，其类型需要 comptime 相的 Eq 和 Hash。缓存设计至少须满足：

1. 查询身份和输入依赖足以区分所缓存的计算。
2. Hash 只定位可能命中的条目；发生碰撞时继续按 Eq 判断。
3. 输入相等必须与被复用结果的含义兼容，不能复用一个实际上能区分这两个输入的计算。
4. 形成键时执行 Eq/Hash 不能递归要求同一个尚未形成的键；实现应给它们提供相应的特殊处理入口。
5. pure 承诺不免除诊断、依赖失效与输入绑定的正确管理。

这是一组实现不变量，不规定所有 query 都使用同一物理键格式。包含不纯执行的 query 不能仅凭其函数结果可比较就整体缓存；否则可能丢失必要的求值或诊断。

### 预算与失败路径

执行器至少区分输入未具备、程序化编译错误和资源耗尽。前者可以依合法的延迟边界等待；后两者不能作为成功结果进入缓存。结果处理与诊断恢复不得把一次失败变成 runtime 回退。

字节码执行代价由实际执行步骤决定；存在一般递归时不能给出统一的有限上界。Eq/Hash 的执行代价也要计入预算，不能假设键形成总是常数时间。
