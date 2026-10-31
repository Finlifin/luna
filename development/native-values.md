# 原生可选值与编译期列表

`?T` 使用独立的 NIR 类型、构造和 pattern，不再绑定库枚举。
`null` 表示当前这一层缺席；向 `??T` 注入一个 `?T` 会保留整个输入，
包括输入本身的缺席状态。传播只取出一层，失败路径按普通提前返回规则清理。
CLI 和 sysroot 配置中的 `optional-type` 已移除。

```flurry
fn missing() -> ?bool { null }
fn wrapped() -> ??bool { missing() }

fn run() -> bool {
    wrapped() match {
        inner? => inner match {
            value? => false,
            null => true,
        },
        null => false,
    }
}
```

这里 `wrapped()` 的外层 present、内层 absent，`run()` 返回 true。
不能把两层缺席合并，也不能把 null 当作一个有默认布尔载荷的值。

这一表示贯穿类型替换、候选匹配、comptime 值准入与冻结、IIR、
内存／移动／清理分析和 LLVM。IR dump 使用 S-expression；宿主后端目前使用
显式 tag 与载荷存储，不把该布局当成对外 ABI 承诺。

`meta.List<T>` 保留 normal-form 头身份。编译期构造、索引、收集与展开
检查完整列表类型以及每个元素的上下文类型；依赖前置参数的收集器在获得
元素类型后完成字面量转换。冻结后的普通列表携带原列表类型，不能用一个
形状相同的任意 normal-form 头冒充受信任的 List。`List<Type>` 保留专用表示。
这不改变 Tuple 接收一个 `List<Type>` 输入的定义，也不会展开 opaque 头。

库中的 OptionalOps 和 ErrorResultOps 分别服务于语言可选类型和错误结果，
不再公开 Option／Result 枚举。迭代协议使用 `inline fn iterate` 与
`inline effect emit_item`，可选值只在当前层 present 时交付一次载荷。
驱动选择器统一使用点路径；math 的实现位于 builtin，经 builtin.prelude
导出 math 入口，数值函数仍是 LLVM intrinsic 包装。

## 验证入口

在仓库根目录运行：

```sh
python3 -B development/check.py --timeout 1800 -- test --release -p nir -p iir -p comptime_interpretor --tests
python3 -B development/check.py --timeout 1800 -- test --release -p elaboration --lib --test calls --test implementation --test ptc
python3 -B development/check.py --timeout 1800 -- test --release -p luna --no-fail-fast --test native_optional --test optional_purity --test type_lists --test dependent_list_collectors --test library_protocols --test library_iteration --test frontend_cli --test execution --test drop --test effect_memory_transfers --test pure_callbacks --test runtime_purity --test host --test host_intrinsic_protocol --test compilation_cache
```

`native_optional` 检查嵌套缺席、求值次数、传播、推断恢复及 LLVM 执行；
`optional_purity` 检查可选回调的身份传递、循环赋值和不纯／未知调用拒绝。
低层测试检查伪造载荷、错误列表头、作用域、预算耗尽后的恢复、冻结及残余消除。
资源与迭代测试检查清理次数、提前退出、借用和回调重复消费。

本阶段在 `/home/fin/luna` 验收了 4,835 项 Rust 测试及 18 项脚本测试，
选定 Rust 文件的格式检查通过。日志锚点如下：

| 范围 | 结果 | 报告 |
| --- | --- | --- |
| NIR、IIR、VM | 777 项通过，退出 0 | `/tmp/luna-tools-_i602kld/result.json` |
| Elaboration | 446 项通过，退出 0 | `/tmp/luna-tools-c50gjvsr/result.json` |
| Luna 组合检查 | 13 组共 1,596 项通过；另外两组当时失败，退出 101 | `/tmp/luna-tools-3crqehoi/result.json` |
| 修正后重跑 execution、host | 2,016 项通过，退出 0 | `/tmp/luna-tools-xqcs7f_3/result.json` |
| library_check、run_host 脚本 | 11 项、7 项通过，均退出 0 | `/tmp/luna-tools-enxkkc2w/result.json`、`/tmp/luna-tools-w3ei1ryg/result.json` |

旧测试修正包括：按初始化后的剩余预算测试耗尽、更新 List 元素域诊断、
用源程序的清理次数和次序替代生成包装函数名称，以及区分 handler 安装
期间与安装前后的指针槽变化。重复消费 affine 闭包的拒绝例仍保留，并同时
检查 direct-passing 与普通 effect 调用。

## 尚未完成

完整 sysroot 仍受 panic 终止／清理契约、格式化、分配与集合等接口接线限制。
不能把按模块加载的库测试通过视作完整标准库可用。
未展开的 affine fold 仍可能要求环境写回契约；相关测试同时验证展开后的路径，
并保留未展开路径的诊断。通用载荷 Clone／条件 Copy、任意异构错误行的
Once 实现，以及完整 List 库操作仍需继续开发。
