use readero::{
    document::{Error, Result},
    state::Store,
};
use std::{path::PathBuf, sync::mpsc};

type Work = Box<dyn FnOnce(&mut Result<Store>) + Send>;

#[derive(Clone)]
pub struct StateWorker {
    sender: mpsc::Sender<Work>,
}
impl StateWorker {
    pub fn start(path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel::<Work>();
        std::thread::Builder::new()
            .name("readero-state".into())
            .spawn(move || {
                let mut store = Store::open(&path);
                while let Ok(work) = receiver.recv() {
                    if store.is_err() {
                        store = Store::open(&path);
                    }
                    work(&mut store);
                }
            })
            .expect("The reading-state worker could not be started");
        Self { sender }
    }
    /// Enqueue immediately, before the caller starts awaiting the result. This
    /// preserves save-before-open ordering across separately scheduled UI tasks.
    pub fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Store) -> Result<T> + Send + 'static,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>> {
        let (sender, receiver) = async_channel::bounded(1);
        let queued = self
            .sender
            .send(Box::new(move |store| {
                let result = match store {
                    Ok(store) => work(store),
                    Err(error) => Err(Error::Invalid(error.to_string())),
                };
                let _ = sender.send_blocking(result);
            }))
            .map_err(|_| Error::Invalid("The reading-state worker stopped.".into()));
        Box::pin(async move {
            queued?;
            receiver
                .recv()
                .await
                .map_err(|_| Error::Invalid("The reading-state worker stopped.".into()))?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saving_is_enqueued_before_a_subsequent_open_even_if_not_awaited() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("reading.md");
        std::fs::write(&source, "# A reading passage").unwrap();
        let worker = StateWorker::start(temp.path().join("state.sqlite3"));
        let input = source.clone();
        let save = worker.call(move |store| {
            let mut record = store.document(&input)?;
            record.title = "Saved before opening".into();
            store.save(&record)
        });
        let load = worker.call(move |store| store.document(&source));
        let loaded = gtk::glib::MainContext::new().block_on(load).unwrap();
        assert_eq!(loaded.title, "Saved before opening");
        drop(save);
    }
}
