#!/usr/bin/env python3
"""Inspect Readero's loaded AT-SPI tree. Run inside Xvfb and a private D-Bus session."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import gi

gi.require_version('Atspi', '2.0')
from gi.repository import Atspi, GLib


def walk(node, depth=0):
    # AT-SPI may return a null child while WebKit replaces its loading tree.
    if node is None or depth > 30:
        return []
    rows = []
    try:
        role, name, text = node.get_role_name(), node.get_name(), ''
        if node.is_text():
            interface = node.get_text_iface()
            text = Atspi.Text.get_text(interface, 0, min(200, Atspi.Text.get_character_count(interface)))
        rows.append((role, name, text))
        for index in range(node.get_child_count()):
            rows.extend(walk(node.get_child_at_index(index), depth + 1))
    except GLib.Error:
        pass  # A loading/replaced accessibility object may disappear.
    return rows


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
output = args.output.resolve()
output.mkdir(parents=True, exist_ok=False)
source = Path(__file__).resolve().parent.parent / 'examples/quiet-reading.md'
env = dict(os.environ, READERO_DATA_DIR=str(output / 'state'), GDK_BACKEND='x11', GSK_RENDERER='cairo')
with (output / 'native.log').open('w') as log:
    process = subprocess.Popen([str(args.binary.resolve()), str(source)], env=env, stdout=log,
                               stderr=subprocess.STDOUT, start_new_session=True)
    try:
        rows = []
        for _ in range(200):
            context = GLib.MainContext.default()
            while context.pending():
                context.iteration(False)
            desktop = Atspi.get_desktop(0)
            for index in range(desktop.get_child_count()):
                app = desktop.get_child_at_index(index)
                if app.get_process_id() == process.pid:
                    rows = walk(app)
            if any('Good reading starts' in text for _, _, text in rows) and any(name == 'Pages' for _, name, _ in rows):
                break
            time.sleep(0.1)
        checks = {
            'native_controls_exposed': any(role in ('button', 'push button') and 'Open' in name for role, name, _ in rows),
            'semantic_heading_exposed': any(role == 'heading' for role, _, _ in rows),
            'publication_text_exposed': any('Good reading starts' in text for _, _, text in rows),
            'page_mode_named': any(name == 'Pages' for _, name, _ in rows),
            'appearance_named': any(name == 'Reading appearance' for _, name, _ in rows),
            'scope': 'AT-SPI tree inspection only; not an Orca usability session',
        }
        (output / 'tree.json').write_text(json.dumps(rows, indent=2))
        (output / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
        print(json.dumps(checks, indent=2))
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
        process.wait()
raise SystemExit(1 if any(value is False for value in checks.values()) else 0)
