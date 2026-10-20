# Flow: Word and Excel files that stay linked

Flow keeps a Word, Excel or text file linked to the story it was placed in. When the writer sends
a new version, Flow shows what the update would change, then brings the file's changes in and
keeps the edits you made in the layout. Compare shows what changed since an earlier version, and
History keeps those versions in the file. Everything here is also a command, so a script or an
agent can run the same steps without the app ([the update-all job](#the-update-all-job)).

## Place a file linked

1. Choose **File ▸ Place with Flow…** and pick a `.docx`, `.rtf`, `.txt`, `.md` or `.xlsx` file.
2. The **Place with Flow** dialog checks the file first:
   - **Found in the file** lists what needs your attention: Excel error values, formulas with no
     calculated value ("open and save the file in Excel"), values taken from other workbooks,
     and Word fields Word can't fill. It says **Nothing that needs your attention.** when the
     file is clean.
   - **Preview** shows the paragraphs, or the table's rows.
   - For a Word or RTF file: **Preset**, **Remove Styles and Formatting from Text and Tables**,
     **Style Name Conflicts** and **Style Mapping**, as in Import Options.
   - For an Excel file: see [Excel options](#excel-options).
3. Choose **Place**. If the file has errors, tick **Place with 1 error** (or **… errors**) first.

The file is always placed linked from this dialog. **File ▸ Place…** and **File ▸ Place with
Import Options…** link text files only when **Create Links When Placing Text Files** is on
(Preferences ▸ File Handling ▸ Links; it is off by default).

A linked file fills a whole story. Placed at an insertion point, the text comes in unlinked
(the warning says so), and so does a file too large to check. Only a file on disk can be linked,
so the web version doesn't link.

## Update a linked file

Linked files are listed in the **Flow** panel (**Window ▸ Editorial ▸ Flow**), each marked **File
changed**, **Missing**, **Too large to check** or **Up to date**. **Check all** checks every
changed file.

1. Choose a file marked **File changed**. Flow works out the update without changing anything:
   - **Changes from the file**: the sentences, and the table cells, the file changed.
   - **Your edits kept**: where the layout differs from the file, the layout's version stays.
   - **Conflicts**: where you and the file both changed a sentence. The file's text goes in, and
     yours moves to a note.
   - **Errors in the file**, which stop the update until you allow them (step 2), and **Values
     from other workbooks**, which may be out of date but don't stop it.
   - Whether the story still fits its frames, and a strip of the pages the update touches. Click
     a line or a page to go there.
2. Choose **Update**. With errors in the file, tick **Update with 1 error** (or **… errors**)
   first, or fix the file and choose **Check again**.

After the update, the panel lists what came in under the same headings, and **Highlight on the
page** marks it on the page until you choose **Clear highlights**. Errors stay highlighted until
they're fixed. Each conflict becomes a note by **Flow** that starts "Layout had:", in the
**Notes** panel (**Window ▸ Editorial ▸ Notes**).

**Update** in the **Links** panel (**Window ▸ Links**) opens the same review for a text file. Once
a file has been updated, the Links panel shows **Last update** with what it did (for example
"Last update just now: 1 change, 1 conflict"), a line per conflict that goes to it, and **Review
in Flow**.

What doesn't merge:
- **A story with tracked changes** can't be updated: accept or reject them first.
- **A story placed before Flow could merge** is replaced by the file's text. The review says so
  in red before you choose **Update**.
- **A table whose rows, columns or merged cells changed** comes in as the file's table, without
  the layout's edits and formatting in it. A table that kept its shape merges cell by cell and
  keeps its formatting.

## Relink

When a file moves, the Flow panel marks it **Missing** ("Its story keeps the text from the last
update") and offers **Relink in the Links panel**. There, choose the file and **Relink…**, then
pick the file in its new place: its text is imported into the linked story. **Relink to
Folder…** finds moved graphics by folder.

## Excel options

A workbook is placed as a table. In the **Place with Flow** dialog, under **Workbook**:
- **Sheet**: the first sheet unless you choose another.
- **Cells**: all the cells the sheet uses, or a range such as `B3:F40`.
- **Header rows**: 1 unless you change it (0–20).
- **Table style**: **None** or one of the document's table styles.

The link keeps these, so each update reads the same part of the workbook. The Flow panel shows
it under the file's name (for example `Figures!B3:F40`). Numbers and dates come in as Excel shows
them with US settings. Formulas come in as the values Excel last calculated, so a workbook saved
without them is flagged until it is opened and saved in Excel.

## Compare

**File ▸ Compare with Earlier Version…** compares the document with an earlier version of it, a
`.designcraft` file you pick. The result opens as a new, unsaved document: the earlier version's
lost text is back, marked deleted, and the new text is marked added, word by word, in text,
table cells and footnotes. Text frames that were deleted, and tables whose shape changed, are
reported in the status bar rather than marked.

- **Window ▸ Editorial ▸ Track Changes** lists each change, with **Accept** and **Reject**, and
  **Accept All** and **Reject All** at the bottom. Accepting everything gives the newer text,
  rejecting everything the earlier one.
- **File ▸ Export Changes as PDF…** writes a summary page listing the changes by page, then each
  page with a change, added text highlighted and deleted text struck through.

In the History panel, **Compare with Current** does the same with a kept version. A script can
compare with the document's last save: `compare.file {}` with no `path`.

## History

**Window ▸ Editorial ▸ History** lists the versions kept in the document's file, newest first:
- **Saved** when you save a changed document;
- **As opened**, the file as you opened it, the first time you save changes to a file that had
  no versions;
- **Before Flow update** and **Flow update** around each update;
- your own, from **File ▸ Save a Version…** (it saves the document and keeps the version with a
  label).

Choose a version, then **Restore** (undoable, and kept as a version of its own), **Compare with
Current** or **Export…** (a document of its own). Most versions are stored as their differences
from the one before (a full copy every ten), so they stay small.

Versions are kept by default. **Keep Earlier Versions in Saved Files** (Preferences ▸ File
Handling ▸ History) turns that off for every document, and **Keep Earlier Versions in This File**
in the History panel sets it for one. Anyone you send the file to can restore its versions:
**File ▸ Save a Copy without History…** leaves them out.

## The update-all job

The job a team runs when the writers deliver: update every changed link, export the document for
print, and export what changed for review, with no one opening the layout. Save it as
`update-all`:

```
# update-all: update every changed linked file, then export print.pdf (PDF/X-4) and changes.pdf.
links.update {}
file.exportPdf {"path": "print.pdf", "standard": "x4"}
compare.file {}
compare.exportPdf {"path": "changes.pdf"}
# Close the comparison: the updated document is active again for --save.
file.close {}
```

Run it from the document's folder with the [command-line tool](agents.md):

```sh
designcraft-cli script update-all --in report.designcraft --save report.designcraft
```

- `links.update {}` brings in every changed file, as **Update** does. A file that shows errors
  stops the job before anything changes; `links.check` lists them, and
  `links.update {"allowErrors": true}` brings them in anyway.
- `compare.file {}` compares the updated document with its last save, the file as it was before
  the job, so the comparison has to come before the job saves.
- When nothing changed, the job writes `print.pdf`, then stops at `compare.exportPdf` with "the
  document has no tracked changes", and the document isn't saved again.
- The job's output lists each step's result: what each file brought in (`reports`: changes, your
  edits kept, conflicts, with their pages), the comparison's count of changes, and the PDFs'
  pages and warnings.

## Commands

| Step | Commands |
|---|---|
| Place | `place.check {path, name, sheet?, range?, headerRows?}`, `file.place {path, link: true, sheet?, range?, headerRows?, tableStyle?, styleMap?, styleRules?, preset?}` |
| Update | `links.list`, `links.check`, `links.update {story?, dryRun?, allowErrors?}`, `links.clearHighlights` |
| Relink | `links.relink {story, path}`, `links.relinkFolder {dir}` |
| Compare | `compare.file {path?}`, `compare.versions {a, b?}`, `compare.exportPdf {path}`, `changes.list`, `changes.acceptAll`, `changes.rejectAll` |
| History | `version.list`, `version.commit {label?}`, `version.restore {id}`, `version.show {id}`, `version.export {id, path}`, `version.prune`, `version.keep {on?}` |

`designcraft-cli describe COMMAND` gives each one's parameters in full.
