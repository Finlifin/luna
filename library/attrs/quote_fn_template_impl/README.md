# attrs.quote_fn_template

`attrs.quote_fn_template` 为用于 match arm 组装的无体 `template quote fn` 提供官方属性：

- `attrs.quote_fn_template.for_match_arms`：产生 `{ __flurry_quote_fn_template_for_match_arms: true }`，标记目标 `template quote fn` 声明作为 quoted match arm 规则集合的接入接口，可由 `match` 中的 `use expr` 导入，并由 `quote fn name(case pattern) use template { ... }` 贡献规则臂。
- `attrs.quote_fn_template.allow_non_exhaustive_instance`：产生 `{ __flurry_quote_fn_template_allow_non_exhaustive_instance: true }`，允许附着于同一 `for_match_arms` 模板的单条 `case` 实例本身非穷尽；最终 `match` 仍必须穷尽或提供合法失败路径。
