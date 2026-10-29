import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { InviteService } from './invite.service';
import { SessionService } from './session.service';
import { FAKE_SERVER, FakeSession, SIGNED_IN, provideFakeSession } from '../testing/fake-session';

const SHOWN = {
  organization: '0192f5c6-0000-7000-8000-000000000a0c',
  role: 'admin',
  joining: 'passkey',
} as const;

async function opened(fake: FakeSession, link = '/#invite=c0de'): Promise<InviteService> {
  window.history.replaceState(null, '', link);
  TestBed.configureTestingModule({
    providers: provideFakeSession(fake.answer('server/discover', { id: FAKE_SERVER }), SIGNED_IN),
  });
  await TestBed.inject(SessionService).start();
  return TestBed.inject(InviteService);
}

describe('InviteService', () => {
  it('says what the invite opens, and that a spent one opens nothing', async () => {
    const invite = await opened(new FakeSession().answer('invite/show', { shown: SHOWN }));
    expect(invite.pending()).toBe(true);
    await invite.load();
    expect(invite.opened()).toEqual({ kind: 'shown', shown: SHOWN });
    TestBed.resetTestingModule();
    const gone = new ApiError({ code: ErrorCode.NO_INVITE, message: 'opens nothing' });
    const spent = await opened(new FakeSession().answer('invite/show', gone));
    await spent.load();
    expect(spent.opened()).toEqual({ kind: 'gone' });
  });

  it('accepts as the person signed in, and keeps the invite when the server refuses', async () => {
    const refused = new ApiError({ code: ErrorCode.NO_INVITE, message: 'opens nothing' });
    const fake = new FakeSession().answer('invite/accept', refused);
    const invite = await opened(fake);
    expect(await invite.accept()).toBe('opens nothing');
    expect(invite.pending()).toBe(true);
    fake.answer('invite/accept', { person: { id: 'p', role: 'admin', you: true } });
    expect(await invite.accept()).toBeNull();
    expect(invite.pending()).toBe(false);
  });

  it('opens nothing for a tab with no invite', async () => {
    const invite = await opened(new FakeSession(), '/');
    expect(invite.pending()).toBe(false);
    await invite.load();
    expect(invite.opened()).toEqual({ kind: 'gone' });
  });
});
