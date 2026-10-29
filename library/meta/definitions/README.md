# 受检定义构造

structs.fl、enums.fl 包含已接通的 builder；结构反射接口及 Type 固有方法位于 ../reflection/structure.fl。草稿到 Type 的发布必须经过编译器检查；尚未交付的 fn/trait/impl builder 不用占位实现伪装。builder 复用 Type 的受检反射方法。
