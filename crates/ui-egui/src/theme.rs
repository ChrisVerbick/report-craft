//! Design tokens (InDesign's four interface brightness levels), fonts and egui style.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Brightness {
    Dark,
    #[default]
    MediumDark,
    MediumLight,
    Light,
    /// Accessibility: black chrome, white text, yellow selection and focus.
    HighContrast,
    /// The compiled-in edition's own colours (`designcraft_edition`). Medium Dark in a build
    /// without one, so a setting an edition saved still reads.
    Edition,
}

impl Brightness {
    pub const ALL: [Brightness; 5] = [Brightness::Dark, Brightness::MediumDark, Brightness::MediumLight, Brightness::Light, Brightness::HighContrast];
    pub fn label(self) -> &'static str {
        match self {
            Brightness::Dark => "Dark",
            Brightness::MediumDark => "Medium Dark",
            Brightness::MediumLight => "Medium Light",
            Brightness::Light => "Light",
            Brightness::HighContrast => "High Contrast",
            Brightness::Edition => designcraft_edition::current().map_or("Edition", |e| e.name),
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Brightness::Dark => "dark",
            Brightness::MediumDark => "mediumDark",
            Brightness::MediumLight => "mediumLight",
            Brightness::Light => "light",
            Brightness::HighContrast => "highContrast",
            Brightness::Edition => "edition",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().chain([Brightness::Edition]).find(|b| b.id().eq_ignore_ascii_case(s) || b.label().eq_ignore_ascii_case(s))
    }
}

/// The levels the Interface Color Theme menu offers: the edition's first when the build has one.
pub fn available(edition: Option<&designcraft_edition::Edition>) -> Vec<Brightness> {
    edition.map(|_| Brightness::Edition).into_iter().chain(Brightness::ALL).collect()
}

/// Every colour the UI uses. Widgets never hard-code colours.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub dark: bool,
    pub app_bar: Color32,
    pub panel: Color32,
    pub panel_darker: Color32,
    pub input: Color32,
    pub input_border: Color32,
    pub divider: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_dim: Color32,
    pub text_disabled: Color32,
    pub icon: Color32,
    pub hover: Color32,
    pub tool_active: Color32,
    pub accent: Color32,
    pub accent_strong: Color32,
    pub row_selected: Color32,
    pub pasteboard: Color32,
    pub ruler: Color32,
    pub ruler_tick: Color32,
    pub tab_inactive: Color32,
    pub button: Color32,
    pub radius: u8,
    /// 1 pt chrome edges.
    pub border: Color32,
    /// Outline of fields and buttons.
    pub field_border: Color32,
    /// Active tool / toggle well.
    pub well: Color32,
    pub well_rim: Color32,
    pub ruler_text: Color32,
    pub section_divider: Color32,
    /// Panel tab strips, doc-tab bar, dock headers.
    pub tab_strip: Color32,
    /// Smart guides and spacing marks drawn over the page.
    pub smart_guide: Color32,
    /// Measurement labels on the canvas (smart dimensions, gaps).
    pub measure_bg: Color32,
    pub measure_text: Color32,
    /// Flow: who changed what. The file's changes, the layout's edits it
    /// keeps (the note-anchor amber: a conflict's losing layout text goes into a note), errors the
    /// file shows (`flow_error_fill` behind a white "!"), and the 0% check's tick.
    pub flow_file: Color32,
    pub flow_yours: Color32,
    pub flow_error: Color32,
    pub flow_error_fill: Color32,
    pub flow_ok: Color32,
    /// Live-corner widget and corner diamonds.
    pub live_corner: Color32,
    /// The Windows caption Close button when hovered, and its glyph.
    pub caption_close: Color32,
    pub caption_close_text: Color32,
    /// Text and glyphs on accent fills (primary buttons, a hovered swatch row).
    pub on_accent: Color32,
    /// The Pages panel: the selected page's number badge (and the drop target's outline), the tint
    /// multiplied over the selected thumbnail, the number on the badge, and the other pages' numbers.
    pub selection_badge: Color32,
    pub selection_tint: Color32,
    pub badge_text: Color32,
    pub page_number: Color32,
    /// The Preflight dots: no errors, errors.
    pub status_ok: Color32,
    pub status_error: Color32,
    /// Links statuses: modified, too large to check or low resolution; missing or failed.
    pub link_warn: Color32,
    pub link_error: Color32,
    /// A number field while it is typed in.
    pub focus_bg: Color32,
    pub focus_text: Color32,
    pub focus_border: Color32,
    /// Corner radius of windows and menus.
    pub frame_radius: u8,
    /// The fill behind selected tabs, labels, buttons and text, under `text`.
    pub selection_fill: Color32,
}

fn hex(s: u32) -> Color32 {
    Color32::from_rgb((s >> 16) as u8, (s >> 8) as u8, s as u8)
}

/// `c` at alpha `a`, flattened over `bg`.
fn over(c: [u8; 3], a: f32, bg: [u8; 3]) -> Color32 {
    let mix = |c: u8, bg: u8| (f32::from(c) * a + f32::from(bg) * (1.0 - a)).round() as u8;
    Color32::from_rgb(mix(c[0], bg[0]), mix(c[1], bg[1]), mix(c[2], bg[2]))
}

impl Tokens {
    pub fn for_brightness(b: Brightness) -> Self {
        let dark = Tokens {
            dark: true,
            app_bar: hex(0x262626),
            panel: hex(0x323232),
            panel_darker: hex(0x282828),
            input: hex(0x1f1f1f),
            input_border: hex(0x4a4a4a),
            divider: hex(0x1e1e1e),
            text: hex(0xd8d8d8),
            text_strong: hex(0xf2f2f2),
            text_dim: hex(0xa3a3a3),
            text_disabled: hex(0x6a6a6a),
            icon: hex(0xc8c8c8),
            hover: hex(0x424242),
            tool_active: hex(0x1b1b1b),
            accent: hex(0x378ef0),
            accent_strong: hex(0x1473e6),
            row_selected: hex(0x2d4f7c),
            pasteboard: hex(0x232323),
            ruler: hex(0x2e2e2e),
            ruler_tick: hex(0x9a9a9a),
            tab_inactive: hex(0x262626),
            button: hex(0x404040),
            radius: 1,
            border: hex(0x1e1e1e),
            field_border: hex(0x5a5a5a),
            well: hex(0x1b1b1b),
            well_rim: hex(0x3a3a3a),
            ruler_text: hex(0xd8d8d8),
            section_divider: hex(0x282828),
            tab_strip: hex(0x282828),
            smart_guide: hex(0x00c853),
            measure_bg: Color32::from_rgba_unmultiplied(70, 70, 70, 230),
            measure_text: Color32::WHITE,
            flow_file: hex(0x6cb6ff),
            flow_yours: hex(0xf2b544),
            flow_error: hex(0xff8a80),
            flow_error_fill: hex(0xe5484d),
            flow_ok: hex(0x3fb950),
            live_corner: hex(0xffe500),
            caption_close: hex(0xc42b1c),
            caption_close_text: Color32::WHITE,
            on_accent: Color32::WHITE,
            selection_badge: hex(0x2f73e0),
            selection_tint: hex(0x88c7fb),
            badge_text: Color32::WHITE,
            page_number: Color32::WHITE,
            status_ok: Color32::from_rgb(60, 200, 90),
            status_error: Color32::from_rgb(235, 50, 50),
            link_warn: hex(0xe8b32c),
            link_error: hex(0xe54b4b),
            focus_bg: Color32::WHITE,
            focus_text: Color32::from_gray(20),
            focus_border: hex(0x45a0f5),
            frame_radius: 4,
            // Each level's accent_strong (below).
            selection_fill: hex(0x1473e6),
        };
        let level = match b {
            Brightness::Dark => dark,
            // Measured from InDesign 2026 (plan/indesign/11-observed-ui.md §1). An edition's level
            // needs the edition (`resolve`); without one it is Medium Dark.
            Brightness::MediumDark | Brightness::Edition => Tokens {
                app_bar: hex(0x535353),
                panel: hex(0x535353),
                panel_darker: hex(0x424242),
                input: hex(0x454545),
                input_border: hex(0x747474),
                divider: hex(0x4b4b4b),
                text: hex(0xffffff),
                text_strong: hex(0xffffff),
                text_dim: hex(0xb0b0b0),
                text_disabled: hex(0x8a8a8a),
                icon: hex(0xc2c2c2),
                hover: hex(0x606060),
                tool_active: hex(0x303030),
                row_selected: hex(0x4a6a92),
                pasteboard: hex(0x5e5e5e),
                ruler: hex(0x1f1f1f),
                ruler_tick: hex(0x858585),
                tab_inactive: hex(0x424242),
                button: hex(0x535353),
                border: hex(0x383838),
                field_border: hex(0x747474),
                well: hex(0x303030),
                well_rim: hex(0x565656),
                ruler_text: hex(0xffffff),
                section_divider: hex(0x4b4b4b),
                tab_strip: hex(0x424242),
                ..dark
            },
            Brightness::MediumLight => Tokens {
                dark: false,
                app_bar: hex(0xa8a8a8),
                panel: hex(0xb8b8b8),
                panel_darker: hex(0xaaaaaa),
                input: hex(0xd6d6d6),
                input_border: hex(0x8c8c8c),
                divider: hex(0x9c9c9c),
                text: hex(0x1e1e1e),
                text_strong: hex(0x000000),
                text_dim: hex(0x3c3c3c),
                text_disabled: hex(0x7a7a7a),
                icon: hex(0x2a2a2a),
                hover: hex(0xc8c8c8),
                tool_active: hex(0x9a9a9a),
                row_selected: hex(0x8eb2e0),
                pasteboard: hex(0xa0a0a0),
                ruler: hex(0xc0c0c0),
                ruler_tick: hex(0x3a3a3a),
                tab_inactive: hex(0xa6a6a6),
                button: hex(0xb8b8b8),
                border: hex(0x8c8c8c),
                field_border: hex(0x7a7a7a),
                well: hex(0x9a9a9a),
                well_rim: hex(0x8a8a8a),
                ruler_text: hex(0x1e1e1e),
                section_divider: hex(0x9c9c9c),
                tab_strip: hex(0xa6a6a6),
                flow_file: hex(0x0b4fa8),
                flow_yours: hex(0x6e4100),
                flow_error: hex(0x9c1c14),
                flow_error_fill: hex(0xb3261e),
                flow_ok: hex(0x145c2a),
                ..dark
            },
            Brightness::Light => Tokens {
                dark: false,
                app_bar: hex(0xe4e4e4),
                panel: hex(0xf0f0f0),
                panel_darker: hex(0xe2e2e2),
                input: hex(0xffffff),
                input_border: hex(0xb4b4b4),
                divider: hex(0xd0d0d0),
                text: hex(0x1e1e1e),
                text_strong: hex(0x000000),
                text_dim: hex(0x555555),
                text_disabled: hex(0x9a9a9a),
                icon: hex(0x2c2c2c),
                hover: hex(0xdddddd),
                tool_active: hex(0xcfcfcf),
                row_selected: hex(0xc4dbf7),
                pasteboard: hex(0xd4d4d4),
                ruler: hex(0xf2f2f2),
                ruler_tick: hex(0x4a4a4a),
                tab_inactive: hex(0xdcdcdc),
                button: hex(0xf0f0f0),
                border: hex(0xc4c4c4),
                field_border: hex(0xa8a8a8),
                well: hex(0xcfcfcf),
                well_rim: hex(0xbdbdbd),
                ruler_text: hex(0x1e1e1e),
                section_divider: hex(0xd0d0d0),
                tab_strip: hex(0xe2e2e2),
                flow_file: hex(0x1f6feb),
                flow_yours: hex(0x9a5b00),
                flow_error: hex(0xc62828),
                flow_error_fill: hex(0xd93025),
                flow_ok: hex(0x1a7f37),
                ..dark
            },
            Brightness::HighContrast => Tokens {
                app_bar: hex(0x000000),
                panel: hex(0x000000),
                panel_darker: hex(0x000000),
                input: hex(0x000000),
                input_border: hex(0xffffff),
                divider: hex(0xffffff),
                text: hex(0xffffff),
                text_strong: hex(0xffffff),
                text_dim: hex(0xe6e6e6),
                text_disabled: hex(0xa0a0a0),
                icon: hex(0xffffff),
                hover: hex(0x3d3d00),
                tool_active: hex(0x5c5c00),
                accent: hex(0xffeb3b),
                accent_strong: hex(0xffd600),
                row_selected: hex(0x5c5c00),
                pasteboard: hex(0x1a1a1a),
                ruler: hex(0x000000),
                ruler_tick: hex(0xffffff),
                tab_inactive: hex(0x000000),
                button: hex(0x000000),
                border: hex(0xffffff),
                field_border: hex(0xffffff),
                well: hex(0x5c5c00),
                well_rim: hex(0xffeb3b),
                ruler_text: hex(0xffffff),
                section_divider: hex(0xffffff),
                tab_strip: hex(0x000000),
                flow_file: hex(0x59c3ff),
                flow_yours: hex(0xffc046),
                flow_error: hex(0xff7070),
                flow_error_fill: hex(0xff4d4d),
                flow_ok: hex(0x4be06e),
                ..dark
            },
        };
        Tokens { selection_fill: level.accent_strong, ..level }
    }

    /// An edition's theme by role (`docs/editions.md`): its background, raised and hover fills,
    /// its text at its alphas, its accent and its success and error colours. Canvas overlays and
    /// fixed signals keep Dark's.
    pub fn from_edition(t: &designcraft_edition::Theme) -> Tokens {
        let dark = Tokens::for_brightness(Brightness::Dark);
        let rgb = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
        let text = |a: f32| over(t.text, a, t.bg);
        let (bg, raised) = (rgb(t.bg), rgb(t.raised));
        Tokens {
            dark: true,
            app_bar: bg,
            panel: bg,
            panel_darker: bg,
            tab_strip: bg,
            tab_inactive: bg,
            ruler: bg,
            input: raised,
            button: raised,
            well: raised,
            tool_active: raised,
            pasteboard: raised,
            hover: rgb(t.hover),
            divider: text(t.hairline),
            border: text(t.hairline),
            section_divider: text(t.hairline),
            well_rim: text(t.hairline),
            input_border: text(t.faint),
            field_border: text(t.faint),
            text_disabled: text(t.faint),
            text: rgb(t.text),
            text_strong: rgb(t.text),
            text_dim: text(t.secondary),
            icon: text(t.secondary),
            ruler_text: text(t.secondary),
            ruler_tick: text(t.muted),
            accent: rgb(t.accent),
            accent_strong: rgb(t.accent),
            flow_file: rgb(t.accent),
            row_selected: over(t.accent, 0.16, t.bg),
            flow_ok: rgb(t.success),
            flow_error: rgb(t.error),
            radius: t.radius,
            on_accent: rgb(t.on_accent),
            selection_badge: rgb(t.accent),
            selection_tint: rgb(t.accent),
            badge_text: rgb(t.on_accent),
            page_number: rgb(t.text),
            status_ok: rgb(t.success),
            status_error: rgb(t.error),
            // Flow's amber: a link to update, like the layout's kept edits.
            link_warn: dark.flow_yours,
            link_error: rgb(t.error),
            // A field being typed in stays dark, ringed in the accent.
            focus_bg: raised,
            focus_text: rgb(t.text),
            focus_border: rgb(t.accent),
            frame_radius: t.radius,
            // egui sets selected widgets' text in `text`: the accent, darkened, keeps it readable.
            selection_fill: over(t.accent, 0.65, t.bg),
            ..dark
        }
    }

    /// The tokens for `b`: the edition's own for [`Brightness::Edition`], Medium Dark when the
    /// build has no edition (a setting an edition saved, read by a plain build).
    pub fn resolve(b: Brightness, edition: Option<&designcraft_edition::Edition>) -> Tokens {
        match (b, edition) {
            (Brightness::Edition, Some(e)) => Tokens::from_edition(&e.theme),
            _ => Tokens::for_brightness(b),
        }
    }
}

/// Install the UI fonts for the interface language `lang` (it orders the CJK fallbacks).
/// Returns false while the installed fonts are still being cataloged: the built-in faces are
/// installed without waiting, and the caller installs again once they are (see
/// [`system_fonts_ready`]) to add the installed faces for the scripts they lack.
pub fn install_fonts(ctx: &egui::Context, lang: &str) -> bool {
    install_fonts_for(ctx, lang, None)
}

/// [`install_fonts`] with the edition's typeface, when it has one, in front of the UI faces.
pub fn install_fonts_for(ctx: &egui::Context, lang: &str, edition: Option<&designcraft_edition::Edition>) -> bool {
    let craft = designcraft_fonts::CRAFT_FONTS;
    let mut defs = font_definitions(craft, lang);
    if let Some(fonts) = edition.and_then(|e| e.fonts.as_ref()) {
        add_edition_fonts(&mut defs, fonts);
    }
    let system = system_ui_fonts(craft);
    if let Some(system) = system {
        add_system_fallbacks(&mut defs, system, lang);
    }
    ctx.set_fonts(defs);
    system.is_some()
}

/// Whether [`install_fonts`] would find the installed fonts cataloged.
pub fn system_fonts_ready() -> bool {
    designcraft_fonts::FontDb::global().system_fonts_cataloged()
}

/// An installed font the UI falls back to for a script the built-in faces lack.
pub(crate) struct SystemFont {
    /// The ISO 15924 code of the script it is for ("Jpan", "Kore", "Taml"…).
    script: &'static str,
    family: String,
    /// The face as egui takes it, on the UI face's baseline. Shared: installing the fonts again
    /// (another interface language) doesn't copy the file.
    data: Arc<FontData>,
}

impl SystemFont {
    pub(crate) fn new(script: &'static str, family: String, bytes: Vec<u8>, index: u32) -> Self {
        let tweak = baseline_tweak(&bytes, index);
        SystemFont { script, family, data: Arc::new(FontData { index, tweak, ..FontData::from_owned(bytes) }) }
    }
}

/// Installed UI faces (sans serif) per script, with a character every face of the script has:
/// the first listed family that is installed and covers the character, macOS families first,
/// then Windows, then Linux. One family can serve several scripts (Nirmala UI covers the Indic
/// scripts); it is added once.
const SYSTEM_UI_FONTS: &[(&str, char, &[&str])] = &[
    (
        "Jpan",
        'あ',
        &[
            "Hiragino Sans",
            "Hiragino Kaku Gothic ProN",
            "Yu Gothic UI",
            "Yu Gothic",
            "Meiryo",
            "MS Gothic",
            "Noto Sans CJK JP",
            "Noto Sans JP",
            "Source Han Sans JP",
            "IPAexGothic",
            "Droid Sans Fallback",
        ],
    ),
    ("Hans", '中', &["PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Noto Sans CJK SC", "Source Han Sans SC", "WenQuanYi Micro Hei"]),
    // Traditional Chinese and Korean: font menus list those families under their native names.
    ("Hant", '中', &["PingFang TC", "Microsoft JhengHei", "Noto Sans CJK TC", "Source Han Sans TC"]),
    ("Kore", '한', &["Apple SD Gothic Neo", "Malgun Gothic", "Noto Sans CJK KR", "Noto Sans KR", "Source Han Sans K", "NanumGothic", "UnDotum"]),
    // Scripts the built-in faces lack, after the CJK ones (text in the Story Editor, font names).
    ("Taml", 'த', &["Tamil Sangam MN", "Tamil MN", "Nirmala UI", "Latha", "Noto Sans Tamil", "Noto Sans Tamil UI", "Lohit Tamil"]),
    ("Deva", 'क', &["Kohinoor Devanagari", "Devanagari Sangam MN", "Nirmala UI", "Mangal", "Noto Sans Devanagari", "Lohit Devanagari"]),
    ("Beng", 'ক', &["Kohinoor Bangla", "Bangla Sangam MN", "Nirmala UI", "Vrinda", "Noto Sans Bengali", "Lohit Bengali"]),
    ("Gujr", 'ક', &["Kohinoor Gujarati", "Gujarati Sangam MN", "Nirmala UI", "Shruti", "Noto Sans Gujarati", "Lohit Gujarati"]),
    ("Guru", 'ਕ', &["Gurmukhi Sangam MN", "Gurmukhi MN", "Nirmala UI", "Raavi", "Noto Sans Gurmukhi", "Lohit Gurmukhi"]),
    ("Knda", 'ಕ', &["Kannada Sangam MN", "Nirmala UI", "Tunga", "Noto Sans Kannada", "Lohit Kannada"]),
    ("Mlym", 'ക', &["Malayalam Sangam MN", "Nirmala UI", "Kartika", "Noto Sans Malayalam", "Lohit Malayalam"]),
    ("Telu", 'క', &["Kohinoor Telugu", "Telugu Sangam MN", "Nirmala UI", "Gautami", "Noto Sans Telugu", "Lohit Telugu"]),
    ("Orya", 'କ', &["Oriya Sangam MN", "Nirmala UI", "Kalinga", "Noto Sans Oriya", "Lohit Odia"]),
    ("Sinh", 'ක', &["Sinhala Sangam MN", "Nirmala UI", "Iskoola Pota", "Noto Sans Sinhala", "LKLUG"]),
    ("Thai", 'ก', &["Thonburi", "Leelawadee UI", "Tahoma", "Noto Sans Thai", "Loma"]),
    ("Khmr", 'ក', &["Khmer Sangam MN", "Leelawadee UI", "Khmer UI", "Noto Sans Khmer", "Khmer OS"]),
    ("Laoo", 'ກ', &["Lao Sangam MN", "Leelawadee UI", "Lao UI", "Noto Sans Lao", "Phetsarath OT"]),
    ("Mymr", 'က', &["Myanmar Sangam MN", "Myanmar Text", "Noto Sans Myanmar", "Padauk"]),
    ("Hebr", 'א', &["Arial Hebrew", "Segoe UI", "Noto Sans Hebrew", "DejaVu Sans"]),
    ("Armn", 'Ա', &["Mshtakan", "Segoe UI", "Sylfaen", "Noto Sans Armenian", "DejaVu Sans"]),
    ("Geor", 'ა', &["Segoe UI", "Sylfaen", "Noto Sans Georgian", "DejaVu Sans"]),
    ("Ethi", 'አ', &["Kefa", "Ebrima", "Nyala", "Noto Sans Ethiopic", "Abyssinica SIL"]),
];

/// The installed faces for the scripts `craft` (the built-in craft-fonts) doesn't cover, so the
/// interface shows Japanese and Chinese without the craft-fonts build input, and Korean,
/// Traditional Chinese (font names in the font menus), Indic, Southeast Asian and other scripts
/// in any build. egui can't find system fonts itself; the document font scan (not on the web)
/// does.
fn system_ui_fonts(craft: &[designcraft_fonts::CraftFont]) -> Option<&'static [SystemFont]> {
    static FACES: std::sync::OnceLock<Vec<SystemFont>> = std::sync::OnceLock::new();
    let db = designcraft_fonts::FontDb::global();
    // Never wait for the font scan here (the first frame): it runs in the background, and the
    // faces are read once, when it is done.
    // (Tests wait instead, so that the fonts don't change in the middle of one.)
    if !db.system_fonts_cataloged() && !cfg!(test) {
        db.scan_in_background();
        return None;
    }
    Some(FACES.get_or_init(|| installed_ui_fonts(db, craft)))
}

/// The faces of [`SYSTEM_UI_FONTS`] that `db` has, for the scripts `craft` doesn't cover.
fn installed_ui_fonts(db: &designcraft_fonts::FontDb, craft: &[designcraft_fonts::CraftFont]) -> Vec<SystemFont> {
    let mut faces: Vec<SystemFont> = Vec::new();
    for (script, sample, families) in SYSTEM_UI_FONTS {
        if craft.iter().any(|f| f.scripts.contains(script)) {
            continue;
        }
        let found = families.iter().find_map(|family| {
            if !db.has_family(family) {
                return None;
            }
            let face = db.face(family, "Regular");
            // `face` falls back to another family when this one can't be read: keep only the real one.
            (face.family.eq_ignore_ascii_case(family) && !face.data().is_empty() && face.covers(*sample)).then_some((family, face))
        });
        let Some((family, face)) = found else { continue };
        if !faces.iter().any(|f| f.family.eq_ignore_ascii_case(family)) {
            faces.push(SystemFont::new(script, (*family).to_string(), face.data().to_vec(), face.index()));
        }
    }
    faces
}

/// The UI face every family starts with (Source Sans 3; the semibold shares its line metrics).
const UI_FONT: &[u8] = include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf");

/// The tweak that puts a fallback face's baseline on the UI face's. egui centres a fallback's
/// line height in the primary's instead (`pos.y = ascent + (line height − its line height) / 2`),
/// so a CJK face with a taller ascent or a larger line gap would sit above or below the Latin
/// text beside it. Shifts are in ems of the font size.
pub(crate) fn baseline_tweak(face: &[u8], index: u32) -> egui::FontTweak {
    let metrics = |data: &[u8], i: u32| {
        designcraft_fonts::line_metrics(data, i).map(|(upem, ascent, descent, gap)| (ascent / upem, (ascent - descent + gap) / upem))
    };
    let shift = match (metrics(UI_FONT, 0), metrics(face, index)) {
        (Some((ap, hp)), Some((af, hf))) => ap - (af + 0.5 * (hp - hf)),
        _ => 0.0,
    };
    egui::FontTweak { y_offset_factor: if shift.is_finite() { shift.clamp(-0.5, 0.5) } else { 0.0 }, ..Default::default() }
}

/// The script whose letterforms the interface language `lang` prefers for Han characters, Kana
/// and Hangul: "Jpan", "Hans", "Hant" or "Kore". Languages without a CJK preference use Japanese
/// forms, the app's default CJK face.
fn cjk_script(lang: &str) -> &'static str {
    match lang {
        "zh" | "zh-hans" | "zh-cn" => "Hans",
        "zh-hant" | "zh-tw" | "zh-hk" => "Hant",
        "ko" => "Kore",
        _ => "Jpan",
    }
}

/// Add installed faces `system` to every family in the interface language's order: the faces of
/// its script (`cjk_script`) go before the built-in CJK faces, so Han characters take that
/// language's forms (Traditional Chinese, Korean); the other CJK faces go at the end, then the
/// other scripts' faces.
pub(crate) fn add_system_fallbacks(fonts: &mut FontDefinitions, system: &[SystemFont], lang: &str) {
    let name = |f: &SystemFont| format!("system-{}", f.family);
    for f in system {
        fonts.font_data.insert(name(f), f.data.clone());
    }
    let first = cjk_script(lang);
    let preferred: Vec<String> = system.iter().filter(|f| f.script == first).map(name).collect();
    const CJK: [&str; 4] = ["Jpan", "Hans", "Hant", "Kore"];
    let rest: Vec<String> = CJK
        .iter()
        .filter(|s| **s != first)
        .flat_map(|s| system.iter().filter(move |f| f.script == *s))
        .chain(system.iter().filter(|f| !CJK.contains(&f.script)))
        .map(name)
        .collect();
    for (family, stack) in fonts.families.iter_mut() {
        // The Arabic families keep their own order (see `font_definitions`).
        let arabic = matches!(family, FontFamily::Name(n) if n.starts_with("arabic"));
        let at = if arabic { stack.len() } else { stack.iter().position(|n| n.starts_with("craft-")).unwrap_or(stack.len()) };
        stack.splice(at..at, preferred.iter().cloned());
        stack.extend(rest.iter().cloned());
    }
}

/// The UI fonts: the app's own, then the craft-fonts faces from `craft` (empty without
/// `CRAFT_FONTS_DIR`) as fallbacks at the end of every family: Japanese (BIZ UDPGothic first) and
/// Simplified Chinese, in the interface language's order, then Arabic. The `arabic` families
/// put the Arabic face first for right-to-left runs. egui has no system-font discovery, so
/// without craft-fonts CJK and Arabic UI text has no glyphs.
pub(crate) fn font_definitions(craft: &'static [designcraft_fonts::CraftFont], lang: &str) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, data: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(data)));
    };
    add(&mut fonts, "ui", include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf"));
    add(&mut fonts, "ui-semibold", include_bytes!("../../../assets/fonts/SourceSans3-Semibold.ttf"));
    add(&mut fonts, "mono", include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "mono".into());
    fonts.families.insert(FontFamily::Name("semibold".into()), vec!["ui-semibold".into(), "ui".into()]);
    // The wordmark's weight: an edition's Bold when it has one (`add_edition_fonts`).
    fonts.families.insert(FontFamily::Name("bold".into()), vec!["ui-semibold".into(), "ui".into()]);
    // Flow quotes a report's own words in the report's face: the bundled Source Serif 4.
    let serif = match designcraft_fonts::bundled().get(SERIF) {
        Some(bytes) => {
            add(&mut fonts, "serif", bytes);
            vec!["serif".into(), "ui".into()]
        }
        None => vec!["ui".into()],
    };
    fonts.families.insert(FontFamily::Name("serif".into()), serif);
    let pick = |script: &str| -> Vec<&designcraft_fonts::CraftFont> { craft.iter().filter(|f| f.scripts.contains(&script)).collect() };
    let mut japanese = pick("Jpan");
    // BIZ UDPGothic (the UI face) first, Regular before Bold; the semibold family prefers Bold.
    japanese.sort_by_key(|f| (f.family != "BIZ UDPGothic", f.style != "Regular"));
    let chinese = pick("Hans");
    let arabic = pick("Arab");
    let name = |f: &designcraft_fonts::CraftFont| format!("craft-{}-{}", f.family, f.style);
    for f in japanese.iter().chain(&chinese) {
        // CJK faces sit on the Latin baseline (see `baseline_tweak`).
        fonts.font_data.insert(name(f), Arc::new(FontData { tweak: baseline_tweak(f.bytes, 0), ..FontData::from_static(f.bytes) }));
    }
    for f in &arabic {
        add(&mut fonts, &name(f), f.bytes);
    }
    // Han characters take the forms of the interface language: Chinese faces first for Chinese.
    let zh = matches!(cjk_script(lang), "Hans" | "Hant");
    for (family, stack) in fonts.families.iter_mut() {
        let bold = matches!(family, FontFamily::Name(n) if matches!(n.as_ref(), "semibold" | "bold"));
        let mut ja = japanese.clone();
        if bold {
            ja.sort_by_key(|f| (f.family != "BIZ UDPGothic", f.style != "Bold"));
        }
        let (first, second) = if zh { (&chinese, &ja) } else { (&ja, &chinese) };
        stack.extend(first.iter().chain(second).chain(&arabic).map(|f| name(f)));
    }
    // Arabic runs (rtl.rs): the Arabic face first, so letters and spaces shape as one run.
    for family in ["arabic", "arabic-semibold"] {
        let ui = if family == "arabic" { "ui" } else { "ui-semibold" };
        let stack = arabic.iter().map(|f| name(f)).chain([ui.to_string(), "ui".to_string()]).chain(japanese.iter().chain(&chinese).map(|f| name(f)));
        let mut seen = std::collections::HashSet::new();
        fonts.families.insert(FontFamily::Name(family.into()), stack.filter(|n| seen.insert(n.clone())).collect());
    }
    fonts
}

/// The edition's typeface in front of the UI faces: Regular for the interface, Semibold for its
/// headings, Bold (else Semibold) for the wordmark. Every other face stays, as their fallbacks.
pub(crate) fn add_edition_fonts(fonts: &mut FontDefinitions, edition: &designcraft_edition::Fonts) {
    for (name, data, family) in [
        ("edition-ui", edition.regular, FontFamily::Proportional),
        ("edition-ui-semibold", edition.semibold, FontFamily::Name("semibold".into())),
        ("edition-ui-bold", edition.bold.unwrap_or(edition.semibold), FontFamily::Name("bold".into())),
    ] {
        // epaint parses every face when the fonts load and stops the app on one it can't read, and
        // the build checks only a font's first bytes: such a face is left out, for the bundled ones.
        if designcraft_fonts::line_metrics(data, 0).is_none() {
            log::warn!("the edition's {name} face isn't a font egui can read; using the bundled faces");
            continue;
        }
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(data)));
        fonts.families.entry(family).or_default().insert(0, name.into());
    }
}

/// Source Serif 4 Regular in [`designcraft_fonts::bundled`].
const SERIF: usize = 4;

/// The report's face, for quoting its text (Flow).
pub fn serif(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("serif".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn apply(ctx: &egui::Context, t: &Tokens) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, *t));
    let text_styles: std::collections::BTreeMap<TextStyle, FontId> = [
        (TextStyle::Small, FontId::proportional(10.0)),
        (TextStyle::Body, FontId::proportional(11.0)),
        (TextStyle::Button, FontId::proportional(11.0)),
        (TextStyle::Heading, FontId::new(15.0, FontFamily::Name("semibold".into()))),
        (TextStyle::Monospace, FontId::monospace(11.5)),
    ]
    .into();
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    let r = CornerRadius::same(t.radius);
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.extreme_bg_color = t.input;
    v.faint_bg_color = t.panel_darker;
    v.window_stroke = Stroke::new(1.0, t.divider);
    v.window_corner_radius = CornerRadius::same(t.frame_radius);
    v.menu_corner_radius = CornerRadius::same(t.frame_radius);
    v.selection.bg_fill = t.selection_fill;
    v.selection.stroke = Stroke::new(1.0, t.text_strong);
    v.hyperlink_color = t.accent;
    v.override_text_color = Some(t.text);
    craft_ui::theme::WidgetPalette { noninteractive: t.panel, inactive: t.button, hovered: t.hover, active: t.tool_active, open: t.hover }.apply_to(
        &mut v.widgets,
        r,
        Stroke::new(1.0, t.text),
    );
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.divider);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.field_border);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.field_border);
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.field_border);
    // InDesign buttons are outline-only.
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = t.input;
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) };
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.text_styles = text_styles.clone();
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        s.spacing.interact_size = egui::vec2(20.0, 21.0);
        s.spacing.menu_margin = egui::Margin::same(4);
        s.animation_time = 0.06;
    });
}

impl Tokens {
    /// A widget's, frame's or painted well's corner radius `r`, or none in a square theme (an
    /// edition's `radius = 0`: sharp everywhere).
    pub fn corner(&self, r: f32) -> f32 {
        if self.radius == 0 { 0.0 } else { r }
    }

    /// Tokens stored by [`apply`].
    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(egui::Id::NULL)).unwrap_or_else(|| Tokens::for_brightness(Brightness::Dark))
    }
}

/// The WCAG contrast ratio of two colours, from 1 to 21.
#[cfg(test)]
pub(crate) fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |c: Color32| {
        let linear = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod japanese_font_tests {
    fn families() -> [egui::FontFamily; 3] {
        [egui::FontFamily::Proportional, egui::FontFamily::Monospace, egui::FontFamily::Name("semibold".into())]
    }

    fn ctx_with(craft: &'static [designcraft_fonts::CraftFont]) -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::font_definitions(craft, "ja"));
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    #[test]
    fn japanese_ui_glyphs_are_available_in_every_family() {
        if designcraft_fonts::CRAFT_FONTS.is_empty() {
            eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a checkout)");
            return;
        }
        let ctx = ctx_with(designcraft_fonts::CRAFT_FONTS);
        ctx.fonts_mut(|fonts| {
            for family in families() {
                let font = egui::FontId::new(13.0, family);
                // Real glyphs (no tofu) for every character.
                for ch in "日本語の文字縦書き横書き組み方向ルビ圏点".chars() {
                    assert!(fonts.has_glyph(&font, ch), "missing {ch} in {font:?}");
                }
            }
        });
        // The UI face is BIZ UDPGothic, ahead of the Mincho faces.
        let defs = super::font_definitions(designcraft_fonts::CRAFT_FONTS, "ja");
        let stack = &defs.families[&egui::FontFamily::Proportional];
        assert_eq!(stack.iter().find(|n| n.starts_with("craft-")).map(String::as_str), Some("craft-BIZ UDPGothic-Regular"));
    }

    /// Without craft-fonts, an installed Japanese face gives the interface its glyphs (here a
    /// synthetic stand-in, so the test doesn't depend on the machine's fonts).
    #[test]
    fn installed_fonts_give_japanese_glyphs_without_craft_fonts() {
        let bytes = designcraft_fonts::testing::font_with("DC UI Japanese", &['日', '本', '語']).unwrap();
        let system = [super::SystemFont::new("Jpan", "DC UI Japanese".into(), bytes, 0)];
        let mut defs = super::font_definitions(&[], "ja");
        super::add_system_fallbacks(&mut defs, &system, "ja");
        for family in families() {
            assert_eq!(defs.families[&family].last().map(String::as_str), Some("system-DC UI Japanese"), "{family:?}");
        }
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx.fonts_mut(|fonts| {
            let font = egui::FontId::new(13.0, egui::FontFamily::Proportional);
            for ch in "日本語".chars() {
                assert!(fonts.has_glyph(&font, ch), "missing {ch}");
            }
        });
    }

    /// A fallback face with other line metrics lands on the UI face's baseline: egui puts a glyph
    /// at `ascent + (primary line height − its line height) / 2`, plus the tweak.
    #[test]
    fn fallback_faces_share_the_ui_baseline() {
        let ui = designcraft_fonts::line_metrics(super::UI_FONT, 0).unwrap();
        // A synthetic face with a taller ascent and a line gap (Hiragino-like proportions).
        let base = designcraft_fonts::testing::font_with("DC Tall", &['日']).unwrap();
        let tall = designcraft_fonts::testing::with_table_u16(base, b"hhea", 4, 1200u16).unwrap();
        let tall = designcraft_fonts::testing::with_table_u16(tall, b"hhea", 8, 500u16).unwrap();
        let face = designcraft_fonts::line_metrics(&tall, 0).unwrap();
        let tweak = super::baseline_tweak(&tall, 0);
        let em = |m: (f32, f32, f32, f32)| (m.1 / m.0, (m.1 - m.2 + m.3) / m.0);
        let ((ap, hp), (af, hf)) = (em(ui), em(face));
        let baseline = af + 0.5 * (hp - hf) + tweak.y_offset_factor;
        assert!((baseline - ap).abs() < 1e-4, "{baseline} vs {ap}");
        assert!(tweak.y_offset_factor.abs() > 0.01, "the synthetic face needs a shift: {}", tweak.y_offset_factor);
        // The UI face itself needs no shift.
        assert!(super::baseline_tweak(super::UI_FONT, 0).y_offset_factor.abs() < 1e-6);
    }

    /// Korean font names (the font menus show Korean families by their Hangul names) get glyphs
    /// from an installed Korean face, after the Japanese and Chinese ones.
    #[test]
    fn installed_korean_faces_give_hangul_glyphs() {
        let face = |family: &str, chars: &[char], script| {
            super::SystemFont::new(script, family.into(), designcraft_fonts::testing::font_with(family, chars).unwrap(), 0)
        };
        let system = [face("DC UI Korean", &['한', '글'], "Kore"), face("DC UI Japanese", &['日'], "Jpan")];
        let mut defs = super::font_definitions(&[], "ja");
        super::add_system_fallbacks(&mut defs, &system, "ja");
        let stack = &defs.families[&egui::FontFamily::Proportional];
        let at = |n: &str| stack.iter().position(|x| x == n).unwrap();
        assert!(at("system-DC UI Japanese") < at("system-DC UI Korean"));
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx.fonts_mut(|fonts| {
            let font = egui::FontId::new(13.0, egui::FontFamily::Proportional);
            assert!(fonts.has_glyph(&font, '한') && fonts.has_glyph(&font, '글'));
        });
    }

    /// Han characters take the interface language's forms: its script's installed face comes
    /// before the built-in CJK faces and the other scripts' faces.
    #[test]
    fn interface_language_orders_the_cjk_faces() {
        let face =
            |family: &str, script| super::SystemFont::new(script, family.into(), designcraft_fonts::testing::font_with(family, &['日']).unwrap(), 0);
        let system = [face("DC Ja", "Jpan"), face("DC Hans", "Hans"), face("DC Hant", "Hant"), face("DC Ko", "Kore")];
        for (lang, first) in [("ja", "DC Ja"), ("zh", "DC Hans"), ("zh-hant", "DC Hant"), ("ko", "DC Ko"), ("", "DC Ja")] {
            let mut defs = super::font_definitions(&[], lang);
            // Stand-in for a built-in CJK face (craft-fonts may be absent in this build).
            defs.families.entry(egui::FontFamily::Proportional).or_default().push("craft-Stand-in".into());
            super::add_system_fallbacks(&mut defs, &system, lang);
            let stack = &defs.families[&egui::FontFamily::Proportional];
            let at = |n: &str| stack.iter().position(|x| x == n).unwrap();
            let first_at = at(&format!("system-{first}"));
            assert!(first_at < at("craft-Stand-in"), "{lang}: {stack:?}");
            for f in &system {
                let name = format!("system-{}", f.family);
                assert!(first_at <= at(&name), "{lang}: {stack:?}");
                // Another language shares the face's data instead of copying the font file.
                assert!(std::sync::Arc::ptr_eq(&defs.font_data[&name], &f.data), "{lang}: {name}");
            }
        }
    }

    /// Tamil, Devanagari and Thai text get glyphs from installed faces, found by family and
    /// checked for coverage, after the CJK faces. Synthetic fonts in a folder of their own stand
    /// in for the machine's.
    #[test]
    fn installed_faces_give_other_scripts_glyphs_after_the_cjk_faces() {
        let dir = std::env::temp_dir().join(format!("dc-ui-script-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fonts = [
            ("Noto Sans CJK JP", &['あ', '日'][..]),
            // Listed first for Tamil but without Tamil glyphs: passed over.
            ("Tamil Sangam MN", &['a'][..]),
            // One face for two scripts is added once.
            ("Nirmala UI", &['த', 'க', 'क'][..]),
            ("Noto Sans Thai", &['ก'][..]),
        ];
        for (i, (family, chars)) in fonts.iter().enumerate() {
            std::fs::write(dir.join(format!("font{i}.ttf")), designcraft_fonts::testing::font_with(family, chars).unwrap()).unwrap();
        }
        let db = designcraft_fonts::FontDb::with_font_dirs(vec![dir.clone()]);
        let system = super::installed_ui_fonts(&db, &[]);
        let _ = std::fs::remove_dir_all(&dir);
        let picked: Vec<(&str, &str)> = system.iter().map(|f| (f.script, f.family.as_str())).collect();
        assert_eq!(picked, [("Jpan", "Noto Sans CJK JP"), ("Taml", "Nirmala UI"), ("Thai", "Noto Sans Thai")]);

        let mut defs = super::font_definitions(&[], "ja");
        defs.families.entry(egui::FontFamily::Proportional).or_default().push("craft-Stand-in".into());
        super::add_system_fallbacks(&mut defs, &system, "ja");
        let stack = &defs.families[&egui::FontFamily::Proportional];
        let at = |n: &str| stack.iter().position(|x| x == n).unwrap();
        assert!(at("system-Noto Sans CJK JP") < at("craft-Stand-in"), "{stack:?}");
        assert!(at("craft-Stand-in") < at("system-Nirmala UI") && at("system-Nirmala UI") < at("system-Noto Sans Thai"), "{stack:?}");
        defs.families.entry(egui::FontFamily::Proportional).or_default().retain(|n| n != "craft-Stand-in");
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx.fonts_mut(|fonts| {
            let font = egui::FontId::new(13.0, egui::FontFamily::Proportional);
            for ch in ['த', 'क', 'ก'] {
                assert!(fonts.has_glyph(&font, ch), "missing {ch}");
            }
        });
    }

    #[test]
    fn ui_works_without_craft_fonts() {
        let ctx = ctx_with(&[]);
        ctx.fonts_mut(|fonts| {
            // (egui's has_glyph reports false for characters of the face that also supplies the
            // replacement glyph, as Source Sans Semibold does in the semibold stack.)
            let body = egui::FontId::new(13.0, egui::FontFamily::Proportional);
            assert!(fonts.has_glyphs(&body, "DesignCraft"));
            for family in families() {
                let font = egui::FontId::new(13.0, family);
                let galley = fonts.layout_no_wrap("日本語 DesignCraft".into(), font, egui::Color32::WHITE);
                assert!(galley.size().x > 0.0);
            }
        });
        // And the real installer works with whatever this build has.
        super::install_fonts(&egui::Context::default(), "ja");
    }
}

#[cfg(test)]
mod ukrainian_font_tests {
    #[test]
    fn every_catalog_cyrillic_glyph_exists_in_the_bundled_ui_fonts() {
        // Check the actual installed face data, without craft-fonts or system fonts.
        // egui's has_glyph can report false for the face providing its replacement glyph.
        let definitions = super::font_definitions(&[], "uk");
        let db = designcraft_fonts::FontDb::with_font_dirs(Vec::new());
        db.set_system_fallback(false);
        let characters: std::collections::HashSet<_> = crate::i18n::ukrainian_catalog()
            .iter()
            .flat_map(|text| text.chars())
            .chain("ҐґЄєІіЇї".chars())
            .filter(|c| matches!(*c, '\u{0400}'..='\u{052f}'))
            .collect();
        for (name, family, style) in
            [("ui", "Source Sans 3", "Regular"), ("ui-semibold", "Source Sans 3", "Semibold"), ("mono", "JetBrains Mono", "Regular")]
        {
            let data = &definitions.font_data[name].font;
            let face = db.face(family, style);
            assert_eq!(face.data(), data.as_ref(), "configured UI face {name}");
            for character in &characters {
                assert_ne!(face.glyph_for(*character), 0, "missing {character} in {name}");
            }
        }
    }
}

#[cfg(test)]
mod flow_font_tests {
    /// Flow's serif is the bundled Source Serif 4 Regular (the index into `bundled()` stays right).
    #[test]
    fn flow_serif_is_source_serif_4() {
        assert_eq!(designcraft_fonts::bundled()[super::SERIF], &include_bytes!("../../../assets/fonts/SourceSerif4-Regular.ttf")[..]);
    }
}

#[cfg(test)]
mod russian_font_tests {
    #[test]
    fn every_catalog_cyrillic_glyph_exists_in_the_bundled_ui_fonts() {
        // Same check as for Ukrainian: the installed face data only, without craft-fonts or system fonts.
        let definitions = super::font_definitions(&[], "ru");
        let db = designcraft_fonts::FontDb::with_font_dirs(Vec::new());
        db.set_system_fallback(false);
        let characters: std::collections::HashSet<_> = crate::i18n::russian_catalog()
            .iter()
            .flat_map(|text| text.chars())
            .chain("АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя".chars())
            .filter(|c| matches!(*c, '\u{0400}'..='\u{052f}'))
            .collect();
        for (name, family, style) in
            [("ui", "Source Sans 3", "Regular"), ("ui-semibold", "Source Sans 3", "Semibold"), ("mono", "JetBrains Mono", "Regular")]
        {
            let data = &definitions.font_data[name].font;
            let face = db.face(family, style);
            assert_eq!(face.data(), data.as_ref(), "configured UI face {name}");
            for character in &characters {
                assert_ne!(face.glyph_for(*character), 0, "missing {character} in {name}");
            }
        }
    }
}

#[cfg(test)]
mod token_tests {
    use super::{Brightness, Tokens};
    use egui::Color32;

    /// The chrome colours that were literals in the panels are tokens now, and every upstream level
    /// keeps today's values, so a plain build looks exactly as before.
    #[test]
    fn upstream_levels_keep_todays_chrome_colours() {
        for b in Brightness::ALL {
            let t = Tokens::for_brightness(b);
            assert_eq!(t.selection_badge, Color32::from_rgb(0x2f, 0x73, 0xe0), "{b:?}");
            assert_eq!(t.selection_tint, Color32::from_rgb(0x88, 0xc7, 0xfb), "{b:?}");
            assert_eq!(t.badge_text, Color32::WHITE, "{b:?}");
            assert_eq!(t.page_number, Color32::WHITE, "{b:?}");
            assert_eq!(t.status_ok, Color32::from_rgb(60, 200, 90), "{b:?}");
            assert_eq!(t.status_error, Color32::from_rgb(235, 50, 50), "{b:?}");
            assert_eq!(t.link_warn, Color32::from_rgb(0xe8, 0xb3, 0x2c), "{b:?}");
            assert_eq!(t.link_error, Color32::from_rgb(0xe5, 0x4b, 0x4b), "{b:?}");
            assert_eq!(t.focus_bg, Color32::WHITE, "{b:?}");
            assert_eq!(t.focus_text, Color32::from_gray(20), "{b:?}");
            assert_eq!(t.focus_border, Color32::from_rgb(0x45, 0xa0, 0xf5), "{b:?}");
            assert_eq!(t.on_accent, Color32::WHITE, "{b:?}");
            assert_eq!(t.frame_radius, 4, "{b:?}");
            assert_eq!(t.selection_fill, t.accent_strong, "{b:?}");
            assert_eq!(t.corner(16.0), 16.0, "{b:?}");
        }
    }
}

#[cfg(test)]
mod edition_tests {
    use super::{Brightness, Tokens, over};
    use designcraft_edition::{Edition, Fonts, Theme};
    use egui::{Color32, FontFamily};

    /// Task 1's fixture (`crates/edition/tests/fixture/edition.toml`).
    const FIXTURE: Theme = crate::identity::FIXTURE_EDITION.theme;

    fn edition() -> Edition {
        crate::identity::FIXTURE_EDITION
    }

    fn rgb(c: [u8; 3]) -> Color32 {
        Color32::from_rgb(c[0], c[1], c[2])
    }

    /// Every token takes its role's colour from the edition (the spec's mapping table).
    #[test]
    fn edition_tokens_follow_the_roles() {
        let t = Tokens::from_edition(&FIXTURE);
        let at = |a| over(FIXTURE.text, a, FIXTURE.bg);
        // #e8ecf0 at 18% over #101418, worked by hand.
        assert_eq!(at(0.18), Color32::from_rgb(0x37, 0x3b, 0x3f));
        assert_eq!((over(FIXTURE.text, 1.0, FIXTURE.bg), over(FIXTURE.text, 0.0, FIXTURE.bg)), (rgb(FIXTURE.text), rgb(FIXTURE.bg)));
        for c in [t.app_bar, t.panel, t.panel_darker, t.tab_strip, t.tab_inactive, t.ruler] {
            assert_eq!(c, rgb(FIXTURE.bg));
        }
        for c in [t.input, t.button, t.well, t.tool_active, t.pasteboard, t.focus_bg] {
            assert_eq!(c, rgb(FIXTURE.raised));
        }
        assert_eq!(t.hover, rgb(FIXTURE.hover));
        for c in [t.divider, t.border, t.section_divider, t.well_rim] {
            assert_eq!(c, at(0.18));
        }
        for c in [t.input_border, t.field_border, t.text_disabled] {
            assert_eq!(c, at(0.32));
        }
        for c in [t.text, t.text_strong, t.page_number, t.focus_text] {
            assert_eq!(c, rgb(FIXTURE.text));
        }
        for c in [t.text_dim, t.icon, t.ruler_text] {
            assert_eq!(c, at(0.72));
        }
        assert_eq!(t.ruler_tick, at(0.52));
        for c in [t.accent, t.accent_strong, t.flow_file, t.selection_badge, t.selection_tint, t.focus_border] {
            assert_eq!(c, rgb(FIXTURE.accent));
        }
        assert_eq!(t.row_selected, over(FIXTURE.accent, 0.16, FIXTURE.bg));
        assert_eq!((t.badge_text, t.on_accent), (rgb(FIXTURE.on_accent), rgb(FIXTURE.on_accent)));
        assert_eq!(t.selection_fill, over(FIXTURE.accent, 0.65, FIXTURE.bg));
        // Sharp everywhere: a square theme (radius 0) drops the explicit round corners too.
        assert_eq!((t.corner(16.0), t.corner(3.0)), (0.0, 0.0));
        for c in [t.flow_ok, t.status_ok] {
            assert_eq!(c, rgb(FIXTURE.success));
        }
        for c in [t.flow_error, t.status_error, t.link_error] {
            assert_eq!(c, rgb(FIXTURE.error));
        }
        // Canvas overlays and fixed signals keep Dark's; a link's warning is Flow's amber.
        let dark = Tokens::for_brightness(Brightness::Dark);
        assert_eq!(t.link_warn, dark.flow_yours);
        for (ours, darks) in [
            (t.smart_guide, dark.smart_guide),
            (t.live_corner, dark.live_corner),
            (t.flow_error_fill, dark.flow_error_fill),
            (t.flow_yours, dark.flow_yours),
            (t.measure_bg, dark.measure_bg),
            (t.measure_text, dark.measure_text),
            (t.caption_close, dark.caption_close),
            (t.caption_close_text, dark.caption_close_text),
        ] {
            assert_eq!(ours, darks);
        }
        assert_eq!((t.radius, t.frame_radius), (0, 0));
        assert!(t.dark);
    }

    /// Text stays readable on the edition's fills (WCAG AA, 4.5:1).
    #[test]
    fn edition_tokens_meet_contrast() {
        let t = Tokens::from_edition(&FIXTURE);
        for (what, fg, bg) in [
            ("text on panel", t.text, t.panel),
            ("text on input", t.text, t.input),
            ("dim text on panel", t.text_dim, t.panel),
            ("dim text on input", t.text_dim, t.input),
            ("badge", t.badge_text, t.selection_badge),
            ("text on the selection", t.text, t.selection_fill),
        ] {
            let ratio = super::contrast(fg, bg);
            assert!(ratio >= 4.5, "{what}: {ratio:.2}");
        }
    }

    /// A setting saved by an edition ("edition") and read by a plain build gives Medium Dark.
    #[test]
    fn edition_brightness_without_an_edition_is_medium_dark() {
        let plain = Tokens::resolve(Brightness::Edition, None);
        let medium = Tokens::for_brightness(Brightness::MediumDark);
        assert_eq!((plain.panel, plain.text), (medium.panel, medium.text));
        assert_eq!(Brightness::parse("edition"), Some(Brightness::Edition));
        let saved = serde_json::to_string(&Brightness::Edition).unwrap();
        assert_eq!(saved, "\"Edition\"");
        assert_eq!(serde_json::from_str::<Brightness>(&saved).unwrap(), Brightness::Edition);
        // With an edition the level is its own, and the upstream levels ignore it.
        assert_eq!(Tokens::resolve(Brightness::Edition, Some(&edition())).panel, rgb(FIXTURE.bg));
        assert_eq!(Tokens::resolve(Brightness::Light, Some(&edition())).panel, Tokens::for_brightness(Brightness::Light).panel);
    }

    #[test]
    fn available_lists_the_edition_first() {
        assert_eq!(super::available(None), Brightness::ALL.to_vec());
        let with = super::available(Some(&edition()));
        assert_eq!(with.len(), 6);
        assert_eq!(with[0], Brightness::Edition);
        assert_eq!(&with[1..], &Brightness::ALL[..]);
    }

    /// The build checks only a font's first bytes: a face that starts like one but won't parse is
    /// left out (epaint would stop the app at every launch), and the bundled faces stand in.
    #[test]
    fn an_edition_face_that_will_not_parse_is_left_out() {
        // A truncated file: the header check passes, the parse epaint runs fails.
        let broken: &'static [u8] = b"OTTO\x00\x05";
        let serif: &'static [u8] = include_bytes!("../../../assets/fonts/SourceSerif4-Regular.ttf");
        let mut defs = super::font_definitions(&[], "en");
        super::add_edition_fonts(&mut defs, &Fonts { regular: broken, semibold: serif, bold: None });
        assert!(!defs.font_data.contains_key("edition-ui"));
        assert_eq!(defs.families[&FontFamily::Proportional].first().map(String::as_str), Some("ui"));
        assert_eq!(defs.families[&FontFamily::Name("semibold".into())].first().map(String::as_str), Some("edition-ui-semibold"));
        // And the fonts load.
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("Loads");
        });
        output.textures_delta.clear();
    }

    /// The edition's typeface goes in front of the UI faces, and every face a plain build falls
    /// back to (the built-in CJK and Arabic faces, the installed ones) stays, in the same order.
    #[test]
    fn edition_fonts_keep_every_fallback() {
        // Source Serif 4 stands in for the edition's typeface.
        let serif: &'static [u8] = include_bytes!("../../../assets/fonts/SourceSerif4-Regular.ttf");
        let fonts = Fonts { regular: serif, semibold: serif, bold: None };
        let face = |family: &str, script| {
            super::SystemFont::new(script, family.into(), designcraft_fonts::testing::font_with(family, &['日', 'ก']).unwrap(), 0)
        };
        let system = [face("DC Ja", "Jpan"), face("DC Thai", "Thai")];
        let build = |edition: Option<&Fonts>| {
            let mut defs = super::font_definitions(&[], "ja");
            // Stand-ins for the built-in CJK and Arabic faces (craft-fonts may be absent in this build).
            for stack in defs.families.values_mut() {
                stack.extend(["craft-Stand-in-Jpan".to_string(), "craft-Stand-in-Arab".to_string()]);
            }
            if let Some(f) = edition {
                super::add_edition_fonts(&mut defs, f);
            }
            super::add_system_fallbacks(&mut defs, &system, "ja");
            defs
        };
        let (plain, with) = (build(None), build(Some(&fonts)));
        let starts = |defs: &egui::FontDefinitions, family: FontFamily, names: &[&str]| {
            let stack = &defs.families[&family];
            assert!(stack.len() >= names.len() && stack.iter().zip(names).all(|(a, b)| a == b), "{family:?}: {stack:?}");
        };
        starts(&with, FontFamily::Proportional, &["edition-ui", "ui"]);
        starts(&with, FontFamily::Name("semibold".into()), &["edition-ui-semibold", "ui-semibold", "ui"]);
        starts(&with, FontFamily::Name("bold".into()), &["edition-ui-bold", "ui-semibold", "ui"]);
        starts(&plain, FontFamily::Name("bold".into()), &["ui-semibold", "ui"]);
        assert_eq!(with.families.len(), plain.families.len());
        for (family, stack) in &plain.families {
            let first = match family {
                FontFamily::Proportional => Some("edition-ui"),
                FontFamily::Name(n) if n.as_ref() == "semibold" => Some("edition-ui-semibold"),
                FontFamily::Name(n) if n.as_ref() == "bold" => Some("edition-ui-bold"),
                _ => None,
            };
            let expected: Vec<String> = first.map(String::from).into_iter().chain(stack.iter().cloned()).collect();
            assert_eq!(with.families[family], expected, "{family:?}");
        }
        // Without a Bold, the wordmark's family sets the Semibold.
        assert_eq!(with.font_data["edition-ui-bold"].font, with.font_data["edition-ui-semibold"].font);
        assert!(plain.font_data.keys().all(|k| !k.starts_with("edition-")));
    }
}
