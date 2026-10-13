use super::*;
use readero::notebook::{Notebook, Reference};
mod editing;
mod passages;

// Run after GTK's visible layout/paint, before its incremental offscreen text
// validation. Default-idle priority can starve for seconds on a large buffer.
pub(in crate::desktop::shell) fn decoration_priority() -> glib::Priority {
    glib::Priority::from(gtk::ffi::GTK_TEXT_VIEW_PRIORITY_VALIDATE - 1)
}

#[derive(PartialEq)]
struct ActiveConnections {
    section: String,
    passages: Vec<(String, String)>,
}

struct SourceButton {
    widget: gtk::Button,
    passages: Rc<RefCell<Vec<String>>>,
}

/// The live buffer survives reading navigation and source reloads. Only changing
/// documents replaces it; the state worker retains failed writes independently.
pub(super) struct Pane {
    pub(super) split: gtk::Paned,
    pub(super) panel: gtk::Box,
    pub(super) editor: gtk::TextView,
    pub(super) scroll: gtk::ScrolledWindow,
    pub(super) status: gtk::Label,
    location: gtk::Label,
    attach: gtk::Button,
    capture: gtk::Button,
    connect: gtk::Button,
    connection_bar: gtk::Box,
    connection_scroll: gtk::ScrolledWindow,
    passage_request: Cell<u64>,
    gutter: gtk::Fixed,
    actions: gio::SimpleActionGroup,
    pub(super) history: RefCell<readero::notebook::History>,
    decorations: RefCell<Option<glib::SourceId>>,
    tags_dirty: Cell<bool>,
    tags_present: Cell<bool>,
    tag_start: gtk::TextMark,
    tag_end: gtk::TextMark,
    source_buttons: RefCell<std::collections::HashMap<String, SourceButton>>,
    active_connections: RefCell<Option<ActiveConnections>>,
    pending_passage: RefCell<Option<String>>,
    selection_serial: Cell<u64>,
    capture_pending: Cell<bool>,
    selection_available: Cell<bool>,
    pending_relink: RefCell<Option<String>>,
    unresolved: RefCell<std::collections::HashSet<String>>,
    pub(super) retry: gtk::Button,
    close: gtk::Button,
    pub(super) references: gtk::ListBox,
    reference_scroll: gtk::ScrolledWindow,
    record: RefCell<Option<DocumentRecord>>,
    pub(super) draft: RefCell<Option<Notebook>>,
    dirty: Cell<bool>,
    saving: Cell<usize>,
    suppress: Cell<bool>,
    serial: Cell<u64>,
    size: Cell<(i32, i32)>,
    restore_scroll: Cell<bool>,
    restore_idle: RefCell<Option<glib::SourceId>>,
    scroll_mark: gtk::TextMark,
    visibility_intent: Cell<Option<bool>>,
}

impl Pane {
    #[cfg(feature = "smoke")]
    pub(super) fn capture_probe(&self) -> (u64, bool, bool, bool) {
        (
            self.selection_serial.get(),
            self.selection_available.get(),
            self.capture.is_sensitive(),
            self.capture_pending.get(),
        )
    }

    pub(super) fn new(split: gtk::Paned) -> Self {
        split.set_wide_handle(true);
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        panel.add_css_class("notebook");
        panel.set_visible(false);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = text("Notebook", "heading");
        title.set_hexpand(true);
        let close = icon_button("window-close-symbolic", "Hide notebook (Ctrl+Shift+N)");
        heading.append(&title);
        let connect = gtk::Button::with_label("Connect…");
        connect.set_tooltip_text(Some("Connect selected notes to a saved passage"));
        connect.set_focus_on_click(false);
        heading.append(&connect);
        heading.append(&close);
        panel.append(&heading);
        let location = text("Waiting for a reading position…", "dim-label");
        location.set_ellipsize(gtk::pango::EllipsizeMode::End);
        location.set_max_width_chars(32);
        panel.append(&location);
        let attach = gtk::Button::with_label("Save current location");
        attach.set_halign(gtk::Align::Start);
        attach.set_tooltip_text(Some("Save a location without capturing selected text"));

        let capture = gtk::Button::with_label("Save selected passage");
        capture.set_halign(gtk::Align::Start);
        capture.set_focus_on_click(false);
        capture.set_sensitive(false);
        let source_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        source_actions.add_css_class("linked");
        source_actions.set_halign(gtk::Align::Start);
        source_actions.append(&capture);
        let other_sources = gtk::MenuButton::builder()
            .icon_name("pan-down-symbolic")
            .tooltip_text("Other source actions")
            .build();
        other_sources.update_property(&[gtk::accessible::Property::Label("Other source actions")]);
        let source_menu = gtk::Popover::builder().child(&attach).build();
        let weak_menu = source_menu.downgrade();
        attach.connect_clicked(move |_| {
            if let Some(menu) = weak_menu.upgrade() {
                menu.popdown();
            }
        });
        other_sources.set_popover(Some(&source_menu));
        source_actions.append(&other_sources);
        panel.append(&source_actions);
        let editor = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(40)
            .right_margin(12)
            .top_margin(12)
            .bottom_margin(12)
            .accepts_tab(false)
            .vexpand(true)
            .editable(false)
            .build();
        editor.update_property(&[gtk::accessible::Property::Label(
            "Document notebook, plain text",
        )]);
        editor.buffer().set_enable_undo(false);
        editor.buffer().create_tag(
            Some("passage-connection"),
            &[("underline", &gtk::pango::Underline::Single)],
        );
        let scroll_mark = editor.buffer().create_mark(
            Some("notebook-scroll"),
            &editor.buffer().start_iter(),
            true,
        );
        let tag_start = editor
            .buffer()
            .create_mark(None, &editor.buffer().start_iter(), true);
        let tag_end = editor
            .buffer()
            .create_mark(None, &editor.buffer().start_iter(), false);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .min_content_height(60)
            .child(&editor)
            .build();
        scroll.add_css_class("card");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scroll));
        let gutter = gtk::Fixed::new();
        gutter.set_halign(gtk::Align::Start);
        gutter.set_width_request(36);
        overlay.add_overlay(&gutter);
        panel.append(&overlay);
        let connection_bar = gtk::Box::new(gtk::Orientation::Vertical, 3);
        let connection_scroll = gtk::ScrolledWindow::builder()
            .child(&connection_bar)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(80)
            .propagate_natural_height(true)
            .visible(false)
            .build();
        panel.append(&connection_scroll);
        let references = gtk::ListBox::new();
        references.set_selection_mode(gtk::SelectionMode::Single);
        references.add_css_class("navigation-list");
        let links = gtk::ScrolledWindow::builder()
            .child(&references)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(120)
            .propagate_natural_height(true)
            .build();
        panel.append(&links);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let status = text("Loading notebook…", "caption");
        status.set_hexpand(true);
        status.set_wrap(true);
        let retry = gtk::Button::with_label("Retry");
        retry.set_visible(false);
        footer.append(&status);
        footer.append(&retry);
        panel.append(&footer);
        split.set_end_child(Some(&panel));
        Self {
            split,
            panel,
            editor,
            scroll,
            status,
            location,
            attach,
            capture,
            connect,
            connection_bar,
            connection_scroll,
            passage_request: Cell::new(0),
            gutter,
            actions: gio::SimpleActionGroup::new(),
            history: RefCell::new(readero::notebook::History::default()),
            decorations: RefCell::new(None),
            tags_dirty: Cell::new(true),
            tags_present: Cell::new(false),
            tag_start,
            tag_end,
            source_buttons: RefCell::new(std::collections::HashMap::new()),
            active_connections: RefCell::new(None),
            pending_passage: RefCell::new(None),
            selection_serial: Cell::new(0),
            capture_pending: Cell::new(false),
            selection_available: Cell::new(false),
            pending_relink: RefCell::new(None),
            unresolved: RefCell::new(std::collections::HashSet::new()),
            retry,
            close,
            references,
            reference_scroll: links,
            record: RefCell::new(None),
            draft: RefCell::new(None),
            dirty: Cell::new(false),
            saving: Cell::new(0),
            suppress: Cell::new(false),
            serial: Cell::new(0),
            size: Cell::new((0, 0)),
            restore_scroll: Cell::new(false),
            restore_idle: RefCell::new(None),
            scroll_mark,
            visibility_intent: Cell::new(None),
        }
    }

    fn snapshot(&self) -> Option<(DocumentRecord, Notebook)> {
        let record = self.record.borrow().clone()?;
        self.capture_context();
        let mut draft = self.draft.borrow_mut();
        let draft = draft.as_mut()?;
        let buffer = self.editor.buffer();
        draft.cursor = draft.cursor.min(buffer.char_count());
        draft.selection = draft.selection.min(buffer.char_count());
        draft.top = draft.top.min(buffer.char_count());
        Some((record, draft.clone()))
    }

    fn capture_selection(&self) {
        if self.editor.is_mapped() {
            let buffer = self.editor.buffer();
            if let Some(draft) = self.draft.borrow_mut().as_mut() {
                draft.cursor = buffer.cursor_position();
                draft.selection = buffer.iter_at_mark(&buffer.selection_bound()).offset();
            }
        }
    }

    fn capture_context(&self) {
        self.capture_selection();
        if self.editor.is_mapped() && !self.restore_scroll.get() {
            let top = self.top();
            if let Some(draft) = self.draft.borrow_mut().as_mut() {
                draft.top = top;
            }
        }
    }

    fn stop_scrolling(&self) {
        // GTK 4.22 can finalize an animation twice when resetting its buffer
        // during unmap. Setting the current value stops it before that teardown.
        for adjustment in [self.scroll.hadjustment(), self.scroll.vadjustment()] {
            adjustment.set_value(adjustment.value());
        }
    }

    fn cancel_restore(&self) {
        if let Some(idle) = self.restore_idle.take() {
            idle.remove();
        }
        self.restore_scroll.set(false);
    }

    pub(super) fn top(&self) -> i32 {
        let rect = self.editor.visible_rect();
        // Hit-test within a wrapped line. GTK returns no hit over blank space;
        // in that case, binary-search the paragraph by rendered y. Walking every
        // wrapped line makes a long paragraph quadratic to save.
        let mut line = self
            .editor
            .iter_at_location(rect.x() + rect.width() / 2, rect.y())
            .unwrap_or_else(|| {
                let start = self.editor.line_at_y(rect.y()).0;
                let mut end = start;
                end.forward_line();
                let (mut low, mut high) = (start.offset(), end.offset());
                let buffer = self.editor.buffer();
                while high - low > 1 {
                    let middle = low + (high - low) / 2;
                    if self
                        .editor
                        .iter_location(&buffer.iter_at_offset(middle))
                        .y()
                        <= rect.y()
                    {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                buffer.iter_at_offset(low)
            });
        if !self.editor.starts_display_line(&line) {
            self.editor.backward_display_line_start(&mut line);
        }
        line.offset()
    }
}

impl Shell {
    pub(super) fn wire_notebook(self: &Rc<Self>) {
        self.wire_notebook_editing();
        self.wire_passages();
        let area: adw::BreakpointBin = self.object("reading_area");
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse("max-width: 899px").expect("notebook breakpoint"),
        );
        narrow.add_setter(
            &self.notebook.split,
            "orientation",
            Some(&gtk::Orientation::Vertical.to_value()),
        );
        area.add_breakpoint(narrow);
        let weak = Rc::downgrade(self);
        area.connect_current_breakpoint_notify(move |_| {
            if let Some(s) = weak.upgrade()
                && let Some(Surface::Pdf(pdf)) = s.surface.borrow().as_ref()
                && !pdf.is_restoring()
            {
                // Protect the passage before BreakpointBin's next frame
                // restores focus through Papers' document-start traversal.
                pdf.restore(pdf.stable_location());
            }
            let weak = std::rc::Weak::clone(&weak);
            glib::idle_add_local_once(move || {
                if let Some(s) = weak.upgrade() {
                    s.layout_notebook();
                }
            });
        });
        self.connect("notebook_button", |s| s.toggle_notebook());
        let weak = Rc::downgrade(self);
        self.notebook.close.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.toggle_notebook();
            }
        });
        let weak = Rc::downgrade(self);
        self.notebook.attach.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.attach_passage();
            }
        });
        let weak = Rc::downgrade(self);
        self.notebook.retry.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                if s.notebook.draft.borrow().is_none() {
                    s.load_notebook();
                } else {
                    s.save();
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.notebook.editor.buffer().connect_changed(move |_| {
            if let Some(s) = weak.upgrade()
                && !s.notebook.suppress.get()
            {
                s.notebook.cancel_restore();
                s.notebook_changed();
                s.queue_connections();
            }
        });
        let weak = Rc::downgrade(self);
        self.notebook
            .editor
            .buffer()
            .connect_mark_set(move |_, _, mark| {
                if mark
                    .name()
                    .is_some_and(|name| name == "insert" || name == "selection_bound")
                    && let Some(s) = weak.upgrade()
                    && s.notebook.editor.is_mapped()
                    && !s.notebook.suppress.get()
                {
                    s.notebook.cancel_restore();
                    s.notebook_changed();
                    s.queue_connections();
                }
            });
        let weak = Rc::downgrade(self);
        self.notebook
            .scroll
            .vadjustment()
            .connect_value_changed(move |_| {
                if let Some(s) = weak.upgrade()
                    && !s.notebook.restore_scroll.get()
                    && s.notebook.panel.is_visible()
                {
                    s.notebook_changed();
                    s.queue_connections();
                }
            });
        let weak = Rc::downgrade(self);
        self.notebook.split.connect_position_notify(move |split| {
            if let Some(s) = weak.upgrade() {
                let n = &s.notebook;
                if n.suppress.get() || !n.panel.is_visible() {
                    return;
                }
                if let Some(draft) = n.draft.borrow_mut().as_mut() {
                    if split.orientation() == gtk::Orientation::Horizontal {
                        draft.width = (split.width() - split.position())
                            .clamp(Notebook::MIN_WIDTH, Notebook::MAX_SIZE);
                    } else {
                        draft.height = (split.height() - split.position())
                            .clamp(Notebook::MIN_HEIGHT, Notebook::MAX_SIZE);
                    }
                }
                s.notebook_changed();
            }
        });
        // React to allocation changes without keeping a frame callback running
        // while the reader is idle. A sidebar also changes the available width.
        let weak = Rc::downgrade(self);
        self.notebook.split.connect_max_position_notify(move |_| {
            let weak = std::rc::Weak::clone(&weak);
            glib::idle_add_local_once(move || {
                if let Some(s) = weak.upgrade() {
                    let split = &s.notebook.split;
                    let size = (split.width(), split.height());
                    if s.notebook.size.replace(size) != size {
                        s.layout_notebook();
                    }
                }
            });
        });
        let weak = Rc::downgrade(self);
        self.notebook.editor.connect_map(move |_| {
            if let Some(s) = weak.upgrade() {
                let n = &s.notebook;
                n.suppress.set(true);
                if let Some(draft) = n.draft.borrow().as_ref() {
                    let buffer = n.editor.buffer();
                    buffer.select_range(
                        &buffer.iter_at_offset(draft.cursor),
                        &buffer.iter_at_offset(draft.selection),
                    );
                }
                n.suppress.set(false);
                s.restore_notebook_scroll();
                s.queue_connections();
            }
        });
    }

    pub(super) fn activate_notebook(self: &Rc<Self>, record: Option<DocumentRecord>) {
        if let Some(active) = self.notebook.record.borrow_mut().as_mut() {
            self.reading_state.reconcile(active);
        }
        let same = self
            .notebook
            .record
            .borrow()
            .as_ref()
            .zip(record.as_ref())
            .is_some_and(|(a, b)| a.id == b.id || a.path == b.path);
        self.notebook.selection_available.set(false);
        self.notebook.capture_pending.set(false);
        self.notebook
            .selection_serial
            .set(self.notebook.selection_serial.get() + 1);
        self.notebook
            .passage_request
            .set(self.notebook.passage_request.get() + 1);
        if same {
            self.notebook.record.replace(record);
            return;
        }
        self.set_notebook_visible(false);
        let n = &self.notebook;
        n.serial.set(n.serial.get() + 1);
        n.record.replace(record);
        n.draft.take();
        n.history.replace(readero::notebook::History::default());
        n.pending_passage.take();
        n.pending_relink.take();
        n.unresolved.borrow_mut().clear();
        n.selection_available.set(false);
        n.capture_pending.set(false);
        n.selection_serial.set(n.selection_serial.get() + 1);
        n.dirty.set(false);
        n.suppress.set(true);
        n.tags_present.set(false);
        n.editor.buffer().set_text("");
        n.editor.set_editable(false);
        n.restore_scroll.set(false);
        n.visibility_intent.set(None);
        n.suppress.set(false);
        self.render_references();
        self.layout_notebook();
        if n.record.borrow().is_some() {
            self.load_notebook();
        }
    }

    fn load_notebook(self: &Rc<Self>) {
        let Some(record) = self.notebook.record.borrow().clone() else {
            return;
        };
        let n = &self.notebook;
        n.serial.set(n.serial.get() + 1);
        let serial = n.serial.get();
        n.status.set_text("Loading notebook…");
        n.retry.set_visible(false);
        let reply = self.reading_state.notebook(record);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = reply.await;
            let Some(s) = weak.upgrade() else {
                return;
            };
            let n = &s.notebook;
            if n.serial.get() != serial {
                return;
            }
            match result {
                Ok(mut draft) => {
                    let visibility_intent = n.visibility_intent.take();
                    if let Some(visible) = visibility_intent {
                        draft.visible = visible;
                    }
                    n.suppress.set(true);
                    let buffer = n.editor.buffer();
                    buffer.set_enable_undo(false);
                    n.tags_present.set(false);
                    buffer.set_text(&draft.text);
                    buffer.select_range(
                        &buffer.iter_at_offset(draft.cursor),
                        &buffer.iter_at_offset(draft.selection),
                    );
                    n.history.replace(readero::notebook::History::default());
                    n.restore_scroll.set(true);
                    n.editor.set_editable(n.panel.is_sensitive());
                    n.draft.replace(Some(draft));
                    n.tags_dirty.set(true);
                    n.suppress.set(false);
                    s.show_notebook();
                    s.restore_notebook_scroll();
                    s.render_references();
                    s.queue_connections();
                    s.notebook_status();
                    s.notebook_location();
                    if visibility_intent.is_some() {
                        s.notebook_changed();
                    }
                }
                Err(error) => {
                    n.status
                        .set_text(&format!("Notebook could not be loaded. {error}"));
                    n.retry.set_visible(true);
                }
            }
        });
    }

    pub(super) fn toggle_notebook(self: &Rc<Self>) {
        if !self.can_interact() || self.current.borrow().is_none() {
            return;
        }
        let visible = !self.notebook.panel.is_visible();
        if self.focus.get() {
            self.toggle_focus();
        }
        if let Some(draft) = self.notebook.draft.borrow_mut().as_mut() {
            draft.visible = visible;
        } else {
            self.notebook.visibility_intent.set(Some(visible));
        }
        self.set_notebook_visible(visible);
        self.layout_notebook();
        self.notebook_changed();
        if visible {
            self.notebook.editor.grab_focus();
        } else {
            self.save();
        }
    }

    pub(super) fn show_notebook(&self) {
        let visible = self.current.borrow().is_some()
            && !self.focus.get()
            && self.notebook.visibility_intent.get().unwrap_or_else(|| {
                self.notebook
                    .draft
                    .borrow()
                    .as_ref()
                    .is_some_and(|d| d.visible)
            });
        self.set_notebook_visible(visible);
        self.layout_notebook();
    }

    fn set_notebook_visible(&self, visible: bool) {
        let n = &self.notebook;
        if !visible && let Some(idle) = n.restore_idle.take() {
            idle.remove();
        }
        if !visible && n.editor.is_mapped() {
            n.stop_scrolling();
            n.capture_context();
            n.restore_scroll.set(true);
            if gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|widget| widget.is_ancestor(&n.panel))
            {
                self.button("notebook_button").grab_focus();
            }
        }
        n.suppress.set(true);
        n.panel.set_visible(visible);
        n.suppress.set(false);
    }

    fn layout_notebook(&self) {
        let n = &self.notebook;
        let vertical = n.split.orientation() == gtk::Orientation::Vertical;
        // Below this height, document margins alone can consume the remaining
        // reader viewport. Let GTK enforce a usable minimum while stacked.
        self.window
            .set_height_request(if n.panel.is_visible() && vertical {
                650
            } else {
                450
            });
        if !n.panel.is_visible() {
            return;
        }
        n.suppress.set(true);
        n.panel.set_size_request(
            if vertical { -1 } else { Notebook::MIN_WIDTH },
            if vertical { Notebook::MIN_HEIGHT } else { -1 },
        );
        let draft = n.draft.borrow();
        let extent = if vertical {
            n.split.height()
        } else {
            n.split.width()
        };
        let desired = draft.as_ref().map_or(
            if vertical {
                Notebook::DEFAULT_HEIGHT
            } else {
                Notebook::DEFAULT_WIDTH
            },
            |d| {
                if vertical { d.height } else { d.width }
            },
        );
        n.split
            .set_position((extent - desired.min(extent / 2)).max(0));
        n.suppress.set(false);
    }

    fn restore_notebook_scroll(self: &Rc<Self>) {
        let n = &self.notebook;
        if !n.restore_scroll.get() || !n.editor.is_mapped() || n.restore_idle.borrow().is_some() {
            return;
        }
        if let Some(draft) = n.draft.borrow().as_ref() {
            // GTK defers scrolling to a mark until the relevant lines have been
            // laid out. A raw adjustment value can clamp against temporary bounds.
            let buffer = n.editor.buffer();
            buffer.move_mark(&n.scroll_mark, &buffer.iter_at_offset(draft.top));
            n.editor.scroll_to_mark(&n.scroll_mark, 0.0, true, 0.0, 0.0);
        }
        // Fractional-scale layout updates can stop GTK's animation partway.
        // Once layout has run, finish at the mark without animation. This is a
        // one-shot idle, canceled on hide or new editing input, not frame polling.
        let weak = Rc::downgrade(self);
        let idle = glib::idle_add_local_once(move || {
            if let Some(s) = weak.upgrade() {
                let n = &s.notebook;
                n.restore_idle.take();
                if n.draft.borrow().is_some() {
                    let buffer = n.editor.buffer();
                    n.suppress.set(true);
                    let top = n
                        .editor
                        .iter_location(&buffer.iter_at_mark(&n.scroll_mark))
                        .y();
                    n.scroll
                        .vadjustment()
                        .set_value(f64::from(top + n.editor.top_margin()));
                    n.suppress.set(false);
                }
                n.restore_scroll.set(false);
            }
        });
        n.restore_idle.replace(Some(idle));
    }

    fn notebook_changed(self: &Rc<Self>) {
        let n = &self.notebook;
        if n.suppress.get() || n.draft.borrow().is_none() {
            return;
        }
        n.dirty.set(true);
        n.status.set_text("Unsaved changes");
        self.schedule_save();
    }

    pub(super) fn queue_notebook(self: &Rc<Self>) {
        if !self.notebook.dirty.replace(false) {
            return;
        }
        let Some((record, draft)) = self.notebook.snapshot() else {
            return;
        };
        self.notebook.status.set_text("Saving…");
        self.notebook.saving.set(self.notebook.saving.get() + 1);
        let reply = self.reading_state.save_notebook(record, draft);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let _ = reply.await;
            if let Some(s) = weak.upgrade() {
                s.notebook.saving.set(s.notebook.saving.get() - 1);
                s.refresh_storage();
            }
        });
    }

    pub(super) fn notebook_status(&self) {
        let n = &self.notebook;
        if n.draft.borrow().is_none() {
            return;
        }
        let status = self.reading_state.status();
        n.retry.set_visible(status.error.is_some());
        n.status.set_text(if status.error.is_some() {
            "Changes not saved — draft kept in this session"
        } else if n.dirty.get() {
            "Unsaved changes"
        } else if status.pending || n.saving.get() > 0 {
            "Saving…"
        } else {
            "Saved locally"
        });
    }

    pub(super) fn notebook_location(&self) {
        let rotated = matches!(self.surface.borrow().as_ref(), Some(Surface::Pdf(pdf)) if pdf.model.rotation() != 0);
        let label = if rotated {
            "Rotate upright to select"
        } else if self.notebook.pending_relink.borrow().is_some() {
            "Relink selected passage…"
        } else {
            "Save selected passage"
        };
        if self.notebook.capture.label().as_deref() != Some(label) {
            self.notebook.capture.set_label(label);
        }
        self.notebook.capture.set_sensitive(
            (rotated || self.notebook.selection_available.get())
                && !self.notebook.capture_pending.get()
                && self.can_interact()
                && self.notebook.draft.borrow().is_some(),
        );
        let locator = self.record_for_save().and_then(|r| r.locator);
        self.notebook.attach.set_sensitive(
            locator.is_some() && self.can_interact() && self.notebook.draft.borrow().is_some(),
        );
        self.notebook.location.set_text(
            &locator.map_or_else(|| "Waiting for a reading position…".into(), |l| l.label()),
        );
    }

    pub(super) fn attach_passage(self: &Rc<Self>) {
        if !self.can_interact() {
            return;
        }
        let Some(locator) = self.record_for_save().and_then(|r| r.locator) else {
            return;
        };
        self.notebook_metadata(|draft| {
            draft.references.push(Reference {
                passage: None,
                id: uuid::Uuid::new_v4().to_string(),
                label: locator.label(),
                locator,
            });
            Ok(())
        });
        self.render_references();
        self.notebook_changed();
    }

    pub(in crate::desktop::shell) fn render_references(self: &Rc<Self>) {
        let list = &self.notebook.references;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let references = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .map(|d| {
                d.references
                    .iter()
                    .map(|r| {
                        let location = r.locator.label();
                        let location = if r.passage.is_none() {
                            format!("{location} · no quotation")
                        } else {
                            location
                        };
                        (r.id.clone(), r.label.clone(), location)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (id, label, location) in references {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            let follow = icon_button("go-jump-symbolic", "Read this saved passage");
            follow.set_focus_on_click(false);
            let weak = Rc::downgrade(self);
            let follow_id = id.clone();
            follow.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.follow_saved_passage(&follow_id);
                }
            });
            row.append(&follow);
            let name = gtk::Entry::builder()
                .text(&label)
                .hexpand(true)
                .width_chars(8)
                .max_width_chars(18)
                .build();
            name.set_tooltip_text(Some("Reference name — for example, Worked example 1"));
            name.update_property(&[gtk::accessible::Property::Label("Passage reference name")]);
            let rename_id = id.clone();
            let weak = Rc::downgrade(self);
            name.connect_changed(move |entry| {
                if let Some(s) = weak.upgrade() {
                    s.notebook_metadata(|draft| {
                        if let Some(reference) =
                            draft.references.iter_mut().find(|r| r.id == rename_id)
                        {
                            reference.label = entry.text().into();
                        }
                        Ok(())
                    });
                }
            });
            let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
            details.set_hexpand(true);
            details.append(&name);
            let source = text(&location, "caption");
            source.set_ellipsize(gtk::pango::EllipsizeMode::End);
            source.set_max_width_chars(24);
            source.set_tooltip_text(Some(&location));
            details.append(&source);
            row.append(&details);
            let connect = icon_button("insert-link-symbolic", "Connect this passage to notes");
            connect.set_focus_on_click(false);
            let weak = Rc::downgrade(self);
            let target = id.clone();
            connect.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.begin_passage_connection(&target);
                }
            });
            row.append(&connect);
            row.append(&self.passage_details(&id));
            let remove = icon_button("edit-delete-symbolic", "Remove passage reference");
            let weak = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.delete_saved_passage(&id);
                }
            });
            row.append(&remove);
            list.append(&row);
        }
    }
}
