import { TestBed } from '@angular/core/testing';
import type { ConnectorInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import type { SuggestionItem } from '../../lib/conversation';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { announced, fakeTab, stillWaiting } from '../../testing/sign-in-tab';
import { ConnectCardComponent } from './connect-card.component';

/** A connector of the catalog, not connected; `dcr` asks the person to sign in. */
function connector(id: string, name: string, sign_in: ConnectorInfo['sign_in']): ConnectorInfo {
  const url = `https://mcp.${id}.example.com/mcp`;
  return {
    id,
    name,
    company: id,
    url,
    categories: ['dev'],
    tags: ['issues'],
    returns: 'text',
    sign_in,
    description: 'Issues, projects and cycles.',
    docs: url,
    status: 'available',
  };
}

const LINEAR = connector('linear', 'Linear', 'dcr');
const DEEPWIKI = connector('deepwiki', 'DeepWiki', 'none');
const PAGE = 'https://auth.example.com/authorize?state=s1';
const EVERYWHERE = 'Everywhere, including chats';
const TOLD =
  "Linear isn't connected. Connect it to let me use it for you. You sign in on Linear's own page.";

/** `info` connected everywhere, signed in as `who` where the service says. */
function connected(info: ConnectorInfo, who: string | null = null): ConnectorInfo {
  return {
    ...info,
    status: 'connected',
    accounts: [{ status: 'connected', used: EVERYWHERE, ...(who === null ? {} : { who }) }],
  };
}

/** The suggestion of the connector `id`, as a thread holds it. */
function suggestion(id: string, told = TOLD): SuggestionItem {
  return { kind: 'suggestion', key: 'suggestion-7', seq: 7, connector: id, told };
}

/** Lets the card's promises and the page settle. */
async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

/** The card for `item`, the catalog `fake` lists, and what it says to screen readers. */
async function card(
  fake: FakeSession,
  item: SuggestionItem,
): Promise<{
  readonly fixture: { whenStable(): Promise<unknown> };
  readonly root: HTMLElement;
  readonly said: string[];
}> {
  const { said, provider } = announced();
  TestBed.configureTestingModule({ providers: [...provideFakeSession(fake), provider] });
  const fixture = TestBed.createComponent(ConnectCardComponent);
  fixture.componentRef.setInput('item', item);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement, said };
}

/** The text of the part `testId` of `root`. */
function part(root: ParentNode, testId: string): string | undefined {
  return root.querySelector(`[data-testid="${testId}"]`)?.textContent.trim();
}

function press(root: ParentNode, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

describe('ConnectCardComponent', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("offers to connect it, in the server's words, and puts it off in this tab", async () => {
    const fake = new FakeSession().answer('connector/list', {
      categories: [],
      connectors: [LINEAR],
    });
    const { fixture, root } = await card(fake, suggestion('linear'));
    expect(root.querySelector('.tile')?.textContent).toBe('L');
    expect(part(root, 'connect-card-name')).toBe('Linear');
    expect(part(root, 'connect-card-description')).toBe('Issues, projects and cycles.');
    expect(part(root, 'connect-card-told')).toBe(TOLD);
    expect(part(root, 'connect-card-connect')).toBe('Connect Linear');
    expect(part(root, 'connect-card-not-now')).toBe('Not now');
    await expectAccessible(root);

    press(root, 'connect-card-not-now');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connect-card"]')).toBeNull();
    expect(fake.paramsOf('connector/connect')).toEqual([]);
  });

  it('draws nothing for a connector the server does not list', async () => {
    const fake = new FakeSession().answer('connector/list', { categories: [], connectors: [] });
    const { root } = await card(fake, suggestion('linear'));
    expect(root.textContent.trim()).toBe('');
  });

  it('connects one that asks no one to sign in everywhere, then shrinks to a line', async () => {
    let done = false;
    const fake = new FakeSession()
      .answer('connector/list', () => ({
        categories: [],
        connectors: [done ? connected(DEEPWIKI) : DEEPWIKI],
      }))
      .answer('connector/connect', () => {
        done = true;
        return { connector: connected(DEEPWIKI) };
      });
    const tab = fakeTab();
    vi.spyOn(window, 'open').mockReturnValue(tab.tab);
    const { fixture, root, said } = await card(fake, suggestion('deepwiki', 'DeepWiki …'));
    press(root, 'connect-card-connect');
    await settle(fixture);
    expect(fake.paramsOf('connector/connect')).toEqual([{ connector: 'deepwiki' }]);
    expect(tab.closed()).toBe(true);
    expect(root.querySelector('[data-testid="connect-card"]')).toBeNull();
    expect(part(root, 'connect-card-state')).toBe('DeepWiki connected');
    expect(part(root, 'connect-card-used')).toBe(EVERYWHERE);
    expect(said).toEqual(['DeepWiki connected']);
    await expectAccessible(root);
  });

  it('signs in in a new tab, waits, and says who it is connected as', async () => {
    let signedIn = false;
    let polls = 0;
    const fake = new FakeSession()
      .answer('connector/list', () => ({
        categories: [],
        connectors: [signedIn ? connected(LINEAR, 'ada@example.com') : LINEAR],
      }))
      .answer('connector/connect', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', async () => {
        polls += 1;
        if (polls < 3) {
          return stillWaiting();
        }
        signedIn = true;
        return { connection: { url: PAGE, status: 'signed_in' } };
      });
    const tab = fakeTab();
    vi.spyOn(window, 'open').mockReturnValue(tab.tab);
    const { fixture, root, said } = await card(fake, suggestion('linear'));
    press(root, 'connect-card-connect');
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(tab.sentTo).toEqual([PAGE]);
    expect(part(root, 'connect-card-waiting')).toBe('Finish signing in to Linear in the new tab.');
    await settle(fixture);
    expect(part(root, 'connect-card-state')).toBe('Linear connected as ada@example.com');
    expect(said).toEqual([
      'Finish signing in to Linear in the new tab.',
      'Linear connected as ada@example.com',
    ]);
    expect(tab.closed()).toBe(false);
  });

  it('offers to sign in again to one whose sign-in stopped working, where its account is used', async () => {
    let signedIn = false;
    const expired: ConnectorInfo = {
      ...LINEAR,
      status: 'sign_in_again',
      accounts: [{ name: 'Acme', only: 'p1', status: 'sign_in_again', used: 'Only in Acme Site' }],
    };
    const fake = new FakeSession()
      .answer('connector/list', () => ({
        categories: [],
        connectors: [signedIn ? connected(LINEAR) : expired],
      }))
      .answer('connector/connect', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', () => {
        signedIn = true;
        return { connection: { url: PAGE, status: 'signed_in' } };
      });
    const tab = fakeTab();
    vi.spyOn(window, 'open').mockReturnValue(tab.tab);
    const { fixture, root } = await card(fake, suggestion('linear'));
    // In the Connections page's words, not the suggestion's, which no longer holds.
    expect(part(root, 'connect-card-told')).toBe('Sign in again to keep using it');
    expect(part(root, 'connect-card-connect')).toBe('Sign in again');
    await expectAccessible(root);

    press(root, 'connect-card-connect');
    await settle(fixture);
    expect(fake.paramsOf('connector/connect')).toEqual([
      { connector: 'linear', account: 'Acme', only: 'p1' },
    ]);
    expect(tab.sentTo).toEqual([PAGE]);
    expect(part(root, 'connect-card-state')).toBe('Linear connected');
  });
});
