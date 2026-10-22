//! DesignCraft Review: what a reviewer's browser gets from a document.
//!
//! A *review bundle* is a folder: `manifest.json` (the document, its version and its pages),
//! one image per page, `text.json` (the text layer: every line of text with its story, byte
//! range, characters and rectangle, so a selection in the browser becomes a `text.select`
//! address in the app) and the PDF. This crate builds the manifest's page list and the text
//! layer; the engine renders the pages, exports the PDF and writes the folder (`review.export`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use designcraft_compose::{Cache, Line, PathTextMap, highlight_quads};
use designcraft_doc::{CellAddr, Document, SpreadRef};
use designcraft_geom::{Affine, Rect};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// A page in the manifest's page list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    /// Absolute page index, from 0.
    pub index: usize,
    /// The page's name as the document numbers it ("ii", "7").
    pub label: String,
    /// The spread it is on.
    pub spread: usize,
    /// Trim size in points.
    pub width: f64,
    pub height: f64,
    /// The page's image, relative to the bundle folder.
    pub image: String,
}

/// The text layer: every page's lines.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextLayer {
    pub pages: Vec<PageText>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageText {
    /// Absolute page index, from 0.
    pub index: usize,
    pub lines: Vec<LineText>,
}

/// One line of text as laid out: where it is on the page and what it says, with the address
/// `text.select` takes for each character. Glyphs are in the order they are drawn.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineText {
    pub story: u64,
    /// The table cell the text is in (offsets are then into the cell's text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<CellAddr>,
    /// The footnote the text is in (offsets are then into the footnote's text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footnote: Option<u64>,
    /// The text frame it is drawn in.
    pub frame: u64,
    /// UTF-8 byte range in the story (or the cell's or footnote's text).
    pub start: usize,
    pub end: usize,
    /// The characters drawn, in order.
    pub text: String,
    /// The byte offset each character of `text` starts at.
    pub bytes: Vec<u32>,
    /// Left, top, right, bottom in points from the page's top-left trim corner.
    pub rect: [f64; 4],
    /// Right-to-left text.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rtl: bool,
}

/// The page image's name in the bundle: `pages/001.webp` for the first page.
pub fn page_image(index: usize) -> String {
    format!("pages/{:03}.webp", index.saturating_add(1))
}

/// Every page of `doc`, in order.
pub fn page_infos(doc: &Document) -> Vec<PageInfo> {
    let mut out = Vec::new();
    for (si, sp) in doc.spreads.iter().enumerate() {
        for p in &sp.pages {
            let index = out.len();
            let b = p.bounds();
            out.push(PageInfo { index, label: doc.page_name(index), spread: si, width: b.width(), height: b.height(), image: page_image(index) });
        }
    }
    out
}

/// The text layer of `doc`, and warnings naming the text it leaves out (type on a path and
/// vertical frames, for now).
pub fn text_layer(doc: &Document, cache: &Cache) -> (TextLayer, Vec<String>) {
    let mut pages: Vec<PageText> = page_infos(doc).into_iter().map(|p| PageText { index: p.index, lines: vec![] }).collect();
    let mut warnings = Vec::new();
    // `stories` is a BTreeMap: the order is stable.
    for &sid in doc.stories.keys() {
        let cs = cache.get(doc, sid, None);
        for ft in &cs.frames {
            let (Some(loc), Some(it)) = (doc.find(ft.frame), doc.item(ft.frame)) else { continue };
            let SpreadRef::Doc(si) = loc.spread else { continue };
            if ft.vertical {
                warnings.push(format!("vertical text (frame {}) isn't in the text layer", ft.frame.0));
                continue;
            }
            if PathTextMap::new(it, ft).is_some() {
                warnings.push(format!("type on a path (frame {}) isn't in the text layer", ft.frame.0));
                continue;
            }
            let m = doc.parent_xf(&loc) * doc.text_xf(it);
            let mut add = |lines: &[Line], m: Affine, at: Option<CellAddr>| {
                for l in lines {
                    if let Some((abs, line)) = line_text(doc, si, sid.0, ft.frame.0, at, l, m)
                        && let Some(page) = pages.get_mut(abs)
                    {
                        page.lines.push(line);
                    }
                }
            };
            add(&ft.lines, m, None);
            // A cell's or a footnote's text is composed on its own and placed in the frame.
            for tf in &ft.tables {
                for pc in &tf.cells {
                    if let Some(f0) = pc.text.frames.first() {
                        add(&f0.lines, m * Affine::translate(pc.origin.to_vec2()), Some(CellAddr { table: tf.table, row: pc.row, col: pc.col }));
                    }
                }
            }
            for n in &ft.notes {
                if let Some(f0) = n.text.frames.first() {
                    add(&f0.lines, m * Affine::translate(n.origin.to_vec2()), Some(CellAddr::footnote(n.id)));
                }
            }
        }
    }
    (TextLayer { pages }, warnings)
}

/// Line `l` of spread `si`, drawn through `m` (its text space to the spread): the page it is
/// on and its record, unless it has nothing to show or lies off every page.
fn line_text(doc: &Document, si: usize, story: u64, frame: u64, at: Option<CellAddr>, l: &Line, m: Affine) -> Option<(usize, LineText)> {
    let (mut text, mut bytes) = (String::new(), Vec::new());
    for g in &l.glyphs {
        // Control characters and markers' carriers (private use) are drawn as nothing.
        if g.len > 0 && !g.rendered_char.is_control() && !('\u{E000}'..='\u{F8FF}').contains(&g.rendered_char) {
            text.push(g.rendered_char);
            bytes.push(u32::try_from(g.byte).ok()?);
        }
    }
    if text.is_empty() {
        return None;
    }
    let mut bbox: Option<Rect> = None;
    for q in highlight_quads(l, l.range.start, l.range.end, false) {
        for p in q {
            let p = m * p;
            bbox = Some(bbox.map_or(Rect::from_points(p, p), |r| r.union_pt(p)));
        }
    }
    let r = bbox?;
    let sp = doc.spreads.get(si)?;
    let c = r.center();
    let pi = sp.pages.iter().position(|p| p.bounds().contains(c))?;
    let page = sp.pages.get(pi)?.bounds();
    let abs = doc.first_page_of_spread(si).checked_add(pi)?;
    let r = r - page.origin().to_vec2();
    let (footnote, cell) = match at {
        Some(a) => match a.footnote_id() {
            Some(id) => (Some(id), None),
            None => (None, Some(a)),
        },
        None => (None, None),
    };
    let rtl = l.glyphs.iter().find(|g| g.len > 0).is_some_and(|g| g.rtl);
    Some((abs, LineText { story, cell, footnote, frame, start: l.range.start, end: l.range.end, text, bytes, rect: [r.x0, r.y0, r.x1, r.y1], rtl }))
}
