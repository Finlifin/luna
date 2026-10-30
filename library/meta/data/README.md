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

## 相邻项去重与有序性检查

`values.dedup_by(equivalent)` 保留每个相邻组的第一个元素。比较器接收
“最后保留的元素、当前候选元素”，每个候选恰好比较一次；被移除的元素不会
替换组代表。非相邻的重复项仍保留，需要全局去重时可先按相应顺序排序。
比较器也可以表达非传递的分组条件，此时结果严格遵循上述代表选择规则。

`values.is_sorted_by(less)` 使用与 `sort_by` 相同的严格比较器，依次检查
`less(当前项, 前一项)`，第一次返回 true 时立即判定无序，不再调用后续比较。
相等元素可以连续出现。空列表、单元素列表不会调用比较器，且都视为有序。

```flurry
comptime fn distinct_sizes() -> List<Integer> {
    let sizes: List<Integer> = [8, 4, 8, 4, 1];
    let sorted = sizes.sort_by(|left, right| left < right);
    sorted.dedup_by(|left, right| left == right)
    -- [1, 4, 8]
}
```

两种操作都先取得输入快照，回调副作用不改变本次遍历范围。错误原样传播，
不会作为比较结果使用；特别是 Type 相等性求解得到 Unknown 时，不得把它
当作不等而继续去重。保留的 Type 元素不会展开 normal-form 头。

`is_sorted_by` 至多比较 n−1 次；`dedup_by` 对非空输入恰好比较 n−1 次。
去重输出仍复用现有 `append` 的完整表示检查，不承诺线性构造成本。
真实 sysroot 用例位于 `development/examples/sysroot-meta-lists.fl`，同时覆盖
有状态比较器、输入快照、排序组合与不调用回调的边界。
