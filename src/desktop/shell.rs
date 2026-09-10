use super::{
    pdf::{self, Pdf},
    reflow::{self, Reflow},
};
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use papers_document::prelude::*;
use papers_view::prelude::*;
use readero::{document::*, reading_state::ReadingState, resource::Publication};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[path = "probe.rs"]
#[cfg(feature = "smoke")]
mod probe;
#[path = "smoke.rs"]
#[cfg(feature = "smoke")]
mod smoke;

enum Surface {
    Pdf(Rc<Pdf>),
    Reflow(Reflow),
}
#[derive(Clone)]
enum Destination {
    Locator(Locator),
    Href(String),
    PdfResult(papers_view::SearchResult),
    PdfLink(papers_document::Link),
}

#[derive(Clone)]
struct NavigationItem {
    label: String,
    depth: u32,
    target: Destination,
}
#[derive(Clone, PartialEq)]
struct HistoryEntry {
    path: PathBuf,
    locator: Locator,
    settings: Settings,
}

pub struct Shell {
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    ui: gtk::Builder,
    reading_state: ReadingState,
    opening: opening::State,
    current: RefCell<Option<DocumentRecord>>,
    surface: RefCell<Option<Surface>>,
    toc: RefCell<Vec<NavigationItem>>,
    search_index: Cell<i32>,
    back: RefCell<Vec<HistoryEntry>>,
    forward: RefCell<Vec<HistoryEntry>>,
    save_timer: RefCell<Option<glib::SourceId>>,
    dirty_since: Cell<Option<Instant>>,
    focus: Cell<bool>,
    sidebar_before_focus: Cell<bool>,
    updating: Cell<bool>,
}

mod controls;
mod home;
mod navigation;
mod opening;
mod persistence;

impl Shell {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let ui = gtk::Builder::from_string(include_str!("../../assets/window.ui"));
        let window: adw::ApplicationWindow = ui.object("window").expect("window.ui defines window");
        window.set_application(Some(app));
        let data = std::env::var_os("READERO_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| glib::user_data_dir().join("readero"));
        let toasts = adw::ToastOverlay::new();
        let content = window.content();
        window.set_content(gtk::Widget::NONE);
        toasts.set_child(content.as_ref());
        window.set_content(Some(&toasts));
        let shell = Rc::new(Self {
            window,
            toasts,
            ui,
            reading_state: ReadingState::start(data.join("reading.sqlite3")),
            opening: opening::State::default(),
            current: RefCell::new(None),
            surface: RefCell::new(None),
            toc: RefCell::new(Vec::new()),
            search_index: Cell::new(-1),
            back: RefCell::new(Vec::new()),
            forward: RefCell::new(Vec::new()),
            save_timer: RefCell::new(None),
            dirty_since: Cell::new(None),
            focus: Cell::new(false),
            sidebar_before_focus: Cell::new(false),
            updating: Cell::new(false),
        });
        shell
            .object::<gtk::MenuButton>("appearance_button")
            .update_property(&[gtk::accessible::Property::Label("Reading appearance")]);
        shell.wire();
        shell.home();
        shell
    }
    fn object<T: glib::object::IsA<glib::Object>>(&self, id: &str) -> T {
        self.ui
            .object(id)
            .unwrap_or_else(|| panic!("window.ui defines {id}"))
    }

    fn button(&self, id: &str) -> gtk::Button {
        self.object(id)
    }

    fn label(&self, id: &str) -> gtk::Label {
        self.object(id)
    }

    fn content(&self) -> gtk::Stack {
        self.object("content")
    }

    fn sidebar(&self) -> gtk::Revealer {
        self.object("sidebar_revealer")
    }

    fn list(&self) -> gtk::ListBox {
        self.object("navigation_list")
    }

    fn entry(&self) -> gtk::SearchEntry {
        self.object("search_entry")
    }

    fn connect(self: &Rc<Self>, id: &str, action: impl Fn(&Rc<Self>) + 'static) {
        let weak = Rc::downgrade(self);
        self.button(id).connect_clicked(move |_| {
            if let Some(shell) = weak.upgrade() {
                action(&shell);
            }
        });
    }

    fn open_dialog(self: &Rc<Self>, relink: Option<String>) {
        let dialog = gtk::FileDialog::builder()
            .title(if relink.is_some() {
                "Locate your document"
            } else {
                "Open a document"
            })
            .modal(true)
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Reading documents"));
        for suffix in ["pdf", "epub", "md", "markdown"] {
            filter.add_suffix(suffix);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        filters.append(&all);
        dialog.set_filters(Some(&filters));
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(s), Ok(file)) = (weak.upgrade(), result)
                && let Some(path) = file.path()
            {
                if let Some(id) = relink {
                    let pending = s.reading_state.locate(id, path.clone());
                    glib::MainContext::default().spawn_local(async move {
                        let result = pending.await;
                        s.refresh_storage();
                        match result {
                            Ok(record) => s.open(record.path),
                            Err(error) => s.toast(&error.to_string()),
                        }
                    });
                } else {
                    s.open(path);
                }
            }
        });
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn collect_outline(model: &gio::ListModel, depth: u32, items: &mut Vec<NavigationItem>) {
    if depth > 20 {
        return;
    }
    for index in 0..model.n_items() {
        if items.len() >= 3000 {
            return;
        }
        if let Some(outline) = model
            .item(index)
            .and_downcast::<papers_document::Outlines>()
        {
            if let Some(link) = outline.link() {
                let label = outline
                    .label()
                    .or_else(|| link.title())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Untitled section".into());
                items.push(NavigationItem {
                    label,
                    depth,
                    target: Destination::PdfLink(link),
                });
            }
            if let Some(children) = outline.children() {
                collect_outline(&children, depth + 1, items);
            }
        }
    }
}
fn text(value: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(value)
        .xalign(0.0)
        .css_classes([class])
        .build()
}
fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .css_classes(["flat"])
        .build()
}

fn reflow_label(locator: &Locator, section: usize, total: usize) -> String {
    if total == 1
        && let Anchor::Reflow { fraction, .. } = locator.anchor
    {
        format!("{:.0}% read", fraction.clamp(0.0, 1.0) * 100.0)
    } else {
        format!("Section {section} of {total}")
    }
}
