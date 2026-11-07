import { TestBed } from '@angular/core/testing';
import type { ConnectorInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import {
  type ConnectorAction,
  ConnectorRowComponent,
  type Waiting,
} from './connector-row.component';

const NOTION: ConnectorInfo = {
  id: 'notion',
  name: 'Notion',
  company: 'notion',
  url: 'https://mcp.notion.example.com/mcp',
  categories: ['productivity'],
  tags: ['issues'],
  returns: 'text',
  sign_in: 'dcr',
  description: 'Pages, databases and search.',
  docs: 'https://mcp.notion.example.com/mcp',
  status: 'available',
};
const PLACES = [
  { value: 'everywhere', label: 'Everywhere, including chats', short: 'Change' },
  { value: 'p1', label: 'Only in Acme Site', short: 'Change' },
];
const PAGE = 'https://auth.example.com/authorize?state=s1';

/** What the row is drawn from, beside its connector. */
interface Drawn {
  readonly waiting?: Waiting;
  readonly working?: boolean;
  readonly problem?: string;
  readonly choosing?: boolean;
}

/** `connector`'s row, in a list as the page puts it, and what it asked of the page. */
async function drawn(
  connector: ConnectorInfo,
  { waiting, working = false, problem, choosing = true }: Drawn = {},
): Promise<{ readonly root: HTMLElement; readonly acted: ConnectorAction[] }> {
  const fixture = TestBed.createComponent(ConnectorRowComponent);
  fixture.componentRef.setInput('connector', connector);
  fixture.componentRef.setInput('waiting', waiting ?? null);
  fixture.componentRef.setInput('working', working);
  fixture.componentRef.setInput('problem', problem ?? null);
  fixture.componentRef.setInput('choosing', choosing);
  fixture.componentRef.setInput('places', PLACES);
  const acted: ConnectorAction[] = [];
  fixture.componentInstance.acted.subscribe((it) => acted.push(it));
  await fixture.whenStable();
  const root = fixture.nativeElement as HTMLElement;
  const list = document.createElement('div');
  list.setAttribute('role', 'list');
  list.append(root);
  document.body.append(list);
  onTestFinished(() => {
    list.remove();
  });
  return { root, acted };
}

/** The row's tile, name, line and button's word. */
function line(root: ParentNode): string {
  const parts = '.tile, .name, .detail, [data-testid="connector-action"]';
  return [...root.querySelectorAll(parts)].map((part) => part.textContent.trim()).join(' ');
}

function press(root: ParentNode, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

describe('ConnectorRowComponent', () => {
  it('connects one available, and asks to disconnect one connected', async () => {
    const available = await drawn(NOTION);
    expect(line(available.root)).toBe('N Notion Pages, databases and search. Connect');
    expect(available.root.querySelector('[data-testid="connector-add-account"]')).toBeNull();
    press(available.root, 'connector-action');
    expect(available.acted).toEqual([{ kind: 'connect', account: null }]);
    await expectAccessible(available.root);

    const connected = await drawn({ ...NOTION, status: 'connected' }, { choosing: false });
    expect(line(connected.root)).toBe('N Notion Connected Disconnect');
    expect(connected.root.querySelector('.dot')).not.toBeNull();
    press(connected.root, 'connector-action');
    press(connected.root, 'connector-add-account');
    expect(connected.acted).toEqual([
      { kind: 'disconnect', account: null, called: 'Notion' },
      { kind: 'add' },
    ]);
  });

  it('does nothing more while a call for it runs', async () => {
    const { root, acted } = await drawn(NOTION, { working: true });
    expect(line(root)).toBe('N Notion Pages, databases and search. Connecting…');
    const button = root.querySelector<HTMLButtonElement>('[data-testid="connector-action"]');
    expect(button?.disabled).toBe(true);
    press(root, 'connector-action');
    expect(acted).toEqual([]);
  });

  it('offers no choice of place to a person with only their Chats', async () => {
    const accounts = [{ status: 'connected', used: 'Everywhere, including chats' }] as const;
    const one = { ...NOTION, status: 'connected', accounts } as const;
    const { root } = await drawn(one, { choosing: false });
    expect(line(root)).toBe('N Notion Everywhere, including chats Disconnect');
    expect(root.querySelector('[data-testid="connector-place-trigger"]')).toBeNull();
    const chooses = await drawn(one);
    const place = chooses.root.querySelector('[data-testid="connector-place-trigger"]');
    expect(place?.textContent).toContain('Where Notion is used');
  });

  it('shows the sign-in as a link where no tab opened, and stops it', async () => {
    const { root, acted } = await drawn(NOTION, {
      waiting: { link: PAGE, opened: false },
      working: true,
    });
    const waiting = root.querySelector('[data-testid="connector-waiting"]');
    expect(waiting?.textContent).toContain('To finish, open the sign-in for Notion.');
    const link = root.querySelector('[data-testid="connector-open"]');
    expect(link?.getAttribute('href')).toBe(PAGE);
    expect(link?.textContent).toContain('Open the sign-in');
    await expectAccessible(root);
    press(root, 'connector-action');
    expect(acted).toEqual([{ kind: 'cancel' }]);
  });

  it("says why the last thing done with it failed, in the server's words", async () => {
    const { root } = await drawn(NOTION, { problem: 'Notion could not be reached.' });
    expect(root.querySelector('[data-testid="connector-problem"]')?.textContent).toBe(
      'Notion could not be reached.',
    );
    expect(root.querySelector('[data-testid="connector-detail"]')).toBeNull();
  });

  it('signs in again to the account whose sign-in stopped working, or disconnects it', async () => {
    const accounts = [{ status: 'sign_in_again', only: 'p1', used: 'Only in Acme Site' }] as const;
    const { root, acted } = await drawn({ ...NOTION, status: 'sign_in_again', accounts });
    expect(line(root)).toBe('N Notion Sign in again to keep using it Sign in again');
    press(root, 'connector-action');
    press(root, 'connector-disconnect');
    expect(acted).toEqual([
      { kind: 'connect', account: null, only: 'p1' },
      { kind: 'disconnect', account: null, called: 'Notion' },
    ]);
  });

  it('lists two accounts beneath it, each with its own menu, and no button of its own', async () => {
    const accounts = [
      { name: 'Glave', status: 'connected', used: 'Everywhere, including chats' },
      { name: 'Acme', status: 'connected', only: 'p1', used: 'Only in Acme Site' },
    ] as const;
    const { root } = await drawn({ ...NOTION, status: 'connected', accounts });
    expect(line(root)).toBe('N Notion 2 accounts');
    expect(root.querySelector('.dot')).not.toBeNull();
    expect(root.querySelectorAll('[data-testid="connector-account"]')).toHaveLength(2);
    expect(root.querySelector('[data-testid="connector-place-trigger"]')).toBeNull();
    expect(root.querySelector('[data-testid="connector-action"]')).toBeNull();
    await expectAccessible(root);
  });

  it('for a project, names the account its threads use and why the other is not', async () => {
    const accounts = [
      { name: 'Acme', status: 'connected', only: 'p1', used: 'Only in Acme Site' },
    ] as const;
    const replaced = 'Your Notion account used everywhere else is not used here.';
    const { root } = await drawn({ ...NOTION, status: 'connected', accounts, replaced });
    expect(line(root)).toBe('N Notion: Acme Only in Acme Site Disconnect');
    expect(root.querySelector('[data-testid="connector-note"]')?.textContent.trim()).toBe(replaced);
    expect(root.querySelector('[data-testid="connector-accounts"]')).toBeNull();
    await expectAccessible(root);
    const plain = await drawn({ ...NOTION, status: 'connected', accounts });
    expect(plain.root.querySelector('[data-testid="connector-note"]')).toBeNull();
  });
});
