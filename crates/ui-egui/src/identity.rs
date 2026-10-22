//! The app's own name and marks. A build with an edition (`designcraft_edition`) names itself after
//! it and leaves out the ArtCraft marks (the mark, Discord, the ArtCraft website), as the ArtCraft
//! brand licence asks of modified versions (`docs/brand/LICENSE-brand.txt`); it credits DesignCraft
//! in plain text instead.

use designcraft_edition::Edition;
use egui::{Color32, Rect, Sense, pos2, vec2};

/// The plain-text credit an edition gives in place of the ArtCraft marks, as the brand licence
/// allows: DesignCraft's name, its makers and its code licence.
pub const BASED_ON: &str = "Based on DesignCraft by the ArtCraft team. MIT or Apache 2.0.";

/// The labels that name the app (English sources): only these take an edition's name.
const NAMING: [&str; 4] = ["About DesignCraft", "Quit DesignCraft", "Welcome to DesignCraft", "DesignCraft"];

/// The ArtCraft community and website commands. An edition leaves them out of its menus, search
/// and shortcut list; they still run when called by id.
const ARTCRAFT_COMMANDS: [&str; 4] = ["help.discord", "help.website", "help.app", "help.appPage"];

/// The app's own name: the edition's, else "DesignCraft".
pub fn app_name(edition: Option<&Edition>) -> &'static str {
    edition.map_or("DesignCraft", |e| e.name)
}

/// `label` (English, or as `tr()` translated it) with the edition's name for "DesignCraft", when
/// it is one of the labels that name the app. Any other label, or no edition, comes back as it is.
pub fn named(label: &str, edition: Option<&Edition>) -> String {
    let names_app = || NAMING.iter().any(|src| crate::i18n::LANGUAGES.iter().any(|(lang, _)| crate::i18n::tr(lang, src) == label));
    match edition {
        Some(e) if label.contains("DesignCraft") && names_app() => label.replace("DesignCraft", e.name),
        _ => label.to_string(),
    }
}

/// Whether an edition leaves `command` out of its menus, search and shortcut list.
pub fn hidden(command: &str, edition: Option<&Edition>) -> bool {
    edition.is_some() && ARTCRAFT_COMMANDS.contains(&command)
}

/// Paint the edition's mark (SVG) into `rect`, centred and as large as fits, rendered once per
/// size and cached. False, with nothing drawn, when the SVG doesn't render.
pub fn paint_mark(ui: &egui::Ui, rect: Rect, svg: &[u8]) -> bool {
    let ctx = ui.ctx();
    let side = rect.width().min(rect.height());
    let px = ((side * ctx.pixels_per_point()).round() as u32).clamp(8, 1024);
    let key = egui::Id::new(("edition_mark", px, svg));
    let cached: Option<Option<egui::TextureHandle>> = ctx.data(|d| d.get_temp(key));
    let tex = cached.unwrap_or_else(|| {
        let image = designcraft_images::render_svg(svg, px).and_then(|(rgba, w, h)| {
            let size = [usize::try_from(w).ok()?, usize::try_from(h).ok()?];
            // Four bytes a pixel, or `from_rgba_premultiplied` would refuse it.
            (size[0].checked_mul(size[1])?.checked_mul(4)? == rgba.len()).then(|| egui::ColorImage::from_rgba_premultiplied(size, &rgba))
        });
        let tex = image.map(|image| crate::widgets::load_texture(ctx, "edition_mark", image, egui::TextureOptions::LINEAR));
        // A mark that won't render is remembered too, so it isn't tried again every frame.
        ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
        tex
    });
    let Some(tex) = tex else { return false };
    let size = tex.size_vec2();
    let r = Rect::from_center_size(rect.center(), size * (side / size.x.max(size.y).max(1.0)));
    ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    true
}

/// The edition's lockup, built live as its brand sets it: the mark at 0.501 of the width, then the
/// wordmark in the edition's Bold at 0.144 of it and 0.049 of it below, centred, at least 115 px
/// wide. Without the edition's typeface the mark stands alone: its wordmark is never set in
/// another face.
pub fn paint_lockup(ui: &mut egui::Ui, edition: &Edition, width: f32) {
    let width = width.max(115.0);
    let side = width * 0.501;
    let (mark, _) = ui.allocate_exact_size(vec2(width, side), Sense::hover());
    paint_mark(ui, Rect::from_center_size(mark.center(), vec2(side, side)), edition.mark_svg);
    if edition.fonts.is_none() {
        return;
    }
    ui.add_space(width * 0.049);
    let font = egui::FontId::new(width * 0.144, egui::FontFamily::Name("bold".into()));
    let galley = ui.painter().layout_no_wrap(edition.wordmark.to_string(), font, crate::theme::Tokens::get(ui.ctx()).text_strong);
    let (row, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
    ui.painter().galley(pos2(row.center().x - galley.size().x / 2.0, row.min.y), galley, Color32::PLACEHOLDER);
}

/// Task 1's fixture edition (`crates/edition/tests/fixture/`), without a typeface.
#[cfg(test)]
pub(crate) const FIXTURE_EDITION: designcraft_edition::Edition = designcraft_edition::Edition {
    name: "Test Edition",
    bundle_id: "org.example.test-edition",
    data_folder: "Test Edition",
    wordmark: "TEST",
    mark_svg: include_bytes!("../../edition/tests/fixture/mark.svg"),
    theme: designcraft_edition::Theme {
        bg: [0x10, 0x14, 0x18],
        raised: [0x20, 0x26, 0x2c],
        hover: [0x2a, 0x31, 0x38],
        text: [0xe8, 0xec, 0xf0],
        accent: [0x3a, 0xa6, 0x75],
        on_accent: [0x00, 0x00, 0x00],
        success: [0x4c, 0xc3, 0x8a],
        error: [0xe5, 0x48, 0x4d],
        secondary: 0.72,
        muted: 0.52,
        faint: 0.32,
        hairline: 0.18,
        radius: 0,
    },
    fonts: None,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::Item;

    const E: Option<&designcraft_edition::Edition> = Some(&FIXTURE_EDITION);

    /// Only the labels that name the app take the edition's name, in English or translated.
    #[test]
    fn only_labels_that_name_the_app_change() {
        assert_eq!(named("Quit DesignCraft", E), "Quit Test Edition");
        assert_eq!(named("About DesignCraft", E), "About Test Edition");
        assert_eq!(named("Welcome to DesignCraft", E), "Welcome to Test Edition");
        assert_eq!(named("DesignCraft", E), "Test Edition");
        // Translations of them, after `tr()`.
        assert_eq!(named(crate::i18n::tr("de", "About DesignCraft"), E), "Über Test Edition");
        assert_eq!(named("Über DesignCraft", E), "Über Test Edition");
        assert_eq!(named("DesignCraft beenden", E), "Test Edition beenden");
        // Other labels keep DesignCraft's name.
        assert_eq!(named("DesignCraft on GitHub…", E), "DesignCraft on GitHub…");
        assert_eq!(named("Open DesignCraft documents", E), "Open DesignCraft documents");
        for label in ["Quit DesignCraft", "Über DesignCraft", "DesignCraft on GitHub…", "DesignCraft"] {
            assert_eq!(named(label, None), label);
        }
        assert_eq!((app_name(E), app_name(None)), ("Test Edition", "DesignCraft"));
    }

    /// The ArtCraft community and website commands leave an edition's menus.
    #[test]
    fn artcraft_help_commands_are_hidden_in_an_edition() {
        for id in ["help.discord", "help.website", "help.app", "help.appPage"] {
            assert!(hidden(id, E), "{id}");
            assert!(!hidden(id, None), "{id}");
        }
        for id in ["help.github", "help.issues", "help.about", "app.quit"] {
            assert!(!hidden(id, E), "{id}");
        }
        let help = |edition| {
            let tree = crate::menus::menu_tree_for(edition);
            tree.into_iter().find(|(m, _)| *m == "Help").map(|(_, entries)| entries).unwrap_or_default()
        };
        let ids = |entries: &[Item]| -> Vec<String> {
            entries.iter().filter_map(|e| if let Item::Cmd { id, .. } = e { Some(id.clone()) } else { None }).collect()
        };
        let (with, plain) = (help(E), help(None));
        for id in ["help.discord", "help.website", "help.appPage"] {
            assert!(!ids(&with).iter().any(|i| i == id), "{id} in an edition's Help");
            assert!(ids(&plain).iter().any(|i| i == id), "{id} missing from the plain Help");
        }
        assert!(ids(&with).iter().any(|i| i == "help.github") && ids(&with).iter().any(|i| i == "help.about"));
        // The separators that framed them go too.
        assert!(!matches!(with.first(), Some(Item::Sep)) && !matches!(with.last(), Some(Item::Sep)));
        assert!(!with.windows(2).any(|w| matches!(w, [Item::Sep, Item::Sep])));
    }

    /// A mark that won't render (a broken SVG in the edition folder) draws nothing and doesn't
    /// stop the frame, now or from the cache; the fixture's mark draws.
    #[test]
    fn a_mark_that_will_not_render_draws_nothing() {
        let ctx = egui::Context::default();
        let mut drawn = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(18.0, 18.0));
                drawn.push((paint_mark(ui, rect, b"not svg"), paint_mark(ui, rect, FIXTURE_EDITION.mark_svg)));
            });
            output.textures_delta.clear();
        }
        assert_eq!(drawn, [(false, true), (false, true)]);
    }
}
