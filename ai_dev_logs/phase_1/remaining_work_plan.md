# Phase 1 剩余工作规划

> 创建时间: 2026-03-12

## 目标

Phase 1 的终极目标：**编译并运行一个包含函数、结构体、基本泛型的简单 Flurry 程序**。

示例目标程序：
```flurry
fn add(a: i32, b: i32) -> i32 {
    a + b
}

struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 2 };
    let sum = add(p.x, p.y);
}
```

---

## 工作分解 (按执行顺序)

### 里程碑 A: 打通基础管线 — 连接现有模块

#### A1. 实现 `intrinsic::initialize()`
- **位置**: `compiler/intrinsic/src/lib.rs`
- **内容**: 
  - 注册原始类型 (i32, bool, etc.) 到 TyCtxt
  - 建立 LangItems 映射表
  - 消除 `todo!()` panic
- **优先级**: 🔴 阻塞所有后续工作
- **预估复杂度**: 低

#### A2. 在 driver 中接入 Name Resolution
- **位置**: `compiler/luna/src/main.rs`, `compiler/luna/Cargo.toml`
- **内容**:
  - 添加 `resolve` 依赖到 luna crate
  - 在 parse 后调用 `resolve::build_module_tree()` 构建作用域树
  - 将 `Resolver` 集成到 AST lowering 流程，使 lowering 可查询名称
- **优先级**: 🔴 阻塞类型检查
- **预估复杂度**: 中

#### A3. 定义首批 Query Provider
- **位置**: `compiler/interface/src/lib.rs` (或新建 providers 模块)
- **内容**:
  - 创建 `Queries` struct，包含初始 query cache 字段
  - 实现核心 query:
    - `type_of(DefId) -> Ty` — 获取定义的类型
    - `item_sig(DefId) -> FnSig/StructDef` — 获取 item 签名
    - `resolve_path(HirId) -> DefId` — 路径解析
  - 在 `Compiler` 上暴露 query 方法
- **优先级**: 🟡 可与 typeck 并行开发
- **预估复杂度**: 中

---

### 里程碑 B: 类型系统核心

#### B1. HIR 类型标注解析
- **位置**: 新模块或 `compiler/ty/src/`
- **内容**:
  - 将 HIR 类型表达式 (`ExprKind::TyPtr`, `Path` 指向类型等) 转换为 `Ty<'tcx>`
  - 原始类型名称 → `TyKind::Primitive` 映射
  - 用户定义类型路径 → `TyKind::Adt` 映射
  - 泛型参数 → `TyKind::Param` 映射
- **优先级**: 🔴 typeck 的前置
- **预估复杂度**: 中

#### B2. 类型推断引擎
- **位置**: `compiler/ty/src/` (新增 `infer.rs`, `unify.rs`)
- **内容**:
  - Unification table: `InferTy` 变量的 union-find
  - 约束生成: 遍历 HIR expr 收集类型约束
  - 约束求解: 单向统一 (unification)
  - 推断变量实例化: 替换所有 `InferTy` 为具体类型
- **优先级**: 🔴 核心
- **预估复杂度**: 高

#### B3. 类型检查 Pass (typeck)
- **位置**: 新 crate `compiler/typeck/` 或扩展 `compiler/ty/`
- **内容**:
  - 遍历 HIR `Body`:
    - let 绑定: 看标注或推断
    - 字面量: 直接赋予类型
    - 二元/一元运算: 操作数类型一致性检查
    - 函数调用: 参数/返回类型匹配
    - 字段访问: 查询 struct 字段类型
    - if/match: 分支类型一致性
    - 块: 尾表达式类型
  - 将 `Ty<'tcx>` 关联到每个 HIR 节点 (TypeckResults table)
  - 错误报告: 类型不匹配诊断
- **优先级**: 🔴 核心
- **预估复杂度**: 高

#### B4. 泛型参数作用域与替换
- **内容**:
  - 泛型参数在 resolve 中的作用域注册
  - Substitution: 将 `Ty::Param` 替换为具体类型实参
  - 单态化决策（Phase 1 可暂用擦除/boxing）
- **优先级**: 🟡 基础泛型需要
- **预估复杂度**: 中-高

---

### 里程碑 C: MIR

#### C1. MIR 数据结构设计
- **位置**: 新 crate `compiler/mir/`
- **内容**:
  - `Body`: 函数的 MIR 表示
  - `BasicBlock` + `BasicBlockData`: 基本块
  - `Statement` / `StatementKind`: 赋值、存储死值标记等
  - `Terminator` / `TerminatorKind`: goto/return/call/switchInt/drop
  - `Place`: 左值 (local + projections)
  - `Operand`: `Copy(Place)` / `Move(Place)` / `Constant`
  - `Rvalue`: 二元运算、一元运算、聚合构造、引用等
  - `Local` / `LocalDecl`: 局部变量声明
- **优先级**: 🔴 codegen 前置
- **预估复杂度**: 中

#### C2. Typed HIR → MIR Lowering
- **位置**: 新 crate `compiler/mir_build/` 或 `compiler/mir/src/build/`
- **内容**:
  - if/match → 分支基本块
  - 循环 → 循环基本块
  - 表达式求值 → 临时变量 + 赋值语句
  - 函数调用 → `Terminator::Call`
  - 结构体构造 → `Rvalue::Aggregate`
  - 模式匹配 → decision tree → switchInt 链
- **优先级**: 🔴 核心
- **预估复杂度**: 高

#### C3. 基础 MIR Pass
- **内容** (Phase 1 最小集):
  - 常量传播 (可选)
  - 死代码消除 (可选)
  - drop 插入 (初步)
- **优先级**: 🟢 Phase 1 可跳过大部分优化
- **预估复杂度**: 低-中

---

### 里程碑 D: Codegen

#### D1. 后端选择与集成
- **内容**:
  - 推荐选项:
    - **Cranelift**: Rust 原生、集成简单、编译速度快，适合 dev 模式
    - **LLVM (via inkwell)**: 成熟优化、适合 release 模式
    - **两者并存**: 参考 rustc 策略
  - Phase 1 建议: **先用 Cranelift**，门槛最低
- **优先级**: 🔴 必须
- **预估复杂度**: 中

#### D2. MIR → 后端 IR 翻译
- **位置**: 新 crate `compiler/codegen/` 或 `compiler/codegen_cranelift/`
- **内容**:
  - 函数签名翻译
  - 基本块映射
  - 原始类型 → 机器类型映射
  - 算术/比较/逻辑运算翻译
  - 函数调用 ABI
  - 结构体/元组内存布局
  - 栈分配 / 局部变量
- **优先级**: 🔴 核心
- **预估复杂度**: 高

#### D3. 目标文件生成与链接
- **内容**:
  - 生成目标文件 (.o)
  - 调用系统链接器 (cc/ld) 生成可执行文件
  - 入口点 (`main` 函数) 处理
- **优先级**: 🔴 必须
- **预估复杂度**: 中

---

## 执行顺序建议

```
Phase 1 关键路径:

A1 (intrinsic init)
 └─► A2 (resolve 接入)
      └─► B1 (类型标注解析)
           └─► B2 (类型推断) + B3 (typeck)
                └─► C1 (MIR 结构) + C2 (MIR lowering)
                     └─► D1 (后端) + D2 (codegen) + D3 (链接)

可并行:
  A3 (query providers) 可与 B1-B3 同步推进
  C1 (MIR 结构设计) 可在 B 阶段期间预先设计
  D1 (后端选型) 可提前调研
```

---

## 里程碑验收标准

| 里程碑 | 验收标准 |
|--------|----------|
| A 完成 | 编译器启动不 panic；resolve 产出正确作用域树；basic query 可执行 |
| B 完成 | `fn add(a: i32, b: i32) -> i32 { a + b }` 类型检查通过，输出 Typed HIR |
| C 完成 | 上述函数产出正确 MIR (基本块 + return terminator) |
| D 完成 | 上述函数编译为可执行文件并正确运行 |
| Phase 1 | 包含 struct、泛型函数的程序端到端编译运行 |

---

## 风险与依赖

1. **`intrinsic::initialize()` 阻塞一切** — 编译器当前启动即 panic
2. **resolve 与 lowering 的集成方式需要设计** — 当前 lowering 不接收 resolver
3. **泛型单态化策略影响 MIR/codegen 设计** — 需提前决定
4. **Cranelift vs LLVM 选择** — 影响 codegen crate 依赖和 API
5. **HIR 节点类型信息存储方式** — 需要 side table (TypeckResults) 设计
