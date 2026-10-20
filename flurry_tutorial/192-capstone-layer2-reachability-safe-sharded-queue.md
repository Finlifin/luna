# Flurry 演进手记 (192)：终极实战第二层（内存层）：用可达性类型 `&reachability` 与内存视图构建分片环形队列

在上一篇（第 191 篇）中，我们为 **Orion 异构流式信号处理引擎** 打造了 **Layer 1（协议层）**：网卡 DMA 送来的原始字节流通过 `OrionWireFrame` 与 `case NormalizedSample`，被零拷贝地解构成了标准化的 `DecodedSample` 采样点。

现在，海量采样点正以每秒数千万次的速率涌入引擎。我们需要构建 **Layer 2（内存层）**——一个**多通道分片环形队列（Sharded Ring Buffer Queue）**，让不同信道的工作线程各自持有互不重叠的环形缓冲区窗口视图（`ShardWindow`），并行推进写入与读取。

```
+=============================================================================+
|              Orion Signal Engine —— Layer 2: 内存与可达性安全层               |
+=============================================================================+
| Layer 1: OrionWireFrame 零拷贝解码 ---> DecodedSample                        |
|                                              |                              |
|                                              v                              |
| Layer 2: ShardedRingArena (底层连续采样池 + 独立代际身份 Gen#g)       <-- 本篇 |
|          +-- ShardWindow<arena_reach> (通道 0 零拷贝切片视图)                  |
|          +-- ShardWindow<arena_reach> (通道 1 零拷贝切片视图)                  |
+=============================================================================+
```

但在系统级编程中，一旦让多个分片视图直接指向同一块底层环形缓冲区，两个致命的内存安全幽灵就会立刻浮现：
1. **栈缓冲逃逸与悬垂视图（Escaping Stack Buffer）**：如果某个辅助函数在栈上建了一个临时采样槽，不小心把它包装进 `ShardWindow` 返回给了长生命周期的调度器，栈帧销毁后立刻触发 Use-After-Free。
2. **扩容重分配与代际复活（Generation Aliasing on Growth）**：如果环形缓冲区发生动态扩容或重置（Reallocation / Slot Reuse），旧的 `ShardWindow` 哪怕保存的裸地址碰巧被新分配器复用，它在逻辑上也已经指向了“上一代（Old Generation）”的死对象。

今天，我们就用 Flurry 的 **可达性类型修饰（`&reachability`）**、**容器管理存储与目标集分离**以及**指针代际身份（Pointer Generation Identity）**，为 Orion 引擎铸造绝对零悬垂的分片环形队列。

---

## 一、 问题背景：为什么传统的“单一生命周期参数 `'a`”在分片容器里不够用？

在传统借用检查模型中，一个带引用的切片包装结构通常只挂一个生命周期参数 `ShardWindow<'a>`。但在 Orion 这样复杂的流式引擎里，一个分片工作集往往同时涉及**两类寿命完全独立的存储**：
- **分片描述符管理存储（Descriptor Storage, `r_desc`）**：记录当前通道滤波窗口游标、水线元数据的局部控制块，只在单次批处理（Batch）期间存活；
- **底层采样池存储（Sample Arena Storage, `r_arena`）**：真正存放 `DecodedSample` 数据的长寿命 DMA 环形缓冲池，跨越整个流水线周期存活。

如果你把它们揉成同一个生命周期参数，那么当局部批处理结束、`r_desc` 销毁时，你从 `ShardWindow` 里提取出的长寿命采样视图也会被连坐判死刑，无法交给下游的异步滤波器；反过来，如果不做区分地放行，局部栈上的元数据地址又可能混进采样池视图里逃逸出去。

---

## 二、 第一步：用双层 `&reachability` 精确解耦容器与元素目标集

在 Flurry（见第 102～105 篇及 `docs/resource-management/reachability.md`）中，可达性修饰 `&r T` 不是粗粒度的“能活多久的时间段”，而是精确追踪**“该值依赖哪些具体存储集合与外部事实”**的编译期集合变量（`Type.vars.ReachSet`）。

更重要的是，Flurry 支持**多层可达性修饰**（如 `&r_desc &r_arena ShardLease`）：
- 外层 `&r_desc` 修饰**容器管理结构本身**的可达性依赖；
- 内层 `&r_arena` 修饰**从容器内部投影出的采样切片目标（Pointee Targets）**的可达性依赖。

```
+-----------------------------------------------------------------------------+
|            双层可达性修饰：&r_desc (外层描述符) vs &r_arena (内层采样池)       |
+-----------------------------------------------------------------------------+
|                                                                             |
|   局部栈帧 (Batch Scope)                   全局/长寿命采样池 (Arena Scope)    |
|  +-------------------------+              +------------------------------+  |
|  | BatchCursor (r_desc)    |              | ShardedRingArena (r_arena)   |  |
|  | - batch_id: 42          |              | [Sample 0][Sample 1][...]    |  |
|  | - window_ptr -----------+------------->|  ^-------- ShardWindow 指向此 |  |
|  +-------------------------+              +------------------------------+  |
|              |                                           |                  |
|   Batch 结束时 r_desc 销毁                    提取出的 &r_arena SampleSlice   |
|   但只依赖 &r_arena 的采样切片依然 100% 合法存活，可安全送往下一阶段流水线      |
+-----------------------------------------------------------------------------+
```

来看 Orion 分片环形队列的核心数据结构定义：

```flurry
struct DecodedSample {
    channel: u8,
    value: u16,
    is_urgent_beacon: bool,
}

-- 指向底层采样池某一段连续区域的零拷贝只读切片视图
struct SampleSlice {
    base_ptr: *DecodedSample,
    len: usize,
}

-- 分片租约：外层关联临时批处理游标，内层持有指向采样池的 SampleSlice
struct ShardLease
where
    r_arena: Type.vars.ReachSet,
{
    cursor_step: usize,
    payload: &r_arena SampleSlice,
}

-- 从短寿命的 &r_desc ShardLease<r_arena> 中提取出长寿命的 &r_arena SampleSlice。
-- 注意返回类型只携带 &r_arena，完全剥离了短寿命的 r_desc。
pure fn detach_arena_slice(
    lease: &r_desc ShardLease<r_arena>,
) -> &r_arena SampleSlice
where
    r_desc: Type.vars.ReachSet,
    r_arena: Type.vars.ReachSet,
{
    lease.payload
}
```

看到 `detach_arena_slice` 的签名了吗？
- 输入参数 `lease` 的类型是 `&r_desc ShardLease<r_arena>`：它既依赖短寿命的描述符可达集 `r_desc`，内部字段 `payload` 又携带了底层采样池的可达集 `r_arena`。
- 返回值类型是 `&r_arena SampleSlice`：编译器清楚地知道，`lease.payload` 的底层目标集只受 `r_arena` 约束，与外层 `r_desc` 毫无关系。因此，哪怕局部的 `lease` 控制块在当前函数返回后立即销毁，返回的 `&r_arena SampleSlice` 依然可以安全地跨越协程边界传递。

---

## 三、 第二步：拦截栈存储逃逸与扩容后的“旧代际僵尸视图”

有了精确的 `&reachability` 追踪，加上 Flurry 底层内存模型中的 **每次分配独立代际身份（Allocation Generation Identity）**，任何企图破坏环形队列生命周期的不安全操作都会在编译期现出原形。

### 拦截场景 1：把局部栈上的临时采样缓冲包装进长寿命分片视图（`escape_address`）

假设某位开发者写了一个调试辅助函数，在函数局部栈帧里创建了一个 `DecodedSample` 数组 `local_scratch`，然后试图把它包装成 `SampleSlice` 返回给外部调度器：

```flurry
-- ❌ 拒绝例 1：局部栈缓冲区的可达集无法逃逸出当前函数词法作用域
fn make_dangling_shard() -> SampleSlice {
    var local_scratch: Array<DecodedSample, 4> = [
        DecodedSample { channel: 0, value: 100, is_urgent_beacon: false },
        DecodedSample { channel: 0, value: 101, is_urgent_beacon: false },
        DecodedSample { channel: 0, value: 102, is_urgent_beacon: false },
        DecodedSample { channel: 0, value: 103, is_urgent_beacon: false },
    ];

    -- 编译错误 (escape_address)：
    -- Returned `SampleSlice` carries reachability `{local_scratch}`,
    -- which depends on local stack storage that does not survive function return.
    SampleSlice {
        base_ptr: local_scratch[0].ref,
        len: 4,
    }
}
```

编译器精确追踪到 `SampleSlice` 内部隐含了局部存储集合 `{local_scratch}`。当函数返回时，`local_scratch` 所属的栈帧即将弹栈销毁，任何依赖 `{local_scratch}` 的返回值都会被静态拦截。

### 拦截场景 2：环形缓冲区动态扩容后继续读取旧分片视图（`read_after_growth`）

再看第二个更隐蔽的工业级 Bug：先从环形队列 `ring` 中借出一个分片视图 `old_view`（其可达性绑定在 `ring` 当前的底层缓冲区分配代际 $Gen_1$ 上），随后调用 `ring.ref.grow_capacity()` 触发底层存储重分配（进入新代际 $Gen_2$，$Gen_1$ 失效），接着又去读取 `old_view`。

哪怕底层内存分配器碰巧在原地扩展或复用了同一个虚拟内存地址，在 Flurry 的内存模型中，**新分配拥有全新的代际身份 $Gen_2$，绝不会“复活”绑定在 $Gen_1$ 上的旧指针**。

```
+-----------------------------------------------------------------------------+
|             指针代际身份 (Generation Identity) 阻止扩容后旧视图复活           |
+-----------------------------------------------------------------------------+
| 1. let old_view = ring.ref.shard_view(0)                                    |
|    ---> old_view 绑定可达集 &{ring.buf @ Gen#1}                              |
|                                                                             |
| 2. ring.ref.grow_capacity(new_cap)                                      |
|    ---> 独占写操作使 Gen#1 彻底失效  ring.buf 切换至全新代际 Gen#2             |
|                                                                             |
| 3. read_first_sample(old_view)                                              |
|    ---> ❌ 编译期拒绝  old_view 要求的 Gen#1 已在步骤 2 中终结                 |
+-----------------------------------------------------------------------------+
```

来看这段被编译器精准拦截的代码：

```flurry
struct RingQueue {
    buf_ptr: *DecodedSample,
    capacity: usize,
}

fn shard_view(ring: *RingQueue) -> &{ring} SampleSlice {
    SampleSlice {
        base_ptr: ring.buf_ptr.*,
        len: ring.*.capacity,
    }
}

fn grow_capacity(ring: *RingQueue, new_cap: usize) {
    -- 独占可变访问：可能重分配或覆写底层存储，使此前借出的 &{ring} 视图全部失效
    ring.capacity.* = new_cap;
}

-- ❌ 拒绝例 2：在可变扩容操作之后继续使用旧代际的可达性视图
fn bad_read_after_growth(ring: *RingQueue) -> u16 {
    let old_view = shard_view(ring.as(*RingQueue));

    -- 这里对 ring 进行了独占可变修改，终结了 old_view 依赖的早期借用状态
    grow_capacity(ring, 1024);

    -- 编译错误 (read_after_growth / invalidated reachability)：
    -- Cannot use `old_view` carrying reachability `&{ring}` because `ring`
    -- was mutably accessed by `grow_capacity(ring, 1024)` while `old_view` was live.
    unsafe { old_view.base_ptr.value.* }
}
```

如果把读取操作移到 `grow_capacity` 之前，或者在 `grow_capacity` 之后重新获取绑定在新代际上的 `new_view = shard_view(...)`，代码就能正常通过编译：

```flurry
-- ✅ 正向示例：在扩容前完成旧视图读取，或扩容后获取新代际视图
fn safe_growth_workflow(ring: *RingQueue) -> u16 {
    let old_view = shard_view(ring.as(*RingQueue));
    let first_val = unsafe { old_view.base_ptr.value.* }

    -- old_view 最后一次使用已结束，现在可以安全扩容
    grow_capacity(ring, 1024);

    let new_view = shard_view(ring.as(*RingQueue));
    first_val + unsafe { new_view.base_ptr.value.* }
}
```

---

## 四、 本层通关小结与下一层预告

在 **Layer 2（内存层）** 中，我们利用 Flurry 的可达性类型系统为 Orion 引擎解决了跨阶段缓冲区的生命期难题：
- 用双层可达性修饰 `&r_desc ShardLease<r_arena>` 彻底解耦了“短寿命批处理游标”与“长寿命底层 DMA 采样池”；
- 用栈存储逃逸检查与指针代际身份模型，在编译期封死了悬垂切片与扩容后旧视图复活。

但是，光证明“整个环形缓冲区还活着”还不够。当两个分片通道（比如 `shard_a` 和 `shard_b`）同时向同一个 `RingQueue` 的不同槽位写入采样点时，我们怎么向编译器证明 **`shard_a` 写入的槽位下标既没有算术溢出，也绝对不会踩到 `shard_b` 正在写入的槽位**？

在下一篇 **第 193 篇（Layer 3：验证层）** 中，我们将祭出第八卷的硬核心机制——`verified fn`、`predicate` 与 `outcomes write<...>` 内存视图分离契约，在编译期完成环形队列无溢出与槽位分离的数学证明。
