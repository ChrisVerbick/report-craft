# DesignCraft roadmap (detail)

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-20 · **Change:** minor (alpha gate passes (threading row re-tested on macOS 2026-10-20): stage alpha; focus item 0 done; A1 ✅; alpha gate: the threading row's issue (#165) is closed, so the row and A1 wait on a re-test; styles row passes (#169, #289 fixed); focus items 0, 1 and 4 and milestones M1–M3, M11 brought up to date; .indd and .indt open through indd-utils; text threads by hand; the gate waits on the Primary Text Frame page fixes) · **Target:** Adobe InDesign 2026 (21.6)

Forward-looking plan. The one-page summary is [ROADMAP.md](../ROADMAP.md); numbers are in
[target-app-parity.md](target-app-parity.md); the work list is [gaps.md](gaps.md). Hours are
Opus 5.5 agent-hours (calibration in target-app-parity.md). Driving the app from agents:
[agents.md](agents.md).

## Alpha gate

InDesign's core workflows: what a layout professional does with it every day. DesignCraft is
alpha only when every one works end to end on macOS (its main platform) and the work saves and
reopens. **Gate result (2026-10-20): passes, so the stage is alpha.** The 2026-10-10 result failed
on the threading row (#165); it passed its re-test on macOS on 2026-10-20, after #165 closed and
upstream #1169 removed the empty pages autoflow left. Rows marked "no" are the
alpha checklist; the partial rows don't block the workflow on macOS (there is a working path)
but are next in line.

| Core workflow | Works end to end? | Evidence | Hours to pass |
|---|---|---|---|
| Set up a document (size, margins, columns, parent pages, page numbers, sections); save and reopen | yes | New Document, Document Setup, parent pages and overrides, Numbering & Section Options; `.designcraft` round-trip tests; recovery | 0 |
| Flow a long story across frames and pages: thread, unthread, autoflow | yes | threading by hand (ports, loaded text cursor, `text.thread`/`text.unthread`/`text.removeFromThread`), autoflow and the Primary Text Frame flow; re-tested 2026-10-20 in the macOS release app through the control channel: #1165's steps give InDesign's 5 pages, deleting pages or a frame adds no pages beyond what the text needs, emptied pages delete and stay deleted, Smart Text Reflow off deletes a page with text, threading by hand, autoflow, save and reopen (12 checks as commands, then the same steps as clicks and key presses: the Pages panel's buttons and warning, Delete on a selected frame, the Type tool's caret, threading and unthreading at the ports); `deleting_pages_or_frames_of_a_primary_story_adds_no_pages` pins #165 | 0 |
| Format text with paragraph and character styles | yes | Paragraph/Character Style Options, Quick Apply, based-on, nested/GREP styles work; style edit/delete from the panel (#169) and formatting at a caret (#289) fixed | 0 |
| Place, crop and fit images; wrap text around them | yes | place gun, frame fitting and Fitting Options, content grabber, text wrap, Links panel | 0 |
| Arrange and edit objects: select, move, transform, duplicate, copy/paste, align, layers | partial (not blocking on macOS) | all present and working on macOS; Paste in Place lands on page 1 (#348); right-click menus fail for some users (#187; #160, #238 closed as duplicates); copy/paste on Windows/Linux: #185 fixed, #164 closed upstream on 2026-10-20, #351 closed as a duplicate | 10–20 |
| Export a print-ready PDF (bleed, marks, CMYK and spot, PDF/X-4) | yes | krilla export with subset fonts, Separations, marks and bleed, PDF/X-4 with validator; 53 PDF tests | 0 |

Exchanging files with InDesign users (open INDD, round-trip IDML) is a beta requirement, not part
of the alpha gate: IDML works both ways today, INDD opens through indd-utils, and measuring both
on real files is gaps 2 and 3.

**Alpha reached on 2026-10-20** (the gate passes). Ready for real work (~47%) is already above
the ~40% alpha bar.

## Current focus

In order. Each closes a beta blocker from [gaps.md](gaps.md).

1. **Basic editing works on every platform** (gap 1): Paste in Place (#348), RTF paste (#212),
   context menus (#187), reproduced and pinned with control-channel tests on macOS, Windows and
   Linux. 15–30 h.
2. **CI on pull requests** (gap 7): `cargo xtask ci` on every PR, at least macOS and Linux;
   Windows smoke start. 4–8 h. Needs the owner to make it a required check.
3. **INDD real-file corpus** (gap 2): `.indd` and `.indt` open through storytold/indd-utils
   (INDD → IDML in memory → existing importer, warnings in the open result); a pinned corpus and
   a metric show what survives. 10–30 h.
4. **IDML real-file corpus** (gap 3): a pinned corpus, a survival metric, and a fix for #193
   (#338, #339 and #340 were fixed on 2026-10-16). 60–100 h.
5. **Composition oracle** (gap 4): synthetic stories set in InDesign 2026 vs DesignCraft;
   optical kerning; the overset rule. 80–140 h.
6. **Start on any GPU; find every installed font** (gaps 5, 6). 25–50 h.

## Milestones

States re-assessed 2026-10-10 against the code and the issue tracker. "Done" means the feature
set exists; depth gaps are in gaps.md.

| # | Milestone | State |
|---|---|---|
| M0 | Skeleton + vertical slice | ✅ |
| M1 | Selection, transform, layers, pages, MCP | ✅ (handle and snapping issues #188, #337 fixed) |
| M2 | Type I (Type tool, threading, Character/Paragraph, composer) | ✅ core: Type tool, composer, threading, the Primary Text Frame flow (re-tested 2026-10-20) and caret formatting; IME not checked by hand (#322) |
| M3 | Styles (nested/GREP, bullets, keeps, span columns) | ✅ (style editing #169 fixed) |
| M4 | Color & effects | ✅ core; colour management depth |
| M5 | Graphics & links | ✅ core; EPS preview only |
| M6 | Files & export (native, IDML, PDF, PNG/JPEG) | IDML ✅, PDF ✅ (X-4, A-2b, tagged); INDD read ✅ (via indd-utils); PDF/X-1a/X-3 and presets ✗ |
| M7 | Long documents (sections, TOC, index, footnotes, books) | ✅ except Books (shallow) |
| M8 | Tables | ✅ core (table/cell styles, rotation); diagonal lines, anchored objects in cells (#163) |
| M9 | Performance (MT composition, tiles) | in progress: parallel compose 6.7×, damage regions; incremental composition next |
| M10 | Find/Change, spelling, Preflight | ✅ (spelling English only) |
| M11 | Layout power features (liquid/alternate layouts, data merge) | present, shallow; Flex Layout (new in InDesign 2026) present (no object-style Flex settings, no IDML) |
| M12 | Interactive & digital (EPUB, HTML, interactive PDF) | present; HTML basic (#314), fixed-layout EPUB as images |
| M13 | Automation (scripts, batch) | ✅ own command scripts, CLI, MCP; ExtendScript/UXP compatibility ✗ |
| M14 | 1.0 polish & packaging | packaging ✅ (macOS, Windows x64/ARM64, Linux deb/AppImage/Flatpak, FreeBSD, web); polish not started |
| **A1** | **Alpha** (core-workflow gate) | ✅ 2026-10-20: every core workflow works end to end on macOS; see Alpha gate |
| **B1** | **Beta** (ready ≥ 75%, INDD/IDML reliable) | not started as a milestone; see below |

## To beta

Beta needs ~75% ready for real work and no blocking gap in the main file format. From ~47% today
that is ~28 points and **420–750 h** (~120–200 wall-clock hours with four to five agents):

| Work | Gap | Hours |
|---|---|---|
| Basic editing on every platform | 1 | 30–50 |
| INDD real-file corpus | 2 | 10–30 |
| IDML real-file corpus and fixes | 3 | 60–100 |
| Composition oracle and fixes | 4, 13 | 80–140 |
| Start-up robustness, fonts | 5, 6 | 25–50 |
| CI, Windows/Linux runtime tests | 7, 14 | 35–70 |
| IME | 8 | 10–20 |
| PDF/X-1a/X-3, presets, physical print validation | 9, 10 | 65–110 |
| Styles, tables, missing menu items (the most-used half) | 11, 12, 16 | 60–100 |
| Long documents (Books) | 15 | 30–50 |
| **Total** | | **420–750** |

## After beta: to full parity

Remaining gaps 16–25: the rest of the menu long tail, HTML/EPUB depth, colour management, placed
graphics depth (PostScript), performance on large documents, localization to `full` in the
twelve languages, InDesign scripting compatibility (owner decision), AI features (owner decision),
accessibility. **730–1,200 h** more; **1,150–1,950 h** in total from today.

## Superseded plan

The order the project followed until 2026-10-04 ("Next: books · New Window / split view · Ink
Manager and Separations Preview · XML structure, tags and export · buttons and forms · PDF/X-4
output intent · EPS place") is done except Books depth and EPS (preview only).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-20 | minor | Alpha gate passes: the threading row's re-test on macOS (12 checks) and a #165 regression test; stage alpha, A1 ✅ |
| 2026-10-20 | minor | Alpha gate: #165 closed, threading row is a re-test (~1 h); styles row yes; focus items and milestones follow the closed issues; Flex Layout present |
| 2026-10-13 | minor | INDD and INDT open through storytold/indd-utils; focus item 3 shrinks to a real-file corpus (10–30 h) |
| 2026-10-12 | minor | Threading row: threading by hand works; the gate waits on the Primary Text Frame page fixes (#165), 5–7 h |
| 2026-10-10 | trivial | Ready figure 42% → 47% (method aligned with the standard); beta distance 28 points |
| 2026-10-10 | minor | Alpha gate table added (6 core workflows); gate fails on manual threading, stage pre-alpha; A1 milestone and focus item 0 |
| 2026-10-10 | major | Created: milestones moved from ROADMAP.md and re-assessed; Current focus and the beta plan from the full re-measure |


### Windows installer unification — release preparation, 2026-10-12

The shared craft-installer UI preserves this revision's install options and provides persistent installation, repair and Windows Settings removal completion screens. Silent modes stay unattended. Earlier Windows acceptance is recorded in packaging/windows/README.md; normal CI and production-profile upgrade qualification against pinned current releases are in progress. No release or PR has been published.
