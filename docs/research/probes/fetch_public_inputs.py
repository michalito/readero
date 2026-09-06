"""Fetch public source and Ubuntu introspection metadata for local research.

No user document is read or uploaded. Packages are extracted under /tmp, never
installed into the system. Run with Python 3; internet access is required.
"""
from pathlib import Path
import concurrent.futures
import hashlib
import json
import subprocess
import tarfile
import urllib.request

ROOT = Path('/tmp/readero-research')
ROOT.mkdir(exist_ok=True)

def fetch(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'readero-stack-research'})
    with urllib.request.urlopen(request, timeout=40) as response:
        return response.read()

def repo_input(repo, ref):
    meta = json.loads(fetch(f'https://api.github.com/repos/{repo}/commits/{ref}'))
    sha = meta['sha']
    archive = fetch(f'https://codeload.github.com/{repo}/tar.gz/{sha}')
    path = ROOT / (repo.replace('/', '-') + '.tar.gz')
    path.write_bytes(archive)
    directory = ROOT / ('readium' if repo == 'readium/ts-toolkit' else repo.replace('/', '-'))
    directory.mkdir(exist_ok=True)
    with tarfile.open(path) as source:
        source.extractall(directory, filter='data')
    return {'kind': 'source', 'repository': repo, 'requested_ref': ref,
            'commit': sha, 'commit_date': meta['commit']['committer']['date'],
            'archive': str(path), 'sha256': hashlib.sha256(archive).hexdigest()}

def file_input(name, url):
    body = fetch(url)
    (ROOT / name).write_bytes(body)
    return {'kind':'public-file', 'name':name, 'url':url,
            'sha256':hashlib.sha256(body).hexdigest()}

def package_input(package):
    data = subprocess.check_output(['apt-cache', 'show', package], text=True)
    fields = dict(line.split(': ', 1) for line in data.split('\n\n')[0].splitlines() if ': ' in line and not line.startswith(' '))
    path = ROOT / Path(fields['Filename']).name
    url = 'https://archive.ubuntu.com/ubuntu/' + fields['Filename']
    body = fetch(url)
    assert hashlib.sha256(body).hexdigest() == fields['SHA256'], package
    path.write_bytes(body)
    subprocess.run(['dpkg-deb', '-x', str(path), str(ROOT / 'gi')], check=True)
    return {'kind': 'typelib', 'package': package, 'version': fields['Version'],
            'url': url, 'sha256': fields['SHA256']}

tasks = [
    (repo_input, ('GNOME/papers', '50.2')),
    (repo_input, ('johnfactotum/foliate-js', '78914aef4466eb960965702401634c2cb348e9b1')),
    (repo_input, ('futurepress/epub.js', 'v0.3.93')),
    (repo_input, ('readium/ts-toolkit', '893a5cc362605ad19f1be1d905159c1b7282c68d')),
    (file_input, ('epub.min.js', 'https://cdn.jsdelivr.net/npm/epubjs@0.3.93/dist/epub.min.js')),
    (file_input, ('jszip.min.js', 'https://cdn.jsdelivr.net/npm/jszip@3.10.1/dist/jszip.min.js')),
    (package_input, ('gir1.2-papers-4.0',)),
    (package_input, ('gir1.2-webkit-6.0',)),
    (package_input, ('gir1.2-javascriptcoregtk-6.0',)),
    (package_input, ('gir1.2-poppler-0.18',)),
]
if __name__ == '__main__':
    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
        future_tasks = {executor.submit(fn, *args): args for fn, args in tasks}
        for future in concurrent.futures.as_completed(future_tasks):
            try:
                result = future.result()
            except Exception as error:
                result = {'input': future_tasks[future], 'error': str(error)}
            results.append(result)
            print(json.dumps(result), flush=True)
    (ROOT / 'public-inputs.json').write_text(json.dumps(results, indent=2) + '\n')
