# 时间

`std.time.Duration` 已可用（含 `ZERO`、`NANOS_PER_SEC`/`MILLIS_PER_SEC` 等常量、纳秒归一化构造、`checked_*`/`saturating_*` 运算及 `Copy`/`Clone`/`Eq`/`Ord`/`Hash`/`Debug` 实现）。`Instant` 与 `SystemTime` 等单调/墙上时钟类型待宿主时钟 intrinsic 接入；时钟观察不视为纯计算。
