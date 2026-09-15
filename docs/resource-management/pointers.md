# 指针

## Intro

*T 指向 T 的值。取址得到指针，解引用访问其所指对象；通过指针投影结构字段时，还需要辨认结果的指针关系。

指针类型说明所指对象的类型，不独自证明每次访问都满足对象有效性和程序许可条件。

## 语法

~~~text
pointer_type -> *expr
take_reference -> expr.ref
dereference -> expr.*
projection -> expr.id
~~~

## 正式定义

指针位宽与 usize 相同。指针本体的布局不依赖 pointee 的完整布局；这使指针可以打断按值表示依赖。

通过指针访问结构字段时，结果是字段指针，相当于解引用、字段投影后再取址。接收者适配和类型视角见[隐式转换](../type/receiver-adaptation.md)。

依赖某对象的操作必须能取得该对象并满足其有效性契约。取址、解引用和转换的具体许可由相应安全及资源契约约束，不能仅由位宽、类型相容或某次自动适配推出。

## 常见示例

~~~flurry
struct Cell {
    value: u32,
}

let cell = Cell { value: 7 }
let ptr: *Cell = cell.ref;
let field: *u32 = ptr.value;
~~~

field 指向字段。需要字段内容时应按解引用操作的契约处理，不能把这个结果直接当成 u32。

## 与 Rust / Zig / C / C++ 的关键差异

C/C++ 指针是直接的表示类比；Rust 还区分引用与原始指针，不能把它的自动借用规则搬来。Zig 同样有指针类型，但具体访问及表示承诺须分别核对。[Rust 指针类型](https://doc.rust-lang.org/reference/types/pointer.html)

## 运行时实现设计引导

指针保存目标规定的地址表示。字段指针由合法基址和字段布局得到。布局优化必须保留所指对象关系；不能以“指针大小相同”替代不同目标类型的访问契约。

## 编译器实现引导

分别记录指针本体布局、pointee 类型与访问前提。字段投影产生新的字段指针类型，不误插入读值。

布局分析遇到指针停止展开 pointee 的按值表示依赖；访问分析仍须处理 pointee 的实际字段信息与对象有效性。
