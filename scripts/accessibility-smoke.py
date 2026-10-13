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


def named(node, label, depth=0, exact=False):
    if node is None or depth > 30:
        return None
    try:
        if (node.get_name() == label) if exact else (label in node.get_name()):
            return node
        for index in range(node.get_child_count()):
            found = named(node.get_child_at_index(index), label, depth + 1, exact)
            if found is not None:
                return found
    except GLib.Error:
        pass
    return None


def text_node(node, phrase, depth=0):
    if node is None or depth > 30:
        return None
    try:
        if node.is_text() and Atspi.Text.get_text(node.get_text_iface(), 0, -1).startswith(phrase):
            return node
        for index in range(node.get_child_count()):
            found = text_node(node.get_child_at_index(index), phrase, depth + 1)
            if found is not None:
                return found
    except GLib.Error:
        pass
    return None


def activate(node):
    if node is None:
        return False
    if node.is_action() and node.get_action_iface().get_n_actions():
        return node.get_action_iface().do_action(0)
    return any(activate(node.get_child_at_index(index)) for index in range(node.get_child_count()))


def tab_to(node, backwards=False):
    for _ in range(35):
        if focused(node):
            return True
        key(*('Shift_L', 'Tab') if backwards else ('Tab',))
    return False


def wait_for(predicate):
    for _ in range(100):
        result = predicate()
        if result:
            return result
        time.sleep(.05)
    return None


def key(*symbols):
    """Send real X11 key events; only used on the private Xvfb display."""
    from Xlib import X, XK, display
    from Xlib.ext import xtest
    connection = display.Display()
    try:
        codes = [connection.keysym_to_keycode(XK.string_to_keysym(symbol)) for symbol in symbols]
        if not all(codes):
            raise RuntimeError(f'Unknown key: {symbols}')
        for code in codes:
            xtest.fake_input(connection, X.KeyPress, code)
        for code in reversed(codes):
            xtest.fake_input(connection, X.KeyRelease, code)
        connection.sync()
    finally:
        connection.close()
    time.sleep(.3)


def focused(node):
    if node is None:
        return False
    return node.get_state_set().contains(Atspi.StateType.FOCUSED) or any(
        focused(node.get_child_at_index(index)) for index in range(node.get_child_count()))


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
    process.wait()


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--orca', action='store_true', help='Check Orca announcements and real keyboard input; requires Orca and Speech Dispatcher')
args = parser.parse_args()
output = args.output.resolve()
output.mkdir(parents=True, exist_ok=False)
source = Path(__file__).resolve().parent.parent / 'examples/quiet-reading.md'
env = dict(os.environ, READERO_DATA_DIR=str(output / 'state'), GDK_BACKEND='x11', GSK_RENDERER='cairo')
helpers = []
try:
    if args.orca:
        # All settings and speech output are private. The dummy synthesizer keeps
        # real Orca/Speech Dispatcher interaction silent on the user's desktop.
        config = output / 'speech-config'
        config.mkdir()
        (config / 'speechd.conf').write_text('AddModule "dummy" "sd_dummy"\nDefaultModule dummy\nLogLevel 3\n')
        socket = output / 'speech.sock'
        env.update(GSETTINGS_BACKEND='memory', XDG_DATA_HOME=str(output / 'data'),
                   XDG_CONFIG_HOME=str(output / 'config'), XDG_CACHE_HOME=str(output / 'cache'),
                   SPEECHD_ADDRESS=f'unix_socket:{socket}')
        speech = subprocess.Popen(['speech-dispatcher', '--run-single', '--config-dir', str(config),
                                   '--socket-path', str(socket), '--pid-file', str(output / 'speech.pid'),
                                   '--log-dir', str(output), '--timeout', '0'], env=env,
                                  stdout=(output / 'speech.log').open('w'), stderr=subprocess.STDOUT,
                                  start_new_session=True)
        helpers.append(speech)
        for _ in range(100):
            if socket.exists() or speech.poll() is not None:
                break
            time.sleep(.05)
        orca_log = output / 'orca-debug.log'
        orca = subprocess.Popen(['orca', '--debug-file', str(orca_log)], env=env,
                                stdout=(output / 'orca.log').open('w'), stderr=subprocess.STDOUT,
                                start_new_session=True)
        helpers.append(orca)
        time.sleep(2)
    with (output / 'native.log').open('w') as log:
        process = subprocess.Popen([str(args.binary.resolve()), str(source)], env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            rows = []
            reader = None
            for _ in range(600):
                context = GLib.MainContext.default()
                while context.pending():
                    context.iteration(False)
                desktop = Atspi.get_desktop(0)
                for index in range(desktop.get_child_count()):
                    app = desktop.get_child_at_index(index)
                    if app.get_process_id() == process.pid:
                        reader = app
                        rows = walk(app)
                if any('Good reading starts' in text for _, _, text in rows) and any(name == 'Pages' for _, name, _ in rows):
                    break
                time.sleep(0.1)
            if not any('Good reading starts' in text for _, _, text in rows):
                (output / 'startup-tree.json').write_text(json.dumps(rows, indent=2))
                raise RuntimeError('Publication did not become accessible; inspect startup-tree.json and native.log')
            checks = {
                'native_controls_exposed': any(role in ('button', 'push button') and 'Open' in name for role, name, _ in rows),
                'semantic_heading_exposed': any(role == 'heading' for role, _, _ in rows),
                'publication_text_exposed': any('Good reading starts' in text for _, _, text in rows),
                'page_mode_named': any(name == 'Pages' for _, name, _ in rows),
                'appearance_named': any(name == 'Reading appearance' for _, name, _ in rows),
                'scope': 'AT-SPI tree inspection only; not an Orca usability session',
            }
            notebook = named(reader, 'Notebook')
            checks['notebook_control_named_and_actionable'] = bool(notebook and notebook.is_action()
                and notebook.get_action_iface().do_action(0))
            editor = None
            for _ in range(50):
                editor = named(reader, 'Document notebook, plain text')
                if editor and editor.is_editable_text():
                    break
                time.sleep(0.1)
            checks['notebook_editable_text_exposed'] = bool(editor and editor.is_editable_text()
                and editor.get_state_set().contains(Atspi.StateType.EDITABLE))
            checks['notebook_accepts_accessible_edit'] = bool(editor and editor.is_editable_text()
                and editor.get_editable_text_iface().set_text_contents('An accessible notebook example.'))
            checks['other_source_actions_exposed'] = activate(named(reader, 'Other source actions'))
            wait_for(lambda: (node := named(reader, 'Save current location')) and
                     node.get_state_set().contains(Atspi.StateType.SENSITIVE))
            checks['notebook_location_action_exposed'] = activate(named(reader, 'Save current location'))
            wait_for(lambda: named(reader, 'Passage reference name'))
            rows = walk(reader)
            checks['notebook_reference_name_exposed'] = any(name == 'Passage reference name' for _, name, _ in rows)
            checks['notebook_text_readable'] = any('An accessible notebook example.' in text for _, _, text in rows)
            activate(named(reader, 'Remove passage reference'))
            time.sleep(.2)
            publication = text_node(reader, 'Good reading starts')
            quote_length = len('Good reading starts')
            whitespace_selected = bool(publication and
                publication.get_text_iface().add_selection(4, 5))
            whitespace_disabled = wait_for(lambda:
                (action := named(reader, 'Save selected passage')) and
                not action.get_state_set().contains(Atspi.StateType.SENSITIVE))
            checks['whitespace_selection_disables_save_action'] = bool(
                whitespace_selected and whitespace_disabled)
            if publication:
                publication.get_text_iface().remove_selection(0)
            checks['source_selection_accessible'] = bool(publication and
                publication.get_text_iface().add_selection(0, quote_length))
            time.sleep(.3)
            checks['selected_passage_action_accessible'] = activate(named(reader, 'Save selected passage'))
            time.sleep(.5)
            checks['selected_passage_saved'] = named(reader, 'Passage reference name') is not None
            checks['quotation_details_accessible'] = activate(named(reader, 'Show saved quotation and source location'))
            quote = wait_for(lambda: named(reader, 'Saved quotation', exact=True))
            (output / 'quotation-tree.json').write_text(json.dumps(walk(reader), indent=2))
            checks['saved_quote_readable_and_read_only'] = bool(quote and quote.is_text()
                and Atspi.Text.get_text(quote.get_text_iface(), 0, -1) == 'Good reading starts'
                and not quote.get_state_set().contains(Atspi.StateType.EDITABLE))
            key('Escape')
            checks['connection_action_accessible'] = activate(named(reader, 'Connect this passage to notes'))
            time.sleep(.2)
            checks['notebook_selection_accessible'] = editor.get_text_iface().add_selection(3, 13)
            checks['connect_selected_notes_accessible'] = activate(named(reader, 'Connect notes'))
            time.sleep(.3)
            connected_source = named(reader, 'Open connected passage: Good reading starts — ')
            checks['connected_source_name_and_location_exposed'] = bool(connected_source and
                connected_source.get_name().split(' — ', 1)[1].strip())
            checks['connected_section_navigation_accessible'] = activate(connected_source)
            time.sleep(.5)
            checks['connection_removal_accessible'] = activate(named(reader, 'Remove connection; keep notes and passage'))
            time.sleep(.3)
            if args.orca:
                checks['orca_running'] = orca.poll() is None and speech.poll() is None
                time.sleep(5)  # Let the earlier source/connection announcements finish.
                start = len(orca_log.read_text())
                key('Control_L', 'Shift_L', 'n')
                key('Control_L', 'Shift_L', 'n')
                time.sleep(1)
                key('Control_L', 'End')
                key('Return')
                key('a')
                key('b')
                key('c')
                contents = lambda: Atspi.Text.get_text(editor.get_text_iface(), 0, -1)
                checks['keyboard_can_edit_notebook'] = contents().endswith('\nabc')
                key('Control_L', 'z')
                checks['keyboard_can_undo'] = not contents().endswith('abc')
                key('Control_L', 'Shift_L', 'z')
                checks['keyboard_can_redo'] = contents().endswith('abc')
                key('Tab')
                checks['keyboard_can_leave_editor'] = not editor.get_state_set().contains(Atspi.StateType.FOCUSED)
                reference = named(reader, 'Passage reference name')
                checks['keyboard_reaches_reference_name'] = tab_to(reference)
                key('Control_L', 'a')
                key('e')
                key('x')
                checks['keyboard_can_rename_reference'] = Atspi.Text.get_text(reference.get_text_iface(), 0, -1) == 'ex'
                checks['keyboard_reaches_reference_removal'] = tab_to(named(reader, 'Remove passage reference'))
                time.sleep(.6)
                spoken = [line.split('SPEECH OUTPUT:', 1)[1] for line in orca_log.read_text()[start:].splitlines() if 'SPEECH OUTPUT:' in line]
                checks['orca_announces_notebook_editor'] = any('Document notebook' in line for line in spoken)
                checks['orca_announces_notebook_text'] = any('An accessible notebook example.' in line for line in spoken)
                checks['orca_announces_reference_editor'] = any('Passage reference name' in line for line in spoken)
                checks['orca_announces_reference_removal'] = any('Remove passage reference' in line for line in spoken)
                (output / 'announcements.json').write_text(json.dumps(spoken, indent=2) + '\n')
                checks['keyboard_reaches_reference_follow'] = tab_to(named(reader, 'Read this saved passage'), backwards=True)
                key('space')
                checks['keyboard_follow_keeps_notebook_text'] = contents().endswith('abc')
                tab_to(named(reader, 'Remove passage reference'))
                key('space')
                checks['keyboard_can_remove_reference'] = named(reader, 'Passage reference name') is None
                key('Control_L', 'Shift_L', 'n')
                key('Control_L', 'Shift_L', 'n')
                checks['keyboard_notebook_toggle_restores_text'] = contents().endswith('abc') and editor.get_state_set().contains(Atspi.StateType.SHOWING)
                checks['scope'] = 'Real Orca announcement generation and keyboard input on Xvfb, with a silent Speech Dispatcher module; no human listening/usability claim'
            (output / 'tree.json').write_text(json.dumps(rows, indent=2))
            native_log = (output / 'native.log').read_text()
            checks['no_native_criticals'] = '-CRITICAL **' not in native_log and 'panicked at' not in native_log
            (output / 'checks.json').write_text(json.dumps(checks, indent=2) + '\n')
            print(json.dumps(checks, indent=2))
        finally:
            stop(process)
finally:
    for helper in reversed(helpers):
        stop(helper)
raise SystemExit(1 if any(value is False for value in checks.values()) else 0)
