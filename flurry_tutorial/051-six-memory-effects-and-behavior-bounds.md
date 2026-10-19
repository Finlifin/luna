# Flurry 演进手记 (051)：给每次内存触碰记账——六种内存效果（read/write/alloc/free...）与行为上界

在 [第 003 篇](./003-let-does-not-freeze-your-object.md) 中，我们见识了 Flurry 的一条颠覆性设计：
**`let` 只焊死绑定名称（禁止整值重赋），绝不冻结对象内部的字段；而且指针只写一种 `*T`，没有把指针分裂成 `*const T` 和 `*mut T` 两套类型！**

当时，很多写惯了 Rust（`&T` vs `&mut T`）或 C++（`const T*` vs `T*`）的读者心里都有一个巨大的问号：
> “如果指针类型都叫 `*T`，当我把环形缓冲区的指针 `buf: *RingBuffer` 传给一个统计函数 `inspect(buf)` 时，我怎么保证 `inspect` 只是老老实实**读取**了 `buf`，而没有在里面偷偷**修改（Write）**某个字段、没有**销毁（Kill/Free）**它的内存、也没有偷偷往控制台**打印（System）**东西？”

而在 [第 049 篇](./049-memory-views-fields-and-intervals.md) 和 [第 050 篇](./050-disjointness-evidence-and-in-place-mutation.md) 中，我们已经窥见了冰山一角：通过内存视图与 `where outcomes ...` 子句！

今天，我们就来彻底掀开这套机制的引擎盖，看看 Flurry 是如何用 **六种内存效果（Memory Effects）** 与 **行为上界（Behavior Upper Bound）**，给每一次内存触碰和资源流转建立一本铁面无私的“会计总账”的！

---

## 第一版：靠“君子协定”或粗粒度 `const` 指针的尴尬（v0）

假设我们在写环形缓冲区的辅助模块时，有一个指针包装盒 `PointerBox { pointer: *i32 }` 和一个条件清零函数 `set_if_zero(target: *i32, value: i32)`。

在没有精细内存效果契约的传统语言里（或者当我们完全不写契约、也不靠分析器检查上界时）：
- 如果你用 C++ 的 `const PointerBox* self`：语法上的 `const` 只锁住了 `self->pointer` 这个指针槽本身不能改，但你通过 `self->pointer` 拿出来的 `int*` 依然可以肆无忌惮地把外面的 `int` 改掉（Shallow Const）！
- 如果你用 Rust 的 `&PointerBox`：借用规则又走向另一个极端——把整棵可达对象树全锁成只读（Deep Immutability），你想写个“只修改 `self.right`、绝不碰 `self.left`”的方法签名都写不出来！

让我们先写出不加显式效果上界约束的 `v0` 代码，看看我们真正需要向调用方回答哪些问题：

```flurry
use std.prelude.*;

-- v0：当 target 指向的值为 0 时，才将其改写为 value
fn set_if_zero_v0(target: *i32, value: i32) {
    if target.* == 0i32 {
        target.* = value;
    }
}

struct PointerBox {
    pointer: *i32,

    -- v0：读取 self.pointer 字段里存的指针并返回，此时还没有解引用那个 i32！
    fn pointer_value_v0(*self) -> *i32 {
        self.pointer.*
    }
}

test {
    var num = 0i32;
    set_if_zero_v0(num.ref, 42i32);

    let holder = PointerBox { pointer: num.ref }
    let ptr = holder.pointer_value_v0();
    println("{}", ptr.*);
}
```

看着上面这段简单的 `v0` 代码，站在编译器和调用者的视角，有四个灵魂问题必须分清：
1. 在 `set_if_zero_v0` 里，它既读取了 `target`，又可能写入 `target`，但它**绝不会释放/销毁（Kill）** `target` 指向的存储世代！
2. 在 `pointer_value_v0` 里，它只读取了 `self.pointer` 这个存放指针的字段槽，**并没有读取 `num` 本身**；但它返回的指针 `ptr` **依然携带对 `num` 的存活依赖（Connect）**！
3. 为什么平时我们写 `set_if_zero_v0` 不用自己手写一长串效果标注，编译器也能自动算清楚？
4. 当我们想给某个接口立下军令状：“**这个函数最多只准读 `target`，绝不准写！**”时，该怎么写？

---

## 第二版：揭秘 Flurry 的六种内存效果与 `outcomes` 行为上界（v1）

翻开权威文档 `docs/resource-management/memory-effects.md`，Flurry 将程序对内存、资源和外部世界的一切影响，精确归纳为 **六种基础效果（Six Memory Effects）**：

```text
  Flurry 的六种内存与行为效果（The 6 Memory Effects）：

  ┌─────────────┬──────────────────────────────────────┬────────────────────────────────────────┐
  │ 效果名称    │ 精确描述的行为                       │ 注意：它【不】自动提供的保证           │
  ├─────────────┼──────────────────────────────────────┼────────────────────────────────────────┤
  │ 1. read     │ 读取指定视图区域 (read<V>)           │ 目标存活、已初始化仍需先满足！         │
  │ 2. write    │ 修改指定区域，使受影响旧内容事实失效 │ 不释放对象，也不使其他合法别名指针失效 │
  │ 3. kill     │ 终结指定区域的当前生命周期世代(如释放)│ 不递归析构所有保存了该地址的其他对象   │
  │ 4. connect  │ 目标保留来自源区域的存活/可达性依赖  │ 不取得源的所有权，不使已失效的源复活   │
  │ 5. move     │ 转移值、访问要求与仿射(Once)清理责任 │ 不自动终结其拥有的外部分配             │
  │ 6. system   │ 外部可观察事件（如 println、同步等） │ 不替代真实的 read/write/kill 逃逸契约  │
  └─────────────┴──────────────────────────────────────┴────────────────────────────────────────┘
```

仔细品味这张表！你会发现 Flurry 为什么能把内存安全看得如此通透：
- **区分 `write` 与 `kill`**：在 `set_if_zero` 里，修改 `target.* = value` 只是 `write<target>`。它只撤销“旧数值等于 0”的内容事实，但 `target` 所在的存储世代依然活着，其他指向 `target` 的指针依然合法！只有当一块内存被释放或离开作用域销毁时，才会触发 **`kill`**（终结当前世代，让旧世代指针彻底失效）。
- **区分 `read` 与 `connect`**：在 `holder.pointer_value()` 中，读取 `self.pointer` 字段只是 `read<self.pointer>`，并没有去读外面的 `number: i32`；但返回出来的 `*i32` 通过 **`connect`** 效果，把对 `number` 的可达性依赖传递给了返回值！
- **自动推导 vs 显式 `outcomes` 上界**：
  - 当你**不写** `where outcomes ...` 时：编译器会自动扫描函数体、调用链和隐式 `Drop`，推导出最精确的默认效果摘要！所以日常写内部函数时**零样板代码負擔**！
  - 当你**显式写** `where outcomes ...` 时：它就成了不可逾越的**行为上界（Behavior Upper Bound）**！比如在 `set_if_zero` 中，当 `target.* != 0` 时其实只发生了 `read`，当 `target.* == 0` 时发生了 `read` 和 `write`——两者合起来的行为上界就是 `read<target> and write<target>`！

甚至，你还可以用 **`predicate`（程序谓词，返回 `MemoryEffect`）** 把常用的效果组合封装成一个好读的名字，比如 `modify(target)`！

> **🔍 语法小贴士：编写返回 `MemoryEffect` 的 `predicate`**
> 在 Flurry 中，用于程序验证的谓词使用关键字 **`predicate`** 声明（注意：**不要加 `comptime`**！因为程序状态与内存效果属于程序验证层，不属于编译期类型命题 PTC）。
> 当 `predicate` 的返回类型是 `-> MemoryEffect` 时，它的参数用来接收“内存区域表达式”（使用 `quote target: lift Any` 或直接写裸参数名简写），并在函数体里用 `and` 组合多个效果足迹：
> ```flurry
> predicate modify(quote target: lift Any) -> MemoryEffect {
>     read<target> and write<target>
> }
> ```

让我们用 `outcomes` 和自定义效果谓词 `modify` 写出精准无比的 `v1` 版本：

```flurry
use std.prelude.*;

-- 定义一个可复用的内存效果规格谓词 modify：表示允许读取和修改目标区域
predicate modify(quote target: lift Any) -> MemoryEffect {
    read<target> and write<target>
}

-- v1：用 `where outcomes modify(target)` 确立行为上界
fn set_if_zero(target: *i32, value: i32)
where outcomes modify(target)
{
    if target.* == 0i32 {
        target.* = value;
    }
}

struct PointerBox {
    pointer: *i32,

    -- 只读取存放指针的字段槽 `self.pointer`，不读取其指向的目标 i32；
    -- 但返回的 *i32 依然通过 connect 保留对目标对象的存活依赖！
    fn pointer_value(*self) -> *i32
    where outcomes read<self.pointer>
    {
        self.pointer.*
    }
}

test {
    var number = 0i32;
    set_if_zero(number.ref, 6i32);

    let holder = PointerBox { pointer: number.ref }
    let pointer = holder.pointer_value();
    -- 直到这里解引用 pointer.* 时，才真正读取 number！
    println("{}", pointer.*);
}
```

运行输出：

```text
6
```

---

## 第三版：编译器如何抓出“越界写入”与“藏在 `Drop` 里的暗箭”（v2）

有了 `where outcomes` 行为上界，我们就拥有了比 `const` 指针强大百倍的契约武器！

### 1. 宣称只读 `read<target>`，却敢偷偷写内存？当场拒绝！

如果一个函数在签名上立下军令状：`where outcomes read<target>`（只读 `target`），结果函数体里却偷偷写了一行 `target.* = 0`：
编译器**绝不会**为了迁就你的代码而悄悄把 `write<target>` 补进声明里，而是当场报错拒绝编译：

```flurry
-- ❌ 编译期直接拒绝！
-- 错误：函数体中的写入操作 `target.* = 0` 超出了显式声明的只读上界 `outcomes read<target>`！
fn wrong_read_only(target: *i32)
where outcomes read<target>
{
    target.* = 0i32;
}
```

### 2. 函数体一行代码没写，但局部变量的 `Drop` 偷偷改了外部内存或打了日志？照样入账！

这是 Flurry 内存效果分析最让人震撼的地方（详见 `docs/resource-management/memory-effects.md` §清理效果属于函数行为）：
假设有人写了一个看起来人畜无害的函数 `mark_on_exit(p: *bool)`：

```flurry
struct Watch {
    target: *bool,
}

impl Drop for Watch {
    fn drop(*self) {
        -- 在析构函数里通过保存的指针修改了外部存储！
        self.target.*.* = true;
    }
}

fn mark_on_exit(p: *bool) {
    let watch = Watch { target: p }
    -- 函数体结束了！表面上一行赋值都没写！
}
```

如果一个简陋的分析器只扫描函数体里的显式赋值语句，就会误以为 `mark_on_exit` 是一个什么都没改的纯洁函数！
但在 Flurry 中，**所有退出路径上的隐式 `Drop` 清理，都会被完整映射回实参的来源位置，并计入函数的内存效果摘要！**

让我们画一张 **Code Cartoon**，看看编译器的内存效果记账员是怎么顺着 `Drop` 和私有边界查账的：

```text
  情况 A：局部变量 `Watch` 在 `Drop` 中通过指针修改外部参数 `p`
  fn mark_on_exit(p: *bool) {
      let watch = Watch { target: p }
  } ──► [退出作用域触发 Watch.drop] ──► 执行 `self.target.*.* = true`
    ──► 记账员顺着字段来源追溯：`self.target` 指向调用方传入的形参 `p`！
    ──► 最终边界账单：包含 `write<p>`！（同样，若 Drop 里有 println，账单包含 `system`！）

  ───────────────────────────────────────────────────────────────────

  情况 B：纯局部私有变量的修改（不逃逸！）
  fn flip_local() -> bool {
      let value = false;
      flip(value.ref) -- flip 内部有 write<pointer>，但 pointer 指向的是局部私有的 value！
  } ──► 记账员到达 `flip_local` 的函数返回边界：
    ──► 发现 `value` 是本次调用新建、未逃逸的私有局部存储！
    ──► 最终对调用者的边界账单：隐藏私有动作，对外完全无内存写入副作用！
```

让我们写一段 `v2` 测试代码，亲自观察“隐式 `Drop` 的写入与输出效果”以及“局部私有存储的边界隐藏”：

```flurry
use std.prelude.*;

struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        -- 产生 `system` 输出效果
        println("drop {}", self.id.*);
    }
}

struct Watch {
    target: *bool,
}

impl Drop for Watch {
    fn drop(*self) {
        -- 产生对 target 指向位置的 `write` 效果
        self.target.*.* = true;
    }
}

-- 虽然函数体只构造了局部绑定，但其退出清理包含 `system` 输出效果
fn quiet_body() {
    let value = Trace { id: 7u32 }
}

-- 虽然函数体没有显式赋值，但退出时 Watch.drop 会将 p.* 改写为 true！
fn mark_on_exit(p: *bool) {
    let watch = Watch { target: p }
}

-- 辅助函数：有读写指针的效果
fn flip(pointer: *bool) -> bool {
    pointer.* = not pointer.*;
    pointer.*
}

-- 在 local_result 内部修改了局部变量的指针，但在返回边界隐藏了私有存储访问！
fn local_result() -> bool {
    let value = false;
    flip(value.ref)
}

test {
    quiet_body();

    var flag = false;
    mark_on_exit(flag.ref);
    println("flag after mark_on_exit: {}", flag);

    println("local_result: {}", local_result());
}
```

运行输出：

```text
drop 7
flag after mark_on_exit: true
local_result: true
```

如果在 `mark_on_exit` 前面我们原本假设了 `flag == false`，一旦调用完 `mark_on_exit(flag.ref)`，编译器因为早已把 `Watch.drop` 里的 `write<p>` 记入了 `mark_on_exit` 的摘要，就会自动作废 `flag == false` 的旧事实，绝不会发生误判！

---

## 本篇小结

通过本篇的学习，我们彻底掌握了 Flurry 的六种内存效果与 `outcomes` 行为上界：

| 特性 / 效果 | 核心作用 | 关键边界与防呆规则 |
| :--- | :--- | :--- |
| **`read` / `write` / `kill`** | 分别刻画读取、修改内容、以及终结存储世代 | `write` 不终结世代；`read` 与 `kill` 同时出现在上界里绝不意味着允许“先 `kill` 再 `read`” |
| **`connect` / `move` / `system`** | 刻画指针依赖传播、责任转移、以及 I/O 等外部事件 | 读取指针字段 `read<self.ptr>` 通过 `connect` 保留对目标的存活依赖 |
| **默认自动推导** | 省略 `outcomes` 时由编译器根据实际操作与 `Drop` 自动合成 | 日常编码零样板负担；自动隐藏未逃逸的局部私有存储操作 |
| **显式 `outcomes` 上界** | 限制函数允许发生的最大行为范围（可配合 `predicate` 封装） | 实现若超出上界（包括隐式 `Drop` 中的写入或输出），编译期直接拒绝 |

### 留给下一篇的悬念

现在，我们已经能用 `where outcomes ...` 精确限制一个函数“最多能读哪里、写哪里”了。
但是，仔细想一想：
> **“光知道一个函数的效果上界（`outcomes`），就足够了吗？”**

举两个让你冷汗直冒的例子：
1. **例子一**：两个函数 `fn_a(p: *u32)` 和 `fn_b(p: *u32)`，它们的行为上界一模一样，都是 `outcomes read<p> and write<p>`。
   但是，`fn_a` 是**先读旧值、再写入新值 `9`**（最后 `p.* == 9`）；而 `fn_b` 是**先写入 `9`、再写入 `0`**（最后 `p.* == 0`）！如果只看无序的效果集合 `{read, write}`，调用者怎么知道调用完成后 `p.*` 到底变成了几？
2. **例子二**：一个函数 `try_push(buf: *RingBuffer, item: i32) -> !PushErr void` 可能成功，也可能因为缓冲区满了返回 `error PushErr.Full`。
   当它**成功返回**时，`buf.len.*` 增加了 1；当它**报错返回**时，`buf.len.*` 保持原样没变！如果把成功出口和错误出口的状态混为一谈，调用者处理完错误后，怎么知道原有的缓冲区长度不变量还成不成立？

在下一篇 **《Flurry 演进手记 (052)：函数退出时留下了什么——出口状态契约与跨调用事实保持》** 中，我们将把 `requires`、`outcomes` 与 `ensures` 串联成一个完整的状态转移闭环，揭开跨调用事实保持（Frame Rule）的工程奥秘！
