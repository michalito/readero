use gtk::{gio, glib, prelude::*};
use papers_document::{self as document, prelude::*};
use papers_view::{self as papers, prelude::*};
use readero::document::{Anchor, Locator, Mode, Settings};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub struct Pdf {
    pub view: papers::View,
    pub model: papers::DocumentModel,
    pub scroll: gtk::ScrolledWindow,
    pub search: papers::SearchContext,
    restoring: Cell<bool>,
    restore_serial: Cell<u64>,
    last_stable: RefCell<Option<Locator>>,
    changed: Rc<dyn Fn(Locator, String)>,
    // AnnotationsContext borrows this context; keep it alive through view disposal.
    _undo: papers::UndoContext,
}
impl Pdf {
    pub fn new(
        doc: &document::Document,
        settings: &Settings,
        changed: impl Fn(Locator, String) + 'static,
    ) -> Rc<Self> {
        // Papers 50 requires all four model/context objects before first display.
        let annotations: papers::AnnotationModel = glib::Object::new();
        let model: papers::DocumentModel = glib::Object::builder()
            .property("annotation-model", &annotations)
            .build();
        let undo = papers::UndoContext::new(&model);
        let context: papers::AnnotationsContext = glib::Object::builder()
            .property("document-model", &model)
            .property("undo-context", &undo)
            .build();
        let search = papers::SearchContext::new(&model);
        let view = papers::View::new();
        view.set_model(&model);
        view.set_annotations_context(&context);
        view.set_search_context(&search);
        view.set_page_cache_size(64 * 1024 * 1024);
        view.set_allow_links_change_zoom(false);
        model.set_page_layout(papers::PageLayout::Single);
        model.set_min_scale(0.25);
        model.set_max_scale(5.0);
        let scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&view)
            .build();
        scroll.add_css_class("pdf-surface");
        let pdf = Rc::new(Self {
            view,
            model,
            scroll,
            search,
            restoring: Cell::new(true),
            restore_serial: Cell::new(0),
            last_stable: RefCell::new(None),
            changed: Rc::new(changed),
            _undo: undo,
        });
        pdf.configure(settings);
        pdf.model.set_document(Some(doc));
        for adjustment in [pdf.scroll.vadjustment(), pdf.scroll.hadjustment()] {
            let weak = Rc::downgrade(&pdf);
            adjustment.connect_value_changed(move |_| {
                if let Some(pdf) = weak.upgrade() {
                    pdf.emit_position();
                }
            });
        }
        let weak = Rc::downgrade(&pdf);
        pdf.model.connect_page_changed(move |_, _, _| {
            if let Some(pdf) = weak.upgrade() {
                pdf.emit_position();
            }
        });
        pdf
    }
    pub fn configure(&self, settings: &Settings) {
        self.model.set_continuous(settings.mode == Mode::Scroll);
        self.model.set_rotation(settings.rotation);
        self.model
            .set_sizing_mode(match settings.pdf_sizing.as_str() {
                "page" => papers::SizingMode::FitPage,
                "custom" => papers::SizingMode::Free,
                _ => papers::SizingMode::FitWidth,
            });
        if settings.pdf_sizing == "custom" {
            self.model.set_scale(settings.pdf_scale);
        }
    }
    pub fn settings(self: &Rc<Self>, settings: &Settings) {
        let locator = self.stable_location();
        self.restoring.set(true);
        self.configure(settings);
        self.restore(locator);
    }
    pub fn location(&self) -> Option<Locator> {
        let x = f64::from(self.view.width()) / 2.0;
        // A point near the top of the reading surface represents the passage.
        for y in [24.0, 48.0, 96.0, 160.0, 0.0] {
            if let Some(point) = self.view.document_point_for_view_point(x, y) {
                let position = point.point_on_page();
                return Some(Locator {
                    version: 1,
                    anchor: Anchor::Pdf {
                        page: point.page_index(),
                        x: position.x().max(0.0),
                        y: position.y().max(0.0),
                        viewport_y: y,
                    },
                });
            }
        }
        // The viewport can lie entirely in the gutter of a mixed-size PDF.
        // Keep the last readable passage instead of inventing a page origin.
        None
    }
    pub fn restore(self: &Rc<Self>, locator: Option<Locator>) {
        let mut locator = locator;
        if let Some(Locator {
            anchor: Anchor::Pdf { page, x, y, .. },
            ..
        }) = locator.as_mut()
            && let Some(document) = self.model.document()
        {
            *page = (*page).min(document.n_pages() - 1).max(0);
            let (width, height) = document.page_size(*page);
            *x = x.min(width);
            *y = y.min(height);
        }
        self.restoring.set(true);
        if locator.is_some() {
            self.last_stable.replace(locator.clone());
        }
        let serial = self.restore_serial.get() + 1;
        self.restore_serial.set(serial);
        let weak = Rc::downgrade(self);
        // Native page extents are available after allocation. A second settled
        // frame applies the viewport offset and releases location publication.
        glib::timeout_add_local_once(Duration::from_millis(80), move || {
            let Some(pdf) = weak.upgrade() else {
                return;
            };
            if pdf.restore_serial.get() != serial {
                return;
            }
            let offset = if let Some(Locator {
                anchor:
                    Anchor::Pdf {
                        page,
                        x,
                        y,
                        viewport_y,
                    },
                ..
            }) = locator
            {
                let page = page.min(pdf.pages() - 1).max(0);
                // The saved point is at the viewport's horizontal center.
                // Apply that offset before Papers clamps the destination at
                // the page edge, accounting for the current rotation.
                let center = f64::from(pdf.view.width()) / (2.0 * pdf.model.scale());
                let (left, top) = match pdf.model.rotation() {
                    90 => (x, y + center),
                    180 => (x + center, y),
                    270 => (x, y - center),
                    _ => (x - center, y),
                };
                let dest = document::LinkDest::new_xyz(page, left, top, 0.0, true, true, false);
                let link = document::Link::new(
                    Some("Reading position"),
                    &document::LinkAction::new_dest(&dest),
                );
                pdf.view.handle_link(&link);
                viewport_y
            } else {
                0.0
            };
            let weak = Rc::downgrade(&pdf);
            glib::timeout_add_local_once(Duration::from_millis(80), move || {
                let Some(pdf) = weak.upgrade() else {
                    return;
                };
                if pdf.restore_serial.get() != serial {
                    return;
                }
                let adjustment = pdf.scroll.vadjustment();
                adjustment.set_value((adjustment.value() - offset).max(adjustment.lower()));
                pdf.restoring.set(false);
                // Fit/rotation may clamp alignment at a page edge. The intended
                // passage is still the anchor until the reader moves again.
                let stable = pdf.last_stable.borrow().clone();
                if let Some(locator) = stable {
                    (pdf.changed)(locator, pdf.position_label());
                } else {
                    pdf.emit_position();
                }
            });
        });
    }
    fn emit_position(&self) {
        if !self.restoring.get()
            && let Some(locator) = self.location()
        {
            self.last_stable.replace(Some(locator.clone()));
            (self.changed)(locator, self.position_label());
        }
    }
    pub fn stable_location(&self) -> Option<Locator> {
        self.last_stable
            .borrow()
            .clone()
            .or_else(|| self.location())
    }
    pub fn is_restoring(&self) -> bool {
        self.restoring.get()
    }
    pub fn pages(&self) -> i32 {
        self.model.document().map_or(0, |doc| doc.n_pages())
    }
    pub fn position_label(&self) -> String {
        let label = self
            .model
            .document()
            .and_then(|doc| doc.page_label(self.model.page()))
            .map(|x| x.to_string())
            .unwrap_or_else(|| (self.model.page() + 1).to_string());
        format!("Page {label} of {}", self.pages())
    }
    pub fn next(&self) {
        self.view.next_page();
    }
    pub fn previous(&self) {
        self.view.previous_page();
    }
    pub fn goto_page(&self, label: &str) {
        self.model.set_page_by_label(label);
    }
    pub fn find(&self, query: &str) {
        if query.is_empty() && self.search.is_active() {
            self.search.release();
        } else if !query.is_empty() && !self.search.is_active() {
            self.search.activate();
        }
        self.search.set_search_term(query);
    }
    pub fn cancel(&self) {
        self.restoring.set(true);
        if self.search.is_active() {
            self.search.release();
        }
    }
}

pub fn load(
    path: &std::path::Path,
    finished: impl Fn(Result<document::Document, glib::Error>, bool) + 'static,
) -> papers::JobLoad {
    let job = papers::JobLoad::new();
    job.set_uri(gio::File::for_path(path).uri().as_str());
    job.connect_finished(move |job| {
        let result = job.is_succeeded().and_then(|()| {
            job.loaded_document().ok_or_else(|| {
                glib::Error::new(gio::IOErrorEnum::Failed, "The PDF could not be opened.")
            })
        });
        finished(result, job.password().is_some());
    });
    job.scheduler_push_job(papers::JobPriority::PriorityUrgent);
    job
}
