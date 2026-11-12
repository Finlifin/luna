# 六包与真实交付场景

编译器与库是同一交付主线。这里定义重构必须支撑的接口责任和贯通场景，
不把未实现接口写成已可调用 API。`72932b66` 已交付 Linux 上的首批 std I/O，
具体能力与限制见 [宿主基础交付](std-host-foundation.md)，不再将 std 描述为空壳。

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

缓存键选择器 `key_protocols::select_metered` 已删除优先查找 `comptime_eq`／
`comptime_hash` 的旧分支。受信任的 trait 身份决定所用协议，成员固定为 `eq`／`hash`；
缺少成员、阶段不符及重复声明继续拒绝，另一个带前缀的方法不能补位。前缀名称本身
仍可用于普通用户方法，只是不再获得协议特权。孤立比较 fixture 同步改用两个 trait。
真实 sysroot 回归覆盖同名旧成员不能抢占、旧成员不能补齐接口、不能绕过 canonical
成员的 phase 检查，以及 Type 结果缓存的命中与失败恢复。
工作目录 `/home/fin/luna`，执行
`python3 -B development/check.py --timeout 900 -- test -p luna --test key_protocol_members --test key_preparation_budget --test library_comptime_comparison --test invocation_value_templates --no-fail-fast`，
`/tmp/luna-tools-e2i_h2fz/result.json` 退出 0，15 项通过。没有运行全 workspace。

当前存放位置不覆盖第 8 节的作者决定：跨两阶段的公共协议和 elaboration 协议以 builtin
为提供者。迁移时将协议声明归到对应 builtin 职责模块，meta 保留 comptime 数据实现及
必要再导出；不能在两个包复制声明来制造两个同名接口。runtime trait 与 pure comptime
trait 本身仍是两个不同声明，再导出只保持各自身份，不把两者合成一个 trait。

List 的 append_all、固定快照 foreach、逐元素 Eq 短路和 Hash 次数是
[值准入迁移](comptime-value-admission.md)的公开验收入口。Type 元素继续使用普通列表域；
内部持久存储复用不绕过元素协议，也不要求 comptime self 取得 runtime Copy。

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

当前已将 `Display` 与单字节 `ByteWriter` 协议移到 builtin，标量格式化只依赖
`W :- ByteWriter`，接收 `*W` 并返回 `!W.as(ByteWriter).Error void`。stdout 和
File 提供具体实现；File 保持自身关闭责任，用户 writer 可以是不可复制的有状态
对象。std.fmt.Display／std.io.ByteWriter 通过再导出保持同一声明身份。

这一阶段交付的是同步字节写入：成功接受一字节，失败不接受本次字节；格式化首次
失败即停止，已经写入的前缀保留。调用者不能从错误中推断整次格式化没有副作用，
也不能盲目重试整个值。带进度的批量 API、缓冲、Debug 与格式计划仍未交付。

迁移时验证了普通泛型方法、关联错误及 borrowed self 的现有通路，也暴露出两个
独立基础缺口，不能将它们隐藏成库规则：

- [兄弟字段范围复现](repros/writer-sibling-range.fl)：原先外部指针 store
  一律丢弃所有观察；现已按当前地址身份保留确定不相交的标量字段事实。writer
  测试恢复原先写字段再增计数的顺序。一般调用、位写回与未知别名仍保守处理，
  具体准入、拒绝边界和证据见[标量存储更新](solvers-resources-and-lowering.md#64-标量存储更新与读取事实的有效期)。
- [跨阶段文本观察](repros/staged-text-observations.fl)：IIR staging 的 `wrap`
  将已完成的 comptime 参数恢复成 runtime Let；多个观察会按 runtime str 移动
  规则冲突。要结合 Splice 实例、文本来源和阶段访问完成交接，不能全局替换所有
  Local 为常量、复制 producer、给 str 强加 Copy 或只在 writer 处特判。通用文本
  默认方法没有交付；现有 std 的 comptime 文本输出范围也不是最终语言规则。

本阶段验证工作目录 `/home/fin/luna`，使用真实 sysroot 与资源检查；沿用工作区
原有的资源分析排版修改，不宣称为干净 HEAD 验收：

| 命令（均经 development/check.py --timeout 900） | 结果与证据 |
| --- | --- |
| `-- test -p luna --test library_std -- --test-threads=1` | 退出 0，10 项通过；`/tmp/luna-tools-3jasp96j/result.json`。覆盖 O0/O3 非 Copy writer、自定义关联错误、准确前缀/写入次数、失败后恢复、File 借用、原 println 和系统错误路径 |
| `-- test -p luna --test sysroot_preludes --test trait_phases -- --test-threads=1` | 退出 0，15 + 73 项通过；`/tmp/luna-tools-p54hrida/result.json`。覆盖默认导出身份、真实库与阶段选择 |
| `-- run -p luna -- build development/repros/writer-sibling-range.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-writer-sibling-range` | 退出 1，最后的 count 加法范围未证明；`/tmp/luna-tools-f4e63q6x/result.json`。这是迁移初期的正例失败；后续修复与成功证据见上述标量存储更新章节 |
| `-- run -p luna -- build development/repros/staged-text-observations.fl --entry run --sysroot library --fuel 10000000 --output /tmp/luna-staged-text-observations` | 退出 1，第二次 text 读取报告 unavailable value；`/tmp/luna-tools-g5un6efs/result.json`。同样保留为待修复正例 |

Rust 测试文件经 `development/command.py --timeout 120 -- rustfmt --edition 2024
--config skip_children=true compiler/luna/tests/library_std.rs` 格式化，退出 0，
证据 `/tmp/luna-tools-v5no2sdu/result.json`。没有运行全 workspace 测试。
`python3 -B development/demo_println.py` 重新构建并运行退出 0，证据
`/tmp/luna-tools-1hq4va6r/result.json`；`/tmp/flurry-println` 已更新到迁移后的协议。

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

meta 的布局观察、attrs.abi 和 builtin 的目标数值接口需通过真实公开入口接到
[表示/调用计划](layout-and-call-abi.md)。反射取得结构不自动授予字段访问；属性求值
成功不等于 C ABI 可调用；builtin.math/meta.math 也不能由当前机器浮点传参方式补定契约。

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

## 9. 当前生产入口与迁移接线

下表来自静态源码核对，不是本轮执行结果，也不表示整个包完成。

| 路径 | 当前实际承载内容 | 重构必须一起接通的部分 |
| --- | --- | --- |
| [builtin 根](../library/builtin/lib.fl)及 [Tuple 绑定](../library/builtin/types/tuple.fl) | 原语、协议、math 与 prelude；Tuple 为带普通列表参数的 builtin callable 常量 | 普通签名/参数绑定、值准入、NF 身份及表示观察；不只替换库声明 |
| [meta 根](../library/meta/lib.fl)及 data 模块 | List 头、Object、两阶段方法来源、comptime 数学与反射 | 共同值域、值引用和冻结；方法选择按声明 phase，不按包名 |
| [StructBuilder](../library/meta/definitions/structs.fl)与 [FieldDescription](../library/meta/definitions/fields.fl) | 真正的字段编辑、Object 描述解析、Build 调用；不是仅有接口声明 | 保留顺序与编辑失败语义，将 Build 接到通用定义准备/发布服务 |
| [attrs 根](../library/attrs/lib.fl) | comptime_phase 值及 abi Object 构造器 | 普通名称解析、属性求值、目标消费者分别检查；有构造器不等于 ABI 已实现 |
| [build 根](../library/build/lib.fl) | mode、target、package_metadata 与类型包装 | 来源绑定、实际字段依赖、实例和目标；任务/工具接口按构建方案另行接通 |
| [std 根](../library/std/lib.fl) | 显式 prelude、Linux 文件/字节 I/O、Layout、println；builtin 格式协议的 writer 实现 | 动态文本、拥有型数据、allocator、批量进度、格式计划及 runtime 仍需贯通 |
| [verification 根](../library/verification/lib.fl) | 包身份和空 prelude | 保留边界；高级库暂缓不削弱编译器 safe 检查 |

[真实 sysroot 装配](../compiler/luna/src/frontend.rs)枚举六包，并把除 std 之外的提供者
交给普通 prelude 解析；不能根据旧 `intrinsic::PackageId` 只命名 builtin/std 的辅助接口，
断言现有 CLI 只加载两个包。重构统一的是实际包实例身份与来源，不能重新引入全局固定
包编号作为语义身份。

集中绑定也应复用：Object 的原始域绑定位于 `meta/types/bindings.fl`，与依赖 List
及数值协议的操作绑定分开。meta 的操作和公开 Object 模块再导出该声明，attrs 与
build 导入公开 `meta.Object`，不再各自建立原始绑定。完整 sysroot 使用正常的包依赖；
属性消费者不因只需要一个数据域而获得重复声明的特权。跨包签名的相互引用通过
声明准备解决，不以复制绑定作为规避 bootstrap 的手段。

build 已复用 `meta.Object` 与其固有 `get` 方法，删除本包重复的 Object／object_get
原始绑定；mode、target 和 package_metadata 仍保留原声明来源。目标 feature 与包
metadata 的 typed lookup 经过普通方法实例化，保持缺失、关闭和域不匹配的区别。
`build_context`／`build_metadata` 共 47 项通过，含真实 sysroot 的 O0／O3 执行，
证据 `/tmp/luna-tools-2m6haxur/result.json`（工作目录 `/home/fin/luna`，命令
`python3 -B development/check.py --timeout 900 -- test -p luna --test build_context --test build_metadata --no-fail-fast`，
退出 0）。源归属 fixture 加载真实 meta／attrs／build 与 builtin 绑定和比较定义，
保留无 codegen target、32 位与冲突布局检查；完整运行时库由上述 sysroot 用例覆盖。
这是 build 的验证范围，不表示其他库能力已经完成。

### 绑定去重必须保留阶段和函数契约

报告 6.2 的底层符号计数能定位审查位置，但不能单凭同名决定两个声明可互换。
检查对象应包括来源声明、阶段、参数和返回域、资源契约以及底层服务的前提。

| 当前绑定 | 核对结果 | 清理边界 |
| --- | --- | --- |
| Object／object_get | Object 曾在三包重复绑定，查询曾在两包绑定相同 comptime 契约 | 统一域身份，并由 build 复用 meta 的公开方法；不能用复制域声明解决依赖缺口 |
| 文本／Symbol 字节查询 | builtin 是 runtime 接口，meta 是 comptime 接口；runtime str 还有未完成的资源来源要求 | 共用底层操作不等于能直接共享同一个 callable；不能把阶段中立操作 token 或跨阶段调用当作去重方案 |
| integer_representation | 当前分别绑定 usize／isize 输入，以及 runtime／comptime 阶段，结果均为 u128 | 原语名相同仍有不同完整签名；未来用普通泛型封装时也必须保留输入域和目标宽度检查 |
| comptime_integer_convert | Integer → usize、usize → Integer、Integer → u8 是不同转换域 | 不以宿主表示猜返回域，不把原始操作别名当作任意转换；减少重复应通过受检泛型接口完成 |
| builtin.math／meta.math | 作者要求 runtime 与 comptime 都有接口；现有包装主体高度重复 | 可共享生成或普通代码复用机制，但必须输出各自阶段的公开契约；不能删除 meta.math 或让 runtime 函数充当 comptime 函数 |

上述签名区分可在库绑定和
[编译器操作契约](../compiler/elaboration/src/program/builtin_bindings/signatures.rs)
交叉核对。它解释当前不同声明的必要边界，不认可无类型绑定、直降豁免或所有包装
重复都是终局设计。runtime str 的两个无类型绑定及资源来源仍是实际待修复项；
本轮不修改作者已有的资源检查工作区改动。

## 10. 编译器服务绑定不授予库定义特殊语言规则

[language-items.conf](../library/language-items.conf)与
[驱动选择器](../compiler/luna/src/protocols/type_constructors.rs)当前负责定位协议及 intrinsic。
其中 Tuple/List 的 validate 仍检查专用 Role；这属于已知待退役路径，不能把源码层
已经使用普通 callable 绑定当成通用参数机制已迁移完毕。

目标绑定分三项处理：

1. **定位身份。** 按当前包集的普通路径和再导出规则解析到原始声明，保留来源与依赖。
   同一声明换一个导入别名不改变身份；同名但不同声明不获得内置能力。
2. **检查签名。** 通过普通 telescope、参数域、阶段和效果契约验证接口。
   Tuple 接收的 List<Type> 仍是普通值域；不另设 TypeList 参数槽或专用绑定算法。
3. **接入服务。** 编译器提供的原语或 intrinsic 绑定连接到对应受检服务，记录其适用
   前提、目标与证据责任。绑定身份不授予任意用户函数同样的底层表示权限，也不允许
   利用函数体执行结果展开 NF 的头。

布局、反射、模式及 collector 的使用者消费这些普通签名和操作契约；不得继续读 Role
来绕过通用规则。对用户定义头也适用的能力，必须从普通声明及语言规定的契约获得，
不能要求用户进入一份库名称白名单。协议配置的实际读取还需参与查询失效。
迁移时逐项核对配置消费者，不先删除配置文件再用硬编码默认路径补回同一依赖。

StructBuilder 的源码封装可以保留，底层 BuildStruct 服务改为生成普通定义草稿并提交
完整事务；enum、newtype、fn、impl 等按同一发布协议接入。通用的是结构验证、普通
elaboration、作用域与发布责任，不要求所有 builder 使用完全相同的用户接口。
`FieldDescription` 对 Object 的解析仍保留命名顺序和真实 Type 域；字段列表对齐、
重复名称、合法存储和后续布局各有检查阶段，不能在某一层成功后免除其余义务。

builder 返回 Type 也不自动符合 invocation 缓存条件。生成定义的源执行、身份分配及
发布按其契约处理；Type 结果筛选不能替代纯性、依赖和重放许可检查。
准备失败后没有半批声明，stale 重准备不重演已经完成的不纯步骤，仍按发布协议交付。

## 11. 库迁移的验证边界

[sysroot_preludes.rs](../compiler/luna/tests/sysroot_preludes.rs)包含两类资产：构造精简
六包来定位某项解析/协议行为的测试，以及指向仓库完整 library 的测试。两者都保留，
但前者不能单独证明真实六包可共同使用。现有完整库测试中显式使用 `.as(OptionalOps<...>)`
的路径可以验证指定接口，不足以单独证明未限定方法的 pure comptime trait 优先级。
本轮没有执行这些测试，历史名称或断言不作为当前通过证据。

### 11.1 真实六包的两阶段方法基线

2026-11-04 使用[源码构建的独立 driver](architecture-audit.md#6-可复现的现状基线)，
重新核对源码与二进制 hash 后，经完整 library/default preludes 执行临时输入。
没有显式 `.as(...)`，没有原始 builtin 访问，也没有禁用资源协议。
这里的运行证据与上文尚未重跑的既有测试分开记录。

公开接口输入的 comptime 部分如下；同一文件的 `fn run() -> bool` 使用完全相同的主体：

```flurry
comptime fn staged() -> bool {
    let present: ?bool = true;
    let absent: ?bool = null;
    let success: !bool bool = true;
    let failure: !bool bool = error false;
    present.is_some() and absent.is_none()
        and success.is_ok() and failure.is_err()
        and "é🦀".byte_length() == 6
        and ("é".byte(1) matches 169?)
        and ("é".byte(2) matches null)
}
```

实际 NIR 将 staged 的七个成员来源依次选为 ComptimeOptionalOps 两次、
ComptimeErrorResultOps 两次、ComptimeTextBytes 三次；run 则选为对应三个 runtime
trait，次数相同。由 dump 中 trait 声明身份与函数里的 interface-instance 对照确认，
不是仅凭相同返回值推断选择正确。

| 检查 | 结果 | `/tmp/luna-tools-` 下的报告目录后缀 |
| --- | --- | --- |
| public.fl，comptime staged | 退出 0，`(bool true)` | unsaox6w |
| public.fl，dump NIR | 退出 0；上述两组来源身份不同且符合阶段 | jpz5ywa3 |
| public.fl，interpret run | 退出 1；资源检查拒绝 string-view memory provenance，未交付可执行结果 | 99vt5bdi |
| values.fl，仅移除两函数的三个文本检查 | interpret run 退出 0，`(bool true)` | 7dneb3ef |
| priority.fl，用户普通 trait 返回 false，pure comptime trait 同名方法返回 true | comptime staged 及 interpret run（取反）均退出 0、返回 true | zwvqrk8d、t1atswl4 |
| ambiguous.fl，两个同层 pure comptime trait 提供同名方法 | 退出 1，ambiguous visible trait member；没有改选返回 false 的普通 trait | x417vjeb |

输入与摘取的 staged/run NIR 保存在 `/tmp/luna-library-phase-gcjlk26j`；各报告目录中的
result.json 保存完整命令、退出值，stdout.log/stderr.log 保存实际输出。工作目录为
`/home/fin/luna`；公开入口命令为：

```sh
python3 -B development/reproduce.py --timeout 60 --binary /tmp/luna-design-baseline-driver -- comptime /tmp/luna-library-phase-gcjlk26j/public.fl --entry staged --sysroot /home/fin/luna/library --build-target x86_64 linux 64 --fuel 4000000
```

runtime 使用同一参数，将命令换成 interpret、入口换成 run；dump 使用 `--emit nir`
而不带 entry/fuel。本轮未构建 native 产物，未测全部方法、泛型边界或未决候选。
现状结论是：这组普通可见性与两阶段方法选择已真实接通，可选/错误值的 reference
路径可执行；文本的方法选择也正确，但 runtime backing storage 交付仍阻塞资源检查。
后续迁移必须保留来源选择，并补齐文本来源/存活证据；不能为让该例通过而关掉检查、
把 runtime 方法替成 comptime 计算，或将全部 str 一律当作静态存储。

### 11.2 迁移后的验收仍覆盖完整消费者

| 迁移入口 | 必须核对的消费者 | 不足以替代验收的结果 |
| --- | --- | --- |
| 普通 callable / 值域 | 真实 meta.List、Tuple、用户头、具名列表赋值及展开 | 手工构造的已准入 NIR 或只通过某个 builtin 头 |
| 两阶段协议 | ?T、!E T、str 的未限定和显式接口调用，phase 缺失/歧义 | 显式 `.as(...)` 单一路径或按包名优先 |
| builder 发布 | 编辑失败保留草稿、反射、新类型使用、混合批次及失败恢复 | 仅打印 Type 或仅比较字段数量 |
| 属性 | 普通别名/遮蔽、Object 合并、作用目标、真正消费者 | `attrs.abi(.c)` 能构造 Object 就宣称 C ABI 可用 |
| 数学 | builtin.math 与 meta.math 的域、精度、阶段和目标 | 只在执行宿主上算一个浮点常量 |
| 文本与迭代 | 完整库公开入口、来源/清理、提前停止与嵌套 callback | 删除 Copy/Drop 协议或资源检查后得到正确数值 |
| build | 依赖包自身上下文、字段变化、同名不同实例、host/projection 区分 | 一份全局配置对象让单包例子成功 |

验证按当前纵向迁移选择最小充分组，不要求每次改动运行完整 sysroot/native 矩阵。
但修改公共绑定、prelude 或协议身份时，实际跨包消费者属于同一交付闭包，不能留给
“编译器完成后再修库”。本节明确共同迁移责任；后续实际接线进展见下文。

## 12. Type 协议比较接入既有类型等式

`meta/types/equality.fl` 为 Type 提供 `ComptimeEq` 实现，根模块加载该实现，
现有默认 prelude 继续导出原 ComptimeEq 身份。实现直接使用 `self == other`，
由既有类型命题执行机制处理；没有新增按类型名字识别的编译器分支，也没有用 Rust
Term 的结构比较替代 Flurry 的语义相等。默认 `ne` 仍通过同一接口的 `eq` 取反。

Type 不是 runtime 值域的普通数字；实现归入 types 目录，复用 numeric/equality 中
既有协议声明，不复制 trait。List<Type>、List<List<Type>>、?Type 通过现有泛型
容器实现组合，容器不取得额外的类型等式权限。未知等式沿普通调用传播为
TypeContractUnresolved；不能把“无法证明”当成 false 或让 ne 返回 true。

真实 sysroot 验收包含：

- typealias 与目标相等、不同 primitive／名义声明／newtype 不等。
- 同一 opaque normal-form 应用自反；不因 Identity<T> 的正文返回 T 就展开其头。
- 效应行顺序不同但语义相等的 Type 比较。
- 默认方法发现、默认 ne、嵌套容器比较、空列表与可选载荷、长度及首个不等元素短路。
- Identity<bool> 与 bool 的 eq、ne、列表元素比较均保留 Unknown；预算失败后恢复，
  重复执行也不能将 Unknown 变成已缓存的布尔答案。

此变更完成的是协议接线，不改变类型理论。Type 的 ComptimeHash 仍缺失，所以真实
meta 键协议尚不能完整支持 Type 输入。其后续实现必须让已证明相等的语义类型具有
兼容的哈希，考虑别名、类型构造等式、范式参数的所属等式域和适用世界；不能直接
复制 Term 的结构 Hash，也不能用恒定摘要占位并宣称键协议完成。

验证工作目录为 `/home/fin/luna`。使用
`python3 -B development/check.py --timeout 900 -- test -p luna`：

| 参数 | 结果 | 证据目录 |
| --- | --- | --- |
| `--test library_type_equality` | 退出 0，2 通过；真实 sysroot 与默认 prelude | `/tmp/luna-tools-tgbukhxp` |
| `--test library_list_equality --test library_list_hash --test invocation_value_templates --no-fail-fast` | 退出 0，3 + 13 + 4 通过 | `/tmp/luna-tools-y80jj9ad` |

新测试的 Rust 格式化经 `/tmp/luna-tools-8kp97zc3` 退出 0，提交范围 diff 检查通过。
隔离 meta fixture 同步包含真实 types/equality 模块；只有真实 sysroot 的测试负责证明
公开包接线成立，未将拼接 fixture 当作跨包验收。
