use designcraft_compose::{Cache, highlight_quads};
use designcraft_doc::build::NewDocument;
use designcraft_doc::{Document, ItemId, ParaFormat, SpreadRef, StoryId};
use designcraft_geom::{Affine, Rect};

use crate::{page_infos, text_layer};

/// A two-page document with one text frame on its second page.
fn second_page_frame(text: &str) -> (Document, StoryId, ItemId) {
    let mut d = Document::new(&NewDocument { pages: 2, facing_pages: false, ..Default::default() });
    let layer = d.default_layer();
    let (item, story) = d.add_text_frame(SpreadRef::Doc(1), Rect::new(72.0, 72.0, 400.0, 300.0), layer, text, ParaFormat::default()).unwrap();
    (d, story, item)
}

#[test]
fn a_frame_on_the_second_page_gives_a_line_in_that_page_s_coordinates() {
    let (d, sid, item) = second_page_frame("Hello review world");
    let cache = Cache::new();
    let (layer, warnings) = text_layer(&d, &cache);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(layer.pages.len(), 2);
    assert!(layer.pages[0].lines.is_empty());
    let lines = &layer.pages[1].lines;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let l = &lines[0];
    assert_eq!((l.story, l.frame, l.cell, l.footnote), (sid.0, item.0, None, None));
    assert_eq!(l.text, "Hello review world");
    assert_eq!((l.start, l.end), (0, 18));
    assert_eq!(l.bytes.len(), l.text.chars().count());
    assert_eq!(l.bytes[6], 6);
    assert!(!l.rtl);
    // Where the glyphs are: the line's highlight quads through the frame's transform, in the page.
    let cs = cache.get(&d, sid, None);
    let (loc, it) = (d.find(item).unwrap(), d.item(item).unwrap());
    let SpreadRef::Doc(si) = loc.spread else { panic!("not on a spread") };
    let m: Affine = d.parent_xf(&loc) * d.text_xf(it);
    let page = d.spreads[si].pages[0].bounds();
    let line = &cs.frames[0].lines[0];
    let mut expect: Option<Rect> = None;
    for q in highlight_quads(line, line.range.start, line.range.end, false) {
        for p in q {
            let p = m * p;
            expect = Some(expect.map_or(Rect::from_points(p, p), |r| r.union_pt(p)));
        }
    }
    let expect = expect.unwrap() - page.origin().to_vec2();
    for (got, want) in l.rect.iter().zip([expect.x0, expect.y0, expect.x1, expect.y1]) {
        assert!((got - want).abs() < 1e-6, "{:?} vs {expect:?}", l.rect);
    }
    assert!(expect.x0 >= 72.0 && expect.y0 >= 72.0 && expect.x1 <= 400.0, "{expect:?}");
}

#[test]
fn page_infos_name_and_size_every_page() {
    let (d, ..) = second_page_frame("x");
    let pages = page_infos(&d);
    assert_eq!(pages.len(), 2);
    assert_eq!((pages[1].index, pages[1].label.as_str(), pages[1].spread), (1, "2", 1));
    assert_eq!((pages[1].width, pages[1].height), (612.0, 792.0));
    assert_eq!(pages[0].image, "pages/001.webp");
}
