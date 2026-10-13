use super::*;
use readero::{
    document::{Error, Result},
    passage::{
        Capture, MAX_PDF_PAGES, MAX_QUOTE_BYTES, Passage, PdfFragment, Target, unique_match,
    },
};

pub struct ResolvedPassage {
    locator: Locator,
    regions: Vec<(i32, Vec<[f64; 4]>)>,
    pub recovered: bool,
}

fn failure() -> Error {
    Error::Invalid(
        "The saved words could not be identified confidently. This passage needs relinking.".into(),
    )
}
fn rectangle(points: [f64; 4]) -> document::Rectangle {
    document::Rectangle::from_coordinates(points[0], points[1], points[2], points[3])
}
fn style(value: i32) -> document::SelectionStyle {
    match value {
        1 => document::SelectionStyle::Word,
        2 => document::SelectionStyle::Line,
        _ => document::SelectionStyle::Glyph,
    }
}
fn context(
    text: &str,
    quote: &str,
    selected: &gtk::cairo::Region,
    layout: Option<&[document::Rectangle]>,
) -> (String, String) {
    let offset = unique_match(text, quote, "", "").or_else(|| {
        let layout = layout?;
        let mut found = None;
        let mut offset = 0;
        while let Some(relative) = text[offset..].find(quote) {
            let start = offset + relative;
            let first = text[..start].chars().count();
            let glyphs = layout.get(first..first + quote.chars().count())?;
            let inside = |r: &document::Rectangle| {
                selected.contains_point(
                    ((r.x1() + r.x2()) / 2.0) as i32,
                    ((r.y1() + r.y2()) / 2.0) as i32,
                )
            };
            let mut visible = glyphs.iter().filter(|r| r.x2() > r.x1() && r.y2() > r.y1());
            if let Some(first) = visible.next()
                && inside(first)
                && visible.next_back().is_none_or(inside)
            {
                if found.is_some() {
                    return None;
                }
                found = Some(start);
            }
            offset = start + text[start..].chars().next()?.len_utf8();
        }
        found
    });
    offset.map_or_else(
        || (String::new(), String::new()),
        |start| {
            let prefix: String = text[..start]
                .chars()
                .rev()
                .take(128)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let suffix = text[start + quote.len()..].chars().take(128).collect();
            (prefix, suffix)
        },
    )
}

impl Pdf {
    #[cfg(feature = "smoke")]
    pub fn passage_highlighted(&self) -> bool {
        !self.passage_regions.borrow().is_empty()
    }

    pub fn capture_selection(&self, revision: &str) -> Result<Capture> {
        if self.model.rotation() != 0 {
            return Err(Error::Invalid(
                "Rotate the PDF upright before selecting text.".into(),
            ));
        }
        if !self.view.has_selection() {
            return Err(Error::Invalid(
                "Select text in the document first. Scanned pages may have no selectable text."
                    .into(),
            ));
        }
        let selections = self.view.selections();
        if selections.len() > MAX_PDF_PAGES {
            return Err(Error::Invalid(
                "Select a shorter passage (at most 32 PDF pages).".into(),
            ));
        }
        let doc = self.model.document().ok_or_else(failure)?;
        let selector = doc
            .dynamic_cast_ref::<document::Selection>()
            .ok_or_else(failure)?;
        let text_doc = doc.dynamic_cast_ref::<document::DocumentText>();
        let mut fragments = Vec::new();
        let mut bytes = 0;
        for selection in selections {
            let page = doc.page(selection.page()).ok_or_else(failure)?;
            let mut rect = selection.rect();
            let quote = selector
                .selected_text(&page, selection.style(), &mut rect)
                .ok_or_else(failure)?
                .to_string();
            if quote.trim().is_empty() {
                continue;
            }
            bytes += quote.len();
            if bytes > MAX_QUOTE_BYTES {
                return Err(Error::Invalid(
                    "Select a shorter passage (at most 64 KiB of text).".into(),
                ));
            }
            let text = text_doc.and_then(|t| t.text(&page)).unwrap_or_default();
            let rc =
                document::RenderContext::new(&page, 0, 1.0, document::RenderAnnotsFlags::empty());
            let region = selector
                .selection_region(&rc, selection.style(), &mut rect)
                .ok_or_else(failure)?;
            let layout = if unique_match(&text, &quote, "", "").is_none() {
                text_doc.and_then(|t| t.text_layout(&page))
            } else {
                None
            };
            let (prefix, suffix) = context(&text, &quote, &region, layout.as_deref());
            fragments.push(PdfFragment {
                page: selection.page(),
                points: [rect.x1(), rect.y1(), rect.x2(), rect.y2()],
                style: match selection.style() {
                    document::SelectionStyle::Word => 1,
                    document::SelectionStyle::Line => 2,
                    _ => 0,
                },
                quote,
                prefix,
                suffix,
            });
        }
        fragments.sort_by_key(|f| f.page);
        let first = fragments.first().ok_or_else(failure)?;
        let locator = Locator {
            version: 1,
            anchor: Anchor::Pdf {
                page: first.page,
                x: first.points[0],
                y: first.points[1],
                viewport_y: 24.0,
            },
        };
        let passage = Passage {
            quote: self.view.selected_text().ok_or_else(failure)?.to_string(),
            revision: revision.into(),
            target: Target::Pdf { fragments },
        };
        passage.validate(&locator)?;
        Ok(Capture { locator, passage })
    }

    /// Resolve only saved pages. Every candidate is re-extracted before display.
    pub fn resolve_passage(&self, passage: &Passage) -> Result<ResolvedPassage> {
        let Target::Pdf { fragments } = &passage.target else {
            return Err(failure());
        };
        let doc = self.model.document().ok_or_else(failure)?;
        let selector = doc
            .dynamic_cast_ref::<document::Selection>()
            .ok_or_else(failure)?;
        let text_doc = doc
            .dynamic_cast_ref::<document::DocumentText>()
            .ok_or_else(failure)?;
        let mut resolved = Vec::new();
        let mut first = None;
        let mut recovered = false;
        for fragment in fragments {
            if fragment.page >= doc.n_pages() {
                return Err(failure());
            }
            let page = doc.page(fragment.page).ok_or_else(failure)?;
            let (width, height) = doc.page_size(fragment.page);
            let in_bounds = fragment.points[0].max(fragment.points[2]) <= width
                && fragment.points[1].max(fragment.points[3]) <= height;
            let mut points = rectangle(fragment.points);
            let mut selection_style = style(fragment.style);
            let current = if in_bounds {
                selector
                    .selected_text(&page, selection_style, &mut points)
                    .unwrap_or_default()
            } else {
                glib::GString::default()
            };
            let text = text_doc.text(&page).ok_or_else(failure)?;
            let rc =
                document::RenderContext::new(&page, 0, 1.0, document::RenderAnnotsFlags::empty());
            let verified_context = if current.as_str() == fragment.quote {
                let region = selector
                    .selection_region(&rc, selection_style, &mut points)
                    .ok_or_else(failure)?;
                let layout = if unique_match(&text, &fragment.quote, "", "").is_none() {
                    text_doc.text_layout(&page)
                } else {
                    None
                };
                let (prefix, suffix) = context(&text, &fragment.quote, &region, layout.as_deref());
                prefix.ends_with(&fragment.prefix) && suffix.starts_with(&fragment.suffix)
            } else {
                false
            };
            if !verified_context {
                let offset =
                    unique_match(&text, &fragment.quote, &fragment.prefix, &fragment.suffix)
                        .ok_or_else(failure)?;
                let start = text[..offset].chars().count();
                let end = start + fragment.quote.chars().count();
                let layout = text_doc.text_layout(&page).ok_or_else(failure)?;
                let a = layout.get(start).ok_or_else(failure)?;
                let b = layout.get(end - 1).ok_or_else(failure)?;
                points = document::Rectangle::from_coordinates(
                    a.x1(),
                    (a.y1() + a.y2()) / 2.0,
                    b.x2(),
                    (b.y1() + b.y2()) / 2.0,
                );
                selection_style = document::SelectionStyle::Glyph;
                if selector
                    .selected_text(&page, selection_style, &mut points)
                    .as_deref()
                    != Some(&fragment.quote)
                {
                    return Err(failure());
                }
                recovered = true;
            }
            let rc =
                document::RenderContext::new(&page, 0, 1.0, document::RenderAnnotsFlags::empty());
            let region = selector
                .selection_region(&rc, selection_style, &mut points)
                .ok_or_else(failure)?;
            let rectangles: Vec<_> = (0..region.num_rectangles())
                .map(|i| {
                    let r = region.rectangle(i);
                    [
                        f64::from(r.x()),
                        f64::from(r.y()),
                        f64::from(r.x() + r.width()),
                        f64::from(r.y() + r.height()),
                    ]
                })
                .collect();
            if first.is_none() {
                let first_rect = rectangles.first().ok_or_else(failure)?;
                let bounds = rectangles.iter().fold(*first_rect, |b, r| {
                    [
                        b[0].min(r[0]),
                        b[1].min(r[1]),
                        b[2].max(r[2]),
                        b[3].max(r[3]),
                    ]
                });
                // Reveal the top of the ink in the current rotation. An arbitrary
                // drag endpoint or page corner can place all selected text above
                // the viewport, especially at 180 and 270 degrees.
                let (x, y) = match self.model.rotation() {
                    90 => (bounds[0], bounds[3]),
                    180 => (bounds[2], bounds[3]),
                    270 => (bounds[2], bounds[1]),
                    _ => (bounds[0], bounds[1]),
                };
                first = Some(Locator {
                    version: 1,
                    anchor: Anchor::Pdf {
                        page: fragment.page,
                        x,
                        y,
                        viewport_y: 24.0,
                    },
                });
            }
            resolved.push((fragment.page, rectangles));
        }
        Ok(ResolvedPassage {
            locator: first.ok_or_else(failure)?,
            regions: resolved,
            recovered,
        })
    }

    pub fn show_passage(self: &Rc<Self>, passage: ResolvedPassage) {
        self.passage_regions.replace(passage.regions);
        self.restore(Some(passage.locator));
        self.passage_overlay.queue_draw();
    }

    pub fn clear_passage(&self) {
        if !self.passage_regions.borrow().is_empty() {
            self.passage_regions.borrow_mut().clear();
            self.passage_overlay.queue_draw();
        }
    }

    pub(super) fn wire_passage_overlay(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.passage_overlay
            .set_draw_func(move |area, context, width, height| {
                let Some(pdf) = weak.upgrade() else {
                    return;
                };
                if pdf.passage_regions.borrow().is_empty() {
                    return;
                }
                let color = area.color();
                context.set_source_rgba(
                    f64::from(color.red()),
                    f64::from(color.green()),
                    f64::from(color.blue()),
                    0.3,
                );
                for (page, rectangles) in pdf.passage_regions.borrow().iter() {
                    // The inverse mapping is public; sample one visible point on the
                    // target page and apply Papers' scale/rotation to its line regions.
                    let mut origin = None;
                    'scan: for y in (0..height).step_by(24) {
                        for x in [width / 2, width / 4, width * 3 / 4] {
                            if let Some(point) = pdf
                                .view
                                .document_point_for_view_point(f64::from(x), f64::from(y))
                                && point.page_index() == *page
                            {
                                origin = Some((x, y, point.point_on_page()));
                                break 'scan;
                            }
                        }
                    }
                    let Some((x, y, origin)) = origin else {
                        continue;
                    };
                    let convert = |dx: f64, dy: f64| {
                        let dx = (dx - origin.x()) * pdf.model.scale();
                        let dy = (dy - origin.y()) * pdf.model.scale();
                        let (dx, dy) = match pdf.model.rotation() {
                            90 => (-dy, dx),
                            180 => (-dx, -dy),
                            270 => (dy, -dx),
                            _ => (dx, dy),
                        };
                        (f64::from(x) + dx, f64::from(y) + dy)
                    };
                    for r in rectangles {
                        let (x1, y1) = convert(r[0], r[1]);
                        let (x2, y2) = convert(r[2], r[3]);
                        context.rectangle(x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs());
                    }
                }
                let _ = context.fill();
            });
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(pdf) = weak.upgrade() {
                pdf.clear_passage();
            }
            glib::Propagation::Proceed
        });
        self.view.add_controller(scroll);
        let weak = Rc::downgrade(self);
        self.view.connect_selection_changed(move |_| {
            if let Some(pdf) = weak.upgrade() {
                pdf.clear_passage();
            }
        });
    }
}
