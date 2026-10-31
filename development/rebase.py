#!/usr/bin/env python3
"""Guarded local dev history cleanup; no fetch, push, reset, stash or skip.

Plans keep every commit in its original order, using only pick/fixup. Start
requires a clean tracked worktree. Untracked files are left to Git's normal
overwrite protection. A backup ref survives success, failure and abort.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parent.parent


def git(*args, capture=True, env=None):
    return subprocess.run(['git', *args], cwd=ROOT, check=True, text=True,
                          stdin=subprocess.DEVNULL, capture_output=capture,
                          env=env, timeout=300).stdout


def oid(ref):
    return git('rev-parse', '--verify', '--end-of-options', ref + '^{commit}').strip()


def git_path(name):
    return Path(git('rev-parse', '--path-format=absolute', '--git-path', name).strip())


def active():
    return git_path('rebase-merge').exists() or git_path('rebase-apply').exists()


def validate(plan):
    if git('branch', '--show-current').strip() != 'dev':
        raise ValueError('history cleanup is restricted to dev')
    if oid('HEAD') != plan['head']:
        raise ValueError('HEAD changed since planning; prepare a new plan')
    base = oid(plan['base'])
    git('merge-base', '--is-ancestor', base, 'HEAD')
    if git('rev-list', '--merges', base + '..HEAD').strip():
        raise ValueError('merge commits require a separately reviewed strategy')
    commits = git('rev-list', '--reverse', base + '..HEAD').splitlines()
    rows = plan['commits']
    if not commits or [row['commit'] for row in rows] != commits:
        raise ValueError('plan must retain all commits exactly once in original order')
    if rows[0]['action'] != 'pick' or any(r['action'] not in ('pick', 'fixup') for r in rows):
        raise ValueError('only pick/fixup are allowed; first commit must be pick')
    return '\n'.join(r['action'] + ' ' + r['commit'] for r in rows) + '\n'


def finish(marker):
    if active():
        return
    state = json.loads(marker.read_text())
    if git('branch', '--show-current').strip() != 'dev':
        raise ValueError('unexpected branch after rebase; backup/state preserved')
    if git('rev-parse', 'HEAD^{tree}').strip() != state['tree']:
        raise ValueError('final tree differs from original; backup/state preserved for inspection')
    print('Verified identical final tree. Recovery ref: ' + state['backup'])
    marker.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    prepare = sub.add_parser('plan')
    prepare.add_argument('--base', required=True)
    prepare.add_argument('--output', type=Path, required=True)
    for command in ('preview', 'start'):
        sub.add_parser(command).add_argument('--plan', type=Path, required=True)
    for command in ('status', 'continue', 'abort'):
        sub.add_parser(command)
    args = parser.parse_args()
    marker = git_path('luna-history-rebase.json')
    if args.command == 'status':
        print(marker.read_text() if marker.exists() else 'No managed rebase.')
        git('status', '--short', '--branch', capture=False)
        return
    if args.command in ('continue', 'abort'):
        if not marker.exists():
            raise ValueError('no managed rebase; refusing to touch another operation')
        if active():
            head_name = git_path('rebase-merge/head-name')
            if not head_name.exists() or head_name.read_text().strip() != 'refs/heads/dev':
                raise ValueError('unexpected rebase owner; inspect manually')
            git('rebase', '--' + args.command, capture=False,
                env={**os.environ, 'GIT_EDITOR': 'true'})
        finish(marker)
        return
    if active() or marker.exists():
        raise ValueError('existing rebase/state; use status, continue or abort')
    if args.command == 'plan':
        base = oid(args.base)
        rows = [{'action': 'pick', 'commit': c,
                 'subject': git('show', '-s', '--format=%s', c).strip()}
                for c in git('rev-list', '--reverse', base + '..HEAD').splitlines()]
        plan = {'base': base, 'head': oid('HEAD'), 'commits': rows}
        validate(plan)
        with args.output.open('x') as output:
            output.write(json.dumps(plan, indent=2, ensure_ascii=False) + '\n')
        print(args.output)
        return
    plan = json.loads(args.plan.read_text())
    todo = validate(plan)
    if args.command == 'preview':
        for row in plan['commits']:
            print(row['action'], row['commit'][:12], row.get('subject', ''))
        git('diff', '--stat', plan['base'], plan['head'], capture=False)
        return
    if git('status', '--porcelain', '--untracked-files=no').strip():
        raise ValueError('tracked worktree/index is dirty; preserve or commit it before rebase')
    backup = 'refs/backup/dev-before-rebase-' + uuid.uuid4().hex
    state = {'backup': backup, 'head': plan['head'],
             'tree': git('rev-parse', 'HEAD^{tree}').strip(), 'plan': plan}
    with marker.open('x') as output:
        output.write(json.dumps(state, indent=2) + '\n')
    git('update-ref', backup, plan['head'], '0' * len(plan['head']))
    print('Recovery ref: ' + backup, flush=True)
    with tempfile.TemporaryDirectory(prefix='luna-rebase-') as temporary:
        path = Path(temporary) / 'todo'
        path.write_text(todo)
        env = {**os.environ, 'GIT_EDITOR': 'true',
               'GIT_SEQUENCE_EDITOR': shlex.join(['cp', '--', str(path)])}
        git('-c', 'rebase.autoStash=false', 'rebase', '-i', '--no-autosquash',
            '--no-update-refs', '--keep-empty', '--empty=keep', plan['base'],
            capture=False, env=env)
    finish(marker)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print('History operation stopped; inspect status. ' + str(error), file=sys.stderr)
        sys.exit(1)
