import { type GlzMarkdown, type GlzMarkdownOptions, renderMarkdown } from './markdown';
import { GLZ_MARKDOWN_LABELS } from './markdown-html';

const DONE: GlzMarkdownOptions = { streaming: false, labels: GLZ_MARKDOWN_LABELS, highlight: null };
const STREAMING: GlzMarkdownOptions = { ...DONE, streaming: true };

function html(markdown: GlzMarkdown): string {
  return markdown.blocks.map((block) => block.html).join('');
}

/** The rendered HTML as the browser would build it. */
function dom(text: string, options = DONE): DocumentFragment {
  const template = document.createElement('template');
  template.innerHTML = html(renderMarkdown(text, options));
  return template.content;
}

/** Elements that fetch, run or style something, which no reply may make. */
const LOADING =
  'img, image, svg, picture, source, video, audio, iframe, frame, object, embed, link, ' +
  'style, script, use, meta, base, input, form, body, table[background], [style], [src], ' +
  '[srcset], [poster], [background], [ping], [data], [action], [formaction]';

function loads(fragment: DocumentFragment): readonly string[] {
  const found = Array.from(fragment.querySelectorAll(LOADING), (element) => element.outerHTML);
  const handlers = Array.from(fragment.querySelectorAll('*')).filter((element) =>
    element.getAttributeNames().some((name) => name.startsWith('on')),
  );
  return [...found, ...handlers.map((element) => element.outerHTML)];
}

/** What a person reads: the text outside code. */
function prose(fragment: DocumentFragment): string {
  const copy = fragment.cloneNode(true) as DocumentFragment;
  for (const code of copy.querySelectorAll('pre, code')) {
    code.remove();
  }
  return copy.textContent;
}

const REMOTE = 'https://evil.example/p.png?secret=ab12';

describe('renderMarkdown', () => {
  it('renders GitHub-flavoured markdown: headings, emphasis, lists, quotes, rules', () => {
    const fragment = dom(
      '# Notes\n\nSome **bold**, *italic*, ~~old~~ and `code`.\nA new line.\n\n' +
        '- one\n- two\n\n3. three\n\n> quoted\n\n---',
    );
    expect(fragment.querySelector('h1')?.textContent).toBe('Notes');
    expect(fragment.querySelector('strong')?.textContent).toBe('bold');
    expect(fragment.querySelector('em')?.textContent).toBe('italic');
    expect(fragment.querySelector('del')?.textContent).toBe('old');
    expect(fragment.querySelector('p > code')?.textContent).toBe('code');
    expect(fragment.querySelector('p br')).not.toBeNull();
    expect(fragment.querySelectorAll('ul > li')).toHaveLength(2);
    expect(fragment.querySelector('ol')?.getAttribute('start')).toBe('3');
    expect(fragment.querySelector('blockquote')?.textContent.trim()).toBe('quoted');
    expect(fragment.querySelector('hr')).not.toBeNull();
  });

  it('renders tables in a scrolling box, aligned as written', () => {
    const fragment = dom('| Case | Count |\n| :--- | ----: |\n| empty | 3 |');
    const box = fragment.querySelector('.glz-markdown-table');
    expect(box?.getAttribute('tabindex')).toBe('0');
    expect(Array.from(box?.querySelectorAll('th') ?? [], (cell) => cell.textContent)).toEqual([
      'Case',
      'Count',
    ]);
    expect(box?.querySelector('td[align="right"]')?.textContent).toBe('3');
  });

  it('renders task lists as named boxes, not inputs', () => {
    const fragment = dom('- [x] Fix the parser\n- [ ] Write the docs');
    const boxes = Array.from(fragment.querySelectorAll('.glz-markdown-task'));
    expect(boxes.map((box) => box.getAttribute('aria-label'))).toEqual(['Done', 'Not done']);
    expect(boxes.map((box) => box.getAttribute('role'))).toEqual(['img', 'img']);
    expect(boxes[0]?.classList).toContain('glz-markdown-task-done');
    expect(fragment.querySelector('input')).toBeNull();
  });

  it('renders code blocks with their language, a head for the copy button, and text escaped', () => {
    const fragment = dom('```rust\nlet a = b < c && d;\n```\n\n    indented');
    const [named, plain] = Array.from(fragment.querySelectorAll('.glz-markdown-code'));
    expect(named?.querySelector('.glz-markdown-language')?.textContent).toBe('rust');
    expect(named?.querySelector('pre')?.getAttribute('tabindex')).toBe('0');
    expect(named?.querySelector('code')?.className).toBe('language-rust');
    expect(named?.querySelector('code')?.textContent).toBe('let a = b < c && d;');
    expect(plain?.querySelector('.glz-markdown-code-head')?.textContent).toBe('');
    expect(plain?.querySelector('code')?.textContent).toBe('indented');
  });

  it('highlights code in the languages the highlighter knows, and escapes the rest', () => {
    const highlight = (code: string, language: string): string | null =>
      language === 'rust' ? `<span class="hljs-keyword">${code}</span>` : null;
    const fragment = dom('```rust\nfn\n```\n\n```nope\n<b>\n```', { ...DONE, highlight });
    expect(fragment.querySelector('.language-rust .hljs-keyword')?.textContent).toBe('fn');
    expect(fragment.querySelector('.language-nope')?.innerHTML).toBe('&lt;b&gt;');
  });

  it('names the languages to load once a code block’s opening line is whole', () => {
    expect(renderMarkdown('```rus', STREAMING).languages).toEqual([]);
    expect(renderMarkdown('```rust\n', STREAMING).languages).toEqual(['rust']);
    expect(renderMarkdown('```ts\na\n```\n\n- x\n  ```ts\n  b\n  ```', DONE).languages).toEqual([
      'ts',
    ]);
    // Only what a class can carry.
    expect(renderMarkdown('```"><x\na\n```', DONE).languages).toEqual([]);
  });

  it('opens links in a new tab, without opener or referrer, and says so', () => {
    const fragment = dom('[the guide](https://example.com/guide "Guide"), www.example.org, a@b.co');
    const links = Array.from(fragment.querySelectorAll('a'));
    expect(links.map((link) => link.getAttribute('href'))).toEqual([
      'https://example.com/guide',
      'http://www.example.org/',
      'mailto:a@b.co',
    ]);
    for (const link of links) {
      expect(link.getAttribute('target')).toBe('_blank');
      expect(link.getAttribute('rel')).toBe('noopener noreferrer');
      expect(link.querySelector('.glz-visually-hidden')?.textContent).toBe(' (opens in a new tab)');
    }
    expect(links[0]?.getAttribute('title')).toBe('Guide');
  });

  it.each([
    ['[run](javascript:alert(1))', 'run'],
    ['[run](JaVaScRiPt:alert(1))', 'run'],
    ['[data](data:text/html,<b>x</b>)', 'data'],
    ['[file](src/parse.rs)', 'file'],
    ['[here](/settings)', 'here'],
    ['[vb](vbscript:msgbox)', 'vb'],
  ])('shows %j as its text alone: only web and email addresses link', (text, shown) => {
    const fragment = dom(text);
    expect(fragment.querySelector('a')).toBeNull();
    expect(fragment.textContent.trim()).toBe(shown);
  });

  it.each([
    ['an image', `![chart](${REMOTE})`],
    ['an image with a title', `![chart](${REMOTE} "t")`],
    ['an image in angle brackets', '![chart](<https://evil.example/a b.png>)'],
    ['a reference image', `![chart][r]\n\n[r]: ${REMOTE}`],
    ['a reference image defined first', `[r]: ${REMOTE}\n\n![chart][r]`],
    ['a collapsed reference image', `[r]: ${REMOTE}\n\n![r][]`],
    ['a shortcut reference image', `[r]: ${REMOTE}\n\n![r]`],
    ['an image in a link', `[![badge](${REMOTE})](https://ok.example)`],
    ['an image in a table', `| a |\n| - |\n| ![t](${REMOTE}) |`],
    ['an image in a list', `- ![l](${REMOTE})`],
    ['an image in a quote', `> ![q](${REMOTE})`],
    ['an image in a heading', `# ![h](${REMOTE})`],
    ['an SVG data image', '![x](data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=)'],
    ['raw <img>', `<img src="${REMOTE}">`],
    ['raw <img> inline', `Look: <img src=${REMOTE}> here`],
    ['raw <IMG> in capitals', `<IMG SRC=${REMOTE}>`],
    ['raw <img> with a handler', '<img src=x onerror=alert(1)>'],
    ['raw <img> in a link', `[<img src=${REMOTE}>](https://ok.example)`],
    ['raw <img> in a <pre>', `<pre><img/src=${REMOTE}></pre>`],
    ['raw <img> after inline <pre>', `a <pre> b <img/src=${REMOTE}> c </pre>`],
    ['raw <picture>', `<picture><source srcset="${REMOTE}"></picture>`],
    ['raw <video>', `<video poster="${REMOTE}"></video>`],
    ['raw <audio>', `<audio src="${REMOTE}"></audio>`],
    ['raw <iframe>', `<iframe src="${REMOTE}"></iframe>`],
    ['raw <object>', `<object data="${REMOTE}"></object>`],
    ['raw <input type=image>', `<input type="image" src="${REMOTE}">`],
    ['raw <link>', `<link rel="prefetch" href="${REMOTE}">`],
    ['raw <meta> refresh', `<meta http-equiv="refresh" content="0;url=${REMOTE}">`],
    ['raw <style>', `<style>p { background: url(${REMOTE}) }</style>`],
    ['a style attribute', `<div style="background:url(${REMOTE})">x</div>`],
    ['a ping', `<a href="https://ok.example" ping="${REMOTE}">x</a>`],
    ['a background', `<table background="${REMOTE}"><tr><td>x</td></tr></table>`],
    ['SVG <image>', `<svg><image href="${REMOTE}"/></svg>`],
    ['SVG <use>', `<svg><use href="${REMOTE}#a"/></svg>`],
    ['SVG in a fenced block', `\`\`\`svg\n<svg><image href="${REMOTE}"/></svg>\n\`\`\``],
    [
      'Mermaid',
      `\`\`\`mermaid\ngraph TD\n  A["<img src='${REMOTE}'>"] --> B\n  click A "${REMOTE}"\n\`\`\``,
    ],
    ['an escaped tag', `&lt;img src=${REMOTE}&gt;`],
    ['a numeric reference tag', `&#60;img src=${REMOTE}&#62;`],
  ])('never fetches %s: no element loads anything', (_, text) => {
    for (const options of [DONE, STREAMING]) {
      const fragment = dom(text, options);
      expect(loads(fragment)).toEqual([]);
      // Every address left is a link, which takes a click.
      for (const element of fragment.querySelectorAll('[href]')) {
        expect(element.tagName).toBe('A');
        expect(element.getAttribute('target')).toBe('_blank');
      }
    }
  });

  it('shows a remote image as a link to it, named by its description', () => {
    const link = dom(`![Sales by month](${REMOTE})`).querySelector('a');
    expect(link?.getAttribute('href')).toBe(REMOTE);
    expect(link?.classList).toContain('glz-markdown-image');
    expect(link?.textContent).toBe('Image: Sales by month (opens in a new tab)');
    // In a link, an image is its words: links never nest.
    const outer = dom(`[![badge](${REMOTE})](https://ok.example)`).querySelectorAll('a');
    expect(Array.from(outer, (each) => each.getAttribute('href'))).toEqual(['https://ok.example/']);
    expect(outer[0]?.textContent).toBe('Image: badge (opens in a new tab)');
  });

  it('shows raw HTML as the text it is', () => {
    expect(dom('Look: <b>bold</b>').querySelector('p')?.textContent).toBe('Look: <b>bold</b>');
    expect(dom('<div>\n<img src=x>\n</div>').querySelector('.glz-markdown-raw')?.textContent).toBe(
      '<div>\n<img src=x>\n</div>',
    );
  });

  it('renders again only the blocks that changed, reusing the rest', () => {
    const first = renderMarkdown('# Title\n\nFirst para.\n\nSecond', STREAMING);
    const next = renderMarkdown('# Title\n\nFirst para.\n\nSecond para', STREAMING, first);
    expect(next.blocks).toHaveLength(3);
    expect(next.blocks[0]).toBe(first.blocks[0]);
    expect(next.blocks[1]).toBe(first.blocks[1]);
    expect(next.blocks[2]).not.toBe(first.blocks[2]);
    // Finished, the last block renders once more, now whole.
    const done = renderMarkdown('# Title\n\nFirst para.\n\nSecond para', DONE, next);
    expect(done.blocks[1]).toBe(first.blocks[1]);
    expect(done.blocks[2]).not.toBe(next.blocks[2]);
  });

  it('renders every block again when a link definition or the highlighter changes', () => {
    const first = renderMarkdown('See [docs][d].\n\nMore', STREAMING);
    expect(first.blocks[0]?.html).not.toContain('<a');
    const text = 'See [docs][d].\n\nMore\n\n[d]: https://d.example';
    const linked = renderMarkdown(text, DONE, first);
    expect(linked.blocks[0]?.html).toContain('href="https://d.example/"');
    const highlighted = renderMarkdown(text, { ...DONE, highlight: () => null }, linked);
    expect(highlighted.blocks[0]).not.toBe(linked.blocks[0]);
    expect(highlighted.blocks[0]?.html).toBe(linked.blocks[0]?.html);
  });
});

/** A reply as a model writes it, with every kind of block and inline markup. */
const REPLY = [
  '# Release notes',
  '',
  'The **parser** now returns `None` for *empty* input, and ~~panics~~ no longer.',
  '',
  '- [x] Fix the parser',
  '- [ ] Write the docs, see [the guide](https://example.com/guide)',
  '',
  '| Case | Before | After |',
  '| :--- | :----: | ----: |',
  '| empty | panic | `None` |',
  '',
  '```rust',
  'fn parse(input: &str) -> Option<Value> {',
  '    input.parse().ok()',
  '}',
  '```',
  '',
  '> Tests: **12 passed**.',
  '',
  '1. Build',
  '2. Ship',
].join('\n');

describe('renderMarkdown, streaming', () => {
  it('renders every prefix of a reply without markup showing, finished blocks as final', () => {
    const final = renderMarkdown(REPLY, DONE);
    let previous: GlzMarkdown | null = null;
    for (let end = 1; end <= REPLY.length; end += 1) {
      const text = REPLY.slice(0, end);
      const streamed = renderMarkdown(text, STREAMING, previous);
      // Reusing blocks gives what rendering from nothing gives.
      expect(html(streamed)).toBe(html(renderMarkdown(text, STREAMING)));
      const fragment = document.createElement('template');
      fragment.innerHTML = html(streamed);
      const shown = prose(fragment.content);
      expect({ text, shown }).toEqual({ text, shown: shown.replace(/\*\*|~~|`|\]\(/gu, '') });
      // Blocks before the last are final: what was shown never changes.
      const finished = streamed.blocks.filter((block) => !block.tail);
      expect(finished.map((block) => block.html)).toEqual(
        final.blocks.slice(0, finished.length).map((block) => block.html),
      );
      previous = streamed;
    }
  });

  it.each([
    ['**bold', '<p><strong>bold</strong></p>'],
    ['Some *', '<p>Some</p>'],
    ['Use `cargo', '<p>Use <code>cargo</code></p>'],
    ['See [the guide](https://exa', '<p>See the guide</p>'],
    ['Chart: ![sales](https://evil.example/', '<p>Chart:</p>'],
    ['Steps\n\n1. Build\n2', '<p>Steps</p>\n<ol>\n<li>Build</li>\n</ol>'],
    ['Para\n-', '<p>Para</p>'],
    ['| a | b |\n| -', ''],
  ])('renders the half-written %j as it will end', (text, expected) => {
    expect(html(renderMarkdown(text, STREAMING)).trim()).toBe(expected);
  });

  it('keeps an unclosed fence a code block as it grows, and shows it whole when done', () => {
    const growing = dom('```rust\nfn ma', STREAMING);
    expect(growing.querySelector('code')?.textContent).toBe('fn ma');
    const done = dom('```rust\nfn main() {}\n```', DONE);
    expect(done.querySelector('code')?.textContent).toBe('fn main() {}');
  });
});
