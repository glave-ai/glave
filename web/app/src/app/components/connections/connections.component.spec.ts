import { LiveAnnouncer } from '@angular/cdk/a11y';
import { TestBed } from '@angular/core/testing';
import { ApiError, type ConnectionInfo, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { ConnectionsComponent } from './connections.component';

const DOCS: ConnectionInfo = {
  url: 'https://mcp.docs.example.com/mcp',
  name: 'docs',
  status: 'signed_in',
};
const TRACKER: ConnectionInfo = {
  url: 'https://mcp.tracker.example.com/mcp',
  name: 'Issue Tracker',
  status: 'signed_out',
};
const NOTES: ConnectionInfo = { url: 'https://notes.example.com/mcp', status: 'expired' };

const PAGE = 'https://auth.example.com/authorize?state=s1';

/** Words a person on their own should never meet here. */
const ADMIN_OR_FILES = /MCP|mcp\.json|policy|organization|admin|config|toml/u;

/** Lets the component's promises and the page settle. */
async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

function click(root: ParentNode, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

/** The rows of the card `testId`, each as its tile, name, line and button's word. */
function rows(root: ParentNode, testId: string): readonly string[] {
  const card = root.querySelector(`[data-testid="${testId}"]`);
  const found = card?.querySelectorAll('[data-testid="connection"]') ?? [];
  const parts = '.tile, .name, .detail, [data-testid="connection-action"]';
  return [...found].map((row) =>
    [...row.querySelectorAll(parts)].map((part) => part.textContent.trim()).join(' '),
  );
}

/** A tab the page opens, standing in for the browser's. */
function fakeTab(): { readonly tab: Window; readonly sentTo: string[]; closed: () => boolean } {
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
  return { tab, sentTo, closed: () => closed };
}

/** The page on `fake`, and what it says to screen readers. */
async function page(fake: FakeSession): Promise<{
  readonly fixture: { whenStable(): Promise<unknown>; destroy(): void };
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
  const fixture = TestBed.createComponent(ConnectionsComponent);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement, said };
}

/** Polls the fake answers while waiting: a tick each, the flow again. */
async function stillWaiting(): Promise<object> {
  await new Promise((resolve) => setTimeout(resolve));
  return { flow: 'f1' };
}

describe('ConnectionsComponent', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows what is ready to use, then what asks to be connected, in plain words', async () => {
    const fake = new FakeSession().answer('connection/list', {
      connections: [DOCS, TRACKER, NOTES],
    });
    const { root } = await page(fake);
    expect(root.querySelector('h1')?.textContent).toBe('Connections');
    expect(rows(root, 'connections-ready')).toEqual(['D docs Connected Disconnect']);
    expect(rows(root, 'connections-available')).toEqual([
      'IT Issue Tracker mcp.tracker.example.com Connect',
      'N notes.example.com Your sign-in expired Connect',
    ]);
    const connect = root.querySelectorAll('[data-testid="connection-action"]')[1];
    expect(connect?.getAttribute('aria-label')).toBe('Connect Issue Tracker in a new tab');
    expect(root.querySelector('[data-testid="connections-loading"]')).toBeNull();
    expect(root.textContent).not.toMatch(ADMIN_OR_FILES);
    await expectAccessible(root);
  });

  it('connects in a new tab, waits there, then shows the service ready to use', async () => {
    let polls = 0;
    let signIn: () => void = () => undefined;
    const signedInThere = new Promise<void>((resolve) => {
      signIn = resolve;
    });
    const fake = new FakeSession()
      .answer('connection/list', { connections: [TRACKER] })
      .answer('connection/start', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', async () => {
        polls += 1;
        if (polls < 3) {
          return stillWaiting();
        }
        await signedInThere;
        return { connection: { ...TRACKER, status: 'signed_in' } };
      });
    const opened = fakeTab();
    const { fixture, root, said } = await page(fake);

    click(root, 'connection-action');
    expect(window.open).toHaveBeenCalledWith('', '_blank');
    expect(opened.tab.opener).toBeNull();
    await settle(fixture);
    expect(fake.paramsOf('connection/start')).toEqual([{ server: TRACKER.url }]);
    expect(opened.sentTo).toEqual([PAGE]);
    const waiting = root.querySelector('[data-testid="connection-waiting"]');
    expect(waiting?.textContent).toContain('Finish signing in to Issue Tracker in the new tab.');
    expect(waiting?.querySelector('a')?.getAttribute('href')).toBe(PAGE);
    expect(waiting?.querySelector('a')?.getAttribute('target')).toBe('_blank');
    expect(waiting?.querySelector('a')?.textContent).toContain('Open it again');
    const cancel = root.querySelector('[data-testid="connection-action"]');
    expect(cancel?.textContent.trim()).toBe('Cancel');
    expect(cancel?.getAttribute('aria-label')).toBe('Cancel signing in to Issue Tracker');
    expect(said).toEqual(['Finish signing in to Issue Tracker in the new tab.']);
    await expectAccessible(root);

    signIn();
    await settle(fixture);
    expect(polls).toBe(3);
    expect(rows(root, 'connections-ready')).toEqual(['IT Issue Tracker Connected Disconnect']);
    expect(root.querySelector('[data-testid="connections-available"]')).toBeNull();
    expect(fake.paramsOf('connection/list')).toHaveLength(1);
    expect(said.at(-1)).toBe('Issue Tracker, Connected');
    expect(document.activeElement?.getAttribute('aria-label')).toBe('Disconnect Issue Tracker');
    expect(opened.closed()).toBe(false);
  });

  it('stops waiting when cancelled, and can connect again', async () => {
    const fake = new FakeSession()
      .answer('connection/list', { connections: [TRACKER] })
      .answer('connection/start', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', stillWaiting);
    fakeTab();
    const { fixture, root } = await page(fake);
    click(root, 'connection-action');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connection-waiting"]')).not.toBeNull();

    click(root, 'connection-action');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connection-waiting"]')).toBeNull();
    expect(rows(root, 'connections-available')).toEqual([
      'IT Issue Tracker mcp.tracker.example.com Connect',
    ]);
    const polled = fake.paramsOf('connection/poll').length;
    await settle(fixture);
    expect(fake.paramsOf('connection/poll')).toHaveLength(polled);
    expect(root.querySelector('[data-testid="connection-problem"]')).toBeNull();
  });

  it('closes the tab when cancelled before the sign-in began', async () => {
    let begin: () => void = () => undefined;
    const begun = new Promise<void>((resolve) => {
      begin = resolve;
    });
    const fake = new FakeSession()
      .answer('connection/list', { connections: [TRACKER] })
      .answer('connection/start', async () => {
        await begun;
        return { sign_in: { flow: 'f1', url: PAGE } };
      });
    const opened = fakeTab();
    const { fixture, root } = await page(fake);
    click(root, 'connection-action');
    await settle(fixture);
    click(root, 'connection-action');
    begin();
    await settle(fixture);
    expect(opened.closed()).toBe(true);
    expect(opened.sentTo).toEqual([]);
    expect(fake.paramsOf('connection/poll')).toEqual([]);
  });

  it('stops waiting when the page is left', async () => {
    const fake = new FakeSession()
      .answer('connection/list', { connections: [TRACKER] })
      .answer('connection/start', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', stillWaiting);
    fakeTab();
    const { fixture, root } = await page(fake);
    click(root, 'connection-action');
    await settle(fixture);
    fixture.destroy();
    const polled = fake.paramsOf('connection/poll').length;
    await settle(fixture);
    expect(fake.paramsOf('connection/poll')).toHaveLength(polled);
  });

  it('says in the server’s words why a service could not be connected, and closes the tab', async () => {
    const fake = new FakeSession().answer('connection/list', { connections: [TRACKER] }).answer(
      'connection/start',
      new ApiError({
        code: ErrorCode.HOST,
        message: 'glave could not reach the sign-in for Issue Tracker: try again in a while',
      }),
    );
    const opened = fakeTab();
    const { fixture, root } = await page(fake);
    click(root, 'connection-action');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connection-problem"]')?.textContent).toBe(
      'Glave could not reach the sign-in for Issue Tracker: try again in a while.',
    );
    expect(opened.closed()).toBe(true);
    expect(root.querySelector('[data-testid="connection-action"]')?.textContent.trim()).toBe(
      'Connect',
    );
    await expectAccessible(root);
  });

  it('where the browser opens no tab, waits with the sign-in to open', async () => {
    const fake = new FakeSession()
      .answer('connection/list', { connections: [TRACKER] })
      .answer('connection/start', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', stillWaiting);
    vi.spyOn(window, 'open').mockReturnValue(null);
    const { fixture, root, said } = await page(fake);
    click(root, 'connection-action');
    await settle(fixture);
    const waiting = root.querySelector('[data-testid="connection-waiting"]');
    expect(waiting?.textContent).toContain('To finish, open the sign-in for Issue Tracker.');
    const link = root.querySelector('[data-testid="connection-open"]');
    expect(link?.getAttribute('href')).toBe(PAGE);
    expect(link?.textContent).toContain('Open the sign-in');
    expect(said).toEqual(['To finish, open the sign-in for Issue Tracker.']);
    fixture.destroy();
  });

  it('disconnects once asked, and says why when it could not', async () => {
    let kept = true;
    let refuse = true;
    let release: () => void = () => undefined;
    const fake = new FakeSession()
      .answer('connection/list', () => ({
        connections: [{ ...DOCS, status: kept ? 'signed_in' : 'signed_out' }],
      }))
      .answer('connection/remove', () => {
        if (refuse) {
          return new ApiError({
            code: ErrorCode.NO_CREDENTIAL,
            message: 'nothing at https://mcp.docs.example.com/mcp is signed in to here',
          });
        }
        return new Promise<object>((resolve) => {
          release = () => {
            kept = false;
            resolve({});
          };
        });
      });
    const { fixture, root, said } = await page(fake);
    click(root, 'connection-action');
    await settle(fixture);
    expect(document.querySelector('[data-testid="connections-disconnect"]')?.textContent).toContain(
      'Disconnect docs?',
    );
    click(document, 'connections-disconnect-cancel');
    await settle(fixture);
    expect(fake.paramsOf('connection/remove')).toEqual([]);

    click(root, 'connection-action');
    await settle(fixture);
    click(document, 'connections-disconnect-ok');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connection-problem"]')?.textContent).toBe(
      'Nothing at https://mcp.docs.example.com/mcp is signed in to here.',
    );

    refuse = false;
    click(root, 'connection-action');
    await settle(fixture);
    click(document, 'connections-disconnect-ok');
    await settle(fixture);
    const busy = root.querySelector<HTMLButtonElement>('[data-testid="connection-action"]');
    expect(busy?.textContent.trim()).toBe('Disconnecting…');
    expect(busy?.disabled).toBe(true);
    release();
    await settle(fixture);
    expect(fake.paramsOf('connection/remove').at(-1)).toEqual({ server: DOCS.url });
    expect(rows(root, 'connections-available')).toEqual(['D docs mcp.docs.example.com Connect']);
    expect(root.querySelector('[data-testid="connection-problem"]')).toBeNull();
    expect(said.at(-1)).toBe('docs, Disconnected');
  });

  it('shows how often each connection was called this month, in the server’s words', async () => {
    const used = { ...DOCS, usage: { calls: 2, sentence: '2 calls this month' } };
    const fake = new FakeSession().answer('connection/list', { connections: [used] });
    const { root } = await page(fake);
    expect(rows(root, 'connections-ready')).toEqual([
      'D docs Connected 2 calls this month Disconnect',
    ]);
    await expectAccessible(root);
  });

  it('says when there is nothing to connect, and why the list could not be read', async () => {
    const fake = new FakeSession().answer('connection/list', { connections: [] });
    const { root } = await page(fake);
    expect(root.querySelector('[data-testid="connections-empty"]')?.textContent).toContain(
      'Nothing to connect yet',
    );
    await expectAccessible(root);

    TestBed.resetTestingModule();
    const refusing = new FakeSession().answer(
      'connection/list',
      new ApiError({
        code: ErrorCode.NOT_ALLOWED,
        message: 'connections are signed in to on the computer glave runs on, by its person',
      }),
    );
    const refused = await page(refusing);
    expect(refused.root.querySelector('[data-testid="connections-problem"]')?.textContent).toBe(
      'Connections are signed in to on the computer glave runs on, by its person.',
    );
    expect(refused.root.querySelector('[data-testid="connections-loading"]')).toBeNull();
    expect(refused.root.querySelector('[data-testid="connections-empty"]')).toBeNull();
  });
});
