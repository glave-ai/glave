---
name: pdfs
description: Merge, split, reorder and rotate PDF pages, and fill in PDF forms, with glave's own commands, nothing to install. Use whenever a task combines PDFs, keeps or takes out some of their pages, turns pages, or fills in a form.
---
# PDFs

To read a PDF, use `read`: it shows you the pages as they are. To change one, run `glave doc`
with bash. Nothing needs installing, so don't reach for Python, qpdf or a package for this.

```sh
glave doc merge all.pdf a.pdf b.pdf       # a.pdf's pages, then b.pdf's, in a new all.pdf
glave doc pages a.pdf part.pdf 1-3,7      # pages 1 to 3 then 7; 7,1-3 puts 7 first; 5- runs to the end
glave doc rotate a.pdf upright.pdf 90 2,4 # pages 2 and 4 turned clockwise; every page with none named
```

Each makes a new file and keeps the pages as they are: text, images, links and form fields.
To split a PDF, run `pages` once for each part. To change a file itself, give its own name as
the new one and add `--replace`.

## Filling in a form

```sh
glave doc fields form.pdf                 # each field's name, kind, page and value
glave doc fill form.pdf filled.pdf <<'EOF'
{"Name": "Ada Lovelace", "Date": "2027-03-14", "Member": true, "Plan": "Basic"}
EOF
glave doc fields filled.pdf               # check the values
```

- Name each field as `fields` shows it: the label printed on the page is often not its name.
  Read the form with `read` to see which field is which, and what format a date or a number
  takes there.
- Text fields take text. A checkbox takes true or false. A choice takes one of the values
  `fields` lists.
- `fill` writes nothing if a name or a value is wrong, and says which. Leave out fields that
  aren't yours to fill, such as a manager's or an office's.
