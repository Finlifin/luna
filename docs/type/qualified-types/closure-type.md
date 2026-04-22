闭包类型是动态依赖类型与lambda类型的基础。

闭包类型会从环境中捕获变量，这些变量只在必要时被传递，而不会影响主类型的内存布局。

```flurry
struct Foo {
    a: usize,
    elems: Vec<^{ a: usize = self.a } usize>,
}
```
在这个例子中，`elems`字段是一个包含闭包类型的向量。每个闭包类型中，都有一个identifier `a`，它告诉编译器这个缺失值如何取得。
而elems每个元素依然只占用usize的内存空间，因为闭包类型中的`a`的使用是封闭的，而且更进一步说，这里根本没有用到`a`。

```flurry
struct Set where T {
    elems: Vec<T>,

    pub fn insert(*self, elem: T) -> Ref<T, self.ref> 
    where outcomes write<self.elems> {
        self.elems.push(elem);
        Ref<T, self.ref> {
            index: self.elems.len() - 1,
        }
    }
}

struct Ref where T, quote set: *Set<T> {
    index: usize,
}

-- 编译器内部会对具体的deref调用传递set参数
-- 无法从量化了动态依赖参数的implementation中构造trait object
impl Deref for Ref<T, set> where T, quote set: *Set<T> {
    assoc Target: Type = T;

    fn deref(*self) -> *T {
        set.elems[self.index].ref
    }
}
```
对于这样的动态依赖类型来说，Ref<T, set>会被编译器视为^{ set: *Set<T> } T的闭包类型。
```flurry
struct Foo1 where T {
    set: *Set<T>,
    -- 不用储存
    rs: Vec<Ref<T, self.set>>,
}

struct Foo2 where T, quote set: *Set<T> {
    -- Foo2再次变成一个动态依赖类型
    rs: Vec<Ref<T, set>>,
}
```

```flurry
fn map(monad: ?T, lambda f: F) -> ?O
where T, O, F:+ fn(T) -> O {
    monad match {
        null => null,
        some? = f(some)
    }
}

test {
    let vec = Vec.new();

    -- map<i32, usize, ^{ vec: *Vec<i32> = vec.ref } usize>
    let data = map(1) do |x| {
        vec.push(x);
        vec.len()
    }
}
```