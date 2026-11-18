# std 宿主基础交付

2026-11-12。范围是 x86_64 Linux 上的首批可运行接口，不是完整标准库。
公开 API 与限制见 `library/std/README.md`。

作者要求的 println 是
`fn println(comptime format: str, ...varargs: defer std.fmt.Parse(format))`。
本记录中的文本输出 demo 只验证宿主底座，不满足该格式化变参要求。完整交付
仍待类型描述计算和变参实例化接通；当前进度见
[格式化变参边界](runtime-value-delivery.md#格式化变参边界不能把静态文本输出当作-println-交付)。

## 历史宿主输出演示

以下命令与输出记录当时的文本和单值实现。原程序现保存在
[host-stdout.fl](examples/host-stdout.fl)。当前 demo 脚本使用
[host-println.fl](examples/host-println.fl) 验收格式化变参。

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

## 尚未交付

println 当前接收 comptime 文本，花括号按原文输出。运行时整数、bool 与
用户类型通过 Display 和 println_value 输出。动态 str/String、格式串展开、
浮点格式化、allocator、buffered I/O、其它 Unix 平台均不在本次能力范围。
Layout 仅负责布局计算，不分配内存。runtime str 的内存来源与调用边界接线
仍需继续完成，不能仅把它标成 Copy 或擦除检查来开放这些接口。

## 验证入口

工作目录均为 `/home/fin/luna`。不运行全 workspace 测试。

```sh
python3 -B development/check.py --timeout 900 -- test -p luna --test library_std -- --test-threads=1
python3 -B development/check.py --timeout 900 -- test -p program_verification rejection::tests -- --test-threads=1
```

library_std 使用真实 sysroot，编译、链接并运行原生产物。覆盖 O0/O3 中文与
NUL 保留、标量边界、自定义 Display、stderr、路径错误、文件往返、关闭与
自动 Drop、布局溢出及纯度拒绝。C interposer 仅用于测试进程，注入短写、
EINTR、零写入和部分写入后的 EIO，验证错误传播与已经输出的前缀。

当前验证证据（日志均由 development/check.py 生成）：

- `library_std`：8 个原生集成用例通过，退出 0；
  `/tmp/luna-tools-mk_b1sv8/result.json`，结果锚点 `8 passed; 0 failed`。
- `rejection::tests`：17 个通过，退出 0；
  `/tmp/luna-tools-gyzzoocu/result.json`，结果锚点 `17 passed; 0 failed`。
- `host_intrinsic_protocol`、`runtime_purity`、`sysroot_preludes`、`text_bytes`、
  `text_regions`：分别 39、65、15、9、51 个通过，退出 0；命令为
  `python3 -B development/check.py --timeout 900 -- test -p luna --test host_intrinsic_protocol --test text_regions --test text_bytes --test runtime_purity --test sysroot_preludes -- --test-threads=1`，
  `/tmp/luna-tools-bhwpxatk/result.json`。

- 数学 fixture 显式只绑定数值 intrinsic，平台操作不再被误映射到数学包。
  `python3 -B development/check.py --timeout 900 -- test -p luna --test library_meta_math --test target_integers -- --test-threads=1`
  退出 0，分别 9、27 个通过；`/tmp/luna-tools-f_b5swps/result.json`。

`--discard-result` 现在在既有所有权清理与失败检查后直接返回，不再打印
合成 wrapper 的 `(unit)`。普通带结果的开发入口仍保留 S-expression 输出。

最后一组受影响入口检查：

- `/tmp/luna-tools-ucnlz7vw/result.json`：sysroot 绑定单测 1 个、CLI 2 个、
  host 5 个通过；host 中另 1 个失败来自静默模式仍与解释器 `(unit)` 输出
  直接比较。已改为分别检查解释器诊断、原生静默输出与共同的析构行为。
- `/tmp/luna-tools-9a7g1wmc/result.json`：上述 host 用例与既有 runtime str
  借用拒绝用例分别通过，退出 0。调用命令为
  `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test resources -- --test-threads=1 native_discard_entry_selects_only_the_live_owned_error_payload borrowed_string_views_do_not_gain_pointer_free_layout_from_copy`。

编译器仍报告既有 unused/unreachable warnings；本次未将这些警告当作失败或
顺手清理他人改动，也没有执行全 workspace 测试。

最终演示：`python3 -B development/demo_println.py` 构建与运行均退出 0；
构建日志 `/tmp/luna-tools-kzljx98k/result.json`。产物 `/tmp/flurry-println`
输出中文、42、实际 Linux pid、i128 最小值和 done，没有 `(unit)` 尾行。

## 当时的演示复验

`python3 -B development/demo_println.py` 已在当前 dev 上重新构建并运行，
退出 0；构建报告为 `/tmp/luna-tools-0fwuk41k/result.json`。可直接运行
`/tmp/flurry-println`，输出如下（pid 随进程变化）：

```text
你好，Flurry！
answer = 42
Linux pid = 106
i128 minimum = -170141183460469231731687303715884105728
done
```

同一工作区执行
`python3 -B development/check.py --timeout 900 -- test -p luna --test library_std -- --test-threads=1`，
退出 0，`11 passed; 0 failed`；报告
`/tmp/luna-tools-xi8zy3i8/result.json`。包含泛型 ByteWriter 的状态与错误传播、
借用 File 输出，以及 O0/O3 下 Splice 中的输出次数：命名常量、显式调用和
实际执行的分支各构造一次，未执行分支不输出。

此项不证明任意 comptime 参数已能隐式转换为 runtime 值；该接线与动态文本
的内存来源仍待完成。演示使用当前已接通的文本与标量接口。
