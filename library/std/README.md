# std

`std.prelude` 显式导入。第一批宿主实现面向 x86_64 Linux，通过目标 intrinsic
进入 LLVM/libc；它们不能在 comptime 或 reference interpreter 中执行。

| 入口 | 当前内容 |
| --- | --- |
| `std.collections` | `Vec<T, .A, .allocator>`、`VecDeque<T, .A, .allocator>`、`HashMap<K, V, .A, .allocator>`、`HashSet<T, .A, .allocator>` 与消费迭代器（运行时依赖 allocator、零存储开销、ZST 支持、`Once`/`Drop`、`Clone`/`Eq`/`IntoIterator`/`Iterator`、`ByteWriter`/`Write`） |
| `std.string` | `String<.A, .allocator>`、`Utf8Err`（RFC 3629 UTF-8 校验与 `from_utf8_lossy`、`Drop`、`Display`/跨 allocator `equals`/`cmp_ref`/`push_string`） |
| `std.io` | `Reader`、`Write`、`ByteWriter`、`Stdin`（`stdin`）、`Cursor`、`BufReader`、`BufWriter`、`Writer`（stdout/stderr）、完整文本与字节缓冲读写（`write_all`、`write_all_raw`、`write_bytes`、`write_string`、`read_into`/`read_exact`/`read_to_end`/`read_to_string`/`read_until`/`read_line`）、`read_byte`、`IoErr` |
| `std.fmt` | 泛型 allocator 化的 `format<.A, .allocator>`、`format_into`、`write_fmt<W :- ByteWriter>`、`print`/`println`、`eprint`/`eprintln`、`Display`（含 `bool`/整数/`char`/`str`/`String`） |
| `std.fs` | 仿射 `File`、打开/创建、读写（`Reader`/`Write`/`ByteWriter` 与固有 `read_byte`/`write_bytes`/`write_string`）、显式关闭、`Drop`、删除文件 |
| `std.mem` | `AllocErr`/`ReallocErr`、`RawBlock`、`Allocator`、`SystemAllocator`（`GLOBAL`）、`DefaultAllocator`、`FixedBufferAllocator`、`ArenaAllocator`、`Box<T, .A, .allocator>` 与 `Layout` |
| `std.os.linux` | 原始系统调用包装，负 errno 结果 |
| `std.os.unix` | Linux 上的错误转换、EINTR 重试、EOF 转换与原始缓冲区读写 |

```flurry
use std.prelude.*;

fn greet() -> !std.io.IoErr void {
    println("你好，Flurry！")!;
    let answer: u32 = 42;
    println("answer = {}, ready = {}", answer, true)
}
```

公开入口为 `fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format)) -> !std.io.IoErr void`、`fn format(comptime format: str, ...varargs: defer std.fmt.Parse(format)) -> !std.mem.AllocErr String<.A = A, .allocator = a>`（可选子句 `.A: Type = DefaultAllocator, quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref`）、`fn format_into(target: *String<.A = A, .allocator = a>, comptime format: str, ...varargs: defer std.fmt.Parse(format)) -> !std.mem.AllocErr void` 与泛型 `fn write_fmt(output: *W, comptime format: str, ...varargs: defer std.fmt.Parse(format)) -> !W.as(ByteWriter).Err void where W :- ByteWriter`。
`{}` 顺序消费一个实现 Display 的值，`{{` 和 `}}` 分别输出单个花括号。
参数数量、类型和格式在编译期检查。Display 来自 builtin.fmt，接收 `*W`，其中
`W :- builtin.fmt.ByteWriter`，返回 `!W.as(ByteWriter).Err void`。
std.io.Writer、File 与 Vec<u8, .A = A, .allocator = a> 都实现 ByteWriter；`value.write(file.ref)` 借用写入目标，
不转移关闭或释放责任。分配型容器通过运行时依赖的 `quote .allocator: lift *A` 区分身份而不增加本体存储开销。泛型 writer 无需 Clone/Copy，也不必使用 std.io.IoErr。
浮点格式化尚未实现。

`std.fmt.FormatPlan.parse` 已提供 comptime 格式扫描，支持 `{}`、`{{`、`}}`，
保留原文 UTF-8 字节区间并报告错误偏移。`std.fmt.Parse(format)` 使用扫描结果构造逐项带 Display 约束的变参类型描述，
实例按字段各自的类型展开输出。变参也支持完整的具名元组供应。详见
[`fmt_impl/README.md`](fmt_impl/README.md)。

静态文本字面量仍为 `str`；`std.fs.File.open`/`create`/`write_all` 的路径与字面量参数使用 comptime `str` 子句，而 `Writer`/`File` 的 `write_bytes`/`write_string`、`std.os.unix.write_raw`/`read_raw` 与 `std.string.String` 已支持运行时字节序列与拥有型文本。其它 Unix 平台、`process`、`time` 与 `sync` 仍未实现。`Layout` 不执行分配。

公开模块在 `lib.fl` 组织；实现分别放在 `io_impl`、`fmt_impl`、`fs_impl`、
`mem_impl` 与 `platform`。这避免把包内实现模块误当成公开路径，也避免内联
模块与同名自动目录模块冲突。

演示入口：在仓库根目录执行 `python3 -B development/demo_println.py`。
此入口编译并运行 Linux 原生的异构变参演示。源码见
[`host-println.fl`](../../development/examples/host-println.fl)。
