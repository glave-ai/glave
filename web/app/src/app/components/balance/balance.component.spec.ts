import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { BalanceComponent } from './balance.component';

async function render(fake: FakeSession): Promise<HTMLElement> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(BalanceComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return fixture.nativeElement as HTMLElement;
}

function find(root: ParentNode, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

/** What `credits/get` says to an Owner whose balance is `nanos`. */
function credits(nanos: number): object {
  return {
    credits: {
      account: {
        balance: { nanos, currency: 'USD' },
        spent: { nanos: 0, currency: 'USD' },
      },
      people: [],
      since: '2026-11-01T00:00:00.000Z',
      can_set_limit: true,
    },
  };
}

describe('BalanceComponent', () => {
  it('shows what is left, in dollars, where the server keeps a balance', async () => {
    const fake = new FakeSession()
      .answer('server/discover', { ...SERVER, exists: { organization: false, credits: true } })
      .answer('credits/get', credits(4_250_000_000));
    const root = await render(fake);
    expect(find(root, 'balance')?.textContent).toContain('Balance');
    expect(find(root, 'balance-left')?.textContent).toBe('$4.25 left');
    await expectAccessible(root);
  });

  it('says the balance is used up at or below nothing', async () => {
    for (const nanos of [0, -120_000_000]) {
      TestBed.resetTestingModule();
      const fake = new FakeSession()
        .answer('server/discover', { ...SERVER, exists: { organization: false, credits: true } })
        .answer('credits/get', credits(nanos));
      const root = await render(fake);
      expect(find(root, 'balance-left')?.textContent).toBe('Used up');
    }
  });

  it('shows nothing where the server keeps no balance, and does not ask for one', async () => {
    const fake = new FakeSession().answer('server/discover', SERVER);
    const root = await render(fake);
    expect(find(root, 'balance')).toBeNull();
    expect(fake.paramsOf('credits/get')).toEqual([]);
    expect(getComputedStyle(root).display).toBe('contents');
  });

  it('shows nothing to someone the server does not show the balance to', async () => {
    const fake = new FakeSession()
      .answer('server/discover', { ...SERVER, exists: { organization: true, credits: true } })
      .answer('credits/get', {
        credits: { people: [], since: '2026-11-01T00:00:00.000Z', can_set_limit: false },
      });
    const root = await render(fake);
    expect(find(root, 'balance')).toBeNull();
  });
});
