# Flurry 演进手记 (062)：高阶回调里的资源契约——当闭包遇到内存效果与可达性验证

> **系列导读**：在上一期（061）中，我们通过结果分离逻辑（OSL）和 `where outcomes` 子句，让一阶函数的内存脚印（Footprint）变得像数学公式一样精确。但在现代系统编程中，我们经常编写接收闭包回调的高阶函数——比如遍历 `RingBuffer` 并对每个元素执行回调的 `drain_with`。当一段函数在循环中途调用了外部传入的 `%c fn` 回调时，验证器如何确定回调没有偷偷篡改 `RingBuffer` 的游标（重入问题）？又如何确定循环内部的局部计数器没有被未知回调破坏？今天，我们将探索高阶回调、未暴露局部变量隔离与可调用体身份追踪的验证奥秘。

---

## 一、高阶回调的难题：传统方案的“重入篡改（Reentrancy Hazard）”

假设我们要为 `RingBuffer` 实现一个高阶批处理消费函数 `drain_with`：它弹出 `count` 个元素，每弹出一个就调用一次用户传入的 `callback`。

在第 017 期中我们学过，Flurry 的高阶函数通过闭包规格参数 `%c fn(...) -> R where c: Type.vars.Closure` 来声明。让我们先写出没有任何跨回调保护的初始写法：

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

-- 在循环中调用回调，但当回调重入修改 buf 时会发生什么？
fn drain_with_naive(
    buf: *RingBuffer,
    count: usize,
    callback: %c fn(i32) -> void,
) -> void
where
    c: Type.vars.Closure,
    requires valid_ring(buf) and count <= buf.len.*,
    ensures valid_ring(buf)
{
    var step = 0usize;
    while step < count {
        invariant step <= count;
        invariant count <= buf.len.*;
        invariant valid_ring(buf);
        decreases count - step;

        let item = buf.slots.*[buf.head.*];
        buf.head.* = (buf.head.* + 1usize) % 8usize;
        buf.len.* = buf.len.* - 1usize;

        -- ⚠️ 危险时刻：调用外部传入的 callback。
        -- 如果 callback 的闭包环境里捕获了同一个 buf 的指针，
        -- 并在里面把 buf.head 改成了 999usize 或者把 buf.len 清零了怎么办？
        callback(item);

        step = step + 1usize;
    }
}
```

看看上面代码中的 `callback(item)` 这一行。
在可变容器迭代中，这对应经典的**迭代器失效与回调重入问题（Callback Reentrancy Bug）**：调用者传入了一个捕获了 `buf` 指针的闭包，在 `drain_with` 循环执行到一半时，闭包内部再次修改了 `buf` 的游标或长度，导致外层循环的 `count <= buf.len.*` 和 `valid_ring(buf)` 瞬间崩塌。

如果编译器不能阻止或识别这种重入冲突，那么 `drain_with` 里的循环不变量 `invariant valid_ring(buf);` 在跨过 `callback(item)` 时就根本无法得证。

---

## 二、第一重保障：未暴露局部变量（Unexposed Locals）的天然免疫性

在解决 `buf` 的重入问题之前，我们先看一个有趣的问题：在上面的 `while` 循环里，除了 `buf` 之外，还有局部变量 `step` 和按值传入的参数 `count`。
当执行完未知的 `callback(item)` 后，证明器怎么知道 **`step` 和 `count` 没有被 `callback` 偷偷改掉**？

难道我们也需要为 `step` 写一堆分离逻辑证明吗？**完全不需要。**

### 地址暴露追踪（Address Exposure Tracking）与栈帧隔离

Flurry 的程序验证器会自动追踪每一个局部绑定的**地址暴露状态**：

```text
[ drain_with 的栈帧 (Stack Frame) ]
  ┌──────────────────────────────────────────────────────┐
  │ var step = 0usize;   (从未取过 .ref 传递到外部  )     │
  │ let count = ...;     (按值传递的标量绑定，未暴露地址) │
  │                                                      │
  │ 🔒 物理隔离墙：没有任何外部指针或闭包环境能到达这里   │
  ├──────────────────────────────────────────────────────┤
  │ buf: *RingBuffer     (外部传入的指针，可能存在别名)   │
  └──────────────────────────────────────────────────────┘
```

只要一个局部变量（如 `step` 或下例中的 `remaining`）没有把自己的指针（`.ref`）泄露给回调能够触及的范围，验证器就把它标记为 **Unexposed Local（未暴露局部绑定）**。无论中间调用了多少次未知的高阶回调，关于未暴露局部变量的事实（Facts）都会**无条件自动跨过回调调用**：

```flurry
fn invoke_with_preserved_local(
    callback: %c fn() -> void,
    count: i32,
) -> result: i32
where
    c: Type.vars.Closure,
    requires count > 0i32,
    ensures result > 0i32
{
    var remaining = count;
    -- 即使 callback 是一个完全未知的外部闭包，
    -- 因为 remaining 从未暴露过地址，callback 绝对不可能触碰 remaining。
    callback();
    -- ✅ 编译期直接得证：remaining > 0i32 跨越 callback() 自动保留。
    remaining
}
```

---

## 三、第二重保障：可调用体身份追踪（Known Callable Identity）与闭包环境分离

解决了局部变量 `step` 的保留问题后，我们回到核心矛盾：如何保证 `callback(item)` 不会破坏 `buf: *RingBuffer` 的状态？

在 Flurry 中，当高阶函数在编译期单态化（Monomorphization）或通过内存效果验证时，编译器会沿着调用链追踪可调用体的**代码身份（Code Identity）**与**捕获环境脚印（Captured Environment Footprint）**：

1. **纯函数与无捕获 Lambda（Pure / Non-capturing Callable）**：
   如果传入的 `callback` 是一个纯函数（如 `P4`/`P5` 纯度）或只读捕获外部常量的闭包，它的 `outcomes` 根本不包含对 `buf` 的 `write` 效果。验证器直接根据上一期学过的 **Frame Rule** 判定：`valid_ring(buf)` 跨过 `callback(item)` 保持不变。
2. **捕获了外部状态的闭包（Stateful Closure）——要求内存视图分离**：
   如果传入的 `callback` 捕获了一个外部累加器指针（例如 `acc.ref`），并在回调中累加元素，那么只要 `acc.ref` 与 `buf` 指向的内存区域**不相交（Disjoint）**，回调的 `write<acc>` 效果同样与 `buf` 完全正交。
3. **恶意或意外捕获了同一个 `buf` 的重入闭包——编译期直接拦截。**

让我们看看在第一部分中，当调用者试图把一个会篡改同一个 `buf` 的闭包传给 ` verified fn drain_with` 时，编译器是如何精准拦截的：

```flurry
verified fn drain_with(
    buf: *RingBuffer,
    count: usize,
    callback: %c fn(i32) -> void,
) -> void
where
    c: Type.vars.Closure,
    requires valid_ring(buf) and count <= buf.len.*,
    ensures valid_ring(buf),
    ensures buf.len.* == (buf.len.*)'old - count
{
    var step = 0usize;
    while step < count {
        invariant step <= count;
        invariant count - step <= buf.len.*;
        invariant valid_ring(buf);
        invariant buf.len.* == (buf.len.*)'old - step;
        decreases count - step;

        let item = buf.slots.*[buf.head.*];
        buf.head.* = (buf.head.* + 1usize) % 8usize;
        buf.len.* = buf.len.* - 1usize;

        callback(item);
        step = step + 1usize;
    }
}

fn demo_callback_verification(buf: *RingBuffer) -> i32
where
    requires valid_ring(buf) and buf.len.* >= 2usize
{
    var external_sum = 0i32;

    -- ✅ 正例：闭包只捕获了与 buf 完全分离的局部栈变量 external_sum.ref，
    -- 它的写脚印 write<external_sum> 与 buf 互不相交，验证完美通过。
    let sum_ptr = external_sum.ref;
    drain_with(buf, 2usize, |x: i32| {
        sum_ptr.* = sum_ptr.* + x;
    });

    -- ❌ 编译期直接拒绝：（重入篡改拦截）
    -- 下面这个闭包捕获了 buf 自身并在回调中篡改了 buf.len，
    -- 其写脚印 write<buf.len> 与 drain_with 循环不变量保护的 buf 发生冲突。
    drain_with(buf, 1usize, |x: i32| {
        buf.len.* = 0usize;
    });

    external_sum
}
```

---

## 四、深入编译器内部：跨分支与高阶工厂的“可调用体身份保留”

你可能会问：如果可调用体不是直接以内联匿名函数的形式传进去的，而是先存进了局部变量、经过了 `if/else` 分支选择，或者是从一个工厂函数返回出来的，Flurry 的纯度与验证检查器还能认出它的真实身份吗？

答案是：**能。** Flurry 的静态分析器专门设计了**可调用体身份追踪（Callable Identity Tracking）**：

```text
[ 已知身份来源 ]
  ├── 具名函数 (如 invert)
  ├── 无捕获 / 只读值捕获的 Lambda
  └── 返回已知可调用体的高阶工厂函数 (如 make_inverter(flag))
          │
          ├──► 经过局部 let / var 绑定转发
          ├──► 经过 if / else 或 match 分支合流（当所有分支均满足纯度/效果要求）
          ▼
[ 高阶调用点 op(value) ] ──► 依然保留精确的纯度 (P4) 与效果契约。
```

让我们看一段展示这一能力的真实 Flurry 代码：

```flurry
pure fn invert(value: bool) -> bool {
    not value
}

pure fn keep(value: bool) -> bool {
    value
}

-- 高阶调用方：只有当传入的 op 也是纯函数时，apply_bool 整体才能维持 pure (P4)
pure fn apply_bool(
    op: %c fn(bool) -> bool,
    value: bool,
) -> bool
where
    c: Type.vars.Closure,
{
    op(value)
}

-- 即使可调用体经过了条件分支选择，只要每个分支的目标都是纯可调用体，
-- 编译器依然能够证明合流后的 chosen 满足 pure 要求。
pure fn apply_branch_selected(select_invert: bool, value: bool) -> bool {
    let chosen = if select_invert {
        invert
    } else {
        keep
    }
    apply_bool(chosen, value)
}
```

同样地，如果一个闭包只按值捕获了不可变的外部标量（Read-only Value-Capturing Closure），编译器也会精准识别出它的调用不会产生任何外部 `write` 效果，从而允许它在 `pure fn` 和严格的 `verified fn` 循环不变量中自由穿梭。

---

## 五、本节核心概念速查表

| 验证机制 | 核心规则 | 解决的核心问题 |
| :--- | :--- | :--- |
| **未暴露局部变量隔离** | 从未将地址泄露给外部的局部绑定（如循环计数器 `step`），其状态事实自动跨越任何回调调用 | 免去为局部标量编写繁琐的分离逻辑不变量 |
| **闭包脚印分离检查** | 回调闭包的 `outcomes` 写脚印不能与外层循环正在维护不变量的目标（如 `buf`）发生非法重叠 | 在编译期彻底消灭迭代器失效与回调重入（Reentrancy）漏洞 |
| **可调用体身份追踪** | 跨局部绑定、分支合流与高阶工厂追踪具名函数与纯闭包的精确身份 | 让函数式高阶抽象与零开销形式化验证兼得 |

---

## 六、下一期预告

到这里，我们已经掌握了用 `verified fn`、`outcomes` 和高阶回调契约来保护 `RingBuffer` 的状态转移。
但是，如果我们的 `RingBuffer` 里存放的不是简单的 `i32` 整数，而是我们在第三卷开头（041–042 期）见过的**不可复制的线性资源（如 `OwnedChunk`、`Socket` 或带有自定义 `Drop` 的结构体）**呢？

当一个函数在复杂的 `if/else` 分支或循环中，**只移走（Partial Move）了结构体的某一个字段**，或者在有的分支移走了资源、有的分支没有移走，编译器到底是如何精确知道在函数退出或错误早退时“哪些字段该 Drop、哪些字段不能 Drop、按什么顺序 Drop”的？此外，为什么在 Rust 中只要实现了 `Drop` 就彻底禁止局部移动字段，而在 Flurry 中却依然允许安全地移走部分字段？

下一期 **《Flurry 演进手记 (063)：揭秘编译器的局部移动与 Drop 状态机——当复杂分支与循环交织时》**，我们将深入编译器内部的位置树（Location Tree）与 Drop Elaboration 引擎。
