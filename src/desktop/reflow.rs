use gtk::{gio, glib, prelude::*};
use readero::{
    document::*,
    resource::{self, Publication},
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use webkit6::{self as webkit, prelude::*};

pub const WORLD: &str = "readero-private-v1";
const POLICY: &str = "default-src 'none'; script-src readero:; style-src 'unsafe-inline' readero:; img-src blob: data: readero:; font-src blob: data: readero:; connect-src readero:; frame-src 'self' blob:; object-src 'none'; base-uri 'none'; form-action 'none'";

#[derive(Clone, Deserialize)]
pub struct TocItem {
    pub label: String,
    pub href: String,
    #[serde(default)]
    pub depth: u32,
}
#[derive(Clone, Deserialize)]
pub struct SearchItem {
    pub label: String,
    pub locator: Locator,
}
#[derive(Deserialize)]
pub struct Message {
    pub generation: u64,
    #[serde(flatten)]
    pub event: Event,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
    Ready {
        title: String,
        toc: Vec<TocItem>,
        locator: Option<Locator>,
        total: usize,
    },
    Location {
        locator: Locator,
        section: usize,
        total: usize,
    },
    Search {
        query: String,
        items: Vec<SearchItem>,
        complete: bool,
    },
    Jump {
        locator: Option<Locator>,
    },
    External {
        href: String,
    },
    Related {
        href: String,
    },
    Error {
        message: String,
    },
    Notice {
        message: String,
    },
    Escape {
        handled: bool,
    },
}
#[derive(Serialize)]
struct Config<'a> {
    host: &'a str,
    generation: u64,
    entries: &'a [resource::ResourceInfo],
    settings: &'a Settings,
    locator: &'a Option<Locator>,
    format: Format,
    title: &'a str,
    changed: bool,
}

pub struct Reflow {
    pub view: webkit::WebView,
    alive: Arc<AtomicBool>,
}
impl Reflow {
    pub fn new(
        publication: Publication,
        record: &DocumentRecord,
        generation: u64,
        changed: bool,
        on_message: impl Fn(Message) + 'static,
    ) -> Self {
        let host = format!("document-{}", uuid::Uuid::new_v4());
        let config = serde_json::to_string(&Config {
            host: &host,
            generation,
            entries: &publication.entries,
            settings: &record.settings,
            locator: &record.locator,
            format: record.format,
            title: &record.title,
            changed,
        })
        .expect("serializable reading configuration");
        let publication = Arc::new(Mutex::new(publication));
        let alive = Arc::new(AtomicBool::new(true));
        let context = webkit::WebContext::new();
        context.set_cache_model(webkit::CacheModel::DocumentViewer);
        if let Some(security) = context.security_manager() {
            // This is a capability-scoped resource service, not a file URL.
            // WebKit's file-like classification prevents blob chapter loading.
            security.register_uri_scheme_as_secure("readero");
            security.register_uri_scheme_as_cors_enabled("readero");
        }
        let live = Arc::clone(&alive);
        context.register_uri_scheme("readero", move |request| {
            let Some(uri) = request.uri() else {
                return;
            };
            let Ok(url) = url::Url::parse(&uri) else {
                return;
            };
            let path = url.path().trim_start_matches('/').to_owned();
            #[cfg(feature = "smoke")]
            eprintln!("READERO_RESOURCE {} {}", url.host_str().unwrap_or(""), path);
            if url.host_str() == Some("app") {
                if let Some(data) = asset(&path) {
                    finish(request, data.as_bytes().to_vec(), resource::mime(&path));
                } else {
                    missing(request);
                }
                return;
            }
            if url.host_str() != Some(host.as_str()) || !live.load(Ordering::Relaxed) {
                missing(request);
                return;
            }
            let request = request.clone();
            let publication = Arc::clone(&publication);
            let live = Arc::clone(&live);
            glib::MainContext::default().spawn_local(async move {
                let mime = resource::mime(&path);
                let result = gio::spawn_blocking(move || {
                    if !live.load(Ordering::Relaxed) {
                        return Err(Error::Invalid("Document closed.".into()));
                    }
                    publication
                        .lock()
                        .map_err(|_| Error::Invalid("Document resources stopped.".into()))?
                        .read(&path)
                })
                .await;
                match result {
                    Ok(Ok(bytes)) => finish(&request, bytes, mime),
                    _ => missing(&request),
                }
            });
        });
        let manager = webkit::UserContentManager::new();
        manager.connect_script_message_received(Some("readero"), move |_, value| {
            let text = value.to_str();
            if text.len() > 2 * 1024 * 1024 {
                return;
            }
            if let Ok(message) = serde_json::from_str::<Message>(&text) {
                on_message(message);
            }
        });
        manager.register_script_message_handler("readero", Some(WORLD));
        let boot = format!(
            "import('readero://app/reader.js').then(m => m.start({config}, value => window.webkit.messageHandlers.readero.postMessage(value))).catch(e => window.webkit.messageHandlers.readero.postMessage(JSON.stringify({{generation:{generation},type:'error',message:e.message}})));"
        );
        manager.add_script(&webkit::UserScript::for_world(
            &boot,
            webkit::UserContentInjectedFrames::TopFrame,
            webkit::UserScriptInjectionTime::End,
            WORLD,
            &["readero://app/*"],
            &[],
        ));
        let settings = webkit::Settings::new();
        #[cfg(feature = "smoke")]
        settings.set_enable_write_console_messages_to_stdout(true);
        settings.set_enable_javascript_markup(false);
        settings.set_enable_html5_database(false);
        settings.set_enable_html5_local_storage(false);
        settings.set_allow_file_access_from_file_urls(false);
        settings.set_allow_universal_access_from_file_urls(false);
        settings.set_javascript_can_open_windows_automatically(false);
        settings.set_enable_developer_extras(cfg!(debug_assertions));
        let session = webkit::NetworkSession::new_ephemeral();
        let view = webkit::WebView::builder()
            .web_context(&context)
            .network_session(&session)
            .user_content_manager(&manager)
            .settings(&settings)
            .default_content_security_policy(POLICY)
            .hexpand(true)
            .vexpand(true)
            .build();
        view.connect_decide_policy(|_, decision, kind| {
            if matches!(
                kind,
                webkit::PolicyDecisionType::NavigationAction
                    | webkit::PolicyDecisionType::NewWindowAction
            ) && let Some(nav) = decision.downcast_ref::<webkit::NavigationPolicyDecision>()
            {
                let uri = nav
                    .navigation_action()
                    .and_then(|mut action| action.request())
                    .and_then(|request| request.uri());
                #[cfg(feature = "smoke")]
                eprintln!("READERO_NAV {:?}", uri);
                if !uri.as_deref().is_some_and(|uri| {
                    uri == "readero://app/reader.html"
                        || uri == "about:blank"
                        || uri.starts_with("blob:readero://app/")
                }) {
                    decision.ignore();
                    return true;
                }
            }
            false
        });
        view.connect_permission_request(|_, request| {
            request.deny();
            true
        });
        view.load_uri("readero://app/reader.html");
        Self { view, alive }
    }
    pub fn command(&self, value: serde_json::Value) {
        let script = format!("globalThis.readeroCommand?.({value});");
        self.view
            .evaluate_javascript(&script, Some(WORLD), None, gio::Cancellable::NONE, |_| {});
    }
    pub fn checkpoint(
        &self,
    ) -> impl std::future::Future<Output = Result<Option<Locator>>> + 'static {
        let pending = self.view.call_async_javascript_function_future(
            "return JSON.stringify(await globalThis.readeroCheckpoint());",
            None,
            Some(WORLD),
            None,
        );
        async move {
            let result = glib::future_with_timeout(std::time::Duration::from_secs(3), pending)
                .await
                .map_err(|_| {
                    Error::Invalid("The reading view did not finish saving its position.".into())
                })?
                .map_err(|error| Error::Invalid(error.to_string()))?;
            let locator: Option<Locator> = serde_json::from_str(&result.to_str())?;
            if let Some(locator) = &locator {
                locator.validate()?;
            }
            Ok(locator)
        }
    }
    pub fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(manager) = self.view.user_content_manager() {
            manager.unregister_script_message_handler("readero", Some(WORLD));
        }
        self.command(serde_json::json!({"type":"dispose"}));
        self.view.stop_loading();
    }
}
fn finish(request: &webkit::URISchemeRequest, bytes: Vec<u8>, mime: &str) {
    let length = bytes.len() as i64;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
    let response = webkit::URISchemeResponse::new(&stream, length);
    response.set_content_type(mime);
    let headers = webkit::soup::MessageHeaders::new(webkit::soup::MessageHeadersType::Response);
    headers.append("Access-Control-Allow-Origin", "readero://app");
    headers.append("Cache-Control", "no-store");
    response.set_http_headers(headers);
    request.finish_with_response(&response);
}
fn missing(request: &webkit::URISchemeRequest) {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(b""));
    let response = webkit::URISchemeResponse::new(&stream, 0);
    response.set_status(404, Some("Resource unavailable"));
    let headers = webkit::soup::MessageHeaders::new(webkit::soup::MessageHeadersType::Response);
    headers.append("Access-Control-Allow-Origin", "readero://app");
    response.set_http_headers(headers);
    request.finish_with_response(&response);
}
fn asset(path: &str) -> Option<&'static str> {
    match path {
        "reader.html" => Some(include_str!("../../assets/reader.html")),
        "reader.css" => Some(include_str!("../../assets/reader.css")),
        "reader.js" => Some(include_str!("../../assets/reader.js")),
        "search.js" => Some(include_str!("../../assets/search.js")),
        "anchors.js" => Some(include_str!("../../assets/anchors.js")),
        "interaction.js" => Some(include_str!("../../assets/interaction.js")),
        "continuous.js" => Some(include_str!("../../assets/continuous.js")),
        "foliate/epub.js" => Some(include_str!("../../assets/foliate/epub.js")),
        "foliate/epubcfi.js" => Some(include_str!("../../assets/foliate/epubcfi.js")),
        "foliate/paginator.js" => Some(include_str!("../../assets/foliate/paginator.js")),
        _ => None,
    }
}
