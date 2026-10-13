#!/usr/bin/env python3
"""Run native notebook checks at real Wayland scales in a private GNOME compositor."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
from gi.repository import Gio, GLib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--file', type=Path, required=True, action='append')
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--passages-only', action='store_true')
parser.add_argument('--scales', type=float, nargs='+', default=[1, 1.25, 1.5, 2])
args = parser.parse_args()
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
env = dict(os.environ, GSETTINGS_BACKEND='memory', XDG_CURRENT_DESKTOP='GNOME',
           WAYLAND_DISPLAY='readero-qa', GIO_USE_VFS='local', GTK_IM_MODULE='simple')
for key in ('DISPLAY', 'XDG_SESSION_ID', 'SESSION_MANAGER', 'G_DEBUG', 'GSK_RENDERER'):
    env.pop(key, None)
for key, name in [('XDG_RUNTIME_DIR', 'runtime'), ('XDG_CONFIG_HOME', 'config'),
                  ('XDG_DATA_HOME', 'data'), ('XDG_CACHE_HOME', 'cache')]:
    folder = root / name
    folder.mkdir(mode=0o700, exist_ok=True)
    env[key] = str(folder)
bus = subprocess.Popen(['dbus-daemon', '--session', '--nofork', '--print-address=1'],
                       env=env, stdout=subprocess.PIPE, stderr=(root/'bus.log').open('w'),
                       text=True, start_new_session=True)
env['DBUS_SESSION_BUS_ADDRESS'] = bus.stdout.readline().strip()
compositor = None
try:
    compositor = subprocess.Popen(['gnome-shell', '--headless', '--wayland', '--no-x11',
                                  '--virtual-monitor', '2880x2160@60',
                                  '--wayland-display', 'readero-qa'], env=env,
                                 stdout=(root/'compositor.log').open('w'), stderr=subprocess.STDOUT,
                                 start_new_session=True)
    connection = Gio.DBusConnection.new_for_address_sync(env['DBUS_SESSION_BUS_ADDRESS'],
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
        None, None)
    def call(method, parameters=None):
        return connection.call_sync('org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
            'org.gnome.Mutter.DisplayConfig', method, parameters, None,
            Gio.DBusCallFlags.NO_AUTO_START, 5000, None)
    for _ in range(200):
        try:
            state = call('GetCurrentState').unpack()
            break
        except GLib.Error:
            if compositor.poll() is not None:
                raise RuntimeError('Compositor failed')
            time.sleep(.1)
    else:
        raise RuntimeError('Compositor did not become ready')
    owner = connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
        'org.freedesktop.DBus', 'GetConnectionUnixProcessID',
        GLib.Variant('(s)', ('org.gnome.Mutter.DisplayConfig',)), None,
        Gio.DBusCallFlags.NONE, 1000, None).unpack()[0]
    if owner != compositor.pid:
        raise RuntimeError('Refusing to change a display not owned by this harness')
    (root/'initial-display.json').write_text(json.dumps(state, indent=2))
    # Wayland selection ownership requires real input serials. Without a
    # virtual keyboard a headless compositor rejects scripted GTK selections.
    session = connection.call_sync('org.gnome.Mutter.RemoteDesktop', '/org/gnome/Mutter/RemoteDesktop',
        'org.gnome.Mutter.RemoteDesktop', 'CreateSession', None, None,
        Gio.DBusCallFlags.NONE, 1000, None).unpack()[0]
    def input_call(method, parameters=None):
        return connection.call_sync('org.gnome.Mutter.RemoteDesktop', session,
            'org.gnome.Mutter.RemoteDesktop.Session', method, parameters, None,
            Gio.DBusCallFlags.NONE, 1000, None)
    input_call('Start')
    for pressed in (True, False):
        input_call('NotifyKeyboardKeysym', GLib.Variant('(ub)', (0xff1b, pressed)))  # Escape overview
    runs = []
    for scale in args.scales:
        state = call('GetCurrentState').unpack()
        connector = state[1][0][0][0]
        mode = state[1][0][1][0][0]
        call('ApplyMonitorsConfig', GLib.Variant('(uua(iiduba(ssa{sv}))a{sv})',
            (state[0], 1, [(0, 0, scale, 0, True, [(connector, mode, {})])], {})))
        time.sleep(1)
        current = call('GetCurrentState').unpack()
        if current[2][0][2] != scale:
            raise RuntimeError('Compositor did not apply the requested scale')
        (root / f'display-{scale:g}.json').write_text(json.dumps(current, indent=2))
        for index, source in enumerate(args.file):
            target = root / f'scale-{scale:g}-file-{index}'
            process = subprocess.Popen([sys.executable, str(Path(__file__).with_name('native-smoke.py')),
                '--binary', str(args.binary.resolve()), '--file', str(source.resolve()),
                '--output', str(target), '--passages-only' if args.passages_only else '--notebook-only', '--wayland'],
                env=dict(env, G_DEBUG='fatal-criticals'),
                stdout=(root / f'run-{scale:g}-{index}.log').open('w'))
            try:
                while process.poll() is None:
                    time.sleep(1)
                    for pressed in (True, False):
                        input_call('NotifyKeyboardKeysym', GLib.Variant('(ub)', (0xffe1, pressed)))  # Shift
                if process.returncode:
                    raise RuntimeError(f'Native flow failed; inspect {target}')
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)
            report = json.loads((target / 'report.json').read_text())
            checks = json.loads((target / 'checks.json').read_text())
            display = report['display']
            if display['backend'] != 'GdkWaylandDisplay' or display['scale'] != scale:
                raise RuntimeError(f'Application did not observe the expected scale: {display}')
            runs.append({'scale': scale, 'format': source.suffix, 'display': display,
                         'assertions': len(checks), 'passed': all(checks.values())})
            (root / 'checks.json').write_text(json.dumps(runs, indent=2) + '\n')
            print(json.dumps(runs[-1]), flush=True)
finally:
    for process in (compositor, bus):
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
