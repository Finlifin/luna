#!/usr/bin/env python3
"""One-time, tree-preserving squash/rebase of the pre-VM dev history.

Never checks out files or stages the active worktree. A private temporary index
splits the mixed compiler/tutorial commit. --apply creates a backup ref and
updates dev in one compare-and-swap reference transaction. Does not push.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
EXPECTED = '944e40ce'
BASE = 'a44f3d8e'
SNAPSHOT = '9af14ccc'
MIXED = '4536de37'


def git(*args, data=None, env=None):
    return subprocess.run(['git', *args], cwd=ROOT, input=data,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          env=env, check=True, timeout=120).stdout


def oid(ref):
    return git('rev-parse', ref).decode().strip()


def workspace():
    digest = hashlib.sha256(git('diff', '--binary', 'HEAD'))
    digest.update(git('diff', '--cached', '--binary'))
    for name in git('ls-files', '--others', '--exclude-standard', '-z').split(b'\0'):
        if not name:
            continue
        path = ROOT / os.fsdecode(name)
        digest.update(name + b'\0')
        digest.update(str(path.lstat().st_mode).encode() + b'\0')
        digest.update(os.fsencode(os.readlink(path)) if path.is_symlink() else path.read_bytes())
    return digest.hexdigest()


def commit(tree, parent, original, message):
    name, email, date = git('show', '-s', '--format=%an%x00%ae%x00%aI', original).decode().strip().split('\0')
    env = dict(os.environ, GIT_AUTHOR_NAME=name, GIT_AUTHOR_EMAIL=email, GIT_AUTHOR_DATE=date)
    return git('commit-tree', tree, '-p', parent, data=(message + '\n').encode(), env=env).decode().strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    old = oid(EXPECTED)
    if oid('HEAD') != old or git('branch', '--show-current').strip() != b'dev':
        raise RuntimeError('expected dev at 944e40ce; refusing to rewrite a different history')
    if git('diff', '--cached', '--name-only').strip():
        raise RuntimeError('active index contains staged changes')
    if git('status', '--porcelain=v1', '--untracked-files=no').decode().find('\nUU ') >= 0:
        raise RuntimeError('resolve existing conflicts first')
    paths = git('diff', '--name-only', SNAPSHOT, MIXED).decode().splitlines()
    if any(not p.startswith(('compiler/', 'development/', 'flurry_tutorial/')) for p in paths):
        raise RuntimeError('mixed commit contains unexpected paths')
    if any(not p.startswith('flurry_tutorial/') for p in git('diff', '--name-only', MIXED, old).decode().splitlines()):
        raise RuntimeError('last commit contains non-tutorial changes')
    plan = {
        'old_head': old, 'unchanged_parent': oid(BASE), 'final_tree': oid('HEAD^{tree}'),
        'new_commits': [
            'feat(compiler): establish semantic frontend and execution infrastructure',
            'feat(compiler): extend staged elaboration and verification',
            'docs(tutorial): add and refine the Flurry language tutorial',
        ],
        'unchanged_commits': ['62cd5333', BASE],
        'mixed_compiler_paths': sum(p.startswith(('compiler/', 'development/')) for p in paths),
        'mixed_tutorial_paths': sum(p.startswith('flurry_tutorial/') for p in paths),
    }
    print(json.dumps(plan, indent=2), flush=True)
    if not args.apply:
        return
    before = workspace()
    directory = Path(tempfile.mkdtemp(prefix='luna-history-rebase-'))
    env = dict(os.environ, GIT_INDEX_FILE=str(directory / 'index'))
    git('read-tree', SNAPSHOT, env=env)
    patch = git('diff', '--binary', SNAPSHOT, MIXED, '--', 'compiler', 'development')
    git('apply', '--cached', '--binary', data=patch, env=env)
    compiler_tree = git('write-tree', env=env).decode().strip()
    first = commit(oid(SNAPSHOT + '^{tree}'), oid(BASE), SNAPSHOT,
                   plan['new_commits'][0] + '\n\nConsolidate the eight development/cleanup commits from b8fa3571 through\n9af14ccc, including removal of accidentally tracked build outputs.\nPreserve the exact final snapshot of that sequence.')
    second = commit(compiler_tree, first, MIXED,
                    plan['new_commits'][1] + '\n\nPreserve the compiler and development changes from 4536de37.\nSeparate tutorial authoring from the compiler implementation checkpoint.')
    final = commit(plan['final_tree'], second, old,
                   plan['new_commits'][2] + '\n\nPreserve the tutorial changes from 4536de37 and the corrections from\n944e40ce. The final repository tree is unchanged by history cleanup.')
    if oid(final + '^{tree}') != plan['final_tree'] or workspace() != before:
        raise RuntimeError('tree/workspace changed during preparation; dev has not been updated')
    backup = 'refs/heads/backup/dev-before-history-cleanup-' + old[:8]
    transaction = f'start\ncreate {backup} {old}\nupdate refs/heads/dev {final} {old}\nprepare\ncommit\n'
    git('update-ref', '-m', 'rebase: consolidate development snapshots and separate tutorial', '--stdin', data=transaction.encode())
    after = workspace()
    report = dict(plan, new_head=final, new_commit_ids=[first, second, final],
                  backup_ref=backup, workspace_before=before, workspace_after=after,
                  tree_unchanged=oid('HEAD^{tree}') == plan['final_tree'])
    (directory / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(report=str(directory / 'result.json'), **report), indent=2))
    if before != after or not report['tree_unchanged']:
        raise RuntimeError('post-rewrite verification failed; preserve the backup and inspect')


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(error.stderr.decode(errors='replace'))
        raise SystemExit(error.returncode)
