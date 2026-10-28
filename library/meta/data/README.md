# 编译期数据

list.fl 将列表操作 impl 到 meta.List；object.fl 提供有序 Object 查询与编辑；integer.fl 提供编译期整数操作。List 的编译器绑定 normal-form 头仍在包根，避免更改公开 meta.List 身份。
