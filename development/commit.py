#!/usr/bin/env python3
"""Commit an explicit repository-relative file manifest on dev, without pushing.

Does not reset, clean, stash, force, bypass hooks, or change file contents. The
index must start empty so another person's staged work cannot enter the commit.
On failure, preserve the index for inspection rather than guessing a rollback.
"""
import argparse
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def git(*arguments, capture=False):
    return subprocess.run(['git', *arguments], cwd=ROOT, stdin=subprocess.DEVNULL,
                          check=True, text=True, capture_output=capture, timeout=120)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True,
                        help='UTF-8 file containing one repository-relative file path per line')
    parser.add_argument('--message-file', type=Path, required=True)
    parser.add_argument('--patch-file', type=Path,
                        help='stage only this text patch; its paths must exactly match the manifest')
    parser.add_argument('--dry-run', action='store_true',
                        help='validate and preview the selection without changing the index or HEAD')
    args = parser.parse_args()
    if git('branch', '--show-current', capture=True).stdout.strip() != 'dev':
        parser.error('this entrypoint commits only on dev')
    if git('diff', '--cached', '--name-only', capture=True).stdout:
        parser.error('index is not empty; inspect existing staged work first')
    paths = args.manifest.read_text().splitlines()
    if not paths or len(paths) != len(set(paths)):
        parser.error('manifest must be nonempty with no duplicate paths')
    for name in paths:
        path = Path(name)
        if (not name or path.is_absolute() or '..' in path.parts or '.git' in path.parts
                or str(path) != name or '\x00' in name):
            parser.error(f'not a repository-relative file path: {name!r}')
        if (ROOT / path).is_dir():
            parser.error(f'directory entries are not allowed: {name}')
    message = args.message_file.resolve()
    if not message.read_text().strip():
        parser.error('commit message is empty')
    patch = args.patch_file.resolve() if args.patch_file else None
    if patch:
        rows = git('apply', '--numstat', '-z', str(patch), capture=True).stdout.split('\0')
        selected = []
        for row in filter(None, rows):
            fields = row.split('\t', 2)
            if len(fields) != 3 or not all(value.isdecimal() for value in fields[:2]):
                parser.error('patch must contain ordinary text changes')
            selected.append(fields[2])
        if len(selected) != len(set(selected)) or set(selected) != set(paths):
            parser.error('patch paths must exactly match the manifest, with no duplicates')
        # Check both sides before changing the index. A selected hunk must apply
        # to HEAD and already exist in the working files being committed.
        git('apply', '--cached', '--check', str(patch))
        git('apply', '--reverse', '--check', str(patch))
    if args.dry_run:
        print('Selected paths:')
        for name in paths:
            print(f'  {name}')
        print('Commit message:')
        print(message.read_text())
        git('--literal-pathspecs', 'status', '--short', '--', *paths)
        if patch:
            git('apply', '--stat', str(patch))
        else:
            git('--literal-pathspecs', 'diff', '--stat', '--', *paths)
        return 0
    # Literal pathspecs prevent manifest entries from selecting other files.
    if patch:
        git('apply', '--cached', str(patch))
    else:
        git('--literal-pathspecs', 'add', '--', *paths)
    git('diff', '--cached', '--check')
    git('diff', '--cached', '--stat')
    git('commit', '--file', str(message))
    git('log', '-1', '--format=%h %s')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, subprocess.SubprocessError) as error:
        print(f'commit failed; inspect git status before retrying: {error}', file=sys.stderr)
        sys.exit(1)
