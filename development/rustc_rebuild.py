#!/usr/bin/env python3
"""Release only exact outputs of a managed rustc job that Cargo will rebuild.

Called by cargo.py under its maintenance lock and -j 1. Compiler queries,
metadata-only jobs, dependencies, cross targets and unsupported output modes are
forwarded without pruning. This is not a general cache deletion entrypoint.
"""
from pathlib import Path
import os
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path('/tmp/luna-dev-002/target')
PACKAGES = ('elaboration', 'luna')


def option(arguments, name):
    values = []
    for index, argument in enumerate(arguments):
        if argument == name:
            if index + 1 == len(arguments):
                return None
            values.append(arguments[index + 1])
        elif argument.startswith(name + '='):
            values.append(argument[len(name) + 1:])
    return values[0] if len(values) == 1 else None


def candidates(arguments, env, *, root=ROOT, target=TARGET, cwd=None):
    """Conservative filename selection, with no filesystem mutations."""
    if env.get('LUNA_MANAGED_RUSTC_PRUNE') != '1' or not arguments:
        return []
    if Path(arguments[0]).name not in ('rustc', 'rustc.exe'):
        return []  # Preserve any additional wrapper without interpreting its CLI.
    args = arguments[1:]
    if any(a.startswith(('-o', '--print', '--target', '@')) for a in args):
        return []
    package = env.get('CARGO_PKG_NAME')
    if package not in PACKAGES:
        return []
    manifest = root / 'compiler' / package
    if Path(env.get('CARGO_MANIFEST_DIR', '/')) != manifest:
        return []
    crate = option(args, '--crate-name')
    output = option(args, '--out-dir')
    emit = option(args, '--emit')
    if not crate or not re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*', crate) or not output:
        return []
    if not emit or 'link' not in emit.split(',') or any('=' in e for e in emit.split(',')):
        return []
    extra = []
    for index, argument in enumerate(args):
        value = args[index + 1] if argument == '-C' and index + 1 < len(args) else argument[2:] if argument.startswith('-C') else ''
        if value.startswith('extra-filename='):
            extra.append(value.removeprefix('extra-filename='))
    if len(extra) != 1 or not re.fullmatch(r'-[0-9a-f]{8,64}', extra[0]):
        return []
    working = Path.cwd() if cwd is None else Path(cwd)
    sources = [working / a for a in args if a.endswith('.rs') and not a.startswith('-')]
    if len(sources) != 1 or not sources[0].is_file() or not sources[0].resolve().is_relative_to(manifest.resolve()):
        return []
    directory = working / output
    # Reject symlink components as well as escapes from the managed target.
    if directory.resolve() != directory.absolute() or not directory.is_relative_to(target):
        return []
    kinds = option(args, '--crate-type')
    stem = crate + extra[0]
    if '--test' in args or kinds == 'bin':
        names = [stem]
    elif kinds in ('lib', 'rlib'):
        names = ['lib' + stem + '.rlib', 'lib' + stem + '.rmeta']
    else:
        return []
    paths = [directory / name for name in names]
    if any(p.is_symlink() or (p.exists() and not p.is_file()) for p in paths):
        return []
    return [p for p in paths if p.is_file()]


def prune(arguments, env, **paths):
    removed = 0
    for path in candidates(arguments, env, **paths):
        size = path.stat().st_size
        path.unlink()
        removed += size
    return removed


def main():
    arguments = sys.argv[1:]
    if not arguments:
        raise SystemExit('rustc_rebuild.py requires the compiler invocation')
    try:
        removed = prune(arguments, os.environ)
        if removed:
            print(f'cargo-cache: released {removed} bytes of outputs selected for rebuild', file=sys.stderr)
    except OSError as error:
        # A failed optimization is not permission to skip the requested compile.
        try:
            print(f'cargo-cache: rebuild-output pruning unavailable: {error}', file=sys.stderr)
        except OSError:
            pass
    os.execvp(arguments[0], arguments)


if __name__ == '__main__':
    main()
