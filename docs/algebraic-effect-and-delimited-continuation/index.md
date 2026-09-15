# 代数效应与处理器

## Intro

效应声明一个操作请求，处理器提供它在当前环境中的实现。计算可以只依赖操作契约，把处理方式交给调用环境。

效应行还保留处理层次，使内部处理器可以把请求交给外层，并在恢复计算时维持原来的边界。

## 语法

~~~text
algebraic_effect -> async? effect id(parameter*) (-> result)? clauses? (block | (= expr))
effect_qualified_type -> #expr expr
effect_handle -> expr# { case_arm* }
perform_effect -> expr#
~~~

签名可以省略主体；case_arm 与子句同全局语法。具体恢复能力使用其控制流契约，本篇不增加 continuation 源码形式。

## 正式定义

独立操作的完整实例形成效应标签。允许行使用 scoped labels：同标签层次保序，不同标签在身份已决时可交换。重复调用不增加处理层数。

普通直接处理分支产生操作返回值，调用点随后继续。可恢复处理采用 deep 边界：分支在当前边界外执行，恢复或转发的后续计算重新安装当前处理器。

恢复能力按单次、不可逃逸及有依据的清理规则使用。部分覆盖需要向外路由；没有路由时不能宣称操作已完全处理。

完整类型关系见[效应类型](../type/qualified-types/effect-type.md)、[行](../type/rows.md)和[handler](../type/handlers.md)。

## 常见示例

~~~flurry
effect ask() -> u32;

fn request_number() -> #[ask] u32 {
    ask()#
}
~~~

处理 ask 的直接分支必须给调用点一个 u32；这与主体或整个 handler 最后返回什么需要分别判断。未安装处理器的调用仍须由调用环境提供合法入口。

## 与 Rust / Zig / C / C++ 的关键差异

没有与该通用效应行和恢复规则一一对应的构造。回调、上下文参数、错误返回与异常是实际替代手段，适用的控制流不同。理论对照见[handler 的差异说明](../type/handlers.md#与-rust--zig--c--c-的关键差异)。

## 运行时实现设计引导

直接处理可使用普通函数调用与隐藏环境参数。可恢复处理必须维持限定边界及有效恢复能力；不能无条件复制栈或丢弃资源。

转发时连同“恢复后重新安装当前 handler”的路径一起传递。只有标签名、没有 occurrence 路由不足以选择正确入口。

## 编译器实现引导

检查操作签名、输入行、分支允许行、结果转换和所有转发路径。将类型证据与运行时入口一起实例化，使用[handler 算法](../type/handlers.md#编译器实现引导)验证降码。
