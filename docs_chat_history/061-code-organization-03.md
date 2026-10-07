# 061 — code-organization — 03

- 日期：2026-09-27
- global_index：061
- theme：code-organization
- subindex：03
- 本轮目标：完成代码组织正式文档，更新全局 syntax，并补齐之前已定稿主题的语法遗漏。

## 作者输入与授权

作者表示：“已回复。我觉得不用再多说了。可以编写本主题文档、更新syntax了（如果前面的主题有syntax没更新的话也更新了去）”。本轮开始读取的 prompt 与 060 全部预填一致，没有额外修改；完整答复已追加到 060。本轮有明确正式文档写入授权，不再继续发起下一轮设计问题。

## 承接的确认结果

前轮 O1–O6：umbrella 保持成员 package 边界；默认扫描 src；目录自动成为模块，mod.fl 提供目录主体，同名文件／目录或内联模块冲突拒绝；普通 use 不自动公开；scripts 服务一般工程任务；build 属于具体定义的构建实例。

作者补充：builtin.prelude 自动导入其他包，std.prelude 显式导入；配置使用 comptime，build 提供接口；luna 是编译器，anar 是包管理与构建中心；默认包内可见、private 对所属模块及子模块可见、pub 全局可见；pub use 再导出；名称统一，包括 label，并允许 shadowing。

本轮按批准承接 N1–N7：

1. N1 A：前导点沿父静态容器，@ 定位 package 根，显式定位和后续路径不回退祖先。
2. N2 A：每层确定命名项优先于通配；内层先于外层；显式不同来源同名报错，同源导入去重。builtin.prelude 在包根按普通通配竞争。先选名字再查使用种类，不跳过被遮蔽项寻找类型或 label。顺序绑定初始化使用旧环境，新绑定随后生效。
3. N3 A：导入保持原始身份和许可；可以公开隐藏组织路径中的 pub 声明，不能公开原始非 pub 声明；访问路径在导入点合法，消费者使用新入口不重走旧路径。
4. N4 A：自动模块默认包内可见，由文件／mod.fl 的 configuration 提供所属模块属性；缺入口文件的目录照常成立。
5. N5 A：普通通配汇集当前位置可访问的成员；公开通配只收来源的 pub 入口并检查来源许可。选名先于公开过滤，祖先可见名字不冒充容器成员。
6. N6 A：内部通配歧义在使用时报告，公开出口歧义由提供包报告；显式失败立即报错。
7. N7 A：允许有真实来源的循环再导出；空通配环稳定为空，显式索取无来源名字时报缺失；不扩展到构建准备依赖环。

## 写入规划与结果

新增 docs/code-organization 下八篇，每篇按七段骨架组织：

| 文件 | 内容 |
|---|---|
| index.md | 阅读路线、四种组织关系及跨文件入门 |
| packages-and-umbrella.md | 包归属、依赖、根文件、目标与 luna／anar 分工 |
| modules-and-sources.md | 扫描与映射、mod.fl、模块属性、建树和结构失效 |
| scope-and-paths.md | 统一名字空间、路径定位、顺序绑定与标签遮蔽 |
| visibility.md | 三档许可、来源与入口、私有路径的公开出口 |
| imports-and-reexports.md | 显式／通配、身份、歧义、循环、传播与缓存 |
| builtin-packages.md | 六个内置包、两种 prelude、属性解析和 build 来源 |
| configuration-and-scripts.md | comptime 配置、任务、宏准备、源码生成和调度 |

正文共 1431 行、42 个 Flurry 代码块。示例包含跨包和跨文件片段、确定输出、合法构造、单独标出的拒绝例，以及菱形和有来源循环。实现引导给出声明／入口／来源表示、工作队列、完成条件、权限检查、增量依赖与复杂度边界，不指向编译器源码位置。

## Syntax 同步

本主题：补基础 id 路径、pub/private 定义与 use、模块内 configuration、filescope；移除 mod_statement。

之前主题：

- 字面量与基本构造：接通 literal 与 expr，补字面量词法速查和后缀；将 unit/null/undefined/通配与反身绑定分别列出，补括号表达式及 and/or。
- Comptime：补 defer、comptime predicate；comptime 块按已确认的可包含定义语义列出 definition 与 statement，并同步相应正文语法段。
- 函数：保留上一轮已经更新的 anchored lambda、命名供应、环境、quote fn、template 和 case 参数；修复 parameter_type 分支引用与已经存在的产生式名不一致；统一默认表达式占位。
- Trait：保留父 trait 和 bound 的 expr 可组合 A + B + C、否定约束在 requires 中的已定规则；保留此前已加入的 comptime impl／derive、deriver 等。
- 资源与验证：补 program_predicate、outcomes、assumes 和赋值产生式；不把程序 predicate 改为 comptime。移除用户公理入口及风格命名表中的相应残留。
- 控制流：把已定 inline 分支及 inline for 形式明确列出；普通 when 保持原有列表形式，不借本轮新增兜底臂位置约束。
- 属性与模式：补 statement／arm 属性入口、list rest 和非字面量模式入口。
- 宏：补文件级宏导入、两类构造宏调用、token buffer、模板宏捕获与 raw string；模板宏产生式名与函数 template 分开，表面语法未变。

已检查的语法规则名称共 261 项，无重复定义。syntax 是人类速查表；尚无已定正文定义的 assert/asserts/invariant/decreases 语句引用没有借本轮编造新契约，词法与构造宏输入类非终结符仍由相应章节约束。

## 衔接修改

README 加入阅读入口；基本类型与字面量对齐 prelude；成员投影消除“类型命名空间”可能暗示独立类别命名空间的歧义；控制流标签、顺序绑定与统一查找衔接；属性、宏、编译流水线衔接模块与构建依赖；grammar/index 同步组织规则。完整修改文件清单如下。

- `docs/README.md`
- `docs/basics/literals.md`
- `docs/basics/primitives.md`
- `docs/code-organization/builtin-packages.md`
- `docs/code-organization/configuration-and-scripts.md`
- `docs/code-organization/imports-and-reexports.md`
- `docs/code-organization/index.md`
- `docs/code-organization/modules-and-sources.md`
- `docs/code-organization/packages-and-umbrella.md`
- `docs/code-organization/scope-and-paths.md`
- `docs/code-organization/visibility.md`
- `docs/compilation/phases.md`
- `docs/comptime/attribute.md`
- `docs/comptime/index.md`
- `docs/comptime/inlining-comptime-code.md`
- `docs/comptime/normalization-and-instantiation.md`
- `docs/control-flow/basic.md`
- `docs/grammar/index.md`
- `docs/grammar/syntax`
- `docs/macro/construction-macro.md`
- `docs/macro/template-macro.md`
- `docs/resource-management/bindings.md`
- `docs/type/2-level-type.md`
- `docs/type/member-projection.md`

## 边界与未擅自决定的接口

文件模块可见性的 configuration 具体键名／值名，作者没有在 prompt 中补充；正文写已确认的附着位置、默认范围和属性检查，没有发明 attrs API 或键。包描述、umbrella 配置的字段名、anar 命令行、脚本入口和执行器接口也没有自行定义；构建章节写已确认的输入输出和先后关系，示例使用已有 build.mode 或独立辅助函数。

这些库／工具接口细目没有被伪装成已定表面语法。前轮发现的 resolver 实现问题仍留在 060 的检查记录，本轮没有修改编译器。

## 核验

- 八篇均包含完整七段，代码块围栏配对正确。
- 42 个 Flurry 代码块按 grammar 与作者确认的表面形式自检；检测 foreign syntax、错误的 } 后分号及占位省略，未发现问题。
- 修改文档中的相对文件链接与标题锚点检查通过。
- git diff --check 对 docs 通过。
- 26 个重点语法入口存在，产生式无重复命名；本次后续加入词法速查后共 261 项。
- 跨语言差异查阅 Rust/Cargo、Zig、C 标准草案、C++ 标准草案、GCC 与 CMake 官方来源，支持链接保存在正文对应句旁。
- 不运行 luna 作为语言语义验证器；上述检查是文档结构、来源和示例风格检查，不声称全部示例已编译运行。
- 与回合开始的内容摘要比较，compiler 下源码未变。

## 问题与协作状态

本轮没有新拍板问题，用户已授权结束本主题审阅并编写。作者答复完整归档到 060，当前结果登记为 061；prompt 在归档完成后清空，没有用下一主题问题覆盖它。

## 对作者的完整回复

已完成[代码组织文档](/home/fin/luna/docs/code-organization/index.md)：8 篇、42 个 Flurry 代码块，包含再导出与循环解析的实现引导。

已更新 [syntax](/home/fin/luna/docs/grammar/syntax)，补齐前面主题遗漏的语法，并同步相关章节。

结构、链接和示例风格检查通过；未运行编译器验证。答复已归档，`prompt` 已清空。
