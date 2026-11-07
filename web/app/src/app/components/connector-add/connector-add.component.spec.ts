import { type ComponentFixture, TestBed } from '@angular/core/testing';
import type { ConnectorInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { type AccountChosen, ConnectorAddComponent } from './connector-add.component';

const STRIPE: ConnectorInfo = {
  id: 'stripe',
  name: 'Stripe',
  company: 'stripe',
  url: 'https://mcp.stripe.com',
  categories: ['business'],
  tags: ['payments'],
  returns: 'text',
  sign_in: 'dcr',
  description: 'Payments.',
  docs: 'https://docs.stripe.com/mcp',
  status: 'connected',
  accounts: [{ name: 'Glave', status: 'connected', used: 'Everywhere, including chats' }],
};

function byTestId<T extends HTMLElement>(testId: string): T | null {
  return document.querySelector<T>(`[data-testid="${testId}"]`);
}

/** The dialog open for Stripe, from the project `here`, or Settings, and what it chose. */
async function opened(here: string | null): Promise<{
  readonly fixture: ComponentFixture<ConnectorAddComponent>;
  readonly chosen: AccountChosen[];
}> {
  const fixture = TestBed.createComponent(ConnectorAddComponent);
  fixture.componentRef.setInput('connector', STRIPE);
  fixture.componentRef.setInput('here', here);
  fixture.componentRef.setInput('hereName', 'Acme Site');
  fixture.componentRef.setInput('places', [
    { value: 'everywhere', label: 'Everywhere, including chats' },
    { value: 'p1', label: 'Only in Acme Site' },
  ]);
  fixture.componentRef.setInput('open', true);
  const chosen: AccountChosen[] = [];
  fixture.componentInstance.chosen.subscribe((it) => chosen.push(it));
  await fixture.whenStable();
  return { fixture, chosen };
}

/** Picks the radio of the group `testId` that says `text`. */
async function pickRadio(
  fixture: ComponentFixture<ConnectorAddComponent>,
  testId: string,
  text: string,
): Promise<void> {
  const labels = byTestId(testId)?.querySelectorAll('label') ?? [];
  const label = [...labels].find((it) => it.textContent.includes(text));
  label?.querySelector('input')?.click();
  await fixture.whenStable();
}

describe('ConnectorAddComponent', () => {
  it('from a project, signs in to another account, named, used only there', async () => {
    const { fixture, chosen } = await opened('p1');
    const dialog = byTestId('connector-add');
    expect(dialog?.textContent).toContain('Glave');
    expect(dialog?.textContent).toContain('Only in Acme Site');
    await expectAccessible(document.body);
    const name = byTestId<HTMLInputElement>('connector-add-name');
    if (name === null) {
      throw new Error('no name field');
    }
    name.value = 'Acme';
    name.dispatchEvent(new Event('input'));
    byTestId('connector-add-done')?.click();
    await fixture.whenStable();
    expect(chosen).toEqual([{ existing: false, account: 'Acme', only: 'p1' }]);
  });

  it('uses an account connected already, from Settings everywhere unless moved', async () => {
    const { fixture, chosen } = await opened(null);
    // Glave is used everywhere, so a new account starts where none is.
    const places = byTestId('connector-add-place')?.querySelectorAll('label') ?? [];
    const checked = [...places].find((it) => it.querySelector('input')?.checked === true);
    expect(checked?.textContent).toContain('Only in Acme Site');
    await pickRadio(fixture, 'connector-add-which', 'Glave');
    expect(byTestId('connector-add-name')).toBeNull();
    expect(byTestId('connector-add-done')?.textContent.trim()).toBe('Use it');
    await pickRadio(fixture, 'connector-add-place', 'Only in Acme Site');
    byTestId('connector-add-done')?.click();
    await fixture.whenStable();
    expect(chosen).toEqual([{ existing: true, account: 'Glave', only: 'p1' }]);
  });
});
