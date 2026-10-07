# 062 — code-organization — 04

- 日期：2026-09-27
- global_index：062
- theme：code-organization
- subindex：04
- 本轮目标：纠正函数 ABI 的表面形式，同步全局 syntax 与现行正式章节。

## 作者输入

> 没有extern关键字，函数abi使用属性指定如^attrs.abi(.c)

## 确认与处理

作者明确确认 Flurry 没有 extern 关键字，函数 ABI 由属性指定，C ABI 示例为 `^attrs.abi(.c)`。这条更正优先于此前 syntax 与章节中保留的错误产生式；直接承接，不继续提问。

删除六份文件中八处 `(extern "ABI")?` 产生式片段，覆盖函数定义、函数类型与模板实例。全局 syntax 指回通用 attr_def；调用章节补充 ABI 正式说明、C ABI 示例和通过属性读取 ABI 的实现引导；属性章节补充相同表面形式的示例与交叉链接。

修改文件：

- `docs/comptime/attribute.md`
- `docs/comptime/the-comptime-phases-of-traits.md`
- `docs/functions/calls-and-returns.md`
- `docs/functions/higher-order-functions.md`
- `docs/functions/index.md`
- `docs/grammar/syntax`
- `docs/type/qualified-types/closure-type.md`

## 示例与边界

已按 grammar/syntax、grammar/index.md 和作者本轮表面形式自检：

```flurry
^attrs.abi(.c)
fn identity(value: u32) -> u32 {
    value
}
```

不从 ABI 属性推导链接名、符号导出、默认 ABI、外部声明、安全等级或函数类型属性的额外语法。没有修改编译器，也没有改写历史记录。现行 union 文档中的 extern union 明确属于 Zig 对照，保留该真实语言名称，不把它当成 Flurry 语法误删。

## 检查

- 搜索现行正式文档，Flurry 产生式中的 extern 已清除；仅剩明确标为 Zig 的对照用语。
- ABI 示例使用当前属性前缀、投影、符号参数与函数定义形式。
- 更改文件的 git diff --check 通过；没有运行编译器测试。
- prompt 仍为空；本轮没有通过 prompt 收到新答复，也没有新审阅问题。

## 对作者的完整回复

已修正 [syntax](/home/fin/luna/docs/grammar/syntax) 及相关章节，函数 ABI 统一使用 `^attrs.abi(.c)` 属性，并补充了代码示例。更正已归档。
