import { TestBed } from '@angular/core/testing';
import type { ReplySources } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ReplySourcesComponent } from './reply-sources.component';

const SOURCED: ReplySources = {
  reply: 9,
  sentence: 'Sources: 2 pages',
  pages: [
    { url: 'https://tokio.rs/blog/2026-10-tokio-2', title: 'Tokio 2.0 is out' },
    { url: 'https://docs.rs/tokio/latest/tokio/', title: null },
  ],
};

async function render(sources: ReplySources): Promise<HTMLElement> {
  const fixture = TestBed.createComponent(ReplySourcesComponent);
  fixture.componentRef.setInput('sources', sources);
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

describe('ReplySourcesComponent', () => {
  it('says what the reply drew on in the server’s words, then links each page', async () => {
    const root = await render(SOURCED);
    const section = root.querySelector('[data-testid="reply-sources"]');
    const heading = document.getElementById(section?.getAttribute('aria-labelledby') ?? '');
    expect(heading?.textContent).toBe('Sources: 2 pages');
    const links = Array.from(
      root.querySelectorAll<HTMLAnchorElement>('[data-testid="reply-sources-link"]'),
    );
    expect(links.map((link) => link.href)).toEqual(SOURCED.pages.map((page) => page.url));
    expect(links.map((link) => link.querySelector('.title')?.textContent ?? null)).toEqual([
      'Tokio 2.0 is out',
      null,
    ]);
    expect(links.map((link) => link.querySelector('.address')?.textContent)).toEqual([
      'tokio.rs/blog/2026-10-tokio-2',
      'docs.rs/tokio/latest/tokio/',
    ]);
    await expectAccessible(root);
  });

  it('opens each page in a new tab, without opener or referrer, and says so', async () => {
    const root = await render(SOURCED);
    const link = root.querySelector<HTMLAnchorElement>('[data-testid="reply-sources-link"]');
    expect(link?.target).toBe('_blank');
    expect(link?.rel).toBe('noopener noreferrer');
    expect(link?.querySelector('.glz-visually-hidden')?.textContent).toBe('(opens in a new tab)');
    // Its parts read apart.
    expect(link?.textContent).toBe(
      'Tokio 2.0 is out tokio.rs/blog/2026-10-tokio-2 (opens in a new tab)',
    );
  });
});
