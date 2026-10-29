#!/usr/bin/env python3
"""Measure public StructBuilder invocation including source frontend startup."""
import argparse
import json
import hashlib
from pathlib import Path
import statistics
import sys
import tempfile
from process_tools import run

ROOT = Path(__file__).resolve().parent.parent
SOURCE = '''comptime fn run() -> bool {{
    var count: Integer = 0;
    var previous: Type = bool;
    var valid = true;
    while count < {count} {{
        let builder = meta.StructBuilder.new(.name = .Generated);
        builder.add_field(.value, bool);
        builder.add_field(.optional, ?bool);
        let generated = builder.Build();
        let fields = meta.struct_field_types(generated);
        valid = valid and generated != previous and fields.length() == 2
            and fields[0] == bool and fields[1] == ?bool;
        previous = generated;
        count += 1;
    }};
    valid and count == {count}
}}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path,
                        default=Path('/tmp/luna-dev-002/target/release/luna'))
    parser.add_argument('--counts', default='16,256,4096')
    parser.add_argument('--target', nargs=3, default=['x86_64', 'linux', '64'],
                        metavar=('ARCH', 'OS', 'POINTER_BITS'))
    parser.add_argument('--samples', type=int, default=3)
    parser.add_argument('--fuel', type=int, default=100000000)
    args = parser.parse_args()
    try:
        counts = [int(value) for value in args.counts.split(',')]
    except ValueError:
        parser.error('--counts requires comma-separated positive integers')
    if not counts or any(count < 1 for count in counts) or args.samples < 1 or args.fuel < 1:
        parser.error('counts, samples and fuel must be positive')
    if not args.binary.is_file():
        parser.error('build luna first or supply --binary')
    directory = Path(tempfile.mkdtemp(prefix='luna-builder-source-'))
    measurements = []
    with args.binary.open('rb') as executable:
        digest = hashlib.file_digest(executable, 'sha256').hexdigest()
    summary = directory / 'summary.json'
    data = {'scope': 'frontend plus public builder invocation',
            'binary': str(args.binary.resolve()), 'binary_sha256': digest,
            'target': args.target, 'fuel': args.fuel, 'measurements': measurements}
    for count in counts:
        source = directory / f'builder-{count}.fl'
        source.write_text(SOURCE.format(count=count))
        durations = []
        reports = []
        for sample in range(args.samples + 1):
            code, logs = run(
                [str(args.binary.resolve()), str(source), '--sysroot', str(ROOT / 'library'),
                 '--build-target', *args.target,
                 '--invoke-comptime', 'run', '--fuel', str(args.fuel)],
                cwd=ROOT, timeout=120, label=f'builder-source-{count}-{sample}')
            report = logs / 'result.json'
            reports.append(str(report))
            evidence = json.loads(report.read_text())
            output = '' if code else Path(evidence['stdout']['path']).read_text().strip()
            if code or output != '(bool true)':
                data['failure'] = {'count': count, 'sample': sample, 'report': str(report)}
                summary.write_text(json.dumps(data, indent=2) + '\n')
                print(f'failed builder workload: {report}; summary: {summary}', file=sys.stderr)
                return code or 1
            if sample:
                durations.append(evidence['elapsed_seconds'])
        measurements.append({'count': count, 'samples_seconds': durations,
                             'median_seconds': statistics.median(durations),
                             'source': str(source), 'reports_including_warmup': reports})
        summary.write_text(json.dumps(data, indent=2) + '\n')
    print(json.dumps({'summary': str(summary), 'measurements': measurements}))
    return 0


if __name__ == '__main__':
    sys.exit(main())
