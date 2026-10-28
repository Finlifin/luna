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

当前入口是 Rust 驱动 API：构造 `luna::build_context::Snapshot`，调用
`configure_modes(&mut protocols)`，再将 protocols 传给包编译入口。
configure_modes 完全替换原有 mode 表，不沿用旧快照残留值。CLI/流式请求的
配置传输与编译缓存尚未接线，不能仅用 --sysroot 就假定配置已存在。

`build.target`、`build.package_metadata` 和构建脚本工具尚未实现。
执行宿主/目标数据与精确查询证据目前只有驱动侧接口，参见
[构建快照](../../development/build-context.md)。
