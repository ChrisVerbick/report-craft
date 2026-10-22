//! An optional build-time edition: a name, a theme, a mark and a typeface, read from the folder
//! `DESIGNCRAFT_EDITION` names (an absolute path) when this crate is built, as `CRAFT_FONTS_DIR`
//! is read by `designcraft-fonts`. Without it the build is plain DesignCraft and [`current`] is
//! `None`. The folder's format is in `docs/editions.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

/// An edition's identity, look and files, embedded when this crate was built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edition {
    /// The app's name where it names itself ("About …", the title bar, the menu bar).
    pub name: &'static str,
    /// The macOS bundle identifier.
    pub bundle_id: &'static str,
    /// The settings and recovery folder's name (`~/Library/Application Support/<data_folder>`).
    pub data_folder: &'static str,
    /// The text set beside the mark in the About box's lockup.
    pub wordmark: &'static str,
    /// The square mark (SVG): app bar, About box, Dock and app icon.
    pub mark_svg: &'static [u8],
    pub theme: Theme,
    pub fonts: Option<Fonts>,
}

/// Colours by role (sRGB) and alphas of `text` over `bg`, as the edition's `[theme]` gives them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub bg: [u8; 3],
    pub raised: [u8; 3],
    pub hover: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
    pub on_accent: [u8; 3],
    pub success: [u8; 3],
    pub error: [u8; 3],
    pub secondary: f32,
    pub muted: f32,
    pub faint: f32,
    pub hairline: f32,
    pub radius: u8,
}

/// The interface typeface (OpenType or TrueType). Bold sets the wordmark; without it, Semibold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fonts {
    pub regular: &'static [u8],
    pub semibold: &'static [u8],
    pub bold: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/edition.rs"));

/// The edition this build carries, if any.
pub fn current() -> Option<&'static Edition> {
    EDITION
}

/// The app's own name: the edition's, else "DesignCraft".
pub fn app_name() -> &'static str {
    EDITION.map_or("DesignCraft", |e| e.name)
}

/// The settings and recovery folder's name: the edition's, else "DesignCraft".
pub fn data_folder() -> &'static str {
    EDITION.map_or("DesignCraft", |e| e.data_folder)
}

#[cfg(test)]
#[path = "../parse.rs"]
mod parse;

#[cfg(test)]
mod tests {
    /// A build without `DESIGNCRAFT_EDITION` (CI's) carries no edition and names itself DesignCraft.
    #[test]
    fn the_plain_build_has_no_edition() {
        if option_env!("DESIGNCRAFT_EDITION").is_some_and(|v| !v.is_empty()) {
            return; // built as an edition: nothing to check here
        }
        assert!(super::current().is_none());
        assert_eq!(super::app_name(), "DesignCraft");
        assert_eq!(super::data_folder(), "DesignCraft");
    }
}
