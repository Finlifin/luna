# 平台边界

Linux 原始操作由 language-items.conf 按声明身份绑定，输出 i64：非负为成功，
负值为 -errno；read_byte 的 256 表示 EOF。Unix 适配层转换成语言的错误限定
类型与可选类型。它目前只选择 Linux，没有声称支持 macOS/BSD。

输入文本只在调用期间借用，不保留地址。Linux 路径检查内嵌 NUL 与 4096 字节
上限，在栈上补终止字节；不会截断成另一个路径。write 保留部分写入结果。
平台调用带外部副作用，不能成为 pure 函数、观察谓词或 comptime invocation。

平台行为依据 Linux man-pages：[write](https://man7.org/linux/man-pages/man2/write.2.html)、
[open](https://man7.org/linux/man-pages/man2/open.2.html)、
[close](https://man7.org/linux/man-pages/man2/close.2.html)。close 不重试；自动析构忽略
关闭错误，显式 close 负责把错误交给调用者。
