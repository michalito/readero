"""Render authorized EPUB samples in the installed WebKitGTK 6.0.

Run under Xvfb. A private URI handler reads ZIP entries locally. There is no
HTTP server, and CSP denies external network resources. This smoke harness
does not measure native file I/O or GPU performance.
"""
from pathlib import Path
import json
import mimetypes
import sys
import urllib.parse
import zipfile
import gi
for name, version in [('Gtk','4.0'),('WebKit','6.0'),('Soup','3.0')]:
    gi.require_version(name,version)
from gi.repository import Gtk, WebKit, Gio, GLib, Soup

ROOT = Path('/tmp/readero-research')
OUT = Path('/code/readero/docs/research/evidence')
ENGINE = sys.argv[1] if len(sys.argv)>1 else 'foliate'
ALIAS = sys.argv[2] if len(sys.argv)>2 else 'epub-02'
mapping = json.loads((ROOT/'corpus-map.json').read_text())
archive = zipfile.ZipFile(mapping[ALIAS])
sizes = {i.filename:i.file_size for i in archive.infolist()}
foliate = next((ROOT/'johnfactotum-foliate-js').iterdir())
requests = []
Gtk.init()
context = WebKit.WebContext.new()
security = context.get_security_manager()
security.register_uri_scheme_as_secure('reader')
security.register_uri_scheme_as_cors_enabled('reader')

FOLIATE_JS = r'''
import '/foliate/view.js';
import { EPUB } from '/foliate/epub.js';
const start = performance.now();
const sizes = await (await fetch('/sizes.json')).json();
const loader = {
 loadText: async name => name in sizes ? (await fetch('/book/'+name)).text() : null,
 loadBlob: async (name,type) => name in sizes ? new Blob([await (await fetch('/book/'+name)).arrayBuffer()],{type}) : null,
 getSize: name => sizes[name] || 0,
};
const book = await new EPUB(loader).init();
const view = document.createElement('foliate-view');
view.style.cssText = 'display:block;width:100%;height:100%';
document.getElementById('reader').append(view);
await view.open(book);
const section = book.sections.reduce((best,item,i,a) => item.size > a[best].size ? i : best,0);
await view.goTo(section);
await pause(400);
const open = performance.now()-start;
await view.renderer.next();
await pause(200);
const cfi = view.lastLocation.cfi;
const result = {engine:'foliate',section,sectionCount:book.sections.length,firstSectionReadyMs:Math.round(open),cfiProduced:!!cfi};
view.renderer.setAttribute('flow','scrolled');
await pause(300);
await view.goTo(cfi);
await pause(200);
result.scrollKeptSection = view.lastLocation?.section?.current === section || view.resolveCFI(view.lastLocation.cfi).index === section;
result.scrollCFIResolvable = !!view.resolveCFI(cfi);
view.renderer.setAttribute('flow','paginated');
await pause(300); await view.goTo(cfi); await pause(200);
result.pagedCFIResolvable = !!view.resolveCFI(cfi);
result.pagedVisibleRangeContainsAnchor = (()=>{
 const content = view.renderer.getContents()[0];
 const range = view.resolveCFI(cfi).anchor(content.doc);
 const visible = view.lastLocation.range;
 return visible.comparePoint(range.startContainer,range.startOffset) === 0;
})();
window.probe = {view,book,result};
if (CONTINUOUS_PROOF) {
 // Feasibility probe only: bounded three-section window, not a production
 // virtualizer. Reuse Foliate's book resources and CFI conversion unchanged.
 view.style.display='none';
 const scroller=document.createElement('div');
 scroller.style.cssText='height:100%;overflow:auto;overflow-anchor:none';
 document.getElementById('reader').append(scroller);
 const frames=[];
 const center=Math.min(7,book.sections.length-2);
 async function add(index,prepend=false) {
   const frame=document.createElement('iframe');
   frame.style.cssText='display:block;width:100%;border:0;overflow:hidden';
   frame.scrolling='no';
   frame.setAttribute('sandbox','allow-same-origin allow-scripts');
   const ready=new Promise((resolve,reject)=>{frame.onload=resolve;frame.onerror=reject});
   frame.src=await book.sections[index].load();
   if(prepend)scroller.prepend(frame);else scroller.append(frame);
   await ready;
   await frame.contentDocument.fonts.ready;
   await Promise.all(Array.from(frame.contentDocument.images).map(im=>im.decode().catch(()=>{})));
   const doc=frame.contentDocument;
   const style=doc.createElement('style');
   style.textContent='html,body{height:auto!important;overflow:hidden!important;column-count:auto!important;column-width:auto!important}body{max-width:760px;margin:0 auto!important;padding:24px!important}img,svg{max-width:100%;height:auto}pre{white-space:pre-wrap}';
   doc.head.append(style);
   frame.style.height=Math.max(doc.body.scrollHeight,doc.documentElement.scrollHeight)+'px';
   await pause(50);
   frame.style.height=Math.max(doc.body.scrollHeight,doc.documentElement.scrollHeight)+'px';
   frames.push({index,frame});return frame;
 }
 const first=await add(center);
 const second=await add(center+1);
 scroller.scrollTop=second.offsetTop-350;
 await pause(100);
 result.engine='foliate-continuous-proof';
 result.crossChapterBoundaryVisible=second.getBoundingClientRect().top>0 && second.getBoundingClientRect().top<innerHeight;
 const target=Array.from(second.contentDocument.querySelectorAll('p')).find(p=>p.textContent.trim().length>80);
 const range=second.contentDocument.createRange();range.selectNodeContents(target);range.collapse(true);
 const anchor=view.getCFI(center+1,range);
 const before=second.getBoundingClientRect().top;
 const prepended=await add(center-1,true);
 scroller.scrollTop+=prepended.getBoundingClientRect().height;
 await pause(100);
 result.prependAnchorDriftPx=Math.round(Math.abs(second.getBoundingClientRect().top-before));
 result.mountedSectionCount=frames.length;
 result.cfiResolvesToSameParagraph=book.resolveCFI(anchor).anchor(second.contentDocument).startContainer===range.startContainer;
 scroller.remove();view.style.display='block';
 await view.goTo(anchor);await pause(250);
 const pagedDoc=view.renderer.getContents()[0].doc;
 const targetRange=book.resolveCFI(anchor).anchor(pagedDoc);
 result.continuousToPagedAnchorVisible=view.lastLocation.range.comparePoint(targetRange.startContainer,targetRange.startOffset)===0;
 result.customControllerRequired=true;
}
report(result);
'''

EPUBJS_JS = r'''
const start = performance.now();
const book = ePub('reader://local/book/');
await book.ready;
const section = Math.min(7,book.spine.length-1);
const rendition = book.renderTo('reader',{width:'100%',height:'100%',flow:'paginated',spread:'none',allowScriptedContent:false});
await rendition.display(book.spine.get(section).href);
await pause(400);
await rendition.next(); await pause(250);
const cfi = rendition.currentLocation().start.cfi;
const result = {engine:'epubjs',section,sectionCount:book.spine.length,firstSectionReadyMs:Math.round(performance.now()-start),cfiProduced:!!cfi};
rendition.destroy();
const scrolled = book.renderTo('reader',{width:'100%',height:'100%',manager:'continuous',flow:'scrolled',spread:'none',allowScriptedContent:false});
await scrolled.display(cfi); await pause(450);
result.scrollKeptSection = scrolled.currentLocation().start.index === section;
const current = scrolled.manager.views.find(book.spine.get(section));
const next = book.spine.get(section+1);
if(next){
 const appended = scrolled.manager.append(next);
 await appended.display(scrolled.manager.request); appended.show(); await pause(100);
 scrolled.manager.container.scrollTop = appended.element.offsetTop - 400;
 await pause(250);
 const frames = Array.from(document.querySelectorAll('iframe')).filter(x=>x.clientHeight>0);
 result.adjacentChapterFrames = frames.length;
 result.framesInVerticalFlow = frames.length>1 && frames[1].getBoundingClientRect().top >= frames[0].getBoundingClientRect().bottom-2;
 result.chapterBoundaryVisible = frames.some((x,i)=>i>0 && x.getBoundingClientRect().top>0 && x.getBoundingClientRect().top<innerHeight);
}
window.probe = {book,scrolled,result};
report(result);
'''

COMMON = r'''
const pause = ms=>new Promise(r=>setTimeout(r,ms));
const report = data=>window.webkit.messageHandlers.probe.postMessage(JSON.stringify(data));
window.addEventListener('error',e=>report({error:e.message}));
window.addEventListener('unhandledrejection',e=>report({error:String(e.reason),stack:e.reason?.stack}));
'''
scripts = '<script src="/jszip.min.js"></script><script src="/epub.min.js"></script>' if ENGINE=='epubjs' else ''
html = '<!doctype html><html><head><meta charset="utf-8"><style>html,body,#reader{margin:0;width:100%;height:100%;overflow:hidden}body{background:white}</style>'+scripts+'</head><body><div id="reader"></div><script type="module" src="/probe.js"></script></body></html>'
csp = "default-src 'none'; script-src reader: 'unsafe-inline'; style-src reader: blob: 'unsafe-inline'; img-src reader: blob: data:; font-src reader: blob: data:; connect-src reader: blob:; frame-src reader: blob:; object-src 'none'; base-uri reader:"

def serve(request):
    path = urllib.parse.unquote(urllib.parse.urlparse(request.get_uri()).path)
    requests.append(path)
    mime = mimetypes.guess_type(path)[0] or 'application/octet-stream'
    try:
        if path == '/': body,mime = html.encode(),'text/html'
        elif path == '/probe.js': body,mime = (COMMON+'const CONTINUOUS_PROOF='+str(ENGINE=='continuous').lower()+';\n'+(EPUBJS_JS if ENGINE=='epubjs' else FOLIATE_JS)).encode(),'text/javascript'
        elif path == '/sizes.json': body,mime=json.dumps(sizes).encode(),'application/json'
        elif path in ['/epub.min.js','/jszip.min.js']: body=(ROOT/path[1:]).read_bytes(); mime='text/javascript'
        elif path.startswith('/foliate/'):
            target=(foliate/path.removeprefix('/foliate/')).resolve()
            assert target.is_relative_to(foliate)
            body=target.read_bytes()
            if target.suffix=='.js':mime='text/javascript'
        elif path.startswith('/book/'):
            name=path.removeprefix('/book/')
            assert name in sizes and sizes[name]<20*1024*1024
            body=archive.read(name)
        else: raise ValueError('unrecognized resource')
        stream=Gio.MemoryInputStream.new_from_bytes(GLib.Bytes.new(body))
        response=WebKit.URISchemeResponse.new(stream,len(body))
        response.set_content_type(mime)
        headers=Soup.MessageHeaders.new(Soup.MessageHeadersType.RESPONSE)
        headers.append('Content-Security-Policy',csp)
        response.set_http_headers(headers)
        request.finish_with_response(response)
    except Exception as error:
        print(json.dumps({'resource_error':path,'type':type(error).__name__}),flush=True)
        request.finish_error(GLib.Error.new_literal(Gio.io_error_quark(),str(error),Gio.IOErrorEnum.NOT_FOUND))

context.register_uri_scheme('reader',serve)
manager=WebKit.UserContentManager.new()
manager.register_script_message_handler('probe',None)
view=WebKit.WebView(web_context=context,user_content_manager=manager,network_session=WebKit.NetworkSession.new_ephemeral())
view.get_settings().set_enable_developer_extras(True)
window=Gtk.Window(default_width=1050,default_height=800)
window.set_child(view)
loop=GLib.MainLoop()

def message(manager,value):
    result=json.loads(value.to_string())
    result['alias']=ALIAS
    result['resource_requests']=len(requests)
    result['harness']='WebKitGTK 2.52.6, Xvfb, local ZIP URI handler; timings are smoke diagnostics only'
    (OUT/f'webkit-{ENGINE}-{ALIAS}.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result),flush=True)
    def snapshot_done(obj,res):
        try:
            surface=obj.get_snapshot_finish(res)
            if hasattr(surface,'save_to_png'):
                surface.save_to_png(str(ROOT/f'{ENGINE}-{ALIAS}.png'))
            else:
                surface.write_to_png(str(ROOT/f'{ENGINE}-{ALIAS}.png'))
        except Exception as error:print('snapshot:',error)
        loop.quit()
    if 'error' in result:loop.quit()
    else:view.get_snapshot(WebKit.SnapshotRegion.VISIBLE,WebKit.SnapshotOptions.NONE,None,snapshot_done)

manager.connect('script-message-received::probe',message)
view.load_uri('reader://local/')
window.present()
def timeout():
    print(json.dumps({'timeout':True,'requests':len(requests),'last_requests':requests[-5:]}),flush=True)
    loop.quit();return False
GLib.timeout_add_seconds(25,timeout)
loop.run()
window.destroy()
