# LLVM host pipeline（首个可执行子集）

当前 math 源码位于 `library/builtin/math_intrinsics.fl`，由 builtin.prelude 导出模块入口；
sysroot 绑定为 `builtin.math_intrinsics.target.Scalar`，不再加载独立 math 包。
下文历史检查记录中的旧路径与命令保留原貌；当前使用方法见 library/README.md。

## 普通 comptime 宽度与默认供应工厂（2026-10-25）

staged-bitvec 新增 `ordinary_factory(width)` 和带 comptime 可选默认值的 `defaulted_factory()`，返回相应宽度的普通回调；覆盖零宽、65 位、1024 位及闭包捕获。在 `/home/fin/luna` 执行 `python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900` 退出 0，O0/O3 均为 `(bool true)`，`/tmp/luna-host-examples-s1i3_h5i/summary.json`。相关 486 项 Rust 回归通过，`/tmp/luna-tools-ago0xws2/result.json`，包括失败默认供应与错误阶段输入的拒绝。

## 声明宽度函数工厂的原生执行（2026-10-25）

staged-bitvec 增加 `factory<N>`，返回 `ones<N>`；调用点分别取得零宽、8 位和 1024 位回调，并将宽回调捕获到闭包。定义检查不再提前求取抽象 N，实例阶段形成并验证完整 callable。

在 `/home/fin/luna` 执行 `python3 -B development/host_examples.py staged-bitvec list-clauses --release-driver --compare-optimization --timeout 900`，退出 0，四次 O0/O3 原生运行均输出 `(bool true)`，`/tmp/luna-host-examples-lyqszsii/summary.json`。相关 522 项 Rust 回归通过，`/tmp/luna-tools-8lyi0l15/result.json`。原 deferred-bitvec-factory 复现现为通过用例；通用符号宽度和完整 sysroot 尚未完成。

## 延后回调类型的原生交付（2026-10-25）

staged-bitvec 增加 `relay(width, callback)`，其输入及结果是显式延后的 `fn()->BitVec<width>`。具体函数值经过转交、闭包捕获后仍返回正确的 65 位或零宽结果。`python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900` 在 `/home/fin/luna` 退出 0，O0/O3 均为 `(bool true)`，`/tmp/luna-host-examples-qc7groly/summary.json`。相关 452 项 Rust 测试通过，`/tmp/luna-tools-ixxhaheh/result.json`。抽象宽度函数工厂的待补路径另记在前端检查点。

## 延后位宽工厂的字面量取反（2026-10-25）

staged-bitvec 现覆盖 `fn ones()->defer .BitVec'builtin<N> where N:Integer {not 0}` 及连续取反，包含零宽和 1024 位实例。开放定义保留字面量的目标上下文，实例阶段仍检查范围和运算资格，不给 Any 变量授予位运算能力。

`python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900` 在 `/home/fin/luna` 退出 0，O0/O3 均为 `(bool true)`，`/tmp/luna-host-examples-h5ijcegs/summary.json`。相关 439 项 Rust 回归通过，`/tmp/luna-tools-cdymyt_s/result.json`。此前保存的 deferred-bitvec-invert 失败现已修复。

## 调用点形成 BitVec 宽度（2026-10-25）

新增 `staged-bitvec`：显式 defer 让声明子句或普通 comptime 实参决定返回 BitVec 宽度，示例覆盖 0／1／2／65／1024 位及宽值切片。`python3 -B development/host_examples.py staged-bitvec --release-driver --compare-optimization --timeout 900` 在 `/home/fin/luna` 退出 0，O0/O3 均输出 `(bool true)`，`/tmp/luna-host-examples-bubftgfv/summary.json`。BitVec 局部 23 项回归通过；通用符号宽度及开放函数体运算符的剩余限制见前端检查点。

## 捕获声明值的 staging 示例恢复运行（2026-10-25）

list-clauses 中的函数工厂现在可以转交 bool／u8 声明值，生成函数保留捕获域和符号引用，具体实例完成替换。示例新增不同整数捕获实例的状态检查，并在目录入口绑定真实 resources.Copy／Drop，保证指针访问经过资源验证。

在 `/home/fin/luna` 执行 `python3 -B development/host_examples.py list-clauses --release-driver --compare-optimization --timeout 900`，退出 0，O0/O3 均输出 `(bool true)`，报告 `/tmp/luna-host-examples-uee6m44i/summary.json`。相关 452 项 Rust 回归通过，`/tmp/luna-tools-c_6uhz32/result.json`；运行器 9 项通过，`/tmp/luna-tools-0jm5q2jp/result.json`。下节记录的 list-clauses 前端失败已修复，完整 sysroot 和调用协议缺口仍保留。

## 当前工作区的原生交付验收（2026-10-25）

简短运行入口见 [examples/README.md](examples/README.md)。工作区已经具备源码 → 语义前端／IIR → LLVM IR → LLVM 验证与优化 → 目标文件 → host 链接／执行的实现，本次复用并验证该链路，没有另起一套后端。

LLVM 21.1.8 可用。工作目录 `/home/fin/luna`，执行 `python3 -B development/host_examples.py arithmetic number-theory library-tuple list-clauses --release-driver --compare-optimization --timeout 900`，报告 `/tmp/luna-host-examples-zowjagnm/summary.json`。arithmetic、number-theory、library-tuple 的 O0/O3 共六次编译、链接与原生执行通过，输出分别为 `(unsigned-integer 42)`、`(unsigned-integer 1121)`、`(bool true)`。

整批命令退出 1：最后的 list-clauses 新增声明值捕获在前端 elaboration 报 `required comptime evaluation failed: interpretation Type`，没有进入 LLVM，O3 未执行。对应前端测试 24 通过、1 失败，细节保留在 [前端检查点](frontend-checkpoint.md)。这不应隐藏为成功，也不阻止使用前三个示例。完整 sysroot 与一般 ABI 属性仍待补齐。

workspace 格式检查退出 0，`/tmp/luna-tools-b85v3r_x/result.json`。当前改动留在 dev，未提交。

## 库惰性回退与捕获资源（2026-10-25）

真实 builtin Option 新增 unwrap_or_else，Result 新增 or_else 与 unwrap_or_else，均使用已有 `%c fn` 闭包协议。Some／Ok 直接转交值，None／Err 才调用回调；未调用的回调仍负责清理捕获环境。Result.or_else 可更换错误类型，保持普通枚举与语言 error ABI 的边界。host-library-option-callback 与 host-library-result 覆盖两条分支、回调只执行一次、恢复失败的新错误类型及捕获资源恰好清理一次。

工作目录 `/home/fin/luna`：`python3 -B development/host_examples.py library-result library-option-callback --release-driver --compare-optimization --timeout 900` 退出 0，四次原生执行输出 `(bool true)`，报告 `/tmp/luna-host-examples-u11w33ar/summary.json`。`python3 -B development/library_check.py --release --timeout 900 --stage declarations --stage native-option --stage native-result` 退出 0，报告 `/tmp/luna-library-ide423p2/summary.json`；仅所选阶段通过。git diff --check 通过，未修改 Rust，不重复全 workspace 编译测试。

本轮先复现 sysroot 失败，报告 `/tmp/luna-library-p6qzah72/summary.json`，仍因 String、panic、分配／I/O hooks 等未接通而失败。新示例首稿误用保留字 error 作为参数名，报告 `/tmp/luna-host-examples-2hblyqoy/summary.json`；改为 reason 后完成上述验证。改动保留 dev 工作区，未提交。

## 从这里运行示例

当前 dev 工作区已有源码 → 语义检查／IIR → LLVM IR → 验证与优化 → 目标文件 → host 链接与执行的链路。先运行不依赖完整 sysroot 的示例：

```sh
python3 -B development/run_host.py development/examples/host-arithmetic.fl --release-driver --build-timeout 900
```

预期输出 `(unsigned-integer 42)`。返回值打印是开发驱动的适配器，不表示标准库 println 已接通。入口关闭 stdin，分别限制构建和执行时间，保留可执行文件、日志及 summary.json；终端会显示产物目录。`--release-driver` 控制编译器自身构建，生成程序的优化级别另由 `--opt-level 0` 至 `3` 控制。

带库绑定的示例使用目录入口，避免手工遗漏 Tuple、资源协议和包依赖：

```sh
python3 -B development/host_examples.py arithmetic number-theory library-tuple --release-driver --compare-optimization --timeout 900
```

本次重新验收（2026-10-25，工作目录 `/home/fin/luna`）：LLVM `21.1.8` 可用，上述三例各在 O0/O3 编译、链接、执行成功，分别输出 `(unsigned-integer 42)`、`(unsigned-integer 1121)`、`(bool true)`。报告 `/tmp/luna-host-examples-3yh3u5hq/summary.json`，命令退出 0。这是六次原生运行，不是解释器代跑，也不是完整语言或全量 sysroot 验收。

后端入口在 `compiler/luna/src/host/`：`mod.rs` 准备带资源检查的输入，`llvm.rs` 及子模块负责降码，`tools.rs` 管理 LLVM 工具和产物。继续沿用这一实现；前端待补项见 [前端检查点](frontend-checkpoint.md)。完整 String、panic、分配／I/O hooks 和一般函数 ABI 属性 elaboration 仍需后续接入。

## Cargo 输出转发失败不再丢失终态或堵塞子进程（2026-10-25）

检查配额失败路径发现 cargo.py 原转发线程按行读取，先写目的地再匹配配额消息；目的地 write/flush 抛出 OSError 后线程会退出，主线程既未取得该错误，也可能等待被满管道阻塞的子进程。改为最多 64 KiB 分块、跨块诊断匹配、先识别再转发。转发失败继续排空两路管道，进程结束后抛回 OSError；不执行成功发布回调、不重跑可能已执行完的程序，配额错误按现有授权清理缓存。清理状态提示使用尽力输出，提示写入失败不能阻断实际清理。外层 process_tools 的日志落盘失败仍单独报告，不能声称此次修改恢复了之前被截断的日志。

真实子进程测试覆盖两路超过管道容量的无换行输出、跨块配额文本、逐路 EDQUOT/ENOSPC/EPIPE 转发失败与另一路完整性；调度测试覆盖不重复执行、不发布和清理提示失败后的清理调用。工作目录 /home/fin/luna：全运行器测试 66 项通过（`python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p 'test_*.py'`，`/tmp/luna-tools-ma88ja6q/result.json`）。真实 Cargo 定向回归 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host invalid_llvm_tool_directory` 退出 0（`/tmp/luna-tools-z41bi66x/result.json`）。git diff --check 通过；未修改 Rust，不重复全量编译器测试。dev 未提交。

## LLVM 工具目录检查与失败产物保护（2026-10-25）

Toolchain::directory 现在检查 llvm-config --bindir 的成功输出：空白输出明确拒绝，不再让 join("opt") 退化为从 PATH 选择另一套工具；路径 canonicalize 后必须为目录。合法的带空格目录和目录符号链接仍接受。目录检查失败发生在调用 LLVM 子工具前。新增单元测试覆盖空输出、不存在路径、文件路径、恢复及符号链接；新增 CLI 回归验证以上错误不覆盖已有可执行文件，并在恢复真实工具链后编译执行成功。

工作目录 /home/fin/luna：工具链单元组通过（`/tmp/luna-tools-vdog0ltb/result.json`；当时共 19 项，随后删除与既有 host 链接器端到端测试重复的一项新增测试，保留新的目录测试）。格式化退出 0（`/tmp/luna-tools-ph1b75xr/result.json`），git diff --check 通过。

首次 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host linker` 构建遇到 EDQUOT，子进程退出 101，报告入口退出 125，stderr 被截断且终态报告无法保存；`/tmp/luna-tools-yjjn_gz8/result.json` 可能仍是旧 running 快照，不是活动任务证据。随后的 `invalid_llvm_tool_directory` 定向测试经构建入口恢复缓存后通过（`/home/fin/luna/luna-tools-1poc7w2e/result.json`），包含实际 CLI 失败保护与成功恢复。按已有授权排队执行的 cargo.py clean -p luna --release 释放 132 MiB，再重跑 linker 过滤组退出 0（`/tmp/luna-tools-_slc7eem/result.json`），证明真实链接失败诊断、缺失／空／非普通文件产物拒绝及旧产物保留。未改语言语义，dev 未提交。

## 独立转换协议与真实库原生验收（2026-10-25）

从 builtin/traits.fl 原样提取 Into<Target>、From<Source> 到 builtin/conversion.fl，traits 保留 re-export，原 builtin／prelude 路径不变。该模块可独立加载，不依赖 Display/Debug 的 owned String。没有增加隐式转换、自动 blanket impl 或资源豁免。新 host-library-conversion.fl 使用实际转换和资源协议，覆盖 generic From/Into、一次实参求值、同一主体不同目标接口、仿射字段转交及一次 Drop。默认示例目录增至 62 项，library_check 增加 native-conversion，默认十个阶段；选定阶段仍明确报告 scope=selected。

工作目录 /home/fin/luna，本轮证据：
- `python3 -B development/host_examples.py library-conversion --release-driver --compare-optimization --timeout 900`：退出 0，O0/O3 均为 (bool true)，默认预算。`/tmp/luna-host-examples-oj6gkzz1/summary.json`。
- `python3 -B development/library_check.py --release --timeout 900 --stage declarations --stage native-conversion`：退出 0，15 项库声明／解释／原生测试与三项新源码原生测试通过。`/tmp/luna-library-mc9bfcwq/summary.json`。随后将重复消费拒绝收紧为 unavailable 诊断，定向重跑通过：`/tmp/luna-tools-b5_bx1p8/result.json`。
- `python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p 'test_*.py'`：62 项通过，包含 native-conversion 缺项和失败报告拒绝。`/tmp/luna-tools-cpjprvev/result.json`。workspace 格式化退出 0（`/tmp/luna-tools-88pk10bh/result.json`），git diff --check 通过。
- 新 Rust 测试首稿误给解释 API 传模块路径；该 API 接收协议名称，修正为 Copy/Drop 后通过，源码绑定仍使用完整声明路径。
- 完整 sysroot 改前／改后均退出 1，26 条 E4005 诊断一致；仍缺 panic、String、Any 和分配／I/O hooks。改后证据 `/tmp/luna-library-4g5wc57a/summary.json`，不是全库通过。dev 工作区未提交。

## 捕获内存与原生回调集成复核（2026-10-25）

在 /home/fin/luna 执行 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test captured_memory --test method_memory_contracts --no-fail-fast`，退出 0；捕获内存 94 项、方法内存契约 9 项通过。证据：`/tmp/luna-tools-_ggcz2jv/result.json`。

执行 `python3 -B development/host_examples.py field-closure-state owned-field-callback owned-callback effect-callback-cleanup staged-callback-order handler-enum-callback --release-driver --compare-optimization --timeout 900`，退出 0。六个程序在 O0/O3 下共十二次真实原生执行全部通过，覆盖字段闭包状态、拥有资源的回调、效应回调清理、staging 求值顺序与枚举 handler 回调。证据：`/tmp/luna-host-examples-ubzcnuef/summary.json`。此次复核没有发现回归，不修改语言语义或实现；不是全量 sysroot 验收。改动继续保留 dev 工作区，未提交。

## LLVM ldexp 的异构签名与浮点边界（2026-10-25）

新增 ScaleByPowerOfTwo/ldexp 操作及真实 math.ldexp<T>(value:T, exponent:i32)->T 包装，T 限定 f32/f64。它与 powi 共用整数指数参数角色、generic/concrete 检查和 LLVM 异构参数降码，分别保留操作身份。LLVM 直接生成 llvm.ldexp.f32.i32 / llvm.ldexp.f64.i32，不先计算浮点 2^n 再乘，不额外开启 fast-math 或 flush-to-zero。更新 sysroot 清单、显式 CLI 夹具和示例配置；签名回归复用同一检查矩阵，包含错误指数类型、参数颠倒、pending 类型及篡改已签发证据的拒绝。

新示例 examples/host-math-ldexp.fl 覆盖两种精度、最小次正规数、恰在零／最小次正规数中点的下溢、负零、i32 极值指数、上溢和源码实参各求值一次。0.5 × 2^128 / 0.5 × 2^1024 的有限结果专门排除先生成溢出幂的错误改写。math 源码原生套件增至八项，示例目录现有 61 项。

工作目录 /home/fin/luna：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host_intrinsic_protocol` 退出 0，37 项通过（`/tmp/luna-tools-2d166sov/result.json`）。
- `python3 -B development/host_examples.py math-ldexp math-powi --release-driver --compare-optimization --timeout 900` 退出 0，两程序 × O0/O3 全部输出 (bool true)，均用默认编译预算（`/tmp/luna-host-examples-0r7kokax/summary.json`）。
- `python3 -B development/library_check.py --release --timeout 900 --stage native-math` 退出 0，八项实际原生测试通过（`/tmp/luna-library-for3zra9/summary.json`）；仅此选定阶段，不代表完整 sysroot。
- NIR lib 五项通过（`/tmp/luna-tools-isxm5sp4/result.json`）；运行器 61 项通过（`/tmp/luna-tools-g317nrhl/result.json`）；全 workspace 格式检查通过（`/tmp/luna-tools-e4q17ike/result.json`），git diff --check 通过。

构建前按已有授权清理 luna release 可重建缓存约 147.6 MiB，避免继续累计已出现的配额压力；本轮没有配额或链接失败。完整 sysroot 与属性／函数 ABI 等前置缺口保持记录，不因新增数学操作宣称语言已完成。dev 未提交。


## 聚合验收报告在配额失败时保留终态（2026-10-25）

上轮 library_check 的 summary.json 写入因 EDQUOT 抛出异常，无法交付已执行阶段的完整汇总。新增 process_tools.save_summary，采用临时文件与原子替换；写入或替换失败保留旧快照，终端输出 summary_saved=false、wrapper_exit_code=125、错误及完整内存 evidence。library_check、host_examples 和 run_host 共用此机制。汇总无法保存时受控停止，不再启动额外构建／执行；普通测试失败仍遵循原有 keep-going / 独立检查策略。run_host 将失败证据送往 stderr，不污染程序 stdout，finally 仍释放私有驱动。示例执行报告缺失／JSON 截断也记录明确失败，不抛出未处理异常或据子进程 0 宣称通过。

故障注入覆盖写入／rename 失败、旧快照保留及恢复、库阶段停止、keep-going 下停止、运行前／驱动完成后／执行完成后的报告失败、只运行一次、退出码保留、驱动释放、原始 stdout 字节及缺失／截断执行报告。运行器原 60 项通过（`/tmp/luna-tools-5ck8ts_q/result.json`），随后新增缺报告用例后的故障组 5 项通过（`/tmp/luna-tools-ruezxia1/result.json`）；没有把后来新增测试计入先前的 60 项。

工作目录 /home/fin/luna，真实正常路径均退出 0：`python3 -B development/library_check.py --release --timeout 900 --stage native-math` 的 7 项原生测试通过（`/tmp/luna-library-6tagibnu/summary.json`）；`python3 -B development/host_examples.py math-powi --release-driver --compare-optimization --timeout 900` 的 O0/O3 均为 (bool true)（`/tmp/luna-host-examples-insuwywp/summary.json`）；`python3 -B development/run_host.py development/examples/host-arithmetic.fl --release-driver --build-timeout 900` 输出 (unsigned-integer 42)（`/tmp/luna-run-host-4btu612w/summary.json`）。git diff --check 通过。未修改 Rust 或语言语义，不重复编译器全量测试；dev 未提交。


## LLVM powi 与异构 intrinsic 参数签名（2026-10-25）

新增 IntegerPower/powi 操作。NIR 的 operand_type 逐参数描述签名：底数和结果为同一 f32/f64，指数固定 i32。Registry::check、defer 和 Checked::validate_template 共用此规则；generic 类型证据仍由既有 PTC 类型域检查提供。实例化、重写后若指数类型变成浮点、u32 或 i64，旧 binding 不能继续授权。LLVM 先按源码次序求所有实参一次，再生成 llvm.powi.f32.i32 / llvm.powi.f64.i32，不将整数指数转换为浮点，不假定特定乘法／舍入顺序。依据 LLVM LangRef 的 powi 契约，链接见 library/README.md。

真实 math 库新增 pure 泛型 powi，更新 sysroot 默认项、示例运行器操作表及显式绑定夹具。examples/host-math-powi.fl 覆盖负指数、i32 最小指数、零指数、负零符号和实参求值顺序，接实际 resources 协议。math 原生源码套件增至七项；默认示例目录现有 60 项。未将这些 target 操作改为 comptime 可执行。

工作目录 /home/fin/luna：
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host_intrinsic_protocol`：36 项通过（`/tmp/luna-tools-9rex1_g_/result.json`），包括 concrete/pending 异构签名与重写伪造证据拒绝。旧全操作谓词测试矩阵已按各操作参数角色构造；FMA 原生测试仅对未启动子进程的 ETXTBSY 加入有界重试。
- `python3 -B development/host_examples.py math-powi --release-driver --compare-optimization --timeout 900`：O0/O3 原生执行均为 (bool true)，默认编译预算通过（`/tmp/luna-host-examples-x89kk2bq/summary.json`）。
- `python3 -B development/library_check.py --release --timeout 900 --stage manifest --stage native-math`：清单检查及七项 math 原生测试通过（`/tmp/luna-library-k4oyea1r/summary.json`），scope=selected，不代表完整 sysroot。
- NIR lib 5 项通过（`/tmp/luna-tools-i2wl5sgu/result.json`），luna lib 74 项通过（`/tmp/luna-tools-hc1ydnaz/result.json`）。运行器 56 项通过（`/tmp/luna-tools-2wgzgld2/result.json`），格式检查通过（`/tmp/luna-tools-89a9a9nx/result.json`），git diff --check 通过。

初稿测试把 Rigid 字段写成 id，已改为 declaration。联合 lib 构建先遇到 linker Bus error，后续明确出现磁盘配额及报告落盘失败（`/tmp/luna-library-v1_jg4lo`，最终内存证据为子进程 101、wrapper 125，磁盘旧 running 报告不能代表终态）。按已有授权执行 `python3 -B development/cargo.py clean -p luna --release` 释放约 396 MiB 后，上述清单和 luna lib 验证恢复；未删除源码或旧证据。

本轮改动前还复验 resources 365 项全部通过（`/tmp/luna-tools-3h6srguc/result.json`），历史列出的四项失败已不存在。函数 C ABI 仍需通用属性 Object 的求值与检查，维持已有明确拒绝，不靠名称特判放行。dev 工作区未提交。


## Tuple 仿射字段纳入库原生验收（2026-10-25）

library_check.py 新增 native-tuple，复用 host 示例的真实 tuple/meta/resources 包依赖和类型身份绑定。源码测试覆盖嵌套 comptime 类型列表、空 Tuple、单元素求值一次，以及带 Drop 的非 Copy 字段解构。仿射用例通过两个独立标志检查显式消费次序；Drop 的前置观察拒绝重复析构。没有为 Tuple 或 Tracked 添加隐式 Copy。Math 阶段改用与 host 示例相同的 MATH_OPTIONS，删除重复的操作清单。

入口新增可重复 --stage；按原有顺序去重执行所选阶段，终端和 JSON 都标明 scope=selected 及具体列表。不指定仍跑全九阶段，失败继续执行其它独立项；原生结果必须具有预期非零测试数和零失败。

工作目录 /home/fin/luna。`python3 -B development/library_check.py --release --timeout 900` 执行九阶段，八项通过、完整 sysroot 一项失败，最终退出 1（`/tmp/luna-library-yz5zyzav/summary.json`）；该次 Tuple 为前三项。随后加入仿射用例，`python3 -B development/library_check.py --release --timeout 900 --stage native-tuple` 退出 0，四项实际原生测试通过（`/tmp/luna-library-5mx35agp/summary.json`）。完整 sysroot 仍在 HIR lowering 报未解析 panic、String、Any、集合／I/O hooks，未冒充完成。

运行器单测 7 项通过（`/tmp/luna-tools-e3d0w1k8/result.json`），host 示例运行器 9 项通过（`/tmp/luna-tools-zigs7w11/result.json`），git diff --check 通过。本轮没有 Rust 源码修改，不重复 Rust 全量测试或格式化。dev 未提交。


## Sysroot 发现 math 包并绑定 LLVM scalar 默认项（2026-10-25）

单文件 --sysroot 加载器现在发现可选 math/lib.fl，追加在原包列表之后，保留已有包的身份分配。language-items.conf v1 新增 host-intrinsic.<operation> = package.Enum.variant；仓库清单列出所有当前标量操作及真实 math.target.Scalar 声明。默认项按 operation 合并，显式通用 CLI 选项、fma/sqrt/copysign 专用选项以及驱动已绑定身份优先；显式重复／冲突仍进入原有检查，不被合并消掉。非法操作名、重复项、缺少 enum/variant 的 selector 在加载时拒绝；解析后仍验证 intrinsic enum 身份和签名。AST/HIR 路径及显式 package-set 客户端不会隐式应用这些默认项。

新增真实 math/resources 模块的隔离 sysroot 回归，在 LLVM O0/O3 下执行 fma、sqrt 和 wrapping_abs；覆盖两种 CLI 覆盖写法、错误默认项保留旧可执行文件，以及绑定驱动身份优先。该夹具未加载尚不完整的 String/I/O 等模块，不代表完整 sysroot 已可用。更新使用真实仓库清单的旧 Tuple 测试，使其同时加载清单声明的实际 math 包。

工作目录 /home/fin/luna，`python3 -B development/check.py --timeout 900 -- test --release -p luna --lib --test frontend_cli sysroot --no-fail-fast` 退出 0，2 项配置单测及 18 项 sysroot CLI 测试通过（`/tmp/luna-tools-tttwlrvr/result.json`）。最初夹具缺目录、加载器缺 math 发现已修复；随后真实契约在默认 100000 总预算耗尽（`/tmp/luna-tools-5tlicgmf/result.json`）。原生组合回归显式使用 --fuel 1000000，未提高全局默认预算，不宣称预算问题已修复。最终格式化通过（`/tmp/luna-tools-lwlwuqt4/result.json`），git diff --check 通过。dev 工作区未提交。


## quoted tuple 契约的跨包原生示例（2026-10-25）

资源状态对照确认，旧 repro 中的契约解构把调用者 tuple 从完整状态消费为空；根集合、指针和临时值均未变化。这是缺少 Copy 的正确拒绝，不是观察路径泄漏临时绑定的证据。保留 repros/host-quote-contract-patterns.fl 为拒绝例，不削弱资源 gate。临时状态追踪代码已删除。

新增 examples/host-quote-contract-patterns.fl，以应用包本地 trait Duplicate 为真实库 Tuple<bool, bool> 显式提供实现，并由 host 参数绑定 Copy 协议身份。这符合本地 trait 的归属规则；不能在应用包为外部 resources.Copy 和外部 Tuple 声明实现，也不能因元素均 Copy 自动授予 Tuple Copy。标准库 Tuple 的通用复制实现仍待补充。

工作目录 /home/fin/luna，`python3 -B development/host_examples.py quote-contract-patterns --release-driver --compare-optimization` 退出 0，LLVM O0/O3 编译、链接、执行均输出 (bool true)，证据 `/tmp/luna-host-examples-20w7vjsx/summary.json`。目录现有 59 项；不是本轮全目录验收。此案例显式使用 1000000 总预算。新增 host 回归以同一源码删除 Copy 实现，分别验证两档接受／拒绝及拒绝时没有可执行文件。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host quoted_tuple_contract_requires_explicit_copy_and_preserves_caller` 退出 0，1 项通过、653 项未运行（`/tmp/luna-tools-2z2q5ebz/result.json`）。运行器 9 项通过（`/tmp/luna-tools-e5l69t9u/result.json`），全 workspace 格式检查通过（`/tmp/luna-tools-iao2w2dr/result.json`），git diff --check 通过。dev 未提交。


## 真实 Tuple 包与 host 阶段集成（2026-10-25）

阶段检查 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test library_protocols --no-fail-fast` 退出 0：653 项 host 和 14 项库协议通过（`/tmp/luna-tools-a_ujwx8k/result.json`）。这是本轮实际集成证据，不是沿用旧日志。

Tuple 模块的 Type 域改用 .Type'builtin，保持原签名、normal-form 头和递归定义，仅消除对完整 builtin prelude 的名称依赖。host_examples.Example 新增显式库间依赖边，library-tuple 把 tuples → meta 与应用依赖分别传给多包入口。源码使用真实 tuple/meta/resources 文件；comptime FieldTypes 返回类型列表，经 Tuple<...FieldTypes()> 构造空、单元素及嵌套类型，每个 tuple 按值消费一次后解构，没有假定 tuple 自动 Copy。

工作目录 /home/fin/luna，`python3 -B development/host_examples.py library-tuple --release-driver --compare-optimization` 在默认 100000 总预算下退出 0，O0/O3 均输出 (bool true)（`/tmp/luna-host-examples-gefv_2b4/summary.json`）。库源调整后完整 library_protocols 14 项再验通过（`/tmp/luna-tools-uxmh7ai6/result.json`）；运行器九项通过（`/tmp/luna-tools-t0ps6lyn/result.json`），包括精确的跨包身份／依赖边检查。目录现有 58 项。格式检查通过（`/tmp/luna-tools-060vex95/result.json`），git diff --check 通过；dev 未提交。

quoted 计算模式另存为 repros/host-quote-contract-patterns.fl，未登记为通过示例。真实资源协议下仍拒绝 guard 改变 caller resources（`/tmp/luna-run-host-2ncqg2lp/summary.json`），这是原先无资源协议解释测试没有覆盖的边界。试图在应用包为外部 Tuple 实现外部 Copy 协议依法被 Ownership(OpenBeforeLocalIdentity) 拒绝（`/tmp/luna-host-examples-8opnylnl/summary.json`），该尝试已移除。后续应核对观察／复制能力及正确归属包的库实现，不能通过跳过资源 gate 或非法外部实现来宣布该案例完成。

## 联合返回值／内存后置条件的原生示例（2026-10-25）

完整 memory-result-contracts 示例已登记，包含返回值与写入地址的关系、名义字段更新、默认指针别名及多次调用后的检查。依赖实际 library/builtin/resources.fl 的 Copy／Drop；源码没有删减，原 repro 路径保留相对软链接。目录现有 57 个示例。

```sh
python3 -B development/host_examples.py memory-result-contracts --release-driver --compare-optimization
```

该目录项显式传入 --fuel 1000000。CLI 默认总预算 100000 仍不足，未提高全局默认值；这与源码验证的独立预算不同。完整 sysroot 尚未完成；解构契约回归现已恢复，最新证据见 frontend-checkpoint.md。

工作目录 /home/fin/luna，命令退出 0，LLVM O0／O3 编译、链接和原生执行均通过，逐字节输出核验为 (bool true)。证据 `/tmp/luna-host-examples-t5dqiep4/summary.json`，可执行文件保留在各 case 目录。运行器八项测试通过（`/tmp/luna-tools-5j99b2_a/result.json`）；execution 1367 通过、1 项既有失败，详见 frontend-checkpoint.md。dev 未提交。

## 可运行入口交付复核（2026-10-25）

工作目录 `/home/fin/luna`，执行：

```sh
python3 -B development/host_examples.py arithmetic number-theory library-option library-result library-option-callback --release-driver --compare-optimization
```

退出 0，五个程序在 LLVM O0／O3 下共十次编译、链接、原生运行全部通过。算术输出 `(unsigned-integer 42)`，整数算法输出 `(unsigned-integer 1121)`，三个真实库程序均输出 `(bool true)`。回调程序包含 Some／None 两条路径以及拥有环境的调用和清理检查。证据与可执行文件保留在 `/tmp/luna-host-examples-310vu1mo/summary.json` 及同目录各 case。

运行器八项测试通过（`/tmp/luna-tools-2uhh2yhm/result.json`）。这是上述五个示例的本轮验收，不代表全部目录或完整 sysroot 验收。标准库 println、panic 等缺口和前端剩余失败见 frontend-checkpoint.md；打印返回值仍使用开发驱动。dev 未提交。

## 显式类型前缀与可运行 host 示例（2026-10-25）

普通函数、固有方法和 trait 方法的调用准备支持严格的位置参数前缀：固定用户给出的声明参数，剩余参数交由现有调用推断。因此 Option.map<bool>、Result.map_err<bool> 等可从回调取得尾部 Closure 上下文，而不会用推断覆盖显式结果类型。未完成实例不能独立保存；具备可选子句参数以及 named／spread 输入仍沿用原有完整路由。本段取代下文 map<bool> 尚不支持的阶段限制。

源码示例已改用显式结果类型与推断环境的组合。工作目录 /home/fin/luna，`python3 -B development/host_examples.py arithmetic library-option library-result library-option-callback --release-driver --compare-optimization` 退出 0，四程序共八次 LLVM O0/O3 编译、链接和原生执行全部通过，报告 `/tmp/luna-host-examples-smvdo09f/summary.json`。算术输出 (unsigned-integer 42)，三个库示例输出 (bool true)，回调示例检查调用次数及拥有环境清理。

局部回归命令 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test closure_environment_parameters --test library_protocols --test function_type_parameters --no-fail-fast` 退出 0，22 + 14 + 55 项通过（`/tmp/luna-tools-8u6oz8fk/result.json`）。全 workspace 格式检查退出 0（`/tmp/luna-tools-_ph3uaj7/result.json`），git diff --check 通过。

execution 组的后续修复与剩余失败以 frontend-checkpoint.md 顶部的最新检查点为准；完整 sysroot 尚未完成。host 的结果打印使用开发适配器，不等于标准库 println 已可用。改动留在 dev 工作区，未提交。

## 值组合器保留回调环境与不可达调用边（2026-10-25）

Option.map／and_then 与 Result.map／map_err／and_then 改为 %c fn 参数，c 的域为内建 Type.vars.Closure。旧普通 fn 目标依法拒绝非空捕获（复现 `/tmp/luna-tools-fh19rzhy/result.json`）；新接口保留真实环境，普通函数通过空环境消去接入，不授予闭包 Copy、不引入擦除或装箱。新增 examples/host-library-option-callback.fl，原 repro 路径为相对软链接；受检解释通过（`/tmp/luna-tools-0z4_8ga4/result.json`），登记示例两档通过（`/tmp/luna-host-examples-kdib0x34/summary.json`）。

新增五接口 × 调用／不调用 × 观察／消费捕获的 20 程序矩阵。它揭示静态已知 None／Err 的分支没有闭包调用清理上下文，而 LLVM 仍试图生成调用，原报 host closure call has no matching cleanup context（`/tmp/luna-tools-63naiqqx/result.json`）。现在只有已有完整父清理上下文时，缺少该次闭包调用上下文的边生成 trap + unreachable；不执行无证据回调、不借用其他调用帧、不退回规范闭包主体。缺少父上下文／整体 schedule 仍拒绝。合法未调用分支正常清理环境，已消费的捕获不重复清理。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_protocols --test host --no-fail-fast` 退出 0，host 653、库 14 项全部通过（`/tmp/luna-tools-q8itw9q8/result.json`）。新矩阵解释与 40 次 LLVM O0/O3 执行一致；首次测试拼接模块意外形成 {- 块注释已修正。格式检查（`/tmp/luna-tools-52rok_66/result.json`）、目录脚本八项测试（`/tmp/luna-tools-006g56rb/result.json`）及 git diff --check 通过。

新增环境参数也暴露显式参数限制：map<bool> 等只指定结果类型的调用报 trait argument count mismatch（`/tmp/luna-tools-08az38gu/result.json`），当前显式路径要求完整声明实参，部分显式参数与环境推断的接续仍需实现；README 已明确，不将其视为语言语义。普通推导调用和原有库案例通过。默认目录 56 项；完整 sysroot 未完成，dev 未提交。

## 配额错误下保留终态证据（2026-10-25）

针对 Option 验收期间真实出现的报告落盘异常，process_tools 的报告写入改为显式返回结果。首次报告失败不启动命令；PID 发布失败停止并回收已启动进程、关闭尚未由线程接管的管道；最终报告无法写入时返回 125，在终端 JSON 标记 report_saved=false 并附带完整内存中的终态证据，保留实际子进程退出码。旧 running 文件不被冒充为当前终态。日志失败继续排空管道，未保存字节也计入总量；信号处理在报告失败时仍恢复。

工作目录 `/home/fin/luna`。新增四类存储故障注入测试；首次测试发现启动失败后的退出码缺失与未关闭管道，已修复。最终 `python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p 'test_*.py'` 退出 0，53 项通过（`/tmp/luna-tools-lhenn10s/result.json`），包括实际子进程终止、超时、参数保真及构建／示例／库运行器回归。git diff --check 通过。本轮不修改编译器，不重跑 Rust 全量；dev 未提交。

## Option／Result 纳入完整库验收入口（2026-10-25）

library_check.py 新增 native-option／native-result 两个阶段，加载实际库与 resources 包，Option 额外绑定真实可选类型身份。示例新增 value_operations 源测试，先调用 run 再检查结果；运行器要求各一项测试且零失败，不能用空测试集或仅编译成功冒充执行通过。所有阶段仍按所选 Cargo profile 运行，任何失败不跳过后续独立检查。

工作目录 `/home/fin/luna`，`python3 -B development/library_check.py --release --timeout 900` 实际执行八阶段：声明、清单、resources／comparison／math／Option／Result 原生阶段七项通过，完整 sysroot 一项因既有未实现依赖失败，最终退出 1，汇总 passed=false（`/tmp/luna-library-aa5h62r7/summary.json`）。这是完整库未完成的证据，不把七项通过记成整体验收通过。运行器五项 Python 测试通过（`/tmp/luna-tools-ayt6nzs7/result.json`），覆盖 profile、独立失败继续执行、输出契约以及新增两阶段的空集／失败拒绝；git diff --check 通过。未修改编译器语义，dev 未提交。

## 真实 Result 双分支值操作（2026-10-25）

result.fl 保留真实普通枚举及 is_ok／is_err／map／map_err／and_then；依赖 panic 的 unwrap／unwrap_err 原样移到 result_unwrap.fl，完整 builtin 继续发现并使用这些公开方法。查询结果改用独立可用的 bool 拼写。该库枚举与语言 !E T 错误结果 ABI 保持区分，没有用库拆分替代错误系统实现或选择 panic 契约。

新增 examples/host-library-result.fl，使用真实 Result 与资源包，目录登记 library-result。受检解释通过（`/tmp/luna-tools-zeauq6w6/result.json`）；`python3 -B development/host_examples.py library-result --release-driver --compare-optimization` O0/O3 均通过（`/tmp/luna-host-examples-yd_x6se0/summary.json`）。新库测试验证两个仿射载荷分支的选择性消费、map_err 清理、and_then 将成功载荷转入错误结果时的责任保留、最终查询清理，解释与两档 LLVM 全部通过。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_protocols --no-fail-fast` 最终退出 0，13 项通过（`/tmp/luna-tools-1uh_h_o0/result.json`）；首次新测试的类型参数结束与赋值之间缺空格，被解析为 >=，修正后重跑，首次退出 101 不记为成功（`/tmp/luna-tools-a3mtk4jd/result.json`）。目录脚本 8 项 Python 测试通过（`/tmp/luna-tools-v5jj1_1x/result.json`），格式检查通过（`/tmp/luna-tools-necxruty/result.json`），git diff --check 通过。完整 sysroot smoke 仍报告 option_unwrap／result_unwrap 的 panic 与 String／分配／I/O 等缺项（`/tmp/luna-tools-zu4oe3ox/result.json`）。默认目录 55 项，本轮只运行新增项；dev 未提交。

## 真实 Option 值操作的独立 host 接入（2026-10-25）

将 Option 的 panic-dependent unwrap 原样移到 builtin/option_unwrap.fl，完整 builtin 仍发现该模块并保留同一公开方法；option.fl 本身保留真实定义、查询、unwrap_or、map、and_then，查询结果使用独立可用的 bool 拼写。没有提供 panic 占位实现或改变 unwrap 失败语义。该分离让现有值操作能作为真实模块独立验收，不代表完整 sysroot 已完成。

新增 examples/host-library-option.fl 和默认目录项 library-option，直接加载 option.fl、resources.fl 并绑定 optional.Option。受检解释输出 true（`/tmp/luna-tools-_066imyg/result.json`）；公开入口 `python3 -B development/host_examples.py library-option --release-driver --compare-optimization` 两档通过（`/tmp/luna-host-examples-46xo1i3d/summary.json`）。示例使用显式 Option<bool> 构造器，未把当前构造器推断限制改成语义承诺。

库回归使用真实 Option／资源声明，核对非 Copy Watch 的 fallback 退休、map 消费、and_then 返回所有权及查询消费清理；析构检查未提前或重复发生。解释与 LLVM O0/O3 均通过。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_protocols --no-fail-fast` 最终退出 0，12 项通过（`/tmp/luna-tools-ch7s3xnr/result.json`），含全部实际库文件的词法／解析检查。首次运行在链接时触发磁盘配额，报告落盘也失败（保留日志目录 `/tmp/luna-tools-0ujumh2y`）；通过已授权 cargo.py clean --release -p luna 清理 362.2 MiB 可重建产物后重跑成功，没有清理源码或日志。目录运行器 8 项 Python 测试通过（`/tmp/luna-tools-uo2x8dsg/result.json`），格式检查通过（`/tmp/luna-tools-oq619ygr/result.json`），git diff --check 通过。

真实完整 sysroot 再次执行仍失败，panic 诊断现在准确指向 option_unwrap.fl，Result／String／分配／I/O 等缺项保持报告（`/tmp/luna-tools-w401985n/result.json`）。未修改编译器语义；dev 未提交。默认示例目录现为 54 项，本轮只运行新增项。

## 默认资源绑定的内联／结果清理入口（2026-10-25）

CLI 的 inline-functions／discard-result 不再在读取 sysroot 之前强制要求显式协议参数；指定 sysroot 时允许延后校验。真正执行阶段仍要求完整 Copy／Drop 身份：解释和 host 内联入口即使未取得任何身份也进入缺项诊断，不能把内联请求悄悄退回无检查执行；丢弃结果沿用其完整绑定检查。没有默认身份且不要求这些能力的普通开发入口保持原行为，单独所有权分析范围不变。

扩展真实 resources 模块验收，默认绑定同时用于解释／两类源测试的内联选项以及原生丢弃结果（输出 unit）。新增不完整清单矩阵覆盖仅 Copy、仅 Drop、两者皆无，检查解释、LLVM 输出、解释式／原生源测试、内联与结果清理的缺项拒绝，所有拒绝均保留旧产物。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --lib --test frontend_cli --test packages --no-fail-fast` 退出 0，lib 73、CLI 203、多包 48 项全部通过（`/tmp/luna-tools-l847cg9k/result.json`）。随后仅去除一个 if 条件的多余括号警告，无行为变化；格式检查通过（`/tmp/luna-tools-fd2e284z/result.json`），git diff --check 通过。完整 sysroot 未完成，dev 未提交。

## Sysroot 默认资源绑定进入执行（2026-10-25）

实际 library/language-items.conf 现在绑定 builtin.resources.Copy／Drop。行为验收发现此前即使清单已提供资源身份，Action 与单文件 host 入口仍仅按命令行 protocols 是否存在选择受检执行，导致默认绑定下跳过析构，Watch 用例错误返回 false（`/tmp/luna-tools-o8y9kfsj/result.json`）。现以最终解析的身份加显式配置决定是否进入资源路径，解释执行、comptime invocation、host 输出与构建、解释式／原生源测试同步调整；缺少配对身份仍由受检入口报错，不回退成无检查执行。显式协议覆盖及只分析所有权的 action 范围保持原契约。

新增隔离测试加载真实 resources.fl 和真实清单中的资源条目，包根按可见性规则重导出 Drop。它检查作用域退出修改指针目标、两档原生结果、两种源测试入口、双重消费拒绝及旧产物保留。该夹具不加载尚未完成的 String／panic 模块，不冒充完整 sysroot。原真实清单类型构造器测试同步加载真实 resources 模块，清单单测核对资源身份。

工作目录 `/home/fin/luna`。最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli --test packages --no-fail-fast` 退出 0，CLI 202、多包 48 项全部通过（`/tmp/luna-tools-126jz4dr/result.json`）。清单单测此前通过（`/tmp/luna-tools-aq_7buiy/result.json` 中 lib 1 项）；该组合运行的 CLI 曾因夹具直接访问私有子模块失败，未记为成功。接线修复后的中间运行曾因新增源测试把有副作用调用直接放进 assumes 而失败（`/tmp/luna-tools-1zor9cpd/result.json`），已改为先调用再断言结果。最终格式检查通过（`/tmp/luna-tools-1rwwqbnw/result.json`），git diff --check 通过。

本轮重新执行真实 library-smoke：名称解析仍报 panic、String、Any、分配／集合／I/O 等未完成依赖（`/tmp/luna-tools-1eioaxfp/result.json`），没有用占位函数使其通过。完整标准库未完成；dev 未提交。

## 嵌套安装的候选稳定性回归（2026-10-25）

新增 `repros/host-handler-nested-callback-installation.fl`，外层 handler 连续请求内层 handler，内层借用同一回调；两次外层安装之间切换 identity／invert，解释与 LLVM O0/O3 必须得到相同结果。扩展原候选快照测试同时执行平坦与嵌套正例。另六个拒绝变体覆盖被处理计算、外层分支、内层分支中的代码重绑定或地址暴露，包括内层已经结束、外层仍存活时的赋值；失败仍保留既有产物。本轮确认上一轮实现覆盖这些交互，没有新增编译器例外。

工作目录 `/home/fin/luna`。首次拒绝用例把普通函数 subject() 错写为 operation 请求 subject()#，先触发 effect-row elaboration 诊断，命令退出 101（`/tmp/luna-tools-l36uksnq/result.json`）；修正调用语法后，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host handler_callback_snapshot --no-fail-fast` 退出 0，两项测试通过，覆盖六个拒绝程序、两个正例及四次原生执行（`/tmp/luna-tools-avstf3m6/result.json`）。独立嵌套正例解释输出 `(bool true)`（`/tmp/luna-tools-ykl197mz/result.json`）。格式化与 git diff --check 通过；dev 未提交。

## 按安装范围保持 handler 回调候选（2026-10-25）

资源与内存检查不再把整个函数中已经结束的代码赋值视为当前 handler 的不稳定证据。地址暴露仍扫描完整词法作用域，普通代码赋值则扫描完整 EffectHandle 表达式，涵盖被处理的计算和所有分支。安装前、结束后的赋值可以通过；安装期间即使尚未发出第一次请求，也不能替换捕获的代码身份。已有候选完整性、枚举条件路径、布局、所有权和退出清理检查保留，普通闭包的存储验证仍采用原先范围。

新增 `repros/host-handler-callback-installation.fl`：同一变量先改为 invert，安装一次后改回 identity，再安装一次，两次结果分别为 true、false。受检解释和独立 LLVM O3 原生执行均输出 `(bool true)`（`/tmp/luna-tools-ace6o339/result.json`、构建 `/tmp/luna-tools-q899254v/result.json`、执行 `/tmp/luna-tools-n1864wb9/result.json`），产物 `/tmp/luna-handler-callback-installation`。host 回归执行 O0/O3；记录代码字段／整对象与已知空枚举在安装前更新改为执行正例，安装期间更新及各类地址暴露保留拒绝。新增内存摘要用例核对实际候选从写第一个参数切换到写第二个参数，安装后的改回不污染摘要，并检查低预算失败后的恢复。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources --test drop handler --no-fail-fast` 中 host 72、Drop 113 通过；resources 11 通过、1 个新用例因遗漏外层 # 解析失败，组合命令退出 101（`/tmp/luna-tools-48gyyk6k/result.json`）。修正用例后单独运行 `test --release -p luna --test resources handler --no-fail-fast` 退出 0，12 项通过（`/tmp/luna-tools-upkl4pyl/result.json`）。相关组共 197 项不同测试通过，没有将首次组合运行记作成功。格式检查通过（`/tmp/luna-tools-de0zal05/result.json`）。dev 未提交；更一般的安装期间代码身份变化与别名写回仍未开放。

## 上手入口复验（2026-10-25）

当前工作区已有 LLVM host 实现，本轮复用该实现，未重复搭建后端或放宽前端准入。前端待补项保留在 [frontend-checkpoint.md](frontend-checkpoint.md)，开发 README 顶部补上无需额外协议配置的单文件运行命令，并区分开发驱动的结果打印与尚未接通的标准库 println。

工作目录 `/home/fin/luna`。`python3 -B development/host_examples.py arithmetic number-theory field-closure-state --release-driver --compare-optimization` 退出 0，三个程序的六次 O0/O3 原生构建与运行通过，报告 `/tmp/luna-host-examples-bzu04o8n/summary.json`。单文件入口 `python3 -B development/run_host.py development/examples/host-arithmetic.fl --release-driver --opt-level 3` 退出 0，输出 `(unsigned-integer 42)`，编译器构建、原生构建和执行证据见 `/tmp/luna-run-host-hwixu__e/summary.json`。本轮只维护入口说明，没有重跑全量测试；dev 工作区未提交。

## Host 示例集阶段性集成（2026-10-25）

近期 handler／所有权路径调整后，重新执行整个登记示例集：`python3 -B development/host_examples.py --release-driver --compare-optimization --keep-going` 退出 0，原 47 个示例的 94 次 LLVM O0/O3 构建与执行全部满足各自输出契约（`/tmp/luna-host-examples-apd2b0hs/summary.json`）。范围包含算术、泛型、类型数据包、闭包、所有权返回、资源／比较／math 库调用及 direct／inline effect；未发现该示例集中的集成回归。

将 field-closure-state、handler-record-callback、handler-record-state、handler-owned-data、handler-restored-owner、handler-enum-callback 六个已验证场景纳入默认 host_examples 目录。源码移至 `development/examples`，原 `development/repros` 路径保留相对软链接，历史证据入口和 include_str 回归仍读取同一份文件，没有维护两套源码。开发 README 已提供批量运行命令。

新六项通过单独的 `host_examples.py ... --release-driver --compare-optimization` 执行验收，退出 0，12 次构建／执行全部通过（`/tmp/luna-host-examples-icry_hth/summary.json`）。因此当前登记 53 个示例共 106 次执行均有本轮证据，来自两次互不重叠的示例运行；没有重复整批旧例。所有软链接目标存在，git diff --check 通过。工作目录 `/home/fin/luna`；本轮未改编译器或库语义，完整 sysroot 与更一般语言能力不能由此视为完成。dev 未提交。

## Handler 枚举代码捕获的条件证据（2026-10-25）

移除枚举代码字段的一概拒绝。资源层复用带 VARIANT_PATH 的代码路径和 enum_domains：只保留控制代码叶可达性的祖先标签；若这些标签证明某个载荷不存在，向内存摘要显式传递该路径的空 prepared 候选。未知来源不能走空候选通道，未排除的代码叶仍需完整候选及原有布局检查。普通数据兄弟字段的标签不成为跨请求稳定依据，仍可按既有规则更新。

稳定性扫描保留全部声明代码路径，包括当前不存在的载荷。因此安装前或 handler 中将空枚举改成带回调分支、或者暴露其地址，不能继续使用安装时的缺席证明。handler 帧恢复必要的条件标签与带分支路径的代码候选；完整责任、Copy 交付及清理规则保持独立。

新增 `repros/host-handler-enum-callback.fl`，运行时选择 none／两个函数载荷，连续请求同时更新无关 bool 字段。原拒绝为 handler variant code capture requires conditional candidate evidence（`/tmp/luna-tools-i_cxi2i9/result.json`）。最终受检解释输出 `(bool true)`（`/tmp/luna-tools-maqkazry/result.json`），O3 构建／执行退出 0、输出相同（`/tmp/luna-tools-4pkbewqx/result.json`、`/tmp/luna-tools-lzaepis2/result.json`），产物 `/tmp/luna-handler-enum-callback`。

三个新 host 测试覆盖动态标签、各已知标签、两层嵌套枚举与内层已知空载荷，共六个程序、12 次 O0/O3 执行；四个重绑定／取址变体必须拒绝并保留旧产物。Drop 原条件代码 gate 用例改为验证已知空载荷的执行；资源摘要未知回调拒绝测试加入枚举参数，防止把未知当作缺席。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources --test drop handler --no-fail-fast` 退出 0：Drop 113、host 71、resources 11 项全部通过（`/tmp/luna-tools-6rvnmqg6/result.json`）。格式检查通过（`/tmp/luna-tools-a_5lxpfo/result.json`），git diff --check 通过。未改 LLVM ABI，未重复无关全组；dev 未提交。更一般的可变代码身份与含指针环境捕获仍需来源及状态写回证据。

## Handler 移动后恢复责任的清理验收（2026-10-25）

新增 `repros/host-handler-restored-owner.fl`：每次请求将借用 State 移入 handler 局部 old，给原槽放回新的完整 State，再退出本次请求。old 在 handler 边界退休，最后留在捕获槽中的值由安装者退休；没有 Copy 实现。受检解释和独立 O3 原生执行均输出 `(bool true)`（`/tmp/luna-tools-rj9m3mq4/result.json`、`/tmp/luna-tools-fsvod5xw/result.json`、`/tmp/luna-tools-li8p0e3o/result.json`），产物 `/tmp/luna-handler-restored-owner`。

本轮补充已有实现的边界验收，未修改编译器逻辑。Drop 测试核对完整 EnterFunction 事件序列：两次正常请求的普通完成／resume 各有三次 drop State；第一次请求词法 return 或恢复后谓词失败各有两次。Host 测试对上述四个程序执行 O0/O3，正常结果与受检解释一致，失败程序退出非零且不输出结果。另三种恢复前的 return／resume／谓词失败必须因借用责任不完整而拒绝，已有原生产物保持不变。恢复后失败可以运行清理，恢复前失败不能用后续不可达重建补足责任。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test drop handler_restored_owner --no-fail-fast` 退出 0，Drop 1 项、host 2 项通过（`/tmp/luna-tools-4uyhmw5v/result.json`）。格式检查通过（`/tmp/luna-tools-yvwvduzd/result.json`），git diff --check 通过。仅新增用例，未重复上一轮相关全组；dev 未提交。

## Handler 借用普通拥有型数据（2026-10-25）

无指针、无代码的非 Copy 捕获不再仅因缺少整体 Copy 证明而拒绝。此类根加入 borrowed_owner_roots，沿用独立 handler 帧、安装者持有责任和所有正常／失败／效应退出的完整性检查；代码根仍另行提供稳定槽证明。数据布局、别名暴露和指针来源准入保持原规则，不给捕获类型授予 Copy。退出时没有完整恢复的消费仍拒绝；结构体字段观察以及枚举／newtype 移动后重建完整槽的已有操作可以通过。

新增 `repros/host-handler-owned-data.fl`，重复请求更新 State，最终由安装者析构；析构核验 live 和最终 flag。原始拒绝为 scalar Copy evidence（`/tmp/luna-tools-qx75i8cj/result.json`）。最终受检解释输出 `(bool true)`（`/tmp/luna-tools-_g3ztgmf/result.json`），LLVM O3 构建／执行退出 0、输出相同（`/tmp/luna-tools-d958_iz_/result.json`、`/tmp/luna-tools-dopgtsvl/result.json`）；产物 `/tmp/luna-handler-owned-data`。

新 host 测试覆盖直接／嵌套拥有型记录、普通完成／resume／词法 return、正常与损坏初始 live，共 12 个程序、24 次 O0/O3 执行；损坏状态必须运行失败且不输出结果。另四个退出变体检查消费整个捕获后没有恢复责任时拒绝、旧产物保留。原跨函数 aggregate 用例扩展为有／无 Copy 两种路径，实际检查结构体、枚举和 newtype 的执行结果。Drop 中旧的一律拒绝数据／已知代码聚合的测试改为执行正例，条件枚举代码证据仍保留拒绝；消费借用资源的拒绝测试改为核对精确完整性诊断。

工作目录 `/home/fin/luna`。初次 handler 组 host 65 通过、1 项旧 Copy 拒绝断言失败，resources 11 通过（`/tmp/luna-tools-5is8g34h/result.json`）。扩大到 Drop 后，host 66／resources 11 通过，Drop 111 通过、1 项旧诊断断言失败（`/tmp/luna-tools-pp0mgnbt/result.json`）。修正诊断断言后，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test drop handler --no-fail-fast` 退出 0，112 项通过（`/tmp/luna-tools-gvmkpcb3/result.json`）。最终相关组共 189 项不同测试通过；没有将前两次组合运行记为成功。格式检查通过（`/tmp/luna-tools-6o6nmd0l/result.json`），git diff --check 通过。dev 未提交。

## Handler 记录的独立数据字段写入（2026-10-25）

稳定代码捕获的资源检查现在比较完整字段路径：赋值位置与任一代码字段存在祖先／后代重叠时仍拒绝，只写入不重叠的数据兄弟字段可以通过。原内存分析已经允许纯数据字段写入，本轮使资源层与该规则一致。地址暴露仍归一化到整个捕获根，因此即使只给数据字段取址也不会获得未经证明的共享存储许可；代码字段、其父记录的替换与安装前的同类改动仍保持拒绝。路径比较计入共享工作预算，所有退出的完整记录所有权检查保留。

新增 `repros/host-handler-record-state.fl`：记录同时保存函数值与可变 flag，连续两次请求通过同一回调观察交替状态，安装者随后读取最终 flag 并再次调用回调。修复前拒绝为 handler callable capture rebinding or address exposure requires a writeback contract（`/tmp/luna-tools-qbndio2a/result.json`）。修复后受检解释输出 `(bool true)`（`/tmp/luna-tools-4n57y79u/result.json`）；O3 构建／执行退出 0，输出相同（`/tmp/luna-tools-kcfwvcap/result.json`、`/tmp/luna-tools-vybc4z6q/result.json`），产物 `/tmp/luna-handler-record-state`。

新增两项测试覆盖直接／嵌套记录、普通完成／resume／词法 return，析构分别检查两次更新后的 false 与提前返回时的 true；六个正例共 12 次 LLVM O0/O3 执行与受检解释一致。另十个拒绝变体分别在安装前或 handler 中重绑代码字段、替换整对象、对整个记录／代码字段／数据字段取址，失败不覆盖旧产物。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources handler --no-fail-fast` 退出 0，host 64 项、resources 11 项通过（`/tmp/luna-tools-ydemdwnp/result.json`）。格式检查通过（`/tmp/luna-tools-1b7x2bp2/result.json`），git diff --check 通过。没有修改普通调用分析或 LLVM ABI，未重复完整测试组；dev 未提交。

## Handler 借用记录中的回调字段（2026-10-25）

handler 的代码捕获证据由单一根扩展为完整字段位置。按 callable_value_paths 收集并核验每个字段的候选，named／prepared 内存证据使用同一字段路径进入 handler 摘要；资源帧同时恢复根或字段的 Callable。记录本身按借用处理，安装者继续持有清理责任。稳定存储检查仍覆盖整个捕获根，所有正常／失败／效应退出都要求整个借用记录完整，不能仅保留回调而交出兄弟仿射字段。

布局谓词单独检查无指针的数据／代码聚合，不赋予 Copy 或行为证据；每个闭包叶的环境继续要求纯数据布局。枚举条件代码字段仍需条件候选传输，明确拒绝；含指针／代码的闭包环境、捕获槽重绑定和地址暴露也未由本轮放开。普通 operation 输入仍用原先的数据布局规则。本轮未改 LLVM ABI。

`repros/host-handler-record-callback.fl` 在修复前报 handler lexical capture requires scalar Copy evidence（`/tmp/luna-tools-5xzuqmw8/result.json`）。修复后受检解释输出 `(bool true)`（`/tmp/luna-tools-heeifrtm/result.json`），LLVM O3 构建／执行退出 0、输出相同（`/tmp/luna-tools-2yylp9co/result.json`、`/tmp/luna-tools-85cmwkvg/result.json`）；产物 `/tmp/luna-handler-record-callback`。该回调拥有没有 Copy 的 State，两次请求后仍在原环境中调用并只清理一次。

新增三组 host 测试：匿名／名义嵌套记录、嵌套 handler、普通完成／resume／词法 return；两个不同 named 回调字段及运行时候选的调用顺序；消费回调或非 Copy 兄弟字段后的四类退出拒绝。正例合计 20 次 LLVM O0/O3 执行与受检解释一致，八个拒绝变体保持旧产物。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources handler --no-fail-fast` 退出 0，host 62 项、resources 11 项通过（`/tmp/luna-tools-54h_n8f8/result.json`）。格式检查通过（`/tmp/luna-tools-e6qmzz_e/result.json`），git diff --check 通过；未重复完整测试组，dev 未提交。

另更正上一轮默认参数记录：正式 docs/resource-management/runtime-purity.md 明确禁止默认表达式隐式消费外部非 Copy 绑定，`host-owned-default-callback.fl` 的 P3 拒绝符合现行契约。复现注释及下文记录已修正，不将其作为放宽准入的任务。

## 泛型拥有型回调的完整交付示例（2026-10-25）

新增正式示例 `examples/host-owned-field-callback.fl`，接入 host_examples 的 `owned-field-callback` 项及开发入口说明。拥有非 Copy State 的闭包由记录字段交给泛型 warm，调用两次后按值返回，再装入嵌套记录调用一次。最终析构要求调用次数恰好为三、bit 为 true、live 仍成立且 done 尚未写入；然后更新 done。该程序同时观察环境状态保留、返回值存活及最终责任，没有为闭包声明 Copy，也没有绕过资源检查。

本轮确认这些路径已有实现，未修改编译器。新增集成测试分别覆盖尾结果、显式 return、默认／显式 bool 参数；每种路径再测试正常、少调用、多调用、提前标记 retired 四种变体。12 个程序的受检解释与 24 次 LLVM O0/O3 执行符合预期，负例必须运行失败且不输出结果。另一个测试确认转移后的原记录字段仍不可调用，失败不覆盖旧产物。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host owned_field_callback --no-fail-fast` 退出 0，两项测试通过（`/tmp/luna-tools-os8y41vx/result.json`）。公开入口 `python3 -B development/host_examples.py owned-field-callback --release-driver --compare-optimization` 退出 0，两档输出均为 `(bool true)`；报告和原生程序保留在 `/tmp/luna-host-examples-dvgrvlw4/summary.json`。格式检查通过（`/tmp/luna-tools-d1vhs8h_/result.json`），git diff --check 通过。本轮未改公共分析路径，因此没有重跑上一轮 637 项 host／364 项资源全组。

另外保留 `repros/host-owned-default-callback.fl`：让默认参数直接取得前序拥有型回调 input 时，目前被默认表达式的 P3 证据检查拒绝（`/tmp/luna-tools-lw8h_yim/result.json`）。后续核对正式 docs/resource-management/runtime-purity.md：默认表达式不能隐式消费外部非 Copy 绑定，因此本例拒绝符合现行契约，应保留；不是待删除的 P3 限制。正式示例仅使用已证明的 bool 默认值。所有改动在 dev 工作区，未提交。

## 拥有型字段中的闭包原位调用（2026-10-25）

闭包调用分析的 receiver 由单一 PlaceId 扩展为完整 Place：捕获导入、普通／失败／效应退出的所有权写回、指针来源与枚举标签恢复都保留接收者的字段前缀。代码候选从对应根或字段槽读取／更新，实参求值后继续检查接收者身份没有被替换；不构造拥有型临时 callee。LLVM 原有 closure_place 会使用原字段的环境地址，本次未增加环境复制或改变闭包 ABI。

指针写回只清除接收者子树中的旧来源，保留同一记录中其他字段；枚举条件路径同步加上接收者前缀。路径工作继续计入共享分析预算。按值交出整个闭包仍要求完整所有权，已经消费的捕获不会因调用而恢复。真正的临时 callee 和通过指针读取的 callee 仍走原有路径，不据此声称任意别名闭包借用已经完成。

新增 `repros/host-field-closure-state.fl`。原受检解释在第二次调用时报字段 unavailable（`/tmp/luna-tools-_uv4lb5v/result.json`）。修复后解释输出 `(bool true)`（`/tmp/luna-tools-tg3b_81a/result.json`），LLVM O3 构建／执行退出 0、输出相同（`/tmp/luna-tools-h9e0fd30/result.json`、`/tmp/luna-tools-s_ya3g54/result.json`）。产物 `/tmp/luna-field-closure-native`。

六项新测试覆盖直接／嵌套记录、bool／非 Copy 结构体环境、重复调用、指针捕获重绑定、枚举指针来源与标签、兄弟字段保留、条件消费后残余清理、重复消费拒绝及不完整闭包的按值交付拒绝。失败路径通过替换开发入口适配器观察析构计数，保留生成的函数与清理代码，核对内外两项责任各清理一次；O0/O3 均验证。首次四项定向测试通过（`/tmp/luna-tools-og2be1w7/result.json`）。

工作目录 `/home/fin/luna`，最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources --no-fail-fast` 退出 0：host 637 项、resources 364 项通过（`/tmp/luna-tools-orr5q791/result.json`）。格式检查退出 0（`/tmp/luna-tools-9nhvunlw/result.json`），git diff --check 通过。改动保留 dev，未提交。

## 非 Copy 函数字段调用与 host 示例验收（2026-10-25）

已知、无环境的函数值位于拥有型字段时，调用现按位置观察，不再把字段移动到临时 callee 后清理。观察前检查该位置完整可用，且在实参求值前确定目标；不要求已移走兄弟字段的父记录整体完整，不赋予函数值 Copy。单目标及候选集合复用原有调用分析，保留标量、指针、含指针聚合结果的来源。真正临时表达式、带环境闭包和经指针读取的 callee 沿用原路径，未由此实现一般字段闭包借用。

`repros/host-noncopy-field-callee.fl` 的原始拒绝是第二次调用时字段 unavailable（`/tmp/luna-tools-yymyk_3d/result.json`）。新增测试覆盖名义／嵌套／匿名记录、单目标／运行时选择、三种结果形状、实参重绑定 callee 时仍调用原先目标、兄弟字段移动后调用，以及已移动 callee 必须拒绝且不覆盖旧产物。40 次 LLVM O0/O3 原生执行与受检解释一致。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources --no-fail-fast` 退出 0，host 631 项、resources 364 项通过（`/tmp/luna-tools-3uuvy0ve/result.json`）。格式检查退出 0（`/tmp/luna-tools-jrnfcc85/result.json`），git diff --check 通过。

独立脚本验收：`python3 -B development/host_examples.py arithmetic number-theory linked-list generic-integers type-packets handler-parameter-control --release-driver --compare-optimization` 退出 0，六个示例的 O0/O3 共 12 次构建／执行均满足预期；产物与报告在 `/tmp/luna-host-examples-tyka1kou/summary.json`。现有 LLVM 工具链可用。前端恢复入口已整理到 [frontend-checkpoint.md](frontend-checkpoint.md) 顶部，完整 sysroot 仍未完成。所有改动保留 dev 工作区，未提交。

## 借用回调中的非 Copy 数据环境（2026-10-25）

handler 的闭包环境布局检查不再要求每个数据字段都有 Copy。环境被原地借用，完整候选、稳定槽、无指针／代码来源的数据布局仍须成立；所有正常、失败、效应退出的完整所有权检查继续负责禁止责任丢失。因此可以读取或更新拥有型结构体的数据字段，而不复制结构体、不在每次请求结束时清理环境，也不把按值消费当成借用。

新增 `repros/host-handler-owned-environment.fl`：闭包拥有没有 Duplicate 实现的 State，连续请求翻转其 bit，handler 结束后继续调用同一个闭包。State 的 Retire 检查 live、最终 bit，并把 live 写为 false；用于观察过早／重复清理以及未保留同一环境的问题。受检解释的原始拒绝为 handler aggregate capture requires scalar data without pointer or callable fields（`/tmp/luna-tools-qoyh820n/result.json`）。修复后受检解释输出 `(bool true)`（`/tmp/luna-tools-9u2k5hrh/result.json`），LLVM O3 构建、执行退出 0 且输出相同（`/tmp/luna-tools-r3zpp7nj/result.json`、`/tmp/luna-tools-omtwdrtq/result.json`）。

新增 host 测试覆盖普通完成、resume、词法 return；将 live 初始值改为 false 或省略必要的最后一次状态翻转时，受检解释及原生执行均失败，原生构建仍成功且执行不输出结果。8 个程序变体共 16 次 LLVM O0/O3 执行通过对应预期。另一组用例在回调体中按值交出 State，确认三种 handler 出口均拒绝、旧产物保持不变；这验证新增布局许可没有绕过完整环境检查。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources handler --no-fail-fast`：host 59 项、resources 11 项通过（`/tmp/luna-tools-6kp0dl97/result.json`）。格式检查通过（`/tmp/luna-tools-k2joov12/result.json`），git diff --check 通过；dev 未提交。本轮只改动 handler 环境准入，没有重跑上一轮已验证的完整 host／resources 组。

剩余限制：指针或代码字段的环境仍需要来源／候选写回，捕获槽的重绑定与地址暴露仍需对应共享存储契约。按值交出被借用环境的责任尚未支持请求侧所有权状态写回；该拒绝不能泛化成语言禁止 handler 消费仿射资源。


## Handler 借用非 Copy 回调与普通 callee 观察（2026-10-25）

handler 捕获可调用值不再一概要求该值具有 Copy。完整候选、稳定捕获槽和布局检查继续生效；当前允许函数值及拥有 Copy 数据环境的闭包，不为闭包赋予 Copy，也不放宽用户 impl 资格。独立 handler 帧分析之后检查 Report.exits、failed_exits、effect_exits 中所有捕获回调的完整可用性；词法 return／break／continue 经 exit 记录，resume／普通完成亦保留对应出口。借用环境不能被按值交出，失败或部分分支丢失环境也不能靠另一条正常路径恢复。环境仍由安装者负责清理，数据状态可在实际调用中更新。

该检查发现普通命名函数值 callee 原先经 expr(Local) 按值交付，调用就消费没有 Copy 证明的函数值。现对已解析的本地 named 单目标／多目标统一采用调用前观察：检查来源位置仍完整，再准备实参和调用，保留原位置。标量、指针、聚合结果三条入口一致；传参／赋值仍执行原本的 Move/Copy 规则，已经移动的位置不会因调用观察而恢复。临时 callee 与带环境闭包调用沿用既有专门路径。

新增 16 个 handler 程序覆盖非 Copy 命名函数和有状态闭包、直接／跨函数请求、普通完成／resume、内层 handler 调用；退出后再调用同一闭包，检查真实环境状态，没有隐式复制。另有 6 个普通命名 callee 程序覆盖单目标／候选、标量／指针／聚合结果。合计 44 次 LLVM O0/O3 执行与受检解释一致。拒绝测试覆盖按值消费借用回调后的普通完成、resume、词法 return、谓词失败，以及仅部分分支中的返回／失败；同时检查移动后的普通 callee 不可再用、构建失败保留旧产物。

首次 handler 组 56 通过、1 失败（`/tmp/luna-tools-cs9km7u4/result.json`），暴露 named callee 被消费；资源 11 项通过。改正后扩大检查 host 624 通过、1 项测试诊断断言失败，resources 364 项通过（`/tmp/luna-tools-5kn33gnf/result.json`）：移动后调用已拒绝，但指针依赖检查先报缺少 memory contract。拒绝夹具改为无指针函数以单独验证位置可用性。最终在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources --no-fail-fast`：host 625 项、resources 364 项全部通过（`/tmp/luna-tools-rwrng4f0/result.json`）。此次扩大范围是因为普通 callee 读取规则改变，不仅影响 handler。

`repros/host-handler-borrowed-callback.fl` 不为函数／闭包提供 Copy 实现，两次请求后再次调用同一闭包。受检解释输出 `(bool true)`（`/tmp/luna-tools-s76c_txx/result.json`）；LLVM O3 构建与执行退出 0、输出一致（`/tmp/luna-tools-edtr1k1o/result.json`、`/tmp/luna-tools-4wi798u_/result.json`）。格式检查通过（`/tmp/luna-tools-ui903xlp/result.json`），git diff --check 通过；dev 未提交。

剩余限制：含非 Copy 数据字段、指针或代码字段的闭包环境，以及回调存储的重绑定／地址暴露，仍需更一般的请求时状态和来源写回契约。环境布局限制是实现边界，不能写成 Once 禁止重复读取的语言规则；请求指针跨 handler 词法返回的替换也仍未完成。


## Handler 捕获 Copy 函数值的资源接线（2026-10-25）

资源帧现在保留稳定捕获槽的 Callable 身份，并把 named／prepared 内存候选分别交给 handler 参数摘要。命名函数不能经只用于非 named 候选的 memory_code_definitions 入口取证；已分别接通。捕获仍要求显式 Copy 证据、完整候选和全词法作用域内的稳定存储；嵌套 handler 继承已经检查过的稳定槽证明，捕获责任仍归安装者，不能在每个请求结束时重复清理。资源布局允许函数值；带环境代码的内部布局检查要求 Copy 数据环境，但不因此授予源闭包 Copy。

正式 docs/traits/declarations.md 禁止闭包类型作为用户 impl 主体。开发中的直接闭包 Copy impl 夹具已删除，未放宽该规则。当前源码验收覆盖有合法 Copy 证明的函数值和无捕获 lambda，运行时选择候选、直接／跨函数请求、内层 handler 再调用捕获回调。另用两次翻转验证每次请求调用一次，并检查缺少 Copy 或捕获槽重绑定时拒绝且不覆盖已有可执行文件。8 个组合程序加一个重复调用程序共 18 次 LLVM O0/O3 执行与受检解释一致。

首次编译因同名 CallablePlace 使用了资源层类型而失败（`/tmp/luna-tools-vznreg1l/result.json`），已修正。随后 host 53 通过、2 失败，暴露 named／prepared 通道路由问题（`/tmp/luna-tools-7ailmht2/result.json`）。最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources handler --no-fail-fast`：host 55 项、resources 11 项通过（`/tmp/luna-tools-er8xi2r1/result.json`）。格式检查通过（`/tmp/luna-tools-7db70tap/result.json`），git diff --check 通过；工作目录 `/home/fin/luna`，dev 未提交。

可运行复现 `repros/host-handler-copy-code.fl` 显式声明 `impl Duplicate for fn(*bool) {}`，handler 调用回调写入请求指针后再词法返回快照闭包。受检解释输出 `(bool true)`（`/tmp/luna-tools-wiym2uyy/result.json`）；LLVM O3 构建、执行退出 0，输出相同（`/tmp/luna-tools-hlzanc9p/result.json`、`/tmp/luna-tools-hoel9d9t/result.json`）。原无 Copy 证明的复现保留，拒绝没有被隐藏。

剩余工作是非 Copy 可调用值在请求时的借用／所有权保持：Once 不禁止重复读取，不能将本轮的 Copy 限制写成语言规则，也不能默认给闭包赋予 Copy。需在 handler 所有正常／提前／失败出口验证借用环境仍完整，并明确环境状态及代码候选写回；现有 Report 的 exits／failed_exits 可作为验证入口。用户 impl 资格限制保持原样，含指针／代码环境、聚合回调字段及仿射环境消耗尚未由本轮放开。


## Handler 捕获回调的内存证据（2026-10-25）

内存层现把安装位置已有的 named／prepared callable 候选带入 handler 的私有闭包摘要，复用普通闭包的捕获与调用分析；签名本身不提供行为证据。无指针代码环境可进入捕获布局检查，代码捕获存储必须在完整当前 runtime scope 内保持稳定，重绑定或取址仍拒绝。Infer 保存当前 scope 的引用，普通函数、独立表达式、闭包三个入口均接入；没有代码捕获时不做额外的整作用域扫描。此处没有放开资源帧的所有权／Copy 准入。

新增资源测试覆盖命名函数、无捕获 lambda、捕获 bool 的 lambda、运行时函数候选，以及这些候选穿过嵌套 handler 的调用。另用两个分别写入不同参数的候选验证足迹合并；检查未知回调、重绑定、安装前取址拒绝，低预算失败后恢复。首次资源组 9 项通过（`/tmp/luna-tools-zqg__ieu/result.json`）；最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test resources --test host handler --no-fail-fast`：host 52 项、resources 11 项通过（`/tmp/luna-tools-3dkntp_6/result.json`）。格式检查通过（`/tmp/luna-tools-rgqpqpql/result.json`），git diff --check 通过；工作目录 `/home/fin/luna`，dev 未提交。

明确剩余边界：`repros/host-handler-captured-code.fl` 修复前报 handler memory capture needs a data-only or stable pointer contract（`/tmp/luna-tools-v4my3nei/result.json`），现在仍在资源准入处报 handler lexical capture requires scalar Copy evidence; affine captures need request-time ownership（`/tmp/luna-tools-_vh1pfo_/result.json`）。因此不是完整的原生回调捕获支持。后续需明确复用已有 callable delivery／Once/Copy 判断，把资源 Callable 证据及其环境安装进 handler frame，并为可变或仿射捕获实现请求时所有权协议；不能仅删除准入检查。当前整作用域稳定性检查也保守拒绝安装之前的重绑定，不据此定义语言禁止该程序。


## 嵌套 handler 的参数来源与控制作用域（2026-10-25）

handler 参数内存摘要不再一概拒绝嵌套安装。新增有界可变 handler-scope visitor：遍历嵌套安装的 computation、捕获初始化及当前求值节点，保留内层 arm 和新闭包体的控制节点；每个内层 arm 由自己的摘要负责解释 resume。这样不会把内层 resume 错改为外层函数退出，词法 return 的资源与控制证据仍来自原始程序。owner 预扫描纳入内层模式绑定；嵌套结构数量有 64 的保守上限，并继续共享工作预算。该上限是当前分析能力限制，不是语言语法限制。

修复前的受检解释因 effect parameter access requires known pointer provenance 失败（`/tmp/luna-tools-72sxu3th/result.json`）。新增源码测试覆盖内外层各直接／跨函数请求，以及内层普通完成、resume、词法 return，共 12 个程序；受检解释与 LLVM O0/O3 共 24 次原生执行一致。资源测试用两个不同指针检查内层写入及 resume 之后外层的读写都保留，低预算失败后可恢复。复现 `repros/host-nested-handler-parameter.fl` 组合内层写入／resume、外层读取和返回快照闭包，受检解释输出 `(bool true)`（`/tmp/luna-tools-bqgf2h41/result.json`）。

工作目录 `/home/fin/luna`。首次 visitor 编译检查暴露编辑误将新参数加入不可变 walker，已修正（`/tmp/luna-tools-iem04iu5/result.json`）。最终 `python3 -B development/check.py --timeout 900 -- test --release -p nir --test closure_environment_visit`：5 项通过（`/tmp/luna-tools-w16mjfvk/result.json`）；`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources handler --no-fail-fast`：host 52 项、resources 7 项通过（`/tmp/luna-tools-ede9o06u/result.json`）。格式检查通过（`/tmp/luna-tools-dfew3vgd/result.json`），git diff --check 通过。dev 未提交；本轮没有扩大到全量测试，也未解除请求指针跨 handler 词法返回的来源限制。


## Handler 局部指针闭包的调用与逃逸（2026-10-25）

移除 handler 形成阶段对所有指针捕获闭包的一概拒绝。局部闭包的环境来源现在交由既有 memory closure 分析保存，在调用时映射其读写；当前 handler 的词法返回仍独立检查布局与来源。闭包内部 return 继续属于闭包作用域，因此可以把请求指针返回给 handler 内部使用，而不意味着允许指针随 handler 的词法返回逃逸。

新增资源测试直接核对 handler 参数摘要：构造但不调用闭包没有主体读写；调用闭包会保留读写；闭包返回指针后在 handler 内读写也保留对应参数来源。分别验证低预算失败与重新计算恢复。完整安装分析仍保留潜在清理的保守访问前提，不能把未调用主体等同于无需检查环境清理。源码原生测试覆盖直接／跨函数请求、写入闭包、返回指针的局部闭包，以及未调用主体的写入不发生，6 个程序的受检解释与 LLVM O0/O3 共 12 次原生执行一致。额外两种请求路径验证指针闭包跨 handler 返回仍拒绝，构建失败不覆盖已有产物。

工作目录 `/home/fin/luna`。首次测试目标编译失败是新增测试引用了 ArgumentSummary 不存在的 effect_inputs 字段（`/tmp/luna-tools-yxfwfp9j/result.json`），已修正。最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test resources --test host handler --no-fail-fast`：host 51 项、resources 6 项通过（`/tmp/luna-tools-2gf7hc4s/result.json`）。`repros/host-handler-local-pointer-closure.fl` 受检解释得到 `(bool true)`（`/tmp/luna-tools-pursvm3m/result.json`）。格式检查通过（`/tmp/luna-tools-jzz9627r/result.json`），git diff --check 通过。dev 未提交；未运行全量测试。


## Handler 返回闭包的形成／调用边界（2026-10-25）

handler 通过请求参数读写存储后，现可词法返回捕获无指针数据的闭包。私有内存摘要只改写当前求值作用域中的 return，保留捕获初始化的访问，不进入新形成的闭包函数体；因此闭包内部的 return 不会被误改为 handler 的退出。新增有界可变 visitor 与既有不可变 formation visitor 对齐，不修改共享闭包体。布局检查递归检查闭包环境，原资源分析仍负责 callable evidence、所有权和清理。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p nir --test closure_environment_visit`：4 项通过（`/tmp/luna-tools-jkvcpmhk/result.json`）；`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host handler --no-fail-fast`：49 项通过（`/tmp/luna-tools-4je_vcig/result.json`）。新增源码用例覆盖直接／跨函数请求、无捕获／bool 捕获、闭包显式 return，以及返回后修改原存储；4 个程序的受检解释与 LLVM O0/O3 共 8 次原生执行一致。

可运行源码 `repros/host-handler-closure-return.fl` 的受检解释得到 `(bool true)`（`/tmp/luna-tools-lsmr380o/result.json`）。指针捕获变体 `repros/host-handler-pointer-closure-return.fl` 的未受检解释可执行（`/tmp/luna-tools-ins5ukg0/result.json`），受检解释仍因 unknown memory target 拒绝（`/tmp/luna-tools-khj5ynen/result.json`）：请求相对指针来源的跨退出替换尚未实现，不能把这个实现限制视为语言禁止。格式化通过（`/tmp/luna-tools-jxvqpxri/result.json`），git diff --check 通过；dev 未提交。本轮未重跑全部 host 或标准库。


## Handler 参数访问与词法代码结果（2026-10-25）

访问请求指针后返回命名函数，原先仍被词法返回的数据布局门槛拒绝，导致丢失参数访问摘要并报 unknown memory target（修复前 `/tmp/luna-tools-z3niil8w/result.json`）。布局检查现保留严格的 effect_data_layout 入口，另在词法返回分析中允许无捕获代码身份及其聚合字段。函数签名的指针参数／结果不是该代码值的存储别名；原始资源分析继续发布和核验 CallableEvidence。operation 输入、完成结果与捕获布局的检查未据此放宽；存储中的数据指针及捕获闭包仍须独立来源依据。

新增源码回归覆盖两种请求路径，分别返回普通函数、带指针参数的函数、错误结果中的函数，以及拥有型结构体／枚举中的函数字段。词法返回与普通返回路径刻意使用不同的代码身份或错误分支，防止仅因返回类型相同就混合证据。10 个程序的受检解释和 LLVM O0/O3（20 次原生执行）全部通过。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host handler --no-fail-fast`：48 项通过（`/tmp/luna-tools-khzuzfoh/result.json`），包含原有数据指针返回拒绝和资源清理回归。

格式化通过（`/tmp/luna-tools-y1g5ysot/result.json`），git diff --check 通过。可复现源码 repros/host-handler-code-return.fl 的受检解释执行得到 `(bool true)`（`/tmp/luna-tools-r4owvjho/result.json`）。工作目录 `/home/fin/luna`，dev 未提交；本轮未重跑整个资源或 host 全组。

## 失败后的工具后代进程清理（2026-10-25）

LLVM 工具和原生测试通过共同的启动入口在 exec 前建立独立进程组。超时、输出超限或其他失败时，先终止该组，再回收直接子进程；因此即使主进程已经退出，仍持有输出描述符的后代也会被终止。只作用于本入口创建的子进程组，不覆盖主动脱离该组的进程。成功输出与退出状态协议保持原样。

新增真实子进程回归覆盖主进程等待／先退出、输出超限三种失败：确认后代已启动，失败返回后等待其计划写入时间，验证没有遗留进程写出标记文件，再验证正常调用恢复。`python3 -B development/check.py --timeout 900 -- test --release -p luna --lib host::tools` 退出 0，17 项通过（`/tmp/luna-tools-l45dozo6/result.json`），包含既有 LLVM 构建、原生输出、超时和启动恢复检查。格式化通过（`/tmp/luna-tools-7clm3_a_/result.json`），git diff --check 通过；工作目录 `/home/fin/luna`，dev 未提交。

同时核对默认值边界：正式函数文档要求默认值有限、稳定，禁止其中词法 return；handler 中的 return 也在前端被拒绝（`/tmp/luna-tools-1uhf396h/result.json`）。未把后端默认帧的拒绝分支当作放宽语言语义的依据。

## Handler 参数、退出路径与析构验收（2026-10-25）

新增可运行示例 examples/host-handler-parameter-control.fl，已注册到 host_examples.py。请求携带两个独立指针；handler 保存旧值并翻转目标，随后普通完成、显式 resume 或词法 return。Watch 析构检查目标已写回且尚未清理，再设置清理标志。八组布尔输入区分三种退出行为，返回值、目标状态和清理标志共同验证求值／清理次数。

原生回归同时测试直接请求和跨函数请求。正例执行全部八组输入；“初始已清理”和“省略写回”的错误版本分别固定到三条退出路径，要求受检解释失败、LLVM O0/O3 构建成功而执行失败且不输出结果。14 个程序变体共 28 次原生执行通过，另有对应受检解释检查。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host native_handler_parameter_control_paths` 退出 0（`/tmp/luna-tools-p2qunuam/result.json`）。本轮没有改动资源准入或效应 ABI；这是对上一轮实现的扩大验收。

`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py handler-parameter-control --release-driver --compare-optimization` 退出 0，O0/O3 通过（`/tmp/luna-tools-xsnejw4j/result.json`，产物 `/tmp/luna-host-examples-s23a4_2l/`）。示例 runner 的 8 项 Python 测试通过（`/tmp/luna-tools-1bdeimxe/result.json`）；格式化通过（`/tmp/luna-tools-idps2umx/result.json`），git diff --check 通过。工作目录 `/home/fin/luna`，dev 未提交。

## 请求参数访问与 handler 词法数据返回（2026-10-25）

handler 通过请求参数访问指针后向安装者返回 bool 等无指针数据，原先因私有内存分析拒绝所有 lexical return 而失去参数访问摘要。复现 host-handler-parameter-scalar-return.fl 在修复前的受检解释执行报 unknown memory target（`/tmp/luna-tools-yqb9s4ax/result.json`）。内存分析副本现在保留返回表达式的求值／访问，以无值退出结束该路径；不把词法结果合并到 operation 的普通返回。源码、资源清理计划和 LLVM 控制流仍使用原 return。带指针或代码的词法结果仍要求单独的返回来源摘要，未放开请求参数指针返回。

定向 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host handler_parameter_` 两项通过（`/tmp/luna-tools-jojrolbv/result.json`），含直接／跨函数请求、返回表达式写回及求值一次，以及指针跨域返回拒绝。新增资源摘要测试检查 operation 返回 u8、词法返回 bool 的区分、读写访问、低预算失败后恢复。扩大检查遭遇配额：资源组日志不完整（`/tmp/luna-tools-xqtc6fnk/`）；host 组底层退出 0 但 stdout 丢失（`/tmp/luna-tools-mic9mezh/result.json`，evidence-error），均不计为通过。缓存已按授权清理。最终 host handler 46 项通过（`/tmp/luna-tools-tatbvchl/result.json`）；该轮新增资源夹具的 `.*=` 连写导致解析失败，随后补正空格和显式 builtin Byte 别名。资源 handler 组最终 5 项通过（`/tmp/luna-tools-t900yhho/result.json`），其中新增用例覆盖两种 operation 结果类型、两种返回表达式及预算恢复。格式检查通过（`/tmp/luna-tools-3prjepkp/result.json`），git diff --check 通过。工作目录 `/home/fin/luna`，dev 未提交。

## 原生比较的继承输出与工具启动（2026-10-25）

run_test_ir 现在在主进程退出后等到两条输出流 EOF，沿用原始执行截止时间；只看退出状态的 API 仍允许有界保存后的截断，精确输出比较仍拒绝截断。新增 LLVM fork 用例让主进程先退出，后代延迟写入两条流；验证完整内容、静默写端超时、延迟超限拒绝及恢复。该测试是执行器验收，不代表 Flurry 已提供进程 API。

首次工具链组运行中新增用例通过，但既有 artifact 检查出现 ETXTBSY（`/tmp/luna-tools-5lxryk7_/result.json`，15 通过、1 失败）。LLVM 工具启动现共用原生启动的有界 ETXTBSY 重试，只重试未成功 spawn 的情况；上限取 1 秒和工具 timeout 的较小值。新增持有可执行文件写描述符后释放的实际调用验证，启动后的 exit 7 正常报告失败，不重执行。

工作目录 `/home/fin/luna`，最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --lib host::tools`：16 项通过（`/tmp/luna-tools-oih2gb9_/result.json`）。格式化退出 0（`/tmp/luna-tools-a_8fdss9/result.json`），git diff --check 通过。没有重跑整个 host 示例目录；dev 未提交。

本轮还核对了 handler 请求参数的词法指针返回缺口：现有 HandlerReturnValue.origins 是安装者的具体 Place，而请求参数借用使用私有虚拟根；direct_handler_accesses 也不会把返回指针发布为参数访问证据。后续必须增加请求参数相对的返回来源并在请求现场替换，连同跨调用、枚举条件及清理后存活性检查一并完成。不能直接删除 check_handler_return_layout/check_handler_return_origins 的保护；现有 host-handler-parameter-return.fl 的拒绝仍是实现限制。

## LLVM 工具延迟输出完整性（2026-10-25）

工具主进程退出后，collect_tool_tail 原先把一次 drain 没有增长当成成功结束，可能遗漏继承描述符的子进程稍后输出的 LLVM IR。Capture 现在记录实际 EOF；只有 stdout 与 stderr 都关闭才完成收集，仍共用原调用截止时间。stdout 超限继续拒绝，stderr 仍有独立诊断保存上限；等待期间有短暂休眠，不忙轮询。该改动不扩展语言语义，也不改变原生程序的退出状态协议。

新增回归覆盖主进程退出后的延迟 stdout、仅 stderr 保持打开时超时、延迟输出突破大小上限以及随后正常调用恢复。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test --release -p luna --lib host::tools`：15 项通过（`/tmp/luna-tools-a3320vsw/result.json`）。首次误用 host::tools::tests 过滤器匹配零项，不计为验证证据。

`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py arithmetic generic-integers --release-driver --compare-optimization` 退出 0，两个示例 O0/O3 四项通过（`/tmp/luna-tools-pi5o0ah6/result.json`，产物 `/tmp/luna-host-examples-txm_kwqw/`）。格式化通过（`/tmp/luna-tools-xsltphbu/result.json`），git diff --check 通过；dev 未提交。

## 当前原生入口确认（2026-10-25）

在 `/home/fin/luna` 执行 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py arithmetic string generic-integers --release-driver --compare-optimization`，退出 0。三个示例各在 LLVM O0/O3 编译并执行，分别得到 `(unsigned-integer 42)`、`(string "你好，Flurry！")`、`(bool true)`。报告 `/tmp/luna-tools-wlhlisqc/result.json`，独立构建／执行证据及产物 `/tmp/luna-host-examples-1qw92dr8/`。这次只重跑三个入口，不充作整个示例目录重跑证据。

最新结构校验接入后 host 611、CMTT 523、真实库协议 11 项全部通过（`/tmp/luna-tools-vdsdh3qk/result.json`）。语义前端状态见 frontend-checkpoint.md；完整 sysroot 的 panic、String、格式化、分配与 I/O 缺口仍保留，不能把选定原生示例通过视为完整标准库可用。dev 工作区未提交。

## 按包清理覆盖 release profile（2026-10-25）

实际配额恢复中，`development/cargo.py clean -p luna -p elaboration` 删除 0 文件，而同命令增加 `--release` 删除 73 文件、约 430 MiB。Runner.clean 的自动按包清理现在执行默认与 release 两种 profile，均限定同一受管 target／指定包；全 target 清理仍只执行一次。保留共享锁、失败传播与最多一次配额重试，没有清理 registry 或源码。cargo wrapper 测试通过（`/tmp/luna-tools-p0eg2gs8/result.json`），host driver 测试通过（`/tmp/luna-tools-ff2mntol/result.json`）；本次前端检查在释放空间后通过，详见 frontend-checkpoint.md。工作目录 `/home/fin/luna`；git diff --check 通过；dev 未提交。

## 真实库 release 集成入口（2026-10-24）

library_check.py 新增 `--release`，对 test/run 全部阶段统一使用 release profile，并保留 shared Cargo wrapper、独立日志、失败后继续及总失败状态。修正 native-math 的过期计数：fixture 已有 6 项测试，旧脚本仍要求 4 项。新增四项 Python 回归覆盖两种 profile、失败后继续、进程成功但返回值错误，以及数学测试数量不足／失败数非零；全部通过（`/tmp/luna-tools-5nzxglbg/result.json`）。library/README.md 更新可直接使用的命令。

在 `/home/fin/luna` 执行 `python3 -B development/command.py --timeout 1800 -- python3 -B development/library_check.py --release --timeout 900`，最终整体退出 1（`/tmp/luna-tools-0m4uv7vi/result.json`），汇总 `/tmp/luna-library-p270d731/summary.json`：library_protocols 11 项、repository_sysroot manifest 1 项、原生 resources 4 项、comparison 6 项、math 6 项通过；完整 sysroot smoke 名称解析失败。仍缺 panic、String、格式化 Any、分配／集合与 I/O builtin 入口。实际执行真实库文件，没有用替身修正预期或隐藏 sysroot 失败。此前首次运行因加载了旧 math=4 计数也将数学阶段误报失败，最终运行已消除此脚本错误。git diff --check 通过；dev 未提交。

## 差分闭包后的重复传播（2026-10-24）

propagate_relations 不再仅因 DifferenceBounds 的闭包收紧区间就重跑整个循环：图已经计算完整传递闭包，其自身结论不能再加强同一图。后续多项式行发生任何收紧仍触发下一轮，继续保留共享预算、矛盾判断和版本检查。新增 a <= b <= c <= x*x 回归验证非线性收紧向差分链反馈、零预算失败后恢复，以及进一步收紧 x 后再次求解。

相同 entry-polynomials 源码的逐函数测量中，from_equal 从 89,846 降至 89,110 步，另外六个函数保持原数值；不声称整体编译时间改善，也未取消示例的显式预算。临时测量测试已移除。`python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib verification`：92 项通过（`/tmp/luna-tools-vld3rhub/result.json`）。同一 wrapper 执行 luna 的 resources、snapshot_contracts 与临时成本测量：357 + 2 + 1 项通过（`/tmp/luna-tools-u016f416/result.json`）。新增反馈测试独立通过（`/tmp/luna-tools-keqxw_nt/result.json`）。

`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py entry-polynomials --release-driver --compare-optimization` 退出 0，O0/O3 均通过（`/tmp/luna-tools-lqvs58hm/result.json`）。工作目录 `/home/fin/luna`；格式化退出 0（`/tmp/luna-tools-jjlp4rg3/result.json`），git diff --check 通过；dev 未提交。

## 契约预算测量与未保留的优化（2026-10-24）

针对 entry-polynomials 的显式预算做逐函数测量。相同源码、每函数初始 1,000,000 步，flow::analyze_in_package 的消耗分别为 square 21,412、cube 28,342、square_relay 50,192、cube_relay 59,912、run 61,019、from_sample 77,540、from_equal 89,846（`/tmp/luna-tools-wwo8xqay/result.json`）。这是独立流分析成本，不是整条 cleanup pipeline 的精确分项；残余契约检查共用总 fuel，不能据此将默认 100,000 解释为每函数额度。检查源码确认残余验证只在最终 footprint 稳定后执行。

尝试在 freeze_expression 中跳过完全 immutable 表达式的等式扫描，没有带来收益：square/cube 分别增加 3/4 步，其余相同。因此已撤销这一候选优化并移除临时测量测试，未改变生产预算或证明规则。候选版本验证为 resources 357、snapshot_contracts 2 项通过（`/tmp/luna-tools-htilukdl/result.json`）；该报告是测量实验结果，不充作另一轮最终实现验收。最终实现恢复此前已验证版本，示例继续显式使用 1,000,000 fuel。后续性能工作应测量关系传播／事实复制的实际分项，而非继续猜测这个快路径。工作目录 `/home/fin/luna`；dev 未提交。

## 入口快照的聚合与多层调用验收（2026-10-24）

snapshot_contracts 扩展平方／立方关系到多层别名、聚合输入字段、聚合返回字段及三参数等式链；每个合法程序解释得到 true，单向不等式替代等式链、调用前覆盖标量／字段的版本必须拒绝。没有因这些案例修改求解规则。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test snapshot_contracts` 退出 0，两项测试通过（`/tmp/luna-tools-7he8vo4c/result.json`）。

host-entry-polynomials 示例加入 Sample 字段与等式链调用。名义存储启用 paired Copy/Drop 检查，Sample 显式声明 Copy 以允许入口快照；默认预算下资源准入返回 BudgetExhausted，目录为这个示例显式设置 `--fuel 1000000`，未提高生产默认预算。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py entry-polynomials --release-driver --compare-optimization` 最终退出 0，O0/O3 均通过（`/tmp/luna-tools-dcataqvq/result.json`，产物 `/tmp/luna-host-examples-u1gda8q4`）。runner 8 项测试通过（`/tmp/luna-tools-hw4kb_6_/result.json`）。工作目录 `/home/fin/luna`；Rust 格式化退出 0（`/tmp/luna-tools-ui4oarga/result.json`），git diff --check 通过；未重复全量测试，dev 未提交。

## 跨调用非线性后置条件的入口快照（2026-10-24）

square_relay 的最后一项资源失败已解除。最小复现中 `result == value * value` 能证明，而 `result == (value * value)'old` 失败：callee 快照虽然与多个 caller 观察值相等，未映射身份的传播先选了可变参数，导致二／三次关系未使用 caller 的入口快照。connect_call_observations 现在先从已映射的 immutable entry 值传播双向等式，再补其余映射；显式输入／结果映射不被替换，版本与来源检查保留。只有两个方向都成立的等式能参与，不把同范围或单向界当作相等。此修改属于程序验证层，没有使用 PTC，也没有提高多项式次数上限。

新增 snapshot_contracts 回归覆盖平方／立方，直接返回与调用后将参数改为 0 再返回；实际实参换成 0、却仍声称原入口多项式关系的版本必须拒绝。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test snapshot_contracts --test resources --no-fail-fast` 退出 0，resources 357/357、新组 1 项（8 个正反组合）通过（`/tmp/luna-tools-e0r4ttkj/result.json`）。此前四项资源失败均已恢复。本轮没有重跑整个 host 测试组。

新增 `development/examples/host-entry-polynomials.fl` 并注册示例目录。平方与立方 callee、relay 均改写参数，通过零值及上界输入验证返回值。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py entry-polynomials --release-driver --compare-optimization` 退出 0，LLVM O0/O3 均输出 `(bool true)`（`/tmp/luna-tools-h2fpx9ps/result.json`），产物 `/tmp/luna-host-examples-5fu3iyor`。示例目录现有 45 项，仅新增项在本轮执行；runner 的 8 项测试通过（`/tmp/luna-tools-f4b726yi/result.json`）。工作目录 `/home/fin/luna`；格式化退出 0（`/tmp/luna-tools-360kxpu5/result.json`），git diff --check 通过；dev 未提交。

## 验证语句 AST 布局与类型位置阶段（2026-10-24）

修复 predicate statement 的 AST 布局：parser 原先对 axiom/requires/ensures/asserts/assumes/invariant/decreases 使用 unary builder，但 NodeType 声明 `[label, predicate]`。通用作用域扫描会将后续 arena 存储当作第二子节点，导致与源码长度有关的重复扫描；实际表现之一是 TupleElements<T> 函数体的 T 被误报未解析。语句现在显式写入空 label 与 predicate，assumes lowering 同步读取第二槽；未引入新的标签语法或改变 Tuple 身份。新增解析测试覆盖七种语句、相邻节点及缺失谓词拒绝。测试驱动初稿漏处理分号，已修正；最终 2 项通过（`/tmp/luna-tools-dzy0p4g1/result.json`）。

第一次完整检查（`/tmp/luna-tools-yluagipm/result.json`）恢复了 Tuple 指针资源回归，同时发现前一轮阶段修复遗漏 `do {Make()}.Inner<bool>.Payload`：类型位置的 comptime call 被错误送往 runtime Splice。type_expr 现在为类型成员查找明确设置 required comptime，并在成功／失败返回前恢复阶段；普通值位置的投影仍保持调用方阶段。没有放宽 runtime 输入限制或 normal-form 身份规则。

最终在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test host --test resources --no-fail-fast`：host 611/611 通过；resources 356/357 通过，整体退出 101（`/tmp/luna-tools-qhp8f03g/result.json`）。原 Tuple 指针跨调用、两项计算接收者资源回归全部通过。唯一资源失败仍为 resource_signature_contracts_cover_entry_snapshots_and_each_return_path，program-entry-snapshots.fl 的 square_relay 跨调用 `(value * value)'old` 后置条件未证明；不能将资源全组计为通过。Rust 格式化退出 0（`/tmp/luna-tools-fv97yryf/result.json`），git diff --check 通过。dev 未提交。

## 运行时计算接收者的阶段与投影（2026-10-24）

字段域探测不再把 do/if/when/match 接收者强行切换到 required comptime；推测检查沿用调用方阶段，并回滚临时检查状态。结果确为 Type 时，取得类型身份仍走原有 required evaluation。没有展开 normal-form 头，也没有吞掉任意检查错误。

新增 host 用例覆盖四种计算接收者的普通字段及 vars 字段、接收者只求值一次、条件指针的写回目标，以及块内指针别名跨调用。受检解释结果与 LLVM O0/O3 对照通过。现有 computed Type 正例、访问／阶段拒绝例一起通过：`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host computed_`，12 项，退出 0（`/tmp/luna-tools-3e17dt5y/result.json`）。

此前两个资源失败已恢复：同一入口分别运行 `--test resources memory_summary_pointer_blocks` 与 `--test resources temporary_projection_origins_preserve_nested_and_match_result_dependencies`，均退出 0（`/tmp/luna-tools-19z1oa9_/result.json`、`/tmp/luna-tools-vzg5w5vu/result.json`）。未重跑资源全组；此前 Tuple 参数 T 解析和后置条件证明失败仍待处理。工作目录 `/home/fin/luna`；格式化退出 0（`/tmp/luna-tools-pa185k_b/result.json`），git diff --check 通过；dev 未提交。

## 示例目录集成验收与持续诊断（2026-10-24）

host_examples.py 新增可选 `--keep-going`，某个构建／执行失败后继续其余示例及优化档；每项保持独立产物和日志，成功不能覆盖已有失败，最终退出码由整批结果决定。默认快速失败行为不变。新增测试以首项构建失败、后续三项成功验证两种模式，确认失败项不会伪造执行报告；8 项脚本测试通过（`/tmp/luna-tools-6d71yvj3/result.json`）。

在 `/home/fin/luna` 执行 `python3 -B development/command.py --timeout 900 -- python3 -B development/host_examples.py --release-driver --compare-optimization --keep-going`，退出 0：当前目录 44 个示例 × O0/O3 = 88 项全部通过（`/tmp/luna-tools-1b_u4l5y/result.json`）。逐项记录和可执行产物位于 `/tmp/luna-host-examples-e0skys1d/summary.json` 及对应子目录，驱动私有副本在完成后释放。包含本阶段新增的所有权选择、返回闭包、枚举清理特化、动态普通函数与递归分派，以及现有数学、比较、资源和效应示例。这是 host 示例目录的集成证据，不替代尚未完成语言能力的验收。dev 未提交。


## 特化调用的失败路径顺序验收（2026-10-24）

新增 `specialized_call_failures_preserve_prepared_and_callee_cleanup_order`，从实际源码取得 checked IIR 与清理报告，同时验证普通函数和闭包先接收空枚举、再接收拥有 Token 的枚举。参数构造、callee 局部、match arm 局部和 caller 外层对象的析构器将各自编号追加到共享计数。正常返回或主体内失败得到 34231；后续实参失败得到 321，证明未进入第二次 callee，同时先清理已构造实参再清理 caller。通过只替换开发用 main 适配器，读取生成函数失败返回后的实际内存和 flurry_failed；没有改函数体、清理计划或失败传播。解释器同时检查返回／拒绝类别和 5／3 次实际析构调用。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --lib specialized_call_failures` 退出 0（`/tmp/luna-tools-qzvfktw_/result.json`），8 种组合均检查解释行为和 LLVM O0/O3，合计 16 次原生执行。新增验证通过，未发现需要放宽或更改实现的行为。Rust 已格式化；dev 未提交。


## 普通函数的按调用清理特化（2026-10-24）

普通函数现在与闭包一样按已发布调用上下文生成机器体。函数值保留 canonical 代码身份；直接／间接调用在求值实参前保存 callee，并按当前资源帧及 call source 选择本次 `_scope_` 代码。递归边只使用 validate_alias 验证过的祖先 scope。普通函数内部再调用闭包时，内部路由使用该特化机器体自己的上下文，不再复用首次调用的内部计划。原 named_bodies_specialize_nested_closure_dispatch 限制已转为受检解释及 LLVM O0/O3 正例，另有动态普通函数身份和递归枚举资源参数回归。

canonical 机器体直接承载根上下文（没有根时承载第一份已取得上下文），只为其余上下文生成额外机器体。初稿为唯一上下文重复生成函数，导致三项静态析构调用数检查失败（606/609 host 通过，`/tmp/luna-tools-sr4vk8h4/result.json`）；已消除重复，没有改测试预期。共享计划的比较覆盖嵌套普通／闭包调用；不同递归目标不因局部清理表相同而被当作等价。析构器仍用 canonical 调用，因此要求完整计划可安全复用；缺失生产证据仍由 checked body emission 拒绝。底层独立 emitter 的手写机器码桩不重复要求生产报告。

工作目录 `/home/fin/luna`。最终 `python3 -B development/check.py --timeout 900 -- test --release -p luna --lib --test comparison_resources --test host --no-fail-fast` 退出 0：69 + 15 + 609 = 693 项通过（`/tmp/luna-tools-ziqkk6se/result.json`）。新增共享计划回归包括内部普通调用差异、零预算失败和恢复。动态函数值的重复调用用例显式声明 Copy；真实库示例同时声明 Once、Clone、Copy，不引入隐式函数值复制。格式检查（`/tmp/luna-tools-osbis2_1/result.json`）及 git diff --check 通过。

新增 `development/examples/host-owned-dispatch.fl`，通过真实 resources 库组合动态函数选择、递归调用和 none/some 的清理差异。最终 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py owned-dispatch --release-driver --compare-optimization` 退出 0，O0/O3 均通过（`/tmp/luna-tools-sqn65o71/result.json`），产物 `/tmp/luna-host-examples-xfs4ui86`。对同一调用点／同一代码身份仍存在互不兼容证据时继续拒绝歧义；本轮没有改动此前记录的四项语义前端失败。dev 工作区未提交。


## LLVM 闭包调用清理特化（2026-10-24）

上一节记录的直接闭包调用限制已解除：同一闭包先接收空枚举、再接收含资源的枚举，现在可以分别使用两次调用已验证的清理计划。LLVM 为每个 concrete closure 与发布的调用上下文生成独立机器体。闭包值保留原代码身份和原环境；调用点根据代码身份将本次调用路由到相应 `_scope_` 机器体，不修改存储中的代码指针，不复制捕获，也不按参数值猜清理规则。实例与符号编号仍采用确定的图／schedule 顺序，遍历和上下文比较受 fuel 限制。

同一调用点若同一代码身份对应互不兼容的清理计划，仍拒绝歧义。共享普通函数机器体时，same_frame 现在递归比较内部闭包分派及其内联上下文；两份外层局部清理表相同并不足以授权复用。一般命名函数的按调用特化尚未实现，`shared_named_body_cannot_reuse_different_nested_closure_dispatch` 的受检解释通过，但原生明确拒绝 distinct per-call cleanup bodies。这项限制回归不能计为该能力已实现。

comparison_resources 中原来的 enum 拒绝例已改为解释／LLVM O0/O3 正例；新增运行时选择两个闭包身份、各自连续处理 none/some 的正例。最终 13 项通过（`/tmp/luna-tools-yu0fq7td/result.json`）。外层复用测试初稿误把非 Copy 闭包按值传两次，继而试用了未支持的非 Copy 指针读取，分别被资源／前端正确拒绝；最终用工厂创建两份同代码身份的合法闭包值，仅测试共享命名机器体的分派边界，没有添加隐式 Copy。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test --release -p luna --test host closure --lib` 退出 0，原生闭包组 37 项通过（`/tmp/luna-tools-kbv19rmd/result.json`）。新示例 `development/examples/host-owned-variants.fl` 使用真实 resources 库：`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py owned-variants --release-driver --compare-optimization` 退出 0，O0/O3 均通过（`/tmp/luna-tools-gqsp2jjr/result.json`），产物 `/tmp/luna-host-examples-96u_v1fj`。最终 `test --release -p luna --lib` 68 项通过（`/tmp/luna-tools-ytdyx5vr/result.json`）；格式检查（`/tmp/luna-tools-k0pd5rer/result.json`）和 git diff --check 通过。dev 未提交。


## 闭包所有权参数的指针字段与待完成的清理特化（2026-10-24）

闭包形成阶段现在按参数的完整已知布局建立非根指针字段的条件借用前提，并记录参数槽、输入字段路径及 pointee。枚举字段仍保留布局自带的 variant guards。此前按值接收 Watch 的闭包在读 dropped 字段时缺少来源（`/tmp/luna-tools-5f7ys6am/result.json`）；实际调用仍必须将条件足迹代入真实目标，条件来源不能使逃出作用域的指针有效。

comparison_resources 新增返回闭包重复调用／未调用销毁、按值所有权参数、悬空 pointee 拒绝和动态枚举参数。11 项通过（`/tmp/luna-tools-jvb0j4bo/result.json`）；其中一个是明确的原生限制回归，不是功能通过：同一闭包先调用 Choice.none 再调用 Choice.some(Watch) 的受检解释通过，但 LLVM 的共享闭包体要求各调用清理计划一致，当前仍拒绝 `host closure requires distinct per-call cleanup bodies`。后端现在保留这条真实诊断，不再统一伪装成析构器未实现。单一调用点、运行时选择两个 variant 的版本则已通过受检解释与 O0/O3。后续须实现按调用清理上下文生成闭包机器体，或建立经验证的统一动态清理计划；不能仅删除 same_frame 检查。

资源全组同时执行：353 通过、4 失败（同上报告）。失败均在该测试辅助入口生成 NIR 前：memory_summary_pointer_blocks_keep_parameter_paths_and_check_side_effects 与 temporary_projection_origins_preserve_nested_and_match_result_dependencies 的括号 runtime 表达式被要求 comptime；tuple_pointer_slots_cross_calls_without_losing_snapshot_or_write_identity 在 HIR lowering 报 T 未解析；resource_signature_contracts_cover_entry_snapshots_and_each_return_path 的 program-entry-snapshots/program-postconditions 输入在后置条件证明失败。尚未用历史基线复核引入时间，不将全组计为通过，也未为绕过错误修改语义或 fixture。

owned-callback 示例已改为独立 make 函数返回含所有权捕获的环境，再交给 choose_by。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py owned-callback --release-driver --compare-optimization` 退出 0、O0/O3 均通过（`/tmp/luna-tools-rxtlk7vc/result.json`），产物 `/tmp/luna-host-examples-ubkzswr3`。工作目录 `/home/fin/luna`；Rust 格式化和 git diff --check 通过；dev 未提交。


## 闭包预检查的借用来源与资源比较器（2026-10-24）

带资源捕获的比较器在读取 `a.dropped.*.*` 时，原被闭包构造阶段的独立主体检查误报为改变调用方资源（`/tmp/luna-tools-u2ykqd6x/result.json`）。该阶段已经建立条件借用存储，却未将参数和捕获字段的 BorrowedRoot 来源交给观察检查；读取间接指针时新增的条件来源因缺乏证据而被拒绝。

现在构造阶段与实际调用阶段一样，记录根指针参数、根指针捕获和聚合捕获字段的参数槽及输入字段路径；来源仅属于该闭包的独立分析报告，不成为拥有者或清理根。观察检查仍校验已有候选、条件和资源状态不被改变，实际调用仍按真实指针来源验证足迹。没有用合成存储身份跳过有效性检查。

`comparison_resources` 新增运行时选择函数指针的泛型 choose_by，以及带 Watch 捕获的闭包比较器。后者在调用期间借用两个所有权参数，返回选中对象，比较器环境和未选对象在被调函数结束时析构，返回对象在调用方结束时析构；两种比较方向均通过受检解释与 LLVM O0/O3。独立 `development/examples/host-owned-callback.fl` 使用真实 resources 库，已加入示例目录。

工作目录 `/home/fin/luna`，经 `development/check.py --timeout 900 -- test --release -p luna`：`--test comparison_resources` 六项通过（`/tmp/luna-tools-i_5y8y1w/result.json`）；`--test resources closure` 33 项通过（`/tmp/luna-tools-9o8vf5bx/result.json`）；`--test resources guard` 15 项通过（`/tmp/luna-tools-2o7aavow/result.json`），包括失败路径、重复责任和失效候选拒绝。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py owned-callback --release-driver --compare-optimization` 退出 0（`/tmp/luna-tools-y_9oguxd/result.json`），产物 `/tmp/luna-host-examples-e71317bl`。格式检查（`/tmp/luna-tools-8pa2e5k9/result.json`）和 diff 检查通过。dev 未提交。


## 借用比较与泛型所有权选择（2026-10-24）

新增可直接运行的 `development/examples/host-owned-selection.fl`，通过真实 comparison/resources 包实现 `Ord for *Candidate`。泛型 `choose(a:T,b:T)->T where T, requires *T :- Ord` 借用比较两个对象，将胜者的所有权返回调用方；败者在 choose 内析构，胜者在调用方作用域结束时析构。四种布尔键组合涵盖两侧获选及相等时保留左值。析构器的运行时检查防止重复清理；比较时检查两者都未析构。不要求 T 为 Copy，不引入隐式借用比较。

`compiler/luna/tests/comparison_resources.rs` 增加对应受检解释执行和 LLVM O0/O3 回归；重复消费拒绝测试收紧到值不可用诊断，不能由无关的内存契约失败代替。该文件四项通过（`/tmp/luna-tools-v6lvtz3s/result.json`），随后将 choose 泛型化后的定向测试通过（`/tmp/luna-tools-k8g9vuzo/result.json`）。最终独立示例命令 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py owned-selection --release-driver --compare-optimization` 退出 0，O0/O3 均输出预期 bool true（`/tmp/luna-tools-r_heuzqr/result.json`），产物 `/tmp/luna-host-examples-fom7zpv1`。工作目录 `/home/fin/luna`，dev 未提交。此证据覆盖已知布局的借用视图与所有权交付，不代表任意容器或所有内存规格已实现。


## 比较协议与带指针字段的所有权参数（2026-10-24）

比较方法的资源分析现在复用完整参数布局检查：含指针字段的结构体也进入方法内存契约路径，不再只识别根参数为指针的情形。内存摘要按求值顺序处理参数；参数确定提前退出时，保留已经观察的效果并停止分析后续参数及被调用函数，默认参数的临时环境仍恢复。没有放宽内存契约、Copy 或 Drop 准入。

新增 `compiler/luna/tests/comparison_resources.rs`，对六种比较检查两个所有权对象在方法执行后各析构一次、左右参数提前退出的清理边界、消费后再次使用被拒绝；使用实际 comparison 库默认方法，正常与退出路径均对照受检解释执行和 LLVM O0/O3。工作目录 `/home/fin/luna`；`python3 -B development/check.py --timeout 900 -- test --release -p luna --test comparison_resources` 三项通过，报告 `/tmp/luna-tools-b5prnn2b/result.json`。首次编译遇配额错误，受管入口清理可重建缓存后自动重试成功。 定向复核 `--test comparison_resources --test equality_protocol --test ordering_protocol --test method_memory_contracts` 共 21 项通过（`/tmp/luna-tools-aowr0uyc/result.json`）；格式检查（`/tmp/luna-tools-rw1m694b/result.json`）和 `git diff --check` 通过。改动保留 dev 工作区，未提交。


## 泛型平台整数比较与目标隔离（2026-10-24）

usize／isize 原已有具体标量比较，但泛型相等／有序比较的有限域清单遗漏了它们。现将两个平台整数身份加入共享 Scalar 比较的域，并让 P4 类型案例保留这些身份。行为分类仅判断读值／复制／比较所需的行为，不选择位宽；usize 不因 64 位目标就成为 u64，isize 同理。没有把平台整数加入当前固定整数算术证明的有限域枚举。

新增声明回归检查 Package.target 仍为 None、函数形参仍为 Usize；无目标时尝试直接调用不得成功，同一个直接调用路径显式提供 32／64 位目标后均返回 true。另一回归检查两种位宽各自的 signed 最小／最大值、unsigned 最大值和六种比较方向，并混用同一个泛型函数的 i64／u64 实例。64 位专属字面量在 32 位目标拒绝。解释器检查两种目标；仅在目标与实际 LLVM 工具链布局一致时执行 O0/O3，未宣称已在当前宿主原生运行另一位宽。

`generic-order` 示例已加入 usize／isize 的界限选择，O0/O3 均通过（`/tmp/luna-tools-l2z1o810/result.json`），命令仍为 `python3 -B development/host_examples.py generic-order --compare-optimization`。

工作目录 `/home/fin/luna`。定向两项比较测试通过（`/tmp/luna-tools-bj94c81i/result.json`）。合并 host 与 target_integers 的检查遇链接配额且日志保存失败（`/tmp/luna-tools-khmmvmcb`），不计为通过；受管清理可重建产物后拆组。补强直接调用对照后，`python3 -B development/check.py --timeout 900 -- test -p luna --test target_integers --no-fail-fast` 的 19 项全部通过（`/tmp/luna-tools-21tuw1pi/result.json`）。修改保留在 dev，未提交。

`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib program::purity::type_domains -- --nocapture` 的 4 项通过（`/tmp/luna-tools-ebd95bry/result.json`），覆盖平台类型身份、预算失败／恢复及既有条件等式边界。格式检查通过（`/tmp/luna-tools-1gj4xch_/result.json`）。单独 host 链接仍碰到配额（`/tmp/luna-tools-dxkzzyfc`），因此进一步使用受管 `cargo.py clean` 回收整个 target 的可重建产物后再检查；该次失败不计为通过。

最终限制：完整清理 target 后，host harness 仍在链接阶段触及磁盘配额（`/tmp/luna-tools-j4m_ajb9`），日志保存也失败，因此本轮不能报告 host 全组通过。没有继续重复相同重建；再次清理 luna／elaboration 的可重建产物以恢复工作空间。当前有效证据是 19 项 target_integers、4 项类型案例单元测试、generic-order 的 O0/O3，以及格式和 diff 检查。没有改动源码验证要求或伪造全量结果。

## 比较表达式的提前退出（2026-10-24）

`do {return true;} == value` 在无约束泛型函数中原被要求提供相等协议，其他有序比较有同类问题。两侧完成原有 elaboration／类型检查后，现在确认左侧 Never 就保留其控制流，不从不会求值的右侧虚构运算输入域，也不生成不会执行的比较或 not 节点。原失败报告 `/tmp/luna-tools-b1k7hr5c/result.json`；首次修复后的源码与 comptime 回归通过（`/tmp/luna-tools-ny6_7lm_/result.json`）。

右侧提前退出则不能丢弃左侧求值。已有相等节点继续保留左侧；当节点结果为 Never 时，!= 不再额外生成 not 包装。该包装曾使合法 pure 函数误失 P4 依据（`/tmp/luna-tools-glg1urm5/result.json`）。这里没有扩大右侧 Never 时的类型域准入，也没有将任意未知的左操作数假定为可比较。

新增回归覆盖 ==、!=、<、<=、>、>=；无约束泛型的 bool／i8 值与 f32／f64 pure 函数；普通执行与 comptime invocation；右侧未知名称和错误实参仍拒绝；左侧实际写入不能获得 pure 承诺。资源案例分别执行 return／break／continue，Watch 析构器通过运行时判定确认恰好清理一次，右侧写入不得发生。右侧退出的另一组案例则确认左侧写入必须发生。解释执行与 LLVM O0/O3 对照执行。扩展左侧三项测试通过（`/tmp/luna-tools-984p4e1x/result.json`）。

工作目录 `/home/fin/luna`，改动保留在 dev，未提交。

最终验证：`python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast`，605 项全部通过（`/tmp/luna-tools-tb3jr7rw/result.json`）。workspace 格式检查（`/tmp/luna-tools-c9lw3mq0/result.json`）及 `git diff --check` 通过。

## 联合标量域上的泛型比较（2026-10-24）

有序比较原先分别证明泛型域完全属于固定整数族或浮点族，遗漏了共用同一 NIR 标量比较操作的联合域。新增统一的 ordered scalar family 判定，包含十种固定宽度整数、f32、f64、char。每次调用仍要求两个操作数类型一致；具体实例保留 signed／unsigned、Unicode 标量顺序及 IEEE 无序行为。不根据调用猜类型域，不展开 normal-form 头，不将这种操作准入等同于 Ord 的全序承诺。bool／str／Symbol／Type 及任意自定义类型未在此加入原生排序域。

新增解释／LLVM O0/O3 回归逐一检查 13 个类型、signed 极值、u128 最大值、Unicode 字符、全部四种比较方向、实参顺序／次数和 NaN。对应源码同时包含类型联合约束的 pure 函数。非法类型域与 opaque Identity 头必须拒绝，原混合整数／浮点算术拒绝例保持。初稿将泛型调用直接放进 assumes，被现有 closed predicate 形状检查拒绝（`/tmp/luna-tools-uphigf7v/result.json`）；测试改为普通布尔结果逐项核对，没有放宽 predicate 准入。两项定向比较测试通过（`/tmp/luna-tools-tj0jg0sg/result.json`）。首次运行另遇链接配额和日志保存失败（`/tmp/luna-tools-hhhsjlwd`）；受管清理 luna／elaboration 可重建产物后重跑，未将该次失败计为通过。

`examples/host-generic-order.fl` 的同一 bound_if_ordered 函数供整数、浮点、字符调用。动态检查 lower <= upper，超出界限时返回界限值，无序的 value（NaN）保留自身；NaN 界限则不能通过检查。当前 P4 检查不接受该包含动态检查的接口；示例使用普通 fn，未放宽 P4 准入。失败报告及监控的效果还须单独核验，不能据此概括所有 runtime checker 的最终纯度。初稿 pure 声明被拒绝（`/tmp/luna-tools-hg9r1dkq/result.json`），改为普通函数后示例 O0/O3 均通过（`/tmp/luna-tools-cg7qe95i/result.json`）。新增回归还验证颠倒整数／字符上下界、NaN 上界在两种优化级别均运行失败。

```sh
python3 -B development/host_examples.py generic-order --compare-optimization
```

示例目录增至 38 项，本轮仅执行受影响示例。工作目录 `/home/fin/luna`，改动保留在 dev，未提交。

最终 host 检查：`python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast`，601 项通过（`/tmp/luna-tools-y9lj936b/result.json`）。示例运行器六项 Python 测试通过（`/tmp/luna-tools-25djgwby/result.json`）。workspace 格式化通过（`/tmp/luna-tools-l56zljgo/result.json`），`git diff --check` 通过。

## 混合浮点运算与整数循环验证（2026-10-24）

实际运行泛型 Newton 迭代发现：整数循环计数器令整个函数进入机器运算验证，但其中的浮点加减乘除／取负没有 flow 分支，导致 unsupported-construct。原失败 `/tmp/luna-tools-ap7ixcdo/result.json`。验证器现在依源顺序检查 FloatBinary 和 FloatNegate 的操作数，对其结果仅提供未知值描述，继续验证计数器范围、操作数调用和写入。没有将 IEEE 运算映射为精确多项式，也没有推断 NaN 下不成立的恒等式。

新增 `examples/host-generic-iteration.fl`，对 f32／f64 使用同一有界 Newton 迭代，覆盖浮点泛型默认参数、局部复合赋值、u8 计数器及结果的运行时精度判定。这是演示迭代的程序，不替换 math.sqrt intrinsic，也未承诺对所有输入的数值误差界。

```sh
python3 -B development/host_examples.py generic-iteration --compare-optimization
```

新源码回归还检查浮点操作数按顺序且仅求值一次，浮点除零保留 IEEE 行为；操作数内部的整数溢出／整数除零不能被忽略。更新 divisor 后再整数相除必须拒绝，不能沿用旧的非零依据。`value - value == 0.0`、`value + 0.0 == value` 的无条件后置规格不能因浮点表达式被遍历就获得证明。后者在 NaN 输入下并不成立。浮点数值规格仍需要单独的形式化支持。

工作目录 `/home/fin/luna`，验证证据：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast`：598 项通过（`/tmp/luna-tools-ley_z6cl/result.json`）。补充 divisor 写入失效案例后，同入口过滤 `float_operations_do_not_hide_integer_obligations_or_prove_exact_identities -- --nocapture` 再次通过（`/tmp/luna-tools-amubmmro/result.json`）。
- 通过 `development/command.py --timeout 600` 运行上述 generic-iteration 示例命令，O0/O3 均通过（`/tmp/luna-tools-17koyw91/result.json`）。目录增至 37 项，本轮没有全量重跑示例目录。
- 示例运行器六项 Python 测试通过（`/tmp/luna-tools-avq26nvi/result.json`）。改动留在 dev，未提交。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --test verification --no-fail-fast`：85 项通过（`/tmp/luna-tools-xfaqcs7i/result.json`）。本轮合计 683 项不同 Rust 测试与 6 项 Python 测试通过。workspace 格式检查通过（`/tmp/luna-tools-5rv78bfr/result.json`），`git diff --check` 通过。

## 一元运算操作数的提前退出（2026-10-24）

`-do {return value;}` 和 `not do {return value;}` 原先继续生成一元运算节点，令不存在的运算输入触发 Integer 阶段或 Copy／纯度检查。现在先正常检查操作数，确认其为 Never 后保留该表达式；运算不会执行，无须从 expected 类型或绑定协议中虚构输入域。既不注册 Never 的 Copy 协议，也不跳过操作数中的名称、返回类型和副作用检查。

新增源码正例包含无约束泛型的 bool／i8 返回（含 -128）、f32／f64 纯函数、bool 的 not、实参只求值一次，以及有／无 Neg、Not 协议绑定；同样的调用再用 comptime invocation 检查。拒绝例要求 pure 中的外部指针写入、未知名称、错误返回类型继续失败。资源回归分别在取负和 not 下执行 return、break、continue，使用带运行时判定的 Watch 析构器确认每条退出路径只清理一次，并比较解释执行和 LLVM O0/O3。

原故障报告 `/tmp/luna-tools-movjrxbu/result.json`。初版正例的外部指针调用缺少配对资源选项，后端正确拒绝（`/tmp/luna-tools-gspijml0/result.json`）；现将指针与析构放入明确启用 Copy/Drop 的资源案例，基本标量案例保留局部写入。补充 comptime 检查前，`python3 -B development/check.py --timeout 900 -- test -p luna --test host unary_nonreturning -- --nocapture` 的三项测试通过（`/tmp/luna-tools-mg55ylfo/result.json`）。

最终验证：工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast` 的 596 项全部通过（`/tmp/luna-tools-qrs7cxtc/result.json`），含新增 comptime invocation 检查。workspace 格式化通过（`/tmp/luna-tools-t0w36bhz/result.json`），`git diff --check` 通过。改动保留在 dev，未提交。

## 泛型浮点算术与 IEEE 行为（2026-10-24）

将已证明的 f32／f64 有限类型域接入原生加、减、乘、除、取负和有序比较。上下文浮点文字可在泛型运算中保留待定 formation；协议分派复用同一域判断，加载 Add／Sub／Mul／Div／Neg 不改变内建行为。没有放宽浮点余数的协议要求，也不将无约束、整数／浮点混合域或 normal-form Type 头当成浮点数。PTC 仅提供类型域依据，未参与数值程序规格。

新回归发现 P4 在 `a * do {return a;}` 上误要求 Never 结果具有 Copy 性质。标量二元操作现在区分无返回结果与可复制值，继续递归验证操作数行为；带外部指针写入的提前返回仍拒绝，没有跳过副作用检查或为 Never 注册 Copy 协议。首次故障 `/tmp/luna-tools-sij2e12d/result.json`；随后正例通过，新增拒绝例因 `p.*=true` 的词法边界失败（`/tmp/luna-tools-6_ff6eui/result.json`），修正为 `p.* = true`。

浮点测试覆盖 f32／f64、协议绑定和无绑定、实参源顺序／次数、早退、泛型文字、NaN 的四种无序比较、正负无穷、正负零以及 IEEE 乘加分步舍入。`4097.0 * 4097.0 - 16785408.0`（f32）及对应 f64 案例必须得到零，不能擅自以 fused multiply-add 取得一；没有添加 fast-math 标志。新增拒绝例还检查不同泛型输入域未证明相等、混合 bool 域、浮点 remainder 与 opaque Identity 头。

`examples/host-floating.fl` 已扩展成泛型插值、除法和分步乘加示例，两种浮点格式均有实际调用，保留 NaN／Infinity 检查。运行：

```sh
python3 -B development/host_examples.py floating --compare-optimization
```


工作目录 `/home/fin/luna`，最终验证：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast`：593 项通过（`/tmp/luna-tools-hz5_7z7u/result.json`）。
- 受管清理 luna 可重建产物后，`python3 -B development/check.py --timeout 900 -- test -p luna --test runtime_purity --test pure_callbacks --no-fail-fast`：64／80 项通过（`/tmp/luna-tools-qg7wve65/result.json`）。合计 737 项不同 Rust 测试通过。
- 通过 `development/command.py --timeout 600` 执行上述 floating 示例命令，O0/O3 均输出 `(bool true)`（`/tmp/luna-tools-9poi4isw/result.json`，各阶段产物位于 `/tmp/luna-host-examples-cjmigo_1`）。
- workspace 格式检查（`/tmp/luna-tools-mhkovd68/result.json`）及 `git diff --check` 通过。未运行全 workspace；改动留在 dev，未提交。

## 协议绑定下的泛型原生取负（2026-10-24）

源码协议分派原来只识别具体内建数值类型，而取负本身已支持 requires 证明的五种有符号整数域。两条路径不一致，导致绑定 Neg 后在合法泛型取负处误报方法实现不唯一。现在两者共用有符号域判定，不增加额外类型假设，也不展开 normal-form 头。自定义 Flag 的 Neg／Not 仍通过关联 Output 返回 bool；内建 i8 的取负、bool／BitVec 的 not 不被用户提供的同域协议方法替换。

新增端到端回归覆盖五种有符号位宽、负数／零／正数、实参只求值一次，以及无范围约束的未调用泛型定义仍拒绝。原故障报告 `/tmp/luna-tools-l2n55dfy/result.json`；测试初版遗漏名义 Flag 所需的 Copy/Drop 验证（`/tmp/luna-tools-u_pq5lg1/result.json`），补齐后又触及资源与残余契约共享预算（`/tmp/luna-tools-zsymkdl9/result.json`）。最终混合测试显式使用 `--fuel 1000000`；默认值未改，预算成本仍是后续工作。`python3 -B development/check.py --timeout 900 -- test -p luna --test host negation -- --nocapture` 三项通过（`/tmp/luna-tools-jqhi7f0r/result.json`），包含 LLVM O0/O3 和原有 bit pattern 回归。

`library/builtin/ops.fl` 的 Type 注解改为 `.Type'builtin`，消除对父模块导入的依赖，可以独立加载为协议包。generic-integers 示例现显式加载这个实际库文件并绑定七个算术协议，新增有合法范围约束的泛型取负；GCD、无符号差值与取负均保持可执行。通过 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py generic-integers --compare-optimization` 验证，O0/O3 均通过（`/tmp/luna-tools-nq3sy4sf/result.json`）。这个示例没有额外增大 fuel。

曾尝试直接加载完整 sysroot，实际失败于 panic、String 和 std 中未实现的占位入口（`/tmp/luna-tools-clxt_ot1/result.json`，具体诊断 `/tmp/luna-host-examples-xv84g73d/0-generic-integers-O0/build/stderr.log`）。独立协议包不代表完整 sysroot 可运行；未为满足示例伪造这些 API。工作目录均为 `/home/fin/luna`，改动保留在 dev，不提交。


最终验证：`python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast` 的 591 项通过（`/tmp/luna-tools-j6vi9n8l/result.json`）；`test -p luna --test library_protocols --test arithmetic_contexts --no-fail-fast` 的 11／10 项通过（同一受管入口，`/tmp/luna-tools-qqr1o9u7/result.json`）。共 612 项不同 Rust 测试通过。三组合并运行曾因磁盘配额导致日志保存失败（`/tmp/luna-tools-s5783bxm`），不计为通过；受管清理 luna 可重建产物后拆组重跑。示例工具六项 Python 测试通过（`/tmp/luna-tools-qmfjmuwh/result.json`），workspace 格式化通过（`/tmp/luna-tools-vubd2ifs/result.json`），`git diff --check` 通过。

## 泛型整数算法的原生执行（2026-10-24）

在 requires 证明类型属于十种固定宽度整数时，支持泛型加、减、乘及四种有序比较；除法与余数要求类型属于五种无符号整数。类型证明仅确定操作域，溢出、非零及函数契约仍由程序验证层检查。有符号除法／余数继续要求舍入协议，无约束或混入 bool 的域不能获得原生数值操作。

泛型上下文中的文字保留待定表示，在有限域的私有验证实例中完成 formation，覆盖函数体、前后置条件和默认实参；调用方已替换为具体类型的契约也进行带预算的 formation，才提供数值事实。越界文字拒绝，未确定类型的文字不伪造值证据。NIR 阶段检查允许操作数提前返回而产生 Never，但不能凭结果注解为 Never 绕过一致性检查。此处没有把所有待定文字函数都纳入机器操作验证，其他阶段形成规则仍需分别完善。

新增 `examples/host-generic-integers.fl`：同一泛型 GCD 支持 u32 与 u128（含最大值），泛型无符号差值通过分支条件证明减法安全。运行：

```sh
python3 -B development/host_examples.py generic-integers --compare-optimization
```

工作目录 `/home/fin/luna`，本轮验证：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test host generic_integer_arithmetic -- --nocapture`：两项通过（`/tmp/luna-tools-japm38z3/result.json`），覆盖十种位宽、极值、上下文文字、提前返回、实参顺序／次数、解释执行与 LLVM O0/O3，以及未调用定义的非法契约／文字拒绝。
- `test -p nir --test phases`：28 项通过（`/tmp/luna-tools-233akhjz/result.json`）。`test -p luna --test integer_formation --test arithmetic_contexts --test integer_contracts --no-fail-fast`：7／10／14 项通过（`/tmp/luna-tools-ki_zi11b/result.json`）。以上均使用 development/check.py 受管入口。
- 通过 `development/command.py --timeout 600` 运行上述示例命令：O0/O3 均返回 `(bool true)`（`/tmp/luna-tools-2wzazh2n/result.json`，构建与执行产物位于 `/tmp/luna-host-examples-iwwjpppd`）。示例目录增至 36 项；本轮未重跑全部目录。
- 示例运行器的六项 Python 测试通过（`/tmp/luna-tools-bg591aq6/result.json`）。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test elaboration --no-fail-fast`：589／449 项全部通过（`/tmp/luna-tools-dnq2_ta0/result.json`）。
- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib --test verification --no-fail-fast`：243／85 项通过（`/tmp/luna-tools-calhcwm1/result.json`）。本轮合计 1425 项不同 Rust 测试及 6 项 Python 测试通过。
- workspace 格式检查通过（`/tmp/luna-tools-ow3vzqsr/result.json`），`git diff --check` 通过。改动保留在 dev 工作区，未提交。

新增测试最初将所有位宽调用串成一个长 and 表达式，触及程序验证预算（`/tmp/luna-tools-88zzegg4/result.json`）。现按位宽拆成独立程序，覆盖保持；没有提高预算，也不宣称长表达式证明成本已解决。后续发现调用方契约的待定文字事实缺失（`/tmp/luna-tools-41vwlqyg/result.json`），已通过上述 formation 接线修复。

## PTC 查询内搜索顺序复用（2026-10-24）

required_atom 原先在每个搜索节点重走布尔结构、分配工作栈并重新定位原子。现在仅在一个 Search 内建立去重的优先索引顺序；每次选择仍读取当前 assignment，回溯能重新选择先前已赋值的原子。该结构不跨上下文、目标或证明检查复用，也不缓存类型事实。索引辅助数组与工作栈批量扩展先计费；只有完整构建后才发布顺序，半途耗尽预算不能留下部分缓存。搜索的双分支证书与独立检查规则保持原状。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib --no-fail-fast`：243 项通过（`/tmp/luna-tools-f8u84gsm/result.json`）。补强中途预算失败案例后，`--lib required_atom_order_is_query_local_and_assignment_independent -- --nocapture` 再次通过（`/tmp/luna-tools-tcnpc2j5/result.json`），同时检查零预算、部分构建失败、恢复、回溯、普通分支与不同目标隔离。

`test -p elaboration --test ptc --test ptc_kinds --test ptc_integer --no-fail-fast`：57／9／7 项通过（`/tmp/luna-tools-msw8kpbq/result.json`）。`test -p luna --test host --test elaboration generic --no-fail-fast`：host 23、elaboration 50 项通过（`/tmp/luna-tools-7kgqkju1/result.json`），包含八参数关联类型、泛型整数契约与原生执行。合计 389 项不同测试通过，没有用过滤组结果宣称全量通过。格式检查（`/tmp/luna-tools-3g__vsr9/result.json`）和 `git diff --check` 通过。改动留在 dev，未提交。

## 关联类型组合的验证成本（2026-10-24）

有限域验证从“先形成笛卡尔积，再排除组合”改为逐参数排除不可能的前缀。仅用 Entailed 的否定命题排除，Unknown 保留；64 项上限保持，作用于存活组合。类型查询限额仍为每次 4096 步，改按 prove_metered 的实际消耗计费。八个二选一且相等的参数原会触及组合上限，现在形成两个实例。

PTC 原先按原子出现顺序搜索，可能在处理强制等式前枚举多个分支。新增 required_atom 仅改变搜索顺序：沿正向 And、反向 Or 和 Not 查找布尔结构要求的原子；搜索仍生成原有双分支 Split 证书，独立检查器不变。新增回归在受限预算内证明关联类型域，复核证书／反例见证，覆盖否定结构以及预算耗尽后恢复。

源码验收同时发现 P4 分析有相同的完整笛卡尔积限制，已同步修复。直接正向相等前提可复用较早参数的分支；不从 or 或 not 内提取无条件别名。对一般组合的排除查询设单次限额，避免可行组合的反例构造耗尽整个行为分析预算。新增 float 域单元测试检查正向相等、否定相等和带 true 的可选相等，不将条件等式错误提升为前提。

工作目录 `/home/fin/luna`，最终验证：

- `python3 -B development/check.py --timeout 900 -- test -p elaboration --lib --no-fail-fast`：242 项通过（`/tmp/luna-tools-q3sgg29q/result.json`）。
- `test -p elaboration --test ptc --test ptc_kinds --test ptc_integer --no-fail-fast`：57／9／7 项通过（`/tmp/luna-tools-7g9kr_2e/result.json`）。前次受管配额恢复后日志仍不完整，报告为 evidence-error，已重跑，不计前次为通过。
- `test -p luna --test host --test elaboration --no-fail-fast`：587／449 项通过（`/tmp/luna-tools-08xj1sad/result.json`）。新源码回归使用八参数 pure fn，两种整数实例分别验证 LLVM O0/O3，并拒绝未被调用的错误契约；纯度缺口修复前的失败保留在 `/tmp/luna-tools-r4pbjv6b/result.json`。
- `reproduce.py --timeout 30 -- development/repros/correlated-generic-purity.fl --emit nir`：通过（`/tmp/luna-tools-ynabe78k/result.json`）。workspace 格式检查通过（`/tmp/luna-tools-21bnifvx/result.json`），`git diff --check` 通过。改动留在 dev，未提交。
- `test -p luna --test runtime_purity --test pure_callbacks --no-fail-fast`：纯度与回调两组合计 144 项通过（`/tmp/luna-tools-9ffe9ebf/result.json`）。连同上述最终检查，本次共 1495 项测试通过。

本次降低的是类型组合搜索成本；没有更改 wrapping-abs 示例的共享 CLI fuel 配置，也未重验其默认预算是否足够。一般未知／无限类型域仍沿用原有分析边界。

## 有限整数类型域上的泛型契约（2026-10-24）

接通泛型取负、NIR 开放操作的阶段检查和有限类型域的全称程序验证。源检查器仅在类型约束证明 T 属于 i8/i16/i32/i64/i128 时准入泛型取负。NIR 仅推迟当前函数拥有的 Type 参数、且操作数和结果类型相同的整数操作；仍拒绝 runtime comptime Integer、未拥有的 rigid 和伪造结果类型，并保留机器验证义务。

新增 flow/finite_domains.rs：用类型命题证明每个类型参数属于固定宽度整数的有限集合，枚举最多 64 个组合，仅排除已证明不可能的类型组合。每个实例仍由程序验证器检查完整函数及契约，默认实参独立验证；未被调用的错误定义也拒绝。组合枚举／类型替换使用共享预算，每个函数实例沿用单态分析预算。实例化后的残余验证继续执行，既不凭已见调用猜类型域，也不将值规格交给 PTC。一般未知域、无限域及任意数值协议仍需后续支持。

math.wrapping_abs 现显式导出 `result == value'old or result == -value'old`；五种位宽含 INT_MIN 均检查。跨包 wrapping-abs 示例的 magnitude 再次导出该关系，验证公共契约传递。资源检查／残余验证／代码生成共享 CLI fuel，默认 100000、试验 1000000 均耗尽（`/tmp/luna-tools-f4idp41a/result.json`、`/tmp/luna-tools-hwhjjve5/result.json`）；示例目录显式提供 10000000，O0/O3 通过（`/tmp/luna-tools-rc85yjha/result.json`）。未修改全局默认、没有跳过证明；成本优化是剩余工作。残余契约错误现在包含函数名，便于定位预算消耗。

工作目录 `/home/fin/luna`，验证证据：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test elaboration --no-fail-fast`：586／449 项通过（`/tmp/luna-tools-2na1m_iz/result.json`）。新测试包含五位宽原生 O0/O3、最小值及未被调用的错误泛型定义拒绝。初稿拒绝例的 `{-value}` 被正确识别为块注释，修正空格后通过。
- `test -p nir --test phases --no-fail-fast`：28 项通过（`/tmp/luna-tools-kd8s3atj/result.json`）；`test -p elaboration --lib finite_domains -- --nocapture`：组合相关性、数量上限、预算失败和恢复测试通过（`/tmp/luna-tools-ejwi06pj/result.json`）。
- `test -p luna --test library_protocols --test integer_contracts --no-fail-fast`：11／14 项通过（`/tmp/luna-tools-2wl8tggc/result.json`）。首次运行因 EDQUOT 连报告保存都失败，不计通过；受管 cargo.py clean 清理 luna/elaboration 可重建产物后重跑。
- 示例目录脚本 6 项通过（`/tmp/luna-tools-z48u1u2w/result.json`）。Rust 合计 1089 项通过；跨包示例两次构建及执行通过。全 workspace 格式化完成，改动保留 dev，未提交。

## handler 稳定捕获的算术回归（2026-10-24）

核查剩余基线失败后，修正旧测试将 var 声明一概视为不稳定捕获的预期。正式效应语义允许零次或多次请求（docs/algebraic-effect-and-delimited-continuation/algebraic-effect.md）；已有 flow 验证扫描完整函数写入及地址暴露，对可能变化的捕获恢复声明域，而未写入、未逃逸的词法标量在任意请求点都保持安装事实。此次没有改变验证算法，只澄清注释及测试边界。

前端测试保留未知操作参数加一的拒绝，并增加主体写入、handler 内写入及指针别名写入后沿用旧界的拒绝。native 新测试分别使用 let／var 捕获 41，跨函数连续请求两次，在解释执行和 LLVM O0/O3 中均输出 42。

工作目录 `/home/fin/luna`。定向算术组通过（`/tmp/luna-tools-69jef1a5/result.json`）。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test elaboration --no-fail-fast`：host 585、elaboration 449 项，共 1,034 项全部通过（`/tmp/luna-tools-msr7l6b5/result.json`）。全 workspace 格式检查通过（`/tmp/luna-tools-a_kv8ps7/result.json`），`git diff --check` 通过。修改留在 dev，未提交；泛型数值契约等前端缺口仍按检查点续接。

## 未知 trait 成员的查找边界（2026-10-24）

修复已有前端失败：普通 comptime Type 参数上的 `A.as(Source).Missing` 原先在成员不存在时仍查询实现，泄漏 `invalid binder reference 0:0`。selected_method 现在先要求成员属于所选 trait 或其父 trait，再执行证据检查／实现选择；不存在的成员给出 `unknown trait member`，不猜测证据。原拒绝用例改为要求这一具体诊断，测试辅助函数同时输出输入源码和预期诊断，避免混合用例误定位。新增回归覆盖开放 Type 输入的有／无证据、具体类型的有／无实现及父 trait 查找。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test elaboration --no-fail-fast`：448 项通过，剩余 1 项先前已确认的 handler 捕获算术基线失败（`/tmp/luna-tools-4gicpdne/result.json`）；未修改该失败的语义预期。新增未知成员组定向重验通过（`/tmp/luna-tools-42dy2j3_/result.json`）。`test -p luna --test host trait --no-fail-fast`：15 项通过（`/tmp/luna-tools-e3g5riwy/result.json`），验证现有 LLVM trait 调用、资源与契约路径。workspace 格式检查通过（`/tmp/luna-tools-z3alyz7c/result.json`），`git diff --check` 通过。修改留在 dev，未提交。

## 已证明标量域上的泛型相等比较（2026-10-24）

最终验证补充：`python3 -B development/check.py --timeout 900 -- test -p luna --test elaboration --no-fail-fast` 为 446 项通过、2 项失败（`/tmp/luna-tools-_jpo9ad6/result.json`）。临时关闭本次新增的泛型标量比较入口后，同一命令仍为相同的 446／2（`/tmp/luna-tools-e3j3te40/result.json`），确认属于已有工作区基线。失败分别涉及 direct handler 捕获值算术的拒绝预期，以及缺少关联类型证据时出现 invalid binder reference 诊断；未为通过测试改变语义预期。恢复入口后，`test -p luna --test host generic_scalar_equality --no-fail-fast` 两项通过（`/tmp/luna-tools-pgk4qfb_/result.json`）。最终 workspace 格式检查通过（`/tmp/luna-tools-wxl1oqsw/result.json`），`git diff --check` 通过。上述结果不表示前端全绿。

源检查器现允许 rigid 类型参数在 requires 已证明的统一标量域上使用 ==／!=，保留 ScalarEqual 及泛型输入类型，实例化后再采用实际类型的比较语义。可证明域包含 bool、char、str、Symbol、固定宽度整数和 f32/f64；PTC 仅证明类型成员关系，不证明值相等。不从已见调用推断域，不展开 normal-form 函数体，也不将 Type／Integer／BitVec 等不同操作类别混入此入口。

新正例在同一泛型函数中实例化 signed／unsigned 整数、两种浮点、bool、字符、字符串和 Symbol，解释执行与 LLVM O0/O3 对照；覆盖 NaN 不等于自身、正负零相等及 !=。未约束泛型、混入名义类型和不透明 Identity 头保持诊断。这里的 Type 混合域测试只维护当前不支持诊断，不把它定义为语言禁止。

尝试为 math.wrapping_abs 发布泛型结果契约暴露了后续边界：泛型取负尚未接通，临时放行时阶段检查仍将它误判为 comptime Integer 操作。因此本次没有发布该契约，也没有放松泛型取负或机器证明；可继续实现的源码保存在 `repros/generic-wrapping-abs-contract.fl`，当前失败报告 `/tmp/luna-tools-bqtoo15y/result.json`。现有 math 库签名保持原状。

工作目录 `/home/fin/luna`。两项新定向测试通过（`/tmp/luna-tools-v8yoas72/result.json`）。原四组回归在链接阶段遇到磁盘配额，日志写入也失败，报告 `/tmp/luna-tools-izavyggn/result.json` 明确为 evidence-error，不计为通过。通过受管 cargo.py 清理 luna／elaboration 可重建产物后，单独 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast` 的 584 项通过（`/tmp/luna-tools-oickswxc/result.json`）。修改留在 dev，未提交。

## wrapping_abs 的输入关系与谓词移项（2026-10-24）

最终 host／整数契约回归：`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test integer_contracts --no-fail-fast`，582／14 项通过（`/tmp/luna-tools-t9nw_vvy/result.json`）。结合下述低层验证共 918 项通过；最终格式检查（`/tmp/luna-tools-ehxfk3gp/result.json`）及 `git diff --check` 通过。

已检查的 abs intrinsic 现保留输入值快照与结果的关系。非负输入以输入多项式表示结果；排除 INT_MIN 的负区间以其相反数表示结果；未知符号或含 INT_MIN 时保留“结果等于输入，或等于其数学相反数”的候选。候选只作为已证明的逻辑事实保存，不通过提前拆分／合流改变结果身份。输入随后写入时，关系仍指向调用时的值，不改写成对当前存储的承诺。显式函数契约可以导出这些关系，一般函数仍不自动获得新的契约。

源码测试还定位了保留谓词的等价判断缺口：`r == -x` 与 `r + x == 0` 原来无法匹配。现通过有预算限制的多项式相减比较等式／不等式两侧之差，允许等式两侧交换，保持 Equal／NotEqual 种类，不对有序比较套用符号反转规则。值版本检查在等价判断前执行；不除系数，不扩展多项式次数，也不调用 PTC。

新增五种整数位宽的源码正例覆盖非负恒等、负值相反数、跨零候选、old 输入及原变量写入；错误的恒等／零值契约拒绝，改写输入后强行 assumes 结果等于新输入在 O0/O3 都失败。低层测试覆盖谓词移项、两侧交换、比较种类／常数差异、过期值、预算失败与恢复。确定符号的关系必须进入多项式复制路径，跨零候选必须保留逻辑形态；对应原失败见 `/tmp/luna-tools-6ensnlre/result.json` 和 `/tmp/luna-tools-k2h0z5y_/result.json`，修复后的两项关系测试通过（`/tmp/luna-tools-v0a8hk14/result.json`）。

工作目录 `/home/fin/luna`。`test -p elaboration --lib --test verification --no-fail-fast` 已有 236 项单元测试与 85 项集成测试通过（`/tmp/luna-tools-pnj2tx6_/result.json`）；该轮唯一失败是新增测试误用 remember_condition 保存有界等式，修正为未决候选后定向测试通过（`/tmp/luna-tools-v7d1v7yd/result.json`）。格式化通过（`/tmp/luna-tools-gewnc8la/result.json`）。修改留在 dev，未提交。

## wrapping_abs 的区间契约（2026-10-24）

最终受影响 host 回归：`python3 -B development/check.py --timeout 900 -- test -p luna --test host wrapping_absolute --no-fail-fast` 三项通过，包含原生全宽度／实参次数、LLVM poison 标志及新增契约检查（`/tmp/luna-tools-yvwec4qo/result.json`）；`git diff --check` 通过。

程序验证层现为已绑定的 abs intrinsic 计算精确区间包围：全正输入原样保留区间，全负输入反转端点，跨零输入从零起算。只含 INT_MIN 时结果仍为该值；包含 INT_MIN 及下一整数时结果必须保留整个有符号域，不能错误证明非负。分析读取已求值的输入描述，不重新执行实参；精确单值可参与后续验证，但不会变成 comptime invocation。一般函数调用仍依赖其已发布契约，不在本次自动生成跨函数关系规格。

低层测试遍历全部 32,896 个 i8 区间，对每个输入逐值枚举 wrapping_abs 后比较实际最小／最大值；另覆盖 i128 最小值与相邻值、错误位宽、越界区间、步数／整数大小预算失败和恢复。源码契约回归验证五个宽度的跨零／全负／全正／排除最小值／精确最小值，O0/O3 均通过；声称所有输入结果非负或都等于最小值的后置条件仍被拒绝，原有产物保持不变。初稿 `n>{minimum}` 遇到词法边界，改为 `n > {minimum}` 后通过，没有修改解析规则。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p elaboration --lib verification::flow::intrinsics --no-fail-fast` 四项通过（`/tmp/luna-tools-bd1r5jzs/result.json`）。源码新契约测试通过（`/tmp/luna-tools-sqf7k0lp/result.json`）；格式检查通过（`/tmp/luna-tools-0vjcyfsg/result.json`）。开发在 dev，未提交。

## LLVM 整数绝对值与 wrapping_abs（2026-10-24）

新增 `Operation::WrappingAbsoluteValue`，CLI 操作名 abs，接受且只接受一个 i8／i16／i32／i64／i128 输入，并要求输出同域。LLVM 降码为 `llvm.abs.iN(value, i1 false)`，保持 INT_MIN 原值，不产生 poison；详见 [LLVM 21 abs 契约](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-abs-intrinsic)。`math.wrapping_abs<T>` 是对应的 pure 泛型包装，现有浮点 math.abs／fabs 保持不变。整数全域结果不能承诺非负，当前程序验证保守使用原整数域；没有把目标 intrinsic 开放为 comptime invocation，也没有扩展平台整数／无符号输入。

同步了显式操作注册、CLI 帮助、库命令、库检查脚本及示例运行器。新增 `examples/host-wrapping-abs.fl`，可运行 `python3 -B development/host_examples.py wrapping-abs --compare-optimization`。目录现有 35 个示例；本次仅重验受影响子集。

工作目录 `/home/fin/luna`。新测试覆盖全部 256 个 i8 输入、其他四个宽度的极值／相邻值／正负数／零、实参恰好一次、LLVM poison 标志以及错误符号／域／参数数目拒绝。测试初稿直接把含赋值调用写进 assumes，正确触发纯度拒绝（`/tmp/luna-tools-l1xlc1yt/result.json`）；改为先执行、后检查其布尔值后通过。

- `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test host_intrinsic_protocol --test library_protocols --test frontend_cli --no-fail-fast`：host 579、intrinsic 协议 33、库 11 项通过；CLI 初次有两项因手写绑定清单漏 abs 失败（`/tmp/luna-tools-o4weum5j/result.json`）。补齐后 `test -p luna --test frontend_cli --no-fail-fast` 的 201 项全部通过（`/tmp/luna-tools-gp0zg5c9/result.json`）。最终四组合计 824 项通过。
- `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py wrapping-abs math-fma integer-minmax --compare-optimization`：三个跨包示例 O0/O3，共六次构建与原生执行通过（`/tmp/luna-tools-xogo2vxc/result.json`）。
- `python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p 'test_host_examples.py'`：六项通过（`/tmp/luna-tools-38n4bi1x/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-194wzk7h/result.json`）。修改留在 dev，未提交。

## 短路条件的部分谓词保留（2026-10-24）

程序验证层补充检查：`python3 -B development/check.py --timeout 900 -- test -p elaboration --test verification --no-fail-fast`，85 项通过（`/tmp/luna-tools-09_71z5t/result.json`）。连同下述 host 与 integer_contracts，本次共 676 项通过；`git diff --check` 通过。

`if d != 0 and n / d > 1 {n % d}` 原先在余数计算处报除数未证明非零，新回归先复现了该失败（`/tmp/luna-tools-ngdg1fvl/result.json`）。原因是短路合流时右侧计算的观察不能保留，导致整个组合谓词连同有效的左侧观察一起丢弃。现在分别校验两侧观察的版本，用新鲜的未知 Bool 表示缺失的一侧，保留另一侧的有效约束。不是将 `A and unknown` 简化为 A：只有成功分支可以依赖 A；or 的对应保证只在失败分支成立。新鲜观察通过现有预算和事实版本管理，不改动执行 IR，不将程序命题交给 PTC。

新增 host 回归覆盖 if／while、and／or、零除数短路、右侧副作用恰好一次，以及错误分支和右侧改写除数后的拒绝；正例同时比较解释执行与 LLVM O0/O3，拒绝例确认不发布可执行文件。host-number-theory 已恢复直接使用复合 while 条件，其余尚需显式的数值界不在本次扩展范围。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test integer_contracts --no-fail-fast`，577／14 项通过（`/tmp/luna-tools-o1zlo9p2/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py number-theory --compare-optimization`，更新示例 O0/O3 均输出 1121（`/tmp/luna-tools-q4qubwig/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-xmmmffeh/result.json`）。修改留在 dev，未提交。

## 私有驱动复制的存储失败恢复（2026-10-24）

此前 Cargo 编译成功后，复制私有 driver 的 EDQUOT 不进入 Cargo 子进程的存储失败恢复，导致整条 run_host 中断。build_host_driver.py 现在保持编译器源文件打开，在原有维护锁内删除本次未完成的副本、清理受管 target，然后 rewind 并重试复制一次。即使清理删除源路径，已打开文件仍提供完整原产物，无须重建占用空间的依赖。只对 ENOSPC／EDQUOT 重试；持续失败、权限和普通 I/O 错误正常返回，已有文件或悬空软链接不会被覆盖。不会清理其他运行目录或源码。

工作目录 `/home/fin/luna`。`python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p 'test_*host*.py'`：18 项通过（`/tmp/luna-tools-0lzblogv/result.json`），故障注入覆盖两种存储错误、部分复制、清理删除源路径、从头恢复、重试上限和非存储错误。相同入口运行 `test_cargo.py` 的 13 项通过（`/tmp/luna-tools-d7kj2te7/result.json`）。实际 `run_host.py development/examples/host-number-theory.fl --opt-level 3` 构建执行退出 0，输出 `(unsigned-integer 1121)`（`/tmp/luna-tools-82250xwb/result.json`）；没有人为耗尽真实磁盘，配额恢复路径由故障注入验证。修改留在 dev，未提交。

## 当前可运行入口验收（2026-10-24）

本次沿用工作区已有的 IIR → LLVM → 原生对象 → host 链接／执行链路，环境 `llvm-config-21 --version` 为 21.1.8。新增 `examples/host-number-theory.fl` 并注册到示例目录：普通运行时函数完成素数求和、Euclid 算法及 Fibonacci 递归，结果为 `(unsigned-integer 1121)`。示例不依赖额外库或协议配置，可直接执行 `python3 -B development/run_host.py development/examples/host-number-theory.fl --opt-level 3`。函数 contract 与受检查的 assumes 保留，未关闭算术检查来迁就后端。

工作目录 `/home/fin/luna`，本次验证：

- `python3 -B development/command.py --timeout 900 -- python3 -B development/host_examples.py --compare-optimization`：34 个示例、O0/O3 共 68 次构建及原生运行全部通过。报告 `/tmp/luna-tools-p7c4inty/result.json`；汇总 `/tmp/luna-host-examples-24sze8wq/summary.json`，逐项复核构建／执行退出 0 且日志未截断。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test frontend_cli --no-fail-fast`：host 575 项、frontend_cli 201 项通过。报告 `/tmp/luna-tools-w8vj0nh7/result.json`。
- `python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p 'test_*host*.py'`：15 项通过。报告 `/tmp/luna-tools-1d_6eh29/result.json`。
- 上述单文件 run_host 命令由 command.py 包装实际执行，退出 0，stdout 为 `(unsigned-integer 1121)`，报告 `/tmp/luna-tools-kqpidrd0/result.json`。首次单文件验收因保存私有 driver 时磁盘配额不足失败（`/tmp/luna-tools-59omb3yo/result.json`）；仅清理本次探索运行产生的六份重复 driver 后重试成功，保留所有报告、示例程序及完整目录验收的 driver。

语义前端续接边界记录在 frontend-checkpoint.md。本次不是重新实现已有后端，也不表示整个语言都可原生编译；既有不支持的 effect ABI、未知存储来源及前端推导缺口仍保留。所有修改留在 dev 工作区，未提交。

## 无注解 pattern 方法结果的已知输出名（2026-10-24）

修复 `const pat=do {Ops}.pattern();let ~<pat>=true;value` 的名称导出缺口。lowering 以有界工作栈收集块／分支结果中声明身份已知的容器，逐一筛选 member_access 的可访问成员并读取扫描阶段的输出名字；所有可能容器的名字集合必须一致。泛型实参不改变声明身份，但 normal-form 函数头不能冒充名义容器。结构、候选及输出名复制均计费。输出类型和代码类别仍由 elaboration 检查。

名称导出后另暴露无注解调用落入普通函数路径的问题；新增 CMTT 类别提示要求所有结构明确的候选均为代码方法且类别一致，仅用于选择表达式／pattern 代码处理入口。真正 invocation 重新解析接收者、检查实现和权限，没有拿提示中的代表 owner 当作选中的实现。不同容器有同名但不同类型输出时，仍按实际选择的契约拒绝不匹配输入。

正例覆盖原复现、if／when／match、泛型容器和多容器相同输出名；拒绝例覆盖 private、无关容器、不同输出名、同名不同输出类型、错误 subject、runtime 条件和 Identity<Ops> 不透明头。工作目录 `/home/fin/luna`，八组命令 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --test cmtt_methods --test method_memory_contracts --test trait_purity --no-fail-fast`：575／389／523／17／211／142／9／21，共 1,887 项通过（`/tmp/luna-tools-in6kehmo/result.json`）。补强同名不同类型用例后定向拒绝测试通过（`/tmp/luna-tools-wgbeysdy/result.json`）。格式化通过（`/tmp/luna-tools-32fruqtk/result.json`），`git diff --check` 通过。

更新的 type-packets 示例无需给 pattern 常量写类型注解即可导出 enabled，O0/O3 构建执行通过（`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization`，`/tmp/luna-tools-o_4ix96w/result.json`）。当前驱动独立复现原问题输出 bool true（`/tmp/luna-tools-mn916f5o/result.json`）。新增工厂调用复现 `repros/comptime-factory-pattern-binding.fl` 仍报 value 未解析（`/tmp/luna-tools-z8qqmxi4/result.json`，退出 1）；这不是已经完成通用延迟名称解析的声明。本轮未重跑完整示例目录，dev 工作区未提交。

## 计算 Type 接收者上的 CMTT 方法（2026-10-24）

quote fn 基础复现已在原基线通过（`/tmp/luna-tools-l6_3dyyk/result.json`），不把它记为本轮新实现。返回 quoted code 的 comptime 方法原失败为“code method receiver type is not established”（`/tmp/luna-tools-7mtetuls/result.json`，`repros/comptime-computed-code-method.fl`）。本轮将名义代码方法和显式 trait 代码方法的 invocation 主语分类接入已检查的 Type 域：Type 主语完成实际身份后复用原实施选择／访问检查，不制造值 receiver。分类提示仍不能仅按名称授予实现证据。

正例覆盖块／if／when／match、Type 工厂、记录 Type 字段、显式 trait、带显式输出契约的 pattern 方法，以及 quote fn 普通实参恰好一次。拒绝例覆盖代码结果域、非 Type 主语、缺失／private 方法、runtime 函数／输入、缺失 trait 实现、错误 pattern 输出类型和 Identity<Ops> 的不透明头。无注解 pattern 输出绑定仍失败，复现保留 `repros/comptime-computed-pattern-binding.fl`：这是 lowering 名称导出缺口，未修改成“应该拒绝”的语义测试。显式契约测试初稿还修正了 `>=` 连写为 `> =`，避免被词法识别为比较运算符。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --test cmtt_methods --test method_memory_contracts --test trait_purity --no-fail-fast`：573／389／523／17／211／142／9／21 项，共 1,885 项通过（`/tmp/luna-tools-ra97oohq/result.json`）。新正例含解释执行与 LLVM O0/O3。全 workspace 格式化通过（`/tmp/luna-tools-omhpq795/result.json`），`git diff --check` 通过。dev 工作区未提交。

更新 type-packets 示例加入计算 Type 的代码方法调用与 splice。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` O0/O3 构建执行通过（`/tmp/luna-tools-gq48zks5/result.json`）。当前私有驱动重新执行无注解 pattern 复现，仍在 lowering 报 value 未解析（`/tmp/luna-tools-0_kn6c7w/result.json`，退出 1）；该限制不是旧日志推测。本轮未重跑完整示例目录。

## 计算 Type 接收者上的静态函数（2026-10-24）

`do {Ops}.invert(false)` 原失败保留在 `repros/comptime-computed-static-method.fl`（`/tmp/luna-tools-9ph9k47u/result.json`）。static_method 现识别经声明域检查的 Type 主语，并在实例方法路由前处理计算接收者；实际身份通过 required 类型求值完成，随后使用已有 selected_method、可见性、泛型及调用检查。不注入 runtime self，不按函数名猜测接收者身份。

正例覆盖块／if／when／match、Type 工厂、返回记录的 Type 字段、泛型推断／显式实例化、函数值、comptime 静态函数、不同工厂结果以及普通实参恰好一次。拒绝例覆盖参数域、非 Type 主语、缺失／private 方法、runtime 函数／输入和 Identity<Ops> 不透明头。初始 when 测试在语句开头未加括号，HIR 明确解析为 when 语句和独立的 Symbol 调用（`/tmp/luna-tools-3khlef4d/result.json`）；已改为括号表达式，没有改变解析规则。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --no-fail-fast`：571／389／523／17／211 项，共 1,711 项通过（`/tmp/luna-tools-q7z0ghv4/result.json`）。追加 `test -p luna --test method_memory_contracts --test cmtt_methods --test trait_purity --no-fail-fast`：9／142／21 项通过（`/tmp/luna-tools-19synnx0/result.json`）。八组合计 1,883 项通过；新正例含解释执行和 LLVM O0/O3。全 workspace 格式化通过（`/tmp/luna-tools-_8y7fyn9/result.json`），`git diff --check` 通过。dev 工作区未提交，未据此宣称全部计算 Type 上的 quote 方法已实现。

## 组合表达式直接返回 Type 的成员选择（2026-10-24）

`do {Outer}.Inner<bool>.Payload` 原失败见 `repros/comptime-composite-type-member.fl` 和 `/tmp/luna-tools-2983wxrw/result.json`。块／if／when／match 直接产生 Type 时，接收者分类现在接受其已检查域，实际身份仍在使用位置完成。成员表达式的容器准备也完成整个 comptime Type 计算，而非只接受已经是 Type 字面值的块末尾；因此块内的合法局部赋值／分支可以决定名义容器。成员常量转入 runtime 继续要求 Splice 协议。

正例覆盖直接块、局部 const 别名、if、when、match、Type 工厂、成员常量和块内状态选择不同容器。负例覆盖错误 Payload／类型实参、非 Type 接收者、runtime 输入／函数、private 成员及 Identity<Outer> 的不透明头。曾尝试的 runtime 局部状态正例经 docs/type/index.md 核对后改为拒绝例：类型值属于 comptime，不能读取未来 runtime 状态。最终测试在已绑定 Splice 时仍拒绝该输入，不依靠缺失协议制造拒绝。初始缺失 Splice 的报告为 `/tmp/luna-tools-3b3ul5px/result.json`；合法块内状态随后暴露字面值限制（`/tmp/luna-tools-fvv8g4n7/result.json`），修复后定向 3 项通过（`/tmp/luna-tools-g0h82711/result.json`）。

工作目录 `/home/fin/luna`。最终 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --no-fail-fast`：569／389／523／17／211 项，共 1,709 项通过（`/tmp/luna-tools-o3twt1sa/result.json`）。正例含解释执行和 LLVM O0/O3。全 workspace 格式化通过（`/tmp/luna-tools-5qst0ubk/result.json`），`git diff --check` 通过。dev 工作区未提交；未据此宣称未知 Type 身份的通用推导或全部动态成员调用已完成。

type-packets 示例新增直接 Type 块的嵌套成员注解。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` 的 O0/O3 构建与执行通过（`/tmp/luna-tools-j8scu78s/result.json`）。本轮未重新运行完整示例目录。

## 组合表达式结果字段的类型成员（2026-10-24）

`do {packet()}.selected.Inner<bool>.Payload` 的原失败保留在 `repros/comptime-block-type-member.fl`（`/tmp/luna-tools-wh4qit63/result.json`）。块／if／when／match 根现通过不提交的 checking_attempt 获取已检查的结果域；临时局部、捕获和控制证据不发布，字段身份仍通过后续完整 required 计算取得。comptime 探测不适用时，普通 runtime 数据投影保留原检查入口，不能因为探测而被改成 comptime 调用。

正例覆盖块局部 const 别名、if、when、match 和 runtime 数据块；后者连续修改状态并调用 runtime 工厂，检查结果 true／false 及最终状态，验证探测没有额外执行源程序或遗留阶段模式。拒绝例继续检查错误 Payload、非容器 Type、runtime 函数／条件、错误泛型实参、private 成员和 Identity<Outer> 的不透明头。首次定向测试的 runtime 名义数据示例缺少 host 要求的 Copy/Drop 协议配置（`/tmp/luna-tools-q1i17ui6/result.json`），补齐已有 RESOURCE_HEADER 和 CLI 协议后重验，没有取消资源检查。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --no-fail-fast`：567／389／523／17／211 项，共 1,707 项通过（`/tmp/luna-tools-87ii9131/result.json`）。每个新正例含解释执行与 LLVM O0/O3。全 workspace 格式化通过（`/tmp/luna-tools-thr58thv/result.json`），`git diff --check` 通过。改动留在 dev，未提交；未知运行值的通用 Type 身份及部分对象分析仍未完成。

更新的 type-packets 示例加入条件结果上的 Type 字段成员选择。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` 的 O0/O3 构建和执行通过（`/tmp/luna-tools-9w10050n/result.json`）。本轮未重跑完整示例目录。

## 泛型调用返回域的字段实例化（2026-10-24）

`packet(value:T)->Packet<T>` 中的 selected 字段不能直接用声明端 T 判断是否为 Type。原复现 `repros/comptime-generic-returned-type-member.fl` 在继续选择泛型成员时报错（`/tmp/luna-tools-okzqrqrw/result.json`）。现在对仍含 rigid 参数的返回域，使用已有 required 准备过程完成真实调用的参数／默认值推断，取得实例化 result_type，再继续字段域检查。准备不会 invocation 工厂；最终字段所含 Type 的身份仍由完整 required computation 求得。rigid 检查按原有 required 预算计费，不展开 normal-form 头。

正例含推断和显式泛型、扩展位置调用、optional 默认／显式命名供应、闭合函数实例别名，以及两次调用返回不同容器时分别取得 bool 和 str 的 Payload。拒绝例含非 Type 字段、错误实参和显式类型实参、不合法属性供应、runtime 函数／输入、错误 Payload 和 Identity<Outer> 的不透明头。每个正例执行解释器和 LLVM O0/O3，失败构建保留已有文件。

工作目录 `/home/fin/luna`。首轮五组回归（`/tmp/luna-tools-vzpc7prx/result.json`）中，comptime_memory／open_functions／cmtt／newtypes 的 17／389／523／211 项通过，host 的 564 项通过、1 项失败：测试误把 `{value:Outer}` 属性收集当作普通参数命名供应。按 docs/functions/parameters.md 改为扩展位置输入和合法 optional `.value=Outer` 后，定向正例通过（`/tmp/luna-tools-lim9ud_k/result.json`），重跑 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast` 的 565 项全部通过（`/tmp/luna-tools-d2c2ov0s/result.json`）。合计五组 1,705 项通过；没有将首次失败记为通过。

全 workspace 格式化通过（`/tmp/luna-tools-t4aw8f0s/result.json`），`git diff --check` 通过。更新的 type-packets 示例通过 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` O0/O3 构建与执行（`/tmp/luna-tools-0na4iqk2/result.json`）。本轮未重跑整个目录；未知普通参数下的通用部分值分析仍未完成。dev 工作区未提交。

## 返回记录 Type 字段的嵌套成员（2026-10-24）

`packet().selected.Inner<bool>.Payload` 原先无法识别为类型成员（`/tmp/luna-tools-sf31twcd/result.json`）。现在从已检查的 comptime callable 签名取得返回域，沿记录字段实例化声明域；识别到 Type 字段后再通过 required computation 完成身份，继续使用原有使用位置的候选集及成员权限。签名域仅用于区分字段／类型路径，不代替实际 Type 值，也不把普通参数提升为 specialization 输入。复制调用返回域计入 required 预算。直接 Type-returning 调用仍通过同一域提取入口识别。

正例覆盖嵌套泛型成员、多层返回记录、泛型字段及局部函数身份别名；拒绝例覆盖字段载荷、非容器 Type、runtime 调用、不存在成员、错误泛型实参、Identity<Outer> 的不透明头和跨模块 private 类型／别名。解释执行与 LLVM O0/O3 验证相同结果，失败构建保留先前产物。type-packets 示例新增 `completed_outer().selected.Inner<bool>.Payload`。

工作目录 `/home/fin/luna`。最终实现的五组回归命令 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --no-fail-fast`：563／389／523／17／211 项，共 1,703 项通过（`/tmp/luna-tools-pingafd9/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-ovmn2xw5/result.json`）。本轮不宣称未知返回值的通用部分字段分析、任意计算根及全部泛型返回域推导已经完成。dev 工作区未提交。

强化泛型正例为字段域真正依赖 T 的 `Packet<.Type'builtin>` 后，`test -p luna --test host returned_type_ --no-fail-fast` 3 项通过（`/tmp/luna-tools-2mbsv3un/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` 的更新示例 O0/O3 均通过（`/tmp/luna-tools-pb13v83k/result.json`）；本轮未重新运行完整目录。`git diff --check` 通过。

## 计算接收者上的 metatype 路径（2026-10-24）

`packet().vars.Closure` 原先在识别内建命名空间时强制要求 packet() 是 Type（原失败 `/tmp/luna-tools-10x91rvo/result.json`）。现在复用 required computation 的准备／invocation 分离：先检查结果域，数据结果不在探测时执行，交给后续完整字段投影求值；Type 结果才 invocation，并确认值确实是 Type metatype。局部已知声明域及直接类型头继续复用原有路径。没有吞掉失败诊断后改用另一种解释，也不以字段名或 Type 域直接授予上下文类型身份。

正例覆盖调用、块、条件、泛型嵌套记录、带状态闭包的完整 computation，以及返回 metatype 的函数和块；拒绝例覆盖非 Type 字段、错误字段载荷、runtime 函数、返回 bool 的 Type 工厂和 Identity<Type> 的不透明头。每个正例执行解释器及 LLVM O0/O3，失败构建检查原有产物不受影响。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --no-fail-fast`：561／389／523／17／211 项，共 1,701 项通过（`/tmp/luna-tools-ejvn0567/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-9igips2x/result.json`）。修改留在 dev，未提交。

## 完整计算结果的 Type 字段（2026-10-24）

`let checked: packet().selected = true` 先前在 type_expr 的兜底分支报告未实现（`/tmp/luna-tools-2kqguuna/result.json`）。现在在成员／关联类型和部分已知字段路径之后，将普通字段投影交给已有 required comptime 求值路径。执行完整 receiver 和投影，要求返回 Type；不以字段域冒充身份，不改变 normal-form 头，不提升 runtime 输入的阶段。

回归覆盖函数返回值、块结果、条件结果、嵌套泛型记录、显式 comptime 参数，以及同一 required computation 中状态闭包的连续调用。错误载荷、非 Type 字段、缺失字段、runtime 函数／输入和 Identity<bool> 身份仍拒绝；失败构建保留已有文件。状态闭包用例初稿错误地用 let 整体赋值，定向报告 `/tmp/luna-tools-k1qxg5jg/result.json` 失败；按语言规则修正为 var 后重新运行验证，未放宽赋值规则。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --test comptime_memory --test newtypes --no-fail-fast`：559／389／523／17／211 项，共 1,699 项通过（`/tmp/luna-tools-hgr2t4i0/result.json`），正例含解释与 LLVM O0/O3。格式检查通过（`/tmp/luna-tools-uj6ekf1y/result.json`）。一般未知工厂结果的部分字段分析及非局部计算接收者的特殊 metatype 路径消歧尚未完成。dev 工作区未提交。

目录示例 type-packets 新增直接使用函数返回 Type 字段的注解。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` O0/O3 构建与执行通过（`/tmp/luna-tools-l2hf7812/result.json`，产物 `/tmp/luna-host-examples-xk9pvyjs`）。本轮未重复完整示例目录。`git diff --check` 通过。

## 数据字段与 metatype 命名空间消歧（2026-10-24）

修复 `packet.vars.Closure` 的普通 Type 字段被当作内建上下文类型路径的问题。沿局部绑定的声明域检查数据字段路径（含泛型字段替换），仅用于路径消歧；不执行 initializer、不授予字段访问权限，也不把 Type 的域当作已完成身份。真正的 `Type.vars.Closure`、module const metatype 及记录内 Type 字段路径保持有效。非 Type 字段、缺失字段、错误载荷和 `Identity<bool>` 不透明身份继续拒绝，失败构建保留已有产物。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test open_functions --test cmtt --no-fail-fast`：557／389／523 项，共 1,469 项通过，退出 0（`/tmp/luna-tools-afy66vz7/result.json`）。新正例分别经过解释执行及 LLVM O0/O3。全 workspace 格式化通过（`/tmp/luna-tools-zbvie_jm/result.json`），`git diff --check` 通过。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` 通过（`/tmp/luna-tools-o8rsyaem/result.json`）；本轮没有重跑完整示例目录。修改留在 dev 工作区，未提交。

## 当前集成基线（2026-10-24）

最新完整目录验收包含泛型返回域、组合 Type 表达式及静态函数调用。`python3 -B development/command.py --timeout 900 -- python3 -B development/host_examples.py --compare-optimization` 退出 0（`/tmp/luna-tools-q7qpacce/result.json`）：33 个示例在 O0/O3 共 66 次构建与原生执行通过，逐项结果 `/tmp/luna-host-examples-mqh67d8g/summary.json`。独立核对当前 EXAMPLES、原始 build／execute 报告及 stdout：无缺项／重复，全部 completed／exit 0，无输出截断，字节数和预期输出一致。以下保留前序基线。

最新整体验收已包含完整计算结果的 Type 字段和计算接收者 metatype 消歧。工作目录 `/home/fin/luna`，`python3 -B development/command.py --timeout 900 -- python3 -B development/host_examples.py --compare-optimization` 退出 0（`/tmp/luna-tools-uzo97n5l/result.json`）；33 个示例在 O0/O3 共 66 次构建与执行全部通过，逐项结果 `/tmp/luna-host-examples-_m4rul3l/summary.json`。另逐份核对原始报告：目录与当前 EXAMPLES 完全一致，所有 build/execute 均 completed／exit 0，输出未截断、字节数一致，stdout 等于预期加换行。原错误复现现输出 bool true（`/tmp/luna-tools-nh5fubd_/result.json`）。`git diff --check` 通过。以下保留此前集成记录供追溯。

条件成员来源、分支内 const Tuple 解构及直接 NIR Not 链执行合入工作区后，重新验收当前完整目录：33 个示例在 O0/O3 共 66 次构建与原生执行全部通过。工作目录 `/home/fin/luna`，命令 `python3 -B development/command.py --timeout 900 -- python3 -B development/host_examples.py --compare-optimization`，退出 0（`/tmp/luna-tools-objpc4bb/result.json`）；逐项结果 `/tmp/luna-host-examples-apb1xbxm/summary.json`。这替代此前 32 项目录的 `/tmp/luna-tools-jpm_ec8s/result.json` 集成基线。

独立读取本轮原始报告并对照当前 EXAMPLES：33 个名字各有且仅有 O0/O3 两项，66 份 build 与 66 份 execute 报告均 completed／exit 0，私有驱动构建成功，所有原生产物存在。构建命令的源码／多包输入、协议及 intrinsic 选项和目录定义一致；执行命令指向相应原生产物。逐字节确认 stdout 等于该例预期 UTF-8 加单个换行，日志字节计数一致且未截断，summary 输出与原日志一致。复核记录 `/tmp/luna-host-examples-apb1xbxm/audit.json`。

开发工具回归 `python3 -B development/command.py --timeout 300 -- python3 -B -m unittest discover -s development -p 'test_*.py'`，37 项通过，退出 0（`/tmp/luna-tools-1ggoa46i/result.json`）。本次是近期修改后的集成验收，没有新增语言能力或重复全量 Cargo 测试。Object 属性求值／外部函数 ABI、捕获式 continuation、具名 comptime 调用期环境、递归内存摘要及一般字符串来源等缺口仍见 frontend-checkpoint.md；本目录通过不能证明这些能力已完成。`git diff --check` 通过；dev 工作区未提交。

## 部分记录字段投影与直接类型注解（2026-10-24）

`repros/comptime-partial-field-projection.fl` 原先在 `const Selected=saved.packet.selected` 后无法形成成员类型注解（`/tmp/luna-tools-gqsy9jz2/result.json`）。字段观察现沿已检查 NIR 的 Field／AnonymousField 取得对应子描述：名义记录核对声明身份及位置，匿名记录核对名字和位置；完整已知子值与部分已知子结构均可继续投影。缺失字段、未知内容和指针读取没有替代值。

类型位置也可直接使用局部数据字段路径，例如 `saved.packet.selected.Inner<bool>.Payload`。普通表达式检查负责字段与域，再从观察描述取得 Type；成员选择继续使用各自使用位置的候选集合，复用已形成的中间容器。观察入口只接受已观察普通局部值的字段路径，排除 Type 根、调用、显式解引用及其他计算接收者，不借此提前执行 Type 工厂或重复构造。

正例覆盖嵌套字段、子记录别名、字段书写顺序变化、泛型字段默认值、完整已知子对象旁边的未知字段、直接 Type 注解和有状态字段 initializer 恰好一次。存储用例先保存 Type，再通过一个记录副本的字段指针写入 bool；验证写入目标改变，原副本和既有 Type 值保持原身份。写入后再形成 Type 别名或直接注解不能复用旧观察。拒绝例还覆盖非 Type 字段、未知字段、错误 Payload 和 Identity<bool> 不展开；失败保留旧输出。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test comptime_memory --test open_functions --test cmtt --test inline_control --test newtypes --no-fail-fast`：555／17／389／523／249／211 项，共 1,944 项通过，退出 0（`/tmp/luna-tools-s8s78tqp/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-6l8hnl5m/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization`：示例的记录部分改为字段投影及直接注解，O0/O3 构建与执行通过（`/tmp/luna-tools-55o7sa6v/result.json`）。

私有驱动 `/tmp/luna-host-examples-d1i6150z/luna` 经 reproduce.py 对原复现执行 `--interpret run`，返回 bool true（`/tmp/luna-tools-3stgz3zv/result.json`）。一般工厂返回值、未知内存读取及自定义 pattern view 的字段身份仍未补齐；匿名记录经一般调用产生的部分内容也不能由内部 Field 路径支持推断为完成。未重复完整目录，dev 改动未提交。

## tuple_enumerate 的 Type 观察与真实存储（2026-10-24）

`repros/comptime-type-packet-enumeration.fl` 原先在元素解构后只保留 Type 域，Selected 注解失败（此前 `/tmp/luna-tools-izi8n_53/result.json`）。现从已检查的 tuple 输入取得结构证据，在每轮元素模式绑定后保留其已知字段。支持原始构造、部分已知 const 输入与别名；输入本身仍由原循环前缀求值一次。

普通元素绑定仅取得独立观察描述，不进入 specialization_inputs，也不把可执行局部读取替换为 TypeValue。类型注解可读取已知 Type 身份；既有 callable 类型参数及 generator 数据规则优先。每轮恢复原作用域，防止 bool／str 两轮的身份串用。取址、写入和捕获会撤销相关观察；required computation 中也撤销观察，但原有符号参数与常量缓存规则保持不变。

回归验证直接 T 注解、const packet／别名解构后的 Selected、含普通 flag 的部分输入以及有状态 initializer 恰好一次。专门的存储用例对 T 取址，先保留旧 Type，再写入 u32，验证原存储改变、已有 const 观察和按值闭包捕获仍保存旧值。若写入发生在 packet 创建前，旧观察不能使 bool／str 载荷冒充 u32。缺少 Type 的 Copy 实现仍拒绝，原有资源权限没有被类型观察绕开。目录示例 type-packets 增加了该枚举路径。

工作目录 `/home/fin/luna`。首次五组检查 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test comptime_memory --test open_functions --test cmtt --test inline_control --no-fail-fast`：comptime_memory 17、open_functions 389、cmtt 523、inline_control 249 全通过；host 551 通过，一项旧 bitcode 往返测试在执行前遭遇 ETXTBSY，因此该次组合退出 101（`/tmp/luna-tools-6lip1tw4/result.json`），未计作全通过。

host 测试的 Work::execute 现与生产启动入口一致，仅对 exec 前的 ETXTBSY 重试、最多一秒；已经启动的程序和其他错误不重试。真实 ELF 写句柄回归覆盖先确认 ETXTBSY、释放后成功、持续占用有界失败及再次释放后恢复。随后 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --no-fail-fast` 553 项全部通过（`/tmp/luna-tools-7xxphcg9/result.json`）。合并上述未改动语义代码的四组证据，本轮共 1,731 项不同测试通过。最终全 workspace 格式化通过（`/tmp/luna-tools-2f19r4mu/result.json`）。

`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization`：更新例子的 O0/O3 构建及执行通过（`/tmp/luna-tools-ndtivqxl/result.json`）。私有驱动 `/tmp/luna-host-examples-set3icmx/luna` 经 reproduce.py 执行原复现的 `--interpret run --meta-list-type input.Elements --tuple-type input.Tuple` 返回 bool true（`/tmp/luna-tools-1uhqi8dw/result.json`）。任意工厂调用结果、未知存储读取及自定义 pattern view 的完整字段分析仍未完成。未重复完整示例目录，dev 改动未提交。

## 部分聚合的 const 别名与作用域（2026-10-24）

`repros/comptime-partial-packet-alias.fl` 原先在 packet → saved → inner 解构后丢失 Selected 的 Type 身份（`/tmp/luna-tools-g4gk4iys/result.json`）。现在使用独立的不可变分析描述保存已知 Term、聚合形状和 Unknown 字段，经过 const 别名及中间模式绑定传递。引用共享避免复制整棵聚合树；描述深度与比较／遍历预算受限。描述相同只表示证据相同，两个 Unknown 绝不构成运行值相等的证明。它不是运行值，不进入函数实例的静态实参缓存，也不保存或重放 initializer、存储指针和闭包环境。

块与 inline 循环按绑定身份清除离域描述，并保留已有失效；不会用旧 map 恢复已经失效的外层证据。开放函数检查、候选回滚、generator 和孤立失败分支保存／恢复各自上下文。普通 let 不进入这条 const 描述路径。描述实现 S-expression Display，覆盖字符串转义、Unknown 保留及预算失败后的重复读取。

正例包含 Tuple／struct／enum 别名链、as 整体绑定、中间子聚合、同名局部遮蔽、可能候选共有的部分字段、有状态 initializer，以及绑定 packet 后改写原变量／经指针改写原存储。范围 inline 循环在同一源码中依次产生 bool 和 str，检查每轮恢复的 Type 与本轮输入一致。拒绝例继续检查作用域、字段域、Payload 和 Identity<bool> 不展开。目录示例 type-packets 已更新为通过别名使用这三种聚合。

工作目录 `/home/fin/luna`。最终 `python3 -B development/check.py --timeout 900 -- test -p luna --test host --test comptime_memory --test open_functions --test cmtt --test inline_control --no-fail-fast`：550／17／389／523／249 项，共 1,728 项通过，退出 0（`/tmp/luna-tools-j06qsnbw/result.json`）。`python3 -B development/check.py --timeout 600 -- test -p elaboration --lib partial_snapshot_dump_and_budget_preserve_unknown_fields`：1 项通过（`/tmp/luna-tools-g7x38j95/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-qzho1acd/result.json`）。

`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization`：更新例子的 O0/O3 构建与执行通过（`/tmp/luna-tools-6oj_xb2r/result.json`）。使用私有驱动 `/tmp/luna-host-examples-2z86tttk/luna` 经 reproduce.py 对原 alias 复现执行 `--interpret run --meta-list-type input.Elements --tuple-type input.Tuple` 返回 bool true（`/tmp/luna-tools-o7n25emc/result.json`）。

新增剩余问题 `repros/comptime-type-packet-enumeration.fl`：合法 tuple_enumerate 的元素模式仍只取得 Type 域而缺少字段身份，后续 Selected 注解失败（当前驱动复验 `/tmp/luna-tools-izi8n_53/result.json`）。未通过把普通循环元素绑定当成 const 来绕开其存储语义。范围循环回归已经通过，但不据此宣称上述路径已完成。任意工厂返回值、一般控制流及自定义 pattern view 的部分字段分析仍待补齐。未重复完整示例目录；改动保留 dev，未提交。

## 部分已知记录与枚举的类型字段（2026-10-24）

两个复现 `repros/comptime-partial-record-packet.fl`、`repros/comptime-partial-variant-packet.fl` 原先均在 Selected 的类型成员注解处失败（`/tmp/luna-tools-g05wv3zg/result.json`、`/tmp/luna-tools-o0gjx5n2/result.json`）。字段证据分析现沿已检查的 Record／Variant 构造继续处理，不再限于 Tuple。具名记录核对声明身份；枚举另核对标签；匿名结构的内部构造按域中的字段名对应。不能只因为字段数相同就共享字段证据。

已知构造标签或已知字面量可排除不可能匹配的 or 候选，未知字段及 guard 保持可能；其余候选只交付共同的绑定身份。分析把候选可能性与成功路径的字段证据分开：检查成功分支时保留结构字段信息，但不能因此删除失败路径、调用或资源行为。完整值分析沿用相同的可能性信息，旧调用者仍取得原来的绑定映射。

源码正例覆盖泛型字段默认值、记录构造项／模式字段顺序变化、记录内的枚举、不同变体载荷位置、已知标签与字面量筛选，以及带状态闭包的字段初始化。最后一例将构造项书写顺序逆转，验证仍按 struct 声明顺序各执行一次。拒绝例覆盖错误字段域、未知字段、两个仍可能候选中的冲突 Type、错误枚举声明、非 bool guard、非发散 else 及失败分支访问成功绑定。最后一种错误在 HIR 名称解析阶段拒绝，而非 elaboration；测试按实际阶段检查。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test comptime_memory --test open_functions --test cmtt --no-fail-fast`：548／17／389／523 项，共 1,477 项通过，退出 0（`/tmp/luna-tools-1j_ytqfx/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-sfgm5ok6/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization`：扩展后的例子 O0/O3 原生构建及执行通过（`/tmp/luna-tools-fgw7igb2/result.json`）。

使用私有驱动 `/tmp/luna-host-examples-zxt2qn_i/luna` 经 reproduce.py 对两个原复现执行 `--interpret run`，均返回 bool true（`/tmp/luna-tools-k8dyzrxc/result.json`、`/tmp/luna-tools-hwigl3i6/result.json`）。这没有证明任意工厂调用结果、独立部分聚合别名或自定义 pattern view 的字段推导已完成；匿名记录经普通调用返回的路径也不能据此宣称完成。未重复完整 33 项目录；dev 改动未提交。

## 部分已知 const Tuple 的类型字段（2026-10-24）

`repros/comptime-partial-type-packet.fl` 在普通 comptime 函数中同时解构已知 Type 和尚未求值的 bool 参数。原先整个 Tuple 无法冻结，导致已知字段也丢失，后续成员注解报 type expression requires comptime elaboration not yet implemented（`/tmp/luna-tools-m7_me45g/result.json`）。现在从已检查构造逐字段提取证据，并让类型注解读取仍在局部作用域中的已知 Type 身份。

完整字段沿用既有匹配证据分析；部分已知 Tuple 递归处理结构、as、guard、Then 的第一模式及候选绑定 remap，or 仅保留所有候选共有的值。未知字段和整个未知聚合不构造替代值；不调用字段中的函数，不读内存，也不删除 initializer 或 guard。深度与工作预算限制继续生效。该路径仍限于 comptime 正文中的 const；普通 let 的存储和闭包捕获不因此被替换。任意自定义 pattern view、部分已知聚合的独立别名传播及具名函数调用期环境仍待后续工作。

回归包含普通参数 true／false 的独立调用、直接 Type 注解及泛型成员、嵌套／as／or／guard 模式；有状态闭包字段及后续调用验证 initializer 执行一次。拒绝例覆盖错误字段、元数、Payload 域、非 bool guard、非发散 else、不同候选给出的冲突 Type，以及 Identity<bool> 不等于 bool；失败保留原输出。更新了 `examples/host-type-packets.fl`。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 900 -- test -p luna --test host --test comptime_memory --test open_functions --no-fail-fast`：546／17／389 项，共 952 项通过，退出 0（`/tmp/luna-tools-nw5y5tdg/result.json`）。补入 opaque normal-form 拒绝对照后，`python3 -B development/check.py --timeout 600 -- test -p luna --test host partial_comptime_tuple --no-fail-fast` 两项通过（`/tmp/luna-tools-gozn394x/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-n_90m7sm/result.json`）。

`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization`：O0/O3 原生构建及执行通过（`/tmp/luna-tools-v5wbmfxa/result.json`）。使用该轮私有驱动 `/tmp/luna-host-examples-a_myrfzz/luna` 经 reproduce.py 执行原复现，解释结果为 bool true（`/tmp/luna-tools-b0dyrngv/result.json`）。完整 33 项目录沿用上节集成基线，没有重复全目录测试。dev 工作区未提交。

## Guarded Tuple 的成功路径来源（2026-10-24）

`repros/staged-member-guarded-tuple-alias.fl` 的分支内 Tuple 解构在带 guard 时，原先于后续成员注解报 receiver type 缺失（`/tmp/luna-tools-icfxyn3m/result.json`）。来源恢复不再仅接受没有 guard／else 的绑定：经过普通描述检查的 guard 只筛选成功延续，已检查为发散的 else 不向成功绑定提供另一组值。保留原 initializer、pattern、guard 与失败分支的执行及检查，不凭来源信息认定 guard 为真。

源码回归覆盖 guard 和 literal/as refutable pattern；状态闭包第一次调用使 guard 成功，第二次调用选择 bool 容器而非 u32 容器，验证 guard 恰好一次。反转 guard 后进入 `while true {}` 并以耗尽预算拒绝，证明没有跳过失败路径；非发散 else、非 bool guard 和失败分支引用成功绑定仍拒绝，旧产物保留。早先一个拒绝用例的裸 u32 改为显式 `.u32'builtin`，使其检验类型不匹配而非名称解析错误。

限制另存为 `repros/staged-member-guarded-return.fl`：nested const initializer 内 `else {return false;}` 当时仍报 return outside a function（当前诊断修正见下节），哪怕此次选择可能成功也会检查该分支。原普通函数体中直接 const 绑定的 runtime return 机制并未扩展到这里。本轮不把这项拒绝当作完整 guard 支持，也未自行改变跨阶段控制的语言契约。

工作目录 `/home/fin/luna`。最终 `python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --no-fail-fast`：host 542、open_functions 389、cmtt 523，共 1,454 项通过，退出 0（`/tmp/luna-tools-ja3vg1sw/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-shbpjh94/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` O0/O3 通过（`/tmp/luna-tools-n13c_jlh/result.json`）。完整目录基线仍是上节前一轮的 33 项集成验收，本轮没有重复全目录测试。

使用私有驱动 `/tmp/luna-host-examples-cp3nfls9/luna`，经 reproduce.py 的 `--binary`、`--interpret run --meta-list-type input.Elements --tuple-type input.Tuple` 复核：guarded-tuple-alias 返回 bool true（`/tmp/luna-tools-pih5lfa_/result.json`）；guarded-return 退出 1 并保留上述诊断（`/tmp/luna-tools-kkps4fkn/result.json`）。`git diff --check` 通过，dev 未提交。

## required comptime 返回边界的诊断（2026-10-24）

required initializer 的准备过程主动隔离外层 return／loop 上下文，调用接口交付编译期值，没有外层 runtime 控制转移结果。因此不能仅恢复外层 result 就实现跨阶段 return。正式文档明确了外层 const 绑定的失败 else 保持 runtime 控制位置，但未明确嵌套 initializer 中的 return 能否跨出 required computation；已请求作者确定该契约，当前不自行扩展或禁止这项语言能力。

当普通函数中的 required comptime 边界没有返回目标时，现在报告 `return across a required comptime boundary is not supported`。quoted code 中没有目标的 return 仍报告 `return outside a function`。拒绝例验证既有输出文件不被替换；正例验证 comptime 函数自身的 return、initializer 内闭包自身的 return，以及外层 const else 的 runtime return，均比较解释执行与 LLVM O0/O3。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 600 -- test -p luna --test host comptime_ --no-fail-fast`：5 项通过，退出 0（`/tmp/luna-tools-ejej3hiz/result.json`）。`python3 -B development/check.py --timeout 600 -- test -p luna --test test_templates ordinary_quotes_inside_template_bodies_do_not_inherit_test_returns`：1 项通过，退出 0（`/tmp/luna-tools-cefh15uq/result.json`）。这是诊断与既有控制边界回归，不表示跨阶段返回已完成；未重复完整示例目录。改动保留 dev 工作区，未提交。

## const Tuple 中的类型身份与存储边界（2026-10-24）

直接 `const (Selected,unused)=(Container,flag)` 原本可用；把同样的解构放入条件分支，随后返回 `Selected.Inner<bool>` 时会丢失成员来源。复现为 `repros/staged-member-branch-tuple-alias.fl`，修复前报告 `/tmp/luna-tools-q6dx8cvp/result.json`。描述阶段现沿已检查 Tuple 的字面构造、不可变别名及嵌套 tuple／as pattern 恢复依赖，使用 HIR 绑定身份和现有预算。没有将自定义 pattern view 假定为按位置映射，也没有用结构 Tuple 替代正式 normal-form 头。

来源补齐后，实例检查进一步暴露 const 块内绑定只保留 Type 域、没有保留已知身份。现在对 comptime 体内 const 的输入做现有 bounded known_normal_input 结构检查，再经 completed_subject_bindings 保留完成的模式输出。此操作不调用函数、不读取内存，未知整体仍为未知；原始初始化语句保留执行。阶段状态沿现有 block／函数／候选事务恢复路径隔离。

初版误把普通 let 纳入值记录，导致 Type 存储取址和闭包环境消失。六组检查明确失败：comptime_memory 两项、open_functions 一项（`/tmp/luna-tools-1n8fybsg/result.json`）。最终将新路径限定为 const；普通 let 即使处于 comptime，也保留其存储地址、Copy 权限、后续写入和捕获身份。这三个回归已恢复，不通过调整原测试预期掩盖问题。

正例包含直接／嵌套 Tuple、容器保存与两级别名、as pattern，检查解释执行和 LLVM O0/O3；拒绝例包含错误字段位置、元数、bool 冒充容器、越过局部作用域、错误 Payload 域及错误 Tuple 构造器签名，并保留已有产物。新增 `examples/host-type-packets.fl` 和运行器目录项，显式注册 Tuple 与类型列表构造器。

验证目录 `/home/fin/luna`。最终代码使用 `python3 -B development/check.py --timeout 600 -- test -p luna --test comptime_memory --test open_functions --test host --no-fail-fast`，17／389／540 项通过（`/tmp/luna-tools-twlj40nh/result.json`）；再检查 `--test cmtt --test inline_control --test newtypes --no-fail-fast`，523／249／211 项通过（`/tmp/luna-tools-6mvgslhr/result.json`），共 1,929 项，均退出 0。运行器单元测试 `python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p test_host_examples.py` 6 项通过（`/tmp/luna-tools-pssxzfre/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-fvv6ftdb/result.json`）。

`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py type-packets --compare-optimization` O0/O3 通过，报告 `/tmp/luna-tools-k6tl52pb/result.json`。以本轮私有驱动 `/tmp/luna-host-examples-aw9t073o/luna`，通过 reproduce.py 的 `--binary` 执行两个复现（加 `--meta-list-type input.Elements --tuple-type input.Tuple --interpret run`）均返回 bool true：分支版本 `/tmp/luna-tools-l02hshf1/result.json`，直接版本 `/tmp/luna-tools-3vxulw8s/result.json`。`git diff --check` 通过；dev 未提交。

## 条件成员别名与分支局部绑定（2026-10-24）

`repros/staged-member-conditional-alias.fl` 的 `const Selected=if flag {Container.Inner<bool>} else {Container.Other<bool>}; let value:Selected.Payload=flag;` 原先在成员注解处拒绝（`/tmp/luna-tools-pfgsgk70/result.json`）。来源分析现对 if、when、match 和块的结果路径取交集，沿各分支原始表达式保留待定成员依赖；没有正常结果或出现无依据的分支时不能凭该路径放行。已建立的描述检查、覆盖检查、可见性及具体实例域检查仍负责各自的语义。

进一步复现 `repros/staged-member-branch-local-alias.fl` 发现分支内 `const Local=…; Local` 在作用域退出后丢失来源（`/tmp/luna-tools-zr4po6dr/result.json`）。现按 HIR 绑定身份追溯该分支的不可变简单绑定初始化式，支持别名链和同名兄弟局部；不以名字合并作用域，不将可变／guard／解构绑定误作这种别名。工作队列对 `(节点身份, 是否正在选择成员)` 去重，避免共享条件别名子图指数展开，并扣除每个访问边和绑定扫描的预算。

正例对比解释入口和 LLVM O0/O3，覆盖两种成员构造、三种条件表达式、嵌套块、分支局部别名、多级别名及 24 层共享条件别名。拒绝例覆盖错误 Payload、混入 bool、缺失结果、非穷尽 match、遮蔽后的错误容器和越过兄弟作用域访问局部；失败保留原输出。const-match 目录示例新增相同的条件类型选择用法。本次没有接通外部函数 ABI，所需 Object／属性求值与函数类型检查仍记录在 frontend-checkpoint.md。

验证工作目录 `/home/fin/luna`。最终实现执行 `python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test inline_control --test newtypes --test cmtt --no-fail-fast`：host 538、open_functions 389、inline_control 249、newtypes 211、cmtt 523，共 1,910 项通过，退出 0（`/tmp/luna-tools-cza1b4ob/result.json`）。格式化退出 0（`/tmp/luna-tools-wgxrc6km/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization` O0/O3 通过（`/tmp/luna-tools-9n7pn33h/result.json`）。

普通 reproduce 默认缓存二进制已被清理，一次尝试报告 unavailable（`/tmp/luna-tools-sxdocz6d/result.json`），不计为源码失败。随后使用本轮示例驱动 `/tmp/luna-host-examples-l8vq2x7t/luna`，经 `python3 -B development/reproduce.py --timeout 30 --binary <上述驱动> -- <复现文件> --interpret run` 执行两个复现均返回 bool true，报告分别为 `/tmp/luna-tools-cjd37iuh/result.json`、`/tmp/luna-tools-foyjvuip/result.json`。`git diff --check` 通过；dev 工作区未提交。

## 连续 Not 的直接 NIR 执行（2026-10-24）

此前 64 层直接 NIR 解释失败已定位到构造器的 `budget.depth.min(64)`：它保护宿主调用栈，而非用户传入 256 没有传到解释器。保留这道保护，将 Not 链改成显式待处理节点栈，叶子只求值一次，再由内向外执行每个 Not。每节点仍扣除步数，BitVec 运算继续使用原计费与域检查；每个内层节点调用原 finish_evaluation，最外层由原 eval 交付，保留错误、return、effect transfer 与清理处理。

源码回归 32／63／64 层直接 Interpreter::new 均返回正确奇偶结果，原临时拒绝断言已删除。低层测试在 depth=2 下执行 96 层 bool 链并检查精确节点成本；耗尽步数后深度恢复为 0，未修改输入树，另一解释状态可重新执行。另测 BitVec 64 层值保持、中间节点错误域按原来源拒绝，以及 return 穿过链时不会被取反。不是通过提高预算或宿主栈大小解决，也未解除其他递归路径的保护。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 600 -- test -p comptime_interpretor --lib`：78 项通过，退出 0（`/tmp/luna-tools-s9o2gcrm/result.json`）。源码定向回归通过（`/tmp/luna-tools-l98l4h25/result.json`）。格式化退出 0（`/tmp/luna-tools-06wwuhp3/result.json`）。集成回归 `python3 -B development/check.py --timeout 600 -- test -p luna --test inline_control --test inline_returns --test host --no-fail-fast`：host 536、inline_control 249、inline_returns 2 项全部通过，退出 0（`/tmp/luna-tools-uvnxvmu_/result.json`）。连同解释器单元测试共 865 项。`git diff --check` 通过；dev 未提交。下节的直接 NIR 限制描述仅保留为修复前历史。

## 未注解成员别名的来源与描述栈（2026-10-24）

上轮 `const Selected=Container.Inner<T>` 的未注解别名缺口已修复。Checker 增加仅供开放描述使用的 pending-member 来源集合，沿不可变简单绑定与别名链传递；不会据此构造 Type、授权可见性或跳过实例检查。描述中的成员投影、参数应用与注解依赖扫描复用同一来源识别。开放函数、块、分支／模式、循环和临时阶段上下文与既有阶段集合同步保存／恢复，包括 inline_loops 的独立恢复点。拒绝例覆盖错误域、私有成员、泛型函数值不能冒充容器，以及局部同名 bool 不能继承外层来源；正例覆盖别名链、遮蔽退出和下一层泛型成员。

扩大 inline 回归发现 `nested_open_operator_descriptions_remain_bounded_and_executable` 栈溢出（`/tmp/luna-tools-ja8w26sg/result.json`）。将 let 的描述检查移出递归表达式函数后，原 32 层通过；扩展 64 层仍溢出（`/tmp/luna-tools-b2wyj4rk/result.json`），进一步将连续 Not／Neg 描述改为迭代下降和逆序检查，保留每层上下文、协议分派、原深度限制及步数扣减。未增加线程栈或生产默认预算。64 层现在编译成功，并通过 LLVM O0/O3 和 CLI 解释路径。

直接 NIR 解释仍有独立限制：64 层在 Interpreter::new 的默认预算及显式 depth=256 下均返回 DepthExhausted。增加解释深度并未解除该拒绝（`/tmp/luna-tools-74p1wtoe/result.json` 中的测试预期失败）；当前测试将其明确标作未完成路径，而非改口宣称直接解释成功。`repros/deep-open-unary-interpreter.fl` 保存相同源结构，直接 API 复现入口在 inline_control 测试；CLI 经其执行流程可成功解释这个文件，二者不可混称。

验证目录 `/home/fin/luna`。最终编译器实现下，六组联合命令 `python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test newtypes --test inline_control --test inline_returns --no-fail-fast` 中 host 536、open-functions 389、CMTT 523、newtypes 211、inline-returns 2 项通过；整体因上述直接 NIR 解释预期失败退出 101（`/tmp/luna-tools-74p1wtoe/result.json`）。仅修正该测试对现有限制的表述与断言后，`python3 -B development/check.py --timeout 600 -- test -p luna --test inline_control --test inline_returns --no-fail-fast` 的 249／2 项通过，退出 0（`/tmp/luna-tools-yu_425bo/result.json`）。按测试组去重为 1,910 项，包含明确标注的限制诊断测试。

原成员别名复现 `python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-generic-member-inferred-alias.fl --interpret run` 返回 bool true（`/tmp/luna-tools-dwkts2j5/result.json`）。深层源文件的 CLI `--interpret run` 同样返回 bool true（`/tmp/luna-tools-so4twsgv/result.json`）。const-match 改用未注解 Selected，再取得 Value，`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization` O0/O3 通过（`/tmp/luna-tools-5khefmdh/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-l5zox_j7/result.json`），`git diff --check` 通过，dev 未提交。以下参数化成员小节中的未注解别名失败是修复前记录。

## 待定容器的参数化成员（2026-10-24）

`repros/staged-generic-type-member.fl` 的 `Container.Inner<bool>.Payload` 原先报 `explicit instantiation requires a resolved generic function`（`/tmp/luna-tools-q1vyr2l2/result.json`）。开放描述现在单独处理容器身份待定的成员实例化：先检查成员头和各实参表达式，保留完整参数列表，结果域暂不假定为 Type；实际签名、参数路由、默认值和形成要求由具体实例检查。外层成员投影的根识别可穿过参数应用。已知容器／泛型成员继续走原检查路径，不提前对缺少实参的泛型成员单独求值。

正例覆盖直接投影、混合 Type／bool 参数、声明过的命名参数覆盖，以及显式 Type 中间别名，均验证 bool true/false 实例和 O0/O3。拒绝例覆盖过多参数、未知参数名、缺少形成约束证明、跨模块私有中间成员和 runtime 实参不能进入编译期类型形成。Tuple／normal-form 头的语义未改变。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test newtypes --no-fail-fast`：host 533、open-functions 389、CMTT 523、newtypes 211 项通过，退出 0（`/tmp/luna-tools-tjou7_vm/result.json`）。原复现用 `python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-generic-type-member.fl --interpret run` 返回 bool true（`/tmp/luna-tools-_1lh359r/result.json`）。const-match 新增 Settings.Selected<U> 并通过参数化成员取得局部指针注解，`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization` 的 O0/O3 构建执行通过（`/tmp/luna-tools-7i03c61k/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-g80638xb/result.json`），`git diff --check` 通过，dev 未提交；本次未重跑完整示例目录。

未注解中间别名仍有独立缺口：`const Selected=Container.Inner<bool>; let value:Selected.Payload=flag;` 在定义端只有未知描述，后续成员访问无法恢复其待定来源。保存 `repros/staged-generic-member-inferred-alias.fl`，`python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-generic-member-inferred-alias.fl --interpret run` 退出 1，报 `field projection requires an established receiver type`（`/tmp/luna-tools-wi329g8o/result.json`）。后续需继续传递来源或成员结果域约束；不能把未知结果一律当成 Type。

## Type 参数成员的函数体依赖识别（2026-10-24）

没有局部 const 的函数体也可能需要实际 Type 参数：`let pointer:*Container.Payload=value` 或 `let action:fn(Container.Payload)->Container.Payload=callback`。旧扫描只看 let 顶层是否为 projection，因此组合注解在签名已具稳定符号域时漏进普通函数检查；原复现报类型表达式尚未实现（`/tmp/luna-tools-9825i805/result.json`）。现在扫描遍历到的每个 projection，确认其根为本函数的非 runtime 参数且域为 Type，便保留函数体到实例检查。普通值字段、未知域参数和嵌套函数自有上下文不按此规则改变。

正例覆盖指针注解、回调参数／结果注解、不同 Type 实例的成员常量。直接从 comptime 成员进入 runtime 仍要求绑定 Splice 协议：初次无协议测试正确拒绝（`/tmp/luna-tools-ucgnamvh/result.json`），正例补齐明确协议，并保留未绑定协议的拒绝对照。另检查第二个实例的回调域不匹配和跨模块私有成员；失败保护已有原生产物。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test newtypes --no-fail-fast`：host 531、open-functions 389、CMTT 523、newtypes 211 项通过，退出 0（`/tmp/luna-tools-64qkbva2/result.json`）。`python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-parameter-composite-annotation.fl --interpret run --copy-protocol Duplicate --drop-protocol Retire` 解释执行返回 bool true（`/tmp/luna-tools-4_s7pb5d/result.json`）。const-match 新增 Type 参数决定回调注解的例子，通过 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization` 的 O0/O3 构建执行（`/tmp/luna-tools-ukuntp24/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-qxxfw5i2/result.json`），`git diff --check` 通过，dev 未提交。

## 待定类型的指针与 Tuple 组合（2026-10-24）

局部注解的待定输入扫描扩展到构造内部，支持 `*Alias`、`Tuple<Alias,bool>` 和 `Tuple<Container.Payload,bool>`。构造表达式先按 Type 域检查，不能因外层构造结果域为 Type 就声称其输入身份已完成；整个源注解在具体实例重新检查。初值已经确定的指针／Tuple 结构保留在普通描述中，后续读取或解构不再无故丢失结构。Tuple 使用正式 normal-form 头及 meta.List 输入注册，元素通过 tuple pattern 解构；不引入括号 tuple 类型或数字点投影语法。

`repros/staged-type-factory-pointer.fl` 原先在别名处失败（`/tmp/luna-tools-o7xeapea/result.json`）。描述层现在可对含签名 substitution 参数的指针给出读取结果域，仍不发布 ReadPointer NIR；具体实例执行原有布局、Copy、来源及资源检查。用例声明 Duplicate/Retire，按正常约束要求泛型输入可复制；另有移除前置 trait 约束的名义 Owner 对照，仅提供 Copy 实现的版本成功，非 Copy 版本必须在具体检查拒绝并保留旧产物。错误指针注解域仍拒绝。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test newtypes --no-fail-fast`：host 529、open-functions 389、CMTT 523、newtypes 211 项全部通过，退出 0（`/tmp/luna-tools-nriwkx84/result.json`）。指针复现用 `python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-type-factory-pointer.fl --interpret run --copy-protocol Duplicate --drop-protocol Retire` 解释执行通过，结果 bool true（`/tmp/luna-tools-_uk9_0m8/result.json`）。组合类型正例均检查 LLVM O0/O3。

const-match 的配置值示例改用指针注解与读取，运行器为其添加成对 RESOURCE_OPTIONS；未提供协议的初次构建正确拒绝 host 数据指针（`/tmp/luna-tools-q1lwhdrr/result.json`）。补齐显式协议后的 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization` O0/O3 构建执行通过（`/tmp/luna-tools-_vbgrsqo/result.json`）；运行器单元测试 `python3 -B development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p test_host_examples.py` 通过（`/tmp/luna-tools-9ny22hak/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-_c2knc5d/result.json`），`git diff --check` 通过，dev 未提交。

## 待定 Type 成员经局部别名传递（2026-10-24）

`const Alias=Container.Payload; let result:Alias=value;` 原先在 Alias 注解处报 `type expression requires comptime elaboration not yet implemented`（`/tmp/luna-tools-y94f5_ds/result.json`）。开放函数现在识别阶段局部别名的待定描述，在实例提供实际 Type 后再检查原注解。别名具有显式 Type 域却没有完成的类型身份时同样保留待定；`const Alias: Type=…` 不再因注解本身就强制提取尚未完成的初始化值证据。既有完成证据仍走原路径，未知描述不进入已检查 NIR。

回归覆盖未注解别名、两级别名链和显式 Type 注解，各以 bool/u32 两种实例在 LLVM O0/O3 执行。未注解／显式注解两种形式都检查非 Type 常量、错误域与 Identity normal-form 不能等同其参数的拒绝，并保护已有输出文件。const-match 的 configured_value 示例改为显式 Type 别名后再用于注解。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test newtypes --no-fail-fast`：host 527、open-functions 389、CMTT 523、newtypes 211 项全部通过，退出 0（`/tmp/luna-tools-4tcpc09u/result.json`）。`python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-type-factory-alias.fl --interpret run` 返回 bool true，退出 0（`/tmp/luna-tools-bbkqrw3k/result.json`）。更新后的 const-match 示例通过 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization`，O0/O3 构建执行通过（`/tmp/luna-tools-8x0stqpj/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-l1sbkb00/result.json`），`git diff --check` 通过，dev 未提交。

## 阶段成员用于局部类型注解（2026-10-24）

`const Container=Make(T); let result:Container.Payload=value;` 现在可在开放函数中使用。原复现 `repros/staged-type-factory-annotation.fl` 报 `type expression requires comptime elaboration not yet implemented`（`/tmp/luna-tools-drmni1p8/result.json`）。当注解根绑定已有 Type 域而没有容器身份时，定义端检查其成员表达式并保留未知描述；具体实例仍按原源注解检查，不生成假定的 Type，也不丢弃注解。已知容器证据的注解继续走原路径，避免额外执行 Type 计算。

只有显式 comptime 参数、没有 const／inline 控制流的函数原先可能因签名已有稳定符号域而不进入开放函数检查。现在函数体依赖阶段参数的成员注解也被识别为需要输入的工作。定向回归覆盖直接、嵌套、参数接收者及 bool/u32 多实例；拒绝用例带同模板成功对照，检查错误域、跨模块私有成员、非 Type 常量及 Identity normal-form 不能展开为其参数。私有成员测试使用不同模块，同模块可访问情形没有被误判为拒绝要求。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test newtypes --no-fail-fast`：host 525、open-functions 389、CMTT 523、newtypes 211 项全部通过，退出 0（`/tmp/luna-tools-2upie9we/result.json`）。`python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-type-factory-annotation.fl --interpret run` 返回 bool true，退出 0（`/tmp/luna-tools-y0gkmizw/result.json`）。更新后的 const-match 示例通过 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization`，O0/O3 构建执行通过（`/tmp/luna-tools-w1nq_nk8/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-hd7f2v2v/result.json`），`git diff --check` 通过，dev 未提交。

## 阶段 Type factory 的延迟成员选择（2026-10-24）

上轮保留的 `repros/staged-type-factory-member.fl` 已修复。定义端知道局部 const 或显式 comptime 参数的域是 Type，但没有已完成容器证据时，不再调用要求具体容器身份的字段检查；它交付普通未知描述，待实例提供 Type 后重新检查成员和结果。未知描述不进入已检查 Term/NIR。根绑定已明确为 Type 的嵌套链可以继续保留；没有为任意未知域的值增加字段访问能力。

首次将所有 staged 未知接收者一律延迟，导致两个既有拒绝测试错误通过（`/tmp/luna-tools-g5pfvo7l/result.json`）。最终限制未知链传播必须有 Type 根证据，恢复 `fn(U.missing)->T` 等未建立接收者域的定义端诊断。回归另覆盖每实例返回不同名义容器、嵌套类型成员、显式 comptime Type 参数，以及私有／缺失成员、第二个实例的错误结果域、runtime 输入不能提升到 comptime；拒绝时保留已有输出文件。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test newtypes --no-fail-fast`：host 522、open-functions 389、CMTT 523、newtypes 211 项全部通过，退出 0（`/tmp/luna-tools-175qrmsw/result.json`）。`python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-type-factory-member.fl --interpret run` 退出 0，结果 bool true（`/tmp/luna-tools-03rf2gzs/result.json`）。更新后的 const-match 示例通过 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization`，O0/O3 构建执行通过（`/tmp/luna-tools-xll8obau/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-6qql184q/result.json`），`git diff --check` 通过，dev 未提交。以下小节的未知容器缺口记录是修复前状态。

## 嵌套成员的阶段值输入（2026-10-24）

`const Container=Outer<flag>; const result=Container.Inner.FLAG;` 现在保留已知容器头和待定值输入，到阶段实例提供 flag 后完成构造及成员求值。原实现为了分类中间接收者强求已完成 Type，报 `local constant used as a type must contain a completed Type value`（`/tmp/luna-tools-46c5ro94/result.json`）。表达式路径改为沿局部 Type 根识别候选链，再从保留准备代码的容器表达式获取输入；中间结果为普通值时仍交回值投影，Never 的控制流也保留。

不能对任意字段链做 speculative expression checking：首次扩大路径时误触了枚举分组和别名参数中的提前返回，相关测试失败（`/tmp/luna-tools-nk4c7wji/result.json`）。最终实现仅对已确认局部 Type 根启用新候选路径，原直接／延迟分类仍沿用。回归检查外层 bool 输入捕获、内部自有 bool 输入、固定值输入及普通函数中已经完成的 Type factory 调用；示例 const-match 增加 Switch.Nested.ENABLED。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test newtypes --test open_functions --test cmtt --no-fail-fast`：host 519、newtypes 211、open-functions 389、CMTT 523 项全部通过，退出 0（`/tmp/luna-tools-3u4m3_51/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization` O0/O3 构建执行通过（`/tmp/luna-tools-l33eqbqc/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-uu5ytvyd/result.json`），`git diff --check` 通过，dev 未提交。

尚未支持阶段函数定义端通过任意 `Make(flag)` 返回未知容器后选择成员。保存 `repros/staged-type-factory-member.fl`，本轮用 `python3 -B development/reproduce.py --timeout 30 -- development/repros/staged-type-factory-member.fl --interpret run` 复验，退出 1，仍报字段投影不支持（`/tmp/luna-tools-bmcvce_u/result.json`）。这是需继续实现的待定成员选择，不能把上述已知构造头的修复称为任意 Type factory 成员访问完成。

## 深层成员链的重复 elaboration（2026-10-24）

新增 20 层嵌套局部 Type 成员链暴露默认预算下的 `constant copy budget exhausted`（`/tmp/luna-tools-w2v03pjd/result.json`，退出 101）。分类接收者已经得到中间 Type，而后续类型／常量成员选择又从源码递归求解相同前缀，导致重复工作随深度累积。现在把中间结果交还给同一次选择使用；证据不跨 invocation 或 checker 环境缓存，预算不变，normal-form 头不展开，每层可见性仍单独检查。表达式成员路径仍生成容器准备代码，不能仅用逻辑 Type 取代其求值行为。

回归覆盖 20 层直接注解与 Type 常量别名两种写法，各以 LLVM O0/O3 构建并原生执行，且同一程序同时调用 true/false 两个阶段实例。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test newtypes --test normal_patterns --test open_functions --no-fail-fast`，host 518、newtypes 211、normal-patterns 42、open-functions 389 项通过，退出 0（`/tmp/luna-tools-d5tju2ob/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-yplrwmcz/result.json`），`git diff --check` 通过。未重跑完整示例目录，dev 未提交。

## 局部 Type 的嵌套成员（2026-10-24）

局部 Type 接收者现在可逐层选择嵌套类型成员，例如 `const Container=Outer<bool>; let value:Container.Inner.Payload=flag;`。中间类型的自有参数和外层参数分别保留；表达式中的同一成员链也可作为后续 Type 别名。每层选择仍使用原使用位置的可见性候选，私有中间类型或最终成员不能被绕过；普通 record 的同名嵌套字段保持值投影。解析期已有直接／延迟成员分类的路径继续使用原分类。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test newtypes --test normal_patterns --test open_functions --no-fail-fast`，host 517、newtypes 211、normal-patterns 42、open-functions 389 项全部通过，退出 0（`/tmp/luna-tools-j1db1ewf/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization` 验证新增嵌套成员示例 O0/O3 构建和执行，退出 0（`/tmp/luna-tools-s9g1yvv_/result.json`）。全 workspace 格式检查通过（`/tmp/luna-tools-pwb6kcul/result.json`）。本次没有重新执行完整示例目录；上方完整目录结果是此前集成基线。dev 未提交。

## 普通 Type 常量与名义成员（2026-10-24）

阶段函数定义端现在也为普通 `const T=bool` 保留 Type 证据，而不只处理带 guard／else 的 const。原复现错误为后续注解 `type expression requires comptime elaboration not yet implemented`（`/tmp/luna-tools-05ffkz1l/result.json`）。已知 Type／类型列表仍走共享结构证据提取；别名引用显式阶段 Type 参数时保留其绑定层级，各调用实例独立替换。

局部 Type 值接收者的类型／常量成员选择可复用 resolver 已保存的使用位置候选表。仅在接收者已确定为 Type 时进入该路径，不改变普通数据字段访问，也不扩大私有成员可见性。struct／enum 的成员类型及别名链、newtype 身份和基于值参数的成员常量均有验收。内部 NominalType 模式没有被当作新的源级模式语法。

容器头已知而值输入尚待 invocation 时，保留已检查的名义类型构造及其准备语句，供成员选择使用，不把它谎报为完成的 Type。这进一步暴露了复制代码时把外层阶段参数重命名成未绑定临时变量的问题（`/tmp/luna-tools-qd6cnosn/result.json`）。复制现在先按预算收集自由绑定，将其保持为原词法身份，只重新分配内部绑定；闭合常量快照的复制行为不变。runtime 输入仍受阶段准入检查。

7 个别名／成员正例以及独立的 bool/u32 阶段 Type 别名用例均在 O0/O3 验证；拒绝例检查私有成员、newtype／normal-form 不被替换为其表示，以及直接 runtime 参数和局部转发不能形成编译期容器。失败时保留已有输出文件。`examples/host-const-match.fl` 增加 Settings<bool> 的局部 Type 别名及成员类型访问。

验证目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test newtypes --test cmtt --no-fail-fast`：host 513、CMTT 523、newtypes 211、open-functions 389 项全部通过，退出 0（`/tmp/luna-tools-7zdq6vr0/result.json`）。后加的 `test -p luna --test host open_constant_formation_cannot_promote_runtime_inputs` 两个子场景通过，退出 0（`/tmp/luna-tools-8lnk_089/result.json`）。更新后的 `host_examples.py const-match --compare-optimization` 经 command.py 执行，O0/O3 通过（`/tmp/luna-tools-hoesnvg6/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-4e051ncu/result.json`），`git diff --check` 通过。dev 未提交。

## 条件 const 的 normal-form 类型模式（2026-10-24）

条件 const 现在把已知类型结构传给已有的 pattern_observed 路径：例如 `const Box<T,value> = Box<bool,true> else {return false;}` 按 Box 的参数顺序，用已知 bool 确定 value 的域。原失败为 `dependent normal-form value patterns require contextual pattern elaboration not yet implemented`（`/tmp/luna-tools-2yozbsfh/result.json`）。定义端模式探测也复用已经按 comptime 检查的原输入，不再只传入域相同的匿名占位值；临时输入映射在成功和失败时都移除，不触发再次 invocation。

共享 Type 证据提取扩展到显式 normal-form 参数和其中的 Tuple 类型视图。仅在实际头与模式头一致时向下投影，按绑定的 Type 域交付已有描述；不会调用类型函数体，不会为不同头或未知结构发明证据。普通聚合、类型列表及已有 CMTT 路径继续使用同一受预算限制的结构检查。任意未知 Type 主语上的依赖模式推导仍不属于本次完成范围。

新增 5 个 O0/O3 正例覆盖依赖值参数、guard、阶段函数中的类型注解、嵌套 normal-form 及 Tuple 类型模式。3 个拒绝例检查不同头、Identity 包装不展开、实际 u32 域不接受 bool，失败时保留已有产物。`examples/host-const-match.fl` 新增 Descriptor 的类型参数／值参数解构，并在 bool 实例中原生执行。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --test normal_patterns --no-fail-fast`，host 509、CMTT 523、normal-patterns 42、open-functions 389 项全部通过，退出 0（`/tmp/luna-tools-xv17xmmh/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization`，O0/O3 通过，退出 0（`/tmp/luna-tools-0o2exa9h/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-qx1882ns/result.json`），`git diff --check` 通过。dev 未提交。

## 条件 const 的聚合与 Type 证据（2026-10-24）

修复 open definition 检查丢失已知 Type 的问题。此前条件 const 用只保留域的占位输入检查模式，`const T if T == bool = bool else {return false;}; let value:T=flag` 在阶段函数定义中会报告 `type expression requires comptime elaboration not yet implemented`（原失败 `/tmp/luna-tools-uexreuoi/result.json`）。现在将 CMTT 已有的结构化 Type／类型列表证据提取独立为共享 helper：从原检查输入的绑定、Tuple 和 record 投影中保留已有证据，交付给候选绑定，再检查 guard 与后续正文。普通值仍使用抽象证据，不执行 initializer 或选择分支；normal-form 头不展开。

9 个正例覆盖 Tuple 成功／失败与 guard、Type 相等 guard、阶段函数的 Type 注解、混合 Type／值的 Tuple 解构、不同布尔实例以及相同 normal-form 头的相等。3 个边界例检查不把 Identity<bool> 当作 bool、不把 u32 值当作已知 bool，以及不同头缺少相等证据时保留 TypeContractUnresolved。后一用例不是已证实的不等：最初测试错误地期望它选择 false，已修正测试而没有放宽求解器。`examples/host-const-match.fl` 增加阶段函数中的 Type 绑定。

验证目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test cmtt --no-fail-fast`：CMTT 523、open-functions 389、host 506 项通过，前述错误预期使整体退出 101（`/tmp/luna-tools-bno0z7ny/result.json`）。修正该用例后，`test -p luna --test host conditional_constants_keep_tuple_and_type_evidence` 的全部 9 个子场景 O0/O3 通过（`/tmp/luna-tools-hj3otvmo/result.json`）；`test -p luna --test host constant_type_evidence_preserves_normal_form_identity` 的 3 个边界例通过（`/tmp/luna-tools-rex8eulb/result.json`），均退出 0。更新后的 `host_examples.py const-match --compare-optimization` 经 command.py 执行，O0/O3 通过（`/tmp/luna-tools-r4472hr8/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-anhhy5m6/result.json`），`git diff --check` 通过。dev 未提交。

## runtime 局部 const 的模式、guard 与 else（2026-10-24）

局部 ConstDecl 现在与 let/var 一样保存四个子节点，else 在成功绑定建立之前 lowering；AST visitor 的元数据与实际布局同步，正式 syntax 区分局部 const pattern 与模块 const id。数据 const 的 initializer、模式及 guard 经现有 comptime interpreter 匹配一次，以内部结果包交付成功绑定或失败选择；没有重新实现匹配器。失败时交付已经检查的 runtime else，保留原 return／break／continue 目标。不可反驳性、失败分支发散、不可变绑定和阶段检查继续生效；模式失败不执行 guard，成功 guard 对拥有的闭包环境所做更新随绑定一起冻结。

对于带阶段输入的 runtime 函数，open definition 只形成模式／guard 的类型和阶段证据，实际 const 选择留到输入完成后。定义端的探测绑定不再触发常量 invocation；实际 initializer 单独按编译期上下文检查，失败分支继续按 runtime 上下文检查。quoted code 和 generator 数据的条件绑定仍是独立未完成路径。

新增 14 个正例覆盖直接成功／失败、捕获成功绑定的 guard、comptime helper、外层遮蔽、循环 break／continue、显式阶段输入和计算 initializer／guard、闭包 guard 的状态更新、跳过未匹配 guard，以及 Type 常量输出。7 个拒绝例覆盖 runtime 参数或 runtime 调用进入 guard、缺失 else、不发散 else、候选绑定泄漏、写不可变绑定，以及未选 else 的错误类型。另一个资源协议用例检查失败 break 执行 Watch 析构。所有正例在 LLVM O0/O3 检查结果。新增 `examples/host-const-match.fl`，目录现有 32 个示例；本轮仅运行新增示例，没有重跑完整目录。

验证目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --test frontend_cli --no-fail-fast`：frontend 201、open-functions 389、host 503 项通过；新增 guard 跳过用例的源码语法错误导致整体退出 101（`/tmp/luna-tools-8t1qzws5/result.json`）。修正用例后，`test -p luna --test host conditional_local_constants` 2 项及其全部子场景通过，退出 0（`/tmp/luna-tools-wc5otpj7/result.json`）。后加的资源用例也修正了指针赋值分隔后，通过 `test -p luna --test host conditional_local_constants_failure_preserves_runtime_cleanup`，退出 0（`/tmp/luna-tools-20te0ty0/result.json`）；这些修正只涉及 fixture。
- `test -p hir -p parse -p resolve --no-fail-fast` 经同一入口执行：114 项通过、3 项既有 doctest 忽略，退出 0（`/tmp/luna-tools-lh58i87_/result.json`），包含 ConstDecl 四子节点与两种 dump 一致性检查。
- `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py const-match --compare-optimization`：O0/O3 构建和执行均通过，退出 0（`/tmp/luna-tools-h6uik8bs/result.json`）。
- 全 workspace 格式化通过（`/tmp/luna-tools-66a7a3r2/result.json`），`git diff --check` 通过。dev 未提交。

## 递归调用与局部阶段环境（2026-10-24）

局部函数递归调用、兄弟函数互调和函数值转换现在从当前局部定义的固定输入前缀取得外层阶段环境。此前再次构造局部实例时读取父函数的 Bound 参数位置，该位置在当前函数中可能已是另一个 runtime 参数，导致 `expected runtime, got comptime-value`；原始复现 `repros/block-local-staged-recursion.fl` 的失败证据为 `/tmp/luna-tools-hzxvmmtp/result.json`。现在先按源绑定身份查询当前定义继承的环境，只有处在父函数本身时才使用父参数绑定。该修改不修改调用参数的阶段，也不通过额外推导覆盖固定环境。

8 个正例逐一比较解释器与 LLVM O0/O3：布尔阶段值递归、类型阶段输入递归、兄弟函数互调、返回递归函数值、多层阶段输入、兄弟函数的上下文函数值、带自有类型参数的显式递归，以及局部 comptime 互调后的 const 初始化。3 个拒绝例检查递归、兄弟调用和上下文函数值均不能把固定外层 T 重推导为 bool，并保留已有输出文件。`examples/host-local-generics.fl` 的阶段工厂现在返回可递归调用的函数。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test open_functions --no-fail-fast`：500+389 项全部通过，退出 0（`/tmp/luna-tools-zuihiwsz/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py local-generics --compare-optimization`：O0/O3 通过，退出 0（`/tmp/luna-tools-g19e5nx7/result.json`）。全 workspace 格式化通过（`/tmp/luna-tools-rf6751bh/result.json`），`git diff --check` 通过。改动留在 dev，未提交。

普通 comptime 函数中没有显式 comptime 标记的参数仍保持调用参数语义，不能直接套用显式阶段参数的 Bound 替换。其局部捕获复现为 `repros/comptime-local-ordinary-input.fl`；`python3 -B development/reproduce.py --timeout 30 -- development/repros/comptime-local-ordinary-input.fl --interpret run` 退出 1，报告 `local binding is unavailable in this scope`（`/tmp/luna-tools-3lhkd_7_/result.json`）。后续需建立该调用期词法环境，不应把所有普通参数强制改成显式阶段输入或跳过阶段／资源检查。

## 局部定义捕获普通阶段输入（2026-10-24）

显式 `comptime` 普通参数由 resolver 分配稳定声明身份，HIR 同时保留源绑定与定义映射，并按绑定身份排序 dump。局部函数的实例输入前缀现在包含父函数的子句环境和普通阶段输入；参数域按前序输入重新绑定。普通调用、返回函数值和多层局部定义保持该身份，bool/u32 或 true/false 不会合并成同一实例。外层类型等式与 trait bound 同步迁移参数位置，程序状态规格不送入 PTC。

带阶段输入且含 const 初始化的 runtime 函数先描述主体，在阶段输入完成后再计算常量。描述 const 初始化时使用 comptime 上下文；局部 comptime 函数仍遵循正常的阶段准入规则。方法接收者不会截断后续阶段参数的扫描。

12 个源程序覆盖 Type、布尔值、依赖前序 Type 的值、多层定义、同名参数遮蔽、函数值返回、外层类型要求、局部 comptime 调用及方法接收者。每例比较解释器与 LLVM O0/O3 结果。4 个拒绝例检查 runtime 参数不能作为静态声明环境、外层固定输入不可重推导／覆盖、普通值不能用作类型，并验证编译失败保留已有产物。`examples/host-local-generics.fl` 增加携带阶段类型和值的函数工厂。

验证工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 600 -- test -p luna --test frontend_cli --test host --test open_functions --test packages --no-fail-fast` 中 frontend 201、open functions 389、packages 48、host 497 项通过；新增方法用例因未配置资源协议失败，整体退出 101（`/tmp/luna-tools-rd82zh23/result.json`）。用例改为本地结构体并显式配置 Duplicate/Retire 后，`test -p luna --test host native_block_local_captures_staged_parameter_inputs` 的全部 12 个子场景通过，退出 0（`/tmp/luna-tools-4gml9e39/result.json`）；这次修正只改变测试输入，未改编译器实现。`test -p hir -p parse -p resolve --no-fail-fast` 113 项通过，3 项既有 doctest 忽略，退出 0（`/tmp/luna-tools-exk_8chu/result.json`）。更新后的示例经 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py local-generics --compare-optimization` 在 O0/O3 通过（`/tmp/luna-tools-wki_gwjz/result.json`；逐项 `/tmp/luna-host-examples-unetof0d/summary.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-ifrvmk_v/result.json`），`git diff --check` 通过。dev 未提交。

尚未形成的阶段参数域、依赖 runtime 输入的阶段域，以及 comptime 函数中没有显式 comptime 标记的普通参数捕获尚未在此接通；通用属性、捕获式效应和递归内存摘要的缺口保持原记录。

## 局部函数的外层子句环境（2026-10-24）

HIR 保存块内声明的 lexical parent，并在 S-expression dump 中按 child 身份排序输出。局部函数的类型参数表以前置隐式输入保留外层函数实例的子句参数；调用、显式实例化、上下文函数值与 open call 描述都固定这段输入，源码只供应局部函数自己的子句参数。NIR FunctionInstance 和最终实例化键携带完整输入，返回一个局部函数值也保留已选环境。外层类型要求和 trait 约束沿定义环境传递，程序状态前后置条件不作为 PTC 命题复制。

`examples/host-local-generics.fl` 展示局部 inline 函数与函数工厂跨 bool/u32 实例工作。新增 13 个正例均在解释器和 LLVM O0/O3 验证，覆盖多层局部声明、自己的显式／默认类型参数、外层相等约束和 trait bound、quote、局部 comptime 调用以及含 comptime 普通实参的局部函数。4 个拒绝例检查调用实参、显式类型参数及函数上下文不能重推导或覆盖外层输入，失败时保留已有输出文件。该实现不展开 normal-form 头。

扩大回归发现源码块扫描暴露了 AST 布局元数据错误：IfGuardPattern 与 AsBindPattern 被标为三子节点，实际 parser 只构造两个。读取相邻存储会导致循环遍历和栈溢出。现按实际节点契约修为 DoubleChildren，新增 guard/as/组合模式的访问计数与两种 S-expression dump 一致性验证；没有提高默认测试栈限制。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 600 -- test -p luna --test open_functions --test frontend_cli --test host --no-fail-fast`，389+201+496 项全部通过，退出 0（`/tmp/luna-tools-5i90py8x/result.json`）。`test -p hir -p parse --no-fail-fast` 经同一入口执行，97 项通过，3 个既有 doctest 忽略，退出 0（`/tmp/luna-tools-qjsgimed/result.json`）；包接口 48 项在本轮前一组检查中通过（`/tmp/luna-tools-lpsp9a1r/result.json`，该组整体曾因上述栈溢出失败，修复后的完整 open-functions 证据以前一命令为准）。独立示例 `host_examples.py local-generics --compare-optimization` O0/O3 两档通过（`/tmp/luna-tools-y11i00pj/result.json`；逐项 `/tmp/luna-host-examples-sk2hx168/summary.json`）。格式化退出 0（`/tmp/luna-tools-yvwbi837/result.json`）。dev 未提交。

该节最初记录的 `repros/block-local-staged-type-input.fl` 已由下面的普通阶段输入实现覆盖，保留文件作为回归输入。

## 块内声明、词法身份与 host 执行（2026-10-24）

普通函数与源级 test 的主体现在扫描块内静态声明，为每个源码块建立带 file/node 身份的独立作用域。lowering 进入对应作用域，并将该块静态名称表与顺序绑定栈按词法层级交织查询；内层声明可遮蔽外层参数或局部变量，兄弟块不共享声明。静态声明重名拒绝，let/var 的顺序遮蔽继续按原规则处理。定义自身从当前静态声明表取得 resolver 身份，不借用被同名局部变量遮蔽后的表达式解析结果。scope dump 保留 `(source-block (file ...) (node ...))`。

块内属性／修饰符沿完整包装链 lowering，已支持的 inline/pure/comptime/quote/unsafe 不再被笼统登记为未实现。未知属性继续等待 Object 求值并阻止 checked NIR。声明由包级签名和主体检查验证；已登记的 HIR Item 在语句位置成为 NIR `(unit)`，没有 runtime 初始化动作，也不跳过未使用定义的检查。缺少扫描身份时拒绝。源级 test 内可调用局部函数，运行时函数内嵌套 test 仍保留原拒绝边界；quote 表达式和模板生成的声明不由这次源码块扫描提前登记。

`examples/host-local-declarations.fl` 展示局部类型别名、不同块中的同名 inline/quote 函数及退出块后的外层名称恢复。命令 `python3 -B development/host_examples.py local-declarations --compare-optimization`；示例目录现为 30 项。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 600 -- test -p luna --test frontend_cli --test packages --test host --no-fail-fast`，201+48+494 项通过，退出 0（`/tmp/luna-tools-y1biv9ga/result.json`）。新增 host 测试用 11 个程序分别比较解释器与 O0/O3 原生输出，包括局部递归、兄弟块、外层参数／变量遮蔽、声明被顺序变量遮蔽后仍保持独立身份。前端拒绝例覆盖未使用错误主体、错误类型别名、重名声明、越域引用和未知属性。resolve 全 crate 15 项通过（`/tmp/luna-tools-0n46rd7c/result.json`）。新增独立示例 O0/O3 两档通过（`/tmp/luna-tools-rhh8wjyj/result.json`，逐项证据 `/tmp/luna-host-examples-96knhcxi/summary.json`）。dev 未提交。

## 最终清理环境中的残余契约验证

残余函数契约现在在清理足迹、所需类型和函数依赖全部收敛后验证一次，使用最终 runtime-instance／cleanup-access overlay；每个 graph function 均检查，包括没有独立根清理计划的高阶函数。验证仍在返回 Executable 和交付产物之前完成。未收敛轮次只用于计算完整清理计划，不发布计划或复用暂定环境的契约证明。

程序验证中的精确相等识别也改为按变量对索引两个不等式方向，不再逐行扫描整张表寻找反向行。只接受当前值版本、无常数偏移的 `c*(a-b)` 形式，并检查系数确为相反数；同向重复、非等比例系数和旧版本都不能提供相等证据。两个方向可有不同的正比例系数。返回去重且顺序稳定的相等值对，索引、系数运算和条目数仍受预算约束。

2026-10-24，在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 600 -- test -p elaboration --lib --test verification --no-fail-fast`：233+85 项通过（`/tmp/luna-tools-pdsemz6l/result.json`）。新增单元验证不同缩放、单向关系、系数不配对、旧版本、预算耗尽后的恢复，以及 64 个变量的相等链在 5000 steps 内完成识别。

`test -p luna --test host integer_minmax` 经同一入口执行，5 项通过（`/tmp/luna-tools-nw5k1jp_/result.json`）。原关系组合恢复到 1M fuel；析取／分支组合从上轮 4M 降为 2M 通过，在 1M 下仍耗尽（`/tmp/luna-tools-kvxjuzzr/result.json`），默认预算未变。`test -p luna --test host contract` 14 项通过（`/tmp/luna-tools-r_s67dvx/result.json`）；`test -p luna --test integer_contracts --test method_memory_contracts --no-fail-fast` 14+8 项通过（`/tmp/luna-tools-epsqqdl8/result.json`），涵盖残余算术重检及准备后的清理访问／高阶方法契约。上述通过命令退出码均为 0。

本次改变清理准备的检查时点，因此执行完整示例集对照：`python3 -B development/command.py --timeout 900 -- python3 -B development/host_examples.py --compare-optimization` 退出 0，27 个示例在 O0/O3 共 54 次构建和执行全部通过（`/tmp/luna-tools-ff5c0nbs/result.json`，逐项报告 `/tmp/luna-host-examples-ugq8nsbb/summary.json`）。格式化退出 0（`/tmp/luna-tools-t9qfjqn8/result.json`），`git diff --check` 通过；dev 未提交。

## 整数 min/max

新增 `min/max` 目标操作和 `math.min<T>/max<T>` 泛型包装，输入与输出必须是同一种 u8/u16/u32/u64/u128 或 i8/i16/i32/i64/i128。具体类型证据决定 LLVM umin/umax/smin/smax；source admission、实例化重检与目标注册均检查完整输入域，不按位宽抹去符号。浮点继续使用独立的 minimum/maximum 等接口。库 Scalar 与 CLI／包检查／示例入口同步至 38 项绑定。

程序流分析使用各实参求值时的区间快照：min 取两下界的 min 和两上界的 min，max 同理。单点结果可形成常量。另保留 min 结果不大于两个输入、max 结果不小于两个输入的不等式，以及结果等于其中一个实参的析取；通过现有复制证据导出，不把入口值与后续写入后的存储混同。跨调用契约使用 `a'old`／`b'old`，普通参数名仍表达出口局部状态。新增 `examples/host-integer-minmax.fl` 演示整数边界与具有 lower <= upper 前置条件的 clamp。

析取支持同时修复程序逻辑的两处缺口：按当前精确相等关系匹配复制／入口快照后的等价谓词；重放前提时保留区间无法表达的不等事实，并绕过当前被重放前提的“已经知道”捷径，继续检查矛盾。反证框架也保留否定目标中的非凸条件。简单局部值／字面量比较组成的 and/or 直接构造数学条件，避免无效分析分支；调用、内存读取和算术仍走原有短路分析，运行时代码不变。

2026-10-24，`python3 -B development/check.py --timeout 600 -- test -p elaboration --lib --test verification --no-fail-fast` 230+85 项通过（`/tmp/luna-tools-q7hhdn1c/result.json`）。新增单元用例先复现“排除一个相等选项后无法推出另一个”的缺口，再验证修复；另验证同范围和单向不等式不能替代相等证据，旧版本也不能用于条件匹配。

`test -p luna --test host integer_minmax` 经同一入口执行，5 项通过（`/tmp/luna-tools-qs2mrp0v/result.json`），新增 i8/u8/i128/u128 在 O0/O3 的析取契约、跨调用导出、条件分支选项消除，以及错误合取／错误析取拒绝、写入后的旧值断言实际失败。完整关系调用组合及新增析取组合使用显式 4M fuel；恢复 1M 时残余分析仍耗尽（`/tmp/luna-tools-9hrstccv/result.json`）。这是新增逻辑事实的性能回归，默认预算没有提高，不能据通过测试声称该开销已经解决。工作目录均为 `/home/fin/luna`，最终通过命令退出 0。

扩大回归发现 tuple 分量的独立前提被等价匹配误判为冗余（`/tmp/luna-tools-_0auur9h/result.json`：整数契约 13 通过／1 失败，方法内存契约 8 项、staged 聚合契约 31 项通过）。修复后，证明查询可以用精确相等关系，保存复制条件时则仍保留各自身份，避免原值失效／离开作用域后丢失前提；新增对应单元回归。最终 `test -p luna --test integer_contracts` 14 项通过（`/tmp/luna-tools-03zg9_xy/result.json`）；`test -p elaboration --lib --test verification --no-fail-fast` 231+85 项通过（`/tmp/luna-tools-u1dt_jyo/result.json`）；`test -p luna --test host integer_minmax` 5 项再次通过（`/tmp/luna-tools-_1t_wc7i/result.json`）。均使用上述受管检查入口，退出 0。格式化退出 0（`/tmp/luna-tools-4rmwqf25/result.json`），dev 未提交。

关系回归 `python3 -B development/check.py --timeout 600 -- test -p luna --test host integer_minmax` 3 项通过（`/tmp/luna-tools-ih0v40mm/result.json`）。新增一项覆盖 i8/u8/i128/u128、跨调用和写入后的值复制，错误关系拒绝时保留旧产物；写入前后值关系不再成立的 `assumes` 在 O0/O3 都执行失败。先前逐个关系立即传播并对所有输入分配新身份时，组合用例的残余分析耗尽 1M 测试预算（`/tmp/luna-tools-ug304c6z/result.json`）；现在复用不变身份并直接保留已知关系，在同预算下通过。编译器默认预算未改动。工作目录 `/home/fin/luna`，最终检查退出 0。

随后以同一检查入口执行 `test -p elaboration --test verification`，85 项通过（`/tmp/luna-tools-70of1e_l/result.json`）；`test -p luna --test integer_contracts`，14 项通过（`/tmp/luna-tools-24nidz9c/result.json`）。加强运行时失败检查，核对每次仅产生一个 runtime-check-failure 后，`test -p luna --test host integer_minmax_relations` 再次通过（`/tmp/luna-tools-lq4ocfyz/result.json`）。格式化退出 0（`/tmp/luna-tools-zgdwytl3/result.json`），`git diff --check` 通过，dev 未提交。

2026-10-24，在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 600 -- test -p luna --test host integer_minmax`：2 项通过（`/tmp/luna-tools-4m7pb3ve/result.json`），包含真实 math 包十种类型、正负数及边界输入在 O0/O3 的结果矩阵，并验证区间契约、实参求值次数与快照、错误后置条件拒绝及旧产物保留。初次用例因 `+` 缺少两侧空白而被解析器正确拒绝（`/tmp/luna-tools-ia5lhkz6/result.json`），修正用例后重跑通过。

`test -p luna --test host_intrinsic_protocol --test frontend_cli --no-fail-fast` 经同一入口执行，32+196 项通过（`/tmp/luna-tools-j7s7asmo/result.json`）。`python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py integer-minmax --compare-optimization` 退出 0（`/tmp/luna-tools-na83pjsl/result.json`）；仅执行新增示例两档，目录现有 27 项。格式化退出 0（`/tmp/luna-tools-0jehswrd/result.json`），dev 未提交。

## 示例集优化前后验收

示例运行器新增 `--compare-optimization`：复用一次私有驱动构建，逐项执行 O0/O3，分别保存二进制、构建及运行报告。两档均须满足原有逐字节输出契约；O0 成功不能掩盖 O3 失败，失败时仍保存已完成条目和当前失败证据。与 `--opt-level` 互斥，原单档入口保持默认 O0。

2026-10-24，在 `/home/fin/luna` 执行 `python3 -B development/command.py --timeout 900 -- python3 -B development/host_examples.py --compare-optimization`：26 个示例共 52 次构建和原生执行全部通过（`/tmp/luna-tools-1e4uectc/result.json`，逐项证据 `/tmp/luna-host-examples-g__nctil/summary.json`）。覆盖现有示例集，不能据此推断尚未接通的捕获式效应或通用字符串借用已经实现。`python3 -B -m unittest discover -s development -p test_host_examples.py` 6 项通过，包括模拟 O3 错误输出时非零退出、保留两档报告且只构建一次驱动。两条命令退出码均为 0；dev 未提交。

## 固定位宽整数饱和加减

新增 add_sat/sub_sat 操作与 math.saturating_add/saturating_sub 泛型接口，域为 u8/u16/u32/u64/u128 和 i8/i16/i32/i64/i128。输入与输出必须为同一类型；source admission 证明泛型整数域，具体证据保留符号和位宽，LLVM 据此选择 uadd.sat/usub.sat/sadd.sat/ssub.sat。其数学结果钳制在该类型边界内，不改变普通整数运算的溢出契约。Scalar 追加标签；所有库绑定入口同步至 36 项。

程序流分析对加法使用 `[a.lower + b.lower, a.upper + b.upper]`，对减法使用 `[a.lower - b.upper, a.upper - b.lower]`，再将两端钳制到类型范围。数学整数运算和比较受原有预算约束；结果区间为单点时保留常量事实。实参按源码顺序求值并使用值快照，后续实参写入先前来源不改变已求得的值。缺失范围时保留未知结果；不建立饱和结果等于普通加减的关系式。

区间回归：`python3 -B development/check.py --timeout 600 -- test -p elaboration --lib saturation_tests` 2 项通过（`/tmp/luna-tools-jic6ffi_/result.json`），覆盖小区间穷举、128 位中间结果、非法边界、预算耗尽及恢复。`test -p luna --test host saturating_contracts` 2 项通过（`/tmp/luna-tools-44_t4hku/result.json`），覆盖十种整数类型在 O0/O3 的饱和／未饱和范围契约、实参快照、错误后置条件及写入后的失效范围；拒绝时保留已有产物。工作目录均为 `/home/fin/luna`，退出码均为 0。

随后执行 `test -p elaboration --test verification`：85 项通过（`/tmp/luna-tools-7n12mzi8/result.json`）；`test -p luna --test host saturating`：3 项通过（`/tmp/luna-tools-9owsn41j/result.json`），包含真实泛型 math 包的十种类型 O0/O3 原生结果矩阵。均使用上述检查入口，退出 0。格式化通过（`/tmp/luna-tools-bkg5k4bq/result.json`），`git diff --check` 通过；dev 工作区未提交。

2026-10-24，在 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --test host native_saturating` 1 项通过（`/tmp/luna-tools-4oxdizz5/result.json`）；十种类型、O0/O3 共 170 组输入，核对 340 个加减结果，包含上下界溢出及混合正负号的正常结果。`test -p luna --test host_intrinsic_protocol --test frontend_cli --no-fail-fast` 32+196 项通过（`/tmp/luna-tools-576npe11/result.json`），包括混合类型／位宽／符号与非整数拒绝、证据重检和真实 math 包。

新增 examples/host-saturating-integers.fl，经 `host_examples.py saturating-integers --opt-level 3` 构建并执行通过（`/tmp/luna-tools-biyhyzbn/result.json`），目录现有 26 项，本轮只跑新增项。格式化退出 0（`/tmp/luna-tools-0gjj0zo9/result.json`），git diff --check 通过。dev 未提交。

## LLVM bitcode 交付

新增 `--emit-bitcode <entry> --output <path>`，支持单文件和显式多包，以及既有优化、资源协议、intrinsic、丢弃结果及 inline 选项。对象／汇编／bitcode 共用 llvm-as 和经过目标校验的 opt 阶段；bitcode 在此交付，不启动 llc 或链接器。Rust 接口为 `host::bitcode` 和 `host::binary`；`BinaryOutput` 显式区分 Executable／Object／Bitcode，package Action::Build 改为 output 字段。二进制保持原子发布、源码别名保护与非可执行权限；不接受 stdout 或 with-inputs 包装。

2026-10-24，在 `/home/fin/luna`，`development/check.py --timeout 600 -- test -p luna --test host emit_bitcode` 2 项通过（`/tmp/luna-tools-cihc6q7s/result.json`）。带资源清理和跨函数 handler 的源码以单文件／多包、O0／O3 导出，两次字节一致；真实 llvm-dis 还原后 triple、data layout、语义 target metadata 均保留，重新编译原生执行正确。覆盖不可用链接器、失败保留旧文件、源码／硬链接保护和参数冲突。工具层还验证目标冲突拒绝，以及 llc 不产出有效文件时 bitcode 仍独立成功。

最终 `development/check.py --timeout 600 -- test -p luna --lib --test host --test frontend_cli --no-fail-fast` 退出 0，62+479+196 项全部通过（`/tmp/luna-tools-jicc0ng5/result.json`）。格式化退出 0（`/tmp/luna-tools-79uymw7v/result.json`），git diff --check 通过。用法见 development/README.md 和 package-cli.md；dev 未提交。

## 工具成功退出后的产物交付检查

llvm-as、opt、llc、链接器成功退出后，现在先检查预期产物为非空普通文件，再交给后续阶段或发布。空文件、缺失文件、目录及 FIFO 均拒绝；FIFO 不进入阻塞读取。这个检查确认交付形态，不替代对象格式／ABI 校验。源输出保护和原子发布继续由既有入口负责。

2026-10-24，在 `/home/fin/luna`，新增链接器回归先证实旧实现发布空可执行文件（`/tmp/luna-tools-83akwtqk/result.json`）。修复后四种失败形态都保留旧输出，恢复真实链接器后可构建运行（`/tmp/luna-tools-3_alww_l/result.json`）。独立 LLVM 阶段注入空文件／FIFO，也验证恢复及 assembly 分支（`/tmp/luna-tools-db3sgn7a/result.json`）。最终 `development/check.py --timeout 600 -- test -p luna --lib --test host --test source_tests --no-fail-fast` 退出 0，62+477+13 项通过（`/tmp/luna-tools-tiw6ryw7/result.json`）。格式化退出 0（`/tmp/luna-tools-bhksxtth/result.json`），git diff --check 通过。dev 未提交。

## 浮点分类与 bool 结果 intrinsic

分类接口现补齐 isnormal、issubnormal、signbit，Scalar 标签仍追加，全部绑定为 34 项。前两者使用 is.fpclass 的 264／144 掩码；signbit 用 [LLVM copysign](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-copysign-intrinsic) 把符号复制到精确的 1.0，再比较正负，因此原值为负零或负 NaN 时仍为 true。结果均为 bool，继续复用既有浮点域证明及实例化复核，不提供新的静态浮点规格推理。

2026-10-24，在 `/home/fin/luna`，`development/check.py --timeout 600 -- test -p luna --test host float_classification` 3 项通过（`/tmp/luna-tools-xzvzg25x/result.json`）：泛型库矩阵扩展至 17 个位模式 × 两种精度 × O0/O3，各检验六项操作，包含最大次正规数和最小正规数相邻边界；直接谓词／单次求值矩阵扩展至 96 次原生执行。`test -p luna --test host_intrinsic_protocol --test frontend_cli --no-fail-fast` 31+196 项通过（`/tmp/luna-tools-g6v0o_8t/result.json`），包括全部操作的证据篡改拒绝与真实 math 包。更新后的 `host_examples.py float-classification --opt-level 3` 退出 0（`/tmp/luna-tools-azc7k8is/result.json`）。格式化退出 0（`/tmp/luna-tools-zvfmy0tn/result.json`），git diff --check 通过。dev 未提交。

运行时谓词衔接已独立核验：`test -p luna --test host native_float_classification_predicates` 退出 0（`/tmp/luna-tools-jz94t3jd/result.json`）。三类操作 × 两种精度 × 两种位模式 × 正反预期 × O0/O3，共 48 次原生执行；正确谓词输出 true，错误谓词不输出结果且只有一次 runtime-check-failure。普通 intrinsic 实参内的访问标记验证求值恰好一次；这不授权在 assumes 的实参里写外部状态。现有 float-classification 示例也加入直接 intrinsic 谓词，经统一运行器 O3 通过（`/tmp/luna-tools-8b759ghk/result.json`）。本次没有修改编译器实现，也没有声称一般函数调用或浮点静态契约已获支持。格式化退出 0（`/tmp/luna-tools-x471kple/result.json`），git diff --check 通过。

新增 isnan／isinf／isfinite 绑定与 library/math 泛型包装。Registry 和 Checked 的 scalar evidence 现在描述操作数域，结果类型由操作决定；分类操作仅接受 f32/f64 输入及 bool 输出，泛型主体必须先证明输入域，实例化后重新复核。CLI、库检查入口、示例运行器及多包测试均补齐 31 项绑定。原 Scalar 标签编号保留，新标签追加。

LLVM 使用 [is.fpclass](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-is-fpclass-intrinsic)，固定掩码分别为 3、516、504，不启用 fast-math。NIR 类型身份、资源检查和普通实参求值规则保持不变。一般浮点 requires／ensures 的静态条件证明仍未实现；此处提供原生分类与已有谓词结构准入。

2026-10-24，在 `/home/fin/luna` 执行 `development/check.py --timeout 600 -- test -p luna --lib --test host --test host_intrinsic_protocol --test frontend_cli --no-fail-fast`：61 项单元、474 项 host、31 项 intrinsic 协议通过；CLI 的两项失败来自测试参数遗漏 --host-intrinsic（`/tmp/luna-tools-y2yc2xqz/result.json`）。补齐后 `test -p luna --test frontend_cli` 196 项全部通过（`/tmp/luna-tools-v9mhj1ml/result.json`）。最终 `test -p luna --test host float_classification` 2 项通过（`/tmp/luna-tools-dv0lr9wa/result.json`）：真实泛型库在两种精度、O0/O3 下覆盖 56 组位模式，各检查三种分类；另拒绝无约束泛型、整数域、bool 输入及错误结果类型，并保留旧产物。早期新增 fixture 的模块开括号紧邻注释被解释为块注释，已补换行，记录 `/tmp/luna-tools-n6ullv4n/result.json`。

`development/host_examples.py float-classification --opt-level 3` 通过真实多包编译与原生执行，输出 `(bool true)`（外层报告 `/tmp/luna-tools-qkjq_gyg/result.json`）。统一目录现有 25 项，本轮只运行新增项。格式化退出 0（`/tmp/luna-tools-qnllsvio/result.json`），git diff --check 通过。dev 未提交。

## 目标汇编导出

`--emit-asm <entry>` 支持 stdout 或 `--output`，单文件与 `--packages-v1` 共用 host 选项、资源协议、目标校验及优化。`Toolchain::assembly` 与 object 共用 LLVM 验证／优化／llc 路径，仅改变 llc 的输出格式；不启动链接器。多包文本动作统一为 `Action::EmitHost`，`assembly` 区分汇编和 LLVM IR；`--with-inputs` 的汇编结果为 `(host-assembly (string "..."))`，原 LLVM 包装保持不变。生成产物保留现有开发入口 ABI。

2026-10-24，在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 600 -- test -p luna --lib --test frontend_cli --test host --no-fail-fast`，退出 0：61+196+473 项全部通过（`/tmp/luna-tools-fkgatuqt/result.json`）。新端到端用例使用带资源协议、跨函数 handler 的 `host-direct-calls.fl`，以单文件／多包、O0／O3 导出，在指定不存在链接器时仍成功；两次汇编逐字节一致，stdout 与文件相同，再由独立 C driver 组装链接并原生执行。另覆盖互斥动作拒绝、LLVM 不可用保留旧输出、源码及硬链接保护、with-inputs 包装。工具层目标冲突回归同样覆盖 assembly。

最初多包转发白名单遗漏 `--emit-asm`，测试失败于 `/tmp/luna-tools-44hpcmw_/result.json`，已补齐并经最终集成复验。格式化退出 0（`/tmp/luna-tools-dr9mk9nd/result.json`），git diff --check 通过。使用命令见 development/README.md；dev 未提交。

## 原生结果输出失败的退出状态

开发入口的结果打印器记录每次 printf／putchar 的失败，并在返回 main 前刷新缓冲输出；失败状态粘滞，后续成功不能覆盖。标量、String／Symbol、结构结果统一返回 0（成功）或 1（输出失败）。刷新使用 `fflush(NULL)`，不依赖 FILE 布局或平台 stdout 导出符号；因此刷新任何已打开输出流的失败也会使入口失败。这是 host 开发打印器的进程交付行为，不是 Flurry 错误结果或 effect ABI，也不宣称一般库 IO 已实现。

2026-10-24，在 `/home/fin/luna`，新增 Linux `/dev/full` 原生回归先证实旧实现输出失败仍退出 0（`/tmp/luna-tools-0lr1itm9/result.json`）。修复后 O0/O3 的标量、短字符串、超过 stdio 缓冲区的长字符串、结构结果正常输出不变，写满设备均退出 1（`/tmp/luna-tools-hge231j9/result.json`）。独立 LLVM 测试注入字符写入、格式化输出、刷新各自的结果，核验早期失败在后续成功后仍保留。

最终 `python3 -B development/check.py --timeout 600 -- test -p luna --lib --test host --test source_tests --no-fail-fast` 退出 0：61+471+13 项全部通过，报告 `/tmp/luna-tools-rjo9aaq6/result.json`。格式化退出 0（`/tmp/luna-tools-06uk3s08/result.json`），git diff --check 通过。dev 未提交。

## 模块级文本常量复用

LLVM emitter 的 named body、closure 与 outlined handler 共享文本常量池；相同 UTF-8 字节只编码／发布一个全局，编号按首次使用顺序确定，不依赖 HashMap 迭代顺序。String 与 Symbol 可以共享只读字节，但 Value 仍保留各自类型；这不产生 `str` 的 backing-region 资源证据，checked 字符串路径继续拒绝。缓存命中仍按字节计费，编码预算失败不发布条目，后续请求可恢复。

2026-10-24，在 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --lib` 60 项通过（`/tmp/luna-tools-uoralsrd/result.json`）；`test -p luna --test host string` 5 项通过（`/tmp/luna-tools-xjcd9z80/result.json`）；`test -p luna --test host native_text_constants` 1 项通过（`/tmp/luna-tools-r7vbvex7/result.json`）。原生回归验证跨 named function／closure／pattern／outlined handler 仅一份字节常量、两次独立 LLVM 输出一致及 O0/O3 执行正确；String／Symbol 同字节仍分别打印。最初回归使用保留字 matches 作函数名，以及误把打印器 prefix 算入字面量总数，两次测试失败后均修正测试并复验。最终格式化退出 0（`/tmp/luna-tools-odm0phpm/result.json`），git diff --check 通过。dev 未提交。

统一示例入口已接入 `intrinsic-contract` 与 `intrinsic-predicate`，自动提供 intrinsic 绑定和资源协议，目录现有 24 项。2026-10-24 在 `/home/fin/luna` 执行 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py intrinsic-contract intrinsic-predicate --opt-level 3`，当前源码构建、两个示例原生执行均退出 0，各输出 `(bool true)`。本轮只重跑这两个新增目录项，汇总 `/tmp/luna-host-examples-yaxij6eo/summary.json`，外层报告 `/tmp/luna-tools-1kgtvow_/result.json`。dev 未提交。

## 混合契约链的冗余投影与闭包成本

保留真正失败的源码 `repros/host-intrinsic-mixed-contract-chain.fl`：常量结果与范围结果留在同一调用者中，包含七类整数 intrinsic 及前后置条件。旧快照在 1M fuel 下仍于源码 flow 分析耗尽（`/tmp/luna-tools-ou77chx7/result.json`）；修改后同预算实际 O0/O3 构建并运行成功。没有改动默认 fuel，也未减少源码条件或拆分该回归链。

两处优化均保留逻辑事实：同 lineage 的复制若所有源版本仍当前，直接保留／重命名完整关系并添加复制等式，不重新投影中间变量；有失效版本或跨 frame 时仍执行原投影。差分图构造则过滤已被当前区间蕴含的两变量边，避免已知常量的复制等式扩大临时全对闭包；Facts 中原始多项式不删除，区间放宽后仍参与推导。MissingBound 不被假定为已知，预算／其它错误正常传播。

工作目录 `/home/fin/luna`。`development/check.py --timeout 600 -- test -p elaboration --lib redundancy_tests` 1 项通过（`/tmp/luna-tools-5oqczq87/result.json`），24 个值的冗余复制边在 5k 工作预算内处理，随后放宽区间仍推出首尾相等。`test -p elaboration --test verification` 85 项通过（`/tmp/luna-tools-yfm1v8pa/result.json`），包含差分闭包、分支 join、旧版本拒绝及多项式验证。

`test -p luna --test host native_mixed_intrinsic_contract_chain_reuses_current_relations` 1 项通过（`/tmp/luna-tools-7wmojrdy/result.json`）。`test -p luna --test integer_contracts --test staged_aggregate_contracts --test method_memory_contracts --test drop --no-fail-fast` 14+31+8+992 项通过（`/tmp/luna-tools-h3g27l6e/result.json`）。均退出 0；格式化退出 0（`/tmp/luna-tools-7o8lmjt7/result.json`），git diff --check 通过。默认 100k 的该大例仍可能耗尽残余检查预算，具体失败 `/tmp/luna-tools-sfzz7u88/result.json`；本轮只证明相同 1M 预算下从失败到成功。dev 未提交。

## 整数 intrinsic 的程序范围与精确值

流分析先按源顺序求完实参，再复核具体 target evidence。ctpop/ctlz/cttz 的未知输入结果落在 0..width；已证明非零时，ctpop 至少为 1，ctlz/cttz 小于 width。对已知 unsigned 输入，bitreverse、bswap、ctpop、ctlz、cttz、fshl、fshr 得到精确结果；funnel shift 采用模位宽规则，零偏移分别返回左／右输入，128 位路径避免移位 128。输入常量超过声明宽度时拒绝分析，不产生错误事实。浮点操作仍 unknown。

这些是程序规格事实，不是目标 intrinsic 的 comptime invocation。一般未知操作的重复计算等价与跨调用证据重放仍未实现。长混合契约调用链的预算问题记录于 frontend-checkpoint，没有通过扩大编译器默认预算或减弱谓词要求解决。

工作目录 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --test host integer_intrinsic_contract` 退出 0，2 项通过（`/tmp/luna-tools-kt3z8noq/result.json`）。其中 94 个源码 test 各以 O0/O3 执行，共 188 次，覆盖五种位宽、零／非零范围、精确值、模位宽、后续实参修改前面实参的来源仍保持旧值；另 5 个错误／过期边界拒绝例保留旧产物。矩阵按独立源码 test 组织，驱动显式使用 10M fuel。

`test -p luna --test integer_contracts --test host_intrinsic_protocol --no-fail-fast` 14+30 项通过（`/tmp/luna-tools-phl09r4x/result.json`）；`test -p elaboration --lib verification::flow` 5 项通过（`/tmp/luna-tools-h6ln2enh/result.json`）。真实 `examples/host-intrinsic-contract.fl` 经配对 Copy/Drop、ctlz 绑定和 O3 构建退出 0（`/tmp/luna-tools-t4z7nu2b/result.json`），原生执行输出 `(bool true)`（`/tmp/luna-tools-slqdv8xu/result.json`）。最终格式化退出 0（`/tmp/luna-tools-l0mfwqn_/result.json`），git diff --check 通过。dev 未提交。

## 具有目标证据的 intrinsic 运行时谓词

NIR 的规格结构检查现在复核 intrinsic 的声明 tag、标量签名与元数，再递归检查每个实参；无 binding、篡改证据、写入实参均拒绝。Pending binding 只允许模板结构，不是执行许可。资源观察对具体操作重新调用 checked，unsafe scope 只传播已检查的内容，不豁免 Copy、读权限或观察副作用规则；观察 block 的结构遍历也保留同样证据要求。

新增 `examples/host-intrinsic-predicate.fl`，f32/f64 直接在 assumes 中计算 FMA，资源检查后的原生 O0/O3 均可执行。失败只报告一次且不打印成功值。负例在 unsafe intrinsic 实参中写外部变量，源码阶段拒绝并保留旧产物。一般纯函数调用及含 intrinsic 的 requires/ensures 静态条件表达仍有前端缺口，记录于 frontend-checkpoint，没有将这些能力计入完成。

工作目录 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --test host_intrinsic_protocol --test drop --no-fail-fast` 退出 0，30+992 项通过（`/tmp/luna-tools-59lphum0/result.json`）；`test -p nir --test predicate_reads` 9 项通过（`/tmp/luna-tools-x2y2hjvy/result.json`）；`test -p luna --test elaboration program_assumes` 2 项通过（`/tmp/luna-tools-uhalia1i/result.json`）。最终实际示例文件经 `test -p luna --test host intrinsic_predicates` 验证，2 项通过（`/tmp/luna-tools-jzhgi4s_/result.json`）。证据矩阵覆盖全部 28 种标量操作及每个实参位置，另核对模板不能未特化执行。格式化退出 0（`/tmp/luna-tools-hjneuung/result.json`），git diff --check 通过；dev 未提交。

## Host 阶段集成核验

在新增 minimumNumber/maximumNumber 降码、析构逻辑主体检查和编译工具双流诊断之后，统一检查当前 host 层，未跳过失败用例。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 600 -- test -p luna --lib --test host --test source_tests --no-fail-fast` 退出 0：59 项单元、464 项 host、13 项源码测试全部通过；报告 `/tmp/luna-tools-nk7sxq2r/result.json`。这是当前工作区的集成证据，不是整个语言的完成证明。dev 未提交。

后续实现边界确认：剩余 runtime 表达式默认拒绝分支不能直接取消，其中包含 TypeValue、TypeList、NormalFormType 等编译期构造。Capturing/Async 效应也不能映射到 direct ABI；正式 effect-abi 文档要求 continuation 的单次消费、保存栈地址稳定、放弃清理、恢复点外层 evidence 重供应及栈池来源保留。已有 direct/direct-passing 测试通过不证明这些控制后端已实现。

## IEEE minimumNumber／maximumNumber 与 math 包集成

新增 `MinimumNumber`／`MaximumNumber` 的 registry 名称、两参数同精度 f32/f64 准入、泛型源证据与重绑定检查，CLI 绑定名为 minimumnum/maximumnum。library/math 提供同名 pure 泛型包装；Scalar 新标签追加于末尾，不改变旧标签编号。math 的 28 项绑定同步到运行器、库检查入口、CLI 测试和 README，旧入口缺失的 bswap/fshl/fshr 也已补齐。

契约采用 [LLVM minimumNumber](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-minimumnum-intrinsic)：单侧 NaN 返回数值侧，两侧 NaN 返回 NaN，负零小于正零。实际 LLVM 21.1.8 的直接 intrinsic 降码在本机 O0 未通过正负零位模式测试，因此后端用已验证的 minimum/maximum 加单侧 NaN 选择展开；输入值只求值一次，不添加 fast-math。最初尝试的 minnum/maxnum 未作为公开操作交付；其文档存在零排序表述冲突，不能据此放宽测试。直接降码失败证据 `/tmp/luna-tools-3nvetxox/result.json`；最终实现的严格回归通过 `/tmp/luna-tools-el3i2b8g/result.json`，覆盖四种 extrema 操作、f32/f64、O0/O3、两个操作数位置、quiet/signaling NaN、两侧 NaN、正负零和无穷。

工作目录 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --test host_intrinsic_protocol --test frontend_cli` 退出 0，29+196 项通过（`/tmp/luna-tools-y39kro1v/result.json`）；`test -p nir --lib operation_names` 1 项通过（`/tmp/luna-tools-i1rchxsq/result.json`）。真实 app/math/resources 包以全部 28 项绑定、配对 Copy/Drop、`--test-native --opt-level 3` 执行，6 项通过（`/tmp/luna-tools-z4ztssqk/result.json` 保存完整命令）。格式化退出 0（`/tmp/luna-tools-_s7vbk_9/result.json`），git diff --check 通过。未运行全 workspace 测试；dev 未提交。

## 示例执行结果的严格检查

host_examples.py 按 UTF-8 字节核对预期结果及单个末尾换行，不再 strip 掉额外空白。无效编码、输出截断、非零退出和日志缺失均形成明确失败结果；summary.json 保留实际输出、失败原因和构建／执行报告路径。原始日志仍独立保存。

工作目录 `/home/fin/luna`：`python3 -B development/host_examples.py --opt-level 3` 的 22 个示例全部通过，汇总 `/tmp/luna-host-examples-vxyoptu_/summary.json`，各项报告路径均存在。`python3 -B development/command.py --timeout 30 -- python3 -B -m unittest discover -s development -p test_host_examples.py` 退出 0，5 项测试覆盖精确输出、空白差异、无效 UTF-8、截断／失败退出及缺失日志，报告 `/tmp/luna-tools-ul4hcne4/result.json`。本轮未修改 Rust；git diff --check 通过。dev 未提交。

## 编译工具失败的双流诊断

host 的 LLVM／链接工具失败时现在保留 stdout 与 stderr；此前 stdout 中的失败原因会丢失。每个流的诊断展示最多使用 16 KiB 原始字节前缀，控制字符经转义，截断明确标记。这个展示上限独立于成功结果的捕获上限：成功的完整 LLVM 模块仍原样交付，不能因为诊断预算而截断 IR。退出失败、超时和输出超限均复用该诊断路径。

在 `/home/fin/luna` 经 `development/check.py --timeout 300 -- test -p luna` 验证：`--lib host::tools::capture_tests` 12 项通过（`/tmp/luna-tools-ab6pe7l3/result.json`）；`--test host native_linker_failure_reports_both_streams_and_preserves_artifact` 通过（`/tmp/luna-tools-gxpzxtic/result.json`）。后者使用实际失败链接器，核对双流信息与控制字节转义、旧可执行文件保留，再恢复默认链接器成功构建并运行。均退出 0；格式化退出 0（`/tmp/luna-tools-afqbc0d6/result.json`），`git diff --check` 通过。dev 未提交。


## 析构投影的逻辑类型一致性

泛型实例组合补充：Wrapped<Set> 与 Wrapped<Clear> 的底层布局相同，但析构分别检查并设置／清除同一个标志；一层／两层包装、直接局部／泛型闭包捕获四种组合均对照 checked 解释器与 LLVM O0/O3，核对实际清理副作用。IIR 拒绝矩阵同时覆盖不同声明，以及同一 `Wrapped<T> = bool` 声明的 bool/u32 两个实例；替换接收者不能因为实例表示相同而获准。此轮补充验收，没有新增类型等价规则。

目录 `/home/fin/luna`，定向 `--test host native_generic_newtype_instances_keep_distinct_cleanup_subjects` 通过（`/tmp/luna-tools-3l21f9hh/result.json`），扩充的 `--lib destructor_receiver_cannot_be_rebound_to_a_layout_equivalent_newtype` 通过（`/tmp/luna-tools-sc69c5fn/result.json`），均经 `development/check.py --timeout 300 -- test -p luna` 执行并退出 0。格式化退出 0（`/tmp/luna-tools-bh4pqaq4/result.json`），`git diff --check` 通过；dev 未提交。

清理投影现在同时保留最终逻辑类型与物理布局。Invoke 准入及地址生成前均检查投影主体与 cleanup subject 一致；析构函数的接收者还必须是对应主体的指针。Outer、Inner 与其 bool 表示即使拥有相同 LLVM 布局，也不能交换逻辑清理职责。正常路径仍复用透明 newtype 的原地址，没有改变类型身份或构造新的类型等价规则。

单元矩阵在相同布局下验证正确 Inner 主体接受、Outer/bool 替换拒绝。新增 IIR 级测试从合法 First/Second 程序取得冻结的清理计划，只把 First 析构签名的接收者改为 Second 指针；后端明确拒绝，原始输入随后仍产生完全相同的 LLVM dump。

工作目录 `/home/fin/luna`：`development/check.py --timeout 300 -- test -p luna --lib --test host --no-fail-fast` 的 57 项单元测试与 462 项 host 测试通过，退出 0（`/tmp/luna-tools-b1w54cbv/result.json`）。随后新增接收者错配回归定向 1 项通过（`/tmp/luna-tools-3q7z7nld/result.json`）。格式化退出 0（`/tmp/luna-tools-jx31w504/result.json`），`git diff --check` 通过。dev 未提交。


## 并发启动时的 ETXTBSY 重试窗口

此前并发单元测试实测出现 `ETXTBSY` 超过 100ms 后启动失败。原生测试现为这个特定的未启动错误保留最多 1 秒重试窗口，使用剩余期限限制休眠；其他启动错误立即返回，成功创建进程后绝不重试运行。运行时 deadline 仍从成功启动后计算，启动等待和运行时间是两个独立上限，永久写占用仍明确失败。

回归将可释放的写占用保持 200ms，超过旧窗口；另以短期限检查永久占用会终止，再解除占用验证恢复。普通并发测试模式下 `development/check.py --timeout 300 -- test -p luna --lib host::` 41 项通过，退出 0（`/tmp/luna-tools-38hgasik/result.json`）；`--test source_tests native` 7 项通过（`/tmp/luna-tools-n_5phzz8/result.json`）。工作目录 `/home/fin/luna`。格式化退出 0（`/tmp/luna-tools-9okb69ms/result.json`），`git diff --check` 通过。这个有界恢复不保证任意长期写占用都能启动；dev 未提交。


## newtype 表示层的原生清理投影

后续组合验收扩展到普通局部、直接丢弃、调用参数的正常／提前返回、匿名 record 字段、两层未调用闭包捕获和赋值替换。底层 Watch 析构检查外部标志尚未设置再写入 true，既核对清理发生，也拒绝重复清理；赋值测试要求旧值先退休、新值留到离开作用域。checked 解释器与 LLVM O0/O3 全部一致。

computed pattern 的成功／拒绝分支及显式 inline 展开也纳入测试。无失败析构除了结果对照，还检查三层析构调用在未优化 LLVM 中存在，避免只凭 Bool 输出宣称清理完成。带 assumes 的析构仍由前端拒绝，缺口已记入 frontend-checkpoint；本次未放宽该检查。工作目录 `/home/fin/luna`，`development/check.py --timeout 300 -- test -p luna --test host newtype` 12 项通过，退出 0（`/tmp/luna-tools-cbca3f_p/result.json`）；格式化退出 0（`/tmp/luna-tools-jiiddn8d/result.json`），`git diff --check` 通过。dev 未提交。

新增可运行示例 `examples/host-owned-result.fl`，入口返回 `!Failure Ticket`，其中一个错误载荷经 `Envelope = Ticket` 包装。参考解释器能退休该载荷，但原生清理准入最初拒绝：cleanup 的路径包含逻辑 newtype 表示层，而 LLVM 透明布局不包含这一真实字段。

现在清理准入与析构地址计算共同跟踪逻辑类型和物理布局。跨越已物化 newtype 的第 0 表示字段时保留原指针、推进逻辑类型，不生成虚假的 GEP；普通 struct、Tuple、闭包、enum 与错误结果仍按相应字段／活动变体投影。标签仍由原逻辑路径标识，不能因表示层擦除而混淆嵌套变体。仅允许已物化定义的合法第 0 字段，并核验表示一致；未改变类型身份、也未放宽部分构造状态检查。

回归分别让 newtype 包住错误载荷和整个错误枚举，覆盖成功与两个错误分支、析构成功／失败，逐项对照 checked 解释执行与 LLVM O0/O3。Ticket 析构检查并清除 ready，错误清理非活动载荷或重复析构会失败；失败测试保证实际执行析构。单元拒绝例覆盖未知名义类型、错误字段、过深路径和不匹配物理布局。默认示例运行器现有 22 项，新增 owned-result 自动使用 `--discard-result` 及 Copy/Drop 协议，成功输出 unit。


验证目录 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --lib --test host --test drop --test effect_memory_transfers --no-fail-fast` 中 host 460、Drop 992、效应内存 91 项均通过；库单元测试有一个原生启动测试遇到 ETXTBSY（`/tmp/luna-tools-secoq774/result.json`）。补充投影拒绝测试后，`development/check.py --timeout 300 -- test -p luna --lib -- --test-threads=1` 57 项全部通过，退出 0（`/tmp/luna-tools-fvogbatd/result.json`）；该复验不宣称并发启动瞬态问题已修复。`development/host_examples.py owned-result --opt-level 3` 构建与执行通过（`/tmp/luna-host-examples-ajqof0vt/summary.json`）。格式化退出 0（`/tmp/luna-tools-eixvb8ji/result.json`），`git diff --check` 通过。dev 未提交。

## 标量 newtype 入口结果的所有权检查

修复直接主入口的一处所有权漏洞：结构结果打印器会拒绝尚无清理适配器的名义类型，但标量结果分支只检查物理表示。`newtype Token = bool` 等结果因此会被打印成底层标量，未退休 Token。新增回归先在修复前证实带失败析构器的 Token 被错误接受；现在入口按逻辑 `Term::Nominal` 拒绝，不能因为表示透明而绕过已有名义结果边界。错误提示指向已实现的 `--discard-result` 与 Copy/Drop 协议；这不是新增名义值展示功能。

覆盖 bool、u32、f64、void 与 BitVec<0> 五种表示。普通 `--build`、`--emit-object`、`--emit-llvm` 均拒绝并保留既有输出文件；显式丢弃入口在 LLVM O0/O3 下分别验证成功析构输出 unit、失败析构产生唯一谓词诊断且不输出结果。名义值在函数内部的传递、转换和析构继续由既有资源检查与布局机制处理。

工作目录 `/home/fin/luna`：`development/check.py --timeout 300 -- test -p luna --test host` 459 项通过，退出 0（`/tmp/luna-tools-h4am5kkh/result.json`）。格式化退出 0（`/tmp/luna-tools-ug2ohrch/result.json`），`git diff --check` 通过。dev 未提交。


## 复合谓词范围与析构二次失败

真实原生双重失败回归发现 `assumes not flag` 的谓词范围包含了前面的 `assumes`。前缀表达式原先从 parser scope 的上一个已消费 token 取起点；现显式记录当前运算符的起点。Pratt 二元表达式同样显式记录本表达式起点，结束位置取最后消费的 token，以保留 `(a) + (not b)`、`-(a + b)` 的外层括号而不污染内层节点范围。只调整运算表达式的源码范围，不改变优先级、类型或运行时语义；没有宣称其他语法节点的历史范围问题均已修复。

新增解析器回归覆盖混合一元／二元运算、嵌套 not、括号边界和非零 SourceMap 偏移。`native_second_failure_stops_remaining_cleanup` 继续通过观察终止点的内存状态验证剩余析构不执行，并额外执行未改写的原生产物：正常退栈时析构失败输出一条诊断；原始谓词失败后析构再次失败输出两条诊断，严格核对顺序及范围，跨 stop／Drop／trial 的传播不增加消息。两种情形均执行 LLVM O0/O3。


验证工作目录 `/home/fin/luna`：`development/check.py --timeout 300 -- test -p parse` 全部 82 项通过（`/tmp/luna-tools-bitt9tmq/result.json`）；`development/check.py --timeout 600 -- test -p luna --test host --test drop --test effect_memory_transfers --no-fail-fast` 分别 458、992、91 项通过（`/tmp/luna-tools-7nxqd_ie/result.json`），均退出 0。格式化退出 0（`/tmp/luna-tools-62vihnu9/result.json`），`git diff --check` 通过。dev 未提交。

## 原生谓词失败的位置诊断

实际谓词失败现在先向 stderr 输出 `(runtime-check-failure (predicate start end) (owner ...))`，再沿已有清理／失败传播路径退出。覆盖 Check/Require/Ensure 谓词节点、函数前后置条件与 trait 接口前后置条件。调用方对 `flurry_failed` 的传播检查不重复报告；清理过程中如果另一个谓词独立失败，仍可报告其自身位置。使用当前 POSIX host 的 `dprintf`，避免 trap 前 stdout 缓冲导致诊断丢失；常量文本转义，格式字符串中的百分号另行处理。

位置通过 IIR 的 expansion_sources 还原为原始 SourceRange，不把生成节点身份伪装成文件偏移。命名函数带 declaration 身份，匿名闭包以 closure 标示。当前输出是源范围及 owner 身份，尚非文件名／行列／源码片段渲染；没有源谓词的底层算术、布局或内部 guard 仍沿原有 trap 路径。未放宽任何前端验证：不满足 requires 的调用和未证明的 ensures 实现仍在编译时拒绝；接口运行时拒绝通过已有的 IIR 手工扰动测试隔离验证。

回归覆盖跨函数传播只报告一次、析构后的失败传播、正常路径无诊断、闭包、内联后的来源还原，以及独立接口前后置条件位置，均检查 LLVM O0/O3。手工拼装 LLVM 的 handler-frame fixture 同步附带 emitter 的声明和常量。工作目录 `/home/fin/luna`：`development/check.py --timeout 300 -- test -p luna --lib --test host --test source_tests --no-fail-fast` 56+458+13 项通过，退出 0（`/tmp/luna-tools-1wzh9_0c/result.json`）；随后接口精确位置断言定向 2 项通过（`/tmp/luna-tools-yxq1dtor/result.json`）。格式化退出 0（`/tmp/luna-tools-z9ugu79c/result.json`），`git diff --check` 通过。dev 未提交。


## 原生对照测试的有界执行

`Toolchain::execute_test_ir` 曾使用无期限的 `wait_with_output`，错误生成的循环或继承输出描述符的进程可能挂住后端单元测试。现在它与正式原生测试共用非阻塞、有输出上限的执行路径，默认执行时限 10 秒；超时终止并回收直接子进程，诊断保留转义后的 stdout/stderr。编译工具仍各自使用已有 120 秒时限。对照接口保留非零退出码供调用者断言；任一输出超过 16 KiB 时明确拒绝，不能把前缀当作完整结果。仅检查退出状态的正式测试仍允许输出截断，并在失败时标明截断。

新增回归实际执行无限输出循环、分别超限的 stdout/stderr、成功／非零退出与超时后的正常恢复；不依赖外层测试总时限兜底。工作目录 `/home/fin/luna`：`development/check.py --timeout 300 -- test -p luna --lib host::` 40 项通过（`/tmp/luna-tools-qc1nz4k4/result.json`）；`--test source_tests native` 7 项通过（`/tmp/luna-tools-i1czxlcj/result.json`）；`--test host native_source_test_cli` 4 项通过（`/tmp/luna-tools-tnijuun6/result.json`），均退出 0。格式化退出 0（`/tmp/luna-tools-qzw6iytk/result.json`），`git diff --check` 通过。

本次修改前运行当前完整示例目录：`development/host_examples.py --opt-level 3` 的 21 项全部完成 LLVM 构建、原生运行与结果核对（`/tmp/luna-host-examples-2fea0yja/summary.json`）。此集成检查验证当前可执行子集，不表示完整语言已完成；上述运行器修改后另以单元、源码及 CLI 原生测试验证。dev 未提交。


## Funnel shift 与循环移位

新增 `fshl`／`fshr` 目标操作及 math 的 `funnel_shift_left`、`funnel_shift_right`、`rotate_left`、`rotate_right` 泛型包装。三个 intrinsic 输入和输出必须是相同的 u8/u16/u32/u64/u128 类型；移位量按位宽取模，不套用普通 shift 的越界规则。左 funnel 返回拼接值的高半部，右 funnel 返回低半部，因此零移位分别返回第一个／第二个输入。循环移位复用已经求值的形参，不重复执行调用者表达式。契约见 [LLVM LangRef](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-fshl-intrinsic)。

现有通用 intrinsic 降码按已检查的操作、元数和位宽生成 LLVM 调用；无需额外语言语法。math Scalar 家族在末尾追加两个 tag，已有 tag 序号保持不变，显式绑定共 26 项。CLI、默认示例运行器与 library 命令示例同步。数值回归使用独立的逐位索引 oracle，覆盖零、1、位宽减 1、位宽、位宽加 1 和该类型最大值；资源检查开启，顺序断言验证 funnel 三个实参及 rotate 两个实参各求值一次。参考解释器仍不执行目标 intrinsic。


验证目录 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --test host --test host_intrinsic_protocol` 456+29 项通过，退出 0（`/tmp/luna-tools-ap7_6jeg/result.json`）；随后扩充的泛型域拒绝测试定向通过（`/tmp/luna-tools-wrlbfq3v/result.json`）。`development/host_examples.py math-fma --opt-level 3` 真实多包构建与执行通过（`/tmp/luna-host-examples-za41nmre/summary.json`）。格式化退出 0（`/tmp/luna-tools-2k5vvq1n/result.json`），`git diff --check` 通过；dev 未提交。

## 字节序反转 intrinsic

新增 `math.byte_swap<T>(value)` 与显式目标绑定 `--host-intrinsic bswap=math.target.Scalar.bswap`。输入输出同为 u16/u32/u64/u128；反转字节顺序而不反转字节内部位序。域依据 [LLVM 21 bswap 合同](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-bswap-intrinsics) 的正 16 倍数位宽要求，不接纳 u8、signed、target-sized、float 或 BitVec。Operation 的名称、类型域、元数、泛型证明、实例化重验和 CLI 帮助同步；已有通用 LLVM intrinsic 降码生成匹配位宽的 llvm.bswap，无需另设语言关键字。

本节加入 bswap 时，math 的 Scalar 家族和默认 math 示例绑定共 24 项操作；`host-math-fma.fl` 同时检查 `0x01020304 → 0x04030201` 的字节反转。测试使用独立字节数组折叠算法作为数值 oracle，覆盖零、全 1、最高位和混合模式，交换两次恢复、输入只求值一次；LLVM O0/O3 实际运行，IR 断言四种合法位宽且无 i8 bswap。当前参考解释器仍不执行目标 intrinsic，不把本测试称为解释器支持；非法类型、缺少泛型域证明、篡改输入和重绑操作的拒绝检查继续生效。

在 `/home/fin/luna` 经 `development/check.py` 执行 `test -p luna --test host --test host_intrinsic_protocol`，455+29 项通过，退出 0（`/tmp/luna-tools-e09eykgl/result.json`）。`development/host_examples.py math-fma --opt-level 3` 通过真实多包编译与原生执行（`/tmp/luna-host-examples-cjtneeyt/summary.json`）；格式化入口退出 0（`/tmp/luna-tools-b6reyghk/result.json`），`git diff --check` 通过，dev 未提交。

## 字符串结果的可执行入口

聚合展示的边界回归继续区分 `Tuple<>` 与 void：空 Tuple、单元素 Tuple、嵌套空 Tuple／void、成功或错误分支内的 Tuple，以及带空 Tuple／void 字段的匿名 record，都逐字节对照解释器与 LLVM O0/O3。错误结果的字段求值一次测试也改为完整结果对照，仍保留分支载荷断言。在 `/home/fin/luna` 经 `development/check.py` 执行 `test -p luna --test host product`，11 项通过，退出 0（`/tmp/luna-tools-5l7cgsof/result.json`）；格式化入口退出 0（`/tmp/luna-tools-xxp51y_n/result.json`），`git diff --check` 通过。此轮补充验证已有逻辑值展示，没有改变 Tuple 的 normal-form 类型身份；dev 未提交。

逐字节对照修复了主入口的匿名 record 展示：LLVM 现在与解释器统一输出 `(anonymous-record (fields ...))`，包含空 record、嵌套 record 和字符串／Symbol／char 字段。该修复只改变开发用结果 dump，不改变函数 ABI、值布局或类型身份。相应固定输出预期与示例运行器已同步。

默认运行器现在包含 `string` 和 `product-string-result`，共 21 项示例。运行 `python3 -B development/host_examples.py string product-string-result --opt-level 3`，可分别输出 UTF-8 字符串及包含字符串、Tuple、布尔值的 record。后一示例显式绑定 Elements／Tuple，检查字段求值一次、内嵌 NUL、换行、引号、反斜杠和百分号的 S-expression 输出；源码测试将 LLVM O0/O3 与解释器结果逐字节对照。

这些例子使用已有不可变字面量后端，不是新增动态字符串库。`str` 的 backing character storage 来源分析仍未实现；启用 Copy/Drop 资源检查的字符串路径继续拒绝，不把底层静态字面量存储当作任意字符串视图的生命周期证明。`development/host_examples.py string product-string-result --opt-level 3` 在 `/home/fin/luna` 退出 0，两项通过（`/tmp/luna-host-examples-2jujpm6s/summary.json`）。

在 `/home/fin/luna` 经 `development/check.py` 执行 `test -p luna --test host`，454 项通过，退出 0（`/tmp/luna-tools-g2nqx32k/result.json`）；新增对照含 O0/O3、UTF-8、嵌套和空匿名 record。格式化入口退出 0（`/tmp/luna-tools-6mbxoi8a/result.json`），`git diff --check` 通过；dev 未提交。

## 捕获闭包中的指针效应请求

重复调用复现：[host-repeated-pointer-effect.fl](repros/host-repeated-pointer-effect.fl)。同一个 Action 在 apply 中调用两次；第一次请求正常完成，第二次可从安装函数词法返回。两次 handler／callback 清理加 wrapper、installer、caller 共七个顺序检查，不能复用第一次调用的已消耗清理状态。

身份转发证明现允许通过已有 pointer position 读取基础标量以选择分支；读取结果不因此成为完整输入，指针生产调用、写入和临时取址仍不在这一观察片段。该证明不赋予读权限、Copy 或生命周期，实际访问继续由资源／内存分析验证。此前仅用 counter.* 判断第二次调用也会丢失转发证明，从而错误拒绝 data-only delivery；新增用例覆盖正常完成／第二次提前返回和故意错误的最终析构断言，对照 checked 解释执行与 LLVM O0/O3。

验证目录 `/home/fin/luna`：经 `development/check.py` 执行 `test -p elaboration --lib effect_transfer` 两项通过（`/tmp/luna-tools-_cd1usk9/result.json`）；`test -p luna --test host --test effect_memory_transfers` 453+91 项通过（`/tmp/luna-tools-_4965eox/result.json`），均退出 0。来源未知的操作参数解引用仍拒绝，现在由 memory target 检查报错而不是身份转发检查；对应负例分别断言生产函数调用缺证明和 unknown memory target。格式化入口退出 0（`/tmp/luna-tools-od_q9q6o/result.json`），`git diff --check` 通过，dev 未提交。

高阶路径的独立复现为 [host-higher-order-pointer-effect.fl](repros/host-higher-order-pointer-effect.fl)：闭包作为 Action 参数交给 apply，apply 自己也持有带计数契约的资源。正常完成和词法返回必须依次清理 handler、callback、wrapper、installer、caller 五层。原生回归覆盖直接高阶调用、`F == Action` 约束的泛型调用和工厂返回闭包，分别核对正确／错误析构契约及不同返回指针目标；无约束 F 仍不能当作函数调用。这些路径使用已有实现，本轮新增的是五层组合验收，而非新的效应 ABI。

验证目录 `/home/fin/luna`：经 `development/check.py` 执行 `test -p luna --test host order`，23 项通过，退出 0（`/tmp/luna-tools-892c8idu/result.json`），包含新五层矩阵及共享检查器原有调用方；未重复全部前端测试。格式化入口退出 0（`/tmp/luna-tools-3ww2nowv/result.json`），`git diff --check` 通过，dev 未提交。

可运行入口：[host-effect-callback-cleanup.fl](examples/host-effect-callback-cleanup.fl)。运行 `python3 -B development/host_examples.py effect-callback-cleanup --opt-level 3`，会构建并执行包含正常完成／提前返回两条路径的程序，预期 `(bool true)`。此文件直接供源码回归引用，避免示例与测试各维护一套闭包实现；该例已加入默认示例集。

在 `/home/fin/luna` 执行新增示例 O0，退出 0（`/tmp/luna-host-examples-1ewk9ga3/summary.json`）；执行 `python3 -B development/host_examples.py --opt-level 3`，19 项全部通过（`/tmp/luna-host-examples-87h0tien/summary.json`），包括显式多包 math intrinsic／库协议接线。验证范围是当前可执行子集，不代表完整语言实现。

闭包构造的假想调用尚无安装方时，指针实参请求保留缺失的效应内存义务，不在这一阶段要求已存在的完整参数转发证明，也不据此生成结果来源。实际调用仍根据 slot、operation、ABI、signature 取得匹配证据；即使处于假想外层，只要已有安装输入，也必须正常验证其转发或 Copy 契约。

新增 host 矩阵将四层析构用例分别改为捕获闭包、局部具名函数值、运行时选择具名回调。正常完成与非局部返回均核对 handler → subject → installer → caller 顺序，合法／错误析构断言分别对照 checked 解释执行和 LLVM O0/O3。无安装证明的调用仍拒绝；handler 返回局部地址仍拒绝生成产物，既有输出文件不被替换。

在 `/home/fin/luna` 经 `development/check.py` 执行 `test -p luna --test host --test effect_memory_transfers --test drop`，退出 0：451 项 host、91 项效应内存、992 项析构测试通过（`/tmp/luna-tools-sti3r85i/result.json`）。格式化入口退出 0（`/tmp/luna-tools-14tpjs3t/result.json`），`git diff --check` 通过；改动保留在 dev，未提交。

## 词法返回与正常操作参数转发分离

循环退出现在也与正常操作结果区分：`break`／`continue` 不向被暂停请求交付结果，后续不可达表达式不能贡献参数来源。该身份分析不验证或创造循环目标；目标必须由已有词法作用域和资源清理检查证明，局部 loop 表达式仍不在此身份推导片段中。

`repros/host-pointer-loop-cleanup-order.fl` 检查正常完成、break 和 continue 的 handler → subject → loop owner → outer owner 析构顺序。host 回归还覆盖“内层只在 guard 命中时退出，外层 fallback 转发请求参数”，以及含指针 record 的正常／词法返回；每一路径分别检查合法计数和故意错误的析构断言。

验证在 `/home/fin/luna` 通过 `development/check.py` 执行：`test -p elaboration --lib effect_transfer` 2 项通过（`/tmp/luna-tools-0tanptak/result.json`），`test -p luna --test host --test effect_memory_transfers` 450+91 项通过（`/tmp/luna-tools-e2d1i2pq/result.json`），均退出 0。新增源码矩阵对照 checked 解释执行与 LLVM O0/O3；格式化入口退出 0（`/tmp/luna-tools-n4gl5ba_/result.json`），`git diff --check` 通过，dev 未提交。

`repros/host-fallback-cleanup-order.fl` 将 fallback 参数转发、捕获指针的词法返回和四层资源清理组合起来。handler → subject → installer → caller 的析构依次检查并推进同一个计数器；正常完成和提前返回都必须各执行一次。指针／聚合结果调用曾绕过普通调用的退出检查点，导致提前返回漏掉 installer 析构；现在来源分析的统一出口也发布该清理计划，并复用已有检查点避免重复。局部聚合构造的输入身份分析按字段顺序处理非局部退出，但不把构造结果当成完整输入转发，也不替代资源／内存验证。

本次验证目录 `/home/fin/luna`：经 `development/check.py` 运行 `test -p luna --test host --test effect_memory_transfers`，449+91 项通过（`/tmp/luna-tools-4e65h9qi/result.json`）；`test -p elaboration --lib effect_transfer` 两项通过（`/tmp/luna-tools-t4axzjo0/result.json`）。四层清理矩阵逐一核对两个分支、正确／错误的析构断言以及 checked 解释执行与 LLVM O0/O3。格式化入口退出 0（`/tmp/luna-tools-40_ggd32/result.json`），`git diff --check` 通过；dev 未提交。

部分 handler 的 fallback 合成现在保留“本地没有正常完成”的内部空集合，与来源未知分开；只有与已验证的外层输入转发集合合成后才发布非空证明。内层可仅在 guard 命中时词法返回捕获指针，否则由外层交付请求参数。源内存分析、handler completion 输入与借用捕获准入使用一致的合成规则，外层查找核对 slot、operation、ABI 和签名。未知本地完成或缺失外层证明仍拒绝。`test -p elaboration --lib effect_transfer` 退出 0（`/tmp/luna-tools-0pzu5q1e/result.json`）；`test -p luna --test host --test effect_memory_transfers` 448+91 项通过，退出 0（`/tmp/luna-tools-aazqtbsm/result.json`）。新增直接／跨函数请求矩阵在 checked 解释执行与 O0/O3 核对命中及 fallback 的不同写回目标，均经 development/check.py 执行。

effect_transfer 的控制流证明现在将词法 Return 视为不向请求交付结果的退出，不再让这一分支污染其他正常完成／resume 分支的输入身份。若求值 return 的操作数先发生 resume，仍保留该交付。仅做词法返回的 arm 可与转发输入的其他 arm 共存；所有 arm 都不正常完成时，不伪造参数转发证据。该证明只表示完整输入的身份，不赋予借用、Copy、析构或逃逸权限。

词法返回操作数现先作有预算的当前运行时作用域扫描：没有向本请求 resume 的路径时，其函数调用或新值构造不必满足正常完成的输入身份规则；若含 resume，则继续使用原控制流证明。嵌套 closure／handler arm 的 resume 属于独立作用域。新增构造 Watch 与调用 make_watch 两种拥有结果场景，与普通操作参数转发共存，析构分别写回选中目标且检查不能重复。扩展后的指针矩阵也包含 return identity(second)。`test -p elaboration --lib effect_transfer` 两项通过（`/tmp/luna-tools-8w9c5_ox/result.json`），`test -p luna --test host handler` 42 项通过（`/tmp/luna-tools-0zfo7_lx/result.json`），均经 development/check.py 执行且退出 0，源码正例对照 checked 解释执行和 O0/O3。普通完成路径中的未知调用仍不能被当成身份转发。

含借用捕获的 handler 现在也可使用已证明的完整操作参数转发结果布局，保留原捕获稳定性与词法返回来源检查；证明按 handler 缓存，避免按捕获重复遍历。新增六种源码组合覆盖直接／跨函数请求与尾表达式／显式 resume／独立提前返回 arm，两种执行路径分别写回不同目标，checked 解释执行及 O0/O3 一致。操作参数本身作为词法返回值的请求侧替换仍未实现。

验证目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p elaboration --lib effect_transfer` 退出 0（`/tmp/luna-tools-uwmnw088/result.json`），包含退出区分、return 操作数内 resume、预算耗尽后恢复。`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test effect_memory_transfers` 退出 0，446+91 项通过（`/tmp/luna-tools-dwze261l/result.json`）。格式化与 git diff --check 通过；dev 未提交。

## Computed pattern 的间接指针结果

`let _ and p.* is target = ...` 曾在只读检查后重新使用旧 flow 查询来源，丢失本次观察取得的间接借用证据。现在指针读取观察同时返回已检查的资源快照和 ValueOrigins，computed binder 继承该候选的借用根与来源；验证阶段不发布重复清理记录。普通只读观察继续检查 Copy、目标存活、指针证据与 caller 资源不变，不授予写权限。

该传递路径现统一用于所有已获准观察的指针结果，不再仅处理最外层 ReadPointer。新增 if、含局部绑定的块、match 与具名函数返回指针四种组合，分别核验两个分支的实际目标；具名 choose 的执行事件核验恰好一次调用。局部地址逃逸和块内隐藏写入保持拒绝。`python3 -B development/check.py --timeout 600 -- test -p luna --test drop --test host then_patterns` 退出 0，8 项资源测试与 2 项 host 测试通过（`/tmp/luna-tools-75ng2jd3/result.json`），原生矩阵覆盖 O0/O3。此次未重复全量检查。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 600 -- test -p luna --test drop --test host`，退出 0，990+443 项全部通过（`/tmp/luna-tools-b_elf_lk/result.json`）。新增用例覆盖二／三层指针、连续 computed pattern、guard 成功／失败分支、写入观察与已移动目标拒绝，以及 checked 解释执行与 LLVM O0/O3 对照。格式化与 git diff --check 通过；dev 未提交。

## 泛型闭包的具体定义与清理作用域

清理 Schedule 现在发布 ClosureDefinition，以完整 instance/context 关联调用时取得的具体闭包表达式；Display 输出为 `(closure-definition (instance ...) (context ...) (definition ...))`。LLVM 只在该定义与实例图中的完整表达式一致时选用机器体和清理计划，不再要求同一源码位置的所有泛型实例相同。handler 中复制的闭包语法也按完整定义查回注册的机器体。缺失、重复、伪造的定义证据拒绝；同一具体定义若有不同的逐调用清理计划，仍保留原拒绝边界。

验证目录 `/home/fin/luna`：`development/check.py --timeout 600 -- test -p luna --lib --test host` 中 54 项库测试与 440 项 host 测试通过，新增析构用例因赋值缺少空格触发词法解析而失败，整条命令退出 101（`/tmp/luna-tools-gjusclz8/result.json`）。库测试包含定义证据缺失／重复／伪造、预算耗尽后恢复及 Display；host 已通过同一 make<T> 的 bool/u8 实例在 checked 解释执行与 O0/O3 下的正例。

后续析构验收使用两个不同 nominal 捕获，分别要求 valid 为真／假；正常组合成功，分别违反两种析构条件的组合失败。`test -p luna --test host native_generic_closure_instances_preserve_distinct_owned_cleanup` 退出 0（`/tmp/luna-tools-xhyrqlej/result.json`），覆盖三次 checked 解释执行及六次原生执行。未再次全量运行。含指针捕获的版本曾因 observed pointer slot contents provenance is not established 被拒绝（`/tmp/luna-tools-yi3pbyzb/result.json`）。后续定位证明并非闭包返回来源丢失，而是析构谓词 `assumes self.target.*.*` 的间接借用观察路径未建立按需前提；此缺口现已修复。`development/repros/host-generic-closure-pointer-cleanup.fl` 已转为正向回归。

谓词 ReadPointer 先独立验证观察权限，再在资源快照上执行普通读取分析，复用间接借用前提取得和 observation_preserves_pointers 检查。预检查不发布重复运行时清理记录，实际分析仍核验 Copy、目标存活及资源不变。`python3 -B development/check.py --timeout 600 -- test -p luna --test drop --test host guard` 退出 0，120+7 项通过（`/tmp/luna-tools-9_f6w7ep/result.json`），包含 1/2/3 层指针、两种 guard 分支、恰好一次析构，以及既有写入／悬空／权限拒绝例。`test -p luna --test host native_generic_closure` 退出 0（`/tmp/luna-tools-6a1k5f8i/result.json`）：两组矩阵在 checked 解释执行与 O0/O3 验证拥有型捕获的析构，包括指针写回和失败条件。

## 返回条件来源的保存与变体感知合并

HandlerReturnValue 新增 pointer_guards，词法 return 使用有预算的快照复制保存返回值自身的条件来源；不复用挂起表达式的 guards。effect_return_checkpoint 在取得声明的函数结果类型时，复用既有 join_value_origins / join_result_pointer_guards，以类型布局和候选 variant domains 区分空载荷与未知来源。缺少声明结果类型的分析上下文仍保守合并；operation 参数的私有借用根替换尚未实现。

命名结果进入后置条件检查前，除原有指针来源外，现在同时安装返回值的 enum domains 与 pointer guards。清理和观察不再只能看到没有条件的物理槽。新增元数据的 Display 保持有序 S-expression，并检查含 guard 的嵌套输出括号平衡。

验证工作目录 /home/fin/luna：完整 drop 组 986 项通过、1 条旧事件次数断言失败（`/tmp/luna-tools-rupugs_x/result.json`，整条命令退出 101）。该断言仍按相同接口／实现谓词可去重计数，与此前 CMTT 契约边界修复及 docs/cmtt/index.md 的规则不一致；更新为正常路径 3 次（主体 assumes、继承实现条件、接口条件），有显式实现条件时 4 次，拒绝路径仍 1 次。`test -p luna --test drop generic_inline_effect_methods_preserve_interface_postconditions` 退出 0（`/tmp/luna-tools-kcu35bp4/result.json`），展开与未展开、通过与拒绝、显式条件有无的所有组合均核验原清理顺序，未改契约实现。

`test -p luna --test host --test effect_memory_transfers` 中 91 项效应内存与 438 项 host 通过；新增一条 dump 断言错误地期待 --analyze-resources 的根函数报告包含具名 callee 的内部 handler 证据，因此整条命令退出 101（`/tmp/luna-tools-2k7pfjoa/result.json`）。删除不成立的报告范围断言后，原条件载荷回归定向通过 checked 解释执行及 O0/O3（`/tmp/luna-tools-sqyj7prr/result.json`）。元数据格式定向测试通过（`/tmp/luna-tools-t81mn2xi/result.json`）。以上均经 development/check.py 执行，修正后定向命令退出 0；没有把失败命令称为全通过，也没有重复全部测试。

探索的指针 match 后置谓词尚不能自动证明；把 assumes 嵌进该谓词也超出现有支持的 effect-free predicate 片段。相关实验用例未作为支持能力保留，未放宽验证规则。最终格式化退出 0（`/tmp/luna-tools-uk6a9hjs/result.json`），git diff --check 通过。dev 未提交。

## 条件指针载荷返回的原生验收

新增 handler 返回 Choice.none / Choice.some(Watch) 的源码回归，Watch 拥有指向 caller 布尔存储的指针。安装函数带命名结果与 ensures true，结果经过后置条件处理后由 caller 丢弃。trial(true) 必须发生一次析构写回，trial(false) 必须保持目标原值；Drop 自身检查目标此前尚未置位。由此验证空载荷不会触发实际析构或伪造写回。

当前实现已能通过这一组合，没有复现来源保守合并导致的源码拒绝，因此本轮只增加回归，不为假设问题修改生产代码。该结果也不证明所有带条件指针来源的返回都已精确处理。

工作目录 /home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_direct_handler_conditional_pointer_return` 退出 0（`/tmp/luna-tools-zz85x_bi/result.json`），checked 解释执行与 O0/O3 原生执行均核验两个分支。首次用例生成脚本未匹配到原测试文本，故此前 `/tmp/luna-tools-sffynphf/result.json` 的过滤运行没有执行用例，不作为通过证据。最终格式化退出 0（`/tmp/luna-tools-3q30sjs6/result.json`）；git diff --check 通过。dev 未提交。

## 已知安装方捕获的指针返回来源

HandlerReturnValue 现在携带安装方相对的 ValueOrigins，并同步输出稳定的 S-expression origins 字段。词法 return 先检查目标没有随 handler 自身退出销毁，再要求来源属于已检查的捕获借用、仍存活且不是操作参数的私有虚拟借用根。嵌套 installation 向外传播返回证据时重复检查作用域边界；没有把 operation 参数的虚拟根编号当成安装方真实目标。

effect_return_checkpoint 不再向 check_function_result 传空来源。各候选的已知目标保守合并，某分支缺少叶来源时加入 unknown，避免一个分支的已知地址替另一分支作证；清除属于挂起表达式而非返回值的 pointer guards。合并后在安装方退出重新检查 retiring targets，再执行原函数结果／后置条件检查和清理。共享 Arc 快照保留，遍历及集合复制计入预算。当前未实现操作参数返回的请求侧替换，也未实现返回来源的精细条件 guard 合并；不声称所有带指针返回都已可用。

原 host-handler-borrowed-return.fl 复现现通过，并更新为正例：caller 接收 handler 返回的捕获指针后写回原变量。新增原生验收覆盖带指针字段的拥有型 Watch 返回后析构写回、两个候选指针来源、嵌套 handler 返回且外层剩余语句不执行。三个拒绝场景覆盖安装函数局部、handler 局部，以及合法参数来源混入即将销毁的来源；均保留旧原生产物。首次拒绝例测试的诊断断言过窄，现同时接收 memory contract 层先报告的 local memory target cannot escape 与 moves 的 escaping provenance，未放宽拒绝语义。

工作目录 /home/fin/luna：`python3 -B development/check.py --timeout 600 -- test -p luna --test host --test effect_memory_transfers`，437+91=528 项通过（`/tmp/luna-tools-kgvwd7u7/result.json`）；`test -p luna --test drop handler`，111 项通过（`/tmp/luna-tools-2lwjpg46/result.json`）；`test -p elaboration --lib handler_return_display` 1 项通过（`/tmp/luna-tools-1ohqrqxy/result.json`）。随后新增嵌套安装测试，`test -p luna --test host native_direct_handler_returns_pointer_through_nested` 通过（`/tmp/luna-tools-_y2o77_a/result.json`）。所有命令退出 0。原生正例同时对照 checked 解释执行及 O0/O3。最终格式化退出 0（`/tmp/luna-tools-hvbvxt11/result.json`），git diff --check 通过。dev 未提交。

## Handler 指针返回的实际缺口定位

新增源码验收表明，具名函数的 pointer 参数被 handler 捕获并写回调用者存储已经可用；不需要放宽 stable_handler_pointees。`test -p luna --test host native_direct_handler_borrows_caller_parameter_storage` 在 checked 解释执行和 O0/O3 均通过，命令经 development/check.py 运行，退出 0（`/tmp/luna-tools-nsf1tj2f/result.json`）。

保留 `development/repros/host-handler-borrowed-return.fl`：forward 的 handler 返回捕获的调用者指针，caller 随后写回原布尔变量；预期 true。当前 `python3 -B development/check.py --timeout 300 -- run -p luna -- development/repros/host-handler-borrowed-return.fl --interpret run --copy-protocol Duplicate --drop-protocol Retire` 退出 1，准确报告 handler lexical return requires cross-scope pointer result evidence（`/tmp/luna-tools-mlwi_0_8/result.json`）。这是未实现的合法能力，不作为长期拒绝语义固定下来。

源码定位：moves::HandlerReturnValue 只保存 callable/fields/variants；effects::effect_return_checkpoint 对合并结果调用 check_function_result 时传入空 ValueOrigins。handler frame 的捕获指针沿用安装方目标，而 operation 参数另外分配虚拟借用根并在 report.borrowed_roots 记录 Parameter/input_fields。实现必须区分这两类来源，在真实 request 边界完成参数来源替换，并在安装方退出时重新排除即将销毁的目标；还须保留条件指针路径和未知分支，不能仅删除当前拒绝或直接合并不同 frame 的根编号。当前未修改此安全边界。

工作目录 /home/fin/luna。格式化退出 0（`/tmp/luna-tools-1o240z4d/result.json`），git diff --check 通过。dev 未提交。

## 拥有型错误载荷的跨 handler 交付验收

新增 8 个源码组合：direct handler 跨 subject 的机器调用返回错误或成功，经 forward 的 ! 传播把错误行扩展为 Other/Failure，再直接丢弃结果或通过错误消去提取拥有型 Watch 载荷。Watch 没有 Copy 实现，其 Drop 检查 valid。valid/failed 的四种组合分别确认有效错误载荷正常清理、无效错误载荷在清理时失败，以及成功路径不执行不存在的错误载荷析构。错误消去路径同时核验 success true / error false 的可观察结果，不能仅靠“程序没崩溃”通过。

工作目录 /home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_cross_function_direct_error_return` 退出 0（`/tmp/luna-tools-_hyi6qw8/result.json`），1 项矩阵测试覆盖 8 次 checked 解释执行及 16 次 O0/O3 原生执行，包括预期析构失败。当前实现已满足这些组合，本轮未修改生产 lowering。格式化退出 0（`/tmp/luna-tools-6z0d88yd/result.json`），git diff --check 通过。未重复全量测试。跨 scope 指针返回的证据传递仍是独立未实现边界；本测试不声称解除它。dev 未提交。

## 丢弃错误值时的 Never 清理标记

构造 error 值不会发起 effect，也不会自动 return。新增组合验收时发现：错误值被丢弃后，逻辑清理树仍保留不可构造的成功载荷 Never；LLVM 准入将其 Release 标记误当作实际存储要求，拒绝合法程序。独立拥有型载荷复现报告 `/tmp/luna-tools-jfdm86xh/result.json`，退出 101。

validate_cleanup 现在不要求 Never Release 标记具有值布局。它只释放逻辑位置、不构造或读取 Never；实际 owner/program 的存储与 Invoke 仍照常验证。repr 的 Never 拒绝保持，新增内部测试确认普通 Never 以及包含 Never 字段的记录仍没有运行时值布局，错误结果的不可到达成功占位也没有改变这一点。

三个源码场景验证直接丢弃、绑定后丢弃、放入结构记录后丢弃错误值。每个错误载荷拥有 Watch，其析构检查此前尚未执行并置位，最终结果确认析构已经发生；checked 解释执行和 LLVM O0/O3 一致。另有 direct handler 通过两层函数以 return error 返回错误结果的用例，错误路径交付 false、成功路径交付 true，验证 effect 控制通道与用户错误结果 ABI 的组合。

工作目录 /home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi`，53+431+7=491 项通过，退出 0（`/tmp/luna-tools-tzewxaap/result.json`）。随后增强错误／成功结果区分断言，定向 `test -p luna --test host native_direct_nonlocal_error_return` 通过（`/tmp/luna-tools-twehv99_/result.json`）。格式化退出 0（`/tmp/luna-tools-4k_vajqs/result.json`），最后仅修改 fixture 字符串；git diff --check 通过。dev 未提交。

## 控制目标分析的边界与预算

outline 现在只继承其自身树内声明且确实需要动态接收的循环目标。远程目标继续以环境 token 传递，不将整个安装函数的目标集合复制到每个机器 handler。集合处理纳入共享 host fuel。

新增四项内部分析测试：同 scope 的循环 break 与跨 handler 的 continue；闭包捕获初始化式参与安装方分析而闭包主体独立；pattern observation／guard 控制节点不漏扫；64 层嵌套在按节点数给定的线性预算内完成，低预算拒绝后能在原不可变输入上重试。这些是 IR 遍历边界测试，不代表任意构造的 observation/guard 控制语法都获源码准入。源码合法性与机器行为仍由既有 native 测试验证。

工作目录 /home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi`，52+429+7=488 项全部通过，退出 0（`/tmp/luna-tools-lhf9bhm6/result.json`）。最终格式化退出 0（`/tmp/luna-tools-zgljqevm/result.json`）；git diff --check 通过。dev 未提交。

## 按 handler 引用生成循环控制目标

checked 循环不再无条件分配 break/continue token。统一 control_destinations 分析按词法 handler scope 收集实际引用的循环 LocalId，排除同一 handler scope 内声明且直接退出的局部循环；嵌套 handler 独立排队，因此仍能识别它对外层 handler 局部循环的非局部退出。闭包主体由其自己的机器函数分析，outline 入口继承安装方的目标集合。普通循环和仅转发效应的循环不生成目标比较／接收分支，待传播控制仍沿原 EffectEscape 清理。返回目标分析也由此共用同一次有预算的 scope 遍历。

新增 ABI 验收检查普通循环没有 bool token alloca、转发循环没有指针目标比较，真正安装方仍生成接收分支；O0/O3 均执行 true。新增源码测试在闭包内安装 handler，跨函数先 continue 再 break，验证闭包目标没有丢失或误归属外层函数；checked 解释执行与 O0/O3 一致。

工作目录 /home/fin/luna：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi` 中 48 项内部、428 项 host、原 6 项 ABI 通过；新增 ABI fixture 最初误写 loop 源语法而解析失败，整条命令退出 101（`/tmp/luna-tools-2508js3k/result.json`）。修正为 while true 并加入闭包测试后，`test -p luna --test host --test host_direct_abi destination` 共 6 项通过，退出 0（`/tmp/luna-tools-acdr64i5/result.json`）。未更改解析器或语言规则。最终格式化退出 0（`/tmp/luna-tools-emk_7aed/result.json`）；git diff --check 通过。此处缩减生成 IR，不声称有运行时间基准收益。dev 未提交。

## 多包可执行示例与编译器复用

统一 host_examples.py 入口现在支持显式包身份与依赖边，多包 math-fma 绑定 library/math 的全部 23 个 Scalar intrinsic；新 library-order 示例使用真实 comparison/resources 包、Ord + Copy 泛型约束、父 trait 默认方法以及 i128 极值比较。另将已有 error-product、staged-callback-order 接入，默认共 17 个可执行示例。summary.json 保存完整包输入与运行选项，不通过拼接源码代替包编译。

实际构建日志确认缓存回收会删除 luna 产物，原先每个示例的 cargo run 因此反复链接编译器。新增 build_host_driver.py 通过同一个 Runner 构建一次，在维护锁内、回收之前复制本轮可执行文件；失败构建不发布，已有目标不覆盖，复制失败清除不完整文件，发布失败仍执行缓存维护。随后所有源码构建复用本轮私有 luna，均继续由有超时的 process_tools 记录命令与输出。此优化没有禁用缓存回收，也不复用可能过期的旧二进制。

工作目录 /home/fin/luna：`python3 -B development/host_examples.py --opt-level 3` 全部 17 例通过，退出 0，证据 `/tmp/luna-host-examples-obntt6u2/summary.json`；新增四例与原 arithmetic 的 O0 子集 5 例通过，证据 `/tmp/luna-host-examples-mpume4ut/summary.json`。缓存入口 13 项测试通过 `/tmp/luna-tools-wrsqe14n/result.json`，快照发布 3 项测试通过 `/tmp/luna-tools-su8j0gni/result.json`（均通过 command.py 调用 unittest，退出 0）。git diff --check 通过。本轮未修改 Rust 编译器语义；完整 sysroot I/O 与 capturing/async ABI 等边界仍未解除。dev 未提交。

## 按需生成函数返回目标

此前每个 checked 机器函数都分配 return token／返回值槽；即使它只是普通返回或只转发效应，也会生成接收本函数退出的比较和返回分支，额外复制后置条件降码。现在使用有预算的 scope 遍历，仅在本函数的 handler 中出现非局部 Return 时建立目标。独立闭包主体不进入父函数扫描，按其自己的机器体分析；普通函数 Return 不需要此存储。handler arms、guards、pattern observations 各排队一次，嵌套 handler 不反复扫描完整子树；默认参数不借用调用者的控制归属。

新增 LLVM 结构和原生验收：普通 bool 函数只剩一个参数 alloca（没有额外 token／结果槽），仅转发函数保留 flurry_effect_control 检查但没有目标相等比较，真正安装方仍有接收分支；O0/O3 执行结果保持 true。另有闭包内部安装 handler 并 return false 的源码，外层继续执行 not action()，验证目标属于该闭包而非外层函数。现有跨函数 return、loop exits、清理与失败测试同时通过。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi`，48+428+6=482 项通过，退出 0（`/tmp/luna-tools-c2z1foi4/result.json`）。格式化退出 0（`/tmp/luna-tools-b8s1ses9/result.json`），git diff --check 通过。此处减少无用 IR，不声称取得运行时间基准提升。dev 未提交。

## 参数准备中的控制退出验收

核对 docs/functions/parameters.md 和 contracts-and-effects.md：runtime 默认值必须 P3 且有限完成，函数主体的效应行不自动授权默认表达式。资源层也保留 effectful default requires caller escape routing 边界，因此没有仅为放开 LLVM 路径而绕过定义侧要求。新增源码拒绝例同时覆盖缺省调用和显式覆盖，均精确报告 effect request requires an explicit allowed effect row，旧产物保留。

合法的显式实参现在有完整组合验收：在第二个参数求值时，由跨函数 handler 触发 return／break／continue，先前准备的拥有型参数各析构一次，后面的参数副作用与 callee 主体均不执行。具名 optional 参数配合 anchored 参数覆盖声明索引与源求值顺序不一致的情况；optional 声明保留合法默认值。临时拥有型闭包作为 callee 时，在参数阶段退出也会销毁其环境，且闭包主体从未执行。六个参数场景加两个闭包场景，checked 解释执行和 O0/O3 均通过。当前实现已满足这些组合，无需更改生产 lowering；本轮增加行为证据，未宣称解除默认值的语义／实现边界。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_direct_control_during_arguments` 两项矩阵测试通过（`/tmp/luna-tools-sz96aem0/result.json`）；`test -p luna --test host native_effect_rows_do_not_authorize` 一项双分支拒绝测试通过（`/tmp/luna-tools-lxkg0_vk/result.json`），均退出 0。曾误将 optional 参数写成无默认值的声明，语法拒绝后修正 fixture，不改语言规则。git diff --check 通过。dev 未提交。

## Ordinary direct 跨函数 break／continue

checked loop 为 break 和 continue 分别建立当前调用帧内的非零大小 token。handler 的环境只携带其实际引用的目标 token，跨函数不传 LLVM label；本地循环仍直接使用本函数标签。源码 LocalId 决定词法目标，动态 token 决定目标调用实例。nested outlined handler 继续传递继承的远程目标。

独立线程控制通道统一改名为 flurry_effect_control，return 和循环退出共用目标指针路由，仍与普通错误结果、契约失败分离。调用后的检查先判断本函数返回目标，再判断当前可见循环目标；跨函数中间帧执行 EffectEscape，接收方执行准确的 EffectBreak／EffectContinue 后跳转。需要穿过词法 handler 时，先清理其中间 scope，再恢复目标循环所在帧。循环外仍存活的拥有者不被当作函数退出资源销毁。环境复制、目标扫描和 dispatch 分支继续计入 host fuel。

源码测试覆盖两轮 continue／break 的六次清理（handler、请求方、循环内各一次）、循环外拥有者保留、被调用函数自身也有循环而不误接收外层 token、目标位于另一个 outlined handler 的局部循环，以及经过词法 handler 时四层资源各一次、合计八次析构。解释执行和 O0/O3 一致。未启用资源检查的目标仍拒绝，并保留旧产物。跨默认参数边界的控制、affine 捕获、capturing/async ABI 仍待补。

新增 development/examples/host-direct-loop.fl，默认 runner 现十三例。最初测试压缩写法遗漏 Flurry 运算符两侧空格，修正 fixture 后通过，未更改语法。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi --test effect_memory_transfers`，48+423+5+91=567 项通过（`/tmp/luna-tools-f_kvrni7/result.json`）。之后新增词法 handler 验收并加强循环外资源断言，`test -p luna --test host native_cross_function_direct_loop_exits` 三项通过（`/tmp/luna-tools-5w_uc9gj/result.json`），均退出 0。格式化 `/tmp/luna-tools-cocl09o6/result.json` 退出 0；git diff --check 通过。dev 未提交。

## Ordinary direct 跨函数非局部 return

checked host pipeline 现在为每次机器函数调用分配独立的返回目标 token 与该函数返回类型的存储。outlined handler 环境借用这两处存储；handler return 先交付值并完成本分支清理，再通过独立的 thread-local 指针通道 flurry_effect_return 标记目标。入口仍按 operation 的 Y 返回占位值，调用者在使用这个值之前检查控制通道。中间调用沿 EffectEscape 清理并直接传播，不执行自己的普通后置条件；目标安装方清空通道，恢复所属函数资源帧，再沿 EffectReturn 检查并交付真正返回值。用户 ErrorResult 和 flurry_failed 均不承载该控制。

token 是当前调用帧中非零大小存储的地址，递归实例不会仅凭源码位置被混同。析构调用期间暂存并清空外层控制，检查析构完成后没有遗留新退出，再恢复外层请求；已有析构失败仍 trap。checked 根入口同样保存外层通道，检查本次调用已经消化自己的控制，再恢复外层状态，未消费的控制不作为普通结果发布。

验证覆盖两层具名／函数值调用、操作返回 u128 而安装方返回 bool、handler／请求者／转发者／安装方四层析构各一次且严格按序、递归安装身份、嵌套 outlined 与词法 handler、安装方后置条件及中间正常逻辑不执行、无指针拥有结果交付后由调用方析构（非法值须 trap）。原生 ABI harness 在真实析构入口插入控制状态检查，O0/O3 均证明清理调用未观察到外层待传播请求。旧 LLVM 相邻行断言改为核验清理检查、存活标记更新和失败传播的顺序；额外控制保存／恢复指令不再被当成回归。

限制：该路径必须经过 Copy/Drop 资源检查，未检查入口明确拒绝。跨默认参数求值边界的目标接收仍拒绝；安装方循环的非局部 break／continue、affine 捕获与 capturing/async ABI 尚未实现。带指针的拥有返回值仍可能被前端以缺少 cross-scope pointer result evidence 拒绝。析构内部安装 effect 的原始源码实验也被现有 stable-pointer 检查拒绝；这里仅以 ABI instrumentation 检查隔离，不声称解除前端限制。最初析构契约直接观察二重解引用未能建立 provenance，测试改为先加载 bool 再检查；后置条件例也补上正常路径必要的 assumes，没有放宽源契约检查。

新增 development/examples/host-direct-return.fl，默认 runner 现为十二例。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi --test effect_memory_transfers`，48+421+5+91=565 项通过，退出 0（`/tmp/luna-tools-8ftzofov/result.json`）。随后仅细化未检查入口诊断，定向 `test -p luna --test host native_cross_function_direct_return` 通过（`/tmp/luna-tools-nayq3c_h/result.json`）。格式化退出 0（`/tmp/luna-tools-ks_tk3z3/result.json`）。dev 未提交。

## 跨函数非局部退出的清理边界准入

推进非局部控制时复现：源码清理发布器已经为非空效应行的普通函数调用发布 EffectEscape／EffectReturn，而 LLVM 清理准入只把 direct request 和 direct-passing 安装视为边界。两层调用中持有 Watch 的例子因此被错误报告为 destructor invocation not implemented，尚未到达实际未实现的 control lowering。

新增 iir::effects::suspension_boundary，清理发布器与 LLVM 准入共同使用。范围保留 direct request、非空效应行调用（包含函数值／unsafe callable signature 包装）及 direct-passing 安装；空行普通调用不因此获得控制退出资格。这是可能的退出边界分类，不证明当前 handler 会退出，也不表示机器控制 ABI 已实现。provider 的 PredicateFailure hook 独立保留，避免共享分类后重复发布效应 hooks。

新增两层具名／函数值调用源码回归：checked 解释执行确实返回 true；LLVM 清理准入通过，随后准确报告 outlined direct nonlocal control is not implemented，并保留旧产物。基线失败证据 `/tmp/luna-tools-w9y42yal/result.json`；修复后定向通过 `/tmp/luna-tools-2xrtmkya/result.json`。下一步仍需独立控制状态、安装边界结果存储、逐层 EffectEscape 和最终 EffectReturn，不能借用用户 ErrorResult 或把 handler return 当作普通 Y。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi`，48+416+4 项通过，退出 0（`/tmp/luna-tools-lhhy6e6a/result.json`）；`test -p iir` 80 项通过，退出 0（`/tmp/luna-tools-_m1lgouh/result.json`）。合计 548 项。全 workspace 格式化退出 0（`/tmp/luna-tools-crm0nkxk/result.json`），git diff --check 通过。dev 未提交。

## 已检查的 Copy 聚合捕获

跨函数 direct outline 现在接受已取得资源证据的聚合／名义捕获，仍借用安装方原 slot。依据不是 LLVM 布局：moves::handler_resources 在取得 handler scope 时已检查每个自由绑定的实际 Copy 许可，记录 persistent installer storage；带指针字段的类型还须满足已知来源、稳定存活目标和指针槽不重绑定的条件。未检查入口继续限制聚合捕获。此改动不允许 affine 捕获，也不推导底层表示与 newtype 的 Copy 等价。

新增源码矩阵覆盖 struct 的字段修改和闭包隐式捕获、enum 载荷读取／整体替换，以及标量表示 newtype 的显式转换。两次请求观察同一存储的变化，安装结束后可见最终状态。逐项删除 Copy 实现均拒绝并保留旧产物。另有带指针字段 State 的写回和不相交数据字段修改；重绑定指针槽仍明确拒绝。正例 checked 解释执行和 O0/O3 均通过。最初 enum fixture 错写前缀 match，改回 Flurry 后缀 match 后通过，没有修改解析规则。

`host-direct-calls.fl` 示例改用 Control {next:bool,target:*bool}，结合共享捕获修改与每请求的局部析构。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test host_direct_abi --test effect_memory_transfers`，415+4+91=510 项通过，退出 0（`/tmp/luna-tools-9myfk4sd/result.json`）。示例更新后单独重跑 `test -p luna --test host native_cross_function_direct_handlers_share_captures`，解释执行及 O0/O3 通过（`/tmp/luna-tools-obqhz0a4/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-m3gzmaei/result.json`），git diff --check 通过。非局部退出及 affine 捕获仍待实现。dev 未提交。

## Outlined handler 的局部控制边界

outline 现在按 LocalId 收集本入口内部声明的循环，而不是一律拒绝 break／continue。对应目标由 child emitter 建立新的机器标签，沿当前 handler 的精确资源上下文清理；指向安装方循环的控制仍明确拒绝。扫描不进入新闭包的主体，因此闭包自身的 return 不再被误判为 handler 的非局部退出；显式收集闭包的隐式捕获，确保只在闭包中读取的安装方标量也进入 handler 环境。闭包捕获初始化式、pattern observation、默认参数和嵌套 handler 仍参与扫描。

新增源码测试覆盖循环 continue／break 各一次析构、捕获安装方变量的闭包提前返回后 handler 继续执行，以及 handler 自有循环不能授权退出安装方循环。正例的 checked 解释执行与 LLVM O0/O3 均为 true；拒绝例保留旧原生产物。跨函数 handler 自身的 return 及指向安装方的 break／continue 仍待独立控制传播实现。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi direct`，lib 1、host 50、ABI 3 项通过，退出 0（`/tmp/luna-tools-00ao0nyk/result.json`）；名称过滤未运行其它测试。全 workspace 格式化退出 0（`/tmp/luna-tools-ekuspfkh/result.json`），git diff --check 通过。dev 未提交。

## Ordinary direct 源码安装端：可恢复分支与借用捕获

现在跨函数传递词法 ordinary direct handler 时，会生成独立的 `Y (ptr environment, operation arguments...)` 入口及当前机器帧内的环境存储。捕获仅取实际引用的绑定，包括只写的 Assign／BitAssign／BitUpdate 位置和 pattern observation；当前允许原始标量／指针，保存原 slot 的地址，不把 mutable 绑定复制成快照。void 参数省略机器位置但保留逻辑 packet 索引；u128 实参和结果按原布局交付。每次请求调用入口。

入口环境同时供应外层 direct 入口／环境指针和 direct-passing 已安装值。handler 中请求沿外层环境运行，guard 未匹配时按已确定 fallback 槽转发。处理分支在安装位置的精确 cleanup context 中编译，普通完成与显式 resume 均清理分支局部资源；checked failure 返回请求方后逐层清理。新 ABI 测试从真实源码安装 handler，仅替换 LLVM main 以观察外部存储，验证成功／失败两种路径中 handler 和 requester 的析构各一次，O0/O3 均通过。

非局部 return／break／continue 仍明确拒绝（当前扫描也保守拒绝 handler 内局部循环的这些控制），尚未实现独立控制传输及安装边界接收。聚合／名义捕获仍需共享 ownership state，不能因 newtype 的机器表示为标量就放行。capturing/async ABI 未实现。outline 有共享 fuel 与 128 层递归限制。旧 inline-effect 测试由“残余 direct 必须拒绝”改为执行成功：正式文档 algebraic-effect.md 规定 inline 是意向，未展开时允许兼容的普通 evidence 路径；未修改语言承诺。

新增 `development/examples/host-direct-calls.fl`（两层调用、共享可变捕获、每请求 resume 清理），加入默认 runner，现在十一例。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi`，48+410+4=462 项通过，退出 0（`/tmp/luna-tools-kw16dx13/result.json`）。早期 broad run 中旧不支持预期失败，另有新 void fixture 漏写／未用别名指定结果类型的解析失败，修正 fixture 后重跑全部组。全 workspace 格式化报告 `/tmp/luna-tools-i3p3nqdk/result.json`；`git diff --check` 通过。示例实际 O3 构建运行报告 `/tmp/luna-host-examples-3teox4tj/summary.json`。dev 未提交。

## 具名机器函数必须有清理证据

为 ordinary direct 安装端补资源验收时，发现仅取函数值可物化需要调用上下文的函数，但未取得其资源清理计划。LLVM 此前将缺失计划当成无清理动作，照常生成函数体。原生 harness 调用带 Watch 参数的函数实际返回成功，两个析构写回均未执行。现在 checked 具名函数若未取得清理计划，在生成机器函数时明确拒绝 `host checked body ... has no acquired cleanup schedule`；不能凭空假设一个独立 handler 的内存环境。源码端的上下文供应仍待实现。未调用闭包的现有环境销毁路径维持原有处理，不将它误判成已取得具名函数调用证据。

host_direct_abi 新增该拒绝例，以及无需调用上下文的 owned 参数正例。后者用带契约的析构验证请求正常返回后仍执行析构：有效参数正常返回，无效参数按现有 cleanup ABI trap；两个变体均经 O0/O3。最初测试错误预期析构失败返回普通 failed 状态，实际 SIGILL；检查 cleanup emitter 的 flurry_check_failure 后修正预期，没有改动语言或弱化失败行为。此证据不证明普通 direct 非局部退出已实现。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test host_direct_abi`，lib 48、host 405、ABI 2 项通过，退出 0（`/tmp/luna-tools-7pp_tfbc/result.json`）。随后增加析构正／负例，单独执行 `test -p luna --test host_direct_abi`，3 项通过，退出 0（`/tmp/luna-tools-dzzoda35/result.json`）。全 workspace 格式化报告 `/tmp/luna-tools-tvb700l4/result.json`。dev 未提交。

## Ordinary direct 跨函数接收端（安装端尚未实现）

机器函数按声明行顺序接收 effect 参数：direct passing 沿用已有值参数；ordinary direct 使用两个隐藏指针（入口、环境）。参数名为 effectN_entry／effectN_env，N 保留逻辑行位置。入口签名是 `Y (ptr environment, runtime arguments...)`，void 参数不占机器位置，但请求 argument packet 保留逻辑字段索引。转发和行重排按完整 operation identity 选择这两个指针。

callee 请求现在可以经机器入口调用并接回 Y，每次请求均调用；不缓存 ordinary direct 的返回值。checked failure 在请求之后检查，并沿当前调用链停止／清理，独立于用户错误结果。此处只支持现有 checked failure 通道，不能据此声称 handler 非局部 return／break 或 continuation 中止已实现。

新增 `compiler/luna/tests/host_direct_abi.rs`：先从真实 Flurry 源码构建 checked IIR 和 LLVM 函数，再由显式 LLVM harness 供应普通 direct 入口。覆盖 direct/passing 混合行反序转发、u128 输入输出、void 参数省略、每请求调用、独立环境、O0/O3，以及入口报告失败后不执行第二次请求。harness 的两次独立入口调用之间重置 checked failure 状态；最初未重置导致第二次调用在 unit 参数准备阶段就停止，已修正 harness。

源码 handler 安装尚不能产生机器入口／环境。跨函数使用词法 ordinary direct handler 明确拒绝 `host ordinary direct handler outlining is not implemented`，旧产物保护测试保留。下一步需要提取 handler 函数及其安装帧环境，供应 handler 的外层效应行，并设计非局部控制在回到安装边界前的传播与逐层清理；不能把 lexical return 当作 callback 的普通 Y 返回。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host_direct_abi --test host direct`，实际 lib 1、host 41、ABI 1 项通过，退出 0（`/tmp/luna-tools-8ujhoyd6/result.json`）。check -p luna 退出 0（`/tmp/luna-tools-z2c7jtpx/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-3o40c1i6/result.json`），git diff --check 通过。dev 未提交。

## Host 集成：修复错误结果数据 pattern 的内存绑定

对近期效应布局／资源改动进行阶段集成时，`test -p luna --lib --test host` 的 lib 48 项通过，host 402 项通过／3 项失败（`/tmp/luna-tools-_xo3b9wx/result.json`）。共同错误为 memory match pattern requires a richer subject contract；涉及 void 错误传播、catch 提前退出清理与已有 host-error-product 示例。

根因是 ErrorResult 纳入 pointer-free data 后，普通匹配开始走 bind_data_pattern，该路径没有处理 ErrorResult 包装。现在复用 owned_variant_pattern 校验行标签、选择精确载荷类型，再递归绑定；Observe 也在数据路径完成内部 pattern 后移除临时别名。没有把错误 arm 当作任意匹配，也没有丢弃其 guards／写入。针对性 `test -p luna --test host error_` 18 项通过（`/tmp/luna-tools-7quz9p9w/result.json`）。新增内存摘要回归要求 before catch、普通 arm、after catch 的三个指针写入均保留，并验证预算失败后恢复。

新增测试首次把 `.* =` 写成 `.*=`，触发词法拒绝，已修正 fixture；失败报告 `/tmp/luna-tools-uptxmlyo/result.json`（lib 48 通过、内存 90 通过／1 fixture 失败），不计作实现通过。最终在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test effect_memory_transfers`：lib 48、内存 91、host 405 项全部通过，退出 0（`/tmp/luna-tools-x6_9w0jj/result.json`）。格式化退出 0（`/tmp/luna-tools-1qof1nys/result.json`），git diff --check 通过。dev 未提交。

## Direct passing 保存普通错误结果

pointer-free 数据布局检查现在递归处理 ErrorResult 的成功载荷和所有错误载荷；error-only 类型中的 Never 不占成功载荷。LLVM 沿用既有错误结果值表示，通过 direct-passing 隐藏参数交付已安装的值，不使用错误结果通道表示效应中止或 continuation 转移。此扩展不改变两种 ABI 的边界。

新增源码测试让 provider 安装一次 `!Failure bool`，Failure 携带 u128。被调用函数重复请求后分别处理普通错误；true／false 成功值与错误值均经过两层调用验证。错误结果类型本身必须具备 Copy，不能仅凭两臂载荷可复制推导；初始测试缺失该实现而正确拒绝，补上明确实现后 checked 解释执行与 O0/O3 通过。删除这条实现的负例继续拒绝，旧原生产物保留。带指针载荷、callable 和单次消费槽仍有独立边界。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test effect_contracts --test effect_memory_transfers --test host direct_passing`，实际执行的 host 18 项通过、退出 0，其余两组被名称过滤而未执行（`/tmp/luna-tools-org5qfdb/result.json`）。随后无过滤执行 `test -p luna --test effect_contracts --test effect_memory_transfers`，7+90 项全部通过、退出 0（`/tmp/luna-tools-644008nv/result.json`）；覆盖成功分支／任一错误分支中的指针拒绝，以及布局查询预算失败后的恢复。格式化退出 0（`/tmp/luna-tools-xnnhljvx/result.json`），git diff --check 通过。dev 未提交。

## 泛型聚合效应的高阶调用示例

新增 `development/examples/host-passing-aggregates.fl` 并加入默认示例 runner（现在十项）。同一个 setting<T> 声明分别提供布局相同但名义身份不同的 Config／Other；泛型 fetch<T> 直接返回聚合结果。运行时选择函数、效应行反序转发、闭包捕获及按值交给 apply 的路径同时核验两个隐藏聚合参数没有混淆。该组合通过现有实现，无需改变语言或绕过资源检查。

`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_direct_passing_generic_aggregates` 在 `/home/fin/luna` 退出 0（`/tmp/luna-tools-ayjaamvd/result.json`），checked 解释执行和 LLVM O0/O3 均为 true。整理示例格式后实际执行 `python3 -B development/host_examples.py passing-aggregates --opt-level 3`，构建和运行通过（`/tmp/luna-host-examples-z31vcqw6/summary.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-hj9epkrl/result.json`），git diff --check 通过。dev 未提交。

## Direct passing 的泛型／嵌套 Copy newtype

pointer-free 安装布局现在跟随已声明的 newtype representation edge。NIR 通过 `representation_budgeted` 验证并替换声明参数，再递归检查内部布局；IIR 检查已经物化的表示。这个结构视图只服务于布局／内存依赖检查，不改写名义类型、不展开普通 normal form，也不继承底层类型的 Copy 许可。外层 newtype 必须单独满足 Copy。

新增 `Outer -> Wrapped<Pair> -> Pair` 的跨函数源码测试，Pair 含 bool 与 u128；请求显式逐层转换、重复交付，checked 解释执行和 LLVM O0/O3 一致。去掉 Outer 的 Copy 实现仍拒绝，保留旧原生产物。包装指针／callable 的 newtype 仍不进入此路径，单次消费槽仍未实现。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host direct_passing`，16 项通过、退出 0（`/tmp/luna-tools-hpp1w5l8/result.json`）。`test -p luna --test effect_contracts --test effect_memory_transfers`，6+89 项通过、退出 0（`/tmp/luna-tools-ma3a5zpv/result.json`）；包括包装内部依赖拒绝、损坏递归布局的深度诊断和失败后重试。格式化退出 0（`/tmp/luna-tools-5znyebu8/result.json`），git diff --check 通过。dev 工作区未提交。

## Pointer-free Copy 聚合值的 direct passing

direct-passing 的安装值与跨函数隐藏参数现在支持指针无关的 Copy 数据聚合：递归检查 record、Tuple 的授权结构视图、struct 和 enum 的全部载荷。NIR 内存摘要复用 effect_data_layout，IIR 根据已物化声明检查具体布局，LLVM 按既有聚合机器表示传参。普通 normal-form 头不展开。此布局检查不证明 Copy；安装期仍要求实际 Copy 证据，移除 Config 的 Copy 实现会拒绝且保留旧原生产物。带指针或 callable 字段、newtype 以及需要单次消费槽的结果尚不在本次支持范围。

checked reference execution 现在显式取得并安装 effect_values::Plan，避免把 Copy 聚合值误当成单次消费。直接使用解释器 API 的调用方仍需提供该计划，with_cleanup_schedule 本身不凭机器布局授予 Copy。安装分支仍在主体／实参之前执行一次，覆盖后恢复外层值，调用参数按完整 operation identity 和效应行顺序传递。

源码测试覆盖含 u128 的 Config、两层调用与局部覆盖、重复请求、provider 的安装副作用、带空／载荷变体的 enum，checked 解释执行与 LLVM O0/O3 一致。IIR 测试覆盖嵌套 struct/enum、隐藏指针／callable 拒绝、零预算失败后重新检查。最初新测试误将普通 effectful 调用写为 request 语法，修正为正常调用；随后真实复现了缺失交付计划导致的 EffectValueConsumed，接线后通过。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host direct_passing`，15 项通过、退出 0（`/tmp/luna-tools-h6oltmn6/result.json`）；`test -p luna --test effect_contracts --test effect_memory_transfers --test effect_spreads`，6+88+21 项通过、退出 0（`/tmp/luna-tools-ybpfsqow/result.json`）；`test -p iir`，80 项通过、退出 0（`/tmp/luna-tools-srzoel5j/result.json`）。`test -p luna --test drop passing` 退出 0（`/tmp/luna-tools-nbezfdcz/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-4suj6vk2/result.json`），git diff --check 通过。dev 未提交。

## 前导零／尾随零计数的泛型包装

新增 target operation `ctlz`／`cttz` 和 `math.leading_zeros<T>`／`math.trailing_zeros<T>`，输入输出同为 u8/u16/u32/u64/u128。零输入返回位宽。LLVM 降码按 [ctlz](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-ctlz-intrinsic)／[cttz](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-cttz-intrinsic) 合同追加 `i1 false`，声明使用 `i1 immarg`；该标志不属于源操作实参，用户仍只传一个整数。source admission、泛型约束、具体签名验证沿用同一个操作类型域，拒绝浮点、有符号、指针宽度整数和额外实参。

库的完整绑定现在为二十三项，CLI help、包加载 fixture、library_check.py 与 library/README.md 已同步。新增 raw IR 检查覆盖全部位宽，原生 math 测试覆盖零／全一／最高位／混合位模式及参数恰好求值一次，启用 Copy/Drop 后 O0/O3 均执行成功。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol --test host integer_`，host 8 项与协议 3 项通过，退出 0（`/tmp/luna-tools-7v0dzm05/result.json`）。后续 `test -p luna --test host_intrinsic_protocol --test frontend_cli` 中 CLI 196 项全部通过，协议 28 项通过／1 项 FMA 测试临时目录碰撞失败（`/tmp/luna-tools-l77y7x5x/result.json`）。已将该测试的目录创建改为有上限的原子重试，保留其他运行遗留的目录。 重跑完整 host_intrinsic_protocol：29 项全部通过、退出 0（`/tmp/luna-tools-q5nbs1d8/result.json`）。格式化退出 0（`/tmp/luna-tools-7yeir38q/result.json`），git diff --check 通过。dev 未提交。

## 声明契约随效应行进入 IIR

默认九个示例已在 LLVM O3 下重新编译并实际运行，全部通过：`python3 -B development/host_examples.py --opt-level 3`；报告 `/tmp/luna-host-examples-rixjkk56/summary.json`。覆盖整数、浮点、模板、回调、宽整数、析构、direct effect 清理、跨函数 direct passing 和 inline effect 清理。

materialization 现在按完整 operation identity 收集被类型引用的效应声明，实例化其签名并把所需类型布局一起物化，保存到 `Package.effect_contracts`。即使函数只声明效应而主体没有 request、全图没有 handler，LLVM 仍能取得隐藏参数 ABI。IIR 验证声明契约与实际 occurrence 一致，拒绝重复／损坏契约；dump 使用稳定的 S-expression 表。原先仅凭 request/handler 重建契约导致的最小原生失败已修复。

在 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test effect_contracts --test instantiation` 中 effect_contracts 5 项全部通过，instantiation 54 项通过、2 项失败（`/tmp/luna-tools-65l1a1ko/result.json`），不能称为整组通过。失败是 trait contract 的拒绝源码范围及 inline 前后 predicate 次数不一致，已记录至前端检查点；本轮未判定其引入版本。`test -p luna --test host direct_passing` 13 项通过、退出 0（`/tmp/luna-tools-2kza2s1c/result.json`），包括新增加的未使用效应行原生测试。

`test -p iir` 在深层 staging 测试栈溢出退出 101（`/tmp/luna-tools-bpy46a2y/result.json`）。显式跳过 `nested_scalar_comparisons_and_blocks_reach_logical_depth_limits` 后，79 项通过、1 项过滤、退出 0（`/tmp/luna-tools-g0rdo80x/result.json`）；这不代表栈溢出已解决。全 workspace 格式化退出 0（`/tmp/luna-tools-1jz5grfk/result.json`），git diff --check 通过。dev 工作区未提交。

## Direct passing 闭包与泛型实例的集成

新增源码验收：闭包捕获 flag、普通 bool 实参和 direct-passing 隐藏 bool 参数独立变化；先借用调用闭包，再按值转移到 higher-order apply。另以同一个泛型 setting 声明的 bool/u128 实例核验行路由和具体 ABI，避免只按声明身份或同名选择入口。两组均通过 checked 解释执行与 LLVM O0/O3。

拥有 Watch 捕获（析构经指针写回）的闭包暴露了资源检查过强的问题：消费捕获后读取已安装 scalar direct-passing 值，仍对所有指针执行 ordinary handler 的稳定性检查。moves/effects.rs 的 `check_effect_memory_context` 现在仅在**匹配的 effect memory premise 已建立**、ABI 是 DirectPassing 且签名为零参数无指针标量／void 时免除请求期 handler-storage 检查。安装期检查、返回来源、资源转移和 Copy/Drop 规则不变。修复后实际调用／未调用两条路径的捕获清理均通过；再次调用已消费的捕获仍以 `pointer capture unavailable: NotPresent` 拒绝，旧原生产物保留。将同一测试改为普通 direct ABI 仍由原稳定性检查拒绝；本次不放宽普通 handler。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host direct_passing`，12 项通过、退出 0（`/tmp/luna-tools-kzs7xn7p/result.json`）；`test -p luna --test effect_memory_transfers --test effect_spreads`，88+21 项通过、退出 0（`/tmp/luna-tools-38nsksa6/result.json`）。拥有捕获初始复现的拒绝证据为 `/tmp/luna-tools-rgo6r3nw/result.json`，修复后的首次原生通过为 `/tmp/luna-tools-x2gdd7xx/result.json`。最初 scalar 闭包测试错误地多次按值传递闭包，已改正测试，未推定闭包隐式 Copy。格式化退出 0（`/tmp/luna-tools-o7jwm094/result.json`），git diff --check 通过。dev 未提交。

## Scalar direct passing 请求的独立内存足迹（已解除间接调用阻塞）

进一步分析确认上一条候选调用阻塞并非必须复制 handler 捕获前提：scalar direct passing 请求只交付已安装的标量，不在请求处调用 provider。memory/effects.rs 的 `require_effect` 现在优先使用显式前提；没有前提时，只对声明 ABI 为 DirectPassing、实例化后签名为零参数且无指针标量／void 结果的操作，推导请求侧空访问足迹。操作类型实参和签名有预算检查，request 继续核对实际 ABI、签名和行槽。普通 direct、指针结果及 owning payload 仍需要原有前提。

这不是 provider 纯度或 Copy/Drop 证明。安装分支仍正常遍历并记录读写、拒绝和清理；delivery 仍由资源分析单独检查。现有拒绝分析继续保守标记请求 may-reject，本次不从空内存足迹推导无失败保证。

`native_direct_passing_indirect_abi_with_resource_admission` 已替换旧的阻塞预期：两个 runtime 候选、caller/callee 不同行、实参请求都通过 Copy/Drop 检查，checked 解释执行与 LLVM O0/O3 一致。内存推导回归覆盖 bool/u128/f64 泛型实例、零预算失败后重试、provider 指针读写与 assumes 拒绝保留、普通 direct／pointer direct passing／伪造请求 ABI 拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test effect_memory_transfers --test effect_spreads` 退出 0，88+21 项通过（`/tmp/luna-tools-8qy9l5ll/result.json`）；`test -p luna --test host direct_passing` 退出 0，9 项通过（`/tmp/luna-tools-1baqvzvc/result.json`）。最初新测错误地期待 may-reject=false，已改为核验独立拒绝分析保持保守，未修改该分析规则。全量格式化退出 0（`/tmp/luna-tools-8rvtfgw3/result.json`），git diff --check 通过。dev 未提交。

## Scalar direct passing 跨函数机器调用

LLVM 后端现在消费 `iir::effects::Contracts`，为 effectful 函数和闭包机器入口按行顺序追加隐藏 scalar 参数。普通参数在前，closure 的环境指针仍使用既有位置，effect 参数在后；void occurrence 保留逻辑位置但不占机器参数。callee 将收到的值接入其请求槽，普通返回通道只返回 computation 的结果。该路径没有把 computation 当作可存储值，也没有增加语言签名语法。

`expr_with_effects` 将结果 computation 的目标环境单独向下传递：callee 表达式、普通实参、block statements、if 条件、match scrutinee／pattern／guard 使用 caller 环境；只有结果 computation／调用体取得选定环境。EffectHandle 的 provider 仍在外层求值一次，并且早于 computation 的所有实参。EffectRun 的槽映射只改变被执行的结果环境。行重排、子集转发、递归和局部覆盖已真实执行；普通 direct／capturing／async 的跨函数调用仍明确拒绝。入口适配器要求无普通参数且效应行为空。

新增 `development/examples/host-passing-calls.fl` 和 runner 的 `passing-calls` 项，默认示例集现在九项。执行：`python3 -B development/host_examples.py passing-calls --opt-level 3`。本轮实际通过，`/tmp/luna-host-examples-smpiithk/summary.json`。

checked 源码测试覆盖两层转发、递归、实参使用 caller 的不同槽、if/match 分支、行重排、局部覆盖后恢复、拥有参数析构写回，以及 u128/f64/void 混合隐藏参数布局。解释执行与 LLVM O0/O3 一致；原始 LLVM IR 也检查了 i128 普通参数及 i128/double 隐藏参数，void 槽不被伪造为 void 参数。

带效应的间接候选调用的机器 ABI 在未启用资源协议的源码路径通过 O0/O3；启用 Copy/Drop 时目前仍由资源分析的 `every callable candidate requires a memory contract: effect request requires an installed handler memory premise` 拒绝，旧产物保留。没有跳过该安全边界；候选 memory premise 传递需后续修复。该部分不能称为 checked indirect effect calls 已完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 最终广泛检查中 lib 48 项通过，host 395 通过／1 项旧诊断断言失败（`/tmp/luna-tools-gbedekwb/result.json`）。统一普通 direct 不支持的调用侧／定义侧诊断后，`test -p luna --test host native_cross_function_effects_still` 单项复核退出 0（`/tmp/luna-tools-_ufky2rf/result.json`）。前面的 lib 拒绝测试因 IIR 契约取得引入更早的验证而变化，已精确更新为缺失接口／参数绑定不匹配的 IIR 诊断；没有放宽拒绝。所有五个新增 host 行为测试均在上述广泛检查通过。全 workspace 格式化退出 0（`/tmp/luna-tools-ke1glidk/result.json`），git diff --check 通过。dev 未提交。

## 跨函数效应 ABI 的 IIR 契约查询

`iir::effects::Contracts::acquire` 提供绑定到不可变图的具体操作契约表；复用并加强原先只用于 validation 的 occurrence 合同检查。条目包含完整 operation（含泛型输入）、ABI 和具体签名。查询不能只按声明名或参数宽度匹配；row 查询按原顺序保留重复 occurrence。表的 source traversal 顺序稳定，Display 使用 `(effect-contracts (effect-contract ...))`，不依赖 HashMap 顺序。预算耗尽与冲突都不发布部分表，row 在分配输出前收费。

这是机器调用降码的前置接口，**跨函数 effect 机器调用仍未实现**。接线必须遵循 `iir::effects::mapped` 与解释器 `eval_mapped_computation`：callee、普通实参、block 前置 statements、if 条件及 match 观察仍使用 caller 环境，只将选定 row 交给结果 computation。当前 LLVM `EffectRun` 切换整段表达式的方式不能直接推广到跨函数调用。后续先按 direct passing 的安装值传递合同建立隐藏参数，再处理普通 direct handler 的入口／环境和非局部控制；不能把普通 error result 用作 effect 中止控制 ABI。

本次仅新增契约查询和验证，不产生 handler ownership 或 Copy 证明。IIR 没有出现的 operation 合同返回明确 missing；不从同名声明或布局猜测签名。后续跨函数降码需消费该接口，不能将本次测试视为机器 ABI 执行证据。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test effect_contracts --test effect_spreads` 退出 0，2+21 项通过（`/tmp/luna-tools-cw66fpbi/result.json`）。随后新增同声明两个真实泛型实例的独立签名回归，`test -p luna --test effect_contracts` 退出 0，3 项通过（`/tmp/luna-tools-w1husnoj/result.json`）。另覆盖 row 重排／重复、缺失泛型实例、不同图身份、冲突 ABI、零预算失败后重新取得以及稳定 dump。全量格式化退出 0（`/tmp/luna-tools-yenjr2v9/result.json`），git diff --check 通过。dev 未提交。

## 整数位操作的泛型 host 接线

新增 `bitreverse`／`ctpop` target operation，library/math 导出 `bit_reverse<T>`／`count_ones<T>`，输入输出均为同一个 u8/u16/u32/u64/u128 类型。语义分别沿用 [LLVM bitreverse](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-bitreverse-intrinsic) 与 [LLVM ctpop](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-ctpop-intrinsic)：反转整个整数的位序，或返回其中 1 的数量；零输入合法。CLI 使用 `--host-intrinsic bitreverse=math.target.Scalar.bitreverse` 和 `ctpop=math.target.Scalar.ctpop`。README、math 源码验收和绑定脚本同步为二十一项操作。

NIR 具体签名现在保存 Primitive 身份，而不只保存宽度；同宽度 u32／f32 不可互换。操作声明自己的源类型域，泛型体在 elaboration 中证明该类型命题，特化时重新检查；驱动重绑定仍要求原有同操作的 source admission。LLVM 使用 iN 或 fN 后缀，输入、输出和具体绑定必须一致。整数 dump 输出 scalar-type，既有浮点 dump 保留 float-width。没有把 BitVec、signed 或 target-sized 类型隐式纳入本次接口，也没有给解释器伪造目标 intrinsic 执行。

工作目录 `/home/fin/luna`，检查证据：

- `python3 -B development/check.py --timeout 300 -- check -p luna` 退出 0：`/tmp/luna-tools-sx3kl08k/result.json`。
- `test -p luna --test host_intrinsic_protocol --test host integer_` 中原生 host 8 项通过，包括实际 math 包的五种宽度、零／全一／最高位／非对称位模式、参数求值一次，O0/O3 都执行成功。协议组的一个负例最初写错 bool 语法，整条命令退出 101：`/tmp/luna-tools-leot1cnx/result.json`；修正后确认由类型要求拒绝。
- `test -p luna --test host_intrinsic_protocol --test frontend_cli` 退出 0，27+196 项通过：`/tmp/luna-tools-ymp0uj98/result.json`。
- 补充同元数、不同操作／类型域的重绑定拒绝和清空证据后恢复拒绝；`test -p luna --test host_intrinsic_protocol integer_` 退出 0，3 项通过：`/tmp/luna-tools-z03ktj21/result.json`。
- 全量格式化退出 0：`/tmp/luna-tools-cdv2mw0i/result.json`；`git diff --check` 通过。改动保留 dev，未提交。

## Intrinsic 名称统一与示例验收

`nir::intrinsics::Operation` 统一提供操作名称和反向查找，CLI 注册与 LLVM 符号生成共用该定义；未知名称诊断从注册列表生成。既有 NIR dump 中 fused-multiply-add／square-root／copy-sign 的拼写保留，避免改变已有输出契约。名称不参与源码 intrinsic tag 的身份判定。

工作目录 `/home/fin/luna`，本次检查均退出 0：

- `python3 -B development/check.py --timeout 300 -- test -p nir operation_names_tests`：名称唯一性、往返及非法名称拒绝；`/tmp/luna-tools-3kmuabsj/result.json`。
- `python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol --test frontend_cli`：25+196 项通过；`/tmp/luna-tools-742rvyhb/result.json`。
- `python3 -B development/host_examples.py --opt-level 3`：默认八项全部实际编译、链接、执行及比对输出通过；`/tmp/luna-host-examples-ppq75pfi/summary.json`。
- 全 workspace 格式化通过，`/tmp/luna-tools-qb8mo0jg/result.json`；`git diff --check` 通过。改动留在 dev，未提交。

## 浮点 minimum／maximum

新增 `--host-intrinsic minimum=<selector>` 与 `maximum=<selector>`，NIR 注册、泛型签名特化、Display 及 LLVM f32/f64 降码均接通。library/math 提供 `minimum<T>(a,b)`、`maximum<T>(a,b)`，沿用 `T == f32 or T == f64` 的类型域。真实包消费者、验收脚本和 README 绑定表已同步为十九项操作。

按 [LLVM 21 合同](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-minimum-intrinsic)，两者传播 NaN，负零小于正零；不约束 NaN 载荷。没有启用 fast-math。原生测试覆盖两种精度、两种零的实参顺序、普通数值、无穷及 quiet/signaling NaN 的结果分类，LLVM O0/O3 均通过。真实 math 包的 single_precision／double_precision 测试新增泛型调用。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_minimum_maximum` 退出 0（`/tmp/luna-tools-evihs7gd/result.json`）。`test -p luna --test host_intrinsic_protocol --test frontend_cli` 最终退出 0，25+196 项通过（`/tmp/luna-tools-qizjil2b/result.json`）；首次失败是两个 CLI fixture 漏掉新增绑定前的选项名，修正后通过。随后扩展二元操作拒绝回归，`test -p luna --test host_intrinsic_protocol binary_math_requires` 通过（`/tmp/luna-tools-85oanbnq/result.json`）。格式化退出 0（`/tmp/luna-tools-qwp0nymu/result.json`），git diff --check 通过。dev 未提交。

## 资源示例运行入口

`development/host_examples.py` 现在为每项示例保存独立构建选项，默认包含八项。新增的 destructor、direct-effect-cleanup、inline-effect-cleanup 自动启用资源协议，最后一项同时启用 inline 展开。汇总记录优化级别与实际选项，便于区分资源检查路径和未启用协议的执行。

本轮在 `/home/fin/luna` 执行 `python3 -B development/host_examples.py destructor direct-effect-cleanup inline-effect-cleanup --opt-level 3`，退出 0，三项实际构建、链接、执行及输出比对全部通过；证据 `/tmp/luna-host-examples-ynbkerqm/summary.json`。未重复已验证且本轮未改动的编译器全量测试，dev 未提交。

## Checked direct handler 源码路径

资源检查通过的词法 direct handler 现在可进入 LLVM 后端；不再整体拒绝带清理计划的 handler。各 arm 使用精确的安装上下文，普通完成、resume、词法 return/break/continue 和失败清理沿已发布的 schedule 执行。新增 `development/examples/host-direct-effect-cleanup.fl`，可用文件开头的命令编译运行；示例在安装前建立稳定指针，由最终 handler 捕获，析构写回 seen；结果同时核验 resume 值和清理副作用。

源码验收覆盖正常完成、resume、函数 return、break、continue，每条路径分别验证析构谓词成功与失败，并与资源检查后的解释执行比较；LLVM O0/O3 均通过。底层测试继续覆盖嵌套帧析构顺序。跨函数效应环境 ABI 和跨默认求值的非局部退出仍未实现，不能把本项解释为一般代数效应已完成。

稳定指针在安装前建立、由 handler 捕获并用于析构写回的路径已通过；零／一／两层转发、正常完成／resume 均与解释执行一致（O0/O3）。带指针参数用于 handler 局部析构的路径现也已接通：存在隐式清理时，为所有输入指针叶建立保守读写前提，再由资源分析核验 Copy、目标存活及实际 Drop 访问；摘要继续标记可能运行清理。这不是完整、精确的析构内存摘要，可能要求比实际访问更多的输入能力。handler 中对词法捕获取址被 `handler lexical capture address exposure requires a memory contract` 拒绝。捕获取址的拒绝仍保留来源／契约检查及旧产物保护回归，后续需补其独立内存协议。

本轮 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，48+376 项通过（`/tmp/luna-tools-knsz80qu/result.json`）；随后新增仓库示例验收，`test -p luna --test host shipped_direct_effect_cleanup` 退出 0（`/tmp/luna-tools-c6epl2_l/result.json`）。格式化退出 0（`/tmp/luna-tools-oae9sfhq/result.json`），git diff --check 通过；dev 未提交。

## 2026-10-24 可直接运行的示例入口

```sh
python3 -B development/host_examples.py --opt-level 3
```

该入口逐项构建、链接并执行 arithmetic、floating、template、callback、wide-integer、destructor、direct-effect-cleanup、inline-effect-cleanup，核对原生输出；可在命令末尾选择子集。每项的可执行文件、构建／执行日志及总 `summary.json` 保留于打印出的临时目录，构建和执行均有超时，失败立即退出。复用已有 LLVM host 实现及 Cargo 管理入口，不依赖完整 sysroot。

最近一次复验：在 `/home/fin/luna` 执行上述命令，退出 0，五项全部通过；本次构建与实际执行证据位于 `/tmp/luna-host-examples-nhuciahf/summary.json`。词法 direct handler 的 checked 源码准入现已接通，具体证据和剩余边界见下节。完整 sysroot 与一般效应 ABI 仍待继续实现。

本轮在 `/home/fin/luna` 实际执行上述命令，五项均通过（O3），产物及证据 `/tmp/luna-host-examples-9f3wgxu6/summary.json`。另执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host shipped`：退出 0，31 项通过，报告 `/tmp/luna-tools-em7umqoa/result.json`。这验证现有工作区的原生示例路径，不表示以下全部后端边界已解除；未执行全 workspace 测试。工作区保留在 dev，未提交。

入口是 `luna input.fl --build run --output /tmp/program`。前端完成 elaboration 和实例化后，后端直接消费当前 IIR，生成 LLVM IR，经 `llvm-as` 验证、`opt` 优化与逐 pass 验证、`llc` 生成本机目标文件，再调用 C 链接器。编译时不解释执行入口，也不把解释器结果写成常量程序。

现有 `compiler/codegen` 依赖旧 MIR/ty，暂不复活；新实现位于 `compiler/luna/src/host/`。没有新增 Cargo 依赖。默认工具是 `llvm-config-21` 所指目录内的 LLVM 工具和 `cc`；可通过 `LUNA_LLVM_CONFIG`、`LUNA_CC` 分别指定可执行文件路径。本次环境 LLVM 21.1.8 和本机 C 链接器可用。无版本后缀的 llvm-config 指向另一版本，因此不默认使用它。

## 运行

在仓库根目录：

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-arithmetic.fl --build run --output /tmp/flurry-arithmetic
python3 -B development/command.py --timeout 10 -- /tmp/flurry-arithmetic
```

最后一条的 stdout 日志应为 `(unsigned-integer 42)`。同样可构建 `development/examples/host-template.fl`，其输出为 `(bool true)`。包装器返回日志路径；使用 run 在缓存维护前调用编译器，避免 build 后清理删除 luna 二进制。生成的 /tmp/flurry-arithmetic 不属于 Cargo 缓存。

检查目标代码：

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-arithmetic.fl --emit-llvm run --output /tmp/flurry-arithmetic.ll
```

`--emit-llvm` 在前端处理前查询 LLVM host 布局；未显式指定 `--opt-level` 时不运行优化 pass。它导出标准 LLVM 文本，不更改现有各层 IR 的 S-expression dump。`--fuel` 限制实例化及降码工作，默认 100000；它不是原生程序的运行时步数限制。

## 已支持与边界

- void、bool、f32/f64、固定宽度整数；char 内部使用 i32 表示。入口调试输出支持 void、bool、已接纳宽度的整数／BitVec 十进制值和浮点原始十六进制位模式；char 输出 `(char 码点)`，保持与普通无符号整数的区别。
- 局部变量、根位置赋值、分支、短路逻辑、循环、break/continue、提前 return。标量模式匹配及 let-else 见下节。
- 直接调用、普通函数值间接调用、递归、命名实参源序、默认实参及入口默认值包装；实例化后落入上述子集的模板和 comptime 类型输入。数值捕获闭包的构造、移动、返回、高阶传参与有状态调用已接通，配对资源协议下环境清理及拥有一致清理计划的闭包调用已接通；普通函数／lambda 的嵌套调用已接通；同一 callee 的具体祖先清理别名已接通；一致的默认参数子计划已接通；捕获移出、handler 子上下文等边界见末节。
- 整数四则运算、余数及比较。前端仍要求合法范围证明；LLVM 加减乘额外保留溢出检查，除法/余数检查零及有符号溢出，失败执行 llvm.trap。
- 前后置谓词、入口快照和 checked assumes 保留运行时检查。尚未做证明驱动的检查消除。

原生 `main` 是便于验收的开发适配器：调用指定零实参入口、打印其结果、成功返回 0。它不代表最终语言入口 ABI；printf 是适配器的 C ABI 接线，不是 Flurry println 的实现。当前内部函数使用私有 LLVM 调用约定，无 FFI/export 支持。

未验证的数据指针或 Symbol enum 存储、闭包环境、部分临时构造/复杂模式转移/变体部分移动清理、上下文特化析构与代数效应 ABI，以及一般库 intrinsic 接线尚未实现。直接局部对象、产品字段、enum/error 活动载荷、显式 discard、赋值替换及显式谓词与函数契约失败的清理已接通，具体边界见后续各节。未支持的节点/表示明确拒绝，不能据此声称一般程序的原生资源安全已完成。这里只支持本机目标，不支持 cross target；目标缓存尚未实现；独立对象文件交付现支持 --emit-object，见末节。

构建先在独占临时目录生成完整二进制，成功后才原子替换输出并设置可执行权限；前端/降码/工具/链接失败保留旧产物。输入文件及其别名受现有输出保护。工具子进程关闭 stdin；无人值守使用上面的超时包装器。

## 验证

工作目录 `/home/fin/luna`：

```sh
python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --lib --bin luna
```

本轮退出 0，报告 `/tmp/luna-tools-d9gwr5jk/result.json`。其中 host 20 项实际调用 LLVM/链接器并执行本机程序，另含诊断、工具失败、源文件保护、旧产物保留和确定性文本检查；frontend_cli 173 项通过。未执行整个 workspace 测试。语义前端续接见 `frontend-checkpoint.md`。

仓库示例亦单独构建并执行通过：`host-arithmetic.fl` 的原生 stdout 为 `(unsigned-integer 42)`（`/tmp/luna-tools-xdit4vr1/result.json`），`host-template.fl` 为 `(bool true)`（`/tmp/luna-tools-_bwvk9cv/result.json`）。测试后的缓存维护移除了开发 CLI，按上述 build 命令重建成功（`/tmp/luna-tools-xugzl8w3/result.json`）。本轮合计 210 项相关测试通过，工作区改动未提交。


## IEEE 浮点原生降码

f32/f64 字面量以整数到浮点的常量 bitcast 表示，避免十进制往返丢失位模式。加减乘除使用对应 LLVM 浮点指令，不启用 fast-math，也不授权表达式重结合；算术产生的 NaN 与当前参考执行器一致规范化为 quiet NaN。取负使用 fneg，保留负零和载荷位的符号翻转。比较使用有序 fcmp；不等比较沿用 NIR 的相等取反，因此 NaN 不等于自身。浮点除零产生 IEEE 无穷/NaN，不走整数除零 trap。

新增 `development/examples/host-floating.fl`，同样使用 `--build run --output <path>`，输出 `(bool true)`。浮点入口的开发输出为 `(float32 (bits 42280000))` 或 `(float64 (bits 4045000000000000))` 一类精确位模式，不通过 printf 的浮点十进制转换掩盖负零或舍入。

本轮在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`：27 项通过（`/tmp/luna-tools-mw_duyhn/result.json`）。覆盖两种宽度的四则运算、NaN/无穷、有序比较、正负零及倒数、逐操作舍入、最小非正规数。随后加入仓库浮点示例验收并定向执行 `--test host shipped_floating_example_executes_natively`：1 项通过（`/tmp/luna-tools-uw_c07ci/result.json`）。这些检查实际生成并执行 LLVM 本机产物。未重复全 workspace 测试。


## 128 位整数交付

原生入口现可输出完整 i128/u128 十进制值。开发适配器使用调用者栈上的 41 字节缓冲区，按无符号 magnitude 提取最多 39 位数字，另留符号和 NUL；i128 最小值不经有符号取绝对值，因此不会溢出。它只负责结果展示，不改变 Flurry 算术规则。LLVM 对宽整数运算所需的运行库辅助函数由现有 C 链接器接线。

`development/examples/host-wide-integer.fl` 用 `--build run` 构建，原生结果是 `(unsigned-integer 36893488147419103232)`。已验证零、负一、i128 最小/最大值、u128 最大值经过普通调用后的完整输出，跨机器字乘法及无符号除法/余数。有符号源码 `/` 当前仍由前端以舍入协议未实现拒绝；这是前端续接项，不改成隐式截断语义。

本轮工作目录 `/home/fin/luna`。host 组首次执行 30 项通过、1 项因上述有符号源码除法限制失败（`/tmp/luna-tools-rcztevuz/result.json`）；将该验收改为当前语言前端已支持的无符号除法后，定向运行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host native_u128_division_and_remainder` 通过（`/tmp/luna-tools-kbf6j2ap/result.json`）。新增仓库宽整数示例另行定向运行 `shipped_wide_integer_example_executes_natively` 通过（`/tmp/luna-tools-j8sqecz2/result.json`）。所有正例实际经过 LLVM 验证、目标文件生成、链接及执行。未重复 workspace 全量检查；改动未提交。


## 普通函数值与间接调用

普通运行时 `fn` 值现在降为 LLVM 函数指针。函数声明/实例地址从已验证 IIR 依赖图取得，可作为实参、返回值、默认实参以及局部变量；条件选择和赋值保存真实的 runtime 目标。间接调用从 callee 的完整 IIR 签名取得参数/结果类型，先求值 callee 一次，再依源码顺序处理实参。LLVM 的 `ptr` 只是目标表示，不能取代完整函数类型相容性检查；现有 IIR 验证仍先于降码。

`development/examples/host-callback.fl` 展示工厂返回函数值、传参调用及重新赋值，使用 `--build run` 得到 `(bool true)`。此表示仅用于当前支持的普通函数，尚不支持捕获环境或外部 ABI；无捕获 lambda 的环境消除见下节。

本轮发现并保留前端的两个边界：将带前置谓词的函数传给无契约函数类型，仍以“function value would erase program preconditions”拒绝；含提前 return 的 `do` callee 表达式在当前测试中被判定为非函数。后者留待前端恢复开发时处理。函数体内 checked assumes 不会因间接调用而丢失，原生负例实测会触发 trap。

验证工作目录 `/home/fin/luna`，命令 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，最终退出 0，41 项通过，报告 `/tmp/luna-tools-euz0sf9t/result.json`。新增覆盖函数参数/返回、runtime 选择与重赋值、callee 恰好求值一次且早于实参、默认回调、void 调用、u128/f64 间接调用、checked assumes 和契约擦除拒绝，以及仓库回调示例实际编译运行。未重复全 workspace 测试；工作区未提交。


## 无捕获 lambda

后端现在按稳定遍历顺序收集 IIR 的空捕获 closure，将正文提升为内部 LLVM 函数，用已有普通函数指针传参和返回。不引入堆分配。lambda 内的 return 只返回该 lambda；提升时不复制所属函数的 pre/postcondition 或结果绑定。函数本身的契约仍按原路径执行。正文可调用图内普通函数，也可构造和返回另一个空环境 lambda。

注册表使用本次降码期间的节点地址查找已借用正文；地址不进入输出，也不用于排列符号。符号由遍历顺序编号，独立编译进程的 LLVM 文本一致性已验证。该阶段仍拒绝非空捕获。后续数值环境支持见末节：带环境值始终使用独立聚合表示，不能当作裸函数指针；资源清理 ABI 仍未接通。

新增 `development/examples/host-lambda.fl`，构建入口 `run` 的原生输出为 `(bool true)`。在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，44 项通过（`/tmp/luna-tools-sgezpl2q/result.json`）；随后新增边界验收，定向运行 `--test host lambda`，8 项通过（`/tmp/luna-tools-qfqgwk3v/result.json`）。覆盖空环境参数/返回、嵌套 lambda、独立 return、所属函数后置条件隔离、调用普通函数、符号稳定、捕获拒绝与仓库示例。后置条件隔离的初稿未能被前端证明，已改为 lambda 调用后返回显式 true，使前端先证明父函数契约，再检验后端不会把该契约施加到 lambda；没有放松前端证明规则。改动未提交。


## 标量模式匹配

当前 host 后端接入 IIR Match/LetPattern：标量字面量、wildcard、绑定、观察别名、guard、计算型 `and … is`、绑定 remap 与 `or`。源码中的整数/浮点范围及取反已经由前端表示为这些节点，后端沿用同一语义，不重新解释源码 pattern。匿名记录解构现已接入；struct 解构现已在资源验证路径接入；资源视图、enum/error pattern 尚待接入。

主语只执行一次。各候选按源序尝试；失败转到下一候选，候选绑定不可见于后续候选，已发生的外部局部变量赋值继续保留。or 的成功分支将统一绑定写入合并位置；remap 同时取得源绑定再更名。守卫和计算型 pattern 的 return/break/continue 沿现有控制流降码。let-else 成功导出绑定，失败块必须发散。匹配最后保留 trap 防止任何不合法落空读取未初始化结果，不用它取代前端穷尽性验证。

新增 `development/examples/host-pattern.fl`，入口 `run` 原生输出 `(bool true)`。工作目录 `/home/fin/luna`，执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，63 项通过，报告 `/tmp/luna-tools-f3uhj96s/result.json`。新增验收包括范围端点、浮点 NaN 与取反、候选优先级、主语次数、成功后跳过守卫、失败守卫保留外部赋值、两条 or 分支绑定、计算型 pattern、let-else、if/while 模式、guard return 和 let-else continue。均通过真实 LLVM 编译与本机执行；不含全 workspace 回归。改动未提交。


## 匿名记录布局与按值传递

IIR 的匿名记录取得独立递归 LLVM struct 布局，支持构造、嵌套字段读取、根及字段赋值、普通/泛型实例函数传参与返回、条件分支合并和匿名记录 pattern/let 解构。构造字段按源码顺序求值；字段投影使用 extractvalue，字段赋值定位本地存储的 GEP。零大小 void 字段使用空 struct 存储，不丢弃其初始化表达式的求值；普通函数值可存入字段并间接调用。

布局仅含已支持的标量、函数值和匿名记录；嵌套数据指针仅在下述显式资源验证入口接纳。按值复制产生独立记录值，不让对副本字段的赋值修改原记录。LLVM 布局不是类型身份，IIR 的字段标签与完整签名检查仍保留；没有把 Tuple normal-form 头展开成结构元组，也没有接通 nominal 析构或资源清理；Tuple 的可信布局接线见下节。当前开发入口打印器仍只打印标量结果，记录可在程序内部传递并用于计算标量结果。

新增 `development/examples/host-record.fl`，将嵌套记录、字段更新、解构和 lambda 回调组合起来，入口 `run` 输出 `(bool true)`。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host` 66 项通过（`/tmp/luna-tools-8l0zwme5/result.json`）；新增边界测试后定向 `--test host native_record` 5 项通过（`/tmp/luna-tools-q2nbxjro/result.json`）；仓库示例 `--test host shipped_record_example_executes_natively` 1 项通过（`/tmp/luna-tools-35514f7n/result.json`）。包含真实 LLVM 验证、链接及原生执行；指针字段拒绝例验证原产物保留。未重复 workspace 全量测试，改动未提交。


## Tuple 的可信布局

Host 的表示查询现在接收已验证 IIR 图。遇到 normal-form Type 时，只向图内的 head 元数据查询 `tuple_elements`；只有被标记为 Tuple 的 head、Type sort 和单个 `TypeListValue` 输入满足该表示契约时，才获得字段布局。其他 opaque head 仍拒绝，绝不执行 Type 函数体来推导相等性。迁移期内部结构视图可取得相同存储，但不成为源码类型定义。

Tuple 构造和 pattern 解构复用聚合值降码，支持函数参数/结果、嵌套字段、空 tuple、单元素 tuple、or 候选绑定和普通函数值字段。空 tuple 用零大小调用表示，嵌入聚合字段时保留空 struct 位置。变参收集、runtime tuple spread 和扩展调用中 tuple 与匿名记录组合已实际生成原生代码。原入口打印器仍不打印整个聚合结果，示例通过解构计算 bool 返回。

新增 `development/examples/host-tuple.fl`，构建时添加 `--meta-list-type input.TupleElements --tuple-type input.Tuple --build run --output <path>`，输出 `(bool true)`。示例 Type 函数的正文不是布局依据。

本轮 `/home/fin/luna` 下 `python3 -B development/check.py --timeout 300 -- test -p luna --test host` 76 项通过（`/tmp/luna-tools-bizf5fyl/result.json`）。新增测试后 `--test host tuple` 中 7 项通过，扩展调用初稿因对无结构约束 T 投影字段被前端拒绝；改用现有 schema 定义的明确 Options 类型后，定向测试通过（`/tmp/luna-tools-kcfhjq4w/result.json`）。仓库示例单独通过（`/tmp/luna-tools-7qr7iiyg/result.json`）。IIR dump 检查确认在 codegen 前仍有 normal-form head 和 TypeList 输入；原生正例验证求值次数、返回值和分支绑定。没有修改前端语义或运行全 workspace 测试，改动未提交。


## LLVM 优化级别

`--build run --opt-level 0|1|2|3` 选择 LLVM IR 标准 `default<O0/O1/O2/O3>` pipeline，并将同级别传给 llc。构建默认级别为 0，替代之前 llc 隐式默认级别。优化通过同一个 llvm-config 目录内的 opt 执行，开启 `-verify-each`；未启用 fast-math 或放松浮点规则，也不以优化代替前端类型、范围或契约验证。

`--emit-llvm run` 继续导出原始降码文本；显式加 `--opt-level 2` 则调用 opt 导出优化后的文本。源文件名固定为 `flurry-host`，去掉 opt 写入的临时输入路径 ModuleID 注释，确保跨进程 dump 稳定；不删除任何 IR 指令或元数据。优化器失败时不发布新文本或二进制。

例：

```sh
python3 -B development/command.py --timeout 30 -- /tmp/luna-dev-002/target/debug/luna development/examples/host-record.fl --build run --opt-level 2 --output /tmp/flurry-record
python3 -B development/command.py --timeout 30 -- /tmp/luna-dev-002/target/debug/luna development/examples/host-record.fl --emit-llvm run --opt-level 2 --output /tmp/flurry-record.ll
```

本轮 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --lib --bin luna`：16+1+173+85 项通过，共 275 项，报告 `/tmp/luna-tools-hq3rwebn/result.json`。随后补充浮点位模式及失败保留测试，定向执行 `--test host optimi`：6 项通过（`/tmp/luna-tools-4ehj5_dn/result.json`）。优化验收覆盖四个级别的 6 个仓库示例、checked assumes trap、负零、最小非正规数、逐操作舍入、canonical NaN，另有优化 IR 消除 alloca 的实际证据、输出稳定性及非法选项拒绝。没有全 workspace 测试，工作区未提交。


## 显式资源协议验证入口

Host 的 `--build` 和 `--emit-llvm` 现在接受配对的 `--copy-protocol` / `--drop-protocol`。前端先把名称解析成协议声明身份，再复用与资源检查执行器相同的 world、语言项绑定和 `prepare_checked_source`，取得可达上下文的完整 ownership/cleanup 证据。没有另写一套简化资源规则。

当前原生可接纳的是 release-only 清理计划：转移和失效已通过验证，支持布局的值可留在本地栈槽直到函数返回，无需额外运行时释放动作。所有 schedule 的局部/临时/丢弃结果清理都会被扫描；Select、Switch、IfConstructed 内的责任也递归检查。任何 `Invoke` 都明确报 `host destructor invocation is not implemented ... cleanup cannot be discarded`，即使其源码析构体为空，也不擅自删除责任。扫描先检查所有析构调用，再验证 release-only 存储布局，使未实现的存储不会掩盖析构调用缺口。

这只是资源验证链与 LLVM 的接线，不代表 native Drop、捕获环境或一般 nominal 值已经实现；数据指针的受限接线见下节。下一步需要给清理 schedule 的 boundary/target/context 接入真实存储和析构 ABI。不带协议选项的旧开发入口保持原有受限行为。

新增 `development/examples/host-resources.fl`，使用 `--build run --copy-protocol Duplicate --drop-protocol Retire --output <path>`，输出 `(bool true)`。本轮在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --lib --bin luna`，16+1+173+93 = 283 项通过，报告 `/tmp/luna-tools-b8sywpol/result.json`。覆盖标量、循环、默认值、协议配对/解析失败、非 Copy nominal 重复消费（明确在资源准备层拒绝）、不可遗漏的析构责任与旧产物保留。新增仓库示例另以 `--test host shipped_resource_checked_example_executes_natively` 通过（`/tmp/luna-tools-egy_q_8q/result.json`）。未执行 workspace 全量测试，工作区未提交。


## 经过资源验证的数据指针

显式 Copy/Drop 验证成功后，host 可降码 AddressOf、ReadPointer、WritePointer 和 PointerAnonymousField：本地/匿名记录字段取址、二级指针、读取和写回、函数借用传参与返回。数据指针保留独立的 pointee 布局，不能混同普通函数指针。LLVM 没有附加未经证明的 noalias 等承诺。

这条路径先执行完整资源准备，再验证清理计划只有 release-only 动作；只在该证明路径启用 pointer 表示。未选择配对协议时，即使类型和表达式可进入 IIR，原生降码也会明确要求 Copy/Drop 验证。递归 pointee 仍必须有已支持的表示，不会因此开放不支持的 nominal 指针、堆分配或析构调用。

所有本地槽包括零大小槽现在都有可取址的 LLVM alloca；零大小读取不生成 void load。原生测试覆盖零大小值取址、指针保存及二级指针读取。尝试写回 void 的测试仍被前端以 non-Copy pointer writes require memory transfer contracts 拒绝；这是前端现状，未放宽规则或宣称该源码路径已支持。

新增 `development/examples/host-pointer.fl`，以 `--build run --copy-protocol Duplicate --drop-protocol Retire --output <path>` 构建，结果 `(bool true)`，四个优化级别均已验证。`/home/fin/luna` 下执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，102 项通过，报告 `/tmp/luna-tools-m3y65781/result.json`。局部地址逃逸测试另强化为必须在 resource-checked native preparation 阶段拒绝，定向执行通过（`/tmp/luna-tools-o7ul0978/result.json`），避免把一般后端未实现报错误当作内存安全证明。其余覆盖嵌套字段取址、通过指针投影匿名字段、别名写回、借用返回和二级指针。未跑 workspace 全量测试，改动未提交。


## 资源验证后的 struct 布局

资源检查成功且 schedule 无析构调用时，后端现在从 IIR `nominals` 中按完整 Type 查找已实例化 Struct 定义，递归取得字段表示。支持构造、泛型参数实例、按值传参/返回、嵌套字段赋值与解构、取址、通过 struct 指针投影字段，以及接收 `*self` 的普通方法。空 struct 保留零大小 LLVM struct 存储；与 void、空 tuple 共享零大小性质不等于共享语言类型身份。

不带配对资源协议的入口继续拒绝 nominal 存储。完整 IIR 验证在布局之前，同字段布局的两个 struct 不能互传；已增加源程序拒绝测试。包含析构 Invoke 的 schedule 仍优先报清理调用未实现，不会因为现在可以布局 struct 就放过责任。Enum、递归自引用布局、捕获环境和真正的析构 ABI 未在本轮实现；布局递归仍有 128 层上限。

新增 `development/examples/host-struct.fl`，通过泛型 Cell、实例方法、指针写回和解构返回 bool。以 `--build run --copy-protocol Duplicate --drop-protocol Retire --output <path>` 构建，四个优化级别结果均为 `(bool true)`。

`/home/fin/luna` 下执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，110 项通过，报告 `/tmp/luna-tools-o0k4lhqz/result.json`。覆盖上述原生行为、嵌套/空 struct、同布局类型身份拒绝、未验证路径拒绝及已有析构责任拒绝回归。未执行 workspace 全量测试，改动未提交。


## newtype 的显式表示边界

资源验证后的 newtype 从精确实例的 representation 递归取得 LLVM 布局，不额外分配包装字段。`NewtypeConvert` 消费一次输入并要求输入/结果布局一致；在此之前 IIR 已验证转换跨越恰好一个已声明的表示边。布局相同不能授权别的转换，不展开 normal-form 头，也不继承底层类型的源码接口。

本轮支持标量、实例化的泛型和嵌套 newtype、struct 底层值，以及数据指针底层值的显式转换。指针包装仍须完整资源验证，不能用 newtype 绕开借用/逃逸规则。newtype 的独立 Drop 责任仍会阻止原生编译，底层 bool 的简单表示不授权删除外层析构。入口开发打印器按底层布局输出标量，仅用于验收，不是语言层面的 newtype 显示协议。

新增 `development/examples/host-newtype.fl`，以 `--build run --copy-protocol Duplicate --drop-protocol Retire --output <path>` 构建，四个优化级别均输出 `(bool true)`。`/home/fin/luna` 下运行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，118 项通过（`/tmp/luna-tools-v1btbifo/result.json`）。覆盖表示往返、泛型嵌套、struct/指针载荷、输入副作用恰好一次、同布局不同身份拒绝、跨层转换拒绝和析构责任保留。未执行 workspace 全量测试，工作区未提交。


## 无载荷 enum

资源验证后的无载荷 enum 现可构造、按值传递/返回、保存进 struct、重新赋值和通过指针写回，并用于 variant/or/guard 匹配。后端以完整 nominal 实例取得描述；source variant index 只用于查找编码，不直接充当已配置枚举的实际位值。

固定宽度枚举调用现有 encoding plan 构造零载荷位模式，当前接纳 1–128 位；直接使用 `iN` 存储，既不追加 tag，也不按声明顺序重新编号。未配置位编码的普通无载荷枚举暂用私有 u32 判别码。Symbol 编码、载荷变体、空值域构造、超范围索引均不会落到某个猜测表示。开发入口仍不直接打印 enum 结果，示例通过 match 返回 bool。

新增 `development/examples/host-enum.fl`，需要 `--copy-protocol Duplicate --drop-protocol Retire`；四个优化级别的 `--build run` 输出均为 `(bool true)`。现有前端的模式编码还要求 `.discrete_tag true`，示例明确写出，未修改该前端限制。

`/home/fin/luna` 下执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，126 项通过，报告 `/tmp/luna-tools-kvh44d6x/result.json`。检查普通 variant 构造/调用、指针写回、struct 内嵌、or 与 guard、8/128 位实际编码；IR 断言验证 `ret i8 165` 和完整 u128 最大位模式，避免仅用 encode/decode 同错的往返测试。载荷 enum 拒绝时保留已有产物。未执行 workspace 全量测试，工作区未提交。


## 固定编码 enum 的载荷

资源验证后的固定编码枚举可直接使用 1–128 位的 LLVM 整数存储。构造从 encoding plan 的固定值开始，按字段位置嵌入载荷；匹配先检查固定掩码，再提取命中变体的字段。不额外追加 tag，不把源码变体索引当作实际编码。

当前载荷支持固定宽度整数、BitVec 和浮点位表示；带符号整数保留补码，浮点使用 bitcast 保留负零等原始位模式。配套接通这些表示的位视图、等宽位转换及 BitVec 到更宽无符号整数的零扩展；这不代表已实现全部 BitVec 运算。超过 128 位、零宽载荷、Symbol 编码、没有固定编码的普通载荷 enum 仍明确拒绝。原生析构 Invoke 的限制不变。

命名载荷按字段声明顺序初始化，书写顺序只决定名字对应，不改变初始化顺序。新增原生测试检查相反书写顺序下的副作用，以及第一个声明字段提前 return 时跳过剩余字段。前端的既有规则保持不变；不要将普通调用实参的源码求值顺序移用于字段初始化。

新增 `development/examples/host-encoded-payload.fl`：构造 RISC-V SUB，检查完整编码 `0x403100b3`，再通过 pattern 提取目的寄存器。构建需要 `--copy-protocol Duplicate --drop-protocol Retire`，入口 `run` 返回 `(bool true)`。测试分别使用 O0、O1、O2、O3 编译并执行；另覆盖有符号载荷 -128 和浮点负零的位保留。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test encoded_enums`，退出 0：host 132 项、encoded_enums 14 项通过，报告 `/tmp/luna-tools-sutzytpm/result.json`。载荷提前退出的 Never 构造现在直接执行到退出点，不请求 Never 的存储布局。已执行 workspace 格式化；未运行 workspace 全量测试，改动留在 dev 工作区，未提交。

## BitVec 值运算与动态读取

非空的 1–128 位 BitVec 已接通 `and`、`or`、`not`、相等比较、`++` 拼接、静态切片、单比特索引及固定宽度动态窗口。`and/or` 按左到右执行两侧各一次，不采用 bool 短路规则；提前退出仍阻止后续操作数。拼接的左输入占高位；静态切片从低位索引，不依赖主机字节序。LLVM 的 iN 限定位宽，使取反只翻转有效位。

动态位置支持后端已有的固定宽度机器整数。源级仍须通过程序验证的边界契约；降码另外检查起点范围和窗口终点，再缩窄移位量，避免负值、大索引截断及 LLVM 超宽移位 poison。端点表达式先按源码顺序执行，已有机器算术溢出检查保留。更宽表示的 host 支持与资源预算见下节。

新增 `development/examples/host-bitvec.fl`，包含切片拼接旋转、动态半字节提取与掩码组合。使用 `--build run --output <path>`，无需额外资源协议；输出 `(bool true)`。测试在 O0–O3 分别编译并执行，另验证双侧副作用、128 位全宽拼接和提前退出。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test pure_bitvec`，退出 0：host 138 项、pure_bitvec 18 项通过，报告 `/tmp/luna-tools-6vkk9eyo/result.json`。格式检查 `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check` 退出 0（`/tmp/luna-tools-s6x97tz4/result.json`）。未运行 workspace 全量测试，改动未提交。Never 左操作数的前端分派问题记录在 frontend-checkpoint.md，未以修改语言规则绕过。


## BitVec 位置写回

`BitAssign`、`BitUpdate` 和资源验证后的 `BitWritePointer` 已接通 host 降码。静态区间、动态索引、固定宽度动态窗口及相对嵌套选择先按源码顺序求值并检查，再冻结目标地址与绝对起点。随后执行 RHS；只有 RHS 正常完成后才加载目标当前位模式，合并选中位并写回。因此 RHS 修改索引变量不会改变已选位置，RHS 修改同一值或整个父结构时，其未被选中的位与兄弟字段仍保留。

写回支持后端已接通的整数和浮点位表示，补码和浮点原始位模式保持不变；数据指针路径仍要求配对 Copy/Drop 资源验证。动态每层选择相对于上层窗口检查边界，缩窄移位量之前检查，避免移位 poison。零宽选择的支持见下节；空操作也不能删除 RHS。

`host-bitvec.fl` 增加动态局部写回示例：RHS 先将原值更新为 `0xABCD`，再把已选中的四位改为 `0xA`，结果 `0xABAD`。原生回归另检查重叠源/目标、嵌套窗口、父结构替换、指针 RHS 改写、符号位、浮点负零、128 位整段及提前 return。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test pure_bitvec`，退出 0：host 145 项、pure_bitvec 18 项通过，报告 `/tmp/luna-tools-pt8a2_3h/result.json`。更新后的示例在 O0–O3 均实际编译并执行。已格式化，未跑 workspace 全量测试，改动保留在 dev，未提交。


## 零宽 BitVec

`BitVec<0>` 在 host 私有 ABI 中用 LLVM 空结构 `{}` 表示，独立于 void，不生成不存在的 `i0`。零宽常量、参数、返回、本地存储、逻辑运算、拼接单位元、相等比较、静态及动态空切片和向无符号整数扩宽均已接通。开发入口打印其唯一值为 `(unsigned-integer 0)`，不是最终语言显示协议。

空结果不删除输入行为。逻辑运算和拼接仍按顺序求值两个操作数；动态空窗口检查 `0 <= start == end <= width`，合法终点可等于输入宽度，检查完成后不生成移位。空写回仍完成所有位置表达式、边界检查和 RHS；RHS 正常完成后不加载或写回目标，RHS 对目标的新值保留。零宽值不能被用作合法单比特索引的输入。

`host-bitvec.fl` 已加入空窗口写回副作用示例。固定编码 enum 的零宽载荷仍暂拒绝；更宽普通 BitVec 见下节。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test pure_bitvec`，退出 0：host 149 项、pure_bitvec 18 项通过，报告 `/tmp/luna-tools-lp3mmj9r/result.json`。更新后的示例 O0–O3 通过，越界空窗口在 O0/O3 均触发检查失败。已执行格式化，未运行 workspace 全量测试，dev 工作区改动未提交。


## 宽 BitVec 的本机降码

普通 BitVec 的私有 host 表示已扩展到 LLVM iN，当前编译资源预算为 65,536 位；这是后端实现预算，不是语言宽度上限。常量按完整 little-endian u32 字数组转换为十进制 LLVM 常量，不经 u128。逻辑、比较、拼接、切片、动态读取和写回均复用该表示；超过 128 位时，已检查的动态位置向操作数位宽零扩展，而不是错误截断。写回掩码通过目标宽度的全一值和逻辑移位生成，支持宽于 u128 的选择及整段写回。

新增 `development/examples/host-wide-bitvec.fl`，包含 1024 位向量上的动态 256 位写回、切片拼接旋转和比较；入口返回 `(bool true)`，无需资源协议参数。另测试 256 位拼接/写回、257 位常量高位和全一位模式。入口开发打印器仍仅直接显示最多 128 位数值；宽向量示例通过比较返回 bool。固定编码 enum 的宽布局迁移见下节；零宽 enum 载荷的前端阻塞已记入 frontend-checkpoint.md。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test pure_bitvec --test encoded_enums`，退出 0：host 152 项、pure_bitvec 18 项、encoded_enums 14 项通过，报告 `/tmp/luna-tools-t1z464hq/result.json`。已执行 workspace 格式化；未运行 workspace 全量测试。所有改动保留在 dev，未提交。


## 宽固定编码枚举

固定编码 enum 的 host 布局描述已将固定值和判别掩码从 u128 改为由完整位字数组产生的规范十进制常量。整体宽度使用同普通 BitVec 相同的 65,536 位编译预算；构造、按值参数与返回、判别、载荷提取及位视图不再受 128 位限制。位宽和字段位置仍来自已验证的 encoding plan，名义身份不变，不额外追加 tag。

新增 `development/examples/host-wide-enum.fl`：257 位 Message 的最高位区分 low/high，余下 256 位为 payload。示例独立检查最高位、完整载荷位图和两种 variant 分支，需要 `--copy-protocol Duplicate --drop-protocol Retire`。另有回归使用 comptime 函数计算 257 位全一表达式编码，检查最高位与低 256 位，避免只依靠往返一致性掩盖高位丢失。

Symbol 编码、固定编码枚举的零宽载荷和真实析构调用仍未接通；开发入口仍不直接打印枚举值。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test encoded_enums`，退出 0：host 154 项、encoded_enums 14 项通过，报告 `/tmp/luna-tools-8rii84gs/result.json`。新示例 O0–O3 均实际编译并执行。已格式化，未运行 workspace 全量测试，改动保留在 dev，未提交。


## 普通载荷枚举

没有固定编码的普通载荷 enum 已支持资源验证后的 host 降码。当前私有 ABI 使用一个 i32 判别码及各变体分别拥有的载荷结构区；这是尚未压缩的表示，空间开销可大于 tag+union，不承诺为最终语言或外部 ABI。构造只执行所选变体的输入，未选中区的零初始化只是存储表示，不创建语言值或析构责任。模式先检查 tag，再提取相应载荷，不能读取未选中的变体值。

支持位置/命名载荷、泛型实例、嵌套 enum、已支持的标量/零宽/结构表示、函数间传参与返回，以及局部和指针重赋值。命名字段仍按声明顺序初始化。普通枚举的聚合布局不冒充固定编码位视图；bit_width 不为它提供虚构位宽。IIR 名义类型和资源验证仍先于布局，含析构 Invoke 的清理计划继续拒绝。

新增 `development/examples/host-payload-enum.fl`，通过泛型 Result 的 missing/value/pair 分支及重赋值返回 bool，需配对 Duplicate/Retire 协议。O0–O3 实际编译并执行均为 `(bool true)`。独立回归覆盖嵌套异构载荷、普通枚举中的零宽 BitVec、指针写回、声明顺序和析构责任拒绝时保留已有输出。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test encoded_enums`，退出 0：host 159 项、encoded_enums 14 项通过，报告 `/tmp/luna-tools-pdqka337/result.json`。已格式化，未运行 workspace 全量测试，dev 改动未提交。

## 位向量模式的原生验收

位模式已由前端展开为 Observe/Then/Guard/Remap 等现有 NIR/IIR 节点，原生后端复用观察、切片、位转换和绑定降码。本轮没有增加新的 IR 或改动后端逻辑，补齐这一组合的执行证据，避免把单独支持各节点当作完整模式行为已验证。

新增 `development/examples/host-bit-pattern.fl`，直接从 32 位输入匹配 RISC-V ADD/SUB 的固定段，并通过 OR 模式导出目的寄存器；未知编码走 fallback。O0–O3 都编译执行为 `(bool true)`。定向测试覆盖替代分支绑定重映射、否定模式、guard 副作用及失败后继续匹配、257 位 subject 的 256 位字段、补码有符号提取、浮点负零位保留、subject 只求值一次，以及模式总宽度错误时保留已有输出。

在 `/home/fin/luna` 先运行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，163 项通过（`/tmp/luna-tools-r2fh8xhh/result.json`）；追加类型字段、单次求值和拒绝回归后，运行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host bit_pattern`，7 项全部通过（`/tmp/luna-tools-6mwxxgal/result.json`）。均退出 0。已格式化，未跑 workspace 全量测试，dev 工作区未提交。


## 错误结果的本机返回表示

闭合错误行已接通独立的数据返回 ABI，不使用代数效应 continuation。私有 LLVM 布局由 i32 tag、成功载荷槽和各错误类型载荷槽组成；tag 0 表示成功，错误 tag 按该已实例化类型的行顺序分配。字段布局仍是未压缩形式，不承诺为外部 ABI。未选中槽只是存储表示，不创建额外语言值。

支持 Success/Failure 构造、ErrorResult 模式匹配和 Embed 转换。行扩展与重排按完整 Type 身份查找目的 tag，不能沿用源 tag 数字；成功载荷类型为 Never 时不生成不可达成功值的转换。错误传播与 handler 沿用前端展开的匹配、分支及 return；输入只求值一次。嵌套 tuple/record 的 Product 转换见下节；析构 Invoke 仍需后续实现，入口打印器不直接打印错误结果。

新增 `development/examples/host-errors.fl`，包括带载荷的 ReadError、失败返回、跨函数传播及调用方恢复为数值，需要 `--copy-protocol Duplicate --drop-protocol Retire`。O0–O3 实际编译执行都输出 `(bool true)`。测试还验证同样含 bool 载荷的两个不同错误类型在行重排后保持标签身份，以及扩大错误行时同时保留成功和失败分支。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test error_surface`，退出 0：host 171 项、error_surface 117 项通过（`/tmp/luna-tools-frg7lskw/result.json`）。追加错误载荷析构责任测试后，定向执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host native_error_payload_destructor`，1 项通过（`/tmp/luna-tools-8nt8b60m/result.json`）：明确拒绝尚未实现的析构调用，并保留旧产物。已格式化，未运行 workspace 全量测试，dev 工作区未提交。


## 聚合内的错误行转换

ErrorConvert::Product 已接通 host 降码。后端调用现有 `transport_plan` 验证闭合源/目标结构，使用精确类型身份和已绑定的 Tuple role；不根据类型函数体猜测 Tuple，也不展开 normal-form 头。计划中的 Identity 字段原样保留，Error 字段按目标行标签重映射，Tuple/Record 字段按计划顺序递归重建。源表达式只求值一次，后续提取与重建是内部表示转换，不重新执行 producer，也不作为源码解构改变资源责任。

转换递归消耗 host fuel 并有深度上限。目标表示仍须完全可降码，含析构 Invoke 的责任继续拒绝。当前聚合 ABI 尚未压缩；此处支持验证计划允许的 tuple/匿名 record，不能推定为对任意 nominal struct 的字段重排授权。

新增 `development/examples/host-error-product.fl`，使不同错误行顺序的 record 进入泛型参数，通过一次指针写入确认 producer 执行次数，并分别检查成功与失败路径。需配对 Duplicate/Retire 协议，O0–O3 均输出 `(bool true)`。独立测试覆盖嵌套 Tuple、可信 Tuple 头与 record 字段保留。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test error_surface`，退出 0：host 175 项、error_surface 117 项通过，报告 `/tmp/luna-tools-81qwlts5/result.json`。已格式化，未运行 workspace 全量测试，dev 改动未提交。

## 资源清理降码前的稳定交接

`host::prepare_with_protocols` 现在返回不可变的 `host::Prepared`：它保留同一次完整资源准备产生的 IIR graph 与 cleanup schedule，只有只读 `graph()` / `schedule()`，构造器与底层字段私有。`Display` 复用 Executable 的 S-expression，包含入口、上下文清理计划和 IIR。该阶段不执行 runtime 用户代码、不调用 LLVM，也不因为后端尚不支持 Invoke 就丢弃准备产物。

`host::llvm_ir_prepared` 接收这一交接物，检查后端清理能力并降码。现有 `llvm_ir_with_protocols` 已改为顺序调用这两个阶段，CLI 行为不变。准备与降码各有工作预算；降码失败不修改已准备状态，不能通过公开接口替换为空 schedule 来规避责任。

本轮没有实现析构调用。后续必须结合 FunctionSchedule 的 context/aliases、sites、transfers/copies、pattern transfers 和临时构造状态，在 ScopeEnd/TemporaryEnd/Return/Replacement/PredicateFailure 等正确边界执行计划。仅在函数尾部补 drop 不满足现有契约。需要覆盖 Local/Temporary/Prepared 三种目标及 Switch/Select/IfConstructed，不能把调用上下文特化直接合并到同一个无上下文函数符号中。

在 `/home/fin/luna` 运行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，177 项通过，退出 0（`/tmp/luna-tools-yp98rekp/result.json`）。新测试确认含 Invoke 的程序可发布可检查的准备产物，但 LLVM 阶段仍拒绝；失败重试及重复降码保留相同 dump 和 IR。已格式化，未跑 workspace 全量测试，改动未提交。

## 直接局部对象与参数的析构

本机后端开始执行已验证 schedule 中的直接析构：当前接纳无上下文特化、无变体 guard、以整个 Local 为目标的 ScopeEnd/FunctionEnd 站点。Invoke 必须直接指向该对象自身；可附带仅释放字段存储的 Release。后端在恢复外层局部绑定之前执行 scope-end，在正常函数返回前执行 function-end，按 schedule 的 action/step 顺序把对象真实存储地址传给已实例化 Drop 方法。LLVM 模块现在区分实际入口与附加的析构依赖 roots，不再要求 graph 只有一个 root。

这不是完整清理 ABI：Temporary/Prepared 目标、显式 Return/Break/Continue、替换/谓词失败边界、条件构造、部分移动、字段析构、Switch/Select/IfConstructed 和上下文特化 Invoke 仍在 admission 阶段拒绝，保留原产物。Release 本身不调用用户代码。不能只看到 Drop 方法存在就放宽未接线的责任，也不能用函数尾清理代替这些边界。

新增 `development/examples/host-destructor.fl`，析构通过对象保存的 *bool 修改外部值，作用域外读取结果；O0–O3 都编译执行成功。测试还覆盖多个对象逆序析构、循环中每轮恰好一次、显式 return 仍拒绝及既有临时值/枚举/错误载荷责任拒绝。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，182 项通过，退出 0（`/tmp/luna-tools-iut0c17l/result.json`）。已格式化，未执行 workspace 全量测试，dev 改动未提交。

追加按值参数在 FunctionEnd 析构的原生用例后，定向执行通过（`/tmp/luna-tools-xf147w1n/result.json`）。最终复跑上述 host 命令，183 项全部通过（`/tmp/luna-tools-yxosnuc0/result.json`）；旧的空析构拒绝用例保留为临时值清理拒绝，直接局部析构改由可观察写入测试验收，未将不支持路径静默放行。

## 直接清理的 return / break / continue

已扩展直接 Local 清理的边界接线。显式 Return 先完成返回值与后置检查，再执行该 Return 站点的计划；不再执行正常 FunctionEnd 计划。Break/Continue 在发出对应 LLVM 跳转前执行自身站点的计划。终止路径不执行随后正常 Block 的 ScopeEnd，因此同一个局部对象不会因退出和正常结束叠加析构。

这仍沿用直接、无条件、无上下文特化且整个 Local 为目标的 admission 边界。Return 上的 TemporaryEnd、临时/Prepared 目标、条件构造、部分移动、字段与变体清理、谓词失败等复杂 Invoke 仍拒绝。返回值自身的所有权转移仍来自已验证 schedule，后端不通过“所有局部都 drop”的规则补推。

更新 `development/examples/host-destructor.fl`，串联正常 scope-end、按值参数显式 return 与循环 break，O0–O3 均通过。独立测试验证显式 return 不重复函数结束清理、break/continue 只清理一次、返回值在析构改变外部存储之前求值。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，186 项通过，退出 0（`/tmp/luna-tools-qskjcumb/result.json`）。已格式化，未运行 workspace 全量测试，dev 改动未提交。

## 显式 discard 的直接析构

FunctionSchedule.discards 已接入 host admission 与 `K::Discard` 降码。只有无上下文特化、无变体 guard、直接作用于整个返回值的 Invoke/Release 计划进入当前路径；验证计划的 discard source 与 value_source 必须对应实际 `K::Discard` 节点。输入正常完成后创建临时存储并按计划调用 Drop，清理在下一语句之前发生。输入提前 return 时不产生存储、不执行该值的析构。

局部边界与 discard 共用直接析构调用生成，检查方法为单个 runtime subject pointer 输入、void 返回。Release 不触发用户代码。一般 Temporary/Prepared 站点、部分构造、替换、条件析构及有字段路径的 Invoke 仍拒绝；接通显式 discard 不代表所有临时值生命周期已接通。

`host-destructor.fl` 已增加显式构造值丢弃，O0–O3 均通过。回归另验证函数返回的拥有值立即析构、连续两次 discard 各执行一次，以及 producer 提前 return 时没有虚构 Drop。原空 struct/newtype discard 拒绝用例现改为成功验收，准备产物的未实现责任检查改用仍未接线的 Replacement 边界。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，188 项通过，退出 0（`/tmp/luna-tools-ks7iyod8/result.json`）。已格式化，未运行 workspace 全量测试，dev 改动未提交。

## 赋值替换与聚合字段清理

普通 `K::Assign` 已接通 Replacement 清理站点：先完整求值 RHS，正常完成后按计划清理当前位置的旧值，再重新取得目标位置并存储 RHS。不能在 RHS 前析构旧值，也不能缓存旧的父结构覆盖 RHS 或析构对兄弟字段的修改。RHS 提前返回时不执行 Replacement，沿对应 Return 计划清理已有对象。

直接 Invoke 与 Local target 现在支持 struct/tuple 产品布局中的字段路径。每层按实际已验证表示生成 GEP，检查最终 subject 表示和 Drop 方法签名。父析构、字段析构及字段释放按计划顺序执行，不自行调整顺序。enum/error 的判别路径、条件构造、部分移动和上下文特化仍未因此放行；指针替换的临时目标也不属于普通 Local 赋值路径。

测试覆盖 RHS/旧值析构/新值写入的先后顺序、嵌套结构字段析构、替换字段时兄弟字段保持存活、父析构先于字段析构，以及 RHS 返回时旧值只清理一次。更新的 `host-destructor.fl` 加入赋值替换，O0–O3 均成功执行。未实现计划的准备检查改用仍拒绝的 enum 载荷清理。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，193 项通过，退出 0（`/tmp/luna-tools-dd5lsbyz/result.json`）。已格式化，未运行 workspace 全量测试，dev 工作区未提交。

## 显式谓词失败的直接清理

`CheckPredicate`（包括 checked `assumes`）、`RequirePredicate` 和 `EnsurePredicate` 的失败分支现在先执行对应 PredicateFailure 计划，再执行 `llvm.trap`。站点按谓词操作数的 source 匹配，与参考执行器一致；成功分支不执行失败清理。普通算术、位边界检查的 trap 不会被当成谓词失败站点。

这里仍只接纳无条件、无上下文特化的直接 Local/产品字段计划。函数接口、默认值、前后置契约的其他失败边界，以及条件构造、枚举/错误载荷分支清理、一般 Temporary/Prepared 目标仍未接通，不能由本项推定为支持。

回归检查生成 LLVM 中析构调用在 trap 之前，实际构建并执行 O0/O3 的失败程序，并验证谓词成功时正常退出仅清理一次。失败后的析构副作用没有独立外部观测，本项证据是降码顺序与原生失败行为的组合。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，194 项通过、退出 0（`/tmp/luna-tools-ofy47pqp/result.json`）。`python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check` 通过（`/tmp/luna-tools-6l6l7h_8/result.json`）。未运行 workspace 全量测试，dev 工作区未提交。

## 函数前后置条件的直接清理

函数入口前置条件现在使用对应 PredicateFailure 站点。返回检查使用 `PostconditionFailure(origin)`，其中 origin 区分函数体正常返回与每个显式 Return；先检查后置条件，失败时执行该返回点的清理计划，成功后才执行正常返回清理。admission 同时核对条件来源及返回位置，避免以一个返回点的计划替代所有退出路径。

继续限制为直接 Local/产品字段、无条件且无上下文特化的清理；接口前置条件、默认参数失败、Prepared 返回值清理及条件/变体析构仍未因此接通。新增用例使用已证明的合法函数契约，检查每个前后置条件失败块的析构调用与 trap 顺序，并在 O0/O3 下观察成功路径的按值参数析构恰好一次。覆盖隐式返回、显式返回和多个返回点；没有把静态证明无法成立的函数作为合法输入，也没有宣称这些测试动态触发了后置条件失败。

首次定向用例缺少 `requires flag`，正确被前端以 postcondition is not proven 拒绝（`/tmp/luna-tools-brbatv5w/result.json`）；补全用例契约后通过，未修改前端证明规则。在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，195 项全部通过、退出 0（`/tmp/luna-tools-59_pes4a/result.json`）。全 workspace 格式化完成（`/tmp/luna-tools-ah4m_q0c/result.json`），未运行 workspace 全量测试，dev 改动未提交。

## 枚举与错误结果的分支析构

清理降码已支持 Program 的已知变体 guards 和完整 `Switch`。进入载荷前先检查所属变体；沿路径携带每层 tag，按普通 enum 的独立载荷区或 ErrorResult 的活动槽生成 GEP。嵌套 Switch 只执行实际分支的步骤，步骤顺序保持验证计划原样；无匹配 tag 时 trap。不会把所有载荷区当成有效对象逐个析构。

admission 对每个 Invoke 核对活动分支下的 subject 表示，验证 Switch 分支数与表示一致，并受深度和 fuel 限制。产品字段、枚举载荷和错误结果载荷可以嵌套。固定编码 enum 的位段载荷没有可直接传给 Drop 的地址，本次不支持此类字段 Invoke。Select、IfConstructed、部分构造状态、上下文特化，以及一般 Temporary/Prepared 责任仍未放行。

原有 enum/error 空析构拒绝测试改为成功执行；新增可观察测试在 O0/O3 下验证普通/嵌套 enum、错误结果的空分支不析构、有资源分支恰好一次，并覆盖本地 RHS 构造的替换及返回枚举的显式 discard。准备产物仍可重复查看，降码不修改 schedule。

扩展替换测试发现：RHS 调用有可能失败时，验证计划会要求清理调用者仍存活的旧值；当前 trap ABI 无法传播失败并执行该调用者计划，因此仍明确拒绝。保留专门回归检查此限制及旧输出保护，没有为了接受测试删除责任。无潜在失败的简单调用不一定产生该站点，不能据此一概禁止所有调用。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，最终 198 项通过、退出 0（`/tmp/luna-tools-nj0hqj1g/result.json`）。过程中的旧拒绝预期及未支持调用失败路径见 `/tmp/luna-tools-mito3nzu/result.json`、`/tmp/luna-tools-dc6_4ez1/result.json`。未运行 workspace 全量测试，dev 改动未提交。

## 多候选清理选择 Select

Select 已接入 admission 与 LLVM 降码。每个候选的 guards 从外层 tag 到内层载荷短路检查；候选匹配后记录索引，但不立即执行步骤。所有候选检查结束，且确认恰好一个匹配后，才执行该候选的析构步骤。零匹配或重复匹配先 trap，避免以第一个匹配代替唯一性要求。索引使用内部 i32 槽，候选数、guard 数及递归检查受预算限制。

源级回归确认稀疏枚举集合实际发布 cleanup-select，两个活动分支在 O0/O3 下各只析构对应载荷；嵌套 enum 回归覆盖外层空值及内层两个活动变体。底层合成计划测试实际编译执行零匹配、重复匹配、唯一匹配和先失败后成功的候选列表，O0/O3 均符合唯一性契约。IfConstructed 仍需独立的构造状态跟踪；调用失败传播与上下文特化限制保持不变。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host`，199 项通过（`/tmp/luna-tools-ghde_rug/result.json`）；随后新增的嵌套 Select 用例定向通过（`/tmp/luna-tools-yz90fewi/result.json`）。`python3 -B development/check.py --timeout 300 -- test -p luna --lib host::llvm::cleanup::tests` 的底层测试通过（`/tmp/luna-tools-4t3qnrvc/result.json`）。均退出 0；最终已格式化（`/tmp/luna-tools-fw5yun_s/result.json`）。未跑 workspace 全量测试，dev 改动未提交。

## 整对象条件移动的构造状态

Emitter 已为需要 IfConstructed 的 Local owner 分配独立 bool 存活槽：函数入口初始化为 false，绑定构造完成/整体赋值完成置 true，表达式完成后按已验证 transfers 清除已移出对象的标记。标记写入与实际控制流同行，循环每轮绑定重新置 true；正常清理后也清除 owner 状态。无条件清理对象不额外分配标记。

IfConstructed 在步骤执行前检查该 owner 的标记。当前 admission 要求没有字段级 transfer、没有 pattern transfer、没有外部 variant guards，且目标是完整 Local；在此前提下产品字段与整体共享构造状态，字段释放不需要独立位。部分字段移动、模式转移、部分构造及 Temporary/Prepared 状态仍拒绝，不能把整对象标记推广为通用资源状态树。

新增源级测试确认：条件分支移走并丢弃对象后，scope-end 不重复析构；未移走分支在 scope-end 析构；整体重新赋值重建状态；循环 continue/break 及下一轮绑定正确复位。O0/O3 均观察析构写入次数。独立拒绝例确认条件移动单个字段时不会错误套用整对象标记。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 203 项及 luna 库测试全部通过、退出 0（`/tmp/luna-tools-b_iocg4q/result.json`）。已格式化（`/tmp/luna-tools-uucpzomr/result.json`），未执行 workspace 全量测试，dev 改动未提交。过程中修正测试中的无效 loop 拼写为现有 while true，未添加语言语法。

## 产品字段的部分移动与重建

Local 存活槽现在以 `(binding, field path)` 索引，按产品布局包含每个内部节点和叶子。移动/释放一个位置只清除该位置与后代，不清除父节点自身的构造 token；整体移动会清除整个子树。赋值完成后重建目标子树，并自底向上检查直接子字段是否全部存活，以恢复先前被整体移走的祖先。该规则对应参考执行器的 Responsibility：父 token 存活不等于所有字段仍完整。

IfConstructed 查询其实际相对路径，Release 同步清除对应状态；赋值与 transfers 使用各自完整字段路径。原本拒绝的产品字段条件移动现可降码。模式转移、enum/error 载荷的条件部分移动、外部变体 guards 与临时/Prepared 构造状态仍拒绝；不得用产品状态树猜测这些责任。

回归将原空析构拒绝例改为可观察验收：分别覆盖单字段移动、嵌套字段移动、移出字段重新赋值、整体移走后只补回一个字段，以及补回全部字段。每种情况均检查两个独立 bool 的析构翻转，分支两侧和 O0/O3 都通过，既检查已移出字段不重复析构，也检查兄弟字段不被跳过。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 203 项、luna 库 17 项通过、退出 0（`/tmp/luna-tools-_npu55g4/result.json`）。测试总数不变，原拒绝测试已扩展为五组运行时验收。已格式化（`/tmp/luna-tools-xkwn3so3/result.json`）；未跑 workspace 全量测试，dev 改动未提交。

## 模式匹配的剩余临时对象

后端现在在模式与 guard 最终成功后，保留主体的原生存储供已验证的剩余清理计划使用。匹配绑定的静态字段路径及顺序按 schedule 的 pattern_transfers/pattern_orders 核对；主体的剩余字段由原计划决定，不根据 LLVM 的值复制猜测所有权。候选失败不提交剩余对象；let-else 失败时另保留完整输入，交给退出边界清理。

Temporary target 使用来源位置对应的栈存储及独立 runtime 存活标记。多个 match 候选合流后，optional target 只在实际选中过该分支时清理；必要 target 若不存活则 trap。计划执行完清除标记，避免跨循环残留。已接通普通表达式 TemporaryEnd、显式 Return 前的 TemporaryEnd，以及非 block match arm 的 ScopeEnd，保持参考执行器的先后关系。

当前支持绑定、通配、字面量、产品/变体/错误结果结构、观察及 guard 的静态绑定路径。Or/Remap/Then 等复杂动态回执在需要剩余资源清理时仍拒绝；带 IfConstructed 的 Temporary 计划、Prepared 构造槽、调用失败传播也未放行。临时主体存储只在完整求值并选中后建立，本项不能代表部分构造已完成。

新增 O0/O3 源级验收覆盖 let 解构、match 后剩余字段清理、动态 guard 成功与失败后选择下一候选、let-else 失败提前返回及成功路径。两个独立 bool 观察绑定字段和剩余字段各析构一次。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 205 项、luna 库 17 项通过，退出 0（`/tmp/luna-tools-1m09z8j3/result.json`）。已格式化（`/tmp/luna-tools-4p5vun4z/result.json`）；未运行 workspace 全量测试，dev 改动未提交。

## 替代模式的动态路径与绑定顺序

模式降码现在产生动态绑定回执：每个绑定保存原主体字段路径编码和建立顺序；Or 分支按实际成功候选合流回执，Remap 同时重命名值与回执。Then 的派生输入标为不来自原主体，不能假装是原字段。成功选中后才校验回执属于 schedule 允许的路径集合与顺序；失败候选不会提交转移。

产品布局的剩余 Temporary 增加字段存活标记。按每个绑定实际命中的路径清除被移出的子树，IfConstructed 使用对应字段标记，Release 同步清除状态。不同候选移出不同字段的计划因而可执行，不再要求静态唯一字段路径。sum 载荷的条件部分状态及 Prepared 部分构造仍未放行。

pattern_orders 现在保留运行时选中的顺序。非 Replacement 清理站点仅在该绑定组原有 action 位置中按实际建立顺序逆序执行，其他对象的清理位置保持计划顺序。多个顺序组若重叠当前明确拒绝。清理 action 与展开步骤计入 host 降码预算。Remap/Or 输出绑定同步初始化条件存活标记，后续条件移动可使用。

新增 O0/O3 回归：同一绑定名从不同字段取得，第一候选在已绑定字段后失败再进入第二候选；立即丢弃选中字段后观察中间状态，再确认剩余字段清理；两个候选交换绑定名时，两个有序写入的析构仍按实际建立顺序逆序执行。另外验证重映射绑定只在一条分支中移走时不重复析构。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 207 项、luna 库 17 项通过，退出 0（`/tmp/luna-tools-m9iowzij/result.json`）。已格式化（`/tmp/luna-tools-o7xxngm4/result.json`），未跑 workspace 全量测试，dev 改动未提交。调用失败传播、上下文特化、sum 部分移动及构造中的 Prepared 责任继续保留明确拒绝。

## 聚合构造中的已完成字段

Prepared target 开始接入 tuple/record/struct 与 enum 构造。每个字段完整求值后，若 schedule 对它有析构责任，就建立独立栈存储及存活标记，以 `(construction source, slot, value source)` 校验身份。后续字段的 return/break/continue 或已支持失败边界按既有计划清理这些值；未完成的字段不登记。整个聚合构造完成后清除准备槽责任，交给最终聚合值，避免成功路径重复析构。

admission 只认可实际构造节点中对应字段的位置与来源，且当前 Prepared 程序不含条件构造状态。Never 产品/enum 构造按字段顺序执行到退出，不要求构造一个不存在的完整布局。嵌套构造各有自己的准备槽，清理次序保持 schedule 的内层到外层顺序。

本项未覆盖函数实参、默认参数、closure capture、错误转换或返回结果的 Prepared target，也未解决跨调用失败传播。字段存储是完成字段的所有权存储，不是对整个未完成聚合做假构造。

新增 O0/O3 原生测试覆盖 struct/enum 构造的正常完成与提前 return；nested struct 在内层退出时通过有序写入确认内层先清理、外层后清理；循环内构造提前 break/continue 均确认已完成字段只析构一次。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 210 项、luna 库 17 项通过，退出 0（`/tmp/luna-tools-_4pwy0t5/result.json`）。已格式化（`/tmp/luna-tools-akqhqgps/result.json`），未运行 workspace 全量测试，dev 改动未提交。

## 普通调用的实参准备清理

Call 实参完整求值后，按 parameter index 登记 Prepared 存储，并保留实参表达式的 value source；登记顺序仍是 arguments 的源序。后续实参提前 return/break/continue 时，已有槽按验证计划清理。参数检查与路由完成、即将发出 call 时清除准备槽责任，由被调用函数的参数负责后续清理，成功路径不会重复析构。

admission 新增 Call 的精确 `(construction, parameter, value source)` 对照。此处没有把求值顺序替换成参数声明顺序：二者只分别用于责任顺序和槽定位。默认参数的特化上下文/失败清理、接口前置条件失败、跨函数 trap 传播仍待实现，不能由普通实参接线推定为支持。

新增 O0/O3 回归覆盖位置/命名实参的提前 return、循环 break/continue，及正常调用后的参数析构。命名实参用两个有序写入的析构区分：提前退出按实际求值顺序逆序；进入函数后按参数绑定清理计划执行。前一组翻转测试另外验证每个对象恰好析构一次。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 213 项、luna 库 17 项通过，退出 0（`/tmp/luna-tools-yayj4p18/result.json`）。已格式化（`/tmp/luna-tools-4jgus61n/result.json`），未跑 workspace 全量测试，dev 改动未提交。

## 返回结果的后置条件失败责任

没有命名 result binding 的函数在检查后置条件前，将完整返回值登记为 `(return origin, slot 0, return origin)` 的 Prepared 值。后置条件失败先执行对应 PostconditionFailure 的返回值清理，再 trap；成功则释放准备责任，继续正常返回清理并将值交给调用者。隐式返回使用函数体来源，显式返回使用各 Return 来源，互不混用。已有命名 result binding 仍沿 Local 责任路径处理。

admission 仅允许无命名结果绑定、实际存在后置条件且来源对应函数体/Return 的返回准备槽。没有扩大条件 Prepared、默认参数特化或跨调用失败传播能力。

新增用例覆盖隐式返回、显式返回和双返回点。每个后置条件失败块都检查 LLVM 中析构、清除存活标记、trap 的顺序与次数；O0/O3 原生成功路径验证调用者最终丢弃时只析构一次。输入函数满足静态契约，因此本项没有动态触发后置条件失败，其失败路径证据是 IR 检查。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 214 项、luna 库 17 项通过，退出 0（`/tmp/luna-tools-cfj_jksl/result.json`）。已格式化（`/tmp/luna-tools-pq6jcnlx/result.json`），未跑 workspace 全量测试，dev 改动未提交。

## 错误行与拥有资源的交叉验收

新增携带 *bool 的 Fault 析构用例，验证 ErrorConvert::Embed 扩大错误行之后，失败载荷最终只析构一次，成功分支不析构不存在的 Fault；另覆盖泛型 record 参数传递中的拥有错误载荷。IIR 明确包含 Embed，O0/O3 均通过。闭合转换在操作数完成后不再执行用户代码，本轮未为无可观察失败点添加多余准备槽，也没有修改前端语义。

证据修正：新的泛型 record 用例在发布 IIR 中已统一错误行，未留下 Product 节点。此前名为 product 的源级测试不能单凭名称证明该降码分支被执行；Product 的直接降码/资源组合覆盖仍需补充。普通 record 参数中隐式扩大内嵌错误行当前有前端缺口，详见 frontend-checkpoint.md。

在 `/home/fin/luna` 定向执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host native_owned_error_rows`，1 项通过、退出 0（`/tmp/luna-tools-_9ub8c55/result.json`），包含两组 O0/O3 原生编译执行。已格式化（`/tmp/luna-tools-x37lp5t1/result.json`）。本轮仅新增测试和检查点，没有重跑全部 host/workspace 测试，dev 改动未提交。

## Product 降码的直接覆盖

补充内部后端测试，显式构造嵌套 record 的物理错误行 `[bool,char] -> [char,bool]`，直接调用 `error_convert(Conversion::Product, ...)`。这绕过源级行归一化对待测表示边的消除，不改变源码语义，也没有开放可伪造 Prepared 的公开接口。

测试实际生成 LLVM、验证、优化、链接并执行 O0/O3：成功值保持 tag 0；两种错误载荷分别移动到新标签且保留 bool/char 值；外层 sentinel 字段不变；非法 tag 9 走 trap。共四种输入、八次本机执行。该用例证明 Product 的递归表示转换分支被执行，但不单独证明析构协议或前端自动插入 Product，资源转移仍由源级拥有错误载荷测试提供独立证据。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --lib host::llvm::errors::tests`，1 项通过、退出 0（`/tmp/luna-tools-7jo8s8k2/result.json`）。已格式化（`/tmp/luna-tools-m4t526jz/result.json`）；本轮只新增内部测试，没有重复整个 host/workspace 检查，dev 改动未提交。

## 默认表达式的独立清理上下文

Host 现在按调用位置和参数编号选择 `ContextKey::Default` 的确切 FunctionSchedule。进入默认表达式时隔离 locals、产品存活标记、剩余临时对象、Prepared 槽、模式顺序与循环状态，仅装入已准备的参数绑定；完成后恢复调用者状态。admission 沿实际默认实参节点解析该上下文，其他 Function/Closure/effect 特化上下文仍拒绝，不能将任意上下文统一到无上下文符号。

显式谓词失败的降码已接上逐层 DefaultFailure 展开：先清理当前默认表达式，再按内到外顺序恢复调用者计划及存储执行清理。正常路径不执行这条展开。默认表达式没有独立 FunctionEnd 调用点，故该上下文的 FunctionEnd Invoke 不予接纳；非局部控制转移也明确拒绝。

验证边界：带运行时 assumes 的默认值被前端 P3 检查拒绝，包括尝试的 assumes true；没有放宽此规则。因此本轮没有用合法源程序动态执行 DefaultFailure 展开，相关失败路径仍需直接降码测试。成功路径使用 P3 可接受的默认表达式：内部构造并清理空 Token，返回携带 *bool 的 Watch，在被调用函数结束时修改外部状态。重复调用与 O0/O3 均通过，既保留默认内部 Invoke，又确认返回拥有值交付后正常析构。另保留 P3 拒绝回归。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 217 项、luna 库 18 项通过，退出 0（`/tmp/luna-tools-d8mes2xe/result.json`）。已格式化（`/tmp/luna-tools-xm47siq7/result.json`），未跑 workspace 全量测试，dev 改动未提交。

## 默认失败展开的直接验证

新增内部降码测试，构造当前默认表达式、外层默认表达式和调用者三个清理环境。两个默认环境故意复用同一 LocalId，调用者持有 Prepared 槽；通过无缓冲字节输出观察实际析构顺序。O0/O3 下谓词失败均输出 `CBA` 后 trap，成功路径不输出任何析构字节并正常退出。另检查失败块生成之后，编译器恢复当前 locals、schedule 与展开栈，避免失败路径生成污染正常路径。

这是后端直接测试，不绕过或扩大源程序的 P3 接纳条件，也不证明跨函数失败传播 ABI 已实现。在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --lib host::llvm::defaults::tests`，1 项通过、退出 0（`/tmp/luna-tools-v2czhxuh/result.json`），包含四次原生执行。本次只新增测试，未重复全量 host/workspace 测试，dev 改动未提交。

## 运行时选择 staged callable

Host 开始消费 IIR 的 TemplateStorage 身份表：含 comptime 输入的普通 runtime callable 保存表内 i64 标签，物理类型仍携带完整逻辑签名域；Function/FunctionInstance 按声明及完整实例实参编码，TemplateEqual 比较同一域内标签。它们不是裸函数地址，也不能直接生成间接 call；实际调用继续使用 IIR 已完成的静态实例分派。没有身份表的 staged 签名明确拒绝。此改动不暴露源码函数相等运算，也不把标签变成普通整数类型。

新增 `development/examples/host-staged-callback.fl`，覆盖 factory 返回 callback、默认 callback 和两种 comptime 输入；按普通 host 示例命令构建 run，输出 `(bool true)`。`host-staged-callback-order.fl` 另需 `--copy-protocol Duplicate --drop-protocol Retire`，通过两次状态翻转验证 runtime 选择先于实参求值且各一次。两者及同一声明不同实例、可变 callback/别名均实际执行 O0/O3；标量测试另检查未优化 LLVM 保留身份比较，避免仅凭测试名称推定覆盖。

边界：捕获环境及尚未解析的 TraitMethod 仍不支持。可变模板别名在启用资源协议时有清理 source 冲突，已保留精确复现于 frontend-checkpoint.md；本项未放宽资源接纳规则。库 math intrinsic 在当前 IIR 尚无通用调用接线，本次没有据其名称虚构 LLVM intrinsic 映射。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 219 项、luna 库 19 项通过，退出 0（`/tmp/luna-tools-3zywwvan/result.json`）。已格式化（`/tmp/luna-tools-62b1l3sx/result.json`）。未跑 workspace 全量测试，dev 改动未提交。

## Trait 方法回调的本机表示

抽出 `llvm/callables.rs`，Function、FunctionInstance、TraitMethod 统一取得已完成实例。TraitMethod 按 interface、interface arguments、subject、member 与 method arguments 精确查找 IIR dispatch，普通方法取已选 body 的函数地址，staged 方法则保留完整方法身份。方法标签与普通 Function 标签占用不相交区间，按稳定 IIR 遍历分配；重复引用同一方法取得同一标签。不能因为不同 trait 身份选到同一个实现 body 就合并分派身份。表查找与遍历受降码 fuel 限制。

新增 O0/O3 原生验收四组：普通 trait 方法作为函数值；`Select<true>` 与 `Select<false>` 的 runtime 选择和重复 staged 调用；位于 comptime 输入之前的 bool receiver；普通函数与 trait 方法混合选择。未放宽资源规则：一般 nominal receiver 与部分方法回调的资源准备缺口记录在 frontend-checkpoint.md。

本轮完整回归首次在已有 Product 内部测试启动时遇到 `Text file busy`（`/tmp/luna-tools-pzaxqm_p/result.json`，18 项通过、1 项失败，host 尚未执行）。将三个内部原生测试的可执行文件改放在独占创建的 Workspace 中，退出后自动回收，避免使用仅含进程号的共享文件名。修复后，在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 220 项、库 19 项通过，退出 0（`/tmp/luna-tools-k2k98fd4/result.json`）。已格式化（`/tmp/luna-tools-sgcgf4ey/result.json`），未运行 workspace 全量测试，dev 改动未提交。

## 带资源验证的 staged 方法 receiver

修复 host 接入时暴露的物化交接缺口：已展开 callable 别名留下 void Unit 占位，而 CallPreparation 的 statements 要求是参数绑定。最终准备清理步骤仅移除确切的 void Unit；保留运行时绑定及其求值、资源责任，普通 Block 也不做此删改。没有让 move checker 接受任意准备语句。

原 nominal receiver 复现现在通过 paired Copy/Drop 准备并实际运行；新增拥有 Drop 的 Host receiver，调用结束后观察外部 bool 翻转，O0/O3 验证交付与清理均已接通。回归另检查准备序列仍存在且不含 Unit 占位。此修复不宣称普通 trait callback 的所有 callable obligation 已解决。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p elaboration --lib materialization::callable_tests`：10 项通过（`/tmp/luna-tools-zx2l_xh5/result.json`），包括移除占位、保留 runtime 绑定和普通 Block 的局部测试。随后 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test function_type_parameters`：host 222 项、函数类型参数 55 项通过（`/tmp/luna-tools-72x7l9up/result.json`）。均退出 0。已格式化（`/tmp/luna-tools-i1aqozpi/result.json`），未运行 workspace 全量测试，dev 改动未提交。

## 可变 staged 回调与 factory 的清理边界来源

修复可变回调降码复制来源的问题：每个读取选择标签的子节点分配独立 generated source，复制到不同分派分支的 callable 表达式通过已有 freshening 保留展开溯源。Match 包装节点与其内部读取不再共用清理身份。所有调用该降码的普通物化、template factory 和 recursive factory 入口传递同一展开来源表。

扩大回归时发现 factory 分派包装与其内部 tag 计算也共用了来源：`early_returned_local_template_preserves_factory_resource_cleanup` 在 run 的 TemporaryEnd 报两个匹配（`/tmp/luna-tools-eoo5qwvk/result.json`）。已同步修复 factory dispatch 的内部计算与复制分支来源；保留外层调用位置，不删减或合并清理站点。

原 `host-staged-callback-resource-boundary.fl` 现通过完整 Copy/Drop 准备，O0/O3 实际运行检查同一声明不同实例、可变 callback 与独立 alias 快照。新增局部测试检查重复读取和 factory 包装各自生成唯一来源，以及相同输入降码结果确定。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test function_type_parameters --test function_type_dependencies --test host`，函数类型依赖 33 项、函数类型参数 55 项、host 223 项通过，退出 0（`/tmp/luna-tools-_eszudb0/result.json`）。未运行 workspace 全量测试，dev 改动未提交。

相关物化单测另执行 `python3 -B development/check.py --timeout 300 -- test -p elaboration --lib materialization::callable_tests`，12 项通过（`/tmp/luna-tools-ieohredo/result.json`）。最终格式化完成（`/tmp/luna-tools-20fdybqq/result.json`）。

## 普通 trait 回调的资源来源传递

已完成 runtime instance 图中的普通 TraitMethod 值在资源分析中取得选中实现的 callable 来源，再按普通命名函数传给高阶调用；不再把有实现证据的方法参数报告成 unresolved callable obligation。仅处理无 staged 输入的 residual 方法值，保留 staged 身份机制；MethodCallees 使用完整接口检查的 select，不使用 behavior-only 选择擦除接口前后置条件。

新增三组 O0/O3 原生正例：方法值通过局部别名传参、按 runtime 条件选择不同 trait 值实参的方法、方法回调向指针目标写入。均启用 Duplicate/Retire 并先验证 Prepared 图。拒绝例确认携带接口谓词的方法值仍在前端报不能擦除 program contracts，旧输出保留；它不声称接口契约已完成组合降码。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test drop trait`，13 项通过（`/tmp/luna-tools-b2q4n4d3/result.json`）。随后 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test function_type_parameters --test function_type_dependencies`，host 225 项、函数类型参数 55 项、依赖 33 项通过（`/tmp/luna-tools-yxylz_73/result.json`）。均退出 0，已格式化（`/tmp/luna-tools-jvhf5_6b/result.json`）。未运行 workspace 全量测试，dev 改动未提交。

## Enum/error 整值的条件所有权

Host 不再因条件清理程序中含 enum/error 表示而一概拒绝。产品存活标记现在允许把一个完整 sum 当作不可拆分的字段 token；IfConstructed 只能检查根或连续产品字段路径，不能越过 sum 边界检查载荷局部状态。已知仍存活时，现有 Switch/Select 清理只执行活动变体的计划；载荷 Release 不伪造不存在的独立 token，整值 Release 清除 sum 的责任。

新增 O0/O3 原生测试覆盖 enum 的空/有载荷变体、error 的成功/失败分支分别被条件移动与未移动；另覆盖产品中的 enum 字段、条件移动后重新赋值、循环 continue/break 之后的状态重建。用析构修改外部状态验证活动载荷清理；局部 admission 测试明确拒绝 sum 内部的条件状态路径。带外置变体 guards 的条件计划、变体载荷内部部分移动仍未放行。

首次完整回归暴露了新增 Release 物理投影校验会误拒绝已擦除表示的 newtype（`/tmp/luna-tools-4no8cps7/result.json`，库 20 项通过，host 226 项通过、1 项失败）。已撤掉这一额外校验：Release 是经过上游验证的逻辑责任释放，不是读取物理字段；Invoke 和 IfConstructed 仍各自检查实际需要的布局/状态路径。

修正后在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 227 项、库 20 项通过，退出 0（`/tmp/luna-tools-489k1qdy/result.json`）。未运行 workspace 全量测试，dev 改动未提交。

## Staged trait 身份索引的一次性收集

将原先每次引用都扫描完整 IIR 图的身份编号查找，改为与 lambda 收集共用一次图遍历，按 graph.templates 类型域建立借用原节点的身份索引。编号仍使用相同稳定遍历顺序；普通 Function 标签前缀与方法身份标签保持不相交。查询只扫描所属域的唯一方法列表，不复制整张表达式图；收集、去重和查询仍消耗 fuel。本项减少重复遍历，不宣称已经完成性能基准测量。

新增单测检查同一方法不同源码位置共用标签、不同 subject 不合并、类型域独立，以及预算失败不写入半成品并可在补足预算后恢复。四组原生 trait 回调用例另比较两个独立编译进程生成的未优化 LLVM 文本，检查身份编号确定性，再执行 O0/O3。

条件清理中的外置变体 guards 仍保留明确拒绝，本轮没有将其与整值存活标记未经验证地合并。在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，host 227 项、库 21 项通过，退出 0（`/tmp/luna-tools-960_e3fl/result.json`）。已格式化（`/tmp/luna-tools-qmc48uik/result.json`），未运行 workspace 全量测试，dev 改动未提交。

## 原生 source test 入口

新增 `--test-native`，支持 `--opt-level 0..3`、`--test-filter`、paired Copy/Drop 资源验证及显式多包 primary 选择。直接按 tests 元数据中的 DeclarationId 建立 host 根，不靠测试标签查找函数；同名测试保持独立。`testing::run_native_report_filtered` 提供同一嵌入接口，复用 TestReport 的 S-expression 表示。

每项独立 LLVM 编译并启动进程；运行阶段默认 10 秒超时，超时或等待失败时终止并回收子进程，临时产物由独占 Workspace 回收。编译/链接工具仍由外层开发命令限时，不把原生运行超时冒充全流水线超时。子进程标准输入输出不进入报告。编译失败、trap、非零退出与超时逐项记录，后续测试继续；整体失败不覆盖旧报告文件。嵌入 API 可自行传入运行 deadline。`--inline-functions` 在原生测试模式下明确拒绝。

新增 `development/examples/host-tests.fl`，运行命令及边界见 source-tests.md。测试覆盖 O0/O3 的成功与 trap、重复标签身份、筛选时不访问工具链、工具缺失、零预算/零超时拒绝、无限循环超时后继续测试、资源析构、旧输出保护、互斥参数、多包 primary 身份和仓库示例。首次集成发现多包参数白名单遗漏新选项，已补齐 `--test-native` 和 `--opt-level` 转发。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --test frontend_cli --lib`，host 231 项、source_tests 9 项、frontend_cli 173 项、库 21 项通过，退出 0（`/tmp/luna-tools-4jatph2j/result.json`）。已格式化（`/tmp/luna-tools-lfhnm2c_/result.json`），未运行 workspace 全量测试，dev 改动未提交。

## 可配置的原生测试运行期限

新增 `--test-timeout-ms <positive integer>`，将逐项运行期限从固定 10 秒改为显式可配置，默认值仍为 10000 毫秒。普通 CLI 和 `--packages-v1` 一致传递 Duration 至原生执行器；只允许配合 `--test-native`，不改变编译阶段 fuel 或外层进程限时。校验零、负数、符号、非整数、溢出、缺值和重复选项。

新增 CLI 回归在普通/多包入口运行无限循环测试，均确认 200ms 超时后继续执行后一项，并报告两项中一项失败。定向 `python3 -B development/check.py --timeout 300 -- test -p luna --test host native_source_test_cli` 4 项通过（`/tmp/luna-tools-7zr994jo/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --lib`，frontend_cli 173 项、库 21 项通过（`/tmp/luna-tools-z2laftj_/result.json`）。工作目录 `/home/fin/luna`，均退出 0，已格式化（`/tmp/luna-tools-ow8wbsbu/result.json`），未重复全量 host/workspace 测试，dev 改动未提交。

## 原生 source test 的失败隔离验收

新增三组行为回归：一项包含当时 host 不支持的捕获闭包时，降码失败被记录，之后的成功测试、trap 测试及最后的成功测试继续独立运行；按标签筛选后也不受未选中的失败体影响。另使用 API 构造非 void 测试元数据，确认它被单独拒绝而后续合法声明继续执行。链接器缺失时每个测试身份分别报告失败，恢复正常 Toolchain 后同一组重复标签测试可成功运行，证明失败不会污染后续尝试。

本轮仅补充验证，没有放宽捕获闭包支持或改动运行语义。在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test source_tests`，12 项通过，退出 0（`/tmp/luna-tools-9fxwpqnh/result.json`）。未重复 host/workspace 全量测试，dev 改动未提交。


## 原生测试的有界输出诊断

`Toolchain::execute_test` 在运行期限轮询中读取两个非阻塞 Unix socket，每个流仅保留前 16 KiB；超出部分持续排空，失败时标注截断。诊断转义控制字符，非 UTF-8 内容以替换字符显示。成功测试的输出仍不进入报告。每轮读取次数有限，持续输出不能使 deadline 饥饿；子进程退出后不等待继承描述符的其它进程，也不引入需要阻塞 join 的读取线程。这是 Unix host 测试工具能力，不代表 Flurry 已支持一般 FFI。

工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 300 -- test -p luna --lib capture_tests`：2 项通过，退出 0，`/tmp/luna-tools-fkta5ls_/result.json`。真实 LLVM 程序覆盖双流失败诊断、控制字符转义、成功执行和持续输出下的截断与超时。
- `python3 -B development/check.py --timeout 300 -- test -p luna --test source_tests --test host native_source_test`：12 项通过，退出 0，`/tmp/luna-tools-0qsif2gj/result.json`；包含 O0/O3 仓库示例、资源清理、独立失败恢复、过滤、包身份和 CLI 超时。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-m8jxnbco/result.json`。

未重复全 workspace 测试；改动保留在 dev 工作区，未提交。


## 数值捕获闭包的 host 表示与调用

非空环境使用 `{ code_pointer, environment_fields }` 的 LLVM 聚合表示。环境按 IIR 语义次序内嵌，返回或交付闭包值时连同环境传递，不保存创建函数栈帧地址，也不为此引入堆分配。签名和环境类型仍由 IIR 检查，布局不参与语言类型等价。

闭包主体提升为带隐藏环境指针的内部函数，捕获 LocalId 绑定到该环境的字段位置。调用局部或产品字段中的闭包时保持该位置，因此主体更新可在后续调用中观察到；临时返回值建立调用期环境位置。创建只取得捕获，不执行主体。callee 先求值一次，之后实参依源序求值；重入读取／调用该环境保持当前状态，但实参中直接替换正在调用的接收者或其父位置会明确拒绝，不能拼接旧 code_pointer 与新环境。修改不相交的产品字段仍允许。

新增 `development/examples/host-closure.fl`，入口 `run` 输出 `(bool true)`。测试覆盖创建时快照、多个同类型但不同环境／主体的闭包、重复调用更新、返回后调用、高阶传参、环境移动后的状态、嵌套闭包、产品字段、提前 return、实参中的嵌套调用、独立编译的符号确定性；正例在 O0/O3 执行，并与参考执行器比较。仓库示例另有原生验收。

边界：这是现有不启用资源协议的 host 开发入口上的值表示与调用支持，不是闭包完整资源认证。普通闭包不因捕获全是标量就自动 Copy；闭包类型不能作为用户 impl 的主体。本轮未添加这两种能力。最初的重复交付用例不满足该资源依据，已改成合法环境移动和独立构造；未放宽正式语义或改动前端。该阶段仍拒绝配对 Copy/Drop 下的捕获闭包及 Closure cleanup context。后续已接通未调用环境的构造和析构，见下一节；闭包调用后的环境部分移出仍未实现。捕获复制及 Drop 必须随资源 schedule 接通，不能从此处的物理聚合搬运推导语言权限。原有 source-test 降码失败恢复用例现改用未提供资源协议的数据指针，保留真实拒绝场景。

回归中原有清理测试出现一次 `ETXTBSY`；独占路径仍可能因并发 fork 暂存写描述符而短暂不可执行。原生测试启动仅对此错误重试，最多 100 ms，未成功启动之前才重试，不重跑已经执行的测试。其它启动错误直接报告。运行期限仍从 child 成功启动后计算。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib`：host 239、source_tests 12、lib 23 项通过，退出 0，`/tmp/luna-tools-rzw_e3bx/result.json`。此前失败报告 `/tmp/luna-tools-1j6dsdak/result.json` 暴露未启用资源协议的参考执行不执行 Copy 深复制，不可用它认证复制权限；`/tmp/luna-tools-5u1q_xbb/result.json` 记录上述 ETXTBSY，均未冒称通过。未运行整个 workspace；dev 改动未提交。

补充验证：`python3 -B development/check.py --timeout 300 -- test -p luna --lib native_launch_retries` 退出 0（`/tmp/luna-tools-_uph259s/result.json`），保持真实 ELF 的写描述符验证 ETXTBSY，分别覆盖释放后启动成功、持续占用有界失败、释放后再次恢复。随后扩展数值环境用例，包含受范围条件保护的 u32 计数及捕获 u8 的 BitVec 局部写回；`--test host native_captured_lambda_snapshots` 退出 0（`/tmp/luna-tools-krryy6o0/result.json`），各案例参考执行与 O0/O3 结果一致。最终 `cargo fmt --all` 退出 0（`/tmp/luna-tools-54atnnq2/result.json`）。


## 未调用闭包的资源环境清理

配对 Copy/Drop 的 host 入口现可构造持有 nominal 资源和数据指针的闭包，并在不调用闭包主体的情况下移动、返回、交付和销毁环境。逻辑捕获字段 `i` 映射到 LLVM 聚合的 `1, i`，code_pointer 不占用语义捕获序号。此映射同时用于析构路径、嵌套环境、产品中的闭包，以及条件构造状态；完整捕获构造成功后才完成 Prepared 交付，已准备的捕获可按 schedule 保留。

捕获按 schedule 规定逆序析构，销毁不执行闭包主体。ScopeEnd、返回值丢弃、普通函数参数销毁、环境移动、赋值替换、break 及条件移动路径均有实际 LLVM 执行验证。环境中再捕获一个闭包和将闭包存入匿名产品也保持相同逻辑顺序。新增 `development/examples/host-closure-cleanup.fl`，使用文件顶部命令构建，入口输出 `(bool true)`。

接收闭包参数的普通函数可能只具有调用点专用的 `Function` 清理上下文。后端现在选择归属于实际 callee 的计划，逐项比较 sites、discards、transfers、copies、pattern transfers 和 pattern orders；完全相同才复用同一机器函数。不同计划、嵌套专用上下文或递归 context aliases 明确拒绝，不能任意挑一份计划。IIR 清理结构补充结构相等比较，没有改变类型命题或实例身份。单元测试覆盖相同计划复用、责任转移差异、嵌套／递归拒绝、预算耗尽和随后恢复。

条件移动还暴露了参考解释器的清理错误：Release 将旧表示清空后，IfConstructed 对其子字段的投影会先报 Unsupported。现在逐个父路径检查责任是否仍构造，遇到已释放父位置即跳过子步骤；活位置上的非法索引仍报 InvalidPointer。路径工作计入执行预算。源级条件闭包移动在 true/false 两条路径各析构一次，另有独立清理单元测试验证错误和恢复。这是执行层修复，未扩展或放松语义前端。

该阶段仍未实现捕获闭包的调用上下文；后续的一致计划调用支持见下一节。调用后的环境部分移出，以及失败跨函数调用向外传播时的清理仍未实现。例如已有活闭包时调用 `make` 构造替换值，schedule 含调用失败清理时仍拒绝；测试确认保留旧输出文件。直接构造新环境后赋值替换已支持。闭包 Copy 不由此次物理环境表示自动推导，也不允许把闭包类型作为用户 impl 主体。

工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib`：host 244、source_tests 12、lib 25 项通过，退出 0，`/tmp/luna-tools-7ihxnung/result.json`。新环境用例在 O0/O3 执行并与启用配对协议的参考执行对照，仓库示例另有验收。
- `python3 -B development/check.py --timeout 300 -- test -p comptime_interpretor --lib cleanup::`：13 项通过，退出 0，`/tmp/luna-tools-xa3n2y8a/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-tz1qzo4y/result.json`。

`/tmp/luna-tools-6xlp536n/result.json` 记录修复前条件移动在参考执行器中的 Unsupported。未重复整个 workspace 测试；dev 工作区未提交。


## 闭包主体的配对资源调用

host 现可将 `ContextKey::Closure` 的清理计划接入提升后的 lambda 函数。按定义定位具体 IIR 正文，重用计划前比较所有 sites、discards、transfers、copies、pattern transfers 和 pattern orders。调用点必须在当前资源作用域中有对应的 closure context，不能仅有一个类型相容的代码指针就跳过清理证据。lambda 的 FunctionEnd 使用自身正文 source；不继承创建函数的 pre/postcondition、结果绑定或结束边界。

已支持数值／指针捕获的重复调用、只读观察仍持有的 nominal 捕获、捕获资源的原地替换、每次调用创建的局部资源析构、提前 return、以及空环境 lambda 的资源参数析构。捕获替换用例验证旧 Watch 在赋值时析构，新 Watch 在环境退出时析构；返回闭包的用例验证环境脱离创建栈帧后仍可重复调用。参数析构通过 Drop 中的运行时契约失败验证，漏掉 Drop 会错误成功，因此测试不是只检查生成文本。

新增 `development/examples/host-closure-call.fl`，使用文件首行的配对协议构建命令运行，输出 `(bool true)`。其中两次调用分别通过提前 return 和正常退出结束，每次均触发局部 Watch 的析构。所有新执行正例均与启用同样协议的参考执行比较，并运行 O0/O3。

当前接纳要求不移出任何捕获字段；因此每次调用入口可初始化捕获的局部构造标记，环境的实际字段内容仍保留更新。涉及捕获所有权转移时明确拒绝，直到把构造标记存入环境并同步调用者／被调用者。不同调用计划、默认参数／handler 子上下文或递归 cleanup context 仍不复用单一机器函数。普通函数／lambda 嵌套调用的后续支持见下一节。定义 source 对应不同具体实例正文时也拒绝，不能把不同泛型实例合成一套 source-only 清理计划。拒绝用例验证输出文件保持不变。

真实前端缺口另存 `development/repros/host-closure-parameter-provenance.fl`：将含指针字段的 Watch 直接作为 lambda 参数时，资源准备报 `indirect Drop access requires stored pointer provenance`（本轮报告 `/tmp/luna-tools-1cbtd61c/result.json`）。未绕过证明、未放宽资源规则。无此来源证明缺口的资源参数调用已验证析构。一般跨调用失败清理仍待实现。

工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib`：host 248、source_tests 12、lib 25 项通过，退出 0，`/tmp/luna-tools-vd7z9k7y/result.json`。
- 新增返回闭包用例后执行 `--test host native_checked_returned_closure`：1 项通过，退出 0，`/tmp/luna-tools-ovz9g7iv/result.json`。
- 扩展捕获替换场景后执行 `--test host native_checked_closure_calls`：1 项通过，退出 0，`/tmp/luna-tools-zaqh_9h2/result.json`。
- 新增不同泛型实例拒绝例后执行 `--test host native_checked_closure_distinct`：1 项通过，退出 0，`/tmp/luna-tools-t66tj1kj/result.json`。
- 最终 `python3 -B development/command.py --timeout 120 -- cargo fmt --all` 退出 0，`/tmp/luna-tools-0y_vhp3v/result.json`。

未重复 workspace 全量测试，改动保留 dev 工作区，未提交。


## 普通函数与闭包的嵌套清理调用

嵌套 `Function` / `Closure` context 现在按独立机器调用帧处理，不再因为存在子上下文就拒绝外层闭包。每个 named function 和 lambda 仍分别检查其全部计划，只有直接清理、责任转移、复制和模式记录完全一致才复用机器函数。嵌套 callee 的计划由其主体自己的降码接纳检查负责；父帧中实际执行的 handler 上下文仍限制复用；Default 子计划的完整比较及递归 named-call context aliases 见后续进展。普通函数清理作用域定位允许 `Closure -> Function` 路径，FunctionEnd 对应实际 callee 正文。

正例包含二层／三层闭包、闭包经 apply/relay 普通函数交付并执行、外层闭包每次创建新的内层闭包后交付、多次调用及局部 Watch 的析构。另以 Drop 内契约失败验证 `Closure -> Function` 参数析构确实执行；嵌套捕获移出拒绝例证明子上下文不能绕过尚未接通的责任状态同步，并确认旧输出文件不变。没有调整前端或赋予闭包隐式 Copy。

新增 `development/examples/host-nested-closure.fl`。按文件首行命令启用 Duplicate/Retire 构建，运行输出 `(bool true)`；外层两次调用分别创建并销毁自己的内层 Watch。O0/O3 均已执行。

工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib`：host 253、source_tests 12、lib 25 项通过，退出 0，`/tmp/luna-tools-quzzxmi0/result.json`。
- 新增仓库示例后，`python3 -B development/check.py --timeout 300 -- test -p luna --test host shipped_nested_checked_closures`：1 项通过，退出 0，`/tmp/luna-tools-_srzixp6/result.json`。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-3_wgv49t/result.json`。

捕获部分移出、同一源码位置不同具体闭包实例及跨调用失败清理仍待推进。递归普通函数的计划别名见后续进展。未重复 workspace 全量测试，改动保留 dev 工作区，未提交。


## 递归普通函数的清理别名

接通资源 schedule 发布的递归 named-call 别名。别名必须指向同一 instance 中唯一存在的具体祖先 scope，且源调用与目标 scope 的 callee InstanceKey 完全相同；不得指向自身、非祖先、不同 callee、不同 callable kind 或 alias chain。重复边、具体 scope 与别名冲突、歧义／缺失目标均拒绝。目标 callee 还必须存在于 IIR 图中，路径检查和查询计入 host fuel。

原有按 callee 比较清理计划的检查仍然执行。因此递归只是复用已证明一致的机器函数，不是从别名存在推断任意调用上下文可互换。闭包内部发起的递归普通函数调用也适用；直接以 Closure 为末节点的别名，以及穿过 Default/handler 的别名未开放。捕获所有权移出仍受原限制。

新增 `development/examples/host-recursive-closure.fl`，用文件顶部配对协议命令构建，输出 `(unsigned-integer 4)`。普通递归函数每次调用闭包，闭包内 Counted 的析构递增计数。源级验收先确认准备产物中实际存在 aliases，再比较参考执行与 O0/O3 原生结果；更深递归、互递归、外层闭包发起递归并重复执行分别验证 4、5、8 次析构。单元测试覆盖祖先关系、精确 callee 身份、唯一性、缺失目标、预算耗尽及恢复。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib` 退出 0，host 257、source_tests 12、lib 26 项通过，报告 `/tmp/luna-tools-8nr9fvfq/result.json`。最终 `python3 -B development/command.py --timeout 120 -- cargo fmt --all` 退出 0，`/tmp/luna-tools-xh_papce/result.json`。实现前递归调用拒绝记录于 `/tmp/luna-tools-ahjhfe72/result.json`；初稿计数用例的比较符空格导致解析失败，已修正用例并完成最终检查，没有修改解析规则。未运行 workspace 全量测试，dev 改动未提交。


## 闭包与专用调用帧中的默认参数

默认参数子计划现在参与机器函数复用检查。按相对 context 路径收集当前帧内的 Default 子树，逐项比较 sites、discards、transfers、copies 及模式记录；父计划相同但子计划不同、缺失或重复时，不能共用函数体。Function/Closure 子调用拥有独立机器帧，由自身计划选择负责，handler 子上下文仍拒绝。收集和比较工作计入 host fuel。

清理接纳从 context 中最后一个 callable 节点定位真实函数／lambda 正文，再逐层进入尾随 Default 表达式。因此 Closure -> Default、Closure -> Default -> Default，以及经普通函数交付的闭包中的默认参数，都可以保留自己的清理边界。Default 不取得 lambda 的 FunctionEnd 身份；现有默认参数失败清理、状态恢复、P3 限制和非局部 return 拒绝均保留。

源级 O0/O3 验收覆盖默认块创建局部 Token 后返回拥有 Watch 的参数、二层默认参数交付资源、重复闭包调用、外层闭包创建内层闭包并经普通函数调用。另验证默认参数构造闭包后，callee 在默认帧结束后调用它。新增 `development/examples/host-default-closure.fl` 以两次不同指针实参验证环境不复用旧实参；按文件顶部命令运行输出 `(bool true)`。所有新正例与配对协议参考执行对照。

工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib`：host 259、source_tests 12、lib 26 项通过，退出 0，`/tmp/luna-tools-7b849wyx/result.json`。其中现有默认参数失败清理单元测试和源级 P3 拒绝例继续通过。
- 追加默认构造闭包后执行 `--test host native_checked_default_constructed`：1 项通过，退出 0，`/tmp/luna-tools-mo2wlje_/result.json`。
- 追加仓库示例后执行 `--test host shipped_default_closure`：1 项通过，退出 0，`/tmp/luna-tools-zjdw2vph/result.json`。
- 最终 `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：退出 0，`/tmp/luna-tools-4uy235a5/result.json`。

仍未接通捕获移出后的持久构造状态、同源码位置不同具体闭包实例的计划身份，以及算术 trap 等非谓词失败清理；穿过 Default 的递归别名也维持拒绝。未运行 workspace 全量测试，改动保留 dev 工作区，未提交。

## 默认闭包中的递归调用边界

补充源级验收：默认参数构造捕获 bool 的闭包，callee 随后调用该闭包，闭包调用含局部 Token 析构的递归函数；重复外层调用在配对资源协议下与参考执行器一致，LLVM O0/O3 均返回 true。此例没有发布递归 cleanup alias，不能将它作为穿过 Default 的 alias 已接通的证据，因此保持现有别名接纳限制。

直接在默认表达式中调用递归函数仍需 P3 stability/termination evidence，当前前端拒绝本例。新增拒绝测试同时验证原生构建失败保留旧产物，没有为后端运行放宽默认值语义。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test host recursive` 退出 0，5 项通过，报告 `/tmp/luna-tools-e3bo5yx0/result.json`。格式化命令 `python3 -B development/command.py --timeout 120 -- cargo fmt --all` 退出 0，报告 `/tmp/luna-tools-brmr2dlc/result.json`。未重复全量测试；改动留在 dev，未提交。

## 递归析构的可观察验证与拒绝定位

新增默认闭包调用递归函数的资源正例：递归返回后创建 Token，析构通过已验证指针累加外部计数。两次外层调用总计析构四次；参考执行与 LLVM O0/O3 结果一致。相比空析构，该验收能发现漏掉 Invoke 的实现。

同一资源若改为在递归调用之前构造并跨调用存活，参考执行返回正确计数，但 host 仍拒绝对应 PredicateFailure 清理站点。这是跨调用失败传播未接通，不是递归本身或默认闭包不合法。新增负例保留此真实缺口，并检查构建失败不覆盖旧产物；不能以将构造移动到调用之后作为该特性的实现。

后端拒绝 Invoke 时现在报告具体 source 范围、Boundary 和实例身份；诊断继续保留原来的拒绝信息。负例精确断言递归调用的 source 范围和 predicate-failure 边界，方便下一步实现失败 ABI 及逐帧清理。

本轮 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host recursive` 退出 0，7 项通过，`/tmp/luna-tools-vnjixrv7/result.json`。格式化退出 0，`/tmp/luna-tools-f8v86_e0/result.json`。首次包含跨调用活资源的正例尝试被 host 拒绝，报告 `/tmp/luna-tools-k1f5m89p/result.json`，现拆为上述独立正例与缺口回归。未运行全 workspace 测试；dev 未提交。

## Host 内部谓词失败传播与逐帧清理

原生 checked 模块新增私有 thread-local 失败位；所有机器函数保持原参数和结果布局。显式谓词／前后置条件失败先执行本帧既有 cleanup schedule 和默认参数 unwind，然后设置失败位并返回不交付的零占位结果。普通及闭包调用在消费结果之前检查失败位，执行调用点 PredicateFailure 清理，再向自己的 caller 传播。入口适配器检查后才打印结果；失败仍以 trap 终止，不变成 Flurry 错误结果或代数效应。

进入失败清理前暂时清除失败位，保证正常执行的 Drop 不会误认继承的失败；完成清理后重新设置。每次 Drop 后检查其自身失败，析构失败立即终止，不继续操作半析构值。这是当前无 FFI/export 的 host 内部协议，并非语言公共 ABI；未来接入跨模块调用、线程运行时或 FFI 时须明确接线与重入边界。算术安全 guard、非法内部标签等无程序清理来源的 trap 仍立即终止；本项不宣称所有异常均可 unwind。

接通调用点失败边界后，原来拒绝的跨递归调用活资源、enum 资源替换和闭包环境替换已经可以原生运行。新增递归与闭包失败验收：保持编译生成的全部函数和 cleanup 不动，仅替换测试入口，读取两帧资源实际写出的外部计数及失败位，验证两次析构、失败后的写回不执行；O0/O3 均通过。正常入口另外实测终止，保证占位结果不被打印为成功值。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib` 退出 0：host 266、source_tests 12、lib 26 项通过，报告 `/tmp/luna-tools-gxjuoofz/result.json`。追加闭包失败计数用例后，定向 `--test host native_closure_failure_unwinds_callee_and_caller` 通过，报告 `/tmp/luna-tools-kd5p5rej/result.json`。第一轮回归的五项失败均是旧拒绝预期或紧邻 trap 的文本断言，已更新为新路径并保留实际执行验证（原报告 `/tmp/luna-tools-ry68m_d7/result.json`）。

历史章节中“跨调用失败清理未实现”的描述由本节限定更新：已有命名函数、递归和闭包的显式谓词失败路径已接通；一般 runtime／FFI 异常和 handler 仍未接通。改动保留 dev，未提交。

测试辅助函数去重后，定向 `--test host failure_unwinds` 两项通过，退出 0，`/tmp/luna-tools-vpst05m3/result.json`；全 workspace 格式化退出 0，`/tmp/luna-tools-nw2t85zz/result.json`。

## 失败传播中的拥有值交付验收

新增原生实参求值失败测试：第一个实参已构造 Token，第二个实参调用失败，callee 尚未取得参数；第一个实参和 caller 局部资源各析构一次。沿用生成函数的外部计数测试入口，O0/O3 验证次数与失败位，并验证失败后写回 99 不会执行。

前置／后置条件测试使用合法源码（caller 以 checked assumes 建立 flag，callee requires flag、ensures flag）。前端正确拒绝不满足契约的原始尝试；不能用非法源码跳过证明。因此这里只对选定机器函数中保留的运行时 guard 做故障注入，将条件强制为 false，其余机器函数、返回路径和 cleanup 代码保持原样。前置条件失败验证已交付参数归 callee 清理；后置条件失败验证未交付结果归 callee 清理，覆盖隐式／显式 return 和命名／未命名结果四种组合。结合 caller 的局部 Token，外部计数必须恰为 2，失败位必须置位；重复接收、重复析构、漏析构或继续执行都不能通过。这是后端防御分支测试，不是声称存在可通过前端证明的违约源码。

工作目录 `/home/fin/luna`。定向 `python3 -B development/check.py --timeout 300 -- test -p luna --test host condition_` 三项通过（其中一项含四种返回组合），报告 `/tmp/luna-tools-pclgf27q/result.json`。共用测试入口整理后执行 `--test host failure` 全部通过，报告 `/tmp/luna-tools-8ykcue8i/result.json`；格式化退出 0，报告 `/tmp/luna-tools-olgjxm0p/result.json`。未重跑 workspace 全量测试，未更改前端契约规则，dev 未提交。

## 清理调用与析构失败边界验收

新增源级递归失败用例：Token 的 Drop 经普通函数 increment 修改外部计数。两层 unwind 时 helper 正常执行且各加一次，验证清理前暂存原失败位不会让 helper 误认为继承了失败。O0/O3 均通过，最外层依旧终止。

新增析构失败用例覆盖正常退出与已有谓词失败后的二次失败。先构造 Later，再构造 Failing；Failing.drop 写入 1 后调用会失败的 stop，随后写 77，Later.drop 写 99。测试仅将最终 trap 替换为观测／退出函数，保持生成的函数体、清理顺序和失败检查不动：终止时必须仍为 1 且失败位置位。如果吞掉 helper 失败或继续执行后续析构，就会得到 77 或 99；如果吞掉终止，则 main 返回失败。两种退出情形在 O0/O3 下均通过。这些是已有终止策略的执行证据，没有将析构失败改成可恢复错误。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host failure` 退出 0，报告 `/tmp/luna-tools-fob3iui7/result.json`。全 workspace 格式化退出 0，报告 `/tmp/luna-tools-73w3lukq/result.json`。未重复全量测试，dev 未提交。

## 空 effect 行的降码

空行的 EffectComputation、空安装环境／handler 表的 EffectHandle，以及零 routing slot 的 EffectRun 在 host 中按原位置继续求值。后两者额外拒绝携带非空 computation 行的输入；非空请求、handler 和环境没有因此被擦除，仍需后续 effect ABI 实现。此路径不将 effect 与错误结果合并，也不改变 IIR 类型身份。

资源验证修复了空行规范化的不一致：`Term::computation([], T)` 是 T，但显式 EffectComputation 节点可以保留。moves 的来源分析现在在空行且结果类型一致（或合法 Never body）的情况下继续分析 body；非空行仍校验声明行与完成类型。这是支持已验证 IIR 到 host 的必要接线，未放宽 Copy 或借用规则。

新增 `development/examples/host-empty-effects.fl`，普通 `--build run` 输出 `(unsigned-integer 42)`。测试覆盖普通／显式 return、重复调用、拥有 Watch 的 callee 析构对外部状态的影响（参考执行与 O0/O3 对照），以及非空行拒绝时保留旧产物。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test effect_memory_transfers --test effect_spreads` 退出 0，85+21 项通过，`/tmp/luna-tools-810hzvu_/result.json`。最后定向 `--test host effect_` 三项通过，包含仓库示例，`/tmp/luna-tools-s911imqs/result.json`。格式化退出 0，`/tmp/luna-tools-4ver4cae/result.json`。未运行 workspace 全量检查，dev 未提交。非空 effect handler、continuation 和 runtime 实例 ABI 仍待实现。

## 空行 completion 的来源与逃逸回归

新增指针、拥有 Watch 的值和捕获闭包三类 completion，经 `# {}` 返回后分别写回原目标、在 caller scope 退出析构、调用保存的环境。参考 checked 执行与 LLVM O0/O3 一致，证明空包装没有丢掉来源或改为提前释放。另有 `#[] Pointer` 返回自身局部地址的拒绝用例，检查空行包装不掩盖逃逸且构建失败保留旧产物。

本轮检查 `run_computation`：规范化为空行的值直接返回，不生成额外 EffectRun。未为不可达的零 slot 假设修改解释器路由，也未据此放宽非空 effect 环境。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_empty_effect_rows_preserve_delivered_origins` 退出 0，包含三个源级 O0/O3 场景，报告 `/tmp/luna-tools-wa7vf6hd/result.json`；`--test host native_empty_effect_row_does_not_hide_pointer_escape` 退出 0，报告 `/tmp/luna-tools-8ykwc6fm/result.json`。格式化报告 `/tmp/luna-tools-3oh3pafg/result.json`。仅补充行为证据，非空 effect ABI 仍未实现；dev 未提交。

## 同一机器函数内的普通 direct effect

新增 `host/llvm/effects.rs`：handler 安装保存词法 locals 的存储槽和外层环境；环境表保持 operation 完整身份及 occurrence 次序。请求从 allowed_index 取入口，核对 operation 与签名，实参按源码顺序求值一次，再构造内部参数包并复用模式匹配降码。分支运行时恢复安装点 locals 和外层环境，结束后恢复请求侧上下文。因此捕获写回与嵌套同名操作的外层查找都保留。

这条路径在同一机器函数内展开普通 Direct handler 分支，不宣称已实现跨函数 effect ABI。首版要求 total handler，拒绝 fallback、DirectPassing/Capturing/Async、非局部 return/break/continue/resume；启用 paired Copy/Drop 的资源 handler frame 仍明确拒绝。空 handler 的既有资源路径保留。effectful 函数的独立机器调用仍因缺少 computation 表示／环境参数而拒绝，不把它当成普通函数静默调用。路由、安装及捕获环境复制计入 fuel。

模式匹配提取了接收已求值 Value 的入口，使请求实参不会在选择分支时重算。未新增源级 tuple 类型，也未展开 normal-form 头。新增 `development/examples/host-direct-effect.fl`，按顶部命令构建输出 `(bool true)`，示范内层 handler 分支请求外层同名操作。

源级验收覆盖零参／有参、布尔分支覆盖、词法捕获读取与写回、实参副作用先于 handler、嵌套同名 handler 的外层请求；参考执行与 LLVM O0/O3 一致。跨函数效应和 checked 资源 handler 的拒绝用例检查旧产物保留。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --lib` 退出 0，host 280、source_tests 12、lib 26 项通过，报告 `/tmp/luna-tools-hk9piw7m/result.json`。追加仓库示例后定向 `--test host native_lexical_direct_effect_handlers_execute` 通过，报告 `/tmp/luna-tools-rtu_xqhg/result.json`。格式化退出 0，`/tmp/luna-tools-n72kybq4/result.json`。未运行整个 workspace；dev 未提交。

## 普通 direct handler 的部分覆盖与 fallback

同一机器函数内的 direct handler 现支持部分覆盖。安装时将 fallback slot 解析到安装点外层环境，核对完整 operation、签名与 ABI。模式或 guard 全部拒绝后，匹配降码把已求值的参数包交给该外层入口，不重新执行请求实参。每个被选入口使用自身保存的词法 locals 和外层 effect 环境；guard 已完成的写回保留在原存储槽中。外层链由安装快照形成，dispatch 另有深度与 fuel 限制。

普通 match 的全覆盖末端仍保留 trap；仅有明确 fallback 的 effect 匹配才在未命中路径调用外层。内层命中后直接汇合结果，不执行 fallback。资源 handler frame、非局部 handler 控制和跨函数 ABI 的原限制不变。

源级 O0/O3 验收覆盖内层命中、未命中、两层连续转交、实参副作用恰好一次、guard 拒绝及 guard 写回被外层观察。全部与参考解释器结果相同。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib` 退出 0，host 281、lib 26 项通过，报告 `/tmp/luna-tools-e3b42j89/result.json`。格式化退出 0，报告 `/tmp/luna-tools-2yoo6g1g/result.json`。未运行 workspace 全量测试，dev 未提交。

## Direct handler 的提前 resume

同一机器函数内的普通 direct handler 现有独立的结果槽与汇合标签。resume 求值并存入该槽，跳至当前请求的汇合点；不会返回整个 Flurry 函数，也不携带 continuation。普通 match 沿用当前 handler 边界，嵌套 handler 与 fallback 则暂时安装自身边界并在降码结束后恢复外层。记录显式 resume 的路径，保证所有分支均提前交付时仍生成合法汇合标签。

Handler 自身定义的循环现在允许局部 break/continue；目标必须属于该分支内的 Loop identity，外部目标与函数 return 仍拒绝。guard 在已发生控制转移后不再尝试把 Never 结果作为 bool。循环降码同时修复 Never 类型的末端：生成 unreachable，不能将不存在的正常完成视为 Unit 并继续写入结果。

源级 O0/O3 与参考解释器对照覆盖普通／条件 resume、跳过后续副作用、随后再次请求、嵌套同名 handler、fallback、普通 match 内 resume、循环与 guard 内 resume、局部 break/continue。另保留函数 return 的拒绝回归：参考执行确实返回整个函数，而 host 仍明确拒绝该未实现路径，不将其偷换为 resume。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib` 退出 0，报告 `/tmp/luna-tools-bb2sn_vg/result.json`。格式化退出 0，`/tmp/luna-tools-zdie04gt/result.json`。本项仍限非资源的词法 direct 路径；资源 handler、跨函数效应 ABI、捕获式 continuation 未完成。未运行全 workspace 测试，dev 未提交。

## Direct handler 的词法 return 与循环退出

同一机器函数内的非资源 direct handler 现在允许函数 return 和安装点外层循环的 break/continue。安装时保存循环 identity 到 LLVM 标签的映射，分派时与 locals／effect 环境一起恢复；分支内创建的循环仍有自己的目标。return 沿已有机器函数返回路径执行，resume 仍只跳到请求私有结果边界，两者不混淆。原 `native_direct_handler_function_return_is_not_resume` 已由拒绝用例改成实际执行，与参考执行一致。

新增用例发现 handler 克隆正文中的 lambda 地址不再匹配原符号表。后端对非原地址节点查询完整已实例化定义，先校验定义一致且不存在同 source 的不同实例，再取原节点的注册符号；不允许只凭 source 共享不同代码。查询仍受遍历 fuel 限制。这接通分支内无捕获／有捕获 lambda 自身的 return，不误退出安装者。

O0/O3 源级对照覆盖 handler 的函数 return、guard 中 return、外层 break/continue、fallback 分支 return、嵌套包装以及分支内 lambda 的独立函数边界。资源 handler frame 与跨函数 effect 环境 ABI 均未在本项放宽。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib` 退出 0，报告 `/tmp/luna-tools-mxcerd4i/result.json`。格式化退出 0，报告 `/tmp/luna-tools-ns0o4e3r/result.json`。未运行全 workspace 测试，dev 未提交。

## 词法 direct passing 的安装值

零参数、已支持标量结果的 DirectPassing handler 现在在安装时执行 provider。安装实例保存其 Value，随后请求直接取得这个安装结果，不再运行 provider；嵌套安装分别保存值，provider 只使用外层环境，退出覆盖后恢复外层值。安装在循环中出现时，对应指令每次循环仍会执行。provider 的 return/break/continue 可以在主体开始前退出；resume 则完成提供值。部分 provider 的 fallback 取得外层已保存的值，不重新供应外层。

仅接通当前词法非资源路径的零参数标量签名。非标量／拥有值需要后续 Copy 或一次消费交付接线，当前明确拒绝，不能因实现保存一个 Value 就授予复制权限；此限制不是语言禁止函数值或其它类型。checked 资源 handler 与跨函数 environment ABI 仍未接通。

新增 `development/examples/host-direct-passing.fl`：provider 翻转安装标记并 resume true，两次请求后结果仍为 true，证明供应只执行一次。源级对照还覆盖主体短路而未请求、安装先于主体写回、内外覆盖与恢复、循环重新安装、fallback、provider 提前函数返回；O0/O3 与参考执行一致。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib` 退出 0，host 285、lib 26 项通过，报告 `/tmp/luna-tools-qw7ck226/result.json`。加入示例及非标量拒绝后定向 `--test host direct_passing` 两项通过，报告 `/tmp/luna-tools-rpzitmps/result.json`。未运行全 workspace 测试，dev 未提交。

## Host 的可选联合内联与资源 effect

`--build` 和 `--emit-llvm` 现在接受 `--inline-functions`，要求显式配对 Copy/Drop 协议。host 在源 Package 的副本上调用已有 `elaboration::program::inlining::expand`，再从展开后的 Package 重新选择协议、准备入口、实例化、校验资源与生成 cleanup schedule，最后交给 LLVM。原 Package 不修改；不复用展开前的资源计划，也不在编译时解释执行入口。展开预算沿用每阶段 fuel 约定。

这使符合已有联合展开契约的 inline callee + inline direct effect handler 可以带资源原生执行，包括 handler 局部析构和提前 resume。不是通用 effect ABI 的替代：未能展开的跨函数 effect 调用仍拒绝，原生 test 驱动尚未开放此选项。CLI help 已更新，缺协议在进入构建前拒绝；失败不覆盖旧产物。

示例 `development/examples/host-inline-effect-cleanup.fl`：

```sh
python3 -B development/check.py --timeout 300 -- build -p luna
python3 -B development/command.py --timeout 30 -- /tmp/luna-dev-002/target/debug/luna development/examples/host-inline-effect-cleanup.fl --build run --inline-functions --copy-protocol Duplicate --drop-protocol Retire --output /tmp/flurry-inline-effect
python3 -B development/command.py --timeout 10 -- /tmp/flurry-inline-effect
```

示例的 handler 创建 Watch 后 resume，展开后的清理翻转 caller 的 seen，输出 `(bool true)`。测试对普通交付、提前 resume 与仓库示例分别执行参考 checked 内联、LLVM dump 和 O0/O3 原生程序，观察实际析构效果。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --lib --bin luna` 退出 0，host 288、frontend_cli 173、lib 26、bin 1 项通过，共 488 项，报告 `/tmp/luna-tools-06xomoj7/result.json`。格式化退出 0，报告 `/tmp/luna-tools-mnngt511/result.json`。未运行整个 workspace，dev 未提交。

## 原生测试入口的可选内联

`--test-native --inline-functions` 现与 host 构建共用已检查的可选展开，要求 Copy/Drop 配对绑定。新增兼容 API `run_native_report_filtered_with_inlining`，旧 `run_native_report_filtered` 的行为不变。原生测试按原声明身份发现、过滤与记录结果；首个选中测试需要编译时才展开源码副本，后续测试复用该副本，但各自重新准备实例与资源清理计划。展开失败作为选中测试的失败报告，不偷偷回退到未展开执行。

支持单文件及经共用 Action 路径的 package CLI 接线。空过滤不展开也不启动 LLVM；CLI 配置检查和测试元数据检查仍保留。执行失败后继续后续测试，汇总失败时保留已有报告文件。原生运行时 deadline 和按阶段 fuel 规则不变。

`development/examples/host-inline-effect-cleanup.fl` 已加入 `test cleanup`，可以用 `--test-native --inline-functions --copy-protocol Duplicate --drop-protocol Retire` 运行。其测试先取得 run 的结果，再用 assumes 验证，未绕过 predicate 对调用的现有限制。

本轮 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test source_tests --test frontend_cli --lib --bin luna`：退出 0，host 289、source_tests 12、frontend_cli 173、lib 26、bin 1，共 501 项通过，报告 `/tmp/luna-tools-18h3dkqc/result.json`。新增仓库原生 test 后定向测试 O0/O3 通过，`/tmp/luna-tools-i942iver/result.json`；最终格式化退出 0，`/tmp/luna-tools-9_u5dlql/result.json`。未运行整个 workspace，dev 未提交。

## 显式多包 LLVM 导出

`--packages-v1 --emit-llvm <entry>` 已接入 host pipeline。入口按 primary 的语义身份选择，允许依赖包拥有同名函数；资源协议按同一编译结果解析。`--inline-functions` 仍要求 Copy/Drop 配对绑定，并在源码副本上展开后重新准备清理计划。显式 `--opt-level` 使用现有 LLVM 验证／优化入口；未指定时只生成 LLVM 文本。

无 `--with-inputs` 时输出原始 LLVM；启用时结果是 `(llvm-ir (string "..."))`，沿用 IR 字符串转义后放入 compilation-result，输入记录来自同次编译。所有包源码继续受输出别名保护。多包直接 `--build` 尚未接线，可先导出 LLVM；单输入 `--build` 与多包 `--test-native` 已可用。

本轮工作目录 `/home/fin/luna`，定向测试 `package_llvm_dump_selects_primary_identity_and_preserves_inputs` 验证两包同名入口、跨包调用、可选内联、O0/O3 实际原生输出、输入封装及源码覆盖拒绝，退出 0，报告 `/tmp/luna-tools-w4986cne/result.json`。相关回归命令 `python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --test compilation_cache --lib --bin luna` 退出 0，报告 `/tmp/luna-tools-5l6zmii_/result.json`。格式化退出 0，报告 `/tmp/luna-tools-p5t_m2su/result.json`。dev 工作区未提交，未运行全 workspace 测试。

## 显式多包原生构建

续接上一节的限制：`--packages-v1 --build <entry> --output <path>` 现可直接生成本机可执行文件。与多包 LLVM 导出共用 primary 语义身份选择、资源协议绑定与可选内联；优化和链接完成后才通过既有原子输出入口发布，并设置可执行权限。Rust 调用方使用 `Request::build` 取得二进制字节，文本 API `Request::run` 对 build 明确拒绝。二进制 build 不接受 `--with-inputs`，需要输入记录的文本交付仍使用 `--emit-llvm --with-inputs`。resident stream 继续仅接收前端编译。

回归 `package_native_build_preserves_identity_resources_and_previous_artifacts` 实际执行两个包的同名入口，覆盖跨包 inline effect 与析构观察、O0/O3；验证 LLVM/链接器缺失、fuel 耗尽时旧产物不变，失败后恢复成功，依赖源码及其硬链接不可覆盖，缺失输出和二进制输入封装组合属于用法错误。

工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --test compilation_cache --lib --bin luna` 退出 0，报告 `/tmp/luna-tools-azaax75f/result.json`。全 workspace 格式化退出 0，报告 `/tmp/luna-tools-0tzuxnbu/result.json`；未运行全 workspace 测试。改动保留 dev 工作区，未提交。

## 不返回函数的机器返回通道

host 现支持返回 `.noreturn'builtin` 的普通函数、函数值和 lambda，以及不返回的入口。Never 仍没有值表示；仅机器返回通道使用 LLVM void。调用后先处理 checked failure 标志，再发出 unreachable，因此不会把需要传播与清理的失败误当成 LLVM 未定义行为。checked 函数可能经内部失败通道返回，不能机械标为 LLVM noreturn。不返回入口不生成结果打印路径；声称 Never 却有正常完成路径的函数会被降码拒绝。

源级默认参数仍要求 P3 稳定性／终止证据，调用发散函数会在 elaboration 被拒绝；本轮没有放宽该规则。Never 也未被映射为可存储的 Unit，不改变参数、字段或类型身份。

`native_never_return_channels_preserve_control_flow` 验证直接调用、函数值、lambda 的可达分支，以及普通调用与入口的 checked assumes 失败，均在 O0/O3 生成并执行本机程序。

最终状态在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`，退出 0：host 291、lib 26 项通过，报告 `/tmp/luna-tools-dtb55zjg/result.json`。格式化退出 0，报告 `/tmp/luna-tools-awa57g_w/result.json`。未运行全 workspace，dev 未提交。

## 入口聚合结果展示

开发用 main 适配器现可输出结构 tuple 和匿名 record，递归叶子支持现有标量展示域（void、bool、字符数值、f32/f64 位表示、已接纳宽度的整数／BitVec）。例如 `(tuple (bool true) (unit))`；record 保留字段名，当前输出 `(anonymous-record (fields (field (string "ready") (bool true)) ...))`。字段名经过 S-expression 转义、printf 百分号保护及 LLVM 字节转义。Tuple 的分类来自受信 normal-form 结构视图，不展开 Type 函数的头；物理布局相同的 nominal 类型不能据此混同为 tuple。

入口只调用一次，checked failure 在读取与打印聚合字段前处理。叶子包含指针、函数值、nominal 拥有值、错误行或其他未支持表示时明确拒绝；尤其不会丢弃返回 nominal 值的析构责任，一般拥有型结果仍需要入口清理适配器。这是开发诊断展示，不是 println、通用序列化或最终语言 ABI。

可运行例子：

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-product-result.fl --build run --meta-list-type input.Elements --tuple-type input.Tuple --output /tmp/flurry-product-result
python3 -B development/command.py --timeout 10 -- /tmp/flurry-product-result
```

测试直接引用该文件，在 O0/O3 验证嵌套 record/tuple、u128 最大值、浮点位与 unit；另验证拥有型返回值拒绝时保留旧产物，以及 checked failure 不打印部分成功结果。

最终在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib`：退出 0，host 294、lib 26，共 320 项通过，报告 `/tmp/luna-tools-5c9n9zbi/result.json`。格式化退出 0，报告 `/tmp/luna-tools-p_0k9zso/result.json`。未运行全 workspace，改动保留 dev，未提交。

## 显式丢弃拥有型入口结果

`--build run --discard-result --copy-protocol Duplicate --drop-protocol Retire --output /tmp/program` 可以执行返回拥有型值的入口；`--emit-llvm` 及 `--packages-v1` 同样支持此选项。它要求配对资源协议，不与解释执行／原生测试等其他 action 混用。成功时开发适配器输出 `(unit)`；它不展示被丢弃的拥有型值，也不改变普通入口打印器的行为。

实现先复用普通零实参调用计划（包含默认参数），再在源码副本上创建 NIR 包装函数。包装函数用显式 Discard 节点接收调用结果，然后进入既有资源验证、清理计划和 LLVM 降码。返回对象的析构由已验证的计划取得；没有后端硬编码的析构调用、隐式 Copy 或用户源码改写。可选内联先在副本上完成，包装函数随后重新验证。Rust API 为 `host::llvm_ir_discarding_result`，包身份可显式选择。

O0/O3 回归验证默认实参入口返回带析构的 Watch，成功清理与析构谓词失败（失败不输出成功结果）；多包回归验证同名入口按 primary 身份选取、跨包可选内联、Drop 绑定与 observed LLVM 输出。缺少协议、错误 action、重复选项均返回用法错误并保留旧产物。该入口仍受现有资源验证和 LLVM 清理能力约束，不意味着所有拥有型表示、效应 ABI 或析构行为均已实现。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --test compilation_cache --lib --bin luna` 退出 0，报告 `/tmp/luna-tools-8q1jm36u/result.json`。全 workspace 格式化退出 0，报告 `/tmp/luna-tools-71ln1vf9/result.json`；未运行全 workspace 测试，dev 未提交。

## 内建 str 的字面量存储与运行时传递

host 已增加内建 `str` 的私有表示 `{ptr, i64 length}`。字符串字面量放入不可变 UTF-8 全局存储；局部绑定、分支、函数参数与返回沿普通值降码传递地址和字节长度。字面量 pattern 使用长度和字节内容比较，不依赖地址相同，也不把 NUL 当成终点。空字符串不会解引用存储地址。此处不提供可变字符串、分配器、拼接或通用 String 库接口。

开发入口可输出 `(string "...")`，逐字节保留 UTF-8，转义引号、反斜杠、换行／回车／tab、ASCII 和 C1 控制字符，支持嵌入 NUL。该输出依然是开发结果适配器，不是语言 println。结构 tuple/record 入口也支持 str 叶子，共用字符串转义逻辑。示例 `development/examples/host-string.fl` 可用 `--build run --output /tmp/flurry-string` 编译运行。

资源验证仍拒绝 str 的未建模 backing-region provenance；即使声明 Copy，也不放行 checked 路径。本轮回归验证这条拒绝。后端能力不能代替借用字符存储的内存契约，也没有据此改变 library 中未完成的格式化／I/O 依赖。

O0/O3 验证空串、UTF-8、转义、嵌入 NUL、DEL/C1 控制字符输出及字符串模式的内容／长度比较；仓库示例也进入原生执行回归。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib` 退出 0，host 299、lib 26，共 325 项通过，报告 `/tmp/luna-tools-qzmcfyvz/result.json`。格式化退出 0，报告 `/tmp/luna-tools-wgn84s7j/result.json`。未运行全 workspace，dev 改动未提交。

## 聚合中的字符串结果

结构 tuple/record 的 str 叶子已接通。打印器递归发出按顺序执行的格式片段，字符串叶子调用与独立字符串入口共用的转义例程；没有为字符串长度建立动态栈缓冲。原生入口仍先调用一次，完整取得结果、检查失败后才开始读取和输出字段。字段名保留 S-expression 转义与 printf 百分号保护，运行时字符串不会被当成 format。最外层统一输出一次换行，嵌套字符串不会额外换行。

`development/examples/host-product-string-result.fl` 覆盖带副作用的字段求值、嵌套 tuple/record、空串、UTF-8、百分号、NUL、换行、引号与反斜杠，测试在 O0/O3 实际编译运行。str backing-region 的 checked 限制和 nominal 拥有值的入口清理限制保持不变。

最终在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib` 退出 0，host 300、lib 26，共 326 项通过，报告 `/tmp/luna-tools-f455qo5r/result.json`。格式化退出 0，报告 `/tmp/luna-tools-2juwblyd/result.json`。未运行全 workspace，dev 未提交。

## 宽 BitVec 的入口结果输出

入口与结构聚合打印器现可输出 host 宽度预算内的 BitVec 十进制值，补齐 65–127 位及大于 128 位的显示缺口。已有固定宽度整数行为保持不变。转换先按数值位位置提取 32 位分组，再用 64 位除法计算十进制数字，不依赖目标端序，也不要求 LLVM 支持任意宽度的除法运行库。全零值至少输出一个数字，最高分组允许不足 32 位；缓冲大小由位宽保守上界加符号／终止符计算。生成的分组数量计入 host 编译 fuel。

O0/O3 原生验证 65、127、129、256、1024 位全一值，以及聚合中的零、42、2^64 和 2^65+1；参考结果用独立的十进制逐位倍增算法生成，未把后端输出作为预期值。一般表示预算仍为现有的 65536 位；本轮原生测试最大宽度为 1024 位，没有声称穷举所有位宽或改变算术安全契约。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --lib` 退出 0，host 302、lib 26，共 328 项通过，报告 `/tmp/luna-tools-r58cnnw2/result.json`。格式化退出 0，报告 `/tmp/luna-tools-qvqrj0te/result.json`。未运行全 workspace，dev 未提交。

## Intrinsic 源结构接线（尚未执行）

正式 intrinsic 语法此前在 AST lowering 就被通用 unsupported view 拒绝。本轮增加专用 `TakeIntrinsic` HIR 节点及 S-expression Display；标签表达式、application 实参、显式结果 cast 保持各自结构边界。模板克隆与相关遍历识别新节点。NIR 仍明确诊断目标协议与 unsafe admission 未实现，不把标签伪装成普通函数，也不按拼写映射 LLVM intrinsic。后续依赖已列入 frontend-checkpoint。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --test test_templates` 退出 0，报告 `/tmp/luna-tools-xfut9uiw/result.json`。`python3 -B development/check.py --timeout 300 -- test -p hir --test display` 最终退出 0，报告 `/tmp/luna-tools-xy831sz3/result.json`；新增节点的 dump 通过 S-expression 解析检查。格式化退出 0，报告 `/tmp/luna-tools-k9lkrbvr/result.json`。未运行全 workspace，dev 未提交。

## Intrinsic 前置的 unsafe 边界保存

本轮将 unsafe block 保留为专用 HIR 节点，并在函数修饰信息中保存 `is_unsafe`。此前块在 HIR 中与普通 Block 无法区分，unsafe 函数修饰也只能经待实现属性记录识别。现在 dump 和模板克隆都保留其准确边界；普通名字解析仍检查块内部，不因 unsafe 接受不存在的实参。

执行层没有放宽：unsafe 的调用责任、审计传播和目标行为协议尚未实现，pending semantics 与 elaboration 拒绝继续生效。该改动为后续 intrinsic admission 提供结构输入，不宣称 LLVM intrinsic 已可执行。

在 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p hir --test display` 退出 0，13 项通过，报告 `/tmp/luna-tools-dsyuobbz/result.json`；`python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --test test_templates` 退出 0，frontend_cli 180、test_templates 73 项通过，报告 `/tmp/luna-tools-7ltk4oxa/result.json`。格式化退出 0，报告 `/tmp/luna-tools-c586h2n_/result.json`。未运行全 workspace，dev 未提交。

## 普通 unsafe block 的原生执行

普通 unsafe block 现沿 HIR → NIR → IIR 保留审计节点；NIR/IIR dump 为 `(unsafe-scope ...)`，验证器检查结果类型与主体一致。LLVM 在验证完成后执行主体，不为该节点添加特殊优化假设。内部的阶段、纯度、程序规格、资源与算术检查继续生效。函数内联、尾部 return 简化和 effect/resume 展开保留该边界，内存分析穿透它传递指针与 callable 来源。

回归覆盖嵌套块、普通／泛型／pure 函数、解释器与原生结果一致、NIR/IIR 审计 dump、O0/O3 的拥有值清理与向外转移、指针与函数值交付、可选内联及 inline effect/resume。拒绝例覆盖溢出、局部指针逃逸、pure 函数中的外部读写，以及人为伪造 wrapper 结果类型；编译失败保留旧产物。unsafe 函数 admission 与 intrinsic 目标协议尚未实现，仍明确拒绝；这一阶段不声称支持任意 unsafe 操作。

在 `/home/fin/luna` 执行 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --test runtime_purity --test inline_returns --test effect_memory_transfers`，638 项通过，退出 0，报告 `/tmp/luna-tools-0hxgs7l8/result.json`。`test -p nir -p iir` 经同一入口退出 0，报告 `/tmp/luna-tools-7rjnd33j/result.json`。补充 unsafe 块内 pure 外部读写拒绝后，定向 `runtime_purity` 测试退出 0，报告 `/tmp/luna-tools-mwcfovul/result.json`。全 workspace 格式化退出 0，报告 `/tmp/luna-tools-xm3m46ey/result.json`。未运行全 workspace 测试，dev 改动保留工作区，未提交。

## Runtime unsafe 函数调用

普通 runtime `unsafe fn` 已接通源级签名、函数值、NIR/IIR 校验与 LLVM 调用。声明保存独立 `is_unsafe`，值类型保留 `UnsafeFunction`；函数 ABI 继续使用已检查的参数／结果 telescope。仅取得、传递或返回函数值不执行调用，不需要调用权限。调用必须位于词法 unsafe block 或 unsafe 函数的责任范围内；另一个普通函数体和它的默认实参不能借用调用位置的 unsafe 权限。块内形成的闭包保留所在代码的词法检查，单纯把函数值的取得放在 unsafe 块内不会授权块外调用。

NIR 和 IIR 都检查调用边界，直接函数／方法引用还核对声明责任与值类型，防止伪造普通 fn 类型绕过检查。开发入口适配器拒绝直接选择 unsafe 函数为根入口，要求作者写显式 safe wrapper。类型、阶段、程序契约、算术、纯度与资源检查继续生效；这里没有因此接通未实现的 intrinsic 或任意裸内存操作。unsafe comptime／quote 定义仍明确拒绝，避免在 required invocation／代码展开之前未经检查地执行。

示例 `development/examples/host-unsafe-call.fl`：

```sh
python3 -B development/check.py --timeout 300 -- build -p luna
python3 -B development/command.py --timeout 30 -- /tmp/luna-dev-002/target/debug/luna development/examples/host-unsafe-call.fl --build run --output /tmp/flurry-unsafe-call
python3 -B development/command.py --timeout 10 -- /tmp/flurry-unsafe-call
```

输出 `(bool true)`。原生回归覆盖直接／间接调用、unsafe 函数间调用、泛型、trait 方法、默认实参、可选内联、闭包及拥有参数的清理，关键路径在 O0/O3 执行。拒绝例覆盖块外调用、相邻作用域、函数值取得与调用分离、普通 helper／default 借用调用方权限、unsafe 根入口、伪造引用责任及 pure unsafe 函数中的外部读写；失败保留旧产物。

跨阶段 runtime callback 的 Splice 此前有独立冻结缺口（已由下一节修复）：`development/repros/host-unsafe-staged-callback.fl` 使用显式绑定的 Splice 协议，报 `staged argument cannot be frozen: Unsupported`。将同一例子的 unsafe 修饰移除也得到相同错误；回归保留这条已确认限制，不把它归因于 unsafe 调用权限，也不声称已交付该路径。

本轮验证工作目录 `/home/fin/luna`，均经 `python3 -B development/check.py --timeout 300 -- ...` 执行：

- `test -p luna --test host --test frontend_cli --test function_type_dependencies --test runtime_purity --test test_templates`：668 项通过，退出 0，`/tmp/luna-tools-_z0p8kz3/result.json`。补充拥有参数析构与 pure unsafe 拒绝后，`test -p luna --test host --test runtime_purity`：314 + 63 项通过，退出 0，`/tmp/luna-tools-bu5f39ih/result.json`。
- `test -p nir -p iir`：434 项通过，退出 0，`/tmp/luna-tools-6_sy4ta4/result.json`。最初新增安全遍历覆盖了 unresolved pattern-code 的原诊断；调整检查顺序后恢复原诊断并通过该回归。
- `test -p elaboration --test verification --test implementation --test matching --test runtime_identities`：171 项通过，退出 0，`/tmp/luna-tools-r2zbto_b/result.json`；`test -p elaboration --lib verification::`：73 项通过，退出 0，`/tmp/luna-tools-_hx9uf1d/result.json`。
- `test -p comptime_interpretor --test runtime_function_identities --test function_types`：19 项通过，退出 0，`/tmp/luna-tools-mkeknpna/result.json`。

全 workspace 格式检查退出 0，`/tmp/luna-tools-u3k8aybj/result.json`。未运行全 workspace 测试；dev 改动保留工作区，未提交。

## Runtime callback 经 comptime 与 Splice 交付

此前 `host-unsafe-staged-callback.fl` 的冻结失败已修复。Invocation-only 冻结现在可以保留具名 runtime 函数的声明身份与完整实例实参，沿 `FunctionValue` 传递；它不序列化机器地址，也不展开函数体。该值的执行阶段与 unsafe 责任由实际声明、登记的 runtime identity 和实参域共同校验，重建时不得伪造另一个阶段或责任。IIR staged 常量识别同样保留具名 runtime 引用，泛型实例实参继续参与身份。

此能力不把 runtime 函数变成 comptime 可调用，也不允许 Splice 把 unsafe 类型削弱为普通 fn。捕获环境、匿名闭包和未完成实例输入仍不能通过这条具名身份路径冻结；普通数据导出仍拒绝函数身份。冻结与重建都有预算、作用域和类型检查，失败后可继续处理合法输入。

可运行示例为 `development/examples/host-staged-unsafe-callback.fl`，使用 `--splice-protocol Splice --build run --output /tmp/flurry-staged-unsafe`，输出 `(bool true)`。原 `development/repros/host-unsafe-staged-callback.fl` 保留作已解决的回归来源，注释已更新。安全／unsafe callback 都在 O0/O3 下执行，并分别验证未启用资源协议与 paired Copy/Drop 路径；同一函数的 `<true>`／`<false>` 实例不会混淆。拒绝与恢复测试覆盖未知声明、错误实例实参、阶段／责任伪造、越阶段调用、Splice 擦除责任、预算不足和未完成输入。

工作目录 `/home/fin/luna`，通过 `python3 -B development/check.py --timeout 300 -- ...` 执行：`test -p luna --test host --test open_functions --test function_type_dependencies --test frontend_cli` 共 922 项通过（host 316），报告 `/tmp/luna-tools-s9rcw0wz/result.json`；`test -p comptime_interpretor --lib --test runtime_function_identities --test function_types --test type_equality` 99 项通过，报告 `/tmp/luna-tools-sknrg8ii/result.json`；`test -p iir` 70 项通过，报告 `/tmp/luna-tools-2qd24w19/result.json`。均退出 0。旧拒绝测试在修复后因编译成功而失败，随后转换为上述原生执行验收。未运行全 workspace 测试，dev 未提交。

全 workspace 格式检查退出 0，报告 `/tmp/luna-tools-umk1uuhx/result.json`。

## Intrinsic 标签族声明

`^intrinsic_enum enum` 已能进入 HIR/NIR，枚举定义及 dump 保存 `(intrinsic true)`；普通 enum 为 false。标记仅声明操作标签族，尚未给标签绑定 LLVM 行为。不同枚举中的同名 variant 保持不同声明身份；未知叠加属性仍保留待实现诊断，标记用于非 enum 则报错。现有 intrinsic 调用继续拒绝，不能凭返回类型或标签拼写推导指令语义。

本次定向验证工作目录 `/home/fin/luna`，使用 `python3 -B development/check.py --timeout 300 -- ...`：`check -p luna` 退出 0（`/tmp/luna-tools-0uwdwvdk/result.json`）；`test -p luna --test frontend_cli intrinsic_enum_attribute` 两项通过（`/tmp/luna-tools-hhn8obhu/result.json`）；`test -p hir --test display` 13 项通过（`/tmp/luna-tools-114tazmw/result.json`）；`test -p nir --test nominal_parameters --test completed_values --test scoped_contracts` 38 项通过（`/tmp/luna-tools-1cpcva6n/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-921kwqe8/result.json`）。

随后 `test -p luna --test frontend_cli --test host` 退出 0，分别 183、316 项通过，报告 `/tmp/luna-tools-71clkqj1/result.json`；host 包含实际 LLVM 产物执行。格式检查 `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check` 退出 0，报告 `/tmp/luna-tools-dzwyr_r3/result.json`。未运行全 workspace 测试；dev 改动保留工作区，未提交。

## Host intrinsic 目标协议入口

`host::intrinsic::Registry` 接受显式的 `(枚举声明身份, variant 索引) → Operation` 绑定，不按标签拼写识别 LLVM 操作。注册时检查 intrinsic 标记、唯一声明、合法变体和无载荷 unit 标签；当前不接受带参数或约束的标签族，避免丢失实例身份及 admission 条件。重复、未知或非法绑定使整个构造失败，不发布部分注册表。

首个协议为 FusedMultiplyAdd：仅接受三个与显式输出同类型的 f32 或 f64 输入，void/整数输出、错误数量、混合精度均拒绝。成功结果字段私有，Display 为 S-expression。此检查只证明该目标操作的类型接口，不授权 unsafe、comptime 执行或资源访问。模块尚未接入源码调用/IIR/LLVM 发射；源级 intrinsic 调用继续明确拒绝。下一步应让专用操作节点保存标签、源序实参、输出与来源，再接入词法责任及各类行为分析，不能伪装成普通函数调用跳过这些检查。

验证工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol` 退出 0，三项测试覆盖真实源声明身份、两种精度、无输出/非法输入、注册失败与恢复，报告 `/tmp/luna-tools-yzevqm7s/result.json`。格式化退出 0（`/tmp/luna-tools-id91dble/result.json`）。未重复全 workspace 或原生执行测试；本轮尚无新增 LLVM 发射行为。dev 工作区未提交。

## Intrinsic 专用 NIR 节点及分析边界

新增 `ExprKind::Intrinsic { tag, inputs }`，外围 Expr 保存输出类型与来源。Tag 移到 `nir::intrinsics`，host 协议复用同一声明身份。S-expression 明确展示标签及输入列表；只读/可变遍历、类型替换、复制预算和 required 依赖扫描均进入所有输入，替换不重排源序输入。NIR phase validation 检查标签族、变体及 runtime 阶段，call_safety 要求词法 unsafe 边界。当前未实现泛型/带约束标签实例，明确拒绝而不丢弃实例条件。

未知目标行为不会凭输出类型获得优化或执行资格：默认实参稳定性和纯度分析不接纳该节点；程序、内存与所有权验证独立拒绝缺失行为协议；解释器明确报 Unsupported。IIR materialization 与独立 artifact validation 暂时拒绝目标行为未接入的节点。源码仍保持原有拒绝，尚未接通调用 elaboration 和 LLVM 发射；这一阶段交付的是结构保存与责任边界，不能声称 fma 已可原生执行。

工作目录 `/home/fin/luna`，经 `python3 -B development/check.py --timeout 300 -- ...` 验证：`test -p luna --test host_intrinsic_protocol --test frontend_cli --test runtime_purity` 共 252 项通过（`/tmp/luna-tools-82m3ko77/result.json`）；`test -p nir -p iir` 434 项通过（`/tmp/luna-tools-_r607yrv/result.json`）；补充内存、所有权及解释器拒绝验收后，`test -p luna --test host_intrinsic_protocol` 六项通过（`/tmp/luna-tools-jbnqx6p3/result.json`）。均退出 0。最初 IIR 拒绝测试误选 unsafe 函数作为根入口，被更早的入口责任检查拒绝；改用合法 safe wrapper 后验证目标行为检查。格式化退出 0（`/tmp/luna-tools-ztr5wdqd/result.json`）。未运行全 workspace 测试，dev 未提交。

## 源码 intrinsic 到 NIR

源码的直接 `tag'intrinsic(args)` 及紧随调用的 `.as(T)` 现生成专用 NIR 节点。普通分组括号保持直接关系；变量读取或 `do` 块外的后续 as 走普通转换，不回填 intrinsic 输出。没有直接 as 时输出为 void，外围变量/函数类型不能代为指定。标签解析沿实际枚举声明和变体索引，要求 intrinsic_enum、无载荷 unit 标签；当前有实例参数/约束的标签族仍明确拒绝。

实参按源序逐一进行普通表达式 elaboration，既不以输出类型推导操作数，也不制造虚构函数签名。当前输入限位置实参，命名、展开、隐式及其他调用形态尚待目标输入协议。常规 where T 泛型保留抽象类型身份。NIR 发布检查 runtime 阶段与词法 unsafe，pure 函数不能凭结果类型获得目标行为保证。`--emit nir` 可以检查成形节点，`--build` 仍在 IIR 目标行为 admission 处拒绝；尚未连接 LLVM 发射。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --test host_intrinsic_protocol --test open_functions --test runtime_purity` 退出 0，185 + 8 + 389 + 63 共 645 项通过（`/tmp/luna-tools-w2enijom/result.json`）。测试覆盖源序参数身份、void 与 f32 输出、普通泛型、括号、外围/延迟转换拒绝、实参错误、阶段/责任、标记/载荷检查及源码形成结果送入 host 注册表。首次定向检查中旧“全部拒绝”测试因 NIR 成功而失败，已改为验证 NIR 成功和原生产物拒绝。全 workspace 格式化退出 0（`/tmp/luna-tools-x2lxg2tz/result.json`）。未运行全 workspace 测试，dev 未提交。

## 已绑定融合乘加的原生执行

`nir::intrinsics` 现统一提供标签、注册表及签名检查结果，`host::intrinsic::bind(package, registry, fuel)` 在独占的 package 事务中写入节点 binding。检查结果保存实际 tag、操作种类和浮点宽度；类型改写后，NIR、IIR、资源层与后端都重新核对实参/结果与证据一致。绑定入口计入函数复制/类型预算、遍历预算和标签查找成本；失败不返回部分 package。没有注册的标签保留未绑定状态，只有实际需要实例化它时才拒绝；重新绑定不会保留旧注册表的证据。

当前可执行协议只有三个同精度 f32/f64 输入的 FusedMultiplyAdd。输入表达式仍按源序执行一次，各自内存/所有权/程序效应照常检查；目标操作自身不产生指针、拥有载荷或内存访问。Materialization 保留标签族声明，IIR 独立检查其 intrinsic 标记。LLVM 发出 `llvm.fma.f32/f64`，不加 fast-math；必要的 libm 调用由 host 链接器 `-lm` 接线。该目标运算按 [LLVM fma 协议](https://llvm.org/docs/LangRef.html#llvm-fma-intrinsic) 执行一次融合舍入，不把它拆为独立乘加，也不额外规范化其 NaN 载荷。

已通过真实源程序在 O0/O3 下生成、链接及执行：f32/f64 的 3×4+5；可以区分融合/分步舍入的 (1+epsilon)×(1-epsilon)-1；带程序谓词与可变局部状态的三个实参恰好求值一次并保持源序；配对 Copy/Drop 准备后的执行。篡改输入精度、tag 或 IIR 标签族标记被拒绝；未选择的未知操作不阻止合法入口，实际选择未知操作仍拒绝。

该能力目前通过编译器 API 接入，CLI 尚未提供目标操作绑定选择。解释器仍不执行 target intrinsic；泛型输出尚不能在绑定阶段生成具体宽度证据，源码 pure 检查也尚未获得提前绑定的目标行为。下一步需接入驱动选择和实例化时的协议特化，再交付 library math 的泛型包装，不能把本轮称为完整 intrinsic 系统。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --test host_intrinsic_protocol --test runtime_purity` 最终退出 0，185 + 316 + 12 + 63 共 576 项通过（`/tmp/luna-tools-04wyzxyg/result.json`）；`test -p nir -p iir` 434 项通过（`/tmp/luna-tools-_2xqlq7f/result.json`）。第一次集成检查有一项旧 CLI 原生验收在启动前遭遇 ETXTBSY（`/tmp/luna-tools-anvcyl2q/result.json`），已将测试启动统一到有超时和最多 100ms 的 exec 前重试入口；仅重试 ETXTBSY，不重试已启动的程序。未运行全 workspace 测试，dev 未提交。

## 命令行目标绑定及 pure 包装

新增 `--host-fma package.module.Enum.variant`（module 可省略），当前用于 `--build` / `--emit-llvm`，一次命令只接纳一个 FMA 绑定。单文件输入的包名是 input；显式包编译使用 `--package` 指定的包名。选择器沿现有 HIR 模块路径解析，转换为驱动提供的语义包身份与声明身份，具体变体在 NIR 中定位，不借同名变体推断操作。

绑定在 elaboration 前传入 Protocols，并在 intrinsic 节点形成时生成签名证据，因此具体 f32/f64 的 pure 包装现在可以完成纯度检查；这个变化取代前节中“source pure 尚未提前绑定”的限制。绑定仍不授权越阶段执行、unsafe 边界外调用或 pure 函数读取外部指针。无效包、模块、enum、非 intrinsic_enum、非 unit 变体和与驱动身份冲突的选择均拒绝。包请求的 Action 保存选择器，原有完整请求缓存键也包含它；sysroot 默认配置保留显式选择，不创建隐式标签绑定。

示例 `development/examples/host-intrinsic-fma.fl` 可直接运行：

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-intrinsic-fma.fl --host-fma input.machine.Scalar.fused --build run --output /tmp/flurry-fma
python3 -B development/command.py --timeout 10 -- /tmp/flurry-fma
```

输出 `(float32 (bits 41880000))`，即 17。用 `--emit-llvm run` 替代 build 可检查 `llvm.fma.f32` 调用。原生验收覆盖 O0/O3、嵌套模块、多包身份、paired Copy/Drop、失败保留旧产物。同名标签所在的另一个包不会获得授权。泛型 math 包装的抽象浮点类型特化、多个目标操作配置及 native-test 绑定选项仍待实现；不把一个具体 FMA 协议称为完整 intrinsic 系统。

工作目录 `/home/fin/luna`，经 `python3 -B development/check.py --timeout 300 -- ...`：`check -p luna --tests` 退出 0（`/tmp/luna-tools-lrff8cig/result.json`，仅编译测试目标）；`test -p luna --test frontend_cli --test host_intrinsic_protocol --test runtime_purity --lib` 退出 0，26 + 189 + 12 + 63 共 290 项通过（`/tmp/luna-tools-69u1ugqf/result.json`）。未运行全 workspace 测试，dev 未提交。

补充检查：`test -p luna --test compilation_cache --test library_protocols` 经相同入口退出 0，5 + 8 项通过（`/tmp/luna-tools-qc0wfzli/result.json`）；强化拒绝诊断断言后，`test -p luna --test frontend_cli host_fma_binding_preserves` 退出 0（`/tmp/luna-tools-qbx5zoz0/result.json`），明确验证 unsafe、comptime 与 P4 诊断，排除仅因入口参数不匹配而失败的假阳性。最终格式化退出 0（`/tmp/luna-tools-dx1lixhq/result.json`）。

## 首个 library math 泛型包装

`library/math/lib.fl` 提供 pure 泛型 `fma`，通过 `FusedMultiplyAdd` trait 静态选择 f32/f64 的具体 LLVM 包装，并用显式类型命题限定公开函数的两个合法输入类型。实际库源码既以声明验收方式编译，也作为独立 math 包由 CLI 编译、链接、执行；不是用 fixture 替代库实现。`development/examples/host-math-fma.fl` 同时展示类型推导和显式类型实参，运行命令见 library/README.md。O0/O3 均输出 bool true，原始 LLVM 同时含两种宽度的 fma 调用；paired Copy/Drop 路径也通过。非浮点类型即使拥有合法的同包协议实现，仍无法绕过类型域要求；外包为库 trait 与 builtin bool 增加实现还受原有 impl ownership 规则拒绝。混合精度和失败保留旧产物均有验收。

资源验收发现并修复了 pure 泛型的有限标量类型域缺口：以前在绑定 Copy/Drop 时，类型变量仍以完全未知的拥有表示检查，丢失了 `T == f32 or T == f64` 的有限域信息。新增独立 type_domains 检查，用 PTC 只证明声明类型前提覆盖候选标量类型，然后逐个实例调用已有程序/内存/清理纯度检查。程序前后置谓词不进入 PTC。未知覆盖、预算耗尽、非有限类型域都回退到原有保守检查；不从已有 impl 列表推断类型域。跨参数组合最多 64 个，可能包括不可行组合，逐一检查保持保守性。不会展开 normal-form 头，也不把某个分支的行为证明用于其他类型实例。

这交付了库级泛型 fma；没有把任意抽象 T 的 raw intrinsic 节点直接赋予位宽证据。一般 intrinsic 泛型输出、其他 math 操作和解释器 target 操作仍待实现；完整 sysroot 的既有依赖阻塞仍然存在。math 暂通过显式包入口接入，未作为未配置的 pure target 库自动加载到所有 sysroot 编译中。

本轮工作目录 `/home/fin/luna`，经 `python3 -B development/check.py --timeout 300 -- ...`：`test -p luna --test runtime_purity --test trait_purity --test frontend_cli --test host_intrinsic_protocol --test library_protocols` 退出 0，190 + 14 + 8 + 64 + 20 共 296 项通过（`/tmp/luna-tools-3rvqyehl/result.json`）；`test -p elaboration --lib type_domains` 一项通过（`/tmp/luna-tools-zqzf6qek/result.json`），覆盖预算恢复、缺少穷尽前提以及 opaque 头不能充当具体 float 的拒绝。此前 paired resources 的 P4 失败记录在 `/tmp/luna-tools-05ofcd57/result.json`，修复后相同 CLI 验收通过。未运行全 workspace 测试，dev 未提交。

补充验证：增加 f32 分支存在外部写入时同样拒绝的断言后，`test -p luna --test runtime_purity finite_scalar_type_domains` 退出 0（`/tmp/luna-tools-xefaobj8/result.json`）；最终 `cargo fmt --all` 经 development/command.py 执行退出 0（`/tmp/luna-tools-plkv057d/result.json`）。

## Native source tests 的 host intrinsic 接线

`--test-native --host-fma package.module.Enum.variant` 现已贯通单文件和 `--packages-v1`，在 elaboration 前绑定身份，使 pure math 包装和后续 IIR/LLVM 使用同一份验证证据。支持 O0/O3、筛选和 paired Copy/Drop；无效 variant 拒绝，测试失败仍以非零退出码返回。`--test`、`--list-tests` 与普通 NIR dump 不接受这个目标执行选项，解释器能力没有因此扩大。新增可直接执行的 `development/examples/host-math-tests.fl`，命令见 library/README.md。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --lib` 退出 0，26 + 191 共 217 项通过，报告 `/tmp/luna-tools-i_hvg3td/result.json`。文档中的实际 math native-test 命令退出 0，两项测试通过，报告 `/tmp/luna-tools-fdlozke3/result.json`。格式化退出 0，报告 `/tmp/luna-tools-4vu_36on/result.json`；`git diff --check` 通过。未运行全 workspace 测试，dev 未提交。

## 直接泛型 FMA 的待特化绑定

`library/math/lib.fl` 现用一个泛型函数体直接表达 `target.Scalar.fma'intrinsic(a,b,c).as(T)`，不再需要先前的 FusedMultiplyAdd 桥接 trait 和两份具体实现。声明要求 `T == f32 or T == f64`；source elaboration 以现有类型命题检查器证明这个域，同时要求三个输入与输出具有同一类型。未绑定操作仍保持原来的不透明、不可执行行为。

NIR target binding 区分具体位宽与 pending-type。pending 只接受 Type-sort rigid 参数，不是可执行证据；普通类型替换同时处理这份待特化签名，只在 f32/f64 后变为具体位宽。已完成的位宽证据不随任意类型重写而变化。模板阶段检查签名一致性，materialization、资源验证和 LLVM 仍要求完整具体证据；bool 等非法替换和输入/输出不一致都会拒绝。Display 包含 `(pending-type ...)` 或 `(float-width ...)`，normal-form 头不展开。

pure 泛型的有限类型域行为检查现在也适用于没有 Drop 绑定的场景，逐个具体类型检查实际行为，PTC 仍只负责类型域覆盖。不允许缺少域约束的纯泛型借 pending 证据通过行为检查。公开 Registry 的具体 `check` 以及后置 host::intrinsic::bind 仍要求具体输入；本轮贯通的是 source selector 驱动的泛型模板与实例化路径。其他 math 操作、泛型标签族与非标量签名仍未完成。

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol --test frontend_cli --test runtime_purity --test trait_purity --test library_protocols` 退出 0，191 + 15 + 8 + 64 + 20 共 298 项通过（`/tmp/luna-tools-zr3xkdw6/result.json`），包含真实 math 包 O0/O3 build、native test、paired Copy/Drop。`test -p nir -p iir` 退出 0，434 项通过（`/tmp/luna-tools-86lzrveb/result.json`）。补加 opaque-head 拒绝例后 `test -p luna --test host_intrinsic_protocol generic_fma_specializes` 退出 0（`/tmp/luna-tools-x0ryzn5r/result.json`）；用例先前因无 sysroot 的环境中未声明 Type 而失败，已改用 `.Type'builtin` 并明确检查 type requirement 诊断。未运行全 workspace 测试，dev 未提交。

## 真实 library 资源协议与 native 验收

Once/Clone/Copy/Drop 从 builtin/traits.fl 拆到独立的 builtin/resources.fl，traits 保留 re-export，现有 builtin/prelude 和 qualified trait API 继续有效。新增 bool、char、void、固定宽度整数、usize/isize、f32/f64 及普通指针的声明与 Clone 实现；固定数值/void/普通指针实际通过资源检查与执行。usize/isize 只完成接口与指针操作形成，host 数值表示仍未接通，不以未实例化的声明通过冒充平台整数执行完成。

集成发现 `intrinsic_pointer_value` 漏列 void、usize、isize，而 Copy 分类已将其视为内建可复制值。补齐形成分类后，void 指针读写能经原有资源分析和 LLVM 路径执行；零大小局部指针逃逸仍被拒绝。没有绕过存活期或来源检查。实际库协议验收覆盖所有已支持固定整数和两种 float 的 clone、源值保留、普通指针共享目标，解释器与原生执行均通过。新 examples/host-library-resources.fl 在真实多包入口通过 O0/O3 的四项 native source test，检查标量/指针 Clone、Drop 清理、Copy 不执行有副作用的 Clone；旧真实 traits re-export 的非 Copy Clone 验收也通过。

额外自定义 Clone 返回含指针结构体的验收仍失败：后续 `cloned.counter.*` 被资源准备以 `pointer target provenance is not established` 拒绝，见 `/tmp/luna-tools-xb2_7yqo/result.json`。复现源码保存在 development/repros/host-clone-result-pointer-provenance.fl，新增注释不属于原测试输入；此文件尚未单独重跑。返回整个 `self.*` 与重建字段均遇到此问题。通过的 Copy-vs-Clone 示例检查副作用和原有副本，不声称返回结构体字段来源已修复。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --test library_protocols --test host` 退出 0，192 + 317 + 9 共 518 项通过（`/tmp/luna-tools-bggw_bz_/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-ilc8xmlu/result.json`），`git diff --check` 通过。没有执行全 workspace 测试；dev 未提交。

`python3 -B development/library_check.py --timeout 300` 已新增 native-resources/native-math 两个独立阶段；实际执行两阶段分别 4/2 项通过，声明与 manifest 也通过。完整 sysroot 仍因已有 panic、String、Any、集合/格式化/I/O hooks 等未实现而退出 1，汇总 `/tmp/luna-library-1c5zf00t/summary.json`。模块拆分未添加 unresolved resources 导入错误；没有用假实现遮住整体失败。

## Trait 方法的聚合返回来源已接通

上一节 Clone 返回结构体后丢失指针来源的问题已修复。根因并非 Clone 修改 pointee，而是 moves 的聚合返回分析显式排除了 TraitMethod 调用，退回普通 expr 路径后只检查调用效果，没有交付返回字段映射。现在对静态方法先做完整 method selection，再通过已有 named aggregate 路径传递返回 origins、guards 和返回值元数据。不克隆/重放 receiver 或实参，也不使用 behavior-only 选择绕过接口义务。

examples/host-library-resources.fl 已恢复对 cloned.counter 的读取断言。真实 resources 包在 O0/O3 下验证整个 self.* 返回及重新构造 Self 字段两种 Clone；额外 native 测试验证嵌套泛型结构体中的指针来源、局部逃逸拒绝、移动 owner 后访问拒绝、失败保留旧产物。保存的 repro 也通过实际多包 CLI 运行：`python3 -B development/check.py --timeout 300 -- run -p luna -- --packages-v1 --package 1 101 app development/repros/host-clone-result-pointer-provenance.fl --package 2 202 resources library/builtin/resources.fl --dependency 1 resources 2 --primary 1 --copy-protocol resources.Copy --drop-protocol resources.Drop --test-native --opt-level 3` 退出 0，4 项通过（`/tmp/luna-tools-2ph7ns10/result.json`）。其后只将首行注释从 Known rejection 更新为 Regression。

完整 selection 仍拒绝带 program pre/postconditions、尚未组合内存契约的接口。补充验收确认：不满足前置条件时报 program precondition；满足前置条件后仍明确报 interface predicates require a composed memory contract，不悄悄丢弃接口。该独立限制尚未解决。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test resources --test frontend_cli --test library_protocols` 退出 0，192 + 319 + 9 + 357 共 877 项通过（`/tmp/luna-tools-z5lada4h/result.json`）。补充“满足接口前提仍保留组合契约边界”断言后，`test -p luna --test host native_trait_aggregate_result` 退出 0，2 项通过（`/tmp/luna-tools-ru_e4k23/result.json`）。未运行全 workspace 测试，dev 未提交。

## 契约绑定对齐与 staged 契约边界

为组合接口/实现的程序契约加入 `nir::contracts::BindingAlignment`：对已完成类型替换的程序契约，按参数位置对齐 local identities，复用实现的命名结果和对应入口快照；实现没有该观察槽时保留接口身份。校验参数数量、重复参数、结果/快照冲突、无效 snapshot 输入；有限预算不足时拒绝，调用方输入保持不变。该对象只是替换计划，不是程序谓词成立的证据。Display 使用 `(contract-binding-alignment ...)`，映射按 BTreeMap 稳定输出。

materialization 的 staged trait 调用已复用此组件，替代原先 zip 参数和手动合并 snapshot 的内嵌逻辑；谓词表达式仍走已有作用域感知重绑定。新增原生用例同时覆盖接口/实现不同结果名与双方 old-value 快照，以及实现没有命名结果/快照的情形。用例暴露了契约表达式副本共享源码位置的问题：原先资源准备报 `cleanup site has no unique runtime boundary in keep ... (ScopeEnd, 2 matches)`（`/tmp/luna-tools-p9p21979/result.json`）。现对每个实例化后的 pre/postcondition 副本分配独立 expansion-source 映射，保留原始位置，不改变绑定或求值次序。相同用例通过 checked native compilation/execution，失败的接口前提仍拒绝并保留旧产物。

这完成了可复用绑定对齐和已有 staged 路径的修复，没有解除普通 runtime TraitMethod 的 `interface predicates require a composed memory contract`。下一步组合内存摘要仍需携带接口谓词的实际读写足迹、失败清理和入口观察环境；单独选中实现函数身份不能替代这些义务，不能使用 behavior-only selection 绕过。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test resources --test frontend_cli --test test_templates --test staged_aggregate_contracts` 退出 0，192 + 320 + 357 + 31 + 74 共 974 项通过（`/tmp/luna-tools-ss0zpb8_/result.json`）。`test -p nir --test contract_alignment --test scoped_contracts` 同入口退出 0，2 + 5 项通过（`/tmp/luna-tools-1xtjxtk0/result.json`）。格式化退出 0（`/tmp/luna-tools-nd0n7p8r/result.json`）；`git diff --check` 通过。未运行全 workspace 测试，dev 未提交。


## 方法接口与实现的组合内存摘要查询

新增 `memory::infer_method_in_package` 与带完整 callee 身份的 `MethodSummary`。在私有副本中实例化接口和选中实现，对齐参数、命名结果和入口快照，将接口前置条件放在实现前置条件之前、接口后置条件放在实现后置条件之后，再推导内存访问、失败可能性和返回来源。结果不以实现函数的身份缓存，因此不将接口事实泄漏给其他直接实现调用。Display 为 `(method-memory-summary (callee ...) ...)`。

子调用使用已有完整固定点查询；残留 pending/queue 时拒绝发布。普通摘要与组合摘要共用 completed runtime-instance cleanup 足迹导入、形状验证及稳定排序去重。API 当前仅支持 runtime 参数帧，不接受 quote 方法或尚需 staging 的 comptime 参数；不提供外部 callable premises。谓词重绑定不支持的形式明确拒绝。预算失败不修改 package，重新给足预算可独立查询。

源码 elaboration 已会把接口前提继承到实现，所以测试没有把“省略实现前提”误称为契约丢失；专门在测试的 NIR 副本中移除继承前提，验证接口查询仍可恢复读取。另有实际源码的后置条件返回指针观察、实现写入、old-value 参数观察和传递调用覆盖。

这是组合足迹的查询 API，尚未接入 moves 的普通方法调用准入，仍保留 `interface predicates require a composed memory contract`。足迹不是程序契约成立或失败清理完成的证据；下一步需将接口身份、调用前失败的所有权、结果产生后的契约失败清理与入口观察环境一并接入，不能仅取消现有拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test method_memory_contracts --test host --test resources` 退出 0，320 + 3 + 357 共 680 项通过（`/tmp/luna-tools-0mfe2wvw/result.json`）。未运行全 workspace 测试，dev 未提交。

补充 prepared cleanup overlay 测试：组合摘要保留实现正文以外的写入；畸形参数位置拒绝，修复输入后查询恢复。`test -p luna --test method_memory_contracts` 同入口退出 0，4 项通过（`/tmp/luna-tools-1c_xlosh/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-f1sun7_c/result.json`）；`git diff --check` 通过。


## 高阶方法组合摘要的回调前提

新增 `memory::infer_method_with_code_inputs`，接收具体 named callback 候选和 prepared closure 定义及捕获环境。接口谓词与方法体仍在独立组合副本内分析；回调访问经过已有参数来源映射，与接口读取共同进入结果。结果 `callable_inputs` / `prepared_inputs` 保留调用前提，不能作为更换回调后的无条件摘要使用。原来的 `infer_method_in_package` 现在是无外部回调前提的便捷入口。借用回调和 effect 上下文未在此入口开放。

测试覆盖同一方法传入只读/写入回调时的不同足迹、多候选足迹合并、接口独有读取、无捕获及 bool 捕获闭包、签名不匹配、缺失/未知/空 named 候选、畸形 closure 参数布局、预算失败和重新查询恢复。源码形成与 moves 提供的闭包证据用于构造 prepared 输入，没有把未建立身份的回调当成无副作用。带接口谓词的普通 runtime 方法调用仍未解除拒绝：moves 的方法值、聚合结果和普通调用会折叠为实现函数身份，后续需要统一保留接口身份和失败清理义务，而非只替换一个查询调用。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test method_memory_contracts --test captured_memory` 退出 0，6 + 94 共 100 项通过，报告 `/tmp/luna-tools-t9x3xs1y/result.json`。全 workspace 格式化退出 0，报告 `/tmp/luna-tools-9oki91uf/result.json`；`git diff --check` 通过。未执行全 workspace 测试；改动保留 dev 工作区，未提交。


## LLVM 直接方法调用的接口入口检查

此前 LLVM 的 `K::Call` 在 trait callee 上只取选中实现地址，未执行 IIR 保留的接口前置条件。现在按完整 interface/arguments/subject/member/method_arguments 查找具体契约，在所有显式及默认实参准备完成后，以接口参数绑定读取已取得的值，发射前置条件，再将参数交给实现。实参不重算，接口绑定只用于该检查并在之后恢复调用者环境。缺失具体契约或参数布局不一致明确拒绝。失败边界使用现有 `InterfacePreconditionFailure`，检查时 prepared construction 尚未结束。

后端单元验收从源码形成 IIR 后移除实现函数的继承前提，单独验证接口谓词：正常图成功；只将接口条件改成 false 时，O0/O3 原生执行均失败；缺失契约/错误参数帧拒绝，原图再次发射保持一致。这是 backend handoff 的行为测试，不声称篡改后的图通过源码验证。另有源码 CLI/native 用例验证副作用实参只求值一次、optional 缺省参数读取已准备的前序值。

此修复未解除带程序谓词接口的 checked memory/moves 准入限制，也未补齐接口后置条件的独立结果清理。后续仍须完整保留方法身份、组合足迹与所有权失败路径；不能仅靠新增 LLVM 检查宣布资源接线完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --lib` 中 lib 27、frontend_cli 192、既有 host 320 项通过；新增用例初次因将 optional 参数误写为 `second:bool=first` 而解析失败，整体退出 101（`/tmp/luna-tools-umutvbi5/result.json`）。改为合法 `.second:bool=first` 后，`test -p luna --test host native_trait_entry_predicates` 同入口退出 0，1 项通过（`/tmp/luna-tools-jjpoj_yq/result.json`）。本轮相关 540 项均有通过证据；未运行全 workspace 测试，dev 未提交。


## LLVM 接口后置条件与入口观察帧

直接 trait 调用现为接口建立独立的观察帧：实参准备后绑定接口参数并取得 entry snapshots，检查入口谓词；实现调用正常返回后，先处理实现失败传播，再把实际返回值绑定到接口 result local 并检查接口后置条件。每次检查后恢复调用者 locals，连续调用不共用结果/快照绑定；Never 调用没有正常出口检查。快照目标必须是接口参数，不能从调用者恰好同名的 local 中取得。接口契约仍由完整 interface/subject/member/type_arguments 定位。

当前后置条件原生执行只对未启用资源检查的 host 路径开放。checked emission 对存在接口后置条件的调用明确拒绝 `host interface postconditions require a result cleanup boundary`：实现已经取得输入所有权、返回值尚未交付调用者，这个边界需要独立清理计划，不能复用接口入口失败或假定实现析构会覆盖返回值。已有 moves 的组合契约准入限制也未解除。

新增 backend handoff 测试从源码形成 IIR 后移除实现侧前后条件，保留接口结果与 old-value 快照；正常实现原生成功，仅改实现返回值则接口出口检查失败，O0/O3 均验证。坏 snapshot 输入拒绝。另有完整源码 CLI/native 用例，验证接口与实现命名结果不同、连续 true/false 调用各自观察实际返回与入口值。测试构造的变异 IIR 只用于隔离后端行为，不声称通过源码语义验证。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，28 + 322 共 350 项通过（`/tmp/luna-tools-6x1x_rxm/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-dqw62h6m/result.json`），`git diff --check` 通过。未运行全 workspace 测试；改动保留 dev，未提交。


## 接口后提失败的返回值清理边界

IIR cleanup 新增 `InterfacePostconditionFailure`，Display 为 `interface-postcondition-failure`；其 source 是调用表达式，与接口入口失败、实现函数自身的后提失败分开。源清理发布器为直接 TraitMethod Call 注册该 hook，并将参数帧末尾的 `parameters.len()` 槽登记为待交付返回值（Prepared 的 construction/value_source 均为 call source）。这只登记合法运行时位置，不自行制造资源证明。

LLVM checked 发射现在要求该调用恰有一个对应 site，包括经验证可为空的清理计划；缺失或重复 site 拒绝。调用成功产生结果后，将需要清理的返回值保留到独立 prepared 槽，接口后提失败走专用边界，成功则解除该槽的保留。实参进入实现前已经完成转移，结果清理不复用实参槽。后提失败清理通过既有 failure propagation 继续展开调用者，未将错误视为正常结果。

低层 handoff 原生测试提供显式 schedule，清理动作经返回指针将外部 u32 计数器加一。仅替换开发 main 以读取计数器与 failure flag，生成函数、接口检查和析构代码不变；O0/O3 均验证成功路径 0 次、失败路径 1 次。缺失/重复边界拒绝。此测试证明后端执行给定计划，不证明源码 passes 已生成该计划：moves 对普通带程序谓词的方法仍未交付组合调用的返回清理 site，源码准入限制继续保留。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test resources` 退出 0，29 + 322 + 357 共 708 项通过（`/tmp/luna-tools-17koasx_/result.json`）。随后将清理观察从 bool 翻转加强为精确 u32 计数，`test -p luna --lib interface_exit_cleanup` 同入口退出 0（`/tmp/luna-tools-k152_wmd/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-91zlfz5a/result.json`），`git diff --check` 通过；未运行全 workspace 测试，dev 未提交。


## 组合内存摘要保留真实实现调用边界

接口摘要不再把接口 pre/postconditions 拼进实现函数的局部帧。分析现在构造私有调用包装：接口参数/入口快照、入口谓词、普通 FunctionInstance 调用、接口结果绑定与出口谓词。实现自己的程序条件、局部参数重绑定和完成后的 cleanup 足迹通过普通完整调用摘要处理；接口观察不复用实现可变 local。包装仅存在于查询中，不发布为新的源函数或实现函数的无条件缓存条目，完整 callee 身份仍由 MethodSummary 保留。

为此只构造参数/签名元数据与一次调用，不再复制并替换整个实现函数体；预算分别计入参数帧、类型、接口谓词及生成实参。默认实参已在调用方准备，此包装逐位置转交已经取得的值，不重算 defaults。高阶 named/prepared 回调证据沿普通调用路径传递，现有返回指针与 cleanup overlay 验收继续通过。

新增源码回归：实现执行 `p=q` 后返回，接口仍有 `ensures p.*`。旧拼接方案使接口读取跟随实现重绑定，漏掉原始参数 0；现在摘要同时保留原始 p 与实现使用的 q 的读取，且不将局部重绑定当作 pointee 写入。此处仅验证足迹，不把足迹当成接口谓词成立的证明。moves 的完整方法调用准入与 interface-result 清理 site 生成仍待接通。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test method_memory_contracts --test captured_memory` 退出 0，7 + 94 共 101 项通过（`/tmp/luna-tools-8nlsxkwj/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-85pkp5i0/result.json`），`git diff --check` 通过。未运行全 workspace 测试；dev 未提交。


## 参考执行器的接口出口与清理一致性

参考执行器 prepare_call 现在保留完整接口 program contract。已准备实参通过 representation checkpoint 建立独立接口观察值，不直接保存实现将取得的 Storage 槽；入口快照只从接口参数取得。实参仍求值一次。正常调用返回后，把实际结果放入接口 result binding，执行后置条件并记录 CheckPredicate 事件。接口 program 复制/实例化经过 copy-budget 检查，不提前展开 normal-form 类型头。

有 cleanup schedule 时，调用前要求唯一 InterfacePostconditionFailure site。实现成功后将返回值放入参数帧末尾的 prepared 槽；接口后提失败在调用者环境执行该 site，然后报告 ProgramContractRejected；成功则解除保留。实现自身失败不会运行接口后提。缺失/重复 site 报 InvalidCallPlan，不以空清理假设通过。

共用 handoff 测试现在同时运行 LLVM O0/O3 与 IIR reference：移除实现继承检查后仍验证接口入口/出口；带显式返回清理 schedule 的成功路径不调用 release，失败路径恰调用一次。参考执行的 EnterFunction 事件与 LLVM 外部 u32 计数相互核验。另有 NIR Interpreter::new 路径测试，覆盖非 IIR 接口 program 实例化，不依赖实现继承后提。测试不代表源码 moves 已生成这些清理计划，原准入限制保留。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test resources --test host --test method_memory_contracts` 退出 0，29 + 322 + 7 + 357 共 715 项通过（`/tmp/luna-tools-6fvrn3pp/result.json`）。补充 NIR 测试及 copy-budget 后，`test -p luna --lib --test method_memory_contracts` 同入口退出 0，29 + 8 项通过（`/tmp/luna-tools-m_xgqjxe/result.json`）。补充参考执行缺失/重复 site 拒绝后，`test -p luna --lib host::method_contract_tests` 退出 0，3 项通过（`/tmp/luna-tools-ut20izcd/result.json`）。格式化退出 0（`/tmp/luna-tools-gh27e31q/result.json`）；`git diff --check` 通过。未运行全 workspace 测试，dev 未提交。


## 接口调用帧的真实资源报告

从组合内存查询中提取 `memory::prepare_method_invocation`，返回 `MethodInvocation`（完整 callee、选中实现 InstanceKey、独立接口 Function 帧），Display 为 `(method-invocation (callee ...) (implementation ...) ...)`。内存推导与资源分析复用同一帧构造，不各自复制接口契约逻辑。该 Function 的 declaration 仅为分析标签，不能据此替换实现或按实现身份发布无条件摘要；调用身份与外部前提仍必须保留。

新增驱动协议入口 `protocols::analyze_method_cleanup`，在帧构造消耗预算后，以绑定的 Copy/Drop 协议调用真实 moves/cleanup 分析，返回 `MethodCleanup { invocation, report }`，Display 为 `(method-cleanup ...)`。它能生成接口入口失败和返回后接口出口失败的真实资源计划；这不是自动为调用点发布 schedule。调用者仍须提供实际输入来源，并将 frame 参数/结果/源码边界映射到 invocation 的 prepared 槽和 InterfacePre/PostconditionFailure。

新增源码形成的 Factory/Token 用例，Token 有用户 Drop，flag 使用 Copy，接口具有入口条件与 old-value 出口条件。报告验证：入口失败包含输入 Token 的取得清理计划；接口后提失败包含命名返回 Token 的用户 Drop Invoke，且不再包含已转交的输入 Token；包装主体保留独立实现调用。零预算查询拒绝，输出保持 S-expression。早期测试误以为无上下文的静态调用一定出现在 named_calls（该表用于需要调用上下文的证据），已改为检查实际包装 Call 与返回清理计划，而未改变分析行为。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test method_memory_contracts --test resources` 退出 0，30 + 8 + 357 共 395 项通过（`/tmp/luna-tools-b48jdc08/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-soolqeun/result.json`），`git diff --check` 通过。未运行全 workspace 测试；dev 未提交。

## 接口资源报告映射到实际调用槽

新增 `cleanup::lower_method_call_cleanup`，将真实 MethodCleanup 报告映射为调用点的 MethodCallCleanup 片段，并提供 S-expression Display。映射核对完整 callee、签名与实参槽：入口失败清理对应参数的 prepared 槽；出口失败清理参数帧末尾的待交付结果槽。析构程序沿用已有 cleanup lowering，收集实际析构根与 receiver 类型。预算耗尽、身份被改成仅有实现函数、重复参数槽及未支持的调用上下文均拒绝。

调用点映射另核对完整且唯一的 parameter_bindings，以及显式实参之后按声明顺序排列的 defaults。显式实参仍按源序准备，允许其槽顺序与声明顺序不同；清理目标通过参数槽及 value_source 对应。新增缺失/重复绑定、默认实参顺序拒绝及显式重排成功回归。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --lib host::method_contract_tests` 退出 0，6 项通过（含参考执行与 LLVM O0/O3，`/tmp/luna-tools-dq_6k30v/result.json`）；全 workspace 格式化退出 0（`/tmp/luna-tools-tzjery78/result.json`），`git diff --check` 通过。

多个接口谓词共享失败边界时，仅允许相同清理程序合并，不将动作串接造成重复析构。需要独立观察对象清理、延期调用、handler/default/closure 上下文的报告暂不接受；这些情况不能靠丢弃证据发布。该产物只覆盖接口拥有的资源，尚不包含调用者局部变量、外围未完成构造及真实输入来源，因此不是可直接发布的完整 Schedule，也未解除普通方法调用的准入限制。

Factory/Token 源码测试现在从真实资源报告取得上述片段，再为简单调用者显式补充成功后的返回值 discard。共享 fixture 经参考执行器与 LLVM O0/O3 验证成功、入口失败、出口失败三条路径；参考执行事件检查析构调用次数。重复 pre/postconditions 验证边界去重，另检查零预算、错误身份/槽布局及失败后的重新查询。此测试是报告到执行器的 handoff 验证，仍不是生产调用者 schedule 自动组合的端到端验收。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test resources --test host --test method_memory_contracts` 退出 0，30 + 357 + 322 + 8 共 717 项通过（`/tmp/luna-tools-3lpgxs4_/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-uvxmiafk/result.json`），`git diff --check` 通过。未运行全 workspace 测试；dev 未提交。


## 高阶接口帧的资源前提传递

新增 `protocols::analyze_method_cleanup_with_context`，接口帧现在接受与普通函数资源分析一致的 callable/enum/effect、直接借用、字段借用和 borrowed-code 前提；原无前提入口委托到此入口。沿用同一个 Copy/Drop world，报告保留参数索引、代码身份和字段路径，不按函数类型猜测回调，也不将条件报告缓存成实现的无条件证明。

`Ok(MethodCleanup)` 仍只表示接口帧分析返回报告。无回调身份的用例会保留 deferred_calls；有身份时记录指向选中实现的 contextual named call。子调用必须继续检查。新增测试把该子调用的原样前提交给实现资源分析，验证捕获 Token 的闭包调用一次通过、两次消费拒绝。另覆盖命名函数、无捕获闭包、错误参数槽、错误签名、零预算及失败后恢复，检查报告的 S-expression 输出保留回调身份。

调用槽映射现在也拒绝 named/named-candidate 子上下文及未处理的 callable/enum/effect/borrow 前提，避免只映射顶层 entry/result 清理时丢失这些条件。这是当前片段 API 的限制，不是语言限制；生产接线仍需在真实调用者中核对 prepared 输入并展开/组合子上下文。尚未放开带程序谓词的普通方法资源准入。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test resources --test method_memory_contracts` 退出 0，32 + 357 + 8 共 397 项通过（`/tmp/luna-tools-t925foua/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-hx1r4gx7/result.json`），`git diff --check` 通过。未运行全 workspace 测试；dev 未提交。


## 接口失败清理使用调用者局部帧

修复 LLVM method_entry/method_exit 在接口观察帧中发射 caller cleanup 的问题。接口绑定和调用者 LocalId 可能重号；谓词在观察帧计算，但 guard 的失败分支必须切换回调用者槽执行清理。现在通过 map swap 临时切换，发射出错也先恢复，再传播错误；不为每个谓词复制整个调用者帧。参考执行器原本已向 interface failure cleanup 传 caller environment。

接口观察帧现从空局部映射建立，只加入已准备参数及显式快照，不再继承调用者 locals。新增非法 handoff 回归分别让接口 pre/postcondition 引用仅存在于 caller 的 bool 局部变量：LLVM 报 local binding unavailable，参考执行器报 MissingBinding；合法输入及失败后重新发射正常。此改动不扩展当前 checkable_predicate 的可执行片段，也未实现谓词内部任意调用。工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，33 + 322 共 355 项通过（`/tmp/luna-tools-p0o3uvmk/result.json`），全 workspace 格式化退出 0（`/tmp/luna-tools-jve9xa_h/result.json`），`git diff --check` 通过。

扩展显式 schedule 的共享回归：增加与调用者指针 owner 重号的接口 bool 绑定，同时保持实际调用绑定计划一致；出口计划按序清理 prepared 返回值及 caller local，入口计划只清理 caller local。成功、入口失败、出口失败分别检查次数，LLVM O0/O3 外部计数器与参考执行 EnterFunction 事件一致。该 handoff 检查不代表生产 passes 已自动组合 caller schedule；普通方法资源准入限制未改变。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，32 + 322 共 354 项通过（`/tmp/luna-tools-rp7ncbd7/result.json`）。把逐谓词 clone 改为 swap 后，`test -p luna --lib host::method_contract_tests` 同入口退出 0，6 项通过（`/tmp/luna-tools-mmgf4iul/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-ptvln_zz/result.json`），`git diff --check` 通过；dev 未提交。


## 普通调用路径自动发布接口出口清理

moves 现为不走组合内存分析的直接 TraitMethod 调用生成 interface_postcondition 清理报告（Display 保留标记）。在实参已转交后克隆失败分支，将返回值放入 `parameters.len()` 的独立 prepared 槽，再沿原有 exit 分析取得返回值、调用者局部及外围未完成构造的清理。成功路径不消耗这些责任。发布器将其映射为 InterfacePostconditionFailure，即使 scalar/void 不需要运行时析构，也保留唯一的显式空边界。LLVM 清理准入同步识别直接接口入口/出口及返回 prepared 槽，仍检查实际调用 source 与目标。

源码 checked build 已覆盖标量结果、void 结果、调用者 Token、外围实参 Token、以及返回 Token 的实际用户 Drop。可运行示例为 `development/examples/host-interface-contracts.fl`（绑定 Duplicate/Retire）。另以源码生成的完整 schedule 验证三条路径：成功交付后清理结果和 caller，入口失败清理未交付输入和 caller，出口失败清理待交付结果和 caller。只修改具体 IIR 的接口谓词以触发失败；schedule 由生产资源分析生成，未手写动作。参考执行事件检查精确调用次数，LLVM O0/O3 核对成功/失败状态。

该接线没有移除 MethodCallees 对组合内存路径的拒绝：带指针输入/结果、暴露内存和高阶调用上下文仍需保留实际来源与完整身份后接通。它也没有扩展接口谓词中的任意调用能力。此前“普通方法尚无接口结果 site”的记录已被本节的实际支持范围取代。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test resources --test method_memory_contracts` 退出 0，34 + 325 + 357 + 8 共 724 项通过（`/tmp/luna-tools-h50wwhfn/result.json`）。未运行全 workspace 测试；dev 未提交。

将 owned-result 验收改为直接读取上述示例文件后，`test -p luna --test host native_resource_checked_trait_owned` 同入口退出 0（`/tmp/luna-tools-qnp2kpfw/result.json`）。最终全 workspace 格式化退出 0（`/tmp/luna-tools-3l70ppio/result.json`），`git diff --check` 通过。


## 默认实参中的接口失败清理合并

默认实参内部的实现失败与接口后提失败会分别产生相同的外层 DefaultFailure 清理计划。发布器现对 DefaultFailure、InterfacePreconditionFailure、InterfacePostconditionFailure 的完全相同有序动作去重；不同计划拒绝并要求路径区分，不拼接成可能重复/越界清理的动作序列。原来该源码场景报 ambiguous repeated cleanup owner，现可 checked build 并运行。

扩展源码验收覆盖循环中的接口调用及默认实参调用。生产 schedule 共享测试增加默认实参场景，入口失败/出口失败/成功均检查外层 Token 的清理次数；参考执行与 LLVM O0/O3 一致。拥有结果的普通调用回归保留。尝试将 owned Token 作为默认实参结果时，前端仍以 default expression requires P3 stability/termination evidence 拒绝；这并未证明存在实现缺口：docs/functions/parameters.md 要求 P3 默认计算不得交付新的可观察资源身份，不能仅凭 Token 空字段或空 Drop 就放宽。该规则保持不变，本轮默认实参验收使用合法 bool 结果并保留 owned caller 资源。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test resources` 退出 0，34 + 325 + 357 共 716 项通过（`/tmp/luna-tools-i3kexdkf/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-wcrfxzlt/result.json`），`git diff --check` 通过。未运行全 workspace 测试；dev 未提交。


## 普通接口调用传递实现的高阶资源上下文

`moves::record_named_call` 现将直接 TraitMethod 的已准备 callable/enum/effect 输入传给选中实现的资源子上下文。这里的 behavior selection 仅取得实现身份，原始接口 Call 不改写；接口 entry/result 仍由普通调用路径生成边界，原有组合内存路径的完整身份限制保留。此前该函数对 TraitMethod 直接返回 None，丢失实现所需的回调前提。

新增 checked 源码测试：命名函数与无捕获闭包作为接口方法输入，LLVM O0/O3 均返回 true。带 Token 仿射捕获的闭包通过同一生产上下文传递，单次消费可形成完整 schedule 并由参考执行返回 true；重复消费在资源检查阶段拒绝。LLVM 对消费捕获字段仍拒绝 `host closure capture ownership transfer needs persistent environment state`，此处需要后续持久环境状态写回实现，未把参考执行通过称作原生支持。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test resources --test method_memory_contracts` 退出 0，34 + 327 + 357 + 8 共 726 项通过（`/tmp/luna-tools-3ddg6p0m/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-n72wcwos/result.json`），`git diff --check` 通过；dev 未提交。


## LLVM 闭包环境持久构造状态

原来 host 在 select_schedule 拒绝任何 capture ownership transfer，因为构造标记只在被调用函数的局部帧中，外层 cleanup 看不到捕获已被消费。现将 product capture 的构造状态附加到内部闭包布局（code pointer、环境字段、i1 状态数组）；构造时建立 token，bind_environment 直接把捕获及 product 子字段的 live 槽映射到环境中的状态。调用仍按环境指针借用，消费/重建沿既有 set_live 更新，条件 cleanup 读取持久槽。嵌套闭包的字段路径选择最内层环境 token；拷入新的拥有槽时保留值中已有捕获状态，不重置成全构造。该布局是当前 host 内部 ABI，不是外部 FFI 承诺；sum payload 仍按已有 whole-token 模型处理。

解除先前统一拒绝后，checked 接口输入中的仿射闭包单次消费现在可原生执行；重复消费仍由资源检查拒绝。先前两个专门验证 host 暂不支持 capture transfer 的测试已改为 O0/O3 执行验收。新增 `development/examples/host-closure-consume.fl`，用 u32 析构计数验证条件消费 Pair.first 后，剩余环境只清理应有字段：true/false 两路径总计均为 2。测试还将其包入嵌套闭包，O0/O3 同样计数正确；部分消费后整体转移闭包仍拒绝 unavailable owner。不同调用上下文需要不同清理主体时的原有拒绝未解除。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test resources` 退出 0，34 + 328 + 357 共 719 项通过（`/tmp/luna-tools-b8i0tzj3/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-i61qwbps/result.json`），`git diff --check` 通过；dev 未提交。


闭包持久构造状态的失败路径已补充回归：从源码生成完整资源 schedule，闭包根据 runtime 条件消费 Pair.first，可选重建该字段，然后执行 `assumes passed`。八种 take/rebuild/passed 组合验证参考执行的 Drop 事件次数与 LLVM O0/O3 的外部 u32 析构计数，未重建精确为 2，重建精确为 3；同时核对失败标记。仅替换开发 main 读取计数器，未修改函数或清理计划。本轮未发现实现缺陷，保留测试作为持久状态在成功/失败退出时的证据。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib host::method_contract_tests` 退出 0（`/tmp/luna-tools-xozkj6lh/result.json`）；全 workspace 格式化退出 0（`/tmp/luna-tools-6oyd3uky/result.json`），`git diff --check` 通过。未重复全量测试，dev 未提交。


## 接口观察帧同时隔离存活状态

LLVM 接口观察帧此前已隔离 locals，但仍共享 caller 的 live 映射。接口参数绑定与 caller ID 重合时，bind 的 set_live 可把已经转交的 caller 位置重新标成存活。现 MethodFrame 同时保存独立 live 映射；计算接口谓词时使用观察帧，发射失败清理时把 locals/live 一起切回 caller，成功或发射错误后也恢复 caller。

共享 handoff 回归新增显式转移实参的位置：同一 ID 在接口内绑定 bool，在 caller 中原本拥有指针；caller 清理动作带 IfConstructed。对成功、入口失败、出口失败检查，已转移位置不得再析构，出口只清理 pending result。参考执行事件与 LLVM O0/O3 外部计数均一致。此用例检验后端帧隔离，不替代 source planner 的语义证明。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，35 + 328 共 363 项通过（`/tmp/luna-tools-biz6zdgg/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-7jsbho4f/result.json`），`git diff --check` 通过；dev 未提交。


## 临时闭包调用与残余环境共用存储

LLVM 现为有残余清理责任的 callee 保留临时槽，并将同一个槽传作闭包环境指针。复用已有 residual retention 的状态槽管理；不再另建一个调用环境副本，使消费/重建的持久 capture token 可被后续 cleanup 看见。清理准入同步识别 Call 的 callee source 作为合法 Temporary 目标，仍由源资源报告决定责任。此前 `(do {callback})(flag)` 在拥有剩余字段时被 host destructor invocation admission 拒绝。

新增接口工厂返回 owned Task 的源码验收：结果可以先绑定再调用，也可以立即调用。持久捕获计数回归增加临时调用；共享源码 schedule 测试覆盖普通调用、临时调用、临时调用实参内 `assumes` 失败三类路径，分别组合条件消费、字段重建和成功/失败。参考执行 Drop 事件与 LLVM O0/O3 外部 u32 计数一致：调用体未进入时释放整个环境，调用体失败时只清理剩余状态。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host --test resources` 退出 0，35 + 329 + 357 共 721 项通过（`/tmp/luna-tools-4_a0h_54/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-ao3k3k53/result.json`），`git diff --check` 通过；dev 未提交。

### 循环复用与状态寻址

新增同一源码位置连续三轮创建临时闭包、交替消费与保留捕获的源码回归。每轮环境重新构造、调用和清理，最终析构计数严格为 6，LLVM O0/O3 均通过，防止临时槽沿用上一轮已消费的状态。

捕获状态寻址先沿类型结构定位最内层闭包，再计算带预算的前序状态索引，只生成一次状态地址计算。普通 product 字段不再生成无用 GEP，也不再为单次查询构造全部捕获字段路径。此调整不改变闭包布局和清理语义。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，35 + 330 共 365 项通过（`/tmp/luna-tools-j654tfet/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-hhsdqlf2/result.json`），`git diff --check` 通过；未重复全 workspace 测试，dev 未提交。

### 优化前明确 LLVM 工具链目标

工具链在调用 opt 前读取所选 llvm-config 的 `--host-target`，验证非空及字符范围后传入 `-mtriple`。opt 为模块写入目标 triple 和 data layout，对象生成沿用优化模块中的目标；不再直到 llc 才选择目标。O0/O3 回归验证布局存在、重复优化文本稳定，并通过执行 LLVM 指针大小计算核对原生 Rust ABI 的指针大小。此处仍只支持本机，不提供交叉编译承诺。

`usize/isize` 的数值支持仍未完成：前端字面量形成、验证及 staging 的固定宽度查询尚需显式目标布局输入，不能将编译器进程的 Rust 位宽代入 `fixed_integer_layout`。本次仅完成 LLVM 工具链侧目标接线，没有放行这些字面量。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，36 + 330 共 366 项通过（`/tmp/luna-tools-pw922rsl/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-oms8ftg2/result.json`），`git diff --check` 通过；dev 未提交。

目标一致性进一步收紧：`Toolchain::target_layout()` 通过所选 LLVM 的空模块探测返回 `TargetLayout { triple, data_layout }`，并提供 S-expression Display。优化前由 LLVM 无优化解析/打印原始文本或 bitcode，读取规范化 header；存在与工具链不同的 triple/layout 即拒绝，不能覆盖 triple 后沿用冲突布局。当前要求规范化字符串一致；不尝试证明不同布局拼写等价。没有 header 的模块由目标机器补齐。该 API 尚未传入语义前端。

新增拒绝异目标 triple、冲突 data layout 的测试，同时覆盖文本优化和对象编译入口，验证拒绝后同一工具链仍能执行合法模块。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，37 + 330 共 367 项通过（`/tmp/luna-tools-gpjs0kua/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-wtmnfgup/result.json`），`git diff --check` 通过；dev 未提交。

目标探测现改用单函数模块：让 LLVM instcombine 折叠地址空间零中 `sizeof(ptr) * 8`，将结果作为 `TargetLayout.pointer_bits` 并加入 Display。无需执行探测产物，也不解析 triple 名称来猜宽度；data layout 省略默认 `p:` 项时仍由 LLVM 决定正确大小。内部测试在同一个编译器进程探测 i386/x86_64，分别得到 32/64；这只验证布局查询，不表示开放 cross-target 编译入口。前端字面量及 staging 的目标参数传递尚未完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 退出 0，38 + 330 共 368 项通过（`/tmp/luna-tools-ey9rjo79/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-he6xgdms/result.json`），`git diff --check` 通过；dev 未提交。

### 平台整数共享语义接口

新增 `nir::target::TargetData`：显式、可比较/hash 的目标位宽事实，私有字段及检查构造器防止无效宽度；支持当前机器整数表示可容纳的字节对齐 8..=128 位。`integer_layout` 查询 `usize/isize` 的表示，固定宽度查询保持原义，未把平台类型重写为 `u32/u64`。Display 使用 S-expression。LLVM 的 `TargetLayout::semantic_target()` 将探测结果转换成此共享输入。

`Literal::has_primitive_type_in` 可在明确目标下检查平台整数范围；既有无目标接口继续拒绝平台整数字面量。测试覆盖 32/64 位上下界、符号不匹配、128 位极值、非法目标、固定宽度不变及平台类型身份独立。此处尚未修改 package/elaboration/staging 的调用入口，完整源码路径仍待接线；completed value 递归验证也仍走无目标入口。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir --test completed_values --test target_integers` 退出 0（`/tmp/luna-tools-8s1fqu6o/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --lib layout_probe_uses_target_pointer_width_without_executing_target_code` 退出 0（`/tmp/luna-tools-4813im8f/result.json`）。格式化退出 0（`/tmp/luna-tools-hhvvgguo/result.json`），`git diff --check` 通过；未重复全量测试，dev 未提交。

completed value 新增 `validate_in` 和 `validate_with_declarations_in`，将显式目标贯穿所有 Expr 子图的递归验证，包括产品、newtype/error 包装、私有重建块及闭包捕获。原有入口传 None 保持既有拒绝语义。新增同一个嵌套产品/闭包捕获在 32 位拒绝、64 位接受的测试，兼查无目标、零预算、失败消耗及后续请求恢复。闭包运行体仍按原规则视为已检查代码，不把它当值初始化程序执行。`K::ComptimeValue` 所承载 Term 的验证仍由既有独立入口处理，尚未接目标参数；package/elaboration/staging 接线也仍待完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir --test completed_values --test target_integers` 退出 0，15 + 2 共 17 项通过（`/tmp/luna-tools-f89e2g5y/result.json`）。格式化退出 0（`/tmp/luna-tools-trvi9u52/result.json`），`git diff --check` 通过；dev 未提交。

`comptime_values::validate_in` 现接受同一显式目标，completed value 的 `K::ComptimeValue` 分支已将目标传入：具象 Integer Term 在 usize/isize 域中按目标宽度验型。原 validate 无目标仍拒绝平台整数；抽象/聚合 Term 的完整域判定继续属于实例化/解释层，不借本次接线放宽。回归覆盖嵌套记录中的捕获值、32/64 位边界、unsigned 负数拒绝及预算失败恢复。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir --test completed_values --test target_integers` 退出 0，15 + 3 共 18 项通过（`/tmp/luna-tools-w8skjf13/result.json`）。格式化退出 0（`/tmp/luna-tools-4tcedcxm/result.json`），`git diff --check` 通过；package/elaboration/staging 目标配置仍待接线，dev 未提交。

NIR Package 新增 `target: Option<TargetData>`，参与现有 clone/相等性并出现在 S-expression dump 中。elaboration 增加 `elaborate_packages_for_target`；旧入口显式传 None，函数模板展开递归保留目标，Checker 创建的 required comptime、open function、常量、CMTT 元数据快照也携带相同字段。新增交替 32/64/32 位请求后回到无目标请求的测试，确认产物区分目标且不污染后续请求。这里只验证配置保留；尚未放行平台整数字面量，也未完成 IIR/解释器的配置传播。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p elaboration --test target_configuration` 退出 0（`/tmp/luna-tools-e748g6jc/result.json`）；`python3 -B development/check.py --timeout 300 -- check -p nir -p elaboration -p iir -p comptime_interpretor -p luna --tests` 退出 0（`/tmp/luna-tools-d7e3pk9j/result.json`，这是测试代码的编译检查，不是运行全部测试）。格式化退出 0（`/tmp/luna-tools-jntoiqxo/result.json`），`git diff --check` 通过；dev 未提交。

IIR Package 现携带 target，materialization 从 NIR 复制目标，IIR 的 storage metadata 与 Display 保留该字段。NIR/IIR 两个解释器入口都保留目标并提供只读查询；字面量和 runtime 值域检查、associated completed value 验证及 ComptimeValue 标量检查开始使用目标。新增直接 NIR invocation 的真实值测试：`usize(2^32)` 在 32 位目标拒绝，在 64 位目标返回准确 UnsignedInteger；无目标的 `usize(0)` 仍拒绝。配置测试进一步穿过 materialization 验证 IIR 保留目标。IIR 算术/字面量准入、平台整数转换及源码形成尚未全部接通，不能据此声称源码 host 支持完整。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p comptime_interpretor --test target_integers` 两项通过（`/tmp/luna-tools-thf5wkbw/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p elaboration --test target_configuration` 通过（`/tmp/luna-tools-yskk72h5/result.json`）；`python3 -B development/check.py --timeout 300 -- check -p luna -p elaboration -p iir -p comptime_interpretor --tests` 退出 0（`/tmp/luna-tools-9ub74z6g/result.json`）。格式化退出 0（`/tmp/luna-tools-l7y_wvox/result.json`），`git diff --check` 通过；dev 未提交。

解释器的加减乘、截断除法/余数及取负现按显式目标选择平台整数表示，复用 MachineInteger 的溢出和除零检查。NIR 增加 `machine_integer_type_in`，phase 的机器运算收集及 ComptimeValue 检查使用 package.target；无目标旧接口保持原行为。已知签名中的整数字面量实参代理也查询目标布局。测试覆盖 32 位 unsigned 加法溢出而 64 位成功、unsigned 下溢、signed 最小值取负/除 -1、除零、负数截断除余和捕获 Integer Term 的平台域转换。源码/IIR/host 的完整平台整数功能仍未完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p comptime_interpretor --test target_integers --test scalar_equality` 9 项通过（`/tmp/luna-tools-lazj6zli/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p nir --test phases --test target_integers --test completed_values` 通过（`/tmp/luna-tools-spiu7qkm/result.json`）。格式化退出 0（`/tmp/luna-tools-h6h48g9d/result.json`），`git diff --check` 通过；dev 未提交。

平台整数五种 scalar 比较已接入解释器；staging Layouts 新增显式目标输入，从完整 NIR Package 构造时自动保留目标。canonical integer 输入、机器整数折叠和 scalar 比较折叠按此目标验范围；只传 normal-form heads 的旧入口仍无目标。新增 32/64 位 signed/unsigned 比较的解释器与 staging 对照、越界输入拒绝，以及 `u32::MAX + 1` 在 32 位拒绝、64 位折叠且保留 usize 类型身份的测试。尚未把所有只传 heads/index 的调用点改为完整目标布局，源码、通用 IIR 准入和 LLVM 平台表示也仍待接线。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p comptime_interpretor --test target_integers --test scalar_equality` 11 项通过（`/tmp/luna-tools-2m5eg1m7/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p iir --lib staging::` 通过（`/tmp/luna-tools-5_vzu1z9/result.json`）。格式化退出 0（`/tmp/luna-tools-_jfi9xu5/result.json`），`git diff --check` 通过；dev 未提交。

elaboration 的三个常量准备/调用准备布局入口现附带 Checker.target，纯度检查从完整 Package 构造布局。新增目标感知的 `structural_boolean_dependencies_in`，源码 staged provenance 与闭合局部值替换都传入目标，使平台整数比较可进入既有 total initializer 分析；checked arithmetic、调用和写入仍不属于该纯结构片段。内部回归验证 32/64 位比较别名成功折叠，输入不可用时不替换，无目标入口仍不承认平台比较。materialization 现有路径原本传完整 Package，无需重复改写。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p iir --lib staging::` 46 项通过（`/tmp/luna-tools-2cl0n5t1/result.json`）；`python3 -B development/check.py --timeout 300 -- check -p elaboration --tests` 退出 0（`/tmp/luna-tools-txwbta0u/result.json`）。格式化退出 0（`/tmp/luna-tools-yeoa9x78/result.json`），`git diff --check` 通过；dev 未提交。

### 显式目标源码入口

新增 `frontend::compile_for_target`，显式目标通过单文件路径及 sysroot 的多包 elaboration 路径传入；旧入口仍传 None，不自行查询机器位宽。整数字面量 runtime proxy 在形成时查询 Checker.target，类型保留 usize/isize。源码回归验证跨 32 位边界的 unsigned/signed 字面量在 64 位 NIR 中保留身份并由解释器返回正确值；另测 32/64 位上下界及越界拒绝，无目标保持 pointer-width 诊断。CLI 尚未自动选择 host target，完整运行时算术、IIR 准入和 LLVM 平台整数表示仍需接线。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test target_integers` 两项通过（`/tmp/luna-tools-l9q8lhnx/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test frontend_cli --test packages` 通过（`/tmp/luna-tools-0nlqa7yh/result.json`）。格式化退出 0（`/tmp/luna-tools-9mlgzcdb/result.json`），`git diff --check` 通过；dev 未提交。

平台字面量的源码回归已推进到 materialization/IIR invocation，确认目标保留且结果一致。前端 scalar 比较及原生算术分类改用目标布局，signed 除余仍遵守既有协议边界；测试源码 usize 比较、常量加法在 32/64 位都可实例化执行。同步修复 flow 的 machine 检测、残余契约重检和区间包围，使平台运算必须获得目标宽度证明，不能被当作无限精度操作漏检。源码 `4294967295 + 1` 在 32 位编译时拒绝、64 位成功并返回正确值。参数初始区间等其余固定宽度查询尚未全面迁移，复杂平台整数程序可能仍保守拒绝；LLVM 平台表示仍未完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test target_integers` 4 项通过（`/tmp/luna-tools-d1m_43oh/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test integer_contracts --test arithmetic_contexts` 通过（`/tmp/luna-tools-g14ts59y/result.json`）。格式化退出 0（`/tmp/luna-tools-r_yaz9di/result.json`），`git diff --check` 通过；dev 未提交。

flow State 现保留目标并随分支复制、默认参数帧及闭包 invocation 帧传递；绑定域、unknown scalar、模式字段与运算结果域均可按目标取得 usize/isize 上下界。捕获整数及已知签名实参范围检查也查询目标。源码验证：有 `a < 100` 前提的 usize 加一在 32/64 位可通过并执行；无前提加一仍拒绝；返回 usize 的 `r >= 0` 后提可直接由类型域证明。其余内存协议/BitVec 辅助路径仍有固定宽度查询待迁移，不据此声称全部支持。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test target_integers --test integer_contracts` 19 项通过（`/tmp/luna-tools-nz9x73aq/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p elaboration --lib verification::flow::` 通过（`/tmp/luna-tools-rkn13lh1/result.json`）；elaboration 测试代码编译检查退出 0（`/tmp/luna-tools-7nzt3hj6/result.json`）。格式化退出 0（`/tmp/luna-tools-ouqdv2h0/result.json`），`git diff --check` 通过；dev 未提交。

### LLVM 平台整数表示与目标匹配

LLVM 的 primitive 表示查询现读取 IIR target，usize/isize 使用该位宽及符号。带显式目标的模块输出 `!flurry.target` metadata；工具链在 LLVM 规范化打印之后、优化之前读取并核对 pointer bits，不匹配或畸形 metadata 拒绝。元数据随 O0/O3 保留，避免先按 32 位验证、后按 64 位 host 默默交付；无目标旧模块保持既有支持范围。

源码 API 验收已贯通前端、契约验证、实例化、LLVM 优化、链接和执行：带 `a < 100` 前提的 usize 加一在 host O0/O3 均输出 unsigned-integer 42。相反位宽的同一源码可形成 IR，但 optimized_ir/executable 均明确拒绝目标不匹配。CLI 尚未自动查询并传递 host target，完整平台整数内存/BitVec 场景仍待补齐。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test target_integers` 6 项通过（`/tmp/luna-tools-2_63og3z/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --lib --test host` 通过（`/tmp/luna-tools-yuq9c61n/result.json`）。格式化退出 0（`/tmp/luna-tools-v7ypb4jo/result.json`），`git diff --check` 通过；dev 未提交。

### CLI 自动选择 host 目标

单文件及 packages-v1 的 build/emit-LLVM 入口现在在 elaboration 前查询所选 LLVM 工具链并传递 TargetData；加入 `development/examples/host-platform-integers.fl`。CLI 回归验证普通 build 的 O0/O3、emit-LLVM 元数据和多包 build 均成功，原生输出 42。持久编译缓存命中时核对 NIR target 与当前请求查询的目标，位宽不一致不复用。

native test 暂保留原有无目标前端入口：已有契约要求过滤后零测试即使 LLVM 不可用也能返回空结果，提前查询 LLVM 会破坏它。首次回归发现并修复了这一变化，host 全组已重新通过；native test 的平台整数配置尚需延后到选择测试时处理，不能声称完成。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --test packages` 569 项通过（`/tmp/luna-tools-fc329vqy/result.json`）；新增多包平台整数测试单独通过（`/tmp/luna-tools-r2q9c3ms/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test compilation_cache` 5 项通过（`/tmp/luna-tools-8csxnryo/result.json`）。格式化退出 0（`/tmp/luna-tools-kf6t2509/result.json`），`git diff --check` 通过；dev 未提交。

native test 的单文件/多包入口现在查询可用工具链的目标；查询不可用时保留 None，仍只按无目标规则 elaboration，真正选中的测试再由 native runner 报告工具链错误。没有默认位宽，也不允许无目标平台字面量；目标无关源码的空筛选保持成功。native 测试请求的缓存目标查询使用相同规则。新平台测试发现并补上源码 equality 分类的目标布局查询。

回归覆盖 usize 测试单文件 O0/O3 与多包执行成功，以及 LLVM 不可用时单文件/多包空筛选成功、非空筛选明确报错；保留原 inline effect 清理测试。目标相关源码在工具链缺失时仍可能无法完成发现，这是缺少布局而非默认 host 猜测。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_source_tests` 4 项通过（`/tmp/luna-tools-khz10gkb/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test compilation_cache --test target_integers --test source_tests` 通过（`/tmp/luna-tools-vm100kak/result.json`）。格式化退出 0（`/tmp/luna-tools-j9urjye3/result.json`），`git diff --check` 通过；dev 未提交。

### 平台整数内存契约

flow 的结果临时槽分类、调用写入保持指针绑定的判断、内存后置条件导入均改为目标感知的机器整数查询。此前读取 *usize 的前提在建立观察槽时直接 unsupported。新增 checked host 计数器测试：initialize 写入 41 并保证 `p.* < 100`，caller 据此调用要求该前提的 advance，最终读取 42；O0/O3 均成功。插入 overwrite 写入 100 后，相同 advance 在编译时因 flow 证明失败被拒绝，旧后置范围没有跨写入残留。测试使用显式 Copy/Drop 协议，未绕过指针资源验证。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_platform_pointer_counter_preserves_contracts` 通过（`/tmp/luna-tools-rtcb2lrl/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test resources --test method_memory_contracts --test target_integers` 通过（`/tmp/luna-tools-2u0h_i10/result.json`）。格式化退出 0（`/tmp/luna-tools-tvz_t_oy/result.json`），`git diff --check` 通过；dev 未提交。

### 平台整数 BitVec 读索引

前端 bit_read_index、NIR phase 的动态索引/切片检查以及 flow 端点分类现读取目标布局，允许 usize/isize 索引并维持范围证明。LLVM 回归覆盖 usize 单 bit 读取，以及 usize/isize 四位动态窗口，O0/O3 均返回正确位序；索引越界、负索引、有符号索引缺少非负前提均在编译期拒绝。这里是 BitVec 读取接线，平台整数自身的 bit representation 和动态写回验证尚需后续迁移。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_platform_bit` 2 项通过（`/tmp/luna-tools-gjztw6zz/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test pure_bitvec` 18 项通过（`/tmp/luna-tools-uy2wo1qd/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p nir --test phases` 27 项通过（`/tmp/luna-tools-o8pcv2ju/result.json`）。格式化退出 0（`/tmp/luna-tools-glc40pxj/result.json`），`git diff --check` 通过；dev 未提交。

### 平台整数 BitVec 写索引

新增目标感知的 bit_update_error_in，并将 NIR phase、内存/移动验证与解释器写回检查接入同一 target。纯度分析中的机器整数运算识别也使用目标布局，允许 pure 函数计算 usize 端点。原生回归将 host-bitvec 示例的索引改为 usize：O0/O3 正确完成动态窗口写入、右侧先改存储再写入窗口、空窗口仍执行右侧副作用；窗口超界及空窗口端点超界均被 flow 拒绝。平台整数自身作为 bit representation 存储仍未在此实现。

工作目录 `/home/fin/luna`：host 的 native_platform_bit 筛选 3 项通过（`/tmp/luna-tools-22w9e176/result.json`）；补充拒绝例后写回测试通过（`/tmp/luna-tools-j657gwmh/result.json`）。`python3 -B development/check.py --timeout 300 -- test -p nir --test phases -p luna --test pure_bitvec` 45 项通过（`/tmp/luna-tools-mhel7i7p/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-5aqsr9z1/result.json`）；dev 改动未提交。

### 平台整数位表示与写回

Primitive 新增接受显式 TargetData 的 unrestricted_scalar_width_in；无目标仍不提供 usize/isize 位宽。前端位视图与转换、局部/指针位写回、flow 边界、NIR/IIR 指针检查、解释器与 staging 现在使用该目标。isize 转换和写回按目标位宽恢复符号；usize 转换遵循无符号扩宽规则。类型身份不变，不以 u32/u64 替换平台类型。

显式 32/64 位源码测试覆盖 signed(-1) 位表示往返、局部写回、缺少目标拒绝及有符号宽度不符拒绝；staging 测试覆盖目标不匹配、预算失败后恢复。LLVM O0/O3 实际执行有符号往返、usize 动态写回和 isize 指针写回（-1 清除低四位得 -16）。首次指针测试暴露 IIR 检查未传 target，补齐后通过。平台类型作为 enum 编码载荷及符号覆盖分析的剩余固定宽度入口尚未迁移，不能据此声称所有位表示用途均完成。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 300 -- test -p luna --test target_integers --test host platform_bit_representation`：2 项通过，`/tmp/luna-tools-8hi53g67/result.json`。
- `python3 -B development/check.py --timeout 300 -- test -p luna --test host native_platform_bit_pointer_writeback`：1 项通过，`/tmp/luna-tools-rpsx8ajc/result.json`。
- `python3 -B development/check.py --timeout 300 -- test -p iir --lib staged_platform_bit_representations_require_matching_target`：1 项通过，`/tmp/luna-tools-1nuat_cw/result.json`。
- `python3 -B development/check.py --timeout 300 -- test -p luna --test pure_bitvec --test resources --test target_integers -p nir --test phases`：412 项通过，`/tmp/luna-tools-l2k9yoz7/result.json`。

全 workspace 格式化及格式检查退出 0（`/tmp/luna-tools-avksrl1_/result.json`、`/tmp/luna-tools-xiedl1sh/result.json`）；git diff --check 通过。改动保留 dev，未提交。

### 平台整数 enum 编码

enum 的 unsigned base_type 和编码载荷宽度现读取显式目标布局；Representation::validate_in、Enum::validate_encoding_in 在 NIR phase、IIR index 与解释器创建时核对同一 target。无目标兼容入口继续拒绝平台表示。布局宽度相等不改变 usize/isize 的类型身份。

新增 `development/examples/host-platform-enum.fl`：以 usize 为整体编码、isize 为载荷，构造 -1，读取位编码并通过 enum pattern 提取载荷。host 测试实际编译并执行 O0/O3，结果均为 true。显式 32/64 位前端测试同时验证 NIR/IIR 执行，以及无目标、声明宽度冲突、修改目标和独立载荷宽度不符的拒绝。bit pattern 符号覆盖分析仍有固定宽度入口，尚待迁移。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test target_integers --test encoded_enums` 22 项通过（`/tmp/luna-tools-70p5ybac/result.json`）；补充独立载荷拒绝检查后目标用例通过（`/tmp/luna-tools-g8k_zq2k/result.json`）。`test -p luna --test host native_platform_enum_payload_encoding` 通过（`/tmp/luna-tools-_ea06weu/result.json`）；`test -p nir --test enum_encoding` 8 项通过（`/tmp/luna-tools-5949s6zp/result.json`）。曾误用 lib 筛选得到零测试，未计作覆盖。全 workspace 格式化退出 0（`/tmp/luna-tools-ztq4m6b1/result.json`），git diff --check 通过；dev 未提交。

### 平台整数位模式覆盖

覆盖检查与 catch 的计步入口现显式传递 target；位投影及整数 literal pattern 的宽度、符号解释使用相同布局。为位模式绑定增加机器整数与字面量的精确相等约束（支持交换比较两侧）；必须先通过类型、目标和字面量范围验证，不将浮点相等当作位相等。运行时模式覆盖仍由覆盖分析处理，不进入 PTC。

32/64 位 usize/isize 的互补 guard 分区能证明穷尽，重复同一条件保留缺失分支诊断；IIR 执行验证两条分支。本机 O0/O3 也执行通过。8 位目标的单元测试枚举全部 256 个输入，对照有符号/无符号边界字面量，验证预算失败恢复及缺少目标时不产生证明。这里不包含平台整数范围模式的一般 interval domain 支持。

工作目录 `/home/fin/luna`：`test -p luna --test target_integers --test host platform_bit_pattern` 2 项通过（`/tmp/luna-tools-omdi9doo/result.json`）；`test -p elaboration --lib coverage` 66 项通过（`/tmp/luna-tools-ao90x8p6/result.json`）；新增 exhaustive byte oracle 单独通过（`/tmp/luna-tools-lyuyjw8d/result.json`）。以上 test 均经 `python3 -B development/check.py --timeout 300 --` 执行。全 workspace 格式化退出 0（`/tmp/luna-tools-de63k_qz/result.json`），git diff --check 通过；dev 未提交。

`python3 -B development/check.py --timeout 300 -- test -p luna --test error_surface --test target_integers` 亦通过（`/tmp/luna-tools-cwpcj_yp/result.json`），覆盖 catch 调用入口改动及完整平台整数测试组。

### 平台整数范围模式

前端的 range ordering 与 negated pattern domain 接纳显式目标上的 usize/isize；interval coverage 和否定覆盖检查通过 domain_in 获取目标的完整整数域。保持半开区间与闭上界的区分，域的右端用任意精度整数表达，不因机器最大值 + 1 而溢出。未设置 target 时不猜测平台范围。

源码回归覆盖 32/64 位有符号和无符号的最小值、最大值、分界值、单点缺口、遗漏最大值，以及否定范围补集；覆盖 32 位全域的模式移至 64 位必须拒绝为不穷尽。IIR 实际执行边界分支。host O0/O3 验证开放端点和否定范围执行结果。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_platform_range_patterns_and_complements` 通过（`/tmp/luna-tools-q9b5l1lk/result.json`）。coverage 筛选验证 67 项 elaboration 单元测试（`/tmp/luna-tools-_5g_vd4_/result.json`）；该筛选没有执行平台测试，因此另行运行 `python3 -B development/check.py --timeout 300 -- test -p luna --test target_integers --test negated_nominal_patterns`，完整两组通过（`/tmp/luna-tools-rubxhhch/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-ecc4jjgj/result.json`），git diff --check 通过；dev 未提交。

### normal-form 平台值输入：目标接纳接口

Head 的 closed/evidence/scoped 参数检查新增显式目标入口；递归整数参数范围检查改用目标布局，旧入口传 None，继续拒绝没有布局证据的平台参数。materialization 与解释器 admission 传入各自 package/interpreter 的 target。该改动不展开 normal-form 头、不以表示等价替代类型身份，也不跳过原有符号声明证据、作用域与预算检查。

NIR 新测试覆盖嵌套 record 内 usize/isize 参数的 32/64 位边界、负数拒绝、零预算失败后恢复，以及同一 Head 在多个目标和无目标间切换不泄漏范围。当前仅完成参数接纳基础：源码 supported_input_type_in_scope 及解释器类型值复制/冻结的 scoped_term_value_work_with_evidence 仍未传递目标，不能声称平台值索引类型函数已端到端可用。后续需在所有类型值保存/恢复路径上传递目标，不能只放宽声明检查。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p nir --test normal_inputs` 15 项通过（`/tmp/luna-tools-_ugk9c_p/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test nominal_staging --test target_integers` 21 项通过（`/tmp/luna-tools-7vuzubfa/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-3zwndzja/result.json`），git diff --check 通过；dev 未提交。

### normal-form 平台输入：源码与类型描述接线

supported_input_type_in_scope_in 将目标传入源码 normal-form、代码生成器输入声明和 NIR 元数据检查。解释器的 type_description_work、实例参数验证及复制接入带 target 的 scoped type/term work；内部 normal-form 头参数检查也使用目标。无目标入口仍保守处理平台声明。

源码测试以 `Tagged<n:usize>` 保留 opaque 头：4294967296 在 64 位可用，32 位及无目标拒绝；经普通 comptime Get 返回类型值、比较同一应用均成功。retained 函数的 IIR 参数/返回仍是相同 NormalForm，不因 Tagged 的函数体返回 bool 而展开。不同实参不直接推出类型不相等；没有为测试引入头的单射假设。这里未完成一般 nominal/trait 值索引 admission 中的所有固定宽度入口，后续仍需分别迁移。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test nominal_staging --test target_integers -p nir --test normal_inputs` 40 项通过（`/tmp/luna-tools-4ozv4kjj/result.json`）。增加 Get 返回路径后单独复验通过（`/tmp/luna-tools-1ttycnbs/result.json`）。`test -p comptime_interpretor --lib computation_type_tests` 经相同包装器执行，6 项通过（`/tmp/luna-tools-700d3ztl/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-fb1hi_kg/result.json`），git diff --check 通过；dev 未提交。

### nominal 平台值索引

nominal_parameters 的 completed/layout/evidence 接口增加显式 target，递归穿过 record、tuple、nominal 与 error payload；无目标兼容入口继续拒绝平台整数值。newtype 的捕获优先验证保留原顺序并传递目标。function declaration 输入、materialization、解释器接纳与 nominal_value_fields 查询接入同一 target；覆盖及 computed pattern 查询也传递目标，避免检查同一 nominal 实例时使用不同范围。

源码/IIR 测试覆盖 usize 索引为 4294967296 的 struct 与 newtype，在 64 位成立、32 位拒绝，实际字段读取和表示转换返回 42。LLVM O0/O3 测试组合 Choice<42> enum 和 Tagged<42> struct，在显式 Copy/Drop 协议下构造、解构并输出 42。递归 record admission 验证目标范围及预算失败恢复。trait_parameters 的独立参数域检查仍需迁移，不把 nominal 接纳完成等同于全部 trait 值参数支持。

工作目录 `/home/fin/luna`，均经 `python3 -B development/check.py --timeout 300 --`：
- `test -p nir --test nominal_parameters --test function_parameters --test normal_inputs`：38 项通过，`/tmp/luna-tools-inaodb0z/result.json`。
- `test -p luna --test nominal_value_parameters --test nominal_staging --test target_integers`：288 项通过，`/tmp/luna-tools-xnqpjkh7/result.json`。
- `test -p luna --test host native_platform_nominal_value_indices`：O0/O3 通过，`/tmp/luna-tools-ryp7of6b/result.json`。
- `test -p luna --test newtypes --test target_integers`：224 项通过，`/tmp/luna-tools-bg8hhv__/result.json`（含新增 newtype 平台索引测试）。

全 workspace 格式化退出 0（`/tmp/luna-tools-n3rfftd7/result.json`），git diff --check 通过；dev 未提交。

### trait 平台值参数

trait 参数检查从传入的 layouts Package 读取 target，并贯穿 scalar、record、tuple/error/nominal 叶子的接纳；候选 implementation 的参数检查采用同一布局。Index/InherentIndex 新增 for_target，布局更新保留目标；源码、materialization、purity world 与 driver implementation world 均显式初始化目标。索引 dump 包含 target-data，不引入目标默认值。

源码测试验证 Read<4294967296> 在 64 位选择并执行泛型实现，32 位拒绝；原生 O0/O3 执行 Read<42> 返回 42。NIR 测试检查 scalar/record 索引的无目标拒绝、32/64 位范围及预算失败恢复。

工作目录 `/home/fin/luna`，均经 `python3 -B development/check.py --timeout 300 --`：
- `test -p luna --test host --test target_integers platform_trait` 两项通过（`/tmp/luna-tools-5t0w84yl/result.json`）。
- `test -p elaboration --test implementation -p nir --test trait_parameters`：67+20 项通过（`/tmp/luna-tools-m8pxbqio/result.json`）。
- `test -p luna --test trait_purity --test target_integers`：target_integers 14 项通过；trait_purity 16 通过、4 失败（`/tmp/luna-tools-dr__g3jl/result.json`）。失败为 pure_trait_contract_is_inherited_by_unmarked_implementations、pure_default_methods_keep_the_contract_when_selected、pure_method_local_type_binders_survive_generic_dispatch、pure_parent_methods_keep_their_origin_through_child_bounds，均报 runtime pure generic/default function lacks P4 evidence。目前未证明这是基线失败或本次回归，不能宣称完整纯度回归通过；需继续定位泛型 trait 调用证据。

全 workspace 格式化退出 0（`/tmp/luna-tools-oar3n07m/result.json`），git diff --check 通过；dev 未提交。

### 泛型 pure trait 调用的资源报告

修复上一节发现的四项 trait_purity 失败：P4 行为分析能够使用接口的 pure 承诺，但移动/清理分析在 record_named_call 中强制选择尚未实例化的 generic trait implementation，导致报告构造提前失败。现在仅在未建立 runtime_instances 且调用签名仍含开放类型输入时，不生成具体实现的子报告；接口与调用方转移/清理检查仍执行，不提供返回闭包或变体的来源证据。实例化后的方法仍必须选择并验证实际实现，不能借此接受未决分派。

四项原有失败全部通过。新增负例确认 pure 方法承诺不能让已移动的泛型值再次使用、丢弃无法证明清理纯度的结果，或把返回的任意函数提升为纯调用。LLVM O0/O3 的泛型 pure 调用和 trait 默认方法链实际执行返回 true。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test trait_purity --test resources --test runtime_purity --test method_memory_contracts` 450 项通过（`/tmp/luna-tools-10sd9fp9/result.json`）。收紧至未实例化阶段后，host 定向测试通过（`/tmp/luna-tools-iivh536v/result.json`；其中 trait_purity 被名字筛选为空，不计覆盖），trait_purity 全组另行复验。临时诊断打印已移除；没有修改 pure 的语义或添加泛型 Copy 假设。

最终 `test -p luna --test trait_purity` 21 项通过（`/tmp/luna-tools-xkkpic55/result.json`）；全 workspace 格式化退出 0（`/tmp/luna-tools-3cwvkus1/result.json`），git diff --check 通过；dev 未提交。

### 平台参数与泛型纯度后的集成核验

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host --test frontend_cli --test packages` 全部通过，报告 `/tmp/luna-tools-62_oj6od/result.json`：host 346、frontend_cli 192、packages 46，共 584 项。该结果覆盖近期平台布局、normal-form/nominal/trait 值参数、泛型 pure 方法修复后的集成状态，不是全 workspace 完成证明。

下一处可执行能力缺口已定位：library/math 只有 fma 泛型包装；nir::intrinsics::Operation 仅 FusedMultiplyAdd，Registry::defer 将参数数目固定为 3，host LLVM Intrinsic 分支也直接生成 llvm.fma。扩展数学函数须同时处理操作身份绑定、按操作的参数/类型协议、comptime 参考语义及 LLVM lowering，不能仅往库中添加未接线的包装函数。现有 --host-fma 选择器仍有效，尚未扩展新操作。

### 一元数学 intrinsic 与当前运行入口复验

Operation 新增 SquareRoot，参数数目由操作协议决定；Registry 的立即检查、延迟绑定和模板重检共用此数目。LLVM 分别生成 llvm.sqrt.f32/f64，保持声明身份绑定、显式输出类型和同精度输入检查。当前只接通后端注册 API；命令行选择器和 library/math.sqrt 尚未接线。目标 intrinsic 的 comptime invocation 仍拒绝，不以 Rust host 浮点运算替代目标行为。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol` 17 项通过（`/tmp/luna-tools-7c3217h5/result.json`），含 sqrt f32/f64 的 O0/O3 原生执行、负零、无穷和 NaN，以及 arity/type 拒绝与既有 FMA 回归。首次测试将调用直接放进 assumes，受当前 predicate 接纳限制而失败；改为先求值再检查局部结果，没有放宽前端。

`build -p luna` 成功但缓存维护随即删除了 CLI，后续按固定路径执行不可用（`/tmp/luna-tools-r53me9m6/result.json`）。已将本页运行步骤改为 cargo 包装器的 run 入口，先执行编译器再维护缓存。`run -p luna -- development/examples/host-arithmetic.fl --build run --output /tmp/flurry-arithmetic` 成功（`/tmp/luna-tools-a7yhg13x/result.json`），生成程序执行退出 0、输出 `(unsigned-integer 42)`（`/tmp/luna-tools-iezskop3/result.json`）。格式检查通过（`/tmp/luna-tools-b4fruekg/result.json`）；dev 工作区未提交。

### sqrt 源码、库与命令行接线

新增 `--host-sqrt package.module.Enum.variant`，传入普通文件、package-set、原生测试与泛型 elaboration。选择器按声明身份绑定；同一 tag 同时指定 FMA/sqrt 时拒绝，既有输出文件保持不变。两种操作共用 family 验证及浮点域证明；不根据源码名字猜测操作，不允许 comptime 调用。

`library/math/lib.fl` 新增泛型 pure sqrt，支持 f32/f64。加载 math 时需同时指定 FMA 和 sqrt 两个绑定，以检查所有泛型函数正文。现有数学示例、library README 和 library_check.py 已同步；host-math-fma.fl 现在联合验证两种运算及两种精度。完整数学库、一般 intrinsic 签名及自动目标注册仍待实现。

工作目录 `/home/fin/luna`，经 `python3 -B development/check.py --timeout 300 --`：
- `test -p luna --test frontend_cli --test host_intrinsic_protocol --test packages`：193 + 17 + 46 项通过（`/tmp/luna-tools-p12ei886/result.json`）。新增 CLI 用例验证任意 tag 名称、泛型源码、O0/O3、原生测试、重复/冲突选择器、错误参数数目/类型和 comptime 拒绝；package 用例通过真实 math 库编译运行。
- `test -p luna --lib protocols::type_constructors`：4 项通过（`/tmp/luna-tools-um_4j3q1/result.json`）。
- `run -p luna -- --packages-v1 --package 1 101 app development/examples/host-math-tests.fl --package 2 202 math library/math/lib.fl --dependency 1 math 2 --primary 1 --host-fma math.target.Scalar.fma --host-sqrt math.target.Scalar.sqrt --test-native --opt-level 3`：原生两项测试通过（`/tmp/luna-tools-r2eq1o57/result.json`），每项同时检查 FMA 与 sqrt。

没有运行全 workspace 测试；dev 工作区未提交。

### 已接纳泛型 intrinsic 的重复绑定

host::intrinsic::bind 先前对所有签名直接运行 concrete check，因此对源码已经证明浮点域的 generic fma/sqrt 重复绑定时报 OutputType。现在 concrete 检查不能接纳开放类型时，只允许 registry.defer 得到与节点原有证据完全相等的 pending binding。不会在 driver pass 里制造新的类型域证明，也不把已清除的证据复活；变更操作、声明、参数类型或签名仍拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol` 18 项通过（`/tmp/luna-tools-r8yun4e0/result.json`）。新增回归对 FMA/sqrt 分别验证零预算失败后恢复、保留源码接纳的重复绑定、f32/f64 O0/O3 原生执行、清空后拒绝重新制造 pending 证据、错误操作及伪造 operand 类型拒绝。全 workspace 格式化通过（`/tmp/luna-tools-4f8n6ea4/result.json`），git diff --check 通过；dev 未提交。

### 实际库验收与资源协议组合

执行 `python3 -B development/library_check.py --timeout 300`：实际声明 9 项、manifest 1 项、native-resources 4 项、native-math 2 项通过；完整 sysroot 阶段失败，总退出 1（`/tmp/luna-library-6ik6ytbu/summary.json`）。失败为实际库中未解析的 panic、String、格式化 Any、allocation/collection/I/O __builtin_* 与 map-storage 等引用，符合 library README 已记录的未实现依赖；没有替换库文件或用占位函数绕过。

随后加强真实数学验收：library_check 的 native-math 同时加载 library/builtin/resources.fl，显式绑定其 Copy/Drop；host-math-tests.fl 的两种精度各检查 sqrt 参数 do 块恰好求值一次。直接以三个显式 package 执行，O3（`/tmp/luna-tools-u4n11hr_/result.json`）和 O0（`/tmp/luna-tools-f0uusnuq/result.json`）均两项通过。命令通过 `python3 -B development/check.py --timeout 300 -- run -p luna -- --packages-v1`，指定 app/math/resources，两个依赖、Copy/Drop、host-fma/host-sqrt 和 --test-native；完整命令保存在上述 result.json。未因这项测试入口更新重跑已知失败的全 sysroot；git diff --check 通过，dev 未提交。

### Symbol 编码 enum 与 runtime tag_name

Host EnumLayout 保留 Symbol 编码文本，私有 LLVM 存储为只读 UTF-8 指针与长度；成员判别按内容比较，不依据地址、不增加整数 tag。bit_width 明确排除这种表示，不能把布局内部占位 width=0 当作 BitVec。普通 enum、固定编码和 Symbol enum 共用声明叶名称表；新增 EnumName 降码，分组返回完整路径，名称字符串存储独立于原 enum。

EnumName 对操作数求值一次，复用临时保留槽与真实 cleanup schedule；增加该槽的 source 准入，不放宽其它临时来源。带拥有型载荷的临时 enum 取名后析构一次；非 Copy 局部重复取名不消费载荷，作用域退出再析构。测试使用实际析构写回和重复析构检测，并检查 O0/O3。分组、固定编码载荷、Symbol 构造/传参/匹配和名字区别均原生执行。示例为 development/examples/host-symbol-enum.fl，使用 --copy-protocol Duplicate --drop-protocol Retire --build run。通用 runtime Symbol 值、raw Symbol 受检构造及 FFI ABI 仍未实现。

工作目录 `/home/fin/luna`：经 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test encoded_enums`，encoded_enums 14 项通过、host 348 项通过而新增非 Copy 取名用例失败（`/tmp/luna-tools-kbp6z1su/result.json`）：将未绑定 Copy 的 str 直接用于 assumes 触发现有 guard 消费限制。测试改为先求得 bool 再检查，没有修改该前端规则；同一用例 O0/O3 复验通过（`/tmp/luna-tools-xkazxrp6/result.json`）。此前调试还修正了测试的固定编码 bool 载荷和 self 指针解引用写法，并接通 EnumName 临时 cleanup 准入。

最终对 host 的 enum 相关用例定向复验，报告 `/tmp/luna-tools-44m589ig/result.json`；没有重复全 workspace 测试。格式化通过（`/tmp/luna-tools-tvunssfl/result.json`），git diff --check 通过；dev 未提交。

### runtime Symbol 值与真实资源库接线

Host 增加独立 Ty::Symbol，和 str 使用相同私有只读文本存储，但保持不同类型；字面量、参数、比较、模式、record/enum 载荷及指针读写均接入。SymbolName 只改变输出视图，不重复求值；提前 return 正常传播。调试入口和产品叶输出使用 `(symbol "...")`，与字符串区别；共用转义输出函数，不复制 UTF-8/控制字符处理。Symbol 编码 enum 的受检 raw 构造仍未实现。

资源库 resources.fl 新增 Symbol 的 Once/Clone/Copy，clone 读取原值；实际库回归覆盖引用执行与原生执行。新增 development/examples/host-symbol.fl，run 返回 true，raw 返回 Symbol。

工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test symbols`：352 + 29 项通过（`/tmp/luna-tools-_h67hksx/result.json`）。新增 O0/O3 用例覆盖值、名称、单次求值、提前 return、指针/record/enum 存储和 Symbol/str 混合产品输出；既有字符串转义与 UTF-8 输出回归全部通过。首次打印共享函数的参数名与局部名冲突已修正，经 LLVM assembler 实际验证。
- `python3 -B development/check.py --timeout 300 -- test -p luna --test library_protocols repository_resource_protocols`：实际库克隆回归通过，含参考执行与 LLVM 执行（`/tmp/luna-tools-afbrp99x/result.json`）。
- 包装器 `run -p luna -- development/examples/host-symbol.fl --build raw --output /tmp/flurry-symbol` 成功（`/tmp/luna-tools-oi9iw284/result.json`）；生成程序退出 0，输出 `(symbol "ready")`（`/tmp/luna-tools-tmjk1pzf/result.json`）。

未重跑完整 sysroot；既有 panic/String/分配/I/O 未实现依赖仍存在。dev 未提交。

### Symbol 状态的拥有型闭包组合验收

新增 development/examples/host-symbol-closure.fl：工厂返回捕获 Session 的闭包，Session 含 Symbol 状态和析构观察指针；调用把 ready 改为 active，再将闭包转交给 close 改为 closed，最终析构检查最终状态和恰好一次清理。该案例跨越函数返回、可变捕获、高阶传参、Symbol 表示与真实资源计划，不需要增加编译器特例。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_symbol_closure` 通过（`/tmp/luna-tools-bbcov886/result.json`）。同一源码经 checked 参考执行和 LLVM O0/O3 均返回 true；插入转交后的再次调用会因 move 失败，已生成的 native program 保持不变。Symbol 的 Copy 不赋予拥有型 Session/闭包隐含 Copy。格式化通过（`/tmp/luna-tools-_j10ig67/result.json`），git diff --check 通过；dev 未提交。

### IIR 名称视图的独立类型验证

IIR validate_expression_types 现在独立验证 SymbolName/EnumName：正常结果必须为 str；SymbolName 输入必须为 Symbol；EnumName 输入必须对应同一完整类型的已发布 enum 实例，查表消耗验证预算。Never 操作数要求 Never 结果，保留提前退出。该检查不依赖 LLVM 的物理布局，不能因为 Symbol/str 同用文本存储而互相替换，也不展开 normal-form 头。

新增伪造 IIR 测试分别替换 operand 为合法 str 字面量、修改结果为 bool；发布验证及参考执行器入口均拒绝。零预算失败后原图可重新验证成功。工作目录 `/home/fin/luna`：
- `python3 -B development/check.py --timeout 300 -- test -p luna --test symbols`：30 项通过（`/tmp/luna-tools-6ragimnw/result.json`）。
- `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test encoded_enums`：353 + 14 项通过（`/tmp/luna-tools-y_mhu6v4/result.json`），覆盖现有运行时名称、提前退出、拥有型临时清理和两种 enum 表示。

格式化通过（`/tmp/luna-tools-gfrfx6_u/result.json`），git diff --check 通过；dev 未提交。

### 名称视图的泛型契约回归

新增源码组合用例：value:T 在 T==Symbol 或 T==Event<bool> 的类型约束下使用 tag_name，泛型 enum 保留 network.ready 的分组名称。资源检查的参考执行与 LLVM O0/O3 一致返回 true，证明 IIR 的具体实例验证不误拒绝这两条合法路径。

无约束 T、以及 T==Identity<Symbol> 的 opaque normal-form 头均拒绝；不展开 Identity 函数体，也不以物理表示获得名称视图。错误构建保留已生成 native program。初始负例过度限定诊断必须含 tag_name；实际 opaque 类型在取得结构视图时以 UnvalidatedModel 拒绝，已将断言覆盖该具体错误，未改编译器语义。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_generic_name_views` 通过（`/tmp/luna-tools-53w2k7ww/result.json`）。格式化通过（`/tmp/luna-tools-byz9xbzv/result.json`），git diff --check 通过；本轮增加回归测试，没有新增编译器特例；dev 未提交。

### 文本常量降码的字节预算

字符串、Symbol 和 enum 名称/编码的文本生成现在先按 UTF-8 字节数加一个声明工作单位扣减 fuel，再编码和发布 LLVM global。使用写入现有 String 的十六进制编码，避免逐字节创建临时 String；enum 路径直接借用名称/编码文本，不先复制整个文本。该计费限制降码工作，不改变 runtime Symbol/str 语义或编码。

工作目录 `/home/fin/luna`：`test -p luna --lib host::llvm::strings` 两项通过（`/tmp/luna-tools-yswormec/result.json`），验证 UTF-8、NUL、引号、反斜杠、换行及空串的精确预算和转义。实际源码生成 4096 字节 Symbol，同一 prepared artifact 在 fuel=1024 时于文本降码明确失败，fuel=100000 时生成并执行成功，重复生成确定性一致（`/tmp/luna-tools-17y2979e/result.json`）。

最终 `python3 -B development/check.py --timeout 300 -- test -p luna --test host --test symbols` 全部通过（`/tmp/luna-tools-5qo45snm/result.json`）：host 354、symbols 31。全 workspace 格式化通过（`/tmp/luna-tools-hkbklr7s/result.json`），git diff --check 通过；dev 未提交。

### char 开发入口输出的类型标记

原生 scalar main 和 product leaf printer 根据逻辑类型将 char 输出为 `(char 码点)`，与参考执行器一致；物理表示仍为 i32，u32 继续输出 unsigned-integer。没有修改字符表示或转换规则。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_character_results` 通过（`/tmp/luna-tools-4_dsz96g/result.json`）。用例在 O0/O3 下对照参考执行检查 ASCII、中文、补充平面字符、NUL、换行，验证 char/u32 混合 record 输出及 checked 入口。该历史验证当时保留 `(record ...)` 外层；当前格式已在上方“字符串结果的可执行入口”所述修复中统一为解释器的 anonymous-record／fields 结构。测试先前使用未绑定的 u32 后缀及错误的整体 record 输出预期，均修正为现行语义；编译器未为测试添加后缀或修改产品格式。

格式化通过（`/tmp/luna-tools-6bvk1n30/result.json`），git diff --check 通过；dev 未提交。

### 产品入口元数据的文本预算

产品输出 Printer 的字段名转义和 printf 格式常量现在按字节扣减 fuel，失败不返回部分 main 模块；编码直接写入字符串，避免逐字节分配。保留原有百分号加倍、S-expression 字符串转义以及 owned nominal 结果准入边界，不扩大隐式清理或打印权限。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --lib host::llvm::entry` 两项通过（`/tmp/luna-tools-z1ecp2be/result.json`），验证长字段名低预算拒绝、提高预算后确定性恢复及百分号/换行/引号/Unicode 编码。`test -p luna --test host result` 经同一包装器执行，17 项通过（`/tmp/luna-tools-www6ewwq/result.json`），包括返回产品、Symbol/char 输出及拥有结果清理。格式化通过（`/tmp/luna-tools-3a3enw3w/result.json`），git diff --check 通过；dev 未提交。

### 错误结果开发入口输出

开发 main 可以直接输出错误结果，以及 record/tuple 内的错误结果。保留逻辑类型和错误行次序，通过 tag 分支只读取活动载荷；Never 成功域不生成可执行成功分支，非法 tag trap。显示格式与参考执行器的 error-result 一致，不改变语言错误 ABI；错误值作为数据输出后 main 仍返回 0。所有候选载荷均须满足现有打印准入，拥有型 nominal 不会因包在错误结果中而隐式丢弃；显式 --discard-result 仍走资源清理。

新增 development/examples/host-error-result.fl：--build run 返回字符成功值，--build missing 返回 Symbol 错误。工作目录 `/home/fin/luna`，通过统一包装器执行：

- `test -p luna --test host result`：20 项通过，含 O0/O3 活动载荷、Never/void 成功域的错误分支、产品字段单次求值、拥有载荷拒绝及 discard 清理，报告 `/tmp/luna-tools-afoayfma/result.json`。
- `test -p luna --lib host::llvm::entry`：两项预算/转义回归通过，报告 `/tmp/luna-tools-_efwxikg/result.json`。
- `run -p luna -- development/examples/host-error-result.fl --build missing --output /tmp/flurry-error-result` 构建成功，报告 `/tmp/luna-tools-jzmbzjj8/result.json`；原生执行退出 0，输出 error 0 的 `(symbol "missing")` 载荷，报告 `/tmp/luna-tools-_j097fvl/result.json`。

全 workspace 格式化通过（`/tmp/luna-tools-jflr10qb/result.json`）。void 成功值的源码注入失败已记入前端检查点，本项没有修订其语义。改动保留 dev 工作区，未提交。

### copysign 泛型数学包装与精确位语义

新增 Operation::CopySign、二实参同精度协议、source selector / Protocols / 单文件与 packages-v1 的 --host-copysign 接线，以及 LLVM llvm.copysign.f32/f64 降码。操作仍由驱动绑定完整声明身份；名为 radical 或 sqrt 的 tag 也可被明确绑定为 copysign，不从拼写推断行为。同一 tag 与 FMA/sqrt 冲突时拒绝。所有实例化、重绑定及资源准备仍须完成原有类型/行为准入，comptime invocation 不新增目标浮点执行权限。

library/math/lib.fl 提供 pure fn copysign(magnitude:T,sign:T)->T，要求 T == f32 or T == f64。结果只替换符号位，保留第一输入的其余位，含 quiet/signaling NaN 及 payload；依据 [LLVM 21 copysign 语义](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-copysign-intrinsic)。不引入 fast-math 或用乘法替代。加载 math 包现在须显式提供三个绑定，README、示例命令和 library_check 已同步。

工作目录 `/home/fin/luna`，本轮检查均通过统一包装器：

- `test -p luna --test frontend_cli --test host_intrinsic_protocol`：194 + 19 项通过（`/tmp/luna-tools-48k_58_n/result.json`）。包含单文件 build/native test O0/O3、非法参数数量/精度、冲突 selector、非法阶段/重复参数、comptime 拒绝、旧产物保留及泛型重绑定的预算失败/恢复与伪造证据拒绝。
- `test -p luna --test host --lib --bin luna`：359 + 42 + 1 项通过（`/tmp/luna-tools-p2etnf86/result.json`）。新增两精度 O0/O3 的精确位验收，涵盖正负零、最小次正规数、无穷、quiet/signaling NaN payload，以及 NaN 作为符号来源；预期由整数位运算计算，不调用宿主浮点 copysign 作为 oracle。
- 真实 math/resources/app 三包、实际 Copy/Drop、host-math-tests.fl 原生执行：O3（`/tmp/luna-tools-gxywk8i5/result.json`）和 O0（`/tmp/luna-tools-e9qz9rg6/result.json`）均两项通过，覆盖 fma/sqrt/copysign 与两个实参按源序各求值一次。完整命令见报告 command 字段。

格式化通过（`/tmp/luna-tools-udca8i3j/result.json`），git diff --check 通过。没有重复已知失败的完整 sysroot 检查；其它数学操作、一般 intrinsic 签名与目标 intrinsic 的 comptime 执行仍待实现。dev 工作区未提交。

### IIR intrinsic 家族实例身份验证

发现并修复 IIR intrinsic 校验只按声明 ID 查找 enum 的缺口：把已发布无参数 Ops 实例的 key 改为 Ops<bool>，旧验证仍接受，尽管当前 intrinsic 协议只支持无参数家族。回归先在旧实现复现错误接受（`/tmp/luna-tools-6v7tmp0s/result.json`，unwrap_err 得到 Ok），不是将既有正确拒绝当成修复。

validate_expression_types 现在要求完整的无参数 nominal key，对每个扫描项扣减验证预算，随后继续执行原有 family、variant、输入输出及绑定证据验证。没有扩大 intrinsic 泛型支持，也没有展开 normal-form 头。新测试确认伪造实例在 IIR 发布和参考执行器入口均被拒绝；合法图在预算失败后可重新验证。

工作目录 `/home/fin/luna`：统一包装器 `test -p luna --test host_intrinsic_protocol` 20 项通过（`/tmp/luna-tools-t04xyl7c/result.json`）；`test -p iir -p luna --lib --test host` 50 + 42 + 359 项通过（`/tmp/luna-tools-u6k5s1sl/result.json`），覆盖实际 LLVM 原生运行与既有 IIR 验证。格式化通过（`/tmp/luna-tools-o2mlh4mu/result.json`），git diff --check 通过；dev 未提交。

### void 成功值与错误结果

修复此前记录的 !E void 成功注入缺口，依据 docs/type/qualified-types/error-type.md 的普通成功注入规则。block 无显式尾值和裸 return 原来直接要求 void 等于错误结果，因此被拒绝；现在通过统一 implicit_error_success 生成 Unit 与显式 Success ErrorConvert。转换节点仍取得独立生成 source，不把逻辑错误结果冒充物理 void。已分歧的 block 不添加成功尾值；无 else 的 if 先保持原先丢弃分支值/交付 void 的行为，再注入外层目标。

前端回归在旧实现复现 unresolved type equality（`/tmp/luna-tools-trp2m7lt/result.json`），修复后覆盖空函数体、嵌套空 do、裸 return、无 else 条件、两个空分支及成功/失败混合分支；空成功值进入 !E bool 和 !E Never 仍拒绝。host 用真实拥有 Held 的 Drop 写回观察值，验证落尾成功、提前成功和错误提前返回的恰好一次清理，参考执行与 LLVM O0/O3 一致。测试初版的 `_!=>` 被词法解析为 !=，以及在析构谓词中直接观察指针槽的既有限制，分别通过空格和先读取 bool 再检查修正；未放宽解析或资源规则。

工作目录 `/home/fin/luna`，统一包装器 `test -p luna --test error_surface --test host --test frontend_cli`：118 + 360 + 194 项通过（`/tmp/luna-tools-fytzg10_/result.json`）。格式化通过（`/tmp/luna-tools-cq5hb8k3/result.json`），git diff --check 通过。示例 host-error-result.fl 新增 completed 入口：`run -p luna -- development/examples/host-error-result.fl --build completed --output /tmp/flurry-void-error` 构建通过（`/tmp/luna-tools-v5dnb7j6/result.json`），原生输出成功分支的 `(unit)`，退出 0（`/tmp/luna-tools-d1g_vwz3/result.json`）。dev 未提交。

### void 错误传播与 catch 退出的组合验收

新增 development/examples/host-void-error-flow.fl。validate 通过空尾返回 !InputErr void 成功或 Rejected 错误，forward 用后缀 ! 传播失败；handle 保留拥有型 Lease，按前置 catch、普通 arm、后置 catch 顺序处理错误。三阶段分别可裸 return；scenario 观察 continued、before/arm/after、handler 后 finished 以及析构 released 标记。run 覆盖成功/失败与四种退出模式的八个组合，返回 true。

该示例不需要新编译器特例。输入成功跳过全部 catch；传播失败不继续 forward；早退跳过后续阶段和 handler 之后代码，但 Lease 仍清理一次。析构再次执行会触发检查，漏执行则最终结果为 false。没有依靠普通算术计数而引入无关范围证明。

构建方式：

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-void-error-flow.fl --copy-protocol Duplicate --drop-protocol Retire --build run --output /tmp/flurry-void-flow
python3 -B development/command.py --timeout 10 -- /tmp/flurry-void-flow
```

工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_void_error` 三项通过（`/tmp/luna-tools-uzvi1ih2/result.json`），包括上述真实示例的资源检查参考执行和 LLVM O0/O3，以及独立传播/catch 顺序回归、上轮成功注入清理回归。初版测试的 `continued.*=true` 被词法识别成复合运算符，修正为空格分隔后通过，没有改变 lexer。格式化通过（`/tmp/luna-tools-ja66vbxi/result.json`），git diff --check 通过；本轮新增示例与回归，未改编译器，dev 未提交。

### 可重复的通用 host intrinsic 绑定

新增 `--host-intrinsic operation=package.module.Enum.variant`，当前 operation 为 fma/sqrt/copysign。单文件与 packages-v1 的 build、emit-llvm、test-native 均接通；旧 --host-fma/--host-sqrt/--host-copysign 保持有效。Selectors 和 Protocols 保留通用声明/操作集合，统一 host_bindings 供 elaboration 与绑定校验消费；后续增加操作不再要求逐项增加所有 action/frontend 字段。

同一操作可以绑定多个包中的不同 tag；同一已解析声明的相同操作重复绑定幂等，旧/新写法混合亦然。相同声明绑定冲突操作明确拒绝，不能由参数次序覆盖。仍以完整 package/declaration/variant 身份取得行为，不从源成员拼写推断。语法 dump/参考测试仍不接收 host-only CLI 选项；不新增 comptime 目标执行权限。

工作目录 `/home/fin/luna`，统一包装器 `test -p luna --lib --bin luna --test frontend_cli --test host_intrinsic_protocol`：42 + 1 + 196 + 20 项通过（`/tmp/luna-tools-s_lpr2vh/result.json`）。新增两包各声明独立 Ops::root 的泛型源码，通过新参数同时绑定 sqrt，O0/O3 build 与 native test 均执行成功；LLVM dump 同时保留 f32/f64 调用，重复绑定不改变文本。非法格式、未知操作/包/成员、实参数量、阶段错误及旧/新冲突全部拒绝并保持原可执行产物。

真实 math/resources/app 三包全部改用新参数执行 O3，fma/sqrt/copysign 两精度及求值顺序测试通过（`/tmp/luna-tools-8tgrilez/result.json`，完整 command 在报告中）。library_check 的 native-math 入口和 library README 已同步。格式化通过（`/tmp/luna-tools-pci2lcpd/result.json`），git diff --check 通过。没有重复已知失败的完整 sysroot 检查；dev 未提交。

### 四种浮点取整操作

增加 Floor/Ceiling/Truncate/RoundEven 的具体/延期签名校验、S-expression 操作标签、通用 CLI 绑定和 LLVM floor/ceil/trunc/roundeven.f32/f64 降码。统一一实参、同精度输出协议；仍要求源级泛型证明 T==f32 or T==f64，重绑定不能重建已清除的证明或移用其它操作证据。没有新增四组 action/frontend 字段，复用 --host-intrinsic。

library/math 提供 floor、ceil、trunc、roundeven 的 pure 泛型包装，结果仍是输入浮点类型，不是整数转换。前三者方向分别为负无穷、正无穷、零；roundeven 为最近值且中点取偶数，不依赖 runtime 舍入模式。依据 [LLVM 21 rounding intrinsic 契约](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-roundeven-intrinsic)。保持 signed zero 和无穷；NaN 只检查仍为 NaN，不承诺 payload 保留。所有操作均无 fast-math。

math 包现在需绑定全部七个操作（包括未调用的泛型正文）；library README、library_check、相关示例及真实库回归已同步。

工作目录 `/home/fin/luna`：

- 统一包装器 `test -p luna --test frontend_cli --test host_intrinsic_protocol`：196 + 21 项通过（`/tmp/luna-tools-ssso14fu/result.json`）。四种操作纳入泛型两精度实例化、重绑定预算失败/恢复与伪造证据拒绝，补充零/多参数、不同精度及错误输出类型的拒绝例。
- `test -p luna --test host --lib`：363 + 42 项通过（`/tmp/luna-tools-l407vlqu/result.json`）。新增每种操作/精度的 O0/O3 精确位测试：正负半整数、±0、最小正负次正规数、±无穷与两类 NaN。有限数预期是明确列出的数值，不调用宿主取整算法作为 oracle。首轮测试 Rust 常量未标 u64 导致测试编译失败，修正后原生定向验收及整组均通过。
- 真实 math/resources/app 三包、实际 Copy/Drop 和 host-math-tests：O3（`/tmp/luna-tools-4jcbjfup/result.json`）与 O0（`/tmp/luna-tools-r4zpfli7/result.json`）均两项通过。各新函数的实参含可观察状态修改，确认每个实参只求值一次；完整命令见报告。

格式化检查通过（`/tmp/luna-tools-fv2o_ix8/result.json`），git diff --check 通过。未重跑完整 sysroot；其它数学操作和目标 intrinsic 的 comptime invocation 仍未实现。dev 未提交。

### 独立比较协议与 coherence 预算分配

将现有 Eq/Ord/Ordering 声明迁入 library/builtin/comparison.fl，traits.fl 重导出这些真实声明，保持经 traits/builtin 根访问的路径。独立模块无 String、panic 或分配依赖；补充 bool 与 u8/u16/u32/u64/u128、i8/i16/i32/i64/i128 的 Eq/Ord 实现。bool 使用 false < true；没有假设浮点满足全序，也没有补上未经验证的平台宽度实现。

组合真实 comparison/resources 源码时发现 coherence 的总预算被平均分给所有实现对，包括不同接口的廉价排除项；合法资源实现因此报无法证明不重叠（`/tmp/luna-tools-0uz9dqqa/result.json`）。现在先为完整成对遍历/报告保留预算，只给同接口、同 precision 的真实证明义务分配求解预算。不同接口与不同 precision 使用 Index 已验证缓存信息，check/replay 共用相同分配；仍检查完整报告、世界 freshness 和每棵 disjointness 证明。新增 30 个无关接口的回归确认合法证明不再被稀释；低预算、真实重叠和旧世界报告继续拒绝。

比较测试使用实际库正文，验证所有支持标量的最小/最大值、eq/ne/cmp 及 Ord 默认方法；每种类型独立入口，均 checked 参考执行和 LLVM O0/O3 通过。最初将整组放在一个入口导致参考执行预算耗尽，已按类型拆分，未放宽运行预算；调用先绑定 bool 再 assumes，保留当前谓词语言限制。新增 host-library-comparison.fl，以真实多包 Ord+Copy 约束演示泛型 lower 和三项 source test；O3 通过（`/tmp/luna-tools-2geq0eka/result.json`），O0 纳入 library_check。

工作目录 `/home/fin/luna`：统一包装器 `test -p elaboration -p luna --test implementation --test library_protocols --test frontend_cli`：68 + 10 + 196 项通过（`/tmp/luna-tools-uocxv91l/result.json`），包含原 traits 路径重导出和自定义 Key 默认方法回归。格式化通过（`/tmp/luna-tools-ll1usyop/result.json`），git diff --check 通过。

本轮实际运行 `python3 -B development/library_check.py --timeout 300`（`/tmp/luna-library-arcv85sc/summary.json`）：declarations、manifest、native-resources、native-comparison、native-math 通过；sysroot 仍因 panic/String/Any、collections todo 与分配/map/I/O hooks 未定义失败，整体退出 1。README 和入口已同步，不将独立模块可用称为完整 sysroot 可用。dev 未提交。

### 参考执行的清理站点索引

将 checked 参考执行的 cleanup_site 从每个表达式边界扫描整个函数的 sites，改为在验证并接纳清理 schedule 后按 source range 建立借用索引。索引保留所有同位置站点及原顺序；实际执行仍先选择准确的 instance/context，再匹配站点所属 schedule 与 boundary，不合并不同闭包、默认值或效应上下文。索引构造、候选复制和动作执行继续计入预算，资源动作与动态 pattern 析构顺序保持原检查。

comparison/resources 的 11 种标量边界检查增加单函数 check_all，保留原逐类型入口。逐类型预算仍为 100000；完整函数使用 500000。同预算对照：恢复旧扫描时 check_all 因 StepsExhausted 失败（`/tmp/luna-tools-1_ny2p6m/result.json`），索引版 checked 参考执行及 LLVM O0/O3 均通过（`/tmp/luna-tools-p69ayo9x/result.json`）。完整函数在 100000 下仍不足（`/tmp/luna-tools-1pavywrj/result.json`）；此次只消除站点扫描的重复工作，不宣称所有图验证开销已解决。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test resources --test host --test library_protocols` 退出 0（`/tmp/luna-tools-i2n7aq4i/result.json`）。全 workspace 格式化退出 0（`/tmp/luna-tools-gwxct500/result.json`）；git diff --check 通过。改动保留 dev 工作区，未提交。

### 字符与 Symbol 比较协议

library/builtin/comparison.fl 新增 char 的 Eq/Ord 和 Symbol 的 Eq。字符按现有 Unicode scalar 比较语义排序；Symbol 按名字相等，不增加没有定义的 Symbol 排序协议。真实库测试覆盖零字符、ASCII、汉字、补充平面字符、cmp 三种结果、默认比较方法，以及跨函数的 Symbol 相等/不等；checked 参考执行与 LLVM O0/O3 通过。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test library_protocols --test frontend_cli` 退出 0，11 + 196 项通过（`/tmp/luna-tools-ouylyutk/result.json`）。host-library-comparison.fl 增加 unicode_characters 和 symbolic_names；实际 app/comparison/resources 三包 CLI 的 O3 原生测试共 5 项通过（`/tmp/luna-tools-ivdo0qqq/result.json`，完整命令见报告）。library_check 的预期计数与 README 已同步；脚本语法检查、格式化与 git diff --check 通过。未重跑全 sysroot。

尝试将 str 内容比较封装成 Eq 时，真实调用被现有内存验证层拒绝：str 缺少底层字符存储的 provenance 表示（`/tmp/luna-tools-o9939t8i/result.json`）。没有用 Copy 或平凡析构绕过此限制，最终不发布 str Eq 实现；续接字符串库前须先完成字符串视图的存活/来源契约。

### 平台整数比较的声明与目标边界

comparison.fl 新增 usize/isize 的 Eq/Ord 实现。前端比较判定现在允许已定型的平台整数形成 ScalarEqual/ScalarLess 等 NIR，不要求在声明检查阶段选定机器宽度；没有将它们替换成 u64/i64，也没有默认使用当前进程的平台。数值字面量的范围验证、解释器实际参数检查和 host 布局仍需要明确 TargetData。

新增 target_integers 回归覆盖六种比较操作的无目标声明与无目标调用拒绝，以及 32/64 位真实库实现的有符号上下界、无符号最大值、cmp 和默认比较方法。64 位用例在 32 位目标编译时拒绝溢出；与 LLVM host 匹配的目标同时执行 O0/O3。host-library-comparison.fl 增加平台整数泛型 lower 示例，library_check 计数更新为 6，README 同步。

工作目录 `/home/fin/luna`：统一包装器 `test -p luna --test target_integers --test library_protocols`，16 + 11 项通过（`/tmp/luna-tools-anj4fvd1/result.json`）；`test -p luna --test frontend_cli --test host`，196 + 363 项通过（`/tmp/luna-tools-stggb0zy/result.json`）。真实 app/comparison/resources 三包 CLI O3 的 6 项 source test 通过（`/tmp/luna-tools-msvl1jp4/result.json`）。格式化退出 0（`/tmp/luna-tools-mxx7ka9b/result.json`），git diff --check 通过。改动未提交；字符串视图仍待接续完整 provenance 表示，具体边界已记录在 frontend-checkpoint.md。

### 指数、对数、三角函数与浮点幂

新增 Operation 与通用 --host-intrinsic 绑定 sin/cos/exp/exp2/log/log2/log10/pow。前七项接受一个同精度浮点输入，pow 接受两个；NIR 的 concrete/pending admission、重新绑定与 LLVM 降码共享明确操作身份。math/lib.fl 提供具有 f32/f64 类型约束的泛型 pure 包装；不增加名称驱动的隐式识别，不请求 fast-math/afn，也没有加入参考解释器或 comptime invocation 的替代计算。对应语义见 LLVM 21.1 LangRef 的 math intrinsic 条目（链接已放入 library/README.md）；不承诺跨平台超越函数结果逐位一致或 NaN payload 保留。原有链接路径已链接 libm，无需改变依赖结构。

测试扩展：所有新操作加入泛型 admission/rebind 的低预算恢复、清除 admission 后拒绝重建、错操作与伪造输入拒绝；一元操作和 pow 分别验证参数数量、同精度输入与输出。native_transcendental_intrinsics_preserve_special_value_classes 对 f32/f64 和 O0/O3 检查 signed zero、无穷、NaN、log 的零/负数域以及负底数/负指数的 pow；NaN 仅检查类别。真实库 source tests 用有副作用的普通参数验证各实参一次及 pow 的左右顺序，有限非平凡值采用明确区间验收。

工作目录 `/home/fin/luna`：`test -p luna --test host_intrinsic_protocol --test frontend_cli`，22 + 196 项通过（`/tmp/luna-tools-4xs2pu40/result.json`）；新增特殊值回归后的 `test -p luna --test host_intrinsic_protocol` 23 项通过（`/tmp/luna-tools-rf3wy74o/result.json`）。真实 app/math/resources 三包、完整十五项绑定、Copy/Drop 的两个 source tests 在 O3（`/tmp/luna-tools-esbvgb5m/result.json`）及 O0（`/tmp/luna-tools-jnxchx0o/result.json`）均通过。完整命令见各报告；README 命令、CLI 帮助、library_check 和既有 CLI 验收绑定列表均同步。格式化退出 0（`/tmp/luna-tools-3w2rd3si/result.json`），git diff --check 与 library_check Python 语法检查通过。未运行全 sysroot；dev 未提交。

### Never 左操作数的逻辑操作域

修复前端 logical_expression_step：左操作数不交付值时，不能从 Never 默认选定 bool。现在保留已检查的左侧前缀，先尝试由右侧自身类型确定 Bool/BitVec 域；只有右侧尚需上下文（例如未定宽整数掩码）时才使用外层目标。整个表达式仍为 Never，不把外层 bool 返回类型强加给已有 BitVec 类型的右侧。原布尔短路类型前提与普通 BitVec 求值规则保留。

新增 pure_bitvec 回归覆盖 and/or 的裸掩码、do 掩码、函数提供 BitVec、无 BitVec 外层标注、连续左结合运算、双方提前退出以及布尔短路。参考执行的调用事件确认右侧函数没有执行；NIR/IIR 参考执行及 LLVM O0/O3 均通过。拒绝例保留非 Bool/BitVec 域和可完成左侧的 bool/BitVec 混用。

最初修复只传递上下文时，无外层 BitVec 标注仍因 bool/BitVec 类型冲突失败（`/tmp/luna-tools-3zfbgvtf/result.json`）；采用右侧自身类型优先后修复。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test pure_bitvec --test host --test frontend_cli` 退出 0，21 + 363 + 196 项通过（`/tmp/luna-tools-k0iq2exs/result.json`）。格式化退出 0（`/tmp/luna-tools-aefhk9_0/result.json`），git diff --check 通过。frontend-checkpoint 对应旧缺口已标记修复；dev 未提交。

### Runtime 退出块与调用签名的边界

block_context_with_input 对已经确定退出的普通 runtime 块，先在无结果上下文的事务中检查独立定型的尾部；例如 `fn run()->bool {return true; unused()}` 不再要求返回 u8 的 unused() 满足 bool 结果域。不能独立定型的尾部仍取得原上下文。尾部仍被静态检查，实际执行不进入它；整个块保持 Never。此分支不适用于 comptime/类型形成、展开包准备或结构输入上下文，它们仍需检查其声明域与字段布局。

调查同时确认，完全为 Never 的 callee 没有可供实参检查的函数签名；现改为准确的缺失签名诊断，未从不可达函数尾部恢复一个假签名，也未跳过实参检查。新回归验证普通块、嵌套 do、let 输入及函数实参中的提前退出；NIR/IIR 调用事件仅进入 run，LLVM O0/O3 结果一致。未解析名称、实参错误、缺失 unsafe 边界、可正常完成分支的错误结果类型仍拒绝。

扩大检查曾发现两个实际回归：展开枚举包的错误字段检查被跳过（`/tmp/luna-tools-gupiif1c/result.json`），类型列表输入丢失形成上下文（`/tmp/luna-tools-hlhbtwhl/result.json`）。最终分别保留展开/结构输入约束，并将新分支限定为 runtime_value_context；未修改原拒绝例来适应实现。

最终工作目录 `/home/fin/luna`，`python3 -B development/check.py --timeout 300 -- test -p luna --test extended_calls --test default_calls --test spread_control --test pure_bitvec --test error_surface --test staged_aggregate_contracts --test contextual_spreads --test type_spreads` 退出 0，共 354 项通过（`/tmp/luna-tools-y6wyamll/result.json`），包含新增 LLVM O0/O3 验收和上述两个回归。格式化检查见 `/tmp/luna-tools-jpyqkfkb/result.json`；git diff --check 通过。前端检查点已更正，dev 未提交。

### 零大小值写回的求值与存活验收

复查发现 frontend-checkpoint 中的 void 指针写回阻塞已经由现有实现解除，当前 intrinsic_pointer_value 包含 Void，host 也已有基本写回案例。本轮不重复修改编译器，补充有可观察状态的回归：locate 修改左侧标志，produce 检查它且检查右侧此前未执行，再修改右侧标志；写回必须完整保留两者的先后顺序与一次求值。增加右侧提前 return 的版本，确认地址已计算、后续 producer 未调用。

局部零大小对象及嵌套块内对象的地址逃逸，参考执行准备和 native 构建均必须以 local memory target cannot escape 拒绝；测试检查具体诊断及没有生成 native program，不能因为物理存储长度为零就消除存活义务。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host zero_sized_pointer` 退出 0，三项通过（`/tmp/luna-tools-r8pgib6_/result.json`），新正例各经资源检查的参考执行和 LLVM O0/O3。全 workspace 格式化退出 0（`/tmp/luna-tools-8py2t5rj/result.json`），git diff --check 通过。前端检查点已更正；本轮产物为测试与状态修正，未声称新增后端能力，dev 未提交。

## Unsafe 模板与 quote 调用边界收尾

签名模板实例继承声明的 unsafe 标志；quote 展开前先检查原调用者的词法许可和 quote 主体，避免删除 Call 后绕过责任检查。普通 unsafe quote fn 也沿同一路径接通；unsafe comptime fn 仍保留现有拒绝。参考解释器的 IIR callable 缓存统一采用完整 callable_type，修复合法 unsafe 函数作为高阶实参时的类型不匹配。

回归包含普通、inline、quote 三种模板实例、继承 pure/unsafe 的组合、调用方缺少 unsafe 块、将 unsafe 函数传给普通函数参数、嵌套 quote 调用和声明级 unsafe 许可。模板正例经过参考执行与 LLVM O0/O3 原生执行；拒绝例不能靠展开消除。未绕过内存、资源或程序规格检查。

工作目录 `/home/fin/luna`：

- `python3 -B development/check.py --timeout 300 -- test -p luna --test test_templates --test quote_functions --test pure_callbacks --test frontend_cli` 退出 0，分别 81、104、80、196 项通过，报告 `/tmp/luna-tools-w6rx89sd/result.json`。
- `python3 -B development/check.py --timeout 300 -- test -p luna --test test_templates --test quote_functions --test host unsafe` 退出 0，14 项定向检查通过，报告 `/tmp/luna-tools-xuq0vrl6/result.json`。
- `python3 -B development/check.py --timeout 300 -- test -p comptime_interpretor --test runtime_function_identities` 退出 0，15 项通过，报告 `/tmp/luna-tools-loobcbad/result.json`。
- 全 workspace 格式化退出 0，报告 `/tmp/luna-tools-ypwtwd52/result.json`；`git diff --check` 通过。未运行全 workspace 测试，dev 改动未提交。

最初的 quote 正例暴露 AST lowering 仍拒绝 unsafe quote fn（`/tmp/luna-tools-z19x8tpn/result.json`、`/tmp/luna-tools-27cmg_hg/result.json`）；上述成功记录来自接通展开前检查并解除该项 pending admission 之后。

本轮重新执行仓库算术示例：`python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-arithmetic.fl --build run --output /tmp/flurry-host-arithmetic-checked` 退出 0（`/tmp/luna-tools-px3p9rhv/result.json`）；`python3 -B development/command.py --timeout 10 -- /tmp/flurry-host-arithmetic-checked` 退出 0，实际 stdout 为 `(unsigned-integer 42)`（`/tmp/luna-tools-7pr8kcb7/result.json`）。产物经过真实 LLVM 降码、目标文件生成和链接，不是将解释执行结果写成常量程序。

## 固定编码 enum 的零宽载荷

共享 Variant 编码现在允许 BitVec<0> 字段；它仍占一个具名逻辑载荷槽，构造必须提供该值，但不占编码位。重复名字和超出整体宽度的起点仍拒绝；整体 enum 编码继续要求非零宽度。LLVM 构造先执行字段并保留构造状态，再跳过空位合并；提取返回空值，不生成 i0、也不在字段位于最高端时移位整个编码宽度。

新增源码回归覆盖开头、中间、末尾的零宽字段，其求值副作用、按模式提取、原始编码保持，以及空字段求值提前 return 时跳过后续字段。资源检查开启，O0/O3 均实际编译、链接并执行成功。NIR 编码回归验证三个空字段的构造／提取、错误实参数量、重名和越界拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_fixed_enum` 退出 0，9 项通过（`/tmp/luna-tools-ub7_9tfv/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p nir --test enum_encoding` 退出 0（`/tmp/luna-tools-hnsjha_0/result.json`）。格式化退出 0（`/tmp/luna-tools-60hwbfog/result.json`），git diff --check 通过。先前源码用例分别修正位模式的 0b 前缀和 .* 后赋值空格；合法输入的基线拒绝为 enum payload has zero width or duplicate name（`/tmp/luna-tools-ie3fe2bj/result.json`）。未执行全 workspace 测试，dev 未提交。

## 独立 host 目标文件接口

新增 Rust API `luna::host::object(ir, &toolchain) -> Result<Vec<u8>, String>`，返回本机可重定位目标文件。它与 executable 共用 LLVM 汇编、目标布局核对、优化、逐 pass 验证及 PIC 目标生成路径；不运行链接器。executable 在同一私有 workspace 中复用该路径后再链接，避免两套后端行为分叉。返回字节由调用方负责保存与原子发布。

此接口保留输入 LLVM 中的符号与开发入口适配器。它不是 Flurry FFI/export 实现，也不将内部函数 ABI 变成公开承诺；多个带 main 的开发模块不能因此直接合并。外部链接器须提供所需运行库，例如 LLVM math intrinsic 降为 libm 时需要 -lm。CLI 的独立 object 动作已在下一节接通；构建缓存集成仍待实现。

验证：从真实 host-arithmetic.fl 经 --emit-llvm 取得模块，在 linker 指向不存在路径时分别以 O0/O3 生成 object，再交给实际 C 链接器，执行输出 (unsigned-integer 42)。另用外部符号模块确认 object 生成允许未解析符号，无提供者时链接失败，补上独立 C 提供者后链接运行成功。非法 LLVM 和冲突目标布局仍拒绝。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host native_object` 退出 0，2 项通过（`/tmp/luna-tools-80nr4lax/result.json`）；`python3 -B development/check.py --timeout 300 -- test -p luna --test host shipped` 退出 0，31 项仓库示例回归通过（`/tmp/luna-tools-h85jqvtt/result.json`）。格式化退出 0（`/tmp/luna-tools-9638cka6/result.json`），git diff --check 通过。未运行全 workspace 测试，dev 未提交。

默认参数的非局部 return 拒绝保持不变：docs/functions/parameter-elaboration.md 明确规定默认值不允许 return，不能因 LLVM 能发出分支而解除前端规则。

## --emit-object 命令与包集合交付

单文件和 --packages-v1 均接通 `--emit-object <entry> --output <path>`。HostOutput 显式区分 LLVM 文本、object 和 executable；包集合仍复用 Build 的二进制接口，以 object 标志区分产物，Request::build 返回完整字节。object 不运行链接器、不追加文本换行、不主动增加可执行权限；发布复用现有原子输出和源文件／别名保护。未指定输出、与其它动作冲突、请求 --with-inputs 均拒绝。资源协议、intrinsic 绑定、结果丢弃与可选内联沿同一 host 编译路径处理。

```sh
python3 -B development/check.py --timeout 300 -- run -p luna -- development/examples/host-arithmetic.fl --emit-object run --opt-level 3 --output /tmp/flurry-arithmetic.o
python3 -B development/command.py --timeout 30 -- cc /tmp/flurry-arithmetic.o -lm -o /tmp/flurry-arithmetic
python3 -B development/command.py --timeout 10 -- /tmp/flurry-arithmetic
```

这仍交付带开发 main 适配器的本机模块，不是跨目标构建或源级 FFI/export；不能将多个这样的入口模块直接合并。

本轮工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 300 -- test -p luna --test host emit_object` 退出 0，3 项通过（`/tmp/luna-tools-hxcmry4z/result.json`），实际覆盖两种 CLI 输入、O0/O3、缺失链接器仍生成 object、独立链接执行、非可执行权限、参数冲突、旧产物保留、直接输入／硬链接别名／依赖包源码保护及 with-inputs 拒绝。`python3 -B development/check.py --timeout 300 -- test -p luna --lib --bin luna --test frontend_cli --test packages` 退出 0，42+1+196+46 项通过（`/tmp/luna-tools-73tf43qv/result.json`）。格式检查退出 0（`/tmp/luna-tools-w20qgpdg/result.json`），git diff --check 通过。未运行全 workspace 测试，dev 未提交。

### Object 的资源与 intrinsic 组合验收

新增端到端回归确认 --emit-object 并未丢弃已有 host 选项：真实 FMA 示例通过 --host-intrinsic 绑定，原生结果保持 f32 位模式 41880000；拥有型闭包示例启用 Copy/Drop 与 --inline-functions 后，其捕获环境仍正确清理。两者均在 LUNA_CC 指向不存在路径时生成 object，交给实际链接器后以 O0/O3 执行。

显式包集合使用非零 semantic identity，开启 --discard-result、资源协议与可选内联，入口交付的拥有值仍会析构。析构谓词成立时输出 (unit)，不成立时原生进程失败且不输出成功值；没有因分离链接而忽略资源失败传播。

本轮仅扩展验收，没有更改语言语义或后端实现。工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host emit_object_` 退出 0，5 项通过（`/tmp/luna-tools-3vvkzwsp/result.json`）。格式化退出 0（`/tmp/luna-tools-4gymc6ws/result.json`），git diff --check 通过。未重复全 workspace 测试，dev 未提交。

## 浮点绝对值 intrinsic 与 math.abs

Operation::AbsoluteValue 通过 --host-intrinsic fabs=package.Enum.variant 绑定，保留身份检查、同宽 f32/f64 单操作数与同类型结果、pending generic admission、预算及重绑定规则。Display 为 fabs；LLVM 发出 llvm.fabs.f32/f64。library/math 的 abs<T> 是此 intrinsic 的 pure 泛型包装，限制 T 为精确 f32 或 f64；math 包现在需提供十六项绑定，库检查脚本、CLI 回归和 README 命令已同步。

契约遵循 [LLVM 21.1 fabs](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-fabs-intrinsic)：只清除符号位，NaN 的 quiet/signaling 位及 payload 不变。不通过普通算术取负或比较来实现，不添加 fast-math。原生回归覆盖两种宽度、O0/O3、正负零、负数、最小非正规数、负无穷和两类 NaN；另覆盖一次求值、泛型特化、错误输入、低预算及重绑定恢复。真实库包装的调用与 CLI 包绑定均已执行。

工作目录 `/home/fin/luna`：frontend_cli 196 项通过（`/tmp/luna-tools-zumcha7r/result.json`）；该次组合调用中既有 FMA 测试遇到 ExecutableFileBusy，故不记组合调用成功。随后 `python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol` 退出 0，24 项全通过（`/tmp/luna-tools-zbzv82mv/result.json`）。真实 app/math/resources 三包通过 --test-native、--opt-level 3、资源协议及全部十六项 --host-intrinsic 绑定执行，两项测试通过（`/tmp/luna-tools-65c8w2tr/result.json`，完整命令见该报告）。格式化退出 0（`/tmp/luna-tools-h7dr5s0i/result.json`），git diff --check 通过。解释器 intrinsic 执行、泛型浮点字面量及完整 sysroot 仍待接通。dev 未提交。

## LLVM round 与跨包泛型常量组合

增加 RoundAway 操作、NIR arity/domain/rebinding 验证、S-expression operation 名称 round、CLI `--host-intrinsic round=...` 和 `llvm.round.f32/f64` 降码。`math.round<T>` 保持 LLVM intrinsic 泛型包装约定，中点远离零舍入，结果仍为原浮点类型；`roundeven` 的中点取偶不变。依据 LLVM 21 的 [round 契约](https://releases.llvm.org/21.1.0/docs/LangRef.html#llvm-round-intrinsic)。没有引入浮点转整数或 fast-math。

math 包现需绑定全部十七项操作，library README、真实包 CLI 测试和 library_check 入口已同步。host-math-tests.fl 新增两个精度的 round/roundeven 对照、负零及一次求值案例，还加入消费者侧纯泛型 positive_magnitude：调用 math.copysign<T>(value, 0.0)。这验证此前泛型浮点常量限制已在真实跨包且启用 Copy/Drop 的环境中解除，不以改写库函数语义绕过。

工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 300 -- test -p luna --test host_intrinsic_protocol --test frontend_cli` 退出 0，25+196 项通过（`/tmp/luna-tools-mh9uf26r/result.json`）；新增原生测试涵盖 f32/f64 的正负中点、非中点、正负零、无穷和 NaN（只要求 NaN 类别），O0/O3 都实际运行。真实 app/math/resources 包经 cargo.py run、十七项绑定、配对 Copy/Drop、`--test-native --opt-level 3` 执行，四项全部通过（`/tmp/luna-tools-t659yw1x/result.json` 内保存完整命令及 stdout）。格式化退出 0（`/tmp/luna-tools-nwy3_ivv/result.json`），git diff --check 通过。dev 未提交；未运行全 workspace 测试。

## 编译工具调用的时限与输出边界

host 工具调用不再通过无界 Command::output 等待：llvm-config、llvm-as、opt、llc、链接器的每次调用都有独立 120 秒截止时间，stdin 关闭，两个输出通道通过非阻塞读取轮询。stdout 最多保留 64 MiB，超出时整个步骤失败，不能把截断的 LLVM 模块或目标描述交给后续阶段；stderr 最多保留 16 KiB，继续排空并在失败诊断中标记截断，控制字符转义。失败／超时会终止并回收直接工具子进程。默认 native test 运行截止时间仍独立于编译时间。

这不替代 development/check.py 的整体构建超时及进程组清理；工具内部派生的其它进程仍由外层包装器管理。一个构建有多个工具步骤，不承诺整个构建在 120 秒内结束。原子产物发布路径保持不变，任何步骤失败都不替换已有输出。

验证工作目录 `/home/fin/luna`。`python3 -B development/check.py --timeout 300 -- test -p luna --lib` 退出 0，45 项通过（`/tmp/luna-tools-dfsjhv48/result.json`），包含大于原生测试日志上限的完整输出、失败控制字符转义、持续 stderr 洪泛超时、stdout 恰好到达上限与超限但子进程成功退出的拒绝回归。`test -p luna --test host shipped` 退出 0，31 项通过（`/tmp/luna-tools-rt9_nqpz/result.json`），实际走新的工具执行路径生成并运行 LLVM 产物。格式化退出 0（`/tmp/luna-tools-hrwc3e95/result.json`），git diff --check 通过。改动留在 dev，未提交。

## 工具退出后的输出收集期限

LLVM 工具主进程成功退出后，收集剩余 stdout 的循环仍使用该次调用的原始期限。继承管道的子进程持续输出时，不再绕过超时检查；期限到达返回失败，不把已收集的部分模块交付给下一阶段。正常的完整输出、容量上限与诊断截断规则保持不变。本次没有增加进程组终止机制。

2026-10-24，工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 600 -- test -p luna --lib` 退出 0，63 项通过（`/tmp/luna-tools-7ct_q2f_/result.json`）；新增回归以实际已成功退出且仍有待收集输出的进程验证期限拒绝，并用新的有效期限检查完整缓冲数据。`test -p luna --test host shipped` 退出 0（`/tmp/luna-tools-2cqi0qjx/result.json`），实际构建并执行仓库示例。格式化退出 0（`/tmp/luna-tools-0yv0u19t/result.json`），`git diff --check` 通过。dev 未提交。

## 经指针形成的递归数据布局

修复前，已通过前端的 `Node {next:Link,flag:bool}` 与 `Link {none,some(*Node)}` 在 LLVM repr 中无限展开 pointee，最终报 representation nesting budget exhausted（`/tmp/luna-tools-qhky2iqj/result.json`）。数据指针现采用无递归 pointee 的物理表示；读、写、BitVec 写回与字段投影在访问点从已检查 IIR 的逻辑类型取得布局。获取逻辑 pointee 的检查在操作数正常交付后执行，保留 Never／提前返回路径。内部 handler 环境与清理地址同步采用 opaque pointer，析构仍检查完整逻辑 subject 指针类型，IIR 验证与资源准入不变。

新增 `examples/host-recursive-data.fl`，并加入 host_examples 目录。回归覆盖枚举携带递归节点指针、复制节点、字段写回、两个互相引用类型的有限布局、递归节点析构、不同名义类型指针的拒绝与旧产物保留。布局单元检查确认按值递归仍因无有限布局被拒绝，未经配对资源验证的数据指针也仍拒绝。

2026-10-24，工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --lib --test host --no-fail-fast` 退出 0，64+489 项通过（`/tmp/luna-tools-ag5p4g9o/result.json`）。随后增加析构案例，定向 `test -p luna --test host recursive_data` 2 项通过（`/tmp/luna-tools-vssgq66k/result.json`）。新增示例通过 `python3 -B development/command.py --timeout 600 -- python3 -B development/host_examples.py --compare-optimization recursive-data` O0/O3 构建与执行（`/tmp/luna-tools-xrz8xz88/result.json`）。不是一般分配器、拥有式链表库或递归类型系统全部完成的声明。dev 未提交。

## 可运行的栈上链表反转

新增 `examples/host-linked-list.fl`，以三个位于栈上的 Node 和 Link 枚举构造链表；循环先保存 next 再写入 previous，最后沿修改后的链接调用具名 value helper，逐节点检查值与精确长度。没有假设拥有式指针、堆分配器或完整容器库。示例加入 host_examples，可运行 `python3 -B development/host_examples.py --compare-optimization linked-list`。

测试扩展到空链、单节点、双节点与三节点，解释器和 LLVM O0/O3 输出一致。故意将末节点值改为 4，必须发生一次运行时契约失败；故意将反向链接清空，必须输出 false，不能把检查优化掉。首次用例生成器替换顺序使双节点值重复，契约检查正确拒绝（`/tmp/luna-tools-9lr9fuxx/result.json`）；修正生成器后，两组测试全部通过（`/tmp/luna-tools-fjrtx3mo/result.json`）。编译器实现本轮未修改。

2026-10-24，工作目录 `/home/fin/luna`，定向命令 `python3 -B development/check.py --timeout 600 -- test -p luna --test host linked_list` 退出 0；示例运行器经 command.py 执行 O0/O3 构建和执行，退出 0（`/tmp/luna-tools-1htedp5d/result.json`）。格式化退出 0（`/tmp/luna-tools-2nwdk4vy/result.json`）。递归函数遍历仍有内存契约预算缺口，保存在 `repros/host-recursive-node-memory-contract.fl` 并列入 frontend-checkpoint；未将该缺口固化成语言拒绝规则。dev 未提交。

## 内存契约预算失败的实例诊断

继续分析递归 Node 遍历：工作队列只有一个函数实例，访问摘要随沿 next 的递归调用持续展开；当前路径列表不能表达递归访问的闭包。本轮没有提高预算，也没有以不完整摘要放行。推导或完成摘要时的耗尽错误现保留原始原因，同时补充活动函数名、完整实例身份、已发现／待处理实例数及上一轮访问项数。信息只用于诊断，不凭统计量宣称已经证明不收敛。去掉了初始化摘要时紧邻的重复 signature bound_type 检查。

复验 `repros/host-recursive-node-memory-contract.fl` 仍按预期失败，新诊断定位 enable，同一实例上一轮有 80 个访问项（`/tmp/luna-tools-85rd205f/result.json`）。新增测试在 20k、100k 两档推导预算下检查错误上下文、包状态不变及随后独立 observe 推导恢复，不发布递归 bottom seed。

2026-10-24，工作目录 `/home/fin/luna`：经 development/check.py 执行 `test -p luna --test method_memory_contracts`，9 项通过（`/tmp/luna-tools-gj0qy0qf/result.json`）；`test -p luna --test host linked_list`，2 项通过（`/tmp/luna-tools-6fz15q8n/result.json`）；`test -p elaboration --lib memory` 退出 0（`/tmp/luna-tools-ki8kg5jr/result.json`）。格式化退出 0（`/tmp/luna-tools-vqp8mmz_/result.json`），git diff --check 通过。一般递归访问的内存摘要仍未实现，dev 未提交。

## 闭包状态数组直接计数

LLVM 闭包类型输出及构造现在直接遍历物理环境统计状态 token 数量，不再为取得长度而构造所有字段路径及其 Vec。前序编号不变，record／closure 递归展开，sum 仍只占自身 token。单元测试检查手写嵌套路径的编号与数量、空环境、100 层嵌套和 sum 的边界；本次没有改变资源准入或闭包 ABI。

新增循环回归：每轮重新构造捕获可变 bool 和 Watch 的闭包，调用一次消费 Watch，十轮必须恰好析构十次，且每轮 bool 从 false 开始。解释器与 LLVM O0/O3 输出一致。2026-10-24 在 `/home/fin/luna` 经 development/check.py 执行 `test -p luna --test host loop_closures` 退出 0（`/tmp/luna-tools-9j02x6i4/result.json`）；扩大检查 `test -p luna --lib --test host closure` 退出 0（`/tmp/luna-tools-ru4qjit3/result.json`）。格式化退出 0（`/tmp/luna-tools-5u98sbo0/result.json`），git diff --check 通过。dev 未提交。

## 限制布局的总展开规模

Host 表示生成除了原有 128 层深度限制，现为单次展开设置 65,536 个节点的共享预算。仅限制深度不能阻止少量名义声明形成的 DAG 按值展开为指数规模；record、tuple、newtype、closure、错误结果和 enum 载荷现沿整棵物理布局共享计数，enum 无载荷 variant 也计入其元数据节点。耗尽时返回 `host representation node budget exhausted`，不发布部分布局。该限制是当前后端的资源上限，不是语言类型等价或合法性规则；指针依旧只形成一个地址节点，不展开 pointee。

新增单元构造 20 个双字段名义类型，确认在深度上限以内的指数布局被节点预算阻止；8 层二叉布局恰好 511 个节点时成功，510 时失败，随后重新计算可恢复。另覆盖各类结构的兄弟节点共用预算、闭包签名节点、枚举 variant 元数据及大 pointee 的 opaque pointer。

2026-10-24，工作目录 `/home/fin/luna`：`python3 -B development/check.py --timeout 900 -- test -p luna --lib --test host --no-fail-fast` 退出 0（`/tmp/luna-tools-bflkxey6/result.json`）。格式化退出 0（`/tmp/luna-tools-nqqn1io1/result.json`），git diff --check 通过。该预算不等同于完整编译内存／输出字节配额，dev 未提交。

## 单文件原生运行入口

新增 `development/run_host.py`：接受源文件、入口、优化等级及 `--` 后显式传入的 luna 选项，复用 build_host_driver 与 process_tools 完成私有驱动构建、程序构建和有期限的原生执行。构建失败不执行产物；超时、信号退出和非零状态沿既有进程工具传播。runner 元数据位于 stderr，stdout 保留程序原始字节。每阶段和总报告保存到独立临时目录；输出截断或证据读取失败不能成为成功。未增加语言语法或绕过资源检查。

2026-10-24，工作目录 `/home/fin/luna`：经 command.py 执行 `python3 -B -m unittest discover -s development -p test_run_host.py`，4 项测试通过（`/tmp/luna-tools-c_ii6set/result.json`），覆盖包含空格的路径、参数逐项转发、各阶段失败停止、超时状态传播、二进制 stdout 与诊断分离、截断输出和非法期限。实际链表 O3 构建并执行成功，stdout 仅为 `(bool true)` 加换行（`/tmp/luna-tools-3rq263bz/result.json`）；实际死循环在 0.1 秒执行期限返回 124（`/tmp/luna-tools-r3kbvl7g/result.json`）；未知名称源码在编译阶段返回 1（`/tmp/luna-tools-q3jg5mxv/result.json`）。README 已加入直接运行用法。dev 未提交。

## 自定义多包直接运行

`run_host.py` 现允许省略 source，在 `--` 后直接传入已有 `--packages-v1` 请求。输入保持原始包身份、依赖别名、路径、协议／intrinsic 选项和当前工作目录，由 luna 负责包图验证；不另设 manifest 格式、不猜测依赖。source 与多包模式混用、缺失输入及 stream 模式在启动 driver 前拒绝。总报告记录 inputs，后续构建／执行与失败停止逻辑复用单文件入口。

2026-10-24，工作目录 `/home/fin/luna`：command.py 下执行 `python3 -B -m unittest discover -s development -p test_run_host.py`，6 项通过（`/tmp/luna-tools-j3o3c7ft/result.json`）。真实三包 library-order 示例在 O3 构建并运行成功（`/tmp/luna-tools-e38kd7gv/result.json`）；缺失 provider 9 的依赖请求返回 1，未进入原生执行（`/tmp/luna-tools-_zsbq6g7/result.json`）。README 提供完整命令，dev 未提交。

## 包图输入诊断

实际多包运行暴露的缺失依赖不再报告 InternalError。resolve 增加 InvalidPackageGraph（诊断码 4010），共享校验明确请求方、转义后的别名与提供方；覆盖缺失包、空／重复别名、自依赖、builtin 缺失／别名冲突及重复解析 ID。luna 在源码加载前执行同一校验，解析 ID、语义身份、空名和缺失主包也给出各自编号。根作用域的真实内部错误仍保留内部分类，不以输入诊断掩盖实现故障。

2026-10-24，`/home/fin/luna` 下经 development/check.py 执行 `test -p resolve --lib`，2 项通过（`/tmp/luna-tools-k0ujfysn/result.json`）；`test -p luna --test packages --test frontend_cli --no-fail-fast`，48+197 项通过（`/tmp/luna-tools-69jks2wm/result.json`）。回归包括来源文件尚不存在时先拒绝非法边、修正请求后成功、请求方局部别名及错误别名转义。实际 CLI 复验返回 1，并显示 `from package 1 via alias "missing" to package 9: provider package 9 is absent`（`/tmp/luna-tools-jhtahldv/result.json`）。格式化退出 0（`/tmp/luna-tools-mlw254c2/result.json`），git diff --check 通过；dev 未提交。

## 包入口软链接变化使前端缓存失效

补充包流式前端缓存的路径身份校验。原实现仅重扫旧的 discovery exclusions 并比较源码字节；依赖入口从 app/a.fl 改指向内容相同的 app/b.fl 时，仍错误命中旧缓存（复现 `/tmp/luna-tools-mzob_k7i/result.json`），尽管 a/b 的包归属已经交换。现在缓存记录每个显式包入口的 canonical 路径，在复用前及新编译发布前核对。入口变化重新编译，入口无法读取则先移除旧记录并返回错误；不以旧产物兜底。元数据仅在进程内保留，不改变 version-1 输入序列化格式；直接运行工具本身仍每次编译源码。

2026-10-24，在 `/home/fin/luna` 经 development/check.py 执行 `test -p luna --test compilation_cache --test packages --test frontend_cli --no-fail-fast`，7+48+197 项通过（`/tmp/luna-tools-0j87xs_a/result.json`）。新增回归检查相同字节下的实际包归属交换、目录软链接改指向后的返回值变化、失效入口清除缓存及恢复后重新编译。此校验不是原子文件系统快照，dev 未提交。

## 流式前端响应不再复制完整 dump

新增 NIR 的 QuotedDisplay 适配器，与字符串字面量共用转义规则，直接转义 Display 写出的文本片段。包流式服务保留成功编译的 Arc，直接输出 artifact 或带输入的结果；不再先对整个 artifact／compilation-result 调用 to_string／format。输入清单的 text/raw-text 也直接引用原 Arc<str>，不额外复制源文件字符串。version-1 响应字节格式保持不变，局部 IR Display 自身所需的分配仍可能存在。

测试覆盖 Unicode、引号、反斜线、分块控制字符、空字符串；小容量输出端失败会立即停止 Display 生产者，避免先生成完整后续文本。传输在 payload 中途写失败时返回错误，并保持下一帧未读取。

2026-10-24，工作目录 `/home/fin/luna`：通过 development/check.py 执行 `test -p nir --lib`，5 项通过（`/tmp/luna-tools-yylofvwt/result.json`）；`test -p luna --lib --test frontend_cli --no-fail-fast`，67+197 项通过（`/tmp/luna-tools-rjioialo/result.json`）。后补源文本零复制与传输失败案例，定向 `test -p luna --test packages compilation_input_dump` 通过（`/tmp/luna-tools-9utauvxb/result.json`），`test -p luna --lib package_stream` 通过（`/tmp/luna-tools-6z7auq8f/result.json`）。格式化报告 `/tmp/luna-tools-6dwmx5t6/result.json`；dev 未提交。

## 原生运行工具的临时编译器生命周期

`host_examples.py` 和 `run_host.py` 现在在本次调用结束时释放本次独占目录中的私有 luna 副本，包括非零退出和 Python 异常；示例程序、构建输出和报告继续保留。不会扫描或删除历史目录。强制终止父进程（例如 SIGKILL）仍可能留下副本。

2026-10-24，在 `/home/fin/luna` 经 `development/command.py --timeout 120 -- python3 -B -m unittest discover -s development -p ...` 验证：test_host_examples.py 7 项（`/tmp/luna-tools-3zm5anm4/result.json`），test_build_host_driver.py 6 项（`/tmp/luna-tools-o07s5stl/result.json`），test_run_host.py 6 项（`/tmp/luna-tools-3z0q2oj5/result.json`），均退出 0；git diff --check 通过。本次仅验证工具生命周期，没有重新构建编译器。磁盘调查发现历史示例目录保留 83 份编译器副本、约 9.88 GiB；本次修复防止继续累积，但未回收历史副本，不能据此宣称先前完整 host 测试的配额问题已经解除。dev 未提交。

## 泛型平台整数算术与低空间验收

泛型原生算术与有限类型域验证加入 usize/isize，类型身份保留，实际范围来自显式 package.target。有符号除余仍不提供隐式舍入策略。`host-generic-integers.fl` 扩展了 usize 的 GCD／距离和 isize 取负，沿用真实 ops 库协议绑定。

debug 测试链接受配额限制后，本次只清理已授权的 Cargo target，以 `python3 -B development/check.py --timeout 900 -- test --release -p luna --test target_integers` 完成 21 项测试（`/tmp/luna-tools-q6wfin0r/result.json`）。覆盖 32/64 位解释执行、宿主目标下 LLVM O0/O3、无目标拒绝，以及未调用定义的溢出和除零拒绝。release 是构建 Rust 编译器／测试程序的配置，Flurry 生成代码仍分别验证 O0/O3。

扩展示例通过已有 release luna 实际构建并执行，逐字比较 stdout；两级优化都输出 `(bool true)` 加换行（`/tmp/luna-tools-_khr7cbo/result.json`，详细构建／执行报告与可执行程序保留在 `/tmp/luna-platform-native-46z5tmce`）。原生构建仍绑定 operators::Add/Sub/Mul/Div/Rem/Neg/Not。历史私有编译器副本没有删除，本次成功不代表所有 debug 链接的配额问题已经解除。

补充回归：同一工作目录经 development/check.py 运行 `test --release -p luna --test host generic_`，29 项通过（`/tmp/luna-tools-wkefk15y/result.json`），包含已有定宽整数算术、泛型约束、取负协议和浮点操作。连同 target_integers，本轮实际执行 50 项 Rust 测试；未重跑完整 host 组。格式检查 `/tmp/luna-tools-zs__v3eh/result.json` 退出 0，git diff --check 通过。dev 未提交。

## 日常入口选择 release 编译器

`build_host_driver.py --release` 在现有缓存维护锁内构建并复制 release/luna；不会读取 debug/luna。`host_examples.py` 和 `run_host.py` 通过 `--release-driver` 选择此配置，生成代码的优化级别独立，summary 记录 driver_profile。默认行为保持 debug；构建失败、缺失 release 产物不会回退到已有 debug 编译器。临时副本沿用本次运行结束释放的策略。

2026-10-24，工作目录 `/home/fin/luna`：经 command.py 运行 `python3 -B -m unittest discover -s development -p 'test_*host*.py'`，21 项通过（`/tmp/luna-tools-ghqwjrfl/result.json`），覆盖配置与产物对应、缺失产物拒绝、参数不泄漏到 Flurry 编译命令、优化级别独立、错误传播和副本回收。实际使用 `host_examples.py generic-integers generic-iteration generic-order --compare-optimization --release-driver`，6 次原生执行通过（`/tmp/luna-tools-82ubgafr/result.json`）。`run_host.py development/examples/host-arithmetic.fl --release-driver --opt-level 3` 返回 0 且 stdout 恰为 `(unsigned-integer 42)` 加换行（`/tmp/luna-tools-9fcxp0rd/result.json`）。另核对 `/tmp/luna-host-examples-i0a6epo0` 与 `/tmp/luna-run-host-3o5zuvz9` 的临时 luna 已释放，程序和汇总报告保留。git diff --check 通过；dev 未提交。

## math 的逐位与、或、异或

补齐文档已提及的 math.bit_and/bit_or/bit_xor：三个 pure 泛型包装通过显式 Scalar tag 绑定同名 target operation，接受同类型的两个定宽整数，返回相同类型。域为 i8/i16/i32/i64/i128 和 u8/u16/u32/u64/u128；signed 输入按其位模式组合。LLVM 直接生成 and/or/xor 指令，两侧先按源码顺序各求值一次，不用零／全一掩码短路求值，也不构造不存在的 llvm.bit_* 函数。

程序验证继续遍历实参并保留结果整数域，没有把按位关系添加到 PTC 或无依据地输出多项式等式。平台整数尚需 target-aware intrinsic admission，此次不纳入；BitVec 沿用自己的代数。三个新 tag 追加到 math 枚举末尾，既有 tag 顺序保留；CLI 帮助、库验收脚本、示例目录和显式测试绑定同步。新增 host-bitwise.fl 演示按掩码替换选定位。

2026-10-24，工作目录 `/home/fin/luna`：经 development/check.py 运行 `test --release -p luna --test host native_bitwise`，2 项通过（`/tmp/luna-tools-oyr_xhr4/result.json`），覆盖十种整数的 O0/O3、符号边界、零／全一掩码实参顺序和次数，以及错误域／参数类型／arity／unsafe／contract 拒绝。正例首次因拼接 `{` 与 `--` 形成块注释失败（`/tmp/luna-tools-rue6wdv7/result.json`），修正测试源码换行后通过，未改变词法规则。

`test --release -p luna --test host_intrinsic_protocol --test frontend_cli`，34+201 项通过（`/tmp/luna-tools-idyk89nj/result.json`）；经 command.py 的 test_host_examples.py 7 项通过（`/tmp/luna-tools-k_5ze3ch/result.json`）。`host_examples.py bitwise math-fma --release-driver --compare-optimization` 的四次原生运行均符合输出契约（`/tmp/luna-tools-hsw5s1ox/result.json`）。格式检查 `/tmp/luna-tools-7x3kujh8/result.json` 和 git diff --check 通过。共 237 项 Rust、7 项 Python 测试，未重跑全部 host 测试。dev 未提交。

## 位运算 intrinsic 的数值规格

程序验证对 BitAnd/BitOr/BitXor 增加保守区间传递。两个输入区间均为单点时按指定有符号性和位宽计算精确结果，包含 i128 最小值的补码解释；所有输入先验证在机器整数域内。与全一掩码做 and、与零做 or/xor 保留另一输入的区间；非负操作数的 and 产生非负且不超过该操作数上界的结果；unsigned or 保留至少各输入下界的事实，signed or 仅在一个操作数始终为负时推出负结果及相应下界。无法缩窄时保留完整整数域。

所有实参先依次分析，随后使用已求值 Value 的区间；不重新读取其来源局部变量，也不发布未冻结的符号等式。推导属于程序验证层，不进入 PTC；没有开放 target intrinsic 的 comptime invocation。library/README 已同步精度说明。

2026-10-24，工作目录 `/home/fin/luna`：通过 development/check.py 运行 `test --release -p elaboration --lib bitwise`，2 项通过（`/tmp/luna-tools-e3n4bpfi/result.json`）。底层测试对选定 signed/unsigned 8 位区间逐值枚举所有输入组合，核对包含性；另覆盖 i128 边界、错误位宽、越域及预算失败后恢复。`test --release -p luna --test host native_bitwise`，4 项通过（`/tmp/luna-tools-2p2cd_8m/result.json`），包含十种整数的正确 contract、边界及实参写入顺序，LLVM O0/O3 执行；错误 contract 用例要求明确出现 postcondition is not proven，不能用词法／类型失败代替。测试模板曾出现 Rust 格式串括号错误及 intrinsic 常量实参缺少类型的问题，已修正并重新执行。

既有 `test --release -p luna --test host native_integer_intrinsic_contract` 1 项通过（`/tmp/luna-tools-qn_5yzaa/result.json`）。合计 7 项 Rust 测试；格式检查 `/tmp/luna-tools-atqgt7zs/result.json` 与 git diff --check 通过。dev 未提交。

## 平台整数按位操作与纯度案例规划

math.bit_and/bit_or/bit_xor 的域扩展到 usize/isize。Checked 保留源码标量身份，width_for_target 在消费布局时读取显式目标；不缓存某次查询的位宽，不把 usize/isize 等同于 u64/i64。无目标的未执行声明可以完成，依赖具体整数范围的规格或 LLVM 发射必须取得目标布局。程序验证与 LLVM 使用同一个 package.target；其他整数 intrinsic 的平台域仍待实现。

扩展 math 域时发现整包纯度规划耗尽共享预算，导致 bit_xor 缺少 P4 证据。现在对已经绑定到 primitive 的类型条件进行带预算、三值逻辑的局部化简，避免重复查询已确定的候选；有限域覆盖证明、每个候选的函数体检查、未知条件的 PTC 查询和案例数量上限仍保留。未提高预算，未展开 opaque normal-form 头，也未将程序状态规格加入 PTC。

2026-10-24，工作目录 `/home/fin/luna`，通过 development/check.py 执行：

- `test --release -p elaboration --lib type_domains`：6 项通过（`/tmp/luna-tools-28u70g5c/result.json`），覆盖十二种整数候选、否定条件、相关参数、未知头和预算恢复。
- `test --release -p luna --test target_integers --test host_intrinsic_protocol`：23+35 项通过（`/tmp/luna-tools-gi4hqetp/result.json`），包含整包 math 的纯度检查、平台身份、无目标拒绝、32/64 位规格与 LLVM IR、宿主宽度下 O0/O3 执行。先前整包 P4 失败记录为 `/tmp/luna-tools-0nge_zec/result.json`，已由本次成功复验覆盖。
- `test --release -p luna --test runtime_purity --test pure_callbacks`：64+80 项通过（`/tmp/luna-tools-oeoyp63t/result.json`）。

通过 command.py 执行 `python3 -B development/host_examples.py bitwise --release-driver --compare-optimization`，新增 usize 掩码替换和 isize 异或的示例在 O0/O3 均通过（`/tmp/luna-tools-ojfmqlmz/result.json`），产物位于 `/tmp/luna-host-examples-8mtj6msr`。32 位并未在当前宿主上执行。共 208 项 Rust 测试；格式检查 `/tmp/luna-tools-5jcin2y1/result.json` 与 git diff --check 通过。dev 工作区保留改动，未提交。

## usize 位计数、反转与旋转

将 bit_reverse、count_ones、leading_zeros、trailing_zeros、funnel_shift_left/right 的 target operation 域和 math 泛型约束扩展到 usize，rotate_left/right 同步接通。布局消费复用 width_for_target，源码身份保持 usize；零值计数返回目标位宽，funnel shift 的移位量按该位宽取模。byte_swap 仍排除 usize：当前目标模型允许 8 位指针，不能无条件满足其 16 位倍数契约。其他有符号输入和 BitVec 也没有进入这组六种操作的域。

2026-10-24，工作目录 `/home/fin/luna`，经 development/check.py 执行 `test --release -p luna --test target_integers --test host_intrinsic_protocol`，首次构建遇配额错误；管理入口清理 Cargo target 并重试成功。intrinsic 协议 35 项通过，目标整数组 23 项通过、新增 1 项因测试模板缺少中缀操作符空格失败（`/tmp/luna-tools-8in8foaz/result.json`）。修正模板后定向测试通过（`/tmp/luna-tools-3hkp2o7o/result.json`），再执行整个 target_integers 组，24 项通过（`/tmp/luna-tools-vkniddmt/result.json`）。新增测试使用真实 math 包，覆盖 32/64 位计数上界、零值、最高位反转、旋转、整位宽及超位宽 funnel shift；错误计数上界必须出现 postcondition is not proven。两种布局均检查 LLVM IR，仅宿主匹配宽度执行 O0/O3。

排队的示例任务曾等待构建锁超时（`/tmp/luna-tools-qi2xpa6y/result.json`），没有视为执行成功。构建结束后重新运行 `host_examples.py bitwise --release-driver --compare-optimization`，O0/O3 均通过（`/tmp/luna-tools-5gegly9e/result.json`），示例及产物位于 `/tmp/luna-host-examples-g7pw7lxb`。格式检查 `/tmp/luna-tools-0jsw698z/result.json` 与 git diff --check 通过。最终覆盖 59 项不同 Rust 测试；dev 未提交。

## 平台整数饱和运算、极值和绝对值

saturating_add/sub、整数 min/max 的源码域加入 usize/isize；wrapping_abs 加入 isize，并保留 result == value'old or result == -value'old 的公开契约。LLVM 使用目标宽度和源码有符号性选择 uadd/sadd、usub/ssub、umin/smin、umax/smax；abs 的 INT_MIN poison 标志仍固定为 false。程序验证沿用已求值操作数的区间与关系快照，最小有符号整数返回自身。

扩展公开契约后，未指定目标的整个 math 包曾在 wrapping_abs 的 P4 检查失败（`/tmp/luna-tools-1bthc4ng/result.json`）。修复将副作用分类与布局消费分开：纯度检查可识别同类型平台整数算术，溢出证明仍独立执行。机器运算验证入口对已实例化的泛型平台类型案例，在 target=None 时使用私有证明副本检查当前 TargetData 允许的全部 16 种布局（8..128，步长 8）。不选择宿主、不改变输出包目标或源码文字，不将程序命题加入 PTC。非泛型平台字面值形成仍要求显式目标。目标已指定时只按该目标证明；默认参数仍在函数前置条件之外单独验证。

2026-10-24，工作目录 `/home/fin/luna`，通过 development/check.py 执行：

- `test --release -p luna --test host_intrinsic_protocol --test target_integers --no-fail-fast`：35+26 项通过（`/tmp/luna-tools-uehenavu/result.json`）。包含整个 math 包无目标加载、泛型便携定义保留 target=None，以及在 32 位安全但 8 位溢出的未调用定义被拒绝。32/64 位规格和 LLVM IR 均验证，宿主匹配宽度下执行 O0/O3，覆盖饱和上下界、有符号／无符号极值、INT_MIN 绝对值与 min/max 关系契约。
- `test --release -p luna --test runtime_purity --test pure_callbacks`：64+80 项通过（`/tmp/luna-tools-dc5uhzxk/result.json`）。
- `test --release -p luna --test integer_contracts --test integer_formation`：14+7 项通过（`/tmp/luna-tools-uk2i3r75/result.json`）。

共 226 项 Rust 测试。通过 command.py 执行 `host_examples.py saturating-integers integer-minmax wrapping-abs --release-driver --compare-optimization`，六次原生执行均通过（`/tmp/luna-tools-dubqzy5t/result.json`）；产物位于 `/tmp/luna-host-examples-nof4mezr`。格式检查 `/tmp/luna-tools-g6u7wjtv/result.json` 与 git diff --check 通过。byte_swap 的平台布局准入仍待实现；dev 工作区未提交。

## usize byte_swap 的布局准入

math.byte_swap 与 bswap target operation 接受 usize，保留原先 u16/u32/u64/u128 域以及 u8、signed、float、BitVec 拒绝规则。新增 Checked::validate_target_layout，布局未定或 scalar 尚未实例化时返回延迟结果；具体 bswap 宽度必须是 16 的正整数倍。源码准入、NIR phase 校验、程序验证和 LLVM 发射使用相同检查，LLVM 还要求确实取得具体宽度。不把平台身份换为定宽类型，也不在 Checked 中缓存某个目标的布局。

2026-10-24，工作目录 `/home/fin/luna`，经 development/check.py 执行 `test --release -p luna --test target_integers --test host_intrinsic_protocol --no-fail-fast`：27+35 项通过（`/tmp/luna-tools-k3zgcdcc/result.json`）。新测试检查全部 8..128、步长 8 的目标：16 的倍数验证字节交换结果及 LLVM IR，奇数字节宽度要求明确的 bswap 布局诊断；未定目标声明保持 target=None。已生成包改成不合法目标后 NIR 校验与 LLVM 发射拒绝，泛型实例化不能绕过限制。宿主匹配宽度执行 O0/O3。

通过 command.py 执行 `host_examples.py bitwise --release-driver --compare-optimization`，加入 usize 字节交换往返的跨包示例在 O0/O3 均通过（`/tmp/luna-tools-wn_a05ok/result.json`）。格式检查 `/tmp/luna-tools-7hqpq90b/result.json` 与 git diff --check 通过；共 62 项 Rust 测试，dev 未提交。

## Ord 协议的源码比较运算符

非原生类型的 `< <= > >=` 现在通过显式 Ord 绑定分别调用 lt/le/gt/ge。复用普通方法选择与调用 elaboration，支持泛型前提、实现与默认方法；普通实参保持源码顺序各求值一次，返回类型要求 bool。左侧 Never 保留既有提前退出路径，右侧 Never 仍先执行左侧。原生标量比较不改写为协议调用，保持 IEEE 无序行为并避免库中的原生比较实现递归回调自身。Eq 的运算符接线仍未完成。

新增 `--ord-protocol package.module.trait`，贯通普通 CLI、多包入口、语法阶段拒绝检查、sysroot manifest 默认值及显式覆盖；按声明身份绑定，不识别 trait 的源码拼写。真实清单加入 builtin.comparison.Ord。新的 host-custom-order.fl 由 comparison/resources 两个真实包提供协议，Version 的 cmp 实现字典序，泛型 precedes 使用 `<`，其余运算符使用库默认方法；原生路径执行完整 Copy/Drop 资源检查。

2026-10-24，工作目录 `/home/fin/luna`：首次构建在 elaboration 归档时遇配额错误，管理入口清理 Cargo target 后重试成功。随后修正新测试的 OsString 参数类型；第一次原生测试使用未检查名义存储的入口，被既有资源 gate 正确拒绝，现改用真实 resources 包和受检原生入口，没有放宽 gate。CLI 回归中隔离 sysroot 夹具漏带新 manifest 指向的 comparison 模块，已补真实文件并经公开重导出访问，保留模块可见性检查。

通过 development/check.py 执行 `test --release -p luna --test ordering_protocol --test frontend_cli --test packages --lib --no-fail-fast`：4 项 Ord、48 项多包和 68 项编译器单元测试通过（`/tmp/luna-tools-wcbpk57y/result.json`），该轮 CLI 夹具失败；修正后单独执行完整 frontend_cli，201 项通过（`/tmp/luna-tools-m_m7_1a5/result.json`）。Ord 测试覆盖真实库默认方法、泛型操作、解释执行与 LLVM O0/O3、左右求值顺序、提前返回、缺失实现、错误结果类型及 CLI 重复选项拒绝。`test --release -p luna --test library_protocols` 的 11 项通过（`/tmp/luna-tools-svgamfus/result.json`）。合计 332 项不同 Rust 测试。

通过 command.py 执行 `host_examples.py custom-order --release-driver --compare-optimization`，O0/O3 均输出 `(bool true)`（`/tmp/luna-tools-b3fyhwd7/result.json`），产物在 `/tmp/luna-host-examples-hyscg5ou`。test_host_examples.py 的 7 项通过（`/tmp/luna-tools-5g0_22q9/result.json`），格式检查 `/tmp/luna-tools-a6h_m1jg/result.json` 和 git diff --check 通过。dev 未提交。

## Eq 协议的普通相等与不等表达式

新增显式 Eq 身份绑定：非原生类型的 ==／!= 分别通过普通方法选择与 finish_call 调用 eq／ne，支持泛型 Eq 前提及真实库的默认 ne。覆盖的 ne 直接执行，不能替换成编译器对 eq 的取反。返回值必须是 bool，左右操作数各求值一次，保持既有 Never 与提前返回边界。Type、BitVec、原生标量及已证明原生域的泛型仍走原有相等节点；模式常量观察与编译期查询键协议不在本轮范围。

`--eq-protocol` 已接普通 CLI、多包入口、语法阶段拒绝和 manifest 默认／显式覆盖。真实清单加入 builtin.comparison.Eq。custom-order 示例的 Version 新增 ==／!=，现在由实际 comparison/resources 包验证全部六个比较运算符。

2026-10-24，工作目录 `/home/fin/luna`，经 development/check.py 执行 `test --release -p luna --test equality_protocol --test ordering_protocol --test frontend_cli --test packages --lib --no-fail-fast`：68 项单元、201 项 CLI、48 项多包及 4 项 Ord 均通过（`/tmp/luna-tools-2ad3o8tm/result.json`）；新 Eq 测试先有词法空格与类型身份预期错误。核对 docs/type/type-identity.md 与解释器既有契约后，保留 Identity<T> 与 T 的关系 Unknown，不能由“不展开”推出不等；测试通过正式 comptime invocation 检查未决关系，同时验证同一范式应用自相等。未更改类型相等实现。

修正测试后 `test --release -p luna --test equality_protocol` 的 5 项全部通过（`/tmp/luna-tools-fdu0h7uv/result.json`），覆盖真实 Eq 默认方法、泛型操作、解释执行及受资源检查的 LLVM O0/O3、自定义 ne 的可观察副作用、操作数顺序、左右提前返回、缺失实现／错误结果类型拒绝、CLI 重复绑定以及类型身份边界。`test --release -p luna --test library_protocols` 的 11 项通过（`/tmp/luna-tools-9slv5ovl/result.json`），共 337 项不同 Rust 测试。

经 command.py 执行 `host_examples.py custom-order --release-driver --compare-optimization`，O0/O3 均输出 `(bool true)`（`/tmp/luna-tools-c8ab9l3u/result.json`），产物在 `/tmp/luna-host-examples-8rx_d_96`。test_host_examples.py 的 7 项通过（`/tmp/luna-tools-dqghh21o/result.json`）。格式检查 `/tmp/luna-tools-norzlkup/result.json` 与 git diff --check 通过；dev 未提交。

## 完整 sysroot 的 host smoke 检查

`development/library_check.py --stage sysroot` 现在构建并执行实际的 LLVM
host 程序。旧入口使用无目标信息的 `--interpret`，在完整 builtin 中遇到
`usize` 字面量时失败；不应通过猜测指针宽度或删除库代码绕过这一边界。
新的入口使用 host pipeline 已有的目标查询，并将构建和运行的报告分别保存。
构建失败不运行产物；执行失败或输出不是 `(bool true)` 均判为失败。

`development/examples/library-smoke.fl` 使用完整库及其 language-item
manifest，覆盖默认 prelude、平台整数算术、Clone、Tuple 解构和原生可选
类型。此检查证明这些接口在真实 sysroot 中可以构建和运行，不代表所有库
接口均已完成，也没有给无目标信息的解释路径补充隐式平台假设。

工作目录 `/home/fin/luna`，本次验证：

- `python3 -B development/library_check.py --release --stage sysroot`：退出 0，
  构建和运行均成功，输出 `(bool true)`；报告
  `/tmp/luna-library-lojxmvwm/summary.json`。
- `python3 -B -m unittest discover -s development -p test_library_check.py`：
  12 项通过，覆盖构建失败不执行、执行失败、错误返回值、独立阶段继续执行
  与构建/运行证据区分；这些是检查器测试，不代替上述实际 host 运行。
- `git diff --check`：通过。本次未改 Rust，无需重新构建编译器或运行全量测试。

旧解释入口的失败证据为 `/tmp/luna-library-cy826anb/summary.json`，错误为
`integer runtime proxy requires target pointer-width information`。完整库
源码的 LLVM 生成检查 `/tmp/luna-tools-2g7mhyc3/result.json` 退出 0，帮助
将该问题定位到检查入口缺少目标信息，而非源码无法通过 host 前端。

## 完整 sysroot 迭代与内存查询复用

内存依赖固定点现在在一轮、同一不可变包内复用成功的 trait 方法选择和已完成
关联类型视图。方法缓存沿用完整签名、接口、subject 与类型实参作为键，并按
调用位置恢复 source；失败选择和调用时存储不进入缓存。类型视图缓存只保存
由包级分派完成的结果，不将局部推断假设作为全局证据。查询命中仍计费。

新增 `development/examples/sysroot-iteration.fl`，通过普通 prelude 使用
Iterator、Drop 和 usize，验证拒绝前缀不截断 chain 尾部、提前匹配清理未产出
元素、空迭代清理回调捕获。检查入口为：

```sh
python3 -B development/library_check.py --release --stage sysroot-iteration
```

该阶段显式使用 200,000 预算，要求三个实际 native 测试均通过，不能以零测试
或部分成功充数。默认预算没有提高：优化前默认 100,000 下三个用例均失败
(`/tmp/luna-tools-vg_zwpu1/result.json`，退出 1)，200,000 下通过
(`/tmp/luna-tools-e81j8jyz/result.json`，退出 0)。优化后默认预算下两个资源
清理用例通过；较长组合链已通过分派/内存分析，但在后续 cleanup planning
耗尽预算 (`/tmp/luna-tools-_kr194tg/result.json`，退出 1)。仍需优化该路径。

在 `/home/fin/luna` 的验证：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_iteration`：
  41 项通过 (`/tmp/luna-tools-begeury5/result.json`)。
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test method_memory_contracts -- --nocapture`：
  10 项通过 (`/tmp/luna-tools-paedsmx1/result.json`)，包含低预算失败后恢复。
  方法查询工作量：0 个无关声明、1/16 次调用为 525/4305；128 个无关声明为
  1165/4945。追加 15 次调用均消耗 3780，避免无关声明反复放大调用成本。
  这是预算工作量，不是 wall-clock 加速测量。
- 上述 `sysroot-iteration` 阶段在 O0 通过三个用例
  (`/tmp/luna-library-hcdl4488/summary.json`)。
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-iteration.fl --sysroot library --test-native --inline-functions --fuel 200000 --opt-level 3`：
  O3 同样通过三个用例 (`/tmp/luna-tools-qjvjo0md/result.json`)。
- `python3 -B -m unittest discover -s development -p test_library_check.py`：
  13 项检查器测试通过。
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test effect_memory_transfers --test captured_memory`：
  捕获内存 94 项、效应内存传递 99 项通过
  (`/tmp/luna-tools-acl2ss8y/result.json`，退出 0)。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  退出 0 (`/tmp/luna-tools-019x59q3/result.json`)；`git diff --check` 通过。

## 按类型索引已取得的清理责任

CleanupSession 原先每次查找责任，都深度遍历全部已取得布局。现在为同一不可变
包和 Drop world 建立 `type -> acquired root + layout path` 索引。路径区分记录
字段和 variant payload，保留原来的首次取得顺序，复用已选定的子树；不重新
选择析构器，不缓存运行时构造状态或清理计划。索引只在整个新增布局完成计费和
登记准备后一次发布，索引中途耗尽预算不能留下部分责任。

规模回归先取得 0 或 64 个无关布局，再重复查询记录中 optional payload 的
自定义析构责任，要求查询工作量相同。另一回归刻意耗尽索引预算，确认未取得的
子字段不能参与规划，并验证补足预算后恢复。原 handler 槽位拒绝用例在 handler
外先取得替换指针，避免 `other.ref` 先触发另一个变量的 lexical capture 取址
拒绝，继续分别检验槽位重绑和对槽位取址的原有保护。

当前限制：完整 sysroot 的长组合链仍超出默认 100,000 预算，本次复测失败于
后续 `memory summary type budget exhausted`
(`/tmp/luna-tools-qd578au_/result.json`，退出 1)。不能据此宣称默认预算问题已经
解决。显式 200,000 预算的 O3 复测通过三个用例
(`/tmp/luna-tools-m7q35ygy/result.json`，退出 0)，命令：

```sh
python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-iteration.fl --sysroot library --test-native --inline-functions --fuel 200000 --opt-level 3
```

一次构建期间的直接复测因编译器二进制正在替换而未能启动
(`/tmp/luna-tools-xmvceaj3/result.json`，退出 125)，以上 O3 结果来自构建完成后
的实际重跑。首轮 Drop 回归中 995 项通过、一项诊断断言失败
(`/tmp/luna-tools-ijt43qcm/result.json`，退出 101)；第一次拆分仍误判槽位取址
诊断 (`/tmp/luna-tools-es4cwxhr/result.json`，退出 101)，随后保留槽位诊断并
消除替换指针取址的干扰，没有放宽源码拒绝行为。

最终验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test drop --test library_iteration --test conditional_capture_cleanup --test temporary_method_cleanup --test match_cleanup`：
  共 1052 项通过 (`/tmp/luna-tools-lw95sqly/result.json`，退出 0)，包含 996 项
  Drop、41 项迭代、13 项匹配清理，以及条件捕获与临时方法清理各一项。
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test drop cleanup_acquisition_lookup_does_not_rescan_unrelated_layouts -- --nocapture`：
  通过 (`/tmp/luna-tools-ojeeav00/result.json`)，重复查询 16 次在 0/64 个无关
  布局下均消耗 64 个预算单位。这是分析工作量，不是 wall-clock 测量。
- `python3 -B development/library_check.py --release --stage sysroot-iteration`：
  O0 三项实际 native 用例通过 (`/tmp/luna-library-8r07_5wz/summary.json`)。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-qz6pk0nk/result.json`)；`git diff --check` 通过。

## 复用完整布局的析构平凡性证明

同一 CleanupSession 现在缓存完整布局是否不含自定义析构责任的最终结论。肯定
和否定结论都可复用：自定义析构导致的 false 是稳定结果，预算耗尽或未完成查询
则不是。缓存只在完整组件遍历成功后写入，命中仍对完整类型键计费，不授予 Copy
或内存访问许可，也不缓存运行时构造状态。move analyzer 的已证明类型从线性
列表改为集合，避免每次命中都遍历无关类型；传递至被调函数分析时仍逐项计费。

回归先取得包含 64 个字段的布局，再以不足预算连续查询两次，要求两次都失败。
补足预算后分别验证普通记录为 true、自定义析构类型及其可选包装记录为 false；
随后 16 次命中消耗 16 个名义类型键预算，零预算仍失败。此证据仅说明布局证明
查询的工作量，不构成 wall-clock 加速或整个编译管线的预算保证。

最终验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test drop --test library_iteration --test conditional_capture_cleanup --test temporary_method_cleanup --test match_cleanup`：
  1053 项通过 (`/tmp/luna-tools-55dem_jt/result.json`，退出 0)，包含 997 项
  Drop、41 项迭代、13 项匹配清理及条件捕获、临时方法清理各一项。
- `python3 -B development/library_check.py --release --stage sysroot-iteration`：
  显式 200,000 预算的 O0 三项通过 (`/tmp/luna-library-r2q700i_/summary.json`)。
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-iteration.fl --sysroot library --test-native --inline-functions --fuel 200000 --opt-level 3`：
  O3 三项通过 (`/tmp/luna-tools-e406943_/result.json`)。
- 同一 native 命令去掉 `--fuel 200000 --opt-level 3` 后，默认预算仍在长组合链
  上报告 `memory summary type budget exhausted`，另两项通过
  (`/tmp/luna-tools-n7tm1da3/result.json`，退出 1)。因此尚未解决整条链的默认
  预算问题，不以局部缓存收益代替端到端结论。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-kfmq_kk9/result.json`)；`git diff --check` 通过。

## 内置包阶段集成与 Hash 的资源边界

在 `/home/fin/luna` 执行
`python3 -B development/library_check.py --release --timeout 900`：当时已有的
15 个阶段全部通过，`scope=all`、退出 0，报告为
`/tmp/luna-library-dkmraev2/summary.json`。这些阶段验证既有接口，不代表六个
内置包的全部能力已经完成；例如 std 的分配、拥有型字符串及 I/O 仍未实现。

本次将 `hash-protocols` 与 `sysroot-hash` 加入独立可选阶段及默认检查集。
前者运行 library_hash 的双相、碰撞与边界回归；后者运行完整 sysroot 的三个
实际 native 测试，覆盖布尔、字符、整数表示、128 位折叠、编译期/运行时整数
Hash 一致性及 Symbol 的编译期 Hash。检查器拒绝零测试、遗漏用例或失败结果。
数值 Hash 的既有原生回归同时改为带 Copy/Drop 的资源检查入口。

实际发现的未完成路径：runtime Symbol Hash 通过 tag_name 调用字符串 Hash，
因此在资源检查下同样需要字符串视图的 backing provenance，不能用未检查的
LLVM 执行成功来宣称支持。新增回归要求 Symbol/str Hash 在这条资源路径明确
拒绝；没有给 str 添加 Copy 或省略内存检查。相关实际 sysroot 失败证据为
`/tmp/luna-tools-ezav4h8q/result.json`（退出 1）。完整 runtime Symbol Hash
仍待字符串视图资源契约接通，不计入三个通过的 sysroot Hash 用例。

第一版 fixture 直接将 comptime 函数结果交给 runtime let，被阶段检查拒绝
(`/tmp/luna-tools-8w8c1f1m/result.json`)。最终用 inline if 消费编译期结果，
再验证对应 runtime 数值，不将 CMTT 或隐式 Splice 当作绕过阶段边界的办法。

新增检查的验证：

- `python3 -B development/library_check.py --release --timeout 900 --stage hash-protocols --stage sysroot-hash`：
  10 项协议回归、3 项实际 native 用例通过，退出 0，报告
  `/tmp/luna-library-grolyol8/summary.json`（scope=selected）。
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-hash.fl --sysroot library --test-native --opt-level 3`：
  三项 O3 用例通过 (`/tmp/luna-tools-j4d819d9/result.json`)。
- `python3 -B -m unittest discover -s development -p test_library_check.py`：
  13 项检查器测试通过。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-8p4i_uik/result.json`)；`git diff --check` 通过。


## Symbol 的独立字节观察与资源检查下的 Hash

Symbol 名称具有独立于局部承载对象的只读存储，语义依据见
`docs/basics/literals.md` 的 Symbol 与 tag_name 说明。本次增加内部
`symbol_byte_length` / `symbol_byte`，在 NIR 中保留独立的 Symbol 输入域，
同时接通 elaboration、comptime interpreter、LLVM 和文本区域摘要。
索引固定为 u64，读取返回 ?u8；LLVM 只在无符号索引小于长度后加载字节。
操作数仍按顺序求值一次，提前退出不会触发后续读取。

Symbol Hash 的两个阶段直接观察名称字节，保留原 FNV-1a 内容算法，不再为了
Hash 构造 str 描述符。字节读取的区域摘要记录 Static；长度只观察描述符。
这利用 Symbol 已有的存储保证，没有给 str 增加 Copy 或跳过资源检查。
上一节记录的 Symbol Hash 阻塞由本路径解决；一般 str Hash 的 backing
provenance 接线仍未完成，保留明确拒绝的回归。

本阶段验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test text_bytes --test library_hash --test text_regions --test text_view_layout --test symbols --test symbol_staging`：
  95 项通过，退出 0 (`/tmp/luna-tools-phoy6lgh/result.json`)。新增用例覆盖
  UTF-8、越界、输入域拒绝、求值顺序、提前退出和 Static 读取摘要；Symbol
  Hash 的原生测试使用带 Copy/Drop 的资源检查入口，并验证 O0/O3。
- `python3 -B development/library_check.py --release --stage hash-protocols --stage sysroot-hash`：
  10 项协议回归、3 项完整 sysroot 原生用例通过，退出 0
  (`/tmp/luna-library-autvwsqs/summary.json`，scope=selected)。
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-hash.fl --sysroot library --test-native --opt-level 3`：
  三项 O3 用例通过，退出 0 (`/tmp/luna-tools-ilashuvd/result.json`)。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-ft30lkn9/result.json`)；`git diff --check` 通过。

## meta 的转换收集与完整 sysroot 验收

新增 `meta.filter_map` 与 `meta.flat_map`，复用已有 List、可选类型和 comptime
回调能力。两者按输入快照顺序调用回调，分别收集外层存在的结果和内层列表元素，
保留元素域与 normal-form 身份。没有新增编译器 intrinsic 或运行时列表表示。
库说明及完整源码用例分别在 `library/meta/README.md` 和
`development/examples/sysroot-meta-lists.fl`。`sysroot-meta` 已进入默认库检查集。

在 `/home/fin/luna` 的实际验证：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test comptime_list_operations`：
  30 项通过，退出 0 (`/tmp/luna-tools-dy98z6e0/result.json`)。新增回归覆盖
  顺序、快照、空列表、嵌套可选值、Type 身份、失败停止/恢复及错误回调拒绝。
- `python3 -B development/library_check.py --release --stage sysroot-meta`：
  一项实际 host 原生用例通过，退出 0 (`/tmp/luna-library-knri0nss/summary.json`)。
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-meta-lists.fl --sysroot library --test-native --opt-level 3`：
  同一用例 O3 通过，退出 0 (`/tmp/luna-tools-5xfl1szv/result.json`)。
- `python3 -B -m unittest discover -s development -p test_library_check.py`：
  13 项通过，包含新增阶段的测试数/失败结果检查。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-6i500478/result.json`)；`git diff --check` 通过。

初次用完整 sysroot 的 `--invoke-comptime run` 执行时，因库内目标宽度整数
缺少 target 信息被拒绝 (`/tmp/luna-library-_56ie5lq/summary.json`，阶段退出 1)。
最终使用 host 测试中的 inline if 消费编译期结果，未给无目标 invocation 偷加
宿主布局。List 的不可变数据验证仍会影响大规模构造成本，不宣称线性时间。

## 可编辑的 StructBuilder 字段草稿

`meta.StructBuilder` 增加按声明顺序索引的 `insert_field`、`replace_field`、
`remove_field`，均返回 bool。负数和越界不修改草稿；插入允许位于字段数处，
替换与删除要求已有字段。内部先生成完整的新名称列表和 Type 列表，再写回
两者，保留既有快照。字段合法性与存储布局继续由 Build 校验，没有授予生成
类型 Copy 或自动复制原类型的实现。公开示例见
`development/examples/sysroot-edited-struct.fl`，已加入 `sysroot-builders` 阶段。

实际验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test comptime_list_operations`：
  56 项 builder、30 项列表测试通过，退出 0 (`/tmp/luna-tools-fpif8twy/result.json`)。
  覆盖位置与类型同步、草稿快照、负数/大索引、空草稿、全部删除后重插入，以及
  重名发布拒绝后修正并重新构造。
- `python3 -B development/library_check.py --release --stage sysroot-builders --stage sysroot-meta`：
  结构体两项、列表一项实际 host 测试通过，退出 0
  (`/tmp/luna-library-01cltl9i/summary.json`，scope=selected)。
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-edited-struct.fl --sysroot library --test-native --opt-level 3`：
  两项 O3 测试通过 (`/tmp/luna-tools-tmatwgvj/result.json`)。
- `python3 -B -m unittest discover -s development -p test_library_check.py`：
  13 项通过，包含少测、多测、零测试和失败结果拒绝。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-jmifkppq/result.json`)；`git diff --check` 通过。

初版库代码遗漏了指针字段投影的显式解引用；修正为 `self.field_names.*` 等
读写，未改动语言规则。首轮回归的重名诊断断言与实际不符
(`/tmp/luna-tools-fh7eacsv/result.json`)：构造服务当前返回通用 interpretation
Type 错误，不保留具体 duplicate 消息。最终回归断言实际错误并验证修正冲突
后的构造成功。字段默认代码、属性、泛型子句与 impl 构造仍待后续接通。

## definition builder 的草稿诊断

草稿输入校验此前在 VM 内直接返回通用 Type 错误，具体原因未进入发布服务的
诊断通道。本阶段增加 `ErrorKind::InvalidDefinition(DefinitionBuildError)`：
字段数量、重名字段、重名 variant、payload 形状和非法路径分别保留结构化原因。
Display 使用转义后的 S-expression，外层 Error 继续携带构造表达式源码范围。
类型形成、预算和发布事务错误仍使用既有分类；这不是对所有构造错误的完整覆盖。

enum 回归在同一 VM 里先提交重复 payload 字段，要求发布回调次数为零且没有
安装定义；修正后调用一次并成功安装。源码回归检查具体名称/数量与源码范围，
并将上一阶段的重名字段通用 Type 断言升级为 duplicate-field 断言。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib`：
  当时 167 项通过 (`/tmp/luna-tools-oxhm_ydw/result.json`)；随后新增同 VM 恢复
  用例，执行 `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib enum_build::tests`：
  六项通过 (`/tmp/luna-tools-hko2lrpg/result.json`)，未重复全部 VM 测试。
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations --test generated_definitions --test reflected_definitions`：
  60 项通过，退出 0 (`/tmp/luna-tools-iezjulv_/result.json`)。
- `python3 -B development/library_check.py --release --stage sysroot-builders`：
  两项实际原生用例通过 (`/tmp/luna-library-qxp1pd4m/summary.json`)。
- 以完整 sysroot 编译 `/tmp/luna-invalid-builder.fl`：Broken builder 连续添加
  两个 flag 字段后 Build，typealias 使用该结果。命令
  `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna /tmp/luna-invalid-builder.fl --sysroot library --test-native`
  按预期退出 1 (`/tmp/luna-tools-_jhlor4y/result.json`)，包含 DuplicateField、
  Broken、flag 和构造源码范围。即时 Type computation 的外层仍采用既有 Debug
  包装，虽保留结构化原因但尚未统一为直接 invocation 的 Display 形式。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-m2pgbfd3/result.json`)；`git diff --check` 通过。

## build 包的定义来源快照基础

`luna::build_context` 建立版本 1 的显式构建输入 API：按 SourcePackage 的语义
identity 保存 mode、执行宿主与 host pipeline 目标，拒绝缺失/重复输入和未知
身份。resolution ID 与语义 identity 分别校验，查询接收原始 DeclarationId，
不会按包名、路径或当前调用者猜配置。TargetData 继续提供校验过的指针宽度。

每次读取返回 Observation，保留 owner、query 和 value；新快照只比较被读取
的字段。目标 feature 缺失与 false 不混同，其他包或其他字段变化不使该读失效，
来源包消失则失效。快照与读取证据采用稳定、转义的 S-expression Display。

本阶段仅交付驱动 API，未接通源码 build.mode/target、comptime 查询或缓存。
没有创建虚假的 build 常量来掩盖这一缺口。package_metadata、构建脚本工具及
anar 传输同样尚待实现。下一步应接通定义来源查询与缓存证据，而不是将当前
单元测试当作完整 build 包验收。详见 `development/build-context.md`。

本阶段验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context`：
  四项契约测试通过 (`/tmp/luna-tools-he7t4eq9/result.json`)，覆盖来源包、宿主/
  目标差异、精确失效、缺失与关闭 feature、输入拒绝、稳定转义与括号平衡。
- `python3 -B development/library_check.py --release --stage manifest --stage sysroot-meta`：
  manifest 阶段通过，但 sysroot-meta 最初失败
  (`/tmp/luna-library-gobmpzd0/summary.json`)：仅放 README 的 library/build 目录
  也会被识别为已安装包，缺少 lib.fl。已将说明移到 development/build-context.md，
  删除本次创建的空目录；没有添加假实现绕过加载约束。
- 重新执行 `python3 -B development/library_check.py --release --stage sysroot-meta`：
  实际原生用例通过 (`/tmp/luna-library-p5ubztct/summary.json`)。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-3lhqzidf/result.json`)；`git diff --check` 通过。

## build.mode 的源码读取

新增实际 `library/build/lib.fl`。mode 是以编译器内部 marker 声明的上下文
常量；普通名称解析先确定声明，读取再按表达式原始 HIR owner 的语义包身份
取得驱动显式输入。别名不改变身份，普通同名常量不触发上下文读取。
`Snapshot::configure_modes` 完全替换 Protocols 的 mode 表，清除旧配置残留。
缺失或空 mode 拒绝，不借用调用者或 build 提供者的模式，也不读进程环境。

首次源码回归发现 inline if 的条件在 required evaluation 后复查普通分支时
被当成 runtime 输入，从而错误地要求 Splice。现在已选定 inline 条件的复查
继续处于 comptime 上下文；分支体恢复原阶段，未选分支仍需通过普通检查。
普通 runtime 使用 mode 仍调用原有 Splice 路径。

CLI/流式请求尚未提供 mode 传输，编译缓存尚未消费快照观察证据。当前调用方
使用 Rust 驱动 API，必须把配置纳入自身重编译条件；build.target 与 metadata
的源码查询仍未接线。不能将当前普通定义归属测试扩张为全部 template/CMTT
生成场景的保证。

本阶段验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context --test inline_control`：
  九项构建上下文、250 项 inline control 测试通过
  (`/tmp/luna-tools-ry8ax4jv/result.json`)，包括真实 build 包的跨包常量/别名读取、
  配置变化、缺失拒绝、局部遮蔽、依赖 inline 函数及 Splice 边界。
- 首轮 `/tmp/luna-tools-10l64dwv/result.json` 有两项失败：测试中 matches 表达式
  缺少限定范围的括号，以及上述 inline 条件阶段错误；均修复后重跑通过。
- `python3 -B development/library_check.py --release --stage manifest --stage sysroot-meta --stage sysroot-builders`：
  manifest 及三项完整 sysroot 原生用例通过
  (`/tmp/luna-library-2pqqnzmy/summary.json`，scope=selected)。这些用例证明新增
  build 包可加载且不要求未读取的 mode；不替代配置读取的源码 API 测试。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-80msd4rl/result.json`)；`git diff --check` 通过。

## build.mode 的进程入口与缓存隔离

新增 `--build-mode MODE`（仅主源码包）及可重复的
`--package-build-mode IDENTITY=MODE`（语义包身份）。普通 CLI、显式包 CLI 和
版本 1 驻留流式请求共用 ModeOptions，拒绝空值、重复/冲突绑定、未知身份和
不执行 elaboration 的输出模式。Options 的显式初始化点同步增加空配置，原有
无配置行为不变；驱动快照与 CLI 值冲突时拒绝而非静默覆盖。

完整请求缓存键现在包含配置，因此不同 mode 不会复用同一个残余；恢复原配置
可以命中对应旧结果，失败请求不回退到其他模式。当前是保守失效，尚未使用
字段级 Observation。新增 `sysroot-build-mode` 原生检查阶段，实际读取别名并
经普通常量保存配置，再用 inline if 消费结果。

验证（工作目录 `/home/fin/luna`）：

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli --test build_context --test compilation_cache`：
  最后一次组合运行中 11 项上下文、9 项缓存测试通过；CLI 为 225 项通过、
  一项部分 sysroot fixture 失败 (`/tmp/luna-tools-8dgd9dw0/result.json`)。
  修正该 fixture 恢复 manifest 时误带 Iterator/IntoIterator 绑定后，运行
  `python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli sysroot_language_items_bind_real_list_and_tuple_definitions_by_default`：
  该项通过 (`/tmp/luna-tools-lnpe29ld/result.json`)。未重复其余已通过用例。
- 较早组合运行还暴露两个旧属性拒绝文案断言，以及上述 fixture 初始 manifest
  缺模块。复现未定义 attrs/options 表达式后，按实际表达式检查诊断更新断言；
  仍要求拒绝，未放宽执行行为。对应失败报告为 txuudxzt、37m6j88w；独立诊断
  报告为 `/tmp/luna-tools-mraj6het/result.json`、`/tmp/luna-tools-env5kpki/result.json`。
- `python3 -B development/library_check.py --release --stage sysroot-build-mode`：
  实际原生用例通过 (`/tmp/luna-library-l_rl_ibf/summary.json`)。
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-build-mode.fl --sysroot library --build-mode debug --test-native --opt-level 3`：
  O3 通过 (`/tmp/luna-tools-q56pyhog/result.json`)。
- `python3 -B -m unittest discover -s development -p test_library_check.py`：13 项通过。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`：
  通过 (`/tmp/luna-tools-h212bgsr/result.json`)；`git diff --check` 通过。


### Build target snapshots in source (2026-10-28)

Connected the real build package's `target: Target` contextual constant to
explicit source-package snapshots. Target values retain nominal declaration
identity and expose arch, os, pointer_bits, and `features.get(Symbol) -> ?bool`.
The feature query is ordinary comptime library code over an ordered Object;
missing, disabled and enabled remain distinct. Aliases, saved constants and
inlined dependency functions retain their definition's configuration. Concrete
compilation layouts must agree with the selected target layout; execution-host
facts never substitute for target data. `Snapshot::configure` replaces both mode
and target tables, removing stale package and feature entries.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context --test compilation_cache`
  exited 0: 18 build-context and 9 cache tests passed. The build-context suite
  loads the complete repository sysroot and executes both cases in
  `development/examples/sysroot-build-target.fl` at O0 and O3. It also checks
  dependency origin, feature optionality, missing/invalid inputs, target-layout
  conflict and recovery, shadowing, Splice rejection and stable escaped dumps.
  Evidence: `/tmp/luna-tools-8pkj4348/result.json` and `stdout.log`.
- `python3 -B development/library_check.py --release --stage sysroot-build-mode --stage manifest`
  exited 0; actual sysroot mode test and manifest checks passed.
  Evidence: `/tmp/luna-library-asb00cbq/summary.json`.
- `python3 -B -m unittest discover -s development -p test_library_check.py`
  exited 0, 13 runner tests passed after adding the build-context stage.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
  exited 0; `/tmp/luna-tools-izofi2dx/result.json`. `git diff --check` passed.

Earlier local checks exposed and fixed a fixture calling an entrypoint whose
fourth argument is a builtin package ID rather than TargetData
(`/tmp/luna-tools-8cj6k807`), invalid Rust-style pub field syntax in the new
Flurry declaration (`/tmp/luna-tools-q3yb1g3u`), and a native fixture missing its
explicit NIR stage (`/tmp/luna-tools-vduoztbk`). The final combined run above
covers all repaired cases.

Complete target snapshots currently enter through the Rust embedding API.
CLI/resident target transport, observation-selective cache invalidation,
package_metadata and build scripts remain pending. The existing mode-only
CLI/cache behavior remains covered; target snapshot cache reuse is not claimed.


### Target configuration in CLI and resident requests (2026-10-28)

Added complete per-package target inputs to ordinary commands, explicit package
requests and resident frames. Primary and semantic-ID forms supply architecture,
OS, pointer width and optional Boolean features. Feature flags can precede target
flags; orphan features, duplicate inputs, unknown identities, conflicting driver
snapshots and ambiguous primary bindings reject rather than silently override.
Primary pointer width supplies NIR TargetData, while native actions require it
to match the actual backend layout. These are build-context inputs, not LLVM
triple/CPU selection flags.

The full parsed request includes targets and feature presence. Cache tests alter
architecture, OS, pointer width, enabled/disabled/absent features independently,
check the resulting runtime Boolean, then recover the original cached result.
Actual resident frames check reuse, changed input, missing input failure and
recovery. The public sysroot fixture now runs directly from the CLI.

Validation in `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context --test compilation_cache`
  exited 0: 20 build-context and 10 cache tests passed, including complete sysroot
  O0/O3 execution. Evidence: `/tmp/luna-tools-dwkam_q6/result.json`, `stdout.log`.
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --lib cli::tests`
  exited 0: 7 ordinary/package CLI unit tests passed, 73 filtered out.
  Evidence: `/tmp/luna-tools-c9x21fru/result.json`.
- `python3 -B development/library_check.py --release --stage sysroot-build-target --stage sysroot-build-mode`
  exited 0: two native target cases and one mode case passed via real CLI inputs.
  Evidence: `/tmp/luna-library-se_35bmp/summary.json`.
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-build-target.fl --sysroot library --build-target configured_arch configured_os 64 --build-feature vector=true --build-feature disabled=false --test-native --opt-level 3`
  exited 0: both native cases passed at O3.
  Evidence: `/tmp/luna-tools-wuf8yrln/result.json`, `stdout.log`.
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-build-target.fl --sysroot library --build-target configured_arch configured_os 32 --build-feature vector=true --test-native`
  exited 1 as expected with `build target layout conflicts with the compilation target`.
  Evidence: `/tmp/luna-tools-o1nwiqf9/result.json`, `stderr.log`.
- `python3 -B -m unittest discover -s development -p test_library_check.py`
  exited 0: 13 orchestration tests passed, including rejection of missing native
  target test results. `git diff --check` passed.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
  exited 0; `/tmp/luna-tools-jf42azjq/result.json`.

Observation-selective reuse, typed package metadata and build scripts remain
pending. No cross-compilation backend selection is claimed. Options literals
across the existing tests received the new default target field; unrelated
suites were not rerun for that mechanical API update.


### Source provenance through templates and generated members (2026-10-28)

Cross-package signature-template defaults previously acquired fresh HIR IDs
without source provenance. Reading build.mode/build.target from such a default
looked up semantic package 0 rather than the template provider. Reproduced both
failures in `/tmp/luna-tools-rsqbti6k/result.json` (21 passed, 2 failed).

HIR now records source origins separately from binding/declaration identities.
Rebind carries the original DefId without remapping it to generated definitions,
including when cloning a clone. Signature expansion installs this evidence;
generated members publish it through the existing checked DefinitionBatch.
Rejected/prepared drafts do not leak provenance. Package copies charge its
allocated map capacity, and both HIR and resolution evidence expose it in stable
S-expression dumps. Contextual reads without a source definition now reject
rather than silently falling back to package 0.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p hir --test definitions`
  exited 0: 23 tests passed, including provenance publication, rejected draft
  isolation, invalid identity rejection and dump visibility.
  Evidence: `/tmp/luna-tools-h_48v5gs/result.json`.
  An earlier compile (`/tmp/luna-tools-4_vigymk/result.json`) caught the new table
  missing from exhaustive copy-budget accounting; accounting was added before
  this successful run.
- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test build_context --test test_templates --test quote_functions --test derivation_attributes`
  exited 0: 24 build-context, 81 template, 104 quote-function and 10 derivation
  attribute tests passed (219 total). The build tests distinguish provider
  defaults from consumer bodies across fn/inline fn/quote fn instances, check
  target defaults and derived members, and compose provider quotes with caller
  code while checking missing-provider diagnostics. Existing real sysroot
  target tests execute at O0/O3 in the same suite.
  Evidence: `/tmp/luna-tools-_hgf52wr/result.json`, `stdout.log`.
- `python3 -B development/check.py --timeout 900 -- test --release -p elaboration --lib function_templates::member_clone`
  exited 0: 9 tests passed, including cloning a published clone while retaining
  its original source despite generated declaration remapping.
  Evidence: `/tmp/luna-tools-bnxufup5/result.json`.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
  exited 0: `/tmp/luna-tools-rz0_j3nr/result.json`. `git diff --check` passed.

This fixes implemented signature-template and member-cloning paths; shared-body
function templates and all remaining generation features are not thereby
implemented. Build observations still use conservative request-wide cache keys.


### Editable enum definition drafts (2026-10-28)

Extended the real meta.EnumBuilder with insert/replace operations for unit,
tuple and named variants, remove_variant, and insert/replace/remove_group.
Indices refer to logical declaration order. Invalid bounds leave the draft
unchanged; each variant edit prepares all parallel replacement lists before
writing any slot. Group changes leave descendants explicit, and Build still
rejects unrepaired parents, duplicate paths and malformed payloads. Published
Types and saved drafts remain independent. Payload normal-form heads are
preserved, without asserting equality or inequality to their factory bodies.

Validation from `/home/fin/luna`:

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations`
  exited 0: 61 tests passed at `/tmp/luna-tools-r2c8xag8/result.json`, covering the
  new edits, empty drafts, snapshots, invalid bounds, rejected Build and repair.
- After adding nested-parent/opaque-payload coverage and a named-to-unit check,
  `python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations enum_builder_`
  exited 0: 10 relevant tests passed, 52 filtered out. Evidence:
  `/tmp/luna-tools-zq7m_cjo/result.json`. The unaffected cases were not repeated.
- `python3 -B development/library_check.py --release --stage sysroot-enum-builder --stage sysroot-builders`
  exited 0: two edited enum and two existing struct cases passed with the real
  sysroot, public builder APIs and native resource checking.
  Evidence: `/tmp/luna-library-j26t8mzf/summary.json`.
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-edited-enum.fl --sysroot library --test-native --opt-level 3`
  exited 0, both enum cases passed; `/tmp/luna-tools-v23vegi4/result.json`.
- `python3 -B -m unittest discover -s development -p test_library_check.py`
  exited 0: 13 orchestration tests passed with strict native result counts.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`
  exited 0; `/tmp/luna-tools-c8roatnr/result.json`. `git diff --check` passed.

Initial fixture predicates called mutating editors directly inside assumes;
`/tmp/luna-tools-3gxbg03o` rejected those effectful specifications. Examples now
bind call results before checking predicates. An attempted whole-builder pointer
replacement was rejected for missing aggregate Copy/Drop evidence
(`/tmp/luna-tools-urzv50mc`); the final code uses the established immutable List
slot path and does not relax aggregate permissions. An initial opaque Type
inequality check was unresolved (`/tmp/luna-tools-8nddagfj`); the corrected test
checks reflexive Type identity and inspects the preserved NIR head instead.

Enum ABI encodings, generic declaration builders, field defaults, attributes,
and function/trait/impl builders remain separate incomplete work. These editor
methods do not claim complete SoA/definition-builder support.


### Symbol-encoded enum construction through meta builders

`meta.EnumBuilder.BuildSymbol(codes)` now publishes a fresh nominal enum using
an explicit Symbol encoding for each leaf in declaration order. Groups do not
consume code entries. Unit shape, code count and disjoint codes are checked
before the host callback; the returned encoding must match the request before
commit. Ordinary `Build` remains unencoded. Both VM admission and elaboration
registration account for encoding data when copying or comparing descriptors.
Fixed-width and generic enum construction remain unimplemented.

Validation (working directory `/home/fin/luna`):

- `python3 -B development/check.py --timeout 900 -- test --release -p luna --test builder_invocations`:
  65 passed, `/tmp/luna-tools-_o2iz6vd/result.json`. Covers code preservation,
  fresh identities, ordinary/symbol builds from one draft, empty definitions,
  mismatched counts, duplicate codes and payload rejection.
- `python3 -B development/check.py --timeout 900 -- test --release -p comptime_interpretor --lib`:
  173 passed, `/tmp/luna-tools-_3zzx9si/result.json`. Includes host replacement
  rejection before commit and a successful same-VM retry.
- `python3 -B development/library_check.py --release --timeout 900 --stage sysroot-symbol-enum --stage sysroot-enum-builder`:
  both selected stages passed, two native cases each;
  `/tmp/luna-library-zqi754hb/summary.json`.
- `python3 -B development/command.py --timeout 120 -- /tmp/luna-dev-002/target/release/luna development/examples/sysroot-symbol-enum.fl --sysroot library --test-native --opt-level 3`:
  two passed, `/tmp/luna-tools-q43hojwv/result.json`. Exercises generated enum
  values, parameter/return transport, grouped matching and reflection.
- `python3 -B -m unittest discover -s development -p test_library_check.py`:
  13 passed, including exact native case counts for the new stage.
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check`:
  passed, `/tmp/luna-tools-o2tbts4d/result.json`.
- `python3 -B development/check.py --timeout 900 -- test --release -p elaboration --test allocated_definition_publisher`:
  four passed, `/tmp/luna-tools-sb2jgs_8/result.json`; allocation, abandoned/stale
  publication, budget exhaustion and request dump regressions remain intact.
