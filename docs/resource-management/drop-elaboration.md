# 自动清理

## Intro

一个值离开清理范围时，程序会结束它仍持有的责任。若责任已经交给别处，源位置就不再清理；若只交出部分字段，程序就只清理剩余部分。

清理可以输出、释放分配或读取其他对象。因此，程序需要知道清理的确切次序，也要让清理使用的环境活到清理完成。

## 语法

~~~text
drop_implementation -> impl Drop for expr clauses? { definition* }
drop_method -> fn drop(*self) block
~~~

Drop 的接口结果是 void；实现不能私自增加接口未允许的错误或效应要求。

## 正式定义

### 局部责任

退出词法作用域时，程序按局部绑定的声明逆序清理仍持有的值。赋值不改变绑定次序；编译器跳过未初始化和已移出位置。

return、错误传播、break 和 continue 按实际离开的作用域，由内向外执行清理。最后一次显式读取本身不定义析构时机，因此提前释放只能作为保持可观察行为的优化。

### 聚合责任

一个完整对象的清理顺序如下：

1. 若该对象有用户 Drop，先执行它。
2. 再自动清理仍已初始化的字段。
3. 结构字段按声明逆序，元组按元素位置逆序，数组按下标逆序；enum 只处理当前实际变体。

用户 Drop 可以执行额外动作，但不隐式取消字段清理。它若合法移走字段，后续自动清理跳过该字段。

部分初始化失败只清理已成功建立的责任；由于没有构造完成的整个对象，编译器不调用其完整对象 Drop。即使直接在目标存储中构造，实现也须保持分步构造的可观察行为。

### 清理也要满足访问前提

清理前，编译器检查清理动作实际读取或修改的区域。仅因为一个原始指针目标已失效，不必禁止丢弃该指针表示；然而若用户 Drop 会解引用它，则该清理必须仍有合法目标。

固定清理顺序不能满足依赖时，编译器拒绝该程序。编译器不能为了通过检查而自动交换两个有可观察行为的析构。

### 清理环境与控制流

Drop 所需的实现和环境证据在责任建立时固定，并随移动传递。销毁现场不会重新按名称搜索另一个 allocator 或操作环境；调用视角转换也不改变已经建立的清理责任。

自动 Drop 不新增可恢复错误返回。它可以执行接口允许、且清理位置能够兑现的效应；void 不表示纯性或必然正常完成。

内部资源必须在所需 handler 环境仍有效时清理，再结束 handler 自身责任。挂起清理、取消或丢弃续延，还需能够保持环境并兑现剩余责任的协议。开始执行 Drop 不等于全部责任已经完成。

## 常见示例

### 局部与字段的顺序

~~~flurry
struct Trace {
    id: u32,
}

impl Drop for Trace {
    fn drop(*self) {
        println("drop {}", self.id.*);
    }
}

struct Bundle {
    first: Trace,
    second: Trace,
}

impl Drop for Bundle {
    fn drop(*self) {
        println("bundle");
    }
}

test {
    let outside = Trace { id: 0 }
    let bundle = Bundle {
        first: Trace { id: 1 },
        second: Trace { id: 2 },
    }
}
~~~

输出顺序是 `bundle`、`drop 2`、`drop 1`、`drop 0`。

### Drop 的未来读取也要保活

~~~flurry
struct PrintOnDrop {
    text: str,
}

impl Drop for PrintOnDrop {
    fn drop(*self) {
        println("{}", self.text.*);
    }
}

fn valid_cleanup() {
    let text = "hello".to_string();
    let observer = PrintOnDrop { text: text.as_str() }
}

-- 拒绝：退出时 observer.drop 会读取可能已失效的字符。
fn invalid_cleanup() {
    let text = "hello".to_string();
    let observer = PrintOnDrop { text: text.as_str() }
    text.push_str(" more");
}
~~~

valid_cleanup 先执行 observer 的 Drop，再释放 text。而 invalid_cleanup 若没有保持原字符分配的契约，扩容就会破坏后续清理前提；即使没有显式读取 observer，编译器也不能省略这项检查。

### 部分移出依实际 Drop 判断

~~~flurry
struct LabeledText {
    text: String,
    label: u32,
}

impl Drop for LabeledText {
    fn drop(*self) {
        println("label {}", self.label.*);
    }
}

fn take_text(value: LabeledText) -> String {
    value.text
}
~~~

用户 Drop 只读取仍在的 label，text 的责任交给返回值。编译器不能仅因为 LabeledText 有 Drop 就拒绝这次移出；但若 Drop 改为读取 text，编译器则必须重新检查。

### 失败只清理已构造部分

继续使用 Trace 与 Bundle。

~~~flurry
enum BuildErr {
    Refused,
}

fn make_trace(id: u32, fail: bool) -> !BuildErr Trace {
    if fail {
        error BuildErr.Refused
    } else {
        Trace { id: id }
    }
}

fn make_bundle(fail: bool) -> !BuildErr Bundle {
    let first = make_trace(1, false)!;
    let second = make_trace(2, fail)!;
    Bundle { first: first, second: second }
}

fn attempt_bundle(fail: bool) -> !BuildErr void {
    let bundle = make_bundle(fail)!;
}

test {
    attempt_bundle(true)! {
        .Refused => println("failed"),
    }
}
~~~

错误路径输出 `drop 1` 后输出 `failed`，不会输出 `bundle` 或 `drop 2`。

## 与 Rust / Zig / C / C++ 的关键差异

Rust 的局部变量也按声明逆序清理，但结构字段按声明顺序、数组按下标正序清理；而 Flurry 的聚合清理采用逆序。[Rust 析构规则](https://doc.rust-lang.org/reference/destructors.html)

C++ 析构函数之后继续清理成员，成员采用逆序，是相近的对应物。[C++ 析构规则](https://eel.is/c++draft/class.dtor)

C 依靠显式清理路径；Zig 用 defer/errdefer 安排作用域或错误出口动作。Flurry 的自动清理由实际剩余责任决定。[Zig defer](https://ziglang.org/documentation/master/#defer)

## 运行时实现设计引导

实现为每条退出路径生成有序清理序列。静态确定的初始化状态直接选择动作；条件初始化需要路径选择或初始化标记。标记只在成功交付或责任转移完成后更新。

清理状态至少区分未开始、正在执行和已完成。发生挂起时，运行时保存尚未兑现的责任及所需环境。没有相应控制协议时，实现不能将这条清理放到会挂起或丢弃其剩余动作的位置。

不同退出路径可共享相同清理后缀以减少代码量，前提是共享入口的责任集合、初始化状态、环境和可观察次序一致。

## 编译器实现引导

1. 给绑定、临时值和聚合子位置建立清理范围与责任记录。
2. 在移动、覆盖和部分初始化后更新责任状态。
3. 为每个控制出口列出实际离开的范围，生成逆序清理。
4. 先应用用户 Drop 契约，再处理它执行后仍初始化的字段。
5. 检查每条清理动作的访问、效应和环境条件。
6. 清理完成后检查结果及所有外部保留值的有效性。

清理动作在 normalized IR 上参与 safe 检查，不等到低层代码生成时才作为"实现细节"补入。若内部实现尚未展开，编译器使用已交付的操作契约。

诊断应给出导致未来清理无法成立的操作、实际 Drop 读取及要求的来源，而不只指向作用域右花括号。
