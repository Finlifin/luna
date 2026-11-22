# 声明式 DSL、零宏响应式桥接与上下文系统

> **状态**：`Draft v0.1（讨论稿）`  
> **前置阅读**：[00-overview.md](file:///home/fin/luna/tmp/silver-design/00-overview.md)、[01-stack-instance-and-event-loop.md](file:///home/fin/luna/tmp/silver-design/01-stack-instance-and-event-loop.md)

---

## 1. 设计约束：零 CMTT、零宏、零 Projection Pipeline

依据 [scratch](file:///home/fin/luna/tmp/silver-design/scratch) 与 [2026-9-8-variadic-argument-collecting-rules.fl](file:///home/fin/vilya/examples/2026-9-8-variadic-argument-collecting-rules.fl)，Silver 上层 DSL 严格避免引入 CMTT 代码拼接、构造/模板宏或 projection pipeline，完全依靠 Flurry 的通用前端机制实现：

| 语言机制 | 在 Silver DSL 中的职责 | 对应规范来源 |
|---|---|---|
| `extended application` (`vbox { ... }`) | 将位置实参（子视图）与冒号属性（布局/样式/事件回调）分别收集 | [parameters.md](file:///home/fin/luna/docs/functions/parameters.md) |
| `VariadicCollector` / `ExtendedVariadicCollector` | 库级两阶段收集（`Assoc Tmp` + `Self`），支持就地栈暂存与自定义类型校验 | 原型文件（待合入 compiler 协议） |
| `inline for elem in tup'tuple_enumerate` | 编译期展开异构子视图元组与响应式绑定槽位，零 runtime 虚分发 | [inlining-comptime-code.md](file:///home/fin/luna/docs/comptime/inlining-comptime-code.md)、[view-taking.md](file:///home/fin/luna/docs/others/view-taking.md) |
| `LiteralInto` 字面量后缀 | 强类型物理/逻辑单位（`12px`、`1.5rem`、`200ms`） | [protocols.md](file:///home/fin/luna/docs/traits/protocols.md) |
| `lambda` 标记 + `post do` / `tactic lambda` | 响应式绑定 `bind(count_ptr) do |c| ...` 或简写事件处理器 | [parameter-elaboration.md](file:///home/fin/luna/docs/functions/parameter-elaboration.md) |
| `^attrs.effects.direct_passing` | `#Theme`、`#Locale`、`#FocusScope` 的自顶向下注入与部分覆盖 | [direct-passing.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/direct-passing.md) |

---

## 2. 对齐 Flurry 现行语法的 Collector 协议

原型文件 `2026-9-8-variadic-argument-collecting-rules.fl` 中存在若干早期伪代码写法（如 `::`、两元素解构 `(i, arg)`）。在 Silver 设计中按现行文档规范化如下：

> **编译器实现状态注记**：`VariadicCollector` / `ExtendedVariadicCollector` 属于待接入 `luna` elaboration 的既定语言扩展协议（详见 [04-compiler-wishlist.md](file:///home/fin/luna/tmp/silver-design/04-compiler-wishlist.md)）；此外，[reflection-and-layout.md](file:///home/fin/luna/docs/type/reflection-and-layout.md) 中已有针对名义结构体的 `meta.struct_field_names(T)` / `meta.struct_field_types(T)`，而面向匿名结构体 `Ps` 的同类字段反射与编译期字符串/符号投影访问亦已列入提案需求清单。

```flurry
use attrs.comptime_phase;

-- 位置变参收集协议（与 docs/traits/the-comptime-phases-of-traits.md 两相规范对齐）
pub trait VariadicCollector {
    assoc Tmp: Type;
    fn variadic_collect(comptime S: Type, args: S) -> Tuple<Tmp, Self>;

    ^comptime_phase
    assoc TmpComptime: Type;
    ^comptime_phase
    comptime fn variadic_collect_comptime(S: Type, args: S) -> Tuple<TmpComptime, Self>;
}

-- 扩展属性收集协议（接收匿名结构体类型 Ps）
pub trait ExtendedVariadicCollector {
    assoc Tmp: Type;
    fn extended_variadic_collect(comptime Ps: Type, properties: Ps) -> Tuple<Tmp, Self>;

    ^comptime_phase
    assoc TmpComptime: Type;
    ^comptime_phase
    comptime fn extended_variadic_collect_comptime(Ps: Type, properties: Ps) -> Tuple<TmpComptime, Self>;
}
```

### 结构原语（零独立栈） vs 独立组件边界（有界栈）

为了防止“每写一个 `text` 或 `vbox` 就分配一个 256B 栈”，Silver 明确区分两类节点：

1. **结构原语（Structural Primitives）**：`vbox`、`hbox`、`stack`、`text`、`spacer`、`rect`、`button_view` 等纯粹的布局与渲染描述符。它们**不分配独立有界栈**，而是作为外围组件栈帧内的普通聚合/描述值，由 `mount(...)` 统一挂载并持有轻量场景节点句柄。
2. **组件边界（Component Boundaries）**：`child(widget_fn, ...)` 或具名有状态组件调用点。它们通过界外 `spawn_child` effect 为子组件分配独立的 Tier 0 / Tier 1 有界栈，并向父组件返回一个轻量的 `ChildHandle`（负责位置关联与卸载清理）。

---

## 3. 从声明式 DSL 到 `while pull_event()#` 的零宏桥接

### 3.1 完整示例：Layer 2 计数器组件

```flurry
use silver.dsl.{ vbox, hbox, text, button, bind, mount, State }
use silver.theme.Theme;

pub fn counter(initial: i32) -> #[pull_event<>, Theme, SceneOps] void {
    -- 1. 状态直接生存在组件栈帧上（位于 mount 之前）；单次取址后借助接收者适配直接调用方法
    var count = State.new(initial);
    let count_ptr = count.ref;

    -- 2. extended application 描述视图树与细粒度响应绑定，零宏落入 pull_event 循环
    mount {
        vbox {
            gap: 12px,
            padding: 16px,
            background: Theme.surface_color()#,

            text {
                content: bind(count_ptr) do |value| format("当前计数: {}", value),
                color: Theme.text_primary()#,
            },

            hbox {
                gap: 8px,
                button {
                    label: "-1",
                    on_click: || count_ptr.set(count_ptr.get() - 1),
                },
                button {
                    label: "+1",
                    on_click: || count_ptr.set(count_ptr.get() + 1),
                },
            },
        }
    }
}
```

### 3.2 为什么这套写法安全、无锁且零堆开销？

1. **栈状态生命周期的词法逆序证明**：  
   `var count = State.new(initial)` 声明在 `mount { ... }` 之前。`mount` 在组件栈上阻塞至组件卸载（`pull_event` 被放弃）。Flurry 的自动清理保证**先销毁 `mount` 内部持有的所有视图节点与回调闭包，再销毁 `count`**。因此 `count.ref` 传入 `bind(...)` 与两个 `on_click` 闭包在静态 `&r T` 可达性检查下天然成立，**不需要任何 `Rc`、`Arc` 或堆上 `Box`**。
2. **`*T` 顺序别名免除 `RefCell`**：  
   依据 [pointers.md](file:///home/fin/luna/docs/resource-management/pointers.md)，Flurry 的普通 `*State<i32>` 不施加 Rust 式排他可变引用限制。在单线程 UI 调度下，两个 `on_click` 闭包与 `bind` 读取端合法共享同一个 `count.ref`，顺序读取最新状态版本。
3. **无虚拟 DOM Diff**：  
   静态属性（如 `gap: 12px`）只在 `mount` 首帧写入一次场景节点，**不生成任何循环更新代码**；只有通过 `bind(...)` 注入的响应式属性会被分配一个脏掩码槽位（slot bit），在事件循环里精确触发对应叶子属性的局部赋值。

### 3.3 `mount` 与 `ReactiveProp` 的内部实现骨架

下面展示不用任何宏、仅用泛型、`inline for` 与 `ExtendedVariadicCollector` 如何把上例编译成 `while pull_event()# is do { ... }`：

```flurry
-- 状态容器：保存值、所属组件的 slot 位索引、以及指向组件调度唤醒器的指针
pub struct State where T {
    value: T,
    slot_bit: u8,
    owner_dirty_mask: ?*DirtyMask,

    pub fn new(initial: T) -> Self {
        Self { value: initial, slot_bit: 0u8, owner_dirty_mask: null }
    }

    pub fn get(*self) -> T where T :- Copy {
        -- 目标引导读取：返回类型为 T 且 T :- Copy，无需额外写 .*
        self.value
    }

    pub fn set(*self, next: T) {
        self.value.* = next;
        -- 目标引导 Copy 读取 ?*DirtyMask 后直接通过接收者适配调用 mask.mark(...)，无需冗余 .*
        let maybe_mask: ?*DirtyMask = self.owner_dirty_mask;
        if maybe_mask is mask? do {
            mask.mark(self.slot_bit);
        }
    }
}

-- 每个挂载视图节点必须实现的静默绑定与局部更新接口（编译期单态化，非 dyn）
pub trait MountableView {
    assoc MountedState: Type;
    assoc BindingSet: Type;

    fn mount_initial(self, registry: *DirtyRegistry) -> Tuple<MountedState, BindingSet>;
    fn apply_dirty(mounted: *MountedState, bindings: *BindingSet, mask: DirtyMask);
    fn dispatch_motion(mounted: *MountedState, motion: MotionEvent) -> bool;
}

-- 零宏桥接核心：将声明式视图直接转化为与 scratch 一致的 pull_event 协程循环
pub fn mount(view: V) -> #[pull_event<>, SceneOps] void
where V :- MountableView
{
    var dirty_registry = DirtyRegistry.new();
    let (var mounted, var bindings) = view.mount_initial(dirty_registry.ref);

    while pull_event()# is do {
        .state_update(mask) => {
            -- 经 inline for 展开后，此处变为若干条平直的位测试与精确场景写回
            V.apply_dirty(mounted.ref, bindings.ref, mask);
        },
        .user_motion(motion) => {
            let _ = V.dispatch_motion(mounted.ref, motion);
        },
        .child_update(_) => (),
        .shared_state_update(sub_id) => {
            dirty_registry.mark_subscription_dirty(sub_id);
        },
        .props_update(_) => (),
        .custom_event(_) => (),
        .external_event(_) => (),
    }
}
```

以 `vbox { ... }` 为例，其 `BindingSet` 是一个**异构静态元组** `Tuple<TextBinding, HBoxBinding>`，而 `V.apply_dirty` 利用 `inline for` 展开：

```flurry
-- 编译器单态化展开示意（无需用户编写）
inline for child in mounted.children'tuple_enumerate {
    -- 按静态子类型精确分派；若某静态子视图（如 spacer）的 BindingSet 为 ()，整段更新被消除为空操作
    ...
}
```

### 3.4 属性分发：`ExtendedVariadicCollector` 的编译期校验

`vbox { gap: 12px, padding: 16px, ...children }` 在收集阶段由 `ExtendedVariadicCollector` 在编译期对匿名结构体 `Ps` 进行字段检查：
- 已知属性名（`gap`、`padding`、`background`、`align`）逐字段校验类型或转换协议（如 `px`）；
- 拼写错误的属性名（如 `pading: 16px`）在 `comptime` 阶段直接触发清晰诊断错误，**绝不在运行期静默忽略**。

---

## 4. Layer 1 与 Layer 2 的无缝双向嵌套

因为两者在运行时都是“运行在有界栈上、在 `pull_event()#` 处挂起的实体”，互操作不需要任何适配包装层：

### 4.1 声明式视图嵌入 Layer 1 手写协程组件

```flurry
vbox {
    gap: 8px,
    text { content: "调节滑块:" },
    -- 通过 spawn_child 效应分配独立有界栈运行 drag_handle
    child(drag_handle, 0.0f32),
}
```

### 4.2 手写协程组件内嵌 Layer 2 声明式子树

当协程组件只在某个交互分支或外围包装里想使用声明式布局时，可通过局部 `mount_subtree` 句柄直接驱动：

```flurry
pub fn collapsible_panel(title: str) -> #pull_event<> void {
    var expanded = State.new(false);
    let expanded_ptr = expanded.ref;
    var header = mount_subtree(hbox {
        text { content: title },
        button {
            label: bind(expanded_ptr) do |open| if open { "收起" } else { "展开" },
            on_click: || expanded_ptr.set(not expanded_ptr.get()),
        },
    })#;

    while pull_event()# is do {
        .state_update(mask) => header.apply_dirty(mask),
        .user_motion(motion) => header.dispatch_motion(motion),
        .custom_event(_) => (),
        _ => (),
    }
}
```

---

## 5. 无 VDOM 的动态控制流

静态 `vbox` / `hbox` 的子组件槽位在编译期即可确定；面对运行期条件分支和动态列表，Silver 提供两种显式控制流原语，直接管理子栈生死：

| 原语 | 底层状态存储 | 更新策略 | 生命周期语义 |
|---|---|---|---|
| `show_if(cond, then_view, else_view)` | Tagged Enum 持有当前活动分支的 `ChildInstance` | 条件翻转时仅当分支切换才生效 | 旧分支 `ChildInstance` 覆写销毁（触发放弃 `Continuation` 与逆序 `Drop`），新分支挂载 |
| `keyed_list(items, key_fn, item_widget)` | `Key -> ChildInstance` 槽位映射 + 顺序索引表 | 基于 `Key` 做集合增删与位置重排 | 仅重排时复用已有栈实例（保留焦点、滚动、进行中动画/网络请求）；仅删除键触发卸载；数据变化向对应子栈发送 `.props_update` |
| `virtual_list(...)` | 固定窗口容量的滑动 `ChildInstance` 槽池 | 仅对可视区 + overscan 窗口内索引维持栈实例 | 滑出可视区立即卸载或进入可重置池，内存占用恒定 `O(visible_count)` |

---

## 6. 上下文与跨组件共享状态：`^attrs.effects.direct_passing`

### 6.1 只读 / 配置型上下文（`Theme`、`Locale`、`FocusScope`）

依据 [direct-passing.md](file:///home/fin/luna/docs/algebraic-effect-and-delimited-continuation/direct-passing.md)，标记 `^attrs.effects.direct_passing` 的无参 effect 在安装时求值一次，且**支持只覆盖单项、自动穿透函数与组件边界，并在 continuation 恢复时自动获取恢复点最新供应值**：

```flurry
pub effect enum Theme {
    ^attrs.effects.direct_passing
    surface_color() -> Color,
    ^attrs.effects.direct_passing
    text_primary() -> Color,
    ^attrs.effects.direct_passing
    corner_radius() -> f32,
}

-- 局部子树只覆盖 surface_color，其余项目自动外传到外层 Theme handler
pub fn dark_card(child_view: V) -> #[Theme.text_primary, Theme.corner_radius, SceneOps] void
where V :- MountableView
{
    mount(child_view)# {
        Theme.surface_color() => Color.rgb(24u8, 26u8, 32u8),
    }
}
```

**关键机制**：当暗色主题切换为亮色主题时，外层调度器更新恢复点供应的 `Theme` handler 并向订阅组件发送 `.shared_state_update`；被挂起的组件栈恢复后调用 `Theme.surface_color()#` 即拿到新值，而其栈上的计数器、表单输入等局部状态纹丝不动。

### 6.2 可变共享状态（`Store<T>` + RAII 订阅守卫）

对于跨分支共享的可变状态（如全局认证状态、购物车），Silver 结合 direct-passing 与栈上 RAII 守卫：
1. 外层提供 `^attrs.effects.direct_passing effect app_store() -> *Store<AppState>;`；
2. 子组件通过 `let sub = store.subscribe(self_slot)#;` 在自己的组件栈上保存一枚 `SubscriptionGuard`；
3. 当 `Store` 更新时，遍历订阅表向目标 `WidgetSlot` 投入 `.shared_state_update(sub_id)`；
4. 当子组件卸载时，`SubscriptionGuard` 的 `Drop` 自动从 `Store` 订阅列表中摘除自己——**彻底杜绝观察者泄漏与僵尸回调**。

---

## 7. 待讨论问题（Open Questions）

- **Q2.1（依赖追踪方式）**：是采用显式的 `bind(count.ref) do |c| ...`（依赖静态明确、零运行期读屏障），还是额外提供隐式读追踪上下文（类似 Signals，书写更简洁但需记录读依赖图）？
- **Q2.2（`DirtyMask` 位宽扩展）**：单组件通常响应式属性远少于 64 个，使用 `BitVec<64>` 可零堆分配栈上内联；若超大型页面组件超过 64 个 `State`/绑定，是否通过编译器统计槽位总数并在大于 64 时由 `BitVec<n>` 参数化扩位？
- **Q2.3（属性块层级风格）**：参考 `scratch` 中的 `layout: { ... }` / `style: { ... }` 分组属性写法，是否比扁平属性（`padding: 16px, color: ...`）更利于 `ExtendedVariadicCollector` 分域收集与主题继承？
