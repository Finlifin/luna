# 指针与引用

Flurry 通过**指针**和**切片**提供非所有权式的数据访问，在不转移所有权的前提下传递和使用数据。

## 指针（`*T`）

`*T` 是指向类型 `T` 的值的指针。

**获取指针**：使用后缀 `.ref` 操作符：

```flurry
let data = MyData { ... }
let ptr: *MyData = data.ref;
```

**解引用**：使用后缀 `.*` 操作符：

```flurry
let value = ptr.*;             -- 解引用
let field = ptr.some_field;   -- 通过指针访问字段（自动解引用）
ptr.some_method();            -- 方法调用自动解引用
```

**安全性**：

- 通过 `.ref` 在 `normal` 代码中创建的指针受**可达性分析**约束，编译器保证指针在其使用期间不指向已释放的内存（防止悬垂指针）。
- 指针算术、类型转换等底层操作属于 `unsafe` 范畴。

## 切片（`Slice<T>`）

`Slice<T>` 是对内存中连续 `T` 类型元素序列的视图，由（指针，长度）两部分构成。切片本身不拥有数据：

```flurry
let array = [1, 2, 3, 4, 5];
let full:    Slice<i32> = array.slice(..);   -- 整个数组的切片
let partial: Slice<i32> = array.slice(1..4); -- 索引 1..3 的切片
```

切片适用于缓冲区处理、字符串视图等场景，允许函数操作数据子集而无需复制或转移所有权。

## "借用"的概念

Flurry 没有像 Rust 的借用检查，但语义是相似的：

- `.ref` 和 切片**不转移所有权**，原始所有者仍负责数据的生命周期。
- 引用的有效期**不能超过**其所指向数据的生命周期。
- 这一约束由**可达性类型系统**和**副作用系统**在编译期静态强制执行。

```flurry
fn use_ref(ptr: *MyData) {
    -- 通过 ptr 访问数据，但不拥有它
}

fn main() {
    let data = MyData { ... }
    use_ref(data.ref);    -- 传递引用，不转移所有权
    use_ref(data.ref);    -- 可以再次传递
    consume(data);        -- 现在转移所有权
    -- use_ref(data.ref); -- 编译错误：data 已被移走
}
```

详细的可达性分析规则见 [type/](../type/)。
