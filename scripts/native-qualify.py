#!/usr/bin/env python3
"""Native Wayland readiness observations and restart checks; private evidence stays in OUTPUT."""
import argparse
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import time


def stop(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def sample(root_pid):
    # Follow the app's actual descendants, excluding private D-Bus/portal helpers.
    stats = {}
    for item in Path('/proc').iterdir():
        if not item.name.isdigit():
            continue
        try:
            fields = (item / 'stat').read_text().rsplit(')', 1)[1].split()
            stats[int(item.name)] = (int(fields[1]), int(fields[11]) + int(fields[12]))
        except (OSError, ValueError):
            pass
    descendants = {root_pid}
    while True:
        children = {pid for pid, (parent, _) in stats.items() if parent in descendants}
        if children <= descendants:
            break
        descendants |= children
    values = {}
    for pid in descendants:
        try:
            pss = next(float(line.split()[1]) / 1024 for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if line.startswith('Pss:'))
            values[str(pid)] = {'pss_mib': pss, 'ticks': stats[pid][1]}
        except (OSError, KeyError, StopIteration):
            pass
    return values


def run(binary, source, output, state, mode):
    output.mkdir(parents=True)
    env = dict(os.environ, GDK_BACKEND='wayland', READERO_DATA_DIR=str(state),
               READERO_SMOKE_DIR=str(output), READERO_SMOKE_FILE=str(source),
               READERO_PROBE=mode, READERO_PROBE_STARTED_NS=str(time.time_ns()))
    readings = []
    baseline = []
    with (output / 'native.log').open('w') as log:
        process = subprocess.Popen(['dbus-run-session', '--', str(binary)], env=env,
                                   stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        start = time.monotonic()
        try:
            while process.poll() is None:
                if time.monotonic() - start > 210:
                    raise RuntimeError('Native qualification timed out')
                if mode == 'crash' and (output / 'report.json').exists():
                    stop(process)
                    break
                if mode == 'idle' and (output / 'baseline-ready.json').exists() and not (output / 'activity-started.json').exists():
                    baseline.append(sum(p['pss_mib'] for p in sample(json.loads((output / 'baseline-ready.json').read_text())['pid']).values()))
                if mode == 'idle' and (output / 'idle-ready.json').exists():
                    readings.append({'seconds': time.monotonic(), 'processes': sample(json.loads((output / 'baseline-ready.json').read_text())['pid'])})
                time.sleep(0.5 if mode == 'idle' else 0.05)
            if mode != 'crash' and process.returncode != 0:
                raise RuntimeError('Native qualification exited unsuccessfully')
        finally:
            if process.poll() is None:
                stop(process)
    report = json.loads((output / 'report.json').read_text())
    if '-CRITICAL **' in (output / 'native.log').read_text():
        raise RuntimeError('Native critical in qualification log')
    if readings:
        (output / 'samples.json').write_text(json.dumps(readings))
        (output / 'baseline.json').write_text(json.dumps(baseline))
        # Use the longest contiguous segment with an unchanged live app tree.
        segments = []
        for reading in readings:
            if not segments or reading['processes'].keys() != segments[-1][-1]['processes'].keys():
                segments.append([])
            segments[-1].append(reading)
        same = max(segments, key=lambda segment: segment[-1]['seconds'] - segment[0]['seconds'])
        initial = same[0]
        if not initial['processes'] or same[-1]['seconds'] - initial['seconds'] < 60:
            raise RuntimeError('Insufficient stable live-process idle samples')
        final = same[-1]
        seconds = final['seconds'] - initial['seconds']
        ticks = sum(p['ticks'] - initial['processes'][pid]['ticks'] for pid, p in final['processes'].items())
        memory = [sum(p['pss_mib'] for p in r['processes'].values()) for r in same]
        report['idle'] = {'seconds': seconds, 'one_cpu_percent': ticks / os.sysconf('SC_CLK_TCK') / seconds * 100,
                          'start_pss_mib': memory[0], 'end_pss_mib': memory[-1], 'peak_pss_mib': max(memory),
                          'processes': len(initial['processes']), 'gpu_memory': 'not measured',
                          'activity_before_idle': '40 turns and 20 mode changes',
                          'baseline_pss_mib': baseline[-1] if baseline else None,
                          'growth_within_budget': bool(baseline) and memory[-1] <= baseline[-1] + max(50, baseline[-1] * .15)}
    return report


def equivalent(a, b):
    return all(a['record'][key] == b['record'][key] for key in ['id', 'locator', 'settings']) and a['bookmark_count'] == b['bookmark_count']


def distribution(values):
    import math
    values = sorted(values)
    return {'n': len(values), 'median_ms': values[len(values)//2], 'p95_ms': values[math.ceil(len(values)*0.95)-1], 'max_ms': max(values)}


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--file', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--mode', choices=['restart', 'timing', 'idle'], default='restart')
args = parser.parse_args()
binary, source, output = args.binary.resolve(), args.file.resolve(), args.output.resolve()
if output.exists():
    parser.error('Use a fresh output directory')
output.mkdir(parents=True)
state = output / 'state'
summary = {'kind': args.mode, 'surface': 'native Wayland', 'instrumentation': 'optimized native app with opt-in readiness probe; not frame-time or input-latency qualification'}
if args.mode == 'restart':
    seeded = run(binary, source, output / 'killed', state, 'crash')
    resumed = run(binary, source, output / 'resumed', state, 'resume')
    summary['sigkill_keeps_committed_position_settings_bookmarks'] = equivalent(seeded, resumed)
    # Kill a writer after forcing dirty-page spill, leaving a hot rollback journal.
    writer_code = """import sqlite3,sys,time
from pathlib import Path
connection=sqlite3.connect(sys.argv[1])
connection.execute('PRAGMA cache_size=1')
connection.execute('PRAGMA synchronous=FULL')
connection.execute('BEGIN IMMEDIATE')
connection.execute('UPDATE documents SET payload=?', ('x'*131072,))
Path(sys.argv[2]).write_text('ready')
time.sleep(120)
"""
    ready = output / 'writer-ready'
    writer = subprocess.Popen([sys.executable, '-c', writer_code, str(state / 'reading.sqlite3'), str(ready)], start_new_session=True)
    try:
        for _ in range(200):
            if ready.exists():
                break
            time.sleep(0.01)
        if not ready.exists():
            raise RuntimeError('Writer did not reach its uncommitted state')
        journal = state / 'reading.sqlite3-journal'
        summary['interrupted_writer_had_rollback_journal'] = journal.exists() and journal.stat().st_size > 512
    finally:
        stop(writer)
    recovered = run(binary, source, output / 'recovered', state, 'resume')
    summary['killed_writer_recovers_previous_commit'] = equivalent(resumed, recovered)
    with sqlite3.connect(state / 'reading.sqlite3') as connection:
        summary['database_integrity_ok'] = connection.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    closing = run(binary, source, output / 'close-layout', state, 'close-layout')
    reopened = run(binary, source, output / 'after-close', state, 'resume')
    summary['close_during_layout_keeps_position_settings_bookmarks'] = equivalent(closing, reopened)
elif args.mode == 'timing':
    launches = []
    for index in range(10):
        report = run(binary, source, output / f'launch-{index}', state, 'launch')
        launches.append(report['launch_ready_ms'])
    warm = run(binary, source, output / 'warm', state, 'warm')['warm_ready_ms']
    summary.update(launch_readiness=distribution(launches), warm_readiness=distribution(warm),
                   launch_ready_ms=launches, warm_ready_ms=warm,
                   cache_condition='OS cache not cleared; each process launch includes a private D-Bus session',
                   pdf_limitation='PDF observes native ready plus 32 ms; pixel-readiness is not asserted')
else:
    summary['idle'] = run(binary, source, output / 'idle', state, 'idle')['idle']
(output / 'checks.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
sys.exit(1 if any(value is False for value in summary.values()) else 0)
