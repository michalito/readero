"""Exercise installation without modifying the user's real application or data."""

import argparse
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("local_install", ROOT / "scripts/local-install.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary = self.root / "built-readero"
        self.binary.write_bytes(b"#!/bin/sh\necho first\n")
        self.binary.chmod(0o755)
        self.args = argparse.Namespace(prefix="/usr", destdir=str(self.root / "stage"), binary=str(self.binary))

    def test_stage_upgrade_and_uninstall_preserve_other_files_and_data(self):
        with patch.object(installer, "refresh") as refresh:
            installer.install(self.args)
            destination = self.root / "stage/usr"
            executable = destination / "bin/readero"
            self.assertEqual(executable.read_bytes(), self.binary.read_bytes())
            self.assertEqual(executable.stat().st_mode & 0o777, 0o755)
            desktop = destination / f"share/applications/{installer.APP_ID}.desktop"
            self.assertIn('Exec="/usr/bin/readero" %f', desktop.read_text())
            self.assertNotIn(str(self.root), desktop.read_text())
            self.assertEqual(desktop.stat().st_mode & 0o777, 0o644)
            for relative in installer.files():
                self.assertTrue((destination / relative).is_file())
            if shutil.which("desktop-file-validate"):
                subprocess.run(["desktop-file-validate", str(desktop)], check=True)
            # An open executable must retain its original inode across updates.
            with executable.open("rb") as old:
                self.binary.write_bytes(b"#!/bin/sh\necho second\n")
                installer.install(self.args)
                self.assertIn(b"first", old.read())
            self.assertIn(b"second", executable.read_bytes())
            data = destination / "share/readero/reading.sqlite3"
            data.parent.mkdir(parents=True)
            data.write_text("keep bookmarks")
            unrelated = destination / "bin/other-app"
            unrelated.write_text("keep me")
            installer.uninstall(self.args)
            installer.uninstall(self.args)  # repeated removal is harmless
            self.assertFalse(executable.exists())
            self.assertFalse(desktop.exists())
            self.assertEqual(data.read_text(), "keep bookmarks")
            self.assertEqual(unrelated.read_text(), "keep me")
            refresh.assert_not_called()

    def test_user_prefix_with_spaces_and_desktop_metacharacters(self):
        self.args.prefix = str(self.root / 'a space % $ ` " \\ prefix')
        self.args.destdir = ""
        with patch.object(installer, "refresh") as refresh:
            installer.install(self.args)
            refresh.assert_called_once_with(Path(self.args.prefix))
        desktop = Path(self.args.prefix) / f"share/applications/{installer.APP_ID}.desktop"
        text = desktop.read_text()
        self.assertIn("%%", text)
        self.assertIn("\\\\$", text)
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", str(desktop)], check=True)

    def test_missing_binary_leaves_existing_install_untouched(self):
        installer.install(self.args)
        before = (self.root / "stage/usr/bin/readero").read_bytes()
        self.binary.unlink()
        with self.assertRaisesRegex(ValueError, "Missing executable"):
            installer.install(self.args)
        self.assertEqual((self.root / "stage/usr/bin/readero").read_bytes(), before)

    def test_reject_relative_or_traversing_install_paths(self):
        for prefix, stage in [("relative", ""), ("/usr/../etc", ""), ("/usr", "relative"),
                              ("/usr", "/tmp/../etc"), ("/usr\nExec=bad", "")]:
            with self.subTest(prefix=prefix, stage=stage), self.assertRaises(ValueError):
                installer.layout(prefix, stage)

    def test_deb_install_selects_current_version_and_architecture(self):
        dist = self.root / "packages with spaces"
        dist.mkdir()
        version = installer.manifest()["version"]
        selected = dist / f"readero_{version}_amd64.deb"
        for name in (selected.name, "readero_0.0.1_amd64.deb", f"readero_{version}_arm64.deb"):
            (dist / name).write_bytes(b"test package placeholder")
        with patch("sys.argv", ["local-install.py", "install-deb", "--dist-dir", str(dist)]), \
                patch.object(installer.subprocess, "check_output", return_value="amd64\n"), \
                patch.object(installer, "privileged") as privileged:
            installer.main()
            privileged.assert_called_once_with([
                "apt-get", "install", "--yes", "--reinstall", str(selected),
            ])


@unittest.skipUnless(shutil.which("git"), "Git required for source-update checks")
class UpdateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.upstream = self.root / "upstream"
        self.checkout = self.root / "checkout"
        self.upstream.mkdir()
        self.git(self.upstream, "init", "-b", "main")
        self.commit(self.upstream, "initial")
        self.git(self.root, "clone", str(self.upstream), str(self.checkout))

    def git(self, directory, *args):
        return subprocess.check_output(
            ["git", "-c", "user.name=Installer Test", "-c", "user.email=test@example.invalid", *args],
            cwd=directory, text=True, stderr=subprocess.PIPE,
            env={**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"},
        ).strip()

    def commit(self, directory, content):
        (directory / "file").write_text(content)
        self.git(directory, "add", "file")
        self.git(directory, "commit", "-m", content)

    def test_fast_forward_fetches_latest_commit(self):
        self.commit(self.upstream, "new version")
        installer.update_source(self.checkout)
        self.assertEqual((self.checkout / "file").read_text(), "new version")

    def test_local_edits_and_untracked_files_are_preserved(self):
        self.git(self.checkout, "config", "status.showUntrackedFiles", "no")
        for name in ("file", "untracked"):
            with self.subTest(name=name):
                original = (self.checkout / "file").read_text()
                (self.checkout / name).write_text("local work")
                with self.assertRaisesRegex(ValueError, "local changes"):
                    installer.update_source(self.checkout)
                self.assertEqual((self.checkout / name).read_text(), "local work")
                if name == "file":
                    (self.checkout / name).write_text(original)

    def test_missing_upstream_is_actionable(self):
        self.git(self.checkout, "branch", "--unset-upstream")
        with self.assertRaisesRegex(ValueError, "no upstream"):
            installer.update_source(self.checkout)

    def test_detached_head_is_actionable(self):
        self.git(self.checkout, "checkout", "--detach")
        with self.assertRaisesRegex(ValueError, "Detached HEAD"):
            installer.update_source(self.checkout)

    def test_divergence_does_not_replace_local_commits(self):
        self.commit(self.upstream, "upstream work")
        self.commit(self.checkout, "local commit")
        head = self.git(self.checkout, "rev-parse", "HEAD")
        with self.assertRaises(subprocess.CalledProcessError):
            installer.update_source(self.checkout)
        self.assertEqual(self.git(self.checkout, "rev-parse", "HEAD"), head)
        self.assertEqual((self.checkout / "file").read_text(), "local commit")


if __name__ == "__main__":
    unittest.main()
