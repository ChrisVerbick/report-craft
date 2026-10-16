//! Text import (File › Place of text files): plain text, Word documents (`.docx`) and RTF become
//! a frameless [`Story`] with paragraph and character styles, local bold/italic/underline/size,
//! footnotes, tables and bullets/numbering (Word), ready to flow into text frames.
//!
//! Word styles are imported by name (their font, size, bold/italic and space before/after become
//! the style definition, like InDesign's "Use Word styles" import option). RTF keeps bold,
//! italic, underline, size and paragraph breaks.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod docx;
mod docx_numbering;
pub mod export;
mod rtf;
pub mod tagged;
mod xlsx;

/// Data-merge row cap (header not included). More than this is an error.
pub const DATA_MERGE_MAX_ROWS: usize = 100_000;
/// Data-merge column cap. More than this is an error.
pub const DATA_MERGE_MAX_COLS: usize = 200;

/// Worksheet rows for data merge. Empty cells inside the header width are kept.
/// `sheet` names a worksheet; `None` is the first sheet in workbook order.
/// Place keeps [`xlsx::import`], which drops empty cells.
pub fn xlsx_records(bytes: &[u8], sheet: Option<&str>) -> Result<Vec<Vec<String>>, ImportError> {
    xlsx::records(bytes, sheet)
}

use designcraft_doc::{CharAttrs, ParaAttrs, Story};

/// Joins UTF-16 code units that arrive one at a time (RTF `\uN`, Tagged Text `<0xNNNN>`) into
/// characters. A character past U+FFFF comes as two units, a surrogate pair: the first half
/// waits for the second.
#[derive(Default)]
pub(crate) struct Utf16Units {
    high: Option<u32>,
}

impl Utf16Units {
    /// The character that `unit` is or completes. Half a pair without its partner is no character.
    pub(crate) fn push(&mut self, unit: u32) -> Option<char> {
        match (self.high.take(), unit) {
            (Some(high), 0xDC00..=0xDFFF) => char::from_u32(0x10000 + ((high - 0xD800) << 10) + (unit - 0xDC00)),
            (_, 0xD800..=0xDBFF) => {
                self.high = Some(unit);
                None
            }
            _ => char::from_u32(unit),
        }
    }

    /// Other text came in between: a waiting first half has no partner.
    pub(crate) fn reset(&mut self) {
        self.high = None;
    }
}

/// A paragraph or character style found in the source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedStyle {
    pub name: String,
    pub based_on: Option<String>,
    pub para: ParaAttrs,
    pub chars: CharAttrs,
}

/// The imported text.
#[derive(Clone, Debug)]
pub struct Imported {
    pub story: Story,
    pub para_styles: Vec<ImportedStyle>,
    pub char_styles: Vec<ImportedStyle>,
    /// Anything the import couldn't keep (images, fields, …).
    pub warnings: Vec<String>,
    /// What the file shows as errors in its own application (see [`Problem`]).
    pub problems: Vec<Problem>,
}

/// Something the source file shows as an error in its own application, found on import so an
/// update can stop or flag it before it reaches the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    /// Where, as the file's application names it: a cell ("B4"), "paragraph 3", "table cell 2, 1".
    pub at: String,
    /// Its table cell (row, column of the placed table), for a spreadsheet cell or a table cell.
    pub cell: Option<(usize, usize)>,
    /// What the file shows there ("#REF!", "Error! Reference source not found.").
    pub shows: String,
    /// The cell's formula, for a spreadsheet cell.
    pub formula: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// A spreadsheet error value (#REF!, #DIV/0!, #N/A…).
    ErrorValue,
    /// A formula taking its value from another workbook: the value may be out of date.
    OtherWorkbook,
    /// A formula with no calculated value.
    NotCalculated,
    /// A Word field showing an error (a cross-reference to something gone, a missing bookmark…).
    FieldError,
}

impl Problem {
    /// One line for a report or a dialog.
    pub fn message(&self) -> String {
        let formula = self.formula.as_deref().map(|f| format!("={f}")).unwrap_or_default();
        match self.kind {
            ProblemKind::ErrorValue if formula.is_empty() => format!("{}: {}", self.at, self.shows),
            ProblemKind::ErrorValue => format!("{}: {} (formula {formula})", self.at, self.shows),
            ProblemKind::OtherWorkbook => format!("{}: takes its value from another workbook ({formula}); it may be out of date", self.at),
            ProblemKind::NotCalculated => {
                format!("{}: the formula {formula} has no calculated value; open and save the file in Excel", self.at)
            }
            ProblemKind::FieldError => format!("{}: Word shows “{}”", self.at, self.shows),
        }
    }
}

/// What Word shows for a field it can't fill (a cross-reference to a heading that's gone, a
/// bookmark no longer defined…), in the languages reports are most often written in.
const FIELD_ERRORS: &[&str] = &[
    "Error! Reference source not found.",
    "Error! Bookmark not defined.",
    "Error! No text of specified style in document.",
    "Error! Not a valid bookmark self-reference.",
    "Error! Hyperlink reference not valid.",
    "Error! No table of contents entries found.",
    "Fehler! Verweisquelle konnte nicht gefunden werden.",
    "Fehler! Textmarke nicht definiert.",
    "Erreur ! Source du renvoi introuvable.",
    "Erreur ! Signet non défini.",
    "¡Error! No se encuentra el origen de la referencia.",
    "¡Error! Marcador no definido.",
    "Errore. L'origine riferimento non è stata trovata.",
    "Errore. Il segnalibro non è definito.",
    "Fout! Verwijzingsbron niet gevonden.",
    "Fout! Bladwijzer niet gedefinieerd.",
    "Erro! A origem da referência não foi encontrada.",
    "Erro! Indicador não definido.",
];

/// Word field errors in `story` and its table cells.
pub fn field_errors(story: &Story) -> Vec<Problem> {
    // Word may put a no-break space before French punctuation.
    let found = |text: &str| {
        let text = text.replace(['\u{A0}', '\u{202F}'], " ");
        FIELD_ERRORS.iter().filter(move |e| text.contains(*e)).map(|e| (*e).to_string()).collect::<Vec<_>>()
    };
    let problem = |at: String, cell, shows| Problem { kind: ProblemKind::FieldError, at, cell, shows, formula: None };
    let mut out = Vec::new();
    for (i, r) in story.para_ranges().into_iter().enumerate() {
        for shows in found(story.text.get(r).unwrap_or_default()) {
            out.push(problem(format!("paragraph {}", i + 1), None, shows));
        }
    }
    for t in story.tables.values() {
        let cols = t.ncols().max(1);
        for (k, c) in t.cells.iter().enumerate() {
            let (row, col) = (k / cols, k % cols);
            for shows in found(&c.text.text) {
                out.push(problem(format!("table cell {}, {}", row + 1, col + 1), Some((row, col)), shows));
            }
        }
    }
    out
}

#[derive(Debug, PartialEq)]
pub enum ImportError {
    Unsupported(String),
    Corrupt(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Unsupported(s) => write!(f, "unsupported text file: {s}"),
            ImportError::Corrupt(s) => write!(f, "can't read the file: {s}"),
        }
    }
}

/// Text files this crate reads, by extension.
pub fn is_text_file(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".txt") || l.ends_with(".text") || l.ends_with(".md") || l.ends_with(".docx") || l.ends_with(".rtf") || l.ends_with(".xlsx")
}

/// Import `bytes` (format from the file name, else sniffed), with the errors the file shows.
pub fn import(name: &str, bytes: &[u8]) -> Result<Imported, ImportError> {
    let mut imp = import_format(name, bytes)?;
    let fields = field_errors(&imp.story);
    imp.problems.extend(fields);
    Ok(imp)
}

fn import_format(name: &str, bytes: &[u8]) -> Result<Imported, ImportError> {
    let l = name.to_ascii_lowercase();
    if l.ends_with(".xlsx") {
        return xlsx::import(bytes);
    }
    if l.ends_with(".docx") || bytes.starts_with(b"PK\x03\x04") {
        return docx::import(bytes);
    }
    if l.ends_with(".rtf") || bytes.starts_with(b"{\\rtf") {
        return rtf::import(bytes);
    }
    if tagged::is_tagged(bytes) {
        return tagged::import(bytes);
    }
    if l.ends_with(".doc") {
        return Err(ImportError::Unsupported("legacy Word .doc (save it as .docx)".into()));
    }
    Ok(plain(&String::from_utf8_lossy(bytes)))
}

/// Plain text: line breaks become paragraphs (CRLF / CR normalised).
pub fn plain(text: &str) -> Imported {
    let t = text.trim_start_matches('\u{FEFF}').replace("\r\n", "\n").replace('\r', "\n");
    let story = Story::with_text(designcraft_doc::StoryId(0), &t, Default::default());
    Imported { story, para_styles: vec![], char_styles: vec![], warnings: vec![], problems: vec![] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_field_errors_are_found_by_paragraph() {
        let imp =
            import("a.txt", "Intro.\nSee Error! Reference source not found. for details.\nFehler! Textmarke nicht definiert.".as_bytes()).unwrap();
        let found: Vec<(&str, &str)> = imp.problems.iter().map(|p| (p.at.as_str(), p.shows.as_str())).collect();
        assert_eq!(found, [("paragraph 2", "Error! Reference source not found."), ("paragraph 3", "Fehler! Textmarke nicht definiert.")]);
        assert!(imp.problems.iter().all(|p| p.kind == ProblemKind::FieldError));
        assert_eq!(imp.problems[0].message(), "paragraph 2: Word shows “Error! Reference source not found.”");
    }

    #[test]
    fn plain_text_paragraphs() {
        let i = import("a.txt", b"\xEF\xBB\xBFOne\r\nTwo\rThree").unwrap();
        assert_eq!(i.story.text, "One\nTwo\nThree");
        assert_eq!(i.story.paras.len(), 3);
        assert!(is_text_file("Report.DOCX") && !is_text_file("a.png"));
        assert!(matches!(import("x.doc", b"\xD0\xCF"), Err(ImportError::Unsupported(_))));
    }
}
