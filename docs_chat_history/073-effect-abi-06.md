# Runtime 封装栈池的实例供应

- 日期：2026-10-07
- global_index：073
- theme：effect-abi
- subindex：06
- 本轮目标：纠正上一轮把职责区分误写为使用端必须分别供应 runtime 与 pool 的处理。

## 作者输入

作者原文：“不，有了runtime,那肯定可以把stack pool封装到里面啊，有了前者就不用显式指定后者了”。

## 已确认与评估

runtime 可以封装并供应 stack pool，调用方显式指定 runtime 就能取得所需服务，不再要求同时显式指定 pool。独立使用池化 continuation 时保留 .use(pool)。

存储与调度在实现上的职责区分不要求用户暴露两个对象，也不要求用户在创建 runtime 前亲自创建、选择或配置内部池。具体构造仍属于库接口，本文不新增 API。

已有栈的来源归属、存活与清理责任继续由实际协议保证；封装可以将这些责任交给 runtime。作者这一更正已直接落实，没有提出新的确认问题。

## 修改

- ABI：明确独立 pool 与 runtime 封装供应两种接入；去除所有池都须在使用端显式供应的表述。
- Async：三个示例统一只用 .use(runtime)，正文说明 runtime 内部封装并供应栈池。
- Continuation：存储供应允许来自已经安装的 runtime，保留独立同步池的示例。
- 总览：明确两种使用方式。

## 验证与工作状态

沿用已核对的 grammar/index.md 与 handler_application -> expr.use(expr)，没有新增语法。检查四篇文档的示例括号、分号、围栏和空白，限定范围的 git diff --check 通过；没有运行编译器测试。pipeline projection 的 prompt 没有被修改或清空，本轮不是对该问题集的答复。

## 对作者的完整回复

已修正：runtime 封装并供应 stack pool，使用时只需 `.use(runtime)`；独立使用栈池时才需要 `.use(pool)`。

[相关正文](/home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md:48)和异步示例均已同步，去掉了重复指定 pool 的要求。
