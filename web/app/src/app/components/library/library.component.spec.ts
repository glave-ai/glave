import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import type { LibraryItem, Project } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';
import { settle } from '@glave/glaze/testing/settle';

import { ProjectsService } from '../../services/projects.service';
import { ServerService } from '../../services/server.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER, picture, record } from '../../testing/fixtures';
import { objectUrls } from '../../testing/object-urls';
import { LibraryComponent } from './library.component';

const PIECE = { piece: { media_type: 'image/png', bytes: 3, offset: 0, data: 'UE5H' } };
const CHATS = record('chats', 'Chats', 'personal');

async function render(
  items: readonly LibraryItem[],
  projects: readonly Project[] = [CHATS],
  server: object = SERVER,
): Promise<{
  readonly root: HTMLElement;
  readonly fake: FakeSession;
  readonly fixture: ReturnType<typeof TestBed.createComponent<LibraryComponent>>;
}> {
  const fake = new FakeSession()
    .answer('library/list', { items })
    .answer('library/read', PIECE)
    .answer('library/remove', {})
    .answer('project/list', { projects })
    .answer('thread/list', { threads: [] })
    .answer('server/discover', server)
    .answer('share/get', {});
  TestBed.configureTestingModule({
    providers: [provideRouter([]), ...provideFakeSession(fake)],
  });
  await TestBed.inject(ProjectsService).refresh();
  await TestBed.inject(ServerService).load();
  const fixture = TestBed.createComponent(LibraryComponent);
  await settle(fixture);
  return { root: fixture.nativeElement as HTMLElement, fake, fixture };
}

function names(root: HTMLElement): readonly string[] {
  return Array.from(root.querySelectorAll('[data-testid="library-name"]')).map((each) =>
    each.textContent.trim(),
  );
}

describe('LibraryComponent', () => {
  it('says plainly that pictures will appear, with nothing yet', async () => {
    const { root } = await render([]);
    const empty = root.querySelector('[data-testid="library-empty"]');
    expect(empty?.textContent).toContain('Nothing here yet');
    expect(empty?.textContent).toContain('Pictures glave makes for you will appear here.');
    expect(root.querySelector('[data-testid="library-kind"]')).toBeNull();
    await expectAccessible(root);
  });

  it('draws every item newest first, and filters by kind but no project with only Chats', async () => {
    objectUrls();
    const { root } = await render([picture('fox', 'chats', 't1'), picture('hare', 'chats')]);
    expect(names(root)).toEqual(['fox', 'hare']);
    expect(root.querySelector('[data-testid="library-kind"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="library-project"]')).toBeNull();
    await expectAccessible(root);
  });

  it('filters by project when the person has one beside their Chats', async () => {
    objectUrls();
    const { root, fixture } = await render(
      [picture('fox', 'p1'), picture('hare', 'chats')],
      [record('p1', 'glave'), CHATS],
    );
    root.querySelector<HTMLElement>('[data-testid="library-project-trigger"]')?.click();
    await settle(fixture);
    const places = Array.from(
      document.querySelectorAll<HTMLElement>('[data-testid="library-project-option"]'),
    );
    // Only the projects the items are in, in the person's order.
    expect(places.map((place) => place.textContent.trim())).toEqual([
      'Every project',
      'glave',
      'Chats',
    ]);
    places[1]?.click();
    await settle(fixture);
    expect(names(root)).toEqual(['fox']);
    await expectAccessible(root);
  });

  it('offers Share on a picture opened, only where the server shares files', async () => {
    objectUrls();
    const open = async (server: object): ReturnType<typeof render> => {
      const rendered = await render([picture('fox', 'chats', 't1')], [CHATS], server);
      rendered.root.querySelector<HTMLElement>('[data-testid="library-open"]')?.click();
      await settle(rendered.fixture);
      return rendered;
    };
    const plain = await open(SERVER);
    expect(plain.root.querySelector('[data-testid="library-share"]')).toBeNull();
    TestBed.resetTestingModule();
    const { root, fake, fixture } = await open({ ...SERVER, shares: ['anyone'] });
    root.querySelector<HTMLElement>('[data-testid="library-share"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('share/get')).toEqual([{ kind: 'item', project: 'chats', asset: 'fox' }]);
    expect(document.body.textContent).toContain('Share “fox”');
    await expectAccessible(document.body);
  });

  it('opens a picture large, and deletes it from the Library once asked', async () => {
    const urls = objectUrls();
    const { root, fake, fixture } = await render([picture('fox', 'chats', 't1')]);
    root.querySelector<HTMLElement>('[data-testid="library-open"]')?.click();
    await settle(fixture);
    const preview = root.querySelector('[data-testid="library-preview"]');
    expect(preview?.querySelector('img')?.getAttribute('src')).toBe(urls.made[0]);
    const download = root.querySelector('[data-testid="library-picture-download"]');
    expect(download?.getAttribute('download')).toBe('fox.png');
    expect(root.querySelector('[data-testid="library-go"]')?.getAttribute('href')).toBe('/t/t1');
    await expectAccessible(root);

    root.querySelector<HTMLElement>('[data-testid="library-delete"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('library/remove')).toEqual([]);
    root.querySelector<HTMLElement>('[data-testid="library-delete-confirm-ok"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('library/remove')).toEqual([{ project: 'chats', asset: 'fox' }]);
    expect(names(root)).toEqual([]);
  });
});
