"""Test recovery of the prior committed locator after a killed writer."""
from pathlib import Path
import json
import sqlite3
import subprocess
import sys

root = Path('/tmp/readero-research')
root.mkdir(exist_ok=True)
database = root / 'state-probe.sqlite3'
connection = sqlite3.connect(database)
connection.execute('PRAGMA journal_mode=DELETE')
connection.execute('PRAGMA synchronous=FULL')
connection.execute('CREATE TABLE IF NOT EXISTS reading_state(id TEXT PRIMARY KEY, locator TEXT NOT NULL)')
connection.execute('INSERT OR REPLACE INTO reading_state VALUES (?,?)', ('doc', json.dumps({'version':1,'format':'pdf','page_index':2,'y':0.4})))
connection.commit()
connection.close()
code = "import sqlite3,sys,time; c=sqlite3.connect(sys.argv[1]); c.execute('BEGIN IMMEDIATE'); c.execute(\"UPDATE reading_state SET locator='uncommitted' WHERE id='doc'\"); print('ready',flush=True); time.sleep(30)"
process = subprocess.Popen([sys.executable,'-c',code,str(database)], stdout=subprocess.PIPE, text=True)
assert process.stdout.readline().strip() == 'ready'
process.kill()
process.wait()
connection = sqlite3.connect(database)
locator = connection.execute('SELECT locator FROM reading_state').fetchone()[0]
integrity = connection.execute('PRAGMA integrity_check').fetchone()[0]
connection.close()
result = {'sqlite_version':sqlite3.sqlite_version,'journal_mode':'DELETE','synchronous':'FULL','committed_location_survived_killed_writer':json.loads(locator)['page_index']==2,'integrity_check':integrity,'scope':'Process termination during an uncommitted transaction; not a power-loss or filesystem fault test.'}
Path('/code/readero/docs/research/evidence/sqlite-probe.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result))
