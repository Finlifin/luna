# 064 — code-organization — 06

- 日期：2026-09-28
- global_index：064
- theme：code-organization
- subindex：06
- 本轮目标：在文档审阅中纠正将 normal form 展开为函数返回值的错误示例，并同步相关语义说明。

## 作者输入

作者引用了 `docs/functions/comptime_fn.md` 中的错误示例。以下为被纠正的内容，不是合法程序的推荐写法：

```flurry
pure comptime fn Identity<T> -> Type {
    T
}

typealias Number = Identity<u32>;

test {
    let number: Number = 9;
    println("{}", number);
}
```

作者明确纠正：

> 不！normal form是不会展开的，Identity<T>和T在normalized ir没有任何关系

## 评估与已确认结论

错误在于混淆范式应用与普通编译时执行，不是缺少一次优化或等待实参具体化。032 的作者答复已经区分尖括号保留的 normal form 与普通调用的执行语义；本轮进一步明确了不展开函数体的边界。

- 范式应用在 normalized IR 中保留头与实参。函数体返回 T 不建立 Identity<T> 与 T 的关系。
- typealias 只为目标提供另一个名字。Number 指向 Identity<u32>，不能据函数体继续化简为 u32。
- pure 标记、具体实参和 alias 都不构成展开范式或向类型等式层合并两项的理由。
- 普通 comptime 函数调用按执行语义取得结果；若要示范“执行函数后获得 u32，再定义其别名”，应同时采用普通函数参数形式与圆括号调用。
- 不从这次更正额外推导所有不同范式头之间的不等性、普遍单射性或新的转换机制。

本轮直接执行作者对已授权文档的纠正，没有需要拍板的新问题，也没有新增待定设计。

## 文档修改

1. `docs/functions/comptime_fn.md`：将返回类型值的示例改为普通 comptime 函数 `Identity(T: Type)` 和普通调用 `Identity(u32)`，保留输出 9 的测试意图。增加范式应用边界及 normalized IR 不得借函数体生成等式的引导。
2. `docs/comptime/purity-and-normal-form.md`：区分范式保留与普通执行；说明 alias 不展开目标。保留尖括号 Identity 示例，增加接收并交付同一范式类型的函数，避免把它用于无依据的数值初始化；补充保留项的实现步骤。
3. `docs/comptime/normalization-and-instantiation.md`：在定义与 normalization 算法中明确范式应用不走展开求值路径，具体实参不改变这条规则。
4. `docs/type/type-identity.md`：明确 alias 透明性只消除 alias；普通工厂按执行结果比较的规则与范式保留分开叙述。类型比较算法不得展开范式头来建立等式。
5. `docs/type/propositional-constraints/theory-solvers.md`：在 EUF 的准入边界中明确，不能由 Identity 的函数体向等式层加入 Identity<T> 与 T 的关系。

本轮未修改全局 syntax，因为普通调用和范式应用已有不同的产生式；纠正的是示例使用及语义。没有改写历史、草稿或编译器。

## 自检与验证

- 按 `docs/grammar/syntax` 与 `docs/grammar/index.md` 检查修改后的示例：普通函数定义与圆括号调用相配，范式定义与尖括号应用相配；返回类型值的函数使用 PascalCase；语句分号及大括号结尾规则符合约定。
- 被引用的原示例只用于记录作者指出的语义错误，不声称通过语义检查。
- 五份修改文档的 `git diff --check`、行尾空白、代码围栏和新增交叉链接检查通过。
- 本轮文件摘要核对确认编译器未变化。核对还发现 `docs/functions/higher-order-functions.md` 在工作期间有并发修改；该文件不在本轮补丁中，未覆盖或回退。
- 未运行编译器测试；当前编译器不作为这些语言语义的验证依据。
- prompt 仍为空；作者本轮直接在对话中纠正，没有需归档的 prompt 答复。

## 对作者的完整回复

`Number` 只能消解到 `Identity<u32>`，不能继续化简为 `u32`。函数体返回 `T` 不会在 normalized IR 中建立 `Identity<T>` 与 `T` 的关系，因此原示例的数值初始化没有依据。

已将[示例](/home/fin/luna/docs/functions/comptime_fn.md:104)改为普通 comptime 函数定义及圆括号调用，并同步修正范式、类型身份和 PTC 的说明。
