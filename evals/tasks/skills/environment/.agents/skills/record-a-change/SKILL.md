---
name: record-a-change
description: How to record a change for the next release of this crate. Use it whenever you add, change or fix something its users will notice.
---
# Recording a change

CHANGELOG.md is built by the release script from the files in `changes/`. Never edit it by
hand: the next release overwrites hand edits.

Record each change as one new file, `changes/<slug>.toml`, where the slug is a few words in
kebab-case. It holds exactly two lines:

```toml
kind = "added"
summary = "Add `function_name`, which does what it does."
```

- `kind` is `added`, `changed` or `fixed`.
- `summary` is one sentence in the present tense, ending with a period. Name the function.

Then check every record from the project's root: `sh <this skill's folder>/check.sh`.
