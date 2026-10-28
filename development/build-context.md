# build

本包的源码入口尚未接通，不能在 Flurry 程序中使用 `build.mode`、`build.target`
或 `build.package_metadata`。本文记录实现边界，不提供占位值或宿主默认配置。

已实现的基础位于 `luna::build_context`：

- `Snapshot::new(INPUT_VERSION, sources, inputs)` 接收明确的源码包集合和每包
  构建输入。构造成功后快照不可修改；缺失、重复、未知身份和不支持的版本拒绝。
- 每包包含 mode、执行宿主和 host pipeline 目标。目标保存 arch、os、已校验的
  TargetData 及 feature 表。mode/架构等名称保留输入值，不将任意工程配置名称
  限制为编译器运行时的固定枚举；空名称拒绝。
- `read(originating_declaration, query)` 使用定义来源的语义包身份，不使用调用者、
  resolution ID、导入别名或磁盘路径。默认目标不随设备投影隐式改变。
- `Observation` 保留来源、查询项和读取值；`unchanged` 对照新快照逐项检查。
  只读 mode 不依赖目标 feature 或执行宿主。缺失 feature 与显式 false 不等价，
  来源包消失也会使旧证据失效。
- 快照和查询证据的 Display 为稳定的 S-expression，名称转义，包与 feature
  按键排序。接口版本是驱动输入契约，尚无对应的流式/持久化传输格式。

这些是驱动侧基础 API，尚未接入源码 elaboration、comptime invocation 或编译
缓存；因此不能据此宣称内联/template 下的源码配置归属或缓存失效已端到端完成。
下一步需要把定义来源和快照接入源码查询，同时将观察项交付给增量编译缓存。
`package_metadata` 的类型化值输入、构建脚本工具及 anar 调度/传输仍待实现。
不会通过运行时环境变量隐式补齐这些输入。

针对性验证：

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context
```
