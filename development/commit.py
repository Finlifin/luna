#!/usr/bin/env python3
"""Commit an explicit repository-relative file manifest on dev, without pushing.

Does not reset, clean, stash, force, bypass hooks, or change file contents. The
index must start empty unless --include-staged explicitly adopts only manifest paths.
After a failed check, --restage PATH refreshes only the named staged file from
the worktree; other staged selections are preserved, including partial hunks.
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
    parser.add_argument('--include-staged', action='store_true',
                        help='preserve and include already staged manifest paths; reject other staged work')
    parser.add_argument('--restage', action='append', default=[], metavar='PATH',
                        help='with --include-staged, explicitly refresh this staged manifest file from the worktree')
    args = parser.parse_args()
    if git('branch', '--show-current', capture=True).stdout.strip() != 'dev':
        parser.error('this entrypoint commits only on dev')
    staged = set(filter(None, git('diff', '--cached', '--name-only', '--no-renames', '-z',
                                 capture=True).stdout.split('\0')))
    if staged and not args.include_staged:
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
    if not staged.issubset(paths):
        parser.error('staged paths outside the manifest; inspect existing staged work first')
    if args.include_staged and args.patch_file:
        parser.error('--include-staged cannot be combined with --patch-file')
    restage = set(args.restage)
    if restage and not args.include_staged:
        parser.error('--restage requires --include-staged')
    if len(restage) != len(args.restage) or not restage.issubset(staged):
        parser.error('--restage paths must be unique already staged manifest paths')
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
        if restage:
            print('Refresh staged files from worktree:')
            for name in paths:
                if name in restage:
                    print(f'  {name}')
        print('Commit message:')
        print(message.read_text())
        git('--literal-pathspecs', 'status', '--short', '--', *paths)
        if patch:
            git('apply', '--stat', str(patch))
        else:
            git('--literal-pathspecs', 'diff', '--stat', '--', *paths)
        if staged:
            git('diff', '--cached', '--stat')
        return 0
    # Literal pathspecs prevent manifest entries from selecting other files.
    if patch:
        git('apply', '--cached', str(patch))
    else:
        unstaged = [name for name in paths if name not in staged or name in restage]
        if unstaged:
            git('--literal-pathspecs', 'add', '--', *unstaged)
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
