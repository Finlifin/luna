# 一致性易崩点与反例

每条包含不能丢失的信息、负责边界和验收反例。它们是重构的契约，不是已通过的测试清单。
不要将本表机械变成只检查字符串或枚举数量的测试。

## 1. 跨层不变量

| 编号 | 不变量 | 首次生产者 → 维护者 → 最终消费者 | 最容易误实现的反例 |
| --- | --- | --- | --- |
| C01 | 声明、名称与缓存定位分开 | resolver → 重绑定/query → 类型身份 | 改名别名保留同声明；同名新声明不复用旧类型 |
| C02 | NF 头永不因主体展开消失 | 应用形成 → substitution/freeze → PTC/IIR | Identity 的主体返回 T，应用仍不能当 T |
| C03 | Type 列表为普通值 | 参数计划 → VM/freeze → NF/反射 | 用户工厂接受 List<Type> 与 Tuple 一样；错元素域仍拒绝 |
| C04 | 显式输入与可解元变量分开 | 调用绑定 → candidate/CDCL → commit | 不为匹配候选改写显式类型或定义端参数 |
| C05 | scope/binder 随替换保留 | HIR/代码值 → 共用重绑定 → 实例/控制 | 嵌套同名变量、依赖域、return 目标不能被文本替换捕获 |
| C06 | Any 相容不产生相等或代码 | 描述形成 → observation → code admission | q:Any 实为 pattern，expr splice 必须拒绝；any 不是程序 |
| C07 | 检查顺序与执行顺序分开 | ArgumentPlan → staging/inline → VM/native | 后实参推导前 lambda，仍按源规则执行；显式值不重算 |
| C08 | 默认值按各自阶段/覆盖规则执行 | 签名默认 → 参数准备 → invocation | 显式覆盖不运行默认；子句默认不静默 defer |
| C09 | 残余合法性与活动等待分开 | normalization → freeze → instantiation | 没有生产者的 Unknown 不能冻成合法 residual |
| C10 | 不纯执行只执行获准次数 | 控制计划 → VM request/resume → build | 候选失败重试、重复唤醒、缓存命中均不能多执行外部操作 |
| C11 | 模型不等于类型解 | theory → model projection → inference commit | 析取中只满足一支的模型不能证明泛型调用对全域成立 |
| C12 | Unknown 不等于否定 | PTC/verification → selection/diagnostics → caller | 高精度候选耗尽，不能选择宽候选；多给预算可继续 |
| C13 | 证据带前提和世界 | 证明/impl选择 → cache/transform → 操作 | 新增 impl、退出分支或目标变化使旧证据失效 |
| C14 | Core 相同不等于 TypeRef 相同 | 类型抽象 → substitution → equality | 两个 Calibration 对象共用 core，依赖仍不同 |
| C15 | 身份不等于取得与存活 | 依赖生产 → EnvironmentPlan → resources/lowering | 能命名对象但已移出/失效，调用不能据此使用 |
| C16 | comptime 值引用不变成指针或复制 | 参数绑定 → VM slot/lens → 更新 | 对 self 元素赋值更新原容器；不用 runtime Copy 授权 |
| C17 | 视图依赖不等于指针槽 | 来源分析 → 调用/捕获/返回 → cleanup | 复制 str 后释放 backing storage，再读必须拒绝 |
| C18 | 清理也是行为 | Drop 选择 → 资源/纯度 → 所有出口 | 默认供应主体终止但 Drop 发散，不能称有限完成 |
| C19 | 错误行与效应行不同 | 行形成 → 路由/转换 → ABI | 错误包含不提供转换；同名 effect occurrence 不合并层次 |
| C20 | 编译器 builtin 绑定不授权同名源码 | 六包绑定 → 普通解析/签名 → 调用 | 重命名绑定仍工作；用户叫 Tuple 的定义不获特权 |
| C21 | 阶段化同名方法保留来源 | trait形成 → 发现/选择 → invocation | comptime 优先 pure comptime trait；没有凭前缀改名回退 |
| C22 | 生成定义检查和发布原子 | draft → ordinary checking → world/VM | 后半批失败不能留下前半批；修复后可重试 |
| C23 | 编码/布局不授予资源能力 | layout → bit conversion → move/drop | 相同位宽不允许复制仿射载荷或构造非法 enum 编码 |
| C24 | 产物与当前版本关联 | query → target → link/cache | 接口不变但内联主体变，旧机器码不可继续复用 |
| C25 | 存储版本、线性可见性与整体 coherence 分开 | scope/frontier → selection/publication → instance close | 较早调用不能看见未来生成实现；逐批成功仍可能整体冲突 |
| C26 | 动态执行身份不同于指令位置 | VM activation → request/resume → host | 循环同一 PC 要执行两次，同一次暂停重复通知只恢复一次 |

## 2. 必须成组设计的用例

### 2.1 同一值经过不同通道

构造一个含 Type、布尔及有序列表的普通 comptime 值，分别经过局部变量、普通参数、
NF 参数、Any 容器、生成定义属性、冻结恢复。检查值域、顺序和依赖保持一致。
再改其中一个值的域、声明世界或代码类别，必须在正确接收边界拒绝。
不能只测试 JSON/Term round trip 而不走源码及真实 sysroot。

### 2.2 同一调用经过不同准备顺序

后实参供给前 lambda 的类型，默认值引用前参，collector 混合显式元素和展开，
回调在准备时修改外部状态。检查器可以重排检查，实际调用顺序、次数不变。
执行中早退后未到达的实参/分支不得预先执行；失败后的新调用不继承旧 frame。

### 2.3 同一局部定义在多个实例中出现

模板实例使用 bool/u8 等不同输入；局部 fn、alias、newtype 及嵌套前向引用保持独立身份。
依赖仅出现在契约、默认值或间接 callee 时仍保留；无关外围输入不强行捕获。
真正循环应报循环；`flag() { true }` 不得误依赖外围正在求值的默认输入。

### 2.4 同一 TypeCore 的不同对象环境

两个 runtime 对象具有相同形状，产生两份类型引用；core 可共享，substitution 不可混。
一个依赖重复出现保留同一身份。将值跨函数返回、放进闭包或聚合，再经 inline 展开：
检查对应 EnvironmentPlan 的实际取得和清理，不能只断言 interner ID 相等。

### 2.5 一次失败后继续使用

对 candidate、PTC、VM、builder、query 分别在预算边界、非法输入和取消点失败。
然后同一会话中进行合法操作，核验没有泄漏事实、候选赋值、局部位置、诊断或半批声明。
外部执行不能假定可回滚；检查进度恢复和对外失败报告，不用“重跑一遍”掩盖。

### 2.6 一份资源经历所有出口

正常返回、错误传播、pattern 失败、break/continue、handler abandon/return 各有清理路径。
在析构中读取真实 backing region，观察责任恰好退休一次；返回 view 的存活依据覆盖使用期。
O0/O3 结果和可观察轨迹应一致，且每条测试启用真实资源协议。

## 3. 证据的有效期

| 证据 | 失效事件 | 处理 |
| --- | --- | --- |
| 参数域/关联类型 | 前置输入、选定实现或接口变更 | 重做相关观察与准入 |
| PTC 学习结论 | guard 退出、world/target 依赖变化 | 停用或重证，不能无条件留在全局 |
| 代码值接口 | 实际元素替换、固定依赖失效 | 新值重新检查，不复用同下标旧证书 |
| VM 值引用 | 根槽替换、世代结束、会话退出 | 旧 lens 拒绝访问 |
| P3 稳定观察 | write/kill、未知回调、边界外使用 | 更新版本或撤销复用资格 |
| 类型环境取得 | 移动、释放、捕获方式/控制范围变化 | 重新检查取得与存活 |
| 实例验证产物 | 改变控制、输入、cleanup、环境的变换 | 按 pass 契约重证受影响项 |
| 目标 artifact | 主体、依赖、target/ABI/工具版本、实际文件变化 | 重新生成或物化 |

## 4. 不值得保留的“兼容性”

- 历史测试曾接受原始 builtin 访问、special TypeList 槽或 Optional 构造器要求，
  不构成保留实现偏差的理由；迁移必须先给出作者规则及替代机制。
- 历史运行器用局部协议证明局部行为，可以保留为隔离测试，不能代替默认 sysroot 验收。
- 测试把缺能力写成永久拒绝时，应改成明确未完成正例，不能降低目标合法程序。
- 不以文件更短、枚举更少、测试更多证明架构正确；要展示证据跨层流转和旧路径退役。

## 5. 本阶段检查范围

本轮只对设计的来源、源码定位、文档链接、删除范围及受保护文件一致性做检查。
本表的跨层用例是后续实现验收要求，尚未执行，不计为通过。
