# Flurry 演进手记 (043)：当对象内部指向自己——自引用结构（Self-Reference）的移动与安全锚定

在 [上一篇 (042)](./042-partial-moves-and-reinitialization.md) 中，我们学会了对结构体进行部分移出与重新初始化。当时我们留了一个让无数系统程序员头皮发麻的悬念：**如果一个结构体的某个字段，指向了同一个结构体里的另一个字段（即自引用结构，Self-Referential Struct），会发生什么？**

在开发我们的零拷贝 `RingBuffer` 和协议帧解析器时，这个需求简直无处不在：
- 我们希望把**原始数据缓冲区（Owner）**和**当前解析出来的切片视图（View / Cursor）**打包在同一个结构体里，方便在流水线各个阶段之间传递；
- 或者在构建侵入式环形链表节点时，节点里既有内联的数据字段 `value`，又有一个指向上游或自身字段的指针 `pointer`。

如果你在 Rust 里干过这件事，你一定经历过这样的难题：
1. 哪怕你的切片指向的是 `String` 或 `Vec` 在**堆上**分配的内存（按值移动 `String` 根本不会改变堆内存的地址。），Rust 的借用检查器也无法区分“指向栈上描述符”和“指向堆上缓冲区”，一律严禁你把 `storage: String` 和 `view: &str` 装进同一个结构体按值移动；
2. 为了写出自引用结构，你不得不引入复杂的 `Pin<&mut Self>`、`PhantomPinned` 和一堆 `unsafe` 裸指针转换。

为什么 Flurry 不需要发明 `Pin` 这种打补丁式的语法机制，就能把自引用结构管得既灵活又滴水不漏？
秘密就在于：Flurry 的移动与可达性分析，**精确区分了“指向独立外部/堆分配的视图”与“指向对象内联字段的指针”**（详见 `docs/resource-management/moves-and-initialization.md`）。

让我们从一段把这两种情况混为一谈的 初始代码开始。

---

## 一、 两种截然不同的“自己指自己”

来看看工程师在设计零拷贝数据包时写下的两个自引用结构体：

```flurry
use std.prelude.*;

-- 结构 A：storage 拥有堆上的字符分配，view 指向那块堆内存。
struct TextPacket {
    storage: String,
    view: str,
}

-- 结构 B：value 是直接内联在结构体栈槽里的 i32，pointer 指向 local.value。
struct InlineLink {
    value: i32,
    pointer: ?*i32,
}
```

现在问题来了：
- 如果我们在函数里构造了一个 `TextPacket`，能把它按值 `return` 给调用者吗？
- 如果我们在函数里构造了一个 `InlineLink`，让 `local.pointer = local.value.ref;`，能把它按值 `return` 给调用者吗？

下面通过物理内存布局，看看当这两个结构体发生**按值移动（Move）**时，底层到底发生了什么。

---

## 二、 移动 Owner 描述符不搬走堆分配——`TextPacket` 为什么能自由移动？

先看第一个结构体 `TextPacket { storage: String, view: str }`。
当我们写下：

```flurry
fn make_packet() -> TextPacket {
    let storage = "hello".to_string();
    let view = storage.as_str();
    TextPacket { storage: storage, view: view }
}

fn pass_packet(packet: TextPacket) -> TextPacket {
    packet
}
```

下面看看 `make_packet()` 把 `TextPacket` 按值移动给调用者时的内存画卷：

```text
  栈帧 A (make_packet)               栈帧 B (调用者)                 堆内存 (Heap Allocation)
  ┌─────────────────────┐            ┌─────────────────────┐         ┌───┬───┬───┬───┬───┐
  │ storage (描述符)     │ ──(Move)──►│ packet.storage      │ ───────►│'h'│'e'│'l'│'l'│'o'│
  ├─────────────────────┤            ├─────────────────────┤         └───┴───┴───┴───┴───┘
  │ view    (切片视图)   │ ──(Move)──►│ packet.view         │ ──────────▲
  └─────────────────────┘            └─────────────────────┘           │
  [已移出，不再清理]                 [接管责任，继续指向同一块堆内存。]─┘
```

真相大白。
`view` 指向的根本不是 `storage` 在栈上的那几个字节描述符，而是 `storage` 所拥有的**外部独立字符分配（Heap Allocation）**。
当 `TextPacket` 从 `make_packet` 按值移动到 `pass_packet` 再移动到 `test` 时：
1. 移动的只是栈上的两个描述符（`storage` 和 `view`）；
2. 底层的字符分配稳稳地呆在原地，连半个字节都没有挪动过。
3. 因此，在 Flurry 中，**`make_packet()` 和 `pass_packet(packet)` 100% 合法通过，无需任何 `Pin` 或 `unsafe`。**

### 编译器在出口处真正关心的是什么？

既然 `TextPacket` 可以按值整体移动，那如果我们对 `TextPacket` 做上一篇学过的**部分移出（Partial Move）**呢？
对比下面这两个函数，你将立刻领悟 Flurry 编译器出口检查的精髓：

```flurry
-- ✅ 函数 1：移出并返回 owner (storage)，完全合法。
fn take_storage(packet: TextPacket) -> String {
    packet.storage
}

-- ❌ 编译期直接拒绝。
-- 函数 2：只返回 view，却把 owner (storage) 留在函数里销毁。
fn take_view(packet: TextPacket) -> str {
    packet.view
}
```

为什么 `take_storage` 合法，而 `take_view` 被编译期拒绝？
因为在函数退出时，编译器严格按照以下顺序记账（详见 `docs/resource-management/moves-and-initialization.md`）：
1. **先转交返回值**；
2. **再对函数内剩余未移出的局部责任执行清理（Drop）**；
3. **最后核验返回值所依赖的目标在清理之后是否依然存活有效**。

在 `take_storage` 中，字符分配的拥有权跟着 `packet.storage` 一起交给了返回值，留在函数里的残余 `packet.view` 只是平凡清理（不释放字符），所以返回值 100% 有效；
而在 `take_view` 中，你试图把 `packet.view` 返回出去，却把真正的金主 `packet.storage` 留在了函数内部。一旦函数退出，`packet.storage` 就会被自动清理并释放堆上的 `"hello"`，导致返回的 `str` 瞬间变成悬垂视图——编译器在静态出口检查时一举将其拦截。

---

## 三、 内联字段地址不随移动迁移——何时禁止按值移动与安全锚定

现在，我们来看第二种自引用结构：`InlineLink { value: i32, pointer: ?*i32 }`。
注意：这一次，`value: i32` **不是堆分配**，它是直接镶嵌在 `InlineLink` 结构体肚子里的**内联字段（Inline Field）**。

如果有人写下这样的代码，试图在函数里把 `local.pointer` 指向 `local.value.ref`，再把 `local` 按值返回出去：

```flurry
-- ❌ 编译期直接拒绝。
struct InlineLink {
    value: i32,
    pointer: ?*i32,
}

fn escape_inline() -> InlineLink {
    let local = InlineLink { value: 7i32, pointer: null }
    -- 让 pointer 指向 local 自己的内联字段 value
    local.pointer = local.value.ref;
    -- 错误：按值移动不会自动把 pointer 修复为指向新位置。
    -- local.pointer 依然指向当前即将销毁的栈槽 local.value。
    local
}
```

下面来看看为什么这次按值移动会导致严重错误：

```text
  栈帧 A (escape_inline)                     栈帧 B (调用者接收返回值)
  ┌─────────────────────────┐                ┌─────────────────────────┐
  │ local.value: 7          │ ◄──────────┐   │ ret.value: 7            │
  │ (地址: 0x7fff_0010)     │            │   │ (新地址: 0x7fff_0080)   │
  ├─────────────────────────┤            │   ├─────────────────────────┤
  │ local.pointer:          │            └── │ ret.pointer:            │
  │   0x7fff_0010           │ ───(按位移动)─►│   0x7fff_0010 (悬垂  💀)│
  └─────────────────────────┘                └─────────────────────────┘
  [函数退出，0x7fff_0010 栈槽失效。]
```

看清楚了吗？
**移动（Move）只转移值的字节表示与责任，它绝不会自动重写或修复指针对内联字段的指向。**
即使底层的代码生成器（如 LLVM）在某个优化等级下碰巧触发了返回值优化（RVO / NRVO），省掉了栈拷贝，Flurry 的语言规范也明确规定：**普通返回优化绝不作为源程序合法性的依据。** 只要语义上发生了按值移动，内联字段的原地址就会在源作用域结束时失效。

那么，在 Flurry 中，我们该如何**安全地使用内联自引用结构**？
答案是：**在最终承载它的宿主作用域中就地锚定（Anchor in Place），并在建立内联自引用后通过指针 `*InlineLink` 访问它，不再按值搬移整个结构体。**

```flurry
use std.prelude.*;

struct InlineLink {
    value: i32,
    pointer: ?*i32,

    -- 通过指针接收者 `*self` 在原位建立自引用连接。
    fn anchor_self(*self) {
        self.pointer.* = self.value; -- self.value 即为字段指针 *i32，自动注入 ?*i32
    }

    fn read_via_self_link(*self) -> ?i32 {
        let ptr? = self.pointer.* else {
            null
        }
        ptr.*
    }
}

test {
    -- 1. 在宿主作用域就地构造对象
    let node = InlineLink { value: 42i32, pointer: null }

    -- 2. 在原位建立内联自引用（node 从此不再按值移动）
    node.anchor_self();

    -- 3. 哪怕修改 node.value，通过内部自引用指针也能实时读到最新值。
    node.value = 99i32;
    assumes node.read_via_self_link() is 99i32?;
}
```

没有任何 `Pin<Box<...>>` 的样板噪音。编译器直接根据 `node.pointer` 对 `node.value` 的真实可达依赖为你守门：只要你不把已经锚定了内联指针的 `node` 按值搬走到外层作用域，你在原地怎么通过方法或指针读写它都畅通无阻。

---

## 四、 顺便分清类型系统里的“自我三兄弟”——`Self`、`Itself` 与 `itself`

既然聊到了“指向自己（Self-Reference）”，在阅读 Flurry 代码和编写类型契约时，你还会遇到三个长得很像的关键字：**`Self`**、**`Itself`** 和 **`itself`**（详见 `docs/type/self-reference.md`）。

如果不提前分清它们，当你在结构体方法里写精化类型（`refines { ... }`）约束某个参数时，很容易指错人。
让我们用一张对比卡片把这三兄弟的身份焊死：

```text
  struct RingCounter {
      count: u32,

      fn check_meta(
          *self,                                         ◄── `self`: 当前方法的接收者实例
          meta: refines { Type.predicates.is_tuple(Itself) } Any
      ) -> Self                                          ◄── `Self`: 词法所属的外层类型 `RingCounter`
           ────            ───────────────────────────────
                                          ▲
                                          └── `Itself`: 当前精化子句正在检查的目标类型（即参数 `meta` 的类型。）
  }
```

总结这三兄弟的精确分工：
1. **`Self`（首字母大写）**：永远指向**当前词法所属的类型**（或 `trait` / `impl` 的实现主体），比如在 `struct RingCounter` 内部，`Self` 就是 `RingCounter`。
2. **`Itself`（首字母大写）**：指向**最内层类型约束正在检查的那个目标类型**。比如在参数 `meta: refines { Type.predicates.is_tuple(Itself) } Any` 中，`Itself` 指的是调用者传给 `meta` 的具体类型（而不是外层的 `RingCounter`。）。
3. **`itself`（全小写）**：指向**当前值级精化约束正在检查的那个具体值对象**（或在成员签名中作为 `*itself` 接收者关系槽）。注意：当只有类型级要求（没有实际运行时值对象）时，只能用 `Itself`，不能把 `Itself` 改成小写的 `itself` 当作普通变量去读。

来看看它们协同工作的完整代码示例：

```flurry
use std.prelude.*;

struct RingCounter {
    value: u32,

    -- `Self` 指代词法外层类型 `RingCounter`
    fn next(self) -> Self {
        Self { value: self.value + 1u32 }
    }

    -- `Itself` 指代参数 `meta` 被传入的具体类型，要求它必须是一个元组类型。
    fn with_tuple_meta(
        *self,
        meta: refines { Type.predicates.is_tuple(Itself) } Any,
    ) -> u32 {
        self.value.*
    }
}

test {
    let c1 = RingCounter { value: 10u32 }
    let c2 = c1.next();

    -- ✅ 传入元组 `(1u32, true)`，满足 `is_tuple(Itself)` 精化要求。
    let res = c2.with_tuple_meta((1u32, true));
    assumes res == 11u32;

    -- ❌ 如果写 `c2.with_tuple_meta(42u32);`，编译器会编译期拒绝，因为 `u32` 不是元组。
}
```

---

## 本篇小结

在这一篇里，我们攻克了系统编程中令人闻之色变的自引用结构，并理清了“自身”指称词的边界：

| 概念 / 构造 | 核心行为与编译器规则 | 为什么比传统方案更优雅？ |
| :--- | :--- | :--- |
| **指向堆分配的自引用 (`TextPacket`)** | 移动结构体只搬移栈描述符，不搬移底层堆内存，按值返回**合法** | 无需像 Rust 那样因借用检查器限制而被迫引入 `Pin` 或第三方库 |
| **部分移出堆 Owner (`take_view`)** | 若只返回 `view` 而将 `storage` 留在函数内清理，静态**拒绝** | 出口检查自动在执行剩余 `Drop` 后核验返回值的依赖有效性 |
| **指向内联字段的自引用 (`InlineLink`)** | 移动不自动重写内联指针；禁止将指向局部内联字段的对象按值返回 | 在宿主栈帧原位锚定并通过 `*InlineLink` 读写即可，安全且零运行时开销 |
| **`Self` vs `Itself` vs `itself`** | `Self` 指词法所属类型；`Itself` 指最内层受检类型；`itself` 指当前受检值 | 在嵌套类型约束与精化类型中指代永远明确，绝不发生内外层混淆 |

### 下一篇预告

在讨论 `take_storage` 和 `take_view` 时，我们提到了一个关键动作：**“函数退出时，对剩余未移出的责任执行自动清理（Drop）”**。

当我们构建一个复杂的 `RingBuffer` 控制器——里面既有外层的局部守护变量，又有结构体 `Bundle { first: Trace, second: Trace }`，并且 `Bundle` 自己也实现了 `Drop`，它的每个字段 `Trace` 也实现了 `Drop` 时：
1. **到底是先执行父对象 `Bundle` 自己的 `Drop`，还是先执行它肚子里字段 `first` 和 `second` 的 `Drop`？**
2. **结构体里的字段 `first` 和 `second`，到底是按正序（先 `first` 后 `second`，像 Rust 那样）清理，还是按逆序（先 `second` 后 `first`，像 C++ 那样）清理？**
3. 如果在构造 `Bundle` 的半路上，`first` 构造成功了，但构造 `second` 时通过 `!` 报了个错提前返回了——此时 `Bundle` 的 `Drop` 会被调用吗？
4. 如果某个对象的 `Drop` 函数在执行时需要去读取另一个局部变量 `text` 的内容，编译器是怎么防止 `text` 在清理前被提前破坏的？

在下一篇 **《Flurry 演进手记 (044)：析构函数背后的精密钟表——局部与聚合 `Drop` 次序、失败路径与环境约束》** 中，我们将拆开 Flurry 自动清理引擎的钟表齿轮。
