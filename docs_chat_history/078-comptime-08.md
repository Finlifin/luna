# Comptime：invokation 命名

- 日期：2026-10-07
- global_index：078
- theme：comptime
- subindex：08
- 本轮目标：按作者要求替换 comptime 执行载体的术语。

## 作者输入

> 去comptime主题，把comptime execution换成comptime invokation更加贴切

## 评估与已确认结论

作者明确授权本次术语修改，采用指定拼写 comptime invokation。其所指仍为独立 comptime 函数编译为 engine 字节码后，由调用触发的执行载体。

## 文档写入

1. docs/comptime/normalization-and-instantiation.md：执行载体表采用 comptime invokation。
2. docs/comptime/purity-and-normal-form.md：资源预算说明采用 invokation。
3. docs/compilation/phases.md：同步该载体的交叉引用表述。

本轮没有新增拍板问题、待确认语义或 Flurry 代码示例。保留历史记录中的原始用词；正在审阅的编译主题 prompt 保持原有内容。

## 检查

检索当前 docs 中 execution 的相关引用，确认三处均完成替换；对三个修改文件运行 git diff --check。本轮仅改术语，无需编译测试。

## 对作者的完整回复

已统一改为 `comptime invokation`，并同步预算说明及编译流水线中的引用。
