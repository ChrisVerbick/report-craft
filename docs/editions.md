# Editions

An edition is DesignCraft built under another name and look: its own name, colours, mark and
typeface, compiled in. Like `CRAFT_FONTS_DIR`, it is an optional build input: a folder outside
this repository that `DESIGNCRAFT_EDITION` names, as an absolute path. Without it, the build is
plain DesignCraft.

The folder holds `edition.toml` and the files it names. The test fixture,
`crates/edition/tests/fixture/`:

```toml
name = "Test Edition"                   # where the app names itself
bundle_id = "org.example.test-edition"  # the macOS bundle identifier
data_folder = "Test Edition"            # settings, logs and recovery files
wordmark = "TEST"                       # under the mark in the About box

[logo]
mark = "mark.svg"                       # square SVG: app bar, About box, app icon

[theme]
bg = "#101418"                          # also raised, hover, text, accent, on_accent, success, error
secondary = 0.72                        # also muted, faint, hairline: text's alphas over bg
radius = 0                              # corners, 0 to 16
```

The fixture's `[theme]` sets all thirteen keys. An optional `[fonts]` table names the interface
typeface: `regular`, `semibold` and an optional `bold` for the wordmark, OpenType or TrueType.
Without it, the About box shows the mark alone. The build stops on an unknown key, a missing file,
a colour that isn't `#rrggbb` or an alpha outside 0 to 1, and names what is wrong.

Run it, or package it as a Mac app (`target/edition/<name>.app`, signed ad hoc for this Mac):

    DESIGNCRAFT_EDITION=/abs/path/to/edition cargo run --release -p designcraft
    DESIGNCRAFT_EDITION=/abs/path/to/edition packaging/macos/edition.sh

The code is MIT or Apache 2.0, but the ArtCraft marks are not: forks and modified versions must
remove or replace them ([`docs/brand/LICENSE-brand.txt`](brand/LICENSE-brand.txt)). An edition
does so. It leaves out the ArtCraft mark, Discord and website links, and credits DesignCraft in
plain text in its About box. Keep an edition's own brand files and fonts out of this repository.
