//! Exercise missing renderer completions through the real opening lifecycle.
use super::*;
use std::path::Path;

impl Shell {
    pub(super) async fn check_startup_recovery(self: &Rc<Self>, directory: &Path) -> Value {
        let original = self.record_for_save().expect("ready before startup checks");
        let other = directory.join("startup-candidate.md");
        std::fs::write(&other, "# Delayed source\n\nA separate document.").unwrap();
        self.wait_notebook().await;
        self.notebook
            .editor
            .buffer()
            .set_text("Keep my notes → café");
        self.queue_notebook();
        let mut checks = json!({});

        // The real renderer is silent longer than the production deadline.
        // A red run explicitly cancels it afterward so later checks still run.
        for cold in [false, true] {
            if cold {
                self.home();
            }
            let reader = self.content().visible_child();
            reflow::delay_next_start(60_000);
            self.open(other.clone());
            checks[format!("stalled_candidate_mapped_{cold}")] = json!(self.wait_candidate().await);
            for _ in 0..320 {
                if !self.is_opening() {
                    break;
                }
                settle(100).await;
            }
            checks[format!("silent_renderer_open_finishes_{cold}")] = json!(!self.is_opening());
            checks[format!("silent_renderer_failure_explained_{cold}")] = json!(has_label(
                &self.window,
                "The document took too long to open"
            ));
            checks[format!("silent_renderer_controls_recover_{cold}")] = json!(self.can_interact());
            if cold {
                checks["cold_timeout_shows_error"] = json!(
                    !self.is_ready()
                        && self.record_for_save().is_none()
                        && self.content().visible_child_name().as_deref() == Some("status")
                );
            } else {
                checks["timeout_preserves_reader_and_notes"] = json!(
                    self.content().visible_child() == reader
                        && self.record_for_save().is_some_and(|r| r.id == original.id
                            && r.locator == original.locator
                            && r.settings == original.settings)
                        && self
                            .notebook
                            .draft
                            .borrow()
                            .as_ref()
                            .is_some_and(|n| n.text == "Keep my notes → café")
                );
            }
            self.cancel_open();
            self.open(original.path.clone());
            self.wait_ready().await;
            checks[format!("retry_after_timeout_succeeds_{cold}")] = json!(
                !self.is_opening()
                    && self.is_ready()
                    && self.record_for_save().is_some_and(|r| r.id == original.id)
            );
        }

        // Supersession and Home must dispose their deadline, including when
        // a successful new reader has already replaced the original one.
        for home in [false, true] {
            reflow::delay_next_start(60_000);
            self.open(other.clone());
            checks[format!("cancelled_candidate_mapped_{home}")] =
                json!(self.wait_candidate().await);
            if home {
                self.home();
            }
            self.open(original.path.clone());
            self.wait_ready().await;
        }
        let generation = self.generation();
        settle(31_000).await;
        checks["expired_old_deadlines_cannot_replace_reader"] = json!(
            !self.is_opening()
                && self.is_ready()
                && self.generation() == generation
                && self.can_interact()
                && self.record_for_save().is_some_and(|r| r.id == original.id)
        );
        checks["cancelled_startup_shows_no_spurious_error"] = json!(!has_label(
            &self.window,
            "The document took too long to open"
        ));
        checks
    }
}
