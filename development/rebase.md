# 本地提交整理入口

固定入口为 `python3 -B development/rebase.py`。环境授权时可限定此前缀；
普通提交继续使用已有的 `python3 -B development/commit.py`。
脚本不绕过环境审批，不执行 fetch、push、reset、clean 或 stash。

```sh
python3 -B development/rebase.py plan --base <明确的基线提交> --output /tmp/luna-history-plan.json
python3 -B development/rebase.py preview --plan /tmp/luna-history-plan.json
python3 -B development/rebase.py start --plan /tmp/luna-history-plan.json
python3 -B development/rebase.py status
python3 -B development/rebase.py continue
python3 -B development/rebase.py abort
```

`plan` 默认保留全部提交，固定基线和 HEAD。将相邻后续提交的 action 从
`pick` 改为 `fixup`，即可合入前一组并保留该组首个提交的消息。subject
仅用于阅读，不执行为命令。计划禁止删除、重排、exec、merge 和任意 Git
参数；HEAD 改变后必须重新生成计划。输出文件必须不存在，防止覆盖已有计划。

`start` 只接受 dev，要求已跟踪文件和暂存区干净；不自动处理工作区改动。
未跟踪文件保留，Git 的覆盖检查仍然生效。开始前创建唯一的
`refs/backup/dev-before-rebase-*` 引用，并在 Git 私有目录记录原 HEAD、
原文件树及完整计划。仅整理原基线之后的提交，不隐式改用远程分支作为基线。
成功后核对最终文件树与原 HEAD 完全一致，备份引用持续保留。

遇到冲突或超时不自动跳过提交，也不自动回滚。先 `status` 检查，解决并暂存
冲突后 `continue`；放弃本轮整理用 `abort`。两者仅处理脚本登记的 dev
rebase。文件树不一致时保留状态供检查，不宣称成功。

当前工作区已存在未提交的 callable reflection 实现，应先按其实际验证状态
妥善保存，再重新生成计划。当前 origin/dev 与 dev 已分叉；是否同步远程是
独立操作，不能通过本地历史整理悄悄覆盖远程改动。

脚本验证：`python3 -B -m unittest discover -s development -p test_rebase.py`。
测试只在临时仓库重写历史，覆盖实际 fixup、最终树、备份、未跟踪文件保留、
脏工作区拒绝、过期及非法计划拒绝、只读预览；不涉及项目构建。
