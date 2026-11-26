# 生成代码诊断

已提供基础类型与构建器 (`library/meta/diagnostics_impl/mod.fl`)：`DiagnosticLevel`、`LabelStyle`、`SpanLabel`、`Suggestion`、`Diagnostic` 与 `DiagnosticBuilder`，通过 `builtin.compiler.emit_diagnostic` 接通编译期报错与警告/提示通道。完整多 span 渲染细节仍随编译器诊断渲染管线持续扩展。
