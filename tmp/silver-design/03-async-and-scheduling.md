# 异步并发、多 Prompt 组合与帧调度模型

> **状态**：`Draft v0.1（讨论稿）`  
> **前置阅读**：[01-stack-instance-and-event-loop.md](file:///home/fin/luna/tmp/silver-design/01-stack-instance-and-event-loop.md)、[02-dsl-and-reactivity-bridge.md](file:///home/fin/luna/tmp/silver-design/02-dsl-and-reactivity-bridge.md)

---

## 1. 为什么 Silver 不需要 React Fiber

React 团队花费多年构造 Fiber，核心目的只有一个：**在不支持限定 continuation 的 JavaScript 引擎里，用堆链表模拟可中断、可重入、带优先级抢占的逻辑栈帧**，同时又因为组件函数必须重跑而不得不额外配一套 Hooks 数组、Effect 清理队列和 `AbortController` 手动取消逻辑。

在 Flurry 中，**限定 continuation + `async effect` + 结构化并发**由语言和运行时原生提供（参见 [async-algebraic-effect.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md) 与 [effect-abi.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/effect-abi.md)）：
- 暂停与恢复直接在原语级 `builtin.Continuation` 上以常数级栈链切入完成；
- 异步子任务归属于安装范围，提前退出自动取消；
- 帧预算调度、动画节拍和事件响应只是同一栈上的不同 prompt，彼此正交组合。

---

## 2. 组件内结构化异步与自动取消

### 案例：防抖异步搜索组件（无竞态、无卸载后更新泄漏）

```flurry
use silver.async.{ sleep_ms, http_get_text, AsyncTask }

pub fn search_box() -> #[pull_event<>, sleep_ms, http_get_text, SceneOps] void {
    var query = "".to_string();
    var results = "".to_string();
    -- 仅持有最新一次异步请求的句柄；被替换或组件卸载时自动 Drop -> 取消旧任务
    var active_task: ?AsyncTask<void> = null;

    let status_node = SceneOps.alloc_text("请输入关键词")#;

    while pull_event()# is do {
        .custom_event(next_query) => {
            query = next_query;
            -- 赋新任务前，旧 active_task 若仍在运行则先被 Drop（触发结构化取消），消除后发先至竞态
            active_task = spawn_scoped_task() do {
                let timer = sleep_ms(250u32)#;
                timer.await;

                let resp = http_get_text(query.as_str())#;
                let text = resp.await!;
                -- 通过 external_event 安全投递回组件自身事件队列
                post_self_event(.external_event(ExternalEvent { source_id: 1u64, tag: .search_ready }))#;
            }
        },
        .external_event({ source_id: 1u64, tag: .search_ready }) => {
            SceneOps.set_text(status_node.id, results.as_str())#;
        },
        _ => (),
    }
}
```

### 结构化生命周期 guarantees

1. **输入覆盖自动防抖取消**：用户连续键入字符时，将新的任务句柄写入 `active_task`，旧句柄在覆盖赋值路径上先触发 `Drop` 取消仍在等待的 `sleep_ms` 或在途 HTTP 请求，彻底消除响应乱序（out-of-order）问题。
2. **组件卸载强一致清理**：依据 [async-algebraic-effect.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/async-algebraic-effect.md)，含 `async` 分支或派生子任务的组件安装范围在提前退出（组件卸载导致 continuation 被放弃）时，先发起取消并等待相关清理完成，再释放依赖环境与组件栈槽。**“Update on unmounted component” 在类型与控制语义上无法发生。**

---

## 3. 多 Prompt 在同一组件栈上的正交组合

在单一协程组件栈上，多个限定效应可自由嵌套或顺序切换，而不需要改变组件实例模型：

| 效应 | 挂起时机 | 恢复携带值 `Y` | 典型用途 |
|---|---|---|---|
| `pull_event()#` | 等待下一条用户/状态/外部事件 | `WidgetEvent<Custom>` | 主事件循环与子交互状态机 |
| `pending.await` | 子 `async effect` 尚未完成 | 操作结果 `Y`（可含 `!Err`） | 文件读写、网络请求、防抖定时器 |
| `next_frame()#` | 等待下一 VSync 帧节拍 | `FrameInfo { dt_sec, now_ns }` | 手写物理弹簧动画、逐帧手势惯性滚动 |
| `frame_yield()#` | 当前帧时间预算已耗尽，主动让出 CPU | `void` | 大文档增量索引、长列表预解析等不卡帧后台批处理 |

### 示例：同一栈上的交互 + 惯性动画协程

```flurry
effect next_frame(catch continuation: builtin.Continuation) -> f32;

pub fn kinetic_scroller() -> #[pull_event<>, next_frame, SceneOps] void {
    var scroll_y = 0.0f32;
    var velocity = 0.0f32;
    let node = SceneOps.alloc_scroll_viewport(0.0f32)#;

    while pull_event()# is do {
        .user_motion(.wheel(_, dy)) => {
            velocity = velocity + dy * 120.0f32;
            -- 原地进入逐帧动画子循环，直到速度衰减至阈值或被打断
            while math.abs(velocity) > 0.5f32 {
                let dt = next_frame()#;
                scroll_y = scroll_y + velocity * dt;
                velocity = velocity * 0.92f32;
                SceneOps.set_scroll_y(node.id, scroll_y)#;
            }
        },
        _ => (),
    }
}
```

> **提示与双轨分工**：
> - 上述 `next_frame()#` 动画期间若需要随时响应新的 `.pointer_down` 打断惯性滚动，可进一步配合结构化 `select` 效应（同时等待 `pull_event` 与 `next_frame`），或由外部调度器在收到高优用户输入时先恢复并投递中断标志。
> - 对于不涉及布局重排的纯粹合成属性（位移、缩放、旋转、透明度、裁剪、Shader Uniform），Silver 默认采用 **Track 1 合成线程弹簧卸载**：组件仅单次提交物理弹簧参数，UI 协程可完全休眠，交互打断时通过 `PresentationState` 读取瞬时坐标与速度（详见 [05-layout-scene-and-rendering.md §6](file:///home/fin/luna/tmp/silver-design/05-layout-scene-and-rendering.md)）。`next_frame()#` 保留为耦合状态机与布局编排的 **Track 2 协程轨道**。

---

## 4. 帧调度器、优先级车道与同帧合并

### 4.1 三级调度车道

调度器运行在框架主栈上，管理全树 `WidgetSlot` 的唤醒队列，并划分为三条优先级车道：

| 车道 | 触发源 | 调度策略 |
|---|---|---|
| **Urgent（实时交互）** | `.user_motion`、活跃中的 `next_frame` 动画、焦点/IME 输入 | 本帧必须优先跑完至下一次挂起，保证零掉帧输入响应 |
| **Normal（响应更新）** | `.state_update`、`.shared_state_update`、`.props_update`、已完成的 `.await` 唤醒 | 在当前帧主更新阶段按脏组件**自顶向下（父优先于子）**拓扑批处理 |
| **Low / Idle（增量与后台）** | `frame_yield()` 恢复、离屏虚拟列表预热、非关键遥测 | 仅当距 VSync 截止时间仍有余量时分片推进；预算耗尽直接留到下一帧 |

### 4.2 同帧 `DirtyMask` 合并（Event Coalescing）

假设用户在一次点击回调里连续修改了 3 个局部状态（`count`、`label`、`dirty_flag` 均为栈上指针或经接收者适配自动取址调用）：
```flurry
count.set(count.get() + 1);
label.set("已点击");
dirty_flag.set(true);
```
- 第一次 `set` 将 `WidgetSlot.pending_dirty_mask` 置位，并把该 `WidgetSlot` 压入 `Normal` 队列一次；
- 第二、三次 `set` 仅对同一枚 `BitVec<64>` 执行按位 `or`，检测到该 `WidgetSlot` **已在队列中**，不重复入队；
- 到了本帧调度阶段，组件栈**只被恢复一次**，在单次 `.state_update(mask)` 分支里通过 `inline for` 批量完成这 3 处绑定更新。

### 4.3 线程边界与多线程协作

- **UI 主线程**：所有 `WidgetSlot` 栈帧的挂载、恢复与销毁均绑定到 UI 线程执行（遵守 Flurry `multi-prompt-delimited-continuation.md` 中“挂起计算不跨线程恢复”的契约）。这保证组件内 `*State<T>` 顺序别名永远无数据竞争。
- **后台计算与 I/O 线程**：重 CPU 任务与系统 I/O 通过 `async effect` 交由 `std.runtime` 工作池执行；跨线程交付数据遵守 Flurry `Send + Sync` 或 `docs/concurrency/prompt.fl` 中的轻量 STM（`Transactional`），完成后由运行时唤醒主线程调度队列恢复等待中的 `.await` 或投递 `.external_event`。

### 4.4 一帧的调度节拍（Frame Tick 与双线程无锁提交）

```text
[VSync / Phase-Offset Wakeup + 输入重采样]
  1. 排空平台原始输入 → 命中测试（SoA HitTest）→ 向目标 WidgetSlot 投递 Urgent 事件
  2. 执行 Urgent 车道（处理交互状态机与 Track 2 协程动画；回调写入 State 产生 Normal 任务）
  3. 自顶向下排空 Normal 车道（合并后的 DirtyMask / props_update / 异步回调完成）
  4. 执行 Layer 4 增量 SoA 布局与遮挡裁剪，写入双缓冲 FrameArena[frame_idx % 2] 零指针 PrimitiveStream
  5. 微秒级无锁交换 FrameCommit 交给渲染线程（在途帧 ≤ 2，UI 与 Vulkan/WGPU 渲染重叠推进）
  6. 若距下一次 VSync 尚有预算 → 推进 Low/Idle 车道中的 frame_yield 任务；归还释放的有界栈槽
```

详细双线程提交时序与显式同步围栏见 [05-layout-scene-and-rendering.md §4–§5](file:///home/fin/luna/tmp/silver-design/05-layout-scene-and-rendering.md)。

---

## 5. 待讨论问题（Open Questions）

- **Q3.1（动画时打断机制）**：在 `kinetic_scroller` 这类进入动画循环的协程组件里，是提供一个同时监听 `pull_event` 与 `next_frame` 的多路选择原语（`select_event_or_frame()#`），还是让大多数线性过渡动画下沉到 Layer 4 场景树属性动画器，只把复杂交互物理留给协程？
- **Q3.2（父先子后的拓扑唤醒）**：当父子组件在同一帧同时既有 `.props_update` 又有 `.state_update` 时，调度器如何维护低成本的树深度序（如记录 `depth: u16` 桶队列）以防止同一帧内子组件被无意义唤醒两次（glitch）？
- **Q3.3（跨帧协程与资源占留）**：若某 `Low` 车道任务执行了一半通过 `frame_yield()#` 挂起，而同时组件被父级卸载，放弃协议会中止该后台任务——是否需要对大批量计算提供“与组件解耦但绑定窗口寿命”的独立后台作用域？
