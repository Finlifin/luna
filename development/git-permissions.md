# 本地提交入口与长效权限

在 `/home/fin/luna` 执行：

```sh
python3 -B development/commit.py --manifest /tmp/luna-commit-files.txt --message-file /tmp/luna-commit-message.txt
```

manifest 每行一个仓库相对文件路径；message 文件保存提交说明。后续提交只需更新这两个文件，执行命令可以保持相同。预览时追加 `--dry-run`，不会修改 index 或 HEAD。

可保存的权限前缀是 `python3 -B development/commit.py`。它涵盖该入口的后续调用及不同 manifest/message 参数，不是对任意 Python 或 Git 命令的授权。权限是否持久保存由客户端决定；脚本不能自行授予权限。当前会话已列出这一批准前缀。

入口只允许在 dev 提交明确列出的文件，要求暂存区初始为空，拒绝目录及越界路径，使用 literal pathspec，并执行暂存差异检查。它不 push、reset、clean、stash，不禁用 hooks。失败时保留暂存区供检查，不自动回滚。调用者仍需检查清单，避免提交无关改动或敏感文件。

`rebase_history.py` 是先前历史整理的一次性工具，绑定旧 HEAD `944e40ce`，已在当前历史执行完成。不要将它作为日常提交入口，也不要为了重复运行而放宽其 HEAD 检查。未来新的历史改写需要独立计划及相应授权。

验证：`python3 -B -m unittest development.test_commit`。测试在临时仓库执行，覆盖选择性提交、已有暂存内容与非法清单拒绝、预览不改变 HEAD/index/工作区。
