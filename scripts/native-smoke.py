#!/usr/bin/env python3
"""Exercise the real GTK/WebKit app, with private evidence kept outside the repo."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import signal

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, default=Path('target/release/readero'))
parser.add_argument('--file', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--stress', action='store_true')
parser.add_argument('--edit', action='store_true', help='Edit an isolated copy of the Markdown fixture')
parser.add_argument('--protected-fixture', action='store_true')
parser.add_argument('--rapid', type=Path)
parser.add_argument('--storage-failure', action='store_true')
parser.add_argument('--wayland', action='store_true')
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
if (args.output / 'report.json').exists():
    parser.error('Use a fresh output directory so evidence cannot be stale.')
if args.edit:
    import shutil
    copy = args.output / 'editable.md'
    shutil.copyfile(args.file, copy)
    args.file = copy
state = args.output / 'state'
if args.storage_failure:
    state.write_text('A file deliberately blocks creation of the data directory.')
env = dict(os.environ, READERO_DATA_DIR=str(state.resolve()),
           READERO_SMOKE_DIR=str(args.output.resolve()),
           READERO_SMOKE_FILE=str(args.file.resolve()))
if args.edit:
    env['READERO_SMOKE_EDIT'] = '1'
if args.stress:
    env['READERO_SMOKE_STRESS'] = '1'
if args.rapid:
    env['READERO_SMOKE_RAPID_FILE'] = str(args.rapid.resolve())
if args.protected_fixture:
    env['READERO_SMOKE_PASSWORD'] = 'readero-fixture-password'
if args.storage_failure:
    env['READERO_SMOKE_UNSAVED'] = '1'
command = ['dbus-run-session', '--', str(args.binary.resolve())]
if args.wayland:
    env['GDK_BACKEND'] = 'wayland'
else:
    env.update(GDK_BACKEND='x11', GSK_RENDERER='cairo')
    command = ['xvfb-run', '-a', *command]
with (args.output / 'native.log').open('w') as log:
    process = subprocess.Popen(command, env=env, stdout=log,
                               stderr=subprocess.STDOUT, start_new_session=True)
    try:
        returncode = process.wait(timeout=240)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        sys.exit('FAIL: native flow timed out; inspect the local log.')
report_path = args.output / 'report.json'
if returncode or not report_path.exists():
    sys.exit('FAIL: native app did not produce a complete report.')
report = json.loads(report_path.read_text())
required = ['ready', 'requested_document_won']
if args.storage_failure:
    required += ['storage_error_visible']
else:
    required += ['mode_kept_locator', 'resume_kept_locator', 'resume_kept_settings']
    required += [key for key in ['jump_changed_locator', 'back_kept_locator'] if key in report]
    if args.protected_fixture:
        required += ['password_dialog', 'incorrect_password_feedback']
checks = {key: report.get(key) is True for key in required}
if not args.storage_failure:
    checks['regression_checks_completed'] = bool(report.get('regressions'))
    for key, value in report.get('regressions', {}).items():
        checks[key] = value is True
if args.edit:
    checks['editing_checks_completed'] = bool(report.get('editing'))
    for key, value in report.get('editing', {}).items():
        checks[key] = value is True
if not args.storage_failure:
    checks['bookmark_persisted'] = report.get('bookmark_count') == 1
if 'isolation' in report:
    checks['private_bridge'] = report['isolation'].get('bridgeHidden') is True
    checks['no_authored_execution'] = report['isolation'].get('authoredScriptRan') is False
for key in ['continuous', 'paged', 'jump', 'renderer', 'reopened']:
    if isinstance(report.get(key), dict):
        checks[key + '_passage_visible'] = report[key].get('anchorVisible') is True
stress = [item for item in report.get('stress', []) if isinstance(item, dict)]
if stress:
    checks['stress_passages_visible'] = all(item.get('anchorVisible') for item in stress)
    checks['bounded_chapter_frames'] = max(item.get('mounted', 0) for item in stress) <= 5
log = (args.output / 'native.log').read_text()
checks['no_native_criticals'] = '-CRITICAL **' not in log and 'panicked at' not in log
# Safe to retain/share this summary: no filenames, passages, or locators.
(args.output / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
print(json.dumps(checks, indent=2))
sys.exit(0 if all(checks.values()) else 1)
