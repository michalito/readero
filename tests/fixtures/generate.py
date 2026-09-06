"""Generate original, non-private PDF/EPUB fixtures. Requires Python cairo."""
from pathlib import Path
import cairo,zipfile
import sys
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/readero-build/fixtures');root.mkdir(parents=True,exist_ok=True)
surface=cairo.PDFSurface(str(root/'reading.pdf'),595,842)
ctx=cairo.Context(surface)
for page in range(8):
 ctx.set_source_rgb(1,1,1);ctx.paint();ctx.set_source_rgb(.17,.22,.18);ctx.select_font_face('serif');ctx.set_font_size(26);ctx.move_to(60,90);ctx.show_text(f'The reading practice / {page+1}')
 ctx.set_font_size(13)
 for i in range(28):
  ctx.move_to(60,140+i*21);ctx.show_text(f'Passage {page+1}.{i+1}: A quiet page leaves room for careful reading.')
 ctx.show_page()
surface.finish()
ns='http://www.w3.org/1999/xhtml'
with zipfile.ZipFile(root/'reading.epub','w') as z:
 z.writestr('mimetype','application/epub+zip')
 z.writestr('META-INF/container.xml','<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
 manifest='<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
 spine='';nav=''
 for n in range(12):
  name=f'chapter-{n}.xhtml';manifest+=f'<item id="c{n}" href="{name}" media-type="application/xhtml+xml"/>';spine+=f'<itemref idref="c{n}"/>';nav+=f'<li><a href="{name}">Chapter {n+1}</a></li>'
  paras=''.join(f'<p id="p{i}">Passage {n+1}.{i+1}. A reader follows a thought across the page. The shape of a paragraph changes with the window; the words and their place in the document remain. Good reading rewards attention, curiosity, and a little room to think.</p>' for i in range(10))
  z.writestr(name,f'<html xmlns="{ns}"><head><title>Chapter {n+1}</title></head><body><h1>Chapter {n+1}</h1>{paras}<pre>let reading = comfortable;\nreturn reading;</pre></body></html>')
 z.writestr('package.opf',f'<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">readero-fixture</dc:identifier><dc:title>The reading practice</dc:title><dc:language>en</dc:language></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>')
 z.writestr('nav.xhtml',f'<html xmlns="{ns}" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><ol>{nav}</ol></nav></body></html>')
print('Created synthetic PDF and 12-section EPUB fixtures')

# Authored execution and automatic network access must remain disabled.
with zipfile.ZipFile(root / 'reading.epub') as source, zipfile.ZipFile(root / 'adversarial.epub', 'w') as target:
    for item in source.infolist():
        data = source.read(item.filename)
        if item.filename.startswith('chapter-'):
            text = data.decode()
            payload = """<script>document.documentElement.dataset.authored='yes';
window.webkit?.messageHandlers?.readero?.postMessage('{"type":"external","href":"https://example.com"}');</script>
<img src="http://127.0.0.1:8769/tracker" onerror="document.documentElement.dataset.authored='yes'"/>
<iframe src="file:///etc/passwd"></iframe><form action="http://127.0.0.1:8769/form"><input name="probe"/></form>"""
            text = text.replace('<body>', '<body onload="document.documentElement.dataset.authored=\'yes\'">' + payload)
            data = text.encode()
        target.writestr(item, data)
print('Created adversarial EPUB fixture')

# Mixed portrait/landscape dimensions exercise page-coordinate restoration.
surface = cairo.PDFSurface(str(root / 'mixed.pdf'), 595, 842)
ctx = cairo.Context(surface)
for page, (width, height) in enumerate([(595,842),(842,595),(420,640),(1000,720)] * 2):
    surface.set_size(width, height)
    ctx.set_source_rgb(1, 1, 1); ctx.paint()
    ctx.set_source_rgb(.17, .22, .18); ctx.select_font_face('serif')
    ctx.set_font_size(22); ctx.move_to(40, 75); ctx.show_text(f'Mixed reading page {page + 1}')
    ctx.set_font_size(11)
    for line in range(int((height - 130) / 22)):
        ctx.move_to(40, 120 + line * 22)
        ctx.show_text(f'Reading passage {page+1}.{line+1}: Keep the same point in view.')
    ctx.show_page()
surface.finish()
print('Created mixed-dimension PDF fixture')
