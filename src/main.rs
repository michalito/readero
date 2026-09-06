mod desktop;

use adw::prelude::*;
use gtk::{gdk, gio};
use std::{cell::RefCell, rc::Rc};

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder()
        .application_id("io.github.readero.Readero")
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_startup(|_| {
        papers_document::init();
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("../assets/style.css"));
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });
    let shell = Rc::new(RefCell::new(None::<Rc<desktop::shell::Shell>>));
    let active = Rc::clone(&shell);
    app.connect_activate(move |app| {
        let existing = active.borrow().clone();
        let current = existing.unwrap_or_else(|| {
            let created = desktop::shell::Shell::new(app);
            active.replace(Some(Rc::clone(&created)));
            created
        });
        current.window.present();
        #[cfg(feature = "smoke")]
        current.smoke();
    });
    app.connect_open(move |app, files, _| {
        let existing = shell.borrow().clone();
        let current = existing.unwrap_or_else(|| {
            let created = desktop::shell::Shell::new(app);
            shell.replace(Some(Rc::clone(&created)));
            created
        });
        current.window.present();
        if let Some(path) = files.first().and_then(gio::File::path) {
            current.open(path);
        }
    });
    app.run()
}
