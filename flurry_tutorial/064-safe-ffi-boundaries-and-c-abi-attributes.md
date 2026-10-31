# Flurry 演进手记 (064)：安全地跨过 C 语言边界——^attrs.abi(.c)、裸指针契约与 FFI 封装

> **系列导读**：在上一期（063）中，我们深入编译器内部，看清了局部移动与 Drop 状态机如何管理复杂控制流下的资源生命周期。至此，Flurry 语言内部的安全闭环已经严丝合缝。但在真实的系统工程（如驱动开发、操作系统内核与高性能网络栈）中，我们的 `RingBuffer` 在所难免地要与硬件 DMA 寄存器或现有的 C 语言底层库直接对话。今天，我们将探讨 Flurry 如何跨过 FFI（外部函数接口）边界——为什么 Flurry **报错引入 `extern` 关键字**，而用统一的编译期属性 `^attrs.abi(.c)` 取而代之？我们又该如何用契约将充满隐患的裸 C 接口驯化为经过数学验证的零拷贝安全抽象？

---

## 一、语法设计哲学：为什么 Flurry 没有 `extern "C"` 关键字？

在 C++ 或 Rust 中，每当你想要指定函数的底层调用约定（Calling Convention / ABI）时，都必须搬出一个专门的关键字：`extern "C" fn foo()`。

但在 Flurry 的语言设计中，有一条不可动摇的语法规则：
> **函数 ABI 统一通过属性（Attribute）指定，免于引入 `extern` 关键字。**

为什么？因为在 Flurry 看来，一个函数的**语言级语义**（它的参数类型、返回类型、纯度、内存效果与前后置契约）与它在目标机器上的**底层二进制调用约定**（寄存器如何分配、栈由谁清理、符号如何修饰）是正交的两个维度。为了底层 ABI 专门污染核心语法关键字，非但破坏了语法正交性，还阻碍了元编程对 ABI 配置的统一抽象。

在 Flurry 中，任何前缀属性都使用 **`^expr`** 语法写在条目之前。指定 C 语言 ABI 只需写上清晰直观的 **`^attrs.abi(.c)`**：

```flurry
-- ✅ Flurry 标准方式：用前缀属性 ^attrs.abi(.c) 声明 C 调用约定
^attrs.abi(.c)
fn fl_ring_version() -> u32 {
    1u32
}

-- ❌ 语法错误。Flurry 未存 extern 关键字。
-- extern "C" fn fl_ring_version() -> u32 { ... }
```

### ⚠️ 核心语义边界：`^attrs.abi(.c)` 只改变调用约定，不豁免安全检查

很多从其他语言转过来的开发者容易产生一个误解：“加了 C ABI 属性的函数是否就变成脱离编译器管辖的法外之地了？或者变成编译期函数了？”

**完全不是。** Flurry 规范明确规定：
1. `^attrs.abi(.c)` 自身是一个在编译期求值并附着元数据的属性调用，但它修饰的目标函数**依然是普通的运行期函数**（不会因此变成 `comptime fn`）。
2. 给函数加上 `^attrs.abi(.c)` **只配置底层调用约定（Calling Convention），不削弱或绕过 Flurry 的指针有效性、可达性（`&r`）与内存效果检查**。

---

## 二、深入属性机制：外层覆盖内层、前缀覆盖内部配置

除了为函数指定 `^attrs.abi(.c)`，在与 C 语言或硬件寄存器交互时，我们还需要精准控制结构体（`struct`）与联合体（`union`）的内存布局（例如消除字节对齐填充的 `.packed` 布局）。

在 Flurry 中，配置既可以通过条目内部的 `.key value` 属性声明，也可以通过条目外部的前缀 `^attr` 声明。当多个属性同时作用于同一个声明时，编译器严格遵循一条确定性的**由内向外（Innermost First）求值法则**。

### 属性求值流水线与覆盖优先级

```text
[ 源码声明 ]
  ^outer_attr
  ^inner_attr
  struct DmaDescriptor {
      .packed false,   <── (1) 最先处理：结构体内部配置 (.packed false)
      header: u16,
      payload: u32,
  }
      │
      ├─► (2) 接着应用紧贴条目的 ^inner_attr（可读取并覆盖内部配置）
      │
      └─► (3) 最后应用最外层的 ^outer_attr（拥有最高最终决定权。）
```

让我们用具体的代码验证这两条黄金优先级法则：
1. **多个前缀属性叠加时（`^outer ^inner item`）**：紧贴条目的 `^inner` 先执行，外层的 `^outer` 后执行；若两者设置了冲突的同名配置，**后执行的 `^outer` 覆盖 `^inner`**。
2. **前缀属性与结构体内部配置共存时**：结构体内部的 `.packed` 先求值，外部的前缀属性 `^...` 后求值，因此**前缀属性覆盖内部配置**。

```flurry
-- 结构体内部直接使用 .packed true 配置紧凑内存布局（无对齐填充字节）
struct PackedHeader {
    .packed true,
    tag: u8,
    length: u32,
}

-- C 风格联合体（union）：所有字段共享同一段起始内存偏移
union RegisterWord {
    raw: u32,
    bytes: Array<u8, 4>,
}
```

---

## 三、为底层 C DMA 接口打造零拷贝安全装甲

现在，假设底层硬件驱动用 C ABI 暴露了一个将单个 32 位字写入硬件环形槽位的底层函数 `c_dma_write_slot`。在 C 语言头文件里，它只是一个没有任何约束的裸指针函数：
`void c_dma_write_slot(int32_t* base, size_t cap, size_t idx, int32_t val),`

如果我们在初始写法中直接草率地在业务代码里裸调这种底层接口，会发生什么？

### 阶段初始写法：裸露的底层边界与失控的越界风险

```flurry
-- 底层 C ABI 硬件原语：要求 idx < cap，否则触发物理总线越界写故障。
^attrs.abi(.c)
unsafe fn c_dma_write_slot(
    slots: *Array<i32, 8>,
    idx: usize,
    val: i32,
) -> void
where
    requires idx < 8usize,
    outcomes write<slots>
{
    slots.*[idx] = val;
}

fn bad_caller(slots: *Array<i32, 8>, user_idx: usize) -> void {
    -- ❌ 编译期直接报错。
    -- 1. c_dma_write_slot 是 unsafe fn，禁止于没有 unsafe 块的情况下直接调用；
    -- 2. user_idx 是未验证的外部输入，无法证明 requires idx < 8usize。
    c_dma_write_slot(slots, user_idx, 42i32);
}
```

### 1. 用受契约保护的 `unsafe` 块桥接 C 原语

在第一部分中，我们遵循第 059 期学到的安全隔离法则：**把 `unsafe` 关进最小笼子里，向外暴露带完整 `requires`、`outcomes` 和 `ensures` 契约的 Flurry 函数**。

```flurry
struct RingBuffer {
    slots: Array<i32, 8>,
    head: usize,
    tail: usize,
    len: usize,
}

predicate valid_ring(buf: *RingBuffer) {
    buf.head.* < 8usize
        and buf.tail.* < 8usize
        and buf.len.* <= 8usize
        and (buf.head.* + buf.len.*) % 8usize == buf.tail.*
}

predicate modify(target) -> MemoryEffect {
    read<target> and write<target>
}

-- 对外是安全的 verified fn，内部通过已证明的契约安全调用 C ABI 原语
verified fn dma_push_one(buf: *RingBuffer, val: i32) -> void
where
    requires valid_ring(buf) and buf.len.* < 8usize,
    outcomes write<buf.slots> and modify(buf.tail) and modify(buf.len),
    ensures valid_ring(buf),
    ensures buf.len.* == (buf.len.*)'old + 1usize,
    ensures buf.tail.* == ((buf.tail.*)'old + 1usize) % 8usize
{
    -- 证明器已知：valid_ring(buf) 蕴含 buf.tail.* < 8usize，
    -- 良好满足 c_dma_write_slot 的 requires idx < 8usize 前置契约。
    unsafe {
        c_dma_write_slot(buf.slots, buf.tail.*, val);
    }
    buf.tail.* = (buf.tail.* + 1usize) % 8usize;
    buf.len.* = buf.len.* + 1usize;
}
```

### 2. 反向导出将经过数学证明的 Flurry `verified fn` 暴露给 C 语言调用

FFI 是一条双向车道：我们非但能从 Flurry 调用 C，还可以给 Flurry 的函数加上 `^attrs.abi(.c)`，把它编译成标准的 C ABI 符号供 C/C++ 甚至操作系统内核调用。

更棒的是，当我们在 Flurry 内部导出一个 C ABI 函数时，既然外部 C 代码没有 Flurry 的编译期证明器，我们该如何防止粗心的 C 调用者传进非法的游标状态呢？
答案是：**在 C 边界入口函数上使用普通安全模式 + 显式布尔/错误分支检查，检查通过后立刻委托给内部零开销的 `verified fn`。**

```flurry
enum DmaStatus {
    ok,
    buffer_full,
    corrupted_state,
}

-- 导出给外部 C 语言调用的坚固防火墙接口
^attrs.abi(.c)
unsafe fn fl_ring_dma_push(buf: *RingBuffer, val: i32) -> DmaStatus
where
    outcomes read<buf.head> and write<buf.slots> and modify(buf.tail) and modify(buf.len)
{
    -- 第一道防线：拦截来自 C 语言侧的非法游标状态
    if not (buf.head.* < 8usize and buf.tail.* < 8usize and buf.len.* <= 8usize) {
        return DmaStatus.corrupted_state;
    }
    if (buf.head.* + buf.len.*) % 8usize != buf.tail.* {
        return DmaStatus.corrupted_state;
    }
    -- 第二道防线：检查环形缓冲区是否已满
    if buf.len.* >= 8usize {
        return DmaStatus.buffer_full;
    }

    -- 进入零开销核心区。
    -- 此时编译器已从上面的三个 if 分支收集到完整路径敏感事实：
    -- valid_ring(buf) and buf.len.* < 8usize 100% 成立。
    -- 直接调用 verified fn，内部零产出冗余边界检查指令。
    dma_push_one(buf, val);
    DmaStatus.ok
}
```

看看 `fl_ring_dma_push` 的结构有多么优雅：
- **在边界处（Boundary）**：用精准的分支卫兵（Guards）净化来自 C 语言的存疑状态；
- **在核心区（Core）**：一旦跨过卫兵分支，编译器自动通过路径敏感分析（Path-Sensitive Refinement）合成出 `valid_ring(buf) and buf.len.* < 8usize` 的数学证明，无缝接入内部的 `verified fn`。

---

## 四、本节核心概念速查表

| 特性 / 规则 | 语法形式 | 核心语义与红线 |
| :--- | :--- | :--- |
| **C ABI 函数声明** | `^attrs.abi(.c) fn ...` | 指定底层 C 调用约定；**严禁使用 `extern` 关键字** |
| **ABI 属性的语义边界** | 仅影响调用约定 | 免将函数变成 `comptime fn`，也**免除**关闭指针可达性、有效性或契约检查 |
| **属性求值顺序** | `^outer ^inner item` | **由内向外**依次应用：内部 `.config` 最先，紧贴条目的 `^inner` 次之，最外层 `^outer` 最后并具有最高覆盖权 |
| **紧凑结构体与联合体** | `struct Name { .packed true, ... }` / `union Name { ... }` | 精确匹配 C 语言与硬件寄存器的二进制内存布局 |
| **FFI 安全封装模式** | 边界卫兵分支 + 核心 `verified fn` | 在存疑 C 边界做一次显式校验，利用路径敏感分析零开销接入内部已验证代码 |

---

## 五、下一期预告：第三卷大结局（Milestone）

从第 041 期的 `Once` vs `Copy` 出发，历经 24 期的层层递进，我们已经走完了第三卷 **《所有权、资源安全与形式化验证》** 的全部理论与机制拼图：
- **资源轴**：`Once`/`Copy`、局部移动位置树、确定性 `Drop` 与 `defer`/`errdefer`；
- **指针与视图轴**：代际引用计数、路径可达性（`&r`）、借用视图（`MemoryView`）与分离证明；
- **效果与验证轴**：六大基本内存效果、`P0`–`P5` 纯度层级、三次多项式区间推理、`requires`/`ensures`/`outcomes`、`predicate` 与 `verified fn`。

下一期 **《Flurry 演进手记 (065)：第三卷里程碑——实现一个零拷贝、带静态越界与分离证明的环形缓冲区 RingBuffer》**，我们将迎来第三卷的巅峰之作：把这 24 期的所有武器融会贯通，从零组装出一个**工业级、零拷贝、带完整数学证明与 C DMA 接口的 `RingBuffer` 模块**，并开启通往第四卷（双层类型理论 2LTT、编译期宇宙 $U_1$ 与静态形状张量）的大门。
