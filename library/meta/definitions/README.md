# 受检定义构造

structs.fl、enums.fl 包含已接通的反射辅助函数和 builder。草稿到 Type 的发布必须经过编译器检查；尚未交付的 fn/trait/impl builder 不用占位实现伪装。更多反射接口稳定后归 meta/reflection，builder 复用它们。
