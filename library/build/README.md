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
配置。缺少该来源包的 mode 会报错，没有隐式 debug/release 或进程环境默认值。
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

`build.target`、`build.package_metadata` 和构建脚本工具尚未实现。
执行宿主/目标数据与精确查询证据目前只有驱动侧接口，参见
[构建快照](../../development/build-context.md)。
