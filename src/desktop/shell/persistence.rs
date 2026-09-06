use super::*;

impl Shell {
    pub(super) fn position(self: &Rc<Self>, locator: Locator, label: &str) {
        if !self.gate.borrow().accepts(self.generation.get()) || locator.validate().is_err() {
            return;
        }
        let changed = {
            let mut current = self.current.borrow_mut();
            if let Some(record) = current.as_mut() {
                if record.locator.as_ref() == Some(&locator) {
                    false
                } else {
                    record.locator = Some(locator);
                    true
                }
            } else {
                false
            }
        };
        self.label("position_label").set_text(label);
        if changed {
            self.schedule_save();
        }
    }

    pub(super) fn schedule_save(self: &Rc<Self>) {
        let first = self.dirty_since.get().unwrap_or_else(Instant::now);
        self.dirty_since.set(Some(first));
        if let Some(timer) = self.save_timer.borrow_mut().take() {
            timer.remove();
        }
        let delay =
            Duration::from_millis(750).min(Duration::from_secs(2).saturating_sub(first.elapsed()));
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(delay, move || {
            if let Some(s) = weak.upgrade() {
                s.save_timer.borrow_mut().take();
                s.save();
            }
        });
        self.save_timer.replace(Some(timer));
    }

    pub(super) fn record_for_save(&self) -> Option<DocumentRecord> {
        if !self.gate.borrow().accepts(self.generation.get()) {
            return None;
        }
        let mut record = self.current.borrow().clone()?;
        if let Some(Surface::Pdf(pdf)) = self.surface.borrow().as_ref() {
            record.locator = pdf.stable_location().or(record.locator);
        }
        Some(record)
    }

    pub(super) fn save(self: &Rc<Self>) {
        if let Some(timer) = self.save_timer.borrow_mut().take() {
            timer.remove();
        }
        self.dirty_since.set(None);
        if let Some(record) = self.record_for_save() {
            let serial = self.save_serial.get() + 1;
            self.save_serial.set(serial);
            self.unsaved
                .borrow_mut()
                .insert(record.path.clone(), (serial, record));
        }
        // Keep snapshots until acknowledged, even after navigation. Retry also
        // saves documents that are no longer the active reader.
        let records = self.unsaved.borrow().values().cloned().collect::<Vec<_>>();
        for (serial, record) in records {
            let weak = Rc::downgrade(self);
            let session_id = record.id.clone();
            let path = record.path.clone();
            let generation = self.generation.get();
            let pending = self.worker.call(move |store| store.save_session(&record));
            glib::MainContext::default().spawn_local(async move {
                let result = pending.await;
                let Some(s) = weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(id) => {
                        let mut unsaved = s.unsaved.borrow_mut();
                        if unsaved
                            .get(&path)
                            .is_some_and(|(latest, _)| *latest == serial)
                        {
                            unsaved.remove(&path);
                        }
                        let saved_all = unsaved.is_empty();
                        drop(unsaved);
                        if s.is_current(generation) {
                            s.reconnect_identity(&session_id, id);
                        }
                        if saved_all {
                            s.object::<adw::Banner>("save_banner").set_revealed(false);
                        }
                    }
                    Err(error) => s.storage_error(&error.to_string()),
                }
            });
        }
    }

    pub(super) fn reconnect_identity(self: &Rc<Self>, session_id: &str, id: String) {
        let mut changed = false;
        if let Some(record) = self.current.borrow_mut().as_mut()
            && record.id == session_id
            && record.id != id
        {
            record.id = id;
            changed = true;
        }
        if changed && self.object::<gtk::DropDown>("sidebar_kind").selected() == 1 {
            self.show_bookmarks();
        }
    }

    pub(super) fn storage_error(&self, message: &str) {
        let banner: adw::Banner = self.object("save_banner");
        banner.set_title(&format!("Progress is not being saved. {message}"));
        banner.set_revealed(true);
    }

    pub(super) fn watch_markdown(self: &Rc<Self>) {
        if self.monitor.borrow().is_some() {
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
            self.monitor.replace(Some(monitor));
        }
    }

    pub(super) fn schedule_reload(self: &Rc<Self>, path: PathBuf) {
        if let Some(timer) = self.reload_timer.borrow_mut().take() {
            timer.remove();
        }
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(Duration::from_millis(450), move || {
            if let Some(s) = weak.upgrade() {
                s.reload_timer.borrow_mut().take();
                if s.is_current(generation) {
                    s.reload(path);
                }
            }
        });
        self.reload_timer.replace(Some(timer));
    }

    pub(super) fn close(self: &Rc<Self>) {
        if self.closing.replace(true) {
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
        let mut records = self
            .unsaved
            .borrow()
            .iter()
            .map(|(path, (_, record))| (path.clone(), record.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let worker = self.worker.clone();
        let shell = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                if let Some(checkpoint) = checkpoint
                    && let Some(locator) = checkpoint.await?
                    && let Some(record) = record.as_mut()
                {
                    record.locator = Some(locator);
                }
                if let Some(record) = record {
                    records.insert(record.path.clone(), record);
                }
                worker.call(move |store| {
                    for record in records.values() {
                        store.save_session(record)?;
                    }
                    Ok(())
                }).await
            }.await;
            if let Err(error) = result {
                shell.closing.set(false);
                shell.storage_error(&error.to_string());
                let dialog = adw::AlertDialog::builder()
                    .heading("Your latest position wasn’t saved")
                    .body("You can keep reading and retry, or close using the last successfully saved position.")
                    .build();
                dialog.add_responses(&[("stay", "Keep open"), ("close", "Close anyway")]);
                dialog.set_close_response("stay");
                let weak = Rc::downgrade(&shell);
                dialog.connect_response(Some("close"), move |_, _| {
                    if let Some(s) = weak.upgrade() {
                        s.dispose_surface();
                        s.window.destroy();
                    }
                });
                dialog.present(Some(&shell.window));
            } else {
                shell.dispose_surface();
                shell.window.destroy();
            }
        });
    }
}
