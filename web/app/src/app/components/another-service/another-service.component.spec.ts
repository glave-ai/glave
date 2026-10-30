import { TestBed } from '@angular/core/testing';
import { ApiError, type CredentialInfo, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { AnotherServiceComponent } from './another-service.component';

const GITLAB: CredentialInfo = {
  id: '01900000-0000-7000-8000-000000000403',
  forge: 'gitlab',
  url: 'https://gitlab.com',
  login: 'ana',
};

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

function byTestId<T extends HTMLElement>(root: ParentNode, testId: string): T | null {
  return root.querySelector<T>(`[data-testid="${testId}"]`);
}

/** Types `text` into the field `testId`, as a person does. */
function type(root: ParentNode, testId: string, text: string): void {
  const field = byTestId<HTMLInputElement>(root, testId);
  if (field === null) {
    throw new Error(`no field ${testId}`);
  }
  field.value = text;
  field.dispatchEvent(new Event('input'));
}

/** The section shown, and a way to let what it started finish. */
async function shown(fake: FakeSession): Promise<[HTMLElement, () => Promise<void>]> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(AnotherServiceComponent);
  const settled = (): Promise<void> => settle(fixture);
  await settled();
  return [fixture.nativeElement as HTMLElement, settled];
}

describe('AnotherServiceComponent', () => {
  it('adds a service by its address, then connects it with the token it asks for', async () => {
    const fake = new FakeSession().answer('credential/connect', { credential: GITLAB });
    const [root, settled] = await shown(fake);
    await expectAccessible(root);
    type(root, 'another-address', 'https://gitlab.com');
    byTestId(root, 'another-add')?.click();
    await settled();
    expect(byTestId(root, 'another-asks')?.textContent).toContain(
      'gitlab.com needs an access token',
    );
    await expectAccessible(root);
    type(root, 'another-token', 'the-token');
    byTestId(root, 'another-connect')?.click();
    await settled();
    expect(fake.paramsOf('credential/connect')).toEqual([
      { url: 'https://gitlab.com', token: 'the-token' },
    ]);
    // Connected, it is back to an address, the token forgotten.
    expect(byTestId(root, 'another-token-step')).toBeNull();
    expect(byTestId<HTMLInputElement>(root, 'another-address')?.value).toBe('');
  });

  it('asks for what is missing, and says the server’s reason when the token is refused', async () => {
    const refused = new ApiError({
      code: ErrorCode.NO_CREDENTIAL,
      message: 'gitlab.com did not accept this token',
    });
    const fake = new FakeSession().answer('credential/connect', refused);
    const [root, settled] = await shown(fake);
    byTestId(root, 'another-add')?.click();
    await settled();
    expect(byTestId(root, 'another-problem')?.textContent).toContain('address first');
    type(root, 'another-address', 'https://gitlab.com');
    byTestId(root, 'another-add')?.click();
    await settled();
    byTestId(root, 'another-connect')?.click();
    await settled();
    expect(byTestId(root, 'another-problem')?.textContent).toContain('Paste the token first');
    expect(fake.paramsOf('credential/connect')).toEqual([]);
    type(root, 'another-token', 'the-token');
    byTestId(root, 'another-connect')?.click();
    await settled();
    expect(byTestId(root, 'another-problem')?.textContent).toContain('did not accept this token');
    byTestId(root, 'another-cancel')?.click();
    await settled();
    expect(byTestId(root, 'another-token-step')).toBeNull();
  });
});
