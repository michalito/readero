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
        let Some(record) = self.record_for_save() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let session_id = record.id.clone();
        let generation = self.generation.get();
        let pending = self.worker.call(move |store| store.save_session(&record));
        glib::MainContext::default().spawn_local(async move {
            let result = pending.await;
            if let Some(s) = weak.upgrade()
                && s.is_current(generation)
            {
                match result {
                    Ok(id) => {
                        s.reconnect_identity(&session_id, id);
                        s.object::<adw::Banner>("save_banner").set_revealed(false);
                    }
                    Err(error) => s.storage_error(&error.to_string()),
                }
            }
        });
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
                    if let Some(timer) = s.reload_timer.borrow_mut().take() {
                        timer.remove();
                    }
                    let weak = Rc::downgrade(&s);
                    let timer =
                        glib::timeout_add_local_once(Duration::from_millis(450), move || {
                            if let Some(s) = weak.upgrade() {
                                s.reload_timer.borrow_mut().take();
                                let path = s.current.borrow().as_ref().map(|r| r.path.clone());
                                if let Some(path) = path {
                                    s.reload(path);
                                }
                            }
                        });
                    s.reload_timer.replace(Some(timer));
                }
            });
            self.monitor.replace(Some(monitor));
        }
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
                worker.call(move |store| {
                if let Some(record) = record {
                    store.save_session(&record)?;
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
