"""Noninteractive, bounded subprocesses; no shell and no privilege escalation."""
from __future__ import annotations

import json
import math
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
LUNA = Path('/tmp/luna-dev-002/target/debug/luna')
LOG_LIMIT = 8 * 1024 * 1024


class Terminated(Exception):
    pass


def run(command, *, cwd=ROOT, timeout=600, label='command', directory=None, env=None,
        log_limit=LOG_LIMIT):
    if not math.isfinite(timeout) or timeout <= 0 or log_limit < 0:
        raise ValueError('timeout must be positive; log limit must be nonnegative')
    directory = Path(directory) if directory else Path(tempfile.mkdtemp(prefix='luna-tools-'))
    directory.mkdir(parents=True, exist_ok=True)
    report = {'label': label, 'command': list(map(str, command)), 'cwd': str(cwd),
              'timeout_seconds': timeout, 'status': 'running', 'exit_code': None}
    report_path = directory / 'result.json'

    def save():
        temporary = directory / 'result.json.tmp'
        temporary.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        temporary.replace(report_path)

    save()
    # Publish the evidence location before waiting, including when the tool call
    # yields while the subprocess is still running.
    print(json.dumps({'status': 'running', 'report': str(report_path)},
                     ensure_ascii=False), flush=True)
    started = time.monotonic()
    process = None
    readers = []
    errors = []
    previous_term = None
    if threading.current_thread() is threading.main_thread():
        def terminate(signum, frame):
            raise Terminated()
        previous_term = signal.signal(signal.SIGTERM, terminate)
    def pump(stream, path, channel):
        count = kept = 0
        try:
            with path.open('wb') as output:
                while chunk := stream.read1(65536):
                    count += len(chunk)
                    part = chunk[:max(0, log_limit - kept)]
                    output.write(part)
                    output.flush()
                    kept += len(part)
        except OSError as error:
            errors.append(str(error))
            # Keep draining even if evidence storage fails, avoiding pipe deadlock.
            while stream.read(65536):
                pass
        finally:
            stream.close()
            report[channel] = {'bytes': count, 'saved_bytes': kept,
                               'truncated': count > kept, 'path': str(path)}

    def stop():
        if process is None:
            return
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        # Kill the entire group even if its leader exits before a descendant.
        time.sleep(0.2)
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()

    try:
        child_env = dict(os.environ)
        child_env.update(NO_COLOR='1', CARGO_TERM_COLOR='never', GIT_TERMINAL_PROMPT='0')
        if env:
            child_env.update(env)
        process = subprocess.Popen(report['command'], cwd=cwd, env=child_env,
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        report['pid'] = process.pid
        save()
        for channel in ['stdout', 'stderr']:
            reader = threading.Thread(target=pump, args=(getattr(process, channel),
                                      directory / (channel + '.log'), channel), daemon=True)
            reader.start()
            readers.append(reader)
        # Include inherited pipes in the deadline: a child cannot keep us waiting
        # forever after its parent exits.
        deadline = started + timeout
        while process.poll() is None or any(reader.is_alive() for reader in readers):
            if time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired(command, timeout)
            time.sleep(0.02)
        report.update(status='completed', exit_code=process.returncode)
        code = process.returncode if process.returncode >= 0 else 128 - process.returncode
    except subprocess.TimeoutExpired:
        stop()
        report.update(status='timeout', exit_code=process.returncode)
        code = 124
    except (KeyboardInterrupt, Terminated) as error:
        # A second termination request must not interrupt descendant cleanup.
        if previous_term is not None:
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        stop()
        report.update(status='interrupted', exit_code=process.returncode if process else None)
        code = 143 if isinstance(error, Terminated) else 130
    except OSError as error:
        stop()
        report.update(status='unavailable', error=str(error))
        code = 125
    finally:
        if previous_term is not None:
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        for reader in readers:
            reader.join(timeout=1)
        report['elapsed_seconds'] = round(time.monotonic() - started, 3)
        if errors:
            report['log_errors'] = errors
            report['status'] = 'evidence-error'
            code = 125
        report['wrapper_exit_code'] = code
        save()
        if previous_term is not None:
            signal.signal(signal.SIGTERM, previous_term)
    print(json.dumps({'status': report['status'], 'exit_code': report['exit_code'],
                      'wrapper_exit_code': code,
                      'report': str(report_path)}, ensure_ascii=False), flush=True)
    return code, directory
