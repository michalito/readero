//! Explicitly opened resources only. File I/O happens on a worker, never in GTK.
use crate::{document::*, markdown};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const MAX_ENTRY: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 512 * 1024 * 1024;

#[derive(Clone, Serialize)]
pub struct ResourceInfo {
    pub name: String,
    pub size: u64,
}
pub enum Source {
    Epub(zip::ZipArchive<File>),
    Markdown {
        root: PathBuf,
        generated: HashMap<String, Vec<u8>>,
    },
}
pub struct Publication {
    pub source: Source,
    pub entries: Vec<ResourceInfo>,
    pub revision: String,
}

pub fn resource_path(raw: &str) -> Result<PathBuf> {
    let decoded = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map_err(|_| Error::Invalid("Invalid resource name.".into()))?;
    if decoded.is_empty() || decoded.len() > 4096 || decoded.contains(['\\', '\0']) {
        return Err(Error::Invalid("Invalid resource name.".into()));
    }
    let path = Path::new(decoded.as_ref());
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::Invalid(
            "This resource is outside the document.".into(),
        ));
    }
    Ok(path.into())
}

pub fn revision(path: &Path) -> Result<String> {
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
            Format::Epub => {
                if meta.len() > MAX_ARCHIVE {
                    return Err(Error::Invalid(
                        "This EPUB exceeds the current 256 MiB archive limit.".into(),
                    ));
                }
                let mut archive = zip::ZipArchive::new(File::open(path)?)?;
                if archive.len() > 10_000 {
                    return Err(Error::Invalid(
                        "This EPUB contains too many resources.".into(),
                    ));
                }
                let mut entries = Vec::new();
                let mut expanded = 0u64;
                for i in 0..archive.len() {
                    let entry = archive.by_index(i)?;
                    if entry.is_dir() {
                        continue;
                    }
                    resource_path(entry.name())?;
                    if entry.size() > MAX_ENTRY
                        || entry.size() / entry.compressed_size().max(1) > 2000
                    {
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
            Format::Markdown => {
                if meta.len() > 32 * 1024 * 1024 {
                    return Err(Error::Invalid(
                        "This Markdown document exceeds the current 32 MiB limit.".into(),
                    ));
                }
                let source = std::fs::read_to_string(path).map_err(|_| {
                    Error::Invalid("Markdown documents must contain valid UTF-8 text.".into())
                })?;
                let title = path.file_stem().unwrap_or_default().to_string_lossy();
                let parsed = markdown::render(&source, &title)?;
                let mut generated: HashMap<String, Vec<u8>> = HashMap::new();
                generated.insert("content.xhtml".into(), parsed.html.into_bytes());
                generated.insert("nav.xhtml".into(), parsed.navigation.into_bytes());
                generated.insert("META-INF/container.xml".into(),br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.to_vec());
                generated.insert("package.opf".into(), format!(r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">readero-markdown</dc:identifier><dc:title>{}</dc:title><dc:language>en</dc:language></metadata><manifest><item id="content" href="content.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="content"/></spine></package>"#,markdown::escape(&title)).into_bytes());
                let entries = generated
                    .iter()
                    .map(|(name, data)| ResourceInfo {
                        name: name.clone(),
                        size: data.len() as u64,
                    })
                    .collect();
                Ok(Self {
                    source: Source::Markdown {
                        root: path.parent().unwrap_or(Path::new(".")).canonicalize()?,
                        generated,
                    },
                    entries,
                    revision: parsed.revision,
                })
            }
            Format::Pdf => Err(Error::Unsupported),
        }
    }
    pub fn read(&mut self, raw: &str) -> Result<Vec<u8>> {
        let path = resource_path(raw)?;
        let mut bytes = Vec::new();
        match &mut self.source {
            Source::Epub(archive) => {
                let mut entry = archive.by_name(&path.to_string_lossy())?;
                entry.by_ref().take(MAX_ENTRY + 1).read_to_end(&mut bytes)?;
            }
            Source::Markdown { root, generated } => {
                if let Some(value) = generated.get(path.to_string_lossy().as_ref()) {
                    return Ok(value.clone());
                }
                let resolved = root.join(path).canonicalize()?;
                if !resolved.starts_with(&*root) {
                    return Err(Error::Invalid(
                        "This resource is outside the document’s folder.".into(),
                    ));
                }
                File::open(resolved)?
                    .take(MAX_ENTRY + 1)
                    .read_to_end(&mut bytes)?;
            }
        }
        if bytes.len() as u64 > MAX_ENTRY {
            return Err(Error::Invalid("This resource is too large to load.".into()));
        }
        Ok(bytes)
    }
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
}
