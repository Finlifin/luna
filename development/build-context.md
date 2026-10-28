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
克隆语法用 `Package::source_origins` 保存原始定义；新 HIR 绑定身份不改变
构建配置来源。签名 template 的默认值保留提供者上下文，实例主体保留实例
定义上下文。生成成员的原始身份随 ResolutionEvidence 原子发布，继续克隆时
沿用既有来源而不重映射到新声明。无来源的上下文读取拒绝，不回退到包 0。
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
`package_metadata` 的源码取值与语义接纳、构建脚本工具及 anar 调度/传输仍待实现。
不会通过运行时环境变量隐式补齐这些输入。

针对性验证：

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context
```


## 包元数据的驱动查询层

`luna::build_context::metadata` 提供单独版本化的不可变输入快照，不改变现有
mode/target 的配置规则。`Input` 按语义包身份提供有序字段输入，构造快照时拒绝
重复、空字段、未知输入包和歧义依赖图，并转为稳定排序的只读表。

`Entry::new(type_term, value_term, fuel)` 保留原始类型与值，不将库自己的 enum、
nominal Type 或 normal-form 头降成字符串／布尔配置。入口只验证闭合的描述符
结构并计费：拒绝活动绑定、洞、rigid 参数和 runtime 依赖。它**没有证明值属于
给定类型**；消费时使用 `Entry::admit` 或 `Observation::admit` 在提供者的编译世界
中做语义接纳，不能直接当作可信 runtime 值或跳过普通 comptime 阶段检查。

`Snapshot::read(origin, Provider::Dependency(alias), field)` 根据 origin 的语义包
身份和该包的直接依赖别名选择提供者。resolution ID 只用于翻译依赖边；同名包
可以同时存在，不做全局名称搜索，也不递归寻找间接依赖。`Provider::Local`
显式读取来源包自己。此处的 Local/Dependency 是驱动接口，不是新的 Flurry 语法。

元数据未提供是 MissingInput；提供空元数据后查询不存在的字段产生带 absent 的
观察项。显式 false、显式可选 null 和字段不存在三者不同。观察项保留来源包、
依赖别名、实际提供者、字段和不可变值；`unchanged` 检查改绑、类型/值变化、字段
增加/删除以及提供者输入丢失。无关字段变动不使此观察项失效。比较采用保守的
结构相等，不能由结构不同推导 Flurry 类型不等。

这些观察项只覆盖元数据输入依赖，不能替代源码、类型定义和编译上下文的依赖，
也尚未安装到编译缓存。快照、描述符与观察项均提供稳定转义的 S-expression dump。
源码 `build.package_metadata(...)`、CLI/流式传输和 anar 接线仍未实现；此阶段
交付的是后续接线必须依赖的驱动查询契约，没有新增占位的 build 包函数。

验证入口：

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_metadata --test build_context
```

本次验证（工作目录 `/home/fin/luna`）：上述组合命令通过原有 24 项 build context
测试和初版 7 项 metadata 测试，报告 `/tmp/luna-tools-icos208p/result.json`。
补充显式 null 与嵌套 runtime 依赖后，单独运行
`python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_metadata`
通过 9 项，报告 `/tmp/luna-tools-6jr3or6g/result.json`。
格式检查报告 `/tmp/luna-tools-oy2ybqjv/result.json`；`git diff --check` 通过。


### 元数据值的语义接纳

`Entry::admit(&mut Interpreter)` 将描述符解码为不可变数据并在该 VM 的类型世界
中检查值域、nominal 成员、payload、嵌入 Type 和类型形成约束。`Observation::admit`
同时保留 absent；present 值失败时返回带 owner/provider/field 的 AdmissionError。
这里不建立永久的 CheckedEntry，也不缓存 VM 的类型证明；在另一个提供者世界使用
同一描述符必须重新接纳。

实现复用 `Interpreter::decode_immutable_input`。它仅在 comptime 模式工作，并且
与普通 invocation 的 normal-term 输入重建区分：不会为元数据执行 ClosureSnapshot
工厂，也不重建 FunctionValue；这一限制递归适用于 optional、tuple、object、list
及 nominal 字段。普通显式 invocation 仍保留既有 callable 快照重建能力。Type 中
的 normal-form 头不展开，接纳也不授予 Copy／资源复制权限。

这一步尚未接通源码 `build.package_metadata(...)`、配置传输或编译缓存。驱动必须
提供实际类型环境；查询快照自身不加载依赖源码、不执行构建脚本，也不会把结构
检查或一次接纳提升为对其他编译环境的有效性承诺。

语义接纳阶段验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --test immutable_normal_inputs --lib`：
  VM 单元 175 项、解码集成 3 项通过，报告 `/tmp/luna-tools-ars2_owe/result.json`。
- 增加 Object 内隐藏 callable 和具名 FunctionValue 的拒绝例后，
  `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --test immutable_normal_inputs`
  通过 3 项，报告 `/tmp/luna-tools-526lhm7k/result.json`。测试中的工厂可由普通
  invocation 实际执行，元数据路径则保持无 EnterFunction 等执行事件。
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_metadata --test build_context`：
  metadata 12 项、build context 24 项通过，报告 `/tmp/luna-tools-kwyw66wd/result.json`。
  覆盖 nominal 的缺失类型、错误 variant/payload、false formation requirement、
  Object 条目类型不匹配，以及失败后的正常接纳。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过，报告 `/tmp/luna-tools-f10axxfp/result.json`；`git diff --check` 通过。

### 源码调用前的 NIR 读取操作

`BuildMetadata` 保存原始 owner、显式依赖别名与 provider 身份、可选的不可变
描述符。selector 只求值一次；VM 仅接纳选中的描述符，未选择的条目不执行。
缺失别名和已知 provider 未提供输入分别诊断。读取只能发生在 comptime，
进入 IIR 前必须完成物化；节点的遍历、类型替换、复制预算与 S-expression
输出已接入。它不读取进程环境，也不展开 normal-form 头。

这是源码 API 的内部执行支持，尚未把公开 `build.package_metadata` 声明和
驱动快照连接起来，也未接入 CLI 传输与缓存失效。不能据此宣称库 API 可用。

本阶段验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- check --release -p luna`：
  通过，报告 `/tmp/luna-tools-f5gxd9ur/result.json`。
- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --test immutable_normal_inputs`：
  6 项通过，报告 `/tmp/luna-tools-3sc7zzyy/result.json`。覆盖 selector 单次调用、
  未选中工厂不执行、缺失分类、错误后恢复、阶段拒绝、重复别名及 dump 转义。
- `python3 -B development/check.py --timeout 900 -- test --release -p nir --lib`：
  5 项通过，报告 `/tmp/luna-tools-hi_pujgu/result.json`。

### 公开 package_metadata 源码接口

`Snapshot::configure` 把直接依赖映射及类型化字段安装到 elaboration Protocols。
`build.package_metadata` 是上下文绑定的 comptime callable，读取时选取原始源码
定义的 owner，并生成 capture-free closure 包装 BuildMetadata。保存与转交该
callable 不改变来源；调用路由的常量探测也遵守该规则。库提供 PackageMetadata、
MetadataFeatures 及 `get(name, comptime T)`，返回 `?T`；缺失与域不匹配返回 null。
库循环使用普通 comptime while，不要求源码展开。复制元数据前进行预算检查。

实际覆盖了公开库声明、driver 快照配置、直接调用、别名与保存的 callable、
依赖 enum 身份、缺失 alias/input、字段缺失和类型不匹配。真实六包 sysroot
用例 `development/examples/sysroot-build-metadata.fl` 检查 comptime 物化后进入
LLVM host pipeline，在 O0 和 O3 下各执行两项测试。

CLI 元数据传输、缓存失效与构建脚本尚未接通。外部 driver 必须用当前依赖图
构造 Snapshot，并将配置变化纳入自身缓存键；本次不提供隐式全局包名查找。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context --test build_metadata`：
  build context 27 项、metadata 12 项通过，报告 `/tmp/luna-tools-fck_uvxh/result.json`。
- 随后新增真实 sysroot 用例，运行
  `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context real_sysroot_metadata_materializes_at_o0_and_o3`：
  通过，报告 `/tmp/luna-tools-e1k_ggm2/result.json`，覆盖 O0/O3 native 执行。
- 初版签名缺少显式 comptime Type 参数、inline 循环触发暂不支持的 open method，
  以及 callable 探测绕过上下文常量的失败均已修复；初版枚举 fixture 将内部
  variant 索引误写为名称，也已修正，未改变语言层的具名枚举用法。

### 类型化包请求与缓存

Rust embedding 调用者可以把 `Vec<metadata::Input>` 放入 `Request::metadata`。
包请求从自身的 sources/dependencies 构造快照，不接受另一个图的别名映射；
描述符与名称在复制前扣除准备预算。None 与 Some(empty) 保持不同含义。
完整请求缓存键包含字段的名称、类型、值、缺失状态及依赖提供者。这个保守
策略允许安全重用相同请求，不声称不同表示必然是不同类型。

CLI/resident frame 的文本编码尚未实现；字段选择性失效也尚未接通。
直接配置 Protocols 而不经过 Request 的外部驱动仍须自行维护缓存键。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test compilation_cache --test build_context --test build_metadata`：
  11 项缓存、31 项构建上下文、12 项元数据检查通过，报告
  `/tmp/luna-tools-omli73sb/result.json`。新增缓存用例实际检查 true/false、
  缺失字段、字段类型变化、依赖重绑定、无配置、缺失/未知/重复提供者、
  超额输入、错误后恢复及相同请求复用。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过，报告 `/tmp/luna-tools-3wzzzx6m/result.json`；`git diff --check` 通过。

### 显式 package CLI 与驻留传输

`--packages-v1` 和 `--packages-stream-v1` 的请求现在接受一次
`--package-metadata-v1 HEX`，直接解码到 Request 的类型化输入，因此沿用已有
快照验证和完整请求缓存失效。`metadata::wire::encode_hex` 提供生产端入口，
不解析 Display，不读取额外文件，不执行源码或闭包工厂。格式详见
[metadata-wire.md](metadata-wire.md)。输入保留 nominal/normal-form 头、optional
层次、字段类型、枚举 variant、任意精度整数/十进制值、位向量和 IEEE 原始位。

解码具有 128 KiB 字节上限、64 层深度上限及独立的遍历/结构/数字预算。
非法长度、UTF-8、标记、版本、截断和尾随内容均拒绝。执行性描述符与 store
handle 不可传输；effect-row/associated-type 描述符暂未分配 wire tag，明确
拒绝而非丢失信息。独立 implicit-sysroot CLI 尚未接受这一参数。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test metadata_wire --test build_context --test build_metadata --test compilation_cache`：
  wire 3、build context 32、metadata 12、cache 11 项通过，报告
  `/tmp/luna-tools-2658p9bg/result.json`。包含驻留请求的值变化、等价大小写编码
  复用、非法编码、空输入、缺失配置及后续正常请求恢复。
- 增补手写字节包、UTF-8 和未知 tag 检查后，运行
  `python3 -B development/check.py --timeout 900 -- test --release -p luna --test metadata_wire`：
  4 项通过，报告 `/tmp/luna-tools-hadaxmev/result.json`。首轮负整数解码问题已
  修复：符号与无符号 lexical integer 的解析分别处理。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过，报告 `/tmp/luna-tools-7hs7bhpk/result.json`；`git diff --check` 通过。
