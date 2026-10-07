# 栈池也显式指定实例

- 日期：2026-10-07
- global_index：072
- theme：effect-abi
- subindex：05
- 本轮目标：落实作者对栈池实例供应方式的补充，更新已授权编写的代数效应文档。

## 作者输入

作者原文：“stack pool我也想像显试指定实例”。结合上一轮 runtime 的显式实例接入，按“栈池也由使用者显式选择并供应具体实例”落实。

## 评估与已确认

标准库提供池化实现，与用户选择具体实例是不同层次。栈池负责分配、保留与回收，runtime 负责调度、等待与唤醒；同步 continuation 可以独立使用栈池。实例供应沿既有 trait 协议与 elaboration 接入，不要求增加关键字或函数签名段。

采用既有 .use(pool) 形式明确供应实例；范围内继续经协议传递。不同安装可使用同一实例或不同兼容实例，检查实际 ABI 和访问关系。库可以封装显式选择的池，但封装须保留它的存储依赖。

恢复点重新供应外层效应入口，与已经分配的栈属于哪一个实例分别记录。已有栈保持地址与来源；跨池捕获的各段分别归还原池。continuation 逃逸也保留池及其存储依赖，不能先销毁仍被使用的池。

这是作者补充及其与既有地址稳定性、可达性和资源清理规则的接合，不重新询问是否采用默认全局池，不引入新的池构造函数或公开 trait 名。

## 文档修改

- [ABI](../docs/algebraic-effect-and-delimited-continuation/effect-abi.md)：显式实例供应、与 runtime 分工、来源与存活规则；增加同步 .use(pool) 使用端示例，更新跨池栈链示意及分配／归还算法。
- [Continuation](../docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md)：补池依赖，三个测试片段显式接入 pool；说明实例由测试环境预先建立。
- [Async](../docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md)：示例使用 .use(runtime).use(pool)，外层 pool 供应覆盖 runtime 安装及其子工作；记录各段栈的实际来源。
- [总览](../docs/algebraic-effect-and-delimited-continuation/index.md)：加入两类实例的分工和显式供应。

## 检查与后续状态

已按 grammar/syntax 与 grammar/index.md 检查 .use 后缀、handler 模式、resume、括号、逗号和分号。四篇文档的章节顺序、围栏、代码括号与空白检查通过，限定范围的 git diff --check 通过。此次没有新增编译器测试，也不宣称现有编译器已经支持相应协议。

没有新增需要拍板的问题。pipeline projection 的六题仍等待审阅；本轮没有收到对这些问题的回复，prompt 内容保持原样，并通过摘要校验确认。未修改 compiler 或 proposal，未提交或推送。

## 对作者的完整回复

已补入 [ABI 文档](/home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md:48)：stack pool 也通过 `.use(pool)` 显式指定实例。同步 continuation 可以单独使用栈池；异步示例同时供应 runtime 和 pool。

已有栈始终归还分配它的 pool，恢复时更换外层供应不改变这份归属；pool 必须存活到相关栈完成清理并归还。continuation 和 async 章节的示例也已同步。

## 后续更正（2026-10-07）

作者随后明确：runtime 可以封装 stack pool，有 runtime 就不必再显式指定 pool。上文要求异步使用端同时 .use(runtime).use(pool) 的处理已由 [073](073-effect-abi-06.md) 更正；原文保留为历史记录，不作为现行规则。
