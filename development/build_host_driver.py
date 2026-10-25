#!/usr/bin/env python3
"""Build a private luna executable before the managed cache may reclaim it."""
import argparse
import errno
from pathlib import Path
import shutil
import sys

from cargo import ROOT, TARGET, Runner


def build(destination, *, release=False):
    if destination.exists() or destination.is_symlink():
        raise FileExistsError(errno.EEXIST, 'host driver destination already exists', str(destination))
    runner = Runner(ROOT, TARGET)
    # Never replace an existing program (including a dangling symlink).
    # Failure to copy leaves no partially executable driver behind.
    def publish():
        # Keep the inode open across cache cleanup. Rebuilding after a failed
        # copy would recreate the very archives that exhausted the quota.
        # Runner invokes this callback under its maintenance lock.
        with (TARGET / ('release' if release else 'debug') / 'luna').open('rb') as source:
            for attempt in range(2):
                created = False
                try:
                    with destination.open('xb') as output:
                        created = True
                        shutil.copyfileobj(source, output)
                    destination.chmod(0o700)
                    return
                except BaseException as error:
                    if created:
                        destination.unlink(missing_ok=True)
                    if (attempt == 0 and isinstance(error, OSError)
                            and error.errno in (errno.ENOSPC, errno.EDQUOT)):
                        runner.reclaim(force=True)
                        source.seek(0)
                        print('host driver: retrying copy once after cache cleanup',
                              file=sys.stderr, flush=True)
                        continue
                    raise

    arguments = ['build', '-p', 'luna', '--bin', 'luna']
    if release:
        arguments.append('--release')
    return runner.run(arguments, on_success=publish)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--release', action='store_true', help='build the compiler with Cargo release profile')
    args = parser.parse_args()
    try:
        sys.exit(build(args.destination, release=args.release))
    except OSError as error:
        print(f'host driver: {error}', file=sys.stderr)
        sys.exit(1)
