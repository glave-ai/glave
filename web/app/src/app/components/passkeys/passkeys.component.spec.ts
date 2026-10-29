import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type PasskeySummary } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import type { Authenticator } from '../../lib/passkeys';
import { AUTHENTICATOR, passkeyRefused } from '../../services/passkeys.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { MADE_PASSKEY, PASSKEY_OPTIONS } from '../../testing/fixtures';
import { PasskeysComponent } from './passkeys.component';

const PHONE: PasskeySummary = {
  id: 'p1',
  name: 'Phone',
  added: '2026-10-20T09:00:00.000Z',
  used: '2026-10-21T09:00:00.000Z',
};

const LAPTOP: PasskeySummary = { id: 'p2', name: 'Laptop', added: '2026-10-22T09:00:00.000Z' };

const OPTIONS = PASSKEY_OPTIONS;

const MADE = MADE_PASSKEY;

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

/** The card, on `fake`, its passkeys made on `device`. */
async function render(
  fake: FakeSession,
  device: Authenticator,
): Promise<{ readonly fixture: ComponentFixture<PasskeysComponent>; readonly root: HTMLElement }> {
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), { provide: AUTHENTICATOR, useValue: device }],
  });
  const fixture = TestBed.createComponent(PasskeysComponent);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement };
}

const answering = (answer: unknown): Authenticator => ({
  create: () => Promise.resolve(answer as Credential),
  get: () => Promise.resolve(answer as Credential),
});

describe('PasskeysComponent', () => {
  it('lists the passkeys and adds one on this device', async () => {
    let kept: readonly PasskeySummary[] = [PHONE];
    const fake = new FakeSession()
      .answer('passkey/list', () => ({ passkeys: kept }))
      .answer('passkey/add_options', OPTIONS)
      .answer('passkey/add', () => {
        kept = [PHONE, LAPTOP];
        return { passkey: LAPTOP };
      });
    const { fixture, root } = await render(fake, answering(MADE));
    const rows = (): NodeListOf<Element> =>
      root.querySelectorAll('[data-testid="passkeys-passkey"]');
    expect(rows().length).toBe(1);
    expect(rows()[0]?.textContent).toContain('Phone');
    expect(rows()[0]?.textContent).toContain('last used');
    await expectAccessible(root);
    click(root, 'passkeys-add');
    await settle(fixture);
    expect(fake.paramsOf('passkey/add')).toEqual([
      expect.objectContaining({ ceremony: 'c1' }) as unknown,
    ]);
    expect(rows().length).toBe(2);
  });

  it('removes a passkey once the person says so', async () => {
    let kept: readonly PasskeySummary[] = [PHONE, LAPTOP];
    const fake = new FakeSession()
      .answer('passkey/list', () => ({ passkeys: kept }))
      .answer('passkey/remove', () => {
        kept = [LAPTOP];
        return {};
      });
    const { fixture, root } = await render(fake, answering(MADE));
    click(root, 'passkeys-remove');
    await settle(fixture);
    click(document, 'passkeys-confirm-ok');
    await settle(fixture);
    expect(fake.paramsOf('passkey/remove')).toEqual([{ passkey: 'p1' }]);
    expect(root.querySelectorAll('[data-testid="passkeys-passkey"]').length).toBe(1);
  });

  it('says why a passkey was not added: the device made none, or the server refused', async () => {
    const fake = new FakeSession()
      .answer('passkey/list', { passkeys: [] })
      .answer('passkey/add_options', OPTIONS);
    const { fixture, root } = await render(fake, answering(null));
    expect(root.querySelector('[data-testid="passkeys-none"]')).not.toBeNull();
    click(root, 'passkeys-add');
    await settle(fixture);
    const problem = (): string | null | undefined =>
      root.querySelector('[data-testid="passkeys-problem"]')?.textContent;
    expect(problem()).toContain(passkeyRefused());
    fake.answer(
      'passkey/add_options',
      new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'only the person may' }),
    );
    click(root, 'passkeys-add');
    await settle(fixture);
    expect(problem()).toContain('only the person may');
    await expectAccessible(root);
  });
});
