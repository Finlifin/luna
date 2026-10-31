# 六个内置包的源码组织

本文件记录当前目录与职责，不将目录预留解释为功能已实现。
`lib.fl` 负责包根和 prelude 导出；实现按领域放入子目录。
没有 `.fl` 源码的领域以 README 记录边界，具体 API 仍按作者确认与正式 docs 实现。

| 包 | 已有源码 | 后续实现位置 |
|---|---|---|
| builtin | `primitives.fl`、`bindings.fl`、`types/`、`protocols/`、`numeric/`、`text_impl/` | `mem/`、`protocols/formatting/`、`protocols/capabilities/` |
| meta | `data/`、`definitions/`、`numeric/` | `reflection/`、`ast/`、`macros/`、`code/`、`types/`、`compilation/`、`runtime/`、`diagnostics/` |
| attrs | `calling_convention/`、`staging/` | `layout/`、`effects/`、`compilation/`、`derivation/`、`quote_fn_template/` |
| build | `info/` | `package/`、`targets/`、`tasks/`、`generated/`、`util/` |
| std | 根与显式 prelude | `mem/`、`collections/`、`string/`、`fmt/`、`io/reader/`、`io/writer/`、`runtime/`、`fs/`、`process/`、`time/`、`sync/` |
| verification | 包根与空 prelude | 高级程序验证库暂缓，见该包 README |

## 公开入口与实现路径

- `builtin.math` 与 `meta.math` 继续作为公开模块，分别导出 runtime 和
  comptime 数学函数；实现文件位于各包的 `numeric/`。
- `builtin/bindings.fl` 集中绑定文本观察与 Hash 使用的编译器操作；
  `builtin.compiler` 向其他内置包再导出这些声明，不加入 prelude。
  meta 的文本接口复用同一绑定，不重新绑定原始符号。
  绑定复用不代表运行时 str 的资源证明已经完成：现有 backing-region 阻塞仍由
  [原生文本复现](../development/repros/native-text-method-provenance.fl) 保留，不能关闭资源检查绕过。
- `builtin.text` 导出 `text_impl/bytes.fl`。`str` 属于 builtin，String 属于 std。
- Tuple、资源协议和迭代协议继续由 builtin 根及相应 prelude 导出。
  `types/option.fl`、`types/result.fl` 操作原生可选／错误结果，不定义枚举替身。
- `meta.List` 的 normal-form 定义保留在包根；列表方法位于 `data/list.fl`。
  `meta/bindings.fl` 是编译器操作的集中具名绑定层，公开领域接口在各模块中再导出。
  Object、整数操作、StructBuilder、EnumBuilder 的根入口保持可用。
- `attrs.abi`、`attrs.comptime_phase` 由领域实现再导出；build 的 mode、target、
  package_metadata 及相应类型由 `info/` 再导出。
- 非 std 包的 prelude 默认导入规则不变，std.prelude 仍需显式导入。

目录自动生成包内模块，不能与同名内联公开模块或根值隐式合并。因此实现目录
使用 `numeric/`、`text_impl/`，目标描述预留使用 `build/targets/`，避免与公开
math、text 和 build.target 冲突。内部目录名不成为新增的公开 API 承诺。

`language-items.conf` 使用真实声明路径，例如
`builtin.protocols.resources.Copy` 与 `meta.numeric.scalar_math.target.Scalar`。
变更实现归属时同步绑定、源码加载器、独立测试夹具及开发命令；再导出不复制声明。

## 后续文件拆分

新增实现进入对应领域，不继续堆积包根。一个领域变大时再按协议、适配器、
构造与观察等职责拆分；避免为了拆文件复制 trait、类型或 intrinsic 声明。
编译器专用原语入口集中封装，普通库实现复用公开别名与接口。

builtin/mem 承载 safe mode 也需要的基础内存规格；verification 的高级 API
暂缓不影响前者。meta/runtime 是编译期宿主桥接，std/runtime 是目标程序的
效应 runtime。std 容器与 I/O 实现应使用分配器和资源契约，不能填入虚假空实现。

历史原型继续保留在 development/library-prototypes，不纳入 sysroot 源码发现。
教程不是本布局或任何语言语义的权威来源。
