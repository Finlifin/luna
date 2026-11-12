# std

`std.prelude` 显式导入。第一批宿主实现面向 x86_64 Linux，通过目标 intrinsic
进入 LLVM/libc；它们不能在 comptime 或 reference interpreter 中执行。

| 入口 | 当前内容 |
| --- | --- |
| `std.io` | `Writer`、stdout/stderr、完整文本写入、字节读写、`Error` |
| `std.fmt` | `print`/`println`、stderr 对应操作、`Display` 与标量输出 |
| `std.fs` | 仿射 `File`、打开/创建、读写、显式关闭、Drop、删除文件 |
| `std.mem` | `Layout` 的对齐检查、大小与重复布局计算 |
| `std.os.linux` | 原始文件描述符操作，负 errno 结果 |
| `std.os.unix` | Linux 上的错误转换、EINTR 重试、EOF 转换 |

```flurry
use std.prelude.*;

fn greet() -> !std.io.Error void {
    println("你好，Flurry！")!;
    print("answer = ")!;
    let answer: u32 = 42;
    println_value(answer)
}
```

`println` 当前接收一段 comptime 文本；花括号原样输出，没有 `{}` 参数替换。
`print_value`/`println_value` 用普通 `Display` trait 输出运行时 bool 和机器整数，
用户也可以实现该 trait。浮点格式化、任意长度异构参数展开尚未交付。

路径与文本暂用 comptime 输入；底层显式子句例如
`std.os.linux.open<"data.txt">(0, 0)`、`stdout().write_all<"text">()`。
这是现有 runtime str 内存来源接线尚未完成时的接口范围；没有关闭资源检查，
也没有把动态字符串当作静态存储。String、allocator、arena、容器、缓冲 I/O
和其它 Unix 平台仍未实现。`Layout` 不执行分配。

公开模块在 `lib.fl` 组织；实现分别放在 `io_impl`、`fmt_impl`、`fs_impl`、
`mem_impl` 与 `platform`。这避免把包内实现模块误当成公开路径，也避免内联
模块与同名自动目录模块冲突。

演示入口：在仓库根目录执行 `python3 -B development/demo_println.py`。
构建后可直接运行 `/tmp/flurry-println`。源码见
[`host-println.fl`](../../development/examples/host-println.fl)。
