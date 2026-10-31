# Trait 约束组合与否定子句

- 日期：2026-09-26
- global_index：050
- theme：trait-implementation
- subindex：05
- 本轮目标：根据作者纠正，同步父 trait 与 trait bound 的组合表达式语义。

## 作者原话

> trait 定义那里的父trait和trait bound表达式的expr，其实都不只是接收单一trait表达式，而是可以接受A + B + C这样的多个trait约束。为了保持简单，否定约束请在子句中写

## 已确认与落实

父 trait 和 `where T :- expr` 的 expr 都可以通过 `+` 表达多个正向能力的合取；每项允许完整参数化 trait 表达式。否定写入 requires 子句，不扩展父接口或 bound 列表的负向语法。

同步文件：

- [接口声明](../docs/traits/declarations.md)：语法说明、正式定义、组合父接口和泛型 bound 的完整例子、否定子句例子及编译器义务分解。
- [总览](../docs/traits/index.md)：两种位置共用组合规则的速查。
- [全局语法](../docs/grammar/syntax)：保留原有 expr 产生式，用注释明确 + 组合；没有新增专属语法节点。

组合逐项取得证据，不合并接口身份，也不建立书写顺序决定的同名成员优先级。否定需要 PTC 依据；未知或暂未找到正向实现不自动成为否定证明。

## 状态与验证

本轮是对已授权正文的纠正，无需再次提问。资源管理 049 的八组预填仍待作者审阅，本轮没有消费或改写该 prompt，也没有重建一份 trait 问卷。

新例沿用 Reading、Named、Meter，两个调用都应输出 `meter: 8`；show_enabled 用 requires 表达 not。按现行 grammar/style 复核了 +、:-、not、子句分隔与大括号位置，没有使用外来语法或未定义库 API。未进行 Flurry 编译执行。文档 whitespace 检查通过。

## 完整过程回复

> 我会把父 trait 和 `where T :- ...` 统一写成可接受 `A + B + C` 的能力合取，并补上示例；否定约束则明确放在子句的 `requires` 中。

## 完整最终回复

已同步到[接口声明](/home/fin/luna/docs/traits/declarations.md)、总览和语法说明：

- `trait D :- A + B + C` 与 `where T :- A + B + C` 都表示各项能力同时成立。
- 否定约束写成子句中的 `requires not (T :- C)`。

补了多父 trait、组合 bound 和否定子句的代码示例；组合不会改变同名成员的消歧规则。
