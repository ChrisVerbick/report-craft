//! Excel number formats (ECMA-376 Part 1 §18.8.30–31): a cell's number as Excel shows it with US
//! regional settings (period decimal point, comma thousands separator, English month and day
//! names, month/day/year short dates). [`format`] gives `None` for what it can't render exactly
//! (fractions, conditions, other calendars, sub-second times); the importer then keeps the
//! General text and warns, naming the cells.

/// What [`format`] shows, and whether the format pads it to the column's width (`*`), which a
/// placed table has no use for: the text leaves the padding out.
#[derive(Debug, PartialEq)]
pub struct Shown {
    pub text: String,
    pub padded: bool,
}

/// The code of built-in number format `id` as Excel shows it with US regional settings; `None`
/// for the East Asian ones (27–36, 50–81) and ids no format has.
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"$\"#,##0_);\\(\"$\"#,##0\\)",
        6 => "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)",
        7 => "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)",
        8 => "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => r#"_(* #,##0_);_(* \(#,##0\);_(* "-"_);_(@_)"#,
        42 => r#"_("$"* #,##0_);_("$"* \(#,##0\);_("$"* "-"_);_(@_)"#,
        43 => r#"_(* #,##0.00_);_(* \(#,##0.00\);_(* "-"??_);_(@_)"#,
        44 => r#"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"#,
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// A number as Excel's General format shows it: at most 15 significant digits (Excel stores
/// doubles with 17, so `=0.1+0.2` is cached as 0.30000000000000004 and shown as 0.3), whole
/// numbers without ".0", and an exponent for very large or very small values.
pub fn general(f: f64) -> String {
    let r = format!("{f:.14e}").parse::<f64>().unwrap_or(f);
    let a = r.abs();
    if r == 0.0 {
        "0".into()
    } else if (1e-9..1e15).contains(&a) {
        // Shortest text that reads back as the rounded value; whole numbers drop the ".0".
        r.to_string()
    } else {
        let s = format!("{r:E}");
        match s.split_once('E') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}E+{e}"),
            _ => s,
        }
    }
}

/// Number `v` in format `code`, or `None` when this can't show it exactly as Excel does.
/// `date1904`: the workbook counts dates from 1904 (`workbookPr date1904`).
pub fn format(v: f64, code: &str, date1904: bool) -> Option<Shown> {
    if !v.is_finite() {
        return None;
    }
    let sections = split(code)?;
    let pick = |i: usize| sections.get(i).map(String::as_str);
    // One section: negatives get a minus sign; two: the second is for negatives, signless; three:
    // the third is for zero (a fourth is for text cells).
    let (section, minus) = match sections.len() {
        1 => (pick(0)?, v < 0.0),
        2 => (if v < 0.0 { pick(1)? } else { pick(0)? }, false),
        _ if v < 0.0 => (pick(1)?, false),
        _ if v == 0.0 => (pick(2)?, false),
        _ => (pick(0)?, false),
    };
    let mut padded = false;
    let mut english = true;
    let toks = tokens(section, &mut padded, &mut english)?;
    let toks = minutes(toks);
    let dates = toks.iter().any(|t| matches!(t, Tok::Date(..) | Tok::Elapsed(..) | Tok::AmPm(..)));
    let text = if dates {
        // Excel shows a negative date or time as #####.
        if v < 0.0 {
            return None;
        }
        date(v, &toks, date1904, english)?
    } else {
        let n = number(v.abs(), &toks)?;
        if minus { format!("-{n}") } else { n }
    };
    Some(Shown { text, padded })
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Lit(String),
    /// `0`, `#` or `?`.
    Digit(char),
    Point,
    Comma,
    Percent,
    /// `E+` (true) or `E-`, with the letter as written.
    Exp(char, bool),
    General,
    Slash,
    /// `y`, `m` (month), `M` (minute), `d`, `h`, `s`, with how many letters.
    Date(char, usize),
    /// `[h]`, `[mm]`, `[ss]`: hours, minutes or seconds in all.
    Elapsed(char, usize),
    AmPm(String, String),
}

/// `code`'s sections (split at `;` outside quotes, escapes and brackets), at most four.
fn split(code: &str) -> Option<Vec<String>> {
    let mut out = vec![String::new()];
    let mut cs = code.chars();
    while let Some(c) = cs.next() {
        let cur = out.last_mut()?;
        match c {
            ';' => out.push(String::new()),
            '"' => {
                cur.push(c);
                for q in cs.by_ref() {
                    cur.push(q);
                    if q == '"' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                cur.push(c);
                cur.push(cs.next()?);
            }
            '[' => {
                cur.push(c);
                for q in cs.by_ref() {
                    cur.push(q);
                    if q == ']' {
                        break;
                    }
                }
            }
            c => cur.push(c),
        }
    }
    (out.len() <= 4).then_some(out)
}

/// A section's tokens. Sets `padded` for a fill (`*`) and clears `english` for a locale whose
/// month and day names aren't English; `None` for what this doesn't render.
fn tokens(section: &str, padded: &mut bool, english: &mut bool) -> Option<Vec<Tok>> {
    let cs: Vec<char> = section.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let rest = |i: usize, n: usize| cs.get(i..(i + n).min(cs.len())).map(|s| s.iter().collect::<String>()).unwrap_or_default();
    while let Some(&c) = cs.get(i) {
        match c {
            '"' => {
                let end = cs.get(i + 1..)?.iter().position(|&q| q == '"')?;
                out.push(Tok::Lit(cs.get(i + 1..i + 1 + end)?.iter().collect()));
                i += end + 2;
            }
            '\\' => {
                out.push(Tok::Lit(cs.get(i + 1)?.to_string()));
                i += 2;
            }
            // Space as wide as the next character: Excel's text has one space there.
            '_' => {
                cs.get(i + 1)?;
                out.push(Tok::Lit(" ".into()));
                i += 2;
            }
            // Repeat the next character to the column's width.
            '*' => {
                cs.get(i + 1)?;
                *padded = true;
                i += 2;
            }
            '[' => {
                let end = cs.get(i + 1..)?.iter().position(|&q| q == ']')?;
                let inner: String = cs.get(i + 1..i + 1 + end)?.iter().collect();
                bracket(&inner, &mut out, english)?;
                i += end + 2;
            }
            '0' | '#' | '?' => {
                out.push(Tok::Digit(c));
                i += 1;
            }
            '.' => {
                out.push(Tok::Point);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            'E' | 'e' if matches!(cs.get(i + 1), Some('+' | '-')) => {
                out.push(Tok::Exp(c, cs.get(i + 1) == Some(&'+')));
                i += 2;
            }
            'G' | 'g' if rest(i, 7).eq_ignore_ascii_case("general") => {
                out.push(Tok::General);
                i += 7;
            }
            'A' | 'a' if rest(i, 5).eq_ignore_ascii_case("am/pm") => {
                out.push(Tok::AmPm(rest(i, 2), rest(i + 3, 2)));
                i += 5;
            }
            'A' | 'a' if rest(i, 3).eq_ignore_ascii_case("a/p") => {
                out.push(Tok::AmPm(rest(i, 1), rest(i + 2, 1)));
                i += 3;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let l = c.to_ascii_lowercase();
                let n = cs.get(i..)?.iter().take_while(|x| x.to_ascii_lowercase() == l).count();
                out.push(Tok::Date(l, n));
                i += n;
            }
            ' ' | '$' | '-' | '+' | '(' | ')' | ':' | '^' | '\'' | '{' | '}' | '<' | '>' | '=' | '!' | '&' | '~' => {
                out.push(Tok::Lit(c.to_string()));
                i += 1;
            }
            // `@` (text), `B` (Buddhist years), `e` (era years), `g` (era names) and other letters.
            _ => return None,
        }
    }
    Some(out)
}

/// A `[…]` part: a colour (ignored), elapsed time, or a currency and locale (`[$€-407]`).
/// Conditions (`[>=100]`) aren't rendered.
fn bracket(inner: &str, out: &mut Vec<Tok>, english: &mut bool) -> Option<()> {
    let lower = inner.to_ascii_lowercase();
    const COLOURS: [&str; 8] = ["black", "blue", "cyan", "green", "magenta", "red", "white", "yellow"];
    if COLOURS.contains(&lower.as_str()) || lower.strip_prefix("color").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
        return Some(());
    }
    if let Some(l) = lower.chars().next().filter(|l| matches!(l, 'h' | 'm' | 's') && lower.chars().all(|x| x == *l)) {
        out.push(Tok::Elapsed(l, lower.len()));
        return Some(());
    }
    let currency = inner.strip_prefix('$')?;
    let (symbol, locale) = currency.split_once('-').unwrap_or((currency, ""));
    if !symbol.is_empty() {
        out.push(Tok::Lit(symbol.to_string()));
    }
    if !locale.is_empty() {
        let id = u32::from_str_radix(locale, 16).ok()?;
        // Calendars and digit shapes (high bits), and the system's long date and time formats.
        if id > 0xFFFF || id == 0xF800 || id == 0xF400 {
            return None;
        }
        // English is language 0x09 in the low ten bits.
        *english &= id == 0 || id & 0x3FF == 0x09;
    }
    Some(())
}

/// `m`/`mm` after hours or before seconds are minutes.
fn minutes(mut toks: Vec<Tok>) -> Vec<Tok> {
    let timed = |t: Option<&Tok>, l: char| matches!(t, Some(Tok::Date(c, _) | Tok::Elapsed(c, _)) if *c == l);
    let part = |t: &&Tok| !matches!(t, Tok::Lit(_));
    for i in 0..toks.len() {
        let minute = match toks.get(i) {
            Some(Tok::Date('m', n)) if *n <= 2 => {
                let before = toks.get(..i).and_then(|r| r.iter().rev().find(part));
                let after = toks.get(i + 1..).and_then(|r| r.iter().find(part));
                (timed(before, 'h') || timed(after, 's')).then_some(*n)
            }
            _ => None,
        };
        if let (Some(n), Some(t)) = (minute, toks.get_mut(i)) {
            *t = Tok::Date('M', n);
        }
    }
    toks
}

/// |v| in decimal as Excel holds it: 15 significant digits, and the power of ten of the first.
fn decimal(v: f64) -> Option<(Vec<u8>, i32)> {
    let s = format!("{:.14e}", v.abs());
    let (m, e) = s.split_once('e')?;
    Some((m.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect(), e.parse().ok()?))
}

/// |v| rounded to `frac` decimals as Excel rounds for display (its 15 digits, halves up): the
/// whole part's digits ("" for zero) and `frac` decimal digits.
fn fixed(v: f64, frac: usize) -> Option<(String, String)> {
    let (d, exp) = decimal(v)?;
    let frac = i32::try_from(frac).ok()?;
    let digit = |place: i32| -> u8 { usize::try_from(exp - place).ok().and_then(|k| d.get(k)).copied().unwrap_or(0) };
    let top = exp.max(0);
    let mut kept: Vec<u8> = (-frac..=top).rev().map(digit).collect();
    if digit(-frac - 1) >= 5 {
        let mut carry = true;
        for x in kept.iter_mut().rev() {
            if !carry {
                break;
            }
            *x += 1;
            carry = *x == 10;
            if carry {
                *x = 0;
            }
        }
        if carry {
            kept.insert(0, 1);
        }
    }
    let split = kept.len().checked_sub(usize::try_from(frac).ok()?)?;
    let (whole, part) = kept.split_at(split);
    let whole: String = whole.iter().skip_while(|&&x| x == 0).map(|x| char::from(b'0' + x)).collect();
    Some((whole, part.iter().map(|x| char::from(b'0' + x)).collect()))
}

/// A number section (|v|): digits, decimal point, thousands separators and scaling commas,
/// percent, exponent, literals and `General`.
fn number(mut v: f64, toks: &[Tok]) -> Option<String> {
    if toks.iter().any(|t| matches!(t, Tok::Date(..) | Tok::Slash)) {
        return None;
    }
    if toks.contains(&Tok::General) {
        // Literals around General.
        return toks
            .iter()
            .map(|t| match t {
                Tok::General => Some(general(v)),
                Tok::Lit(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
    }
    let digits: Vec<usize> = toks.iter().enumerate().filter(|(_, t)| matches!(t, Tok::Digit(_))).map(|(i, _)| i).collect();
    let (Some(&first), Some(&last)) = (digits.first(), digits.last()) else {
        // Literals only (an accounting zero's "-").
        return toks.iter().map(|t| if let Tok::Lit(s) = t { Some(s.clone()) } else { None }).collect();
    };
    let exp_at = toks.iter().position(|t| matches!(t, Tok::Exp(..)));
    let point = toks.iter().position(|t| *t == Tok::Point).filter(|p| exp_at.is_none_or(|e| *p < e));
    let int_end = point.or(exp_at).unwrap_or(toks.len());
    let mantissa_last = digits.iter().copied().filter(|&i| exp_at.is_none_or(|e| i < e)).max().unwrap_or(last);
    // A comma between whole-part digits groups thousands; one right after the last digit scales by 1,000.
    let mut grouped = false;
    for (i, _) in toks.iter().enumerate().filter(|(_, t)| **t == Tok::Comma) {
        if i > first && i < int_end && digits.iter().any(|&d| d > i && d < int_end) {
            grouped = true;
        } else if i > mantissa_last && exp_at.is_none_or(|e| i < e) {
            v /= 1000.0;
        } else {
            return None;
        }
    }
    for _ in toks.iter().filter(|t| **t == Tok::Percent) {
        v *= 100.0;
    }
    if exp_at.is_some() {
        return scientific(v, toks, point, exp_at?);
    }
    let frac_n = toks.iter().skip(int_end).filter(|t| matches!(t, Tok::Digit(_))).count();
    let (whole, frac) = fixed(v, frac_n)?;
    let mut out = whole_part(&whole, toks.get(..int_end)?, grouped)?;
    if let Some(p) = point {
        out.push('.');
        out.push_str(&fraction(&frac, toks.get(p + 1..)?)?);
    }
    Some(out)
}

/// The whole part's tokens filled right to left with `whole`'s digits; the digits left over go
/// before the first placeholder.
fn whole_part(whole: &str, toks: &[Tok], grouped: bool) -> Option<String> {
    let places: Vec<char> = toks.iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
    if grouped {
        // Grouping with literals between the digits, or `?`, isn't rendered.
        let first = toks.iter().position(|t| matches!(t, Tok::Digit(_)))?;
        let last = toks.iter().rposition(|t| matches!(t, Tok::Digit(_)))?;
        if places.contains(&'?') || toks.get(first..=last)?.iter().any(|t| !matches!(t, Tok::Digit(_) | Tok::Comma)) {
            return None;
        }
        let zeros = places.iter().filter(|&&c| c == '0').count();
        let padded = format!("{whole:0>zeros$}");
        let mut g = String::new();
        for (i, c) in padded.chars().enumerate() {
            if i > 0 && (padded.len() - i) % 3 == 0 {
                g.push(',');
            }
            g.push(c);
        }
        let lits = |r: &[Tok]| {
            r.iter().filter_map(|t| if let Tok::Lit(s) = t { Some(s.as_str()) } else { (*t == Tok::Percent).then_some("%") }).collect::<String>()
        };
        return Some(format!("{}{g}{}", lits(toks.get(..first)?), lits(toks.get(last + 1..)?)));
    }
    let mut ds = whole.chars().rev();
    let first = toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let mut out: Vec<String> = Vec::new();
    for (i, t) in toks.iter().enumerate().rev() {
        match t {
            Tok::Digit(c) => {
                out.push(match ds.next() {
                    Some(d) => d.to_string(),
                    None if *c == '0' => "0".into(),
                    None if *c == '?' => " ".into(),
                    None => String::new(),
                });
                if Some(i) == first {
                    out.push(ds.by_ref().collect::<Vec<char>>().into_iter().rev().collect());
                }
            }
            Tok::Lit(s) => out.push(s.clone()),
            Tok::Percent => out.push("%".into()),
            Tok::Comma => {}
            _ => return None,
        }
    }
    if first.is_none() {
        // No placeholder (".00"): the whole part still shows.
        out.push(whole.to_string());
    }
    Some(out.into_iter().rev().collect())
}

/// The decimals' tokens with `frac`'s digits: `#` and `?` hide trailing zeros (`?` as a space).
fn fraction(frac: &str, toks: &[Tok]) -> Option<String> {
    let ds: Vec<char> = frac.chars().collect();
    let mut k = 0;
    let mut out = String::new();
    for t in toks {
        match t {
            Tok::Digit(c) => {
                let d = *ds.get(k)?;
                let trailing = ds.get(k..)?.iter().all(|&x| x == '0');
                match (c, trailing) {
                    ('0', _) | (_, false) => out.push(d),
                    ('?', true) => out.push(' '),
                    _ => {}
                }
                k += 1;
            }
            Tok::Lit(s) => out.push_str(s),
            Tok::Percent => out.push('%'),
            // Scaling commas after the last digit.
            Tok::Comma => {}
            _ => return None,
        }
    }
    Some(out)
}

/// `0.00E+00`, or engineering notation (`##0.0E+0`: the exponent a multiple of the whole part's
/// placeholders).
fn scientific(v: f64, toks: &[Tok], point: Option<usize>, exp_at: usize) -> Option<String> {
    let int_end = point.unwrap_or(exp_at);
    let int_places: Vec<char> = toks.get(..int_end)?.iter().filter_map(|t| if let Tok::Digit(c) = t { Some(*c) } else { None }).collect();
    let n_i = int_places.len();
    let step = match n_i {
        1 => 1,
        n if int_places.iter().take(n - 1).all(|&c| c == '#') => i32::try_from(n).ok()?,
        _ => return None,
    };
    let frac_n = point.map_or(0, |p| toks.get(p + 1..exp_at).map_or(0, |r| r.iter().filter(|t| matches!(t, Tok::Digit(_))).count()));
    let (_, mut e) = decimal(v)?;
    if v == 0.0 {
        e = 0;
    }
    let mut x = e.div_euclid(step) * step;
    let (mut whole, mut frac) = fixed(v / 10f64.powi(x), frac_n)?;
    // Rounding up can add a digit: 9.99E+00 → 1.00E+01.
    if v != 0.0 && whole.len() > usize::try_from(step).ok()? {
        x += step;
        (whole, frac) = fixed(v / 10f64.powi(x), frac_n)?;
    }
    let Some(Tok::Exp(letter, plus)) = toks.get(exp_at).cloned() else { return None };
    let e_places = toks.get(exp_at + 1..)?.iter().filter(|t| matches!(t, Tok::Digit(_))).count();
    let sign = if x < 0 {
        "-"
    } else if plus {
        "+"
    } else {
        ""
    };
    let mut out = whole_part(&whole, toks.get(..int_end)?, false)?;
    if let Some(p) = point {
        out.push('.');
        out.push_str(&fraction(&frac, toks.get(p + 1..exp_at)?)?);
    }
    let width = e_places.max(1);
    let lits: String = toks.get(exp_at + 1..)?.iter().filter_map(|t| if let Tok::Lit(s) = t { Some(s.as_str()) } else { None }).collect();
    Some(format!("{out}{letter}{sign}{:0>width$}{lits}", x.unsigned_abs()))
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Days since 1970-01-01 → (year, month, day) in the proleptic Gregorian calendar.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A date and time section: `v` days from the workbook's epoch.
fn date(v: f64, toks: &[Tok], date1904: bool, english: bool) -> Option<String> {
    let serial = v.floor();
    let ms = ((v - serial) * 86_400_000.0).round();
    // How Excel rounds a fraction of a second isn't known here.
    if ms % 1000.0 != 0.0 || serial > 2_958_465.0 {
        return None;
    }
    let mut days = serial as i64;
    let mut secs = (ms / 1000.0) as i64;
    if secs >= 86_400 {
        days += 1;
        secs -= 86_400;
    }
    let dated = toks.iter().any(|t| matches!(t, Tok::Date('y' | 'm' | 'd', _)));
    // 1900 dates: serial 1 is 1900-01-01, and Excel counts a 29 February 1900 (serial 60).
    let ((y, mo, d), weekday) = if date1904 {
        let z = days - 24_107;
        (civil(z), (z + 4).rem_euclid(7))
    } else {
        match days {
            0 if dated => return None,
            60 => ((1900, 2, 29), 3),
            1..=59 => (civil(days - 25_568), (days + 6) % 7),
            _ => {
                let z = days - 25_569;
                (civil(z), (z + 4).rem_euclid(7))
            }
        }
    };
    let ampm = toks.iter().any(|t| matches!(t, Tok::AmPm(..)));
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let pad = |n: i64, w: usize| format!("{n:0w$}");
    let name = |names: &[&str], i: usize, len: Option<usize>| -> Option<String> {
        if !english {
            return None;
        }
        let n = names.get(i)?;
        Some(match len {
            Some(l) => n.chars().take(l).collect(),
            None => n.to_string(),
        })
    };
    let mut out = String::new();
    let mut iter = toks.iter().peekable();
    while let Some(t) = iter.next() {
        match t {
            Tok::Lit(x) => out.push_str(x),
            Tok::Comma => out.push(','),
            Tok::Slash => out.push('/'),
            // Fractions of a second aren't rendered.
            Tok::Point if matches!(iter.peek(), Some(Tok::Digit(_))) => return None,
            Tok::Point => out.push('.'),
            Tok::Date('y', n) => out.push_str(&if *n <= 2 { pad(y % 100, 2) } else { pad(y, 4) }),
            Tok::Date('m', 1) => out.push_str(&mo.to_string()),
            Tok::Date('m', 2) => out.push_str(&pad(i64::from(mo), 2)),
            Tok::Date('m', 3) => out.push_str(&name(&MONTHS, mo as usize - 1, Some(3))?),
            Tok::Date('m', 4) => out.push_str(&name(&MONTHS, mo as usize - 1, None)?),
            Tok::Date('m', 5) => out.push_str(&name(&MONTHS, mo as usize - 1, Some(1))?),
            Tok::Date('d', 1) => out.push_str(&d.to_string()),
            Tok::Date('d', 2) => out.push_str(&pad(i64::from(d), 2)),
            Tok::Date('d', 3) => out.push_str(&name(&DAYS, weekday as usize, Some(3))?),
            Tok::Date('d', _) => out.push_str(&name(&DAYS, weekday as usize, None)?),
            Tok::Date('h', n) => {
                let h = if ampm { (h + 11) % 12 + 1 } else { h };
                out.push_str(&if *n == 1 { h.to_string() } else { pad(h, 2) });
            }
            Tok::Date('M', n) => out.push_str(&if *n == 1 { m.to_string() } else { pad(m, 2) }),
            Tok::Date('s', n) => out.push_str(&if *n == 1 { s.to_string() } else { pad(s, 2) }),
            Tok::Elapsed(l, n) => {
                let all = match l {
                    'h' => days * 24 + h,
                    'm' => (days * 24 + h) * 60 + m,
                    _ => ((days * 24 + h) * 60 + m) * 60 + s,
                };
                out.push_str(&pad(all, *n));
            }
            Tok::AmPm(am, pm) => out.push_str(if h < 12 { am } else { pm }),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: f64, code: &str) -> Option<String> {
        format(v, code, false).map(|s| s.text)
    }

    #[test]
    fn decimals_thousands_percent_and_rounding() {
        for (v, code, shown) in [
            (1234.5, "0", "1235"),
            (1234.5, "0.00", "1234.50"),
            (1234567.891, "#,##0", "1,234,568"),
            (1234567.891, "#,##0.00", "1,234,567.89"),
            (0.5, "#,##0.00", "0.50"),
            (0.0, "#,##0", "0"),
            (12.0, "00,000", "00,012"),
            (1.005, "0.00", "1.01"),
            (2.5, "0", "3"),
            (-2.5, "0", "-3"),
            (0.125, "0.00", "0.13"),
            (0.0042, "0.00%", "0.42%"),
            (0.5, "0%", "50%"),
            (1.5, "0.0#", "1.5"),
            (1.25, "0.0#", "1.25"),
            (1.0, "#.##", "1."),
            (0.5, "#.00", ".50"),
            (12.5, ".00", "12.50"),
            (1.5, "0.??", "1.5 "),
            (5.0, "???", "  5"),
            (123456789.0, "#,##0,", "123,457"),
            (1234567890.0, "0.0,,", "1234.6"),
            (999.5, "#,##0", "1,000"),
            (-0.001, "0.00", "-0.00"),
            (123456789.0, "000-00-0000", "123-45-6789"),
            (1e20, "0", "100000000000000000000"),
        ] {
            assert_eq!(f(v, code).as_deref(), Some(shown), "{v} in {code}");
        }
    }

    #[test]
    fn sections_currency_literals_and_colours() {
        let money = builtin(7).unwrap();
        assert_eq!(f(1234.5, money).as_deref(), Some("$1,234.50 "));
        assert_eq!(f(-1234.5, money).as_deref(), Some("($1,234.50)"));
        assert_eq!(f(-3.0, "0;[Red]-0").as_deref(), Some("-3"));
        assert_eq!(f(-3.0, "0;(0)").as_deref(), Some("(3)"));
        assert_eq!(f(0.0, r#"0;-0;"zero""#).as_deref(), Some("zero"));
        assert_eq!(f(0.0, "0;-0;;@").as_deref(), Some(""));
        assert_eq!(f(12.0, r#"0" units""#).as_deref(), Some("12 units"));
        assert_eq!(f(12.0, r#"\$0.00"#).as_deref(), Some("$12.00"));
        assert_eq!(f(12.0, "[$€-407] #,##0.00").as_deref(), Some("€ 12.00"));
        assert_eq!(f(12.0, "[$USD] 0").as_deref(), Some("USD 12"));
        assert_eq!(f(12.5, r#""Total: "General"#).as_deref(), Some("Total: 12.5"));
        assert_eq!(f(12345.0, "0.00E+00").as_deref(), Some("1.23E+04"));
        assert_eq!(f(0.000123, "0.00E+00").as_deref(), Some("1.23E-04"));
        assert_eq!(f(9.999, "0.00E+00").as_deref(), Some("1.00E+01"));
        assert_eq!(f(12345.0, "##0.0E+0").as_deref(), Some("12.3E+3"));
        assert_eq!(f(0.0, "0.00E+00").as_deref(), Some("0.00E+00"));
    }

    #[test]
    fn accounting_formats_leave_their_padding_out() {
        let acc = builtin(44).unwrap();
        assert_eq!(format(1234.5, acc, false), Some(Shown { text: " $1,234.50 ".into(), padded: true }));
        assert_eq!(f(-1234.5, acc).as_deref(), Some(" $(1,234.50)"));
        assert_eq!(f(0.0, acc).as_deref(), Some(" $-   "));
    }

    #[test]
    fn dates_and_times() {
        // 2023-01-01 is serial 44927 (1900 dates), a Sunday.
        for (v, code, shown) in [
            (44927.0, "m/d/yyyy", "1/1/2023"),
            (44927.0, "mm/dd/yy", "01/01/23"),
            (44927.0, "d-mmm-yy", "1-Jan-23"),
            (44927.0, "dddd, mmmm d, yyyy", "Sunday, January 1, 2023"),
            (44927.0, "ddd mmmmm", "Sun J"),
            (44927.75, "h:mm AM/PM", "6:00 PM"),
            (44927.75, "hh:mm:ss", "18:00:00"),
            (44927.0 + 1.0 / 24.0 + 5.0 / 1440.0, "h:mm a/p", "1:05 a"),
            (0.5, "h:mm", "12:00"),
            (1.5, "[h]:mm:ss", "36:00:00"),
            (0.0625, "[mm]:ss", "90:00"),
            (60.0, "yyyy-mm-dd", "1900-02-29"),
            (61.0, "yyyy-mm-dd ddd", "1900-03-01 Thu"),
            (1.0, "yyyy-mm-dd ddd", "1900-01-01 Sun"),
            (44927.0, "[$-409]mmmm d", "January 1"),
        ] {
            assert_eq!(f(v, code).as_deref(), Some(shown), "{v} in {code}");
        }
        assert_eq!(format(0.0, "yyyy-mm-dd", true).map(|s| s.text).as_deref(), Some("1904-01-01"));
        assert_eq!(format(43465.0, "m/d/yyyy", true).map(|s| s.text).as_deref(), Some("1/1/2023"));
    }

    #[test]
    fn what_is_not_rendered_exactly_is_none() {
        for (v, code) in [
            (1.5, "# ?/?"),
            (1500.0, "[>=1000]0.0,\"K\";0"),
            (44927.0, "[$-407]mmmm"),
            (44927.0, "[$-F800]dddd, mmmm dd, yyyy"),
            (0.5000001, "h:mm:ss"),
            (0.5, "mm:ss.0"),
            (-1.0, "m/d/yyyy"),
            (0.0, "m/d/yyyy"),
            (44927.0, "e"),
            (1.0, "\"unterminated"),
        ] {
            assert_eq!(f(v, code), None, "{v} in {code}");
        }
    }
}
