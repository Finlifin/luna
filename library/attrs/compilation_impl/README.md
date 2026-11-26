# 编译策略属性

- `attrs.compilation.intrinsic_enum`：产生 `{ __flurry_intrinsic_enum: true }`，标记由编译器提供表示与行为的内置枚举（仅允许附着在 `enum` 声明上，由 elaboration 的 `__flurry_intrinsic_enum` 属性消费者消费，不再在 AST lowering 中按未解析名称特判）。
