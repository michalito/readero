use crate::document::{Error, Result};
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Markdown {
    pub html: String,
    pub navigation: String,
    pub revision: String,
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn safe_link(value: &str, image: bool) -> bool {
    let compact: String = value.chars().filter(|c| !c.is_control()).collect();
    if compact.starts_with("//") {
        return false;
    }
    match url::Url::parse(&compact) {
        Ok(url) => !image && matches!(url.scheme(), "https" | "http" | "mailto"),
        Err(url::ParseError::RelativeUrlWithoutBase) => !compact.starts_with('/'),
        _ => false,
    }
}

pub fn render(source: &str, title: &str) -> Result<Markdown> {
    if source.trim().is_empty() {
        return Err(Error::Empty);
    }
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(source, options).into_offset_iter();
    let mut events = Vec::new();
    let mut depth = 0usize;
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut heading = None;
    let mut heading_ids = HashSet::new();
    let mut toc = Vec::new();
    for (event, range) in parser {
        if matches!(event, Event::Start(_)) && depth == 0 {
            // Content identity survives insertions before the passage. Identical
            // blocks have an occurrence suffix; ambiguous edits remain a fallback.
            let hash = format!("{:x}", Sha256::digest(source[range.clone()].as_bytes()));
            let count = ids.entry(hash[..16].into()).or_default();
            let id = format!("b{}-{}", &hash[..16], count);
            *count += 1;
            events.push(Event::Html(format!("<div id=\"{id}\" data-reader-block=\"{id}\" data-source-start=\"{}\" data-source-end=\"{}\">", range.start, range.end).into()));
            if matches!(event, Event::Start(Tag::Heading { .. })) {
                heading = Some((id, String::new(), events.len()));
            }
        }
        match &event {
            Event::Start(_) => depth += 1,
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, label, _)) = &mut heading {
                    label.push_str(text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((block, label, position)) = heading.take() {
                    let slug: String = label
                        .to_lowercase()
                        .chars()
                        .filter_map(|ch| {
                            if ch.is_whitespace() {
                                Some('-')
                            } else if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                                Some(ch)
                            } else {
                                None
                            }
                        })
                        .collect();
                    let base = if slug.is_empty() { "section" } else { &slug };
                    let mut id = base.to_owned();
                    let mut suffix = 1;
                    while !heading_ids.insert(id.clone()) {
                        id = format!("{base}-{suffix}");
                        suffix += 1;
                    }
                    if let Some(Event::Start(Tag::Heading { id: heading_id, .. })) =
                        events.get_mut(position)
                    {
                        *heading_id = Some(id.into());
                    }
                    toc.push((block, label));
                }
            }
            _ => {}
        }
        let end = matches!(event, Event::End(_));
        let event = match event {
            Event::Html(value) | Event::InlineHtml(value) => Event::Text(value),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Link {
                link_type,
                dest_url: if safe_link(&dest_url, false) {
                    dest_url
                } else {
                    CowStr::from("#")
                },
                title,
                id,
            }),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Image {
                link_type,
                dest_url: if safe_link(&dest_url, true) {
                    dest_url
                } else {
                    CowStr::from("")
                },
                title,
                id,
            }),
            other => other,
        };
        events.push(event);
        if end {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                events.push(Event::Html("</div>".into()));
            }
        }
    }
    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    let title = escape(title);
    let html = format!(
        "<!DOCTYPE html><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{title}</title></head><body>{body}</body></html>"
    );
    let links = toc
        .iter()
        .map(|(id, label)| {
            format!(
                "<li><a href=\"content.xhtml#{id}\">{}</a></li>",
                escape(label)
            )
        })
        .collect::<String>();
    let navigation = format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head><body><nav epub:type=\"toc\"><ol>{links}</ol></nav></body></html>"
    );
    Ok(Markdown {
        html,
        navigation,
        revision: format!("{:x}", Sha256::digest(source.as_bytes())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_html_and_active_urls_but_keeps_relative_images() {
        let md = render("# α heading\n\n<script>bad()</script>\n\n[bad](javascript:alert%281%29)\n\n![figure](figures/example.png)\n\n- [x] Done\n", "A & B").unwrap();
        assert!(!md.html.contains("<script>"));
        assert!(md.html.contains("&lt;script&gt;"));
        assert!(!md.html.contains("javascript:"));
        assert!(md.html.contains("figures/example.png"));
        assert!(md.navigation.contains("α heading"));
        assert!(md.html.contains("checkbox"));
    }
    #[test]
    fn heading_links_have_unique_readable_fragments() {
        let md = render("# A `reading` passage!\n\n# A reading passage!\n\n# A reading passage-1\n\n# Café 🐢\n\n[Jump](#a-reading-passage)", "test").unwrap();
        for id in [
            "a-reading-passage",
            "a-reading-passage-1",
            "a-reading-passage-1-1",
            "café-",
        ] {
            assert!(md.html.contains(&format!("id=\"{id}\"")));
        }
        assert!(md.html.contains("href=\"#a-reading-passage\""));
    }
    #[test]
    fn block_identity_survives_insertion_before_unicode_passage() {
        let before = render("# Heading\n\nA passage with 🦀 and café.", "test").unwrap();
        let after = render(
            "An inserted paragraph.\n\n# Heading\n\nA passage with 🦀 and café.",
            "test",
        )
        .unwrap();
        let marker = before
            .html
            .split("data-reader-block=\"")
            .nth(2)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(after.html.contains(marker));
        assert_ne!(before.revision, after.revision);
    }
}
