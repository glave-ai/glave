import { type ComponentFixture, TestBed } from '@angular/core/testing';
import type { MockInstance } from 'vitest';
import { Router, provideRouter } from '@angular/router';
import { ApiError, ErrorCode, type ShownInvite } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import type { Authenticator } from '../../lib/passkeys';
import { type BrowserSession, TOKEN_KEY } from '../../lib/token';
import { AUTHENTICATOR } from '../../services/passkeys.service';
import { SessionService } from '../../services/session.service';
import {
  FAKE_SERVER,
  FakeSession,
  SIGNED_IN,
  provideFakeSession,
} from '../../testing/fake-session';
import { MADE_PASSKEY, PASSKEY_OPTIONS } from '../../testing/fixtures';
import { InviteComponent } from './invite.component';

const SHOWN: ShownInvite = {
  organization: '0192f5c6-0000-7000-8000-000000000a0c',
  role: 'member',
  by: 'Dana Lee',
  joining: 'passkey',
};

const OPTIONS = PASSKEY_OPTIONS;

const MADE = MADE_PASSKEY;

const device: Authenticator = {
  create: () => Promise.resolve(MADE as unknown as Credential),
  get: () => Promise.resolve(null),
};

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

/** The page opened with an invite's link, on `fake`, signed in as `session`. */
async function render(
  fake: FakeSession,
  session: BrowserSession | null,
): Promise<{
  readonly fixture: ComponentFixture<InviteComponent>;
  readonly navigate: MockInstance<Router['navigateByUrl']>;
  readonly root: HTMLElement;
}> {
  window.history.replaceState(null, '', '/#invite=c0de');
  // The tab starts with the session this browser kept, if any.
  if (session !== null) {
    localStorage.setItem(TOKEN_KEY, JSON.stringify(session));
  }
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      ...provideFakeSession(fake.answer('server/discover', { id: FAKE_SERVER }), session),
      { provide: AUTHENTICATOR, useValue: device },
    ],
  });
  await TestBed.inject(SessionService).start();
  const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
  const fixture = TestBed.createComponent(InviteComponent);
  await settle(fixture);
  return { fixture, navigate, root: fixture.nativeElement as HTMLElement };
}

function click(root: ParentNode, testId: string): void {
  root.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.click();
}

describe('InviteComponent', () => {
  // A passkey made in one test keeps its session for the next: each starts signed out.
  afterEach(() => {
    localStorage.clear();
  });

  it('new to glave, makes a passkey with the invite and joins', async () => {
    const fake = new FakeSession()
      .answer('invite/show', { shown: SHOWN })
      .answer('passkey/create_options', OPTIONS)
      .answer('passkey/create', { session: 'glave_account_made', server: FAKE_SERVER });
    const { fixture, navigate, root } = await render(fake, null);
    expect(window.location.hash).toBe('');
    expect(fake.paramsOf('invite/show')).toEqual([{ code: 'c0de' }]);
    const page = root.querySelector('[data-testid="invite-passkey"]');
    expect(page?.textContent).toContain('Dana Lee invited you to glave');
    expect(page?.textContent).toContain("You'll join as a member.");
    await expectAccessible(root);
    click(root, 'invite-make-passkey');
    await settle(fixture);
    expect(fake.paramsOf('passkey/create_options')).toEqual([{ code: 'c0de', link: 'invite' }]);
    expect(TestBed.inject(SessionService).inviteCode()).toBeNull();
    expect(navigate).toHaveBeenCalledWith('/');
  });

  it('where the team signs people in with its own login, sends them there', async () => {
    const fake = new FakeSession().answer('invite/show', {
      shown: { ...SHOWN, joining: 'company', role: 'admin' },
    });
    const { fixture, navigate, root } = await render(fake, null);
    const page = root.querySelector('[data-testid="invite-company"]');
    expect(page?.textContent).toContain("You'll join as an admin.");
    await expectAccessible(root);
    click(root, 'invite-company-sign-in');
    await settle(fixture);
    expect(navigate).toHaveBeenCalledWith('/sign-in');
    expect(TestBed.inject(SessionService).inviteCode()).toBe('c0de');
  });

  it('signed in already, joins as one more team of theirs, or puts it aside', async () => {
    const fake = new FakeSession()
      .answer('invite/show', { shown: { ...SHOWN, joining: 'accept' } })
      .answer('invite/accept', { person: { id: 'p', role: 'member', you: true } });
    const { fixture, navigate, root } = await render(fake, SIGNED_IN);
    const page = root.querySelector('[data-testid="invite-joining"]');
    expect(page?.textContent).toContain('Join this team?');
    expect(page?.textContent).toContain('Dana Lee invited you.');
    await expectAccessible(root);
    click(root, 'invite-join');
    await settle(fixture);
    expect(fake.paramsOf('invite/accept')).toEqual([{ code: 'c0de' }]);
    expect(navigate).toHaveBeenCalledWith('/');

    TestBed.resetTestingModule();
    const later = await render(
      new FakeSession().answer('invite/show', { shown: { ...SHOWN, joining: 'accept' } }),
      SIGNED_IN,
    );
    click(later.root, 'invite-not-now');
    await settle(later.fixture);
    expect(TestBed.inject(SessionService).inviteCode()).toBeNull();
    expect(later.navigate).toHaveBeenCalledWith('/');
  });

  it('says an invite that opens nothing doesn’t work anymore', async () => {
    const gone = new ApiError({ code: ErrorCode.NO_INVITE, message: 'this invite opens nothing' });
    const { root } = await render(new FakeSession().answer('invite/show', gone), null);
    const page = root.querySelector('[data-testid="invite-gone"]');
    expect(page?.textContent).toContain("This invite doesn't work anymore");
    expect(page?.textContent).toContain('Ask whoever sent it for a new one.');
    await expectAccessible(root);
  });

  it('tells someone in the team already that there is nothing to join', async () => {
    const fake = new FakeSession().answer('invite/show', {
      shown: { ...SHOWN, joining: 'member' },
    });
    const { fixture, navigate, root } = await render(fake, SIGNED_IN);
    const page = root.querySelector('[data-testid="invite-member"]');
    expect(page?.textContent).toContain("You're in this team already");
    await expectAccessible(root);
    root.querySelector<HTMLElement>('[data-testid="invite-open"]')?.click();
    await settle(fixture);
    expect(navigate).toHaveBeenCalledWith('/');
  });
});
