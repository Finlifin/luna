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


def save_summary(path, evidence, *, stream=None):
    """Atomically publish aggregate evidence; preserve its last snapshot on error.

    False means orchestration must stop with 125 before starting further work.
    The terminal payload is authoritative when storage retains an older snapshot.
    """
    path = Path(path)
    temporary = path.with_name(path.name + '.tmp')
    try:
        temporary.write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + '\n',
                             encoding='utf-8')
        temporary.replace(path)
        return True
    except OSError as error:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        print(json.dumps({'status': 'evidence-error', 'wrapper_exit_code': 125,
                          'summary': str(path), 'summary_saved': False,
                          'error': str(error), 'evidence': evidence}, ensure_ascii=False),
              file=stream, flush=True)
        return False


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
        try:
            temporary.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
            temporary.replace(report_path)
            return True
        except OSError as error:
            report.setdefault('report_errors', []).append(str(error))
            return False

    def publish(saved):
        terminal = {'status': report['status'], 'exit_code': report['exit_code'],
                    'wrapper_exit_code': report['wrapper_exit_code'],
                    'report': str(report_path), 'report_saved': saved}
        if not saved:
            # The file may still contain an earlier running state. Preserve the
            # terminal evidence on stdout when storage cannot accept an update.
            terminal['evidence'] = report
        print(json.dumps(terminal, ensure_ascii=False), flush=True)

    if not save():
        report.update(status='evidence-error', wrapper_exit_code=125)
        publish(False)
        return 125, directory
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
            errors.append(f'{channel}: {error}')
            # Keep draining even if evidence storage fails, avoiding pipe deadlock.
            try:
                while chunk := stream.read(65536):
                    count += len(chunk)
            except OSError as drain_error:
                errors.append(f'{channel} drain: {drain_error}')
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
        if not save():
            raise OSError('could not publish subprocess identity')
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
        report.update(status='unavailable', error=str(error),
                      exit_code=process.returncode if process else None)
        code = 125
    finally:
        if previous_term is not None:
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        for reader in readers:
            reader.join(timeout=1)
        if process is not None and not readers:
            # Startup can fail after Popen but before the pump threads own the
            # pipes (for example when publishing its PID exhausts the quota).
            for stream in (process.stdout, process.stderr):
                if stream is not None:
                    stream.close()
        report['elapsed_seconds'] = round(time.monotonic() - started, 3)
        if errors:
            report['log_errors'] = errors
        if errors or report.get('report_errors'):
            report['status'] = 'evidence-error'
            code = 125
        report['wrapper_exit_code'] = code
        try:
            saved = save()
            if not saved:
                code = 125
                report.update(status='evidence-error', wrapper_exit_code=code)
        finally:
            if previous_term is not None:
                signal.signal(signal.SIGTERM, previous_term)
    publish(saved)
    return code, directory
