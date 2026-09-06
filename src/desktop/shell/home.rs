use super::*;

impl Shell {
    pub(super) fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }

    pub(super) fn show_status(&self, title: &str, description: &str, loading: bool) {
        if let Some(old) = self.content().child_by_name("status") {
            self.content().remove(&old);
        }
        let page = adw::StatusPage::builder()
            .title(title)
            .description(description)
            .build();
        if loading {
            let spinner = gtk::Spinner::builder()
                .spinning(true)
                .width_request(32)
                .height_request(32)
                .halign(gtk::Align::Center)
                .build();
            page.set_child(Some(&spinner));
        } else {
            page.set_icon_name(Some("dialog-information-symbolic"));
        }
        self.content().add_named(&page, Some("status"));
        self.content().set_visible_child_name("status");
    }

    pub(super) fn error(&self, message: &str) {
        self.set_reading(false);
        self.show_status("Couldn’t open this document", message, false);
    }

    pub(super) fn home(self: &Rc<Self>) {
        self.cancel_open();
        self.save();
        self.dispose_surface();
        self.current.replace(None);
        let generation = self.gate.borrow_mut().begin();
        self.generation.set(generation);
        if self.focus.get() {
            self.focus.set(false);
            self.object::<adw::HeaderBar>("header").set_visible(true);
        }
        self.set_reading(false);
        self.window.set_title(Some("Readero"));
        self.render_home(&[]);
        let serial = self.open_serial.get();
        let worker = self.worker.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = worker.call(|store| store.recents()).await;
            let Some(s) = weak.upgrade() else {
                return;
            };
            if !s.is_current(generation) || !s.is_open(serial) {
                return;
            }
            match result {
                Ok(recents) => s.render_home(&recents),
                Err(error) => s.storage_error(&error.to_string()),
            }
        });
    }

    pub(super) fn render_home(self: &Rc<Self>, recents: &[DocumentRecord]) {
        if let Some(old) = self.content().child_by_name("home") {
            self.content().remove(&old);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["home"])
            .build();
        let clamp = adw::Clamp::builder()
            .maximum_size(690)
            .tightening_threshold(580)
            .margin_start(40)
            .margin_end(40)
            .margin_top(60)
            .margin_bottom(40)
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let icon = gtk::Image::from_icon_name("accessories-dictionary-symbolic");
        icon.set_pixel_size(42);
        icon.set_halign(gtk::Align::Start);
        icon.add_css_class("reading-mark");
        icon.set_margin_bottom(25);
        column.append(&icon);
        let eyebrow = text("YOUR READING SPACE", "home-eyebrow");
        eyebrow.set_margin_bottom(12);
        column.append(&eyebrow);
        let title = text(
            if recents.is_empty() {
                "A little room to read."
            } else {
                "Welcome back."
            },
            "home-title",
        );
        title.set_margin_bottom(12);
        column.append(&title);
        let description = text(
            if recents.is_empty() {
                "Open a document. Settle in. Pick up where you left off."
            } else {
                "Your next page is right where you left it."
            },
            "home-description",
        );
        description.set_margin_bottom(30);
        column.append(&description);
        if let Some(record) = recents.first() {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
            card.add_css_class("continue-card");
            card.append(&text(
                &format!(
                    "CONTINUE READING  ·  {}",
                    record.format.label().to_uppercase()
                ),
                "format-tag",
            ));
            let title = text(&record.title, "continue-title");
            title.set_wrap(true);
            card.append(&title);
            card.append(&text(
                &record
                    .locator
                    .as_ref()
                    .map_or_else(|| "Ready when you are".into(), Locator::label),
                "dim-label",
            ));
            let button = gtk::Button::builder()
                .label(if record.path.is_file() {
                    "Continue reading"
                } else {
                    "Locate document"
                })
                .halign(gtk::Align::Start)
                .margin_top(8)
                .css_classes(["suggested-action", "home-open"])
                .build();
            let remove = self.remove_recent_button(record.id.clone(), record.path.clone());
            let record = record.clone();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    if record.path.is_file() {
                        s.open(record.path.clone());
                    } else {
                        s.open_dialog(Some(record.id.clone()));
                    }
                }
            });
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            actions.append(&button);
            actions.append(&remove);
            card.append(&actions);
            card.set_margin_bottom(28);
            column.append(&card);
        } else {
            let open = gtk::Button::builder()
                .label("Open a document")
                .halign(gtk::Align::Start)
                .css_classes(["suggested-action", "home-open"])
                .build();
            let weak = Rc::downgrade(self);
            open.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.open_dialog(None);
                }
            });
            column.append(&open);
            let formats = text("PDF  ·  EPUB  ·  Markdown", "dim-label");
            formats.set_margin_top(18);
            formats.add_css_class("caption");
            column.append(&formats);
            let hint = text(
                "You can also drop a file anywhere in this window.",
                "dim-label",
            );
            hint.add_css_class("caption");
            hint.set_margin_top(70);
            column.append(&hint);
        }
        if recents.len() > 1 {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let title = text("RECENTLY OPENED", "home-eyebrow");
            title.set_hexpand(true);
            row.append(&title);
            let open = gtk::Button::builder()
                .label("Open another…")
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(self);
            open.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.open_dialog(None);
                }
            });
            row.append(&open);
            row.set_margin_bottom(10);
            column.append(&row);
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["recent-list", "boxed-list"])
                .build();
            for record in recents.iter().skip(1) {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let title = text(&record.title, "recent-title");
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                title.set_max_width_chars(44);
                labels.append(&title);
                labels.append(&text(
                    &format!(
                        "{}  ·  {}",
                        record.format.label(),
                        record
                            .locator
                            .as_ref()
                            .map_or_else(|| "Not started".into(), Locator::label)
                    ),
                    "caption",
                ));
                let open = gtk::Button::builder()
                    .child(&labels)
                    .hexpand(true)
                    .css_classes(["flat"])
                    .build();
                let record = record.clone();
                let weak = Rc::downgrade(self);
                let open_record = record.clone();
                open.connect_clicked(move |_| {
                    if let Some(s) = weak.upgrade() {
                        if open_record.path.is_file() {
                            s.open(open_record.path.clone());
                        } else {
                            s.open_dialog(Some(open_record.id.clone()));
                        }
                    }
                });
                row.append(&open);
                let remove = self.remove_recent_button(record.id, record.path);
                row.append(&remove);
                list.append(&row);
            }
            column.append(&list);
        }
        clamp.set_child(Some(&column));
        scroll.set_child(Some(&clamp));
        self.content().add_named(&scroll, Some("home"));
        self.content().set_visible_child_name("home");
    }
    fn remove_recent_button(self: &Rc<Self>, id: String, path: PathBuf) -> gtk::Button {
        let remove = icon_button("list-remove-symbolic", "Remove from recents");
        let worker = self.worker.clone();
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        let serial = self.open_serial.get();
        remove.connect_clicked(move |_| {
            let worker = worker.clone();
            let weak = std::rc::Weak::clone(&weak);
            let id = id.clone();
            let Some(s) = weak.upgrade() else {
                return;
            };
            // Cancel retained retries before queuing hide, so Home and Close
            // cannot resurrect this recent. Include provisional session IDs.
            let mut removed = Vec::new();
            s.unsaved.borrow_mut().retain(|saved_path, snapshot| {
                if saved_path == &path || snapshot.1.id == id {
                    removed.push((saved_path.clone(), snapshot.clone()));
                    false
                } else {
                    true
                }
            });
            let pending = worker.call(move |store| store.hide(&id));
            glib::MainContext::default().spawn_local(async move {
                let result = pending.await;
                if let Some(s) = weak.upgrade() {
                    match result {
                        Ok(()) => {
                            if s.unsaved.borrow().is_empty() {
                                s.object::<adw::Banner>("save_banner").set_revealed(false);
                            }
                            if s.is_current(generation) && s.is_open(serial) {
                                s.home();
                            }
                        }
                        Err(error) => {
                            let mut unsaved = s.unsaved.borrow_mut();
                            for (path, snapshot) in removed {
                                unsaved.entry(path).or_insert(snapshot);
                            }
                            drop(unsaved);
                            s.storage_error(&error.to_string());
                        }
                    }
                }
            });
        });
        remove
    }
}
