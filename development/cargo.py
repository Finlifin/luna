#!/usr/bin/env python3
"""Bounded local Cargo cache; run from any directory, without shell expansion.

Usage: python3 development/cargo.py test -p luna --test execution
       python3 development/cargo.py run -p luna -- input.fl --emit nir
       python3 development/cargo.py clean

Only /tmp/luna-dev-002/target is cleaned. Source files, Cargo.lock, and the
registry/download cache are never cleanup targets. Builds and cleanup share
one lock; use this entrypoint instead of concurrent direct Cargo commands.
"""
from __future__ import annotations

import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path("/tmp/luna-dev-002/target")
MAX_CACHE = 2 * 1024**3
MIN_FREE = 2 * 1024**3
REBUILDABLE_PACKAGES = ("elaboration", "luna")
SPACE_ERRORS = ("quota exceeded", "no space left on device", "os error 122", "os error 28")


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

    def cache_size(self) -> int:
        if not self.target.exists():
            return 0
        result = subprocess.run(["du", "-sk", str(self.target)], check=True, text=True, capture_output=True)
        return int(result.stdout.split()[0]) * 1024

    def pressure(self) -> bool:
        # df does not expose all container/user quotas. A cache cap is needed too.
        return self.cache_size() >= MAX_CACHE or shutil.disk_usage(self.target.parent).free < MIN_FREE

    def clean(self, *, full: bool = False, packages: tuple[str, ...] = REBUILDABLE_PACKAGES) -> None:
        command = ["cargo", "clean", "--target-dir", str(self.target)]
        if not full:
            for package in packages:
                command += ["-p", package]
        print("cargo-cache: cleaning " + ("the target directory" if full else "/".join(packages) + " outputs"), file=sys.stderr, flush=True)
        subprocess.run(command, cwd=self.root, env=self.env, check=True)

    def reclaim(self, *, force: bool = False) -> None:
        if force:
            # A reported quota failure is stronger evidence than disk_usage:
            # user/container quotas may be exhausted even below our cache cap.
            self.clean()
            self.clean(full=True)
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
        # Stream all evidence to the caller's log; retain no unbounded transcript.
        # Preserve stdout/stderr separation, particularly for `run -- --emit nir`.
        def pump(source, destination):
            for line in source:
                destination.write(line)
                destination.flush()
                text = line.decode(errors="replace").lower()
                if any(marker in text for marker in SPACE_ERRORS):
                    quota_failure.set()
        readers = [threading.Thread(target=pump, args=(process.stdout, sys.stdout.buffer)),
                   threading.Thread(target=pump, args=(process.stderr, sys.stderr.buffer))]
        for reader in readers:
            reader.start()
        status = process.wait()
        for reader in readers:
            reader.join()
        process.stdout.close()
        process.stderr.close()
        return status, quota_failure.is_set()

    def run(self, arguments: list[str]) -> int:
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
                    print("cargo-cache: retrying once after a storage failure", file=sys.stderr, flush=True)
                    status, quota_failure = self.execute(command)
                if status and quota_failure:
                    self.reclaim(force=True)
                return status
            finally:
                # Leave enough headroom for the next sandbox/tool invocation.
                self.reclaim()


if __name__ == "__main__":
    try:
        raise SystemExit(Runner(ROOT, TARGET).run(sys.argv[1:]))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"cargo-cache: {error}", file=sys.stderr)
        raise SystemExit(1)
