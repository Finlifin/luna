# attrs.derivation

编译器预展开默认派生选择消费 `{ derivation_default: bool }` 属性。规范入口
`attrs.derivation.default` 的导出尚待允许在成员位置声明 `default`（当前在
`docs/basics/lexical.md` 与 `compiler/parse/tests/derivation.rs` 中为预留关键字）
的语法决定，因此这里暂不以变通别名替代。
