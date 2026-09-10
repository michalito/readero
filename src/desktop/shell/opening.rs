use super::*;

/// Native document replacement is owned here. Callers supply complete intentions;
/// candidate resources and callback identities never escape this module.
#[derive(Default)]
pub(super) struct State {
    generation: Cell<u64>,
    ready: Cell<bool>,
    serial: Cell<u64>,
    candidate: RefCell<Option<Candidate>>,
    closing: Cell<bool>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    reload_timer: RefCell<Option<glib::SourceId>>,
    dirty: RefCell<Option<PathBuf>>,
}

struct Request {
    path: PathBuf,
    location: Option<HistoryEntry>,
    history: Option<(Vec<HistoryEntry>, Vec<HistoryEntry>)>,
    href: Option<String>,
    reload: bool,
}

impl Request {
    fn file(path: PathBuf) -> Self {
        Self {
            path,
            location: None,
            history: None,
            href: None,
            reload: false,
        }
    }
}

struct Candidate {
    request: Request,
    reader: Option<(DocumentRecord, Reflow)>,
    job: Option<papers_view::JobLoad>,
    password: Option<adw::AlertDialog>,
    sidebar: bool,
}

struct Committed {
    generation: u64,
    href: Option<String>,
    sidebar: Option<bool>,
}

impl Shell {
    pub fn open(self: &Rc<Self>, path: PathBuf) {
        self.open_document(Request::file(path));
    }

    #[cfg(feature = "smoke")]
    pub(super) fn reload(self: &Rc<Self>, path: PathBuf) {
        self.schedule_reload(path);
        if let Some(timer) = self.opening.reload_timer.take() {
            timer.remove();
        }
        self.start_reload();
    }

    fn start_reload(self: &Rc<Self>) {
        if self.is_opening() || self.opening.closing.get() {
            return;
        }
        let Some(path) = self.opening.dirty.take() else {
            return;
        };
        if !self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|r| r.path == path)
        {
            return;
        }
        self.open_document(Request {
            reload: true,
            ..Request::file(path)
        });
    }

    fn open_document(self: &Rc<Self>, request: Request) {
        if self.opening.closing.get() {
            return;
        }
        self.discard_candidate(true);
        let serial = self.opening.serial.get();
        let path = request.path.clone();
        let reload = request.reload;
        self.opening.candidate.replace(Some(Candidate {
            request,
            reader: None,
            job: None,
            password: None,
            sidebar: self.sidebar().reveals_child(),
        }));
        if let Err(error) = Format::from_path(&path) {
            self.open_error(&error.to_string());
            return;
        }
        self.save();
        if self.surface.borrow().is_none() {
            self.show_status("Opening your document…", "", true);
        }
        let weak = Rc::downgrade(self);
        // Reload uses the active reading state, captured after parsing. It must
        // not replace it with an older persisted snapshot.
        let pending = (!reload).then(|| self.reading_state.document(path.clone()));
        glib::MainContext::default().spawn_local(async move {
            let loaded = if let Some(pending) = pending { Some(pending.await) } else { None };
            let Some(shell) = weak.upgrade() else { return; };
            if !shell.is_open(serial) { return; }
            let mut record = match loaded {
                Some(Ok(opened)) => {
                    if let Some(error) = opened.storage_error { shell.storage_error(&error); }
                    opened.record
                }
                Some(Err(error)) => {
                    shell.open_error(&if path.is_file() { error.to_string() } else {
                        "This document could not be found. Return to reading home to locate its new location.".into()
                    });
                    return;
                }
                None => {
                    let Some(record) = shell.current.borrow().clone() else { return; };
                    record
                }
            };
            if let Some(candidate) = shell.opening.candidate.borrow().as_ref()
                && let Some(target) = &candidate.request.location
            {
                record.locator = Some(target.locator.clone());
                record.settings = target.settings.clone();
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
                Ok(Ok(publication)) => {
                    shell.capture_latest(&mut record);
                    shell.stage_reflow(publication, record, serial);
                }
                Ok(Err(error)) => shell.open_error(&error.to_string()),
                Err(_) => shell.open_error("The document worker stopped. Try opening the file again."),
            }
        });
    }

    fn capture_latest(&self, record: &mut DocumentRecord) {
        let explicit_location = self
            .opening
            .candidate
            .borrow()
            .as_ref()
            .is_some_and(|c| c.request.location.is_some());
        if !explicit_location
            && let Some(latest) = self.record_for_save()
            && latest.path == record.path
        {
            record.locator = latest.locator;
            record.settings = latest.settings;
        }
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
                    let mut record = record.clone();
                    s.capture_latest(&mut record);
                    if doc.n_pages() <= 0 {
                        s.open_error("This PDF has no readable pages.");
                        return;
                    }
                    if let Some(candidate) = s.opening.candidate.borrow_mut().as_mut() {
                        candidate.job.take();
                    }
                    let committed = s.commit_open(record.clone());
                    let generation = committed.generation;
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
                    s.arm_reload();
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
        if let Some(candidate) = self.opening.candidate.borrow_mut().as_mut() {
            candidate.job = Some(job);
        }
    }

    // Opening has its own serial: the current reader keeps accepting position
    // updates until the candidate has successfully initialized.
    fn is_open(&self, serial: u64) -> bool {
        self.opening.serial.get() == serial && self.is_opening() && !self.opening.closing.get()
    }

    pub(super) fn generation(&self) -> u64 {
        self.opening.generation.get()
    }

    pub(super) fn is_ready(&self) -> bool {
        self.opening.ready.get()
    }

    pub(super) fn is_opening(&self) -> bool {
        self.opening.candidate.borrow().is_some()
    }

    pub(super) fn view_stamp(&self) -> (u64, u64) {
        (self.generation(), self.opening.serial.get())
    }

    pub(super) fn is_view(&self, stamp: (u64, u64)) -> bool {
        self.view_stamp() == stamp && !self.opening.closing.get()
    }

    fn candidate_mapped(&self) -> bool {
        self.opening
            .candidate
            .borrow()
            .as_ref()
            .is_some_and(|c| c.reader.is_some())
    }

    pub(super) fn can_interact(&self) -> bool {
        !self.opening.closing.get() && !self.candidate_mapped()
    }

    pub(super) fn accepts_position(&self) -> bool {
        self.is_ready() && self.can_interact()
    }

    pub(super) fn prepare_navigation(self: &Rc<Self>) -> bool {
        if !self.can_interact() {
            return false;
        }
        if self.is_opening() {
            self.cancel_open();
        }
        true
    }

    fn advance_document(&self) -> u64 {
        let generation = self.generation() + 1;
        self.opening.generation.set(generation);
        self.opening.ready.set(false);
        generation
    }

    fn opening_controls(&self, enabled: bool) {
        for id in [
            "modes",
            "back_button",
            "forward_button",
            "appearance_button",
            "bookmark_button",
            "search_button",
            "sidebar_button",
            "focus_button",
            "page_button",
            "sidebar_revealer",
        ] {
            self.object::<gtk::Widget>(id).set_sensitive(enabled);
        }
        if enabled {
            self.sync_controls();
        }
    }

    pub(super) fn cancel_open(self: &Rc<Self>) {
        self.discard_candidate(true);
        self.arm_reload();
    }

    fn discard_candidate(&self, retry_reload: bool) {
        self.opening.serial.set(self.opening.serial.get() + 1);
        let candidate = self.opening.candidate.take();
        if let Some(candidate) = candidate {
            if retry_reload && candidate.request.reload && self.opening.dirty.borrow().is_none() {
                self.opening.dirty.replace(Some(candidate.request.path));
            }
            if let Some(job) = candidate.job {
                job.cancel();
            }
            if let Some(dialog) = candidate.password {
                dialog.close();
            }
            if let Some((_, reader)) = candidate.reader {
                reader.close();
                self.content().remove(&reader.view);
                if self.surface.borrow().is_some() {
                    self.content().set_visible_child_name("reader");
                }
            }
        }
        self.opening_controls(true);
    }

    fn open_error(self: &Rc<Self>, message: &str) {
        let reload = self
            .opening
            .candidate
            .borrow()
            .as_ref()
            .is_some_and(|c| c.request.reload);
        self.discard_candidate(false);
        if self.surface.borrow().is_some() {
            self.content().set_visible_child_name("reader");
            let action = if reload { "update" } else { "open" };
            let toast = adw::Toast::new(&format!("Couldn’t {action} this document. {message}"));
            toast.set_priority(adw::ToastPriority::High);
            self.toasts.add_toast(toast);
        } else {
            self.error(message);
        }
        self.arm_reload();
    }

    fn commit_open(self: &Rc<Self>, record: DocumentRecord) -> Committed {
        let candidate = self
            .opening
            .candidate
            .take()
            .expect("only a ready candidate can commit");
        let same_document = self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|r| r.path == record.path);
        self.save();
        if !candidate.request.reload && self.focus.get() {
            self.toggle_focus();
        }
        if same_document {
            // Keep edits observed during preparation, including an ordinary
            // reopen of the same document. The replacement may predate them.
            self.dispose_reader();
        } else {
            self.dispose_surface();
        }
        let generation = self.advance_document();
        self.current.replace(Some(record));
        if !candidate.request.reload {
            let (back, forward) = candidate.request.history.unwrap_or_default();
            self.back.replace(back);
            self.forward.replace(forward);
        }
        self.toc.borrow_mut().clear();
        self.set_reading(false);
        Committed {
            generation,
            href: candidate.request.href,
            sidebar: candidate.request.reload.then_some(candidate.sidebar),
        }
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
                    let pending = s
                        .opening
                        .candidate
                        .borrow_mut()
                        .as_mut()
                        .and_then(|c| c.reader.take());
                    let Some((record, reader)) = pending else {
                        return;
                    };
                    let result = s.commit_open(record);
                    let generation = result.generation;
                    committed.set(Some(generation));
                    s.content().remove(&reader.view);
                    s.content().add_named(&reader.view, Some("reader"));
                    s.content().set_visible_child_name("reader");
                    reader.view.set_sensitive(true);
                    s.surface.replace(Some(Surface::Reflow(reader)));
                    message.generation = generation;
                    s.web_message(message);
                    s.opening_controls(true);
                    if let Some(visible) = result.sidebar {
                        s.sidebar().set_reveal_child(visible && !s.focus.get());
                    }
                    if let Some(href) = result.href {
                        s.goto(Destination::Href(href), false);
                    }
                    s.arm_reload();
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
        reader.view.set_sensitive(false);
        if let Some(candidate) = self.opening.candidate.borrow_mut().as_mut() {
            candidate.sidebar = self.sidebar().reveals_child();
            candidate.reader = Some((record, reader));
        }
        self.opening_controls(false);
        // Map the candidate so WebKit can finish real layout before committing.
        self.content().set_visible_child_name("candidate");
    }

    fn ready(self: &Rc<Self>, generation: u64, title: &str) {
        if !self.is_current(generation) {
            return;
        }
        self.opening.ready.set(true);
        if let Some(record) = self.current.borrow_mut().as_mut() {
            record.title = title.to_owned();
            record.last_opened = now();
        }
        self.label("document_title").set_text(title);
        self.window.set_title(Some(&format!("{title} — Readero")));
        self.set_reading(true);
        self.sync_controls();
        self.refresh_sidebar();
        self.watch_markdown();
        self.save();
    }

    pub(super) fn web_message(self: &Rc<Self>, message: reflow::Message) {
        if !self.is_current(message.generation) || self.candidate_mapped() {
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
                if self.is_ready() {
                    self.toast(&message);
                } else {
                    self.error(&message);
                }
            }
        }
    }

    pub(super) fn is_current(&self, generation: u64) -> bool {
        self.generation() == generation && !self.opening.closing.get()
    }

    pub(super) fn dispose_surface(&self) {
        if let Some(monitor) = self.opening.monitor.take() {
            monitor.cancel();
        }
        if let Some(timer) = self.opening.reload_timer.take() {
            timer.remove();
        }
        self.opening.dirty.take();
        self.dispose_reader();
        self.advance_document();
    }

    fn dispose_reader(&self) {
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
                    let job = s
                        .opening
                        .candidate
                        .borrow()
                        .as_ref()
                        .and_then(|c| c.job.clone());
                    if let Some(job) = job {
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
        if let Some(candidate) = self.opening.candidate.borrow_mut().as_mut() {
            candidate.password = Some(dialog.clone());
        }
        dialog.present(Some(&self.window));
    }
    pub(super) fn home(self: &Rc<Self>) {
        if self.opening.closing.get() {
            return;
        }
        self.discard_candidate(false);
        self.save();
        self.dispose_surface();
        self.current.replace(None);
        let generation = self.generation();
        if self.focus.get() {
            self.focus.set(false);
            self.object::<adw::HeaderBar>("header").set_visible(true);
        }
        self.set_reading(false);
        self.window.set_title(Some("Readero"));
        self.render_home(&[]);
        let serial = self.opening.serial.get();
        let weak = Rc::downgrade(self);
        let pending = self.reading_state.recents();
        glib::MainContext::default().spawn_local(async move {
            let result = pending.await;
            let Some(s) = weak.upgrade() else {
                return;
            };
            if !s.is_view((generation, serial)) {
                return;
            }
            match result {
                Ok(recents) => s.render_home(&recents),
                Err(error) => s.storage_error(&error.to_string()),
            }
        });
    }

    pub(super) fn push_history(self: &Rc<Self>, locator: Locator) {
        if !self.prepare_navigation() {
            return;
        }
        let Some(record) = self.current.borrow().clone() else {
            return;
        };
        let entry = HistoryEntry {
            path: record.path,
            locator,
            settings: record.settings,
        };
        let mut back = self.back.borrow_mut();
        if back.last() != Some(&entry) {
            back.push(entry);
            if back.len() > 100 {
                back.remove(0);
            }
        }
        self.forward.borrow_mut().clear();
    }

    pub(super) fn history(self: &Rc<Self>, forward: bool) {
        if !self.prepare_navigation() {
            return;
        }
        let mut back = self.back.borrow().clone();
        let mut ahead = self.forward.borrow().clone();
        let target = if forward { ahead.pop() } else { back.pop() };
        let Some(target) = target else {
            return;
        };
        if let Some(record) = self.record_for_save()
            && let Some(locator) = record.locator
        {
            let current = HistoryEntry {
                path: record.path,
                locator,
                settings: record.settings,
            };
            if forward {
                back.push(current);
            } else {
                ahead.push(current);
            }
        }
        if self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|r| r.path == target.path)
        {
            self.back.replace(back);
            self.forward.replace(ahead);
            if self
                .current
                .borrow()
                .as_ref()
                .is_some_and(|r| r.settings != target.settings)
            {
                self.change_settings(|settings| *settings = target.settings.clone());
            }
            self.goto(Destination::Locator(target.locator), false);
        } else {
            self.open_document(Request {
                location: Some(target.clone()),
                history: Some((back, ahead)),
                ..Request::file(target.path)
            });
        }
    }

    pub(super) fn related(self: &Rc<Self>, href: &str) {
        if !self.can_interact() {
            return;
        }
        let path = self
            .current
            .borrow()
            .as_ref()
            .and_then(|r| r.path.parent().map(|root| root.to_path_buf()));
        if let Some(root) = path
            && let Ok(relative) =
                readero::resource::resource_path(href.split('#').next().unwrap_or(href))
        {
            let path = root.join(relative);
            if path.canonicalize().is_ok_and(|p| p.starts_with(&root)) {
                let mut back = self.back.borrow().clone();
                if let Some(record) = self.record_for_save()
                    && let Some(locator) = record.locator
                {
                    back.push(HistoryEntry {
                        path: record.path,
                        locator,
                        settings: record.settings,
                    });
                }
                self.open_document(Request {
                    href: href
                        .split_once('#')
                        .map(|(_, fragment)| format!("content.xhtml#{fragment}")),
                    history: Some((back, Vec::new())),
                    ..Request::file(path)
                });
            }
        }
    }
    pub(super) fn watch_markdown(self: &Rc<Self>) {
        if self.opening.monitor.borrow().is_some() {
            return;
        }
        let path = self
            .current
            .borrow()
            .as_ref()
            .filter(|r| r.format == Format::Markdown)
            .map(|r| r.path.clone());
        let Some(path) = path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        // Watching the directory survives atomic replacement and deletion of
        // the source, including replacements that cannot yet be rendered.
        if let Ok(monitor) = gio::File::for_path(parent)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        {
            let weak = Rc::downgrade(self);
            monitor.connect_changed(move |_, file, other, event| {
                if file.path().as_ref() != Some(&path)
                    && other.and_then(gio::File::path).as_ref() != Some(&path)
                {
                    return;
                }
                if matches!(
                    event,
                    gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::Changed
                        | gio::FileMonitorEvent::Renamed
                        | gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::Deleted
                        | gio::FileMonitorEvent::MovedIn
                        | gio::FileMonitorEvent::MovedOut
                ) && let Some(s) = weak.upgrade()
                {
                    s.schedule_reload(path.clone());
                }
            });
            self.opening.monitor.replace(Some(monitor));
        }
    }

    pub(super) fn schedule_reload(self: &Rc<Self>, path: PathBuf) {
        if !self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|r| r.path == path && r.format == Format::Markdown)
        {
            return;
        }
        self.opening.dirty.replace(Some(path));
        if let Some(timer) = self.opening.reload_timer.take() {
            timer.remove();
        }
        self.arm_reload();
    }

    fn arm_reload(self: &Rc<Self>) {
        if self.is_opening()
            || self.opening.closing.get()
            || self.opening.dirty.borrow().is_none()
            || self.opening.reload_timer.borrow().is_some()
        {
            return;
        }
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(Duration::from_millis(450), move || {
            if let Some(s) = weak.upgrade() {
                s.opening.reload_timer.take();
                s.start_reload();
            }
        });
        self.opening.reload_timer.replace(Some(timer));
    }

    #[cfg(feature = "smoke")]
    pub(super) fn reload_pending(&self) -> bool {
        self.opening.dirty.borrow().is_some()
    }

    #[cfg(feature = "smoke")]
    pub(super) fn watching_source(&self) -> bool {
        self.opening
            .monitor
            .borrow()
            .as_ref()
            .is_some_and(|m| !m.is_cancelled())
    }

    pub(super) fn close(self: &Rc<Self>) {
        if self.opening.closing.replace(true) {
            return;
        }
        let mut record = self.record_for_save();
        let checkpoint = match self.surface.borrow().as_ref() {
            Some(Surface::Reflow(reader)) if record.is_some() => Some(reader.checkpoint()),
            _ => None,
        };
        if let Some(timer) = self.save_timer.borrow_mut().take() {
            timer.remove();
        }
        self.cancel_open();
        let shell = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                if let Some(checkpoint) = checkpoint
                    && let Some(locator) = checkpoint.await?
                    && let Some(record) = record.as_mut()
                {
                    // Position notifications are suspended during Close. Keep
                    // the checkpoint in the live record as well as the save,
                    // so a failed save cannot make retry/reload use an old place.
                    record.locator = Some(locator.clone());
                    if let Some(current) = shell.current.borrow_mut().as_mut() {
                        current.locator = Some(locator);
                    }
                }
                shell.reading_state.save(record).await
            }.await;
            if let Err(error) = result {
                shell.storage_error(&error.to_string());
                let dialog = adw::AlertDialog::builder()
                    .heading("Your latest position wasn’t saved")
                    .body("You can keep reading and retry, or close using the last successfully saved position.")
                    .build();
                dialog.add_responses(&[("stay", "Keep open"), ("close", "Close anyway")]);
                dialog.set_close_response("stay");
                let weak = Rc::downgrade(&shell);
                dialog.connect_response(None, move |_, response| {
                    if let Some(s) = weak.upgrade() {
                        if response == "close" {
                            s.finish_close();
                        } else {
                            s.opening.closing.set(false);
                            s.opening_controls(true);
                            if let Some(Surface::Reflow(reader)) = s.surface.borrow().as_ref() {
                                reader.command(serde_json::json!({"type":"snapshot"}));
                            }
                            s.arm_reload();
                        }
                    }
                });
                dialog.present(Some(&shell.window));
            } else {
                shell.finish_close();
            }
        });
    }

    fn finish_close(&self) {
        self.opening.closing.set(true);
        self.discard_candidate(false);
        self.dispose_surface();
        self.window.destroy();
    }
}
