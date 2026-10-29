import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type InviteInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { ServerService } from '../../services/server.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { SERVER } from '../../testing/fixtures';
import { InviteDialogComponent } from './invite-dialog.component';

const MADE: InviteInfo = {
  id: 'i1',
  role: 'admin',
  by: { kind: 'person', id: 'p-dana' },
  made: '2026-10-29T09:00:00.000Z',
  expires: '2026-11-05T09:00:00.000Z',
  email: 'alex@example.com',
};

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

function byTestId<T extends HTMLElement>(testId: string): T | null {
  return document.querySelector<T>(`[data-testid="${testId}"]`);
}

function fakeServer(): FakeSession {
  return new FakeSession()
    .answer('server/discover', { ...SERVER, origin: 'https://glave.example', email: true })
    .answer('people/list', { people: [] })
    .answer('invite/list', { invites: [] });
}

async function opened(fake: FakeSession): Promise<ComponentFixture<InviteDialogComponent>> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  await TestBed.inject(ServerService).load();
  const fixture = TestBed.createComponent(InviteDialogComponent);
  fixture.componentRef.setInput('open', true);
  await settle(fixture);
  return fixture;
}

describe('InviteDialogComponent', () => {
  it('makes an invite for an email and role, then shows its link and that it was emailed', async () => {
    const fake = fakeServer().answer('invite/create', { invite: MADE, code: 'c0de' });
    const fixture = await opened(fake);
    await expectAccessible(document.body);
    const email = byTestId<HTMLInputElement>('invite-email');
    if (email === null) {
      throw new Error('no email field');
    }
    email.value = 'alex@example.com';
    email.dispatchEvent(new Event('input'));
    document
      .querySelector<HTMLInputElement>('[data-testid="invite-role"] input[value="admin"]')
      ?.click();
    await settle(fixture);
    byTestId('invite-make')?.click();
    await settle(fixture);
    expect(fake.paramsOf('invite/create')).toEqual([{ role: 'admin', email: 'alex@example.com' }]);
    expect(byTestId<HTMLInputElement>('invite-link')?.value).toBe(
      'https://glave.example/#invite=c0de',
    );
    expect(byTestId('invite-told')?.textContent).toContain('We emailed it to alex@example.com');
    expect(byTestId('invite-told')?.textContent).toContain('for 7 days');
    await expectAccessible(document.body);
  });

  it('makes a link alone without an email, and says why when the server refuses', async () => {
    const refused = new ApiError({
      code: ErrorCode.NOT_AVAILABLE,
      message: 'this glave sends no email: share the invite’s link yourself',
    });
    const fake = fakeServer().answer('invite/create', refused);
    const fixture = await opened(fake);
    byTestId('invite-make')?.click();
    await settle(fixture);
    expect(fake.paramsOf('invite/create')).toEqual([{ role: 'member' }]);
    expect(byTestId('invite-problem')?.textContent).toContain('share the invite’s link yourself');
  });

  it('asks for no address where glave sends no email', async () => {
    const fake = new FakeSession()
      .answer('server/discover', { ...SERVER, origin: 'https://glave.example' })
      .answer('people/list', { people: [] })
      .answer('invite/list', { invites: [] });
    await opened(fake);
    expect(byTestId('invite-email')).toBeNull();
    expect(byTestId('invite-role')).not.toBeNull();
  });
});
