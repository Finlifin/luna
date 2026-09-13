# 模板宏

## Intro

模板宏把一段 token 替换为另一段 token。参数捕获的是源码片段；替换本身不执行其中的表达式。

读懂一个模板宏，可以先把参数代入替换体，再按展开后的程序理解它。需要按语法结构处理输入时，使用[构造宏](./construction-macro.md)。

## 语法

以下是模板宏的局部表面语法。`token*` 表示源码 token 序列，不是 Flurry 表达式列表。

```text
template_definition -> define id integer { token* }
    | define id integer ... { token* }
    | define id(id*) { token* }
fixed_template_call -> $id token*
repeated_template_call -> $id token* $
named_template_call -> $id(token*)
template_parameter -> $integer
    | $id
```

固定捕获调用中的 token 数由定义指定。重复捕获调用的末尾 `$` 是结束标记。命名参数形式以圆括号包围实参；参数名在替换体中以 `$name` 引用。

宏的替换体可以包含普通语言的语法片段。替换后的字段、参数和语句分别遵循[全局语法](../grammar/syntax)及[分隔符规则](../grammar/index.md#分隔符)。

## 正式定义

### 可见性与捕获对象

模板宏只在当前文件有效，按源码顺序先定义后使用。调用显式写出 `$` 前缀。

捕获和替换以 token 为单位，不根据类型或语法树决定参数含义。一个输入片段是否是合法表达式、声明或模式，由它展开后的使用位置检查。

### 固定个数

`define name N` 捕获调用名后紧随的 N 个 token。替换体中的 `$0` 到 `$N-1` 分别引用这些 token，编号从 0 开始。

同一参数可以在替换体中出现多次；每次都插入相应 token。若展开后的程序因而多次执行某个表达式，其行为按展开后的程序决定，不能将 token 替换解释成先求值一次再复用结果。

### 按组重复

`define name N ...` 将调用中结束标记 `$` 之前的输入按每 N 个 token 一组处理。每组独立代入同一个替换体，展开结果按输入组的顺序连接。

### 命名参数

命名参数捕获对应的 token 序列，以 `$name` 代入。一个命名参数可以接收多个 token；它与固定模式中一个编号对应一个 token 的规则不同。

### 展开结果

展开结果接回调用位置继续接受语言检查。宏可以用于生成完整声明，也可以生成周围语法所需的片段；它不豁免展开结果的名称、类型或控制流规则。

## 常见示例

### 两个 token 生成一个函数

```flurry
define constant_function 2 {
    fn $0() -> i32 {
        $1
    }
}

$constant_function one 1
$constant_function two 2
```

每次调用捕获函数名和一个字面量 token。替换后分别得到返回 1 与 2 的函数。

### 按组生成常量

```flurry
define constants 2 ... {
    const $0 = $1;
}

$constants FIRST 1 SECOND 2 THIRD 3 $
```

三组输入分别生成三个常量声明；分号来自替换体。

### 捕获一段 token

```flurry
define print_value(value) {
    println("{}", $value);
}

test {
    $print_value(1 + 2)
}
```

替换体把整段 `1 + 2` 放入调用参数位置。

## 与 Rust / Zig / C / C++ 的关键差异

| 对照语言 | 对应机制与区别 |
|---|---|
| Rust | `macro_rules!` 的捕获可以声明 `expr`、`pat` 等语法片段种类；Flurry 模板宏捕获 token，结构化输入由构造宏处理。[Rust 声明宏](https://doc.rust-lang.org/reference/macros-by-example.html) |
| Zig | 无对等的模板 token 替换构造；计算与参数化生成可使用 comptime。Flurry 模板宏还可以生成尚未解析的源码片段。[Zig comptime](https://ziglang.org/documentation/master/#comptime) |
| C | 函数式预处理宏用形参替换对应实参 token；Flurry 还提供固定 token 数及按组重复的捕获形式，且调用使用显式 `$`。[C 预处理宏参数](https://gcc.gnu.org/onlinedocs/cpp/Macro-Arguments.html) |
| C++ | `#define` 使用预处理宏替换；Flurry 的模板宏可见性限定在当前文件的源码顺序内。[C++ 宏替换](https://eel.is/c++draft/cpp.replace) |

## 运行时实现设计引导

模板宏本身没有 runtime 调用。实现应按展开后的程序执行：

1. 取得已经完成宏展开与语言检查的代码。
2. 对重复生成的表达式、声明和控制流应用普通运行时语义。
3. 仅在普通优化规则允许时合并重复计算；不能因为几段代码来自同一个宏参数就认定其结果可复用。

不变量：捕获 token 不捕获表达式的运行时结果。运行时成本由展开代码决定，不由宏调用看起来有几个参数决定。

## 编译器实现引导

### 词法展开接口

模板展开接入词法阶段，对源码按顺序维护文件内宏环境。

1. 遇到定义，记录捕获形式、参数描述和替换体 token。
2. 遇到调用，使用调用前已建立的宏环境解析宏名。
3. 固定形式取得规定数量的 token；重复形式收集到结束 `$` 并按组切分；命名形式建立参数到 token 序列的映射。
4. 按替换体顺序插入普通 token 与参数 token，重复形式逐组连接结果。
5. 将展开结果交回前端，并保留调用位置、定义位置和参数来源，供后续诊断使用。

参数引用必须落在捕获描述内。输入不足、不能完成分组或缺少所需结束标记时，不能以未初始化参数继续展开。

### 替换算法

前置条件：当前调用的参数捕获已完成；替换体中的每个参数引用都有对应输入。

```text
substitute(body, captured):
    output = empty_token_buffer()
    for part in body:
        if part is a parameter_reference:
            append(output, captured[part.parameter])
        else:
            append(output, part)
    return output
```

算法不求值、不按类型改写插入片段。单次替换的时间由替换体遍历和实际输出长度决定；输出 E 个 token 至少需要 O(E) 的物化成本。重复与后续展开都应计入总输出，不能只按原始输入长度声称整个宏处理线性。

实现应为展开过程保留资源计数与可诊断的失败路径。遇到资源耗尽，报告展开链中的相关位置；不能把截断的 token 序列当作成功结果提交。
