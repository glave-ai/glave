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

describe('ConnectionsComponent', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows what is ready to use, then what asks to be connected, in plain words', async () => {
    const fake = new FakeSession().answer('connection/list', {
      connections: [DOCS, TRACKER, NOTES],
    });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(ConnectionsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('h1')?.textContent).toBe('Connections');
    expect(rows(root, 'connections-ready')).toEqual(['D docs Connected Disconnect']);
    expect(rows(root, 'connections-available')).toEqual([
      'IT Issue Tracker mcp.tracker.example.com Connect',
      'N notes.example.com Your sign-in expired Connect',
    ]);
    const connect = root.querySelectorAll('[data-testid="connection-action"]')[1];
    expect(connect?.getAttribute('aria-label')).toBe('Connect Issue Tracker');
    expect(root.querySelector('[data-testid="connections-loading"]')).toBeNull();
    expect(root.textContent).not.toMatch(ADMIN_OR_FILES);
    await expectAccessible(root);
  });

  it('connects in a new tab, waits there, then shows the service ready to use', async () => {
    let signedIn = false;
    let polls = 0;
    let signIn: () => void = () => undefined;
    const signedInThere = new Promise<void>((resolve) => {
      signIn = resolve;
    });
    const fake = new FakeSession()
      .answer('connection/list', () => ({
        connections: [{ ...TRACKER, status: signedIn ? 'signed_in' : 'signed_out' }],
      }))
      .answer('connection/start', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', async () => {
        polls += 1;
        if (polls < 3) {
          await new Promise((resolve) => setTimeout(resolve));
          return { flow: 'f1' };
        }
        await signedInThere;
        signedIn = true;
        return { connection: { ...TRACKER, status: 'signed_in' } };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const opened = fakeTab();
    const fixture = TestBed.createComponent(ConnectionsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;

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
    const cancel = root.querySelector('[data-testid="connection-action"]');
    expect(cancel?.textContent.trim()).toBe('Cancel');
    expect(cancel?.getAttribute('aria-label')).toBe('Cancel signing in to Issue Tracker');
    await expectAccessible(root);

    signIn();
    await settle(fixture);
    expect(polls).toBe(3);
    expect(rows(root, 'connections-ready')).toEqual(['IT Issue Tracker Connected Disconnect']);
    expect(root.querySelector('[data-testid="connections-available"]')).toBeNull();
    expect(root.querySelector('[data-testid="connection-waiting"]')).toBeNull();
    expect(opened.closed()).toBe(false);
  });

  it('stops waiting when cancelled, and can connect again', async () => {
    const fake = new FakeSession()
      .answer('connection/list', { connections: [TRACKER] })
      .answer('connection/start', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', async () => {
        await new Promise((resolve) => setTimeout(resolve));
        return { flow: 'f1' };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    fakeTab();
    const fixture = TestBed.createComponent(ConnectionsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
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

  it('says in the server’s words why a service could not be connected, and closes the tab', async () => {
    const fake = new FakeSession().answer('connection/list', { connections: [TRACKER] }).answer(
      'connection/start',
      new ApiError({
        code: ErrorCode.HOST,
        message: 'Issue Tracker asks no one to sign in: it works as it is',
      }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const opened = fakeTab();
    const fixture = TestBed.createComponent(ConnectionsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    click(root, 'connection-action');
    await settle(fixture);
    expect(root.querySelector('[data-testid="connection-problem"]')?.textContent).toBe(
      'Issue Tracker asks no one to sign in: it works as it is.',
    );
    expect(opened.closed()).toBe(true);
    expect(root.querySelector('[data-testid="connection-action"]')?.textContent.trim()).toBe(
      'Connect',
    );
    await expectAccessible(root);
  });

  it('waits even where the browser opens no tab, with the sign-in to open', async () => {
    const fake = new FakeSession()
      .answer('connection/list', { connections: [TRACKER] })
      .answer('connection/start', { sign_in: { flow: 'f1', url: PAGE } })
      .answer('connection/poll', async () => {
        await new Promise((resolve) => setTimeout(resolve));
        return { flow: 'f1' };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    vi.spyOn(window, 'open').mockReturnValue(null);
    const fixture = TestBed.createComponent(ConnectionsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
    click(root, 'connection-action');
    await settle(fixture);
    const again = root.querySelector('[data-testid="connection-open"]');
    expect(again?.getAttribute('href')).toBe(PAGE);
    expect(again?.textContent).toContain('Open it again');
    fixture.destroy();
  });

  it('disconnects once asked, and says why when it could not', async () => {
    let kept = true;
    let refuse = true;
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
        kept = false;
        return {};
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(ConnectionsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
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
    expect(fake.paramsOf('connection/remove').at(-1)).toEqual({ server: DOCS.url });
    expect(rows(root, 'connections-available')).toEqual(['D docs mcp.docs.example.com Connect']);
    expect(root.querySelector('[data-testid="connection-problem"]')).toBeNull();
  });

  it('says when there is nothing to connect, and why the list could not be read', async () => {
    const fake = new FakeSession().answer('connection/list', { connections: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const fixture = TestBed.createComponent(ConnectionsComponent);
    await settle(fixture);
    const root: HTMLElement = fixture.nativeElement;
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
    TestBed.configureTestingModule({ providers: provideFakeSession(refusing) });
    const refused = TestBed.createComponent(ConnectionsComponent);
    await settle(refused);
    const page: HTMLElement = refused.nativeElement;
    expect(page.querySelector('[data-testid="connections-problem"]')?.textContent).toBe(
      'Connections are signed in to on the computer glave runs on, by its person.',
    );
    expect(page.querySelector('[data-testid="connections-loading"]')).toBeNull();
    expect(page.querySelector('[data-testid="connections-empty"]')).toBeNull();
  });
});
