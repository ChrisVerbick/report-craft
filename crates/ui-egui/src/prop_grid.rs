//! The Properties panel's grid: six tracks across a section and an even row step, so every
//! section's labels and fields line up.

use egui::{Rect, Ui, vec2};

use crate::theme::Tokens;

/// A column of the grid. The lead holds a section's own widget (the reference point, a swatch, a
/// checkbox), each slot a field's label or icon, and the trail a link toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    Lead,
    SlotA,
    FieldA,
    SlotB,
    FieldB,
    Trail,
}

const LEAD: f32 = 26.0;
const SLOT: f32 = 16.0;
const GAP: f32 = 4.0;

/// Rows laid over the tracks at the cursor.
pub struct PropGrid {
    origin: egui::Pos2,
    row_h: f32,
    step: f32,
    tracks: [(f32, f32); 6],
}

impl PropGrid {
    /// (x offset, width) of each track across `width`: lead 26, slots and trail 16, the two
    /// fields share the rest, 4 px between tracks; widths clamp at 0.
    pub fn tracks(width: f32) -> [(f32, f32); 6] {
        let width = if width.is_finite() { width.max(0.0) } else { 0.0 };
        let field = ((width - LEAD - 3.0 * SLOT - 5.0 * GAP) / 2.0).max(0.0);
        let mut x = 0.0;
        [LEAD, SLOT, field, SLOT, field, SLOT].map(|w| {
            let at = x;
            x += w + GAP;
            (at, w)
        })
    }

    /// Allocates `rows` rows (`row_h` each, `row_gap` between) at the cursor, across
    /// [`full_width`](crate::widgets::full_width): the width `tracks(full_width(ui))` describes
    /// before allocating.
    pub fn new(ui: &mut Ui, t: &Tokens, rows: usize) -> PropGrid {
        let tracks = Self::tracks(crate::widgets::full_width(ui));
        let n = rows as f32;
        let height = (n * t.row_h + (n - 1.0) * t.row_gap).max(0.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
        PropGrid { origin: rect.min, row_h: t.row_h, step: t.row_h + t.row_gap, tracks }
    }

    pub fn cell(&self, row: usize, track: Track) -> Rect {
        self.span(row, track, track)
    }

    pub fn span(&self, row: usize, from: Track, to: Track) -> Rect {
        self.span_rows(row..row + 1, from, to)
    }

    /// The tracks `from` to `to` over `rows`, with the gaps between them.
    pub fn span_rows(&self, rows: std::ops::Range<usize>, from: Track, to: Track) -> Rect {
        let (from, to) = if from as usize <= to as usize { (from, to) } else { (to, from) };
        let (x0, _) = self.tracks[from as usize];
        let (x1, w1) = self.tracks[to as usize];
        let last = rows.end.saturating_sub(1).max(rows.start);
        let top = rows.start as f32 * self.step;
        let bottom = last as f32 * self.step + self.row_h;
        Rect::from_min_max(self.origin + vec2(x0, top), self.origin + vec2(x1 + w1, bottom))
    }
}

/// Whether `text`, in the label font, fits a box `width` wide that draws `chrome` px around its
/// value ([`NUM_CHROME`](crate::widgets::NUM_CHROME) and its siblings). The pair rule: a pair
/// of boxes shares a row only when both show their usual values whole.
pub fn value_fits(ui: &Ui, chrome: f32, text: &str, width: f32) -> bool {
    let text_w = ui.painter().layout_no_wrap(text.to_owned(), crate::widgets::label_font(ui), egui::Color32::PLACEHOLDER).size().x;
    chrome + text_w <= width
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Tokens;
    use crate::widgets::{DROPDOWN_CHROME, NUM_CHROME};
    use egui::{Rect, pos2, vec2};

    fn today() -> Tokens {
        Tokens::for_brightness(Default::default())
    }

    fn raw(width: f32) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 400.0))), ..Default::default() }
    }

    /// Runs `f` in a frame `width` wide, after a first frame that loads the bundled fonts.
    fn frame(t: &Tokens, width: f32, f: &mut dyn FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, "");
        crate::theme::apply(&ctx, t);
        for i in 0..2 {
            let mut out = ctx.run_ui(raw(width), |ui| {
                if i == 1 {
                    f(ui)
                }
            });
            out.textures_delta.clear();
        }
    }

    /// Where the tracks land at the widths that docks of 220, 253, 280, 300 (comfortable) and
    /// 420 pt give their sections.
    #[test]
    fn grid_tracks_at_every_width() {
        for (w, field) in [(189.0, 47.5), (222.0, 64.0), (249.0, 77.5), (265.0, 85.5), (389.0, 147.5)] {
            let t = PropGrid::tracks(w);
            assert_eq!(t[0], (0.0, 26.0), "{w}");
            assert_eq!(t[1], (30.0, 16.0), "{w}");
            assert_eq!(t[2], (50.0, field), "{w}");
            assert_eq!(t[4].1, field, "{w}");
            assert_eq!(t[5].0 + t[5].1, w, "{w}");
        }
        for w in [0.0, f32::NAN, -50.0] {
            for (x, tw) in PropGrid::tracks(w) {
                assert!(x >= 0.0 && tw >= 0.0, "{w}: {x} {tw}");
            }
        }
    }

    #[test]
    fn spans_cover_their_tracks() {
        let t = today();
        frame(&t, 300.0, &mut |ui| {
            let g = PropGrid::new(ui, &t, 3);
            let s = g.span(1, Track::SlotA, Track::FieldB);
            assert_eq!(s.min.x, g.cell(1, Track::SlotA).min.x);
            assert_eq!(s.max.x, g.cell(1, Track::FieldB).max.x);
            assert_eq!((s.min.y, s.max.y), (g.cell(1, Track::Lead).min.y, g.cell(1, Track::Lead).max.y));
            let r = g.span_rows(0..2, Track::Lead, Track::Lead);
            assert_eq!(r.min.y, g.cell(0, Track::Lead).min.y);
            assert_eq!(r.max.y, g.cell(1, Track::Lead).max.y);
            assert_eq!(r.height(), 2.0 * t.row_h + t.row_gap);
        });
    }

    #[test]
    fn rows_step_by_height_and_gap() {
        for t in [today(), Tokens { row_h: 24.0, row_gap: 6.0, ..today() }] {
            frame(&t, 300.0, &mut |ui| {
                let g = PropGrid::new(ui, &t, 4);
                assert_eq!(g.cell(3, Track::FieldA).min.y - g.cell(0, Track::FieldA).min.y, 3.0 * (t.row_h + t.row_gap));
                assert_eq!(g.cell(3, Track::FieldA).height(), t.row_h);
            });
        }
    }

    /// The grid reads the tokens every frame, so switching the theme or its density leaves no
    /// stale geometry behind.
    #[test]
    fn grid_follows_the_tokens() {
        let ctx = egui::Context::default();
        let mut steps = vec![];
        for row_h in [21.0, 24.0] {
            crate::theme::apply(&ctx, &Tokens { row_h, ..today() });
            let mut out = ctx.run_ui(raw(300.0), |ui| {
                let t = Tokens::get(ui.ctx());
                let g = PropGrid::new(ui, &t, 2);
                steps.push(g.cell(1, Track::Lead).min.y - g.cell(0, Track::Lead).min.y);
            });
            out.textures_delta.clear();
        }
        assert_eq!(steps, [25.0, 28.0]);
    }

    /// The measurements behind the pair rule (spec §2), in the bundled UI font.
    #[test]
    fn values_fit_where_measured() {
        frame(&today(), 400.0, &mut |ui| {
            assert!(value_fits(ui, NUM_CHROME, "10.5 pt", 77.5));
            assert!(!value_fits(ui, NUM_CHROME, "10.5 pt", 64.0));
            assert!(value_fits(ui, DROPDOWN_CHROME, "Metrics", 64.0));
            assert!(value_fits(ui, NUM_CHROME, "(14.4 pt)", 84.0));
            assert!(!value_fits(ui, NUM_CHROME, "(14.4 pt)", 75.0));
        });
    }
}
