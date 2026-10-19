# Flurry 演进手记 (047)：追踪指针从哪里来、到哪里去——可达性限定类型 &ReachabilitySet T 入门

在 [上一篇 (046)](./046-pointer-identities-generations-and-sequential-aliasing.md) 中，我们了解了 Flurry 的指针目标身份与三维有效性模型。在同一个函数内部，编译器可以顺着每一行代码清清楚楚地追踪每个指针指向哪个局部槽位。

但是，当我们把 `RingBuffer` 拆成多个独立的函数和模块时，问题来了：
假设我们要写一个辅助函数 `select_longer`，它接收两个来自不同缓冲区的字符串视图 `first: str` 和 `second: str`，比较它们的长度，然后把较长的那个 `str` 返回给调用者。

如果调用者写下：

```flurry
let chosen = select_longer(buf_a.as_str(), buf_b.as_str());
```

由于 Flurry 采用模块化契约检查——**编译器在检查调用点时，只看 `select_longer` 的函数签名，绝不会每次都把 `select_longer` 的函数体源码搬过来重新展开分析一遍**！
那么，仅仅看着 `fn select_longer(first: str, second: str) -> str` 这个签名，编译器怎么知道返回的 `chosen` 到底依赖 `buf_a` 还是依赖 `buf_b` 呢？

在 C/C++ 中，函数签名对这件事完全是个哑巴，全靠人类自己读文档猜；
在 Rust 中，你需要在类型里塞进一套专门的生命周期撇号语法 `fn select_longer<'a>(first: &'a str, second: &'a str) -> &'a str`。但当结构体里既有 소유权对象又有多个不同来源的指针时，Rust 的 `'a, 'b, 'c` 很快就会在整棵类型树上传染蔓延。

Flurry 用一套更直接、更正交的机制解决了这个问题：**可达性限定类型（Reachability-Qualified Type）`&R T`**（详见 `docs/resource-management/reachability.md` 与 `docs/grammar/index.md`）！

---

## 第一版：别认错人！为什么类型前面的 `&` 不是“取地址”？（v0）

在正式认识 `&R T` 之前，我们先消除一个 C/Rust 程序员最容易产生的视觉误会。

还记得我们在第一卷反复强调过的 Flurry 语法黄金法则吗？
- **在普通值表达式中，偏爱后缀**：取地址一律写 **`expr.ref`**，解引用一律写 **`expr.*`**。在值表达式里，根本没有前缀 `&x` 这种写法！
- **在类型表达式中，偏爱前缀**：我们在前面已经见过了三种前缀修饰类型，今天我们将迎来第四位家族成员：

```text
  Flurry 的前缀限定类型家族（详见 `docs/grammar/index.md`）：
  ┌──────────────────────────┬────────────────────────────────────────────────┐
  │ 1. `?T`                  │ 可选类型 (Option Type)                         │
  │ 2. `!Errors T`           │ 错误限定类型 (Error-Qualified Type)            │
  │ 3. `#Effects T`          │ 效应限定类型 (Effect-Qualified Type)           │
  │ 4. `*T`                  │ 指针类型 (Pointer Type)                        │
  │ 5. `&ReachabilitySet T`  │ 可达性限定类型 (Reachability-Qualified Type)！ │
  └──────────────────────────┴────────────────────────────────────────────────┘
```

看清楚了吗？
当你看到 **`&R T`** 时，它**不是**“指向 `T` 的引用”（指针永远是 `*T`），而是给类型 `T` 挂上了一个**可达性限定前缀 `&R`**：
> **`&R T` 的含义是：这个类型为 `T` 的值（无论是指针 `*u8`、切片视图 `str`，还是内部藏着指针的结构体 `BufferView`），在使用时要求可达性集合 `R` 中的外部资源及相关事实保持存活有效！**

注意两条重要的边界规则（详见 `docs/resource-management/reachability.md`）：
1. **限定本身只声明“依赖要求”，不延长目标寿命**：给类型写上 `&R T` 是向调用者坦白“我依赖 `R`”，它不会像魔法一样替你把即将销毁的对象留在内存里，也不会自动授予对 `R` 的写入或释放许可。
2. **`T` 可以是任何含视图的类型**：哪怕 `T` 是一个深层嵌套的用户自定义结构体 `PacketView`，只要里面包含了指向外部资源的指针，你都可以在最外层直接用 `&R PacketView` 来描述它的外部依赖，而不必像在 Rust 里那样把 `'a` 侵入式地写进每一层结构体泛型参数里！

---

## 第二版：跨函数边界的依赖追踪——`&R1 &R2 T` 保守并集上界（v1）

现在，让我们用 `&R T` 来写出刚才那个 `longest`（或 `select_longer`）函数的完整签名（详见 `docs/resource-management/reachability.md`）：

```flurry
use std.prelude.*;

fn longest(first: &R1 str, second: &R2 str) -> &R1 &R2 str
where R1: Type.vars.ReachSet, R2: Type.vars.ReachSet,
    outcomes read(first) and read(second)
{
    if first.len() >= second.len() {
        first
    } else {
        second
    }
}
```

停下来，仔细端详这个函数的签名：
1. **`where R1: Type.vars.ReachSet, R2: Type.vars.ReachSet`**：声明了两个可达性集合变量 `R1` 和 `R2`，分别代表第一個参数 `first` 所依赖的底层资源集合，以及第二个参数 `second` 所依赖的底层资源集合。
2. **返回类型 `-> &R1 &R2 str`**：多个 `&R` 前缀连写在一起时，表示**合并（取并集）它们的可达性要求**！

让我们画一张 Code Cartoon，看看当调用者调用 `longest` 时，编译器是怎么利用 `&R1 &R2 str` 查账的：

```text
  调用端环境：
  let buf_a = "ring_buffer_alpha".to_string();  ──► 资源身份: Alloc_A
  let buf_b = "beta".to_string();               ──► 资源身份: Alloc_B

  调用：
  let result = longest(buf_a.as_str(), buf_b.as_str());
                       ──────────────  ──────────────
                         R1 = {Alloc_A}  R2 = {Alloc_B}
                                │              │
                                └──────┬───────┘
                                       ▼
                        返回值 `result` 的类型获得限定：
                        `&{Alloc_A} &{Alloc_B} str`
                        (即：只要还会使用 `result`，`Alloc_A` 和 `Alloc_B` 都必须保持有效！)
```

> **💡 核心心智模型：为什么 `&R1 &R2 str` 是“要求的上界”而不是“同时指向两块内存”？**
> 也许你会问：“在运行时，`longest` 明明只返回了 `first` **或者** `second` 其中的一个，怎么会变成 `&R1 &R2 str` 呢？”
> 这正是静态类型系统**保守上界（Conservative Upper Bound）**的智慧：
> - 因为在编译期，编译器不展开函数体、也不知道运行时 `first.len() >= second.len()` 究竟是真是假，所以为了保证绝对安全，函数签名用 `R1` 和 `R2` 的**并集**作为返回值依赖的上界；
> - 它**并不是声称**返回的那个 `str` 指针同时劈叉指向了两块字符内存，而是告诉调用者：“为了安全使用这个返回值，请确保 `first` 的来源 `R1` 和 `second` 的来源 `R2` 此刻都还活着！”
> - 同时，编译器规定：函数的实际依赖要求可以向更大的保守上界扩展（比如只返回 `first` 的函数也可以声明返回 `&R1 &R2 str`），但**绝不允许在没有依据的情况下删掉实际仍然需要的来源**！

来看看如果调用者在拿到 `result` 之后、使用 `result` 之前，不小心把其中一个来源 `buf_b` 给改坏（扩容重分配）或移走了，编译器会怎么拦截：

```flurry
-- ❌ 编译期直接拒绝！
fn broken_caller() {
    let buf_a = "ring_buffer_alpha".to_string();
    let buf_b = "beta".to_string();

    -- result 携带了 &{buf_a} &{buf_b} 的可达性要求
    let result = longest(buf_a.as_str(), buf_b.as_str());

    -- 破坏了 buf_b 的字符分配世代！
    buf_b.push_str("_reallocated_and_invalidated");

    -- 错误：使用 result 要求 buf_a 和 buf_b 的原字符分配均保持有效，
    -- 但 buf_b 的原分配已在上一行失效！
    println("{}", result);
}
```

而在正确保持两个来源存活的代码中，调用则畅通无阻：

```flurry
test {
    let buf_a = "ring_buffer_alpha".to_string();
    let buf_b = "beta".to_string();

    let result = longest(buf_a.as_str(), buf_b.as_str());
    println("longest is: {}", result);
    assumes result == "ring_buffer_alpha";
}
```

---

## 第三版：分清“自身拥有的内部资源”与“指向外部的可达依赖”（v2）

在设计 `RingBuffer` 容器时，有一个极其重要的问题必须彻底厘清：
**为什么 `String` 或 `Vec<i32>` 内部明明也有指向堆内存的指针，但它们的返回类型却不需要写 `&R String` 或 `&R Vec<i32>`？而一旦变成 `Vec<*i32>`，就又需要追踪外部可达依赖了？**

让我们画一张 Code Cartoon，把**“自身拥有的内部资源（Owned Internal Resource）”**和**“外部可达性要求（External Reachability Requirement）”**放在显微镜下对比（详见 `docs/resource-management/reachability.md`）：

```text
  情况 A：`Vec<i32>` 或 `String`（纯拥有者 Owner）
  ┌──────────────────┐         ┌─────────────────┐
  │ Vec<i32> 描述符  │ ──拥有─►│ [ 10, 20, 30 ]  │ (堆缓冲区归 Vec 自己全权管理！)
  └──────────────────┘         └─────────────────┘
  ──► 当 `Vec<i32>` 按值移交时，堆缓冲区的寿命跟着 `Vec<i32>` 一起走，
      不依赖任何外部局部变量的存活！因此它没有外部 `&R` 要求！

  情况 B：`Vec<*i32>`（拥有元素缓冲区的指针容器）
  ┌──────────────────┐         ┌─────────────────┐         ┌──────────────────┐
  │ Vec<*i32> 描述符 │ ──拥有─►│ [ ptr1, ptr2 ]  │ ──指向─►│ 外部变量 `local` │
  └──────────────────┘         └─────────────────┘         └──────────────────┘
     (这层堆内存归 Vec 管)         (但里面装的指针元素，依然依赖外部的 `local`！)
```

看明白这层精妙的边界了吗？
1. **`Vec<*i32>` 自己拥有的那块元素数组堆内存**，属于它自己的内部资源，可以随着 `Vec` 的按值返回安全移交；
2. 但是，**数组里面装的每一个 `*i32` 指针元素所指向的外部目标**，并不归 `Vec` 拥有！因此，`Vec<*i32>` 依然必须诚实地暴露它内部元素对外部目标的可达性要求！

### 顺便认识一下：六大粗粒度来源根（Origin Roots）

在阅读编译器的诊断信息或编写高级底层契约时，你还会见到 Flurry 定义的六个内置粗粒度**来源根（Origin Roots）**：
- **`.constant`**：编译期/只读常量区；
- **`.heap`**：动态堆分配区；
- **`.local`**：函数栈帧局部存储区；
- **`.global`**：全局静态存储区；
- **`.tls`**：线程局部存储区（Thread-Local Storage）；
- **`.external`**：外部 FFI 或硬件映射区。

但请千万记住 Flurry 的安全红线（详见 `docs/resource-management/reachability.md`）：
**来源根只表示粗粒度的来源类别，绝不等于“同属一个类别的对象就永远有效”！**
- 属于 `.heap` 的对象，一旦 `free` 或扩容，它的世代照样终结；
- 属于 `.local` 的对象，每次函数调用都有自己独立的栈帧生命周期——编译器**绝不会把不同函数调用的局部存储合并成一个全程序长生不老的 `.local` 对象**！

---

## 本篇小结

在这一篇里，我们正式掌握了 Flurry 跨函数追踪指针与视图来源的利器——可达性限定类型 `&R T`：

| 核心概念 | 语法 / 表示 | 关键语义规则 |
| :--- | :--- | :--- |
| **可达性限定类型** | `&R T` | 前缀类型修饰（非取址运算！）；表示使用 `T` 时要求集合 `R` 中的资源与事实保持有效 |
| **多来源并集上界** | `&R1 &R2 T` | 保守合并多个可能来源的可达性要求；只声明依赖，不自动延长目标寿命或授予写权限 |
| **内部拥有 vs 外部要求** | `Vec<i32>` vs `Vec<*i32>` | 容器自身管理的堆分配随移动转交；但容器内指针元素指向的外部对象依赖仍须暴露 |
| **粗粒度来源根** | `.local`, `.heap`, `.constant` 等 | 仅标识存储类别；不同调用的 `.local` 栈帧严格隔离，绝不合并为永久有效 |

### 留给下一篇的悬念

现在我们已经认识了 `&R T`，也知道了 `Vec<*i32>` 会保留元素指向外部目标的可达依赖。
在真实的工程代码里，总有一些“手巧”的程序员试图通过各种花式手段，把一个指向局部变量 `let local = 23i32;` 的指针偷偷带出函数作用域（即**悬垂指针逃逸，Dangling Escape**）：
- **手段一**：直接 `return local.ref;`；
- **手段二（暗度陈仓）**：创建一个堆容器 `let values = Vec.new();`，执行 `values.push(local.ref);`，然后把 `values` 返回出去——心想：“`Vec` 是在堆上的，我把指针塞进堆里带出去，编译器总管不着了吧？”
- **手段三（偷梁换柱）**：把 `local.ref` 塞进一个闭包或者擦除类型的动态接口里返回出去；
- **手段四（金蝉脱壳）**：先让 `var saved = local.ref;`，然后在返回前用外部传入的参数 `saved = outer;` 覆盖它——哪种覆盖能安全过关，哪种在分支里的半吊子覆盖又会被编译器精准识破？

在下一篇 **《Flurry 演进手记 (048)：阻止悬垂指针逃逸出作用域——返回值、容器与闭包中的可达性检查》** 中，我们将上演一场精彩的“指针越狱与编译器缉捕”实战演练！
