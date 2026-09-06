use super::*;

impl Shell {
    pub(super) fn wire(self: &Rc<Self>) {
        self.connect("open_button", |s| s.open_dialog(None));
        self.connect("home_button", |s| s.home());
        self.connect("focus_button", |s| s.toggle_focus());
        self.connect("sidebar_button", |s| {
            s.sidebar().set_reveal_child(!s.sidebar().reveals_child())
        });
        self.connect("search_button", |s| s.show_search());
        self.connect("search_next", |s| s.next_search_result(true));
        self.connect("search_previous", |s| s.next_search_result(false));
        self.connect("bookmark_button", |s| s.bookmark());
        self.connect("next_button", |s| s.turn(true));
        self.connect("previous_button", |s| s.turn(false));
        self.connect("back_button", |s| s.history(false));
        self.connect("forward_button", |s| s.history(true));
        let scroll: gtk::ToggleButton = self.object("scroll_mode");
        let pages: gtk::ToggleButton = self.object("pages_mode");
        pages.set_group(Some(&scroll));
        for (button, mode) in [(scroll, Mode::Scroll), (pages, Mode::Pages)] {
            let weak = Rc::downgrade(self);
            button.connect_toggled(move |button| {
                if button.is_active()
                    && let Some(s) = weak.upgrade()
                    && !s.updating.get()
                {
                    s.change_settings(|settings| settings.mode = mode);
                }
            });
        }
        let kind: gtk::DropDown = self.object("sidebar_kind");
        kind.set_model(Some(&gtk::StringList::new(&[
            "Contents",
            "Bookmarks",
            "Search",
        ])));
        let weak = Rc::downgrade(self);
        kind.connect_selected_notify(move |_| {
            if let Some(s) = weak.upgrade() {
                s.refresh_sidebar();
            }
        });
        let weak = Rc::downgrade(self);
        self.entry().connect_search_changed(move |entry| {
            if let Some(s) = weak.upgrade() {
                s.search(&entry.text());
            }
        });
        let weak = Rc::downgrade(self);
        self.entry().connect_stop_search(move |_| {
            if let Some(s) = weak.upgrade() {
                s.sidebar().set_reveal_child(false);
                s.search("");
            }
        });
        self.appearance();
        self.menu();
        self.page_popover();
        self.shortcuts();
        let drop = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            if let (Some(s), Ok(file)) = (weak.upgrade(), value.get::<gio::File>())
                && let Some(path) = file.path()
            {
                s.open(path);
                return true;
            }
            false
        });
        self.window.add_controller(drop);
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if !window.is_active()
                && let Some(s) = weak.upgrade()
            {
                s.save();
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(s) = weak.upgrade() {
                s.close();
            }
            glib::Propagation::Stop
        });
        let banner: adw::Banner = self.object("save_banner");
        let weak = Rc::downgrade(self);
        banner.connect_button_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.save();
            }
        });
    }

    pub(super) fn set_reading(&self, reading: bool) {
        self.object::<gtk::Widget>("modes").set_visible(reading);
        self.object::<gtk::Widget>("footer")
            .set_visible(reading && !self.focus.get());
        for id in [
            "focus_button",
            "appearance_button",
            "bookmark_button",
            "search_button",
            "sidebar_button",
        ] {
            self.object::<gtk::Widget>(id).set_visible(reading);
        }
        if !reading {
            self.sidebar().set_reveal_child(false);
        }
        self.object::<gtk::MenuButton>("page_button")
            .set_visible(matches!(
                self.surface.borrow().as_ref(),
                Some(Surface::Pdf(_))
            ));
    }

    pub(super) fn sync_controls(&self) {
        let settings = self.current.borrow().as_ref().map(|r| r.settings.clone());
        if let Some(settings) = settings {
            self.updating.set(true);
            self.object::<gtk::ToggleButton>(if settings.mode == Mode::Scroll {
                "scroll_mode"
            } else {
                "pages_mode"
            })
            .set_active(true);
            self.updating.set(false);
        }
        self.button("back_button")
            .set_sensitive(!self.back.borrow().is_empty());
        self.button("forward_button")
            .set_sensitive(!self.forward.borrow().is_empty());
    }

    pub(super) fn change_settings(self: &Rc<Self>, change: impl FnOnce(&mut Settings)) {
        let settings = {
            let mut current = self.current.borrow_mut();
            let Some(record) = current.as_mut() else {
                return;
            };
            change(&mut record.settings);
            record.settings.normalize();
            record.settings.clone()
        };
        match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => pdf.settings(&settings),
            Some(Surface::Reflow(reflow)) => {
                reflow.command(serde_json::json!({"type":"settings","settings":settings}))
            }
            None => {}
        }
        self.sync_controls();
        self.schedule_save();
    }

    pub(super) fn toggle_focus(&self) {
        if self.surface.borrow().is_none() {
            return;
        }
        let focus = !self.focus.get();
        self.focus.set(focus);
        if focus {
            self.sidebar_before_focus
                .set(self.sidebar().reveals_child());
            self.sidebar().set_reveal_child(false);
        } else {
            self.sidebar()
                .set_reveal_child(self.sidebar_before_focus.get());
        }
        self.object::<adw::HeaderBar>("header").set_visible(!focus);
        self.object::<gtk::Box>("footer").set_visible(!focus);
    }

    pub(super) fn escape(&self) {
        if self.gate.borrow().accepts(self.generation.get())
            && let Some(Surface::Reflow(reflow)) = self.surface.borrow().as_ref()
        {
            reflow.command(serde_json::json!({"type":"escape"}));
        } else {
            self.escape_chrome();
        }
    }

    pub(super) fn escape_chrome(&self) {
        if self.focus.get() {
            self.toggle_focus();
        } else {
            self.sidebar().set_reveal_child(false);
        }
    }

    pub(super) fn appearance(self: &Rc<Self>) {
        let popover = gtk::Popover::new();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 14);
        column.add_css_class("appearance");
        column.set_width_request(270);
        column.append(&text("Reading appearance", "heading"));
        let pdf_options = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let fits = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (label, sizing) in [("Fit width", "width"), ("Fit page", "page")] {
            let button = gtk::Button::with_label(label);
            button.set_hexpand(true);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.change_settings(|settings| settings.pdf_sizing = sizing.into());
                }
            });
            fits.append(&button);
        }
        pdf_options.append(&fits);
        let zoom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (label, delta) in [("−", -0.15), ("+", 0.15)] {
            let button = gtk::Button::with_label(label);
            button.set_hexpand(true);
            button.set_tooltip_text(Some(if delta > 0.0 { "Zoom in" } else { "Zoom out" }));
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    let scale = match s.surface.borrow().as_ref() {
                        Some(Surface::Pdf(pdf)) => pdf.model.scale(),
                        _ => 1.0,
                    };
                    s.change_settings(|settings| {
                        settings.pdf_sizing = "custom".into();
                        settings.pdf_scale = scale + delta;
                    });
                }
            });
            zoom.append(&button);
        }
        let rotate = gtk::Button::with_label("Rotate");
        let weak = Rc::downgrade(self);
        rotate.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.change_settings(|settings| settings.rotation += 90);
            }
        });
        zoom.append(&rotate);
        pdf_options.append(&zoom);
        column.append(&pdf_options);
        let reflow_options = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let palettes = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for (label, palette) in [
            ("Light", Palette::Light),
            ("Warm", Palette::Warm),
            ("Dark", Palette::Dark),
        ] {
            let button = gtk::Button::with_label(label);
            button.set_hexpand(true);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.change_settings(|settings| settings.palette = palette);
                }
            });
            palettes.append(&button);
        }
        reflow_options.append(&palettes);
        let font =
            gtk::DropDown::from_strings(&["Reading serif", "Reading sans", "Publisher style"]);
        let weak = Rc::downgrade(self);
        font.connect_selected_notify(move |font| {
            if let Some(s) = weak.upgrade()
                && !s.updating.get()
            {
                s.change_settings(|settings| {
                    settings.font = match font.selected() {
                        1 => "sans",
                        2 => "publisher",
                        _ => "serif",
                    }
                    .into()
                });
            }
        });
        reflow_options.append(&font);
        let mut scales = Vec::new();
        for (label, min, max, step, initial) in [
            ("Text size", 14.0, 32.0, 1.0, 20.0),
            ("Line spacing", 1.2, 2.2, 0.1, 1.65),
            ("Reading width", 480.0, 1100.0, 20.0, 740.0),
        ] {
            reflow_options.append(&text(label, "caption"));
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
            scale.set_value(initial);
            scale.set_draw_value(false);
            scale.set_tooltip_text(Some(label));
            let weak = Rc::downgrade(self);
            scale.connect_value_changed(move |scale| {
                if let Some(s) = weak.upgrade()
                    && !s.updating.get()
                {
                    s.change_settings(|settings| match label {
                        "Text size" => settings.font_size = scale.value(),
                        "Line spacing" => settings.line_height = scale.value(),
                        _ => settings.width = scale.value() as u32,
                    });
                }
            });
            reflow_options.append(&scale);
            scales.push(scale);
        }
        column.append(&reflow_options);
        popover.set_child(Some(&column));
        let weak = Rc::downgrade(self);
        popover.connect_show(move |_| {
            if let Some(s) = weak.upgrade() {
                let record = s.current.borrow().clone();
                if let Some(record) = record {
                    s.updating.set(true);
                    pdf_options.set_visible(record.format == Format::Pdf);
                    reflow_options.set_visible(record.format != Format::Pdf);
                    font.set_selected(match record.settings.font.as_str() {
                        "sans" => 1,
                        "publisher" => 2,
                        _ => 0,
                    });
                    for (scale, value) in scales.iter().zip([
                        record.settings.font_size,
                        record.settings.line_height,
                        f64::from(record.settings.width),
                    ]) {
                        scale.set_value(value);
                    }
                    s.updating.set(false);
                }
            }
        });
        self.object::<gtk::MenuButton>("appearance_button")
            .set_popover(Some(&popover));
    }

    pub(super) fn page_popover(self: &Rc<Self>) {
        let popover = gtk::Popover::new();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        column.set_margin_top(12);
        column.set_margin_bottom(12);
        column.set_margin_start(12);
        column.set_margin_end(12);
        column.append(&text("Go to page", "heading"));
        let entry = gtk::Entry::builder()
            .placeholder_text("Page number or label")
            .width_chars(18)
            .build();
        let weak = Rc::downgrade(self);
        let close = popover.clone();
        entry.connect_activate(move |entry| {
            if let Some(s) = weak.upgrade() {
                if let Some(locator) = s.record_for_save().and_then(|r| r.locator) {
                    s.push_history(locator);
                }
                if let Some(Surface::Pdf(pdf)) = s.surface.borrow().as_ref() {
                    pdf.goto_page(&entry.text());
                }
                s.sync_controls();
            }
            close.popdown();
        });
        column.append(&entry);
        popover.set_child(Some(&column));
        self.object::<gtk::MenuButton>("page_button")
            .set_popover(Some(&popover));
    }

    pub(super) fn menu(self: &Rc<Self>) {
        let popover = gtk::Popover::new();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.set_margin_top(8);
        column.set_margin_bottom(8);
        column.set_margin_start(8);
        column.set_margin_end(8);
        for (label, action) in [
            ("Open document…", "open"),
            ("Fullscreen · F11", "fullscreen"),
            ("Keyboard shortcuts", "shortcuts"),
            ("About Readero", "about"),
        ] {
            let button = gtk::Button::builder()
                .label(label)
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(self);
            let close = popover.clone();
            button.connect_clicked(move |_| {
                close.popdown();
                if let Some(s) = weak.upgrade() {
                    match action {
                        "open" => s.open_dialog(None),
                        "fullscreen" => s.fullscreen(),
                        "shortcuts" => s.shortcut_help(),
                        _ => {
                            let dialog = adw::AboutDialog::builder()
                                .application_name("Readero")
                                .application_icon("io.github.readero.Readero")
                                .version(env!("CARGO_PKG_VERSION"))
                                .comments("A quiet place for your documents. Built for reading, one passage at a time.")
                                .build();
                            dialog.present(Some(&s.window));
                        }
                    }
                }
            });
            column.append(&button);
        }
        popover.set_child(Some(&column));
        self.object::<gtk::MenuButton>("menu_button")
            .set_popover(Some(&popover));
    }

    pub(super) fn fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    pub(super) fn shortcut_help(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading("A few useful keys")
            .body(concat!(
                "Ctrl+O    Open a document\nCtrl+F    Find in this document\n",
                "Ctrl+D    Bookmark this passage\nCtrl+L    Go to a PDF page\n",
                "Alt+← / →    Back / forward\nF8    Contents and bookmarks\n",
                "F9    Focus on reading\nF11    Fullscreen\n",
                "Esc    Return to the reading controls\n\n",
                "In Pages: ← / → or Page Up / Page Down"
            ))
            .build();
        dialog.add_response("close", "Close");
        dialog.present(Some(&self.window));
    }

    pub(super) fn shortcuts(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(s) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);
            let handled = match key {
                gdk::Key::o | gdk::Key::O if ctrl => {
                    s.open_dialog(None);
                    true
                }
                gdk::Key::f | gdk::Key::F if ctrl => {
                    s.show_search();
                    true
                }
                gdk::Key::g | gdk::Key::G if ctrl => {
                    s.next_search_result(!state.contains(gdk::ModifierType::SHIFT_MASK));
                    true
                }
                gdk::Key::d | gdk::Key::D if ctrl => {
                    s.bookmark();
                    true
                }
                gdk::Key::l | gdk::Key::L if ctrl => {
                    s.object::<gtk::MenuButton>("page_button").popup();
                    true
                }
                gdk::Key::Left if alt => {
                    s.history(false);
                    true
                }
                gdk::Key::Right if alt => {
                    s.history(true);
                    true
                }
                gdk::Key::F8 => {
                    s.sidebar().set_reveal_child(!s.sidebar().reveals_child());
                    true
                }
                gdk::Key::F9 => {
                    s.toggle_focus();
                    true
                }
                gdk::Key::F11 => {
                    s.fullscreen();
                    true
                }
                gdk::Key::Escape => {
                    s.escape();
                    false
                }
                _ => false,
            };
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.window.add_controller(keys);
    }
}
