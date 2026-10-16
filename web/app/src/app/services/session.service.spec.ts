import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { TOKEN_KEY } from '../lib/token';
import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { SESSION_CLIENT_FACTORY, SessionService } from './session.service';

describe('SessionService', () => {
  it('is signed in with a kept token, and calls through the client', async () => {
    const fake = new FakeSession().answer('server/discover', { name: 'glave' });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const session = TestBed.inject(SessionService);
    expect(session.signedIn()).toBe(true);
    expect(session.local()).toBe(true);
    expect(await session.call('server/discover', {})).toEqual({ name: 'glave' });
  });

  it('signs out when the server refuses the sign-in', async () => {
    const fake = new FakeSession().answer(
      'space/list',
      new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'no' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const session = TestBed.inject(SessionService);
    await expect(session.call('space/list', {})).rejects.toBeInstanceOf(ApiError);
    expect(session.signedIn()).toBe(false);
    expect(sessionStorage.getItem(TOKEN_KEY)).toBeNull();
  });

  it('streams a turn and returns its result', async () => {
    const fake = new FakeSession().streams([
      { jsonrpc: '2.0', method: 'turn/live', params: { thread: 't', update: { kind: 'waiting' } } },
    ]);
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const session = TestBed.inject(SessionService);
    const turn = session.stream('turn/start', { thread: 't', prompt: 'hi' });
    const first = await turn.next();
    expect(first.done).toBe(false);
    const last = await turn.next();
    expect(last.done).toBe(true);
  });

  it('signs each call in with the token, from the default client', () => {
    sessionStorage.setItem(TOKEN_KEY, 'glave_local_x');
    const factory = TestBed.inject(SESSION_CLIENT_FACTORY);
    expect(typeof factory).toBe('function');
    expect(TestBed.inject(SessionService).signedIn()).toBe(true);
  });

  it('is with a glave elsewhere when signed in with another token', () => {
    sessionStorage.setItem(TOKEN_KEY, '"an-access-token"');
    const session = TestBed.inject(SessionService);
    expect(session.signedIn()).toBe(true);
    expect(session.local()).toBe(false);
    session.signOut();
    expect(session.local()).toBe(false);
  });
});
