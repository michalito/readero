use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("This file type isn’t supported. Choose a PDF, EPUB, or Markdown document.")]
    Unsupported,
    #[error("The document is empty.")]
    Empty,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Database(#[from] rusqlite::Error),
    #[error("Saved reading data could not be read: {0}")]
    Json(#[from] serde_json::Error),
    #[error("The EPUB archive could not be read: {0}")]
    Zip(#[from] zip::result::ZipError),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Pdf,
    Epub,
    Markdown,
}

impl Format {
    pub fn from_path(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "pdf" => Ok(Self::Pdf),
            "epub" => Ok(Self::Epub),
            "md" | "markdown" => Ok(Self::Markdown),
            _ => Err(Error::Unsupported),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Epub => "EPUB",
            Self::Markdown => "Markdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Scroll,
    Pages,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Palette {
    #[default]
    Light,
    Warm,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    pub palette: Palette,
    pub font_size: f64,
    pub line_height: f64,
    pub width: u32,
    pub font: String,
    pub pdf_sizing: String,
    pub pdf_scale: f64,
    pub rotation: i32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Scroll,
            palette: Palette::Light,
            font_size: 20.0,
            line_height: 1.65,
            width: 740,
            font: "serif".into(),
            pdf_sizing: "width".into(),
            pdf_scale: 1.0,
            rotation: 0,
        }
    }
}
impl Settings {
    pub fn for_format(format: Format) -> Self {
        Self {
            mode: if format == Format::Epub {
                Mode::Pages
            } else {
                Mode::Scroll
            },
            ..Self::default()
        }
    }
    pub fn normalize(&mut self) {
        self.font_size = self.font_size.clamp(14.0, 32.0);
        self.line_height = self.line_height.clamp(1.2, 2.2);
        self.width = self.width.clamp(480, 1100);
        self.pdf_scale = self.pdf_scale.clamp(0.25, 5.0);
        self.rotation = self.rotation.rem_euclid(360) / 90 * 90;
        if !["serif", "sans", "publisher"].contains(&self.font.as_str()) {
            self.font = "serif".into();
        }
        if !["width", "page", "custom"].contains(&self.pdf_sizing.as_str()) {
            self.pdf_sizing = "width".into();
        }
    }
}

/// Identity is independent of screen pages, typography and presentation mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Locator {
    pub version: u32,
    #[serde(flatten)]
    pub anchor: Anchor,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Anchor {
    Pdf {
        page: i32,
        x: f64,
        y: f64,
        viewport_y: f64,
    },
    Reflow {
        href: String,
        cfi: String,
        section: usize,
        fraction: f64,
        #[serde(default)]
        quote: String,
        #[serde(default)]
        block: String,
        /// UTF-16 offsets used by DOM Range, retained across Markdown edits.
        #[serde(default)]
        block_offset: Option<usize>,
        #[serde(default)]
        quote_offset: usize,
    },
}
impl Locator {
    pub fn validate(&self) -> Result<()> {
        let valid = self.version == 1
            && match &self.anchor {
                Anchor::Pdf {
                    page,
                    x,
                    y,
                    viewport_y,
                } => {
                    *page >= 0
                        && [x, y, viewport_y]
                            .iter()
                            .all(|v| v.is_finite() && **v >= 0.0)
                }
                Anchor::Reflow {
                    href,
                    cfi,
                    section,
                    fraction,
                    quote,
                    block,
                    block_offset,
                    quote_offset,
                } => {
                    href.len() <= 4096
                        && cfi.len() <= 16_384
                        && *section < 10_000
                        && fraction.is_finite()
                        && (0.0..=1.0).contains(fraction)
                        && quote.len() <= 2048
                        && block.len() <= 512
                        && block_offset.is_none_or(|offset| offset <= 32 * 1024 * 1024)
                        && *quote_offset <= quote.encode_utf16().count()
                }
            };
        if valid {
            Ok(())
        } else {
            Err(Error::Invalid(
                "The reading position uses an unsupported or invalid format.".into(),
            ))
        }
    }
    pub fn label(&self) -> String {
        match &self.anchor {
            Anchor::Pdf { page, .. } => format!("Page {}", page + 1),
            Anchor::Reflow { section, quote, .. } => {
                let excerpt: String = quote
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(70)
                    .collect();
                if excerpt.is_empty() {
                    format!("Section {}", section + 1)
                } else {
                    format!("{} · {excerpt}", section + 1)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentRecord {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub format: Format,
    pub revision: String,
    pub locator: Option<Locator>,
    pub settings: Settings,
    pub last_opened: i64,
}
impl DocumentRecord {
    pub fn new(path: PathBuf) -> Result<Self> {
        let format = Format::from_path(&path)?;
        let title = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            path,
            title,
            format,
            revision: String::new(),
            locator: None,
            settings: Settings::for_format(format),
            last_opened: 0,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: String,
    pub document_id: String,
    pub label: String,
    pub locator: Locator,
}

/// Temporary opening/reflow events must never replace a committed passage.
#[derive(Default, Debug)]
pub struct SaveGate {
    generation: u64,
    ready: bool,
}
impl SaveGate {
    pub fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.ready = false;
        self.generation
    }
    pub fn ready(&mut self, generation: u64) {
        if self.generation == generation {
            self.ready = true;
        }
    }
    pub fn accepts(&self, generation: u64) -> bool {
        self.ready && self.generation == generation
    }
    pub fn current(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_reflow_locations_remain_readable() {
        let old = r#"{"version":1,"kind":"reflow","href":"content.xhtml","cfi":"epubcfi(/6/2!/4/2)","section":0,"fraction":0.4,"quote":"A passage","block":"b1"}"#;
        let locator: Locator = serde_json::from_str(old).unwrap();
        assert!(locator.validate().is_ok());
        assert!(matches!(
            locator.anchor,
            Anchor::Reflow {
                block_offset: None,
                quote_offset: 0,
                ..
            }
        ));
    }
    #[test]
    fn stale_completion_cannot_enable_saving() {
        let mut gate = SaveGate::default();
        let a = gate.begin();
        let b = gate.begin();
        gate.ready(a);
        assert!(!gate.accepts(b));
        gate.ready(b);
        assert!(gate.accepts(b));
        assert!(!gate.accepts(a));
    }
    #[test]
    fn locators_reject_unknown_versions_and_nonfinite_coordinates() {
        let mut point = Locator {
            version: 1,
            anchor: Anchor::Pdf {
                page: 0,
                x: 0.0,
                y: f64::NAN,
                viewport_y: 0.0,
            },
        };
        assert!(point.validate().is_err());
        point.anchor = Anchor::Pdf {
            page: 0,
            x: 0.0,
            y: 4.0,
            viewport_y: 0.0,
        };
        assert!(point.validate().is_ok());
        point.version = 2;
        assert!(point.validate().is_err());
    }
}
