use super::*;
use readero::passage::Capture;
use webkit6::prelude::*;

#[derive(serde::Deserialize)]
struct Resolved {
    #[serde(flatten)]
    capture: Capture,
    recovered: bool,
}

impl Shell {
    pub(in crate::desktop::shell) fn wire_passages(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.notebook.capture.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.save_selected_passage();
            }
        });
        let weak = Rc::downgrade(self);
        self.notebook.connect.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.connect_notebook_selection();
            }
        });
    }

    pub(in crate::desktop::shell) fn cancel_passage_navigation(&self) {
        self.notebook
            .passage_request
            .set(self.notebook.passage_request.get() + 1);
        self.notebook.selection_available.set(false);
        self.notebook
            .selection_serial
            .set(self.notebook.selection_serial.get() + 1);
        if let Some(Surface::Pdf(pdf)) = self.surface.borrow().as_ref() {
            pdf.clear_passage();
        }
        self.notebook_location();
    }

    fn passage_request_current(
        &self,
        generation: u64,
        request: u64,
        reference: &Reference,
    ) -> bool {
        self.is_current(generation)
            && self.notebook.passage_request.get() == request
            && self.notebook.draft.borrow().as_ref().is_some_and(|d| {
                d.references
                    .iter()
                    .any(|r| r.id == reference.id && r.passage == reference.passage)
            })
    }

    pub(in crate::desktop::shell) fn source_selection(&self, serial: u64, available: bool) {
        self.notebook
            .passage_request
            .set(self.notebook.passage_request.get() + 1);
        self.notebook.selection_serial.set(serial);
        self.notebook.selection_available.set(available);
        self.notebook_location();
    }

    pub(in crate::desktop::shell) fn wire_pdf_passages(
        self: &Rc<Self>,
        pdf: &Rc<Pdf>,
        generation: u64,
    ) {
        let weak = Rc::downgrade(self);
        pdf.view.connect_selection_changed(move |view| {
            if let Some(s) = weak.upgrade()
                && s.is_current(generation)
            {
                s.source_selection(s.notebook.selection_serial.get() + 1, view.has_selection());
            }
        });
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(s) = weak.upgrade()
                && s.is_current(generation)
            {
                s.cancel_passage_navigation();
            }
            glib::Propagation::Proceed
        });
        pdf.view.add_controller(scroll);
        let weak = Rc::downgrade(self);
        let weak_pdf = Rc::downgrade(pdf);
        pdf.view.connect_local("popup", false, move |values| {
            if let (Some(s), Some(pdf)) = (weak.upgrade(), weak_pdf.upgrade())
                && s.is_current(generation)
            {
                let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let rotated = pdf.model.rotation() != 0;
                let save = gtk::Button::with_label(if rotated {
                    "Rotate upright to select"
                } else {
                    "Save selected passage"
                });
                save.set_sensitive(
                    rotated || (pdf.view.has_selection() && s.notebook.selection_available.get()),
                );
                let popover = gtk::Popover::builder().child(&column).build();
                let weak_popover = popover.downgrade();
                let weak = Rc::downgrade(&s);
                save.connect_clicked(move |_| {
                    if let Some(s) = weak.upgrade() {
                        s.save_selected_passage();
                    }
                    if let Some(p) = weak_popover.upgrade() {
                        p.popdown();
                    }
                });
                column.append(&save);
                let copy = gtk::Button::with_label("Copy selected text");
                copy.set_sensitive(pdf.view.has_selection());
                let weak_pdf = Rc::downgrade(&pdf);
                let weak_popover = popover.downgrade();
                copy.connect_clicked(move |_| {
                    if let Some(pdf) = weak_pdf.upgrade() {
                        pdf.view.copy();
                    }
                    if let Some(p) = weak_popover.upgrade() {
                        p.popdown();
                    }
                });
                column.append(&copy);
                popover.set_parent(&pdf.view);
                let x = values
                    .get(2)
                    .and_then(|v| v.get::<f64>().ok())
                    .unwrap_or(24.0);
                let y = values
                    .get(3)
                    .and_then(|v| v.get::<f64>().ok())
                    .unwrap_or(24.0);
                popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                popover.connect_closed(|p| p.unparent());
                popover.popup();
            }
            None
        });
    }

    pub(in crate::desktop::shell) fn wire_reflow_passages(self: &Rc<Self>) {
        let surface = self.surface.borrow();
        let Some(Surface::Reflow(reader)) = surface.as_ref() else {
            return;
        };
        let weak = Rc::downgrade(self);
        reader.view.connect_context_menu(move |_, menu, _| {
            if let Some(s) = weak.upgrade() {
                let action = gio::SimpleAction::new("save-selection", None);
                action.set_enabled(s.notebook.selection_available.get());
                let weak = Rc::downgrade(&s);
                action.connect_activate(move |_, _| {
                    if let Some(s) = weak.upgrade() {
                        s.save_selected_passage();
                    }
                });
                menu.append(&webkit6::ContextMenuItem::from_gaction(
                    &action,
                    "Save selected passage",
                    None,
                ));
            }
            false
        });
    }

    pub(in crate::desktop::shell) fn save_selected_passage(self: &Rc<Self>) {
        if !self.can_interact() || self.notebook.capture_pending.get() {
            return;
        }
        let rotated = matches!(self.surface.borrow().as_ref(), Some(Surface::Pdf(pdf)) if pdf.model.rotation() != 0);
        if rotated {
            self.change_settings(|s| s.rotation = 0);
            self.toast("PDF upright. Select text, then save the passage.");
            return;
        }
        if !self.notebook.selection_available.get() {
            self.toast("Select text in the document first.");
            return;
        }
        if self.notebook.draft.borrow().is_none() {
            self.toast("Wait for the notebook to finish loading.");
            return;
        }
        let generation = self.generation();
        let serial = self.notebook.selection_serial.get();
        let revision = self
            .current
            .borrow()
            .as_ref()
            .map(|r| r.revision.clone())
            .unwrap_or_default();
        let pending = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => {
                let result = pdf.capture_selection(&revision);
                glib::MainContext::default().spawn_local({
                    let weak = Rc::downgrade(self);
                    async move {
                        if let Some(s) = weak.upgrade() {
                            s.finish_capture(generation, serial, result);
                        }
                    }
                });
                None
            }
            Some(Surface::Reflow(reader)) => Some(reader.selection_request::<Capture>(
                "readeroCaptureSelection",
                serde_json::json!(serial),
            )),
            None => return,
        };
        self.notebook.capture_pending.set(true);
        self.notebook_location();
        if let Some(pending) = pending {
            let weak = Rc::downgrade(self);
            glib::MainContext::default().spawn_local(async move {
                let result = pending.await;
                if let Some(s) = weak.upgrade() {
                    s.finish_capture(generation, serial, result);
                }
            });
        }
    }

    fn finish_capture(
        self: &Rc<Self>,
        generation: u64,
        serial: u64,
        result: readero::document::Result<Capture>,
    ) {
        if !self.is_current(generation) {
            return;
        }
        self.notebook.capture_pending.set(false);
        self.notebook_location();
        if self.notebook.selection_serial.get() != serial {
            self.toast("The selection changed. Select the passage again.");
            return;
        }
        let capture = match result.and_then(|c| c.passage.validate(&c.locator).map(|()| c)) {
            Ok(c) => c,
            Err(error) => {
                self.toast(&error.to_string());
                return;
            }
        };
        let pending_relink = self.notebook.pending_relink.borrow().clone();
        if let Some(id) = pending_relink {
            self.confirm_relink(id, capture);
            return;
        }
        let mut saved_id = String::new();
        if !self.notebook_metadata(|d| {
            if let Some(existing) = d
                .references
                .iter()
                .find(|r| r.passage.as_ref() == Some(&capture.passage))
            {
                saved_id.clone_from(&existing.id);
            } else {
                saved_id = uuid::Uuid::new_v4().to_string();
                d.references.push(Reference {
                    id: saved_id.clone(),
                    label: capture.passage.label(),
                    locator: capture.locator,
                    passage: Some(capture.passage),
                });
            }
            d.visible = true;
            Ok(())
        }) {
            return;
        }
        if self.focus.get() {
            self.toggle_focus();
        }
        self.show_notebook();
        self.render_references();
        self.reveal_saved_passage(&saved_id);
        self.toast("Selected passage captured");
    }

    fn reveal_saved_passage(self: &Rc<Self>, id: &str) {
        let index = self.notebook.draft.borrow().as_ref().and_then(|draft| {
            draft
                .references
                .iter()
                .position(|reference| reference.id == id)
        });
        let Some(row) = index.and_then(|index| self.notebook.references.row_at_index(index as i32))
        else {
            return;
        };
        self.notebook.references.select_row(Some(&row));
        // Wait for the newly built rows to be allocated, then reveal the saved
        // item without moving focus or the notebook's text selection.
        let weak = Rc::downgrade(self);
        let serial = self.notebook.serial.get();
        let after_layout = Cell::new(false);
        self.notebook.references.add_tick_callback(move |list, _| {
            if !after_layout.replace(true) {
                return glib::ControlFlow::Continue;
            }
            if let Some(s) = weak.upgrade()
                && s.notebook.serial.get() == serial
                && list.selected_row().as_ref() == Some(&row)
                && let Some(bounds) = row.compute_bounds(list)
            {
                s.notebook.reference_scroll.vadjustment().clamp_page(
                    f64::from(bounds.y()),
                    f64::from(bounds.y() + bounds.height()),
                );
            }
            glib::ControlFlow::Break
        });
    }

    pub(in crate::desktop::shell) fn cancel_pending_connection(self: &Rc<Self>) -> bool {
        let pending = self.notebook.pending_passage.take().is_some();
        let relink = self.notebook.pending_relink.take().is_some();
        if pending || relink {
            self.notebook.capture.set_label("Save selected passage");
            self.queue_connections();
            self.toast("Connection cancelled");
        }
        pending || relink
    }

    pub(in crate::desktop::shell) fn begin_passage_connection(self: &Rc<Self>, id: &str) {
        self.notebook.pending_passage.replace(Some(id.into()));
        self.notebook.editor.grab_focus();
        self.queue_connections();
        self.toast("Select the notebook text to connect, then press Connect. Escape cancels.");
    }

    fn note_range(&self) -> Option<(i32, i32)> {
        let buffer = self.notebook.editor.buffer();
        if let Some((start, end)) = buffer.selection_bounds() {
            return (!buffer.text(&start, &end, true).trim().is_empty())
                .then_some((start.offset(), end.offset()));
        }
        let caret = buffer.cursor_position();
        if let Some(section) = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .and_then(|d| d.section_at(caret))
            .cloned()
        {
            buffer.select_range(
                &buffer.iter_at_offset(section.start),
                &buffer.iter_at_offset(section.end),
            );
            return Some((section.start, section.end));
        }
        let mut start = buffer.iter_at_offset(caret);
        start.set_line_offset(0);
        let mut end = start;
        end.forward_line();
        if buffer.text(&start, &end, true).trim().is_empty() {
            return None;
        }
        loop {
            let mut previous = start;
            if !previous.backward_line() || buffer.text(&previous, &start, true).trim().is_empty() {
                break;
            }
            start = previous;
        }
        loop {
            let mut next = end;
            next.forward_line();
            if next == end || buffer.text(&end, &next, true).trim().is_empty() {
                break;
            }
            end = next;
        }
        if end.offset() > start.offset() && end.starts_line() {
            end.backward_char();
        }
        buffer.select_range(&start, &end);
        (end.offset() > start.offset()).then_some((start.offset(), end.offset()))
    }

    pub(in crate::desktop::shell) fn connect_notebook_selection(self: &Rc<Self>) {
        if !self.can_interact() || !self.notebook.editor.is_editable() {
            return;
        }
        let Some((start, end)) = self.note_range() else {
            self.toast("Select some notebook text to connect.");
            return;
        };
        let pending = self.notebook.pending_passage.borrow().clone();
        if let Some(id) = pending {
            if self.notebook_metadata(|d| d.connect(start, end, &id)) {
                self.notebook.pending_passage.take();
                self.queue_connections();
            }
            return;
        }
        let ids = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .map(|d| d.references.iter().map(|r| r.id.clone()).collect())
            .unwrap_or_default();
        self.choose_passage(ids, Some((start, end)));
    }

    pub(in crate::desktop::shell) fn follow_notebook_section(self: &Rc<Self>, offset: i32) {
        let ids = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .and_then(|d| d.section_at(offset))
            .map(|s| s.passages.clone())
            .unwrap_or_default();
        self.follow_passage_ids(ids);
    }

    pub(in crate::desktop::shell) fn follow_passage_ids(self: &Rc<Self>, ids: Vec<String>) {
        if ids.len() == 1 {
            self.follow_saved_passage(&ids[0]);
        } else if !ids.is_empty() {
            self.choose_passage(ids, None);
        }
    }

    fn choose_passage(self: &Rc<Self>, ids: Vec<String>, range: Option<(i32, i32)>) {
        if ids.is_empty() {
            self.toast("Save a passage in the document first.");
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading(if range.is_some() {
                "Connect a saved passage"
            } else {
                "Open connected passage"
            })
            .build();
        dialog.add_response("cancel", "Cancel");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let generation = self.generation();
        let serial = self.notebook.serial.get();
        for id in ids {
            let reference = self
                .notebook
                .draft
                .borrow()
                .as_ref()
                .and_then(|d| d.references.iter().find(|r| r.id == id))
                .cloned();
            let Some(reference) = reference else {
                continue;
            };
            let preview = reference
                .passage
                .as_ref()
                .map(|p| p.quote.chars().take(150).collect::<String>())
                .unwrap_or_else(|| reference.locator.label());
            let label = gtk::Label::builder()
                .label(format!("{}\n{}", reference.label, preview))
                .wrap(true)
                .xalign(0.0)
                .max_width_chars(40)
                .build();
            let button = gtk::Button::builder().child(&label).build();
            let weak = Rc::downgrade(self);
            let weak_dialog = dialog.downgrade();
            button.connect_clicked(move |_| {
                if let Some(s) = weak.upgrade()
                    && s.is_current(generation)
                    && s.notebook.serial.get() == serial
                {
                    if let Some((start, end)) = range {
                        s.notebook_metadata(|d| d.connect(start, end, &id));
                    } else {
                        s.follow_saved_passage(&id);
                    }
                }
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.close();
                }
            });
            list.append(&button);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .max_content_height(350)
            .propagate_natural_height(true)
            .build();
        dialog.set_extra_child(Some(&scroll));
        dialog.present(Some(&self.window));
    }

    pub(in crate::desktop::shell) fn follow_saved_passage(self: &Rc<Self>, id: &str) {
        if !self.can_interact() {
            return;
        }
        let reference = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .and_then(|d| d.references.iter().find(|r| r.id == id))
            .cloned();
        let Some(reference) = reference else {
            return;
        };
        let Some(passage) = reference.passage.clone() else {
            self.goto(Destination::Locator(reference.locator), true);
            return;
        };
        let generation = self.generation();
        let request = self.notebook.passage_request.get() + 1;
        self.notebook.passage_request.set(request);
        let pending = match self.surface.borrow().as_ref() {
            Some(Surface::Pdf(pdf)) => {
                let result = pdf.resolve_passage(&passage);
                let pdf = Rc::clone(pdf);
                let weak = Rc::downgrade(self);
                glib::idle_add_local_once(move || {
                    let Some(s) = weak.upgrade() else {
                        return;
                    };
                    if !s.passage_request_current(generation, request, &reference) {
                        return;
                    }
                    match result {
                        Ok(resolved) => {
                            let recovered = resolved.recovered;
                            if let Some(current) = s.record_for_save().and_then(|r| r.locator) {
                                s.push_history(current);
                            }
                            pdf.show_passage(resolved);
                            s.notebook.unresolved.borrow_mut().remove(&reference.id);
                            s.sync_controls();
                            if recovered {
                                s.toast("Passage recovered from its quotation");
                            }
                        }
                        Err(_) => s.passage_needs_relink(reference),
                    }
                });
                return;
            }
            Some(Surface::Reflow(reader)) => reader
                .selection_request::<Resolved>("readeroResolvePassage", serde_json::json!(passage)),
            None => return,
        };
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = pending.await;
            let Some(s) = weak.upgrade() else { return; };
            if !s.passage_request_current(generation, request, &reference) { return; }
            match result {
                Ok(resolved) if resolved.capture.passage.validate(&resolved.capture.locator).is_ok() => {
                    if let Some(current) = s.record_for_save().and_then(|r| r.locator) { s.push_history(current); }
                    if let Some(Surface::Reflow(reader)) = s.surface.borrow().as_ref() {
                        reader.command(serde_json::json!({"type":"passage", "locator":resolved.capture.locator, "passage":resolved.capture.passage}));
                    }
                    s.notebook.unresolved.borrow_mut().remove(&reference.id);
                    s.sync_controls();
                    if resolved.recovered { s.toast("Passage recovered from its quotation and context"); }
                }
                _ => s.passage_needs_relink(reference),
            }
        });
    }

    fn passage_needs_relink(self: &Rc<Self>, reference: Reference) {
        self.notebook
            .unresolved
            .borrow_mut()
            .insert(reference.id.clone());
        let quote = reference
            .passage
            .as_ref()
            .map(|p| p.quote.chars().take(600).collect::<String>())
            .unwrap_or_default();
        let dialog = adw::AlertDialog::builder().heading("Passage needs relinking").body(format!("The saved words could not be identified confidently. Your quotation and connections are preserved.\n\n{quote}")).build();
        dialog.add_responses(&[
            ("cancel", "Keep passage"),
            ("location", "Open last known location"),
            ("relink", "Relink to selected text"),
        ]);
        let weak = Rc::downgrade(self);
        let generation = self.generation();
        dialog.connect_response(None, move |_, response| {
            if let Some(s) = weak.upgrade()
                && s.is_current(generation)
            {
                if response == "location" {
                    s.goto(Destination::Locator(reference.locator.clone()), true);
                    s.toast("Last known location; the saved words are not verified");
                }
                if response == "relink" {
                    s.notebook
                        .pending_relink
                        .replace(Some(reference.id.clone()));
                    s.notebook.capture.set_label("Relink selected passage…");
                    s.toast("Select the source text, then press Relink selected passage.");
                }
            }
        });
        dialog.present(Some(&self.window));
        self.render_references();
    }

    fn confirm_relink(self: &Rc<Self>, id: String, capture: Capture) {
        let previous = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .and_then(|d| d.references.iter().find(|r| r.id == id))
            .cloned();
        let Some(previous) = previous else {
            return;
        };
        let old = previous
            .passage
            .as_ref()
            .map(|p| p.quote.as_str())
            .unwrap_or("");
        let replaced = old != capture.passage.quote;
        let dialog = adw::AlertDialog::builder().heading(if replaced { "Replace the saved quotation?" } else { "Relink this passage?" }).body(format!("All connections to this passage will use the new selection.\n\nSaved: {}\n\nSelected: {}", old.chars().take(400).collect::<String>(), capture.passage.quote.chars().take(400).collect::<String>())).build();
        dialog.add_responses(&[
            ("cancel", "Cancel"),
            (
                "relink",
                if replaced {
                    "Replace passage with this selection"
                } else {
                    "Relink"
                },
            ),
        ]);
        let weak = Rc::downgrade(self);
        let generation = self.generation();
        dialog.connect_response(Some("relink"), move |_, _| {
            if let Some(s) = weak.upgrade()
                && s.is_current(generation)
            {
                if !s.notebook_metadata(|d| {
                    let current = d
                        .references
                        .iter_mut()
                        .find(|r| r.id == id)
                        .ok_or_else(|| Error::Invalid("This passage was removed.".into()))?;
                    if current.passage != previous.passage {
                        return Err(Error::Invalid(
                            "This passage changed. Select it again.".into(),
                        ));
                    }
                    current.locator = capture.locator.clone();
                    current.passage = Some(capture.passage.clone());
                    Ok(())
                }) {
                    return;
                }
                s.notebook.pending_relink.take();
                s.notebook.capture.set_label("Save selected passage");
                s.notebook.unresolved.borrow_mut().remove(&id);
                s.render_references();
            }
        });
        dialog.present(Some(&self.window));
    }

    pub(in crate::desktop::shell) fn delete_saved_passage(self: &Rc<Self>, id: &str) {
        let count = self.notebook.draft.borrow().as_ref().map_or(0, |d| {
            d.sections
                .iter()
                .filter(|s| s.passages.iter().any(|p| p == id))
                .count()
        });
        if count == 0 {
            self.notebook_metadata(|d| {
                d.remove_passage(id);
                Ok(())
            });
            self.render_references();
            return;
        }
        let dialog = adw::AlertDialog::builder().heading("Delete this saved passage?").body(format!("Disconnect it from {count} notebook section(s). Your notebook text will remain unchanged.")).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete and disconnect")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        let id = id.to_owned();
        let weak = Rc::downgrade(self);
        let generation = self.generation();
        dialog.connect_response(Some("delete"), move |_, _| {
            if let Some(s) = weak.upgrade()
                && s.is_current(generation)
            {
                s.notebook_metadata(|d| {
                    d.remove_passage(&id);
                    Ok(())
                });
                s.render_references();
            }
        });
        dialog.present(Some(&self.window));
    }

    pub(in crate::desktop::shell) fn passage_details(self: &Rc<Self>, id: &str) -> gtk::MenuButton {
        let button = gtk::MenuButton::builder()
            .icon_name("dialog-information-symbolic")
            .tooltip_text("Show saved quotation and source location")
            .build();
        button.update_property(&[gtk::accessible::Property::Label(
            "Show saved quotation and source location",
        )]);
        let id = id.to_owned();
        let weak = Rc::downgrade(self);
        button.set_create_popup_func(move |button| {
            if let Some(s) = weak.upgrade() {
                let draft = s.notebook.draft.borrow();
                if let Some(reference) = draft
                    .as_ref()
                    .and_then(|d| d.references.iter().find(|r| r.id == id))
                {
                    button.set_popover(Some(&s.passage_popover(reference)));
                }
            }
        });
        button
    }

    fn passage_popover(&self, reference: &Reference) -> gtk::Popover {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let location = text(&reference.locator.label(), "caption");
        location.set_wrap(true);
        location.set_max_width_chars(36);
        column.append(&location);
        if self.notebook.unresolved.borrow().contains(&reference.id) {
            column.append(&text("Needs relinking", "warning"));
        }
        if let Some(passage) = &reference.passage {
            let quote = gtk::TextView::builder()
                .editable(false)
                .cursor_visible(false)
                .wrap_mode(gtk::WrapMode::WordChar)
                .build();
            quote.buffer().set_text(&passage.quote);
            quote.update_property(&[gtk::accessible::Property::Label("Saved quotation")]);
            let scroll = gtk::ScrolledWindow::builder()
                .child(&quote)
                .min_content_width(240)
                .min_content_height(100)
                .max_content_height(240)
                .propagate_natural_height(true)
                .build();
            column.append(&scroll);
        } else {
            column.append(&text("Saved location · no quotation", "caption"));
        }
        gtk::Popover::builder().child(&column).build()
    }
}
