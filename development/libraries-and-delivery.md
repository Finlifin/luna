# 六包与真实交付场景

编译器与库是同一交付主线。这里定义重构必须支撑的接口责任和贯通场景，
不把未实现接口写成已可调用 API。当前 std 的再导出壳不是标准库完成。

## 1. 包的长期责任

| 包 | 责任 | 重构需要保留/补齐的能力 |
| --- | --- | --- |
| builtin | 原语绑定、基础类型与协议、mem、runtime math | 集中绑定、普通 callable telescope、资源/模式/inline 迭代、目标 intrinsic |
| meta | comptime 数据、反射、代码与定义构造、编译观察、meta.math | 值引用、通用冻结、CMTT、完整 definition transaction、稳定编译器服务 |
| attrs | 属性 Object 构造与领域化组合 | 作用目标、合并规则、依赖、阶段与 ABI 契约；不把原始字典键散到库各处 |
| build | 包/源码/构建信息、显式任务及工具接口 | 定义来源上下文、实际读取依赖、输入与产物、取消失败；由 luna 管理 |
| std | allocator、拥有型容器/字符串、格式化、writer、runtime | runtime identity、内存 view、错误行、资源和效应路由的联合兑现 |
| verification | 高级证明接口 | 保留包身份与边界；高级内容暂缓，不推迟 safe 内存检查 |

`math` 不另建包。builtin.math 对应 runtime LLVM intrinsic 泛型包装，meta.math
对应 comptime 语义。共享操作契约和合法域，不要求用同一种底层执行器。
两者都包含目标/精度边界，不能以 host 浮点行为替换所有目标语义。

## 2. 名称、目录和 bootstrap

文件按数据、定义、反射、资源、模式、数值、编译服务等职责组织；根及 prelude
再导出稳定公共路径。不要因目录迁移产生两个 Tuple、Order 或 List 声明身份。
Order 使用项目命名规范，原语别名只在集中绑定层定义；使用处不能直接写 `.symbol'builtin`。

builtin.Tuple 的签名引用 meta.List，meta 又使用基础 Type；这是分层准备声明和实际求值
的问题，不能通过删真实依赖或给某包偷偷加特权解决。
先分配包/模块/声明身份，再准入最小绑定和签名，按真实语义依赖处理属性、值和主体。
名称相互可见不许可任意构建任务环或具体值计算环。

五个默认 prelude 是普通名称来源，提供者自身排除；std.prelude 显式导入。
同一声明从多个入口到达去重，两个不同同名声明仍要依照遮蔽/歧义规则处理。
六包为 builtin type 写 impl 的豁免只覆盖孤儿归属，不豁免 coherence 或阶段检查。

## 3. 类型方法与阶段

只用于一个阶段、为一个类型准备的方法放固有 impl。跨 runtime/comptime 的同名方法
按作者决定分别由 runtime trait 和 pure comptime trait 提供，适用于 ?T、!E T、str 等。
查找先确定名称来源及 phase 优先级，再进行普通实现选择；不能只给名字加 comptime_ 前缀。
当前 meta 的 ComptimeEq/Hash/Ord 和可选/错误协议可作为真实迁移案例。

## 4. 内存和 allocator

容器必须记录实际 allocator 依赖，而非只保留 trait 类型。借出 view 的有效期、扩容后的
失效、元素初始化、失败回滚和最终释放都进入接口契约。
默认分配器不返回 AllocErr 的要求应由实例身份及失败契约推导，不识别字符串路径。

| 情景 | 应保持的行为 |
| --- | --- |
| DefaultAllocator.default 及保留同身份的转发 | 不增加可恢复 AllocErr |
| 自定义可失败 allocator | 保留相应错误 |
| 类型相同但对象身份不同 | 不能继承默认实例的特殊失败契约 |
| 容量运算或元素构造失败 | 与分配失败分开，不能一并擦除 |
| 扩容移动部分元素后失败 | 清理已完成部分，原/新存储责任符合契约 |

默认分配失败最终采用哪种不可恢复策略尚未确定，本阶段不代作者选择平台 abort。
这不阻碍设计显式失败 allocator 的接口、依赖和资源模型。

## 5. 格式化与 writer

基础 Display/Debug 位于 builtin，不能以返回 String 迫使基础协议依赖拥有分配。
格式说明可以在 comptime 准备，runtime 以明确 writer 能力执行写入。
fmt 参数的具体公开签名仍需按既定语言能力推敲，本设计不登记一个假定的 Rust 式 API。

目标 writer 支持组合：缓冲、限额、计数、编码、错误转换和部分写入。
接口需说明实际写入量、失败时保留的状态、后续继续写入的许可，以及缓冲 flush/drop 行为。
println 只是组合后的系统输出入口，不能跳过能力和错误协议先用编译器打印 intrinsic 冒充。
runtime 服务可以封装 stack pool，明确 runtime 的实例选择即足够。

## 6. 重构中的七条贯通线

| 场景 | 必须走过的链 | 失败/边界案例 |
| --- | --- | --- |
| 普通值形成类型 | public meta.List → 用户 NF/真实 Tuple → TypeRef → 布局/调用 | 不同元素域、顺序、world、默认值和预算；无库名特判 |
| SoA 风格构造 | Type 反射 → draft → struct/fn/impl 批次 → 发布 → 普通调用 | 后半批错误不泄漏前半批；泛型不同实例不共用错误字段布局 |
| 正则自动机生成 | 数据算法 → quoted expr/pattern/arm → 参数化代码 → 执行 | Any 恢复、绑定卫生、单洞、资源与代码规模；不能以字符串解释器替代 |
| inline 迭代 | 真实 Iterator → 多组合器 → callee+handler 联合展开 → native | 提前停止、错误短路、嵌套 callback、临时接收者和清理一次 |
| 文本视图 | builtin.TextBytes → 普通调用/返回 → backing region → cleanup → native | 描述符替换、死亡存储、条件载荷、闭包转交 |
| allocator 容器 | 显式分配器 → 容器/借出 view → 扩容/错误 → 释放 | 实例身份、部分初始化、迭代中变更、默认错误契约 |
| 构建及 pipeline | build 输入 → 类型接口 → IIR handle → 后端 → 产物恢复 | host/device 配置分开、实际依赖变化、工具失败和缺失产物 |

这些是完整路径的设计要求，不宣称本轮运行或完成。选择实现批次时以这些路径的阻塞
决定基础工作顺序，不能在 VM 已无明显收益处无限微调而搁置库。

其中 build 的真实 mode/target/metadata 接口、pipeline 的任务通道和多产物发布边界
见[构建与产物交付](build-and-artifact-delivery.md)。工程缓存不能吞掉源 invocation，
也不能把后端返回 void 或工具退出 0 当作代码及入口已成功交付。

## 7. 现有资产的处理

真实 library 与 compiler/luna/tests 保留。development/examples、repros、bench
以及 library-prototypes 保留为验证输入和历史意向；prototype 不是正式 API。
现有 native 例子若禁用了资源协议，只能证明那条执行路径，不证明 safe sysroot 可用。
旧设计里提到 Option/Result、Ordering、anar、仍用 next 的 Iterator 已被后续决定/实现
取代，不迁入新设计作为待办。

## 8. 从旧草稿迁出的作者原始补充

以下是原 builtin-packages-proposal.md 的作者批注中仍直接约束本阶段的部分；
保留原意，其他已定内容已纳入 [语言契约](language-contracts.md)。

> Eq、Ord这些放进builtin.math。comptime和runtime都有的trait、elaboration协议trait，一般都放进builtin。
>
> Iterator、Copy、Clone、Drop、Tuple这些常用内容记得放进builtin.prelude导出。
>
> 得益于flurry强大的表达力，各种api的设计不必照搬rust, flurry可以有自己的设计，比如Display trait的fmt可以接收一个comptime fmt之类的。
>
> 注意std的容器要带头使用allocator风格，并且如果是DefaultAllocator.default的话，返回值就不能带有std.mem.AllocErr，这样简化了默认分配器的使用体验。
>
> 多分几层文件组织嘛，不用小气。

原草稿其他候选目录和未定签名不因迁移成为作者承诺。完整删除前内容已保存于本地
备份，位置见 [清理记录](documentation-reset.md)。
