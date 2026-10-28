# build

`build.mode` 是由驱动显式提供的上下文常量，值为 Symbol。普通名称解析先选中
声明，再使用读取表达式所属定义的语义包身份选择配置。别名和再导出不改变此
规则；局部同名定义仍按普通遮蔽规则处理。

```flurry
comptime fn tracing_enabled() -> bool {
    build.mode matches .debug
}
```

调用依赖包中的 tracing_enabled 时，读取的是该依赖的构建输入。`const saved =
build.mode` 同样保存自身定义来源的 mode，不在调用时改取调用者或 build 包的
配置。签名 template 的默认值仍读取 template 提供者的配置，实例主体则读取
实例自身的配置；quote fn 和已生成 quoted code 也保留其代码来源。缺少该来源
包的 mode 会报错，没有隐式 debug/release 或进程环境默认值。
普通 runtime 使用仍须满足 Splice 契约；可以在 inline if 中消费构建条件。

命令行通过 `--build-mode debug` 配置主源码包，依赖包不会继承此值。
显式包请求可重复使用 `--package-build-mode 200=release`，这里的 200 是
`--package` 提供的语义 identity，不是解析用 resolution ID。未知身份、重复
绑定、空 mode，以及同时为主包提供两种绑定方式都拒绝。构建输入只接受 NIR
编译或执行入口，不能在语法 dump 中静默忽略。

```sh
python3 -B development/library_check.py --release --stage sysroot-build-mode
```

该阶段通过完整 sysroot 编译并运行构建条件示例。普通命令也可使用：

```sh
luna development/examples/sysroot-build-mode.fl --sysroot library --build-mode debug --test-native
```

版本 1 的包 CLI 与驻留流式协议接受相同选项。配置是请求的一部分：当前缓存
按完整请求保守失效，mode 变化会重新编译；尚未使用字段级 Observation 做精确
失效。缺失配置失败不会返回其他配置的旧产物，恢复原配置仍可复用匹配的结果。

Rust 驱动仍可构造 `luna::build_context::Snapshot`，调用
`configure_modes(&mut protocols)` 后传入包编译入口；它完全替换旧 mode 表。
同时提供进程选项和驱动绑定时，不允许以不同值覆盖快照。单独指定 --sysroot
仍不意味着 mode 配置存在。

`build.target` 是同样按读取定义归属选择的不可变 `Target` 值，可保存、传参和
导入别名。公开字段为 `arch: Symbol`、`os: Symbol`、`pointer_bits: Integer`
和 `features: TargetFeatures`。后者的 `get(Symbol) -> ?bool` 在 comptime 查询：
未配置返回 null，显式关闭返回 false?，开启返回 true?。

feature 的 typed lookup 使用编译期 Object 的按名查询入口，一次扫描已经取得的
输入快照，避免在库循环中为每个索引重复传送整个 Object。驱动快照仍检查唯一
feature 名称；这个查询优化不改变包归属、缓存键或缺失配置的诊断。

```flurry
const target = build.target;
const vector_enabled = target.features.get(.avx2) matches true?;

fn use_vector_path() -> bool {
    inline if vector_enabled { true } else { false }
}
```

目标输入由 Rust 驱动 `Snapshot::configure(&mut protocols)` 同时安装 mode 和
host pipeline target。安装会替换旧表，不保留旧快照的包或 feature。执行宿主
仍是另一组数据，不替代目标；设备投影也不改写默认目标。编译入口提供具体
TargetData 时，读取到的目标布局必须与它一致。读取缺少目标的包会报错，单独
传入 mode 或选择 LLVM host 后端均不自动补齐目标配置。

命令行与版本 1 包/流式请求可用以下选项传入目标：

- `--build-target ARCH OS POINTER_BITS`：仅为主包指定目标。
- `--build-feature NAME=true|false`：添加主包目标 feature，可重复使用不同名称。
- `--package-build-target ID ARCH OS POINTER_BITS`：指定语义包 ID 的目标。
- `--package-build-feature ID NAME=true|false`：添加该包的目标 feature。

feature 可以出现在目标选项之前，但必须有同一包的完整目标选项；不能用
feature 参数局部覆盖驱动快照。重复目标、重复 feature、主包双重绑定、未知
语义 ID、空名称和不支持的指针宽度均拒绝。目标不会隐式传播到依赖包。
主包输入提供 NIR 所需的指针宽度；native 操作要求它与实际后端布局一致。
架构、系统和 feature 是驱动提供的构建事实，这些选项本身不切换 LLVM 的
目标三元组或启用 CPU 指令集，不据此宣称已支持完整交叉编译。

```sh
python3 -B development/library_check.py --release --stage sysroot-build-target
luna development/examples/sysroot-build-target.fl --sysroot library \
  --build-target configured_arch configured_os 64 \
  --build-feature vector=true --build-feature disabled=false --test-native
```

目标输入参与完整请求缓存键，包含 feature 缺失与 false 的区别。字段级查询
观察项尚未接入编译缓存。目标值中的 Object/Integer 等 comptime 数据仍受普通阶段与
Splice 检查，不能直接当作 runtime 配置对象使用。
`build.package_metadata(alias)` 通过源码定义所属包的直接依赖别名查询元数据。
保存或转交查询函数不会把读取上下文切换到调用者。驱动使用
`luna::build_context::metadata::Snapshot::configure` 安装快照；没有输入时不会
隐式读取进程环境。未知别名与 provider 未提供输入分别失败，显式空输入则合法。

```flurry
comptime {
    let features = build.package_metadata("cuda").features;
    let arch = features.get(.target_arch, cuda.config.TargetArch);
}
```

`MetadataFeatures.get(name, comptime T)` 保留原始类型身份，缺失字段或域不匹配
返回 `null`。元数据在提供者类型世界中接纳，不展开 normal-form 头、不执行
闭包工厂，也不授予资源复制权限。示例中的 `cuda` 是调用包的显式依赖，
`TargetArch` 由该依赖提供。

它与 TargetFeatures 共用按名 Object 查询。若手工构造 entries 并包含重复 key，
最后一项决定结果；最后一项类型不符时返回 null，不回退至此前同名项。普通位置
条目不参加名称匹配，非法的条目数据仍导致查询失败。

Rust 包编译请求可通过 `Request::metadata` 安装输入，完整请求缓存键包含
字段名称、类型、值及依赖映射，配置变化会保守地使缓存失效。显式 package CLI
和驻留请求支持 `--package-metadata-v1 HEX`，格式和限制见
[元数据传输](../../development/metadata-wire.md)。按观察字段选择性复用和构建脚本
工具尚未接通；直接使用 Protocols 的外部驱动
仍需维护自己的缓存键。参见[构建快照](../../development/build-context.md)。
