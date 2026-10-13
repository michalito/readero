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
modes = parser.add_mutually_exclusive_group()
modes.add_argument('--passages-only', action='store_true', help='Run source selection and notebook connection integration')
modes.add_argument('--notebook-only', action='store_true', help='Run the focused notebook integration flow')
modes.add_argument('--startup-only', action='store_true', help='Open, snapshot, and close for repeated startup qualification')
modes.add_argument('--startup-recovery', action='store_true', help='Exercise silent renderer timeout, retry, and cancellation')
parser.add_argument('--edit', action='store_true', help='Edit an isolated copy of the Markdown fixture')
parser.add_argument('--protected-fixture', action='store_true')
parser.add_argument('--rapid', type=Path)
parser.add_argument('--storage-failure', action='store_true')
parser.add_argument('--wayland', action='store_true')
args = parser.parse_args()
focused = args.notebook_only or args.passages_only or args.startup_only or args.startup_recovery
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
if args.passages_only:
    env['READERO_SMOKE_PASSAGES'] = '1'
if args.notebook_only:
    env['READERO_SMOKE_NOTEBOOK'] = '1'
if args.startup_only:
    env['READERO_SMOKE_STARTUP'] = '1'
if args.startup_recovery:
    env['READERO_SMOKE_STARTUP_RECOVERY'] = '1'
if args.rapid:
    env['READERO_SMOKE_RAPID_FILE'] = str(args.rapid.resolve())
if args.protected_fixture:
    env['READERO_SMOKE_PASSWORD'] = 'readero-fixture-password'
if args.storage_failure:
    env['READERO_SMOKE_UNSAVED'] = '1'
command = ['dbus-run-session', '--', str(args.binary.resolve())]
if args.wayland:
    env['GDK_BACKEND'] = 'wayland'
    env.pop('READERO_SMOKE_PRIVATE_X11', None)
else:
    env.update(GDK_BACKEND='x11', GSK_RENDERER='cairo', READERO_SMOKE_PRIVATE_X11='1')
    if args.passages_only and args.file.suffix.lower() != '.pdf':
        try:
            import Xlib  # Trusted publication-link key input on private Xvfb.
        except ImportError:
            parser.error('The Xvfb passage flow requires python3-xlib.')
    command = ['xvfb-run', '-a', *command]
def run_application(environment, directory):
    with (directory / 'native.log').open('w') as log:
        process = subprocess.Popen(command, env=environment, stdout=log,
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
    report_path = directory / 'report.json'
    if returncode or not report_path.exists():
        sys.exit('FAIL: native app did not produce a complete report.')
    return json.loads(report_path.read_text())


report = run_application(env, args.output)
required = ['ready', 'requested_document_won']
if args.storage_failure:
    required += ['storage_error_visible']
elif not focused:
    required += ['mode_kept_locator', 'resume_kept_locator', 'resume_kept_settings']
    required += [key for key in ['jump_changed_locator', 'back_kept_locator'] if key in report]
    if args.protected_fixture:
        required += ['password_dialog', 'incorrect_password_feedback']
checks = {key: report.get(key) is True for key in required}
if not args.storage_failure and not focused:
    checks['regression_checks_completed'] = bool(report.get('regressions'))
    for key, value in report.get('regressions', {}).items():
        checks[key] = value is True
if not args.storage_failure and (not focused or args.notebook_only):
    checks['notebook_checks_completed'] = bool(report.get('notebook'))
    for key, value in report.get('notebook', {}).items():
        checks[key] = value is True
if args.edit and not focused:
    checks['editing_checks_completed'] = bool(report.get('editing'))
    for key, value in report.get('editing', {}).items():
        checks[key] = value is True
if not args.storage_failure and not focused:
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
if args.passages_only:
    checks['passage_checks_completed'] = bool(report.get('passages'))
    checks.update({key: value is True for key, value in report.get('passages', {}).items()})
if args.startup_recovery:
    checks['startup_recovery_completed'] = bool(report.get('startup_recovery'))
    checks.update({key: value is True for key, value in report.get('startup_recovery', {}).items()})
if (args.notebook_only or args.passages_only) and not args.storage_failure:
    restart_dir = args.output / 'restart'
    restart_dir.mkdir()
    restart_env = dict(env, READERO_SMOKE_DIR=str(restart_dir.resolve()),
                       READERO_SMOKE_NOTEBOOK_RESTART='1')
    if args.passages_only:
        restart_env['READERO_SMOKE_PASSAGES_RESTART'] = '1'
    restarted = run_application(restart_env, restart_dir)
    restart_key = 'passages' if args.passages_only else 'notebook_restart'
    checks['restart_checks_completed'] = bool(restarted.get(restart_key))
    for key, value in restarted.get(restart_key, {}).items():
        checks[key] = value is True
    restart_log = (restart_dir / 'native.log').read_text()
    checks['restart_no_native_criticals'] = '-CRITICAL **' not in restart_log and 'panicked at' not in restart_log
# Safe to retain/share this summary: no filenames, passages, or locators.
(args.output / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
print(json.dumps(checks, indent=2))
sys.exit(0 if all(checks.values()) else 1)
