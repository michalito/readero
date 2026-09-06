"""Exercise installed Papers 50.2 against local PDFs on a virtual display.

This is a component smoke test, not an application benchmark. Original files are
read-only; results contain aliases and numerical data only.
"""
from pathlib import Path
import json
import time
import gi
for name, version in [('Gtk', '4.0'), ('Adw', '1'), ('PapersDocument', '4.0'), ('PapersView', '4.0'), ('Poppler', '0.18')]:
    gi.require_version(name, version)
from gi.repository import Gtk, Adw, GLib, PapersDocument as D, PapersView as V, Poppler
import cairo

OUT = Path('/code/readero/docs/research/evidence')
mapping = json.loads(Path('/tmp/readero-research/corpus-map.json').read_text())
Gtk.init()
Adw.init()
D.init()
context = GLib.MainContext.default()

def settle(seconds=0.35):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        while context.pending():
            context.iteration(False)
        time.sleep(0.005)

results = []
for alias in ['pdf-01', 'pdf-03', 'pdf-04']:
    path = Path(mapping[alias])
    job = V.JobLoad.new()
    job.set_uri(path.as_uri())
    begin = time.perf_counter()
    job.run()
    doc = job.get_loaded_document()
    record = {'alias': alias, 'load_ms': round(1000*(time.perf_counter()-begin), 2), 'pages': doc.get_n_pages()}
    # Papers 50's application supplies this construct-only model explicitly.
    # The convenience constructor leaves it null in the tested release.
    annotation_model = V.AnnotationModel()
    model = V.DocumentModel(annotation_model=annotation_model)
    undo = V.UndoContext(document_model=model)
    annots = V.AnnotationsContext(document_model=model, undo_context=undo)
    search = V.SearchContext.new(model)
    model.set_page_layout(V.PageLayout.SINGLE)
    model.set_sizing_mode(V.SizingMode.FIT_WIDTH)
    model.set_continuous(True)
    view = V.View.new()
    view.set_annotations_context(annots)
    view.set_search_context(search)
    view.set_model(model)
    model.set_document(doc)
    view.set_page_cache_size(64 * 1024 * 1024)
    scroll = Gtk.ScrolledWindow()
    scroll.set_child(view)
    window = Gtk.Window(default_width=1000, default_height=750)
    window.set_child(scroll)
    window.present()
    settle()
    target_page = min(1, doc.get_n_pages()-1)
    model.set_page(target_page)
    settle()
    view.select_all()
    record['selection_nonempty'] = bool(view.get_selected_text())
    record['selection_characters'] = len(view.get_selected_text() or '')
    model.set_continuous(False)
    settle()
    record['paged_kept_page'] = model.get_page() == target_page
    model.set_continuous(True)
    settle()
    record['scroll_kept_page'] = model.get_page() == target_page
    point = view.get_document_point_for_view_point(view.get_width()/2, 100)
    record['document_coordinate_available'] = point is not None
    if point:
        record['point'] = {'page': point.page_index, 'x': round(point.point_on_page.x, 2), 'y': round(point.point_on_page.y, 2)}
        dest = D.LinkDest.new_xyz(point.page_index, point.point_on_page.x, point.point_on_page.y, 0, False, True, False)
        link = D.Link.new('Research location', D.LinkAction.new_dest(dest))
        model.set_page(0)
        settle()
        view.handle_link(link)
        settle()
        record['coordinate_link_restored_page'] = model.get_page() == point.page_index
    # Derive a local token without including it in results.
    text = Poppler.Document.new_from_file(path.as_uri(), None).get_page(target_page).get_text()
    token = next((x for x in text.split() if len(x)>5 and x.isalpha()), None)
    if token:
        search.set_search_term(token)
        settle(0.6)
        record['search_result_count'] = search.get_result_model().get_n_items()
    # Render one sample with system Poppler for visual inspection only.
    if alias == 'pdf-01':
        page = Poppler.Document.new_from_file(path.as_uri(), None).get_page(0)
        width, height = page.get_size()
        scale = 1100 / width
        surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, int(width*scale), int(height*scale))
        cr = cairo.Context(surface)
        cr.set_source_rgb(1, 1, 1); cr.paint(); cr.scale(scale, scale)
        page.render(cr)
        surface.write_to_png('/tmp/readero-research/pdf-sample.png')
    window.destroy()
    settle(0.1)
    results.append(record)

(OUT/'native-pdf-probe.json').write_text(json.dumps({'environment':'Xvfb, GTK cairo renderer, Python GI; not a GPU/performance benchmark','results':results},indent=2)+'\n')
print(json.dumps(results,indent=2))
