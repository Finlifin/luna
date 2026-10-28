# build

源码 `build.mode` 已通过显式驱动配置接通，见 [build 包说明](../library/build/README.md)。
`build.target` 已通过 Rust 快照输入接通；`build.package_metadata` 尚未接通。
不提供占位值或宿主默认配置。

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
  按键排序。接口版本是驱动输入契约，完整 Snapshot 仍无持久化传输格式；
  mode/目标输入可通过版本 1 包请求的命令行参数传输。

`configure_modes(&mut protocols)` 将快照中的 mode 绑定到源码 elaboration，读取
位置按原始 HIR 定义归属取值，产物保留已选中的 Symbol。
`configure(&mut protocols)` 同时替换 modes 和 targets。目标使用真实 build 包的
Target/TargetFeatures 声明，编译器校验 marker 的字段布局后生成 nominal 值；
features 保存排序后的 Object，公开 get 方法是库中的普通 comptime 代码。
具体编译布局与目标快照冲突时拒绝，未提供编译布局时可以独立进行 comptime
配置查询。源码别名、常量和依赖包读取不会转移配置归属。
字段级缓存证据尚未接线，Observation 仍是驱动侧接口；CLI/流式 mode 和目标输入已
进入完整请求缓存键，但不能据此宣称精确缓存
失效或所有 template/CMTT 生成场景已经端到端完成。
下一步需要将观察项交付给增量编译缓存。CLI 目标参数仅提供构建事实和主包的
语义布局，不改变 LLVM 目标三元组或 CPU feature 设置；native 布局冲突拒绝。
`package_metadata` 的类型化值输入、构建脚本工具及 anar 调度/传输仍待实现。
不会通过运行时环境变量隐式补齐这些输入。

针对性验证：

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context
```
