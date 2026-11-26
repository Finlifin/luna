# 编译管线接口

已提供基础接口 (`library/meta/compilation_impl/mod.fl`)：`CompileErr`、`SourceLocationInfo`、`TargetCfg`、`IIR` handle、`cfg()` 以及 `err` / `warn` / `note` / `*_at` / `emit_diagnostic`。Pass 调度、pipeline projection 与完整产物接口尚待后续管线开发补全。
