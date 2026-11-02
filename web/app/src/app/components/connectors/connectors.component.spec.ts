import { LiveAnnouncer } from '@angular/cdk/a11y';
import { TestBed } from '@angular/core/testing';
import type { ConnectorInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { ConnectorsComponent } from './connectors.component';

/** A connector of the catalog, `available` unless said. */
function connector(
  id: string,
  name: string,
  category: string,
  description: string,
  status: ConnectorInfo['status'] = 'available',
): ConnectorInfo {
  const url = `https://mcp.${id}.example.com/mcp`;
  return { id, name, url, category, description, docs: url, status };
}

const CATEGORIES = [
  { id: 'dev', name: 'Developer tools' },
  { id: 'productivity', name: 'Productivity' },
];
const LINEAR = connector('linear', 'Linear', 'dev', 'Issues, projects and cycles.');
const DEEPWIKI = connector('deepwiki', 'DeepWiki', 'dev', 'Answers about public repositories.');
const NOTION = connector('notion', 'Notion', 'productivity', 'Pages, databases and search.');
const PAGE = 'https://auth.example.com/authorize?state=s1';

/** Lets the component's promises and the page settle. */
async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

/** The rows of the card `testId`, each as its tile, name, line and button's word. */
function rows(root: ParentNode, testId: string): readonly string[] {
  const card = root.querySelector(`[data-testid="${testId}"]`);
  const found = card?.querySelectorAll('[data-testid="connector"]') ?? [];
  const parts = '.tile, .name, .detail, [data-testid="connector-action"]';
  return [...found].map((row) =>
    [...row.querySelectorAll(parts)].map((part) => part.textContent.trim()).join(' '),
  );
}

/** Presses the button of the connector `id`. */
function press(root: ParentNode, id: string): void {
  root
    .querySelector(`[data-connector="${id}"] [data-testid="connector-action"]`)
    ?.dispatchEvent(new MouseEvent('click'));
}

/** A tab the page opens, standing in for the browser's. */
function fakeTab(): { readonly sentTo: string[]; readonly closed: () => boolean } {
  const sentTo: string[] = [];
  let closed = false;
  const tab = {
    opener: window,
    get closed() {
      return closed;
    },
    close: () => {
      closed = true;
    },
    location: { replace: (url: string) => sentTo.push(url) },
  } as unknown as Window;
  vi.spyOn(window, 'open').mockReturnValue(tab);
  return { sentTo, closed: () => closed };
}

async function page(fake: FakeSession): Promise<{
  readonly fixture: { whenStable(): Promise<unknown>; detectChanges(): void };
  readonly root: HTMLElement;
  readonly said: string[];
}> {
  const said: string[] = [];
  const announce = (text: string): Promise<void> => {
    said.push(text);
    return Promise.resolve();
  };
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), { provide: LiveAnnouncer, useValue: { announce } }],
  });
  const fixture = TestBed.createComponent(ConnectorsComponent);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement, said };
}

describe('ConnectorsComponent', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows those connected first, then the rest by category, and finds them by search', async () => {
    const fake = new FakeSession().answer('connector/list', {
      categories: CATEGORIES,
      connectors: [LINEAR, DEEPWIKI, { ...NOTION, status: 'connected' }],
    });
    const { fixture, root } = await page(fake);
    expect(rows(root, 'connectors-connected')).toEqual(['N Notion Connected Disconnect']);
    expect(rows(root, 'connectors-dev')).toEqual([
      'L Linear Issues, projects and cycles. Connect',
      'D DeepWiki Answers about public repositories. Connect',
    ]);
    expect(root.querySelector('[data-testid="connectors-productivity"]')).toBeNull();
    await expectAccessible(root);

    const search = root.querySelector<HTMLInputElement>('[data-testid="connectors-search"]');
    if (search === null) {
      throw new Error('no search');
    }
    search.value = 'repositories';
    search.dispatchEvent(new Event('input'));
    await settle(fixture);
    expect(rows(root, 'connectors-dev')).toEqual([
      'D DeepWiki Answers about public repositories. Connect',
    ]);
    expect(root.querySelector('[data-testid="connectors-connected"]')).toBeNull();
    search.value = 'nothing like it';
    search.dispatchEvent(new Event('input'));
    await settle(fixture);
    expect(root.querySelector('[data-testid="connectors-none"]')?.textContent).toBe(
      'No app matches that search.',
    );
  });

  it('connects one that asks no one to sign in at once, closing the tab it opened', async () => {
    let connected = false;
    const fake = new FakeSession()
      .answer('connector/list', () => ({
        categories: CATEGORIES,
        connectors: [{ ...DEEPWIKI, status: connected ? 'connected' : 'available' }],
      }))
      .answer('connector/connect', () => {
        connected = true;
        return { connector: { ...DEEPWIKI, status: 'connected' } };
      });
    const tab = fakeTab();
    const { fixture, root, said } = await page(fake);
    press(root, 'deepwiki');
    await settle(fixture);
    expect(fake.paramsOf('connector/connect')).toEqual([{ connector: 'deepwiki' }]);
    expect(tab.closed()).toBe(true);
    expect(rows(root, 'connectors-connected')).toEqual(['D DeepWiki Connected Disconnect']);
    expect(said).toEqual(['DeepWiki, Connected']);
  });

  it('signs in to one in a new tab, waits, and can be cancelled', async () => {
    const fake = new FakeSession()
      .answer('connector/list', { categories: CATEGORIES, connectors: [LINEAR] })
      .answer('connector/connect', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', async () => {
        await new Promise((resolve) => setTimeout(resolve));
        return { flow: 'f1' };
      });
    const tab = fakeTab();
    const { fixture, root, said } = await page(fake);
    press(root, 'linear');
    await settle(fixture);
    expect(tab.sentTo).toEqual([PAGE]);
    const waiting = root.querySelector('[data-testid="connector-waiting"]');
    expect(waiting?.textContent).toContain('Finish signing in to Linear in the new tab.');
    expect(said).toEqual(['Finish signing in to Linear in the new tab.']);
    await expectAccessible(root);

    press(root, 'linear');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connector-waiting"]')).toBeNull();
    expect(root.querySelector('[data-testid="connector-problem"]')).toBeNull();
    expect(rows(root, 'connectors-dev')).toEqual(['L Linear Issues, projects and cycles. Connect']);
  });

  it('asks before disconnecting, then lists them again', async () => {
    let connected = true;
    const fake = new FakeSession()
      .answer('connector/list', () => ({
        categories: CATEGORIES,
        connectors: [{ ...NOTION, status: connected ? 'connected' : 'available' }],
      }))
      .answer('connector/disconnect', () => {
        connected = false;
        return {};
      });
    const { fixture, root } = await page(fake);
    press(root, 'notion');
    await settle(fixture);
    expect(fake.paramsOf('connector/disconnect')).toEqual([]);
    document.querySelector<HTMLElement>('[data-testid="connectors-disconnect-ok"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('connector/disconnect')).toEqual([{ connector: 'notion' }]);
    expect(rows(root, 'connectors-productivity')).toEqual([
      'N Notion Pages, databases and search. Connect',
    ]);
  });
});
