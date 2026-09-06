"""Create and resolve the research dependency graph in a temporary workspace."""
from pathlib import Path
import json
import os
import subprocess

root=Path('/tmp/readero-research')
probe=root/'rust-compat'
(probe/'src').mkdir(parents=True,exist_ok=True)
papers=next((root/'GNOME-papers').iterdir())
manifest='''[package]
name="readero-compatibility-probe"
version="0.0.0"
edition="2024"
[dependencies]
gtk={package="gtk4",version="=0.10.3"}
glib="=0.21.5"
libadwaita="=0.8.1"
webkit6="=0.5.0"
pulldown-cmark="=0.13.4"
rusqlite="=0.40.2"
'''
manifest+='papers-view={path="'+str(papers/'rust/papers-view')+'"}\n'
(probe/'Cargo.toml').write_text(manifest)
(probe/'src/main.rs').write_text('fn main() {}\n')
environment=dict(os.environ,CARGO_HOME=str(root/'cargo-home'))
data=subprocess.check_output(['cargo','metadata','--manifest-path',str(probe/'Cargo.toml'),'--format-version','1'],env=environment,text=True)
(root/'cargo-metadata.json').write_text(data)
metadata=json.loads(data)
glib_versions=[p['version'] for p in metadata['packages'] if p['name']=='glib']
assert glib_versions==['0.21.5'],glib_versions
print('Dependency resolution succeeded; a single GLib 0.21 binding family is present. No GUI compile/link was performed.')
