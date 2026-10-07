# 运行时依赖类型

## Intro

一个索引可以只保存整数，却必须在某个特定存储中解释。这种设计借鉴了依赖类型（dependent type）的思想，将值与其上下文的关系纳入类型层面的描述。类型记录这种依赖，操作时再从有效环境取得存储。

这使程序能够区分“这个整数属于哪个对象”和“这个整数本身占多少空间”。依赖运行时对象不意味着每个值都要附带一个指针，也不意味着值一定具有动态大小。

## 语法

~~~text
quote_decl_clause -> quote decl_clause
type_bound_decl_clause -> id: expr
lift_type -> lift expr
normal_form_head_application -> expr<argument*>
closure_type -> %expr expr
~~~

参数声明沿用全局语法。quote 参数保留允许的运行时依赖信息，不读取未来内容。

## 正式定义

### 依赖输入

类型依赖的输入要么在 comptime 可计算，要么是具有 identity 的 runtime 值。后者参与类型及操作环境，但不因被引用就成为编译时已知数据。

依赖对象必须在需要它的操作处可取得，并满足相应资源有效性要求。捕获依赖不会自动延长对象或底层分配的生命。

同一声明中两个依赖参数是否相等，按其 identity 契约判断；两个对象内容相同不够。对象移动、赋值和失效由资源契约约束，类型比较不能擅自用地址或当前内容代替。

### 本体与环境

值本体保存其数据；环境提供解释或操作这些数据所需的依赖。环境可以位于外层聚合、调用的隐藏参数、有效全局位置，或在需要时封闭进携带者。

存在环境依赖不强制改变每个元素的本体布局。若依赖只在操作时需要，且可从共同外层环境取得，多元素容器无需逐元素复制同一依赖。

若结果跨过原环境边界，必须仍有合法的环境取得方式；仅把类型文本中的参数抹成 any 不会创造它。

## 常见示例

用三个节点的小型 SOA 存储说明依赖关系。数值列与左子节点列分开保存；索引方法每次只读取所需的一列：

~~~flurry
struct NodeStorage {
    values: Array<u32, 3>,
    left_indices: Array<u32, 3>,
}

struct NodeIndex
where quote storage: lift *NodeStorage {
    index: u32,

    unsafe fn value(*self) -> ?u32 {
        let index = self.index.*;
        if index < 3 {
            storage.*.values[index.as(usize)]
        } else {
            null
        }
    }

    unsafe fn left(*self) -> ?Self {
        let index = self.index.*;
        if index < 3 {
            let child = storage.*.left_indices[index.as(usize)];
            if child < 3 {
                Self { index: child }
            } else {
                null
            }
        } else {
            null
        }
    }
}

test {
    let storage = NodeStorage {
        values: [10, 20, 30],
        left_indices: [3, 0, 1],
    }
    let root = NodeIndex<storage.ref> { index: 2 }

    unsafe {
        root.value() match {
            value? => println("root value: {}", value),
            null => println("invalid root"),
        }

        root.left() match {
            child? => {
                child.value() match {
                    value? => println("left value: {}", value),
                    null => println("invalid child"),
                }
            },
            null => println("no left child"),
        }
    }
}
~~~

输出为 `root value: 30`、`left value: 20`。本例约定编号 `3` 表示没有子节点；这是应用自己的编码约定。方法先检查编号，再访问数组。`value` 不读取左子节点列，`left` 不读取数值列。

`NodeIndex` 本体只有 `index`。`storage` 出现在类型参数及操作环境中；`left` 返回的 `Self` 继续携带同一个存储依赖，不会变成可用于任意存储的整数包装。本例把底层存储访问集中在 unsafe 方法中，并在存储有效的同一作用域内调用。

下面用两个内容相同的对象展示 identity 边界：

~~~flurry
test {
    let first_storage = NodeStorage {
        values: [10, 20, 30],
        left_indices: [3, 0, 1],
    }
    let second_storage = NodeStorage {
        values: [10, 20, 30],
        left_indices: [3, 0, 1],
    }

    let first = NodeIndex<first_storage.ref> { index: 2 }
    let second = NodeIndex<second_storage.ref> { index: 2 }

    unsafe {
        first.value() match {
            value? => println("first: {}", value),
            null => println("invalid"),
        }

        second.value() match {
            value? => println("second: {}", value),
            null => println("invalid"),
        }
    }
}
~~~

两次都输出数值 `30`，但两个索引分别依赖不同存储。把 `first` 直接交给要求 `NodeIndex<second_storage.ref>` 的位置，不能由内容相同取得类型相等依据。

这个实现把所有列固定为三个元素，使例子可以完整展示而不依赖容器库。扩大存储、生成列和封装访问接口可以继续使用同一依赖关系；存储移动、扩容与释放的有效性仍须遵循相应资源契约。

## 与 Rust / Zig / C / C++ 的关键差异

C 的句柄加上下文指针、C++ 的索引包装和显式环境是实际替代手段；Rust 可以用类型与资源协议编码部分关系，Zig 可以用显式参数携带上下文。Flurry 允许运行时对象 identity 进入依赖描述，不能用静态整数泛型参数完整类比。[Rust 泛型参数](https://doc.rust-lang.org/reference/items/generics.html)

## 运行时实现设计引导

为每种依赖操作建立环境取得计划：

1. 列出操作确实使用的依赖。
2. 从当前聚合、调用参数或合法全局位置取得它们。
3. 必须跨边界传递时，建立对应封闭环境。
4. 将依赖与值本体共同交付给选定操作。
5. 在资源契约要求的位置验证有效性。

同一外层环境可以服务多个值；消除逐值冗余不应改变类型区分或让值逃逸后失去依赖。隐藏参数属于实现传递方式，不自动成为用户可见的额外字段。

## 编译器实现引导

类型采用两层表示：底层为可 intern 的不可变带洞类型结构，引用端附带 substitution，将槽位映射到相应类型、值或 runtime 依赖项。模板洞不是本次推断的可变元变量；同一模板上的不同替换也不自动构成相同类型。

环境取得计划与类型依赖身份分别记录。IIR 中类型的 comptime 部分已完成，合法 runtime 依赖由其替换及绑定环境解释。构造、无捕获替换、比较、缓存和交付算法见[运行时依赖类型的表示](../compilation/runtime-dependent-types.md)。

分别记录本体表示依赖与操作环境依赖，不能把所有类型依赖都加入 sizeof 图。

对每个使用点形成“需要依赖—可取得依赖”的对应表。环境合成按依赖顺序验证绑定；无法交付的依赖报告其原始来源及跨越边界。

实例化保留 runtime identity，不把它简化为当前地址常量。可共享环境的优化先证明所有相关值使用同一个依赖；不能仅凭同名参数合并。

验收覆盖：两个不同存储的同值索引不可混同、共享环境不增逐元素指针、本体固定大小与运行时依赖共存、逃逸后仍需环境、访问单列不强制加载整节点。
