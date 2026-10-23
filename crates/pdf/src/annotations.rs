//! Review comments as the PDF's own markup annotations (Highlight, StrikeOut, Text, Ink), added
//! as an incremental update like the form fields: krilla writes none of them. Coordinates come in
//! as page points from the page's top-left trim corner and go out in PDF space.

use std::fmt::Write as _;

use designcraft_doc::Document;

use crate::export::Sheet;
use crate::forms::pdf_string;
use crate::transitions::{int_after, object, rfind};
use crate::{AnnotationKind, PdfOptions};

/// `secs` since the epoch as a PDF date, `D:YYYYMMDDHHMMSSZ`.
fn pdf_date(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let t = secs.rem_euclid(86_400);
    format!("D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

/// Where page `abs` is in the output: its sheet, and a map from page points (top-left origin)
/// to PDF points on that sheet.
fn place(doc: &Document, sheets: &[Sheet], opts: &PdfOptions, abs: usize) -> Option<(usize, impl Fn(f64, f64) -> (f64, f64))> {
    let (si, pi) = doc.page_loc(abs)?;
    let trim = doc.spreads.get(si)?.pages.get(pi)?.bounds();
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    let k = sheets.iter().position(|sh| sh.spread == si && (opts.spreads || (near(sh.trim.x0, trim.x0) && near(sh.trim.y0, trim.y0))))?;
    let media = sheets.get(k)?.media;
    let h = media.height();
    Some((k, move |x: f64, y: f64| (trim.x0 + x - media.x0, h - (trim.y0 + y - media.y0))))
}

/// Add `opts.annotations` to `pdf` (page indices absolute). `None` when the file's structure
/// isn't the simple kind this understands; annotations on pages not exported are left out.
pub(crate) fn add(pdf: &[u8], doc: &Document, sheets: &[Sheet], opts: &PdfOptions) -> Option<Vec<u8>> {
    let t_at = rfind(pdf, b"trailer")?;
    let trailer = String::from_utf8_lossy(&pdf[t_at..]).to_string();
    let size = int_after(&trailer, "/Size")?;
    let root = int_after(&trailer, "/Root")?;
    let prev = int_after(&trailer, "startxref")?;
    let info = int_after(&trailer, "/Info");
    let id = trailer.find("/ID").and_then(|i| trailer[i..].find(']').map(|j| trailer[i..i + j + 1].to_string()));
    let cat = String::from_utf8_lossy(&pdf[object(pdf, root)?]).to_string();
    let pages_id = int_after(&cat, "/Pages")?;
    let pages = String::from_utf8_lossy(&pdf[object(pdf, pages_id)?]).to_string();
    let kids_str = &pages[pages.find("/Kids")? + 5..];
    let kids_str = &kids_str[kids_str.find('[')? + 1..kids_str.find(']')?];
    let nums: Vec<usize> = kids_str.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    let kids: Vec<usize> = nums.chunks(2).map(|c| c[0]).collect();

    let mut out = pdf.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let mut entries: Vec<(usize, usize)> = Vec::new();
    let mut next = size;
    let mut put = |out: &mut Vec<u8>, id: usize, body: &str| {
        entries.push((id, out.len()));
        out.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
    };
    let mut per_page: Vec<Vec<usize>> = vec![Vec::new(); kids.len()];
    // (comment id → (object id, its /Rect)) so replies can thread to it.
    let mut written: Vec<(String, usize, String)> = Vec::new();
    let fmt_rect = |r: [f64; 4]| format!("[{:.2} {:.2} {:.2} {:.2}]", r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3]));
    // Comments first, then the replies to them.
    let ordered = opts.annotations.iter().filter(|a| a.in_reply_to.is_none()).chain(opts.annotations.iter().filter(|a| a.in_reply_to.is_some()));
    for a in ordered {
        let Some((k, to_pdf)) = place(doc, sheets, opts, a.page) else { continue };
        let Some(&page_ref) = kids.get(k) else { continue };
        let common = format!(
            "/Type/Annot/P {page_ref} 0 R/F 4/T{}/Contents{}/NM{}/M({})",
            pdf_string(&a.author),
            pdf_string(&a.contents),
            pdf_string(&a.id),
            pdf_date(a.modified)
        );
        let parent = a.in_reply_to.as_ref().and_then(|p| written.iter().find(|(cid, _, _)| cid == p).map(|(_, oid, rect)| (*oid, rect.clone())));
        let (body, rect) = match (&a.kind, parent) {
            (_, Some((oid, rect))) => {
                (format!("<<{common}/Subtype/Text/Rect{rect}/IRT {oid} 0 R/RT/R/Name/Comment/Open false/C[0.95 0.5 0.1]>>"), rect)
            }
            (AnnotationKind::Highlight(rects) | AnnotationKind::StrikeOut(rects), None) => {
                if rects.is_empty() {
                    continue;
                }
                let mut quads = String::new();
                let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                for r in rects {
                    let (x1, yt) = to_pdf(r[0], r[1]);
                    let (x2, yb) = to_pdf(r[2], r[3]);
                    let _ = write!(quads, "{x1:.2} {yt:.2} {x2:.2} {yt:.2} {x1:.2} {yb:.2} {x2:.2} {yb:.2} ");
                    bbox = [bbox[0].min(x1), bbox[1].min(yb), bbox[2].max(x2), bbox[3].max(yt)];
                }
                let rect = fmt_rect(bbox);
                let (sub, color) =
                    if matches!(a.kind, AnnotationKind::Highlight(_)) { ("Highlight", "[1 0.85 0.2]") } else { ("StrikeOut", "[0.85 0.2 0.1]") };
                (format!("<<{common}/Subtype/{sub}/Rect{rect}/QuadPoints[{}]/C{color}/CA 0.7>>", quads.trim_end()), rect)
            }
            (AnnotationKind::Text { x, y }, None) => {
                let (px, py) = to_pdf(*x, *y);
                let rect = fmt_rect([px, py - 20.0, px + 20.0, py]);
                (format!("<<{common}/Subtype/Text/Rect{rect}/Name/Comment/Open false/C[0.95 0.5 0.1]>>"), rect)
            }
            (AnnotationKind::Ink(path), None) => {
                if path.len() < 2 {
                    continue;
                }
                let mut ink = String::new();
                let mut bbox = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
                for p in path {
                    let (px, py) = to_pdf(p[0], p[1]);
                    let _ = write!(ink, "{px:.2} {py:.2} ");
                    bbox = [bbox[0].min(px), bbox[1].min(py), bbox[2].max(px), bbox[3].max(py)];
                }
                let rect = fmt_rect([bbox[0] - 2.0, bbox[1] - 2.0, bbox[2] + 2.0, bbox[3] + 2.0]);
                (format!("<<{common}/Subtype/Ink/Rect{rect}/InkList[[{}]]/BS<</W 2>>/C[0.85 0.2 0.1]>>", ink.trim_end()), rect)
            }
        };
        let oid = next;
        next += 1;
        put(&mut out, oid, &body);
        per_page[k].push(oid);
        written.push((a.id.clone(), oid, rect));
    }
    if written.is_empty() {
        return Some(pdf.to_vec());
    }
    // Pages list their annotations.
    for (k, ids) in per_page.iter().enumerate() {
        if ids.is_empty() {
            continue;
        }
        let refs: String = ids.iter().map(|i| format!(" {i} 0 R")).collect();
        let body = String::from_utf8_lossy(&pdf[object(pdf, kids[k])?]).to_string();
        let new = if let Some(a) = body.find("/Annots[") {
            let close = a + body[a..].find(']')?;
            format!("{}{refs}{}", &body[..close], &body[close..])
        } else if let Some(arr_id) = int_after(&body, "/Annots") {
            let arr = String::from_utf8_lossy(&pdf[object(pdf, arr_id)?]).to_string();
            let close = arr.rfind(']')?;
            put(&mut out, arr_id, format!("{}{refs}{}", arr[..close].trim(), &arr[close..]).trim());
            body.clone()
        } else {
            let close = body.rfind(">>")?;
            format!("{}/Annots[{}]{}", &body[..close], refs.trim(), &body[close..])
        };
        if new != body {
            put(&mut out, kids[k], new.trim());
        }
    }
    let xref_at = out.len();
    entries.sort();
    let mut x = String::from("xref\n");
    for (oid, off) in &entries {
        let _ = write!(x, "{oid} 1\n{off:010} 00000 n\r\n");
    }
    let info = info.map(|i| format!("/Info {i} 0 R")).unwrap_or_default();
    let id = id.unwrap_or_default();
    let _ = write!(x, "trailer\n<</Size {next}/Root {root} 0 R{info}{id}/Prev {prev}>>\nstartxref\n{xref_at}\n%%EOF\n");
    out.extend_from_slice(x.as_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pdf_dates_are_utc_calendar_dates() {
        assert_eq!(super::pdf_date(0), "D:19700101000000Z");
        assert_eq!(super::pdf_date(1_700_000_000), "D:20231114221320Z");
    }
}
