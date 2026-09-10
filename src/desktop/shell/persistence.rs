use super::*;

impl Shell {
    pub(super) fn position(self: &Rc<Self>, locator: Locator, label: &str) {
        if !self.accepts_position() || locator.validate().is_err() {
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
        if !self.is_ready() {
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
        let pending = self.reading_state.save(self.record_for_save());
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let _ = pending.await;
            if let Some(s) = weak.upgrade() {
                s.refresh_storage();
            }
        });
    }

    pub(super) fn refresh_storage(self: &Rc<Self>) {
        let changed = self
            .current
            .borrow_mut()
            .as_mut()
            .is_some_and(|record| self.reading_state.reconcile(record));
        if changed && self.object::<gtk::DropDown>("sidebar_kind").selected() == 1 {
            self.show_bookmarks();
        }
        let status = self.reading_state.status();
        if let Some(error) = status.error {
            self.storage_error(&error);
        } else if !status.pending {
            self.object::<adw::Banner>("save_banner")
                .set_revealed(false);
        }
    }

    pub(super) fn storage_error(&self, message: &str) {
        let banner: adw::Banner = self.object("save_banner");
        banner.set_title(&format!("Progress is not being saved. {message}"));
        banner.set_revealed(true);
    }
}
