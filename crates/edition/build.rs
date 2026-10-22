//! Embeds the edition that `DESIGNCRAFT_EDITION` (an absolute path) describes, or none.

mod parse;

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed=DESIGNCRAFT_EDITION");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").map_err(|e| e.to_string())?).join("edition.rs");
    let code = match parse::edition_dir(std::env::var("DESIGNCRAFT_EDITION").ok().as_deref())? {
        None => "pub static EDITION: Option<&Edition> = None;\n".to_string(),
        Some(dir) => {
            let file = dir.join("edition.toml");
            println!("cargo:rerun-if-changed={}", dir.display());
            println!("cargo:rerun-if-changed={}", file.display());
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let edition = parse::parse_edition(&text, &dir)?;
            for f in edition.files() {
                println!("cargo:rerun-if-changed={}", f.display());
            }
            parse::generated_rust(&edition)
        }
    };
    std::fs::write(&out, code).map_err(|e| format!("{}: {e}", out.display()))
}
