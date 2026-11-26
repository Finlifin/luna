# 语法组合与构造宏（`meta.macros`）

提供 Flurry 构造宏的标注属性、定义入口及配套语法类型再导出；类型检查后的 quoted code 组合归属 `meta.code`。

## 导出接口与四种输入模式

- **属性标注**：
  - `macro3`（Mode 3，`^macro3`）：接收并输出 `TokenBuffer` 的自由 token 构造宏
  - `macro4`（Mode 4，`^macro4`）：接收词法分析前原始文本 `str` 并输出 `TokenBuffer` 的源文本构造宏
- **内联构造器**：
  - `@define_macro1`（Mode 1 AST）：按处理器参数类型解析固定、可选（`?T`）或变长（`...name: Repetition<T>`）Flurry AST 片段
  - `@define_macro2`（Mode 2 PEG）：声明无左递归的 PEG 文法（以 `Entry` 为入口，支持序列表达 `~`、有序选择 `|`、前瞻 `&`/`!`、重复 `*`/`+`/`?` 及 `Flurry*` 片段），并将匹配结果投影为 `PegNode` 或对应 AST 片段
- **配套再导出**：`ast`、`peg`、`Repetition`、`Token`、`TokenBuffer`、`ToTokens`
