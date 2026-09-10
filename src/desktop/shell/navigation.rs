use super::*;

impl Shell {
    pub(super) fn turn(self: &Rc<Self>, next: bool) {
        if !self.prepare_navigation() {
            return;
        }
        match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => {
                if next {
                    pdf.next()
                } else {
                    pdf.previous()
                }
            }
            Some(Surface::Reflow(reflow)) => {
                reflow.command(serde_json::json!({"type":if next{"next"}else{"previous"}}))
            }
            None => {}
        }
    }

    pub(super) fn goto(self: &Rc<Self>, target: Destination, remember: bool) {
        if !self.prepare_navigation() {
            return;
        }
        if remember && let Some(locator) = self.record_for_save().and_then(|r| r.locator) {
            self.push_history(locator);
        }
        match (self.surface.borrow().as_ref(), target) {
            (Some(Surface::Pdf(pdf)), Destination::Locator(locator)) => pdf.restore(Some(locator)),
            (Some(Surface::Pdf(pdf)), Destination::PdfResult(result)) => {
                pdf.search.autoselect_result(&result)
            }
            (Some(Surface::Pdf(pdf)), Destination::PdfLink(link)) => pdf.view.handle_link(&link),
            (Some(Surface::Reflow(reflow)), Destination::Locator(locator)) => {
                reflow.command(serde_json::json!({"type":"goto","locator":locator}))
            }
            (Some(Surface::Reflow(reflow)), Destination::Href(href)) => {
                reflow.command(serde_json::json!({"type":"href","href":href}))
            }
            _ => {}
        }
        self.sync_controls();
    }

    pub(super) fn show_search(&self) {
        if !self.can_interact() {
            return;
        }
        self.sidebar().set_reveal_child(true);
        self.object::<gtk::DropDown>("sidebar_kind").set_selected(2);
        self.entry().set_visible(true);
        self.object::<gtk::Box>("search_navigation")
            .set_visible(true);
        self.entry().grab_focus();
    }

    pub(super) fn clear_navigation(&self) {
        let list = self.list();
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
    }

    pub(super) fn refresh_sidebar(self: &Rc<Self>) {
        self.clear_navigation();
        let kind = self.object::<gtk::DropDown>("sidebar_kind").selected();
        self.entry().set_visible(kind == 2);
        self.object::<gtk::Box>("search_navigation")
            .set_visible(kind == 2);
        self.label("search_status").set_text("");
        match kind {
            0 => {
                let toc = self.toc.borrow().clone();
                if toc.is_empty() {
                    self.label("search_status")
                        .set_text("No table of contents in this document.");
                }
                for item in toc {
                    self.navigation_row(&item.label, item.target, item.depth);
                }
            }
            1 => self.show_bookmarks(),
            _ => self.search(&self.entry().text()),
        }
    }

    pub(super) fn navigation_row(self: &Rc<Self>, title: &str, target: Destination, depth: u32) {
        let label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(29)
            .margin_start((depth.min(3) * 12) as i32)
            .build();
        let button = gtk::Button::builder()
            .child(&label)
            .css_classes(["flat"])
            .build();
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.goto(target.clone(), true);
            }
        });
        self.list().append(&button);
    }

    pub(super) fn search(self: &Rc<Self>, query: &str) {
        if !self.can_interact() {
            return;
        }
        self.search_index.set(-1);
        let remember = !query.is_empty()
            && match self.surface.borrow().as_ref() {
                Some(Surface::Pdf(pdf)) => pdf.search.search_term().is_none_or(|s| s.is_empty()),
                _ => false,
            };
        if remember && let Some(locator) = self.record_for_save().and_then(|r| r.locator) {
            self.push_history(locator);
        }
        self.clear_navigation();
        self.label("search_status").set_text(if query.is_empty() {
            "Search the whole document."
        } else {
            "Searching…"
        });
        match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => pdf.find(query),
            Some(Surface::Reflow(reflow)) => {
                reflow.command(serde_json::json!({"type":"search","query":query}))
            }
            None => {}
        }
    }

    pub(super) fn next_search_result(&self, next: bool) {
        if !self.can_interact() {
            return;
        }
        if self.object::<gtk::DropDown>("sidebar_kind").selected() != 2 {
            return;
        }
        let list = self.list();
        let mut count = 0;
        while list.row_at_index(count).is_some() {
            count += 1;
        }
        if count == 0 {
            return;
        }
        let index = if self.search_index.get() < 0 {
            if next { 0 } else { count - 1 }
        } else {
            (self.search_index.get() + if next { 1 } else { -1 }).rem_euclid(count)
        };
        self.search_index.set(index);
        if let Some(button) = list
            .row_at_index(index)
            .and_then(|row| row.child())
            .and_downcast::<gtk::Button>()
        {
            button.emit_clicked();
            button.grab_focus();
        }
    }

    pub(super) fn pdf_search_results(self: &Rc<Self>, count: i32) {
        if self.object::<gtk::DropDown>("sidebar_kind").selected() != 2 {
            return;
        }
        self.clear_navigation();
        self.label("search_status").set_text(if count == 0 {
            "No matches. Scanned pages may not contain searchable text."
        } else {
            "Search results"
        });
        let results = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => pdf.search.result_model(),
            _ => None,
        };
        if let Some(results) = results {
            self.label("search_status")
                .set_text(&format!("{} matches", results.n_items()));
            for i in 0..results.n_items().min(500) {
                if let Some(result) = results.item(i).and_downcast::<papers_view::SearchResult>() {
                    let title = format!("Page {} · Match {}", result.page() + 1, i + 1);
                    self.navigation_row(&title, Destination::PdfResult(result), 0);
                }
            }
        }
    }

    pub(super) fn bookmark(self: &Rc<Self>) {
        if !self.can_interact() {
            return;
        }
        let Some(record) = self.record_for_save() else {
            return;
        };
        let Some(locator) = record.locator.clone() else {
            return;
        };
        let bookmark = Bookmark {
            id: uuid::Uuid::new_v4().to_string(),
            document_id: record.id.clone(),
            label: locator.label(),
            locator,
        };
        let pending = self.reading_state.bookmark(record, bookmark);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = pending.await;
            if let Some(s) = weak.upgrade() {
                s.refresh_storage();
                match result {
                    Ok(()) => {
                        s.toast("Passage bookmarked");
                        if s.object::<gtk::DropDown>("sidebar_kind").selected() == 1 {
                            s.show_bookmarks();
                        }
                    }
                    Err(error) => s.action_error("Couldn’t bookmark this passage.", &error),
                }
            }
        });
    }

    pub(super) fn show_bookmarks(self: &Rc<Self>) {
        let Some(id) = self.current.borrow().as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let weak = Rc::downgrade(self);
        let generation = self.generation();
        let pending = self.reading_state.bookmarks(id);
        glib::MainContext::default().spawn_local(async move {
            let result = pending.await;
            let Some(s) = weak.upgrade() else {
                return;
            };
            if !s.is_current(generation)
                || s.object::<gtk::DropDown>("sidebar_kind").selected() != 1
            {
                return;
            }
            s.clear_navigation();
            match result {
                Ok(bookmarks) => {
                    if bookmarks.is_empty() {
                        s.label("search_status")
                            .set_text("Save a passage with Ctrl+D.");
                    }
                    for bookmark in bookmarks {
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                        let open = gtk::Button::builder()
                            .label(&bookmark.label)
                            .hexpand(true)
                            .css_classes(["flat"])
                            .build();
                        let remove = icon_button("edit-delete-symbolic", "Remove bookmark");
                        let weak = Rc::downgrade(&s);
                        let locator = bookmark.locator;
                        open.connect_clicked(move |_| {
                            if let Some(s) = weak.upgrade() {
                                s.goto(Destination::Locator(locator.clone()), true);
                            }
                        });
                        let reading_state = s.reading_state.clone();
                        let weak = Rc::downgrade(&s);
                        let id = bookmark.id;
                        remove.connect_clicked(move |_| {
                            let reading_state = reading_state.clone();
                            let weak = std::rc::Weak::clone(&weak);
                            let id = id.clone();
                            glib::MainContext::default().spawn_local(async move {
                                let result = reading_state.remove_bookmark(id).await;
                                if let Some(s) = weak.upgrade() {
                                    if let Err(error) = result {
                                        s.storage_error(&error.to_string());
                                    }
                                    s.show_bookmarks();
                                }
                            });
                        });
                        row.append(&open);
                        row.append(&remove);
                        s.list().append(&row);
                    }
                }
                Err(error) => s.storage_error(&error.to_string()),
            }
        });
    }

    pub(super) fn external(&self, href: &str) {
        if url::Url::parse(href)
            .is_ok_and(|url| matches!(url.scheme(), "https" | "http" | "mailto"))
        {
            gtk::UriLauncher::new(href).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
        }
    }
}
