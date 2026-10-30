# Luna CLI

Luna 使用子命令选择工作，再用该命令的选项配置输入、目标与产物。
不带参数显示总览；`luna help <command>` 与 `luna <command> --help`
显示对应帮助；`luna --version`（或 `-V`）显示编译器版本。
命令解析不读取源文件，也不启动编译。

| 命令 | 作用 | 默认行为 |
| --- | --- | --- |
| `check <input>` | 编译至 typed NIR，检查前端语义 | 成功时不输出产物 |
| `build <input> -o <path>` | 生成 host 可执行文件 | 入口 `main`；也可 `--emit object` / `bitcode` |
| `test <input>` | 运行测试 | native；`--engine reference` 使用参考解释器 |
| `dump <input>` | 输出编译产物 | NIR；用 `--emit` 选择格式 |
| `comptime <input> --entry <name>` | 编译期 invocation | 必须显式给入口 |
| `interpret <input>` | 参考执行 runtime 入口并打印结果 | 入口 `main` |
| `analyze resources <input>` | 输出所有权义务与清理计划 | 需要 `--copy-protocol` |
| `analyze text-regions <input>` | 输出文本读取／返回依赖 | 入口 `main` |
| `packages <command> ...` | 对显式包图执行上述命令 | 用 `--package` 与 `--primary` 提供输入 |
| `serve packages-v1` | 服务 stdin/stdout 包请求流 | 缓存容量 8，可用 `--cache-entries` 调整 |

`check` 不等于 verified mode 程序证明，`analyze` 也不证明完整内存安全。
`interpret` 明确表示参考执行；本次没有新增隐式选择后端的 `run` 命令。

## 常用命令

```sh
luna check app.fl --sysroot library
luna build app.fl --entry main -o app --sysroot library --opt-level 2
luna test app.fl --sysroot library
luna test app.fl --engine reference --filter parsing
luna test app.fl --list
luna dump app.fl --emit hir -o app.hir
luna dump app.fl --emit iir --entry main
luna dump app.fl --emit llvm --entry main --opt-level 2
luna build app.fl --emit object -o app.o
luna comptime app.fl --entry Schema --definition-fuel 1000000
luna interpret app.fl --entry main
luna analyze resources app.fl --entry main --copy-protocol Copy --drop-protocol Drop
luna dump -- -input.fl
```

`dump --emit` 支持 `ast`、`hir`、`nir`、`iir`、`llvm`、`asm`、
`attributes`、`merged-attributes`。只有 IIR、LLVM 和汇编 dump 接受
`--entry`；它们默认使用 `main`。二进制产物通过 `build --emit` 提供，
必须给输出路径。`-o` 是 `--output` 的别名。

`test --timeout-ms` 配置 native 测试的单项时限。`--list` 只发现测试，
不能与 `--engine` 或 `--timeout-ms` 混用。过滤通过 `--filter` 指定。

源路径和输出路径保持操作系统原生表示，允许非 UTF-8 路径；入口名、
格式名、协议选择器等语义名称仍要求 UTF-8。`--` 之后只接受一个源路径。
未知子命令、重复选项、无关动作选项及不合法组合在编译前拒绝，退出码为 2。
编译与执行错误沿用原错误传播；输出继续通过现有安全发布逻辑写入。

## 多包与流式调用

```sh
luna packages check --package 1 101 app app.fl --primary 1
luna packages dump --package 1 101 app app.fl --primary 1 --emit hir
luna packages comptime --package 1 101 app app.fl --primary 1 --entry inspect
luna packages build --package 1 101 app app.fl --primary 1 --entry main -o app
luna serve packages-v1 --cache-entries 8
```

多包命令使用与源命令相同的动作选项，输入则来自 `--package`、
`--dependency`、`--primary`、`--builtin` 和 `--builtin-impl-package`。
它不接受位置源路径。`--package-metadata-v1` 和 `--with-inputs`
保持既有协议；`packages check` 不接受输出包装。
流式请求内部的 v1 格式保持不变，避免混淆交互 CLI 与传输协议版本。

## 迁移

| 兼容入口 | 新入口 |
| --- | --- |
| `luna file.fl --emit nir` | `luna dump file.fl --emit nir` |
| `luna file.fl --build main --output app` | `luna build file.fl -o app` |
| `luna file.fl --test-native` | `luna test file.fl` |
| `luna file.fl --test` | `luna test file.fl --engine reference` |
| `luna file.fl --list-tests` | `luna test file.fl --list` |
| `luna file.fl --invoke-comptime f` | `luna comptime file.fl --entry f` |
| `luna file.fl --instantiate main` | `luna dump file.fl --emit iir` |
| `luna file.fl --interpret main` | `luna interpret file.fl` |
| `luna --packages-v1 ... --emit nir` | `luna packages dump ... --emit nir` |
| `luna --packages-stream-v1` | `luna serve packages-v1` |

旧的 source-first、`--packages-v1` 和 `--packages-stream-v1` 入口暂时保留，
用于兼容既有自动化；新命令不接受旧动作旗标混用。兼容入口原先默认输出 HIR，
新的 `dump` 默认 NIR。未知裸单词视为命令拼写错误；没有 `.fl` 后缀的源文件
应使用子命令或显式路径（例如 `./source`）。

开发脚本 `host_examples.py`、`library_check.py` 和 `run_host.py` 已改为
向 Luna 发出子命令。旧协议 fixtures 保留以继续验证兼容层。

## 实现与验证

公共命令语法位于 `compiler/luna/src/cli/subcommands.rs`。它先校验命令所属
参数，再进入共享的选项验证与既有执行管线；包流 v1 不经过交互命令解析，
避免改变传输协议。`check` 和 `packages check` 各有显式执行分支，不通过
丢弃已经格式化的 dump 来伪装检查模式。

本轮验证在 `/home/fin/luna` 执行，以下入口均退出 0：

- `python3 -B development/check.py --timeout 900 -- test -p luna --test subcommands`：
  7 项测试，包含真实 CLI 检查、dump、invocation、解释执行、测试发现、
  reference/native 测试、native 构建与执行、多包检查；
  `/tmp/luna-tools-35pywiz1/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test subcommands --lib cli`：
  9 项既有 CLI/parser 单元测试通过（该过滤器不运行 subcommands 用例）；
  `/tmp/luna-tools-fs_zwztx/result.json`。
- `python3 -B development/check.py --timeout 900 -- test -p luna --test frontend_cli usage`：
  5 项旧入口与诊断回归通过；`/tmp/luna-tools-z6aj6ea7/result.json`。
- `python3 -B -m unittest discover -s development -p 'test_*host*.py'`：25 项通过。
- `python3 -B -m unittest discover -s development -p 'test_library_check.py'`：13 项通过。
- `python3 -B development/command.py --timeout 120 -- cargo fmt --all`：
  `/tmp/luna-tools-0ouaw8is/result.json`；`git diff --check` 通过。

没有运行全量语言测试；本次未改变语言语义与后端实现。

实际内置包多包验收（2026-10-31）：
`python3 -B development/library_check.py --stage attributes --stage merged-attributes --stage meta-values --timeout 300`
退出 0，三个阶段的 `value_matches` 均为 true；证据为
`/tmp/luna-library-f01o3ubh/summary.json`。这次运行也纠正了两个旧示例遗漏
普通 `bool` 导入的问题，并给属性检查的包图补齐 builtin primitive 依赖。
未向 resolver 恢复隐式 primitive 名称。对应 runner 的 13 项单元测试通过。
