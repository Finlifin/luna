#!/usr/bin/env python3
"""Bounded local Cargo cache; run from any directory, without shell expansion.

Usage: python3 development/cargo.py [--no-fmt] test -p luna --test execution
       python3 development/cargo.py run -p luna -- input.fl --emit nir
       python3 development/cargo.py fmt [--check] [--all | paths...]
       python3 development/cargo.py clean

Before `build`, `check`, `test`, or `run`, modified `.rs` files in git status
are automatically formatted via `development/rust_fmt.py` (`rustfmt.toml` plus
item-level blank-line separation, ignoring configured fragment files). Pass
`--no-fmt` or set `LUNA_SKIP_AUTO_FMT=1` to bypass automatic formatting.

Only /tmp/luna-dev-002/target is cleaned. Source files, Cargo.lock, and the
registry/download cache are never cleanup targets. Builds and cleanup share
one lock; use this entrypoint instead of concurrent direct Cargo commands.

After an observed storage failure, retain a soft cache cap at 75% of the
pre-cleanup target footprint (64 MiB minimum, never above the current cap).
The default is 4 GiB. The cap lives in /tmp/luna-dev-002/target-cache-limit,
outside Cargo's cleanup scope. Remove that local file after increasing the
environment's quota to return to the default. It is a headroom heuristic, not
a measurement or guarantee of available quota; unrelated disk usage can still
cause failures. Evidence forwarding errors still never replay completed work.
"""
from __future__ import annotations

import errno
import fcntl
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path("/tmp/luna-dev-002/target")
MAX_CACHE = 4 * 1024**3
MIN_CACHE = 64 * 1024**2
MIN_FREE = 2 * 1024**3
REBUILDABLE_PACKAGES = ("elaboration", "luna")
SPACE_ERRORS = ("quota exceeded", "no space left on device", "os error 122", "os error 28")
SPACE_ERRNOS = (errno.EDQUOT, errno.ENOSPC)
TRUTHY_FLAGS = frozenset({"1", "true", "yes", "on"})
FALSY_FLAGS = frozenset({"0", "false", "no", "off"})


def diagnostic(message: str) -> None:
    # Cache recovery must still run when the evidence destination is full.
    try:
        print(message, file=sys.stderr, flush=True)
    except OSError:
        pass


def load_rust_fmt():
    module_name = "luna_rust_fmt"
    existing = sys.modules.get(module_name)
    if existing is not None:
        return existing
    rust_fmt_path = Path(__file__).resolve().with_name("rust_fmt.py")
    spec = importlib.util.spec_from_file_location(module_name, rust_fmt_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {rust_fmt_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    except Exception:
        sys.modules.pop(module_name, None)
        raise
    return module


def split_runner_flags(arguments: list[str]) -> tuple[bool, list[str]]:
    skip_fmt = False
    index = 0
    while index < len(arguments) and arguments[index] == "--no-fmt":
        skip_fmt = True
        index += 1
    return skip_fmt, arguments[index:]


def should_skip_auto_fmt(skip_flag: bool, env: dict[str, str]) -> bool:
    if skip_flag:
        return True
    skip_env = env.get("LUNA_SKIP_AUTO_FMT", "").strip().lower()
    if skip_env in TRUTHY_FLAGS:
        return True
    auto_env = env.get("LUNA_AUTO_FMT", "").strip().lower()
    if auto_env in FALSY_FLAGS:
        return True
    return False


def cargo_arguments(arguments: list[str], target: Path) -> list[str]:
    _, remaining = split_runner_flags(arguments)
    if not remaining or remaining[0] not in {"build", "check", "test", "run", "clean"}:
        raise ValueError("expected build, check, test, run, clean, or fmt")
    split = remaining.index("--") if "--" in remaining else len(remaining)
    options, tail = remaining[:split], remaining[split:]
    if any(arg == "--target-dir" or arg.startswith("--target-dir=") for arg in options):
        raise ValueError("target-dir is managed by this entrypoint")
    # Keep caller-selected tests/packages/features; always use the shared cache.
    common = ["--target-dir", str(target)]
    if options[0] != "clean":
        common += ["--locked", "--offline"]
    return ["cargo", *options, *common, *tail]


def default_build_jobs() -> int:
    return max(1, min(os.cpu_count() or 1, 12))


class Runner:
    def __init__(self, root: Path, target: Path):
        if target.resolve() != target.absolute():
            raise ValueError("managed target must not redirect through a symlink")
        self.root, self.target = root, target
        self.env = dict(os.environ)
        self.env.setdefault("CARGO_BUILD_JOBS", str(default_build_jobs()))
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

    def modified_rust_files(self) -> list[Path]:
        if not any((p / ".git").exists() for p in (self.root, *self.root.parents)):
            return []
        top_proc = subprocess.run(
            ["git", "-C", str(self.root), "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
        )
        repo_root = (
            Path(top_proc.stdout.strip()).resolve()
            if top_proc.returncode == 0 and top_proc.stdout.strip()
            else self.root.resolve()
        )
        result = subprocess.run(
            [
                "git",
                "-C",
                str(self.root),
                "status",
                "--porcelain",
                "-z",
                "--untracked-files=all",
                "--",
                "*.rs",
            ],
            capture_output=True,
        )
        if result.returncode != 0:
            return []
        rust_fmt = load_rust_fmt()
        ignored = rust_fmt.load_ignored_paths(self.root)
        if repo_root != self.root.resolve():
            ignored |= rust_fmt.load_ignored_paths(repo_root)
        entries = result.stdout.split(b"\0")
        paths: list[Path] = []
        seen: set[Path] = set()
        index = 0
        while index < len(entries):
            raw = entries[index]
            index += 1
            if not raw:
                continue
            text = raw.decode("utf-8", errors="surrogateescape")
            if len(text) < 4:
                continue
            status_xy = text[:2]
            rel_path = text[3:]
            if "R" in status_xy or "C" in status_xy:
                # `-z` puts the original source path as the next NUL record.
                index += 1
            if "D" in status_xy:
                continue
            candidate = (repo_root / rel_path).resolve()
            if candidate in ignored or candidate in seen or not candidate.is_file():
                continue
            seen.add(candidate)
            paths.append(candidate)
        return paths

    def tracked_rust_files(self) -> list[Path]:
        if not any((p / ".git").exists() for p in (self.root, *self.root.parents)):
            return sorted(self.root.rglob("*.rs"))
        top_proc = subprocess.run(
            ["git", "-C", str(self.root), "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
        )
        repo_root = (
            Path(top_proc.stdout.strip()).resolve()
            if top_proc.returncode == 0 and top_proc.stdout.strip()
            else self.root.resolve()
        )
        result = subprocess.run(
            ["git", "-C", str(self.root), "ls-files", "-z", "--", "*.rs"],
            capture_output=True,
        )
        if result.returncode != 0:
            return []
        rust_fmt = load_rust_fmt()
        ignored = rust_fmt.load_ignored_paths(self.root) | rust_fmt.load_ignored_paths(repo_root)
        paths: list[Path] = []
        for raw in result.stdout.split(b"\0"):
            if not raw:
                continue
            rel = raw.decode("utf-8", errors="surrogateescape")
            candidate = (repo_root / rel).resolve()
            if candidate not in ignored and candidate.is_file():
                paths.append(candidate)
        return paths

    def format_modified_rust(self, *, skip_flag: bool = False) -> None:
        if should_skip_auto_fmt(skip_flag, self.env):
            return
        files = self.modified_rust_files()
        if not files:
            return
        rust_fmt = load_rust_fmt()
        try:
            status, detail = rust_fmt.format_files(self.root, files, check=False)
        except OSError as error:
            diagnostic(f"rustfmt-auto: warning: cannot run formatter: {error}")
            return
        if status != 0:
            summary = detail.splitlines()[0] if detail else f"exit status {status}"
            diagnostic(f"rustfmt-auto: warning: skipped invalid/unformattable changes ({summary})")

    def run_fmt(self, args: list[str]) -> int:
        check = False
        format_all = False
        raw_paths: list[str] = []
        for arg in args:
            if arg == "--check":
                check = True
            elif arg == "--all":
                format_all = True
            elif arg.startswith("-"):
                raise ValueError(f"unsupported fmt option: {arg}")
            else:
                raw_paths.append(arg)
        if format_all and raw_paths:
            raise ValueError("fmt --all does not take explicit file paths")
        rust_fmt = load_rust_fmt()
        if format_all:
            cargo_cmd = ["cargo", "fmt", "--all"]
            if check:
                cargo_cmd += ["--", "--check"]
            proc = subprocess.run(cargo_cmd, cwd=self.root, env=self.env)
            targets = self.tracked_rust_files()
            if not check:
                changed: list[Path] = []
                for path in targets:
                    original = path.read_text(encoding="utf-8")
                    updated = rust_fmt.ensure_blank_lines_between_items(original)
                    if updated != original:
                        path.write_text(updated, encoding="utf-8")
                        changed.append(path)
                if changed:
                    subprocess.run(["cargo", "fmt", "--all"], cwd=self.root, env=self.env)
                return proc.returncode
            unspaced = [
                str(p)
                for p in targets
                if rust_fmt.ensure_blank_lines_between_items(p.read_text(encoding="utf-8"))
                != p.read_text(encoding="utf-8")
            ]
            if unspaced:
                diagnostic("missing blank line between Rust items in: " + ", ".join(unspaced))
                return 1
            return proc.returncode
        if raw_paths:
            ignored = rust_fmt.load_ignored_paths(self.root)
            files = rust_fmt._expand_paths(self.root, raw_paths, ignored)
        else:
            files = self.modified_rust_files()
        status, message = rust_fmt.format_files(self.root, files, check=check)
        if message:
            diagnostic(message)
        return status

    def run(self, arguments: list[str], *, on_success=None) -> int:
        skip_fmt, remaining = split_runner_flags(arguments)
        if remaining and remaining[0] == "fmt":
            self.target.parent.mkdir(parents=True, exist_ok=True)
            with (self.target.parent / "cargo-maintenance.lock").open("a") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                return self.run_fmt(remaining[1:])
        command = cargo_arguments(remaining, self.target)
        self.target.parent.mkdir(parents=True, exist_ok=True)
        with (self.target.parent / "cargo-maintenance.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            if remaining[0] != "clean":
                self.format_modified_rust(skip_flag=skip_fmt)
            self.reclaim()
            try:
                status, quota_failure = self.execute(command)
                if status and quota_failure and remaining[0] != "clean":
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
