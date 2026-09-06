use super::{
    pdf::{self, Pdf},
    reflow::{self, Reflow},
    worker::StateWorker,
};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use papers_document::prelude::*;
use papers_view::prelude::*;
use readero::{document::*, resource::Publication};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[path = "probe.rs"]
#[cfg(feature = "smoke")]
mod probe;
#[path = "smoke.rs"]
#[cfg(feature = "smoke")]
mod smoke;

enum Surface {
    Pdf(Rc<Pdf>),
    Reflow(Reflow),
}
#[derive(Clone)]
enum Destination {
    Locator(Locator),
    Href(String),
    PdfResult(papers_view::SearchResult),
    PdfLink(papers_document::Link),
}

#[derive(Clone)]
struct NavigationItem {
    label: String,
    depth: u32,
    target: Destination,
}
#[derive(Clone, PartialEq)]
struct HistoryEntry {
    path: PathBuf,
    locator: Locator,
    settings: Settings,
}

pub struct Shell {
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    ui: gtk::Builder,
    worker: StateWorker,
    generation: Cell<u64>,
    open_serial: Cell<u64>,
    opening: Cell<bool>,
    pending_reader: RefCell<Option<(DocumentRecord, Reflow)>>,
    pending_history: RefCell<Option<(Vec<HistoryEntry>, Vec<HistoryEntry>)>>,
    unsaved: RefCell<std::collections::HashMap<PathBuf, (u64, DocumentRecord)>>,
    save_serial: Cell<u64>,
    gate: RefCell<SaveGate>,
    current: RefCell<Option<DocumentRecord>>,
    surface: RefCell<Option<Surface>>,
    job: RefCell<Option<papers_view::JobLoad>>,
    toc: RefCell<Vec<NavigationItem>>,
    search_index: Cell<i32>,
    back: RefCell<Vec<HistoryEntry>>,
    forward: RefCell<Vec<HistoryEntry>>,
    pending_location: RefCell<Option<HistoryEntry>>,
    pending_sidebar: Cell<Option<bool>>,
    pending_href: RefCell<Option<String>>,
    save_timer: RefCell<Option<glib::SourceId>>,
    dirty_since: Cell<Option<Instant>>,
    focus: Cell<bool>,
    sidebar_before_focus: Cell<bool>,
    updating: Cell<bool>,
    closing: Cell<bool>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    reload_timer: RefCell<Option<glib::SourceId>>,
    reload_serial: Cell<u64>,
}

mod controls;
mod home;
mod navigation;
mod persistence;

impl Shell {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let ui = gtk::Builder::from_string(include_str!("../../assets/window.ui"));
        let window: adw::ApplicationWindow = ui.object("window").expect("window.ui defines window");
        window.set_application(Some(app));
        let data = std::env::var_os("READERO_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| glib::user_data_dir().join("readero"));
        let toasts = adw::ToastOverlay::new();
        let content = window.content();
        window.set_content(gtk::Widget::NONE);
        toasts.set_child(content.as_ref());
        window.set_content(Some(&toasts));
        let shell = Rc::new(Self {
            window,
            toasts,
            ui,
            worker: StateWorker::start(data.join("reading.sqlite3")),
            generation: Cell::new(0),
            open_serial: Cell::new(0),
            opening: Cell::new(false),
            pending_reader: RefCell::new(None),
            pending_history: RefCell::new(None),
            unsaved: RefCell::new(std::collections::HashMap::new()),
            save_serial: Cell::new(0),
            gate: RefCell::new(SaveGate::default()),
            current: RefCell::new(None),
            surface: RefCell::new(None),
            job: RefCell::new(None),
            toc: RefCell::new(Vec::new()),
            search_index: Cell::new(-1),
            back: RefCell::new(Vec::new()),
            forward: RefCell::new(Vec::new()),
            pending_location: RefCell::new(None),
            pending_sidebar: Cell::new(None),
            pending_href: RefCell::new(None),
            save_timer: RefCell::new(None),
            dirty_since: Cell::new(None),
            focus: Cell::new(false),
            sidebar_before_focus: Cell::new(false),
            updating: Cell::new(false),
            closing: Cell::new(false),
            monitor: RefCell::new(None),
            reload_timer: RefCell::new(None),
            reload_serial: Cell::new(0),
        });
        shell
            .object::<gtk::MenuButton>("appearance_button")
            .update_property(&[gtk::accessible::Property::Label("Reading appearance")]);
        shell.wire();
        shell.home();
        shell
    }
    fn object<T: glib::object::IsA<glib::Object>>(&self, id: &str) -> T {
        self.ui
            .object(id)
            .unwrap_or_else(|| panic!("window.ui defines {id}"))
    }

    fn button(&self, id: &str) -> gtk::Button {
        self.object(id)
    }

    fn label(&self, id: &str) -> gtk::Label {
        self.object(id)
    }

    fn content(&self) -> gtk::Stack {
        self.object("content")
    }

    fn sidebar(&self) -> gtk::Revealer {
        self.object("sidebar_revealer")
    }

    fn list(&self) -> gtk::ListBox {
        self.object("navigation_list")
    }

    fn entry(&self) -> gtk::SearchEntry {
        self.object("search_entry")
    }

    fn connect(self: &Rc<Self>, id: &str, action: impl Fn(&Rc<Self>) + 'static) {
        let weak = Rc::downgrade(self);
        self.button(id).connect_clicked(move |_| {
            if let Some(shell) = weak.upgrade() {
                action(&shell);
            }
        });
    }

    pub fn open(self: &Rc<Self>, path: PathBuf) {
        self.open_document(path);
    }

    fn reload(self: &Rc<Self>, path: PathBuf) {
        if self.opening.get() {
            // Keep the change queued until the candidate either commits (and
            // disposes this timer) or fails/cancels, restoring this reader.
            self.schedule_reload(path);
            return;
        }
        let generation = self.generation.get();
        let serial = self.reload_serial.get() + 1;
        self.reload_serial.set(serial);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            // Validate first. Empty, missing or partly written files leave the
            // current reader, save gate and directory watcher usable.
            let input = path.clone();
            let result =
                gio::spawn_blocking(move || Publication::open(&input, Format::Markdown)).await;
            let Some(s) = weak.upgrade() else {
                return;
            };
            if !s.is_current(generation) || s.reload_serial.get() != serial {
                return;
            }
            // A candidate may have started during Publication::open. Keep the
            // current reader's update pending until that candidate resolves.
            if s.opening.get() {
                s.schedule_reload(path);
                return;
            }
            let publication = match result {
                Ok(Ok(publication)) => publication,
                Ok(Err(error)) => {
                    s.toast(&format!("Couldn’t update this document. {error}"));
                    return;
                }
                Err(_) => {
                    s.toast("The document worker stopped. Save the file again to retry.");
                    return;
                }
            };
            // Capture the latest session state after the asynchronous read.
            let Some(record) = s.record_for_save().or_else(|| s.current.borrow().clone()) else {
                return;
            };
            s.cancel_open();
            s.save();
            let sidebar = s.sidebar().reveals_child();
            s.dispose_reader();
            let generation = s.gate.borrow_mut().begin();
            s.generation.set(generation);
            s.current.replace(Some(record.clone()));
            s.pending_sidebar.set(Some(sidebar));
            s.toc.borrow_mut().clear();
            s.set_reading(false);
            s.mount_reflow(publication, record, generation);
        });
    }

    fn open_document(self: &Rc<Self>, path: PathBuf) {
        self.cancel_open();
        let serial = self.open_serial.get();
        if let Err(error) = Format::from_path(&path) {
            self.open_error(&error.to_string());
            return;
        }
        self.opening.set(true);
        self.save();
        if self.surface.borrow().is_none() {
            self.show_status("Opening your document…", "", true);
        }
        let weak = Rc::downgrade(self);
        let input = path.clone();
        let pending = self.worker.call(move |store| store.document(&input));
        glib::MainContext::default().spawn_local(async move {
            let loaded = pending.await;
            let Some(shell) = weak.upgrade() else { return; };
            if !shell.is_open(serial) { return; }
            let mut record = match loaded {
                Ok(record) => record,
                Err(error) if path.is_file() => {
                    match DocumentRecord::new(path) {
                        Ok(record) => {
                            shell.storage_error(&error.to_string());
                            record
                        },
                        Err(error) => { shell.open_error(&error.to_string()); return; }
                    }
                }
                Err(_) => {
                    shell.open_error("This document could not be found. Return to reading home to locate its new location.");
                    return;
                }
            };
            if let Some((_, unsaved)) = shell.unsaved.borrow().get(&record.path) {
                record.locator = unsaved.locator.clone();
                record.settings = unsaved.settings.clone();
            }
            if let Some(target)=shell.pending_location.borrow_mut().take() && target.path==record.path {
                record.locator=Some(target.locator);record.settings=target.settings;
            }
            if record.format == Format::Pdf {
                shell.load_pdf(record, serial);
                return;
            }
            let path = record.path.clone();
            let format = record.format;
            let result = gio::spawn_blocking(move || Publication::open(&path, format)).await;
            if !shell.is_open(serial) { return; }
            match result {
                Ok(Ok(publication)) => shell.stage_reflow(publication, record, serial),
                Ok(Err(error)) => shell.open_error(&error.to_string()),
                Err(_) => shell.open_error("The document worker stopped. Try opening the file again."),
            }
        });
    }

    fn load_pdf(self: &Rc<Self>, record: DocumentRecord, serial: u64) {
        let weak = Rc::downgrade(self);
        let path = record.path.clone();
        let job = pdf::load(&path, move |result, supplied_password| {
            let Some(s) = weak.upgrade() else {
                return;
            };
            if !s.is_open(serial) {
                return;
            }
            match result {
                Ok(doc) => {
                    if doc.n_pages() <= 0 {
                        s.open_error("This PDF has no readable pages.");
                        return;
                    }
                    s.job.borrow_mut().take();
                    let generation = s.commit_open(record.clone());
                    let weak = Rc::downgrade(&s);
                    let pdf = Pdf::new(&doc, &record.settings, move |locator, label| {
                        if let Some(s) = weak.upgrade()
                            && s.is_current(generation)
                        {
                            s.position(locator, &label);
                        }
                    });
                    s.content().add_named(&pdf.scroll, Some("reader"));
                    s.content().set_visible_child_name("reader");
                    s.surface.replace(Some(Surface::Pdf(Rc::clone(&pdf))));
                    let title = doc
                        .title()
                        .filter(|title| !title.trim().is_empty())
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| record.title.clone());
                    let weak = Rc::downgrade(&s);
                    pdf.search.connect_finished(move |_, count| {
                        if let Some(s) = weak.upgrade()
                            && s.is_current(generation)
                        {
                            s.pdf_search_results(count);
                        }
                    });
                    s.ready(generation, &title);
                    pdf.restore(record.locator.clone());
                    let weak = Rc::downgrade(&s);
                    let weak_pdf = Rc::downgrade(&pdf);
                    pdf.view.connect_handle_link(move |_, _, _| {
                        if let Some(s) = weak.upgrade()
                            && s.is_current(generation)
                            && let Some(pdf) = weak_pdf.upgrade()
                            && !pdf.is_restoring()
                            && let Some(locator) = pdf.stable_location()
                        {
                            s.push_history(locator);
                            s.sync_controls();
                        }
                    });
                    let weak = Rc::downgrade(&s);
                    pdf.view.connect_external_link(move |_, action| {
                        if let Some(s) = weak.upgrade()
                            && let Some(uri) = action.uri()
                        {
                            s.external(&uri);
                        }
                    });
                    // Papers' outline job assumes a non-null links model.
                    // PDFs without an outline must not enqueue that job.
                    if !doc
                        .dynamic_cast_ref::<papers_document::DocumentLinks>()
                        .is_some_and(|links| links.has_document_links())
                    {
                        return;
                    }
                    let outlines = papers_view::JobLinks::new(&doc);
                    let weak = Rc::downgrade(&s);
                    outlines.connect_finished(move |job| {
                        let Some(s) = weak.upgrade() else {
                            return;
                        };
                        if !s.is_current(generation) {
                            return;
                        }
                        if let Some(model) = job.model() {
                            let mut items = Vec::new();
                            collect_outline(&model, 0, &mut items);
                            s.toc.replace(items);
                            s.refresh_sidebar();
                        }
                    });
                    outlines.scheduler_push_job(papers_view::JobPriority::PriorityLow);
                }
                Err(error) if error.matches(papers_document::DocumentError::Encrypted) => {
                    s.password(serial, supplied_password)
                }
                Err(error) => s.open_error(&error.to_string()),
            }
        });
        self.job.replace(Some(job));
    }

    // Opening has its own serial: the current reader keeps accepting position
    // updates until the candidate has successfully initialized.
    fn is_open(&self, serial: u64) -> bool {
        self.open_serial.get() == serial && !self.closing.get()
    }

    fn cancel_open(&self) {
        self.opening.set(false);
        self.open_serial.set(self.open_serial.get() + 1);
        if let Some(job) = self.job.borrow_mut().take() {
            job.cancel();
        }
        if let Some((_, reader)) = self.pending_reader.borrow_mut().take() {
            reader.close();
            self.content().remove(&reader.view);
            if self.surface.borrow().is_some() {
                self.content().set_visible_child_name("reader");
            }
        }
        self.pending_location.take();
        self.pending_href.take();
        self.pending_history.take();
    }

    fn open_error(&self, message: &str) {
        self.cancel_open();
        if self.surface.borrow().is_some() {
            self.content().set_visible_child_name("reader");
            self.toast(&format!("Couldn’t open this document. {message}"));
        } else {
            self.error(message);
        }
    }

    fn commit_open(self: &Rc<Self>, record: DocumentRecord) -> u64 {
        self.opening.set(false);
        self.save();
        if self.focus.get() {
            self.toggle_focus();
        }
        self.dispose_surface();
        let generation = self.gate.borrow_mut().begin();
        self.generation.set(generation);
        self.current.replace(Some(record));
        self.pending_sidebar.set(None);
        if let Some((back, forward)) = self.pending_history.take() {
            self.back.replace(back);
            self.forward.replace(forward);
        } else {
            self.back.borrow_mut().clear();
            self.forward.borrow_mut().clear();
        }
        self.toc.borrow_mut().clear();
        self.set_reading(false);
        generation
    }

    fn stage_reflow(
        self: &Rc<Self>,
        publication: Publication,
        mut record: DocumentRecord,
        serial: u64,
    ) {
        let changed = !record.revision.is_empty() && record.revision != publication.revision;
        record.revision = publication.revision.clone();
        let committed = Cell::new(None);
        let notices = RefCell::new(Vec::<String>::new());
        let weak = Rc::downgrade(self);
        let reader = Reflow::new(publication, &record, serial, changed, move |mut message| {
            let Some(s) = weak.upgrade() else {
                return;
            };
            if let Some(generation) = committed.get() {
                message.generation = generation;
                s.web_message(message);
                return;
            }
            if !s.is_open(serial) {
                return;
            }
            match &message.event {
                reflow::Event::Ready { .. } => {
                    let pending = s.pending_reader.borrow_mut().take();
                    let Some((record, reader)) = pending else {
                        return;
                    };
                    let generation = s.commit_open(record);
                    committed.set(Some(generation));
                    s.content().remove(&reader.view);
                    s.content().add_named(&reader.view, Some("reader"));
                    s.content().set_visible_child_name("reader");
                    s.surface.replace(Some(Surface::Reflow(reader)));
                    message.generation = generation;
                    s.web_message(message);
                    for message in notices.take() {
                        s.toast(&message);
                    }
                }
                reflow::Event::Error { message } => s.open_error(message),
                reflow::Event::Notice { message } => notices.borrow_mut().push(message.clone()),
                _ => {}
            }
        });
        self.content().add_named(&reader.view, Some("candidate"));
        self.pending_reader.replace(Some((record, reader)));
        // Map the candidate so WebKit can finish real layout before committing.
        self.content().set_visible_child_name("candidate");
    }

    fn mount_reflow(
        self: &Rc<Self>,
        publication: Publication,
        record: DocumentRecord,
        generation: u64,
    ) {
        let changed = !record.revision.is_empty() && record.revision != publication.revision;
        if let Some(current) = self.current.borrow_mut().as_mut() {
            current.revision = publication.revision.clone();
        }
        let weak = Rc::downgrade(self);
        let reflow = Reflow::new(publication, &record, generation, changed, move |message| {
            if let Some(s) = weak.upgrade() {
                s.web_message(message);
            }
        });
        self.content().add_named(&reflow.view, Some("reader"));
        self.content().set_visible_child_name("reader");
        self.surface.replace(Some(Surface::Reflow(reflow)));
    }

    fn ready(self: &Rc<Self>, generation: u64, title: &str) {
        if !self.is_current(generation) {
            return;
        }
        self.gate.borrow_mut().ready(generation);
        if let Some(record) = self.current.borrow_mut().as_mut() {
            record.title = title.to_owned();
            record.last_opened = now();
        }
        self.label("document_title").set_text(title);
        self.window.set_title(Some(&format!("{title} — Readero")));
        self.set_reading(true);
        self.sync_controls();
        self.refresh_sidebar();
        if let Some(visible) = self.pending_sidebar.take() {
            self.sidebar()
                .set_reveal_child(visible && !self.focus.get());
        }
        self.watch_markdown();
        self.save();
    }

    fn web_message(self: &Rc<Self>, message: reflow::Message) {
        if !self.is_current(message.generation) {
            return;
        }
        match message.event {
            reflow::Event::Ready {
                title,
                toc,
                locator,
                total,
            } => {
                self.toc.replace(
                    toc.into_iter()
                        .map(|item| NavigationItem {
                            label: item.label,
                            depth: item.depth,
                            target: Destination::Href(item.href),
                        })
                        .collect(),
                );
                self.ready(message.generation, &title);
                if let Some(locator) = locator {
                    let section = match &locator.anchor {
                        Anchor::Reflow { section, .. } => section + 1,
                        _ => 1,
                    };
                    let label = reflow_label(&locator, section, total);
                    self.position(locator, &label);
                }
                let href = self.pending_href.borrow_mut().take();
                if let Some(href) = href {
                    self.goto(Destination::Href(href), false);
                }
            }
            reflow::Event::Location {
                locator,
                section,
                total,
            } => {
                let label = reflow_label(&locator, section, total);
                self.position(locator, &label);
            }
            reflow::Event::Search {
                query,
                items,
                complete,
            } => {
                if self.object::<gtk::DropDown>("sidebar_kind").selected() == 2
                    && self.entry().text().as_str() == query
                {
                    self.clear_navigation();
                    self.label("search_status").set_text(&format!(
                        "{}{} matches{}",
                        items.len(),
                        if items.len() == 500 { "+" } else { "" },
                        if complete { "" } else { " · searching…" }
                    ));
                    for item in items {
                        self.navigation_row(&item.label, Destination::Locator(item.locator), 0);
                    }
                }
            }
            reflow::Event::Jump { locator } => {
                if let Some(locator) = locator {
                    self.push_history(locator);
                    self.sync_controls();
                }
            }
            reflow::Event::External { href } => self.external(&href),
            reflow::Event::Related { href } => self.related(&href),
            reflow::Event::Notice { message } => self.toast(&message),
            reflow::Event::Escape { handled } => {
                if !handled {
                    self.escape_chrome();
                }
            }
            reflow::Event::Error { message } => {
                if self.gate.borrow().accepts(self.generation.get()) {
                    self.toast(&message);
                } else {
                    self.error(&message);
                }
            }
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.gate.borrow().current(generation) && !self.closing.get()
    }

    fn dispose_surface(&self) {
        self.reload_serial.set(self.reload_serial.get() + 1);
        if let Some(monitor) = self.monitor.borrow_mut().take() {
            monitor.cancel();
        }
        if let Some(timer) = self.reload_timer.borrow_mut().take() {
            timer.remove();
        }
        self.dispose_reader();
    }

    fn dispose_reader(&self) {
        if let Some(job) = self.job.borrow_mut().take() {
            job.cancel();
        }
        let surface = self.surface.borrow_mut().take();
        if let Some(surface) = &surface {
            match surface {
                Surface::Pdf(pdf) => pdf.cancel(),
                Surface::Reflow(reflow) => reflow.close(),
            }
        }
        if let Some(widget) = self.content().child_by_name("reader") {
            self.content().remove(&widget);
        }
        drop(surface);
    }

    fn open_dialog(self: &Rc<Self>, relink: Option<String>) {
        let dialog = gtk::FileDialog::builder()
            .title(if relink.is_some() {
                "Locate your document"
            } else {
                "Open a document"
            })
            .modal(true)
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Reading documents"));
        for suffix in ["pdf", "epub", "md", "markdown"] {
            filter.add_suffix(suffix);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        filters.append(&all);
        dialog.set_filters(Some(&filters));
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(s), Ok(file)) = (weak.upgrade(), result)
                && let Some(path) = file.path()
            {
                if let Some(id) = relink {
                    let worker = s.worker.clone();
                    let mut snapshots = s.unsaved.borrow().values().cloned().collect::<Vec<_>>();
                    snapshots.sort_by_key(|(serial, _)| *serial);
                    let snapshots = snapshots
                        .into_iter()
                        .map(|(_, record)| record)
                        .collect::<Vec<_>>();
                    glib::MainContext::default().spawn_local(async move {
                        let p = path.clone();
                        match worker
                            .call(move |store| store.relink_sessions(&id, &p, &snapshots))
                            .await
                        {
                            Ok((record, session_ids)) => {
                                let mut unsaved = s.unsaved.borrow_mut();
                                let mut rebound = Vec::new();
                                unsaved.retain(|_, (serial, snapshot)| {
                                    if snapshot.id == record.id
                                        || session_ids.contains(&snapshot.id)
                                    {
                                        snapshot.id = record.id.clone();
                                        snapshot.path = record.path.clone();
                                        rebound.push((*serial, snapshot.clone()));
                                        false
                                    } else {
                                        true
                                    }
                                });
                                for snapshot in rebound {
                                    let entry = unsaved
                                        .entry(record.path.clone())
                                        .or_insert_with(|| snapshot.clone());
                                    if entry.0 < snapshot.0 {
                                        *entry = snapshot;
                                    }
                                }
                                drop(unsaved);
                                s.open(path);
                            }
                            Err(error) => s.toast(&error.to_string()),
                        }
                    });
                } else {
                    s.open(path);
                }
            }
        });
    }

    fn password(self: &Rc<Self>, serial: u64, incorrect: bool) {
        let dialog = adw::AlertDialog::builder()
            .heading("Unlock this PDF")
            .body(if incorrect {
                "That password didn’t unlock the PDF. Try again."
            } else {
                "Enter the document password. It is used only for this session."
            })
            .build();
        let entry = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Cancel"), ("unlock", "Unlock")]);
        dialog.set_default_response(Some("unlock"));
        dialog.set_close_response("cancel");
        dialog.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);
        dialog.set_focus(Some(&entry));
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if let Some(s) = weak.upgrade() {
                if !s.is_open(serial) {
                    return;
                }
                if response == "unlock" {
                    let password = entry.text().to_string();
                    entry.set_text("");
                    // Papers retains the encrypted document on its first job.
                    // A fresh job ignores its password on that initial load;
                    // retry the retained job so the security interface applies it.
                    if let Some(job) = s.job.borrow().as_ref() {
                        job.set_password(Some(&password));
                        job.scheduler_push_job(papers_view::JobPriority::PriorityUrgent);
                    }
                } else {
                    s.cancel_open();
                    if s.surface.borrow().is_none() {
                        s.home();
                    }
                }
            }
        });
        dialog.present(Some(&self.window));
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn collect_outline(model: &gio::ListModel, depth: u32, items: &mut Vec<NavigationItem>) {
    if depth > 20 {
        return;
    }
    for index in 0..model.n_items() {
        if items.len() >= 3000 {
            return;
        }
        if let Some(outline) = model
            .item(index)
            .and_downcast::<papers_document::Outlines>()
        {
            if let Some(link) = outline.link() {
                let label = outline
                    .label()
                    .or_else(|| link.title())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Untitled section".into());
                items.push(NavigationItem {
                    label,
                    depth,
                    target: Destination::PdfLink(link),
                });
            }
            if let Some(children) = outline.children() {
                collect_outline(&children, depth + 1, items);
            }
        }
    }
}
fn text(value: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .css_classes([class])
        .build()
}
fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .css_classes(["flat"])
        .build()
}

fn reflow_label(locator: &Locator, section: usize, total: usize) -> String {
    if total == 1
        && let Anchor::Reflow { fraction, .. } = locator.anchor
    {
        format!("{:.0}% read", fraction.clamp(0.0, 1.0) * 100.0)
    } else {
        format!("Section {section} of {total}")
    }
}
