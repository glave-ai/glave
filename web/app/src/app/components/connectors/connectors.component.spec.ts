import { TestBed } from '@angular/core/testing';
import { ApiError, type ConnectorInfo, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ProjectsService } from '../../services/projects.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { record } from '../../testing/fixtures';
import { type FakeTab, announced, fakeTab, stillWaiting } from '../../testing/sign-in-tab';
import { ConnectorsComponent } from './connectors.component';

/** A connector of the catalog, `available` unless said. */
function connector(
  id: string,
  name: string,
  home: ConnectorInfo['categories'][number],
  description: string,
  status: ConnectorInfo['status'] = 'available',
): ConnectorInfo {
  const url = `https://mcp.${id}.example.com/mcp`;
  return {
    id,
    name,
    company: id,
    url,
    categories: [home],
    tags: ['issues'],
    returns: 'text',
    sign_in: 'dcr',
    description,
    docs: url,
    status,
  };
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

/** Opens the menu `testId` and picks its option that says `text`. */
async function choose(
  fixture: { whenStable(): Promise<unknown> },
  testId: string,
  text: string,
): Promise<void> {
  await settle(fixture);
  document.querySelector<HTMLElement>(`[data-testid="${testId}-trigger"]`)?.click();
  await fixture.whenStable();
  const options = document.querySelectorAll<HTMLElement>(`[data-testid="${testId}-option"]`);
  [...options].find((option) => option.textContent.includes(text))?.click();
  await settle(fixture);
}

/** Presses the button of the connector `id`. */
function press(root: ParentNode, id: string): void {
  root
    .querySelector(`[data-connector="${id}"] [data-testid="connector-action"]`)
    ?.dispatchEvent(new MouseEvent('click'));
}

async function page(
  fake: FakeSession,
  project: string | null = null,
): Promise<{
  readonly fixture: { whenStable(): Promise<unknown>; detectChanges(): void };
  readonly root: HTMLElement;
  readonly said: string[];
}> {
  const { said, provider } = announced();
  TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), provider] });
  const fixture = TestBed.createComponent(ConnectorsComponent);
  fixture.componentRef.setInput('project', project);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement, said };
}

/** A fake tab, which `window.open` gives. */
function openedTab(): FakeTab {
  const tab = fakeTab();
  vi.spyOn(window, 'open').mockReturnValue(tab.tab);
  return tab;
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
    const tab = openedTab();
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
      .answer('connection/poll', stillWaiting);
    const tab = openedTab();
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
    expect(tab.closed()).toBe(false);
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

  it('shows the sign-in as a link where no tab opened', async () => {
    const fake = new FakeSession()
      .answer('connector/list', { categories: CATEGORIES, connectors: [LINEAR] })
      .answer('connector/connect', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', stillWaiting);
    vi.spyOn(window, 'open').mockReturnValue(null);
    const { fixture, root } = await page(fake);
    press(root, 'linear');
    await settle(fixture);
    const waiting = root.querySelector('[data-testid="connector-waiting"]');
    expect(waiting?.textContent).toContain('To finish, open the sign-in for Linear.');
    const link = root.querySelector('[data-testid="connector-open"]');
    expect(link?.getAttribute('href')).toBe(PAGE);
    expect(link?.textContent).toContain('Open the sign-in');
    await expectAccessible(root);
  });

  it("says why connecting failed, in the server's words, and closes the tab it opened", async () => {
    const fake = new FakeSession()
      .answer('connector/list', { categories: CATEGORIES, connectors: [LINEAR] })
      .answer(
        'connector/connect',
        new ApiError({
          code: ErrorCode.HOST,
          message: 'glave could not reach the sign-in for Linear: try again in a while',
        }),
      );
    const tab = openedTab();
    const { fixture, root } = await page(fake);
    press(root, 'linear');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connector-problem"]')?.textContent).toBe(
      'Glave could not reach the sign-in for Linear: try again in a while.',
    );
    expect(tab.closed()).toBe(true);
  });

  it('offers to sign in again, or to disconnect, one whose sign-in stopped working', async () => {
    const fake = new FakeSession()
      .answer('connector/list', {
        categories: CATEGORIES,
        connectors: [{ ...LINEAR, status: 'sign_in_again' }],
      })
      .answer('connector/disconnect', {});
    const { fixture, root } = await page(fake);
    expect(rows(root, 'connectors-connected')).toEqual([
      'L Linear Sign in again to keep using it Sign in again',
    ]);
    root.querySelector<HTMLElement>('[data-testid="connector-disconnect"]')?.click();
    await settle(fixture);
    document.querySelector<HTMLElement>('[data-testid="connectors-disconnect-ok"]')?.click();
    await settle(fixture);
    expect(fake.paramsOf('connector/disconnect')).toEqual([{ connector: 'linear' }]);
    await expectAccessible(root);
  });

  it('says where each connected one is used, and moves it to one place or every place', async () => {
    let only: string | null = null;
    const used = (): ConnectorInfo => ({
      ...NOTION,
      status: 'connected',
      accounts: [
        only === null
          ? { status: 'connected', used: 'Everywhere, including chats' }
          : { status: 'connected', only, used: 'Only in Acme Site' },
      ],
    });
    const fake = new FakeSession()
      .answer('project/list', {
        projects: [record('p1', 'Acme Site'), record('p2', 'Chats', 'personal')],
      })
      .answer('thread/list', { threads: [] })
      .answer('connector/list', () => ({ categories: CATEGORIES, connectors: [used()] }))
      .answer('connector/place', (params) => {
        only = (params as { only?: string }).only ?? null;
        return { connector: used() };
      });
    const { fixture, root } = await page(fake);
    await TestBed.inject(ProjectsService).refresh();
    await settle(fixture);
    expect(rows(root, 'connectors-connected')).toEqual([
      'N Notion Everywhere, including chats Disconnect',
    ]);
    const place = root.querySelector('[data-testid="connector-place-trigger"]');
    expect(place?.textContent).toContain('Where Notion is used');
    await expectAccessible(root);
    await choose(fixture, 'connector-place', 'Only in Acme Site');
    expect(fake.paramsOf('connector/place')).toEqual([{ connector: 'notion', only: 'p1' }]);
    expect(rows(root, 'connectors-connected')).toEqual(['N Notion Only in Acme Site Disconnect']);
    await choose(fixture, 'connector-place', 'Everywhere, including chats');
    expect(fake.paramsOf('connector/place')).toEqual([
      { connector: 'notion', only: 'p1' },
      { connector: 'notion' },
    ]);
  });

  it('for a project, lists what its threads use and connects one only there', async () => {
    const fake = new FakeSession()
      .answer('project/list', {
        projects: [record('p1', 'Acme Site'), record('p2', 'Chats', 'personal')],
      })
      .answer('thread/list', { threads: [] })
      .answer('connector/list', (params) =>
        (params as { used_in?: string }).used_in === 'p1'
          ? { categories: CATEGORIES, connectors: [{ ...NOTION, status: 'connected' }] }
          : {
              categories: CATEGORIES,
              connectors: [
                DEEPWIKI,
                { ...NOTION, status: 'connected' },
                {
                  ...LINEAR,
                  status: 'connected',
                  accounts: [{ status: 'connected', only: 'p3', used: 'Only in Beta' }],
                },
              ],
            },
      )
      .answer('connector/connect', { connector: { ...DEEPWIKI, status: 'connected' } });
    openedTab();
    const { fixture, root } = await page(fake, 'p1');
    await TestBed.inject(ProjectsService).refresh();
    await settle(fixture);
    const heading = root.querySelector('[data-testid="connectors-connected"] h3');
    expect(heading?.textContent).toBe('Used in Acme Site');
    expect(rows(root, 'connectors-connected')).toEqual(['N Notion Connected Disconnect']);
    expect(rows(root, 'connectors-elsewhere')).toEqual(['L Linear Only in Beta Disconnect']);
    expect(root.querySelector('[data-testid="connectors-here"]')?.textContent).toContain(
      'Apps you connect here are used only in Acme Site.',
    );
    press(root, 'deepwiki');
    await settle(fixture);
    expect(fake.paramsOf('connector/connect')).toEqual([{ connector: 'deepwiki', only: 'p1' }]);
    await expectAccessible(root);
  });

  /** The page for a person with only their Chats, Notion connected with `accounts`. */
  async function onlyChats(accounts: NonNullable<ConnectorInfo['accounts']>): Promise<HTMLElement> {
    const fake = new FakeSession()
      .answer('project/list', { projects: [record('p2', 'Chats', 'personal')] })
      .answer('thread/list', { threads: [] })
      .answer('connector/list', {
        categories: CATEGORIES,
        connectors: [{ ...NOTION, status: 'connected', accounts }],
      });
    const { fixture, root } = await page(fake);
    await TestBed.inject(ProjectsService).refresh();
    await settle(fixture);
    return root;
  }

  it('offers no choice of place to a person with only their Chats', async () => {
    const root = await onlyChats([{ status: 'connected', used: 'Everywhere, including chats' }]);
    expect(rows(root, 'connectors-connected')).toEqual([
      'N Notion Everywhere, including chats Disconnect',
    ]);
    expect(root.querySelector('[data-testid="connector-place-trigger"]')).toBeNull();
  });

  it('lists two accounts beneath their service, each with its own menu', async () => {
    const two: ConnectorInfo = {
      ...NOTION,
      status: 'connected',
      accounts: [
        { name: 'Glave', status: 'connected', used: 'Everywhere, including chats' },
        { name: 'Acme', status: 'sign_in_again', only: 'p1', used: 'Only in Acme Site' },
      ],
    };
    const fake = new FakeSession()
      .answer('project/list', {
        projects: [record('p1', 'Acme Site'), record('p2', 'Chats', 'personal')],
      })
      .answer('thread/list', { threads: [] })
      .answer('connector/list', { categories: CATEGORIES, connectors: [two] })
      .answer('connector/place', { connector: two })
      .answer('connector/connect', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', { connection: { url: PAGE, status: 'signed_in' } });
    openedTab();
    const { fixture, root } = await page(fake);
    await TestBed.inject(ProjectsService).refresh();
    await settle(fixture);
    expect(rows(root, 'connectors-connected')).toEqual(['N Notion 2 accounts']);
    const beneath = [...root.querySelectorAll('[data-testid="connector-account"]')].map((it) =>
      [...it.querySelectorAll('.label, .used')].map((part) => part.textContent.trim()).join(' '),
    );
    expect(beneath).toEqual(['Glave Everywhere, including chats', 'Acme Only in Acme Site']);
    expect(root.querySelector('[data-testid="connector-place-trigger"]')).toBeNull();
    await expectAccessible(root);

    const menus = root.querySelectorAll<HTMLElement>(
      '[data-testid="connector-account-menu-trigger"]',
    );
    expect(menus[0]?.getAttribute('aria-label')).toBe('Options for Notion Glave');
    const pick = async (at: number, text: string): Promise<void> => {
      menus[at]?.click();
      await fixture.whenStable();
      const items = document.querySelectorAll<HTMLElement>(
        '[data-testid="connector-account-menu-action"]',
      );
      [...items].find((item) => item.textContent.includes(text))?.click();
      await settle(fixture);
    };
    await pick(0, 'Only in Acme Site');
    expect(fake.paramsOf('connector/place')).toEqual([
      { connector: 'notion', account: 'Glave', only: 'p1' },
    ]);
    await pick(1, 'Sign in again');
    expect(fake.paramsOf('connector/connect')).toEqual([
      { connector: 'notion', account: 'Acme', only: 'p1' },
    ]);
    await pick(1, 'Disconnect');
    const heading = document.querySelector('[data-testid="connectors-disconnect"] h2');
    expect(heading?.textContent).toContain('Disconnect Notion (Acme)?');
  });

  it('adds another account of a service, named, where the page is', async () => {
    const one: ConnectorInfo = {
      ...NOTION,
      status: 'connected',
      accounts: [{ status: 'connected', used: 'Everywhere, including chats' }],
    };
    const fake = new FakeSession()
      .answer('project/list', {
        projects: [record('p1', 'Acme Site'), record('p2', 'Chats', 'personal')],
      })
      .answer('thread/list', { threads: [] })
      .answer('connector/list', { categories: CATEGORIES, connectors: [one] })
      .answer('connector/connect', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', { connection: { url: PAGE, status: 'signed_in' } });
    openedTab();
    const { fixture, root } = await page(fake, 'p1');
    await TestBed.inject(ProjectsService).refresh();
    await settle(fixture);
    root
      .querySelector<HTMLElement>('[data-connector="notion"] [data-testid="connector-add-account"]')
      ?.click();
    await settle(fixture);
    const dialog = document.querySelector('[data-testid="connector-add"]');
    expect(dialog?.textContent).toContain('Connect Notion');
    expect(dialog?.textContent).toContain('Already connected');
    expect(dialog?.textContent).toContain('Sign in to another Notion account');
    expect(dialog?.textContent).toContain('Nothing else can see it');
    const done = document.querySelector<HTMLElement>('[data-testid="connector-add-done"]');
    expect(done?.textContent.trim()).toBe('Sign in to Notion');
    done?.click();
    await settle(fixture);
    const problem = document.querySelector('[data-testid="connector-add-problem"]');
    expect(problem?.textContent).toContain('Give this account a name.');
    expect(fake.paramsOf('connector/connect')).toEqual([]);
    const name = document.querySelector<HTMLInputElement>('[data-testid="connector-add-name"]');
    if (name === null) {
      throw new Error('no name field');
    }
    name.value = ' Acme ';
    name.dispatchEvent(new Event('input'));
    done?.click();
    await settle(fixture);
    expect(fake.paramsOf('connector/connect')).toEqual([
      { connector: 'notion', account: 'Acme', only: 'p1' },
    ]);
  });

  it('for a project, names the account its threads use and why the other is not', async () => {
    const fake = new FakeSession()
      .answer('project/list', {
        projects: [record('p1', 'Acme Site'), record('p2', 'Chats', 'personal')],
      })
      .answer('thread/list', { threads: [] })
      .answer('connector/list', (params) => {
        const own = { name: 'Acme', status: 'connected', only: 'p1', used: 'Only in Acme Site' };
        const glave = { name: 'Glave', status: 'connected', used: 'Everywhere, including chats' };
        return (params as { used_in?: string }).used_in === 'p1'
          ? {
              categories: CATEGORIES,
              connectors: [
                {
                  ...NOTION,
                  status: 'connected',
                  accounts: [own],
                  replaced: 'Your Notion account used everywhere else is not used here.',
                },
              ],
            }
          : {
              categories: CATEGORIES,
              connectors: [{ ...NOTION, status: 'connected', accounts: [glave, own] }],
            };
      });
    const { fixture, root } = await page(fake, 'p1');
    await TestBed.inject(ProjectsService).refresh();
    await settle(fixture);
    expect(rows(root, 'connectors-connected')).toEqual([
      'N Notion: Acme Only in Acme Site Disconnect',
    ]);
    const note = root.querySelector('[data-connector="notion"] [data-testid="connector-note"]');
    expect(note?.textContent.trim()).toBe(
      'Your Notion account used everywhere else is not used here.',
    );
    expect(note?.classList.contains('shown')).toBe(true);
    await expectAccessible(root);
  });
});
