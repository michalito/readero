//! Opt-in qualification probes without the smoke driver's fixed navigation waits.
use super::*;
use serde_json::{Value, json};
use webkit6::prelude::*;

async fn pause(ms: u64) {
    glib::timeout_future(Duration::from_millis(ms)).await;
}
fn write(path: &std::path::Path, value: &impl serde::Serialize) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

impl Shell {
    pub(super) fn probe(self: &Rc<Self>, directory: PathBuf, file: PathBuf, mode: String) {
        let shell = Rc::clone(self);
        glib::MainContext::default().spawn_local(async move {
            shell.open(file.clone());
            shell.probe_ready().await;
            let started = std::env::var("READERO_PROBE_STARTED_NS")
                .ok()
                .and_then(|s| s.parse::<u128>().ok());
            let elapsed = started.map(|start| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    .saturating_sub(start) as f64
                    / 1_000_000.0
            });
            let mut report = json!({"launch_ready_ms": elapsed, "pid":std::process::id()});
            if mode == "warm" {
                let mut times = Vec::new();
                for _ in 0..30 {
                    let start = Instant::now();
                    shell.open(file.clone());
                    shell.probe_ready().await;
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                report["warm_ready_ms"] = json!(times);
            }
            if mode == "crash" || mode == "close-layout" {
                for _ in 0..4 {
                    shell.turn(true);
                    pause(100).await;
                }
                shell.probe_ready().await;
                shell.bookmark();
                shell.change_settings(|settings| {
                    settings.mode = Mode::Pages;
                    settings.font_size = 22.0;
                });
                if mode == "crash" {
                    shell.probe_ready().await;
                    shell.save();
                }
            }
            shell.reading_state.barrier(Duration::ZERO).await.unwrap();
            let record = shell
                .record_for_save()
                .expect("native probe must reach a readable document");
            let id = record.id.clone();
            report["record"] = json!(record);
            report["bookmark_count"] =
                json!(shell.reading_state.bookmarks(id).await.unwrap().len());
            write(&directory.join("report.json"), &report);
            if mode == "crash" {
                pause(120_000).await;
            }
            if mode == "idle" {
                pause(4000).await;
                write(
                    &directory.join("baseline-ready.json"),
                    &json!({"pid":std::process::id()}),
                );
                pause(5000).await;
                write(&directory.join("activity-started.json"), &json!(true));
                for step in 0..40 {
                    shell.turn(step < 20);
                    if step % 2 == 0 {
                        shell.change_settings(|settings| {
                            settings.mode = if settings.mode == Mode::Scroll {
                                Mode::Pages
                            } else {
                                Mode::Scroll
                            }
                        });
                    }
                    shell.probe_ready().await;
                }
                shell.save();
                pause(4000).await;
                write(
                    &directory.join("idle-ready.json"),
                    &json!({"pid":std::process::id()}),
                );
                pause(120_000).await;
            }
            shell.close();
        });
    }
    async fn probe_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if !self.is_opening() && self.is_ready() {
                let (view, pdf) = match self.surface.borrow().as_ref() {
                    Some(Surface::Reflow(reader)) => (Some(reader.view.clone()), false),
                    Some(Surface::Pdf(_)) => (None, true),
                    _ => (None, false),
                };
                if pdf {
                    pause(32).await;
                    return;
                }
                if let Some(view) = view {
                    let expected = self.current.borrow().as_ref().map(|r| r.settings.mode);
                    if let Ok(result) = view
                        .evaluate_javascript_future(
                            "JSON.stringify(globalThis.readeroDiagnostics?.())",
                            Some(reflow::WORLD),
                            None,
                        )
                        .await
                        && let Ok(value) = serde_json::from_str::<Value>(&result.to_str())
                        && value["stable"] == true
                        && value["anchorVisible"] == true
                        && value["mode"]
                            == if expected == Some(Mode::Pages) {
                                "pages"
                            } else {
                                "scroll"
                            }
                    {
                        return;
                    }
                }
            }
            pause(10).await;
        }
        panic!("native qualification did not reach a readable, stable passage");
    }
}
