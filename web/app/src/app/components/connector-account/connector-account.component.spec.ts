import { TestBed } from '@angular/core/testing';
import type { ConnectorAccount } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { type AccountAction, ConnectorAccountComponent } from './connector-account.component';

const PLACES = [
  { value: 'everywhere', label: 'Everywhere, including chats' },
  { value: 'p1', label: 'Only in Acme Site' },
];

/** The account `account` drawn, and what its menu says it picked. */
async function drawn(account: ConnectorAccount): Promise<{
  readonly root: HTMLElement;
  readonly picked: AccountAction[];
  readonly pick: (text: string) => Promise<void>;
}> {
  const fixture = TestBed.createComponent(ConnectorAccountComponent);
  fixture.componentRef.setInput('service', 'Stripe');
  fixture.componentRef.setInput('account', account);
  fixture.componentRef.setInput('places', PLACES);
  fixture.componentRef.setInput('place', account.only ?? 'everywhere');
  const picked: AccountAction[] = [];
  fixture.componentInstance.picked.subscribe((it) => picked.push(it));
  await fixture.whenStable();
  const root = fixture.nativeElement as HTMLElement;
  const pick = async (text: string): Promise<void> => {
    root.querySelector<HTMLElement>('[data-testid="connector-account-menu-trigger"]')?.click();
    await fixture.whenStable();
    const items = document.querySelectorAll<HTMLElement>(
      '[data-testid="connector-account-menu-action"]',
    );
    [...items].find((item) => item.textContent.includes(text))?.click();
    await fixture.whenStable();
  };
  return { root, picked, pick };
}

describe('ConnectorAccountComponent', () => {
  it('names the account and where it is used, and its menu moves or disconnects it', async () => {
    const { root, picked, pick } = await drawn({
      name: 'Acme',
      status: 'connected',
      only: 'p1',
      used: 'Only in Acme Site',
    });
    expect(root.querySelector('.label')?.textContent).toBe('Acme');
    expect(root.querySelector('.used')?.textContent).toBe('Only in Acme Site');
    // As the page holds it: an item of its service's list of accounts.
    const list = document.createElement('div');
    list.setAttribute('role', 'list');
    document.body.append(list);
    list.append(root);
    await expectAccessible(list);
    await pick('Everywhere');
    await pick('Disconnect');
    expect(picked).toEqual([{ kind: 'place', place: 'everywhere' }, { kind: 'disconnect' }]);
  });

  it('goes by the service for the account never named, and offers signing in again', async () => {
    const { root, picked, pick } = await drawn({
      status: 'sign_in_again',
      used: 'Everywhere, including chats',
    });
    expect(root.querySelector('.label')?.textContent).toBe('Stripe');
    await pick('Sign in again');
    expect(picked).toEqual([{ kind: 'again' }]);
  });
});
