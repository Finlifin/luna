# 语法对象（`meta.ast`）

为构造宏与源码元编程提供未类型检查的词法/语法表示；保留完整 `SourceSpan` 来源与扩展溯源链，并明确区分**语法对象**（此处）与**已类型检查的 quoted code**（归属 `meta.code`）。

## 导出接口

- **位置与 token 序列**：
  - `SourceSpan`（文件编号、字节起止区间、宏展开 ID）
  - `Token`（`.id` / `.int` / `.string` / 运算符等 `.kind`、`text`、`spacing` 与 `span`）与常用构造函数（`Token.ident`、`Token.symbol`、`Token.literal_int`、`Token.punct` 等）
  - `TokenBuffer`（支持值引用更新 `push`、`append`、`concat`、`join`、`to_list` 等）
  - `ToTokens` comptime 协议及其标量/`Token`/`TokenBuffer`/`List`/`Optional`/`Flurry*`/`PegNode` 实现，支撑 `{{ ... }}` 内的 `$name` 与 `${expr}` 插值
- **通用与 typed AST 包装**：
  - `SyntaxNode`、`SyntaxTree`
  - 10 种 Flurry 语法片段包装类型：`FlurryIdent`、`FlurryLiteral`、`FlurryPath`、`FlurryExpr`、`FlurryType`、`FlurryPattern`、`FlurryStatement`、`FlurryExtendedStatement`、`FlurryBlock`、`FlurryItem`
- **PEG 语法树与量词容器**：
  - `PegTreeNode`、`PegNode`（`.rule()`、`.span()`、`.text()`、`.children()`、`.tokens` 与 `ToTokens`）
  - `Repetition<T>`（`meta.List<T>` 别名）与 `Optional<T>`（`?T` 别名）
