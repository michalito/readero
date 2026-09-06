#!/usr/bin/env python3
"""Extract Ubuntu development packages to /tmp; never installs system packages.

For normal development, use the apt dependencies documented in README instead.
This helper uses this machine's configured Ubuntu package metadata and verifies
each download against its SHA-256 before extracting it.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import urllib.request

ROOT = Path('/tmp/readero-build')
PREFIX = ROOT / 'native'
PACKAGES = ['libgtk-4-dev', 'libadwaita-1-dev', 'libwebkitgtk-6.0-dev',
            'libpapers-dev', 'libsqlite3-dev', 'pkgconf-bin', 'libpkgconf7', 'krb5-multidev']


def main():
    PREFIX.mkdir(parents=True, exist_ok=True)
    archive = ROOT / 'packages'
    archive.mkdir(exist_ok=True)
    output = subprocess.check_output([
        'apt-cache', 'depends', '--recurse', '--no-recommends', '--no-suggests',
        '--no-conflicts', '--no-breaks', '--no-replaces', '--no-enhances',
        *PACKAGES], text=True)
    names = set(PACKAGES)
    names.update(line for line in output.splitlines()
                 if re.fullmatch(r'[a-z0-9.+-]+-dev', line))
    records = []
    for name in sorted(names):
        meta = subprocess.check_output(['apt-cache', 'show', '--no-all-versions', name], text=True)
        fields = dict(line.split(': ', 1) for line in meta.splitlines() if ': ' in line and not line.startswith(' '))
        if 'Filename' not in fields:
            continue
        dest = archive / Path(fields['Filename']).name
        if not dest.exists():
            url = 'https://archive.ubuntu.com/ubuntu/' + fields['Filename']
            with urllib.request.urlopen(url, timeout=90) as response:
                dest.write_bytes(response.read())
        digest = hashlib.sha256(dest.read_bytes()).hexdigest()
        if digest != fields['SHA256']:
            raise ValueError(f'Checksum mismatch: {name}')
        subprocess.run(['dpkg-deb', '-x', str(dest), str(PREFIX)], check=True)
        records.append({'package': name, 'version': fields['Version'], 'sha256': digest})
        print(f'Prepared {name}', flush=True)
    # Development symlinks normally point to co-installed runtime libraries.
    # Reuse the existing runtime; no alternate engine is injected at run time.
    for link in (PREFIX / 'usr/lib').rglob('*.so'):
        if link.is_symlink() and not link.exists():
            native = Path('/') / link.parent.relative_to(PREFIX) / link.readlink()
            if native.exists():
                link.unlink()
                link.symlink_to(native.resolve())
    (ROOT / 'packages.json').write_text(json.dumps(records, indent=2) + '\n')
    print(f'Build metadata ready under {PREFIX}')


if __name__ == '__main__':
    main()
