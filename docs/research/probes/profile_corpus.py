"""Read local document structure; emit aliases and counts, never book text."""
from pathlib import Path
from collections import Counter
import json
import posixpath
import time
import xml.etree.ElementTree as ET
import zipfile
import gi
gi.require_version('Poppler', '0.18')
from gi.repository import Poppler

ROOT = Path('/home/mis/Downloads')
OUTPUT = Path('/code/readero/docs/research/evidence')
OUTPUT.mkdir(parents=True, exist_ok=True)
results, mapping = [], {}
files = sorted((p for p in ROOT.rglob('*') if p.is_file() and p.suffix.lower() in {'.pdf', '.epub', '.md', '.markdown'}), key=lambda p: (p.suffix.lower(), -p.stat().st_size))
counts = Counter()
for path in files:
    kind = path.suffix.lower().lstrip('.')
    counts[kind] += 1
    alias = f'{kind}-{counts[kind]:02d}'
    mapping[alias] = str(path)
    record = {'alias': alias, 'format': kind, 'bytes': path.stat().st_size}
    try:
        if kind == 'epub':
            with zipfile.ZipFile(path) as z:
                entries = z.infolist()
                container = ET.fromstring(z.read('META-INF/container.xml'))
                opf_path = next(x.attrib['full-path'] for x in container.iter() if x.tag.endswith('rootfile'))
                package = ET.fromstring(z.read(opf_path))
                manifest = [x for x in package.iter() if x.tag.endswith('}item')]
                xhtml = [x for x in manifest if x.attrib.get('media-type') == 'application/xhtml+xml']
                tags = Counter()
                max_section = 0
                xml_failures = 0
                for item in xhtml:
                    name = posixpath.normpath(posixpath.join(posixpath.dirname(opf_path), item.attrib['href']))
                    size = z.getinfo(name).file_size
                    max_section = max(max_section, size)
                    if size > 10 * 1024 * 1024:
                        continue
                    try:
                        doc = ET.fromstring(z.read(name))
                        tags.update(x.tag.split('}')[-1] for x in doc.iter())
                    except ET.ParseError:
                        xml_failures += 1
                record.update(epub_version=package.attrib.get('version'), zip_entries=len(entries), uncompressed_bytes=sum(x.file_size for x in entries), spine_items=sum(x.tag.endswith('}itemref') for x in package.iter()), xhtml_items=len(xhtml), largest_xhtml_bytes=max_section, images=sum(x.attrib.get('media-type', '').startswith('image/') for x in manifest), tables=tags['table'], pre_blocks=tags['pre'], code_elements=tags['code'], script_elements=tags['script'], math_elements=tags['math'], xml_parse_failures=xml_failures, encryption_metadata='META-INF/encryption.xml' in z.namelist(), fixed_layout=any(x.attrib.get('property')=='rendition:layout' and x.text=='pre-paginated' for x in package.iter()))
        elif kind == 'pdf':
            started = time.perf_counter()
            doc = Poppler.Document.new_from_file(path.as_uri(), None)
            record.update(pages=doc.get_n_pages(), open_ms=round((time.perf_counter()-started)*1000, 2))
            sample_indices = sorted(set([0, doc.get_n_pages()//2, doc.get_n_pages()-1]))
            record['sample_pages'] = [{'index': i, 'size_points': list(doc.get_page(i).get_size()), 'text_chars': len(doc.get_page(i).get_text()), 'image_regions': len(doc.get_page(i).get_image_mapping())} for i in sample_indices]
    except Exception as e:
        record['error'] = type(e).__name__
    results.append(record)

(OUTPUT/'corpus-profile.json').write_text(json.dumps({'date':'2026-09-04','counts':dict(counts),'documents':results},indent=2)+'\n')
# Keep the private filename mapping outside the project deliverables.
Path('/tmp/readero-research/corpus-map.json').write_text(json.dumps(mapping,indent=2)+'\n')
print(json.dumps({'counts':dict(counts),'epub_max_bytes':max(x['bytes'] for x in results if x['format']=='epub'),'epub_max_section_bytes':max(x.get('largest_xhtml_bytes',0) for x in results),'pdf_max_pages':max(x.get('pages',0) for x in results),'errors':[x['alias'] for x in results if 'error' in x]},indent=2))
