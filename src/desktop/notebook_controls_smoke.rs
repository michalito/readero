//! Native delegated actions and delayed clipboard delivery in the isolated driver.
use super::*;
use glib::subclass::prelude::*;

mod delayed {
    use super::*;
    use gdk::subclass::prelude::*;
    #[derive(Default)]
    pub struct Provider {
        pub release: RefCell<Option<async_channel::Receiver<()>>>,
        pub started: Cell<bool>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Provider {
        const NAME: &'static str = "ReaderoSmokeDelayedClipboard";
        type Type = super::DelayedClipboard;
        type ParentType = gdk::ContentProvider;
    }
    impl ObjectImpl for Provider {}
    impl ContentProviderImpl for Provider {
        fn formats(&self) -> gdk::ContentFormats {
            gdk::ContentFormats::new(&["text/plain;charset=utf-8"])
        }
        fn write_mime_type_future(
            &self,
            _: &str,
            stream: &gio::OutputStream,
            priority: glib::Priority,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = std::result::Result<(), glib::Error>> + 'static>,
        > {
            self.started.set(true);
            let release = self.release.borrow().clone().unwrap();
            let stream = stream.clone();
            Box::pin(async move {
                let _ = release.recv().await;
                stream
                    .write_all_future(b"LATE CLIPBOARD".to_vec(), priority)
                    .await
                    .map(|_| ())
                    .map_err(|(_, error)| error)
            })
        }
    }
}
glib::wrapper! {
    pub struct DelayedClipboard(ObjectSubclass<delayed::Provider>) @extends gdk::ContentProvider;
}
impl DelayedClipboard {
    fn install(clipboard: &gdk::Clipboard) -> (Self, async_channel::Sender<()>) {
        let provider: Self = glib::Object::new();
        let (send, receive) = async_channel::bounded(1);
        *provider.imp().release.borrow_mut() = Some(receive);
        clipboard.set_content(Some(&provider)).unwrap();
        (provider, send)
    }
    async fn started(&self) -> bool {
        for _ in 0..100 {
            if self.imp().started.get() {
                return true;
            }
            settle(10).await;
        }
        false
    }
}

impl Shell {
    pub(super) async fn check_reference_controls(self: &Rc<Self>, id: &str) -> Value {
        let mut checks = json!({});
        let buffer = self.notebook.editor.buffer();
        let context = (
            buffer.cursor_position(),
            buffer.iter_at_mark(&buffer.selection_bound()).offset(),
        );
        let initial = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .unwrap()
            .references
            .clone();
        let name = self.notebook.reference_name(id).unwrap();
        name.grab_focus();
        let delegate = name.delegate().unwrap().downcast::<gtk::Text>().unwrap();
        delegate.insert_text(" native rename", &mut -1);
        let renamed = self
            .notebook
            .draft
            .borrow()
            .as_ref()
            .unwrap()
            .references
            .clone();
        checks["reference_delegate_edits_shared_model"] = json!(renamed != initial);
        let keys = name.observe_controllers();
        let keyboard = (0..keys.n_items())
            .filter_map(|i| keys.item(i))
            .find_map(|object| object.downcast::<gtk::EventControllerKey>().ok())
            .unwrap();
        keyboard.emit_by_name::<bool>(
            "key-pressed",
            &[&gdk::Key::z, &0u32, &gdk::ModifierType::CONTROL_MASK],
        );
        checks["reference_keyboard_undo_uses_shared_history"] =
            json!(self.notebook.draft.borrow().as_ref().unwrap().references == initial);
        let rebuilt = self.notebook.reference_name(id).unwrap();
        checks["reference_undo_preserves_editing_focus"] = json!(
            gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|focus| focus == rebuilt || focus.is_ancestor(&rebuilt))
        );
        let keys = rebuilt.observe_controllers();
        let keyboard = (0..keys.n_items())
            .filter_map(|i| keys.item(i))
            .find_map(|object| object.downcast::<gtk::EventControllerKey>().ok())
            .unwrap();
        keyboard.emit_by_name::<bool>(
            "key-pressed",
            &[
                &gdk::Key::Z,
                &0u32,
                &(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK),
            ],
        );
        checks["reference_keyboard_redo_uses_shared_history"] =
            json!(self.notebook.draft.borrow().as_ref().unwrap().references == renamed);
        // Activate the menu's actual action from the native GtkText delegate.
        let current = self.notebook.reference_name(id).unwrap();
        let menu = current.extra_menu().unwrap();
        let action = menu
            .item_attribute_value(0, "action", None)
            .unwrap()
            .get::<String>()
            .unwrap();
        let text = current.delegate().unwrap().downcast::<gtk::Text>().unwrap();
        text.activate_action(&action, None).unwrap();
        checks["reference_context_undo_uses_shared_history"] =
            json!(self.notebook.draft.borrow().as_ref().unwrap().references == initial);
        let current = self.notebook.reference_name(id).unwrap();
        let action = current
            .extra_menu()
            .unwrap()
            .item_attribute_value(1, "action", None)
            .unwrap()
            .get::<String>()
            .unwrap();
        current
            .delegate()
            .unwrap()
            .downcast::<gtk::Text>()
            .unwrap()
            .activate_action(&action, None)
            .unwrap();
        checks["reference_context_redo_uses_shared_history"] =
            json!(self.notebook.draft.borrow().as_ref().unwrap().references == renamed);
        self.notebook_undo(false);
        settle(150).await;
        let current = self.notebook.reference_name(id).unwrap();
        current.grab_focus();
        let text = current.delegate().unwrap().downcast::<gtk::Text>().unwrap();
        let clipboard = text.clipboard();
        let (provider, release) = DelayedClipboard::install(&clipboard);
        settle(50).await;
        // The native Ctrl+V action signal starts GTK's asynchronous paste.
        text.emit_by_name::<()>("paste-clipboard", &[]);
        checks["reference_delayed_native_paste_started"] = json!(provider.started().await);
        self.render_references();
        release.send(()).await.unwrap();
        settle(100).await;
        checks["retired_reference_paste_preserves_model_and_redo"] = json!(
            self.notebook.draft.borrow().as_ref().unwrap().references == initial
                && self.notebook.history.borrow().can_redo()
        );
        let current = self.notebook.reference_name(id).unwrap();
        let text = current.delegate().unwrap().downcast::<gtk::Text>().unwrap();
        let (provider, release) = DelayedClipboard::install(&clipboard);
        settle(50).await;
        // The native Ctrl+V action signal starts GTK's asynchronous paste.
        text.emit_by_name::<()>("paste-clipboard", &[]);
        checks["reference_modal_paste_started"] = json!(provider.started().await);
        let dialog = adw::AlertDialog::builder()
            .heading("Clipboard guard fixture")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.present(Some(&self.window));
        release.send(()).await.unwrap();
        settle(100).await;
        checks["reference_modal_paste_is_rejected"] = json!(
            self.notebook.draft.borrow().as_ref().unwrap().references == initial
                && current.text() == initial.iter().find(|r| r.id == id).unwrap().label
        );
        dialog.close();
        settle(150).await;
        clipboard.set_text("");
        self.notebook.editor.grab_focus();
        buffer.select_range(
            &buffer.iter_at_offset(context.0),
            &buffer.iter_at_offset(context.1),
        );
        checks
    }

    pub(super) async fn check_clipboard_sessions(
        self: &Rc<Self>,
        directory: &std::path::Path,
    ) -> Value {
        let mut checks = json!({});
        let record = self.current.borrow().clone().unwrap();
        let buffer = self.notebook.editor.buffer();
        let before = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        let (_provider, release) = self.begin_delayed_body_paste().await;
        let dialog = adw::AlertDialog::builder()
            .heading("Body clipboard guard fixture")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.present(Some(&self.window));
        release.send(()).await.unwrap();
        settle(100).await;
        checks["body_modal_clipboard_cannot_edit_buffer_or_model"] = json!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), true) == before
                && self.notebook.draft.borrow().as_ref().unwrap().text == before
        );
        dialog.close();
        settle(150).await;
        for mode in ["keyboard", "menu", "primary"] {
            if !self.notebook.panel.is_visible() {
                self.toggle_notebook();
            }
            settle(100).await;
            self.refresh_notebook_actions();
            let old = self.notebook.editor.buffer();
            let before = old.text(&old.start_iter(), &old.end_iter(), true);
            let clipboard = if mode == "primary" {
                self.notebook.editor.primary_clipboard()
            } else {
                self.notebook.editor.clipboard()
            };
            let (provider, release) = DelayedClipboard::install(&clipboard);
            match mode {
                "keyboard" => self
                    .notebook
                    .editor
                    .emit_by_name::<()>("paste-clipboard", &[]),
                "menu" => self
                    .notebook
                    .editor
                    .activate_action("notebook.paste", None)
                    .unwrap(),
                _ => old.paste_clipboard(&clipboard, None, true),
            }
            checks[format!("{mode}_delayed_clipboard_started")] = json!(provider.started().await);
            // The same activation path used for a document switch installs a
            // different notebook, including a fresh buffer, before delivery.
            let other = directory.join(format!("clipboard-{mode}.md"));
            std::fs::write(
                &other,
                format!("# Clipboard {mode}\n\nSeparate notebook fixture."),
            )
            .unwrap();
            self.open(other);
            self.wait_ready().await;
            self.wait_notebook().await;
            let fresh = self.notebook.editor.buffer();
            release.send(()).await.unwrap();
            settle(100).await;
            checks[format!("{mode}_late_paste_cannot_mutate_new_document")] = json!(
                old != fresh
                    && fresh.char_count() == 0
                    && self
                        .notebook
                        .draft
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .text
                        .is_empty()
            );
            checks[format!("{mode}_retired_buffer_rejects_delivery")] =
                json!(old.text(&old.start_iter(), &old.end_iter(), true) == before);
            self.open(record.path.clone());
            self.wait_ready().await;
            self.wait_notebook().await;
            clipboard.set_text("");
        }
        checks
    }

    pub(super) async fn begin_delayed_body_paste(
        &self,
    ) -> (DelayedClipboard, async_channel::Sender<()>) {
        let result = DelayedClipboard::install(&self.notebook.editor.clipboard());
        self.notebook
            .editor
            .emit_by_name::<()>("paste-clipboard", &[]);
        assert!(result.0.started().await, "delayed paste did not start");
        result
    }
}
