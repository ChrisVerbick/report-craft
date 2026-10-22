//! Reading an edition folder: shared by the build script, which embeds it, and this crate's tests.
//! It sits beside `build.rs`, outside `src/`, because the shipped library never compiles it: the
//! engine's file guard (`tests_file_guard.rs`) scans `src/` for code a command could reach.

use std::path::{Path, PathBuf};

/// An edition as read from its folder: the values the build embeds, with absolute file paths.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub name: String,
    pub bundle_id: String,
    pub data_folder: String,
    pub wordmark: String,
    pub mark: PathBuf,
    pub theme: ParsedTheme,
    pub fonts: Option<ParsedFonts>,
}

/// `[theme]`: colours by role and alphas, as the crate's `Theme` holds them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParsedTheme {
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

/// `[fonts]`: the interface typeface's files.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedFonts {
    pub regular: PathBuf,
    pub semibold: PathBuf,
    pub bold: Option<PathBuf>,
}

impl Parsed {
    /// Every file the edition names, for the build to watch.
    pub fn files(&self) -> Vec<&Path> {
        let mut files = vec![self.mark.as_path()];
        if let Some(f) = &self.fonts {
            files.extend([f.regular.as_path(), f.semibold.as_path()]);
            files.extend(f.bold.as_deref());
        }
        files
    }
}

/// The folder `DESIGNCRAFT_EDITION` names: none when it is unset or empty. It must be an absolute
/// path to a folder, since cargo runs build scripts from the crate's own folder.
pub fn edition_dir(env: Option<&str>) -> Result<Option<PathBuf>, String> {
    let Some(value) = env.filter(|v| !v.trim().is_empty()) else { return Ok(None) };
    let dir = PathBuf::from(value);
    if !dir.is_absolute() {
        return Err(format!("DESIGNCRAFT_EDITION must be an absolute path (cargo runs build scripts from the crate's folder): {value}"));
    }
    if !dir.is_dir() {
        return Err(format!("DESIGNCRAFT_EDITION: no folder at {}", dir.display()));
    }
    Ok(Some(dir))
}

const TOP: &[&str] = &["name", "bundle_id", "data_folder", "wordmark", "logo", "theme", "fonts"];
const THEME: &[&str] =
    &["bg", "raised", "hover", "text", "accent", "on_accent", "success", "error", "secondary", "muted", "faint", "hairline", "radius"];

/// `edition.toml`, its files looked up in `dir`.
pub fn parse_edition(toml_text: &str, dir: &Path) -> Result<Parsed, String> {
    let top: toml::Table = toml_text.parse().map_err(|e| format!("edition.toml: {e}"))?;
    known(&top, "", TOP)?;
    let text = |k: &str| top.get(k).and_then(toml::Value::as_str).map(str::to_string).ok_or_else(|| format!("edition.toml: `{k}` (text) is missing"));
    let logo = table(&top, "logo")?;
    known(logo, "logo", &["mark"])?;
    let theme = table(&top, "theme")?;
    known(theme, "theme", THEME)?;
    let colour = |k: &str| colour(theme, k);
    let alpha = |k: &str| alpha(theme, k);
    let radius = theme
        .get("radius")
        .and_then(toml::Value::as_integer)
        .and_then(|r| u8::try_from(r).ok())
        .filter(|r| *r <= 16)
        .ok_or_else(|| "edition.toml: theme.radius must be a whole number from 0 to 16".to_string())?;
    let fonts = match top.get("fonts") {
        None => None,
        Some(v) => {
            let f = v.as_table().ok_or_else(|| "edition.toml: `fonts` must be a table".to_string())?;
            known(f, "fonts", &["regular", "semibold", "bold"])?;
            let bold = if f.contains_key("bold") { Some(font(dir, f, "bold")?) } else { None };
            Some(ParsedFonts { regular: font(dir, f, "regular")?, semibold: font(dir, f, "semibold")?, bold })
        }
    };
    Ok(Parsed {
        name: text("name")?,
        bundle_id: text("bundle_id")?,
        data_folder: text("data_folder")?,
        wordmark: text("wordmark")?,
        mark: file(dir, logo, "logo", "mark")?,
        theme: ParsedTheme {
            bg: colour("bg")?,
            raised: colour("raised")?,
            hover: colour("hover")?,
            text: colour("text")?,
            accent: colour("accent")?,
            on_accent: colour("on_accent")?,
            success: colour("success")?,
            error: colour("error")?,
            secondary: alpha("secondary")?,
            muted: alpha("muted")?,
            faint: alpha("faint")?,
            hairline: alpha("hairline")?,
            radius,
        },
        fonts,
    })
}

/// The `$OUT_DIR/edition.rs` text that embeds `p`.
pub fn generated_rust(p: &Parsed) -> String {
    let bytes = |f: &Path| format!("include_bytes!({:?})", f.to_string_lossy());
    let rgb = |c: [u8; 3]| format!("[{}, {}, {}]", c[0], c[1], c[2]);
    let t = &p.theme;
    let fonts = match &p.fonts {
        None => "None".to_string(),
        Some(f) => format!(
            "Some(Fonts {{ regular: {}, semibold: {}, bold: {} }})",
            bytes(&f.regular),
            bytes(&f.semibold),
            f.bold.as_deref().map_or_else(|| "None".to_string(), |b| format!("Some({})", bytes(b)))
        ),
    };
    format!(
        "pub static EDITION: Option<&Edition> = Some(&Edition {{\n    name: {:?},\n    bundle_id: {:?},\n    data_folder: {:?},\n    wordmark: {:?},\n    mark_svg: {},\n    theme: Theme {{\n        bg: {}, raised: {}, hover: {}, text: {}, accent: {}, on_accent: {}, success: {}, error: {},\n        secondary: {:?}, muted: {:?}, faint: {:?}, hairline: {:?}, radius: {},\n    }},\n    fonts: {},\n}});\n",
        p.name,
        p.bundle_id,
        p.data_folder,
        p.wordmark,
        bytes(&p.mark),
        rgb(t.bg),
        rgb(t.raised),
        rgb(t.hover),
        rgb(t.text),
        rgb(t.accent),
        rgb(t.on_accent),
        rgb(t.success),
        rgb(t.error),
        t.secondary,
        t.muted,
        t.faint,
        t.hairline,
        t.radius,
        fonts,
    )
}

fn known(t: &toml::Table, section: &str, keys: &[&str]) -> Result<(), String> {
    match t.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) if section.is_empty() => Err(format!("edition.toml: unknown key `{k}`")),
        Some(k) => Err(format!("edition.toml: unknown key `{k}` in [{section}]")),
        None => Ok(()),
    }
}

fn table<'a>(t: &'a toml::Table, k: &str) -> Result<&'a toml::Table, String> {
    t.get(k).and_then(toml::Value::as_table).ok_or_else(|| format!("edition.toml: the [{k}] table is missing"))
}

fn colour(t: &toml::Table, k: &str) -> Result<[u8; 3], String> {
    let bad = || format!("edition.toml: theme.{k} must be a colour like \"#1a2b3c\"");
    let hex = t
        .get(k)
        .and_then(toml::Value::as_str)
        .and_then(|s| s.strip_prefix('#'))
        .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(bad)?;
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(bad);
    Ok([byte(0)?, byte(2)?, byte(4)?])
}

fn alpha(t: &toml::Table, k: &str) -> Result<f32, String> {
    match t.get(k).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))) {
        Some(a) if (0.0..=1.0).contains(&a) => Ok(a as f32),
        _ => Err(format!("edition.toml: theme.{k} must be a number from 0 to 1")),
    }
}

/// `section.k`, a file name, as a path in `dir` that must exist.
fn file(dir: &Path, t: &toml::Table, section: &str, k: &str) -> Result<PathBuf, String> {
    let name = t.get(k).and_then(toml::Value::as_str).ok_or_else(|| format!("edition.toml: {section}.{k} (a file name) is missing"))?;
    let path = dir.join(name);
    if path.is_file() { Ok(path) } else { Err(format!("edition.toml: {section}.{k}: no file {name} in {}", dir.display())) }
}

/// `fonts.k`: a file that starts as an OpenType or TrueType font does.
fn font(dir: &Path, t: &toml::Table, k: &str) -> Result<PathBuf, String> {
    let path = file(dir, t, "fonts", k)?;
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    match data.get(..4) {
        Some([0, 1, 0, 0] | [b'O', b'T', b'T', b'O'] | [b't', b'r', b'u', b'e']) => Ok(path),
        _ => Err(format!(
            "edition.toml: fonts.{k}: {} isn't an OpenType or TrueType font",
            path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture")
    }

    fn fixture_toml() -> String {
        std::fs::read_to_string(fixture().join("edition.toml")).unwrap()
    }

    /// The fixture's text with `old` (which it must hold) replaced by `new`.
    fn with(old: &str, new: &str) -> String {
        let t = fixture_toml();
        assert!(t.contains(old), "{old}");
        t.replacen(old, new, 1)
    }

    /// An empty variable is no edition (CI may set it to ""); a relative path is refused, since cargo
    /// runs build scripts from the crate's own folder.
    #[test]
    fn edition_dir_from_env() {
        assert_eq!(edition_dir(None), Ok(None));
        assert_eq!(edition_dir(Some("")), Ok(None));
        assert!(edition_dir(Some("rel/dir")).unwrap_err().contains("absolute"));
        assert_eq!(edition_dir(Some(fixture().to_str().unwrap())), Ok(Some(fixture())));
        let missing = fixture().join("no-such-folder");
        assert!(edition_dir(Some(missing.to_str().unwrap())).unwrap_err().contains(missing.to_str().unwrap()));
    }

    #[test]
    fn the_fixture_parses() {
        let p = parse_edition(&fixture_toml(), &fixture()).unwrap();
        assert_eq!(p.name, "Test Edition");
        assert_eq!(p.theme.bg, [0x10, 0x14, 0x18]);
        assert_eq!(p.theme.secondary, 0.72);
        assert_eq!(p.theme.radius, 0);
        assert_eq!(p.fonts, None);
        assert!(p.mark.is_absolute() && p.mark.ends_with("mark.svg"), "{}", p.mark.display());
        assert_eq!(p.files(), vec![p.mark.as_path()], "the build watches the mark (no fonts here)");
    }

    /// Each kind of mistake stops the build with a message naming the key or file.
    #[test]
    fn bad_editions_say_what_is_wrong() {
        let err = |text: String| parse_edition(&text, &fixture()).unwrap_err();
        assert!(err(with("[theme]", "[theme]\ncolour = \"#000000\"")).contains("colour"));
        assert!(err(with("name = \"Test Edition\"\n", "")).contains("name"));
        assert!(err(with("bg = \"#101418\"", "bg = \"black\"")).contains("bg"));
        assert!(err(with("secondary = 0.72", "secondary = 1.5")).contains("secondary"));
        assert!(err(with("mark = \"mark.svg\"", "mark = \"nope.svg\"")).contains("nope.svg"));
        // A font that is neither OpenType nor TrueType: the fixture's SVG named as one.
        let fonts = format!("{}\n[fonts]\nregular = \"mark.svg\"\nsemibold = \"mark.svg\"\n", fixture_toml());
        assert!(err(fonts).contains("mark.svg"));
    }

    #[test]
    fn generated_rust_embeds_the_files() {
        let p = parse_edition(&fixture_toml(), &fixture()).unwrap();
        let code = generated_rust(&p);
        assert!(code.contains("Some(&Edition {"), "{code}");
        assert!(code.contains("name: \"Test Edition\""), "{code}");
        assert!(code.contains(&format!("include_bytes!({:?})", p.mark.to_string_lossy())), "{code}");
    }
}
