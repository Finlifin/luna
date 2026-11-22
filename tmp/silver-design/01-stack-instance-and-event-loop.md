# 栈实例、有界栈池与事件循环模型

> **状态**：`Draft v0.1（讨论稿）`  
> **前置阅读**：[00-overview.md](file:///home/fin/luna/tmp/silver-design/00-overview.md)

---

## 1. 栈实例 = 组件实例

传统声明式 GUI 需要将“描述如何渲染”的函数拆成组件描述（Widget / VNode）与框架后台维护的状态节点（Fiber / Element / State 对象），再在两者之间靠数组下标（Hooks）或 key 做弱关联。

在 Silver 中，**组件函数的执行栈自身就是组件实例**：
- 组件的局部 `let`/`var` 变量（计数器、缓存、子句柄、动画插值器、RAII 订阅守卫）直接保存在栈帧里；
- 组件的阶段状态机（例如“空闲” vs “拖拽中” vs “等待弹窗确认”）直接表示为当前程序计数器（PC）停在哪一个 `pull_event()#` 挂起点，而无需手动维护 `enum Phase`；
- 卸载组件不需要框架回调 `on_unmount` / `dispose`：丢弃持有该组件的 `builtin.Continuation` 句柄，Flurry 控制后端就会沿隐藏中止通道重新进入被挂起栈帧，按**词法声明逆序**执行全部局部变量的 `Drop`，到达限定边界后归还栈槽。

### 组件生命周期状态转移

| 阶段 | 执行位置 | 栈与 continuation 状态 | 说明 |
|---|---|---|---|
| **Mount（挂载）** | 组件栈（新分配栈槽） | 活跃执行直至首次 `pull_event()#` | 执行首帧初始化：构建局部状态、注册场景句柄、派生子组件 |
| **Suspended（挂起待命）** | 框架主栈 / 调度队列 | `builtin.Continuation` 由 `WidgetSlot` 唯一持有 | 栈帧冻结；不占用 CPU，仅保留静态确定上界的小栈块 |
| **Resume（事件响应）** | 组件栈（单次消费并接回返回位置） | 正在运行；恢复后再次挂起时交出**新** `Continuation` | 取得 `WidgetEvent`（或子弹窗/异步返回值）推进一轮逻辑 |
| **Unmount（卸载清理）** | 组件栈（隐藏中止通道逐帧展开） | 未恢复 `Continuation` 离开作用域触发放弃协议 | 按词法逆序自动 `Drop` 局部绑定、子栈与结构化异步任务，最后归还栈槽 |

> **语义契约提醒**：依据 Flurry [multi-prompt-delimited-continuation.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/multi-prompt-delimited-continuation.md)，`builtin.Continuation` 不可 `Copy`/`Clone`，恢复与放弃争用同一个**唯一所有权**；且放弃走隐藏中止通道，不会进入任何用户错误 `catch` arm，也不会跳过尚存资源的 `Drop`。

---

## 2. 为什么绝大多数组件栈可以确定较小的静态上界

通常人们排斥有栈协程（stackful coroutine）做大规模对象实例，原因是“传统有栈协程必须按最坏深递归预留 64KB–1MB 虚拟栈”。Silver 能把组件栈压缩到几百字节并静态定界，依赖 Flurry 的两个结构性特征：

### 2.1 Handler 分支在限定边界之外执行

在 Flurry 中，effect handler 的分支体在**限定边界之外**（即外层安装者的执行栈，在 Silver 中就是框架主栈）运行。这意味着组件栈内发出如下重操作请求时：
- 测量布局、文本断行与字体塑形（`#LayoutOps`）
- 向保留场景树提交节点或重绘脏区域（`#SceneOps`）
- 打开文件、网络请求或编码解码（`#AsyncIO`）
- 派生子组件栈（`spawn_child`）

组件自身栈帧**只需保留调用点现场和出入口参数**，真正深达数千字节的排版器、光栅化准备或系统调用栈全部压在框架主栈上运行，**绝不计入组件栈上界**。

### 2.2 子组件与递归树跨边界分配，打破单栈递归

即便是自递归 UI 结构（如文件树 `FileTreeNode`、JSON 折叠视图、无限评论楼层），父节点也不是在自己的调用栈上递归调用子节点函数，而是通过 `spawn_child` effect 在边界外为子节点单独分配一个同级有界栈。因此，**单个组件函数自身的内部调用图通常是非递归的 DAG**，编译器能够直接沿局部调用图求出精确栈帧上界（`frame_bytes + max_callee_bytes_within_boundary`）。

```text
框架主栈 S0（运行调度器、Layout/Scene/Render handler，可使用常规 OS 栈）
  │
  ├── .use(pool_tier1_256b) ──► 组件栈 S1（Counter 组件实例，静态上界 184B → 落入 256B 槽）
  │                               └── 发出 SceneOps.set_text(...)#
  │                                     └─► 切回 S0 外层栈执行重操作，S1 栈指针不再增长
  │
  └── .use(pool_tier1_512b) ──► 组件栈 S2（DragHandle 组件实例，静态上界 340B → 落入 512B 槽）
```

### 2.3 三级栈存储架构（均通过 `.use(pool)` 接入）

依据 Flurry [effect-abi.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md)，每段捕获栈保留自己的分配来源，通过 `.use(pool)` 传入控制协议，无需把池参数写进每个组件函数签名：

| 层级 | 池类型 | 适用场景 | 分配与安全策略 |
|---|---|---|---|
| **Tier 0** | **静态内联栈槽** `StackSlot<bound>` | 固定组合的轻量叶子/内部子状态机 | 直接内嵌在父组件或固定面板槽位中，零堆分配与零间接寻址 |
| **Tier 1** | **尺寸分级池** `BoundedStackPool` | 绝大部分编译器可定界组件（`128B / 256B / 512B / 1KB / 2KB / 4KB`，16B 对齐） | 依赖 Flurry 运行时依赖 allocator 分配连续页块切分；Debug 模式在栈底附带 canary redzone；Release 模式由静态上界保证不越界，省去每栈 `mprotect` guard page 系统调用 |
| **Tier 2** | **通用保护栈池** `StackPool` | 显式标注允许内部深递归、或栈内调用未知动态闭包且无法定界的组件 | 按需提交物理页 + guard page 防溢出，走标准溢出失败路径 |

> **量级估算**：若一个中等复杂页面有 10,000 个同时挂载的组件实例，均摊落入 256B 槽位，总栈内存仅约 **2.5 MB**（对比 64KB 默认栈池所需的 640 MB 降低两个数量级），且全部按分级空闲链表 O(1) 复用。

#### 分级池的 Flurry Allocator 签名示意

遵循 Flurry 运行时依赖类型 allocator 范式（不硬编码全局 `SystemAllocator`）：

```flurry
use std.mem.Allocator;
use std.mem.DefaultAllocator;

pub struct BoundedStackPool
where
    slot_bytes: usize,
    .A: Type = DefaultAllocator,
    requires slot_bytes >= 128usize and (slot_bytes % 16usize) == 0usize and A :- Allocator,
    quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
{
    -- 仅保存 slab 元数据与空闲位图/链表；默认全局 allocator 不额外增加存储开销
    free_head: ?*u8,
    slab_count: usize,
}
```

---

## 3. 统一事件模型：`pull_event` 与 `WidgetEvent`

对照 [scratch](file:///home/fin/luna/tmp/silver-design/scratch) 给出的事件分类并补齐父向子属性更新通道，定义组件挂起等待的核心事件原语：

```flurry
-- 注：部分跨界载荷类型（如 SceneUpdate / PropsPatch）属于与 Layer 4 衔接的占位类型

^attrs.packed.bit
pub struct KeyModifiers {
    shift: bool = false,
    ctrl: bool = false,
    alt: bool = false,
    super_key: bool = false,
}

pub enum KeyCode {
    backspace,
    enter,
    escape,
    delete,
    tab,
    char(char),
}

pub enum MotionEvent {
    pointer_down(f32, f32, u8),
    pointer_move(f32, f32),
    pointer_up(f32, f32, u8),
    pointer_cancel,
    wheel(f32, f32),
    key_down(KeyCode, KeyModifiers),
    key_up(KeyCode, KeyModifiers),
    focus_gained,
    focus_lost,
}

-- 注：DirtyMask 按动态运行期槽位下标索引，因此采用 BitVec<64>；
-- 具名固定标志位统一采用 ^attrs.packed.bit struct（编译期降码至 BitVec<w>）。
pub struct DirtyMask {
    bits: BitVec<64>,
}

pub struct ChildUpdate {
    child_slot: u32,
    kind: Symbol,
}

pub struct ExternalEvent {
    source_id: u64,
    tag: Symbol,
}

-- Custom 默认为 void，允许仅用标准事件流的组件省略泛型参数
pub enum WidgetEvent
where .Custom: Type = void
{
    user_motion(MotionEvent),
    child_update(ChildUpdate),
    state_update(DirtyMask),
    shared_state_update(u64),
    props_update(u64),
    custom_event(Custom),
    external_event(ExternalEvent),
}

-- 可捕获控制流的核心等待效应：Y 为收到的 WidgetEvent<Custom>
pub effect pull_event(catch continuation: builtin.Continuation) -> WidgetEvent<Custom>
where .Custom: Type = void;
```

### 调度器类型擦除 vs 组件内强类型

框架调度器需要把成千上万个不同 `Custom` 类型的组件放入同一个就绪队列和事件树，这里存在一个关键边界：
- **调度器侧**只持有统一边界的 `WidgetSlot`（封装有界栈上的 `builtin.Continuation` 句柄、父子关系与输入队列，具体安全封装见 [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md)）。
- **组件安装边界**把强类型的 `pull_event<Custom>` 适配到调度器的通用唤醒槽位，因此组件内部对 `.custom_event(payload)` 的模式匹配完全保持零装箱强类型。

---

## 4. Layer 1：手写协程组件直写范式

### 示例 A：多阶段交互状态机（`drag_handle`）

传统 retained GUI 实现拖拽手柄需要把 `is_dragging`、`start_x`、`origin_val` 全部提升到结构体字段并在单一 `on_event` switch 里做状态分支；在 Silver 里，拖拽态就是一个嵌套 `while pull_event()# is do { ... }` 循环：

```flurry
-- 假定 SceneNodeHandle 为绑定到场景树的 RAII 句柄（Layer 4 详细设计后确定）
pub fn drag_handle(initial_offset: f32) -> #pull_event<> void {
    var offset = initial_offset;
    let node = SceneOps.alloc_box(offset)#;

    -- 外层循环：空闲态
    while pull_event()# is do {
        .user_motion(.pointer_down(down_x, _, 0u8)) => {
            let start_offset = offset;
            SceneOps.set_highlight(node.id, true)#;

            -- 内层循环：拖拽态，直接通过词法栈帧捕获 down_x 与 start_offset
            var dragging = true;
            while dragging and (pull_event()# is do {
                .user_motion(.pointer_move(cur_x, _)) => {
                    offset = start_offset + (cur_x - down_x);
                    SceneOps.set_offset_x(node.id, offset)#;
                    true
                },
                .user_motion(.pointer_up(_, _, 0u8)) => false,
                .user_motion(.pointer_cancel) => {
                    offset = start_offset;
                    SceneOps.set_offset_x(node.id, offset)#;
                    false
                },
                _ => true,
            }) {}

            SceneOps.set_highlight(node.id, false)#;
        },
        .props_update(_) => {
            -- 父组件下发属性变更时处理
        },
        _ => (),
    }
}
```

**关键收益**：
- 若用户**拖拽到一半时父组件把该 `drag_handle` 卸载**（例如切换 Tab 页），调度器直接放弃该 `WidgetSlot` 的 `Continuation`，`node` 的 `Drop` 自动归还场景节点，没有任何悬空拖拽定时器或残留高亮状态；
- 因为 `SceneOps.*` 是界外 effect，`drag_handle` 栈帧仅含几个 `f32`/`bool` 与句柄，编译期栈上界维持在最小档（例如 128B/256B）。

### 示例 B：多 prompt 顺序模态流程

同一个组件可以自由组合多个不同 effect 而不冲突：

```flurry
effect confirm_delete(item_name: str, catch continuation: builtin.Continuation) -> bool;

pub fn editable_row(name: str) -> #[pull_event<>, confirm_delete, SceneOps] void {
    let row = SceneOps.alloc_row(name)#;

    while pull_event()# is do {
        .user_motion(.key_down(.delete, _)) => {
            -- 挂起在另一个 prompt 上等待外层弹窗确认，组件自身局部栈原封不动
            if confirm_delete(name)# {
                SceneOps.mark_deleted(row.id)#;
                return;
            }
        },
        _ => (),
    }
}
```

---

## 5. 安全性边界与静态约束

1. **禁止 C 帧跨 `pull_event()#` 挂起**  
   依据 Flurry [effect-abi.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md)，挂起区域中允许保留 C ABI 帧后再恢复，但**禁止放弃越过 C 帧屏障（否则触发运行时 trap）**。而组件卸载天然通过放弃 continuation 完成，因此 Silver 规范规定：**任何 C/平台 FFI 回调都必须通过界外 handler 调用或立即返回，严禁在组件挂起边界内持活 C 栈帧**（可由编译器跨边界检查保证）。
2. **`Continuation` 单次消费互斥**  
   事件派发（resume）与父级卸载（abort/drop）可能在同帧内指向同一 `WidgetSlot`。调度器必须以原子/独占转移语义取出 `Continuation`：卸载标志已置位时，待处理的事件派发直接丢弃，绝不二次切入已消费或正在中止的栈。
3. **`Drop` 契约收敛**  
   卸载时的 `Drop` 沿隐藏中止通道执行，不允许再调用可挂起等待用户输入的操作（例如在 `Drop` 里调用 `pull_event()#`），也不可交付未处理的非空控制字。

---

## 6. 待讨论问题（Open Questions）

- **Q1.1（`WidgetEvent` 扩展性）**：是采用统一泛型参数化 `enum WidgetEvent<Custom>` 配合内部转换适配器，还是拆分成多个独立细粒度 effect（如 `effect enum WidgetSignal { wait_motion() -> MotionEvent, wait_dirty() -> DirtyMask }`）让编译器按需只订阅用到的源？
- **Q1.2（`.props_update` 载荷传递）**：父组件向已挂载子协程组件传递增量 props 更新时，怎样在不引入通用堆装箱 `Any` 的前提下保持签名强类型与小栈开销？
- **Q1.3（栈上界静态估算与溢出降级）**：对于编译器判定“几乎确定小于 512B，但包含一次用户泛型回调”的情况，是直接拒绝进入 Tier 1 并要求升级到 Tier 2，还是允许用户用 `^attrs.stack_bound(...)` 显式标注并做编译期验证？
