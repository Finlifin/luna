# 六包目录迁移验收（2026-10-29）

工作目录：`/home/fin/luna`。布局与未实现边界见
[源码结构](../library/STRUCTURE.md)。迁移保持常用公开再导出，显式内部 selector
随真实声明位置更新；不将预留目录计为已实现 API。

以下命令均通过（退出 0）：

```sh
python3 -B development/check.py --timeout 900 -- test --release -p luna --lib --test frontend_cli sysroot
python3 -B development/check.py --timeout 900 -- test --release -p luna --test sysroot_preludes --test library_meta_math --test library_protocols
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli shipped_library_files_use_current_syntax
python3 -B -m unittest discover -s development -p test_host_examples.py
python3 -B -m unittest discover -s development -p test_library_check.py
python3 -B development/command.py --timeout 120 -- cargo fmt --all -- --check
```

前两组分别通过 22 和 39 项测试；语法检查递归发现六包全部 `.fl` 文件。
证据分别为 `/tmp/luna-tools-s5rih5cr/result.json`、
`/tmp/luna-tools-5xghkyux/result.json`、`/tmp/luna-tools-cr848ei5/result.json`；
Python 两组分别通过 9 和 13 项；格式检查见
`/tmp/luna-tools-88wywmx9/result.json`。

此前组合运行中的 build_context（32）、library_attributes（6）、
library_meta_objects（21）全部通过，见 `/tmp/luna-tools-5am6f290/stdout.log`；
该次命令最后因 sysroot_preludes 的旧显式 selector 失败，修正后已用上面的
独立命令复核，不将该次组合命令称为成功。comptime_list_operations（41）在
`/tmp/luna-tools-64zx1stu/stdout.log` 中通过。

## 已有 Hash 缓存失败

该次较广检查随后执行 library_hash，8 项通过，以下两项报
`Protocol("interpretation InvalidCallPlan")`：

- `wide_hash_collisions_still_require_equality`
- `text_and_symbol_cache_keys_compare_content_and_recover_after_budget_exhaustion`

为区分迁移回归与已有问题，将相同测试的 attrs provider 替换为
`git show 4b1977cc:library/attrs/lib.fl` 的原始内容重新编译执行；其余被搬动
的协议文件正文未变。结果仍为相同两项失败、8 项通过，见
`/tmp/luna-tools-9tgpejve/result.json`。临时测试源保存为
`/tmp/luna-library-layout-baseline.rs`，已从仓库测试发现目录移出。
此问题不在目录迁移中修改，也未将 Hash 缓存验收标为通过。
