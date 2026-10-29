# 编译期数据

list.fl 将列表操作 impl 到 meta.List；object.fl 提供有序 Object 查询与编辑；integer.fl 提供编译期整数操作。List 的编译器绑定 normal-form 头仍在包根，避免更改公开 meta.List 身份。

## 前缀累积

`values.scan(initial, combine)` 返回依次处理每个元素后的状态；
`values.scan_exclusive(initial, combine)` 返回处理每个元素前的状态。
两者都产生与输入等长的列表，并从左到右对每个元素调用一次 comptime 回调。
空输入返回空列表，不调用回调；exclusive 的最后一次回调也会执行，错误和副作用不会被丢弃。

```flurry
use meta.List;

comptime fn offsets() -> List<Integer> {
    let sizes: List<Integer> = [4, 8, 1];
    sizes.scan_exclusive(0, |offset, size| offset + size)
    -- [0, 4, 12]
}

comptime fn ends() -> List<Integer> {
    let sizes: List<Integer> = [4, 8, 1];
    sizes.scan(0, |offset, size| offset + size)
    -- [4, 12, 13]
}
```

累积状态的类型可以与元素类型不同，例如将 `List<Type>` 扫描为一系列逐步增长的
`List<Type>`。输入在开始遍历时取值快照；回调改写调用者的列表不会改变本轮遍历。
输出按普通值快照规则保存，替换累积值不会改写之前的结果；不额外深拷贝指针指向的存储。`Type` 元素保留其
normal-form 身份，不展开生成函数的头。

当前实现复用 `append`，每次追加仍进行完整表示检查；不承诺线性构造成本。
