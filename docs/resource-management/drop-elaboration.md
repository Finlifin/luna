# Drop 精化

Flurry 通过 RAII（Resource Acquisition Is Initialization）原则和编译器的**Drop 精化（Drop Elaboration）**实现自动化、确定性的资源管理。

## RAII 原则

资源的生命周期与持有它的值的生命周期绑定：

- **获取即初始化**：`File.open(...)`、`Vec.new()` 等调用在创建值时同时获取资源。
- **析构即释放**：当值的所有权结束时，其持有的资源必须被释放。

## `Drop` Trait

需要自定义清理逻辑的类型实现 `Drop` Trait：

```flurry
trait Drop {
    fn drop(*self);
}

impl Drop for MyFileWrapper {
    fn drop(*self) {
        unsafe { close_file_handle(self.handle) }
    }
}
```

不实现 `Drop` 的类型，其 drop 操作默认为递归 drop 各字段（如果字段实现了 `Drop`）。

## 编译器自动插入 `drop`

开发者通常**不需要手动调用 `drop`**。编译器会在每个值的所有权结束处自动插入 drop 调用。

**离开作用域时**：

```flurry
{
    let file = File.open("temp.txt")!;
    -- ... 使用 file ...
}   -- 编译器在此处自动插入 drop(file)
```

**所有权转移后**：原绑定失去所有权，编译器不会在其离开作用域时为其插入 drop；drop 责任随所有权一起转移：

```flurry
fn process(f: File) {
    -- ...
}   -- 编译器在此处插入 drop(f)

let my_file = File.open("data.log")!;
process(my_file);   -- 所有权转移给 f
-- my_file 离开作用域，但所有权已移走，不会 drop
```

**控制流分支**：编译器分析所有分支，确保每条路径上都正确 drop：

```flurry
fn example(use_it: bool) {
    let data = allocate_resource();
    if use_it {
        consume(data);    -- data 在这里被消耗
    }
    -- 若 use_it 为 false，编译器在函数结束处自动插入 drop(data)
}
```

## 优势

- **自动化**：无需手写资源释放，减少泄漏风险。
- **确定性**：释放时机由作用域和所有权规则静态决定，而非 GC。
- **及时性**：资源在不再需要时立即释放，适合文件锁、网络连接等需要精确生命周期的场景。
