# 效应限定类型

## Intro

效应限定描述计算可以请求哪些操作，以及这些请求对应的处理层。这一机制源自代数效应（algebraic effects）的研究：将副作用建模为可被外部处理器拦截和解释的请求，从而在保持纯函数式推理的同时支持受控的副作用。效应限定记录允许行为，不逐次统计执行轨迹。

同一个操作可以调用多次并使用同一处理器；嵌套处理器则可能需要同名标签的多个层次。

## 语法

~~~text
effect_type -> #expr expr
effect_handle -> expr# { case_arm* }
perform_effect -> expr#
~~~

独立 effect 声明与操作签名沿用全局语法。effect enum 的 E 引用整组独立操作，E.v 引用其中一项；各项采用操作签名。Comp(E,A) 是内部记号，表示允许行 E 下产生 A 的计算。

## 正式定义

效应标签按完整独立操作实例区分。分组用于组织操作，不把整组自动变成一个不可分割标签。相同声明但不同参数实例的身份按参数关系判断。

行采用 scoped labels：不同标签在已证明不等时交换，同标签 occurrence 保序。这一设计来源于行类型效应系统中对多层 handler 的需求——传统集合行无法区分同一效应的多层嵌套处理，而 scoped labels 通过保持 occurrence 顺序解决了这一问题。#a #a T 保留两层 a；空效应限定消去。

同类前缀按行连接保留层次。顺序组合计算则在共同允许行下检查，不按调用次数连接行。这是两个不同操作。

handler 消去选定层，并将分支与剩余计算检查到输出环境。分支可以向外请求同种效应，前提是输出环境有对应路由。部分覆盖需要未覆盖请求的向外路径，不能宣称整个标签已经消失。

操作的入口类别还可能要求捕获、异步处理或安装时直接传值。这些属性与结果交付共同组成 ABI 契约，不能只保留一个表面返回类型。完整规则见[代数效应主题](../../algebraic-effect-and-delimited-continuation/index.md)。

## 常见示例

声明一个请求数值的操作。主体只决定如何使用结果，handler 决定每次请求返回什么：

~~~flurry
effect read_number() -> u32;

fn sum_two_requests() -> #[read_number] u32 {
    let left = read_number()#;
    let right = read_number()#;
    left + right
}

test {
    let total = sum_two_requests()# {
        read_number() => 7,
    }
    println("total: {}", total);
}
~~~

输出为 `total: 14`。两次请求都进入同一个分支；函数的允许行只需要一层 `read_number`。

再给内层处理分支一个向外请求的用途：从外层取得数值，除以二以后交回主体：

~~~flurry
fn sum_with_halved_input() -> #[read_number] u32 {
    sum_two_requests()# {
        read_number() => {
            let outside = read_number()#;
            outside / 2
        },
    }
}

test {
    let total = sum_with_halved_input()# {
        read_number() => 10,
    }
    println("total: {}", total);
}
~~~

输出为 `total: 10`。每次内层请求得到 `5`，主体相加得到 `10`。内层分支中的请求由外层提供 `10`，不会重新进入自己的除二分支。

两个例子把次数与层次分开：第一段执行两次请求，使用一层；第二段每次请求经过内外两个入口。内层处理结束后，`sum_with_halved_input` 仍要求外层 `read_number` 环境。

## 与 Rust / Zig / C / C++ 的关键差异

这些语言没有与 scoped effect 行及其 handler 规则直接对应的通用构造。实际替代包括回调、显式上下文参数、错误返回和异常；它们并不同时提供本篇的类型层次与恢复契约。实现类比可参考行类型效应编译研究，但不据此引入其他语言的完整机制。[行类型效应编译](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/12/algeff.pdf)

## 运行时实现设计引导

直接返回的效应可以降为普通调用与返回。当计算需要恢复能力时，编译器应采用[handler 契约](../handlers.md)所规定的边界，不因“代数效应”这个名称无条件复制或外带栈片段。

运行时入口映射保留每个 occurrence。分支发出的同名请求不能错误回到自身。

## 编译器实现引导

编译器将允许行、操作签名和路由证据共同附着在计算上。实例化开放行时同步替换证据，不能只修改类型文本。

编译器在组合时先决定共同环境，再验证子计算嵌入；在处理时拆出选定层并验证分支输出。所有规范化都需保留同标签次序，而未知标签关系保持残余。
