# 单元、null 与其它构造表达式

下列记号常与字面量一起出现在检索中，但各自有独立含义。字面量本身见 [字面量](./literals.md)。

## `()`

`void` 的唯一值。见 [基本数据类型](./primitives.md)。

```flurry
fn ignore_result() {
    do_side_effect();
    ();
}
```

## `null`

构造「没有值」的可选类型 `?T`。

```flurry
let maybe: ?i32 = null;
```

与 `?` 传播、模式中的 null 分支等，见选项类型与模式匹配相关章节。

## `_`

模式与部分位置中的通配 / 忽略。见 [模式匹配](../control-flow/pattern-matching/index.md)。

## `undefined`

表示未初始化或未定义占位；可用上下文见安全与类型相关章节。

## 语法

```text
unit_expr        -> ()
null_construct   -> null
undefined_expr   -> undefined
wildcard         -> _
```
