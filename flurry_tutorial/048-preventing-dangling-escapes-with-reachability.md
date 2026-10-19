# Flurry 演进手记 (048)：阻止悬垂指针逃逸出作用域——返回值、容器与闭包中的可达性检查

在 [上一篇 (047)](./047-reachability-types-tracking-origins.md) 中，我们学习了可达性限定类型 `&R T` 是如何记录一个值对外部资源的依赖集合的。

今天，在为我们的 `RingBuffer` 编写槽位注册与回调导出接口时，我们将直面 C 和 C++ 历史上造成最多高危安全漏洞（CVE）的头号杀手：**栈上局部地址逃逸（Stack Address Escape / Dangling Pointer Return）**。

每个写过 C 语言的程序员都知道不能写 `int x = 23; return &x;`。
但在真实的、上万行的大型系统工程里，悬垂指针从来不会这么赤裸裸地写在脸上！它们往往披着三件极具欺骗性的伪装外衣：
1. **伪装一（装进堆容器偷渡）**：把局部变量的地址 `local.ref` 压进一个堆分配的动态数组 `Vec<*i32>` 里，然后把 `Vec` 返回出去；
2. **伪装二（写进外部输出参数或被返回闭包捕获）**：通过一个外部传进来的容器指针 `out_list: *Vec<*i32>` 把 `local.ref` 存进去，或者返回一个捕获了 `local.ref` 的闭包；
3. **伪装三（分支里的半吊子覆盖）**：指针变量 `saved` 一开始指向局部变量 `local.ref`，只在 `if` 分支里改指向了合法的外部参数 `outer`，而 `else` 分支里忘了改，最后把 `saved` 返回出去！

让我们在这场“越狱与缉捕”的实战演练中，看看 Flurry 编译器的**出口可达性核验器（Exit Reachability Verifier）**是如何把这三种伪装扒得底朝天，又是如何放行真正合法的“金蝉脱壳”的！（详见 `docs/resource-management/reachability.md` 与 `docs/resource-management/pointers.md`）。

---

## 第一幕：直接返回数值 vs 直接返回局部地址（v0）

我们先从最基础的对比热身：为什么 `make_number()` 可以把局部变量 `local` 返回出去，而 `escape_address()` 却不能把 `local.ref` 返回出去？

```flurry
use std.prelude.*;

-- ✅ 完全合法：读取局部变量取得独立标量值，不再依赖 local 的栈槽位！
fn make_number() -> i32 {
    let local = 23i32;
    local
}

-- ❌ 编译期直接拒绝！
fn escape_address() -> *i32 {
    let local = 23i32;
    -- 错误：返回的指针依赖局部位置 `local`，但 `local` 在函数退出清理后已终结！
    local.ref
}
```

让我们画一张 Code Cartoon，重温编译器在**函数出口（Function Exit）**执行的铁血三步曲：

```text
  函数出口的三步核验流水线（详见 `docs/resource-management/reachability.md`）：

  [第 1 步：准备结果] ──► 计算返回值表达式，记录返回值携带的可达依赖集合 R_ret
                          (在 `make_number` 中：i32 是独立标量，R_ret = 空集 ∅)
                          (在 `escape_address` 中：*i32 指向 local，R_ret = {local})
                                │
                                ▼
  [第 2 步：执行清理] ──► 按声明逆序清理当前函数作用域内所有未移出的局部绑定与栈槽！
                          ──► 局部变量 `local` 的存储世代在此刻【终结 KILLED 💀】！
                                │
                                ▼
  [第 3 步：出口验尸] ──► 检查第 1 步记录的 R_ret（以及所有写入外部对象的值、捕获环境）
                          中是否有任何目标在第 2 步被 KILLED？
                          ├── `make_number`:    ∅ 中无失效目标 ──► ✅ 放行！
                          └── `escape_address`: `{local}` 已 KILLED ──► ❌ 当场击毙！
```

记住这个**“先准备结果 -> 再执行局部清理 -> 最后核验结果与外部保留关系”**的三步顺序！有了这个心智模型，接下来无论越狱者穿多厚的马甲，你都能一眼看穿它。

---

## 第二幕：击碎“堆容器偷渡”伪装——`Vec<i32>` vs `Vec<*i32>`（v1）

现在看越狱者的第一件伪装外衣：
一位程序员想把局部配置值放进 `Vec` 里返回。他先写了存数值的 `make_values()`，顺利通过；接着他又依葫芦画瓢写了存指针的 `make_addresses()`：

```flurry
-- ✅ 完全合法！
fn make_values() -> Vec<i32> {
    let values = Vec.new();
    let local = 23i32;
    -- 把 local 的数值 23 复制进 Vec 的堆缓冲区，与 local 的栈槽位彻底脱钩！
    values.push(local);
    values
}

-- ❌ 编译期直接拒绝！
fn make_addresses() -> Vec<*i32> {
    let values = Vec.new();
    let local = 23i32;
    -- 试图把指向局部栈槽的指针 `local.ref` 塞进堆上的 Vec 里偷渡出去！
    values.push(local.ref);
    -- 错误：虽然 `values` 自己的堆缓冲区可以转交，但元素仍携带对即将终结的 `local` 的依赖！
    values
}
```

为什么把 `local.ref` 塞进堆上的 `Vec` 里也逃不出编译器的手掌心？
让我们用一张 Code Cartoon 看看 `values.push(local.ref)` 在可达性依赖图上留下了什么痕迹：

```text
  栈帧 (make_addresses)             堆内存 (Heap)              栈帧局部变量 (即将销毁！)
  ┌───────────────────┐            ┌─────────────────┐        ┌───────────────────┐
  │ values: Vec<*i32> │ ──(拥有)──►│ 元素[0]: *i32   │ ──────►│ let local = 23i32 │
  └───────────────────┘            └─────────────────┘        └───────────────────┘
   │                                                           ▲
   └──► `values` 的整体可达依赖 R = { 元素指向的目标: `local` } ─┘

  当 `make_addresses` 退出时：
  1. `values` 自己的堆内存所有权转移给了调用者（堆内存没释放）；
  2. 但栈上的 `local` 在退出时被 KILLED 💀！
  3. 出口验尸发现返回的 `values` 依然背负着对已死目标 `{local}` 的依赖 ──► ❌ 静态拒绝！
```

不仅如此，根据 `docs/resource-management/reachability.md` 的规定，编译器在出口处核验的**绝不仅仅是函数的返回值**，而是全面覆盖以下三个出口通道：
1. **函数的返回值**（如上面的 `return values;`）；
2. **写入外部对象的保留值**（比如函数参数传入了一个外部容器指针 `out_vec: *Vec<*i32>`，你在函数里写 `out_vec.push(local.ref);`——哪怕函数返回类型是 `void`，因为外部对象 `out_vec` 在函数退出后依然存活，把局部地址写进外部对象的行为同样会在出口验尸时被当场抓获！）；
3. **返回或逃逸闭包的捕获环境**（哪怕把闭包或对象用 `dyn Trait` 擦除了具体类型，类型擦除也绝不会抹去它底层背负的可达性依赖要求！）。

---

## 第三幕：金蝉脱壳——用完整覆盖（Strong Update）洗白局部依赖（v2）

既然只要在出口处背负着对局部变量 `local` 的依赖就会被拦截，那么请看下面这个非常有意思的工程场景：

在我们的 `RingBuffer` 游标扫描函数中，我们需要一个临时指针变量 `var cursor: *i32`。
- 在函数前半段，为了做本地预演计算，我们先让 `cursor` 指向了一个函数内部的局部暂存变量 `let scratch = 0i32; var cursor = scratch.ref;`；
- 在本地计算完成后、函数真正返回之前，我们把 `cursor` 重新赋值为指向外部传入的环形缓冲区槽位 `outer: *i32`，然后返回 `cursor`！

请问：这时候 `cursor` 曾经指向过局部变量 `scratch`，编译器会让它安全返回吗？

**答案取决于：你是“100% 确定无疑地完整覆盖了它”，还是“只在某个分支里可能覆盖了它”！**

让我们对比这两个版本，并用 Code Cartoon 看看编译器的**强更新（Strong Update）**与**弱更新（Conservative Merge）**规则（详见 `docs/resource-management/reachability.md` 与 `docs/resource-management/pointers.md`）：

```flurry
use std.prelude.*;

-- ✅ 版本 A：对单个精确位置执行完整覆盖（Strong Update），100% 合法通过！
fn choose_outer_safely(outer: *i32) -> *i32 {
    let scratch = 1i32;
    var cursor = scratch.ref; -- 此时 cursor 的依赖集合为 {scratch}

    -- 在此处先用 cursor 做一些本地读取计算...
    let _ = cursor.*;

    -- 关键一步：对单个确定槽位 `cursor` 执行无条件完整覆盖！
    -- 编译器将 cursor 的依赖集合从 {scratch} 彻底替换为 {outer}！
    cursor = outer;

    -- 出口验尸时，cursor 的依赖只有 {outer}，完全不背负 scratch 的地址！
    cursor
}

-- ❌ 编译期直接拒绝！
-- 版本 B：只在条件分支中部分覆盖，另一条路径仍残留对 scratch 的依赖！
fn uncertain_overwrite_bug(outer: *i32, use_outer: bool) -> *i32 {
    let scratch = 1i32;
    var cursor = scratch.ref; -- 依赖: {scratch}

    if use_outer {
        cursor = outer;       -- then 分支依赖: {outer}
    }
    -- else 分支没改 cursor，依然依赖: {scratch}！
    -- 分支汇合后，编译器保守取并集，cursor 的依赖为 {outer, scratch}！

    -- 错误：出口清理后 scratch 已死，而 cursor 仍可能指向 scratch！
    cursor
}
```

让我们画一张 Code Cartoon，把这两种更新在依赖图上的变化看得清清楚楚：

```text
  场景 A：无条件完整覆盖 (`choose_outer_safely`)
  [初始] `cursor` ──依赖──► { scratch (局部) }
  [覆盖] `cursor = outer;` (目标是单个精确局部槽位！触发【强更新 Strong Update】)
         `cursor` ──依赖──► { outer (外部参数) }  (旧的 `{scratch}` 依赖被彻底擦除！✨)
  [出口] `scratch` 销毁，但 `cursor` 只依赖 `{outer}` ──► ✅ 安全返回！

  场景 B：条件分支汇合或未知别名覆盖 (`uncertain_overwrite_bug`)
  [初始] `cursor` ──依赖──► { scratch }
         ├── [if true]  `cursor = outer;` ──► { outer }
         └── [if false] (未覆盖)          ──► { scratch }
  [汇合] 两条路径取并集：`cursor` ──依赖──► { outer, scratch }
  [出口] `scratch` 销毁，`cursor` 仍残留 `{scratch}` 依赖 ──► ❌ 静态拒绝！
```

总结强更新（洗白旧依赖）的三个严格前提：
1. 被赋值的目标必须是**单个精确的具体实例位置**（如局部变量 `cursor`）；如果是通过一个可能指向多个对象的未知别名指针去写，或者往一个汇总了多个元素的数组槽位里写，编译器不能借一次写入就盲目清空全部旧依赖。
2. 必须是**完整覆盖（Full Overwrite）**，而不能只是修改了结构体里的某一半字段，另一半字段里还留着旧指针。
3. 在到达函数出口的所有**可达路径**上，旧的局部依赖都必须已经被彻底替换或消费完毕！

---

## 本篇小结

通过这场“悬垂指针越狱与缉捕”的实战，我们彻底摸清了 Flurry 跨作用域可达性检查的铜墙铁壁：

| 逃逸企图 / 场景 | 代码形态 | 编译器的判定与理由 |
| :--- | :--- | :--- |
| **按值返回标量** | `let local = 23; return local;` | **✅ 放行**：标量值复制后与原栈槽脱钩，无外部可达依赖 |
| **直接返回局部指针** | `let local = 23; return local.ref;` | **❌ 拒绝**：出口清理先终结 `local`，返回结果依赖已死目标 |
| **装进堆容器偷渡** | `vec.push(local.ref); return vec;` | **❌ 拒绝**：容器虽拥有堆缓冲区，但元素仍暴露对局部 `local` 的可达依赖 |
| **写入外部对象或闭包捕获** | `out_vec.push(local.ref);` | **❌ 拒绝**：出口检查同时核验返回值、外部写入保留值与捕获环境（含 `dyn` 擦除） |
| **精确槽位完整覆盖** | `var p = local.ref; p = outer; return p;` | **✅ 放行**：对单一确定位置做完整覆盖触发强更新，新依赖 `{outer}` 替换旧依赖 |

### 留给下一篇的悬念

到现在为止，我们已经掌握了单个对象和单个指针的生命周期与可达性追踪。
但是，回到我们的第三卷主线任务——**高性能零拷贝 `RingBuffer`**！

在真正的环形缓冲区和信号处理内核里，我们面对的是一整块连续的内存大数组（比如容量为 1024 的底层缓冲区 `storage`），我们需要：
1. 把这块大缓冲区安全地**切分（Slice / View）**成前半段的“消费者读取区间 `[0..mid]`”和后半段的“生产者写入区间 `[mid..len]`”；
2. 或者只取出结构体数组里某一个字段的投影视图；
3. 最关键的是：当我们把 `[0..mid]` 和 `[mid..len]` 两个可变切片同时传给一个原地处理函数时，编译器怎么知道**这两个来自同一个大数组的不同区间是互不重叠（Disjoint）的**？

在下一篇 **《Flurry 演进手记 (049)：把一块大缓冲区安全切成两半——内存视图（Memory Views）、字段视图与区间视图》** 中，我们将正式进入 Flurry 内存视图与区间代数的精彩世界！
