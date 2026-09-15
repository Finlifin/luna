# optional 类型

## Intro

?T 表示一个值可能缺失，也可能包含一个 T。每增加一层 optional，就增加一层可观察的缺失位置。

因此，“外层有值、里面为空”和“外层为空”不能混为同一个状态。

## 语法

~~~text
optional_type -> ?expr
option_some_pattern -> pattern?
propagate_option -> expr?
~~~

null、匹配和传播使用既有表达式与控制流规则。本篇定义类型与注入关系，不额外定义传播目的地。

## 正式定义

?T 的存在分支保存完整 T，缺失分支为该层 null。已具有具体类型的值注入外层 optional 时保留整个源值，不递归改写内部缺失状态。

无具体源类型的 null 按目标 optional 构造最外层缺失。有目标上下文时先按目标检查分支；无目标时沿用同型/Any 汇合，不自动合成 optional。

消除一层 optional 只区分该层状态。嵌套 optional 不因主体也为 optional 而自动折叠。

类型语义不承诺统一的 tag 大小，也不保证任意主体都能使用空指针编码。

## 常见示例

先写一个可能找不到结果的配置查询。普通数值按返回目标注入，未找到时返回该层的 `null`：

~~~flurry
fn retry_limit(profile: u32) -> ?u32 {
    profile match {
        0 => 3,
        1 => 0,
        _ => null,
    }
}

fn show_limit(profile: u32) {
    retry_limit(profile) match {
        count? => println("retry limit: {}", count),
        null => println("profile not found"),
    }
}

test {
    show_limit(0);
    show_limit(1);
    show_limit(9);
}
~~~

输出依次为 `retry limit: 3`、`retry limit: 0`、`profile not found`。零是有效配置值，不能用它代替缺失状态。

嵌套 optional 可以表达一次配置修改：外层空表示没有提交修改，外层有值且内层空表示清除限制，两层有值表示设置新限制：

~~~flurry
fn describe_update(update: ??u32) {
    update match {
        inner? => {
            inner match {
                limit? => println("set limit to {}", limit),
                null => println("clear limit"),
            }
        },
        null => println("leave unchanged"),
    }
}

test {
    let cleared: ?u32 = null;
    let configured: ?u32 = 5;

    describe_update(null);
    describe_update(cleared);
    describe_update(configured);
}
~~~

输出依次为 `leave unchanged`、`clear limit`、`set limit to 5`。后两次调用已有具体的 `?u32` 源类型，因此整个源值注入外层；内层的 `null` 不会被改成外层缺失。

## 与 Rust / Zig / C / C++ 的关键差异

Rust Option、C++ optional 与 Zig optional 都提供可选值。C 使用标志、哨兵或指针约定；不能假设这些表示与任意 ?T 等价。Flurry 的嵌套注入保留源值整体。[Rust Option](https://doc.rust-lang.org/std/option/)、[C++ optional](https://eel.is/c++draft/optional)

## 运行时实现设计引导

为每层保留可区分状态。只有在主体合法值域提供足够空闲编码时，才能复用编码空间；优化不得把不同层的缺失合并。

注入既有 optional 时按源表示整体嵌入；禁止把内部 null 当作外层 null 的捷径。

## 编译器实现引导

检查顺序为目标类型、源具体类型、是否已有相同类型、所需外层注入、表示计划。null 的目标定型与一般源值注入分开。

嵌套验收至少区分外层空、外层有值且内层空、两层有值。优化前后逐状态验证，不只验证一个非空输入。
