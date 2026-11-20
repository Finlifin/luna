# 拥有型文本 (`std.string`)

已实现 `std.string.{String, Utf8Err}`（定义于 [`string_impl/string.fl`](string_impl/string.fl)）；借用 `str` 仍属于 `builtin`。

- 基于 `std.collections.Vec<u8>` 继承仿射（`Once`）与结构化 `Drop` 契约，全程维护 RFC 3629 UTF-8 有效性。
- 已提供：`new`、`with_capacity`、`from_str`、`from_utf8`（失败返回携带精确 `valid_up_to` 的 `Utf8Err` 并安全释放字节缓冲）、`from_utf8_lossy`（有效 UTF-8 走零拷贝路径，非法序列替换为 U+FFFD）、`len`、`capacity`、`is_empty`、`byte`、`as_bytes_addr`、`is_char_boundary`、`is_valid_utf8`、`into_bytes`、`reserve`、`shrink_to_fit`、`clear`、`push_str`、`push_string`、`push_char`、`pop_char`、`truncate`、`try_clone`、`equals`、`equals_str`、`cmp_ref`、`starts_with`、`ends_with`、`find`、`contains`、`hash_bytes`、`write_to`。
- 已实现 trait：`Display`、`Eq`、`Ord` 与与 `str` 一致（64 位 FNV-1a）的 `Hash`。原始字节写入格式化使用 `Vec<u8>` 与 `std.fmt.write_fmt`，维护 UTF-8 不变量的拥有型文本格式化使用 `std.fmt.format` 与 `std.fmt.format_into`。
