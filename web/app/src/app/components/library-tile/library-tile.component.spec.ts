import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { shown } from '../../lib/library';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { picture } from '../../testing/fixtures';
import { objectUrls } from '../../testing/object-urls';
import { LibraryTileComponent } from './library-tile.component';

/** A picture of three bytes, as `library/read` answers it whole. */
const PIECE = { piece: { media_type: 'image/png', bytes: 3, offset: 0, data: 'UE5H' } };

const READER = { locale: 'en-US', timeZone: 'UTC' };
const NOW = Date.parse('2026-11-16T18:00:00Z');

async function render(chat: string | null): Promise<{
  readonly root: HTMLElement;
  readonly fake: FakeSession;
  readonly opened: () => number;
}> {
  const fake = new FakeSession().answer('library/read', PIECE);
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake)],
  });
  const fixture = TestBed.createComponent(LibraryTileComponent);
  fixture.componentRef.setInput('shown', shown(picture('fox', 'p1', chat), NOW, READER));
  let count = 0;
  fixture.componentInstance.opened.subscribe(() => (count += 1));
  await settle(fixture);
  return { root: fixture.nativeElement as HTMLElement, fake, opened: () => count };
}

describe('LibraryTileComponent', () => {
  it('draws its picture, read through the server, its name, when, and its chat', async () => {
    const urls = objectUrls();
    const { root, fake, opened } = await render('t1');
    expect(fake.paramsOf('library/read')).toEqual([{ asset: 'fox' }]);
    expect(root.querySelector('img')?.getAttribute('src')).toBe(urls.made[0]);
    expect(root.querySelector('[data-testid="library-name"]')?.textContent).toBe('fox');
    expect(root.querySelector('[data-testid="library-when"]')?.textContent).toBe('Today, 9:12 AM');
    const chat = root.querySelector('[data-testid="library-chat"]');
    expect(chat?.getAttribute('href')).toBe('/t/t1');
    expect(chat?.textContent.trim()).toBe('About fox');
    const open = root.querySelector<HTMLElement>('[data-testid="library-open"]');
    expect(open?.getAttribute('aria-label')).toBe('Open fox');
    await expectAccessible(root);
    open?.click();
    expect(opened()).toBe(1);
  });

  it('links to no chat once its chat is gone', async () => {
    objectUrls();
    const { root } = await render(null);
    expect(root.querySelector('[data-testid="library-chat"]')).toBeNull();
  });
});
