use super::*;

impl Shell {
    pub(in crate::desktop::shell) fn wire_notebook_editing(self: &Rc<Self>) {
        self.wire_notebook_buffer();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(s) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if state.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gdk::Key::z | gdk::Key::Z)
            {
                s.notebook_undo(state.contains(gdk::ModifierType::SHIFT_MASK));
            } else if key == gdk::Key::Menu
                || (key == gdk::Key::F10 && state.contains(gdk::ModifierType::SHIFT_MASK))
            {
                s.notebook_menu(30.0, 30.0);
            } else if key == gdk::Key::Escape && s.cancel_pending_connection() {
            } else {
                return glib::Propagation::Proceed;
            }
            glib::Propagation::Stop
        });
        self.notebook.editor.add_controller(keys);
        for name in [
            "undo", "redo", "cut", "copy", "paste", "all", "connect", "open",
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(s) = weak.upgrade() {
                    let buffer = s.notebook.editor.buffer();
                    let clipboard = s.notebook.editor.clipboard();
                    match name {
                        "undo" => s.notebook_undo(false),
                        "redo" => s.notebook_undo(true),
                        "cut" => buffer.cut_clipboard(&clipboard, true),
                        "copy" => buffer.copy_clipboard(&clipboard),
                        "paste" => buffer.paste_clipboard(&clipboard, None, true),
                        "all" => buffer.select_range(&buffer.start_iter(), &buffer.end_iter()),
                        "connect" => s.connect_notebook_selection(),
                        "open" => s.follow_notebook_section(buffer.cursor_position()),
                        _ => unreachable!(),
                    }
                }
            });
            self.notebook.actions.add_action(&action);
        }
        self.notebook
            .panel
            .insert_action_group("notebook", Some(&self.notebook.actions));
        self.notebook
            .editor
            .insert_action_group("notebook", Some(&self.notebook.actions));
        let menu = gtk::GestureClick::new();
        menu.set_button(3);
        menu.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        menu.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(s) = weak.upgrade() {
                s.notebook_menu(x, y);
            }
        });
        self.notebook.editor.add_controller(menu);
        let follow = gtk::GestureClick::new();
        follow.set_button(1);
        follow.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        follow.connect_pressed(move |gesture, _, x, y| {
            if gesture
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
                && let Some(s) = weak.upgrade()
            {
                let (x, y) = s.notebook.editor.window_to_buffer_coords(
                    gtk::TextWindowType::Widget,
                    x as i32,
                    y as i32,
                );
                if let Some(iter) = s.notebook.editor.iter_at_location(x, y) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    s.follow_notebook_section(iter.offset());
                }
            }
        });
        self.notebook.editor.add_controller(follow);
        for adjustment in [
            self.notebook.scroll.vadjustment(),
            self.notebook.scroll.hadjustment(),
        ] {
            let weak = Rc::downgrade(self);
            adjustment.connect_changed(move |_| {
                if let Some(s) = weak.upgrade() {
                    s.queue_connections();
                }
            });
        }
    }

    pub(in crate::desktop::shell) fn wire_notebook_buffer(self: &Rc<Self>) {
        let buffer = self.notebook.editor.buffer();
        let weak = Rc::downgrade(self);
        buffer.connect_begin_user_action(move |buffer| {
            if let Some(s) = weak.upgrade()
                && s.notebook.editor.buffer() == *buffer
                && !s.notebook.suppress.get()
                && s.can_interact()
                && s.notebook.editor.is_editable()
            {
                s.notebook.capture_selection();
                if let Some(n) = s.notebook.draft.borrow().as_ref() {
                    s.notebook.history.borrow_mut().begin(n);
                }
            }
        });
        let weak = Rc::downgrade(self);
        buffer.connect_end_user_action(move |buffer| {
            if let Some(s) = weak.upgrade()
                && s.notebook.editor.buffer() == *buffer
                && !s.notebook.suppress.get()
                && s.can_interact()
                && s.notebook.editor.is_editable()
            {
                s.notebook.capture_selection();
                if let Some(n) = s.notebook.draft.borrow().as_ref() {
                    s.notebook.history.borrow_mut().end(n);
                }
                s.queue_connections();
            }
        });
        let weak = Rc::downgrade(self);
        buffer.connect_insert_text(move |buffer, at, text| {
            if let Some(s) = weak.upgrade()
                && (s.notebook.editor.buffer() != *buffer
                    || (!s.notebook.suppress.get()
                        && (!s.notebook_accepts_input()
                            || !s.record_notebook_edit(at.offset(), at.offset(), text))))
            {
                buffer.stop_signal_emission_by_name("insert-text");
            }
        });
        let weak = Rc::downgrade(self);
        buffer.connect_delete_range(move |buffer, start, end| {
            if let Some(s) = weak.upgrade()
                && (s.notebook.editor.buffer() != *buffer
                    || (!s.notebook.suppress.get()
                        && (!s.notebook_accepts_input()
                            || !s.record_notebook_edit(start.offset(), end.offset(), ""))))
            {
                buffer.stop_signal_emission_by_name("delete-range");
            }
        });
        let weak = Rc::downgrade(self);
        buffer.connect_changed(move |buffer| {
            if let Some(s) = weak.upgrade()
                && s.notebook.editor.buffer() == *buffer
                && s.can_interact()
                && !s.notebook.suppress.get()
            {
                s.notebook.cancel_restore();
                s.notebook_changed();
                s.queue_connections();
            }
        });
        let weak = Rc::downgrade(self);
        buffer.connect_mark_set(move |buffer, _, mark| {
            if mark
                .name()
                .is_some_and(|name| name == "insert" || name == "selection_bound")
                && let Some(s) = weak.upgrade()
                && s.notebook.editor.is_mapped()
                && s.notebook.editor.buffer() == *buffer
                && s.can_interact()
                && !s.notebook.suppress.get()
            {
                s.notebook.cancel_restore();
                s.notebook_changed();
                s.queue_connections();
            }
        });
    }

    pub(in crate::desktop::shell) fn notebook_accepts_input(&self) -> bool {
        self.can_interact()
            && self.notebook.editor.is_editable()
            && self.window.visible_dialog().is_none()
    }

    pub(in crate::desktop::shell) fn wire_reference_name(self: &Rc<Self>, name: &gtk::Entry) {
        name.set_enable_undo(false);
        let menu = gio::Menu::new();
        menu.append(Some("Undo notebook change"), Some("notebook.undo"));
        menu.append(Some("Redo notebook change"), Some("notebook.redo"));
        name.set_extra_menu(Some(&menu));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            if state.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gdk::Key::z | gdk::Key::Z)
                && let Some(s) = weak.upgrade()
            {
                s.notebook_undo(state.contains(gdk::ModifierType::SHIFT_MASK));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        name.add_controller(keys);
        let serial = self.notebook.serial.get();
        // Native typing and clipboard delivery originate in GtkText. Editable
        // signals propagate wrapper -> delegate, never delegate -> wrapper.
        let delegate = name.delegate().expect("GtkEntry editable delegate");
        let weak_name = name.downgrade();
        let weak = Rc::downgrade(self);
        delegate.connect_insert_text(move |editable, _, _| {
            if let Some(s) = weak.upgrade()
                && !weak_name
                    .upgrade()
                    .is_some_and(|entry| s.reference_name_accepts_input(&entry, serial))
            {
                editable.stop_signal_emission_by_name("insert-text");
            }
        });
        let weak_name = name.downgrade();
        let weak = Rc::downgrade(self);
        delegate.connect_delete_text(move |editable, _, _| {
            if let Some(s) = weak.upgrade()
                && !weak_name
                    .upgrade()
                    .is_some_and(|entry| s.reference_name_accepts_input(&entry, serial))
            {
                editable.stop_signal_emission_by_name("delete-text");
            }
        });
    }

    pub(in crate::desktop::shell) fn reference_name_accepts_input(
        &self,
        entry: &gtk::Entry,
        serial: u64,
    ) -> bool {
        self.notebook.serial.get() == serial
            && self.notebook_accepts_input()
            && self
                .notebook
                .reference_names
                .borrow()
                .iter()
                .any(|(_, live)| live == entry)
    }

    fn record_notebook_edit(&self, start: i32, end: i32, text: &str) -> bool {
        self.notebook.tags_dirty.set(true);
        let buffer = self.notebook.editor.buffer();
        if end > start
            && start <= buffer.iter_at_mark(&self.notebook.tag_start).offset()
            && end >= buffer.iter_at_mark(&self.notebook.tag_end).offset()
        {
            self.notebook.tags_present.set(false);
        }
        self.notebook.capture_selection();
        let result = if let Some(n) = self.notebook.draft.borrow_mut().as_mut() {
            self.notebook.history.borrow_mut().edit(n, start, end, text)
        } else {
            return false;
        };
        if let Err(error) = result {
            self.toast(&error.to_string());
            return false;
        }
        true
    }

    pub(in crate::desktop::shell) fn notebook_undo(self: &Rc<Self>, redo: bool) {
        let n = &self.notebook;
        if !n.editor.is_editable() || !self.can_interact() {
            return;
        }
        if !(if redo {
            n.history.borrow().can_redo()
        } else {
            n.history.borrow().can_undo()
        }) {
            return;
        }
        let Some((edits, cursor, selection)) = n.draft.borrow_mut().as_mut().map(|draft| {
            let edits = n.history.borrow_mut().undo(draft, redo);
            (edits, draft.cursor, draft.selection)
        }) else {
            return;
        };
        n.tags_dirty.set(true);
        n.cancel_restore();
        n.suppress.set(true);
        let buffer = n.editor.buffer();
        for edit in edits {
            buffer.delete(
                &mut buffer.iter_at_offset(edit.start),
                &mut buffer.iter_at_offset(edit.start + edit.removed),
            );
            buffer.insert(&mut buffer.iter_at_offset(edit.start), &edit.text);
        }
        buffer.select_range(
            &buffer.iter_at_offset(cursor),
            &buffer.iter_at_offset(selection),
        );
        n.suppress.set(false);
        self.notebook_changed();
        self.render_references();
        self.queue_connections();
    }

    fn notebook_menu(self: &Rc<Self>, x: f64, y: f64) {
        self.refresh_notebook_actions();
        let menu = gio::Menu::new();
        for (label, name) in [
            ("Undo notebook change", "undo"),
            ("Redo notebook change", "redo"),
            ("Cut", "cut"),
            ("Copy", "copy"),
            ("Paste", "paste"),
            ("Select all", "all"),
            ("Connect passage…", "connect"),
            ("Open connected passage", "open"),
        ] {
            menu.append(Some(label), Some(&format!("notebook.{name}")));
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&self.notebook.editor);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.connect_closed(|p| p.unparent());
        popover.popup();
    }

    pub(in crate::desktop::shell) fn refresh_notebook_actions(&self) {
        let n = &self.notebook;
        let editable = n.editor.is_editable() && self.can_interact();
        let buffer = n.editor.buffer();
        let linked = n
            .draft
            .borrow()
            .as_ref()
            .is_some_and(|d| d.section_at(buffer.cursor_position()).is_some());
        for (name, enabled) in [
            ("undo", editable && n.history.borrow().can_undo()),
            ("redo", editable && n.history.borrow().can_redo()),
            ("cut", editable && buffer.has_selection()),
            ("copy", buffer.has_selection()),
            ("paste", editable),
            ("all", buffer.char_count() > 0),
            ("connect", editable && buffer.char_count() > 0),
            ("open", linked && self.can_interact()),
        ] {
            if let Some(action) = n
                .actions
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                action.set_enabled(enabled);
            }
        }
    }

    pub(in crate::desktop::shell) fn notebook_metadata(
        self: &Rc<Self>,
        change: impl FnOnce(&mut Notebook) -> readero::document::Result<()>,
    ) -> bool {
        self.notebook.tags_dirty.set(true);
        self.notebook.capture_selection();
        let result = {
            let mut draft = self.notebook.draft.borrow_mut();
            let Some(n) = draft.as_mut() else {
                return false;
            };
            self.notebook.history.borrow_mut().metadata(n, change)
        };
        if let Err(error) = result {
            self.toast(&error.to_string());
            return false;
        }
        self.notebook_changed();
        self.queue_connections();
        self.refresh_notebook_actions();
        true
    }

    pub(in crate::desktop::shell) fn queue_connections(self: &Rc<Self>) {
        if self.notebook.decorations.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.notebook
            .decorations
            .replace(Some(glib::idle_add_local_full(
                decoration_priority(),
                move || {
                    if let Some(s) = weak.upgrade() {
                        s.notebook.decorations.take();
                        s.render_connections();
                    }
                    glib::ControlFlow::Break
                },
            )));
    }

    fn render_connections(self: &Rc<Self>) {
        let n = &self.notebook;
        self.refresh_notebook_actions();

        let buffer = n.editor.buffer();
        let tag = buffer.tag_table().lookup("passage-connection").unwrap();

        let (sections, active) = n.draft.borrow().as_ref().map_or_else(
            || (Vec::new(), None),
            |d| {
                (
                    d.sections.clone(),
                    d.section_at(buffer.cursor_position()).cloned(),
                )
            },
        );
        let mut visible_ids = std::collections::HashSet::new();
        let pending = n.pending_passage.borrow().is_some();
        let label = if pending {
            "Connect notes"
        } else {
            "Connect…"
        };
        if n.connect.label().as_deref() != Some(label) {
            n.connect.set_label(label);
            n.connect.set_tooltip_text(Some(if pending {
                "Connect selected notes; Escape cancels"
            } else {
                "Connect selected notes to a saved passage"
            }));
            if pending {
                n.connect.add_css_class("suggested-action");
            } else {
                n.connect.remove_css_class("suggested-action");
            }
        }
        if n.panel.is_visible() && n.editor.is_mapped() && !sections.is_empty() {
            let visible = n.editor.visible_rect();
            let first = n.top();
            let mut bottom = n
                .editor
                .iter_at_location(
                    visible.x() + visible.width() / 2,
                    visible.y() + visible.height() - 1,
                )
                .unwrap_or_else(|| n.editor.line_at_y(visible.y() + visible.height()).0);
            n.editor.forward_display_line(&mut bottom);
            let last = bottom.offset().max(first);
            let old_start = buffer.iter_at_mark(&n.tag_start);
            let old_end = buffer.iter_at_mark(&n.tag_end);
            // GTK tags track text edits, but recoloring every section forces
            // offscreen paragraph layout. Keep decorations in the viewport;
            // marks let us clear the previous viewport after intervening edits.
            if n.tags_dirty.replace(false)
                || old_start.offset() != first
                || old_end.offset() != last
            {
                if n.tags_present.replace(false) {
                    buffer.remove_tag(&tag, &old_start, &old_end);
                }
                buffer.move_mark(&n.tag_start, &buffer.iter_at_offset(first));
                buffer.move_mark(&n.tag_end, &buffer.iter_at_offset(last));
                let start = sections.partition_point(|section| section.end <= first);
                for section in sections
                    .iter()
                    .skip(start)
                    .take_while(|section| section.start < last)
                {
                    n.tags_present.set(true);
                    buffer.apply_tag(
                        &tag,
                        &buffer.iter_at_offset(section.start.max(first)),
                        &buffer.iter_at_offset(section.end.min(last)),
                    );
                }
            }
            let start = sections.partition_point(|section| section.end <= first);
            let mut lines: Vec<(String, i32, Vec<String>)> = Vec::new();
            for section in sections
                .into_iter()
                .skip(start)
                .take_while(|section| section.start < last)
            {
                let start = n
                    .editor
                    .iter_location(&buffer.iter_at_offset(section.start.max(first)));
                let end = n
                    .editor
                    .iter_location(&buffer.iter_at_offset(section.end.min(last) - 1));
                if end.y() < visible.y() || start.y() > visible.y() + visible.height() {
                    continue;
                }
                let y = n
                    .editor
                    .buffer_to_window_coords(
                        gtk::TextWindowType::Widget,
                        0,
                        start.y().max(visible.y()),
                    )
                    .1;
                // Several distinct selections can share a displayed line.
                // Give that line one chooser rather than overlapping buttons.
                if let Some((_, _, passages)) = lines.last_mut().filter(|(_, at, _)| *at == y) {
                    for id in section.passages {
                        if !passages.contains(&id) {
                            passages.push(id);
                        }
                    }
                } else {
                    lines.push((section.id, y, section.passages));
                }
            }
            for (id, y, passages) in lines {
                visible_ids.insert(id.clone());
                let mut buttons = n.source_buttons.borrow_mut();
                let source = buttons.entry(id).or_insert_with(|| {
                    let button = icon_button("go-jump-symbolic", "Open connected passage");
                    button.add_css_class("flat");
                    button.add_css_class("passage-source");
                    button.set_focus_on_click(false);
                    let weak = Rc::downgrade(self);
                    let targets = Rc::new(RefCell::new(Vec::new()));
                    let clicked_targets = Rc::clone(&targets);
                    button.connect_clicked(move |_| {
                        if let Some(s) = weak.upgrade() {
                            s.follow_passage_ids(clicked_targets.borrow().clone());
                        }
                    });
                    n.gutter.put(&button, 0.0, f64::from(y));
                    SourceButton {
                        widget: button,
                        passages: targets,
                    }
                });
                let button = &source.widget;
                let label = if passages.len() == 1 {
                    n.draft
                        .borrow()
                        .as_ref()
                        .and_then(|draft| draft.references.iter().find(|r| r.id == passages[0]))
                        .map(|reference| {
                            format!(
                                "Open connected passage: {} — {}",
                                reference.label,
                                reference.locator.label()
                            )
                        })
                        .unwrap_or_else(|| "Open connected passage".into())
                } else {
                    format!("Choose from {} connected passages", passages.len())
                };
                if button.tooltip_text().as_deref() != Some(&label) {
                    button.set_tooltip_text(Some(&label));
                    button.update_property(&[gtk::accessible::Property::Label(&label)]);
                }
                source.passages.replace(passages);
                if n.gutter.child_position(button) != (0.0, f64::from(y)) {
                    n.gutter.move_(button, 0.0, f64::from(y));
                }
            }
        }
        if visible_ids.is_empty() && n.tags_dirty.replace(false) && n.tags_present.replace(false) {
            buffer.remove_tag(
                &tag,
                &buffer.iter_at_mark(&n.tag_start),
                &buffer.iter_at_mark(&n.tag_end),
            );
        }
        n.source_buttons.borrow_mut().retain(|id, source| {
            if visible_ids.contains(id) {
                true
            } else {
                n.gutter.remove(&source.widget);
                false
            }
        });
        let active_key = active.as_ref().map(|section| {
            let draft = n.draft.borrow();
            let labels = section
                .passages
                .iter()
                .filter_map(|id| {
                    draft
                        .as_ref()?
                        .references
                        .iter()
                        .find(|r| &r.id == id)
                        .map(|r| (r.id.clone(), r.label.clone()))
                })
                .collect();
            ActiveConnections {
                section: section.id.clone(),
                passages: labels,
            }
        });
        if *n.active_connections.borrow() == active_key {
            return;
        }
        n.active_connections.replace(active_key);
        while let Some(child) = n.connection_bar.first_child() {
            n.connection_bar.remove(&child);
        }
        n.connection_scroll.set_visible(active.is_some());
        if let Some(section) = active {
            n.connection_bar
                .append(&text("Passages for this section", "caption"));
            for id in section.passages {
                let reference = n
                    .draft
                    .borrow()
                    .as_ref()
                    .and_then(|d| d.references.iter().find(|r| r.id == id))
                    .cloned();
                let Some(reference) = reference else {
                    continue;
                };
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
                let label = gtk::Label::builder()
                    .label(&reference.label)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .max_width_chars(24)
                    .build();
                let open = gtk::Button::builder()
                    .child(&label)
                    .tooltip_text(&reference.label)
                    .build();
                open.set_focus_on_click(false);
                open.set_hexpand(true);
                let weak = Rc::downgrade(self);
                let target = id.clone();
                open.connect_clicked(move |_| {
                    if let Some(s) = weak.upgrade() {
                        s.follow_saved_passage(&target);
                    }
                });
                row.append(&open);
                let remove = icon_button(
                    "edit-clear-symbolic",
                    "Remove connection; keep notes and passage",
                );
                let weak = Rc::downgrade(self);
                let section_id = section.id.clone();
                remove.connect_clicked(move |_| {
                    if let Some(s) = weak.upgrade() {
                        s.notebook_metadata(|d| {
                            d.disconnect(&section_id, &id);
                            Ok(())
                        });
                    }
                });
                row.append(&remove);
                n.connection_bar.append(&row);
            }
        }
    }
}
