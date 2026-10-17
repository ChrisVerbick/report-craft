//! Excel workbooks (`.xlsx`) → a story holding one table: the first worksheet's used range, or the
//! sheet, cell range and header rows [`XlsxOptions`] name; cell values as shown text (shared and
//! inline strings, numbers, booleans). Data merge uses [`records`], which keeps empty cells and can
//! name a sheet.

use std::collections::BTreeMap;

use designcraft_doc::{CharFormat, ParaFormat, Story, StoryId, Table};

use crate::docx::{El, parse, part};
use crate::numfmt;
use crate::{ImportError, Imported, Problem, ProblemKind, XlsxOptions};

/// "B12" → (row 11, column 1).
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let row: usize = r[letters.len()..].parse().ok()?;
    let col = letters.chars().try_fold(0usize, |acc, c| {
        let digit = (c.to_ascii_uppercase() as u8).checked_sub(b'A')? as usize + 1;
        acc.checked_mul(26)?.checked_add(digit)
    })?;
    Some((row.checked_sub(1)?, col.checked_sub(1)?))
}

/// "B3:F40", "$B$3:$F$40", "f40:b3" or one cell → its (top-left, bottom-right) cells, from 0.
fn parse_range(r: &str) -> Option<((usize, usize), (usize, usize))> {
    let r = r.trim().replace('$', "");
    let (a, b) = r.split_once(':').unwrap_or((&r, &r));
    let ((r0, c0), (r1, c1)) = (cell_ref(a.trim())?, cell_ref(b.trim())?);
    Some(((r0.min(r1), c0.min(c1)), (r0.max(r1), c0.max(c1))))
}

/// The workbook's sheet names in workbook order (none without a workbook part).
fn names(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Result<Vec<String>, ImportError> {
    let Some(wb) = part(zip, "xl/workbook.xml")? else { return Ok(Vec::new()) };
    let wb = parse(&wb)?;
    Ok(wb
        .child("sheets")
        .map(|s| s.els().filter(|e| e.name == "sheet").filter_map(|e| e.attr("name").map(str::to_string)).collect())
        .unwrap_or_default())
}

pub fn sheet_names(bytes: &[u8]) -> Result<Vec<String>, ImportError> {
    names(&mut zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Corrupt(e.to_string()))?)
}

/// Each cell style's number format (`cellXfs`, by the cells' `s`): its id, and its code from the
/// workbook's own formats or the built-in ones (none for a built-in this doesn't know).
fn number_formats(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Result<Vec<(u32, Option<String>)>, ImportError> {
    let Some(x) = part(zip, "xl/styles.xml")? else { return Ok(Vec::new()) };
    let st = parse(&x)?;
    let own: BTreeMap<u32, String> = st
        .child("numFmts")
        .map(|n| {
            n.els()
                .filter(|e| e.name == "numFmt")
                .filter_map(|e| Some((e.attr("numFmtId")?.trim().parse().ok()?, e.attr("formatCode")?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let id = |xf: &El| xf.attr("numFmtId").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
    Ok(st
        .child("cellXfs")
        .map(|x| {
            x.els()
                .filter(|e| e.name == "xf")
                .map(|xf| (id(xf), own.get(&id(xf)).cloned().or_else(|| numfmt::builtin(id(xf)).map(str::to_string))))
                .collect()
        })
        .unwrap_or_default())
}

/// Whether the workbook counts dates from 1904 (`workbookPr date1904`).
fn date1904(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Result<bool, ImportError> {
    let Some(wb) = part(zip, "xl/workbook.xml")? else { return Ok(false) };
    let wb = parse(&wb)?;
    Ok(wb.child("workbookPr").and_then(|p| p.attr("date1904")).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")))
}

/// "A1", "A1 and B2", "A1, B2, C3 and 4 more".
fn cell_list(at: &[String]) -> String {
    match at {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, c, rest @ ..] => format!("{a}, {b}, {c} and {} more", rest.len()),
    }
}

/// The worksheet part of the first sheet (workbook order), via the workbook relationships.
fn first_sheet(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Result<Option<String>, ImportError> {
    let Some(wb) = part(zip, "xl/workbook.xml")? else { return Ok(None) };
    let Some(rels) = part(zip, "xl/_rels/workbook.xml.rels")? else { return Ok(None) };
    Ok(first_sheet_target(&wb, &rels))
}

fn first_sheet_target(wb: &str, rels: &str) -> Option<String> {
    let wb = parse(wb).ok()?;
    let rid = wb.child("sheets")?.els().find(|e| e.name == "sheet")?.attr("id")?.to_string();
    let rels = parse(rels).ok()?;
    let target = rels.els().find(|r| r.attr("Id") == Some(rid.as_str()))?.attr("Target")?.trim_start_matches('/').to_string();
    Some(if target.starts_with("xl/") { target } else { format!("xl/{target}") })
}

/// Whether formula `f` takes a value from another workbook: Excel writes those references with
/// the linked workbook's number, `[1]Sheet!A1` (a table's `Table1[Column]` is not one).
fn other_workbook(f: &str) -> bool {
    f.match_indices('[').any(|(i, _)| {
        let rest = f.get(i + 1..).unwrap_or_default();
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && rest.get(digits..).is_some_and(|r| r.starts_with(']'))
    })
}

/// Excel's name for the cell at (row, column), both from 0: (1, 1) is B2.
fn a1((row, col): (usize, usize)) -> String {
    let mut letters = Vec::new();
    let mut n = col.saturating_add(1);
    while n > 0 {
        letters.push(char::from(b'A' + ((n - 1) % 26) as u8));
        n = (n - 1) / 26;
    }
    letters.iter().rev().collect::<String>() + &row.saturating_add(1).to_string()
}

fn value(c: &El, shared: &[String]) -> String {
    let v = || c.child("v").map(El::text).unwrap_or_default();
    match c.attr("t") {
        Some("s") => v().trim().parse::<usize>().ok().and_then(|i| shared.get(i).cloned()).unwrap_or_default(),
        Some("inlineStr") => c.child("is").map(El::text).unwrap_or_default(),
        // A cached formula string is text even when it looks like a number.
        Some("str") => v(),
        Some("b") => {
            if v().trim() == "1" {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }
        _ => {
            let raw = v();
            match raw.trim().parse::<f64>() {
                Ok(f) if f.is_finite() => numfmt::general(f),
                _ => raw,
            }
        }
    }
}

/// One worksheet as a grid of cached cell text, empty cells kept, for data merge.
/// The first used row is the header. Later rows that are entirely empty are left out.
/// Cells past the header are kept so the caller can warn. Over 200 columns or 100,000
/// data rows is an error and the grid is not built.
pub fn records(bytes: &[u8], sheet: Option<&str>) -> Result<Vec<Vec<String>>, ImportError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Corrupt(e.to_string()))?;
    let sheet_part = sheet_target(&mut zip, sheet)?;
    let sheet_xml = parse(&part(&mut zip, &sheet_part)?.ok_or_else(|| ImportError::Corrupt(format!("no {sheet_part} (not an Excel workbook)")))?)?;
    let shared: Vec<String> = part(&mut zip, "xl/sharedStrings.xml")?
        .map(|x| parse(&x))
        .transpose()?
        .map(|sst| sst.els().filter(|e| e.name == "si").map(El::text).collect())
        .unwrap_or_default();
    let mut cells: Vec<(usize, usize, String)> = Vec::new();
    if let Some(data) = sheet_xml.child("sheetData") {
        for (ri, row) in data.els().filter(|e| e.name == "row").enumerate() {
            for (ci, c) in row.els().filter(|e| e.name == "c").enumerate() {
                let (r, col) = c.attr("r").and_then(cell_ref).unwrap_or((ri, ci));
                cells.push((r, col, value(c, &shared)));
            }
        }
    }
    if cells.is_empty() {
        return Err(ImportError::Corrupt("the worksheet is empty".into()));
    }
    let header_row = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let mut min_c = usize::MAX;
    let mut max_c = 0usize;
    let mut header_used = false;
    for (r, c, _) in &cells {
        if *r == header_row {
            header_used = true;
            min_c = min_c.min(*c);
            max_c = max_c.max(*c);
        }
    }
    if !header_used || min_c == usize::MAX {
        return Err(ImportError::Corrupt("the worksheet has no header".into()));
    }
    let width = max_c.saturating_sub(min_c).saturating_add(1);
    if width > crate::DATA_MERGE_MAX_COLS {
        return Err(ImportError::Corrupt(format!("the sheet has {width} columns; the limit is {}", crate::DATA_MERGE_MAX_COLS)));
    }
    // Group by row. Gaps (empty rows) are not materialised.
    let mut by_row: BTreeMap<usize, Vec<(usize, String)>> = BTreeMap::new();
    for (r, c, v) in cells {
        by_row.entry(r).or_default().push((c, v));
    }
    let mut data_rows = 0usize;
    for (r, cols) in &by_row {
        if *r == header_row {
            continue;
        }
        let any = cols.iter().any(|(_, v)| !v.is_empty());
        if any {
            data_rows = data_rows.saturating_add(1);
        }
    }
    if data_rows > crate::DATA_MERGE_MAX_ROWS {
        return Err(ImportError::Corrupt(format!("the sheet has {data_rows} data rows; the limit is {}", crate::DATA_MERGE_MAX_ROWS)));
    }
    let mut out = Vec::with_capacity(data_rows.saturating_add(1));
    for (r, cols) in &by_row {
        let mut row = vec![String::new(); width];
        let mut extra: Vec<String> = Vec::new();
        for (c, v) in cols {
            if *c < min_c {
                // Outside the header span. Keep one non-empty value so the caller can warn.
                if !v.is_empty() {
                    extra.push(v.clone());
                }
                continue;
            }
            let at = c - min_c;
            if at < width {
                if let Some(cell) = row.get_mut(at) {
                    *cell = v.clone();
                }
            } else if !v.is_empty() {
                extra.push(v.clone());
            }
        }
        if *r != header_row && row.iter().all(|c| c.is_empty()) && extra.is_empty() {
            continue;
        }
        row.extend(extra);
        out.push(row);
    }
    Ok(out)
}

/// Workbook order, or the sheet named `wanted`.
fn sheet_target(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, wanted: Option<&str>) -> Result<String, ImportError> {
    let wb = parse(&part(zip, "xl/workbook.xml")?.ok_or_else(|| ImportError::Corrupt("no xl/workbook.xml (not an Excel workbook)".into()))?)?;
    let sheets: Vec<(String, String)> = wb
        .child("sheets")
        .map(|s| s.els().filter(|e| e.name == "sheet").filter_map(|e| Some((e.attr("name")?.to_string(), e.attr("id")?.to_string()))).collect())
        .unwrap_or_default();
    let (name, rid) = match wanted {
        None => sheets.first().cloned().ok_or_else(|| ImportError::Corrupt("the workbook has no sheets".into()))?,
        Some(w) => sheets.into_iter().find(|(n, _)| n == w).ok_or_else(|| ImportError::Corrupt(format!("no worksheet \"{w}\"")))?,
    };
    let rels = parse(&part(zip, "xl/_rels/workbook.xml.rels")?.ok_or_else(|| ImportError::Corrupt("no workbook relationships".into()))?)?;
    let target = rels
        .els()
        .find(|r| r.attr("Id") == Some(rid.as_str()))
        .and_then(|r| r.attr("Target"))
        .ok_or_else(|| ImportError::Corrupt(format!("worksheet \"{name}\" has no file")))?;
    let target = target.trim_start_matches('/');
    Ok(if target.starts_with("xl/") { target.to_string() } else { format!("xl/{target}") })
}

pub fn import_with(bytes: &[u8], opts: &XlsxOptions) -> Result<Imported, ImportError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Corrupt(e.to_string()))?;
    let sheet_part = match opts.sheet.as_deref() {
        None => first_sheet(&mut zip)?.unwrap_or_else(|| "xl/worksheets/sheet1.xml".into()),
        Some(name) => {
            let names = names(&mut zip)?;
            if !names.iter().any(|n| n == name) {
                return Err(ImportError::Options(format!("the workbook has no sheet “{name}” (its sheets: {})", names.join(", "))));
            }
            sheet_target(&mut zip, Some(name))?
        }
    };
    let range = match opts.range.as_deref() {
        None => None,
        Some(r) => Some(parse_range(r).ok_or_else(|| ImportError::Options(format!("“{r}” isn't a cell range like B3:F40")))?),
    };
    let inside = |(r, c): (usize, usize)| range.is_none_or(|((r0, c0), (r1, c1))| (r0..=r1).contains(&r) && (c0..=c1).contains(&c));
    let formats = number_formats(&mut zip)?;
    let date1904 = date1904(&mut zip)?;
    // Number formats that can't be shown exactly, and ones that fill the cell's width: their cells.
    let mut unshown: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut filled: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let sheet = parse(&part(&mut zip, &sheet_part)?.ok_or_else(|| ImportError::Corrupt(format!("no {sheet_part} (not an Excel workbook)")))?)?;
    let shared: Vec<String> = part(&mut zip, "xl/sharedStrings.xml")?
        .map(|x| parse(&x))
        .transpose()?
        .map(|sst| sst.els().filter(|e| e.name == "si").map(El::text).collect())
        .unwrap_or_default();
    let mut cells: Vec<((usize, usize), String)> = Vec::new();
    // What Excel would show as a problem, by sheet position (made table positions below).
    let mut problems: Vec<((usize, usize), Problem)> = Vec::new();
    if let Some(data) = sheet.child("sheetData") {
        for (ri, row) in data.els().filter(|e| e.name == "row").enumerate() {
            for (ci, c) in row.els().filter(|e| e.name == "c").enumerate() {
                let at = c.attr("r").and_then(cell_ref).unwrap_or((ri, ci));
                if !inside(at) {
                    continue;
                }
                let mut v = value(c, &shared);
                let formula = c.child("f").map(El::text).filter(|f| !f.trim().is_empty());
                let address = c.attr("r").map_or_else(|| a1(at), str::to_string);
                // A number as its format shows it.
                let format = c.attr("s").and_then(|s| s.trim().parse::<usize>().ok()).and_then(|i| formats.get(i));
                let number = c.child("v").and_then(|x| x.text().trim().parse::<f64>().ok());
                if let (None | Some("n"), Some((id, code)), Some(n)) = (c.attr("t"), format, number) {
                    match code.as_deref() {
                        Some(code) if code.eq_ignore_ascii_case("general") || code == "@" => {}
                        Some(code) => match numfmt::format(n, code, date1904) {
                            Some(shown) => {
                                if shown.padded {
                                    filled.entry(format!("number format “{code}”")).or_default().push(address.clone());
                                }
                                v = shown.text;
                            }
                            None => unshown.entry(format!("number format “{code}”")).or_default().push(address.clone()),
                        },
                        None => unshown.entry(format!("built-in number format {id}")).or_default().push(address.clone()),
                    }
                }
                let problem =
                    |kind, shows: &str| Problem { kind, at: address.clone(), cell: None, shows: shows.to_string(), formula: formula.clone() };
                if c.attr("t") == Some("e") {
                    problems.push((at, problem(ProblemKind::ErrorValue, &v)));
                } else if let Some(f) = &formula {
                    if other_workbook(f) {
                        problems.push((at, problem(ProblemKind::OtherWorkbook, &v)));
                    }
                    if c.child("v").is_none() {
                        problems.push((at, problem(ProblemKind::NotCalculated, &v)));
                    }
                }
                if !v.is_empty() {
                    cells.push((at, v));
                }
            }
        }
    }
    let mut warnings: Vec<String> = unshown
        .iter()
        .map(|(what, at)| format!("{}: Excel's {what} can't be shown exactly here, so the number comes in unformatted", cell_list(at)))
        .chain(filled.iter().map(|(what, at)| format!("{}: Excel's {what} fills the cell's width; placed without the fill", cell_list(at))))
        .collect();
    if cells.is_empty() {
        warnings.push(match opts.range.as_deref() {
            Some(r) => format!("the range {r} is empty"),
            None => "the worksheet is empty".into(),
        });
    }
    // The used range, from the first used row/column. Cell references come from the file: the
    // table is capped before anything is allocated for it.
    const MAX_CELLS: usize = 100_000;
    const MAX_COLS: usize = 1_000;
    // A range names its cells, empty ones too; otherwise the used range.
    let ((r0, c0), (r1, c1)) = range.unwrap_or_else(|| {
        let first = cells.iter().fold((usize::MAX, usize::MAX), |(r, c), ((a, b), _)| (r.min(*a), c.min(*b)));
        let last = cells.iter().fold((0, 0), |(r, c), ((a, b), _)| (r.max(*a), c.max(*b)));
        (first, last)
    });
    let (nrows, ncols) = if cells.is_empty() && range.is_none() { (1, 1) } else { ((r1 - r0).saturating_add(1), (c1 - c0).saturating_add(1)) };
    let ncols_kept = ncols.min(MAX_COLS);
    if ncols_kept < ncols {
        warnings.push(format!("only the first {ncols_kept} columns were placed"));
    }
    let nrows_kept = nrows.min((MAX_CELLS / ncols_kept).max(1));
    if nrows_kept < nrows {
        warnings.push(format!("only the first {nrows_kept} rows were placed"));
    }
    let mut rows: Vec<Vec<String>> = vec![vec![String::new(); ncols_kept]; nrows_kept];
    for ((r, c), v) in cells {
        if let Some(cell) = rows.get_mut(r - r0).and_then(|row| row.get_mut(c - c0)) {
            *cell = v;
        }
    }
    // A problem's table cell, when the table keeps it.
    let problems = problems
        .into_iter()
        .map(|((r, c), p)| {
            let cell = r.checked_sub(r0).zip(c.checked_sub(c0)).filter(|(r, c)| *r < nrows_kept && *c < ncols_kept);
            Problem { cell, ..p }
        })
        .collect();
    let mut story = Story::new(StoryId(0));
    let width = 72.0 * ncols_kept as f64;
    let mut t = Table::from_strings(1, &rows, width, &ParaFormat::default(), &CharFormat::default());
    // At least one body row stays.
    let header = opts.header_rows.min(t.nrows().saturating_sub(1));
    if header < opts.header_rows {
        warnings.push(format!("{} header rows asked for, {header} kept: a table needs a body row", opts.header_rows));
    }
    t.set_header_footer(header, 0);
    story.insert_table(0, t);
    Ok(Imported { story, para_styles: vec![], char_styles: vec![], warnings, problems })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::ProblemKind;

    fn import(bytes: &[u8]) -> Result<Imported, ImportError> {
        import_with(bytes, &XlsxOptions::default())
    }

    fn workbook() -> Vec<u8> {
        workbook_with(
            r#"<worksheet><sheetData><row r="2"><c r="B2" t="s"><v>0</v></c><c r="C2" t="s"><v>1</v></c></row><row r="3"><c r="B3" t="s"><v>2</v></c><c r="C3"><v>3.5</v></c></row><row r="4"><c r="B4" t="inlineStr"><is><t>Cake</t></is></c><c r="C4"><v>12.0</v></c></row></sheetData></worksheet>"#,
        )
    }

    fn workbook_with(sheet: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                (
                    "xl/workbook.xml",
                    r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Prices" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                ("xl/sharedStrings.xml", r#"<sst><si><t>Item</t></si><si><t>Price</t></si><si><r><t>Te</t></r><r><t>a</t></r></si></sst>"#),
                ("xl/worksheets/sheet1.xml", sheet),
            ] {
                z.start_file(name, o).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    /// Error values, formulas taking their value from another workbook and formulas never
    /// calculated are reported with their cell; a structured table reference is not another
    /// workbook.
    #[test]
    fn error_cells_and_risky_formulas_are_reported() {
        let wb = workbook_with(
            r#"<worksheet><sheetData>
            <row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Total</t></is></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>Tea</t></is></c><c r="B2" t="e"><f>SUM(Z9:Z99)/0</f><v>#DIV/0!</v></c></row>
            <row r="3"><c r="A3" t="inlineStr"><is><t>Cake</t></is></c><c r="B3"><f>[1]Prices!B4</f><v>12</v></c></row>
            <row r="4"><c r="A4" t="inlineStr"><is><t>Jam</t></is></c><c r="B4"><f>SUM(Table1[Amount])</f><v>7</v></c></row>
            <row r="5"><c r="A5" t="inlineStr"><is><t>Bread</t></is></c><c r="B5"><f>A1*2</f></c></row>
            </sheetData></worksheet>"#,
        );
        let imp = import(&wb).unwrap();
        let found: Vec<(ProblemKind, &str, Option<(usize, usize)>, Option<&str>)> =
            imp.problems.iter().map(|p| (p.kind, p.at.as_str(), p.cell, p.formula.as_deref())).collect();
        assert_eq!(
            found,
            [
                (ProblemKind::ErrorValue, "B2", Some((1, 1)), Some("SUM(Z9:Z99)/0")),
                (ProblemKind::OtherWorkbook, "B3", Some((2, 1)), Some("[1]Prices!B4")),
                (ProblemKind::NotCalculated, "B5", Some((4, 1)), Some("A1*2")),
            ]
        );
        assert_eq!(imp.problems[0].shows, "#DIV/0!");
        let t = imp.story.tables.values().next().unwrap();
        assert_eq!(t.cell(1, 1).unwrap().text.text, "#DIV/0!", "the cell still shows what Excel shows");
        assert_eq!(imp.problems[0].message(), "B2: #DIV/0! (formula =SUM(Z9:Z99)/0)");
    }

    #[test]
    fn first_sheet_used_range_becomes_a_table() {
        let imp = import(&workbook()).unwrap();
        let t = imp.story.tables.values().next().expect("a table");
        assert_eq!((t.nrows(), t.ncols()), (3, 2), "the used range B2:C4");
        let cell = |r, c| t.cell(r, c).unwrap().text.text.clone();
        assert_eq!([cell(0, 0), cell(0, 1), cell(1, 0), cell(1, 1), cell(2, 0), cell(2, 1)], ["Item", "Price", "Tea", "3.5", "Cake", "12"]);
        assert_eq!(cell_ref("AA10"), Some((9, 26)));
    }

    /// A cell far away from the others used to allocate the whole used range (billions of cells)
    /// before the cell cap applied; a long column name overflowed the column number.
    #[test]
    fn far_away_cell_references_are_capped() {
        let wb = workbook_with(
            r#"<worksheet><sheetData><row><c r="A1" t="inlineStr"><is><t>a</t></is></c><c r="A999999999" t="inlineStr"><is><t>z</t></is></c><c r="XFDXFDXFDXFDXFDXFD1" t="inlineStr"><is><t>far</t></is></c></row></sheetData></worksheet>"#,
        );
        let imp = import(&wb).unwrap();
        let t = imp.story.tables.values().next().expect("a table");
        assert!(t.nrows() * t.ncols() <= 100_000);
        assert!(!imp.warnings.is_empty());
        assert_eq!(cell_ref("XFDXFDXFDXFDXFDXFD1"), None);
    }

    fn sheets(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in parts {
                z.start_file(*name, o).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    /// A summary sheet first, then a price list under a title row, with an error inside B2:D4 and
    /// one outside it.
    fn two_sheets() -> Vec<u8> {
        let s = |r: &str, t: &str| format!(r#"<c r="{r}" t="inlineStr"><is><t>{t}</t></is></c>"#);
        let prices = format!(
            r#"<worksheet><sheetData><row r="1">{}</row><row r="2">{}{}{}</row><row r="3">{}<c r="C3"><v>3.5</v></c></row><row r="4">{}<c r="C4" t="e"><f>C9/0</f><v>#DIV/0!</v></c>{}</row><row r="5">{}<c r="E5" t="e"><v>#REF!</v></c></row></sheetData></worksheet>"#,
            s("A1", "Price list"),
            s("B2", "Item"),
            s("C2", "Price"),
            s("D2", "Note"),
            s("B3", "Tea"),
            s("B4", "Cake"),
            s("D4", "new"),
            s("B5", "Jam"),
        );
        sheets(&[
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Summary" sheetId="1" r:id="rId1"/><sheet name="Prices" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Type="x" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="x" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Summary</t></is></c></row></sheetData></worksheet>"#,
            ),
            ("xl/worksheets/sheet2.xml", &prices),
        ])
    }

    /// Place with Flow's workbook options: a named sheet and a cell range bring in exactly those
    /// cells (empty ones too), header rows become the table's header rows, and only the range's
    /// errors are reported.
    #[test]
    fn a_named_sheet_and_range_import_only_those_cells() {
        let opts = |range: &str, header_rows| XlsxOptions { sheet: Some("Prices".into()), range: Some(range.into()), header_rows };
        let imp = import_with(&two_sheets(), &opts("B2:D4", 1)).unwrap();
        let t = imp.story.tables.values().next().unwrap();
        let rows: Vec<Vec<String>> = (0..t.nrows()).map(|r| (0..t.ncols()).map(|c| t.cell(r, c).unwrap().text.text.clone()).collect()).collect();
        assert_eq!(rows, [["Item", "Price", "Note"], ["Tea", "3.5", ""], ["Cake", "#DIV/0!", "new"]]);
        assert_eq!(t.header_rows(), 1);
        let found: Vec<(&str, Option<(usize, usize)>)> = imp.problems.iter().map(|p| (p.at.as_str(), p.cell)).collect();
        assert_eq!(found, [("C4", Some((2, 1)))], "E5 is outside the range");
        // Absolute references, lower case and reversed corners name the same cells.
        let same = import_with(&two_sheets(), &opts("$d$4:$B$2", 1)).unwrap();
        assert_eq!(same.story.tables.values().next().unwrap().cell(2, 2).unwrap().text.text, "new");
        // At least one body row stays.
        let all = import_with(&two_sheets(), &opts("B2:D4", 9)).unwrap();
        assert_eq!(all.story.tables.values().next().unwrap().header_rows(), 2);
        assert!(all.warnings.iter().any(|w| w.contains("header rows")), "{:?}", all.warnings);
        assert_eq!(sheet_names(&two_sheets()).unwrap(), ["Summary", "Prices"]);
        // Without options: the first sheet's used range, as before.
        assert_eq!(import(&two_sheets()).unwrap().story.tables.values().next().unwrap().cell(0, 0).unwrap().text.text, "Summary");
    }

    /// Cells import as Excel shows them, in their number formats (built-in and the workbook's
    /// own), dates counted from 1904 when the workbook says so; a format that can't be shown
    /// exactly is a warning naming its cells.
    #[test]
    fn numbers_import_in_their_number_formats() {
        let styles = r##"<styleSheet><numFmts><numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0.00"/><numFmt numFmtId="165" formatCode="# ?/?"/><numFmt numFmtId="166" formatCode="_(* #,##0_)"/></numFmts><cellXfs><xf numFmtId="0"/><xf numFmtId="4"/><xf numFmtId="10"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="165"/><xf numFmtId="166"/></cellXfs></styleSheet>"##;
        let book = |workbook_pr: &str, date: &str| {
            let cells = format!(
                r#"<c r="A1" s="1"><v>1234.5</v></c><c r="B1" s="2"><v>0.0425</v></c><c r="C1" s="3"><v>{date}</v></c><c r="D1" s="4"><v>-12</v></c><c r="E1" s="5"><v>1.5</v></c><c r="F1" s="6"><v>1234</v></c><c r="G1"><v>12.0</v></c><c r="H1" s="1" t="inlineStr"><is><t>note</t></is></c>"#
            );
            sheets(&[
                (
                    "xl/workbook.xml",
                    &format!(
                        r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">{workbook_pr}<sheets><sheet name="Prices" sheetId="1" r:id="rId1"/></sheets></workbook>"#
                    ),
                ),
                ("xl/_rels/workbook.xml.rels", r#"<Relationships><Relationship Id="rId1" Type="x" Target="worksheets/sheet1.xml"/></Relationships>"#),
                ("xl/styles.xml", styles),
                ("xl/worksheets/sheet1.xml", &format!("<worksheet><sheetData><row r=\"1\">{cells}</row></sheetData></worksheet>")),
            ])
        };
        let shown = |imp: &Imported| {
            let t = imp.story.tables.values().next().unwrap();
            (0..t.ncols()).map(|c| t.cell(0, c).unwrap().text.text.clone()).collect::<Vec<_>>()
        };
        let imp = import(&book("", "44927")).unwrap();
        assert_eq!(shown(&imp), ["1,234.50", "4.25%", "1/1/2023", "-$12.00", "1.5", " 1,234 ", "12", "note"]);
        assert!(imp.warnings.iter().any(|w| w.contains("E1") && w.contains("# ?/?")), "{:?}", imp.warnings);
        assert!(imp.warnings.iter().any(|w| w.contains("F1") && w.contains("width")), "{:?}", imp.warnings);
        let imp = import(&book(r#"<workbookPr date1904="1"/>"#, "43465")).unwrap();
        assert_eq!(shown(&imp)[2], "1/1/2023", "1904 dates");
    }

    #[test]
    fn a_bad_range_or_a_missing_sheet_is_an_error() {
        let with = |sheet: &str, range: &str| {
            import_with(&two_sheets(), &XlsxOptions { sheet: Some(sheet.into()), range: Some(range.into()), header_rows: 0 })
                .err()
                .map(|e| e.to_string())
        };
        assert!(with("Prices", "B2-D4").is_some_and(|e| e.contains("B2-D4")));
        assert!(with("Prices", "B0:D4").is_some());
        assert!(with("Nope", "B2:D4").is_some_and(|e| e.contains("Nope")));
    }

    #[test]
    fn records_keep_empty_cells_cached_values_and_a_named_sheet() {
        let first = workbook();
        let rows = records(&first, None).unwrap();
        assert_eq!(rows[0], vec!["Item".to_string(), "Price".to_string()]);
        assert_eq!(rows[1], vec!["Tea".to_string(), "3.5".to_string()]);
        assert_eq!(rows[2][1], "12");
        let book = sheets(&[
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Ignore" sheetId="1" r:id="rId1"/><sheet name="People" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Type="x" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="x" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Nope</t></is></c></row></sheetData></worksheet>"#,
            ),
            (
                "xl/worksheets/sheet2.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Name</t></is></c><c r="B1" t="inlineStr"><is><t>Qty</t></is></c><c r="C1" t="inlineStr"><is><t>On</t></is></c><c r="D1" t="inlineStr"><is><t>When</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Ada</t></is></c><c r="B2"><v>12.0</v></c><c r="C2" t="b"><v>1</v></c><c r="D2"><v>44927</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>Grace</t></is></c><c r="C3"><f>1+1</f></c></row><row r="4"><c r="B4"><v>1</v></c></row></sheetData></worksheet>"#,
            ),
        ]);
        let rows = records(&book, Some("People")).unwrap();
        assert_eq!(rows[0], vec!["Name".to_string(), "Qty".to_string(), "On".to_string(), "When".to_string()]);
        assert_eq!(rows[1], vec!["Ada".to_string(), "12".to_string(), "TRUE".to_string(), "44927".to_string()]);
        // Formula with no cached value is empty. The short row is padded. The all-empty gap is skipped
        // only when every cell is empty; row 4 has a Qty, so it stays, and Name is empty.
        assert_eq!(rows[2], vec!["Grace".to_string(), String::new(), String::new(), String::new()]);
        assert_eq!(rows[3][0], "");
        assert_eq!(rows[3][1], "1");
        assert!(records(&book, Some("Missing")).is_err());
    }

    fn one_value(cell: &str) -> Vec<u8> {
        workbook_with(&format!(
            r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Value</t></is></c></row><row r="2">{cell}</row></sheetData></worksheet>"#
        ))
    }

    #[test]
    fn numeric_values_keep_their_precision_in_tables_and_records() {
        for text in ["1e-12", "-0.0000000000123", "1.23456789012345", "1e300", "12.0", "-2.5e-300"] {
            let book = one_value(&format!(r#"<c r="A2"><v>{text}</v></c>"#));
            let placed = crate::import("values.xlsx", &book).unwrap();
            let table = placed.story.tables.values().next().unwrap();
            let rows = crate::xlsx_records(&book, None).unwrap();
            for got in [&table.cell(1, 0).unwrap().text.text, &rows[1][0]] {
                assert_eq!(got.parse::<f64>().unwrap(), text.parse::<f64>().unwrap(), "cell {text} became {got}");
            }
        }
    }

    #[test]
    fn numeric_values_show_as_excel_general_format() {
        for (text, shown) in [
            ("0.30000000000000004", "0.3"),
            ("2.0000000000000004", "2"),
            ("12.0", "12"),
            ("-0.5", "-0.5"),
            ("0", "0"),
            ("123456789012345", "123456789012345"),
            ("1e15", "1E+15"),
            ("1e300", "1E+300"),
            ("1e-12", "1E-12"),
            ("-1.5e-10", "-1.5E-10"),
            ("inf", "inf"),
        ] {
            let book = one_value(&format!(r#"<c r="A2"><v>{text}</v></c>"#));
            let rows = crate::xlsx_records(&book, None).unwrap();
            assert_eq!(rows[1][0], shown, "cell {text}");
        }
    }

    #[test]
    fn cached_string_values_are_not_reformatted_as_numbers() {
        for text in ["00123", "1e3", "12.0", " 42 "] {
            let book = one_value(&format!(r#"<c r="A2" t="str"><f>"{text}"</f><v>{text}</v></c>"#));
            let placed = crate::import("values.xlsx", &book).unwrap();
            let table = placed.story.tables.values().next().unwrap();
            let rows = crate::xlsx_records(&book, None).unwrap();
            assert_eq!(table.cell(1, 0).unwrap().text.text, text);
            assert_eq!(rows[1][0], text);
        }
    }
}
