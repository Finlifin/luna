# std 宿主基础交付

首批接口面向 x86_64 Linux。公开 API 见[标准库说明](../library/std/README.md)。

## 格式化变参演示

[host-println.fl](examples/host-println.fl) 使用同一个 comptime 函数计算两种格式串。
下面两次调用分别检查一个和两个字段，第二个字段使用 bool 的 Display。

```flurry
println(status_format(false), answer)!;
println(status_format(true), answer, true)!;
```

在仓库根目录执行：

```sh
python3 -B development/demo_println.py
/tmp/flurry-println
```

脚本通过现有有超时的 Cargo 入口构建 `development/examples/host-println.fl`，
使用真实 sysroot、资源检查和 LLVM O3，关闭开发用的返回值打印，然后运行产物。
第二条命令可以在演示时直接执行，无需再次启动编译器。

构建显式给出 10000000 fuel。目前完整 sysroot 的源码保留和 materialization
会超过默认预算；这不是关闭资源检查，也不表示最终需要这么高的默认预算。

## 实现边界

- Linux 目标 intrinsic 只承载同步 libc 边界，不包含 println、格式串或 File
  的语言特判。签名校验支持不同类型的操作数，返回值统一编码为 i64。
- write/open/unlink 只同步借用输入文本，不保存指针。路径检查嵌入 NUL 与
  长度上限。字节读写使用局部存储；read 的 256 返回值表示 EOF。
- 平台调用不能进入 comptime，不能通过 P4 或 observation 的纯度检查。
  LLVM 调用不携带 readnone/readonly/speculatable 属性。
- Unix 层处理 errno、读写 EINTR 与 EOF；io 层处理短写和零写入。close 不
  重试。File 显式 close 先撤销所有权，再执行系统调用，Drop 不重复关闭。
- builtin.Splice 提供普通标量的阶段间构造协议；不支持任意对象、指针或
  managed storage 的无条件转移。
- 资源检查修复两条通用路径：文本原语的临时操作数包逐项分析；拒绝传播
  读取已选定的 runtime residual 与其契约，不再误读带 comptime 洞的源码体。
  无效或重复的 residual 仍拒绝，不回退到源码模板。

## 当前范围

`{}` 按顺序消费一个实现 Display 的实参，`{{` 和 `}}` 各输出一个花括号。
格式错误、实参数量错误和缺少 Display 的字段都会在编译时被拒绝。
动态 str/String、浮点格式化、allocator、buffered I/O 与其它 Unix 平台仍待实现。
Layout 仅负责布局计算。

## 验证入口

从仓库根目录运行以下定向测试。

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test library_std_format -- --test-threads=3
python3 -B development/check.py --timeout 900 -- test -p luna --test library_std -- --test-threads=1
```

`library_std_format` 检查编译期格式扫描、逐字段的 Display 约束及完整具名参数包。
它还覆盖传入 runtime 格式串、字段数不符、缺少 Display 实现及花括号不配对的拒绝用例。
`library_std` 覆盖原生字节输出、短写、EINTR、文件关闭和布局错误等宿主行为。

11 月 18 日在 dev 用真实 sysroot 重新构建并运行[演示源码](examples/host-println.fl)，
进程退出 0。原生程序的 stdout 如下，pid 随运行变化。

```text
你好，Flurry！
answer = 42
answer = 42, ready = true
Linux pid = 348
i128 minimum = {-170141183460469231731687303715884105728}
done
```
