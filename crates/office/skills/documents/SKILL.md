---
name: documents
description: Read and edit Word documents (.docx) with glave's own commands, nothing to install. Use whenever a task reads a Word document or changes one.
---
# Word documents

glave reads and edits Word documents itself: run `glave doc` with bash. Nothing needs
installing, so don't reach for Python, LibreOffice or a package for this.

```sh
glave doc read report.docx              # its text: headings (#), lists, tables, comments
glave doc read report.docx > report.md  # a long one, to read a part at a time
```

Tracked changes read as accepted, insertions in and deletions out; each comment follows its
paragraph as `[comment by … on "…": …]`.

## Editing a document

To change a document and keep everything else as it is, edit its XML in place; never remake
it:

```sh
glave doc unpack report.docx report     # its parts, an element to a line
# edit report/word/document.xml with edit
glave doc pack report report.docx --replace
glave doc read report.docx              # check the change
```

- The text is in `word/document.xml`: a paragraph `<w:p>` holds runs `<w:r>`, each with its
  formatting in `<w:rPr>` and its text in `<w:t>`. Change the words inside `<w:t>` and keep
  the runs and their `<w:rPr>`, so the formatting stays.
- Word splits words across runs, so search for one word of the text and read the lines
  around it. When new words span runs, put each in the run whose formatting it should have,
  and delete runs left empty.
- Text that starts or ends with a space needs `<w:t xml:space="preserve">`. Write `&amp;`,
  `&lt;` and `&gt;` for `&`, `<` and `>`.
- Don't add tracked changes unless asked. `pack` checks every part and says where one breaks.

glave doesn't make new Word documents yet: start from a copy of one, or say so.
