//! Captured source ranges. A reading locator alone never claims an exact quote.
use crate::document::{Anchor, Error, Locator, Result};
use serde::{Deserialize, Serialize};

pub const MAX_QUOTE_BYTES: usize = 64 * 1024;
pub const MAX_PDF_PAGES: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Passage {
    pub quote: String,
    #[serde(default)]
    pub revision: String,
    #[serde(flatten)]
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    Pdf {
        fragments: Vec<PdfFragment>,
    },
    Reflow {
        href: String,
        cfi: String,
        start: usize,
        end: usize,
        prefix: String,
        suffix: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PdfFragment {
    pub page: i32,
    /// Unrotated page coordinates, as supplied by Papers' selection descriptor.
    pub points: [f64; 4],
    pub style: i32,
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capture {
    pub locator: Locator,
    pub passage: Passage,
}

impl Passage {
    pub fn validate(&self, locator: &Locator) -> Result<()> {
        locator.validate()?;
        let text_ok = |text: &str| !text.contains('\0');
        let context_ok = |text: &str| text_ok(text) && text.chars().count() <= 128;
        let valid = !self.quote.trim().is_empty()
            && self.quote.len() <= MAX_QUOTE_BYTES
            && text_ok(&self.quote)
            && self.revision.len() <= 256
            && text_ok(&self.revision)
            && match (&self.target, &locator.anchor) {
                (Target::Pdf { fragments }, Anchor::Pdf { page, .. }) => {
                    !fragments.is_empty()
                        && fragments.len() <= MAX_PDF_PAGES
                        && fragments[0].page == *page
                        && fragments.windows(2).all(|p| p[0].page < p[1].page)
                        && fragments.iter().all(|f| {
                            f.page >= 0
                                && (0..=2).contains(&f.style)
                                && f.points.iter().all(|v| v.is_finite() && *v >= 0.0)
                                && !f.quote.trim().is_empty()
                                && text_ok(&f.quote)
                                && context_ok(&f.prefix)
                                && context_ok(&f.suffix)
                        })
                        && fragments.iter().map(|f| f.quote.len()).sum::<usize>() <= MAX_QUOTE_BYTES
                        && pdf_quote_matches_fragments(&self.quote, fragments)
                }
                (
                    Target::Reflow {
                        href,
                        cfi,
                        start,
                        end,
                        prefix,
                        suffix,
                    },
                    Anchor::Reflow { href: location, .. },
                ) => {
                    href == location
                        && !href.is_empty()
                        && !cfi.is_empty()
                        && cfi.len() <= 16_384
                        && text_ok(cfi)
                        && start < end
                        && end - start == self.quote.encode_utf16().count()
                        && context_ok(prefix)
                        && context_ok(suffix)
                }
                _ => false,
            };
        if valid {
            Ok(())
        } else {
            Err(Error::Invalid(
                "The selected passage is empty, too large, or invalid. Select a shorter passage."
                    .into(),
            ))
        }
    }

    pub fn label(&self) -> String {
        self.quote
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(70)
            .collect()
    }
}

fn pdf_quote_matches_fragments(quote: &str, fragments: &[PdfFragment]) -> bool {
    let mut remaining = quote;
    for fragment in fragments {
        let Some(start) = remaining.find(&fragment.quote) else {
            return false;
        };
        // Papers inserts whitespace between selected pages. Everything else
        // shown as the saved quotation must be verified by a page fragment.
        if !remaining[..start].trim().is_empty() {
            return false;
        }
        remaining = &remaining[start + fragment.quote.len()..];
    }
    remaining.trim().is_empty()
}

/// Exact, contextual matching only. Repeated quotations without enough context
/// are ambiguous; a caller may offer explicit last-known-location navigation.
pub fn unique_match(text: &str, quote: &str, prefix: &str, suffix: &str) -> Option<usize> {
    if quote.is_empty() {
        return None;
    }
    let mut found = None;
    let mut offset = 0;
    while let Some(relative) = text[offset..].find(quote) {
        let start = offset + relative;
        let end = start + quote.len();
        if text[..start].ends_with(prefix) && text[end..].starts_with(suffix) {
            if found.is_some() {
                return None;
            }
            found = Some(start);
        }
        offset = start + text[start..].chars().next()?.len_utf8();
    }
    found
}
