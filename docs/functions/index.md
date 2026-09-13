# 函数

函数是 Flurry 里封装可复用逻辑的基本单元：接收参数、执行计算，并可能返回结果。签名可以同时表达普通参数、具名默认值、变参、编译期参数，以及用子句写出的泛型约束。

## 你会学到

- 如何定义、调用函数与方法
- 各类参数：位置、具名可选、变参、拓展变参、隐式、`comptime`
- 用子句参数表达泛型
- 返回值、具名返回值与提前返回

## 快速上手

用 `fn` 定义函数。函数体末尾的表达式会隐式作为返回值；无返回值时可省略 `-> void`：

```flurry
fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn print_greeting(name: String) {
    println("Hello, {}!", name);
}

let sum = add(5, 3);
print_greeting("Flurry".to_string());
```

---

## 方法

写在类型（`struct`、`enum`、`mod` 等）内部，且第一个参数是 `*self`（指针）或 `self`（取得所有权）时，称为**方法**。没有 `self` 的函数仍可通过类型名调用：

```flurry
struct Point {
    x: f64,
    y: f64,

    pub fn origin() -> Self {
        Point { x: 0.0, y: 0.0 }
    }

    pub fn distance(*self) -> f64 {
        math.sqrt(self.x * self.x + self.y * self.y)
    }

    pub fn translate(*self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
    }
}

let p = Point { x: 3.0, y: 4.0 }
let d = p.distance();
let origin = Point.origin();
```

更常见的是在单独的 `impl` 块里为类型补充方法：

```flurry
impl Counter {
    fn new() -> Self { Counter { value: 0 } }
    fn increment(*self) { self.value += 1; }
    fn get(*self) -> i32 { self.value }
}
```

`p.distance()` 大致是 `Point.distance(p.ref)` 的语法糖。

---

## 参数

### 位置参数

按定义顺序传递，是最基本的形式：

```flurry
fn process(data: Slice<u8>, len: usize) { ... }

process(buffer[..], buffer.len());
```

### 具名可选参数

以 `.` 开头，**必须**提供默认值。调用时按名字传参，顺序任意，可省略以使用默认值：

```flurry
fn configure(
    .retries: usize = 3,
    .use_tls: bool = true,
    .log_level: LogLevel = LogLevel.info,
) { ... }

configure(.log_level = LogLevel.debug);
configure(.use_tls = false, .retries = 5);
configure();   -- 全部使用默认值
```

### 变长参数

以 `...` 前缀声明。调用时多余的实参会被收集进某个**序列集**；序列集的具体类型由你在签名里选定。

运行时函数里，序列集常常是切片：

```flurry
fn sum_all(...numbers: Slice<i32>) -> i32 {
    numbers.iter().sum()
}

sum_all(1, 2, 3, 4);
```

`comptime` 参数与变参结合，可以得到类型安全的可变参数（如 `println`）。此时序列集由编译期函数根据格式串构造为元组：

```flurry
fn println(comptime format: str, ...varargs: fmt.Parse(format)) { ... }
```

也可以显式约束序列集本身的类型：

```flurry
-- 完全匿名，序列集视作元组
fn show(...args: T) where T, requires Type.predicates.is_tuple(T) {
    -- 按元素展开元组
    inline for arg in args {
        println("{}", arg);
    }
}

-- 这里序列集为一个可装任意类型元素的comptime list（comptime类型系统为渐进依赖类型）
comptime fn make_list(...elements: List<Any>) -> List<Any> {
    elements
}

let _ = make_list(1, "hello", true);
```

### 拓展变参

一次签名里可以声明多个 `...` 参数，分别收集**序列集**与**属性集**。这是设计嵌套式 API 的利器：

```flurry
-- 属性集收集进匿名结构体（实例化 S 时自动生成）
fn div(...children: T, ...properties: S) -> HtmlElement {
    ...
}

let elem = div {
    "Hello, world!",

    section {
        h3 { "Hello, world!" },
        p { "This is a paragraph." },
    },

    class: "container",
    style: {
        background: "red",
        color: "white",
    },
    layout: {
        width: "100%",
        height: "100%",
    },
}
```

位置上的子节点进入 `children`；`name: value` 形式的字段进入 `properties`。

### 隐式参数

由编译器在调用点自动提供，调用者无需显式传递：

```flurry
fn log(msg: String, implicit __src__: builtin.SourceLocation) {
    println("{}:{}: {}", __src__.file, __src__.line, msg);
}

log("done");   -- 自动传入调用点的源码位置
```

### `comptime` 参数

```flurry
fn create_array(comptime T: Type, comptime N: Integer, init: T) -> Array<T, N> {
    ...
}

fn alloc(comptime T: Type, size: usize) -> &fresh Slice<T> {
    let byte_size = size * Type.size_of(T);
    ...
}
```

---

## 子句参数

函数支持**子句参数**。最常见的用法是声明类型变量，对应其他语言里的泛型：

```flurry
fn map(f: fn(T) -> U, list: Slice<T>) -> Vec<U>
where T, U
{
    list.iter().map(f).collect()
}
```

需要约束时，可在 `where` 中继续写 `requires`。调用时类型参数通常由编译器推断。

---

## 返回值

用 `-> ReturnType` 标注返回类型。末尾表达式隐式返回；也可用 `return` 从任意位置提前返回：

```flurry
fn double(x: i32) -> i32 {
    x * 2
}

fn find(s: String, c: char) -> ?usize {
    for (i, ch) in s.chars().enumerate() {
        return i if ch == c;
    }
    null
}
```

可以为返回值绑定名称，便于在规范（specification）里引用：

```flurry
fn abs(x: i32) -> result: i32
where ensures result >= 0
{
    return -x while x < 0;
    x
}
```

`return … while …` 是带守卫的提前返回语法糖。
