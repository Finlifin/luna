# 022 — comptime — 01

- 日期：2026-08-22
- global_index：022
- theme：comptime
- subindex：01
- 本轮目标：切换到编译时计算主题，盘点现有 proposal，并先确认语义地基

## 作者本轮输入摘要

作者要求：模式匹配暂缓，当前细节留到具体实现时再敲定；从基础内容进入编译时计算主题。后续设计多为 proposal，可能不健全或不完备，因此应提出更深入的问题，与作者共同讨论。

## 当前材料盘点

- 模式匹配本轮冻结，不继续追位向量主题遗留的 4 个问题。
- `docs/comptime/` 中目前有实质内容的文件主要是 `attribute.md` 与 `inlining-comptime-code.md`；`index.md`、`normalization-and-instantiation.md`、`purity-and-normal-form.md`、`quoting-and-splicing.md`、`the-comptime-phases-of-traits.md` 仍为空占位文件。
- `docs/comptime/prompt.fl` 是设计备忘，提出了 U₀/U₁、normalization/instantiation、pure comptime fn、可选子句参数、defer、泛型即编译期函数、trait 的 comptime/runtime 两面性、作用域和 evaluator 等大量方向，但这些不能直接视为定稿语义。
- `docs/type/boostrap.md` 与 `docs/compilation/phases.md` 已使用两级类型论和两阶段流水线叙述，但本轮不把其中 proposal 自动升格为已确认结论。

## 可行性评估

总体方向有价值：把编译期计算、泛型、类型级计算和代码生成放在同一框架中，并区分定义端检查与调用端特化，能够解释许多系统语言难以同时满足的需求。

当前最大风险不是某个语法细节，而是几个不同维度被同一组词混用：

1. U₀/U₁ 是语义宇宙，normalization/instantiation 是编译阶段，二者不应互相替代。
2. normalization 目前同时包含类型检查、约束收集、符号执行、SMT 推理和部分求值；若不划边界，定义错误与调用错误无法稳定分类。
3. quote、lift、splice 同时可能指代码、类型和值的跨宇宙传递；三者的对象和合法性尚未分开。
4. pure comptime fn、编译器环境读取、调用点 source span 和缓存键之间存在直接张力。
5. “泛型是返回函数的编译期函数”是很强的统一解释，但它必须回答开放项如何检查、where 参数如何形成真实参数、以及依赖结果类型如何表示。

## 需要你拍板的问题

### A. 语义层与阶段

1. U₀/U₁ 与 normalization/instantiation 的关系是什么？
   - A. U₀/U₁ 是语义宇宙；normalization/instantiation 是处理这些宇宙的编译阶段（推荐候选）。这样可以允许多个 pass 服务同一宇宙。
   - B. U₀/U₁ 本身就是两个执行阶段，normalization/instantiation 只是 U₁ 内部子阶段。术语简单，但容易把运行时语义和编译流水线绑死。
   - C. 放弃宇宙术语，只保留“编译期/运行时阶段”。实现和入门较简单，但 quote/lift/splice 的类型论边界会变模糊。
   - Other: …

2. normalization 的最低语义承诺是什么？
   - A. 必须完成开放定义的名称、类型、约束和分支局部环境检查；只有不依赖未知参数的纯 U₁ 表达式可以顺便求值。
   - B. 允许符号执行和 SMT 证明，并把可证明的事实传给后续检查；solver 返回 unknown 时形成明确的延迟义务。
   - C. normalization 只做 elaboration/type checking，不承担证明或求值；所有具体计算推迟到 instantiation。
   - Other: …

3. 什么情况必须在定义端报错，什么情况可以留到调用端？
   - A. 开放项中无法由签名约束支持的操作是定义错误；只依赖具体参数的约束失败是实例化错误。
   - B. 任何无法在定义端证明安全的操作都报定义错误，追求最强静态保证，但会显著限制泛型表达力。
   - C. 只要失败原因依赖参数，就允许延迟到 instantiation；这最灵活，但需要严格标记延迟区域以免错误位置漂移。
   - Other: …

### B. 纯性、环境与缓存

4. `pure comptime fn` 的纯性到底约束什么？
   - A. 结果只依赖显式 U₁ 参数；读取环境、source span、时间、I/O 等必须通过显式 effect/capability 参数进入。
   - B. 允许读取编译器快照；快照自动成为隐式输入，仍可缓存，但缓存键必须包含该快照。
   - C. 允许任意编译期副作用；`pure` 只是优化/缓存承诺，违反时由实现或用户承担后果。
   - Other: …

5. instantiation 的语义缓存键由哪些内容决定？
   - A. 定义身份 + 显式类型/值/子句参数；调用点元数据另行传递，不改变函数特化结果。
   - B. 还包含 source span、build 配置或调用者等隐式上下文；表达力强，但相同参数可能产生多个特化。
   - C. 语言不承诺缓存等价性，缓存只是实现优化；最自由，但 evaluator 的确定性要求较弱。
   - Other: …

6. `where` 中的可选子句参数默认值是否允许不纯计算？
   - A. 允许；默认值在调用端求值，并作为普通显式参数进入函数体，正是隔离环境依赖的机制。
   - B. 不允许；所有默认值必须纯，环境信息只能通过显式调用参数传入。
   - C. 允许读取受限的编译器 capability，但不能做任意 I/O 或网络访问。
   - Other: …

### C. 跨宇宙对象

7. `lift` 的对象是什么？
   - A. 只把 U₀ 类型提升为 U₁ 的 `Type`；普通运行时值不能凭空被读取。
   - B. 编译器可见的常量值也能提升到 U₁，但必须满足可观察性/表示约束。
   - C. 不设一般 lift；需要类型信息用 `Type`，需要代码用 quote，值只通过 comptime 参数传递。
   - Other: …

8. `splice` 是否区分“值拼接”和“代码拼接”？
   - A. 区分：U₁ 值只能 splice 为可表示的 U₀ 常量，代码生成必须经 quote/code value 再 splice。
   - B. 统一为一个操作，由结果类型决定是值还是代码；语法统一但类型规则更复杂。
   - C. U₁ 结果一律先表示成代码片段，值拼接只是代码拼接的特例。
   - Other: …

9. 哪些 U₁ 值可以进入 U₀？
   - A. 只有具有明确 U₀ 表示、布局和初始化规则的值；不能表示的值是静态错误。
   - B. 任意可由 evaluator 序列化的值都可进入 U₀，布局由实现决定。
   - C. 只有字面量和类型系统列出的少数内建值可 splice，用户类型需显式声明可 splice 能力。
   - Other: …

10. quote 的语义是什么？
   - A. quote 产生带卫生规则的代码值；自由变量捕获必须显式声明。
   - B. quote 只是 AST/语法数据，不自动捕获词法环境；后续解析和绑定由用户或库完成。
   - C. quote 产生带类型和环境的 code value，能够保留捕获并在 splice 时检查。
   - Other: …

### D. 参数、控制流与依赖结果

11. `comptime` 参数与普通参数的边界是什么？
   - A. `comptime` 参数是 U₁ 输入，在运行时调用约定中被擦除；调用点必须提供或推导其值。
   - B. 它仍是普通参数，只是要求调用前可求值，因此可以保留运行时表示。
   - C. 不设独立参数标记，能否进入 U₁ 由参数类型或上下文决定。
   - Other: …

12. `inline if/when/for` 在未知条件下如何检查？
   - A. normalization 检查所有分支并传播路径条件；instantiation 再选择并展开分支。
   - B. 条件无法在 normalization 求值时延迟整个控制流，未选分支不检查。
   - C. `inline` 只表示代码生成倾向，语义等同运行时控制流。
   - Other: …

13. `defer` 是否保留为独立的延迟类型计算机制？
   - A. 保留，只允许在依赖 comptime 参数的签名/局部类型位置，并明确延迟检查的边界。
   - B. 删除该概念，改用显式依赖类型或 refinement/gradual dependent type 表达。
   - C. 只允许库级类型函数在 instantiation 求值，普通函数签名不得依赖尚未确定的值。
   - Other: …

14. 依赖结果类型的表达力先取哪条路线？
   - A. U₁ 函数可以返回依赖具体值的类型，instantiation 时必须得到具体类型。
   - B. 先用 refinement/gradual dependent type 的 `Any` 表示未知结构，逐步精化。
   - C. 结果类型必须在定义端具体化；类似格式字符串解析的需求改为运行时描述符或固定泛型结构。
   - Other: …

### E. 约束、资源与 trait

15. `requires` / SMT / 逻辑命题的求解失败如何处理？
   - A. 只允许可判定片段；无法决定就是编译错误。
   - B. 允许 solver 返回 unknown，并把义务延迟到 instantiation 或要求用户补充证明/约束。
   - C. 命题本质是纯 comptime 布尔表达式，不承诺独立 theorem solver 语义。
   - Other: …

16. 纯 comptime 递归和资源耗尽的语言语义是什么？
   - A. 要求静态可终止，无法证明终止即拒绝。
   - B. 允许一般递归，但有实现定义或标准规定的 fuel/时间/内存上限，耗尽给确定的编译错误。
   - C. 不规定上限或失败分类；工具链可挂起、终止或自行恢复。
   - Other: …

17. trait 的 U₀/U₁ 两面性采用哪种模型？
   - A. 每个方法显式声明所属宇宙；同一 trait 可分别实现两面，调用按当前宇宙选择。
   - B. 一个方法可自动在两个宇宙间 lift/splice，只有无法转换时才报错。
   - C. U₀ trait 与 U₁ trait 是两套独立 trait 系统，通过显式适配器互通。
   - Other: …

## 已确认 / 仍待定

### 本轮已确认

- 模式匹配主题暂缓，不继续追位向量细节。
- `comptime` 是下一主题。
- 后续设计按 proposal 对待；本轮只讨论，不把现有备忘自动升格为正式规范。

### 现有材料中的方向性主张（未在本轮确认）

- U₀/U₁、quote/lift/splice。
- normalization/instantiation 两阶段模型。
- `inline` 控制流、`comptime {}`、pure comptime fn。
- 可选子句参数、`defer`、泛型即编译期函数、trait 的 comptime/runtime 两面性。

### 主要待定

- 上述 17 个问题；优先级最高的是 1–10，因为它们决定其余机制的语义边界。

## 文档写入

- 否：作者尚未授权写入正式 `docs/`；本轮只讨论并建立问题清单。
- 本轮记录写入 `docs_chat_history/022-comptime-01.md`。

## 作者答复（prompt）

1 宇宙沿用自2ltt原理论，今后统一使用comptime/runtime即可
2 根据我的观察，实际上我们不能对所有flurry code都统一使用一个级别的normalization检查强度。
  在保证def-site不可检查性不会扩散、开放分散定义（如impl/derive/extend/template）保持最终coherence的前提下，
  我已经指认出数种normalization强度，并且不确定是否已经完全够用：
    ``` flurry
    {- normal -}
    -- 这里是我们的主战场，normalization检查强度最高
    fn foo(name: T) where T, requires T:- Display or T:- AsStr {
        println("Hello, {}!", name);
        --> println: fn(comptime format: str, ...args: defer std.fmt.Parse(format))
        --| e0: str = str_literal("Hello, {}!")
        --| runtime e1: T = param(name)
        --| e2: Type = defer std.fmt.Parse(e0) ⇓ Tuple<refines Any { Itself:- Display }>
        --| runtime e3: e2 = tuple(e1)
        --| runtime e4: void = apply(println, e0, e3)
        --| solve(clause ∧ path_condition ∧ model)
    }

    {- explicit defered -}
    -- std.fmt.Parse: pure comptime fn Parse(comptime format: str) -> R: Type where ensures Type.predicates.is_tuple(R)
    fn println(comptime format: str, ...args: defer std.fmt.Parse(format), .writer...) {
        ...
        -- for e'tuple_enumerate, requires Type.predicates.is_tuple(e'type)
        -- and only available for inline for loops in runtime context
        -- 所以到这里实际上args在编译器眼里是args: refines Any { Type.predicates.is_tuple(args'type) }
        inline for (arg, i) in args'tuple_enumerate {
            const T = arg'type;
            inline when {
                T:- Display => writer.write(arg.to_string().as_bytes()).unwrap();,
                ...
                else => inline if T is do {
                    Array<E, len> => {
                        writer.write("[").unwrap();
                        for (i, elem) in arg.iter().enumerate() {
                            const spliter = if i == 0 { "" } else { ", " };
                            println("{}{}", elem, spliter, .writer = writer);
                        }
                        writer.write("]").unwrap();
                    },
                    ...
                }
            }
        }
    }

    {- loose, i.e. gradual dependent type -}
    -- 由于comptime flurry采用的是渐进依赖类型系统，受限于理论的局限性，comptime context的类型约束只有在instantiation时才会被检查，
    -- 因此flurry实际上获得了更加强大的表达力
    -- 并且在单个comptime context中，implementation/extension/derivation都是线性可见的
    pure comptime fn StructOfArray(...args: meta.List<Type>, ...config: meta.Map<Symbol, Any>) -> Type {
        let DataType = args.get(0, Type).expect("Must provide a data type as the first argument");
        let IndexType = args.get(1, Type).expect("Must provide an index type as the second argument");
        -- 在这里处理DataType并取得其字段信息
        let iir_layout = {
            ...
        }

        -- 处理并检查config
        let config = {
            let Allocator = config.get(.allocator, Type).unwrap_or(std.mem.DefaultAllocator);
            let growing_strategy = config.get(.growing_strategy, GrowingStrategy).unwrap_or(GrowingStrategy.default());
            ...
        }

        var struct_builder = StructBuilder.new();
        -- 根据每个字段生成一个数组
        for field in iir_layout.fields {
            ...
            struct_builder.add_field("_${field.name}_data", Buffer<field.type>, .default = Buffer<field.type>.default());
        }

        let Result = struct_builder.build();

        impl Result {
            -- unbounded get, directly panics if index is out of bounds
            pub unsafe fn get(*self,
            comptime field: Symbol, 
            index: IndexType,
            ^attrs.only_if(build.mode == .debug) __src__: meta.compilation.Src,
            ) -> FieldType<DataType, field> {
                let <config.index_extraction[0]> = index else {
                    process.panic("Index extraction failed");
                }

                let buf: Buffer<FieldType<DataType, field>> = self'get_struct_field("_${field.name}_data");
                if <config.index_extraction[1]> >= buf.len() {
                    inline if build.mode == .debug {
                        process.panic("Index out of bounds, called from ${__src__}");
                    } else {
                        process.panic("Index out of bounds");
                    }
                }

                buf.get(<config.index_extraction[1]>)
            }
            ...
        }
        
        Result
    }

    {- staging by contextual modal type -}
    -- 这里最为复杂，因为上下文模态类型实际上不是跟其它系统正交的，并且还直接影响到命名解析，我们需要找到一个合适的自动化机制来
    -- 保证可用性、健壮性
    -- 这有些复杂，后面会单开一个主题
    ```

3 见上，我们将在不同normalization强度下分别展开讨论

4 C，编译器视其为公理而不是待证引理
5 A，并且所有参与query缓存的编译时值，其类型必须实现Eq的comptime相
6 B
7 lift几乎只有一种使用场景，那就是将quote起来的runtime代码/值/对象类型化。A
8 flurry中存在两种staged compilation，一种是comptime/runtime，一种是基于contextual modal type的，前者会将comptime值splice到runtime常量，
  而后者则是splice到runtime代码。但是，quoted code实际上也是编译时的值，因此在底层讨论splice时，会将comptime值分为两类
9 A，比如对与结构体来说，如果它的成员都可以splice到runtime, 那么它也可以splice到runtime。也许我们可以设计一个auto trait?
10 quote有两种意思，一指将runtime符号保障为comptime值，即2ltt中的quote,
   使得显示运行时依赖类型、内存副作用（需要表达read<some_location>这样的内存副作用）这种内容可以被融入到现有系统中；
   另一指基于CMTT的多阶段compilation中的quote, 这是"AST/语法数据"的quote, 其comptime类型是一个contextual modal type, 因此是卫生的
11 A
12 A
13 A，在很多场景中，defer可以显著地阻断不良定义的传播，让lsp或着编译器不至于束手无策（比如遇到println调用时， 它们知道要就地执行有关comptime代码）。
    defer还有哪些适用场景尚待推敲
14 你这个问题表达得不太好
15 在最高normalization强度下，A。值得注意的是，requires/ensures中的comptime谓词，要求必须是良定义的命题，
   而inline if/inline when中的谓词，可以是任意comptime bool表达式, normalization时编译器尝试自己从中提取良定义的命题
16 B
17 显然，comptime的函数定义时必须通过comptime关键字修饰，因此是很容易区分两者的，
   就像很多类型也有comptime/runtime两相一样，所以它们可以共享很多基础，然而两相含义是不一样的

实际上我准备了两种需要并存的comptime执行模型，一种是在runtime fn definition这样的场景，
其中存在着零星的comptime执行，我称为comptime interpretation；
这样的情况下会就地解释执行有关ir, 而对于独立定义的comptime fn, 
其会被编译为comptime engine的字节码，在调用该函数时便由engine执行它的字节码，
我称为comptime execution；
这是合理的。
