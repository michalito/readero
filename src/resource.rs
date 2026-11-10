//! The resources of an explicitly opened reflowable document, and nothing else.
//!
//! An EPUB is inspected once and then read in place; a Markdown source is
//! packaged by [`markdown`] and may load sidecars from its own directory.
//! File I/O happens on a worker, never in GTK.
use crate::{document::*, markdown};
use serde::Serialize;
use std::{
    fs::File,
    io::Read,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

/// The largest archive entry or sidecar file that is served.
const MAX_ENTRY: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_MARKDOWN: u64 = 32 * 1024 * 1024;

#[derive(Clone, Serialize)]
pub struct ResourceInfo {
    pub name: String,
    pub size: u64,
}

pub struct Publication {
    source: Source,
    pub entries: Vec<ResourceInfo>,
    /// The hash of a Markdown source; the size and modification time of an
    /// EPUB, which is too large to hash on every open.
    pub revision: String,
}

enum Source {
    Epub(zip::ZipArchive<File>),
    Markdown {
        generated: Vec<(&'static str, Vec<u8>)>,
        /// The canonical directory of the source; sidecars resolve inside it.
        folder: PathBuf,
    },
}

/// The path a requested resource names, relative to its document.
pub fn resource_path(raw: &str) -> Result<PathBuf> {
    resource_name(raw).map(PathBuf::from)
}

/// Request URLs are percent-encoded; ZIP entry names are literal and are
/// validated without decoding.
fn resource_name(raw: &str) -> Result<String> {
    let name = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map_err(|_| Error::Invalid("Invalid resource name.".into()))?;
    validate_name(&name)?;
    Ok(name.into_owned())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 4096 || name.contains(['\\', '\0']) {
        return Err(Error::Invalid("Invalid resource name.".into()));
    }
    if Path::new(name)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::Invalid(
            "This resource is outside the document.".into(),
        ));
    }
    Ok(())
}

fn too_large() -> Error {
    Error::Invalid("This resource is too large to load.".into())
}

/// Reads no more than `limit` bytes, whatever size the source declares.
fn read_within(source: impl Read, limit: u64, exceeded: fn() -> Error) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    source.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(exceeded());
    }
    Ok(bytes)
}

fn revision(path: &Path) -> Result<String> {
    let meta = path.metadata()?;
    Ok(format!(
        "{}:{}",
        meta.len(),
        meta.modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

impl Publication {
    pub fn open(path: &Path, format: Format) -> Result<Self> {
        let meta = path.metadata()?;
        if !meta.is_file() {
            return Err(Error::Invalid("Choose a document file.".into()));
        }
        if meta.len() == 0 {
            return Err(Error::Empty);
        }
        match format {
            Format::Epub => Self::epub(path, meta.len()),
            Format::Markdown => Self::markdown(path, meta.len()),
            Format::Pdf => Err(Error::Unsupported),
        }
    }

    /// Every entry is checked before any is read, and nothing is extracted.
    /// Declared sizes bound the listing; `read` bounds what is decompressed.
    fn epub(path: &Path, size: u64) -> Result<Self> {
        if size > MAX_ARCHIVE {
            return Err(Error::Invalid(
                "This EPUB exceeds the current 256 MiB archive limit.".into(),
            ));
        }
        let mut archive = zip::ZipArchive::new(File::open(path)?)?;
        if archive.len() > MAX_ENTRIES {
            return Err(Error::Invalid(
                "This EPUB contains too many resources.".into(),
            ));
        }
        let mut entries = Vec::new();
        let mut expanded = 0u64;
        for index in 0..archive.len() {
            let entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            validate_name(entry.name())?;
            if entry.size() > MAX_ENTRY {
                return Err(Error::Invalid(
                    "An EPUB resource exceeds the safe loading limit.".into(),
                ));
            }
            expanded = expanded.saturating_add(entry.size());
            if expanded > MAX_EXPANDED {
                return Err(Error::Invalid(
                    "This EPUB expands beyond the current 512 MiB limit.".into(),
                ));
            }
            entries.push(ResourceInfo {
                name: entry.name().into(),
                size: entry.size(),
            });
        }
        if !entries.iter().any(|x| x.name == "META-INF/container.xml") {
            return Err(Error::Invalid(
                "This archive does not contain an EPUB publication.".into(),
            ));
        }
        Ok(Self {
            source: Source::Epub(archive),
            entries,
            revision: revision(path)?,
        })
    }

    fn markdown(path: &Path, size: u64) -> Result<Self> {
        let too_long =
            || Error::Invalid("This Markdown document exceeds the current 32 MiB limit.".into());
        if size > MAX_MARKDOWN {
            return Err(too_long());
        }
        let source = read_within(File::open(path)?, MAX_MARKDOWN, too_long)?;
        let title = path.file_stem().unwrap_or_default().to_string_lossy();
        let package = markdown::package(&source, &title)?;
        let folder = path
            .canonicalize()?
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| Error::Invalid("Choose a document file.".into()))?;
        Ok(Self {
            entries: package
                .resources
                .iter()
                .map(|(name, data)| ResourceInfo {
                    name: (*name).into(),
                    size: data.len() as u64,
                })
                .collect(),
            source: Source::Markdown {
                generated: package.resources,
                folder,
            },
            revision: package.revision,
        })
    }

    pub fn read(&mut self, raw: &str) -> Result<Vec<u8>> {
        let name = resource_name(raw)?;
        match &mut self.source {
            Source::Epub(archive) => read_within(archive.by_name(&name)?, MAX_ENTRY, too_large),
            Source::Markdown { generated, folder } => {
                match generated.iter().find(|(known, _)| *known == name) {
                    Some((_, data)) => Ok(data.clone()),
                    None => read_within(sidecar(folder, &name)?, MAX_ENTRY, too_large),
                }
            }
        }
    }
}

fn outside_folder() -> Error {
    Error::Invalid("This resource is outside the document’s folder.".into())
}

/// A regular file inside the document's folder once links are resolved.
fn sidecar(folder: &Path, name: &str) -> Result<File> {
    let resolved = folder.join(name).canonicalize()?;
    if !resolved.starts_with(folder) {
        return Err(outside_folder());
    }
    // Opening a pipe would otherwise wait for a writer that may never come.
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(resolved)?;
    confined(file, folder)
}

/// Judges the opened file itself: its name may have been pointed somewhere
/// else, or at something that is not a file, after it was resolved.
fn confined(file: File, folder: &Path) -> Result<File> {
    let opened = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    if !opened.starts_with(folder) {
        return Err(outside_folder());
    }
    if !file.metadata()?.is_file() {
        return Err(Error::Invalid("This resource is not a file.".into()));
    }
    Ok(file)
}

pub fn mime(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html",
        "xhtml" => "application/xhtml+xml",
        "xml" | "opf" | "ncx" => "application/xml",
        "css" => "text/css",
        "js" => "text/javascript",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traversal_is_rejected_after_decoding() {
        for path in [
            "../secret",
            "%2e%2e/secret",
            "/etc/passwd",
            "a\\..\\b",
            "a/%00",
            "a/../../b",
        ] {
            assert!(resource_path(path).is_err(), "{path}");
        }
        assert_eq!(
            resource_path("images/caf%C3%A9.png").unwrap(),
            PathBuf::from("images/café.png")
        );
    }
    #[test]
    fn markdown_symlink_cannot_escape_granted_directory() {
        let temp = tempfile::tempdir().unwrap();
        let allowed = temp.path().join("allowed");
        std::fs::create_dir(&allowed).unwrap();
        let source = allowed.join("book.md");
        std::fs::write(&source, "# Test").unwrap();
        let secret = temp.path().join("secret");
        std::fs::write(&secret, "private").unwrap();
        std::os::unix::fs::symlink(&secret, allowed.join("image.png")).unwrap();
        let mut publication = Publication::open(&source, Format::Markdown).unwrap();
        assert!(publication.read("image.png").is_err());
        assert!(publication.read("content.xhtml").is_ok());
    }
    /// What `sidecar` resolved can be replaced before it is opened, so the
    /// decision has to rest on the handle that was actually obtained.
    #[test]
    fn an_opened_file_is_judged_by_what_it_is_not_by_the_name_it_had() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().canonicalize().unwrap().join("granted");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("figure.png"), "figure").unwrap();
        let outside = folder.parent().unwrap().join("secret");
        std::fs::write(&outside, "private").unwrap();

        let inside = confined(File::open(folder.join("figure.png")).unwrap(), &folder);
        let mut text = String::new();
        inside.unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "figure");
        assert!(confined(File::open(&outside).unwrap(), &folder).is_err());
        assert!(confined(File::open(&folder).unwrap(), &folder).is_err());
    }
}
