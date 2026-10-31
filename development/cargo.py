#!/usr/bin/env python3
"""Bounded local Cargo cache; run from any directory, without shell expansion.

Usage: python3 development/cargo.py test -p luna --test execution
       python3 development/cargo.py run -p luna -- input.fl --emit nir
       python3 development/cargo.py clean

Only /tmp/luna-dev-002/target is cleaned. Source files, Cargo.lock, and the
registry/download cache are never cleanup targets. Builds and cleanup share
one lock; use this entrypoint instead of concurrent direct Cargo commands.

After an observed storage failure, retain a soft cache cap at 75% of the
pre-cleanup target footprint (64 MiB minimum, never above the current cap).
The default is 2 GiB. The cap lives in /tmp/luna-dev-002/target-cache-limit,
outside Cargo's cleanup scope. Remove that local file after increasing the
environment's quota to return to the default. It is a headroom heuristic, not
a measurement or guarantee of available quota; unrelated disk usage can still
cause failures. Evidence forwarding errors still never replay completed work.
"""
from __future__ import annotations

import fcntl
import errno
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path("/tmp/luna-dev-002/target")
MAX_CACHE = 2 * 1024**3
MIN_CACHE = 64 * 1024**2
MIN_FREE = 2 * 1024**3
REBUILDABLE_PACKAGES = ("elaboration", "luna")
SPACE_ERRORS = ("quota exceeded", "no space left on device", "os error 122", "os error 28")
SPACE_ERRNOS = (errno.EDQUOT, errno.ENOSPC)


def diagnostic(message: str) -> None:
    # Cache recovery must still run when the evidence destination is full.
    try:
        print(message, file=sys.stderr, flush=True)
    except OSError:
        pass


def cargo_arguments(arguments: list[str], target: Path) -> list[str]:
    if not arguments or arguments[0] not in {"build", "check", "test", "run", "clean"}:
        raise ValueError("expected build, check, test, run, or clean")
    split = arguments.index("--") if "--" in arguments else len(arguments)
    options, tail = arguments[:split], arguments[split:]
    if any(arg == "--target-dir" or arg.startswith("--target-dir=") for arg in options):
        raise ValueError("target-dir is managed by this entrypoint")
    # Keep caller-selected tests/packages/features; always use the shared cache.
    common = ["--target-dir", str(target)]
    if options[0] != "clean":
        common += ["-j", "1", "--locked", "--offline"]
    return ["cargo", *options, *common, *tail]


class Runner:
    def __init__(self, root: Path, target: Path):
        if target.resolve() != target.absolute():
            raise ValueError("managed target must not redirect through a symlink")
        self.root, self.target = root, target
        self.env = dict(os.environ)
        self.env.update(CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0")
        wrappers = ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
                    'CARGO_BUILD_RUSTC_WRAPPER', 'CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER')
        if not any(key in self.env for key in wrappers):
            self.env['RUSTC_WRAPPER'] = str(ROOT / 'development/rustc_rebuild.py')
            self.env['LUNA_MANAGED_RUSTC_PRUNE'] = '1'

    def cache_size(self) -> int:
        if not self.target.exists():
            return 0
        result = subprocess.run(["du", "-sk", str(self.target)], check=True, text=True, capture_output=True)
        return int(result.stdout.split()[0]) * 1024

    def pressure(self) -> bool:
        # df does not expose all container/user quotas. A cache cap is needed too.
        return self.cache_size() >= self.cache_limit() or shutil.disk_usage(self.target.parent).free < MIN_FREE

    def cache_limit(self) -> int:
        # Kept outside target so a full Cargo cleanup does not forget observed
        # quota pressure. This is a local soft limit, not a quota measurement.
        try:
            limit = int(self.target.with_name(self.target.name + '-cache-limit').read_text())
            if MIN_CACHE <= limit <= MAX_CACHE:
                return limit
        except (OSError, ValueError):
            pass
        return MAX_CACHE

    def remember_pressure(self, observed: int) -> None:
        if observed <= 0:
            return
        limit = min(self.cache_limit(), max(MIN_CACHE, observed * 3 // 4))
        state = self.target.with_name(self.target.name + '-cache-limit')
        temporary = state.with_suffix('.tmp')
        try:
            # Called under the maintenance lock, after reclaiming storage.
            temporary.write_text(f'{limit}\n')
            temporary.replace(state)
        except OSError as error:
            diagnostic(f'cargo-cache: could not retain soft limit: {error}')
            return
        diagnostic(f'cargo-cache: learned soft cache limit {limit} bytes')

    def clean(self, *, full: bool = False, packages: tuple[str, ...] = REBUILDABLE_PACKAGES) -> None:
        command = ["cargo", "clean", "--target-dir", str(self.target)]
        if not full:
            for package in packages:
                command += ["-p", package]
        diagnostic("cargo-cache: cleaning " + ("the target directory" if full else "/".join(packages) + " outputs"))
        subprocess.run(command, cwd=self.root, env=self.env, check=True)
        if not full:
            # Package-scoped clean may select only the default profile. The
            # managed cache also serves release builds; remove those package
            # outputs before considering a full dependency-cache purge.
            subprocess.run([*command, "--release"], cwd=self.root, env=self.env, check=True)

    def reclaim(self, *, force: bool = False) -> None:
        if force:
            # A reported quota failure is stronger evidence than disk_usage:
            # user/container quotas may be exhausted even below our cache cap.
            try:
                observed = self.cache_size()
            except (OSError, subprocess.SubprocessError, ValueError):
                observed = 0
            self.clean()
            self.clean(full=True)
            self.remember_pressure(observed)
            return
        if not self.pressure():
            return
        # Test harnesses are cheaper to rebuild than the compiler. Keep the
        # elaboration archive whenever removing luna outputs supplies headroom.
        self.clean(packages=("luna",))
        if not self.pressure():
            return
        self.clean(packages=("elaboration",))
        if self.pressure():
            self.clean(full=True)

    def execute(self, command: list[str]) -> tuple[int, bool]:
        process = subprocess.Popen(command, cwd=self.root, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert process.stdout is not None and process.stderr is not None
        quota_failure = threading.Event()
        forwarding_errors = []
        # Stream all evidence to the caller's log; retain no unbounded transcript.
        # Preserve stdout/stderr separation, particularly for `run -- --emit nir`.
        def pump(source, destination, label):
            markers = tuple(marker.encode() for marker in SPACE_ERRORS)
            overlap = max(map(len, markers)) - 1
            tail = b''
            failed = False
            # A diagnostic (or a generated artifact) need not contain newlines.
            # read1 bounds memory and forwards currently available pipe bytes.
            while chunk := source.read1(64 * 1024):
                text = tail + chunk.lower()
                if any(marker in text for marker in markers):
                    quota_failure.set()
                tail = text[-overlap:]
                if failed:
                    continue
                try:
                    destination.write(chunk)
                    destination.flush()
                except OSError as error:
                    # Keep draining both pipes: otherwise Cargo can block on a
                    # full pipe while the main thread waits for it to exit.
                    forwarding_errors.append((label, error))
                    failed = True
        readers = [threading.Thread(target=pump, args=(process.stdout, sys.stdout.buffer, 'stdout')),
                   threading.Thread(target=pump, args=(process.stderr, sys.stderr.buffer, 'stderr'))]
        for reader in readers:
            reader.start()
        status = process.wait()
        for reader in readers:
            reader.join()
        process.stdout.close()
        process.stderr.close()
        if forwarding_errors:
            label, error = next(
                (item for item in forwarding_errors if item[1].errno in SPACE_ERRNOS),
                forwarding_errors[0],
            )
            raise OSError(error.errno, f"cannot forward Cargo {label}: {error}") from error
        return status, quota_failure.is_set()

    def run(self, arguments: list[str], *, on_success=None) -> int:
        command = cargo_arguments(arguments, self.target)
        self.target.parent.mkdir(parents=True, exist_ok=True)
        with (self.target.parent / "cargo-maintenance.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            self.reclaim()
            try:
                status, quota_failure = self.execute(command)
                if status and quota_failure and arguments[0] != "clean":
                    # This process is already inside the sandbox: recovery does
                    # not need another sandbox mount when the quota is full.
                    self.reclaim(force=True)
                    diagnostic("cargo-cache: retrying once after a storage failure")
                    status, quota_failure = self.execute(command)
                if status and quota_failure:
                    self.reclaim(force=True)
                if status == 0 and on_success is not None:
                    # Publish required artifacts while still holding the cache
                    # lock, before either this or another process reclaims them.
                    on_success()
                return status
            except OSError as error:
                if error.errno in SPACE_ERRNOS:
                    # Evidence delivery failure is not permission to repeat a
                    # program/test that may already have completed successfully.
                    self.reclaim(force=True)
                raise
            finally:
                # Leave enough headroom for the next sandbox/tool invocation.
                self.reclaim()


if __name__ == "__main__":
    try:
        raise SystemExit(Runner(ROOT, TARGET).run(sys.argv[1:]))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        diagnostic(f"cargo-cache: {error}")
        raise SystemExit(1)
