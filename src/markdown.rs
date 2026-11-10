//! The Markdown dialect, packaged as a synthetic reflowable publication.
//!
//! CommonMark with tables, task lists, strikethrough and footnotes. Raw HTML is
//! shown as text, except that a comment standing alone is hidden, as is front
//! matter at the start of the source. The reader loads the result exactly as it
//! loads an EPUB, so this module alone knows the names and layout of the
//! generated resources.
use crate::document::{Error, Result};
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const CONTAINER: &str = "META-INF/container.xml";
const MANIFEST: &str = "package.opf";
const CONTENT: &str = "content.xhtml";
const NAVIGATION: &str = "nav.xhtml";

/// The generated resources of one Markdown source, in a fixed order.
pub struct Package {
    pub resources: Vec<(&'static str, Vec<u8>)>,
    /// Identifies the exact source bytes.
    pub revision: String,
}

pub fn package(source: &[u8], title: &str) -> Result<Package> {
    let text = std::str::from_utf8(source)
        .map_err(|_| Error::Invalid("Markdown documents must contain valid UTF-8 text.".into()))?;
    let rendered = render(text)?;
    let title = escape(title);
    Ok(Package {
        resources: vec![
            (CONTAINER, container().into_bytes()),
            (MANIFEST, manifest(&title).into_bytes()),
            (CONTENT, content(&title, &rendered.body).into_bytes()),
            (NAVIGATION, navigation(&rendered.outline).into_bytes()),
        ],
        revision: format!("{:x}", Sha256::digest(source)),
    })
}

/// Where the reader finds a heading named by a link's fragment. Links carry
/// the fragment percent-encoded; the heading itself is addressed by its text.
pub fn heading_href(fragment: &str) -> String {
    format!(
        "{CONTENT}#{}",
        percent_decode_str(fragment).decode_utf8_lossy()
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn container() -> String {
    format!(
        r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="{MANIFEST}" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#
    )
}

fn manifest(title: &str) -> String {
    format!(
        r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="id">readero-markdown</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="content" href="{CONTENT}" media-type="application/xhtml+xml"/>
    <item id="nav" href="{NAVIGATION}" media-type="application/xhtml+xml" properties="nav"/>
  </manifest>
  <spine>
    <itemref idref="content"/>
  </spine>
</package>"#
    )
}

fn content(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>{title}</title></head><body>{body}</body></html>"
    )
}

/// A nested list. Each heading sits inside the nearest earlier heading of a
/// higher level, so a skipped level does not leave an empty entry.
fn navigation(outline: &[Heading]) -> String {
    let mut items = String::new();
    // Levels of the entries that are still open, outermost first.
    let mut open: Vec<HeadingLevel> = Vec::new();
    let close = |items: &mut String, entries: usize| {
        if entries > 0 {
            items.push_str("</li>");
            items.push_str(&"</ol></li>".repeat(entries - 1));
        }
    };
    for heading in outline {
        let ancestors = open.partition_point(|level| *level < heading.level);
        if ancestors < open.len() {
            close(&mut items, open.len() - ancestors);
        } else if !open.is_empty() {
            // The first entry inside the heading before it.
            items.push_str("<ol>");
        }
        open.truncate(ancestors);
        open.push(heading.level);
        items.push_str(&format!(
            "<li><a href=\"{CONTENT}#{}\">{}</a>",
            heading.block,
            escape(&heading.label)
        ));
    }
    close(&mut items, open.len());
    format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head><body><nav epub:type=\"toc\"><ol>{items}</ol></nav></body></html>"
    )
}

struct Rendered {
    body: String,
    outline: Vec<Heading>,
}

/// A top-level heading with visible text, addressed by the block that
/// contains it.
struct Heading {
    level: HeadingLevel,
    block: String,
    label: String,
}

/// Saved positions name these identities, so they must not change for
/// unchanged source text.
#[derive(Default)]
struct Blocks(HashMap<String, usize>);

impl Blocks {
    /// Content identity survives insertions before the passage. Identical
    /// blocks have an occurrence suffix; ambiguous edits remain a fallback.
    fn identify(&mut self, source: &str) -> String {
        let hash = format!("{:x}", Sha256::digest(source.as_bytes()));
        let occurrence = self.0.entry(hash[..16].into()).or_default();
        let id = format!("b{}-{occurrence}", &hash[..16]);
        *occurrence += 1;
        id
    }
}

#[derive(Default)]
struct Outline {
    fragments: HashSet<String>,
    open: Option<OpenHeading>,
    headings: Vec<Heading>,
}

struct OpenHeading {
    level: HeadingLevel,
    block: String,
    /// The text links name the heading by. Line breaks contribute nothing,
    /// as they never have, so that no existing fragment changes.
    text: String,
    /// The text as the outline shows it.
    label: String,
    /// Index of the heading's start event, which receives the fragment.
    start: usize,
}

impl Outline {
    fn open(&mut self, level: HeadingLevel, block: String, start: usize) {
        self.open = Some(OpenHeading {
            level,
            block,
            text: String::new(),
            label: String::new(),
            start,
        });
    }

    fn text(&mut self, text: &str) {
        if let Some(heading) = &mut self.open {
            heading.text.push_str(text);
            heading.label.push_str(text);
        }
    }

    fn line_break(&mut self) {
        if let Some(heading) = &mut self.open {
            heading.label.push(' ');
        }
    }

    /// Records the open heading and returns its start event and the unique,
    /// readable fragment that links address it by.
    fn close(&mut self) -> Option<(usize, String)> {
        let heading = self.open.take()?;
        let slug: String = heading
            .text
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
        let mut fragment = base.to_owned();
        let mut suffix = 1;
        while !self.fragments.insert(fragment.clone()) {
            fragment = format!("{base}-{suffix}");
            suffix += 1;
        }
        // A heading without text keeps its fragment but has nothing to list.
        let label = heading
            .label
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !label.is_empty() {
            self.headings.push(Heading {
                level: heading.level,
                block: heading.block,
                label,
            });
        }
        Some((heading.start, fragment))
    }
}

/// The length of front matter at the very start of a source: a `---` line, a
/// `key:` line, further keys, list items and indented lines, and a closing
/// `---` or `...` line.
///
/// Hiding prose is worse than showing metadata, so any other line, a blank
/// one included, leaves the whole block to be read as ordinary Markdown.
fn front_matter(source: &str) -> usize {
    let is_key = |text: &str| {
        text.split_once(':').is_some_and(|(key, value)| {
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                && (value.is_empty() || value.starts_with([' ', '\t']))
        })
    };
    let mut length = 0;
    for (number, line) in source.split_inclusive('\n').enumerate() {
        length += line.len();
        let text = line.strip_suffix('\n').unwrap_or(line);
        let text = text.strip_suffix('\r').unwrap_or(text);
        // The parser ends a line at a lone carriage return too, so text
        // after one would be hidden without having been looked at.
        if text.contains('\r') {
            break;
        }
        // Only spaces and tabs may trail: other blank characters are text.
        let text = text.trim_end_matches([' ', '\t']);
        let continues = match number {
            0 => text == "---",
            1 => is_key(text),
            _ if matches!(text, "---" | "...") => return length,
            _ => {
                is_key(text)
                    || text == "-"
                    || text.starts_with("- ")
                    || (text.starts_with([' ', '\t']) && !text.is_empty())
            }
        };
        if !continues {
            break;
        }
    }
    0
}

/// A single well-formed HTML comment and nothing else. Only the comment is
/// hidden: whatever surrounds it, such as a quotation, is rendered as it would
/// be around nothing.
///
/// Anything a browser would read differently stays visible as raw HTML: an
/// unclosed comment, which runs to the end of the source, and the short forms
/// `<!-->` and `<!--->` or an inner `--!>`, after which text is shown.
fn is_comment(html: &str) -> bool {
    html.trim()
        .strip_prefix("<!--")
        .and_then(|rest| rest.strip_suffix("-->"))
        .is_some_and(|body| {
            !body.starts_with('>')
                && !body.starts_with("->")
                && !body.contains("-->")
                && !body.contains("--!>")
        })
}

fn render(source: &str) -> Result<Rendered> {
    // Neither a byte-order mark nor front matter is text to read. Blocks are
    // identified by the source that follows, exactly as the parser sees it.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let source = &source[front_matter(source)..];
    if source.trim().is_empty() {
        return Err(Error::Empty);
    }
    let dialect = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES;
    let mut events = Vec::new();
    let mut blocks = Blocks::default();
    let mut outline = Outline::default();
    let mut depth = 0usize;
    // Where the open top-level block begins, and the lines of an open raw
    // HTML block: whether it is a comment is only known once it ends.
    let mut wrapper = None::<usize>;
    let mut raw_block = None::<String>;
    for (event, range) in Parser::new_ext(source, dialect).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    let id = blocks.identify(&source[range]);
                    wrapper = Some(events.len());
                    events.push(Event::Html(
                        format!("<div id=\"{id}\" data-reader-block=\"{id}\">").into(),
                    ));
                    if let Tag::Heading { level, .. } = &tag {
                        outline.open(*level, id, events.len());
                    }
                }
                depth += 1;
                match tag {
                    Tag::HtmlBlock => raw_block = Some(String::new()),
                    tag => events.push(Event::Start(permitted(tag))),
                }
            }
            Event::End(TagEnd::HtmlBlock) => {
                let raw = raw_block.take().unwrap_or_default();
                if !is_comment(&raw) {
                    events.push(Event::Text(raw.into()));
                }
                depth = depth.saturating_sub(1);
            }
            Event::End(end) => {
                if matches!(end, TagEnd::Heading(_))
                    && let Some((start, fragment)) = outline.close()
                    && let Some(Event::Start(Tag::Heading { id, .. })) = events.get_mut(start)
                {
                    *id = Some(fragment.into());
                }
                events.push(Event::End(end));
                depth = depth.saturating_sub(1);
            }
            Event::Text(text) => {
                outline.text(&text);
                events.push(Event::Text(text));
            }
            Event::Code(code) => {
                outline.text(&code);
                events.push(Event::Code(code));
            }
            line_break @ (Event::SoftBreak | Event::HardBreak) => {
                outline.line_break();
                events.push(line_break);
            }
            Event::Html(line) => match &mut raw_block {
                Some(raw) => raw.push_str(&line),
                None => events.push(Event::Text(line)),
            },
            Event::InlineHtml(raw) => {
                if !is_comment(&raw) {
                    events.push(Event::Text(raw));
                }
            }
            other => events.push(other),
        }
        if depth == 0
            && let Some(start) = wrapper.take()
        {
            // A block that was a hidden comment leaves no trace.
            if events.len() == start + 1 {
                events.pop();
            } else {
                events.push(Event::Html("</div>".into()));
            }
        }
    }
    // Hidden comments or link definitions alone give the reader nothing.
    if events.is_empty() {
        return Err(Error::Empty);
    }
    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    Ok(Rendered {
        body,
        outline: outline.headings,
    })
}

/// Rewrites a destination the reader must not follow or load.
fn permitted(tag: Tag) -> Tag {
    match tag {
        Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        } => Tag::Link {
            link_type,
            dest_url: if safe_link(&dest_url, false) {
                dest_url
            } else {
                CowStr::from("#")
            },
            title,
            id,
        },
        Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        } => Tag::Image {
            link_type,
            dest_url: if safe_link(&dest_url, true) {
                dest_url
            } else {
                CowStr::from("")
            },
            title,
            id,
        },
        other => other,
    }
}

/// What the renderer percent-encodes in a destination, besides non-ASCII text.
const ENCODED_IN_DESTINATION: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn safe_link(value: &str, image: bool) -> bool {
    let compact: String = value.chars().filter(|c| !c.is_control()).collect();
    if compact.starts_with("//") {
        return false;
    }
    match url::Url::parse(&compact) {
        Ok(url) => !image && matches!(url.scheme(), "https" | "http" | "mailto"),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            !compact.starts_with('/') && (!image || stays_inside(value))
        }
        _ => false,
    }
}

/// Whether a relative reference stays within the document's folder.
///
/// The reader resolves it with the folder as the root of a URL, where a path
/// that climbs out is clamped and names a different file. Resolved one level
/// down, by the same rules, such a path leaves the level instead. It is
/// judged as it will be written into the document: a space or tab is encoded
/// there and so belongs to a name, where a URL would otherwise drop it.
fn stays_inside(reference: &str) -> bool {
    let written = utf8_percent_encode(reference, ENCODED_IN_DESTINATION).to_string();
    let resolve = |base: &str| url::Url::parse(base).and_then(|base| base.join(&written));
    match (
        resolve("readero://document/"),
        resolve("readero://document/folder/"),
    ) {
        (Ok(root), Ok(nested)) => nested.path().strip_prefix("/folder") == Some(root.path()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The content and navigation documents of a rendered source.
    fn documents(source: &str, title: &str) -> (String, String) {
        let package = package(source.as_bytes(), title).unwrap();
        let text = |name: &str| {
            let (_, bytes) = package.resources.iter().find(|(n, _)| *n == name).unwrap();
            String::from_utf8(bytes.clone()).unwrap()
        };
        (text(CONTENT), text(NAVIGATION))
    }

    /// Every value of an attribute, given the text that precedes it.
    fn values<'a>(html: &'a str, before: &str) -> Vec<&'a str> {
        html.split(before)
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap())
            .collect()
    }

    fn blocks(html: &str) -> Vec<&str> {
        values(html, "data-reader-block=\"")
    }

    /// The outline as the reader sees it: each entry's depth and label.
    fn outline(navigation: &str) -> Vec<(usize, &str)> {
        let mut entries = Vec::new();
        let mut depth = 0usize;
        let mut rest = navigation;
        while let Some(at) = rest.find('<') {
            rest = &rest[at..];
            if rest.starts_with("<ol>") {
                depth += 1;
            } else if rest.starts_with("</ol>") {
                depth -= 1;
            } else if rest.starts_with("<a ") {
                let label = rest.split_once('>').unwrap().1;
                entries.push((depth - 1, label.split("</a>").next().unwrap()));
            }
            rest = &rest[1..];
        }
        assert_eq!(depth, 0, "unbalanced lists in {navigation}");
        assert_eq!(
            navigation.matches("<li>").count(),
            navigation.matches("</li>").count()
        );
        entries
    }

    #[test]
    fn escapes_html_and_active_urls_but_keeps_relative_images() {
        let (html, navigation) = documents(
            "# α heading\n\n<script>bad()</script>\n\n[bad](javascript:alert%281%29)\n\n![figure](figures/example.png)\n\n- [x] Done\n",
            "A & B",
        );
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("javascript:"));
        assert!(html.contains("figures/example.png"));
        assert!(html.contains("<title>A &amp; B</title>"));
        assert!(navigation.contains("α heading"));
        assert!(html.contains("checkbox"));
    }

    #[test]
    fn heading_links_have_unique_readable_fragments() {
        let (html, _) = documents(
            "# A `reading` passage!\n\n# A reading passage!\n\n# A reading passage-1\n\n# Café 🐢\n\n[Jump](#a-reading-passage)",
            "test",
        );
        for id in [
            "a-reading-passage",
            "a-reading-passage-1",
            "a-reading-passage-1-1",
            "café-",
        ] {
            assert!(html.contains(&format!("id=\"{id}\"")));
        }
        assert!(html.contains("href=\"#a-reading-passage\""));
    }

    #[test]
    fn block_identity_survives_insertion_before_unicode_passage() {
        let passage = "# Heading\n\nA passage with 🦀 and café.";
        let before = package(passage.as_bytes(), "test").unwrap();
        let (html, _) = documents(passage, "test");
        let (edited, _) = documents(&format!("An inserted paragraph.\n\n{passage}"), "test");
        let marker = values(&html, "data-reader-block=\"")[1];
        assert!(edited.contains(marker));
        let after = package(
            format!("An inserted paragraph.\n\n{passage}").as_bytes(),
            "test",
        );
        assert_ne!(before.revision, after.unwrap().revision);
    }

    /// Saved positions, bookmarks and passages name block identities, and links
    /// name heading fragments. Neither may drift for source that did not change.
    #[test]
    fn identities_of_unchanged_source_are_stable() {
        let source = "Same.\n\nSame.\n\n# Title <!-- c -->\r\n\r\nBody *text*.\r\n\r\n#\n\n# Section\n\n## Section\n\n## Καλημέρα κόσμε\n\nA passage with 🦀 and café.\n\n- one\n- two\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n\nNote[^n].\n\n[^n]: Footnote.\n\n<div>raw</div>\n";
        let (html, _) = documents(source, "test");
        assert_eq!(
            values(&html, "data-reader-block=\""),
            [
                "b6d4eeb7079408e81-0",
                "b6d4eeb7079408e81-1",
                "bb0c4e12a6750dcca-0",
                "b7a78171691b65391-0",
                "b32c4858e22cc2c96-0",
                "b7b99682411f34d36-0",
                "be35816853a7c0923-0",
                "bb1ede53ccd3b64bc-0",
                "b9f34dec8dd57557c-0",
                "b00a713bf6accdc27-0",
                "b294531383a3ede3a-0",
                "b01c3f38557bc11c4-0",
                "b4233548c84d81449-0",
                "b1c3e68d0239968fd-0",
                "bdeaed9f9451a5ea0-0",
                "b7ae6400a84b99a4e-0",
            ]
        );
        let fragments: Vec<_> = (1..=6)
            .flat_map(|level| values(&html, &format!("<h{level} id=\"")))
            .collect();
        assert_eq!(
            fragments,
            [
                "title-",
                "section",
                "section-1",
                "section-2",
                "καλημέρα-κόσμε"
            ]
        );
        assert_eq!(
            package(source.as_bytes(), "test").unwrap().revision,
            "b51e68ea3b29b4ba218add821f6b79663e41693a50369de292deb3702b9ef50c"
        );
    }

    #[test]
    fn the_outline_nests_by_level_and_lists_only_headings_with_text() {
        let (html, navigation) = documents(
            "# One\n\n## One A\n\n### One A i\n\n## One B\n\n# Two\n\n#### Deep\n\n#\n\n## \n\n# Section\n\nLine one\nline  two\n===\n",
            "test",
        );
        assert_eq!(
            outline(&navigation),
            [
                (0, "One"),
                (1, "One A"),
                (2, "One A i"),
                (1, "One B"),
                (0, "Two"),
                (1, "Deep"),
                (0, "Section"),
                (0, "Line one line two"),
            ]
        );
        // Headings without text stay out of the outline but keep the fragments
        // they always claimed, so a later "Section" is addressed as before. A
        // line break shows as a space in the outline and stays out of the
        // fragment, which links may already name.
        for fragment in ["section", "section-1", "section-2", "line-oneline--two"] {
            assert!(html.contains(&format!("id=\"{fragment}\"")), "{fragment}");
        }
        let (html, _) = documents("One\ntwo\n===\n\n# One two\n", "test");
        assert!(html.contains("id=\"onetwo\"") && html.contains("id=\"one-two\""));
        assert_eq!(outline(&documents("Only text.", "test").1), []);
        assert_eq!(
            outline(&documents("### Deep first\n\n# Then shallow", "test").1),
            [(0, "Deep first"), (0, "Then shallow")]
        );
    }

    #[test]
    fn a_byte_order_mark_is_not_part_of_the_text() {
        let marked = "\u{feff}# Heading\n\nText.";
        let (html, navigation) = documents(marked, "test");
        assert!(html.contains("<h1 id=\"heading\">Heading</h1>"));
        assert_eq!(outline(&navigation), [(0, "Heading")]);
        assert_eq!(
            blocks(&html),
            blocks(&documents("# Heading\n\nText.", "test").0)
        );
        // The revision still describes the bytes on disk.
        assert_ne!(
            package(marked.as_bytes(), "test").unwrap().revision,
            package(b"# Heading\n\nText.", "test").unwrap().revision
        );
    }

    #[test]
    fn leading_front_matter_is_hidden_and_the_body_reads_as_it_would_alone() {
        let body = "# Real heading\n\nBody text.\n";
        let expected = documents(body, "test");
        for front in [
            "---\ntitle: Hello\ntags: [a, b]\n---\n",
            "---\r\ntitle: Hello\r\n---\r\n",
            "---  \ntitle:\n  nested: Hello\n...\n",
            "---\ntitle: Hello\ntags:\n- a\n- b\n-\n\tdeep: c\n---\n",
            "\u{feff}---\ntitle: Hello\n---\n",
            "---\nτίτλος: Hello\n---\n\n",
        ] {
            let (html, navigation) = documents(&format!("{front}{body}"), "test");
            assert_eq!(html, expected.0, "{front:?}");
            assert_eq!(navigation, expected.1, "{front:?}");
        }
        assert!(matches!(
            package(b"---\ntitle: Only metadata\n---\n", "test"),
            Err(Error::Empty)
        ));
    }

    #[test]
    fn rules_and_prose_are_not_mistaken_for_front_matter() {
        for (source, shown) in [
            ("---\nA sentence, not a key.\n\n---\n", "A sentence"),
            (
                "---\n\ntitle: after a blank\n\n---\n",
                "title: after a blank",
            ),
            ("---\ntitle: never closed\n", "title: never closed"),
            (
                "Intro.\n\n---\ntitle: mid-document\n---\n",
                "title: mid-document",
            ),
            ("----\ntitle: four dashes\n---\n", "title: four dashes"),
            ("---\nhttps://example.com/x\n---\n", "https://example.com/x"),
            ("---\ntitle:no space\n---\n", "title:no space"),
            ("---\ndate created: today\n---\n", "date created: today"),
            // One line that looks like a key does not make what follows metadata.
            (
                "---\nNote: Read this\n\nA paragraph.\n---\n",
                "A paragraph.",
            ),
            (
                "---\nNote: Read this\nA paragraph.\n---\n",
                "Note: Read this",
            ),
            ("---\ntitle: x\n# A heading\n---\n", "A heading"),
            ("---\ntitle: x\n```\n---\n```\n", "title: x"),
            // The parser ends a line at a lone carriage return as well.
            ("---\ntitle: x\r\rRead this.\n---\n", "Read this."),
            ("---\ntitle: x\r# A heading\n---\n", "A heading"),
            ("---\ntitle: x\r\r\nQ: prose\n---\n", "Q: prose"),
            // Only spaces and tabs are blank to the parser.
            ("---\u{a0}\ntitle: Read this.\n---\n", "Read this."),
            ("---\ntitle: Read this.\n---\u{a0}\n", "Read this."),
            ("---\ntitle:\u{a0}Read this.\n---\n", "Read this."),
            ("---\ntitle: x\n\u{2003}\nRead this.\n---\n", "Read this."),
        ] {
            let (html, _) = documents(&format!("{source}\nKept."), "test");
            assert!(html.contains(shown) && html.contains("Kept."), "{source:?}");
        }
    }

    #[test]
    fn a_comment_standing_alone_is_hidden_and_other_raw_html_is_shown() {
        let (html, _) = documents(
            "<!-- a note -->\n\n<!-- several\nlines -->\n\nBefore <!-- inline\nnote --> after.\n\n> <!-- quoted -->\n> Quoted text.\n",
            "test",
        );
        assert!(
            !html.contains("&lt;!--") && !html.contains("note"),
            "{html}"
        );
        assert!(html.contains("Before  after.") && html.contains("Quoted text."));
        // Only the paragraph and the quotation remain as blocks.
        assert_eq!(blocks(&html).len(), 2);

        let (html, _) = documents(
            "<!-- kept --> with text\n\n<div>raw</div>\n\n<kbd>Ctrl</kbd>\n\n<!-- one --><!-- two -->\n\n```\n<!-- code -->\n```\n",
            "test",
        );
        for shown in [
            "&lt;!-- kept --&gt; with text",
            "&lt;div&gt;raw&lt;/div&gt;",
            "&lt;kbd&gt;Ctrl&lt;/kbd&gt;",
            "&lt;!-- one --&gt;&lt;!-- two --&gt;",
            "&lt;!-- code --&gt;",
        ] {
            assert!(html.contains(shown), "{shown}");
        }
        // An unclosed comment swallows the rest of the source as raw HTML.
        let (html, _) = documents("Shown.\n\n<!-- unclosed\n\n# Also shown\n", "test");
        assert!(html.contains("&lt;!-- unclosed") && html.contains("# Also shown"));
        // A browser ends these comments early and shows the text after them.
        for malformed in [
            "<!-->Read this.<!-- end -->",
            "<!--->Read this.<!-- end -->",
            "<!-- a --!>Read this.<!-- end -->",
            "Inline <!-- a --!>Read this.<!-- end --> text",
        ] {
            let (html, _) = documents(malformed, "test");
            assert!(html.contains("Read this."), "{malformed}");
        }
        for comment in ["<!---->", "<!-- a - b -- c --->", "   <!-- indented -->  "] {
            assert!(is_comment(comment), "{comment}");
        }
    }

    #[test]
    fn what_surrounds_a_hidden_comment_is_rendered_as_usual() {
        let (html, _) = documents(
            "A note[^x].\n\n[^x]: <!-- hidden -->\n\n> <!-- hidden -->\n\n- <!-- hidden -->\n- Visible\n\n| <!-- hidden --> | b |\n|---|---|\n| 1 | 2 |\n\n# <!-- hidden -->\n\n# Section\n",
            "test",
        );
        assert!(!html.contains("hidden"), "{html}");
        for kept in [
            "id=\"x\"",
            "<blockquote>",
            "<li></li>",
            "<li>Visible</li>",
            "<th></th>",
            "<h1 id=\"section\"></h1>",
            "<h1 id=\"section-1\">Section</h1>",
        ] {
            assert!(html.contains(kept), "{kept}");
        }
        assert_eq!(blocks(&html).len(), 7);
    }

    #[test]
    fn a_source_that_shows_nothing_is_empty() {
        for hidden in [
            "<!-- nothing else -->\n",
            "<!-- one -->\n\n<!-- two -->\n",
            "[label]: https://example.org\n",
            "---\ntitle: x\n---\n<!-- note -->\n",
        ] {
            assert!(
                matches!(package(hidden.as_bytes(), "test"), Err(Error::Empty)),
                "{hidden:?}"
            );
        }
        assert!(package(b"> <!-- an empty quotation is still one -->\n", "test").is_ok());
    }

    #[test]
    fn an_image_above_the_documents_folder_is_not_swapped_for_another_file() {
        // The reader resolves `src` as a URL: an encoded slash does not
        // separate folders there, so `a%2fb` is one level to climb, not two,
        // while an empty segment between two slashes is a level of its own.
        for (destination, kept) in [
            ("../up.png", false),
            ("%2e%2e/up.png", false),
            ("a/../../up.png", false),
            ("a%2fb/../../up.png", false),
            ("./.%2E/up.png", false),
            ("sub/../ok.png", true),
            ("sub//../../ok.png", true),
            ("a%2fb/ok.png", true),
            ("figures/one.png?v=../..#../top", true),
        ] {
            let (html, _) = documents(&format!("![image]({destination})"), "test");
            let expected = if kept { destination } else { "" };
            assert_eq!(values(&html, "<img src=\""), [expected], "{destination}");
        }
        // A space or tab is written encoded, so it is part of a folder's name
        // and the dots beside it do not climb.
        for (destination, written) in [
            ("< ../figure.png>", "%20../figure.png"),
            ("<.\t./figure.png>", ".%09./figure.png"),
            ("<a b/../c.png>", "a%20b/../c.png"),
        ] {
            let (html, _) = documents(&format!("![image]({destination})"), "test");
            assert_eq!(values(&html, "<img src=\""), [written], "{destination}");
        }
        // Links are followed through the folder check when they are opened.
        let (html, _) = documents("[up](../other.md)", "test");
        assert!(html.contains("href=\"../other.md\""));
    }

    /// The folder check has to see a destination exactly as the reader will.
    #[test]
    fn destinations_are_judged_as_the_renderer_writes_them() {
        let destination: String = (0u8..128).map(char::from).chain(['é', '🦀']).collect();
        let image = Tag::Image {
            link_type: pulldown_cmark::LinkType::Inline,
            dest_url: destination.as_str().into(),
            title: "".into(),
            id: "".into(),
        };
        let mut written = String::new();
        html::push_html(
            &mut written,
            [Event::Start(image), Event::End(TagEnd::Image)].into_iter(),
        );
        // The two characters written as HTML entities reach the reader as themselves.
        let src = values(&written, "<img src=\"")[0]
            .replace("&amp;", "&")
            .replace("&#x27;", "'");
        assert_eq!(
            src,
            utf8_percent_encode(&destination, ENCODED_IN_DESTINATION).to_string()
        );
    }

    #[test]
    fn a_link_fragment_addresses_the_heading_by_its_text() {
        let (html, _) = documents("# Καφές notes\n\n[there](other.md#καφές-notes)\n", "test");
        assert!(html.contains("<h1 id=\"καφές-notes\">"));
        let encoded = values(&html, "href=\"other.md#")[0];
        assert!(encoded.contains('%'), "{encoded}");
        assert_eq!(heading_href(encoded), "content.xhtml#καφές-notes");
        assert_eq!(
            heading_href("linked-passage"),
            "content.xhtml#linked-passage"
        );
    }

    #[test]
    fn a_package_names_its_resources_in_a_fixed_order() {
        let package = package(b"# Text", "A <b> & \"q\"").unwrap();
        let names: Vec<_> = package.resources.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "META-INF/container.xml",
                "package.opf",
                "content.xhtml",
                "nav.xhtml"
            ]
        );
        let manifest = String::from_utf8(package.resources[1].1.clone()).unwrap();
        assert!(manifest.contains("<dc:title>A &lt;b&gt; &amp; &quot;q&quot;</dc:title>"));
        assert_eq!(
            heading_href("linked-passage"),
            "content.xhtml#linked-passage"
        );
    }

    #[test]
    fn text_that_is_not_utf8_or_is_blank_is_refused() {
        let invalid = package(b"\xff\xfe#\0 \0x\0", "test").err().unwrap();
        assert!(invalid.to_string().contains("UTF-8"));
        for blank in [" \n\n", "\u{a0}\n", "\u{feff}", "\u{feff}\n\u{2003}\n"] {
            assert!(
                matches!(package(blank.as_bytes(), "test"), Err(Error::Empty)),
                "{blank:?}"
            );
        }
    }
}
