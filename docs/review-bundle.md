# The review bundle

`review.export {dir, scale?}` writes what a reviewer's browser needs to show a document, select
its text and send comments back with an address the app understands. It is a folder:

```
manifest.json      the document, its version, its pages
pages/001.webp     one image per page, trim area only, `scale` pixels per point (default 2)
text.json          the text layer: every line of text, where it is and what it says
document.pdf       the PDF, for reviewers who want to download it
```

The export also gives the document a **review id** the first time (`id` in `document.json`, a
UUID) and keeps a version `Review vN` in the document's history, so Compare can later show what
changed since reviewers saw it. The version is written by the next Save, like a Flow update's;
the export never writes the document's own file. Save and Save As keep the id; Save a Copy,
snippets and IDML carry none, so a copy made for another client starts a review of its own.
`review.detach {}` clears it by hand.

## manifest.json

```json
{
  "format": "designcraft-review", "version": 1, "generator": "DesignCraft 0.9.0",
  "documentId": "0f3c…", "versionId": 4, "title": "Annual report", "scale": 2.0,
  "pages": [{ "index": 0, "label": "1", "spread": 0, "width": 612.0, "height": 792.0, "image": "pages/001.webp" }],
  "pdf": "document.pdf", "text": "text.json",
  "warnings": []
}
```

`index` is the absolute page index from 0; `label` is the page's name as the document numbers it;
`width` and `height` are the trim size in points; `versionId` is null when no version was kept
(Keep Earlier Versions is off for the document), and `warnings` say so, and name any text left
out of the text layer.

## text.json

```json
{ "pages": [ { "index": 0, "lines": [
  { "story": 3, "frame": 12, "start": 0, "end": 18,
    "text": "Hello review world", "bytes": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
    "rect": [72.0, 74.1, 161.3, 86.6] },
  { "story": 3, "frame": 12, "cell": { "table": 1, "row": 0, "col": 2 }, "start": 0, "end": 5, "text": "North", "bytes": [0, 1, 2, 3, 4], "rect": [320.5, 410.0, 352.2, 421.0] },
  { "story": 3, "frame": 12, "footnote": 7, "start": 0, "end": 23, "text": "…", "bytes": [], "rect": [72.0, 700.0, 300.0, 710.0] }
] } ] }
```

One record per laid-out line, in story order then frame order:

- `story`, and `cell {table, row, col}` or `footnote` (an id) when the text is in a table cell or
  a footnote: exactly the parameters `text.select`, `changes.list` and `note.new` take. `start`
  and `end` are UTF-8 byte offsets into the story's text, or the cell's or footnote's own text
  when one is named.
- `text` is the characters drawn, in drawing order; `bytes[i]` is the byte offset the `i`th
  character of `text` starts at. A selection from the `i`th to the `j`th character is the range
  `bytes[i]..bytes[j]` (or `..end` past the last character). Ligatures draw as one character
  that stands for its letters' first byte.
- `rect` is left, top, right, bottom in points from the page's top-left trim corner; multiply by
  the manifest's `scale` for pixels in the page image.
- `rtl` is present and true for right-to-left lines (glyphs are then in visual order).
- `frame` is the text frame item drawn in.

Left out for now, each named in the manifest's `warnings`: type on a path and vertical frames.
Text on parent pages (running heads, page numbers) isn't in the layer yet either. Text on the
pasteboard or overset has no line.
