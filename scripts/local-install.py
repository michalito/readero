#!/usr/bin/env python3
"""Local install and preflight helpers; Python 3.11+, no third-party modules."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
APP_ID = "io.github.readero.Readero"
LIBRARIES = {
    "gtk4": "4.14",
    "libadwaita-1": "1.5",
    "webkitgtk-6.0": "2.42",
    "papers-document-4.0": "4",
    "papers-view-4.0": "49",
    "sqlite3": "3.34.1",
}
PACKAGES = [
    "build-essential", "pkg-config", "python3", "git", "libgtk-4-dev",
    "libadwaita-1-dev", "libwebkitgtk-6.0-dev", "libpapers-dev",
    "libsqlite3-dev", "dpkg-dev", "desktop-file-utils", "libgtk-3-bin",
]


def manifest():
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]


def run(args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def privileged(args):
    if os.geteuid() != 0:
        if not shutil.which("sudo"):
            raise ValueError("sudo is required for system packages; user-local installation needs no sudo.")
        args = ["sudo", *args]
    run(args)


def doctor(cargo):
    problems = []
    for command in (cargo, "rustc", "cc", "python3"):
        if not shutil.which(command):
            problems.append(f"Missing command: {command}")
    if shutil.which("rustc"):
        version = subprocess.check_output(["rustc", "--version"], text=True).split()[1]
        required = manifest()["rust-version"]
        numeric = lambda value: tuple(int(x) for x in value.split("-")[0].split("."))
        if numeric(version) < numeric(required):
            problems.append(f"Rust {version} is too old; need {required} or newer. Run make toolchain and select stable Rust.")
        else:
            print(f"Rust {version} (minimum {required})")
    pkg_config = os.environ.get("PKG_CONFIG", "pkg-config")
    if not shutil.which(pkg_config):
        problems.append(f"Missing pkg-config: {pkg_config}")
    else:
        for library, minimum in LIBRARIES.items():
            result = subprocess.run([pkg_config, "--print-errors", f"--atleast-version={minimum}", library],
                                    capture_output=True, text=True)
            if result.returncode:
                problems.append(f"Need {library} >= {minimum}: {result.stderr.strip()}")
            else:
                version = subprocess.check_output([pkg_config, "--modversion", library], text=True).strip()
                print(f"{library} {version} (minimum {minimum})")
    if problems:
        raise ValueError("\n".join(problems) + "\nRun make deps on Ubuntu 26.04; see README for LOCAL_DEPS=1.")
    print("Ready to build Readero. Dependencies will use Cargo.lock (--locked).")


def layout(prefix, destdir):
    prefix = Path(prefix).expanduser()
    if not prefix.is_absolute() or any(ord(c) < 32 or ord(c) == 127 for c in str(prefix)):
        raise ValueError("PREFIX must be an absolute path without control characters.")
    if ".." in prefix.parts:
        raise ValueError("PREFIX must not contain '..'.")
    if destdir:
        stage = Path(destdir).expanduser()
        if not stage.is_absolute() or ".." in stage.parts:
            raise ValueError("DESTDIR must be an absolute staging path without '..'.")
        return prefix, stage / prefix.relative_to("/")
    return prefix, prefix


def files():
    return {
        f"share/icons/hicolor/scalable/apps/{APP_ID}.svg": ROOT / "data" / f"{APP_ID}.svg",
        "share/doc/readero/PAPERS-COPYING": ROOT / "vendor/PAPERS-COPYING",
        "share/doc/readero/FOLIATE-LICENSE": ROOT / "assets/foliate/LICENSE",
        "share/doc/readero/quiet-reading.md": ROOT / "examples/quiet-reading.md",
    }


def desktop_entry(prefix):
    executable = str(prefix / "bin/readero")
    # Desktop Entry string escaping followed by Exec's quoted-argument escaping.
    escaped = executable.replace("\\", "\\\\\\\\").replace('"', '\\\\"')
    escaped = escaped.replace("$", "\\\\$").replace("`", "\\\\`").replace("%", "%%")
    source = (ROOT / "data" / f"{APP_ID}.desktop").read_text()
    return source.replace("Exec=readero %f", f'Exec="{escaped}" %f')


def atomic_write(destination, content, mode):
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{destination.name}.", dir=destination.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(content)
            os.fchmod(stream.fileno(), mode)
        os.replace(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)


def refresh(prefix):
    for command, args in (
        ("update-desktop-database", [str(prefix / "share/applications")]),
        ("gtk-update-icon-cache", ["-f", "-t", str(prefix / "share/icons/hicolor")]),
    ):
        if shutil.which(command) and Path(args[-1]).is_dir():
            result = subprocess.run([command, *args], capture_output=True, text=True)
            if result.returncode:
                print(f"Warning: {command}: {result.stderr.strip()}", file=sys.stderr)


def install(args):
    prefix, destination = layout(args.prefix, args.destdir)
    binary = Path(args.binary)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError(f"Missing executable {binary}; run make build first.")
    # Read all inputs before modifying the installation. Replace the executable
    # atomically so updating an already-running application is possible.
    payload = {relative: (source.read_bytes(), 0o644) for relative, source in files().items()}
    payload[f"share/applications/{APP_ID}.desktop"] = (desktop_entry(prefix).encode(), 0o644)
    payload["bin/readero"] = (binary.read_bytes(), 0o755)
    for relative, (content, mode) in payload.items():
        atomic_write(destination / relative, content, mode)
    if not args.destdir:
        refresh(prefix)
    print(f"Installed Readero {manifest()['version']} to {destination}")
    if not args.destdir:
        print(f"Launch: {prefix / 'bin/readero'}")
        if str(prefix / "bin") not in os.get_exec_path():
            print(f"For terminal access, add {prefix / 'bin'} to PATH. The desktop launcher uses an absolute path.")


def uninstall(args):
    prefix, destination = layout(args.prefix, args.destdir)
    for relative in [*files(), f"share/applications/{APP_ID}.desktop", "bin/readero"]:
        (destination / relative).unlink(missing_ok=True)
    doc = destination / "share/doc/readero"
    if doc.is_dir() and not any(doc.iterdir()):
        doc.rmdir()
    if not args.destdir:
        refresh(prefix)
    print(f"Removed Readero from {destination}; reading data and bookmarks are preserved.")


def update_source(root=ROOT):
    if subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=all"], cwd=root, text=True).strip():
        raise ValueError("Checkout has local changes. Commit or stash them before make update.")
    branch_result = subprocess.run(["git", "symbolic-ref", "--quiet", "--short", "HEAD"], cwd=root,
                                   capture_output=True, text=True)
    if branch_result.returncode:
        raise ValueError("Detached HEAD cannot be updated. Switch to a tracked branch, or use make install for this checkout.")
    branch = branch_result.stdout.strip()
    upstream = subprocess.run(["git", "rev-parse", "--abbrev-ref", "@{upstream}"], cwd=root,
                              capture_output=True, text=True)
    if upstream.returncode:
        raise ValueError(f"Branch {branch} has no upstream. Configure a remote/tracking branch, or use make install for this checkout.")
    print(f"Updating {branch} from {upstream.stdout.strip()} (fast-forward only)", flush=True)
    run(["git", "pull", "--ff-only", "--no-rebase"], cwd=root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor").add_argument("--cargo", default="cargo")
    commands.add_parser("deps")
    commands.add_parser("update-source")
    package = commands.add_parser("install-deb")
    package.add_argument("--dist-dir", default="dist")
    for command in ("install", "uninstall"):
        sub = commands.add_parser(command)
        sub.add_argument("--prefix", default=str(Path.home() / ".local"))
        sub.add_argument("--destdir", default="")
        if command == "install":
            sub.add_argument("--binary", default="target/release/readero")
    args = parser.parse_args()
    if args.command == "doctor":
        doctor(args.cargo)
    elif args.command == "deps":
        release = dict(line.split("=", 1) for line in Path("/etc/os-release").read_text().splitlines() if "=" in line)
        if release.get("ID", "").strip('"') != "ubuntu" or release.get("VERSION_ID", "").strip('"') != "26.04":
            raise ValueError("Automatic dependency setup supports Ubuntu 26.04. See README for native library requirements.")
        privileged(["apt-get", "update"])
        privileged(["apt-get", "install", "--yes", *PACKAGES])
    elif args.command == "update-source":
        update_source()
    elif args.command == "install-deb":
        arch = subprocess.check_output(["dpkg", "--print-architecture"], text=True).strip()
        package = Path(args.dist_dir).resolve() / f"readero_{manifest()['version']}_{arch}.deb"
        if not package.is_file():
            raise ValueError(f"Missing package {package}; run make package first.")
        privileged(["apt-get", "install", "--yes", "--reinstall", str(package)])
    else:
        {"install": install, "uninstall": uninstall}[args.command](args)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"readero: {error}", file=sys.stderr)
        sys.exit(1)
