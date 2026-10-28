# 基础文本

bytes.fl 实现 str/Symbol 的字节观察接口；公共入口仍是 builtin.text。str 属于 builtin，拥有型 String 属于 std/string。编码策略和 I/O 适配不放在这里。
