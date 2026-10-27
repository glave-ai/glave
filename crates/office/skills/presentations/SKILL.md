---
name: presentations
description: Read and edit PowerPoint decks (.pptx) with glave's own commands, nothing to install. Use whenever a task reads a deck, changes one or fills one in from a template.
---
# PowerPoint decks

glave reads and edits decks itself: run `glave doc` with bash. Nothing needs installing, so
don't reach for Python, LibreOffice or a package for this.

```sh
glave doc read deck.pptx   # each slide's title, text and tables, then its speaker notes
```

## Editing a deck

To change a deck or fill in a template and keep its design, edit its XML in place:

```sh
glave doc unpack deck.pptx deck   # its parts, and which is each slide
# edit the slides' XML with edit
glave doc pack deck deck.pptx --replace
glave doc read deck.pptx          # check the change
```

- Each slide is a part `ppt/slides/slideN.xml`, but N need not be its place in the deck:
  `unpack` lists them in order. Its text is in shapes `<p:sp>`: paragraphs `<a:p>` of runs
  `<a:r>`, each with its formatting in `<a:rPr>` and its text in `<a:t>`. Change the words
  inside `<a:t>` and keep the runs, so the formatting stays. Speaker notes are in
  `ppt/notesSlides/`.
- To add a slide, copy a slide's part and its `_rels` file, leaving out its notes, then add
  the copy to `ppt/presentation.xml` (a `<p:sldId>` with a new id and `r:id`),
  `ppt/_rels/presentation.xml.rels` and `[Content_Types].xml`. `pack` checks that the parts
  tie together and says what is missing.
- Write `&amp;`, `&lt;` and `&gt;` for `&`, `<` and `>`.
